//! The executor: a tree walk over `Program`, the plan with its keys interned, reproducing what the
//! JavaScript codegen.ts emits. Generic over the document (`Doc`) and the output (`Sink`).
//!
//! Rules, from codegen.ts `fieldExpr` and `array`:
//! - A missing or null source is null and a miss, never an error.
//! - A present value whose transforms (then the routing validator) give null is an error and a miss.
//!   Either way `default` then applies, and the field still counts as a miss.
//! - `coalesce` takes the first present value and does not retry after a rejection.
//! - `from` with several paths hands the transforms a list (null only if every path is absent).
//!   Without transforms the field is that list.
//! - A list `from` that is present but not an array is null, an error and a miss on the list slot.
//! - `item` maps each element. A plucked or plain element that a transform rejects is null in place
//!   and one error on the list slot. An absent element is null.
//! - Language 2: a list's `ListOps` choose `[]` or null for a missing and for an empty source.
//!   `Empty::Missing` leaves the key out when the source is missing or null. `OmitNull` leaves the
//!   key out when nothing under it is written as something other than null.

use crate::canonical::{Empty, Semantic};
use crate::doc::{Doc, Key};
use crate::plan::{CompileError, Field, ListKind, ListOps, Named, Plan, Scalar, TxRef};
use crate::sink::{Out, Sink, copy};
use crate::spec::{self, Source};
use crate::stats::Stats;
use crate::transforms::{Transform, validate};
use crate::value::Value;
use serde_json::Value as J;
use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

enum Step {
    Key(Key),
    Idx(usize),
}

type Path = Vec<Step>;

/// A path read in one `Scope`: the number of its node in the scope's prefix tree.
type At = usize;

/// Every path read against one document, the root (with the objects nested in it, which read the
/// same document) or one list element, as a prefix tree: node 0 is the document itself and node
/// `n > 0` is `nodes[n - 1]`, a step from its parent node. A prefix that several reads share is
/// resolved once per document (`Frame`) instead of once per read.
#[derive(Default)]
struct Scope {
    nodes: Vec<(At, Step)>,
    /// Per node, its slot in `Frame::cache` when it is a prefix of two or more reads, `NONE`
    /// otherwise: only those are worth keeping.
    slot: Vec<u32>,
    /// How many nodes have a slot.
    slots: usize,
}

const NONE: u32 = u32::MAX;

/// A `Scope` while the plan is compiled.
#[derive(Default)]
struct ScopeBuilder {
    nodes: Vec<(At, Step)>,
    index: HashMap<(At, StepId), At>,
    /// Reads that pass through each node as a strict prefix.
    through: Vec<u32>,
}

#[derive(PartialEq, Eq, Hash)]
enum StepId {
    Key(u64),
    Idx(usize),
}

impl ScopeBuilder {
    fn at(&mut self, steps: Path) -> At {
        let mut node = 0;
        let last = steps.len();
        for (i, step) in steps.into_iter().enumerate() {
            if node > 0 && i < last {
                self.through[node - 1] += 1;
            }
            let id = match &step {
                Step::Key(k) => StepId::Key(k.tag),
                Step::Idx(i) => StepId::Idx(*i),
            };
            node = match self.index.get(&(node, id)) {
                Some(&n) => n,
                None => {
                    let id = match &step {
                        Step::Key(k) => StepId::Key(k.tag),
                        Step::Idx(i) => StepId::Idx(*i),
                    };
                    self.nodes.push((node, step));
                    self.through.push(0);
                    self.index.insert((node, id), self.nodes.len());
                    self.nodes.len()
                }
            };
        }
        node
    }

    fn finish(self) -> Scope {
        let mut slots = 0;
        let slot = (self.through.iter())
            .map(|&n| {
                if n < 2 {
                    return NONE;
                }
                slots += 1;
                slots as u32 - 1
            })
            .collect();
        Scope {
            nodes: self.nodes,
            slot,
            slots,
        }
    }
}

enum Src {
    From(At),
    Many(Vec<At>),
    Coalesce(Vec<At>),
}

struct Leaf {
    src: Src,
    tx: Vec<TxRef>,
    validator: Option<Semantic>,
    default: Option<serde_json::Value>,
    stat: usize,
}

impl Leaf {
    /// No transform and no validator: the value goes out as it is.
    fn bare(&self) -> bool {
        self.tx.is_empty() && self.validator.is_none()
    }
}

enum Op {
    Null,
    Scalar(Leaf),
    Object(Vec<(usize, Op)>),
    /// An item list maps each element with `Item`. Otherwise each element is read at `Elems.0`
    /// (empty for a plain list) and transformed.
    List {
        from: At,
        kind: Kind,
        stat: usize,
        ops: ListOps,
    },
    /// Written as it is. Counts no miss, so it is always a fill.
    Const(J),
    /// Left out of its object when it writes nothing but null.
    OmitNull(Box<Op>),
}

enum Kind {
    /// The item's fields, read in their own scope (one element).
    Item(Vec<(usize, Op)>, Scope),
    Elems(Path, Vec<TxRef>),
}

/// A `Plan` compiled for execution: path keys become `Key`s, every index is range-checked.
pub struct Program {
    root: Vec<(usize, Op)>,
    scope: Scope,
    keys: Vec<String>,
}

struct Builder<'p> {
    plan: &'p Plan,
    keys: Vec<String>,
    scope: ScopeBuilder,
}

impl Builder<'_> {
    fn err(&self, what: String) -> CompileError {
        let m = &self.plan.meta;
        CompileError {
            message: format!(
                "{} {} v{}: invalid plan: {what}",
                m.provider, m.endpoint, m.version
            ),
        }
    }

    fn stat(&self, stat: usize) -> Result<usize, CompileError> {
        if stat < self.plan.stat_names.len() {
            Ok(stat)
        } else {
            Err(self.err(format!("stat slot {stat} out of range")))
        }
    }

    fn tx(&self, tx: &[TxRef]) -> Result<Vec<TxRef>, CompileError> {
        for t in tx {
            if let TxRef::Custom { index, .. } = t
                && *index >= self.plan.custom_names.len()
            {
                return Err(self.err(format!("custom transform {index} out of range")));
            }
        }
        Ok(tx.to_vec())
    }

    fn path(&mut self, p: &spec::Path) -> Path {
        p.iter()
            .map(|s| match s {
                spec::Step::Idx(i) => Step::Idx(*i),
                spec::Step::Key(k) => {
                    let tag = match self.keys.iter().position(|x| x == k) {
                        Some(t) => t,
                        None => {
                            self.keys.push(k.clone());
                            self.keys.len() - 1
                        }
                    };
                    Step::Key(Key::new(k.clone(), tag as u64))
                }
            })
            .collect()
    }

    /// `p` as a node of the current scope.
    fn at(&mut self, p: &spec::Path) -> At {
        let steps = self.path(p);
        self.scope.at(steps)
    }

    fn leaf(&mut self, s: &Scalar) -> Result<Leaf, CompileError> {
        let src = match &s.source {
            Source::From(p) => Src::From(self.at(p)),
            Source::FromMany(ps) => Src::Many(ps.iter().map(|p| self.at(p)).collect()),
            Source::Coalesce(ps) => Src::Coalesce(ps.iter().map(|p| self.at(p)).collect()),
        };
        Ok(Leaf {
            src,
            tx: self.tx(&s.tx)?,
            validator: s.validator,
            default: s.default.clone(),
            stat: self.stat(s.stat)?,
        })
    }

    fn fields(&mut self, fields: &[Named<Field>]) -> Result<Vec<(usize, Op)>, CompileError> {
        fields
            .iter()
            .map(|f| {
                if f.name_idx >= self.plan.names.len() {
                    return Err(self.err(format!("name index {} out of range", f.name_idx)));
                }
                Ok((f.name_idx, self.op(&f.value)?))
            })
            .collect()
    }

    fn op(&mut self, field: &Field) -> Result<Op, CompileError> {
        Ok(match field {
            Field::Null => Op::Null,
            Field::Scalar(s) => Op::Scalar(self.leaf(s)?),
            Field::Object(c) => Op::Object(self.fields(c)?),
            Field::Const(v, stat) => {
                self.stat(*stat)?;
                Op::Const(v.clone())
            }
            Field::OmitNull(inner) => Op::OmitNull(Box::new(self.op(inner)?)),
            Field::List {
                from,
                kind,
                stat,
                ops,
            } => Op::List {
                from: self.at(from),
                kind: match kind {
                    ListKind::Item(c) => {
                        let outer = std::mem::take(&mut self.scope);
                        let fields = self.fields(c);
                        let inner = std::mem::replace(&mut self.scope, outer);
                        Kind::Item(fields?, inner.finish())
                    }
                    ListKind::Pluck(p, tx) => Kind::Elems(self.path(p), self.tx(tx)?),
                    ListKind::Plain(tx) => Kind::Elems(Vec::new(), self.tx(tx)?),
                },
                stat: self.stat(*stat)?,
                ops: *ops,
            },
        })
    }
}

impl Program {
    /// Compiles `plan`, rejecting any name, stat or custom transform index out of range.
    pub fn new(plan: &Plan) -> Result<Program, CompileError> {
        let mut b = Builder {
            plan,
            keys: Vec::new(),
            scope: ScopeBuilder::default(),
        };
        let root = b.fields(&plan.fields)?;
        let scope = std::mem::take(&mut b.scope).finish();
        // Routing fields are only checked here (`extract` reads them). Their paths join no scope.
        for r in &plan.routing {
            b.leaf(&r.scalar)?;
        }
        Ok(Program {
            root,
            scope,
            keys: b.keys,
        })
    }

    /// Distinct path keys. A `Key`'s `tag` is its index here.
    pub fn keys(&self) -> &[String] {
        &self.keys
    }
}

/// The value at `path`, `None` when it or any step is missing or null.
fn read<D: Doc>(doc: Option<&D>, path: &[Step]) -> Option<D> {
    let mut cur = doc?.clone();
    for s in path {
        cur = match s {
            Step::Key(k) => cur.get(k)?,
            Step::Idx(i) => cur.idx(*i)?,
        };
    }
    (!cur.is_null()).then_some(cur)
}

/// One document being read in its scope, with the prefixes resolved so far.
struct Frame<'s, D> {
    doc: Option<D>,
    scope: &'s Scope,
    /// Per `Scope::slot`: `None` not resolved yet, `Some(v)` resolved. Empty for a one-field read
    /// (`execute_field`), where it does not pay.
    cache: Vec<Option<Option<D>>>,
}

impl<'s, D: Doc> Frame<'s, D> {
    fn new(doc: Option<D>, scope: &'s Scope, cache: bool) -> Self {
        let cache = match cache {
            true => vec![None; scope.slots],
            false => Vec::new(),
        };
        Frame { doc, scope, cache }
    }

    /// The value at prefix node `n`, null or not. Resolved once when cached.
    fn prefix(&mut self, n: At) -> Option<D> {
        if n == 0 {
            return self.doc.clone();
        }
        let scope = self.scope;
        let slot = scope.slot[n - 1] as usize;
        if let Some(Some(v)) = self.cache.get(slot) {
            return v.clone();
        }
        let (parent, step) = &scope.nodes[n - 1];
        let v = self.prefix(*parent).and_then(|p| step_of(&p, step));
        if let Some(c) = self.cache.get_mut(slot) {
            *c = Some(v.clone());
        }
        v
    }

    /// The value read at node `n`, `None` when it or any step is missing or null, as `read`.
    fn read(&mut self, n: At) -> Option<D> {
        let v = match n {
            0 => self.doc.clone(),
            n => {
                let scope = self.scope;
                let (parent, step) = &scope.nodes[n - 1];
                self.prefix(*parent).and_then(|p| step_of(&p, step))
            }
        };
        v.filter(|d| !d.is_null())
    }
}

fn step_of<D: Doc>(d: &D, s: &Step) -> Option<D> {
    match s {
        Step::Key(k) => d.get(k),
        Step::Idx(i) => d.idx(*i),
    }
}

/// A `default` as a borrowed `Value`.
fn json_value(v: &serde_json::Value) -> Value<'_> {
    use serde_json::Value as J;
    match v {
        J::Bool(b) => Value::Bool(*b),
        J::Number(n) => n
            .as_i64()
            .map_or_else(|| Value::Float(n.as_f64().unwrap_or(f64::NAN)), Value::Int),
        J::String(s) => Value::Str(s.into()),
        J::Null | J::Array(_) | J::Object(_) => Value::Null,
    }
}

/// Most nodes `json` copies. A host object that shares its children (`{"a": x, "b": x}` down a
/// chain) has exponentially many paths though it is small. This bounds the walk.
pub(crate) const MAX_NODES: usize = 1_000_000;

/// A document value as JSON, for a custom transform's container input. `None` past `MAX_DEPTH`
/// nested containers (a host object that contains itself) or past `budget` values (see `MAX_NODES`).
fn json<D: Doc>(d: &D, enclosing: usize, budget: &mut usize) -> Option<J> {
    copy(d, enclosing, budget).map(to_json)
}

fn to_json(o: Out) -> J {
    match o {
        Out::Bool(b) => J::Bool(b),
        Out::Int(i) => J::from(i),
        Out::BigInt(s) => s.parse::<f64>().map_or(J::Null, J::from),
        Out::Float(f) => J::from(f),
        Out::Str(s) => J::String(s),
        Out::List(xs) => J::Array(xs.into_iter().map(to_json).collect()),
        Out::Map(m) => J::Object(m.into_iter().map(|(k, v)| (k, to_json(v))).collect()),
        // `copy` builds neither.
        Out::Null | Out::Obj(_) => J::Null,
    }
}

fn apply<'v>(custom: &[Arc<dyn Transform>], t: &TxRef, v: Value<'v>) -> Value<'v> {
    match t {
        TxRef::Builtin(b) => b.apply(v, None),
        TxRef::Custom { index, arg, .. } => match custom.get(*index) {
            Some(f) => {
                catch_unwind(AssertUnwindSafe(|| f.apply(v, arg.as_ref()))).unwrap_or(Value::Null)
            }
            None => Value::Null,
        },
    }
}

/// `pipe` over a document value. A custom transform first in line gets a container as JSON.
fn pipe_doc<'d, D: Doc>(custom: &[Arc<dyn Transform>], tx: &[TxRef], d: &'d D) -> Value<'d> {
    match (d.leaf(), tx.split_first()) {
        (Value::Container, Some((first @ TxRef::Custom { .. }, rest))) => {
            let mut budget = MAX_NODES;
            pipe(
                custom,
                rest,
                container(custom, first, json(d, 0, &mut budget)),
            )
        }
        (v, _) => pipe(custom, tx, v),
    }
}

/// A custom transform over a container input (`None`: too deep, null). Callers handle a default
/// transform first.
fn container(custom: &[Arc<dyn Transform>], t: &TxRef, v: Option<J>) -> Value<'static> {
    let (TxRef::Custom { index, arg, .. }, Some(v)) = (t, v) else {
        return Value::Null;
    };
    match custom.get(*index) {
        Some(f) => catch_unwind(AssertUnwindSafe(|| f.apply_container(&v, arg.as_ref())))
            .unwrap_or(Value::Null),
        None => Value::Null,
    }
}

/// Transforms in order, stopping at the first null.
fn pipe<'v>(custom: &[Arc<dyn Transform>], tx: &[TxRef], mut v: Value<'v>) -> Value<'v> {
    for t in tx {
        if v.is_null() {
            break;
        }
        v = apply(custom, t, v);
    }
    v
}

fn check(validator: Option<Semantic>, v: Value<'_>) -> Value<'_> {
    match validator {
        Some(s) if !v.is_null() => validate(s, v),
        _ => v,
    }
}

/// A present value through a scalar field's transforms, then its routing validator. `Null` is a
/// rejection. `Container` means the value goes out as it is. `map` and `extract` share it.
pub(crate) fn transform<'d, D: Doc>(
    custom: &[Arc<dyn Transform>],
    tx: &[TxRef],
    validator: Option<Semantic>,
    d: &'d D,
) -> Value<'d> {
    check(validator, pipe_doc(custom, tx, d))
}

/// `transform` for `from` with several paths (`None` where a path is absent). The first step sees
/// the list: only `join` reads one, a custom transform sees a container.
pub(crate) fn transform_many<'d, D: Doc>(
    custom: &[Arc<dyn Transform>],
    tx: &[TxRef],
    validator: Option<Semantic>,
    docs: &'d [Option<D>],
) -> Value<'d> {
    let vals: Vec<Value> = docs
        .iter()
        .map(|d| d.as_ref().map_or(Value::Null, |d| d.leaf()))
        .collect();
    let v = match tx.split_first() {
        None => Value::Null, // a routing validator alone rejects a list
        Some((TxRef::Builtin(b), rest)) => pipe(custom, rest, b.apply_list(&vals)),
        Some((first, rest)) => {
            let mut budget = MAX_NODES;
            let items = docs.iter().map(|d| {
                d.as_ref()
                    .map_or(Some(J::Null), |d| json(d, 1, &mut budget))
            });
            let items = items.collect::<Option<_>>().map(J::Array);
            pipe(custom, rest, container(custom, first, items))
        }
    };
    check(validator, v)
}

/// Runs `program` over `doc` (`None` reads as absent everywhere) and counts one response in `stats`.
/// `custom` is aligned with `Plan::custom_names`. A transform that panics gives null, an error.
pub fn execute<D: Doc, S: Sink>(
    program: &Program,
    doc: Option<&D>,
    sink: &mut S,
    stats: &Stats,
    custom: &[Arc<dyn Transform>],
) -> S::Out {
    stats.response();
    let mut f = Frame::new(doc.cloned(), &program.scope, true);
    Exec {
        sink,
        stats,
        custom,
        filled: 0,
    }
    .object(&program.root, &mut f)
}

/// One field of `program` over `doc`, as `execute` maps it, counting no stats. `path` is the
/// chain of field indices from the root (`[i, j]`: member `j` of the object field `i`). `[]` is the
/// whole root object. `None` when `path` names no field.
pub fn execute_field<D: Doc, S: Sink>(
    program: &Program,
    path: &[usize],
    doc: Option<&D>,
    sink: &mut S,
    custom: &[Arc<dyn Transform>],
) -> Option<S::Out> {
    let quiet = Stats::none();
    let mut e = Exec {
        sink,
        stats: &quiet,
        custom,
        filled: 0,
    };
    let mut f = Frame::new(doc.cloned(), &program.scope, false);
    let Some((&last, parents)) = path.split_last() else {
        return Some(e.object(&program.root, &mut f));
    };
    let mut level = &program.root;
    for &i in parents {
        let mut op = &level.get(i)?.1;
        if let Op::OmitNull(o) = op {
            op = o;
        }
        match op {
            Op::Object(c) => level = c,
            _ => return None,
        }
    }
    Some(e.op(&level.get(last)?.1, &mut f))
}

struct Exec<'e, S> {
    sink: &'e mut S,
    stats: &'e Stats,
    custom: &'e [Arc<dyn Transform>],
    /// How many values have been written as something other than null. `OmitNull` compares it
    /// before and after its field.
    filled: usize,
}

impl<S: Sink> Exec<'_, S> {
    fn object<D: Doc>(&mut self, fields: &[(usize, Op)], f: &mut Frame<'_, D>) -> S::Out {
        self.sink.open(true);
        let mut members = Vec::with_capacity(fields.len());
        for (name, op) in fields {
            match op {
                Op::OmitNull(inner) => {
                    let (mark, filled) = (self.sink.mark(), self.filled);
                    self.sink.next(Some(*name), members.is_empty());
                    let out = self.op(inner, f);
                    if self.filled == filled {
                        self.sink.rewind(mark);
                    } else {
                        members.push((*name, out));
                    }
                }
                // Known before anything is written: the key is left out.
                Op::List {
                    from,
                    stat,
                    ops:
                        ListOps {
                            missing: Empty::Missing,
                            ..
                        },
                    ..
                } if f.read(*from).is_none() => self.stats.miss(*stat),
                op => {
                    self.sink.next(Some(*name), members.is_empty());
                    members.push((*name, self.op(op, f)));
                }
            }
        }
        self.sink.object(members)
    }

    fn op<D: Doc>(&mut self, op: &Op, f: &mut Frame<'_, D>) -> S::Out {
        match op {
            Op::Null => self.sink.scalar(Value::Null),
            Op::Object(c) => self.object(c, f),
            Op::Scalar(l) => self.scalar(l, f),
            Op::List {
                from,
                kind,
                stat,
                ops,
            } => self.list(*from, kind, *stat, *ops, f),
            Op::Const(v) => {
                self.filled += !v.is_null() as usize;
                self.sink.passthrough(&v)
            }
            Op::OmitNull(inner) => self.op(inner, f),
        }
    }

    fn scalar<D: Doc>(&mut self, l: &Leaf, f: &mut Frame<'_, D>) -> S::Out {
        // None: absent. Some(None): rejected.
        let out = match &l.src {
            Src::From(p) => f.read(*p).map(|d| self.single(l, &d)),
            Src::Coalesce(ps) => ps
                .iter()
                .find_map(|p| f.read(*p))
                .map(|d| self.single(l, &d)),
            Src::Many(ps) => self.many(l, f, ps),
        };
        match out {
            Some(Some(o)) => {
                self.filled += 1;
                o
            }
            out => {
                if out.is_some() {
                    self.stats.error(l.stat);
                }
                self.stats.miss(l.stat);
                self.filled += l.default.is_some() as usize;
                let v = l.default.as_ref().map_or(Value::Null, json_value);
                self.sink.scalar(v)
            }
        }
    }

    fn single<D: Doc>(&mut self, l: &Leaf, d: &D) -> Option<S::Out> {
        if l.bare() {
            return Some(self.sink.passthrough(d));
        }
        let v = transform(self.custom, &l.tx, l.validator, d);
        match v {
            Value::Null => None,
            // A custom transform handing back its container input: the input, as it is.
            Value::Container => Some(self.sink.passthrough(d)),
            v => Some(self.sink.scalar(v)),
        }
    }

    fn tuple<D: Doc>(&mut self, docs: &[Option<D>]) -> S::Out {
        self.sink.open(false);
        let items = (docs.iter().enumerate())
            .map(|(n, d)| {
                self.sink.next(None, n == 0);
                match d {
                    Some(d) => self.sink.passthrough(d),
                    None => self.sink.scalar(Value::Null),
                }
            })
            .collect();
        self.sink.list(items)
    }

    fn many<D: Doc>(
        &mut self,
        l: &Leaf,
        f: &mut Frame<'_, D>,
        ps: &[At],
    ) -> Option<Option<S::Out>> {
        // one Vec per read, as codegen.ts allocates one array. Only multi-path `from` pays.
        let docs: Vec<Option<D>> = ps.iter().map(|p| f.read(*p)).collect();
        if docs.iter().all(Option::is_none) {
            return None;
        }
        if l.bare() {
            return Some(Some(self.tuple(&docs)));
        }
        let v = transform_many(self.custom, &l.tx, l.validator, &docs);
        Some(match v {
            Value::Null => None,
            Value::Container => Some(self.tuple(&docs)),
            v => Some(self.sink.scalar(v)),
        })
    }

    fn list<D: Doc>(
        &mut self,
        from: At,
        kind: &Kind,
        stat: usize,
        ops: ListOps,
        f: &mut Frame<'_, D>,
    ) -> S::Out {
        let Some(x) = f.read(from) else {
            self.stats.miss(stat);
            return self.empty(ops.missing);
        };
        let Some(n) = x.len() else {
            self.stats.error(stat);
            self.stats.miss(stat);
            return self.sink.scalar(Value::Null);
        };
        if n == 0 {
            return self.empty(ops.empty);
        }
        self.filled += 1;
        if let Kind::Item(..) = kind {
            self.stats.elements(stat, n);
        }
        self.sink.open(false);
        let items = (0..n)
            .map(|i| {
                self.sink.next(None, i == 0);
                match kind {
                    Kind::Item(fields, scope) => {
                        self.object(fields, &mut Frame::new(x.item(i), scope, true))
                    }
                    Kind::Elems(path, tx) => self.element(read(x.item(i).as_ref(), path), tx, stat),
                }
            })
            .collect();
        self.sink.list(items)
    }

    /// `[]` for `Empty::List`, null otherwise.
    fn empty(&mut self, e: Empty) -> S::Out {
        if e != Empty::List {
            return self.sink.scalar(Value::Null);
        }
        self.filled += 1;
        self.sink.open(false);
        self.sink.list(Vec::new())
    }

    fn element<D: Doc>(&mut self, d: Option<D>, tx: &[TxRef], stat: usize) -> S::Out {
        let Some(d) = d else {
            return self.sink.scalar(Value::Null);
        };
        if tx.is_empty() {
            return self.sink.passthrough(&d);
        }
        match pipe_doc(self.custom, tx, &d) {
            Value::Null => {
                self.stats.error(stat);
                self.sink.scalar(Value::Null)
            }
            Value::Container => self.sink.passthrough(&d),
            v => self.sink.scalar(v),
        }
    }
}

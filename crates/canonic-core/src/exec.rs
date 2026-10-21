//! The executor: a tree walk over `Program`, the plan with its keys interned, reproducing what the
//! JavaScript codegen.ts emits. Generic over the document (`Doc`) and the output (`Sink`).
//!
//! Rules, from codegen.ts `fieldExpr` and `array`:
//! - a missing or null source is null and a miss, never an error;
//! - a present value whose transforms (then the routing validator) give null is an error and a miss;
//!   either way `default` then applies, and still counts as a miss;
//! - `coalesce` takes the first present value and does not retry after a rejection;
//! - `from` with several paths hands the transforms a list (null only if every path is absent);
//!   without transforms the field is that list;
//! - a list `from` that is present but not an array is null, an error and a miss on the list slot;
//! - `item` maps each element; a plucked or plain element that a transform rejects is null in place
//!   and one error on the list slot, an absent element is just null.

use crate::canonical::Semantic;
use crate::doc::{Doc, Key};
use crate::json::MAX_DEPTH;
use crate::plan::{CompileError, Field, ListKind, Named, Plan, Scalar, TxRef};
use crate::sink::Sink;
use crate::spec::{self, Source};
use crate::stats::Stats;
use crate::transforms::{Transform, validate};
use crate::value::Value;
use serde_json::Value as J;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

enum Step {
    Key(Key),
    Idx(usize),
}

type Path = Vec<Step>;

enum Src {
    From(Path),
    Many(Vec<Path>),
    Coalesce(Vec<Path>),
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
    /// An item list maps each element with `Item`; otherwise each element is read at `Elems.0`
    /// (empty for a plain list) and transformed.
    List {
        from: Path,
        kind: Kind,
        stat: usize,
    },
}

enum Kind {
    Item(Vec<(usize, Op)>),
    Elems(Path, Vec<TxRef>),
}

/// A `Plan` compiled for execution: path keys become `Key`s, every index is range-checked.
pub struct Program {
    root: Vec<(usize, Op)>,
    keys: Vec<String>,
}

struct Builder<'p> {
    plan: &'p Plan,
    keys: Vec<String>,
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
                    Step::Key(Key {
                        s: k.clone(),
                        tag: tag as u64,
                    })
                }
            })
            .collect()
    }

    fn leaf(&mut self, s: &Scalar) -> Result<Leaf, CompileError> {
        let src = match &s.source {
            Source::From(p) => Src::From(self.path(p)),
            Source::FromMany(ps) => Src::Many(ps.iter().map(|p| self.path(p)).collect()),
            Source::Coalesce(ps) => Src::Coalesce(ps.iter().map(|p| self.path(p)).collect()),
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
                let op = match &f.value {
                    Field::Null => Op::Null,
                    Field::Scalar(s) => Op::Scalar(self.leaf(s)?),
                    Field::Object(c) => Op::Object(self.fields(c)?),
                    Field::List { from, kind, stat } => Op::List {
                        from: self.path(from),
                        kind: match kind {
                            ListKind::Item(c) => Kind::Item(self.fields(c)?),
                            ListKind::Pluck(p, tx) => Kind::Elems(self.path(p), self.tx(tx)?),
                            ListKind::Plain(tx) => Kind::Elems(Vec::new(), self.tx(tx)?),
                        },
                        stat: self.stat(*stat)?,
                    },
                };
                Ok((f.name_idx, op))
            })
            .collect()
    }
}

impl Program {
    /// Compiles `plan`, rejecting any name, stat or custom transform index out of range.
    pub fn new(plan: &Plan) -> Result<Program, CompileError> {
        let mut b = Builder {
            plan,
            keys: Vec::new(),
        };
        let root = b.fields(&plan.fields)?;
        for r in &plan.routing {
            b.leaf(&r.scalar)?;
        }
        Ok(Program { root, keys: b.keys })
    }

    /// Distinct path keys; a `Key`'s `tag` is its index here.
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
/// chain) has exponentially many paths though it is small; this bounds the walk.
const MAX_NODES: usize = 1_000_000;

/// A document value as JSON, for a custom transform's container input. `None` past `MAX_DEPTH` (a
/// host object that contains itself) or past `budget` nodes (see `MAX_NODES`).
fn json<D: Doc>(d: &D, depth: usize, budget: &mut usize) -> Option<J> {
    if depth > MAX_DEPTH || *budget == 0 {
        return None;
    }
    *budget -= 1;
    if let Some(n) = d.len() {
        let items = (0..n).map(|i| {
            d.item(i)
                .map_or(Some(J::Null), |x| json(&x, depth + 1, budget))
        });
        return items.collect::<Option<_>>().map(J::Array);
    }
    if let Some(m) = d.members() {
        let members = m
            .into_iter()
            .map(|(k, v)| Some((k.into_owned(), json(&v, depth + 1, budget)?)));
        return members.collect::<Option<_>>().map(J::Object);
    }
    Some(match d.leaf() {
        Value::Bool(b) => J::Bool(b),
        Value::Int(i) => J::from(i),
        Value::BigInt(s) => s.parse::<f64>().map_or(J::Null, J::from),
        Value::Float(f) => J::from(f),
        Value::Str(s) => J::String(s.into_owned()),
        Value::Null | Value::Container => J::Null,
    })
}

/// Runs `program` over `doc` (`None` reads as absent everywhere) and counts one response in `stats`.
/// `custom` is aligned with `Plan::custom_names`; a transform that panics gives null, an error.
pub fn execute<D: Doc, S: Sink>(
    program: &Program,
    doc: Option<&D>,
    sink: &mut S,
    stats: &Stats,
    custom: &[Arc<dyn Transform>],
) -> S::Out {
    stats.response();
    Exec {
        sink,
        stats,
        custom,
    }
    .object(&program.root, doc)
}

struct Exec<'e, S> {
    sink: &'e mut S,
    stats: &'e Stats,
    custom: &'e [Arc<dyn Transform>],
}

impl<S: Sink> Exec<'_, S> {
    fn object<D: Doc>(&mut self, fields: &[(usize, Op)], doc: Option<&D>) -> S::Out {
        let members = fields
            .iter()
            .map(|(name, op)| (*name, self.op(op, doc)))
            .collect();
        self.sink.object(members)
    }

    fn op<D: Doc>(&mut self, op: &Op, doc: Option<&D>) -> S::Out {
        match op {
            Op::Null => self.sink.scalar(Value::Null),
            Op::Object(c) => self.object(c, doc),
            Op::Scalar(l) => self.scalar(l, doc),
            Op::List { from, kind, stat } => self.list(from, kind, *stat, doc),
        }
    }

    fn apply<'v>(&self, t: &TxRef, v: Value<'v>) -> Value<'v> {
        match t {
            TxRef::Builtin(b) => b.apply(v, None),
            TxRef::Custom { index, arg, .. } => match self.custom.get(*index) {
                Some(f) => catch_unwind(AssertUnwindSafe(|| f.apply(v, arg.as_ref())))
                    .unwrap_or(Value::Null),
                None => Value::Null,
            },
        }
    }

    /// `pipe` over a document value; a custom transform first in line gets a container as JSON.
    fn pipe_doc<'d, D: Doc>(&self, tx: &[TxRef], d: &'d D) -> Value<'d> {
        match (d.leaf(), tx.split_first()) {
            (Value::Container, Some((first @ TxRef::Custom { .. }, rest))) => {
                let mut budget = MAX_NODES;
                self.pipe(rest, self.container(first, json(d, 0, &mut budget)))
            }
            (v, _) => self.pipe(tx, v),
        }
    }

    /// A custom transform over a container input (`None`: too deep, null); callers handle a default
    /// transform first.
    fn container(&self, t: &TxRef, v: Option<J>) -> Value<'static> {
        let (TxRef::Custom { index, arg, .. }, Some(v)) = (t, v) else {
            return Value::Null;
        };
        match self.custom.get(*index) {
            Some(f) => catch_unwind(AssertUnwindSafe(|| f.apply_container(&v, arg.as_ref())))
                .unwrap_or(Value::Null),
            None => Value::Null,
        }
    }

    /// Transforms in order, stopping at the first null.
    fn pipe<'v>(&self, tx: &[TxRef], mut v: Value<'v>) -> Value<'v> {
        for t in tx {
            if v.is_null() {
                break;
            }
            v = self.apply(t, v);
        }
        v
    }

    fn check<'v>(validator: Option<Semantic>, v: Value<'v>) -> Value<'v> {
        match validator {
            Some(s) if !v.is_null() => validate(s, v),
            _ => v,
        }
    }

    fn scalar<D: Doc>(&mut self, l: &Leaf, doc: Option<&D>) -> S::Out {
        // None: absent. Some(None): rejected.
        let out = match &l.src {
            Src::From(p) => read(doc, p).map(|d| self.single(l, &d)),
            Src::Coalesce(ps) => ps
                .iter()
                .find_map(|p| read(doc, p))
                .map(|d| self.single(l, &d)),
            Src::Many(ps) => self.many(l, doc, ps),
        };
        match out {
            Some(Some(o)) => o,
            out => {
                if out.is_some() {
                    self.stats.error(l.stat);
                }
                self.stats.miss(l.stat);
                let v = l.default.as_ref().map_or(Value::Null, json_value);
                self.sink.scalar(v)
            }
        }
    }

    fn single<D: Doc>(&mut self, l: &Leaf, d: &D) -> Option<S::Out> {
        if l.bare() {
            return Some(self.sink.passthrough(d));
        }
        let v = Self::check(l.validator, self.pipe_doc(&l.tx, d));
        match v {
            Value::Null => None,
            // A custom transform handing back its container input: the input, as it is.
            Value::Container => Some(self.sink.passthrough(d)),
            v => Some(self.sink.scalar(v)),
        }
    }

    fn tuple<D: Doc>(&mut self, docs: &[Option<D>]) -> S::Out {
        let items = docs
            .iter()
            .map(|d| match d {
                Some(d) => self.sink.passthrough(d),
                None => self.sink.scalar(Value::Null),
            })
            .collect();
        self.sink.list(items)
    }

    fn many<D: Doc>(&mut self, l: &Leaf, doc: Option<&D>, ps: &[Path]) -> Option<Option<S::Out>> {
        // one Vec per read, as codegen.ts allocates one array; only multi-path `from` pays.
        let docs: Vec<Option<D>> = ps.iter().map(|p| read(doc, p)).collect();
        if docs.iter().all(Option::is_none) {
            return None;
        }
        if l.bare() {
            return Some(Some(self.tuple(&docs)));
        }
        let vals: Vec<Value> = docs
            .iter()
            .map(|d| d.as_ref().map_or(Value::Null, |d| d.leaf()))
            .collect();
        // The first step sees the list. Only `join` reads one; a custom transform sees a container.
        let v = match l.tx.split_first() {
            None => Value::Null, // a routing validator alone rejects a list
            Some((TxRef::Builtin(b), rest)) => self.pipe(rest, b.apply_list(&vals)),
            Some((first, rest)) => {
                let mut budget = MAX_NODES;
                let items = docs.iter().map(|d| {
                    d.as_ref()
                        .map_or(Some(J::Null), |d| json(d, 1, &mut budget))
                });
                let items = items.collect::<Option<_>>().map(J::Array);
                self.pipe(rest, self.container(first, items))
            }
        };
        let v = Self::check(l.validator, v);
        Some(match v {
            Value::Null => None,
            Value::Container => Some(self.tuple(&docs)),
            v => Some(self.sink.scalar(v)),
        })
    }

    fn list<D: Doc>(&mut self, from: &[Step], kind: &Kind, stat: usize, doc: Option<&D>) -> S::Out {
        let Some(x) = read(doc, from) else {
            self.stats.miss(stat);
            return self.sink.scalar(Value::Null);
        };
        let Some(n) = x.len() else {
            self.stats.error(stat);
            self.stats.miss(stat);
            return self.sink.scalar(Value::Null);
        };
        let items = match kind {
            Kind::Item(fields) => {
                self.stats.elements(stat, n);
                (0..n)
                    .map(|i| self.object(fields, x.item(i).as_ref()))
                    .collect()
            }
            Kind::Elems(path, tx) => (0..n)
                .map(|i| self.element(read(x.item(i).as_ref(), path), tx, stat))
                .collect(),
        };
        self.sink.list(items)
    }

    fn element<D: Doc>(&mut self, d: Option<D>, tx: &[TxRef], stat: usize) -> S::Out {
        let Some(d) = d else {
            return self.sink.scalar(Value::Null);
        };
        if tx.is_empty() {
            return self.sink.passthrough(&d);
        }
        match self.pipe_doc(tx, &d) {
            Value::Null => {
                self.stats.error(stat);
                self.sink.scalar(Value::Null)
            }
            Value::Container => self.sink.passthrough(&d),
            v => self.sink.scalar(v),
        }
    }
}

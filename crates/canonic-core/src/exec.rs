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
//! - Language 2: an object whose `present` path is missing or null is null, and every field
//!   inside it is a miss. With `if_empty` or an object `x-empty` of null or `{}`, an
//!   object that writes nothing but null is rewound to that. An `x-empty` placeholder (`[]` or `{}`
//!   for an unmapped field or a missing list) is not a fill for this check.
//! - Language 2: a list with `wrap`, flatten, `filter`, `unique`, `sort_by` or `limit`
//!   collects its elements first (`collect`). Its `empty` applies to what is left. Other lists
//!   stream the source array.
//! - Language 2: a `group_by` list groups what `collect` leaves (`group::groups`) and maps
//!   each group, its first element as the element and the group as `$group`. An aggregate folds
//!   one path across an array or the group. A missing array is empty, one that is not an array
//!   is null and an error, and so is a value an aggregate cannot fold.
//! - Language 2: a path that starts with `$input` reads the request context (`Frame::ctx`),
//!   in a plan with `context` set, and in every condition. A `when` that fails gives null, or
//!   leaves the key out with `omit`, before anything under it is read. Its fields are misses.
//! - Language 2 (R48): under strict shape the resolver has put each node's empty form into the
//!   plan (`ListOps`, `Scalar::empty`, `When::empty`, `Container::strict`). The null paths write
//!   it as a placeholder that is no fill, and count the same misses and errors.

use crate::canonical::{Empty, Node, Semantic, empty_form};
use crate::cond::{Cond, Operand, Rank, holds, key_text};
use crate::doc::{Doc, Key, KeyPath as Path, KeyStep as Step, read, step as step_of};
use crate::group::{Agg, AggOp, aggregate, groups};
use crate::plan::{self, CompileError, Field, ListKind, ListOps, Named, Plan, Scalar, TxRef};
use crate::sink::{Out, Sink, copy};
use crate::spec::{self, is_group};
use crate::stats::Stats;
use crate::transforms::{Builtin, Transform, conform, validate};
use crate::value::Value;
use serde_json::Value as J;
use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

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
    /// The node of `$input`, which reads the context. 0 (the document) when no path reads it.
    input: At,
}

const NONE: u32 = u32::MAX;

/// A `Scope` while the plan is compiled.
#[derive(Default)]
struct ScopeBuilder {
    nodes: Vec<(At, Step)>,
    index: HashMap<(At, StepId), At>,
    /// Reads that pass through each node as a strict prefix.
    through: Vec<u32>,
    input: At,
}

#[derive(PartialEq, Eq, Hash)]
enum StepId {
    Key(u64),
    Idx(usize),
}

impl StepId {
    /// A parent's children are all exact or all case-insensitive (`Builder::path`), so the tag
    /// alone tells two keys apart.
    fn of(s: &Step) -> StepId {
        match s {
            Step::Key(k) | Step::KeyCi(k) => StepId::Key(k.tag),
            Step::Idx(i) => StepId::Idx(*i),
        }
    }
}

impl ScopeBuilder {
    fn at(&mut self, steps: Path) -> At {
        let mut node = 0;
        let last = steps.len();
        for (i, step) in steps.into_iter().enumerate() {
            if node > 0 && i < last {
                self.through[node - 1] += 1;
            }
            node = match self.index.get(&(node, StepId::of(&step))) {
                Some(&n) => n,
                None => {
                    let id = StepId::of(&step);
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
            input: self.input,
        }
    }
}

enum Src {
    From(At),
    Many(Vec<At>),
    Coalesce(Vec<At>),
    /// R56: an entry is taken when it is present and its condition holds.
    CoalesceWhen(Vec<Entry2>),
    Computed(Vec<Item>),
}

/// A `coalesce` entry (R56, R57): where it reads, and the condition it must pass.
type Entry2 = (At, Option<Cond<Path>>);

/// An entry of a computed `from` list (R52).
enum Item {
    At(At),
    Rule(Box<Inner>),
}

/// A computed entry: a scalar rule with no stat. `many`: `items` is a list the first transform
/// folds, otherwise it holds the one path.
struct Inner {
    items: Vec<Item>,
    many: bool,
    tx: Vec<TxRef>,
}

/// What an entry of a computed list gave. `Val` is a computed entry that gave a value.
enum Entry<D> {
    Missing,
    Doc(D),
    Val(Value<'static>),
}

struct Leaf {
    src: Src,
    tx: Vec<TxRef>,
    validator: Option<Semantic>,
    default: Option<serde_json::Value>,
    /// R54: the first transform runs on a missing source, with null.
    on_missing: bool,
    stat: usize,
    /// R48: written in place of null when there is no default.
    empty: Option<J>,
}

impl Leaf {
    /// No transform and no validator: the value goes out as it is.
    fn bare(&self) -> bool {
        self.tx.is_empty() && self.validator.is_none()
    }
}

/// Where a list or an aggregate reads its elements: an array, or the group (`$group`).
enum Input {
    At(At),
    Group,
}

struct Fold {
    op: AggOp,
    input: Input,
    /// Only `flatten` is set.
    ops: ListOps<Path>,
    pluck: Path,
    propagate: bool,
    sep: String,
    stat: usize,
}

/// A container transform (A10, `Field::Container`).
struct Whole {
    from: Input,
    tx: TxRef,
    node: Node,
    on_missing: bool,
    stat: usize,
    /// R48: a failure writes `empty` too, and `conform` fills strict empty forms.
    strict: bool,
    /// The node's empty form, which a missing source writes. Null for no key.
    empty: J,
}

enum Op {
    Null,
    Container(Box<Whole>),
    Scalar(Leaf),
    Aggregate(Box<Fold>),
    Object(Vec<(usize, Op)>),
    /// An item list maps each element with `Item`. Otherwise each element is read at `Elems.0`
    /// (empty for a plain list) and transformed.
    List {
        from: Input,
        kind: Kind,
        stat: usize,
        ops: Box<ListOps<Path>>,
        /// R57: when set, the source is the first entry that passes (`pick`), not `from`.
        coalesce: Vec<Entry2>,
    },
    /// Written as it is. Counts no miss, so it is always a fill. The stat slot is `None` for an
    /// unmapped field's `x-empty`, a placeholder that does not make its object filled.
    Const(J, Option<usize>),
    /// An object with `present` paths (all of them `Cond::Present`), or `if_empty`: what it
    /// writes (null or `{}`) when it writes nothing but null.
    Optional {
        members: Vec<(usize, Op)>,
        present: Cond<Path>,
        if_empty: Option<J>,
    },
    /// Left out of its object when it writes nothing but null.
    OmitNull(Box<Op>),
    /// `op` when `cond` holds, else `empty` (R48) or null, or no key with `omit`.
    When {
        cond: Cond<Path>,
        omit: bool,
        op: Box<Op>,
        empty: Option<J>,
    },
    /// R55: the members of the first branch whose condition holds (`None`: always). No branch, or
    /// one that writes nothing but null, writes `empty`.
    Union {
        branches: Vec<Arm>,
        empty: J,
    },
}

/// A union branch: its condition (`None`: always) and its members.
type Arm = (Option<Cond<Path>>, Vec<(usize, Op)>);

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
    /// `Plan::reject`, read against the root.
    reject: Vec<Cond<Path>>,
    /// Some path reads `$input`, the request context.
    input: bool,
    /// `Plan::array`: the output is the one root field's value, not an object.
    array: bool,
}

struct Builder<'p> {
    plan: &'p Plan,
    keys: Vec<String>,
    scope: ScopeBuilder,
    /// A path read so far reads `$input`.
    input: bool,
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

    /// `p` with interned keys. In a case-insensitive plan its keys are `KeyCi`, except on a
    /// `$input` path: the context keeps exact keys.
    fn path(&mut self, p: &spec::Path) -> Path {
        let ci = self.plan.case_insensitive
            && !matches!(p.first(), Some(spec::Step::Key(k)) if k == "$input");
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
                    let key = Key::new(k.clone(), tag as u64);
                    match ci {
                        true => Step::KeyCi(key),
                        false => Step::Key(key),
                    }
                }
            })
            .collect()
    }

    fn cond(&mut self, c: &Cond) -> Cond<Path> {
        // A condition path that starts with `$input` always reads the context (`cond::holds`).
        let input = |p: &spec::Path| matches!(p.first(), Some(spec::Step::Key(k)) if k == "$input");
        self.input |= c.paths().into_iter().any(input);
        let op = |b: &mut Self, o: &Operand<spec::Path>| match o {
            Operand::Num(n) => Operand::Num(*n),
            Operand::Path(p) => Operand::Path(b.path(p)),
        };
        match c {
            Cond::Present(p, b) => Cond::Present(self.path(p), *b),
            Cond::Eq(p, v) => Cond::Eq(self.path(p), v.clone()),
            Cond::In(p, vs) => Cond::In(self.path(p), vs.clone()),
            Cond::Missing(p) => Cond::Missing(self.path(p)),
            Cond::Empty(p) => Cond::Empty(self.path(p)),
            Cond::All(cs) => Cond::All(cs.iter().map(|c| self.cond(c)).collect()),
            Cond::Any(cs) => Cond::Any(cs.iter().map(|c| self.cond(c)).collect()),
            Cond::Not(c) => Cond::Not(Box::new(self.cond(c))),
            Cond::Matches(p, re) => Cond::Matches(self.path(p), re.clone()),
            Cond::StartsWith(p, t) => Cond::StartsWith(self.path(p), t.clone()),
            Cond::EndsWith(p, t) => Cond::EndsWith(self.path(p), t.clone()),
            Cond::Contains(p, v) => Cond::Contains(self.path(p), v.clone()),
            Cond::Compare(p, cmp, o) => {
                let p = self.path(p);
                Cond::Compare(p, *cmp, op(self, o))
            }
        }
    }

    /// The options with interned paths. They read elements, so they join no scope.
    fn ops(&mut self, o: &ListOps) -> ListOps<Path> {
        ListOps {
            missing: o.missing,
            empty: o.empty,
            wrong: o.wrong,
            wrap: o.wrap,
            flatten: o.flatten.iter().map(|p| self.path(p)).collect(),
            filter: o.filter.as_ref().map(|c| self.cond(c)),
            unique: o.unique.as_ref().map(|p| self.path(p)),
            sort: o.sort.as_ref().map(|(p, desc)| (self.path(p), *desc)),
            limit: o.limit,
            group: o.group.as_ref().map(|(p, c)| (self.path(p), *c)),
            group_also: o.group_also.iter().map(|p| self.path(p)).collect(),
            from_group: o.from_group,
        }
    }

    /// `p` as a node of the current scope. In a plan with `context`, `$input` is the context.
    fn at(&mut self, p: &spec::Path) -> At {
        let steps = self.path(p);
        if self.plan.context && matches!(p.first(), Some(spec::Step::Key(k)) if k == "$input") {
            self.input = true;
            self.scope.input = self.scope.at(steps[..1].to_vec());
        }
        self.scope.at(steps)
    }

    fn entries(&mut self, es: &[(spec::Path, Option<Cond>)]) -> Vec<Entry2> {
        let entry = |b: &mut Self, (p, c): &(spec::Path, Option<Cond>)| {
            (b.at(p), c.as_ref().map(|c| b.cond(c)))
        };
        es.iter().map(|e| entry(self, e)).collect()
    }

    /// The entries of a computed list.
    fn items(&mut self, items: &[plan::Item]) -> Result<Vec<Item>, CompileError> {
        let mut out = Vec::new();
        for item in items {
            out.push(match item {
                plan::Item::Path(p) => Item::At(self.at(p)),
                plan::Item::Rule(r) => {
                    let tx = self.tx(&r.tx)?;
                    let (items, many) = match &r.from {
                        plan::Source::From(p) => (vec![Item::At(self.at(p))], false),
                        plan::Source::FromMany(ps) => {
                            (ps.iter().map(|p| Item::At(self.at(p))).collect(), true)
                        }
                        plan::Source::Computed(xs) => (self.items(xs)?, true),
                        plan::Source::Coalesce(_) | plan::Source::CoalesceWhen(_) => {
                            return Err(self.err("coalesce in a computed source".into()));
                        }
                    };
                    Item::Rule(Box::new(Inner { items, many, tx }))
                }
            });
        }
        Ok(out)
    }

    fn leaf(&mut self, s: &Scalar) -> Result<Leaf, CompileError> {
        let src = match &s.source {
            plan::Source::Computed(xs) => Src::Computed(self.items(xs)?),
            plan::Source::From(p) => Src::From(self.at(p)),
            plan::Source::FromMany(ps) => Src::Many(ps.iter().map(|p| self.at(p)).collect()),
            plan::Source::Coalesce(ps) => Src::Coalesce(ps.iter().map(|p| self.at(p)).collect()),
            plan::Source::CoalesceWhen(es) => Src::CoalesceWhen(self.entries(es)),
        };
        Ok(Leaf {
            src,
            tx: self.tx(&s.tx)?,
            validator: s.validator,
            default: s.default.clone(),
            on_missing: s.on_missing,
            stat: self.stat(s.stat)?,
            empty: s.empty.clone(),
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
            Field::Object {
                members,
                present,
                if_empty,
            } => {
                let members = self.fields(members)?;
                if present.is_empty() && if_empty.is_none() {
                    Op::Object(members)
                } else {
                    let present = present
                        .iter()
                        .map(|p| self.cond(&Cond::Present(p.clone(), true)))
                        .collect();
                    Op::Optional {
                        members,
                        present: Cond::All(present),
                        if_empty: if_empty.map(|e| match e {
                            Empty::Object => J::Object(Default::default()),
                            _ => J::Null,
                        }),
                    }
                }
            }
            Field::Union { branches, empty } => {
                let mut out = Vec::with_capacity(branches.len());
                for b in branches {
                    let when = b.when.as_ref().map(|c| self.cond(c));
                    out.push((when, self.fields(&b.members)?));
                }
                Op::Union {
                    branches: out,
                    empty: match empty {
                        Some(Empty::Object) => J::Object(Default::default()),
                        _ => J::Null,
                    },
                }
            }
            Field::Const(v, stat) => Op::Const(v.clone(), Some(self.stat(*stat)?)),
            Field::Empty(e) => {
                let v = match e {
                    Empty::List => J::Array(Vec::new()),
                    Empty::Object => J::Object(Default::default()),
                    Empty::Null | Empty::Missing => J::Null,
                };
                Op::Const(v, None)
            }
            Field::OmitNull(inner) => Op::OmitNull(Box::new(self.op(inner)?)),
            Field::When {
                when,
                omit,
                field,
                empty,
            } => Op::When {
                cond: self.cond(when),
                omit: *omit,
                op: Box::new(self.op(field)?),
                empty: empty.clone(),
            },
            Field::Container {
                from,
                tx,
                node,
                on_missing,
                stat,
                strict,
            } => Op::Container(Box::new(Whole {
                from: match is_group(from) {
                    true => Input::Group,
                    false => Input::At(self.at(from)),
                },
                tx: self.tx(std::slice::from_ref(tx))?.remove(0),
                node: node.clone(),
                on_missing: *on_missing,
                stat: self.stat(*stat)?,
                strict: *strict,
                empty: empty_form(node, *strict).unwrap_or(J::Null),
            })),
            Field::Aggregate(a, stat) => Op::Aggregate(Box::new(Fold {
                op: a.op,
                input: match is_group(&a.from) {
                    true => Input::Group,
                    false => Input::At(self.at(&a.from)),
                },
                ops: ListOps {
                    flatten: a.flatten.iter().map(|p| self.path(p)).collect(),
                    ..ListOps::default()
                },
                pluck: self.path(&a.pluck),
                propagate: a.propagate,
                sep: a.sep.clone().unwrap_or_else(|| " ".into()),
                stat: self.stat(*stat)?,
            })),
            Field::List {
                from,
                kind,
                stat,
                ops,
                coalesce,
            } => Op::List {
                from: match ops.from_group {
                    true => Input::Group,
                    false => Input::At(self.at(from)),
                },
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
                ops: Box::new(self.ops(ops)),
                coalesce: self.entries(coalesce),
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
            input: false,
        };
        let root = b.fields(&plan.fields)?;
        let reject = plan.reject.iter().map(|r| b.cond(&r.cond)).collect();
        let scope = std::mem::take(&mut b.scope).finish();
        // Routing fields are only checked here (`extract` reads them). Their paths join no scope.
        for r in &plan.routing {
            b.leaf(&r.scalar)?;
        }
        Ok(Program {
            root,
            scope,
            keys: b.keys,
            reject,
            input: b.input,
            array: plan.array,
        })
    }

    /// Whether some path reads `$input`. A plan that does not ignores the context.
    pub fn reads_input(&self) -> bool {
        self.input
    }

    /// The index of the first `reject` condition that holds on `doc`, `$input` reading `ctx`.
    pub fn rejected<D: Doc>(&self, doc: Option<&D>, ctx: Option<&D>) -> Option<usize> {
        self.reject.iter().position(|c| holds(c, doc, None, ctx))
    }

    /// Distinct path keys. A `Key`'s `tag` is its index here.
    pub fn keys(&self) -> &[String] {
        &self.keys
    }
}

/// One document being read in its scope, with the prefixes resolved so far.
struct Frame<'s, D> {
    doc: Option<D>,
    scope: &'s Scope,
    /// Per `Scope::slot`: `None` not resolved yet, `Some(v)` resolved. Empty for a one-field read
    /// (`execute_field`), where it does not pay.
    cache: Vec<Option<Option<D>>>,
    /// Inside a grouped item map: the group, which `$group` reads.
    group: Option<&'s [D]>,
    /// The request context, which `$input` reads.
    ctx: Option<D>,
}

impl<'s, D: Doc> Frame<'s, D> {
    fn new(doc: Option<D>, scope: &'s Scope, cache: bool) -> Self {
        let cache = match cache {
            true => vec![None; scope.slots],
            false => Vec::new(),
        };
        Frame {
            doc,
            scope,
            cache,
            group: None,
            ctx: None,
        }
    }

    /// The value at prefix node `n`, null or not. Resolved once when cached.
    fn prefix(&mut self, n: At) -> Option<D> {
        if n == 0 {
            return self.doc.clone();
        }
        if n == self.scope.input {
            return self.ctx.clone();
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
            n if n == self.scope.input => self.ctx.clone(),
            n => {
                let scope = self.scope;
                let (parent, step) = &scope.nodes[n - 1];
                self.prefix(*parent).and_then(|p| step_of(&p, step))
            }
        };
        v.filter(|d| !d.is_null())
    }
}

/// R56, R57: the first entry that is present, not null and whose condition holds. A list skips
/// an entry that is `[]` too. A condition reads the response (the element, in an item map).
fn pick<D: Doc>(es: &[Entry2], f: &mut Frame<'_, D>, list: bool) -> Option<D> {
    es.iter().find_map(|(at, c)| {
        let d = f.read(*at).filter(|d| !(list && d.len() == Some(0)))?;
        let ok = c
            .as_ref()
            .is_none_or(|c| holds(c, f.doc.as_ref(), None, f.ctx.as_ref()));
        ok.then_some(d)
    })
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

/// The elements of a list source (`None`: missing) after `ops`, in this order: wrap, flatten,
/// filter, unique, sort, limit. `None` when the source is missing or not an array (and not
/// wrapped). Inside a filter, paths read the element, as in an item map, and `$input` reads `ctx`.
pub(crate) fn collect<D: Doc>(
    from: Option<D>,
    ops: &ListOps<Path>,
    ctx: Option<&D>,
) -> Option<Vec<D>> {
    let x = from?;
    let xs: Vec<D> = match x.len() {
        Some(n) => (0..n).filter_map(|i| x.item(i)).collect(),
        None if ops.wrap => vec![x],
        None => return None,
    };
    Some(refine(xs, ops, ctx))
}

/// `collect` after the source: flatten, filter, unique, sort, limit.
fn refine<D: Doc>(mut xs: Vec<D>, ops: &ListOps<Path>, ctx: Option<&D>) -> Vec<D> {
    // A missing, null or non-array value on a flatten path adds nothing.
    for p in &ops.flatten {
        xs = (xs.iter())
            .filter_map(|e| read(Some(e), p))
            .flat_map(|a| (0..a.len().unwrap_or(0)).filter_map(move |i| a.item(i)))
            .collect();
    }
    if let Some(c) = &ops.filter {
        xs.retain(|e| holds(c, Some(e), None, ctx));
    }
    // A null or missing key never matches another, so every such element stays.
    if let Some(p) = &ops.unique {
        let mut seen = std::collections::HashSet::new();
        xs.retain(|e| {
            read(Some(e), p)
                .and_then(|v| key_text(&v))
                .is_none_or(|k| seen.insert(k))
        });
    }
    if let Some((p, desc)) = &ops.sort {
        let mut keyed: Vec<(Rank, D)> = (xs.into_iter())
            .map(|e| (Rank::of(read(Some(&e), p).as_ref()), e))
            .collect();
        keyed.sort_by(|a, b| a.0.cmp(&b.0, *desc));
        xs = keyed.into_iter().map(|(_, e)| e).collect();
    }
    if let Some(n) = ops.limit {
        xs.truncate(n);
    }
    xs
}

/// A document value as JSON, for a custom transform's container input. `None` past `MAX_DEPTH`
/// nested containers (a host object that contains itself) or past `budget` values (see `MAX_NODES`).
pub(crate) fn json<D: Doc>(d: &D, enclosing: usize, budget: &mut usize) -> Option<J> {
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
        // `len` first in line reads an array's length. An object has none.
        (Value::Container, Some((TxRef::Builtin(Builtin::Len), rest))) => match d.len() {
            Some(n) => pipe(custom, rest, Value::Int(n as i64)),
            None => Value::Null,
        },
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
    let vals = docs
        .iter()
        .map(|d| d.as_ref().map_or(Value::Null, |d| d.leaf()))
        .collect();
    let items = || {
        let mut budget = MAX_NODES;
        let items = docs.iter().map(|d| {
            d.as_ref()
                .map_or(Some(J::Null), |d| json(d, 1, &mut budget))
        });
        items.collect::<Option<_>>().map(J::Array)
    };
    fold_list(custom, tx, validator, vals, items)
}

/// `transform_many` for the entries of a computed list.
fn transform_entries<'d, D: Doc>(
    custom: &[Arc<dyn Transform>],
    tx: &[TxRef],
    validator: Option<Semantic>,
    entries: &'d [Entry<D>],
) -> Value<'d> {
    let vals = (entries.iter())
        .map(|e| match e {
            Entry::Missing => Value::Null,
            Entry::Doc(d) => d.leaf(),
            Entry::Val(v) => v.clone(),
        })
        .collect();
    let items = || {
        let mut budget = MAX_NODES;
        let items = entries.iter().map(|e| match e {
            Entry::Missing => Some(J::Null),
            Entry::Doc(d) => json(d, 1, &mut budget),
            Entry::Val(v) => Some(value_json(v)),
        });
        items.collect::<Option<_>>().map(J::Array)
    };
    fold_list(custom, tx, validator, vals, items)
}

fn value_json(v: &Value<'_>) -> J {
    match v {
        Value::Bool(b) => J::Bool(*b),
        Value::Int(i) => J::from(*i),
        Value::BigInt(s) => s.parse::<f64>().map_or(J::Null, J::from),
        Value::Float(f) => J::from(*f),
        Value::Str(s) => J::String(s.to_string()),
        Value::Null | Value::Container => J::Null,
    }
}

/// The first step over a list of values, then the rest. `items` gives the list as JSON for a
/// custom first step.
fn fold_list<'d>(
    custom: &[Arc<dyn Transform>],
    tx: &[TxRef],
    validator: Option<Semantic>,
    vals: Vec<Value<'d>>,
    items: impl FnOnce() -> Option<J>,
) -> Value<'d> {
    let v = match tx.split_first() {
        None => Value::Null, // a routing validator alone rejects a list
        Some((TxRef::Builtin(b), rest)) => pipe(custom, rest, b.apply_list(&vals)),
        Some((first, rest)) => pipe(custom, rest, container(custom, first, items())),
    };
    check(validator, v)
}

/// An entry of a computed list. A computed entry that gives null (or a container) is missing.
fn entry<D: Doc>(custom: &[Arc<dyn Transform>], item: &Item, f: &mut Frame<'_, D>) -> Entry<D> {
    let r = match item {
        Item::At(p) => return f.read(*p).map_or(Entry::Missing, Entry::Doc),
        Item::Rule(r) => r,
    };
    let v = match r.items.as_slice() {
        [Item::At(p)] if !r.many => match f.read(*p) {
            Some(d) => transform(custom, &r.tx, None, &d).into_owned(),
            None => return Entry::Missing,
        },
        items => {
            let es: Vec<Entry<D>> = items.iter().map(|i| entry(custom, i, f)).collect();
            if es.iter().all(|e| matches!(e, Entry::Missing)) {
                return Entry::Missing;
            }
            transform_entries(custom, &r.tx, None, &es).into_owned()
        }
    };
    match v {
        Value::Null | Value::Container => Entry::Missing,
        v => Entry::Val(v),
    }
}

/// Runs `program` over `doc` (`None` reads as absent everywhere) and counts one response in `stats`.
/// `custom` is aligned with `Plan::custom_names`. A transform that panics gives null, an error.
pub fn execute<D: Doc, S: Sink>(
    program: &Program,
    doc: Option<&D>,
    ctx: Option<&D>,
    sink: &mut S,
    stats: &Stats,
    custom: &[Arc<dyn Transform>],
) -> S::Out {
    stats.response();
    let mut f = Frame::new(doc.cloned(), &program.scope, true);
    f.ctx = ctx.cloned();
    Exec {
        sink,
        stats,
        custom,
        filled: 0,
    }
    .root(program, &mut f)
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
        return Some(e.root(program, &mut f));
    };
    let mut level = &program.root;
    for &i in parents {
        let mut op = &level.get(i)?.1;
        while let Op::OmitNull(o) | Op::When { op: o, .. } = op {
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
    /// The root object, or for an array root its one field's value.
    fn root<D: Doc>(&mut self, program: &Program, f: &mut Frame<'_, D>) -> S::Out {
        match (program.array, program.root.first()) {
            (true, Some((_, op))) => self.op(op, f),
            _ => self.object(&program.root, f),
        }
    }

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
                Op::When {
                    cond,
                    omit: true,
                    op,
                    ..
                } if !holds(cond, f.doc.as_ref(), None, f.ctx.as_ref()) => self.miss(op),
                Op::List {
                    from: Input::At(at),
                    stat,
                    ops,
                    ..
                } if ops.missing == Empty::Missing && f.read(*at).is_none() => {
                    self.stats.miss(*stat)
                }
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
            Op::Aggregate(a) => self.fold(a, f),
            Op::Container(w) => self.whole(w, f),
            Op::List {
                from,
                kind,
                stat,
                ops,
                coalesce,
            } => self.list(from, kind, *stat, ops, coalesce, f),
            Op::Const(v, stat) => {
                self.filled += (stat.is_some() && !v.is_null()) as usize;
                self.sink.passthrough(&v)
            }
            Op::OmitNull(inner) => self.op(inner, f),
            Op::When {
                cond, op, empty, ..
            } => {
                if holds(cond, f.doc.as_ref(), None, f.ctx.as_ref()) {
                    return self.op(op, f);
                }
                self.miss(op);
                match empty {
                    Some(e) => self.sink.passthrough(&e),
                    None => self.sink.scalar(Value::Null),
                }
            }
            Op::Union { branches, empty } => {
                // Inside an item map the frame's document is the element, which `holds` then reads.
                let chosen = branches.iter().position(|(c, _)| {
                    (c.as_ref()).is_none_or(|c| holds(c, f.doc.as_ref(), None, f.ctx.as_ref()))
                });
                for (i, (_, members)) in branches.iter().enumerate() {
                    if Some(i) != chosen {
                        members.iter().for_each(|(_, op)| self.miss(op));
                    }
                }
                let Some(i) = chosen else {
                    return self.sink.passthrough(&empty);
                };
                let (mark, filled) = (self.sink.mark(), self.filled);
                let out = self.object(&branches[i].1, f);
                if self.filled > filled {
                    return out;
                }
                self.sink.rewind(mark);
                self.sink.passthrough(&empty)
            }
            Op::Optional {
                members,
                present,
                if_empty,
            } => {
                // Inside an item map the frame's document is the element, which `holds` then reads.
                if !holds(present, f.doc.as_ref(), None, f.ctx.as_ref()) {
                    members.iter().for_each(|(_, op)| self.miss(op));
                    return self.sink.scalar(Value::Null);
                }
                let Some(if_empty) = if_empty else {
                    return self.object(members, f);
                };
                let (mark, filled) = (self.sink.mark(), self.filled);
                let out = self.object(members, f);
                if self.filled > filled {
                    return out;
                }
                self.sink.rewind(mark);
                self.sink.passthrough(&if_empty)
            }
        }
    }

    /// Counts a miss for every field in `op`, which is not mapped.
    fn miss(&self, op: &Op) {
        match op {
            Op::Null | Op::Const(_, None) => {}
            Op::Scalar(Leaf { stat, .. }) | Op::List { stat, .. } | Op::Const(_, Some(stat)) => {
                self.stats.miss(*stat)
            }
            Op::Aggregate(a) => self.stats.miss(a.stat),
            Op::Container(w) => self.stats.miss(w.stat),
            Op::Object(c) | Op::Optional { members: c, .. } => {
                c.iter().for_each(|(_, op)| self.miss(op))
            }
            Op::OmitNull(op) | Op::When { op, .. } => self.miss(op),
            Op::Union { branches, .. } => (branches.iter())
                .flat_map(|(_, c)| c)
                .for_each(|(_, op)| self.miss(op)),
        }
    }

    fn scalar<D: Doc>(&mut self, l: &Leaf, f: &mut Frame<'_, D>) -> S::Out {
        // None: absent. Some(None): rejected.
        let out = match &l.src {
            Src::From(p) => match f.read(*p) {
                Some(d) => Some(self.single(l, &d)),
                // R54: a null result is a miss, not an error.
                None if l.on_missing => self.on_null(l),
                None => None,
            },
            Src::Coalesce(ps) => ps
                .iter()
                .find_map(|p| f.read(*p))
                .map(|d| self.single(l, &d)),
            Src::CoalesceWhen(es) => pick(es, f, false).map(|d| self.single(l, &d)),
            Src::Many(ps) => self.many(l, f, ps),
            Src::Computed(xs) => self.computed(l, f, xs),
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
                match (&l.default, &l.empty) {
                    (None, Some(e)) => self.sink.passthrough(&e),
                    (d, _) => self.sink.scalar(d.as_ref().map_or(Value::Null, json_value)),
                }
            }
        }
    }

    /// R54: the first transform on null, then the rest as `pipe` runs them. `None`: a null result,
    /// a miss. `Some(None)`: a raise or a container result, a rejection.
    fn on_null(&mut self, l: &Leaf) -> Option<Option<S::Out>> {
        let (first, rest) = l.tx.split_first()?;
        let v = match first {
            TxRef::Builtin(b) => Some(b.apply(Value::Null, None)),
            TxRef::Custom { index, arg, .. } => self.custom.get(*index).and_then(|f| {
                catch_unwind(AssertUnwindSafe(|| f.try_apply(Value::Null, arg.as_ref())))
                    .ok()
                    .flatten()
            }),
        };
        match v.map(|v| pipe(self.custom, rest, v)) {
            Some(Value::Null) => None,
            None | Some(Value::Container) => Some(None),
            Some(v) => Some(Some(self.sink.scalar(v))),
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

    /// `many` for a computed list: every entry missing is a miss, not an error.
    fn computed<D: Doc>(
        &mut self,
        l: &Leaf,
        f: &mut Frame<'_, D>,
        items: &[Item],
    ) -> Option<Option<S::Out>> {
        let es: Vec<Entry<D>> = items.iter().map(|i| entry(self.custom, i, f)).collect();
        if es.iter().all(|e| matches!(e, Entry::Missing)) {
            return None;
        }
        let tuple = |this: &mut Self| {
            this.sink.open(false);
            let out = (es.iter().enumerate())
                .map(|(n, e)| {
                    this.sink.next(None, n == 0);
                    match e {
                        Entry::Doc(d) => this.sink.passthrough(d),
                        Entry::Val(v) => this.sink.scalar(v.clone()),
                        Entry::Missing => this.sink.scalar(Value::Null),
                    }
                })
                .collect();
            this.sink.list(out)
        };
        if l.bare() {
            return Some(Some(tuple(self)));
        }
        Some(
            match transform_entries(self.custom, &l.tx, l.validator, &es) {
                Value::Null => None,
                Value::Container => Some(tuple(self)),
                v => Some(self.sink.scalar(v)),
            },
        )
    }

    fn many<D: Doc>(
        &mut self,
        l: &Leaf,
        f: &mut Frame<'_, D>,
        ps: &[At],
    ) -> Option<Option<S::Out>> {
        // One Vec per read, as codegen.ts allocates one array. Only multi-path `from` pays.
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
        from: &Input,
        kind: &Kind,
        stat: usize,
        ops: &ListOps<Path>,
        coalesce: &[Entry2],
        f: &mut Frame<'_, D>,
    ) -> S::Out {
        let xs = match (from, f.group) {
            (Input::Group, Some(g)) => refine(g.to_vec(), ops, f.ctx.as_ref()),
            (Input::Group, None) => {
                self.stats.miss(stat);
                return self.empty(ops.missing, false);
            }
            (Input::At(at), _) => {
                let x = match coalesce.is_empty() {
                    true => f.read(*at),
                    false => pick(coalesce, f, true),
                };
                let Some(x) = x else {
                    self.stats.miss(stat);
                    return self.empty(ops.missing, false);
                };
                // Lists without A4, A5 or A6 options stream the source array, as in language 1.
                if !ops.collects() {
                    return match x.len() {
                        Some(n) => self.elements(n, |i| x.item(i), None, kind, stat, ops, f),
                        None => self.wrong(stat, ops.wrong),
                    };
                }
                match collect(Some(x), ops, f.ctx.as_ref()) {
                    Some(xs) => xs,
                    None => return self.wrong(stat, ops.wrong),
                }
            }
        };
        let Some((key, consecutive)) = &ops.group else {
            return self.elements(xs.len(), |i| xs.get(i).cloned(), None, kind, stat, ops, f);
        };
        let keys: Vec<Path> = std::iter::once(key.clone())
            .chain(ops.group_also.iter().cloned())
            .collect();
        let gs = groups(xs, &keys, *consecutive);
        let first = |i: usize| gs.get(i).and_then(|g| g.first()).cloned();
        self.elements(gs.len(), first, Some(&gs), kind, stat, ops, f)
    }

    /// An aggregate (`Fold`): its values, then `group::aggregate`.
    fn fold<D: Doc>(&mut self, a: &Fold, f: &mut Frame<'_, D>) -> S::Out {
        // None: not an array.
        let xs = match &a.input {
            Input::Group => Some(f.group.map(<[D]>::to_vec).unwrap_or_default()),
            Input::At(at) => match f.read(*at) {
                None => Some(Vec::new()),
                Some(x) => (x.len()).map(|n| (0..n).filter_map(|i| x.item(i)).collect()),
            },
        };
        let out = xs.and_then(|xs| {
            let values = (refine(xs, &a.ops, None).iter())
                .map(|e| read(Some(e), &a.pluck))
                .collect();
            aggregate(a.op, values, a.propagate, &a.sep)
        });
        match out {
            Some(Agg::Doc(d)) => {
                self.filled += 1;
                self.sink.passthrough(&d)
            }
            Some(Agg::Value(v)) if !v.is_null() => {
                self.filled += 1;
                self.sink.scalar(v)
            }
            out => {
                if out.is_none() {
                    self.stats.error(a.stat);
                }
                self.stats.miss(a.stat);
                self.sink.scalar(Value::Null)
            }
        }
    }

    /// A container transform: the source as JSON, the transform, then `conform`. A missing source
    /// writes the node's `x-empty`, a placeholder that is no fill.
    fn whole<D: Doc>(&mut self, w: &Whole, f: &mut Frame<'_, D>) -> S::Out {
        let mut budget = MAX_NODES;
        // None: missing. Some(None): too deep to copy.
        let input = match &w.from {
            Input::Group => f.group.map(|g| {
                let items = g.iter().map(|d| json(d, 1, &mut budget));
                items.collect::<Option<_>>().map(J::Array)
            }),
            Input::At(at) => f.read(*at).map(|d| json(&d, 0, &mut budget)),
        };
        // R54: a missing source with `on_missing` runs the transform on null.
        let missing = input.is_none();
        let input = match (input, w.on_missing) {
            (None, true) => Some(Some(J::Null)),
            (input, _) => input,
        };
        let Some(input) = input else {
            return self.missing_whole(w);
        };
        let out = match (&w.tx, input) {
            (TxRef::Custom { index, arg, .. }, Some(v)) => self.custom.get(*index).and_then(|t| {
                catch_unwind(AssertUnwindSafe(|| t.apply_container_out(&v, arg.as_ref())))
                    .ok()
                    .flatten()
            }),
            _ => None,
        };
        match out.and_then(|v| conform(&v, &w.node, w.strict)) {
            Some(v) if !v.is_null() => {
                self.filled += 1;
                self.sink.passthrough(&&v)
            }
            // A null result for a missing source is a miss, not an error.
            // Guard: a custom result that is null (not a fill) on a missing source is a miss.
            Some(_) if missing => self.missing_whole(w),
            _ if w.strict => {
                self.stats.error(w.stat);
                self.missing_whole(w)
            }
            _ => self.wrong(w.stat, None),
        }
    }

    /// A missing source: a miss, and the node's empty form, a placeholder that is no fill.
    fn missing_whole(&mut self, w: &Whole) -> S::Out {
        self.stats.miss(w.stat);
        self.sink.passthrough(&&w.empty)
    }

    /// A source that is not an array: an error, a miss, and null or the strict empty form (R48), a
    /// placeholder that is no fill.
    fn wrong(&mut self, stat: usize, e: Option<Empty>) -> S::Out {
        self.stats.error(stat);
        self.stats.miss(stat);
        self.empty(e.unwrap_or(Empty::Null), false)
    }

    /// A list of the `n` elements `get` gives. With `groups`, element `i` is the first of group `i`,
    /// and an item map reads the group as `$group`. An item map reads the context of `outer`.
    #[allow(clippy::too_many_arguments)]
    fn elements<D: Doc>(
        &mut self,
        n: usize,
        get: impl Fn(usize) -> Option<D>,
        groups: Option<&[Vec<D>]>,
        kind: &Kind,
        stat: usize,
        ops: &ListOps<Path>,
        outer: &Frame<'_, D>,
    ) -> S::Out {
        if n == 0 {
            return self.empty(ops.empty, true);
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
                        let mut f = Frame::new(get(i), scope, true);
                        f.group = groups.and_then(|g| g.get(i)).map(Vec::as_slice);
                        f.ctx = outer.ctx.clone();
                        self.object(fields, &mut f)
                    }
                    Kind::Elems(path, tx) => self.element(read(get(i).as_ref(), path), tx, stat),
                }
            })
            .collect();
        self.sink.list(items)
    }

    /// `[]` for `Empty::List`, null otherwise. `[]` for an empty source array is a fill. For a
    /// missing or wrong source it is a placeholder, which does not make its object filled.
    fn empty(&mut self, e: Empty, fill: bool) -> S::Out {
        if e != Empty::List {
            return self.sink.scalar(Value::Null);
        }
        self.filled += fill as usize;
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

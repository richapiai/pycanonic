//! A compiled plan ready to map: the program, its stats, the linked custom transforms and the parse
//! filter. Port of the `map` and `stats` half of canonic/packages/canonic/src/artifact.ts.

use crate::cond::Cond;
use crate::doc::{Doc, Either};
use crate::exec::{Program, execute, execute_field};
use crate::extract::{ExtractStream, Extractor};
use crate::json::{Filter, Node, parse};
use crate::plan::Source;
use crate::plan::{CompileError, Field, ListKind, Named, Plan, Scalar, TxRef};
use crate::sink::{Out, RustSink, Sink};
use crate::spec::{Path, Step, is_group};
use crate::stats::Stats;
use crate::transforms::{Builtin, Transform};
use crate::value::Value;
use crate::write::{JsonSink, member_keys};
use std::collections::HashMap;
use std::sync::Arc;

/// A response that a `reject` condition refuses. `reason` is the condition's spec JSON text,
/// for example `{"missing":"data.name"}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rejected {
    pub reason: String,
}

pub struct Artifact {
    pub plan: Arc<Plan>,
    pub stats: Stats,
    program: Program,
    custom: Vec<Arc<dyn Transform>>,
    filter: Filter,
    extractor: Arc<Extractor>,
    /// `"name":` per canonical name, for `map_json`.
    json_keys: Vec<Box<[u8]>>,
}

impl Artifact {
    /// Links `custom` by name against `plan.custom_names` and range-checks every index in the plan.
    pub fn new(
        plan: Plan,
        custom: HashMap<String, Arc<dyn Transform>>,
    ) -> Result<Artifact, CompileError> {
        let program = Program::new(&plan)?;
        let m = &plan.meta;
        let custom = plan
            .custom_names
            .iter()
            .map(|name| {
                custom.get(name).cloned().ok_or_else(|| CompileError {
                    message: format!(
                        "{} {} v{}: transform {} is not registered",
                        m.provider,
                        m.endpoint,
                        m.version,
                        serde_json::Value::from(name.as_str())
                    ),
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Artifact {
            stats: Stats::new(&plan),
            filter: filter_for(&plan),
            extractor: Arc::new(Extractor::new(&plan, &custom)),
            json_keys: member_keys(&plan.names),
            plan: Arc::new(plan),
            program,
            custom,
        })
    }

    /// Distinct path keys the plan reads. `Key::tag` indexes this.
    pub fn keys(&self) -> &[String] {
        self.program.keys()
    }

    /// Parses `body` with the plan's filter and maps it. A body that does not parse reads as absent:
    /// every field null, counted as misses. `$input` paths read `ctx`, the request context. Without
    /// one they read as absent.
    pub fn map_bytes(&self, body: &[u8], ctx: Option<&serde_json::Value>) -> Result<Out, Rejected> {
        let node = self.parse(body);
        self.map_with_ctx(node.as_ref().as_ref(), ctx, &mut RustSink)
    }

    /// `map_bytes` written straight to JSON text, no intermediate Python objects: the plan runs into
    /// a `JsonSink`, so not even an `Out` tree is built.
    pub fn map_json(
        &self,
        body: &[u8],
        ctx: Option<&serde_json::Value>,
    ) -> Result<Vec<u8>, Rejected> {
        let node = self.parse(body);
        let mut sink = JsonSink {
            buf: Vec::with_capacity(body.len()),
            open: Vec::new(),
            keys: &self.json_keys,
        };
        let root = self.map_with_ctx(node.as_ref().as_ref(), ctx, &mut sink)?;
        Ok(sink.finish(root))
    }

    pub fn map_doc<D: Doc>(
        &self,
        doc: &D,
        ctx: Option<&serde_json::Value>,
    ) -> Result<Out, Rejected> {
        self.map_with_ctx(Some(doc), ctx, &mut RustSink)
    }

    /// `map_bytes` with every canonical object's keys by name (`Out::Map` all the way down), as
    /// `ShadowDiff::add_out` and `GateOptions::known` read output.
    pub fn map_named(&self, body: &[u8], ctx: Option<&serde_json::Value>) -> Result<Out, Rejected> {
        Ok(named(self.map_bytes(body, ctx)?, &self.plan.names))
    }

    /// `map_doc`, keys by name as `map_named`.
    pub fn map_doc_named<D: Doc>(
        &self,
        doc: &D,
        ctx: Option<&serde_json::Value>,
    ) -> Result<Out, Rejected> {
        Ok(named(self.map_doc(doc, ctx)?, &self.plan.names))
    }

    /// `map_with` for a context given as JSON. With no context, the body's own document type runs
    /// alone. With one, body and context run as `Either`.
    pub fn map_with_ctx<D: Doc, S: Sink>(
        &self,
        doc: Option<&D>,
        ctx: Option<&serde_json::Value>,
        sink: &mut S,
    ) -> Result<S::Out, Rejected> {
        match ctx {
            None => self.map_with(doc, None, sink),
            Some(c) => {
                let doc = doc.map(|d| Either::Body(d.clone()));
                self.map_with(doc.as_ref(), Some(&Either::Ctx(c)), sink)
            }
        }
    }

    /// The routing fields read straight from `raw`, aligned with `plan.routing`: `None` when not
    /// found, `Some(Value::Null)` when found and rejected by the field's transforms or validator.
    /// Anchored fields are found by their key token, the rest by one scan. Counts no stats.
    pub fn extract(&self, raw: &[u8]) -> Vec<Option<Value<'static>>> {
        self.extractor.extract(raw)
    }

    /// `extract` over chunks as they arrive. It always scans: anchors are for `extract` only.
    pub fn stream(&self) -> ExtractStream {
        ExtractStream::new(self.extractor.clone())
    }

    /// `body` parsed with the plan's filter, `None` when it does not parse (which reads as absent).
    pub fn parse<'a>(&self, body: &'a [u8]) -> Option<Node<'a>> {
        parse(body, &self.filter).ok()
    }

    /// One field as `map_with` maps it, by index path (see `exec::execute_field`). Counts no stats,
    /// as a JavaScript View getter does not. `None` when `path` names no field.
    pub fn map_field<D: Doc, S: Sink>(
        &self,
        path: &[usize],
        doc: Option<&D>,
        sink: &mut S,
    ) -> Option<S::Out> {
        execute_field(&self.program, path, doc, sink, &self.custom)
    }

    /// Maps `doc` into `sink`. `$input` paths read `ctx`, a document of the same type. A `reject`
    /// condition that holds on `doc` stops the map before any field is read: nothing goes into
    /// `sink`, and stats count a rejection, not a response.
    pub fn map_with<D: Doc, S: Sink>(
        &self,
        doc: Option<&D>,
        ctx: Option<&D>,
        sink: &mut S,
    ) -> Result<S::Out, Rejected> {
        if let Some(i) = self.program.rejected(doc, ctx) {
            self.stats.rejected();
            let reason = self.plan.reject[i].reason.clone();
            return Err(Rejected { reason });
        }
        Ok(execute(
            &self.program,
            doc,
            ctx,
            sink,
            &self.stats,
            &self.custom,
        ))
    }

    /// Whether the plan reads `$input`. A plan that does not ignores the context.
    pub fn reads_input(&self) -> bool {
        self.program.reads_input()
    }
}

/// `o` with each `Out::Obj` key index replaced by its name in `names` (`Plan::names`).
fn named(o: Out, names: &[String]) -> Out {
    match o {
        Out::Obj(m) => Out::Map(
            m.into_iter()
                .map(|(k, v)| (names[k].clone(), named(v, names)))
                .collect(),
        ),
        Out::Map(m) => Out::Map(m.into_iter().map(|(k, v)| (k, named(v, names))).collect()),
        Out::List(xs) => Out::List(xs.into_iter().map(|x| named(x, names)).collect()),
        o => o,
    }
}

/// What the parser has to build for `plan`: every path it reads, everything below a value that
/// goes out as it is. Where an array has both an index and all elements read, the index gets both.
pub fn filter_for(plan: &Plan) -> Filter {
    // A case-insensitive plan keeps the whole body, because a key that matches only by
    // case has a name the filter does not list. A lowercase key table would keep the pruning.
    if plan.case_insensitive {
        return Filter::all();
    }
    let mut f = Filter::default();
    fields(&mut f, &plan.fields);
    for p in plan.reject.iter().flat_map(|r| cond_paths(&r.cond)) {
        node(&mut f, p).all = true;
    }
    settle(&mut f);
    f
}

fn fields(f: &mut Filter, fs: &[Named<Field>]) {
    for x in fs {
        let mut field = &x.value;
        loop {
            match field {
                Field::OmitNull(inner) => field = inner,
                // A condition can compare whole values.
                Field::When {
                    when, field: inner, ..
                } => {
                    for p in cond_paths(when) {
                        node(f, p).all = true;
                    }
                    field = inner;
                }
                _ => break,
            }
        }
        match field {
            // A const reads nothing.
            Field::Null | Field::Const(..) | Field::Empty(_) => {}
            Field::OmitNull(_) | Field::When { .. } => unreachable!("the loop unwraps it"),
            Field::Object {
                members, present, ..
            } => {
                // A present check reads only whether the value is there and not null.
                for p in present {
                    node(f, p);
                }
                fields(f, members)
            }
            // A branch condition can compare whole values.
            Field::Union { branches, .. } => {
                for b in branches {
                    for p in b.when.iter().flat_map(cond_paths) {
                        node(f, p).all = true;
                    }
                    fields(f, &b.members);
                }
            }
            Field::Scalar(s) => scalar(f, s),
            // A container transform gets its source whole.
            Field::Container { from, .. } if is_group(from) => {
                let e = Filter {
                    all: true,
                    ..Filter::default()
                };
                elements(f, from, &[], false, true, e);
            }
            Field::Container { from, .. } => node(f, from).all = true,
            // An aggregate can write or compare its values whole.
            Field::Aggregate(a, _) => {
                let mut e = Filter::default();
                node(&mut e, &a.pluck).all = true;
                for p in a.filter.iter().flat_map(cond_paths) {
                    node(&mut e, p).all = true;
                }
                elements(f, &a.from, &a.flatten, false, is_group(&a.from), e);
            }
            Field::List {
                from,
                kind,
                ops,
                coalesce,
                ..
            } => {
                let mut e = Filter::default();
                match kind {
                    ListKind::Item(c) => fields(&mut e, c),
                    ListKind::Pluck(p, tx) => node(&mut e, p).all |= tx.is_empty() || custom(tx),
                    ListKind::Plain(tx) => e.all |= tx.is_empty() || custom(tx),
                }
                // Conditions, `unique` and `sort_by` can compare whole values.
                let filter = ops.filter.iter().flat_map(cond_paths);
                let keys = ops.unique.iter().chain(ops.sort.iter().map(|(p, _)| p));
                let group = (ops.group.iter().map(|(p, _)| p)).chain(&ops.group_also);
                for p in filter.chain(keys).chain(group) {
                    node(&mut e, p).all = true;
                }
                // Every coalesce entry is a source, and its condition compares whole values.
                let first = std::iter::once((from, None)).take((coalesce.is_empty()) as usize);
                let entries = coalesce.iter().map(|(p, c)| (p, c.as_ref()));
                for (p, c) in first.chain(entries) {
                    for q in c.into_iter().flat_map(cond_paths) {
                        node(f, q).all = true;
                    }
                    let (flatten, group) = (&ops.flatten, ops.from_group);
                    elements(f, p, flatten, ops.wrap, group, e.clone());
                }
            }
        }
    }
}

/// `e`, the filter of each element read from `from` through `flatten`, merged into `f`. The
/// elements of `$group` are the elements of the grouped list, which `f` already is.
fn elements(f: &mut Filter, from: &Path, flatten: &[Path], wrap: bool, group: bool, mut e: Filter) {
    // Each flatten path holds the elements of the level below it.
    for p in flatten.iter().rev() {
        let mut up = Filter::default();
        node(&mut up, p).elems = Some(Box::new(e));
        e = up;
    }
    if group {
        return merge(f, &e);
    }
    let src = node(f, from);
    // A wrapped source can be the one element itself.
    if wrap {
        merge(src, &e);
    }
    merge(src.elems.get_or_insert_default(), &e);
}

/// The element paths a condition reads. `$input` paths read the context, not the body.
fn cond_paths(c: &Cond) -> Vec<&Path> {
    (c.paths().into_iter())
        .filter(|p| !matches!(p.first(), Some(Step::Key(k)) if k == "$input"))
        .collect()
}

fn custom(tx: &[TxRef]) -> bool {
    tx.iter().any(|t| matches!(t, TxRef::Custom { .. }))
}

fn scalar(f: &mut Filter, s: &Scalar) {
    // A custom transform's container input goes to it whole (`Transform::apply_container`).
    // `len` first in line counts an array's elements, so the array is read whole.
    let len = matches!(s.tx.first(), Some(TxRef::Builtin(Builtin::Len)));
    let whole = (s.validator.is_none() && s.tx.is_empty()) || custom(&s.tx) || len;
    // a computed entry may run a custom transform or `len`, so all its paths are read
    // whole. Narrow it to the entries that need it if the artifact size matters.
    let whole = whole || matches!(s.source, Source::Computed(_));
    for p in s.source.paths() {
        node(f, p).all |= whole;
    }
    // A condition can compare whole values.
    if let Source::CoalesceWhen(es) = &s.source {
        for q in es.iter().flat_map(|(_, c)| c).flat_map(cond_paths) {
            node(f, q).all = true;
        }
    }
}

fn node<'f>(mut f: &'f mut Filter, path: &Path) -> &'f mut Filter {
    for step in path {
        f = match step {
            Step::Key(k) => key(f, k.as_bytes()),
            Step::Idx(i) => idx(f, *i),
        };
    }
    f
}

fn key<'f>(f: &'f mut Filter, k: &[u8]) -> &'f mut Filter {
    let i = match f.keys.iter().position(|(x, _)| **x == *k) {
        Some(i) => i,
        None => {
            f.keys.push((k.into(), Filter::default()));
            f.keys.len() - 1
        }
    };
    &mut f.keys[i].1
}

fn idx(f: &mut Filter, n: usize) -> &mut Filter {
    let i = match f.idxs.iter().position(|(x, _)| *x == n) {
        Some(i) => i,
        None => {
            f.idxs.push((n, Filter::default()));
            f.idxs.len() - 1
        }
    };
    &mut f.idxs[i].1
}

fn merge(a: &mut Filter, b: &Filter) {
    a.all |= b.all;
    for (k, c) in &b.keys {
        merge(key(a, k), c);
    }
    for (n, c) in &b.idxs {
        merge(idx(a, *n), c);
    }
    if let Some(e) = &b.elems {
        merge(a.elems.get_or_insert_default(), e);
    }
}

/// The parser lets an `idxs` entry win over `elems`. Fold `elems` into each so neither read is lost.
fn settle(f: &mut Filter) {
    if let Some(e) = &f.elems {
        for (_, c) in &mut f.idxs {
            merge(c, e);
        }
    }
    let keys = f.keys.iter_mut().map(|(_, c)| c);
    for c in keys.chain(f.idxs.iter_mut().map(|(_, c)| c)) {
        settle(c);
    }
    if let Some(e) = &mut f.elems {
        settle(e);
    }
}

//! A compiled plan ready to map: the program, its stats, the linked custom transforms and the parse
//! filter. Port of the `map` and `stats` half of canonic/packages/canonic/src/artifact.ts.

use crate::doc::Doc;
use crate::exec::{Program, execute};
use crate::json::{Filter, parse};
use crate::plan::{CompileError, Field, ListKind, Named, Plan, Scalar};
use crate::sink::{Out, RustSink, Sink};
use crate::spec::{Path, Source, Step};
use crate::stats::Stats;
use crate::transforms::Transform;
use crate::write::write_json;
use std::collections::HashMap;
use std::sync::Arc;

pub struct Artifact {
    pub plan: Arc<Plan>,
    pub stats: Stats,
    program: Program,
    custom: Vec<Arc<dyn Transform>>,
    filter: Filter,
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
            .collect::<Result<_, _>>()?;
        Ok(Artifact {
            stats: Stats::new(&plan),
            filter: filter_for(&plan),
            plan: Arc::new(plan),
            program,
            custom,
        })
    }

    /// Distinct path keys the plan reads; `Key::tag` indexes this.
    pub fn keys(&self) -> &[String] {
        self.program.keys()
    }

    /// Parses `body` with the plan's filter and maps it. A body that doesn't parse reads as absent:
    /// every field null, counted as misses.
    pub fn map_bytes(&self, body: &[u8]) -> Out {
        let node = parse(body, &self.filter).ok();
        self.map_with(node.as_ref().as_ref(), &mut RustSink)
    }

    /// `map_bytes` written straight to JSON text, no intermediate Python objects.
    pub fn map_json(&self, body: &[u8]) -> Vec<u8> {
        let out = self.map_bytes(body);
        let mut buf = Vec::with_capacity(body.len());
        write_json(&self.plan, &out, &mut buf);
        buf
    }

    pub fn map_doc<D: Doc>(&self, doc: &D) -> Out {
        self.map_with(Some(doc), &mut RustSink)
    }

    pub fn map_with<D: Doc, S: Sink>(&self, doc: Option<&D>, sink: &mut S) -> S::Out {
        execute(&self.program, doc, sink, &self.stats, &self.custom)
    }
}

/// What the parser has to build for `plan`: every path it reads, everything below a value that
/// goes out as it is. Where an array has both an index and all elements read, the index gets both.
pub fn filter_for(plan: &Plan) -> Filter {
    let mut f = Filter::default();
    fields(&mut f, &plan.fields);
    settle(&mut f);
    f
}

fn fields(f: &mut Filter, fs: &[Named<Field>]) {
    for x in fs {
        match &x.value {
            Field::Null => {}
            Field::Object(c) => fields(f, c),
            Field::Scalar(s) => scalar(f, s),
            Field::List { from, kind, .. } => {
                let e = node(f, from).elems.get_or_insert_default();
                match kind {
                    ListKind::Item(c) => fields(e, c),
                    ListKind::Pluck(p, tx) => node(e, p).all |= tx.is_empty(),
                    ListKind::Plain(tx) => e.all |= tx.is_empty(),
                }
            }
        }
    }
}

fn scalar(f: &mut Filter, s: &Scalar) {
    let bare = s.tx.is_empty() && s.validator.is_none();
    let paths = match &s.source {
        Source::From(p) => std::slice::from_ref(p),
        Source::FromMany(ps) | Source::Coalesce(ps) => ps.as_slice(),
    };
    for p in paths {
        node(f, p).all |= bare;
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

/// The parser lets an `idxs` entry win over `elems`; fold `elems` into each so neither read is lost.
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

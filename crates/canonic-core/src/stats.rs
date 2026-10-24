//! Per-field fill and error counts across `map` calls. Like artifact.ts, misses are counted rather
//! than fills (a filled field writes nothing) and fill is the field's total minus its misses.

use crate::plan::{Field, ListKind, Named, Plan, Scalar};
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldStats {
    pub fill: u64,
    pub errors: u64,
    /// Only for a field inside a list's `item` map: the elements seen, which `fill` is out of.
    pub total: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatsSnapshot {
    pub responses: u64,
    /// In stat slot order, which is spec key order, named as in `Plan::stat_names`.
    pub fields: Vec<(String, FieldStats)>,
}

pub struct Stats {
    names: Vec<String>,
    responses: AtomicU64,
    misses: Box<[AtomicU64]>,
    errors: Box<[AtomicU64]>,
    /// Per item list slot: the elements it has walked.
    elems: Box<[AtomicU64]>,
    /// Per slot: the item list slot its fill is out of, `None` for responses.
    within: Box<[Option<usize>]>,
}

fn counters(n: usize) -> Box<[AtomicU64]> {
    (0..n).map(|_| AtomicU64::new(0)).collect()
}

fn walk(fields: &[Named<Field>], list: Option<usize>, within: &mut [Option<usize>]) {
    for f in fields {
        match f.value.inner() {
            Field::Null | Field::Empty(_) => {}
            Field::OmitNull(_) => unreachable!("inner() unwraps it"),
            Field::Object { members, .. } => walk(members, list, within),
            Field::Scalar(Scalar { stat, .. }) | Field::Const(_, stat) => {
                if let Some(w) = within.get_mut(*stat) {
                    *w = list;
                }
            }
            Field::List { kind, stat, .. } => {
                if let Some(w) = within.get_mut(*stat) {
                    *w = list;
                }
                if let ListKind::Item(c) = kind {
                    walk(c, Some(*stat), within);
                }
            }
        }
    }
}

impl Stats {
    pub fn new(plan: &Plan) -> Stats {
        let n = plan.stat_names.len();
        let mut within = vec![None; n];
        walk(&plan.fields, None, &mut within);
        Stats {
            names: plan.stat_names.clone(),
            responses: AtomicU64::new(0),
            misses: counters(n),
            errors: counters(n),
            elems: counters(n),
            within: within.into(),
        }
    }

    /// No slots, so every count is dropped (`bump` ignores them) and nothing is allocated.
    pub(crate) fn none() -> Stats {
        Stats {
            names: Vec::new(),
            responses: AtomicU64::new(0),
            misses: counters(0),
            errors: counters(0),
            elems: counters(0),
            within: Box::new([]),
        }
    }

    // Out-of-range slots are ignored rather than panicking. Artifact::new range-checks the plan.
    fn bump(c: &[AtomicU64], slot: usize, by: u64) {
        if let Some(c) = c.get(slot) {
            c.fetch_add(by, Relaxed);
        }
    }

    pub(crate) fn response(&self) {
        self.responses.fetch_add(1, Relaxed);
    }

    pub(crate) fn miss(&self, slot: usize) {
        Self::bump(&self.misses, slot, 1);
    }

    pub(crate) fn error(&self, slot: usize) {
        Self::bump(&self.errors, slot, 1);
    }

    pub(crate) fn elements(&self, slot: usize, n: usize) {
        Self::bump(&self.elems, slot, n as u64);
    }

    pub fn snapshot(&self) -> StatsSnapshot {
        let fields = self
            .names
            .iter()
            .enumerate()
            .map(|(i, name)| {
                // Misses first: a total only grows before its misses do, so fill cannot go negative
                // under concurrent map calls (saturating covers a concurrent reset).
                let misses = self.misses[i].load(Relaxed);
                let errors = self.errors[i].load(Relaxed);
                let (total, shown) = match self.within[i] {
                    Some(l) => {
                        let t = self.elems.get(l).map_or(0, |c| c.load(Relaxed));
                        (t, Some(t))
                    }
                    None => (self.responses.load(Relaxed), None),
                };
                let fill = total.saturating_sub(misses);
                (
                    name.clone(),
                    FieldStats {
                        fill,
                        errors,
                        total: shown,
                    },
                )
            })
            .collect();
        StatsSnapshot {
            responses: self.responses.load(Relaxed),
            fields,
        }
    }

    pub fn reset(&self) {
        self.responses.store(0, Relaxed);
        for c in self.misses.iter().chain(&*self.errors).chain(&*self.elems) {
            c.store(0, Relaxed);
        }
    }
}

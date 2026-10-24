//! Conditions and their evaluation over a `Doc`. The plan holds them with spec paths
//! (`Cond`), the executor with its own interned paths (`Cond<KeyPath>`), which `holds` reads.

use crate::doc::{Doc, KeyPath, KeyStep, read};
use crate::spec::Path;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
pub enum Cond<P = Path> {
    /// `{"path": p, "present": true|false}`: present means there and not null.
    Present(P, bool),
    Eq(P, Value),
    In(P, Vec<Value>),
    Missing(P),
    Empty(P),
    /// Every condition holds. An empty list holds.
    All(Vec<Cond<P>>),
}

/// The value `path` names. A path that starts with `$input` reads `ctx`. Any other path reads
/// `element` inside an item map, else `root`.
fn value<D: Doc>(
    path: &[KeyStep],
    root: Option<&D>,
    element: Option<&D>,
    ctx: Option<&D>,
) -> Option<D> {
    match path.split_first() {
        Some((KeyStep::Key(k), rest)) if k.s == "$input" => read(ctx, rest),
        _ => read(element.or(root), path),
    }
}

/// Whether `c` holds. Only `Present` and `All` are evaluated so far.
pub fn holds<D: Doc>(
    c: &Cond<KeyPath>,
    root: Option<&D>,
    element: Option<&D>,
    ctx: Option<&D>,
) -> bool {
    match c {
        Cond::Present(p, want) => value(p, root, element, ctx).is_some() == *want,
        Cond::All(cs) => cs.iter().all(|c| holds(c, root, element, ctx)),
        Cond::Eq(..) | Cond::In(..) | Cond::Missing(_) | Cond::Empty(_) => {
            unreachable!("no plan builds this condition yet")
        }
    }
}

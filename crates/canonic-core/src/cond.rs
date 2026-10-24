//! Conditions, their parsing and their evaluation over a `Doc`. The plan holds them
//! with spec paths (`Cond`), the executor with its own interned paths (`Cond<KeyPath>`), which
//! `holds` reads. Also the value order and the JSON key text that sorting, `unique` and grouping
//! share.

use crate::doc::{Doc, KeyPath, KeyStep, read};
use crate::exec::{MAX_NODES, json};
use crate::spec::{Path, parse_path};
use crate::value::Value as V;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::cmp::Ordering;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Cond<P = Path> {
    /// `{"path": p, "present": true|false}`: present means there and not null.
    Present(P, bool),
    /// `{"path": p, "eq": v}`. A missing value equals null.
    Eq(P, Value),
    /// `{"path": p, "in": [v, ...]}`: equals one of them.
    In(P, Vec<Value>),
    /// `{"missing": p}`: absent or null.
    Missing(P),
    /// `{"empty": p}`: absent, null, `""`, `[]` or `{}`.
    Empty(P),
    /// Every condition holds. An empty list holds.
    All(Vec<Cond<P>>),
}

const SHAPE: &str = "must be {\"path\": <path>, \"eq\" | \"in\" | \"present\": <value>}";

/// A condition as the spec writes it, in the condition forms (`eq`, `in`, `present`). `Err` holds every
/// issue as (location suffix, message), for example `(".in", "must be an array")`.
pub fn parse(v: &Value) -> Result<Cond, Vec<(String, String)>> {
    let Some(o) = v.as_object().filter(|o| o.contains_key("path")) else {
        return Err(vec![(String::new(), SHAPE.into())]);
    };
    let mut issues = Vec::new();
    for k in o.keys() {
        if !["path", "eq", "in", "present"].contains(&k.as_str()) {
            issues.push((format!(".{k}"), "unknown property".into()));
        }
    }
    let path = o["path"].as_str().and_then(parse_path);
    if path.is_none() {
        let m = "must be a path like a.b[0][\"c d\"]";
        issues.push((".path".into(), m.into()));
    }
    let tests: Vec<&str> = ["eq", "in", "present"]
        .into_iter()
        .filter(|k| o.contains_key(*k))
        .collect();
    if tests.len() != 1 {
        issues.push((String::new(), "needs exactly one of eq, in, present".into()));
    }
    let c = match tests.first().map(|k| (*k, &o[*k])) {
        Some(("in", Value::Array(a))) => path.map(|p| Cond::In(p, a.clone())),
        Some(("in", _)) => {
            issues.push((".in".into(), "must be an array".into()));
            None
        }
        Some(("present", Value::Bool(b))) => path.map(|p| Cond::Present(p, *b)),
        Some(("present", _)) => {
            issues.push((".present".into(), "must be true or false".into()));
            None
        }
        Some((_, v)) => path.map(|p| Cond::Eq(p, v.clone())),
        None => None,
    };
    match c {
        Some(c) if issues.is_empty() => Ok(c),
        _ => Err(issues),
    }
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

/// Whether `c` holds.
pub fn holds<D: Doc>(
    c: &Cond<KeyPath>,
    root: Option<&D>,
    element: Option<&D>,
    ctx: Option<&D>,
) -> bool {
    let at = |p: &KeyPath| value(p, root, element, ctx);
    match c {
        Cond::Present(p, want) => at(p).is_some() == *want,
        Cond::Eq(p, v) => equal(at(p).as_ref(), v),
        Cond::In(p, vs) => {
            let d = at(p);
            vs.iter().any(|v| equal(d.as_ref(), v))
        }
        Cond::Missing(p) => at(p).is_none(),
        Cond::Empty(p) => at(p).is_none_or(|d| empty(&d)),
        Cond::All(cs) => cs.iter().all(|c| holds(c, root, element, ctx)),
    }
}

/// `""`, `[]` or `{}`.
fn empty<D: Doc>(d: &D) -> bool {
    match d.leaf() {
        V::Str(s) => s.is_empty(),
        V::Container => d.len() == Some(0) || d.members().is_some_and(|m| m.is_empty()),
        _ => false,
    }
}

/// A document value (`None`: missing or null) against a JSON value. Numbers compare by value,
/// containers as JSON (object key order does not count), and nothing else crosses types.
fn equal<D: Doc>(d: Option<&D>, v: &Value) -> bool {
    let Some(d) = d else {
        return v.is_null();
    };
    match (d.leaf(), v) {
        (V::Bool(a), Value::Bool(b)) => a == *b,
        (V::Str(a), Value::String(b)) => a == b.as_str(),
        (V::Int(a), Value::Number(b)) if b.is_i64() => Some(a) == b.as_i64(),
        (a @ (V::Int(_) | V::Float(_) | V::BigInt(_)), Value::Number(b)) => {
            number(&a) == b.as_f64()
        }
        (V::Container, Value::Array(_) | Value::Object(_)) => {
            json(d, 0, &mut { MAX_NODES }).as_ref() == Some(v)
        }
        _ => false,
    }
}

fn number(v: &V) -> Option<f64> {
    match v {
        V::Int(i) => Some(*i as f64),
        V::Float(f) => Some(*f),
        V::BigInt(s) => s.parse().ok(),
        _ => None,
    }
}

/// A value as JSON text with object keys sorted, so equal values give equal text. `None` past the
/// copy limits (a host object that contains itself).
pub(crate) fn key_text<D: Doc>(d: &D) -> Option<String> {
    fn sort(v: &mut Value) {
        match v {
            Value::Object(m) => {
                m.sort_keys();
                m.values_mut().for_each(sort);
            }
            Value::Array(a) => a.iter_mut().for_each(sort),
            _ => {}
        }
    }
    let mut v = json(d, 0, &mut { MAX_NODES })?;
    sort(&mut v);
    Some(v.to_string())
}

/// A value's place in the shared order of sorting and `min`/`max`: numbers by value, then strings
/// by code point, then booleans (`false` first), then containers by their key text, then null.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Rank {
    Num(f64),
    Str(String),
    Bool(bool),
    Text(String),
    Null,
}

impl Rank {
    /// `d` read for sorting. `None` (missing or null) is `Null`.
    pub(crate) fn of<D: Doc>(d: Option<&D>) -> Rank {
        let Some(d) = d else { return Rank::Null };
        match d.leaf() {
            V::Null => Rank::Null,
            V::Bool(b) => Rank::Bool(b),
            V::Str(s) => Rank::Str(s.into_owned()),
            V::Container => Rank::Text(key_text(d).unwrap_or_default()),
            n => number(&n).map_or(Rank::Null, Rank::Num),
        }
    }

    fn kind(&self) -> u8 {
        match self {
            Rank::Num(_) => 0,
            Rank::Str(_) => 1,
            Rank::Bool(_) => 2,
            Rank::Text(_) => 3,
            Rank::Null => 4,
        }
    }

    /// The order ascending, or descending with null still last.
    pub(crate) fn cmp(&self, o: &Rank, desc: bool) -> Ordering {
        let asc = match (self, o) {
            (Rank::Num(a), Rank::Num(b)) => a.partial_cmp(b).unwrap_or(Ordering::Equal),
            (Rank::Str(a), Rank::Str(b)) | (Rank::Text(a), Rank::Text(b)) => a.cmp(b),
            (Rank::Bool(a), Rank::Bool(b)) => a.cmp(b),
            _ => self.kind().cmp(&o.kind()),
        };
        match desc && *self != Rank::Null && *o != Rank::Null {
            true => asc.reverse(),
            false => asc,
        }
    }
}

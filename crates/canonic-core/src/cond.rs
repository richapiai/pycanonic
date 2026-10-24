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

const REJECT_SHAPE: &str = "must be {\"path\": <path>, \"eq\" | \"in\" | \"present\": <value>}, \
                            {\"missing\": <path>} or {\"empty\": <path>}";

/// A `reject` condition: the condition forms of `parse`, or `{"missing": p}` or `{"empty": p}`.
pub fn parse_reject(v: &Value) -> Result<Cond, Vec<(String, String)>> {
    let one = v
        .as_object()
        .filter(|o| o.len() == 1)
        .and_then(|o| o.iter().next());
    let Some((k, p)) = one.filter(|(k, _)| *k == "missing" || *k == "empty") else {
        return parse(v).map_err(|issues| match issues.as_slice() {
            [(at, m)] if at.is_empty() && m == SHAPE => vec![(String::new(), REJECT_SHAPE.into())],
            _ => issues,
        });
    };
    let Some(path) = p.as_str().and_then(parse_path) else {
        let m = "must be a path like a.b[0][\"c d\"]";
        return Err(vec![(format!(".{k}"), m.into())]);
    };
    Ok(match k.as_str() {
        "missing" => Cond::Missing(path),
        _ => Cond::Empty(path),
    })
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

/// A document value (`None`: missing or null) against a JSON value. Numbers compare by value, also
/// inside containers, which compare by their key text. Nothing else crosses types.
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
            key_text(d).is_some_and(|t| t == text(v.clone()))
        }
        _ => false,
    }
}

pub(crate) fn number(v: &V) -> Option<f64> {
    match v {
        V::Int(i) => Some(*i as f64),
        V::Float(f) => Some(*f),
        V::BigInt(s) => s.parse().ok(),
        _ => None,
    }
}

/// A value as JSON text with object keys sorted and integral numbers written as integers, so
/// values that `eq`, `unique` and grouping call equal give equal text (`{"a": 1.0}` is `{"a":1}`).
/// `None` past the copy limits (a host object that contains itself).
pub(crate) fn key_text<D: Doc>(d: &D) -> Option<String> {
    json(d, 0, &mut { MAX_NODES }).map(text)
}

/// `key_text` of a JSON value.
fn text(mut v: Value) -> String {
    fn canon(v: &mut Value) {
        match v {
            Value::Object(m) => {
                m.sort_keys();
                m.values_mut().for_each(canon);
            }
            Value::Array(a) => a.iter_mut().for_each(canon),
            Value::Number(n) => {
                // past 2^63 an integral float stays float text, so it equals no integer.
                if let Some(f) = n.as_f64().filter(|f| f.fract() == 0.0 && f.abs() < 9.2e18) {
                    *v = Value::from(f as i64);
                }
            }
            _ => {}
        }
    }
    canon(&mut v);
    v.to_string()
}

/// A value's place in the shared order of sorting and `min`/`max`: numbers by value, then strings
/// by code point, then booleans (`false` first), then containers by their key text, then null
/// (with a container past the copy limits).
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
            // Past the copy limits it has no text, so it ranks last, as null does.
            V::Container => key_text(d).map_or(Rank::Null, Rank::Text),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::Key;
    use std::borrow::Cow;

    /// An array that holds itself, deeper than any copy limit.
    #[derive(Clone)]
    struct Deep;

    impl Doc for Deep {
        fn get(&self, _: &Key) -> Option<Self> {
            None
        }
        fn idx(&self, _: usize) -> Option<Self> {
            Some(Deep)
        }
        fn leaf(&self) -> V<'_> {
            V::Container
        }
        fn is_null(&self) -> bool {
            false
        }
        fn len(&self) -> Option<usize> {
            Some(1)
        }
        fn item(&self, _: usize) -> Option<Self> {
            Some(Deep)
        }
        fn members(&self) -> Option<Vec<(Cow<'_, str>, Self)>> {
            None
        }
    }

    #[test]
    fn a_container_past_the_copy_limits_ranks_last() {
        assert_eq!(key_text(&Deep), None);
        let deep = Rank::of(Some(&Deep));
        assert_eq!(deep, Rank::Null);
        let empty = Rank::Text("[]".into());
        assert_eq!(empty.cmp(&deep, false), Ordering::Less);
        assert_eq!(Rank::Str(String::new()).cmp(&deep, true), Ordering::Less);
    }
}

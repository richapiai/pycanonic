//! Conditions, their parsing and their evaluation over a `Doc`. The plan holds them
//! with spec paths (`Cond`), the executor with its own interned paths (`Cond<KeyPath>`), which
//! `holds` reads. Also the value order and the JSON key text that sorting, `unique` and grouping
//! share.

use crate::doc::{Doc, KeyPath, KeyStep, read};
use crate::exec::{MAX_NODES, json};
use crate::spec::{Path, parse_path};
use crate::strings::Pattern;
use crate::value::Value as V;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::cmp::Ordering;

/// The right side of a compare: a number, or the number a path reads.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Operand<P> {
    Num(f64),
    Path(P),
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Cmp {
    Lt,
    Lte,
    Gt,
    Gte,
}

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
    /// `{"path": p, "matches": "<regex>"}`: a string the regex finds a match in.
    Matches(P, Pattern),
    /// `{"path": p, "starts_with": s}`: a string.
    StartsWith(P, String),
    /// `{"path": p, "ends_with": s}`: a string.
    EndsWith(P, String),
    /// `{"path": p, "contains": v}`: a substring of a string, or an element of an array equal to v.
    Contains(P, Value),
    /// `{"path": p, "lt" | "lte" | "gt" | "gte": n | {"path": q}}`: numbers only.
    Compare(P, Cmp, Operand<P>),
    /// `{"not": c}`.
    Not(Box<Cond<P>>),
    /// `{"any": [c, ...]}`. An empty list does not hold.
    Any(Vec<Cond<P>>),
}

impl<P> Cond<P> {
    /// Every path the condition reads, also inside combinators and as a compare operand.
    pub fn paths(&self) -> Vec<&P> {
        match self {
            Cond::Present(p, _)
            | Cond::Eq(p, _)
            | Cond::In(p, _)
            | Cond::Missing(p)
            | Cond::Empty(p)
            | Cond::Matches(p, _)
            | Cond::StartsWith(p, _)
            | Cond::EndsWith(p, _)
            | Cond::Contains(p, _) => vec![p],
            Cond::Compare(p, _, Operand::Num(_)) => vec![p],
            Cond::Compare(p, _, Operand::Path(q)) => vec![p, q],
            Cond::Not(c) => c.paths(),
            Cond::All(cs) | Cond::Any(cs) => cs.iter().flat_map(|c| c.paths()).collect(),
        }
    }
}

const SHAPE: &str = "must be {\"path\": <path>, \"eq\" | \"in\" | \"present\" | \"matches\" | \"starts_with\" | \"ends_with\" | \"contains\" | \"lt\" | \"lte\" | \"gt\" | \"gte\": <value>}, {\"not\": <condition>}, {\"all\": [<condition>, ...]} or {\"any\": [<condition>, ...]}";
const REJECT_SHAPE: &str = "must be {\"path\": <path>, \"eq\" | \"in\" | \"present\" | \"matches\" | \"starts_with\" | \"ends_with\" | \"contains\" | \"lt\" | \"lte\" | \"gt\" | \"gte\": <value>}, {\"not\": <condition>}, {\"all\": [<condition>, ...]}, {\"any\": [<condition>, ...]}, {\"missing\": <path>} or {\"empty\": <path>}";
// The language 1 texts stay as 0.1.1 wrote them.
const SHAPE_V1: &str = "must be {\"path\": <path>, \"eq\" | \"in\" | \"present\": <value>}";
const REJECT_SHAPE_V1: &str = "must be {\"path\": <path>, \"eq\" | \"in\" | \"present\": <value>}, \
                               {\"missing\": <path>} or {\"empty\": <path>}";
const PATH_SHAPE: &str = "must be a path like a.b[0][\"c d\"]";
const ALL_TESTS: [&str; 11] = [
    "eq",
    "in",
    "present",
    "matches",
    "starts_with",
    "ends_with",
    "contains",
    "lt",
    "lte",
    "gt",
    "gte",
];
const COMBINATORS: [&str; 3] = ["not", "all", "any"];

type Issues = Vec<(String, String)>;

/// A condition as the spec writes it: the condition forms (`eq`, `in`, `present`), the string tests
/// (`matches`, `starts_with`, `ends_with`, `contains`), the compares (`lt`, `lte`, `gt`, `gte`) and
/// the combinators `not`, `all` and `any`. `Err` holds every issue as (location suffix, message),
/// for example `(".in", "must be an array")`.
pub fn parse(v: &Value) -> Result<Cond, Issues> {
    go(v, None, false, false)
}

/// A condition in the language 1 grammar: only `eq`, `in` and `present`. A spec without
/// `"language": 2` is checked with it, so a new form fails there as it did before.
pub fn parse_v1(v: &Value) -> Result<Cond, Issues> {
    go(v, None, false, true)
}

/// `parse_reject` in the language 1 grammar.
pub fn parse_reject_v1(v: &Value) -> Result<Cond, Issues> {
    go(v, None, true, true)
}

/// A condition whose `path` may be left out: without it the condition tests `own`.
pub fn parse_relative(v: &Value, own: &Path) -> Result<Cond, Issues> {
    go(v, Some(own), false, false)
}

/// A `reject` condition: the forms of `parse`, or `{"missing": p}` or `{"empty": p}`.
pub fn parse_reject(v: &Value) -> Result<Cond, Issues> {
    go(v, None, true, false)
}

fn go(v: &Value, own: Option<&Path>, reject: bool, v1: bool) -> Result<Cond, Issues> {
    let tests = if v1 { &ALL_TESTS[..3] } else { &ALL_TESTS[..] };
    let shape = || {
        let m = match (reject, v1) {
            (true, true) => REJECT_SHAPE_V1,
            (true, false) => REJECT_SHAPE,
            (false, true) => SHAPE_V1,
            (false, false) => SHAPE,
        };
        vec![(String::new(), m.to_string())]
    };
    let Some(o) = v.as_object() else {
        return Err(shape());
    };
    if !o.contains_key("path") {
        let comb = o
            .iter()
            .find(|(k, _)| COMBINATORS.contains(&k.as_str()) && !v1);
        if let Some((k, p)) = comb {
            if o.len() != 1 {
                let m = "a combinator takes no other key";
                return Err(vec![(String::new(), m.into())]);
            }
            return combinator(k, p, own, reject);
        }
        let one = Some(o)
            .filter(|o| o.len() == 1)
            .and_then(|o| o.iter().next());
        if let Some((k, p)) = one.filter(|(k, _)| reject && (*k == "missing" || *k == "empty")) {
            let Some(path) = p.as_str().and_then(parse_path) else {
                return Err(vec![(format!(".{k}"), PATH_SHAPE.into())]);
            };
            return Ok(match k.as_str() {
                "missing" => Cond::Missing(path),
                _ => Cond::Empty(path),
            });
        }
        if own.is_none() {
            return Err(shape());
        }
    }
    let mut issues = Vec::new();
    for k in o.keys() {
        if k != "path" && !tests.contains(&k.as_str()) {
            issues.push((format!(".{k}"), "unknown property".into()));
        }
    }
    let path = match o.get("path") {
        Some(p) => p.as_str().and_then(parse_path),
        None => own.cloned(),
    };
    if path.is_none() {
        issues.push((".path".into(), PATH_SHAPE.into()));
    }
    let found: Vec<&str> = tests
        .iter()
        .copied()
        .filter(|k| o.contains_key(*k))
        .collect();
    if found.len() != 1 {
        let m = format!("needs exactly one of {}", tests.join(", "));
        issues.push((String::new(), m));
    }
    let c = match found.first().map(|k| (*k, &o[*k])) {
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
        Some(("matches", Value::String(s))) => match Pattern::compile(s) {
            Ok(re) => path.map(|p| Cond::Matches(p, re)),
            Err(e) => {
                let e = e.to_string();
                let m = e.lines().last().unwrap_or("").trim();
                let m = m.strip_prefix("error: ").unwrap_or(m);
                issues.push((".matches".into(), format!("invalid regex: {m}")));
                None
            }
        },
        Some((k @ ("starts_with" | "ends_with"), Value::String(s))) => {
            let s = s.clone();
            path.map(|p| match k {
                "starts_with" => Cond::StartsWith(p, s),
                _ => Cond::EndsWith(p, s),
            })
        }
        Some((k @ ("matches" | "starts_with" | "ends_with"), _)) => {
            issues.push((format!(".{k}"), "must be a string".into()));
            None
        }
        Some(("contains", v)) => path.map(|p| Cond::Contains(p, v.clone())),
        Some((k @ ("lt" | "lte" | "gt" | "gte"), v)) => {
            let op = match (v.as_f64(), v.as_object().and_then(|o| o.get("path"))) {
                (Some(n), _) if v.is_number() => Some(Operand::Num(n)),
                (_, Some(q)) if v.as_object().is_some_and(|o| o.len() == 1) => {
                    match q.as_str().and_then(parse_path) {
                        Some(q) => Some(Operand::Path(q)),
                        None => {
                            issues.push((format!(".{k}.path"), PATH_SHAPE.into()));
                            None
                        }
                    }
                }
                _ => {
                    let m = "must be a number or {\"path\": <path>}";
                    issues.push((format!(".{k}"), m.into()));
                    None
                }
            };
            let cmp = match k {
                "lt" => Cmp::Lt,
                "lte" => Cmp::Lte,
                "gt" => Cmp::Gt,
                _ => Cmp::Gte,
            };
            path.zip(op).map(|(p, op)| Cond::Compare(p, cmp, op))
        }
        Some((_, v)) => path.map(|p| Cond::Eq(p, v.clone())),
        None => None,
    };
    match c {
        Some(c) if issues.is_empty() => Ok(c),
        _ => Err(issues),
    }
}

/// `{"not": c}`, `{"all": [c, ...]}` or `{"any": [c, ...]}`. The inner conditions use the same
/// grammar, so they can be combinators too.
fn combinator(k: &str, v: &Value, own: Option<&Path>, reject: bool) -> Result<Cond, Issues> {
    let at = |i: usize, sub: Issues| {
        (sub.into_iter())
            .map(|(s, m)| (format!(".{k}[{i}]{s}"), m))
            .collect::<Issues>()
    };
    if k == "not" {
        return match go(v, own, reject, false) {
            Ok(c) => Ok(Cond::Not(Box::new(c))),
            Err(e) => Err(e
                .into_iter()
                .map(|(s, m)| (format!(".not{s}"), m))
                .collect()),
        };
    }
    let Some(a) = v.as_array() else {
        return Err(vec![(
            format!(".{k}"),
            "must be an array of conditions".into(),
        )]);
    };
    let (mut cs, mut issues) = (Vec::new(), Vec::new());
    for (i, c) in a.iter().enumerate() {
        match go(c, own, reject, false) {
            Ok(c) => cs.push(c),
            Err(e) => issues.extend(at(i, e)),
        }
    }
    match (issues.is_empty(), k) {
        (false, _) => Err(issues),
        (true, "all") => Ok(Cond::All(cs)),
        (true, _) => Ok(Cond::Any(cs)),
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
        Cond::Any(cs) => cs.iter().any(|c| holds(c, root, element, ctx)),
        Cond::Not(c) => !holds(c, root, element, ctx),
        Cond::Matches(p, re) => on_str(at(p), |s| re.is_match(s)),
        Cond::StartsWith(p, t) => on_str(at(p), |s| s.starts_with(t.as_str())),
        Cond::EndsWith(p, t) => on_str(at(p), |s| s.ends_with(t.as_str())),
        Cond::Contains(p, v) => at(p).is_some_and(|d| match (d.leaf(), v) {
            (V::Str(s), Value::String(t)) => s.contains(t.as_str()),
            (V::Container, v) => (0..d.len().unwrap_or(0)).any(|i| equal(d.item(i).as_ref(), v)),
            _ => false,
        }),
        Cond::Compare(p, cmp, rhs) => {
            let r = match rhs {
                Operand::Num(n) => Some(*n),
                Operand::Path(q) => at(q).and_then(|d| finite(&d)),
            };
            match (at(p).and_then(|d| finite(&d)), r) {
                (Some(a), Some(b)) => match cmp {
                    Cmp::Lt => a < b,
                    Cmp::Lte => a <= b,
                    Cmp::Gt => a > b,
                    Cmp::Gte => a >= b,
                },
                _ => false,
            }
        }
    }
}

/// Whether `f` holds for the text of a string value. Nothing else reads as text.
fn on_str<D: Doc>(d: Option<D>, f: impl Fn(&str) -> bool) -> bool {
    d.is_some_and(|d| matches!(d.leaf(), V::Str(s) if f(&s)))
}

/// A finite number: an `Int` or `Float` value. A string or a `BigInt` is not one.
fn finite<D: Doc>(d: &D) -> Option<f64> {
    match d.leaf() {
        V::Int(i) => Some(i as f64),
        V::Float(f) if f.is_finite() => Some(f),
        _ => None,
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
/// `None` past the copy limits (a host object that contains itself) and for NaN, which is null
/// to `unique` and grouping (R42).
pub(crate) fn key_text<D: Doc>(d: &D) -> Option<String> {
    if matches!(d.leaf(), V::Float(f) if f.is_nan()) {
        return None;
    }
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
                // Past 2^63 an integral float stays float text, so it equals no integer.
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
/// (with NaN and a container past the copy limits).
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
            // NaN ranks as null (R42): with it the order is not total and the sort panics.
            n => number(&n)
                .filter(|f| !f.is_nan())
                .map_or(Rank::Null, Rank::Num),
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

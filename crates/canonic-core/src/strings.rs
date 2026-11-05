//! The string transforms of language 2: regex, strip_prefix, strip_suffix, replace, lookup,
//! null_if, upper, title and split. `transforms.rs` dispatches to them. Each returns `None` (null)
//! on input it cannot handle and never panics.

use crate::transforms::{Builtin, owned, to_text};
use crate::value::Value;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value as J};
use std::borrow::Cow;

/// A regular expression, compiled once when the plan is built or loaded. A plan holds its text.
#[derive(Debug, Clone)]
pub struct Pattern(regex::Regex);

impl Pattern {
    pub(crate) fn new(s: &str) -> Option<Pattern> {
        Pattern::compile(s).ok()
    }

    pub(crate) fn compile(s: &str) -> Result<Pattern, regex::Error> {
        regex::Regex::new(s).map(Pattern)
    }

    pub(crate) fn is_match(&self, s: &str) -> bool {
        self.0.is_match(s)
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// Group 1 when the pattern has a group, else the whole match.
    fn find<'s>(&self, s: &'s str) -> Option<&'s str> {
        if self.0.captures_len() > 1 {
            self.0.captures(s)?.get(1).map(|m| m.as_str())
        } else {
            self.0.find(s).map(|m| m.as_str())
        }
    }
}

impl PartialEq for Pattern {
    fn eq(&self, other: &Pattern) -> bool {
        self.0.as_str() == other.0.as_str()
    }
}

impl Serialize for Pattern {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.0.as_str())
    }
}

impl<'de> Deserialize<'de> for Pattern {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Pattern, D::Error> {
        let s = String::deserialize(d)?;
        Pattern::new(&s).ok_or_else(|| serde::de::Error::custom(format!("invalid regex {s:?}")))
    }
}

/// What `replace` looks for.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Find {
    Text(String),
    Pattern(Pattern),
}

/// The `else` of `lookup`: the result for a value the table does not have.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Otherwise {
    Keep,
    Null,
    Value(J),
}

/// A language 2 string transform on `v`. Other transforms are not handled here.
pub(crate) fn apply<'a>(b: &Builtin, v: Value<'a>) -> Option<Value<'a>> {
    use Builtin as B;
    match b {
        B::Regex(p) => part(v, |x| p.find(x)),
        B::StripPrefix(p) => part(v, |x| non_empty(x.strip_prefix(p.as_str()), x)),
        B::StripSuffix(p) => part(v, |x| non_empty(x.strip_suffix(p.as_str()), x)),
        B::Replace { find, to } => match v {
            Value::Str(s) => Some(Value::Str(replace(s, find, to))),
            _ => None,
        },
        B::Lookup { table, otherwise } => {
            let hit = key_text(&v).map(|k| table.get(k.as_ref()));
            match (hit, otherwise) {
                (None, _) => None,
                (Some(Some(j)), _) | (Some(None), Otherwise::Value(j)) => Some(from_json(j)),
                (Some(None), Otherwise::Keep) => Some(v),
                (Some(None), Otherwise::Null) => None,
            }
        }
        B::NullIf(xs) => match v {
            Value::Container => None,
            v => (!xs.iter().any(|x| same(&v, x))).then_some(v),
        },
        B::Upper => match v {
            Value::Str(s) => Some(owned(s.to_uppercase())),
            _ => None,
        },
        B::Title => match v {
            Value::Str(s) => Some(owned(title(&s))),
            _ => None,
        },
        B::Split { sep, index } => part(v, |x| {
            let parts: Vec<&str> = x.split(sep.as_str()).collect();
            let i = if *index < 0 {
                parts.len() as i64 + index
            } else {
                *index
            };
            usize::try_from(i)
                .ok()
                .and_then(|i| parts.get(i).copied())
                .filter(|p| !p.is_empty())
        }),
        _ => None,
    }
}

/// The argument as an object with all of `required` and nothing outside `required` and `optional`.
pub(crate) fn object<'j>(
    arg: Option<&'j J>,
    required: &[&str],
    optional: &[&str],
) -> Option<&'j Map<String, J>> {
    let o = arg?.as_object()?;
    let known = |k: &String| required.contains(&k.as_str()) || optional.contains(&k.as_str());
    (required.iter().all(|k| o.contains_key(*k)) && o.keys().all(known)).then_some(o)
}

pub(crate) fn replace_arg(arg: Option<&J>) -> Option<Builtin> {
    let o = object(arg, &["to"], &["from", "pattern"])?;
    let to = o["to"].as_str()?.to_string();
    let find = match (o.get("from"), o.get("pattern")) {
        (Some(J::String(f)), None) if !f.is_empty() => Find::Text(f.clone()),
        (None, Some(J::String(p))) => Find::Pattern(Pattern::new(p)?),
        _ => return None,
    };
    Some(Builtin::Replace { find, to })
}

pub(crate) fn lookup_arg(arg: Option<&J>) -> Option<Builtin> {
    let o = object(arg, &["table"], &["else"])?;
    let scalar = |j: &J| !matches!(j, J::Array(_) | J::Object(_));
    let table = o["table"].as_object().filter(|t| t.values().all(scalar))?;
    let otherwise = match o.get("else") {
        None | Some(J::Null) => Otherwise::Null,
        Some(J::String(s)) if s == "null" => Otherwise::Null,
        Some(J::String(s)) if s == "keep" => Otherwise::Keep,
        Some(j) if scalar(j) => Otherwise::Value(j.clone()),
        Some(_) => return None,
    };
    let table = table.clone();
    Some(Builtin::Lookup { table, otherwise })
}

pub(crate) fn split_arg(arg: Option<&J>) -> Option<Builtin> {
    let o = object(arg, &["sep", "index"], &[])?;
    let sep = o["sep"].as_str().filter(|s| !s.is_empty())?.to_string();
    let index = o["index"].as_i64()?;
    Some(Builtin::Split { sep, index })
}

/// The part of a string `f` picks, borrowed when the string is. `None` for anything else.
fn part<'a>(v: Value<'a>, f: impl Fn(&str) -> Option<&str>) -> Option<Value<'a>> {
    let Value::Str(s) = v else {
        return None;
    };
    Some(Value::Str(match s {
        Cow::Borrowed(b) => Cow::Borrowed(f(b)?),
        Cow::Owned(o) => Cow::Owned(f(&o)?.to_string()),
    }))
}

/// `stripped`, or `x` when nothing was stripped. A string stripped to nothing is `None`.
fn non_empty<'s>(stripped: Option<&'s str>, x: &'s str) -> Option<&'s str> {
    match stripped {
        Some("") => None,
        Some(rest) => Some(rest),
        None => Some(x),
    }
}

/// Every occurrence replaced. `to` is literal text, also for a pattern.
fn replace<'a>(s: Cow<'a, str>, find: &Find, to: &str) -> Cow<'a, str> {
    match find {
        Find::Text(f) if !s.contains(f.as_str()) => s,
        Find::Text(f) => Cow::Owned(s.replace(f.as_str(), to)),
        Find::Pattern(p) => match s {
            Cow::Borrowed(b) => p.0.replace_all(b, regex::NoExpand(to)),
            Cow::Owned(o) => Cow::Owned(p.0.replace_all(&o, regex::NoExpand(to)).into_owned()),
        },
    }
}

/// A scalar's string form, as `to_string` gives it, for a `lookup` key.
fn key_text<'v>(v: &'v Value) -> Option<Cow<'v, str>> {
    match v {
        Value::Str(s) => Some(Cow::Borrowed(s)),
        v => match to_text(v.clone()) {
            Some(Value::Str(s)) => Some(Cow::Owned(s.into_owned())),
            _ => None,
        },
    }
}

/// A JSON scalar as a `Value`. Arguments hold no containers.
fn from_json(j: &J) -> Value<'static> {
    match j {
        J::Bool(b) => Value::Bool(*b),
        J::Number(n) => match (n.as_i64(), n.as_u64()) {
            (Some(i), _) => Value::Int(i),
            (None, Some(u)) => Value::BigInt(Cow::Owned(u.to_string())),
            _ => n.as_f64().map_or(Value::Null, Value::Float),
        },
        J::String(s) => owned(s.clone()),
        _ => Value::Null,
    }
}

/// The comparison of `eq` and the conformance rule: numbers by value, nothing else crosses types.
fn same(v: &Value, j: &J) -> bool {
    match (v, j) {
        (Value::Bool(a), J::Bool(b)) => a == b,
        (Value::Str(a), J::String(b)) => a == b,
        (Value::Int(a), J::Number(b)) if b.is_i64() => Some(*a) == b.as_i64(),
        (a @ (Value::Int(_) | Value::Float(_) | Value::BigInt(_)), J::Number(b)) => {
            crate::cond::number(a) == b.as_f64()
        }
        _ => false,
    }
}

/// The first letter or digit of each word upper case, the rest lower case. A word starts after
/// anything but a letter, a digit or an apostrophe, so `jean-luc o'brien` is `Jean-Luc O'brien`.
fn title(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut start = true;
    for c in s.chars() {
        if start {
            out.extend(c.to_uppercase());
        } else {
            out.extend(c.to_lowercase());
        }
        start = !(c.is_alphanumeric() || c == '\'' || c == '\u{2019}');
    }
    out
}

/// A `format` template: text with `{}` (the one value) or `{n}` (source n) slots.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Template {
    parts: Vec<Part>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
enum Part {
    Text(String),
    Slot(usize),
    Next,
}

impl Template {
    /// `{{` and `}}` write a brace. `None` for no placeholder, `{}` mixed with `{n}`, an unclosed
    /// `{` or a stray `}`.
    pub(crate) fn parse(s: &str) -> Option<Template> {
        let (mut parts, mut text) = (Vec::new(), String::new());
        let mut chars = s.chars().peekable();
        while let Some(c) = chars.next() {
            match (c, chars.peek()) {
                ('{', Some('{')) | ('}', Some('}')) => {
                    chars.next();
                    text.push(c);
                }
                ('{', _) => {
                    let mut name = String::new();
                    loop {
                        match chars.next()? {
                            '}' => break,
                            c => name.push(c),
                        }
                    }
                    let part = match name.as_str() {
                        "" => Part::Next,
                        n if n.bytes().all(|b| b.is_ascii_digit()) => Part::Slot(n.parse().ok()?),
                        _ => return None,
                    };
                    if !text.is_empty() {
                        parts.push(Part::Text(std::mem::take(&mut text)));
                    }
                    parts.push(part);
                }
                ('}', _) => return None,
                _ => text.push(c),
            }
        }
        if !text.is_empty() {
            parts.push(Part::Text(text));
        }
        let next = parts.contains(&Part::Next);
        let slot = parts.iter().any(|p| matches!(p, Part::Slot(_)));
        (next != slot).then_some(Template { parts })
    }

    pub(crate) fn max_slot(&self) -> Option<usize> {
        self.parts
            .iter()
            .filter_map(|p| match p {
                Part::Slot(n) => Some(*n),
                _ => None,
            })
            .max()
    }

    pub(crate) fn uses_next(&self) -> bool {
        self.parts.contains(&Part::Next)
    }

    /// The template as a spec writes it.
    pub(crate) fn text(&self) -> String {
        self.parts
            .iter()
            .map(|p| match p {
                Part::Text(t) => t.replace('{', "{{").replace('}', "}}"),
                Part::Slot(n) => format!("{{{n}}}"),
                Part::Next => "{}".into(),
            })
            .collect()
    }

    /// Fills the slots from `items`. Null if any slot's item is null, missing or a container.
    pub(crate) fn render<'a>(&self, items: &[Value<'a>]) -> Option<Value<'a>> {
        let mut out = String::new();
        for p in &self.parts {
            let n = match p {
                Part::Text(t) => {
                    out.push_str(t);
                    continue;
                }
                Part::Slot(n) => *n,
                Part::Next => 0,
            };
            match to_text(items.get(n)?.clone())? {
                Value::Str(s) => out.push_str(&s),
                _ => return None,
            }
        }
        Some(owned(out))
    }
}

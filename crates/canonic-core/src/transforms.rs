//! Default transforms and routing validators. Every one returns `Null` on input it cannot handle
//! and never panics. The executor counts a `Null` from non-null input as a field error.
//! Port of canonic/packages/canonic/src/transforms.ts.

use crate::canonical::Semantic;
use crate::dates::{self, DateOut, Layout, date_from_epoch, date_from_string};
use crate::strings::{self, Find, Otherwise, Pattern};
use crate::value::{Value, js_number_to_string};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value as J};
use std::borrow::Cow;

pub trait Transform: Send + Sync {
    fn apply<'a>(&self, v: Value<'a>, arg: Option<&serde_json::Value>) -> Value<'a>;

    /// A custom transform's object or array input (a several-path `from` is an array), as JSON,
    /// since JavaScript hands the injected function the real object. The default calls `apply` with
    /// `Value::Container`. Integers past i64 inside the container arrive as floats.
    fn apply_container(
        &self,
        _v: &serde_json::Value,
        arg: Option<&serde_json::Value>,
    ) -> Value<'static> {
        self.apply(Value::Container, arg).into_owned()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CastTo {
    String,
    Number,
    Integer,
    Boolean,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NamePart {
    First,
    Last,
}

/// The end of a range that `parse_range` gives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RangeEnd {
    Min,
    Max,
}

/// A default transform with its argument already checked and bound.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Builtin {
    Cast(CastTo),
    Lower,
    Trim,
    SplitName(NamePart),
    Join(Option<String>),
    ParseDate,
    NormalizePhone,
    NormalizeLinkedin,
    ToDomain,
    // Language 2.
    Regex(Pattern),
    StripPrefix(String),
    StripSuffix(String),
    Replace {
        find: Find,
        to: String,
    },
    Lookup {
        table: Map<String, J>,
        otherwise: Otherwise,
    },
    NullIf(Vec<J>),
    Upper,
    Title,
    Split {
        sep: String,
        index: i64,
    },
    ToString,
    FormatDate(DateOut),
    /// `parse_date` with `{"formats": [...]}`. Without an argument it is `ParseDate`.
    ParseDateFormats(Vec<Layout>),
    ParseNumber {
        units: bool,
    },
    ParseRange(RangeEnd),
}

/// Why `builtin` refused: the name is not a default transform, or its argument is wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArgError {
    UnknownTransform,
    InvalidArgument,
}

/// How many of the default transforms language 1 has. They come first in the list.
pub const LANGUAGE_1_TRANSFORMS: usize = 9;

const NAMES: [&str; 22] = [
    "cast",
    "lower",
    "trim",
    "split_name",
    "join",
    "parse_date",
    "normalize_phone",
    "normalize_linkedin",
    "to_domain",
    // Language 2 only, from here on.
    "regex",
    "strip_prefix",
    "strip_suffix",
    "replace",
    "lookup",
    "null_if",
    "upper",
    "title",
    "split",
    "to_string",
    "format_date",
    "parse_number",
    "parse_range",
];

/// True for a default transform that only a language 2 spec can name.
pub(crate) fn language_2_only(name: &str) -> bool {
    NAMES[LANGUAGE_1_TRANSFORMS..].contains(&name)
}

pub fn default_transform_names() -> &'static [&'static str] {
    &NAMES
}

/// Resolves a default transform by name, checking its argument like `transformArgs`.
pub fn builtin(name: &str, arg: Option<&serde_json::Value>) -> Result<Builtin, ArgError> {
    use serde_json::Value as J;
    let text = match arg {
        Some(J::String(s)) => Some(s.as_str()),
        _ => None,
    };
    let no_arg = |b: Builtin| {
        if arg.is_none() {
            Ok(b)
        } else {
            Err(ArgError::InvalidArgument)
        }
    };
    match name {
        "cast" => match text {
            Some("string") => Ok(Builtin::Cast(CastTo::String)),
            Some("number") => Ok(Builtin::Cast(CastTo::Number)),
            Some("integer") => Ok(Builtin::Cast(CastTo::Integer)),
            Some("boolean") => Ok(Builtin::Cast(CastTo::Boolean)),
            _ => Err(ArgError::InvalidArgument),
        },
        "split_name" => match text {
            Some("first") => Ok(Builtin::SplitName(NamePart::First)),
            Some("last") => Ok(Builtin::SplitName(NamePart::Last)),
            _ => Err(ArgError::InvalidArgument),
        },
        "join" => match arg {
            None => Ok(Builtin::Join(None)),
            Some(J::String(s)) => Ok(Builtin::Join(Some(s.clone()))),
            Some(_) => Err(ArgError::InvalidArgument),
        },
        "lower" => no_arg(Builtin::Lower),
        "trim" => no_arg(Builtin::Trim),
        "parse_date" if arg.is_none() => Ok(Builtin::ParseDate),
        "parse_date" => dates::formats_arg(arg)
            .map(Builtin::ParseDateFormats)
            .ok_or(ArgError::InvalidArgument),
        "format_date" => text
            .and_then(DateOut::new)
            .map(Builtin::FormatDate)
            .ok_or(ArgError::InvalidArgument),
        "parse_number" => match arg {
            None => Ok(Builtin::ParseNumber { units: false }),
            a => strings::object(a, &["units"], &[])
                .and_then(|o| o["units"].as_bool())
                .map(|units| Builtin::ParseNumber { units })
                .ok_or(ArgError::InvalidArgument),
        },
        "parse_range" => match text {
            Some("min") => Ok(Builtin::ParseRange(RangeEnd::Min)),
            Some("max") => Ok(Builtin::ParseRange(RangeEnd::Max)),
            _ => Err(ArgError::InvalidArgument),
        },
        "normalize_phone" => no_arg(Builtin::NormalizePhone),
        "normalize_linkedin" => no_arg(Builtin::NormalizeLinkedin),
        "to_domain" => no_arg(Builtin::ToDomain),
        "upper" => no_arg(Builtin::Upper),
        "title" => no_arg(Builtin::Title),
        "to_string" => no_arg(Builtin::ToString),
        "regex" => text
            .and_then(Pattern::new)
            .map(Builtin::Regex)
            .ok_or(ArgError::InvalidArgument),
        "strip_prefix" | "strip_suffix" => match text {
            Some(s) if !s.is_empty() && name == "strip_prefix" => {
                Ok(Builtin::StripPrefix(s.into()))
            }
            Some(s) if !s.is_empty() => Ok(Builtin::StripSuffix(s.into())),
            _ => Err(ArgError::InvalidArgument),
        },
        "null_if" => match arg {
            Some(J::Array(xs)) => Ok(Builtin::NullIf(xs.clone())),
            _ => Err(ArgError::InvalidArgument),
        },
        "replace" => strings::replace_arg(arg).ok_or(ArgError::InvalidArgument),
        "lookup" => strings::lookup_arg(arg).ok_or(ArgError::InvalidArgument),
        "split" => strings::split_arg(arg).ok_or(ArgError::InvalidArgument),
        _ => Err(ArgError::UnknownTransform),
    }
}

impl Transform for Builtin {
    /// The argument is bound in the variant. `_arg` is for custom transforms only.
    fn apply<'a>(&self, v: Value<'a>, _arg: Option<&serde_json::Value>) -> Value<'a> {
        let out = match self {
            // An Int is already a whole number. Keep it exact instead of rounding through f64.
            Builtin::Cast(CastTo::Number | CastTo::Integer) if matches!(v, Value::Int(_)) => {
                Some(v)
            }
            Builtin::Cast(CastTo::Number) => to_number(&v).map(number),
            Builtin::Cast(CastTo::Integer) => to_integer(&v).map(number),
            Builtin::Cast(CastTo::Boolean) => to_boolean(&v).map(Value::Bool),
            Builtin::Cast(CastTo::String) => to_text(v),
            Builtin::Lower => match v {
                Value::Str(s) => Some(Value::Str(lower(s))),
                _ => None,
            },
            Builtin::Trim => match v {
                Value::Str(s) => Some(Value::Str(sub(s, js_trim))),
                _ => None,
            },
            Builtin::SplitName(part) => match v {
                Value::Str(s) if split_name(&s, *part).is_some() => {
                    Some(Value::Str(sub(s, |x| split_name(x, *part).unwrap_or(x))))
                }
                _ => None,
            },
            Builtin::Join(_) => match v {
                Value::Str(s) if !s.is_empty() => Some(Value::Str(s)),
                _ => None,
            },
            Builtin::ParseDate => match v {
                Value::Str(s) if date_from_string(&s).is_some() => {
                    Some(Value::Str(sub(s, |x| date_from_string(x).unwrap_or(x))))
                }
                Value::Str(_) => None,
                v => as_f64(&v).and_then(date_from_epoch).map(owned),
            },
            Builtin::NormalizePhone => normalize_phone(&v).map(owned),
            Builtin::NormalizeLinkedin => as_str(&v).and_then(normalize_linkedin).map(owned),
            Builtin::ToDomain => match v {
                Value::Str(s) => to_domain(s).map(Value::Str),
                _ => None,
            },
            b @ (Builtin::Regex(_)
            | Builtin::StripPrefix(_)
            | Builtin::StripSuffix(_)
            | Builtin::Replace { .. }
            | Builtin::Lookup { .. }
            | Builtin::NullIf(_)
            | Builtin::Upper
            | Builtin::Title
            | Builtin::Split { .. }) => strings::apply(b, v),
            Builtin::ToString => to_text(v),
            Builtin::FormatDate(out) => dates::format_date(&v, *out),
            Builtin::ParseDateFormats(layouts) => match dates::read_formats(layouts, &v) {
                Some(s) => Some(owned(s)),
                None => return Builtin::ParseDate.apply(v, None),
            },
            Builtin::ParseNumber { units } => parse_number(&v, *units).map(number),
            Builtin::ParseRange(end) => parse_range(&v, *end).map(number),
        };
        out.unwrap_or(Value::Null)
    }
}

impl Builtin {
    /// The name and argument as a spec writes them.
    pub fn spec(&self) -> (&'static str, Option<J>) {
        let text = |s: &str| Some(J::from(s));
        match self {
            Builtin::Cast(c) => (
                "cast",
                text(match c {
                    CastTo::String => "string",
                    CastTo::Number => "number",
                    CastTo::Integer => "integer",
                    CastTo::Boolean => "boolean",
                }),
            ),
            Builtin::SplitName(NamePart::First) => ("split_name", text("first")),
            Builtin::SplitName(NamePart::Last) => ("split_name", text("last")),
            Builtin::Join(sep) => ("join", sep.as_deref().and_then(text)),
            Builtin::Lower => ("lower", None),
            Builtin::Trim => ("trim", None),
            Builtin::ParseDate => ("parse_date", None),
            Builtin::NormalizePhone => ("normalize_phone", None),
            Builtin::NormalizeLinkedin => ("normalize_linkedin", None),
            Builtin::ToDomain => ("to_domain", None),
            Builtin::Regex(p) => ("regex", text(p.as_str())),
            Builtin::StripPrefix(s) => ("strip_prefix", text(s)),
            Builtin::StripSuffix(s) => ("strip_suffix", text(s)),
            Builtin::Replace { find, to } => {
                let (k, f) = match find {
                    Find::Text(s) => ("from", s.as_str()),
                    Find::Pattern(p) => ("pattern", p.as_str()),
                };
                ("replace", Some(serde_json::json!({k: f, "to": to})))
            }
            Builtin::Lookup { table, otherwise } => {
                let e = match otherwise {
                    Otherwise::Keep => J::from("keep"),
                    Otherwise::Null => J::from("null"),
                    Otherwise::Value(j) => j.clone(),
                };
                (
                    "lookup",
                    Some(serde_json::json!({"table": table, "else": e})),
                )
            }
            Builtin::NullIf(xs) => ("null_if", Some(J::Array(xs.clone()))),
            Builtin::Upper => ("upper", None),
            Builtin::Title => ("title", None),
            Builtin::Split { sep, index } => (
                "split",
                Some(serde_json::json!({"sep": sep, "index": index})),
            ),
            Builtin::ToString => ("to_string", None),
            Builtin::FormatDate(out) => ("format_date", text(out.name())),
            Builtin::ParseDateFormats(layouts) => {
                let list: Vec<&str> = layouts.iter().map(Layout::as_str).collect();
                ("parse_date", Some(serde_json::json!({ "formats": list })))
            }
            Builtin::ParseNumber { units: false } => ("parse_number", None),
            Builtin::ParseNumber { units: true } => {
                ("parse_number", Some(serde_json::json!({"units": true})))
            }
            Builtin::ParseRange(RangeEnd::Min) => ("parse_range", text("min")),
            Builtin::ParseRange(RangeEnd::Max) => ("parse_range", text("max")),
        }
    }

    /// True for a transform only language 2 has, which needs plan format 2.
    pub(crate) fn is_language_2(&self) -> bool {
        use Builtin as B;
        !matches!(
            self,
            B::Cast(_)
                | B::Lower
                | B::Trim
                | B::SplitName(_)
                | B::Join(_)
                | B::ParseDate
                | B::NormalizePhone
                | B::NormalizeLinkedin
                | B::ToDomain
        )
    }

    /// Applies the transform to a JS array, as `from` with several paths produces. Only `join`
    /// reads one. Every other default transform returns `Null` on an array.
    pub fn apply_list<'a>(&self, items: &[Value<'a>]) -> Value<'a> {
        let Builtin::Join(sep) = self else {
            return Value::Null;
        };
        let sep = sep.as_deref().unwrap_or(" ");
        let mut out = String::new();
        for x in items {
            let text = match x {
                Value::Str(s) if !s.is_empty() => Cow::Borrowed(s.as_ref()),
                Value::Int(i) => Cow::Owned(i.to_string()),
                Value::BigInt(d) => Cow::Borrowed(d.as_ref()),
                Value::Float(f) if f.is_finite() => Cow::Owned(js_number_to_string(*f)),
                _ => continue,
            };
            if !out.is_empty() {
                out.push_str(sep);
            }
            out.push_str(&text);
        }
        if out.is_empty() {
            Value::Null
        } else {
            owned(out)
        }
    }
}

/// Routing validator for `semantic`: the value unchanged when valid, `Null` otherwise.
pub fn validate(semantic: Semantic, v: Value<'_>) -> Value<'_> {
    let s = as_str(&v);
    let ok = match semantic {
        Semantic::Email => s.is_some_and(valid_email),
        Semantic::Phone => is_phone(&v),
        Semantic::Domain => s.is_some_and(is_domain_ci),
        Semantic::LinkedinUrl => s.and_then(normalize_linkedin).is_some(),
        Semantic::Url => s.is_some_and(valid_url),
        Semantic::IsoDate => s.is_some_and(|s| s == js_trim(s) && date_from_string(s).is_some()),
        Semantic::Money => valid_money(&v),
    };
    if ok { v } else { Value::Null }
}

pub(crate) fn owned<'a>(s: String) -> Value<'a> {
    Value::Str(Cow::Owned(s))
}

fn as_str<'v>(v: &'v Value) -> Option<&'v str> {
    match v {
        Value::Str(s) => Some(s),
        _ => None,
    }
}

/// A substring of `s`, borrowed when `s` is borrowed.
fn sub<'a>(s: Cow<'a, str>, f: impl Fn(&str) -> &str) -> Cow<'a, str> {
    match s {
        Cow::Borrowed(b) => Cow::Borrowed(f(b)),
        Cow::Owned(o) => {
            let part = f(&o);
            if part.len() == o.len() {
                Cow::Owned(o)
            } else {
                Cow::Owned(part.to_string())
            }
        }
    }
}

/// JavaScript's whitespace: Unicode White_Space without U+0085, plus U+FEFF.
fn is_js_space(c: char) -> bool {
    (c.is_whitespace() && c != '\u{85}') || c == '\u{feff}'
}

pub(crate) fn js_trim(s: &str) -> &str {
    // ASCII edges by byte, and the char walk only when an edge is non-ASCII (it can be Unicode space).
    let ascii_space = |c: &u8| matches!(c, b'\t'..=b'\r' | b' ');
    let b = s.as_bytes();
    let start = b.iter().position(|c| !ascii_space(c)).unwrap_or(b.len());
    let end = b
        .iter()
        .rposition(|c| !ascii_space(c))
        .map_or(start, |i| i + 1);
    if b[start..end].first().is_some_and(|c| !c.is_ascii())
        || b[start..end].last().is_some_and(|c| !c.is_ascii())
    {
        return s.trim_matches(is_js_space);
    }
    &s[start..end]
}

fn lower(s: Cow<'_, str>) -> Cow<'_, str> {
    // One pass for the common case: ASCII with nothing to lower.
    if s.bytes().all(|b| b.is_ascii() && !b.is_ascii_uppercase()) {
        s
    } else if s.is_ascii() {
        Cow::Owned(s.to_ascii_lowercase())
    } else {
        Cow::Owned(s.to_lowercase())
    }
}

/// The number a `Value` holds, as JavaScript sees it.
pub(crate) fn as_f64(v: &Value) -> Option<f64> {
    match v {
        Value::Int(i) => Some(*i as f64),
        Value::BigInt(d) => d.parse().ok(),
        Value::Float(f) => Some(*f),
        _ => None,
    }
}

/// An integral double that fits i64 becomes `Int`, anything else stays `Float`.
fn number<'a>(f: f64) -> Value<'a> {
    const LIMIT: f64 = 9_223_372_036_854_775_808.0; // 2^63
    if f.fract() == 0.0 && (-LIMIT..LIMIT).contains(&f) {
        Value::Int(f as i64)
    } else {
        Value::Float(f)
    }
}

/// JavaScript `Number(s)` for decimal strings. `""`, blanks and 0x/0b/0o literals are rejected.
fn js_number(s: &str) -> Option<f64> {
    js_number_unbounded(s).filter(|n| n.is_finite())
}

/// `js_number` without the finiteness check: `"1e400"` is `Some(inf)`, as `Number()` gives Infinity.
fn js_number_unbounded(s: &str) -> Option<f64> {
    let trimmed = js_trim(s);
    let t = trimmed.as_bytes();
    let body = match t.first() {
        Some(b'+' | b'-') => &t[1..],
        _ => t,
    };
    let digits = |from: usize| {
        from + body[from..]
            .iter()
            .take_while(|c| c.is_ascii_digit())
            .count()
    };
    let int_end = digits(0);
    let mut end = int_end;
    let mut frac_digits = 0;
    if body.get(end) == Some(&b'.') {
        let frac_end = digits(end + 1);
        frac_digits = frac_end - end - 1;
        end = frac_end;
    }
    if int_end == 0 && frac_digits == 0 {
        return None; // also rejects "Infinity": it is never finite
    }
    if matches!(body.get(end), Some(b'e' | b'E')) {
        let sign = usize::from(matches!(body.get(end + 1), Some(b'+' | b'-')));
        let exp_end = digits(end + 1 + sign);
        if exp_end == end + 1 + sign {
            return None;
        }
        end = exp_end;
    }
    if end != body.len() {
        return None; // trailing text, or a 0x/0b/0o literal
    }
    // Up to 15 digits and nothing else is exact in an f64: no float parse needed.
    if end == int_end && int_end <= 15 {
        let n = body.iter().fold(0u64, |n, c| n * 10 + u64::from(c - b'0')) as f64;
        return Some(if t[0] == b'-' { -n } else { n });
    }
    trimmed.parse().ok()
}

fn to_number(v: &Value) -> Option<f64> {
    match v {
        Value::Str(s) => js_number(s),
        v => as_f64(v).filter(|f| f.is_finite()),
    }
}

fn to_integer(v: &Value) -> Option<f64> {
    to_number(v).filter(|f| f.fract() == 0.0)
}

fn to_boolean(v: &Value) -> Option<bool> {
    match v {
        Value::Bool(b) => Some(*b),
        Value::Str(s) => {
            let s = js_trim(s);
            // By length first: one case-insensitive compare at most.
            let is = |w: &str| s.eq_ignore_ascii_case(w);
            match s.len() {
                4 if is("true") => Some(true),
                3 if is("yes") => Some(true),
                1 if s == "1" => Some(true),
                5 if is("false") => Some(false),
                2 if is("no") => Some(false),
                1 if s == "0" => Some(false),
                _ => None,
            }
        }
        v => match as_f64(v) {
            Some(1.0) => Some(true),
            Some(0.0) => Some(false),
            _ => None,
        },
    }
}

pub(crate) fn to_text(v: Value<'_>) -> Option<Value<'_>> {
    Some(match v {
        Value::Str(s) => Value::Str(s),
        Value::Bool(b) => owned(b.to_string()),
        Value::Int(i) => owned(i.to_string()),
        Value::BigInt(d) => Value::Str(d),
        Value::Float(f) if f.is_finite() => owned(js_number_to_string(f)),
        _ => return None,
    })
}

fn split_name(s: &str, part: NamePart) -> Option<&str> {
    let s = js_trim(s);
    if s.is_empty() {
        return None;
    }
    match part {
        NamePart::First => Some(s.split_once(' ').map_or(s, |(first, _)| first)),
        NamePart::Last => s.rsplit_once(' ').map(|(_, last)| last),
    }
}

/// A phone as text. A number must be whole and positive. Otherwise its digits pass as a phone.
fn phone_text<'v>(v: &'v Value) -> Option<Cow<'v, str>> {
    match v {
        Value::Str(s) => Some(Cow::Borrowed(js_trim(s))),
        Value::Int(i) if *i > 0 => Some(Cow::Owned(i.to_string())),
        Value::BigInt(d) if !d.starts_with('-') => Some(Cow::Borrowed(d)),
        Value::Float(f) if f.fract() == 0.0 && *f > 0.0 => {
            Some(Cow::Owned(js_number_to_string(*f)))
        }
        _ => None,
    }
}

/// Digits plus a leading +, 7 to 15 digits, separators ` -().` dropped. Country-aware parsing is
/// an injected transform.
fn normalize_phone(v: &Value) -> Option<String> {
    let s = phone_text(v)?;
    let (plus, rest) = match s.strip_prefix('+') {
        Some(rest) => ("+", rest),
        None => ("", s.as_ref()),
    };
    let mut out = String::with_capacity(s.len());
    out.push_str(plus);
    // A byte at or above 0x80 is neither a digit nor a separator, so bytes reject what chars did.
    for c in rest.bytes() {
        if c.is_ascii_digit() {
            out.push(c as char);
        } else if !PHONE_SEPARATORS.contains(&c) {
            return None;
        }
    }
    (7..=15).contains(&(out.len() - plus.len())).then_some(out)
}

const PHONE_SEPARATORS: &[u8] = b" -().";

/// `normalize_phone(v).is_some()` without building the string.
fn is_phone(v: &Value) -> bool {
    let Some(s) = phone_text(v) else {
        return false;
    };
    let rest = s.strip_prefix('+').unwrap_or(&s);
    let mut digits = 0;
    for c in rest.bytes() {
        if c.is_ascii_digit() {
            digits += 1;
        } else if !PHONE_SEPARATORS.contains(&c) {
            return false;
        }
    }
    (7..=15).contains(&digits)
}

/// Drops a scheme whose `://` starts within the first 10 UTF-16 units.
fn strip_scheme(s: &str) -> &str {
    match s.as_bytes().windows(3).position(|w| w == b"://") {
        Some(i) if i > 0 && s[..i].encode_utf16().nth(9).is_none() => &s[i + 3..],
        _ => s,
    }
}

/// Dot-separated labels of 1 to 63 bytes of `[a-z0-9-]`, none starting or ending with `-`, at
/// least two of them, 253 bytes at most. One pass over the bytes.
fn is_domain(s: &str) -> bool {
    domain_bytes(s.bytes())
}

/// `is_domain(&lower(s))` without the lowercased copy. Only ASCII takes the fast path: a non-ASCII
/// letter can lowercase to ASCII (the Kelvin sign to `k`).
fn is_domain_ci(s: &str) -> bool {
    if s.is_ascii() {
        domain_bytes(s.bytes().map(|c| c.to_ascii_lowercase()))
    } else {
        is_domain(&s.to_lowercase())
    }
}

fn domain_bytes(b: impl ExactSizeIterator<Item = u8>) -> bool {
    if b.len() > 253 {
        return false;
    }
    let (mut len, mut prev, mut dots) = (0usize, b'.', 0usize);
    for c in b {
        if c == b'.' {
            if len == 0 || prev == b'-' {
                return false;
            }
            (len, dots) = (0, dots + 1);
        } else {
            let ok = c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-';
            if !ok || (len == 0 && c == b'-') || len == 63 {
                return false;
            }
            len += 1;
        }
        prev = c;
    }
    dots > 0 && len > 0 && prev != b'-'
}

/// `s` up to the first of the ASCII bytes `stops`.
fn cut<'s>(s: &'s str, stops: &[u8]) -> &'s str {
    s.bytes()
        .position(|c| stops.contains(&c))
        .map_or(s, |i| &s[..i])
}

fn host_of(s: &str) -> &str {
    cut(s, b"/?#:")
}

/// The domain of `v`, borrowed from it when trimming and lowering changed nothing.
fn to_domain(v: Cow<'_, str>) -> Option<Cow<'_, str>> {
    match lower(sub(v, js_trim)) {
        Cow::Borrowed(s) => domain_of(s).map(Cow::Borrowed),
        Cow::Owned(s) => domain_of(&s).map(|h| Cow::Owned(h.to_string())),
    }
}

/// The domain inside trimmed, lowercased `s`.
fn domain_of(mut s: &str) -> Option<&str> {
    if s.contains('@') && !s.contains('/') {
        s = s.rsplit_once('@').map_or(s, |(_, after)| after);
    }
    let mut host = host_of(strip_scheme(s));
    host = host.strip_prefix("www.").unwrap_or(host);
    host = host.strip_suffix('.').unwrap_or(host);
    is_domain(host).then_some(host)
}

fn normalize_linkedin(v: &str) -> Option<String> {
    let lowered = lower(Cow::Borrowed(js_trim(v)));
    let s = strip_scheme(&lowered);
    let host = host_of(s);
    if host != "linkedin.com" && !host.ends_with(".linkedin.com") {
        return None;
    }
    let rest = s[host.len()..].strip_prefix('/')?;
    let mut parts = cut(rest, b"?#").split('/');
    let kind = parts.next().unwrap_or("");
    let slug = parts.next().unwrap_or("");
    if !["in", "company", "school"].contains(&kind) || slug.is_empty() {
        return None;
    }
    let mut out = String::with_capacity(25 + kind.len() + 1 + slug.len());
    out.push_str("https://www.linkedin.com/");
    out.push_str(kind);
    out.push('/');
    out.push_str(slug);
    Some(out)
}

fn valid_email(v: &str) -> bool {
    match (v.find('@'), v.rfind('@')) {
        (Some(at), Some(last)) if at >= 1 && at == last && !v.contains(' ') => {
            is_domain_ci(&v[at + 1..])
        }
        _ => false,
    }
}

fn valid_url(v: &str) -> bool {
    let s = lower(Cow::Borrowed(v));
    (s.starts_with("http://") || s.starts_with("https://")) && is_domain(host_of(strip_scheme(&s)))
}

fn is_code(s: &[u8], at: usize) -> bool {
    s.get(at..at + 3)
        .is_some_and(|c| c.iter().all(u8::is_ascii_uppercase))
}

/// The amount left once a currency sign or code and a K/M/B suffix are taken off.
fn bare_amount(v: &str) -> &str {
    let mut s = js_trim(v);
    if let Some(rest) = s.strip_prefix(['$', '€', '£', '¥', '₹']) {
        s = rest;
    } else if is_code(s.as_bytes(), 0) {
        s = s[3..].trim_start_matches(is_js_space);
    }
    let b = s.as_bytes();
    if b.len() > 4 && b[b.len() - 4] == b' ' && is_code(b, b.len() - 3) {
        s = &s[..s.len() - 4];
    }
    s.strip_suffix(['K', 'M', 'B', 'k', 'm', 'b']).unwrap_or(s)
}

/// Positive only: 0 and negatives are how providers say "unknown". A number must be finite, so a
/// BigInt past f64 range is rejected as JSON.parse's Infinity is in TypeScript.
fn valid_money(v: &Value) -> bool {
    match v {
        Value::Str(s) => {
            let s = bare_amount(s);
            let b = s.as_bytes();
            b.first().is_some_and(u8::is_ascii_digit)
                && b.iter()
                    .all(|c| c.is_ascii_digit() || *c == b',' || *c == b'.')
                // Number() of a long digit string is Infinity, and TypeScript's `Infinity > 0` passes it.
                && js_number_unbounded(&s.replace(',', "")).is_some_and(|n| n > 0.0)
        }
        v => as_f64(v).is_some_and(|f| f.is_finite() && f > 0.0),
    }
}

/// A number, or text with commas and a leading currency sign or 3-letter code. With `units`, a
/// K, M or B suffix in any case multiplies by 10^3, 10^6 or 10^9.
fn parse_number(v: &Value, units: bool) -> Option<f64> {
    match v {
        Value::Str(s) => number_text(s, units),
        v => to_number(v),
    }
}

fn number_text(s: &str, units: bool) -> Option<f64> {
    let mut s = js_trim(s);
    if let Some(rest) = s.strip_prefix(['$', '€', '£', '¥', '₹']) {
        s = rest;
    } else if is_code(s.as_bytes(), 0) {
        s = s[3..].trim_start_matches(is_js_space);
    }
    let mut text = s.replace(',', "");
    let exponent = match text.as_bytes().last() {
        Some(b'k' | b'K') => "e3",
        Some(b'm' | b'M') => "e6",
        Some(b'b' | b'B') => "e9",
        _ => "",
    };
    if units && !exponent.is_empty() {
        // Exact through the decimal parse: 3.2e6, where 3.2 * 1e6 can round off a whole number.
        text.pop();
        text.push_str(exponent);
    }
    js_number(&text)
}

/// One end of `"51-200"`, `"51 - 200"`, `"10,001+"` (no max) or `"5"` (both ends). Both ends
/// must read as numbers.
fn parse_range(v: &Value, end: RangeEnd) -> Option<f64> {
    let Value::Str(s) = v else {
        return to_number(v);
    };
    let s = js_trim(s);
    // The dash after the first character, so a leading minus stays a sign.
    let dash = s.get(1..).and_then(|r| r.find('-')).map(|i| i + 1);
    let (lo, hi) = match (s.strip_suffix('+'), dash) {
        (Some(lo), _) => (lo, None),
        (None, Some(i)) => (&s[..i], Some(&s[i + 1..])),
        (None, None) => (s, Some(s)),
    };
    let lo = number_text(lo, false)?;
    let hi = match hi {
        Some(h) => Some(number_text(h, false)?),
        None => None,
    };
    match end {
        RangeEnd::Min => Some(lo),
        RangeEnd::Max => hi,
    }
}

#[cfg(test)]
mod fast_paths {
    //! The allocation-free checks agree with the compositions they replace.
    use super::*;
    use proptest::prelude::*;

    const EDGE: &[&str] = &[
        "",
        ".",
        "a.b",
        "Example.COM",
        "\u{212A}.com",
        "İ.com",
        "x-.com",
        "-x.com",
        "a..b",
        "a.b.",
        "ÉCOLE.fr",
        "a\u{0301}.com",
        "WWW.Example.Com",
        "1.2",
        "x.c-m",
    ];

    fn strings() -> impl Strategy<Value = String> {
        prop_oneof![
            "[a-zA-Z0-9.\\-]{0,20}",
            "[a-zA-Z0-9.\\-@ +()\u{212A}\u{130}é]{0,20}",
            any::<String>(),
        ]
    }

    fn check(s: &str) {
        assert_eq!(lower(Cow::Borrowed(s)), s.to_lowercase(), "lower {s:?}");
        assert_eq!(
            is_domain_ci(s),
            is_domain(&s.to_lowercase()),
            "is_domain_ci {s:?}"
        );
        let v = Value::Str(Cow::Borrowed(s));
        assert_eq!(
            is_phone(&v),
            normalize_phone(&v).is_some(),
            "is_phone {s:?}"
        );
    }

    #[test]
    fn edge_cases() {
        for s in EDGE {
            check(s);
        }
        for v in [
            Value::Int(1234567),
            Value::Int(-5),
            Value::Float(12345678.0),
            Value::Null,
        ] {
            assert_eq!(
                is_phone(&v),
                normalize_phone(&v).is_some(),
                "is_phone {v:?}"
            );
        }
    }

    proptest! {
        #[test]
        fn agree(s in strings()) {
            check(&s);
        }
    }
}

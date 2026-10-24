//! Spec types, the path grammar, and the hand-written validator for spec.schema.json.
//! For a spec without `language`, the validator must accept and reject exactly what the JSON Schema
//! does (tests/validate.rs). spec.schema.json stays language 1.

use crate::canonical::Empty;
use crate::cond::{self, Cond};
use crate::group::{AGG_OPS, AggOp};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Step {
    Key(String),
    Idx(usize),
}
pub type Path = Vec<Step>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Draft,
    Shadow,
    Promoted,
    Deprecated,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TransformRef {
    pub name: String,
    pub arg: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Source {
    From(Path),
    FromMany(Vec<Path>),
    Coalesce(Vec<Path>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScalarRule {
    pub source: Source,
    pub default: Option<Value>,
    pub transform: Vec<TransformRef>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ArrayRule {
    pub from: Path,
    pub item: Option<FieldMap>,
    pub pluck: Option<Path>,
    pub transform: Vec<TransformRef>,
    /// Language 2: the output for a missing or null source, `Null` or `List`. None: the schema's
    /// `x-empty` decides.
    pub missing: Option<Empty>,
    /// Language 2: the output for an empty source array, `Null` or `List`.
    pub empty: Option<Empty>,
    /// Language 2. `from` is the path before the first `[]`, `flatten` the paths between
    /// the next ones. `unique` is the empty path for `true`, `sort` holds `desc`.
    pub wrap: bool,
    pub flatten: Vec<Path>,
    pub filter: Option<Cond>,
    pub unique: Option<Path>,
    pub sort: Option<(Path, bool)>,
    pub limit: Option<usize>,
    /// Language 2: `group_by` and whether groups are consecutive.
    pub group: Option<(Path, bool)>,
}

/// A path that is `$group`, which language 2 reserves for the group of a grouped item map.
pub fn is_group(p: &Path) -> bool {
    matches!(p.as_slice(), [Step::Key(k)] if k == "$group")
}

/// Language 2: `{"aggregate": <op>, "from": <path>}`. `from` is split as an array `from` is:
/// the source, then the `flatten` paths, then `pluck`, the path after the last `[]` (empty for
/// the elements themselves). A source of `$group` is the group of a grouped item map.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Aggregate {
    pub op: AggOp,
    pub from: Path,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub flatten: Vec<Path>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pluck: Path,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub propagate: bool,
    /// `join` only. The default is a space, as for the `join` transform.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sep: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Rule {
    Scalar(ScalarRule),
    Array(ArrayRule),
    /// Language 2: `{"const": <any JSON value>}`, written as it is.
    Const(Value),
    /// Language 2, on an object node: `{"present": <path> | [<path>, ...]}` and/or
    /// `{"if_empty": "null"}`. The object's leaves keep their own keys.
    Object {
        present: Vec<Path>,
        if_empty_null: bool,
    },
    Aggregate(Aggregate),
    /// Language 2: `rule` applies only when `when` holds. Otherwise the field is null, or
    /// left out of its object with `omit`.
    When {
        when: Cond,
        omit: bool,
        rule: Box<Rule>,
    },
}

/// Canonical field path to its rule, in spec order. A key ending in `[]` holds a `Rule::Array`.
pub type FieldMap = Vec<(String, Rule)>;

#[derive(Debug, Clone, PartialEq)]
pub struct Spec {
    /// The spec language: 1 (the TypeScript grammar) or 2. A spec without `language` is 1.
    pub language: u8,
    pub provider: String,
    pub endpoint: String,
    pub version: u32,
    pub status: Status,
    pub fields: FieldMap,
    pub anchors: BTreeMap<String, String>,
    /// Language 2: each condition, with its spec JSON text, which names it in `Rejected`.
    pub reject: Vec<(Cond, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Issue {
    pub at: String,
    pub message: String,
}

impl fmt::Display for Issue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.at, self.message)
    }
}

/// The keys of an array rule. The ones from `missing` on are language 2.
const ARRAY_KEYS: [&str; 14] = [
    "from",
    "item",
    "pluck",
    "transform",
    "missing",
    "empty",
    "wrap",
    "filter",
    "unique",
    "sort_by",
    "order",
    "limit",
    "group_by",
    "consecutive",
];

/// The keys that any rule object can take.
const WHEN: [&str; 2] = ["when", "omit"];

const STATUSES: [&str; 4] = ["draft", "shadow", "promoted", "deprecated"];

fn ident_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_' || c == b'$'
}
fn ident_rest(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c == b'$' || c == b'-'
}

/// Parses `["..."]` contents starting after the opening quote. Returns the key and the index past `]`.
/// A lone surrogate escape becomes U+FFFD, since a Rust `String` cannot hold it.
fn quoted(p: &str, start: usize) -> Option<(String, usize)> {
    let mut units: Vec<u16> = Vec::new();
    let mut chars = p[start..].char_indices();
    while let Some((i, c)) = chars.next() {
        match c {
            '"' => {
                let end = start + i + 1;
                return (p.as_bytes().get(end) == Some(&b']'))
                    .then(|| (String::from_utf16_lossy(&units), end + 1));
            }
            '\\' => {
                let e = match chars.next()?.1 {
                    '"' => '"',
                    '\\' => '\\',
                    '/' => '/',
                    'b' => '\u{8}',
                    'f' => '\u{c}',
                    'n' => '\n',
                    'r' => '\r',
                    't' => '\t',
                    'u' => {
                        let hex: String =
                            (0..4).map_while(|_| chars.next().map(|(_, h)| h)).collect();
                        if hex.len() != 4 || !hex.bytes().all(|h| h.is_ascii_hexdigit()) {
                            return None;
                        }
                        units.push(u16::from_str_radix(&hex, 16).ok()?);
                        continue;
                    }
                    _ => return None,
                };
                units.push(e as u16);
            }
            c if (c as u32) < 0x20 => return None,
            c => units.extend(c.encode_utf16(&mut [0; 2]).iter()),
        }
    }
    None
}

/// Parses a source path: identifiers `[A-Za-z_$][A-Za-z0-9_$-]*` joined by dots, `[n]` indices and
/// `["quoted keys"]` in JSON string syntax. A leading bare `$` is the response itself (`$` alone is the
/// empty path). A key that is literally named `$` is written `["$"]`. Returns None for an invalid path.
pub fn parse_path(p: &str) -> Option<Path> {
    let mut segments = steps(p, false)?;
    segments.pop()
}

/// A language 2 array `from`: a path that can hold `[]` (flatten). The segments around each `[]`,
/// so `jobs[].skills[]` gives `[jobs, skills, $]`. A path without `[]` is one segment.
pub fn parse_from(p: &str) -> Option<Vec<Path>> {
    steps(p, true)
}

fn steps(p: &str, flat: bool) -> Option<Vec<Path>> {
    let b = p.as_bytes();
    if b.is_empty() {
        return None;
    }
    let mut segments = Vec::new();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let first = i == 0;
        if flat && b[i..].starts_with(b"[]") {
            segments.push(std::mem::take(&mut out));
            i += 2;
        } else if b[i] == b'[' {
            if b.get(i + 1) == Some(&b'"') {
                let (key, end) = quoted(p, i + 2)?;
                out.push(Step::Key(key));
                i = end;
            } else {
                let digits = b[i + 1..].iter().take_while(|c| c.is_ascii_digit()).count();
                let j = i + 1 + digits;
                if digits == 0 || (digits > 1 && b[i + 1] == b'0') || b.get(j) != Some(&b']') {
                    return None;
                }
                // an index past usize::MAX saturates. It reads nothing either way.
                out.push(Step::Idx(p[i + 1..j].parse().unwrap_or(usize::MAX)));
                i = j + 1;
            }
        } else {
            if !first {
                if b[i] != b'.' {
                    return None;
                }
                i += 1;
            }
            if !b.get(i).is_some_and(|&c| ident_start(c)) {
                return None;
            }
            let j = i + 1 + b[i + 1..].iter().take_while(|&&c| ident_rest(c)).count();
            if !(first && &p[i..j] == "$") {
                out.push(Step::Key(p[i..j].to_string()));
            }
            i = j;
        }
    }
    segments.push(out);
    Some(segments)
}

fn is_name(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty()
        && (b[0].is_ascii_alphabetic() || b[0] == b'_')
        && b.iter().all(|c| c.is_ascii_alphanumeric() || *c == b'_')
}

fn is_canon(s: &str) -> bool {
    s.split('.').all(is_name)
}

/// A field key as canonical names and whether it ends in `[]`. Language 1 keys are dotted
/// identifiers. Language 2 keys use the path grammar with keys only, so `["first-name"]` and
/// `meta["@type"]` name any field. None for a key that is not valid in either. The resolver also
/// reads language 1 keys with it, which gives the same names as splitting on dots.
pub fn field_key(key: &str) -> Option<(Vec<String>, bool)> {
    let (path, array) = match key.strip_suffix("[]") {
        Some(p) => (p, true),
        None => (key, false),
    };
    // A leading bare `$` is the response in a source path. It names no canonical field.
    if matches!(path.as_bytes(), [b'$'] | [b'$', b'.' | b'[', ..]) {
        return None;
    }
    let names = parse_path(path)?
        .into_iter()
        .map(|s| match s {
            Step::Key(k) => Some(k),
            Step::Idx(_) => None,
        })
        .collect::<Option<Vec<_>>>()?;
    Some((names, array))
}

fn json_string(s: &str) -> String {
    Value::String(s.to_string()).to_string()
}

#[derive(Default)]
struct Checker {
    issues: Vec<Issue>,
    /// Set by `header`, before any field is checked.
    language: u8,
    /// Inside the item map of a `group_by` array, where `$group` names the group.
    grouped: bool,
}

impl Checker {
    fn fail(&mut self, at: &str, message: &str) {
        self.issues.push(Issue {
            at: at.to_string(),
            message: message.to_string(),
        });
    }
    /// Records that `rule`, a language 2 key, sits in a language 1 spec.
    fn lang2(&mut self, at: &str, rule: &str) {
        if self.language == 1 {
            self.fail(at, &format!("{} needs \"language\": 2", json_string(rule)));
        }
    }
    fn keys(&mut self, o: &Map<String, Value>, at: &str, allowed: &[&str]) {
        for k in o.keys().filter(|k| !allowed.contains(&k.as_str())) {
            self.fail(&format!("{at}.{k}"), "unknown property");
        }
    }
    fn path(&mut self, v: Option<&Value>, at: &str) {
        if v.and_then(Value::as_str).and_then(parse_path).is_none() {
            self.fail(at, "must be a path like a.b[0][\"c d\"]");
        }
    }
    /// `path` for a path that reads the body or the context (`$input`), never `$group`.
    fn source(&mut self, v: Option<&Value>, at: &str) {
        self.path(v, at);
        if let Some(p) = v.and_then(Value::as_str).and_then(parse_path) {
            self.root(&p, at, true);
        }
    }
    /// Language 2: a path that starts with `$group` or `$input` where that root cannot be read.
    /// `$group` is only a list or aggregate `from` (`group_source` checks those). `$input` is only
    /// a scalar `from` or a condition path, where `input` is true.
    fn root(&mut self, p: &Path, at: &str, input: bool) {
        match p.first() {
            _ if self.language != 2 => {}
            Some(Step::Key(k)) if k == "$group" => {
                self.fail(at, "$group is only a list or aggregate from")
            }
            Some(Step::Key(k)) if k == "$input" && !input => {
                self.fail(at, "$input is only a scalar from or a condition path")
            }
            _ => {}
        }
    }
    fn paths(&mut self, v: &Value, at: &str) {
        match v.as_array() {
            Some(a) if a.len() >= 2 => {
                for (i, p) in a.iter().enumerate() {
                    self.source(Some(p), &format!("{at}[{i}]"));
                }
            }
            _ => self.fail(at, "must be an array of at least 2 paths"),
        }
    }
    fn step(&mut self, v: &Value, at: &str) {
        match v {
            Value::String(s) if !is_name(s) => self.fail(at, "invalid transform name"),
            Value::String(_) => {}
            Value::Object(o) if o.len() == 1 => {
                if !o.keys().all(|k| is_name(k)) {
                    self.fail(at, "invalid transform name");
                }
            }
            _ => self.fail(at, "must be a transform name or { name: argument }"),
        }
    }
    fn transform(&mut self, v: &Value, at: &str) {
        let Some(a) = v.as_array() else {
            return self.step(v, at);
        };
        if a.is_empty() {
            self.fail(at, "must not be empty");
        }
        for (i, s) in a.iter().enumerate() {
            self.step(s, &format!("{at}[{i}]"));
        }
    }
    fn scalar(&mut self, r: &Value, at: &str) {
        if let Value::String(_) = r {
            return self.source(Some(r), at);
        }
        let Some(r) = r.as_object() else {
            return self.fail(at, "must be a path or a rule object");
        };
        self.keys(
            r,
            at,
            &["from", "coalesce", "default", "transform", "when", "omit"],
        );
        self.when(r, at);
        if r.contains_key("from") == r.contains_key("coalesce") {
            self.fail(at, "needs exactly one of from, coalesce");
        }
        if let Some(from) = r.get("from") {
            if from.is_array() {
                self.paths(from, &format!("{at}.from"));
            } else {
                self.source(Some(from), &format!("{at}.from"));
            }
        }
        if let Some(c) = r.get("coalesce") {
            self.paths(c, &format!("{at}.coalesce"));
        }
        if r.get("default")
            .is_some_and(|d| !(d.is_string() || d.is_number() || d.is_boolean()))
        {
            self.fail(
                &format!("{at}.default"),
                "must be a string, number or boolean",
            );
        }
        if let Some(t) = r.get("transform") {
            self.transform(t, &format!("{at}.transform"));
        }
    }
    fn array(&mut self, r: &Value, at: &str) {
        let Some(r) = r.as_object() else {
            return self.fail(at, "array fields need a rule object with from");
        };
        self.keys(r, at, &[&ARRAY_KEYS[..], &WHEN].concat());
        self.when(r, at);
        for k in &ARRAY_KEYS[4..] {
            if r.contains_key(*k) {
                self.lang2(at, k);
            }
        }
        for k in ["missing", "empty"] {
            if r.get(k).is_some_and(|v| list_empty(v).is_none()) {
                self.fail(&format!("{at}.{k}"), "must be \"null\" or \"[]\"");
            }
        }
        self.list_ops(r, at);
        let (item, pluck) = (r.get("item"), r.get("pluck"));
        if item.is_some() && pluck.is_some() {
            self.fail(at, "item and pluck can't be combined");
        }
        if let Some(item) = item {
            let outer = std::mem::replace(&mut self.grouped, r.contains_key("group_by"));
            self.fields(item, &format!("{at}.item"));
            self.grouped = outer;
        }
        if pluck.is_some() {
            self.key_path(pluck, &format!("{at}.pluck"));
        }
        if item.is_some() && r.contains_key("transform") {
            self.fail(&format!("{at}.transform"), "not allowed with item");
        }
        if let Some(t) = r.get("transform") {
            self.transform(t, &format!("{at}.transform"));
        }
    }
    /// The language 2 options of an array rule, and its `from`.
    fn list_ops(&mut self, r: &Map<String, Value>, at: &str) {
        let from = r.get("from").and_then(Value::as_str);
        match from.and_then(parse_from) {
            Some(s) if s.len() > 1 && self.language == 2 => {
                if !s.last().unwrap().is_empty() {
                    self.fail(&format!("{at}.from"), "a path with [] must end in []");
                }
                self.group_source(&s[0], &format!("{at}.from"));
            }
            _ => self.path(r.get("from"), &format!("{at}.from")),
        }
        if let Some(p) = from.and_then(parse_path) {
            self.group_source(&p, &format!("{at}.from"));
        }
        if r.get("wrap").is_some_and(|v| !v.is_boolean()) {
            self.fail(&format!("{at}.wrap"), "must be true or false");
        }
        if let Some(c) = r.get("filter") {
            self.conds(c, &format!("{at}.filter"));
        }
        match r.get("unique") {
            None | Some(Value::Bool(true)) => {}
            Some(Value::String(p)) if parse_path(p).is_some() => {
                self.root(&parse_path(p).unwrap(), &format!("{at}.unique"), false)
            }
            Some(_) => self.fail(&format!("{at}.unique"), "must be true or a path"),
        }
        if r.contains_key("sort_by") {
            self.key_path(r.get("sort_by"), &format!("{at}.sort_by"));
        }
        match r.get("order") {
            Some(o) if o != "asc" && o != "desc" => {
                self.fail(&format!("{at}.order"), "must be \"asc\" or \"desc\"")
            }
            Some(_) if !r.contains_key("sort_by") => {
                self.fail(&format!("{at}.order"), "needs sort_by")
            }
            _ => {}
        }
        if r.get("limit")
            .is_some_and(|l| l.as_u64().is_none_or(|n| n == 0))
        {
            self.fail(at, "limit must be a positive integer");
        }
        if r.contains_key("group_by") {
            self.key_path(r.get("group_by"), &format!("{at}.group_by"));
        }
        match r.get("consecutive") {
            Some(c) if !c.is_boolean() => {
                self.fail(&format!("{at}.consecutive"), "must be true or false")
            }
            Some(_) if !r.contains_key("group_by") => {
                self.fail(&format!("{at}.consecutive"), "needs group_by")
            }
            _ => {}
        }
    }
    /// `path` for a path that reads list elements, which neither `$group` nor `$input` names.
    fn key_path(&mut self, v: Option<&Value>, at: &str) {
        self.path(v, at);
        if let Some(p) = v.and_then(Value::as_str).and_then(parse_path) {
            self.root(&p, at, false);
        }
    }
    /// A list or aggregate source path (language 2): `$input` is not one. One that starts with
    /// `$group` is only inside a grouped item map, and only as the whole path before `[]`.
    fn group_source(&mut self, first: &Path, at: &str) {
        if self.language == 2 && first.first() == Some(&Step::Key("$input".into())) {
            return self.fail(at, "$input is only a scalar from or a condition path");
        }
        if self.language != 2 || first.first() != Some(&Step::Key("$group".into())) {
            return;
        }
        if !self.grouped {
            self.fail(at, "$group needs a group_by array");
        } else if first.len() > 1 {
            self.fail(at, "use $group or $group[].<path>");
        }
    }
    fn aggregate(&mut self, o: &Map<String, Value>, at: &str) {
        self.lang2(at, "aggregate");
        self.keys(
            o,
            at,
            &["aggregate", "from", "nulls", "sep", "when", "omit"],
        );
        self.when(o, at);
        let op = o.get("aggregate").and_then(Value::as_str).and_then(agg_op);
        if op.is_none() {
            let names: Vec<&str> = AGG_OPS.iter().map(|(n, _)| *n).collect();
            let m = format!("must be one of {}", names.join(", "));
            self.fail(&format!("{at}.aggregate"), &m);
        }
        match o.get("from").and_then(Value::as_str).and_then(parse_from) {
            Some(s) => self.group_source(&s[0], &format!("{at}.from")),
            None => self.path(None, &format!("{at}.from")),
        }
        if o.get("nulls")
            .is_some_and(|n| n != "skip" && n != "propagate")
        {
            self.fail(&format!("{at}.nulls"), "must be \"skip\" or \"propagate\"");
        }
        match o.get("sep") {
            Some(s) if !s.is_string() => self.fail(&format!("{at}.sep"), "must be a string"),
            Some(_) if op.is_some_and(|op| op != AggOp::Join) => {
                self.fail(&format!("{at}.sep"), "needs join")
            }
            _ => {}
        }
    }
    /// One condition, or a list of them that must all hold.
    fn conds(&mut self, c: &Value, at: &str) {
        match c {
            Value::Array(a) => {
                for (i, c) in a.iter().enumerate() {
                    self.cond(c, &format!("{at}[{i}]"));
                }
            }
            c => self.cond(c, at),
        }
    }
    fn cond(&mut self, c: &Value, at: &str) {
        self.parsed(cond::parse(c), at)
    }
    fn parsed(&mut self, c: Result<Cond, Vec<(String, String)>>, at: &str) {
        match c {
            Ok(Cond::Present(p, _) | Cond::Eq(p, _) | Cond::In(p, _)) => {
                self.root(&p, &format!("{at}.path"), true)
            }
            Ok(Cond::Missing(p)) => self.root(&p, &format!("{at}.missing"), true),
            Ok(Cond::Empty(p)) => self.root(&p, &format!("{at}.empty"), true),
            Ok(Cond::All(_)) => {}
            Err(issues) => {
                for (sub, m) in issues {
                    self.fail(&format!("{at}{sub}"), &m);
                }
            }
        }
    }
    /// Language 2: top-level `reject`, a list of conditions.
    fn reject(&mut self, r: &Value) {
        self.lang2("spec", "reject");
        match r.as_array().filter(|a| !a.is_empty()) {
            Some(a) => {
                for (i, c) in a.iter().enumerate() {
                    self.parsed(cond::parse_reject(c), &format!("spec.reject[{i}]"));
                }
            }
            None => self.fail("spec.reject", "must be a non-empty array of conditions"),
        }
    }
    /// Language 2: `when` and `omit`, which any rule object can take.
    fn when(&mut self, o: &Map<String, Value>, at: &str) {
        if let Some(c) = o.get("when") {
            self.lang2(at, "when");
            self.conds(c, &format!("{at}.when"));
        }
        if let Some(v) = o.get("omit") {
            self.lang2(at, "omit");
            if !v.is_boolean() {
                self.fail(&format!("{at}.omit"), "must be true or false");
            } else if !o.contains_key("when") {
                self.fail(at, "omit needs when");
            }
        }
    }
    fn optional(&mut self, o: &Map<String, Value>, at: &str) {
        self.keys(o, at, &["present", "if_empty", "when", "omit"]);
        self.when(o, at);
        if let Some(p) = o.get("present") {
            self.lang2(at, "present");
            let pat = format!("{at}.present");
            match p {
                Value::Array(a) if !a.is_empty() => {
                    for (i, p) in a.iter().enumerate() {
                        self.source(Some(p), &format!("{pat}[{i}]"));
                    }
                }
                Value::Array(_) => self.fail(&pat, "must be a path or a non-empty array of paths"),
                p => self.source(Some(p), &pat),
            }
        }
        if let Some(v) = o.get("if_empty") {
            self.lang2(at, "if_empty");
            if v != "null" {
                self.fail(&format!("{at}.if_empty"), "must be \"null\"");
            }
        }
    }
    fn fields(&mut self, v: &Value, at: &str) {
        let Some(o) = v.as_object().filter(|o| !o.is_empty()) else {
            return self.fail(at, "must be a non-empty object");
        };
        for (k, r) in o {
            let kat = format!("{at}[{}]", json_string(k));
            let canon = match self.language {
                1 => is_canon(k) || k.strip_suffix("[]").is_some_and(is_canon),
                _ => field_key(k).is_some(),
            };
            if !canon {
                self.fail(
                    &kat,
                    match self.language {
                        1 => "canonical field must be dotted identifiers, optionally ending in []",
                        _ => "canonical field must be a path of keys, optionally ending in []",
                    },
                );
            } else if let Some(o) = r.as_object().filter(|o| o.contains_key("const")) {
                self.lang2(&kat, "const");
                self.keys(o, &kat, &["const", "when", "omit"]);
                self.when(o, &kat);
            } else if let Some(o) = r.as_object().filter(|o| optional(o)) {
                self.optional(o, &kat);
            } else if k.ends_with("[]") {
                self.array(r, &kat);
            } else if let Some(o) = r.as_object().filter(|o| o.contains_key("aggregate")) {
                self.aggregate(o, &kat);
            } else {
                self.scalar(r, &kat);
            }
        }
    }
    fn spec(&mut self, s: &Value) {
        let Some(s) = s.as_object() else {
            return self.fail("spec", "must be an object");
        };
        self.keys(
            s,
            "spec",
            &[
                "$schema", "language", "provider", "endpoint", "version", "status", "anchors",
                "fields", "reject",
            ],
        );
        self.header(s);
        if let Some(a) = s.get("anchors") {
            self.anchors(a);
        }
        if let Some(r) = s.get("reject") {
            self.reject(r);
        }
        self.fields(s.get("fields").unwrap_or(&Value::Null), "spec.fields");
    }
    fn header(&mut self, s: &Map<String, Value>) {
        if s.get("$schema").is_some_and(|v| !v.is_string()) {
            self.fail("spec.$schema", "must be a string");
        }
        self.language = match language(s.get("language")) {
            Some(l) => l,
            None => {
                self.fail("spec.language", "must be 1 or 2");
                // Checked as language 2, so a bad value adds no `needs "language": 2` issues.
                2
            }
        };
        for k in ["provider", "endpoint"] {
            if s.get(k).and_then(Value::as_str).is_none_or(str::is_empty) {
                self.fail(&format!("spec.{k}"), "must be a non-empty string");
            }
        }
        if version(s.get("version")).is_none_or(|v| v < 1.0) {
            self.fail("spec.version", "must be an integer >= 1");
        }
        if !s
            .get("status")
            .and_then(Value::as_str)
            .is_some_and(|v| STATUSES.contains(&v))
        {
            self.fail(
                "spec.status",
                &format!("must be one of {}", STATUSES.join(", ")),
            );
        }
    }
    fn anchors(&mut self, a: &Value) {
        let Some(a) = a.as_object() else {
            return self.fail("spec.anchors", "must be an object");
        };
        for (k, t) in a {
            if t.as_str().is_none_or(str::is_empty) {
                self.fail(
                    &format!("spec.anchors[{}]", json_string(k)),
                    "must be a non-empty string",
                );
            }
        }
    }
}

/// The version as a whole number (JS `Number.isInteger`, so `2.0` counts), or None.
fn version(v: Option<&Value>) -> Option<f64> {
    v?.as_f64().filter(|f| f.fract() == 0.0)
}

/// 1 when absent, the integer 1 or 2 when present, None for any other value.
fn language(v: Option<&Value>) -> Option<u8> {
    match v.map(Value::as_u64) {
        None => Some(1),
        Some(Some(l @ (1 | 2))) => Some(l as u8),
        Some(_) => None,
    }
}

/// A rule object for an object node: it has `present` or `if_empty`, or `when` and nothing else
/// but `omit`.
fn optional(o: &Map<String, Value>) -> bool {
    o.contains_key("present")
        || o.contains_key("if_empty")
        || (o.contains_key("when") && o.keys().all(|k| WHEN.contains(&k.as_str())))
}

fn agg_op(name: &str) -> Option<AggOp> {
    AGG_OPS.iter().find(|(n, _)| *n == name).map(|(_, o)| *o)
}

/// An array rule's `missing` or `empty` value.
fn list_empty(v: &Value) -> Option<Empty> {
    match v.as_str()? {
        "null" => Some(Empty::Null),
        "[]" => Some(Empty::List),
        _ => None,
    }
}

/// Grammar check only (what spec.schema.json expresses). Semantic checks come with compilation.
pub fn validate_spec(spec: &Value) -> Vec<Issue> {
    let mut c = Checker::default();
    c.spec(spec);
    c.issues
}

// The builders below run only on a spec that passed validate_spec, so the unwraps cannot fail.

fn path(v: &Value) -> Path {
    parse_path(v.as_str().unwrap()).unwrap()
}

fn paths(v: &Value) -> Vec<Path> {
    v.as_array().unwrap().iter().map(path).collect()
}

fn transform(v: Option<&Value>) -> Vec<TransformRef> {
    let step = |s: &Value| match s {
        Value::String(name) => TransformRef {
            name: name.clone(),
            arg: None,
        },
        _ => {
            let (name, arg) = s.as_object().unwrap().iter().next().unwrap();
            TransformRef {
                name: name.clone(),
                arg: Some(arg.clone()),
            }
        }
    };
    match v {
        None => vec![],
        Some(Value::Array(a)) => a.iter().map(step).collect(),
        Some(s) => vec![step(s)],
    }
}

fn field_map(v: &Value) -> FieldMap {
    v.as_object()
        .unwrap()
        .iter()
        .map(|(k, r)| (k.clone(), rule(k, r)))
        .collect()
}

/// A `when` value: one condition, or all of a list.
fn conds(v: &Value) -> Cond {
    match v {
        Value::Array(a) => Cond::All(a.iter().map(|c| cond::parse(c).unwrap()).collect()),
        c => cond::parse(c).unwrap(),
    }
}

fn rule(key: &str, r: &Value) -> Rule {
    let rule = bare_rule(key, r);
    match r.get("when") {
        Some(c) => Rule::When {
            when: conds(c),
            omit: r.get("omit") == Some(&Value::Bool(true)),
            rule: Box::new(rule),
        },
        None => rule,
    }
}

/// `rule` without its `when`.
fn bare_rule(key: &str, r: &Value) -> Rule {
    if let Some(c) = r.get("const") {
        return Rule::Const(c.clone());
    }
    if let Some(o) = r.as_object().filter(|o| optional(o)) {
        return Rule::Object {
            present: match o.get("present") {
                Some(Value::Array(a)) => a.iter().map(path).collect(),
                Some(p) => vec![path(p)],
                None => Vec::new(),
            },
            if_empty_null: o.contains_key("if_empty"),
        };
    }
    if key.ends_with("[]") {
        let mut from = parse_from(r["from"].as_str().unwrap()).unwrap();
        if from.len() > 1 {
            from.pop(); // the empty path after the last `[]`
        }
        let first = from.remove(0);
        return Rule::Array(ArrayRule {
            from: first,
            flatten: from,
            wrap: r.get("wrap") == Some(&Value::Bool(true)),
            filter: r.get("filter").map(conds),
            unique: r.get("unique").map(|u| match u {
                Value::String(p) => parse_path(p).unwrap(),
                _ => Vec::new(),
            }),
            sort: r
                .get("sort_by")
                .map(|p| (path(p), r.get("order").is_some_and(|o| o == "desc"))),
            limit: r.get("limit").and_then(Value::as_u64).map(|n| n as usize),
            group: r
                .get("group_by")
                .map(|p| (path(p), r.get("consecutive") != Some(&Value::Bool(false)))),
            item: r.get("item").map(field_map),
            pluck: r.get("pluck").map(path),
            transform: transform(r.get("transform")),
            missing: r.get("missing").and_then(list_empty),
            empty: r.get("empty").and_then(list_empty),
        });
    }
    if let Some(op) = r.get("aggregate") {
        let mut from = parse_from(r["from"].as_str().unwrap()).unwrap();
        let pluck = match from.len() {
            1 => Vec::new(),
            _ => from.pop().unwrap(),
        };
        return Rule::Aggregate(Aggregate {
            op: agg_op(op.as_str().unwrap()).unwrap(),
            from: from.remove(0),
            flatten: from,
            pluck,
            propagate: r.get("nulls").is_some_and(|n| n == "propagate"),
            sep: r.get("sep").and_then(Value::as_str).map(String::from),
        });
    }
    let source = match r {
        Value::String(_) => Source::From(path(r)),
        _ => match (r.get("from"), r.get("coalesce")) {
            (Some(f @ Value::Array(_)), _) => Source::FromMany(paths(f)),
            (Some(f), _) => Source::From(path(f)),
            (None, c) => Source::Coalesce(paths(c.unwrap())),
        },
    };
    Rule::Scalar(ScalarRule {
        source,
        default: r.get("default").cloned(),
        transform: transform(r.get("transform")),
    })
}

/// Validates the grammar and returns the typed spec, or every grammar issue.
pub fn parse_spec(spec: &Value) -> Result<Spec, Vec<Issue>> {
    let issues = validate_spec(spec);
    if !issues.is_empty() {
        return Err(issues);
    }
    let v = version(spec.get("version")).unwrap();
    if v > u32::MAX as f64 {
        return Err(vec![Issue {
            at: "spec.version".into(),
            message: format!("must be at most {}", u32::MAX),
        }]);
    }
    let status = match spec["status"].as_str().unwrap() {
        "draft" => Status::Draft,
        "shadow" => Status::Shadow,
        "promoted" => Status::Promoted,
        _ => Status::Deprecated,
    };
    let anchors = spec.get("anchors").and_then(Value::as_object);
    Ok(Spec {
        language: language(spec.get("language")).unwrap(),
        provider: spec["provider"].as_str().unwrap().to_string(),
        endpoint: spec["endpoint"].as_str().unwrap().to_string(),
        version: v as u32,
        status,
        fields: field_map(&spec["fields"]),
        anchors: anchors
            .into_iter()
            .flatten()
            .map(|(k, t)| (k.clone(), t.as_str().unwrap().to_string()))
            .collect(),
        reject: spec
            .get("reject")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|c| {
                let text = serde_json::to_string(c).expect("JSON serializes");
                (cond::parse_reject(c).unwrap(), text)
            })
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn issues_for(language: Option<u8>) -> Vec<String> {
        let mut s = json!({"provider": "p", "endpoint": "e", "version": 1, "status": "draft"});
        if let Some(l) = language {
            s["language"] = json!(l);
        }
        let mut c = Checker::default();
        c.header(s.as_object().unwrap());
        c.lang2("spec.fields[\"skills[]\"]", "group_by");
        c.issues.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn lang2_flags_a_language_1_spec_only() {
        let want = [r#"spec.fields["skills[]"]: "group_by" needs "language": 2"#];
        assert_eq!(issues_for(None), want);
        assert_eq!(issues_for(Some(1)), want);
        assert!(issues_for(Some(2)).is_empty());
    }
}

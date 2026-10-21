//! Spec types, the path grammar, and the hand-written validator for spec.schema.json.
//! The validator must accept and reject exactly what the JSON Schema does (tests/validate.rs).

use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    Key(String),
    Idx(usize),
}
pub type Path = Vec<Step>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

#[derive(Debug, Clone, PartialEq)]
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
}

#[derive(Debug, Clone, PartialEq)]
pub enum Rule {
    Scalar(ScalarRule),
    Array(ArrayRule),
}

/// Canonical field path to its rule, in spec order. A key ending in `[]` holds a `Rule::Array`.
pub type FieldMap = Vec<(String, Rule)>;

#[derive(Debug, Clone, PartialEq)]
pub struct Spec {
    pub provider: String,
    pub endpoint: String,
    pub version: u32,
    pub status: Status,
    pub fields: FieldMap,
    pub anchors: BTreeMap<String, String>,
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

const STATUSES: [&str; 4] = ["draft", "shadow", "promoted", "deprecated"];

fn ident_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_' || c == b'$'
}
fn ident_rest(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c == b'$' || c == b'-'
}

/// Parses `["..."]` contents starting after the opening quote. Returns the key and the index past `]`.
/// A lone surrogate escape becomes U+FFFD, since a Rust `String` can't hold it.
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
/// empty path); a key that is literally named `$` is written `["$"]`. Returns None for an invalid path.
pub fn parse_path(p: &str) -> Option<Path> {
    let b = p.as_bytes();
    if b.is_empty() {
        return None;
    }
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let first = i == 0;
        if b[i] == b'[' {
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
                // an index past usize::MAX saturates; it reads nothing either way.
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
    Some(out)
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

fn json_string(s: &str) -> String {
    Value::String(s.to_string()).to_string()
}

#[derive(Default)]
struct Checker {
    issues: Vec<Issue>,
}

impl Checker {
    fn fail(&mut self, at: &str, message: &str) {
        self.issues.push(Issue {
            at: at.to_string(),
            message: message.to_string(),
        });
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
    fn paths(&mut self, v: &Value, at: &str) {
        match v.as_array() {
            Some(a) if a.len() >= 2 => {
                for (i, p) in a.iter().enumerate() {
                    self.path(Some(p), &format!("{at}[{i}]"));
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
            return self.path(Some(r), at);
        }
        let Some(r) = r.as_object() else {
            return self.fail(at, "must be a path or a rule object");
        };
        self.keys(r, at, &["from", "coalesce", "default", "transform"]);
        if r.contains_key("from") == r.contains_key("coalesce") {
            self.fail(at, "needs exactly one of from, coalesce");
        }
        if let Some(from) = r.get("from") {
            if from.is_array() {
                self.paths(from, &format!("{at}.from"));
            } else {
                self.path(Some(from), &format!("{at}.from"));
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
        self.keys(r, at, &["from", "item", "pluck", "transform"]);
        self.path(r.get("from"), &format!("{at}.from"));
        let (item, pluck) = (r.get("item"), r.get("pluck"));
        if item.is_some() && pluck.is_some() {
            self.fail(at, "item and pluck can't be combined");
        }
        if let Some(item) = item {
            self.fields(item, &format!("{at}.item"));
        }
        if pluck.is_some() {
            self.path(pluck, &format!("{at}.pluck"));
        }
        if item.is_some() && r.contains_key("transform") {
            self.fail(&format!("{at}.transform"), "not allowed with item");
        }
        if let Some(t) = r.get("transform") {
            self.transform(t, &format!("{at}.transform"));
        }
    }
    fn fields(&mut self, v: &Value, at: &str) {
        let Some(o) = v.as_object().filter(|o| !o.is_empty()) else {
            return self.fail(at, "must be a non-empty object");
        };
        for (k, r) in o {
            let kat = format!("{at}[{}]", json_string(k));
            if is_canon(k) {
                self.scalar(r, &kat);
            } else if k.strip_suffix("[]").is_some_and(is_canon) {
                self.array(r, &kat);
            } else {
                self.fail(
                    &kat,
                    "canonical field must be dotted identifiers, optionally ending in []",
                );
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
                "$schema", "provider", "endpoint", "version", "status", "anchors", "fields",
            ],
        );
        self.header(s);
        if let Some(a) = s.get("anchors") {
            self.anchors(a);
        }
        self.fields(s.get("fields").unwrap_or(&Value::Null), "spec.fields");
    }
    fn header(&mut self, s: &Map<String, Value>) {
        if s.get("$schema").is_some_and(|v| !v.is_string()) {
            self.fail("spec.$schema", "must be a string");
        }
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

/// Grammar check only (what spec.schema.json expresses). Semantic checks come with compilation.
pub fn validate_spec(spec: &Value) -> Vec<Issue> {
    let mut c = Checker::default();
    c.spec(spec);
    c.issues
}

// The builders below run only on a spec that passed validate_spec, so the unwraps can't fail.

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

fn rule(key: &str, r: &Value) -> Rule {
    if key.ends_with("[]") {
        return Rule::Array(ArrayRule {
            from: path(&r["from"]),
            item: r.get("item").map(field_map),
            pluck: r.get("pluck").map(path),
            transform: transform(r.get("transform")),
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
    })
}

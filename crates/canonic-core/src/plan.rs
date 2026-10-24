//! The compiled plan: what codegen.ts knows about field order, stat naming and null/default/error
//! semantics, as data. The executor runs it. `source()` prints it for humans.

use crate::canonical::Semantic;
use crate::spec::{Path, Source, Status, Step};
use crate::transforms::{Builtin, CastTo, NamePart};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use std::fmt::Write;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Meta {
    pub provider: String,
    pub endpoint: String,
    pub version: u32,
    pub status: Status,
}

/// A canonical field: its own name (not the dotted path) and its index in `Plan::names`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Named<T> {
    pub name: String,
    pub name_idx: usize,
    pub value: T,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TxRef {
    Builtin(Builtin),
    /// `index` into the `custom` slice given to `compile`. `arg` is passed as written, unchecked.
    Custom {
        index: usize,
        name: String,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "present"
        )]
        arg: Option<Value>,
    },
}

/// Read a field, then `tx` in order, then the routing `validator`. Null or a rejection gives `default`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Scalar {
    pub source: Source,
    pub tx: Vec<TxRef>,
    pub validator: Option<Semantic>,
    /// A string, number or boolean. The grammar rejects `default: null`.
    pub default: Option<Value>,
    pub stat: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ListKind {
    /// Each element mapped to an object. Field stats are out of the elements seen.
    Item(Vec<Named<Field>>),
    /// Each element read at the path, then transformed. Element misses are not counted, errors are.
    Pluck(Path, Vec<TxRef>),
    /// Each element as it is, then transformed.
    Plain(Vec<TxRef>),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Field {
    /// A canonical field the spec does not map.
    Null,
    Scalar(Scalar),
    Object(Vec<Named<Field>>),
    List {
        from: Path,
        kind: ListKind,
        stat: usize,
    },
}

/// A routing field as `extract` reads it. `name` is the spec key, for example "company.domain".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoutingField {
    pub name: String,
    pub scalar: Scalar,
    pub anchor: Option<String>,
}

/// The newest plan JSON layout this build writes and reads. Format 1 plans still load.
pub const PLAN_FORMAT: u32 = 2;

/// The `format` stamp of a serialized plan, 1 or 2. `compile` writes 2 only when the plan holds a
/// language 2 rule ([`Plan::needs_v2`]), so other plans stay readable by format 1 builds. Reading
/// any other value (or none: serde's "missing field `format`") is an error, so an old or newer
/// plan fails to load instead of mapping wrongly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Format(pub u32);

impl Serialize for Format {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u32(self.0)
    }
}

impl<'de> Deserialize<'de> for Format {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Format, D::Error> {
        let v = Value::deserialize(d)?;
        if v == 1 || v == 2 {
            Ok(Format(v.as_u64().unwrap() as u32))
        } else {
            Err(serde::de::Error::custom(format!(
                "plan format {v} is not supported. This build reads format {PLAN_FORMAT}. Rebuild the plan."
            )))
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Plan {
    pub format: Format,
    pub meta: Meta,
    /// Every canonical field, in schema order.
    pub fields: Vec<Named<Field>>,
    /// Routing fields in schema order.
    pub routing: Vec<RoutingField>,
    /// Indexes into `routing` of the anchored fields, in the spec's `anchors` order: a provider
    /// that keeps its key order then costs `extract` one pass.
    #[serde(default)]
    pub anchors: Vec<usize>,
    /// Stat slot names in spec order, as codegen.ts names them: "company.domain", "jobs[]", "jobs[].title".
    pub stat_names: Vec<String>,
    /// The custom transform names given to `compile`. `TxRef::Custom::index` points here.
    pub custom_names: Vec<String>,
    /// Interned canonical field names. `Named::name_idx` points here.
    pub names: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompileError {
    pub message: String,
}

impl std::fmt::Display for CompileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for CompileError {}

/// A present field is `Some`, even when it is `null`. Only an absent field is `None`.
fn present<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Value>, D::Error> {
    Value::deserialize(d).map(Some)
}

impl Plan {
    /// True when a field needs plan format 2. No language 1 variant does. The matches name every
    /// variant, so a new one must say which format it needs.
    pub fn needs_v2(&self) -> bool {
        fn field(f: &Field) -> bool {
            match f {
                Field::Null | Field::Scalar(_) => false,
                Field::Object(c) => c.iter().any(|n| field(&n.value)),
                Field::List { kind, .. } => match kind {
                    ListKind::Item(c) => c.iter().any(|n| field(&n.value)),
                    ListKind::Pluck(..) | ListKind::Plain(_) => false,
                },
            }
        }
        self.fields.iter().any(|n| field(&n.value))
    }

    /// One line per canonical field, in schema order, e.g.
    /// `email <- coalesce(a, b) | trim | lower | validate email`.
    pub fn source(&self) -> String {
        let mut out = String::new();
        lines(&mut out, "", &self.fields);
        out
    }
}

fn lines(out: &mut String, prefix: &str, fields: &[Named<Field>]) {
    for f in fields {
        let name = format!("{prefix}{}", f.name);
        match &f.value {
            Field::Null => _ = writeln!(out, "{name} <- null"),
            Field::Object(c) => lines(out, &format!("{name}."), c),
            Field::Scalar(s) => {
                let src = match &s.source {
                    Source::From(p) => path(p),
                    Source::FromMany(ps) => format!("from({})", list(ps)),
                    Source::Coalesce(ps) => format!("coalesce({})", list(ps)),
                };
                let mut line = format!("{name} <- {src}{}", pipe(&s.tx));
                if let Some(v) = s.validator {
                    line += &format!(" | validate {}", v.name());
                }
                if let Some(d) = &s.default {
                    line += &format!(" ?? {d}");
                }
                _ = writeln!(out, "{line}");
            }
            Field::List { from, kind, .. } => {
                let from = path(from);
                match kind {
                    ListKind::Item(c) => {
                        _ = writeln!(out, "{name}[] <- {from}[]");
                        lines(out, &format!("{name}[]."), c);
                    }
                    ListKind::Pluck(p, tx) => {
                        _ = writeln!(out, "{name}[] <- {from}[].{}{}", path(p), pipe(tx))
                    }
                    ListKind::Plain(tx) => _ = writeln!(out, "{name}[] <- {from}[]{}", pipe(tx)),
                }
            }
        }
    }
}

fn list(ps: &[Path]) -> String {
    ps.iter().map(path).collect::<Vec<_>>().join(", ")
}

/// A path in spec syntax: identifiers joined by dots, `[n]`, `["quoted"]`, or `$` when empty.
fn path(p: &Path) -> String {
    if p.is_empty() {
        return "$".into();
    }
    let mut s = String::new();
    for step in p {
        match step {
            Step::Idx(i) => _ = write!(s, "[{i}]"),
            Step::Key(k) if ident(k) => {
                if !s.is_empty() {
                    s.push('.');
                }
                s += k;
            }
            Step::Key(k) => _ = write!(s, "[{}]", Value::from(k.as_str())),
        }
    }
    s
}

fn ident(k: &str) -> bool {
    let b = k.as_bytes();
    k != "$"
        && b.first()
            .is_some_and(|c| c.is_ascii_alphabetic() || *c == b'_' || *c == b'$')
        && b.iter()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'$' | b'-'))
}

fn pipe(tx: &[TxRef]) -> String {
    let mut s = String::new();
    for t in tx {
        let (name, arg): (&str, Option<Value>) = match t {
            TxRef::Custom { name, arg, .. } => (name, arg.clone()),
            TxRef::Builtin(b) => match b {
                Builtin::Cast(c) => ("cast", Some(cast_name(*c).into())),
                Builtin::SplitName(NamePart::First) => ("split_name", Some("first".into())),
                Builtin::SplitName(NamePart::Last) => ("split_name", Some("last".into())),
                Builtin::Join(sep) => ("join", sep.clone().map(Value::from)),
                Builtin::Lower => ("lower", None),
                Builtin::Trim => ("trim", None),
                Builtin::ParseDate => ("parse_date", None),
                Builtin::NormalizePhone => ("normalize_phone", None),
                Builtin::NormalizeLinkedin => ("normalize_linkedin", None),
                Builtin::ToDomain => ("to_domain", None),
            },
        };
        match arg {
            Some(a) => _ = write!(s, " | {name} {a}"),
            None => _ = write!(s, " | {name}"),
        }
    }
    s
}

fn cast_name(c: CastTo) -> &'static str {
    match c {
        CastTo::String => "string",
        CastTo::Number => "number",
        CastTo::Integer => "integer",
        CastTo::Boolean => "boolean",
    }
}

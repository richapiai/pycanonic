//! The compiled plan: what codegen.ts knows about field order, stat naming and null/default/error
//! semantics, as data. The executor runs it. `source()` prints it for humans.

use crate::canonical::{Empty, Semantic};
use crate::cond::Cond;
use crate::spec::{Aggregate, Path, Source, Status, Step};
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

/// Language 2 array rule options. The default is language 1 behavior. The plan holds spec paths,
/// the executor its interned paths (`ListOps<KeyPath>`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ListOps<P = Path> {
    /// The output for a missing or null source: `Null` or `List`.
    pub missing: Empty,
    /// The output for an empty source array, also after filtering: `List` or `Null`.
    pub empty: Empty,
    /// A source that is a scalar or an object is a one-element array.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub wrap: bool,
    /// A6, applied in field order. `flatten`: each element is replaced by the elements of the array
    /// at each path in turn (`from: "jobs[].skills[]"` holds `[skills]`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub flatten: Vec<P>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter: Option<Cond<P>>,
    /// Keeps the first element of each key at the path. The empty path is the whole element.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unique: Option<P>,
    /// The path to sort by and whether the order is descending. Stable, null last.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sort: Option<(P, bool)>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    /// A5, after `limit`: the key path and whether groups are consecutive. Each group is mapped
    /// once, its first element as the element and the group as `$group`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<(P, bool)>,
    /// The source is `$group`: the group of the grouped item map around, not a path.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub from_group: bool,
}

impl<P> Default for ListOps<P> {
    fn default() -> ListOps<P> {
        ListOps {
            missing: Empty::Null,
            empty: Empty::List,
            wrap: false,
            flatten: Vec::new(),
            filter: None,
            unique: None,
            sort: None,
            limit: None,
            group: None,
            from_group: false,
        }
    }
}

impl ListOps {
    fn is_default(&self) -> bool {
        *self == ListOps::default()
    }
}

impl<P> ListOps<P> {
    /// True when the elements must be collected (`exec::collect`) instead of streamed.
    pub fn collects(&self) -> bool {
        self.wrap
            || !self.flatten.is_empty()
            || self.filter.is_some()
            || self.unique.is_some()
            || self.sort.is_some()
            || self.limit.is_some()
            || self.group.is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Field {
    /// A canonical field the spec does not map.
    Null,
    Scalar(Scalar),
    /// A canonical object. Language 2: when a `present` path is missing or null, the object
    /// is null and nothing inside it is mapped. With `if_empty` (`Null` or `Object`, from the spec's
    /// `if_empty` or the node's `x-empty`), an object that writes nothing but null is
    /// written as that. Without either it is written as `[members]`, as in format 1.
    #[serde(serialize_with = "object_out", deserialize_with = "object_in")]
    Object {
        members: Vec<Named<Field>>,
        present: Vec<Path>,
        if_empty: Option<Empty>,
    },
    List {
        from: Path,
        kind: ListKind,
        stat: usize,
        #[serde(default, skip_serializing_if = "ListOps::is_default")]
        ops: ListOps,
    },
    /// Language 2 (`x-empty: "missing"`): the key is left out of its object when the field maps to
    /// null. An object field is left out when no leaf inside it is written as something other
    /// than null.
    OmitNull(Box<Field>),
    /// Language 2: a value written as it is, whatever the body. Always a fill. The `usize` is the stat slot.
    Const(Value, usize),
    /// Language 2: an aggregate over an array or a group. The `usize` is the stat slot.
    Aggregate(Aggregate, usize),
    /// Language 2: a canonical field the spec does not map, written as its `x-empty`:
    /// `List` (`[]`) or `Object` (`{}`). An unmapped null or left-out field is `Null` or `OmitNull`.
    Empty(Empty),
}

/// The members, present paths and option of a `Field::Object`, in the variant's field order.
type ObjectParts = (Vec<Named<Field>>, Vec<Path>, Option<Empty>);

#[derive(Serialize, Deserialize)]
#[serde(untagged)]
enum ObjectForm {
    /// Format 1: the members alone.
    Plain(Vec<Named<Field>>),
    Optional {
        members: Vec<Named<Field>>,
        present: Vec<Path>,
        if_empty: Option<Empty>,
    },
}

fn object_out<S: serde::Serializer>(
    members: &[Named<Field>],
    present: &[Path],
    if_empty: &Option<Empty>,
    s: S,
) -> Result<S::Ok, S::Error> {
    if present.is_empty() && if_empty.is_none() {
        return members.serialize(s);
    }
    #[derive(Serialize)]
    struct Optional<'a> {
        members: &'a [Named<Field>],
        present: &'a [Path],
        if_empty: Option<Empty>,
    }
    let if_empty = *if_empty;
    Optional {
        members,
        present,
        if_empty,
    }
    .serialize(s)
}

fn object_in<'de, D: Deserializer<'de>>(d: D) -> Result<ObjectParts, D::Error> {
    Ok(match ObjectForm::deserialize(d)? {
        ObjectForm::Plain(members) => (members, Vec::new(), None),
        ObjectForm::Optional {
            members,
            present,
            if_empty,
        } => (members, present, if_empty),
    })
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
                "plan format {v} is not supported. This build reads format 1 or {PLAN_FORMAT}. Rebuild the plan."
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

impl Field {
    /// The field an `OmitNull` wraps, or the field itself.
    pub fn inner(&self) -> &Field {
        match self {
            Field::OmitNull(f) => f.inner(),
            f => f,
        }
    }
}

impl Plan {
    /// True when a field needs plan format 2. No language 1 variant does. The matches name every
    /// variant, so a new one must say which format it needs.
    pub fn needs_v2(&self) -> bool {
        fn field(f: &Field) -> bool {
            match f {
                Field::Null | Field::Scalar(_) => false,
                Field::Const(..) | Field::OmitNull(_) | Field::Empty(_) | Field::Aggregate(..) => {
                    true
                }
                Field::Object {
                    members,
                    present,
                    if_empty,
                } => {
                    !present.is_empty()
                        || if_empty.is_some()
                        || members.iter().any(|n| field(&n.value))
                }
                Field::List { kind, ops, .. } => {
                    !ops.is_default()
                        || match kind {
                            ListKind::Item(c) => c.iter().any(|n| field(&n.value)),
                            ListKind::Pluck(..) | ListKind::Plain(_) => false,
                        }
                }
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
        line(out, &format!("{prefix}{}", f.name), &f.value);
    }
}

fn line(out: &mut String, name: &str, field: &Field) {
    match field {
        Field::Null => _ = writeln!(out, "{name} <- null"),
        Field::OmitNull(f) => {
            _ = writeln!(out, "{name}: left out when null");
            line(out, name, f);
        }
        Field::Const(v, _) => _ = writeln!(out, "{name} <- const {v}"),
        Field::Empty(e) => {
            _ = writeln!(
                out,
                "{name} <- {}",
                if *e == Empty::List { "[]" } else { "{}" }
            )
        }
        Field::Object {
            members,
            present,
            if_empty,
        } => {
            if !present.is_empty() {
                _ = writeln!(out, "{name}: null unless present({})", list(present));
            }
            match if_empty {
                Some(Empty::Object) => _ = writeln!(out, "{name}: {{}} when empty"),
                Some(_) => _ = writeln!(out, "{name}: null when empty"),
                None => {}
            }
            lines(out, &format!("{name}."), members)
        }
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
        Field::Aggregate(a, _) => {
            let mut from = flat(&a.from, &a.flatten);
            if !a.pluck.is_empty() {
                from += &format!("[].{}", path(&a.pluck)).replace(".[", "[");
            } else if !a.flatten.is_empty() {
                from += "[]";
            }
            let mut line = format!("{name} <- {}({from})", a.op.name());
            if a.propagate {
                line += " | nulls propagate";
            }
            if let Some(sep) = &a.sep {
                line += &format!(" | sep {}", Value::from(sep.as_str()));
            }
            _ = writeln!(out, "{line}");
        }
        Field::List {
            from, kind, ops, ..
        } => {
            let from = flat(from, &ops.flatten);
            let mut opts = String::new();
            if ops.wrap {
                opts += " | wrap";
            }
            if let Some(c) = &ops.filter {
                opts += &format!(" | filter {}", cond(c));
            }
            if let Some(p) = &ops.unique {
                opts += &format!(" | unique {}", path(p));
            }
            if let Some((p, desc)) = &ops.sort {
                let order = if *desc { "desc" } else { "asc" };
                opts += &format!(" | sort_by {} {order}", path(p));
            }
            if let Some(n) = ops.limit {
                opts += &format!(" | limit {n}");
            }
            if let Some((p, consecutive)) = &ops.group {
                let all = if *consecutive { "" } else { " all" };
                opts += &format!(" | group_by {}{all}", path(p));
            }
            if ops.missing == Empty::List {
                opts += " | missing []";
            }
            if ops.empty == Empty::Null {
                opts += " | empty null";
            }
            match kind {
                ListKind::Item(c) => {
                    _ = writeln!(out, "{name}[] <- {from}[]{opts}");
                    lines(out, &format!("{name}[]."), c);
                }
                ListKind::Pluck(p, tx) => {
                    _ = writeln!(out, "{name}[] <- {from}[].{}{}{opts}", path(p), pipe(tx))
                }
                ListKind::Plain(tx) => _ = writeln!(out, "{name}[] <- {from}[]{}{opts}", pipe(tx)),
            }
        }
    }
}

/// A source path, then `[]` and each flatten path.
fn flat(from: &Path, flatten: &[Path]) -> String {
    let mut s = path(from);
    for p in flatten {
        s += "[]";
        if !p.is_empty() {
            s += &format!(".{}", path(p)).replace(".[", "[");
        }
    }
    s
}

fn cond(c: &Cond) -> String {
    match c {
        Cond::Present(p, b) => format!("{} present {b}", path(p)),
        Cond::Eq(p, v) => format!("{} eq {v}", path(p)),
        Cond::In(p, vs) => format!("{} in {}", path(p), Value::from(vs.clone())),
        Cond::Missing(p) => format!("missing {}", path(p)),
        Cond::Empty(p) => format!("empty {}", path(p)),
        Cond::All(cs) => {
            let all: Vec<String> = cs.iter().map(cond).collect();
            format!("all({})", all.join(", "))
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

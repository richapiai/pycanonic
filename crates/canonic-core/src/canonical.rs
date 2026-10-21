//! Canonical registry loader: a JSON Schema per endpoint in, a field tree out.
//! Leaves may carry `semantic`, `aliases` and `routing: true` next to their JSON Schema keywords.
//! Port of canonic/packages/canonic/src/canonical.ts.

use crate::spec::Issue;
use serde_json::{Map, Value};

/// The value's kind; exactly the semantics that have a routing validator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Semantic {
    Email,
    Phone,
    Domain,
    LinkedinUrl,
    Url,
    IsoDate,
    Money,
}

impl Semantic {
    /// Unknown tags are not a load error in canonic; they carry no semantic, and the routing check rejects them later.
    fn parse(tag: &str) -> Option<Self> {
        Some(match tag {
            "email" => Self::Email,
            "phone" => Self::Phone,
            "domain" => Self::Domain,
            "linkedin_url" => Self::LinkedinUrl,
            "url" => Self::Url,
            "iso_date" => Self::IsoDate,
            "money" => Self::Money,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    Scalar {
        semantic: Option<Semantic>,
        routing: bool,
        aliases: Vec<String>,
    },
    /// Children in schema order, which is the canonical output order.
    Object(Vec<(String, Node)>),
    /// `None`: no `items`, elements of unknown shape copied as they are.
    Array(Option<Box<Node>>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    pub kind: Kind,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Schema {
    pub root: Vec<(String, Node)>,
}

// toJSON and every own name of Object.prototype: a record or a view can't safely carry them.
const RESERVED: [&str; 13] = [
    "toJSON",
    "constructor",
    "__defineGetter__",
    "__defineSetter__",
    "hasOwnProperty",
    "__lookupGetter__",
    "__lookupSetter__",
    "isPrototypeOf",
    "propertyIsEnumerable",
    "toString",
    "valueOf",
    "__proto__",
    "toLocaleString",
];

/// `/^[A-Za-z_][A-Za-z0-9_]*$/` and not reserved.
fn name_ok(name: &str) -> bool {
    let mut b = name.bytes();
    b.next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == b'_')
        && b.all(|c| c.is_ascii_alphanumeric() || c == b'_')
        && !RESERVED.contains(&name)
}

fn has_type(s: &Map<String, Value>, t: &str) -> bool {
    match s.get("type") {
        Some(Value::String(v)) => v == t,
        Some(Value::Array(v)) => v.iter().any(|x| x.as_str() == Some(t)),
        _ => false,
    }
}

fn load_node(s: &Value, at: &str, in_array: bool) -> Result<Node, String> {
    let Some(s) = s.as_object() else {
        let at = if at.is_empty() { "the schema" } else { at };
        return Err(format!("{at} must be a schema object"));
    };
    if let Some(Value::Object(props)) = s.get("properties") {
        return Ok(Node {
            kind: Kind::Object(load_object(props, at, in_array)?),
        });
    }
    if has_type(s, "array") {
        // No `items`: elements of unknown shape, copied as they are.
        let items = s
            .get("items")
            .map(|i| load_node(i, &format!("{at}[]"), true).map(Box::new))
            .transpose()?;
        if let Some(Kind::Array(_)) = items.as_ref().map(|n| &n.kind) {
            return Err(format!(
                "{at} is an array of arrays, which specs can't address"
            ));
        }
        return Ok(Node {
            kind: Kind::Array(items),
        });
    }
    let routing = s.get("routing") == Some(&Value::Bool(true));
    if routing && in_array {
        return Err(format!("routing field {at} can't be inside an array"));
    }
    let aliases = match s.get("aliases") {
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(|x| x.as_str().map(String::from))
            .collect(),
        _ => vec![],
    };
    let semantic = s
        .get("semantic")
        .and_then(Value::as_str)
        .and_then(Semantic::parse);
    Ok(Node {
        kind: Kind::Scalar {
            semantic,
            routing,
            aliases,
        },
    })
}

fn load_object(
    props: &Map<String, Value>,
    at: &str,
    in_array: bool,
) -> Result<Vec<(String, Node)>, String> {
    props
        .iter()
        .map(|(name, child)| {
            let path = if at.is_empty() {
                name.clone()
            } else {
                format!("{at}.{name}")
            };
            if !name_ok(name) {
                return Err(format!(
                    "field name {} is not allowed",
                    Value::from(path.as_str())
                ));
            }
            Ok((name.clone(), load_node(child, &path, in_array)?))
        })
        .collect()
}

pub fn load_schema(schema: &Value) -> Result<Schema, Issue> {
    let issue = |message| Issue {
        at: "canonical schema".into(),
        message,
    };
    match load_node(schema, "", false).map_err(issue)?.kind {
        Kind::Object(root) => Ok(Schema { root }),
        _ => Err(issue("root must be an object with properties".into())),
    }
}

/// What compiling would reject in a canonical schema, without needing a spec. Empty when it loads.
pub fn validate_canonical(schema: &Value) -> Vec<Issue> {
    load_schema(schema).err().into_iter().collect()
}

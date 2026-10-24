//! Canonical registry loader: a JSON Schema per endpoint in, a field tree out.
//! Leaves can carry `semantic`, `aliases` and `routing: true` next to their JSON Schema keywords.
//! Port of canonic/packages/canonic/src/canonical.ts.

use crate::spec::Issue;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// The value's kind: exactly the semantics that have a routing validator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
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
    /// The tag as written in the schema.
    pub fn name(self) -> &'static str {
        match self {
            Self::Email => "email",
            Self::Phone => "phone",
            Self::Domain => "domain",
            Self::LinkedinUrl => "linkedin_url",
            Self::Url => "url",
            Self::IsoDate => "iso_date",
            Self::Money => "money",
        }
    }

    /// Unknown tags are not a load error in canonic. They carry no semantic, and the routing check rejects them later.
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
    /// Children in schema order, which is the canonical output order. `open` (language 2,
    /// `"x-open": true`): a map with dynamic keys, copied as it is. Specs do not address its children.
    Object {
        children: Vec<(String, Node)>,
        open: bool,
    },
    /// `items: None`: no `items`, elements of unknown shape copied as they are. `nested` (language
    /// 2): the elements are arrays, so the array can only be copied whole.
    Array {
        items: Option<Box<Node>>,
        nested: bool,
    },
}

/// Language 2 `x-empty`: the output for a missing or null source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Empty {
    Null,
    List,
    Object,
    /// The key is left out.
    Missing,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    pub kind: Kind,
    /// Read only in language 2. Task 4 uses it for output.
    pub empty: Option<Empty>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Schema {
    pub root: Vec<(String, Node)>,
}

// toJSON and every own name of Object.prototype: a record or a view cannot safely carry them.
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

fn load_node(s: &Value, at: &str, in_array: bool, lang2: bool) -> Result<Node, String> {
    let Some(s) = s.as_object() else {
        let at = if at.is_empty() { "the schema" } else { at };
        return Err(format!("{at} must be a schema object"));
    };
    let empty = match s.get("x-empty").filter(|_| lang2) {
        None => None,
        Some(v) => Some(match v.as_str() {
            Some("null") => Empty::Null,
            Some("[]") => Empty::List,
            Some("{}") => Empty::Object,
            Some("missing") => Empty::Missing,
            _ => {
                let at = if at.is_empty() { "the schema" } else { at };
                return Err(format!(
                    "{at}: x-empty must be one of null, [], {{}}, missing"
                ));
            }
        }),
    };
    let node = |kind| Ok(Node { kind, empty });
    let open = lang2 && s.get("x-open") == Some(&Value::Bool(true));
    let props = match s.get("properties") {
        Some(Value::Object(props)) => Some(props),
        _ => None,
    };
    if open || props.is_some() {
        let children = match props {
            Some(p) => load_object(p, at, in_array, lang2)?,
            None => vec![],
        };
        return node(Kind::Object { children, open });
    }
    if has_type(s, "array") {
        // No `items`: elements of unknown shape, copied as they are.
        let items = s
            .get("items")
            .map(|i| load_node(i, &format!("{at}[]"), true, lang2).map(Box::new))
            .transpose()?;
        let nested = matches!(
            items.as_deref(),
            Some(Node {
                kind: Kind::Array { .. },
                ..
            })
        );
        if nested && !lang2 {
            return Err(format!(
                "{at} is an array of arrays, which specs can't address"
            ));
        }
        return node(Kind::Array { items, nested });
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
    node(Kind::Scalar {
        semantic,
        routing,
        aliases,
    })
}

fn load_object(
    props: &Map<String, Value>,
    at: &str,
    in_array: bool,
    lang2: bool,
) -> Result<Vec<(String, Node)>, String> {
    props
        .iter()
        .map(|(name, child)| {
            let path = if at.is_empty() {
                name.clone()
            } else {
                format!("{at}.{name}")
            };
            // Language 2 takes any string as a name.
            if !lang2 && !name_ok(name) {
                return Err(format!(
                    "field name {} is not allowed",
                    Value::from(path.as_str())
                ));
            }
            Ok((name.clone(), load_node(child, &path, in_array, lang2)?))
        })
        .collect()
}

/// Loads the schema for a spec of `language` (1 or 2). Language 2 accepts any field name, open maps
/// (`x-open`), arrays of arrays and `x-empty`.
pub fn load_schema(schema: &Value, language: u8) -> Result<Schema, Issue> {
    let issue = |message| Issue {
        at: "canonical schema".into(),
        message,
    };
    match load_node(schema, "", false, language >= 2)
        .map_err(issue)?
        .kind
    {
        Kind::Object { children, .. } => Ok(Schema { root: children }),
        _ => Err(issue("root must be an object with properties".into())),
    }
}

/// What compiling rejects in a canonical schema, without needing a spec. Empty when it loads.
pub fn validate_canonical(schema: &Value) -> Vec<Issue> {
    load_schema(schema, 1).err().into_iter().collect()
}

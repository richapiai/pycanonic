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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    /// Language 2 (R55): `oneOf`, two or more titled object branches. A spec picks one per record
    /// with `one_of`. Each branch holds its children in schema order.
    Union { branches: Vec<Branch> },
}

/// A `oneOf` branch: its title and its children in schema order.
pub type Branch = (String, Vec<(String, Node)>);

/// Language 2 `x-empty`: the output for a missing or null source. The plan also uses it for the
/// array rule options `missing` and `empty` (`Null` or `List` there).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Empty {
    Null,
    List,
    Object,
    /// The key is left out.
    Missing,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Node {
    pub kind: Kind,
    /// Read only in language 2. A leaf takes `Null` or `Missing`, an array `Null`, `List` or
    /// `Missing`, an object `Null`, `Object` or `Missing`.
    pub empty: Option<Empty>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Schema {
    /// The root object's children. Empty for an array root.
    pub root: Vec<(String, Node)>,
    /// Language 2: the root node when it is an array, which a spec maps with the field `"[]"`.
    pub array: Option<Node>,
    /// Language 2 (R48): `"x-shape": "strict"` at the root. A node with no usable value writes
    /// its empty form (`empty_shape`).
    pub strict: bool,
}

/// R48: what a node with no usable value writes under strict shape. `None` is `x-empty: "missing"`,
/// no key. Without `x-empty`, an array is `[]`, a leaf null, and an object an object of its
/// children's empty forms.
pub fn empty_shape(node: &Node) -> Option<Value> {
    Some(match (node.empty, &node.kind) {
        (Some(Empty::Missing), _) => return None,
        (Some(Empty::Null), _) | (None, Kind::Scalar { .. }) => Value::Null,
        (Some(Empty::List), _) | (None, Kind::Array { .. }) => Value::Array(Vec::new()),
        (Some(Empty::Object), _) => Value::Object(Map::new()),
        (None, Kind::Object { children, .. }) => Value::Object(
            (children.iter())
                .filter_map(|(k, c)| Some((k.clone(), empty_shape(c)?)))
                .collect(),
        ),
        // No single shape exists, so a `oneOf` node writes null.
        (None, Kind::Union { .. }) => Value::Null,
    })
}

/// The empty form of a node with no usable value: `empty_shape` under strict shape, else its
/// `x-empty` (`[]` or `{}`), null without one. `None` leaves the key out.
pub fn empty_form(node: &Node, strict: bool) -> Option<Value> {
    match (strict, node.empty) {
        (true, _) => empty_shape(node),
        (false, Some(Empty::Missing)) => None,
        (false, Some(Empty::List | Empty::Object)) => empty_shape(node),
        (false, _) => Some(Value::Null),
    }
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
    let here = if at.is_empty() { "the schema" } else { at };
    let empty = match s.get("x-empty").filter(|_| lang2) {
        None => None,
        Some(v) => Some(match v.as_str() {
            Some("null") => Empty::Null,
            Some("[]") => Empty::List,
            Some("{}") => Empty::Object,
            Some("missing") => Empty::Missing,
            _ => {
                return Err(format!(
                    "{here}: x-empty must be one of null, [], {{}}, missing"
                ));
            }
        }),
    };
    let node = |kind: Kind| {
        let fits = match (&kind, empty) {
            (_, None | Some(Empty::Null | Empty::Missing)) => true,
            (Kind::Array { .. }, Some(e)) => e == Empty::List,
            (Kind::Object { .. } | Kind::Union { .. }, Some(e)) => e == Empty::Object,
            (Kind::Scalar { .. }, _) => false,
        };
        if !fits {
            let (tag, kind) = match empty {
                Some(Empty::List) => ("[]", "an array"),
                _ => ("{}", "an object"),
            };
            return Err(format!("{here}: x-empty \"{tag}\" needs {kind} node"));
        }
        Ok(Node { kind, empty })
    };
    let open = lang2 && s.get("x-open") == Some(&Value::Bool(true));
    let props = match s.get("properties") {
        Some(Value::Object(props)) => Some(props),
        _ => None,
    };
    // R47: a union is only a language 2 `oneOf` of object branches on a node without
    // `properties`. Any other `oneOf` (a nullable leaf, required alternatives) is ignored, as in 0.1.1.
    let union = match s.get("oneOf").and_then(Value::as_array) {
        Some(bs) if lang2 && !s.contains_key("properties") && !bs.is_empty() => bs
            .iter()
            .all(|b| matches!(b.get("properties"), Some(Value::Object(_))))
            .then_some(bs),
        _ => None,
    };
    if let Some(bs) = union {
        return node(Kind::Union {
            branches: load_union(bs, at, here, in_array, lang2)?,
        });
    }
    if open && props.is_none() && !has_type(s, "object") {
        return Err(format!("{here}: x-open needs an object node"));
    }
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

/// The branches of a `oneOf` node: each `{"title": <identifier>, "properties": {...}}`, its
/// children at `<at>(<title>).<name>`.
fn load_union(
    branches: &[Value],
    at: &str,
    here: &str,
    in_array: bool,
    lang2: bool,
) -> Result<Vec<Branch>, String> {
    if branches.len() < 2 {
        return Err(format!("{here}: oneOf needs two or more branches"));
    }
    let mut out: Vec<Branch> = Vec::new();
    for b in branches {
        let Some(title) = b.get("title").and_then(Value::as_str) else {
            return Err(format!("{here}: oneOf branch needs a title"));
        };
        let quoted = Value::from(title);
        if !name_ok(title) {
            return Err(format!(
                "{here}: oneOf branch title {quoted} is not an identifier"
            ));
        }
        if out.iter().any(|(t, _)| t == title) {
            return Err(format!("{here}: duplicate branch title {quoted}"));
        }
        let Some(Value::Object(props)) = b.get("properties") else {
            unreachable!("load_node checked every branch");
        };
        let children = load_object(props, &format!("{at}({title})"), in_array, lang2)?;
        out.push((title.to_string(), children));
    }
    Ok(out)
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
/// (`x-open`), arrays of arrays, `x-empty` and an array root.
pub fn load_schema(schema: &Value, language: u8) -> Result<Schema, Issue> {
    let issue = |message| Issue {
        at: "canonical schema".into(),
        message,
    };
    let node = load_node(schema, "", false, language >= 2).map_err(issue)?;
    let strict = match schema.get("x-shape").filter(|_| language >= 2) {
        None => false,
        Some(v) if v == "strict" => true,
        Some(_) => return Err(issue("x-shape must be \"strict\"".into())),
    };
    match node.kind {
        Kind::Object { children, .. } => Ok(Schema {
            root: children,
            array: None,
            strict,
        }),
        // The body is always there, so there is no key to leave out.
        Kind::Array { .. } if language >= 2 && node.empty == Some(Empty::Missing) => {
            Err(issue("an array root can't have x-empty \"missing\"".into()))
        }
        Kind::Array { .. } if language >= 2 => Ok(Schema {
            root: Vec::new(),
            array: Some(node),
            strict,
        }),
        _ => Err(issue("root must be an object with properties".into())),
    }
}

/// What compiling rejects in a canonical schema, without needing a spec. Empty when it loads.
pub fn validate_canonical(schema: &Value) -> Vec<Issue> {
    load_schema(schema, 1).err().into_iter().collect()
}

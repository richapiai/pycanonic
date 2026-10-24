//! The document the executor reads: the borrowed JSON tree here, Python objects in the extension.

use crate::json::{Node, key_hash, same_key};
use crate::value::Value;
use std::any::Any;
use std::borrow::Cow;
use std::collections::HashMap;
use std::collections::hash_map::Entry;

/// An object key on a plan path. `tag` is unique per distinct key string within one `Program`
/// (`Program::keys()[tag] == s`), so a host can intern its own string once per tag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Key {
    pub s: String,
    pub tag: u64,
    /// `json::key_hash(s)`, for the parse tree's key tables. `Key::new` sets it.
    pub(crate) hash: u64,
}

impl Key {
    pub fn new(s: String, tag: u64) -> Key {
        let hash = key_hash(s.as_bytes());
        Key { s, tag, hash }
    }
}

/// One step of a path the executor reads, its keys interned per `Program`.
#[derive(Debug, Clone, PartialEq)]
pub enum KeyStep {
    Key(Key),
    Idx(usize),
}

pub type KeyPath = Vec<KeyStep>;

/// The value at `path`, `None` when it or any step is missing or null.
pub(crate) fn read<D: Doc>(doc: Option<&D>, path: &[KeyStep]) -> Option<D> {
    let mut cur = doc?.clone();
    for s in path {
        cur = match s {
            KeyStep::Key(k) => cur.get(k)?,
            KeyStep::Idx(i) => cur.idx(*i)?,
        };
    }
    (!cur.is_null()).then_some(cur)
}

/// Read access to a parsed document. A missing member and a `null` are both "absent" to the plan.
// `len` is "is this an array, and how long". An is_empty adds nothing.
#[allow(clippy::len_without_is_empty)]
pub trait Doc: Clone {
    /// The member `key` of an object (not an array). The last occurrence wins, as in `JSON.parse`.
    fn get(&self, key: &Key) -> Option<Self>;
    /// Element `i` of an array.
    fn idx(&self, i: usize) -> Option<Self>;
    /// The value as a scalar. `Value::Container` for an object or array.
    fn leaf(&self) -> Value<'_>;
    fn is_null(&self) -> bool;
    /// Element count of an array, `None` for anything else.
    fn len(&self) -> Option<usize>;
    /// Element `i` while walking a list `from`.
    fn item(&self, i: usize) -> Option<Self>;
    /// Members of an object in document order, for passing a container through. `None` otherwise.
    fn members(&self) -> Option<Vec<(Cow<'_, str>, Self)>>;
    /// The host's own object behind this value (a Python object in the extension), which a sink
    /// can pass through as it is instead of rebuilding it. `None` for the parsed JSON tree.
    fn host(&self) -> Option<&dyn Any> {
        None
    }
}

/// Integer text that fits i64 is `Int`, other integer text `BigInt`, a fraction or exponent `Float`.
fn number(raw: &str) -> Value<'_> {
    if raw.bytes().any(|b| matches!(b, b'.' | b'e' | b'E')) {
        Value::Float(raw.parse().unwrap_or(f64::NAN))
    } else {
        raw.parse()
            .map_or(Value::BigInt(Cow::Borrowed(raw)), Value::Int)
    }
}

impl<'a, 'b> Doc for &'a Node<'b> {
    /// A duplicated key reads its last value, as `JSON.parse` does. See `members`.
    fn get(&self, key: &Key) -> Option<Self> {
        match self {
            Node::Obj(m) => (m.iter())
                .rfind(|(k, _)| same_key(k.as_bytes(), key.s.as_bytes()))
                .map(|(_, v)| v),
            Node::Indexed(ix) => ix.get(key.s.as_bytes(), key.hash),
            _ => None,
        }
    }

    fn idx(&self, i: usize) -> Option<Self> {
        match self {
            Node::Arr(a) => a.get(i),
            _ => None,
        }
    }

    fn leaf(&self) -> Value<'_> {
        match self {
            Node::Null => Value::Null,
            Node::Bool(b) => Value::Bool(*b),
            Node::Num(raw) => number(raw),
            Node::Str(s) => Value::Str(Cow::Borrowed(s)),
            // Skipped is a container the filter pruned: the parser always reads scalars it reaches.
            Node::Arr(_) | Node::Obj(_) | Node::Indexed(_) | Node::Skipped => Value::Container,
        }
    }

    fn is_null(&self) -> bool {
        matches!(self, Node::Null)
    }

    fn len(&self) -> Option<usize> {
        match self {
            Node::Arr(a) => Some(a.len()),
            _ => None,
        }
    }

    fn item(&self, i: usize) -> Option<Self> {
        self.idx(i)
    }

    /// Like `JSON.parse`, a repeated key keeps its first position and its last value, the value
    /// `get` reads too. Only `extract` keeps the first value (known-limits.md, "Duplicate keys").
    fn members(&self) -> Option<Vec<(Cow<'_, str>, Self)>> {
        let m = match self {
            Node::Obj(m) => m,
            Node::Indexed(ix) => &ix.members,
            _ => return None,
        };
        let mut out: Vec<(Cow<'_, str>, Self)> = Vec::with_capacity(m.len());
        let mut at: HashMap<&str, usize> = HashMap::with_capacity(m.len());
        for (k, v) in m {
            match at.entry(k) {
                Entry::Occupied(e) => out[*e.get()].1 = v,
                Entry::Vacant(e) => {
                    e.insert(out.len());
                    out.push((Cow::Borrowed(k), v));
                }
            }
        }
        Some(out)
    }
}

/// A JSON value from the plan (a `const`), so a sink passes it through like a document value.
impl Doc for &serde_json::Value {
    fn get(&self, key: &Key) -> Option<Self> {
        self.as_object()?.get(&key.s)
    }

    fn idx(&self, i: usize) -> Option<Self> {
        self.as_array()?.get(i)
    }

    fn leaf(&self) -> Value<'_> {
        use serde_json::Value as J;
        match self {
            J::Null => Value::Null,
            J::Bool(b) => Value::Bool(*b),
            J::Number(n) => match (n.as_i64(), n.is_u64()) {
                (Some(i), _) => Value::Int(i),
                (None, true) => Value::BigInt(Cow::Owned(n.to_string())),
                (None, false) => Value::Float(n.as_f64().unwrap_or(f64::NAN)),
            },
            J::String(s) => Value::Str(Cow::Borrowed(s)),
            J::Array(_) | J::Object(_) => Value::Container,
        }
    }

    fn is_null(&self) -> bool {
        serde_json::Value::is_null(self)
    }

    fn len(&self) -> Option<usize> {
        self.as_array().map(Vec::len)
    }

    fn item(&self, i: usize) -> Option<Self> {
        self.idx(i)
    }

    fn members(&self) -> Option<Vec<(Cow<'_, str>, Self)>> {
        let m = self.as_object()?;
        Some(
            m.iter()
                .map(|(k, v)| (Cow::Borrowed(k.as_str()), v))
                .collect(),
        )
    }
}

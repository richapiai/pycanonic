//! Where the executor writes: `RustSink` builds an `Out` tree, the extension builds Python objects.

use crate::doc::Doc;
use crate::json::MAX_DEPTH;
use crate::value::Value;

pub trait Sink {
    type Out;
    fn scalar(&mut self, v: Value<'_>) -> Self::Out;
    /// A source value with no transforms, containers included, as it is.
    fn passthrough<D: Doc>(&mut self, d: &D) -> Self::Out;
    fn list(&mut self, items: Vec<Self::Out>) -> Self::Out;
    /// Called before the first child of an object (`object`) or a list, then `next` before each
    /// child, then `object` or `list` with the children. A sink that writes as it goes (JSON text)
    /// uses these. The default does nothing.
    fn open(&mut self, object: bool) {
        let _ = object;
    }
    /// Before each child: its canonical name (`Plan::names` index) in an object, `None` in a list.
    fn next(&mut self, name: Option<usize>, first: bool) {
        let _ = (name, first);
    }
    /// A canonical object. Each `usize` indexes `Plan::names`.
    fn object(&mut self, members: Vec<(usize, Self::Out)>) -> Self::Out;
}

#[derive(Debug, Clone, PartialEq)]
pub enum Out {
    Null,
    Bool(bool),
    Int(i64),
    /// Integer text past i64, as written in the source.
    BigInt(String),
    Float(f64),
    Str(String),
    List(Vec<Out>),
    /// A canonical object. Keys index `Plan::names`.
    Obj(Vec<(usize, Out)>),
    /// A source object passed through, keys as in the document.
    Map(Vec<(String, Out)>),
}

pub struct RustSink;

impl Sink for RustSink {
    type Out = Out;

    fn scalar(&mut self, v: Value<'_>) -> Out {
        match v {
            Value::Null | Value::Container => Out::Null,
            Value::Bool(b) => Out::Bool(b),
            Value::Int(i) => Out::Int(i),
            Value::BigInt(s) => Out::BigInt(s.into_owned()),
            Value::Float(f) => Out::Float(f),
            Value::Str(s) => Out::Str(s.into_owned()),
        }
    }

    fn passthrough<D: Doc>(&mut self, d: &D) -> Out {
        if let Some(n) = d.len() {
            let items = (0..n)
                .map(|i| d.item(i).map_or(Out::Null, |x| self.passthrough(&x)))
                .collect();
            return Out::List(items);
        }
        if let Some(m) = d.members() {
            return Out::Map(
                m.into_iter()
                    .map(|(k, v)| (k.into_owned(), self.passthrough(&v)))
                    .collect(),
            );
        }
        self.scalar(d.leaf())
    }

    fn list(&mut self, items: Vec<Out>) -> Out {
        Out::List(items)
    }

    fn object(&mut self, members: Vec<(usize, Out)>) -> Out {
        Out::Obj(members)
    }
}

/// A document value as an `Out` with keys by name (`Out::Map`), for a walk that must stop on a host
/// object that contains itself: `None` past `MAX_DEPTH` nested containers, as the parser allows, or
/// past `budget` values. `enclosing` counts the containers around `d`.
pub(crate) fn copy<D: Doc>(d: &D, enclosing: usize, budget: &mut usize) -> Option<Out> {
    if *budget == 0 {
        return None;
    }
    *budget -= 1;
    if let Some(n) = d.len() {
        if enclosing >= MAX_DEPTH {
            return None;
        }
        let items = (0..n).map(|i| {
            d.item(i)
                .map_or(Some(Out::Null), |x| copy(&x, enclosing + 1, budget))
        });
        return items.collect::<Option<_>>().map(Out::List);
    }
    if let Some(m) = d.members() {
        if enclosing >= MAX_DEPTH {
            return None;
        }
        let members = m
            .into_iter()
            .map(|(k, v)| Some((k.into_owned(), copy(&v, enclosing + 1, budget)?)));
        return members.collect::<Option<_>>().map(Out::Map);
    }
    Some(RustSink.scalar(d.leaf()))
}

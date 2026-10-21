//! Where the executor writes: `RustSink` builds an `Out` tree, the extension builds Python objects.

use crate::doc::Doc;
use crate::value::Value;

pub trait Sink {
    type Out;
    fn scalar(&mut self, v: Value<'_>) -> Self::Out;
    /// A source value with no transforms, containers included, as it is.
    fn passthrough<D: Doc>(&mut self, d: &D) -> Self::Out;
    fn list(&mut self, items: Vec<Self::Out>) -> Self::Out;
    /// A canonical object; each `usize` indexes `Plan::names`.
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
    /// A canonical object; keys index `Plan::names`.
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

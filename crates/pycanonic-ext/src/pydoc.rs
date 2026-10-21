//! `Doc` over a Python object tree: `dict` members by interned key, `list`/`tuple` elements, scalars.

use canonic_core::{Doc, Key, Value};
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyDict, PyFloat, PyInt, PyList, PyString, PyTuple};
use std::any::Any;
use std::borrow::Cow;

#[derive(Clone)]
pub struct PyDoc<'a, 'py> {
    pub obj: Bound<'py, PyAny>,
    /// Interned key strings, indexed by `Key::tag`.
    pub keys: &'a [Py<PyString>],
}

/// UTF-8 text of `s`. A lone surrogate, which UTF-8 can't hold, becomes one U+FFFD as in the parser.
pub fn text<'s>(s: &'s Bound<'_, PyString>) -> Cow<'s, str> {
    if let Ok(t) = s.to_str() {
        return Cow::Borrowed(t);
    }
    let fixed = s
        .call_method1("encode", ("utf-16", "surrogatepass"))
        .and_then(|b| b.call_method1("decode", ("utf-16", "replace")));
    match fixed
        .as_ref()
        .map(|f| f.cast::<PyString>().map(|f| f.to_str()))
    {
        Ok(Ok(Ok(t))) => Cow::Owned(t.to_owned()),
        _ => Cow::Owned(s.to_string_lossy().into_owned()),
    }
}

/// A Python scalar as a `Value`: `bool` before `int` (a subclass), an `int` past i64 as `BigInt`.
/// Anything that isn't `None`, `str`, `bool`, `int` or `float` is `Container`.
pub fn leaf<'o>(o: &'o Bound<'_, PyAny>) -> Value<'o> {
    if o.is_none() {
        Value::Null
    } else if let Ok(s) = o.cast::<PyString>() {
        Value::Str(text(s))
    } else if let Ok(b) = o.cast::<PyBool>() {
        Value::Bool(b.is_true())
    } else if o.is_instance_of::<PyInt>() {
        match o.extract::<i64>() {
            Ok(i) => Value::Int(i),
            Err(_) => o.str().map_or(Value::Null, |s| {
                Value::BigInt(Cow::Owned(text(&s).into_owned()))
            }),
        }
    } else if let Ok(f) = o.cast::<PyFloat>() {
        Value::Float(f.value())
    } else {
        Value::Container
    }
}

impl<'py> PyDoc<'_, 'py> {
    fn wrap(&self, obj: Bound<'py, PyAny>) -> Self {
        PyDoc {
            obj,
            keys: self.keys,
        }
    }
}

impl Doc for PyDoc<'_, '_> {
    fn get(&self, key: &Key) -> Option<Self> {
        let d = self.obj.cast::<PyDict>().ok()?;
        let k = self.keys.get(key.tag as usize)?.bind(self.obj.py());
        d.get_item(k).ok().flatten().map(|v| self.wrap(v))
    }

    fn idx(&self, i: usize) -> Option<Self> {
        let v = if let Ok(l) = self.obj.cast::<PyList>() {
            l.get_item(i).ok()
        } else {
            self.obj.cast::<PyTuple>().ok()?.get_item(i).ok()
        };
        v.map(|v| self.wrap(v))
    }

    fn leaf(&self) -> Value<'_> {
        leaf(&self.obj)
    }

    fn is_null(&self) -> bool {
        self.obj.is_none()
    }

    fn len(&self) -> Option<usize> {
        if let Ok(l) = self.obj.cast::<PyList>() {
            Some(l.len())
        } else {
            self.obj.cast::<PyTuple>().ok().map(|t| t.len())
        }
    }

    fn item(&self, i: usize) -> Option<Self> {
        self.idx(i)
    }

    fn members(&self) -> Option<Vec<(Cow<'_, str>, Self)>> {
        let d = self.obj.cast::<PyDict>().ok()?;
        let key = |k: &Bound<'_, PyAny>| match k.cast::<PyString>() {
            Ok(s) => text(s).into_owned(),
            Err(_) => k
                .str()
                .map_or_else(|_| String::new(), |s| text(&s).into_owned()),
        };
        Some(
            d.iter()
                .map(|(k, v)| (Cow::Owned(key(&k)), self.wrap(v)))
                .collect(),
        )
    }

    fn host(&self) -> Option<&dyn Any> {
        Some(self.obj.as_unbound())
    }
}

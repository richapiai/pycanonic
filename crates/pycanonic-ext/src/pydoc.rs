//! `Doc` over a Python object tree: `dict` members by interned key, `list`/`tuple` elements, scalars.

use canonic_core::{Doc, Key, Value};
use pyo3::exceptions::{PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyDict, PyFloat, PyInt, PyList, PyString, PyTuple};
use serde_json::Value as J;
use std::any::Any;
use std::borrow::Cow;

#[derive(Clone)]
pub struct PyDoc<'a, 'py> {
    pub obj: Bound<'py, PyAny>,
    /// Interned key strings, indexed by `Key::tag`.
    pub keys: &'a [Py<PyString>],
}

/// UTF-8 text of `s`. A lone surrogate, which UTF-8 cannot hold, becomes one U+FFFD as in the parser.
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
/// Anything that is not `None`, `str`, `bool`, `int` or `float` is `Container`.
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
            Err(_) => match o.str() {
                Ok(s) => Value::BigInt(Cow::Owned(text(&s).into_owned())),
                // CPython refuses `str(int)` past 4300 digits. JSON.parse reads that as a double.
                Err(_) => Value::Float(o.extract::<f64>().unwrap_or_else(|_| {
                    if o.gt(0).unwrap_or(false) {
                        f64::INFINITY
                    } else {
                        f64::NEG_INFINITY
                    }
                })),
            },
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
        if let Some(v) = d.get_item(k).ok().flatten() {
            return Some(self.wrap(v));
        }
        // A spec's lone surrogate is U+FFFD in the plan. A dict key holding one matches it the same way.
        // ASCII first: a word-at-a-time check, where `contains(char)` is a memcmp per candidate.
        if key.s.is_ascii() || !key.s.contains(char::REPLACEMENT_CHARACTER) {
            return None;
        }
        d.iter()
            .find(|(k, _)| k.cast::<PyString>().is_ok_and(|s| text(s) == key.s))
            .map(|(_, v)| self.wrap(v))
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
        // Keys as `json.dumps` writes them. A key it cannot encode (a tuple) falls back to `str(k)`.
        let key = |k: &Bound<'_, PyAny>| {
            if let Ok(s) = k.cast::<PyString>() {
                return text(s).into_owned();
            }
            if let Ok(b) = k.cast::<PyBool>() {
                return b.is_true().to_string();
            }
            if k.is_none() {
                return "null".to_string();
            }
            if let Ok(f) = k.cast::<PyFloat>() {
                let f = f.value();
                if f.is_nan() {
                    return "NaN".to_string();
                }
                if f.is_infinite() {
                    return if f > 0.0 { "Infinity" } else { "-Infinity" }.to_string();
                }
                return k
                    .repr()
                    .map_or_else(|_| f.to_string(), |r| text(&r).into_owned());
            }
            k.str()
                .map_or_else(|_| String::new(), |s| text(&s).into_owned())
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

/// Deep enough for any spec, schema or request context. A cyclic object fails instead of overflowing the stack.
const MAX_DEPTH: usize = 256;

/// A Python value as JSON, `dict` key order kept.
pub(crate) fn json(o: &Bound<'_, PyAny>, depth: usize) -> PyResult<J> {
    if depth > MAX_DEPTH {
        return Err(PyValueError::new_err("nested too deeply (or cyclic)"));
    }
    if o.is_none() {
        Ok(J::Null)
    } else if let Ok(s) = o.cast::<PyString>() {
        Ok(J::String(text(s).into_owned()))
    } else if let Ok(b) = o.cast::<PyBool>() {
        Ok(J::Bool(b.is_true()))
    } else if o.is_instance_of::<PyInt>() {
        // Past i64 and u64 a number is an f64, as JSON.parse reads it.
        Ok(match (o.extract::<i64>(), o.extract::<u64>()) {
            (Ok(i), _) => J::from(i),
            (_, Ok(u)) => J::from(u),
            _ => J::from(o.extract::<f64>()?),
        })
    } else if let Ok(f) = o.cast::<PyFloat>() {
        serde_json::Number::from_f64(f.value())
            .map(J::Number)
            .ok_or_else(|| PyValueError::new_err("NaN and infinity are not JSON"))
    } else if let Ok(d) = o.cast::<PyDict>() {
        let mut m = serde_json::Map::with_capacity(d.len());
        for (k, v) in d.iter() {
            let k = k
                .cast::<PyString>()
                .map_err(|_| PyTypeError::new_err("JSON object keys must be str"))?;
            m.insert(text(k).into_owned(), json(&v, depth + 1)?);
        }
        Ok(J::Object(m))
    } else if o.is_instance_of::<PyList>() || o.is_instance_of::<PyTuple>() {
        let items = o.try_iter()?.map(|x| json(&x?, depth + 1));
        Ok(J::Array(items.collect::<PyResult<_>>()?))
    } else {
        Err(PyTypeError::new_err(format!(
            "{} is not JSON",
            o.get_type().name()?
        )))
    }
}

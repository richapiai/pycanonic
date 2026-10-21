//! Python output: `PySink` builds `dict`s and `list`s for a `PyDoc` input with the GIL held, and
//! `to_py` converts the `Out` tree a bytes input was mapped to (detached) into the same objects.

use crate::pydoc::text;
use canonic_core::{Doc, Out, Value};
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyDict, PyFloat, PyInt, PyList, PyString};
use std::borrow::Cow;

pub fn value<'py>(py: Python<'py>, v: &Value<'_>) -> Bound<'py, PyAny> {
    match v {
        Value::Null | Value::Container => py.None().into_bound(py),
        Value::Bool(b) => PyBool::new(py, *b).to_owned().into_any(),
        Value::Int(i) => i
            .into_pyobject(py)
            .map_or_else(|e| match e {}, Bound::into_any),
        // CPython refuses `int(text)` past 4300 digits; JSON.parse reads that as a double, so do we.
        Value::BigInt(s) => py
            .get_type::<PyInt>()
            .call1((s.as_ref(),))
            .unwrap_or_else(|_| PyFloat::new(py, s.parse().unwrap_or(f64::NAN)).into_any()),
        Value::Float(f) => PyFloat::new(py, *f).into_any(),
        Value::Str(s) => PyString::new(py, s).into_any(),
    }
}

fn dict<'py, K: IntoPyObject<'py>>(
    py: Python<'py>,
    members: impl IntoIterator<Item = (K, Bound<'py, PyAny>)>,
) -> Bound<'py, PyAny> {
    let d = PyDict::new(py);
    for (k, v) in members {
        d.set_item(k, v).expect("setting a str key on a new dict");
    }
    d.into_any()
}

fn list<'py>(py: Python<'py>, items: Vec<Bound<'py, PyAny>>) -> Bound<'py, PyAny> {
    PyList::new(py, items)
        .expect("a list of Python objects")
        .into_any()
}

/// An `Out` tree as Python objects; canonical keys come from `names`, interned once per artifact.
pub fn to_py<'py>(py: Python<'py>, o: &Out, names: &[Py<PyString>]) -> Bound<'py, PyAny> {
    let v = |x: Value<'_>| value(py, &x);
    match o {
        Out::Null => v(Value::Null),
        Out::Bool(b) => v(Value::Bool(*b)),
        Out::Int(i) => v(Value::Int(*i)),
        Out::BigInt(s) => v(Value::BigInt(Cow::Borrowed(s))),
        Out::Float(f) => v(Value::Float(*f)),
        Out::Str(s) => v(Value::Str(Cow::Borrowed(s))),
        Out::List(xs) => list(py, xs.iter().map(|x| to_py(py, x, names)).collect()),
        Out::Obj(m) => dict(
            py,
            m.iter()
                .map(|(i, x)| (names[*i].bind(py), to_py(py, x, names))),
        ),
        Out::Map(m) => dict(py, m.iter().map(|(k, x)| (k.as_str(), to_py(py, x, names)))),
    }
}

pub struct PySink<'a, 'py> {
    pub py: Python<'py>,
    pub names: &'a [Py<PyString>],
}

impl<'py> canonic_core::Sink for PySink<'_, 'py> {
    type Out = Bound<'py, PyAny>;

    fn scalar(&mut self, v: Value<'_>) -> Self::Out {
        value(self.py, &v)
    }

    fn passthrough<D: Doc>(&mut self, d: &D) -> Self::Out {
        if let Some(o) = d.host().and_then(|h| h.downcast_ref::<Py<PyAny>>()) {
            let o = o.bind(self.py);
            // A str with a lone surrogate goes out with U+FFFD, as from bytes; anything else as is.
            if let Ok(s) = o.cast::<PyString>()
                && let Cow::Owned(t) = text(s)
            {
                return PyString::new(self.py, &t).into_any();
            }
            return o.clone();
        }
        if let Some(n) = d.len() {
            let items = (0..n)
                .map(|i| match d.item(i) {
                    Some(x) => self.passthrough(&x),
                    None => self.scalar(Value::Null),
                })
                .collect();
            return list(self.py, items);
        }
        if let Some(m) = d.members() {
            let members: Vec<_> = m
                .into_iter()
                .map(|(k, v)| (k.into_owned(), self.passthrough(&v)))
                .collect();
            return dict(self.py, members);
        }
        self.scalar(d.leaf())
    }

    fn list(&mut self, items: Vec<Self::Out>) -> Self::Out {
        list(self.py, items)
    }

    fn object(&mut self, members: Vec<(usize, Self::Out)>) -> Self::Out {
        let names = self.names;
        let py = self.py;
        dict(py, members.into_iter().map(|(i, v)| (names[i].bind(py), v)))
    }
}

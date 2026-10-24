//! Python callables as custom transforms, called with the GIL: `f(value)`, or `f(value, arg)` when
//! the spec gives an argument. A result that is not `str`, `int`, `float`, `bool` or `None`, or an
//! `Exception`, is null, which the executor counts. Any other `BaseException` (KeyboardInterrupt,
//! SystemExit) is held for the thread and raised from `map` once it returns. See `take_pending`.
//! A container transform can return a list or dict. A result that is not JSON is `None`.

use crate::pydoc::{json, leaf};
use crate::pysink::value;
use canonic_core::{Transform, Value};
use pyo3::exceptions::PyException;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};
use serde_json::Value as J;
use std::cell::RefCell;

thread_local! {
    /// A `BaseException` a transform raised during the current map call on this thread.
    static PENDING: RefCell<Option<PyErr>> = const { RefCell::new(None) };
}

/// The `BaseException` to raise from the map call that just returned, if a transform raised one.
pub fn take_pending() -> Option<PyErr> {
    PENDING.take()
}

pub struct PyTransform(pub Py<PyAny>);

/// JSON as Python objects, for a transform's argument and its container input.
pub fn json_to_py<'py>(py: Python<'py>, v: &J) -> Bound<'py, PyAny> {
    match v {
        J::Null => py.None().into_bound(py),
        J::Bool(b) => value(py, &Value::Bool(*b)),
        J::Number(n) => match (n.as_i64(), n.as_u64()) {
            (Some(i), _) => value(py, &Value::Int(i)),
            (None, Some(u)) => value(py, &Value::BigInt(u.to_string().into())),
            _ => value(py, &Value::Float(n.as_f64().unwrap_or(f64::NAN))),
        },
        J::String(s) => value(py, &Value::Str(s.into())),
        J::Array(a) => PyList::new(py, a.iter().map(|x| json_to_py(py, x)))
            .expect("a list of Python objects")
            .into_any(),
        J::Object(m) => {
            let d = PyDict::new(py);
            for (k, x) in m {
                d.set_item(k, json_to_py(py, x))
                    .expect("setting a str key on a new dict");
            }
            d.into_any()
        }
    }
}

impl PyTransform {
    /// `f(v)` or `f(v, arg)`, then `out` on its result. `None` for an exception.
    fn call<'py, T>(
        &self,
        py: Python<'py>,
        v: Bound<'py, PyAny>,
        arg: Option<&J>,
        out: impl FnOnce(&Bound<'py, PyAny>) -> Option<T>,
    ) -> Option<T> {
        // After a KeyboardInterrupt or the like, the rest of this map call calls no more Python.
        if PENDING.with_borrow(Option::is_some) {
            return None;
        }
        let f = self.0.bind(py);
        let r = match arg {
            None => f.call1((v,)),
            Some(a) => f.call1((v, json_to_py(py, a))),
        };
        match r {
            Ok(r) => out(&r),
            // An Exception is dropped here, which clears it: counted as an error, never printed.
            Err(e) if e.is_instance_of::<PyException>(py) => None,
            Err(e) => {
                PENDING.set(Some(e));
                None
            }
        }
    }

    fn scalar(&self, py: Python<'_>, v: Bound<'_, PyAny>, arg: Option<&J>) -> Value<'static> {
        let out = |r: &Bound<'_, PyAny>| match leaf(r) {
            Value::Container => None,
            v => Some(v.into_owned()),
        };
        self.call(py, v, arg, out).unwrap_or(Value::Null)
    }
}

impl Transform for PyTransform {
    fn apply<'a>(&self, v: Value<'a>, arg: Option<&J>) -> Value<'a> {
        Python::attach(|py| self.scalar(py, value(py, &v), arg))
    }

    fn apply_container(&self, v: &J, arg: Option<&J>) -> Value<'static> {
        Python::attach(|py| self.scalar(py, json_to_py(py, v), arg))
    }

    fn apply_container_out(&self, v: &J, arg: Option<&J>) -> Option<J> {
        // A result that is not JSON (a set, a NaN, a non-str key) raises in `json`, a rejection.
        Python::attach(|py| self.call(py, json_to_py(py, v), arg, |r| json(r, 0).ok()))
    }
}

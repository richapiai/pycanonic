//! Python callables as custom transforms, called `(value, arg)` with the GIL. A result that isn't
//! `str`, `int`, `float`, `bool` or `None`, or an exception, is null, which the executor counts.

use crate::pydoc::leaf;
use crate::pysink::value;
use canonic_core::{Transform, Value};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};
use serde_json::Value as J;

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
    fn call(&self, py: Python<'_>, v: Bound<'_, PyAny>, arg: Option<&J>) -> Value<'static> {
        let arg = arg.map_or_else(|| py.None().into_bound(py), |a| json_to_py(py, a));
        // An exception is dropped here, which clears it: counted as an error, never printed.
        match self.0.bind(py).call1((v, arg)) {
            Ok(r) => match leaf(&r) {
                Value::Container => Value::Null,
                v => v.into_owned(),
            },
            Err(_) => Value::Null,
        }
    }
}

impl Transform for PyTransform {
    fn apply<'a>(&self, v: Value<'a>, arg: Option<&J>) -> Value<'a> {
        Python::attach(|py| self.call(py, value(py, &v), arg))
    }

    fn apply_container(&self, v: &J, arg: Option<&J>) -> Value<'static> {
        Python::attach(|py| self.call(py, json_to_py(py, v), arg))
    }
}

mod artifact;
mod drift;
mod pydoc;
mod pysink;
mod transforms;
mod view;

use pyo3::prelude::*;

#[pymodule]
mod _core {
    use crate::artifact::{custom, value_error};
    use crate::pydoc::{leaf, text};
    use canonic_core::{ArgError, Issue, Plan, Semantic, Transform, Value};
    use pyo3::exceptions::{PyTypeError, PyValueError};
    use pyo3::prelude::*;
    use pyo3::types::{PyBool, PyBytes, PyDict, PyFloat, PyInt, PyList, PyString, PyTuple};
    use serde_json::Value as J;

    #[pymodule_export]
    use crate::artifact::{Artifact, ExtractStream, Stats};
    #[pymodule_export]
    use crate::drift::{FillMonitor, fingerprint, shape};
    #[pymodule_export]
    use crate::view::View;

    #[pyfunction]
    fn version() -> &'static str {
        canonic_core::version()
    }

    /// Deep enough for any spec or schema; a cyclic object fails instead of overflowing the stack.
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

    /// A spec or schema given as a `dict`, or as JSON text (`str` or `bytes`, read by `json.loads`).
    fn document(o: &Bound<'_, PyAny>) -> PyResult<J> {
        if o.is_instance_of::<PyString>() || o.is_instance_of::<PyBytes>() {
            let parsed = o.py().import("json")?.call_method1("loads", (o,))?;
            return json(&parsed, 0);
        }
        json(o, 0)
    }

    #[pyfunction]
    #[pyo3(signature = (spec, canonical, transforms=None))]
    fn compile(
        py: Python<'_>,
        spec: &Bound<'_, PyAny>,
        canonical: &Bound<'_, PyAny>,
        transforms: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Artifact> {
        let custom = custom(transforms)?;
        // In the dict's order, so the plan (and `plan_json`) is the same on every run.
        let names: Vec<String> = match transforms {
            Some(d) => d.keys().extract()?,
            None => Vec::new(),
        };
        let plan = canonic_core::compile(&document(spec)?, &document(canonical)?, &names)
            .map_err(value_error)?;
        Artifact::new(py, plan, custom)
    }

    #[pyfunction]
    #[pyo3(signature = (plan_json, transforms=None))]
    fn load_plan(
        py: Python<'_>,
        plan_json: &str,
        transforms: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Artifact> {
        let plan: Plan = serde_json::from_str(plan_json)
            .map_err(|e| PyValueError::new_err(format!("invalid plan: {e}")))?;
        Artifact::new(py, plan, custom(transforms)?)
    }

    fn issues(v: Vec<Issue>) -> Vec<(String, String)> {
        v.into_iter().map(|i| (i.at, i.message)).collect()
    }

    #[pyfunction]
    fn validate_spec(spec: &Bound<'_, PyAny>) -> PyResult<Vec<(String, String)>> {
        Ok(issues(canonic_core::validate_spec(&document(spec)?)))
    }

    #[pyfunction]
    fn validate_canonical(schema: &Bound<'_, PyAny>) -> PyResult<Vec<(String, String)>> {
        Ok(issues(canonic_core::validate_canonical(&document(schema)?)))
    }

    /// A default transform by name; a `list` or `tuple` value is the several-path `from` input.
    #[pyfunction]
    #[pyo3(signature = (name, value, arg=None))]
    fn default_transform<'py>(
        name: &str,
        value: &Bound<'py, PyAny>,
        arg: Option<&Bound<'py, PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let arg = arg.map(|a| json(a, 0)).transpose()?;
        let b = canonic_core::builtin(name, arg.as_ref()).map_err(|e| {
            PyValueError::new_err(match e {
                ArgError::UnknownTransform => format!("unknown transform {:?}", name),
                ArgError::InvalidArgument => format!("invalid argument for transform {:?}", name),
            })
        })?;
        let py = value.py();
        let out = if value.is_instance_of::<PyList>() || value.is_instance_of::<PyTuple>() {
            let items: Vec<Bound<'py, PyAny>> = value.try_iter()?.collect::<PyResult<_>>()?;
            let vals: Vec<Value<'_>> = items.iter().map(|x| leaf(x)).collect();
            b.apply_list(&vals).into_owned()
        } else {
            b.apply(leaf(value), None).into_owned()
        };
        Ok(crate::pysink::value(py, &out))
    }

    /// The routing validator for `semantic`: `value` when valid, `None` otherwise.
    #[pyfunction]
    fn validator<'py>(semantic: &str, value: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
        let s: Semantic = serde_json::from_value(J::from(semantic))
            .map_err(|_| PyValueError::new_err(format!("unknown semantic {semantic:?}")))?;
        let ok = !canonic_core::validate(s, leaf(value)).is_null();
        Ok(if ok {
            value.clone()
        } else {
            value.py().None().into_bound(value.py())
        })
    }
}

mod artifact;
mod drift;
mod pydoc;
mod pysink;
mod shadow;
mod transforms;
mod view;

use pyo3::prelude::*;

#[pymodule]
mod _core {
    use crate::artifact::{custom, value_error};
    use crate::pydoc::{json, leaf};
    use canonic_core::{ArgError, Issue, Plan, Semantic, Transform, Value};
    use pyo3::exceptions::PyValueError;
    use pyo3::prelude::*;
    use pyo3::types::{PyBytes, PyDict, PyList, PyString, PyTuple};
    use serde_json::Value as J;

    #[pymodule_export]
    use crate::artifact::{Artifact, ExtractStream, Rejected, Stats};
    #[pymodule_export]
    use crate::drift::{FillMonitor, fingerprint, shape};
    #[pymodule_export]
    use crate::shadow::{ShadowDiff, merge_gate};
    #[pymodule_export]
    use crate::view::View;

    #[pymodule_init]
    fn init(m: &Bound<'_, PyModule>) -> PyResult<()> {
        crate::artifact::add_reason(m.py())
    }

    #[pyfunction]
    fn version() -> &'static str {
        canonic_core::version()
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

    /// A default transform by name. A `list` or `tuple` value is the several-path `from` input.
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

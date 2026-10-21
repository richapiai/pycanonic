//! The `Artifact` class: a compiled plan with its stats, mapping Python objects, `bytes` or `str`.

use crate::pydoc::{PyDoc, text};
use crate::pysink::{PySink, to_py};
use crate::transforms::{PyTransform, take_pending};
use canonic_core::{CompileError, Plan, Status, Transform};
use pyo3::exceptions::{PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict, PyString};
use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;

#[pyclass(frozen, module = "pycanonic._core")]
pub struct Artifact {
    inner: canonic_core::Artifact,
    /// Interned path keys, indexed by `Key::tag`.
    keys: Vec<Py<PyString>>,
    /// Interned canonical field names, indexed like `Plan::names`.
    names: Vec<Py<PyString>>,
}

/// Python callables by transform name.
pub fn custom(
    transforms: Option<&Bound<'_, PyDict>>,
) -> PyResult<HashMap<String, Arc<dyn Transform>>> {
    let mut map: HashMap<String, Arc<dyn Transform>> = HashMap::new();
    for (k, f) in transforms.into_iter().flat_map(|d| d.iter()) {
        let name: String = k.extract()?;
        if !f.is_callable() {
            return Err(PyTypeError::new_err(format!(
                "transform {name:?} is not callable"
            )));
        }
        map.insert(name, Arc::new(PyTransform(f.unbind())));
    }
    Ok(map)
}

pub fn value_error(e: CompileError) -> PyErr {
    PyValueError::new_err(e.message)
}

/// The UTF-8 bytes of a `bytes` or `str` input; `None` for any other object.
fn body_of<'s>(src: &'s Bound<'_, PyAny>) -> Option<Cow<'s, [u8]>> {
    if let Ok(b) = src.cast::<PyBytes>() {
        Some(Cow::Borrowed(b.as_bytes()))
    } else if let Ok(s) = src.cast::<PyString>() {
        Some(match text(s) {
            Cow::Borrowed(t) => Cow::Borrowed(t.as_bytes()),
            Cow::Owned(t) => Cow::Owned(t.into_bytes()),
        })
    } else {
        None
    }
}

impl Artifact {
    pub fn new(
        py: Python<'_>,
        plan: Plan,
        custom: HashMap<String, Arc<dyn Transform>>,
    ) -> PyResult<Artifact> {
        let inner = canonic_core::Artifact::new(plan, custom).map_err(value_error)?;
        let intern = |s: &String| PyString::intern(py, s).unbind();
        Ok(Artifact {
            keys: inner.keys().iter().map(intern).collect(),
            names: inner.plan.names.iter().map(intern).collect(),
            inner,
        })
    }
}

#[pymethods]
impl Artifact {
    #[getter]
    fn provider(&self) -> &str {
        &self.inner.plan.meta.provider
    }

    #[getter]
    fn endpoint(&self) -> &str {
        &self.inner.plan.meta.endpoint
    }

    #[getter]
    fn version(&self) -> u32 {
        self.inner.plan.meta.version
    }

    #[getter]
    fn status(&self) -> &'static str {
        match self.inner.plan.meta.status {
            Status::Draft => "draft",
            Status::Shadow => "shadow",
            Status::Promoted => "promoted",
            Status::Deprecated => "deprecated",
        }
    }

    #[getter]
    fn routing(&self) -> Vec<String> {
        self.inner
            .plan
            .routing
            .iter()
            .map(|r| r.name.clone())
            .collect()
    }

    #[getter]
    fn source(&self) -> String {
        self.inner.plan.source()
    }

    /// `bytes` or `str` is parsed and mapped with the GIL released; any other object is read as is.
    fn map<'py>(&self, py: Python<'py>, src: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
        let out = if let Some(body) = body_of(src) {
            let out = py.detach(|| self.inner.map_bytes(&body));
            to_py(py, &out, &self.names)
        } else {
            let doc = PyDoc {
                obj: src.clone(),
                keys: &self.keys,
            };
            let mut sink = PySink {
                py,
                names: &self.names,
            };
            self.inner.map_with(Some(&doc), &mut sink)
        };
        take_pending().map_or(Ok(out), Err)
    }

    /// The canonical object as JSON text, without building Python objects; runs with the GIL released.
    fn map_json<'py>(
        &self,
        py: Python<'py>,
        raw: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyBytes>> {
        let body =
            body_of(raw).ok_or_else(|| PyTypeError::new_err("map_json takes bytes or str"))?;
        let out = py.detach(|| self.inner.map_json(&body));
        take_pending().map_or(Ok(PyBytes::new(py, &out)), Err)
    }

    fn stats_snapshot<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let s = self.inner.stats.snapshot();
        let fields = PyDict::new(py);
        for (name, f) in s.fields {
            let d = PyDict::new(py);
            d.set_item("fill", f.fill)?;
            d.set_item("errors", f.errors)?;
            if let Some(t) = f.total {
                d.set_item("total", t)?;
            }
            fields.set_item(name, d)?;
        }
        let out = PyDict::new(py);
        out.set_item("responses", s.responses)?;
        out.set_item("fields", fields)?;
        Ok(out)
    }

    fn stats_reset(&self) {
        self.inner.stats.reset();
    }

    /// The artifact's field counters, as `snapshot()` and `reset()`.
    #[getter]
    fn stats(slf: Py<Self>) -> Stats {
        Stats(slf)
    }

    fn plan_json(&self) -> String {
        serde_json::to_string(&*self.inner.plan).expect("a plan serializes")
    }
}

/// `artifact.stats`: a view of the artifact's counters, not a copy.
#[pyclass(frozen, module = "pycanonic._core")]
pub struct Stats(Py<Artifact>);

#[pymethods]
impl Stats {
    /// `{"responses": n, "fields": {spec_field: {"fill", "errors", "total"?}}}`, as `stats_snapshot()`.
    fn snapshot<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        self.0.get().stats_snapshot(py)
    }

    /// Zeroes every counter.
    fn reset(&self) {
        self.0.get().stats_reset();
    }
}

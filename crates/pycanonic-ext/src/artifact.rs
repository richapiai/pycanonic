//! The `Artifact` class: a compiled plan with its stats, mapping Python objects or JSON text
//! (`bytes`, `str`, `bytearray`, `memoryview`).

use crate::pydoc::{PyDoc, text};
use crate::pysink::{PySink, value};
use crate::transforms::{PyTransform, take_pending};
use crate::view::{Level, View, levels};
use canonic_core::{CompileError, Plan, Status, Transform, Value};
use pyo3::exceptions::{PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{
    PyByteArray, PyBytes, PyDict, PyFloat, PyInt, PyList, PyMapping, PyMemoryView, PyString,
    PyTuple,
};
use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

#[pyclass(frozen, module = "pycanonic._core")]
pub struct Artifact {
    pub(crate) inner: canonic_core::Artifact,
    /// Interned path keys, indexed by `Key::tag`.
    pub(crate) keys: Vec<Py<PyString>>,
    /// Interned canonical field names, indexed like `Plan::names`.
    pub(crate) names: Vec<Py<PyString>>,
    /// The canonical object levels a `View` walks. See `view::levels`.
    pub(crate) levels: Vec<Level>,
    /// Interned routing field names, in `plan.routing` order: the keys of `extract`'s dict.
    pub(crate) routing: Vec<Py<PyString>>,
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

/// The JSON text of a `bytes`, `str`, `bytearray` or `memoryview` input. `None` for any other
/// object. A `bytearray` or `memoryview` is copied: another thread can change it while the GIL is
/// released.
pub(crate) fn body_of<'s>(src: &'s Bound<'_, PyAny>) -> PyResult<Option<Cow<'s, [u8]>>> {
    // A parsed dict is the common non-text input: one type compare instead of four checks.
    Ok(if src.is_exact_instance_of::<PyDict>() {
        None
    } else if let Ok(b) = src.cast::<PyBytes>() {
        Some(Cow::Borrowed(b.as_bytes()))
    } else if let Ok(s) = src.cast::<PyString>() {
        Some(match text(s) {
            Cow::Borrowed(t) => Cow::Borrowed(t.as_bytes()),
            Cow::Owned(t) => Cow::Owned(t.into_bytes()),
        })
    } else if let Ok(b) = src.cast::<PyByteArray>() {
        Some(Cow::Owned(b.to_vec()))
    } else if src.is_instance_of::<PyMemoryView>() {
        let b = src.call_method0("tobytes")?;
        Some(Cow::Owned(b.cast::<PyBytes>()?.as_bytes().to_vec()))
    } else {
        None
    })
}

/// The root `map` and `view` read as a Python object tree: what `json.loads` returns (`dict`,
/// `list`, `None`, `bool`, `int`, `float`) or a `tuple` as is, any other `Mapping` copied one level
/// into a `dict`. Anything else is a `TypeError`, not an all-`None` result.
pub(crate) fn object_root<'py>(src: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    if src.is_instance_of::<PyDict>()
        || src.is_instance_of::<PyList>()
        || src.is_instance_of::<PyTuple>()
        || src.is_none()
        || src.is_instance_of::<PyInt>()
        || src.is_instance_of::<PyFloat>()
    {
        Ok(src.clone())
    } else if let Ok(m) = src.cast::<PyMapping>() {
        let d = PyDict::new(src.py());
        d.update(m)?;
        Ok(d.into_any())
    } else {
        Err(PyTypeError::new_err(format!(
            "expected JSON text (bytes, str, bytearray, memoryview) or a parsed value (dict, \
             list, tuple, Mapping, None, bool, int, float), not {}",
            src.get_type().name()?
        )))
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
            routing: inner.plan.routing.iter().map(|r| intern(&r.name)).collect(),
            levels: levels(py, &inner.plan.names, &inner.plan.fields),
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

    /// JSON text (`bytes`, `str`, `bytearray`, `memoryview`) is parsed with the GIL released, then
    /// mapped straight into Python objects. A `dict`, `list`, `tuple` or `Mapping` is read as is.
    fn map<'py>(&self, py: Python<'py>, src: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
        let out = if let Some(body) = body_of(src)? {
            let node = py.detach(|| self.inner.parse(&body));
            let mut sink = PySink {
                py,
                names: &self.names,
            };
            self.inner.map_with(node.as_ref().as_ref(), &mut sink)
        } else {
            let doc = PyDoc {
                obj: object_root(src)?,
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

    /// The canonical object as JSON text, without building Python objects. Runs with the GIL released.
    fn map_json<'py>(
        &self,
        py: Python<'py>,
        raw: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyBytes>> {
        let body = body_of(raw)?.ok_or_else(|| {
            PyTypeError::new_err("map_json takes bytes, str, bytearray or memoryview")
        })?;
        let out = py.detach(|| self.inner.map_json(&body));
        take_pending().map_or(Ok(PyBytes::new(py, &out)), Err)
    }

    /// The routing fields read straight from JSON text, keyed by spec field. GIL released.
    fn extract<'py>(
        &self,
        py: Python<'py>,
        raw: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyDict>> {
        let body = body_of(raw)?.ok_or_else(|| {
            PyTypeError::new_err("extract takes bytes, str, bytearray or memoryview")
        })?;
        let out = py.detach(|| self.inner.extract(&body));
        take_pending().map_or_else(|| routing_dict(py, &self.routing, &out), Err)
    }

    /// A lazy canonical object over `src` (as `map` takes it): each field is mapped on first read.
    fn view(slf: Py<Self>, py: Python<'_>, src: &Bound<'_, PyAny>) -> PyResult<View> {
        View::new(py, slf, src)
    }

    /// A fresh incremental reader of the routing fields. One per response.
    fn stream(slf: Py<Self>) -> ExtractStream {
        let inner = slf.get().inner.stream();
        ExtractStream {
            artifact: slf,
            inner: Mutex::new(Some(inner)),
        }
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

/// Routing values as `{spec_field: value}`, in `plan.routing` order (`names`, interned). Not found
/// and rejected are `None`.
fn routing_dict<'py>(
    py: Python<'py>,
    names: &[Py<PyString>],
    vals: &[Option<Value<'static>>],
) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new(py);
    for (name, v) in names.iter().zip(vals) {
        d.set_item(name.bind(py), v.as_ref().map(|v| value(py, v)))?;
    }
    Ok(d)
}

/// `artifact.stream()`: `push` chunks, then `result()` once.
#[pyclass(frozen, module = "pycanonic._core")]
pub struct ExtractStream {
    artifact: Py<Artifact>,
    /// Taken by `result()`, which consumes the core stream.
    inner: Mutex<Option<canonic_core::ExtractStream>>,
}

fn finished() -> PyErr {
    PyValueError::new_err("stream already finished")
}

#[pymethods]
impl ExtractStream {
    /// Feeds `bytes`, `str`, `bytearray` or `memoryview`. `True` once every routing field is
    /// available. GIL released.
    fn push(&self, py: Python<'_>, chunk: &Bound<'_, PyAny>) -> PyResult<bool> {
        let body = body_of(chunk)?.ok_or_else(|| {
            PyTypeError::new_err("push takes bytes, str, bytearray or memoryview")
        })?;
        let done = py.detach(|| {
            let mut g = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
            g.as_mut().map(|s| s.push(&body))
        });
        let done = done.ok_or_else(finished)?;
        take_pending().map_or(Ok(done), Err)
    }

    /// Ends the input and returns the routing values. A second call raises `ValueError`.
    fn result<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let s = self
            .inner
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
            .ok_or_else(finished)?;
        let out = py.detach(|| s.result());
        take_pending().map_or_else(|| routing_dict(py, &self.artifact.get().routing, &out), Err)
    }
}

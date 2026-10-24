//! `shape`, `fingerprint` and `FillMonitor` from the core, over Python values and stats dicts.

use crate::pydoc::PyDoc;
use canonic_core::{AlertKind, FieldStats, Filter, StatsSnapshot};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict};

/// Sorted key paths of a parsed response, or of JSON text given as `bytes`. Only a `dict` and a
/// `list`/`tuple` have members. Any other value (a `str`, a `set`) is a scalar, as TypeScript sees
/// it. A Python object deeper than 512 levels, with more than 1,000,000 containers or 64 MiB of
/// distinct paths (a cyclic one, or one that shares its children) raises `ValueError`.
#[pyfunction]
pub fn shape(value: &Bound<'_, PyAny>) -> PyResult<Vec<String>> {
    if let Ok(b) = value.cast::<PyBytes>() {
        let node = canonic_core::parse(b.as_bytes(), &Filter::all())
            .map_err(|e| PyValueError::new_err(format!("invalid JSON: {e:?}")))?;
        return Ok(canonic_core::shape(&&node));
    }
    let doc = PyDoc {
        obj: value.clone(),
        keys: &[],
    };
    canonic_core::shape_bounded(&doc)
        .ok_or_else(|| PyValueError::new_err("shape: object too large or cyclic"))
}

/// FNV-1a over `shape(value)`, as 8 hex digits. Equal to canonic's for the same document.
/// Raises `ValueError` where `shape` does.
#[pyfunction]
pub fn fingerprint(value: &Bound<'_, PyAny>) -> PyResult<String> {
    Ok(canonic_core::hash_shape(&shape(value)?))
}

#[pyclass(module = "pycanonic._core")]
pub struct FillMonitor(canonic_core::FillMonitor);

#[pymethods]
impl FillMonitor {
    #[new]
    #[pyo3(signature = (*, min_responses=200, history=24, sigmas=4.0, min_drop=0.25, min_rise=0.01))]
    fn new(min_responses: u64, history: usize, sigmas: f64, min_drop: f64, min_rise: f64) -> Self {
        FillMonitor(canonic_core::FillMonitor::new(
            canonic_core::FillMonitorOptions {
                min_responses,
                history,
                sigmas,
                min_drop,
                min_rise,
            },
        ))
    }

    /// `window` as `stats_snapshot()` returns it, any mapping. Returns alerts as dicts of
    /// `FieldAlert`'s fields.
    fn check<'py>(
        &mut self,
        py: Python<'py>,
        window: &Bound<'py, PyAny>,
    ) -> PyResult<Vec<Bound<'py, PyDict>>> {
        let mut fields = Vec::new();
        for kv in window
            .get_item("fields")?
            .call_method0("items")?
            .try_iter()?
        {
            let (name, s): (String, Bound<'py, PyAny>) = kv?.extract()?;
            let total = s.call_method1("get", ("total",))?;
            let stats = FieldStats {
                fill: s.get_item("fill")?.extract()?,
                errors: s.get_item("errors")?.extract()?,
                total: (!total.is_none()).then(|| total.extract()).transpose()?,
            };
            fields.push((name, stats));
        }
        let snap = StatsSnapshot {
            responses: window.get_item("responses")?.extract()?,
            // The monitor reads fill and errors only.
            rejected: 0,
            fields,
        };
        self.0
            .check(&snap)
            .into_iter()
            .map(|a| {
                let d = PyDict::new(py);
                d.set_item("field", a.field)?;
                let kind = match a.kind {
                    AlertKind::Fill => "fill",
                    AlertKind::Errors => "errors",
                };
                d.set_item("kind", kind)?;
                d.set_item("rate", a.rate)?;
                d.set_item("baseline", a.baseline)?;
                d.set_item("responses", a.responses)?;
                Ok(d)
            })
            .collect()
    }
}

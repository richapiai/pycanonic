//! `shape`, `fingerprint` and `FillMonitor` from the core, over Python values and stats dicts.

use crate::pydoc::PyDoc;
use canonic_core::json::Node;
use canonic_core::{AlertKind, FieldStats, Filter, StatsSnapshot};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict};

/// `bytes` parsed by the core parser as JSON text; anything else walked as the Python object it
/// is, where only a `dict` and a `list`/`tuple` have members and any other value (a `str`, a `set`)
/// is a scalar, as TypeScript sees it. The core walk bounds a cyclic object.
fn with_doc<T>(
    value: &Bound<'_, PyAny>,
    parsed: fn(&&Node<'_>) -> T,
    object: fn(&PyDoc<'_, '_>) -> T,
) -> PyResult<T> {
    let Ok(b) = value.cast::<PyBytes>() else {
        return Ok(object(&PyDoc {
            obj: value.clone(),
            keys: &[],
        }));
    };
    let all = Filter {
        all: true,
        ..Filter::default()
    };
    let node = canonic_core::parse(b.as_bytes(), &all)
        .map_err(|e| PyValueError::new_err(format!("invalid JSON: {e:?}")))?;
    Ok(parsed(&&node))
}

/// Sorted key paths of a parsed response, or of JSON text given as `bytes`.
#[pyfunction]
pub fn shape(value: &Bound<'_, PyAny>) -> PyResult<Vec<String>> {
    with_doc(
        value,
        |d| canonic_core::shape(d),
        |d| canonic_core::shape(d),
    )
}

/// FNV-1a over `shape(value)`, as 8 hex digits; equal to canonic's for the same document.
#[pyfunction]
pub fn fingerprint(value: &Bound<'_, PyAny>) -> PyResult<String> {
    with_doc(
        value,
        |d| canonic_core::fingerprint(d),
        |d| canonic_core::fingerprint(d),
    )
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

    /// `window` as `stats_snapshot()` returns it; alerts as dicts of `FieldAlert`'s fields.
    fn check<'py>(
        &mut self,
        py: Python<'py>,
        window: &Bound<'py, PyDict>,
    ) -> PyResult<Vec<Bound<'py, PyDict>>> {
        let item = |d: &Bound<'py, PyDict>, k: &str| {
            d.get_item(k)?
                .ok_or_else(|| PyValueError::new_err(format!("window has no {k:?}")))
        };
        let mut fields = Vec::new();
        for (name, s) in item(window, "fields")?.cast_into::<PyDict>()?.iter() {
            let s = s.cast_into::<PyDict>()?;
            let stats = FieldStats {
                fill: item(&s, "fill")?.extract()?,
                errors: item(&s, "errors")?.extract()?,
                total: s
                    .get_item("total")?
                    .filter(|t| !t.is_none())
                    .map(|t| t.extract())
                    .transpose()?,
            };
            fields.push((name.extract()?, stats));
        }
        let snap = StatsSnapshot {
            responses: item(window, "responses")?.extract()?,
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

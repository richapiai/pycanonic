//! `ShadowDiff` and `merge_gate` from the core, over parsed Python dicts and report dicts.

use crate::pydoc::PyDoc;
use canonic_core::{AlertKind, FieldAlert, FieldDiff, GateOptions, Out, ShadowReport};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyMapping};

fn doc<'a, 'py>(obj: &Bound<'py, PyAny>) -> PyDoc<'a, 'py> {
    PyDoc {
        obj: obj.clone(),
        keys: &[],
    }
}

fn too_large(what: &str) -> PyErr {
    PyValueError::new_err(format!("{what}: object too large or cyclic"))
}

#[pyclass(module = "pycanonic._core")]
pub struct ShadowDiff(canonic_core::ShadowDiff);

#[pymethods]
impl ShadowDiff {
    #[new]
    fn new(routing: Vec<String>) -> Self {
        ShadowDiff(canonic_core::ShadowDiff::new(routing))
    }

    /// One response: the promoted and candidate `map()` output for the same body. A value deeper
    /// than 512 levels or with more than 1,000,000 values (a cyclic one) raises `ValueError` and
    /// records nothing.
    fn add(&mut self, promoted: &Bound<'_, PyAny>, candidate: &Bound<'_, PyAny>) -> PyResult<()> {
        if self.0.add_bounded(&doc(promoted), &doc(candidate)) {
            Ok(())
        } else {
            Err(too_large("ShadowDiff.add"))
        }
    }

    fn snapshot<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let r = self.0.snapshot();
        let fields = PyDict::new(py);
        for (path, d) in r.fields {
            let f = PyDict::new(py);
            f.set_item("compared", d.compared)?;
            f.set_item("same", d.same)?;
            f.set_item("improvements", d.improvements)?;
            f.set_item("regressions", d.regressions)?;
            f.set_item("conflicts", d.conflicts)?;
            fields.set_item(path, f)?;
        }
        let out = PyDict::new(py);
        out.set_item("responses", r.responses)?;
        out.set_item("routing", r.routing)?;
        out.set_item("fields", fields)?;
        Ok(out)
    }
}

fn report(r: &Bound<'_, PyAny>) -> PyResult<ShadowReport> {
    let mut fields = Vec::new();
    for kv in r.get_item("fields")?.call_method0("items")?.try_iter()? {
        let (path, d): (String, Bound<'_, PyAny>) = kv?.extract()?;
        let n = |k: &str| d.get_item(k)?.extract::<u64>();
        let diff = FieldDiff {
            compared: n("compared")?,
            same: n("same")?,
            improvements: n("improvements")?,
            regressions: n("regressions")?,
            conflicts: n("conflicts")?,
        };
        fields.push((path, diff));
    }
    Ok(ShadowReport {
        responses: r.get_item("responses")?.extract()?,
        routing: r.get_item("routing")?.extract()?,
        fields,
    })
}

/// A `(candidate, expected)` pair, or a mapping with those keys as in TypeScript.
fn known(k: &Bound<'_, PyAny>) -> PyResult<(Out, Out)> {
    let (c, e): (Bound<'_, PyAny>, Bound<'_, PyAny>) = if k.cast::<PyMapping>().is_ok() {
        (k.get_item("candidate")?, k.get_item("expected")?)
    } else {
        let v: Vec<Bound<'_, PyAny>> = k.try_iter()?.collect::<PyResult<_>>()?;
        let [c, e] = <[_; 2]>::try_from(v)
            .map_err(|_| PyValueError::new_err("known: each entry is (candidate, expected)"))?;
        (c, e)
    };
    let out = |v: &Bound<'_, PyAny>| {
        canonic_core::to_out_bounded(&doc(v)).ok_or_else(|| too_large("merge_gate"))
    };
    Ok((out(&c)?, out(&e)?))
}

/// A `FieldAlert` dataclass, or a mapping with its keys.
fn alert(a: &Bound<'_, PyAny>) -> PyResult<FieldAlert> {
    let mapping = a.cast::<PyMapping>().is_ok();
    let get = |k: &str| if mapping { a.get_item(k) } else { a.getattr(k) };
    let kind = match get("kind")?.extract::<String>()?.as_str() {
        "fill" => AlertKind::Fill,
        "errors" => AlertKind::Errors,
        other => {
            return Err(PyValueError::new_err(format!(
                "unknown alert kind {other:?}"
            )));
        }
    };
    Ok(FieldAlert {
        field: get("field")?.extract()?,
        kind,
        rate: get("rate")?.extract()?,
        baseline: get("baseline")?.extract()?,
        responses: get("responses")?.extract()?,
    })
}

/// The auto-merge rule over a `ShadowDiff.snapshot()` report; `{"merge": bool, "reasons": [...]}`.
#[pyfunction]
#[pyo3(signature = (report, *, max_change=0.01, known=None, fill_alerts=None))]
pub fn merge_gate<'py>(
    py: Python<'py>,
    report: &Bound<'py, PyAny>,
    max_change: f64,
    known: Option<&Bound<'py, PyAny>>,
    fill_alerts: Option<&Bound<'py, PyAny>>,
) -> PyResult<Bound<'py, PyDict>> {
    let each = |o: Option<&Bound<'py, PyAny>>| -> PyResult<Vec<Bound<'py, PyAny>>> {
        o.map_or(Ok(Vec::new()), |o| o.try_iter()?.collect())
    };
    let opts = GateOptions {
        max_change,
        known: each(known)?
            .iter()
            .map(self::known)
            .collect::<PyResult<_>>()?,
        fill_alerts: each(fill_alerts)?
            .iter()
            .map(alert)
            .collect::<PyResult<_>>()?,
    };
    let r = canonic_core::merge_gate(&self::report(report)?, &opts);
    let out = PyDict::new(py);
    out.set_item("merge", r.merge)?;
    out.set_item("reasons", r.reasons)?;
    Ok(out)
}

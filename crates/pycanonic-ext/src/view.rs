//! `artifact.view(src)`: each canonical field is mapped on its first read and kept, nested objects
//! are views, and nothing counts stats; codegen.ts's View classes. `to_dict()` maps the whole level.

use crate::artifact::{Artifact, body_of};
use crate::pydoc::PyDoc;
use crate::pysink::PySink;
use crate::transforms::take_pending;
use canonic_core::json::Node;
use canonic_core::{Field, Named, Plan};
use pyo3::intern;
use pyo3::prelude::*;
use pyo3::sync::PyOnceLock;
use pyo3::types::{PyIterator, PyList, PyString};
use std::sync::Arc;

type MaybeNode<'a> = Option<Node<'a>>;

self_cell::self_cell!(
    /// A body and its parse tree, which borrows from it; `None` when it doesn't parse.
    struct Parsed {
        owner: Vec<u8>,
        #[covariant]
        dependent: MaybeNode,
    }
);

/// The document every view of one `artifact.view` call reads.
enum Source {
    /// `bytes` or `str`, copied and parsed once with the plan's filter.
    Parsed(Parsed),
    /// Any other object, read as `map` reads it.
    Object(Py<PyAny>),
}

#[pyclass(frozen, module = "pycanonic._core")]
pub struct View {
    artifact: Py<Artifact>,
    doc: Arc<Source>,
    /// Field indices from the root to this view's object; empty for the root.
    path: Vec<usize>,
    /// One slot per field at this level: the mapped value, or the nested `View`.
    cache: Box<[PyOnceLock<Py<PyAny>>]>,
}

/// The fields of the object at `path`; a view's path only ever names objects.
fn level<'p>(plan: &'p Plan, path: &[usize]) -> &'p [Named<Field>] {
    path.iter().fold(&plan.fields, |fs, &i| match &fs[i].value {
        Field::Object(c) => c,
        _ => unreachable!("a view's path names an object"),
    })
}

fn slots(n: usize) -> Box<[PyOnceLock<Py<PyAny>>]> {
    (0..n).map(|_| PyOnceLock::new()).collect()
}

impl View {
    pub fn new(artifact: Py<Artifact>, src: &Bound<'_, PyAny>) -> View {
        let doc = match body_of(src) {
            // parsed with the GIL held, cheap for API-sized bodies; detach if large ones show up.
            Some(body) => Source::Parsed(Parsed::new(body.into_owned(), |b| {
                artifact.get().inner.parse(b)
            })),
            None => Source::Object(src.clone().unbind()),
        };
        let n = artifact.get().inner.plan.fields.len();
        View {
            artifact,
            doc: Arc::new(doc),
            path: Vec::new(),
            cache: slots(n),
        }
    }

    /// The field (or whole object, for this view's own path) at `path`, as `map` builds it.
    fn run<'py>(&self, py: Python<'py>, path: &[usize]) -> PyResult<Bound<'py, PyAny>> {
        let a = self.artifact.get();
        let mut sink = PySink {
            py,
            names: &a.names,
        };
        let out = match &*self.doc {
            Source::Parsed(p) => {
                let node = p.borrow_dependent().as_ref();
                a.inner.map_field(path, node.as_ref(), &mut sink)
            }
            Source::Object(o) => {
                let doc = PyDoc {
                    obj: o.bind(py).clone(),
                    keys: &a.keys,
                };
                a.inner.map_field(path, Some(&doc), &mut sink)
            }
        };
        let out = out.expect("a view's path names a field");
        take_pending().map_or(Ok(out), Err)
    }

    /// Field `i` of this level, mapped (or made a nested view) on first read.
    fn field(&self, py: Python<'_>, fields: &[Named<Field>], i: usize) -> PyResult<Py<PyAny>> {
        if let Some(v) = self.cache[i].get(py) {
            return Ok(v.clone_ref(py));
        }
        let path = || [self.path.as_slice(), &[i]].concat();
        let v = match &fields[i].value {
            Field::Object(c) => Py::new(
                py,
                View {
                    artifact: self.artifact.clone_ref(py),
                    doc: self.doc.clone(),
                    path: path(),
                    cache: slots(c.len()),
                },
            )?
            .into_any(),
            // The root's fields, most reads, skip allocating a path.
            _ if self.path.is_empty() => self.run(py, &[i])?.unbind(),
            _ => self.run(py, &path())?.unbind(),
        };
        // Not get_or_init: a transform may read this view again. A racing reader's value may win.
        let _ = self.cache[i].set(py, v);
        Ok(self.cache[i].get(py).expect("just set").clone_ref(py))
    }

    fn names<'py>(&self, py: Python<'py>) -> Vec<Bound<'py, PyString>> {
        let a = self.artifact.get();
        level(&a.inner.plan, &self.path)
            .iter()
            .map(|f| a.names[f.name_idx].bind(py).clone())
            .collect()
    }
}

#[pymethods]
impl View {
    /// A field first, so a read skips the failed generic lookup `__getattr__` would wait for (a
    /// field named `to_dict` hides the method); anything else is `object.__getattribute__`.
    fn __getattribute__<'py>(
        slf: &Bound<'py, Self>,
        name: &Bound<'py, PyString>,
    ) -> PyResult<Py<PyAny>> {
        let py = slf.py();
        let this = slf.get();
        let a = this.artifact.get();
        let plan = &a.inner.plan;
        let fields = level(plan, &this.path);
        // A name UTF-8 can't hold (a lone surrogate) is no field.
        let key = name.to_str().ok();
        let found = key.and_then(|k| fields.iter().position(|f| plan.names[f.name_idx] == k));
        let Some(i) = found else {
            let object = py.get_type::<PyAny>();
            return Ok(object
                .getattr(intern!(py, "__getattribute__"))?
                .call1((slf, name))?
                .unbind());
        };
        this.field(py, fields, i)
    }

    /// The same `dict` as `artifact.map(src)` (this object's part of it, for a nested view).
    fn to_dict<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.run(py, &self.path)
    }

    /// Field names at this level, in schema order.
    fn __iter__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyIterator>> {
        PyList::new(py, self.names(py))?.try_iter()
    }

    fn __dir__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyList>> {
        let l = PyList::new(py, self.names(py))?;
        l.append("to_dict")?;
        Ok(l)
    }
}

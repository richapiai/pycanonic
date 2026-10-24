//! `artifact.view(src)`: each canonical field is mapped on its first read and kept, nested objects
//! are views, and nothing counts stats, as in the View classes of codegen.ts. `to_dict()` maps
//! the whole level.

use crate::artifact::{Artifact, body_of, object_root};
use crate::pydoc::PyDoc;
use crate::pysink::PySink;
use crate::transforms::take_pending;
use canonic_core::json::Node;
use canonic_core::{Field, Named};
use pyo3::exceptions::PyKeyError;
use pyo3::intern;
use pyo3::prelude::*;
use pyo3::sync::PyOnceLock;
use pyo3::types::{PyIterator, PyList, PyString};
use std::sync::Arc;

type MaybeNode<'a> = Option<Node<'a>>;

self_cell::self_cell!(
    /// A body and its parse tree, which borrows from it. `None` when it does not parse.
    struct Parsed {
        owner: Vec<u8>,
        #[covariant]
        dependent: MaybeNode,
    }
);

/// The document every view of one `artifact.view` call reads. Nested views share it: an `Arc` of
/// the parse, or another reference to the object. Neither points back at a view, so the root's
/// cache holding its children makes no cycle the GC cannot see.
enum Source {
    /// `bytes` or `str`, copied and parsed once with the plan's filter.
    Parsed(Arc<Parsed>),
    /// Any other object, read as `map` reads it.
    Object(Py<PyAny>),
}

impl Source {
    fn share(&self, py: Python<'_>) -> Source {
        match self {
            Source::Parsed(p) => Source::Parsed(p.clone()),
            Source::Object(o) => Source::Object(o.clone_ref(py)),
        }
    }
}

/// One canonical field of a `Level`, worked out once per artifact.
pub struct LevelField {
    name_idx: usize,
    /// Field indices from the root to this field, as `map_field` takes them.
    path: Vec<usize>,
    /// The `Level` of a nested object field.
    child: Option<usize>,
    /// A `View` attribute (method or dunder) of the same name wins on attribute access.
    shadowed: bool,
}

/// The fields of one canonical object. `levels[0]` is the root.
pub struct Level {
    path: Vec<usize>,
    fields: Vec<LevelField>,
}

/// A `View` attribute of that name, or any `__x__` name: Python looks protocol names up on the
/// instance (`__wrapped__`, `__getnewargs__`), and those must never get a field.
fn shadowed(ty: &Bound<'_, pyo3::types::PyType>, name: &str) -> bool {
    (name.len() > 4 && name.starts_with("__") && name.ends_with("__"))
        || ty.hasattr(name).unwrap_or(true)
}

/// Every object level of `fields`, depth first, the root at index 0.
pub fn levels(py: Python<'_>, names: &[String], fields: &[Named<Field>]) -> Vec<Level> {
    fn walk(
        out: &mut Vec<Level>,
        ty: &Bound<'_, pyo3::types::PyType>,
        names: &[String],
        fs: &[Named<Field>],
        path: Vec<usize>,
    ) -> usize {
        let me = out.len();
        out.push(Level {
            path: path.clone(),
            fields: Vec::new(),
        });
        let level = fs
            .iter()
            .enumerate()
            .map(|(i, f)| {
                let mut p = path.clone();
                p.push(i);
                // An object that can be null is mapped whole on read, not a nested view.
                let child = match f.value.inner() {
                    Field::Object {
                        members,
                        present,
                        if_empty: None,
                    } if present.is_empty() => Some(walk(out, ty, names, members, p.clone())),
                    _ => None,
                };
                LevelField {
                    name_idx: f.name_idx,
                    path: p,
                    child,
                    shadowed: shadowed(ty, &names[f.name_idx]),
                }
            })
            .collect();
        out[me].fields = level;
        me
    }
    let mut out = Vec::new();
    walk(&mut out, &py.get_type::<View>(), names, fields, Vec::new());
    out
}

/// A view of one canonical object. Attribute access reads a field unless `View` has an attribute
/// of that name (`to_dict`) or the name is a `__dunder__`. `view[name]` always reads the field.
#[pyclass(frozen, module = "pycanonic._core")]
pub struct View {
    artifact: Py<Artifact>,
    /// Shared with nested views.
    doc: Source,
    /// Index into the artifact's `levels`.
    level: usize,
    /// One slot per field at this level: the mapped value, or the nested `View`.
    cache: Box<[PyOnceLock<Py<PyAny>>]>,
}

fn slots(n: usize) -> Box<[PyOnceLock<Py<PyAny>>]> {
    (0..n).map(|_| PyOnceLock::new()).collect()
}

impl View {
    pub fn new(py: Python<'_>, artifact: Py<Artifact>, src: &Bound<'_, PyAny>) -> PyResult<View> {
        let a = artifact.get();
        let doc = match body_of(src)? {
            Some(body) => {
                let body = body.into_owned();
                Source::Parsed(Arc::new(
                    py.detach(|| Parsed::new(body, |b| a.inner.parse(b))),
                ))
            }
            None => Source::Object(object_root(src)?.unbind()),
        };
        let n = a.levels[0].fields.len();
        Ok(View {
            artifact,
            doc,
            level: 0,
            cache: slots(n),
        })
    }

    fn fields(&self) -> &[LevelField] {
        &self.artifact.get().levels[self.level].fields
    }

    /// The field at `path`, or this whole object for its own path, as `map` builds it.
    fn run<'py>(&self, py: Python<'py>, path: &[usize]) -> PyResult<Bound<'py, PyAny>> {
        let a = self.artifact.get();
        let mut sink = PySink {
            py,
            names: &a.names,
        };
        let out = match &self.doc {
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

    /// The index of field `name` at this level. Attribute names and field names are interned, so
    /// a pointer compare almost always settles it. The text compare covers the rest.
    fn find(&self, name: &Bound<'_, PyString>) -> Option<usize> {
        let a = self.artifact.get();
        let fields = self.fields();
        if let Some(i) = fields.iter().position(|f| name.is(&a.names[f.name_idx])) {
            return Some(i);
        }
        // A name UTF-8 cannot hold (a lone surrogate) is no field.
        let key = name.to_str().ok()?;
        let names = &a.inner.plan.names;
        fields.iter().position(|f| names[f.name_idx] == key)
    }

    /// Field `i` of this level, mapped (or made a nested view) on first read.
    fn field(&self, py: Python<'_>, i: usize) -> PyResult<Py<PyAny>> {
        if let Some(v) = self.cache[i].get(py) {
            return Ok(v.clone_ref(py));
        }
        let f = &self.fields()[i];
        let v = match f.child {
            Some(level) => Py::new(
                py,
                View {
                    artifact: self.artifact.clone_ref(py),
                    doc: self.doc.share(py),
                    level,
                    cache: slots(self.artifact.get().levels[level].fields.len()),
                },
            )?
            .into_any(),
            None => self.run(py, &f.path)?.unbind(),
        };
        // Not get_or_init: a transform can read this view again. A racing reader's value can win.
        let _ = self.cache[i].set(py, v);
        Ok(self.cache[i].get(py).expect("just set").clone_ref(py))
    }

    fn names<'py>(&self, py: Python<'py>) -> Vec<Bound<'py, PyString>> {
        let a = self.artifact.get();
        let fields = self.fields();
        fields
            .iter()
            .map(|f| a.names[f.name_idx].bind(py).clone())
            .collect()
    }
}

#[pymethods]
impl View {
    /// A field, unless `View` has an attribute of that name (`to_dict`, a dunder): that wins, and
    /// `view[name]` reads the field. Fields are looked up first so a read skips the generic lookup.
    fn __getattribute__<'py>(
        slf: &Bound<'py, Self>,
        name: &Bound<'py, PyString>,
    ) -> PyResult<Py<PyAny>> {
        let this = slf.get();
        if let Some(i) = this.find(name)
            && !this.fields()[i].shadowed
        {
            return this.field(slf.py(), i);
        }
        let object = slf.py().get_type::<PyAny>();
        Ok(object
            .getattr(intern!(slf.py(), "__getattribute__"))?
            .call1((slf, name))?
            .unbind())
    }

    /// Field `name`, whatever attribute shares its name. `KeyError` when there is no such field.
    fn __getitem__(&self, py: Python<'_>, name: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        let i = name.cast::<PyString>().ok().and_then(|n| self.find(n));
        match i {
            Some(i) => self.field(py, i),
            None => Err(PyKeyError::new_err(name.clone().unbind())),
        }
    }

    /// The same `dict` as `artifact.map(src)` (this object's part of it, for a nested view).
    fn to_dict<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let path = &self.artifact.get().levels[self.level].path;
        self.run(py, path)
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

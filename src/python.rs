//! Python bindings: `import fastnode`, then `db = fastnode.Store("data.db")`.
//! JSON values cross the boundary as native Python objects (input via
//! pythonize, output through a hand-rolled converter; see `to_py`).

use crate::{Link, LinkMode, LinkOptions, Predicate, Store};
use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};
use pythonize::depythonize;
use std::sync::Mutex;

fn err(e: anyhow::Error) -> PyErr {
    PyRuntimeError::new_err(format!("{e:#}"))
}

// serde_json's arbitrary_precision Serialize leaks its private marker dict, so
// Values cross to Python through this converter instead of pythonize.
fn to_py(py: Python<'_>, v: &serde_json::Value) -> PyResult<Py<PyAny>> {
    use serde_json::Value::*;
    Ok(match v {
        Null => py.None(),
        Bool(b) => b.into_pyobject(py)?.to_owned().into_any().unbind(),
        Number(n) => {
            if let Some(i) = n.as_i64() {
                i.into_pyobject(py)?.into_any().unbind()
            } else if let Some(u) = n.as_u64() {
                u.into_pyobject(py)?.into_any().unbind()
            } else {
                n.as_f64()
                    .ok_or_else(|| PyRuntimeError::new_err(format!("number out of range: {n}")))?
                    .into_pyobject(py)?
                    .into_any()
                    .unbind()
            }
        }
        String(s) => s.into_pyobject(py)?.into_any().unbind(),
        Array(xs) => {
            let list = PyList::empty(py);
            for x in xs {
                list.append(to_py(py, x)?)?;
            }
            list.into_any().unbind()
        }
        Object(map) => {
            let dict = PyDict::new(py);
            for (k, x) in map {
                dict.set_item(k, to_py(py, x)?)?;
            }
            dict.into_any().unbind()
        }
    })
}

fn link_options(mode: Option<&str>, limit: Option<usize>) -> PyResult<LinkOptions> {
    let mode = match mode {
        None => LinkMode::default(),
        Some(m) => serde_json::from_value(serde_json::json!(m)).map_err(|e| err(e.into()))?,
    };
    Ok(LinkOptions { mode, limit: limit.unwrap_or(100) })
}

fn as_value(v: &Bound<'_, PyAny>) -> PyResult<serde_json::Value> {
    depythonize(v).map_err(|e| err(e.into()))
}

fn ser_to_py<T: serde::Serialize>(py: Python<'_>, v: &T) -> PyResult<Py<PyAny>> {
    let v = serde_json::to_value(v).map_err(|e| err(e.into()))?;
    to_py(py, &v)
}

/// An open FastNode database. All JSON arguments are plain Python dicts/lists.
/// The inner mutex keeps it safe to share across Python threads; calls serialize.
#[pyclass(name = "Store")]
struct PyStore {
    inner: Mutex<Store>,
}

#[pymethods]
impl PyStore {
    /// Opens (or creates) a database. Use ":memory:" for a transient one.
    #[new]
    fn new(path: &str) -> PyResult<Self> {
        Ok(Self { inner: Mutex::new(Store::open(path).map_err(err)?) })
    }

    /// Creates one Node from {"type":…, "summary":…, "attrs":{…}}, returns its id.
    fn create(&self, node: &Bound<'_, PyAny>) -> PyResult<u32> {
        self.inner.lock().unwrap().create(depythonize(node).map_err(|e| err(e.into()))?).map_err(err)
    }

    /// Creates many Nodes atomically, returns their ids.
    fn create_many(&self, nodes: &Bound<'_, PyAny>) -> PyResult<Vec<u32>> {
        self.inner.lock().unwrap().create_many(depythonize(nodes).map_err(|e| err(e.into()))?).map_err(err)
    }

    /// Fetches one Node with its links; None when missing.
    #[pyo3(signature = (id, links=None, link_limit=None))]
    fn get(&self, py: Python<'_>, id: u32, links: Option<&str>, link_limit: Option<usize>) -> PyResult<Option<Py<PyAny>>> {
        let node = self.inner.lock().unwrap().get_with(id, &link_options(links, link_limit)?).map_err(err)?;
        node.map(|n| ser_to_py(py, &n)).transpose()
    }

    /// Runs a query dict (the query protocol from the README), returns the result dict.
    fn query(&self, py: Python<'_>, query: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        let result = self.inner.lock().unwrap().query(&depythonize(query).map_err(|e| err(e.into()))?).map_err(err)?;
        ser_to_py(py, &result)
    }

    /// Every Node whose attrs contain this scalar at any path.
    fn find(&self, value: &Bound<'_, PyAny>) -> PyResult<Vec<u32>> {
        let set = self.inner.lock().unwrap().select(&Predicate::Any { value: as_value(value)? }).map_err(err)?;
        Ok(set.iter().collect())
    }

    /// Applies a JSON Merge Patch to {type, summary, attrs}.
    fn patch(&self, id: u32, patch: &Bound<'_, PyAny>) -> PyResult<()> {
        self.inner.lock().unwrap().patch(id, as_value(patch)?).map_err(err)
    }

    fn delete(&self, id: u32) -> PyResult<bool> {
        self.inner.lock().unwrap().delete(id).map_err(err)
    }

    fn link(&self, from: u32, relation: &str, to: u32) -> PyResult<bool> {
        self.inner.lock().unwrap().link(&Link { from, relation: relation.into(), to }).map_err(err)
    }

    fn unlink(&self, from: u32, relation: &str, to: u32) -> PyResult<bool> {
        self.inner.lock().unwrap().unlink(&Link { from, relation: relation.into(), to }).map_err(err)
    }

    /// Streams an array/JSONL file of Nodes into the database.
    #[pyo3(signature = (path, batch_size=5000))]
    fn import(&self, py: Python<'_>, path: &str, batch_size: usize) -> PyResult<Py<PyAny>> {
        let file = std::fs::File::open(path).map_err(|e| err(e.into()))?;
        let report = self.inner.lock().unwrap().import(std::io::BufReader::new(file), batch_size).map_err(err)?;
        ser_to_py(py, &report)
    }

    fn stats(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let stats = self.inner.lock().unwrap().stats().map_err(err)?;
        ser_to_py(py, &stats)
    }
}

#[pymodule]
fn fastnode(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyStore>()?;
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    Ok(())
}

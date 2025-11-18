//! Python bindings for synddb-client via PyO3

#[cfg(feature = "python")]
use pyo3::prelude::*;

#[cfg(feature = "python")]
use crate::{SyndDB, Config};

#[cfg(feature = "python")]
#[pyfunction]
fn attach(conn: &PyAny, sequencer_url: String) -> PyResult<()> {
    // TODO: Extract raw SQLite connection handle from Python object
    // This requires interfacing with Python's sqlite3.Connection object
    // which internally wraps a sqlite3* pointer

    // For now, just log
    println!("Python: attach called with url={}", sequencer_url);

    Ok(())
}

#[cfg(feature = "python")]
#[pymodule]
fn synddb(_py: Python, m: &PyModule) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(attach, m)?)?;
    Ok(())
}

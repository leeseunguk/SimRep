//! The extension is opt-in so ordinary Rust workspace builds need no Python.
#![cfg(feature = "python")]

mod flight;
mod geometry;
mod pa;
mod refine;

use pyo3::prelude::*;

// Compile the command frontend itself, rather than maintaining a second copy of
// its import/export, geometry-sidecar and benchmark defaults and validation.
include!("../../simrep-cli/src/main.rs");

#[pyfunction]
fn command(py: Python<'_>, arguments: Vec<String>) -> PyResult<Option<String>> {
    let cli = Cli::try_parse_from(std::iter::once("simrep".to_owned()).chain(arguments))
        .map_err(pa::error)?;
    // Keep the CLI entry point linked; command execution uses its fallible run.
    let _entry = main;
    py.detach(|| json::capture(cli))
        .map(|summary| summary.map(|value| value.to_string()))
        .map_err(pa::error)
}

#[pymodule]
fn _native(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(command, module)?)?;
    module.add_function(wrap_pyfunction!(pa::read, module)?)?;
    module.add_function(wrap_pyfunction!(pa::write, module)?)?;
    module.add_function(wrap_pyfunction!(pa::adjust, module)?)?;
    module.add_function(wrap_pyfunction!(refine::refine, module)?)?;
    module.add_function(wrap_pyfunction!(geometry::refine_geometry, module)?)?;
    module.add_function(wrap_pyfunction!(flight::read_particles, module)?)?;
    module.add_function(wrap_pyfunction!(flight::fly, module)?)?;
    Ok(())
}

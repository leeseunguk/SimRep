use pyo3::prelude::*;
use serde_json::json;
use simrep_core::uncertainty::{Level, grid_study, observed_order};

#[pyfunction]
pub fn grid_uncertainty(levels: Vec<(f64, f64)>, least_squares: bool) -> PyResult<String> {
    let levels: Vec<_> = levels
        .into_iter()
        .map(|(h, value)| Level { h, value })
        .collect();
    let error =
        |e: simrep_core::uncertainty::Error| pyo3::exceptions::PyValueError::new_err(e.to_string());
    if least_squares {
        let study = grid_study(&levels).map_err(error)?;
        Ok(json!({
            "p": study.p, "extrapolated": study.phi_ext,
            "u_num": study.u_fine, "sigma": study.sigma,
            "safety_factor": study.fs, "estimator": format!("{:?}", study.estimator),
            "weighted": study.weighted, "behavior": format!("{:?}", study.behaviour),
        })
        .to_string())
    } else {
        let triplet = levels.try_into().map_err(|_: Vec<Level>| {
            pyo3::exceptions::PyValueError::new_err("observed_order requires exactly three levels")
        })?;
        let result = observed_order(&triplet).map_err(error)?;
        Ok(json!({
            "p": result.p, "extrapolated": result.phi_ext,
            "gci_fine": result.gci_fine, "approximate_relative_error": result.e_a,
            "behavior": format!("{:?}", result.behaviour),
        })
        .to_string())
    }
}

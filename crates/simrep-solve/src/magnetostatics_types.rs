//! Magnetic PA metadata needed by CPU field reconstruction without GPU solvers.
use serde::{Deserialize, Serialize};

/// Meaning of the values in a magnetic potential PA.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Formulation {
    Scalar,
    PlanarAz,
    AxisymmetricPsi,
    CartesianA,
}

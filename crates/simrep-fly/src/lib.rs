//! CPU f64 reference particle flying in SIMION units: mm, us, eV and Gauss.
//!
//! Grid potentials use C2 vacuum-boundary patches, or interpolating tensor
//! cubic B-splines on electrode-free grids. Both give a C1 electric field.
//! See the crate README for boundary conditions and collision conventions.

mod expression;
mod field;
mod flight;
pub mod gpu;
mod instance;
pub mod physics;
pub mod surface;

pub use expression::VoltageExpression;
pub use field::{
    AnalyticField, BasisPotential, Field, FieldSample, GridField, Reconstruction, Superposition,
};
pub use flight::{
    FlightOptions, FlightResult, Integrator, Particle, ParticleState, StepLimits, StopPlane, fly,
    fly_many, fly_many_from_electrode,
};
pub use instance::{InstanceField, WorkbenchField};

/// Elementary charge / atomic mass unit, converted to mm/us^2 per V/mm.
pub const ELECTRIC_ACCELERATION: f64 = 1.602_176_634e-19 / 1.660_539_068_92e-27 * 1e-6;
/// Cyclotron angular frequency in 1/us per Gauss, for charge/mass in e/amu.
pub const MAGNETIC_FREQUENCY: f64 = ELECTRIC_ACCELERATION * 1e-4;
/// Speed of light in mm/us.
pub const SPEED_OF_LIGHT: f64 = 299_792.458;

pub type Vector = [f64; 3];

/// A terminal flight event. Electrode indices refer to the original x-fast PA.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplatReason {
    Electrode {
        index: usize,
    },
    /// Physical electrode number from continuous source geometry or a cuts sidecar.
    SurfaceElectrode {
        number: usize,
    },
    OutsideGrid,
    StopPlane,
    TimeLimit,
    StepLimit,
    StepUnderflow,
    NonFinite,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FlyError(pub &'static str);

impl std::fmt::Display for FlyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

impl std::error::Error for FlyError {}

pub(crate) fn dot(a: Vector, b: Vector) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

pub(crate) fn cross(a: Vector, b: Vector) -> Vector {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

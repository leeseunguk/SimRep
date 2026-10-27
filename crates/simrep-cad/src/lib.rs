//! STEP/IGES electrodes in millimeters (negative inside).
//!
//! Enable `occt` to dynamically link an installed OCCT 7.6/7.7 SDK.
//! The default workspace does not build or link a CAD kernel.

#[cfg(feature = "occt")]
mod measure_mesh;
#[cfg(feature = "occt")]
mod occt;
#[cfg(feature = "occt")]
pub use occt::*;

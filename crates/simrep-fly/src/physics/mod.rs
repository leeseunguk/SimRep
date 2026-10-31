//! CPU f64 independent-ion magnetic fields and neutral-gas collisions,
//! plus synchronized space-charge ensemble integration.

#[cfg(feature = "gpu")]
pub mod beam;
pub mod collision;
#[cfg(feature = "gpu")]
pub mod ensemble;
#[cfg(feature = "gpu")]
pub mod gpu_pic;
pub mod magnetic;
#[cfg(feature = "gpu")]
pub mod space_charge;

#[cfg(feature = "gpu")]
pub use beam::{BeamInteraction, BeamOptions, BeamReport, BeamTermination, fly_beam};
#[cfg(feature = "gpu")]
pub use ensemble::{EnsembleOptions, EnsembleReport, fly_ensemble};
#[cfg(feature = "gpu")]
pub use gpu_pic::GpuPic;
#[cfg(feature = "gpu")]
pub use space_charge::{ChargeMode, DirectCharge, PicMesh, SpaceCharge};

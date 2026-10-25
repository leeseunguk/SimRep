//! CPU f64 independent-ion magnetic fields and neutral-gas collisions,
//! plus synchronized space-charge ensemble integration.

pub mod beam;
pub mod collision;
pub mod ensemble;
pub mod gpu_pic;
pub mod magnetic;
pub mod space_charge;

pub use beam::{BeamInteraction, BeamOptions, BeamReport, BeamTermination, fly_beam};
pub use ensemble::{EnsembleOptions, EnsembleReport, fly_ensemble};
pub use gpu_pic::GpuPic;
pub use space_charge::{ChargeMode, DirectCharge, PicMesh, SpaceCharge};

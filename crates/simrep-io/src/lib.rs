//! SIMION file formats (PA/PA#/PA0, fast adjust, ION, FLY2) and imported magnetic field maps.

pub mod bfield;
pub mod fast_adjust;
#[cfg(feature = "lua")]
pub mod fly2;
#[cfg(not(feature = "lua"))]
#[path = "fly2_literal.rs"]
pub mod fly2;
pub mod pa;
pub mod particles;

pub use fast_adjust::read_fast_adjust;
pub use fly2::{
    Fly2Context, Fly2Field, Fly2FieldProvider, Fly2Instance, parse_fly2, read_fly2, read_fly2_in,
};
pub use pa::{FastScalable, Pa0Metadata, PaFile, PaMode, read_pa, write_pa};
pub use particles::{ParticleDefinition, parse_ion, read_ion, read_particles, read_particles_in};

//! SIMION file formats (.pa, .fly2, .gem, .rec) and STL.

pub mod bfield;
pub mod fast_adjust;
pub mod fly2;
pub mod pa;
pub mod particles;

pub use fast_adjust::read_fast_adjust;
pub use fly2::{
    Fly2Context, Fly2Field, Fly2FieldProvider, Fly2Instance, parse_fly2, read_fly2, read_fly2_in,
};
pub use pa::{FastScalable, Pa0Metadata, PaFile, PaMode, read_pa, write_pa};
pub use particles::{ParticleDefinition, parse_ion, read_ion, read_particles, read_particles_in};

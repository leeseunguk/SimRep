//! Potential solvers: multigrid (CPU/GPU), cut-cell, residual correction.

// Keep the CPU algorithms' standard-library paths identical on native targets.
// On wasm only, route time through the browser clock while reexporting the rest
// of std unchanged. web-time uses the same std::time::Duration type.
#[cfg(target_arch = "wasm32")]
extern crate self as std;
#[cfg(target_arch = "wasm32")]
extern crate std as native_std;
#[cfg(target_arch = "wasm32")]
pub use native_std::*;
#[cfg(target_arch = "wasm32")]
pub mod time {
    pub use web_time::*;
}

pub mod adaptive;
pub mod cutcell;
#[cfg(feature = "gpu")]
pub mod dielectric;
#[cfg(feature = "gpu")]
pub mod gpu;
#[cfg(all(feature = "webgpu", not(feature = "gpu")))]
pub mod gpu {
    pub type GpuResult<T> = Result<T, Box<dyn std::error::Error>>;
}
#[cfg(feature = "gpu")]
pub mod magnetostatics;
#[cfg(not(feature = "gpu"))]
#[path = "magnetostatics_types.rs"]
pub mod magnetostatics;
pub mod mg;
#[cfg(feature = "webgpu")]
pub mod webgpu;

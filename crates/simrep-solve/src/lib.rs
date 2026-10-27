//! Potential solvers: multigrid (CPU/GPU), cut-cell, residual correction.

pub mod adaptive;
pub mod cutcell;
pub mod dielectric;
pub mod gpu;
pub mod magnetostatics;
pub mod mg;

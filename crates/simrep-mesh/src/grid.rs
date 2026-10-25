use crate::{Mesh, Part, Point, Solid, invalid};
use rayon::prelude::*;
use simrep_core::PotentialArray;
use std::io;
use std::time::{Duration, Instant};

/// Cartesian sample nodes, x fastest. Origin and spacing are in mm.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GridSpec {
    pub dims: [usize; 3],
    pub origin_mm: Point,
    pub spacing_mm: Point,
}

impl GridSpec {
    pub fn validate(self) -> io::Result<usize> {
        if self.dims.contains(&0)
            || self.origin_mm.iter().any(|v| !v.is_finite())
            || self.spacing_mm.iter().any(|v| !v.is_finite() || *v <= 0.0)
        {
            return Err(invalid(
                "grid requires positive dimensions/spacing and finite coordinates",
            ));
        }
        for i in 0..3 {
            if !(self.origin_mm[i] + (self.dims[i] - 1) as f64 * self.spacing_mm[i]).is_finite() {
                return Err(invalid("grid extent is not finite"));
            }
        }
        self.dims
            .into_iter()
            .try_fold(1usize, |n, d| n.checked_mul(d))
            .ok_or_else(|| invalid("grid size overflows usize"))
    }

    pub fn point(self, index: usize) -> Point {
        let [nx, ny, _] = self.dims;
        let indices = [index % nx, (index / nx) % ny, index / (nx * ny)];
        std::array::from_fn(|i| self.origin_mm[i] + indices[i] as f64 * self.spacing_mm[i])
    }
}

#[derive(Debug)]
pub struct Voxelization {
    pub array: PotentialArray,
    pub origin_mm: Point,
    /// Counts before overlap resolution, in mesh part order.
    pub points_per_part: Vec<usize>,
    /// A later part wins if solids overlap. This count includes each replaced
    /// assignment (so a triple overlap is counted twice).
    pub overlap_assignments: usize,
    pub elapsed: Duration,
}

/// Sample closed solids at grid nodes and put the supplied electrode number
/// into each occupied PA point. Later parts win overlaps deterministically.
/// PA files cannot store an origin; retain `origin_mm` in the calling project.
pub fn voxelize(mesh: &Mesh, grid: GridSpec, electrodes: &[u32]) -> io::Result<Voxelization> {
    voxelize_inner(mesh, grid, electrodes, false)
}

/// Explicit best-effort parity sampling without requiring closed topology.
/// This preserves the input geometry; it is not a repair. Call `inspect` and
/// retain its reports, and compare the result against the intended solid.
pub fn voxelize_with_defects(
    mesh: &Mesh,
    grid: GridSpec,
    electrodes: &[u32],
) -> io::Result<Voxelization> {
    voxelize_inner(mesh, grid, electrodes, true)
}

fn voxelize_inner(
    mesh: &Mesh,
    grid: GridSpec,
    electrodes: &[u32],
    allow_defects: bool,
) -> io::Result<Voxelization> {
    let start = Instant::now();
    mesh.validate()?;
    let count = grid.validate()?;
    if electrodes.len() != mesh.parts.len()
        || electrodes.contains(&0)
        || electrodes.iter().any(|&e| e > 100_000)
    {
        return Err(invalid(
            "supply one electrode number in 1..=100000 for every part",
        ));
    }
    let [nx, ny, nz] = grid.dims;
    let mut assignment = vec![0_u32; count];
    let mut points_per_part = Vec::new();
    let mut overlap_assignments = 0;
    for (part, &electrode) in mesh.parts.iter().zip(electrodes) {
        let solid = if allow_defects {
            Solid::from_surface(part)?
        } else {
            Solid::new(part)?
        };
        let counts = assignment
            .par_chunks_mut(nx)
            .enumerate()
            .map(|(row, output)| {
                let start = [
                    grid.origin_mm[0],
                    grid.origin_mm[1] + (row % ny) as f64 * grid.spacing_mm[1],
                    grid.origin_mm[2] + (row / ny) as f64 * grid.spacing_mm[2],
                ];
                let inside = solid.row(start, grid.spacing_mm[0], nx);
                let mut counts = (0, 0);
                for (value, inside) in output.iter_mut().zip(inside) {
                    if inside {
                        counts.0 += 1;
                        counts.1 += usize::from(*value != 0);
                        *value = electrode;
                    }
                }
                counts
            })
            .reduce(|| (0, 0), |a, b| (a.0 + b.0, a.1 + b.1));
        points_per_part.push(counts.0);
        overlap_assignments += counts.1;
    }
    let mut array = PotentialArray::new(nx, ny, nz);
    [array.dx_mm, array.dy_mm, array.dz_mm] = grid.spacing_mm;
    for (i, &electrode) in assignment.iter().enumerate() {
        array.electrodes[i] = electrode != 0;
        array.potentials[i] = f64::from(electrode);
    }
    Ok(Voxelization {
        array,
        origin_mm: grid.origin_mm,
        points_per_part,
        overlap_assignments,
        elapsed: start.elapsed(),
    })
}

#[derive(Clone, Debug)]
pub struct SignedDistanceGrid {
    pub name: String,
    pub dims: [usize; 3],
    pub origin: Point,
    pub spacing: Point,
    /// Signed Euclidean distance in mm within two maximum grid spacings of the
    /// mesh; magnitudes are clamped beyond that band. Negative inside.
    /// Index = x + nx * (y + ny * z).
    pub values: Vec<f64>,
    pub electrode_number: u32,
    pub voltage: f64,
    pub elapsed: Duration,
}

impl SignedDistanceGrid {
    pub fn grid(&self) -> GridSpec {
        GridSpec {
            dims: self.dims,
            origin_mm: self.origin,
            spacing_mm: self.spacing,
        }
    }

    /// Trilinear interpolation inside the sampled domain; no extrapolation.
    pub fn interpolate(&self, point_mm: Point) -> Option<f64> {
        let position: Point =
            std::array::from_fn(|i| (point_mm[i] - self.origin[i]) / self.spacing[i]);
        if (0..3).any(|i| {
            !position[i].is_finite() || position[i] < 0.0 || position[i] > (self.dims[i] - 1) as f64
        }) {
            return None;
        }
        let lo: [usize; 3] = position.map(|v| v.floor() as usize);
        let hi: [usize; 3] = std::array::from_fn(|i| (lo[i] + 1).min(self.dims[i] - 1));
        let fraction: Point = std::array::from_fn(|i| position[i] - lo[i] as f64);
        let mut value = 0.0;
        for z in 0..2 {
            for y in 0..2 {
                for x in 0..2 {
                    let corner = [x, y, z];
                    let index: [usize; 3] =
                        std::array::from_fn(|i| if corner[i] == 0 { lo[i] } else { hi[i] });
                    let weight: f64 = (0..3)
                        .map(|i| {
                            if corner[i] == 0 {
                                1.0 - fraction[i]
                            } else {
                                fraction[i]
                            }
                        })
                        .product();
                    value += weight
                        * self.values
                            [index[0] + self.dims[0] * (index[1] + self.dims[1] * index[2])];
                }
            }
        }
        Some(value)
    }
}

/// Sample a closed mesh exactly in a band of two maximum grid spacings.
/// Farther nodes retain their exact membership and a clamped magnitude.
pub fn sample_sdf(
    part: &Part,
    grid: GridSpec,
    electrode_number: u32,
    voltage: f64,
) -> io::Result<SignedDistanceGrid> {
    let start = Instant::now();
    let count = grid.validate()?;
    if electrode_number == 0 || electrode_number > 100_000 || !voltage.is_finite() {
        return Err(invalid(
            "SDF electrode needs a number in 1..=100000 and finite voltage",
        ));
    }
    let solid = Solid::new(part)?;
    let mut values = vec![0.0; count];
    let band_mm = 2.0 * grid.spacing_mm.into_iter().fold(0.0, f64::max);
    simrep_geom::SignedDistance::sample_grid_band(
        &solid,
        grid.dims,
        grid.origin_mm,
        grid.spacing_mm,
        band_mm,
        &mut values,
    );
    if values.iter().any(|v| !v.is_finite()) {
        return Err(invalid("SDF coordinates exceed the finite distance range"));
    }
    Ok(SignedDistanceGrid {
        name: part.name.clone(),
        dims: grid.dims,
        origin: grid.origin_mm,
        spacing: grid.spacing_mm,
        values,
        electrode_number,
        voltage,
        elapsed: start.elapsed(),
    })
}

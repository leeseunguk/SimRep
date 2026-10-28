use simrep_core::{FieldType, PotentialArray, Symmetry};

use crate::{FlyError, SplatReason, Vector};

mod boundary;
mod ghost;

pub use ghost::GhostFillReport;

/// Boundary-aware vacuum reconstruction, or the original spline through metal.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Reconstruction {
    #[default]
    BoundaryAware,
    GlobalCubic,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FieldSample {
    pub potential_v: f64,
    pub electric_v_per_mm: Vector,
    pub magnetic_gauss: Vector,
}

/// Field evaluation is separate from geometry so integrator trial stages may
/// extend beyond a boundary. Only accepted trajectories produce splats.
pub trait Field: Sync {
    fn sample(&self, position_mm: Vector, time_us: f64) -> FieldSample;

    /// Smallest physical active-axis cell spacing, including field-free gaps.
    fn spatial_scale_mm(&self) -> Option<f64> {
        None
    }

    /// Shortest known smooth voltage time scale (inverse angular frequency).
    fn time_scale_us(&self) -> Option<f64> {
        None
    }

    fn status(&self, _position_mm: Vector) -> Option<SplatReason> {
        None
    }

    /// First geometry event on a straight segment, as a fraction in [0, 1].
    fn intersection(&self, _from: Vector, _to: Vector) -> Option<(f64, SplatReason)> {
        None
    }

    /// Known voltage discontinuity strictly after absolute time in us.
    fn next_time_event(&self, _time_us: f64) -> Option<f64> {
        None
    }

    /// Collision after leaving the initially occupied metal, for an explicit
    /// surface-emission launch. Ordinary flights retain their start-in-metal splat.
    fn launch_intersection(&self, from: Vector, to: Vector) -> Option<(f64, SplatReason)> {
        self.intersection(from, to)
    }
}

/// Closures return V, E in V/mm, and B in Gauss, at absolute time in us.
pub struct AnalyticField<F>(pub F);

impl<F: Fn(Vector, f64) -> FieldSample + Sync> Field for AnalyticField<F> {
    fn sample(&self, position_mm: Vector, time_us: f64) -> FieldSample {
        (self.0)(position_mm, time_us)
    }
}

/// A C2 interpolating potential, reconstructed once on construction.
///
/// Vacuum-boundary patches use six coefficients per active axis; the global
/// cubic uses four (and remains the default without electrodes). Outside the
/// physical domain the end polynomial is extrapolated for integrator stages;
/// `status` and `intersection` still report the physical array boundary.
#[derive(Clone, Debug)]
pub struct GridField {
    array: PotentialArray,
    coefficients: Vec<f64>,
    coefficient_dims: [usize; 3],
    boundary: Option<boundary::BoundarySpline>,
    transparent_faces: [bool; 6],
    node_cell_collisions: bool,
    solid_cells: Option<Vec<bool>>,
    reconstruction: Reconstruction,
    compact: bool,
    ghost_fill: Option<GhostFillReport>,
}

impl GridField {
    pub fn new(array: PotentialArray) -> Result<Self, FlyError> {
        Self::with_reconstruction(array, Reconstruction::BoundaryAware)
    }

    /// The same boundary-aware reconstruction with compact nodal derivatives.
    /// CPU samples materialize only their cell's polynomial; GPU preparation
    /// can upload the shared derivatives instead of all cell powers.
    pub fn new_compact(array: PotentialArray) -> Result<Self, FlyError> {
        Self::build(array, Reconstruction::BoundaryAware, true)
    }

    /// A scalar GPU field-map component, retaining C2 jets without material.
    pub(crate) fn new_gpu_map(array: PotentialArray) -> Result<Self, FlyError> {
        let mut field = Self::new(array)?;
        field.boundary = Some(boundary::BoundarySpline::new_compact(&field.array));
        field.coefficients.clear();
        Ok(field)
    }

    pub fn with_reconstruction(
        array: PotentialArray,
        reconstruction: Reconstruction,
    ) -> Result<Self, FlyError> {
        Self::build(array, reconstruction, false)
    }

    fn build(
        array: PotentialArray,
        reconstruction: Reconstruction,
        compact: bool,
    ) -> Result<Self, FlyError> {
        let dims = [array.nx, array.ny, array.nz];
        let len = dims
            .into_iter()
            .try_fold(1usize, |a, b| a.checked_mul(b))
            .ok_or(FlyError("grid dimensions overflow"))?;
        if dims.contains(&0)
            || array.potentials.len() != len
            || array.electrodes.len() != len
            || array.potentials.iter().any(|v| !v.is_finite())
            || [array.dx_mm, array.dy_mm, array.dz_mm]
                .iter()
                .any(|h| !h.is_finite() || *h <= 0.0)
        {
            return Err(FlyError(
                "invalid grid dimensions, storage, spacing or potentials",
            ));
        }
        if array.field_type != FieldType::Electrostatic {
            return Err(FlyError(
                "GridField requires electric potentials, not magnetic PA metadata",
            ));
        }
        if array.symmetry == Symmetry::Cylindrical && (array.nz != 1 || array.ny < 2) {
            return Err(FlyError("cylindrical grids require nz=1 and ny>=2"));
        }
        let mirrors = [array.mirrors.x, array.mirrors.y, array.mirrors.z];
        let anti = [
            array.anti_mirrors.x,
            array.anti_mirrors.y,
            array.anti_mirrors.z,
        ];
        for axis in 0..3 {
            if anti[axis] && !mirrors[axis] {
                return Err(FlyError("anti-mirrors require the matching mirror flag"));
            }
            if anti[axis] {
                for (i, &value) in array.potentials.iter().enumerate() {
                    let coordinate = [
                        i % dims[0],
                        (i / dims[0]) % dims[1],
                        i / (dims[0] * dims[1]),
                    ];
                    if coordinate[axis] == 0 && value != 0.0 {
                        return Err(FlyError("anti-mirror potential must be zero on its plane"));
                    }
                }
            }
        }
        if array.symmetry == Symmetry::Cylindrical && anti[1] {
            return Err(FlyError(
                "a cylindrical radial axis cannot be anti-mirrored",
            ));
        }
        let boundary = (reconstruction == Reconstruction::BoundaryAware
            && array.electrodes.iter().any(|&e| e))
        .then(|| {
            if compact {
                boundary::BoundarySpline::new_compact(&array)
            } else {
                boundary::BoundarySpline::new(&array)
            }
        });
        let mut coefficients = if boundary.is_some() {
            Vec::new()
        } else {
            array.potentials.clone()
        };
        let mut coefficient_dims = dims;
        if boundary.is_none() {
            for axis in 0..3 {
                if dims[axis] == 1 {
                    continue;
                }
                let even = (mirrors[axis] && !anti[axis])
                    || (axis == 1 && array.symmetry == Symmetry::Cylindrical);
                (coefficients, coefficient_dims) =
                    prefilter(&coefficients, coefficient_dims, axis, even, anti[axis]);
            }
        }
        Ok(Self {
            array,
            coefficients,
            coefficient_dims,
            boundary,
            transparent_faces: [false; 6],
            node_cell_collisions: true,
            solid_cells: None,
            reconstruction,
            compact,
            ghost_fill: None,
        })
    }

    /// Rebuild shared jets on a vacuum continuation of near-surface metal
    /// knots. The original PA and its collision mask remain unchanged.
    pub fn with_surface(
        mut self,
        surface: &crate::surface::SurfaceModel,
    ) -> Result<Self, FlyError> {
        surface.validate_array(&self.array)?;
        let (filled, report) = ghost::fill(&self.array, surface);
        if self.ghost_fill.is_some() || report.degree_counts.iter().sum::<usize>() > 0 {
            let rebuilt = Self::build(filled, self.reconstruction, self.compact)?;
            self.coefficients = rebuilt.coefficients;
            self.coefficient_dims = rebuilt.coefficient_dims;
            self.boundary = rebuilt.boundary;
        }
        self.ghost_fill = Some(report);
        Ok(self)
    }

    pub fn ghost_fill_report(&self) -> Option<&GhostFillReport> {
        self.ghost_fill.as_ref()
    }

    /// Additional physical occupancy without treating material nodes as
    /// Dirichlet data during potential reconstruction.
    pub(crate) fn with_solid_cells(mut self, cells: Vec<bool>) -> Result<Self, FlyError> {
        let dims =
            [self.array.nx, self.array.ny, self.array.nz].map(|n| n.saturating_sub(1).max(1));
        if cells.len() != dims.iter().product::<usize>() {
            return Err(FlyError(
                "material cell mask dimensions must match the potential grid",
            ));
        }
        self.solid_cells = cells.iter().any(|&v| v).then_some(cells);
        self.node_cell_collisions = true;
        Ok(self)
    }

    pub(crate) fn has_solid_cells(&self) -> bool {
        self.solid_cells.is_some()
    }

    pub fn array(&self) -> &PotentialArray {
        &self.array
    }

    /// PA extent membership, independently of electrode occupancy.
    pub(crate) fn contains(&self, position_mm: Vector) -> bool {
        let (q, _, _) = self.coordinates(position_mm);
        let dims = [self.array.nx, self.array.ny, self.array.nz];
        let h = [self.array.dx_mm, self.array.dy_mm, self.array.dz_mm];
        (0..3).all(|a| dims[a] == 1 || (0.0..=(dims[a] - 1) as f64).contains(&(q[a] / h[a])))
    }

    /// Ideal wire grids retain Dirichlet field values but do not absorb ions.
    /// Face order: xmin, xmax, ymin, ymax, zmin, zmax.
    pub fn with_transparent_faces(mut self, faces: [bool; 6]) -> Self {
        self.transparent_faces = faces;
        self
    }

    /// Staircase PA bodies occupy cells whose corners are all electrode nodes.
    pub(crate) fn with_node_cell_collisions(mut self) -> Self {
        self.node_cell_collisions = true;
        self
    }

    /// Historical comparison geometry: each electrode node occupies its
    /// nearest-node voxel, clipped to the PA extent.
    pub fn with_nearest_node_collisions(mut self) -> Self {
        self.node_cell_collisions = false;
        self
    }

    pub(crate) fn collision_mask(&self) -> Vec<u32> {
        self.array
            .electrodes
            .iter()
            .enumerate()
            .map(|(i, &metal)| {
                let node = [
                    i % self.array.nx,
                    (i / self.array.nx) % self.array.ny,
                    i / (self.array.nx * self.array.ny),
                ];
                u32::from(metal && !self.transparent_node(node))
            })
            .collect()
    }

    fn transparent_node(&self, node: [usize; 3]) -> bool {
        let dims = [self.array.nx, self.array.ny, self.array.nz];
        (0..3).any(|a| {
            dims[a] > 1
                && (node[a] == 0 && self.transparent_faces[2 * a]
                    || node[a] + 1 == dims[a] && self.transparent_faces[2 * a + 1])
        })
    }

    pub(crate) fn coordinates(&self, position: Vector) -> (Vector, Vector, f64) {
        let mut q = position;
        if self.array.symmetry == Symmetry::Cylindrical {
            q[1] = position[1].hypot(position[2]);
            q[2] = 0.0;
        }
        let mirrors = [
            self.array.mirrors.x,
            self.array.mirrors.y,
            self.array.mirrors.z,
        ];
        let anti = [
            self.array.anti_mirrors.x,
            self.array.anti_mirrors.y,
            self.array.anti_mirrors.z,
        ];
        let mut reflection = [1.0; 3];
        let mut sign = 1.0;
        for axis in 0..3 {
            if mirrors[axis] && q[axis] < 0.0 {
                q[axis] = -q[axis];
                reflection[axis] = -1.0;
                if anti[axis] {
                    sign = -sign;
                }
            }
        }
        (q, reflection, sign)
    }

    pub(crate) fn compatible(&self, other: &Self) -> bool {
        let a = &self.array;
        let b = &other.array;
        (a.nx, a.ny, a.nz, a.symmetry, a.mirrors, a.anti_mirrors)
            == (b.nx, b.ny, b.nz, b.symmetry, b.mirrors, b.anti_mirrors)
            && [a.dx_mm, a.dy_mm, a.dz_mm] == [b.dx_mm, b.dy_mm, b.dz_mm]
            && a.electrodes == b.electrodes
            && self.transparent_faces == other.transparent_faces
            && self.node_cell_collisions == other.node_cell_collisions
            && self.solid_cells == other.solid_cells
    }

    /// Whether collisions require all corners of a PA cell to be electrodes.
    pub fn uses_node_cell_collisions(&self) -> bool {
        self.node_cell_collisions
    }

    /// Borrow precomputed powers, or export shared 3D boundary jets. No
    /// resampling, re-solving, or full precomputed coefficient clone.
    pub(crate) fn gpu_data(
        &self,
        nodal: bool,
    ) -> ([usize; 3], [usize; 3], std::borrow::Cow<'_, [f64]>, bool) {
        if let Some(b) = &self.boundary {
            if nodal && b.degrees == [6, 6, 6] {
                return (
                    b.cells,
                    b.degrees,
                    std::borrow::Cow::Owned(b.jets.iter().flatten().copied().collect()),
                    true,
                );
            }
            if !b.coefficients.is_empty() {
                return (
                    b.cells,
                    b.degrees,
                    std::borrow::Cow::Borrowed(&b.coefficients),
                    false,
                );
            }
        }
        let (cells, degrees, powers) = self.gpu_patches();
        (cells, degrees, std::borrow::Cow::Owned(powers), false)
    }

    /// Native physics also contracts jets for planar and cylindrical grids.
    pub(crate) fn gpu_physics_data(
        &self,
        nodal: bool,
    ) -> ([usize; 3], [usize; 3], std::borrow::Cow<'_, [f64]>, bool) {
        if nodal && let Some(b) = &self.boundary {
            return (
                b.cells,
                b.degrees,
                std::borrow::Cow::Owned(b.jets.iter().flatten().copied().collect()),
                true,
            );
        }
        self.gpu_data(nodal)
    }

    /// Export the same cell powers used by the CPU; no resampling or re-solving.
    pub(crate) fn gpu_patches(&self) -> ([usize; 3], [usize; 3], Vec<f64>) {
        if let Some(b) = &self.boundary {
            if b.coefficients.is_empty() {
                let size = b.degrees.iter().product::<usize>();
                let mut coefficients = vec![0.0; b.cells.iter().product::<usize>() * size];
                for (index, c) in coefficients.chunks_mut(size).enumerate() {
                    b.cell_powers(index, c);
                }
                return (b.cells, b.degrees, coefficients);
            }
            return (b.cells, b.degrees, b.coefficients.clone());
        }
        let dims = [self.array.nx, self.array.ny, self.array.nz];
        let cells = dims.map(|n| n.saturating_sub(1).max(1));
        let degrees = dims.map(|n| if n == 1 { 1 } else { 4 });
        let size: usize = degrees.iter().product();
        let mut coefficients = vec![0.0; cells.iter().product::<usize>() * size];
        const POWERS: [[f64; 4]; 4] = [
            [1.0 / 6.0, -0.5, 0.5, -1.0 / 6.0],
            [2.0 / 3.0, 0.0, -1.0, 0.5],
            [1.0 / 6.0, 0.5, 0.5, -0.5],
            [0.0, 0.0, 0.0, 1.0 / 6.0],
        ];
        for z in 0..cells[2] {
            for y in 0..cells[1] {
                for x in 0..cells[0] {
                    let mut c = vec![0.0; size];
                    for k in 0..degrees[2] {
                        for j in 0..degrees[1] {
                            for i in 0..degrees[0] {
                                c[(k * degrees[1] + j) * degrees[0] + i] =
                                    self.coefficients[((z + k) * self.coefficient_dims[1] + y + j)
                                        * self.coefficient_dims[0]
                                        + x
                                        + i];
                            }
                        }
                    }
                    for axis in 0..3 {
                        if degrees[axis] == 1 {
                            continue;
                        }
                        let stride = [1, degrees[0], degrees[0] * degrees[1]][axis];
                        for i in 0..size {
                            if i / stride % 4 != 0 {
                                continue;
                            }
                            let input = std::array::from_fn::<_, 4, _>(|j| c[i + j * stride]);
                            for p in 0..4 {
                                c[i + p * stride] = (0..4).map(|j| input[j] * POWERS[j][p]).sum();
                            }
                        }
                    }
                    let start = ((z * cells[1] + y) * cells[0] + x) * size;
                    for (out, value) in coefficients[start..start + size].iter_mut().zip(c) {
                        *out = value;
                    }
                }
            }
        }
        (cells, degrees, coefficients)
    }
}

impl Field for GridField {
    fn spatial_scale_mm(&self) -> Option<f64> {
        [
            (self.array.nx, self.array.dx_mm),
            (self.array.ny, self.array.dy_mm),
            (self.array.nz, self.array.dz_mm),
        ]
        .into_iter()
        .filter_map(|(n, h)| (n > 1).then_some(h))
        .min_by(f64::total_cmp)
    }

    fn sample(&self, position_mm: Vector, _time_us: f64) -> FieldSample {
        let (q, reflection, sign) = self.coordinates(position_mm);
        let dims = [self.array.nx, self.array.ny, self.array.nz];
        let spacing = [self.array.dx_mm, self.array.dy_mm, self.array.dz_mm];
        let (v, gradient) = if let Some(boundary) = &self.boundary {
            boundary.sample(std::array::from_fn(|a| q[a] / spacing[a]))
        } else {
            self.sample_cubic(q, dims, spacing)
        };
        let mut electric =
            std::array::from_fn(|a| -sign * reflection[a] * gradient[a] / spacing[a]);
        if self.array.symmetry == Symmetry::Cylindrical {
            let radius = position_mm[1].hypot(position_mm[2]);
            let radial = electric[1];
            electric[1] = if radius == 0.0 {
                0.0
            } else {
                radial * position_mm[1] / radius
            };
            electric[2] = if radius == 0.0 {
                0.0
            } else {
                radial * position_mm[2] / radius
            };
        }
        FieldSample {
            potential_v: sign * v,
            electric_v_per_mm: electric,
            magnetic_gauss: [0.0; 3],
        }
    }

    fn status(&self, position_mm: Vector) -> Option<SplatReason> {
        let (q, _, _) = self.coordinates(position_mm);
        let dims = [self.array.nx, self.array.ny, self.array.nz];
        let h = [self.array.dx_mm, self.array.dy_mm, self.array.dz_mm];
        let mut node = [0; 3];
        for a in 0..3 {
            // A single-point axis represents translational invariance.
            if dims[a] == 1 {
                continue;
            }
            let u = q[a] / h[a];
            if !(0.0..=(dims[a] - 1) as f64).contains(&u) {
                return Some(SplatReason::OutsideGrid);
            }
            node[a] = if self.node_cell_collisions {
                (u.floor() as usize).min(dims[a] - 2)
            } else {
                u.round() as usize
            };
        }
        let index = (node[2] * dims[1] + node[1]) * dims[0] + node[0];
        if let Some(cells) = &self.solid_cells {
            let d = dims.map(|n| n.saturating_sub(1).max(1));
            if cells[(node[2] * d[1] + node[1]) * d[0] + node[0]] {
                return Some(SplatReason::Electrode { index });
            }
        }
        if self.node_cell_collisions {
            let counts = dims.map(|n| if n > 1 { 2 } else { 1 });
            for z in 0..counts[2] {
                for y in 0..counts[1] {
                    for x in 0..counts[0] {
                        let p = [node[0] + x, node[1] + y, node[2] + z];
                        let i = (p[2] * dims[1] + p[1]) * dims[0] + p[0];
                        if !self.array.electrodes[i] || self.transparent_node(p) {
                            return None;
                        }
                    }
                }
            }
            return Some(SplatReason::Electrode { index });
        }
        (self.array.electrodes[index] && !self.transparent_node(node))
            .then_some(SplatReason::Electrode { index })
    }

    fn intersection(&self, from: Vector, to: Vector) -> Option<(f64, SplatReason)> {
        self.intersect(from, to, true, false)
    }

    fn launch_intersection(&self, from: Vector, to: Vector) -> Option<(f64, SplatReason)> {
        self.intersect(from, to, true, true)
    }
}

impl GridField {
    fn sample_cubic(&self, q: Vector, dims: [usize; 3], spacing: Vector) -> (f64, Vector) {
        let mut bases = [[0.0; 4]; 3];
        let mut derivatives = [[0.0; 4]; 3];
        let mut starts = [0; 3];
        let mut counts = [4; 3];
        for axis in 0..3 {
            if dims[axis] == 1 {
                counts[axis] = 1;
                bases[axis][0] = 1.0;
                continue;
            }
            let coordinate = q[axis] / spacing[axis];
            let cell = coordinate.floor().max(0.0).min((dims[axis] - 2) as f64) as usize;
            let t = coordinate - cell as f64;
            starts[axis] = cell;
            bases[axis] = [
                (1.0 - t).powi(3) / 6.0,
                (3.0 * t.powi(3) - 6.0 * t * t + 4.0) / 6.0,
                (-3.0 * t.powi(3) + 3.0 * t * t + 3.0 * t + 1.0) / 6.0,
                t.powi(3) / 6.0,
            ];
            derivatives[axis] = [
                -0.5 * (1.0 - t).powi(2),
                1.5 * t * t - 2.0 * t,
                -1.5 * t * t + t + 0.5,
                0.5 * t * t,
            ];
        }
        let mut v = 0.0;
        let mut gradient = [0.0; 3];
        for k in 0..counts[2] {
            for j in 0..counts[1] {
                for i in 0..counts[0] {
                    let index = ((starts[2] + k) * self.coefficient_dims[1] + starts[1] + j)
                        * self.coefficient_dims[0]
                        + starts[0]
                        + i;
                    let c = self.coefficients[index];
                    v += c * bases[0][i] * bases[1][j] * bases[2][k];
                    gradient[0] += c * derivatives[0][i] * bases[1][j] * bases[2][k];
                    gradient[1] += c * bases[0][i] * derivatives[1][j] * bases[2][k];
                    gradient[2] += c * bases[0][i] * bases[1][j] * derivatives[2][k];
                }
            }
        }
        (v, gradient)
    }
}

impl GridField {
    /// Electrode collision on a segment, allowing field-free travel outside PA.
    pub fn electrode_intersection(&self, from: Vector, to: Vector) -> Option<(f64, SplatReason)> {
        self.intersect(from, to, false, false)
    }

    pub(crate) fn intersect(
        &self,
        from: Vector,
        to: Vector,
        stop_at_edge: bool,
        launch: bool,
    ) -> Option<(f64, SplatReason)> {
        let mut departing =
            launch && matches!(self.status(from), Some(SplatReason::Electrode { .. }));
        let mut status = |p| {
            let s = self
                .status(p)
                .filter(|s| stop_at_edge || *s != SplatReason::OutsideGrid);
            if departing && matches!(s, Some(SplatReason::Electrode { .. })) {
                return None;
            }
            departing = false;
            s
        };
        if let Some(reason) = status(from) {
            return Some((0.0, reason));
        }
        // Partition at every occupied-cell boundary (or half-grid voxel face
        // for the explicit historical convention), including thin bodies
        // missed by both endpoints and all RK stages.
        let dims = [self.array.nx, self.array.ny, self.array.nz];
        let h = [self.array.dx_mm, self.array.dy_mm, self.array.dz_mm];
        let mirrors = [
            self.array.mirrors.x,
            self.array.mirrors.y,
            self.array.mirrors.z,
        ];
        let delta: Vector = std::array::from_fn(|a| to[a] - from[a]);
        let mut cuts = vec![0.0, 1.0];
        let offset = if self.node_cell_collisions { 0.0 } else { 0.5 };
        for a in 0..3 {
            if dims[a] == 1 {
                continue;
            }
            if a == 1 && self.array.symmetry == Symmetry::Cylindrical {
                let aa = delta[1] * delta[1] + delta[2] * delta[2];
                if aa == 0.0 {
                    continue;
                }
                let bb = from[1] * delta[1] + from[2] * delta[2];
                let closest = (-bb / aa).clamp(0.0, 1.0);
                let rmin = (from[1] + closest * delta[1]).hypot(from[2] + closest * delta[2]);
                let rmax = from[1].hypot(from[2]).max(to[1].hypot(to[2]));
                let low = (rmin / h[a] - offset).ceil().max(0.0) as usize;
                let high = (rmax / h[a] - offset).floor().min((dims[a] - 2) as f64) as usize;
                if low <= high {
                    for j in low..=high {
                        radial_cuts(&mut cuts, from, delta, (j as f64 + offset) * h[a]);
                    }
                }
                radial_cuts(&mut cuts, from, delta, (dims[a] - 1) as f64 * h[a]);
            } else if delta[a] != 0.0 {
                let signs: &[f64] = if mirrors[a] { &[-1.0, 1.0] } else { &[1.0] };
                for &sign in signs {
                    let min = (sign * from[a]).min(sign * to[a]);
                    let max = (sign * from[a]).max(sign * to[a]);
                    let low = (min / h[a] - offset).ceil().max(0.0) as usize;
                    let high = (max / h[a] - offset).floor().min((dims[a] - 2) as f64) as isize;
                    if high >= low as isize {
                        for j in low..=high as usize {
                            push_cut(
                                &mut cuts,
                                (sign * (j as f64 + offset) * h[a] - from[a]) / delta[a],
                            );
                        }
                    }
                    push_cut(
                        &mut cuts,
                        (sign * (dims[a] - 1) as f64 * h[a] - from[a]) / delta[a],
                    );
                }
                push_cut(&mut cuts, -from[a] / delta[a]);
            }
        }
        cuts.sort_unstable_by(f64::total_cmp);
        cuts.dedup();
        for pair in cuts.windows(2) {
            let mid = 0.5 * (pair[0] + pair[1]);
            let p = std::array::from_fn(|a| from[a] + mid * delta[a]);
            if let Some(reason) = status(p) {
                return Some((pair[0], reason));
            }
        }
        status(to).map(|reason| (1.0, reason))
    }
}

fn push_cut(cuts: &mut Vec<f64>, t: f64) {
    if t > 0.0 && t < 1.0 {
        cuts.push(t);
    }
}

fn radial_cuts(cuts: &mut Vec<f64>, from: Vector, delta: Vector, radius: f64) {
    let a = delta[1] * delta[1] + delta[2] * delta[2];
    let b = from[1] * delta[1] + from[2] * delta[2];
    let c = from[1] * from[1] + from[2] * from[2] - radius * radius;
    let discriminant = b * b - a * c;
    if discriminant >= 0.0 {
        let root = discriminant.sqrt();
        push_cut(cuts, (-b - root) / a);
        push_cut(cuts, (-b + root) / a);
    }
}

fn prefilter(
    input: &[f64],
    dims: [usize; 3],
    axis: usize,
    even: bool,
    odd: bool,
) -> (Vec<f64>, [usize; 3]) {
    let n = dims[axis];
    let mut out_dims = dims;
    out_dims[axis] += 2;
    let mut output = vec![0.0; out_dims.iter().product()];
    let stride = [1, dims[0], dims[0] * dims[1]][axis];
    let out_stride = [1, out_dims[0], out_dims[0] * out_dims[1]][axis];
    let mut values = vec![0.0; n];
    let mut diagonal = vec![0.0; n];
    for z in 0..dims[2] {
        for y in 0..dims[1] {
            for x in 0..dims[0] {
                let point = [x, y, z];
                if point[axis] != 0 {
                    continue;
                }
                let base = (z * dims[1] + y) * dims[0] + x;
                for i in 0..n {
                    values[i] = input[base + i * stride];
                }
                let slope = |reverse: bool| {
                    let f = |i: usize| values[if reverse { n - 1 - i } else { i }];
                    let d = match n {
                        2 => f(1) - f(0),
                        3 => (-3.0 * f(0) + 4.0 * f(1) - f(2)) / 2.0,
                        _ => (-11.0 * f(0) + 18.0 * f(1) - 9.0 * f(2) + 2.0 * f(3)) / 6.0,
                    };
                    if reverse { -d } else { d }
                };
                let left = if even { 0.0 } else { slope(false) };
                let right = slope(true);
                for value in &mut values {
                    *value *= 6.0;
                }
                values[0] = if odd {
                    values[0] / 6.0
                } else {
                    values[0] / 2.0 + left
                };
                values[n - 1] = values[n - 1] / 2.0 - right;
                diagonal.fill(4.0);
                diagonal[0] = if odd { 1.0 } else { 2.0 };
                diagonal[n - 1] = 2.0;
                for i in 1..n {
                    let upper = if i == 1 && odd { 0.0 } else { 1.0 };
                    let factor = 1.0 / diagonal[i - 1];
                    diagonal[i] -= factor * upper;
                    values[i] -= factor * values[i - 1];
                }
                values[n - 1] /= diagonal[n - 1];
                for i in (0..n - 1).rev() {
                    let upper = if i == 0 && odd { 0.0 } else { 1.0 };
                    values[i] = (values[i] - upper * values[i + 1]) / diagonal[i];
                }
                let out_base = (z * out_dims[1] + y) * out_dims[0] + x;
                output[out_base] = if odd {
                    -values[1]
                } else {
                    values[1] - 2.0 * left
                };
                for i in 0..n {
                    output[out_base + (i + 1) * out_stride] = values[i];
                }
                output[out_base + (n + 1) * out_stride] = values[n - 2] + 2.0 * right;
            }
        }
    }
    (output, out_dims)
}

/// One unit-voltage solution and its time-dependent voltage (absolute us).
pub struct BasisPotential {
    pub grid: GridField,
    pub voltage: Box<dyn Fn(f64) -> f64 + Send + Sync>,
}

/// SIMION-style fast adjustment without rebuilding a combined grid.
pub struct Superposition {
    bases: Vec<BasisPotential>,
    magnetic: Box<dyn Fn(Vector, f64) -> Vector + Send + Sync>,
}

impl Superposition {
    pub fn new(bases: Vec<BasisPotential>) -> Result<Self, FlyError> {
        let first = bases
            .first()
            .ok_or(FlyError("superposition needs at least one basis"))?;
        if bases
            .iter()
            .any(|basis| !first.grid.compatible(&basis.grid))
        {
            return Err(FlyError(
                "basis grids must share geometry, spacing, symmetry and electrode mask",
            ));
        }
        Ok(Self {
            bases,
            magnetic: Box::new(|_, _| [0.0; 3]),
        })
    }

    pub fn with_magnetic(
        mut self,
        field: impl Fn(Vector, f64) -> Vector + Send + Sync + 'static,
    ) -> Self {
        self.magnetic = Box::new(field);
        self
    }
}

impl Field for Superposition {
    fn spatial_scale_mm(&self) -> Option<f64> {
        self.bases[0].grid.spatial_scale_mm()
    }

    fn sample(&self, position_mm: Vector, time_us: f64) -> FieldSample {
        let mut result = FieldSample {
            magnetic_gauss: (self.magnetic)(position_mm, time_us),
            ..FieldSample::default()
        };
        for basis in &self.bases {
            let voltage = (basis.voltage)(time_us);
            let sample = basis.grid.sample(position_mm, time_us);
            result.potential_v += voltage * sample.potential_v;
            for a in 0..3 {
                result.electric_v_per_mm[a] += voltage * sample.electric_v_per_mm[a];
            }
        }
        result
    }

    fn status(&self, position_mm: Vector) -> Option<SplatReason> {
        self.bases[0].grid.status(position_mm)
    }

    fn intersection(&self, from: Vector, to: Vector) -> Option<(f64, SplatReason)> {
        self.bases[0].grid.intersection(from, to)
    }

    fn launch_intersection(&self, from: Vector, to: Vector) -> Option<(f64, SplatReason)> {
        self.bases[0].grid.launch_intersection(from, to)
    }
}

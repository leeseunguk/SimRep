//! Symmetric linear ghost-fluid Dirichlet boundaries on Cartesian grids.
//!
//! A cut edge replaces c*(u_j-u_i) by (c/theta)*(V-u_i). Free/free
//! coefficients are unchanged, so -W L remains symmetric positive definite.
//! Small cuts are removed by fixing nodes within 2*h_min^2/L of a surface,
//! never by clamping theta to a constant fraction of a cell.

use rayon::prelude::*;
use simrep_geom::{Electrode, Point, SignedDistance};

use crate::mg::{self, Axis, Grid, Options, Report, SolveError};

#[derive(Clone, Debug)]
pub(crate) struct CutRow {
    pub node: usize,
    /// Physical unweighted free-neighbor coefficients; cut links are zero.
    pub neighbors: [f64; 6],
    pub boundary: [f64; 6],
    pub voltage: [f64; 6],
}

impl CutRow {
    pub fn diagonal(&self) -> f64 {
        self.neighbors.iter().chain(&self.boundary).sum()
    }

    pub fn source(&self, gauge: f64) -> f64 {
        self.boundary
            .iter()
            .zip(self.voltage)
            .map(|(a, v)| a * (v - gauge))
            .sum()
    }

    pub fn residual(&self, x: &[f64], strides: [usize; 3], gauge: f64) -> f64 {
        let mut r = 0.0;
        for s in 0..6 {
            if self.neighbors[s] != 0.0 {
                let j = self
                    .node
                    .wrapping_add_signed(strides[s / 2] as isize * if s % 2 == 0 { -1 } else { 1 });
                r += self.neighbors[s] * (x[j] - x[self.node]);
            }
            if self.boundary[s] != 0.0 {
                r += self.boundary[s] * (self.voltage[s] - gauge - x[self.node]);
            }
        }
        r
    }
}

/// Prepared continuous geometry. Preparation is reusable and is not solve time.
/// `origin` is the physical coordinate of node (0,0,0); cylindrical coordinates
/// are (r,z,0), with origin[0] equal to Grid's radial_origin.
#[derive(Clone)]
pub struct CutCells {
    pub grid: Grid,
    pub origin: Point,
    pub fixed: Vec<bool>,
    pub values: Vec<f64>,
    pub snap_distance: f64,
    pub(crate) slots: Vec<u32>,
    pub(crate) rows: Vec<CutRow>,
}

impl CutCells {
    pub fn new(grid: Grid, origin: Point, electrodes: &[Electrode]) -> Result<Self, SolveError> {
        Self::new_with_root_tolerance(grid, origin, electrodes, f64::INFINITY)
    }

    /// Prepare geometry with a stricter root-distance stopping tolerance.
    /// The quadratic snap rule is unchanged. The default root tolerance is
    /// still an upper bound; use this for scaled conservative importer SDFs.
    pub fn new_with_root_tolerance(
        grid: Grid,
        origin: Point,
        electrodes: &[Electrode],
        maximum_root_distance: f64,
    ) -> Result<Self, SolveError> {
        Self::prepare(grid, origin, electrodes, maximum_root_distance, false)
    }

    /// Local adaptive subdomain; its Dirichlet anchor may be in the base grid.
    pub fn new_patch(
        grid: Grid,
        origin: Point,
        electrodes: &[Electrode],
    ) -> Result<Self, SolveError> {
        Self::prepare(grid, origin, electrodes, f64::INFINITY, true)
    }

    fn prepare(
        grid: Grid,
        origin: Point,
        electrodes: &[Electrode],
        maximum_root_distance: f64,
        unanchored: bool,
    ) -> Result<Self, SolveError> {
        let count = grid
            .dims
            .iter()
            .try_fold(1usize, |n, d| n.checked_mul(*d))
            .ok_or_else(|| mg::invalid("grid size overflows usize"))?;
        if grid.dims[0] < 2
            || grid.dims[1] < 2
            || grid.dims[2] == 0
            || !grid.spacing.iter().all(|h| {
                h.is_finite() && *h > 0.0 && (1.0 / (h * h)).is_finite() && 1.0 / (h * h) > 0.0
            })
            || !origin.iter().all(|x| x.is_finite())
            || electrodes.is_empty()
            || !electrodes.iter().all(|e| e.voltage.is_finite())
            || maximum_root_distance <= 0.0
            || maximum_root_distance.is_nan()
            || count > u32::MAX as usize
        {
            return Err(mg::invalid("invalid SDF grid, origin, or electrodes"));
        }
        if let mg::Geometry::Cylindrical { radial_origin } = grid.geometry
            && (grid.dims[2] != 1 || radial_origin != origin[0] || radial_origin < 0.0)
        {
            return Err(mg::invalid(
                "cylindrical SDF origin must equal nonnegative radial_origin",
            ));
        }
        let h = (0..3)
            .filter(|&a| grid.dims[a] > 1)
            .map(|a| grid.spacing[a])
            .fold(f64::INFINITY, f64::min);
        let length = (0..3)
            .map(|a| (grid.dims[a] - 1) as f64 * grid.spacing[a])
            .fold(0.0, f64::max);
        if !length.is_finite() {
            return Err(mg::invalid("SDF domain extent must be finite"));
        }
        let snap_distance = 2.0 * h * h / length;
        let at = |i| {
            let c = mg::coords(i, grid.dims);
            std::array::from_fn(|a| origin[a] + c[a] as f64 * grid.spacing[a])
        };
        let masks = electrodes
            .par_iter()
            .map(|electrode| {
                let mut mask = vec![false; count];
                electrode
                    .shape
                    .conductor_mask(grid.dims, origin, grid.spacing, snap_distance, &mut mask)
                    .map_err(mg::invalid)?;
                Ok(mask)
            })
            .collect::<Result<Vec<_>, SolveError>>()?;
        let owners = (0..count)
            .into_par_iter()
            .map(|i| {
                let mut owner: Option<usize> = None;
                for (e, mask) in masks.iter().enumerate() {
                    if !mask[i] {
                        continue;
                    }
                    if let Some(previous) = owner
                        && electrodes[previous].voltage != electrodes[e].voltage
                    {
                        return Err(mg::invalid("electrodes at different voltages overlap"));
                    }
                    owner = Some(e);
                }
                Ok(owner)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let fixed: Vec<_> = owners.iter().map(Option::is_some).collect();
        let values: Vec<_> = owners
            .iter()
            .map(|e| e.map_or(0.0, |e| electrodes[e].voltage))
            .collect();
        if !unanchored && !fixed.iter().any(|&f| f) {
            return Err(mg::invalid("SDF grid must contain a conductor node"));
        }
        let axes: [Axis; 3] = std::array::from_fn(|a| {
            let radial = match grid.geometry {
                mg::Geometry::Cylindrical { radial_origin } if a == 0 => Some(radial_origin),
                _ => None,
            };
            Axis::new(grid.dims[a], grid.spacing[a], radial)
        });
        let strides = [1, grid.dims[0], grid.dims[0] * grid.dims[1]];
        let edge = grid.spacing.into_iter().fold(0.0_f64, f64::max);
        let root_tol = (snap_distance / 64.0).min(maximum_root_distance);
        let mut near = vec![false; count];
        for electrode in electrodes {
            electrode.shape.mark_near_nodes(
                grid.dims,
                origin,
                grid.spacing,
                edge + root_tol,
                &mut near,
            );
        }
        let rows: Vec<_> = (0..count)
            .into_par_iter()
            .filter_map(|i| {
                if fixed[i] || !near[i] {
                    return None;
                }
                let c = mg::coords(i, grid.dims);
                let p = at(i);
                let mut row = CutRow {
                    node: i,
                    neighbors: [0.0; 6],
                    boundary: [0.0; 6],
                    voltage: [0.0; 6],
                };
                let mut cut = false;
                for s in 0..6 {
                    let a = s / 2;
                    let plus = s % 2 == 1;
                    if (!plus && c[a] == 0) || (plus && c[a] + 1 == grid.dims[a]) {
                        continue;
                    }
                    let coefficient = if plus {
                        axes[a].plus[c[a]]
                    } else {
                        axes[a].minus[c[a]]
                    };
                    let j = i.wrapping_add_signed(strides[a] as isize * if plus { 1 } else { -1 });
                    let end = at(j);
                    let mut hit: Option<(f64, f64)> = None;
                    let root_tol = (snap_distance / 64.0).min(maximum_root_distance);
                    for electrode in electrodes {
                        // Free starts are outside every electrode. A segment with
                        // no triangle inside the root tolerance cannot be a hit.
                        if electrode.shape.segment_clear(p, end, root_tol) {
                            continue;
                        }
                        if let Some(t) = intersection(
                            electrode.shape.as_ref(),
                            p,
                            end,
                            grid.spacing[a],
                            root_tol,
                        ) && hit.is_none_or(|(previous, _)| t < previous)
                        {
                            hit = Some((t, electrode.voltage));
                        }
                    }
                    // A snapped conductor has been moved by at most O(h^2/L).
                    if hit.is_none()
                        && let Some(e) = owners[j]
                    {
                        hit = Some((1.0, electrodes[e].voltage));
                    }
                    if let Some((theta, voltage)) = hit {
                        row.boundary[s] = coefficient / theta;
                        row.voltage[s] = voltage;
                        cut = true;
                    } else {
                        row.neighbors[s] = coefficient;
                    }
                }
                cut.then_some(row)
            })
            .collect();
        let mut slots = vec![0; count];
        for (slot, row) in rows.iter().enumerate() {
            slots[row.node] = slot as u32 + 1;
        }
        Ok(Self {
            grid,
            origin,
            fixed,
            values,
            snap_distance,
            slots,
            rows,
        })
    }

    pub fn cut_nodes(&self) -> usize {
        self.rows.len()
    }

    /// Reuse the prepared geometry with different conductor voltages.
    /// The mapping is applied only to Dirichlet data, not free-node guesses.
    pub fn with_voltages(&self, voltage: impl Fn(f64) -> f64) -> Result<Self, SolveError> {
        let mut result = self.clone();
        for (value, &fixed) in result.values.iter_mut().zip(&result.fixed) {
            if fixed {
                *value = voltage(*value);
                if !value.is_finite() {
                    return Err(mg::invalid("electrode voltage must be finite"));
                }
            }
        }
        for row in &mut result.rows {
            for s in 0..6 {
                if row.boundary[s] != 0.0 {
                    row.voltage[s] = voltage(row.voltage[s]);
                    if !row.voltage[s].is_finite() {
                        return Err(mg::invalid("electrode voltage must be finite"));
                    }
                }
            }
        }
        Ok(result)
    }

    /// Electric field at a free node. Quadratic unequal-distance differences
    /// use the actual surface voltage when an axis link is cut. Mirror faces
    /// have zero normal field. Conductor nodes return None.
    pub fn electric_field(&self, potential: &[f64], i: usize) -> Option<Point> {
        assert_eq!(potential.len(), self.values.len());
        if self.fixed[i] {
            return None;
        }
        let c = mg::coords(i, self.grid.dims);
        let strides = [1, self.grid.dims[0], self.grid.dims[0] * self.grid.dims[1]];
        let row = (self.slots[i] > 0).then(|| &self.rows[self.slots[i] as usize - 1]);
        Some(std::array::from_fn(|a| {
            if c[a] == 0 || c[a] + 1 == self.grid.dims[a] {
                return 0.0;
            }
            let h = self.grid.spacing[a];
            let mut distances = [h; 2];
            let mut values = [potential[i - strides[a]], potential[i + strides[a]]];
            if let Some(row) = row {
                for side in 0..2 {
                    let s = 2 * a + side;
                    if row.boundary[s] > 0.0 {
                        let radial = match self.grid.geometry {
                            mg::Geometry::Cylindrical { radial_origin } if a == 0 => {
                                let r = radial_origin / h + c[a] as f64;
                                if side == 0 {
                                    1.0 - 0.5 / r
                                } else {
                                    1.0 + 0.5 / r
                                }
                            }
                            _ => 1.0,
                        };
                        distances[side] = radial / (h * row.boundary[s]);
                        values[side] = row.voltage[s];
                    }
                }
            }
            let [hm, hp] = distances;
            -(hp / (hm * (hm + hp)) * (potential[i] - values[0])
                + hm / (hp * (hm + hp)) * (values[1] - potential[i]))
        }))
    }

    pub fn solve(&self, potential: &mut [f64], options: Options) -> Result<Report, SolveError> {
        mg::solve_cut(potential, self, options)
    }
}

/// First surface intersection on a segment, as a fraction in [0,1].
/// `tolerance` is a positive distance in the geometry's length unit.
/// The shape must satisfy the same conservative-distance contract as CutCells.
/// A start point inside/on the conductor returns zero.
pub fn surface_fraction(
    shape: &dyn SignedDistance,
    start: Point,
    end: Point,
    tolerance: f64,
) -> Option<f64> {
    if shape.segment_clear(start, end, tolerance) {
        let start_distance = shape.distance(start);
        if start_distance > 0.0 {
            return None;
        }
        if start_distance <= 0.0 {
            return Some(0.0);
        }
    }
    if shape.distance(start) <= 0.0 {
        return Some(0.0);
    }
    let length = (0..3)
        .map(|a| (end[a] - start[a]).powi(2))
        .sum::<f64>()
        .sqrt();
    if length == 0.0 {
        return None;
    }
    intersection(shape, start, end, length, tolerance)
}

/// Sphere tracing detects even an electrode crossed between two free endpoints.
/// Once bracketed, bisection gives a geometry root independent of SDF magnitude.
fn intersection(
    shape: &dyn SignedDistance,
    p: Point,
    q: Point,
    length: f64,
    tolerance: f64,
) -> Option<f64> {
    let point = |t: f64| std::array::from_fn(|a| p[a] + t * (q[a] - p[a]));
    let mut lo = 0.0;
    let mut d = shape.distance(p);
    // A positive start farther than the segment, or a segment that cannot
    // reach the surface, has no root. An interior start (d <= 0) still
    // returns the existing bracket below.
    if d > 0.0
        && (d > length
            || shape.segment_clear(p, q, tolerance)
            || shape
                .segment_distance_bounds(p, q)
                .is_some_and(|bounds| bounds[0] > tolerance))
    {
        return None;
    }
    let end_distance = shape.distance(q);
    if end_distance >= 0.0 && end_distance <= tolerance {
        let near_end = shape.distance(point(0.999));
        // At tangency, sphere tracing makes O(1/sqrt(tolerance)) tiny
        // advances. A connected sublevel set supplies a safe bracket instead.
        if near_end - end_distance < 1e-6 * length && shape.segment_single_interval(p, q, tolerance)
        {
            let mut upper = 1.0;
            for _ in 0..48 {
                let mid = 0.5 * (lo + upper);
                if shape.distance(point(mid)) > tolerance {
                    lo = mid;
                } else {
                    upper = mid;
                }
            }
            return Some(0.5 * (lo + upper));
        }
    }
    while lo < 1.0 {
        let hi = (lo + d / length).min(1.0);
        if hi <= lo {
            return Some(lo);
        }
        let next = shape.distance(point(hi));
        if next <= 0.0 {
            let mut upper = hi;
            for _ in 0..48 {
                let mid = 0.5 * (lo + upper);
                if shape.distance(point(mid)) > 0.0 {
                    lo = mid;
                } else {
                    upper = mid;
                }
            }
            return Some(0.5 * (lo + upper));
        }
        if hi == 1.0 {
            return None;
        }
        if next <= tolerance {
            return Some(hi);
        }
        lo = hi;
        d = next;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use simrep_geom::Sdf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn tangent_endpoint_brackets_without_marching_tiny_steps() {
        struct Counted {
            calls: AtomicUsize,
            shape: Sdf,
        }
        impl SignedDistance for Counted {
            fn distance(&self, p: Point) -> f64 {
                self.calls.fetch_add(1, Ordering::Relaxed);
                self.shape.distance(p)
            }
            fn segment_single_interval(&self, from: Point, to: Point, level: f64) -> bool {
                self.shape.segment_single_interval(from, to, level)
            }
        }
        let shape = Counted {
            calls: AtomicUsize::new(0),
            shape: Sdf::Cylinder {
                center: [0.0; 3],
                radius: 1.0,
                axis: 2,
            },
        };
        let tolerance = 1e-9;
        let hit = surface_fraction(&shape, [0.5, 1.0, 0.0], [0.0, 1.0, 0.0], tolerance).unwrap();
        let expected = 1.0 - ((1.0 + tolerance).powi(2) - 1.0).sqrt() / 0.5;
        assert!((hit - expected).abs() < 1e-9);
        assert!(shape.calls.load(Ordering::Relaxed) < 64);
    }

    #[test]
    fn voltage_mapping_reuses_identical_cut_geometry_and_solution() {
        let grid = Grid {
            dims: [9, 5, 1],
            spacing: [0.125; 3],
            geometry: mg::Geometry::Planar,
        };
        let electrodes = |left, right| {
            [
                Electrode::new(
                    left,
                    Sdf::Plane {
                        normal: [1.0, 0.0, 0.0],
                        offset: 0.23,
                    },
                ),
                Electrode::new(
                    right,
                    Sdf::Plane {
                        normal: [-1.0, 0.0, 0.0],
                        offset: -0.79,
                    },
                ),
            ]
        };
        let prepared = CutCells::new(grid, [0.0; 3], &electrodes(3.0, 7.0)).unwrap();
        for (left, right) in [(10000.0, 0.0), (0.0, 10000.0), (-7.0, 19.0)] {
            let reused = prepared
                .with_voltages(|v| if v == 3.0 { left } else { right })
                .unwrap();
            let rebuilt = CutCells::new(grid, [0.0; 3], &electrodes(left, right)).unwrap();
            assert_eq!(reused.fixed, rebuilt.fixed);
            assert_eq!(reused.values, rebuilt.values);
            assert_eq!(reused.slots, rebuilt.slots);
            assert_eq!(reused.rows.len(), rebuilt.rows.len());
            for (a, b) in reused.rows.iter().zip(&rebuilt.rows) {
                assert_eq!(a.node, b.node);
                assert_eq!(a.neighbors, b.neighbors);
                assert_eq!(a.boundary, b.boundary);
                assert_eq!(a.voltage, b.voltage);
            }
            let mut a = reused.values.clone();
            let mut b = rebuilt.values.clone();
            reused.solve(&mut a, Options::default()).unwrap();
            rebuilt.solve(&mut b, Options::default()).unwrap();
            assert_eq!(a, b);
        }
        assert!(prepared.with_voltages(|_| f64::NAN).is_err());
    }
}

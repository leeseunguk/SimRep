//! Shared coaxial coil-map construction and calibration. The GPU upload path
//! (gpu/coil_map.rs) and `MagneticField::prepare_coil_map` both use this single
//! implementation: 0.0625 mm domains, C2 jets, axis limit and the same
//! deterministic calibration probes.
use rayon::prelude::*;
use simrep_core::{PotentialArray, Symmetry};

use super::{Coil, MagneticField};
use crate::{Field, FieldSample, FlyError, GridField, SplatReason, Vector, cross, dot};

/// Grid step shared by the CPU and GPU map paths.
const MAP_STEP_MM: f64 = 0.0625;

/// Sampled field residuals, not a global analytic error bound. Full-flight
/// endpoint and TOF noninferiority remain the independent acceptance gate.
#[derive(Clone, Debug, serde::Serialize)]
pub struct CoilMapCalibration {
    pub nodes: usize,
    pub max_absolute_gauss: f64,
    pub max_relative: f64,
    pub accepted: bool,
}

struct Region {
    lo: f64,
    hi: f64,
    radius: f64,
    spacing: [f64; 2],
    dims: [usize; 2],
    // Interleaved axial B and Br/r; nine derivative jets per node.
    jets: Vec<[f64; 2]>,
}

/// Common geometry of a coaxial hoop group: its axis frame and the
/// filament-free axial/radial rectangles that admit a two-component map.
pub(crate) struct CoilMapPlan {
    pub(crate) origin: Vector,
    pub(crate) axis: Vector,
    radial: Vector,
    pub(crate) domains: Vec<(f64, f64, f64)>,
}

impl CoilMapPlan {
    /// Detect >=16 strictly coaxial loops and derive the map rectangles from
    /// the same bounds convention used by the GPU upload. `None` falls back
    /// to exact Biot-Savart everywhere.
    pub(crate) fn new(magnetic: &MagneticField, bounds: Option<[Vector; 2]>) -> Option<Self> {
        if magnetic.coils.len() < 16 {
            return None;
        }
        let Some(Coil::Loop {
            center_mm: origin,
            normal: axis,
            radius_mm,
            ..
        }) = magnetic.coils.first()
        else {
            return None;
        };
        let origin = *origin;
        let axis = *axis;
        let mut radius = *radius_mm;
        let mut ends = [0.0_f64; 2];
        for coil in &magnetic.coils {
            let Coil::Loop {
                center_mm,
                normal,
                radius_mm,
                ..
            } = coil
            else {
                return None;
            };
            let delta = std::array::from_fn(|a| center_mm[a] - origin[a]);
            let x = dot(delta, axis);
            let perpendicular = std::array::from_fn(|a| delta[a] - x * axis[a]);
            if (dot(*normal, axis).abs() - 1.0).abs() > 1e-12
                || dot(perpendicular, perpendicular) > radius_mm.powi(2) * 1e-24
            {
                return None;
            }
            radius = radius.min(*radius_mm);
            ends[0] = ends[0].min(x);
            ends[1] = ends[1].max(x);
        }
        let seed = if axis[2].abs() < 0.9 {
            [0.0, 0.0, 1.0]
        } else {
            [0.0, 1.0, 0.0]
        };
        let radial = cross(axis, seed);
        let length = dot(radial, radial).sqrt();
        let radial = radial.map(|v| v / length);
        let mut extent = [ends[0] - 8.0 * radius, ends[1] + 8.0 * radius];
        let mut radial_max = 8.0 * radius;
        if let Some(b) = bounds {
            extent = [f64::INFINITY, f64::NEG_INFINITY];
            radial_max = 0.0;
            for corner in 0..8 {
                let delta = std::array::from_fn(|a| b[(corner >> a) & 1][a] - origin[a]);
                let x = dot(delta, axis);
                let r = std::array::from_fn(|a| delta[a] - x * axis[a]);
                extent[0] = extent[0].min(x);
                extent[1] = extent[1].max(x);
                radial_max = radial_max.max(dot(r, r).sqrt());
            }
        }
        let domains: Vec<_> = [
            (extent[0], extent[1], 0.6 * radius),
            (extent[0], ends[0] - 2.0 * radius, radial_max),
            (ends[1] + 2.0 * radius, extent[1], radial_max),
        ]
        .into_iter()
        .filter(|&(lo, hi, r)| hi > lo && r > 0.0)
        .collect();
        Some(Self {
            origin,
            axis,
            radial,
            domains,
        })
    }

    /// Sample every rectangle on its node grid and build both C2 component
    /// maps. Costs are identical for the CPU and GPU callers.
    pub(crate) fn build(
        &self,
        coils: &[Coil],
        spacing: f64,
    ) -> Result<Vec<CoilMapDomain>, FlyError> {
        self.domains
            .iter()
            .map(|&(lo, hi, rmax)| CoilMapDomain::new(self, coils, lo, hi, rmax, spacing))
            .collect()
    }
}

/// Exact coil sum in gauss; uniform/PA components do not enter the map.
fn exact_b(plan: &CoilMapPlan, coils: &[Coil], x: f64, r: f64) -> [f64; 2] {
    let point = std::array::from_fn(|a| plan.origin[a] + plan.axis[a] * x + plan.radial[a] * r);
    let mut b = [0.0; 3];
    for coil in coils {
        let next = coil.field_gauss(point);
        for a in 0..3 {
            b[a] += next[a];
        }
    }
    [dot(b, plan.axis), dot(b, plan.radial)]
}

/// One calibrated map rectangle with its two nodal-derivative grids.
pub(crate) struct CoilMapDomain {
    pub(crate) lo: f64,
    pub(crate) hi: f64,
    pub(crate) radius: f64,
    pub(crate) spacing: [f64; 2],
    pub(crate) dims: [usize; 2],
    grids: [GridField; 2],
}

impl CoilMapDomain {
    fn new(
        plan: &CoilMapPlan,
        coils: &[Coil],
        lo: f64,
        hi: f64,
        rmax: f64,
        spacing: f64,
    ) -> Result<Self, FlyError> {
        let nx = (((hi - lo) / spacing).ceil() as usize + 1).max(6);
        let nr = ((rmax / spacing).ceil() as usize + 1).max(6);
        let dx = (hi - lo) / (nx - 1) as f64;
        let dr = rmax / (nr - 1) as f64;
        let values: Vec<[f64; 2]> = (0..nx * nr)
            .into_par_iter()
            .map(|index| {
                let x = lo + (index % nx) as f64 * dx;
                let r = (index / nx) as f64 * dr;
                let b = exact_b(plan, coils, x, r);
                let factor = if r != 0.0 {
                    b[1] / r
                } else {
                    let point: Vector = std::array::from_fn(|a| plan.origin[a] + plan.axis[a] * x);
                    coils
                        .iter()
                        .map(|coil| {
                            let Coil::Loop {
                                center_mm,
                                normal,
                                radius_mm,
                                current_a,
                            } = coil
                            else {
                                unreachable!()
                            };
                            let delta = std::array::from_fn(|a| point[a] - center_mm[a]);
                            let z = dot(delta, *normal);
                            let a = radius_mm * radius_mm + z * z;
                            3.0 * std::f64::consts::PI * current_a * radius_mm * radius_mm * z
                                / (a * a * a.sqrt())
                        })
                        .sum()
                };
                [b[0], factor]
            })
            .collect();
        let component_grid = |component: usize| -> Result<GridField, FlyError> {
            let mut array = PotentialArray::new(nx, nr, 1);
            array.dx_mm = dx;
            array.dy_mm = dr;
            array.symmetry = Symmetry::Cylindrical;
            array.potentials = values.iter().map(|v| v[component]).collect();
            GridField::new_gpu_map(array)
        };
        Ok(Self {
            lo,
            hi,
            radius: rmax,
            spacing: [dx, dr],
            dims: [nx, nr],
            grids: [component_grid(0)?, component_grid(1)?],
        })
    }

    /// Shared node jets in the GPU upload layout: `(cells, degrees, planes)`.
    pub(crate) fn physics_data(&self, component: usize) -> ([usize; 3], [usize; 3], Vec<f64>) {
        let (cells, degrees, jets, _) = self.grids[component].gpu_physics_data(true);
        (cells, degrees, jets.into_owned())
    }

    /// The 512 deterministic off-node calibration probes, as x/r offsets.
    pub(crate) fn probes(&self) -> Vec<[f64; 2]> {
        let (hi, lo, rmax) = (self.hi, self.lo, self.radius);
        (0..512)
            .map(|i| {
                [
                    (i % 32) as f64 / 32.0 * (hi - lo) + 0.371 * (hi - lo) / 32.0,
                    (i / 32) as f64 / 16.0 * rmax + 0.613 * rmax / 16.0,
                ]
            })
            .collect()
    }

    /// One scalar map evaluation in the GPU probe convention: the cylindrical
    /// scalar grid sampled at `[x_offset, r, 0]`.
    pub(crate) fn grid_sample(&self, component: usize, q: [f64; 2]) -> f64 {
        self.grids[component]
            .sample([q[0], q[1], 0.0], 0.0)
            .potential_v
    }

    /// Compare `sampled` (as `[Bx, Br_over_r]` per probe) against the exact
    /// field at the same probes and apply the shared acceptance gates.
    pub(crate) fn calibrate(
        &self,
        plan: &CoilMapPlan,
        coils: &[Coil],
        sampled: &[[f64; 2]],
    ) -> CoilMapCalibration {
        let probes = self.probes();
        let mut absolute = 0.0_f64;
        let mut relative = 0.0_f64;
        let mut finite = true;
        for i in 0..probes.len() {
            let (x, r) = (self.lo + probes[i][0], probes[i][1]);
            let reference = exact_b(plan, coils, x, r);
            let sample = sampled[i];
            let error = (sample[0] - reference[0]).hypot(sample[1] * r - reference[1]);
            finite &= error.is_finite();
            absolute = absolute.max(error);
            relative = relative.max(error / reference[0].hypot(reference[1]).max(1e-30));
        }
        CoilMapCalibration {
            nodes: self.dims[0] * self.dims[1],
            max_absolute_gauss: absolute,
            max_relative: relative,
            accepted: finite && absolute <= 2e-9 && relative <= 5e-10,
        }
    }
}

/// A field borrowing its exact source, so public coil edits cannot stale a map.
/// Small/noncoaxial groups and points outside accepted regions stay exact.
pub struct PreparedMagneticField<'a> {
    source: &'a MagneticField,
    origin: Vector,
    axis: Vector,
    regions: Vec<Region>,
    calibration: Vec<CoilMapCalibration>,
}

impl<'a> PreparedMagneticField<'a> {
    /// Explicit exact Biot-Savart path, with no map construction.
    pub fn exact(source: &'a MagneticField) -> Self {
        Self {
            source,
            origin: [0.0; 3],
            axis: [1.0, 0.0, 0.0],
            regions: Vec::new(),
            calibration: Vec::new(),
        }
    }

    pub(super) fn new(
        source: &'a MagneticField,
        bounds: Option<[Vector; 2]>,
    ) -> Result<Self, FlyError> {
        let mut prepared = Self::exact(source);
        if bounds.is_some_and(|b| {
            (0..3).any(|a| !b[0][a].is_finite() || !b[1][a].is_finite() || b[0][a] >= b[1][a])
        }) {
            return Err(FlyError("coil map needs finite increasing bounds"));
        }
        let Some(plan) = CoilMapPlan::new(source, bounds) else {
            return Ok(prepared);
        };
        prepared.origin = plan.origin;
        prepared.axis = plan.axis;
        for domain in plan.build(&source.coils, MAP_STEP_MM)? {
            let mut region = Region {
                lo: domain.lo,
                hi: domain.hi,
                radius: domain.radius,
                spacing: domain.spacing,
                dims: domain.dims,
                jets: vec![[0.0; 2]; domain.dims[0] * domain.dims[1] * 9],
            };
            for component in 0..2 {
                let (_, _, jets) = domain.physics_data(component);
                for (node, value) in region.jets.iter_mut().zip(jets.iter()) {
                    node[component] = *value;
                }
            }
            let sampled: Vec<_> = domain
                .probes()
                .iter()
                .map(|q| region.sample(domain.lo + q[0], q[1]))
                .collect();
            let calibration = domain.calibrate(&plan, &source.coils, &sampled);
            let accepted = calibration.accepted;
            prepared.calibration.push(calibration);
            if accepted {
                prepared.regions.push(region);
            }
        }
        Ok(prepared)
    }

    pub fn calibration(&self) -> &[CoilMapCalibration] {
        &self.calibration
    }

    fn mapped(&self, p: Vector) -> Option<Vector> {
        if self.regions.is_empty() {
            return None;
        }
        let delta = std::array::from_fn(|a| p[a] - self.origin[a]);
        let x = dot(delta, self.axis);
        let radial: Vector = std::array::from_fn(|a| delta[a] - x * self.axis[a]);
        let r = dot(radial, radial).sqrt();
        let region = self
            .regions
            .iter()
            .find(|region| x >= region.lo && x <= region.hi && r <= region.radius)?;
        let b = region.sample(x, r);
        Some(std::array::from_fn(|a| {
            b[0] * self.axis[a] + b[1] * radial[a]
        }))
    }
}

impl Field for PreparedMagneticField<'_> {
    fn sample(&self, p: Vector, t: f64) -> FieldSample {
        self.source.sample_with_coils(p, t, self.mapped(p))
    }
    fn spatial_scale_mm(&self) -> Option<f64> {
        self.source.spatial_scale_mm()
    }
    fn status(&self, p: Vector) -> Option<SplatReason> {
        self.source.status(p)
    }
    fn intersection(&self, a: Vector, b: Vector) -> Option<(f64, SplatReason)> {
        self.source.intersection(a, b)
    }
    fn launch_intersection(&self, a: Vector, b: Vector) -> Option<(f64, SplatReason)> {
        self.source.launch_intersection(a, b)
    }
}

impl Region {
    fn sample(&self, x: f64, r: f64) -> [f64; 2] {
        let q = [(x - self.lo) / self.spacing[0], r / self.spacing[1]];
        let cell: [usize; 2] =
            std::array::from_fn(|a| (q[a].floor() as usize).min(self.dims[a] - 2));
        let weights = std::array::from_fn::<_, 2, _>(|a| hermite(q[a] - cell[a] as f64));
        let count = self.dims[0] * self.dims[1];
        let mut b = [0.0; 2];
        for j in 0..6 {
            let row = (cell[1] + j / 3) * self.dims[0] + cell[0];
            let mut axial = 0.0;
            let mut radial = 0.0;
            for (i, &weight) in weights[0].iter().enumerate() {
                let v = self.jets[(j % 3 * 3 + i % 3) * count + row + i / 3];
                axial += weight * v[0];
                radial += weight * v[1];
            }
            b[0] += weights[1][j] * axial;
            b[1] += weights[1][j] * radial;
        }
        b
    }
}

fn hermite(t: f64) -> [f64; 6] {
    [
        1.0 + t * t * t * (-10.0 + t * (15.0 - 6.0 * t)),
        t + t * t * t * (-6.0 + t * (8.0 - 3.0 * t)),
        t * t * (0.5 + t * (-1.5 + t * (1.5 - 0.5 * t))),
        t * t * t * (10.0 + t * (-15.0 + 6.0 * t)),
        t * t * t * (-4.0 + t * (7.0 - 3.0 * t)),
        t * t * t * (0.5 + t * (-1.0 + 0.5 * t)),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calibrated_map_preserves_axis_rotations_and_exact_fallback() {
        let mut source =
            MagneticField::solenoid([3.0, -50.0, 7.0], [3.0, 50.0, 7.0], 10.0, 0.7958, 100)
                .unwrap();
        source.uniform_gauss = [1.0, 2.0, 3.0];
        let prepared = source
            .prepare_coil_map(Some([[2.0, -150.0, 6.0], [4.0, 150.0, 8.0]]))
            .unwrap();
        assert_eq!(prepared.calibration().len(), 3);
        assert!(
            prepared.calibration().iter().all(|c| c.accepted),
            "{:?}",
            prepared.calibration()
        );
        for p in [[3.0, 0.0, 7.0], [3.001, 0.13, 7.0], [3.31, -0.71, 6.78]] {
            let mapped = prepared.sample(p, 0.0).magnetic_gauss;
            let exact = source.sample(p, 0.0).magnetic_gauss;
            let error = std::array::from_fn(|a| mapped[a] - exact[a]);
            assert!(dot(error, error).sqrt() < 2e-9);
        }
        let p = [13.5, 0.0, 7.0];
        assert!(prepared.mapped(p).is_none());
        assert_eq!(prepared.sample(p, 0.0), source.sample(p, 0.0));
        assert_eq!(prepared.status(p), source.status(p));
        assert_eq!(
            PreparedMagneticField::exact(&source).sample([3.31, 0.13, 7.0], 0.0),
            source.sample([3.31, 0.13, 7.0], 0.0)
        );
    }

    #[test]
    fn failed_calibration_and_noncoaxial_groups_stay_exact() {
        let mut source =
            MagneticField::solenoid([-4.0, 0.0, 0.0], [4.0, 0.0, 0.0], 10.0, 1e12, 16).unwrap();
        let bounds = Some([[-1.0; 3], [1.0; 3]]);
        {
            let prepared = source.prepare_coil_map(bounds).unwrap();
            assert!(!prepared.calibration()[0].accepted);
            assert!(prepared.regions.is_empty());
            assert_eq!(
                prepared.sample([0.31, 0.17, 0.0], 0.0),
                source.sample([0.31, 0.17, 0.0], 0.0)
            );
        }
        source
            .coils
            .push(Coil::segment([0.0; 3], [1.0; 3], 1.0).unwrap());
        let prepared = source.prepare_coil_map(bounds).unwrap();
        assert!(prepared.calibration().is_empty());
        assert_eq!(
            prepared.sample([0.31, 0.17, 0.0], 0.0),
            source.sample([0.31, 0.17, 0.0], 0.0)
        );
    }
}

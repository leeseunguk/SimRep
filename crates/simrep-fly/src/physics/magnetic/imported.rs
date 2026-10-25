//! Imported vector fields share the coil map's C2 scalar-component jets.
use simrep_core::{MirrorFlags, PotentialArray, Symmetry};

use crate::{Field, FieldSample, FlyError, GridField, SplatReason, Vector, dot};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OutsideMap {
    #[default]
    Error,
    Zero,
    Clamp,
}

/// Placement maps source mm into workbench mm: origin + scale * axes * p.
/// Columns of axes form a proper orthonormal rotation. Field magnitudes are
/// rotated, never multiplied by the geometric placement scale.
#[derive(Clone, Debug)]
pub struct MapPlacement {
    pub origin_mm: Vector,
    pub axes: [Vector; 3],
    pub scale: f64,
    pub outside: OutsideMap,
    /// Per-plane component parities: None=no mirror, Some([+/-1;3]).
    /// Planes pass through the map origin. Axisymmetric maps only mirror x.
    pub mirrors: [Option<[i8; 3]>; 3],
}

impl Default for MapPlacement {
    fn default() -> Self {
        Self {
            origin_mm: [0.0; 3],
            axes: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            scale: 1.0,
            outside: OutsideMap::Error,
            mirrors: [None; 3],
        }
    }
}

#[derive(Clone, Debug)]
pub struct ImportedMagneticMap {
    pub(crate) grids: [GridField; 3],
    pub(crate) placement: MapPlacement,
    pub(crate) axisymmetric: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct DivergenceReport {
    pub max_abs_gauss_per_mm: f64,
    pub rms_gauss_per_mm: f64,
    /// max |div B| * smallest spacing / max |B| (zero for zero B).
    pub normalized_max: f64,
    pub nodes: usize,
}

impl ImportedMagneticMap {
    /// x-fast vectors in gauss. Axisymmetric coordinates are axial/radial,
    /// values are [Baxial, Br, 0], and radial nodes begin exactly at r=0.
    /// Supply axes mapping the local axial direction to workbench coordinates.
    pub fn from_nodes(
        dims: [usize; 3],
        spacing_mm: Vector,
        mut values: Vec<Vector>,
        axisymmetric: bool,
        placement: MapPlacement,
    ) -> Result<Self, FlyError> {
        let count = dims.into_iter().try_fold(1usize, |n, d| n.checked_mul(d));
        if dims.contains(&0)
            || count != Some(values.len())
            || spacing_mm.iter().any(|v| !v.is_finite() || *v <= 0.0)
            || values.iter().flatten().any(|v| !v.is_finite())
            || placement.origin_mm.iter().any(|v| !v.is_finite())
            || !placement.scale.is_finite()
            || placement.scale <= 0.0
            || placement.axes.iter().flatten().any(|v| !v.is_finite())
        {
            return Err(FlyError(
                "magnetic map requires finite vectors, positive spacing/scale and matching nonempty dimensions",
            ));
        }
        for a in 0..3 {
            for b in 0..3 {
                if (dot(placement.axes[a], placement.axes[b]) - f64::from(a == b)).abs() > 1e-10 {
                    return Err(FlyError(
                        "magnetic map axes must be an orthonormal rotation",
                    ));
                }
            }
            if placement.mirrors[a].is_some_and(|p| p.iter().any(|&v| v != 1 && v != -1)) {
                return Err(FlyError("magnetic mirror parities must be +1 or -1"));
            }
        }
        if dot(
            crate::cross(placement.axes[0], placement.axes[1]),
            placement.axes[2],
        ) < 0.0
        {
            return Err(FlyError("magnetic map rotation must be right-handed"));
        }
        if axisymmetric {
            if dims[2] != 1 || dims[1] < 2 || placement.mirrors[1..].iter().any(Option::is_some) {
                return Err(FlyError(
                    "axisymmetric map requires nz=1, >=2 radii and only axial mirrors",
                ));
            }
            for x in 0..dims[0] {
                if values[x][1].abs() > 1e-12 * values[x][0].abs().max(1.0)
                    || (0..dims[1]).any(|r| values[r * dims[0] + x][2] != 0.0)
                {
                    return Err(FlyError(
                        "axisymmetric map requires Br=0 on axis and no azimuthal field",
                    ));
                }
                // Even Br/r is smooth at r=0. Use the derivative of the odd
                // radial field via a quadratic in r^2 (linear for two radii).
                let factor = |r: usize| values[r * dims[0] + x][1] / (r as f64 * spacing_mm[1]);
                let axis = if dims[1] >= 4 {
                    1.5 * factor(1) - 0.6 * factor(2) + 0.1 * factor(3)
                } else if dims[1] == 3 {
                    (4.0 * factor(1) - factor(2)) / 3.0
                } else {
                    factor(1)
                };
                for r in 1..dims[1] {
                    values[r * dims[0] + x][1] /= r as f64 * spacing_mm[1];
                }
                values[x][1] = axis;
            }
        }
        let component = |c: usize| {
            let mut pa = PotentialArray::new(dims[0], dims[1], dims[2]);
            pa.refinable = false;
            [pa.dx_mm, pa.dy_mm, pa.dz_mm] = spacing_mm;
            if axisymmetric {
                pa.symmetry = Symmetry::Cylindrical;
            }
            let parity = placement.mirrors.map(|p| p.map(|p| p[c]));
            pa.mirrors = MirrorFlags {
                x: parity[0].is_some(),
                y: parity[1].is_some(),
                z: parity[2].is_some(),
            };
            pa.anti_mirrors = MirrorFlags {
                x: parity[0] == Some(-1),
                y: parity[1] == Some(-1),
                z: parity[2] == Some(-1),
            };
            pa.potentials = values.iter().map(|b| b[c]).collect();
            for (axis, p) in parity.iter().enumerate() {
                if *p == Some(-1) {
                    for (i, &v) in pa.potentials.iter().enumerate() {
                        let q = [i % dims[0], i / dims[0] % dims[1], i / (dims[0] * dims[1])];
                        if q[axis] == 0 && v != 0.0 {
                            return Err(FlyError(
                                "odd magnetic mirror component must vanish on mirror plane",
                            ));
                        }
                    }
                }
            }
            GridField::new_gpu_map(pa)
        };
        Ok(Self {
            grids: [component(0)?, component(1)?, component(2)?],
            placement,
            axisymmetric,
        })
    }

    pub(crate) fn local(&self, p: Vector) -> Vector {
        let delta = std::array::from_fn(|a| p[a] - self.placement.origin_mm[a]);
        self.placement
            .axes
            .map(|axis| dot(axis, delta) / self.placement.scale)
    }

    fn lookup(&self, mut local: Vector) -> Vector {
        let pa = self.grids[0].array();
        if self.placement.outside == OutsideMap::Clamp {
            if self.axisymmetric {
                let r = local[1].hypot(local[2]);
                let max = (pa.ny - 1) as f64 * pa.dy_mm;
                if r > max {
                    local[1] *= max / r;
                    local[2] *= max / r;
                }
            }
            let dims = [pa.nx, pa.ny, pa.nz];
            let h = [pa.dx_mm, pa.dy_mm, pa.dz_mm];
            for a in 0..3 {
                if dims[a] == 1 || self.axisymmetric && a != 0 {
                    continue;
                }
                let max = (dims[a] - 1) as f64 * h[a];
                let min = if self.placement.mirrors[a].is_some() {
                    -max
                } else {
                    0.0
                };
                local[a] = local[a].clamp(min, max);
            }
        }
        local
    }

    /// Finite-difference diagnostic of the imported nodal B, not a Maxwell
    /// constraint or an interpolation/trajectory error bound.
    pub fn divergence_report(&self) -> DivergenceReport {
        let pa = self.grids[0].array();
        let dims = [pa.nx, pa.ny, pa.nz];
        let h = [pa.dx_mm, pa.dy_mm, pa.dz_mm];
        let index = |q: [usize; 3]| (q[2] * dims[1] + q[1]) * dims[0] + q[0];
        let mut max: f64 = 0.0;
        let mut sum = 0.0;
        let mut bmax: f64 = 0.0;
        for i in 0..pa.potentials.len() {
            let q = [i % dims[0], i / dims[0] % dims[1], i / (dims[0] * dims[1])];
            let value = |a: usize, n: usize| {
                let v = self.grids[a].array().potentials[n];
                if self.axisymmetric && a == 1 {
                    v * (n / dims[0] % dims[1]) as f64 * h[1]
                } else {
                    v
                }
            };
            let mut div = 0.0;
            for a in 0..3 {
                if dims[a] == 1 {
                    continue;
                }
                let mut lo = q;
                let mut hi = q;
                lo[a] = q[a].saturating_sub(1);
                hi[a] = (q[a] + 1).min(dims[a] - 1);
                div +=
                    (value(a, index(hi)) - value(a, index(lo))) / ((hi[a] - lo[a]) as f64 * h[a]);
            }
            if self.axisymmetric {
                div += self.grids[1].array().potentials[i];
            }
            let norm = (0..3).map(|a| value(a, i).powi(2)).sum::<f64>().sqrt();
            bmax = bmax.max(norm);
            max = max.max(div.abs());
            sum += div * div;
        }
        let hmin = (0..3)
            .filter(|&a| dims[a] > 1)
            .map(|a| h[a])
            .min_by(f64::total_cmp)
            .unwrap_or(1.0);
        DivergenceReport {
            max_abs_gauss_per_mm: max / self.placement.scale,
            rms_gauss_per_mm: (sum / pa.potentials.len() as f64).sqrt() / self.placement.scale,
            normalized_max: if bmax > 0.0 { max * hmin / bmax } else { 0.0 },
            nodes: pa.potentials.len(),
        }
    }
}

impl Field for ImportedMagneticMap {
    fn spatial_scale_mm(&self) -> Option<f64> {
        self.grids[0]
            .spatial_scale_mm()
            .map(|h| h * self.placement.scale)
    }
    fn sample(&self, p: Vector, t: f64) -> FieldSample {
        let local = self.local(p);
        if self.placement.outside == OutsideMap::Zero && !self.grids[0].contains(local) {
            return FieldSample::default();
        }
        let q = self.lookup(local);
        let mut b = self.grids.each_ref().map(|g| g.sample(q, t).potential_v);
        if self.axisymmetric {
            b = [b[0], b[1] * q[1], b[1] * q[2]];
        }
        FieldSample {
            magnetic_gauss: std::array::from_fn(|a| {
                (0..3).map(|c| self.placement.axes[c][a] * b[c]).sum()
            }),
            ..Default::default()
        }
    }
    fn status(&self, p: Vector) -> Option<SplatReason> {
        (self.placement.outside == OutsideMap::Error)
            .then(|| self.grids[0].status(self.local(p)))
            .flatten()
    }
    fn intersection(&self, a: Vector, b: Vector) -> Option<(f64, SplatReason)> {
        (self.placement.outside == OutsideMap::Error)
            .then(|| self.grids[0].intersection(self.local(a), self.local(b)))
            .flatten()
    }
}

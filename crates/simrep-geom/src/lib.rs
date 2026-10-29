//! Continuous electrode geometry, independent of grids and potential solvers.
//!
//! Coordinates and distances use the caller's length unit. Negative distance
//! means inside the conductor; zero is its surface. CSG preserves the sign and
//! the conservative distance bound, not necessarily the exact Euclidean distance.

use rayon::prelude::*;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

pub type Point = [f64; 3];

/// A finite, continuous signed-distance bound. Analytic implementations are
/// normally one-Lipschitz; sampled fields can report a larger bound.
/// Near the zero set it must be comparable to geometric distance, with a
/// constant independent of grid refinement (a scaled-down arbitrary field or
/// a zero plateau is not a signed-distance representation).
/// Mesh importers may implement this trait without depending on a solver.
pub trait SignedDistance: Send + Sync {
    fn distance(&self, point: Point) -> f64;

    /// Conservative global Lipschitz constant for the represented field.
    fn lipschitz_bound(&self) -> f64 {
        1.0
    }

    /// Optional conservative bounds on all distance values along a segment.
    /// Consumers may reject a segment only when its lower bound exceeds their
    /// root tolerance. Returning None retains pointwise sphere tracing.
    fn segment_distance_bounds(&self, _from: Point, _to: Point) -> Option<[f64; 2]> {
        None
    }

    /// Whether distance <= level forms at most one interval on this segment.
    /// This permits direct bracketing when the far endpoint is in that set.
    fn segment_single_interval(&self, _from: Point, _to: Point, _level: f64) -> bool {
        false
    }

    /// Unsigned distance from the segment to the surface exceeds `tolerance`.
    /// A segment deep inside the solid, far from the boundary, also returns
    /// true. Callers that treat an interior start as a hit must test that
    /// start before using this as a rejection.
    fn segment_clear(&self, from: Point, to: Point, tolerance: f64) -> bool {
        self.segment_distance_bounds(from, to)
            .is_some_and(|bounds| bounds[0] > tolerance)
    }

    /// Mark nodes with `distance <= threshold`. `Err` means a distance was
    /// non-finite. The default evaluates every node.
    fn conductor_mask(
        &self,
        dims: [usize; 3],
        origin: Point,
        spacing: Point,
        threshold: f64,
        mask: &mut [bool],
    ) -> Result<(), &'static str> {
        let bad = AtomicBool::new(false);
        mask.par_iter_mut().enumerate().for_each(|(i, flag)| {
            let c = [i % dims[0], i / dims[0] % dims[1], i / (dims[0] * dims[1])];
            let distance = self.distance(std::array::from_fn(|a| {
                origin[a] + c[a] as f64 * spacing[a]
            }));
            if !distance.is_finite() {
                bad.store(true, Ordering::Relaxed);
            }
            *flag = distance <= threshold;
        });
        if bad.load(Ordering::Relaxed) {
            Err("SDF returned a non-finite distance")
        } else {
            Ok(())
        }
    }

    /// Set bits for nodes an edge of length about `band` might intersect.
    /// Does not clear bits. The default marks the whole grid.
    fn mark_near_nodes(
        &self,
        _dims: [usize; 3],
        _origin: Point,
        _spacing: Point,
        _band: f64,
        near: &mut [bool],
    ) {
        near.fill(true);
    }

    /// Fill x-fastest samples on a validated grid. The output length must equal
    /// the product of the positive dimensions. Meshes may batch classification.
    fn sample_grid(&self, dims: [usize; 3], origin: Point, spacing: Point, values: &mut [f64]) {
        values.par_iter_mut().enumerate().for_each(|(i, value)| {
            let c = [i % dims[0], i / dims[0] % dims[1], i / (dims[0] * dims[1])];
            *value = self.distance(std::array::from_fn(|a| {
                origin[a] + c[a] as f64 * spacing[a]
            }));
        });
    }

    /// Sample exactly within `band_mm` of the surface; clamp magnitudes beyond
    /// it while preserving conductor membership. The band must be positive.
    /// This is sufficient for near-surface cut geometry, not far-field distance.
    fn sample_grid_band(
        &self,
        dims: [usize; 3],
        origin: Point,
        spacing: Point,
        band_mm: f64,
        values: &mut [f64],
    ) {
        self.sample_grid(dims, origin, spacing, values);
        values
            .par_iter_mut()
            .for_each(|value| *value = value.clamp(-band_mm, band_mm));
    }
}

impl<F: Fn(Point) -> f64 + Send + Sync> SignedDistance for F {
    fn distance(&self, point: Point) -> f64 {
        self(point)
    }
}

#[derive(Clone)]
pub struct Electrode {
    pub voltage: f64,
    pub shape: Arc<dyn SignedDistance>,
}

impl Electrode {
    pub fn new(voltage: f64, shape: impl SignedDistance + 'static) -> Self {
        Self {
            voltage,
            shape: Arc::new(shape),
        }
    }
}

/// Analytic primitives and constructive solid geometry.
#[derive(Clone, Debug)]
pub enum Sdf {
    Sphere {
        center: Point,
        radius: f64,
    },
    /// Infinite cylinder, parallel to `axis` (0, 1, or 2).
    Cylinder {
        center: Point,
        radius: f64,
        axis: usize,
    },
    Box {
        center: Point,
        half_extent: Point,
    },
    /// The normal must be unit length. Inside is dot(normal, point) <= offset.
    Plane {
        normal: Point,
        offset: f64,
    },
    Union(Box<Sdf>, Box<Sdf>),
    Intersection(Box<Sdf>, Box<Sdf>),
    Difference(Box<Sdf>, Box<Sdf>),
    Sampled(SampledSdf),
}

impl Sdf {
    pub fn union(self, other: Self) -> Self {
        Self::Union(Box::new(self), Box::new(other))
    }
    pub fn intersection(self, other: Self) -> Self {
        Self::Intersection(Box::new(self), Box::new(other))
    }
    pub fn difference(self, other: Self) -> Self {
        Self::Difference(Box::new(self), Box::new(other))
    }
}

impl SignedDistance for Sdf {
    fn distance(&self, p: Point) -> f64 {
        match self {
            Self::Sphere { center, radius } => {
                p.iter()
                    .zip(center)
                    .map(|(x, c)| (x - c).powi(2))
                    .sum::<f64>()
                    .sqrt()
                    - radius
            }
            Self::Cylinder {
                center,
                radius,
                axis,
            } => {
                p.iter()
                    .zip(center)
                    .enumerate()
                    .filter(|(a, _)| a != axis)
                    .map(|(_, (x, c))| (x - c).powi(2))
                    .sum::<f64>()
                    .sqrt()
                    - radius
            }
            Self::Box {
                center,
                half_extent,
            } => {
                let q: Point = std::array::from_fn(|a| (p[a] - center[a]).abs() - half_extent[a]);
                q.iter().map(|x| x.max(0.0).powi(2)).sum::<f64>().sqrt()
                    + q.into_iter().fold(f64::NEG_INFINITY, f64::max).min(0.0)
            }
            Self::Plane { normal, offset } => {
                p.iter().zip(normal).map(|(x, n)| x * n).sum::<f64>() - offset
            }
            Self::Union(a, b) => a.distance(p).min(b.distance(p)),
            Self::Intersection(a, b) => a.distance(p).max(b.distance(p)),
            Self::Difference(a, b) => a.distance(p).max(-b.distance(p)),
            Self::Sampled(grid) => grid.distance(p),
        }
    }

    fn segment_single_interval(&self, _from: Point, _to: Point, _level: f64) -> bool {
        fn convex(shape: &Sdf) -> bool {
            match shape {
                Sdf::Sphere { .. } | Sdf::Cylinder { .. } | Sdf::Box { .. } | Sdf::Plane { .. } => {
                    true
                }
                Sdf::Intersection(a, b) => convex(a) && convex(b),
                _ => false,
            }
        }
        convex(self)
    }
}

/// X-fastest trilinear samples. Outside the sampled box coordinates are clamped;
/// callers must cover the solver domain (including all boundary intersections).
#[derive(Clone, Debug)]
pub struct SampledSdf {
    pub dims: [usize; 3],
    pub origin: Point,
    pub spacing: Point,
    pub values: Vec<f64>,
}

impl SampledSdf {
    pub fn new(
        dims: [usize; 3],
        origin: Point,
        spacing: Point,
        values: Vec<f64>,
    ) -> Result<Self, &'static str> {
        if dims.contains(&0)
            || dims.iter().try_fold(1usize, |n, d| n.checked_mul(*d)) != Some(values.len())
            || !origin.iter().all(|v| v.is_finite())
            || !spacing.iter().all(|v| v.is_finite() && *v > 0.0)
            || !values.iter().all(|v| v.is_finite())
        {
            return Err("invalid sampled SDF dimensions, coordinates, or values");
        }
        Ok(Self {
            dims,
            origin,
            spacing,
            values,
        })
    }
}

impl SignedDistance for SampledSdf {
    fn lipschitz_bound(&self) -> f64 {
        let strides = [1, self.dims[0], self.dims[0] * self.dims[1]];
        let mut gradient = [0.0_f64; 3];
        for (a, &stride) in strides.iter().enumerate() {
            for i in 0..self.values.len() {
                if i / stride % self.dims[a] + 1 < self.dims[a] {
                    gradient[a] = gradient[a]
                        .max(((self.values[i + stride] - self.values[i]) / self.spacing[a]).abs());
                }
            }
        }
        gradient.into_iter().map(|x| x * x).sum::<f64>().sqrt()
    }

    fn distance(&self, p: Point) -> f64 {
        let q: Point = std::array::from_fn(|a| {
            ((p[a] - self.origin[a]) / self.spacing[a]).clamp(0.0, (self.dims[a] - 1) as f64)
        });
        let lo = q.map(|v| v.floor() as usize);
        let t: Point = std::array::from_fn(|a| q[a] - lo[a] as f64);
        let mut result = 0.0;
        for corner in 0..8 {
            let c: [usize; 3] =
                std::array::from_fn(|a| (lo[a] + ((corner >> a) & 1)).min(self.dims[a] - 1));
            let weight: f64 = (0..3)
                .map(|a| {
                    if (corner >> a) & 1 == 0 {
                        1.0 - t[a]
                    } else {
                        t[a]
                    }
                })
                .product();
            result += weight * self.values[c[0] + self.dims[0] * (c[1] + self.dims[1] * c[2])];
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primitive_signs_and_csg() {
        let sphere = Sdf::Sphere {
            center: [0.0; 3],
            radius: 1.0,
        };
        let block = Sdf::Box {
            center: [0.0; 3],
            half_extent: [0.5; 3],
        };
        assert_eq!(sphere.distance([2.0, 0.0, 0.0]), 1.0);
        assert_eq!(block.distance([0.0; 3]), -0.5);
        assert!(sphere.clone().difference(block.clone()).distance([0.0; 3]) > 0.0);
        assert!(
            sphere
                .clone()
                .intersection(block.clone())
                .distance([0.8, 0.0, 0.0])
                > 0.0
        );
        assert!(sphere.union(block).distance([0.8, 0.0, 0.0]) < 0.0);
        let cylinder = Sdf::Cylinder {
            center: [0.0; 3],
            radius: 2.0,
            axis: 2,
        };
        assert_eq!(cylinder.distance([0.0, 2.0, 19.0]), 0.0);
        let plane = Sdf::Plane {
            normal: [1.0, 0.0, 0.0],
            offset: 2.0,
        };
        assert_eq!(plane.distance([3.0, 5.0, 1.0]), 1.0);
    }

    #[test]
    fn sampled_affine_distance_including_degenerate_axis() {
        let grid = SampledSdf::new(
            [3, 2, 1],
            [0.0; 3],
            [1.0; 3],
            vec![-1.0, 0.0, 1.0, -1.0, 0.0, 1.0],
        )
        .unwrap();
        assert!((grid.distance([0.25, 0.7, 0.0]) + 0.75).abs() < 1e-15);
        assert!(SampledSdf::new([0, 1, 1], [0.0; 3], [1.0; 3], vec![]).is_err());
    }
}

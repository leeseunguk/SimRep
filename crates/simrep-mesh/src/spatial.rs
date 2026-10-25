use crate::{Part, Point, add, cross, dot, inspect, invalid, mul, norm, sub};
use rayon::prelude::*;
use std::io;
use std::ops::Range;

#[derive(Clone, Copy)]
struct Bounds {
    min: Point,
    max: Point,
}

impl Bounds {
    fn empty() -> Self {
        Self {
            min: [f64::INFINITY; 3],
            max: [f64::NEG_INFINITY; 3],
        }
    }
    fn include(&mut self, other: Self) {
        for i in 0..3 {
            self.min[i] = self.min[i].min(other.min[i]);
            self.max[i] = self.max[i].max(other.max[i]);
        }
    }
    fn distance2(self, point: Point) -> f64 {
        (0..3)
            .map(|i| {
                (self.min[i] - point[i])
                    .max(0.0)
                    .max(point[i] - self.max[i])
                    .powi(2)
            })
            .sum()
    }
    fn ray(self, origin: Point, direction: Point, limit: f64) -> bool {
        let mut low = 0.0_f64;
        let mut high = limit;
        for i in 0..3 {
            if direction[i] == 0.0 {
                if origin[i] < self.min[i] || origin[i] > self.max[i] {
                    return false;
                }
            } else {
                let a = (self.min[i] - origin[i]) / direction[i];
                let b = (self.max[i] - origin[i]) / direction[i];
                low = low.max(a.min(b));
                high = high.min(a.max(b));
                if high < low {
                    return false;
                }
            }
        }
        true
    }
}

struct Triangle {
    points: [Point; 3],
    bounds: Bounds,
    center: Point,
}

struct Node {
    bounds: Bounds,
    range: Range<usize>,
    children: Option<Box<[Node; 2]>>,
}

impl Node {
    fn build(triangles: &mut [Triangle], offset: usize) -> Self {
        let mut bounds = Bounds::empty();
        let mut centers = Bounds::empty();
        for t in triangles.iter() {
            bounds.include(t.bounds);
            centers.include(Bounds {
                min: t.center,
                max: t.center,
            });
        }
        let children = if triangles.len() > 8 {
            let axis = (0..3)
                .max_by(|&a, &b| {
                    (centers.max[a] - centers.min[a]).total_cmp(&(centers.max[b] - centers.min[b]))
                })
                .unwrap();
            let middle = triangles.len() / 2;
            triangles
                .select_nth_unstable_by(middle, |a, b| a.center[axis].total_cmp(&b.center[axis]));
            let parallel = triangles.len() > 32_768;
            let (left, right) = triangles.split_at_mut(middle);
            let (a, b) = if parallel {
                rayon::join(
                    || Self::build(left, offset),
                    || Self::build(right, offset + middle),
                )
            } else {
                (
                    Self::build(left, offset),
                    Self::build(right, offset + middle),
                )
            };
            Some(Box::new([a, b]))
        } else {
            None
        };
        Self {
            bounds,
            range: offset..offset + triangles.len(),
            children,
        }
    }
}

/// Read-only BVH over a closed manifold part. Classification uses even/odd
/// solid membership (nested shells form cavities), independent of winding.
/// Edge/vertex ray hits retry three non-axis-aligned rays, then use the
/// generalized winding number. Points on the surface count as inside.
pub struct Solid {
    triangles: Vec<Triangle>,
    root: Node,
    tolerance: f64,
}

impl simrep_geom::SignedDistance for Solid {
    fn distance(&self, point: Point) -> f64 {
        self.signed_distance(point)
    }

    fn sample_grid(&self, dims: [usize; 3], origin: Point, spacing: Point, values: &mut [f64]) {
        self.sample_grid_band(dims, origin, spacing, f64::INFINITY, values);
    }

    fn sample_grid_band(
        &self,
        dims: [usize; 3],
        origin: Point,
        spacing: Point,
        band_mm: f64,
        values: &mut [f64],
    ) {
        values
            .par_chunks_mut(dims[0] * dims[1])
            .enumerate()
            .for_each(|(z, slab)| {
                for (y, values) in slab.chunks_mut(dims[0]).enumerate() {
                    let start = [
                        origin[0],
                        origin[1] + y as f64 * spacing[1],
                        origin[2] + z as f64 * spacing[2],
                    ];
                    let inside = self.row(start, spacing[0], dims[0]);
                    let mut closest = 0;
                    for (x, (value, inside)) in values.iter_mut().zip(inside).enumerate() {
                        let point = [origin[0] + x as f64 * spacing[0], start[1], start[2]];
                        // A bounded BVH query cannot visit remote triangles.
                        // Retain the exact nearest triangle as a coherent seed.
                        let mut best = triangle_distance2(point, self.triangles[closest].points)
                            .min(band_mm * band_mm);
                        self.nearest(&self.root, point, &mut best, &mut closest);
                        let distance = best.sqrt().min(band_mm);
                        *value = if distance <= self.tolerance {
                            0.0
                        } else if inside {
                            -distance
                        } else {
                            distance
                        };
                    }
                }
            });
    }
}

impl Solid {
    pub fn new(part: &Part) -> io::Result<Self> {
        let report = inspect(part)?;
        if !report.is_closed_manifold() || report.inconsistent_edges != 0 {
            return Err(invalid(format!(
                "part {:?} is not a closed manifold: {report:?}",
                part.name
            )));
        }
        Self::from_surface(part)
    }

    /// Explicit best-effort sampling of a defective triangle surface. This
    /// does not close holes or resolve junctions. Inspect/repair the part
    /// first and retain its issue report; only closed meshes define a solid
    /// unambiguously. Zero-area triangles still require repair.
    pub fn from_surface(part: &Part) -> io::Result<Self> {
        part.validate()?;
        if part.triangles.iter().any(|face| {
            let [a, b, c] = face.map(|i| part.vertices[i]);
            norm(cross(sub(b, a), sub(c, a))) == 0.0
        }) {
            return Err(invalid(
                "remove zero-area triangles before surface sampling",
            ));
        }
        let mut triangles: Vec<_> = part
            .triangles
            .iter()
            .map(|face| {
                let points = face.map(|i| part.vertices[i]);
                let min = std::array::from_fn(|i| {
                    points.iter().map(|v| v[i]).fold(f64::INFINITY, f64::min)
                });
                let max = std::array::from_fn(|i| {
                    points
                        .iter()
                        .map(|v| v[i])
                        .fold(f64::NEG_INFINITY, f64::max)
                });
                Triangle {
                    center: mul(add(min, max), 0.5),
                    points,
                    bounds: Bounds { min, max },
                }
            })
            .collect();
        let root = Node::build(&mut triangles, 0);
        let diagonal = norm(sub(root.bounds.max, root.bounds.min));
        let coordinate_scale = root
            .bounds
            .min
            .into_iter()
            .chain(root.bounds.max)
            .map(f64::abs)
            .fold(diagonal, f64::max);
        let tolerance = (diagonal * 1e-10).max(coordinate_scale * f64::EPSILON * 16.0);
        Ok(Self {
            triangles,
            root,
            tolerance,
        })
    }

    pub fn bounds(&self) -> [Point; 2] {
        [self.root.bounds.min, self.root.bounds.max]
    }

    pub fn contains(&self, point: Point) -> bool {
        if point.iter().any(|v| !v.is_finite()) {
            return false;
        }
        if self.root.bounds.distance2(point) > self.tolerance * self.tolerance {
            return false;
        }
        if self.distance(point) <= self.tolerance {
            return true;
        }
        self.classify(point)
    }

    pub fn distance(&self, point: Point) -> f64 {
        let mut best = f64::INFINITY;
        let mut closest = 0;
        self.nearest(&self.root, point, &mut best, &mut closest);
        best.sqrt()
    }

    pub fn signed_distance(&self, point: Point) -> f64 {
        let distance = self.distance(point);
        if distance <= self.tolerance {
            0.0
        } else if self.classify(point) {
            -distance
        } else {
            distance
        }
    }

    /// Exact ray/triangle intersections of a finite segment, sorted and
    /// deduplicated at shared triangle edges. Both endpoints may be vacuum.
    pub fn segment_intersections(&self, from: Point, to: Point) -> Vec<f64> {
        if from == to {
            return Vec::new();
        }
        self.ray_hits(from, sub(to, from), 1.0)
            .0
            .into_iter()
            .filter(|t| *t <= 1.0)
            .collect()
    }

    fn nearest(&self, node: &Node, point: Point, best: &mut f64, closest: &mut usize) {
        if node.bounds.distance2(point) > *best {
            return;
        }
        if let Some(children) = &node.children {
            let [a, b] = children.as_ref();
            let (first, second) = if a.bounds.distance2(point) < b.bounds.distance2(point) {
                (a, b)
            } else {
                (b, a)
            };
            self.nearest(first, point, best, closest);
            self.nearest(second, point, best, closest);
        } else {
            for i in node.range.clone() {
                let triangle = &self.triangles[i];
                if triangle.bounds.distance2(point) > *best {
                    continue;
                }
                let distance = triangle_distance2(point, triangle.points);
                if distance < *best {
                    *best = distance;
                    *closest = i;
                }
            }
        }
    }

    fn intersections(
        &self,
        node: &Node,
        origin: Point,
        direction: Point,
        hits: &mut (Vec<f64>, bool),
        limit: f64,
    ) {
        if !node.bounds.ray(origin, direction, limit) {
            return;
        }
        if let Some(children) = &node.children {
            for child in children.iter() {
                self.intersections(child, origin, direction, hits, limit);
            }
        } else {
            for triangle in &self.triangles[node.range.clone()] {
                let [a, b, c] = triangle.points;
                let e1 = sub(b, a);
                let e2 = sub(c, a);
                let h = cross(direction, e2);
                let determinant = dot(e1, h);
                if determinant.abs() <= 1e-14 * norm(e1) * norm(e2) * norm(direction) {
                    // A segment can enter along a face or an edge without
                    // crossing its plane. Clip the coplanar ray against the
                    // triangle's three barycentric half-planes.
                    let normal = cross(e1, e2);
                    let s = sub(origin, a);
                    if dot(s, normal).abs() > self.tolerance * norm(normal) {
                        continue;
                    }
                    let aa = dot(e1, e1);
                    let ab = dot(e1, e2);
                    let bb = dot(e2, e2);
                    let denominator = aa * bb - ab * ab;
                    let barycentric = |p| {
                        let x = dot(p, e1);
                        let y = dot(p, e2);
                        [
                            (bb * x - ab * y) / denominator,
                            (aa * y - ab * x) / denominator,
                        ]
                    };
                    let [u, v] = barycentric(s);
                    let [du, dv] = barycentric(direction);
                    let (mut low, mut high) = (0.0_f64, limit);
                    for (value, slope) in [(u, du), (v, dv), (1.0 - u - v, -du - dv)] {
                        if slope > 0.0 {
                            low = low.max(-value / slope);
                        } else if slope < 0.0 {
                            high = high.min(-value / slope);
                        } else if value < 0.0 {
                            high = -1.0;
                        }
                    }
                    if low <= high {
                        hits.1 = true;
                        hits.0.push(low);
                        if high.is_finite() {
                            hits.0.push(high);
                        }
                    }
                    continue;
                }
                let inv = 1.0 / determinant;
                let s = sub(origin, a);
                let u = dot(s, h) * inv;
                let q = cross(s, e1);
                let v = dot(direction, q) * inv;
                let epsilon = 1e-10;
                if u < -epsilon || v < -epsilon || u + v > 1.0 + epsilon {
                    continue;
                }
                let t = dot(e2, q) * inv;
                if t < -self.tolerance / norm(direction) || t > limit {
                    continue;
                }
                hits.1 |= u <= epsilon || v <= epsilon || 1.0 - u - v <= epsilon;
                hits.0.push(t.max(0.0));
            }
        }
    }

    fn ray_hits(&self, origin: Point, direction: Point, limit: f64) -> (Vec<f64>, bool) {
        let mut hits = (Vec::new(), false);
        self.intersections(&self.root, origin, direction, &mut hits, limit);
        hits.0.sort_unstable_by(f64::total_cmp);
        let epsilon = self.tolerance / norm(direction);
        hits.0.dedup_by(|a, b| (*a - *b).abs() <= epsilon);
        hits
    }

    fn classify(&self, point: Point) -> bool {
        if self.root.bounds.distance2(point) > self.tolerance * self.tolerance {
            return false;
        }
        for direction in [
            [1.0, 0.3713906763541037, 0.529117217723],
            [0.217731, 1.0, 0.613191],
            [0.419173, 0.263117, 1.0],
        ] {
            let (hits, ambiguous) = self.ray_hits(point, direction, f64::INFINITY);
            if !ambiguous {
                return hits.len() % 2 == 1;
            }
        }
        let angle: f64 = self
            .triangles
            .iter()
            .map(|triangle| {
                let [a, b, c] = triangle.points.map(|v| sub(v, point));
                let denominator = norm(a) * norm(b) * norm(c)
                    + dot(a, b) * norm(c)
                    + dot(b, c) * norm(a)
                    + dot(c, a) * norm(b);
                2.0 * dot(a, cross(b, c)).atan2(denominator)
            })
            .sum();
        let winding = (angle.abs() / (4.0 * std::f64::consts::PI)).round() as u64;
        winding % 2 == 1
    }

    /// One accelerated ray for an entire x row; degenerate rays use the
    /// pointwise robust classifier instead of moving the requested samples.
    pub(crate) fn row(&self, start: Point, dx: f64, count: usize) -> Vec<bool> {
        if start[1] < self.root.bounds.min[1] - self.tolerance
            || start[1] > self.root.bounds.max[1] + self.tolerance
            || start[2] < self.root.bounds.min[2] - self.tolerance
            || start[2] > self.root.bounds.max[2] + self.tolerance
        {
            return vec![false; count];
        }
        let origin = [
            self.root.bounds.min[0] - dx.max(self.tolerance * 4.0),
            start[1],
            start[2],
        ];
        let (hits, ambiguous) = self.ray_hits(origin, [1.0, 0.0, 0.0], f64::INFINITY);
        if ambiguous || hits.len() % 2 != 0 {
            return (0..count)
                .map(|x| self.contains([start[0] + x as f64 * dx, start[1], start[2]]))
                .collect();
        }
        (0..count)
            .map(|x| {
                let t = start[0] + x as f64 * dx - origin[0];
                let index = hits.partition_point(|&hit| hit < t - self.tolerance);
                index % 2 == 1
                    || hits
                        .get(index)
                        .is_some_and(|hit| (hit - t).abs() <= self.tolerance)
            })
            .collect()
    }
}

fn triangle_distance2(p: Point, [a, b, c]: [Point; 3]) -> f64 {
    let ab = sub(b, a);
    let ac = sub(c, a);
    let ap = sub(p, a);
    let d1 = dot(ab, ap);
    let d2 = dot(ac, ap);
    if d1 <= 0.0 && d2 <= 0.0 {
        return dot(ap, ap);
    }
    let bp = sub(p, b);
    let d3 = dot(ab, bp);
    let d4 = dot(ac, bp);
    if d3 >= 0.0 && d4 <= d3 {
        return dot(bp, bp);
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        let delta = sub(p, add(a, mul(ab, d1 / (d1 - d3))));
        return dot(delta, delta);
    }
    let cp = sub(p, c);
    let d5 = dot(ab, cp);
    let d6 = dot(ac, cp);
    if d6 >= 0.0 && d5 <= d6 {
        return dot(cp, cp);
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        let delta = sub(p, add(a, mul(ac, d2 / (d2 - d6))));
        return dot(delta, delta);
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && d4 - d3 >= 0.0 && d5 - d6 >= 0.0 {
        let delta = sub(
            p,
            add(b, mul(sub(c, b), (d4 - d3) / ((d4 - d3) + (d5 - d6)))),
        );
        return dot(delta, delta);
    }
    let denominator = 1.0 / (va + vb + vc);
    let closest = add(a, add(mul(ab, vb * denominator), mul(ac, vc * denominator)));
    let delta = sub(p, closest);
    dot(delta, delta)
}

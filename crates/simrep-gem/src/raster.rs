//! Integer row spans. CSG combines spans rather than re-evaluating its tree at
//! every grid point. Only non-analytic projections use the point fallback.
use crate::geometry::{
    Point, Primitive, RasterMode, Shape, cylinder_frame, dot, polygon_corner, polygon_margin,
    polygon_orientation, polygon_plane,
};

type Spans = Vec<(usize, usize)>;

fn intersect(a: &Spans, b: &Spans) -> Spans {
    let mut out = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        let lo = a[i].0.max(b[j].0);
        let hi = a[i].1.min(b[j].1);
        if lo < hi {
            out.push((lo, hi));
        }
        if a[i].1 < b[j].1 {
            i += 1;
        } else {
            j += 1;
        }
    }
    out
}

fn union(mut a: Spans, b: Spans) -> Spans {
    a.extend(b);
    a.sort_unstable();
    let mut out: Spans = Vec::new();
    for (lo, hi) in a {
        if let Some(last) = out.last_mut()
            && lo <= last.1
        {
            last.1 = last.1.max(hi);
        } else {
            out.push((lo, hi));
        }
    }
    out
}

fn subtract(a: Spans, b: Spans) -> Spans {
    let mut out = Vec::new();
    for (mut lo, hi) in a {
        for &(start, end) in &b {
            if start >= hi {
                break;
            }
            if end > lo {
                if start > lo {
                    out.push((lo, start));
                }
                lo = lo.max(end);
            }
        }
        if lo < hi {
            out.push((lo, hi));
        }
    }
    out
}

fn sampled(n: usize, mut test: impl FnMut(usize) -> bool) -> Spans {
    let mut out = Vec::new();
    let mut start = None;
    for x in 0..n {
        if test(x) {
            start.get_or_insert(x);
        } else if let Some(lo) = start.take() {
            out.push((lo, x));
        }
    }
    if let Some(lo) = start {
        out.push((lo, n));
    }
    out
}

/// Intersect a continuous interval with a*x <= b.
fn clip(bounds: &mut (f64, f64), a: f64, b: f64) {
    if a > 0. {
        bounds.1 = bounds.1.min(b / a);
    } else if a < 0. {
        bounds.0 = bounds.0.max(b / a);
    } else if b < 0. {
        bounds.0 = 1.;
        bounds.1 = 0.;
    }
}

fn quadratic(a: f64, b: f64, c: f64) -> Vec<(f64, f64)> {
    if a == 0. {
        let mut bounds = (f64::NEG_INFINITY, f64::INFINITY);
        clip(&mut bounds, b, -c);
        return vec![bounds];
    }
    let discriminant = b * b - 4. * a * c;
    let roundoff = 16. * f64::EPSILON * (b * b).abs().max((4. * a * c).abs());
    if discriminant < -roundoff {
        return if a < 0. {
            vec![(f64::NEG_INFINITY, f64::INFINITY)]
        } else {
            Vec::new()
        };
    }
    let d = discriminant.max(0.).sqrt();
    let lo = ((-b - d) / (2. * a)).min((-b + d) / (2. * a));
    let hi = ((-b - d) / (2. * a)).max((-b + d) / (2. * a));
    if a > 0. {
        vec![(lo, hi)]
    } else {
        vec![(f64::NEG_INFINITY, lo), (hi, f64::INFINITY)]
    }
}

impl Primitive {
    fn spans(&self, p: Point, d: Point, m: Point, on: bool, mode: RasterMode, n: usize) -> Spans {
        let auto = mode.auto;
        let mut bounds = (f64::NEG_INFINITY, f64::INFINITY);
        let intervals = match self {
            Self::Lua(_) => {
                return sampled(n, |x| {
                    self.contains(std::array::from_fn(|i| p[i] + d[i] * x as f64), m, on, mode)
                });
            }
            Self::Mesh(triangles) => {
                let hits = crate::stl::intersections(triangles, p, d);
                hits.chunks_exact(2)
                    .map(|v| {
                        (
                            v[0] - m[0] / dot(d, d).sqrt(),
                            v[1] + m[0] / dot(d, d).sqrt(),
                        )
                    })
                    .collect()
            }
            Self::Box { min, max } => {
                for i in 0..3 {
                    clip(&mut bounds, -d[i], p[i] - min[i] + m[i]);
                    clip(&mut bounds, d[i], max[i] + m[i] - p[i]);
                }
                vec![bounds]
            }
            Self::Ellipsoid {
                center,
                radii,
                dimensions,
            } => {
                let q: Point = std::array::from_fn(|i| (p[i] - center[i]) / (radii[i] + m[i]));
                let v: Point = std::array::from_fn(|i| d[i] / (radii[i] + m[i]));
                let (mut a, mut b, mut c) = (0., 0., -1.);
                for i in 0..*dimensions {
                    a += v[i] * v[i];
                    b += 2. * q[i] * v[i];
                    c += q[i] * q[i];
                }
                quadratic(a, b, c)
            }
            Self::Cylinder {
                center,
                radii,
                length,
            } => {
                let q = [
                    (p[0] - center[0]) / (radii[0] + m[0]),
                    (p[1] - center[1]) / (radii[1] + m[1]),
                ];
                let v = [d[0] / (radii[0] + m[0]), d[1] / (radii[1] + m[1])];
                clip(&mut bounds, -d[2], p[2] - center[2] + length + m[2]);
                clip(&mut bounds, d[2], center[2] + m[2] - p[2]);
                quadratic(
                    v[0] * v[0] + v[1] * v[1],
                    2. * (q[0] * v[0] + q[1] * v[1]),
                    q[0] * q[0] + q[1] * q[1] - 1.,
                )
                .into_iter()
                .map(|(a, b)| (a.max(bounds.0), b.min(bounds.1)))
                .collect()
            }
            Self::Polygon(vertices) => {
                if auto && vertices.iter().flatten().all(|v| v.is_finite()) {
                    for i in 0..2 {
                        let min =
                            vertices.iter().map(|p| p[i]).fold(f64::INFINITY, f64::min) - m[i];
                        let max = vertices
                            .iter()
                            .map(|p| p[i])
                            .fold(f64::NEG_INFINITY, f64::max)
                            + m[i];
                        clip(&mut bounds, -d[i], p[i] - min);
                        clip(&mut bounds, d[i], max - p[i]);
                    }
                }
                let Some(area) = polygon_orientation(vertices) else {
                    return sampled(n, |x| {
                        self.contains(std::array::from_fn(|i| p[i] + d[i] * x as f64), m, on, mode)
                    });
                };
                for i in 0..vertices.len() {
                    let plane = polygon_plane(vertices[i], vertices[(i + 1) % vertices.len()]);
                    let offset = polygon_margin(plane[1], plane[0], m);
                    clip(
                        &mut bounds,
                        -(d[0] * plane[0] + d[1] * plane[1]) * area,
                        offset + dot(plane, [p[0], p[1], 1.]) * area,
                    );
                    if vertices[i].iter().all(|v| v.is_finite()) {
                        let normal = polygon_corner(vertices, i, area);
                        let offset = (normal[0] * m[0]).hypot(normal[1] * m[1]).copysign(m[0]);
                        clip(
                            &mut bounds,
                            normal[0] * d[0] + normal[1] * d[1],
                            offset
                                - normal[0] * (p[0] - vertices[i][0])
                                - normal[1] * (p[1] - vertices[i][1]),
                        );
                    }
                }
                vec![bounds]
            }
            Self::Cylinder3d { start, end, radius } => {
                let (origin, axis, min, max) = cylinder_frame(*start, *end);
                let q: Point = std::array::from_fn(|i| p[i] - origin[i]);
                let axial = dot(q, axis);
                let velocity = dot(d, axis);
                let radial: Point = std::array::from_fn(|i| q[i] - axis[i] * axial);
                let v: Point = std::array::from_fn(|i| d[i] - axis[i] * velocity);
                clip(&mut bounds, -velocity, axial - min + m[0]);
                clip(&mut bounds, velocity, max + m[0] - axial);
                quadratic(
                    dot(v, v),
                    2. * dot(radial, v),
                    dot(radial, radial) - (radius + m[0]).powi(2),
                )
                .into_iter()
                .map(|(a, b)| (a.max(bounds.0), b.min(bounds.1)))
                .collect()
            }
            Self::Hyperbola { center, radii } => {
                if auto {
                    return sampled(n, |x| {
                        self.contains(std::array::from_fn(|i| p[i] + d[i] * x as f64), m, on, mode)
                    });
                }
                let x = (p[0] - center[0]) / (radii[0] + m[0]);
                let y = (p[1] - center[1]) / (radii[1] - m[1]);
                let vx = d[0] / (radii[0] + m[0]);
                let vy = d[1] / (radii[1] - m[1]);
                quadratic(
                    vx * vx - vy * vy,
                    2. * (x * vx - y * vy),
                    1. + x * x - y * y,
                )
            }
            Self::HalfSpace { origin, normal } => {
                let q = std::array::from_fn(|i| p[i] - origin[i]);
                let margin = (0..3)
                    .map(|i| (normal[i] * m[i]).powi(2))
                    .sum::<f64>()
                    .sqrt()
                    .copysign(m[0]);
                clip(&mut bounds, dot(d, *normal), margin - dot(q, *normal));
                vec![bounds]
            }
            _ => {
                return sampled(n, |x| {
                    self.contains(std::array::from_fn(|i| p[i] + d[i] * x as f64), m, on, mode)
                });
            }
        };
        let mut out = Vec::new();
        for (lo, hi) in intervals {
            if lo > hi || hi < 0. || lo > n as f64 {
                continue;
            }
            // Include a guard point at each analytic root, then use the exact
            // primitive predicate for the open/closed boundary convention.
            let mut start = (lo.ceil() - 1.).max(0.) as usize;
            let mut end = (hi.floor() + 2.).clamp(0., n as f64) as usize;
            while start < end
                && !self.contains(
                    std::array::from_fn(|i| p[i] + d[i] * start as f64),
                    m,
                    on,
                    mode,
                )
            {
                start += 1;
            }
            while start < end
                && !self.contains(
                    std::array::from_fn(|i| p[i] + d[i] * (end - 1) as f64),
                    m,
                    on,
                    mode,
                )
            {
                end -= 1;
            }
            if start < end {
                out.push((start, end));
            }
        }
        out
    }
}

impl Shape {
    pub(crate) fn bounds(&self, margin: Point) -> (Point, Point) {
        let unlimited = ([f64::NEG_INFINITY; 3], [f64::INFINITY; 3]);
        match self {
            Self::Potential(_, shape) => shape.bounds(margin),
            Self::Primitive(primitive) => match primitive {
                Primitive::Mesh(triangles) => (
                    std::array::from_fn(|i| {
                        triangles
                            .iter()
                            .flatten()
                            .map(|p| p[i])
                            .fold(f64::INFINITY, f64::min)
                            - margin[i]
                    }),
                    std::array::from_fn(|i| {
                        triangles
                            .iter()
                            .flatten()
                            .map(|p| p[i])
                            .fold(f64::NEG_INFINITY, f64::max)
                            + margin[i]
                    }),
                ),
                Primitive::Box { min, max } => (
                    std::array::from_fn(|i| min[i] - margin[i]),
                    std::array::from_fn(|i| max[i] + margin[i]),
                ),
                Primitive::Ellipsoid {
                    center,
                    radii,
                    dimensions,
                } => (
                    std::array::from_fn(|i| {
                        if i < *dimensions {
                            center[i] - radii[i] - margin[i]
                        } else {
                            f64::NEG_INFINITY
                        }
                    }),
                    std::array::from_fn(|i| {
                        if i < *dimensions {
                            center[i] + radii[i] + margin[i]
                        } else {
                            f64::INFINITY
                        }
                    }),
                ),
                Primitive::Cylinder {
                    center,
                    radii,
                    length,
                } => (
                    [
                        center[0] - radii[0] - margin[0],
                        center[1] - radii[1] - margin[1],
                        center[2] - length - margin[2],
                    ],
                    [
                        center[0] + radii[0] + margin[0],
                        center[1] + radii[1] + margin[1],
                        center[2] + margin[2],
                    ],
                ),
                Primitive::Cylinder3d { start, end, radius } => (
                    std::array::from_fn(|i| start[i].min(end[i]) - radius - margin[i]),
                    std::array::from_fn(|i| start[i].max(end[i]) + radius + margin[i]),
                ),
                Primitive::Polygon(vertices)
                    if vertices.iter().flatten().all(|v| v.is_finite()) =>
                {
                    (
                        std::array::from_fn(|i| {
                            if i < 2 {
                                vertices.iter().map(|p| p[i]).fold(f64::INFINITY, f64::min)
                                    - 2. * margin[i]
                            } else {
                                f64::NEG_INFINITY
                            }
                        }),
                        std::array::from_fn(|i| {
                            if i < 2 {
                                vertices
                                    .iter()
                                    .map(|p| p[i])
                                    .fold(f64::NEG_INFINITY, f64::max)
                                    + 2. * margin[i]
                            } else {
                                f64::INFINITY
                            }
                        }),
                    )
                }
                _ => unlimited,
            },
            Self::Transform(t, s) => {
                let (min, max) = s.bounds(t.margin(margin));
                let a = t.inverse;
                let cross = |u: Point, v: Point| {
                    [
                        u[1] * v[2] - u[2] * v[1],
                        u[2] * v[0] - u[0] * v[2],
                        u[0] * v[1] - u[1] * v[0],
                    ]
                };
                let cof = [cross(a[1], a[2]), cross(a[2], a[0]), cross(a[0], a[1])];
                let det = dot(a[0], cof[0]);
                let mut lo = t.origin;
                let mut hi = t.origin;
                for i in 0..3 {
                    for j in 0..3 {
                        let c = cof[j][i] / det;
                        if c == 0. {
                            continue;
                        }
                        lo[i] += (c * min[j]).min(c * max[j]);
                        hi[i] += (c * min[j]).max(c * max[j]);
                    }
                }
                (lo, hi)
            }
            Self::Intersection(shapes) => {
                let mut out = unlimited;
                for s in shapes {
                    let (min, max) = s.bounds(margin);
                    for i in 0..3 {
                        out.0[i] = out.0[i].max(min[i]);
                        out.1[i] = out.1[i].min(max[i]);
                    }
                }
                out
            }
            Self::Region(regions) => {
                let mut out = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
                let mut any = false;
                for r in regions.iter().filter(|r| !r.exclude) {
                    any = true;
                    let (min, max) = r.shape.bounds(margin);
                    for i in 0..3 {
                        out.0[i] = out.0[i].min(min[i]);
                        out.1[i] = out.1[i].max(max[i]);
                    }
                }
                if any { out } else { unlimited }
            }
            Self::Revolve { shape, axis, .. } => {
                let (min, max) = shape.bounds(margin);
                let radius = min[1].abs().max(max[1].abs());
                let mut out = ([-radius; 3], [radius; 3]);
                out.0[*axis] = min[0];
                out.1[*axis] = max[0];
                out
            }
            Self::Extrude {
                shape,
                min,
                max,
                axis,
            } => {
                let (lo, hi) = shape.bounds(margin);
                let (a, b, c) = match axis {
                    2 => (0, 1, 2),
                    0 => (1, 2, 0),
                    _ => (2, 0, 1),
                };
                let mut out = unlimited;
                out.0[a] = lo[0];
                out.0[b] = lo[1];
                out.0[c] = min - margin[c];
                out.1[a] = hi[0];
                out.1[b] = hi[1];
                out.1[c] = max + margin[c];
                out
            }
        }
    }

    pub(crate) fn spans(
        &self,
        p: Point,
        d: Point,
        margin: Point,
        on: bool,
        mode: RasterMode,
        n: usize,
    ) -> Spans {
        match self {
            Self::Potential(_, shape) => shape.spans(p, d, margin, on, mode, n),
            Self::Primitive(s) => s.spans(p, d, margin, on, mode, n),
            Self::Transform(t, s) => {
                let direction = std::array::from_fn(|i| dot(t.inverse[i], d));
                s.spans(
                    t.point(p),
                    direction,
                    t.margin(margin).map(|m| m.copysign(margin[0])),
                    on,
                    mode,
                    n,
                )
            }
            Self::Intersection(shapes) => {
                let mut out = vec![(0, n)];
                for s in shapes {
                    out = intersect(&out, &s.spans(p, d, margin, on, mode, n));
                    if out.is_empty() {
                        break;
                    }
                }
                out
            }
            Self::Region(regions) => {
                let mut included = if regions.iter().any(|r| !r.exclude) {
                    Vec::new()
                } else {
                    vec![(0, n)]
                };
                let mut excluded = Vec::new();
                for r in regions {
                    let (m, on) = mode.inclusion(r, margin, on);
                    let spans = r.shape.spans(p, d, m, on, mode, n);
                    if r.exclude {
                        excluded = union(excluded, spans)
                    } else {
                        included = union(included, spans)
                    }
                }
                subtract(included, excluded)
            }
            Self::Extrude {
                shape,
                min,
                max,
                axis,
            } => {
                let (a, b, c) = match axis {
                    2 => (0, 1, 2),
                    0 => (1, 2, 0),
                    _ => (2, 0, 1),
                };
                let mask = Primitive::Box {
                    min: [*min, -1e100, -1e100],
                    max: [*max, 1e100, 1e100],
                }
                .spans(
                    [p[c], 0., 0.],
                    [d[c], 0., 0.],
                    [margin[c], 0., 0.],
                    on,
                    mode,
                    n,
                );
                intersect(
                    &mask,
                    &shape.spans(
                        [p[a], p[b], 0.],
                        [d[a], d[b], 0.],
                        [margin[a], margin[b], margin[c]],
                        on,
                        mode,
                        n,
                    ),
                )
            }
            Self::Revolve {
                shape,
                start,
                end,
                axis,
                ..
            } if d[(axis + 1) % 3] == 0. && d[(axis + 2) % 3] == 0. => {
                let a = *axis;
                let b = (a + 1) % 3;
                let c = (a + 2) % 3;
                let angle = (p[c].atan2(p[b]).to_degrees() - start).rem_euclid(360.);
                let radius = p[b].hypot(p[c]);
                let allowed = end - start >= 360. - 1e-10
                    || angle <= end - start + 1e-10
                    || angle >= 360. - 1e-10;
                if end - start < 360. - 1e-10 && radius == 0. {
                    return Vec::new();
                }
                if !allowed {
                    Vec::new()
                } else {
                    shape.spans([p[a], radius, 0.], [d[a], 0., 0.], margin, on, mode, n)
                }
            }
            _ => sampled(n, |x| {
                self.contains(
                    std::array::from_fn(|i| p[i] + d[i] * x as f64),
                    margin,
                    on,
                    mode,
                )
            }),
        }
    }

    pub(crate) fn edge_spans(
        &self,
        p: Point,
        scale: Point,
        margin: Point,
        mode: RasterMode,
        dims: [usize; 3],
    ) -> Spans {
        let d = [scale[0], 0., 0.];
        let all = self.spans(p, d, margin, false, mode, dims[0]);
        let mut interior = all.clone();
        for axis in 0..3 {
            if dims[axis] == 1 {
                continue;
            }
            for direction in [-1., 1.] {
                let mut q = p;
                q[axis] += direction * scale[axis];
                interior = intersect(&interior, &self.spans(q, d, margin, false, mode, dims[0]));
            }
        }
        subtract(all, interior)
    }
}

//! Analytic CSG descriptions. Coordinates and radii are in the enclosing frame.

use crate::parser::Node;
use crate::{Error, Result};
use serde::{Deserialize, Serialize};

pub type Point = [f64; 3];

#[derive(Clone, Copy)]
pub(crate) struct RasterMode {
    pub auto: bool,
    pub fractional: bool,
    pub electrode: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub struct Transform {
    /// Row-major matrix mapping enclosing coordinates into local coordinates.
    pub inverse: [[f64; 3]; 3],
    pub origin: Point,
}

impl Transform {
    pub fn identity() -> Self {
        Self {
            inverse: [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
            origin: [0.; 3],
        }
    }
    pub(crate) fn point(&self, p: Point) -> Point {
        let p = std::array::from_fn::<_, 3, _>(|i| p[i] - self.origin[i]);
        std::array::from_fn(|i| dot(self.inverse[i], p))
    }
    pub(crate) fn margin(&self, m: Point) -> Point {
        if m == [0.; 3] {
            return m;
        }
        let a = self.inverse;
        let cross = |u: Point, v: Point| {
            [
                u[1] * v[2] - u[2] * v[1],
                u[2] * v[0] - u[0] * v[2],
                u[0] * v[1] - u[1] * v[0],
            ]
        };
        let cofactors = [cross(a[1], a[2]), cross(a[2], a[0]), cross(a[0], a[1])];
        let determinant = dot(a[0], cofactors[0]);
        std::array::from_fn(|i| {
            1. / (0..3)
                .map(|j| (cofactors[i][j] / (determinant * m[j])).powi(2))
                .sum::<f64>()
                .sqrt()
        })
    }
}

// Variant order and field order below are part of cuts-v3's binary CSG format.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum Primitive {
    Mesh(Vec<crate::stl::Triangle>),
    Box {
        min: Point,
        max: Point,
    },
    Ellipsoid {
        center: Point,
        radii: Point,
        dimensions: usize,
    },
    Cylinder {
        center: Point,
        radii: [f64; 2],
        length: f64,
    },
    Polygon(Vec<[f64; 2]>),
    Points {
        points: Vec<Point>,
        dimensions: usize,
    },
    Hyperbola {
        center: [f64; 2],
        radii: [f64; 2],
    },
    Parabola {
        vertex: [f64; 2],
        focus: f64,
    },
    HalfSpace {
        origin: Point,
        normal: Point,
    },
    Cylinder3d {
        start: Point,
        end: Point,
        radius: f64,
    },
    #[serde(skip)]
    Lua(crate::lua::Callback),
}

/// A simple analytic description retained for geometry inspection/cut-cell consumers.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum Shape {
    Primitive(Primitive),
    Transform(Transform, Box<Shape>),
    Intersection(Vec<Shape>),
    Region(Vec<Region>),
    Revolve {
        shape: Box<Shape>,
        start: f64,
        end: f64,
        axis: usize,
    },
    Extrude {
        shape: Box<Shape>,
        min: f64,
        max: f64,
        axis: usize,
    },
    #[serde(skip)]
    Potential(crate::lua::Callback, Box<Shape>),
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub enum Inclusion {
    /// A bare 8.2 shape inherits the enclosing inclusion test.
    Inherited,
    Expanded,
    Inside,
    InsideOrOn,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Region {
    pub exclude: bool,
    pub inclusion: Inclusion,
    pub shape: Shape,
}

impl RasterMode {
    pub(crate) fn inclusion(self, r: &Region, margin: Point, on: bool) -> (Point, bool) {
        let epsilon = if self.fractional {
            1.
        } else if self.auto {
            0.00005 / 0.35
        } else {
            0.00005 / 0.5
        };
        match r.inclusion {
            Inclusion::Inherited => (margin, on),
            Inclusion::Expanded => (
                margin.map(|m| {
                    m.abs()
                        * if self.auto && r.exclude == self.electrode {
                            -1.
                        } else {
                            1.
                        }
                }),
                false,
            ),
            Inclusion::Inside => (margin.map(|m| -m.abs() * epsilon), false),
            Inclusion::InsideOrOn => (margin.map(|m| m.abs() * epsilon), false),
        }
    }
}

pub(crate) fn dot(a: Point, b: Point) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

pub(crate) fn cylinder_frame(start: Point, end: Point) -> (Point, Point, f64, f64) {
    if let Some(axis) = (0..3).find(|&i| !start[i].is_finite() || !end[i].is_finite()) {
        let mut origin = start;
        origin[axis] = 0.;
        let mut direction = [0.; 3];
        direction[axis] = 1.;
        (
            origin,
            direction,
            start[axis].min(end[axis]),
            start[axis].max(end[axis]),
        )
    } else {
        let axis = std::array::from_fn(|i| end[i] - start[i]);
        let length = dot(axis, axis).sqrt();
        (start, axis.map(|v| v / length), 0., length)
    }
}

fn homogeneous(p: [f64; 2]) -> Point {
    if p[0].is_infinite() || p[1].is_infinite() {
        [
            if p[0].is_infinite() {
                p[0].signum()
            } else {
                0.
            },
            if p[1].is_infinite() {
                p[1].signum()
            } else {
                0.
            },
            0.,
        ]
    } else {
        [p[0], p[1], 1.]
    }
}

pub(crate) fn polygon_plane(a: [f64; 2], b: [f64; 2]) -> Point {
    let a = homogeneous(a);
    let b = homogeneous(b);
    let mut plane = [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ];
    let scale = plane[0]
        .abs()
        .max(plane[1].abs())
        .max(if plane[0] == 0. && plane[1] == 0. {
            plane[2].abs()
        } else {
            0.
        });
    if scale != 0. {
        for v in &mut plane {
            *v /= scale;
        }
    }
    plane
}

pub(crate) fn polygon_orientation(vertices: &[[f64; 2]]) -> Option<f64> {
    let mut sign = 0.;
    for i in 0..vertices.len() {
        let plane = polygon_plane(vertices[i], vertices[(i + 1) % vertices.len()]);
        let cross = dot(plane, homogeneous(vertices[(i + 2) % vertices.len()]));
        if cross.abs() > 1e-10 {
            if sign == 0. {
                sign = cross.signum();
            } else if sign * cross < 0. {
                return None;
            }
        }
    }
    (sign != 0.).then_some(sign)
}

pub(crate) fn polygon_corner(vertices: &[[f64; 2]], i: usize, orientation: f64) -> [f64; 2] {
    let a = polygon_plane(
        vertices[(i + vertices.len() - 1) % vertices.len()],
        vertices[i],
    );
    let b = polygon_plane(vertices[i], vertices[(i + 1) % vertices.len()]);
    let al = a[0].hypot(a[1]);
    let bl = b[0].hypot(b[1]);
    if al == 0. || bl == 0. {
        return [0.; 2];
    }
    let sum = [a[0] / al + b[0] / bl, a[1] / al + b[1] / bl];
    let length = sum[0].hypot(sum[1]);
    if length == 0. {
        [0.; 2]
    } else {
        sum.map(|v| -orientation * v / length)
    }
}

fn compare(value: f64, bound: f64, on: bool) -> bool {
    if on {
        value <= bound + 1e-10
    } else {
        value < bound - 1e-10
    }
}

impl Primitive {
    pub(crate) fn contains(&self, p: Point, margin: Point, on: bool, mode: RasterMode) -> bool {
        let auto = mode.auto;
        match self {
            #[cfg(feature = "lua")]
            Self::Lua(callback) => !matches!(
                callback.call(p),
                mlua::Value::Nil | mlua::Value::Boolean(false)
            ),
            #[cfg(not(feature = "lua"))]
            Self::Lua(callback) => callback.contains(p),
            Self::Mesh(triangles) => compare(crate::stl::distance(triangles, p), margin[0], on),
            Self::Box { min, max } => (0..3).all(|i| {
                compare(min[i] - margin[i], p[i], on || margin[i] != 0.)
                    && compare(p[i], max[i] + margin[i], on || margin[i] != 0.)
            }),
            Self::Ellipsoid {
                center,
                radii,
                dimensions,
            } => {
                let sum = (0..*dimensions)
                    .map(|i| ((p[i] - center[i]) / (radii[i] + margin[i])).powi(2))
                    .sum();
                compare(sum, 1., on || margin[0] > 0.)
            }
            Self::Cylinder {
                center,
                radii,
                length,
            } => {
                compare(
                    ((p[0] - center[0]) / (radii[0] + margin[0])).powi(2)
                        + ((p[1] - center[1]) / (radii[1] + margin[1])).powi(2),
                    1.,
                    on,
                ) && compare(center[2] - length - margin[2], p[2], on)
                    && compare(p[2], center[2] + margin[2], on)
            }
            Self::Polygon(vertices) => {
                if auto
                    && vertices.iter().flatten().all(|v| v.is_finite())
                    && (0..2).any(|i| {
                        let min =
                            vertices.iter().map(|p| p[i]).fold(f64::INFINITY, f64::min) - margin[i];
                        let max = vertices
                            .iter()
                            .map(|p| p[i])
                            .fold(f64::NEG_INFINITY, f64::max)
                            + margin[i];
                        !compare(min, p[i], on) || !compare(p[i], max, on)
                    })
                {
                    return false;
                }
                if let Some(area) = polygon_orientation(vertices) {
                    return (0..vertices.len()).all(|i| {
                        let plane = polygon_plane(vertices[i], vertices[(i + 1) % vertices.len()]);
                        if plane == [0.; 3] {
                            return true;
                        }
                        compare(
                            -dot(plane, [p[0], p[1], 1.]) * area,
                            polygon_margin(plane[1], plane[0], margin),
                            on,
                        ) && (!vertices[i].iter().all(|v| v.is_finite()) || {
                            let normal = polygon_corner(vertices, i, area);
                            normal == [0.; 2]
                                || compare(
                                    normal[0] * (p[0] - vertices[i][0])
                                        + normal[1] * (p[1] - vertices[i][1]),
                                    (normal[0] * margin[0])
                                        .hypot(normal[1] * margin[1])
                                        .copysign(margin[0]),
                                    on,
                                )
                        })
                    });
                }
                let mut inside = false;
                let mut distance = f64::INFINITY;
                for i in 0..vertices.len() {
                    let a = vertices[i];
                    let b = vertices[(i + 1) % vertices.len()];
                    let dx = b[0] - a[0];
                    let dy = b[1] - a[1];
                    let len2 = dx * dx + dy * dy;
                    let t = if len2 == 0. {
                        0.
                    } else {
                        ((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / len2
                    }
                    .clamp(0., 1.);
                    let factor = if margin[0] == 0. || len2 == 0. {
                        1.
                    } else {
                        (polygon_margin(dx, dy, margin) / margin[0]) / len2.sqrt()
                    };
                    distance =
                        distance.min((p[0] - a[0] - t * dx).hypot(p[1] - a[1] - t * dy) / factor);
                    if (a[1] > p[1]) != (b[1] > p[1])
                        && p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0]
                    {
                        inside = !inside;
                    }
                }
                let signed = if inside { -distance } else { distance };
                compare(signed, margin[0], on)
            }
            Self::Points { points, dimensions } => points.iter().any(|q| {
                (0..*dimensions).all(|i| {
                    let m = if mode.auto && !mode.fractional {
                        margin[i] * (0.5 / 0.35)
                    } else {
                        margin[i]
                    };
                    compare((p[i] - q[i]).abs(), m, on)
                })
            }),
            Self::Hyperbola { center, radii } => {
                let x = p[0] - center[0];
                let y = (p[1] - center[1]).abs();
                if auto {
                    let curve = radii[1] * (1. + (x / radii[0]).powi(2)).sqrt();
                    let slope = radii[1].powi(2) * x / (radii[0].powi(2) * curve);
                    let offset = margin[1].hypot(slope * margin[0]).copysign(margin[0]);
                    return compare(curve - y, offset, on);
                }
                let test =
                    (y / (radii[1] - margin[1])).powi(2) - (x / (radii[0] + margin[0])).powi(2);
                compare(1., test, on)
            }
            Self::Parabola { vertex, focus } => {
                let x = p[0] - vertex[0];
                let y = p[1] - vertex[1];
                let f = x * x / (4. * focus);
                let distance = (f - y) * focus.signum();
                let slope = x / (2. * focus);
                compare(
                    distance,
                    (margin[1].powi(2) + (slope * margin[0]).powi(2))
                        .sqrt()
                        .copysign(margin[0]),
                    on,
                )
            }
            Self::HalfSpace { origin, normal } => {
                let q = std::array::from_fn(|i| p[i] - origin[i]);
                let distance = dot(q, *normal);
                let m = (0..3)
                    .map(|i| (normal[i] * margin[i]).powi(2))
                    .sum::<f64>()
                    .sqrt()
                    .copysign(margin[0]);
                compare(distance, m, on)
            }
            Self::Cylinder3d { start, end, radius } => {
                let (origin, axis, min, max) = cylinder_frame(*start, *end);
                let q = std::array::from_fn(|i| p[i] - origin[i]);
                let axial = dot(q, axis);
                let radial = (dot(q, q) - axial * axial).max(0.).sqrt();
                compare(min - margin[0], axial, on)
                    && compare(axial, max + margin[0], on)
                    && compare(radial, radius + margin[0], on)
            }
        }
    }
}

pub(crate) fn polygon_margin(dx: f64, dy: f64, margin: Point) -> f64 {
    let x = (dy * margin[0]).abs();
    let y = (dx * margin[1]).abs();
    (x.max(y) + (std::f64::consts::SQRT_2 - 1.) * x.min(y)).copysign(margin[0])
}

impl Shape {
    pub(crate) fn has_potential(&self) -> bool {
        match self {
            Self::Potential(..) => true,
            Self::Transform(_, shape) => shape.has_potential(),
            _ => false,
        }
    }

    pub(crate) fn potential(&self, p: Point) -> Option<f64> {
        match self {
            Self::Potential(callback, _) => Some(callback.potential(p)),
            Self::Transform(t, shape) => shape.potential(t.point(p)),
            _ => None,
        }
    }

    pub(crate) fn check_callbacks(&self) -> Result<()> {
        match self {
            Self::Primitive(Primitive::Lua(callback)) => callback.check()?,
            Self::Potential(callback, shape) => {
                callback.check()?;
                shape.check_callbacks()?;
            }
            Self::Transform(_, shape)
            | Self::Revolve { shape, .. }
            | Self::Extrude { shape, .. } => shape.check_callbacks()?,
            Self::Intersection(shapes) => {
                for shape in shapes {
                    shape.check_callbacks()?;
                }
            }
            Self::Region(regions) => {
                for region in regions {
                    region.shape.check_callbacks()?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    pub(crate) fn segment_single_interval(&self, from: Point, to: Point, level: f64) -> bool {
        match self {
            Self::Primitive(
                Primitive::Box { .. }
                | Primitive::Cylinder3d { .. }
                | Primitive::Ellipsoid { .. }
                | Primitive::Cylinder { .. }
                | Primitive::HalfSpace { .. },
            ) => true,
            Self::Transform(t, shape) => {
                let norm1 = (0..3)
                    .map(|j| (0..3).map(|i| t.inverse[i][j].abs()).sum::<f64>())
                    .fold(0.0, f64::max);
                let norm_inf = t
                    .inverse
                    .iter()
                    .map(|r| r.iter().map(|v| v.abs()).sum::<f64>())
                    .fold(0.0, f64::max);
                shape.segment_single_interval(
                    t.point(from),
                    t.point(to),
                    level * (norm1 * norm_inf).sqrt(),
                )
            }
            Self::Intersection(shapes) => shapes
                .iter()
                .all(|s| s.segment_single_interval(from, to, level)),
            Self::Region(regions) => {
                let mut included = 0;
                for region in regions {
                    let Ok(bounds) = region.shape.distance_bounds(from, to) else {
                        return false;
                    };
                    if region.exclude {
                        if bounds[0] < -level {
                            return false;
                        }
                    } else if bounds[0] <= level {
                        included += 1;
                        if included > 1 || !region.shape.segment_single_interval(from, to, level) {
                            return false;
                        }
                    }
                }
                true
            }
            _ => false,
        }
    }

    /// Interval enclosure used only to cull segments, never to replace SDF
    /// samples or to move a surface root.
    pub(crate) fn distance_bounds(&self, from: Point, to: Point) -> Result<[f64; 2]> {
        let bounds = match self {
            Self::Primitive(Primitive::Box { min, max }) => {
                let lower: Point = std::array::from_fn(|a| {
                    (min[a] - from[a].max(to[a])).max(from[a].min(to[a]) - max[a])
                });
                let upper: Point = std::array::from_fn(|a| {
                    (min[a] - from[a].min(to[a])).max(from[a].max(to[a]) - max[a])
                });
                [lower, upper].map(|q| {
                    let outside = q.map(|v| v.max(0.0));
                    dot(outside, outside).sqrt()
                        + q.into_iter().fold(f64::NEG_INFINITY, f64::max).min(0.0)
                })
            }
            Self::Primitive(Primitive::Cylinder3d { start, end, radius }) => {
                let (origin, axis, min, max) = cylinder_frame(*start, *end);
                let p: Point = std::array::from_fn(|a| from[a] - origin[a]);
                let q: Point = std::array::from_fn(|a| to[a] - origin[a]);
                let axial = [dot(p, axis), dot(q, axis)];
                let radial: [Point; 2] = [
                    std::array::from_fn(|a| p[a] - axial[0] * axis[a]),
                    std::array::from_fn(|a| q[a] - axial[1] * axis[a]),
                ];
                let delta: Point = std::array::from_fn(|a| radial[1][a] - radial[0][a]);
                let length2 = dot(delta, delta);
                let t = if length2 > 0.0 {
                    (-dot(radial[0], delta) / length2).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let nearest: Point = std::array::from_fn(|a| radial[0][a] + t * delta[a]);
                // The scalar SDF subtracts axial^2 from |p|^2. Enclose that
                // cancellation, as well as the projected-segment arithmetic.
                let roundoff = 128.0 * f64::EPSILON * dot(p, p).max(dot(q, q));
                let low = (dot(nearest, nearest) - roundoff).max(0.0).sqrt() - radius;
                let high = (dot(radial[0], radial[0]).max(dot(radial[1], radial[1])) + roundoff)
                    .sqrt()
                    - radius;
                [
                    low.max((min - axial[0].max(axial[1])).max(axial[0].min(axial[1]) - max)),
                    high.max((min - axial[0].min(axial[1])).max(axial[0].max(axial[1]) - max)),
                ]
            }
            Self::Intersection(shapes) => {
                let mut bounds = [-f64::MAX; 2];
                for shape in shapes {
                    let next = shape.distance_bounds(from, to)?;
                    for a in 0..2 {
                        bounds[a] = bounds[a].max(next[a]);
                    }
                }
                bounds
            }
            Self::Region(regions) => {
                let mut bounds = if regions.iter().any(|r| !r.exclude) {
                    [f64::MAX; 2]
                } else {
                    [-f64::MAX; 2]
                };
                for region in regions.iter().filter(|r| !r.exclude) {
                    let next = region.shape.distance_bounds(from, to)?;
                    for a in 0..2 {
                        bounds[a] = bounds[a].min(next[a]);
                    }
                }
                for region in regions.iter().filter(|r| r.exclude) {
                    let next = region.shape.distance_bounds(from, to)?;
                    bounds = [bounds[0].max(-next[1]), bounds[1].max(-next[0])];
                }
                bounds
            }
            _ => {
                let distance = self.signed_distance(from)?;
                let delta = std::array::from_fn(|a| to[a] - from[a]);
                let length = dot(delta, delta).sqrt();
                [distance - length, distance + length]
            }
        };
        Ok(bounds)
    }

    pub(crate) fn validate_sdf(&self) -> Result<()> {
        match self {
            Self::Primitive(
                Primitive::Ellipsoid { dimensions, .. } | Primitive::Points { dimensions, .. },
            ) if !(1..=3).contains(dimensions) => {
                return Err(Error("invalid source primitive dimensions".into()));
            }
            Self::Transform(_, shape)
            | Self::Revolve { shape, .. }
            | Self::Extrude { shape, .. } => shape.validate_sdf()?,
            Self::Intersection(shapes) => {
                for shape in shapes {
                    shape.validate_sdf()?;
                }
            }
            Self::Region(regions) => {
                for region in regions {
                    region.shape.validate_sdf()?;
                }
            }
            _ => {}
        }
        self.signed_distance([0.0; 3])?;
        Ok(())
    }

    /// Conservative signed distance in the enclosing coordinate system.
    /// Negative values are inside conductor material; CSG uses min/max.
    pub fn signed_distance(&self, p: Point) -> Result<f64> {
        let distance=match self {
            Self::Primitive(primitive) => match primitive {
                Primitive::Mesh(triangles)=>crate::stl::distance(triangles,p),
                Primitive::Box {min,max} => {
                    let q:Point=std::array::from_fn(|i|(min[i]-p[i]).max(p[i]-max[i]));
                    let outside=q.map(|v|v.max(0.));
                    dot(outside,outside).sqrt()+q.into_iter().fold(f64::NEG_INFINITY,f64::max).min(0.)
                }
                Primitive::Ellipsoid {center,radii,dimensions} => {
                    let norm=(0..*dimensions).map(|i|((p[i]-center[i])/radii[i]).powi(2)).sum::<f64>().sqrt();
                    (norm-1.)*radii[..*dimensions].iter().copied().fold(f64::INFINITY,f64::min)
                }
                Primitive::Cylinder {center,radii,length} => {
                    let radial=(((p[0]-center[0])/radii[0]).powi(2)+((p[1]-center[1])/radii[1]).powi(2)).sqrt()-1.;
                    (radial*radii[0].min(radii[1])).max((center[2]-length-p[2]).max(p[2]-center[2]))
                }
                Primitive::Polygon(vertices) => {
                    let mut inside=false;
                    let mut nearest=f64::INFINITY;
                    for i in 0..vertices.len() {
                        let a=vertices[i];
                        let b=vertices[(i+1)%vertices.len()];
                        let dx=b[0]-a[0];
                        let dy=b[1]-a[1];
                        let len2=dx*dx+dy*dy;
                        let t=if len2==0. {0.} else {((p[0]-a[0])*dx+(p[1]-a[1])*dy)/len2}.clamp(0.,1.);
                        nearest=nearest.min((p[0]-a[0]-t*dx).hypot(p[1]-a[1]-t*dy));
                        if (a[1]>p[1])!=(b[1]>p[1]) && p[0]<(b[0]-a[0])*(p[1]-a[1])/(b[1]-a[1])+a[0] {
                            inside = !inside;
                        }
                    }
                    if inside {-nearest} else {nearest}
                }
                Primitive::Points {points,dimensions} => points.iter().map(|q|
                    (0..*dimensions).map(|i|(p[i]-q[i]).powi(2)).sum::<f64>().sqrt()
                ).fold(f64::INFINITY,f64::min),
                Primitive::HalfSpace {origin,normal} => {
                    dot(std::array::from_fn(|i|p[i]-origin[i]),*normal)/dot(*normal,*normal).sqrt()
                }
                Primitive::Cylinder3d {start,end,radius} => {
                    let (origin,axis,min,max)=cylinder_frame(*start,*end);
                    let q=std::array::from_fn(|i|p[i]-origin[i]);
                    let axial=dot(q,axis);
                    ((dot(q,q)-axial*axial).max(0.).sqrt()-radius).max((min-axial).max(axial-max))
                }
                Primitive::Lua(_) => return Err(Error("signed-distance sampling of Lua shapes is unsupported; GEM rasterization remains available".into())),
                Primitive::Hyperbola {..}|Primitive::Parabola {..} => return Err(Error("signed-distance sampling of hyperbola/parabola is unsupported; GEM rasterization remains available".into())),
            },
            Self::Potential(..) => return Err(Error("signed-distance sampling of Lua potentials is unsupported; GEM rasterization remains available".into())),
            Self::Transform(t,shape) => {
                let norm1=(0..3).map(|j|(0..3).map(|i|t.inverse[i][j].abs()).sum::<f64>()).fold(0.,f64::max);
                let norm_inf=t.inverse.iter().map(|r|r.iter().map(|x|x.abs()).sum::<f64>()).fold(0.,f64::max);
                shape.signed_distance(t.point(p))?/(norm1*norm_inf).sqrt()
            }
            Self::Intersection(shapes) => {
                let mut distance=-f64::MAX;
                for s in shapes {distance=distance.max(s.signed_distance(p)?);}
                distance
            }
            Self::Region(regions) => {
                let mut distance=if regions.iter().any(|r|!r.exclude) {f64::MAX} else {-f64::MAX};
                for r in regions.iter().filter(|r|!r.exclude) {distance=distance.min(r.shape.signed_distance(p)?);}
                for r in regions.iter().filter(|r|r.exclude) {distance=distance.max(-r.shape.signed_distance(p)?);}
                distance
            }
            Self::Revolve {shape,start,end,axis,..} => {
                let (ax,r1,r2)=match axis {0=>(0,1,2),1=>(1,2,0),_=>(2,0,1)};
                let distance=shape.signed_distance([p[ax],p[r1].hypot(p[r2]),0.])?;
                if end-start>=360. {distance} else {
                    let (s,c)=start.to_radians().sin_cos();
                    let first=s*p[r1]-c*p[r2];
                    let (s,c)=end.to_radians().sin_cos();
                    let last=c*p[r2]-s*p[r1];
                    distance.max(if end-start<=180. {first.max(last)} else {first.min(last)})
                }
            }
            Self::Extrude {shape,min,max,axis} => {
                let (a,b,c)=match axis {2=>(0,1,2),0=>(1,2,0),_=>(2,0,1)};
                shape.signed_distance([p[a],p[b],0.])?.max((min-p[c]).max(p[c]-max))
            }
        };
        if !distance.is_finite() {
            return Err(Error(
                "signed-distance sampling requires finite geometry values".into(),
            ));
        }
        Ok(distance)
    }

    pub(crate) fn contains(&self, p: Point, margin: Point, on: bool, mode: RasterMode) -> bool {
        match self {
            Self::Potential(_, shape) => shape.contains(p, margin, on, mode),
            Self::Primitive(s) => s.contains(p, margin, on, mode),
            Self::Transform(t, s) => s.contains(
                t.point(p),
                t.margin(margin).map(|m| m.copysign(margin[0])),
                on,
                mode,
            ),
            Self::Intersection(shapes) => shapes.iter().all(|s| s.contains(p, margin, on, mode)),
            Self::Region(regions) => {
                let mut included = !regions.iter().any(|r| !r.exclude);
                for r in regions {
                    let (m, on) = mode.inclusion(r, margin, on);
                    let hit = r.shape.contains(p, m, on, mode);
                    if r.exclude && hit {
                        return false;
                    }
                    if !r.exclude && hit {
                        included = true;
                    }
                }
                included
            }
            Self::Revolve {
                shape,
                start,
                end,
                axis,
            } => {
                let (ax, r1, r2) = match axis {
                    0 => (0, 1, 2),
                    1 => (1, 2, 0),
                    _ => (2, 0, 1),
                };
                let angle = p[r2].atan2(p[r1]).to_degrees();
                let angle = (angle - start).rem_euclid(360.);
                let radius = p[r1].hypot(p[r2]);
                let width = end - start;
                let allowed =
                    width >= 360. - 1e-10 || angle <= width + 1e-10 || angle >= 360. - 1e-10;
                if width < 360. - 1e-10 && radius == 0. {
                    return false;
                }
                allowed && shape.contains([p[ax], radius, 0.], margin, on, mode)
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
                compare(min - margin[c], p[c], on)
                    && compare(p[c], max + margin[c], on)
                    && shape.contains(
                        [p[a], p[b], 0.],
                        [margin[a], margin[b], margin[c]],
                        on,
                        mode,
                    )
            }
        }
    }
}

pub(crate) fn transform(node: &Node) -> Result<Option<Transform>> {
    let n = |i, d| node.num(i, d);
    let mut t = Transform::identity();
    let (scale, angles) = match node.name.as_str() {
        "locate" | "project" | "project_it" | "transform" => {
            t.origin = [n(0, 0.)?, n(1, 0.)?, n(2, 0.)?];
            let s = n(3, 1.)?;
            (
                [s * n(7, 1.)?, s * n(8, 1.)?, s * n(9, 1.)?],
                [n(6, 0.)?, n(4, 0.)?, n(5, 0.)?],
            )
        }
        "scale" => {
            let x = n(0, 1.)?;
            ([x, n(1, x)?, n(2, x)?], [0.; 3])
        }
        "rotate_x" => ([1.; 3], [n(0, 0.)?, 0., 0.]),
        "rotate_y" => ([1.; 3], [0., n(0, 0.)?, 0.]),
        "rotate_z" => ([1.; 3], [0., 0., n(0, 0.)?]),
        _ => return Ok(None),
    };
    if scale.iter().any(|x| !x.is_finite() || *x == 0.) {
        return Err(Error(format!(
            "line {}: locate/scale requires nonzero finite scaling",
            node.line
        )));
    }
    // Forward order Rx, Rz, Ry. Compose inverse as Rx^-1 Rz^-1 Ry^-1 S^-1.
    let trig = |angle: f64| {
        let (s, c) = angle.to_radians().sin_cos();
        (
            if s.abs() < 1e-15 { 0. } else { s },
            if c.abs() < 1e-15 { 0. } else { c },
        )
    };
    let (sx, cx) = trig(angles[0]);
    let (sy, cy) = trig(angles[1]);
    let (sz, cz) = trig(angles[2]);
    let rx = [[1., 0., 0.], [0., cx, sx], [0., -sx, cx]];
    let rz = [[cz, sz, 0.], [-sz, cz, 0.], [0., 0., 1.]];
    let ry = [[cy, 0., -sy], [0., 1., 0.], [sy, 0., cy]];
    let multiply = |a: [[f64; 3]; 3], b: [[f64; 3]; 3]| {
        std::array::from_fn(|i| std::array::from_fn(|j| (0..3).map(|k| a[i][k] * b[k][j]).sum()))
    };
    let r = multiply(multiply(rx, rz), ry);
    t.inverse = std::array::from_fn(|i| std::array::from_fn(|j| r[i][j] / scale[j]));
    Ok(Some(t))
}

pub(crate) fn primitive(node: &Node) -> Result<Option<Primitive>> {
    let n = |i, d| node.num(i, d);
    let name = node.name.as_str();
    let shape = match name {
        "stl" => Primitive::Mesh(crate::stl::read(
            std::path::Path::new(
                node.args
                    .first()
                    .ok_or_else(|| Error("stl requires a filename".into()))?
                    .text()?,
            ),
            if node.args.len() == 4 {
                Some([n(1, 0.)?, n(2, 0.)?, n(3, 0.)?])
            } else if node.args.len() == 1 {
                None
            } else {
                return Err(Error(
                    "stl takes a filename and optional x,y,z selector".into(),
                ));
            },
        )?),
        "box" | "box2d" | "box3d" | "centered_box" | "centered_box2d" | "centered_box3d"
        | "cent_box" | "cent_box2d" | "cent_box3d" | "corner_box" | "corner_box2d"
        | "corner_box3d" | "corn_box" | "corn_box2d" | "corn_box3d" => {
            let d = if name.ends_with("3d") { 3 } else { 2 };
            let mut min = [-1e6; 3];
            let mut max = [1e6; 3];
            for i in 0..d {
                let start = n(i, 0.)?;
                let end = n(i + d, 10.)?;
                if name.starts_with("cent") {
                    min[i] = start - end.abs() / 2.;
                    max[i] = start + end.abs() / 2.;
                } else if name.starts_with("corn") {
                    min[i] = start;
                    max[i] = start + end;
                } else {
                    min[i] = start.min(end);
                    max[i] = start.max(end);
                }
            }
            Primitive::Box { min, max }
        }
        "circle" | "ellipse" | "sphere" | "ellipsoid" => {
            let d = if matches!(name, "circle" | "ellipse") {
                2
            } else {
                3
            };
            let center = if d == 2 {
                [n(0, 0.)?, n(1, 0.)?, 0.]
            } else {
                [n(0, 0.)?, n(1, 0.)?, n(2, 0.)?]
            };
            let rx = n(d, 10.)?.abs();
            let ry = n(d + 1, rx)?.abs();
            let rz = if d == 3 { n(5, ry)?.abs() } else { 1e6 };
            if [rx, ry, rz].contains(&0.) {
                return Err(Error("circle/sphere radii must be nonzero".into()));
            }
            Primitive::Ellipsoid {
                center,
                radii: [rx, ry, rz],
                dimensions: d,
            }
        }
        "cylinder" => {
            let rx = n(3, 10.)?.abs();
            Primitive::Cylinder {
                center: [n(0, 0.)?, n(1, 0.)?, n(2, 0.)?],
                radii: [rx, n(4, rx)?.abs()],
                length: n(5, 10.)?.abs(),
            }
        }
        "polyline" => {
            if node.args.len() < 6 || !node.args.len().is_multiple_of(2) {
                return Err(Error("polyline requires at least three x,y pairs".into()));
            }
            let points = node
                .args
                .as_chunks::<2>()
                .0
                .iter()
                .map(|p| Ok([p[0].number()?, p[1].number()?]))
                .collect::<Result<Vec<_>>>()?;
            if points.iter().flatten().any(|v| v.is_nan()) {
                return Err(Error("polyline coordinates cannot be NaN".into()));
            }
            Primitive::Polygon(points)
        }
        "points" | "points2d" | "points3d" => {
            let d = if name == "points3d" { 3 } else { 2 };
            if node.args.is_empty() || !node.args.len().is_multiple_of(d) {
                return Err(Error("points requires complete coordinate tuples".into()));
            }
            let points = node
                .args
                .chunks_exact(d)
                .map(|p| {
                    Ok([
                        p[0].number()?,
                        p[1].number()?,
                        if d == 3 { p[2].number()? } else { 0. },
                    ])
                })
                .collect::<Result<_>>()?;
            Primitive::Points {
                points,
                dimensions: d,
            }
        }
        "hyperbola" => Primitive::Hyperbola {
            center: [n(0, 0.)?, n(1, 0.)?],
            radii: [n(2, 10.)?.abs(), n(3, n(2, 10.)?)?.abs()],
        },
        "parabola" => {
            let focus = n(2, 10.)?;
            if focus == 0. {
                return Err(Error("parabola focus offset cannot be zero".into()));
            }
            Primitive::Parabola {
                vertex: [n(0, 0.)?, n(1, 0.)?],
                focus,
            }
        }
        "half_space" => {
            if let Ok(axis) = node.string(0, "") {
                let axis = axis.as_bytes();
                if axis.len() != 2 || !b"+-".contains(&axis[0]) || !b"xyz".contains(&axis[1]) {
                    return Err(Error(
                        "half_space axis must be '+x', '-x', '+y', '-y', '+z', or '-z'".into(),
                    ));
                }
                let i = (axis[1] - b'x') as usize;
                let mut origin = [0.; 3];
                origin[i] = n(1, 0.)?;
                let mut normal = [0.; 3];
                normal[i] = if axis[0] == b'-' { 1. } else { -1. };
                Primitive::HalfSpace { origin, normal }
            } else {
                let normal = [n(3, 1.)?, n(4, 0.)?, n(5, 0.)?];
                if dot(normal, normal) == 0. {
                    return Err(Error("half_space normal cannot be zero".into()));
                }
                Primitive::HalfSpace {
                    origin: [n(0, 0.)?, n(1, 0.)?, n(2, 0.)?],
                    normal,
                }
            }
        }
        "cylinder3d" => {
            let start = [n(0, 0.)?, n(1, 0.)?, n(2, 0.)?];
            let end = [n(3, 0.)?, n(4, 0.)?, n(5, 10.)?];
            let infinite_axes = (0..3)
                .filter(|&i| !start[i].is_finite() || !end[i].is_finite())
                .collect::<Vec<_>>();
            if start == end
                || start.iter().chain(&end).any(|x| x.is_nan())
                || (!infinite_axes.is_empty()
                    && (infinite_axes.len() != 1
                        || (0..3).any(|i| i != infinite_axes[0] && start[i] != end[i])))
            {
                return Err(Error(
                    "cylinder3d requires distinct endpoints; infinite endpoints must share one axis".into(),
                ));
            }
            Primitive::Cylinder3d {
                start,
                end,
                radius: n(6, 10.)?.abs(),
            }
        }
        _ => return Ok(None),
    };
    if !node.children.is_empty() {
        return Err(Error(format!(
            "line {}: primitive '{}' cannot have a block",
            node.line, node.name
        )));
    }
    Ok(Some(shape))
}

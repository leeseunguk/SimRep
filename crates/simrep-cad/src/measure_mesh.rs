//! Independent Euclidean triangle-set minimum; no OCCT extrema queries.
use simrep_geom::Point;

type Triangle = [Point; 3];
type Bounds = [Point; 2];

fn sub(a: Point, b: Point) -> Point {
    std::array::from_fn(|i| a[i] - b[i])
}
fn dot(a: Point, b: Point) -> f64 {
    (0..3).map(|i| a[i] * b[i]).sum()
}
fn cross(a: Point, b: Point) -> Point {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn along(a: Point, b: Point, t: f64) -> Point {
    std::array::from_fn(|i| a[i] + t * (b[i] - a[i]))
}
fn point_segment(p: Point, a: Point, b: Point) -> f64 {
    let v = sub(b, a);
    let t = if dot(v, v) == 0.0 {
        0.0
    } else {
        (dot(sub(p, a), v) / dot(v, v)).clamp(0.0, 1.0)
    };
    let d = sub(p, along(a, b, t));
    dot(d, d)
}
fn inside(p: Point, tri: Triangle, normal: Point) -> bool {
    (0..3).all(|i| dot(cross(sub(tri[(i + 1) % 3], tri[i]), sub(p, tri[i])), normal) >= 0.0)
}
fn point_triangle(p: Point, tri: Triangle) -> f64 {
    let normal = cross(sub(tri[1], tri[0]), sub(tri[2], tri[0]));
    let n2 = dot(normal, normal);
    if n2 > 0.0 {
        let height = dot(sub(p, tri[0]), normal);
        let projection = std::array::from_fn(|i| p[i] - normal[i] * height / n2);
        if inside(projection, tri, normal) {
            return height * height / n2;
        }
    }
    (0..3)
        .map(|i| point_segment(p, tri[i], tri[(i + 1) % 3]))
        .fold(f64::INFINITY, f64::min)
}
fn segment_segment(a: Point, b: Point, c: Point, d: Point) -> f64 {
    // The unconstrained line minimum can lie inside both segments. Otherwise
    // the constrained minimum lies on one of the four endpoints.
    let u = sub(b, a);
    let v = sub(d, c);
    let w = sub(a, c);
    let uu = dot(u, u);
    let vv = dot(v, v);
    let uv = dot(u, v);
    let det = dot(cross(u, v), cross(u, v));
    let mut best = point_segment(a, c, d)
        .min(point_segment(b, c, d))
        .min(point_segment(c, a, b))
        .min(point_segment(d, a, b));
    if det > 0.0 {
        let t = (uv * dot(v, w) - vv * dot(u, w)) / det;
        let s = (uu * dot(v, w) - uv * dot(u, w)) / det;
        if (0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&s) {
            let delta = sub(along(a, b, t), along(c, d, s));
            best = best.min(dot(delta, delta));
        }
    }
    best
}
fn crosses(a: Point, b: Point, tri: Triangle) -> bool {
    let normal = cross(sub(tri[1], tri[0]), sub(tri[2], tri[0]));
    let h0 = dot(sub(a, tri[0]), normal);
    let h1 = dot(sub(b, tri[0]), normal);
    let denominator = h0 - h1;
    if denominator == 0.0 {
        return false; // Coplanar intersections are covered by edge distances.
    }
    let t = h0 / denominator;
    (0.0..=1.0).contains(&t) && inside(along(a, b, t), tri, normal)
}
fn triangle_distance2(a: Triangle, b: Triangle) -> f64 {
    let mut best = f64::INFINITY;
    for i in 0..3 {
        if crosses(a[i], a[(i + 1) % 3], b) || crosses(b[i], b[(i + 1) % 3], a) {
            return 0.0;
        }
        best = best
            .min(point_triangle(a[i], b))
            .min(point_triangle(b[i], a));
        for j in 0..3 {
            best = best.min(segment_segment(a[i], a[(i + 1) % 3], b[j], b[(j + 1) % 3]));
        }
    }
    best
}
fn bounds(triangles: &[Triangle]) -> Bounds {
    let mut bounds = [[f64::INFINITY; 3], [f64::NEG_INFINITY; 3]];
    for p in triangles.iter().flatten() {
        for (i, &value) in p.iter().enumerate() {
            bounds[0][i] = bounds[0][i].min(value);
            bounds[1][i] = bounds[1][i].max(value);
        }
    }
    bounds
}
fn box_distance2(a: Bounds, b: Bounds) -> f64 {
    (0..3)
        .map(|i| (a[0][i] - b[1][i]).max(b[0][i] - a[1][i]).max(0.0).powi(2))
        .sum()
}
struct Node {
    bounds: Bounds,
    start: usize,
    end: usize,
    children: Option<Box<[Node; 2]>>,
}
impl Node {
    fn build(triangles: &mut [Triangle], start: usize) -> Self {
        let bounds = bounds(triangles);
        let children = if triangles.len() > 8 {
            let axis = (0..3)
                .max_by(|&a, &b| {
                    (bounds[1][a] - bounds[0][a]).total_cmp(&(bounds[1][b] - bounds[0][b]))
                })
                .unwrap();
            let mid = triangles.len() / 2;
            triangles.select_nth_unstable_by(mid, |a, b| {
                a.iter()
                    .map(|p| p[axis])
                    .sum::<f64>()
                    .total_cmp(&b.iter().map(|p| p[axis]).sum())
            });
            let (left, right) = triangles.split_at_mut(mid);
            Some(Box::new([
                Self::build(left, start),
                Self::build(right, start + mid),
            ]))
        } else {
            None
        };
        Self {
            bounds,
            start,
            end: start + triangles.len(),
            children,
        }
    }
    fn minimum(&self, other: &Self, a: &[Triangle], b: &[Triangle], best: &mut f64) {
        if box_distance2(self.bounds, other.bounds) >= *best {
            return;
        }
        match (&self.children, &other.children) {
            (Some(children), _)
                if other.children.is_none() || self.end - self.start >= other.end - other.start =>
            {
                let first = usize::from(
                    box_distance2(children[0].bounds, other.bounds)
                        > box_distance2(children[1].bounds, other.bounds),
                );
                children[first].minimum(other, a, b, best);
                children[1 - first].minimum(other, a, b, best);
            }
            (_, Some(children)) => {
                let first = usize::from(
                    box_distance2(self.bounds, children[0].bounds)
                        > box_distance2(self.bounds, children[1].bounds),
                );
                self.minimum(&children[first], a, b, best);
                self.minimum(&children[1 - first], a, b, best);
            }
            _ => {
                for &ta in &a[self.start..self.end] {
                    for &tb in &b[other.start..other.end] {
                        *best = best.min(triangle_distance2(ta, tb));
                    }
                }
            }
        }
    }
}
pub(crate) fn minimum(mut a: Vec<Triangle>, mut b: Vec<Triangle>) -> f64 {
    let left = Node::build(&mut a, 0);
    let right = Node::build(&mut b, 0);
    let mut best = f64::INFINITY;
    left.minimum(&right, &a, &b, &mut best);
    best.sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interior_crossing_and_edge_edge_minima() {
        let a = [[-2.0, -1.0, 0.0], [2.0, -1.0, 0.0], [0.0, 2.0, 0.0]];
        let crossing = [[0.0, 0.0, -1.0], [0.0, 0.0, 1.0], [3.0, 0.0, 1.0]];
        assert_eq!(minimum(vec![a], vec![crossing]), 0.0);
        let edge_a = [[-2.0, 0.0, 0.0], [2.0, 0.0, 0.0], [-2.0, -1.0, 0.0]];
        let edge_b = [[0.0, -2.0, 0.7], [0.0, 2.0, 0.7], [0.0, -2.0, 1.7]];
        // No vertex has the minimum: a vertex-only cross-check would miss it.
        assert!(edge_a.iter().all(|&p| point_triangle(p, edge_b) > 0.49));
        assert!(edge_b.iter().all(|&p| point_triangle(p, edge_a) > 0.49));
        assert!((minimum(vec![edge_a], vec![edge_b]) - 0.7).abs() < 1e-12);
    }

    #[test]
    fn bvh_matches_exhaustive_minimum() {
        let triangles: Vec<_> = (0..24)
            .map(|i| {
                let x = i as f64;
                [[x, 0.0, 0.0], [x + 0.3, 0.0, 0.0], [x, 0.4, 0.1]]
            })
            .collect();
        let shifted: Vec<_> = triangles
            .iter()
            .map(|t| t.map(|p| [p[0] + 0.2, p[1] + 0.3, p[2] + 0.5]))
            .collect();
        let expected = triangles
            .iter()
            .flat_map(|a| shifted.iter().map(|b| triangle_distance2(*a, *b)))
            .fold(f64::INFINITY, f64::min)
            .sqrt();
        assert_eq!(minimum(triangles, shifted), expected);
    }
}

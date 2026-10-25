//! STL solids, including the connected-component selector used by GEM.
use std::collections::HashMap;
use std::path::Path;

use crate::geometry::{Point, dot};
use crate::{Error, Result};

pub type Triangle = [Point; 3];

fn sub(a: Point, b: Point) -> Point {
    std::array::from_fn(|i| a[i] - b[i])
}
fn cross(a: Point, b: Point) -> Point {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

pub(crate) fn intersections(triangles: &[Triangle], p: Point, d: Point) -> Vec<f64> {
    let mut out = Vec::new();
    for &[a, b, c] in triangles {
        let e1 = sub(b, a);
        let e2 = sub(c, a);
        let h = cross(d, e2);
        let det = dot(e1, h);
        if det.abs() < 1e-12 {
            continue;
        }
        let s = sub(p, a);
        let u = dot(s, h) / det;
        if !(-1e-10..=1. + 1e-10).contains(&u) {
            continue;
        }
        let q = cross(s, e1);
        let v = dot(d, q) / det;
        if v < -1e-10 || u + v > 1. + 1e-10 {
            continue;
        }
        out.push(dot(e2, q) / det);
    }
    out.sort_by(f64::total_cmp);
    out.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
    out
}

fn triangle_distance(p: Point, [a, b, c]: Triangle) -> f64 {
    let ab = sub(b, a);
    let ac = sub(c, a);
    let ap = sub(p, a);
    let (d1, d2) = (dot(ab, ap), dot(ac, ap));
    if d1 <= 0. && d2 <= 0. {
        return dot(ap, ap).sqrt();
    }
    let bp = sub(p, b);
    let (d3, d4) = (dot(ab, bp), dot(ac, bp));
    if d3 >= 0. && d4 <= d3 {
        return dot(bp, bp).sqrt();
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0. && d1 >= 0. && d3 <= 0. {
        let v = d1 / (d1 - d3);
        return dot(sub(ap, ab.map(|x| v * x)), sub(ap, ab.map(|x| v * x))).sqrt();
    }
    let cp = sub(p, c);
    let (d5, d6) = (dot(ab, cp), dot(ac, cp));
    if d6 >= 0. && d5 <= d6 {
        return dot(cp, cp).sqrt();
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0. && d2 >= 0. && d6 <= 0. {
        let w = d2 / (d2 - d6);
        let q = sub(ap, ac.map(|x| w * x));
        return dot(q, q).sqrt();
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0. && d4 - d3 >= 0. && d5 - d6 >= 0. {
        let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
        let q = sub(bp, sub(c, b).map(|x| w * x));
        return dot(q, q).sqrt();
    }
    let n = cross(ab, ac);
    dot(ap, n).abs() / dot(n, n).sqrt()
}

pub(crate) fn distance(triangles: &[Triangle], p: Point) -> f64 {
    let nearest = triangles
        .iter()
        .map(|t| triangle_distance(p, *t))
        .fold(f64::INFINITY, f64::min);
    let hits = intersections(triangles, p, [1., 0., 0.]);
    if hits.iter().filter(|&&t| t > 0.).count() % 2 == 1 {
        -nearest
    } else {
        nearest
    }
}

pub(crate) fn read(path: &Path, selector: Option<Point>) -> Result<Vec<Triangle>> {
    let bytes = std::fs::read(path).map_err(|e| Error(format!("{}: {e}", path.display())))?;
    let mut triangles = Vec::new();
    if bytes.len() >= 84
        && 84 + u32::from_le_bytes(bytes[80..84].try_into().unwrap()) as usize * 50 == bytes.len()
    {
        for record in bytes[84..].chunks_exact(50) {
            let mut triangle = [[0.; 3]; 3];
            for (i, v) in triangle.iter_mut().flatten().enumerate() {
                *v = f32::from_le_bytes(record[12 + 4 * i..16 + 4 * i].try_into().unwrap()) as f64;
            }
            triangles.push(triangle);
        }
    } else {
        let text =
            std::str::from_utf8(&bytes).map_err(|_| Error("invalid binary or ASCII STL".into()))?;
        let mut words = text.split_whitespace();
        let mut vertices = Vec::new();
        while let Some(word) = words.next() {
            if word.eq_ignore_ascii_case("vertex") {
                let mut vertex = [0.; 3];
                for v in &mut vertex {
                    *v = words
                        .next()
                        .and_then(|s| s.parse().ok())
                        .ok_or_else(|| Error("invalid STL vertex".into()))?;
                }
                vertices.push(vertex);
            }
        }
        if !vertices.len().is_multiple_of(3) {
            return Err(Error("STL has an incomplete triangle".into()));
        }
        triangles.extend(vertices.chunks_exact(3).map(|v| [v[0], v[1], v[2]]));
    }
    if triangles.is_empty() || triangles.iter().flatten().flatten().any(|v| !v.is_finite()) {
        return Err(Error("STL requires finite, nonempty triangles".into()));
    }
    if let Some(point) = selector {
        let mut parent = (0..triangles.len()).collect::<Vec<_>>();
        fn root(parent: &mut [usize], mut i: usize) -> usize {
            while parent[i] != i {
                parent[i] = parent[parent[i]];
                i = parent[i];
            }
            i
        }
        let mut vertices = HashMap::new();
        for (i, triangle) in triangles.iter().enumerate() {
            for vertex in triangle {
                let key = vertex.map(|v| if v == 0. { 0 } else { v.to_bits() });
                if let Some(&j) = vertices.get(&key) {
                    let a = root(&mut parent, i);
                    let b = root(&mut parent, j);
                    parent[a] = b;
                } else {
                    vertices.insert(key, i);
                }
            }
        }
        let closest = triangles
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| {
                triangle_distance(point, **a).total_cmp(&triangle_distance(point, **b))
            })
            .unwrap()
            .0;
        let component = root(&mut parent, closest);
        triangles = triangles
            .into_iter()
            .enumerate()
            .filter_map(|(i, t)| (root(&mut parent, i) == component).then_some(t))
            .collect();
    }
    Ok(triangles)
}

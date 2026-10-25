use crate::{GridSpec, Mesh, Part, SurfaceOptions, invalid};
use simrep_core::{PotentialArray, Symmetry};
use std::collections::{BTreeMap, HashMap};
use std::io;

#[derive(Default)]
struct Section {
    vertices: Vec<[f64; 2]>,
    indices: HashMap<[u64; 2], usize>,
    edges: Vec<[usize; 2]>,
    caps: Vec<[usize; 3]>,
}

impl Section {
    fn vertex(&mut self, point: [f64; 2]) -> usize {
        let key = point.map(|v| if v == 0.0 { 0 } else { v.to_bits() });
        *self.indices.entry(key).or_insert_with(|| {
            self.vertices.push(point);
            self.vertices.len() - 1
        })
    }

    fn sweep(
        &self,
        name: String,
        cylindrical: bool,
        grid: GridSpec,
        options: SurfaceOptions,
    ) -> io::Result<Part> {
        let closed = cylindrical && options.sweep_degrees == 360.0;
        let steps = if cylindrical {
            options.angular_segments
        } else {
            1
        };
        let rings = if closed {
            steps
        } else {
            steps
                .checked_add(1)
                .ok_or_else(|| invalid("surface size overflow"))?
        };
        let count = self.vertices.len();
        let mut part = Part {
            name,
            ..Part::default()
        };
        let capacity = count
            .checked_mul(rings)
            .ok_or_else(|| invalid("surface size overflow"))?;
        let mut indices = Vec::with_capacity(capacity);
        for ring in 0..rings {
            let angle = options.sweep_degrees.to_radians() * ring as f64 / steps as f64;
            let (sin, cos) = angle.sin_cos();
            for (i, &[x, y]) in self.vertices.iter().enumerate() {
                let r = y - grid.origin_mm[1];
                if cylindrical && r == 0.0 && ring != 0 {
                    indices.push(indices[i]);
                    continue;
                }
                indices.push(part.vertices.len());
                part.vertices.push(if cylindrical {
                    [x, grid.origin_mm[1] + r * cos, grid.origin_mm[2] + r * sin]
                } else {
                    [
                        x,
                        y,
                        grid.origin_mm[2] + (ring as f64 - 0.5) * options.extrusion_depth_mm,
                    ]
                });
            }
        }
        let mut triangle = |a, b, c| {
            if a != b && b != c && c != a {
                part.triangles.push([a, b, c]);
            }
        };
        for ring in 0..steps {
            let next = (ring + 1) % rings;
            for &[a, b] in &self.edges {
                let [a, b, c, d] = [
                    indices[ring * count + a],
                    indices[ring * count + b],
                    indices[next * count + b],
                    indices[next * count + a],
                ];
                triangle(a, b, c);
                triangle(a, c, d);
            }
        }
        if !closed {
            for &[a, b, c] in &self.caps {
                triangle(indices[c], indices[b], indices[a]);
                let end = (rings - 1) * count;
                triangle(indices[end + a], indices[end + b], indices[end + c]);
            }
        }
        part.validate()?;
        Ok(part)
    }
}

pub(crate) fn electrodes(
    pa: &PotentialArray,
    grid: GridSpec,
    electrode: Option<f64>,
    options: SurfaceOptions,
) -> io::Result<Mesh> {
    let cylindrical = pa.symmetry == Symmetry::Cylindrical;
    let mut sections: BTreeMap<u64, Section> = BTreeMap::new();
    for y in 0..pa.ny {
        for x in 0..pa.nx {
            let i = x + pa.nx * y;
            let value = pa.potentials[i];
            if !pa.electrodes[i] || electrode.is_some_and(|e| e != value) {
                continue;
            }
            let key = if value == 0.0 { 0 } else { value.to_bits() };
            let section = sections.entry(key).or_default();
            let corner = |dx: usize, dy: usize| {
                [
                    grid.origin_mm[0] + (x as f64 + dx as f64 - 0.5) * grid.spacing_mm[0],
                    grid.origin_mm[1]
                        + if cylindrical && y == 0 && dy == 0 {
                            0.0
                        } else {
                            (y as f64 + dy as f64 - 0.5) * grid.spacing_mm[1]
                        },
                ]
            };
            let quad = [
                section.vertex(corner(0, 0)),
                section.vertex(corner(1, 0)),
                section.vertex(corner(1, 1)),
                section.vertex(corner(0, 1)),
            ];
            section
                .caps
                .extend([[quad[0], quad[1], quad[2]], [quad[0], quad[2], quad[3]]]);
            for (side, (dx, dy)) in [(0, -1), (1, 0), (0, 1), (-1, 0)].into_iter().enumerate() {
                let neighbor = x.checked_add_signed(dx).zip(y.checked_add_signed(dy));
                if let Some((xx, yy)) = neighbor
                    && let Some(j) = pa.index(xx, yy, 0)
                    && pa.electrodes[j]
                    && pa.potentials[j] == value
                {
                    continue;
                }
                section.edges.push([quad[side], quad[(side + 1) % 4]]);
            }
        }
    }
    let parts = sections
        .into_iter()
        .map(|(key, section)| {
            section.sweep(
                format!("electrode-{}", f64::from_bits(key)),
                cylindrical,
                grid,
                options,
            )
        })
        .collect::<io::Result<_>>()?;
    let mesh = Mesh { parts };
    mesh.validate()?;
    Ok(mesh)
}

pub(crate) fn isopotential(
    pa: &PotentialArray,
    grid: GridSpec,
    level: f64,
    options: SurfaceOptions,
) -> io::Result<Mesh> {
    if pa.nx < 2 || pa.ny < 2 || !level.is_finite() {
        return Err(invalid(
            "2D isopotential requires nx,ny>=2 and a finite level",
        ));
    }
    let scale = pa
        .potentials
        .iter()
        .map(|v| (v - level).abs())
        .fold(0.0, f64::max);
    if scale == 0.0 || !scale.is_finite() {
        return Err(invalid(
            "constant or overflowing potential range has no isocontour",
        ));
    }
    let mut section = Section::default();
    for y in 0..pa.ny - 1 {
        for x in 0..pa.nx - 1 {
            let nodes = [
                x + pa.nx * y,
                x + 1 + pa.nx * y,
                x + 1 + pa.nx * (y + 1),
                x + pa.nx * (y + 1),
            ];
            let v = nodes.map(|i| (pa.potentials[i] - level) / scale);
            let case = v
                .iter()
                .enumerate()
                .fold(0, |mask, (i, &v)| mask | (usize::from(v < 0.0) << i));
            // The bilinear center chooses connectivity in saddle cells.
            let center_low = v.iter().sum::<f64>() < 0.0;
            let pairs: &[[usize; 2]] = match case {
                1 => &[[0, 3]],
                2 => &[[1, 0]],
                3 => &[[1, 3]],
                4 => &[[2, 1]],
                5 if center_low => &[[0, 1], [2, 3]],
                5 => &[[0, 3], [2, 1]],
                6 => &[[2, 0]],
                7 => &[[2, 3]],
                8 => &[[3, 2]],
                9 => &[[0, 2]],
                10 if center_low => &[[1, 2], [3, 0]],
                10 => &[[1, 0], [3, 2]],
                11 => &[[1, 2]],
                12 => &[[3, 1]],
                13 => &[[0, 1]],
                14 => &[[3, 0]],
                _ => &[],
            };
            for pair in pairs {
                let edge = pair.map(|edge| {
                    // Canonical node order makes neighboring-cell interpolation
                    // bit-identical, including exact-level endpoints.
                    let a = nodes[edge].min(nodes[(edge + 1) % 4]);
                    let b = nodes[edge].max(nodes[(edge + 1) % 4]);
                    let va = (pa.potentials[a] - level) / scale;
                    let vb = (pa.potentials[b] - level) / scale;
                    let t = va / (va - vb);
                    let p = grid.point(a);
                    let q = grid.point(b);
                    let point = if t == 0.0 {
                        [p[0], p[1]]
                    } else if t == 1.0 {
                        [q[0], q[1]]
                    } else {
                        [p[0] + t * (q[0] - p[0]), p[1] + t * (q[1] - p[1])]
                    };
                    section.vertex(point)
                });
                if edge[0] != edge[1] {
                    section.edges.push(edge);
                }
            }
        }
    }
    let part = section.sweep(
        format!("isopotential-{level}"),
        pa.symmetry == Symmetry::Cylindrical,
        grid,
        options,
    )?;
    Ok(Mesh { parts: vec![part] })
}

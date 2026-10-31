mod gltf_io;
mod ply;
mod threemf;

use crate::{Mesh, Part, Point, Transform, Unit, add, cross, dot, invalid, mul, norm, sub};
use std::collections::HashMap;
use std::fs::File;
use std::io::{self, BufRead, BufReader, BufWriter, Cursor, Read, Seek, Write};
use std::path::Path;

#[derive(Clone, Copy, Debug, Default)]
pub struct ImportOptions {
    /// Override file units. STL/OBJ/PLY default to mm, glTF to m, 3MF to its
    /// model unit. The final mesh always uses mm.
    pub units: Option<Unit>,
    pub transform: Transform,
}

pub fn read_mesh(path: impl AsRef<Path>, options: ImportOptions) -> io::Result<Mesh> {
    let path = path.as_ref();
    options.transform.validate()?;
    let extension = extension(path)?;
    let (mut mesh, source_unit) = match extension.as_str() {
        "stl" => (
            read_stl(path, BufReader::new(File::open(path)?))?,
            Unit::Millimeter,
        ),
        "obj" => (
            read_obj(BufReader::new(File::open(path)?))?,
            Unit::Millimeter,
        ),
        "ply" => (ply::read(path)?, Unit::Millimeter),
        "3mf" => threemf::read(path)?,
        "gltf" | "glb" => (gltf_io::read(path)?, Unit::Meter),
        _ => {
            return Err(invalid(format!(
                "unsupported mesh extension {extension:?}; expected stl, obj, ply, 3mf, gltf or glb"
            )));
        }
    };
    let transform = options.transform.compose(Transform::scale(
        options.units.unwrap_or(source_unit).millimeters(),
    ));
    for part in &mut mesh.parts {
        part.transform(transform)?;
    }
    mesh.validate()?;
    Ok(mesh)
}

/// Import from caller-owned bytes, resolving glTF buffers through the same
/// loader. The path selects the format and retains native part naming.
pub fn read_mesh_with(
    path: impl AsRef<Path>,
    options: ImportOptions,
    mut load: impl FnMut(&Path) -> io::Result<Vec<u8>>,
) -> io::Result<Mesh> {
    let path = path.as_ref();
    options.transform.validate()?;
    let bytes = load(path)?;
    let (mut mesh, source_unit) = match extension(path)?.as_str() {
        "stl" => (read_stl(path, Cursor::new(&bytes))?, Unit::Millimeter),
        "obj" => (read_obj(Cursor::new(&bytes))?, Unit::Millimeter),
        "ply" => (ply::read_from(Cursor::new(&bytes))?, Unit::Millimeter),
        "3mf" => threemf::read_from(Cursor::new(&bytes))?,
        "gltf" | "glb" => (gltf_io::read_with(path, &bytes, &mut load)?, Unit::Meter),
        other => {
            return Err(invalid(format!(
                "unsupported mesh extension {other:?}; expected stl, obj, ply, 3mf, gltf or glb"
            )));
        }
    };
    let transform = options.transform.compose(Transform::scale(
        options.units.unwrap_or(source_unit).millimeters(),
    ));
    for part in &mut mesh.parts {
        part.transform(transform)?;
    }
    mesh.validate()?;
    Ok(mesh)
}

/// Write STL (binary), OBJ, glTF (embedded buffer), GLB or USDA. STL has no part
/// names; reimport reconstructs its edge-connected components.
pub fn write_mesh(path: impl AsRef<Path>, mesh: &Mesh) -> io::Result<()> {
    mesh.validate()?;
    let path = path.as_ref();
    match extension(path)?.as_str() {
        "stl" | "obj" => mesh.write_to(path, BufWriter::new(File::create(path)?)),
        "gltf" | "glb" => gltf_io::write(path, mesh),
        "usda" => crate::write_usda(path, Some(mesh), &[]),
        other => Err(invalid(format!(
            "unsupported mesh export {other:?}; expected stl, obj, gltf, glb or usda"
        ))),
    }
}

impl Mesh {
    /// Write STL, OBJ, embedded-buffer glTF or GLB to caller-owned storage.
    /// Only the path extension is inspected; no filesystem is accessed.
    pub fn write_to(&self, path: impl AsRef<Path>, mut writer: impl Write) -> io::Result<()> {
        self.validate()?;
        let path = path.as_ref();
        match extension(path)?.as_str() {
            "stl" => {
                let mut triangles = Vec::with_capacity(self.triangle_count());
                for part in &self.parts {
                    for triangle in &part.triangles {
                        let vertices = triangle.map(|i| part.vertices[i]);
                        let normal =
                            cross(sub(vertices[1], vertices[0]), sub(vertices[2], vertices[0]));
                        let normal = if norm(normal) > 0.0 {
                            mul(normal, 1.0 / norm(normal))
                        } else {
                            [0.0; 3]
                        };
                        let vertices = vertices.map(|v| stl_io::Vertex::new(v.map(|x| x as f32)));
                        if vertices.iter().flat_map(|v| v.0).any(|v| !v.is_finite()) {
                            return Err(invalid("mesh is outside STL Float32 range"));
                        }
                        triangles.push(stl_io::Triangle {
                            normal: stl_io::Normal::new(normal.map(|v| v as f32)),
                            vertices,
                        });
                    }
                }
                stl_io::write_stl(&mut writer, triangles.iter())?;
            }
            "obj" => {
                let mut offset = 1;
                for part in &self.parts {
                    writeln!(writer, "o {}", part.name.replace(['\n', '\r'], "_"))?;
                    for [x, y, z] in &part.vertices {
                        writeln!(writer, "v {x} {y} {z}")?;
                    }
                    for [a, b, c] in &part.triangles {
                        writeln!(writer, "f {} {} {}", a + offset, b + offset, c + offset)?;
                    }
                    offset += part.vertices.len();
                }
            }
            "gltf" | "glb" => return gltf_io::write_with(path, self, writer),
            other => {
                return Err(invalid(format!(
                    "unsupported byte mesh export {other:?}; expected stl, obj, gltf or glb"
                )));
            }
        }
        writer.flush()
    }
}

fn extension(path: &Path) -> io::Result<String> {
    path.extension()
        .and_then(|s| s.to_str())
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| invalid("file has no extension; import accepts stl, obj, ply, 3mf, gltf or glb; export accepts stl, obj, gltf, glb or usda"))
}

fn read_obj(mut reader: impl BufRead) -> io::Result<Mesh> {
    let (models, _) = tobj::load_obj_buf(
        &mut reader,
        &tobj::LoadOptions {
            triangulate: false,
            single_index: true,
            ignore_points: true,
            ignore_lines: true,
        },
        |_| Ok((Vec::new(), Default::default())),
    )
    .map_err(|e| invalid(e.to_string()))?;
    let parts = models
        .into_iter()
        .filter(|model| !model.mesh.indices.is_empty())
        .map(|model| {
            let vertices: Vec<Point> = model.mesh.positions.as_chunks::<3>().0.to_vec();
            let indices: Vec<usize> = model.mesh.indices.iter().map(|&i| i as usize).collect();
            let mut triangles = Vec::new();
            if model.mesh.face_arities.is_empty() {
                for face in indices.as_chunks::<3>().0 {
                    triangles.extend(triangulate(face, &vertices)?);
                }
            } else {
                let mut offset = 0;
                for arity in model.mesh.face_arities {
                    let end = offset + arity as usize;
                    triangles.extend(triangulate(&indices[offset..end], &vertices)?);
                    offset = end;
                }
            }
            Ok(Part {
                name: model.name,
                vertices,
                triangles,
            })
        })
        .collect::<io::Result<_>>()?;
    Ok(Mesh { parts })
}

/// Project a planar polygon onto its most stable coordinate plane, ear-cut
/// concavities, and preserve the source winding.
fn triangulate(face: &[usize], vertices: &[Point]) -> io::Result<Vec<[usize; 3]>> {
    if face.len() < 3 || face.iter().any(|&i| i >= vertices.len()) {
        return Err(invalid("invalid polygon indices"));
    }
    if face.len() == 3 {
        return Ok(vec![[face[0], face[1], face[2]]]);
    }
    let origin = vertices[face[0]];
    let mut normal = [0.0; 3];
    for i in 0..face.len() {
        normal = add(
            normal,
            cross(
                sub(vertices[face[i]], origin),
                sub(vertices[face[(i + 1) % face.len()]], origin),
            ),
        );
    }
    let axis = (0..3)
        .max_by(|&a, &b| normal[a].abs().total_cmp(&normal[b].abs()))
        .unwrap();
    if normal[axis] == 0.0 {
        return Err(invalid("polygon has zero projected area"));
    }
    let projected: Vec<f64> = face
        .iter()
        .flat_map(|&i| {
            let p = sub(vertices[i], origin);
            [p[(axis + 1) % 3], p[(axis + 2) % 3]]
        })
        .collect();
    let indices = earcutr::earcut(&projected, &[], 2)
        .map_err(|e| invalid(format!("polygon triangulation: {e:?}")))?;
    Ok(indices
        .as_chunks::<3>()
        .0
        .iter()
        .map(|v| {
            let mut triangle = [face[v[0]], face[v[1]], face[v[2]]];
            let [a, b, c] = triangle.map(|i| vertices[i]);
            if dot(cross(sub(b, a), sub(c, a)), normal) < 0.0 {
                triangle.swap(1, 2);
            }
            triangle
        })
        .collect())
}

fn read_stl(path: &Path, mut reader: impl Read + Seek) -> io::Result<Mesh> {
    let mesh = stl_io::read_stl(&mut reader)?;
    let mut part = Part {
        name: path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
        vertices: mesh.vertices.iter().map(|v| v.0.map(f64::from)).collect(),
        triangles: mesh.faces.iter().map(|f| f.vertices).collect(),
    };
    part.validate()?;
    crate::repair::weld(&mut part);
    Ok(Mesh {
        parts: components(part),
    })
}

/// Split STL solids using shared edges (not shared points). Preserve a
/// triangle's winding and never connect parts merely because they touch.
fn components(part: Part) -> Vec<Part> {
    let mut parent: Vec<_> = (0..part.triangles.len()).collect();
    fn root(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    let mut edges = HashMap::new();
    for (i, [a, b, c]) in part.triangles.iter().copied().enumerate() {
        for (a, b) in [(a, b), (b, c), (c, a)] {
            let edge = (a.min(b), a.max(b));
            if let Some(&other) = edges.get(&edge) {
                let a = root(&mut parent, i);
                let b = root(&mut parent, other);
                parent[a] = b;
            } else {
                edges.insert(edge, i);
            }
        }
    }
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut group_ids = HashMap::new();
    for i in 0..parent.len() {
        let r = root(&mut parent, i);
        let next = groups.len();
        let group = *group_ids.entry(r).or_insert_with(|| {
            groups.push(Vec::new());
            next
        });
        groups[group].push(i);
    }
    groups
        .into_iter()
        .enumerate()
        .map(|(i, faces)| {
            let mut result = Part {
                name: format!("{}-{}", part.name, i + 1),
                ..Part::default()
            };
            let mut indices = HashMap::new();
            for face in faces {
                result.triangles.push(part.triangles[face].map(|old| {
                    *indices.entry(old).or_insert_with(|| {
                        result.vertices.push(part.vertices[old]);
                        result.vertices.len() - 1
                    })
                }));
            }
            result
        })
        .collect()
}

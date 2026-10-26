use crate::{Mesh, Part, Transform, invalid};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use gltf::mesh::Mode;
use serde_json::json;
use std::fs;
use std::io::{self, Write};
use std::path::Path;

pub(super) fn read(path: &Path) -> io::Result<Mesh> {
    let gltf = gltf::Gltf::open(path).map_err(|e| invalid(e.to_string()))?;
    let buffers: Vec<Vec<u8>> = gltf
        .buffers()
        .map(|buffer| match buffer.source() {
            gltf::buffer::Source::Bin => gltf
                .blob
                .clone()
                .ok_or_else(|| invalid("missing GLB buffer")),
            gltf::buffer::Source::Uri(uri) => {
                if uri.starts_with("data:") {
                    let (header, data) = uri
                        .split_once(',')
                        .ok_or_else(|| invalid("invalid data URI"))?;
                    if !header.ends_with(";base64") {
                        return Err(invalid("glTF buffer data URI must be base64"));
                    }
                    STANDARD.decode(data).map_err(|e| invalid(e.to_string()))
                } else {
                    if uri.contains("://") {
                        return Err(invalid("network glTF buffers are not supported"));
                    }
                    fs::read(
                        path.parent()
                            .unwrap_or(Path::new("."))
                            .join(decode_uri(uri)?),
                    )
                }
            }
        })
        .collect::<io::Result<_>>()?;
    for buffer in gltf.buffers() {
        if buffers[buffer.index()].len() < buffer.length() {
            return Err(invalid("glTF buffer is shorter than byteLength"));
        }
    }
    let mut result = Mesh::default();
    fn visit(
        node: gltf::Node<'_>,
        parent: Transform,
        buffers: &[Vec<u8>],
        result: &mut Mesh,
        depth: usize,
    ) -> io::Result<()> {
        if depth > 256 {
            return Err(invalid("glTF scene graph is cyclic or too deep"));
        }
        let columns = node.transform().matrix();
        let local = Transform(std::array::from_fn(|i| {
            std::array::from_fn(|j| f64::from(columns[j][i]))
        }));
        let world = parent.compose(local);
        if let Some(mesh) = node.mesh() {
            let mut part = Part {
                name: format!(
                    "{}/{}",
                    node.name().unwrap_or("node"),
                    mesh.name().unwrap_or("mesh")
                ),
                ..Part::default()
            };
            for primitive in mesh.primitives() {
                let reader = primitive.reader(|buffer| Some(&buffers[buffer.index()]));
                let vertices = reader
                    .read_positions()
                    .ok_or_else(|| invalid("glTF primitive has no uncompressed POSITION accessor"))?
                    .map(|p| p.map(f64::from))
                    .collect::<Vec<_>>();
                let indices: Vec<usize> = reader
                    .read_indices()
                    .map(|v| v.into_u32().map(|v| v as usize).collect())
                    .unwrap_or_else(|| (0..vertices.len()).collect());
                let triangles: Vec<[usize; 3]> = match primitive.mode() {
                    Mode::Triangles => {
                        if !indices.len().is_multiple_of(3) {
                            return Err(invalid(
                                "glTF TRIANGLES index count is not divisible by 3",
                            ));
                        }
                        indices.as_chunks::<3>().0.to_vec()
                    }
                    Mode::TriangleStrip => indices
                        .windows(3)
                        .enumerate()
                        .map(|(j, v)| {
                            if j % 2 == 0 {
                                [v[0], v[1], v[2]]
                            } else {
                                [v[1], v[0], v[2]]
                            }
                        })
                        .collect(),
                    Mode::TriangleFan => (1..indices.len().saturating_sub(1))
                        .map(|j| [indices[0], indices[j], indices[j + 1]])
                        .collect(),
                    _ => return Err(invalid("glTF primitive is not a triangle surface")),
                };
                if triangles.iter().flatten().any(|&i| i >= vertices.len()) {
                    return Err(invalid("glTF primitive index is outside its vertex array"));
                }
                let offset = part.vertices.len();
                part.vertices.extend(vertices);
                part.triangles
                    .extend(triangles.into_iter().map(|t| t.map(|i| i + offset)));
            }
            part.validate()?;
            part.transform(world)?;
            result.parts.push(part);
        }
        for child in node.children() {
            visit(child, world, buffers, result, depth + 1)?;
        }
        Ok(())
    }
    if let Some(scene) = gltf.default_scene().or_else(|| gltf.scenes().next()) {
        for node in scene.nodes() {
            visit(node, Transform::IDENTITY, &buffers, &mut result, 0)?;
        }
    } else {
        let children: std::collections::HashSet<_> = gltf
            .nodes()
            .flat_map(|n| n.children().map(|c| c.index()))
            .collect();
        for node in gltf.nodes().filter(|n| !children.contains(&n.index())) {
            visit(node, Transform::IDENTITY, &buffers, &mut result, 0)?;
        }
    }
    Ok(result)
}

fn decode_uri(uri: &str) -> io::Result<String> {
    let mut result = Vec::new();
    let mut bytes = uri.bytes();
    while let Some(byte) = bytes.next() {
        if byte == b'%' {
            let a = bytes.next().and_then(|v| (v as char).to_digit(16));
            let b = bytes.next().and_then(|v| (v as char).to_digit(16));
            result.push(match (a, b) {
                (Some(a), Some(b)) => (a * 16 + b) as u8,
                _ => return Err(invalid("invalid percent escape in glTF URI")),
            });
        } else {
            result.push(byte);
        }
    }
    String::from_utf8(result).map_err(|e| invalid(e.to_string()))
}

pub(super) fn write(path: &Path, mesh: &Mesh) -> io::Result<()> {
    let mut buffer = Vec::new();
    let mut views = Vec::new();
    let mut accessors = Vec::new();
    let mut meshes = Vec::new();
    let mut nodes = Vec::new();
    for (i, part) in mesh.parts.iter().enumerate() {
        let offset = buffer.len();
        let mut min = [f32::INFINITY; 3];
        let mut max = [f32::NEG_INFINITY; 3];
        for point in &part.vertices {
            for axis in 0..3 {
                let value = (point[axis] / 1000.0) as f32;
                if !value.is_finite() {
                    return Err(invalid("mesh is outside glTF Float32 range"));
                }
                buffer.extend_from_slice(&value.to_le_bytes());
                min[axis] = min[axis].min(value);
                max[axis] = max[axis].max(value);
            }
        }
        views.push(
            json!({"buffer":0,"byteOffset":offset,"byteLength":buffer.len()-offset,"target":34962}),
        );
        let position = accessors.len();
        accessors.push(json!({"bufferView":views.len()-1,"componentType":5126,"count":part.vertices.len(),"type":"VEC3","min":min,"max":max}));
        let offset = buffer.len();
        for &index in part.triangles.iter().flatten() {
            let index =
                u32::try_from(index).map_err(|_| invalid("glTF vertex index exceeds UInt32"))?;
            buffer.extend_from_slice(&index.to_le_bytes());
        }
        views.push(
            json!({"buffer":0,"byteOffset":offset,"byteLength":buffer.len()-offset,"target":34963}),
        );
        let indices = accessors.len();
        accessors.push(json!({"bufferView":views.len()-1,"componentType":5125,"count":part.triangles.len()*3,"type":"SCALAR"}));
        meshes.push(json!({"name":part.name,"primitives":[{"attributes":{"POSITION":position},"indices":indices,"mode":4}]}));
        nodes.push(json!({"name":part.name,"mesh":i}));
    }
    let glb = path
        .extension()
        .is_some_and(|s| s.eq_ignore_ascii_case("glb"));
    let mut buffer_json = json!({"byteLength":buffer.len()});
    if !glb {
        buffer_json["uri"] = json!(format!(
            "data:application/octet-stream;base64,{}",
            STANDARD.encode(&buffer)
        ));
    }
    let document = json!({
        "asset":{"version":"2.0","generator":"simrep-mesh"},
        "scene":0,"scenes":[{"nodes":(0..nodes.len()).collect::<Vec<_>>()}],
        "nodes":nodes,"meshes":meshes,"buffers":[buffer_json],"bufferViews":views,"accessors":accessors
    });
    let mut json_bytes = serde_json::to_vec(&document)?;
    let mut file = io::BufWriter::new(fs::File::create(path)?);
    if glb {
        while !json_bytes.len().is_multiple_of(4) {
            json_bytes.push(b' ');
        }
        while !buffer.len().is_multiple_of(4) {
            buffer.push(0);
        }
        let total = u32::try_from(28 + json_bytes.len() + buffer.len())
            .map_err(|_| invalid("GLB exceeds 4 GiB"))?;
        for word in [0x46546c67, 2, total, json_bytes.len() as u32, 0x4e4f534a] {
            file.write_all(&word.to_le_bytes())?;
        }
        file.write_all(&json_bytes)?;
        file.write_all(&(buffer.len() as u32).to_le_bytes())?;
        file.write_all(&0x004e4942_u32.to_le_bytes())?;
        file.write_all(&buffer)?;
    } else {
        file.write_all(&json_bytes)?;
    }
    file.flush()
}

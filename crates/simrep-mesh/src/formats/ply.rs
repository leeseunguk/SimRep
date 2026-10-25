//! PLY 1.0 scalar/list parsing, including both binary byte orders.
//! A small local reader avoids depending on the unmaintained ply-rs parser.
use crate::{Mesh, Part, invalid};
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Read};
use std::path::Path;

#[derive(Clone, Copy)]
enum Scalar {
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    F32,
    F64,
}

impl Scalar {
    fn parse(s: &str) -> io::Result<Self> {
        match s {
            "char" | "int8" => Ok(Self::I8),
            "uchar" | "uint8" => Ok(Self::U8),
            "short" | "int16" => Ok(Self::I16),
            "ushort" | "uint16" => Ok(Self::U16),
            "int" | "int32" => Ok(Self::I32),
            "uint" | "uint32" => Ok(Self::U32),
            "float" | "float32" => Ok(Self::F32),
            "double" | "float64" => Ok(Self::F64),
            _ => Err(invalid(format!("unsupported PLY scalar {s}"))),
        }
    }
}

struct Property {
    name: String,
    scalar: Scalar,
    count: Option<Scalar>,
}

struct Element {
    name: String,
    count: usize,
    properties: Vec<Property>,
}

pub(super) fn read(path: &Path) -> io::Result<Mesh> {
    let mut reader = BufReader::new(File::open(path)?);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    if line.trim() != "ply" {
        return Err(invalid("not a PLY file"));
    }
    let mut elements: Vec<Element> = Vec::new();
    let mut format = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            return Err(invalid("truncated PLY header"));
        }
        let fields: Vec<_> = line.split_whitespace().collect();
        match fields.as_slice() {
            ["end_header"] => break,
            ["format", value, "1.0"] => format = value.to_string(),
            ["element", name, count] => elements.push(Element {
                name: name.to_string(),
                count: count
                    .parse()
                    .map_err(|_| invalid("invalid PLY element count"))?,
                properties: Vec::new(),
            }),
            ["property", scalar, name] => {
                elements
                    .last_mut()
                    .ok_or_else(|| invalid("PLY property before element"))?
                    .properties
                    .push(Property {
                        name: name.to_string(),
                        scalar: Scalar::parse(scalar)?,
                        count: None,
                    });
            }
            ["property", "list", count, scalar, name] => {
                elements
                    .last_mut()
                    .ok_or_else(|| invalid("PLY property before element"))?
                    .properties
                    .push(Property {
                        name: name.to_string(),
                        scalar: Scalar::parse(scalar)?,
                        count: Some(Scalar::parse(count)?),
                    });
            }
            ["comment", ..] | ["obj_info", ..] | [] => {}
            _ => return Err(invalid(format!("invalid PLY header line {line:?}"))),
        }
    }
    let mut body = Vec::new();
    reader.read_to_end(&mut body)?;
    let mut ascii = if format == "ascii" {
        Some(
            std::str::from_utf8(&body)
                .map_err(|e| invalid(e.to_string()))?
                .split_whitespace(),
        )
    } else {
        None
    };
    let little = match format.as_str() {
        "ascii" | "binary_little_endian" => true,
        "binary_big_endian" => false,
        _ => return Err(invalid("unsupported PLY format")),
    };
    let mut offset = 0;
    let mut number = |scalar: Scalar| -> io::Result<f64> {
        if let Some(tokens) = &mut ascii {
            return tokens
                .next()
                .ok_or_else(|| invalid("truncated ASCII PLY"))?
                .parse()
                .map_err(|_| invalid("invalid ASCII PLY scalar"));
        }
        macro_rules! value {
            ($ty:ty, $n:expr) => {{
                let bytes: [u8; $n] = body
                    .get(offset..offset + $n)
                    .ok_or_else(|| invalid("truncated binary PLY"))?
                    .try_into()
                    .unwrap();
                offset += $n;
                if little {
                    <$ty>::from_le_bytes(bytes) as f64
                } else {
                    <$ty>::from_be_bytes(bytes) as f64
                }
            }};
        }
        Ok(match scalar {
            Scalar::I8 => value!(i8, 1),
            Scalar::U8 => value!(u8, 1),
            Scalar::I16 => value!(i16, 2),
            Scalar::U16 => value!(u16, 2),
            Scalar::I32 => value!(i32, 4),
            Scalar::U32 => value!(u32, 4),
            Scalar::F32 => value!(f32, 4),
            Scalar::F64 => value!(f64, 8),
        })
    };
    let mut vertices = Vec::new();
    let mut faces: Vec<(i64, Vec<usize>)> = Vec::new();
    for element in elements {
        if element.count > body.len() {
            return Err(invalid("PLY element count exceeds data size"));
        }
        for _ in 0..element.count {
            let mut point = [0.0; 3];
            let mut coordinates = [false; 3];
            let mut face = None;
            let mut group = 0_i64;
            for property in &element.properties {
                if let Some(count) = property.count {
                    let n = integer(number(count)?)?;
                    if n > body.len() {
                        return Err(invalid("PLY list count exceeds data size"));
                    }
                    let indices =
                        property.name == "vertex_indices" || property.name == "vertex_index";
                    let mut list = Vec::new();
                    for _ in 0..n {
                        let value = number(property.scalar)?;
                        if indices {
                            list.push(integer(value)?);
                        }
                    }
                    if element.name == "face" && indices {
                        face = Some(list);
                    }
                } else {
                    let value = number(property.scalar)?;
                    if element.name == "vertex"
                        && let Some(axis) = ["x", "y", "z"].iter().position(|&n| n == property.name)
                    {
                        point[axis] = value;
                        coordinates[axis] = true;
                    }
                    if element.name == "face"
                        && matches!(property.name.as_str(), "object_id" | "part_id")
                    {
                        if !value.is_finite()
                            || value.fract() != 0.0
                            || value.abs() > (1_i64 << 53) as f64
                        {
                            return Err(invalid("invalid PLY object id"));
                        }
                        group = value as i64;
                    }
                }
            }
            if element.name == "vertex" {
                if !coordinates.iter().all(|v| *v) {
                    return Err(invalid("PLY vertex requires x, y and z"));
                }
                vertices.push(point);
            } else if element.name == "face" {
                faces.push((
                    group,
                    face.ok_or_else(|| invalid("PLY face has no vertex_indices"))?,
                ));
            }
        }
    }
    let mut groups = BTreeMap::new();
    for (group, face) in faces {
        if face.len() < 3 || face.iter().any(|&i| i >= vertices.len()) {
            return Err(invalid("invalid PLY polygon indices"));
        }
        let triangles: &mut Vec<[usize; 3]> = groups.entry(group).or_default();
        triangles.extend(super::triangulate(&face, &vertices)?);
    }
    Ok(Mesh {
        parts: groups
            .into_iter()
            .map(|(id, triangles)| Part {
                name: format!("ply-{id}"),
                vertices: vertices.clone(),
                triangles,
            })
            .collect(),
    })
}

fn integer(value: f64) -> io::Result<usize> {
    if !value.is_finite() || value < 0.0 || value.fract() != 0.0 || value >= usize::MAX as f64 {
        return Err(invalid("PLY index/count is not a nonnegative integer"));
    }
    Ok(value as usize)
}

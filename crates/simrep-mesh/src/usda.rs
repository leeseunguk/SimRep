//! Dependency-free ASCII USD from the public UsdGeom Mesh/BasisCurves schemas.
//! Coordinates stay in millimeters; the stage declares metersPerUnit = 0.001.

use crate::{Mesh, Point, Trajectory, invalid, vtk::validate_trajectories};
use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::Path;

/// Write a static USD scene containing triangle meshes, trajectory curves, or
/// both. Geometry uses USD's Float32 points; times and velocities remain f64.
/// Input order determines prim/curve order. No timestamps or external assets
/// are emitted. Mesh names are retained as displayName metadata, with indexed
/// prim names so duplicate names and arbitrary source text cannot collide.
pub fn write_usda(
    path: impl AsRef<Path>,
    mesh: Option<&Mesh>,
    trajectories: &[Trajectory],
) -> io::Result<()> {
    if let Some(mesh) = mesh {
        mesh.validate()?;
        for part in &mesh.parts {
            if i32::try_from(part.vertices.len()).is_err() {
                return Err(invalid("USD mesh exceeds Int32 vertex indices"));
            }
        }
    }
    validate_trajectories(trajectories)?;
    if mesh.is_none() && trajectories.is_empty() {
        return Err(invalid("USD scene has no meshes or trajectories"));
    }
    for trajectory in trajectories {
        if i32::try_from(trajectory.points.len()).is_err() {
            return Err(invalid("USD curve exceeds Int32 vertex count"));
        }
    }
    if mesh
        .into_iter()
        .flat_map(|m| &m.parts)
        .flat_map(|p| &p.vertices)
        .chain(
            trajectories
                .iter()
                .flat_map(|t| &t.points)
                .map(|p| &p.position_mm),
        )
        .flatten()
        .any(|&v| !(v as f32).is_finite())
    {
        return Err(invalid("geometry is outside USD Float32 range"));
    }

    let mut writer = BufWriter::new(File::create(path)?);
    writeln!(
        writer,
        "#usda 1.0\n(\n    defaultPrim = \"SimRep\"\n    metersPerUnit = 0.001\n    upAxis = \"Z\"\n)\ndef Xform \"SimRep\"\n{{"
    )?;
    if let Some(mesh) = mesh {
        for (i, part) in mesh.parts.iter().enumerate() {
            // USD uses C-style octal escapes, not JSON's \uNNNN escapes.
            let mut name = String::new();
            for c in part.name.chars() {
                match c {
                    '"' => name.push_str("\\\""),
                    '\\' => name.push_str("\\\\"),
                    '\n' => name.push_str("\\n"),
                    '\r' => name.push_str("\\r"),
                    '\t' => name.push_str("\\t"),
                    '\x08' => name.push_str("\\b"),
                    '\x0c' => name.push_str("\\f"),
                    c if c.is_ascii_control() => name.push_str(&format!("\\{:03o}", c as u8)),
                    c => name.push(c),
                }
            }
            writeln!(
                writer,
                "    def Mesh \"part_{i}\" (\n        displayName = \"{name}\"\n    )\n    {{\n        uniform token subdivisionScheme = \"none\"\n        uniform token orientation = \"rightHanded\""
            )?;
            write_points(&mut writer, part.vertices.iter().copied(), 0.0)?;
            write_array(
                &mut writer,
                "int[] faceVertexCounts",
                part.triangles.iter().map(|_| 3),
                "",
            )?;
            write_array(
                &mut writer,
                "int[] faceVertexIndices",
                part.triangles.iter().flatten(),
                "",
            )?;
            writeln!(writer, "    }}")?;
        }
    }
    if !trajectories.is_empty() {
        writeln!(
            writer,
            "    def BasisCurves \"Trajectories\"\n    {{\n        uniform token type = \"linear\"\n        uniform token wrap = \"nonperiodic\"\n        float[] widths = [0.1] (interpolation = \"constant\")\n        color3f[] primvars:displayColor = [(1, 0.25, 0)] (interpolation = \"constant\")"
        )?;
        write_points(
            &mut writer,
            trajectories
                .iter()
                .flat_map(|t| &t.points)
                .map(|p| p.position_mm),
            0.05,
        )?;
        write_array(
            &mut writer,
            "int[] curveVertexCounts",
            trajectories.iter().map(|t| t.points.len()),
            "",
        )?;
        write_array(
            &mut writer,
            "uint64[] primvars:trajectory_id",
            trajectories.iter().map(|t| t.id),
            " (interpolation = \"uniform\")",
        )?;
        write_array(
            &mut writer,
            "double[] primvars:time_us",
            trajectories
                .iter()
                .flat_map(|t| &t.points)
                .map(|p| p.time_us),
            " (interpolation = \"vertex\")",
        )?;
        // Custom scientific units avoid USD's velocities attribute, whose
        // meaning is motion per second, not per microsecond along a static path.
        write!(writer, "        vector3d[] primvars:velocity_mm_per_us = [")?;
        for (i, point) in trajectories.iter().flat_map(|t| &t.points).enumerate() {
            let [x, y, z] = point.velocity_mm_per_us;
            if i != 0 {
                write!(writer, ", ")?;
            }
            write!(writer, "({x}, {y}, {z})")?;
        }
        writeln!(writer, "] (interpolation = \"vertex\")\n    }}")?;
    }
    writeln!(writer, "}}")?;
    writer.flush()
}

fn write_array(
    writer: &mut impl Write,
    declaration: &str,
    values: impl IntoIterator<Item = impl std::fmt::Display>,
    metadata: &str,
) -> io::Result<()> {
    write!(writer, "        {declaration} = [")?;
    for (i, value) in values.into_iter().enumerate() {
        if i != 0 {
            write!(writer, ", ")?;
        }
        write!(writer, "{value}")?;
    }
    writeln!(writer, "]{metadata}")
}

fn write_points(
    writer: &mut impl Write,
    points: impl Iterator<Item = Point>,
    radius_mm: f32,
) -> io::Result<()> {
    let mut lo = [f32::INFINITY; 3];
    let mut hi = [f32::NEG_INFINITY; 3];
    write!(writer, "        point3f[] points = [")?;
    for (i, point) in points.enumerate() {
        let [x, y, z] = point.map(|v| v as f32);
        for (axis, value) in [x, y, z].into_iter().enumerate() {
            lo[axis] = lo[axis].min(value - radius_mm);
            hi[axis] = hi[axis].max(value + radius_mm);
        }
        if i != 0 {
            write!(writer, ", ")?;
        }
        write!(writer, "({x}, {y}, {z})")?;
    }
    let [lx, ly, lz] = lo;
    let [hx, hy, hz] = hi;
    writeln!(
        writer,
        "]\n        float3[] extent = [({lx}, {ly}, {lz}), ({hx}, {hy}, {hz})]"
    )
}

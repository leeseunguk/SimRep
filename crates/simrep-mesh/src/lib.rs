//! Triangle meshes in millimeters, solid sampling, and scientific file export.
//!
//! Geometry is static: scene transforms are baked into vertices on import.
//! No native CAD library, GPU context, or SIMION installation is required.

mod formats;
mod grid;
mod repair;
mod spatial;
mod surface;
mod sweep;
mod usda;
mod vtk;

pub use formats::{ImportOptions, read_mesh, write_mesh};
pub use grid::{
    GridSpec, SignedDistanceGrid, Voxelization, sample_sdf, voxelize, voxelize_with_defects,
};
pub use repair::{MeshReport, RepairReport, inspect, repair, weld_and_orient};
pub use spatial::{Solid, SurfaceGap};
pub use surface::{
    SurfaceOptions, electrode_surface, electrode_surface_with_options, isosurface, pa_isosurface,
};
pub use usda::write_usda;
pub use vtk::{
    Trajectory, TrajectoryPoint, write_revolved_vts, write_scalar_vti, write_trajectories_csv,
    write_vti, write_vtp,
};

use std::io;
use std::str::FromStr;

pub type Point = [f64; 3];

/// A named object/primitive. Triangle indices are local to this part.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Part {
    pub name: String,
    pub vertices: Vec<Point>,
    pub triangles: Vec<[usize; 3]>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Mesh {
    pub parts: Vec<Part>,
}

impl Mesh {
    pub fn triangle_count(&self) -> usize {
        self.parts.iter().map(|p| p.triangles.len()).sum()
    }

    pub fn validate(&self) -> io::Result<()> {
        if self.parts.is_empty() {
            return Err(invalid("mesh has no parts"));
        }
        for part in &self.parts {
            part.validate()?;
        }
        Ok(())
    }
}

impl Part {
    /// Portable cuts-v3 payload: u64 LE vertex/triangle counts, xyz f64 LE
    /// vertices, then u64 LE vertex indices. No quantization or host word sizes.
    pub fn source_bytes(&self) -> io::Result<Vec<u8>> {
        self.validate()?;
        let mut bytes = Vec::with_capacity(16 + 24 * (self.vertices.len() + self.triangles.len()));
        bytes.extend_from_slice(&(self.vertices.len() as u64).to_le_bytes());
        bytes.extend_from_slice(&(self.triangles.len() as u64).to_le_bytes());
        for v in self.vertices.iter().flatten() {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        for i in self.triangles.iter().flatten() {
            bytes.extend_from_slice(&(*i as u64).to_le_bytes());
        }
        Ok(bytes)
    }

    pub fn from_source_bytes(bytes: &[u8]) -> io::Result<Self> {
        if bytes.len() < 16 || !(bytes.len() - 16).is_multiple_of(24) {
            return Err(invalid("invalid cuts mesh payload length"));
        }
        let count = |offset| u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap());
        let vertices = usize::try_from(count(0)).map_err(|_| invalid("mesh count overflow"))?;
        let triangles = usize::try_from(count(8)).map_err(|_| invalid("mesh count overflow"))?;
        if vertices.checked_add(triangles) != Some((bytes.len() - 16) / 24) {
            return Err(invalid("cuts mesh counts do not match payload"));
        }
        let mut part = Self::default();
        for record in bytes[16..16 + 24 * vertices].as_chunks::<24>().0 {
            part.vertices.push(std::array::from_fn(|a| {
                f64::from_le_bytes(record[8 * a..8 * a + 8].try_into().unwrap())
            }));
        }
        for record in bytes[16 + 24 * vertices..].as_chunks::<24>().0 {
            let mut face = [0; 3];
            for a in 0..3 {
                face[a] = usize::try_from(u64::from_le_bytes(
                    record[8 * a..8 * a + 8].try_into().unwrap(),
                ))
                .map_err(|_| invalid("mesh index overflow"))?;
            }
            part.triangles.push(face);
        }
        part.validate()?;
        Ok(part)
    }

    pub fn validate(&self) -> io::Result<()> {
        if self.triangles.is_empty() {
            return Err(invalid(format!("part {:?} has no triangles", self.name)));
        }
        if self.vertices.iter().flatten().any(|x| !x.is_finite()) {
            return Err(invalid("mesh contains non-finite coordinates"));
        }
        if self
            .triangles
            .iter()
            .flatten()
            .any(|&i| i >= self.vertices.len())
        {
            return Err(invalid("triangle index is outside vertex array"));
        }
        Ok(())
    }

    pub fn transform(&mut self, transform: Transform) -> io::Result<()> {
        transform.validate()?;
        for vertex in &mut self.vertices {
            *vertex = transform.apply(*vertex);
        }
        if transform.determinant() < 0.0 {
            for triangle in &mut self.triangles {
                triangle.swap(1, 2);
            }
        }
        self.validate()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Unit {
    Micron,
    #[default]
    Millimeter,
    Centimeter,
    Meter,
    Inch,
    Foot,
}

impl Unit {
    pub fn millimeters(self) -> f64 {
        match self {
            Self::Micron => 0.001,
            Self::Millimeter => 1.0,
            Self::Centimeter => 10.0,
            Self::Meter => 1000.0,
            Self::Inch => 25.4,
            Self::Foot => 304.8,
        }
    }
}

impl FromStr for Unit {
    type Err = io::Error;

    fn from_str(s: &str) -> io::Result<Self> {
        match s {
            "um" | "micron" => Ok(Self::Micron),
            "mm" | "millimeter" => Ok(Self::Millimeter),
            "cm" | "centimeter" => Ok(Self::Centimeter),
            "m" | "meter" => Ok(Self::Meter),
            "in" | "inch" => Ok(Self::Inch),
            "ft" | "foot" => Ok(Self::Foot),
            _ => Err(invalid(format!(
                "unknown length unit {s:?}; expected um/micron, mm/millimeter, cm/centimeter, m/meter, in/inch or ft/foot"
            ))),
        }
    }
}

/// Row-major affine matrix applied to column vectors. Translation is in mm
/// after source units have been converted to mm.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transform(pub [[f64; 4]; 4]);

impl Default for Transform {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Transform {
    pub const IDENTITY: Self = Self([
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]);

    pub fn scale(scale: f64) -> Self {
        let mut result = Self::IDENTITY;
        for i in 0..3 {
            result.0[i][i] = scale;
        }
        result
    }

    /// `self * child`, so the child transform is applied first.
    pub fn compose(self, child: Self) -> Self {
        let mut result = [[0.0; 4]; 4];
        for (i, row) in result.iter_mut().enumerate() {
            for (j, value) in row.iter_mut().enumerate() {
                *value = (0..4).map(|k| self.0[i][k] * child.0[k][j]).sum();
            }
        }
        Self(result)
    }

    pub fn apply(self, point: Point) -> Point {
        std::array::from_fn(|i| self.0[i][3] + (0..3).map(|j| self.0[i][j] * point[j]).sum::<f64>())
    }

    fn determinant(self) -> f64 {
        dot(
            [self.0[0][0], self.0[1][0], self.0[2][0]],
            cross(
                [self.0[0][1], self.0[1][1], self.0[2][1]],
                [self.0[0][2], self.0[1][2], self.0[2][2]],
            ),
        )
    }

    fn validate(self) -> io::Result<()> {
        if self.0.iter().flatten().any(|v| !v.is_finite())
            || self.0[3] != [0.0, 0.0, 0.0, 1.0]
            || self.determinant() == 0.0
        {
            return Err(invalid("transform must be finite, affine, and nonsingular"));
        }
        Ok(())
    }
}

pub(crate) fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

pub(crate) fn sub(a: Point, b: Point) -> Point {
    std::array::from_fn(|i| a[i] - b[i])
}

pub(crate) fn add(a: Point, b: Point) -> Point {
    std::array::from_fn(|i| a[i] + b[i])
}

pub(crate) fn mul(a: Point, scale: f64) -> Point {
    a.map(|v| v * scale)
}

pub(crate) fn dot(a: Point, b: Point) -> f64 {
    a.iter().zip(b).map(|(a, b)| a * b).sum()
}

pub(crate) fn cross(a: Point, b: Point) -> Point {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

pub(crate) fn norm(a: Point) -> f64 {
    dot(a, a).sqrt()
}

use crate::{GridSpec, Point, SurfaceOptions, invalid, surface::expanded_pa};
use simrep_core::{PotentialArray, Symmetry};
use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::Path;

fn array(
    writer: &mut impl Write,
    name: &str,
    scalar: &str,
    components: usize,
    offset: usize,
) -> io::Result<()> {
    let name = name
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('"', "&quot;");
    writeln!(
        writer,
        "<DataArray type=\"{scalar}\" Name=\"{name}\" NumberOfComponents=\"{components}\" format=\"appended\" offset=\"{offset}\"/>"
    )
}

fn image_header(writer: &mut impl Write, grid: GridSpec, scalar: &str) -> io::Result<()> {
    let [nx, ny, nz] = grid.dims.map(|n| n - 1);
    let [ox, oy, oz] = grid.origin_mm;
    let [dx, dy, dz] = grid.spacing_mm;
    writeln!(
        writer,
        "<?xml version=\"1.0\"?>\n<VTKFile type=\"ImageData\" version=\"1.0\" byte_order=\"LittleEndian\" header_type=\"UInt64\">\n<ImageData WholeExtent=\"0 {nx} 0 {ny} 0 {nz}\" Origin=\"{ox} {oy} {oz}\" Spacing=\"{dx} {dy} {dz}\">\n<Piece Extent=\"0 {nx} 0 {ny} 0 {nz}\"><PointData Scalars=\"{scalar}\">"
    )
}

fn length(writer: &mut impl Write, bytes: usize) -> io::Result<()> {
    writer.write_all(&(bytes as u64).to_le_bytes())
}

fn image_data_start(writer: &mut impl Write) -> io::Result<()> {
    write!(
        writer,
        "</PointData><CellData/></Piece></ImageData>\n<AppendedData encoding=\"raw\">_"
    )
}

fn finish(writer: &mut impl Write) -> io::Result<()> {
    write!(writer, "\n</AppendedData></VTKFile>")?;
    writer.flush()
}

/// VTK ImageData, x-fastest Float64 potential, UInt8 electrode mask, and
/// optionally the node-centered negative potential gradient. Length headers
/// are UInt64 and the appended binary payload is little endian, uncompressed.
/// Electric PA potentials are V; the field is V/mm. Mirrors are expanded.
/// Cylindrical arrays are the 2D x-r coordinate chart, with field=(Ex,Er,0),
/// not a Cartesian volume. Radius zero maps to origin_mm[1].
pub fn write_vti(
    path: impl AsRef<Path>,
    pa: &PotentialArray,
    origin_mm: Point,
    field: bool,
) -> io::Result<()> {
    let (pa, grid) = expanded_pa(pa, origin_mm)?;
    let count = pa.potentials.len();
    let mut writer = BufWriter::new(File::create(path)?);
    image_header(&mut writer, grid, "potential")?;
    array(&mut writer, "potential", "Float64", 1, 0)?;
    array(&mut writer, "electrode", "UInt8", 1, 8 + count * 8)?;
    if field {
        array(&mut writer, "field", "Float64", 3, 16 + count * 9)?;
    }
    image_data_start(&mut writer)?;
    length(&mut writer, count * 8)?;
    for value in &pa.potentials {
        writer.write_all(&value.to_le_bytes())?;
    }
    length(&mut writer, count)?;
    for &value in &pa.electrodes {
        writer.write_all(&[u8::from(value)])?;
    }
    if field {
        length(&mut writer, count * 24)?;
        for i in 0..count {
            for value in node_field(&pa, grid, i) {
                writer.write_all(&value.to_le_bytes())?;
            }
        }
    }
    finish(&mut writer)
}

fn node_field(pa: &PotentialArray, grid: GridSpec, i: usize) -> Point {
    let strides = [1, pa.nx, pa.nx * pa.ny];
    let coords = [i % pa.nx, i / pa.nx % pa.ny, i / (pa.nx * pa.ny)];
    std::array::from_fn(|axis| {
        if grid.dims[axis] == 1
            || (pa.symmetry == Symmetry::Cylindrical && axis == 1 && coords[1] == 0)
        {
            return 0.0;
        }
        let lo = if coords[axis] == 0 {
            i
        } else {
            i - strides[axis]
        };
        let hi = if coords[axis] + 1 == grid.dims[axis] {
            i
        } else {
            i + strides[axis]
        };
        let steps = (hi - lo) / strides[axis];
        -(pa.potentials[hi] - pa.potentials[lo]) / (steps as f64 * grid.spacing_mm[axis])
    })
}

/// Revolve a cylindrical x-r PA into a VTK StructuredGrid, x fastest,
/// then radius, then angle. The full-sweep seam is repeated exactly, and
/// the radial-axis nodes coincide. The field is Cartesian (Ex,Ey,Ez).
pub fn write_revolved_vts(
    path: impl AsRef<Path>,
    pa: &PotentialArray,
    origin_mm: Point,
    field: bool,
    angular_segments: usize,
    sweep_degrees: f64,
) -> io::Result<()> {
    SurfaceOptions {
        angular_segments,
        sweep_degrees,
        ..SurfaceOptions::default()
    }
    .validate()?;
    if pa.symmetry != Symmetry::Cylindrical {
        return Err(invalid("revolved VTS requires a cylindrical PA"));
    }
    let (pa, grid) = expanded_pa(pa, origin_mm)?;
    let plane = pa.potentials.len();
    let count = angular_segments
        .checked_add(1)
        .and_then(|n| plane.checked_mul(n))
        .filter(|n| n.checked_mul(57).is_some())
        .ok_or_else(|| invalid("revolved grid size overflow"))?;
    let (nx, ny) = (pa.nx - 1, pa.ny - 1);
    let mut writer = BufWriter::new(File::create(path)?);
    let vectors = if field { " Vectors=\"field\"" } else { "" };
    writeln!(
        writer,
        "<?xml version=\"1.0\"?>\n<VTKFile type=\"StructuredGrid\" version=\"1.0\" byte_order=\"LittleEndian\" header_type=\"UInt64\">\n<StructuredGrid WholeExtent=\"0 {nx} 0 {ny} 0 {angular_segments}\">\n<Piece Extent=\"0 {nx} 0 {ny} 0 {angular_segments}\"><PointData Scalars=\"potential\"{vectors}>"
    )?;
    array(&mut writer, "potential", "Float64", 1, 0)?;
    array(&mut writer, "electrode", "UInt8", 1, 8 + count * 8)?;
    if field {
        array(&mut writer, "field", "Float64", 3, 16 + count * 9)?;
    }
    write!(writer, "</PointData><CellData/><Points>")?;
    let point_offset = if field {
        24 + count * 33
    } else {
        16 + count * 9
    };
    array(&mut writer, "position_mm", "Float64", 3, point_offset)?;
    write!(
        writer,
        "</Points></Piece></StructuredGrid>\n<AppendedData encoding=\"raw\">_"
    )?;
    length(&mut writer, count * 8)?;
    for _ in 0..=angular_segments {
        for value in &pa.potentials {
            writer.write_all(&value.to_le_bytes())?;
        }
    }
    length(&mut writer, count)?;
    for _ in 0..=angular_segments {
        for &value in &pa.electrodes {
            writer.write_all(&[u8::from(value)])?;
        }
    }
    let angle = |step: usize| {
        if step == angular_segments && sweep_degrees == 360.0 {
            (0.0, 1.0)
        } else {
            (sweep_degrees.to_radians() * step as f64 / angular_segments as f64).sin_cos()
        }
    };
    if field {
        length(&mut writer, count * 24)?;
        let fields: Vec<_> = (0..plane).map(|i| node_field(&pa, grid, i)).collect();
        for step in 0..=angular_segments {
            let (sin, cos) = angle(step);
            for &[ex, er, _] in &fields {
                for value in [ex, er * cos, er * sin] {
                    writer.write_all(&value.to_le_bytes())?;
                }
            }
        }
    }
    length(&mut writer, count * 24)?;
    for step in 0..=angular_segments {
        let (sin, cos) = angle(step);
        for i in 0..plane {
            let x = grid.origin_mm[0] + (i % pa.nx) as f64 * grid.spacing_mm[0];
            let radius = (i / pa.nx) as f64 * grid.spacing_mm[1];
            for value in [x, origin_mm[1] + radius * cos, origin_mm[2] + radius * sin] {
                writer.write_all(&value.to_le_bytes())?;
            }
        }
    }
    finish(&mut writer)
}

pub fn write_scalar_vti(
    path: impl AsRef<Path>,
    grid: GridSpec,
    name: &str,
    values: &[f64],
) -> io::Result<()> {
    if grid.validate()? != values.len() || values.iter().any(|v| !v.is_finite()) {
        return Err(invalid("scalar VTI buffer length or values are invalid"));
    }
    // Fixed active-scalar name; the user-supplied name is an escaped DataArray
    // name only, not inserted into other XML attributes.
    let mut writer = BufWriter::new(File::create(path)?);
    image_header(&mut writer, grid, "")?;
    array(&mut writer, name, "Float64", 1, 0)?;
    image_data_start(&mut writer)?;
    length(&mut writer, values.len() * 8)?;
    for value in values {
        writer.write_all(&value.to_le_bytes())?;
    }
    finish(&mut writer)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrajectoryPoint {
    pub time_us: f64,
    pub position_mm: Point,
    pub velocity_mm_per_us: Point,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Trajectory {
    pub id: u64,
    pub points: Vec<TrajectoryPoint>,
}

fn validate_trajectories(trajectories: &[Trajectory]) -> io::Result<usize> {
    let mut ids = std::collections::HashSet::new();
    let mut count = 0usize;
    for trajectory in trajectories {
        if !ids.insert(trajectory.id) {
            return Err(invalid("trajectory ids must be unique"));
        }
        if trajectory.points.len() < 2 {
            return Err(invalid("a trajectory polyline needs at least two points"));
        }
        count = count
            .checked_add(trajectory.points.len())
            .ok_or_else(|| invalid("trajectory point count overflow"))?;
        for point in &trajectory.points {
            if !point.time_us.is_finite()
                || point
                    .position_mm
                    .iter()
                    .chain(&point.velocity_mm_per_us)
                    .any(|v| !v.is_finite())
            {
                return Err(invalid("trajectory samples must be finite"));
            }
        }
        if trajectory
            .points
            .windows(2)
            .any(|p| p[1].time_us < p[0].time_us)
        {
            return Err(invalid("trajectory times must be nondecreasing"));
        }
    }
    Ok(count)
}

/// One VTK PolyLine cell per trajectory, including id, time and velocity.
pub fn write_vtp(path: impl AsRef<Path>, trajectories: &[Trajectory]) -> io::Result<()> {
    let count = validate_trajectories(trajectories)?;
    let mut writer = BufWriter::new(File::create(path)?);
    writeln!(
        writer,
        "<?xml version=\"1.0\"?>\n<VTKFile type=\"PolyData\" version=\"1.0\" byte_order=\"LittleEndian\" header_type=\"UInt64\">\n<PolyData><Piece NumberOfPoints=\"{count}\" NumberOfVerts=\"0\" NumberOfLines=\"{}\" NumberOfStrips=\"0\" NumberOfPolys=\"0\">",
        trajectories.len()
    )?;
    write!(
        writer,
        "<PointData Scalars=\"time_us\" Vectors=\"velocity_mm_per_us\">"
    )?;
    array(&mut writer, "time_us", "Float64", 1, 0)?;
    array(
        &mut writer,
        "velocity_mm_per_us",
        "Float64",
        3,
        8 + count * 8,
    )?;
    write!(writer, "</PointData><CellData Scalars=\"trajectory_id\">")?;
    array(&mut writer, "trajectory_id", "UInt64", 1, 16 + count * 32)?;
    write!(writer, "</CellData><Points>")?;
    array(
        &mut writer,
        "position_mm",
        "Float64",
        3,
        24 + count * 32 + trajectories.len() * 8,
    )?;
    write!(writer, "</Points><Lines>")?;
    let connectivity = 32 + count * 56 + trajectories.len() * 8;
    array(&mut writer, "connectivity", "Int64", 1, connectivity)?;
    array(
        &mut writer,
        "offsets",
        "Int64",
        1,
        connectivity + 8 + count * 8,
    )?;
    write!(
        writer,
        "</Lines></Piece></PolyData>\n<AppendedData encoding=\"raw\">_"
    )?;
    length(&mut writer, count * 8)?;
    for point in trajectories.iter().flat_map(|t| &t.points) {
        writer.write_all(&point.time_us.to_le_bytes())?;
    }
    length(&mut writer, count * 24)?;
    for point in trajectories.iter().flat_map(|t| &t.points) {
        for v in point.velocity_mm_per_us {
            writer.write_all(&v.to_le_bytes())?;
        }
    }
    length(&mut writer, trajectories.len() * 8)?;
    for trajectory in trajectories {
        writer.write_all(&trajectory.id.to_le_bytes())?;
    }
    length(&mut writer, count * 24)?;
    for point in trajectories.iter().flat_map(|t| &t.points) {
        for v in point.position_mm {
            writer.write_all(&v.to_le_bytes())?;
        }
    }
    length(&mut writer, count * 8)?;
    for i in 0..count {
        writer.write_all(&(i as i64).to_le_bytes())?;
    }
    length(&mut writer, trajectories.len() * 8)?;
    let mut offset = 0_i64;
    for trajectory in trajectories {
        offset += trajectory.points.len() as i64;
        writer.write_all(&offset.to_le_bytes())?;
    }
    finish(&mut writer)
}

pub fn write_trajectories_csv(
    path: impl AsRef<Path>,
    trajectories: &[Trajectory],
) -> io::Result<()> {
    validate_trajectories(trajectories)?;
    let mut writer = BufWriter::new(File::create(path)?);
    writeln!(
        writer,
        "trajectory_id,time_us,x_mm,y_mm,z_mm,vx_mm_per_us,vy_mm_per_us,vz_mm_per_us"
    )?;
    for trajectory in trajectories {
        for p in &trajectory.points {
            let [x, y, z] = p.position_mm;
            let [vx, vy, vz] = p.velocity_mm_per_us;
            writeln!(
                writer,
                "{},{},{x},{y},{z},{vx},{vy},{vz}",
                trajectory.id, p.time_us
            )?;
        }
    }
    writer.flush()
}

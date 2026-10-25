use clap::{Args, Subcommand};
use simrep_io::{PaFile, read_pa, write_pa};
use simrep_mesh::{
    GridSpec, ImportOptions, SurfaceOptions, Trajectory, TrajectoryPoint, Transform, Unit,
    electrode_surface_with_options, inspect, pa_isosurface, read_mesh, repair, sample_sdf,
    voxelize, voxelize_with_defects, write_mesh, write_revolved_vts, write_scalar_vti,
    write_trajectories_csv, write_vti, write_vtp,
};
use simrep_solve::gpu::GpuResult;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

#[derive(Args)]
pub struct MeshImportOptions {
    pub input: PathBuf,
    /// Output PA#; potentials at metal points are electrode numbers.
    pub output: PathBuf,
    /// Number of nodes nx,ny,nz.
    #[arg(long, value_parser = dimensions)]
    grid: [usize; 3],
    /// Node spacing sx,sy,sz in mm.
    #[arg(long, default_value = "1,1,1", value_parser = numbers::<3>)]
    spacing: [f64; 3],
    /// Position of grid node (0,0,0) in mesh coordinates, in mm.
    #[arg(long, default_value = "0,0,0", value_parser = numbers::<3>, allow_hyphen_values = true)]
    origin: [f64; 3],
    /// Source unit override: mm, cm, m, inch (file default otherwise).
    #[arg(long)]
    units: Option<Unit>,
    /// Row-major 3x4 affine transform; translation in mm, after source scaling.
    #[arg(long, default_value = "1,0,0,0,0,1,0,0,0,0,1,0", value_parser = numbers::<12>, allow_hyphen_values = true)]
    transform: [f64; 12],
    /// One number per imported part; defaults to 1,2,... in reported part order.
    #[arg(long, value_delimiter = ',')]
    electrodes: Vec<u32>,
    /// Exact welding, degenerate/duplicate removal, consistent outward winding.
    #[arg(long)]
    repair: bool,
    /// Explicit best-effort parity for reported open/non-manifold surfaces.
    #[arg(long, conflicts_with = "sdf_prefix")]
    allow_defects: bool,
    /// Also write PREFIX-part.vti and PREFIX-part.json sampled SDFs.
    #[arg(long)]
    sdf_prefix: Option<PathBuf>,
    /// One voltage per part for SDF metadata (not PA# values); defaults to zero.
    #[arg(long, value_delimiter = ',', allow_hyphen_values = true)]
    voltages: Vec<f64>,
}

#[derive(Args)]
pub struct PaExport {
    input: PathBuf,
    output: PathBuf,
    #[arg(long, default_value = "0,0,0", value_parser = numbers::<3>, allow_hyphen_values = true)]
    origin: [f64; 3],
}

#[derive(Args)]
pub struct AngularOptions {
    /// Number of angular intervals for cylindrical revolution.
    #[arg(long, default_value_t = 64)]
    angular_segments: usize,
    /// Cylindrical sweep from +y toward +z, in degrees (0,360].
    #[arg(long, default_value_t = 360.0)]
    sweep_degrees: f64,
}

#[derive(Args)]
pub struct SurfaceExportOptions {
    #[command(flatten)]
    angular: AngularOptions,
    /// Planar 2D extrusion depth in mm, centered on origin z.
    #[arg(long, default_value_t = 1.0)]
    extrusion_depth: f64,
}

impl From<SurfaceExportOptions> for SurfaceOptions {
    fn from(options: SurfaceExportOptions) -> Self {
        Self {
            angular_segments: options.angular.angular_segments,
            sweep_degrees: options.angular.sweep_degrees,
            extrusion_depth_mm: options.extrusion_depth,
        }
    }
}

#[derive(Subcommand)]
pub enum ExportCommand {
    /// Exposed electrode voxel faces to STL, OBJ, glTF or GLB.
    Geometry {
        #[command(flatten)]
        files: PaExport,
        /// Select one electrode potential/number; otherwise export all.
        #[arg(long, allow_hyphen_values = true)]
        electrode: Option<f64>,
        #[command(flatten)]
        surface: SurfaceExportOptions,
    },
    /// Potential, electrode mask and field to VTI, or revolved cylindrical VTS.
    Grid {
        #[command(flatten)]
        files: PaExport,
        #[arg(long)]
        no_field: bool,
        /// Revolve a cylindrical PA into 3D StructuredGrid (.vts).
        #[arg(long)]
        revolve: bool,
        #[command(flatten)]
        angular: AngularOptions,
    },
    /// Potential surface (marching cubes or swept 2D contour) to a mesh.
    Isosurface {
        #[command(flatten)]
        files: PaExport,
        #[arg(long, allow_hyphen_values = true)]
        potential: f64,
        #[command(flatten)]
        surface: SurfaceExportOptions,
    },
    /// Numeric trajectory sample CSV to VTP polylines or normalized CSV.
    Trajectories { input: PathBuf, output: PathBuf },
}

fn numbers<const N: usize>(text: &str) -> Result<[f64; N], String> {
    let values: Vec<f64> = text
        .split(',')
        .map(|v| {
            v.trim()
                .parse::<f64>()
                .map_err(|_| "expected comma-separated numbers".to_owned())
        })
        .collect::<Result<_, _>>()?;
    if values.iter().any(|v| !v.is_finite()) {
        return Err("numbers must be finite".into());
    }
    values
        .try_into()
        .map_err(|_| format!("expected {N} comma-separated numbers"))
}

fn dimensions(text: &str) -> Result<[usize; 3], String> {
    let values = text
        .split(',')
        .map(|v| {
            v.trim()
                .parse::<usize>()
                .map_err(|_| "expected positive grid dimensions".to_owned())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let dims: [usize; 3] = values
        .try_into()
        .map_err(|_| "expected nx,ny,nz".to_owned())?;
    if dims.iter().any(|&n| n < 2) {
        return Err("3D import requires at least 2 nodes on every axis".into());
    }
    Ok(dims)
}

pub fn import(options: MeshImportOptions) -> GpuResult<()> {
    let grid = GridSpec {
        dims: options.grid,
        origin_mm: options.origin,
        spacing_mm: options.spacing,
    };
    grid.validate()?;
    if options.output.extension().is_none_or(|s| s != "pa#") {
        return Err("mesh-import output must have .pa# extension".into());
    }
    let mut matrix = Transform::IDENTITY;
    for i in 0..3 {
        matrix.0[i].copy_from_slice(&options.transform[i * 4..(i + 1) * 4]);
    }
    let mut mesh = read_mesh(
        &options.input,
        ImportOptions {
            units: options.units,
            transform: matrix,
        },
    )?;
    let electrodes = if options.electrodes.is_empty() {
        (1..=u32::try_from(mesh.parts.len())?).collect()
    } else {
        options.electrodes
    };
    let voltages = if options.voltages.is_empty() {
        vec![0.0; mesh.parts.len()]
    } else {
        options.voltages
    };
    if voltages.len() != mesh.parts.len() || voltages.iter().any(|v| !v.is_finite()) {
        return Err("supply one finite SDF voltage per part".into());
    }
    for (i, part) in mesh.parts.iter_mut().enumerate() {
        println!(
            "part {i}: {:?} ({} triangles), electrode={:?}",
            part.name,
            part.triangles.len(),
            electrodes.get(i)
        );
        if options.repair {
            println!("  repair: {:?}", repair(part)?);
        } else {
            println!("  check: {:?}", inspect(part)?);
        }
    }
    let result = if options.allow_defects {
        eprintln!("allow-defects: sampling the reported surface without topology repair");
        voxelize_with_defects(&mesh, grid, &electrodes)?
    } else {
        voxelize(&mesh, grid, &electrodes)?
    };
    write_pa(&options.output, &PaFile::new(result.array))?;
    println!(
        "voxelization: {:.3} s, counts={:?}, overlap assignments={}",
        result.elapsed.as_secs_f64(),
        result.points_per_part,
        result.overlap_assignments
    );
    println!("PA origin is external metadata: {:?} mm", grid.origin_mm);
    if let Some(prefix) = options.sdf_prefix {
        for (i, part) in mesh.parts.iter().enumerate() {
            let sdf = sample_sdf(part, grid, electrodes[i], voltages[i])?;
            let stem = format!("{}-{i}", prefix.display());
            write_scalar_vti(
                format!("{stem}.vti"),
                grid,
                "signed_distance_mm",
                &sdf.values,
            )?;
            let metadata = serde_json::json!({
                "name":sdf.name,"dims":sdf.dims,"origin":sdf.origin,"spacing":sdf.spacing,
                "values":sdf.values,"electrode_number":sdf.electrode_number,"voltage":sdf.voltage,
                "length_unit":"mm","inside":"negative","order":"x-fastest",
            });
            let mut writer = BufWriter::new(File::create(format!("{stem}.json"))?);
            serde_json::to_writer(&mut writer, &metadata)?;
            writer.flush()?;
            println!("SDF part {i}: {:.3} s", sdf.elapsed.as_secs_f64());
        }
    }
    Ok(())
}

pub fn export(command: ExportCommand) -> GpuResult<()> {
    match command {
        ExportCommand::Geometry {
            files,
            electrode,
            surface,
        } => {
            let pa = read_pa(&files.input)?;
            let mesh =
                electrode_surface_with_options(&pa.array, files.origin, electrode, surface.into())?;
            write_mesh(&files.output, &mesh)?;
            println!(
                "exported {} triangles in {} parts",
                mesh.triangle_count(),
                mesh.parts.len()
            );
        }
        ExportCommand::Grid {
            files,
            no_field,
            revolve,
            angular,
        } => {
            let pa = read_pa(&files.input)?.array;
            if revolve {
                if !has_extension(&files.output, "vts") {
                    return Err("revolved grid output must have .vts extension".into());
                }
                write_revolved_vts(
                    &files.output,
                    &pa,
                    files.origin,
                    !no_field,
                    angular.angular_segments,
                    angular.sweep_degrees,
                )?;
            } else {
                if !has_extension(&files.output, "vti") {
                    return Err(
                        "grid output must have .vti extension (or use --revolve for .vts)".into(),
                    );
                }
                write_vti(&files.output, &pa, files.origin, !no_field)?;
            }
        }
        ExportCommand::Isosurface {
            files,
            potential,
            surface,
        } => {
            let pa = read_pa(&files.input)?.array;
            write_mesh(
                &files.output,
                &pa_isosurface(&pa, files.origin, potential, surface.into())?,
            )?;
        }
        ExportCommand::Trajectories { input, output } => {
            let trajectories = read_trajectories(&input)?;
            if has_extension(&output, "vtp") {
                write_vtp(&output, &trajectories)?;
            } else if has_extension(&output, "csv") {
                write_trajectories_csv(&output, &trajectories)?;
            } else {
                return Err("trajectory output must have .vtp or .csv extension".into());
            }
        }
    }
    Ok(())
}

fn has_extension(path: &Path, extension: &str) -> bool {
    path.extension()
        .is_some_and(|s| s.eq_ignore_ascii_case(extension))
}

fn read_trajectories(path: &Path) -> io::Result<Vec<Trajectory>> {
    let mut lines = BufReader::new(File::open(path)?).lines();
    let expected = "trajectory_id,time_us,x_mm,y_mm,z_mm,vx_mm_per_us,vy_mm_per_us,vz_mm_per_us";
    if lines.next().transpose()?.as_deref() != Some(expected) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("trajectory CSV header must be {expected}"),
        ));
    }
    let mut groups: BTreeMap<u64, Vec<TrajectoryPoint>> = BTreeMap::new();
    for (i, line) in lines.enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let fields: Vec<_> = line.split(',').collect();
        if fields.len() != 8 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("trajectory CSV line {}: expected 8 numeric columns", i + 2),
            ));
        }
        let id = fields[0]
            .parse()
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid trajectory id"))?;
        let values = fields[1..]
            .iter()
            .map(|v| {
                v.parse::<f64>().map_err(|_| {
                    io::Error::new(io::ErrorKind::InvalidData, "invalid trajectory scalar")
                })
            })
            .collect::<io::Result<Vec<_>>>()?;
        groups.entry(id).or_default().push(TrajectoryPoint {
            time_us: values[0],
            position_mm: [values[1], values[2], values[3]],
            velocity_mm_per_us: [values[4], values[5], values[6]],
        });
    }
    Ok(groups
        .into_iter()
        .map(|(id, points)| Trajectory { id, points })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn parses_additive_mesh_commands() {
        let cli = crate::Cli::try_parse_from([
            "simrep",
            "mesh-import",
            "lens.stl",
            "lens.pa#",
            "--grid",
            "12,13,14",
            "--units",
            "inch",
            "--origin",
            "-1,0,0",
            "--electrodes",
            "3,7",
            "--repair",
        ])
        .unwrap();
        let crate::Command::MeshImport(options) = cli.command else {
            panic!("wrong command");
        };
        assert_eq!(options.grid, [12, 13, 14]);
        assert_eq!(options.units, Some(Unit::Inch));
        assert_eq!(options.electrodes, [3, 7]);
        assert!(options.repair);
        assert!(
            crate::Cli::try_parse_from(["simrep", "export", "grid", "lens.pa", "lens.vti"]).is_ok()
        );
        assert!(
            crate::Cli::try_parse_from([
                "simrep",
                "mesh-import",
                "lens.stl",
                "lens.pa#",
                "--grid",
                "12,13,1"
            ])
            .is_err()
        );
    }

    #[test]
    fn parses_two_dimensional_export_options() {
        let cli = crate::Cli::try_parse_from([
            "simrep",
            "export",
            "geometry",
            "einzel.pa0",
            "cutaway.glb",
            "--angular-segments",
            "96",
            "--sweep-degrees",
            "270",
            "--extrusion-depth",
            "5",
        ])
        .unwrap();
        let crate::Command::Export(ExportCommand::Geometry { surface, .. }) = cli.command else {
            panic!("wrong command");
        };
        let options: SurfaceOptions = surface.into();
        assert_eq!(options.angular_segments, 96);
        assert_eq!(options.sweep_degrees, 270.0);
        assert_eq!(options.extrusion_depth_mm, 5.0);
        assert!(
            crate::Cli::try_parse_from([
                "simrep",
                "export",
                "grid",
                "einzel.pa0",
                "volume.vts",
                "--revolve",
                "--angular-segments",
                "32",
                "--sweep-degrees",
                "270",
                "--no-field",
            ])
            .is_ok()
        );
        assert!(
            crate::Cli::try_parse_from([
                "simrep",
                "export",
                "isosurface",
                "planar.pa",
                "contour.obj",
                "--potential",
                "50",
                "--extrusion-depth",
                "2",
            ])
            .is_ok()
        );
    }
}

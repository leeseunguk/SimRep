use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use clap::{Args, ValueEnum};
use rayon::prelude::*;
use serde::Serialize;
use simrep_core::{FieldType, PotentialArray, Symmetry};
use simrep_fly::surface::{SourceGeometry, SourceSurface};
use simrep_geom::{Electrode, SignedDistance};
use simrep_io::{Pa0Metadata, PaFile, write_pa};
use simrep_mesh::{ImportOptions, Solid, Unit, read_mesh, repair};
use simrep_solve::adaptive::{AdaptiveGrid, Region, interpolate};
use simrep_solve::cutcell::{CutCells, surface_fraction};
use simrep_solve::gpu::{Gpu, GpuResult};
use simrep_solve::mg::{Geometry, Grid, Options};

use crate::{Backend, Device, parse_tolerance};

#[path = "geometry_gap.rs"]
mod gap;

const REFERENCE: f64 = 10_000.0;
type Source = (String, Arc<dyn SignedDistance>, SourceGeometry, GapSource);

enum GapSource {
    Gem([[f64; 3]; 2]),
    Mesh(Arc<Solid>),
    #[cfg(feature = "cad")]
    Cad(usize),
}

#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum CadMode {
    Auto,
    Exact,
    Tessellated,
}

#[derive(Args, Clone)]
pub struct GeometryOptions {
    /// GEM, STEP/IGES CAD, or STL/OBJ/PLY/3MF/glTF mesh input.
    input: PathBuf,
    /// Node counts nx,ny,nz; defaults to GEM pa_define.
    #[arg(long, value_parser = dimensions)]
    grid: Option<[usize; 3]>,
    /// Node spacing sx,sy,sz in mm; defaults to GEM spacing or 1 mm.
    #[arg(long, value_parser = numbers)]
    spacing: Option<[f64; 3]>,
    /// Physical coordinate of the first PA node (mm).
    #[arg(long, default_value = "0,0,0", value_parser = numbers, allow_hyphen_values = true)]
    origin: [f64; 3],
    /// GEM source electrode value, or mesh/CAD part name/index = number:volts.
    #[arg(long, required = true, value_parser = assignment, allow_hyphen_values = true)]
    electrode: Vec<Assignment>,
    /// Device used to refine each electrode basis.
    #[arg(long, value_enum, default_value = "cpu")]
    device: Device,
    /// GPU backend used when --device gpu is selected.
    #[arg(long, value_enum, default_value = "vulkan")]
    backend: Backend,
    /// Relative infinity norm tolerance for the physical f64 residual.
    #[arg(long, default_value = "1e-10", value_parser = parse_tolerance)]
    tolerance: f64,
    /// Maximum multigrid cycles for each electrode basis.
    #[arg(long, default_value_t = 100)]
    max_cycles: usize,
    /// Output stem; produces .pa0, .pa1..N, .cuts.json and .pa#.
    #[arg(long)]
    out: PathBuf,
    /// Include dense v1 links and the v2 sampled-SDF fallback with v3 sources.
    #[arg(long)]
    sidecar_dense_sdf: bool,
    /// Fallback samples per PA interval with --sidecar-dense-sdf.
    #[arg(long, default_value_t = 2, value_parser = supersampling)]
    surface_supersampling: usize,
    /// Mesh/CAD units: um/micron, mm/millimeter, cm/centimeter, m/meter, in/inch, ft/foot.
    #[arg(long)]
    units: Option<Unit>,
    /// Weld exact vertices, remove duplicate/degenerate faces, fix winding.
    #[arg(long)]
    repair: bool,
    /// CAD B-rep distance, or OCCT tessellation fallback (requires feature cad).
    #[arg(long, value_enum, default_value = "auto")]
    cad_mode: CadMode,
    /// Absolute CAD tessellation chordal deflection in mm (default h_min/100).
    #[arg(long)]
    cad_tolerance: Option<f64>,
    /// Cartesian base-node box xmin,ymin,zmin,xmax,ymax,zmax:factor (2 or 4).
    #[arg(long, value_parser = refine_region, allow_hyphen_values = true)]
    refine_region: Vec<Region>,
    /// Inter-electrode gap resolution policy (2.5 cells minimum, 10 recommended).
    #[arg(long, value_enum, default_value = "warn")]
    gap_check: gap::Mode,
}

fn refine_region(text: &str) -> Result<Region, String> {
    let (bounds, factor) = text
        .rsplit_once(':')
        .ok_or("expected xmin,ymin,zmin,xmax,ymax,zmax:factor")?;
    let bounds = bounds
        .split(',')
        .map(str::parse::<f64>)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "expected six finite bounds")?;
    let factor = factor
        .parse::<usize>()
        .map_err(|_| "factor must be 2 or 4")?;
    if bounds.len() != 6 || bounds.iter().any(|v| !v.is_finite()) || ![2, 4].contains(&factor) {
        return Err("expected six finite bounds and factor 2 or 4".into());
    }
    Ok(Region {
        bounds_mm: [
            [bounds[0], bounds[1], bounds[2]],
            [bounds[3], bounds[4], bounds[5]],
        ],
        factor,
    })
}

#[derive(Clone, Debug, Serialize)]
struct Assignment {
    part: String,
    number: usize,
    voltage: f64,
}

fn numbers(text: &str) -> Result<[f64; 3], String> {
    let v = text
        .split(',')
        .map(str::parse::<f64>)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "expected three comma-separated numbers")?;
    if v.iter().any(|x| !x.is_finite()) {
        return Err("coordinates must be finite".into());
    }
    v.try_into()
        .map_err(|_| "expected three comma-separated numbers".into())
}

fn dimensions(text: &str) -> Result<[usize; 3], String> {
    let v = text
        .split(',')
        .map(str::parse::<usize>)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "expected nx,ny,nz")?;
    let v: [usize; 3] = v.try_into().map_err(|_| "expected nx,ny,nz")?;
    if v[0] < 2 || v[1] < 2 || v[2] == 0 {
        return Err("nx,ny must be at least 2; nz at least 1".into());
    }
    Ok(v)
}

fn supersampling(text: &str) -> Result<usize, String> {
    text.parse::<usize>()
        .ok()
        .filter(|n| *n > 0)
        .ok_or_else(|| "surface supersampling must be a positive integer".into())
}

fn assignment(text: &str) -> Result<Assignment, String> {
    let (part, value) = text.split_once('=').ok_or("expected part=number:voltage")?;
    let (number, voltage) = value
        .split_once(':')
        .ok_or("expected part=number:voltage")?;
    let number: usize = number
        .parse()
        .map_err(|_| "expected part=number:voltage with integer number 1..1000")?;
    let voltage: f64 = voltage
        .parse()
        .map_err(|_| "expected part=number:voltage with finite voltage")?;
    if part.is_empty() || !(1..=1000).contains(&number) || !voltage.is_finite() {
        return Err("part is required, number must be 1..1000, voltage finite".into());
    }
    Ok(Assignment {
        part: part.into(),
        number,
        voltage,
    })
}

struct Part {
    name: String,
    shape: Arc<dyn SignedDistance>,
    assignment: Assignment,
    source: SourceGeometry,
    gap_source: GapSource,
}

struct SolverShape {
    source: Arc<dyn SignedDistance>,
    cylindrical: bool,
    tolerance: f64,
}

impl SolverShape {
    fn point(&self, mut p: [f64; 3]) -> [f64; 3] {
        if self.cylindrical {
            p.swap(0, 1);
        }
        p
    }

    fn snap(&self, distance: f64) -> f64 {
        if distance.abs() <= self.tolerance {
            0.0
        } else {
            distance
        }
    }
}

impl SignedDistance for SolverShape {
    fn distance(&self, p: [f64; 3]) -> f64 {
        self.snap(self.source.distance(self.point(p)))
    }

    fn segment_distance_bounds(&self, from: [f64; 3], to: [f64; 3]) -> Option<[f64; 2]> {
        self.source
            .segment_distance_bounds(self.point(from), self.point(to))
            .map(|bounds| bounds.map(|d| self.snap(d)))
    }

    fn segment_single_interval(&self, from: [f64; 3], to: [f64; 3], level: f64) -> bool {
        self.source.segment_single_interval(
            self.point(from),
            self.point(to),
            if level.abs() <= self.tolerance {
                self.tolerance
            } else {
                level
            },
        )
    }

    fn segment_clear(&self, from: [f64; 3], to: [f64; 3], tolerance: f64) -> bool {
        self.source
            .segment_clear(self.point(from), self.point(to), tolerance)
    }

    fn conductor_mask(
        &self,
        dims: [usize; 3],
        origin: [f64; 3],
        spacing: [f64; 3],
        threshold: f64,
        mask: &mut [bool],
    ) -> Result<(), &'static str> {
        if self.cylindrical {
            let bad = std::sync::atomic::AtomicBool::new(false);
            mask.par_iter_mut().enumerate().for_each(|(i, flag)| {
                let c = [i % dims[0], i / dims[0] % dims[1], i / (dims[0] * dims[1])];
                let point = std::array::from_fn(|a| origin[a] + c[a] as f64 * spacing[a]);
                let distance = self.distance(point);
                if !distance.is_finite() {
                    bad.store(true, std::sync::atomic::Ordering::Relaxed);
                }
                *flag = distance <= threshold;
            });
            return if bad.load(std::sync::atomic::Ordering::Relaxed) {
                Err("SDF returned a non-finite distance")
            } else {
                Ok(())
            };
        }
        // Snap only zeros |d| <= tolerance, which is already inside the
        // snap threshold used by CutCells, so the source comparison matches.
        self.source
            .conductor_mask(dims, origin, spacing, threshold.max(self.tolerance), mask)
    }

    fn mark_near_nodes(
        &self,
        dims: [usize; 3],
        origin: [f64; 3],
        spacing: [f64; 3],
        band: f64,
        near: &mut [bool],
    ) {
        if self.cylindrical {
            near.fill(true);
        } else {
            self.source
                .mark_near_nodes(dims, origin, spacing, band, near);
        }
    }
}

#[derive(Serialize)]
struct Sidecar {
    version: u32,
    dims: [usize; 3],
    origin_mm: [f64; 3],
    spacing_mm: [f64; 3],
    symmetry: &'static str,
    mirrors: [bool; 3],
    electrode_numbers: Vec<usize>,
    #[serde(skip_serializing)]
    sampled_sdfs: Vec<simrep_fly::surface::SurfaceSamples>,
    source_geometry: Vec<SourceSurface>,
    assignments: Vec<Assignment>,
    /// PA x-fastest order; None means vacuum, including solver-only snaps.
    #[serde(skip_serializing)]
    node_electrodes: Vec<Option<usize>>,
    #[serde(skip_serializing)]
    solver_fixed: Vec<bool>,
    /// Physical first intersections, ordered -x,+x,-y,+y,-z,+z.
    #[serde(skip_serializing)]
    cuts: Vec<PhysicalCuts>,
    surface_tolerance_mm: f64,
    snap_distance_mm: f64,
}

#[derive(Serialize)]
struct PhysicalCuts {
    node: usize,
    fractions: [Option<f64>; 6],
    electrodes: [Option<usize>; 6],
}

#[derive(Serialize)]
#[serde(untagged)]
enum CutValues<'a> {
    Fractions(&'a [Option<f64>; 6]),
    Electrodes(&'a [Option<usize>; 6]),
}

fn write_json_array<T: Serialize + Sync>(writer: &mut impl Write, values: &[T]) -> GpuResult<()> {
    const CHUNK: usize = 16_384;
    writer.write_all(b"[")?;
    let mut first = true;
    // Bound temporary JSON storage, while formatting chunks in parallel.
    for batch in values.chunks(CHUNK * rayon::current_num_threads()) {
        let chunks = batch
            .par_chunks(CHUNK)
            .map(serde_json::to_vec)
            .collect::<Result<Vec<_>, _>>()?;
        for chunk in chunks {
            if !first {
                writer.write_all(b",")?;
            }
            writer.write_all(&chunk[1..chunk.len() - 1])?;
            first = false;
        }
    }
    writer.write_all(b"]")?;
    Ok(())
}

fn write_json_runs<T: Serialize + PartialEq>(
    writer: &mut impl Write,
    values: &[T],
) -> GpuResult<()> {
    writer.write_all(b"[")?;
    let mut start = 0;
    while start < values.len() {
        let end = start
            + values[start..]
                .iter()
                .position(|value| *value != values[start])
                .unwrap_or(values.len() - start);
        if start != 0 {
            writer.write_all(b",")?;
        }
        serde_json::to_writer(&mut *writer, &(start, end, &values[start]))?;
        start = end;
    }
    writer.write_all(b"]")?;
    Ok(())
}

impl Sidecar {
    fn write_cuts(&self, writer: &mut impl Write, electrodes: bool) -> GpuResult<()> {
        const CHUNK: usize = 16_384;
        let empty = [None; 6];
        let mut row = serde_json::to_vec(&empty)?;
        row.push(b',');
        let blank = row.repeat(CHUNK);
        let count = self.node_electrodes.len();
        let chunks = count.div_ceil(CHUNK);
        let mut first = true;
        writer.write_all(b"[")?;
        for batch in (0..chunks).step_by(rayon::current_num_threads()) {
            let end = (batch + rayon::current_num_threads()).min(chunks);
            let encoded = (batch..end)
                .into_par_iter()
                .map(|chunk| {
                    let low = chunk * CHUNK;
                    let high = (low + CHUNK).min(count);
                    let mut cut = self.cuts.partition_point(|cut| cut.node < low);
                    if self.cuts.get(cut).is_none_or(|cut| cut.node >= high) {
                        return Ok(None);
                    }
                    let values: Vec<_> = (low..high)
                        .map(|node| {
                            if let Some(hit) = self.cuts.get(cut)
                                && hit.node == node
                            {
                                cut += 1;
                                if electrodes {
                                    CutValues::Electrodes(&hit.electrodes)
                                } else {
                                    CutValues::Fractions(&hit.fractions)
                                }
                            } else {
                                CutValues::Fractions(&empty)
                            }
                        })
                        .collect();
                    serde_json::to_vec(&values).map(Some)
                })
                .collect::<Result<Vec<_>, serde_json::Error>>()?;
            for (i, encoded) in encoded.into_iter().enumerate() {
                if !first {
                    writer.write_all(b",")?;
                }
                if let Some(bytes) = encoded {
                    writer.write_all(&bytes[1..bytes.len() - 1])?;
                } else {
                    let rows = (count - (batch + i) * CHUNK).min(CHUNK);
                    writer.write_all(&blank[..row.len() * rows - 1])?;
                }
                first = false;
            }
        }
        writer.write_all(b"]")?;
        Ok(())
    }

    fn write(&self, writer: &mut impl Write) -> GpuResult<()> {
        let metadata = serde_json::to_vec(self)?;
        writer.write_all(&metadata[..metadata.len() - 1])?;
        writer.write_all(b",\"sampled_sdfs\":[")?;
        for (i, samples) in self.sampled_sdfs.iter().enumerate() {
            if i != 0 {
                writer.write_all(b",")?;
            }
            let metadata = serde_json::to_vec(&serde_json::json!({
                "number": samples.number,
                "dims": samples.dims,
                "origin_mm": samples.origin_mm,
                "spacing_mm": samples.spacing_mm,
            }))?;
            writer.write_all(&metadata[..metadata.len() - 1])?;
            writer.write_all(b",\"values\":")?;
            write_json_array(writer, &samples.values)?;
            writer.write_all(b"}")?;
        }
        writer.write_all(b"]")?;
        if self.sampled_sdfs.is_empty() {
            writer.write_all(b",\"node_electrode_runs\":")?;
            write_json_runs(writer, &self.node_electrodes)?;
            writer.write_all(b",\"solver_fixed_runs\":")?;
            write_json_runs(writer, &self.solver_fixed)?;
            writer.write_all(b",\"link_intersections\":")?;
            write_json_array(writer, &self.cuts)?;
        } else {
            writer.write_all(b",\"node_electrodes\":")?;
            write_json_array(writer, &self.node_electrodes)?;
            writer.write_all(b",\"solver_fixed\":")?;
            write_json_array(writer, &self.solver_fixed)?;
            writer.write_all(b",\"cut_fractions\":")?;
            self.write_cuts(writer, false)?;
            writer.write_all(b",\"cut_electrodes\":")?;
            self.write_cuts(writer, true)?;
        }
        writer.write_all(b"}")?;
        Ok(())
    }
}

fn point(i: usize, dims: [usize; 3], origin: [f64; 3], h: [f64; 3]) -> [f64; 3] {
    let c = [i % dims[0], i / dims[0] % dims[1], i / (dims[0] * dims[1])];
    std::array::from_fn(|a| origin[a] + c[a] as f64 * h[a])
}

fn permute<T: Copy>(v: &[T], nx: usize, ny: usize) -> Vec<T> {
    (0..v.len()).map(|i| v[i / ny + nx * (i % ny)]).collect()
}

pub fn run(args: GeometryOptions) -> GpuResult<()> {
    let cad = args.input.extension().is_some_and(|s| {
        ["step", "stp", "iges", "igs"]
            .iter()
            .any(|extension| s.eq_ignore_ascii_case(extension))
    });
    if !cad && (args.cad_mode != CadMode::Auto || args.cad_tolerance.is_some()) {
        return Err("--cad-mode and --cad-tolerance apply only to CAD input: .step, .stp, .iges or .igs; --cad-mode accepts auto, exact or tessellated".into());
    }
    if !args.input.extension().is_some_and(|s| {
        [
            "gem", "step", "stp", "iges", "igs", "stl", "obj", "ply", "3mf", "gltf", "glb",
        ]
        .iter()
        .any(|extension| s.eq_ignore_ascii_case(extension))
    }) {
        return Err("refine-geometry input must be .gem, .step, .stp, .iges, .igs, .stl, .obj, .ply, .3mf, .gltf or .glb".into());
    }
    if cad && args.cad_mode == CadMode::Auto {
        match run_once(args.clone()) {
            Err(error)
                if error
                    .to_string()
                    .contains("SDF returned a non-finite distance") =>
            {
                eprintln!(
                    "OCCT distance failed; restarting the complete CAD solve with tessellation"
                );
                return run_once(GeometryOptions {
                    cad_mode: CadMode::Tessellated,
                    ..args
                });
            }
            result => return result,
        }
    }
    run_once(args)
}

fn run_once(args: GeometryOptions) -> GpuResult<()> {
    let total = Instant::now();
    let cad_failed = Arc::new(AtomicBool::new(false));
    let gem = args
        .input
        .extension()
        .is_some_and(|s| s.eq_ignore_ascii_case("gem"));
    let cad = args.input.extension().is_some_and(|s| {
        ["step", "stp", "iges", "igs"]
            .iter()
            .any(|extension| s.eq_ignore_ascii_case(extension))
    });
    let (mut pa, sources): (PaFile, Vec<Source>) = if gem {
        if args.units.is_some() || args.repair {
            return Err("--units and --repair apply only to mesh input".into());
        }
        let gem = simrep_gem::read_gem(&args.input)?;
        if gem.field_type != FieldType::Electrostatic || !gem.refinable {
            return Err("refine-geometry requires refinable electrostatic geometry".into());
        }
        let dims = args.grid.unwrap_or(gem.dimensions);
        let spacing = args.spacing.unwrap_or(gem.spacing_mm);
        validate_grid(dims, args.origin, spacing)?;
        let mut a = PotentialArray::new(dims[0], dims[1], dims[2]);
        a.symmetry = gem.symmetry;
        a.mirrors = gem.mirrors;
        [a.dx_mm, a.dy_mm, a.dz_mm] = spacing;
        let sources = gem
            .electrode_sdfs()?
            .into_iter()
            .map(|s| {
                let name = s.voltage.to_string();
                let source = SourceGeometry::gem(&s)?;
                let bounds = s.bounds_mm();
                Ok((
                    name,
                    Arc::new(s) as Arc<dyn SignedDistance>,
                    source,
                    GapSource::Gem(bounds),
                ))
            })
            .collect::<Result<Vec<_>, Box<dyn std::error::Error + Send + Sync>>>()?;
        let mut pa = PaFile::new(a);
        pa.ng = gem.ng;
        (pa, sources)
    } else if cad {
        cad_sources(&args, Arc::clone(&cad_failed))?
    } else {
        let dims = args.grid.ok_or("mesh input requires --grid nx,ny,nz")?;
        let spacing = args.spacing.unwrap_or([1.0; 3]);
        validate_grid(dims, args.origin, spacing)?;
        if dims[2] < 2 {
            return Err("mesh input requires a planar 3D grid (nz >= 2)".into());
        }
        let mut mesh = read_mesh(
            &args.input,
            ImportOptions {
                units: args.units,
                ..Default::default()
            },
        )?;
        let mut sources = Vec::new();
        for (i, part) in mesh.parts.iter_mut().enumerate() {
            if args.repair {
                let report = repair(part)?;
                println!("Repair part {i} {:?}: {report:?}", part.name);
            }
            let solid = Arc::new(Solid::new(part)?);
            let name = if part.name.is_empty() {
                i.to_string()
            } else {
                part.name.clone()
            };
            sources.push((
                name,
                Arc::clone(&solid) as Arc<dyn SignedDistance>,
                SourceGeometry::mesh(part, None)?,
                GapSource::Mesh(solid),
            ));
        }
        let mut a = PotentialArray::new(dims[0], dims[1], dims[2]);
        [a.dx_mm, a.dy_mm, a.dz_mm] = spacing;
        (PaFile::new(a), sources)
    };
    let a = &pa.array;
    let dims = [a.nx, a.ny, a.nz];
    let spacing = [a.dx_mm, a.dy_mm, a.dz_mm];
    let cylindrical = a.symmetry == Symmetry::Cylindrical;
    if !args.refine_region.is_empty() && (cylindrical || args.device == Device::Gpu) {
        return Err(
            "adaptive refinement requires Cartesian geometry and --device cpu or auto".into(),
        );
    }
    if cylindrical && (dims[2] != 1 || args.origin[1] != 0.0 || args.origin[2] != 0.0) {
        return Err("cylindrical PA requires nz=1 and radial/z origins zero; shift axial origin or spacing instead".into());
    }
    let mut used = vec![false; args.electrode.len()];
    let mut voltages = BTreeMap::new();
    let mut parts = Vec::new();
    for (i, (name, shape, source, gap_source)) in sources.into_iter().enumerate() {
        let matches: Vec<_> = args
            .electrode
            .iter()
            .enumerate()
            .filter(|(_, e)| e.part == name || (!gem && e.part == i.to_string()))
            .collect();
        if matches.len() != 1 {
            return Err(
                format!("part {i} {name:?} requires exactly one --electrode assignment").into(),
            );
        }
        let (slot, assignment) = matches[0];
        if used[slot] {
            return Err(format!("ambiguous part selector {:?}", assignment.part).into());
        }
        used[slot] = true;
        if voltages
            .insert(assignment.number, assignment.voltage)
            .is_some_and(|v| v != assignment.voltage)
        {
            return Err("parts sharing an electrode number must have the same voltage".into());
        }
        println!(
            "Part {i} {name:?} -> electrode {}, {} V",
            assignment.number, assignment.voltage
        );
        parts.push(Part {
            name,
            shape,
            assignment: assignment.clone(),
            source,
            gap_source,
        });
    }
    if used.contains(&false) {
        return Err("an --electrode selector did not match any input part".into());
    }
    let h_min = (0..3)
        .filter(|&a| dims[a] > 1)
        .map(|a| spacing[a])
        .fold(f64::INFINITY, f64::min);
    let tolerance = 1e-9 * h_min;
    let gap_check = gap::check(&args, &parts, dims, spacing)?;
    let source_time = total.elapsed();
    let phase = Instant::now();
    let mut owners = vec![None; a.potentials.len()];
    let mut indices =
        vec![-1; (*voltages.keys().max().ok_or("geometry has no electrodes")? + 1).max(32)];
    let masks = parts
        .par_iter()
        .map(|part| {
            let mut mask = vec![false; owners.len()];
            part.shape
                .conductor_mask(dims, args.origin, spacing, tolerance, &mut mask)
                .map_err(|error| -> Box<dyn std::error::Error + Send + Sync> { error.into() })?;
            Ok(mask)
        })
        .collect::<GpuResult<Vec<_>>>()?;
    validate_cad_queries(&cad_failed)?;
    for (part, mask) in parts.iter().zip(&masks) {
        for (owner, &hit) in owners.iter_mut().zip(mask) {
            if !hit {
                continue;
            }
            if owner.is_some_and(|n| n != part.assignment.number) {
                return Err(format!(
                    "different electrode numbers overlap at a PA node ({})",
                    part.name
                )
                .into());
            }
            *owner = Some(part.assignment.number);
        }
    }
    for (i, owner) in owners.iter().enumerate() {
        if let Some(number) = owner
            && indices[*number] == -1
        {
            indices[*number] = i as i64;
        }
    }
    for number in voltages.keys() {
        if indices[*number] == -1 {
            return Err(format!(
                "electrode {number} has no inside/on-surface grid node; refine the grid"
            )
            .into());
        }
    }
    let owner_time = phase.elapsed();
    let phase = Instant::now();
    let grid = Grid {
        dims: if cylindrical {
            [dims[1], dims[0], 1]
        } else {
            dims
        },
        spacing: if cylindrical {
            [spacing[1], spacing[0], spacing[2]]
        } else {
            spacing
        },
        geometry: if cylindrical {
            Geometry::Cylindrical { radial_origin: 0.0 }
        } else {
            Geometry::Planar
        },
    };
    let origin = if cylindrical {
        [0.0, args.origin[0], 0.0]
    } else {
        args.origin
    };
    let electrodes = parts
        .iter()
        .map(|p| {
            Electrode::new(
                p.assignment.number as f64,
                SolverShape {
                    source: Arc::clone(&p.shape),
                    cylindrical,
                    tolerance,
                },
            )
        })
        .collect::<Vec<_>>();
    let cuts = CutCells::new_with_root_tolerance(grid, origin, &electrodes, tolerance)?;
    validate_cad_queries(&cad_failed)?;
    let geometry_time = phase.elapsed();
    let phase = Instant::now();
    let fixed = if cylindrical {
        permute(&cuts.fixed, grid.dims[0], grid.dims[1])
    } else {
        cuts.fixed.clone()
    };
    let link_band = spacing.into_iter().fold(0.0_f64, f64::max) + tolerance;
    let near_parts: Vec<Vec<bool>> = parts
        .par_iter()
        .map(|part| {
            let mut near = vec![false; owners.len()];
            part.shape
                .mark_near_nodes(dims, args.origin, spacing, link_band, &mut near);
            near
        })
        .collect();
    let physical_cuts = owners
        .par_iter()
        .enumerate()
        .filter_map(|(i, owner)| {
            if owner.is_some() || near_parts.iter().all(|near| !near[i]) {
                return None;
            }
            let mut fractions = [None; 6];
            let mut cut_electrodes = [None; 6];
            let p = point(i, dims, args.origin, spacing);
            let c = [i % dims[0], i / dims[0] % dims[1], i / (dims[0] * dims[1])];
            for s in 0..6 {
                let axis = s / 2;
                if (s % 2 == 0 && c[axis] == 0) || (s % 2 == 1 && c[axis] + 1 == dims[axis]) {
                    continue;
                }
                let mut q = p;
                q[axis] += if s % 2 == 0 {
                    -spacing[axis]
                } else {
                    spacing[axis]
                };
                for (part, near) in parts.iter().zip(&near_parts) {
                    if !near[i] || part.shape.segment_clear(p, q, tolerance) {
                        continue;
                    }
                    if let Some(t) = surface_fraction(part.shape.as_ref(), p, q, tolerance)
                        && fractions[s].is_none_or(|previous| t < previous)
                    {
                        fractions[s] = Some(t);
                        cut_electrodes[s] = Some(part.assignment.number);
                    }
                }
            }
            fractions
                .iter()
                .any(Option::is_some)
                .then_some(PhysicalCuts {
                    node: i,
                    fractions,
                    electrodes: cut_electrodes,
                })
        })
        .collect();
    validate_cad_queries(&cad_failed)?;
    let link_time = phase.elapsed();
    let phase = Instant::now();
    pa.array.electrodes = owners.iter().map(Option::is_some).collect();
    pa.array.potentials = owners.iter().map(|n| n.unwrap_or(0) as f64).collect();
    // Same physical mask as the cut-cell output, for an actual `simrep refine`
    // staircase comparison; no solver-only snapping is exported as metal.
    let geometry_path = args.out.with_extension("pa#");
    write_pa(&geometry_path, &pa)?;
    let mut output_paths = vec![geometry_path];
    let sample_dims = dims.map(|n| {
        (n - 1)
            .checked_mul(args.surface_supersampling)
            .and_then(|n| n.checked_add(1))
    });
    let sample_dims = sample_dims
        .into_iter()
        .collect::<Option<Vec<_>>>()
        .ok_or("surface sample dimensions overflow")?;
    let sample_dims = [sample_dims[0], sample_dims[1], sample_dims[2]];
    let sample_count = sample_dims
        .into_iter()
        .try_fold(1usize, usize::checked_mul)
        .ok_or("surface sample count overflows usize")?;
    let sample_spacing = spacing.map(|h| h / args.surface_supersampling as f64);
    let electrode_numbers: Vec<_> = voltages.keys().copied().collect();
    let sampled_sdfs = electrode_numbers
        .iter()
        .filter(|_| args.sidecar_dense_sdf)
        .map(|&number| {
            let mut sources = parts.iter().filter(|part| part.assignment.number == number);
            let mut values = vec![0.0; sample_count];
            sources.next().unwrap().shape.sample_grid(
                sample_dims,
                args.origin,
                sample_spacing,
                &mut values,
            );
            for part in sources {
                let mut other = vec![0.0; sample_count];
                part.shape
                    .sample_grid(sample_dims, args.origin, sample_spacing, &mut other);
                values
                    .par_iter_mut()
                    .zip(other)
                    .for_each(|(value, next)| *value = value.min(next));
            }
            let samples =
                simrep_geom::SampledSdf::new(sample_dims, args.origin, sample_spacing, values)?;
            Ok(simrep_fly::surface::SurfaceSamples {
                number,
                dims: sample_dims,
                origin_mm: args.origin,
                spacing_mm: sample_spacing,
                values: samples.values,
            })
        })
        .collect::<GpuResult<Vec<_>>>()?;
    validate_cad_queries(&cad_failed)?;
    let sample_time = phase.elapsed();
    let phase = Instant::now();
    let sidecar = Sidecar {
        version: 3,
        dims,
        origin_mm: args.origin,
        spacing_mm: spacing,
        symmetry: if cylindrical { "cylindrical" } else { "planar" },
        mirrors: [pa.array.mirrors.x, pa.array.mirrors.y, pa.array.mirrors.z],
        electrode_numbers,
        sampled_sdfs,
        source_geometry: parts
            .iter()
            .map(|p| SourceSurface {
                number: p.assignment.number,
                geometry: p.source.clone(),
            })
            .collect(),
        assignments: args.electrode.clone(),
        node_electrodes: owners,
        solver_fixed: fixed,
        cuts: physical_cuts,
        surface_tolerance_mm: tolerance,
        snap_distance_mm: cuts.snap_distance,
    };
    let sidecar_path = args.out.with_extension("cuts.json");
    let mut writer = BufWriter::new(File::create(&sidecar_path)?);
    sidecar.write(&mut writer)?;
    writer.flush()?;
    output_paths.push(sidecar_path);
    let sidecar_time = phase.elapsed();
    println!(
        "geometry_preparation_ms: source={:.3}, owners={:.3}, cut_cells={:.3}, links={:.3}, samples={:.3}, sidecar={:.3}",
        source_time.as_secs_f64() * 1000.0,
        owner_time.as_secs_f64() * 1000.0,
        geometry_time.as_secs_f64() * 1000.0,
        link_time.as_secs_f64() * 1000.0,
        sample_time.as_secs_f64() * 1000.0,
        sidecar_time.as_secs_f64() * 1000.0,
    );
    if !args.refine_region.is_empty() {
        let adaptive = AdaptiveGrid::new(cuts, &args.refine_region, &electrodes)?;
        pa.pa0 = Some(Pa0Metadata {
            reference_voltage: REFERENCE,
            electrode_indices: indices,
            scalable: None,
        });
        return adaptive_export(
            &args,
            pa,
            adaptive,
            &voltages,
            &electrodes,
            output_paths,
            (total.elapsed(), total),
        );
    }
    let gpu = if args.device == Device::Gpu {
        let gpu =
            pollster::block_on(Gpu::new(args.backend.backends()))?.ok_or("no GPU adapter found")?;
        crate::require_hardware_gpu(&gpu.info)?;
        crate::report_gpu_residual(gpu.has_f64());
        println!(
            "Adapter: {} ({:?}, {:?})",
            gpu.info.name, gpu.info.backend, gpu.info.device_type
        );
        Some(gpu)
    } else {
        None
    };
    let mut compute = Duration::ZERO;
    let mut preparation = total.elapsed();
    let mut combined = vec![0.0; pa.array.potentials.len()];
    let mut total_cycles = 0;
    let mut final_residual = 0.0_f64;
    let mut all_converged = true;
    for (&number, &voltage) in &voltages {
        let start = Instant::now();
        let cuts = cuts.with_voltages(|owner| {
            if owner == number as f64 {
                REFERENCE
            } else {
                0.0
            }
        })?;
        validate_cad_queries(&cad_failed)?;
        let mut values = cuts.values.clone();
        let basis_preparation = start.elapsed();
        preparation += basis_preparation;
        println!(
            "Basis {number}: geometry_ms={:.3}",
            basis_preparation.as_secs_f64() * 1000.0
        );
        let options = Options {
            relative_tolerance: args.tolerance,
            max_cycles: args.max_cycles,
            ..Default::default()
        };
        let (converged, cycles, residual, elapsed) = if let Some(gpu) = &gpu {
            let r = gpu.solve_sdf(&mut values, &cuts, options)?;
            preparation += r.timings.setup + r.timings.upload + r.timings.readback;
            (
                r.converged,
                r.iterations,
                r.residual_history.last().unwrap().relative,
                r.timings.solve,
            )
        } else {
            let r = cuts.solve(&mut values, options)?;
            preparation += r.timings.setup;
            (
                r.converged,
                r.iterations,
                r.residual_history.last().unwrap().relative,
                r.timings.solve,
            )
        };
        compute += elapsed;
        total_cycles += cycles;
        final_residual = final_residual.max(residual);
        all_converged &= converged;
        println!(
            "Basis {number}: compute_ms={:.6}, cycles={cycles}, relative_residual={residual:.9e}",
            elapsed.as_secs_f64() * 1000.0
        );
        if !converged {
            return Err(
                format!("electrode {number} did not converge after {cycles} cycles").into(),
            );
        }
        pa.array.potentials = if cylindrical {
            permute(&values, grid.dims[0], grid.dims[1])
        } else {
            values
        };
        for (v, basis) in combined.iter_mut().zip(&pa.array.potentials) {
            *v += basis * (voltage / REFERENCE);
        }
        let path = args.out.with_extension(format!("pa{number}"));
        write_pa(&path, &pa)?;
        output_paths.push(path);
    }
    pa.array.potentials = combined;
    pa.pa0 = Some(Pa0Metadata {
        reference_voltage: REFERENCE,
        electrode_indices: indices,
        scalable: None,
    });
    let path = args.out.with_extension("pa0");
    write_pa(&path, &pa)?;
    output_paths.push(path);
    println!(
        "Grid: {dims:?}, origin_mm={:?}, spacing_mm={spacing:?}, cut_nodes={}",
        args.origin,
        cuts.cut_nodes()
    );
    println!(
        "compute_ms={:.6}, preparation_ms={:.6}, end_to_end_ms={:.6}",
        compute.as_secs_f64() * 1000.0,
        preparation.as_secs_f64() * 1000.0,
        total.elapsed().as_secs_f64() * 1000.0
    );
    crate::json::set_summary(serde_json::json!({
        "requested": {
            "device": args.device.to_possible_value().unwrap().get_name(),
            "backend": args.backend.to_possible_value().unwrap().get_name(),
            "precision": "f64",
        },
        "executed": {
            "device": if gpu.is_some() { "gpu" } else { "cpu" },
            "backend": gpu.as_ref().map(|gpu| format!("{:?}", gpu.info.backend).to_lowercase()),
            "precision": if gpu.is_some() { "mixed-f32-f64" } else { "f64" },
        },
        "fallback_reason": gpu.as_ref().filter(|g| !g.has_f64())
            .map(|_| "adapter lacks native f64; CPU f64 residual with GPU f32 corrections"),
        "reason": if gpu.is_some() { "explicit --device gpu" } else { "explicit --device cpu" },
        "cycles": total_cycles,
        "final_residual": final_residual,
        "tolerance": args.tolerance,
        "converged": all_converged,
        "compute_ms": compute.as_secs_f64() * 1000.0,
        "prep_ms": preparation.as_secs_f64() * 1000.0,
        "output_paths": output_paths,
        "gap_check": gap_check,
    }));
    Ok(())
}

fn adaptive_export(
    args: &GeometryOptions,
    mut pa: PaFile,
    adaptive: AdaptiveGrid,
    voltages: &BTreeMap<usize, f64>,
    electrodes: &[Electrode],
    mut paths: Vec<PathBuf>,
    timing: (Duration, Instant),
) -> GpuResult<()> {
    use simrep_fly::adaptive::{AdaptiveManifest, AdaptivePatchManifest};
    let metadata = pa.pa0.take().unwrap();
    let mut manifest = AdaptiveManifest {
        schema_version: 1,
        base: args
            .out
            .with_extension("pa0")
            .file_name()
            .unwrap()
            .to_str()
            .ok_or("output name must be UTF-8")?
            .into(),
        origin_mm: args.origin,
        spacing_mm: adaptive.base.grid.spacing,
        reference_voltage: REFERENCE,
        electrode_voltages: voltages.clone(),
        patches: Vec::new(),
    };
    let mut patch_files = Vec::new();
    for (slot, patch) in adaptive.patches.iter().enumerate() {
        let lower: [usize; 3] = std::array::from_fn(|a| (patch.lo[a] * patch.factor).min(2));
        let upper: [usize; 3] = std::array::from_fn(|a| {
            ((adaptive.base.grid.dims[a] - 1 - patch.hi[a]) * patch.factor).min(2)
        });
        let dims: [usize; 3] =
            std::array::from_fn(|a| patch.cuts.grid.dims[a] + lower[a] + upper[a]);
        let spacing = patch.cuts.grid.spacing;
        let origin: [f64; 3] =
            std::array::from_fn(|a| patch.cuts.origin[a] - lower[a] as f64 * spacing[a]);
        let mut array = PotentialArray::new(dims[0], dims[1], dims[2]);
        [array.dx_mm, array.dy_mm, array.dz_mm] = spacing;
        let mut indices = vec![-1; metadata.electrode_indices.len()];
        for i in 0..array.potentials.len() {
            let p = point(i, dims, origin, spacing);
            for electrode in electrodes {
                if electrode.shape.distance(p) <= 0.0 {
                    let number = electrode.voltage as usize;
                    array.electrodes[i] = true;
                    if indices[number] == -1 {
                        indices[number] = i as i64;
                    }
                }
            }
        }
        let mut file = PaFile::new(array);
        file.ng = pa.ng;
        patch_files.push((file, indices));
        manifest.patches.push(AdaptivePatchManifest {
            level: patch.factor.ilog2(),
            factor: patch.factor,
            bounds_mm: [
                patch.cuts.origin,
                std::array::from_fn(|a| {
                    patch.cuts.origin[a] + (patch.cuts.grid.dims[a] - 1) as f64 * spacing[a]
                }),
            ],
            origin_mm: origin,
            spacing_mm: spacing,
            file: args
                .out
                .with_extension(format!("patch{slot}.pa0"))
                .file_name()
                .unwrap()
                .to_str()
                .ok_or("output name must be UTF-8")?
                .into(),
            ghost_layers: 2,
        });
    }
    let mut combined = vec![0.0; pa.array.potentials.len()];
    let mut patch_combined: Vec<_> = patch_files
        .iter()
        .map(|(p, _)| vec![0.0; p.array.potentials.len()])
        .collect();
    let mut compute = Duration::ZERO;
    let mut prep = timing.0;
    let mut cycles = 0;
    let mut residual = 0.0_f64;
    let mut unknowns = 0;
    let mut matrix_bytes = 0;
    for (&number, &voltage) in voltages {
        let grid = adaptive.with_voltages(|n| if n == number as f64 { REFERENCE } else { 0.0 })?;
        let solution = grid.solve(Options {
            relative_tolerance: args.tolerance,
            max_cycles: args.max_cycles,
            ..Default::default()
        })?;
        let report = &solution.report;
        if !report.converged {
            return Err(format!(
                "adaptive electrode {number} did not converge after {} iterations",
                report.iterations
            )
            .into());
        }
        compute += report.timings.solve;
        prep += report.timings.setup;
        cycles += report.iterations;
        residual = residual.max(report.residual_history.last().unwrap().relative);
        unknowns = solution.unknowns;
        matrix_bytes = solution.matrix_bytes;
        pa.array.potentials = solution.base;
        for (v, basis) in combined.iter_mut().zip(&pa.array.potentials) {
            *v += basis * voltage / REFERENCE;
        }
        let path = args.out.with_extension(format!("pa{number}"));
        write_pa(&path, &pa)?;
        paths.push(path);
        for (slot, ((file, _), values)) in
            patch_files.iter_mut().zip(&mut patch_combined).enumerate()
        {
            let description = &manifest.patches[slot];
            let patch = &adaptive.patches[slot];
            let dims = [file.array.nx, file.array.ny, file.array.nz];
            for (i, v) in file.array.potentials.iter_mut().enumerate() {
                let p = point(i, dims, description.origin_mm, description.spacing_mm);
                *v = if (0..3).all(|a| {
                    p[a] >= description.bounds_mm[0][a] - 1e-10
                        && p[a] <= description.bounds_mm[1][a] + 1e-10
                }) {
                    interpolate(
                        &solution.patches[slot],
                        patch.cuts.grid,
                        patch.cuts.origin,
                        p,
                    )
                } else {
                    interpolate(
                        &pa.array.potentials,
                        adaptive.base.grid,
                        adaptive.base.origin,
                        p,
                    )
                };
                values[i] += *v * voltage / REFERENCE;
            }
            let path = args.out.with_extension(format!("patch{slot}.pa{number}"));
            write_pa(&path, file)?;
            paths.push(path);
        }
    }
    pa.array.potentials = combined;
    pa.pa0 = Some(metadata);
    let path = args.out.with_extension("pa0");
    write_pa(&path, &pa)?;
    paths.push(path);
    for (slot, ((mut file, mut indices), values)) in
        patch_files.into_iter().zip(patch_combined).enumerate()
    {
        file.array.potentials = values;
        // SIMION's ordinary PA0 trailer requires at least one local conductor.
        // Cropped vacuum-only patches use the manifest voltage table instead.
        if let Some(last) = indices.iter().rposition(|&i| i != -1) {
            indices.truncate((last + 1).max(32));
            file.pa0 = Some(Pa0Metadata {
                reference_voltage: REFERENCE,
                electrode_indices: indices,
                scalable: None,
            });
        }
        let path = args.out.with_extension(format!("patch{slot}.pa0"));
        write_pa(&path, &file)?;
        paths.push(path);
    }
    let path = args.out.with_extension("adaptive.json");
    let mut writer = BufWriter::new(File::create(&path)?);
    serde_json::to_writer_pretty(&mut writer, &manifest)?;
    writer.flush()?;
    paths.push(path);
    println!(
        "adaptive: unknowns={unknowns}, matrix_bytes={matrix_bytes}, compute_ms={:.6}",
        compute.as_secs_f64() * 1000.0
    );
    crate::json::set_summary(serde_json::json!({
        "requested": {"device":"cpu","backend":null,"precision":"f64"},
        "executed": {"device":"cpu","backend":null,"precision":"f64"},
        "reason":"adaptive variational Cartesian CPU", "fallback_reason":null,
        "cycles":cycles,"final_residual":residual,"tolerance":args.tolerance,"converged":true,
        "compute_ms":compute.as_secs_f64()*1000.0,"prep_ms":prep.as_secs_f64()*1000.0,
        "end_to_end_ms":timing.1.elapsed().as_secs_f64()*1000.0,
        "unknowns":unknowns,"matrix_bytes":matrix_bytes,"output_paths":paths
    }));
    Ok(())
}

#[cfg(feature = "cad")]
struct FlaggedPart {
    part: simrep_cad::CadPart,
    failed: Arc<AtomicBool>,
}

#[cfg(feature = "cad")]
impl SignedDistance for FlaggedPart {
    fn distance(&self, point: [f64; 3]) -> f64 {
        let distance = self.part.distance(point);
        if !distance.is_finite() {
            self.failed.store(true, Ordering::Relaxed);
        }
        distance
    }

    fn segment_distance_bounds(&self, from: [f64; 3], to: [f64; 3]) -> Option<[f64; 2]> {
        self.part.segment_distance_bounds(from, to)
    }

    fn segment_single_interval(&self, from: [f64; 3], to: [f64; 3], level: f64) -> bool {
        self.part.segment_single_interval(from, to, level)
    }

    fn segment_clear(&self, from: [f64; 3], to: [f64; 3], tolerance: f64) -> bool {
        self.part.segment_clear(from, to, tolerance)
    }

    fn conductor_mask(
        &self,
        dims: [usize; 3],
        origin: [f64; 3],
        spacing: [f64; 3],
        threshold: f64,
        mask: &mut [bool],
    ) -> Result<(), &'static str> {
        self.part
            .conductor_mask(dims, origin, spacing, threshold, mask)
            .inspect_err(|_| self.failed.store(true, Ordering::Relaxed))
    }

    fn mark_near_nodes(
        &self,
        dims: [usize; 3],
        origin: [f64; 3],
        spacing: [f64; 3],
        band: f64,
        near: &mut [bool],
    ) {
        self.part.mark_near_nodes(dims, origin, spacing, band, near);
    }

    fn sample_grid(
        &self,
        dims: [usize; 3],
        origin: [f64; 3],
        spacing: [f64; 3],
        values: &mut [f64],
    ) {
        values.par_iter_mut().enumerate().for_each(|(i, value)| {
            let c = [i % dims[0], i / dims[0] % dims[1], i / (dims[0] * dims[1])];
            *value = self.distance(std::array::from_fn(|a| {
                origin[a] + c[a] as f64 * spacing[a]
            }));
        });
    }
}

#[cfg(feature = "cad")]
fn cad_sources(
    args: &GeometryOptions,
    failed: Arc<AtomicBool>,
) -> GpuResult<(PaFile, Vec<Source>)> {
    let dims = args.grid.ok_or("CAD input requires --grid nx,ny,nz")?;
    let spacing = args.spacing.unwrap_or([1.0; 3]);
    validate_grid(dims, args.origin, spacing)?;
    if dims[2] < 2 {
        return Err("CAD input requires a planar 3D grid (nz >= 2)".into());
    }
    let h_min = spacing.into_iter().fold(f64::INFINITY, f64::min);
    let options = simrep_cad::ImportOptions {
        mode: match args.cad_mode {
            CadMode::Auto => simrep_cad::Mode::Auto,
            CadMode::Exact => simrep_cad::Mode::Exact,
            CadMode::Tessellated => simrep_cad::Mode::Tessellated,
        },
        band_mm: 2.0 * spacing.into_iter().fold(0.0, f64::max),
        chordal_tolerance_mm: args.cad_tolerance.unwrap_or(h_min / 100.0),
        units: args.units,
    };
    let sources = simrep_cad::read_cad(&args.input, options)?
        .into_iter()
        .enumerate()
        .map(|(i, part)| {
            println!(
                "CAD part {i} {:?}: {:?}, chordal_tolerance_mm={}, triangles={}",
                part.name, part.mode, options.chordal_tolerance_mm, part.triangle_count
            );
            let name = part.name.clone();
            let source =
                SourceGeometry::mesh(&part.tessellation, Some(options.chordal_tolerance_mm))?;
            let shape = FlaggedPart {
                part,
                failed: Arc::clone(&failed),
            };
            Ok((
                name,
                Arc::new(shape) as Arc<dyn SignedDistance>,
                source,
                GapSource::Cad(i),
            ))
        })
        .collect::<GpuResult<Vec<_>>>()?;
    let mut array = PotentialArray::new(dims[0], dims[1], dims[2]);
    [array.dx_mm, array.dy_mm, array.dz_mm] = spacing;
    Ok((PaFile::new(array), sources))
}

#[cfg(not(feature = "cad"))]
fn cad_sources(_: &GeometryOptions, _: Arc<AtomicBool>) -> GpuResult<(PaFile, Vec<Source>)> {
    Err("STEP/IGES import requires cargo build -p simrep-cli --features cad; see docs/step-import.md"
        .into())
}

fn validate_cad_queries(failed: &AtomicBool) -> GpuResult<()> {
    if failed.load(Ordering::Relaxed) {
        return Err("SDF returned a non-finite distance (CAD); use --cad-mode tessellated".into());
    }
    Ok(())
}

fn validate_grid(dims: [usize; 3], origin: [f64; 3], spacing: [f64; 3]) -> GpuResult<()> {
    if dims[0] < 2
        || dims[1] < 2
        || dims[2] == 0
        || dims
            .iter()
            .try_fold(1usize, |n, d| n.checked_mul(*d))
            .is_none_or(|n| n > u32::MAX as usize)
        || spacing.iter().any(|h| !h.is_finite() || *h <= 0.0)
        || origin.iter().any(|v| !v.is_finite())
    {
        return Err("invalid grid dimensions, spacing, or origin".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parallel_arrays_preserve_json_values() {
        for count in [0, 1, 16_385, 600_000] {
            let values: Vec<_> = (0..count)
                .map(|i| {
                    if i % 7 == 0 {
                        None
                    } else {
                        Some(i as f64 * 0.125)
                    }
                })
                .collect();
            let mut actual = Vec::new();
            write_json_array(&mut actual, &values).unwrap();
            assert_eq!(actual, serde_json::to_vec(&values).unwrap());
        }
    }

    #[test]
    fn sparse_cut_rows_expand_to_unchanged_dense_arrays() {
        for count in [0usize, 1, 16_385, 300_001] {
            let mut indices = vec![0, 16_383, 16_384, count.saturating_sub(1)];
            indices.retain(|&i| i < count);
            indices.sort_unstable();
            indices.dedup();
            let cuts: Vec<_> = indices
                .iter()
                .map(|&node| PhysicalCuts {
                    node,
                    fractions: [Some(0.0), None, Some(0.25), None, None, None],
                    electrodes: [Some(3), None, Some(7), None, None, None],
                })
                .collect();
            let sidecar = Sidecar {
                version: 3,
                dims: [count, 1, 1],
                origin_mm: [0.0; 3],
                spacing_mm: [1.0; 3],
                symmetry: "planar",
                mirrors: [false; 3],
                electrode_numbers: vec![3, 7],
                sampled_sdfs: Vec::new(),
                source_geometry: Vec::new(),
                assignments: Vec::new(),
                node_electrodes: vec![None; count],
                solver_fixed: vec![false; count],
                cuts,
                surface_tolerance_mm: 1e-9,
                snap_distance_mm: 0.01,
            };
            let mut fractions = vec![[None; 6]; count];
            let mut electrodes = vec![[None; 6]; count];
            for cut in &sidecar.cuts {
                fractions[cut.node] = cut.fractions;
                electrodes[cut.node] = cut.electrodes;
            }
            for (electrode_array, expected) in [
                (false, serde_json::to_vec(&fractions).unwrap()),
                (true, serde_json::to_vec(&electrodes).unwrap()),
            ] {
                let mut actual = Vec::new();
                sidecar.write_cuts(&mut actual, electrode_array).unwrap();
                assert_eq!(actual, expected);
            }
        }
    }
}

#[path = "sdf_auto.rs"]
pub(crate) mod sdf_auto;

use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::{self, BufReader, BufWriter, Write};
use std::path::PathBuf;
use std::time::Instant;

use crate::flight_shards::{self, Shard};
use clap::{Args, ValueEnum};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use simrep_fly::{
    ELECTRIC_ACCELERATION, Field, FlightOptions, GridField, InstanceField, Particle,
    Reconstruction, SPEED_OF_LIGHT, SplatReason, VoltageExpression, fly_many,
    gpu::{DeviceType, Gpu},
    physics::{
        BeamOptions, BeamTermination, ChargeMode, DirectCharge, EnsembleOptions, GpuPic, PicMesh,
        SpaceCharge,
        collision::{CollisionModel, Hs1, fly_collisional, fly_many_collisional},
        fly_beam, fly_ensemble,
        magnetic::{
            Electromagnetic, ImportedMagneticMap, MapPlacement, OutsideMap, PreparedMagneticField,
            VectorPotential,
        },
    },
    surface::{SurfaceAccuracy, SurfaceModel},
};
use simrep_io::{Fly2Context, Fly2Field, Fly2Instance, read_fast_adjust, read_particles_in};
use simrep_solve::gpu::GpuResult;
use simrep_solve::magnetostatics::{FieldFile as MagneticFieldFile, Formulation};
use simrep_solve::mg::{Geometry, Grid, Options as PoissonOptions};

const GPU_COMPUTE_THRESHOLD: usize = 4096;
const GPU_PIC_THRESHOLD: usize = 100_000;
const HS1_GPU_COMPUTE_THRESHOLD: usize = 10_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum, Serialize)]
enum FlyDevice {
    Auto,
    Cpu,
    Gpu,
}
impl FlyDevice {
    fn requests_gpu(self, ions: usize, crossover: Option<usize>) -> bool {
        self == Self::Gpu || self == Self::Auto && crossover.is_some_and(|n| ions >= n)
    }
}
#[derive(Clone, Copy, clap::ValueEnum, Serialize)]
enum SpaceChargeMethod {
    Direct,
    Pic,
}
#[derive(Clone, Copy, clap::ValueEnum, Serialize)]
enum ChargeModeName {
    Factor,
    Coulombic,
    Beam,
}

#[derive(Args, Serialize)]
pub struct FlyOptions {
    /// Auto uses the measured compute crossover; CPU for small beams.
    #[arg(long, value_enum, default_value = "auto")]
    device: FlyDevice,
    /// Native GPU backend (unused on CPU).
    #[arg(long, value_enum, default_value = "vulkan")]
    backend: crate::Backend,
    /// Zero-based adapter index or an unambiguous case-insensitive name substring.
    #[arg(long)]
    gpu_adapter: Option<String>,
    /// Interleaved partition i/N (1 <= i <= N); preserves global ion indices.
    #[arg(long)]
    shard: Option<Shard>,
    /// SIMION PA or PA0; sibling PA1..PAN supply fast-adjust voltage changes.
    #[arg(long)]
    pa: Option<PathBuf>,
    /// Continuous electrode sidecar; defaults to the PA's sibling .cuts.json.
    #[arg(long, requires = "pa")]
    cuts: Option<PathBuf>,
    /// Disable sibling .cuts.json pickup and use staircase collisions.
    #[arg(long)]
    no_cuts: bool,
    /// Add exact collisions to the staircase field.
    #[arg(long, value_parser = ["insulators", "all"], requires = "pa")]
    surface_collisions: Option<String>,
    /// Collision surfaces: exact (default), sampled sdf, or opt-in measured auto.
    #[arg(long, default_value = "exact", value_parser = ["exact", "sdf", "auto"])]
    surface_backend: String,
    #[arg(long, hide = true, default_value_t = 2)]
    surface_sdf_resolution_divisor: u32,
    /// SIMION ION CSV or sandboxed, context-aware FLY2 definitions.
    #[arg(long)]
    particles: PathBuf,
    /// Electrode=volts overrides, for example --voltage 1=0,2=110,3=0.
    #[arg(long, value_delimiter = ',', value_parser = voltage)]
    voltage: Vec<(usize, f64)>,
    /// Per-electrode voltage expression at absolute time t (microseconds).
    #[arg(long, value_parser = expression_assignment)]
    voltage_expr: Vec<(usize, String)>,
    /// Named expression parameter, for example --param frequency=0.14 (cycles/us).
    #[arg(long, value_parser = parameter)]
    param: Vec<(String, f64)>,
    /// JSON voltage run settings: {"parameters":{...},"voltage_expr":{"2":"..."}}.
    #[arg(long)]
    run_file: Option<PathBuf>,
    /// Workbench position in mm of the unmirrored PA origin.
    #[arg(long, default_value = "0,0,0", value_parser = numbers::<3>, allow_hyphen_values = true)]
    origin: [f64; 3],
    /// PA unit X,Y,Z directions in workbench coordinates, column by column.
    #[arg(long, default_value = "1,0,0,0,1,0,0,0,1", value_parser = numbers::<9>, allow_hyphen_values = true)]
    axes: [f64; 9],
    /// Positive geometric scale of the PA instance.
    #[arg(long, default_value_t = 1.0)]
    scale: f64,
    /// xmin,ymin,zmin,xmax,ymax,zmax in mm; permits field-free drift outside PA.
    #[arg(long, value_parser = numbers::<6>, allow_hyphen_values = true)]
    bounds: Option<[f64; 6]>,
    /// Detector nx,ny,nz,d: nx*x+ny*y+nz*z=d, in workbench mm.
    #[arg(long, value_parser = numbers::<4>, allow_hyphen_values = true)]
    plane: Option<[f64; 4]>,
    /// CSV output path; omitted writes to stdout (stderr with --json).
    #[arg(long)]
    output: Option<PathBuf>,
    /// Export sampled trajectories as CSV with global ion indices.
    #[arg(long, requires = "output")]
    trajectories: Option<PathBuf>,
    /// Retain every Nth accepted trajectory step (initial/final states included).
    #[arg(long, default_value_t = 1)]
    sample_every: usize,
    /// GPU trajectory capacity per ion; overflow is an error.
    #[arg(long, default_value_t = 10000)]
    max_samples_per_ion: usize,
    /// Override automatic field-aware absolute (mm, mm/us) and relative tolerances.
    #[arg(long)]
    tolerance: Option<f64>,
    /// Override the automatic geometry/physics step ceiling (microseconds).
    #[arg(long)]
    max_step_us: Option<f64>,
    /// Maximum flight duration in microseconds.
    #[arg(long, default_value_t = 100.0)]
    duration_us: f64,
    /// Disable relativistic dynamics (SIMION enables them by default).
    #[arg(long)]
    nonrelativistic: bool,
    /// Use the original global cubic spline, including metal nodes.
    #[arg(long)]
    global_cubic: bool,
    /// Historical nearest-node electrode voxels instead of occupied PA cells.
    #[arg(long)]
    nearest_node_collisions: bool,
    /// Disable calibrated CPU coil maps and sum exact Biot-Savart filaments.
    #[arg(long)]
    exact_coils: bool,
    /// External regular-grid vector B exports; repeated maps superpose.
    #[arg(long)]
    bfield_map: Vec<PathBuf>,
    /// Input magnetic-map format.
    #[arg(long, default_value = "auto", value_parser = ["auto", "comsol", "opera", "csv", "txt", "axisymmetric"])]
    bfield_map_format: String,
    /// Coordinate,flux units: mm/G (default), m/T, cm/mT or inch/G.
    #[arg(long, value_parser = map_units)]
    #[serde(serialize_with = "flight_shards::debug_units")]
    bfield_map_units: Option<(simrep_io::bfield::LengthUnit, simrep_io::bfield::FluxUnit)>,
    /// origin xyz(mm), rotation columns(9), positive geometric scale.
    #[arg(long, value_parser = numbers::<13>, allow_hyphen_values = true)]
    bfield_map_transform: Option<[f64; 13]>,
    /// Zero-based source columns x,y,z,Bx,By,Bz (or z,r,Bz,Br).
    #[arg(long, value_delimiter = ',')]
    bfield_map_columns: Option<Vec<usize>>,
    /// Column delimiter for text magnetic maps (auto-detected if omitted).
    #[arg(long)]
    bfield_map_delimiter: Option<char>,
    /// Behavior outside imported magnetic maps.
    #[arg(long, default_value = "error", value_parser = ["error", "zero", "clamp"])]
    bfield_map_outside: String,
    /// Axial-vector mirror planes through source grid minima; x,y,z or axial z.
    #[arg(long, value_delimiter = ',', value_parser = ["x", "y", "z"])]
    bfield_map_mirror: Vec<String>,
    /// Report finite-difference nodal div B; diagnostic, not an accuracy bound.
    #[arg(long)]
    bfield_map_divergence: bool,
    /// CPU curl-A field sidecar (.mag.json), additive with magnetic PAs/coils.
    #[arg(long)]
    magnetic_vector_potential: Vec<PathBuf>,
    #[command(flatten)]
    physics: crate::physics_cmd::PhysicsOptions,
    /// Coupled equal-time ensemble; beam instead uses leader-normal planes.
    #[arg(long, value_enum)]
    space_charge: Option<SpaceChargeMethod>,
    /// Interpret charge weights as factors, total coulombs, or steady beam current.
    #[arg(long, value_enum, default_value = "factor",
        requires_ifs([("factor", "space_charge"), ("coulombic", "space_charge")]))]
    charge_mode: ChargeModeName,
    /// Factor multiplier or total absolute represented charge in coulombs.
    #[arg(long, default_value_t = 1.0)]
    charge_amount: f64,
    /// Total absolute steady line current; requires --charge-mode beam.
    #[arg(long)]
    beam_current_a: Option<f64>,
    /// Direct Plummer or beam line-cloud radius in mm (zero = singular sources).
    #[arg(long, default_value_t = 0.0)]
    charge_softening_mm: f64,
    /// Full Cartesian PIC mesh dimensions; requires explicit --bounds.
    #[arg(long, value_delimiter = ',', default_values_t = [33usize,33,33])]
    pic_dims: Vec<usize>,
    /// Fraction of the local space-charge timescale used for a shared step.
    #[arg(long)]
    space_charge_step_fraction: Option<f64>,
}
impl FlyOptions {
    fn compute_crossover(
        &self,
        has_magnetic: bool,
        collision: Option<&CollisionModel>,
    ) -> Option<usize> {
        if has_magnetic {
            return None;
        }
        let default = Hs1::default();
        match collision {
            None => Some(GPU_COMPUTE_THRESHOLD),
            Some(CollisionModel::Hs1(hs))
                if self.pa.is_none()
                    && self.bounds.is_none()
                    && self.plane.is_none()
                    && self.space_charge.is_none()
                    && self.duration_us == 50.0
                    && self.max_step_us == Some(0.05)
                    && self.tolerance.is_none()
                    && matches!(
                        self.physics.integrator,
                        crate::physics_cmd::Integration::Rk45
                    )
                    && self.physics.step_us == FlightOptions::default().initial_step_us
                    && hs.gas.pressure_pa == default.gas.pressure_pa
                    && hs.gas.temperature_k == default.gas.temperature_k
                    && hs.gas.mass_amu == default.gas.mass_amu
                    && hs.gas.velocity_mm_per_us == default.gas.velocity_mm_per_us
                    && hs.cross_section_m2 == default.cross_section_m2
                    && hs.mean_free_path_mm == default.mean_free_path_mm
                    && hs.steps_per_mean_free_time == default.steps_per_mean_free_time =>
            {
                Some(HS1_GPU_COMPUTE_THRESHOLD)
            }
            _ => None,
        }
    }
}
pub(crate) fn numbers<const N: usize>(text: &str) -> Result<[f64; N], String> {
    let values = text
        .split(',')
        .map(|s| {
            s.trim()
                .parse::<f64>()
                .map_err(|_| "expected comma-separated numbers".to_owned())
        })
        .collect::<Result<Vec<_>, _>>()?;
    if values.iter().any(|v| !v.is_finite()) {
        return Err("coordinates must be finite".into());
    }
    values
        .try_into()
        .map_err(|_| format!("expected {N} comma-separated numbers"))
}
fn map_units(
    text: &str,
) -> Result<(simrep_io::bfield::LengthUnit, simrep_io::bfield::FluxUnit), String> {
    use simrep_io::bfield::{FluxUnit as B, LengthUnit as L};
    let (length, flux) = text.split_once('/').ok_or("expected coordinate/flux units: length mm, m, cm, inch or in / flux G, gauss, T, tesla or mT (for example mm/G)")?;
    let length = match length.to_ascii_lowercase().as_str() {
        "mm" => L::Mm,
        "m" => L::M,
        "cm" => L::Cm,
        "inch" | "in" => L::Inch,
        _ => return Err("length unit must be mm, m, cm, inch or in".into()),
    };
    let flux = match flux.to_ascii_lowercase().as_str() {
        "g" | "gauss" => B::Gauss,
        "t" | "tesla" => B::Tesla,
        "mt" => B::Millitesla,
        _ => return Err("flux unit must be G, gauss, T, tesla or mT".into()),
    };
    Ok((length, flux))
}
fn load_magnetic_maps(options: &FlyOptions) -> GpuResult<Vec<ImportedMagneticMap>> {
    use simrep_io::bfield::{self, Format, ReadOptions};
    let format = match options.bfield_map_format.as_str() {
        "comsol" => Format::Comsol,
        "opera" => Format::Opera,
        "axisymmetric" => Format::Axisymmetric,
        "csv" | "txt" => Format::Delimited,
        _ => Format::Auto,
    };
    let read_options = ReadOptions {
        format,
        length_unit: options.bfield_map_units.map(|u| u.0),
        flux_unit: options.bfield_map_units.map(|u| u.1),
        columns: options.bfield_map_columns.clone(),
        delimiter: options.bfield_map_delimiter,
    };
    let mut maps = Vec::new();
    for path in &options.bfield_map {
        let data = bfield::read(path, &read_options)?;
        let mut placement = MapPlacement::default();
        let rotation = options.bfield_map_transform.map_or(placement.axes, |t| {
            std::array::from_fn(|a| std::array::from_fn(|b| t[3 + 3 * a + b]))
        });
        let frame = if data.axisymmetric {
            [[0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]
        } else {
            placement.axes
        };
        placement.axes = std::array::from_fn(|a| {
            std::array::from_fn(|b| (0..3).map(|c| rotation[c][b] * frame[a][c]).sum())
        });
        let translation = options
            .bfield_map_transform
            .map_or([0.0; 3], |t| [t[0], t[1], t[2]]);
        placement.scale = options.bfield_map_transform.map_or(1.0, |t| t[12]);
        placement.origin_mm = std::array::from_fn(|a| {
            translation[a]
                + placement.scale
                    * (0..3)
                        .map(|c| placement.axes[c][a] * data.origin_mm[c])
                        .sum::<f64>()
        });
        placement.outside = match options.bfield_map_outside.as_str() {
            "zero" => OutsideMap::Zero,
            "clamp" => OutsideMap::Clamp,
            _ => OutsideMap::Error,
        };
        for mirror in &options.bfield_map_mirror {
            let axis = match mirror.as_str() {
                "x" => 0,
                "y" => 1,
                _ => 2,
            };
            if data.axisymmetric {
                if axis != 2 {
                    return Err("axisymmetric map only supports axial z mirror".into());
                }
                placement.mirrors[0] = Some([1, -1, 1]);
            } else {
                placement.mirrors[axis] =
                    Some(std::array::from_fn(|c| if c == axis { 1 } else { -1 }));
            }
        }
        let map = ImportedMagneticMap::from_nodes(
            data.dims,
            data.spacing_mm,
            data.values_gauss,
            data.axisymmetric,
            placement,
        )?;
        if options.bfield_map_divergence {
            let report = map.divergence_report();
            eprintln!(
                "BFIELD_MAP_DIVERGENCE,file={},nodes={},max_abs_gauss_per_mm={:.9e},rms_gauss_per_mm={:.9e},normalized_max={:.9e}",
                path.display(),
                report.nodes,
                report.max_abs_gauss_per_mm,
                report.rms_gauss_per_mm,
                report.normalized_max
            );
            if report.normalized_max > 0.01 {
                eprintln!(
                    "warning: {} has normalized nodal div B > 0.01; inspect export units, noise and resolution",
                    path.display()
                );
            }
        }
        eprintln!(
            "BFIELD_MAP,file={},dims={:?},spacing_mm={:?},units={:?}/{:?},outside={}{}",
            path.display(),
            data.dims,
            data.spacing_mm,
            data.length_unit,
            data.flux_unit,
            options.bfield_map_outside,
            if options.bfield_map_outside == "error" {
                " (exit stops with OutsideGrid)"
            } else {
                ""
            }
        );
        maps.push(map);
    }
    Ok(maps)
}
fn voltage(text: &str) -> Result<(usize, f64), String> {
    let (number, volts) = text.split_once('=').ok_or("expected electrode=volts")?;
    let number = number
        .parse()
        .map_err(|_| "expected electrode=volts with a nonnegative integer electrode number")?;
    let volts: f64 = volts
        .parse()
        .map_err(|_| "expected electrode=volts with a finite voltage")?;
    if !volts.is_finite() {
        return Err("electrode voltage must be finite".into());
    }
    Ok((number, volts))
}
fn expression_assignment(text: &str) -> Result<(usize, String), String> {
    let (n, expression) = text
        .split_once('=')
        .ok_or("expected electrode=expression")?;
    let n = n.parse().map_err(|_| "invalid electrode number")?;
    if expression.trim().is_empty() {
        return Err("voltage expression is empty".into());
    }
    Ok((n, expression.to_owned()))
}
fn parameter(text: &str) -> Result<(String, f64), String> {
    let (name, value) = text.split_once('=').ok_or("expected name=value")?;
    let value: f64 = value.parse().map_err(|_| "invalid parameter value")?;
    if !value.is_finite() {
        return Err("parameter must be finite".into());
    }
    Ok((name.to_owned(), value))
}
#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct VoltageRun {
    parameters: BTreeMap<String, f64>,
    voltage_expr: BTreeMap<usize, String>,
    physics: crate::physics_cmd::PhysicsRun,
}

// Adapter names/selectors remain portable; the resolved numerical execution
// path (including software vs hardware class) does not.
fn execution_identity(
    precision: &str,
    adapter: Option<(wgpu::Backend, wgpu::DeviceType)>,
) -> serde_json::Value {
    match adapter {
        Some((backend, class)) => serde_json::json!({
            "device": "gpu", "precision": precision,
            "backend": format!("{backend:?}").to_lowercase(),
            "adapter_class": format!("{class:?}").to_lowercase(),
        }),
        None => serde_json::json!({
            "device": "cpu", "precision": precision, "backend": "cpu", "adapter_class": "cpu",
        }),
    }
}

pub fn run(mut options: FlyOptions) -> GpuResult<()> {
    let start = Instant::now();
    if options.shard.is_some()
        && (options.space_charge.is_some()
            || matches!(options.charge_mode, ChargeModeName::Beam)
            || options.beam_current_a.is_some())
    {
        return Err("--shard cannot be used with coupled physics: direct Coulomb, PIC space charge, or beam/leader mode".into());
    }
    if options.gpu_adapter.is_some() && options.device != FlyDevice::Gpu {
        return Err("--gpu-adapter requires --device gpu".into());
    }
    if options.sample_every == 0 {
        return Err("--sample-every must be positive".into());
    }
    if options
        .trajectories
        .as_ref()
        .is_some_and(|p| Some(p) == options.output.as_ref())
    {
        return Err("--trajectories must differ from --output".into());
    }
    let identity = serde_json::to_value(&options)?;
    let adaptive_manifest = if let Some(path) = options.pa.as_ref().filter(|p| {
        p.extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("json"))
    }) {
        let manifest: simrep_fly::adaptive::AdaptiveManifest =
            serde_json::from_reader(BufReader::new(File::open(path)?))?;
        if manifest.schema_version != 1
            || !manifest.reference_voltage.is_finite()
            || manifest.reference_voltage <= 0.0
            || manifest.electrode_voltages.values().any(|v| !v.is_finite())
        {
            return Err("unsupported or invalid adaptive manifest".into());
        }
        let parent = path
            .parent()
            .unwrap_or(std::path::Path::new("."))
            .to_path_buf();
        options.pa = Some(parent.join(&manifest.base));
        Some((manifest, parent))
    } else {
        None
    };
    if adaptive_manifest.is_some()
        && options.device == FlyDevice::Gpu
        && options.surface_backend != "auto"
    {
        return Err("adaptive fields require --device cpu or auto".into());
    }
    let mut run: VoltageRun = match &options.run_file {
        Some(path) => serde_json::from_reader(BufReader::new(File::open(path)?))?,
        None => VoltageRun::default(),
    };
    let mut parameters = BTreeSet::new();
    for (name, value) in &options.param {
        if !parameters.insert(name) {
            return Err(
                format!("duplicate --param key {name:?}; supply each parameter once").into(),
            );
        }
        if let Some(previous) = run.parameters.insert(name.clone(), *value)
            && previous != *value
        {
            eprintln!("parameter {name}: {previous} -> {value} (--param overrides run-file value)");
        }
    }
    let mut expression_numbers = BTreeSet::new();
    for (number, expression) in &options.voltage_expr {
        if !expression_numbers.insert(number) {
            return Err(format!(
                "duplicate --voltage-expr electrode {number}; supply each electrode once"
            )
            .into());
        }
        if let Some(previous) = run.voltage_expr.insert(*number, expression.clone())
            && previous != *expression
        {
            eprintln!(
                "electrode {number} expression: {previous:?} -> {expression:?} (--voltage-expr overrides run-file value)"
            );
        }
    }
    let mut voltage_numbers = BTreeSet::new();
    for &(number, _) in &options.voltage {
        if !voltage_numbers.insert(number) {
            return Err(format!(
                "duplicate --voltage electrode {number}; supply each electrode once"
            )
            .into());
        }
    }
    let expressions = run
        .voltage_expr
        .into_iter()
        .map(|(n, source)| VoltageExpression::parse(&source, &run.parameters).map(|e| (n, e)))
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    if adaptive_manifest.is_some() && !expressions.is_empty() {
        return Err("adaptive fields do not yet support voltage expressions".into());
    }
    if options.pa.is_none() && (!options.voltage.is_empty() || !expressions.is_empty()) {
        return Err("electrode voltages require --pa".into());
    }
    let read_electric = |adjustments: &[(usize, f64)]| match &options.pa {
        Some(path) if adjustments.is_empty() => simrep_io::read_pa(path).map(|file| file.array),
        Some(path) => read_fast_adjust(path, adjustments),
        None => Ok(simrep_core::PotentialArray::new(1, 1, 1)),
    };
    let adaptive_field = |base: &GridField,
                          adjustments: &[(usize, f64)]|
     -> GpuResult<Option<simrep_fly::AdaptiveField>> {
        let Some((manifest, parent)) = &adaptive_manifest else {
            return Ok(None);
        };
        let arrays = manifest
            .patches
            .iter()
            .map(|patch| -> GpuResult<_> {
                let path = parent.join(&patch.file);
                let mut array = simrep_io::read_pa(&path)?.array;
                for &(number, voltage) in adjustments {
                    let current = manifest
                        .electrode_voltages
                        .get(&number)
                        .ok_or("adaptive voltage has no basis")?;
                    let basis =
                        simrep_io::read_pa(path.with_extension(format!("pa{number}")))?.array;
                    if [basis.nx, basis.ny, basis.nz] != [array.nx, array.ny, array.nz]
                        || basis.electrodes != array.electrodes
                        || [basis.dx_mm, basis.dy_mm, basis.dz_mm]
                            != [array.dx_mm, array.dy_mm, array.dz_mm]
                    {
                        return Err("adaptive basis geometry differs from patch PA0".into());
                    }
                    let delta = (voltage - current) / manifest.reference_voltage;
                    for (v, b) in array.potentials.iter_mut().zip(basis.potentials) {
                        *v += delta * b;
                    }
                }
                Ok(array)
            })
            .collect::<GpuResult<Vec<_>>>()?;
        Ok(Some(simrep_fly::AdaptiveField::new(
            base.clone(),
            manifest,
            arrays,
        )?))
    };
    let beam_mode = matches!(options.charge_mode, ChargeModeName::Beam);
    if beam_mode && (options.beam_current_a.is_none() || options.space_charge.is_some()) {
        return Err(
            "beam needs --beam-current-a and cannot be combined with --space-charge".into(),
        );
    }
    if !beam_mode && options.beam_current_a.is_some() {
        return Err("--beam-current-a requires --charge-mode beam".into());
    }
    if beam_mode && options.device == FlyDevice::Gpu && options.surface_backend != "auto" {
        return Err("leader-plane beam integration requires --device cpu or auto".into());
    }
    if options.device == FlyDevice::Gpu
        && options.surface_backend != "auto"
        && matches!(options.space_charge, Some(SpaceChargeMethod::Direct))
    {
        return Err("direct Coulomb physics requires --device cpu or auto".into());
    }
    if options.surface_collisions.is_some() && !options.no_cuts {
        return Err("--surface-collisions requires --no-cuts".into());
    }
    if options.cuts.is_some() && options.no_cuts && options.surface_collisions.is_none() {
        return Err("--cuts conflicts with --no-cuts".into());
    }
    if options.surface_collisions.is_some()
        && options.device == FlyDevice::Gpu
        && options.surface_backend == "exact"
    {
        return Err("--surface-collisions requires --device cpu or auto".into());
    }
    let cuts_path = options.cuts.clone().or_else(|| {
        options
            .pa
            .as_ref()
            .map(|path| path.with_extension("cuts.json"))
            .filter(|path| {
                (!options.no_cuts || options.surface_collisions.is_some()) && path.is_file()
            })
    });
    if options.surface_collisions.is_some() && cuts_path.is_none() {
        return Err("--surface-collisions requires a --cuts or sibling sidecar".into());
    }
    let cuts_source = if options.no_cuts && options.surface_collisions.is_none() {
        "disabled"
    } else if options.cuts.is_some() {
        "explicit"
    } else if cuts_path.is_some() {
        "sibling"
    } else {
        "none"
    };
    if cuts_source == "none" {
        let pa = options
            .pa
            .as_deref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "<pa0>".to_owned());
        let sidecar = options
            .pa
            .as_ref()
            .map(|p| p.with_extension("cuts.json").display().to_string())
            .unwrap_or_else(|| "<stem>.cuts.json".to_owned());
        eprintln!(
            "warning: no exact-surface sidecar {sidecar} next to {pa}; flying with staircase collisions (pass --cuts PATH, or --no-cuts to silence)"
        );
    }
    let surface = cuts_path
        .as_ref()
        .map(|path| -> GpuResult<SurfaceModel> {
            Ok(
                SurfaceModel::from_reader(BufReader::new(File::open(path)?))?
                    .with_tolerance(options.tolerance.unwrap_or(1e-12) / options.scale)?,
            )
        })
        .transpose()?
        .map(|surface| {
            if options.surface_collisions.as_deref() == Some("insulators") {
                surface.select_collision_only(true)
            } else {
                surface
            }
        })
        .map(std::sync::Arc::new);
    if options.surface_backend == "sdf" && surface.is_none() {
        return Err("--surface-backend sdf requires exact surfaces or --surface-collisions".into());
    }
    let surface_mode = match surface
        .as_ref()
        .filter(|_| options.surface_collisions.is_none())
        .map(|surface| surface.accuracy())
    {
        Some(SurfaceAccuracy::ApproximateV1) => "approximate-v1",
        Some(SurfaceAccuracy::SampledSdf) => "sampled-sdf",
        Some(SurfaceAccuracy::SourceGeometry) => "source-geometry",
        None if options.nearest_node_collisions => "nearest-node",
        None => "staircase",
    };
    if let Some(surface) = &surface {
        if options.device == FlyDevice::Gpu
            && options.surface_backend == "exact"
            && !matches!(options.space_charge, Some(SpaceChargeMethod::Pic))
        {
            return Err("surface collisions require CPU; use --device cpu or auto, or --no-cuts for staircase GPU flying".into());
        }
        eprintln!(
            "surface: {} ({}; {})",
            cuts_path.as_ref().unwrap().display(),
            if options.cuts.is_some() {
                "explicit --cuts"
            } else {
                "automatic sibling pickup"
            },
            match surface.accuracy() {
                _ if options.surface_backend == "sdf" => "sampled SDF collision backend",
                SurfaceAccuracy::ApproximateV1 =>
                    "APPROXIMATE v1 reconstruction from link intersections",
                SurfaceAccuracy::SampledSdf => "APPROXIMATE sampled SDF (v2); CPU",
                SurfaceAccuracy::SourceGeometry => "source geometry (v3); CPU",
            }
        );
    } else if options.pa.is_some() {
        eprintln!(
            "surface: staircase ({})",
            if options.no_cuts {
                "--no-cuts disables sibling pickup"
            } else {
                "no sibling .cuts.json found"
            }
        );
    }
    let axes = std::array::from_fn(|a| std::array::from_fn(|b| options.axes[3 * a + b]));
    let bounds = options
        .bounds
        .map(|b| [[b[0], b[1], b[2]], [b[3], b[4], b[5]]]);
    let plane = options.plane.map(|p| ([p[0], p[1], p[2]], p[3]));
    let (mut magnetic, collision, seed) =
        options
            .physics
            .prepare(run.physics, options.origin, axes, options.scale, bounds)?;
    magnetic.maps.extend(load_magnetic_maps(&options)?);
    for path in &options.magnetic_vector_potential {
        let header: MagneticFieldFile = serde_json::from_reader(BufReader::new(File::open(path)?))?;
        if header.version != 1 || header.formulation == Formulation::Scalar {
            return Err(
                "vector-potential sidecar requires version 1 and a vector formulation".into(),
            );
        }
        let parent = path.parent().unwrap_or(std::path::Path::new("."));
        let field = if header.formulation == Formulation::CartesianA {
            let paths = header
                .components
                .ok_or("Cartesian A sidecar requires three component paths")?;
            let arrays = [
                simrep_io::read_pa(parent.join(&paths[0]))?.array,
                simrep_io::read_pa(parent.join(&paths[1]))?.array,
                simrep_io::read_pa(parent.join(&paths[2]))?.array,
            ];
            VectorPotential::cartesian(
                arrays,
                header.ng,
                options.origin,
                axes,
                options.scale,
                bounds,
            )?
        } else {
            if header.components.is_some() {
                return Err("only Cartesian A sidecars may name three components".into());
            }
            VectorPotential::placed(
                simrep_io::read_pa(parent.join(header.potential))?.array,
                header.formulation,
                header.ng,
                options.origin,
                axes,
                options.scale,
                bounds,
            )?
        };
        magnetic
            .vector_potentials
            .push(field.with_material_cells(header.solid_cells)?);
    }
    let cpu_only_magnetic = !magnetic.vector_potentials.is_empty()
        || magnetic
            .arrays
            .iter()
            .chain(&magnetic.support)
            .any(|pa| pa.has_material_cells());
    if cpu_only_magnetic && options.device == FlyDevice::Gpu && options.surface_backend != "auto" {
        return Err(
            "vector potentials and magnetic material cells require --device cpu or auto".into(),
        );
    }
    let identity = flight_shards::run_identity(identity)?;
    let particle_data = if options
        .particles
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("ion"))
    {
        simrep_io::read_ion(&options.particles)?
    } else {
        let mut launch_adjustments: BTreeMap<_, _> = options.voltage.iter().copied().collect();
        launch_adjustments.extend(expressions.iter().map(|(&n, e)| (n, e.evaluate(0.0))));
        let array = read_electric(&launch_adjustments.into_iter().collect::<Vec<_>>())?;
        let instances = options
            .pa
            .as_ref()
            .map(|_| Fly2Instance {
                origin_mm: options.origin,
                axes,
                scale: options.scale,
                spacing_mm: [array.dx_mm, array.dy_mm, array.dz_mm],
                dimensions: [array.nx, array.ny, array.nz],
                electrodes: Some(array.electrodes.clone().into()),
                ..Default::default()
            })
            .into_iter()
            .collect();
        let reconstruction = if options.global_cubic {
            Reconstruction::GlobalCubic
        } else {
            Reconstruction::BoundaryAware
        };
        let grid = GridField::with_reconstruction(array, reconstruction)?;
        let adaptive = adaptive_field(&grid, &options.voltage)?;
        let mut source =
            InstanceField::new(grid, options.origin, axes, options.scale, bounds, plane)?;
        if let Some(adaptive) = adaptive {
            source = source.with_adaptive(adaptive);
        }
        if let Some(surface) = &surface {
            source = if options.surface_collisions.is_some() {
                source.with_collision_surface(surface.clone())?
            } else {
                source.with_surface(surface.clone())?
            };
        }
        let provider = |_, point| {
            let sample = source.sample(point, 0.0);
            Ok(Fly2Field {
                potential_v: sample.potential_v,
                electric_v_per_mm: sample.electric_v_per_mm,
                magnetic_gauss: magnetic.sample(point, 0.0).magnetic_gauss,
                electrode: matches!(
                    source.status(point),
                    Some(SplatReason::Electrode { .. } | SplatReason::SurfaceElectrode { .. })
                ),
            })
        };
        read_particles_in(
            &options.particles,
            &Fly2Context {
                instances,
                field: Some(&provider),
                seed,
                ..Default::default()
            },
        )?
    };
    // Expand the entire source before partitioning: FLY2 sampling and birth
    // order are properties of the original run, not of an individual worker.
    let total_ions = particle_data.len();
    let indices: Vec<_> = (0..total_ions)
        .filter(|&i| options.shard.is_none_or(|s| s.contains(i)))
        .collect();
    let particle_data: Vec<_> = particle_data
        .into_iter()
        .enumerate()
        .filter(|(i, _)| options.shard.is_none_or(|s| s.contains(*i)))
        .map(|(_, p)| p)
        .collect();
    let has_magnetic = magnetic.uniform_gauss != [0.0; 3]
        || !magnetic.arrays.is_empty()
        || !magnetic.vector_potentials.is_empty()
        || !magnetic.coils.is_empty()
        || !magnetic.maps.is_empty();
    let crossover = if adaptive_manifest.is_some()
        || beam_mode
        || cpu_only_magnetic
        || has_magnetic
        || magnetic.support.is_some()
    {
        None
    } else {
        match options.space_charge {
            Some(SpaceChargeMethod::Pic) => Some(GPU_PIC_THRESHOLD),
            Some(SpaceChargeMethod::Direct) => None,
            None => options.compute_crossover(
                has_magnetic || magnetic.support.is_some(),
                collision.as_ref(),
            ),
        }
    };
    let init = Instant::now();
    let surface_auto = options.surface_backend == "auto";
    let (auto_gpu, auto_reason) = if surface_auto {
        let (gpu, reason) = sdf_auto::select(
            sdf_auto::Policy {
                device: match options.device {
                    FlyDevice::Cpu => "cpu",
                    FlyDevice::Auto => "auto",
                    FlyDevice::Gpu => "gpu",
                },
                vulkan: options.backend.backends().contains(wgpu::Backends::VULKAN),
                surfaces: surface.is_some(),
                physics_supported: !beam_mode
                    && options.space_charge.is_none()
                    && adaptive_manifest.is_none()
                    && !cpu_only_magnetic
                    && !has_magnetic
                    && magnetic.support.is_none()
                    && collision.is_none(),
                ions: total_ions,
                divisor: options.surface_sdf_resolution_divisor,
            },
            || match options.gpu_adapter.as_deref() {
                Some(selector) => pollster::block_on(crate::gpu_adapter::selected(
                    wgpu::Backends::VULKAN,
                    selector,
                )),
                None => pollster::block_on(Gpu::new(wgpu::Backends::VULKAN)),
            },
        )?;
        (gpu, Some(reason))
    } else {
        (None, None)
    };
    let executed_surface_backend = if surface_auto {
        if auto_gpu.is_some() { "sdf" } else { "exact" }
    } else {
        options.surface_backend.as_str()
    };
    let request_gpu = !beam_mode && options.device.requests_gpu(total_ions, crossover);
    let mut reason = if adaptive_manifest.is_some() {
        "adaptive fields require CPU".to_owned()
    } else if beam_mode {
        "leader-plane beam integration requires CPU".to_owned()
    } else if surface.is_some() && !matches!(options.space_charge, Some(SpaceChargeMethod::Pic)) {
        "surface collisions require CPU".to_owned()
    } else if cpu_only_magnetic {
        "vector potentials and magnetic material cells require CPU".to_owned()
    } else if has_magnetic || magnetic.support.is_some() {
        "magnetic field: GPU magnetic flight not selected automatically".to_owned()
    } else if matches!(options.space_charge, Some(SpaceChargeMethod::Direct)) {
        "direct Coulomb physics requires CPU".to_owned()
    } else if let Some(threshold) = crossover {
        format!(
            "{} ions {} measured crossover {threshold}",
            total_ions,
            if request_gpu { ">=" } else { "<" }
        )
    } else {
        "no measured GPU crossover for this collision profile".to_owned()
    };
    let gpu = if surface_auto {
        auto_gpu
    } else if (surface.is_none()
        || options.surface_backend == "sdf" && options.device == FlyDevice::Gpu
        || options.surface_collisions.is_none()
            && matches!(options.space_charge, Some(SpaceChargeMethod::Pic)))
        && request_gpu
    {
        let selected = match options.gpu_adapter.as_deref() {
            Some(selector) => pollster::block_on(crate::gpu_adapter::selected(
                options.backend.backends(),
                selector,
            ))?,
            None => pollster::block_on(Gpu::new(options.backend.backends()))?,
        };
        match selected {
            None if options.device == FlyDevice::Gpu => {
                return Err("no adapter available for GPU flying".into());
            }
            None => {
                reason = "no adapter available for the requested backend".into();
                None
            }
            Some(gpu) => {
                if options.device == FlyDevice::Gpu {
                    crate::require_hardware_gpu(&gpu.info)?;
                }
                if options.device == FlyDevice::Auto && gpu.info.device_type == DeviceType::Cpu {
                    reason = format!("adapter {} is a CPU/software adapter", gpu.info.name);
                    None
                } else if options.device == FlyDevice::Auto && !gpu.has_f64() {
                    reason = format!("adapter {} lacks native f64", gpu.info.name);
                    None
                } else if options.device == FlyDevice::Auto
                    && collision.is_some()
                    && gpu.info.name != "NVIDIA GeForce RTX 3070"
                {
                    reason = format!(
                        "collision crossover was measured on NVIDIA GeForce RTX 3070, not {}",
                        gpu.info.name
                    );
                    None
                } else {
                    Some(gpu)
                }
            }
        }
    } else {
        None
    };
    let initialization = init.elapsed();
    if let Some(auto_reason) = auto_reason {
        reason = auto_reason;
        eprintln!("{reason}");
    }
    let use_gpu = gpu.is_some();
    eprintln!(
        "device: {}, requested: {:?}, auto_gpu_threshold: {:?} (measured compute-only class calibration); {}",
        if use_gpu { "gpu" } else { "cpu" },
        options.device,
        crossover,
        if options.device == FlyDevice::Auto {
            format!("auto: {} ({reason})", if use_gpu { "GPU" } else { "CPU" })
        } else {
            "explicit --device".to_owned()
        }
    );
    if let Some(gpu) = &gpu {
        eprintln!(
            "Adapter: {} ({:?}, {:?})",
            gpu.info.name, gpu.info.backend, gpu.info.device_type
        );
        if matches!(options.space_charge, Some(SpaceChargeMethod::Pic)) {
            crate::report_gpu_residual(gpu.has_f64());
        } else if !gpu.has_f64() {
            eprintln!(
                "GPU precision: f32 (adapter lacks native f64; tolerance floor 2e-7, two-float TOF)"
            );
        }
    }
    let reconstruction = if options.global_cubic {
        Reconstruction::GlobalCubic
    } else {
        Reconstruction::BoundaryAware
    };
    let grid_field = |array| -> Result<GridField, simrep_fly::FlyError> {
        let field = GridField::with_reconstruction(array, reconstruction)?;
        Ok(if options.nearest_node_collisions {
            field.with_nearest_node_collisions()
        } else {
            field
        })
    };
    let mut bases = Vec::new();
    let mut dynamic_bases = Vec::new();
    let mut adjustments: BTreeMap<_, _> = options.voltage.iter().copied().collect();
    adjustments.extend(expressions.keys().map(|&n| (n, 0.0)));
    let adjustments: Vec<_> = adjustments.into_iter().collect();
    let array = if use_gpu && options.space_charge.is_none() {
        // read_fast_adjust owns all SIMION normalization/background rules.
        // Differencing zero/unit adjustments obtains the same unit basis
        // without duplicating those file-format rules in the flyer.
        let zeros: Vec<_> = adjustments.iter().map(|&(n, _)| (n, 0.0)).collect();
        let background = read_electric(&zeros)?;
        for &(n, _) in &adjustments {
            let zero = read_electric(&[(n, 0.0)])?;
            let mut unit = read_electric(&[(n, 1.0)])?;
            for (v, z) in unit.potentials.iter_mut().zip(zero.potentials) {
                *v -= z;
            }
            let basis = grid_field(unit)?;
            if let Some(e) = expressions.get(&n) {
                dynamic_bases.push((basis, e.clone()));
            } else {
                bases.push(basis);
            }
        }
        background
    } else {
        for (&n, expression) in &expressions {
            let zero = read_electric(&[(n, 0.0)])?;
            let mut unit = read_electric(&[(n, 1.0)])?;
            for (v, z) in unit.potentials.iter_mut().zip(zero.potentials) {
                *v -= z;
            }
            dynamic_bases.push((grid_field(unit)?, expression.clone()));
        }
        read_electric(&adjustments)?
    };
    let sdf = if executed_surface_backend == "sdf" {
        Some(std::sync::Arc::new(
            simrep_fly::surface_sdf::SdfVolumes::build(
                surface.as_ref().unwrap(),
                &array,
                options.surface_sdf_resolution_divisor,
            )?,
        ))
    } else {
        None
    };
    let sdf_report = sdf.as_ref().map(|s| s.volumes.iter().map(|v| serde_json::json!({ "number": v.number, "collision_only": v.collision_only, "resolution_mm": v.spacing, "lipschitz": v.lipschitz, "interp_error_mm": v.interp_error_mm, "error_bound_mm": v.error_bound_mm })).collect::<Vec<_>>());
    let grid = grid_field(array)?;
    let adaptive = adaptive_field(&grid, &adjustments)?;
    let mut field = InstanceField::new(grid, options.origin, axes, options.scale, bounds, plane)?
        .with_voltage_expressions(dynamic_bases)?;
    if let Some(adaptive) = adaptive {
        field = field.with_adaptive(adaptive);
    }
    if let Some(surface) = surface {
        field = if options.surface_collisions.is_some() {
            field.with_collision_surface(surface)?
        } else {
            field.with_surface(surface)?
        };
    }
    if let Some(sdf) = sdf {
        field = field.with_surface_sdf(sdf);
    }
    let ghost_fill = field.ghost_fill_report();
    if let Some(report) = &ghost_fill {
        eprintln!("ghost_fill: {}", serde_json::to_string(report)?);
    }
    if (beam_mode || options.space_charge.is_some()) && collision.is_some() {
        return Err("space charge does not support collision models".into());
    }
    let nonrelativistic = options.nonrelativistic || collision.is_some();
    if collision.is_some() && !options.nonrelativistic {
        eprintln!(
            "dynamics: nonrelativistic (collision models require nonrelativistic integration)"
        );
    }
    let weights = particle_data
        .iter()
        .map(|p| p.charge_weight)
        .collect::<Vec<_>>();
    let particles = particle_data
        .into_iter()
        .map(|p| {
            if nonrelativistic {
                Particle::from_energy(
                    p.mass_amu,
                    p.charge_e,
                    p.position_mm,
                    p.energy_ev,
                    p.direction,
                    p.birth_time_us,
                )
            } else {
                Particle::from_relativistic_energy(
                    p.mass_amu,
                    p.charge_e,
                    p.position_mm,
                    p.energy_ev,
                    p.direction,
                    p.birth_time_us,
                )
            }
        })
        .collect::<Result<Vec<_>, _>>()?;
    let flight_options = FlightOptions {
        integrator: options.physics.integrator.into(),
        automatic_tolerance: options.tolerance.is_none(),
        relative_tolerance: options.tolerance.unwrap_or(1e-12),
        position_tolerance_mm: options.tolerance.unwrap_or(1e-12),
        velocity_tolerance_mm_per_us: options.tolerance.unwrap_or(1e-12),
        initial_step_us: options
            .physics
            .step_us
            .min(options.max_step_us.unwrap_or(f64::MAX)),
        max_step_us: options.max_step_us.unwrap_or(f64::MAX),
        duration_us: options.duration_us,
        relativistic: !nonrelativistic,
        sample_every: options.trajectories.as_ref().map(|_| options.sample_every),
        ..FlightOptions::default()
    };
    let mut beam_terminations = None;
    let cpu_magnetic = if gpu.is_none() || beam_mode || options.space_charge.is_some() {
        Some(if options.exact_coils {
            PreparedMagneticField::exact(&magnetic)
        } else {
            magnetic.prepare_coil_map(bounds)?
        })
    } else {
        None
    };
    if let Some(prepared) = &cpu_magnetic {
        for (region, c) in prepared.calibration().iter().enumerate() {
            eprintln!(
                "cpu_coil_map: region={region}, nodes={}, max_abs_gauss={:.6e}, max_relative={:.6e}, accepted={}",
                c.nodes, c.max_absolute_gauss, c.max_relative, c.accepted
            );
        }
    }
    let (results, setup, flight) = if beam_mode {
        let field = Electromagnetic {
            electric: field,
            magnetic: cpu_magnetic.unwrap(),
        };
        let setup = start.elapsed();
        let report = fly_beam(
            &field,
            &particles,
            &BeamOptions {
                flight: flight_options,
                current_a: options.beam_current_a.unwrap(),
                charge_weights: weights,
                softening_mm: options.charge_softening_mm,
                space_charge_step_fraction: options
                    .space_charge_step_fraction
                    .unwrap_or_else(|| BeamOptions::default().space_charge_step_fraction),
                ..BeamOptions::default()
            },
        )?;
        eprintln!(
            "beam_semantics: SIMION-documented beam semantics; global_steps: {}, rejected: {}",
            report.ensemble.global_steps, report.ensemble.rejected_steps
        );
        beam_terminations = Some(report.terminations);
        (report.ensemble.results, setup, report.ensemble.compute_time)
    } else if let Some(method) = options.space_charge {
        let mut space_charge = match method {
            SpaceChargeMethod::Direct => SpaceCharge::Direct(DirectCharge {
                softening_mm: options.charge_softening_mm,
            }),
            SpaceChargeMethod::Pic => {
                let bounds = bounds.ok_or("PIC requires --bounds covering the full ensemble")?;
                let dims: [usize; 3] = options
                    .pic_dims
                    .clone()
                    .try_into()
                    .map_err(|_| "PIC needs three dimensions")?;
                if dims.iter().any(|&n| n < 3) {
                    return Err("PIC dimensions must each be at least three".into());
                }
                let count = dims
                    .into_iter()
                    .try_fold(1usize, usize::checked_mul)
                    .ok_or("PIC mesh size overflow")?;
                let spacing =
                    std::array::from_fn(|a| (bounds[1][a] - bounds[0][a]) / (dims[a] - 1) as f64);
                let fixed: Vec<_> = (0..count)
                    .map(|i| {
                        let c = [
                            i % dims[0],
                            (i / dims[0]) % dims[1],
                            i / (dims[0] * dims[1]),
                        ];
                        let p = std::array::from_fn(|a| bounds[0][a] + c[a] as f64 * spacing[a]);
                        (0..3).any(|a| c[a] == 0 || c[a] + 1 == dims[a])
                            || matches!(
                                field.status(p),
                                Some(
                                    SplatReason::Electrode { .. }
                                        | SplatReason::SurfaceElectrode { .. }
                                )
                            )
                    })
                    .collect();
                let mesh = PicMesh::new(
                    Grid {
                        dims,
                        spacing,
                        geometry: Geometry::Planar,
                    },
                    bounds[0],
                    fixed,
                    PoissonOptions {
                        relative_tolerance: options.tolerance.unwrap_or(1e-12),
                        ..PoissonOptions::default()
                    },
                )?;
                if let Some(gpu) = &gpu {
                    let solver_gpu = simrep_solve::gpu::Gpu::from_device(
                        gpu.device().clone(),
                        gpu.queue().clone(),
                        gpu.info.clone(),
                    );
                    SpaceCharge::GpuPic(Box::new(GpuPic::new(solver_gpu, mesh, particles.len())?))
                } else {
                    SpaceCharge::Pic(Box::new(mesh))
                }
            }
        };
        let field = Electromagnetic {
            electric: field,
            magnetic: cpu_magnetic.unwrap(),
        };
        let setup = start.elapsed();
        let report = fly_ensemble(
            &field,
            &particles,
            &EnsembleOptions {
                flight: flight_options,
                charge_mode: match options.charge_mode {
                    ChargeModeName::Factor => ChargeMode::Factor(options.charge_amount),
                    ChargeModeName::Coulombic => ChargeMode::Coulombic(options.charge_amount),
                    ChargeModeName::Beam => unreachable!(),
                },
                charge_weights: weights,
                space_charge_step_fraction: options.space_charge_step_fraction.unwrap_or(0.1),
            },
            &mut space_charge,
        )?;
        eprintln!(
            "ensemble_global_steps: {}, rejected: {}",
            report.global_steps, report.rejected_steps
        );
        (report.results, setup, report.compute_time)
    } else if let Some(gpu) = &gpu {
        let mut resident = if has_magnetic || collision.is_some() {
            gpu.prepare_physics(&field, &bases, &magnetic, collision.as_ref(), seed)?
        } else {
            gpu.prepare(&field, &bases)?
        };
        let voltages: Vec<_> = std::iter::once(1.0)
            .chain(
                adjustments
                    .iter()
                    .filter(|(n, _)| !expressions.contains_key(n))
                    .map(|&(_, v)| v),
            )
            .chain(std::iter::repeat_n(0.0, expressions.len()))
            .collect();
        resident.set_voltages(&voltages)?;
        let setup = start.elapsed();
        let sampling_indices: Vec<_> = (0..particles.len()).collect();
        let sampling = options
            .trajectories
            .as_ref()
            .map(|_| simrep_fly::gpu::TrajectorySampling {
                ion_indices: &sampling_indices,
                every: options.sample_every,
                max_samples_per_ion: options.max_samples_per_ion,
            });
        let mut gpu_options = flight_options.clone();
        gpu_options.sample_every = None;
        let report = resident.fly_interleaved(
            &particles,
            &gpu_options,
            options.shard.map_or(0, |s| (s.index - 1) as u64),
            options.shard.map_or(1, |s| s.count as u64),
            sampling.as_ref(),
        )?;
        eprintln!(
            "gpu_init_s: {:.6}, gpu_setup_s: {:.6}, gpu_upload_s: {:.6}, gpu_fly_s: {:.6}, gpu_readback_s: {:.6}",
            initialization.as_secs_f64(),
            (resident.timings.setup + report.timings.setup).as_secs_f64(),
            (resident.timings.upload + report.timings.upload).as_secs_f64(),
            report.timings.fly.as_secs_f64(),
            report.timings.readback.as_secs_f64()
        );
        eprintln!(
            "GPU precision: {}; compute clock: {}",
            if gpu.has_f64() {
                "native f64, tolerance floor 1e-13"
            } else {
                "f32, tolerance floor 2e-7, two-float TOF"
            },
            if gpu.has_timestamps() {
                "GPU pass timestamps"
            } else {
                "submission-through-completion (timestamps unavailable)"
            }
        );
        (report.results, setup, report.timings.fly)
    } else {
        let field = Electromagnetic {
            electric: field,
            magnetic: cpu_magnetic.unwrap(),
        };
        let setup = start.elapsed();
        let flight_start = Instant::now();
        let results = if let Some(model) = &collision {
            if options.shard.is_some() {
                particles
                    .par_iter()
                    .zip(&indices)
                    .map(|(p, &i)| {
                        fly_collisional(
                            &field,
                            p,
                            &flight_options,
                            model,
                            seed.wrapping_add((i as u64).wrapping_mul(0x9e3779b97f4a7c15)),
                        )
                    })
                    .collect::<Result<Vec<_>, _>>()?
            } else {
                fly_many_collisional(&field, &particles, &flight_options, model, seed)?
            }
        } else {
            fly_many(&field, &particles, &flight_options)?
        };
        (results, setup, flight_start.elapsed())
    };
    let writer: Box<dyn Write> = match &options.output {
        Some(path) => Box::new(File::create(path)?),
        None if crate::json::enabled() => Box::new(io::stderr().lock()),
        None => Box::new(io::stdout().lock()),
    };
    let mut writer = BufWriter::new(writer);
    writeln!(writer, "{}", flight_shards::CSV_HEADER)?;
    let mut splats = 0;
    let mut counts: BTreeMap<_, usize> = [
        "electrode",
        "outside",
        "plane",
        "time_limit",
        "step_limit",
        "step_underflow",
        "nonfinite",
        "beam_diverged",
        "beam_leader_stopped",
    ]
    .into_iter()
    .map(|reason| (reason, 0))
    .collect();
    let mut arrivals = Vec::new();
    for (i, (particle, result)) in particles.iter().zip(&results).enumerate() {
        let p = result.final_state.position_mm;
        let v2 = result
            .final_state
            .velocity_mm_per_us
            .iter()
            .map(|v| v * v)
            .sum::<f64>();
        let ke = if nonrelativistic {
            particle.mass_amu * v2 / (2.0 * ELECTRIC_ACCELERATION)
        } else {
            let inverse_gamma = (1.0 - v2 / SPEED_OF_LIGHT.powi(2)).sqrt();
            particle.mass_amu * v2 / (ELECTRIC_ACCELERATION * inverse_gamma * (1.0 + inverse_gamma))
        };
        let reason = match beam_terminations.as_ref().map(|t| t[i]) {
            Some(BeamTermination::Diverged) => "beam_diverged",
            Some(BeamTermination::LeaderStopped) => "beam_leader_stopped",
            _ => match result.splat_reason {
                SplatReason::Electrode { .. } | SplatReason::SurfaceElectrode { .. } => "electrode",
                SplatReason::OutsideGrid => "outside",
                SplatReason::StopPlane => "plane",
                SplatReason::TimeLimit => "time_limit",
                SplatReason::StepLimit => "step_limit",
                SplatReason::StepUnderflow => "step_underflow",
                SplatReason::NonFinite => "nonfinite",
            },
        };
        *counts.get_mut(reason).unwrap() += 1;
        if reason == "plane" {
            arrivals.push(result.tof_us);
        }
        if matches!(
            result.splat_reason,
            SplatReason::Electrode { .. }
                | SplatReason::SurfaceElectrode { .. }
                | SplatReason::OutsideGrid
                | SplatReason::StopPlane
        ) {
            splats += 1;
        }
        writeln!(
            writer,
            "{},{:.15e},{:.15e},{:.15e},{:.15e},{:.15e},{},{},{:.15e},{:.15e},{:.15e}",
            indices[i] + 1,
            result.tof_us,
            p[0],
            p[1],
            p[2],
            ke,
            reason,
            match result.splat_reason {
                SplatReason::SurfaceElectrode { number } => number.to_string(),
                _ => String::new(),
            },
            result.final_state.velocity_mm_per_us[0],
            result.final_state.velocity_mm_per_us[1],
            result.final_state.velocity_mm_per_us[2]
        )?;
    }
    writer.flush()?;
    let precision = if use_gpu && options.space_charge.is_some() {
        "mixed-f32-f64"
    } else if gpu.as_ref().is_none_or(Gpu::has_f64) {
        "f64"
    } else {
        "f32"
    };
    let execution = execution_identity(
        precision,
        gpu.as_ref().map(|g| (g.info.backend, g.info.device_type)),
    );
    let run_hash = flight_shards::sha256(&serde_json::to_vec(
        &serde_json::json!({"inputs": identity, "execution": execution}),
    )?);
    let mut summary = serde_json::json!({
        "schema_version": 1, "simrep_version": env!("CARGO_PKG_VERSION"), "ion_count": particles.len(), "total_ions": total_ions, "run_hash": run_hash,
        "shard": options.shard.map(|s| serde_json::json!({"index": s.index, "count": s.count, "ions": particles.len()})),
        "splat_counts": counts, "arrivals": arrivals.len(), "arrival_tof_us": flight_shards::tof_stats(&arrivals), "compute_ms": flight.as_secs_f64() * 1000.0, "prep_ms": setup.as_secs_f64() * 1000.0, "ghost_fill": ghost_fill,
        "requested": { "device": format!("{:?}", options.device).to_lowercase(), "precision": "f64", "backend": format!("{:?}", options.backend).to_lowercase(), "gpu_adapter": options.gpu_adapter, "surface_backend": options.surface_backend, "surface_mode": if options.no_cuts { "staircase" } else if options.cuts.is_some() { "continuous" } else if options.nearest_node_collisions { "nearest-node" } else { "auto" } },
        "executed": { "device": if use_gpu { "gpu" } else { "cpu" }, "precision": precision, "backend": gpu.as_ref().map(|g| format!("{:?}", g.info.backend).to_lowercase()), "gpu_adapter": gpu.as_ref().map(|g| &g.info.name), "adapter_class": execution["adapter_class"], "surface_mode": surface_mode, "surface_backend": if cuts_path.is_some() || surface_auto { executed_surface_backend } else { "staircase" },
            "surface_sdf_resolution_mm": sdf_report.as_ref().map(|r| r.iter().map(|v| &v["resolution_mm"]).collect::<Vec<_>>()),
            "surface_sdf_lipschitz": sdf_report.as_ref().map(|r| r.iter().map(|v| serde_json::json!({"number": v["number"], "lipschitz": v["lipschitz"]})).collect::<Vec<_>>()),
            "surface_sdf_interp_error_mm": sdf_report.as_ref().map(|r| r.iter().map(|v| serde_json::json!({"number": v["number"], "interp_error_mm": v["interp_error_mm"]})).collect::<Vec<_>>()),
            "surface_sdf_error_bound_mm": sdf_report.as_ref().map(|r| r.iter().map(|v| serde_json::json!({"number": v["number"], "error_bound_mm": v["error_bound_mm"]})).collect::<Vec<_>>()), "surface_collisions": options.surface_collisions, "cuts_path": cuts_path, "cuts_source": cuts_source },
        "reason": if surface_auto || options.device == FlyDevice::Auto { reason.as_str() } else { "explicit --device" },
        "fallback_reason": if surface_auto { None } else if gpu.as_ref().is_some_and(|g| !g.has_f64()) { Some(if options.space_charge.is_some() { "adapter lacks native f64; GPU f32 corrections with CPU f64 flight/residual" } else { "adapter lacks native f64; GPU f32 flight with two-float TOF" }) } else { (options.device == FlyDevice::Auto && !use_gpu).then_some(reason.as_str()) },
        "output_paths": options.output.iter().collect::<Vec<_>>(),
    });
    eprintln!(
        "ions: {}, splats: {}, setup_s: {:.6}, fly_s: {:.6}, total_s: {:.6}, ions_per_s: {:.0}",
        particles.len(),
        splats,
        setup.as_secs_f64(),
        flight.as_secs_f64(),
        start.elapsed().as_secs_f64(),
        particles.len() as f64 / flight.as_secs_f64()
    );
    eprintln!(
        "accepted_steps: {}, rejected_steps: {}",
        results.iter().map(|r| r.accepted_steps).sum::<usize>(),
        results.iter().map(|r| r.rejected_steps).sum::<usize>()
    );
    let beam_stops = beam_terminations.as_ref().map_or(0, |terms| {
        let leader_allowed = results.first().is_some_and(|r| {
            matches!(
                r.splat_reason,
                SplatReason::Electrode { .. }
                    | SplatReason::SurfaceElectrode { .. }
                    | SplatReason::OutsideGrid
                    | SplatReason::StopPlane
            ) || options.physics.allow_time_limit && r.splat_reason == SplatReason::TimeLimit
        });
        if leader_allowed {
            terms
                .iter()
                .filter(|&&t| t == BeamTermination::LeaderStopped)
                .count()
        } else {
            0
        }
    });
    let time_limits = results
        .iter()
        .enumerate()
        .filter(|(i, r)| {
            r.splat_reason == SplatReason::TimeLimit
                && beam_terminations
                    .as_ref()
                    .is_none_or(|t| matches!(t[*i], BeamTermination::Flight(_)))
        })
        .count();
    let completed = splats
        + beam_stops
        + if options.physics.allow_time_limit {
            time_limits
        } else {
            0
        };
    if completed != particles.len() {
        crate::json::set_summary(summary);
        return Err(format!(
            "{} particles did not reach an allowed termination; inspect CSV reasons",
            particles.len() - completed
        )
        .into());
    }
    let trajectory = if let Some(path) = &options.trajectories {
        let mut writer = BufWriter::new(File::create(path)?);
        writeln!(writer, "{}", flight_shards::TRAJECTORY_HEADER)?;
        for (&index, result) in indices.iter().zip(&results) {
            for state in result
                .trajectory
                .as_ref()
                .ok_or("missing sampled trajectory")?
            {
                writeln!(
                    writer,
                    "{},{:.15e},{:.15e},{:.15e},{:.15e},{:.15e},{:.15e},{:.15e}",
                    index + 1,
                    state.time_us,
                    state.position_mm[0],
                    state.position_mm[1],
                    state.position_mm[2],
                    state.velocity_mm_per_us[0],
                    state.velocity_mm_per_us[1],
                    state.velocity_mm_per_us[2]
                )?;
            }
        }
        writer.flush()?;
        summary["output_paths"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!(path));
        let absolute = std::fs::canonicalize(path)?;
        let output_parent = options
            .output
            .as_ref()
            .unwrap()
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(std::path::Path::new("."));
        let output_parent = std::fs::canonicalize(output_parent)?;
        let file = absolute
            .strip_prefix(&output_parent)
            .unwrap_or(&absolute)
            .to_path_buf();
        Some(flight_shards::TrajectoryFile {
            file,
            sha256: flight_shards::sha256(&std::fs::read(path)?),
        })
    } else {
        None
    };
    if let Some(path) = &options.output {
        summary["output_paths"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!(flight_shards::sidecar(path)));
        flight_shards::write_metadata(
            path,
            &flight_shards::FlightFile {
                schema_version: 1,
                run_hash,
                total_ions,
                shard: options.shard,
                csv_sha256: flight_shards::sha256(&std::fs::read(path)?),
                tof_us: results.iter().map(|r| r.tof_us).collect(),
                summary: summary.clone(),
                trajectories: trajectory,
            },
        )?;
    }
    crate::json::set_summary(summary);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    #[derive(Parser)]
    struct TestCli {
        #[command(flatten)]
        fly: FlyOptions,
    }
    #[test]
    fn resolved_backend_and_adapter_class_separate_shard_identities() {
        let hash = |backend, class| {
            flight_shards::sha256(
                &serde_json::to_vec(&serde_json::json!({
                    "inputs": {"requested_backend": "auto"},
                    "execution": execution_identity("f64", Some((backend, class))),
                }))
                .unwrap(),
            )
        };
        let vulkan = hash(wgpu::Backend::Vulkan, wgpu::DeviceType::DiscreteGpu);
        assert_ne!(
            vulkan,
            hash(wgpu::Backend::Dx12, wgpu::DeviceType::DiscreteGpu)
        );
        assert_ne!(
            vulkan,
            hash(wgpu::Backend::Metal, wgpu::DeviceType::DiscreteGpu)
        );
        assert_ne!(
            vulkan,
            hash(wgpu::Backend::Vulkan, wgpu::DeviceType::IntegratedGpu)
        );
        assert_ne!(vulkan, hash(wgpu::Backend::Vulkan, wgpu::DeviceType::Cpu));
        assert_eq!(execution_identity("f64", None)["backend"], "cpu");
    }
    #[test]
    fn placement_and_voltages_parse_without_losing_negative_values() {
        let cli = TestCli::try_parse_from([
            "fly",
            "--pa",
            "lens.pa0",
            "--particles",
            "ions.ion",
            "--voltage",
            "1=0,2=-110",
            "--bounds=-101,-19,-19,291,19,19",
            "--plane",
            "1,1,0,80",
            "--global-cubic",
            "--nearest-node-collisions",
        ])
        .unwrap();
        assert_eq!(cli.fly.voltage, [(1, 0.0), (2, -110.0)]);
        assert_eq!(cli.fly.bounds.unwrap()[0], -101.0);
        assert_eq!(cli.fly.plane.unwrap(), [1.0, 1.0, 0.0, 80.0]);
        assert!(cli.fly.global_cubic);
        assert!(cli.fly.nearest_node_collisions);
        assert_eq!(cli.fly.device, FlyDevice::Auto);
        assert!(numbers::<3>("1,2").is_err());
        assert!(voltage("2=NaN").is_err());
    }
    #[test]
    fn gpu_device_is_additive() {
        let cli = TestCli::try_parse_from([
            "fly",
            "--pa",
            "lens.pa0",
            "--particles",
            "ions.ion",
            "--device",
            "gpu",
        ])
        .unwrap();
        assert_eq!(cli.fly.device, FlyDevice::Gpu);
        assert!(matches!(cli.fly.backend, crate::Backend::Vulkan));
    }
    #[test]
    fn beam_current_and_gpu_pic_modes_parse_additively() {
        let cli = TestCli::try_parse_from([
            "fly",
            "--particles",
            "ions.ion",
            "--charge-mode",
            "beam",
            "--beam-current-a",
            "1e-5",
        ])
        .unwrap();
        assert!(matches!(cli.fly.charge_mode, ChargeModeName::Beam));
        assert_eq!(cli.fly.beam_current_a, Some(1e-5));
        assert!(cli.fly.space_charge.is_none());
        let pic = TestCli::try_parse_from([
            "fly",
            "--particles",
            "ions.ion",
            "--space-charge",
            "pic",
            "--charge-mode",
            "coulombic",
            "--device",
            "gpu",
        ])
        .unwrap();
        assert!(matches!(pic.fly.space_charge, Some(SpaceChargeMethod::Pic)));
        assert_eq!(pic.fly.device, FlyDevice::Gpu);
        assert!(
            TestCli::try_parse_from([
                "fly",
                "--particles",
                "ions.ion",
                "--charge-mode",
                "coulombic"
            ])
            .is_err()
        );
    }
    #[test]
    fn automatic_device_uses_the_compute_crossover_and_respects_overrides() {
        let measured = Some(GPU_COMPUTE_THRESHOLD);
        assert!(!FlyDevice::Auto.requests_gpu(0, measured));
        assert!(!FlyDevice::Auto.requests_gpu(GPU_COMPUTE_THRESHOLD - 1, measured));
        assert!(FlyDevice::Auto.requests_gpu(GPU_COMPUTE_THRESHOLD, measured));
        assert!(!FlyDevice::Cpu.requests_gpu(GPU_COMPUTE_THRESHOLD, measured));
        assert!(FlyDevice::Gpu.requests_gpu(0, None));
        assert!(!FlyDevice::Auto.requests_gpu(usize::MAX, None));
    }
    #[test]
    fn hs1_auto_crossover_is_limited_to_the_measured_profile() {
        let mut options = TestCli::try_parse_from([
            "fly",
            "--particles",
            "ions.ion",
            "--duration-us",
            "50",
            "--max-step-us",
            "0.05",
        ])
        .unwrap()
        .fly;
        let model = CollisionModel::Hs1(Hs1::default());
        let measured = options.compute_crossover(false, Some(&model));
        assert_eq!(measured, Some(10_000));
        assert!(!FlyDevice::Auto.requests_gpu(9_999, measured));
        assert!(FlyDevice::Auto.requests_gpu(10_000, measured));
        assert_eq!(options.compute_crossover(true, Some(&model)), None);
        options.physics.integrator = crate::physics_cmd::Integration::Boris;
        assert_eq!(options.compute_crossover(false, Some(&model)), None);
        options.physics.integrator = crate::physics_cmd::Integration::Rk45;
        let mut changed = Hs1::default();
        changed.gas.pressure_pa *= 2.0;
        assert_eq!(
            options.compute_crossover(false, Some(&CollisionModel::Hs1(changed))),
            None
        );
        options.max_step_us = None;
        assert_eq!(options.compute_crossover(false, Some(&model)), None);
        options.max_step_us = Some(0.05);
        options.pa = Some("lens.pa0".into());
        assert_eq!(options.compute_crossover(false, Some(&model)), None);
    }
    #[test]
    fn waveform_run_settings_and_cli_arguments() {
        let run: VoltageRun = serde_json::from_str(
            r#"{"parameters":{"f":0.14},"voltage_expr":{"2":"100*cos(2*pi*f*t)"}}"#,
        )
        .unwrap();
        let e = VoltageExpression::parse(&run.voltage_expr[&2], &run.parameters).unwrap();
        assert_eq!(e.evaluate(0.0), 100.0);
        let cli = TestCli::try_parse_from([
            "fly",
            "--pa",
            "a.pa0",
            "--particles",
            "p.ion",
            "--voltage-expr",
            "2=100*sin(2*pi*f*t)",
            "--voltage-expr",
            "3=-5",
            "--param",
            "f=0.14",
            "--run-file",
            "run.json",
        ])
        .unwrap();
        assert_eq!(cli.fly.voltage_expr.len(), 2);
        assert_eq!(cli.fly.param, [("f".to_owned(), 0.14)]);
        assert_eq!(cli.fly.run_file, Some(PathBuf::from("run.json")));
        assert!(serde_json::from_str::<VoltageRun>(r#"{"unknown":5}"#).is_err());
    }
    #[test]
    fn magnetic_and_collision_inputs_are_additive() {
        let cli = TestCli::try_parse_from([
            "fly",
            "--particles",
            "ions.ion",
            "--bfield=0,0,100",
            "--integrator",
            "boris",
            "--step-us",
            "0.0001",
            "--collision",
            "hs1",
            "--pressure-pa",
            "0.53",
            "--gas-velocity=-0.01,0,0",
            "--allow-time-limit",
            "--exact-coils",
        ])
        .unwrap();
        assert!(cli.fly.pa.is_none());
        assert!(cli.fly.physics.allow_time_limit);
        assert!(cli.fly.exact_coils);
        assert_eq!(cli.fly.physics.step_us, 0.0001);
        let run: VoltageRun = serde_json::from_str(
            r#"{
            "physics":{"uniform_gauss":[0,0,10],"coils":[
                {"kind":"loop","center_mm":[0,0,0],"normal":[0,0,1],"radius_mm":30,"current_a":100}
            ]}
        }"#,
        )
        .unwrap();
        assert_eq!(run.physics.uniform_gauss, [0.0, 0.0, 10.0]);
        assert_eq!(run.physics.coils.len(), 1);
        assert!(serde_json::from_str::<VoltageRun>(r#"{"physics":{"unknown":1}}"#).is_err());
    }
    #[test]
    fn external_maps_cli_units_columns_placement_and_axisymmetric_frame() {
        use std::io::Write as _;
        let mut file = tempfile::NamedTempFile::new().unwrap();
        writeln!(file, "0,0,2,3,2,1\n0,0,3,3,2,1").unwrap();
        let cli = TestCli::try_parse_from([
            "fly",
            "--particles",
            "ions.ion",
            "--bfield-map",
            file.path().to_str().unwrap(),
            "--bfield-map-format",
            "csv",
            "--bfield-map-units",
            "mm/mT",
            "--bfield-map-columns",
            "2,1,0,5,4,3",
            "--bfield-map-transform",
            "10,20,30,0,1,0,0,0,1,1,0,0,2",
            "--bfield-map-outside",
            "clamp",
            "--bfield-map-divergence",
        ])
        .unwrap();
        let maps = load_magnetic_maps(&cli.fly).unwrap();
        let b = maps[0].sample([10.0, 25.0, 30.0], 0.0).magnetic_gauss;
        assert_eq!(b, [30.0, 10.0, 20.0]);
        assert_eq!(cli.fly.compute_crossover(true, None), None);
        let mut file = tempfile::NamedTempFile::new().unwrap();
        writeln!(file, "0 0 100 0\n1 0 100 0\n0 1 100 0\n1 1 100 0").unwrap();
        let cli = TestCli::try_parse_from([
            "fly",
            "--particles",
            "ions.ion",
            "--bfield-map",
            file.path().to_str().unwrap(),
            "--bfield-map-format",
            "axisymmetric",
        ])
        .unwrap();
        let maps = load_magnetic_maps(&cli.fly).unwrap();
        assert_eq!(
            maps[0].sample([0.2, 0.3, 0.5], 0.0).magnetic_gauss,
            [0.0, 0.0, 100.0]
        );
        assert!(map_units("bad/T").is_err());
        assert!(map_units("mm/bad").is_err());
    }
    #[test]
    fn external_maps_are_available_to_fly2_field_queries() {
        let directory = tempfile::tempdir().unwrap();
        let map = directory.path().join("uniform.csv");
        let particles = directory.path().join("ions.fly2");
        let output = directory.path().join("flight.csv");
        std::fs::write(&map, "0,0,0,2,3,4\n").unwrap();
        std::fs::write(
            &particles,
            "local bx,by,bz=simion.wb:bfield(0,0,0)\n\
             assert(bx==2 and by==3 and bz==4)\n\
             particles { standard_beam {n=1, mass=100, charge=0, ke=1} }\n",
        )
        .unwrap();
        let cli = TestCli::try_parse_from([
            "fly",
            "--device",
            "auto",
            "--particles",
            particles.to_str().unwrap(),
            "--bfield-map",
            map.to_str().unwrap(),
            "--duration-us",
            "0.01",
            "--allow-time-limit",
            "--output",
            output.to_str().unwrap(),
        ])
        .unwrap();
        run(cli.fly).unwrap();
        let csv = std::fs::read_to_string(output).unwrap();
        assert_eq!(csv.lines().count(), 2);
        assert_eq!(
            csv.lines().nth(1).unwrap().split(',').nth(6),
            Some("time_limit")
        );
    }
    #[test]
    fn external_map_cli_flight_stops_at_boundary_and_zero_extends() {
        let directory = tempfile::tempdir().unwrap();
        let map = directory.path().join("uniform.csv");
        let ions = directory.path().join("ions.ion");
        let output = directory.path().join("flight.csv");
        std::fs::write(
            &map,
            "x[mm],y[mm],z[mm],Bx[G],By[G],Bz[G]\n0,0,0,0,0,10\n1,0,0,0,0,10\n",
        )
        .unwrap();
        std::fs::write(&ions, "0,100,0,0.5,0,0,0,0,1\n").unwrap();
        for policy in ["error", "zero"] {
            let cli = TestCli::try_parse_from([
                "fly",
                "--device",
                "cpu",
                "--particles",
                ions.to_str().unwrap(),
                "--bfield-map",
                map.to_str().unwrap(),
                "--bfield-map-outside",
                policy,
                "--duration-us",
                "2",
                "--nonrelativistic",
                "--allow-time-limit",
                "--output",
                output.to_str().unwrap(),
            ])
            .unwrap();
            run(cli.fly).unwrap();
            let csv = std::fs::read_to_string(&output).unwrap();
            let row: Vec<_> = csv.lines().nth(1).unwrap().split(',').collect();
            let x = row[2].parse::<f64>().unwrap();
            if policy == "error" {
                assert_eq!(row[6], "outside");
                assert!((x - 1.0).abs() < 1e-11);
            } else {
                assert_eq!(row[6], "time_limit");
                assert!(x > 1.0);
            }
        }
    }
}

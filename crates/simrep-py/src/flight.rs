use std::{collections::BTreeMap, fs::File, io::BufReader, sync::Arc, time::Instant};

use numpy::{
    IntoPyArray, PyArray2, PyReadonlyArray2, PyReadonlyArray3, PyUntypedArrayMethods,
    ndarray::Array2,
};
use pyo3::{prelude::*, types::PyDict};
use serde::Deserialize;
use simrep_fly::{
    ELECTRIC_ACCELERATION, Field, FlightOptions, FlightResult, GridField, InstanceField,
    Integrator, Particle, Reconstruction, SPEED_OF_LIGHT, SplatReason, StepLimits,
    VoltageExpression, fly_many,
    gpu::{DeviceType, Gpu, TrajectorySampling},
    physics::{
        BeamOptions, BeamTermination, ChargeMode, DirectCharge, EnsembleOptions, GpuPic, PicMesh,
        SpaceCharge,
        collision::{CollisionModel, DiffusionStatistics, Gas, Hs1, Sds, fly_many_collisional},
        fly_beam, fly_ensemble,
        magnetic::{Coil, Electromagnetic, MagneticField, MagneticPa, PreparedMagneticField},
    },
    surface::SurfaceModel,
};
use simrep_solve::mg::{Geometry, Grid, Options};

use crate::{
    pa::{self, Result, error},
    refine::backends,
};

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum CoilInput {
    Segment {
        first_mm: [f64; 3],
        last_mm: [f64; 3],
        current_a: f64,
    },
    Loop {
        center_mm: [f64; 3],
        normal: [f64; 3],
        radius_mm: f64,
        current_a: f64,
    },
    Solenoid {
        first_mm: [f64; 3],
        last_mm: [f64; 3],
        radius_mm: f64,
        current_a: f64,
        turns: usize,
    },
}

#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Settings {
    device: String,
    backend: String,
    has_pa: bool,
    particle_format: String,
    duration_us: f64,
    tolerance: Option<f64>,
    initial_step_us: f64,
    min_step_us: f64,
    max_step_us: Option<f64>,
    max_steps: usize,
    max_grid_cells: f64,
    max_cyclotron_radians: f64,
    max_voltage_phase_radians: f64,
    relativistic: bool,
    global_cubic: bool,
    nearest_node_collisions: bool,
    cuts: Option<String>,
    cuts_json: Option<serde_json::Value>,
    surface_collisions: Option<String>,
    surface_backend: String,
    surface_sdf_resolution_divisor: u32,
    transparent_faces: [bool; 6],
    sample_every: Option<usize>,
    trajectory_indices: Option<Vec<usize>>,
    max_samples_per_ion: usize,
    integrator: String,
    origin_mm: [f64; 3],
    axes: [[f64; 3]; 3],
    scale: f64,
    bounds: Option<[[f64; 3]; 2]>,
    plane: Option<[f64; 4]>,
    voltage_expr: BTreeMap<usize, String>,
    parameters: BTreeMap<String, f64>,
    bfield: [f64; 3],
    magnetic_pa: Vec<String>,
    magnetic_pas: Vec<crate::physics_cmd::MagneticPaRun>,
    magnetic_support: Option<crate::physics_cmd::MagneticPaRun>,
    coils: Vec<CoilInput>,
    exact_coils: bool,
    collision: Option<String>,
    seed: u64,
    pressure_pa: Option<f64>,
    temperature_k: Option<f64>,
    gas_mass_amu: Option<f64>,
    gas_velocity: [f64; 3],
    cross_section_m2: f64,
    mean_free_path_mm: Option<f64>,
    collision_steps: f64,
    mbmr: Option<String>,
    gas_diameter_nm: f64,
    ion_diameter_nm: Option<f64>,
    mobility_stp: Option<f64>,
    diffusion: bool,
    space_charge: Option<String>,
    charge_mode: String,
    charge_amount: f64,
    beam_current_a: Option<f64>,
    charge_softening_mm: f64,
    pic_dims: [usize; 3],
    space_charge_step_fraction: Option<f64>,
}

impl Default for Settings {
    fn default() -> Self {
        let o = FlightOptions::default();
        Self {
            device: "auto".into(),
            backend: "vulkan".into(),
            has_pa: true,
            particle_format: "energy".into(),
            duration_us: 100.0,
            tolerance: None,
            initial_step_us: o.initial_step_us,
            min_step_us: o.min_step_us,
            max_step_us: None,
            max_steps: o.max_steps,
            max_grid_cells: o.step_limits.max_grid_cells,
            max_cyclotron_radians: 0.2,
            max_voltage_phase_radians: 0.2,
            relativistic: true,
            global_cubic: false,
            nearest_node_collisions: false,
            cuts: None,
            cuts_json: None,
            surface_collisions: None,
            surface_backend: "exact".into(),
            surface_sdf_resolution_divisor: 2,
            transparent_faces: [false; 6],
            sample_every: None,
            trajectory_indices: None,
            max_samples_per_ion: 10_000,
            integrator: "rk45".into(),
            origin_mm: [0.0; 3],
            axes: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            scale: 1.0,
            bounds: None,
            plane: None,
            voltage_expr: BTreeMap::new(),
            parameters: BTreeMap::new(),
            bfield: [0.0; 3],
            magnetic_pa: Vec::new(),
            magnetic_pas: Vec::new(),
            magnetic_support: None,
            coils: Vec::new(),
            exact_coils: false,
            collision: None,
            seed: 1,
            pressure_pa: None,
            temperature_k: None,
            gas_mass_amu: None,
            gas_velocity: [0.0; 3],
            cross_section_m2: 2.27e-18,
            mean_free_path_mm: None,
            collision_steps: 20.0,
            mbmr: None,
            gas_diameter_nm: 0.366,
            ion_diameter_nm: None,
            mobility_stp: None,
            diffusion: true,
            space_charge: None,
            charge_mode: "factor".into(),
            charge_amount: 1.0,
            beam_current_a: None,
            charge_softening_mm: 0.0,
            pic_dims: [33; 3],
            space_charge_step_fraction: None,
        }
    }
}

#[pyfunction]
#[pyo3(signature = (path, potentials=None, electrodes=None, metadata=None, settings=None, bases=None))]
pub fn read_particles<'py>(
    py: Python<'py>,
    path: String,
    potentials: Option<PyReadonlyArray3<'py, f64>>,
    electrodes: Option<PyReadonlyArray3<'py, bool>>,
    metadata: Option<&str>,
    settings: Option<&str>,
    bases: Option<Vec<(usize, PyReadonlyArray3<'py, f64>)>>,
) -> PyResult<Bound<'py, PyArray2<f64>>> {
    let file = match (potentials, electrodes, metadata) {
        (Some(p), Some(e), Some(m)) => Some(pa::owned(p, e, m)?),
        (None, None, None) => None,
        _ => {
            return Err(error(
                "FLY2 context requires potentials, electrodes and metadata together",
            ));
        }
    };
    let mut native_bases = Vec::new();
    for (number, values) in bases.unwrap_or_default() {
        let mut a = file
            .as_ref()
            .ok_or_else(|| error("FLY2 voltage bases require PA context"))?
            .array
            .clone();
        if values.shape() != [a.nz, a.ny, a.nx] {
            return Err(error("voltage basis shape differs from PA"));
        }
        a.potentials = values.as_array().iter().copied().collect();
        native_bases.push((number, a));
    }
    let rows = py
        .detach(|| -> Result<_> {
            let s: Settings = settings
                .map(serde_json::from_str)
                .transpose()?
                .unwrap_or_default();
            let array = file.map(|p| p.array);
            let instances = array
                .as_ref()
                .map(|a| simrep_io::Fly2Instance {
                    origin_mm: s.origin_mm,
                    axes: s.axes,
                    scale: s.scale,
                    spacing_mm: [a.dx_mm, a.dy_mm, a.dz_mm],
                    dimensions: [a.nx, a.ny, a.nz],
                    electrodes: Some(a.electrodes.clone().into()),
                    ..Default::default()
                })
                .into_iter()
                .collect();
            let reconstruction = if s.global_cubic {
                Reconstruction::GlobalCubic
            } else {
                Reconstruction::BoundaryAware
            };
            let mut expressions = Vec::new();
            for (&number, text) in &s.voltage_expr {
                let a = native_bases
                    .iter()
                    .find(|(n, _)| *n == number)
                    .ok_or_else(|| format!("electrode {number} has no unit-voltage basis"))?
                    .1
                    .clone();
                expressions.push((
                    GridField::with_reconstruction(a, reconstruction)?,
                    VoltageExpression::parse(text, &s.parameters)?,
                ));
            }
            let field = array
                .map(|a| -> Result<_> {
                    Ok(InstanceField::new(
                        GridField::with_reconstruction(a, reconstruction)?,
                        s.origin_mm,
                        s.axes,
                        s.scale,
                        s.bounds,
                        None,
                    )?
                    .with_voltage_expressions(expressions)?)
                })
                .transpose()?;
            let magnetic = magnetic(&s)?;
            let provider = |_, point| {
                let f = field.as_ref().ok_or_else(|| {
                    std::io::Error::other("FLY2 field queries require PA context")
                })?;
                let sample = f.sample(point, 0.);
                Ok(simrep_io::Fly2Field {
                    potential_v: sample.potential_v,
                    electric_v_per_mm: sample.electric_v_per_mm,
                    magnetic_gauss: magnetic.sample(point, 0.).magnetic_gauss,
                    electrode: matches!(f.status(point), Some(SplatReason::Electrode { .. })),
                })
            };
            let particles = simrep_io::read_particles_in(
                &path,
                &simrep_io::Fly2Context {
                    instances,
                    field: Some(&provider),
                    seed: s.seed,
                    ..Default::default()
                },
            )?;
            let n = particles.len();
            let values = particles
                .into_iter()
                .flat_map(|p| {
                    [
                        p.birth_time_us,
                        p.mass_amu,
                        p.charge_e,
                        p.position_mm[0],
                        p.position_mm[1],
                        p.position_mm[2],
                        p.direction[0],
                        p.direction[1],
                        p.direction[2],
                        p.energy_ev,
                        p.charge_weight,
                    ]
                })
                .collect::<Vec<_>>();
            Ok(Array2::from_shape_vec((n, 11), values)?)
        })
        .map_err(|e| error(format!("{path}: {e}")))?;
    Ok(rows.into_pyarray(py))
}

fn particles(rows: Array2<f64>, s: &Settings) -> Result<(Vec<Particle>, Vec<f64>)> {
    let energy = match s.particle_format.as_str() {
        "energy" => true,
        "velocity" => false,
        _ => return Err("particle_format must be energy or velocity".into()),
    };
    let cols = if energy { 10 } else { 9 };
    if rows.ncols() != cols && rows.ncols() != cols + 1 {
        return Err(format!(
            "{} particles require {cols} columns plus optional charge weight",
            s.particle_format
        )
        .into());
    }
    let mut particles = Vec::new();
    let mut weights = Vec::new();
    for row in rows.rows() {
        let weight = if row.len() > cols { row[cols] } else { 1.0 };
        if !weight.is_finite() || weight < 0.0 {
            return Err("charge weights must be finite and nonnegative".into());
        }
        weights.push(weight);
        let particle = if energy {
            let p = [row[3], row[4], row[5]];
            let d = [row[6], row[7], row[8]];
            if s.relativistic && s.collision.is_none() {
                Particle::from_relativistic_energy(row[1], row[2], p, row[9], d, row[0])?
            } else {
                Particle::from_energy(row[1], row[2], p, row[9], d, row[0])?
            }
        } else {
            Particle::new(
                row[0],
                row[1],
                [row[2], row[3], row[4]],
                [row[5], row[6], row[7]],
                row[8],
            )?
        };
        particles.push(particle);
    }
    Ok((particles, weights))
}

fn magnetic(s: &Settings) -> Result<MagneticField> {
    if s.bfield.iter().any(|v| !v.is_finite()) {
        return Err("bfield must be finite gauss".into());
    }
    let mut b = MagneticField {
        uniform_gauss: s.bfield,
        ..Default::default()
    };
    let placed =
        |pa: &crate::physics_cmd::MagneticPaRun, bounds, modifiers| -> Result<MagneticPa> {
            let file = simrep_io::read_pa(&pa.path)?;
            let mut field = MagneticPa::placed(
                file.array,
                pa.ng.unwrap_or(file.ng as f64),
                pa.origin_mm,
                pa.axes,
                pa.scale,
                bounds,
            )?;
            if modifiers && let Some(axis) = pa.polarity_flip_axis {
                field = field.with_polarity_flip_axis(axis)?;
            }
            if modifiers && let Some((axis, start, end)) = pa.angular_sector {
                field = field.with_angular_sector(axis, start, end)?;
            }
            Ok(field)
        };
    for pa in &s.magnetic_pas {
        b.arrays.push(placed(pa, s.bounds, true)?);
    }
    for path in &s.magnetic_pa {
        let f = simrep_io::read_pa(path)?;
        b.arrays.push(MagneticPa::placed(
            f.array,
            f.ng as f64,
            s.origin_mm,
            s.axes,
            s.scale,
            s.bounds,
        )?);
    }
    if let Some(pa) = &s.magnetic_support {
        b.support = Some(placed(pa, None, false)?);
    }
    for coil in &s.coils {
        match *coil {
            CoilInput::Segment {
                first_mm,
                last_mm,
                current_a,
            } => b.coils.push(Coil::segment(first_mm, last_mm, current_a)?),
            CoilInput::Loop {
                center_mm,
                normal,
                radius_mm,
                current_a,
            } => b
                .coils
                .push(Coil::hoop(center_mm, normal, radius_mm, current_a)?),
            CoilInput::Solenoid {
                first_mm,
                last_mm,
                radius_mm,
                current_a,
                turns,
            } => b.coils.extend(
                MagneticField::solenoid(first_mm, last_mm, radius_mm, current_a, turns)?.coils,
            ),
        }
    }
    Ok(b)
}

fn collision(s: &Settings) -> Result<Option<CollisionModel>> {
    let Some(kind) = s.collision.as_deref() else {
        return Ok(None);
    };
    let sds = kind == "sds";
    let gas = Gas {
        pressure_pa: s
            .pressure_pa
            .unwrap_or(if sds { 760.0 * 133.322 } else { 0.53 }),
        temperature_k: s.temperature_k.unwrap_or(if sds { 298.15 } else { 273.0 }),
        mass_amu: s.gas_mass_amu.unwrap_or(if sds { 28.94515 } else { 4.0 }),
        velocity_mm_per_us: s.gas_velocity,
    };
    let model = match kind {
        "hs1" => CollisionModel::Hs1(Hs1 {
            gas,
            cross_section_m2: s.cross_section_m2,
            mean_free_path_mm: s.mean_free_path_mm,
            steps_per_mean_free_time: s.collision_steps,
        }),
        "sds" => CollisionModel::Sds(Sds {
            gas,
            gas_diameter_nm: s.gas_diameter_nm,
            ion_diameter_nm: s.ion_diameter_nm,
            mobility_stp: s.mobility_stp,
            diffusion: s.diffusion,
            statistics: DiffusionStatistics::from_path(
                s.mbmr
                    .as_ref()
                    .ok_or("SDS requires mbmr='/path/to/mbmr.dat'")?,
            )?,
        }),
        _ => return Err("collision must be hs1 or sds".into()),
    };
    model.validate()?;
    Ok(Some(model))
}

type FlyArrays<'py> = (Bound<'py, PyDict>, Vec<Bound<'py, PyArray2<f64>>>, String);

#[pyfunction]
pub fn fly<'py>(
    py: Python<'py>,
    potentials: PyReadonlyArray3<'py, f64>,
    electrodes: PyReadonlyArray3<'py, bool>,
    metadata: &str,
    rows: PyReadonlyArray2<'py, f64>,
    bases: Vec<(usize, PyReadonlyArray3<'py, f64>)>,
    settings: &str,
) -> PyResult<FlyArrays<'py>> {
    let file = pa::owned(potentials, electrodes, metadata)?;
    let s: Settings = serde_json::from_str(settings).map_err(error)?;
    let rows = rows.as_array().to_owned();
    let mut native_bases = Vec::new();
    for (number, values) in bases {
        if values.shape() != [file.array.nz, file.array.ny, file.array.nx] {
            return Err(error("voltage basis shape differs from PA"));
        }
        let mut a = file.array.clone();
        a.potentials = values.as_array().iter().copied().collect();
        native_bases.push((number, a));
    }
    let (particles, results, terminations, report) = py.detach(|| -> Result<_> {
        let start = Instant::now();
        backends(&s.backend)?;
        let (particles, weights) = particles(rows, &s)?;
        let indices = s.trajectory_indices.clone().unwrap_or_else(|| {
            if s.sample_every.is_some() { (0..particles.len()).collect() } else { Vec::new() }
        });
        if indices.last().is_some_and(|&i| i >= particles.len()) || indices.windows(2).any(|w| w[0] >= w[1]) {
            return Err("trajectory_indices must be ordered unique zero-based particle indices".into());
        }
        let reconstruct = if s.global_cubic { Reconstruction::GlobalCubic } else { Reconstruction::BoundaryAware };
        let mut expressions = Vec::new();
        for (number, source) in &s.voltage_expr {
            let a = native_bases.iter().find(|(n, _)| n == number).ok_or_else(|| format!("electrode {number} has no unit-voltage basis"))?.1.clone();
            expressions.push((GridField::with_reconstruction(a, reconstruct)?, VoltageExpression::parse(source, &s.parameters)?));
        }
        let mut grid = GridField::with_reconstruction(file.array, reconstruct)?.with_transparent_faces(s.transparent_faces);
        if s.nearest_node_collisions { grid = grid.with_nearest_node_collisions(); }
        let plane = s.plane.map(|p| ([p[0],p[1],p[2]],p[3]));
        let mut field = InstanceField::new(grid, s.origin_mm, s.axes, s.scale, s.bounds, plane)?.with_voltage_expressions(expressions)?;
        if let Some(path) = &s.cuts {
            let surface = SurfaceModel::from_reader(BufReader::new(File::open(path)?))?
                .with_tolerance(s.tolerance.unwrap_or(1e-12) / s.scale)?;
            let surface = if s.surface_collisions.as_deref() == Some("insulators") { surface.select_collision_only(true) } else { surface };
            field = if s.surface_collisions.is_some() { field.with_collision_surface(Arc::new(surface))? } else { field.with_surface(Arc::new(surface))? };
            eprintln!("surface: {path} (explicit cuts)");
        }
        if let Some(value) = &s.cuts_json {
            let surface = SurfaceModel::from_reader(value.to_string().as_bytes())?
                .with_tolerance(s.tolerance.unwrap_or(1e-12) / s.scale)?;
            let surface = if s.surface_collisions.as_deref() == Some("insulators") { surface.select_collision_only(true) } else { surface };
            field = if s.surface_collisions.is_some() { field.with_collision_surface(Arc::new(surface))? } else { field.with_surface(Arc::new(surface))? };
        }
        if !matches!(s.surface_backend.as_str(), "exact" | "sdf") { return Err("surface_backend must be exact or sdf".into()); }
        if s.surface_backend == "sdf" { field = field.with_surface_sdf_resolution(s.surface_sdf_resolution_divisor)?; }
        let magnetic = magnetic(&s)?;
        let collision = collision(&s)?;
        let beam_mode = s.charge_mode == "beam";
        if !matches!(s.charge_mode.as_str(), "factor" | "coulombic" | "beam") { return Err("charge_mode must be factor, coulombic or beam".into()); }
        if beam_mode && (s.beam_current_a.is_none() || s.space_charge.is_some()) { return Err("beam needs beam_current_a and cannot be combined with space_charge".into()); }
        if !beam_mode && s.beam_current_a.is_some() { return Err("beam_current_a requires charge_mode='beam'".into()); }
        if beam_mode && s.device == "gpu" { return Err("leader-plane beam integration requires device='cpu' or 'auto'".into()); }
        if s.device == "gpu" && s.space_charge.as_deref() == Some("direct") { return Err("direct Coulomb physics requires device='cpu' or 'auto'".into()); }
        if (beam_mode || s.space_charge.is_some()) && collision.is_some() { return Err("space charge does not support collision models".into()); }
        if s.surface_collisions.is_some() && s.device == "gpu" && s.surface_backend != "sdf" { return Err("surface_collisions requires device=cpu or auto".into()); }
        let cpu_only = beam_mode || s.transparent_faces.contains(&true)
            || ((s.surface_collisions.is_some() || ((s.cuts.is_some() || s.cuts_json.is_some()) && s.space_charge.as_deref() != Some("pic")))
                && (s.surface_backend != "sdf" || s.device == "auto"));
        if s.device == "gpu" && cpu_only { return Err("surface collisions and transparent faces require device='cpu' or 'auto'".into()); }
        let has_magnetic = magnetic.uniform_gauss != [0.0; 3] || !magnetic.arrays.is_empty()
            || !magnetic.coils.is_empty() || magnetic.support.is_some();
        let hs = Hs1::default();
        let crossover = match s.space_charge.as_deref() {
            Some("pic") => Some(100_000),
            Some("direct") => None,
            Some(_) => return Err("space_charge must be direct or pic".into()),
            None if has_magnetic => None,
            None => match &collision {
                None => Some(4096),
                Some(CollisionModel::Hs1(model))
                    if !s.has_pa && s.bounds.is_none() && s.plane.is_none()
                        && s.duration_us == 50.0 && s.max_step_us == Some(0.05) && s.tolerance.is_none()
                        && s.integrator == "rk45" && s.initial_step_us == FlightOptions::default().initial_step_us
                        && model.gas.pressure_pa == hs.gas.pressure_pa
                        && model.gas.temperature_k == hs.gas.temperature_k
                        && model.gas.mass_amu == hs.gas.mass_amu
                        && model.gas.velocity_mm_per_us == hs.gas.velocity_mm_per_us
                        && model.cross_section_m2 == hs.cross_section_m2
                        && model.mean_free_path_mm == hs.mean_free_path_mm
                        && model.steps_per_mean_free_time == hs.steps_per_mean_free_time => Some(10_000),
                _ => None,
            },
        };
        let mut auto_reason = if cpu_only {
            "beam integration, surface collisions or transparent faces require CPU".to_owned()
        } else if s.space_charge.as_deref() == Some("direct") {
            "direct Coulomb physics requires CPU".to_owned()
        } else if has_magnetic && crossover.is_none() {
            "magnetic field: GPU magnetic flight not selected automatically".to_owned()
        } else if let Some(threshold) = crossover {
            format!("{} ions {} measured crossover {threshold}", particles.len(),
                if particles.len() >= threshold { ">=" } else { "<" })
        } else {
            "no measured GPU crossover for this collision profile".to_owned()
        };
        let gpu = match s.device.as_str() {
            "cpu" => None,
            "auto" if cpu_only || crossover.is_none_or(|n| particles.len() < n) => None,
            "gpu" | "auto" => {
                let gpu = pollster::block_on(Gpu::new(backends(&s.backend)?))?;
                if s.device == "gpu" && gpu.is_none() { return Err("no GPU adapter found; select device='cpu'".into()); }
                if let Some(gpu) = &gpu {
                    if s.device == "gpu" { crate::require_hardware_gpu(&gpu.info)?; }
                    if gpu.info.device_type == DeviceType::Cpu {
                        auto_reason = format!("adapter {} is a CPU/software adapter", gpu.info.name);
                    } else if !gpu.has_f64() {
                        auto_reason = format!("adapter {} lacks native f64", gpu.info.name);
                    } else if collision.is_some() && gpu.info.name != "NVIDIA GeForce RTX 3070" {
                        auto_reason = format!("collision crossover was measured on NVIDIA GeForce RTX 3070, not {}", gpu.info.name);
                    }
                } else {
                    auto_reason = "no adapter available for the requested backend".into();
                }
                gpu.filter(|g| s.device == "gpu" || (g.info.device_type != DeviceType::Cpu && g.has_f64()
                    && (collision.is_none() || g.info.name == "NVIDIA GeForce RTX 3070")))
            }
            _ => return Err("device must be auto, cpu or gpu".into()),
        };
        if s.device == "auto" {
            eprintln!("auto: {} ({auto_reason})", if gpu.is_some() { "GPU" } else { "CPU" });
        }
        if let Some(gpu) = &gpu {
            eprintln!("Adapter: {} ({:?}, {:?})", gpu.info.name, gpu.info.backend, gpu.info.device_type);
            if s.space_charge.as_deref() == Some("pic") {
                crate::report_gpu_residual(gpu.has_f64());
            } else if !gpu.has_f64() {
                eprintln!("GPU precision: f32 (adapter lacks native f64; tolerance floor 2e-7, two-float TOF)");
            }
        }
        if collision.is_some() && s.relativistic {
            eprintln!("dynamics: nonrelativistic (collision models require nonrelativistic integration)");
        }
        let o = FlightOptions {
            integrator: match s.integrator.as_str() { "rk45" => Integrator::Rk45, "boris" => Integrator::Boris, _ => return Err("integrator must be rk45 or boris".into()) },
            duration_us: s.duration_us, initial_step_us: s.initial_step_us.min(s.max_step_us.unwrap_or(f64::MAX)),
            min_step_us: s.min_step_us, max_step_us: s.max_step_us.unwrap_or(f64::MAX),
            max_steps: s.max_steps, automatic_tolerance: s.tolerance.is_none(),
            relative_tolerance: s.tolerance.unwrap_or(1e-12),
            position_tolerance_mm: s.tolerance.unwrap_or(1e-12), velocity_tolerance_mm_per_us: s.tolerance.unwrap_or(1e-12),
            relativistic: s.relativistic && collision.is_none(), sample_every: s.sample_every,
            step_limits: StepLimits { max_grid_cells: s.max_grid_cells, max_cyclotron_radians: s.max_cyclotron_radians, max_voltage_phase_radians: s.max_voltage_phase_radians },
            ..FlightOptions::default()
        };
        let setup_ms;
        let compute_ms;
        let results;
        let mut terminations = None;
        let mut coil_map_calibration = Vec::new();
        if let Some(gpu) = gpu.as_ref().filter(|_| s.space_charge.is_none()) {
            let mut resident = if has_magnetic || collision.is_some() {
                gpu.prepare_physics(&field, &[], &magnetic, collision.as_ref(), s.seed)?
            } else { gpu.prepare(&field, &[])? };
            setup_ms = start.elapsed().as_secs_f64() * 1000.0;
            let mut gpu_options = o;
            gpu_options.sample_every = None;
            let r = if let Some(every) = s.sample_every {
                resident.fly_sampled(&particles, &gpu_options, &TrajectorySampling {
                    ion_indices: &indices, every, max_samples_per_ion: s.max_samples_per_ion,
                })?
            } else { resident.fly(&particles, &gpu_options)? };
            compute_ms = r.timings.fly.as_secs_f64() * 1000.0;
            results = r.results;
        } else {
            let mut space = match s.space_charge.as_deref() {
                None => None,
                Some("direct") => Some(SpaceCharge::Direct(DirectCharge { softening_mm: s.charge_softening_mm })),
                Some("pic") => {
                    let bounds = s.bounds.ok_or("PIC requires bounds covering the full ensemble")?;
                    let dims = s.pic_dims;
                    if dims.iter().any(|&n| n < 3) { return Err("PIC dimensions must each be >=3".into()); }
                    let count = dims.into_iter().try_fold(1usize, usize::checked_mul).ok_or("PIC mesh size overflow")?;
                    let h = std::array::from_fn(|a| (bounds[1][a] - bounds[0][a]) / (dims[a]-1) as f64);
                    let fixed = (0..count).map(|i| {
                        let c = [i % dims[0], i / dims[0] % dims[1], i / (dims[0]*dims[1])];
                        let p = std::array::from_fn(|a| bounds[0][a] + c[a] as f64 * h[a]);
                        (0..3).any(|a| c[a] == 0 || c[a]+1 == dims[a]) || matches!(field.status(p), Some(SplatReason::Electrode { .. } | SplatReason::SurfaceElectrode { .. }))
                    }).collect();
                    let mesh = PicMesh::new(Grid { dims, spacing: h, geometry: Geometry::Planar }, bounds[0], fixed, Options { relative_tolerance: s.tolerance.unwrap_or(1e-12), ..Default::default() })?;
                    Some(if let Some(gpu) = &gpu {
                        let solver = simrep_solve::gpu::Gpu::from_device(gpu.device().clone(), gpu.queue().clone(), gpu.info.clone());
                        SpaceCharge::GpuPic(Box::new(GpuPic::new(solver, mesh, particles.len())?))
                    } else { SpaceCharge::Pic(Box::new(mesh)) })
                }
                _ => return Err("space_charge must be direct or pic".into()),
            };
            let prepared = if s.exact_coils { PreparedMagneticField::exact(&magnetic) } else { magnetic.prepare_coil_map(s.bounds)? };
            coil_map_calibration = prepared.calibration().to_vec();
            let field = Electromagnetic { electric: field, magnetic: prepared };
            setup_ms = start.elapsed().as_secs_f64() * 1000.0;
            let compute = Instant::now();
            results = if beam_mode {
                let report = fly_beam(&field, &particles, &BeamOptions {
                    flight: o, current_a: s.beam_current_a.unwrap(), charge_weights: weights,
                    softening_mm: s.charge_softening_mm,
                    space_charge_step_fraction: s.space_charge_step_fraction.unwrap_or(1.0),
                    ..BeamOptions::default()
                })?;
                terminations = Some(report.terminations);
                report.ensemble.results
            } else if let Some(space) = &mut space {
                let charge_mode = match s.charge_mode.as_str() { "factor" => ChargeMode::Factor(s.charge_amount), "coulombic" => ChargeMode::Coulombic(s.charge_amount), _ => return Err("charge_mode must be factor or coulombic".into()) };
                fly_ensemble(&field, &particles, &EnsembleOptions { flight: o, charge_mode, charge_weights: weights, space_charge_step_fraction: s.space_charge_step_fraction.unwrap_or(0.1) }, space)?.results
            } else if let Some(model) = &collision {
                fly_many_collisional(&field, &particles, &o, model, s.seed)?
            } else {
                fly_many(&field, &particles, &o)?
            };
            compute_ms = compute.elapsed().as_secs_f64() * 1000.0;
        }
        let mut results = results;
        if s.trajectory_indices.is_some() {
            for (index, result) in results.iter_mut().enumerate() {
                if indices.binary_search(&index).is_err() { result.trajectory = None; }
            }
        }
        let report = serde_json::json!({"compute_ms":compute_ms,"setup_ms":setup_ms,"total_ms":start.elapsed().as_secs_f64()*1000.0,"device":if gpu.is_some() {"gpu"} else {"cpu"},"relativistic":o_relativistic(&s),"surface_backend":s.surface_backend,"precision":gpu.as_ref().map_or("f64", |g| if g.has_f64() { "f64" } else { "f32" }),"executed":{"device":if gpu.is_some(){"gpu"}else{"cpu"},"precision":gpu.as_ref().map_or("f64",|g|if g.has_f64(){"f64"}else{"f32"}),"surface_backend":if s.cuts.is_some()||s.cuts_json.is_some(){s.surface_backend.as_str()}else{"staircase"}},"coil_map_calibration":coil_map_calibration,"trajectory_indices":indices}).to_string();
        Ok((particles, results, terminations, report))
    }).map_err(error)?;
    result_arrays(
        py,
        &particles,
        results,
        terminations,
        report,
        o_relativistic(&s),
    )
}

fn o_relativistic(s: &Settings) -> bool {
    s.relativistic && s.collision.is_none()
}

fn reason(r: SplatReason) -> &'static str {
    match r {
        SplatReason::Electrode { .. } | SplatReason::SurfaceElectrode { .. } => "electrode",
        SplatReason::OutsideGrid => "outside",
        SplatReason::StopPlane => "plane",
        SplatReason::TimeLimit => "time_limit",
        SplatReason::StepLimit => "step_limit",
        SplatReason::StepUnderflow => "step_underflow",
        SplatReason::NonFinite => "nonfinite",
    }
}

fn result_arrays<'py>(
    py: Python<'py>,
    particles: &[Particle],
    results: Vec<FlightResult>,
    terminations: Option<Vec<BeamTermination>>,
    report: String,
    relativistic: bool,
) -> PyResult<FlyArrays<'py>> {
    let dict = PyDict::new(py);
    dict.set_item(
        "ion",
        (1..=results.len()).collect::<Vec<_>>().into_pyarray(py),
    )?;
    dict.set_item(
        "tof_us",
        results
            .iter()
            .map(|r| r.tof_us)
            .collect::<Vec<_>>()
            .into_pyarray(py),
    )?;
    for (axis, name) in ["x_mm", "y_mm", "z_mm"].into_iter().enumerate() {
        dict.set_item(
            name,
            results
                .iter()
                .map(|r| r.final_state.position_mm[axis])
                .collect::<Vec<_>>()
                .into_pyarray(py),
        )?;
    }
    for (axis, name) in ["vx_mm_per_us", "vy_mm_per_us", "vz_mm_per_us"]
        .into_iter()
        .enumerate()
    {
        dict.set_item(
            name,
            results
                .iter()
                .map(|r| r.final_state.velocity_mm_per_us[axis])
                .collect::<Vec<_>>()
                .into_pyarray(py),
        )?;
    }
    let energies = particles
        .iter()
        .zip(&results)
        .map(|(p, r)| {
            let v2 = r
                .final_state
                .velocity_mm_per_us
                .iter()
                .map(|v| v * v)
                .sum::<f64>();
            if relativistic {
                let inverse_gamma = (1.0 - v2 / SPEED_OF_LIGHT.powi(2)).sqrt();
                p.mass_amu * v2 / (ELECTRIC_ACCELERATION * inverse_gamma * (1.0 + inverse_gamma))
            } else {
                p.mass_amu * v2 / (2.0 * ELECTRIC_ACCELERATION)
            }
        })
        .collect::<Vec<_>>();
    dict.set_item("ke_ev", energies.into_pyarray(py))?;
    dict.set_item(
        "time_us",
        results
            .iter()
            .map(|r| r.final_state.time_us)
            .collect::<Vec<_>>()
            .into_pyarray(py),
    )?;
    dict.set_item(
        "accepted_steps",
        results
            .iter()
            .map(|r| r.accepted_steps)
            .collect::<Vec<_>>()
            .into_pyarray(py),
    )?;
    dict.set_item(
        "rejected_steps",
        results
            .iter()
            .map(|r| r.rejected_steps)
            .collect::<Vec<_>>()
            .into_pyarray(py),
    )?;
    dict.set_item(
        "reason",
        results
            .iter()
            .enumerate()
            .map(|(i, r)| match terminations.as_ref().map(|t| t[i]) {
                Some(BeamTermination::Diverged) => "beam_diverged",
                Some(BeamTermination::LeaderStopped) => "beam_leader_stopped",
                _ => reason(r.splat_reason),
            })
            .collect::<Vec<_>>(),
    )?;
    dict.set_item(
        "electrode_number",
        results
            .iter()
            .map(|r| match r.splat_reason {
                SplatReason::SurfaceElectrode { number } => number,
                _ => 0,
            })
            .collect::<Vec<_>>()
            .into_pyarray(py),
    )?;
    let trajectories = results
        .into_iter()
        .filter_map(|r| r.trajectory)
        .map(|states| {
            let n = states.len();
            let v = states
                .into_iter()
                .flat_map(|s| {
                    [
                        s.time_us,
                        s.position_mm[0],
                        s.position_mm[1],
                        s.position_mm[2],
                        s.velocity_mm_per_us[0],
                        s.velocity_mm_per_us[1],
                        s.velocity_mm_per_us[2],
                    ]
                })
                .collect();
            Ok(Array2::from_shape_vec((n, 7), v)
                .map_err(error)?
                .into_pyarray(py))
        })
        .collect::<PyResult<_>>()?;
    Ok((dict, trajectories, report))
}

use std::{fs::File, io::BufReader, path::PathBuf};

use clap::{Args, ValueEnum};
use serde::{Deserialize, Serialize};
use simrep_fly::{
    FlyError, Integrator,
    physics::{
        collision::{CollisionModel, DiffusionStatistics, Gas, Hs1, Sds},
        magnetic::{Coil, MagneticField, MagneticPa},
    },
};
use simrep_io::read_pa;
use simrep_solve::gpu::GpuResult;
use simrep_solve::magnetostatics::{FieldFile as MagneticFieldFile, Formulation};

#[derive(Clone, Copy, ValueEnum, Serialize)]
pub enum Integration {
    Rk45,
    Boris,
}
impl From<Integration> for Integrator {
    fn from(value: Integration) -> Self {
        match value {
            Integration::Rk45 => Self::Rk45,
            Integration::Boris => Self::Boris,
        }
    }
}
#[derive(Clone, Copy, ValueEnum, Serialize)]
pub enum Collisions {
    Hs1,
    Sds,
}

#[derive(Args, Serialize)]
pub struct PhysicsOptions {
    /// Magnetic PA, using its header ng and the electric PA placement.
    #[arg(long)]
    magnetic_pa: Vec<PathBuf>,
    /// Constant workbench Bx,By,Bz in gauss.
    #[arg(long, value_parser = super::fly_cmd::numbers::<3>, allow_hyphen_values = true)]
    bfield: Option<[f64; 3]>,
    /// JSON array of tagged segment/loop/solenoid coil definitions.
    #[arg(long)]
    bfield_coil: Option<PathBuf>,
    /// Particle integration method.
    #[arg(long, value_enum, default_value = "rk45")]
    pub integrator: Integration,
    /// Fixed Boris step or adaptive initial RK45/SDS step, in microseconds.
    #[arg(long, default_value_t = 0.001)]
    pub step_us: f64,
    /// Gas collision model (disabled if omitted).
    #[arg(long, value_enum)]
    collision: Option<Collisions>,
    /// Deterministic random seed for particle/collision sampling.
    #[arg(long, default_value_t = 1)]
    seed: u64,
    /// Background gas pressure in pascals.
    #[arg(long)]
    pressure_pa: Option<f64>,
    /// Background gas temperature in kelvin.
    #[arg(long)]
    temperature_k: Option<f64>,
    /// Background gas molecular mass in amu.
    #[arg(long)]
    gas_mass_amu: Option<f64>,
    /// Background gas velocity vx,vy,vz in mm/us.
    #[arg(long, default_value = "0,0,0", value_parser = super::fly_cmd::numbers::<3>, allow_hyphen_values = true)]
    gas_velocity: [f64; 3],
    /// HS1 collision cross section in square meters.
    #[arg(long, default_value_t = 2.27e-18)]
    cross_section_m2: f64,
    /// Override HS1 mean free path in mm.
    #[arg(long)]
    mean_free_path_mm: Option<f64>,
    /// Minimum integration steps per mean free collision time.
    #[arg(long, default_value_t = 20.0)]
    collision_steps: f64,
    /// Runtime path to SIMION example collision_sds/mbmr.dat (not bundled).
    #[arg(long)]
    mbmr: Option<PathBuf>,
    /// SDS background gas diameter in nm.
    #[arg(long, default_value_t = 0.366)]
    gas_diameter_nm: f64,
    /// SDS ion diameter in nm.
    #[arg(long)]
    ion_diameter_nm: Option<f64>,
    /// SDS reduced mobility at STP in 1e-4 m^2/(V s).
    #[arg(long)]
    mobility_stp: Option<f64>,
    /// Disable random SDS diffusion.
    #[arg(long)]
    no_diffusion: bool,
    /// Treat the requested duration as a successful terminal event.
    #[arg(long)]
    pub allow_time_limit: bool,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PhysicsRun {
    pub uniform_gauss: [f64; 3],
    pub coils: Vec<CoilRun>,
    pub magnetic_pas: Vec<MagneticPaRun>,
    pub magnetic_support: Option<MagneticPaRun>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MagneticPaRun {
    pub path: PathBuf,
    #[serde(default)]
    pub origin_mm: [f64; 3],
    #[serde(default = "identity")]
    pub axes: [[f64; 3]; 3],
    #[serde(default = "one")]
    pub scale: f64,
    pub ng: Option<f64>,
    pub polarity_flip_axis: Option<usize>,
    /// axis,start_degrees,end_degrees, about the workbench origin.
    pub angular_sector: Option<(usize, f64, f64)>,
}
fn identity() -> [[f64; 3]; 3] {
    [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]
}
fn one() -> f64 {
    1.0
}
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CoilRun {
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
impl PhysicsOptions {
    pub fn prepare(
        &self,
        mut run: PhysicsRun,
        origin: [f64; 3],
        axes: [[f64; 3]; 3],
        scale: f64,
        bounds: Option<[[f64; 3]; 2]>,
    ) -> GpuResult<(MagneticField, Option<CollisionModel>, u64)> {
        if let Some(path) = &self.bfield_coil {
            let coils: Vec<CoilRun> = serde_json::from_reader(BufReader::new(File::open(path)?))?;
            run.coils.extend(coils);
        }
        if let Some(b) = self.bfield {
            run.uniform_gauss = b;
        }
        if run.uniform_gauss.iter().any(|v| !v.is_finite()) {
            return Err(FlyError("uniform magnetic field must be finite").into());
        }
        let mut magnetic = MagneticField {
            uniform_gauss: run.uniform_gauss,
            ..Default::default()
        };
        if let Some(pa) = run.magnetic_support {
            let file = read_pa(pa.path)?;
            magnetic.support = Some(MagneticPa::placed(
                file.array,
                pa.ng.unwrap_or(file.ng as f64),
                pa.origin_mm,
                pa.axes,
                pa.scale,
                None,
            )?);
        }
        for coil in run.coils {
            match coil {
                CoilRun::Segment {
                    first_mm,
                    last_mm,
                    current_a,
                } => {
                    magnetic
                        .coils
                        .push(Coil::segment(first_mm, last_mm, current_a)?);
                }
                CoilRun::Loop {
                    center_mm,
                    normal,
                    radius_mm,
                    current_a,
                } => {
                    magnetic
                        .coils
                        .push(Coil::hoop(center_mm, normal, radius_mm, current_a)?);
                }
                CoilRun::Solenoid {
                    first_mm,
                    last_mm,
                    radius_mm,
                    current_a,
                    turns,
                } => {
                    magnetic.coils.extend(
                        MagneticField::solenoid(first_mm, last_mm, radius_mm, current_a, turns)?
                            .coils,
                    );
                }
            }
        }
        run.magnetic_pas
            .extend(self.magnetic_pa.iter().map(|path| MagneticPaRun {
                path: path.clone(),
                origin_mm: origin,
                axes,
                scale,
                ng: None,
                polarity_flip_axis: None,
                angular_sector: None,
            }));
        for pa in run.magnetic_pas {
            let file = read_pa(&pa.path)?;
            let mut field = MagneticPa::placed(
                file.array,
                pa.ng.unwrap_or(file.ng as f64),
                pa.origin_mm,
                pa.axes,
                pa.scale,
                bounds,
            )?;
            if let Some(axis) = pa.polarity_flip_axis {
                field = field.with_polarity_flip_axis(axis)?;
            }
            if let Some((axis, start, end)) = pa.angular_sector {
                field = field.with_angular_sector(axis, start, end)?;
            }
            let sidecar = pa.path.with_extension("mag.json");
            if sidecar.is_file() {
                let header: MagneticFieldFile =
                    serde_json::from_reader(BufReader::new(File::open(sidecar)?))?;
                if header.version != 1
                    || header.formulation != Formulation::Scalar
                    || header.components.is_some()
                {
                    return Err("vector-potential PA requires --magnetic-vector-potential with its version 1 sidecar".into());
                }
                field = field.with_material_cells(header.solid_cells)?;
            }
            magnetic.arrays.push(field);
        }
        let collision = match self.collision {
            None => None,
            Some(kind) => {
                let sds = matches!(kind, Collisions::Sds);
                let gas = Gas {
                    pressure_pa: self.pressure_pa.unwrap_or(if sds {
                        760.0 * 133.322
                    } else {
                        0.53
                    }),
                    temperature_k: self
                        .temperature_k
                        .unwrap_or(if sds { 298.15 } else { 273.0 }),
                    mass_amu: self
                        .gas_mass_amu
                        .unwrap_or(if sds { 28.94515 } else { 4.0 }),
                    velocity_mm_per_us: self.gas_velocity,
                };
                let model = match kind {
                    Collisions::Hs1 => CollisionModel::Hs1(Hs1 {
                        gas,
                        cross_section_m2: self.cross_section_m2,
                        mean_free_path_mm: self.mean_free_path_mm,
                        steps_per_mean_free_time: self.collision_steps,
                    }),
                    Collisions::Sds => CollisionModel::Sds(Sds {
                        gas,
                        gas_diameter_nm: self.gas_diameter_nm,
                        ion_diameter_nm: self.ion_diameter_nm,
                        mobility_stp: self.mobility_stp,
                        diffusion: !self.no_diffusion,
                        statistics: DiffusionStatistics::from_path(
                            self.mbmr
                                .as_ref()
                                .ok_or("SDS requires --mbmr /path/to/mbmr.dat")?,
                        )?,
                    }),
                };
                model.validate()?;
                Some(model)
            }
        };
        Ok((magnetic, collision, self.seed))
    }
}

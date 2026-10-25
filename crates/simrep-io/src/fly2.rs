//! Sandboxed execution of SIMION FLY2 particle definitions.
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use simrep_lua::mlua::{self, ChunkMode, Table, Value};

use crate::particles::{ParticleDefinition, invalid};

/// Placement of a PA instance. Axes are columns, as in the flight APIs.
#[derive(Clone, Debug)]
pub struct Fly2Instance {
    pub origin_mm: [f64; 3],
    pub axes: [[f64; 3]; 3],
    pub scale: f64,
    pub spacing_mm: [f64; 3],
    pub dimensions: [usize; 3],
    /// PA coordinates of the working origin, in unscaled millimeters.
    pub working_origin_mm: [f64; 3],
    /// Optional x-fastest PA node mask for `pa:electrode`, not collision cells.
    pub electrodes: Option<Arc<[bool]>>,
}

impl Default for Fly2Instance {
    fn default() -> Self {
        Self {
            origin_mm: [0.; 3],
            axes: [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
            scale: 1.,
            spacing_mm: [1.; 3],
            dimensions: [1; 3],
            working_origin_mm: [0.; 3],
            electrodes: None,
        }
    }
}

impl Fly2Instance {
    fn position(&self, p: [f64; 3], working: bool) -> [f64; 3] {
        std::array::from_fn(|a| {
            self.origin_mm[a]
                + (0..3)
                    .map(|b| {
                        self.axes[b][a]
                            * self.scale
                            * (p[b] * self.spacing_mm[b]
                                - if working {
                                    0.
                                } else {
                                    self.working_origin_mm[b]
                                })
                    })
                    .sum::<f64>()
        })
    }

    fn direction(&self, p: [f64; 3]) -> [f64; 3] {
        std::array::from_fn(|a| (0..3).map(|b| self.axes[b][a] * p[b]).sum())
    }

    fn inverse(&self, p: [f64; 3]) -> [f64; 3] {
        std::array::from_fn(|a| {
            ((0..3)
                .map(|b| self.axes[a][b] * (p[b] - self.origin_mm[b]))
                .sum::<f64>()
                / self.scale
                + self.working_origin_mm[a])
                / self.spacing_mm[a]
        })
    }
}

/// Values at a workbench point; E is V/mm, B is gauss, potential is volts.
#[derive(Clone, Copy, Debug, Default)]
pub struct Fly2Field {
    pub potential_v: f64,
    pub electric_v_per_mm: [f64; 3],
    pub magnetic_gauss: [f64; 3],
    pub electrode: bool,
}

/// Field callback receives a one-based instance number (zero for the whole
/// workbench) and a position in workbench millimeters.
pub type Fly2FieldProvider<'a> =
    dyn Fn(usize, [f64; 3]) -> io::Result<Fly2Field> + Send + Sync + 'a;

/// Workbench data supplied explicitly to a FLY2 definition.
#[derive(Default)]
pub struct Fly2Context<'a> {
    pub instances: Vec<Fly2Instance>,
    pub field: Option<&'a Fly2FieldProvider<'a>>,
    /// Native random stream seed. Zero and one are distinct, repeatable seeds.
    pub seed: u64,
    /// Additional read-only Lua search roots (typically the SIMION examples tree).
    pub lua_roots: Vec<PathBuf>,
}

fn lua_error(e: mlua::Error) -> io::Error {
    invalid(format!("FLY2 Lua: {e}"))
}

/// Execute a context-free FLY2 string. Includes require a file path.
pub fn parse_fly2(input: &str) -> io::Result<Vec<ParticleDefinition>> {
    execute(input, None, &Fly2Context::default())
}

pub fn read_fly2(path: impl AsRef<Path>) -> io::Result<Vec<ParticleDefinition>> {
    read_fly2_in(path, &Fly2Context::default())
}

pub fn read_fly2_in(
    path: impl AsRef<Path>,
    context: &Fly2Context<'_>,
) -> io::Result<Vec<ParticleDefinition>> {
    let path = path.as_ref().canonicalize()?;
    execute(&std::fs::read_to_string(&path)?, Some(&path), context)
}

fn execute(
    input: &str,
    path: Option<&Path>,
    context: &Fly2Context<'_>,
) -> io::Result<Vec<ParticleDefinition>> {
    for instance in &context.instances {
        if let Some(mask) = &instance.electrodes
            && instance
                .dimensions
                .into_iter()
                .try_fold(1usize, usize::checked_mul)
                != Some(mask.len())
        {
            return Err(invalid("FLY2 context electrode mask dimensions differ"));
        }
        let dot = |a: [f64; 3], b: [f64; 3]| (0..3).map(|i| a[i] * b[i]).sum::<f64>();
        if !instance.scale.is_finite()
            || instance.scale <= 0.
            || instance.dimensions.contains(&0)
            || instance
                .spacing_mm
                .iter()
                .any(|h| !h.is_finite() || *h <= 0.)
            || instance
                .origin_mm
                .iter()
                .chain(&instance.working_origin_mm)
                .chain(instance.axes.iter().flatten())
                .any(|v| !v.is_finite())
            || (0..3).any(|a| {
                (0..3).any(|b| {
                    (dot(instance.axes[a], instance.axes[b]) - f64::from(a == b)).abs() > 1e-10
                })
            })
        {
            return Err(invalid(
                "FLY2 context has an invalid PA placement or spacing",
            ));
        }
    }
    let lua = simrep_lua::sandbox("FLY2").map_err(lua_error)?;
    let output = Mutex::new(Vec::new());
    let seed = Mutex::new(context.seed);
    let directories = Mutex::new(
        path.and_then(Path::parent)
            .map(Path::to_path_buf)
            .into_iter()
            .collect::<Vec<_>>(),
    );
    let mut roots = context.lua_roots.clone();
    if let Some(parent) = path.and_then(Path::parent) {
        roots.push(parent.to_path_buf());
        if let Some(examples) = parent
            .ancestors()
            .find(|p| p.file_name().is_some_and(|n| n == "examples"))
        {
            roots.push(examples.to_path_buf());
            roots.push(examples.parent().unwrap().join("lib/lua"));
        }
    }
    let roots = roots
        .into_iter()
        .filter_map(|p| p.canonicalize().ok())
        .collect::<Vec<_>>();
    let modules = lua.create_table().map_err(lua_error)?;
    lua.scope(|scope| {
        let globals = lua.globals();
        globals.set(
            "rand",
            scope.create_function(|_, ()| {
                // SplitMix64: platform-independent and separate from process-global libc rand.
                let mut seed = seed.lock().unwrap();
                *seed = seed.wrapping_add(0x9e3779b97f4a7c15);
                let mut z = *seed;
                z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
                z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
                Ok(((z ^ (z >> 31)) >> 11) as f64 / (1u64 << 53) as f64)
            })?,
        )?;
        globals.set(
            "_seed",
            scope.create_function(|_, value: u64| {
                *seed.lock().unwrap() = value;
                Ok(())
            })?,
        )?;
        globals.set(
            "_emit",
            scope.create_function(|_, (coordinates, row): (i64, Table)| {
                let v = |key| -> mlua::Result<[f64; 3]> {
                    let t: Table = row.get(key)?;
                    Ok([t.get(1)?, t.get(2)?, t.get(3)?])
                };
                let mut particle = ParticleDefinition {
                    birth_time_us: row.get("tob")?,
                    mass_amu: row.get("mass")?,
                    charge_e: row.get("charge")?,
                    position_mm: v("position")?,
                    direction: v("direction")?,
                    energy_ev: row.get("ke")?,
                    charge_weight: row.get("cwf")?,
                    color: row.get("color")?,
                };
                if coordinates != 0 {
                    let instance = coordinates
                        .checked_sub(1)
                        .and_then(|i| usize::try_from(i).ok())
                        .and_then(|i| context.instances.get(i))
                        .ok_or_else(|| {
                            mlua::Error::external(format!(
                                "missing workbench instance context for coordinates={coordinates}"
                            ))
                        })?;
                    particle.position_mm = instance.position(particle.position_mm, true);
                    particle.direction = instance.direction(particle.direction);
                }
                particle.validate().map_err(mlua::Error::external)?;
                let mut output = output.lock().unwrap();
                if output.len() >= 10_000_000 {
                    return Err(mlua::Error::external("FLY2 particle limit exceeded"));
                }
                output.push(particle);
                Ok(())
            })?,
        )?;
        globals.set(
            "_check_coordinates",
            scope.create_function(|_, coordinates: usize| {
                if coordinates > context.instances.len() {
                    return Err(mlua::Error::external(format!(
                        "missing workbench instance context for coordinates={coordinates}"
                    )));
                }
                Ok(())
            })?,
        )?;
        let loader = scope.create_function(|lua, (name, cached): (String, bool)| {
            if cached {
                let previous: Value = modules.get(name.as_str())?;
                if previous != Value::Nil {
                    return Ok(previous);
                }
            }
            let file = if cached {
                format!("{}.lua", name.replace('.', "/"))
            } else {
                name.clone()
            };
            let candidate = directories.lock().unwrap().last().map(|p| p.join(&file));
            let resolved = candidate
                .into_iter()
                .chain(roots.iter().map(|p| p.join(&file)))
                .filter_map(|p| p.canonicalize().ok())
                .find(|p| {
                    p.extension().is_some_and(|e| e == "lua" || e == "fly2")
                        && roots.iter().any(|r| p.starts_with(r))
                })
                .ok_or_else(|| {
                    mlua::Error::external(format!(
                        "Lua module {name:?} not found in allowed read-only roots"
                    ))
                })?;
            if cached {
                modules.set(name.as_str(), true)?;
            }
            let source = std::fs::read_to_string(&resolved).map_err(mlua::Error::external)?;
            directories
                .lock()
                .unwrap()
                .push(resolved.parent().unwrap().to_path_buf());
            let result = lua
                .load(source.trim_start_matches('\u{feff}'))
                .set_mode(ChunkMode::Text)
                .set_name(resolved.to_string_lossy())
                .eval::<Value>();
            directories.lock().unwrap().pop();
            let value = result?;
            let value = if value == Value::Nil {
                Value::Boolean(true)
            } else {
                value
            };
            if cached {
                modules.set(name.as_str(), value.clone())?;
            }
            Ok(value)
        })?;
        globals.set("_read_lua", loader)?;
        let instances = lua.create_table()?;
        for (index, instance) in context.instances.iter().enumerate() {
            let t = lua.create_table()?;
            let pa = lua.create_table()?;
            for (key, value) in ["nx", "ny", "nz"].into_iter().zip(instance.dimensions) {
                pa.set(key, value)?;
            }
            for (key, value) in ["dx_mm", "dy_mm", "dz_mm"]
                .into_iter()
                .zip(instance.spacing_mm)
            {
                pa.set(key, value)?;
            }
            for (key, value) in ["ox", "oy", "oz"]
                .into_iter()
                .zip(instance.working_origin_mm)
            {
                t.set(key, value)?;
            }
            for (key, value) in ["x", "y", "z"].into_iter().zip(instance.origin_mm) {
                t.set(key, value)?;
            }
            t.set("scale", instance.scale)?;
            let sample = move |p: [f64; 3]| {
                context.field.ok_or_else(|| {
                    mlua::Error::external("missing field provider context for FLY2 field query")
                })?(index + 1, p)
                .map_err(mlua::Error::external)
            };
            pa.set(
                "potential_vc",
                scope.create_function(move |_, (_t, x, y, z): (Table, f64, f64, f64)| {
                    Ok(sample(instance.position([x, y, z], false))?.potential_v)
                })?,
            )?;
            pa.set(
                "field_vc",
                scope.create_function(move |_, (_t, x, y, z): (Table, f64, f64, f64)| {
                    let e = sample(instance.position([x, y, z], false))?.electric_v_per_mm;
                    let local: [f64; 3] = std::array::from_fn(|a| {
                        (0..3).map(|b| instance.axes[a][b] * e[b]).sum::<f64>()
                            * instance.scale
                            * instance.spacing_mm[a]
                    });
                    Ok((local[0], local[1], local[2]))
                })?,
            )?;
            pa.set(
                "electrode",
                scope.create_function(move |_, (_t, x, y, z): (Table, f64, f64, f64)| {
                    if let Some(mask) = &instance.electrodes {
                        let q = [x, y, z];
                        if q.iter().any(|x| !x.is_finite() || x.fract() != 0.) {
                            return Err(mlua::Error::external(
                                "pa:electrode requires integer grid coordinates",
                            ));
                        }
                        if (0..3).any(|a| q[a] < 0. || q[a] >= instance.dimensions[a] as f64) {
                            return Ok(false);
                        }
                        let [x, y, z] = q.map(|x| x as usize);
                        return Ok(
                            mask[(z * instance.dimensions[1] + y) * instance.dimensions[0] + x]
                        );
                    }
                    Ok(sample(instance.position([x, y, z], false))?.electrode)
                })?,
            )?;
            t.set(
                "field_wc",
                scope.create_function(move |_, (_t, x, y, z): (Table, f64, f64, f64)| {
                    let e = sample([x, y, z])?.electric_v_per_mm;
                    Ok((e[0], e[1], e[2]))
                })?,
            )?;
            t.set(
                "potential_wc",
                scope.create_function(move |_, (_t, x, y, z): (Table, f64, f64, f64)| {
                    Ok(sample([x, y, z])?.potential_v)
                })?,
            )?;
            t.set(
                "pa_to_wb_coords",
                scope.create_function(move |_, (_t, x, y, z): (Table, f64, f64, f64)| {
                    let p = instance.position([x, y, z], false);
                    Ok((p[0], p[1], p[2]))
                })?,
            )?;
            t.set(
                "wb_to_pa_coords",
                scope.create_function(move |_, (_t, x, y, z): (Table, f64, f64, f64)| {
                    let p = instance.inverse([x, y, z]);
                    Ok((p[0], p[1], p[2]))
                })?,
            )?;
            t.set("pa", pa)?;
            instances.set(index + 1, t)?;
        }
        let simion = lua.create_table()?;
        let wb = lua.create_table()?;
        // Access to an absent workbench is an explicit context error, not an empty fake WB.
        if context.instances.is_empty() {
            let meta = lua.create_table()?;
            meta.set(
                "__index",
                scope.create_function(|_, (_t, key): (Table, Value)| -> mlua::Result<Value> {
                    Err(mlua::Error::external(format!(
                        "missing workbench instance context (simion.wb.instances[{key:?}])"
                    )))
                })?,
            )?;
            instances.set_metatable(Some(meta))?;
        }
        wb.set("instances", instances)?;
        let query = |p| {
            context.field.ok_or_else(|| {
                mlua::Error::external("missing field provider context for FLY2 workbench query")
            })?(0, p)
            .map_err(mlua::Error::external)
        };
        wb.set(
            "epotential",
            scope.create_function(move |_, (_t, x, y, z): (Table, f64, f64, f64)| {
                Ok(query([x, y, z])?.potential_v)
            })?,
        )?;
        wb.set(
            "efield",
            scope.create_function(move |_, (_t, x, y, z): (Table, f64, f64, f64)| {
                let e = query([x, y, z])?.electric_v_per_mm;
                Ok((e[0], e[1], e[2]))
            })?,
        )?;
        wb.set(
            "bfield",
            scope.create_function(move |_, (_t, x, y, z): (Table, f64, f64, f64)| {
                let b = query([x, y, z])?.magnetic_gauss;
                Ok((b[0], b[1], b[2]))
            })?,
        )?;
        simion.set("wb", wb)?;
        simion.set("rand", globals.get::<Value>("rand")?)?;
        globals.set("simion", simion)?;
        lua.load(include_str!("fly2.lua"))
            .set_name("FLY2 environment")
            .exec()?;
        lua.load(input.trim_start_matches('\u{feff}'))
            .set_mode(ChunkMode::Text)
            .set_name(path.map_or_else(|| "FLY2 input".into(), |p| p.to_string_lossy()))
            .exec()?;
        lua.load("_finish_particles()").exec()?;
        Ok(())
    })
    .map_err(lua_error)?;
    Ok(output.into_inner().unwrap())
}

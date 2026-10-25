//! Export native initial conditions without flying, for the FLY2 oracle.
use std::io;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::Deserialize;
use simrep_fly::{Field, GridField, InstanceField, SplatReason};
use simrep_io::{Fly2Context, Fly2Field, Fly2Instance, read_pa, read_particles_in};

#[derive(Deserialize)]
struct Placement {
    origin_mm: [f64; 3],
    axes: [[f64; 3]; 3],
    scale: f64,
    spacing_mm: [f64; 3],
    dimensions: [usize; 3],
    working_origin_mm: [f64; 3],
    pa: Option<PathBuf>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = std::env::args().collect::<Vec<_>>();
    let path = args
        .get(1)
        .ok_or("usage: particles FILE [CONTEXT.json] [SEED]")?;
    let placements: Vec<Placement> = match args.get(2).filter(|s| !s.is_empty()) {
        Some(path) => serde_json::from_reader(std::fs::File::open(path)?)?,
        None => Vec::new(),
    };
    let mut fields = Vec::new();
    let mut instances = Vec::new();
    for p in placements {
        let instance = Fly2Instance {
            origin_mm: p.origin_mm,
            axes: p.axes,
            scale: p.scale,
            spacing_mm: p.spacing_mm,
            dimensions: p.dimensions,
            working_origin_mm: p.working_origin_mm,
            electrodes: None,
        };
        let field =
            p.pa.map(|path| -> Result<_, Box<dyn std::error::Error>> {
                let array = read_pa(path)?.array;
                // InstanceField places the unmirrored PA origin, not its working origin.
                let origin = std::array::from_fn(|a| {
                    p.origin_mm[a]
                        - (0..3)
                            .map(|b| p.axes[b][a] * p.scale * p.working_origin_mm[b])
                            .sum::<f64>()
                });
                Ok(InstanceField::new(
                    GridField::new(array)?,
                    origin,
                    p.axes,
                    p.scale,
                    None,
                    None,
                )?)
            })
            .transpose()?;
        fields.push(field);
        instances.push(instance);
    }
    let trace = args
        .get(4)
        .map(
            |path| -> Result<Vec<Vec<f64>>, Box<dyn std::error::Error>> {
                std::fs::read_to_string(path)?
                    .lines()
                    .map(|line| {
                        let values = line
                            .split(',')
                            .map(str::parse)
                            .collect::<Result<Vec<f64>, _>>()?;
                        if values.len() != 12 {
                            return Err("field replay row requires 12 values".into());
                        }
                        Ok(values)
                    })
                    .collect()
            },
        )
        .transpose()?;
    let cursor = Mutex::new(0usize);
    let provider = |number: usize, point: [f64; 3]| -> io::Result<Fly2Field> {
        if let Some(trace) = &trace {
            let mut cursor = cursor.lock().unwrap();
            let row = trace
                .get(*cursor)
                .ok_or_else(|| io::Error::other("field replay exhausted"))?;
            if row[0] != number as f64 || (0..3).any(|a| (row[a + 1] - point[a]).abs() > 1e-10) {
                return Err(io::Error::other(format!(
                    "field replay query {} differs: instance {number}, point {point:?}, expected {:?}",
                    *cursor,
                    &row[..4]
                )));
            }
            *cursor += 1;
            return Ok(Fly2Field {
                potential_v: row[4],
                electric_v_per_mm: [row[5], row[6], row[7]],
                magnetic_gauss: [row[8], row[9], row[10]],
                electrode: row[11] != 0.,
            });
        }
        let field = fields
            .get(number.saturating_sub(1))
            .and_then(Option::as_ref)
            .ok_or_else(|| io::Error::other("instance has no PA field data"))?;
        let sample = field.sample(point, 0.);
        Ok(Fly2Field {
            potential_v: sample.potential_v,
            electric_v_per_mm: sample.electric_v_per_mm,
            magnetic_gauss: sample.magnetic_gauss,
            electrode: matches!(field.status(point), Some(SplatReason::Electrode { .. })),
        })
    };
    let context = Fly2Context {
        instances,
        field: Some(&provider),
        seed: args.get(3).map(|s| s.parse()).transpose()?.unwrap_or(0),
        lua_roots: vec![PathBuf::from("/mnt/c/Program Files/SIMION-2024/lib/lua")],
    };
    for p in read_particles_in(path, &context)? {
        let az = (-p.direction[2]).atan2(p.direction[0]).to_degrees();
        let el = p.direction[1]
            .atan2(p.direction[0].hypot(p.direction[2]))
            .to_degrees();
        println!(
            "{},{},{},{},{},{},{},{},{},{},{}",
            p.birth_time_us,
            p.mass_amu,
            p.charge_e,
            p.position_mm[0],
            p.position_mm[1],
            p.position_mm[2],
            az,
            el,
            p.energy_ev,
            p.charge_weight,
            p.color
        );
    }
    if trace
        .as_ref()
        .is_some_and(|t| *cursor.lock().unwrap() != t.len())
    {
        return Err("native FLY2 did not consume the entire oracle field trace".into());
    }
    Ok(())
}

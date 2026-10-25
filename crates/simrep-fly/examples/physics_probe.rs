//! Oracle helper: PA path, ng, placement, point on stdin; native B on stdout.
use std::io::{self, BufRead};

use simrep_fly::{
    AnalyticField, Field, FieldSample, FlightOptions, Particle,
    physics::{
        collision::{CollisionModel, Gas, Hs1, fly_many_collisional},
        magnetic::{Coil, MagneticField, MagneticPa},
    },
};
use simrep_io::read_pa;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args().nth(1).as_deref() == Some("mobility") {
        let hs = Hs1 {
            gas: Gas {
                mass_amu: 4.0,
                temperature_k: 273.0,
                ..Default::default()
            },
            steps_per_mean_free_time: 20.0,
            ..Default::default()
        };
        let field_strength = 0.0005;
        let mass = 200.0;
        let duration = 1000.0;
        let expected = hs.low_field_mobility(mass, 1.0) * field_strength;
        let field = AnalyticField(|_, _| FieldSample {
            electric_v_per_mm: [field_strength, 0.0, 0.0],
            ..Default::default()
        });
        let ion = Particle::new(mass, 1.0, [0.0; 3], [expected, 0.0, 0.0], 0.0)?;
        let ions = vec![ion; 16384];
        let model = CollisionModel::Hs1(hs);
        let results = fly_many_collisional(
            &field,
            &ions,
            &FlightOptions {
                duration_us: duration,
                max_step_us: 0.1,
                ..Default::default()
            },
            &model,
            314159,
        )?;
        let x: Vec<_> = results
            .iter()
            .map(|r| r.final_state.position_mm[0])
            .collect();
        let mean = x.iter().sum::<f64>() / x.len() as f64;
        let standard_error = (x.iter().map(|v| (v - mean).powi(2)).sum::<f64>()
            / (x.len() * (x.len() - 1)) as f64)
            .sqrt()
            / duration;
        println!(
            "mobility_ions={},expected_drift_mm_per_us={expected:.12e},measured_drift_mm_per_us={:.12e},standard_error={standard_error:.12e},relative_difference={:.12e}",
            x.len(),
            mean / duration,
            mean / duration / expected - 1.0
        );
        return Ok(());
    }
    let mut cached_pa: Option<(String, MagneticPa)> = None;
    for line in io::stdin().lock().lines() {
        let line = line?;
        let columns: Vec<_> = line.split(',').collect();
        let values = columns[2..]
            .iter()
            .map(|s| s.parse::<f64>())
            .collect::<Result<Vec<_>, _>>()?;
        let b = match columns[0] {
            "PA" => {
                let key = columns[..16].join(",");
                if cached_pa.as_ref().is_none_or(|(old, _)| *old != key) {
                    let file = read_pa(columns[1])?;
                    let origin = values[1..4].try_into()?;
                    let axes =
                        std::array::from_fn(|a| std::array::from_fn(|b| values[4 + 3 * a + b]));
                    let field =
                        MagneticPa::placed(file.array, values[0], origin, axes, values[13], None)?;
                    cached_pa = Some((key, field));
                }
                cached_pa
                    .as_ref()
                    .unwrap()
                    .1
                    .sample(values[14..17].try_into()?, 0.0)
                    .magnetic_gauss
            }
            "LOOP" => Coil::hoop([0.0; 3], [0.0, 0.0, 1.0], 30.0, 100.0)?
                .field_gauss(values[..3].try_into()?),
            "HELMHOLTZ" => {
                let mut sum = [0.0; 3];
                for z in [-15.0, 15.0] {
                    let b = Coil::hoop([0.0, 0.0, z], [0.0, 0.0, 1.0], 30.0, 100.0)?
                        .field_gauss(values[..3].try_into()?);
                    for a in 0..3 {
                        sum[a] += b[a];
                    }
                }
                sum
            }
            "SOLENOID" => {
                MagneticField::solenoid([-50.0, 0.0, 0.0], [50.0, 0.0, 0.0], 10.0, 0.7958, 100)?
                    .sample(values[..3].try_into()?, 0.0)
                    .magnetic_gauss
            }
            _ => return Err("unknown probe".into()),
        };
        println!(
            "{},{},{:.17e},{:.17e},{:.17e}",
            columns[0], columns[1], b[0], b[1], b[2]
        );
    }
    Ok(())
}

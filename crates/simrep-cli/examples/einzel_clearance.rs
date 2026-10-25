//! Diagnose the documented, z=0 einzel workload without changing the CLI.
use simrep_fly::{FlightOptions, GridField, InstanceField, Particle, fly_many};
use simrep_io::{read_fast_adjust, read_particles};
use std::fs::File;
use std::io::{BufWriter, Write};

fn rectangle_distance(from: [f64; 3], to: [f64; 3], bounds: [f64; 4]) -> f64 {
    let d = [to[0] - from[0], to[1] - from[1]];
    let mut cuts = [0.0; 6];
    cuts[1] = 1.0;
    let mut count = 2;
    for a in 0..2 {
        if d[a] != 0.0 {
            for edge in [bounds[a], bounds[a + 2]] {
                let t = (edge - from[a]) / d[a];
                if t > 0.0 && t < 1.0 {
                    cuts[count] = t;
                    count += 1;
                }
            }
        }
    }
    cuts[..count].sort_unstable_by(f64::total_cmp);
    let mut minimum = f64::INFINITY;
    for interval in cuts[..count].windows(2) {
        let middle = (interval[0] + interval[1]) * 0.5;
        let mut constant = [0.0; 2];
        let mut slope = [0.0; 2];
        for a in 0..2 {
            let p = from[a] + middle * d[a];
            if p < bounds[a] || p > bounds[a + 2] {
                let edge = if p < bounds[a] {
                    bounds[a]
                } else {
                    bounds[a + 2]
                };
                constant[a] = from[a] - edge;
                slope[a] = d[a];
            }
        }
        let denominator = slope[0].powi(2) + slope[1].powi(2);
        let t = if denominator == 0.0 {
            middle
        } else {
            (-(constant[0] * slope[0] + constant[1] * slope[1]) / denominator)
                .clamp(interval[0], interval[1])
        };
        minimum = minimum
            .min((constant[0] + t * slope[0]).powi(2) + (constant[1] + t * slope[1]).powi(2));
    }
    minimum.sqrt()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.len() != 3 {
        return Err("usage: einzel_clearance lens.pa0 ions.ion clearance.csv".into());
    }
    let field = InstanceField::new(
        GridField::new(read_fast_adjust(
            &args[0],
            &[(1, 0.0), (2, 110.0), (3, 0.0)],
        )?)?,
        [0.0; 3],
        [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        1.0,
        Some([[-101.0, -19.0, -19.0], [291.0, 19.0, 19.0]]),
        None,
    )?;
    let inputs = read_particles(&args[1])?;
    let particles = inputs
        .iter()
        .map(|p| {
            if p.position_mm[2] != 0.0 || p.direction[2] != 0.0 {
                return Err(simrep_fly::FlyError("clearance study requires z=0 flights"));
            }
            Particle::from_relativistic_energy(
                p.mass_amu,
                p.charge_e,
                p.position_mm,
                p.energy_ev,
                p.direction,
                p.birth_time_us,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    let options = FlightOptions {
        relative_tolerance: 1e-8,
        position_tolerance_mm: 1e-8,
        velocity_tolerance_mm_per_us: 1e-8,
        duration_us: 100.0,
        relativistic: true,
        sample_every: Some(1),
        ..FlightOptions::default()
    };
    let mut output = BufWriter::new(File::create(&args[2])?);
    writeln!(
        output,
        "ion,initial_radius_mm,min_electrode_distance_mm,max_radius_in_pa_mm"
    )?;
    for (batch, chunk) in particles.chunks(1024).enumerate() {
        let results = fly_many(&field, chunk, &options)?;
        for (offset, result) in results.into_iter().enumerate() {
            let index = batch * 1024 + offset;
            let path = result.trajectory.as_ref().unwrap();
            let mut clearance = f64::INFINITY;
            for segment in path.windows(2) {
                for (left, right) in [(0.0, 28.5), (29.5, 56.5), (57.5, 90.0)] {
                    for (low, high) in [(-19.0, -17.5), (17.5, 19.0)] {
                        clearance = clearance.min(rectangle_distance(
                            segment[0].position_mm,
                            segment[1].position_mm,
                            [left, low, right, high],
                        ));
                    }
                }
            }
            let radius = inputs[index].position_mm[1].abs();
            let maximum = path
                .iter()
                .filter(|p| (0.0..=90.0).contains(&p.position_mm[0]))
                .map(|p| p.position_mm[1].abs())
                .fold(0.0, f64::max);
            writeln!(
                output,
                "{},{radius:.12e},{clearance:.12e},{maximum:.12e}",
                index + 1
            )?;
        }
    }
    output.flush()?;
    eprintln!("clearance records: {}", particles.len());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::rectangle_distance;

    #[test]
    fn segment_clearance_handles_intersections_edges_and_corners() {
        let bounds = [0.0, 0.0, 1.0, 1.0];
        assert_eq!(
            rectangle_distance([-1.0, 0.5, 0.0], [2.0, 0.5, 0.0], bounds),
            0.0
        );
        assert_eq!(
            rectangle_distance([-1.0, 2.0, 0.0], [2.0, 2.0, 0.0], bounds),
            1.0
        );
        let corner = rectangle_distance([-2.0, 1.0, 0.0], [-1.0, 2.0, 0.0], bounds);
        assert!((corner - 2.0_f64.sqrt()).abs() < 1e-15);
    }
}

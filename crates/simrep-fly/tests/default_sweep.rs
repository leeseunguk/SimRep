//! Opt-in, serial default-control sweep; see bench/default_flight_sweep.py.
use std::{path::PathBuf, time::Instant};

use simrep_fly::{
    FlightOptions, GridField, InstanceField, Particle, fly_many,
    physics::magnetic::{Electromagnetic, MagneticField},
};
use simrep_io::{read_fast_adjust, read_particles};

#[test]
#[ignore = "requires SIMREP_DEFAULT_SWEEP (external default-flight sweep fixtures)"]
fn einzel_step_control_sweep() {
    run_sweep();
}

pub fn run_sweep() {
    let root = std::env::var("SIMREP_DEFAULT_SWEEP").expect("requires SIMREP_DEFAULT_SWEEP");
    rayon::ThreadPoolBuilder::new()
        .num_threads(16)
        .build_global()
        .unwrap();
    let root = PathBuf::from(root);
    let array = read_fast_adjust(
        root.join("_benchfly/einzel.pa0"),
        &[(1, 0.0), (2, 110.0), (3, 0.0)],
    )
    .unwrap();
    let field = InstanceField::new(
        GridField::new(array).unwrap(),
        [0.0; 3],
        [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        1.0,
        Some([[-101.0, -19.0, -19.0], [291.0, 19.0, 19.0]]),
        None,
    )
    .unwrap();
    let combined = Electromagnetic {
        electric: field,
        magnetic: MagneticField::default(),
    };
    let cli_field = std::env::var_os("SIMREP_SWEEP_CLI_FIELD").is_some();
    let particles = read_particles(root.join("_benchfly/einzel_10000.ion"))
        .unwrap()
        .into_iter()
        .map(|p| {
            Particle::from_relativistic_energy(
                p.mass_amu,
                p.charge_e,
                p.position_mm,
                p.energy_ev,
                p.direction,
                p.birth_time_us,
            )
            .unwrap()
        })
        .collect::<Vec<_>>();
    let tolerances = if let Ok(value) = std::env::var("SIMREP_SWEEP_TOLERANCE") {
        vec![value.parse::<f64>().unwrap()]
    } else if std::env::var_os("SIMREP_SWEEP_QUICK").is_some() {
        vec![0.0]
    } else {
        vec![0.0, 1e-12, 1e-10, 1e-8, 1e-6, 1e-4]
    };
    let cells = if let Ok(value) = std::env::var("SIMREP_SWEEP_CELL") {
        vec![if value == "default" {
            FlightOptions::default().step_limits.max_grid_cells
        } else {
            value.parse::<f64>().unwrap()
        }]
    } else {
        vec![1.0, 2.0, 4.0, 8.0, f64::MAX]
    };
    let repeats = std::env::var("SIMREP_SWEEP_REPEATS")
        .map(|value| value.parse::<usize>().unwrap())
        .unwrap_or(5);
    for tolerance in tolerances {
        for &cells in &cells {
            let mut options = FlightOptions {
                duration_us: 100.0,
                relativistic: true,
                automatic_tolerance: tolerance == 0.0,
                relative_tolerance: if tolerance == 0.0 { 1e-12 } else { tolerance },
                position_tolerance_mm: if tolerance == 0.0 { 1e-12 } else { tolerance },
                velocity_tolerance_mm_per_us: if tolerance == 0.0 { 1e-12 } else { tolerance },
                ..Default::default()
            };
            options.step_limits.max_grid_cells = cells;
            let mut times = Vec::new();
            let mut results = Vec::new();
            for _ in 0..repeats {
                let start = Instant::now();
                results = if cli_field {
                    fly_many(&combined, &particles, &options).unwrap()
                } else {
                    fly_many(&combined.electric, &particles, &options).unwrap()
                };
                times.push(start.elapsed().as_secs_f64());
            }
            times.sort_by(f64::total_cmp);
            println!(
                "SWEEP,tolerance={tolerance:e},cells={cells:e},median_s={:.9},accepted={},rejected={}",
                times[times.len() / 2],
                results.iter().map(|r| r.accepted_steps).sum::<usize>(),
                results.iter().map(|r| r.rejected_steps).sum::<usize>(),
            );
        }
    }
}

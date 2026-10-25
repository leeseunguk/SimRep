//! Compute-only scale probe: `cargo run --release -p simrep-fly --example space_charge -- 10000`.
use simrep_fly::{
    AnalyticField, FieldSample, FlightOptions, Particle,
    physics::{ChargeMode, DirectCharge, EnsembleOptions, PicMesh, SpaceCharge, fly_ensemble},
};
use simrep_solve::mg::{Geometry, Grid, Options};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let n = std::env::args()
        .nth(1)
        .map_or(Ok(1000), |v| v.parse::<usize>())?;
    if n == 0 {
        return Err("particle count must be positive".into());
    }
    let particles: Vec<_> = (0..n)
        .map(|i| {
            // Deterministic equal-volume radial shells with spherical Fibonacci directions.
            let u = (i as f64 + 0.5) / n as f64;
            let radius = u.cbrt();
            let z = 1.0 - 2.0 * ((i * 8191 % n) as f64 + 0.5) / n as f64;
            let theta = i as f64 * 2.399_963_229_728_653;
            let r = (1.0 - z * z).sqrt();
            Particle::new(
                100.0,
                1.0,
                [
                    radius * r * theta.cos(),
                    radius * r * theta.sin(),
                    radius * z,
                ],
                [0.0; 3],
                0.0,
            )
        })
        .collect::<Result<_, _>>()?;
    let options = EnsembleOptions {
        flight: FlightOptions {
            duration_us: 0.01,
            initial_step_us: 0.001,
            max_step_us: 0.001,
            ..FlightOptions::default()
        },
        charge_mode: ChargeMode::Coulombic(1e-14),
        ..EnsembleOptions::default()
    };
    let grid = Grid {
        dims: [33; 3],
        spacing: [0.25; 3],
        geometry: Geometry::Planar,
    };
    let fixed = (0..33 * 33 * 33)
        .map(|i| {
            [i % 33, (i / 33) % 33, i / (33 * 33)]
                .into_iter()
                .any(|c| c == 0 || c == 32)
        })
        .collect();
    let pic = PicMesh::new(
        grid,
        [-4.0; 3],
        fixed,
        Options {
            relative_tolerance: 1e-8,
            ..Options::default()
        },
    )?;
    let field = AnalyticField(|_, _| FieldSample::default());
    for (name, mut method) in [
        ("direct", SpaceCharge::Direct(DirectCharge::default())),
        ("pic", SpaceCharge::Pic(Box::new(pic))),
    ] {
        let report = fly_ensemble(&field, &particles, &options, &mut method)?;
        println!(
            "{name}: N={n}, global_steps={}, compute_s={:.9}",
            report.global_steps,
            report.compute_time.as_secs_f64()
        );
    }
    Ok(())
}

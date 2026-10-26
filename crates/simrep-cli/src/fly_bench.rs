use std::time::Instant;

use clap::Args;
use simrep_core::{PotentialArray, Symmetry};
use simrep_fly::{
    BasisPotential, FlightOptions, GridField, Particle, SplatReason, StopPlane, Superposition,
    fly_many,
};
use simrep_solve::gpu::GpuResult;

#[derive(Args)]
pub struct FlyBenchOptions {
    /// Positive number of reference ions to fly.
    #[arg(long, alias = "particles", default_value_t = 10_000, value_parser = clap::value_parser!(usize))]
    ions: usize,
}

pub fn run(options: FlyBenchOptions) -> GpuResult<()> {
    if options.ions == 0 {
        return Err("--ions must be positive".into());
    }
    let setup = Instant::now();
    let mut pa = PotentialArray::new(91, 21, 1);
    pa.symmetry = Symmetry::Cylindrical;
    // Paraxial expansion of a Gaussian on-axis lens potential:
    // V(x,r) = phi(x) - r^2 phi''(x)/4, phi=exp(-(x-45)^2/(2*8^2)).
    // This synthetic field exercises interpolation, axial rotation and fast
    // adjustment; it is not the recorded SIMION einzel geometry.
    for r in 0..pa.ny {
        for x in 0..pa.nx {
            let u = (x as f64 - 45.0) / 8.0;
            let phi = (-0.5 * u * u).exp();
            let second = (u * u - 1.0) * phi / 64.0;
            let i = pa.index(x, r, 0).unwrap();
            pa.potentials[i] = phi - (r * r) as f64 * second / 4.0;
        }
    }
    let field = Superposition::new(vec![BasisPotential {
        grid: GridField::new(pa)?,
        voltage: Box::new(|_| 30.0),
    }])?;
    let particles: Vec<_> = (0..options.ions)
        .map(|i| {
            let radius = 0.1 + 1.9 * (i % 1000) as f64 / 999.0;
            let angle = i as f64 * 2.399_963_229_728_653;
            Particle::from_energy(
                40.0,
                1.0,
                [1.0, radius * angle.cos(), radius * angle.sin()],
                100.0,
                [1.0, 0.0, 0.0],
                0.0,
            )
        })
        .collect::<Result<_, _>>()?;
    let flight_options = FlightOptions {
        duration_us: 10.0,
        initial_step_us: 0.005,
        max_step_us: 0.1,
        relative_tolerance: 1e-8,
        position_tolerance_mm: 1e-8,
        velocity_tolerance_mm_per_us: 1e-8,
        stop_plane: Some(StopPlane {
            axis: 0,
            coordinate_mm: 89.0,
        }),
        ..Default::default()
    };
    let setup_seconds = setup.elapsed().as_secs_f64();
    let start = Instant::now();
    let results = fly_many(&field, &particles, &flight_options)?;
    let seconds = start.elapsed().as_secs_f64();
    let arrived = results
        .iter()
        .filter(|r| r.splat_reason == SplatReason::StopPlane)
        .count();
    let steps: usize = results.iter().map(|r| r.accepted_steps).sum();
    let rejected: usize = results.iter().map(|r| r.rejected_steps).sum();
    let checksum: f64 = results
        .iter()
        .map(|r| r.tof_us + r.final_state.position_mm[1])
        .sum();
    println!("CPU f64 RK45, rayon, interpolated 91x21 cylindrical einzel-like grid");
    println!("ions: {}, detector arrivals: {arrived}", options.ions);
    println!("setup: {setup_seconds:.6} s, fly: {seconds:.6} s");
    println!(
        "throughput: {:.0} ions/s, accepted steps: {steps}, rejected: {rejected}",
        options.ions as f64 / seconds
    );
    println!("trajectory checksum (TOF us + final y mm): {checksum:.12e}");
    crate::json::set_summary(serde_json::json!({
        "ion_count": options.ions,
        "arrivals": arrived,
        "compute_ms": seconds * 1000.0,
        "prep_ms": setup_seconds * 1000.0,
        "accepted_steps": steps,
        "rejected_steps": rejected,
        "checksum": checksum,
    }));
    if let Some(baseline) = match options.ions {
        10_000 => Some(2.2),
        100_000 => Some(21.5),
        _ => None,
    } {
        println!(
            "SIMION recorded einzel baseline: {baseline:.3} s; timing ratio: {:.2}x (different synthetic field, not accuracy parity)",
            baseline / seconds
        );
    }
    if arrived != options.ions {
        return Err(format!("only {arrived}/{} ions reached the detector", options.ions).into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use crate::{Cli, Command};

    #[test]
    fn parses_default_and_explicit_ion_count() {
        let cli = Cli::try_parse_from(["simrep", "fly-bench"]).unwrap();
        let Command::FlyBench(options) = cli.command else {
            panic!("wrong subcommand")
        };
        assert_eq!(options.ions, 10_000);
        let cli = Cli::try_parse_from(["simrep", "fly-bench", "--ions", "100000"]).unwrap();
        let Command::FlyBench(options) = cli.command else {
            panic!("wrong subcommand")
        };
        assert_eq!(options.ions, 100_000);
    }
}

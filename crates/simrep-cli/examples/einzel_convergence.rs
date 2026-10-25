//! Independent physical-grid study. Run from the repository root:
//! cargo run --release -p simrep-cli --example einzel_convergence

use std::error::Error;
use std::fs::{File, create_dir_all};
use std::io::{BufWriter, Write};
use std::path::Path;
use std::time::Instant;

use simrep_core::{PotentialArray, Symmetry};
use simrep_fly::{
    ELECTRIC_ACCELERATION, Field, FlightOptions, FlightResult, GridField, InstanceField, Particle,
    SPEED_OF_LIGHT, SplatReason, fly_many,
};
use simrep_io::{PaFile, read_fast_adjust, read_pa, read_particles, write_pa};
use simrep_solve::mg::{Geometry, Grid, Options, solve};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

const PA: &str = "/mnt/c/Users/lsu10/simrep-oracle/_benchfly/einzel.pa0";
const IONS: &str = "/mnt/c/Users/lsu10/simrep-oracle/_real_einzel/subset.ion";
const OUTPUT: &str = "target/fly-real";

fn transpose<T: Copy>(values: &[T], nx: usize, ny: usize) -> Vec<T> {
    (0..values.len())
        .map(|i| values[i / ny + nx * (i % ny)])
        .collect()
}

fn geometry(template: &PotentialArray, factor: usize) -> PotentialArray {
    let mut a = PotentialArray::new(90 * factor + 1, 19 * factor + 1, 1);
    a.symmetry = template.symmetry;
    a.mirrors = template.mirrors;
    a.dx_mm = 1.0 / factor as f64;
    a.dy_mm = a.dx_mm;
    // Cylindrical z is inactive; retain the original spacing metadata.
    a.dz_mm = template.dz_mm;
    for r in 18 * factor..a.ny {
        for (lo, hi, volts) in [(0, 28, 0.0), (30, 56, 110.0), (58, 90, 0.0)] {
            for x in lo * factor..=hi * factor {
                let i = x + a.nx * r;
                a.electrodes[i] = true;
                a.potentials[i] = volts;
            }
        }
    }
    a
}

// Recompute the physical Laplacian directly in axial-x/radial-y PA order,
// independently of the solver's transposed storage and weighted operator.
fn residual(a: &PotentialArray) -> f64 {
    let mut largest: f64 = 0.0;
    for r in 0..a.ny {
        for x in 0..a.nx {
            let i = x + a.nx * r;
            if a.electrodes[i] {
                continue;
            }
            let v = a.potentials[i];
            let axial = if x == 0 {
                2.0 * (a.potentials[i + 1] - v)
            } else if x + 1 == a.nx {
                2.0 * (a.potentials[i - 1] - v)
            } else {
                (a.potentials[i - 1] - v) + (a.potentials[i + 1] - v)
            };
            let radial = if r == 0 {
                4.0 * (a.potentials[i + a.nx] - v)
            } else if r + 1 == a.ny {
                2.0 * (a.potentials[i - a.nx] - v)
            } else {
                (1.0 - 0.5 / r as f64) * (a.potentials[i - a.nx] - v)
                    + (1.0 + 0.5 / r as f64) * (a.potentials[i + a.nx] - v)
            };
            largest = largest.max((axial / a.dx_mm.powi(2) + radial / a.dy_mm.powi(2)).abs());
        }
    }
    largest
}

fn energy(p: &Particle, result: &FlightResult) -> f64 {
    let v2: f64 = result
        .final_state
        .velocity_mm_per_us
        .iter()
        .map(|v| v * v)
        .sum();
    let inverse_gamma = (1.0 - v2 / SPEED_OF_LIGHT.powi(2)).sqrt();
    p.mass_amu * v2 / (ELECTRIC_ACCELERATION * inverse_gamma * (1.0 + inverse_gamma))
}

fn reason(result: &FlightResult) -> &'static str {
    match result.splat_reason {
        SplatReason::Electrode { .. } | SplatReason::SurfaceElectrode { .. } => "electrode",
        SplatReason::OutsideGrid => "outside",
        SplatReason::StopPlane => "plane",
        SplatReason::TimeLimit => "time_limit",
        SplatReason::StepLimit => "step_limit",
        SplatReason::StepUnderflow => "step_underflow",
        SplatReason::NonFinite => "nonfinite",
    }
}

fn write_flights(path: &Path, particles: &[Particle], results: &[FlightResult]) -> Result<()> {
    let mut csv = BufWriter::new(File::create(path)?);
    writeln!(csv, "ion,tof_us,x_mm,y_mm,z_mm,ke_ev,reason")?;
    for (i, (p, result)) in particles.iter().zip(results).enumerate() {
        let [x, y, z] = result.final_state.position_mm;
        writeln!(
            csv,
            "{},{:.15e},{x:.15e},{y:.15e},{z:.15e},{:.15e},{}",
            i + 1,
            result.tof_us,
            energy(p, result),
            reason(result)
        )?;
    }
    csv.flush()?;
    Ok(())
}

fn potential_difference(
    coarse: &PotentialArray,
    fine: &PotentialArray,
    ratio: usize,
) -> (f64, f64) {
    let mut max: f64 = 0.0;
    let mut squares = 0.0;
    let mut count = 0;
    for r in 0..coarse.ny {
        for x in 0..coarse.nx {
            let i = x + coarse.nx * r;
            if !coarse.electrodes[i] {
                let difference =
                    coarse.potentials[i] - fine.potentials[ratio * x + fine.nx * ratio * r];
                max = max.max(difference.abs());
                squares += difference * difference;
                count += 1;
            }
        }
    }
    (max, (squares / count as f64).sqrt())
}

fn probes(csv: &mut impl Write, factor: usize, field: &GridField) -> Result<()> {
    for [x, y, z] in [
        [40.0, 0.0, 0.0],
        [29.0, 10.0, 0.0],
        [29.0, 15.0, 0.0],
        [30.0, 16.0, 0.0],
        [43.0, 16.0, 0.0],
        [57.0, 16.0, 0.0],
        [60.0, 15.0, 0.0],
    ] {
        let s = field.sample([x, y, z], 0.0);
        let [ex, ey, ez] = s.electric_v_per_mm;
        println!(
            "probe {factor}x [{x},{y},{z}]: potential_v={:.15e}, electric_v_per_mm=[{ex:.15e},{ey:.15e},{ez:.15e}]",
            s.potential_v
        );
        writeln!(
            csv,
            "{factor},{x},{y},{z},{:.15e},{ex:.15e},{ey:.15e},{ez:.15e}",
            s.potential_v
        )?;
    }
    csv.flush()?;
    Ok(())
}

fn main() -> Result<()> {
    if std::env::args().any(|arg| arg == "--boundary") {
        return boundary_study();
    }
    create_dir_all(OUTPUT)?;
    let original = read_fast_adjust(PA, &[(1, 0.0), (2, 110.0), (3, 0.0)])?;
    assert_eq!((original.nx, original.ny, original.nz), (91, 20, 1));
    assert_eq!(original.symmetry, Symmetry::Cylindrical);
    assert_eq!([original.dx_mm, original.dy_mm], [1.0; 2]);
    assert_eq!(original.electrodes, geometry(&original, 1).electrodes);
    let particles = read_particles(IONS)?
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
        })
        .collect::<std::result::Result<Vec<_>, _>>()?;
    assert_eq!(particles.len(), 1000);
    let flight_options = FlightOptions {
        duration_us: 100.0,
        max_step_us: 0.05,
        relative_tolerance: 1e-11,
        position_tolerance_mm: 1e-11,
        velocity_tolerance_mm_per_us: 1e-11,
        relativistic: true,
        ..FlightOptions::default()
    };
    let out = Path::new(OUTPUT);
    let mut probe_csv = BufWriter::new(File::create(out.join("grid-probes.csv"))?);
    writeln!(
        probe_csv,
        "factor,x_mm,y_mm,z_mm,potential_v,ex_v_mm,ey_v_mm,ez_v_mm"
    )?;
    probes(&mut probe_csv, 0, &GridField::new(original.clone())?)?;
    let mut solves = BufWriter::new(File::create(out.join("grid-solves.csv"))?);
    writeln!(
        solves,
        "factor,nx,ny,h_mm,electrodes,cycles,relative_residual,reported_residual_v_mm2,independent_residual_v_mm2,solve_s,fly_s,accepted,rejected"
    )?;
    let mut grids = Vec::new();
    let mut flights = Vec::new();
    for factor in [1, 2, 4, 8] {
        let mut a = geometry(&original, factor);
        let mut values = transpose(&a.potentials, a.nx, a.ny);
        let fixed = transpose(&a.electrodes, a.nx, a.ny);
        let report = solve(
            &mut values,
            &fixed,
            Grid {
                dims: [a.ny, a.nx, 1],
                spacing: [a.dy_mm, a.dx_mm, a.dz_mm],
                geometry: Geometry::Cylindrical { radial_origin: 0.0 },
            },
            Options {
                relative_tolerance: 1e-14,
                max_cycles: 250,
                ..Options::default()
            },
        )?;
        let last = report.residual_history.last().unwrap();
        a.potentials = transpose(&values, a.ny, a.nx);
        let checked = residual(&a);
        println!(
            "grid {factor}x: {}x{}, cycles={}, converged={}, relative={:.9e}, absolute={:.9e}, independent={checked:.9e} V/mm^2, solve_s={:.6}",
            a.nx,
            a.ny,
            report.iterations,
            report.converged,
            last.relative,
            last.absolute,
            report.elapsed.as_secs_f64()
        );
        assert!(report.converged, "grid {factor} did not converge");
        assert!(checked < 1e-8, "independent residual is not tight");
        if factor == 1 {
            let (max, rms) = potential_difference(&original, &a, 1);
            println!("SIMION PA vs solved 1x potential: max={max:.9e}, rms={rms:.9e} V");
        }
        write_pa(
            out.join(format!("grid-{factor}.pa")),
            &PaFile::new(a.clone()),
        )?;
        let grid = GridField::new(a.clone())?;
        probes(&mut probe_csv, factor, &grid)?;
        let field = InstanceField::new(
            grid,
            [0.0; 3],
            [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            1.0,
            Some([[-101.0, -19.0, -19.0], [291.0, 19.0, 19.0]]),
            None,
        )?;
        let start = Instant::now();
        let results = fly_many(&field, &particles, &flight_options)?;
        let elapsed = start.elapsed().as_secs_f64();
        write_flights(
            &out.join(format!("grid-{factor}.csv")),
            &particles,
            &results,
        )?;
        let accepted: usize = results.iter().map(|r| r.accepted_steps).sum();
        let rejected: usize = results.iter().map(|r| r.rejected_steps).sum();
        let outside = results.iter().filter(|r| reason(r) == "outside").count();
        let electrode = results.iter().filter(|r| reason(r) == "electrode").count();
        println!(
            "grid {factor}x flight: {} ions, outside={outside}, electrode={electrode}, accepted={accepted}, rejected={rejected}, fly_s={elapsed:.6}",
            results.len()
        );
        assert_eq!(
            outside + electrode,
            particles.len(),
            "nonphysical termination"
        );
        writeln!(
            solves,
            "{factor},{},{},{:.9e},{},{},{:.9e},{:.9e},{checked:.9e},{:.6},{elapsed:.6},{accepted},{rejected}",
            a.nx,
            a.ny,
            a.dx_mm,
            a.electrodes.iter().filter(|&&v| v).count(),
            report.iterations,
            last.relative,
            last.absolute,
            report.elapsed.as_secs_f64()
        )?;
        solves.flush()?;
        grids.push(a);
        flights.push(results);
    }
    let mut comparisons = BufWriter::new(File::create(out.join("grid-comparison.csv"))?);
    writeln!(
        comparisons,
        "coarse,fine,reason_changes,position_max_mm,position_rms_mm,tof_max_us,tof_rms_us,ke_max_ev,ke_rms_ev,potential_max_v,potential_rms_v,worst_ion"
    )?;
    for (c, f) in [(0, 1), (1, 2), (2, 3), (1, 3)] {
        let mut maxima = [0.0_f64; 3];
        let mut squares = [0.0; 3];
        let mut changes = 0;
        let mut worst = 0;
        for (i, ((a, b), p)) in flights[c]
            .iter()
            .zip(&flights[f])
            .zip(&particles)
            .enumerate()
        {
            changes += usize::from(reason(a) != reason(b));
            let d = a
                .final_state
                .position_mm
                .iter()
                .zip(b.final_state.position_mm)
                .map(|(x, y)| (x - y).powi(2))
                .sum::<f64>()
                .sqrt();
            if d > maxima[0] {
                worst = i + 1;
            }
            for (axis, value) in [
                d,
                (a.tof_us - b.tof_us).abs(),
                (energy(p, a) - energy(p, b)).abs(),
            ]
            .into_iter()
            .enumerate()
            {
                maxima[axis] = maxima[axis].max(value);
                squares[axis] += value * value;
            }
        }
        let rms = squares.map(|v| (v / particles.len() as f64).sqrt());
        let (vmax, vrms) = potential_difference(&grids[c], &grids[f], 1 << (f - c));
        println!(
            "{}x -> {}x: position max={:.9e}, rms={:.9e} mm, tof max={:.9e}, rms={:.9e} us, ke max={:.9e}, rms={:.9e} eV, reason_changes={changes}, worst_ion={worst}, potential max={vmax:.9e}, rms={vrms:.9e} V",
            1 << c,
            1 << f,
            maxima[0],
            rms[0],
            maxima[1],
            rms[1],
            maxima[2],
            rms[2]
        );
        writeln!(
            comparisons,
            "{},{},{changes},{:.15e},{:.15e},{:.15e},{:.15e},{:.15e},{:.15e},{vmax:.15e},{vrms:.15e},{worst}",
            1 << c,
            1 << f,
            maxima[0],
            rms[0],
            maxima[1],
            rms[1],
            maxima[2],
            rms[2]
        )?;
    }
    comparisons.flush()?;
    println!(
        "Artifacts: {OUTPUT}/grid-{{1,2,4,8}}.{{csv,pa}}, grid-solves.csv, grid-comparison.csv, grid-probes.csv"
    );
    Ok(())
}

/// Reuse the original grids and 8x flights; never overwrite the reference.
fn boundary_study() -> Result<()> {
    let particles = read_particles(IONS)?
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
        })
        .collect::<std::result::Result<Vec<_>, _>>()?;
    assert_eq!(particles.len(), 1000);
    let reference = std::fs::read_to_string(format!("{OUTPUT}/grid-8.csv"))?;
    let reference: Vec<Vec<f64>> = reference
        .lines()
        .skip(1)
        .map(|line| line.split(',').skip(1).take(4).map(str::parse).collect())
        .collect::<std::result::Result<_, _>>()?;
    assert_eq!(reference.len(), particles.len());
    let options = FlightOptions {
        duration_us: 100.0,
        max_step_us: 0.05,
        relative_tolerance: 1e-11,
        position_tolerance_mm: 1e-11,
        velocity_tolerance_mm_per_us: 1e-11,
        relativistic: true,
        ..FlightOptions::default()
    };
    let mut csv = BufWriter::new(File::create(format!("{OUTPUT}/boundary-comparison.csv"))?);
    writeln!(
        csv,
        "grid,position_max_mm,position_rms_mm,tof_max_us,tof_rms_us,fly_s,accepted,rejected"
    )?;
    for label in ["original", "1", "2", "4", "8"] {
        let a = if label == "original" {
            read_fast_adjust(PA, &[(1, 0.0), (2, 110.0), (3, 0.0)])?
        } else {
            read_pa(format!("{OUTPUT}/grid-{label}.pa"))?.array
        };
        let field = InstanceField::new(
            GridField::new(a)?,
            [0.0; 3],
            [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            1.0,
            Some([[-101.0, -19.0, -19.0], [291.0, 19.0, 19.0]]),
            None,
        )?;
        let start = Instant::now();
        let results = fly_many(&field, &particles, &options)?;
        let elapsed = start.elapsed().as_secs_f64();
        write_flights(
            &Path::new(OUTPUT).join(format!("boundary-{label}.csv")),
            &particles,
            &results,
        )?;
        let mut maxima = [0.0_f64; 2];
        let mut squares = [0.0; 2];
        for (result, expected) in results.iter().zip(&reference) {
            assert_eq!(result.splat_reason, SplatReason::OutsideGrid);
            let position = result
                .final_state
                .position_mm
                .iter()
                .zip(&expected[1..])
                .map(|(a, b)| (a - b).powi(2))
                .sum::<f64>()
                .sqrt();
            for (axis, d) in [position, (result.tof_us - expected[0]).abs()]
                .into_iter()
                .enumerate()
            {
                maxima[axis] = maxima[axis].max(d);
                squares[axis] += d * d;
            }
        }
        let rms = squares.map(|v| (v / particles.len() as f64).sqrt());
        let accepted: usize = results.iter().map(|r| r.accepted_steps).sum();
        let rejected: usize = results.iter().map(|r| r.rejected_steps).sum();
        println!(
            "boundary {label} vs original 8x: position max={:.9e}, rms={:.9e} mm; tof max={:.9e}, rms={:.9e} us; fly_s={elapsed:.6}, accepted={accepted}, rejected={rejected}",
            maxima[0], rms[0], maxima[1], rms[1]
        );
        writeln!(
            csv,
            "{label},{:.15e},{:.15e},{:.15e},{:.15e},{elapsed:.6},{accepted},{rejected}",
            maxima[0], rms[0], maxima[1], rms[1]
        )?;
        csv.flush()?;
    }
    Ok(())
}

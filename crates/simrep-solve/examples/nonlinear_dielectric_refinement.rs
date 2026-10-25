//! Physical-grid refinement of the shipped finite-box nonlinear sphere.
//! Usage: <original directory> <output directory> <spacing mm> generate|cpu|observe
//!        [tolerance=1e-11] [repetitions=1] [snapshot name=native.pa]
use std::{path::Path, time::Instant};

use simrep_io::{PaFile, read_pa, write_pa};
use simrep_solve::{
    dielectric::{FieldLaw, FieldSampler, NonlinearOptions, NonlinearPermittivity},
    gpu::{Backends, Gpu, GpuResult},
    mg::{Geometry, Grid, Options},
};

fn index(c: [usize; 3], d: [usize; 3]) -> usize {
    c[0] + d[0] * (c[1] + d[1] * c[2])
}

fn potential(v: &[f64], d: [usize; 3], p: [f64; 3]) -> f64 {
    let p = p.map(f64::abs);
    let lo: [usize; 3] = std::array::from_fn(|a| (p[a].floor() as usize).min(d[a] - 2));
    let t: [f64; 3] = std::array::from_fn(|a| p[a] - lo[a] as f64);
    (0..8)
        .map(|corner| {
            let c = std::array::from_fn(|a| lo[a] + usize::from(corner & (1 << a) != 0));
            let w: f64 = (0..3)
                .map(|a| {
                    if corner & (1 << a) != 0 {
                        t[a]
                    } else {
                        1.0 - t[a]
                    }
                })
                .product();
            w * v[index(c, d)]
        })
        .sum()
}

fn field(v: &[f64], d: [usize; 3], p: [f64; 3], h: f64, half_step: bool) -> [f64; 3] {
    std::array::from_fn(|a| {
        let mut left = p;
        let mut right = p;
        if half_step {
            left[a] -= 0.5;
            right[a] += 0.5;
        } else {
            left[a] = p[a].floor();
            right[a] = left[a] + 1.0;
        }
        (potential(v, d, left) - potential(v, d, right)) / h
    })
}

fn observables(pa: &PaFile, h: f64, half_step: bool) {
    let d = [pa.array.nx, pa.array.ny, pa.array.nz];
    let v = &pa.array.potentials;
    let law = FieldLaw::SaturatingQuadratic {
        lambda: 8e-11,
        minimum: 1.0,
    };
    let epsilon0 = 8.8541878128e-15; // F/mm
    let mut dipole = 0.0;
    let mut cells = 0;
    for z in 0..d[2] - 1 {
        for y in 0..d[1] - 1 {
            for x in 0..d[0] - 1 {
                let mut p = [x as f64 + 0.5, y as f64 + 0.5, z as f64 + 0.5];
                let r =
                    ((p[0] * h - 50.0).powi(2) + (p[1] * h).powi(2) + (p[2] * h).powi(2)).sqrt();
                if r >= 10.0 {
                    continue;
                }
                cells += 1;
                let cell_center = p;
                if r > 10.0 - h {
                    let s = (10.0 - h) / r;
                    p = [(p[0] - 50.0 / h) * s + 50.0 / h, p[1] * s, p[2] * s];
                }
                let e = field(v, d, p, h, half_step);
                let k = law.epsilon(6.0, e.iter().map(|e| e * e).sum::<f64>().sqrt());
                // The offset field defines epsilon, not the spatial location
                // of P = epsilon_0 (epsilon_r - 1) E in its material cell.
                dipole += (k - 1.0) * field(v, d, cell_center, h, false)[0];
            }
        }
    }
    dipole *= 4.0 * epsilon0 * h.powi(3);
    let mut charge = 0.0;
    for z in 0..d[2] {
        for y in 0..d[1] {
            let weight = |c, n| if c == 0 || c + 1 == n { 0.5 } else { 1.0 };
            charge += weight(y, d[1])
                * weight(z, d[2])
                * (v[index([0, y, z], d)] - v[index([1, y, z], d)]);
        }
    }
    charge = 4.0 * epsilon0 * h * charge - epsilon0 * 1e7 * 100.0;
    let center = 50.0 / h;
    let ec = (potential(v, d, [center - 1.0, 0.0, 0.0])
        - potential(v, d, [center + 1.0, 0.0, 0.0]))
        / (2.0 * h);
    print!(
        "OBS h={h} half_step={half_step} cells={cells} Ecenter={ec:.12e} dipole_C_mm={dipole:.12e} excess_charge_C={charge:.12e}"
    );
    for p in [
        [25.0, 0.0, 0.0],
        [45.0, 0.0, 0.0],
        [65.0, 5.0, 5.0],
        [75.0, 20.0, 0.0],
    ] {
        print!(
            " V{}={:.12e}",
            p.iter().map(f64::to_string).collect::<Vec<_>>().join("_"),
            potential(v, d, p.map(|x| x / h))
        );
    }
    println!();
}

fn main() -> GpuResult<()> {
    let args: Vec<_> = std::env::args().collect();
    let original = Path::new(args.get(1).ok_or("expected original directory")?);
    let root = Path::new(args.get(2).ok_or("expected output directory")?);
    let h: f64 = args.get(3).ok_or("expected spacing")?.parse()?;
    let action = args.get(4).ok_or("expected generate, cpu, or observe")?;
    let tolerance = args.get(5).map(|s| s.parse()).transpose()?.unwrap_or(1e-11);
    let repetitions = args.get(6).map(|s| s.parse()).transpose()?.unwrap_or(1);
    let snapshot = args.get(7).map(String::as_str).unwrap_or("native.pa");
    if action == "verify" {
        let generated = read_pa(root.join("potential.pa"))?.array;
        let shipped = read_pa(original.join("sphere_nonlinear_3dp.pa"))?.array;
        assert_eq!(generated.electrodes, shipped.electrodes);
        let generated = read_pa(root.join("epsilon.pa"))?.array;
        let shipped = read_pa(original.join("sphere_nonlinear_3dp-dielectric.pa"))?.array;
        assert_eq!(generated.potentials, shipped.potentials);
        println!("VERIFIED original potential mask and every original cell label");
        return Ok(());
    }
    if action == "generate" {
        std::fs::create_dir_all(root)?;
        let mut pa = read_pa(original.join("sphere_nonlinear_3dp.pa"))?;
        let d = [
            (100.0 / h) as usize + 1,
            (50.0 / h) as usize + 1,
            (50.0 / h) as usize + 1,
        ];
        pa.array.nx = d[0];
        pa.array.ny = d[1];
        pa.array.nz = d[2];
        pa.array.dx_mm = h;
        pa.array.dy_mm = h;
        pa.array.dz_mm = h;
        pa.array.potentials = (0..d.iter().product())
            .map(|i| if i % d[0] == 0 { 1e7 } else { 0.0 })
            .collect();
        pa.array.electrodes = (0..pa.array.potentials.len())
            .map(|i| i % d[0] == 0 || i % d[0] + 1 == d[0])
            .collect();
        write_pa(root.join("potential.pa"), &pa)?;
        let cd = d.map(|n| n - 1);
        pa.array.nx = cd[0];
        pa.array.ny = cd[1];
        pa.array.nz = cd[2];
        pa.array.potentials = (0..cd.iter().product())
            .map(|i| {
                let p = [
                    (i % cd[0]) as f64 + 0.5,
                    ((i / cd[0]) % cd[1]) as f64 + 0.5,
                    (i / (cd[0] * cd[1])) as f64 + 0.5,
                ];
                if (p[0] * h - 50.0).powi(2) + (p[1] * h).powi(2) + (p[2] * h).powi(2) < 100.0 {
                    6.0
                } else {
                    1.0
                }
            })
            .collect();
        pa.array.electrodes = vec![false; pa.array.potentials.len()];
        write_pa(root.join("epsilon.pa"), &pa)?;
        write_pa(root.join("epsilon2.pa"), &pa)?;
        println!("GENERATED h={h} nodes={d:?} cells={cd:?}");
        return Ok(());
    }
    if action == "observe" {
        let pa = read_pa(root.join(snapshot))?;
        observables(
            &pa,
            h,
            snapshot.starts_with("simion") || snapshot.contains("simrep-"),
        );
        if snapshot == "simion.pa" {
            let previous = read_pa(root.join("simion-check.pa"))?;
            let max = pa
                .array
                .potentials
                .iter()
                .zip(previous.array.potentials)
                .map(|(a, b)| (a - b).abs())
                .fold(0.0_f64, f64::max);
            println!("SIMION_TIGHTENING h={h} last_five_updates_max_V={max:.12e}");
            let native = read_pa(root.join("native.pa"))?;
            let max = pa
                .array
                .potentials
                .iter()
                .zip(native.array.potentials)
                .map(|(a, b)| (a - b).abs())
                .fold(0.0_f64, f64::max);
            println!("MODEL_DIFFERENCE h={h} maximum_V={max:.12e}");
        }
        return Ok(());
    }
    let gpu = match action.as_str() {
        "cpu" => None,
        "vulkan" => Some(pollster::block_on(Gpu::new(Backends::VULKAN))?.ok_or("no GPU")?),
        _ => return Err("expected generate, verify, observe, cpu, or vulkan".into()),
    };
    if let Some(gpu) = &gpu {
        println!("ADAPTER {} {:?}", gpu.info.name, gpu.info.device_type);
    }
    let mut pa = read_pa(root.join("potential.pa"))?;
    let eps = read_pa(root.join("epsilon.pa"))?.array.potentials;
    let grid = Grid {
        dims: [pa.array.nx, pa.array.ny, pa.array.nz],
        spacing: [h; 3],
        geometry: Geometry::Planar,
    };
    let mut times = Vec::new();
    for repetition in 1..=repetitions {
        let mut values = pa.array.potentials.clone();
        let start = Instant::now();
        let material = NonlinearPermittivity::from_cells(
            grid,
            &pa.array.electrodes,
            &eps,
            vec![(
                6.0,
                FieldLaw::SaturatingQuadratic {
                    lambda: 8e-11,
                    minimum: 1.0,
                },
            )],
            FieldSampler::SphereOffset {
                center: [50.0, 0.0, 0.0],
                radius: 10.0,
                offset: h,
            },
        )?;
        let report = material.solve(
            &mut values,
            Options {
                relative_tolerance: tolerance,
                max_cycles: 300,
                ..Options::default()
            },
            NonlinearOptions {
                field_tolerance: tolerance,
                ..NonlinearOptions::default()
            },
            gpu.as_ref(),
        )?;
        let seconds = start.elapsed().as_secs_f64();
        assert!(report.converged, "{report:?}");
        println!(
            "NATIVE h={h} repetition={repetition} compute_s={seconds:.9} iterations={} residual={:.12e}",
            report.iterations,
            report.residual_history.last().unwrap().relative
        );
        times.push(seconds);
        pa.array.potentials = values;
        if repetition != repetitions {
            for (i, v) in pa.array.potentials.iter_mut().enumerate() {
                *v = if i % grid.dims[0] == 0 { 1e7 } else { 0.0 };
            }
        }
    }
    times.sort_by(f64::total_cmp);
    println!(
        "NATIVE_MEDIAN h={h} tolerance={tolerance:e} repetitions={repetitions} seconds={:.9}",
        times[times.len() / 2]
    );
    write_pa(root.join(snapshot), &pa)?;
    observables(&pa, h, false);
    Ok(())
}

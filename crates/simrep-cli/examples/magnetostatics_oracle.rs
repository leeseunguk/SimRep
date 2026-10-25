//! Compare exported literal SIMION problems with converged discrete references.
//! Run magnetostatics_audit.lua first. Usage: <directory> [case] [cpu|vulkan].
use std::{path::Path, time::Instant};

use simrep_core::{FieldType, PotentialArray, Symmetry};
use simrep_fly::{
    AnalyticField, Field, FieldSample, FlightOptions, GridField, Particle, fly,
    physics::magnetic::{MagneticPa, VectorPotential},
};
use simrep_io::read_pa;
use simrep_solve::{
    gpu::{Backends, Gpu, GpuResult},
    magnetostatics::{self, Formulation, MagneticProblem},
    mg::{self, Geometry, Grid, Options},
};

const CASES: &[&str] = &[
    "c_magnet_2dp",
    "current_cylinder_2dp",
    "current_sphere_2dc",
    "current_sphere_3dp",
    "emag_quad_2dp",
    "mag_2cylinder_2dc",
    "mag_2ring_2dc",
    "mag_cylinder_2dc",
    "mag_halbach_cylinder_2dp",
    "mag_quad_2dp",
    "mag_ring_2dc",
    "mag_sphere_2dc",
    "mag_sphere_3dp",
    "mu_cylinder_2dp",
    "mu_sphere_2dc",
    "mu_sphere_3dp",
    "mu_torus_3dp",
    "mucurrent_cylinder_2dp",
    "solenoid_2dc",
    "uniform_2dp",
];

fn filename(name: &str, suffix: &str, extension: &str) -> String {
    format!(
        "{name}{}.{extension}",
        if suffix == "base" {
            String::new()
        } else {
            format!("-{suffix}")
        }
    )
}

fn transpose<T: Copy>(values: &[T], nx: usize, ny: usize) -> Vec<T> {
    (0..values.len())
        .map(|i| values[i / ny + nx * (i % ny)])
        .collect()
}

fn ordered<T: Copy>(values: &[T], array: &PotentialArray) -> Vec<T> {
    if array.symmetry == Symmetry::Cylindrical {
        transpose(values, array.nx, array.ny)
    } else {
        values.to_vec()
    }
}

fn maximum(a: &[f64], b: &[f64]) -> f64 {
    a.iter()
        .zip(b)
        .map(|(a, b)| (a - b).abs())
        .fold(0.0, f64::max)
}

fn median(values: &mut [f64]) -> f64 {
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

fn main() -> GpuResult<()> {
    let args: Vec<_> = std::env::args().collect();
    let root = Path::new(args.get(1).ok_or("expected native export directory")?);
    let selected = args.get(2).filter(|s| s.as_str() != "all");
    let gpu = match args.get(3).map(String::as_str).unwrap_or("cpu") {
        "cpu" => None,
        "vulkan" => Some(pollster::block_on(Gpu::new(Backends::VULKAN))?.ok_or("no Vulkan GPU")?),
        _ => return Err("expected cpu or vulkan".into()),
    };
    for &name in CASES.iter().filter(|&&n| selected.is_none_or(|s| s == n)) {
        let jobs = std::fs::read_to_string(root.join(format!("{name}.jobs.csv")))?;
        if jobs.trim().is_empty() {
            return Err(format!("{name}: native export is incomplete").into());
        }
        let mut cartesian_arrays = Vec::new();
        let mut cartesian_references = Vec::new();
        let mut cartesian_compute = 0.0;
        let mut cartesian_native = 0.0;
        for line in jobs.lines() {
            let columns: Vec<_> = line.split(',').collect();
            if columns.len() != 5 {
                return Err("invalid native jobs CSV".into());
            }
            let suffix = columns[0];
            let kind = match columns[1] {
                "scalar" => Formulation::Scalar,
                "planar_az" => Formulation::PlanarAz,
                "axisymmetric_psi" => Formulation::AxisymmetricPsi,
                "cartesian_a" => Formulation::CartesianA,
                _ => return Err("invalid native formulation".into()),
            };
            let input = read_pa(root.join(filename(name, suffix, "input.pa")))?;
            let a = &input.array;
            let cylindrical = a.symmetry == Symmetry::Cylindrical;
            let grid = Grid {
                dims: if cylindrical {
                    [a.ny, a.nx, 1]
                } else {
                    [a.nx, a.ny, a.nz]
                },
                spacing: if cylindrical {
                    [a.dy_mm, a.dx_mm, a.dz_mm]
                } else {
                    [a.dx_mm, a.dy_mm, a.dz_mm]
                },
                geometry: if cylindrical {
                    Geometry::Cylindrical { radial_origin: 0.0 }
                } else {
                    Geometry::Planar
                },
            };
            let fixed = ordered(&a.electrodes, a);
            let mut initial = ordered(&a.potentials, a);
            for (v, &fixed) in initial.iter_mut().zip(&fixed) {
                if !fixed {
                    *v = 0.0;
                }
            }
            let cells = grid
                .dims
                .map(|n| n.saturating_sub(1).max(1))
                .iter()
                .product();
            let prepare = Instant::now();
            let mu = if columns[3].is_empty() {
                vec![1.0; cells]
            } else {
                magnetostatics::load_cells(
                    &root.join(filename(name, columns[3], "input.pa")),
                    grid,
                    cylindrical,
                )?
            };
            let source = if columns[2].is_empty() {
                vec![0.0; cells]
            } else {
                magnetostatics::load_cells(
                    &root.join(filename(name, columns[2], "input.pa")),
                    grid,
                    cylindrical,
                )?
            };
            let mut magnetization = vec![[0.0; 3]; cells];
            if kind == Formulation::Scalar {
                for (component, label) in ["Mx", "My", "Mz"].iter().enumerate() {
                    let path = root.join(filename(name, label, "input.pa"));
                    if path.is_file() {
                        let axis = if cylindrical && component < 2 {
                            1 - component
                        } else {
                            component
                        };
                        for (m, value) in magnetization.iter_mut().zip(magnetostatics::load_cells(
                            &path,
                            grid,
                            cylindrical,
                        )?) {
                            m[axis] = value;
                        }
                    }
                }
            }
            let problem = match kind {
                Formulation::Scalar => MagneticProblem::scalar(
                    grid,
                    &fixed,
                    &mu,
                    &magnetization,
                    Some(&source),
                    input.ng as f64,
                )?,
                Formulation::CartesianA => {
                    if mu.iter().any(|&v| v != mu[0]) {
                        return Err("nonuniform Cartesian permeability".into());
                    }
                    MagneticProblem::cartesian_current_component(
                        grid,
                        &fixed,
                        mu[0],
                        &source,
                        input.ng as f64,
                    )?
                }
                _ => MagneticProblem::currents(grid, &fixed, &mu, &source, input.ng as f64)?,
            };
            let preparation_s = prepare.elapsed().as_secs_f64();
            let mut options = Options {
                relative_tolerance: if name == "uniform_2dp" { 1e-15 } else { 1e-10 },
                max_cycles: 2000,
                ..Default::default()
            };
            let native_default = read_pa(root.join(filename(name, suffix, "default.pa")))?.array;
            let native_strict = read_pa(root.join(filename(name, suffix, "strict.pa")))?.array;
            let default = ordered(&native_default.potentials, a);
            let strict = ordered(&native_strict.potentials, a);
            let mut reference = initial.clone();
            let report = problem.solve(
                &mut reference,
                Options {
                    relative_tolerance: 1e-12,
                    ..options
                },
            )?;
            if name == "uniform_2dp" {
                // This case has an analytic reference, more accurate than a
                // residual-scaled discrete solve on the same grid.
                for (i, v) in reference.iter_mut().enumerate() {
                    *v = if kind == Formulation::Scalar {
                        100.0 * (a.nx - 1 - i % a.nx) as f64 / (a.nx - 1) as f64
                    } else {
                        100.0 * (i / a.nx % a.ny) as f64 / (a.ny - 1) as f64
                    };
                }
            } else if !report.converged {
                // Strong material contrasts can put the f64 row residual
                // below the requested threshold's representable floor.
                // Certify the iterate independently from the native seed;
                // never call a large residual alone a converged reference.
                let mut independent = strict.clone();
                let second = problem.solve(
                    &mut independent,
                    Options {
                        relative_tolerance: 1e-12,
                        ..options
                    },
                )?;
                let difference = maximum(&reference, &independent);
                let magnitude = reference.iter().map(|v| v.abs()).fold(1.0, f64::max);
                let bound = 64.0 * f64::EPSILON * magnitude;
                if difference > bound {
                    return Err(format!("{name}/{suffix}: unresolved reference: residual={:?}, independent difference={difference:e}, 64-ulp bound={bound:e}",report.residual_history.last()).into());
                }
                options.absolute_tolerance = 4.0
                    * report
                        .residual_history
                        .last()
                        .unwrap()
                        .absolute
                        .max(second.residual_history.last().unwrap().absolute);
                println!(
                    "MAGNETIC_REFERENCE_F64_FLOOR,{name},{suffix},independent_max_difference={difference:.9e},potential_64ulp_bound={bound:.9e},absolute_residual_floor={:.9e}",
                    options.absolute_tolerance / 4.0
                );
            }
            if kind == Formulation::Scalar
                && cylindrical
                && !columns[2].is_empty()
                && mu.iter().all(|&v| v == 1.0)
            {
                // Native cylindrical charge averaging can be distinguished
                // from iteration error without treating SIMION as truth.
                let planar = Grid {
                    geometry: Geometry::Planar,
                    ..grid
                };
                let arithmetic: Vec<_> = magnetostatics::cell_source(planar, &source)?
                    .into_iter()
                    .map(|v| v * 4.0 * std::f64::consts::PI / input.ng as f64)
                    .collect();
                let mut arithmetic_reference = initial.clone();
                let r = mg::solve_poisson(
                    &mut arithmetic_reference,
                    &fixed,
                    &arithmetic,
                    grid,
                    Options {
                        relative_tolerance: 1e-12,
                        ..options
                    },
                )?;
                if !r.converged {
                    return Err("arithmetic-source comparison reference did not converge".into());
                }
                let mut seed = strict.clone();
                let native_residual = mg::solve_poisson(
                    &mut seed,
                    &fixed,
                    &arithmetic,
                    grid,
                    Options {
                        max_cycles: 0,
                        ..options
                    },
                )?
                .residual_history[0]
                    .relative;
                println!(
                    "MAGNETIC_SOURCE_MODELS,{name},{suffix},finite_volume_vs_arithmetic={:.9e},native_strict_vs_arithmetic={:.9e},native_arithmetic_relative_residual={native_residual:.9e}",
                    maximum(&reference, &arithmetic_reference),
                    maximum(&strict, &arithmetic_reference)
                );
            }
            if kind == Formulation::Scalar && magnetization.iter().any(|m| *m != [0.0; 3]) {
                let conservative = MagneticProblem::scalar(
                    grid,
                    &fixed,
                    &mu,
                    &magnetization,
                    None,
                    input.ng as f64,
                )?;
                let mut conservative_reference = initial.clone();
                let r = conservative.solve(&mut conservative_reference, options)?;
                if !r.converged {
                    return Err("conservative-M source model comparison did not converge".into());
                }
                println!(
                    "MAGNETIC_MAGNETIZATION_MODELS,{name},{suffix},conservative_M_vs_literal_Lua_source={:.9e},relative_residual={:.9e}",
                    maximum(&reference, &conservative_reference),
                    r.residual_history.last().unwrap().relative
                );
            }
            let mut native_residual = strict.clone();
            let native_residual = problem
                .solve(
                    &mut native_residual,
                    Options {
                        max_cycles: 0,
                        ..options
                    },
                )?
                .residual_history[0]
                .relative;
            let mut compute = Vec::new();
            let mut setup = Vec::new();
            let mut transfers = Vec::new();
            let mut actual = initial.clone();
            let mut relative = 0.0;
            for _ in 0..3 {
                actual.copy_from_slice(&initial);
                if let Some(gpu) = &gpu {
                    let r = problem.solve_gpu(gpu, &mut actual, options)?;
                    if !r.converged {
                        return Err(format!("{name}/{suffix}: GPU not converged").into());
                    }
                    compute.push(r.timings.solve.as_secs_f64());
                    setup.push(r.timings.setup.as_secs_f64());
                    transfers.push((r.timings.upload + r.timings.readback).as_secs_f64());
                    relative = r.residual_history.last().unwrap().relative;
                } else {
                    let r = problem.solve(&mut actual, options)?;
                    if !r.converged {
                        return Err(format!("{name}/{suffix}: CPU not converged").into());
                    }
                    compute.push(r.timings.solve.as_secs_f64());
                    setup.push(r.timings.setup.as_secs_f64());
                    transfers.push(0.0);
                    relative = r.residual_history.last().unwrap().relative;
                }
            }
            let compute_s = median(&mut compute);
            let setup_s = median(&mut setup);
            let transfer_s = median(&mut transfers);
            let our_error = maximum(&actual, &reference);
            let default_error = maximum(&default, &reference);
            let model_gap = maximum(&strict, &reference);
            let native_numerical = maximum(&default, &strict);
            let times = std::fs::read_to_string(root.join(format!("{name}.default.times.csv")))?;
            let mut times: Vec<f64> = times
                .lines()
                .filter_map(|l| {
                    let c: Vec<_> = l.split(',').collect();
                    (c[0] == suffix).then(|| c[2].parse::<f64>())
                })
                .collect::<Result<_, _>>()?;
            let mut native_s = if times.is_empty() {
                f64::NAN
            } else {
                median(&mut times)
            };
            if kind == Formulation::CartesianA {
                cartesian_compute += compute_s;
                if suffix == "Ax" {
                    cartesian_native = native_s;
                }
                native_s = f64::NAN;
            }
            let norm = reference.iter().map(|x| x * x).sum::<f64>();
            let scale_fit = strict
                .iter()
                .zip(&reference)
                .map(|(s, r)| s * r)
                .sum::<f64>()
                / norm;
            println!(
                "MAGNETIC_RESULT,{name},{suffix},{kind:?},nodes={},reps=3,preparation_s={preparation_s:.9},setup_s={setup_s:.9},compute_s={compute_s:.9},transfer_s={transfer_s:.9},native_default_s={native_s:.9},speedup={:.6},our_vs_converged={our_error:.9e},native_default_vs_converged={default_error:.9e},native_strict_model_gap={model_gap:.9e},native_default_vs_strict={native_numerical:.9e},native_strict_relative_residual={native_residual:.9e},our_relative_residual={relative:.9e},native_scale_fit={scale_fit:.12},matched={}",
                actual.len(),
                native_s / compute_s,
                our_error <= default_error + 1e-11
            );
            let mut array = a.clone();
            let solid_cells = if cylindrical {
                let d = grid.dims.map(|n| n.saturating_sub(1).max(1));
                transpose(&problem.solid_cells, d[0], d[1])
            } else {
                problem.solid_cells.clone()
            };
            array.potentials = if cylindrical {
                transpose(&actual, a.ny, a.nx)
            } else {
                actual
            };
            if kind != Formulation::Scalar {
                array.anti_mirrors.x = columns[4].contains('x');
                array.anti_mirrors.y = columns[4].contains('y');
                array.anti_mirrors.z = columns[4].contains('z');
            }
            let mut reference_array = array.clone();
            reference_array.potentials = if cylindrical {
                transpose(&reference, a.ny, a.nx)
            } else {
                reference
            };
            if kind == Formulation::CartesianA {
                cartesian_arrays.push(array);
                cartesian_references.push(reference_array);
            } else {
                let field = reconstructed(
                    array,
                    kind,
                    input.ng as f64,
                    columns[4],
                    solid_cells.clone(),
                )?;
                let reference_field = reconstructed(
                    reference_array,
                    kind,
                    input.ng as f64,
                    columns[4],
                    solid_cells.clone(),
                )?;
                native_field_probes(root, name, suffix, &*field, a, &solid_cells)?;
                flight_probe(name, suffix, &*field, &*reference_field, a)?;
            }
        }
        if !cartesian_arrays.is_empty() {
            let arrays: [PotentialArray; 3] = cartesian_arrays
                .try_into()
                .map_err(|_| "expected three Cartesian components")?;
            let shape = arrays[0].clone();
            let references: [PotentialArray; 3] = cartesian_references
                .try_into()
                .map_err(|_| "expected three Cartesian references")?;
            let field = VectorPotential::cartesian(
                arrays,
                1.0,
                [0.0; 3],
                [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
                1.0,
                None,
            )?;
            let reference_field = VectorPotential::cartesian(
                references,
                1.0,
                [0.0; 3],
                [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
                1.0,
                None,
            )?;
            let cells = shape.nx.saturating_sub(1).max(1)
                * shape.ny.saturating_sub(1).max(1)
                * shape.nz.saturating_sub(1).max(1);
            native_field_probes(root, name, "Ax", &field, &shape, &vec![false; cells])?;
            flight_probe(name, "Ax+Ay+Az", &field, &reference_field, &shape)?;
            println!(
                "MAGNETIC_CARTESIAN_GROUP,{name},reps=3,compute_s={cartesian_compute:.9},native_default_s={cartesian_native:.9},speedup={:.6}",
                cartesian_native / cartesian_compute
            );
        }
    }
    Ok(())
}

fn reconstructed(
    mut array: PotentialArray,
    kind: Formulation,
    ng: f64,
    anti: &str,
    solid: Vec<bool>,
) -> GpuResult<Box<dyn Field>> {
    Ok(
        if kind == Formulation::Scalar && array.potentials.len() > 1_048_576 {
            // Same C2 boundary jets, without 216 powers per large 3D cell.
            array.field_type = FieldType::Electrostatic;
            let grid = GridField::new_compact(array)?;
            Box::new(AnalyticField(move |p, t| FieldSample {
                magnetic_gauss: grid.sample(p, t).electric_v_per_mm.map(|v| v * ng),
                ..Default::default()
            }))
        } else if kind == Formulation::Scalar {
            let mut field = MagneticPa::new(array, ng)?;
            if let Some(axis) = anti.chars().next().map(|c| (c as u8 - b'x') as usize) {
                field = field.with_polarity_flip_axis(axis)?;
            }
            Box::new(field.with_material_cells(solid)?)
        } else {
            Box::new(VectorPotential::new(array, kind, ng)?.with_material_cells(solid)?)
        },
    )
}

fn native_field_probes(
    root: &Path,
    name: &str,
    suffix: &str,
    field: &dyn Field,
    array: &PotentialArray,
    solid: &[bool],
) -> GpuResult<()> {
    let transforms = std::fs::read_to_string(root.join(format!("{name}.transforms.csv")))?;
    let transform = transforms
        .lines()
        .find(|l| l.split(',').next() == Some(suffix))
        .ok_or("missing native placement")?;
    let transform: Vec<f64> = transform
        .split(',')
        .skip(1)
        .map(str::parse)
        .collect::<Result<_, _>>()?;
    let origin = [transform[0], transform[1], transform[2]];
    let steps: [[f64; 3]; 3] =
        std::array::from_fn(|a| std::array::from_fn(|i| transform[3 + 3 * a + i] - origin[i]));
    let dot = |a: [f64; 3], b: [f64; 3]| a.iter().zip(b).map(|(x, y)| x * y).sum::<f64>();
    let lengths = steps.map(|v| dot(v, v).sqrt());
    let axes: [[f64; 3]; 3] = std::array::from_fn(|a| steps[a].map(|v| v / lengths[a]));
    let spacing = [array.dx_mm, array.dy_mm, array.dz_mm];
    let dims = [array.nx, array.ny, array.nz].map(|n| n.saturating_sub(1).max(1));
    for mode in ["default", "strict"] {
        let text =
            std::fs::read_to_string(root.join(filename(name, suffix, &format!("{mode}.b.csv"))))?;
        let mut maximum: f64 = 0.0;
        let mut count = 0;
        for line in text.lines() {
            let v: Vec<f64> = line.split(',').map(str::parse).collect::<Result<_, _>>()?;
            let delta = std::array::from_fn(|a| v[a] - origin[a]);
            let local: [f64; 3] =
                std::array::from_fn(|a| dot(delta, axes[a]) * spacing[a] / lengths[a]);
            let cell: [usize; 3] = std::array::from_fn(|a| {
                ((local[a] / spacing[a]).floor().max(0.0) as usize).min(dims[a] - 1)
            });
            if solid[cell[0] + dims[0] * (cell[1] + dims[1] * cell[2])] {
                continue;
            }
            let b = field.sample(local, 0.0).magnetic_gauss;
            let world: [f64; 3] = std::array::from_fn(|a| (0..3).map(|i| axes[i][a] * b[i]).sum());
            maximum = maximum.max(
                (0..3)
                    .map(|a| (world[a] - v[3 + a]).powi(2))
                    .sum::<f64>()
                    .sqrt(),
            );
            count += 1;
        }
        println!(
            "MAGNETIC_NATIVE_FIELD,{name},{suffix},{mode},vacuum_probes={count},max_vector_error_gauss={maximum:.9e}"
        );
    }
    Ok(())
}

fn flight_probe(
    name: &str,
    suffix: &str,
    field: &dyn Field,
    reference: &dyn Field,
    array: &PotentialArray,
) -> GpuResult<()> {
    let start = [
        0.73 * (array.nx - 1) as f64 * array.dx_mm,
        0.61 * (array.ny - 1) as f64 * array.dy_mm,
        if array.nz > 1 {
            0.57 * (array.nz - 1) as f64 * array.dz_mm
        } else {
            0.0
        },
    ];
    let ion = Particle::new(100.0, 1.0, start, [0.1, 0.07, 0.0], 0.0)?;
    let sampled = AnalyticField(|p, t| field.sample(p, t));
    let options = FlightOptions {
        duration_us: 0.1,
        max_step_us: 0.001,
        ..Default::default()
    };
    let r = fly(&sampled, &ion, &options)?;
    let reference_sampled = AnalyticField(|p, t| reference.sample(p, t));
    let reference_result = fly(&reference_sampled, &ion, &options)?;
    let endpoint_difference = (0..3)
        .map(|a| (r.final_state.position_mm[a] - reference_result.final_state.position_mm[a]).abs())
        .fold(0.0, f64::max);
    let velocity_difference = (0..3)
        .map(|a| {
            (r.final_state.velocity_mm_per_us[a]
                - reference_result.final_state.velocity_mm_per_us[a])
                .abs()
        })
        .fold(0.0, f64::max);
    if endpoint_difference > 1e-6 || velocity_difference > 1e-6 {
        return Err(format!("{name}/{suffix}: flight differs from converged field: position={endpoint_difference:e}, velocity={velocity_difference:e}").into());
    }
    if r.splat_reason != simrep_fly::SplatReason::TimeLimit {
        return Err(format!("{name}/{suffix}: field-flight failed: {:?}", r.splat_reason).into());
    }
    let initial = ion
        .initial
        .velocity_mm_per_us
        .iter()
        .map(|v| v * v)
        .sum::<f64>();
    let final_energy = r
        .final_state
        .velocity_mm_per_us
        .iter()
        .map(|v| v * v)
        .sum::<f64>();
    if (final_energy / initial - 1.0).abs() > 1e-8 {
        return Err("magnetic flight did not conserve kinetic energy".into());
    }
    println!(
        "MAGNETIC_FLIGHT,{name},{suffix},steps={},relative_energy_error={:.9e},endpoint_vs_converged_mm={endpoint_difference:.9e},velocity_vs_converged_mm_per_us={velocity_difference:.9e}",
        r.accepted_steps,
        (final_energy / initial - 1.0).abs()
    );
    Ok(())
}

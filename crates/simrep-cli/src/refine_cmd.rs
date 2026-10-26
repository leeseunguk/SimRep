use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use clap::{Args, ValueEnum};
use simrep_core::{PotentialArray, Symmetry};
use simrep_io::{Pa0Metadata, PaFile, read_pa, write_pa};
use simrep_solve::dielectric::{self, LoadedMaterial};
use simrep_solve::gpu::{DeviceType, Gpu, GpuResult};
use simrep_solve::mg::{Geometry, Grid, Options, solve};

use crate::{Backend, parse_tolerance};

mod magnetic_refine;

// SIMION's PA1..PAn convention is 10000 V, not 1 V. Fast adjust divides
// the requested voltage by this reference before superposing the bases.
const REFERENCE_VOLTAGE: f64 = 10_000.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum RefineDevice {
    Auto,
    Cpu,
    Gpu,
}

impl RefineDevice {
    fn requests_gpu(self, grid: Grid, nonlinear: Option<bool>) -> bool {
        self == Self::Gpu
            || self == Self::Auto
                && nonlinear.is_some_and(|nonlinear| dielectric::prefer_gpu(grid, nonlinear))
    }
}

#[derive(Args)]
pub struct RefineOptions {
    /// SIMION electrode definition (.pa#) or ordinary potential array (.pa).
    file: PathBuf,
    /// Auto uses the measured linear/nonlinear 3D dielectric crossover.
    #[arg(long, value_enum, default_value = "auto")]
    device: RefineDevice,
    /// GPU backend used when GPU refinement is selected.
    #[arg(long, value_enum, default_value = "vulkan")]
    backend: Backend,
    /// Relative infinity norm of the physical f64 Laplace residual.
    #[arg(long, default_value = "1e-10", value_parser = parse_tolerance)]
    tolerance: f64,
    /// Maximum multigrid cycles for each refinement.
    #[arg(long, default_value_t = 100)]
    max_cycles: usize,
    /// SIMION cell-permittivity PA or linear/nonlinear JSON material specification.
    #[arg(long)]
    permittivity: Option<PathBuf>,
    /// JSON permeability/magnetization/current specification for a magnetic PA.
    #[arg(long, conflicts_with = "permittivity")]
    magnetic_material: Option<PathBuf>,
}

#[derive(Default)]
struct Timings {
    setup: Duration,
    solve: Duration,
    transfers: Duration,
    io: Duration,
}

/// Translate SIMION's axial-x/radial-y storage to the solver's radial-x/axial-y
/// storage. Planar layouts, including nz=1, need no permutation.
struct Layout {
    grid: Grid,
    fixed: Vec<bool>,
    cylindrical: bool,
}

impl Layout {
    fn new(a: &PotentialArray) -> Self {
        let cylindrical = a.symmetry == Symmetry::Cylindrical;
        let mut fixed = a.electrodes.clone();
        for (i, value) in fixed.iter_mut().enumerate() {
            if antimirror_plane(a, i) {
                *value = true;
            }
        }
        if cylindrical {
            fixed = transpose(&fixed, a.nx, a.ny);
        }
        Self {
            grid: if cylindrical {
                Grid {
                    dims: [a.ny, a.nx, 1],
                    spacing: [a.dy_mm, a.dx_mm, a.dz_mm],
                    geometry: Geometry::Cylindrical { radial_origin: 0.0 },
                }
            } else {
                Grid {
                    dims: [a.nx, a.ny, a.nz],
                    spacing: [a.dx_mm, a.dy_mm, a.dz_mm],
                    geometry: Geometry::Planar,
                }
            },
            fixed,
            cylindrical,
        }
    }

    fn refine(
        &self,
        a: &mut PotentialArray,
        options: Options,
        gpu: Option<&Gpu>,
        permittivity: Option<&LoadedMaterial>,
        timings: &mut Timings,
    ) -> GpuResult<(bool, usize, f64)> {
        let start = Instant::now();
        // Odd reflection has phi=0 on the origin plane. These solver-only
        // constraints must not turn free grid nodes into file electrodes.
        for i in 0..a.potentials.len() {
            if antimirror_plane(a, i) {
                if a.electrodes[i] && a.potentials[i] != 0.0 {
                    return Err("nonzero electrode on an anti-mirror plane".into());
                }
                a.potentials[i] = 0.0;
            }
        }
        let mut values = if self.cylindrical {
            transpose(&a.potentials, a.nx, a.ny)
        } else {
            std::mem::take(&mut a.potentials)
        };
        timings.setup += start.elapsed();
        let (converged, cycles, residual) = if let Some(material) = permittivity {
            let report = material.solve(&mut values, options, gpu)?;
            timings.solve += report.compute;
            if let Some(change) = report.field_change {
                println!(
                    "  nonlinear outer iterations: {}, relative field change: {change:.9e}",
                    report.iterations
                );
            }
            (report.converged, report.iterations, report.residual)
        } else if let Some(gpu) = gpu {
            let report = gpu.solve(&mut values, &self.fixed, self.grid, options)?;
            timings.setup += report.timings.setup;
            timings.solve += report.timings.upload + report.timings.solve + report.timings.readback;
            timings.transfers += report.timings.upload + report.timings.readback;
            println!(
                "  GPU setup: {:.3} ms, upload: {:.3} ms, solve: {:.3} ms, readback: {:.3} ms; residual: {} f64",
                ms(report.timings.setup),
                ms(report.timings.upload),
                ms(report.timings.solve),
                ms(report.timings.readback),
                if report.gpu_f64 { "GPU" } else { "CPU" },
            );
            (
                report.converged,
                report.iterations,
                *report.residual_history.last().unwrap(),
            )
        } else {
            let report = solve(&mut values, &self.fixed, self.grid, options)?;
            timings.setup += report.timings.setup;
            timings.solve += report.timings.solve;
            (
                report.converged,
                report.iterations,
                *report.residual_history.last().unwrap(),
            )
        };
        let start = Instant::now();
        a.potentials = if self.cylindrical {
            transpose(&values, a.ny, a.nx)
        } else {
            values
        };
        timings.setup += start.elapsed();
        println!(
            "  cycles: {cycles}, residual f64: absolute = {:.9e} V/mm^2, relative = {:.9e}",
            residual.absolute, residual.relative,
        );
        if !converged {
            return Err(format!("refinement did not converge after {cycles} cycles").into());
        }
        Ok((converged, cycles, residual.relative))
    }
}

fn transpose<T: Copy>(values: &[T], nx: usize, ny: usize) -> Vec<T> {
    (0..values.len())
        .map(|i| values[i / ny + nx * (i % ny)])
        .collect()
}

fn antimirror_plane(a: &PotentialArray, i: usize) -> bool {
    (a.anti_mirrors.x && i.is_multiple_of(a.nx))
        || (a.anti_mirrors.y && (i / a.nx).is_multiple_of(a.ny))
        || (a.anti_mirrors.z && i / (a.nx * a.ny) == 0)
}

fn electrode_number(value: f64) -> Option<usize> {
    if (1.0..=1000.0).contains(&value) && value.fract() == 0.0 {
        Some(value as usize)
    } else {
        None
    }
}

fn electrode_table(a: &PotentialArray) -> Vec<i64> {
    let mut indices = vec![-1; 1001];
    for (i, (&value, &fixed)) in a.potentials.iter().zip(&a.electrodes).enumerate() {
        if fixed {
            let slot = electrode_number(value).unwrap_or(0);
            if indices[slot] == -1
                || (slot == 0 && value.abs() > a.potentials[indices[slot] as usize].abs())
            {
                indices[slot] = i as i64;
            }
        }
    }
    let entries = indices
        .iter()
        .rposition(|&i| i != -1)
        .map_or(32, |i| (i + 1).max(32));
    indices.truncate(entries);
    indices
}

fn initialize_basis(output: &mut PaFile, source: &PaFile, slot: usize, scalable_voltage: f64) {
    output.pa0 = None;
    output.max_voltage = source.max_voltage.max(100_000.0);
    for (i, value) in output.array.potentials.iter_mut().enumerate() {
        let input = source.array.potentials[i];
        *value = if !source.array.electrodes[i] {
            0.0
        } else if slot == 0 && electrode_number(input).is_none() {
            if scalable_voltage == 0.0 {
                0.0
            } else {
                (input / scalable_voltage) * REFERENCE_VOLTAGE
            }
        } else if electrode_number(input) == Some(slot) {
            REFERENCE_VOLTAGE
        } else {
            0.0
        };
    }
}

fn output(path: &Path, pa: &PaFile, timings: &mut Timings) -> GpuResult<()> {
    let start = Instant::now();
    write_pa(path, pa).map_err(|error| format!("{}: {error}", path.display()))?;
    timings.io += start.elapsed();
    println!("Wrote {}", path.display());
    Ok(())
}

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

pub fn run(options: RefineOptions) -> GpuResult<()> {
    if options.magnetic_material.is_some() {
        return magnetic_refine::run(options);
    }
    let total = Instant::now();
    let extension = options
        .file
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    let adjustable = extension.eq_ignore_ascii_case("pa#");
    if !adjustable && !extension.eq_ignore_ascii_case("pa") {
        return Err("refine input must have a .pa or .pa# extension".into());
    }
    let start = Instant::now();
    let mut pa =
        read_pa(&options.file).map_err(|error| format!("{}: {error}", options.file.display()))?;
    let mut timings = Timings {
        io: start.elapsed(),
        ..Timings::default()
    };
    if !pa.array.refinable {
        return Err("PA is marked non-refinable (field map)".into());
    }
    let start = Instant::now();
    let layout = Layout::new(&pa.array);
    let permittivity = options
        .permittivity
        .as_ref()
        .map(|path| dielectric::load_material(path, layout.grid, &layout.fixed, layout.cylindrical))
        .transpose()?;
    if adjustable
        && permittivity
            .as_ref()
            .is_some_and(LoadedMaterial::is_nonlinear)
    {
        return Err("nonlinear permittivity cannot use PA# fast-adjust bases; refine an ordinary PA at its actual voltages".into());
    }
    let request_gpu = options.device.requests_gpu(
        layout.grid,
        permittivity.as_ref().map(LoadedMaterial::is_nonlinear),
    );
    let mut reason = match options.device {
        RefineDevice::Cpu => "explicit --device cpu",
        RefineDevice::Gpu => "explicit --device gpu",
        RefineDevice::Auto if permittivity.is_none() => {
            "no dielectric material; use --device gpu for GPU refine"
        }
        RefineDevice::Auto if request_gpu => "dielectric grid meets measured GPU crossover",
        RefineDevice::Auto => "dielectric grid below measured GPU crossover",
    };
    let gpu = if request_gpu {
        let candidate = pollster::block_on(Gpu::new(options.backend.backends()))?;
        if let Some(gpu) = &candidate
            && options.device == RefineDevice::Gpu
        {
            crate::require_hardware_gpu(&gpu.info)?;
        }
        reason = match &candidate {
            None => "no adapter available for the requested backend",
            Some(gpu) if gpu.info.device_type == DeviceType::Cpu => {
                "adapter is a CPU/software adapter"
            }
            _ => reason,
        };
        let gpu = candidate.filter(|gpu| {
            options.device == RefineDevice::Gpu || gpu.info.device_type != DeviceType::Cpu
        });
        if gpu.is_none() && options.device == RefineDevice::Gpu {
            return Err("no GPU adapter found".into());
        }
        gpu
    } else {
        None
    };
    if let Some(gpu) = &gpu {
        println!(
            "Adapter: {} ({:?}, {:?})",
            gpu.info.name, gpu.info.backend, gpu.info.device_type
        );
        crate::report_gpu_residual(gpu.has_f64());
    }
    println!(
        "Device: {} (requested {:?}); {}",
        if gpu.is_some() { "GPU" } else { "CPU" },
        options.device,
        if options.device == RefineDevice::Auto {
            format!(
                "auto: {} ({reason})",
                if gpu.is_some() { "GPU" } else { "CPU" }
            )
        } else {
            "explicit --device".to_owned()
        }
    );
    println!(
        "Grid: {} x {} x {}, {:?}, spacing mm: {} x {} x {}",
        pa.array.nx,
        pa.array.ny,
        pa.array.nz,
        pa.array.symmetry,
        pa.array.dx_mm,
        pa.array.dy_mm,
        pa.array.dz_mm,
    );
    let solver_options = Options {
        relative_tolerance: options.tolerance,
        max_cycles: options.max_cycles,
        ..Options::default()
    };
    timings.setup += start.elapsed();
    let mut cycles = 0;
    let mut final_residual = 0.0_f64;
    let mut converged = true;
    let mut output_paths = Vec::new();
    if adjustable {
        let start = Instant::now();
        let indices = electrode_table(&pa.array);
        if indices.iter().all(|&i| i == -1) {
            return Err("PA# has no electrodes".into());
        }
        let mut basis = pa.clone();
        let mut base = pa.clone();
        base.array.potentials.fill(0.0);
        let scalable_voltage = if indices[0] == -1 {
            0.0
        } else {
            pa.array.potentials[indices[0] as usize]
        };
        timings.setup += start.elapsed();
        for (slot, &index) in indices.iter().enumerate() {
            if index == -1 {
                continue;
            }
            let suffix = if slot == 0 {
                "pa_".into()
            } else {
                format!("pa{slot}")
            };
            println!("Refining {suffix}");
            let start = Instant::now();
            initialize_basis(&mut basis, &pa, slot, scalable_voltage);
            timings.setup += start.elapsed();
            let (basis_converged, basis_cycles, basis_residual) = layout.refine(
                &mut basis.array,
                solver_options,
                gpu.as_ref(),
                permittivity.as_ref(),
                &mut timings,
            )?;
            cycles += basis_cycles;
            final_residual = final_residual.max(basis_residual);
            converged &= basis_converged;
            let path = options.file.with_extension(suffix);
            output(&path, &basis, &mut timings)?;
            output_paths.push(path);
            if slot == 0 {
                let start = Instant::now();
                base.array
                    .potentials
                    .iter_mut()
                    .zip(&basis.array.potentials)
                    .enumerate()
                    .for_each(|(i, (base, &value))| {
                        *base = if pa.array.electrodes[i] {
                            let input = pa.array.potentials[i];
                            if electrode_number(input).is_none() {
                                input
                            } else {
                                0.0
                            }
                        } else {
                            value * (scalable_voltage / REFERENCE_VOLTAGE)
                        };
                    });
                timings.setup += start.elapsed();
            }
        }
        base.pa0 = Some(Pa0Metadata {
            reference_voltage: REFERENCE_VOLTAGE,
            electrode_indices: indices,
            scalable: None,
        });
        let path = options.file.with_extension("pa0");
        output(&path, &base, &mut timings)?;
        output_paths.push(path);
    } else {
        let (basis_converged, basis_cycles, basis_residual) = layout.refine(
            &mut pa.array,
            solver_options,
            gpu.as_ref(),
            permittivity.as_ref(),
            &mut timings,
        )?;
        cycles += basis_cycles;
        final_residual = final_residual.max(basis_residual);
        converged &= basis_converged;
        output(&options.file, &pa, &mut timings)?;
        output_paths.push(options.file.clone());
    }
    println!(
        "Setup: {:.3} ms, solve: {:.3} ms, IO: {:.3} ms, end-to-end: {:.3} ms",
        ms(timings.setup),
        ms(timings.solve),
        ms(timings.io),
        ms(total.elapsed()),
    );
    crate::json::set_summary(serde_json::json!({
        "requested": {
            "device": options.device.to_possible_value().unwrap().get_name(),
            "backend": options.backend.to_possible_value().unwrap().get_name(),
            "precision": "f64",
        },
        "executed": {
            "device": if gpu.is_some() { "gpu" } else { "cpu" },
            "backend": gpu.as_ref().map(|gpu| format!("{:?}", gpu.info.backend).to_lowercase()),
            "precision": if gpu.is_some() { "mixed-f32-f64" } else { "f64" },
        },
        "fallback_reason": if gpu.as_ref().is_some_and(|g| !g.has_f64()) {
            Some("adapter lacks native f64; CPU f64 residual with GPU f32 corrections")
        } else {
            (request_gpu && gpu.is_none()).then_some(reason)
        },
        "reason": reason,
        "cycles": cycles,
        "final_residual": final_residual,
        "tolerance": options.tolerance,
        "converged": converged,
        "compute_ms": ms(timings.solve - timings.transfers),
        "prep_ms": ms(timings.setup + timings.transfers),
        "output_paths": output_paths,
    }));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn options(file: PathBuf) -> RefineOptions {
        RefineOptions {
            file,
            device: RefineDevice::Cpu,
            backend: Backend::Vulkan,
            tolerance: 1e-12,
            max_cycles: 100,
            permittivity: None,
            magnetic_material: None,
        }
    }

    #[test]
    fn pasharp_round_trip_and_basis_superposition_match_full_solve() {
        for symmetry in [Symmetry::Planar, Symmetry::Cylindrical] {
            let dir = std::env::temp_dir().join(format!(
                "simrep-refine-basis-{}-{symmetry:?}",
                std::process::id()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            let input = dir.join("small.pa#");
            let mut a = PotentialArray::new(11, 9, 1);
            a.symmetry = symmetry;
            a.mirrors.y = true;
            a.dx_mm = 0.25;
            a.dy_mm = 0.75;
            for y in 0..a.ny {
                for x in 0..a.nx {
                    let i = y * a.nx + x;
                    if x == 0 || x == a.nx - 1 || (x == 5 && y == 4) {
                        a.electrodes[i] = true;
                        a.potentials[i] = if x == 0 {
                            1.0
                        } else if x == a.nx - 1 {
                            500.0
                        } else {
                            -2.5
                        };
                    }
                }
            }
            let original = PaFile::new(a);
            write_pa(&input, &original).unwrap();
            let summary = crate::json::capture(
                crate::Cli::try_parse_from([
                    "simrep",
                    "--json",
                    "refine",
                    input.to_str().unwrap(),
                    "--device",
                    "cpu",
                    "--tolerance",
                    "1e-12",
                ])
                .unwrap(),
            )
            .unwrap()
            .unwrap();
            assert_eq!(
                summary["requested"],
                serde_json::json!({"device": "cpu", "backend": "vulkan", "precision": "f64"})
            );
            assert_eq!(
                summary["executed"],
                serde_json::json!({"device": "cpu", "backend": null, "precision": "f64"})
            );
            assert!(summary["fallback_reason"].is_null());
            assert_eq!(summary["converged"], true);
            assert_eq!(summary["tolerance"], 1e-12);
            assert!(summary["compute_ms"].as_f64().unwrap() >= 0.0);
            assert!(summary["prep_ms"].as_f64().unwrap() >= 0.0);
            assert_eq!(
                summary["output_paths"],
                serde_json::json!([
                    input.with_extension("pa_"),
                    input.with_extension("pa1"),
                    input.with_extension("pa500"),
                    input.with_extension("pa0"),
                ])
            );
            let layout = Layout::new(&original.array);
            let mut expected_cycles = 0;
            let mut expected_residual = 0.0_f64;
            for slot in [0, 1, 500] {
                let mut basis = original.clone();
                initialize_basis(&mut basis, &original, slot, -2.5);
                let (_, cycles, residual) = layout
                    .refine(
                        &mut basis.array,
                        Options {
                            relative_tolerance: 1e-12,
                            ..Options::default()
                        },
                        None,
                        None,
                        &mut Timings::default(),
                    )
                    .unwrap();
                expected_cycles += cycles;
                expected_residual = expected_residual.max(residual);
            }
            assert_eq!(summary["cycles"], expected_cycles);
            assert_eq!(summary["final_residual"], expected_residual);
            assert_eq!(read_pa(&input).unwrap(), original);
            let base = read_pa(input.with_extension("pa0")).unwrap();
            let first = read_pa(input.with_extension("pa1")).unwrap();
            let second = read_pa(input.with_extension("pa500")).unwrap();
            let scalable = read_pa(input.with_extension("pa_")).unwrap();
            let metadata = base.pa0.as_ref().unwrap();
            assert_eq!(metadata.reference_voltage, REFERENCE_VOLTAGE);
            assert_eq!(metadata.electrode_indices.len(), 501);
            assert_eq!(metadata.electrode_indices[0], 49);
            assert_eq!(metadata.electrode_indices[1], 0);
            assert_eq!(metadata.electrode_indices[500], 10);
            assert_eq!(scalable.array.potentials[49], REFERENCE_VOLTAGE);
            assert_eq!(base.array.potentials[49], -2.5);
            assert_eq!(first.array.electrodes, original.array.electrodes);
            assert_eq!(first.array.symmetry, symmetry);
            assert_eq!((first.array.dx_mm, first.array.dy_mm), (0.25, 0.75));
            assert_eq!(first.array.potentials[0], REFERENCE_VOLTAGE);
            assert_eq!(second.array.potentials[10], REFERENCE_VOLTAGE);
            let mut full = original.clone();
            for (value, &fixed) in full.array.potentials.iter_mut().zip(&full.array.electrodes) {
                if fixed {
                    *value = if *value == 1.0 {
                        7.0
                    } else if *value == 500.0 {
                        -3.0
                    } else {
                        *value
                    };
                }
            }
            let plain = input.with_extension("pa");
            write_pa(&plain, &full).unwrap();
            run(options(plain.clone())).unwrap();
            let full = read_pa(&plain).unwrap();
            for i in 0..full.array.potentials.len() {
                let sum = base.array.potentials[i]
                    + (7.0 * first.array.potentials[i] - 3.0 * second.array.potentials[i])
                        / REFERENCE_VOLTAGE;
                assert!(
                    (sum - full.array.potentials[i]).abs() < 2e-9,
                    "{symmetry:?} node {i}: {sum} != {}",
                    full.array.potentials[i]
                );
            }
            std::fs::remove_dir_all(dir).unwrap();
        }
    }

    #[test]
    fn anisotropic_3d_antimirror_plane_is_zero_without_becoming_an_electrode() {
        let path = std::env::temp_dir().join(format!(
            "simrep-refine-antimirror-{}.pa",
            std::process::id()
        ));
        let mut a = PotentialArray::new(9, 5, 4);
        a.mirrors.x = true;
        a.anti_mirrors.x = true;
        a.dx_mm = 0.125;
        a.dy_mm = 0.25;
        a.dz_mm = 0.75;
        for i in 0..a.potentials.len() {
            if i % a.nx == a.nx - 1 {
                a.electrodes[i] = true;
                a.potentials[i] = 3.5;
            }
        }
        let original = PaFile::new(a);
        write_pa(&path, &original).unwrap();
        run(options(path.clone())).unwrap();
        let refined = read_pa(&path).unwrap();
        assert_eq!(refined.array.electrodes, original.array.electrodes);
        assert_eq!(refined.array.anti_mirrors, original.array.anti_mirrors);
        for (i, value) in refined.array.potentials.iter().enumerate() {
            let exact = 3.5 * (i % refined.array.nx) as f64 / 8.0;
            assert!((value - exact).abs() < 1e-9, "node {i}: {value} != {exact}");
        }
        let mut bad = original;
        bad.array.electrodes[0] = true;
        bad.array.potentials[0] = 1.0;
        write_pa(&path, &bad).unwrap();
        assert!(run(options(path.clone())).is_err());
        assert_eq!(read_pa(&path).unwrap(), bad);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn cylindrical_radial_y_axis_and_spacing_match_harmonic_polynomial() {
        let path = std::env::temp_dir().join(format!(
            "simrep-refine-cylindrical-{}.pa",
            std::process::id()
        ));
        let mut a = PotentialArray::new(9, 7, 1);
        a.symmetry = Symmetry::Cylindrical;
        a.mirrors.y = true;
        a.dx_mm = 0.25;
        a.dy_mm = 0.5;
        let exact = |x: usize, y: usize| (y as f64 * 0.5).powi(2) - 2.0 * (x as f64 * 0.25).powi(2);
        for y in 0..a.ny {
            for x in 0..a.nx {
                if x == 0 || x == a.nx - 1 || y == a.ny - 1 {
                    let i = y * a.nx + x;
                    a.electrodes[i] = true;
                    a.potentials[i] = exact(x, y);
                }
            }
        }
        write_pa(&path, &PaFile::new(a)).unwrap();
        run(options(path.clone())).unwrap();
        let result = read_pa(&path).unwrap();
        for (i, value) in result.array.potentials.iter().enumerate() {
            assert!((value - exact(i % 9, i / 9)).abs() < 1e-9);
        }
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn cli_validates_tolerance_and_selects_refine_device() {
        let cli = crate::Cli::try_parse_from([
            "simrep",
            "refine",
            "lens.pa#",
            "--device",
            "gpu",
            "--backend",
            "vulkan",
            "--tolerance",
            "1e-12",
        ])
        .unwrap();
        let crate::Command::Refine(args) = cli.command else {
            panic!("wrong command");
        };
        assert_eq!(args.device, RefineDevice::Gpu);
        assert_eq!(args.tolerance, 1e-12);
        for tolerance in ["NaN", "0", "-1", "inf"] {
            assert!(
                crate::Cli::try_parse_from([
                    "simrep",
                    "refine",
                    "lens.pa",
                    "--tolerance",
                    tolerance,
                ])
                .is_err()
            );
        }
    }

    #[test]
    fn automatic_dielectric_device_uses_the_measured_compute_crossover() {
        let grid = |dims| Grid {
            dims,
            spacing: [1.0; 3],
            geometry: Geometry::Planar,
        };
        for dims in [[101, 101, 1], [101, 201, 1], [65, 65, 65], [101, 51, 51]] {
            assert!(!RefineDevice::Auto.requests_gpu(grid(dims), Some(false)));
        }
        for dims in [[201, 101, 101], [401, 201, 201]] {
            assert!(RefineDevice::Auto.requests_gpu(grid(dims), Some(false)));
        }
        assert!(!RefineDevice::Auto.requests_gpu(grid([51, 26, 26]), Some(true)));
        for dims in [
            [101, 51, 51],
            [201, 101, 101],
            [251, 126, 126],
            [401, 201, 201],
        ] {
            assert!(RefineDevice::Auto.requests_gpu(grid(dims), Some(true)));
            assert!(!RefineDevice::Auto.requests_gpu(grid(dims), None));
            for material in [None, Some(false), Some(true)] {
                assert!(RefineDevice::Gpu.requests_gpu(grid(dims), material));
                assert!(!RefineDevice::Cpu.requests_gpu(grid(dims), material));
            }
        }
    }

    #[test]
    fn nonconvergence_does_not_overwrite_plain_input() {
        let path = std::env::temp_dir().join(format!(
            "simrep-refine-unconverged-{}.pa",
            std::process::id()
        ));
        let mut a = PotentialArray::new(5, 5, 1);
        a.electrodes[0] = true;
        a.potentials[0] = 1.0;
        let original = PaFile::new(a);
        write_pa(&path, &original).unwrap();
        let mut args = options(path.clone());
        args.max_cycles = 0;
        assert!(run(args).is_err());
        assert_eq!(read_pa(&path).unwrap(), original);
        std::fs::remove_file(path).unwrap();
    }
}

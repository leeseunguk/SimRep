use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand, ValueEnum};
use simrep_core::{FieldType, Symmetry};
use simrep_io::read_pa;
use simrep_solve::gpu::{Backends, GRID_SIZE, Gpu, GpuResult};
use simrep_solve::mg::{Geometry, Grid, Options, solve};

// Preserve the human CLI while keeping stdout exclusively machine-readable.
macro_rules! println {
    ($($arg:tt)*) => {
        if crate::json::enabled() {
            ::std::eprintln!($($arg)*);
        } else {
            ::std::println!($($arg)*);
        }
    };
}

mod cad_measure;
mod fly_bench;
mod fly_cmd;
mod json;
mod mesh_cmd;
mod physics_cmd;
mod refine_cmd;
mod refine_geometry;
mod refine_sdf;
#[cfg(test)]
mod repeatability;

#[derive(Parser)]
#[command(name = "simrep", version, about = "Native ion optics simulator")]
struct Cli {
    /// Print one JSON summary to stdout; send human output and CSV records to stderr.
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Report the selected adapter and native compute features.
    GpuInfo(GpuOptions),
    /// Validate a 64^3 seven-point residual against CPU f64.
    GpuSmoke(GpuOptions),
    /// Print the header and potential statistics of a SIMION PA file.
    PaInfo {
        /// SIMION PA, PA#, PA0 or numbered basis file.
        file: PathBuf,
    },
    /// Compile SIMION GEM geometry (including GEM Lua preprocessing) to a potential array.
    Gem2pa {
        /// Source GEM geometry file.
        input: PathBuf,
        /// Destination PA# (defaults to the input stem with .pa#).
        output: Option<PathBuf>,
    },
    /// Fly CPU f64 reference ions through a synthetic einzel-like grid.
    FlyBench(fly_bench::FlyBenchOptions),
    /// Fly particles through electric/magnetic fields and report terminal events.
    Fly(Box<fly_cmd::FlyOptions>),
    /// Benchmark multigrid on a 3D harmonic boundary case.
    RefineBench(RefineBenchOptions),
    /// Refine a PA or PA# and write potentials or fast-adjust basis files.
    Refine(refine_cmd::RefineOptions),
    /// Refine analytic continuous electrodes with symmetric ghost-fluid boundaries.
    RefineSdf(refine_sdf::SdfOptions),
    /// Refine GEM or mesh electrodes at their continuous surfaces.
    RefineGeometry(Box<refine_geometry::GeometryOptions>),
    /// Measure complete CAD face sets with an independent tessellation cross-check.
    CadMeasure(cad_measure::MeasureOptions),
    /// Import a 3D mesh as grouped electrodes in a SIMION PA#.
    MeshImport(Box<mesh_cmd::MeshImportOptions>),
    /// Export electrode surfaces, fields, isopotentials or trajectory samples.
    #[command(subcommand)]
    Export(mesh_cmd::ExportCommand),
}

#[derive(Args)]
struct GpuOptions {
    /// Native compute backend used to select an adapter.
    #[arg(long, value_enum, default_value = "auto")]
    backend: Backend,
}

#[derive(Args)]
struct RefineBenchOptions {
    /// Compute device for the multigrid solve.
    #[arg(long, value_enum, default_value = "cpu")]
    device: Device,
    /// Native GPU backend (unused on CPU).
    #[arg(long, value_enum, default_value = "vulkan")]
    backend: Backend,
    /// Nodes along each axis; integer at least 3.
    #[arg(long, default_value_t = 128, value_parser = parse_grid_size)]
    n: usize,
    /// Positive finite relative residual tolerance.
    #[arg(long, default_value = "1e-10", value_parser = parse_tolerance)]
    tolerance: f64,
    /// Maximum number of multigrid cycles.
    #[arg(long, default_value_t = 100)]
    max_cycles: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, ValueEnum)]
enum Device {
    Cpu,
    Gpu,
}

fn parse_grid_size(value: &str) -> Result<usize, String> {
    let n = value.parse::<usize>().map_err(|_| "n must be an integer")?;
    if n < 3 {
        return Err("n must be at least 3".into());
    }
    Ok(n)
}

fn parse_tolerance(value: &str) -> Result<f64, String> {
    let tolerance = value
        .parse::<f64>()
        .map_err(|_| "tolerance must be a number")?;
    if !tolerance.is_finite() || tolerance <= 0.0 {
        return Err("tolerance must be positive and finite".into());
    }
    Ok(tolerance)
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum Backend {
    /// Prefer a high-performance adapter among DX12 and Vulkan devices.
    Auto,
    Dx12,
    Vulkan,
}

impl Backend {
    fn backends(self) -> Backends {
        match self {
            Self::Auto => Backends::DX12 | Backends::VULKAN,
            Self::Dx12 => Backends::DX12,
            Self::Vulkan => Backends::VULKAN,
        }
    }
}

fn require_hardware_gpu(info: &simrep_solve::gpu::AdapterInfo) -> GpuResult<()> {
    if info.device_type == simrep_solve::gpu::DeviceType::Cpu {
        return Err(format!(
            "--device gpu requires a hardware GPU; adapter {} ({:?}) is a CPU/software adapter; use --device cpu",
            info.name, info.backend
        )
        .into());
    }
    Ok(())
}

fn report_gpu_residual(native_f64: bool) {
    if !native_f64 {
        eprintln!("GPU residual: CPU f64 (adapter lacks native f64; GPU f32 corrections retained)");
    }
}

fn main() -> ExitCode {
    // Rayon defaults to available_parallelism() unless RAYON_NUM_THREADS is set.
    // Build the pool here so a later library cannot install a smaller one first.
    let _ = rayon::ThreadPoolBuilder::new().build_global();
    let arguments: Vec<_> = std::env::args_os().collect();
    let json_requested = arguments.iter().any(|argument| argument == "--json");
    let cli = match Cli::try_parse_from(arguments) {
        Ok(cli) => cli,
        Err(error) => {
            if !error.use_stderr() {
                let _ = error.print();
                return ExitCode::SUCCESS;
            }
            let _ = error.print();
            if json_requested {
                ::std::println!("{}", json::error(&error.to_string(), "invalid_arguments"));
                return ExitCode::FAILURE;
            }
            return ExitCode::from(error.exit_code() as u8);
        }
    };
    match json::capture(cli) {
        Ok(summary) => {
            if let Some(summary) = summary {
                ::std::println!("{summary}");
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("error: {error}");
            if json_requested {
                let kind = if error.is::<std::io::Error>() {
                    "io"
                } else {
                    "command_failed"
                };
                ::std::println!("{}", json::execution_error(error.as_ref(), kind));
            }
            ExitCode::FAILURE
        }
    }
}

fn pa_info(file: &Path) -> GpuResult<()> {
    let pa = read_pa(file).map_err(|error| format!("{}: {error}", file.display()))?;
    let a = &pa.array;
    let electrodes = a.electrodes.iter().filter(|&&e| e).count();
    let min = a.potentials.iter().copied().fold(f64::INFINITY, f64::min);
    let max = a
        .potentials
        .iter()
        .copied()
        .fold(f64::NEG_INFINITY, f64::max);
    println!("file: {}", file.display());
    println!(
        "mode: {} ({}-byte header)",
        pa.mode.value(),
        pa.mode.header_bytes()
    );
    println!("dimensions: {} x {} x {}", a.nx, a.ny, a.nz);
    println!(
        "symmetry: {}",
        match a.symmetry {
            Symmetry::Planar => "planar",
            Symmetry::Cylindrical => "cylindrical",
        }
    );
    println!(
        "mirrors: x={} y={} z={}",
        a.mirrors.x, a.mirrors.y, a.mirrors.z
    );
    println!(
        "anti-mirrors: x={} y={} z={}",
        a.anti_mirrors.x, a.anti_mirrors.y, a.anti_mirrors.z
    );
    println!(
        "field: {}",
        match a.field_type {
            FieldType::Electrostatic => "electrostatic",
            FieldType::Magnetic => "magnetic",
        }
    );
    println!("spacing_mm: {} x {} x {}", a.dx_mm, a.dy_mm, a.dz_mm);
    println!("refinable: {}", a.refinable);
    println!("max_voltage: {}", pa.max_voltage);
    println!("ng: {}", pa.ng);
    println!("points: {}", a.potentials.len());
    println!("electrode_points: {electrodes}");
    println!("non_electrode_points: {}", a.potentials.len() - electrodes);
    println!("potential_min: {min}");
    println!("potential_max: {max}");
    if let Some(metadata) = &pa.pa0 {
        let count = metadata
            .electrode_indices
            .iter()
            .filter(|&&i| i != -1)
            .count();
        println!("adjustable_electrodes: {count}");
        println!("reference_voltage: {}", metadata.reference_voltage);
    }
    // SIMION 2024 was observed rejecting cylindrical headers when dy != dz;
    // oracle context: tools/simion-oracle/README.md.
    // Other spacing/dimension values are already validated by simrep-io's PA reader.
    let mut simion_issues = Vec::new();
    if a.symmetry == Symmetry::Cylindrical && a.dy_mm != a.dz_mm {
        simion_issues.push("cylindrical symmetry requires dy_mm == dz_mm in SIMION 2024");
    }
    json::set_summary(serde_json::json!({
        "file": file,
        "simion_compatible": simion_issues.is_empty(),
        "simion_issues": simion_issues,
        "mode": pa.mode.value(),
        "header_bytes": pa.mode.header_bytes(),
        "dims": [a.nx, a.ny, a.nz],
        "symmetry": match a.symmetry { Symmetry::Planar => "planar", Symmetry::Cylindrical => "cylindrical" },
        "mirrors": [a.mirrors.x, a.mirrors.y, a.mirrors.z],
        "anti_mirrors": [a.anti_mirrors.x, a.anti_mirrors.y, a.anti_mirrors.z],
        "field_type": match a.field_type { FieldType::Electrostatic => "electrostatic", FieldType::Magnetic => "magnetic" },
        "spacing_mm": [a.dx_mm, a.dy_mm, a.dz_mm],
        "refinable": a.refinable,
        "max_voltage": pa.max_voltage,
        "ng": pa.ng,
        "points": a.potentials.len(),
        "electrode_points": electrodes,
        "non_electrode_points": a.potentials.len() - electrodes,
        "potential_min": min,
        "potential_max": max,
        "potential_mean": a.potentials.iter().sum::<f64>() / a.potentials.len() as f64,
        "adjustable_electrodes": pa.pa0.as_ref().map(|m| m.electrode_indices.iter().filter(|&&i| i != -1).count()),
        "reference_voltage": pa.pa0.as_ref().map(|m| m.reference_voltage),
    }));
    Ok(())
}

fn run(cli: Cli) -> GpuResult<()> {
    let (options, smoke) = match cli.command {
        Command::GpuInfo(options) => (options, false),
        Command::GpuSmoke(options) => (options, true),
        Command::PaInfo { file } => return pa_info(&file),
        Command::Gem2pa { input, output } => {
            let gem = simrep_gem::read_gem(&input)?;
            let result = gem.rasterize()?;
            let output = output.unwrap_or_else(|| input.with_extension("pa#"));
            simrep_io::write_pa(&output, &result.pa)?;
            result.write_surfaces(&output)?;
            let mut paths = vec![output.clone()];
            if result
                .surfaces
                .as_ref()
                .is_some_and(|s| s.axes.iter().any(|a| !a.is_empty()))
            {
                paths.push(output.with_extension("pa-surf"));
            }
            let electrode_numbers: std::collections::BTreeSet<_> = result
                .pa
                .array
                .potentials
                .iter()
                .zip(&result.pa.array.electrodes)
                .filter_map(|(&v, &e)| {
                    (e && (1.0..=1000.0).contains(&v) && v.fract() == 0.0).then_some(v as u32)
                })
                .collect();
            json::set_summary(serde_json::json!({
                "output_paths": paths,
                "dims": gem.dimensions,
                "spacing_mm": gem.spacing_mm,
                "electrode_numbers": electrode_numbers,
                "compute_ms": result.compute_time.as_secs_f64() * 1000.0,
            }));
            println!("output: {}", output.display());
            println!(
                "dimensions: {} x {} x {}",
                gem.dimensions[0], gem.dimensions[1], gem.dimensions[2]
            );
            println!(
                "compute_ms: {:.6}",
                result.compute_time.as_secs_f64() * 1000.0
            );
            return Ok(());
        }
        Command::RefineBench(options) => return run_refine_bench(options),
        Command::Refine(options) => return refine_cmd::run(options),
        Command::RefineSdf(options) => return refine_sdf::run(options),
        Command::RefineGeometry(options) => return refine_geometry::run(*options),
        Command::CadMeasure(options) => return cad_measure::run(options),
        Command::FlyBench(options) => return fly_bench::run(options),
        Command::Fly(options) => return fly_cmd::run(*options),
        Command::MeshImport(options) => return mesh_cmd::import(*options),
        Command::Export(command) => return mesh_cmd::export(command),
    };
    let backends = options.backend.backends();
    let gpu = pollster::block_on(Gpu::new(backends))?.ok_or_else(|| {
        format!(
            "no adapter found for {backends:?}; WSL without Vulkan cannot run GPU checks. \
             Use Windows cargo.exe (see docs/dev/windows-gpu.md)."
        )
    })?;
    println!("Adapter: {}", gpu.info.name);
    println!("Backend: {:?}", gpu.info.backend);
    println!("Device type: {:?}", gpu.info.device_type);
    println!("Driver: {} ({})", gpu.info.driver, gpu.info.driver_info);
    println!("Adapter features: {:?}", gpu.adapter_features);
    println!("SHADER_F64: {}", gpu.has_f64());
    let mut summary = serde_json::json!({
        "adapter_name": gpu.info.name,
        "adapter_type": format!("{:?}", gpu.info.device_type),
        "backend": format!("{:?}", gpu.info.backend).to_lowercase(),
        "f64_support": gpu.has_f64(),
        "software_adapter": gpu.info.device_type == simrep_solve::gpu::DeviceType::Cpu,
        "driver": gpu.info.driver,
        "driver_info": gpu.info.driver_info,
    });
    if smoke {
        let mut checks = Vec::new();
        println!("Grid: {GRID_SIZE}^3, unit-spacing residual b - Laplacian(x)");
        for result in gpu.smoke()? {
            checks.push(serde_json::json!({
                "precision": result.precision,
                "kernel_ms": result.kernel_time.map(|t| t.as_secs_f64() * 1000.0),
                "submit_readback_ms": result.submit_readback_time.as_secs_f64() * 1000.0,
                "max_abs_error": result.max_abs_error,
                "tolerance": result.tolerance,
                "cells_checked": result.cells_checked,
            }));
            let kernel_time = result
                .kernel_time
                .map(|time| format!("{:.3} ms", time.as_secs_f64() * 1_000.0))
                .unwrap_or_else(|| "unavailable (TIMESTAMP_QUERY not supported)".into());
            println!(
                "{} PASS: GPU kernel = {}, submit + readback = {:.3} ms, \
                 max abs error vs CPU f64 = {:.9e} (tolerance {:.1e}, {} cells)",
                result.precision,
                kernel_time,
                result.submit_readback_time.as_secs_f64() * 1_000.0,
                result.max_abs_error,
                result.tolerance,
                result.cells_checked,
            );
        }
        if !gpu.has_f64() {
            println!("f64 SKIP: SHADER_F64 unavailable; residual correction must use CPU f64.");
        }
        summary["checks"] = serde_json::json!(checks);
    }
    json::set_summary(summary);
    Ok(())
}

fn run_refine_bench(options: RefineBenchOptions) -> GpuResult<()> {
    let n = options.n;
    let count = n
        .checked_mul(n)
        .and_then(|n2| n2.checked_mul(n))
        .ok_or("grid size overflows usize")?;
    let spacing = 1.0 / (n - 1) as f64;
    let pi = std::f64::consts::PI;
    let alpha = 2.0_f64.sqrt() * pi;
    // Precompute separated factors so setup/error reporting do not dominate
    // small benchmarks with millions of transcendental evaluations.
    let sin: Vec<_> = (0..n).map(|i| (pi * i as f64 * spacing).sin()).collect();
    let sinh: Vec<_> = (0..n)
        .map(|i| (alpha * i as f64 * spacing).sinh() / alpha.sinh())
        .collect();
    let exact = |x: usize, y: usize, z: usize| sin[x] * sin[y] * sinh[z];
    let mut values = vec![0.0; count];
    let mut fixed = vec![false; count];
    for z in 0..n {
        for y in 0..n {
            for x in 0..n {
                if x == 0 || y == 0 || z == 0 || x == n - 1 || y == n - 1 || z == n - 1 {
                    let i = x + n * (y + n * z);
                    fixed[i] = true;
                    values[i] = exact(x, y, z);
                }
            }
        }
    }
    let grid = Grid {
        dims: [n; 3],
        spacing: [spacing; 3],
        geometry: Geometry::Planar,
    };
    let solver_options = Options {
        relative_tolerance: options.tolerance,
        max_cycles: options.max_cycles,
        ..Options::default()
    };
    let (converged, iterations, levels, history, elapsed) = match options.device {
        Device::Cpu => {
            let report = solve(&mut values, &fixed, grid, solver_options)?;
            println!("CPU solver workers: {}", report.worker_threads);
            println!(
                "Setup: {:.3} ms, solve: {:.3} ms",
                report.timings.setup.as_secs_f64() * 1_000.0,
                report.timings.solve.as_secs_f64() * 1_000.0,
            );
            println!(
                "Smoothing: {:.3} ms, restriction: {:.3} ms, prolongation: {:.3} ms, residual: {:.3} ms",
                report.timings.smoothing.as_secs_f64() * 1_000.0,
                report.timings.restriction.as_secs_f64() * 1_000.0,
                report.timings.prolongation.as_secs_f64() * 1_000.0,
                report.timings.residual.as_secs_f64() * 1_000.0,
            );
            (
                report.converged,
                report.iterations,
                report.levels,
                report.residual_history,
                report.elapsed,
            )
        }
        Device::Gpu => {
            let start = std::time::Instant::now();
            let gpu = pollster::block_on(Gpu::new(options.backend.backends()))?
                .ok_or("no GPU adapter found")?;
            require_hardware_gpu(&gpu.info)?;
            println!(
                "Adapter: {} ({:?}, {:?})",
                gpu.info.name, gpu.info.backend, gpu.info.device_type
            );
            println!(
                "Device initialization: {:.3} ms",
                start.elapsed().as_secs_f64() * 1000.0
            );
            let report = gpu.solve(&mut values, &fixed, grid, solver_options)?;
            report.check_converged()?;
            println!(
                "Residual path: {}",
                if report.gpu_f64 {
                    "GPU f64"
                } else {
                    "CPU f64 (adapter lacks native f64; full correction download + residual upload each cycle)"
                }
            );
            println!(
                "Setup: {:.3} ms, upload: {:.3} ms, solve: {:.3} ms, readback: {:.3} ms",
                report.timings.setup.as_secs_f64() * 1000.0,
                report.timings.upload.as_secs_f64() * 1000.0,
                report.timings.solve.as_secs_f64() * 1000.0,
                report.timings.readback.as_secs_f64() * 1000.0
            );
            (
                report.converged,
                report.iterations,
                report.levels,
                report.residual_history,
                report.elapsed,
            )
        }
    };
    let threads = std::env::var("RAYON_NUM_THREADS").ok().unwrap_or_else(|| {
        std::thread::available_parallelism()
            .map_or(1, usize::from)
            .to_string()
    });
    let mut error = 0.0_f64;
    for z in 0..n {
        for y in 0..n {
            for x in 0..n {
                error = error.max((values[x + n * (y + n * z)] - exact(x, y, z)).abs());
            }
        }
    }
    let first = history.first().unwrap();
    let last = history.last().unwrap();
    println!("Grid: {n}^3 ({count} nodes), Rayon threads: {threads}");
    let stages = if options.device == Device::Cpu {
        "setup + solve"
    } else {
        "setup + upload + solve + readback"
    };
    println!(
        "Elapsed ({stages}): {:.3} ms",
        elapsed.as_secs_f64() * 1_000.0
    );
    println!("Cycles: {iterations}, levels: {levels}");
    println!(
        "Relative residual (f64): initial = {:.9e}, final = {:.9e}",
        first.relative, last.relative
    );
    println!("Max analytic discretization error: {error:.9e}");
    json::set_summary(serde_json::json!({
        "device": format!("{:?}", options.device).to_lowercase(),
        "dims": [n, n, n],
        "cycles": iterations,
        "levels": levels,
        "converged": converged,
        "final_residual": last.relative,
        "tolerance": options.tolerance,
        "elapsed_ms": elapsed.as_secs_f64() * 1000.0,
        "max_analytic_error": error,
    }));
    if !converged {
        return Err(format!("refinement did not converge after {iterations} cycles").into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smoke_defaults_to_both_native_backends() {
        let cli = Cli::try_parse_from(["simrep", "gpu-smoke"]).unwrap();
        let Command::GpuSmoke(options) = cli.command else {
            panic!("wrong subcommand");
        };
        assert_eq!(
            options.backend.backends(),
            Backends::DX12 | Backends::VULKAN
        );
    }

    #[test]
    fn info_can_select_each_backend() {
        for (name, expected) in [("dx12", Backends::DX12), ("vulkan", Backends::VULKAN)] {
            let cli = Cli::try_parse_from(["simrep", "gpu-info", "--backend", name]).unwrap();
            let Command::GpuInfo(options) = cli.command else {
                panic!("wrong subcommand");
            };
            assert_eq!(options.backend.backends(), expected);
        }
    }

    #[test]
    fn refine_bench_parses_options_and_rejects_invalid_values() {
        let cli = Cli::try_parse_from(["simrep", "refine-bench", "--n", "256"]).unwrap();
        let Command::RefineBench(options) = cli.command else {
            panic!("wrong subcommand");
        };
        assert_eq!(options.n, 256);
        assert_eq!(options.tolerance, 1e-10);
        assert_eq!(options.max_cycles, 100);
        assert_eq!(options.device, Device::Cpu);
        let cli = Cli::try_parse_from([
            "simrep",
            "refine-bench",
            "--device",
            "gpu",
            "--backend",
            "vulkan",
        ])
        .unwrap();
        let Command::RefineBench(options) = cli.command else {
            panic!("wrong subcommand");
        };
        assert_eq!(options.device, Device::Gpu);
        assert_eq!(options.backend.backends(), Backends::VULKAN);
        for n in ["0", "1", "2"] {
            assert!(Cli::try_parse_from(["simrep", "refine-bench", "--n", n]).is_err());
        }
        for tolerance in ["0", "NaN", "inf"] {
            assert!(
                Cli::try_parse_from(["simrep", "refine-bench", "--tolerance", tolerance]).is_err()
            );
        }
    }

    #[test]
    fn refine_bench_runs_the_cpu_solver() {
        run_refine_bench(RefineBenchOptions {
            device: Device::Cpu,
            backend: Backend::Vulkan,
            n: 8,
            tolerance: 1e-8,
            max_cycles: 100,
        })
        .unwrap();
        assert!(
            run_refine_bench(RefineBenchOptions {
                device: Device::Cpu,
                backend: Backend::Vulkan,
                n: 8,
                tolerance: 1e-12,
                max_cycles: 0
            })
            .is_err()
        );
    }
}

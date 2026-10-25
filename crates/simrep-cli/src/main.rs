use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand, ValueEnum};
use simrep_core::{FieldType, Symmetry};
use simrep_io::read_pa;
use simrep_solve::gpu::{Backends, GRID_SIZE, Gpu, GpuResult};
use simrep_solve::mg::{Geometry, Grid, Options, solve};

mod fly_bench;
mod fly_cmd;
mod mesh_cmd;
mod physics_cmd;
mod refine_cmd;
mod refine_geometry;
mod refine_sdf;

#[derive(Parser)]
#[command(name = "simrep", version, about = "Native ion optics simulator")]
struct Cli {
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
        file: PathBuf,
    },
    /// Compile SIMION GEM geometry (including GEM Lua preprocessing) to a potential array.
    Gem2pa {
        input: PathBuf,
        output: Option<PathBuf>,
    },
    /// Fly CPU f64 reference ions through a synthetic einzel-like grid.
    FlyBench(fly_bench::FlyBenchOptions),
    Fly(Box<fly_cmd::FlyOptions>),
    /// Benchmark multigrid on a 3D harmonic boundary case.
    RefineBench(RefineBenchOptions),
    Refine(refine_cmd::RefineOptions),
    /// Refine analytic continuous electrodes with symmetric ghost-fluid boundaries.
    RefineSdf(refine_sdf::SdfOptions),
    /// Refine GEM or mesh electrodes at their continuous surfaces.
    RefineGeometry(Box<refine_geometry::GeometryOptions>),
    /// Import a 3D mesh as grouped electrodes in a SIMION PA#.
    MeshImport(Box<mesh_cmd::MeshImportOptions>),
    /// Export electrode surfaces, fields, isopotentials or trajectory samples.
    #[command(subcommand)]
    Export(mesh_cmd::ExportCommand),
}

#[derive(Args)]
struct GpuOptions {
    #[arg(long, value_enum, default_value = "auto")]
    backend: Backend,
}

#[derive(Args)]
struct RefineBenchOptions {
    #[arg(long, value_enum, default_value = "cpu")]
    device: Device,
    #[arg(long, value_enum, default_value = "vulkan")]
    backend: Backend,
    #[arg(long, default_value_t = 128, value_parser = parse_grid_size)]
    n: usize,
    #[arg(long, default_value = "1e-10", value_parser = parse_tolerance)]
    tolerance: f64,
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

#[derive(Clone, Copy, ValueEnum)]
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

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
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
    if smoke {
        println!("Grid: {GRID_SIZE}^3, unit-spacing residual b - Laplacian(x)");
        for result in gpu.smoke()? {
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
    }
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
            println!(
                "Adapter: {} ({:?}, {:?})",
                gpu.info.name, gpu.info.backend, gpu.info.device_type
            );
            println!(
                "Device initialization: {:.3} ms",
                start.elapsed().as_secs_f64() * 1000.0
            );
            let report = gpu.solve(&mut values, &fixed, grid, solver_options)?;
            println!(
                "Residual path: {}",
                if report.gpu_f64 {
                    "GPU f64"
                } else {
                    "CPU f64 (full correction download + residual upload each cycle)"
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

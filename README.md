English | [한국어](README.ko.md)

# SimRep — a native ion optics simulator faster than SIMION

SimRep calculates the electric fields produced by electrodes and simulates ion trajectories through them.
It implements the core workflow of SIMION-2024, rebuilt for **faster computation at matched accuracy** and **more accurate electrode surfaces**.

- Rust computation core, multicore CPU execution, and wgpu GPU support (Vulkan/DX12).
- Opens existing SIMION files: PA, GEM, FLY2, ION, and workbenches.
- Available as a desktop app, the `simrep` CLI, and the `simrep` Python package.

![Einzel lens template flown in the SimRep app](docs/report/figures/F01.png)

> Details and validation evidence are in the [technical report](docs/report/build/simrep-technical-report.pdf) (Korean, 39 pages). Historical benchmarks come from that report; later reconstruction comparisons are identified separately below.

---

## What is different?

SIMION represents electrodes as grid-sized **staircases**, solves potentials with SOR iterations, and reads electric fields by linear interpolation.
SimRep takes a different approach.

| | SIMION | SimRep |
|---|---|---|
| Electrode boundary | Grid-sized staircase | **Cut-cell** boundaries at the actual surface position |
| Potential solver | SOR; convergence by iterate change | **Multigrid**; convergence by equation residual |
| Electric field | Linear interpolation, discontinuous at cell boundaries | **Smooth** 5th-order reconstruction across cell boundaries |
| Electrode collision | Staircase | **Exact intersections** with source geometry |
| Compute device | CPU | Multicore CPU + GPU; `--device auto` uses measured thresholds (magnetic flight currently always selects CPU) |

---

## How fast is it?

Comparisons follow two rules:

1. Compare **at matched accuracy**, measured against a fine-grid reference solution. SIMION is evaluated against the same reference.
2. Compare **compute time only**. File loading and GPU initialization are excluded and reported separately for context.

![Compute-time speedup over SIMION at matched accuracy](docs/report/figures/F02.png)

| Workload | Accuracy | Speedup over SIMION |
|---|---|---|
| 1,737 ten-thousand nodes, 3D potential solve (GPU) | Maximum difference from tightly converged SIMION: 6.42e-5 V | **19.3x** (9.1x including preparation) |
| Einzel lens ion flight | 4.9x smaller error | **11.5x** over default-quality SIMION |
| 3D quadrupole, exact-surface path (historical reconstruction) | About 700x smaller endpoint error | **71x** |
| Neutral-gas collisions, HS1 / SDS | Distribution tests passed | **54x / 251x** |
| 7 magnetic examples | Existing accuracy criteria passed | 1.4–48.7x |

The quadrupole row preserves the `pre-2026-10-28 reconstruction` measurement, whose RMS was **0.0034 mm**.
Current code gives **0.00386 mm** against the retained `8x` reference and **0.00388 mm** against the regenerated `8x` reference.
The regenerated reference's `4x`/`8x` refinement difference is **0.00055 mm**; this does not independently establish a deterioration in physical error.
The [reconstruction validation record](docs/validation/CUT_GHOST_FILL.md) documents both the improved near-surface analyzer trajectories and this internal quadrupole metric's reference sensitivity.

SimRep was faster in all 24 matched-accuracy comparisons (1.4–251x).
Measurements used a Ryzen 9 5950X and RTX 3070. Timing scope varies slightly between comparisons; see chapter 8 and figure 22 of the report.

### Accuracy: actual surfaces instead of staircases

This cumulative distribution shows endpoint errors for 1,000 ions in a 3D quadrupole. Further left means more accurate.
Orange is SIMION; gray is SimRep with staircase collisions in the same potential field; blue uses actual surfaces for both potential boundaries and collisions.

![Cumulative endpoint error distribution in a 3D quadrupole](docs/report/figures/F08.png)

### Application: electrostatic energy analyzer (ESA) channels

An instrument STEP model was imported directly to solve the potentials in a pair of toroidal analyzer sections. Electrons were flown while sweeping plate voltages across 32 logarithmic channels to find each channel's transmitted energy. The axisymmetric geometry follows the CAD section, with grid spacing 0.125 mm and 4,096 electrons per point.

![ESA meridional potential and electron trajectories](docs/images/esa/esa_capture_meridian.png)

The axisymmetric section shows potential colors and contours, and 60 trajectories of 4.12 keV electrons at a plate voltage difference of 376.6 V. The 30 green trajectories reach the MCP detector; the 30 pink trajectories hit plates or supports. Computed and rendered by SimRep.

![ESA potential array and flight results in the desktop app](docs/images/esa/esa_capture_app.png)

The same potential array, voltage, and 60 electrons flown in the desktop app's precise mode (CPU double precision). Computed and rendered by SimRep.

![ESA 3D view in the desktop app](docs/images/esa/esa_capture_app_3d.png)

An oblique 3D view of the same flight, showing electrodes revolved from the axisymmetric section and the trajectories. Computed and rendered by SimRep.

![Transmitted-energy centroid and FWHM versus plate voltage difference](docs/images/esa/esa_centroid_fwhm.png)

- Transmitted energy is proportional to plate voltage difference. The fitted analyzer constant is k = 0.0923 V/eV; the geometric value from the plate gap is 0.100 V/eV.
- Channel resolution FWHM/E is 0.10–0.22. It varies with sample size and grid spacing and must be read with numerical uncertainty in mind.

![Energy resolution FWHM/E by channel](docs/images/esa/esa_de_over_e.png)

Representative channels were rerun after fixing near-surface electric-field reconstruction. Near-plate trajectory bias decreased, but k remained nearly unchanged at 0.0928 V/eV. The earlier channel conclusions were therefore not caused by that reconstruction defect.

![Analyzer constant before and after the reconstruction fix](docs/images/esa/esa_k_fix.png)

A design exploration also varied plate radii and slits. Widening the plate gap to about 3.15 mm gave k = 0.1243 V/eV, within 0.25% of the target 0.124 V/eV. The optimization did not converge, and source position, source size, and 3D structure remain unspecified: this is not a fabrication-ready design. Instrument geometry and raw data are not included in this repository.

#### Comparison with the same model in SIMION 2024

SIMION 2024 used the same staircase potential array (normalized grid spacing 0.125 mm), the same source of 4,096 electrons, relativistic motion, and the same detector plane. Both potential solves were tightly converged; at channel 26 voltage, the maximum whole-grid potential difference was 20.3 µV.

![Electron trajectories from the same potential array in SIMION 2024](docs/images/esa/esa_simion_capture.png)

SIMION 2024 screenshot: all 60 SIMION-detected electrons and 60 non-detected electrons were flown, captured with `simion.print_screen()`, and cropped to the plot. Green trajectories reach the detector plane; red trajectories hit plates or supports; brown denotes electrodes. Computed and rendered by SIMION.

![Channel 16 transmission: SIMION, SimRep staircase, and SimRep actual surfaces](docs/images/esa/esa_simion_transmission.png)

![Channel 26 detector positions: SIMION and SimRep staircase](docs/images/esa/esa_simion_hits.png)

- At channel 26, 4,092 of 4,096 electrons had the same outcome (detection or collision) as SIMION. The remaining 4 differed: 3 detected only by SIMION and 1 detected only by SimRep.
- For the 57 common detections, the position difference was RMS 0.13 mm.
- At channels 1, 16, and 32, transmitted-energy centroids agreed within 0.3–0.5% between SIMION and SimRep staircase. Staircase FWHM was 7.6–8.2% narrower than SIMION.
- SimRep actual-surface FWHM was 19–35% narrower than SIMION.

The main reason for narrower actual-surface results is that insulating supports also stop electrons. The staircase potential array has no insulator nodes, so array-based SIMION and SimRep staircase let electrons pass through those supports. Those staircase results agree with each other. Disabling support collisions in actual-surface flight brings FWHM and detection counts back toward the staircase results. Below are FWHM and detection counts at grid spacing 0.0625 mm, with 495,616 electrons per channel.

| Channel | SimRep staircase | SimRep actual surfaces | Actual surfaces, no support collisions |
|---:|---:|---:|---:|
| 1 | 0.261 eV, 1,529 hits | 0.174 eV, 1,101 hits | 0.258 eV, 1,531 hits |
| 16 | 38.2 eV, 1,527 hits | 25.8 eV, 1,114 hits | 38.1 eV, 1,537 hits |
| 32 | 7,871 eV, 1,591 hits | 5,611 eV, 1,121 hits | 7,980 eV, 1,580 hits |

Channel 32 FWHM has large sampling uncertainty: the actual-surface FWHM bootstrap standard deviation is 1,728 eV. MCP `analyzer_scan` jointly bootstraps particles across voltages to report centroid, FWHM, and FWHM/E sampling uncertainty, separately from scan spacing and grid error. See [analyzer diagnostics](mcp/README.md#analyzer-voltage-convention).

![SIMION 2024 and SimRep compute time at matched accuracy](docs/images/esa/esa_simion_timing.png)

| Run (channel 26, 4,096 electrons) | Compute time (median of 3 runs) | Speedup over SIMION |
|---|---:|---:|
| SIMION 2024, trajectory quality 9, 1 thread | 8.08 s | 1x |
| SimRep RK45 tolerance 1e-3, 1 thread | 0.475 s | 17x |
| SimRep RK45 tolerance 1e-3, 16 threads | 0.040 s | 204x |

SIMION ran single-threaded with Lua user programs and recording disabled. SimRep used the same staircase collisions. Only compute time is compared, excluding file loading and preparation. Against an RK45 1e-11 reference, detector-position RMS was SIMION 0.13 mm and SimRep 0.12 mm, a comparable accuracy level. Another job occupied a CPU core during measurement.

---

## Desktop app

Choose a template or open a SIMION file, then follow **geometry → field solve → voltage and particle settings → flight → results** in one window.

![Import → solve → fly → results](docs/report/figures/F03.png)

- **Live sliders**: the GPU continually re-flies ions while you drag electrode voltages.

  ![Live re-flight with a voltage slider](docs/report/figures/F09.png)

- **5-step tour**: a real template introduces voltage adjustment, flight, TOF histograms, and CSV export.

  ![Tour step 2](docs/report/figures/F21.png)

- **Surface badges** distinguish actual-surface computation (precise mode, CPU double precision) from live preview (GPU single precision).

  ![Exact surfaces enabled and disabled](docs/report/figures/F22.png)

- Korean/English UI, light/dark/system themes, undo/redo, and autosave 1.5 seconds after changes.
- **Saved flights**: project v2 stores per-ion results, trajectories, and settings under `X.simrep.d/runs/` beside `X.simrep`. Reopening reuses saved results when the array and settings match, or re-flies otherwise. See [project format](docs/project-format.md).
- **History and comparison**: the results panel retains the latest 50 runs by default, plus pinned runs. Rename, annotate, pin, or delete runs; compare a pair using histograms, detector maps, and setting differences.
- **AI chat history and reports**: save and reopen assistant conversations with the project; export an HTML report with run summaries, plots, and comparisons, or print/save PDF. See [frontend documentation](docs/dev/frontend.md).
- Import STL, OBJ, PLY, 3MF, and glTF meshes, and STEP/IGES CAD electrodes.
- [Visual QA record](docs/report/visual-qa/REPORT.md): 42 surfaces, light/dark themes, at widths 1280 and 1920.

### Installation (Windows)

The Windows NSIS installer (about 28.5 MiB) installs per user **without administrator privileges**.
It includes OpenCascade for STEP/IGES import and the Visual C++ runtime.
See the Windows installer section in [frontend documentation](docs/dev/frontend.md) for build instructions.

GPU features require a graphics card supporting WebGPU/Vulkan or DX12. Without a GPU, computation uses the CPU.

---

## AI assistant

The desktop app's **Assistant** panel lets you control SimRep by chatting with an AI such as ChatGPT or Claude, in English or Korean.
The AI invokes SimRep MCP tools (`pa_info`, `refine`, `fly`, `summarize_results`, etc.) to solve fields, fly ions, and summarize results.
Sign in with your own subscription account; the provider's plan and billing apply.

![Einzel lens 3D view and AI panel with refine/fly cards and a Korean response](docs/images/ai/ai-panel.png)

This screenshot records a real ChatGPT account asked to solve an einzel lens and fly ions.
The model checked files with `simrep_plan` and `pa_info`, requested approval for `refine` and `fly`, then read the result CSV to summarize convergence, detector arrivals, and TOF in Korean.
The app's 3D view and histogram show the template flight; the AI ran separately in the same folder with its own settings.

- **Approval modes: ask / allow all in chat / bypass.** Ask mode requires an approval card for writing tools such as `refine`, `fly`, and export; unanswered requests expire after two minutes. Read-only tools run directly. Allow all in chat or bypass removes repeated clicks, with visible bypass/automatic indicators. Every mode remains restricted to SimRep tools inside the workspace.

  ![Approval card for the fly tool](docs/images/ai/ai-approval.png)

- **Credentials stay in the local bridge, outside the WebView.** Sign-in uses the system browser; only the local AI bridge (`ai/`) holds tokens. The app window receives account display names, conversations, and tool events, not tokens. The AI has no shell or arbitrary file access, only workspace-restricted SimRep tools.

  ![AI accounts with account names redacted](docs/images/ai/ai-accounts.png)

- **Providers**: subscription OAuth supports ChatGPT, Claude, GitHub Copilot, xAI (Grok), Kimi Code, Devin, Meta, Kiro, and OpenRouter. Cursor and Radius support sign-in only, not chat yet. OpenAI, Gemini, DeepSeek, Mistral, and others can use environment-variable API keys, billed as API usage rather than subscription usage.
- **Harness**: every provider receives the same system context (units, device rules, PA families, and verification rules). Task recipes for einzel lenses, ESA, quadrupoles, magnetic fields, and CAD load on demand. `analyzer_scan`, `grid_convergence`, `check_geometry`, and `compare_runs` handle analyzer scans, numerical grid convergence (u_num), gap checks, and comparisons. Reported speeds are checked against tool `compute_ms`; fallback or nonconvergence gets a `[verify]` note.
- **Bounded design search**: MCP `optimize_design` runs a resumable GEM design loop with explicit budgets. Budget exhaustion returns a candidate, not proof of an optimum. See [MCP tools](mcp/README.md).
- **Getting started**: run `npm ci` in `ai/`, build the SimRep CLI, and launch the app. Sign in from the top bar and open **Assistant**. See [AI setup and providers](ai/README.md) and [harness architecture](ai/harness/README.md).
- **Limits**: automated tests use fake servers for authentication and tool flows; real provider login and plan access require account-level verification. Model availability depends on the provider and account plan. See “Providers, billing and availability” in [AI documentation](ai/README.md).

---

## Command-line tool `simrep`

```sh
# Build from the repository root
cargo build --release -p simrep-cli

# Help
./target/release/simrep --help
```

STEP/IGES import requires OpenCascade (OCCT) and is not part of the default build.
Build with `cargo build --release -p simrep-cli --features cad`; see [CAD import](docs/step-import.md).
The Windows installer already includes CAD support.

Common commands (filenames are examples):

```sh
# Convert SIMION GEM to a potential array (PA#), including GEM Lua preprocessing
simrep gem2pa quadrupole.gem

# Solve potentials. auto uses measured GPU thresholds only for dielectric materials;
# ordinary electrostatic inputs use CPU. Reproducing the GPU 19.3x result needs --device gpu
simrep refine quadrupole.pa#

# Solve GEM/mesh/CAD with actual surfaces and write the exact-surface .cuts.json file
# For GEM, assign every source electrode exactly once as SRC=N:V
simrep refine-geometry bench/quadrupole.gem --electrode 1=1:2.5 --electrode 2=2:-2.5 --electrode 3=3:0 --out quadrupole
# Mesh/CAD selectors are part names (or indices for unnamed parts), not GEM source values
# Example: simrep refine-geometry housing.stl --grid 121,37,37 --electrode housing=1:2.5 --out housing

# Fly with adjustable electrode voltages; sibling .cuts.json is used automatically
simrep fly --pa quadrupole.pa0 --particles ions.ion --voltage 1=2.5,2=-2.5

# Export fields and trajectories to VTK/CSV and other formats
simrep export grid quadrupole.pa0 field.vti
```

- **Sampled-SDF surface backend (experimental)**: `--surface-backend sdf --device gpu` enables approximate GPU flights with source-geometry collision surfaces; CPU SDF is also available. The reported certified interpolation error bound describes the approximation, not exact intersections; **CPU exact is the reference**. The default remains `exact`; opt-in `--surface-backend auto` currently also selects CPU exact. See the [RTX 3070 measurements and crossover policy](docs/dev/sdf-auto-crossover.md).
- **Portable flight sharding**: `fly --shard i/N` splits independent ions across processes, GPUs, or PCs without MPI. Select a GPU with `--device gpu --gpu-adapter NAME_OR_INDEX`; merge with `simrep merge-flights out.csv a.csv b.csv` or Python `simrep.merge_flights(output, shards)`. Preserve JSON sidecars and identical inputs, settings, backend, and precision: the merged CSV is byte-identical to an unsharded run on the same numerical path. Source indices and random streams are preserved; space charge and beam modes cannot be sharded. See [flight shards](docs/python.md#merging-portable-flight-shards).
- **Aperture checks**: `aperture-check` compares staircase, effective, and exact aperture widths along probe lines. See [Python/CLI mapping](docs/python.md#aperture-checks).
- **Collision-only insulators**: `refine-geometry --insulator PART` adds collision geometry without a potential boundary. For staircase flight, `--no-cuts --surface-collisions insulators|all` restores selected collision surfaces; GPU execution requires SDF. See [collision-only insulators](docs/python.md#collision-only-insulators).

Other features include `mesh-import`, external magnetic maps from COMSOL/Opera and similar tools (`fly --bfield-map`), and time-varying voltages (`--voltage-expr`).
Use `simrep <command> --help` for options.

### `--json` output

For automation and AI agents, place the global `--json` flag before or after the command.
Successful commands write a single JSON summary to stdout and progress to stderr.
`--help` and `--version` keep their text output. Write flight CSV with `--output`; without a filename, JSON mode sends CSV to stderr too.

```sh
simrep --json refine quadrupole.pa# --device cpu
simrep fly --json --pa quadrupole.pa0 --particles ions.ion --output flight.csv
```

Every summary includes `schema_version: 1` and `simrep_version`.
Flight returns `splat_counts`, detector `arrivals`, and `arrival_tof_us` (mean, population standard deviation, minimum, maximum; null without arrivals).
Solves return `cycles`, `final_residual` (relative residual), `tolerance`, and `converged`.
Both report `compute_ms` separately from `prep_ms`, and include `requested`, `executed`, and `fallback_reason`.
Across multiple basis solves, cycles are summed and residual is the largest final residual. CPU's executed backend is null.
GPU solve precision `mixed-f32-f64` means f32 corrections with an f64 iterate and residual.
Conversions and exports report actual files in `output_paths`.
Failures return a single `{"error":{"message":"...","kind":"..."},...}` object, exit status 1, and an error on stderr.
`error.kind` is `invalid_arguments`, `io`, or `command_failed`.

---

## Python package `simrep`

Use the CLI features with NumPy arrays for parameter sweeps and analysis (Python 3.10 or newer).

```sh
python3 -m venv .venv && . .venv/bin/activate
python -m pip install maturin numpy scipy
maturin develop --release --manifest-path crates/simrep-py/Cargo.toml
```

```python
import simrep

field = simrep.refine("einzel.pa#")             # Solve potentials
result = simrep.fly(field, "ions.ion")          # Fly ions
```

Examples: [einzel voltage optimization](crates/simrep-py/examples/einzel_scan.py), [TOF resolution scan](crates/simrep-py/examples/tof_scan.py).
See the [CLI-to-Python mapping](docs/python.md).

---

## Supported physics

- Electrostatics (Laplace/Poisson), cylindrical symmetry and 3D
- Magnetic fields: uniform, magnetic PA, loop/solenoid coils (Biot–Savart and coil maps), imported field maps
- Magnetic materials: iron cores and permanent magnets (scalar potential), currents (vector potential)
- Linear and nonlinear dielectrics
- Neutral-gas collisions: HS1 and SDS models from SIMION examples
- Space charge: direct Coulomb sum, PIC, SIMION beam mode
- Relativistic motion; adaptive RK45 integration (optional Boris)

---

## Repository layout

| Path | Contents |
|---|---|
| `crates/` | 11 Rust core crates: solvers, flight, file I/O, GEM/Lua, mesh, CAD, CLI, Python |
| `app/` | Desktop app (Tauri 2 + React + WebGPU) |
| `docs/` | Design and validation; start with [plan](docs/PLAN.md) and [implemented equations](docs/spec/equations.md) |
| `bench/` | SIMION speed and accuracy comparisons |
| `docs/report/` | Technical report (DOCX/PDF), figures, and source ledger |
| `tools/` | Build, comparison, and Windows utilities |

For development checks (full tests, GPU verification, etc.), see “WSL build and checks” in [Python documentation](docs/python.md), [Windows GPU guide](docs/dev/windows-gpu.md), and [frontend guide](docs/dev/frontend.md).

---

## Known limitations

- Device measurements cover a single RTX 3070. Magnetic flight always uses CPU under `--device auto`; supported combinations can explicitly request `--device gpu` (vector-potential and magnetic-material paths remain CPU-only). Ordinary electrostatic refine also stays CPU under auto; only dielectric materials use its measured GPU thresholds. Changing automatic selection for another GPU requires measurements and a policy update.
- The default exact-intersection surface backend remains CPU-only. Experimental GPU surface flight uses sampled SDF, with a certified approximation bound; auto currently stays CPU exact. See [crossover record](docs/dev/sdf-auto-crossover.md).
- SIMION workbench Lua user programs are not supported. Use configuration and expressions for time-varying voltages and detection.
- No STEP export or nonlinear B–H magnetic materials yet.
- External magnetic maps must use regular grids.
- Fine-grid numerical reference solutions are not rigorous error bounds against a continuous solution.

See chapter 9 of the report for the full list.

---

## Relationship to SIMION

SimRep neither uses nor reverse-engineers SIMION code (its license agreement prohibits this).
Only command-line batch outputs, installed examples, and public documentation were used for comparison.
SIMION is a product of Scientific Instrument Services.

## License

SimRep is licensed under the [PolyForm Noncommercial License 1.0.0](LICENSE) (`PolyForm-Noncommercial-1.0.0`): you may use, modify and share it for any noncommercial purpose, including personal study, research and use by educational, public research or charitable organizations. Commercial use is not licensed.
Parts adapted from Iteration-CFD are under the Prosperity Public License 3.0.0 (also noncommercial); see [licenses/THIRD_PARTY.md](licenses/THIRD_PARTY.md).
OpenCascade, used for STEP/IGES import, is LGPL 2.1 with an exception; it and all other bundled third-party components keep their own licences, and their notices ship with the installer.

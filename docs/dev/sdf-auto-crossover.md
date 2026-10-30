# Automatic GPU SDF crossover record

Status: unmeasured. No GPU SDF crossover is enabled.

The single policy table is `crates/simrep-cli/src/sdf_auto.rs`, compiled into
both the CLI and Python extension. Its keys are native flight precision
(f64 or f32) and SDF resolution divisor (2 or 4). Every entry is `None`, meaning
never automatically choose GPU SDF. The exact default is independent of this
table and is never changed by its values or the experiment override.

Before replacing any `None`, append a measurement record here identifying:

- The raw artifacts, SimRep revision, hardware adapter, Vulkan driver and OS.
- The precision and resolution divisor, geometry, ion source, and physics.
- CPU exact and GPU SDF `compute_ms` on both sides of the proposed threshold,
  with repeated runs and contention disclosed. Preparation and export are excluded.
- Surface approximation errors and termination/trajectory comparisons against
  CPU exact, including the reported certified interpolation bound.

A timing alone is not an accuracy validation. A number must have supporting
artifacts and a justified applicable workload before entering the policy table.

`SIMREP_SDF_GPU_CROSSOVER=<ions>` temporarily overrides the threshold for
experiments at the supported table keys. It does not bypass the surface,
device, Vulkan hardware, GPU physics, or storage-binding requirements. Invalid
ion counts are errors. GPU SDF currently supports the single electric-instance
path only, so magnetic fields and collision models, coupled direct/PIC space
charge, beam/leader integration, adaptive fields, and transparent faces keep
auto on CPU exact. Native GPU flight precision
follows adapter capability (f64 when supported, otherwise f32).

Under opt-in `surface_backend=auto`, absence of a measured crossover resolves
to CPU exact, including with device gpu. This policy decision has a null
`fallback_reason`; `reason` explains the selected path. Other resolution
divisors also stay CPU exact because they have no table key.


## Measurement 2026-10-30 (RTX 3070, Vulkan) - decision: keep every entry `None`

Revision: 889d102 (kernel ca4ce86 + dispatch 889d102). Adapter: NVIDIA GeForce RTX 3070, Vulkan, Windows 11;
native f64 via SHADER_F64. Workload: einzel `refine-geometry` example (electrodes 1=1:0, 2=2:60, 3=3:0),
deterministic beams of 2,400 / 24,000 / 240,000 electrons with angular spread (most hit electrodes),
detector plane x = 58 mm, electric-only physics. `compute_ms` medians of 5 serial runs under a shared
timing lock; other user jobs (Opera FEA) were running, so timings are contended.

| Ions | N | CPU exact | GPU SDF f64 | GPU SDF f32 |
|---:|---:|---:|---:|---:|
| 2,400 | 2 | 22.1 | 462.0 | 64.1 |
| 24,000 | 2 | 214.1 | 1,436.0 | 206.7 |
| 240,000 | 2 | 2,126.2 | 11,705.2 | 1,680.0 |
| 240,000 | 4 | 2,126.2 | 12,089.5 | 2,481.0 |

CPU exact is from the same machine and beams (all threads). GPU SDF outcomes matched CPU SDF for
120/120 flights. The best GPU SDF speedup over CPU exact was 1.39x (f32, 240,000 ions, N=2), below the
1.5x threshold required for an automatic switch, and native f64 never beat CPU exact. No crossover is
enabled. Re-measure on an idle GPU or a faster adapter before entering a number.

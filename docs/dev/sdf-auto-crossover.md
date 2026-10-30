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

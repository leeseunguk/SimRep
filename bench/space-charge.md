# Space-charge validation and compute timing

Date: 2026-10-23. Ryzen 9 5950X, WSL Linux native CPU f64, release build,
four Rayon workers. SIMION 2024-09-26-8.2.1.0 is the native Windows oracle.
The historical tables below are CPU space-charge measurements. The additive
GPU PIC and documented beam studies are described in the new section below.

Current default-only runs use automatic geometry/physics step ceilings and
1e-12 external tolerances. Factor/Coulombic compute is 0.279076 / 0.248803 s,
below SIMION's 0.91 / 0.93 s. Endpoint RMS against the same independent
DOP853 reference improves from 4.10729e-7 / 1.47293e-3 mm with the old CLI
defaults to 3.81460e-7 / 1.01389e-4 mm. See `bench/default-flight.md`;
the manually selected fine steps below remain distinct configured studies.

## Matched initial states

`tools/simion-oracle/space_charge_compare.py` makes a separate working copy
of `repulsion1.iob`, its original 103-particle FLY2 source, and `blank.pa`.
The readout program sets grouped, nonrelativistic flight and T.Qual=-9,
then selects factor=1 or Coulombic amount=3.47e-13 C. The original shipped
IOB's Beam mode is therefore deliberately replaced by the stated point/cloud
mode; these are controlled variants, not reproductions of its Beam algorithm.
The original `.oracle_recording.txt` is not an endpoint oracle (it records
Lua's space-charge potential diagnostics).

Every actual randomly drawn birth, mass, charge, position, velocity and CWF
is recorded and replayed, so the two programs do not get different random
samples. The replay converts measured velocity to native KE using the native
conversion constants. SIMION `ion_time_of_flight` includes TOB; the harness
subtracts TOB to compare native lifetime TOF. The external PA is zero, placed
at (-10,0,0) with scale 0.1; both workbench bounds are
[-10,-4,-4] to [10,4,4] mm. All 103 particles physically terminate.

Artifacts are outside the repository in
`C:\Users\lsu10\simrep-oracle\space-charge\{factor,coulombic}`:
`oracle.txt`, `replay.ion`, `native-{1e-04,5e-05}.csv`, and
`comparison-{1e-04,5e-05}.json`.

## Endpoint and TOF statistics

Native uses exact point Coulomb, a synchronized midpoint Boris step,
the closest-approach criterion with fraction=0.1, and external tolerance=1e-8.
dt below is the maximum, not a claim that every global step has that length.
Endpoint RMS/max are per-particle Cartesian distances; TOF RMS is the
per-particle lifetime difference.

| Mode | Max dt (us) | Endpoint RMS / max (mm) | TOF RMS (us) | SIMION mean TOF (us) | Native mean TOF (us) |
|---|---:|---:|---:|---:|---:|
| factor=1 | 1e-4 | 0.010609165 / 0.085873792 | 1.224707e-5 | 0.0344562145 | 0.0344658318 |
| factor=1 | 5e-5 | 0.010609147 / 0.085873713 | 1.224709e-5 | 0.0344562145 | 0.0344658318 |
| Coulombic 3.47e-13 C | 1e-4 | 0.590094028 / 2.792020702 | 6.026634e-4 | 0.0357518200 | 0.0356071799 |
| Coulombic 3.47e-13 C | 5e-5 | 0.590068669 / 2.791852127 | 6.026187e-4 | 0.0357518200 | 0.0356071912 |

Comparing the two **native** time resolutions directly gives:

| Mode | Endpoint RMS / max change (mm) | TOF RMS change (us) |
|---|---:|---:|
| factor=1 | 1.170760e-7 / 7.520758e-7 | 6.479561e-10 |
| Coulombic 3.47e-13 C | 3.580365e-5 / 1.874363e-4 | 5.354996e-8 |

The strong Coulombic SIMION difference is not explained by our remaining
time-step error. The independent reference below resolves the accuracy verdict:
**native is substantially more accurate for the stated point-Coulomb problem**.
Quantitative compatibility with SIMION's built-in cloud algorithm is not claimed.

Accuracy authority remains our analytic and self-converged references in
`docs/physics-space-charge.md`: direct and PIC both show second-order dt
halving; a uniform sphere agrees with its continuum envelope to 0.33%,
direct/PIC radius scales agree to 0.027%, and a uniform cylindrical PIC beam
agrees with the analytic envelope integration to 0.0171%.

## Independent accuracy verdict

`bench/space_charge_reference.py` integrates the recorded initial positions
and velocities with SciPy 1.18.1 DOP853, not the production force sum, Boris
push, field interpolation, or event code. NumPy 2.2.6 evaluates
`p'=v`, `v'_i=k*(q_i/m_i)*ELECTRIC_ACCELERATION*sum_j Q_j*r_ij/|r_ij|^3`.
The self term is excluded. Births restart the coupled ODE at their exact
recorded times; terminal, outward box-crossing events use DOP853 dense output
and remove the source before the remaining particles resume. Charges are
fixed from the entire input population, including future births.
The reference uses `rtol=3e-13`, position `atol=1e-12` mm and velocity
`atol=1e-10` mm/us. The three maximum steps are 0.001, 0.0005 and 0.00025 us:

| Mode | Reference endpoint RMS change, first halving (mm) | Second halving (mm) | Second-halving max (mm) |
|---|---:|---:|---:|
| factor=1 | 3.260859e-12 | 2.768064e-12 | 1.016605e-11 |
| Coulombic 3.47e-13 C | 9.500638e-12 | 6.323582e-13 | 6.112797e-12 |

The finest independent solution is the accuracy reference, not SIMION:

| Mode / implementation | Max dt (us) | Endpoint RMS / max error (mm) | TOF RMS error (us) |
|---|---:|---:|---:|
| factor=1 native | 1e-4 | 3.300258e-7 / 2.161880e-6 | 3.383178e-9 |
| factor=1 SIMION T.Qual=-9 | - | 0.010609200 / 0.085874419 | 1.224799e-5 |
| Coulombic native | 1e-4 | 4.863959e-5 / 2.509349e-4 | 7.258986e-8 |
| Coulombic native | 5e-5 | 1.284997e-5 / 6.431056e-5 | 1.905794e-8 |
| Coulombic native | 2.5e-5 | 3.335221e-6 / 1.699775e-5 | 4.939690e-9 |
| Coulombic native | 1.25e-5 | 8.456660e-7 / 4.337054e-6 | 1.250408e-9 |
| Coulombic SIMION T.Qual=-9 | - | 0.590059409 / 2.791788202 | 6.026025e-4 |

Strong-charge endpoint error ratios are 3.785, 3.853 and 3.944: second-order
dt convergence is preserved, including births and source loss. The original
analytic two-body and PIC convergence tests also remain in the release suite.
The production fraction stays 0.1 and softening stays zero. Factor=1's
closest-encounter limit also constrains dt, so reducing only `max_step_us`
does not uniformly halve every step; its 3.30e-7 mm RMS error is already
well below SIMION's 0.0106 mm.

The actual external `blank.pa` is asserted to contain only zero potentials.
The harness creates 1x, 2x, 4x and 8x node refinements at the same physical
extent, with scales 0.1, 0.05, 0.025 and 0.0125. At max dt=1e-4 **all endpoint
CSV bytes are identical across those grids in both modes**. This is expected:
the exact external field is zero at every resolution, so its spatial error
is zero, rather than an unmeasured electrode interpolation error. This verdict
does not extend to a nonzero-field repulsion geometry without its own refinement.

The model distinction is precise but does not identify a private kernel:
native uses unsoftened point Coulomb at synchronized force times, with
`Q_i=Q*q_i*CWF_i/sum(abs(q_j)*CWF_j)` in Coulombic mode. Here the denominator
is 100.002 (100 unit-CWF electrons, two CWF=0.001 electrons, and a neutral).
SIMION's local `charge_repulsion.html` documents nonzero automatic clouds in
Coulombic mode; its special factor=1 effective cloud **diameter** is 3e-11 mm.
The Coulombic cloud radius and overlap force law are not specified there,
so no Plummer radius is fitted or claimed equivalent. The same page's
"More repulsion notes" describes last-known per-particle contributions while
advancing particles in sequence, unlike the native equal-time midpoint sum.
Its endpoint localization also differs: the neutral factor-case leader ends
at x=9.99773264 instead of the exact x=10 face. These are documented
model/method differences, not evidence that SIMION defines the correct
point-charge solution. Their individual contributions to 0.590059409 mm
are not isolated by this experiment.

Independent artifacts are under
`/tmp/simrep-space-charge-reference/{factor,coulombic}`:
`dop853-events-*.csv`, `native-*-x{1,2,4,8}.csv`, refined blank PAs, and
`verdict.json` with commands, timings and errors. Reference caches are keyed
by the exact initial states and represented charges; reference-only runs
write `reference.json` without replacing a measured native verdict.
The archived SIMION inputs and results are not overwritten.

## Compute time only

Native `EnsembleReport.compute_time` includes force sums, step decisions,
geometry, pushing, and (for PIC) every deposition, Poisson solve and gather.
Runtime worker creation, mesh construction, external-field reconstruction,
input/output and CSV formatting are excluded. Per-step CPU Poisson hierarchy
construction is included; only its potential is warm-started.
SIMION times below are its own `Fly completed ... seconds`, not process
wall time. Native replay runs were serialized; the SIMION timings are reused
from the recorded, matched-initial-state oracle runs above.

| Mode | Native max dt (us) | Native compute (s) | SIMION compute (s) | SIMION / native |
|---|---:|---:|---:|---:|
| factor=1 | 1e-4 | 0.226757 | 0.91 | 4.01 |
| factor=1 | 5e-5 | 0.299248 | 0.91 | 3.04 |
| Coulombic 3.47e-13 C | 1e-4 | 0.267262 | 0.93 | 3.48 |
| Coulombic 3.47e-13 C | 5e-5 | 0.369717 | 0.93 | 2.52 |
| Coulombic 3.47e-13 C | 2.5e-5 | 0.468491 | 0.93 | 1.99 |
| Coulombic 3.47e-13 C | 1.25e-5 | 0.759510 | 0.93 | 1.22 |

**The faster-than-SIMION goal is met on both small-N direct cases at better
accuracy against the independent point-charge reference.** This is not a
claim to reproduce SIMION's cloud model. Even the finest native strong-charge
run is faster than the recorded SIMION run.
The table and `verdict.json` contain the final serialized pass. The first
pass at 1e-4 measured 0.250403 s (factor) and 0.232196 s (Coulombic), so
the observed faster-than-SIMION result is not dependent on selecting one
particularly fast run.

The cause was launching Rayon twice per global attempt for a population of
only tens of live particles, despite 103 total input births. Before changing
code, a thread-count ablation at max dt=1e-4 measured:

| Mode | Old four-worker compute (s) | Old one-worker compute (s) | New four-worker runtime, first pass (s) |
|---|---:|---:|---:|
| factor=1 | 1.744348 | 0.714027 | 0.250403 |
| Coulombic | 1.569488 | 0.629932 | 0.232196 |

Those fresh old-code timings differ from the earlier 3.03/2.80 s measurements;
the same-input before/after ablation is the performance evidence. Both direct
force and closest-approach sums now use ordinary iteration below 256 live
particles and retain target-parallel Rayon at 256 and above. Source summation
order, the stability criterion, all tolerances, and the Boris push are unchanged.
The 1e-4 endpoint CSVs are byte-identical to the old-code replays. Factor still
uses 4,838 accepted global steps / 204 retries and Coulombic 4,412 / 306,
including births and globally localized splats. The regression checks the
103/255/256/257-particle paths with one and four workers, both point and
softened kernels, for identical source-order results.

A separate, deterministic 10,000-particle cold sphere probe demonstrates the
requested N range and the reason to switch algorithms. Both methods advance
10 steps of 0.001 us; total represented charge=1e-14 C, mass=100 amu, charge=1.
PIC uses a 33³ mesh with 0.25 mm cells and zero correction on the six faces,
Poisson relative tolerance=1e-8:

| Method | N | Global steps | Compute (s) |
|---|---:|---:|---:|
| direct O(N²), no N² matrix | 10,000 | 10 | 5.964276306 |
| CIC PIC, warm potential | 10,000 | 10 | 0.246605100 |

PIC is 24.2 times faster on this probe. This is an algorithm-scaling
measurement, **not** a SIMION speed comparison or evidence that every pairwise
encounter is resolved by the mesh.

## Reproduction

Final checks all exited 0:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
RAYON_NUM_THREADS=16 cargo test --workspace --release
cargo build --release -p simrep-cli
```

The complete workspace suite includes all ten space-charge tests, the
direct/PIC CLI test, CWF input regressions, planar/cylindrical manufactured
Poisson sources, and the existing CPU/GPU and geometry tests. No existing test
was suppressed or weakened. The release GPU smoke test reported two checks executed
and the unavailable DX12 backend skipped; this is not a Windows GPU validation.
A prior unoptimized run reached the monitor deadline in
the existing 257³ cut-cell test; optimized test code completed that test in
21.80 seconds. The final subscription allowed 30 minutes.
The Rust LSP daemon timed out on diagnostics; warning-free
Clippy and the full compiler/test run are the available diagnostic evidence.
Oracle replay also exposed a sub-ULP boundary-crossing clock stall; it is fixed
and covered by `sub_ulp_crossing_does_not_underflow_the_global_clock`.

```sh
cargo build --release -p simrep-cli
python3 tools/simion-oracle/space_charge_compare.py --mode factor --amount 1
python3 tools/simion-oracle/space_charge_compare.py --mode coulombic --amount 3.47e-13
python3 tools/simion-oracle/space_charge_compare.py \
  --mode factor --amount 1 --native-only --native target/release/simrep
python3 tools/simion-oracle/space_charge_compare.py \
  --mode coulombic --amount 3.47e-13 --native-only --native target/release/simrep
# Repeat native-only calls with --max-step 5e-5 for the refinement above.
# Independent reference, native dt halving, and 1/2/4/8x external-grid probes:
OPENBLAS_NUM_THREADS=1 python3 bench/space_charge_reference.py \
  --mode coulombic --native target/release/simrep \
  --native-steps 1e-4 5e-5 2.5e-5 1.25e-5
OPENBLAS_NUM_THREADS=1 python3 bench/space_charge_reference.py \
  --mode factor --native target/release/simrep \
  --native-steps 1e-4 5e-5 2.5e-5 1.25e-5
RAYON_NUM_THREADS=4 cargo run --release -p simrep-fly --example space_charge -- 10000
```

The harness's Windows invocations are foreground, with a C-drive cwd, closed
stdin and a 25-second subprocess deadline. The archived actual oracle calls
exited 0; this follow-up reused them rather than running Windows flight again.
The prescribed cleanup command exited 0:

```text
== process counts ==

Name           Count
----           -----
cmd.exe            4
conhost.exe       44
node.exe           4
powershell.exe     1

== conhost by parent ==

Name                              Count
----                              -----
wsl.exe                              22
wslhost.exe                           5
cmd.exe                               4
lmgrd.exe                             2
node_repl.exe                         2
rlm.exe                               1
node.exe                              1
powershell.exe                        1
codex.exe                             1
sunshine.exe                          1
dispatcher.exe                        1
GoogleDriveFS.exe                     1
wlanext.exe                           1
matlab-mcp-server-windows-x64.exe     1

orphan conhost: 0
stopped orphan conhost: 0
```

## Documented beam and physical GPU PIC (2026-10-24)

These are additive contracts, not changes to the historical Coulombic/factor
studies above. Beam uses `physics::fly_beam` with single-pass leader-normal
planes and unequal follower TOFs. It is labelled **SIMION-documented beam
semantics**. The explicit infinite-line/cylindrical-shell kernel, CWF/current
normalization, static-field requirement, loss rules and omitted private cloud
details are in `docs/physics-space-charge.md`. No iterative steady-state
trajectory/Poisson solution is claimed.

The cold unit-radius cylindrical beam has v=10 mm/us, I=1e-8 A, q/m=1 e/amu
and duration 0.48 us. Independent RK4 of `R''=C/R` gives R=1.193692751585 mm.
The 48-shell ring-current representation has the correct enclosed-current
field including the symmetric half-self-shell limit:

| Maximum leader dt (us) | Radius error (mm) | Error ratio |
|---|---:|---:|
| 0.04 | 4.3327595e-5 | - |
| 0.02 | 1.0830672e-5 | 4.00 |
| 0.01 | 2.7075909e-6 | 4.00 |

Beam regression tests separately check line-current units and absent axial
force, unequal follower TOFs, TOB invariance, moving plane normals, backward
loss, and boundary localization down to position/clock roundoff. Endpoint
roundoff is not allowed to create a succession of non-advancing half-clock
steps; the full follower-time guess is formed directly from the plane speeds.

### GPU PIC agreement and compute time

The native Windows Vulkan adapter was NVIDIA GeForce RTX 3070, DiscreteGpu.
Physical GPU Poisson was also exercised through DX12's CPU-f64-residual
fallback and Vulkan's native and forced-CPU residual paths. Planar,
cylindrical-axis and offset-axis physical sources agree with CPU Poisson;
fixed source nodes are ignored. The physical RHS convergence scale is the
same gauge-shifted **combined** source/boundary RHS used by CPU Poisson,
including small nonzero sources rather than a unit-voltage fallback.

At N=256 with staggered births, external E/B and a 17³ Cartesian mesh, maximum
CPU/GPU differences were 1.45908e-14 V/mm in gathered field, 4.44089e-16 mm
in final position, and 1.69725e-14 mm/us in velocity. The independent
cylindrical CIC/gather comparison was within 7.45290e-14 V/mm. These are
actual hardware results, not the Linux software-adapter check.

The bounded compute study uses 100,000 identical launches in a 33³ mesh,
total charge 1e-15 C, dt=0.0002 us, duration 0.0006 us (three global steps),
four Rayon workers, and Poisson relative tolerance 1e-11 on both devices.
GPU construction, shader compilation and hierarchy preparation are excluded;
every in-flight transfer, synchronization, deposition, Poisson solve, field
gather, push, event decision and external-field estimate is included.
CPU rebuilds its current Poisson hierarchy each force evaluation; GPU reuses
the prepared hierarchy. No process-start or CSV time enters the comparison.

An initially contended timing set was discarded and repeated after the
physical-RHS scale correction:

| Trial | CPU compute (s) | GPU compute (s) |
|---|---:|---:|
| 1 | 0.4044553 | 0.2526633 |
| 2 | 0.4073832 | 0.2895435 |
| 3 | 0.4340534 | 0.2997268 |
| Median | **0.4073832** | **0.2895435** |

The measured CPU/GPU compute ratio is **1.407** at matched precision;
maximum endpoint disagreement across all 100,000 particles was
3.33067e-16 mm. This is a short bounded large-N study, not a claim of that
speedup at every mesh size or flight duration. CLI auto selection enables
GPU PIC at N >= 100,000; explicit GPU mode supports smaller accuracy studies.

Reproduce the native hardware test after the Windows build recipe:

```sh
export SIMREP_PIC_BENCH=1 RAYON_NUM_THREADS=4
export WSLENV="${WSLENV:+$WSLENV:}SIMREP_PIC_BENCH:RAYON_NUM_THREADS"
# From /mnt/c, closed stdin, foreground, bounded by timeout:
cargo.exe test --release -p simrep-fly --test gpu_pic
cargo.exe test --release -p simrep-solve --test gpu_poisson
```

The actual runs used 60/90-second executable deadlines and a 180-second
incremental build-and-test deadline, all from `/mnt/c` with stdin closed.

### SIMION beam endpoint study

**Verdict: the numerical beam defaults beat SIMION at equal-or-better
accuracy against the stated converged line-model references: 3.19x faster
for repulsion1 and 1.99x for mixed beam, using compute time only.** This is
not a claim to reproduce SIMION's undocumented private cloud kernel.

Both original cases are replayed from their actual recorded launches:
103 trajectories in `repulsion1`, 201 in `repulsion_mixed_beam`, I=1e-5 A,
grouped nonrelativistic SIMION Beam repulsion, T.Qual=-9. The originals are
unchanged. Endpoint instrumentation and working copies are under
`C:\Users\lsu10\simrep-oracle\beam-endpoints\{repulsion1,repulsion_mixed_beam}`.
The shipped potential-only Lua diagnostic is not an endpoint reference.
Recorded velocities determine native launch KE using native constants;
SIMION refinement reuses its original recorded KE. TOB is subtracted only
when comparing lifetime TOF.

Both cases have a neutral, straight leader and an exactly zero external PA,
at origin (-10,0,0), scale 0.1, with bounds [-10,-4,-4] to [10,4,4] mm.
`beam_reference.py` independently integrates the stated unsoftened parallel-line
model using DOP853 in plane position x, restarting at every side loss and
stopping survivors on x=10 mm. It shares no production force, integrator or
event code. Fixed launch populations and currents define the case; changing
N or adding a cloud radius changes the physical model rather than refining
this endpoint problem.

#### Independent endpoint convergence

Maximum plane spacing is halved; for the recorded leader speed
593.096975 mm/us this also halves the maximum leader dt. Adaptive DOP853
steps resolve closer encounters below that ceiling. The entries are changes
from the preceding resolution, not differences from SIMION:

| Case | Maximum plane spacing (mm) | Relative tolerance | Endpoint RMS change (mm) | Maximum change (mm) | TOF RMS change (us) |
|---|---:|---:|---:|---:|---:|
| repulsion1 | 0.25 | 3e-12 | - | - | - |
| repulsion1 | 0.125 | 3e-12 | 1.34610e-12 | 9.31703e-12 | 1.99816e-15 |
| repulsion1 | 0.0625 | 3e-12 | 1.81344e-14 | 9.59253e-14 | 2.86690e-17 |
| mixed | 0.25 | 3e-12 | - | - | - |
| mixed | 0.125 | 3e-12 | 5.33099e-8 | 6.59842e-7 | 4.99548e-14 |
| mixed | 0.0625 | 3e-12 | 4.64281e-8 | 5.74855e-7 | 3.54471e-14 |
| mixed | 0.0625 | 3e-13 | 1.83749e-6 | 2.27467e-5 | 1.04165e-12 |
| mixed | 0.0625 | 3e-14 | 1.75293e-7 | 2.16912e-6 | 1.01683e-13 |

The final mixed reference meets the chosen per-ion stability checks,
maximum endpoint change <1e-5 mm and maximum TOF change <1e-10 us;
the last maximum TOF change is 1.44161e-12 us. Every exit class is stable.
Tightening tolerance matters: the apparent fixed-tolerance spacing plateau
alone understated the remaining mixed error. The adopted references are
0.0625 mm / 3e-12 for repulsion1 and 0.0625 mm / 3e-14 for mixed.
The field is identically zero on every refined PA, so no external-field
solution error is hidden in these references.

The old mixed dt=1e-4/5e-5/2.5e-5 us runs changed individual endpoints by
0.204852/0.185873 mm RMS. At dt=1e-4 us, 187 charged ions exceeded 1e-4 mm
error: all electrons #2-101 and all carbon ions except
#106/#110/#137/#143/#149/#151/#153/#167/#173/#183/#186/#198/#201.
Stable aggregate radius was not evidence of accuracy.
The difficult part is close line scattering, not a fluctuating exit surface:
the rays focus near x=0, and the independent solver sampled charged pairs
as close as 7.34e-9 mm. Ion #78 dominates the reference tolerance sensitivity.
The converged mixed exit is ion #98 at y=+4 mm, x=9.11399179 mm; the other
200 trajectories end on the leader plane. For repulsion1 the reference has
31 leader-plane ends and 19/15/19/19 y+/y-/z+/z- exits.

Native resolution checks use those same references, not SIMION endpoints:

| Case | Maximum dt (us) | SC fraction | Endpoint RMS/max error (mm) | TOF RMS error (us) |
|---|---:|---:|---:|---:|
| repulsion1 | 1e-4 | 1 | 1.19035e-4 / 3.63651e-4 | 1.85459e-7 |
| repulsion1 | 5e-5 | 1 | 2.97059e-5 / 9.07787e-5 | 4.62683e-8 |
| repulsion1 | 2.5e-5 | 1 | 7.41981e-6 / 2.26760e-5 | 1.15547e-8 |
| repulsion1 | 1.25e-5 | 1 | 1.85419e-6 / 5.66707e-6 | 2.88725e-9 |
| mixed | 2.5e-5 | 1 | 0.463964 / 3.208129 | 1.62034e-5 |
| mixed | 2.5e-5 | 0.1 | 0.126925 / 1.110293 | 1.53864e-6 |
| mixed | 2.5e-5 | 0.05 | 0.134634 / 1.118323 | 3.91257e-7 |
| mixed | 2.5e-5 | 0.025 | 0.143594 / 1.103123 | 1.56906e-7 |
| mixed | 1e-6 | 0.05 | 0.0627212 / 0.616594 | 3.68191e-8 |
| mixed | 1e-6 | 0.025 | 0.0806870 / 0.867034 | 2.85655e-8 |

Repulsion1 shows second-order endpoint convergence. In mixed, refining only
the SC fraction at a fixed bulk cap does not establish convergence; the
near-focus scattering is sensitive to the incoming trajectory as well.
The tighter mixed context is the measured dt=1e-6/fraction=0.05 run, not a
claim that native mixed endpoints are individually converged. Its endpoint
RMS/max and TOF RMS errors are approximately 14.5/5.9/3,240 times below
SIMION's respective errors. The independently converged reference remains
the definition of accuracy for every row.

At fixed dt/fraction, refining the external PA by 2x and 4x while preserving
physical extent gave identical position, velocity and TOF arrays and
unchanged exit classes: repulsion1 at dt=1.25e-5/fraction=1 and mixed at
dt=2.5e-5/fraction=1. This isolates the mixed difficulty from field-grid error.

#### Numerical robustness and compute profile

The initial beam limiter used only nearest distance and net acceleration.
Signed fields can cancel during an unresolved pair encounter. The limiter
now bounds pairwise acceleration and relative transverse transit in leader
time. A second defect displaced midpoint positions with the old SC
acceleration before applying the midpoint kick. A two-attracting-line test
showed artificial angular-momentum change from -2.0822666961 to -2.0811892247
even at fraction 0.1. In the full mixed case, fraction 0.8 collapsed lines
#76/#125 to identical floating-point positions at x=-2.20597912 mm; fraction
0.75 accepted a state requiring dt=5.35e-19 us, below its clock ULP.

Force-plane positions now drift before the central kick; half-kick velocity
predictors retain second-order plane normals and densities. The conservation
regression covers fractions 0.1/0.75/0.8/1 and unequal follower clocks.
A charged, rotating leader has position errors
4.11055e-6/1.03149e-6/2.60885e-7 mm at dt=0.02/0.01/0.005 us.
Endpoint encounter-scale validation bisects an unresolved trial before
accepting coincidence or a rapidly collapsing scale. Invalid midpoint
predictions are likewise rejected. No physical softening or cloud fit is used.
The robustness sweep completed all 23 fractions on each case: 0.1 through 1
in 0.05 increments, plus 0.749/0.751/0.799/0.801 around the old failures.
All 46 runs completed within their 90-second per-flight deadline with no
failure flags or exit-class disagreements against the references. This is
evidence for the two captured cases, not a claim that genuinely coincident
charged input lines have a finite unsoftened force.

Temporary section timers identified the costs below at dt=0.001 us,
fraction 0.1, four workers. These are diagnostic runs, not final speed claims:

| Case | Line fields | Separate limiter | Midpoint/end plane solves | External reintegration | Events/commit/misc |
|---|---:|---:|---:|---:|---:|
| repulsion1 | 8.7% | 28.1% | 37.2% | 17.9% | 8.1% |
| mixed | 15.7% | 54.1% | 16.5% | 7.6% | 6.1% |

The initial field and limiter now share a source-ordered, target-parallel pair
pass (64-target crossover); midpoint fields also use physical target ratios.
Force/limit results are reused across retries and no-loss accepted planes,
but never across a source loss. Geometry crossings are reused within a trial.
Restoring the pre-event trial only after an actual loss reduced repulsion1
from 3,127 to 404 accepted planes in the diagnostic ablation; it avoids
growing back from a nearly zero localized step after each splat. Temporary
timers were removed before the final measurements.

#### Model difference is separate from numerical error

The following SIMION errors use our converged line references. They are
compatibility discrepancies, not estimates of error in a known private cloud
kernel:

| Case | SIMION control | Endpoint RMS/max (mm) | TOF RMS/max (us) | Exit-class disagreements |
|---|---|---:|---:|---:|
| repulsion1 | original T.Qual=-9 | 0.304992 / 1.104812 | 4.30555e-4 / 1.77517e-3 | 4 |
| repulsion1 | max dt=1.5625e-6 us | 0.306441 / 1.110769 | 4.33656e-4 / 1.78771e-3 | 4 |
| mixed | original T.Qual=-9 | 0.908870 / 3.637525 | 1.19347e-4 / 1.53363e-3 | 2 |
| mixed | max dt=1.5625e-6 us | 0.906749 / 3.668031 | 1.17967e-4 / 1.54286e-3 | 2 |

`beam_simion_refine.py` deterministically replays the recorded launches and
halves its `beam_max_dt` hook from 1e-4 to 1.5625e-6 us. The first three caps
give identical endpoints because SIMION's natural step is already smaller.
The last halving changes SIMION endpoints by RMS/max
0.000894769/0.00199857 mm in repulsion1 and 0.00864792/0.0492531 mm in mixed.
These numerical changes are reported separately from the much larger
cross-model discrepancy; RMS differences cannot be subtracted to isolate a
private-kernel error. The missing manual appendix prevents attributing that
residual uniquely to cloud overlap, density projection, or other private
choices. Our infinite parallel-line kernel, current normalization and loss
rules are explicit in `docs/physics-space-charge.md` and were not fitted.

The persistent SIMION exit disagreements are #29/#38/#76/#92 in repulsion1
and #10/#98 in mixed. A common terminal follower plane can trail the leader
by a few micrometres; classification uses that shared plane, not exact
equality with the leader coordinate. This prevents plane ordering and the
CSV distinction between `outside` at x=10 and `beam_leader_stopped` from
being mistaken for different physical exit classes.

#### Matched-precision compute verdict and defaults

Defaults here mean numerical controls: no native `--max-step-us`,
`--tolerance`, or `--space-charge-step-fraction` override. Both programs
are explicitly nonrelativistic and use I=1e-5 A with matching launches
and bounds; the model distinction above remains. Beam's fraction is now
1.0; the synchronized ensemble retains
0.1. Automatic spatial ceilings remain four cells, with external-field
error control, pairwise encounter limits and endpoint rejection.

Other workers paused CPU/GPU-heavy jobs for the serialized final timing
window. Every entry below is a median of three fresh runs; native final
state arrays were identical across all three trials. Only `fly_s` and
SIMION's own `Fly completed ... seconds` enter the comparison. Process
start, field preparation, CSV output and reference construction are excluded.

| Case | Program/control | Endpoint RMS/max (mm) | TOF RMS/max (us) | Exit disagreements | Compute median (s) |
|---|---|---:|---:|---:|---:|
| repulsion1 | SIMION T.Qual=-9 | 0.304992 / 1.104812 | 4.30555e-4 / 1.77517e-3 | 4 | 0.860000 |
| repulsion1 | native numerical defaults | 0.00569009 / 0.0170960 | 8.90735e-6 / 2.60387e-5 | 0 | **0.269405** |
| repulsion1 | native dt=1.25e-5, fraction=1 | 1.85419e-6 / 5.66707e-6 | 2.88725e-9 / 8.58769e-9 | 0 | 1.418918 |
| mixed | SIMION T.Qual=-9 | 0.908870 / 3.637525 | 1.19347e-4 / 1.53363e-3 | 2 | 4.210000 |
| mixed | native numerical defaults | 0.408970 / 2.402977 | 6.01615e-6 / 8.52936e-5 | 0 | **2.116568** |
| mixed | native dt=1e-6, fraction=0.05 | 0.0627212 / 0.616594 | 3.68191e-8 / 5.22001e-7 | 0 | 44.478372 |

Raw native compute samples, in seconds:

* repulsion1 defaults: 0.276513, 0.269405, 0.263249;
  tighter context: 1.440442, 1.384335, 1.418918.
* mixed defaults: 2.105214, 2.171898, 2.116568;
  tighter context: 44.478372, 44.324233, 45.870279.

SIMION's inactive max-dt=1e-4 callback gave samples
0.85/0.87/0.86 s and 4.21/4.21/4.15 s. A separate callback-free baseline
(`beam_simion_refine.py --steps 0`) gave 0.91/0.88/0.83 s and
5.33/5.39/4.38 s, with medians 0.88/5.33 s. Endpoints were identical
between controls. The speed ratios deliberately use the **faster** measured
SIMION medians, 0.86/4.21 s, rather than inflate the comparison with the
slower callback-free control or historical contended timings.

The cheapest measured successful controls are the automatic beam defaults
with fraction 1.0. A 23-value fraction sweep on each case verifies that the
choice is not a workaround around the old 0.75/0.8 failures. The tighter
contexts cost more than the baseline SIMION runs; no universal speed win
at every precision is claimed, and the native mixed context is not itself
fully endpoint-converged. The independent references are converged, and
each native/SIMION error is measured against those same references.

Reproduce using the captured launches and archived independent references:

```sh
ROOT=/mnt/c/Users/lsu10/simrep-oracle/beam-convergence
CASE=repulsion1
OPENBLAS_NUM_THREADS=1 python3 tools/simion-oracle/beam_reference.py \
  --case /mnt/c/Users/lsu10/simrep-oracle/beam-endpoints/$CASE \
  --reference-cache "$ROOT/robust/$CASE" \
  --output "$ROOT/replay/$CASE" --native target/release/simrep \
  --native-steps 0 --baseline-fraction 0 --trials 3 --repeat
# For mixed: CASE=repulsion_mixed_beam; also pass:
# --reference-dx .0625 --reference-tolerance 3e-14
# Tighter contexts: --native-steps 1.25e-5 --baseline-fraction 1 (repulsion1)
# or --native-steps 1e-6 --baseline-fraction .05 (mixed).
python3 tools/simion-oracle/beam_simion_refine.py \
  --case /mnt/c/Users/lsu10/simrep-oracle/beam-endpoints/$CASE \
  --output "$ROOT/replay-simion/$CASE/trial-1" --steps 0
# Repeat into trial-2/trial-3; --steps 1e-4 reproduces the inactive-hook control.
```

`--baseline-fraction 0` and `--native-steps 0` are harness sentinels that
omit those CLI flags, not zero-valued physical controls. The artifacts under
`beam-convergence/robust`, `convergence`, `fine-mixed`, and
`timing/{native-default,native-tight,simion,simion-baseline}` retain input and
native binary SHA-256s, references, exact commands, stderr, per-trial endpoint CSVs
and raw compute samples. Independent reference caches preserve every
spacing/tolerance resolution used above.

### Earlier GPU PIC verification and Windows cleanup

`cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --
-D warnings`, `cargo clippy -p simrep-py --all-targets -- -D warnings`, and
`cargo test --workspace --release` all exited 0 on the final code. The LSP
daemon did not respond within its 30-second deadline; compiler and clippy
diagnostics were clean. No existing tests were suppressed or weakened.

The final native Windows physical-Poisson run passed all nine available
backend/geometry/precision combinations. Maximum CPU difference was
7.31860e-13 V, with 21 correction cycles. A separate real CLI smoke used
two 1 amu ions, a 17³ mesh, 1e-15 C and three synchronized steps:
`--space-charge pic --device {cpu,gpu,auto}` all exited 0. CPU/GPU final
positions agreed within 1.11023e-16 mm and velocities within 5.80049e-18 mm/us;
TOFs and termination reasons matched. Auto chose CPU for this small case and
its CSV was byte-identical to explicit CPU. All three logs reported the
100,000-ion PIC crossover. These tiny-N timings are not speed benchmarks.
This bounded smoke intentionally ends at `--duration-us 0.0006`; pass the
existing `--allow-time-limit` flag to accept that termination. Without it,
the CLI correctly writes `time_limit` records and returns exit 1.

The final `tools/win/winproc.ps1 cleanup` ran after the Windows CLI exited,
from `/mnt/c`, foreground, closed stdin, with a 60-second deadline. Its output:

```text
== process counts ==

Name           Count
----           -----
cmd.exe            5
conhost.exe       47
node.exe           4
powershell.exe     2

== conhost by parent ==

Name                              Count
----                              -----
wsl.exe                              23
wslhost.exe                           5
cmd.exe                               4
lmgrd.exe                             2
powershell.exe                        2
node_repl.exe                         2
node.exe                              1
operafea-op3solve.exe                 1
rlm.exe                               1
GoogleDriveFS.exe                     1
sunshine.exe                          1
dispatcher.exe                        1
codex.exe                             1
wlanext.exe                           1
matlab-mcp-server-windows-x64.exe     1

orphan conhost: 0
stopped orphan conhost: 0
```

### Beam follow-up verification and Windows cleanup (2026-10-24)

The final beam code passed `cargo fmt --all -- --check`,
`cargo clippy --workspace --all-targets -- -D warnings`,
`cargo clippy -p simrep-py --all-targets -- -D warnings`, and
`RAYON_NUM_THREADS=16 cargo test --workspace --release`, all exit 0.
All ten beam regressions passed, including grazing transit, localized-loss
recovery, unequal-clock angular-momentum conservation and charged rotating
plane convergence. No existing test was suppressed or weakened.
The Rust LSP could not run because `rust-analyzer` was missing from the
stable toolchain; compiler and Clippy diagnostics were clean.

Both timing batches and their cleanup commands exited 0. Every Windows
flight was foreground, bounded by a subprocess timeout, run from its
C-drive working copy with closed stdin. After the last callback-free
SIMION run, the final `tools/win/winproc.ps1 cleanup` ran from `/mnt/c`,
foreground, with stdin closed and a 40-second deadline. Its output was:

```text
== process counts ==

Name           Count
----           -----
cmd.exe            4
conhost.exe       38
node.exe           4
powershell.exe     1

== conhost by parent ==

Name                              Count
----                              -----
wsl.exe                              15
wslhost.exe                           5
cmd.exe                               4
node_repl.exe                         2
lmgrd.exe                             2
node.exe                              1
powershell.exe                        1
matlab-mcp-core-server-win64.exe      1
rlm.exe                               1
GoogleDriveFS.exe                     1
sunshine.exe                          1
dispatcher.exe                        1
codex.exe                             1
wlanext.exe                           1
matlab-mcp-server-windows-x64.exe     1

orphan conhost: 0
stopped orphan conhost: 0
BEAM_BASELINE_EXIT=0 CLEANUP_EXIT=0
```

# Geometry/physics-aware flight defaults

## Main integration: all-corner collision defaults

The selected main policy is **four cells / 1e-8 maximum automatic position
and relative tolerance**, with the launch-speed-scaled velocity budget below.
Both candidate policies were rebuilt and measured in main with its all-corner
occupied-cell collision default, extent-only field queries, vacuum mirror
constraints, and continuous-surface support retained. No nearest-node collision
flag, explicit step ceiling, or explicit tolerance was supplied.

These are five fresh serial processes per row, release CPU f64 with 16 Rayon
workers; only `fly_s` is timed. All 10,000 ions splatted in each run.

| Cells / tolerance cap | 10k median s | Accepted | Rejected | Einzel subset RMS mm | Maximum change from prior defaults mm |
|---|---:|---:|---:|---:|---:|
| **4 / 1e-8, selected** | **0.184954** | 1,141,574 | 250,214 | 0.226123540 | 0.003005085 |
| 8 / 2e-8, rejected | 0.145724 | 812,720 | 225,542 | 0.226122557 | 0.005876707 |

The four-cell times were 0.177004, 0.184954, 0.200907, 0.184458 and
0.192876 s; the eight-cell times were 0.145724, 0.185495, 0.138636,
0.160880 and 0.135234 s. Both meet the 0.39 s gate. Four cells retain
the documented 0.003210 mm maximum endpoint-change margin; eight cells with
2e-8 do not. Four-cell maximum reference error improves from the preceding
default's 1.323463343 to 1.323331557 mm; eight-cell maximum error rises to
1.329340050 mm. The selected einzel RMS increase is 0.1843%, below the
previously documented 0.187% margin, and remains far below SIMION's 1.106314 mm.

All cases below were run with `bench/default_flight.py`, five serial repeats.
Errors are RMS against its retained independent references.

| Case | Prior-default RMS mm | 4 / 1e-8 RMS mm | 8 / 2e-8 RMS mm | SIMION RMS mm |
|---|---:|---:|---:|---:|
| einzel subset | 0.225707623 | 0.226123540 | 0.226122557 | 1.106314 |
| TOF | 6.88958514e-8 | 6.98024406e-8 | 6.89018328e-8 | 9.49303809e-8 |
| cosine RF | 1.21089841e-12 | 1.21089841e-12 | 1.21089841e-12 | 1.23449634e-6 |
| square RF | 1.73137168e-12 | 1.73083298e-10 | 3.66458659e-10 | 1.21905900e-6 |

The selected TOF RMS increase is 9.07e-10 mm, below the documented
2.09e-9 mm absolute margin; cosine endpoints are unchanged and square RF
remains below the documented 3.67e-10 mm margin. Four-cell TOF RMS errors
are 0.0109990036 us (einzel), 4.46449515e-10 us (TOF), and zero for RF.
The magnetic CLI runner still identifies stale input files for sector 2dc
and solenoid; the matched captured-START API gate is authoritative.

The matched sector-3dp API gate passes for both policies:

| Cells / cap | Compute median s | Accepted / rejected | Maximum position error mm | Maximum TOF error us |
|---|---:|---:|---:|---:|
| 4 / 1e-8 | 0.0080359 | 9,705 / 2,021 | 7.90769620e-8 | 3.70318958e-8 |
| 8 / 2e-8 | 0.0073068 | 8,347 / 2,150 | 9.73155900e-8 | 4.80432618e-8 |

Both beat SIMION's 0.04 s, 0.107902141 mm and 0.00692312618 us.
Both also pass the old API's maximum-error noninferiority assertions.
The reference's eightfold refinement drift remains 8.54001e-9 mm and
2.14289e-8 us. The selected main CLI sector-3dp median is 0.006276 s.
The former 0.339 s worker timing is not the integrated-main timing: main's
newer field-query and collision implementation is retained, not overwritten.

Commands, all five timing/step logs, CSVs and JSON are retained in
`target/default-flight/integration-4` and `integration-8`. The worker captures
below are historical and do not override this main-policy selection.

The final selected-policy matched gate also passed all seven magnetic cases,
including the sector-2dc and solenoid cases whose saved CLI inputs are stale.
Its sector-3dp median was 0.0083044 s with the same errors and step counts as
the four-cell row above. HS1 and SDS each preserve all original output-column
values for 4096 ions; both direct space-charge modes preserve them for 103
ions. Main's independently added electrode-number CSV column is retained.
The HS1/SDS SIMION KS p-values remain exactly those in the retained table.
Factor=1 and Coulombic endpoint RMS remain 3.81460407e-7 and
1.01389188e-4 mm, respectively.

Final selected-policy verification:

```text
cargo fmt --all -- --check                                exit 0
cargo clippy --workspace --all-targets -- -D warnings      exit 0
cargo clippy -p simrep-py --all-targets -- -D warnings      exit 0
cargo test --workspace --release                         exit 0
185 passed; 0 failed; 0 ignored (43 suite summaries)
Matched magnetic opt-in gate: 1 passed; 0 failed (all seven cases)
```

Logs: `/tmp/simrep-step-integration-validation.log` and
`/tmp/simrep-step-integration-accuracy.log`. LSP diagnostics timed out;
the requested compiler and clippy checks completed cleanly.
The exact requested `/tmp/e10k.csv` command was also rerun after the final
build: 10,000 splats, 1,141,574 accepted / 250,214 rejected steps,
`fly_s=0.163828` s (a single confirmation run, not another five-run median).
Cargo.lock was not edited, no commit was created, and no Windows executable
was launched.

2026-10-23, Ryzen 9 5950X, WSL2 release CPU f64, 16 Rayon workers.
Speed means integration compute only. Preparation, reconstruction, loading,
output and process wall time are excluded. SIMION compute is its retained
`Fly completed` interval, not its process wall time. Short SIMION intervals
have 0.01 s resolution.

## Policy and tolerance selection

The CLI previously used 0.1 us / 1e-8; the API already used 0.1 us / 1e-10.
Both now use **automatic step ceilings and field-aware tolerances**. A trial is
limited by four smallest physical active-axis grid cells of estimated travel,
0.2 radians of local cyclotron rotation, and 0.2 radians of the fastest
recognized voltage phase. Grid placement scale is included. The grid ceiling
is retained in field-free workbench gaps. Whole accepted segments are tested
for electrode intersections, so a longer step cannot skip a thin electrode.
Local electric acceleration limits launches from
rest. Relativistic cyclotron frequency includes gamma.

The public `FlightOptions.step_limits: StepLimits` exposes these three values.
`max_step_us = f64::MAX` means automatic; any other positive finite API value,
or an explicit CLI `--max-step-us`, selects that ceiling instead. Explicit
ceilings retain the requested numeric tolerances and bypass the automatic
tolerance estimate, including for converged-reference runs. RK45 error control,
known voltage edges, collision optical depth,
space-charge closest approaches, PIC restrictions and particle births still
shorten steps. Boris retains its fixed requested initial step. Coupled space
charge retains an additional one-cell ceiling: its second-order push does not
estimate Coulomb force error with RK45. Removing that separate ceiling raised
the strong-charge RMS from 1.01389e-4 to 1.00721e-3 mm and was rejected.

Numeric tolerances remain 1e-12. For a static electric grid, the default
`automatic_tolerance` estimates a launch budget from five E samples along the
initial velocity, separated by physical grid cells:
`clamp(1e-3 * |E0-4E1+6E2-4E3+E4| / max|Ei|, 1e-12, 1e-8)`.
The fourth difference vanishes for cubic potentials. Missing grid scale,
launch from rest, any zero stencil field, a magnetic stencil sample, or known
voltage time dependence retains 1e-12. Explicit CLI `--tolerance`, an explicit
step ceiling, `automatic_tolerance: false`, or any nondefault numeric API
tolerance preserves the exact configured values. This is a bounded finite-grid
error estimate, not a continuum error bound or a per-workload lookup.
When the estimate exceeds 1e-12, the absolute velocity budget is
`max(tolerance, tolerance * initial_speed_mm_per_us)`. Position and relative
budgets stay at the estimated value. This uses the launch speed to scale a
velocity error instead of over-resolving tiny transverse components.
A 3e-8 budget trial reduced standalone API compute to 0.419667 s, but raised
einzel RMS to 0.227006143 mm (+0.575%); it was rejected and the 1e-8 budget
retained.

RK45 reuses its endpoint derivative and field sample only at exactly matching
state and absolute time. Rejected trials and event probes share the initial
sample; collision kicks and voltage-edge left limits invalidate endpoint reuse.
RK endpoint times use the same compensated sum as the next accepted state,
avoiding false cache misses caused by different floating-point associations.
The error norm reuses physical velocities already present in the derivatives.
Spatial and voltage-phase metadata are cached once per flight. The next known
voltage edge is refreshed only when reached, and the same cached edge controls
RK endpoint left-limit samples. Rejected trials and event probes do not repeat
these metadata queries.
Event bisection stops at the absolute position tolerance divided by the largest
initial, final, or chord speed, with the existing 48-iteration upper bound.
Before bisection, chord intersections propose a few re-integrated event times
and a nearby opposite bound. Every bound is verified against the original
step segment; unsuccessful proposals leave a valid bracket for bisection.
The event is still re-integrated, rather than interpolating its velocity or TOF.
Event probes omit the unused embedded error estimate and retain their verified
high-bound state, rather than integrating the final high bound twice.

The 1e-11 candidate passed the old CLI comparison but failed old-API
sector-3dp TOF noninferiority. Against the same reference, old API maximum TOF
error was 4.95526e-8 us, while that candidate was about 1.25e-7 us.
The retained 1e-12 magnetic choice passes maximum position and TOF noninferiority against
both old settings in all seven matched magnetic cases. Tolerance was not
relaxed for magnetic cases.

Affine `sin`, `cos` and `exp` arguments in parsed voltage expressions supply
inverse angular-frequency/rate metadata. Step and pulse discontinuities use
the existing exact time-event splitting. General field closures cannot reveal
their frequency or geometry: custom `Field` implementations can supply
`spatial_scale_mm` and `time_scale_us`, or callers can set an explicit ceiling.
These local estimates complement RK45 error control, not a continuum error
bound for arbitrary fields.

## Retained worker measurements before main integration

Here "old" is the frozen executable with reconstructed 0.1 us / 1e-8 CLI
settings; "current" is the same frozen executable's one-cell / 1e-12 automatic
defaults. "New" records the worker's eight-cell field-aware trial, with shared RK
samples and tolerance-aware event localization. New times are five-run serial
medians. The old/current 10k rows are also five-run medians; their other rows
are the retained baseline captures from the same machine, not medians.

The default-only CLI 10k median is **0.338590 s**, below the 0.39 s target.
It takes 894,300 accepted / 251,042 rejected steps, versus 1,861,010 / 307,666
under the old settings and 6,821,988 / 2,471,620 under the
one-cell defaults. Preparation, input, reconstruction and CSV output are not
included in any native compute clock.

| Case | Old compute s | Current compute s | New compute s | SIMION compute s |
|---|---:|---:|---:|---:|
| einzel, preserved 10k beam | 0.494011 | 1.985470 | **0.338590** | not remeasured |
| einzel, preserved 1k reference subset | 0.048953 | 0.195407 | 0.030154 | 10.39 |
| TOF, four templates | 0.0008483 | 0.0028715 | 0.0009414 | 0.12 |
| cosine RF | 0.0005741 | 0.0010921 | 0.0009189 | 0.12 |
| square RF | 0.0006940 | 0.0006579 | 0.0005562 | 0.07 |
| magnet1 | 0.001482 | 0.004427 | 0.003140 | 0.01 |
| mag90 | 0.001420 | 0.006472 | 0.003928 | 0.01 |
| sector 2dp | 0.001910 | 0.012848 | 0.003798 | 0.05 |
| sector 2dc | 0.002471 | 0.005279 | 0.003516 | 0.04 |
| sector 3dp | 0.028854 | 0.010708 | 0.007751 | 0.04 |
| Helmholtz | 0.002249 | 0.004702 | 0.004267 | 0.12 |
| Solenoid | 0.245272 | 1.774387 | 1.315470 | 7.01 |
| direct factor=1, 103 ions | 0.257942 | 0.281273 | 0.248939 | 0.91 |
| direct Coulombic, 103 ions | 0.247949 | 0.267668 | 0.238599 | 0.93 |
| HS1, 4096 ions | 0.186143 | 0.173213 | 0.146168 | 9.96 |
| SDS, 4096 ions | 0.246547 | 0.004870 | 0.005330 | 1.23 |

The 10k row does not substitute a SIMION process wall interval for compute.
The retained SIMION interval for the actual 1k reference subset remains the
accuracy-and-compute comparison. The magnetic CLI input files retain the
previously documented stale-input caveat; the matched API gate below is
authoritative for magnetic accuracy and compute.

Endpoint RMS against the same independent references:

| Case | Old RMS mm | Current RMS mm | New RMS mm | SIMION RMS mm | New TOF RMS us |
|---|---:|---:|---:|---:|---:|
| einzel subset | 0.225708628 | 0.225707623 | 0.226129269 | 1.106314 | 0.0109993215 |
| TOF | 1.84088849e-5 | 6.88958514e-8 | 7.09839989e-8 | 9.49303809e-8 | 2.64902059e-10 |
| cosine | 3.93412755e-10 | 1.21089841e-12 | 1.21089841e-12 | 1.23449634e-6 | 0 |
| square | 5.49961385e-11 | 1.73137168e-12 | 3.66458659e-10 | 1.21905900e-6 | 0 |
| factor=1 | 4.10728720e-7 | 3.81460407e-7 | 3.81460407e-7 | 0.010609200 | 3.46066567e-9 |
| Coulombic | 1.47292739e-3 | 1.01389188e-4 | 1.01389188e-4 | 0.590059409 | 1.48165111e-7 |

The einzel RMS increase is 0.187% (0.422 micrometers on a 225.7 micrometer
finite-grid error); its maximum error improves from 1.323463343 to
1.323331557 mm. TOF RMS increases by 0.166%. New-versus-current endpoints differ
by 0.635 micrometers RMS and at most 3.210 micrometers. The TOF-workbench RMS
increase is only 2.09e-9 mm (2.09 picometers), and square-RF RMS remains below
0.367 picometers, versus SIMION's 1.219 nanometers. These are the measured
negligible margins, rather than a claim of bitwise noninferiority.

Cosine, factor=1, Coulombic, HS1 and SDS CSVs are byte-identical to current
defaults. Consequently the retained HS1/SDS KS p-values below are unchanged:
all new-versus-SIMION comparisons pass the 5% gate. The Coulombic one-cell
restriction is necessary for this preservation.

### Matched magnetic API gate

All seven cases use captured `START` positions and velocities, freshly
recomputed 0.0025 us / 1e-12 references, 16 workers and five-run medians.
The old/current control settings below run on the optimized integrator;
they are control-policy comparisons, not the frozen-binary timings above.

| Case | Old CLI settings s | Current settings s | New default s | SIMION s |
|---|---:|---:|---:|---:|
| magnet1 | 0.0014337 | 0.0048883 | 0.0035573 | 0.01 |
| mag90 | 0.0015124 | 0.0056020 | 0.0059393 | 0.01 |
| sector 2dp | 0.0023865 | 0.0132877 | 0.0060328 | 0.05 |
| sector 2dc | 0.0031140 | 0.0063568 | 0.0052999 | 0.04 |
| sector 3dp | 0.0285007 | 0.0141715 | 0.0070086 | 0.04 |
| Helmholtz | 0.0011208 | 0.0031370 | 0.0032375 | 0.12 |
| Solenoid | 0.2332241 | 1.3658283 | 1.2021245 | 7.01 |

| Case | Current max position mm | New max position mm | SIMION max position mm | New max TOF us | SIMION max TOF us |
|---|---:|---:|---:|---:|---:|
| magnet1 | 9.72737e-9 | 1.14169e-8 | 4.65061e-2 | 4.65116e-9 | 3.69699e-3 |
| mag90 | 5.38804e-9 | 6.85245e-9 | 3.15272e-2 | 6.28724e-11 | 1.28151e-4 |
| sector 2dp | 2.43045e-9 | 4.91977e-9 | 9.14382e-3 | 7.82592e-11 | 2.90031e-4 |
| sector 2dc | 3.10394e-9 | 3.28059e-9 | 2.08654e-3 | 3.11928e-11 | 8.53504e-5 |
| sector 3dp | 8.30480e-8 | 9.73156e-8 | 1.07902e-1 | 4.80062e-8 | 6.92313e-3 |
| Helmholtz | 1.46264e-13 | 1.46264e-13 | 2.12810e-1 | 0 | 6.47337e-5 |
| Solenoid | 2.36185e-9 | 1.84348e-9 | 2.33651 | 2.33458e-12 | 2.23144e-3 |

The existing old-API maximum position/TOF noninferiority assertions all pass.
The largest increase over current maximum position error is 1.43e-8 mm,
in sector 3dp. Its reference's additional eightfold refinement changes
endpoints by 8.54e-9 mm and TOF by 2.14e-8 us. The new maximum TOF increase
over current is only 1.89e-9 us, below that reference's measured TOF drift.

### Separate standalone API timing diagnostic

The standalone API benchmark and CLI produce exactly 894,300 accepted /
251,042 rejected steps on the same 10k beam. Alternating five fresh API and
CLI processes, with field preparation outside both clocks, measured
**0.459012297 s API versus 0.339495 s CLI**. The separate API executable
therefore does not meet a strict independent 0.39 s API gate. The CLI target,
all workload accuracy gates, and the matched magnetic API gates pass; this
remaining generated-code/call-context timing difference is recorded rather
than hidden by substituting CLI time for the standalone API measurement.

The final serial API spatial sweep uses the same automatic tolerance policy,
five fresh processes per candidate and 16 workers. "Unbounded" is
`max_grid_cells = f64::MAX`; cyclotron and voltage guards remain active.

| Grid-cell ceiling | API compute median s | Accepted | Rejected |
|---|---:|---:|---:|
| 1 | 0.692541999 | 3,035,102 | 143,844 |
| 2 | 0.472699100 | 1,687,294 | 203,010 |
| 4 | 0.472871600 | 1,141,574 | 250,214 |
| 8 | 0.471424600 | 894,300 | 251,042 |
| unbounded | 0.759620302 | 674,622 | 251,042 |

Two through eight cells are nearly tied in this standalone executable; eight
uses the fewest bounded accepted steps. Removing the spatial ceiling entirely
reduces steps but increases compute. The main integration selection above
supersedes this worker-only sweep.

## Retained one-cell baseline: preserved physical workloads

The tables below retain the preceding one-cell / 1e-12 increment. Their
"New" columns are the current-default baseline for the optimization reported
above, not the optimized defaults.

These runs supply no step or tolerance flags. The RF example's `default`
selector constructs `FlightOptions::default()`; physical durations, placements
and particle inputs remain those of the existing benchmarks. Old RF settings
are reconstructed through its formerly required positional arguments.

Endpoint error is Cartesian RMS against the existing independent 8x einzel,
8x TOF, RF quarter-step, or event-localized DOP853 Coulomb reference.

| Case | Old CLI compute s | New compute s | SIMION compute s | Old endpoint RMS mm | New endpoint RMS mm | SIMION endpoint RMS mm | New TOF RMS us |
|---|---:|---:|---:|---:|---:|---:|---:|
| einzel, preserved 1k subset | 0.030878 | 0.223796 | 10.39 | 0.225708628 | 0.225707623 | 1.106314 | 0.0109811081 |
| TOF, four templates | 0.0010754 | 0.0029872 | 0.12 | 1.84089e-5 | 6.88959e-8 | 9.49304e-8 | 2.63312e-10 |
| cosine | 0.0006763 | 0.0009842 | 0.12 | 3.93413e-10 | 1.21090e-12 | 1.23450e-6 | 0 |
| square | 0.0007661 | 0.0008078 | 0.07 | 5.49961e-11 | 1.73137e-12 | 1.21906e-6 | 0 |
| direct factor=1, 103 ions | 0.219089 | 0.279076 | 0.91 | 4.10729e-7 | 3.81460e-7 | 0.010609200 | 3.46067e-9 |
| direct Coulombic, 103 ions | 0.231766 | 0.248803 | 0.93 | 1.47293e-3 | 1.01389e-4 | 0.590059409 | 1.48165e-7 |

The einzel SIMION interval is measured for the actual preserved subset in
`_real_einzel/subset-tq209.txt`, not extrapolated from another beam.
Its TOF RMS is 0.055319 us; TOF-workbench SIMION TOF RMS is 3.30900e-7 us.
RF duration is prescribed. The strong-charge default is compared with the
old **default**, not the historically manually tuned 1e-4 us run; that finer
configured run remains more accurate than either default.

For the older API's tighter 1e-10 tolerance, the corresponding endpoint RMS
values are 0.225708551 mm (einzel), 1.95142e-7 mm (TOF),
9.73221e-11 mm (cosine), and 5.49961e-11 mm (square).
The new defaults improve each RMS. The field-free space-charge external
tolerance does not change the old API's result relative to the old CLI.
The finite-grid einzel error dominates its integration error; its maximum
endpoint error is 1.323463343 mm and maximum TOF error is 0.066291936 us.

## Magnetic defaults from matched captured initial states

`magnetic_performance` reads the current `START` positions and **velocities**
directly, constructs fields and Rayon pools before timing, and takes the median
of five flights. All rows use the same current captured initial states and
their recomputed 0.0025 us / 1e-12 native reference.

This matters: some saved `ions.ion` / `native-reference.csv` files predate
subsequent random SIMION launches in `flight.log`. Comparing those old ions
with the new SIMION finals would produce meaningless large errors. The CLI
runner checks directions/positions before reporting such a SIMION accuracy
comparison; the matched test below is authoritative for all seven cases.

| Case | Old CLI compute s | Old API compute s | New default compute s | SIMION compute s |
|---|---:|---:|---:|---:|
| magnet1 | 0.0016997 | 0.0021879 | 0.0060987 | 0.01 |
| mag90 | 0.0016170 | 0.0036667 | 0.0077360 | 0.01 |
| sector 2dp | 0.0026247 | 0.0042478 | 0.0146786 | 0.05 |
| sector 2dc | 0.0034193 | 0.0047083 | 0.0070629 | 0.04 |
| sector 3dp | 0.0324118 | 0.0311279 | 0.0138264 | 0.04 |
| Helmholtz | 0.0010447 | 0.0017716 | 0.0037697 | 0.12 |
| Solenoid | 0.2844011 | 1.1531514 | 3.7085829 | 7.01 |

Errors below are maxima, not RMS. The optional benchmark asserts new position
and TOF errors are no larger than old API errors and smaller than SIMION errors.

| Case | Old CLI position mm | Old API position mm | New position mm | SIMION position mm | Old API TOF us | New TOF us | SIMION TOF us |
|---|---:|---:|---:|---:|---:|---:|---:|
| magnet1 | 1.78409e-5 | 6.11053e-7 | 9.72738e-9 | 4.65061e-2 | 4.01298e-7 | 9.66864e-9 | 3.69699e-3 |
| mag90 | 2.53992e-5 | 5.13900e-7 | 5.38825e-9 | 3.15272e-2 | 4.14453e-9 | 4.48654e-11 | 1.28151e-4 |
| sector 2dp | 1.76679e-5 | 2.88942e-7 | 2.43044e-9 | 9.14382e-3 | 7.11680e-9 | 4.77662e-11 | 2.90031e-4 |
| sector 2dc | 2.34229e-5 | 3.01237e-7 | 3.10395e-9 | 2.08654e-3 | 3.69089e-9 | 2.91269e-11 | 8.53504e-5 |
| sector 3dp | 2.21069e-5 | 9.77063e-7 | 8.30480e-8 | 1.07902e-1 | 4.95526e-8 | 4.61196e-8 | 6.92313e-3 |
| Helmholtz | 3.12328e-7 | 1.46869e-8 | 1.46264e-13 | 2.12810e-1 | 2.72217e-11 | 0 | 6.47337e-5 |
| Solenoid | 2.04132e-4 | 1.22616e-6 | 2.36183e-9 | 2.33651 | 1.30390e-9 | 3.23447e-12 | 2.23144e-3 |

Sector-3dp accepted/rejected totals change from 87,819/0 (old CLI) or
87,865/53 (old API) to **23,315/1,376**. Its new default is 2.89x faster
than the measured SIMION compute. The reference's existing additional
eightfold refinement changes endpoints by at most 8.53998e-9 mm and TOF by
2.14289e-8 us; the quoted differences are against a numerical reference,
not an exact continuum trajectory.

## HS1 and SDS statistical checks

Both models retain 4096 ions, seed 813 and 50 us duration. No timestep,
tolerance or initial-step option is passed. HS1 still obeys the 20-subdivision
optical-depth ceiling. SDS can use its analytic damped update over long
field-free intervals instead of taking 500 fixed-cap steps.

| Model | Old default compute s | New compute s | SIMION compute s |
|---|---:|---:|---:|
| HS1 | 0.171172 | 0.183027 | 9.96 |
| SDS | 0.265300 | 0.004897 | 1.23 |

Two-sample exact KS p-values use the retained SIMION samples and the old native
default sample separately. TOF is quantized only below 1e-7 us, as in the
existing benchmark; TOF and zero-field SDS KE are degenerate atoms.

| Model / quantity | New versus SIMION p | New versus old default p |
|---|---:|---:|
| HS1 X | 0.515856 | 1 |
| HS1 Y | 0.384564 | 1 |
| HS1 Z | 0.977716 | 1 |
| HS1 TOF | 1 | 1 |
| HS1 KE | 0.384564 | 1 |
| SDS X | 0.355182 | 0.0885325 |
| SDS Y | 0.625039 | 0.182615 |
| SDS Z | 0.789186 | 0.588023 |
| SDS TOF | 1 | 1 |
| SDS KE | 1 | 1 |

None of the new comparisons rejects at 5%. The old CLI default's SDS Y sample
had p=0.0145511 versus this retained oracle; that pre-existing sample result
was not removed or rerun with another seed.

## Reproduction and integration boundary

```sh
cargo build --release -p simrep-cli --examples --bin simrep
python3 bench/default_flight.py target/release/simrep \
  target/default-flight/new --rf-example target/release/examples/rf_tof --repeats 5
python3 bench/default_flight.py target/release/simrep \
  target/default-flight/new --only einzel --einzel-10k --repeats 5
python3 bench/default_flight_sweep.py target/default-flight/sweep.log \
  --quick --cli-field
python3 bench/default_flight_sweep.py target/default-flight/api-cli.log \
  --defaults-only --cli-field --paired-cli target/release/simrep
SIMREP_DEFAULTS_ONLY=1 \
SIMREP_PHYSICS_ORACLE=/mnt/c/Users/<user>/simrep-oracle/physics-validation-20261023 \
  cargo test --release -p simrep-fly --test magnetic_performance -- --nocapture
```

The CLI runner preserves full commands, CSVs, stderr timing/step counts, and
JSON results. `--before DIRECTORY` compares saved captures and collision
distributions, including new-versus-current endpoint deltas; `--reuse` only
re-analyzes them. `--legacy` reconstructs the old CLI's explicit 0.1 us / 1e-8
settings on either executable. `--legacy --api-baseline` uses the old API's
1e-10 tolerance instead. Preserve the pre-change executable and RF example
before rebuilding when reproducing the frozen-binary before/current columns.
The API sweep builds a standalone executable against Cargo-reported production
release artifacts, then runs five serial processes per candidate. Omitting
`--quick` also sweeps 1e-12, 1e-10, 1e-8, 1e-6 and 1e-4 numeric tolerances.
Independent Coulomb
reference CSVs are the cached artifacts of `bench/space_charge_reference.py`.

No `src/gpu/` file was changed for this CPU increment. The GPU implementation
must consume the additive `StepLimits` values and distinguish the
`max_step_us == f64::MAX` automatic sentinel before f32 serialization.
This page establishes CPU/default-small-beam results, not a fresh Windows
large-beam GPU measurement; those remain a separate integration gate.

The self-contained regressions cover placement/inactive-axis transit scales,
acceleration from rest, cyclotron frequency, long slow drift, an entire thin
electrode crossed in a single explicit step, narrow voltage pulses, and an RF
whole-cycle alias where all seven unconstrained RK stages see the same phase.
No SIMION process was launched for these native reruns. Final foreground
`winproc.ps1 cleanup`, from `/mnt/c` with closed stdin and a 30 s timeout,
reported `orphan conhost: 0`, `stopped orphan conhost: 0`, and no `simion.exe`.

`cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --
-D warnings`, and `cargo test --workspace --release` all exited 0 after the
final tolerance change. The opt-in matched magnetic default gate also exited 0.
LSP requests timed out without diagnostics; cargo/clippy provide the successful
compiler checks. This does not replace the separate GPU worker's native
Windows default-policy measurements.

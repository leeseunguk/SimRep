# Real SIMION-array flight comparison

Date: 2026-10-23. Ryzen 9 5950X, 32 logical CPUs, WSL2 Rust 1.95.0,
release CPU f64 RK45 + Rayon. SIMION is the native Windows
2024-09-26-8.2.1.0 executable. This comparison does not use our refine output
for its matched-input benchmark.

Current CLI/API defaults use geometry/physics-aware step ceilings and 1e-12
tolerances. The preserved 1k subset now measures 0.225707623 mm endpoint RMS
and 0.223796 s compute, versus SIMION's 1.106314 mm and measured 10.39 s.
See `bench/default-flight.md` for before/after default-only results; fixed-step
settings in the historical studies below are retained as historical settings.

**Boundary-aware reconstruction now passes the coarse-grid comparison:** endpoint
RMS is **0.225707 mm** against the same independent 8x reference, versus
5.895332 mm for the old global spline and 1.106314 mm for SIMION T.Qual 209.
The release 10k/100k totals are **0.382090 / 3.810812 s**, 1.053x / 1.118x
the old totals and within the 1.3x speed budget. The historical diagnosis and
baseline below are retained; see "Boundary-aware correction results" for the
new reconstruction and reproduction commands.

The later quadrupole study below corrects a separate collision convention:
staircase PAs now use fully occupied node cells in both 2D and 3D. Historical
nearest-node results remain labeled as such. The new convention leaves every
record of the original 100k einzel run unchanged.

## Input and placement

The benchmark loads the existing SIMION-refined
`C:\Users\lsu10\simrep-oracle\_benchfly\einzel.pa0` and `.pa1` through `.pa3`.
The arrays have 91 axial x nodes, 20 radial y nodes, cylindrical symmetry,
1 mm spacing and reference voltage 10000 V. Explicit electrode voltages are
`1=0,2=110,3=0`. Fast adjustment adds `(requested-current)/reference` times each
solution to PA0, preserving unmodified voltages and background.

Particles are the original `einzel_10000.ion` / `einzel_100000.ion`, not a
synthetic source: 100 amu, +1 e, 200 eV, x=z=0, initial velocity along +x,
uniformly spaced initial y from -15 to +15 mm. Relativistic dynamics are on.

No binary IOB layout is specified in the available supplemental documentation.
The CLI therefore requires explicit placement. The documented SIMION
`simion.wb:load`, `simion.wb.bounds`, PA-instance properties and transformation
methods independently confirmed these original IOB settings:

* One electrostatic PA; x=y=z=ox=oy=oz=az=el=rt=0, scale=1.
* Workbench bounds `[-101,-19,-19]` to `[291,19,19]` mm.
* PA unit axes are the workbench unit axes.
* Outside the PA, the field is zero; particles continue to the workbench wall.

The 90 mm PA exit is consequently not the final detector. The CLI defaults to
terminating at the PA edge unless `--bounds` is supplied. `--origin`, `--axes`
(three PA direction columns), and `--scale` support explicit general placement.
Nonzero SIMION working origins must be incorporated in `--origin`. Multiple PA
instances, magnetic arrays, Lua dynamics and binary legacy FLY/IOB parsing are
not implemented by this command.

The supported FLY2 subset is literal `particles { coordinates=0,
standard_beam { n=..., tob=..., mass=..., charge=..., ke=...,
position=vector(...), direction=vector(...) } }`. Multiple beams, numeric
scientific notation, line comments and optional numeric CWF/color are accepted.
Sequences, distributions, expressions, arbitrary Lua, other coordinate modes
and unknown fields are rejected. A literal FLY2 axis particle was also flown
through the real PA to x=291 mm.

## Matched physical-workload timing

Commands were run serially, without other benchmarks running:

```sh
cargo build --release -p simrep-cli
P=/mnt/c/Users/lsu10/simrep-oracle/_benchfly
target/release/simrep fly --device cpu --pa "$P/einzel.pa0" \
  --particles "$P/einzel_10000.ion" --voltage 1=0,2=110,3=0 \
  --bounds=-101,-19,-19,291,19,19 --output target/fly-real/native-10000.csv
target/release/simrep fly --device cpu --pa "$P/einzel.pa0" \
  --particles "$P/einzel_100000.ion" --voltage 1=0,2=110,3=0 \
  --bounds=-101,-19,-19,291,19,19 --output target/fly-real/native-100000.csv
```

Tolerances are 1e-8 absolute mm, absolute mm/us and relative; initial step
0.001 us, maximum step 0.1 us. This baseline used global interpolating cubic
B-splines through metal; it remains available with `--global-cubic`.

| ions | setup s | flight s | total s, including CSV | SIMION wall s | wall ratio | splats |
|---:|---:|---:|---:|---:|---:|---:|
| 10,000 | 0.024708 | 0.333597 | 0.363012 | 2.201 | 6.063x | 10,000 |
| 100,000 | 0.084027 | 3.277596 | 3.410063 | 21.504 | 6.306x | 100,000 |

10k accepted/rejected steps: 1,856,300 / 306,402. 100k:
18,559,032 / 3,059,262. Flight throughput is 29,976 / 30,510 ions/s.
The baseline wall times are from `bench/simion-baseline.md`, including process
startup. Our total includes parsing, PA loading, spline setup and terminal CSV
output, but excludes process startup. SIMION retained trajectory files in the
baseline; this CLI retains terminal states only. The physical particles,
geometry and voltages match, but integrators, interpolation and retained output
do not. These numbers establish neither a 10x speedup nor accuracy parity.

## Per-ion oracle measurements

Existing `einzel/oracle.json` and `_benchfly/oracle.json` contain 100k splat
status lines but no TOF. Status positions are coarsely formatted. The two
directories also contain separately refined, byte-different PAs, so each was
compared using its own arrays, rather than treating them as interchangeable.

`tools/simion-oracle/compare_fly.py prepare` stages a separate copy. Its
`fly_reference.lua` only prints initial/final states at 17-digit precision;
`sim_segment_global=1` permits readout outside PAs. Original arrays/logs are
not overwritten. For test_plane it reproduces the original three test planes.
The readout runs disable trajectory retention; their times are not benchmarks.
The existing status logs and separate readout are not bitwise identical:
differences between their displayed and high-precision terminal coordinates
reach about 0.6 mm, within one original grid unit.

| input / comparison | ions | position norm max / RMS mm | TOF max / RMS us | KE max / RMS eV |
|---|---:|---:|---:|---:|
| _benchfly, original status stdout | 100,000 | 20.255045 / 2.853639 | unavailable | unavailable |
| _benchfly, separate full-precision final | 100,000 | 20.034206 / 2.840353 | 1.013403 / 0.143443 | 0.684205 / 0.133467 |
| einzel, original status stdout | 100,000 | 20.467791 / 2.910773 | unavailable | unavailable |
| einzel, separate full-precision final | 100,000 | 20.120123 / 2.898000 | 1.014614 / 0.146316 | 0.683578 / 0.132542 |
| test_plane, plane 3 | 11 | 0.002640 / 0.001324 | 0.000272 / 0.000121 | 0.043315 / 0.028953 |
| test_plane, final workbench splat | 11 | 0.012667 / 0.006116 | 0.001001 / 0.000676 | 0.031531 / 0.021801 |

All 11 test_plane ions cross plane 3, `x+y=80 mm`, matching the original
`num hits on test plane 3: 11`. Initial particles were extracted from the
readout, including SIMION's x=-99.999 mm initial coordinate; this avoids
guessing the binary FLY definition. The CLI's oblique plane terminates at the
same plane; a separate run continues to the workbench splat.

An 11-ion (-15,-12,...,+15 mm) check gives position max/RMS
4.071193/1.736029 mm and TOF 0.228744/0.097551 us at default SIMION T.Qual.
Changing SIMION to T.Qual 109 gives 4.112537/1.753627 mm and
0.230436/0.098273 us, so smaller SIMION steps do not remove the discrepancy.
Tightening our tolerance from 1e-8 to 1e-10 and max step from 0.1 to 0.05 us
changes this check by at most 0.001333 mm and 0.0000552 us: substantially less
than the disagreement, although not a complete bound on interpolation error.

## Classification by trajectory clearance

`einzel_clearance` samples every accepted step and computes the exact minimum
distance of its z=0 step segments to the same nearest-node electrode rectangles
used for collisions. The inner metal radius is 17.5 mm; axial voxel extents
are `[0,28.5]`, `[29.5,56.5]`, `[57.5,90]` mm. This is a diagnostic of current
staircase geometry, not an assumed continuous electrode boundary at r=18.

No ion hits an electrode in either code. All 100k terminate on workbench walls:
1,998 hit y walls in both, 95,672 hit the x wall in both, and 2,330 hit a y wall
in SIMION but the x wall in our run. The closest approach is 0.776270 mm.
The worst ion is #999 (initial y about -14.7006 mm): SIMION terminates at
`(270.957986,19,0)` mm after 14.580969 us; ours at `(290.992191,19,0)` mm
after 15.594372 us. Small angle differences are amplified by 200 mm of drift
and by which finite workbench wall is encountered first.

| initial radius mm | ions | position max / RMS mm | TOF max / RMS us |
|---|---:|---:|---:|
| 0-3 | 20,000 | 0.005224 / 0.001744 | 0.000729 / 0.000205 |
| 3-6 | 20,000 | 0.011703 / 0.005582 | 0.001230 / 0.000633 |
| 6-9 | 20,000 | 0.019558 / 0.009401 | 0.001847 / 0.001041 |
| 9-12 | 20,000 | 0.087511 / 0.037349 | 0.004094 / 0.002135 |
| 12-14 | 13,332 | 0.786195 / 0.325470 | 0.019622 / 0.007025 |
| 14-15 | 6,668 | 20.034206 / 10.989703 | 1.013403 / 0.555393 |

| minimum electrode clearance mm | ions | position max / RMS mm | TOF max / RMS us |
|---|---:|---:|---:|
| 0.5-1 | 1,666 | 18.813693 / 13.144849 | 0.960613 / 0.685198 |
| 1-2 | 5,670 | 20.034206 / 9.556852 | 1.013403 / 0.474178 |
| 2-4 | 12,166 | 0.759743 / 0.287523 | 0.016591 / 0.005849 |
| 4-8 | 23,754 | 0.087511 / 0.035220 | 0.005275 / 0.002106 |
| 8-20 | 56,744 | 0.017255 / 0.005549 | 0.001847 / 0.000681 |

The growth is localized near electrode edges, rather than a broad wrong mass,
charge, mm/us conversion, cylindrical-axis mapping, mirror, voltage, or placement
error. A C2 potential spline extending through a metal/vacuum interface imposes
smoothness where the physical normal electric field can jump. This is a
specific interpolation concern; changing solver residual or RK tolerance alone
does not address it.

## Independent grid-convergence result

**SIMION is closer to the fine-grid reference than our old coarse spline.**
The disagreement cannot be dismissed as SIMION being less accurate.

`compare_fly.py subset` selects the worst 100 endpoint errors plus 900 uniformly
spaced source indices, filling duplicate indices deterministically to exactly
1,000. `einzel_convergence` independently solves the same documented GEM boxes
at 1x/2x/4x/8x resolution, with fixed physical extents x=0..90 and r=0..19 mm,
metal r=18..19 mm and axial intervals 0..28, 30..56, 58..90 mm. The CPU solver
uses radial-x storage internally; the example explicitly transposes to/from the
PA's axial-x/radial-y layout. Flight options are relativistic RK45, 1e-11
absolute/relative tolerances and maximum step 0.05 us.

| grid | dimensions | h mm | independent max Laplacian residual V/mm² |
|---:|---:|---:|---:|
| 1x | 91 x 20 | 1 | 4.1633e-13 |
| 2x | 181 x 39 | 0.5 | 1.8758e-12 |
| 4x | 361 x 77 | 0.25 | 1.0507e-11 |
| 8x | 721 x 153 | 0.125 | 4.5475e-11 |

All 4,000 refined-grid flights terminate on workbench boundaries without an
electrode hit. The independent 1x potential agrees with the SIMION-refined
coarse node potentials to max/RMS 2.11e-4/3.40e-5 V. At seven probes the max
potential/field discrepancy between these two node arrays passed through our
same spline is 2.17e-5 V / 2.30e-6 V/mm. This checks solver geometry, radial
transposition, units and fast adjustment independently of either flight code.

| comparison, 1,000 ions | position max / RMS mm | TOF max / RMS us |
|---|---:|---:|
| our 2x vs 4x | 3.134760 / 0.300148 | 0.157853 / 0.015151 |
| our 4x vs 8x | 0.138044 / 0.049504 | 0.006911 / 0.002472 |
| our original coarse vs 8x | 17.154020 / 5.895332 | 0.867525 / 0.297908 |
| SIMION coarse, T.Qual 109 vs our 8x | 4.282817 / 1.107392 | 0.200518 / 0.055370 |
| SIMION coarse, T.Qual 209 vs our 8x | 4.281588 / 1.106314 | 0.200515 / 0.055319 |
| our 2x vs 8x | 3.272804 / 0.285175 | 0.164763 / 0.014375 |

The 8x solution is a numerical reference, not a proven continuum error bound.
Nevertheless the last refinement difference is much smaller than either
coarse error and cleanly separates the two methods. Refinement is not uniformly
monotonic per ion: 56 exit faces change between 1x/2x and two between 4x/8x.
Those are all `outside` events, so comparison must include which wall is hit.

For the same 1,000 ions, raising SIMION T.Qual 109 to 209 changes endpoints by
max/RMS 0.004012/0.001203 mm and TOF by 0.000191/0.0000556 us. This is
far smaller than the interpolation disagreement. Against our tight coarse
flight, T.Qual 209 still gives max/RMS 20.207642/6.920684 mm and
1.022882/0.350398 us.

### Field evaluation and required correction

The shipped readable `lib/python/SIMION/PA.py` (`potential_real`, `field_real`)
and equivalent C++/Perl SL libraries bilinearly/trilinearly interpolate
potentials and take centered differences at +/-0.5 gu, clamped one-sided at PA
edges. Their source explicitly excludes SIMION proper's special electrode-edge
handling. The supplemental manual documents that such handling exists but
does not specify its formula; that is in the unavailable full manual's
Computational Methods appendix. Consequently the SL implementation must not be
claimed as an exact reimplementation of SIMION's runtime.

Direct documented `pa:field_vc` probes verify that the difference is in field
evaluation, not a constant voltage, length or mass factor:

| point (x,r) mm | component | our coarse V/mm | SIMION V/mm | our 8x V/mm |
|---|---|---:|---:|---:|
| (40,0) | Ex | -1.062299 | -1.061721 | -1.064457 |
| (29,15) | Ex | -13.105568 | -12.582209 | -12.242288 |
| (30,16) | Er | -5.632503 | -8.017369 | -5.958497 |
| (57,16) | Ex | 18.999843 | 17.629981 | 17.016554 |

One local component does not determine the final focusing error; at (30,16)
our radial component is closer, while the integrated coarse trajectory is
substantially worse. The largest trajectory errors occur within the cubic
stencil's metal/vacuum boundary neighborhood, not from electrode impacts.

The following correction specification motivated the implementation reported
below: reconstruct the vacuum domain rather than applying the global C2
prefilter through flat metal interiors.

1. End a vacuum stencil at the first electrode node, using its Dirichlet
   potential but not further interior metal nodes to estimate vacuum slopes.
   For example, a radial run ending at node j can use the one-sided cubic
   boundary slope `(11*V[j]-18*V[j-1]+9*V[j-2]-2*V[j-3])/(6*h)`, rather than
   letting the constant metal values beyond j force its fitted slope toward 0.
2. Preserve a shared C2 potential reconstruction inside each connected vacuum
   region, while allowing the physical normal-field discontinuity at metal.
   A cellwise switch to unrelated one-sided polynomials would introduce field
   jumps and defeat adaptive RK45; it is not a sufficient correction.
3. Handle axial electrode-gap corners and the cylindrical axis in the same
   boundary system. Benchmark this same subset against 8x before accepting the
   stencil: a minimum coarse target is to beat SIMION's 1.106314 mm endpoint
   RMS and 0.055319 us TOF RMS, while retaining the existing analytic accuracy
   tests and measuring the original 10k/100k throughput again.

Using a 2x or 4x resolved field already reduces the error substantially, but
substituting that field into the matched coarse benchmark would change the
workload. The 6.31x coarse timing therefore is not advertised as a
precision-preserving speedup.

## Boundary-aware correction results

`GridField::new` now ends each one-dimensional vacuum run at its first
electrode node, uses that node's Dirichlet potential, and clamps a cubic spline
to one-sided vacuum slopes. It does not use deeper flat metal samples to set
those vacuum slopes. Ordinary mirrors and the cylindrical axis retain zero
normal slope; anti-mirrors retain zero second derivative. Short runs use
quadratic/linear endpoint fits.

The directional cubic solves supply shared nodal first and second derivatives.
Mixed derivatives average the directional differentiation orders because the
electrode mask makes those operators noncommuting near corners. Tensor quintic
Hermite patches then share all these derivatives, giving a C2 potential and C1
electric field across vacuum cell faces, rather than switching between
unrelated one-sided cell polynomials. The input node potentials, including
Dirichlet nodes, are interpolated exactly. Cell power coefficients are prepared
once and sampled with Horner evaluation. Electrode-free arrays retain the
original cubic reconstruction and smooth-field convergence behavior.
The nearest-node collision geometry and integrator were not changed.

`Reconstruction::GlobalCubic` and `simrep fly --global-cubic` preserve the old
method for comparisons. The boundary-aware method is the default for arrays
containing electrodes. It stores 6^d coefficients per cell in d active
dimensions, versus the original spline's padded coefficient lattice; this is
a memory/initialization cost, not a finer field solve or changed PA resolution.

### Same 1,000 ions, same independent 8x reference

The original subset and `target/fly-real/grid-8.csv` were reused unchanged.
All five new runs terminate outside the PA at a workbench wall; none hits an
electrode or reaches a numerical step/time limit. RK45 settings remain
1e-11 absolute/relative tolerance and maximum step 0.05 us.

| field vs original 8x reference | endpoint max / RMS mm | TOF max / RMS us |
|---|---:|---:|
| old coarse global spline | 17.154020 / 5.895332 | 0.867525 / 0.297908 |
| SIMION coarse, T.Qual 209 | 4.281588 / 1.106314 | 0.200515 / 0.055319 |
| boundary-aware, original SIMION coarse PA | 1.323463 / **0.225707** | 0.066292 / **0.010981** |
| boundary-aware, independently solved 1x PA | 1.323909 / 0.225838 | 0.066312 / 0.010987 |
| boundary-aware, solved 2x PA | 0.458482 / 0.146931 | 0.022944 / 0.007317 |
| boundary-aware, solved 4x PA | 0.140446 / 0.047836 | 0.007031 / 0.002387 |
| boundary-aware, solved 8x PA | 0.00003022 / 0.000002182 | 0.000001508 / 0.0000001091 |

The original coarse result meets both the <1.106314 mm minimum and <=0.5 mm
aim. It also beats SIMION's TOF RMS. Endpoint RMS is 26.1x smaller than the old
coarse reconstruction and 4.90x smaller than SIMION's result against this
reference. The independently solved 1x/2x/4x endpoint RMS ratios give observed
orders **0.620 / 1.619**; TOF orders are **0.586 / 1.616**. These near-electrode
data do not exhibit fourth-order grid convergence. The PA solve, electrode
corners and boundary derivative truncation remain part of the finite-grid
error. The separate existing smooth-field flight convergence test is unchanged.

The boundary-aware 8x result differs from the old 8x reference by only
2.18e-6 mm RMS, much less than the 4x error. This checks that the conclusion is
not an artifact of choosing the old interpolator for the numerical reference.
The reference remains a numerical convergence result, not a continuum bound.

### Matched coarse-grid speed after correction

Serial release runs used the original 10k/100k ION files, original SIMION PA0
and voltage adjustment, original workbench bounds, and unchanged default flight
tolerances. No other benchmark ran alongside them.

| ions | setup s | flight s | total s including CSV | old total s | new/old | SIMION s | SIMION/new |
|---:|---:|---:|---:|---:|---:|---:|---:|
| 10,000 | 0.017722 | 0.358728 | **0.382090** | 0.363012 | 1.053x | 2.201 | 5.76x |
| 100,000 | 0.076096 | 3.688588 | **3.810812** | 3.410063 | 1.118x | 21.504 | 5.64x |

Both totals pass the 1.3x limit (0.471916 / 4.433082 s). Accepted/rejected
steps are 1,861,010 / 307,666 and 18,610,376 / 3,079,016. All ions reach
workbench walls. As before, terminal-only native CSV output and SIMION's
retained trajectories differ, so the timing ratio is not a pure integrator
speed ratio.

Reproduce the accuracy comparison without refining or overwriting the old
reference:

```sh
cargo run --release -p simrep-cli --example einzel_convergence -- --boundary
```

This reads saved `grid-{1,2,4,8}.pa` arrays and writes
`target/fly-real/boundary-{original,1,2,4,8}.csv` and
`boundary-comparison.csv`. The original convergence invocation still performs
the complete independent solve study; do not rerun it to replace the saved
historical reference inadvertently.

Use the original speed commands above without `--global-cubic`, with output
paths `target/fly-real/boundary-native-{10000,100000}.csv`, to measure the new
default. No Windows process was launched for this correction.

## 3D quadrupole: collision geometry and spatial convergence

The reported coarse CPU/GPU RMS of 2.022 mm against an unconverged 4x PA was
mostly a collision-geometry discrepancy, not an integrator or GPU precision
error. SIMION uses occupied node cells; our former electric flyer used
half-grid nearest-node voxels. A reference that shrank those voxels during
refinement mixed different material boundaries.

The input is the app template at
`C:\Users\lsu10\Documents\SimRep\quadrupole-2\quadrupole.pa0` and bases 1..3:
121x37x37 nodes, 0.5 mm spacing, Y/Z mirrors, voltages `1=2.5,2=-2.5,3=0`.
The existing `_quad_gpu/particles.ion` has 1,000 ions, 100 amu, +1 e, 100 eV,
x=1 mm, +x velocity, and the recorded golden-angle disk of radius 3 mm.
The PA fills the `[0,60]x[-18,18]x[-18,18]` mm workbench; no external drift
volume is needed.

### Three distinct geometries

1. **Node-cell staircase:** all four corners in 2D or all eight in 3D must be
   electrode nodes. This is now the raw-PA default, including cylindrical PAs.
2. **Historical nearest-node voxels:** half a grid interval around each
   electrode node, retained explicitly by
   `GridField::with_nearest_node_collisions()` and
   `simrep fly --nearest-node-collisions`.
3. **Physical round rods:** radius 5.75 mm at transverse offset 10.75 mm,
   x=12..48 mm; grounded plates x=6..7 and 53..54 mm, with radius-4 mm
   circular apertures. `bench/quadrupole.gem` records this geometry.

The zero-voltage probe `tools/simion-oracle/quad_collision_probe.py` independently
settles the PA convention. SIMION passes the aperture at radius 4.49 mm and
splats at radius 4.5 mm, at x=6 mm. Its rod splats are |z|=5.5 mm for y=0..1 mm
and |z|=6 mm at y=2 mm. The old nearest-node rod starts at 4.75 mm, and its
plate starts at x=52.75 mm instead of 53 mm. The probe uses a separate copied
workbench and sets all electrode voltages to zero, removing electric
deflection from this measurement.

### References below the RAM limit

`quadrupole_convergence` independently solves fixed geometry with the cut-cell
operator at relative residual tolerance `1e-12`. In addition to Y/Z symmetry,
the template is symmetric about x=30 mm. Solving its half-length quadrant
reduces the full 8x problem to **481x289x289 nodes**. Fine fields use the
global cubic reconstruction, avoiding 216 coefficients or 27 derivative
components per node at these sizes. Both material geometry and collision
geometry stay fixed within each refinement ladder.

| Reference check, all 1,000 ions | Endpoint RMS mm | Changed exit classes |
|---|---:|---:|
| Physical 2x vs 4x | 0.001708267 | 0 |
| Physical 4x vs 8x | 0.000696157 | 0 |
| Fixed nearest-node voxels, 4x vs 8x | 0.019637580 | 0 |
| Fixed node cells, full 4x vs 8x | 0.191739763 | 1 |
| Fixed node cells, full 8x vs local 16x | 0.001649565 | 0 |
| Local 16x, exterior data from full 4x vs full 8x | 0.000076981 | 0 |

The node-cell grazing ion required a further check. Its local 16x grid is
**961x257x257**, covering the full axial half-length and |y|,|z|<=8 mm.
Dirichlet values on the transverse exterior faces come from the independently
solved full-domain fields. No trajectory or trial stage leaves this box.
Only vacuum nodes receive the exterior data; fixed conductor voltages remain
exact and are asserted after the solve.
At 8x, the cropped solve reproduces the full-domain endpoints to RMS/max
`2.713e-6 / 2.484e-5 mm`, with no changed exit classes. The separate 4x/8x
exterior-data control above measures sensitivity to that imposed boundary,
rather than assuming it is exact.

Peak process RSS was **2,915,244 KiB** for physical 8x, **2,924,544 KiB** for
voxel 8x, **2,878,800 KiB** for node-cell 8x, and **4,332,824 KiB** for the
local 16x controls. Physical 8x solve compute was 11.047 s; node-cell 8x was
9.192 s; local 16x with 8x exterior data was 11.618 s. These are reference
construction times, not flight-speed denominators. No Richardson
extrapolation is used: corners and grazing events do not support a consistent
order across the complete coarse ladder. The final refinement differences
are numerical-reference uncertainty checks, not proven continuum bounds.

### Corrected coarse PA result

The same input potentials and PA resolution are retained. Only the collision
convention and the independently diagnosed mirror-jet defect change.
The timing run uses the original `1e-8` tolerances and 0.1 us maximum step;
the tight control uses `1e-11` and 0.025 us.

| Coarse input vs node-cell local-16x reference | Endpoint RMS mm | TOF RMS us | Changed exit classes |
|---|---:|---:|---:|
| Historical CPU, nearest-node collisions | 2.364893 | 0.170241 | 90 |
| CPU, corrected cell collisions | **1.023402** | **0.0730118** | 22 |
| SIMION T.Qual 3 | 1.047461 | 0.0747713 | 23 |
| SIMION T.Qual 209 | 1.023425 | 0.0730333 | 22 |

The corrected coarse CPU is more accurate than T.Qual 3. Its small numerical
RMS advantage over T.Qual 209 is below reference uncertainty, so the strict
comparison establishes **matched accuracy**, not a resolved accuracy win.
The remaining coarse node-cell error is shared with SIMION; increasing
trajectory quality does not remove its spatial discretization error.

Clearance is the exact minimum on accepted straight step segments to merged
boxes of the fixed node-cell electrodes. The same new-coarse trajectories
define the ion groups for all three columns:

| Minimum cell clearance mm | Ions | Old CPU RMS mm | New CPU RMS mm | SIMION 209 RMS mm |
|---|---:|---:|---:|---:|
| <1e-8, including contacts | 150 | 3.610882 | 2.358393 | 2.358490 |
| 1e-8..0.25 | 66 | 6.923640 | 1.742163 | 1.742096 |
| 0.25..0.5 | 68 | 2.608640 | 0.185984 | 0.186152 |
| 0.5..1 | 98 | 0.162368 | 0.162361 | 0.162324 |
| 1..2 | 333 | 0.136018 | 0.136028 | 0.135939 |
| 2..4 | 284 | 0.075832 | 0.075833 | 0.075806 |
| >4 | 1 | <1e-18 | <1e-18 | <2e-12 |

The change removes false early impacts near rods and aperture edges. Away
from those boundaries the old and new errors are nearly identical.
Separately, mixed Hermite derivatives could leak one-sided metal derivatives
onto a vacuum mirror face. A synthetic rod tip produced a mirror-normal
field of -0.005818565 V/mm and hence a jump under reflection. Shared
odd-normal jets are now zero on mixed mirror faces, while wholly Dirichlet
faces retain their one-sided condition. The real quadrupole's vacuum mirror
normals are now exactly zero. Tests rotate the fixture through all three
axes and check 3D corner node interpolation and C1 fields without changing
existing tolerances.

### Physical geometry and the better field route

Errors against a different geometry must not be presented as interpolation
error on the same problem:

| Method | Node-cell local-16x RMS mm | Nearest-node 8x RMS mm | Physical round-rod 8x RMS mm |
|---|---:|---:|---:|
| Historical CPU nearest-node run | 2.364893 | 1.777465 | 1.405925 |
| CPU cell-default coarse PA | 1.023402 | 2.869093 | 2.382840 |
| SIMION T.Qual 209 coarse PA | 1.023425 | 2.868858 | 2.382644 |
| Coarse cut field plus exact-SDF collisions | -- | -- | **0.00343530** |

The physical cut control has max endpoint error **0.0201860 mm**, TOF RMS
**0.000185836 us**, and no changed exit classes. Its physical endpoint RMS is
about **694x smaller** than the coarse SIMION result. It uses the analytic
surface callback in the reference example; it is not a measurement of the
separate `--cuts` consumer.

The actual full 121x37x37 `refine-geometry` output is also staged:
`cli-cut.pa0`, bases 1..3, `cli-cut.pa#`, and `cli-cut.cuts.json`.
There are 11,160 cut nodes. The three basis solves total **441.074 ms**
compute; geometry preparation is **8.497 s**, outside that compute time.
The user route is physical geometry refinement followed by flying with its
matching surface sidecar, not refining a staircase mask and then changing
its collision voxels.

### Compute time at achieved precision

These are serial WSL release CPU runs on the same 5950X with 16 Rayon
workers. Setup and output are excluded from flight compute; SIMION's
recorded `Fly completed` compute times likewise exclude process startup.
The PA itself is the quadrupole workbench, so the benchmark does not add an
artificial zero-field discontinuity outside a coincident workbench box.

| 1,000 ions | Field setup s | Flight compute s |
|---|---:|---:|
| CPU, explicit nearest-node comparison | 0.444787 | 0.079285 |
| CPU, corrected cell default | 0.428838 | **0.065829** |
| Coarse physical cut control, half-X symmetry | 0.233089 | 0.068113 |
| SIMION T.Qual 3, coarse PA | not measured here | 0.100000 |
| SIMION T.Qual 209, coarse PA | not measured here | 9.280000 |

The corrected raw-PA CPU flight is **1.52x faster than T.Qual 3** at no worse
accuracy, and **141x faster than T.Qual 209** at matched accuracy. This is a
resident-field compute comparison, not an end-to-end speed claim. The
historical GPU 54.04 ms result used nearest-node collisions and is not a
matched-precision result for the new default.

### Unchanged 2D, TOF, RF and magnetic controls

The complete 100k einzel CSV is byte-identical to the preserved CPU output,
with exactly the original accepted/rejected totals. The 1k subset remains
**0.225707156 mm endpoint RMS / 0.010981092 us TOF RMS** against its saved 8x
reference. The test-plane crossing and final-wall controls have identical
reported coordinates and TOF between nearest-node and cell-default runs;
their endpoint RMS versus SIMION is 0.00132585 / 0.00611747 mm.

| Preserved independent reference | Before endpoint RMS mm | After endpoint RMS mm | Before/after endpoint and TOF difference |
|---|---:|---:|---:|
| TOF, 8x grid and tight steps | 6.922704e-8 | 6.922704e-8 | 0 |
| Cosine RF, quarter-step reference | 2.161336e-13 | 2.161336e-13 | 0 |
| Square RF, quarter-step reference | 1.305701e-13 | 1.305701e-13 | 0 |

The TOF reference-time RMS remains 3.498512e-10 us. The seven captured
magnetic examples all pass their existing native-reference accuracy gates.
At the existing 1 us cap, their maximum endpoint errors retain the
documented values: magnet1 2.34507e-7, mag90 4.92294e-7, sector 2dp
2.04451e-7, sector 2dc 2.74429e-7, sector 3dp 2.44417e-6, Helmholtz
1.46869e-8 and solenoid 1.22616e-6 mm.

All recorded TOF/RF numerical rows, including velocities, are identical to
their preserved CPU records. Their diagnostic compute times are:

| Small beam | Previous CPU compute ms | Current CPU compute ms |
|---|---:|---:|
| TOF, 4 ions | 2.6165 | 1.8124 |
| Cosine RF, 7 ions | 3.1972 | 1.0575 |
| Square RF, 7 ions | 2.4470 | 1.1732 |

The magnetic measurements below use the existing 1 us cap, 1e-10 tolerances,
16 workers and five-run medians. Previous medians are the retained historical
measurements, not a newly paired performance experiment.

| Magnetic model | Previous CPU compute ms | Current CPU compute ms |
|---|---:|---:|
| magnet1 | 2.2343 | 2.0960 |
| mag90 | 2.6643 | 2.8401 |
| sector 2dp | 4.6034 | 3.4442 |
| sector 2dc | 3.9928 | 3.9242 |
| sector 3dp | 10.0560 | 6.4321 |
| Helmholtz | 1.7843 | 2.0831 |
| Solenoid | 738.3168 | 595.5210 |

| Existing workload | Previous CPU total s | Cell-default CPU total s |
|---|---:|---:|
| Einzel 10k, including CSV | 0.382090 | 0.471319 |
| Einzel 100k, including CSV | 3.810812 | 3.945250 |

The new einzel flight phases are 0.441434 / 3.749933 s. The single-run
total ratios are 1.234 / 1.035; both remain within the documented 1.3x
budget. The identical states and step counts distinguish timing variation
from an accuracy or step-control change. Magnetic profiling medians remain
below the corresponding SIMION compute times; no magnetic model or
tolerance was changed.

### Historical CPU-only validation and integration boundary

The final CPU-owned checkout passes `cargo fmt --all -- --check`,
`cargo clippy --workspace --all-targets -- -D warnings`, all 18 release
accuracy tests, and the reference example's segment-clearance test.
The release workspace run passes the CLI/core/CPU suites before failing
the GPU geometry test: its unchanged legacy shader stops at x=3.5 mm
where the cell-default CPU stops at x=4.0 mm. The explicit nearest-node
comparison in that same test passes. No tolerance was weakened.
The separately owned GPU collision port and compact reconstruction must be
integrated before claiming a green full workspace or matched GPU results.
The language server could not run because `rust-analyzer` is absent from
the installed stable toolchain; compiler and strict Clippy checks succeeded.

### Integrated CPU, GPU and surface validation

The main checkout now combines the collision convention, mirror correction,
compact GPU reconstruction and continuous surface model. Mirror enforcement
runs after averaging each nodal jet and before storing it; the rotated 3D
mirror test checks exactly equal CPU samples from `GridField::new` and
`GridField::new_compact`, both on and beside the mirror plane. Historical
single-node voxel fixtures explicitly select nearest-node collisions, including
surface launch, GPU RF launch and automatic-step collision controls. No
assertion tolerance changed.

`cargo fmt --all -- --check`, strict all-target workspace Clippy, and
`cargo test --workspace --release` all exit 0. The workspace reports **178
passed, 0 failed**; its standalone GPU harnesses also pass 16 multigrid cases
and two residual checks on Linux llvmpipe, with DX12 explicitly unavailable.
The reference example's segment-clearance unit test passes separately.
Native Windows `cargo.exe test --release -p simrep-fly --test gpu` reports
**1 passed, 0 failed** on **NVIDIA GeForce RTX 3070 (DiscreteGpu)**.
LSP diagnostics remain unavailable because the stable toolchain has no
`rust-analyzer`; compiler and strict Clippy checks supply the code diagnostics.

Rerunning `original 1 --clearance` against the saved node-cell local-16x
reference gives endpoint RMS **1.023402317928029 mm**, TOF RMS
**0.073011795265478 us**, and 22 changed exit classes. Both vacuum mirror
normals are exactly zero. Rerunning `physical 1 --timing` against physical 8x
gives endpoint RMS **0.003435297160132 mm**, maximum **0.020186030469620 mm**,
TOF RMS **0.000185835501015 us**, and no changed exit class. The latter remains
the exact-SDF callback control, not a new measurement of the `--cuts` consumer.

The final Windows invocation uses `/mnt/c`, a 30 s timeout and closed stdin.
Cleanup exits 0 with no cargo, rustc or simrep process listed, zero orphan
consoles and zero stopped orphan consoles. The process counts are appended to
`bench/quadrupole-winproc.txt`.

### Reproduction and staged evidence

```sh
cargo build --release -p simrep-cli --examples
RAYON_NUM_THREADS=16 target/release/examples/quadrupole_convergence original 1 --clearance
RAYON_NUM_THREADS=16 target/release/examples/quadrupole_convergence original 1 --timing
for n in 2 4 8; do
  RAYON_NUM_THREADS=16 target/release/examples/quadrupole_convergence physical "$n"
  RAYON_NUM_THREADS=16 target/release/examples/quadrupole_convergence staircase "$n"
  RAYON_NUM_THREADS=16 target/release/examples/quadrupole_convergence node-staircase "$n"
done
RAYON_NUM_THREADS=16 target/release/examples/quadrupole_convergence node-local-8 8
RAYON_NUM_THREADS=16 target/release/examples/quadrupole_convergence node-local-8 16
RAYON_NUM_THREADS=16 target/release/examples/quadrupole_convergence node-local-4 16
RAYON_NUM_THREADS=16 target/release/examples/quadrupole_convergence physical 1 --timing
python3 tools/simion-oracle/collision_rf_tof.py
SIMREP_PHYSICS_ORACLE=/mnt/c/Users/lsu10/simrep-oracle/physics-validation-20261023 \
  RAYON_NUM_THREADS=16 cargo test --release -p simrep-fly --test magnetic_performance -- --nocapture
```

The example automatically chooses the global-cubic coefficient lattice for fine
references and bounds ordinary full-domain factors to 1/2/4/8. The local
16x cases are explicit. Saved PA solutions are reused without re-solving.
`tools/simion-oracle/compare_quadrupole.py` compares native CSV and full-precision
SIMION readout JSON, including exit-class changes and clearance groups.
The cut-cell CLI artifacts, all three reference ladders, comparison script
and exact commands are copied to
`C:\Users\lsu10\simrep-oracle\_quad_cut\README.txt`.
The old `_quad_gpu/reference-4-*.csv` remains historical, not a converged
spatial reference.

The final Windows command used `/mnt/c` as its working directory, a 60 s
timeout and closed stdin. `winproc.ps1 cleanup` exited zero with no remaining
SIMION process and `orphan conhost: 0` / `stopped orphan conhost: 0`.
Its complete output is retained in `bench/quadrupole-winproc.txt`.

## Source-geometry cuts v3 (October 23, 2026)

The quadrupole user route now carries source geometry from `refine-geometry`
to `fly --cuts`: analytic GEM CSG is evaluated before combination, and imported
closed triangles use BVH segment intersections. The default boundary-aware
121x37x37, h=0.5 mm run measures **0.00343534 mm endpoint RMS**,
**0.02018961 mm maximum**, and **zero exit-class changes out of 1,000** against
the independent physical 8x reference at h=0.0625 mm. TOF RMS/max are
0.000185835/0.001466332 us. This passes the requested 0.005 mm RMS and zero
class changes. The three exact-source coarse bases have relative residuals
3.432e-13, 3.431e-13, and 7.195e-13.

The earlier v2 same-workload result was 0.19562950 mm RMS with one class
change. Sampling an already-combined CSG field rounds away the aperture/cap
corner: ion 972 escaped through a point inside physical metal. More uniform
sampling does not recover that sign. V3 retains the tree, including transforms
and ordered electrode/vacuum fills, so this ion now hits the downstream
aperture. V1 and V2 still load with an explicit approximate label; v3 retains
their sampled fallback. The binary payloads, versioning and CAD tessellation
limit are specified in `docs/geometry-pipeline.md`.

Artifacts for this run are `target/cuts-v3/quadrupole.{pa0,cuts.json,csv}`.
Source particles and the independent reference remain unchanged:
`C:\Users\lsu10\simrep-oracle\_quad_gpu\particles.ion` and
`C:\Users\lsu10\simrep-oracle\_quad_cut\physical-8-global.csv`.
Reproduction uses the commands in `docs/geometry-pipeline.md`; options are
`--voltage 1=2.5,2=-2.5,3=0`, `--bounds=0,-18,-18,60,18,18`,
`--tolerance 1e-11`, `--max-step-us 0.025`, CPU f64, 16 Rayon threads,
relativistic dynamics and the default reconstruction.

The quadrupole was also exported with `simrep export geometry`, reimported
from STL, repaired/checked, refined on a full 121x73x73 domain, and flown with
the real CLI. Its 106,800 stored triangles give 1.442479e-8 mm RMS /
1.516489e-7 mm maximum difference from direct mesh-SDF flights on the same
field, with no exit changes. This is a matched-triangle control. PA geometry
export is voxelized and is not an analytic GEM tessellation: that altered STL
gives 1.74105488 mm RMS / 43 exit changes against the analytic physical 8x
reference, so it is not presented as the analytic acceptance case.

The sphere/cylinder STL segment errors are 2.740999e-5 / 6.564742e-7 mm
against analytic surfaces, independent of fallback supersampling 1/2/4/8.
The einzel 36-neutral-impact collision control is 0 mm RMS versus physical
8x at printed precision (nearest-node staircase: 0.425 mm). The original
einzel beam's 0.42564868 mm RMS remains dominated by its coarse field; the
physical 4x/8x RMS difference is 0.04699796 mm.

Five interleaved serial CPU runs (16 Rayon threads, no concurrent build or
benchmark) give median flight compute **0.130802 s cuts / 0.097657 s no-cuts =
1.339x** for quadrupole and **0.156970 / 0.103468 s = 1.517x** for the 1,036-ion
einzel/neutral-impact population. The latter is slightly above the approximately
1.5x target. The no-cuts timings use the default all-corner staircase rule,
not the historical nearest-node harness switch. Inputs and numerical options
are matched; differing collision geometry may change accepted work.

Geometry setup is excluded from those compute ratios. Median setup is
0.636995/0.208365 s (quadrupole cuts/no-cuts), including full-domain fallback
JSON loading and validation, and 0.012957/0.012807 s (einzel). Median total
including CSV but excluding process startup is 0.788538/0.329190 s and
0.170630/0.117345 s, respectively. Retaining fallback data makes quadrupole
setup appreciable; a compute result is not a whole-command speed claim.

Final fmt, workspace/all-target Clippy with `-D warnings`, explicit
`simrep-py` Clippy, and `cargo test --workspace --release` all exited zero.
There were 188 passing standard tests, 16 passing Vulkan multigrid cases and
f32/f64 smoke passes; DX12 was unavailable in WSL. CAD-enabled Clippy and both
STEP/IGES CLI tests passed too. OCCT's existing C++ `std::iterator` deprecation
warning remains; Rust LSP timed out, with compiler/Clippy diagnostics clean.

## Reproduction and local evidence

Diagnostic artifacts are under `target/fly-real/`; full-precision SIMION logs
are under `C:\Users\lsu10\simrep-oracle\_real_einzel`, `_real_original`,
and `_real_plane`. Large raw logs are deliberately not committed.

```sh
python3 tools/simion-oracle/compare_fly.py compare \
  /mnt/c/Users/lsu10/simrep-oracle/_real_einzel/reference-100000.txt \
  target/fly-real/native-100000.csv
cargo run --release -p simrep-cli --example einzel_clearance -- \
  "$P/einzel.pa0" "$P/einzel_100000.ion" target/fly-real/clearance.csv
python3 tools/simion-oracle/compare_fly.py classify \
  /mnt/c/Users/lsu10/simrep-oracle/_real_einzel/reference-100000.txt \
  target/fly-real/native-100000.csv target/fly-real/clearance.csv
cargo run --release -p simrep-cli --example einzel_convergence
```

## Historical 2D validation and Windows process hygiene

Final `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --
-D warnings`, and `cargo test --workspace` all exited 0. The existing 12 CPU
flight accuracy tests remain passing at unchanged tolerances. Three new
boundary tests pass: a 3D cubic vacuum field next to flat metal, shared C1
fields/node interpolation around a corner, and cylindrical-axis/reflection/
anti-mirror boundary conditions. Earlier checks cover fast-adjust
superposition and malformed input, literal FLY2, placement/scale, field-free
entry/drift, oblique planes and the diagnostic segment-clearance calculation.
The LSP daemon timed out, so successful compiler/Clippy diagnostics provide the
code checks. DX12 GPU tests skipped for no adapter; Vulkan tests passed on
llvmpipe, including f64 residual error zero.

Every Windows executable ran directly in the foreground of its shell, with
`timeout` and stdin redirected from `/dev/null`; no detached Windows launch was
used. No Windows timeout required killing a process. The last Windows command
was:

```sh
timeout 60 powershell.exe -NoProfile -ExecutionPolicy Bypass \
  -File tools/win/winproc.ps1 cleanup < /dev/null
```

It exited 0 and printed:

```text
cmd.exe            5
conhost.exe       34
node.exe           4
powershell.exe     2
orphan conhost: 0
stopped orphan conhost: 0
```

No `simion.exe` remained in the process-count table. The remaining consoles
belong to existing processes and were not killed.

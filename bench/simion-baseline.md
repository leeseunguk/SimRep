# SIMION baseline performance

- SIMION: `SIMION 2024-09-26-8.2.1.0` (`C:\Program Files\SIMION-2024\simion.exe --nogui --version`)
- Date: 2026-10-23
- Machine: Ryzen 9 5950X 16C/32T, RTX 3070 8 GB, 32 GB RAM (run under WSL2; simion.exe is the native Windows binary)
- Harness: `tools/simion-oracle/run_oracle.py` (foreground `simion.exe --nogui --noprompt`, wall clock per call; `seconds` includes SIMION startup ~0.05-0.10 s). `simion time` below is the time SIMION itself reports (`Refine completed ... in X Sec` / `Fly completed ... X seconds`), which excludes process startup and file I/O.
- Default threads = SIMION decides (uncapped `--num-threads`); `1t` = `--num-threads 1`. Default convergence = SIMION's built-in refine objective (observed deltas below).

## Refine

| case | PA | dims / points | electrodes | conv target | iters | delta | simion time | wall s (def threads) | wall s (1 thread) |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| perf 5 planar, small | `small_pa_planar.pa#` | 103x103x1 = 10,609 | 2 | default | 380 / 390 (+1) | 0.0031 V | 0.06 s | 0.188 | 0.098 |
| perf 5 planar, big | `big_pa_planar.pa#` | 821x821x1 = 674,041 | 2 | default | 520 / 530 (+1) | 0.0044 V | 0.19 s | 0.284 | 0.592 |
| perf 6 cylinder, small | `small_pa_cylinder.pa#` | 26x103x1 = 2,678 | 2 | default | 240 / 270 (+1) | 0.0013 V | 0.09 s | 0.149 | 0.095 |
| perf 6 cylinder, big | `big_pa_cylinder.pa#` | 801x3281x1 = 2,628,081 | 2 | default | 290 / 320 (+1) | 7.3e-06 V | 0.17 s | 0.218 | 0.296 |
| perf 8 rf mirror | `rf_mirror.pa#` | 401x83x1 = 33,283 | (pre-refined, 1 iter) | default | 1 | 1e-100 V | ~0 s | 0.166 | 0.105 |
| perf 8 rf square wave | `rf_square_wave.pa#` | 401x83x1 = 33,283 | (pre-refined, 1 iter) | default | 1 | 1e-100 V | ~0 s | 0.171 | 0.096 |
| einzel (benchfly) | `einzel.pa#` | 91x20x1 = 1,820 | 3 | default | (small) | — | ~0.1 s | 0.189 | — |
| `_bench3d` sc3d_x2 | `sc3d_x2.pa#` | 259x259x259 = **17,373,979** | 2 (500/-333 V shells) | default | 490 x2 | 0.0048 V | 2.02 s | **2.191** | **8.646** |
| `_bench3d` sc3d_x2 | `sc3d_x2.pa#` | 17,373,979 | 2 | `--convergence 1e-7` | 1970 / 1980 | 9.1e-08 V | 14.23 s | **14.365** | not run |

`iters` = refine iterations per solution array (each `.paN`), in log order; `(+1)` is the trivial pa0 combine pass.

## Fly

| case | IOB | particles | wall s (def threads) | wall s (1 thread) | simion time | result |
| --- | --- | --- | --- | --- | --- | --- |
| perf 6 small cyl | `small_pa_cylinder.iob` | 2 | 0.083 | — | 0.00 s | 2 splats |
| perf 6 big cyl | `big_pa_cylinder.iob` | 1 | 0.104 | — | 0.01 s | 1 splat |
| perf 5 big planar | `big_pa_planar.iob` | 1 | 0.079 | — | 0.00 s | 1 splat |
| perf 8 rf mirror | `rf_mirror.iob` | 7 | 0.080 | — | 0.01 s | 7 splats |
| perf 8 rf square | `rf_square_wave.iob` | 7 | 0.076 | — | 0.00 s | 7 splats |
| `_benchfly` einzel 10k | `einzel.iob --particles einzel_10000.ion` | 10,000 | 2.201 | 2.156 | 2.13 s | 10,000 splats |
| `_benchfly` einzel 100k | `einzel.iob --particles einzel_100000.ion` | 100,000 | **21.504** | **21.149** | 21.37 s | 100,000 splats |

Fly is effectively single-threaded: `--num-threads 1` makes no difference.

## Large 3D refine case

`examples/performance_tests` ships **no large 3D case** — all `.pa#`/`.pa` files there are 2D (planar/cylindrical, max ~2.6M points) or trivially small. The `sc3d_x2` case (259^3 = 17.37M points, 139 MB `.pa#`) was built headless via `gem2pa` from a two-shell-spheres `.gem` (`pa_define(130,130,130,planar,xyz)` scaled 2x) by a previous setup step; SIMION's CLI does support this, so the case is real and reproducible, not synthetic smoke. Gem source: `C:\Users\lsu10\simrep-oracle\_bench3d\sc3d.gem`.

## Exact invocations

All from `cd /mnt/c`, `< /dev/null`, foreground with `timeout`, via the harness:

```sh
R=tools/simion-oracle/run_oracle.py

# performance_tests examples (copied fresh to C:\Users\lsu10\simrep-oracle\<example>)
python3 $R performance_tests/6_cylindrical_pa_tests --work-dir <w> --refine --fly big_pa_cylinder.iob
python3 $R performance_tests/6_cylindrical_pa_tests --work-dir <w> --refine --num-threads 1
python3 $R performance_tests/8_rf_tests             --refine --fly rf_mirror.iob
python3 $R performance_tests/8_rf_tests             --refine --num-threads 1
python3 $R performance_tests/8_rf_tests             --work-dir <w> --fly rf_square_wave.iob
python3 $R performance_tests/5_planar_pa_tests      --work-dir <w> --refine --fly big_pa_planar.iob
python3 $R performance_tests/5_planar_pa_tests      --work-dir <w> --refine --num-threads 1

# big ion-count fly (einzel IOB + generated .ion files)
python3 $R _benchfly --work-dir C:\...\simrep-oracle\_benchfly --fly einzel.iob \
    --globals "--default-num-particles 110000" --fly-args "--particles einzel_100000.ion"
# (same with --num-threads 1, and with einzel_10000.ion / --default-num-particles 12000)

# large 3D refine
python3 $R _bench3d --work-dir C:\...\simrep-oracle\_bench3d --refine
python3 $R _bench3d --work-dir C:\...\simrep-oracle\_bench3d --refine --num-threads 1
python3 $R _bench3d --work-dir C:\...\simrep-oracle\_bench3d --refine --refine-args "--convergence 1e-7"
```

The effective SIMION command line for e.g. the 100k-ion fly (cwd `C:\Users\lsu10\simrep-oracle\_benchfly`):

```
simion.exe --nogui --noprompt --default-num-particles 110000 fly \
    --recording-enable 1 --recording-output einzel.oracle_recording.txt \
    --particles einzel_100000.ion einzel.iob
```

Notes:
- `gem2pa` writes `.pa` (solution array), not `.pa#`; to get a refinable `big_pa_*.pa#` the IOB's auto-build prompt (default OK under `--noprompt`) was used once, then refine was run on the `.pa#`.
- Performance-test `.pa#` files in the examples are pre-built; `rf_mirror`/`rf_square_wave` refine in 1 iteration (already converged), so their times are load+check only.
- Refine thread scaling (3D, default convergence): 2.19 s default threads vs 8.65 s single-thread (~4x). For 2D cases the times are dominated by process startup.

## Evidence

Raw per-call logs (`oracle.json`, full SIMION stdout/stderr, exit codes, wall times):
`~/.omo/evidence/01a1666a-618f-765c-9d88-07324ea07e51/simion-baseline/a1/` —
`bench3d_default.json`, `bench3d_1t.json`, `bench3d_conv1e7.json`, `cyl_default.json`, `cyl_1t.json`, `cyl_fly_small.json`, `planar_default.json`, `planar_1t.json`, `rf_default.json`, `rf_1t.json`, `rf_fly_sqw.json`, `fly_10k_default.json`, `fly_10k_1t.json`, `fly_100k_default.json`, `fly_100k_1t.json`.

Process hygiene verified at end: `winproc.ps1 cleanup` — no `simion.exe` running, 0 orphan conhost.

## Oracle coverage (phase 0)

Date: 2026-10-23. Full oracle results (argv, exit codes, stdout/stderr, per-call seconds) under
`C:\Users\lsu10\simrep-oracle\<example>\oracle.json`, produced by `tools/simion-oracle/run_oracle.py`
(900 s per-call timeout, default threads). For examples with multiple `.iob`s, extra flies were run
via `--work-dir` passes and merged into the same `oracle.json`. `refine exit/s` = all refine runs
exit 0 and total refine wall time; `fly exit/s` likewise over every `.iob` flown; `ions` = splats
per fly (from `Fly completed` in stdout), in the order the IOBs are listed.

| example | refine exit / s | fly exit / s | ions (per iob) | has Lua | notes |
| --- | --- | --- | --- | --- | --- |
| `quad` | 0 / 0.288 (3 `.pa`) | 0 / 0.954 (quad.iob) | 25 | y | no `.pa#`/`.pa` shipped: 4x `gem2pa` (all exit 0) then `refine` on quad/quadin/quadout `.pa`; fly auto-built `.pa#`+`.pa0` from gems. `quad.lua` exists but is not attached to `quad.iob`; ran with `--programs 1`, static DC fields only. quad_monolithic/quad_wgem subvariants not covered |
| `tof` | 0 / 0.112 (source.pa#) | 0 / 0.078 (tof.iob) | 4 | n | mirror.pa/detect.pa are pre-refined and loaded as-is; `tof.rec` used |
| `performance_tests/1_ke_to_velocity` | 0 / 0.071 | 0 / 0.095 | 2 | n | `.prg` auto-compiled by SIMION at fly time |
| `performance_tests/2_linear_acceleration` | 0 / 2.308 (5 runs) | 0 / 0.289 (3 iobs) | 1, 3, 1 | n | `relativistic_cross.iob` auto-built `relativistic_cross.pa#`+`.pa0` from its `.gem`; a second `--refine` pass then covered all 3 `.pa#` |
| `performance_tests/3_linear_reflection` | 0 / 0.112 | 0 / 0.083 | 3 | n | |
| `performance_tests/4_radius_of_refraction` | 0 / 0.155 (2 `.pa`) | 0 / 0.159 (2 iobs) | 5, 3 | n | `.pa`s refined in place; iobs load `.pa` directly |
| `performance_tests/5_planar_pa_tests` | 0 / 0.582 (3 runs) | 0 / 0.560 (2 iobs) | 1, 1 | n | `block.pa` refined + fly auto-built `big/small_pa_planar.pa#`; second `--refine` pass covered both `.pa#` |
| `performance_tests/6_cylindrical_pa_tests` | 0 / 0.562 (3 runs) | 0 / 0.350 (2 iobs) | 1, 2 | n | same auto-build pattern; `small_pa_cylinder.pa#` shipped |
| `performance_tests/7_dc_quadrupole_tests` | 0 / 1.285 (3 runs) | 0 / 1.226 (2 iobs) | 6, 6 | n | `block.pa` refined; `quad.pa#`/`quad1.pa#` auto-built on first fly, refined in a second pass |
| `performance_tests/8_rf_tests` | 0 / 0.372 (2 `.pa#`) | 0 / 0.151 (2 iobs) | 7, 7 | n | `.pa#`s pre-converged (1-iteration refine) |

All 28 SIMION calls across the 10 examples exited 0; no timeouts, no failures. `has Lua` = a `.lua`
file ships with the example; legacy `.prg` programs in performance_tests are auto-compiled by SIMION
during fly and do not count as Lua.

## Oracle coverage (phase 5)

Date: 2026-10-23. Same protocol as "Oracle coverage (phase 0)": every SIMION call via
`tools/simion-oracle/run_oracle.py` (foreground `simion.exe --nogui --noprompt`, 1800 s per-call
timeout), plus two documented manual calls for `dielectric/plates_2dp` (needs a `.pa#` output name)
and one batch-lua run of `solenoid/solenoid_write.lua`. All flies ran with `--programs 1` and
`--recording-enable 1` (recording output `<iob stem>.oracle_recording.txt` in the work dir).
Full argv/stdout per call is in `oracle*.json` artifacts under
`.omo/evidence/phase5-oracle/` (also `C:\Users\lsu10\simrep-oracle\<example>\oracle.json`, the last
fly per work dir). `compute s` = SIMION's own reported seconds (`Refine completed ... in X Sec`,
`File saved ... in X Sec`, `Fly completed. N splats, X seconds`), summed over the runs listed;
wall time per call is in the artifacts. `ions` = splats per fly. Every call exited 0; no timeouts.

| example | files | Lua? | runs | exit | SIMION compute s | notes |
| --- | --- | --- | --- | --- | --- | --- |
| `magnet` | 3 `.gem`, 3 `.pa#`, `mag90.iob`+`.fly`, `magnet1.iob`+`.fly` | none | 3 gem2pa, 3 refine `.pa#`, 2 fly | 0 | 0.33 | `magnet1.iob` uses `magnet1.pa0` (magnetic) + `elect1.pa0` (electric); both flies 10 splats |
| `magnetic_sector` | `sector_2dc/2dp/3dp.iob`, 6 `.gem`, 2 `.lua`, `.con`, `.fly2`s | `sector_2dc.lua` (zero B-field outside `phi=90` sector, draw wedge); `sector_3dp.lua` (Liebl focal-point marks + B-field polarity flip; reads `params.txt`) | 6 gem2pa, 6 refine, 3 fly | 0 | 0.67 | `sector_3dp.gem` writes `params.txt` at gem2pa time; splats 15/50/15 (2dp/2dc/3dp); `.fly2` variants and `sector_2dc_*.gem` alternate-geometry subcases not flown (no IOB) |
| `helmholtz_coil` | `helmholtz.iob`+`.lua`+`.rec`+`.fly2`, `mag.pa`, 3 coil subfolders each `mag.pa`+`.iob`+`.lua` | all 4: `simionx.MField` Biot-Savart coil field -> `mfield_adjust` (rectangular=line-segment loops, saddle/thick=hoop sets; all print axial Bz + `B_actual`/`B_theo` checks) | 1 refine `mag.pa` (empty, 1 iter) + 4 fly | 0 | 5.43 | splats 27/1/0/27 (helmholtz/rectangular/saddle/thick); saddle+rectangular Lua splat ions at `tof>0.03 us` |
| `solenoid` | `solenoid.iob`+`.lua`, `toroid.iob`+`.lua`, `mag.pa`, `solenoid_write.lua` | `solenoid.lua` Biot-Savart `solenoid_hoops` field + in-field color/marker; `toroid.lua` 73-hoop toroid field; `solenoid_write.lua` = standalone batch script writing `out.csv` (run, rc=0) | 1 refine `mag.pa` + 2 fly + 1 lua | 0 | 8.99 | splats 100 (solenoid) / 5 (toroid); `out.csv` Bx on axis 9.81-8.51 G written to work dir |
| `repulsion` | `repulsion1/2`, `pierce`, `cylindrical_diode`, `spherical_diode` `.iob`+`.lua`; `blank.pa`, `blank_small.pa`, 3 `.gem` | 5: Coulomb space-charge models (`repulsion1/pierce/spherical_diode` print kQ/r sums on a point line; `repulsion2` accumulates space-charge potentials into `blank_small.pa`; `cylindrical_diode` prints time-averaged kQ/r voltage) | 3 gem2pa, 5 refine, 5 fly | 0 | 29.26 | splats 103/1003/442/1000/1000; `blank*.pa` are empty containers; alternate `.fly2` beam files (`*_coulombic`, `*_mixed*`, `repulsion2.con`) not flown |
| `collision_hs1` | `collision_hs1.iob`+`.fly`+`.lua`, `test2.iob`+`.fly2`+`.lua`, `collision_hs1.pa`, `make.lua`, `old/*.prg` | `collision_hs1.lua` (544 ln) = hard-sphere elastic ion-neutral collision model (adjustable P/T/mass/diameter); `test2.lua` imports it with `_mode` position-dependent gas velocity/temperature profiles; `make.lua` = batch generator for the `_mode 1.3` CSV/PA inputs (not needed at default `_mode=1`, not run) | 1 refine `collision_hs1.pa` + 2 fly | 0 | 0.77 | splats 10 (hs1) / 57 (test2) |
| `collision_sds` | `test/test2/test3/test-sds-hs1` `.iob`+`.lua`, `collision_sds.lua`, `test.pa#`, 5 `test3_*.pa`, `.dat` fields, `test.rec` | `collision_sds.lua` (1543 ln) = Statistical Diffusion Simulation mobility/diffusion model driven by `mbmr.dat` + `*_defs.dat`/PAs; test/test2/test3 attach it with field sources adjustable/func/PA; `test-sds-hs1.lua` merges SDS+HS1 side-by-side per ion parity; support `arraylib`/`textfilelib`; `test3_prepare_arrays.lua` = batch PA builder (PAs already shipped, not run) | 1 refine `test.pa#` (4 electrodes+pa0) + 4 fly | 0 | 27.02 | splats 500/700/700/50; `test.rec` used for `test.iob` |
| `dielectric` | 10 `.iob`+`.lua`+`.con`, 26 `.gem` | all 10: `flym` solves `pa:refine{permittivity=dipa}` (Poisson with dielectric/charge/conductivity PAs; `*_nonlinear*` iterate field-dependent permittivity 15x; `float_cylinder` adds charge PA via `../poisson/palib.lua`; `*_solve*.lua` = batch equivalents, not needed at fly) | 26 gem2pa + manual `gem2pa plates_2dp.gem plates_2dp.pa#` + `refine` + 10 fly | 0 | 137.21 | dielectric `.pa`s deliberately NOT refined (permittivity arrays; Lua re-solves at flym); `plates_2dp.iob` needs `.pa0`+`.pa#` so its PA was built manually as `.pa#`+refined; splats all 0 (no ions; these are field-solve examples, verification prints in fly stdout e.g. `E(center)_theo/actual`); `sphere_nonlinear_3dp` fly compute 106.8 s |
| `geometry` | 38 `.gem` top + 36 `.gem` in `8.1/`, `tube_multipa.wgem`, `two_cylinder*.stl`, `twisted_rods_build.lua` | `twisted_rods_build.lua` = standalone PA-builder script (alternative to `twisted_rods.gem`, not needed for gem2pa) | 74 gem2pa + 72 refine (all exit 0) | 0 | 9.15 | no `.iob` exists here, so no fly; `det_inc.gem`/`dyn_inc.gem` compile-only include gems (produce no PA); `tube_multipa.wgem` skipped (not a `.gem`); `sphere_mesh.pa` refine heaviest at 34460 iters/5.9 s; refine of `sphere_aniso.pa` asked a quality prompt -> `--noprompt` answered Y |
| `geometry/8.1` | 36 `.gem` (legacy 8.1 syntax variants of the same shapes) | none | 36 gem2pa + 34 refine, all exit 0 | 0 | 1.46 | totals folded into the `geometry` row; `det_inc.gem`/`dyn_inc.gem` are include-only |

236 SIMION calls total across phase-5 coverage (233 via `run_oracle.py`, plus manual
gem2pa + refine for `plates_2dp.pa#` and the `solenoid_write.lua` batch run): every call exit 0. Recording outputs (`*.oracle_recording.txt`), generated `.pa`/`.pa0`/`.pa#`
arrays, `params.txt` and `out.csv` live in `C:\Users\lsu10\simrep-oracle\<example>\` alongside the
per-invocation `oracle.json` snapshots archived in `.omo/evidence/phase5-oracle/`.

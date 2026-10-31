# FLY2 oracle validation

Direct SIMION `create_ionfile` export; no particle flights.

| Result | Files |
| --- | ---: |
| match | 97 |
| statistical match | 57 |
| mismatch | 0 |
| unsupported | 0 |

Random definitions aggregate seeds 1 through 8. SIMION seeds both
`math.randomseed` and `simion.seed`; native uses explicit SplitMix64 seeds.
Streams are intentionally not expected to match sample-for-sample.
Each scalar physical column plus radial and position-direction projections
is checked with two-sample KS and six-standard-error first/second moment
bounds. KS uses a Bonferroni family-wise alpha of 0.01 over 154 x 20 tests.
Deterministic rows are compared in order at 2e-11 relative / 1e-13 absolute
tolerance (the oracle writes 14 significant digits). CWF uses 1e-24
absolute tolerance so tiny emission currents cannot be mistaken for zero.
PA-coordinate ION exports are normalized to physical WB units using the
actual IOB placement. Field-emission sources replay the SIMION field-query
trace, checking query order and coordinates, to test FLY2 independently
of the flight engine's potential reconstruction. Their IOB working copy
loads saved PA fields with its workbench program and automatic particle
source disabled; the original
FLY2 and cathodelib.lua are executed unchanged.

## Regression checks

- `cargo fmt --all -- --check`: passed.
- `cargo clippy --workspace --all-targets -- -D warnings`: passed.
- `cargo clippy -p simrep-py --features python -- -D warnings`: passed.
- `cargo test --workspace --release`: passed, including 15 particle-input tests.
- Python production extension (`python,pyo3/extension-module`): 31 tests passed,
  including PA-coordinate placement, in-memory fields, individual electrode
  nodes and initial voltage-expression fields.
- Shared-sandbox GEM rerun: 254 standalone matches, seven include fragments,
  zero unsupported. Saved SIMION oracle outputs were reused, and all native
  sources were recompiled. Its performance rows are contended, not new timing
  claims.
- LSP diagnostics were unavailable: the daemon timed out twice after 30 seconds.
  Compiler and clippy checks above completed cleanly.

Deliberately unsupported features are binary FLY, native Lua extensions,
unrestricted filesystem/process access, negative coordinate modes, filled
deterministic circle/cone sequences, workbench programs and PA mutation inside
the source environment. None prevents loading an installed FLY2 source.

## Final Windows process cleanup

`tools/win/winproc.ps1 cleanup` ran in the foreground with a 30-second timeout,
a `/mnt/c` working directory and closed stdin. No SIMION process remained.
The two busy conhosts have live WSL parents and were left untouched.

```text
== process counts ==
Name           Count
cmd.exe            5
conhost.exe       24
node.exe           4
powershell.exe     2

== conhost by parent ==
cmd.exe                               4
wsl.exe                               3
wslhost.exe                           2
node_repl.exe                         2
lmgrd.exe                             2
powershell.exe                        2
operafea-op3solve.exe                 1
rlm.exe                               1
node.exe                              1
sunshine.exe                          1
dispatcher.exe                        1
wlanext.exe                           1
codex.exe                             1
matlab-mcp-server-windows-x64.exe     1
GoogleDriveFS.exe                     1

orphan conhost: 0
busy conhost (>50% of a core): 2
  pid 26152 cpu 100% parent wsl.exe (49640) wsl.exe -d Ubuntu --exec sh -s -- $REPO
  pid 52060 cpu 100% parent wsl.exe (33024) wsl.exe -d Ubuntu --exec sh -c "cd '/mnt/d/Private/Opera_test' && export PATH=\"$HOME/.local/bin:$PA
stopped orphan conhost: 0
```

## Per-file results

| File | Result | Particles (all seeds) / reason |
| --- | --- | --- |
| `bender_cut/bend.fly2` | match | 2 |
| `bender_cut/bend_xy.fly2` | match | 2 |
| `bradbury_nielsen_grid/bngrid.fly2` | match | 231 |
| `child_particles/child.fly2` | match | 5 |
| `child_particles/child80.fly2` | match | 5 |
| `collision_hs1/test2.fly2` | match | 57 |
| `collision_rs/test.fly2` | statistical match | 800 |
| `contour/drag-equipotential.fly2` | match | 38 |
| `contour/drag-vector.fly2` | match | 285 |
| `contour/drag.fly2` | match | 15 |
| `contour/solenoid.fly2` | match | 42 |
| `contour/solenoid81.fly2` | match | 0 |
| `count_transmission/count.fly2` | statistical match | 1600 |
| `dielectric/charged_dielectric_2dp.fly2` | match | 0 |
| `dielectric/cylinder_2dp.fly2` | match | 0 |
| `dielectric/cylinder_nonlinear_2dp.fly2` | match | 0 |
| `dielectric/float_cylinder_2dp.fly2` | match | 0 |
| `dielectric/plates_2dp.fly2` | match | 0 |
| `dielectric/resistive_2dp.fly2` | match | 0 |
| `dielectric/sphere_2dc.fly2` | match | 0 |
| `dielectric/sphere_3dp.fly2` | match | 0 |
| `dielectric/sphere_nonlinear_2dc.fly2` | match | 0 |
| `dielectric/sphere_nonlinear_3dp.fly2` | match | 0 |
| `electrostatic_induction/parallel_plates.fly2` | match | 1 |
| `electrostatic_induction/parallel_plates2.fly2` | match | 2 |
| `faims/cylindrical_plates.fly2` | match | 5 |
| `faims/cylindrical_transverse.fly2` | match | 5 |
| `faims/cylindrical_transverse_focus.fly2` | match | 6 |
| `faims/plates.fly2` | match | 5 |
| `faims/plates_drag.fly2` | match | 5 |
| `faims/repulsion.fly2` | statistical match | 4000 |
| `faims/spectrum.fly2` | statistical match | 240 |
| `faims/y_range.fly2` | match | 5 |
| `field_array/cfd.fly2` | match | 8 |
| `field_array/field_array.fly2` | statistical match | 800 |
| `field_array/field_array_x.fly2` | statistical match | 800 |
| `field_array/solenoid_pa.fly2` | statistical match | 800 |
| `field_emission/fe_geom.fly2` | match | 0 |
| `field_emission/fe_rays.fly2` | match | 20 |
| `geometry_optimization/mesh_size.fly2` | match | 11 |
| `helmholtz_coil/helmholtz.fly2` | match | 27 |
| `helmholtz_coil/helmholtz_many.fly2` | match | 1323 |
| `helmholtz_coil/saddle_coil/saddle_coil.fly2` | match | 0 |
| `helmholtz_coil/thick_coil/thick_coil.fly2` | match | 27 |
| `icrcell/icr.fly2` | statistical match | 160 |
| `ims/ims.fly2` | statistical match | 200 |
| `ionfunnel/funnel.fly2` | statistical match | 80 |
| `ionfunnel/funnel2.fly2` | statistical match | 80 |
| `ionfunnel/funnel3.fly2` | statistical match | 80 |
| `kingdon_trap/kingdon_2dc.fly2` | match | 8 |
| `kingdon_trap/quadro_logarithmic.fly2` | statistical match | 800 |
| `kingdon_trap/twocylinder_1dc.fly2` | match | 4 |
| `lens_properties/cardinal_points.fly2` | match | 4 |
| `lens_properties/confusion_disc.fly2` | statistical match | 800 |
| `lens_properties/helmholtz_lagrange.fly2` | match | 11 |
| `lens_properties/particle_spread_measurement/spread.fly2` | statistical match | 800 |
| `lens_properties/pq.fly2` | match | 2 |
| `lens_properties/snell.fly2` | match | 10 |
| `lens_properties/surface_plot.fly2` | match | 3 |
| `lens_properties/zoom_afocal.fly2` | match | 2 |
| `lens_properties/zoom_lens_curve.fly2` | match | 4 |
| `lens_properties/zoom_lens_curve_75.fly2` | match | 4 |
| `magnetic_potential/c_magnet_2dp.fly2` | match | 1 |
| `magnetic_potential/current_cylinder_2dp.fly2` | match | 0 |
| `magnetic_potential/current_sphere_2dc.fly2` | match | 0 |
| `magnetic_potential/current_sphere_3dp.fly2` | match | 0 |
| `magnetic_potential/emag_quad_2dp.fly2` | match | 1 |
| `magnetic_potential/mag_2cylinder_2dc.fly2` | match | 0 |
| `magnetic_potential/mag_2ring_2dc.fly2` | match | 0 |
| `magnetic_potential/mag_cylinder_2dc.fly2` | statistical match | 80 |
| `magnetic_potential/mag_halbach_cylinder_2dp.fly2` | match | 0 |
| `magnetic_potential/mag_quad_2dp.fly2` | match | 0 |
| `magnetic_potential/mag_ring_2dc.fly2` | match | 0 |
| `magnetic_potential/mag_sphere_2dc.fly2` | match | 0 |
| `magnetic_potential/mag_sphere_3dp.fly2` | match | 0 |
| `magnetic_potential/mu_cylinder_2dp.fly2` | match | 0 |
| `magnetic_potential/mu_sphere_2dc.fly2` | match | 0 |
| `magnetic_potential/mu_sphere_3dp.fly2` | match | 0 |
| `magnetic_potential/mu_torus_3dp.fly2` | match | 0 |
| `magnetic_potential/mucurrent_cylinder_2dp.fly2` | match | 0 |
| `magnetic_potential/solenoid_2dc.fly2` | match | 0 |
| `magnetic_potential/spotential_from_bfield.fly2` | match | 0 |
| `magnetic_potential/uniform_2dp.fly2` | match | 0 |
| `magnetic_sector/sector_2dc.fly2` | statistical match | 400 |
| `magnetic_sector/sector_2dc_conical.fly2` | statistical match | 160 |
| `magnetic_sector/sector_2dc_focal.fly2` | match | 6 |
| `magnetic_sector/sector_2dc_masses.fly2` | match | 18 |
| `magnetic_sector/sector_2dp.fly2` | match | 15 |
| `magnetic_sector/sector_3dp.fly2` | match | 15 |
| `misc/disc_seq.fly2` | match | 317 |
| `misc/quadrilateral.fly2` | match | 5000 |
| `molecular_beam/aa_deflector.fly2` | match | 36 |
| `molecular_beam/aa_deflector_accel_adjust.fly2` | match | 36 |
| `molecular_beam/octupole.fly2` | match | 10 |
| `multiprocess/quad-1.fly2` | statistical match | 200 |
| `multiprocess/quad-2.fly2` | statistical match | 200 |
| `octupole/octupole.fly2` | statistical match | 400 |
| `particles/distribution_rejection_sampling.fly2` | statistical match | 160 |
| `particles/lambert_cosine.fly2` | statistical match | 8000 |
| `plot/tof_histogram.fly2` | statistical match | 40000 |
| `poisson/octupole_sc/octupole.fly2` | statistical match | 400 |
| `poisson/octupole_sc/octupole_coulombic.fly2` | statistical match | 2000 |
| `poisson/pierce_gun/pierce_2dcyl.fly2` | match | 20 |
| `poisson/pierce_gun/pierce_2dpla.fly2` | match | 20 |
| `poisson/pierce_gun/pierce_3dpla.fly2` | match | 400 |
| `poisson/pierce_gun/pierce_collisionpulse_2dcyl.fly2` | match | 1000 |
| `poisson/pierce_gun/pierce_pulse_2dcyl.fly2` | match | 1000 |
| `poisson/pierce_gun/pierce_pulse_2dcyl3d.fly2` | statistical match | 8000 |
| `poisson/pierce_gun/pierce_pulse_2dcylbig.fly2` | match | 1000 |
| `poisson/pierce_gun/pierce_pulse_2dpla.fly2` | match | 1000 |
| `poisson/pierce_gun/pierce_pulse_3dpla.fly2` | match | 20000 |
| `poisson/pipe_with_beam/pipe_beam_bfieldearth.fly2` | match | 4 |
| `poisson/pipe_with_beam/pipe_beam_bfieldself.fly2` | statistical match | 800 |
| `poisson/pipe_with_beam/pipe_beam_gravity.fly2` | match | 3 |
| `poisson/pipe_with_beam/pipe_beam_offaxis_poisson3d.fly2` | statistical match | 800 |
| `poisson/pipe_with_beam/pipe_beam_offaxis_repulsionbeam.fly2` | statistical match | 808 |
| `poisson/pipe_with_beam/pipe_beam_offaxis_repulsionbeamimage.fly2` | statistical match | 1600 |
| `poisson/pipe_with_beam/pipe_beam_onaxis_poisson2d.fly2` | statistical match | 800 |
| `poisson/pipe_with_beam/pipe_beam_onaxis_repulsionbeam.fly2` | statistical match | 800 |
| `poisson/pipe_with_beam/pipe_beam_onaxis_repulsioncoulomb.fly2` | statistical match | 4000 |
| `poisson_superfish/bmag.fly2` | match | 1 |
| `poisson_superfish/vmag.fly2` | match | 1 |
| `pseudopotential/trap.fly2` | statistical match | 800 |
| `quad/quad.fly2` | statistical match | 200 |
| `quad/quad1.fly2` | statistical match | 240 |
| `quad/quad_monolithic/quad_monolithic.fly2` | statistical match | 200 |
| `quad/quad_wgem/quad_wgem.fly2` | statistical match | 200 |
| `quickfield/bmag.fly2` | match | 1 |
| `quickfield/vmag.fly2` | match | 1 |
| `random_fly2/random.fly2` | statistical match | 160 |
| `repulsion/cylindrical_diode.fly2` | statistical match | 8000 |
| `repulsion/pierce.fly2` | statistical match | 3536 |
| `repulsion/pierce_coulomb.fly2` | statistical match | 4000 |
| `repulsion/repulsion1.fly2` | statistical match | 824 |
| `repulsion/repulsion1_coulombic.fly2` | statistical match | 8024 |
| `repulsion/repulsion2.fly2` | statistical match | 8024 |
| `repulsion/repulsion_mixed_beam.fly2` | statistical match | 1608 |
| `repulsion/repulsion_mixed_coulombic.fly2` | statistical match | 8008 |
| `repulsion/spherical_diode.fly2` | statistical match | 8000 |
| `secondary/secondary-test.fly2` | match | 1 |
| `secondary/secondary.fly2` | match | 20 |
| `secondary/secondary2.fly2` | statistical match | 400 |
| `secondary/tube.fly2` | statistical match | 160 |
| `solenoid/solenoid.fly2` | statistical match | 800 |
| `solenoid/toroid.fly2` | statistical match | 40 |
| `spectrum/spectrum.fly2` | statistical match | 400 |
| `surface_enhancement/quad.fly2` | statistical match | 160 |
| `surface_enhancement/sc.fly2` | match | 9 |
| `surface_enhancement/sc3d.fly2` | match | 9 |
| `surface_enhancement/tilted_plates.fly2` | match | 0 |
| `swirl/swirl.fly2` | statistical match | 80000 |
| `tandem_van_de_graaff/tandem_ppc3.fly2` | match | 5 |
| `test_sphere/surf.fly2` | statistical match | 400 |
| `waveform/plates.fly2` | match | 1 |

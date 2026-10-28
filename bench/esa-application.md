# ESA application example (summary values)

Public summary of a private electrostatic-analyzer study (CAD geometry and raw data are not in this repository). Values are copied from the study's stored results: channel_table.csv (32 log channels, axisymmetric CAD meridian, h=0.125 mm, 4,096 electrons per point), the fixed-code unchanged-hardware control (U=372 V, h=0.125 mm) and the selected design's energy validation (h=0.125 mm, 32,768 electrons). The README captures (docs/images/esa/esa_capture_*.png, 2026-10-28) use the same h=0.125 mm field fast-adjusted to the fitted channel nearest 4 keV (U = k_fit x 4079.26 eV) and the stored 4,096-electron source at that channel's transmission-peak energy; 60 of those electrons (30 detected, 30 stopped on a plate or support, chosen with a fixed seed) were re-flown with sampled trajectories, and the same 60 were loaded into the desktop app as a .ion file.

## Values

| key | value |
|---|---|
| esa_channels | 32 |
| esa_h_mm | 0.125 |
| esa_n_per_point | 4096 |
| esa_k_fit | 0.092326 |
| esa_k_geometric | 0.1 |
| esa_de_min | 0.09728616166687656 |
| esa_de_max | 0.22038385490825993 |
| esa_k_fixed_code | 0.09284696 |
| esa_k_design | 0.1243062 |
| esa_k_target | 0.124 |
| esa_design_gap_mm | 3.14844 |
| esa_capture_u_v | 376.6215610740203 |
| esa_capture_e_ev | 4120.0552 |
| esa_capture_rays | 60 |
| esa_capture_pass | 30 |
| esa_capture_fail | 30 |

## SIMION 2024 comparison

Values behind the README subsection "SIMION 2024와 같은 모델로 비교" (2026-10-28, private study; raw CSV/PA/STEP data are not in this repository). Both programs flew the same normalized staircase PA (h=0.125 mm, axisymmetric, y mirror) and the same 4,096 stored electron sources (relativistic) to the same detector plane at the channel-26 voltage (U=376.6 V, 4.12 keV). Potential difference: SIMION strict refine vs SimRep relative residual 1e-11, max over all 294,769 nodes. Outcome/position values: per-particle comparison of SIMION trajectory quality 209 vs SimRep RK45 1e-9 staircase (no cuts). Channel 1/16/32 centroid/FWHM (h=0.125 mm): 4,096 electrons x 121 energies per mode, FWHM by linear interpolation either side of the maximum. The h=0.0625 mm rows are the staircase and exact-surface scans plus the exact-surface rerun with the insulating-support collision group removed (same 495,616 electrons per channel); the channel-32 bootstrap standard deviation is from the channel-32 FWHM resampling study. Fair timing: channel 26, 4,096 electrons, staircase PA, SIMION `--programs 0`, recording off, TQual 9, `--num-threads 1` vs SimRep `--no-cuts` RK45 1e-3 `compute_ms`; 3 sequential runs each, median; another user's process held one core at 100% during the runs. Matched accuracy: detector-position RMS over the common hits vs a SimRep 1e-11 staircase reference. The SIMION capture (docs/images/esa/esa_simion_capture.png) is a `simion.print_screen()` PNG (scale 3, cropped to the drawing area) after flying 120 of the same electrons (all 60 SIMION detector hits, 60 evenly chosen non-hits) at TQual 9 in the SIMION 2024 GUI; an in-run check counted 60 hits and 0 colour/outcome mismatches.

| key | value |
|---|---|
| esa_cmp_n | 4096 |
| esa_cmp_same | 4092 |
| esa_cmp_common_hits | 57 |
| esa_cmp_only_simion | 3 |
| esa_cmp_only_stair | 1 |
| esa_cmp_hit_rms_mm | 0.1330670241503671 |
| esa_cmp_pot_max_v | 2.0255901773680307e-05 |
| esa_cmp_simion_capture_rays | 120 |
| esa_cmp_simion_capture_hits | 60 |
| esa_cmp_ch1_simion_centroid_ev | 1.0016430412371136 |
| esa_cmp_ch1_simion_fwhm_ev | 0.27212500000000006 |
| esa_cmp_ch1_stair_centroid_ev | 0.9986930693069307 |
| esa_cmp_ch1_stair_fwhm_ev | 0.24986013986013988 |
| esa_cmp_ch1_exact_centroid_ev | 0.9931454545454546 |
| esa_cmp_ch1_exact_fwhm_ev | 0.17631818181818193 |
| esa_cmp_ch16_simion_centroid_ev | 147.06341273494985 |
| esa_cmp_ch16_simion_fwhm_ev | 39.74827493151518 |
| esa_cmp_ch16_stair_centroid_ev | 146.61749022613958 |
| esa_cmp_ch16_stair_fwhm_ev | 36.71815232897012 |
| esa_cmp_ch16_exact_centroid_ev | 145.74451086632772 |
| esa_cmp_ch16_exact_fwhm_ev | 25.77770966499554 |
| esa_cmp_ch32_simion_centroid_ev | 30943.879472693032 |
| esa_cmp_ch32_simion_fwhm_ev | 8625.0 |
| esa_cmp_ch32_stair_centroid_ev | 30791.31274131274 |
| esa_cmp_ch32_stair_fwhm_ev | 7928.571428571435 |
| esa_cmp_ch32_exact_centroid_ev | 30603.214285714286 |
| esa_cmp_ch32_exact_fwhm_ev | 6962.5 |
| esa_g62_h_mm | 0.0625 |
| esa_g62_n | 495616 |
| esa_g62_ch1_stair_fwhm_ev | 0.2610769230769231 |
| esa_g62_ch1_stair_hits | 1529 |
| esa_g62_ch1_exact_hits | 1101 |
| esa_g62_ch1_noins_fwhm_ev | 0.2575833333333335 |
| esa_g62_ch1_noins_hits | 1531 |
| esa_g62_ch16_stair_fwhm_ev | 38.208212618670515 |
| esa_g62_ch16_stair_hits | 1527 |
| esa_g62_ch16_exact_hits | 1114 |
| esa_g62_ch16_noins_fwhm_ev | 38.06968849529292 |
| esa_g62_ch16_noins_hits | 1537 |
| esa_g62_ch32_stair_fwhm_ev | 7871.250000000007 |
| esa_g62_ch32_stair_hits | 1591 |
| esa_g62_ch32_exact_hits | 1121 |
| esa_g62_ch32_noins_fwhm_ev | 7980.000000000007 |
| esa_g62_ch32_noins_hits | 1580 |
| esa_g62_ch32_exact_fwhm_bootstrap_sd_ev | 1728.0361006951457 |
| esa_fair_simion_s | 8.08 |
| esa_fair_simrep_1t_s | 0.4754 |
| esa_fair_simrep_16t_s | 0.0397 |
| esa_fair_threads_single | 1 |
| esa_fair_threads_multi | 16 |
| esa_fair_runs | 3 |
| esa_fair_simion_rms_mm | 0.1304284643472587 |
| esa_fair_simrep_rms_mm | 0.12091328468539693 |
| esa_g62_ch1_exact_fwhm_ev | 0.17401515151515168 |
| esa_g62_ch16_exact_fwhm_ev | 25.778820821065437 |
| esa_g62_ch32_exact_fwhm_ev | 5611.36363636364 |

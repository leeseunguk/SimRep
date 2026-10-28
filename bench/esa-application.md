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

# Examples

Four small end-to-end examples, each under a second of compute on CPU.
They only need the `simrep` Python package (or the `simrep` CLI for the
shell variants), NumPy, and the files in `examples/data/`. No SIMION
installation is required. Matplotlib is optional in `einzel_tof.py`; the
histogram is skipped when it is not installed.

Setup (from the repository root):

```sh
python3 -m venv .venv && . .venv/bin/activate
python -m pip install maturin numpy
maturin develop --release --manifest-path crates/simrep-py/Cargo.toml
```

Run every example from the repository root so the `examples/data` paths
resolve. All of them print a single `KEY ...` result line; the pytest in
`crates/simrep-py/tests/test_examples.py` runs each script and checks
that line.

## einzel_tof.py: GEM -> PA -> refine -> fly -> TOF

```sh
python examples/einzel_tof.py
```

Output (Ryzen 5950X, CPU device):

```text
refine: 32 ions, electrodes [1, 2, 3], compute_ms=7.05
KEY tof_us mean=0.831355 std=0.000001 detected=32/32
fly compute_ms=2.79 setup_ms=9.54
matplotlib not installed; skipping histogram
```

CLI equivalent:

```sh
simrep gem2pa examples/data/einzel.gem einzel.pa#
simrep refine einzel.pa#
simrep fly --pa einzel.pa0 --particles examples/data/ions.ion \
    --voltage 1=0,2=60,3=0 --plane 1,0,0,58 --duration-us 2000 \
    --output results.csv
```

`ions.ion` rows are `birth,mass,q,x,y,z,azimuth_deg,elevation_deg,KE,CWF`.
The Python example uses 1000 eV ions; the committed `.ion` file uses
500 eV to match the sweep example below.

## voltage_sweep.py: fast adjust over lens voltage

```sh
python examples/voltage_sweep.py
```

```text
refine compute_ms=6.99
lens_voltage_V,transmission_plane,transmission_aperture
-400,1.0000,0.9375
-200,1.0000,0.5625
-100,1.0000,0.4375
0,1.0000,0.3438
100,1.0000,0.4375
200,1.0000,0.7188
400,1.0000,0.3438
KEY best_voltage=-400 aperture_transmission=0.9375
```

The einzel lens shows its expected two-sided focus curve: transmission
through a 1 mm detector aperture peaks at -400 V and rises again toward
+200 V. `field.fast_adjust` is a weighted sum of unit-voltage bases; the
solver runs once.

CLI equivalent:

```sh
simrep gem2pa examples/data/einzel.gem einzel.pa#
simrep refine einzel.pa#
for V in -400 -200 -100 0 100 200 400; do
    simrep fly --pa einzel.pa0 --particles examples/data/ions.ion \
        --voltage 1=0,2=$V,3=0 --plane 1,0,0,58 --duration-us 4000
done
```

## exact_surface.py: refine-geometry cuts vs staircase

```sh
python examples/exact_surface.py
```

```text
refine-geometry compute_ms=138.7
exact surface:   19 electrode hits, hit radius 8.0000 mm (surface at 8.0)
staircase:       17 electrode hits, hit radius 7.6361 mm
KEY staircase_hit_radius=7.6361 exact_hit_radius=8.0000 offset_mm=0.3639
```

Staircase collisions stop ions at voxel cell faces, on average 0.36 mm
inside the 8 mm sphere, and two grazing ions slip between cells and reach
the detector. Exact-surface flight puts every hit at radius 8.0000.

CLI equivalent:

```sh
simrep refine-geometry examples/data/electrodes.gem \
    --electrode 1=1:0 --electrode 2=2:0 --out e
simrep fly --pa e.pa0 --particles examples/data/surf_ions.ion \
    --plane 1,0,0,32 --duration-us 3000            # picks up e.cuts.json
simrep fly --pa e.pa0 --particles examples/data/surf_ions.ion \
    --plane 1,0,0,32 --duration-us 3000 --no-cuts  # staircase
```

## magnetic_cyclotron.py: uniform B vs analytic gyroradius

```sh
python examples/magnetic_cyclotron.py
```

```text
analytic: r=9.105721 mm, v=21.964213 mm/us, T=2.604825 us
measured: r=9.105721 mm from 261 samples
KEY radius_mm analytic=9.105721 measured=9.105721 error_ppm=-0.00
```

r = m v / (q B) for a 100 eV, 40 amu ion in 1 T agrees with the measured
orbit to sub-ppm. To check an imported solver map instead, export a
regular-grid vector map and use `fly --bfield-map` (see
`docs/bfield-import.md`).

CLI equivalent:

```sh
simrep fly --particles examples/data/cyclotron.ion --bfield 0,0,10000 \
    --bounds -60,-60,-5,60,60,5 --duration-us 6 --allow-time-limit
```

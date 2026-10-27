//! Investigation-only measurements; does not change solver or reconstruction.
use simrep_core::PotentialArray;
use simrep_fly::{
    AnalyticField, Field, FieldSample, FlightOptions, GridField, Particle, SplatReason, StopPlane,
    fly,
};
use simrep_geom::{Electrode, Point, Sdf, SignedDistance};
use simrep_solve::{
    cutcell::{CutCells, surface_fraction},
    mg::{Geometry, Grid, Options},
};

const A: f64 = 0.31;
const B: f64 = 0.83;

#[derive(Default)]
struct Norm {
    max: f64,
    squares: f64,
    count: usize,
}

impl Norm {
    fn add(&mut self, value: f64) {
        self.max = self.max.max(value.abs());
        self.squares += value * value;
        self.count += 1;
    }

    fn print(&self, label: &str) {
        print!(
            " {label}={:.9e},{:.9e},{}",
            self.max,
            (self.squares / self.count.max(1) as f64).sqrt(),
            self.count
        );
    }
}

struct NoSnap(Sdf);

impl SignedDistance for NoSnap {
    fn distance(&self, point: Point) -> f64 {
        self.0.distance(point)
    }

    fn segment_distance_bounds(&self, from: Point, to: Point) -> Option<[f64; 2]> {
        self.0.segment_distance_bounds(from, to)
    }

    fn segment_single_interval(&self, from: Point, to: Point, level: f64) -> bool {
        self.0.segment_single_interval(from, to, level)
    }

    fn conductor_mask(
        &self,
        dims: [usize; 3],
        origin: Point,
        spacing: Point,
        _threshold: f64,
        mask: &mut [bool],
    ) -> Result<(), &'static str> {
        self.0.conductor_mask(dims, origin, spacing, 0.0, mask)
    }

    fn mark_near_nodes(
        &self,
        dims: [usize; 3],
        origin: Point,
        spacing: Point,
        band: f64,
        near: &mut [bool],
    ) {
        self.0.mark_near_nodes(dims, origin, spacing, band, near);
    }
}

fn exact(p: Point, sphere: bool) -> (f64, Point) {
    let dimensions = if sphere { 3 } else { 2 };
    let r = p[..dimensions]
        .iter()
        .map(|x| x * x)
        .sum::<f64>()
        .sqrt()
        .max(1e-3);
    let (v, derivative) = if sphere {
        (
            (1.0 / r - 1.0 / B) / (1.0 / A - 1.0 / B),
            -1.0 / (r * r * (1.0 / A - 1.0 / B)),
        )
    } else {
        ((B / r).ln() / (B / A).ln(), -1.0 / (r * (B / A).ln()))
    };
    (v, std::array::from_fn(|a| -derivative * p[a] / r))
}

fn prepare(n: usize, sphere: bool, phase: f64, variant: &str) -> CutCells {
    let h = 2.0 / (n - 1) as f64;
    let center = [phase * h, 0.0, 0.0];
    let shape = |radius| {
        if sphere {
            Sdf::Sphere { center, radius }
        } else {
            Sdf::Cylinder {
                center,
                radius,
                axis: 2,
            }
        }
    };
    let outside = Sdf::Box {
        center: [0.0; 3],
        half_extent: [4.0; 3],
    }
    .difference(shape(B));
    let electrodes = if variant == "no-snap" {
        [
            Electrode::new(1.0, NoSnap(shape(A))),
            Electrode::new(0.0, NoSnap(outside)),
        ]
    } else {
        [Electrode::new(1.0, shape(A)), Electrode::new(0.0, outside)]
    };
    CutCells::new_with_root_tolerance(
        Grid {
            dims: [n, n, if sphere { n } else { 1 }],
            spacing: [h; 3],
            geometry: Geometry::Planar,
        },
        [-1.0, -1.0, if sphere { -1.0 } else { 0.0 }],
        &electrodes,
        if variant == "default" {
            f64::INFINITY
        } else {
            1e-14
        },
    )
    .unwrap()
}

fn nodal(cuts: &CutCells, values: &[f64], sphere: bool, phase: f64, variant: &str) {
    let n = cuts.grid.dims[0];
    let h = cuts.grid.spacing[0];
    let dimensions = if sphere { 3 } else { 2 };
    let point = |i: usize| {
        let c = [i % n, i / n % n, i / (n * n)];
        std::array::from_fn(|a| {
            cuts.origin[a] + c[a] as f64 * h - if a == 0 { phase * h } else { 0.0 }
        })
    };
    let analytic: Vec<_> = (0..values.len())
        .map(|i| {
            if cuts.fixed[i] {
                cuts.values[i]
            } else {
                exact(point(i), sphere).0
            }
        })
        .collect();
    let mut v_all = Norm::default();
    let mut v_snap = Norm::default();
    let mut v_regions: [Norm; 3] = Default::default();
    let mut e_regions: [Norm; 3] = Default::default();
    let mut e_exact: [Norm; 3] = Default::default();
    for (i, &v) in values.iter().enumerate() {
        let p = point(i);
        let r = p[..dimensions].iter().map(|x| x * x).sum::<f64>().sqrt();
        let d = (r - A).min(B - r);
        if d <= 0.0 {
            continue;
        }
        let (target, electric) = exact(p, sphere);
        v_all.add(v - target);
        if cuts.fixed[i] {
            v_snap.add(v - target);
            continue;
        }
        let band = usize::from(d >= 2.0 * h);
        let field = cuts.electric_field(values, i).unwrap();
        let field_exact = cuts.electric_field(&analytic, i).unwrap();
        for region in [Some(band), (d >= 0.1).then_some(2)].into_iter().flatten() {
            v_regions[region].add(v - target);
            for a in 0..dimensions {
                e_regions[region].add(field[a] - electric[a]);
                e_exact[region].add(field_exact[a] - electric[a]);
            }
        }
    }
    print!("NODE sphere={sphere} n={n} phase={phase} variant={variant}");
    v_all.print("v_all");
    v_snap.print("v_snap");
    for (i, label) in ["band", "interior", "fixed-interior"]
        .into_iter()
        .enumerate()
    {
        v_regions[i].print(&format!("v_{label}"));
        e_regions[i].print(&format!("e_{label}"));
        e_exact[i].print(&format!("analytic_e_{label}"));
    }
    println!();
}

fn slice(cuts: &CutCells, values: &[f64], sphere: bool, phase: f64, data: &str) -> GridField {
    let n = cuts.grid.dims[0];
    let h = cuts.grid.spacing[0];
    let mut pa = PotentialArray::new(n, n, 1);
    pa.dx_mm = h;
    pa.dy_mm = h;
    let z = if sphere { n / 2 } else { 0 };
    for y in 0..n {
        for x in 0..n {
            let i = y * n + x;
            let source = z * n * n + i;
            let p = [-1.0 + x as f64 * h - phase * h, -1.0 + y as f64 * h, 0.0];
            let radius = p[0].hypot(p[1]);
            // refine-geometry writes physical flags, not solver-snapped flags.
            pa.electrodes[i] = radius <= A || radius >= B;
            pa.potentials[i] = if data == "solved" || data == "ghost" {
                values[source]
            } else {
                let v = exact(p, sphere).0;
                if data == "clamped" {
                    v.clamp(0.0, 1.0)
                } else {
                    v
                }
            };
        }
    }
    if data == "ghost" {
        // Prototype only: give first-layer metal knots a vacuum continuation.
        // A surface voltage and a vacuum knot at least one full step from the
        // surface define each linear extrapolation, avoiding tiny-cut division.
        let inner = if sphere {
            Sdf::Sphere {
                center: [0.0; 3],
                radius: A,
            }
        } else {
            Sdf::Cylinder {
                center: [0.0; 3],
                radius: A,
                axis: 2,
            }
        };
        let outer_shape = if sphere {
            Sdf::Sphere {
                center: [0.0; 3],
                radius: B,
            }
        } else {
            Sdf::Cylinder {
                center: [0.0; 3],
                radius: B,
                axis: 2,
            }
        };
        let outer = Sdf::Box {
            center: [0.0; 3],
            half_extent: [4.0; 3],
        }
        .difference(outer_shape);
        let original = pa.potentials.clone();
        let inside = |c| (0..n as isize).contains(&c);
        for y in 0..n {
            for x in 0..n {
                let j = y * n + x;
                if !pa.electrodes[j] {
                    continue;
                }
                let p = [-1.0 + x as f64 * h - phase * h, -1.0 + y as f64 * h, 0.0];
                let (shape, voltage) = if p[0].hypot(p[1]) <= A {
                    (&inner, 1.0)
                } else {
                    (&outer, 0.0)
                };
                let mut sum = 0.0;
                let mut count = 0;
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        let (ix, iy) = (x as isize + dx, y as isize + dy);
                        let (kx, ky) = (ix + dx, iy + dy);
                        if (dx == 0 && dy == 0) || ![ix, iy, kx, ky].into_iter().all(inside) {
                            continue;
                        }
                        let i = iy as usize * n + ix as usize;
                        let k = ky as usize * n + kx as usize;
                        if pa.electrodes[i] || pa.electrodes[k] || cuts.fixed[z * n * n + k] {
                            continue;
                        }
                        let from = [-1.0 + ix as f64 * h - phase * h, -1.0 + iy as f64 * h, 0.0];
                        if let Some(theta) = surface_fraction(shape, from, p, 1e-14) {
                            sum +=
                                voltage + (original[k] - voltage) * (theta - 1.0) / (theta + 1.0);
                            count += 1;
                        }
                    }
                }
                if count > 0 {
                    pa.potentials[j] = sum / count as f64;
                }
            }
        }
    }
    GridField::new_compact(pa).unwrap()
}

fn offnode(field: &GridField, n: usize, sphere: bool, phase: f64, data: &str) {
    let h = 2.0 / (n - 1) as f64;
    for region in ["band", "fixed-interior"] {
        let radii = if region == "band" {
            [0.1, 0.4, 1.1, 1.9]
                .into_iter()
                .flat_map(|d| [A + d * h, B - d * h])
                .filter(|r| *r > A && *r < B)
                .collect::<Vec<_>>()
        } else {
            vec![0.42, 0.47, 0.57, 0.67, 0.72]
        };
        let mut potential = Norm::default();
        let mut electric = Norm::default();
        for r in radii {
            for angle in 0..256 {
                let phi = std::f64::consts::TAU * (angle as f64 + 0.371) / 256.0;
                let p = [r * phi.cos(), r * phi.sin(), 0.0];
                let sample = field.sample([p[0] + 1.0 + phase * h, p[1] + 1.0, 0.0], 0.0);
                let (v, e) = exact(p, sphere);
                potential.add(sample.potential_v - v);
                for (actual, target) in sample.electric_v_per_mm[..2].iter().zip(e) {
                    electric.add(actual - target);
                }
            }
        }
        print!("OFFNODE sphere={sphere} n={n} phase={phase} data={data} region={region}");
        potential.print("v");
        electric.print("e");
        println!();
    }
}

struct Shifted<'a>(&'a GridField, f64);

impl Field for Shifted<'_> {
    fn sample(&self, p: Point, t: f64) -> FieldSample {
        self.0.sample([p[0] + 1.0 + self.1, p[1] + 1.0, 0.0], t)
    }
}

fn centroid(field: &impl Field, radius: f64) -> f64 {
    let reference = 1.0 / (2.0 * radius * (1.0 / A - 1.0 / B));
    let options = FlightOptions {
        automatic_tolerance: false,
        initial_step_us: 1e-4,
        max_step_us: 0.002,
        duration_us: 0.5,
        relative_tolerance: 1e-11,
        position_tolerance_mm: 1e-11,
        velocity_tolerance_mm_per_us: 1e-11,
        sample_every: std::env::var_os("DIAG_TRACE").map(|_| 1),
        stop_plane: Some(StopPlane {
            axis: 0,
            coordinate_mm: 0.0,
        }),
        ..Default::default()
    };
    let mut energies = [0.0; 2];
    let mut clearance = f64::INFINITY;
    for (edge, energy) in energies.iter_mut().enumerate() {
        let target = radius + if edge == 0 { -0.0025 } else { 0.0025 };
        let (mut lo, mut hi) = (0.9 * reference, 1.1 * reference);
        for _ in 0..30 {
            let mid = (lo + hi) * 0.5;
            let particle =
                Particle::from_energy(1.0, -1.0, [radius, 0.0, 0.0], mid, [0.0, 1.0, 0.0], 0.0)
                    .unwrap();
            let result = fly(field, &particle, &options).unwrap();
            assert_eq!(result.splat_reason, SplatReason::StopPlane);
            if let Some(trajectory) = &result.trajectory {
                for state in trajectory {
                    let r = state.position_mm[0].hypot(state.position_mm[1]);
                    clearance = clearance.min((r - A).min(B - r));
                }
            }
            if result.final_state.position_mm[1] < target {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        *energy = (lo + hi) * 0.5;
    }
    if clearance.is_finite() {
        assert!(clearance > 0.0, "analyzer ray entered a conductor");
        println!("TRACE radius={radius} minimum_clearance_mm={clearance:.9e}");
    }
    (energies[0] + energies[1]) * 0.5
}

fn main() {
    let args: Vec<_> = std::env::args().collect();
    let variant = args.get(1).map(String::as_str).unwrap_or("default");
    let sizes: Vec<usize> = args[2..].iter().map(|s| s.parse().unwrap()).collect();
    for sphere in [true, false] {
        if std::env::var("DIAG_SHAPE").is_ok_and(|shape| (shape == "sphere") != sphere) {
            continue;
        }
        for &n in &sizes {
            let options = Options {
                relative_tolerance: 0.0,
                // The 257^3 absolute residual reaches a binary64 floor near
                // 1.3e-10. This diagnostic tolerance remains far below its
                // O(h^2) potential error; no registered gate is changed.
                absolute_tolerance: if n >= 257 { 1e-8 } else { 1e-10 },
                max_cycles: 200,
                ..Default::default()
            };
            for phase in [0.0, 0.25, 0.5, 0.75] {
                let cuts = prepare(n, sphere, phase, variant);
                let mut values = cuts.values.clone();
                let report = cuts.solve(&mut values, options).unwrap();
                assert!(
                    report.converged,
                    "n={n} phase={phase} {:?}",
                    report.residual_history
                );
                nodal(&cuts, &values, sphere, phase, variant);
                if variant != "default" || phase != 0.0 {
                    continue;
                }
                for data in ["solved", "clamped", "continuation", "ghost"] {
                    if std::env::var("DIAG_DATA").is_ok_and(|selected| selected != data) {
                        continue;
                    }
                    let field = slice(&cuts, &values, sphere, phase, data);
                    offnode(&field, n, sphere, phase, data);
                    if sphere && n >= 33 {
                        for radius in [0.57, 0.34] {
                            let center =
                                centroid(&Shifted(&field, phase * cuts.grid.spacing[0]), radius);
                            let reference = 1.0 / (2.0 * radius * (1.0 / A - 1.0 / B));
                            println!(
                                "FLIGHT n={n} data={data} radius={radius} centroid={center:.12e} error={:.12e}",
                                center - reference
                            );
                        }
                    }
                }
            }
        }
    }
    let analytic = AnalyticField(|p, _| {
        let (v, e) = exact(p, true);
        FieldSample {
            potential_v: v,
            electric_v_per_mm: e,
            magnetic_gauss: [0.0; 3],
        }
    });
    for radius in [0.57, 0.34] {
        let center = centroid(&analytic, radius);
        let reference = 1.0 / (2.0 * radius * (1.0 / A - 1.0 / B));
        println!(
            "REFERENCE radius={radius} centroid={center:.12e} error={:.12e}",
            center - reference
        );
    }
}

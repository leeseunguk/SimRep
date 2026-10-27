//! Compute/preparation comparison with identical exact quadrupole collisions.
use std::{sync::Arc, time::Instant};

use simrep_core::PotentialArray;
use simrep_fly::{
    Field, FieldSample, FlightOptions, GridField, Particle, SplatReason, Vector, fly_many,
    surface::SurfaceModel,
};
use simrep_geom::{Electrode, Sdf};
use simrep_solve::{
    cutcell::CutCells,
    mg::{Geometry, Grid, Options},
};

struct Exact<'a> {
    grid: &'a GridField,
    surface: &'a SurfaceModel,
}

impl Field for Exact<'_> {
    fn sample(&self, p: Vector, t: f64) -> FieldSample {
        self.grid.sample(p, t)
    }

    fn spatial_scale_mm(&self) -> Option<f64> {
        self.grid.spatial_scale_mm()
    }

    fn status(&self, p: Vector) -> Option<SplatReason> {
        self.surface
            .membership([p[0] - 2.0, p[1] - 2.0, p[2]])
            .map(|number| SplatReason::SurfaceElectrode { number })
    }

    fn intersection(&self, a: Vector, b: Vector) -> Option<(f64, SplatReason)> {
        self.surface
            .first_hit(
                [a[0] - 2.0, a[1] - 2.0, a[2]],
                [b[0] - 2.0, b[1] - 2.0, b[2]],
            )
            .map(|(t, number)| (t, SplatReason::SurfaceElectrode { number }))
    }
}

fn main() {
    let n = 65;
    let h = 4.0 / (n - 1) as f64;
    let mut shapes = Vec::new();
    for center in [
        [1.35, 0.0, 0.0],
        [-1.35, 0.0, 0.0],
        [0.0, 1.35, 0.0],
        [0.0, -1.35, 0.0],
    ] {
        shapes.push(Sdf::Cylinder {
            center,
            radius: 0.35,
            axis: 2,
        });
    }
    shapes.push(
        Sdf::Box {
            center: [0.0; 3],
            half_extent: [10.0; 3],
        }
        .difference(Sdf::Cylinder {
            center: [0.0; 3],
            radius: 1.95,
            axis: 2,
        }),
    );
    let electrodes: Vec<_> = shapes
        .iter()
        .enumerate()
        .map(|(i, shape)| {
            Electrode::new(
                if i < 2 {
                    1.0
                } else if i < 4 {
                    -1.0
                } else {
                    0.0
                },
                shape.clone(),
            )
        })
        .collect();
    let cuts = CutCells::new_with_root_tolerance(
        Grid {
            dims: [n, n, 1],
            spacing: [h; 3],
            geometry: Geometry::Planar,
        },
        [-2.0, -2.0, 0.0],
        &electrodes,
        1e-13,
    )
    .unwrap();
    let mut values = cuts.values.clone();
    let report = cuts
        .solve(
            &mut values,
            Options {
                relative_tolerance: 0.0,
                absolute_tolerance: 1e-10,
                max_cycles: 200,
                ..Default::default()
            },
        )
        .unwrap();
    assert!(report.converged);
    let mut array = PotentialArray::new(n, n, 1);
    array.dx_mm = h;
    array.dy_mm = h;
    array.potentials = values;
    let surface = SurfaceModel::from_sdfs(
        shapes
            .into_iter()
            .enumerate()
            .map(|(i, shape)| (i + 1, Arc::new(shape) as Arc<_>))
            .collect(),
        1e-12,
    )
    .unwrap()
    .with_pa_domain(&array, [-2.0, -2.0, 0.0])
    .unwrap();
    for i in 0..array.potentials.len() {
        array.electrodes[i] = surface
            .membership([-2.0 + (i % n) as f64 * h, -2.0 + (i / n) as f64 * h, 0.0])
            .is_some();
    }
    let ions: Vec<_> = (0..1000)
        .map(|i| {
            Particle::new(
                40.0,
                1.0,
                [
                    2.0 + 0.6 * ((i % 40) as f64 / 39.0 - 0.5),
                    2.0 + 0.6 * ((i / 40) as f64 / 24.0 - 0.5),
                    0.0,
                ],
                [0.0, 0.0, 30.0],
                0.0,
            )
            .unwrap()
        })
        .collect();
    let options = FlightOptions {
        duration_us: 0.2,
        automatic_tolerance: false,
        relative_tolerance: 1e-10,
        position_tolerance_mm: 1e-10,
        velocity_tolerance_mm_per_us: 1e-10,
        ..Default::default()
    };
    for filled in [false, true] {
        for repeat in 0..3 {
            let start = Instant::now();
            let grid = GridField::new_compact(array.clone()).unwrap();
            let grid = if filled {
                grid.with_surface(&surface).unwrap()
            } else {
                grid
            };
            let prep_ms = start.elapsed().as_secs_f64() * 1000.0;
            let field = Exact {
                grid: &grid,
                surface: &surface,
            };
            let start = Instant::now();
            let results = fly_many(&field, &ions, &options).unwrap();
            let compute_ms = start.elapsed().as_secs_f64() * 1000.0;
            assert!(
                results
                    .iter()
                    .all(|r| r.splat_reason == SplatReason::TimeLimit)
            );
            println!(
                "COST filled={filled} repeat={repeat} ions={} prep_ms={prep_ms:.6} compute_ms={compute_ms:.6} ghost={:?}",
                ions.len(),
                grid.ghost_fill_report()
            );
        }
    }
}

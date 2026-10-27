use std::sync::Arc;

use simrep_core::{PotentialArray, validation::record_order};
use simrep_fly::{
    Field, FlightOptions, GridField, InstanceField, Particle, VoltageExpression, fly,
    surface::SurfaceModel,
};
use simrep_geom::Sdf;

const AXES: [[f64; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
const INNER: f64 = 0.31;
const OUTER: f64 = 0.83;

fn annulus(n: usize, sphere: bool) -> (PotentialArray, Arc<SurfaceModel>) {
    let h = 2.0 / (n - 1) as f64;
    let mut array = PotentialArray::new(n, n, 1);
    array.dx_mm = h;
    array.dy_mm = h;
    for y in 0..n {
        for x in 0..n {
            let i = y * n + x;
            let r = (-1.0 + x as f64 * h).hypot(-1.0 + y as f64 * h).max(1e-3);
            array.electrodes[i] = !(INNER..OUTER).contains(&r);
            array.potentials[i] = exact(r, sphere).0.clamp(0.0, 1.0);
        }
    }
    let shape = |radius| {
        if sphere {
            Sdf::Sphere {
                center: [0.0; 3],
                radius,
            }
        } else {
            Sdf::Cylinder {
                center: [0.0; 3],
                radius,
                axis: 2,
            }
        }
    };
    let outside = Sdf::Box {
        center: [0.0; 3],
        half_extent: [4.0; 3],
    }
    .difference(shape(OUTER));
    let surface = SurfaceModel::from_sdfs(
        vec![(1, Arc::new(shape(INNER))), (2, Arc::new(outside))],
        1e-13,
    )
    .unwrap()
    .with_pa_domain(&array, [-1.0, -1.0, 0.0])
    .unwrap();
    (array, Arc::new(surface))
}

fn exact(r: f64, sphere: bool) -> (f64, f64) {
    if sphere {
        (
            (1.0 / r - 1.0 / OUTER) / (1.0 / INNER - 1.0 / OUTER),
            1.0 / (r * r * (1.0 / INNER - 1.0 / OUTER)),
        )
    } else {
        (
            (OUTER / r).ln() / (OUTER / INNER).ln(),
            1.0 / (r * (OUTER / INNER).ln()),
        )
    }
}

fn band_errors(n: usize, sphere: bool) -> [f64; 4] {
    let (array, surface) = annulus(n, sphere);
    let field = GridField::new_compact(array)
        .unwrap()
        .with_surface(&surface)
        .unwrap();
    let h = 2.0 / (n - 1) as f64;
    let mut max = [0.0_f64; 2];
    let mut squares = [0.0; 2];
    let mut count = [0; 2];
    for distance in [0.1, 0.4, 1.1, 1.9] {
        for radius in [INNER + distance * h, OUTER - distance * h] {
            for angle in 0..128 {
                let phi = std::f64::consts::TAU * (angle as f64 + 0.371) / 128.0;
                let direction = [phi.cos(), phi.sin()];
                let sample = field.sample(
                    [
                        1.0 + radius * direction[0],
                        1.0 + radius * direction[1],
                        0.0,
                    ],
                    0.0,
                );
                let (potential, electric) = exact(radius, sphere);
                let error = (sample.potential_v - potential).abs();
                max[0] = max[0].max(error);
                squares[0] += error * error;
                count[0] += 1;
                for (actual, component) in sample.electric_v_per_mm[..2].iter().zip(direction) {
                    let error = (actual - electric * component).abs();
                    max[1] = max[1].max(error);
                    squares[1] += error * error;
                    count[1] += 1;
                }
            }
        }
    }
    [
        max[0],
        max[1],
        (squares[0] / count[0] as f64).sqrt(),
        (squares[1] / count[1] as f64).sqrt(),
    ]
}

#[test]
fn exact_surface_band_reconstruction_converges() {
    for (sphere, name) in [(true, "sphere"), (false, "cylinder")] {
        let errors = [33, 65, 129].map(|n| band_errors(n, sphere));
        for (i, metric, order) in [
            (0, "potential-max", 4.0),
            (1, "field-max", 3.0),
            (2, "potential-rms", 4.0),
            (3, "field-rms", 3.0),
        ] {
            let levels = errors.map(|e| e[i]);
            record_order(
                &format!("mms-cut-flight-{name}-{metric}"),
                "equation (7); exact vacuum data, two-cell physical surface band",
                levels,
                order,
            );
            let measured = (levels[0] / levels[2]).ln() / 4.0_f64.ln();
            assert!(
                measured > order - 1.0,
                "{name} {metric}: {levels:?}, order={measured}"
            );
        }
    }
}

#[test]
fn surface_attachment_fills_background_and_bases_in_either_order() {
    let (array, surface) = annulus(33, false);
    let base = GridField::new_compact(array).unwrap();
    let expression = VoltageExpression::parse("2.3", &Default::default()).unwrap();
    let instance = || InstanceField::new(base.clone(), [0.0; 3], AXES, 1.0, None, None).unwrap();
    let before = instance()
        .with_voltage_expressions(vec![(base.clone(), expression.clone())])
        .unwrap()
        .with_surface(surface.clone())
        .unwrap();
    let after = instance()
        .with_surface(surface)
        .unwrap()
        .with_voltage_expressions(vec![(base, expression)])
        .unwrap();
    assert_eq!(before.ghost_fill_report(), after.ghost_fill_report());
    for p in [[1.315, 1.0, 0.0], [1.81, 1.02, 0.0], [1.43, 1.17, 0.0]] {
        assert_eq!(before.sample(p, 0.0), after.sample(p, 0.0));
    }
    assert_eq!(
        before.ghost_fill_report().unwrap().skipped_electrodes,
        Vec::<usize>::new()
    );
}

#[test]
fn surface_attachment_rebuilds_adaptive_base_and_patch_in_either_order() {
    use simrep_fly::adaptive::{AdaptiveManifest, AdaptivePatchManifest};
    let (array, surface) = annulus(33, false);
    let base = GridField::new_compact(array).unwrap();
    let origin = [0.1875, -0.1875, 0.0];
    let offset = [1.1875, 0.8125, 0.0];
    let mut patch = PotentialArray::new(17, 13, 1);
    patch.dx_mm = 0.03125;
    patch.dy_mm = 0.03125;
    for i in 0..patch.potentials.len() {
        let r = (origin[0] + (i % patch.nx) as f64 * patch.dx_mm)
            .hypot(origin[1] + (i / patch.nx) as f64 * patch.dy_mm);
        patch.electrodes[i] = !(INNER..OUTER).contains(&r);
        patch.potentials[i] = exact(r, false).0.clamp(0.0, 1.0);
    }
    let manifest = AdaptiveManifest {
        schema_version: 1,
        base: "base.pa0".into(),
        origin_mm: [-1.0, -1.0, 0.0],
        spacing_mm: [0.0625, 0.0625, 1.0],
        reference_voltage: 10000.0,
        electrode_voltages: Default::default(),
        patches: vec![AdaptivePatchManifest {
            level: 1,
            factor: 2,
            bounds_mm: [[0.25, -0.125, 0.0], [0.625, 0.125, 0.0]],
            origin_mm: origin,
            spacing_mm: [0.03125, 0.03125, 1.0],
            file: "patch.pa0".into(),
            ghost_layers: 2,
        }],
    };
    let raw_patch = GridField::new(patch.clone()).unwrap();
    let patch_surface = SurfaceModel::from_sdfs(
        vec![
            (
                1,
                Arc::new(Sdf::Cylinder {
                    center: [0.0; 3],
                    radius: INNER,
                    axis: 2,
                }),
            ),
            (
                2,
                Arc::new(
                    Sdf::Box {
                        center: [0.0; 3],
                        half_extent: [4.0; 3],
                    }
                    .difference(Sdf::Cylinder {
                        center: [0.0; 3],
                        radius: OUTER,
                        axis: 2,
                    }),
                ),
            ),
        ],
        1e-13,
    )
    .unwrap()
    .with_pa_domain(&patch, origin)
    .unwrap();
    let expected = raw_patch.clone().with_surface(&patch_surface).unwrap();
    let adaptive =
        || simrep_fly::AdaptiveField::new(base.clone(), &manifest, vec![patch.clone()]).unwrap();
    let instance = || InstanceField::new(base.clone(), [0.0; 3], AXES, 1.0, None, None).unwrap();
    let before = instance()
        .with_adaptive(adaptive())
        .with_surface(surface.clone())
        .unwrap();
    let after = instance()
        .with_surface(surface)
        .unwrap()
        .with_adaptive(adaptive());
    assert_eq!(before.ghost_fill_report(), after.ghost_fill_report());
    let position = [1.315, 1.01, 0.0];
    let local = std::array::from_fn(|a| position[a] - offset[a]);
    let target = expected.sample(local, 0.0);
    assert!((target.potential_v - raw_patch.sample(local, 0.0).potential_v).abs() > 1e-5);
    assert_eq!(before.sample(position, 0.0), target);
    assert_eq!(after.sample(position, 0.0), target);
}

#[test]
fn non_cut_instance_retains_sample_and_flight_bits() {
    let (array, _) = annulus(33, false);
    let grid = GridField::new_compact(array).unwrap();
    let instance = InstanceField::new(grid.clone(), [0.0; 3], AXES, 1.0, None, None).unwrap();
    assert!(instance.ghost_fill_report().is_none());
    for p in [[1.315, 1.001, 0.0], [1.81, 1.02, 0.0], [1.43, 1.17, 0.0]] {
        let original = grid.sample(p, 0.0);
        let current = instance.sample(p, 0.0);
        assert_eq!(
            original.potential_v.to_bits(),
            current.potential_v.to_bits()
        );
        for axis in 0..2 {
            assert_eq!(
                original.electric_v_per_mm[axis].to_bits(),
                current.electric_v_per_mm[axis].to_bits()
            );
        }
    }
    let ion = Particle::new(40.0, 1.0, [1.57, 1.0, 0.0], [0.0, 0.3, 0.0], 0.0).unwrap();
    let options = FlightOptions {
        duration_us: 0.01,
        ..Default::default()
    };
    let original = fly(&grid, &ion, &options).unwrap();
    let current = fly(&instance, &ion, &options).unwrap();
    assert_eq!(original.splat_reason, current.splat_reason);
    for (a, b) in original
        .final_state
        .position_mm
        .into_iter()
        .chain(original.final_state.velocity_mm_per_us)
        .zip(
            current
                .final_state
                .position_mm
                .into_iter()
                .chain(current.final_state.velocity_mm_per_us),
        )
    {
        assert_eq!(a.to_bits(), b.to_bits());
    }
    assert_eq!(original.tof_us.to_bits(), current.tof_us.to_bits());
}

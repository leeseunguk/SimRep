use std::{fmt::Write, time::Instant};

use simrep_core::{FieldType, PotentialArray};
use simrep_fly::{
    AnalyticField, Field, FieldSample, FlightOptions, Particle, SplatReason, StopPlane, fly,
    physics::magnetic::{
        Coil, ImportedMagneticMap, MagneticField, MagneticPa, MapPlacement, OutsideMap,
        VectorPotential,
    },
};
use simrep_io::bfield::{self, Format, ReadOptions};
use simrep_solve::magnetostatics::Formulation;

fn distance(a: [f64; 3], b: [f64; 3]) -> f64 {
    a.into_iter()
        .zip(b)
        .map(|(a, b)| (a - b).powi(2))
        .sum::<f64>()
        .sqrt()
}

fn import(text: &str, format: Format, outside: OutsideMap) -> ImportedMagneticMap {
    let data = bfield::parse(
        text,
        &ReadOptions {
            format,
            ..Default::default()
        },
    )
    .unwrap();
    ImportedMagneticMap::from_nodes(
        data.dims,
        data.spacing_mm,
        data.values_gauss,
        data.axisymmetric,
        MapPlacement {
            origin_mm: data.origin_mm,
            outside,
            ..Default::default()
        },
    )
    .unwrap()
}

/// Deterministic manufactured exports, not screenshots or vendor executables.
fn export(format: Format, n: usize, field: &impl Field) -> String {
    let mut text = String::new();
    if format == Format::Auto {
        text.push_str("% Grid\n");
        for _ in 0..3 {
            for i in 0..n {
                write!(text, "{:.17e} ", -2.0 + 4.0 * i as f64 / (n - 1) as f64).unwrap();
            }
            text.push('\n');
        }
        for c in 0..3 {
            writeln!(text, "% Data B{c} (G)").unwrap();
            for z in 0..n {
                for y in 0..n {
                    for x in 0..n {
                        let p = [x, y, z].map(|i| -2.0 + 4.0 * i as f64 / (n - 1) as f64);
                        write!(text, "{:.17e} ", field.sample(p, 0.0).magnetic_gauss[c]).unwrap();
                    }
                    text.push('\n');
                }
            }
        }
        return text;
    }
    match format {
        Format::Opera => text.push_str(
            "1 1 1\n1 X [MM]\n2 Y [MM]\n3 Z [MM]\n4 BX [GAUSS]\n5 BY [GAUSS]\n6 BZ [GAUSS]\n0\n",
        ),
        Format::Comsol => text.push_str("% x (mm),y (mm),z (mm),Bx (G),By (G),Bz (G)\n"),
        _ => text.push_str("x[mm],y[mm],z[mm],Bx[G],By[G],Bz[G]\n"),
    }
    for z in 0..n {
        for y in 0..n {
            for x in 0..n {
                let p = [x, y, z].map(|i| -2.0 + 4.0 * i as f64 / (n - 1) as f64);
                let b = field.sample(p, 0.0).magnetic_gauss;
                writeln!(
                    text,
                    "{:.17e},{:.17e},{:.17e},{:.17e},{:.17e},{:.17e}",
                    p[0], p[1], p[2], b[0], b[1], b[2]
                )
                .unwrap();
            }
        }
    }
    text
}

#[test]
fn imported_formats_flight_convergence_and_compute_timings() {
    let solenoid =
        MagneticField::solenoid([0.0, 0.0, -5.0], [0.0, 0.0, 5.0], 10.0, 100.0, 16).unwrap();
    let uniform = AnalyticField(|_, _| FieldSample {
        magnetic_gauss: [0.0, 0.0, 100.0],
        ..Default::default()
    });
    // Source-free 2D sector potential ~log(radius): B tangent with 1/r.
    let sector = AnalyticField(|p: [f64; 3], _| {
        let x = p[0] + 10.0;
        let y = p[1];
        let r2 = x * x + y * y;
        FieldSample {
            magnetic_gauss: [-1000.0 * y / r2, 1000.0 * x / r2, 0.0],
            ..Default::default()
        }
    });
    let coil = Coil::hoop([0.0; 3], [0.0, 0.0, 1.0], 10.0, 1000.0).unwrap();
    let loop_field = AnalyticField(|p, _| FieldSample {
        magnetic_gauss: coil.field_gauss(p),
        ..Default::default()
    });
    let ion = Particle::new(100.0, 1.0, [-1.4, 0.13, 0.17], [1.0, 0.03, -0.02], 0.0).unwrap();
    let options = FlightOptions {
        duration_us: 5.0,
        max_step_us: 0.05,
        stop_plane: Some(StopPlane {
            axis: 0,
            coordinate_mm: 1.4,
        }),
        ..Default::default()
    };
    for (name, field) in [
        ("uniform", &uniform as &dyn Field),
        ("loop", &loop_field),
        ("solenoid", &solenoid),
        ("sector", &sector),
    ] {
        let start = Instant::now();
        let exact = fly(&AnalyticField(|p, t| field.sample(p, t)), &ion, &options).unwrap();
        let exact_s = start.elapsed().as_secs_f64();
        assert_eq!(exact.splat_reason, SplatReason::StopPlane);
        for format in [
            Format::Delimited,
            Format::Opera,
            Format::Comsol,
            Format::Auto,
        ] {
            let mut previous: Option<(f64, f64, f64)> = None;
            for n in [6, 11, 21] {
                // &dyn Field is not Sized, but its forwarding wrapper is.
                let wrapper = AnalyticField(|p, t| field.sample(p, t));
                let text = export(format, n, &wrapper);
                let start = Instant::now();
                let map = import(&text, format, OutsideMap::Error);
                let preparation = start.elapsed().as_secs_f64();
                let start = Instant::now();
                let actual = fly(&map, &ion, &options).unwrap();
                let compute = start.elapsed().as_secs_f64();
                assert_eq!(actual.splat_reason, exact.splat_reason);
                let endpoint = distance(
                    actual.final_state.position_mm,
                    exact.final_state.position_mm,
                );
                let tof = (actual.tof_us - exact.tof_us).abs();
                let order = previous.map(|(e, _, h)| {
                    if endpoint > 0.0 {
                        (e / endpoint).ln() / (h / (4.0 / (n - 1) as f64)).ln()
                    } else {
                        f64::NAN
                    }
                });
                let tof_order = previous.map(|(_, e, h)| {
                    if tof > 0.0 {
                        (e / tof).ln() / (h / (4.0 / (n - 1) as f64)).ln()
                    } else {
                        f64::NAN
                    }
                });
                println!(
                    "BFIELD_CONVERGENCE,{name},{format:?},n={n},h_mm={:.9e},endpoint_mm={endpoint:.12e},tof_us={tof:.12e},endpoint_order={order:?},tof_order={tof_order:?},prepare_s={preparation:.9},compute_s={compute:.9},exact_compute_s={exact_s:.9}",
                    4.0 / (n - 1) as f64
                );
                assert!(endpoint < 1e-3 && tof < 1e-3);
                if name == "uniform" {
                    assert!(endpoint < 1e-10 && tof < 1e-10);
                } else if let Some((error, previous_tof, _)) = previous {
                    assert!(
                        endpoint < error * 0.6,
                        "{name} {format:?}: {endpoint} >= {error}"
                    );
                    assert!(
                        tof < previous_tof * 0.8 || tof < 1e-10,
                        "{name} {format:?}: TOF did not converge"
                    );
                }
                previous = Some((endpoint, tof, 4.0 / (n - 1) as f64));
            }
        }
    }
}

#[test]
fn axisymmetric_exports_converge_to_exact_coil_flights() {
    let loop_field = MagneticField {
        coils: vec![Coil::hoop([0.0; 3], [1.0, 0.0, 0.0], 10.0, 1000.0).unwrap()],
        ..Default::default()
    };
    let solenoid =
        MagneticField::solenoid([-5.0, 0.0, 0.0], [5.0, 0.0, 0.0], 10.0, 100.0, 16).unwrap();
    let uniform = MagneticField {
        uniform_gauss: [100.0, 0.0, 0.0],
        ..Default::default()
    };
    let ion = Particle::new(100.0, 1.0, [-1.4, 0.13, 0.17], [1.0, 0.2, -0.1], 0.0).unwrap();
    let options = FlightOptions {
        duration_us: 5.0,
        max_step_us: 0.05,
        stop_plane: Some(StopPlane {
            axis: 0,
            coordinate_mm: 1.4,
        }),
        ..Default::default()
    };
    for (name, field) in [
        ("uniform", uniform),
        ("loop", loop_field),
        ("solenoid", solenoid),
    ] {
        let start = Instant::now();
        let exact = fly(&field, &ion, &options).unwrap();
        let exact_s = start.elapsed().as_secs_f64();
        let mut previous: Option<(f64, f64)> = None;
        for n in [6, 11, 21] {
            let mut text = String::from("% z(mm) r(mm) Bz(G) Br(G)\n");
            for r in 0..n {
                for x in 0..n {
                    let x = -2.0 + 4.0 * x as f64 / (n - 1) as f64;
                    let r = 4.0 * r as f64 / (n - 1) as f64;
                    let b = field.sample([x, r, 0.0], 0.0).magnetic_gauss;
                    writeln!(text, "{x:.17e} {r:.17e} {:.17e} {:.17e}", b[0], b[1]).unwrap();
                }
            }
            let start = Instant::now();
            let map = import(&text, Format::Axisymmetric, OutsideMap::Error);
            let preparation = start.elapsed().as_secs_f64();
            let start = Instant::now();
            let actual = fly(&map, &ion, &options).unwrap();
            let compute = start.elapsed().as_secs_f64();
            assert_eq!(actual.splat_reason, exact.splat_reason);
            let endpoint = distance(
                actual.final_state.position_mm,
                exact.final_state.position_mm,
            );
            let tof = (actual.tof_us - exact.tof_us).abs();
            let order = previous.map(|(e, _)| {
                if endpoint > 0.0 {
                    (e / endpoint).log2()
                } else {
                    f64::NAN
                }
            });
            let tof_order = previous.map(|(_, e)| {
                if tof > 0.0 {
                    (e / tof).log2()
                } else {
                    f64::NAN
                }
            });
            println!(
                "BFIELD_CONVERGENCE,{name},Axisymmetric,n={n},h_mm={:.9e},endpoint_mm={endpoint:.12e},tof_us={tof:.12e},endpoint_order={order:?},tof_order={tof_order:?},prepare_s={preparation:.9},compute_s={compute:.9},exact_compute_s={exact_s:.9}",
                4.0 / (n - 1) as f64
            );
            assert!(endpoint < 1e-3 && tof < 1e-3);
            if name == "uniform" {
                assert!(endpoint < 1e-10 && tof < 1e-10);
            } else if let Some((error, previous_tof)) = previous {
                assert!(
                    endpoint < error * 0.6 || endpoint < 1e-10,
                    "{name}: {endpoint} >= {error}"
                );
                assert!(
                    tof < previous_tof * 0.8 || tof < 1e-10,
                    "{name}: TOF did not converge"
                );
            }
            previous = Some((endpoint, tof));
        }
    }
}

#[test]
fn imported_axis_limit_transform_mirrors_superposition_and_outside() {
    let n = 7;
    let mut text = String::new();
    for r in 0..n {
        for z in 0..n {
            // div B=0: Baxial=2*x, Br=-r. Internal axial coordinate x.
            writeln!(text, "{z} {r} {} {}", 2 * z, -(r as f64)).unwrap();
        }
    }
    let data = bfield::parse(
        &text,
        &ReadOptions {
            format: Format::Axisymmetric,
            ..Default::default()
        },
    )
    .unwrap();
    let axes = [[0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]];
    let map = ImportedMagneticMap::from_nodes(
        data.dims,
        data.spacing_mm,
        data.values_gauss,
        true,
        MapPlacement {
            origin_mm: [10.0, 20.0, 30.0],
            axes,
            scale: 2.0,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(
        distance(
            map.sample([11.0, 24.0, 32.0], 0.0).magnetic_gauss,
            [-0.5, 4.0, -1.0]
        ) < 1e-11
    );
    assert!(
        distance(
            map.sample([10.0, 24.0, 30.0], 0.0).magnetic_gauss,
            [0.0, 4.0, 0.0]
        ) < 1e-11
    );
    assert!(map.divergence_report().max_abs_gauss_per_mm < 1e-12);
    assert_eq!(
        map.status([10.0, 33.0, 30.0]),
        Some(SplatReason::OutsideGrid)
    );
    let (fraction, reason) = map
        .intersection([10.0, 22.0, 30.0], [10.0, 34.0, 30.0])
        .unwrap();
    assert!((fraction - 10.0 / 12.0).abs() < 1e-12);
    assert_eq!(reason, SplatReason::OutsideGrid);
    let field = MagneticField {
        uniform_gauss: [1.0, 2.0, 3.0],
        maps: vec![map],
        ..Default::default()
    };
    assert!(
        distance(
            field.sample([10.0, 24.0, 30.0], 0.0).magnetic_gauss,
            [1.0, 6.0, 3.0]
        ) < 1e-11
    );

    let values = (0..49)
        .map(|i| [3.0, i as f64 % 7.0, 0.0])
        .collect::<Vec<_>>();
    let mirror = ImportedMagneticMap::from_nodes(
        [7, 7, 1],
        [1.0; 3],
        values,
        false,
        MapPlacement {
            mirrors: [Some([1, -1, 1]), None, None],
            ..Default::default()
        },
    )
    .unwrap();
    assert!(
        distance(
            mirror.sample([-2.3, 1.1, 0.0], 0.0).magnetic_gauss,
            [3.0, -2.3, 0.0]
        ) < 1e-11
    );
    assert!(mirror.status([-5.0, 2.0, 100.0]).is_none());
    for outside in [OutsideMap::Error, OutsideMap::Zero, OutsideMap::Clamp] {
        let field = import("0 0 0 0 0 10\n1 0 0 0 0 20", Format::Delimited, outside);
        assert_eq!(
            field.status([2.0, 0.0, 0.0]).is_some(),
            outside == OutsideMap::Error
        );
        if outside != OutsideMap::Error {
            assert_eq!(
                field.sample([2.0, 0.0, 0.0], 0.0).magnetic_gauss[2],
                if outside == OutsideMap::Zero {
                    0.0
                } else {
                    20.0
                }
            );
        }
        let ion = Particle::new(100.0, 0.0, [0.5, 0.0, 0.0], [1.0, 0.0, 0.0], 0.0).unwrap();
        let result = fly(
            &field,
            &ion,
            &FlightOptions {
                duration_us: 2.0,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            result.splat_reason,
            if outside == OutsideMap::Error {
                SplatReason::OutsideGrid
            } else {
                SplatReason::TimeLimit
            }
        );
        assert!(
            (result.final_state.position_mm[0]
                - if outside == OutsideMap::Error {
                    1.0
                } else {
                    2.5
                })
            .abs()
                < 1e-11
        );
    }
}

#[test]
fn imported_map_boundary_validation_rejects_invalid_data() {
    let build = |dims, values, placement| {
        ImportedMagneticMap::from_nodes(dims, [1.0; 3], values, false, placement)
    };
    assert!(build([0, 1, 1], vec![], MapPlacement::default()).is_err());
    assert!(build([2, 1, 1], vec![[0.0; 3]], MapPlacement::default()).is_err());
    assert!(build([1, 1, 1], vec![[f64::NAN; 3]], MapPlacement::default()).is_err());
    for scale in [0.0, -1.0, f64::INFINITY] {
        assert!(
            build(
                [1, 1, 1],
                vec![[0.0; 3]],
                MapPlacement {
                    scale,
                    ..Default::default()
                }
            )
            .is_err()
        );
    }
    assert!(
        build(
            [1, 1, 1],
            vec![[0.0; 3]],
            MapPlacement {
                axes: [[-1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
                ..Default::default()
            }
        )
        .is_err()
    );
    assert!(
        build(
            [2, 1, 1],
            vec![[1.0; 3]; 2],
            MapPlacement {
                mirrors: [Some([1, -1, -1]), None, None],
                ..Default::default()
            }
        )
        .is_err()
    );
}

#[test]
fn imported_maps_superpose_with_prepared_coils_pas_and_vector_potentials() {
    let mut magnetic =
        MagneticField::solenoid([0.0, -50.0, 0.0], [0.0, 50.0, 0.0], 10.0, 0.7958, 100).unwrap();
    let point = [0.3, 0.4, 0.2];
    let coils = magnetic.sample(point, 0.0).magnetic_gauss;
    magnetic.uniform_gauss = [7.0, 8.0, 9.0];
    magnetic
        .maps
        .push(import("0 0 0 4 5 6", Format::Delimited, OutsideMap::Error));
    let mut pa = PotentialArray::new(3, 3, 3);
    pa.field_type = FieldType::Magnetic;
    for z in 0..3 {
        for y in 0..3 {
            for x in 0..3 {
                pa.potentials[(z * 3 + y) * 3 + x] = z as f64;
            }
        }
    }
    magnetic.arrays.push(MagneticPa::new(pa, 1.0).unwrap());
    let mut az = PotentialArray::new(3, 3, 1);
    for y in 0..3 {
        for x in 0..3 {
            az.potentials[y * 3 + x] = (x + 2 * y) as f64;
        }
    }
    magnetic
        .vector_potentials
        .push(VectorPotential::new(az, Formulation::PlanarAz, 1.0).unwrap());
    let expected = std::array::from_fn(|a| coils[a] + [13.0, 12.0, 14.0][a]);
    assert!(distance(magnetic.sample(point, 0.0).magnetic_gauss, expected) < 1e-11);
    let prepared = magnetic
        .prepare_coil_map(Some([[-1.0, -150.0, -1.0], [1.0, 150.0, 1.0]]))
        .unwrap();
    assert!(prepared.calibration().iter().any(|c| c.accepted));
    assert!(distance(prepared.sample(point, 0.0).magnetic_gauss, expected) < 2e-9);
}

#[test]
fn magnetic_pa_geometry_precedes_map_boundary_at_the_same_start() {
    let mut pa = PotentialArray::new(3, 3, 3);
    pa.field_type = FieldType::Magnetic;
    pa.electrodes.fill(true);
    let map = ImportedMagneticMap::from_nodes(
        [2, 2, 2],
        [1.0; 3],
        vec![[0.0, 0.0, 1.0]; 8],
        false,
        MapPlacement {
            origin_mm: [10.0; 3],
            ..Default::default()
        },
    )
    .unwrap();
    let magnetic = MagneticField {
        arrays: vec![MagneticPa::new(pa, 1.0).unwrap()],
        maps: vec![map],
        ..Default::default()
    };
    // Preserve the PA-first event priority already used by the GPU and the
    // segment-intersection path when both domains stop at the same point.
    assert!(matches!(
        magnetic.status([1.0; 3]),
        Some(SplatReason::Electrode { .. })
    ));
    let ion = Particle::new(100.0, 1.0, [1.0; 3], [1.0, 0.0, 0.0], 0.0).unwrap();
    let result = fly(&magnetic, &ion, &FlightOptions::default()).unwrap();
    assert!(matches!(result.splat_reason, SplatReason::Electrode { .. }));
    assert_eq!(result.tof_us, 0.0);
}

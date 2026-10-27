#![cfg(feature = "occt")]

use simrep_cad::{MeasureOptions, MeasurementModel, write_fixture};
use simrep_geom::SignedDistance;
use simrep_mesh::Unit;

#[test]
fn measurement_and_refinement_imports_agree_on_units_and_placements() {
    let dir = tempfile::tempdir().unwrap();
    for extension in ["step", "igs"] {
        let path = dir.path().join(format!("pair.{extension}"));
        write_fixture(&path, 3, "AP242DIS", "MM").unwrap();
        for units in [
            None,
            Some(Unit::Millimeter),
            Some(Unit::Centimeter),
            Some(Unit::Inch),
        ] {
            let scale = units.map_or(1.0, Unit::millimeters);
            let model = MeasurementModel::read_with_units(&path, units).unwrap();
            let gap = model.gap("0", "1", MeasureOptions::default()).unwrap();
            assert!((gap.min_distance_mm - 0.58 * scale).abs() < 1e-8);
            let parts = simrep_cad::read_cad(
                &path,
                simrep_cad::ImportOptions {
                    units,
                    band_mm: scale,
                    ..Default::default()
                },
            )
            .unwrap();
            for (point, part) in gap.closest_points_mm.into_iter().zip(&parts) {
                assert!(part.distance(point).abs() < 1e-8);
            }
            let expected_bounds = [[-0.91, -0.29], [0.29, 0.91]];
            for (part, bounds) in parts.iter().zip(expected_bounds) {
                assert!((part.bounds_mm[0][0] - bounds[0] * scale).abs() < 1e-6 * scale);
                assert!((part.bounds_mm[1][0] - bounds[1] * scale).abs() < 1e-6 * scale);
            }
        }
    }
}

#[test]
fn placed_spheres_have_analytic_gap_and_witnesses_in_step_and_iges() {
    let dir = tempfile::tempdir().unwrap();
    for ext in ["step", "stp", "iges", "igs"] {
        let path = dir.path().join(format!("pair.{ext}"));
        write_fixture(&path, 3, "AP242DIS", "MM").unwrap();
        let model = MeasurementModel::read(&path).unwrap();
        assert_eq!(model.parts().len(), 2);
        let gap = model.gap("0", "1", MeasureOptions::default()).unwrap();
        // Radius .31, centers -.6 and +.6: 1.2 - 2*.31 = .58 mm.
        assert!((gap.min_distance_mm - 0.58).abs() < 1e-9, "{gap:?}");
        assert_eq!(gap.faces_a, vec![0]);
        assert_eq!(gap.faces_b, vec![0]);
        assert_eq!(gap.closest_faces, [0, 0]);
        for (point, x) in gap.closest_points_mm.iter().zip([-0.29, 0.29]) {
            assert!((point[0] - x).abs() < 1e-9);
            assert!(point[1].abs() < 1e-9 && point[2].abs() < 1e-9);
        }
        assert!(!gap.touching);
        assert!(gap.cross_check_error_mm <= 0.01, "{gap:?}");
        assert!((gap.tessellated_distance_mm - 0.58).abs() < 0.01);
        if ext == "step" {
            let named = model
                .gap("pair/left:faces=0", "pair/right", MeasureOptions::default())
                .unwrap();
            assert_eq!(named.min_distance_mm, gap.min_distance_mm);
        }
        let pairs = model.all_pairs(MeasureOptions::default()).unwrap();
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].min_distance_mm, gap.min_distance_mm);
    }
}

#[test]
fn touching_and_threshold_are_distance_classifications() {
    let dir = tempfile::tempdir().unwrap();
    for ext in ["step", "igs"] {
        let path = dir.path().join(format!("touching.{ext}"));
        write_fixture(&path, 4, "AP242DIS", "MM").unwrap();
        let model = MeasurementModel::read(&path).unwrap();
        let gap = model.gap("0", "1", MeasureOptions::default()).unwrap();
        assert!(gap.min_distance_mm < 1e-9, "{gap:?}");
        assert!(gap.touching);
        assert!(gap.tessellated_distance_mm < 0.01, "{gap:?}");
    }
    let path = dir.path().join("pair.step");
    write_fixture(&path, 3, "AP242DIS", "MM").unwrap();
    let model = MeasurementModel::read(&path).unwrap();
    let options = MeasureOptions {
        touch_tolerance_mm: 0.59,
        ..Default::default()
    };
    assert!(model.gap("0", "1", options).unwrap().touching);
    let distance = model
        .gap("0", "1", MeasureOptions::default())
        .unwrap()
        .min_distance_mm;
    assert!(
        model
            .gap(
                "0",
                "1",
                MeasureOptions {
                    touch_tolerance_mm: distance,
                    ..Default::default()
                }
            )
            .unwrap()
            .touching
    );
}

#[test]
fn complete_cylinder_faces_and_trimmed_face_subsets() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cylinder.step");
    write_fixture(&path, 1, "AP242DIS", "MM").unwrap();
    let model = MeasurementModel::read(&path).unwrap();
    assert_eq!(model.parts()[0].face_count, 3);
    let full = model
        .gap("0", "cylinder:faces=1", MeasureOptions::default())
        .unwrap();
    assert_eq!(full.faces_a, vec![0, 1, 2]);
    assert!(full.touching && full.min_distance_mm < 1e-9);
    let caps = model
        .gap("0:faces=1", "0:faces=2", MeasureOptions::default())
        .unwrap();
    assert!((caps.min_distance_mm - 4.0).abs() < 1e-9, "{caps:?}");
    assert_eq!(caps.closest_faces, [1, 2]);
    assert!(!caps.touching);
    assert!((caps.tessellated_distance_mm - 4.0).abs() < 1e-9);
}

#[test]
fn invalid_selectors_and_face_coverage_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pair.step");
    write_fixture(&path, 3, "AP242DIS", "MM").unwrap();
    let model = MeasurementModel::read(&path).unwrap();
    for bad in [
        "",
        "2",
        "left",
        "0:faces=",
        "0:faces=0,0",
        "0:faces=1",
        "0:faces=-1",
        "0:faces=0,",
        "0:faces=,0",
        "0:faces=0:faces=0",
        "0:faces=184467440737095516160",
    ] {
        assert!(
            model.gap(bad, "1", MeasureOptions::default()).is_err(),
            "{bad:?}"
        );
    }
    for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert!(
            model
                .all_pairs(MeasureOptions {
                    cad_tolerance_mm: bad,
                    ..Default::default()
                })
                .is_err()
        );
    }
    for bad in [-1.0, f64::NAN, f64::INFINITY] {
        assert!(
            model
                .gap(
                    "0",
                    "1",
                    MeasureOptions {
                        touch_tolerance_mm: bad,
                        ..Default::default()
                    }
                )
                .is_err()
        );
    }
    assert!(MeasurementModel::read(dir.path().join("pair.stl")).is_err());
}

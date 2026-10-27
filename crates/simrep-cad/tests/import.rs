#![cfg(feature = "occt")]

use simrep_cad::{ImportOptions, Mode, read_cad, write_fixture};
use simrep_geom::SignedDistance;
use simrep_mesh::Unit;

#[test]
fn exchange_schemas_names_units_and_curved_distances() {
    let dir = tempfile::tempdir().unwrap();
    for schema in ["AP203", "AP214IS", "AP242DIS"] {
        for kind in 0..3 {
            let path = dir.path().join(format!("{schema}-{kind}.step"));
            write_fixture(&path, kind, schema, "MM").unwrap();
            let parts = read_cad(&path, ImportOptions::default()).unwrap();
            assert_eq!(parts.len(), 1);
            assert_eq!(parts[0].name, ["sphere", "cylinder", "ring"][kind as usize]);
            if kind == 1 {
                assert!((parts[0].distance([0.0, 0.0, 2.2]) - 0.2).abs() < 1e-7);
                let edge = parts[0].distance([0.5, 0.0, 2.4]);
                assert!((edge - 0.19_f64.hypot(0.4)).abs() < 1e-7);
            } else if kind == 2 {
                assert!((parts[0].distance([0.0; 3]) - 0.36).abs() < 1e-7);
                assert!((parts[0].distance([0.55, 0.0, 0.0]) + 0.19).abs() < 1e-7);
            }
            for i in 0..50 {
                let p = [
                    0.025 + i as f64 * 0.019,
                    0.017 + (i % 7) as f64 * 0.031,
                    -0.23 + (i % 11) as f64 * 0.047,
                ];
                let expected = match kind {
                    0 => p.iter().map(|x| x * x).sum::<f64>().sqrt() - 0.31,
                    1 => p[0].hypot(p[1]) - 0.31,
                    _ => (p[0].hypot(p[1]) - 0.55).hypot(p[2]) - 0.19,
                };
                let got = parts[0].try_distance(p).unwrap();
                assert!(
                    (got - expected).abs() < 1e-7,
                    "{schema} {kind} {p:?}: {got} vs {expected}"
                );
            }
        }
    }
    for (unit, mm) in [("M", 1000.0), ("INCH", 25.4)] {
        for extension in ["stp", "igs"] {
            let path = dir.path().join(format!("sphere-{unit}.{extension}"));
            write_fixture(&path, 0, "AP242DIS", unit).unwrap();
            let part = read_cad(&path, ImportOptions::default()).unwrap().remove(0);
            assert!((part.distance([0.51, 0.0, 0.0]) - 0.2).abs() < 1e-8);
            let part = read_cad(
                &path,
                ImportOptions {
                    units: Some(Unit::Millimeter),
                    ..Default::default()
                },
            )
            .unwrap()
            .remove(0);
            let got = part.distance([0.51 / mm, 0.0, 0.0]);
            assert!((got - 0.2 / mm).abs() < 1e-8, "{unit} {extension}: {got}");
        }
    }
}

#[test]
fn iges_solid_and_explicit_mesh_fallback() {
    let dir = tempfile::tempdir().unwrap();
    for extension in ["step", "iges", "igs"] {
        let path = dir.path().join(format!("sphere.{extension}"));
        write_fixture(&path, 0, "AP214IS", "MM").unwrap();
        let part = read_cad(&path, ImportOptions::default()).unwrap().remove(0);
        assert!((part.distance([0.2, 0.0, 0.0]) + 0.11).abs() < 1e-7);
        assert!((part.distance([0.51, 0.0, 0.0]) - 0.2).abs() < 1e-7);
        let mesh = read_cad(
            &path,
            ImportOptions {
                mode: Mode::Tessellated,
                chordal_tolerance_mm: 0.002,
                ..Default::default()
            },
        )
        .unwrap()
        .remove(0);
        assert_eq!(mesh.mode, Mode::Tessellated);
        assert!(mesh.triangle_count > 100);
        assert!(mesh.distance([0.0; 3]) < -0.3);
        assert!((mesh.distance([0.51, 0.0, 0.0]) - 0.2).abs() < 0.002);
    }
}

#[test]
fn band_saturation_is_conservative_and_continuous() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sphere.step");
    write_fixture(&path, 0, "AP203", "MM").unwrap();
    let part = read_cad(
        &path,
        ImportOptions {
            band_mm: 0.2,
            ..Default::default()
        },
    )
    .unwrap()
    .remove(0);
    assert_eq!(part.distance([0.0; 3]), -0.2);
    assert_eq!(part.distance([100.0; 3]), 0.2);
    for i in 0..100 {
        let p = [i as f64 / 100.0, 0.03, 0.04];
        let q = [p[0] + 0.007, p[1], p[2]];
        assert!((part.distance(p) - part.distance(q)).abs() <= 0.0070000001);
    }
    assert!(part.try_distance([f64::NAN; 3]).is_err());
    assert!(
        read_cad(
            &path,
            ImportOptions {
                band_mm: 0.0,
                ..Default::default()
            }
        )
        .is_err()
    );
}

#[test]
fn named_assembly_instances_remain_separate_located_electrodes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pair.step");
    write_fixture(&path, 3, "AP242DIS", "MM").unwrap();
    let parts = read_cad(&path, ImportOptions::default()).unwrap();
    assert_eq!(parts.len(), 2);
    assert_eq!(parts[0].name, "pair/left");
    assert_eq!(parts[1].name, "pair/right");
    for (part, x) in parts.iter().zip([-0.6, 0.6]) {
        assert!((part.distance([x, 0.0, 0.0]) + 0.31).abs() < 1e-7);
        assert!((part.distance([-x, 0.0, 0.0]) - 0.89).abs() < 1e-7);
    }
}

#[test]
fn exact_cylinder_cutcell_beats_tessellation_at_equal_grid() {
    use simrep_geom::Electrode;
    use simrep_solve::cutcell::CutCells;
    use simrep_solve::mg::{Geometry, Grid, Options};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cylinder.step");
    write_fixture(&path, 1, "AP214IS", "MM").unwrap();
    let n = 65;
    let h = 2.0 / (n - 1) as f64;
    let mut errors = Vec::new();
    for mode in [Mode::Exact, Mode::Tessellated] {
        let part = read_cad(
            &path,
            ImportOptions {
                mode,
                band_mm: 2.0 * h,
                chordal_tolerance_mm: 0.02,
                ..Default::default()
            },
        )
        .unwrap()
        .remove(0);
        let cuts = CutCells::new(
            Grid {
                dims: [n, n, 1],
                spacing: [h; 3],
                geometry: Geometry::Planar,
            },
            [-1.0, -1.0, 0.0],
            &[
                Electrode::new(1.0, part),
                Electrode::new(0.0, |p: [f64; 3]| 0.83 - p[0].hypot(p[1])),
            ],
        )
        .unwrap();
        let mut values = cuts.values.clone();
        let report = cuts
            .solve(
                &mut values,
                Options {
                    relative_tolerance: 1e-11,
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(report.converged);
        let mut sum = 0.0;
        for (i, &value) in values.iter().enumerate() {
            let r = (i % n) as f64 * h - 1.0;
            let r = r.hypot((i / n) as f64 * h - 1.0);
            if r > 0.31 && r < 0.83 {
                let exact = (0.83 / r).ln() / (0.83_f64 / 0.31).ln();
                sum += (value - exact).powi(2);
            }
        }
        errors.push(sum.sqrt());
    }
    assert!(errors[0] < errors[1], "{errors:?}");
}

#[test]
fn parallel_face_mesher_matches_serial_on_sphere_fixture() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sphere.step");
    simrep_cad::write_fixture(&path, 0, "AP242DIS", "MM").unwrap();
    let matched = simrep_cad::parallel_mesh_matches(&path, 0.01).unwrap();
    assert!(
        matched,
        "parallel OCCT tessellation changed the serial triangulation"
    );
}

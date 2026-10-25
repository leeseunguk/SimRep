use std::process::Command;

#[cfg(not(feature = "cad"))]
#[test]
fn cad_input_explains_the_optional_feature() {
    let dir = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_simrep"))
        .args([
            "refine-geometry",
            "sphere.step",
            "--electrode",
            "0=1:1",
            "--out",
            "sphere",
        ])
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("--features cad"));
}

#[cfg(feature = "cad")]
#[test]
fn cad_formats_and_mesh_fallback_write_fast_adjust_and_cuts() {
    let dir = tempfile::tempdir().unwrap();
    for extension in ["step", "stp", "iges", "igs"] {
        let input = format!("sphere.{extension}");
        simrep_cad::write_fixture(dir.path().join(&input), 0, "AP242DIS", "MM").unwrap();
        for mode in ["exact", "tessellated"] {
            let output = Command::new(env!("CARGO_BIN_EXE_simrep"))
                .args([
                    "refine-geometry",
                    &input,
                    "--grid",
                    "9,9,9",
                    "--spacing",
                    "0.2,0.2,0.2",
                    "--origin",
                    "-0.8,-0.8,-0.8",
                    "--electrode",
                    if extension == "step" {
                        "sphere=3:17"
                    } else {
                        "0=3:17"
                    },
                    "--cad-mode",
                    mode,
                    "--cad-tolerance",
                    "0.002",
                    "--sidecar-dense-sdf",
                    "--out",
                    "sphere",
                ])
                .current_dir(dir.path())
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            let pa =
                simrep_io::read_fast_adjust(dir.path().join("sphere.pa0"), &[(3, 7.0)]).unwrap();
            assert!(pa.electrodes[4 + 9 * (4 + 9 * 4)]);
            assert!(pa.potentials.iter().all(|v| (v - 7.0).abs() < 1e-7));
            let cuts: serde_json::Value = serde_json::from_slice(
                &std::fs::read(dir.path().join("sphere.cuts.json")).unwrap(),
            )
            .unwrap();
            let cut = cuts["cut_fractions"][6 + 9 * (4 + 9 * 4)][0]
                .as_f64()
                .unwrap();
            assert!((cut - 0.45).abs() < if mode == "exact" { 1e-7 } else { 0.01 });
            assert_eq!(cuts["version"], 3);
            assert_eq!(
                cuts["source_geometry"][0]["geometry"]["chordal_tolerance_mm"],
                0.002
            );
            let surface = simrep_fly::surface::SurfaceModel::from_reader(
                std::fs::File::open(dir.path().join("sphere.cuts.json")).unwrap(),
            )
            .unwrap();
            assert_eq!(surface.membership([0.0; 3]), Some(3));
            let (t, number) = surface
                .first_hit([-0.7, 0.0, 0.0], [0.7, 0.0, 0.0])
                .unwrap();
            assert_eq!(number, 3);
            assert!((-0.7 + 1.4 * t + 0.31).abs() < 0.002);
        }
    }
}

#[cfg(feature = "cad")]
#[test]
fn located_step_electrodes_refine_and_fly_with_fast_adjust() {
    let dir = tempfile::tempdir().unwrap();
    simrep_cad::write_fixture(dir.path().join("pair.step"), 3, "AP242DIS", "MM").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_simrep"))
        .args([
            "refine-geometry",
            "pair.step",
            "--grid",
            "17,17,17",
            "--spacing",
            "0.125,0.125,0.125",
            "--origin",
            "-1,-1,-1",
            "--electrode",
            "pair/left=1:10",
            "--electrode",
            "pair/right=2:-10",
            "--cad-mode",
            "exact",
            "--out",
            "pair",
        ])
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    for extension in ["pa0", "pa1", "pa2", "pa#", "cuts.json"] {
        assert!(dir.path().join(format!("pair.{extension}")).is_file());
    }
    let adjusted =
        simrep_io::read_fast_adjust(dir.path().join("pair.pa0"), &[(1, 5.0), (2, -5.0)]).unwrap();
    assert!(adjusted.potentials.iter().all(|v| v.is_finite()));
    assert!((adjusted.potentials[3 + 17 * (8 + 17 * 8)] - 5.0).abs() < 1e-8);
    assert!((adjusted.potentials[13 + 17 * (8 + 17 * 8)] + 5.0).abs() < 1e-8);
    std::fs::write(dir.path().join("probe.ion"), "0,1,1,0,-0.7,0,0,90,100\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_simrep"))
        .args([
            "fly",
            "--pa",
            "pair.pa0",
            "--particles",
            "probe.ion",
            "--voltage",
            "1=5,2=-5",
            "--origin",
            "-1,-1,-1",
            "--plane",
            "0,1,0,0.7",
            "--device",
            "cpu",
            "--output",
            "flight.csv",
        ])
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let csv = std::fs::read_to_string(dir.path().join("flight.csv")).unwrap();
    let rows: Vec<_> = csv.lines().skip(1).collect();
    assert_eq!(rows.len(), 1);
    let fields: Vec<_> = rows[0].split(',').collect();
    let x: f64 = fields[2].parse().unwrap();
    let y: f64 = fields[3].parse().unwrap();
    assert!(x > 0.001 && x < 0.2, "charged probe must deflect: {csv}");
    assert!((y - 0.7).abs() < 1e-8, "{csv}");
    assert_eq!(fields[6], "plane");
}

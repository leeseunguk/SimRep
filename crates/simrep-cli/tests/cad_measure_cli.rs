use std::process::{Command, Output};

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_simrep"))
        .args(args)
        .output()
        .unwrap()
}

fn json(output: &Output) -> serde_json::Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "{error}: stdout={}, stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

#[cfg(not(feature = "cad"))]
#[test]
fn measurement_explains_optional_feature_in_json() {
    let result = run(&["cad-measure", "pair.step", "--gap", "0", "1", "--json"]);
    assert!(!result.status.success());
    let body = json(&result);
    assert_eq!(body["schema_version"], 1);
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("--features cad")
    );
}

#[test]
fn measurement_argument_modes_and_tolerances_are_validated() {
    for args in [
        vec!["cad-measure", "pair.step"],
        vec!["cad-measure", "pair.step", "--gap", "0"],
        vec!["cad-measure", "pair.step", "--gap", "0", "1", "--all-pairs"],
        vec![
            "cad-measure",
            "pair.step",
            "--all-pairs",
            "--cad-tolerance",
            "0",
        ],
        vec![
            "cad-measure",
            "pair.step",
            "--all-pairs",
            "--cad-tolerance",
            "NaN",
        ],
        vec![
            "cad-measure",
            "pair.step",
            "--all-pairs",
            "--touch-tolerance=-1",
        ],
        vec![
            "cad-measure",
            "pair.step",
            "--all-pairs",
            "--touch-tolerance",
            "inf",
        ],
    ] {
        let mut args = args;
        args.push("--json");
        let output = run(&args);
        assert!(!output.status.success(), "{args:?}");
        let body = json(&output);
        assert_eq!(body["schema_version"], 1);
        assert_eq!(body["error"]["kind"], "invalid_arguments");
    }
}

#[cfg(feature = "cad")]
#[test]
fn complete_face_gap_and_matrices_are_schema_one_json() {
    let dir = tempfile::tempdir().unwrap();
    for (kind, stem, expected, touching) in [(3, "pair", 0.58, false), (4, "touching", 0.0, true)] {
        for ext in ["step", "igs"] {
            let path = dir.path().join(format!("{stem}.{ext}"));
            simrep_cad::write_fixture(&path, kind, "AP242DIS", "MM").unwrap();
            let output = run(&[
                "cad-measure",
                path.to_str().unwrap(),
                "--all-pairs",
                "--json",
            ]);
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let body = json(&output);
            assert_eq!(body["schema_version"], 1);
            assert!(body["simrep_version"].is_string());
            assert_eq!(body["parts"].as_array().unwrap().len(), 2);
            assert_eq!(body["measurements"].as_array().unwrap().len(), 1);
            let m = &body["measurements"][0];
            let gap = m["min_distance_mm"].as_f64().unwrap();
            assert!((gap - expected).abs() < 1e-9, "{body}");
            assert_eq!(m["touching"], touching);
            assert_eq!(m["faces_a"], serde_json::json!([0]));
            assert_eq!(m["faces_b"], serde_json::json!([0]));
            assert_eq!(m["closest_faces"], serde_json::json!([0, 0]));
            assert_eq!(m["closest_points_mm"].as_array().unwrap().len(), 2);
            assert!(m["cross_check_error_mm"].as_f64().unwrap() <= 0.01);
            assert_eq!(
                body["distance_matrix_mm"][0][1],
                body["distance_matrix_mm"][1][0]
            );
            assert_eq!(body["distance_matrix_mm"][0][1], m["min_distance_mm"]);
            assert_eq!(body["distance_matrix_mm"][0][0], 0.0);
            assert_eq!(
                body["touching_matrix"],
                serde_json::json!([[true, touching], [touching, true]])
            );
        }
    }
}

#[cfg(feature = "cad")]
#[test]
fn named_subsets_and_configurable_touch_threshold() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pair.step");
    simrep_cad::write_fixture(&path, 3, "AP242DIS", "MM").unwrap();
    let output = run(&[
        "cad-measure",
        path.to_str().unwrap(),
        "--gap",
        "pair/left:faces=0",
        "pair/right",
        "--touch-tolerance",
        "0.59",
        "--cad-tolerance",
        "0.001",
        "--json",
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let body = json(&output);
    assert!(body["distance_matrix_mm"].is_null());
    assert!(body["touching_matrix"].is_null());
    assert_eq!(body["measurements"][0]["touching"], true);
    assert_eq!(body["measurements"][0]["name_a"], "pair/left");
    assert!(
        body["measurements"][0]["cross_check_error_mm"]
            .as_f64()
            .unwrap()
            <= 0.001
    );
}

#[cfg(feature = "cad")]
#[test]
fn invalid_face_subsets_and_part_selectors_fail_without_success_summary() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pair.step");
    simrep_cad::write_fixture(&path, 3, "AP242DIS", "MM").unwrap();
    for selector in [
        "0:faces=",
        "0:faces=0,0",
        "0:faces=1",
        "0:faces=0,",
        "2",
        "missing",
    ] {
        let output = run(&[
            "cad-measure",
            path.to_str().unwrap(),
            "--gap",
            selector,
            "1",
            "--json",
        ]);
        assert!(!output.status.success(), "{selector}");
        let body = json(&output);
        assert_eq!(body["schema_version"], 1);
        assert!(body["error"]["message"].is_string());
        assert!(body.get("measurements").is_none());
    }
}

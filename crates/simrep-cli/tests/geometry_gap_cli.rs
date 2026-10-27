use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn run(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_simrep"))
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap()
}

fn summary(output: Output) -> serde_json::Value {
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn outputs(dir: &Path) -> Vec<Vec<u8>> {
    ["pa0", "pa1", "pa2", "pa#", "cuts.json"]
        .map(|ext| fs::read(dir.join(format!("field.{ext}"))).unwrap())
        .to_vec()
}

#[test]
fn gem_gap_policy_and_json_leave_pa_and_cuts_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("plates.gem"),
        "pa_define(9,5,5,planar)\ne(1){fill{within{box3d(-1,-1,-1,2,5,5)}}}\ne(2){fill{within{box3d(4,-1,-1,9,5,5)}}}",
    )
    .unwrap();
    let args = [
        "--json",
        "refine-geometry",
        "plates.gem",
        "--electrode",
        "1=1:0",
        "--electrode",
        "2=2:10",
        "--out",
        "field",
    ];
    let checked = summary(run(dir.path(), &args));
    assert_eq!(checked["schema_version"], 1);
    let report = &checked["gap_check"];
    assert_eq!(report["mode"], "warn");
    assert_eq!(report["threshold_cells"], 2.5);
    assert_eq!(report["recommended_cells"], 10.0);
    assert_eq!(report["pairs"][0]["electrodes"], serde_json::json!([1, 2]));
    assert_eq!(report["pairs"][0]["method"], "bounded");
    assert_eq!(report["pairs"][0]["status"], "warn");
    assert!((report["pairs"][0]["cells_across"].as_f64().unwrap() - 2.0).abs() < 1e-6);
    assert!((report["recommended_spacing_mm"].as_f64().unwrap() - 0.2).abs() < 1e-6);
    let bracket = &report["pairs"][0]["gap_bracket_mm"];
    assert!(bracket[0].as_f64().unwrap() <= 2.0);
    assert!(bracket[1].as_f64().unwrap() >= 2.0);
    assert_eq!(
        report["pairs"][0]["location_mm"].as_array().unwrap().len(),
        2
    );
    let before = outputs(dir.path());
    let mut off = args.to_vec();
    off.extend(["--gap-check", "off"]);
    let unchecked = summary(run(dir.path(), &off));
    assert_eq!(unchecked["gap_check"]["mode"], "off");
    assert_eq!(unchecked["gap_check"]["pairs"], serde_json::json!([]));
    assert_eq!(before, outputs(dir.path()));
    let mut strict = args.to_vec();
    strict.extend(["--gap-check", "error"]);
    let out = strict.iter().position(|arg| *arg == "--out").unwrap();
    strict[out + 1] = "rejected";
    let output = run(dir.path(), &strict);
    assert!(!output.status.success());
    let error: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        error["error"]["message"]
            .as_str()
            .unwrap()
            .contains("gap check failed")
    );
    assert!(!dir.path().join("rejected.pa#").exists());
}

#[test]
fn mesh_fixture_has_an_exact_gap_and_uses_coarsest_active_spacing() {
    let dir = tempfile::tempdir().unwrap();
    let tetra = simrep_mesh::Part {
        name: "left".into(),
        vertices: vec![[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        triangles: vec![[0, 2, 1], [0, 1, 3], [0, 3, 2], [1, 2, 3]],
    };
    let mut right = tetra.clone();
    right.name = "right".into();
    for point in &mut right.vertices {
        point[0] += 2.0;
    }
    simrep_mesh::write_mesh(
        dir.path().join("pair.obj"),
        &simrep_mesh::Mesh {
            parts: vec![tetra, right],
        },
    )
    .unwrap();
    let args = [
        "--json",
        "refine-geometry",
        "pair.obj",
        "--electrode",
        "0=1:0",
        "--electrode",
        "1=2:10",
        "--grid",
        "15,5,5",
        "--spacing",
        "0.25,0.5,0.5",
        "--origin",
        "-0.25,-0.5,-0.5",
        "--out",
        "field",
    ];
    let checked = summary(run(dir.path(), &args));
    let pair = &checked["gap_check"]["pairs"][0];
    assert_eq!(pair["method"], "exact");
    assert_eq!(pair["gap_mm"], 1.0);
    assert_eq!(pair["cells_across"], 2.0);
    assert_eq!(pair["status"], "warn");
    let before = outputs(dir.path());
    let mut off = args.to_vec();
    off.extend(["--gap-check", "off"]);
    summary(run(dir.path(), &off));
    assert_eq!(before, outputs(dir.path()));
}

#[test]
fn unresolved_gem_pair_fails_before_any_output() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("spheres.gem"),
        "pa_define(17,17,17,planar)\ne(1){fill{within{sphere(8,8,8,2)}}}\ne(2){fill{within{sphere(8,8,8,4)} notin{sphere(8,8,8,3)}}}",
    )
    .unwrap();
    let output = run(
        dir.path(),
        &[
            "--json",
            "refine-geometry",
            "spheres.gem",
            "--electrode",
            "1=1:0",
            "--electrode",
            "2=2:0",
            "--gap-check",
            "error",
            "--out",
            "field",
        ],
    );
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("unresolved"));
    assert!(!dir.path().join("field.pa#").exists());
}

#[cfg(feature = "cad")]
#[test]
fn cad_pair_fixture_passes_and_fails_at_expected_grids_and_honors_units() {
    let dir = tempfile::tempdir().unwrap();
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/cad-fixtures/pair.step"),
        dir.path().join("pair.step"),
    )
    .unwrap();
    let args = [
        "--json",
        "refine-geometry",
        "pair.step",
        "--electrode",
        "0=1:0",
        "--electrode",
        "1=2:10",
        "--grid",
        "11,11,11",
        "--origin",
        "-1,-1,-1",
        "--spacing",
        "0.2,0.2,0.2",
        "--gap-check",
        "error",
        "--cad-tolerance",
        "0.002",
        "--out",
        "field",
    ];
    let checked = summary(run(dir.path(), &args));
    let pair = &checked["gap_check"]["pairs"][0];
    assert_eq!(pair["method"], "exact");
    assert!((pair["gap_mm"].as_f64().unwrap() - 0.58).abs() < 1e-9);
    assert!((pair["cells_across"].as_f64().unwrap() - 2.9).abs() < 1e-9);
    assert_eq!(pair["status"], "ok");
    let before = outputs(dir.path());
    let mut off = args.to_vec();
    let policy = off.iter().position(|arg| *arg == "--gap-check").unwrap();
    off[policy + 1] = "off";
    summary(run(dir.path(), &off));
    assert_eq!(before, outputs(dir.path()));
    let mut failed = args.to_vec();
    let spacing = failed.iter().position(|arg| *arg == "--spacing").unwrap();
    failed[spacing + 1] = "0.25,0.25,0.25";
    let out = failed.iter().position(|arg| *arg == "--out").unwrap();
    failed[out + 1] = "rejected";
    let output = run(dir.path(), &failed);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("gap check failed"));
    assert!(!dir.path().join("rejected.pa#").exists());
    let mut scaled = args.to_vec();
    scaled.extend(["--units", "cm"]);
    scaled[spacing + 1] = "2,2,2";
    let origin = scaled.iter().position(|arg| *arg == "--origin").unwrap();
    scaled[origin + 1] = "-10,-10,-10";
    let checked = summary(run(dir.path(), &scaled));
    assert!((checked["gap_check"]["pairs"][0]["gap_mm"].as_f64().unwrap() - 5.8).abs() < 1e-8);
}

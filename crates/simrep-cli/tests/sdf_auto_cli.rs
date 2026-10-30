use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

fn invoke(dir: &Path, args: &[&str], threshold: Option<&str>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_simrep"));
    command
        .current_dir(dir)
        .args(args)
        .env_remove("SIMREP_SDF_GPU_CROSSOVER");
    if let Some(threshold) = threshold {
        command.env("SIMREP_SDF_GPU_CROSSOVER", threshold);
    }
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn fixture(dir: &Path) {
    fs::write(dir.join("plates.gem"), "pa_define(9,5,1,planar)\ne(1){fill{within{box(-1,-1,2,5)}}}\ne(2){fill{within{box(6,-1,9,5)}}}").unwrap();
    fs::write(dir.join("ions.ion"), "0,40,1,4,2,0,0,0,1\n").unwrap();
    invoke(
        dir,
        &[
            "refine-geometry",
            "plates.gem",
            "--electrode",
            "1=1:0",
            "--electrode",
            "2=2:0",
            "--out",
            "plates",
            "--device",
            "cpu",
        ],
        None,
    );
}

fn fly(dir: &Path, options: &[&str], threshold: Option<&str>) -> (serde_json::Value, Output) {
    let mut args = vec![
        "--json",
        "fly",
        "--pa",
        "plates.pa0",
        "--particles",
        "ions.ion",
        "--duration-us",
        "0.01",
        "--allow-time-limit",
        "--output",
        "flight.csv",
    ];
    args.extend_from_slice(options);
    let output = invoke(dir, &args, threshold);
    (serde_json::from_slice(&output.stdout).unwrap(), output)
}

#[test]
fn auto_is_opt_in_and_default_exact_flight_is_byte_identical() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    let (default, _) = fly(dir.path(), &[], Some("0"));
    let before = fs::read(dir.path().join("flight.csv")).unwrap();
    let (exact, _) = fly(dir.path(), &["--surface-backend", "exact"], Some("0"));
    assert_eq!(before, fs::read(dir.path().join("flight.csv")).unwrap());
    assert_eq!(default["requested"]["surface_backend"], "exact");
    assert_eq!(default["executed"]["surface_backend"], "exact");
    assert_eq!(default["run_hash"], exact["run_hash"]);
    let (auto, output) = fly(dir.path(), &["--surface-backend", "auto"], None);
    assert_eq!(before, fs::read(dir.path().join("flight.csv")).unwrap());
    assert_eq!(auto["requested"]["surface_backend"], "auto");
    assert_eq!(auto["executed"]["surface_backend"], "exact");
    assert_eq!(auto["executed"]["device"], "cpu");
    assert_eq!(auto["fallback_reason"], serde_json::Value::Null);
    assert_eq!(
        auto["reason"],
        "auto: CPU exact (no measured GPU SDF crossover)"
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains(auto["reason"].as_str().unwrap()));
}

#[test]
fn explicit_gpu_auto_without_measurements_resolves_to_cpu_exact() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    for physics in [
        vec![],
        vec!["--space-charge", "direct"],
        vec!["--bfield=0,0,1"],
        vec!["--collision", "hs1"],
    ] {
        let mut options = vec!["--device", "gpu", "--surface-backend", "auto"];
        options.extend(physics);
        let (summary, _) = fly(dir.path(), &options, None);
        assert_eq!(summary["executed"]["device"], "cpu");
        assert_eq!(summary["executed"]["surface_backend"], "exact");
        assert!(summary["fallback_reason"].is_null());
    }
}

#[test]
fn environment_override_never_selects_a_software_adapter() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    let probe = invoke(
        dir.path(),
        &["--json", "gpu-info", "--backend", "vulkan"],
        None,
    );
    let adapter: serde_json::Value = serde_json::from_slice(&probe.stdout).unwrap();
    if adapter["software_adapter"] != true {
        // Software refusal is exercised on Mesa/CI; hardware selection is
        // covered deterministically by the pure resolver unit tests.
        return;
    }
    for device in ["auto", "gpu"] {
        let (summary, output) = fly(
            dir.path(),
            &["--surface-backend", "auto", "--device", device],
            Some("0"),
        );
        assert_eq!(summary["executed"]["device"], "cpu");
        assert_eq!(summary["executed"]["surface_backend"], "exact");
        assert!(summary["fallback_reason"].is_null());
        assert!(
            summary["reason"]
                .as_str()
                .unwrap()
                .contains("software adapters refused")
        );
        assert!(String::from_utf8_lossy(&output.stderr).contains("software adapters refused"));
    }
}

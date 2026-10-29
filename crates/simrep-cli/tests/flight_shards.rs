use serde_json::{Value, json};
use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn invoke(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_simrep"))
        .arg("--json")
        .args(args)
        .env("RAYON_NUM_THREADS", "2")
        .current_dir(dir)
        .output()
        .unwrap()
}
fn run(dir: &Path, args: &[&str]) -> Value {
    let output = invoke(dir, args);
    assert!(
        output.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}
fn rejected(dir: &Path, args: &[&str], message: &str) {
    let output = invoke(dir, args);
    assert!(!output.status.success(), "{args:?}");
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        value["error"]["message"]
            .as_str()
            .unwrap()
            .contains(message),
        "{value}"
    );
}
fn fixture(dir: &Path) {
    fs::write(
        dir.join("einzel.gem"),
        include_str!("../../../examples/data/einzel.gem"),
    )
    .unwrap();
    run(
        dir,
        &[
            "refine-geometry",
            "einzel.gem",
            "--device=cpu",
            "--electrode=1=1:0",
            "--electrode=2=2:30",
            "--electrode=3=3:0",
            "--out=lens",
        ],
    );
    // Nonmonotone births exercise source order rather than time sorting.
    let ions: String = (0..15)
        .map(|k| {
            format!(
                "{},40,1,1,{},0,0,0,{}\n",
                (k % 3) as f64 * 0.01,
                (k % 5) as f64 * 0.1,
                80 + k
            )
        })
        .collect();
    fs::write(dir.join("ions.ion"), ions).unwrap();
}
fn flight(mode: &str) -> Vec<&str> {
    let mut args = vec![
        "fly",
        "--device=cpu",
        "--pa=lens.pa0",
        "--particles=ions.ion",
        "--voltage=1=0,2=30,3=0",
        "--plane=1,0,0,58",
    ];
    if mode == "staircase" {
        args.push("--no-cuts");
    }
    args
}

#[test]
fn einzel_shards_merge_byte_identically_with_exact_statistics_and_trajectories() {
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path();
    fixture(dir);
    for mode in ["exact", "staircase"] {
        let mut base = flight(mode);
        base.extend([
            "--sample-every=4",
            "--output=full.csv",
            "--trajectories=full-trajectories.csv",
        ]);
        let full = run(dir, &base);
        let csv = fs::read(dir.join("full.csv")).unwrap();
        let trajectories = fs::read(dir.join("full-trajectories.csv")).unwrap();
        assert_eq!(full["arrivals"], 15);
        for n in [1, 2, 3, 7] {
            let mut inputs = Vec::new();
            let mut compute = Vec::new();
            for i in 1..=n {
                let shard = format!("--shard={i}/{n}");
                let path = format!("{mode}-{i}-{n}.csv");
                let output = format!("--output={path}");
                let trajectory = format!("--trajectories={mode}-{i}-{n}-trajectories.csv");
                let mut args = flight(mode);
                args.extend(["--sample-every=4", &shard, &output, &trajectory]);
                let report = run(dir, &args);
                let expected: Vec<_> = (0..15).filter(|k| k % n == i - 1).map(|k| k + 1).collect();
                let text = fs::read_to_string(dir.join(&path)).unwrap();
                let actual: Vec<usize> = text
                    .lines()
                    .skip(1)
                    .map(|l| l.split(',').next().unwrap().parse().unwrap())
                    .collect();
                assert_eq!(actual, expected);
                assert_eq!(
                    report["shard"],
                    json!({"index": i, "count": n, "ions": expected.len()})
                );
                assert_eq!(report["run_hash"], full["run_hash"]);
                compute.push(report["compute_ms"].as_f64().unwrap());
                inputs.push(path);
            }
            // Reverse input order; assembly order is the original global index.
            let mut merge = vec!["merge-flights", "merged.csv"];
            merge.extend(inputs.iter().rev().map(String::as_str));
            let merged = run(dir, &merge);
            assert_eq!(fs::read(dir.join("merged.csv")).unwrap(), csv);
            assert_eq!(
                fs::read(dir.join("merged.trajectories.csv")).unwrap(),
                trajectories
            );
            for key in [
                "ion_count",
                "splat_counts",
                "arrivals",
                "arrival_tof_us",
                "run_hash",
            ] {
                assert_eq!(merged[key], full[key], "{mode} N={n}: {key}");
            }
            assert_eq!(
                merged["compute_ms_max"].as_f64().unwrap(),
                compute.iter().copied().fold(0.0, f64::max)
            );
            assert_eq!(
                merged["compute_ms_sum"].as_f64().unwrap(),
                compute.iter().rev().sum::<f64>()
            );
            let sidecar: Value =
                serde_json::from_slice(&fs::read(dir.join("merged.csv.json")).unwrap()).unwrap();
            assert_eq!(sidecar["summary"], merged);
        }
        run(
            dir,
            &[
                "export",
                "trajectories",
                "merged.trajectories.csv",
                "merged.vtp",
            ],
        );
        assert!(fs::metadata(dir.join("merged.vtp")).unwrap().len() > 0);
    }
}

#[test]
fn merge_rejects_different_runs_missing_and_duplicate_shards_and_changed_csv() {
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path();
    fixture(dir);
    for (shard, out) in [
        ("--shard=1/2", "--output=one.csv"),
        ("--shard=2/2", "--output=two.csv"),
    ] {
        let mut args = flight("exact");
        args.extend([shard, out]);
        run(dir, &args);
    }
    let mut changed = flight("exact");
    *changed
        .iter_mut()
        .find(|a| a.starts_with("--voltage="))
        .unwrap() = "--voltage=1=0,2=31,3=0";
    changed.extend(["--shard=2/2", "--output=changed.csv"]);
    run(dir, &changed);
    rejected(
        dir,
        &["merge-flights", "out.csv", "one.csv", "changed.csv"],
        "run hash",
    );
    rejected(
        dir,
        &["merge-flights", "out.csv", "one.csv"],
        "missing shard",
    );
    rejected(
        dir,
        &["merge-flights", "out.csv", "one.csv", "one.csv"],
        "duplicate shard",
    );
    assert!(!dir.join("out.csv").exists());
    let mut text = fs::read_to_string(dir.join("two.csv")).unwrap();
    text.push_str("1,0,0,0,0,0,plane,,0,0,0\n");
    fs::write(dir.join("two.csv"), text).unwrap();
    rejected(
        dir,
        &["merge-flights", "out.csv", "one.csv", "two.csv"],
        "checksum",
    );
    assert!(!dir.join("out.csv").exists());
}

#[test]
fn sharding_refuses_coupled_physics_before_loading_inputs_and_validates_partition() {
    let dir = tempfile::tempdir().unwrap();
    for physics in [
        vec!["--space-charge=direct"],
        vec!["--space-charge=pic"],
        vec!["--charge-mode=beam", "--beam-current-a=1e-9"],
    ] {
        let mut args = vec!["fly", "--particles=missing.ion", "--shard=1/1"];
        args.extend(physics);
        rejected(dir.path(), &args, "coupled physics");
    }
    for partition in ["0/2", "3/2", "1/0", "1", "-1/2", "1/2/3"] {
        rejected(
            dir.path(),
            &[
                "fly",
                "--particles=missing.ion",
                &format!("--shard={partition}"),
            ],
            "1 <= i <= N",
        );
    }
}

#[test]
fn seeded_hs1_sds_and_fly2_sources_retain_their_global_streams() {
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path();
    fs::write(dir.join("ions.fly2"), "particles { standard_beam { n=15, mass=40, charge=1, ke=100, x=0, y=0, z=0, az=0, el=0 } }\n").unwrap();
    let values: String = (0..5)
        .flat_map(|table| {
            (0..1002).map(move |i| format!("{} ", i as f64 * (1.0 + table as f64 * 0.1)))
        })
        .collect();
    fs::write(dir.join("synthetic-mbmr.dat"), values).unwrap();
    for collision in [
        vec!["--collision=hs1", "--mean-free-path-mm=0.01"],
        vec![
            "--collision=sds",
            "--mbmr=synthetic-mbmr.dat",
            "--pressure-pa=0.53",
            "--mobility-stp=1.5",
            "--ion-diameter-nm=1.0",
        ],
    ] {
        let mut base = vec![
            "fly",
            "--device=cpu",
            "--particles=ions.fly2",
            "--seed=734",
            "--duration-us=0.02",
            "--max-step-us=0.001",
            "--allow-time-limit",
        ];
        base.extend(collision);
        let mut full = base.clone();
        full.push("--output=full.csv");
        run(dir, &full);
        let bytes = fs::read(dir.join("full.csv")).unwrap();
        let mut other_seed = base.clone();
        other_seed.retain(|a| !a.starts_with("--seed="));
        other_seed.extend(["--seed=735", "--output=other.csv"]);
        run(dir, &other_seed);
        assert_ne!(
            bytes,
            fs::read(dir.join("other.csv")).unwrap(),
            "collision randomness must be exercised"
        );
        for n in [1, 2, 3, 7, 17] {
            let mut paths = Vec::new();
            for i in 1..=n {
                let shard = format!("--shard={i}/{n}");
                let path = format!("{i}-{n}.csv");
                let out = format!("--output={path}");
                let mut args = base.clone();
                args.extend([shard.as_str(), out.as_str()]);
                run(dir, &args);
                paths.push(path);
            }
            let mut merge = vec!["merge-flights", "merged.csv"];
            merge.extend(paths.iter().map(String::as_str));
            run(dir, &merge);
            assert_eq!(bytes, fs::read(dir.join("merged.csv")).unwrap());
        }
    }
}

#[test]
fn adapter_selector_reaches_hardware_validation_and_rejects_unknown_selectors() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("ions.ion"), "0,40,1,0,0,0,0,0,100\n").unwrap();
    let probe = invoke(dir.path(), &["gpu-info", "--backend=vulkan"]);
    if !probe.status.success() {
        return;
    }
    let info: Value = serde_json::from_slice(&probe.stdout).unwrap();
    let base = [
        "fly",
        "--particles=ions.ion",
        "--device=gpu",
        "--backend=vulkan",
        "--duration-us=0.001",
        "--allow-time-limit",
    ];
    for selector in ["0", info["adapter_name"].as_str().unwrap()] {
        let option = format!("--gpu-adapter={selector}");
        let mut args = base.to_vec();
        args.push(&option);
        let output = invoke(dir.path(), &args);
        if info["software_adapter"] == true {
            assert!(!output.status.success());
            assert!(String::from_utf8_lossy(&output.stderr).contains("requires a hardware GPU"));
        } else {
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
    let mut args = base.to_vec();
    args.push("--gpu-adapter=not-a-real-simrep-adapter");
    rejected(dir.path(), &args, "not found");
    args.pop();
    args.push("--gpu-adapter=999999");
    rejected(dir.path(), &args, "not found");
}

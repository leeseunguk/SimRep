use std::fs;
use std::path::Path;
use std::process::Command;

fn run(dir: &Path, args: &[&str]) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_simrep"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{args:?}\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn adaptive_families_fast_adjust_flight_and_fly2_source() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("plates.gem"), "pa_define(9,5,1,planar)\ne(1){fill{within{box(-1,-1,0,5)}}}\ne(2){fill{within{box(8,-1,9,5)}}}").unwrap();
    let common = [
        "refine-geometry",
        "plates.gem",
        "--electrode",
        "1=3:0",
        "--electrode",
        "2=7:8",
        "--tolerance",
        "1e-12",
    ];
    let mut args = common.to_vec();
    args.extend(["--out", "adaptive", "--refine-region", "2,1,0,6,3,0:2"]);
    run(dir.path(), &args);
    let manifest: simrep_fly::adaptive::AdaptiveManifest = serde_json::from_str(
        &fs::read_to_string(dir.path().join("adaptive.adaptive.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(manifest.schema_version, 1);
    assert_eq!(manifest.patches.len(), 1);
    assert_eq!(manifest.patches[0].ghost_layers, 2);
    for number in [3, 7] {
        assert!(dir.path().join(format!("adaptive.pa{number}")).is_file());
        assert!(
            dir.path()
                .join(format!("adaptive.patch0.pa{number}"))
                .is_file()
        );
    }
    let base = simrep_io::read_pa(dir.path().join("adaptive.pa0"))
        .unwrap()
        .array;
    for (i, &v) in base.potentials.iter().enumerate() {
        assert!((v - (i % 9) as f64).abs() < 2e-8);
    }
    fs::write(dir.path().join("ions.ion"), "0,1,-1,3,2,0,0,0,1\n").unwrap();
    for (pa, output) in [
        ("adaptive.pa0", "base.csv"),
        ("adaptive.adaptive.json", "patch.csv"),
    ] {
        run(
            dir.path(),
            &[
                "fly",
                "--pa",
                pa,
                "--particles",
                "ions.ion",
                "--device",
                "cpu",
                "--plane",
                "1,0,0,5",
                "--tolerance",
                "1e-10",
                "--output",
                output,
            ],
        );
    }
    let a = fs::read_to_string(dir.path().join("base.csv")).unwrap();
    let b = fs::read_to_string(dir.path().join("patch.csv")).unwrap();
    // Compare numerical CSV columns, not solver-dependent text formatting.
    let row_a: Vec<_> = a.lines().nth(1).unwrap().split(',').collect();
    let row_b: Vec<_> = b.lines().nth(1).unwrap().split(',').collect();
    for (a, b) in row_a.iter().zip(&row_b) {
        if let (Ok(a), Ok(b)) = (a.parse::<f64>(), b.parse::<f64>()) {
            assert!((a - b).abs() < 2e-7, "{row_a:?} != {row_b:?}");
        } else {
            assert_eq!(a, b);
        }
    }
    // A known offset proves the FLY2 provider reads the patch, not just PA0.
    let patch_path = dir.path().join("adaptive.patch0.pa0");
    let mut patch = simrep_io::read_pa(&patch_path).unwrap();
    assert!(!patch.array.electrodes.iter().any(|&e| e));
    for v in &mut patch.array.potentials {
        *v += 100.0;
    }
    simrep_io::write_pa(&patch_path, &patch).unwrap();
    fs::write(dir.path().join("source.fly2"),
        "assert(math.abs(simion.wb:epotential(4,2,0)-108)<1e-6)\nparticles{standard_beam{n=1,mass=1,charge=-1,x=3,y=2,z=0,ke=1,az=0,el=0}}").unwrap();
    run(
        dir.path(),
        &[
            "fly",
            "--pa",
            "adaptive.adaptive.json",
            "--particles",
            "source.fly2",
            "--voltage",
            "7=16",
            "--device",
            "cpu",
            "--plane",
            "1,0,0,5",
            "--output",
            "source.csv",
        ],
    );
}

#[test]
fn absent_refine_flag_preserves_uniform_family_bytes() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("plates.gem"), "pa_define(9,5,1,planar)\ne(1){fill{within{box(-1,-1,0,5)}}}\ne(2){fill{within{box(8,-1,9,5)}}}").unwrap();
    for stem in ["first", "second"] {
        run(
            dir.path(),
            &[
                "refine-geometry",
                "plates.gem",
                "--electrode",
                "1=3:0",
                "--electrode",
                "2=7:8",
                "--out",
                stem,
            ],
        );
    }
    for extension in ["pa0", "pa3", "pa7", "pa#", "cuts.json"] {
        assert_eq!(
            fs::read(dir.path().join(format!("first.{extension}"))).unwrap(),
            fs::read(dir.path().join(format!("second.{extension}"))).unwrap()
        );
    }
    assert!(!dir.path().join("first.adaptive.json").exists());
}

#[test]
fn auto_selection_reports_nodes_and_preserves_affine_potential() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("plates.gem"), "pa_define(9,5,1,planar)\ne(1){fill{within{box(-1,-1,0,5)}}}\ne(2){fill{within{box(8,-1,9,5)}}}").unwrap();
    let mut selection = None;
    for stem in ["first", "second"] {
        let text = run(
            dir.path(),
            &[
                "--json",
                "refine-geometry",
                "plates.gem",
                "--electrode",
                "1=3:0",
                "--electrode",
                "2=7:8",
                "--refine",
                "auto",
                "--gap-check",
                "warn",
                "--out",
                stem,
            ],
        );
        let summary: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(summary["schema_version"], 1);
        let report = &summary["auto_refinement"];
        if let Some(previous) = &selection {
            assert_eq!(previous, report);
        }
        selection = Some(report.clone());
        assert_eq!(report["base_nodes"], 45);
        assert_eq!(report["patch_nodes"], 153);
        assert_eq!(report["stored_nodes"], 198);
        assert_eq!(report["boxes"][0]["factor"], 2);
        assert!(
            summary["adaptive_profiles"][0]["phases"]["mg_cycles"]
                .as_u64()
                .unwrap()
                > 0
        );
        let pa = simrep_io::read_pa(dir.path().join(format!("{stem}.pa0"))).unwrap();
        for (i, v) in pa.array.potentials.iter().enumerate() {
            assert!((v - (i % 9) as f64).abs() < 2e-7);
        }
    }
    for extension in [
        "pa0",
        "pa3",
        "pa7",
        "pa#",
        "cuts.json",
        "patch0.pa0",
        "patch0.pa3",
        "patch0.pa7",
    ] {
        assert_eq!(
            fs::read(dir.path().join(format!("first.{extension}"))).unwrap(),
            fs::read(dir.path().join(format!("second.{extension}"))).unwrap()
        );
    }
}

#[test]
fn auto_rejects_invalid_factors_and_unsupported_geometry() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("cyl.gem"),
        "pa_define(9,5,1,cylindrical)\ne(1){fill{within{box(-1,-1,0,5)}}}",
    )
    .unwrap();
    for flag in ["auto", "auto:3", "uniform"] {
        let output = Command::new(env!("CARGO_BIN_EXE_simrep"))
            .current_dir(dir.path())
            .args([
                "refine-geometry",
                "cyl.gem",
                "--electrode",
                "1=1:0",
                "--refine",
                flag,
                "--out",
                "bad",
            ])
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(!dir.path().join("bad.pa0").exists());
    }
}

#[test]
fn invalid_regions_and_unsupported_devices_fail() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("plates.gem"),
        "pa_define(9,5,1,planar)\ne(1){fill{within{box(-1,-1,0,5)}}}",
    )
    .unwrap();
    for region in ["1,1,0,5,3,0:3", "1.1,1,0,5,3,0:2", "-1,1,0,5,3,0:2"] {
        let status = Command::new(env!("CARGO_BIN_EXE_simrep"))
            .current_dir(dir.path())
            .args([
                "refine-geometry",
                "plates.gem",
                "--electrode",
                "1=1:0",
                "--out",
                "bad",
                "--refine-region",
                region,
            ])
            .output()
            .unwrap();
        assert!(!status.status.success());
    }
}

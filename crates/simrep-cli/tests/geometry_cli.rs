use std::fs;
use std::path::Path;
use std::process::Command;

fn run(dir: &Path, args: &[&str]) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_simrep"))
        .args(args)
        .current_dir(dir)
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
fn gem_basis_flags_snap_sidecar_and_fast_adjust_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("plates.gem"),
        "pa_define(9,5,1,planar)\ne(1){fill{within{box(-1,-1,2.95,5)}}}\ne(2){fill{within{box(6,-1,9,5)}}}",
    ).unwrap();
    run(
        dir.path(),
        &[
            "refine-geometry",
            "plates.gem",
            "--electrode",
            "1=3:-7",
            "--electrode",
            "2=7:19",
            "--out",
            "plates",
            "--sidecar-dense-sdf",
            "--tolerance",
            "1e-12",
        ],
    );
    let base = simrep_io::read_pa(dir.path().join("plates.pa0")).unwrap();
    let sidecar: serde_json::Value =
        serde_json::from_slice(&fs::read(dir.path().join("plates.cuts.json")).unwrap()).unwrap();
    let metadata = base.pa0.as_ref().unwrap();
    assert_eq!(sidecar["version"], 3);
    assert_eq!(sidecar["electrode_numbers"], serde_json::json!([3, 7]));
    assert_eq!(
        sidecar["sampled_sdfs"][0]["dims"],
        serde_json::json!([17, 9, 1])
    );
    assert_eq!(metadata.reference_voltage, 10000.0);
    for number in [3, 7] {
        let i = metadata.electrode_indices[number] as usize;
        assert!(base.array.electrodes[i]);
    }
    for y in 0..5 {
        for x in 0..9 {
            let i = x + 9 * y;
            assert_eq!(base.array.electrodes[i], x <= 2 || x >= 6);
        }
    }
    // x=3 is positive-distance material snapped only in the solver.
    assert!(!base.array.electrodes[3]);
    assert_eq!(sidecar["solver_fixed"][3], true);
    assert_eq!(sidecar["node_electrodes"][3], serde_json::Value::Null);
    assert!((sidecar["cut_fractions"][3][0].as_f64().unwrap() - 0.05).abs() < 1e-8);
    assert_eq!(sidecar["cut_electrodes"][3][0], 3);
    // x=6 is exactly on the surface and must be a file electrode.
    assert!(base.array.electrodes[6]);
    let adjusted =
        simrep_io::read_fast_adjust(dir.path().join("plates.pa0"), &[(3, 5.0), (7, 11.0)]).unwrap();
    for (i, &fixed) in adjusted.electrodes.iter().enumerate() {
        if fixed {
            assert!((adjusted.potentials[i] - if i % 9 <= 2 { 5.0 } else { 11.0 }).abs() < 1e-12);
        }
    }
    let first = simrep_io::read_pa(dir.path().join("plates.pa3")).unwrap();
    let second = simrep_io::read_pa(dir.path().join("plates.pa7")).unwrap();
    for (i, &v) in adjusted.potentials.iter().enumerate() {
        assert!(
            (v - (5.0 * first.array.potentials[i] + 11.0 * second.array.potentials[i]) / 10000.0)
                .abs()
                < 1e-10
        );
    }
    run(
        dir.path(),
        &[
            "refine-geometry",
            "plates.gem",
            "--electrode",
            "1=3:-7",
            "--electrode",
            "2=7:19",
            "--out",
            "compact",
            "--tolerance",
            "1e-12",
        ],
    );
    let compact: serde_json::Value =
        serde_json::from_slice(&fs::read(dir.path().join("compact.cuts.json")).unwrap()).unwrap();
    assert_eq!(compact["sampled_sdfs"], serde_json::json!([]));
    assert!(compact.get("cut_fractions").is_none());
    let links: Vec<_> = sidecar["cut_fractions"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
        .filter(|(_, row)| row.as_array().unwrap().iter().any(|v| !v.is_null()))
        .map(|(node, fractions)| {
            serde_json::json!({
                "node":node, "fractions":fractions,
                "electrodes":sidecar["cut_electrodes"][node]
            })
        })
        .collect();
    assert_eq!(compact["link_intersections"], serde_json::json!(links));
    assert_eq!(compact["source_geometry"], sidecar["source_geometry"]);
    for (runs, dense) in [
        ("node_electrode_runs", "node_electrodes"),
        ("solver_fixed_runs", "solver_fixed"),
    ] {
        let mut expanded = Vec::new();
        for run in compact[runs].as_array().unwrap() {
            let start = run[0].as_u64().unwrap() as usize;
            let end = run[1].as_u64().unwrap() as usize;
            assert_eq!(expanded.len(), start);
            expanded.resize(end, run[2].clone());
        }
        assert_eq!(serde_json::json!(expanded), sidecar[dense]);
    }
    for extension in ["pa0", "pa3", "pa7", "pa#"] {
        assert_eq!(
            fs::read(dir.path().join(format!("plates.{extension}"))).unwrap(),
            fs::read(dir.path().join(format!("compact.{extension}"))).unwrap()
        );
    }
    let mut legacy = sidecar.clone();
    legacy["version"] = serde_json::json!(2);
    let fallback =
        simrep_fly::surface::SurfaceModel::from_reader(legacy.to_string().as_bytes()).unwrap();
    assert_eq!(
        fallback.accuracy(),
        simrep_fly::surface::SurfaceAccuracy::SampledSdf
    );
    run(
        dir.path(),
        &["refine", "plates.pa#", "--tolerance", "1e-12"],
    );
}

#[test]
fn aligned_gem_surfaces_match_the_staircase_operator() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("aligned.gem"),
        "pa_define(9,5,1,planar)\ne(1){fill{within{box(-1,-1,2,5)}}}\ne(2){fill{within{box(6,-1,9,5)}}}",
    ).unwrap();
    run(
        dir.path(),
        &[
            "refine-geometry",
            "aligned.gem",
            "--electrode",
            "1=1:0",
            "--electrode",
            "2=2:17",
            "--out",
            "aligned",
            "--tolerance",
            "1e-12",
        ],
    );
    let cut = simrep_io::read_pa(dir.path().join("aligned.pa0"))
        .unwrap()
        .array;
    run(
        dir.path(),
        &["refine", "aligned.pa#", "--tolerance", "1e-12"],
    );
    let stair = simrep_io::read_fast_adjust(dir.path().join("aligned.pa0"), &[(2, 17.0)]).unwrap();
    assert_eq!(cut.electrodes, stair.electrodes);
    for (a, b) in cut.potentials.iter().zip(&stair.potentials) {
        assert!((a - b).abs() < 1e-7);
    }
}

#[test]
fn mesh_formats_feed_the_continuous_pipeline() {
    let dir = tempfile::tempdir().unwrap();
    let part = simrep_mesh::Part {
        name: "cube".into(),
        vertices: vec![
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
            [1.0, 0.0, 1.0],
            [1.0, 1.0, 1.0],
            [0.0, 1.0, 1.0],
        ],
        triangles: vec![
            [0, 2, 1],
            [0, 3, 2],
            [4, 5, 6],
            [4, 6, 7],
            [0, 1, 5],
            [0, 5, 4],
            [1, 2, 6],
            [1, 6, 5],
            [2, 3, 7],
            [2, 7, 6],
            [3, 0, 4],
            [3, 4, 7],
        ],
    };
    let mesh = simrep_mesh::Mesh { parts: vec![part] };
    for extension in ["stl", "obj", "gltf", "glb"] {
        let input = format!("cube.{extension}");
        simrep_mesh::write_mesh(dir.path().join(&input), &mesh).unwrap();
        run(
            dir.path(),
            &[
                "refine-geometry",
                &input,
                "--grid",
                "5,5,5",
                "--spacing",
                "0.5,0.5,0.5",
                "--origin",
                "-0.5,-0.5,-0.5",
                "--electrode",
                "0=1:12",
                "--out",
                "cube",
            ],
        );
        let a = simrep_io::read_pa(dir.path().join("cube.pa0"))
            .unwrap()
            .array;
        assert_eq!(a.electrodes.iter().filter(|&&f| f).count(), 27);
        assert!(a.potentials.iter().all(|v| (v - 12.0).abs() < 1e-7));
        let sidecar: serde_json::Value =
            serde_json::from_slice(&fs::read(dir.path().join("cube.cuts.json")).unwrap()).unwrap();
        assert_eq!(sidecar["sampled_sdfs"], serde_json::json!([]));
        let surface = simrep_fly::surface::SurfaceModel::from_reader(
            fs::File::open(dir.path().join("cube.cuts.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            surface.accuracy(),
            simrep_fly::surface::SurfaceAccuracy::SourceGeometry
        );
        assert_eq!(surface.membership([0.1, 0.2, 0.3]), Some(1));
        let (t, number) = surface
            .first_hit([-0.4, 0.2, 0.3], [1.4, 0.2, 0.3])
            .unwrap();
        assert_eq!(number, 1);
        assert!((-0.4 + 1.8 * t).abs() < 1e-12);
    }
}

#[test]
fn fly_picks_up_v3_sidecar_and_reports_electrode_number() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("plates.gem"),
        "pa_define(9,5,1,planar)\ne(1){fill{within{box(-1,-1,2.95,5)}}}\ne(2){fill{within{box(6,-1,9,5)}}}",
    ).unwrap();
    run(
        dir.path(),
        &[
            "refine-geometry",
            "plates.gem",
            "--electrode",
            "1=3:0",
            "--electrode",
            "2=7:0",
            "--surface-supersampling",
            "4",
            "--out",
            "plates",
        ],
    );
    // x=3.1 is vacuum on the physical surface, but lies in the nearest-node
    // voxel of x=3 only in this case; fire toward the right plate.
    fs::write(dir.path().join("one.ion"), "0,100,1,3.1,2,0,0,0,1,1\n").unwrap();
    let args = [
        "fly",
        "--pa",
        "plates.pa0",
        "--particles",
        "one.ion",
        "--nonrelativistic",
        "--device",
        "auto",
    ];
    let output = Command::new(env!("CARGO_BIN_EXE_simrep"))
        .current_dir(dir.path())
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("source geometry (v3)"));
    assert!(stderr.contains("surface: plates.cuts.json (automatic sibling pickup;"));
    assert!(stderr.contains("auto: CPU (surface collisions require CPU)"));
    assert!(stderr.contains("device: cpu"));
    let text = String::from_utf8(output.stdout).unwrap();
    let row: Vec<_> = text.lines().nth(1).unwrap().split(',').collect();
    assert!(
        (row[2].parse::<f64>().unwrap() - 6.0).abs() < 1e-8,
        "{text}"
    );
    assert_eq!(row[7], "7");
    let staircase_output = Command::new(env!("CARGO_BIN_EXE_simrep"))
        .current_dir(dir.path())
        .args([
            "fly",
            "--pa",
            "plates.pa0",
            "--particles",
            "one.ion",
            "--nonrelativistic",
            "--no-cuts",
        ])
        .output()
        .unwrap();
    assert!(staircase_output.status.success());
    assert!(
        String::from_utf8_lossy(&staircase_output.stderr)
            .contains("surface: staircase (--no-cuts disables sibling pickup)")
    );
    let staircase = String::from_utf8(staircase_output.stdout).unwrap();
    let row: Vec<_> = staircase.lines().nth(1).unwrap().split(',').collect();
    use simrep_fly::Field;
    let grid = simrep_fly::GridField::new(
        simrep_io::read_pa(dir.path().join("plates.pa0"))
            .unwrap()
            .array,
    )
    .unwrap();
    let expected = 3.1
        + (8.0 - 3.1)
            * grid
                .intersection([3.1, 2.0, 0.0], [8.0, 2.0, 0.0])
                .unwrap()
                .0;
    assert!(
        (row[2].parse::<f64>().unwrap() - expected).abs() < 1e-8,
        "{staircase}"
    );
    assert_eq!(row[7], "");
    let output = Command::new(env!("CARGO_BIN_EXE_simrep"))
        .current_dir(dir.path())
        .args([
            "fly",
            "--pa",
            "plates.pa0",
            "--particles",
            "one.ion",
            "--device",
            "gpu",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("surface collisions require CPU"));
    // Explicit path loading is independent of the stem, and missing files fail.
    fs::rename(
        dir.path().join("plates.cuts.json"),
        dir.path().join("explicit.json"),
    )
    .unwrap();
    run(
        dir.path(),
        &[
            "fly",
            "--pa",
            "plates.pa0",
            "--particles",
            "one.ion",
            "--cuts",
            "explicit.json",
        ],
    );
}

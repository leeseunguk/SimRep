use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use serde_json::{Value, json};

fn invoke(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_simrep"))
        .arg("--json")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap()
}

fn object(output: &Output) -> Value {
    // from_slice rejects progress, CSV, or a second JSON value on stdout.
    let value: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "{error}: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    assert!(value.is_object());
    assert_eq!(value["schema_version"], 1);
    assert_eq!(value["simrep_version"], env!("CARGO_PKG_VERSION"));
    value
}

fn run(dir: &Path, args: &[&str]) -> Value {
    let output = invoke(dir, args);
    assert!(
        output.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    object(&output)
}

fn numbers(value: &Value, keys: &[&str]) {
    for key in keys {
        assert!(value[key].is_number(), "{key}: {value}");
    }
}

fn paths(dir: &Path, value: &Value) {
    let paths = value["output_paths"].as_array().unwrap();
    assert!(!paths.is_empty());
    for path in paths {
        assert!(dir.join(path.as_str().unwrap()).is_file(), "{path}");
    }
}

fn plates(dir: &Path) {
    fs::write(
        dir.join("plates.gem"),
        "pa_define(9,5,1,planar)\ne(1){fill{within{box(-1,-1,2,5)}}}\ne(2){fill{within{box(6,-1,9,5)}}}",
    )
    .unwrap();
}

fn refinement(value: &Value) {
    assert_eq!(value["requested"]["device"], "cpu");
    assert_eq!(value["executed"]["device"], "cpu");
    assert_eq!(value["executed"]["precision"], "f64");
    assert!(value["executed"]["backend"].is_null());
    assert!(value["reason"].is_string());
    assert!(value["fallback_reason"].is_null());
    assert!(value["cycles"].is_u64());
    assert_eq!(value["converged"], true);
    numbers(
        value,
        &["final_residual", "tolerance", "compute_ms", "prep_ms"],
    );
    assert!(value["final_residual"].as_f64().unwrap() <= value["tolerance"].as_f64().unwrap());
}

#[test]
fn gem_refine_geometry_and_pa_info_have_structured_summaries() {
    let dir = tempfile::tempdir().unwrap();
    plates(dir.path());
    let gem = run(dir.path(), &["gem2pa", "plates.gem"]);
    assert_eq!(gem["dims"], json!([9, 5, 1]));
    assert_eq!(gem["spacing_mm"], json!([1.0, 1.0, 1.0]));
    assert_eq!(gem["electrode_numbers"], json!([1, 2]));
    paths(dir.path(), &gem);

    let refined = run(dir.path(), &["refine", "plates.pa#", "--device=cpu"]);
    refinement(&refined);
    paths(dir.path(), &refined);
    assert_eq!(refined["output_paths"].as_array().unwrap().len(), 3);

    let geometry = run(
        dir.path(),
        &[
            "refine-geometry",
            "plates.gem",
            "--device=cpu",
            "--electrode=1=1:0",
            "--electrode=2=2:17",
            "--out=geometry",
        ],
    );
    refinement(&geometry);
    paths(dir.path(), &geometry);
    assert_eq!(geometry["output_paths"].as_array().unwrap().len(), 5);

    let info = run(dir.path(), &["pa-info", "geometry.pa0"]);
    assert_eq!(info["dims"], json!([9, 5, 1]));
    assert_eq!(info["field_type"], "electrostatic");
    assert_eq!(info["symmetry"], "planar");
    for key in ["mirrors", "anti_mirrors"] {
        assert_eq!(info[key].as_array().unwrap().len(), 3);
        assert!(info[key].as_array().unwrap().iter().all(Value::is_boolean));
    }
    assert!(info["refinable"].is_boolean());
    numbers(
        &info,
        &[
            "mode",
            "header_bytes",
            "ng",
            "max_voltage",
            "points",
            "electrode_points",
            "non_electrode_points",
            "potential_min",
            "potential_max",
            "potential_mean",
            "adjustable_electrodes",
            "reference_voltage",
        ],
    );
}

#[test]
fn cuts_fill_fly2_source_and_dynamic_bases_and_report_unavailable_voltages() {
    use simrep_fly::{Field, GridField, surface::SurfaceModel};
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("cuts.gem"),
        "pa_define(17,9,1,planar)\ne(1){fill{within{box(-1,-1,2.3,9)}}}\ne(2){fill{within{box(13.7,-1,17,9)}}}").unwrap();
    run(
        dir.path(),
        &[
            "refine-geometry",
            "cuts.gem",
            "--device=cpu",
            "--electrode=1=1:0",
            "--electrode=2=2:17",
            "--out=cuts",
        ],
    );
    let surface =
        SurfaceModel::from_reader(fs::File::open(dir.path().join("cuts.cuts.json")).unwrap())
            .unwrap();
    let array = simrep_io::read_fast_adjust(dir.path().join("cuts.pa0"), &[(2, 17.0)]).unwrap();
    let raw = GridField::new(array.clone()).unwrap();
    let field = raw.clone().with_surface(&surface).unwrap();
    let point = [2.31, 4.0, 0.0];
    let expected = field.sample(point, 0.0).potential_v;
    assert!((raw.sample(point, 0.0).potential_v - expected).abs() > 1e-5);
    fs::write(dir.path().join("source.fly2"), format!(
        "assert(math.abs(simion.wb:epotential(2.31,4,0)-({expected:.17e}))<1e-10)\nparticles{{standard_beam{{n=1,mass=40,charge=1,x=7,y=4,z=0,ke=100,az=0,el=0}}}}"
    )).unwrap();
    let flight = run(
        dir.path(),
        &[
            "fly",
            "--device=cpu",
            "--pa=cuts.pa0",
            "--particles=source.fly2",
            "--voltage-expr=2=17",
            "--plane=1,0,0,9",
        ],
    );
    assert_eq!(flight["executed"]["surface_mode"], "source-geometry");
    assert_eq!(flight["ghost_fill"]["skipped_electrodes"], json!([]));
    assert_eq!(flight["ghost_fill"]["kept_nodes"], 0);
    for degree in 0..3 {
        assert_eq!(
            flight["ghost_fill"]["degree_counts"][degree],
            2 * field.ghost_fill_report().unwrap().degree_counts[degree]
        );
    }
    fs::write(dir.path().join("ions.ion"), "0,40,1,7,4,0,0,0,100\n").unwrap();
    for missing in [false, true] {
        let mut unavailable = array.clone();
        for (i, value) in unavailable.potentials.iter_mut().enumerate() {
            if unavailable.electrodes[i] {
                *value += (i / unavailable.nx) as f64 * 0.001;
            }
        }
        if missing {
            unavailable.electrodes.fill(false);
        }
        simrep_io::write_pa(
            dir.path().join("unavailable.pa"),
            &simrep_io::PaFile::new(unavailable),
        )
        .unwrap();
        let flight = run(
            dir.path(),
            &[
                "fly",
                "--device=cpu",
                "--pa=unavailable.pa",
                "--cuts=cuts.cuts.json",
                "--particles=ions.ion",
                "--plane=1,0,0,9",
            ],
        );
        assert_eq!(flight["ghost_fill"]["skipped_electrodes"], json!([1, 2]));
        assert_eq!(flight["ghost_fill"]["degree_counts"], json!([0, 0, 0]));
    }
}

#[test]
fn fly_summarizes_arrivals_and_never_mixes_csv_with_json() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("ions.ion"), "0,1,1,0,0,0,0,0,1,1\n").unwrap();
    let args = [
        "fly",
        "--device=cpu",
        "--particles=ions.ion",
        "--bounds=-1,-1,-1,3,1,1",
        "--plane=1,0,0,1",
        "--duration-us=1",
    ];
    let output = invoke(dir.path(), &args);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value = object(&output);
    assert!(String::from_utf8_lossy(&output.stderr).contains("ion,tof_us"));
    assert_eq!(value["ion_count"], 1);
    assert_eq!(value["arrivals"], 1);
    assert_eq!(value["splat_counts"]["plane"], 1);
    for reason in [
        "electrode",
        "outside",
        "plane",
        "time_limit",
        "step_limit",
        "step_underflow",
        "nonfinite",
        "beam_diverged",
        "beam_leader_stopped",
    ] {
        assert!(value["splat_counts"][reason].is_u64());
    }
    numbers(&value["arrival_tof_us"], &["mean", "std", "min", "max"]);
    assert_eq!(value["arrival_tof_us"]["std"], 0.0);
    numbers(&value, &["compute_ms", "prep_ms"]);
    assert_eq!(value["executed"]["device"], "cpu");
    assert_eq!(value["executed"]["precision"], "f64");
    assert_eq!(value["executed"]["surface_mode"], "staircase");
    assert!(value["executed"]["backend"].is_null());
    assert_eq!(value["requested"]["device"], "cpu");
    assert_eq!(value["requested"]["precision"], "f64");
    assert!(value["requested"]["backend"].is_string());
    assert_eq!(value["output_paths"], json!([]));
    let mut file_args = args.to_vec();
    file_args.push("--output=flight.csv");
    paths(dir.path(), &run(dir.path(), &file_args));

    let no_arrivals = run(
        dir.path(),
        &[
            "fly",
            "--particles=ions.ion",
            "--device=auto",
            "--duration-us=0.00001",
            "--allow-time-limit",
        ],
    );
    assert_eq!(no_arrivals["arrivals"], 0);
    assert_eq!(no_arrivals["splat_counts"]["time_limit"], 1);
    assert!(no_arrivals["fallback_reason"].is_string());
    for key in ["mean", "std", "min", "max"] {
        assert!(no_arrivals["arrival_tof_us"][key].is_null());
    }
}

#[test]
fn mesh_import_and_each_export_report_all_written_paths() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("cube.obj"), "o cube\nv 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nv 0 0 1\nv 1 0 1\nv 1 1 1\nv 0 1 1\nf 1 4 3 2\nf 5 6 7 8\nf 1 2 6 5\nf 2 3 7 6\nf 3 4 8 7\nf 4 1 5 8\n").unwrap();
    let imported = run(
        dir.path(),
        &[
            "mesh-import",
            "cube.obj",
            "cube.pa#",
            "--grid=5,5,5",
            "--spacing=0.5,0.5,0.5",
            "--origin=-0.5,-0.5,-0.5",
            "--electrodes=7",
            "--repair",
            "--sdf-prefix=cube-sdf",
        ],
    );
    assert_eq!(imported["dims"], json!([5, 5, 5]));
    assert_eq!(imported["spacing_mm"], json!([0.5, 0.5, 0.5]));
    assert_eq!(imported["electrode_numbers"], json!([7]));
    assert_eq!(imported["output_paths"].as_array().unwrap().len(), 3);
    paths(dir.path(), &imported);
    for extension in ["stl", "obj", "gltf", "glb"] {
        let output = format!("surface.{extension}");
        let exported = run(dir.path(), &["export", "geometry", "cube.pa#", &output]);
        paths(dir.path(), &exported);
    }
    paths(
        dir.path(),
        &run(dir.path(), &["export", "grid", "cube.pa#", "field.vti"]),
    );
    paths(
        dir.path(),
        &run(
            dir.path(),
            &[
                "export",
                "isosurface",
                "cube.pa#",
                "iso.glb",
                "--potential=3.5",
            ],
        ),
    );
    fs::write(dir.path().join("samples.csv"), "trajectory_id,time_us,x_mm,y_mm,z_mm,vx_mm_per_us,vy_mm_per_us,vz_mm_per_us\n1,0,0,0,0,1,0,0\n1,1,1,0,0,1,0,0\n").unwrap();
    paths(
        dir.path(),
        &run(
            dir.path(),
            &["export", "trajectories", "samples.csv", "paths.vtp"],
        ),
    );
}

#[test]
fn cpu_benchmarks_support_the_global_flag_after_the_subcommand() {
    let dir = tempfile::tempdir().unwrap();
    for (args, key) in [
        (vec!["fly-bench", "--ions=2"], "checksum"),
        (
            vec!["refine-bench", "--device=cpu", "--n=7"],
            "final_residual",
        ),
        (vec!["refine-sdf", "--device=cpu", "--n=9"], "reports"),
    ] {
        assert!(run(dir.path(), &args).get(key).is_some());
    }
    let output = Command::new(env!("CARGO_BIN_EXE_simrep"))
        .args(["fly-bench", "--ions=2", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(object(&output)["checksum"].is_number());
}

#[test]
fn gpu_queries_use_any_available_adapter_or_report_an_honest_skip() {
    let dir = tempfile::tempdir().unwrap();
    let output = invoke(dir.path(), &["gpu-info", "--backend=vulkan"]);
    let info = object(&output);
    if !output.status.success() {
        assert!(info["error"]["message"].is_string());
        eprintln!(
            "SKIP GPU JSON checks: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    for key in ["adapter_name", "adapter_type", "backend"] {
        assert!(info[key].is_string());
    }
    for key in ["f64_support", "software_adapter"] {
        assert!(info[key].is_boolean());
    }
    let smoke = run(dir.path(), &["gpu-smoke", "--backend=vulkan"]);
    assert!(!smoke["checks"].as_array().unwrap().is_empty());
    for check in smoke["checks"].as_array().unwrap() {
        numbers(
            check,
            &[
                "submit_readback_ms",
                "max_abs_error",
                "tolerance",
                "cells_checked",
            ],
        );
    }
}

#[test]
fn errors_have_one_json_object_and_exit_one() {
    let dir = tempfile::tempdir().unwrap();
    for args in [
        vec!["pa-info", "missing.pa"],
        vec!["refine", "invalid.txt", "--device=cpu"],
        vec!["fly", "--device=invalid"],
    ] {
        let output = invoke(dir.path(), &args);
        assert_eq!(output.status.code(), Some(1));
        let value = object(&output);
        assert!(value["error"]["message"].is_string());
        assert!(value["error"]["kind"].is_string());
        assert!(!output.stderr.is_empty());
    }
}

#[test]
fn cuts_sources_and_warning_are_reported() {
    let dir = tempfile::tempdir().unwrap();
    plates(dir.path());
    run(
        dir.path(),
        &[
            "refine-geometry",
            "plates.gem",
            "--device=cpu",
            "--electrode=1=1:0",
            "--electrode=2=2:0",
            "--out=field",
        ],
    );
    fs::write(dir.path().join("ions.ion"), "0,1,1,3,2,0,0,0,1\n").unwrap();
    let base = [
        "fly",
        "--device=cpu",
        "--pa=field.pa0",
        "--particles=ions.ion",
        "--duration-us=0.001",
        "--allow-time-limit",
    ];
    for (extra, source, path) in [
        (
            Some("--cuts=field.cuts.json"),
            "explicit",
            Some("field.cuts.json"),
        ),
        (None, "sibling", Some("field.cuts.json")),
        (Some("--no-cuts"), "disabled", None),
    ] {
        let mut args = base.to_vec();
        if let Some(extra) = extra {
            args.push(extra);
        }
        let output = invoke(dir.path(), &args);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let value = object(&output);
        assert_eq!(value["executed"]["cuts_source"], source);
        assert_eq!(value["executed"]["cuts_path"], json!(path));
        assert!(
            !String::from_utf8_lossy(&output.stderr).contains("warning: no exact-surface sidecar")
        );
    }
    fs::remove_file(dir.path().join("field.cuts.json")).unwrap();
    let output = invoke(dir.path(), &base);
    assert!(output.status.success());
    assert_eq!(object(&output)["executed"]["cuts_source"], "none");
    assert!(object(&output)["executed"]["cuts_path"].is_null());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        stderr
            .lines()
            .filter(|line| line.starts_with("warning:"))
            .collect::<Vec<_>>(),
        vec![
            "warning: no exact-surface sidecar field.cuts.json next to field.pa0; flying with staircase collisions (pass --cuts PATH, or --no-cuts to silence)"
        ]
    );
}

#[test]
fn pa_info_reports_cylindrical_spacing_incompatibility() {
    let dir = tempfile::tempdir().unwrap();
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&(-2_i32).to_le_bytes());
    bytes.extend_from_slice(&0_i32.to_le_bytes());
    bytes.extend_from_slice(&100_000_f64.to_le_bytes());
    for dimension in [3_i32, 3, 1] {
        bytes.extend_from_slice(&dimension.to_le_bytes());
    }
    bytes.extend_from_slice(&2_i32.to_le_bytes());
    for spacing in [1.0_f64, 1.0, 2.0] {
        bytes.extend_from_slice(&spacing.to_le_bytes());
    }
    bytes.extend_from_slice(&[0; 72]);
    fs::write(dir.path().join("bad.pa"), bytes).unwrap();
    let info = run(dir.path(), &["pa-info", "bad.pa"]);
    assert_eq!(info["simion_compatible"], false);
    assert_eq!(
        info["simion_issues"],
        json!(["cylindrical symmetry requires dy_mm == dz_mm in SIMION 2024"])
    );
}

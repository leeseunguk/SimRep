use std::fs;
use std::process::Command;

#[test]
fn mesh_import_and_all_export_commands_run_end_to_end() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("cube.obj");
    fs::write(&input,"o cube\nv 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nv 0 0 1\nv 1 0 1\nv 1 1 1\nv 0 1 1\nf 1 4 3 2\nf 5 6 7 8\nf 1 2 6 5\nf 2 3 7 6\nf 3 4 8 7\nf 4 1 5 8\n").unwrap();
    let run = |args: &[&str]| {
        let output = Command::new(env!("CARGO_BIN_EXE_simrep"))
            .args(args)
            .current_dir(dir.path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "args={args:?}\nstdout={}\nstderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    };
    run(&[
        "mesh-import",
        "cube.obj",
        "cube.pa#",
        "--grid",
        "5,5,5",
        "--spacing",
        "0.5,0.5,0.5",
        "--origin",
        "-0.5,-0.5,-0.5",
        "--electrodes",
        "7",
        "--repair",
        "--sdf-prefix",
        "cube-sdf",
        "--voltages",
        "120",
    ]);
    let pa = simrep_io::read_pa(dir.path().join("cube.pa#"))
        .unwrap()
        .array;
    assert_eq!(pa.electrodes.iter().filter(|&&v| v).count(), 27);
    let sdf: serde_json::Value =
        serde_json::from_slice(&fs::read(dir.path().join("cube-sdf-0.json")).unwrap()).unwrap();
    assert_eq!(sdf["dims"], serde_json::json!([5, 5, 5]));
    assert_eq!(sdf["electrode_number"], 7);
    assert_eq!(sdf["voltage"], 120.0);
    assert_eq!(sdf["values"].as_array().unwrap().len(), 125);
    assert!(sdf["values"][62].as_f64().unwrap() < 0.0);
    for extension in ["stl", "obj", "gltf", "glb"] {
        let output = format!("electrode.{extension}");
        run(&["export", "geometry", "cube.pa#", &output]);
        assert_eq!(
            simrep_mesh::read_mesh(dir.path().join(output), Default::default())
                .unwrap()
                .parts
                .len(),
            1
        );
    }
    run(&["export", "grid", "cube.pa#", "cube.vti"]);
    run(&[
        "export",
        "isosurface",
        "cube.pa#",
        "iso.glb",
        "--potential",
        "3.5",
    ]);
    fs::write(dir.path().join("samples.csv"),"trajectory_id,time_us,x_mm,y_mm,z_mm,vx_mm_per_us,vy_mm_per_us,vz_mm_per_us\n1,0,0,0,0,1,0,0\n1,1,1,0,0,1,0,0\n").unwrap();
    run(&["export", "trajectories", "samples.csv", "paths.vtp"]);
    run(&["export", "trajectories", "samples.csv", "paths.csv"]);
    assert!(fs::metadata(dir.path().join("paths.vtp")).unwrap().len() > 100);
}

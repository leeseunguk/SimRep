use std::{fs, process::Command};

use simrep_core::{FieldType, PotentialArray, Symmetry};
use simrep_io::{PaFile, read_pa, write_pa};
use simrep_solve::magnetostatics::{FieldFile, Formulation};

fn run(dir: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_simrep"))
        .current_dir(dir)
        .env("RAYON_NUM_THREADS", "4")
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn scalar_material_refine_persists_solid_cells_and_flight_stops_there() {
    let dir = tempfile::tempdir().unwrap();
    let mut a = PotentialArray::new(9, 9, 1);
    a.field_type = FieldType::Magnetic;
    for i in 0..81 {
        a.electrodes[i] = i % 9 == 0 || i % 9 == 8;
        a.potentials[i] = if i % 9 == 8 { 1.0 } else { 0.0 };
    }
    write_pa(dir.path().join("p.pa"), &PaFile::new(a)).unwrap();
    let mut mu = PotentialArray::new(8, 8, 1);
    for (i, v) in mu.potentials.iter_mut().enumerate() {
        *v = if i % 8 < 4 { 1.0 } else { 5.0 };
    }
    write_pa(dir.path().join("mu.pa"), &PaFile::new(mu)).unwrap();
    fs::write(dir.path().join("m.json"), r#"{"permeability":"mu.pa"}"#).unwrap();
    let output = run(
        dir.path(),
        &[
            "refine",
            "p.pa",
            "--magnetic-material",
            "m.json",
            "--tolerance",
            "1e-12",
        ],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let header: FieldFile =
        serde_json::from_slice(&fs::read(dir.path().join("p.mag.json")).unwrap()).unwrap();
    assert_eq!(header.formulation, Formulation::Scalar);
    assert_eq!(header.solid_cells.iter().filter(|&&v| v).count(), 32);
    let p = read_pa(dir.path().join("p.pa")).unwrap();
    assert!((p.array.potentials[4] - 5.0 / 6.0).abs() < 1e-9);
    fs::write(dir.path().join("ion.ion"), "0,1,0,1,2,0,0,0,1,1\n").unwrap();
    let output = run(
        dir.path(),
        &[
            "fly",
            "--particles",
            "ion.ion",
            "--magnetic-pa",
            "p.pa",
            "--nonrelativistic",
            "--duration-us",
            "100",
        ],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let csv = String::from_utf8(output.stdout).unwrap();
    assert!(
        csv.lines().nth(1).unwrap().split(',').nth(6) == Some("electrode"),
        "{csv}"
    );
}

#[test]
fn current_psi_refine_loads_cpu_vector_field_and_rejects_explicit_gpu() {
    let dir = tempfile::tempdir().unwrap();
    let mut a = PotentialArray::new(9, 9, 1);
    a.field_type = FieldType::Magnetic;
    a.symmetry = Symmetry::Cylindrical;
    a.mirrors.y = true;
    for i in 0..81 {
        let r = (i / 9) as f64;
        a.electrodes[i] = i / 9 == 8;
        a.potentials[i] = if i / 9 == 8 { 0.5 * r * r } else { 0.0 };
    }
    let mut pa = PaFile::new(a);
    pa.ng = 1;
    write_pa(dir.path().join("psi.pa"), &pa).unwrap();
    let mut j = PotentialArray::new(8, 8, 1);
    j.symmetry = Symmetry::Cylindrical;
    j.mirrors.y = true;
    write_pa(dir.path().join("j.pa"), &PaFile::new(j)).unwrap();
    fs::write(dir.path().join("m.json"), r#"{"current_density":"j.pa"}"#).unwrap();
    let output = run(
        dir.path(),
        &[
            "refine",
            "psi.pa",
            "--magnetic-material",
            "m.json",
            "--tolerance",
            "1e-12",
        ],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result = read_pa(dir.path().join("psi.pa")).unwrap();
    for (i, v) in result.array.potentials.iter().enumerate() {
        assert!((v - 0.5 * (i / 9).pow(2) as f64).abs() < 1e-8);
    }
    fs::write(dir.path().join("ion.ion"), "0,100,1,4,1,0,0,0,1,1\n").unwrap();
    let output = run(
        dir.path(),
        &[
            "fly",
            "--particles",
            "ion.ion",
            "--magnetic-vector-potential",
            "psi.mag.json",
            "--duration-us",
            "0.001",
            "--allow-time-limit",
        ],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("device: cpu"));
    let output = run(
        dir.path(),
        &[
            "fly",
            "--particles",
            "ion.ion",
            "--magnetic-vector-potential",
            "psi.mag.json",
            "--device",
            "gpu",
        ],
    );
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("require --device cpu or auto"));
}

#[test]
fn cartesian_component_refine_and_header_load_end_to_end() {
    let dir = tempfile::tempdir().unwrap();
    for (name, component) in ["ax", "ay", "az"].iter().zip(0..3) {
        let mut a = PotentialArray::new(7, 7, 7);
        a.field_type = FieldType::Magnetic;
        for i in 0..343 {
            let c = [i % 7, i / 7 % 7, i / 49];
            a.electrodes[i] = c.iter().any(|&x| x == 0 || x == 6);
            if a.electrodes[i] {
                a.potentials[i] = match component {
                    0 => -0.5 * c[1] as f64,
                    1 => 0.5 * c[0] as f64,
                    _ => 0.0,
                };
            }
        }
        let mut pa = PaFile::new(a);
        pa.ng = 1;
        write_pa(dir.path().join(format!("{name}.pa")), &pa).unwrap();
        write_pa(
            dir.path().join(format!("j{name}.pa")),
            &PaFile::new(PotentialArray::new(6, 6, 6)),
        )
        .unwrap();
    }
    fs::write(dir.path().join("m.json"),r#"{"cartesian":{"potentials":["ax.pa","ay.pa","az.pa"],"current_density":["jax.pa","jay.pa","jaz.pa"]}}"#).unwrap();
    let output = run(
        dir.path(),
        &["refine", "ax.pa", "--magnetic-material", "m.json"],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    fs::write(dir.path().join("ion.ion"), "0,100,1,3,3,3,0,0,1,1\n").unwrap();
    let output = run(
        dir.path(),
        &[
            "fly",
            "--particles",
            "ion.ion",
            "--magnetic-vector-potential",
            "ax.mag.json",
            "--duration-us",
            "0.001",
            "--allow-time-limit",
        ],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

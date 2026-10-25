use std::process::Command;

use simrep_core::{PotentialArray, Symmetry};
use simrep_io::{PaFile, read_pa, write_pa};

#[test]
fn refine_accepts_simion_cell_arrays_in_planar_and_cylindrical_layouts() {
    for symmetry in [Symmetry::Planar, Symmetry::Cylindrical] {
        let dir = tempfile::tempdir().unwrap();
        let mut potential = PotentialArray::new(17, 9, 1);
        potential.symmetry = symmetry;
        potential.mirrors.y = symmetry == Symmetry::Cylindrical;
        potential.dx_mm = 1.0 / 16.0;
        for i in 0..potential.potentials.len() {
            potential.electrodes[i] = i % 17 == 0 || i % 17 == 16;
            potential.potentials[i] = if i % 17 == 16 { 1.0 } else { 0.0 };
        }
        let mut epsilon = PotentialArray::new(16, 8, 1);
        epsilon.symmetry = symmetry;
        epsilon.mirrors.y = symmetry == Symmetry::Cylindrical;
        epsilon.dx_mm = potential.dx_mm;
        for (i, e) in epsilon.potentials.iter_mut().enumerate() {
            *e = if i % 16 < 8 { 1.0 } else { 3.0 };
        }
        let input = dir.path().join("plates.pa");
        let material = dir.path().join("epsilon.pa");
        write_pa(&input, &PaFile::new(potential.clone())).unwrap();
        write_pa(&material, &PaFile::new(epsilon)).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_simrep"))
            .args([
                "refine",
                input.to_str().unwrap(),
                "--permittivity",
                material.to_str().unwrap(),
                "--tolerance",
                "1e-12",
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let result = read_pa(&input).unwrap().array;
        for (i, &v) in result.potentials.iter().enumerate() {
            let x = (i % 17) as f64 / 16.0;
            let exact = (x.min(0.5) + (x - 0.5).max(0.0) / 3.0) / (2.0 / 3.0);
            assert!((v - exact).abs() < 1e-9, "{symmetry:?} {i}: {v} != {exact}");
        }
        assert_eq!(result.electrodes, potential.electrodes);
    }
}

#[test]
fn refine_rejects_invalid_material_pa_without_overwriting_input() {
    let dir = tempfile::tempdir().unwrap();
    let mut potential = PotentialArray::new(5, 5, 1);
    potential.electrodes[0] = true;
    let input = dir.path().join("potential.pa");
    let material = dir.path().join("bad.pa");
    let original = PaFile::new(potential);
    write_pa(&input, &original).unwrap();
    write_pa(&material, &original).unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_simrep"))
        .args([
            "refine",
            input.to_str().unwrap(),
            "--permittivity",
            material.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert_eq!(read_pa(&input).unwrap(), original);
}

#[test]
fn refine_json_sdf_materials_match_the_library_operator() {
    let dir = tempfile::tempdir().unwrap();
    let mut potential = PotentialArray::new(17, 9, 1);
    for i in 0..potential.potentials.len() {
        potential.electrodes[i] = i % 17 == 0 || i % 17 == 16;
        potential.potentials[i] = if i % 17 == 16 { 1.0 } else { 0.0 };
    }
    let input = dir.path().join("plates.pa");
    let material = dir.path().join("materials.json");
    // A plane is sampled at nodes: the interface is midway between nodes
    // 7 and 8, so the series resistance is 7.5 + 8.5/3 grid units.
    std::fs::write(&material, r#"{"background":1,"regions":[{"epsilon":3,"shape":{"kind":"plane","normal":[-1,0,0],"offset":-8}}]}"#).unwrap();
    write_pa(&input, &PaFile::new(potential)).unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_simrep"))
        .args([
            "refine",
            input.to_str().unwrap(),
            "--permittivity",
            material.to_str().unwrap(),
            "--tolerance",
            "1e-12",
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let values = read_pa(input).unwrap().array.potentials;
    for (i, &v) in values.iter().enumerate() {
        let x = (i % 17) as f64;
        let exact = (x.min(7.5) + (x - 7.5).max(0.0) / 3.0) / (7.5 + 8.5 / 3.0);
        assert!((v - exact).abs() < 1e-9);
    }
}

#[test]
fn nonlinear_json_material_solves_actual_voltages_and_rejects_fast_adjust() {
    let dir = tempfile::tempdir().unwrap();
    let mut potential = PotentialArray::new(17, 5, 1);
    potential.dx_mm = 1.0 / 16.0;
    for i in 0..potential.potentials.len() {
        potential.electrodes[i] = i % 17 == 0 || i % 17 == 16;
        potential.potentials[i] = if i % 17 == 16 { 1.0 } else { 0.0 };
    }
    let mut epsilon = PotentialArray::new(16, 4, 1);
    epsilon.dx_mm = potential.dx_mm;
    epsilon.potentials.fill(3.0);
    let input = dir.path().join("nonlinear.pa");
    let cells = dir.path().join("cells.pa");
    let spec = dir.path().join("nonlinear.json");
    write_pa(&input, &PaFile::new(potential.clone())).unwrap();
    write_pa(&cells, &PaFile::new(epsilon)).unwrap();
    std::fs::write(&spec,r#"{"nonlinear":{"cell_permittivity":"cells.pa","laws":[{"epsilon0":3,"law":{"kind":"saturating_quadratic","lambda":0.02,"minimum":1}}]}}"#).unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_simrep"))
        .args([
            "refine",
            input.to_str().unwrap(),
            "--permittivity",
            spec.to_str().unwrap(),
            "--device",
            "cpu",
            "--tolerance",
            "1e-11",
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let values = read_pa(&input).unwrap().array.potentials;
    for (i, v) in values.iter().enumerate() {
        assert!((v - (i % 17) as f64 / 16.0).abs() < 1e-8);
    }
    let sharp = input.with_extension("pa#");
    write_pa(&sharp, &PaFile::new(potential)).unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_simrep"))
        .args([
            "refine",
            sharp.to_str().unwrap(),
            "--permittivity",
            spec.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(!sharp.with_extension("pa0").exists());
}

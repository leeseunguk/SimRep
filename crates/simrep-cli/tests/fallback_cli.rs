use std::process::{Command, Output};

use simrep_core::PotentialArray;
use simrep_io::{PaFile, write_pa};

fn command(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_simrep"))
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn automatic_flight_reports_small_beam_and_magnetic_decisions() {
    let dir = tempfile::tempdir().unwrap();
    let ions = dir.path().join("one.ion");
    std::fs::write(&ions, "0,100,1,0,0,0,0,0,1,1\n").unwrap();
    for (extra, reason) in [
        (vec![], "1 ions < measured crossover 4096"),
        (
            vec!["--bfield=0,0,10"],
            "magnetic field: GPU magnetic flight not selected automatically",
        ),
    ] {
        let mut args = vec![
            "fly",
            "--particles",
            ions.to_str().unwrap(),
            "--duration-us=0.001",
            "--allow-time-limit",
        ];
        args.extend(extra);
        let result = command(&args);
        let stderr = String::from_utf8_lossy(&result.stderr);
        assert!(result.status.success(), "{stderr}");
        assert!(stderr.contains("device: cpu, requested: Auto"), "{stderr}");
        assert!(
            stderr.contains(&format!("auto: CPU ({reason})")),
            "{stderr}"
        );
    }
}

#[test]
fn automatic_refine_reports_vacuum_and_dielectric_policy() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("zero.pa");
    let mut array = PotentialArray::new(5, 5, 1);
    array.electrodes[0] = true;
    write_pa(&input, &PaFile::new(array)).unwrap();
    let material = dir.path().join("epsilon.json");
    std::fs::write(&material, r#"{"background":1,"regions":[]}"#).unwrap();
    for (extra, reason) in [
        (
            vec![],
            "no dielectric material; use --device gpu for GPU refine",
        ),
        (
            vec!["--permittivity", material.to_str().unwrap()],
            "dielectric grid below measured GPU crossover",
        ),
    ] {
        let mut args = vec!["refine", input.to_str().unwrap()];
        args.extend(extra);
        let result = command(&args);
        let stdout = String::from_utf8_lossy(&result.stdout);
        assert!(result.status.success(), "{:?}", result);
        assert!(
            stdout.contains(&format!("auto: CPU ({reason})")),
            "{stdout}"
        );
    }
}

#[test]
fn invalid_choices_list_accepted_values() {
    for (args, accepted) in [
        (
            vec!["fly", "--particles=x.ion", "--device=typo"],
            vec!["auto", "cpu", "gpu"],
        ),
        (
            vec!["fly", "--particles=x.ion", "--bfield-map-format=typo"],
            vec!["auto", "comsol", "opera", "csv", "txt", "axisymmetric"],
        ),
        (
            vec!["fly", "--particles=x.ion", "--bfield-map-units=typo/T"],
            vec!["mm", "cm", "inch"],
        ),
        (
            vec!["refine-geometry", "x.gem", "--cad-mode=typo", "--out=x"],
            vec!["auto", "exact", "tessellated"],
        ),
    ] {
        let result = command(&args);
        assert!(!result.status.success());
        let stderr = String::from_utf8_lossy(&result.stderr);
        for choice in accepted {
            assert!(stderr.contains(choice), "{stderr}");
        }
    }
}

#[test]
fn explicit_gpu_rejects_cpu_only_physics_before_adapter_selection() {
    for (extra, reason) in [
        (vec!["--space-charge=direct"], "direct Coulomb physics"),
        (
            vec!["--charge-mode=beam", "--beam-current-a=1e-9"],
            "leader-plane beam integration",
        ),
    ] {
        let mut args = vec!["fly", "--particles=unused.ion", "--device=gpu"];
        args.extend(extra);
        let result = command(&args);
        let stderr = String::from_utf8_lossy(&result.stderr);
        assert!(!result.status.success());
        assert!(stderr.contains(reason), "{stderr}");
        assert!(stderr.contains("--device cpu or auto"), "{stderr}");
    }
}

#[test]
fn refine_cpu_recovery_is_accepted_and_software_gpu_hints_are_valid() {
    let dir = tempfile::tempdir().unwrap();
    let gem = dir.path().join("plates.gem");
    std::fs::write(
        &gem,
        "pa_define(5,5,1,planar)\ne(1){fill{within{box(-1,-1,1,5)}}}\n\
         e(2){fill{within{box(3,-1,5,5)}}}",
    )
    .unwrap();
    let out = dir.path().join("plates");
    let probe = command(&["gpu-info", "--backend=vulkan"]);
    let software_gpu = String::from_utf8_lossy(&probe.stdout).contains("Device type: Cpu");
    for mut args in [
        vec!["refine-bench", "--n=5"],
        vec!["refine-sdf", "--n=9"],
        vec![
            "refine-geometry",
            gem.to_str().unwrap(),
            "--electrode=1=1:0",
            "--electrode=2=2:1",
            "--out",
            out.to_str().unwrap(),
        ],
    ] {
        if software_gpu {
            let mut gpu_args = args.clone();
            gpu_args.extend(["--device=gpu", "--backend=vulkan"]);
            let result = command(&gpu_args);
            let stderr = String::from_utf8_lossy(&result.stderr);
            assert!(!result.status.success(), "{result:?}");
            assert!(stderr.contains("requires a hardware GPU"), "{stderr}");
            assert!(stderr.contains("use --device cpu"), "{stderr}");
            assert!(!stderr.contains("or auto"), "{stderr}");
        }
        args.push("--device=cpu");
        let result = command(&args);
        assert!(result.status.success(), "{result:?}");
    }
}

#[test]
fn unsupported_cad_settings_and_formats_list_supported_inputs() {
    for (extra, accepted) in [
        (
            vec!["--cad-mode=exact"],
            vec![".step", ".stp", ".iges", ".igs"],
        ),
        (
            vec!["--cad-tolerance=0.01"],
            vec![".step", ".stp", ".iges", ".igs"],
        ),
        (
            vec![],
            vec![
                ".gem", ".step", ".stl", ".obj", ".ply", ".3mf", ".gltf", ".glb",
            ],
        ),
    ] {
        let mut args = vec![
            "refine-geometry",
            "input.unknown",
            "--electrode=1=1:0",
            "--out=unused",
        ];
        args.extend(extra);
        let result = command(&args);
        let stderr = String::from_utf8_lossy(&result.stderr);
        assert!(!result.status.success());
        for extension in accepted {
            assert!(stderr.contains(extension), "{stderr}");
        }
    }
}

#[test]
fn duplicate_flight_keys_are_rejected_before_loading_inputs() {
    for (values, option) in [
        (vec!["--param=f=1", "--param=f=2"], "--param"),
        (vec!["--voltage=2=1", "--voltage=2=2"], "--voltage"),
        (
            vec!["--voltage-expr=2=t", "--voltage-expr=2=2*t"],
            "--voltage-expr",
        ),
    ] {
        let mut args = vec!["fly", "--particles=unused.ion"];
        args.extend(values);
        let result = command(&args);
        let stderr = String::from_utf8_lossy(&result.stderr);
        assert!(!result.status.success());
        assert!(stderr.contains(&format!("duplicate {option}")), "{stderr}");
        assert!(stderr.contains("once"), "{stderr}");
    }
}

#[test]
fn uppercase_opera_table_extensions_keep_automatic_format_detection() {
    let dir = tempfile::tempdir().unwrap();
    for extension in ["table", "TABLE", "TaBlE"] {
        let path = dir.path().join(format!("field.{extension}"));
        std::fs::write(
            &path,
            "1 1 1\n1 X [CM]\n2 Y [CM]\n3 Z [CM]\n4 BX [MT]\n5 BY [MT]\n6 BZ [MT]\n0\n0 0 0 1 2 3\n",
        )
        .unwrap();
        let grid = simrep_io::bfield::read(&path, &Default::default()).unwrap();
        assert_eq!(grid.dims, [1, 1, 1]);
        assert_eq!(grid.values_gauss, [[10.0, 20.0, 30.0]]);
    }
}

#[test]
fn empty_magnetic_material_is_rejected_without_overwriting_input() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("magnetic.pa");
    let mut array = PotentialArray::new(5, 5, 1);
    array.field_type = simrep_core::FieldType::Magnetic;
    let mut original = PaFile::new(array);
    original.ng = 1;
    write_pa(&input, &original).unwrap();
    let spec = dir.path().join("material.json");
    std::fs::write(&spec, "{}").unwrap();
    let result = command(&[
        "refine",
        input.to_str().unwrap(),
        "--magnetic-material",
        spec.to_str().unwrap(),
    ]);
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(!result.status.success());
    assert!(
        stderr.contains("empty magnetic-material specification"),
        "{stderr}"
    );
    for accepted in [
        "permeability",
        "magnetization",
        "negative_divergence",
        "current_density",
        "solid",
    ] {
        assert!(stderr.contains(accepted), "{stderr}");
    }
    assert_eq!(simrep_io::read_pa(&input).unwrap(), original);
    assert!(!input.with_extension("mag.json").exists());
}

#[test]
fn invalid_voltage_and_map_units_explain_accepted_syntax_and_aliases() {
    for (option, value, accepted) in [
        (
            "--voltage",
            "bad=1",
            vec!["electrode=volts", "nonnegative integer"],
        ),
        (
            "--voltage",
            "1=bad",
            vec!["electrode=volts", "finite voltage"],
        ),
        (
            "--bfield-map-units",
            "bad/T",
            vec!["mm", "m", "cm", "inch", "in"],
        ),
        (
            "--bfield-map-units",
            "mm/bad",
            vec!["G", "gauss", "T", "tesla", "mT"],
        ),
    ] {
        let result = command(&["fly", "--particles=unused.ion", option, value]);
        let stderr = String::from_utf8_lossy(&result.stderr);
        assert!(!result.status.success());
        for accepted in accepted {
            assert!(stderr.contains(accepted), "{stderr}");
        }
    }
}

#[test]
fn invalid_material_json_lists_accepted_fields_and_shapes() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("zero.pa");
    write_pa(&input, &PaFile::new(PotentialArray::new(5, 5, 1))).unwrap();
    let material = dir.path().join("epsilon.json");
    for (json, accepted) in [
        (
            r#"{"background":1,"regions":[],"typo":true}"#,
            vec!["background", "origin_mm", "regions"],
        ),
        (
            r#"{"background":1,"regions":[{"epsilon":2,"shape":{"kind":"typo"}}]}"#,
            vec![
                "sphere",
                "cylinder",
                "box",
                "plane",
                "union",
                "intersection",
                "difference",
            ],
        ),
    ] {
        std::fs::write(&material, json).unwrap();
        let result = command(&[
            "refine",
            input.to_str().unwrap(),
            "--permittivity",
            material.to_str().unwrap(),
        ]);
        assert!(!result.status.success());
        let stderr = String::from_utf8_lossy(&result.stderr);
        for choice in accepted {
            assert!(stderr.contains(choice), "{stderr}");
        }
    }
}

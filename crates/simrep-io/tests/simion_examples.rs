use simrep_core::{FieldType, Symmetry};
use simrep_io::{PaFile, read_pa};
use std::path::PathBuf;

#[test]
#[ignore = "requires SIMION examples (SIMION_ROOT)"]
fn installed_simion_examples_read_and_round_trip_without_copying_fixtures() {
    let root = std::env::var_os("SIMION_ROOT").expect("requires SIMION_ROOT");
    let examples = PathBuf::from(root).join("examples");
    assert!(
        examples.is_dir(),
        "requires SIMION examples at {}",
        examples.display()
    );
    let files = [
        "einzel/einzel.pa#",
        "einzel/einzel.pa0",
        "tof/source.pa0",
        "misc/test3d.pa",
        "field_array/mag.pa",
        "performance_tests/2_linear_acceleration/non-relativistic.pa#",
        "field_array/flow_vx.pa",
    ];
    for name in files {
        let path = examples.join(name);
        let original = std::fs::read(&path).unwrap();
        let file = read_pa(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(
            file.array.potentials.len(),
            file.array.nx * file.array.ny * file.array.nz
        );
        assert!(file.array.potentials.iter().all(|v| v.is_finite()));
        if name.starts_with("einzel/") {
            assert_eq!(file.array.symmetry, Symmetry::Cylindrical);
            assert_eq!(file.array.nz, 1);
            assert!(file.array.electrodes.iter().any(|&e| e));
        }
        if name.ends_with(".pa0") {
            assert!(
                file.pa0.is_some(),
                "{name} must contain fast-adjust metadata"
            );
        }
        if name == "misc/test3d.pa" {
            assert_eq!(file.array.symmetry, Symmetry::Planar);
            assert!(file.array.nz > 1);
        }
        if name == "field_array/mag.pa" {
            assert_eq!(file.array.field_type, FieldType::Magnetic);
        }
        if name == "field_array/flow_vx.pa" {
            assert!(!file.array.refinable);
        }
        let mut encoded = Vec::new();
        file.write(&mut encoded)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(PaFile::read(encoded.as_slice()).unwrap(), file, "{name}");
        assert!(
            encoded == original,
            "{name}: binary round trip differs at byte {:?} (lengths {} and {})",
            encoded.iter().zip(&original).position(|(a, b)| a != b),
            encoded.len(),
            original.len()
        );
        eprintln!(
            "{name}: mode {}, {} x {} x {}, {:?}, {:?}, {} electrode points",
            file.mode.value(),
            file.array.nx,
            file.array.ny,
            file.array.nz,
            file.array.symmetry,
            file.array.field_type,
            file.array.electrodes.iter().filter(|&&e| e).count()
        );
    }
}

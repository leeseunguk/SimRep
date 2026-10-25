use simrep_core::{FieldType, MirrorFlags, PotentialArray, Symmetry};
use simrep_io::{FastScalable, Pa0Metadata, PaFile, PaMode, read_pa, write_pa};
use std::io::ErrorKind;

fn sample(nz: usize) -> PaFile {
    let mut array = PotentialArray::new(3, 4, nz);
    for (i, (potential, electrode)) in array
        .potentials
        .iter_mut()
        .zip(&mut array.electrodes)
        .enumerate()
    {
        *potential = i as f64 * 0.25 - 2.0;
        *electrode = i % 3 == 0;
    }
    PaFile::new(array)
}

fn encode(file: &PaFile) -> Vec<u8> {
    let mut bytes = Vec::new();
    file.write(&mut bytes).unwrap();
    bytes
}

#[test]
fn round_trip_header_modes_symmetries_fields_and_mirrors() {
    for mode in [PaMode::Standard, PaMode::Extended] {
        for symmetry in [Symmetry::Planar, Symmetry::Cylindrical] {
            for field in [FieldType::Electrostatic, FieldType::Magnetic] {
                for flags in 0..8 {
                    let mut file = sample(if symmetry == Symmetry::Cylindrical {
                        1
                    } else {
                        2
                    });
                    file.mode = mode;
                    file.array.symmetry = symmetry;
                    file.array.field_type = field;
                    file.array.refinable = flags & 2 == 0;
                    file.array.mirrors = MirrorFlags {
                        x: flags & 1 != 0,
                        y: symmetry == Symmetry::Cylindrical || flags & 2 != 0,
                        z: symmetry == Symmetry::Planar && flags & 4 != 0,
                    };
                    file.array.anti_mirrors = MirrorFlags {
                        x: flags & 1 != 0,
                        y: flags & 2 != 0,
                        z: symmetry == Symmetry::Planar && flags & 4 != 0,
                    };
                    file.ng = 90_000;
                    if mode == PaMode::Extended {
                        file.array.dx_mm = 0.125;
                        file.array.dy_mm = 0.25;
                        file.array.dz_mm = 0.5;
                    }
                    let bytes = encode(&file);
                    assert_eq!(
                        bytes.len(),
                        mode.header_bytes() + 8 * file.array.potentials.len()
                    );
                    assert_eq!(PaFile::read(bytes.as_slice()).unwrap(), file);
                }
            }
        }
    }
}

#[test]
fn non_unit_spacing_upgrades_standard_header_without_losing_spacing() {
    let mut file = sample(1);
    file.array.dy_mm = 2.5;
    let decoded = PaFile::read(encode(&file).as_slice()).unwrap();
    assert_eq!(decoded.mode, PaMode::Extended);
    assert_eq!(decoded.array, file.array);

    file.array.dy_mm = 1.0;
    file.mode = PaMode::Extended;
    assert_eq!(PaFile::read(encode(&file).as_slice()).unwrap(), file);
}

#[test]
fn matches_reference_byte_layout_and_strict_electrode_threshold() {
    // Independently construct the standard header using the reference layout:
    // i32 mode, i32 symmetry, f64 max_voltage, i32 nx/ny/nz, i32 mirror.
    let mut bytes = Vec::new();
    bytes.extend((-1_i32).to_le_bytes());
    bytes.extend(1_i32.to_le_bytes());
    bytes.extend(100_000_f64.to_le_bytes());
    for value in [3_i32, 3, 1, (100 << 4) | 2 | 8 | (1 << 28) | (1 << 29)] {
        bytes.extend(value.to_le_bytes());
    }
    let raw: [f64; 9] = [
        100_000.0, 100_000.25, 199_990.0, -100_000.0, -0.0, 0.0, 1.0, 2.0, 3.0,
    ];
    for value in raw {
        bytes.extend(value.to_le_bytes());
    }
    let file = PaFile::read(bytes.as_slice()).unwrap();
    assert_eq!(
        file.array.potentials[..4],
        [100_000.0, -99_999.75, -10.0, -100_000.0]
    );
    assert_eq!(file.array.electrodes[..4], [false, true, true, false]);
    assert_eq!(file.array.field_type, FieldType::Magnetic);
    assert!(file.array.mirrors.y);
    assert!(file.array.anti_mirrors.x);
    assert!(!file.array.refinable);
    assert_eq!(encode(&file), bytes);
}

#[test]
fn pa0_round_trip_old_sparse_and_scalable_trailers() {
    for entries in [32, 128, 1001] {
        for flag in 0..3 {
            let mut file = sample(1);
            let mut indices = vec![-1; entries];
            indices[1] = 0;
            indices[entries - 1] = 3;
            let scalable = match flag {
                0 => None,
                1 => Some(FastScalable::Disabled),
                _ => Some(FastScalable::Voltages(vec![0.25; entries])),
            };
            file.pa0 = Some(Pa0Metadata {
                reference_voltage: 10_000.0,
                electrode_indices: indices,
                scalable,
            });
            let bytes = encode(&file);
            assert_eq!(PaFile::read(bytes.as_slice()).unwrap(), file);
            let trailer = file.mode.header_bytes() + file.array.potentials.len() * 8;
            assert_eq!(&bytes[trailer..trailer + 4], &2_i32.to_le_bytes());
        }
    }
}

#[test]
fn file_round_trip_for_pa_pasharp_and_pa0_suffixes() {
    for suffix in ["pa", "pa#", "pa0"] {
        let path = std::env::temp_dir().join(format!("simrep-pa-{}.{suffix}", std::process::id()));
        let mut file = sample(1);
        if suffix == "pa0" {
            let mut electrode_indices = vec![-1; 32];
            electrode_indices[1] = 0;
            file.pa0 = Some(Pa0Metadata {
                reference_voltage: 10_000.0,
                electrode_indices,
                scalable: None,
            });
        }
        write_pa(&path, &file).unwrap();
        let decoded = read_pa(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(decoded, file);
    }
}

#[test]
fn rejects_bad_modes_truncated_headers_and_points() {
    let file = sample(2);
    let bytes = encode(&file);
    for end in 0..bytes.len() {
        assert_eq!(
            PaFile::read(&bytes[..end]).unwrap_err().kind(),
            ErrorKind::UnexpectedEof
        );
    }
    for mode in [0_i32, 1, -3, i32::MIN] {
        let mut bad = bytes.clone();
        bad[..4].copy_from_slice(&mode.to_le_bytes());
        assert_eq!(
            PaFile::read(bad.as_slice()).unwrap_err().kind(),
            ErrorKind::InvalidData
        );
    }
    let mut extended = file;
    extended.mode = PaMode::Extended;
    let bytes = encode(&extended);
    for end in 32..56 {
        assert_eq!(
            PaFile::read(&bytes[..end]).unwrap_err().kind(),
            ErrorKind::UnexpectedEof
        );
    }
}

#[test]
fn rejects_invalid_header_and_unencodable_data_before_writing() {
    let good = sample(2);
    let mut cases = Vec::new();
    let mut bad = good.clone();
    bad.array.nx = 0;
    cases.push(bad);
    let mut bad = good.clone();
    bad.array.potentials.pop();
    cases.push(bad);
    let mut bad = good.clone();
    bad.array.electrodes.clear();
    cases.push(bad);
    let mut bad = good.clone();
    bad.array.symmetry = Symmetry::Cylindrical;
    cases.push(bad);
    let mut bad = good.clone();
    bad.array.dx_mm = f64::NAN;
    cases.push(bad);
    let mut bad = good.clone();
    bad.max_voltage = 0.0;
    cases.push(bad);
    let mut bad = good.clone();
    bad.ng = 90_001;
    cases.push(bad);
    let mut bad = good.clone();
    bad.array.potentials[0] = -bad.max_voltage; // Offset would erase electrode membership.
    cases.push(bad);
    let mut bad = good.clone();
    bad.array.potentials[1] = bad.max_voltage + 1.0; // Would look like an electrode.
    cases.push(bad);
    let mut bad = good;
    bad.array.potentials[1] = f64::INFINITY;
    cases.push(bad);
    for bad in cases {
        let mut output = Vec::new();
        assert!(bad.write(&mut output).is_err());
        assert!(output.is_empty());
    }
}

#[test]
fn rejects_malformed_pa0_metadata() {
    let mut file = sample(1);
    let mut indices = vec![-1; 32];
    indices[1] = 0;
    file.pa0 = Some(Pa0Metadata {
        reference_voltage: 10_000.0,
        electrode_indices: indices,
        scalable: None,
    });
    let bytes = encode(&file);
    assert_eq!(
        PaFile::read(&bytes[..bytes.len() - 1]).unwrap_err().kind(),
        ErrorKind::UnexpectedEof
    );
    let mut bad = bytes.clone();
    let trailer = 32 + file.array.potentials.len() * 8;
    bad[trailer..trailer + 4].copy_from_slice(&0_i32.to_le_bytes());
    assert_eq!(
        PaFile::read(bad.as_slice()).unwrap_err().kind(),
        ErrorKind::InvalidData
    );
    let mut bad = bytes.clone();
    let first_offset = trailer + 12 + 4;
    bad[first_offset..first_offset + 4].copy_from_slice(&12_i32.to_le_bytes());
    assert!(PaFile::read(bad.as_slice()).is_err());
    let mut bad = bytes;
    bad.extend(2_i32.to_le_bytes());
    assert_eq!(
        PaFile::read(bad.as_slice()).unwrap_err().kind(),
        ErrorKind::InvalidData
    );
}

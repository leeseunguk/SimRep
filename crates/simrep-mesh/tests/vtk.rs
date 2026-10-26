use simrep_core::{PotentialArray, Symmetry};
use simrep_mesh::{
    ImportOptions, SurfaceOptions, Trajectory, TrajectoryPoint, electrode_surface, inspect,
    pa_isosurface, read_mesh, write_mesh, write_revolved_vts, write_trajectories_csv, write_vti,
    write_vtp,
};
use std::collections::HashMap;
use std::fs;
use tempfile::tempdir;

fn arrays(bytes: &[u8]) -> HashMap<String, Vec<u8>> {
    let marker = b"<AppendedData encoding=\"raw\">_";
    let offset = bytes
        .windows(marker.len())
        .position(|v| v == marker)
        .unwrap()
        + marker.len();
    let header = std::str::from_utf8(&bytes[..offset - marker.len()]).unwrap();
    let mut reader = quick_xml::Reader::from_str(header);
    let mut result = HashMap::new();
    loop {
        match reader.read_event().unwrap() {
            quick_xml::events::Event::Empty(e) if e.local_name().as_ref() == "DataArray" => {
                let attrs: HashMap<_, _> = e
                    .attributes()
                    .map(|a| {
                        let a = a.unwrap();
                        (
                            a.key.as_ref().to_owned(),
                            a.normalized_value(quick_xml::XmlVersion::Implicit1_0)
                                .unwrap()
                                .into_owned(),
                        )
                    })
                    .collect();
                let start = offset + attrs["offset"].parse::<usize>().unwrap();
                let size = u64::from_le_bytes(bytes[start..start + 8].try_into().unwrap()) as usize;
                result.insert(
                    attrs["Name"].clone(),
                    bytes[start + 8..start + 8 + size].to_vec(),
                );
            }
            quick_xml::events::Event::Eof => break,
            _ => {}
        }
    }
    result
}

fn floats(bytes: &[u8]) -> Vec<f64> {
    bytes
        .as_chunks::<8>()
        .0
        .iter()
        .map(|&b| f64::from_le_bytes(b))
        .collect()
}

fn integers(bytes: &[u8]) -> Vec<u64> {
    bytes
        .as_chunks::<8>()
        .0
        .iter()
        .map(|&b| u64::from_le_bytes(b))
        .collect()
}

#[test]
fn vti_payload_order_lengths_and_anisotropic_linear_field() {
    let dir = tempdir().unwrap();
    let mut pa = PotentialArray::new(3, 2, 2);
    [pa.dx_mm, pa.dy_mm, pa.dz_mm] = [0.5, 2.0, 3.0];
    for z in 0..2 {
        for y in 0..2 {
            for x in 0..3 {
                let i = pa.index(x, y, z).unwrap();
                pa.potentials[i] = 5.0 * x as f64 * pa.dx_mm - 2.0 * y as f64 * pa.dy_mm
                    + 3.0 * z as f64 * pa.dz_mm;
                pa.electrodes[i] = x == 0;
            }
        }
    }
    let path = dir.path().join("grid.vti");
    write_vti(&path, &pa, [1.0, -2.0, 3.0], true).unwrap();
    let payload = arrays(&fs::read(path).unwrap());
    assert_eq!(payload.len(), 3);
    assert_eq!(floats(&payload["potential"]), pa.potentials);
    assert_eq!(
        payload["electrode"],
        pa.electrodes
            .iter()
            .copied()
            .map(u8::from)
            .collect::<Vec<_>>()
    );
    let field = floats(&payload["field"]);
    assert_eq!(field.len(), pa.potentials.len() * 3);
    for &value in field.as_chunks::<3>().0 {
        assert_eq!(value, [-5.0, 2.0, -3.0]);
    }
}

#[test]
fn planar_2d_vti_expands_even_and_odd_mirrors_without_duplicate_plane_nodes() {
    let dir = tempdir().unwrap();
    let mut pa = PotentialArray::new(3, 3, 1);
    pa.mirrors.x = true;
    pa.mirrors.y = true;
    pa.anti_mirrors.x = true;
    pa.dx_mm = 0.5;
    pa.dy_mm = 2.0;
    for y in 0..3 {
        for x in 0..3 {
            let i = pa.index(x, y, 0).unwrap();
            pa.potentials[i] = 3.0 * x as f64 * pa.dx_mm;
            pa.electrodes[i] = y == 1;
        }
    }
    let path = dir.path().join("planar.vti");
    write_vti(&path, &pa, [2.0, 3.0, 4.0], true).unwrap();
    let bytes = fs::read(path).unwrap();
    assert!(
        bytes
            .windows(b"WholeExtent=\"0 4 0 4 0 0\"".len())
            .any(|b| b == b"WholeExtent=\"0 4 0 4 0 0\"")
    );
    let payload = arrays(&bytes);
    let potential = floats(&payload["potential"]);
    assert_eq!(potential.len(), 25);
    for &row in potential.as_chunks::<5>().0 {
        assert_eq!(row, [-3.0, -1.5, 0.0, 1.5, 3.0]);
    }
    for &f in floats(&payload["field"]).as_chunks::<3>().0 {
        assert_eq!(f, [-3.0, 0.0, 0.0]);
    }
    assert_eq!(payload["electrode"].iter().filter(|&&e| e == 1).count(), 10);
}

#[test]
fn cylindrical_vti_and_vts_decode_coordinates_potential_mask_and_rotated_field() {
    let dir = tempdir().unwrap();
    let mut pa = PotentialArray::new(3, 3, 1);
    pa.symmetry = Symmetry::Cylindrical;
    pa.mirrors.x = true;
    pa.mirrors.y = true;
    pa.dx_mm = 0.5;
    pa.dy_mm = 2.0;
    for y in 0..3 {
        for x in 0..3 {
            let i = pa.index(x, y, 0).unwrap();
            pa.potentials[i] = (x as f64 * pa.dx_mm).powi(2) + (y as f64 * pa.dy_mm).powi(2);
            pa.electrodes[i] = y == 2;
        }
    }
    let vti = dir.path().join("meridional.vti");
    write_vti(&vti, &pa, [3.0, 4.0, 5.0], true).unwrap();
    let plane = arrays(&fs::read(vti).unwrap());
    let plane_potential = floats(&plane["potential"]);
    assert_eq!(plane_potential.len(), 15);
    assert_eq!(&plane_potential[..5], [1.0, 0.25, 0.0, 0.25, 1.0]);
    let plane_field = floats(&plane["field"]);
    assert_eq!(&plane_field[3 * (5 + 2)..3 * (5 + 2) + 3], [0.0, -4.0, 0.0]);
    assert_eq!(plane_field[1], 0.0); // even radial axis

    for with_field in [true, false] {
        let vts = dir.path().join("revolved.vts");
        write_revolved_vts(&vts, &pa, [3.0, 4.0, 5.0], with_field, 4, 360.0).unwrap();
        let payload = arrays(&fs::read(vts).unwrap());
        assert_eq!(payload.contains_key("field"), with_field);
        let potential = floats(&payload["potential"]);
        let points = floats(&payload["position_mm"]);
        assert_eq!(potential.len(), 75);
        for step in 0..5 {
            assert_eq!(&potential[step * 15..(step + 1) * 15], plane_potential);
            assert_eq!(
                &payload["electrode"][step * 15..(step + 1) * 15],
                plane["electrode"]
            );
        }
        assert_eq!(&points[..45], &points[180..225]); // exact seam
        let sample = (15 + 5 + 2) * 3; // theta=90, x=0, r=2
        assert!((points[sample] - 3.0).abs() < 1e-12);
        assert!((points[sample + 1] - 4.0).abs() < 1e-12);
        assert!((points[sample + 2] - 7.0).abs() < 1e-12);
        if with_field {
            let field = floats(&payload["field"]);
            assert!((field[sample + 1]).abs() < 1e-12);
            assert_eq!(field[sample + 2], -4.0);
        }
    }
    let partial = dir.path().join("partial.vts");
    write_revolved_vts(&partial, &pa, [0.0; 3], false, 3, 270.0).unwrap();
    let payload = arrays(&fs::read(partial).unwrap());
    let points = floats(&payload["position_mm"]);
    let last = (3 * 15 + 5 + 2) * 3;
    assert!(points[last + 1].abs() < 1e-12);
    assert_eq!(points[last + 2], -2.0);
}

#[test]
fn installed_einzel_pa0_exports_geometry_grid_and_isopotentials_read_only() {
    let input = std::path::Path::new("/mnt/c/Program Files/SIMION-2024/examples/einzel/einzel.pa0");
    if !input.exists() {
        return; // optional installed interoperability fixture
    }
    let pa = simrep_io::read_pa(input).unwrap().array;
    assert_eq!(pa.symmetry, Symmetry::Cylindrical);
    let geometry = electrode_surface(&pa, [0.0; 3], None).unwrap();
    for part in &geometry.parts {
        assert_eq!(inspect(part).unwrap(), Default::default());
    }
    let lo = pa.potentials.iter().copied().fold(f64::INFINITY, f64::min);
    let hi = pa
        .potentials
        .iter()
        .copied()
        .fold(f64::NEG_INFINITY, f64::max);
    let iso = pa_isosurface(&pa, [0.0; 3], (lo + hi) / 2.0, SurfaceOptions::default()).unwrap();
    let dir = tempdir().unwrap();
    for (name, mesh) in [("geometry", &geometry), ("isopotential", &iso)] {
        for ext in ["stl", "obj", "gltf", "glb"] {
            let output = dir.path().join(format!("{name}.{ext}"));
            write_mesh(&output, mesh).unwrap();
            let decoded = read_mesh(output, ImportOptions::default()).unwrap();
            assert_eq!(decoded.triangle_count(), mesh.triangle_count());
        }
    }
    let output = dir.path().join("grid.vti");
    write_vti(&output, &pa, [0.0; 3], true).unwrap();
    let plane = arrays(&fs::read(&output).unwrap());
    let output = dir.path().join("grid.vts");
    write_revolved_vts(&output, &pa, [0.0; 3], true, 64, 360.0).unwrap();
    let revolved = arrays(&fs::read(&output).unwrap());
    assert_eq!(revolved["potential"].len(), 65 * plane["potential"].len());
    assert_eq!(revolved["field"].len(), 3 * revolved["potential"].len());
}

#[test]
fn vtp_polylines_preserve_sample_attributes_connectivity_and_ids() {
    let dir = tempdir().unwrap();
    let point = |time| TrajectoryPoint {
        time_us: time,
        position_mm: [time, 2.0 * time, 3.0 * time],
        velocity_mm_per_us: [1.0, 2.0, 3.0],
    };
    let trajectories = vec![
        Trajectory {
            id: 40,
            points: vec![point(0.0), point(1.0)],
        },
        Trajectory {
            id: 99,
            points: vec![point(2.0), point(3.0), point(4.0)],
        },
    ];
    let path = dir.path().join("paths.vtp");
    write_vtp(&path, &trajectories).unwrap();
    let payload = arrays(&fs::read(path).unwrap());
    assert_eq!(payload.len(), 6);
    assert_eq!(integers(&payload["connectivity"]), [0, 1, 2, 3, 4]);
    assert_eq!(integers(&payload["offsets"]), [2, 5]);
    assert_eq!(integers(&payload["trajectory_id"]), [40, 99]);
    assert_eq!(floats(&payload["time_us"]), [0.0, 1.0, 2.0, 3.0, 4.0]);
    assert_eq!(floats(&payload["position_mm"])[12..], [4.0, 8.0, 12.0]);
    assert_eq!(
        floats(&payload["velocity_mm_per_us"])[12..],
        [1.0, 2.0, 3.0]
    );
    let csv = dir.path().join("paths.csv");
    write_trajectories_csv(&csv, &trajectories).unwrap();
    let text = fs::read_to_string(csv).unwrap();
    let records: Vec<_> = text
        .lines()
        .skip(1)
        .map(|line| line.split(',').collect::<Vec<_>>())
        .collect();
    assert_eq!(records.len(), 5);
    assert_eq!(records[4], ["99", "4", "4", "8", "12", "1", "2", "3"]);
    let reversed = [Trajectory {
        id: 1,
        points: vec![point(1.0), point(0.0)],
    }];
    assert!(write_vtp(dir.path().join("invalid.vtp"), &reversed).is_err());
}

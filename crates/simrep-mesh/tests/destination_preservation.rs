use simrep_core::PotentialArray;
use simrep_io::pa::{PaFile, write_pa};
use simrep_mesh::{
    GridSpec, Mesh, Part, Trajectory, write_mesh, write_revolved_vts, write_scalar_vti,
    write_trajectories_csv, write_usda, write_vti, write_vtp,
};
use std::{fs, io, path::Path};
use tempfile::tempdir;

fn preserves_existing(extension: &str, export: impl FnOnce(&Path) -> io::Result<()>) {
    let dir = tempdir().unwrap();
    let path = dir.path().join(format!("existing.{extension}"));
    let sentinel = b"KEEP_EXISTING_EXPORT\x00\xff";
    fs::write(&path, sentinel).unwrap();
    let error = export(&path).unwrap_err();
    assert_eq!(
        error.kind(),
        io::ErrorKind::InvalidData,
        "{extension}: {error}"
    );
    assert_eq!(fs::read(&path).unwrap(), sentinel, "{extension}: {error}");
}

#[test]
fn float32_mesh_exports_preserve_existing_destination_on_range_error() {
    let mesh = Mesh {
        parts: vec![Part {
            name: "finite f64 triangle".into(),
            vertices: vec![[1e50, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            triangles: vec![[0, 1, 2]],
        }],
    };
    mesh.validate().unwrap();
    for extension in ["gltf", "glb", "stl", "usda"] {
        preserves_existing(extension, |path| {
            let result = write_mesh(path, &mesh);
            assert!(
                result
                    .as_ref()
                    .unwrap_err()
                    .to_string()
                    .contains("Float32 range")
            );
            result
        });
    }
}

#[test]
fn obj_preserves_existing_destination_on_invalid_mesh() {
    let mesh = Mesh {
        parts: vec![Part {
            name: "invalid index".into(),
            vertices: vec![[0.0; 3]; 3],
            triangles: vec![[0, 1, 3]],
        }],
    };
    preserves_existing("obj", |path| write_mesh(path, &mesh));
}

#[test]
fn scientific_exports_validate_before_opening_existing_destination() {
    let pa = PotentialArray::new(3, 3, 1);
    preserves_existing("vti", |path| {
        write_vti(path, &pa, [f64::NAN, 0.0, 0.0], true)
    });
    preserves_existing("vts", |path| {
        write_revolved_vts(path, &pa, [0.0; 3], true, 16, 360.0)
    });
    preserves_existing("scalar.vti", |path| {
        write_scalar_vti(
            path,
            GridSpec {
                dims: [3, 3, 1],
                spacing_mm: [1.0; 3],
                origin_mm: [0.0; 3],
            },
            "invalid",
            &[f64::NAN; 9],
        )
    });
    let trajectories = [Trajectory {
        id: 1,
        points: vec![],
    }];
    preserves_existing("vtp", |path| write_vtp(path, &trajectories));
    preserves_existing("csv", |path| write_trajectories_csv(path, &trajectories));
    preserves_existing("usda", |path| write_usda(path, None, &trajectories));
    let mut file = PaFile::new(pa);
    file.array.potentials[0] = f64::NAN;
    for extension in ["pa", "pa#", "pa0"] {
        preserves_existing(extension, |path| write_pa(path, &file));
    }
}

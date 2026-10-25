use simrep_core::{PotentialArray, Symmetry};
use simrep_mesh::{
    ImportOptions, Mesh, Part, Solid, SurfaceOptions, electrode_surface,
    electrode_surface_with_options, inspect, pa_isosurface, read_mesh, write_mesh,
};
use std::f64::consts::PI;
use tempfile::tempdir;

fn measurements(part: &Part) -> (f64, f64) {
    let mut volume = 0.0;
    let mut area = 0.0;
    for t in &part.triangles {
        let [a, b, c] = t.map(|i| part.vertices[i]);
        let u: [f64; 3] = std::array::from_fn(|i| b[i] - a[i]);
        let v: [f64; 3] = std::array::from_fn(|i| c[i] - a[i]);
        let cross = [
            u[1] * v[2] - u[2] * v[1],
            u[2] * v[0] - u[0] * v[2],
            u[0] * v[1] - u[1] * v[0],
        ];
        area += cross.iter().map(|v| v * v).sum::<f64>().sqrt() / 2.0;
        volume += a.iter().zip(cross).map(|(a, c)| a * c).sum::<f64>() / 6.0;
    }
    (volume, area)
}

fn cylindrical_electrode(inner_node: usize) -> PotentialArray {
    let mut pa = PotentialArray::new(5, 5, 1);
    pa.symmetry = Symmetry::Cylindrical;
    pa.mirrors.y = true;
    for y in inner_node..3 {
        for x in 1..4 {
            let i = pa.index(x, y, 0).unwrap();
            pa.potentials[i] = 7.0;
            pa.electrodes[i] = true;
        }
    }
    pa
}

#[test]
fn cylinder_annulus_and_capped_cutaway_match_analytic_volume_and_area() {
    for inner_node in [0, 1] {
        let pa = cylindrical_electrode(inner_node);
        let inner = if inner_node == 0 { 0.0 } else { 0.5 };
        let outer = 2.5;
        let length = 3.0;
        for degrees in [360.0, 270.0] {
            let options = SurfaceOptions {
                sweep_degrees: degrees,
                ..SurfaceOptions::default()
            };
            let mesh = electrode_surface_with_options(&pa, [0.0; 3], None, options).unwrap();
            let part = &mesh.parts[0];
            assert_eq!(inspect(part).unwrap(), Default::default());
            let (volume, area) = measurements(part);
            let fraction = degrees / 360.0;
            let expected_volume = fraction * PI * (outer * outer - inner * inner) * length;
            let expected_area = fraction
                * (2.0 * PI * (outer + inner) * length
                    + 2.0 * PI * (outer * outer - inner * inner))
                + if degrees == 360.0 {
                    0.0
                } else {
                    2.0 * length * (outer - inner)
                };
            assert!(
                (volume / expected_volume - 1.0).abs() < 0.002,
                "{volume} vs {expected_volume}"
            );
            assert!(
                (area / expected_area - 1.0).abs() < 0.002,
                "{area} vs {expected_area}"
            );
            let solid = Solid::new(part).unwrap();
            assert!(solid.contains([2.0, 1.0, 0.1]));
            assert_eq!(solid.contains([2.0, 0.0, 0.0]), inner == 0.0);
        }
    }
}

#[test]
fn planar_extrusion_expands_mirrors_and_retains_electrode_interfaces() {
    let mut pa = PotentialArray::new(3, 3, 1);
    pa.mirrors.x = true;
    pa.mirrors.y = true;
    pa.dx_mm = 0.5;
    pa.dy_mm = 2.0;
    for y in 0..2 {
        for x in 0..2 {
            let i = pa.index(x, y, 0).unwrap();
            pa.potentials[i] = if y == 0 { 7.0 } else { 8.0 };
            pa.electrodes[i] = true;
        }
    }
    let options = SurfaceOptions {
        extrusion_depth_mm: 4.0,
        ..SurfaceOptions::default()
    };
    let mesh = electrode_surface_with_options(&pa, [3.0, 4.0, 5.0], None, options).unwrap();
    assert_eq!(mesh.parts.len(), 2);
    let mut volume = 0.0;
    for part in &mesh.parts {
        assert_eq!(inspect(part).unwrap(), Default::default());
        volume += measurements(part).0;
        let bounds = Solid::new(part).unwrap().bounds();
        assert_eq!([bounds[0][0], bounds[1][0]], [2.25, 3.75]);
        assert_eq!([bounds[0][2], bounds[1][2]], [3.0, 7.0]);
    }
    assert!((volume - 1.5 * 6.0 * 4.0).abs() < 1e-10);
    let selected = electrode_surface_with_options(&pa, [0.0; 3], Some(7.0), options).unwrap();
    assert_eq!(selected.parts.len(), 1);
    for nz in [1, 3] {
        let mut pa = PotentialArray::new(3, 3, nz);
        pa.electrodes[0] = true;
        pa.potentials[0] = 7.0;
        let error = electrode_surface(&pa, [0.0; 3], Some(9.0))
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("electrode 9 has no exposed faces"),
            "{error}"
        );
        assert!(
            error.contains("accepted electrode values: [7.0]"),
            "{error}"
        );
    }
}

#[test]
fn cylindrical_axial_mirror_and_mesh_formats_preserve_closed_solids() {
    let mut pa = cylindrical_electrode(1);
    pa.mirrors.x = true;
    let mesh = electrode_surface(&pa, [3.0, 4.0, 5.0], None).unwrap();
    assert_eq!(inspect(&mesh.parts[0]).unwrap(), Default::default());
    let dir = tempdir().unwrap();
    for ext in ["stl", "obj", "gltf", "glb"] {
        let path = dir.path().join(format!("annulus.{ext}"));
        write_mesh(&path, &mesh).unwrap();
        let decoded = read_mesh(path, ImportOptions::default()).unwrap();
        // STL splits disconnected solids; named formats retain one electrode.
        assert_eq!(decoded.parts.len(), if ext == "stl" { 2 } else { 1 });
        assert_eq!(decoded.triangle_count(), mesh.triangle_count());
        for part in decoded.parts {
            assert_eq!(inspect(&part).unwrap(), Default::default(), "{ext}");
        }
    }
}

#[test]
fn marching_squares_revolves_sphere_and_extrudes_anisotropic_plane() {
    let mut pa = PotentialArray::new(21, 21, 1);
    pa.symmetry = Symmetry::Cylindrical;
    pa.mirrors.x = true;
    pa.mirrors.y = true;
    pa.dx_mm = 0.1;
    pa.dy_mm = 0.1;
    for y in 0..pa.ny {
        for x in 0..pa.nx {
            let i = pa.index(x, y, 0).unwrap();
            pa.potentials[i] = (x as f64 * 0.1).powi(2) + (y as f64 * 0.1).powi(2);
        }
    }
    let sphere = pa_isosurface(&pa, [0.0; 3], 2.25, SurfaceOptions::default()).unwrap();
    assert_eq!(inspect(&sphere.parts[0]).unwrap(), Default::default());
    let (volume, area) = measurements(&sphere.parts[0]);
    assert!((volume / (4.0 / 3.0 * PI * 1.5_f64.powi(3)) - 1.0).abs() < 0.01);
    assert!((area / (4.0 * PI * 1.5_f64.powi(2)) - 1.0).abs() < 0.01);

    pa.symmetry = Symmetry::Planar;
    pa.mirrors = Default::default();
    pa.dx_mm = 0.5;
    pa.dy_mm = 2.0;
    for i in 0..pa.potentials.len() {
        pa.potentials[i] = 1e12 + (i % pa.nx) as f64 * pa.dx_mm;
    }
    let mesh = pa_isosurface(
        &pa,
        [3.0, 4.0, 5.0],
        1e12 + 1.25,
        SurfaceOptions {
            extrusion_depth_mm: 6.0,
            ..SurfaceOptions::default()
        },
    )
    .unwrap();
    assert!(mesh.parts[0].vertices.iter().all(|p| p[0] == 4.25));
    let bounds = Solid::from_surface(&mesh.parts[0]).unwrap().bounds();
    assert_eq!([bounds[0][2], bounds[1][2]], [2.0, 8.0]);
    assert_eq!(measurements(&mesh.parts[0]).1, 40.0 * 6.0);
}

#[test]
fn rejects_invalid_sweep_parameters_and_empty_contours() {
    let pa = cylindrical_electrode(0);
    for options in [
        SurfaceOptions {
            angular_segments: 2,
            ..SurfaceOptions::default()
        },
        SurfaceOptions {
            sweep_degrees: 0.0,
            ..SurfaceOptions::default()
        },
        SurfaceOptions {
            sweep_degrees: 361.0,
            ..SurfaceOptions::default()
        },
        SurfaceOptions {
            extrusion_depth_mm: f64::NAN,
            ..SurfaceOptions::default()
        },
    ] {
        assert!(electrode_surface_with_options(&pa, [0.0; 3], None, options).is_err());
    }
    assert!(pa_isosurface(&pa, [0.0; 3], 100.0, SurfaceOptions::default()).is_err());
    let empty = Mesh::default();
    assert!(empty.validate().is_err());
}

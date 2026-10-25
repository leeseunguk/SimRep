mod common;

use common::cube;
use simrep_core::PotentialArray;
use simrep_mesh::{
    GridSpec, Mesh, Part, Solid, electrode_surface, inspect, isosurface, repair, sample_sdf,
    voxelize, voxelize_with_defects,
};
use std::f64::consts::TAU;

#[test]
fn portable_source_mesh_and_exact_finite_segment_hits() {
    let part = cube(-1.0, 1.0);
    let bytes = part.source_bytes().unwrap();
    let restored = Part::from_source_bytes(&bytes).unwrap();
    assert_eq!(restored.vertices, part.vertices);
    assert_eq!(restored.triangles, part.triangles);
    assert!(Part::from_source_bytes(&bytes[..bytes.len() - 1]).is_err());
    let mesh = Solid::new(&restored).unwrap();
    for (from, to) in [
        ([-2.0, 0.2, 0.3], [2.0, 0.2, 0.3]),
        ([-2.0, 1.0, 0.3], [2.0, 1.0, 0.3]), // coplanar face
        ([-2.0, 1.0, 1.0], [2.0, 1.0, 1.0]), // shared edge
        ([-2.0; 3], [2.0; 3]),               // shared vertex
    ] {
        let hits = mesh.segment_intersections(from, to);
        assert!((hits[0] - 0.25).abs() < 1e-12, "{hits:?}");
        assert!((hits[hits.len() - 1] - 0.75).abs() < 1e-12, "{hits:?}");
    }
    assert!(
        mesh.segment_intersections([-2.0, 0.0, 0.0], [-1.1, 0.0, 0.0])
            .is_empty()
    );
}

fn sphere(latitudes: usize, longitudes: usize) -> Part {
    let mut part = Part {
        name: "sphere".into(),
        vertices: vec![[0.0, 0.0, 1.0], [0.0, 0.0, -1.0]],
        triangles: Vec::new(),
    };
    for latitude in 1..latitudes {
        let theta = std::f64::consts::PI * latitude as f64 / latitudes as f64;
        for longitude in 0..longitudes {
            let phi = TAU * longitude as f64 / longitudes as f64;
            part.vertices.push([
                theta.sin() * phi.cos(),
                theta.sin() * phi.sin(),
                theta.cos(),
            ]);
        }
    }
    for j in 0..longitudes {
        let next = (j + 1) % longitudes;
        part.triangles.push([0, 2 + j, 2 + next]);
        let last = 2 + (latitudes - 2) * longitudes;
        part.triangles.push([1, last + next, last + j]);
        for i in 0..latitudes - 2 {
            let a = 2 + i * longitudes + j;
            let b = 2 + i * longitudes + next;
            let c = a + longitudes;
            let d = b + longitudes;
            part.triangles.extend([[a, c, d], [a, d, b]]);
        }
    }
    part
}

fn torus(major: usize, minor: usize) -> Part {
    let mut part = Part {
        name: "torus".into(),
        ..Part::default()
    };
    for i in 0..major {
        let u = TAU * i as f64 / major as f64;
        for j in 0..minor {
            let v = TAU * j as f64 / minor as f64;
            part.vertices.push([
                (2.0 + 0.6 * v.cos()) * u.cos(),
                (2.0 + 0.6 * v.cos()) * u.sin(),
                0.6 * v.sin(),
            ]);
            let a = i * minor + j;
            let b = ((i + 1) % major) * minor + j;
            let c = ((i + 1) % major) * minor + (j + 1) % minor;
            let d = i * minor + (j + 1) % minor;
            part.triangles.extend([[a, b, c], [a, c, d]]);
        }
    }
    part
}

#[test]
fn batched_sdf_matches_point_queries_including_aligned_faces_and_cavities() {
    for part in [cube(-1.0, 1.0), torus(32, 16)] {
        let solid = Solid::new(&part).unwrap();
        let grid = GridSpec {
            dims: [25, 23, 21],
            origin_mm: [-3.0, -3.0, -1.0],
            spacing_mm: [0.25, 0.25, 0.1],
        };
        let mut values = vec![0.0; grid.validate().unwrap()];
        simrep_geom::SignedDistance::sample_grid(
            &solid,
            grid.dims,
            grid.origin_mm,
            grid.spacing_mm,
            &mut values,
        );
        for (i, value) in values.into_iter().enumerate() {
            assert_eq!(value, solid.signed_distance(grid.point(i)));
        }
    }
}

#[test]
fn narrow_band_sdf_preserves_exact_near_surface_distances_and_all_flags() {
    let mut cavity = cube(-2.0, 2.0);
    let inner = cube(-1.0, 1.0);
    let offset = cavity.vertices.len();
    cavity.vertices.extend(inner.vertices);
    cavity.triangles.extend(
        inner
            .triangles
            .into_iter()
            .map(|face| face.map(|i| i + offset)),
    );
    for part in [cube(-1.0, 1.0), sphere(12, 24), torus(32, 16), cavity] {
        let solid = Solid::new(&part).unwrap();
        let grid = GridSpec {
            dims: [29, 27, 25],
            origin_mm: [-3.0, -3.0, -1.0],
            spacing_mm: [0.25, 0.25, 0.1],
        };
        let band = 0.5;
        let sampled = sample_sdf(&part, grid, 1, 0.0).unwrap();
        for (i, value) in sampled.values.iter().enumerate() {
            let exact = solid.signed_distance(grid.point(i));
            assert!(
                (value - exact.clamp(-band, band)).abs() <= 1e-12,
                "{i}: {value} != {exact}"
            );
            assert_eq!(*value <= 0.0, exact <= 0.0, "{i}");
        }
    }
}

fn compare(part: Part, exact: impl Fn([f64; 3]) -> f64, tolerance: f64) {
    assert!(inspect(&part).unwrap().is_closed_manifold());
    let solid = Solid::new(&part).unwrap();
    let mut max_error = 0.0_f64;
    for z in -7..=7 {
        for y in -7..=7 {
            for x in -7..=7 {
                let p = [
                    x as f64 * 0.43 + 0.113,
                    y as f64 * 0.43 + 0.071,
                    z as f64 * 0.43 + 0.019,
                ];
                let analytic = exact(p);
                let distance = solid.signed_distance(p);
                max_error = max_error.max((distance - analytic).abs());
                if analytic.abs() > tolerance {
                    assert_eq!(
                        solid.contains(p),
                        analytic < 0.0,
                        "{p:?}: mesh={distance}, exact={analytic}"
                    );
                }
            }
        }
    }
    assert!(
        max_error < tolerance,
        "SDF error {max_error} >= {tolerance}"
    );
}

#[test]
fn cube_sphere_torus_membership_and_true_distances() {
    compare(
        cube(-1.0, 1.0),
        |p| {
            let q = p.map(|x| x.abs() - 1.0);
            q.iter().map(|x| x.max(0.0).powi(2)).sum::<f64>().sqrt()
                + q.into_iter().fold(f64::NEG_INFINITY, f64::max).min(0.0)
        },
        1e-10,
    );
    compare(
        sphere(24, 48),
        |p| p.iter().map(|v| v * v).sum::<f64>().sqrt() - 1.0,
        0.009,
    );
    compare(
        torus(64, 32),
        |[x, y, z]| (x.hypot(y) - 2.0).hypot(z) - 0.6,
        0.012,
    );
}

#[test]
fn curved_sdf_error_decreases_with_mesh_resolution() {
    let coarse = Solid::new(&sphere(8, 16)).unwrap();
    let fine = Solid::new(&sphere(32, 64)).unwrap();
    let mut coarse_error = 0.0_f64;
    let mut fine_error = 0.0_f64;
    for i in 1..80 {
        let theta = i as f64 * 0.117;
        let p = [0.8 * theta.cos(), 0.8 * theta.sin(), 0.5];
        let exact = p.iter().map(|v| v * v).sum::<f64>().sqrt() - 1.0;
        coarse_error = coarse_error.max((coarse.signed_distance(p) - exact).abs());
        fine_error = fine_error.max((fine.signed_distance(p) - exact).abs());
    }
    assert!(
        fine_error < coarse_error / 8.0,
        "{coarse_error} -> {fine_error}"
    );
}

#[test]
fn repairs_duplicates_degenerates_and_flips_but_reports_holes_and_nonmanifolds() {
    let mut part = cube(0.0, 2.0);
    part.vertices.push(part.vertices[0]);
    part.triangles[0][0] = 8;
    part.triangles[2].swap(1, 2);
    part.triangles.push([0, 0, 1]);
    part.triangles.push(part.triangles[3]);
    let result = repair(&mut part).unwrap();
    assert_eq!(result.before.duplicate_vertices, 1);
    assert_eq!(result.before.degenerate_triangles, 1);
    assert_eq!(result.before.duplicate_triangles, 1);
    assert_eq!(result.removed_triangles, 2);
    assert_eq!(result.flipped_triangles, 1);
    assert_eq!(result.after, Default::default());
    part.triangles.pop();
    let hole = inspect(&part).unwrap();
    assert_eq!(hole.boundary_edges, 3);
    assert_eq!(hole.boundary_loops, 1);
    assert!(Solid::new(&part).is_err());
    assert_eq!(repair(&mut part).unwrap().after.boundary_edges, 3);
    let mesh = Mesh { parts: vec![part] };
    let grid = GridSpec {
        dims: [3; 3],
        origin_mm: [0.0; 3],
        spacing_mm: [1.0; 3],
    };
    assert!(voxelize(&mesh, grid, &[1]).is_err());
    assert!(
        voxelize_with_defects(&mesh, grid, &[1])
            .unwrap()
            .points_per_part[0]
            > 0
    );

    let mut junction = cube(0.0, 2.0);
    junction.vertices.push([1.0, 1.0, -1.0]);
    junction.triangles.push([0, 1, 8]);
    assert_eq!(inspect(&junction).unwrap().non_manifold_edges, 1);
}

#[test]
fn ray_grazing_surface_points_and_nested_cavity_are_deterministic() {
    let mut outer = cube(-2.0, 2.0);
    let inner = cube(-1.0, 1.0);
    let offset = outer.vertices.len();
    outer.vertices.extend(inner.vertices);
    outer
        .triangles
        .extend(inner.triangles.into_iter().map(|t| t.map(|i| i + offset)));
    let solid = Solid::new(&outer).unwrap();
    for _ in 0..3 {
        assert!(!solid.contains([0.0; 3]));
        assert!(solid.contains([1.5, 0.0, 0.0]));
        assert!(solid.contains([2.0, 2.0, 2.0]));
        assert_eq!(solid.signed_distance([1.0, 0.3, 0.2]), 0.0);
        assert!(!solid.contains([2.001, 2.0, 2.0]));
    }
}

#[test]
fn voxel_nodes_sdf_layout_metadata_interpolation_and_overlap() {
    let mesh = Mesh {
        parts: vec![cube(-1.0, 1.0), cube(0.0, 2.0)],
    };
    let grid = GridSpec {
        dims: [9, 9, 9],
        origin_mm: [-2.0; 3],
        spacing_mm: [0.5; 3],
    };
    let result = voxelize(&mesh, grid, &[3, 7]).unwrap();
    assert_eq!(result.points_per_part, [125, 125]);
    assert_eq!(result.overlap_assignments, 27);
    assert_eq!(
        result.array.potentials[result.array.index(4, 4, 4).unwrap()],
        7.0
    );
    assert_eq!(
        result.array.potentials[result.array.index(2, 2, 2).unwrap()],
        3.0
    );
    let sdf = sample_sdf(&mesh.parts[0], grid, 3, 120.0).unwrap();
    assert_eq!(sdf.dims, grid.dims);
    assert_eq!(sdf.origin, grid.origin_mm);
    assert_eq!(sdf.spacing, grid.spacing_mm);
    assert_eq!(sdf.electrode_number, 3);
    assert_eq!(sdf.voltage, 120.0);
    assert!(sdf.values.iter().all(|v| v.is_finite()));
    let center = 4 + 9 * (4 + 9 * 4);
    assert_eq!(sdf.values[center], -1.0);
    assert_eq!(sdf.interpolate([0.0; 3]), Some(-1.0));
    assert_eq!(sdf.interpolate([1.25, 0.0, 0.0]), Some(0.25));
    assert_eq!(sdf.interpolate([2.0, 0.0, 0.0]), Some(1.0));
    assert_eq!(sdf.interpolate([2.01, 0.0, 0.0]), None);
    assert!(voxelize(&mesh, grid, &[0, 7]).is_err());
}

#[test]
fn electrode_faces_roundtrip_voxel_membership_and_anisotropic_iso_plane() {
    let mut pa = PotentialArray::new(4, 4, 4);
    for z in 1..3 {
        for y in 1..3 {
            for x in 1..3 {
                let i = pa.index(x, y, z).unwrap();
                pa.electrodes[i] = true;
                pa.potentials[i] = 7.0;
            }
        }
    }
    let surface = electrode_surface(&pa, [0.0; 3], None).unwrap();
    assert_eq!(surface.triangle_count(), 48);
    assert_eq!(inspect(&surface.parts[0]).unwrap(), Default::default());
    let grid = GridSpec {
        dims: [4; 3],
        origin_mm: [0.0; 3],
        spacing_mm: [1.0; 3],
    };
    assert_eq!(voxelize(&surface, grid, &[7]).unwrap().array, pa);
    let grid = GridSpec {
        dims: [7, 8, 9],
        origin_mm: [-2.0, 3.0, 5.0],
        spacing_mm: [0.5, 0.25, 0.75],
    };
    let values: Vec<_> = (0..grid.validate().unwrap())
        .map(|i| 1e12 + grid.point(i)[0])
        .collect();
    let mesh = isosurface(&values, grid, 1e12 - 0.75).unwrap();
    assert!(
        mesh.parts[0]
            .vertices
            .iter()
            .all(|v| (v[0] + 0.75).abs() < 1e-9)
    );
    assert!(
        mesh.parts[0]
            .vertices
            .iter()
            .any(|v| v[1] == 3.0 && v[2] == 5.0)
    );
}

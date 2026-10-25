use simrep_mesh::{GridSpec, Mesh, Part, sample_sdf, voxelize};
use std::f64::consts::TAU;
use std::time::Instant;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let start = Instant::now();
    let (major, minor) = (1000, 500);
    let mut part = Part {
        name: "million-triangle-torus".into(),
        vertices: Vec::with_capacity(major * minor),
        triangles: Vec::with_capacity(major * minor * 2),
    };
    for i in 0..major {
        let u = TAU * i as f64 / major as f64;
        for j in 0..minor {
            let v = TAU * j as f64 / minor as f64;
            part.vertices.push([
                (0.7 + 0.23 * v.cos()) * u.cos(),
                (0.7 + 0.23 * v.cos()) * u.sin(),
                0.23 * v.sin(),
            ]);
            let a = i * minor + j;
            let b = ((i + 1) % major) * minor + j;
            let c = ((i + 1) % major) * minor + (j + 1) % minor;
            let d = i * minor + (j + 1) % minor;
            part.triangles.extend([[a, b, c], [a, c, d]]);
        }
    }
    println!(
        "mesh construction (excluded): {:.3} s",
        start.elapsed().as_secs_f64()
    );
    let mesh = Mesh { parts: vec![part] };
    let grid = GridSpec {
        dims: [256; 3],
        origin_mm: [-1.0; 3],
        spacing_mm: [2.0 / 255.0; 3],
    };
    if std::env::args().any(|arg| arg == "--sdf") {
        let result = sample_sdf(&mesh.parts[0], grid, 1, 1.0)?;
        println!(
            "triangles: {}, grid: 256^3, Rayon workers: {}, topology + BVH + SDF: {:.6} s",
            mesh.triangle_count(),
            rayon::current_num_threads(),
            result.elapsed.as_secs_f64()
        );
        assert!(result.values.iter().all(|v| v.is_finite()));
        return Ok(());
    }
    let result = voxelize(&mesh, grid, &[1])?;
    println!(
        "triangles: {}, grid: 256^3 ({} nodes), Rayon workers: {}",
        mesh.triangle_count(),
        grid.validate()?,
        rayon::current_num_threads()
    );
    println!(
        "topology + BVH + voxelization: {:.6} s; occupied: {:?}; overlaps: {}",
        result.elapsed.as_secs_f64(),
        result.points_per_part,
        result.overlap_assignments
    );
    let analytic_volume = 2.0 * std::f64::consts::PI.powi(2) * 0.7 * 0.23_f64.powi(2);
    let sampled_volume = result.points_per_part[0] as f64 * grid.spacing_mm.iter().product::<f64>();
    let error = (sampled_volume / analytic_volume - 1.0).abs();
    println!("torus volume relative error: {error:.6e}");
    if error > 0.01 {
        return Err("benchmark geometry failed analytic volume check".into());
    }
    Ok(())
}

use simrep_mesh::{
    GridSpec, ImportOptions, Solid, Transform, read_mesh, repair, voxelize_with_defects,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 2 && args.len() != 5 {
        return Err("usage: simion_compare STL ORACLE_PA [tx_mm ty_mm tz_mm]".into());
    }
    let mut transform = Transform::IDENTITY;
    if args.len() == 5 {
        for axis in 0..3 {
            transform.0[axis][3] = args[axis + 2].parse()?;
        }
    }
    let mut mesh = read_mesh(
        &args[0],
        ImportOptions {
            transform,
            ..ImportOptions::default()
        },
    )?;
    for part in &mut mesh.parts {
        let report = repair(part)?;
        println!("part {:?}: {report:?}", part.name);
        if !report.after.is_closed_manifold() {
            let mut edges = std::collections::BTreeMap::new();
            for &[a, b, c] in &part.triangles {
                for (a, b) in [(a, b), (b, c), (c, a)] {
                    *edges.entry((a.min(b), a.max(b))).or_insert(0usize) += 1;
                }
            }
            for ((a, b), uses) in edges.into_iter().filter(|(_, uses)| *uses != 2).take(12) {
                println!(
                    "  edge {:?} -> {:?}: {uses} triangle uses",
                    part.vertices[a], part.vertices[b]
                );
            }
        }
    }
    let oracle = simrep_io::read_pa(&args[1])?.array;
    let grid = GridSpec {
        dims: [oracle.nx, oracle.ny, oracle.nz],
        origin_mm: [0.0; 3],
        spacing_mm: [oracle.dx_mm, oracle.dy_mm, oracle.dz_mm],
    };
    let result = voxelize_with_defects(&mesh, grid, &vec![1; mesh.parts.len()])?;
    let solids: Vec<_> = mesh
        .parts
        .iter()
        .map(Solid::from_surface)
        .collect::<Result<_, _>>()?;
    let epsilon = grid.spacing_mm.into_iter().fold(f64::INFINITY, f64::min) * 1e-5;
    let mut mismatches = 0;
    let mut off_surface = 0;
    for (i, (&ours, &theirs)) in result
        .array
        .electrodes
        .iter()
        .zip(&oracle.electrodes)
        .enumerate()
    {
        if ours != theirs {
            mismatches += 1;
            let point = grid.point(i);
            let distance = solids
                .iter()
                .map(|s| s.distance(point))
                .fold(f64::INFINITY, f64::min);
            if distance > epsilon {
                if off_surface < 5 {
                    println!(
                        "off-surface mismatch at {point:?}: ours={ours}, SIMION={theirs}, distance_mm={distance}"
                    );
                }
                off_surface += 1;
            }
        }
    }
    println!(
        "nodes: {}, our metal: {}, SIMION metal: {}, mismatches: {mismatches}, off-surface mismatches: {off_surface}, elapsed: {:.6} s",
        grid.validate()?,
        result.array.electrodes.iter().filter(|&&v| v).count(),
        oracle.electrodes.iter().filter(|&&v| v).count(),
        result.elapsed.as_secs_f64()
    );
    if off_surface > 0 {
        return Err("SIMION voxelization differs away from the mesh surface".into());
    }
    Ok(())
}

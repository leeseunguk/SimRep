//! Compare sidecar and independent source collisions on identical potentials.
//! `cuts_trace STEM [--mesh exported.stl] [--output DIRECTORY] [--boundary]`
use std::error::Error;
use std::fs::{File, create_dir_all};
use std::io::{BufReader, BufWriter, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use simrep_fly::surface::{SourceSurface, SurfaceModel, SurfaceSamples};
use simrep_fly::{
    Field, FlightOptions, GridField, InstanceField, Particle, Reconstruction, fly_many,
};
use simrep_geom::{Electrode, SampledSdf, Sdf, SignedDistance};
use simrep_io::{PaFile, read_fast_adjust, read_particles, write_pa};
use simrep_solve::cutcell::CutCells;
use simrep_solve::mg::{Geometry, Grid, Options};

fn shapes() -> Vec<Arc<dyn SignedDistance>> {
    let rod = |y, z| {
        Arc::new(
            Sdf::Cylinder {
                center: [30.0, y, z],
                radius: 5.75,
                axis: 0,
            }
            .intersection(Sdf::Box {
                center: [30.0, 0.0, 0.0],
                half_extent: [18.0, 100.0, 100.0],
            }),
        ) as Arc<dyn SignedDistance>
    };
    let plate = |x| {
        Sdf::Box {
            center: [x, 0.0, 0.0],
            half_extent: [0.5, 100.0, 100.0],
        }
        .difference(Sdf::Cylinder {
            center: [30.0, 0.0, 0.0],
            radius: 4.0,
            axis: 0,
        })
    };
    vec![
        rod(10.75, 0.0),
        rod(0.0, 10.75),
        Arc::new(plate(6.5).union(plate(53.5))),
    ]
}

fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let stem = PathBuf::from(
        args.first()
            .ok_or("usage: cuts_trace STEM [--solve-exact]")?,
    );
    let output = args.windows(2).find(|a| a[0] == "--output").map_or_else(
        || PathBuf::from("target/cuts-trace"),
        |a| PathBuf::from(&a[1]),
    );
    create_dir_all(&output)?;
    let sidecar: serde_json::Value = serde_json::from_reader(BufReader::new(File::open(
        stem.with_extension("cuts.json"),
    )?))?;
    let origin: [f64; 3] = serde_json::from_value(sidecar["origin_mm"].clone())?;
    let (geometry, numbers) = if let Some(option) = args.windows(2).find(|a| a[0] == "--mesh") {
        let mesh = simrep_mesh::read_mesh(&option[1], Default::default())?;
        let sources: Vec<SourceSurface> =
            serde_json::from_value(sidecar["source_geometry"].clone())?;
        if sources.len() != mesh.parts.len() {
            return Err("mesh/source part counts differ".into());
        }
        let shapes = mesh
            .parts
            .iter()
            .map(|part| {
                let solid = simrep_mesh::Solid::new(part)?;
                Ok(Arc::new(move |p| solid.signed_distance(p)) as Arc<dyn SignedDistance>)
            })
            .collect::<Result<Vec<_>, std::io::Error>>()?;
        (shapes, sources.iter().map(|s| s.number).collect::<Vec<_>>())
    } else {
        (shapes(), vec![1, 2, 3])
    };
    let mut array = read_fast_adjust(stem.with_extension("pa0"), &[(1, 2.5), (2, -2.5), (3, 0.0)])?;
    if args.iter().any(|a| a == "--solve-exact") {
        let electrodes: Vec<_> = geometry
            .iter()
            .zip([2.5, -2.5, 0.0])
            .map(|(s, v)| {
                let s = Arc::clone(s);
                Electrode::new(v, move |p| s.distance(p))
            })
            .collect();
        let cuts = CutCells::new_with_root_tolerance(
            Grid {
                dims: [array.nx, array.ny, array.nz],
                spacing: [array.dx_mm, array.dy_mm, array.dz_mm],
                geometry: Geometry::Planar,
            },
            [0.0; 3],
            &electrodes,
            1e-11,
        )?;
        let report = cuts.solve(
            &mut array.potentials,
            Options {
                relative_tolerance: 1e-12,
                ..Default::default()
            },
        )?;
        assert!(report.converged);
        write_pa(output.join("exact.pa"), &PaFile::new(array.clone()))?;
    }
    let samples: Vec<SurfaceSamples> = serde_json::from_value(sidecar["sampled_sdfs"].clone())?;
    let sampled: Vec<_> = samples
        .into_iter()
        .map(|s| SampledSdf::new(s.dims, s.origin_mm, s.spacing_mm, s.values))
        .collect::<Result<_, _>>()?;
    let particles = read_particles("/mnt/c/Users/lsu10/simrep-oracle/_quad_gpu/particles.ion")?
        .into_iter()
        .map(|p| {
            Particle::from_relativistic_energy(
                p.mass_amu,
                p.charge_e,
                p.position_mm,
                p.energy_ev,
                p.direction,
                p.birth_time_us,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    let options = FlightOptions {
        duration_us: 100.0,
        relative_tolerance: 1e-11,
        position_tolerance_mm: 1e-11,
        velocity_tolerance_mm_per_us: 1e-11,
        max_step_us: 0.025,
        relativistic: true,
        sample_every: Some(1),
        ..Default::default()
    };
    let reconstruction = if args.iter().any(|a| a == "--boundary") {
        Reconstruction::BoundaryAware
    } else {
        Reconstruction::GlobalCubic
    };
    for route in ["sidecar", "source"] {
        let surface = if route == "sidecar" {
            SurfaceModel::from_reader(BufReader::new(File::open(
                stem.with_extension("cuts.json"),
            )?))?
        } else {
            SurfaceModel::from_sdfs(
                geometry
                    .iter()
                    .zip(&numbers)
                    .map(|(s, n)| (*n, Arc::clone(s)))
                    .collect(),
                5e-11,
            )?
            .with_pa_domain(&array, origin)?
        };
        let field = InstanceField::new(
            GridField::with_reconstruction(array.clone(), reconstruction)?,
            origin,
            [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            1.0,
            Some([[0.0, -18.0, -18.0], [60.0, 18.0, 18.0]]),
            None,
        )?
        .with_surface(Arc::new(surface))?;
        let start = Instant::now();
        let results = fly_many(&field, &particles, &options)?;
        println!("{route}: flight_s={:.6}", start.elapsed().as_secs_f64());
        let mut endpoints = BufWriter::new(File::create(output.join(format!("{route}.csv")))?);
        writeln!(endpoints, "ion,tof_us,x_mm,y_mm,z_mm,reason")?;
        let mut trace = BufWriter::new(File::create(output.join(format!("{route}-steps.csv")))?);
        writeln!(
            trace,
            "ion,step,time_us,x,y,z,vx,vy,vz,ex,ey,ez,source_d,sampled_d"
        )?;
        for (i, result) in results.iter().enumerate() {
            let [x, y, z] = result.final_state.position_mm;
            writeln!(
                endpoints,
                "{},{:.17e},{x:.17e},{y:.17e},{z:.17e},{:?}",
                i + 1,
                result.tof_us,
                result.splat_reason
            )?;
            if ![1, 500, 900, 971, 972, 973, 1000].contains(&(i + 1)) {
                continue;
            }
            for (step, state) in result.trajectory.as_ref().unwrap().iter().enumerate() {
                let [x, y, z] = state.position_mm;
                let [vx, vy, vz] = state.velocity_mm_per_us;
                let [ex, ey, ez] = field
                    .sample(state.position_mm, state.time_us)
                    .electric_v_per_mm;
                let p = [
                    x,
                    if array.mirrors.y { y.abs() } else { y },
                    if array.mirrors.z { z.abs() } else { z },
                ];
                let source_d = geometry
                    .iter()
                    .map(|s| s.distance(p))
                    .fold(f64::INFINITY, f64::min);
                let sampled_d = sampled
                    .iter()
                    .map(|s| s.distance(p))
                    .fold(f64::INFINITY, f64::min);
                writeln!(
                    trace,
                    "{},{step},{:.17e},{x:.17e},{y:.17e},{z:.17e},{vx:.17e},{vy:.17e},{vz:.17e},{ex:.17e},{ey:.17e},{ez:.17e},{source_d:.17e},{sampled_d:.17e}",
                    i + 1,
                    state.time_us
                )?;
            }
        }
    }
    Ok(())
}

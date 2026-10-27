//! Compare numerical bytes, not rounded text or wall-clock diagnostics.
use std::fs;
use std::path::Path;

use clap::Parser;
use rayon::ThreadPoolBuilder;
use serde_json::Value;
use simrep_fly::{
    AnalyticField, FieldSample, FlightOptions, FlightResult, Integrator, Particle, ParticleState,
    SplatReason, StopPlane, fly_many,
    physics::{
        BeamInteraction, BeamOptions, ChargeMode, DirectCharge, EnsembleOptions, EnsembleReport,
        PicMesh, SpaceCharge,
        collision::{CollisionModel, DiffusionStatistics, Gas, Hs1, Sds, fly_many_collisional},
        fly_beam, fly_ensemble,
    },
};
use simrep_geom::{Electrode, Sdf};
use simrep_mesh::{GridSpec, Mesh, Part, Solid};
use simrep_solve::{
    cutcell::CutCells,
    mg::{self, Geometry, Grid, Options, Residual},
};

#[track_caller]
fn across_threads(work: impl Fn() -> Vec<u8> + Sync) {
    let mut expected = None;
    for threads in [1, 2, 8, 32] {
        let actual = ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap()
            .install(&work);
        if let Some(expected) = &expected {
            assert!(
                actual == *expected,
                "numerical bytes differ with {threads} requested Rayon threads; first difference at byte {:?}",
                actual.iter().zip(expected).position(|(a, b)| a != b)
            );
        } else {
            expected = Some(actual);
        }
    }
}

fn floats(out: &mut Vec<u8>, values: impl IntoIterator<Item = f64>) {
    for value in values {
        out.extend_from_slice(&value.to_le_bytes());
    }
}

fn count(out: &mut Vec<u8>, value: usize) {
    out.extend_from_slice(&(value as u64).to_le_bytes());
}

fn state(out: &mut Vec<u8>, state: ParticleState) {
    floats(out, state.position_mm);
    floats(out, state.velocity_mm_per_us);
    floats(out, [state.time_us]);
}

fn flights(results: &[FlightResult]) -> Vec<u8> {
    let mut out = Vec::new();
    count(&mut out, results.len());
    for result in results {
        state(&mut out, result.final_state);
        let (tag, index) = match result.splat_reason {
            SplatReason::Electrode { index } => (0, index),
            SplatReason::SurfaceElectrode { number } => (1, number),
            SplatReason::OutsideGrid => (2, 0),
            SplatReason::StopPlane => (3, 0),
            SplatReason::TimeLimit => (4, 0),
            SplatReason::StepLimit => (5, 0),
            SplatReason::StepUnderflow => (6, 0),
            SplatReason::NonFinite => (7, 0),
        };
        out.push(tag);
        count(&mut out, index);
        floats(&mut out, [result.tof_us]);
        count(&mut out, result.accepted_steps);
        count(&mut out, result.rejected_steps);
        out.push(u8::from(result.trajectory.is_some()));
        if let Some(trajectory) = &result.trajectory {
            count(&mut out, trajectory.len());
            for &sample in trajectory {
                state(&mut out, sample);
            }
        }
    }
    out
}

fn ensemble(report: &EnsembleReport) -> Vec<u8> {
    let mut out = flights(&report.results);
    count(&mut out, report.global_steps);
    count(&mut out, report.rejected_steps);
    out
}

fn residuals(out: &mut Vec<u8>, iterations: usize, levels: usize, history: &[Residual]) {
    count(out, iterations);
    count(out, levels);
    count(out, history.len());
    for residual in history {
        floats(out, [residual.absolute, residual.relative]);
    }
}

fn laplace_case(dims: [usize; 3]) -> (Grid, Vec<bool>, Vec<f64>) {
    let grid = Grid {
        dims,
        spacing: [0.13, 0.17, 0.23],
        geometry: Geometry::Planar,
    };
    let n = dims.into_iter().product();
    let mut fixed = vec![false; n];
    let mut initial = vec![0.0; n];
    for i in 0..n {
        let c = [i % dims[0], i / dims[0] % dims[1], i / (dims[0] * dims[1])];
        if (0..3).any(|a| dims[a] > 1 && (c[a] == 0 || c[a] + 1 == dims[a]))
            || (c[0] == dims[0] / 2 && c[1] < dims[1] / 3)
        {
            fixed[i] = true;
            initial[i] =
                3.0 + 0.21 * c[0] as f64 - 0.13 * c[1] as f64 + 0.017 * (c[2] * c[2]) as f64;
        }
    }
    (grid, fixed, initial)
}

#[test]
fn repeatability_multigrid_potentials_and_every_residual() {
    // Exercise both the small-pool policy and a >1048576-node problem
    // that actually executes with every requested worker count.
    for dims in [[65, 49, 33], [129, 97, 85]] {
        let (grid, fixed, initial) = laplace_case(dims);
        across_threads(|| {
            let mut values = initial.clone();
            let report = mg::solve(&mut values, &fixed, grid, Options::default()).unwrap();
            assert!(report.converged && report.iterations > 1);
            let requested = rayon::current_num_threads();
            assert_eq!(
                report.worker_threads,
                if initial.len() <= 1_048_576 {
                    requested.min(4)
                } else {
                    requested
                }
            );
            let mut out = Vec::new();
            floats(&mut out, values);
            residuals(
                &mut out,
                report.iterations,
                report.levels,
                &report.residual_history,
            );
            out
        });
    }
}

fn command(args: &[String]) -> Value {
    let mut summary = crate::json::capture(crate::Cli::parse_from(
        ["simrep".to_owned(), "--json".to_owned()]
            .into_iter()
            .chain(args.iter().cloned()),
    ))
    .unwrap()
    .unwrap();
    // Compare all stable machine output; durations are intentionally excluded.
    for key in ["compute_ms", "prep_ms"] {
        assert!(summary.as_object_mut().unwrap().remove(key).is_some());
    }
    summary
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
            let phi = std::f64::consts::TAU * longitude as f64 / longitudes as f64;
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
            part.triangles
                .extend([[a, a + longitudes, b + longitudes], [a, b + longitudes, b]]);
        }
    }
    part
}

fn geometry_command(input: &Path, stem: &Path, mesh: bool) -> Vec<u8> {
    let mut args = vec![
        "refine-geometry".into(),
        input.display().to_string(),
        "--device=cpu".into(),
        "--grid=25,23,21".into(),
        "--spacing=0.125,0.125,0.125".into(),
        "--origin=-1.5,-1.375,-1.25".into(),
        "--sidecar-dense-sdf".into(),
        "--surface-supersampling=1".into(),
        format!("--out={}", stem.display()),
    ];
    args.extend(
        if mesh {
            ["--electrode=0=1:17", "--electrode=1=2:-3"]
        } else {
            ["--electrode=1=1:17", "--electrode=2=2:-3"]
        }
        .map(str::to_owned),
    );
    let summary = command(&args);
    assert_eq!(summary["converged"], true);
    assert!(summary["cycles"].as_u64().unwrap() > 0);
    let paths = summary["output_paths"].as_array().unwrap();
    assert_eq!(paths.len(), 5); // PA#, PA0, two bases and cuts-v3 sidecar.
    let mut out = serde_json::to_vec(&summary).unwrap();
    for path in paths {
        let bytes = fs::read(path.as_str().unwrap()).unwrap();
        count(&mut out, bytes.len());
        out.extend(bytes);
    }
    out
}

#[test]
fn repeatability_gem_and_parallel_mesh_bvh_geometry_preparation() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("spheres.obj");
    // 36480 triangles exceed Node::build's 32768-triangle rayon::join
    // threshold, including equal-centroid ties and shared edges/vertices.
    let source = sphere(96, 192);
    assert!(source.triangles.len() > 32_768);
    let parts: Vec<_> = [-0.7, 0.7]
        .into_iter()
        .map(|offset| {
            let mut part = source.clone();
            for p in &mut part.vertices {
                *p = [offset + 0.34 * p[0], 0.34 * p[1], 0.34 * p[2]];
            }
            part
        })
        .collect();
    simrep_mesh::write_mesh(
        &input,
        &Mesh {
            parts: parts.clone(),
        },
    )
    .unwrap();
    let spec = GridSpec {
        dims: [25, 23, 21],
        origin_mm: [-1.5, -1.375, -1.25],
        spacing_mm: [0.125; 3],
    };
    across_threads(|| {
        let solid = Solid::new(&parts[0]).unwrap();
        let sampled = simrep_mesh::sample_sdf(&parts[0], spec, 1, 17.0).unwrap();
        assert!(sampled.values.iter().any(|v| *v < 0.0));
        let voxel = simrep_mesh::voxelize(
            &Mesh {
                parts: parts.clone(),
            },
            spec,
            &[1, 2],
        )
        .unwrap();
        assert!(voxel.points_per_part.iter().all(|n| *n > 0));
        let mut out = geometry_command(&input, &dir.path().join("mesh"), true);
        floats(&mut out, sampled.values);
        floats(&mut out, voxel.array.potentials);
        out.extend(voxel.array.electrodes.into_iter().map(u8::from));
        for n in voxel.points_per_part {
            count(&mut out, n);
        }
        count(&mut out, voxel.overlap_assignments);
        // Include exact ray roots as well as batched masks and narrow-band SDF.
        for y in [0.0, 0.031, 0.34] {
            let hits = solid.segment_intersections([-1.5, y, 0.0], [0.0, y, 0.0]);
            count(&mut out, hits.len());
            floats(&mut out, hits);
        }
        let cuts = CutCells::new(
            Grid {
                dims: spec.dims,
                spacing: spec.spacing_mm,
                geometry: Geometry::Planar,
            },
            spec.origin_mm,
            &[Electrode::new(17.0, solid)],
        )
        .unwrap();
        assert!(cuts.cut_nodes() > 0);
        count(&mut out, cuts.cut_nodes());
        floats(&mut out, [cuts.snap_distance]);
        out.extend(cuts.fixed.iter().copied().map(u8::from));
        floats(&mut out, cuts.values.iter().copied());
        // Cut-aware gradients observe the private cut coefficients/voltages.
        for i in 0..cuts.values.len() {
            if let Some(e) = cuts.electric_field(&cuts.values, i) {
                floats(&mut out, e);
            }
        }
        out
    });
    let gem = dir.path().join("spheres.gem");
    fs::write(&gem, "pa_define(25,23,21,planar)\ne(1){fill{within{sphere(-0.7,0,0,0.34)}}}\ne(2){fill{within{sphere(0.7,0,0,0.34)}}}").unwrap();
    across_threads(|| geometry_command(&gem, &dir.path().join("gem"), false));
}

fn particles(n: usize) -> Vec<Particle> {
    (0..n)
        .map(|i| {
            Particle::new(
                40.0 + (i % 5) as f64,
                1.0,
                [
                    0.0,
                    (i % 19) as f64 * 0.035 - 0.3,
                    (i / 19) as f64 * 0.035 - 0.3,
                ],
                [1.0 + (i % 11) as f64 * 0.013, 0.001, -0.002],
                (i % 3) as f64 * 0.0001,
            )
            .unwrap()
        })
        .collect()
}

fn flight_options() -> FlightOptions {
    FlightOptions {
        duration_us: 0.2,
        initial_step_us: 0.01,
        max_step_us: 0.02,
        sample_every: Some(2),
        relative_tolerance: 1e-7,
        position_tolerance_mm: 1e-9,
        velocity_tolerance_mm_per_us: 1e-9,
        ..Default::default()
    }
}

#[test]
fn repeatability_flight_trajectories_csv_and_production_tof_statistics() {
    let field = AnalyticField(|p: [f64; 3], t: f64| FieldSample {
        electric_v_per_mm: [0.01 * t.cos(), -0.03 * p[1], 0.02 * p[2]],
        magnetic_gauss: [0.0, 0.0, 20.0],
        ..Default::default()
    });
    let p = particles(513);
    across_threads(|| {
        let mut out = Vec::new();
        for integrator in [Integrator::Rk45, Integrator::Boris] {
            let options = FlightOptions {
                integrator,
                stop_plane: Some(StopPlane {
                    axis: 0,
                    coordinate_mm: 0.15,
                }),
                ..flight_options()
            };
            let results = fly_many(&field, &p, &options).unwrap();
            assert!(
                results
                    .iter()
                    .all(|r| r.splat_reason == SplatReason::StopPlane)
            );
            out.extend(flights(&results));
        }
        out
    });
    let dir = tempfile::tempdir().unwrap();
    let ion = dir.path().join("ions.ion");
    let csv = dir.path().join("flight.csv");
    let rows: String = (0..513)
        .map(|i| {
            format!(
                "{},40,1,0,0,0,0,0,{},1\n",
                (i % 7) as f64 * 0.01,
                0.2 + (i % 31) as f64 * 0.1
            )
        })
        .collect();
    fs::write(&ion, rows).unwrap();
    let args = vec![
        "fly".into(),
        "--device=cpu".into(),
        format!("--particles={}", ion.display()),
        format!("--output={}", csv.display()),
        "--bounds=-2,-2,-2,4,2,2".into(),
        "--plane=1,0,0,1".into(),
        "--duration-us=10".into(),
    ];
    across_threads(|| {
        let summary = command(&args);
        assert_eq!(summary["arrivals"], 513);
        assert!(summary["arrival_tof_us"]["std"].as_f64().unwrap() > 0.0);
        let mut out = serde_json::to_vec(&summary).unwrap();
        out.extend(fs::read(&csv).unwrap());
        out
    });
}

fn pic_mesh() -> PicMesh {
    let n = 17;
    PicMesh::new(
        Grid {
            dims: [n; 3],
            spacing: [0.25; 3],
            geometry: Geometry::Planar,
        },
        [-2.0; 3],
        (0..n * n * n)
            .map(|i| {
                [i % n, i / n % n, i / (n * n)]
                    .into_iter()
                    .any(|c| c == 0 || c + 1 == n)
            })
            .collect(),
        Options {
            relative_tolerance: 1e-10,
            ..Default::default()
        },
    )
    .unwrap()
}

#[test]
fn repeatability_direct_coulomb_pic_and_coupled_flight() {
    // 257 live particles cross the direct-field and time-scale parallel branch.
    let p = particles(257);
    let positions: Vec<_> = p.iter().map(|p| p.initial.position_mm).collect();
    let weights: Vec<_> = (0..p.len()).map(|i| 0.5 + (i % 7) as f64 * 0.125).collect();
    let q = ChargeMode::Coulombic(1e-16).charges(&p, &weights).unwrap();
    let field = AnalyticField(|_, _| FieldSample::default());
    let options = EnsembleOptions {
        flight: FlightOptions {
            duration_us: 0.02,
            ..flight_options()
        },
        charge_mode: ChargeMode::Coulombic(1e-16),
        charge_weights: weights,
        space_charge_step_fraction: 1.0,
    };
    across_threads(|| {
        let mut out = Vec::new();
        for softening_mm in [0.0, 0.01] {
            let e = DirectCharge { softening_mm }.field(&positions, &q).unwrap();
            assert!(e.iter().flatten().any(|v| *v != 0.0));
            floats(&mut out, e.into_iter().flatten());
        }
        let mut mesh = pic_mesh();
        let e = mesh.field(&positions, &q, &vec![1.0; p.len()]).unwrap();
        floats(&mut out, e.into_iter().flatten());
        floats(&mut out, mesh.potentials().iter().copied());
        count(&mut out, mesh.last_cycles);
        floats(&mut out, [mesh.last_residual]);
        for mut charge in [
            SpaceCharge::Direct(DirectCharge { softening_mm: 0.01 }),
            SpaceCharge::Pic(Box::new(pic_mesh())),
        ] {
            let report = fly_ensemble(&field, &p, &options, &mut charge).unwrap();
            assert!(report.global_steps > 0);
            out.extend(ensemble(&report));
        }
        out
    });
}

#[test]
fn repeatability_beam_lines_rings_and_terminations() {
    let p = particles(257);
    let field = AnalyticField(|_, _| FieldSample::default());
    across_threads(|| {
        let mut out = Vec::new();
        for interaction in [BeamInteraction::Lines, BeamInteraction::Rings] {
            let report = fly_beam(
                &field,
                &p,
                &BeamOptions {
                    flight: FlightOptions {
                        duration_us: 0.02,
                        ..flight_options()
                    },
                    current_a: 1e-12,
                    softening_mm: 0.01,
                    interaction,
                    ..Default::default()
                },
            )
            .unwrap();
            out.extend(ensemble(&report.ensemble));
            // The terminal enum has no floating values.
            out.extend(format!("{:?}", report.terminations).bytes());
        }
        out
    });
}

#[test]
fn repeatability_seeded_hs1_and_sds_collisions() {
    let dir = tempfile::tempdir().unwrap();
    let table = dir.path().join("synthetic-mbmr.dat");
    // Synthetic monotone ICDFs test the runtime sampler, not SIMION's data.
    let values: String = (0..5)
        .flat_map(|table| {
            (0..1002).map(move |i| format!("{} ", i as f64 * (1.0 + table as f64 * 0.1)))
        })
        .collect();
    fs::write(&table, values).unwrap();
    let models = [
        CollisionModel::Hs1(Hs1 {
            mean_free_path_mm: Some(0.01),
            ..Default::default()
        }),
        CollisionModel::Sds(Sds {
            gas: Gas::default(),
            gas_diameter_nm: 0.366,
            ion_diameter_nm: Some(1.0),
            mobility_stp: Some(1.5),
            diffusion: true,
            statistics: DiffusionStatistics::from_path(&table).unwrap(),
        }),
    ];
    let p = particles(257);
    let field = AnalyticField(|_, _| FieldSample {
        electric_v_per_mm: [0.01, 0.0, 0.0],
        ..Default::default()
    });
    across_threads(|| {
        let mut out = Vec::new();
        for model in &models {
            let a = fly_many_collisional(&field, &p, &flight_options(), model, 734).unwrap();
            let b = fly_many_collisional(&field, &p, &flight_options(), model, 735).unwrap();
            assert_ne!(
                flights(&a),
                flights(&b),
                "collision randomness must actually be exercised"
            );
            out.extend(flights(&a));
        }
        out
    });
}

#[test]
fn repeatability_gpu_solver_flight_collision_and_pic() {
    use simrep_core::PotentialArray;
    use simrep_fly::{
        GridField, InstanceField,
        gpu::{Gpu as FlightGpu, TrajectorySampling},
        physics::{GpuPic, magnetic::MagneticField},
    };
    use simrep_solve::gpu::{Backends, Gpu};

    let Some(gpu) = pollster::block_on(Gpu::new(Backends::VULKAN)).unwrap() else {
        eprintln!("SKIP repeatability GPU: no Vulkan adapter (software adapters are accepted)");
        return;
    };
    eprintln!(
        "repeatability GPU: {} ({:?}), SHADER_F64={}",
        gpu.info.name,
        gpu.info.device_type,
        gpu.has_f64()
    );
    let (grid, fixed, initial) = laplace_case([17, 15, 13]);
    let cuts = CutCells::new(
        Grid {
            dims: [17, 17, 17],
            spacing: [0.125; 3],
            geometry: Geometry::Planar,
        },
        [-1.0; 3],
        &[
            Electrode::new(
                17.0,
                Sdf::Sphere {
                    center: [-0.55, 0.0, 0.0],
                    radius: 0.31,
                },
            ),
            Electrode::new(
                -3.0,
                Sdf::Sphere {
                    center: [0.55, 0.0, 0.0],
                    radius: 0.31,
                },
            ),
        ],
    )
    .unwrap();
    across_threads(|| {
        let mut out = Vec::new();
        for native in [false, true] {
            if native && !gpu.has_f64() {
                continue;
            }
            for cut in [false, true] {
                let mut potential = if cut {
                    cuts.values.clone()
                } else {
                    initial.clone()
                };
                let report = if cut {
                    gpu.solve_sdf_with_precision(&mut potential, &cuts, Options::default(), native)
                } else {
                    gpu.solve_with_precision(
                        &mut potential,
                        &fixed,
                        grid,
                        Options::default(),
                        native,
                    )
                }
                .unwrap();
                assert!(report.converged && report.iterations > 0);
                out.push(u8::from(report.gpu_f64));
                floats(&mut out, potential);
                residuals(
                    &mut out,
                    report.iterations,
                    report.levels,
                    &report.residual_history,
                );
            }
        }
        out
    });
    eprintln!("PASS repeatability GPU multigrid/cut-cell matrix");
    let flyer = pollster::block_on(FlightGpu::from_device(
        gpu.device().clone(),
        gpu.queue().clone(),
        gpu.info.clone(),
    ))
    .unwrap();
    let mut array = PotentialArray::new(9, 9, 9);
    array.dx_mm = 0.25;
    array.dy_mm = 0.25;
    array.dz_mm = 0.25;
    for (i, value) in array.potentials.iter_mut().enumerate() {
        *value = -0.01 * (i % 9) as f64 + 0.003 * (i / 9 % 9).pow(2) as f64;
    }
    let electric = InstanceField::new(
        GridField::new(array).unwrap(),
        [-1.0; 3],
        [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        1.0,
        None,
        None,
    )
    .unwrap();
    let magnetic = MagneticField {
        uniform_gauss: [0.0, 0.0, 20.0],
        ..Default::default()
    };
    let model = CollisionModel::Hs1(Hs1 {
        mean_free_path_mm: Some(0.01),
        ..Default::default()
    });
    let p = particles(257);
    let options = FlightOptions {
        sample_every: None,
        ..flight_options()
    };
    across_threads(|| {
        let mut out = Vec::new();
        for collision in [None, Some(&model)] {
            let mut resident = flyer
                .prepare_physics(&electric, &[], &magnetic, collision, 734)
                .unwrap();
            let sampling = TrajectorySampling {
                ion_indices: &[0, 128, 256],
                every: 2,
                max_samples_per_ion: 4096,
            };
            let first = resident.fly_sampled(&p, &options, &sampling).unwrap();
            let second = resident.fly_sampled(&p, &options, &sampling).unwrap();
            assert!(
                flights(&first.results) == flights(&second.results),
                "resident GPU flight must repeat"
            );
            assert!(
                first
                    .results
                    .iter()
                    .all(|r| r.splat_reason == SplatReason::TimeLimit)
            );
            if collision.is_some() {
                let mut other_seed = flyer
                    .prepare_physics(&electric, &[], &magnetic, collision, 735)
                    .unwrap();
                assert!(
                    flights(&first.results)
                        != flights(
                            &other_seed
                                .fly_sampled(&p, &options, &sampling)
                                .unwrap()
                                .results
                        )
                );
            }
            out.extend(flights(&first.results));
        }
        out
    });
    eprintln!("PASS repeatability GPU resident flight/HS1 matrix");
    if !gpu.has_f64() {
        eprintln!(
            "SKIP repeatability GPU PIC/native-f64 solve: SHADER_F64 unavailable; mixed solve and flight executed"
        );
        return;
    }
    across_threads(|| {
        let mut pic = GpuPic::new(
            Gpu::from_device(gpu.device().clone(), gpu.queue().clone(), gpu.info.clone()),
            pic_mesh(),
            p.len(),
        )
        .unwrap();
        let positions: Vec<_> = p.iter().map(|p| p.initial.position_mm).collect();
        let weights: Vec<_> = (0..p.len()).map(|i| 0.5 + (i % 7) as f64 * 0.125).collect();
        let q = ChargeMode::Coulombic(1e-16).charges(&p, &weights).unwrap();
        let fields = pic.field(&positions, &q, &vec![1.0; p.len()]).unwrap();
        let mut out = Vec::new();
        floats(&mut out, fields.into_iter().flatten());
        floats(&mut out, pic.potentials().iter().copied());
        count(&mut out, pic.last_cycles);
        floats(&mut out, [pic.last_residual]);
        let options = EnsembleOptions {
            flight: FlightOptions {
                duration_us: 0.02,
                ..flight_options()
            },
            charge_mode: ChargeMode::Coulombic(1e-16),
            charge_weights: weights,
            space_charge_step_fraction: 1.0,
        };
        let field = AnalyticField(|_, _| FieldSample::default());
        let report = fly_ensemble(
            &field,
            &p,
            &options,
            &mut SpaceCharge::GpuPic(Box::new(pic)),
        )
        .unwrap();
        out.extend(ensemble(&report));
        out
    });
}

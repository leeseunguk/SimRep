//! Reproduce geometry acceptance with real CLI outputs, or STL analytic tests.
//! cargo run --release -p simrep-cli --example geometry_pipeline -- einzel
//! cargo run --release -p simrep-cli --example geometry_pipeline -- mesh
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use simrep_geom::Electrode;
use simrep_io::{read_fast_adjust, read_pa};
use simrep_mesh::{Mesh, Part, Solid, read_mesh, write_mesh};
use simrep_solve::cutcell::CutCells;
use simrep_solve::mg::{self, Geometry, Grid, Options};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

fn command(binary: &Path, args: &[String]) -> Result<String> {
    let output = Command::new(binary).args(args).output()?;
    let text = String::from_utf8(output.stdout)?;
    print!("{text}");
    let diagnostics = String::from_utf8(output.stderr)?;
    eprint!("{diagnostics}");
    if !output.status.success() {
        return Err(format!("{args:?}: {}\n{}", output.status, diagnostics).into());
    }
    Ok(format!("{text}{diagnostics}"))
}

fn strings(args: &[&str]) -> Vec<String> {
    args.iter().map(|s| s.to_string()).collect()
}

fn bilinear(a: &simrep_core::PotentialArray, x: f64, r: f64) -> f64 {
    let x = (x / a.dx_mm).clamp(0.0, (a.nx - 1) as f64);
    let r = (r / a.dy_mm).clamp(0.0, (a.ny - 1) as f64);
    let ix = (x.floor() as usize).min(a.nx - 2);
    let ir = (r.floor() as usize).min(a.ny - 2);
    let tx = x - ix as f64;
    let tr = r - ir as f64;
    (1.0 - tr) * ((1.0 - tx) * a.potentials[ix + a.nx * ir] + tx * a.potentials[ix + 1 + a.nx * ir])
        + tr * ((1.0 - tx) * a.potentials[ix + a.nx * (ir + 1)]
            + tx * a.potentials[ix + 1 + a.nx * (ir + 1)])
}

fn csv_positions(path: &Path) -> Result<Vec<[f64; 3]>> {
    fs::read_to_string(path)?
        .lines()
        .skip(1)
        .map(|line| {
            let v: Vec<_> = line.split(',').collect();
            Ok([v[2].parse()?, v[3].parse()?, v[4].parse()?])
        })
        .collect()
}

fn endpoint_error(candidate: &[[f64; 3]], reference: &[[f64; 3]]) -> (f64, f64) {
    assert_eq!(candidate.len(), reference.len());
    let mut sum = 0.0;
    let mut max: f64 = 0.0;
    for (a, b) in candidate.iter().zip(reference) {
        let d2: f64 = (0..3).map(|i| (a[i] - b[i]).powi(2)).sum();
        sum += d2;
        max = max.max(d2.sqrt());
    }
    ((sum / candidate.len() as f64).sqrt(), max)
}

fn einzel(out: &Path) -> Result<()> {
    let binary = std::env::var_os("SIMREP_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/release/simrep"));
    let references = std::env::var_os("SIMREP_REFERENCE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/fly-real"));
    let input = "/mnt/c/Program Files/SIMION-2024/examples/einzel/einzel.gem";
    let ions = "target/simrep-oracle/_real_einzel/subset.ion";
    let reference4 = read_pa(references.join("grid-4.pa"))?.array;
    let reference8 = read_pa(references.join("grid-8.pa"))?.array;
    let endpoints = csv_positions(&references.join("grid-8.csv"))?;
    let mut results = Vec::new();
    for (name, dims, spacing) in [
        ("aligned", "91,20,1", "1,1,1".to_string()),
        // Preserve [0,90] axial and [0,19] radial domain exactly.
        ("nonaligned", "92,21,1", format!("{},0.95,1", 90.0 / 91.0)),
    ] {
        let cut = out.join(format!("{name}-cut"));
        let stair = out.join(format!("{name}-stair"));
        let cut_text = command(
            &binary,
            &strings(&[
                "refine-geometry",
                input,
                "--grid",
                dims,
                "--spacing",
                &spacing,
                "--electrode",
                "1=1:0",
                "--electrode",
                "2=2:110",
                "--electrode",
                "3=3:0",
                "--tolerance",
                "1e-12",
                "--out",
                cut.to_str().unwrap(),
            ]),
        )?;
        fs::copy(cut.with_extension("pa#"), stair.with_extension("pa#"))?;
        let stair_text = command(
            &binary,
            &strings(&[
                "refine",
                stair.with_extension("pa#").to_str().unwrap(),
                "--tolerance",
                "1e-12",
            ]),
        )?;
        let cut_array =
            read_fast_adjust(cut.with_extension("pa0"), &[(1, 0.0), (2, 110.0), (3, 0.0)])?;
        let stair_array = read_fast_adjust(
            stair.with_extension("pa0"),
            &[(1, 0.0), (2, 110.0), (3, 0.0)],
        )?;
        assert_eq!(cut_array.electrodes, stair_array.electrodes);
        let nodal_difference = cut_array
            .potentials
            .iter()
            .zip(&stair_array.potentials)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0, f64::max);
        if name == "aligned" {
            assert!(
                nodal_difference < 1e-6,
                "aligned fields differ by {nodal_difference} V"
            );
        }
        let mut entries = Vec::new();
        for (method, stem) in [("cut", &cut), ("stair", &stair)] {
            let a = read_fast_adjust(
                stem.with_extension("pa0"),
                &[(1, 0.0), (2, 110.0), (3, 0.0)],
            )?;
            let mut errors = Vec::new();
            for reference in [&reference4, &reference8] {
                let mut sum = 0.0;
                let mut max: f64 = 0.0;
                let mut count = 0;
                for r in 0..a.ny {
                    for x in 0..a.nx {
                        let i = x + a.nx * r;
                        if a.electrodes[i] {
                            continue;
                        }
                        let d = a.potentials[i]
                            - bilinear(reference, x as f64 * a.dx_mm, r as f64 * a.dy_mm);
                        sum += d * d;
                        max = max.max(d.abs());
                        count += 1;
                    }
                }
                errors.push(serde_json::json!({"rms_v":(sum/count as f64).sqrt(),"max_v":max}));
            }
            let csv = stem.with_extension("csv");
            command(
                &binary,
                &strings(&[
                    "fly",
                    "--pa",
                    stem.with_extension("pa0").to_str().unwrap(),
                    "--particles",
                    ions,
                    "--voltage",
                    "1=0,2=110,3=0",
                    "--bounds=-101,-19,-19,291,19,19",
                    "--tolerance",
                    "1e-11",
                    "--max-step-us",
                    "0.05",
                    "--output",
                    csv.to_str().unwrap(),
                ]),
            )?;
            let (rms, max) = endpoint_error(&csv_positions(&csv)?, &endpoints);
            entries.push(
                serde_json::json!({"method":method, "potential_vs_4x":errors[0],
                "potential_vs_8x":errors[1], "endpoint_rms_mm":rms, "endpoint_max_mm":max}),
            );
        }
        if name == "nonaligned" {
            assert!(
                entries[0]["potential_vs_8x"]["rms_v"].as_f64().unwrap()
                    < entries[1]["potential_vs_8x"]["rms_v"].as_f64().unwrap()
            );
            assert!(
                entries[0]["endpoint_rms_mm"].as_f64().unwrap()
                    < entries[1]["endpoint_rms_mm"].as_f64().unwrap()
            );
        }
        results.push(
            serde_json::json!({"case":name,"grid":dims,"spacing":spacing,
            "results":entries,"max_cut_stair_difference_v":nodal_difference,
            "cut_stdout":cut_text,"stair_stdout":stair_text}),
        );
    }
    let result = serde_json::to_string_pretty(&results)?;
    println!("{result}");
    fs::write(out.join("einzel-results.json"), result)?;
    Ok(())
}

fn sphere(radius: f64, slices: usize, rings: usize) -> Part {
    let mut p = Part {
        name: "sphere".into(),
        ..Default::default()
    };
    p.vertices.push([0.0, 0.0, radius]);
    for j in 1..rings {
        let phi = std::f64::consts::PI * j as f64 / rings as f64;
        for i in 0..slices {
            let theta = std::f64::consts::TAU * i as f64 / slices as f64;
            p.vertices.push([
                radius * phi.sin() * theta.cos(),
                radius * phi.sin() * theta.sin(),
                radius * phi.cos(),
            ]);
        }
    }
    let bottom = p.vertices.len();
    p.vertices.push([0.0, 0.0, -radius]);
    for i in 0..slices {
        let next = (i + 1) % slices;
        p.triangles.push([0, 1 + i, 1 + next]);
        for j in 0..rings - 2 {
            let a = 1 + j * slices + i;
            let b = 1 + j * slices + next;
            p.triangles.push([a, a + slices, b + slices]);
            p.triangles.push([a, b + slices, b]);
        }
        p.triangles.push([
            bottom,
            1 + (rings - 2) * slices + next,
            1 + (rings - 2) * slices + i,
        ]);
    }
    p
}

fn cylinder(radius: f64, slices: usize) -> Part {
    let mut p = Part {
        name: "cylinder".into(),
        ..Default::default()
    };
    for z in [-2.0, 2.0] {
        for i in 0..slices {
            let theta = std::f64::consts::TAU * i as f64 / slices as f64;
            p.vertices
                .push([radius * theta.cos(), radius * theta.sin(), z]);
        }
    }
    p.vertices.extend([[0.0, 0.0, -2.0], [0.0, 0.0, 2.0]]);
    for i in 0..slices {
        let j = (i + 1) % slices;
        p.triangles.extend([
            [i, j, j + slices],
            [i, j + slices, i + slices],
            [2 * slices, j, i],
            [2 * slices + 1, i + slices, j + slices],
        ]);
    }
    p
}

fn mesh(out: &Path) -> Result<()> {
    let mut results = Vec::new();
    for spherical in [true, false] {
        let name = if spherical { "sphere" } else { "cylinder" };
        let mut solids = Vec::new();
        for (suffix, radius) in [("inner", 0.31), ("outer", 0.83)] {
            let part = if spherical {
                sphere(radius, 192, 96)
            } else {
                cylinder(radius, 512)
            };
            let path = out.join(format!("{name}-{suffix}.stl"));
            write_mesh(&path, &Mesh { parts: vec![part] })?;
            let imported = read_mesh(path, Default::default())?;
            assert_eq!(imported.parts.len(), 1);
            solids.push(Arc::new(Solid::new(&imported.parts[0])?));
        }
        let inner = Arc::clone(&solids[0]);
        let outer = Arc::clone(&solids[1]);
        // The outer STL represents the cavity surface; the conductor is its
        // complement. End caps lie outside the cylinder's sampled z=0 plane.
        let electrodes = [
            Electrode::new(1.0, move |p| inner.signed_distance(p)),
            Electrode::new(0.0, move |p| -outer.signed_distance(p)),
        ];
        let grid = Grid {
            dims: [65, 65, if spherical { 65 } else { 1 }],
            spacing: [1.0 / 32.0; 3],
            geometry: Geometry::Planar,
        };
        let origin = [-1.0, -1.0, if spherical { -1.0 } else { 0.0 }];
        let cuts = CutCells::new(grid, origin, &electrodes)?;
        let mut methods = Vec::new();
        for staircase in [false, true] {
            // Physical node membership, without positive-distance snaps, just
            // as the CLI's exported PA# consumed by `simrep refine`.
            let fixed: Vec<_> = (0..cuts.values.len())
                .map(|i| {
                    let p = [
                        origin[0] + (i % 65) as f64 / 32.0,
                        origin[1] + (i / 65 % 65) as f64 / 32.0,
                        origin[2] + (i / (65 * 65)) as f64 / 32.0,
                    ];
                    electrodes
                        .iter()
                        .any(|e| e.shape.distance(p) <= 1e-9 / 32.0)
                })
                .collect();
            let mut values = cuts.values.clone();
            let options = Options {
                relative_tolerance: 1e-12,
                ..Default::default()
            };
            let report = if staircase {
                mg::solve(&mut values, &fixed, grid, options)?
            } else {
                cuts.solve(&mut values, options)?
            };
            assert!(report.converged);
            let mut error: f64 = 0.0;
            for (i, &v) in values.iter().enumerate() {
                let p = [
                    origin[0] + (i % 65) as f64 / 32.0,
                    origin[1] + (i / 65 % 65) as f64 / 32.0,
                    origin[2] + (i / (65 * 65)) as f64 / 32.0,
                ];
                let r = p
                    .iter()
                    .take(if spherical { 3 } else { 2 })
                    .map(|x| x * x)
                    .sum::<f64>()
                    .sqrt();
                if !(0.31..0.83).contains(&r) {
                    continue;
                }
                let exact = if spherical {
                    (1.0 / r - 1.0 / 0.83) / (1.0 / 0.31 - 1.0 / 0.83)
                } else {
                    (0.83 / r).ln() / (0.83_f64 / 0.31).ln()
                };
                error = error.max((v - exact).abs());
            }
            methods.push(
                serde_json::json!({"method":if staircase {"stair"} else {"cut"},
                "max_error_v":error,"compute_ms":report.timings.solve.as_secs_f64()*1000.0}),
            );
        }
        assert!(
            methods[0]["max_error_v"].as_f64().unwrap() * 10.0
                < methods[1]["max_error_v"].as_f64().unwrap()
        );
        results.push(serde_json::json!({"geometry":name,"results":methods}));
    }
    let result = serde_json::to_string_pretty(&results)?;
    println!("{result}");
    fs::write(out.join("mesh-results.json"), result)?;
    Ok(())
}

fn gpu(out: &Path) -> Result<()> {
    let cpu = read_pa(out.join("nonaligned-cut.pa0"))?.array;
    let mut results = Vec::new();
    for name in ["gpu-cut", "dx12-cut"] {
        let native = read_pa(format!("target/geometry-pipeline/{name}.pa0"))?.array;
        assert_eq!(cpu.electrodes, native.electrodes);
        let error = cpu
            .potentials
            .iter()
            .zip(&native.potentials)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0, f64::max);
        assert!(error < 1e-7, "{name} differs from CPU by {error} V");
        results.push(serde_json::json!({"backend":name,"max_cpu_difference_v":error}));
    }
    let text = serde_json::to_string_pretty(&results)?;
    println!("{text}");
    fs::write(out.join("gpu-results.json"), text)?;
    Ok(())
}

/// Collision-only acceptance: refine generated STL and compare to the source
/// analytic SDF, keeping the PA field and nodal mask fixed between flyers.
fn surface(out: &Path) -> Result<()> {
    use simrep_fly::surface::{SurfaceModel, SurfaceSamples};
    use simrep_fly::{Field, GridField, InstanceField};
    use simrep_geom::{Sdf, SignedDistance};
    use std::time::Instant;
    let binary = std::env::var_os("SIMREP_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/release/simrep"));
    let mut results = Vec::new();
    for spherical in [true, false] {
        let name = if spherical { "sphere" } else { "cylinder" };
        let shape = if spherical {
            Sdf::Sphere {
                center: [0.0; 3],
                radius: 0.63,
            }
        } else {
            Sdf::Cylinder {
                center: [0.0; 3],
                radius: 0.63,
                axis: 2,
            }
        };
        let part = if spherical {
            sphere(0.63, 384, 192)
        } else {
            cylinder(0.63, 2048)
        };
        let mesh = out.join(format!("collision-{name}.stl"));
        write_mesh(&mesh, &Mesh { parts: vec![part] })?;
        let from = [-0.95, 0.173, 0.114];
        let to = [0.95, from[1], from[2]];
        let exact = SurfaceModel::from_sdfs(vec![(17, Arc::new(shape.clone()))], 1e-12)?;
        let exact_t = exact.first_hit(from, to).unwrap().0;
        let mut entries = Vec::new();
        let mut analytic_errors = Vec::new();
        let mut mesh_errors = Vec::new();
        for factor in [1, 2, 4, 8] {
            let stem = out.join(format!("collision-{name}-{factor}"));
            command(
                &binary,
                &strings(&[
                    "refine-geometry",
                    mesh.to_str().unwrap(),
                    "--grid",
                    "11,11,11",
                    "--spacing",
                    "0.2,0.2,0.2",
                    "--origin=-1,-1,-1",
                    "--electrode",
                    "0=17:0",
                    "--sidecar-dense-sdf",
                    "--surface-supersampling",
                    &factor.to_string(),
                    "--out",
                    stem.to_str().unwrap(),
                ]),
            )?;
            let model =
                SurfaceModel::from_reader(fs::File::open(stem.with_extension("cuts.json"))?)
                    .map_err(|e| e.to_string())?;
            let mesh_error = (model.first_hit(from, to).unwrap().0 - exact_t).abs() * 1.9;
            mesh_errors.push(mesh_error);
            // The same sidecar layout, sampled directly from the analytic SDF.
            let sample = SurfaceSamples::sample(
                17,
                &shape,
                [10 * factor + 1; 3],
                [-1.0; 3],
                [0.2 / factor as f64; 3],
            )?;
            let mut json: serde_json::Value =
                serde_json::from_reader(fs::File::open(stem.with_extension("cuts.json"))?)?;
            // Deliberately exercise the v2 fallback, independently of the v3
            // triangle source retained in the generated sidecar.
            json["version"] = serde_json::json!(2);
            json["sampled_sdfs"] = serde_json::json!([sample]);
            let analytic = SurfaceModel::from_reader(json.to_string().as_bytes())
                .map_err(|e| e.to_string())?;
            let analytic_error = (analytic.first_hit(from, to).unwrap().0 - exact_t).abs() * 1.9;
            analytic_errors.push(analytic_error);
            let array = read_pa(stem.with_extension("pa0"))?.array;
            let local_from = from.map(|p| p + 1.0);
            let local_to = to.map(|p| p + 1.0);
            let field = GridField::new(array)?.with_nearest_node_collisions();
            let stair_error =
                (field.intersection(local_from, local_to).unwrap().0 - exact_t).abs() * 1.9;
            let placed = InstanceField::new(
                field,
                [-1.0; 3],
                [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
                1.0,
                None,
                None,
            )?;
            let mut timings = Vec::new();
            for surface in [false, true] {
                let field = if surface {
                    placed.clone().with_surface(Arc::new(
                        SurfaceModel::from_reader(fs::File::open(
                            stem.with_extension("cuts.json"),
                        )?)
                        .map_err(|e| e.to_string())?,
                    ))?
                } else {
                    placed.clone()
                };
                let start = Instant::now();
                for _ in 0..10_000 {
                    std::hint::black_box(
                        field.intersection(std::hint::black_box(from), std::hint::black_box(to)),
                    );
                }
                timings.push(start.elapsed().as_secs_f64() * 1e3);
            }
            assert!(analytic_error < 0.1 * 0.2);
            assert!(mesh_error < stair_error);
            assert!(shape.distance(from) > 0.0 && shape.distance(to) > 0.0);
            entries.push(serde_json::json!({
                "supersampling":factor,"analytic_hit_error_mm":analytic_error,
                "stl_hit_error_mm":mesh_error,"staircase_hit_error_mm":stair_error,
                "staircase_10000_segments_ms":timings[0],"surface_10000_segments_ms":timings[1],
            }));
        }
        assert!(
            analytic_errors[0] > 20.0 * analytic_errors[3],
            "{analytic_errors:?}"
        );
        assert!(mesh_errors.iter().all(|e| *e < 5e-5), "{mesh_errors:?}");
        assert!(
            mesh_errors.windows(2).all(|e| e[0] == e[1]),
            "{mesh_errors:?}"
        );
        results.push(serde_json::json!({"geometry":name,"pa_spacing_mm":0.2,
            "staircase_rule":"nearest-node voxels","results":entries}));
    }
    let text = serde_json::to_string_pretty(&results)?;
    println!("{text}");
    fs::write(out.join("surface-results.json"), text)?;
    Ok(())
}

/// Compare the non-aligned lens to independently refined physical-surface
/// references rather than retaining fine-grid nearest-node splat offsets.
fn surface_einzel(out: &Path) -> Result<()> {
    let binary = std::env::var_os("SIMREP_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/release/simrep"));
    let input = "/mnt/c/Program Files/SIMION-2024/examples/einzel/einzel.gem";
    let original_ions = "target/simrep-oracle/_real_einzel/subset.ion";
    let original_count = simrep_io::read_particles(original_ions)?.len();
    let mut beam = fs::read_to_string(original_ions)?;
    beam.push('\n');
    // The supplied beam clears every electrode. Add neutral radial impacts
    // along the physical lens walls to isolate collision error from the field.
    for axial in [5.0, 14.0, 24.0, 33.0, 42.0, 52.0, 62.0, 72.0, 86.0] {
        for radial in [15.3, 16.2, 17.0, 17.4] {
            beam.push_str(&format!("0,100,0,{axial},{radial},0,0,90,1,1\n"));
        }
    }
    let ions = out.join("surface-einzel-impact.ion");
    fs::write(&ions, beam)?;
    let mut endpoints = Vec::new();
    let mut times = Vec::new();
    for (name, dims, spacing, no_cuts) in [
        (
            "surface-reference4",
            "361,77,1",
            "0.25,0.25,1".to_string(),
            false,
        ),
        (
            "surface-reference8",
            "721,153,1",
            "0.125,0.125,1".to_string(),
            false,
        ),
        (
            "surface-nonaligned",
            "92,21,1",
            format!("{},0.95,1", 90.0 / 91.0),
            false,
        ),
        (
            "surface-nonaligned",
            "92,21,1",
            format!("{},0.95,1", 90.0 / 91.0),
            true,
        ),
    ] {
        let stem = out.join(name);
        if !no_cuts {
            command(
                &binary,
                &strings(&[
                    "refine-geometry",
                    input,
                    "--grid",
                    dims,
                    "--spacing",
                    &spacing,
                    "--electrode",
                    "1=1:0",
                    "--electrode",
                    "2=2:110",
                    "--electrode",
                    "3=3:0",
                    "--tolerance",
                    "1e-12",
                    "--out",
                    stem.to_str().unwrap(),
                ]),
            )?;
        }
        let csv = out.join(format!(
            "{name}{}.csv",
            if no_cuts { "-nocuts" } else { "" }
        ));
        let mut args = strings(&[
            "fly",
            "--device",
            "cpu",
            "--pa",
            stem.with_extension("pa0").to_str().unwrap(),
            "--particles",
            ions.to_str().unwrap(),
            "--voltage",
            "1=0,2=110,3=0",
            "--bounds=-101,-19,-19,291,19,19",
            "--tolerance",
            "1e-11",
            "--max-step-us",
            "0.05",
            "--output",
            csv.to_str().unwrap(),
        ]);
        if no_cuts {
            args.push("--no-cuts".into());
            args.push("--nearest-node-collisions".into());
        }
        let text = command(&binary, &args)?;
        let fly_s = text
            .split("fly_s: ")
            .nth(1)
            .ok_or("missing fly compute clock")?
            .split(',')
            .next()
            .unwrap()
            .parse::<f64>()?;
        times.push(fly_s);
        endpoints.push(csv_positions(&csv)?);
    }
    let convergence = endpoint_error(&endpoints[0], &endpoints[1]);
    let surface = endpoint_error(&endpoints[2], &endpoints[1]);
    let staircase = endpoint_error(&endpoints[3], &endpoints[1]);
    let original_beam = endpoint_error(
        &endpoints[2][..original_count],
        &endpoints[1][..original_count],
    );
    let impact_surface = endpoint_error(
        &endpoints[2][original_count..],
        &endpoints[1][original_count..],
    );
    let impact_staircase = endpoint_error(
        &endpoints[3][original_count..],
        &endpoints[1][original_count..],
    );
    assert!(
        surface.0 < staircase.0,
        "surface {surface:?}, staircase {staircase:?}"
    );
    assert!(
        convergence.0 < surface.0,
        "reference {convergence:?}, surface {surface:?}"
    );
    let result = serde_json::json!({
        "staircase_rule":"nearest-node voxels",
        "reference4_vs_8_rms_mm":convergence.0,"reference4_vs_8_max_mm":convergence.1,
        "original_beam_ions":original_count,"original_beam_surface_vs_reference8_rms_mm":original_beam.0,
        "neutral_wall_impacts":endpoints[1].len()-original_count,
        "wall_impact_surface_vs_reference8_rms_mm":impact_surface.0,
        "wall_impact_staircase_vs_reference8_rms_mm":impact_staircase.0,
        "surface_vs_reference8_rms_mm":surface.0,"surface_vs_reference8_max_mm":surface.1,
        "same_field_staircase_vs_reference8_rms_mm":staircase.0,
        "same_field_staircase_vs_reference8_max_mm":staircase.1,
        "surface_fly_s":times[2],"staircase_fly_s":times[3],"compute_overhead":times[2]/times[3]
    });
    let text = serde_json::to_string_pretty(&result)?;
    println!("{text}");
    fs::write(out.join("surface-einzel-results.json"), text)?;
    Ok(())
}

fn main() -> Result<()> {
    let out = Path::new("target/geometry-pipeline");
    fs::create_dir_all(out)?;
    match std::env::args().nth(1).as_deref() {
        Some("mesh") => mesh(out),
        Some("einzel") => einzel(out),
        Some("gpu") => gpu(out),
        Some("surface") => surface(out),
        Some("surface-einzel") => surface_einzel(out),
        _ => Err("choose mesh, einzel, gpu, surface, or surface-einzel".into()),
    }
}

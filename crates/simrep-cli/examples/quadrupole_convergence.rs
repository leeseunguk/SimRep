//! Memory-bounded quadrupole reference. X is reflected about its midplane.
//! cargo run --release -p simrep-cli --example quadrupole_convergence -- physical 4
use std::error::Error;
use std::fs::{File, create_dir_all};
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use simrep_core::{MirrorFlags, PotentialArray};
use simrep_fly::{
    Field, FieldSample, FlightOptions, GridField, InstanceField, Particle, Reconstruction,
    SplatReason, fly_many, surface::SurfaceModel,
};
use simrep_geom::{Electrode, SampledSdf, Sdf, SignedDistance};
use simrep_io::{PaFile, read_fast_adjust, read_pa, read_particles, write_pa};
use simrep_solve::cutcell::{CutCells, surface_fraction};
use simrep_solve::mg::{Geometry, Grid, Options};

const INPUT: &str = "/mnt/c/Users/lsu10/simrep-oracle/_quad_gpu";
const OUTPUT: &str = "target/quadrupole-convergence";
type Result<T> = std::result::Result<T, Box<dyn Error>>;
type Bounds = [[f64; 3]; 2];

fn electrode_boxes(a: &PotentialArray, node_cells: bool) -> Vec<Bounds> {
    let mut boxes = Vec::new();
    let offset = if node_cells { 0.0 } else { 0.25 };
    for (voltage, x, xlo, xhi) in [
        (2.5, 60, 12.0 - offset, 48.0 + offset),
        (-2.5, 60, 12.0 - offset, 48.0 + offset),
        (0.0, 12, 6.0 - offset, 7.0 + offset),
        (0.0, 106, 53.0 - offset, 54.0 + offset),
    ] {
        let mut strips: Vec<[f64; 4]> = Vec::new();
        let y_end = a.ny - usize::from(node_cells);
        for z in 0..a.nz - usize::from(node_cells) {
            let mut y = 0;
            while y < y_end {
                let owns = |y: usize| {
                    let corners = if node_cells { 2 } else { 1 };
                    (0..corners).all(|dz| {
                        (0..corners).all(|dy| {
                            let i = ((z + dz) * a.ny + y + dy) * a.nx + x;
                            a.electrodes[i] && (a.potentials[i] - voltage).abs() < 1e-8
                        })
                    })
                };
                if !owns(y) {
                    y += 1;
                    continue;
                }
                let first = y;
                while y < y_end && owns(y) {
                    y += 1;
                }
                let lo = first as f64 * 0.5 - offset;
                let hi = y as f64 * 0.5 - offset;
                let zlo = z as f64 * 0.5 - offset;
                if let Some(previous) = strips
                    .iter_mut()
                    .find(|r| r[0] == lo && r[2] == hi && r[3] == zlo)
                {
                    previous[3] += 0.5;
                } else {
                    strips.push([lo, zlo, hi, zlo + 0.5]);
                }
            }
        }
        for r in strips {
            for sy in [-1.0_f64, 1.0] {
                for sz in [-1.0_f64, 1.0] {
                    boxes.push([
                        [xlo, (sy * r[0]).min(sy * r[2]), (sz * r[1]).min(sz * r[3])],
                        [xhi, (sy * r[0]).max(sy * r[2]), (sz * r[1]).max(sz * r[3])],
                    ]);
                }
            }
        }
    }
    boxes
}

fn segment_box_distance_squared(from: [f64; 3], to: [f64; 3], bounds: Bounds) -> f64 {
    let d: [f64; 3] = std::array::from_fn(|a| to[a] - from[a]);
    let mut cuts = [0.0; 8];
    cuts[1] = 1.0;
    let mut count = 2;
    for a in 0..3 {
        if d[a] != 0.0 {
            for side in bounds {
                let t = (side[a] - from[a]) / d[a];
                if t > 0.0 && t < 1.0 {
                    cuts[count] = t;
                    count += 1;
                }
            }
        }
    }
    cuts[..count].sort_unstable_by(f64::total_cmp);
    let mut minimum = f64::INFINITY;
    for interval in cuts[..count].windows(2) {
        let middle = (interval[0] + interval[1]) * 0.5;
        let mut constant = [0.0; 3];
        let mut slope = [0.0; 3];
        for a in 0..3 {
            let p = from[a] + middle * d[a];
            if p < bounds[0][a] || p > bounds[1][a] {
                let edge = if p < bounds[0][a] {
                    bounds[0][a]
                } else {
                    bounds[1][a]
                };
                constant[a] = from[a] - edge;
                slope[a] = d[a];
            }
        }
        let denominator: f64 = slope.iter().map(|v| v * v).sum();
        let t = if denominator == 0.0 {
            middle
        } else {
            (-constant.iter().zip(slope).map(|(c, s)| c * s).sum::<f64>() / denominator)
                .clamp(interval[0], interval[1])
        };
        minimum = minimum.min(
            constant
                .iter()
                .zip(slope)
                .map(|(c, s)| (c + t * s).powi(2))
                .sum(),
        );
    }
    minimum
}

fn shapes() -> Vec<Electrode> {
    let rod = |y, z| {
        Sdf::Cylinder {
            center: [0.0, y, z],
            radius: 5.75,
            axis: 0,
        }
        .intersection(Sdf::Box {
            center: [0.0; 3],
            half_extent: [18.0, 100.0, 100.0],
        })
    };
    let plate = Sdf::Box {
        center: [23.5, 0.0, 0.0],
        half_extent: [0.5, 100.0, 100.0],
    }
    .difference(Sdf::Cylinder {
        center: [0.0; 3],
        radius: 4.0,
        axis: 0,
    });
    vec![
        Electrode::new(2.5, rod(10.75, 0.0)),
        Electrode::new(-2.5, rod(0.0, 10.75)),
        Electrode::new(0.0, plate),
    ]
}

fn staircase_shapes(original: &PotentialArray, factor: usize, node_cells: bool) -> Vec<Electrode> {
    let offset = if node_cells { 0.0 } else { 0.25 };
    [
        (2.5, 60, -18.0 - offset, 18.0 + offset),
        (-2.5, 60, -18.0 - offset, 18.0 + offset),
        (0.0, 12, 23.0 - offset, 24.0 + offset),
    ]
    .into_iter()
    .map(|(voltage, x, lo, hi): (f64, usize, f64, f64)| {
        let mut rectangles = Vec::new();
        let y_end = original.ny - usize::from(node_cells);
        for z in 0..original.nz - usize::from(node_cells) {
            let mut y = 0;
            while y < y_end {
                let owns = |y: usize| {
                    let corners = if node_cells { 2 } else { 1 };
                    (0..corners).all(|dz| {
                        (0..corners).all(|dy| {
                            let i = ((z + dz) * original.ny + y + dy) * original.nx + x;
                            original.electrodes[i]
                                && (original.potentials[i] - voltage).abs() < 1e-8
                        })
                    })
                };
                if !owns(y) {
                    y += 1;
                    continue;
                }
                let first = y;
                while y < y_end && owns(y) {
                    y += 1;
                }
                rectangles.push([
                    first as f64 * 0.5 - offset,
                    z as f64 * 0.5 - offset,
                    y as f64 * 0.5 - offset,
                    z as f64 * 0.5 + 0.5 - offset,
                ]);
            }
        }
        let n = 36 * factor + 1;
        let h = 0.5 / factor as f64;
        let samples = (0..n * n)
            .map(|i| {
                let p = [(i % n) as f64 * h, (i / n) as f64 * h];
                rectangles
                    .iter()
                    .map(|r| {
                        let d = [
                            (p[0] - (r[0] + r[2]) * 0.5).abs() - (r[2] - r[0]) * 0.5,
                            (p[1] - (r[1] + r[3]) * 0.5).abs() - (r[3] - r[1]) * 0.5,
                        ];
                        d[0].max(0.0).hypot(d[1].max(0.0)) + d[0].max(d[1]).min(0.0)
                    })
                    .fold(f64::INFINITY, f64::min)
            })
            .collect();
        let section = SampledSdf::new([n, n, 1], [0.0; 3], [h, h, 1.0], samples).unwrap();
        Electrode::new(voltage, move |p: [f64; 3]| {
            let axial = (p[0] - (lo + hi) * 0.5).abs() - (hi - lo) * 0.5;
            axial.max(section.distance([p[1], p[2], 0.0]) / 2.0_f64.sqrt())
        })
    })
    .collect()
}

struct ReferenceField {
    grid: GridField,
    electrodes: Vec<Electrode>,
    collision: GridField,
    surface_collisions: bool,
}

impl Field for ReferenceField {
    fn sample(&self, p: [f64; 3], time: f64) -> FieldSample {
        let extent = (self.grid.array().ny - 1) as f64 * self.grid.array().dy_mm;
        assert!(
            extent >= 18.0 || p[1].abs().max(p[2].abs()) <= extent,
            "trajectory left the locally refined reference"
        );
        self.grid.sample([p[0] - 30.0, p[1], p[2]], time)
    }

    fn status(&self, p: [f64; 3]) -> Option<SplatReason> {
        if !(0.0..=60.0).contains(&p[0]) || p[1].abs() > 18.0 || p[2].abs() > 18.0 {
            return Some(SplatReason::OutsideGrid);
        }
        if !self.surface_collisions {
            return self.collision.status(p);
        }
        let q = [(p[0] - 30.0).abs(), p[1].abs(), p[2].abs()];
        self.electrodes
            .iter()
            .position(|e| e.shape.distance(q) <= 0.0)
            .map(|index| SplatReason::Electrode { index })
    }

    fn intersection(&self, from: [f64; 3], to: [f64; 3]) -> Option<(f64, SplatReason)> {
        let mut event = self
            .collision
            .intersection(from, to)
            .filter(|(_, reason)| !self.surface_collisions || *reason == SplatReason::OutsideGrid);
        if self.surface_collisions {
            for (index, electrode) in self.electrodes.iter().enumerate() {
                let shape = |p: [f64; 3]| {
                    electrode
                        .shape
                        .distance([(p[0] - 30.0).abs(), p[1].abs(), p[2].abs()])
                };
                if let Some(t) = surface_fraction(&shape, from, to, 1e-10)
                    && event.is_none_or(|(old, _)| t < old)
                {
                    event = Some((t, SplatReason::Electrode { index }));
                }
            }
        }
        event
    }
}

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let mode = args.first().map(String::as_str).unwrap_or("original");
    let factor = args.get(1).map_or(Ok(1), |v| v.parse::<usize>())?;
    let local_outer = match mode {
        "node-local-4" => Some(4),
        "node-local-8" => Some(8),
        _ => None,
    };
    if !([1, 2, 4, 8].contains(&factor) || factor == 16 && local_outer.is_some()) {
        return Err("reference factor must be 1, 2, 4 or 8".into());
    }
    // Fine references must not allocate 6^3 coefficients for every cell.
    let global = factor > 1 || args.iter().any(|s| s == "--global");
    let trajectories = args.iter().any(|s| s == "--clearance");
    let timing = args.iter().any(|s| s == "--timing");
    let nearest_node = args.iter().any(|s| s == "--nearest-node-collisions");
    let ghost_fill = args.iter().any(|s| s == "--ghost-fill");
    create_dir_all(OUTPUT)?;
    let original = read_fast_adjust(
        PathBuf::from(INPUT).join("quadrupole.pa0"),
        &[(1, 2.5), (2, -2.5), (3, 0.0)],
    )?;
    let boxes = if trajectories {
        electrode_boxes(&original, false)
    } else {
        Vec::new()
    };
    let cell_boxes = if trajectories {
        electrode_boxes(&original, true)
    } else {
        Vec::new()
    };
    if trajectories {
        for (i, &metal) in original.electrodes.iter().enumerate() {
            let p = [
                (i % original.nx) as f64 * 0.5,
                (i / original.nx % original.ny) as f64 * 0.5,
                (i / (original.nx * original.ny)) as f64 * 0.5,
            ];
            assert_eq!(
                metal,
                boxes
                    .iter()
                    .any(|b| { (0..3).all(|a| (b[0][a]..=b[1][a]).contains(&p[a])) }),
                "voxel box mismatch at {p:?}"
            );
        }
        println!(
            "exact clearance: {} merged electrode boxes; all PA nodes match",
            boxes.len()
        );
    }
    let particles = read_particles(PathBuf::from(INPUT).join("particles.ion"))?
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
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let method = if global { "global" } else { "boundary" };
    let mut label = method.to_string();
    if nearest_node {
        label.push_str("-nearest");
    }
    if timing {
        label.push_str("-timing");
    }
    if ghost_fill {
        label.push_str("-ghost");
    }
    let mut output = BufWriter::new(File::create(
        PathBuf::from(OUTPUT).join(format!("{mode}-{factor}-{label}.csv")),
    )?);
    let tolerance = if timing { 1e-8 } else { 1e-11 };
    let options = FlightOptions {
        relative_tolerance: tolerance,
        position_tolerance_mm: tolerance,
        velocity_tolerance_mm_per_us: tolerance,
        max_step_us: if timing { 0.1 } else { 0.025 },
        duration_us: 100.0,
        relativistic: true,
        sample_every: trajectories.then_some(1),
        ..Default::default()
    };
    let start = Instant::now();
    let grid_field = |array| -> std::result::Result<GridField, simrep_fly::FlyError> {
        let field = GridField::with_reconstruction(
            array,
            if global {
                Reconstruction::GlobalCubic
            } else {
                Reconstruction::BoundaryAware
            },
        )?;
        Ok(if nearest_node {
            field.with_nearest_node_collisions()
        } else {
            field
        })
    };
    let field: Box<dyn Field> = if mode == "original" {
        Box::new(InstanceField::new(
            grid_field(original)?,
            [0.0; 3],
            [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            1.0,
            None,
            None,
        )?)
    } else {
        let cache_suffix = if local_outer.is_some() {
            "-vacuum-exterior"
        } else {
            ""
        };
        let path = PathBuf::from(OUTPUT).join(format!("{mode}-{factor}{cache_suffix}.pa"));
        let electrodes = if mode == "staircase" || mode == "node-staircase" || local_outer.is_some()
        {
            if factor < 2 {
                return Err("fixed voxel boundaries require at least 2x sampling".into());
            }
            staircase_shapes(&original, factor, mode != "staircase")
        } else {
            shapes()
        };
        let array = if path.exists() {
            read_pa(&path)?.array
        } else {
            let transverse = if local_outer.is_some() { 16 } else { 36 };
            let mut a = PotentialArray::new(
                60 * factor + 1,
                transverse * factor + 1,
                transverse * factor + 1,
            );
            a.mirrors = MirrorFlags {
                x: true,
                y: true,
                z: true,
            };
            [a.dx_mm, a.dy_mm, a.dz_mm] = [0.5 / factor as f64; 3];
            let grid = Grid {
                dims: [a.nx, a.ny, a.nz],
                spacing: [a.dx_mm; 3],
                geometry: Geometry::Planar,
            };
            let solver_options = Options {
                relative_tolerance: 1e-12,
                ..Default::default()
            };
            let report = if mode == "physical"
                || mode == "staircase"
                || mode == "node-staircase"
                || local_outer.is_some()
            {
                let mut cuts =
                    CutCells::new_with_root_tolerance(grid, [0.0; 3], &electrodes, 1e-11)?;
                if let Some(outer_factor) = local_outer {
                    let outer = GridField::with_reconstruction(
                        read_pa(
                            PathBuf::from(OUTPUT).join(format!("node-staircase-{outer_factor}.pa")),
                        )?
                        .array,
                        Reconstruction::GlobalCubic,
                    )?;
                    for z in 0..a.nz {
                        for y in 0..a.ny {
                            if y + 1 != a.ny && z + 1 != a.nz {
                                continue;
                            }
                            for x in 0..a.nx {
                                let i = (z * a.ny + y) * a.nx + x;
                                // Exterior data replace vacuum rows only.
                                if cuts.fixed[i] {
                                    continue;
                                }
                                let p =
                                    [x as f64 * a.dx_mm, y as f64 * a.dy_mm, z as f64 * a.dz_mm];
                                cuts.fixed[i] = true;
                                cuts.values[i] = outer.sample(p, 0.0).potential_v;
                            }
                        }
                    }
                    // Drop the full-domain field before allocating the solver hierarchy.
                }
                a.electrodes.clone_from(&cuts.fixed);
                let report = cuts.solve(&mut a.potentials, solver_options)?;
                // Solver-only O(h^2) snaps are not physical conductor nodes.
                for (i, fixed) in a.electrodes.iter_mut().enumerate() {
                    let p = [
                        (i % a.nx) as f64 * a.dx_mm,
                        (i / a.nx % a.ny) as f64 * a.dy_mm,
                        (i / (a.nx * a.ny)) as f64 * a.dz_mm,
                    ];
                    *fixed =
                        if let Some(e) = electrodes.iter().find(|e| e.shape.distance(p) <= 1e-10) {
                            assert!(
                                (a.potentials[i] - e.voltage).abs() < 1e-9,
                                "reference changed a conductor voltage at {p:?}"
                            );
                            true
                        } else {
                            false
                        };
                }
                report
            } else {
                return Err("mode must be original, physical, staircase or node-staircase".into());
            };
            println!(
                "solve {mode} {factor}: compute_s={:.6}, total_s={:.6}, converged={}, residual={:.3e}",
                report.timings.solve.as_secs_f64(),
                report.elapsed.as_secs_f64(),
                report.converged,
                report.residual_history.last().unwrap().relative,
            );
            if !report.converged {
                return Err("reference solve did not converge".into());
            }
            write_pa(&path, &PaFile::new(a.clone()))?;
            a
        };
        let mut grid = grid_field(array)?;
        if ghost_fill {
            let surface = SurfaceModel::from_sdfs(
                electrodes
                    .iter()
                    .enumerate()
                    .map(|(i, e)| (i + 1, Arc::clone(&e.shape)))
                    .collect(),
                1e-11,
            )?
            .with_pa_domain(grid.array(), [0.0; 3])?;
            grid = grid.with_surface(&surface)?;
            println!(
                "ghost_fill: {}",
                serde_json::to_string(grid.ghost_fill_report().unwrap())?
            );
        }
        Box::new(ReferenceField {
            grid,
            electrodes,
            collision: GridField::with_reconstruction(original, Reconstruction::GlobalCubic)?
                .with_nearest_node_collisions(),
            surface_collisions: mode != "staircase",
        })
    };
    let setup_s = start.elapsed().as_secs_f64();
    if mode == "original" && !timing {
        for axis in [1, 2] {
            let mut worst = (0.0_f64, [0.0; 3]);
            for x in 0..240 {
                for r in 0..72 {
                    let mut p = [x as f64 * 0.25 + 0.125, 0.0, 0.0];
                    p[3 - axis] = r as f64 * 0.25 + 0.125;
                    if field.status(p).is_none() {
                        let normal = field.sample(p, 0.0).electric_v_per_mm[axis].abs();
                        if normal > worst.0 {
                            worst = (normal, p);
                        }
                    }
                }
            }
            println!(
                "mirror axis={axis}: max_vacuum_normal={:.12e}, point={:?}",
                worst.0, worst.1
            );
        }
    }
    let flight_start = Instant::now();
    // The public batch API is generic over sized fields.
    struct Borrowed<'a>(&'a dyn Field);
    impl Field for Borrowed<'_> {
        fn sample(&self, p: [f64; 3], t: f64) -> FieldSample {
            self.0.sample(p, t)
        }
        fn status(&self, p: [f64; 3]) -> Option<SplatReason> {
            self.0.status(p)
        }
        fn intersection(&self, a: [f64; 3], b: [f64; 3]) -> Option<(f64, SplatReason)> {
            self.0.intersection(a, b)
        }
    }
    let results = fly_many(&Borrowed(&*field), &particles, &options)?;
    let flight_s = flight_start.elapsed().as_secs_f64();
    writeln!(
        output,
        "ion,tof_us,x_mm,y_mm,z_mm,reason,min_physical_vertex_bound_mm,min_voxel_segment_clearance_mm,min_cell_segment_clearance_mm"
    )?;
    let electrodes = shapes();
    for (i, result) in results.iter().enumerate() {
        let [x, y, z] = result.final_state.position_mm;
        let clearance = result.trajectory.as_ref().map_or(f64::NAN, |path| {
            path.iter()
                .flat_map(|s| {
                    let p = s.position_mm;
                    let q = [(p[0] - 30.0).abs(), p[1].abs(), p[2].abs()];
                    electrodes.iter().map(move |e| e.shape.distance(q))
                })
                .fold(f64::INFINITY, f64::min)
        });
        let segment_clearance = |boxes: &[Bounds]| {
            result.trajectory.as_ref().map_or(f64::NAN, |path| {
                let mut closest = f64::INFINITY;
                for pair in path.windows(2) {
                    let from = pair[0].position_mm;
                    let to = pair[1].position_mm;
                    for &bounds in boxes {
                        let lower_bound: f64 = (0..3)
                            .map(|a| {
                                (bounds[0][a] - from[a].max(to[a]))
                                    .max(from[a].min(to[a]) - bounds[1][a])
                                    .max(0.0)
                                    .powi(2)
                            })
                            .sum();
                        if lower_bound < closest {
                            closest = closest.min(segment_box_distance_squared(from, to, bounds));
                        }
                    }
                }
                closest.sqrt()
            })
        };
        let voxel_clearance = segment_clearance(&boxes);
        let cell_clearance = segment_clearance(&cell_boxes);
        writeln!(
            output,
            "{},{:.17e},{x:.17e},{y:.17e},{z:.17e},{:?},{clearance:.17e},{voxel_clearance:.17e},{cell_clearance:.17e}",
            i + 1,
            result.tof_us,
            result.splat_reason
        )?;
    }
    output.flush()?;
    println!(
        "{mode} {factor} {label}: setup_s={setup_s:.6}, flight_s={flight_s:.6}, ions={}",
        results.len()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::segment_box_distance_squared;

    #[test]
    fn segment_clearance_includes_box_faces_edges_and_corners() {
        let b = [[0.0; 3], [1.0; 3]];
        assert_eq!(
            segment_box_distance_squared([-1.0, 0.5, 0.5], [2.0, 0.5, 0.5], b),
            0.0
        );
        assert_eq!(
            segment_box_distance_squared([-1.0, 2.0, 0.5], [2.0, 2.0, 0.5], b),
            1.0
        );
        assert_eq!(
            segment_box_distance_squared([-1.0, 2.0, 2.0], [2.0, 2.0, 2.0], b),
            2.0
        );
        assert!(
            (segment_box_distance_squared([-2.0, 1.0, 1.0], [-1.0, 2.0, 2.0], b) - 8.0 / 3.0).abs()
                < 1e-14
        );
        assert_eq!(segment_box_distance_squared([0.5; 3], [0.5; 3], b), 0.0);
    }
}

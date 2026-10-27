//! Read-only, pre-solve surface gap measurements. GEM SDFs are conservative:
//! neither a sampled SDF minimum nor a small SDF residual is an exact gap.
use std::cmp::Ordering;
use std::collections::{BTreeMap, BinaryHeap};

use serde::Serialize;
use simrep_geom::{Point, SignedDistance};
use simrep_solve::gpu::GpuResult;

use super::{GapSource, GeometryOptions, Part};

const THRESHOLD: f64 = 2.5;
const RECOMMENDED: f64 = 10.0;
const WORK_BUDGET: usize = 32_768;

#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum, Serialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum Mode {
    Off,
    Warn,
    Error,
}

#[derive(Serialize)]
pub(super) struct Report {
    mode: Mode,
    threshold_cells: f64,
    recommended_cells: f64,
    pairs: Vec<Pair>,
    recommended_spacing_mm: Option<f64>,
}

impl Report {
    pub(super) fn hints(&self) -> Vec<super::auto::GapHint> {
        self.pairs
            .iter()
            .map(|p| super::auto::GapHint {
                electrodes: p.electrodes,
                bracket_mm: p.gap_bracket_mm.unwrap_or([p.gap_mm; 2]),
                points_mm: p.location_mm,
                resolved: p.status != "unresolved",
            })
            .collect()
    }
}

#[derive(Serialize)]
struct Pair {
    electrodes: [usize; 2],
    method: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    gap_mm: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    gap_bracket_mm: Option<[Option<f64>; 2]>,
    cells_across: f64,
    location_mm: Option<[Point; 2]>,
    status: &'static str,
}

#[derive(Clone, Copy)]
struct Measurement {
    lower: f64,
    upper: f64,
    points: Option<[Point; 2]>,
    bounded: bool,
    resolved: bool,
}

pub(super) fn check(
    args: &GeometryOptions,
    parts: &[Part],
    dims: [usize; 3],
    spacing: Point,
) -> GpuResult<Report> {
    let mut report = Report {
        mode: args.gap_check,
        threshold_cells: THRESHOLD,
        recommended_cells: RECOMMENDED,
        pairs: Vec::new(),
        recommended_spacing_mm: None,
    };
    if args.gap_check == Mode::Off && args.refine.is_none() {
        return Ok(report);
    }
    // A gap can point along the coarsest active axis. Using h_min could
    // incorrectly pass an anisotropic lattice.
    let h = (0..3)
        .filter(|&a| dims[a] > 1)
        .map(|a| spacing[a])
        .fold(0.0, f64::max);
    let tolerance = (h / 10.0).min(1e-3);
    let domain = [
        args.origin,
        std::array::from_fn(|a| args.origin[a] + (dims[a] - 1) as f64 * spacing[a]),
    ];
    #[cfg(feature = "cad")]
    let cad = if parts
        .iter()
        .any(|p| matches!(p.gap_source, GapSource::Cad(_)))
    {
        Some(simrep_cad::MeasurementModel::read_with_units(
            &args.input,
            args.units,
        )?)
    } else {
        None
    };
    let mut measurements: BTreeMap<[usize; 2], Measurement> = BTreeMap::new();
    for (i, a) in parts.iter().enumerate() {
        for b in &parts[i + 1..] {
            if a.assignment.number == b.assignment.number {
                continue;
            }
            let mut numbers = [a.assignment.number, b.assignment.number];
            let mut measurement = match (&a.gap_source, &b.gap_source) {
                (GapSource::Mesh(left), GapSource::Mesh(right)) => {
                    let gap = left.surface_gap(right);
                    Measurement {
                        lower: gap.distance_mm,
                        upper: gap.distance_mm,
                        points: Some(gap.points_mm),
                        bounded: false,
                        resolved: true,
                    }
                }
                (GapSource::Gem(left), GapSource::Gem(right)) => bounded(
                    a.shape.as_ref(),
                    b.shape.as_ref(),
                    *left,
                    *right,
                    domain,
                    tolerance,
                    WORK_BUDGET,
                ),
                #[cfg(feature = "cad")]
                (GapSource::Cad(left), GapSource::Cad(right)) => {
                    let gap = cad.as_ref().unwrap().gap(
                        &left.to_string(),
                        &right.to_string(),
                        simrep_cad::MeasureOptions {
                            cad_tolerance_mm: args.cad_tolerance.unwrap_or(tolerance),
                            ..Default::default()
                        },
                    )?;
                    Measurement {
                        lower: gap.min_distance_mm,
                        upper: gap.min_distance_mm,
                        points: Some(gap.closest_points_mm),
                        bounded: false,
                        resolved: true,
                    }
                }
                _ => unreachable!("a geometry import has a single source kind"),
            };
            if numbers[0] > numbers[1] {
                numbers.swap(0, 1);
                if let Some(points) = &mut measurement.points {
                    points.swap(0, 1);
                }
            }
            measurements
                .entry(numbers)
                .and_modify(|current| {
                    let lower = current.lower.min(measurement.lower);
                    if measurement.upper < current.upper {
                        current.points = measurement.points;
                    }
                    current.upper = current.upper.min(measurement.upper);
                    current.lower = lower;
                    current.resolved = current.upper - current.lower <= tolerance;
                })
                .or_insert(measurement);
        }
    }
    for (electrodes, measurement) in measurements {
        let cells = measurement.lower / h;
        let status = if !measurement.resolved {
            "unresolved"
        } else if cells < THRESHOLD {
            "warn"
        } else {
            "ok"
        };
        let recommended = measurement.lower / RECOMMENDED;
        report.recommended_spacing_mm = Some(
            report
                .recommended_spacing_mm
                .map_or(recommended, |previous| previous.min(recommended)),
        );
        let range = if measurement.bounded {
            format!("[{}, {}]", measurement.lower, measurement.upper)
        } else {
            measurement.lower.to_string()
        };
        println!(
            "Gap electrodes {}-{}: {} gap_mm={}, cells_across={cells:.6}, location_mm={:?}, status={status}; recommended_spacing_mm={recommended:.9} (10 cells; minimum 2.5)",
            electrodes[0],
            electrodes[1],
            if measurement.bounded {
                "bounded"
            } else {
                "exact"
            },
            range,
            measurement.points,
        );
        if status != "ok" && args.gap_check != Mode::Off {
            eprintln!(
                "gap check {status}: electrodes {}-{} have {cells:.6} cells across (minimum 2.5); recommended spacing {recommended:.9} mm for 10 cells",
                electrodes[0], electrodes[1],
            );
        }
        report.pairs.push(Pair {
            electrodes,
            method: if measurement.bounded {
                "bounded"
            } else {
                "exact"
            },
            gap_mm: (!measurement.bounded).then_some(measurement.lower),
            gap_bracket_mm: measurement.bounded.then_some([
                Some(measurement.lower),
                measurement.upper.is_finite().then_some(measurement.upper),
            ]),
            cells_across: cells,
            location_mm: measurement.points,
            status,
        });
    }
    if args.gap_check == Mode::Error
        && args.refine.is_none()
        && report.pairs.iter().any(|p| p.status != "ok")
    {
        return Err("gap check failed: fewer than 2.5 cells across an electrode gap or an unresolved GEM gap; refine --spacing or use --gap-check warn/off".into());
    }
    Ok(report)
}

fn distance(a: Point, b: Point) -> f64 {
    (0..3).map(|i| (a[i] - b[i]).powi(2)).sum::<f64>().sqrt()
}

fn box_gap(a: [Point; 2], b: [Point; 2]) -> f64 {
    (0..3)
        .map(|i| (a[0][i] - b[1][i]).max(b[0][i] - a[1][i]).max(0.0).powi(2))
        .sum::<f64>()
        .sqrt()
}

#[derive(Clone, Copy)]
struct Cell {
    bounds: [Point; 2],
    lower: f64,
}

impl PartialEq for Cell {
    fn eq(&self, other: &Self) -> bool {
        self.lower == other.lower
    }
}
impl Eq for Cell {}
impl PartialOrd for Cell {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Cell {
    fn cmp(&self, other: &Self) -> Ordering {
        other.lower.total_cmp(&self.lower)
    }
}

impl Cell {
    fn center(self) -> Point {
        std::array::from_fn(|i| self.bounds[0][i] / 2.0 + self.bounds[1][i] / 2.0)
    }

    fn new(bounds: [Point; 2], a: &dyn SignedDistance, b: &dyn SignedDistance) -> Self {
        let mut cell = Self { bounds, lower: 0.0 };
        let center = cell.center();
        // |d_A|+|d_B| is 2-Lipschitz and no greater than the sum of
        // Euclidean surface distances. The latter's infimum is the gap.
        // The complete closest-point segment lies in the enclosing hull
        // of the two surface bounds, even when their boxes do not overlap.
        let safety = 128.0 * f64::EPSILON * center.into_iter().map(f64::abs).fold(1.0, f64::max);
        cell.lower = (a.distance(center).abs() + b.distance(center).abs()
            - distance(bounds[0], bounds[1])
            - safety)
            .max(0.0);
        cell
    }
}

/// A sign-changing segment certifies a surface point within its physical
/// width. A small conservative SDF residual alone supplies no upper bound.
fn project(shape: &dyn SignedDistance, mut p: Point, tolerance: f64) -> Option<(Point, f64)> {
    for _ in 0..48 {
        let d = shape.distance(p);
        if !d.is_finite() {
            return None;
        }
        if d == 0.0 {
            return Some((p, 0.0));
        }
        let step = tolerance.max(1e-7);
        let gradient: Point = std::array::from_fn(|i| {
            let mut lo = p;
            let mut hi = p;
            lo[i] -= step;
            hi[i] += step;
            (shape.distance(hi) - shape.distance(lo)) / (2.0 * step)
        });
        let norm2 = gradient.iter().map(|v| v * v).sum::<f64>();
        if !norm2.is_finite() || norm2 < 1e-24 {
            return None;
        }
        let next = std::array::from_fn(|i| p[i] - 1.25 * d * gradient[i] / norm2);
        let next_d = shape.distance(next);
        if !next_d.is_finite() {
            return None;
        }
        if next_d == 0.0 {
            return Some((next, 0.0));
        }
        if d.is_sign_positive() != next_d.is_sign_positive() {
            let (mut outside, mut inside) = if d > 0.0 { (p, next) } else { (next, p) };
            for _ in 0..64 {
                let width = distance(outside, inside);
                if width <= tolerance {
                    return Some((outside, width));
                }
                let mid = std::array::from_fn(|i| outside[i] / 2.0 + inside[i] / 2.0);
                if shape.distance(mid) > 0.0 {
                    outside = mid;
                } else {
                    inside = mid;
                }
            }
            return None;
        }
        p = next;
    }
    None
}

fn upper_bound(
    a: &dyn SignedDistance,
    b: &dyn SignedDistance,
    seed: Point,
    tolerance: f64,
    best: &mut Measurement,
) {
    let Some((mut left, mut left_error)) = project(a, seed, tolerance) else {
        return;
    };
    for _ in 0..4 {
        let Some((right, right_error)) = project(b, left, tolerance) else {
            return;
        };
        let upper = distance(left, right) + left_error + right_error;
        if upper < best.upper {
            best.upper = upper;
            best.points = Some([left, right]);
        }
        let Some(projected) = project(a, right, tolerance) else {
            return;
        };
        (left, left_error) = projected;
    }
}

fn bounded(
    a: &dyn SignedDistance,
    b: &dyn SignedDistance,
    left: [Point; 2],
    right: [Point; 2],
    fallback: [Point; 2],
    tolerance: f64,
    budget: usize,
) -> Measurement {
    let finite = left.iter().chain(&right).flatten().all(|v| v.is_finite());
    let domain = std::array::from_fn(|end| {
        std::array::from_fn(|i| {
            let value = if end == 0 {
                left[0][i].min(right[0][i])
            } else {
                left[1][i].max(right[1][i])
            };
            if value.is_finite() {
                value
            } else {
                fallback[end][i]
            }
        })
    });
    let separation = box_gap(left, right);
    let baseline = if separation.is_finite() {
        separation
    } else {
        0.0
    };
    let mut best = Measurement {
        lower: baseline,
        upper: f64::INFINITY,
        points: None,
        bounded: true,
        resolved: false,
    };
    for bounds in [domain, left, right] {
        if bounds.iter().flatten().all(|v| v.is_finite()) {
            for bits in 0..9 {
                let seed = if bits == 8 {
                    std::array::from_fn(|i| bounds[0][i] / 2.0 + bounds[1][i] / 2.0)
                } else {
                    std::array::from_fn(|i| bounds[(bits >> i) & 1][i])
                };
                upper_bound(a, b, seed, tolerance * 1e-4, &mut best);
            }
        }
    }
    let mut queue = BinaryHeap::from([Cell::new(domain, a, b)]);
    for iteration in 0..budget {
        let Some(cell) = queue.pop() else {
            break;
        };
        best.lower = if finite {
            baseline.max(cell.lower).min(best.upper)
        } else {
            baseline
        };
        if best.upper - best.lower <= tolerance {
            best.resolved = true;
            return best;
        }
        if cell.lower > best.upper {
            continue;
        }
        if iteration % 64 == 0 {
            upper_bound(a, b, cell.center(), tolerance * 1e-4, &mut best);
        }
        let axis = (0..3)
            .max_by(|&i, &j| {
                (cell.bounds[1][i] - cell.bounds[0][i])
                    .total_cmp(&(cell.bounds[1][j] - cell.bounds[0][j]))
            })
            .unwrap();
        let middle = cell.center()[axis];
        for end in 0..2 {
            let mut bounds = cell.bounds;
            bounds[1 - end][axis] = middle;
            queue.push(Cell::new(bounds, a, b));
        }
    }
    if let Some(cell) = queue.peek() {
        best.lower = if finite {
            baseline.max(cell.lower).min(best.upper)
        } else {
            baseline
        };
    }
    best.resolved = best.upper - best.lower <= tolerance;
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gem_pair(text: &str, tolerance: f64, budget: usize) -> Measurement {
        let gem = simrep_gem::parse_gem(text).unwrap();
        let sources = gem.electrode_sdfs().unwrap();
        bounded(
            &sources[0],
            &sources[1],
            sources[0].bounds_mm(),
            sources[1].bounds_mm(),
            [[-8.0; 3], [8.0; 3]],
            tolerance,
            budget,
        )
    }

    fn contains(measurement: Measurement, analytic: f64) {
        assert!(
            measurement.lower <= analytic + 1e-12,
            "{} > {analytic}",
            measurement.lower
        );
        assert!(
            measurement.upper >= analytic - 1e-12,
            "{} < {analytic}",
            measurement.upper
        );
        let points = measurement.points.unwrap();
        assert!(distance(points[0], points[1]) <= measurement.upper + 1e-12);
    }

    #[test]
    fn concentric_gem_spheres_bracket_analytic_gap_and_budget_exhaustion() {
        let text = "pa_define(17,17,17,planar)\ne(1){fill{within{sphere(8,8,8,2)}}}\ne(2){fill{within{sphere(8,8,8,4)} notin{sphere(8,8,8,3)}}}";
        let gap = gem_pair(text, 1e-3, WORK_BUDGET);
        contains(gap, 1.0);
        let exhausted = gem_pair(text, 1e-3, 0);
        contains(exhausted, 1.0);
        assert!(!exhausted.resolved);
        assert!(gap.lower >= exhausted.lower);
        // Positive endpoints are within the reported physical bracket error
        // of the inner sphere and the shell's inner surface.
        let points = gap.points.unwrap();
        assert!((distance(points[0], [8.0; 3]) - 2.0).abs() < 1e-6);
        assert!((distance(points[1], [8.0; 3]) - 3.0).abs() < 1e-6);
    }

    #[test]
    fn transformed_csg_gap_bracket_contains_analytic_value() {
        let text = "pa_define(17,17,17,planar)\ne(1){locate(2,2,2,2){fill{within{box3d(0,0,0,1,1,1)}}}}\ne(2){fill{within{box3d(5,2,2,7,4,4)} notin{sphere(6,3,3,0.2)}}}";
        let gap = gem_pair(text, 1e-3, WORK_BUDGET);
        contains(gap, 1.0);
        assert!(gap.resolved, "[{}, {}]", gap.lower, gap.upper);
    }

    #[test]
    fn conservative_residual_cannot_be_used_as_an_upper_bound() {
        let left = |p: Point| ((p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt() - 1.0) * 0.1;
        let right =
            |p: Point| (((p[0] - 4.0).powi(2) + p[1] * p[1] + p[2] * p[2]).sqrt() - 1.0) * 0.1;
        let bounds = [[-1.0; 3], [1.0; 3]];
        let other = [[3.0, -1.0, -1.0], [5.0, 1.0, 1.0]];
        let result = bounded(&left, &right, bounds, other, bounds, 1e-3, WORK_BUDGET);
        contains(result, 2.0);
        assert!(result.resolved);
    }

    #[test]
    fn unbounded_extrusions_can_use_global_box_separation_but_not_local_samples() {
        let text = "pa_define(9,5,1,planar)\ne(1){fill{within{box(-1,-1,2,5)}}}\ne(2){fill{within{box(4,-1,9,5)}}}";
        let result = gem_pair(text, 1e-3, WORK_BUDGET);
        contains(result, 2.0);
        assert!(result.resolved);
        let outer = |p: Point| p[0].hypot(p[1]) - 2.0;
        let inner = |p: Point| p[0].hypot(p[1]) - 1.0;
        let bounds = [[-2.0, -2.0, f64::NEG_INFINITY], [2.0, 2.0, f64::INFINITY]];
        let result = bounded(
            &outer,
            &inner,
            bounds,
            bounds,
            [[-3.0; 3], [3.0; 3]],
            1e-3,
            128,
        );
        contains(result, 1.0);
        assert_eq!(result.lower, 0.0);
        assert!(!result.resolved);
    }
}

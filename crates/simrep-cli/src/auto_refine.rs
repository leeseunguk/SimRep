// Refinement-policy adaptation from using76/Iteration-CFD:
// rust/src/automesher/octree.rs, rust/src/adapt.rs, and
// docs/08-site-mesh-playbook.md (retrieved 2026-10-27).
// Copyright (c) 2026 Iterations Co., Ltd.
// Collaborator and contributor: Meteo Simulation Co., Ltd.
// Prosperity Public License 3.0.0; see licenses/THIRD_PARTY.md.
//! Static surface/gap marking, sorted box closure, and a 2:1 interface gate.
//! This is not the upstream polyhedral mesher or a solution-error estimator.
use std::sync::Arc;

use serde::Serialize;
use simrep_geom::{Point, SignedDistance};
use simrep_solve::adaptive::Region;
use simrep_solve::mg::Grid;

pub const MIN_CELLS: f64 = 2.5;

#[derive(Clone)]
pub struct GapHint {
    pub electrodes: [usize; 2],
    pub bracket_mm: [Option<f64>; 2],
    pub points_mm: Option<[Point; 2]>,
    pub resolved: bool,
}

#[derive(Serialize)]
pub struct BoxReport {
    pub bounds_mm: [Point; 2],
    pub factor: usize,
    pub dims: [usize; 3],
    pub nodes: usize,
}

#[derive(Serialize)]
pub struct GapPrediction {
    pub electrodes: [usize; 2],
    pub bracket_mm: [Option<f64>; 2],
    pub location_mm: Option<[Point; 2]>,
    /// Local prediction at the witness, not a certificate over the pair.
    pub expected_cells_across: [Option<f64>; 2],
    /// Conservative domain-wide lower bound; unrefined vacuum retains base h.
    pub minimum_cells_across: Option<f64>,
    pub status: &'static str,
}

#[derive(Serialize)]
pub struct Selection {
    pub requested_factor: usize,
    pub surface_band_mm: f64,
    pub candidate_boxes: usize,
    pub merged_boxes: usize,
    pub balance_limited_boxes: usize,
    pub skipped_gap_witnesses: usize,
    pub base_nodes: usize,
    pub patch_nodes: usize,
    pub stored_nodes: usize,
    pub boxes: Vec<BoxReport>,
    pub gaps: Vec<GapPrediction>,
}

impl Selection {
    pub fn regions(&self) -> Vec<Region> {
        self.boxes
            .iter()
            .map(|b| Region {
                bounds_mm: b.bounds_mm,
                factor: b.factor,
            })
            .collect()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct NodeBox {
    lo: [usize; 3],
    hi: [usize; 3],
}

/// Pure deterministic selection; every length threshold scales with base h.
/// Surface-band leaves are dyadic subdivisions of the base domain. Touching
/// leaves close to bounding boxes at a sorted fixed point: the prototype
/// cannot assemble touching or nested patches. Closure may fill the domain.
pub fn select(
    grid: Grid,
    origin: Point,
    surfaces: &[Arc<dyn SignedDistance>],
    gaps: &[GapHint],
    factor: usize,
) -> Result<Selection, &'static str> {
    if ![2, 4].contains(&factor) {
        return Err("automatic refinement factor must be 2 or 4");
    }
    let h = (0..3)
        .filter(|&a| grid.dims[a] > 1)
        .map(|a| grid.spacing[a])
        .fold(0.0_f64, f64::max);
    let domain = NodeBox {
        lo: [0; 3],
        hi: grid.dims.map(|n| n - 1),
    };
    let position =
        |c: [usize; 3]| std::array::from_fn(|a| origin[a] + c[a] as f64 * grid.spacing[a]);
    let mut boxes = Vec::new();
    let mut pending = vec![domain];
    while let Some(b) = pending.pop() {
        let center =
            std::array::from_fn(|a| origin[a] + 0.5 * (b.lo[a] + b.hi[a]) as f64 * grid.spacing[a]);
        let radius = (0..3)
            .map(|a| (0.5 * (b.hi[a] - b.lo[a]) as f64 * grid.spacing[a]).powi(2))
            .sum::<f64>()
            .sqrt();
        let mut near = false;
        for shape in surfaces {
            let distance = shape.distance(center);
            if !distance.is_finite() {
                return Err("SDF returned a non-finite distance during automatic refinement");
            }
            // The SignedDistance contract is 1-Lipschitz. This cannot prune
            // a zero set (including a sharp wedge) inside the band.
            near |= distance.abs() <= radius + h;
        }
        if !near {
            continue;
        }
        let split: Vec<_> = (0..3).filter(|&a| b.hi[a] - b.lo[a] > 4).collect();
        if split.is_empty() {
            // One base-cell buffer keeps trace faces away from marked leaves.
            boxes.push(NodeBox {
                lo: std::array::from_fn(|a| b.lo[a].saturating_sub(1)),
                hi: std::array::from_fn(|a| (b.hi[a] + 1).min(domain.hi[a])),
            });
        } else {
            for child in 0..1usize << split.len() {
                let mut c = b;
                for (bit, &a) in split.iter().enumerate() {
                    let mid = (b.lo[a] + b.hi[a]) / 2;
                    if child & (1 << bit) == 0 {
                        c.hi[a] = mid;
                    } else {
                        c.lo[a] = mid;
                    }
                }
                pending.push(c);
            }
        }
    }
    let mut skipped_gap_witnesses = 0;
    for gap in gaps {
        if gap.resolved && gap.bracket_mm[0].is_some_and(|g| g / h >= MIN_CELLS) {
            continue;
        }
        let Some(points) = gap.points_mm else {
            skipped_gap_witnesses += 1;
            continue;
        };
        if points.iter().flatten().any(|p| !p.is_finite())
            || (0..3).any(|a| {
                points[0][a].max(points[1][a]) < origin[a]
                    || points[0][a].min(points[1][a]) > position(domain.hi)[a]
            })
        {
            skipped_gap_witnesses += 1;
            continue;
        }
        boxes.push(NodeBox {
            lo: std::array::from_fn(|a| {
                let q = (points[0][a].min(points[1][a]) - origin[a]) / grid.spacing[a];
                (q.floor().max(0.0) as usize)
                    .saturating_sub(2)
                    .min(domain.hi[a])
            }),
            hi: std::array::from_fn(|a| {
                let q = (points[0][a].max(points[1][a]) - origin[a]) / grid.spacing[a];
                (q.ceil().max(0.0) as usize)
                    .saturating_add(2)
                    .min(domain.hi[a])
            }),
        });
    }
    let candidate_boxes = boxes.len();
    boxes.sort_unstable();
    // A union can introduce new contacts, so restart after each merge.
    loop {
        let pair = (0..boxes.len()).find_map(|i| {
            (i + 1..boxes.len())
                .find(|&j| {
                    (0..3).all(|a| {
                        boxes[i].lo[a] <= boxes[j].hi[a] && boxes[j].lo[a] <= boxes[i].hi[a]
                    })
                })
                .map(|j| (i, j))
        });
        let Some((i, j)) = pair else { break };
        let b = boxes.remove(j);
        boxes[i] = NodeBox {
            lo: std::array::from_fn(|a| boxes[i].lo[a].min(b.lo[a])),
            hi: std::array::from_fn(|a| boxes[i].hi[a].max(b.hi[a])),
        };
        boxes.sort_unstable();
    }
    let mut balance_limited_boxes = 0;
    let mut reports = Vec::new();
    for b in boxes {
        // A factor-4 internal face needs a surrounding level-1 halo. Nested
        // or touching boxes are forbidden today; cap this box instead of
        // claiming a 2:1 balance that the assembled grid does not have.
        let local_factor = if factor == 4 && b != domain {
            balance_limited_boxes += 1;
            2
        } else {
            factor
        };
        let dims = std::array::from_fn(|a| (b.hi[a] - b.lo[a]) * local_factor + 1);
        let nodes = dims
            .into_iter()
            .try_fold(1usize, usize::checked_mul)
            .filter(|&n| n <= u32::MAX as usize)
            .ok_or("automatic refinement patch node count overflows")?;
        reports.push(BoxReport {
            bounds_mm: [position(b.lo), position(b.hi)],
            factor: local_factor,
            dims,
            nodes,
        });
    }
    let guaranteed_factor = reports
        .iter()
        .find(|b| b.bounds_mm == [position(domain.lo), position(domain.hi)])
        .map_or(1, |b| b.factor);
    let predictions = gaps
        .iter()
        .map(|gap| {
            let local_factor = gap.points_mm.map_or(1, |points| {
                reports
                    .iter()
                    .find(|b| {
                        points.iter().all(|p| {
                            (0..3).all(|a| p[a] >= b.bounds_mm[0][a] && p[a] <= b.bounds_mm[1][a])
                        })
                    })
                    .map_or(1, |b| b.factor)
            });
            let expected = gap
                .bracket_mm
                .map(|g| g.map(|g| g * local_factor as f64 / h));
            GapPrediction {
                electrodes: gap.electrodes,
                bracket_mm: gap.bracket_mm,
                location_mm: gap.points_mm,
                expected_cells_across: expected,
                minimum_cells_across: gap.bracket_mm[0].map(|g| g * guaranteed_factor as f64 / h),
                status: if !gap.resolved {
                    "unresolved_bracket"
                } else if expected[0].is_none_or(|n| n < MIN_CELLS) {
                    "gap_unresolvable"
                } else {
                    "witness_resolved"
                },
            }
        })
        .collect();
    let base_nodes = grid.dims.into_iter().product();
    let patch_nodes = reports.iter().map(|b| b.nodes).sum::<usize>();
    Ok(Selection {
        requested_factor: factor,
        surface_band_mm: h,
        candidate_boxes,
        merged_boxes: candidate_boxes - reports.len(),
        balance_limited_boxes,
        skipped_gap_witnesses,
        base_nodes,
        patch_nodes,
        stored_nodes: base_nodes + patch_nodes,
        boxes: reports,
        gaps: predictions,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use simrep_geom::Sdf;
    use simrep_solve::mg::Geometry;

    fn grid() -> Grid {
        Grid {
            dims: [33, 17, 1],
            spacing: [1.0; 3],
            geometry: Geometry::Planar,
        }
    }

    #[test]
    fn deterministic_clamped_disjoint_and_balanced() {
        let surfaces: Vec<Arc<dyn SignedDistance>> = vec![
            Arc::new(Sdf::Sphere {
                center: [1.0, 2.0, 0.0],
                radius: 0.3,
            }),
            Arc::new(Sdf::Sphere {
                center: [30.0, 14.0, 0.0],
                radius: 0.3,
            }),
        ];
        let a = select(grid(), [0.0; 3], &surfaces, &[], 4).unwrap();
        let b = select(
            grid(),
            [0.0; 3],
            &surfaces.into_iter().rev().collect::<Vec<_>>(),
            &[],
            4,
        )
        .unwrap();
        assert_eq!(
            serde_json::to_vec(&a).unwrap(),
            serde_json::to_vec(&b).unwrap()
        );
        assert_eq!(a.boxes.len(), 2);
        assert_eq!(a.balance_limited_boxes, 2);
        for (i, b) in a.boxes.iter().enumerate() {
            assert_eq!(b.factor, 2);
            for axis in 0..3 {
                assert!(b.bounds_mm[0][axis] >= 0.0);
                assert!(b.bounds_mm[1][axis] <= (grid().dims[axis] - 1) as f64);
            }
            for other in &a.boxes[i + 1..] {
                assert!((0..3).any(|axis| {
                    b.bounds_mm[1][axis] < other.bounds_mm[0][axis]
                        || other.bounds_mm[1][axis] < b.bounds_mm[0][axis]
                }));
            }
        }
        assert_eq!(a.stored_nodes, a.base_nodes + a.patch_nodes);
    }

    #[test]
    fn touching_gap_boxes_merge_and_unresolvable_gaps_are_reported() {
        let gaps: Vec<_> = [4.0, 7.0]
            .map(|x| GapHint {
                electrodes: [1, 2],
                bracket_mm: [Some(0.5), Some(0.5)],
                points_mm: Some([[x, 8.0, 0.0], [x + 0.5, 8.0, 0.0]]),
                resolved: true,
            })
            .into();
        let a = select(grid(), [0.0; 3], &[], &gaps, 2).unwrap();
        assert_eq!(a.boxes.len(), 1);
        assert_eq!(a.merged_boxes, 1);
        assert_eq!(a.gaps[0].expected_cells_across, [Some(1.0); 2]);
        assert_eq!(a.gaps[0].status, "gap_unresolvable");
        let mut reversed = gaps;
        reversed.reverse();
        let b = select(grid(), [0.0; 3], &[], &reversed, 2).unwrap();
        assert_eq!(a.boxes[0].bounds_mm, b.boxes[0].bounds_mm);
    }

    #[test]
    fn absent_witness_and_nonfinite_surface_are_not_silently_accepted() {
        let gap = GapHint {
            electrodes: [1, 2],
            bracket_mm: [Some(0.0), None],
            points_mm: None,
            resolved: false,
        };
        let a = select(grid(), [0.0; 3], &[], &[gap], 2).unwrap();
        assert!(a.boxes.is_empty());
        assert_eq!(a.skipped_gap_witnesses, 1);
        assert_eq!(a.gaps[0].status, "unresolved_bracket");
        assert!(select(grid(), [0.0; 3], &[Arc::new(|_| f64::NAN)], &[], 2).is_err());
    }

    #[test]
    fn factor_four_has_no_internal_four_to_one_interface() {
        let shape: Arc<dyn SignedDistance> = Arc::new(|_: Point| 0.0);
        let selection = select(grid(), [0.0; 3], &[shape], &[], 4).unwrap();
        assert_eq!(selection.boxes.len(), 1);
        assert_eq!(selection.boxes[0].bounds_mm, [[0.0; 3], [32.0, 16.0, 0.0]]);
        assert_eq!(selection.boxes[0].factor, 4);
        assert_eq!(selection.balance_limited_boxes, 0);
    }
}

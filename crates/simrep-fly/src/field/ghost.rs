//! Vacuum continuation for metal knots used by the existing shared Hermite jets.
use std::collections::BTreeMap;

use rayon::prelude::*;
use serde::Serialize;
use simrep_core::PotentialArray;

use crate::{Vector, surface::SurfaceModel};

// The one-sided closure consumes four knots, but the tridiagonal spline solve
// also has exponentially decaying nonlocal influence. Three layers leave a
// visible metal-core tail in mixed jets; extend through eight Chebyshev layers.
const LAYERS: usize = 8;
const RADIUS: usize = LAYERS + 2;
const CONDITION_LIMIT: f64 = 1e8;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct GhostFillReport {
    /// Counts for degree 1, 2 and 3, respectively.
    pub degree_counts: [usize; 3],
    /// Eligible metal knots whose fits failed, retaining the PA value.
    pub kept_nodes: usize,
    /// Electrodes with no PA metal knot or nonconstant metal potentials.
    pub skipped_electrodes: Vec<usize>,
    pub layers: usize,
}

impl GhostFillReport {
    pub(crate) fn merge(&mut self, other: &Self) {
        for (count, n) in self.degree_counts.iter_mut().zip(other.degree_counts) {
            *count += n;
        }
        self.kept_nodes += other.kept_nodes;
        self.skipped_electrodes.extend(&other.skipped_electrodes);
        self.skipped_electrodes.sort_unstable();
        self.skipped_electrodes.dedup();
    }
}

struct Datum {
    point: Vector,
    value: f64,
}

pub(super) fn fill(
    a: &PotentialArray,
    surface: &SurfaceModel,
) -> (PotentialArray, GhostFillReport) {
    let dims = [a.nx, a.ny, a.nz];
    let spacing = [a.dx_mm, a.dy_mm, a.dz_mm];
    let h = (0..3)
        .filter(|&axis| dims[axis] > 1)
        .map(|axis| spacing[axis])
        .fold(f64::INFINITY, f64::min);
    let coordinate = |i: usize| [i % dims[0], i / dims[0] % dims[1], i / (dims[0] * dims[1])];
    let point = |i: usize| {
        let c = coordinate(i);
        std::array::from_fn(|axis| c[axis] as f64 * spacing[axis] / h)
    };
    let nodes: Vec<_> = if let Some(cuts) = &surface.reconstruction_cuts {
        cuts.nodes.clone()
    } else {
        (0..a.potentials.len())
            .into_par_iter()
            .map(|i| {
                let p = point(i).map(|v| v * h);
                surface.membership(surface.source_point(p))
            })
            .collect()
    };
    let mut voltages: BTreeMap<_, Option<f64>> =
        surface.electrode_numbers().map(|n| (n, None)).collect();
    let mut nonconstant = std::collections::BTreeSet::new();
    for (i, number) in nodes.iter().enumerate() {
        if !a.electrodes[i] {
            continue;
        }
        if let Some(number) = number
            && let Some(voltage) = voltages.get_mut(number)
        {
            if voltage.is_some_and(|v| v != a.potentials[i]) {
                nonconstant.insert(*number);
            } else {
                *voltage = Some(a.potentials[i]);
            }
        }
    }
    let mut report = GhostFillReport {
        layers: LAYERS,
        ..Default::default()
    };
    voltages.retain(|number, voltage| {
        if voltage.is_none() || nonconstant.contains(number) {
            report.skipped_electrodes.push(*number);
            false
        } else {
            true
        }
    });
    if voltages.is_empty() {
        return (a.clone(), report);
    }
    // Separable sliding box maxima mark the Chebyshev band in O(nodes).
    // Do not search a fit cube around every node deep in a thick conductor.
    let mut near: Vec<_> = nodes
        .iter()
        .enumerate()
        .map(|(i, n)| !a.electrodes[i] && n.is_none())
        .collect();
    for axis in 0..3 {
        let n = dims[axis];
        if n == 1 {
            continue;
        }
        let stride = [1, dims[0], dims[0] * dims[1]][axis];
        let mut expanded = vec![false; near.len()];
        for line in 0..near.len() / n {
            let base = line / stride * stride * n + line % stride;
            let mut count = (0..=LAYERS.min(n - 1))
                .filter(|&c| near[base + c * stride])
                .count();
            for c in 0..n {
                expanded[base + c * stride] = count > 0;
                if c >= LAYERS {
                    count -= usize::from(near[base + (c - LAYERS) * stride]);
                }
                if c + LAYERS + 1 < n {
                    count += usize::from(near[base + (c + LAYERS + 1) * stride]);
                }
            }
        }
        near = expanded;
    }
    let mut knots: Vec<Vec<Datum>> = (0..a.potentials.len()).map(|_| Vec::new()).collect();
    if let Some(cuts) = &surface.reconstruction_cuts {
        for link in &cuts.links {
            let (i, fractions, numbers) = (link.node, link.fractions, link.electrodes);
            for (side, fraction) in fractions.into_iter().enumerate() {
                if let (Some(t), Some(number)) = (fraction, numbers[side])
                    && let Some(Some(voltage)) = voltages.get(&number)
                {
                    let mut p = point(i);
                    p[side / 2] += if side % 2 == 0 { -t } else { t } * spacing[side / 2] / h;
                    knots[i].push(Datum {
                        point: p,
                        value: *voltage,
                    });
                }
            }
        }
    } else {
        // In-memory source surfaces carry the same exact intersections, obtained
        // once here rather than using a signed-distance value as a cut fraction.
        let stride = [1, dims[0], dims[0] * dims[1]];
        for (i, node_knots) in knots.iter_mut().enumerate() {
            if a.electrodes[i] {
                continue;
            }
            let c = coordinate(i);
            let from = surface.source_point(point(i).map(|v| v * h));
            for side in 0..6 {
                let axis = side / 2;
                if dims[axis] == 1
                    || side % 2 == 0 && c[axis] == 0
                    || side % 2 == 1 && c[axis] + 1 == dims[axis]
                {
                    continue;
                }
                let j = i.wrapping_add_signed(if side % 2 == 0 {
                    -(stride[axis] as isize)
                } else {
                    stride[axis] as isize
                });
                if !a.electrodes[j] {
                    continue;
                }
                let to = surface.source_point(point(j).map(|v| v * h));
                if let Some((t, number)) = surface.first_hit(from, to)
                    && let Some(Some(voltage)) = voltages.get(&number)
                {
                    let p = std::array::from_fn(|axis| {
                        point(i)[axis] + t * (point(j)[axis] - point(i)[axis])
                    });
                    node_knots.push(Datum {
                        point: p,
                        value: *voltage,
                    });
                }
            }
        }
    }
    let orders = [1, 2, 3].map(|degree| {
        let mut powers = Vec::new();
        for z in 0..=if dims[2] > 1 { degree } else { 0 } {
            for y in 0..=if dims[1] > 1 { degree - z } else { 0 } {
                for x in 0..=if dims[0] > 1 { degree - y - z } else { 0 } {
                    powers.push([x, y, z]);
                }
            }
        }
        powers
    });
    let fits: Vec<_> = (0..a.potentials.len())
        .into_par_iter()
        .filter_map(|i| {
            if !a.electrodes[i] || !near[i] || !nodes[i].is_some_and(|n| voltages.contains_key(&n))
            {
                return None;
            }
            let c = coordinate(i);
            let center = point(i);
            let mut vacuum = Vec::new();
            let mut boundary = Vec::new();
            let ranges = std::array::from_fn::<_, 3, _>(|axis| {
                c[axis].saturating_sub(RADIUS)..=(c[axis] + RADIUS).min(dims[axis] - 1)
            });
            for z in ranges[2].clone() {
                for y in ranges[1].clone() {
                    for x in ranges[0].clone() {
                        let j = (z * dims[1] + y) * dims[0] + x;
                        let distance = |p: Vector| {
                            (0..3)
                                .map(|axis| (p[axis] - center[axis]).powi(2))
                                .sum::<f64>()
                        };
                        if !a.electrodes[j] && nodes[j].is_none() {
                            vacuum.push((
                                distance(point(j)),
                                Datum {
                                    point: point(j),
                                    value: a.potentials[j],
                                },
                            ));
                        }
                        boundary.extend(knots[j].iter().map(|k| {
                            (
                                distance(k.point),
                                Datum {
                                    point: k.point,
                                    value: k.value,
                                },
                            )
                        }));
                    }
                }
            }
            vacuum.sort_unstable_by(|a, b| a.0.total_cmp(&b.0));
            boundary.sort_unstable_by(|a, b| a.0.total_cmp(&b.0));
            for degree in (1..=3).rev() {
                let powers = &orders[degree - 1];
                let data: Vec<_> = vacuum
                    .iter()
                    .take(2 * powers.len())
                    .chain(boundary.iter().take(powers.len()))
                    .map(|(_, datum)| datum)
                    .collect();
                if let Some(mut value) = fit(&data, center, powers) {
                    let anti = [a.anti_mirrors.x, a.anti_mirrors.y, a.anti_mirrors.z];
                    if (0..3).any(|axis| anti[axis] && c[axis] == 0) {
                        value = 0.0;
                    }
                    return Some((i, value, degree));
                }
            }
            Some((i, a.potentials[i], 0))
        })
        .collect();
    let mut filled = a.clone();
    for (i, value, degree) in fits {
        if degree == 0 {
            report.kept_nodes += 1;
        } else {
            report.degree_counts[degree - 1] += 1;
            filled.potentials[i] = value;
        }
    }
    (filled, report)
}

// Householder QR with column pivoting. All pivot/rank decisions depend only on
// geometry, never on voltages, so accepted fits are fixed linear operators.
fn fit(data: &[&Datum], center: Vector, powers: &[[usize; 3]]) -> Option<f64> {
    let (m, n) = (data.len(), powers.len());
    if 2 * m < 3 * n {
        return None;
    }
    let mut columns = vec![vec![0.0; m]; n];
    let mut rhs = Vec::with_capacity(m);
    for (row, datum) in data.iter().enumerate() {
        let p = std::array::from_fn::<_, 3, _>(|axis| datum.point[axis] - center[axis]);
        let weight = 1.0 / (p.iter().map(|v| v * v).sum::<f64>() + 0.01).sqrt();
        rhs.push(weight * datum.value);
        for (column, power) in columns.iter_mut().zip(powers) {
            column[row] = weight
                * (0..3)
                    .map(|axis| p[axis].powi(power[axis] as i32))
                    .product::<f64>();
        }
    }
    let mut permutation: Vec<_> = (0..n).collect();
    for k in 0..n {
        let pivot = (k..n).max_by(|&a, &b| {
            let norm = |j: usize| columns[j][k..].iter().map(|v| v * v).sum::<f64>();
            norm(a).total_cmp(&norm(b))
        })?;
        columns.swap(k, pivot);
        permutation.swap(k, pivot);
        let norm = columns[k][k..].iter().map(|v| v * v).sum::<f64>().sqrt();
        if norm <= f64::EPSILON {
            return None;
        }
        let diagonal = -norm.copysign(columns[k][k]);
        let mut reflector = columns[k][k..].to_vec();
        reflector[0] -= diagonal;
        let scale = 2.0 / reflector.iter().map(|v| v * v).sum::<f64>();
        for column in &mut columns[k + 1..] {
            let dot = scale
                * reflector
                    .iter()
                    .zip(&column[k..])
                    .map(|(v, w)| v * w)
                    .sum::<f64>();
            for (v, r) in column[k..].iter_mut().zip(&reflector) {
                *v -= dot * r;
            }
        }
        let dot = scale
            * reflector
                .iter()
                .zip(&rhs[k..])
                .map(|(v, w)| v * w)
                .sum::<f64>();
        for (v, r) in rhs[k..].iter_mut().zip(&reflector) {
            *v -= dot * r;
        }
        columns[k][k] = diagonal;
    }
    let norm = (0..n)
        .map(|row| (row..n).map(|col| columns[col][row].abs()).sum::<f64>())
        .fold(0.0, f64::max);
    let mut inverse_rows = vec![0.0; n];
    for column in 0..n {
        let mut inverse = vec![0.0; n];
        for row in (0..=column).rev() {
            inverse[row] = (f64::from(row == column)
                - (row + 1..=column)
                    .map(|j| columns[j][row] * inverse[j])
                    .sum::<f64>())
                / columns[row][row];
            inverse_rows[row] += inverse[row].abs();
        }
    }
    if norm * inverse_rows.into_iter().fold(0.0, f64::max) > CONDITION_LIMIT {
        return None;
    }
    let mut solution = vec![0.0; n];
    for row in (0..n).rev() {
        solution[row] = (rhs[row]
            - (row + 1..n)
                .map(|j| columns[j][row] * solution[j])
                .sum::<f64>())
            / columns[row][row];
    }
    Some(solution[permutation.iter().position(|&p| p == 0)?])
}

#[cfg(test)]
mod tests {
    use super::*;
    use simrep_geom::Sdf;
    use std::sync::Arc;

    fn capacitor(left: f64, right: f64) -> (PotentialArray, SurfaceModel) {
        let mut array = PotentialArray::new(19, 13, 1);
        array.dx_mm = 0.1;
        array.dy_mm = 0.1;
        for y in 0..array.ny {
            for x in 0..array.nx {
                let i = y * array.nx + x;
                let p = x as f64 * array.dx_mm;
                array.electrodes[i] = !(0.235..1.565).contains(&p);
                let t = ((p - 0.235) / 1.33).clamp(0.0, 1.0);
                array.potentials[i] = left * (1.0 - t) + right * t;
            }
        }
        let surface = SurfaceModel::from_sdfs(
            vec![
                (
                    7,
                    Arc::new(Sdf::Plane {
                        normal: [1.0, 0.0, 0.0],
                        offset: 0.235,
                    }),
                ),
                (
                    9,
                    Arc::new(Sdf::Plane {
                        normal: [-1.0, 0.0, 0.0],
                        offset: -1.565,
                    }),
                ),
            ],
            1e-13,
        )
        .unwrap()
        .with_pa_domain(&array, [0.0; 3])
        .unwrap();
        (array, surface)
    }

    #[test]
    fn ghost_fill_is_linear_in_dirichlet_and_vacuum_data() {
        let (first, surface) = capacitor(1.0, 0.0);
        let (second, _) = capacitor(0.0, 1.0);
        let (sum, _) = capacitor(-3.7, 2.3);
        let (first, first_report) = fill(&first, &surface);
        let (second, second_report) = fill(&second, &surface);
        let (sum, sum_report) = fill(&sum, &surface);
        assert_eq!(first_report, second_report);
        assert_eq!(first_report, sum_report);
        assert!(first_report.degree_counts[2] > 0);
        assert_eq!(first_report.kept_nodes, 0);
        for ((a, b), c) in first
            .potentials
            .iter()
            .zip(second.potentials)
            .zip(sum.potentials)
        {
            assert!((c - (-3.7 * a + 2.3 * b)).abs() < 3e-12);
        }
    }

    #[test]
    fn nonconstant_or_missing_dirichlet_data_are_not_guessed() {
        let (mut array, surface) = capacitor(1.0, 0.0);
        for (i, voltage) in array.potentials.iter_mut().enumerate() {
            if array.electrodes[i] && i % array.nx < 3 {
                *voltage += (i / array.nx) as f64 * 0.001;
            }
        }
        let (filled, report) = fill(&array, &surface);
        assert_eq!(report.skipped_electrodes, vec![7]);
        for (i, &value) in array.potentials.iter().enumerate() {
            if i % array.nx < 3 {
                assert_eq!(value.to_bits(), filled.potentials[i].to_bits());
            }
        }
        array.electrodes.fill(false);
        let (filled, report) = fill(&array, &surface);
        assert_eq!(report.skipped_electrodes, vec![7, 9]);
        assert_eq!(array.potentials, filled.potentials);
    }

    #[test]
    fn rank_test_falls_back_without_dividing_by_tiny_cut_fractions() {
        let powers = vec![[0, 0, 0], [1, 0, 0], [2, 0, 0], [3, 0, 0]];
        let data: Vec<_> = [1e-14, 1.0, 2.0, 3.0, 4.0, 5.0]
            .map(|x| Datum {
                point: [x, 0.0, 0.0],
                value: 1.0 + 2.0 * x + 0.3 * x * x,
            })
            .into_iter()
            .collect();
        assert!(
            (fit(&data.iter().collect::<Vec<_>>(), [0.0; 3], &powers).unwrap() - 1.0).abs() < 1e-12
        );
        let rank_deficient = vec![[0, 0, 0], [1, 0, 0], [0, 1, 0]];
        assert!(fit(&data.iter().collect::<Vec<_>>(), [0.0; 3], &rank_deficient).is_none());
        assert!(
            fit(
                &data.iter().collect::<Vec<_>>(),
                [0.0; 3],
                &rank_deficient[..2]
            )
            .is_some()
        );
    }

    #[test]
    fn sparse_vacuum_uses_reported_linear_fallback() {
        let mut array = PotentialArray::new(6, 1, 1);
        array.electrodes = vec![true, true, false, false, true, true];
        array.potentials = vec![0.0, 0.0, 0.8 / 2.6, 1.8 / 2.6, 1.0, 1.0];
        let surface = SurfaceModel::from_sdfs(
            vec![
                (
                    1,
                    Arc::new(Sdf::Plane {
                        normal: [1.0, 0.0, 0.0],
                        offset: 1.2,
                    }),
                ),
                (
                    2,
                    Arc::new(Sdf::Plane {
                        normal: [-1.0, 0.0, 0.0],
                        offset: -3.8,
                    }),
                ),
            ],
            1e-13,
        )
        .unwrap()
        .with_pa_domain(&array, [0.0; 3])
        .unwrap();
        let (filled, report) = fill(&array, &surface);
        assert_eq!(report.degree_counts, [4, 0, 0]);
        assert_eq!(report.kept_nodes, 0);
        assert!((filled.potentials[0] + 1.2 / 2.6).abs() < 1e-12);
        assert!((filled.potentials[5] - 3.8 / 2.6).abs() < 1e-12);
    }

    #[test]
    fn three_dimensional_fill_reproduces_a_vacuum_polynomial() {
        let (mut array, surface) = capacitor(-2.0, 3.0);
        array.nz = 5;
        array.potentials = array.potentials.repeat(array.nz);
        array.electrodes = array.electrodes.repeat(array.nz);
        array.dz_mm = 0.1;
        let surface = surface.with_pa_domain(&array, [0.0; 3]).unwrap();
        let (filled, report) = fill(&array, &surface);
        assert!(report.degree_counts[2] > 0);
        assert_eq!(report.kept_nodes, 0);
        for (i, value) in filled.potentials.iter().enumerate() {
            let x = (i % array.nx) as f64 * 0.1;
            assert!((value - (-2.0 + 5.0 * (x - 0.235) / 1.33)).abs() < 1e-10);
        }
    }

    #[test]
    fn cylindrical_fill_preserves_axial_field_and_radial_symmetry() {
        use crate::{Field, GridField};
        let (mut array, surface) = capacitor(-2.0, 3.0);
        array.symmetry = simrep_core::Symmetry::Cylindrical;
        let surface = surface.with_pa_domain(&array, [0.0; 3]).unwrap();
        let original = array.clone();
        let field = GridField::new_compact(array)
            .unwrap()
            .with_surface(&surface)
            .unwrap();
        assert_eq!(field.array().potentials, original.potentials);
        assert_eq!(field.array().electrodes, original.electrodes);
        for p in [[0.24, 0.3, 0.4], [0.24, 0.0, 0.0], [1.56, 0.0, -0.5]] {
            let sample = field.sample(p, 0.0);
            assert!((sample.potential_v - (-2.0 + 5.0 * (p[0] - 0.235) / 1.33)).abs() < 1e-10);
            assert!((sample.electric_v_per_mm[0] + 5.0 / 1.33).abs() < 1e-9);
            assert!(sample.electric_v_per_mm[1].abs() < 1e-9);
            assert!(sample.electric_v_per_mm[2].abs() < 1e-9);
        }
    }

    #[test]
    fn replacing_surface_with_no_eligible_electrodes_restores_original_field() {
        use crate::{Field, GridField};
        let (array, surface) = capacitor(1.0, 0.0);
        let skipped = SurfaceModel::from_sdfs(
            vec![(
                12,
                Arc::new(Sdf::Plane {
                    normal: [1.0, 0.0, 0.0],
                    offset: -10.0,
                }),
            )],
            1e-13,
        )
        .unwrap()
        .with_pa_domain(&array, [0.0; 3])
        .unwrap();
        for compact in [false, true] {
            let original = if compact {
                GridField::new_compact(array.clone())
            } else {
                GridField::new(array.clone())
            }
            .unwrap();
            let filled = original.clone().with_surface(&surface).unwrap();
            assert!(
                filled
                    .ghost_fill_report()
                    .unwrap()
                    .degree_counts
                    .iter()
                    .sum::<usize>()
                    > 0
            );
            let replaced = filled.with_surface(&skipped).unwrap();
            assert_eq!(
                replaced.ghost_fill_report().unwrap().skipped_electrodes,
                vec![12]
            );
            for p in [[0.2, 0.4, 0.0], [0.24, 0.4, 0.0], [1.56, 0.4, 0.0]] {
                assert_eq!(replaced.sample(p, 0.0), original.sample(p, 0.0));
            }
        }
    }

    #[test]
    fn band_mask_leaves_deep_metal_untouched() {
        let mut array = PotentialArray::new(61, 3, 1);
        for i in 0..array.potentials.len() {
            let x = (i % array.nx) as f64;
            array.electrodes[i] = !(15.2..44.8).contains(&x);
            array.potentials[i] = ((x - 15.2) / 29.6).clamp(0.0, 1.0);
        }
        let surface = SurfaceModel::from_sdfs(
            vec![
                (
                    1,
                    Arc::new(Sdf::Plane {
                        normal: [1.0, 0.0, 0.0],
                        offset: 15.2,
                    }),
                ),
                (
                    2,
                    Arc::new(Sdf::Plane {
                        normal: [-1.0, 0.0, 0.0],
                        offset: -44.8,
                    }),
                ),
            ],
            1e-12,
        )
        .unwrap()
        .with_pa_domain(&array, [0.0; 3])
        .unwrap();
        let (filled, report) = fill(&array, &surface);
        assert_eq!(report.degree_counts.iter().sum::<usize>(), 48);
        assert_eq!(report.kept_nodes, 0);
        for (i, &value) in array.potentials.iter().enumerate() {
            let x = i % array.nx;
            if !(8..=52).contains(&x) || !array.electrodes[i] {
                assert_eq!(value.to_bits(), filled.potentials[i].to_bits());
            } else {
                assert!((filled.potentials[i] - (x as f64 - 15.2) / 29.6).abs() < 1e-10);
            }
        }
    }
}

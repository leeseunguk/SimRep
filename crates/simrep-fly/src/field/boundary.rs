use rayon::prelude::*;
use simrep_core::{PotentialArray, Symmetry};

use crate::Vector;

/// Tensor quintics share the value, first and second (including mixed)
/// derivatives at every node. Thus the potential is C2 and E is C1 even where
/// adjacent vacuum-line splines have different boundary conditions.
#[derive(Clone, Debug)]
pub(super) struct BoundarySpline {
    pub(super) cells: [usize; 3],
    pub(super) degrees: [usize; 3],
    pub(super) coefficients: Vec<f64>,
    pub(super) jets: Vec<Vec<f64>>,
}

// Value, first derivative, second derivative at t=0, then at t=1.
const HERMITE: [[f64; 6]; 6] = [
    [1.0, 0.0, 0.0, -10.0, 15.0, -6.0],
    [0.0, 1.0, 0.0, -6.0, 8.0, -3.0],
    [0.0, 0.0, 0.5, -1.5, 1.5, -0.5],
    [0.0, 0.0, 0.0, 10.0, -15.0, 6.0],
    [0.0, 0.0, 0.0, -4.0, 7.0, -3.0],
    [0.0, 0.0, 0.0, 0.5, -1.0, 0.5],
];

impl BoundarySpline {
    pub(super) fn new(a: &PotentialArray) -> Self {
        Self::build(a, false)
    }

    pub(super) fn new_compact(a: &PotentialArray) -> Self {
        // Derivative lines are short; waking 32 workers for every mixed jet
        // costs more than solving them on four workers for interactive arrays.
        if a.potentials.len() <= 1_048_576 && rayon::current_num_threads() > 4 {
            static POOL: std::sync::OnceLock<rayon::ThreadPool> = std::sync::OnceLock::new();
            return POOL
                .get_or_init(|| {
                    rayon::ThreadPoolBuilder::new()
                        .num_threads(4)
                        .build()
                        .expect("field derivative worker pool")
                })
                .install(|| Self::build(a, true));
        }
        Self::build(a, true)
    }

    fn build(a: &PotentialArray, compact: bool) -> Self {
        let dims = [a.nx, a.ny, a.nz];
        let components = dims.map(|n| if n == 1 { 1 } else { 3 });
        let jet_stride = [1, components[0], components[0] * components[1]];
        let mut jets = vec![a.potentials.clone()];
        let mut seconds: [Vec<Option<Vec<f64>>>; 3] =
            std::array::from_fn(|_| (0..components.iter().product()).map(|_| None).collect());
        let constant = a.potentials.iter().all(|&v| v == a.potentials[0]);
        for component in 1..components.iter().product() {
            if constant {
                jets.push(vec![0.0; a.potentials.len()]);
                continue;
            }
            let order = std::array::from_fn::<_, 3, _>(|axis| {
                component / jet_stride[axis] % components[axis]
            });
            let mut jet = vec![0.0; a.potentials.len()];
            let mut count = 0;
            // Symmetrize the mixed derivatives: the mask makes the directional
            // line operators noncommuting at electrode corners.
            for axis in 0..3 {
                if order[axis] == 0 {
                    continue;
                }
                let parent = component - order[axis] * jet_stride[axis];
                let derivative = if order[axis] == 2 {
                    seconds[axis][parent].take().unwrap()
                } else {
                    let (first, second) = differentiate(a, &jets[parent], axis);
                    seconds[axis][parent] = Some(second);
                    first
                };
                for (value, d) in jet.iter_mut().zip(derivative) {
                    *value += d;
                }
                count += 1;
            }
            for value in &mut jet {
                *value /= count as f64;
            }
            enforce_vacuum_mirrors(a, &mut jet, order);
            jets.push(jet);
        }
        let cells = dims.map(|n| n.saturating_sub(1).max(1));
        let degrees = dims.map(|n| if n == 1 { 1 } else { 6 });
        let mut result = Self {
            cells,
            degrees,
            coefficients: Vec::new(),
            jets,
        };
        if !compact {
            let size = degrees.iter().product::<usize>();
            let mut coefficients = vec![0.0; cells.iter().product::<usize>() * size];
            coefficients
                .par_chunks_mut(size)
                .enumerate()
                .for_each(|(index, c)| result.cell_powers(index, c));
            result.coefficients = coefficients;
        }
        result
    }

    pub(super) fn cell_powers(&self, index: usize, c: &mut [f64]) {
        let cells = self.cells;
        let degrees = self.degrees;
        let dims =
            std::array::from_fn::<_, 3, _>(|a| if degrees[a] == 1 { 1 } else { cells[a] + 1 });
        let components = degrees.map(|n| if n == 1 { 1 } else { 3 });
        let x = index % cells[0];
        let y = index / cells[0] % cells[1];
        let z = index / (cells[0] * cells[1]);
        for (i, value) in c.iter_mut().enumerate() {
            let local = [
                i % degrees[0],
                i / degrees[0] % degrees[1],
                i / (degrees[0] * degrees[1]),
            ];
            let node = [x + local[0] / 3, y + local[1] / 3, z + local[2] / 3];
            let component =
                (local[0] % 3) + components[0] * ((local[1] % 3) + components[1] * (local[2] % 3));
            *value = self.jets[component][(node[2] * dims[1] + node[1]) * dims[0] + node[0]];
        }
        for axis in 0..3 {
            if degrees[axis] == 1 {
                continue;
            }
            let stride = [1, degrees[0], degrees[0] * degrees[1]][axis];
            for i in 0..c.len() {
                if !(i / stride).is_multiple_of(6) {
                    continue;
                }
                let input = std::array::from_fn::<_, 6, _>(|j| c[i + j * stride]);
                for p in 0..6 {
                    c[i + p * stride] = (0..6).map(|j| input[j] * HERMITE[j][p]).sum();
                }
            }
        }
    }

    pub(super) fn sample(&self, q: Vector) -> (f64, Vector) {
        let cell: [usize; 3] =
            std::array::from_fn(|a| q[a].floor().max(0.0).min((self.cells[a] - 1) as f64) as usize);
        let t: Vector = std::array::from_fn(|a| q[a] - cell[a] as f64);
        let [nx, ny, nz] = self.degrees;
        let size = nx * ny * nz;
        let start = ((cell[2] * self.cells[1] + cell[1]) * self.cells[0] + cell[0]) * size;
        let mut local = [0.0; 216];
        let c = if self.coefficients.is_empty() {
            self.cell_powers(start / size, &mut local[..size]);
            &local[..size]
        } else {
            &self.coefficients[start..start + size]
        };
        let mut value = 0.0;
        let mut gradient = [0.0; 3];
        for z in (0..nz).rev() {
            let mut vy = 0.0;
            let mut dx = 0.0;
            let mut dy = 0.0;
            for y in (0..ny).rev() {
                let mut vx = 0.0;
                let mut d = 0.0;
                for x in (0..nx).rev() {
                    d = d * t[0] + vx;
                    vx = vx * t[0] + c[(z * ny + y) * nx + x];
                }
                dy = dy * t[1] + vy;
                vy = vy * t[1] + vx;
                dx = dx * t[1] + d;
            }
            gradient[2] = gradient[2] * t[2] + value;
            value = value * t[2] + vy;
            gradient[0] = gradient[0] * t[2] + dx;
            gradient[1] = gradient[1] * t[2] + dy;
        }
        (value, gradient)
    }
}

fn enforce_vacuum_mirrors(a: &PotentialArray, jet: &mut [f64], order: [usize; 3]) {
    let dims = [a.nx, a.ny, a.nz];
    let mirrors = [a.mirrors.x, a.mirrors.y, a.mirrors.z];
    let anti = [a.anti_mirrors.x, a.anti_mirrors.y, a.anti_mirrors.z];
    for axis in 0..3 {
        let even = mirrors[axis] && !anti[axis] || axis == 1 && a.symmetry == Symmetry::Cylindrical;
        if !even || order[axis] != 1 {
            continue;
        }
        let u = (axis + 1) % 3;
        let v = (axis + 2) % 3;
        let index = |i: usize, j: usize| {
            let mut node = [0; 3];
            node[u] = i;
            node[v] = j;
            (node[2] * dims[1] + node[1]) * dims[0] + node[0]
        };
        // A wholly Dirichlet face retains its one-sided normal condition.
        // On a mixed mirror face every shared odd-normal jet must be zero:
        // an electrode corner on the plane can belong to a vacuum cell
        // because a corner on the next normal plane is free.
        if !(0..dims[v]).any(|j| (0..dims[u]).any(|i| !a.electrodes[index(i, j)])) {
            continue;
        }
        for j in 0..dims[v] {
            for i in 0..dims[u] {
                jet[index(i, j)] = 0.0;
            }
        }
    }
}

fn differentiate(a: &PotentialArray, input: &[f64], axis: usize) -> (Vec<f64>, Vec<f64>) {
    let dims = [a.nx, a.ny, a.nz];
    let n = dims[axis];
    let stride = [1, dims[0], dims[0] * dims[1]][axis];
    let mirrors = [a.mirrors.x, a.mirrors.y, a.mirrors.z];
    let anti = [a.anti_mirrors.x, a.anti_mirrors.y, a.anti_mirrors.z];
    let even = (mirrors[axis] && !anti[axis]) || (axis == 1 && a.symmetry == Symmetry::Cylindrical);
    let mut result = vec![0.0; input.len()];
    let mut second = vec![0.0; input.len()];
    let bases: Vec<_> = (0..input.len())
        .filter(|base| (base / stride).is_multiple_of(n))
        .collect();
    let lines: Vec<_> = bases
        .into_par_iter()
        .map(|base| {
            let values: Vec<_> = (0..n).map(|i| input[base + i * stride]).collect();
            // A fixed Dirichlet pole/electrode on the mirror face takes
            // precedence over the vacuum Neumann mirror condition.
            let mirror_vacuum = even && !a.electrodes[base];
            let (mut first, mut second) = line_derivatives(&values, mirror_vacuum, anti[axis]);
            let mut index = 0;
            while index < n {
                if a.electrodes[base + index * stride] {
                    index += 1;
                    continue;
                }
                let first_vacuum = index;
                while index < n && !a.electrodes[base + index * stride] {
                    index += 1;
                }
                let lo = first_vacuum.saturating_sub(1);
                let hi = index.min(n - 1);
                // A full vacuum interval already has exactly these derivatives.
                if lo == 0 && hi == n - 1 {
                    continue;
                }
                let (d, m) = line_derivatives(
                    &values[lo..=hi],
                    lo == 0 && mirror_vacuum,
                    lo == 0 && anti[axis],
                );
                first[lo..=hi].copy_from_slice(&d);
                second[lo..=hi].copy_from_slice(&m);
            }
            (base, first, second)
        })
        .collect();
    for (base, first, m) in lines {
        for (i, (d, m)) in first.into_iter().zip(m).enumerate() {
            result[base + i * stride] = d;
            second[base + i * stride] = m;
        }
    }
    (result, second)
}

/// Clamped cubic spline on a vacuum interval, including the first Dirichlet
/// metal node, never nodes deeper in the metal. All derivatives are in gu.
fn line_derivatives(v: &[f64], even: bool, odd: bool) -> (Vec<f64>, Vec<f64>) {
    let n = v.len();
    if n == 1 {
        return (vec![0.0], vec![0.0]);
    }
    let slope = |reverse: bool| {
        let f = |i| v[if reverse { n - 1 - i } else { i }];
        let d = match n {
            2 => f(1) - f(0),
            3 => (-3.0 * f(0) + 4.0 * f(1) - f(2)) / 2.0,
            _ => (-11.0 * f(0) + 18.0 * f(1) - 9.0 * f(2) + 2.0 * f(3)) / 6.0,
        };
        if reverse { -d } else { d }
    };
    let left = if even { 0.0 } else { slope(false) };
    let right = slope(true);
    let mut m = vec![0.0; n];
    let mut diagonal = vec![4.0; n];
    diagonal[0] = if odd { 1.0 } else { 2.0 };
    diagonal[n - 1] = 2.0;
    m[0] = if odd { 0.0 } else { 6.0 * (v[1] - v[0] - left) };
    m[n - 1] = 6.0 * (right - v[n - 1] + v[n - 2]);
    for i in 1..n - 1 {
        m[i] = 6.0 * (v[i - 1] - 2.0 * v[i] + v[i + 1]);
    }
    for i in 1..n {
        let upper = if i == 1 && odd { 0.0 } else { 1.0 };
        let factor = 1.0 / diagonal[i - 1];
        diagonal[i] -= factor * upper;
        m[i] -= factor * m[i - 1];
    }
    m[n - 1] /= diagonal[n - 1];
    for i in (0..n - 1).rev() {
        let upper = if i == 0 && odd { 0.0 } else { 1.0 };
        m[i] = (m[i] - upper * m[i + 1]) / diagonal[i];
    }
    let mut d: Vec<_> = (0..n - 1)
        .map(|i| v[i + 1] - v[i] - (2.0 * m[i] + m[i + 1]) / 6.0)
        .collect();
    d.push(right);
    (d, m)
}

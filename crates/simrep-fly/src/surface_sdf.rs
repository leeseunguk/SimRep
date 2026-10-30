//! Sampled collision surfaces. Their zero sets approximate the source surfaces;
//! intersections shall be interpreted relative to the trilinear interpolant.
use simrep_core::{PotentialArray, Symmetry};

use crate::{FlyError, Vector, surface::SurfaceModel};

#[derive(Clone, Debug)]
pub struct SdfVolume {
    pub number: usize,
    pub collision_only: bool,
    pub dims: [usize; 3],
    pub origin: Vector,
    pub spacing: Vector,
    pub samples_f32: Vec<f32>,
    pub samples_f64: Vec<f64>,
    pub lipschitz: f64,
    /// Maximum observed error at 4096 deterministic pseudo-random band points.
    /// This is an estimate, not a certified upper bound.
    pub interp_error_mm: f64,
    /// Bound on interpolation error relative to the source field, in mm.
    pub error_bound_mm: f64,
}

#[derive(Clone, Debug)]
pub struct SdfVolumes {
    pub volumes: Vec<SdfVolume>,
    cylindrical: bool,
    mirrors: [bool; 3],
    tolerance: f64,
}

impl SdfVolumes {
    pub(crate) fn gpu_geometry(&self) -> (bool, [bool; 3], f64) {
        (self.cylindrical, self.mirrors, self.tolerance)
    }

    pub fn build(
        surface: &SurfaceModel,
        pa_domain: &PotentialArray,
        resolution_divisor: u32,
    ) -> Result<Self, FlyError> {
        surface.validate_array(pa_domain)?;
        if resolution_divisor == 0 {
            return Err(FlyError("SDF resolution divisor must be positive"));
        }
        let base = [pa_domain.nx, pa_domain.ny, pa_domain.nz];
        let spacing = [pa_domain.dx_mm, pa_domain.dy_mm, pa_domain.dz_mm]
            .map(|s| s / f64::from(resolution_divisor));
        let mut dims = [0; 3];
        for a in 0..3 {
            dims[a] = (base[a] - 1)
                .checked_mul(resolution_divisor as usize)
                .and_then(|n| n.checked_add(1))
                .ok_or(FlyError("SDF grid size overflow"))?;
        }
        let count = dims
            .into_iter()
            .try_fold(1usize, usize::checked_mul)
            .ok_or(FlyError("SDF grid size overflow"))?;
        let origin = surface.source_point([0.0; 3]);
        let band = 4.0 * spacing.into_iter().fold(f64::INFINITY, f64::min);
        let mut volumes = Vec::new();
        for electrode in &surface.electrodes {
            // Preserve a sidecar interpolant exactly instead of resampling it.
            let (dims, origin, spacing, values, error_bound_mm) =
                if let Some(sampled) = &electrode.sampled {
                    (
                        sampled.dims,
                        sampled.origin,
                        sampled.spacing,
                        sampled.values.clone(),
                        0.0,
                    )
                } else {
                    let mut values = Vec::with_capacity(count);
                    for i in 0..count {
                        let c = [i % dims[0], i / dims[0] % dims[1], i / (dims[0] * dims[1])];
                        let p = std::array::from_fn(|a| origin[a] + c[a] as f64 * spacing[a]);
                        values.push(electrode.shape.distance(p));
                    }
                    let bound = electrode.lipschitz
                        * (0..3)
                            .filter(|&a| dims[a] > 1)
                            .map(|a| spacing[a] * 0.5)
                            .sum::<f64>();
                    (dims, origin, spacing, values, bound)
                };
            let count = values.len();
            let samples_f32: Vec<f32> = values.iter().map(|&v| v as f32).collect();
            let strides = [1, dims[0], dims[0] * dims[1]];
            let mut gradients = [0.0_f64; 3];
            for a in 0..3 {
                for i in 0..count {
                    if i / strides[a] % dims[a] + 1 < dims[a] {
                        gradients[a] = gradients[a]
                            .max(((values[i + strides[a]] - values[i]) / spacing[a]).abs());
                        gradients[a] = gradients[a].max(
                            ((f64::from(samples_f32[i + strides[a]]) - f64::from(samples_f32[i]))
                                / spacing[a])
                                .abs(),
                        );
                    }
                }
            }
            let lipschitz = gradients.into_iter().map(|g| g * g).sum::<f64>().sqrt();
            let mut volume = SdfVolume {
                number: electrode.number,
                collision_only: surface.collision_only_numbers.contains(&electrode.number),
                dims,
                origin,
                spacing,
                samples_f32,
                samples_f64: values,
                lipschitz,
                interp_error_mm: 0.0,
                error_bound_mm,
            };
            let mut state = 0x6a09e667f3bcc909_u64;
            for _ in 0..4096 {
                let p = std::array::from_fn(|a| {
                    state ^= state << 13;
                    state ^= state >> 7;
                    state ^= state << 17;
                    origin[a]
                        + ((state >> 11) as f64 / ((1_u64 << 53) as f64))
                            * (dims[a] - 1) as f64
                            * spacing[a]
                });
                let exact = electrode.shape.distance(p);
                if exact.abs() <= band {
                    volume.interp_error_mm = volume
                        .interp_error_mm
                        .max((volume.interpolate(p) - exact).abs());
                }
            }
            volumes.push(volume);
        }
        Ok(Self {
            volumes,
            cylindrical: pa_domain.symmetry == Symmetry::Cylindrical,
            mirrors: [
                pa_domain.mirrors.x,
                pa_domain.mirrors.y,
                pa_domain.mirrors.z,
            ],
            tolerance: surface.tolerance_mm,
        })
    }

    pub fn memory_bytes(&self) -> usize {
        self.volumes
            .iter()
            .map(|v| v.samples_f32.len() * 4 + v.samples_f64.len() * 8)
            .sum()
    }

    fn distance(&self, volume: &SdfVolume, mut p: Vector) -> f64 {
        if self.cylindrical {
            p[1] = p[1].hypot(p[2]);
            p[2] = 0.0;
        }
        for (a, coordinate) in p.iter_mut().enumerate() {
            if self.mirrors[a] {
                *coordinate = volume.origin[a] + (*coordinate - volume.origin[a]).abs();
            }
        }
        let mut boundary = f64::NEG_INFINITY;
        for (a, &coordinate) in p.iter().enumerate() {
            if volume.dims[a] > 1 {
                let low = if self.mirrors[a] || self.cylindrical && a == 1 {
                    f64::NEG_INFINITY
                } else {
                    volume.origin[a] - coordinate
                };
                boundary = boundary.max(low.max(
                    coordinate - volume.origin[a] - (volume.dims[a] - 1) as f64 * volume.spacing[a],
                ));
            }
        }
        volume.interpolate(p).max(boundary)
    }

    pub fn membership(&self, p: Vector) -> Option<usize> {
        self.volumes
            .iter()
            .find(|v| self.distance(v, p) <= 0.0)
            .map(|v| v.number)
    }

    pub fn first_hit(&self, from: Vector, to: Vector) -> Option<(f64, usize)> {
        self.segment_hit(from, to, false)
    }

    pub(crate) fn segment_hit(
        &self,
        from: Vector,
        to: Vector,
        launch: bool,
    ) -> Option<(f64, usize)> {
        let delta: Vector = std::array::from_fn(|a| to[a] - from[a]);
        let length = delta.into_iter().map(|d| d * d).sum::<f64>().sqrt();
        let mut best: Option<(f64, usize)> = None;
        for volume in &self.volumes {
            let at = |t: f64| std::array::from_fn(|a| from[a] + t * delta[a]);
            let mut t = 0.0;
            let d = self.distance(volume, from);
            if length == 0.0 {
                if !launch && d <= 0.0 {
                    return Some((0.0, volume.number));
                }
                continue;
            }
            if !launch && d <= 0.0 {
                if best.is_none_or(|b| b.0 > 0.0) {
                    best = Some((0.0, volume.number));
                }
                continue;
            }
            let end = self.distance(volume, to);
            let l = volume.lipschitz.max(1.0); // PA clipping has unit slope.
            if d / l > length && end > 0.0 {
                continue;
            }
            // Every sign change of the interpolant lies at a breakpoint: a
            // cell/fold boundary or a polynomial root inside one cell. Between
            // breakpoints the sign is constant, so this cannot skip a crossing.
            let breaks = self.breakpoints(volume, from, delta);
            let mut leaving = launch && d <= 0.0;
            let mut previous = 0.0;
            for &tb in breaks.iter().skip(1) {
                let mid = 0.5 * (t + tb);
                for probe in [mid, tb] {
                    let inside = self.distance(volume, at(probe)) <= 0.0;
                    if leaving {
                        if !inside {
                            leaving = false;
                            previous = probe;
                        }
                        continue;
                    }
                    if !inside {
                        previous = probe;
                        continue;
                    }
                    // First inside probe: localize the entry between the last
                    // outside probe and this one (no other root in between).
                    let (mut lo, mut hi) = (previous, probe);
                    while (hi - lo) * length > self.tolerance * 1e-3 {
                        let m = 0.5 * (lo + hi);
                        if m == lo || m == hi {
                            break;
                        }
                        if self.distance(volume, at(m)) <= 0.0 {
                            hi = m;
                        } else {
                            lo = m;
                        }
                    }
                    if best.is_none_or(|b| hi < b.0) {
                        best = Some((hi, volume.number));
                    }
                    t = 2.0; // done with this volume
                    break;
                }
                if t > 1.0 {
                    break;
                }
                t = tb;
            }
        }
        best
    }

    /// Sorted parameters in [0, 1] containing every root of the interpolated
    /// distance along the segment: cell faces, mirror folds, the PA box faces
    /// and the real roots of the per-cell polynomial (trilinear: cubic in t;
    /// cylindrical meridian: roots of q B^2 - A^2, a quartic).
    fn breakpoints(&self, volume: &SdfVolume, from: Vector, delta: Vector) -> Vec<f64> {
        let mut cuts = vec![0.0, 1.0];
        let o = volume.origin;
        let h = volume.spacing;
        let dims = volume.dims;
        let push = |cuts: &mut Vec<f64>, t: f64| {
            if t > 0.0 && t < 1.0 && t.is_finite() {
                cuts.push(t);
            }
        };
        // Linear axes (x always; y and z unless cylindrical).
        let linear_axes: &[usize] = if self.cylindrical { &[0] } else { &[0, 1, 2] };
        for &a in linear_axes {
            if dims[a] < 2 || delta[a] == 0.0 {
                continue;
            }
            if self.mirrors[a] {
                push(&mut cuts, (o[a] - from[a]) / delta[a]);
            }
            let c0 = from[a] - o[a];
            let c1 = c0 + delta[a];
            for sign in [1.0, -1.0] {
                if !self.mirrors[a] && sign < 0.0 {
                    continue;
                }
                let (lo, hi) = (sign * c0, sign * c1);
                let (lo, hi) = (lo.min(hi) / h[a], lo.max(hi) / h[a]);
                let k0 = lo.ceil().max(0.0) as i64;
                let k1 = hi.floor().min((dims[a] - 1) as f64) as i64;
                for k in k0..=k1 {
                    push(&mut cuts, (sign * k as f64 * h[a] - c0) / delta[a]);
                }
            }
        }
        // Cylindrical radius r(t) = |(y, z)(t)|: grid circles are roots of
        // q(t) = R^2, a quadratic.
        let q = [
            from[1] * from[1] + from[2] * from[2],
            2.0 * (from[1] * delta[1] + from[2] * delta[2]),
            delta[1] * delta[1] + delta[2] * delta[2],
        ];
        if self.cylindrical && dims[1] > 1 {
            let r_at = |t: f64| (q[0] + t * (q[1] + t * q[2])).max(0.0).sqrt();
            let mut rmin = r_at(0.0).min(r_at(1.0));
            let rmax = r_at(0.0).max(r_at(1.0));
            if q[2] > 0.0 {
                let ts = (-q[1] / (2.0 * q[2])).clamp(0.0, 1.0);
                rmin = rmin.min(r_at(ts));
            }
            let mut radii = Vec::new();
            if self.mirrors[1] {
                radii.push(o[1]);
            }
            for sign in [1.0, -1.0] {
                if !self.mirrors[1] && sign < 0.0 {
                    continue;
                }
                let (lo, hi) = ((rmin - o[1]) * sign, (rmax - o[1]) * sign);
                let (lo, hi) = (lo.min(hi) / h[1], lo.max(hi) / h[1]);
                for k in lo.ceil().max(0.0) as i64..=hi.floor().min((dims[1] - 1) as f64) as i64 {
                    radii.push(o[1] + sign * k as f64 * h[1]);
                }
            }
            for r in radii {
                if r < 0.0 {
                    continue;
                }
                for t in poly_roots(&[q[0] - r * r, q[1], q[2]], 0.0, 1.0) {
                    push(&mut cuts, t);
                }
            }
        }
        cuts.sort_by(f64::total_cmp);
        cuts.dedup();
        // Polynomial roots inside every piece that lies in the sampled box.
        let mut out = cuts.clone();
        for w in cuts.windows(2) {
            let (t0, t1) = (w[0], w[1]);
            if t1 - t0 <= 0.0 {
                continue;
            }
            if let Some(poly) = self.piece_polynomial(volume, from, delta, q, t0, t1) {
                for s in poly_roots(&poly, 0.0, 1.0) {
                    let t = t0 + s * (t1 - t0);
                    if t > t0 && t < t1 {
                        out.push(t);
                    }
                }
            }
        }
        out.sort_by(f64::total_cmp);
        out.dedup();
        out
    }

    /// Polynomial in the local piece parameter s in [0, 1] whose real roots
    /// contain every root of the interpolant on the piece; None outside the box.
    fn piece_polynomial(
        &self,
        volume: &SdfVolume,
        from: Vector,
        delta: Vector,
        q: [f64; 3],
        t0: f64,
        t1: f64,
    ) -> Option<Vec<f64>> {
        let o = volume.origin;
        let h = volume.spacing;
        let dims = volume.dims;
        let tm = 0.5 * (t0 + t1);
        let dt = t1 - t0;
        // Folded coordinate along a linear axis as an affine function of s.
        let fold = |a: usize, c_mid: f64| -> f64 {
            if self.mirrors[a] && c_mid < o[a] {
                -1.0
            } else {
                1.0
            }
        };
        let mut idx = [0usize; 3];
        // Affine local coordinates u_a(s) = alpha + beta s (linear axes).
        let mut lin: [[f64; 2]; 3] = [[0.0, 0.0]; 3];
        for a in 0..3 {
            if dims[a] < 2 || (self.cylindrical && a > 0) {
                continue;
            }
            let c_mid = from[a] + tm * delta[a];
            let sgn = fold(a, c_mid);
            let x_mid = (o[a] + sgn * (c_mid - o[a]) - o[a]) / h[a];
            if !(0.0..=(dims[a] - 1) as f64).contains(&x_mid) {
                return None;
            }
            idx[a] = (x_mid.floor() as usize).min(dims[a] - 2);
            let c0 = from[a] + t0 * delta[a];
            let x0 = (o[a] + sgn * (c0 - o[a]) - o[a]) / h[a] - idx[a] as f64;
            lin[a] = [x0, sgn * delta[a] * dt / h[a]];
        }
        let value =
            |i: usize, j: usize, k: usize| volume.samples_f64[(k * dims[1] + j) * dims[0] + i];
        if !self.cylindrical {
            // Trilinear: product of linear weights -> cubic in s.
            let mut total = vec![0.0; 4];
            for k in 0..2 {
                for j in 0..2 {
                    for i in 0..2 {
                        let corner = [i, j, k];
                        let mut p = vec![1.0];
                        let mut skip = false;
                        for a in 0..3 {
                            if dims[a] < 2 {
                                if corner[a] == 1 {
                                    skip = true;
                                }
                                continue;
                            }
                            let w = if corner[a] == 0 {
                                vec![1.0 - lin[a][0], -lin[a][1]]
                            } else {
                                vec![lin[a][0], lin[a][1]]
                            };
                            p = poly_mul(&p, &w);
                        }
                        if skip {
                            continue;
                        }
                        let c = value(
                            idx[0] + if dims[0] > 1 { i } else { 0 },
                            idx[1] + if dims[1] > 1 { j } else { 0 },
                            idx[2] + if dims[2] > 1 { k } else { 0 },
                        );
                        for (n, coef) in p.iter().enumerate() {
                            total[n] += c * coef;
                        }
                    }
                }
            }
            return Some(total);
        }
        // Cylindrical meridian (x, r): f = A0(u) + B0(u) (r' - r0) / h_r with
        // r' = sigma r + kappa. Writing f = A + B r gives roots of q B^2 - A^2.
        if dims[1] < 2 {
            return None;
        }
        let r_mid = (q[0] + tm * (q[1] + tm * q[2])).max(0.0).sqrt();
        let (sigma, kappa) = if self.mirrors[1] && r_mid < o[1] {
            (-1.0, 2.0 * o[1])
        } else {
            (1.0, 0.0)
        };
        let rf_mid = sigma * r_mid + kappa;
        let v_mid = (rf_mid - o[1]) / h[1];
        if !(0.0..=(dims[1] - 1) as f64).contains(&v_mid) {
            return None;
        }
        let j0 = (v_mid.floor() as usize).min(dims[1] - 2);
        let r0 = o[1] + j0 as f64 * h[1];
        let u = if dims[0] > 1 { lin[0] } else { [0.0, 0.0] };
        let i0 = idx[0];
        let i1 = if dims[0] > 1 { i0 + 1 } else { i0 };
        let (c00, c10, c01, c11) = (
            value(i0, j0, 0),
            value(i1, j0, 0),
            value(i0, j0 + 1, 0),
            value(i1, j0 + 1, 0),
        );
        let one_minus_u = [1.0 - u[0], -u[1]];
        let a0 = poly_add(&poly_scale(&one_minus_u, c00), &poly_scale(&u, c10));
        let b0 = poly_add(
            &poly_scale(&one_minus_u, c01 - c00),
            &poly_scale(&u, c11 - c10),
        );
        let a = poly_add(&a0, &poly_scale(&b0, (kappa - r0) / h[1]));
        let b = poly_scale(&b0, sigma / h[1]);
        // q in the local parameter: t = t0 + s dt.
        let qs = [
            q[0] + t0 * (q[1] + t0 * q[2]),
            (q[1] + 2.0 * t0 * q[2]) * dt,
            q[2] * dt * dt,
        ];
        let p = poly_add(
            &poly_mul(&qs, &poly_mul(&b, &b)),
            &poly_scale(&poly_mul(&a, &a), -1.0),
        );
        Some(p)
    }
}

fn poly_mul(a: &[f64], b: &[f64]) -> Vec<f64> {
    let mut out = vec![0.0; a.len() + b.len() - 1];
    for (i, x) in a.iter().enumerate() {
        for (j, y) in b.iter().enumerate() {
            out[i + j] += x * y;
        }
    }
    out
}

fn poly_add(a: &[f64], b: &[f64]) -> Vec<f64> {
    let mut out = vec![0.0; a.len().max(b.len())];
    for (i, x) in a.iter().enumerate() {
        out[i] += x;
    }
    for (i, y) in b.iter().enumerate() {
        out[i] += y;
    }
    out
}

fn poly_scale(a: &[f64], s: f64) -> Vec<f64> {
    a.iter().map(|x| x * s).collect()
}

fn poly_eval(p: &[f64], s: f64) -> f64 {
    p.iter().rev().fold(0.0, |acc, c| acc * s + c)
}

/// Sorted, distinct real roots of p in [lo, hi] plus its critical points there
/// (a superset of the sign changes, including double roots): the interval is split
/// at the derivative's roots into monotone pieces and each bracketing piece bisected.
fn poly_roots(p: &[f64], lo: f64, hi: f64) -> Vec<f64> {
    let mut p = p.to_vec();
    while p.len() > 1 && p.last() == Some(&0.0) {
        p.pop();
    }
    if p.len() <= 1 {
        return Vec::new();
    }
    let derivative: Vec<f64> = p
        .iter()
        .enumerate()
        .skip(1)
        .map(|(n, c)| n as f64 * c)
        .collect();
    let critical = poly_roots(&derivative, lo, hi);
    let mut edges = vec![lo];
    edges.extend(critical.iter().copied());
    edges.push(hi);
    let mut roots = critical;
    for w in edges.windows(2) {
        let (mut a, mut b) = (w[0], w[1]);
        let (fa, fb) = (poly_eval(&p, a), poly_eval(&p, b));
        if fa == 0.0 {
            roots.push(a);
        }
        if fb == 0.0 {
            roots.push(b);
        }
        if fa * fb >= 0.0 {
            continue;
        }
        let up = fa < 0.0;
        for _ in 0..80 {
            let m = 0.5 * (a + b);
            if m == a || m == b {
                break;
            }
            if (poly_eval(&p, m) < 0.0) == up {
                a = m;
            } else {
                b = m;
            }
        }
        roots.push(0.5 * (a + b));
    }
    roots.retain(|r| (lo..=hi).contains(r));
    // Recursive callers use these as interval edges, not merely as probes.
    // Appending new roots after the critical points breaks that ordering.
    roots.sort_by(f64::total_cmp);
    roots.dedup();
    roots
}

impl SdfVolume {
    pub fn interpolate(&self, p: Vector) -> f64 {
        let mut low = [0; 3];
        let mut high = [0; 3];
        let mut f = [0.0; 3];
        for a in 0..3 {
            let x =
                ((p[a] - self.origin[a]) / self.spacing[a]).clamp(0.0, (self.dims[a] - 1) as f64);
            low[a] = x.floor() as usize;
            high[a] = (low[a] + 1).min(self.dims[a] - 1);
            f[a] = x - low[a] as f64;
        }
        let mut value = 0.0;
        for z in 0..2 {
            for y in 0..2 {
                for x in 0..2 {
                    let c = [x, y, z];
                    let mut weight = 1.0;
                    let mut pos = [0; 3];
                    for a in 0..3 {
                        pos[a] = if c[a] == 0 { low[a] } else { high[a] };
                        weight *= if c[a] == 0 { 1.0 - f[a] } else { f[a] };
                    }
                    value += weight
                        * self.samples_f64
                            [(pos[2] * self.dims[1] + pos[1]) * self.dims[0] + pos[0]];
                }
            }
        }
        value
    }
}

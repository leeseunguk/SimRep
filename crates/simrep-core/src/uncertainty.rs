// Adapted from using76/Iteration-CFD rust/src/vv.rs (2026-10-27).
// Source: https://github.com/using76/Iteration-CFD/blob/main/rust/src/vv.rs
// Copyright (c) 2026 Iterations Co., Ltd.
// Prosperity Public License 3.0.0; see licenses/THIRD_PARTY.md.
//! Numerical uncertainty: Celik et al. (2008), DOI 10.1115/1.2960953;
//! Eca and Hoekstra (2014), DOI 10.1016/j.jcp.2014.01.006, Appendix A;
//! ASME V&V 20 comparison error and root-sum-square validation uncertainty.
//!
//! Grid levels are finest first. GCI is relative; GridStudy::u_fine is
//! absolute. These are safety-factored uncertainty estimates, not rigorous
//! error bounds or calibrated standard deviations. Oscillatory/diverging
//! triplets deliberately have no Richardson order; fixed fits report them.
//! Equation labels (94.n) refer to the upstream transcription.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    Config(String),
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Config(message) => f.write_str(message),
        }
    }
}
impl std::error::Error for Error {}
pub type Result<T> = std::result::Result<T, Error>;
fn check_levels(levels: &[Level]) -> Result<()> {
    let mut previous = 0.0;
    for level in levels {
        if !level.h.is_finite() || !level.value.is_finite() || level.h <= previous {
            return Err(Error::Config(
                "levels must be finite, positive, and finest first".into(),
            ));
        }
        previous = level.h;
    }
    Ok(())
}

/// One mesh level: its typical cell size (94.1) and the gate's value on it.
/// Level 1 of a sequence is the FINEST mesh; `h` must grow along a sequence.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Level {
    pub h: f64,
    pub value: f64,
}

/// The arithmetic is `f64` whatever `f64` is, so the crate's `single`
/// feature cannot degrade an estimate (the conversion `mms_error` also makes).
fn w(x: f64) -> f64 {
    x
}

/// ...and back.
fn sc(x: f64) -> f64 {
    x
}

/// `f64` to `f64`, refusing what the papers' procedure cannot read.
fn finite(x: f64, what: &str) -> Result<f64> {
    let v = w(x);
    if v.is_finite() {
        Ok(v)
    } else {
        Err(Error::Config(format!("{what}: not finite ({v})")))
    }
}

/// (94.1) - the typical cell size of a uniform mesh: the cube root of the
/// volume per cell in 3-D, the square root of the area per cell in 2-D.
/// `dim` is 2 or 3; anything else is refused by name.
pub fn h_of(total_volume: f64, n_cells: usize, dim: usize) -> Result<f64> {
    match dim {
        2 | 3 => {}
        other => {
            return Err(Error::Config(format!(
                "h_of: dim must be 2 or 3, got {other}"
            )));
        }
    }
    if n_cells == 0 {
        return Err(Error::Config("h_of: n_cells is zero".to_string()));
    }
    let v = finite(total_volume, "h_of: total_volume")?;
    if v <= 0.0 {
        return Err(Error::Config(format!(
            "h_of: total_volume is {v}, not positive"
        )));
    }
    let per_cell = v / n_cells as f64;
    Ok(sc(if dim == 3 {
        per_cell.cbrt()
    } else {
        per_cell.sqrt()
    }))
}

/// Classification of the finest triplet. For unequal ratios a positive-order
/// power law can have R > 1; monotonicity uses the unequal-ratio equation,
/// not the equal-ratio R < 1 shortcut.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Behaviour {
    /// A positive-order monotone power law exists.
    Monotone,
    /// `R > 1`: differences grow monotonically - the mesh sequence is moving
    /// away from the answer.
    MonotoneDiverging,
    /// `R < 0`, `|R| < 1`: alternating signs, shrinking.
    Oscillatory,
    /// `R < 0`, `|R| > 1`: alternating signs, growing.
    OscillatoryDiverging,
    /// A zero difference makes an order unidentifiable.
    Indeterminate,
}

impl Behaviour {
    /// Lower-case words for [`GridStudy::one_line`].
    fn words(self) -> &'static str {
        match self {
            Behaviour::Monotone => "monotone",
            Behaviour::MonotoneDiverging => "monotone diverging",
            Behaviour::Oscillatory => "oscillatory",
            Behaviour::OscillatoryDiverging => "oscillatory diverging",
            Behaviour::Indeterminate => "indeterminate",
        }
    }
}

/// (94.2)-(94.5) on the finest three levels. `p`, `phi_ext` and `gci_fine`
/// are `Some` only for [`Behaviour::Monotone`] - for any other behaviour the
/// order "cannot be established" (Eça & Hoekstra 2014, their section 2.4.2)
/// and the uncertainty comes from the least-squares fits of (94.6)-(94.9)
/// instead.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Triplet {
    pub r21: f64,
    pub r32: f64,
    pub eps21: f64,
    pub eps32: f64,
    /// `R = eps21/eps32` of (94.2).
    pub ratio: Option<f64>,
    pub behaviour: Behaviour,
    pub p: Option<f64>,
    pub phi_ext: Option<f64>,
    /// `e_a = |(phi_1 - phi_2)/phi_1|`; None when phi_1 is zero.
    pub e_a: Option<f64>,
    pub gci_fine: Option<f64>,
    /// Bisection iterations; 0 when there was no order to find.
    pub iterations: usize,
}

/// (94.2)-(94.5), Celik et al. (2008) §2 steps 1-5, `levels[0]` finest.
///
/// Invalid grid sizes or non-finite data are refused. A zero finest value
/// still permits an order and Richardson limit, but its relative GCI is None.
pub fn observed_order(levels: &[Level; 3]) -> Result<Triplet> {
    let h = [
        finite(levels[0].h, "observed_order: level 1 h")?,
        finite(levels[1].h, "observed_order: level 2 h")?,
        finite(levels[2].h, "observed_order: level 3 h")?,
    ];
    let phi = [
        finite(levels[0].value, "observed_order: level 1 value")?,
        finite(levels[1].value, "observed_order: level 2 value")?,
        finite(levels[2].value, "observed_order: level 3 value")?,
    ];
    if !(0.0 < h[0] && h[0] < h[1] && h[1] < h[2]) {
        return Err(Error::Config(format!(
            "observed_order: h is not strictly increasing fine to coarse: {}, {}, {}",
            h[0], h[1], h[2]
        )));
    }
    let eps21 = phi[0] - phi[1];
    let eps32 = phi[1] - phi[2];
    if !eps21.is_finite() || !eps32.is_finite() {
        return Err(Error::Config("grid differences overflow".into()));
    }
    let r21 = h[1] / h[0];
    let r32 = h[2] / h[1];
    let ratio = (eps32 != 0.0).then_some(eps21 / eps32);
    let behaviour = if eps21 == 0.0 || eps32 == 0.0 {
        Behaviour::Indeterminate
    } else if eps21 / eps32 > 0.0 {
        if eps32 / eps21 > r32.ln() / r21.ln() {
            Behaviour::Monotone
        } else {
            Behaviour::MonotoneDiverging
        }
    } else if (eps21 / eps32).abs() < 1.0 {
        Behaviour::Oscillatory
    } else {
        Behaviour::OscillatoryDiverging
    };
    let e_a = (phi[0] != 0.0).then_some((eps21 / phi[0]).abs());
    let mut p = None;
    let mut phi_ext = None;
    let mut gci = None;
    let mut iterations = 0usize;
    if behaviour == Behaviour::Monotone {
        // Solve Celik's unequal-ratio equation in a bracket. Its logarithm
        // is increasing for p>0; unlike fixed-point iteration this also works
        // when the finest refinement ratio is much larger than the coarse one.
        let target = (eps32 / eps21).ln();
        let equation =
            |p: f64| p * r21.ln() + (p * r32.ln()).exp_m1().ln() - (p * r21.ln()).exp_m1().ln();
        let (mut low, mut high) = (0.0, 1.0);
        while equation(high) < target && high < 1024.0 {
            high *= 2.0;
        }
        if !equation(high).is_finite() || equation(high) < target {
            return Err(Error::Config(
                "observed order could not be bracketed".into(),
            ));
        }
        for k in 0..100 {
            let middle = (low + high) / 2.0;
            if equation(middle) < target {
                low = middle;
            } else {
                high = middle;
            }
            iterations = k + 1;
            if high - low < 1e-12 {
                break;
            }
        }
        let pk = (low + high) / 2.0;
        // (94.4) and (94.5), F_s = 1.25 - Celik's three-level value, the only
        // kind this crate makes (SPEC-LIT 94.1).
        let rp = r21.powf(pk);
        p = Some(sc(pk));
        phi_ext = Some(sc((rp * phi[0] - phi[1]) / (rp - 1.0)));
        gci = e_a.map(|error| 1.25 * error / (rp - 1.0));
    }
    Ok(Triplet {
        r21: sc(r21),
        r32: sc(r32),
        eps21: sc(eps21),
        eps32: sc(eps32),
        ratio,
        behaviour,
        p,
        phi_ext,
        e_a,
        gci_fine: gci,
        iterations,
    })
}

/// (94.4) for a given `p`: the Richardson estimate of the exact value from
/// the finest two levels, `phi_ext = (r21^p phi_1 - phi_2) / (r21^p - 1)`.
pub fn richardson_extrapolate(fine: Level, medium: Level, p: f64) -> f64 {
    let rp = (w(medium.h) / w(fine.h)).powf(w(p));
    sc((rp * w(fine.value) - w(medium.value)) / (rp - 1.0))
}

/// (94.5) for a given `p` and safety factor:
/// `GCI_fine = F_s |(phi_1 - phi_2)/phi_1| / (r21^p - 1)`.
pub fn gci_fine(fine: Level, medium: Level, p: f64, fs: f64) -> f64 {
    let rp = (w(medium.h) / w(fine.h)).powf(w(p));
    let e_a = ((w(fine.value) - w(medium.value)) / w(fine.value)).abs();
    sc(w(fs) * e_a / (rp - 1.0))
}

/// Which of (94.6a)'s error forms a least-squares fit used. `PowerSeries` is
/// `eps_RE = alpha h^p` with `p` fitted (the outer search of Appendix B);
/// the other three fix the exponent, which is how an oscillatory or a
/// scattered sequence, or an observed order outside `[0.5, 2]`, still gets
/// an uncertainty (Eça & Hoekstra 2014, their section 2.4.2 and Appendix A).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Estimator {
    PowerSeries,
    First,
    Second,
    FirstSecond,
}

impl Estimator {
    /// (94.7)'s `n_par`, counting `phi_0`: 3 for `PowerSeries`
    /// (`phi_0, alpha, p`) and `FirstSecond` (`phi_0, alpha_1, alpha_2`),
    /// 2 for `First` and `Second`. A fit on `n_g` levels has no residual
    /// freedom left when `n_g - n_par <= 0` and is not attempted.
    fn n_par(self) -> usize {
        match self {
            Estimator::First | Estimator::Second => 2,
            Estimator::PowerSeries | Estimator::FirstSecond => 3,
        }
    }

    /// Lower-case words for [`GridStudy::one_line`].
    fn words(self) -> &'static str {
        match self {
            Estimator::PowerSeries => "power series",
            Estimator::First => "first order",
            Estimator::Second => "second order",
            Estimator::FirstSecond => "first+second order",
        }
    }
}

/// One least-squares fit of (94.6): `phi_i ~ phi_0 + eps(h_i)`. `alpha[1]` is
/// used by [`Estimator::FirstSecond`] only; `p` by [`Estimator::PowerSeries`]
/// only. `fitted` is the fit evaluated at every level given, same order.
#[derive(Debug, Clone, PartialEq)]
pub struct Fit {
    pub estimator: Estimator,
    pub weighted: bool,
    pub phi0: f64,
    pub alpha: [f64; 2],
    pub p: Option<f64>,
    /// (94.7): the standard deviation of the fit, over `n_g - n_par` degrees
    /// of freedom. Exact-zero on the three-level `PowerSeries` path, where
    /// the fit IS the triplet solution (94.3)-(94.4).
    pub sigma: f64,
    pub fitted: Vec<f64>,
}

/// (94.6c): `w_i = 1` unweighted, or `w_i = (1/h_i)/sum_j (1/h_j)` weighted;
/// `nw_i` is what (94.7)'s sum weights with - 1, or `n_g w_i`.
fn weights(levels: &[Level], weighted: bool) -> (Vec<f64>, Vec<f64>) {
    if !weighted {
        let n = levels.len();
        (vec![1.0; n], vec![1.0; n])
    } else {
        let inv: Vec<f64> = levels.iter().map(|l| 1.0 / w(l.h)).collect();
        let sum: f64 = inv.iter().sum();
        let nw_factor = levels.len() as f64;
        let wts: Vec<f64> = inv.iter().map(|x| x / sum).collect();
        let nws: Vec<f64> = wts.iter().map(|x| nw_factor * x).collect();
        (wts, nws)
    }
}

/// The design row `[1, eps-feature(s)]` of (94.6b) for `est` at `h`; `p` is
/// read only by `PowerSeries`. A `FirstSecond` row has three entries, the
/// rest two.
fn design(est: Estimator, h: f64, p: f64) -> [f64; 3] {
    match est {
        Estimator::PowerSeries => [1.0, h.powf(p), 0.0],
        Estimator::First => [1.0, h, 0.0],
        Estimator::Second => [1.0, h * h, 0.0],
        Estimator::FirstSecond => [1.0, h, h * h],
    }
}

fn n_cols(est: Estimator) -> usize {
    match est {
        Estimator::FirstSecond => 3,
        _ => 2,
    }
}

/// Gaussian elimination with partial pivoting, `n <= 3` - the weighted
/// normal equations of (94.6b) are at most 3x3. `None` when singular.
fn gauss_small(a: &mut [[f64; 3]; 3], b: &mut [f64; 3], n: usize) -> Option<[f64; 3]> {
    for col in 0..n {
        let piv = (col..n).fold(col, |best, r| {
            if a[r][col].abs() > a[best][col].abs() {
                r
            } else {
                best
            }
        });
        if a[piv][col] == 0.0 {
            return None;
        }
        a.swap(col, piv);
        b.swap(col, piv);
        for r in col + 1..n {
            let fct = a[r][col] / a[col][col];
            let pivot = a[col];
            for (entry, coefficient) in a[r][col..n].iter_mut().zip(&pivot[col..n]) {
                *entry -= fct * coefficient;
            }
            b[r] -= fct * b[col];
        }
    }
    let mut x = [0.0; 3];
    for r in (0..n).rev() {
        let mut acc = b[r];
        for cc in r + 1..n {
            acc -= a[r][cc] * x[cc];
        }
        x[r] = acc / a[r][r];
    }
    Some(x)
}

/// The raw product of one weighted least-squares solve: the coefficients, the
/// fit at every level, (94.6b)'s weighted SSR `S_X`, and the same sum with
/// (94.7)'s `nw_i` weights.
struct FitRaw {
    phi0: f64,
    alpha: [f64; 2],
    fitted: Vec<f64>,
    s_w: f64,
    s_nw: f64,
}

/// One solve of (94.6b) for `est` (at exponent `p`, read by `PowerSeries`
/// only). `None` when the normal equations are singular.
fn raw_fit(levels: &[Level], est: Estimator, p: f64, weighted: bool) -> Option<FitRaw> {
    let (wt, nw) = weights(levels, weighted);
    let n = n_cols(est);
    let mut a = [[0.0f64; 3]; 3];
    let mut b = [0.0f64; 3];
    for (i, l) in levels.iter().enumerate() {
        let dsn = design(est, w(l.h), p);
        for j in 0..n {
            b[j] += wt[i] * dsn[j] * w(l.value);
            for k in 0..n {
                a[j][k] += wt[i] * dsn[j] * dsn[k];
            }
        }
    }
    let coef = gauss_small(&mut a, &mut b, n)?;
    let mut fitted = Vec::with_capacity(levels.len());
    let mut s_w = 0.0;
    let mut s_nw = 0.0;
    for (i, l) in levels.iter().enumerate() {
        let dsn = design(est, w(l.h), p);
        let mut v = 0.0;
        for j in 0..n {
            v += coef[j] * dsn[j];
        }
        let r = w(l.value) - v;
        s_w += wt[i] * r * r;
        s_nw += nw[i] * r * r;
        fitted.push(v);
    }
    Some(FitRaw {
        phi0: coef[0],
        alpha: [coef[1], coef[2]],
        fitted,
        s_w,
        s_nw,
    })
}

/// (94.7) with `nw_i` from (94.6c). The caller has refused `n_g - n_par <= 0`.
fn sigma_of(s_nw: f64, n_g: usize, n_par: usize) -> f64 {
    (s_nw / (n_g - n_par) as f64).sqrt()
}

/// The MAGNITUDE of the chosen fit's error term (94.6a) at `h` - the
/// `|eps(phi_1)|` that (94.9) and `eps_fine` are stated in.
fn eps_term(fit: &Fit, h: f64) -> f64 {
    match fit.estimator {
        Estimator::PowerSeries => {
            (w(fit.alpha[0]) * h.powf(w(fit.p.expect("power series carries p")))).abs()
        }
        Estimator::First => (w(fit.alpha[0]) * h).abs(),
        Estimator::Second => (w(fit.alpha[0]) * h * h).abs(),
        Estimator::FirstSecond => (w(fit.alpha[0]) * h + w(fit.alpha[1]) * h * h).abs(),
    }
}

/// Golden-section search for the minimum of `f` on `[lo, hi]`, `iters`
/// iterations, returning the final bracket midpoint. DESIGN: this is the
/// outer minimisation of `S_RE` over `p` in (94.6b) - for fixed `p` the fit
/// is linear in `(phi_0, alpha)`, so the inner problem is one 2x2 weighted
/// normal-equations solve, and the search reaches the minimiser Eça &
/// Hoekstra reach through `dS/dp = 0` (their Appendix B.2) without ever
/// dividing by a vanishing derivative.
fn golden(f: impl Fn(f64) -> f64, lo: f64, hi: f64, iters: usize) -> f64 {
    let invphi = (5.0_f64.sqrt() - 1.0) / 2.0;
    let (mut a, mut b) = (lo, hi);
    let mut c = b - invphi * (b - a);
    let mut d = a + invphi * (b - a);
    let mut fc = f(c);
    let mut fd = f(d);
    for _ in 0..iters {
        if fc < fd {
            b = d;
            d = c;
            fd = fc;
            c = b - invphi * (b - a);
            fc = f(c);
        } else {
            a = c;
            c = d;
            fc = fd;
            d = a + invphi * (b - a);
            fd = f(d);
        }
    }
    (a + b) / 2.0
}

/// (94.6)-(94.7) with [`Estimator::PowerSeries`].
///
/// On exactly three levels the fit is the exact triplet solution - `p` from
/// [`observed_order`], `phi_0 = phi_ext` of (94.4), `alpha = (phi_1 -
/// phi_ext)/h_1^p`, `sigma = 0`, `weighted` recorded but unused - and a
/// non-monotone triplet is refused by name. On four or more levels `p` comes
/// from the golden section over `[0.05, 8]` (see [`golden`]). Fewer than
/// three levels is refused by name.
pub fn fit_power_series(levels: &[Level], weighted: bool) -> Result<Fit> {
    check_levels(levels)?;
    if levels.len() < 3 {
        return Err(Error::Config(format!(
            "fit_power_series: {} levels; three are the minimum (SPEC-LIT 94.1)",
            levels.len()
        )));
    }
    if levels.len() == 3 {
        let trip = observed_order(&[levels[0], levels[1], levels[2]])?;
        if trip.behaviour != Behaviour::Monotone {
            return Err(Error::Config(format!(
                "fit_power_series: the finest triplet is {:?} - (94.3) has no order to fit",
                trip.behaviour
            )));
        }
        let p = w(trip.p.expect("monotone triplet carries p"));
        let phi0 = w(trip.phi_ext.expect("monotone triplet carries phi_ext"));
        let a0 = (w(levels[0].value) - phi0) / w(levels[0].h).powf(p);
        let fitted: Vec<f64> = levels
            .iter()
            .map(|l| sc(phi0 + a0 * w(l.h).powf(p)))
            .collect();
        return Ok(Fit {
            estimator: Estimator::PowerSeries,
            weighted,
            phi0: sc(phi0),
            alpha: [sc(a0), sc(0.0)],
            p: Some(sc(p)),
            sigma: sc(0.0),
            fitted,
        });
    }
    let objective = |p: f64| -> f64 {
        raw_fit(levels, Estimator::PowerSeries, p, weighted)
            .map(|r| r.s_w)
            .unwrap_or(f64::INFINITY)
    };
    let p = golden(objective, 0.05, 8.0, 200);
    let raw = raw_fit(levels, Estimator::PowerSeries, p, weighted).ok_or_else(|| {
        Error::Config("fit_power_series: singular normal equations at the fitted p".to_string())
    })?;
    let sigma = sigma_of(raw.s_nw, levels.len(), Estimator::PowerSeries.n_par());
    Ok(Fit {
        estimator: Estimator::PowerSeries,
        weighted,
        phi0: sc(raw.phi0),
        alpha: [sc(raw.alpha[0]), sc(0.0)],
        p: Some(sc(p)),
        sigma: sc(sigma),
        fitted: raw.fitted.into_iter().map(sc).collect(),
    })
}

/// (94.6)-(94.7) with a fixed-exponent estimator - [`Estimator::First`],
/// [`Estimator::Second`] or [`Estimator::FirstSecond`]. Refuses
/// [`Estimator::PowerSeries`] (that is [`fit_power_series`]'s job) and a fit
/// with `n_g - n_par <= 0`, both by name.
pub fn fit_fixed(levels: &[Level], estimator: Estimator, weighted: bool) -> Result<Fit> {
    check_levels(levels)?;
    if estimator == Estimator::PowerSeries {
        return Err(Error::Config(
            "fit_fixed: PowerSeries has a fitted exponent - call fit_power_series".to_string(),
        ));
    }
    let n = levels.len();
    let n_par = estimator.n_par();
    if n <= n_par {
        return Err(Error::Config(format!(
            "fit_fixed: {n} levels leave no residual freedom for {estimator:?} \
             (n_g - n_par = {n} - {n_par} <= 0)"
        )));
    }
    let raw = raw_fit(levels, estimator, 0.0, weighted)
        .ok_or_else(|| Error::Config("fit_fixed: singular normal equations".to_string()))?;
    let sigma = sigma_of(raw.s_nw, n, n_par);
    Ok(Fit {
        estimator,
        weighted,
        phi0: sc(raw.phi0),
        alpha: [sc(raw.alpha[0]), sc(raw.alpha[1])],
        p: None,
        sigma: sc(sigma),
        fitted: raw.fitted.into_iter().map(sc).collect(),
    })
}

/// What a multi-level gate reports: the whole of Eça & Hoekstra's Appendix A,
/// evaluated at the finest level. `behaviour` and `triplet` are always the
/// finest THREE levels' (94.2)-(94.5); `estimator`/`weighted`/`phi_ext`/
/// `sigma` name the chosen fit; `p` is the observed order - the triplet's
/// whenever the triplet is monotone (reported even when a fixed-exponent
/// estimator carries the uncertainty), `None` otherwise; `eps_fine` is
/// `|eps(phi_1)|` of the chosen fit; `fs` by (94.8); `u_fine` by (94.9).
#[derive(Debug, Clone, PartialEq)]
pub struct GridStudy {
    pub n_levels: usize,
    pub behaviour: Behaviour,
    pub estimator: Estimator,
    pub weighted: bool,
    pub p: Option<f64>,
    /// The chosen fit's `phi_0` - the Richardson estimate on the
    /// `PowerSeries` path.
    pub phi_ext: f64,
    pub eps_fine: f64,
    pub sigma: f64,
    pub delta_phi: f64,
    pub fs: f64,
    pub u_fine: f64,
    pub triplet: Triplet,
}

/// The fixed-exponent candidates of Appendix A step 1, in the tie-breaking
/// order (unweighted before weighted, `eps_1` before `eps_2` before
/// `eps_12`); `eps_12` only when its degrees of freedom are positive.
fn fixed_candidates(levels: &[Level], with_twelve: bool) -> Result<Vec<Fit>> {
    let n = levels.len();
    let mut fits = Vec::new();
    for weighted in [false, true] {
        fits.push(fit_fixed(levels, Estimator::First, weighted)?);
        fits.push(fit_fixed(levels, Estimator::Second, weighted)?);
        if with_twelve && n > Estimator::FirstSecond.n_par() {
            fits.push(fit_fixed(levels, Estimator::FirstSecond, weighted)?);
        }
    }
    Ok(fits)
}

/// Smallest sigma; exact ties keep the earlier fit (the candidates' order).
fn smallest(fits: Vec<Fit>) -> Fit {
    fits.into_iter()
        .reduce(|a, b| if w(b.sigma) < w(a.sigma) { b } else { a })
        .expect("at least one candidate fit")
}

/// The whole of Appendix A on `levels`, `levels[0]` finest, `n_g >= 3`
/// (refused below). `triplet` is always the finest three levels'
/// (94.2)-(94.5); `phi_ext` is the chosen fit's `phi_0`; `eps_fine` is
/// `|eps(phi_1)|` of the chosen fit; `fs` by (94.8); `u_fine` by (94.9a)
/// when `sigma < delta_phi`, else (94.9b).
pub fn grid_study(levels: &[Level]) -> Result<GridStudy> {
    let n = levels.len();
    if n < 3 {
        return Err(Error::Config(format!(
            "grid_study: {} levels; three are the minimum (SPEC-LIT 94.1)",
            n
        )));
    }
    for (i, l) in levels.iter().enumerate() {
        finite(l.h, &format!("grid_study: level {} h", i + 1))?;
        finite(l.value, &format!("grid_study: level {} value", i + 1))?;
    }
    check_levels(levels)?;
    if levels.iter().all(|l| l.value == levels[0].value) {
        return Err(Error::Config(
            "constant grid data do not identify an order or uncertainty".into(),
        ));
    }
    let trip = observed_order(&[levels[0], levels[1], levels[2]])?;
    // Appendix A step 1: the error estimate.
    let (chosen, study_p) = if trip.behaviour != Behaviour::Monotone {
        // p "cannot be established": the fixed-exponent fits carry it.
        let best = smallest(fixed_candidates(levels, true)?);
        (best, None)
    } else {
        let re_u = fit_power_series(levels, false)?;
        let re_w = fit_power_series(levels, true)?;
        let in_range = |fit: &Fit| matches!(fit.p, Some(p) if 0.5 <= w(p) && w(p) <= 2.0);
        let (u_in, w_in) = (in_range(&re_u), in_range(&re_w));
        if u_in || w_in {
            let best = match (u_in, w_in) {
                (true, true) => {
                    if w(re_u.sigma) <= w(re_w.sigma) {
                        re_u
                    } else {
                        re_w
                    }
                }
                (true, false) => re_u,
                (false, true) => re_w,
                (false, false) => unreachable!("a fit is in range"),
            };
            let p = best.p.expect("in-range power-series fit carries p");
            (best, Some(p))
        } else {
            // Both RE fits out of range: p* from the better of them, then
            // four fixed fits when p* > 2, six when p* < 0.5.
            let pstar = if w(re_u.sigma) <= w(re_w.sigma) {
                w(re_u.p.expect("power-series fit carries p"))
            } else {
                w(re_w.p.expect("power-series fit carries p"))
            };
            let best = smallest(fixed_candidates(levels, pstar < 0.5)?);
            (best, Some(sc(pstar)))
        }
    };
    // Step 2: the data-range parameter (94.8), over ALL levels.
    let mut lo = f64::INFINITY;
    let mut hi = f64::NEG_INFINITY;
    for l in levels {
        let v = w(l.value);
        lo = lo.min(v);
        hi = hi.max(v);
    }
    let delta_phi = (hi - lo) / (n - 1) as f64;
    // Step 3: the safety factor (94.8) - 1.25 only on the reliable path.
    let fs = if chosen.estimator == Estimator::PowerSeries
        && matches!(chosen.p, Some(p) if 0.5 <= w(p) && w(p) < 2.1)
        && w(chosen.sigma) < delta_phi
    {
        1.25
    } else {
        3.0
    };
    // Step 4: the uncertainty (94.9a)/(94.9b), at the FINEST level.
    let ef = eps_term(&chosen, w(levels[0].h));
    let df = (w(levels[0].value) - w(chosen.fitted[0])).abs();
    let sg = w(chosen.sigma);
    let u = if sg < delta_phi {
        fs * ef + sg + df
    } else {
        3.0 * (sg / delta_phi) * (ef + sg + df)
    };
    Ok(GridStudy {
        n_levels: n,
        behaviour: trip.behaviour,
        estimator: chosen.estimator,
        weighted: chosen.weighted,
        p: study_p,
        phi_ext: chosen.phi0,
        eps_fine: sc(ef),
        sigma: chosen.sigma,
        delta_phi: sc(delta_phi),
        fs: sc(fs),
        u_fine: sc(u),
        triplet: trip,
    })
}

impl GridStudy {
    /// One line for a note or the summary, e.g.
    /// `3 levels, monotone, p = 2.145, phi_ext = 1.521650e0, U_fine = 3.747e-3 (Fs 3, second order, unweighted)`
    /// - `p = n/a` when `None`; behaviour and estimator in words.
    pub fn one_line(&self) -> String {
        let p = match self.p {
            Some(p) => format!("{:.3}", w(p)),
            None => "n/a".to_string(),
        };
        format!(
            "{} levels, {}, p = {}, phi_ext = {:.6e}, U_fine = {:.3e} (Fs {}, {}, {})",
            self.n_levels,
            self.behaviour.words(),
            p,
            w(self.phi_ext),
            w(self.u_fine),
            w(self.fs),
            self.estimator.words(),
            if self.weighted {
                "weighted"
            } else {
                "unweighted"
            },
        )
    }
}

/// (94.10) - Coleman & Stern's validation metric: the comparison error
/// `E = S - D` and the validation uncertainty `u_val` as the root-sum-square
/// of the numerical, input and datum uncertainties; `u_num` is (94.9)'s
/// `U(phi_1)`, the study's `u_fine`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Validation {
    /// The simulation's finest-level value.
    pub s: f64,
    /// The reference datum.
    pub d: f64,
    pub e: f64,
    pub u_num: f64,
    pub u_input: f64,
    pub u_d: f64,
    pub u_val: f64,
}

/// (94.10): `E = S - D`, `u_val = sqrt(u_num^2 + u_input^2 + u_D^2)`.
pub fn validation(s: f64, d: f64, u_num: f64, u_input: f64, u_d: f64) -> Validation {
    assert!([s, d, u_num, u_input, u_d].into_iter().all(f64::is_finite));
    assert!(u_num >= 0.0 && u_input >= 0.0 && u_d >= 0.0);
    let e = w(s) - w(d);
    let u_val = (w(u_num) * w(u_num) + w(u_input) * w(u_input) + w(u_d) * w(u_d)).sqrt();
    Validation {
        s,
        d,
        e: sc(e),
        u_num,
        u_input,
        u_d,
        u_val: sc(u_val),
    }
}

impl Validation {
    /// e.g. `Nu at Kr = 1: E = S - D = -4.710e-2 +/- 3.747e-3 (-3.00 % +/- 0.24 % of D; u_num 3.747e-3, u_input 0, u_D 0): |E| > u_val, the disagreement is not the mesh`
    /// - the last clause is `|E| <= u_val, the comparison cannot see a
    /// modelling error at this resolution` in the other case.
    pub fn one_line(&self, what: &str) -> String {
        let num = |x: f64| -> String {
            let v = w(x);
            if v == 0.0 {
                "0".to_string()
            } else {
                format!("{v:.3e}")
            }
        };
        let clause = if w(self.e).abs() <= w(self.u_val) {
            "|E| <= u_val, the comparison cannot see a modelling error at this resolution"
        } else {
            "|E| > u_val, the disagreement is not the mesh"
        };
        let pct = if w(self.d) != 0.0 {
            format!(
                "({:.2} % +/- {:.2} % of D; ",
                100.0 * w(self.e) / w(self.d),
                100.0 * w(self.u_val) / w(self.d)
            )
        } else {
            "(- % +/- - % of D; ".to_string()
        };
        format!(
            "{}: E = S - D = {:.3e} +/- {:.3e} {}u_num {}, u_input {}, u_D {}): {}",
            what,
            w(self.e),
            w(self.u_val),
            pct,
            num(self.u_num),
            num(self.u_input),
            num(self.u_d),
            clause,
        )
    }
}

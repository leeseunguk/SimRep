use simrep_core::{PotentialArray, validation::record_order};
use simrep_fly::{Field, GridField, Reconstruction};

fn errors(n: usize, boundary: bool) -> (f64, f64) {
    let h = 1.0 / (n - 1) as f64;
    let mut pa = PotentialArray::new(n, n, 1);
    pa.dx_mm = h;
    pa.dy_mm = h;
    for y in 0..n {
        for x in 0..n {
            let i = pa.index(x, y, 0).unwrap();
            pa.potentials[i] = (x as f64 * h).exp() * (y as f64 * h).cos();
            // Outer planes activate the boundary-aware reconstruction without
            // introducing an error from a potential solve.
            pa.electrodes[i] = boundary && (x == 0 || x + 1 == n || y == 0 || y + 1 == n);
        }
    }
    let field = GridField::with_reconstruction(
        pa,
        if boundary {
            Reconstruction::BoundaryAware
        } else {
            Reconstruction::GlobalCubic
        },
    )
    .unwrap();
    let mut potential = 0.0_f64;
    let mut electric = 0.0_f64;
    // Fixed physical probes, independent of grid; exclude exterior boundary
    // closure so this isolates the documented vacuum reconstruction order.
    for iy in 0..19 {
        for ix in 0..19 {
            let (x, y) = (
                0.2 + 0.6 * (ix as f64 + 0.37) / 19.0,
                0.2 + 0.6 * (iy as f64 + 0.23) / 19.0,
            );
            let exact = x.exp() * y.cos();
            let sample = field.sample([x, y, 0.0], 0.0);
            potential = potential.max((sample.potential_v - exact).abs());
            electric = electric
                .max((sample.electric_v_per_mm[0] + exact).abs())
                .max((sample.electric_v_per_mm[1] - x.exp() * y.sin()).abs());
        }
    }
    (potential, electric)
}

#[test]
fn field_reconstruction_mms_orders() {
    for (boundary, name) in [(false, "mms-global-cubic"), (true, "mms-boundary-aware")] {
        let errors = [17, 33, 65].map(|n| errors(n, boundary));
        record_order(
            &format!("{name}-potential"),
            "equation (7), exact analytic nodal data",
            errors.map(|e| e.0),
            4.0,
        );
        record_order(
            &format!("{name}-field"),
            "equation (7), interior vacuum gradient",
            errors.map(|e| e.1),
            3.0,
        );
        // These are interpolation-only studies; no solve or flight error is
        // present. A lower measured order is retained in the gates table.
        assert!(errors[2].0 < errors[1].0 && errors[1].0 < errors[0].0);
        assert!(errors[2].1 < errors[1].1 && errors[1].1 < errors[0].1);
    }
}

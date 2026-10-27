use simrep_core::uncertainty::{
    Behaviour, Estimator, Level, fit_power_series, grid_study, h_of, observed_order, validation,
};

#[test]
fn celik_2008_worked_examples_table_one() {
    // Celik et al., JFE 130 (2008) 078001, Table 1. Use N-derived sizes,
    // not the rounded printed refinement ratios.
    for (cells, phi, p, extrapolated, gci_percent) in [
        (
            [18000, 8000, 4500],
            [6.063, 5.972, 5.863],
            1.53,
            6.1685,
            2.2,
        ),
        (
            [18000, 4500, 980],
            [10.7880, 10.7250, 10.6050],
            0.75,
            10.8801,
            1.1,
        ),
    ] {
        let levels = std::array::from_fn(|i| Level {
            h: h_of(1.0, cells[i], 2).unwrap(),
            value: phi[i],
        });
        let result = observed_order(&levels).unwrap();
        println!("Celik: {result:?}");
        assert!((result.p.unwrap() - p).abs() < 0.005);
        assert!((result.phi_ext.unwrap() - extrapolated).abs() < 0.0001);
        assert!((result.gci_fine.unwrap() * 100.0 - gci_percent).abs() < 0.05);
    }
}

#[test]
fn exact_power_unequal_ratios_and_richardson() {
    let levels = [0.125_f64, 0.2, 0.5].map(|h| Level {
        h,
        value: 2.0 + 0.7 * h.powf(1.7),
    });
    let result = observed_order(&levels).unwrap();
    assert!((result.p.unwrap() - 1.7).abs() < 1e-10);
    assert!((result.phi_ext.unwrap() - 2.0).abs() < 1e-12);
    assert!(
        (result.gci_fine.unwrap() * levels[0].value - 1.25 * (levels[0].value - 2.0)).abs() < 1e-12
    );
}

#[test]
fn unequal_ratios_do_not_misclassify_convergence_and_zero_is_not_nan() {
    let levels = [0.1_f64, 0.5, 0.6].map(|h| Level { h, value: h * h });
    let triplet = observed_order(&levels).unwrap();
    assert!(triplet.ratio.unwrap() > 1.0);
    assert_eq!(triplet.behaviour, Behaviour::Monotone);
    assert!((triplet.p.unwrap() - 2.0).abs() < 1e-10);
    let zero = [0.25_f64, 0.5, 1.0].map(|h| Level {
        h,
        value: h * h - 0.0625,
    });
    let triplet = observed_order(&zero).unwrap();
    assert!((triplet.p.unwrap() - 2.0).abs() < 1e-10);
    assert!(triplet.gci_fine.is_none());
    assert!((triplet.phi_ext.unwrap() + 0.0625).abs() < 1e-12);
    let plateau = [
        Level {
            h: 0.125,
            value: 1.0,
        },
        Level {
            h: 0.25,
            value: 1.0,
        },
        Level { h: 0.5, value: 1.1 },
        Level { h: 1.0, value: 1.3 },
    ];
    let study = grid_study(&plateau).unwrap();
    assert_eq!(study.behaviour, Behaviour::Indeterminate);
    assert!(study.p.is_none());
    assert!(study.u_fine.is_finite() && study.u_fine > 0.0);
    let constant = plateau.map(|l| Level { value: 1.0, ..l });
    assert!(grid_study(&constant).is_err());
}

#[test]
fn least_squares_exact_and_noisy_four_grid_sequences() {
    let hs = [0.125_f64, 0.25, 0.5, 1.0];
    let levels = hs.map(|h| Level {
        h,
        value: 2.0 + 0.7 * h.powf(1.5),
    });
    for weighted in [false, true] {
        let fit = fit_power_series(&levels, weighted).unwrap();
        assert!((fit.p.unwrap() - 1.5).abs() < 1e-8);
        assert!((fit.phi0 - 2.0).abs() < 1e-10);
        assert!(fit.sigma < 1e-12);
    }
    let noisy = std::array::from_fn::<_, 4, _>(|i| Level {
        h: hs[i],
        value: levels[i].value + if i % 2 == 0 { 0.2 } else { -0.2 },
    });
    let study = grid_study(&noisy).unwrap();
    assert_eq!(study.behaviour, Behaviour::Oscillatory);
    assert_eq!(study.p, None);
    assert_eq!(study.estimator, Estimator::First);
    assert_eq!(study.fs, 3.0);
    assert!(study.sigma >= study.delta_phi);
    assert!((study.u_fine - 1.5945).abs() < 0.001);
}

#[test]
fn validation_and_invalid_sequences() {
    let result = validation(1.0, 2.0, 1.5, 0.5, 0.25);
    assert_eq!(result.e, -1.0);
    assert_eq!(result.u_val, (2.25_f64 + 0.25 + 0.0625).sqrt());
    let levels = [0.125_f64, 0.25, 0.5, 1.0].map(|h| Level { h, value: 2.0 + h });
    for bad in [0.0, -1.0, f64::NAN, f64::INFINITY, 0.25] {
        let mut invalid = levels;
        invalid[3].h = bad;
        assert!(grid_study(&invalid).is_err());
        assert!(fit_power_series(&invalid, false).is_err());
    }
}

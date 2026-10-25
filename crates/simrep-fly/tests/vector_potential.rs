use simrep_core::{FieldType, PotentialArray, Symmetry};
use simrep_fly::{
    AnalyticField, Field, FieldSample, FlightOptions, Particle, SplatReason, fly,
    physics::magnetic::{MagneticField, MagneticPa, VectorPotential},
};
use simrep_solve::magnetostatics::Formulation;

fn array(symmetry: Symmetry, f: impl Fn(f64, f64, f64) -> f64) -> PotentialArray {
    let mut a = PotentialArray::new(17, 17, if symmetry == Symmetry::Planar { 17 } else { 1 });
    a.symmetry = symmetry;
    a.field_type = FieldType::Magnetic;
    a.dx_mm = 0.25;
    a.dy_mm = 0.25;
    a.dz_mm = 0.25;
    for (i, v) in a.potentials.iter_mut().enumerate() {
        *v = f(
            (i % 17) as f64 * 0.25,
            (i / 17 % 17) as f64 * 0.25,
            (i / (17 * 17)) as f64 * 0.25,
        );
    }
    a
}

#[test]
fn planar_curl_and_axis_limit_are_c2_consistent_and_superposable() {
    let mut az = array(Symmetry::Planar, |x, y, _| 0.01 * (x * x + y * y));
    az.nz = 1;
    az.potentials.truncate(17 * 17);
    az.electrodes.truncate(17 * 17);
    let az = VectorPotential::new(az, Formulation::PlanarAz, 100.0).unwrap();
    let psi = array(Symmetry::Cylindrical, |x, r, _| {
        r * r * (1.0 + 0.1 * x * x) / 100.0
    });
    let psi = VectorPotential::new(psi, Formulation::AxisymmetricPsi, 100.0).unwrap();
    for r in [0.0, 1e-12, 0.13, 0.5, 1.27] {
        let p = [1.13, r, 0.0];
        let b = psi.sample(p, 0.0).magnetic_gauss;
        assert!(
            (b[0] - 2.0 * (1.0 + 0.1 * p[0] * p[0])).abs() < 1e-8,
            "{b:?}"
        );
        assert!((b[1] + 0.2 * p[0] * r).abs() < 1e-8);
        let planar = az.sample(p, 0.0).magnetic_gauss;
        assert!((planar[0] - 2.0 * r).abs() < 1e-8);
        assert!((planar[1] + 2.0 * p[0]).abs() < 1e-8);
    }
    let field = MagneticField {
        uniform_gauss: [1.0, 2.0, 3.0],
        vector_potentials: vec![az, psi],
        ..Default::default()
    };
    let b = field.sample([1.0, 0.5, 0.0], 0.0).magnetic_gauss;
    assert!(
        (b[0] - 4.2).abs() < 1e-8 && (b[1] + 0.1).abs() < 1e-8 && b[2] == 3.0,
        "{b:?}"
    );
}

#[test]
fn three_component_curl_preserves_component_parity_and_placement() {
    let mut ax = array(Symmetry::Planar, |_, y, z| y * y + z);
    let mut ay = array(Symmetry::Planar, |x, _, z| z * z + x);
    let mut az = array(Symmetry::Planar, |x, y, _| x * x + y);
    // Az is odd in y for a separate exact linear-parity check.
    az.potentials
        .iter_mut()
        .enumerate()
        .for_each(|(i, v)| *v = (i / 17 % 17) as f64 * 0.25);
    az.mirrors.y = true;
    az.anti_mirrors.y = true;
    ax.mirrors.y = true;
    ay.mirrors.y = true;
    // Keep other components even in y, as their definitions require.
    let field = VectorPotential::cartesian(
        [ax, ay, az],
        1.0,
        [3.0, 4.0, 5.0],
        [[0.0, 1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
        2.0,
        None,
    )
    .unwrap();
    // Local p=(1,-0.5,2): B=(1-4,1,1-(-1))=(-3,1,2).
    // Left-handed placement maps an axial vector by det(R)*R.
    let b = field.sample([2.0, 6.0, 9.0], 0.0).magnetic_gauss;
    assert!(
        (b[0] + 1.0).abs() < 1e-8 && (b[1] - 3.0).abs() < 1e-8 && (b[2] + 2.0).abs() < 1e-8,
        "{b:?}"
    );
}

#[test]
fn computed_uniform_psi_field_matches_analytic_flight_on_axis_and_off_axis() {
    let psi = array(Symmetry::Cylindrical, |_, r, _| 0.5 * r * r);
    let field = VectorPotential::new(psi, Formulation::AxisymmetricPsi, 100.0).unwrap();
    let analytic = AnalyticField(|_, _| FieldSample {
        magnetic_gauss: [100.0, 0.0, 0.0],
        ..Default::default()
    });
    let options = FlightOptions {
        duration_us: 0.05,
        initial_step_us: 0.001,
        max_step_us: 0.005,
        ..Default::default()
    };
    for radius in [0.0, 0.3] {
        let ion = Particle::new(1.0, 1.0, [2.0, radius, 0.0], [0.0, 1.0, 0.0], 0.0).unwrap();
        let actual = fly(&field, &ion, &options).unwrap();
        let expected = fly(&analytic, &ion, &options).unwrap();
        assert_eq!(actual.splat_reason, SplatReason::TimeLimit);
        for a in 0..3 {
            assert!(
                (actual.final_state.position_mm[a] - expected.final_state.position_mm[a]).abs()
                    < 1e-9
            );
            assert!(
                (actual.final_state.velocity_mm_per_us[a]
                    - expected.final_state.velocity_mm_per_us[a])
                    .abs()
                    < 1e-9
            );
        }
    }
}

#[test]
fn scalar_material_mask_stops_at_exact_thin_cell_without_changing_vacuum_field() {
    let mut pa = array(Symmetry::Planar, |x, _, _| -x);
    pa.nz = 1;
    pa.potentials.truncate(17 * 17);
    pa.electrodes.truncate(17 * 17);
    let mut cells = vec![false; 16 * 16];
    for y in 0..16 {
        cells[5 + 16 * y] = true;
    }
    let field = MagneticPa::new(pa, 100.0)
        .unwrap()
        .with_material_cells(cells)
        .unwrap();
    assert!(field.has_material_cells());
    assert_eq!(
        field.status([1.3, 1.0, 0.0]),
        Some(SplatReason::Electrode { index: 4 * 17 + 5 })
    );
    let hit = field
        .intersection([0.5, 1.0, 0.0], [3.0, 1.0, 0.0])
        .unwrap();
    assert!((hit.0 - 0.3).abs() < 1e-14);
    assert!((field.sample([1.0, 1.0, 0.0], 0.0).magnetic_gauss[0] - 100.0).abs() < 1e-9);
    let ion = Particle::new(1.0, 0.0, [0.5, 1.0, 0.0], [1.0, 0.0, 0.0], 0.0).unwrap();
    let result = fly(
        &field,
        &ion,
        &FlightOptions {
            duration_us: 3.0,
            max_step_us: 2.0,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(matches!(result.splat_reason, SplatReason::Electrode { .. }));
    assert!((result.final_state.position_mm[0] - 1.25).abs() < 1e-10);
}

use simrep_core::PotentialArray;
use simrep_io::{
    FastScalable, Pa0Metadata, PaFile, parse_fly2, parse_ion, read_fast_adjust, read_pa,
    read_particles, write_pa,
};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "simrep-io-flight-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

fn close(actual: &[f64], expected: &[f64]) {
    assert_eq!(actual.len(), expected.len());
    for (&a, &e) in actual.iter().zip(expected) {
        assert!((a - e).abs() < 1e-9, "{a} != {e}");
    }
}

fn file(potentials: [f64; 9]) -> PaFile {
    let mut array = PotentialArray::new(3, 3, 1);
    array.potentials = potentials.to_vec();
    array.electrodes = vec![true, false, true, false, false, false, true, false, true];
    PaFile::new(array)
}

fn metadata(entries: &[(usize, i64)]) -> Pa0Metadata {
    let mut electrode_indices = vec![-1; 32];
    for &(number, index) in entries {
        electrode_indices[number] = index;
    }
    Pa0Metadata {
        reference_voltage: 1000.0,
        electrode_indices,
        scalable: None,
    }
}

fn write_fast_adjust_fixture(root: &Fixture) -> PathBuf {
    let p1 = file([1000.0, 750.0, 0.0, 600.0, 500.0, 250.0, 0.0, 100.0, 0.0]);
    let p2 = file([0.0, 250.0, 1000.0, 200.0, 400.0, 500.0, 0.0, 600.0, 0.0]);
    let background = file([0.0, 5.0, 0.0, 7.0, -4.0, 2.0, -20.0, 3.0, 10.0]);
    let mut base = background.clone();
    for (i, value) in base.array.potentials.iter_mut().enumerate() {
        *value += 12.0 / 1000.0 * p1.array.potentials[i] - 8.0 / 1000.0 * p2.array.potentials[i];
    }
    base.pa0 = Some(metadata(&[(0, 6), (1, 0), (2, 2)]));
    write_pa(root.path("lens.pa1"), &p1).unwrap();
    write_pa(root.path("lens.pa2"), &p2).unwrap();
    write_pa(root.path("lens.pa_"), &background).unwrap();
    let path = root.path("lens.pa0");
    write_pa(&path, &base).unwrap();
    path
}

#[test]
fn fast_adjust_normalizes_basis_and_preserves_other_voltages_and_background() {
    let fixture = Fixture::new();
    let path = write_fast_adjust_fixture(&fixture);
    let base = read_pa(&path).unwrap().array;
    assert_eq!(read_fast_adjust(&path, &[]).unwrap(), base);
    let adjusted = read_fast_adjust(&path, &[(1, 20.0)]).unwrap();
    let p1 = read_pa(fixture.path("lens.pa1")).unwrap().array;
    let expected = base
        .potentials
        .iter()
        .zip(p1.potentials)
        .map(|(base, basis)| base + 8.0 / 1000.0 * basis)
        .collect::<Vec<_>>();
    close(&adjusted.potentials, &expected);
    assert_eq!(adjusted.electrodes, base.electrodes);
    close(
        &[
            adjusted.potentials[0],
            adjusted.potentials[2],
            adjusted.potentials[6],
            adjusted.potentials[8],
        ],
        &[20.0, -8.0, -20.0, 10.0],
    );
    assert_eq!(read_pa(&path).unwrap().array, base);
}

#[test]
fn fast_adjust_superimposes_multiple_overrides_and_signed_pa_background() {
    let fixture = Fixture::new();
    let path = write_fast_adjust_fixture(&fixture);
    let adjusted = read_fast_adjust(&path, &[(2, 5.0), (0, -40.0), (1, 20.0)]).unwrap();
    let p1 = read_pa(fixture.path("lens.pa1")).unwrap().array;
    let p2 = read_pa(fixture.path("lens.pa2")).unwrap().array;
    let background = read_pa(fixture.path("lens.pa_")).unwrap().array;
    let expected = (0..9)
        .map(|i| {
            2.0 * background.potentials[i]
                + 20.0 / 1000.0 * p1.potentials[i]
                + 5.0 / 1000.0 * p2.potentials[i]
        })
        .collect::<Vec<_>>();
    close(&adjusted.potentials, &expected);
}

#[test]
fn fast_adjust_uses_extended_scalable_voltage_table_for_overlapping_solutions() {
    let fixture = Fixture::new();
    let p1 = file([10.0, 8.0, 20.0, 4.0, 3.0, 2.0, 0.0, 1.0, 0.0]);
    let p2 = file([-4.0, -2.0, 2.0, -1.0, 0.0, 1.0, 0.0, 2.0, 0.0]);
    let mut base = p1.clone();
    for (i, value) in base.array.potentials.iter_mut().enumerate() {
        *value = 40.0 / 20.0 * p1.array.potentials[i] + 12.0 / -4.0 * p2.array.potentials[i];
    }
    let mut info = metadata(&[(1, 0), (2, 2)]);
    let mut voltages = vec![f64::NAN; 32];
    voltages[1] = 40.0;
    voltages[2] = 12.0;
    info.scalable = Some(FastScalable::Voltages(voltages));
    base.pa0 = Some(info);
    write_pa(fixture.path("lens.PA0"), &base).unwrap();
    write_pa(fixture.path("lens.PA1"), &p1).unwrap();
    write_pa(fixture.path("lens.PA2"), &p2).unwrap();
    let adjusted = read_fast_adjust(fixture.path("lens.PA0"), &[(1, 60.0), (2, -8.0)]).unwrap();
    let expected = (0..9)
        .map(|i| 3.0 * p1.array.potentials[i] + 2.0 * p2.array.potentials[i])
        .collect::<Vec<_>>();
    close(&adjusted.potentials, &expected);
}

#[test]
fn fast_adjust_rejects_unknown_duplicate_nonfinite_and_missing_inputs() {
    let fixture = Fixture::new();
    let path = write_fast_adjust_fixture(&fixture);
    let error = read_fast_adjust(&path, &[(3, 1.0)])
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("accepted electrode numbers: [0, 1, 2]"),
        "{error}"
    );
    for overrides in [
        vec![(3, 1.0)],
        vec![(1001, 1.0)],
        vec![(1, f64::NAN)],
        vec![(1, f64::INFINITY)],
        vec![(1, 1.0), (1, 2.0)],
    ] {
        assert_eq!(
            read_fast_adjust(&path, &overrides).unwrap_err().kind(),
            ErrorKind::InvalidData
        );
    }
    let mut base = read_pa(&path).unwrap();
    base.pa0.as_mut().unwrap().reference_voltage = 0.0;
    write_pa(&path, &base).unwrap();
    assert!(read_fast_adjust(&path, &[(1, 2.0)]).is_err());
    base.pa0 = None;
    write_pa(&path, &base).unwrap();
    assert!(read_fast_adjust(&path, &[]).is_err());
    let path = write_fast_adjust_fixture(&fixture);
    std::fs::remove_file(fixture.path("lens.pa2")).unwrap();
    assert_eq!(
        read_fast_adjust(&path, &[(1, 2.0)]).unwrap_err().kind(),
        ErrorKind::NotFound
    );
    assert!(read_fast_adjust(fixture.path("lens.pa1"), &[]).is_err());
}

#[test]
fn fast_adjust_rejects_mismatched_geometry_even_for_unspecified_solutions() {
    let fixture = Fixture::new();
    let path = write_fast_adjust_fixture(&fixture);
    let solution = read_pa(fixture.path("lens.pa2")).unwrap();
    for change in 0..6 {
        let mut changed = solution.clone();
        match change {
            0 => changed.array.dx_mm = 2.0,
            1 => changed.array.mirrors.x = true,
            2 => changed.array.electrodes[1] = true,
            3 => changed.ng += 1,
            4 => changed.array.anti_mirrors.x = true,
            5 => {
                changed.array.nx = 4;
                changed.array.potentials.resize(12, 0.0);
                changed.array.electrodes.resize(12, false);
            }
            _ => unreachable!(),
        }
        write_pa(fixture.path("lens.pa2"), &changed).unwrap();
        let error = read_fast_adjust(&path, &[(1, 15.0)]).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidData);
        assert!(error.to_string().contains("geometry"));
    }
}

#[test]
fn ion_parses_physical_units_comments_scientific_notation_and_optional_fields() {
    let particles = parse_ion(
        "\u{feff}; test\r\n\
         2.5e-3, 4, -2, 1, 2, 3, 90, 0, 1e3, 9, 15 ; source weighting and color\r\n\
         0, 100, 0, -1, 0, 2, 0, 90, 0\r\n\
         0, 1, 1, 0, 0, 0, 0, 0, .1, 1\n",
    )
    .unwrap();
    assert_eq!(particles.len(), 3);
    assert_eq!(particles[0].birth_time_us, 0.0025);
    assert_eq!(particles[0].mass_amu, 4.0);
    assert_eq!(particles[0].charge_e, -2.0);
    assert_eq!(particles[0].energy_ev, 1000.0);
    assert_eq!(particles[0].charge_weight, 9.0);
    assert_eq!(particles[1].charge_weight, 1.0);
    assert_eq!(particles[0].position_mm, [1.0, 2.0, 3.0]);
    close(&particles[0].direction, &[0.0, 0.0, -1.0]);
    close(&particles[1].direction, &[0.0, 1.0, 0.0]);
    close(&particles[2].direction, &[1.0, 0.0, 0.0]);
}

#[test]
fn ion_rejects_bad_fields_and_reports_the_input_line() {
    for row in [
        "0,100,1,0,0,0,0,0",
        "0,100,1,0,0,0,0,0,1,1,0,0",
        "0,0,1,0,0,0,0,0,1",
        "0,-1,1,0,0,0,0,0,1",
        "0,100,1,0,0,0,0,0,-1",
        "0,100,1,0,NaN,0,0,0,1",
        "0,100,1,0,0,0,inf,0,1",
        "0,100,1,0,0,0,0,0,1,garbage",
        "0,100,1,0,0,0,0,0,1,",
        "\"0\",100,1,0,0,0,0,0,1",
    ] {
        let error = parse_ion(&format!(";comment\n{row}")).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidData);
        assert!(error.to_string().contains("line 2"), "{error}");
    }
}

#[test]
fn fly2_parses_multiple_literal_beams_repetition_and_defaults() {
    let particles = parse_fly2(
        "-- literal input\r\nparticles {\n coordinates = 0,\n\
         standard_beam { n=2, tob=1.5, mass=4, charge=-2, ke=2e3,\n\
         position=vector(1,-2,.3), direction=vector(0,3,4), cwf=2, color=15, },\n\
         standard_beam {},\n}",
    )
    .unwrap();
    assert_eq!(particles.len(), 3);
    assert_eq!(particles[0], particles[1]);
    assert_eq!(particles[0].birth_time_us, 1.5);
    assert_eq!(particles[0].mass_amu, 4.0);
    assert_eq!(particles[0].charge_e, -2.0);
    assert_eq!(particles[0].energy_ev, 2000.0);
    assert_eq!(particles[0].charge_weight, 2.0);
    assert_eq!(particles[0].position_mm, [1.0, -2.0, 0.3]);
    close(&particles[0].direction, &[0.0, 0.6, 0.8]);
    assert_eq!(
        particles[2],
        parse_ion("0,0.00054857990946,-1,0,0,0,0,0,0.1").unwrap()[0]
    );
    assert_eq!(
        parse_fly2("particles {standard_beam {n=1; ke=1;};}")
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn fly2_rejects_unavailable_context_unknown_fields_and_invalid_physical_values() {
    for input in [
        "particles { standard_beam {n=sequence {1,2}} }",
        "particles { standard_beam {ke=distribution(fn)} }",
        "particles { coordinates=1, standard_beam {} }",
        "particles { unknown_beam {} }",
        "particles { standard_beam {n=0} }",
        "particles { standard_beam {n=1.5} }",
        "particles { standard_beam {n=1e100} }",
        "particles { standard_beam {mass=0} }",
        "particles { standard_beam {ke=-1} }",
        "particles { standard_beam {position=vector(0,0)} }",
        "particles { standard_beam {ke=1 mass=2} }",
        "particles { standard_beam {",
    ] {
        let error = parse_fly2(input).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidData, "{input}");
        assert!(error.to_string().contains("FLY2"), "{error}");
    }
}

#[test]
fn fly2_executes_lua_sequences_and_signed_weights() {
    let p = parse_fly2(
        "--[[ safe Lua ]] local t={coordinates=0}; \
        for i=1,2 do t[#t+1]=standard_beam { \
        tob=arithmetic_sequence{first=1,last=2,n=2}, \
        x=arithmetic_sequence{first=10,last=20,n=2}, mass=i, charge=1, \
        ke=1+2, cwf=-2, color=7} end; particles(t)",
    )
    .unwrap();
    assert_eq!(p.len(), 8);
    assert_eq!(
        p.iter().map(|p| p.birth_time_us).collect::<Vec<_>>(),
        [1., 2., 1., 2., 1., 2., 1., 2.]
    );
    assert_eq!(p[2].position_mm, [20., 0., 0.]);
    assert_eq!(p[0].charge_weight, -2.);
    assert_eq!(p[0].color, 7.);
    assert_eq!(p[4].mass_amu, 2.);
    assert_eq!(p[0].energy_ev, 3.);
    assert!(parse_fly2("particles {}").unwrap().is_empty());
    assert_eq!(
        parse_fly2("particles {standard_beam {direction=vector(0,0,0)}}").unwrap()[0].direction,
        [1., 0., 0.]
    );
}

#[test]
fn fly2_uses_explicit_instance_transforms_and_field_provider() {
    use simrep_io::{Fly2Context, Fly2Field, Fly2Instance, read_particles_in};
    let fixture = Fixture::new();
    let path = fixture.path("context.fly2");
    std::fs::write(
        &path,
        "local p=simion.wb.instances[1]; particles {coordinates=1, \
        standard_beam {position=vector(2,3,4),direction=vector(1,0,0), \
        ke=p:potential_wc(10,20,30),mass=1,charge=1}}",
    )
    .unwrap();
    let error = read_particles(&path).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("missing workbench instance context")
    );
    let provider = |number, point| {
        assert_eq!(number, 1);
        assert_eq!(point, [10., 20., 30.]);
        Ok(Fly2Field {
            potential_v: 5.,
            ..Default::default()
        })
    };
    let context = Fly2Context {
        instances: vec![Fly2Instance {
            origin_mm: [10., 20., 30.],
            axes: [[0., 1., 0.], [-1., 0., 0.], [0., 0., 1.]],
            scale: 2.,
            spacing_mm: [0.5, 1., 2.],
            ..Default::default()
        }],
        field: Some(&provider),
        ..Default::default()
    };
    let p = read_particles_in(&path, &context).unwrap();
    assert_eq!(p[0].position_mm, [4., 22., 46.]);
    assert_eq!(p[0].direction, [0., 1., 0.]);
    assert_eq!(p[0].energy_ev, 5.);
    let missing = Fly2Context {
        field: None,
        ..context
    };
    assert!(
        read_particles_in(&path, &missing)
            .unwrap_err()
            .to_string()
            .contains("missing field provider context")
    );
}

#[test]
fn fly2_restricts_includes_and_host_capabilities() {
    let fixture = Fixture::new();
    std::fs::write(fixture.path("helper.lua"), "return {energy=12}").unwrap();
    std::fs::write(
        fixture.path("main.fly2"),
        "local h=require 'helper'; particles {standard_beam {ke=h.energy}}",
    )
    .unwrap();
    assert_eq!(
        read_particles(fixture.path("main.fly2")).unwrap()[0].energy_ev,
        12.
    );
    for text in [
        "os.execute('echo bad')",
        "io.open('bad','w')",
        "dofile('/etc/passwd')",
        "require '../outside'",
    ] {
        std::fs::write(fixture.path("bad.fly2"), text).unwrap();
        assert!(read_particles(fixture.path("bad.fly2")).is_err());
    }
}

#[test]
fn fly2_counts_geometry_units_and_random_seeds_follow_the_constructor_contract() {
    use simrep_io::{Fly2Context, read_fly2_in};
    let fixture = Fixture::new();
    let path = fixture.path("random.fly2");
    std::fs::write(
        &path,
        "particles {standard_beam {n=64,mass=1,charge=1, \
        tob=1*ns,ke=2*keV,position=circle_distribution {radius=2,normal=vector(1,0,0),fill=true}, \
        direction=cone_direction_distribution {axis=vector(1,0,0),half_angle=20,fill=true}}}",
    )
    .unwrap();
    let first = read_fly2_in(
        &path,
        &Fly2Context {
            seed: 42,
            ..Default::default()
        },
    )
    .unwrap();
    let repeated = read_fly2_in(
        &path,
        &Fly2Context {
            seed: 42,
            ..Default::default()
        },
    )
    .unwrap();
    let different = read_fly2_in(
        &path,
        &Fly2Context {
            seed: 43,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(first, repeated);
    assert_ne!(first, different);
    assert_eq!(first.len(), 64);
    for p in first {
        assert_eq!(p.birth_time_us, 0.001);
        assert_eq!(p.energy_ev, 2000.);
        assert_eq!(p.position_mm[0], 0.);
        assert!(p.position_mm[1].hypot(p.position_mm[2]) <= 2.);
        assert!(p.direction[0] >= 20f64.to_radians().cos());
    }
    let sequence=parse_fly2("particles {standard_beam {n=2, \
        tob=arithmetic_sequence{first=1,last=3,n=3},x=arithmetic_sequence{first=10,last=20,n=2}}, \
        standard_beam {position=circle_sequence {center=vector(0,0,0),normal=vector(1,0,0),radius=1,n=4}}}").unwrap();
    assert_eq!(sequence.len(), 6);
    assert_eq!(sequence[0].birth_time_us, 1.);
    assert_eq!(sequence[1].birth_time_us, 2.);
    assert_eq!(sequence[1].position_mm[0], 10.);
    close(&sequence[2].position_mm, &[0., 1., 0.]);
    close(&sequence[3].position_mm, &[0., 0., 1.]);
    let speed = parse_fly2("particles {standard_beam {mass=28.01,speed=0.8,color=17}}").unwrap();
    assert!((speed[0].energy_ev - 0.092897017606216).abs() < 1e-14);
    assert_eq!(speed[0].color, 1.);
    assert_eq!(parse_fly2("particles {standard_beam {position=gaussian3d_distribution {mean=vector(1,2,3),stdev=vector(0,0,0)}}}").unwrap()[0].position_mm,[1.,2.,3.]);
    let window =
        parse_fly2("particles {standard_beam {position=vector(2,0,0),window=vector(0,0,0)}}")
            .unwrap();
    assert_eq!(window[0].position_mm, [2., 0., 0.]);
    assert_eq!(window[0].direction, [1., 0., 0.]);
}

#[test]
fn fly2_pa_electrode_reads_nodes_not_flight_collision_cells() {
    use simrep_io::{Fly2Context, Fly2Instance, read_fly2_in};
    let fixture = Fixture::new();
    let path = fixture.path("nodes.fly2");
    std::fs::write(
        &path,
        "local pa=simion.wb.instances[1].pa; \
        assert(pa:electrode(1,1,0)); assert(not pa:electrode(0,0,0)); \
        assert(not pa:electrode(-1,0,0)); particles {standard_beam {}}",
    )
    .unwrap();
    let mut mask = vec![false; 9];
    mask[4] = true;
    let context = Fly2Context {
        instances: vec![Fly2Instance {
            dimensions: [3, 3, 1],
            electrodes: Some(mask.into()),
            ..Default::default()
        }],
        ..Default::default()
    };
    assert_eq!(read_fly2_in(&path, &context).unwrap().len(), 1);
}

#[test]
fn read_particles_dispatches_extensions_and_rejects_legacy_formats() {
    let fixture = Fixture::new();
    std::fs::write(fixture.path("beam.ION"), "0,4,-1,1,2,3,0,0,10,1,0").unwrap();
    std::fs::write(
        fixture.path("beam.FLY2"),
        "particles {standard_beam {mass=4,charge=-1,ke=10,position=vector(1,2,3)}}",
    )
    .unwrap();
    assert_eq!(
        read_particles(fixture.path("beam.ION")).unwrap(),
        read_particles(fixture.path("beam.FLY2")).unwrap()
    );
    assert_eq!(
        read_particles(Path::new("beam.fly")).unwrap_err().kind(),
        ErrorKind::InvalidData
    );
}

use simrep_core::PotentialArray;
use simrep_fly::{surface::SurfaceModel, surface_sdf::SdfVolumes};
use simrep_geom::{SampledSdf, Sdf, SignedDistance};
use std::sync::Arc;

fn model(
    shape: Arc<dyn SignedDistance>,
    dims: [usize; 3],
    origin: [f64; 3],
    h: f64,
    n: u32,
) -> (SurfaceModel, SdfVolumes) {
    let mut pa = PotentialArray::new(dims[0], dims[1], dims[2]);
    pa.dx_mm = h;
    pa.dy_mm = h;
    pa.dz_mm = h;
    let model = SurfaceModel::from_sdfs(vec![(1, shape)], 1e-10)
        .unwrap()
        .with_pa_domain(&pa, origin)
        .unwrap();
    let sdf = SdfVolumes::build(&model, &pa, n).unwrap();
    (model, sdf)
}

#[test]
fn cylinder_and_sampled_sdf_counterexamples() {
    let (_, sdf) = model(
        Arc::new(Sdf::Cylinder {
            center: [0.0; 3],
            radius: 0.2,
            axis: 2,
        }),
        [21, 21, 1],
        [-1.0, -1.0, 0.0],
        0.1,
        2,
    );
    assert!(sdf.first_hit([0.2, 0.2, 0.0], [0.09, 0.09, 0.0]).is_some());
    let shape = SampledSdf::new([3, 1, 1], [0.0; 3], [1.0; 3], vec![2.0, -2.0, 2.0]).unwrap();
    let (_, sdf) = model(Arc::new(shape), [3, 1, 1], [0.0; 3], 1.0, 2);
    let (t, _) = sdf.first_hit([0.0; 3], [1.6, 0.0, 0.0]).unwrap();
    assert!((t * 1.6 - 0.5).abs() < 1e-9);
}

#[test]
fn documented_unresolved_grazing_sphere() {
    let (exact, sdf) = model(
        Arc::new(Sdf::Sphere {
            center: [0.0; 3],
            radius: 1.0,
        }),
        [8, 12, 1],
        [-0.35, 0.0, 0.0],
        0.1,
        1,
    );
    let from = [-0.3, 0.9999, 0.0];
    let to = [0.3, 0.9999, 0.0];
    assert!(exact.first_hit(from, to).is_some());
    assert!(sdf.first_hit(from, to).is_none());
}

#[test]
fn random_segments_against_interpolant_and_source() {
    let mut seed = 0x39da_371c_5678_9abc_u64;
    let mut random = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed >> 11) as f64 / (1_u64 << 53) as f64
    };
    for sphere in [true, false] {
        let shape: Arc<dyn SignedDistance> = if sphere {
            Arc::new(Sdf::Sphere {
                center: [0.0; 3],
                radius: 0.7,
            })
        } else {
            Arc::new(Sdf::Cylinder {
                center: [0.0; 3],
                radius: 0.7,
                axis: 2,
            })
        };
        let (exact, sdf) = model(shape.clone(), [31, 31, 1], [-1.5, -1.5, 0.0], 0.1, 2);
        let v = &sdf.volumes[0];
        for _ in 0..100 {
            let angle = random() * std::f64::consts::TAU;
            let offset = (random() * 2.0 - 1.0) * 0.72;
            let dir = [angle.cos(), angle.sin()];
            let from = [
                -1.1 * dir[0] - offset * dir[1],
                -1.1 * dir[1] + offset * dir[0],
                0.0,
            ];
            let to = [
                1.1 * dir[0] - offset * dir[1],
                1.1 * dir[1] + offset * dir[0],
                0.0,
            ];
            let mut previous = v.interpolate(from);
            let mut first = None;
            let mut penetration = 0.0_f64;
            for i in 1..=10_000 {
                let t = i as f64 / 10_000.0;
                let p = std::array::from_fn(|a| from[a] + t * (to[a] - from[a]));
                let d = v.interpolate(p);
                if first.is_none() && previous > 0.0 && d <= 0.0 {
                    first = Some(t);
                }
                previous = d;
                penetration = penetration.max(-shape.distance(p));
            }
            let hit = sdf.first_hit(from, to);
            if let Some(t) = first {
                let actual = hit.expect("interpolant sign change missed").0;
                assert!(actual <= t && t - actual <= 0.0001 + 1e-9);
            }
            if let Some((t, _)) = exact.first_hit(from, to)
                && penetration > 2.0 * v.error_bound_mm
            {
                let st = hit.expect("well-resolved exact hit missed").0;
                assert!((st - t).abs() * 2.2 < 2.0 * 0.05 * 0.05);
            }
        }
    }
}

#[test]
fn mms_annulus_sphere_and_cylinder_crossings() {
    for sphere in [true, false] {
        let shape = |radius| {
            if sphere {
                Sdf::Sphere {
                    center: [0.0; 3],
                    radius,
                }
            } else {
                Sdf::Cylinder {
                    center: [0.0; 3],
                    radius,
                    axis: 2,
                }
            }
        };
        let mut pa = PotentialArray::new(33, 33, 1);
        pa.dx_mm = 2.0 / 32.0;
        pa.dy_mm = pa.dx_mm;
        let outer = Sdf::Box {
            center: [0.0; 3],
            half_extent: [4.0; 3],
        }
        .difference(shape(0.83));
        let surface = SurfaceModel::from_sdfs(
            vec![(1, Arc::new(shape(0.31))), (2, Arc::new(outer))],
            1e-10,
        )
        .unwrap()
        .with_pa_domain(&pa, [-1.0, -1.0, 0.0])
        .unwrap();
        let sdf = SdfVolumes::build(&surface, &pa, 2).unwrap();
        let mut max_error = 0.0_f64;
        let mut squares = 0.0;
        let mut exact_time = std::time::Duration::ZERO;
        let mut sampled_time = std::time::Duration::ZERO;
        for i in 0..128 {
            let angle = std::f64::consts::TAU * (i as f64 + 0.371) / 128.0;
            let direction = [angle.cos(), angle.sin(), 0.0];
            let from = direction.map(|x| x * 0.5);
            let to = direction.map(|x| x * 0.9);
            let start = std::time::Instant::now();
            let exact = surface.first_hit(from, to).unwrap();
            exact_time += start.elapsed();
            let start = std::time::Instant::now();
            let sampled = sdf.first_hit(from, to).unwrap();
            sampled_time += start.elapsed();
            assert_eq!(exact.1, sampled.1);
            let error = (sampled.0 - exact.0).abs() * 0.4;
            max_error = max_error.max(error);
            squares += error * error;
        }
        println!(
            "mms {} N=2 count=128 max_mm={max_error} rms_mm={} exact_compute_ms={} sdf_compute_ms={}",
            if sphere { "sphere" } else { "cylinder" },
            (squares / 128.0_f64).sqrt(),
            exact_time.as_secs_f64() * 1000.0,
            sampled_time.as_secs_f64() * 1000.0
        );
    }
}

#[test]
fn narrow_interpolant_crossing_is_not_skipped() {
    let pa = PotentialArray::new(2, 2, 1);
    let shape = SampledSdf::new(
        [2, 2, 1],
        [0.0; 3],
        [1.0; 3],
        vec![0.2499, -0.2501, -0.2501, 0.2499],
    )
    .unwrap();
    let surface = SurfaceModel::from_sdfs(vec![(1, Arc::new(shape))], 0.1)
        .unwrap()
        .with_pa_domain(&pa, [0.0; 3])
        .unwrap();
    let sdf = SdfVolumes::build(&surface, &pa, 1).unwrap();
    assert!(sdf.volumes[0].interpolate([0.5, 0.5, 0.0]) < 0.0);
    let (t, number) = sdf
        .first_hit([0.0; 3], [1.0, 1.0, 0.0])
        .expect("must detect the narrow negative interval");
    assert_eq!(number, 1);
    assert!((t - 0.49).abs() * 2.0_f64.sqrt() <= 0.1);
}

#[test]
fn sampled_sidecar_preserves_its_interpolant() {
    let sidecar = serde_json::json!({
        "version": 2, "dims": [2, 1, 1], "origin_mm": [0.0, 0.0, 0.0],
        "spacing_mm": [2.0, 1.0, 1.0], "symmetry": "planar",
        "electrode_numbers": [1], "node_electrodes": [null, null],
        "cut_fractions": [[null, null, null, null, null, null], [null, null, null, null, null, null]],
        "cut_electrodes": [[null, null, null, null, null, null], [null, null, null, null, null, null]],
        "sampled_sdfs": [{"number": 1, "dims": [3, 1, 1], "origin_mm": [0.0, 0.0, 0.0],
            "spacing_mm": [1.0, 1.0, 1.0], "values": [1.0, -1.0, 1.0]}]
    });
    let mut pa = PotentialArray::new(2, 1, 1);
    pa.dx_mm = 2.0;
    let surface = SurfaceModel::from_reader(sidecar.to_string().as_bytes()).unwrap();
    let sdf = SdfVolumes::build(&surface, &pa, 1).unwrap();
    assert_eq!(sdf.volumes[0].error_bound_mm, 0.0);
    assert_eq!(sdf.volumes[0].interpolate([1.0, 0.0, 0.0]), -1.0);
}

#[test]
fn cylindrical_and_mirrored_segments_never_skip_an_interpolant_crossing() {
    use simrep_core::Symmetry;
    let mut seed = 0x1234_5678_9abc_def1_u64;
    let mut random = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed >> 11) as f64 / (1_u64 << 53) as f64
    };
    for (cylindrical, mirror_y) in [(true, false), (false, true), (true, true)] {
        let mut pa = PotentialArray::new(41, 21, 1);
        pa.dx_mm = 0.1;
        pa.dy_mm = 0.1;
        pa.dz_mm = 0.1;
        if cylindrical {
            pa.symmetry = Symmetry::Cylindrical;
        }
        pa.mirrors.y = mirror_y;
        let shape: Arc<dyn SignedDistance> = Arc::new(Sdf::Sphere {
            center: [2.0, 0.6, 0.0],
            radius: 0.45,
        });
        let surface = SurfaceModel::from_sdfs(vec![(1, shape)], 1e-10)
            .unwrap()
            .with_pa_domain(&pa, [0.0; 3])
            .unwrap();
        let sdf = SdfVolumes::build(&surface, &pa, 2).unwrap();
        let mut checked = 0;
        for _ in 0..400 {
            let from = [
                random() * 4.0,
                (random() * 2.0 - 1.0) * 1.9,
                if cylindrical {
                    (random() * 2.0 - 1.0) * 1.9
                } else {
                    0.0
                },
            ];
            let to = [
                random() * 4.0,
                (random() * 2.0 - 1.0) * 1.9,
                if cylindrical {
                    (random() * 2.0 - 1.0) * 1.9
                } else {
                    0.0
                },
            ];
            if sdf.membership(from).is_some() {
                continue;
            }
            let mut dense = None;
            for i in 1..=20_000 {
                let t = i as f64 / 20_000.0;
                let p = std::array::from_fn(|a| from[a] + t * (to[a] - from[a]));
                if sdf.membership(p).is_some() {
                    dense = Some(t);
                    break;
                }
            }
            let hit = sdf.first_hit(from, to).map(|h| h.0);
            if let Some(td) = dense {
                checked += 1;
                let th = hit.unwrap_or_else(|| {
                    panic!("missed crossing at t={td} cyl={cylindrical} mirror={mirror_y}")
                });
                assert!(th <= td + 1e-9, "hit {th} after dense crossing {td}");
            }
        }
        assert!(checked > 20, "too few crossing segments: {checked}");
    }
}

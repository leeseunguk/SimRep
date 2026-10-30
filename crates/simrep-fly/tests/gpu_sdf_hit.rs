//! Standalone GPU SDF acceptance scenarios. Software Vulkan is intentional.
use std::{sync::Arc, time::Instant};

use rayon::prelude::*;
use simrep_core::{PotentialArray, Symmetry};
use simrep_fly::{
    Vector,
    gpu::{Backends, Gpu, Precision},
    sdf_hit::{GpuSdf, gpu_first_hits},
    surface::SurfaceModel,
    surface_sdf::SdfVolumes,
};
use simrep_geom::{SampledSdf, Sdf, SignedDistance};

#[path = "common/narrow_cylindrical.rs"]
mod narrow_cylindrical;

fn build(
    shape: Arc<dyn SignedDistance>,
    dims: [usize; 3],
    origin: Vector,
    h: f64,
    tolerance: f64,
) -> SdfVolumes {
    let mut pa = PotentialArray::new(dims[0], dims[1], dims[2]);
    pa.dx_mm = h;
    pa.dy_mm = h;
    pa.dz_mm = h;
    let surface = SurfaceModel::from_sdfs(vec![(1, shape)], tolerance)
        .unwrap()
        .with_pa_domain(&pa, origin)
        .unwrap();
    SdfVolumes::build(&surface, &pa, 1).unwrap()
}

fn compare(label: &str, sdf: &SdfVolumes, segments: &[(Vector, Vector)]) {
    let cpu: Vec<_> = segments.iter().map(|&(a, b)| sdf.first_hit(a, b)).collect();
    for precision in [Precision::Precise, Precision::Preview] {
        let gpu = GpuSdf::new(sdf, precision).unwrap();
        let hits = gpu.first_hits(segments).unwrap();
        assert_eq!(hits.len(), cpu.len());
        let mut agreement = 0;
        let mut max_dt = 0.0_f64;
        let mut max_mm = 0.0_f64;
        for (i, (&reference, &actual)) in cpu.iter().zip(&hits).enumerate() {
            assert_eq!(
                reference.map(|h| h.1),
                actual.map(|h| h.1),
                "{label} segment {i}: {:?}",
                segments[i]
            );
            agreement += 1;
            if let (Some((a, _)), Some((b, _))) = (reference, actual) {
                assert!(b.is_finite() && (0.0..=1.0).contains(&b));
                let length = (0..3)
                    .map(|j| (segments[i].1[j] - segments[i].0[j]).powi(2))
                    .sum::<f64>()
                    .sqrt();
                max_dt = max_dt.max((a - b).abs());
                max_mm = max_mm.max((a - b).abs() * length);
                let tolerance = if gpu.has_f64() { 1e-9 } else { 1e-3 };
                assert!(
                    (a - b).abs() <= tolerance,
                    "{label} segment {i}: CPU={a:.17e} GPU={b:.17e} tolerance={tolerance}"
                );
            }
        }
        assert_eq!(agreement, segments.len());
        println!(
            "SDF_AGREEMENT case={label} precision={} adapter={:?} count={} agreement={agreement} max_dt={max_dt:.17e} max_error_mm={max_mm:.17e}",
            if gpu.has_f64() { "f64" } else { "f32" },
            gpu.adapter_info(),
            segments.len()
        );
    }
}

#[test]
fn gpu_sdf_reference_agreement() {
    let Some(probe) = pollster::block_on(Gpu::new(Backends::VULKAN)).unwrap() else {
        eprintln!("SKIP GPU SDF: no Vulkan adapter");
        return;
    };
    println!(
        "SDF_ADAPTER {:?} native_f64={}",
        probe.info,
        probe.has_f64()
    );
    drop(probe);
    let (quartic, from, to) = narrow_cylindrical::fixture();
    let first = quartic.first_hit(from, to).unwrap();
    assert_eq!(first.1, 1);
    assert!((first.0 - 0.05).abs() < 1e-9);
    compare("cylindrical_quartic_first_entry", &quartic, &[(from, to)]);
    let cylinder = build(
        Arc::new(Sdf::Cylinder {
            center: [0.0; 3],
            radius: 0.2,
            axis: 2,
        }),
        [41, 41, 1],
        [-1.0, -1.0, 0.0],
        0.05,
        1e-10,
    );
    compare(
        "cylinder_counterexample",
        &cylinder,
        &[([0.2, 0.2, 0.0], [0.09, 0.09, 0.0])],
    );
    let sampled = build(
        Arc::new(SampledSdf::new([3, 1, 1], [0.0; 3], [1.0; 3], vec![2.0, -2.0, 2.0]).unwrap()),
        [3, 1, 1],
        [0.0; 3],
        1.0,
        1e-10,
    );
    compare(
        "sampled_2_minus2_2",
        &sampled,
        &[
            ([0.0; 3], [1.6, 0.0, 0.0]),
            ([0.0; 3], [200.0, 0.0, 0.0]),
            ([1.0, 0.0, 0.0], [1.0, 0.0, 0.0]),
            ([3.0, 0.0, 0.0], [3.0, 0.0, 0.0]),
            ([2.0, 0.0, 0.0], [0.0; 3]),
            ([-20.0, 0.0, 0.0], [20.0, 0.0, 0.0]),
            ([0.5, 0.0, 0.0], [0.5, 0.0, 0.0]),
        ],
    );
    let narrow = build(
        Arc::new(
            SampledSdf::new(
                [2, 2, 1],
                [0.0; 3],
                [1.0; 3],
                vec![0.2499, -0.2501, -0.2501, 0.2499],
            )
            .unwrap(),
        ),
        [2, 2, 1],
        [0.0; 3],
        1.0,
        0.1,
    );
    compare("narrow_crossing", &narrow, &[([0.0; 3], [1.0, 1.0, 0.0])]);
    let grazing = build(
        Arc::new(Sdf::Sphere {
            center: [0.0; 3],
            radius: 1.0,
        }),
        [8, 12, 1],
        [-0.35, 0.0, 0.0],
        0.1,
        1e-10,
    );
    let grazing_segment = [([-0.3, 0.9999, 0.0], [0.3, 0.9999, 0.0])];
    assert!(
        grazing
            .first_hit(grazing_segment[0].0, grazing_segment[0].1)
            .is_none()
    );
    compare("grazing_sphere_expected_miss", &grazing, &grazing_segment);
    assert!(
        gpu_first_hits(&sampled, &[], Precision::Precise)
            .unwrap()
            .is_empty()
    );
    let mut empty = sampled.clone();
    empty.volumes.clear();
    assert_eq!(
        gpu_first_hits(&empty, &[([0.0; 3], [1.0; 3])], Precision::Precise).unwrap(),
        vec![None]
    );
    assert!(
        gpu_first_hits(
            &sampled,
            &[([f64::NAN, 0.0, 0.0], [0.0; 3])],
            Precision::Preview
        )
        .is_err()
    );
    println!("SDF_BOUNDARIES empty_segments=pass empty_volumes=pass nonfinite_rejected=pass");
    let mut seed = 0x39da_371c_5678_9abc_u64;
    let mut random = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed >> 11) as f64 / (1_u64 << 53) as f64
    };
    for (label, cylindrical, mirrored) in [
        ("planar", false, false),
        ("cylindrical", true, false),
        ("mirrored", false, true),
        ("cylindrical_mirrored", true, true),
    ] {
        let mut pa = PotentialArray::new(11, 11, if cylindrical { 1 } else { 11 });
        pa.dx_mm = 0.2;
        pa.dy_mm = 0.2;
        pa.dz_mm = 0.2;
        if cylindrical {
            pa.symmetry = Symmetry::Cylindrical;
        }
        pa.mirrors.x = mirrored;
        pa.mirrors.y = mirrored;
        pa.mirrors.z = mirrored && !cylindrical;
        let origin = if cylindrical {
            [-1.0, 0.0, 0.0]
        } else {
            [-1.0; 3]
        };
        let shape: Arc<dyn SignedDistance> = Arc::new(Sdf::Sphere {
            center: [0.1, 0.7, 0.1],
            radius: 0.45,
        });
        let other: Arc<dyn SignedDistance> = Arc::new(Sdf::Sphere {
            center: [0.65, 1.1, 0.1],
            radius: 0.2,
        });
        // Coincident volumes verify that ties retain input order, not number order.
        let surface =
            SurfaceModel::from_sdfs(vec![(7, shape.clone()), (3, shape), (11, other)], 1e-10)
                .unwrap()
                .with_pa_domain(&pa, origin)
                .unwrap();
        let sdf = SdfVolumes::build(&surface, &pa, 1).unwrap();
        let segments: Vec<_> = (0..10_000)
            .map(|_| {
                let a = [
                    random() * 4.0 - 2.0,
                    random() * 4.8 - 2.4,
                    random() * 4.8 - 2.4,
                ];
                let b = [
                    random() * 4.0 - 2.0,
                    random() * 4.8 - 2.4,
                    random() * 4.8 - 2.4,
                ];
                (a, b)
            })
            .collect();
        compare(label, &sdf, &segments);
    }
}

#[test]
#[ignore = "one million segment compute benchmark; opt in explicitly"]
fn gpu_sdf_million_segments() {
    let sdf = build(
        Arc::new(Sdf::Sphere {
            center: [0.0; 3],
            radius: 0.6,
        }),
        [11, 11, 11],
        [-1.0; 3],
        0.2,
        1e-10,
    );
    let mut seed = 0x1234_5678_9abc_def1_u64;
    let mut random = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed >> 11) as f64 / (1_u64 << 53) as f64
    };
    let segments: Vec<_> = (0..1_000_000)
        .map(|_| {
            let from = std::array::from_fn(|_| random() * 2.0 - 1.0);
            let to = std::array::from_fn(|a| from[a] + (random() - 0.5) * 0.1);
            (from, to)
        })
        .collect();
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(16)
        .build()
        .unwrap();
    let start = Instant::now();
    let cpu: Vec<_> = pool.install(|| {
        segments
            .par_iter()
            .map(|&(a, b)| sdf.first_hit(a, b))
            .collect()
    });
    let cpu_ms = start.elapsed().as_secs_f64() * 1000.0;
    for precision in [Precision::Precise, Precision::Preview] {
        let gpu = GpuSdf::new(&sdf, precision).unwrap();
        let _ = gpu.first_hits(&segments[..64]).unwrap();
        let start = Instant::now();
        let (hits, compute) = gpu.first_hits_profiled(&segments).unwrap();
        let gpu_total_ms = start.elapsed().as_secs_f64() * 1000.0;
        let gpu_compute_ms = compute
            .expect("benchmark requires GPU timestamp queries")
            .as_secs_f64()
            * 1000.0;
        let agreement = cpu
            .iter()
            .zip(&hits)
            .filter(|(a, b)| a.map(|h| h.1) == b.map(|h| h.1))
            .count();
        assert_eq!(agreement, segments.len());
        println!(
            "SDF_TIMING adapter={:?} software={} precision={} count={} cpu_threads=16 cpu_compute_ms={cpu_ms:.6} gpu_compute_ms={gpu_compute_ms:.6} gpu_total_ms={gpu_total_ms:.6} agreement={agreement}",
            gpu.adapter_info(),
            gpu.adapter_info().device_type == wgpu::DeviceType::Cpu,
            if gpu.has_f64() { "f64" } else { "f32" },
            segments.len()
        );
    }
}

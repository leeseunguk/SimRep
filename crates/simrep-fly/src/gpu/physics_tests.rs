use super::*;
use crate::Field;
use crate::physics::{
    collision::{CollisionModel, DiffusionStatistics, Gas, Hs1, Sds, fly_many_collisional},
    magnetic::{Coil, MagneticField, MagneticPa},
};
use simrep_core::{FieldType, PotentialArray};

fn empty_electric() -> InstanceField {
    InstanceField::new(
        GridField::new(PotentialArray::new(2, 2, 2)).unwrap(),
        [0.0; 3],
        [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        1.0,
        Some([[-1000.0; 3], [1000.0; 3]]),
        None,
    )
    .unwrap()
}

struct Combined<'a>(&'a InstanceField, &'a MagneticField);

impl Field for Combined<'_> {
    fn sample(&self, p: [f64; 3], t: f64) -> crate::FieldSample {
        let mut sample = self.0.sample(p, t);
        sample.magnetic_gauss = self.1.sample(p, t).magnetic_gauss;
        sample
    }
    fn status(&self, p: [f64; 3]) -> Option<SplatReason> {
        self.0.status(p).or_else(|| self.1.status(p))
    }
    fn intersection(&self, a: [f64; 3], b: [f64; 3]) -> Option<(f64, SplatReason)> {
        self.0
            .intersection(a, b)
            .into_iter()
            .chain(self.1.intersection(a, b))
            .min_by(|a, b| a.0.total_cmp(&b.0))
    }
}

fn compare(cpu: &[FlightResult], gpu: &[FlightResult], tolerance: f64) {
    for (a, b) in cpu.iter().zip(gpu) {
        assert_eq!(a.splat_reason, b.splat_reason, "{a:?} {b:?}");
        assert!((a.tof_us - b.tof_us).abs() < tolerance, "{a:?} {b:?}");
        for axis in 0..3 {
            assert!(
                (a.final_state.position_mm[axis] - b.final_state.position_mm[axis]).abs()
                    < tolerance,
                "{a:?} {b:?}"
            );
            assert!(
                (a.final_state.velocity_mm_per_us[axis] - b.final_state.velocity_mm_per_us[axis])
                    .abs()
                    < tolerance,
                "{a:?} {b:?}"
            );
        }
    }
}

#[test]
fn gpu_imported_magnetic_maps_precise_and_preview() {
    use crate::physics::magnetic::{ImportedMagneticMap, MapPlacement, OutsideMap};
    for precision in [Precision::Precise, Precision::Preview] {
        let Some(gpu) =
            pollster::block_on(Gpu::new_with_precision(Backends::VULKAN, precision)).unwrap()
        else {
            eprintln!("SKIP GPU imported magnetic maps: no Vulkan adapter");
            return;
        };
        eprintln!(
            "GPU imported maps: {:?}, adapter={}, f64={}",
            precision,
            gpu.info.name,
            gpu.has_f64()
        );
        let electric = InstanceField::new(
            GridField::new(PotentialArray::new(1, 1, 1)).unwrap(),
            [10.0, -5.0, 8.0],
            [[-1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            1.0,
            Some([[-1000.0; 3], [1000.0; 3]]),
            None,
        )
        .unwrap();
        let particles = [
            Particle::new(100.0, 1.0, [0.5, 0.7, 0.6], [2.0, 0.2, -0.1], 0.0).unwrap(),
            Particle::new(100.0, -1.0, [-0.6, 0.5, 0.8], [-2.0, 0.1, 1.0], 3.0).unwrap(),
        ];
        for axisymmetric in [false, true] {
            for outside in [OutsideMap::Error, OutsideMap::Zero, OutsideMap::Clamp] {
                let dims = if axisymmetric { [7, 7, 1] } else { [7, 7, 7] };
                let values = (0..dims.iter().product::<usize>())
                    .map(|i| {
                        let x = (i % 7) as f64;
                        let y = (i / 7 % 7) as f64;
                        let z = (i / 49) as f64;
                        if axisymmetric {
                            [100.0 + 0.2 * x * x - 0.1 * y * y, -0.2 * x * y, 0.0]
                        } else {
                            [20.0 + y * z, 30.0 + x * z, 100.0 + x * y]
                        }
                    })
                    .collect();
                let placement = if axisymmetric {
                    MapPlacement {
                        outside,
                        mirrors: [Some([1, -1, 1]), None, None],
                        ..Default::default()
                    }
                } else {
                    MapPlacement {
                        origin_mm: [-3.0; 3],
                        axes: [[0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]],
                        scale: 1.5,
                        outside,
                        ..Default::default()
                    }
                };
                let map = ImportedMagneticMap::from_nodes(
                    dims,
                    [1.0; 3],
                    values,
                    axisymmetric,
                    placement,
                )
                .unwrap();
                let extra = ImportedMagneticMap::from_nodes(
                    [1, 1, 1],
                    [1.0; 3],
                    vec![[1.0, 2.0, 3.0]],
                    false,
                    MapPlacement::default(),
                )
                .unwrap();
                let magnetic = MagneticField {
                    uniform_gauss: [2.0, -1.0, 0.5],
                    maps: vec![map, extra],
                    coils: vec![Coil::hoop([0.0; 3], [0.0, 0.0, 1.0], 30.0, 20.0).unwrap()],
                    ..Default::default()
                };
                let options = FlightOptions {
                    duration_us: 4.0,
                    max_step_us: 0.05,
                    ..Default::default()
                };
                let cpu =
                    crate::fly_many(&Combined(&electric, &magnetic), &particles, &options).unwrap();
                let actual = gpu
                    .prepare_physics(&electric, &[], &magnetic, None, 17)
                    .unwrap()
                    .fly(&particles, &options)
                    .unwrap();
                compare(
                    &cpu,
                    &actual.results,
                    if gpu.has_f64() { 3e-9 } else { 3e-4 },
                );
                assert!(
                    actual
                        .results
                        .iter()
                        .all(|r| r.splat_reason != SplatReason::NonFinite)
                );
            }
        }
    }
}

fn assert_philox(gpu: &Gpu) {
    let functions = include_str!("physics.wgsl");
    let start = functions.find("fn multiply_wide(").unwrap();
    let end = functions.find("fn random_uniform(").unwrap();
    let source = format!(
        "{}\n{}",
        &functions[start..end],
        r#"
@group(0) @binding(0) var<storage,read_write> results: array<vec4<u32>>;
@compute @workgroup_size(1)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let counters = array<vec4<u32>,3>(
        vec4<u32>(0u), vec4<u32>(0xffffffffu),
        vec4<u32>(0x243f6a88u,0x85a308d3u,0x13198a2eu,0x03707344u));
    let keys = array<vec2<u32>,3>(
        vec2<u32>(0u),vec2<u32>(0xffffffffu),vec2<u32>(0xa4093822u,0x299f31d0u));
    results[id.x] = philox(counters[id.x],keys[id.x]);
}"#
    );
    let shader = gpu
        .device
        .create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Philox known answers"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
    let pipeline = gpu
        .device
        .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Philox known answers"),
            layout: None,
            module: &shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
    let values = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Philox output"),
        size: 48,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let readback = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Philox readback"),
        size: 48,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: values.as_entire_binding(),
        }],
    });
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.dispatch_workgroups(3, 1, 1);
    }
    encoder.copy_buffer_to_buffer(&values, 0, &readback, 0, 48);
    gpu.queue.submit([encoder.finish()]);
    let (tx, rx) = std::sync::mpsc::channel();
    readback.map_async(wgpu::MapMode::Read, .., move |result| {
        tx.send(result).unwrap()
    });
    gpu.device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(TIMEOUT),
        })
        .unwrap();
    rx.recv_timeout(TIMEOUT).unwrap().unwrap();
    let mapped = readback.get_mapped_range(..).unwrap();
    let actual: &[[u32; 4]] = bytemuck::cast_slice(&mapped);
    // Random123's published Philox4x32-10 zero, all-one and pi vectors.
    assert_eq!(
        actual,
        &[
            [0x6627e8d5, 0xe169c58d, 0xbc57ac4c, 0x9b00dbd8],
            [0x408f276d, 0x41c83b0e, 0xa20bc7c6, 0x6d5451fd],
            [0xd16cfe09, 0x94fdcceb, 0x5001e420, 0x24126ea1],
        ]
    );
}

#[test]
fn gpu_magnetic_uniform_pa_coils_and_boris() {
    for precision in [Precision::Precise, Precision::Preview] {
        let Some(gpu) = pollster::block_on(Gpu::new_with_precision(Backends::VULKAN, precision))
            .unwrap_or_else(|e| panic!("{e}"))
        else {
            eprintln!("SKIP GPU magnetic: no Vulkan adapter");
            return;
        };
        eprintln!(
            "GPU physics adapter: {} {:?} {:?}, native_f64={}",
            gpu.info.name,
            gpu.info.backend,
            gpu.info.device_type,
            gpu.has_f64()
        );
        let e = empty_electric();
        let particles = [
            Particle::new(100.0, 1.0, [0.5, 0.7, 0.6], [1.0, 0.2, -0.1], 0.0).unwrap(),
            Particle::new(100.0, -1.0, [0.6, 0.5, 0.8], [-0.2, 0.1, 1.0], 3.0).unwrap(),
        ];
        let mut array = PotentialArray::new(5, 5, 5);
        array.field_type = FieldType::Magnetic;
        array.mirrors.x = true;
        for z in 0..5 {
            for y in 0..5 {
                for x in 0..5 {
                    let index = (z * 5 + y) * 5 + x;
                    array.potentials[index] = -3.0 * y as f64 - 2.0 * z as f64
                        + 0.001 * ((x * x) as f64 - (y * y) as f64) * z as f64;
                    array.electrodes[index] = x == 4;
                }
            }
        }
        let pa = MagneticPa::placed(
            array,
            20.0,
            [-10.0; 3],
            [[0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]],
            10.0,
            Some([[-1000.0; 3], [1000.0; 3]]),
        )
        .unwrap();
        let cases = [
            MagneticField {
                uniform_gauss: [0.0, 0.0, 100.0],
                ..Default::default()
            },
            MagneticField {
                arrays: vec![pa],
                ..Default::default()
            },
            MagneticField {
                coils: vec![
                    Coil::segment([-3.0, -1.0, -2.0], [-3.0, 5.0, 2.0], 7.0).unwrap(),
                    Coil::hoop([0.0; 3], [1.0, 0.0, 0.0], 30.0, 120.0).unwrap(),
                ],
                ..Default::default()
            },
        ];
        for (source, magnetic) in cases.iter().enumerate() {
            for integrator in [Integrator::Rk45, Integrator::Boris] {
                eprintln!("GPU magnetic case: {precision:?}, source={source}, {integrator:?}");
                let options = FlightOptions {
                    integrator,
                    duration_us: 2.0,
                    initial_step_us: 0.01,
                    max_step_us: 0.05,
                    ..Default::default()
                };
                let cpu = crate::fly_many(&Combined(&e, magnetic), &particles, &options).unwrap();
                let actual = gpu
                    .prepare_physics(&e, &[], magnetic, None, 813)
                    .unwrap()
                    .fly(&particles, &options)
                    .unwrap();
                compare(
                    &cpu,
                    &actual.results,
                    if gpu.has_f64() { 2e-9 } else { 2e-4 },
                );
            }
        }
        let reflected = InstanceField::new(
            GridField::new(PotentialArray::new(1, 1, 1)).unwrap(),
            [10.0, -5.0, 8.0],
            [[-1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            1.0,
            Some([[-1000.0; 3], [1000.0; 3]]),
            None,
        )
        .unwrap();
        let reflected_b = MagneticField {
            uniform_gauss: [0.0, 0.0, 100.0],
            ..Default::default()
        };
        for integrator in [Integrator::Rk45, Integrator::Boris] {
            let options = FlightOptions {
                integrator,
                duration_us: 2.0,
                max_step_us: 0.01,
                ..Default::default()
            };
            let cpu =
                crate::fly_many(&Combined(&reflected, &reflected_b), &particles, &options).unwrap();
            let actual = gpu
                .prepare_physics(&reflected, &[], &reflected_b, None, 813)
                .unwrap()
                .fly(&particles, &options)
                .unwrap();
            compare(
                &cpu,
                &actual.results,
                if gpu.has_f64() { 2e-9 } else { 2e-4 },
            );
        }
        let magnetic = MagneticField::default();
        let options = FlightOptions {
            duration_us: 1.0,
            max_step_us: 0.01,
            ..Default::default()
        };
        let analytic = crate::AnalyticField(|p: [f64; 3], t: f64| crate::FieldSample {
            magnetic_gauss: [5.0 * p[1], 2.0 * t.sin(), 20.0 + p[0].powi(2)],
            ..Default::default()
        });
        let analytic_ions = [
            particles[0].clone(),
            Particle::new(100.0, -1.0, [-0.6, 0.5, 0.8], [-0.2, 0.1, 1.0], 3.0).unwrap(),
        ];
        let cpu = crate::fly_many(&analytic, &analytic_ions, &options).unwrap();
        let mut prepared = gpu.prepare_physics(&e, &[], &magnetic, None, 813).unwrap();
        prepared
            .set_magnetic_expressions([
                "5.0*position.y",
                "2.0*sin (time)",
                "20.0+pow (position.x,2.0)",
            ])
            .unwrap();
        let actual = prepared.fly(&analytic_ions, &options).unwrap();
        compare(
            &cpu,
            &actual.results,
            if gpu.has_f64() { 2e-9 } else { 2e-4 },
        );
        {
            let mut array = PotentialArray::new(2, 2, 2);
            for (index, potential) in array.potentials.iter_mut().enumerate() {
                *potential = -0.1 * (index % 2) as f64;
            }
            let electric = InstanceField::new(
                GridField::new(array).unwrap(),
                [0.0; 3],
                [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
                1.0,
                Some([[-1000.0; 3], [1000.0; 3]]),
                None,
            )
            .unwrap();
            let magnetic = MagneticField {
                uniform_gauss: [0.0, 0.0, 100.0],
                ..Default::default()
            };
            let options = FlightOptions {
                duration_us: 0.1,
                max_step_us: 0.01,
                ..Default::default()
            };
            let cpu =
                crate::fly_many(&Combined(&electric, &magnetic), &particles, &options).unwrap();
            let mut resident = gpu
                .prepare_physics(&electric, &[], &magnetic, None, 813)
                .unwrap();
            let stopped = resident
                .fly(
                    &particles,
                    &FlightOptions {
                        stop_plane: Some(crate::StopPlane {
                            axis: 0,
                            coordinate_mm: 0.55,
                        }),
                        ..options.clone()
                    },
                )
                .unwrap();
            assert_eq!(stopped.results[0].splat_reason, SplatReason::StopPlane);
            resident.set_voltages(&[1.0]).unwrap();
            let actual = resident.fly(&particles, &options).unwrap();
            compare(
                &cpu,
                &actual.results,
                if gpu.has_f64() { 2e-9 } else { 2e-4 },
            );
            let options = FlightOptions {
                integrator: Integrator::Boris,
                ..options
            };
            let cpu = crate::fly_many(&electric, &particles, &options).unwrap();
            let actual = gpu
                .prepare(&electric, &[])
                .unwrap()
                .fly(&particles, &options)
                .unwrap();
            compare(
                &cpu,
                &actual.results,
                if gpu.has_f64() { 2e-9 } else { 2e-4 },
            );
        }

        let magnetic = MagneticField {
            uniform_gauss: [0.0, 0.0, 10000.0],
            ..Default::default()
        };
        let ions = [Particle::new(1.0, 1.0, [0.5; 3], [1.0, 0.0, 0.0], 0.0).unwrap()];
        let options = FlightOptions {
            integrator: Integrator::Boris,
            initial_step_us: 1.0,
            duration_us: 1.0,
            ..Default::default()
        };
        let result = gpu
            .prepare_physics(&e, &[], &magnetic, None, 813)
            .unwrap()
            .fly(&ions, &options)
            .unwrap();
        assert!(
            result.results[0].accepted_steps
                >= (crate::MAGNETIC_FREQUENCY * 10000.0 / options.step_limits.max_cyclotron_radians)
                    .ceil() as usize
        );
        for cylindrical in [false, true] {
            let mut array = PotentialArray::new(7, 7, if cylindrical { 1 } else { 7 });
            array.field_type = FieldType::Magnetic;
            array.mirrors.x = true;
            if cylindrical {
                array.symmetry = Symmetry::Cylindrical;
            }
            for z in 0..array.nz {
                for y in 1..=4 {
                    for x in 1..=4 {
                        let solid = if cylindrical {
                            y >= 3
                        } else {
                            x >= 3 && (1..=4).contains(&z)
                        };
                        if solid {
                            array.electrodes[(z * 7 + y) * 7 + x] = true;
                        }
                    }
                }
            }
            let pa = MagneticPa::placed(
                array,
                1.0,
                [0.0; 3],
                [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
                1.0,
                Some([[-1000.0; 3], [1000.0; 3]]),
            )
            .unwrap();
            let magnetic = MagneticField {
                arrays: vec![pa],
                ..Default::default()
            };
            let ions = if cylindrical {
                vec![Particle::new(1.0, 0.0, [2.25, 0.25, 0.0], [0.0, 2.0, 0.0], 0.0).unwrap()]
            } else {
                vec![
                    Particle::new(1.0, 0.0, [0.25, 2.25, 2.25], [2.0, 0.0, 0.0], 0.0).unwrap(),
                    Particle::new(1.0, 0.0, [-0.25, 2.25, 2.25], [-2.0, 0.0, 0.0], 0.0).unwrap(),
                ]
            };
            let options = FlightOptions {
                duration_us: 2.0,
                max_step_us: 2.0,
                ..Default::default()
            };
            let cpu = crate::fly_many(&Combined(&e, &magnetic), &ions, &options).unwrap();
            let actual = gpu
                .prepare_physics(&e, &[], &magnetic, None, 813)
                .unwrap()
                .fly(&ions, &options)
                .unwrap();
            compare(
                &cpu,
                &actual.results,
                if gpu.has_f64() { 2e-8 } else { 2e-4 },
            );
            assert!(
                actual
                    .results
                    .iter()
                    .all(|r| matches!(r.splat_reason, SplatReason::Electrode { .. }))
            );
        }
    }
}

#[test]
fn gpu_two_dimensional_nodal_magnetic_samples() {
    let Some(gpu) = pollster::block_on(Gpu::new(Backends::VULKAN)).unwrap() else {
        return;
    };
    if !gpu.has_f64() {
        return;
    }
    let functions = include_str!("physics.wgsl");
    let functions = &functions[functions.find("fn hermite_weights(").unwrap()
        ..functions.find("fn root_vector(").unwrap()];
    let functions = functions
        .replace("f32", "f64")
        .replace("= 0.0;", "= 0.0lf;");
    let source = format!(
        r#"
struct Probe {{ dims: vec4<u32>, padding: vec4<u32>, spacing: vec4<f64> }}
@group(0) @binding(0) var<uniform> p: Probe;
@group(0) @binding(1) var<storage,read> transport_data: array<f64>;
@group(0) @binding(2) var<storage,read> points: array<vec4<f64>>;
@group(0) @binding(3) var<storage,read_write> output: array<vec4<f64>>;
const HERMITE: array<vec3<f64>,6> = array<vec3<f64>,6>(
 vec3<f64>(-10,15,-6),vec3<f64>(-6,8,-3),vec3<f64>(-1.5,1.5,-0.5),
 vec3<f64>(10,-15,6),vec3<f64>(-4,7,-3),vec3<f64>(0.5,-1,0.5));
fn coefficient(index: u32) -> f64 {{return transport_data[index];}}
{functions}
@compute @workgroup_size(64)
fn probe(@builtin(global_invocation_id) id: vec3<u32>) {{
 if id.x >= arrayLength(&points) {{return;}}
 let v = points[id.x].xyz;
 let radius = length(v.yz);
 let q = vec3<f64>(v.x/p.spacing.x,select(v.y,radius,p.dims.w!=0u)/p.spacing.y,select(v.z/p.spacing.z,0.0,p.dims.w!=0u));
 let cell = vec3<u32>(vec2<u32>(floor(q.xy)),0u);
 let gradient = nodal_gradient(cell,q-vec3<f64>(cell));
 var b = -p.spacing.w*gradient/p.spacing.xyz;
 if p.dims.w!=0u {{b = vec3<f64>(b.x,b.y*v.y/radius,b.y*v.z/radius);}}
 output[id.x] = vec4<f64>(b,nodal_value(0u,p.dims.xyz,cell,q-vec3<f64>(cell)));
}}"#
    );
    let module = gpu
        .device
        .create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("off-node magnetic jets"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
    let pipeline = gpu
        .device
        .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("off-node magnetic jets"),
            layout: None,
            module: &module,
            entry_point: Some("probe"),
            compilation_options: Default::default(),
            cache: None,
        });
    let positions = [
        [0.37, 0.29, 0.31, 0.0],
        [0.81, 0.63, -0.42, 0.0],
        [1.59, 1.17, 0.23, 0.0],
        [2.13, 0.71, 0.0, 0.0],
    ];
    for symmetry in [Symmetry::Planar, Symmetry::Cylindrical] {
        let mut array = PotentialArray::new(9, 8, 1);
        array.field_type = FieldType::Magnetic;
        array.symmetry = symmetry;
        array.dx_mm = 0.4;
        array.dy_mm = 0.3;
        for y in 0..8 {
            for x in 0..9 {
                let a = x as f64 * 0.4;
                let b = y as f64 * 0.3;
                array.potentials[y * 9 + x] = a * a * a + 0.2 * a * b * b + 2.0 * b * b;
            }
        }
        array.electrodes[7 * 9 + 8] = true;
        let magnetic = MagneticField {
            arrays: vec![MagneticPa::new(array, 2.0).unwrap()],
            ..Default::default()
        };
        let view = magnetic.arrays[0].gpu_parts();
        assert!(!view.instance.grid.gpu_data(true).3);
        let (_, _, jets, nodal) = view.instance.grid.gpu_physics_data(true);
        assert!(nodal);
        let mut parameters = bytemuck::cast_slice(&[
            9u32,
            8,
            1,
            u32::from(symmetry == Symmetry::Cylindrical),
            0,
            0,
            0,
            0,
        ])
        .to_vec();
        parameters.extend_from_slice(bytemuck::cast_slice(&[0.4f64, 0.3, 1.0, 2.0]));
        let make = |label, bytes: &[u8], usage| {
            gpu.device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some(label),
                    contents: bytes,
                    usage,
                })
        };
        let params = make("sample metadata", &parameters, wgpu::BufferUsages::UNIFORM);
        let values = make(
            "two-dimensional jets",
            bytemuck::cast_slice(&jets),
            wgpu::BufferUsages::STORAGE,
        );
        let points = make(
            "off-node points",
            bytemuck::cast_slice(&positions),
            wgpu::BufferUsages::STORAGE,
        );
        let size = std::mem::size_of_val(&positions) as u64;
        let output = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sampled B"),
            size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sampled B readback"),
            size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let layout = pipeline.get_bind_group_layout(0);
        let entries: Vec<_> = [&params, &values, &points, &output]
            .iter()
            .enumerate()
            .map(|(binding, buffer)| wgpu::BindGroupEntry {
                binding: binding as u32,
                resource: buffer.as_entire_binding(),
            })
            .collect();
        let bindings = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("magnetic probe"),
            layout: &layout,
            entries: &entries,
        });
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &bindings, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }
        encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, size);
        let submission = gpu.queue.submit([encoder.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        readback.slice(..).map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        gpu.device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submission),
                timeout: Some(gpu.poll_timeout()),
            })
            .unwrap();
        rx.recv_timeout(gpu.poll_timeout()).unwrap().unwrap();
        let mapped = readback.slice(..).get_mapped_range().unwrap();
        let sampled: &[[f64; 4]] = bytemuck::cast_slice(&mapped);
        for (point, b) in positions.iter().zip(sampled) {
            let expected = magnetic
                .sample(point[..3].try_into().unwrap(), 0.0)
                .magnetic_gauss;
            let potential = view
                .instance
                .grid
                .sample(point[..3].try_into().unwrap(), 0.0)
                .potential_v;
            assert!(
                (b[3] - potential).abs() < 2e-10,
                "{symmetry:?} nodal value GPU={}, CPU={potential}",
                b[3]
            );
            for a in 0..3 {
                assert!(
                    (b[a] - expected[a]).abs() < 2e-10,
                    "{symmetry:?} axis={a}, GPU={}, CPU={}",
                    b[a],
                    expected[a]
                );
            }
        }
    }
}

#[test]
fn gpu_native_scalar_domain_failure() {
    let Some(gpu) = pollster::block_on(Gpu::new(Backends::VULKAN)).unwrap() else {
        return;
    };
    if !gpu.has_f64() {
        return;
    }
    let electric = empty_electric();
    let magnetic = MagneticField::default();
    let mut prepared = gpu
        .prepare_physics(&electric, &[], &magnetic, None, 813)
        .unwrap();
    prepared
        .set_magnetic_expressions(["0.0", "0.0", "log (0.0)"])
        .unwrap();
    let particle = Particle::new(100.0, 1.0, [0.5; 3], [1.0, 0.2, 0.1], 0.0).unwrap();
    let options = FlightOptions {
        duration_us: 1.0,
        max_step_us: 0.01,
        ..Default::default()
    };
    let report = prepared.fly(&[particle], &options).unwrap();
    for result in report.results {
        assert_eq!(
            result.splat_reason,
            SplatReason::NonFinite,
            "tof={}, position={:?}",
            result.tof_us,
            result.final_state.position_mm
        );
    }
}

#[test]
fn gpu_preview_time_limit_integrates_the_compensated_remainder() {
    let Some(gpu) = pollster::block_on(Gpu::new_with_precision(
        Backends::VULKAN,
        Precision::Preview,
    ))
    .unwrap() else {
        eprintln!("SKIP Preview duration: no Vulkan adapter");
        return;
    };
    let field = InstanceField::new(
        GridField::new(PotentialArray::new(1, 1, 1)).unwrap(),
        [0.0; 3],
        [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        1.0,
        None,
        None,
    )
    .unwrap();
    let options = FlightOptions {
        duration_us: 50.0,
        max_step_us: 0.05,
        ..Default::default()
    };
    let particle = Particle::from_energy(200.0, 1.0, [0.0; 3], 0.1, [1.0, 0.0, 0.0], 0.0).unwrap();
    // Replay the worst clock overshoot in the independent 4096-ion KS fixture.
    let collision = CollisionModel::Hs1(Hs1::default());
    let magnetic = MagneticField::default();
    let report = gpu
        .prepare_physics(&field, &[], &magnetic, Some(&collision), 813)
        .unwrap()
        .fly_indexed(&[particle], &options, 324)
        .unwrap();
    assert_eq!(report.results[0].splat_reason, SplatReason::TimeLimit);
    assert_eq!(report.results[0].tof_us.to_bits(), 50.0_f64.to_bits());

    let drift = Particle::new(100.0, 0.0, [0.0; 3], [0.25, 0.125, 0.0], 0.0).unwrap();
    let report = gpu
        .prepare(&field, &[])
        .unwrap()
        .fly_sampled(
            &[drift],
            &options,
            &TrajectorySampling {
                ion_indices: &[0],
                every: 1,
                max_samples_per_ion: 2048,
            },
        )
        .unwrap();
    let result = &report.results[0];
    assert_eq!(result.splat_reason, SplatReason::TimeLimit);
    assert_eq!(result.tof_us.to_bits(), 50.0_f64.to_bits());
    assert_eq!(result.final_state.time_us.to_bits(), 50.0_f64.to_bits());
    let samples = result.trajectory.as_ref().unwrap();
    assert!(samples.iter().all(|s| s.time_us <= options.duration_us));
    for sample in samples {
        for (axis, velocity) in [0.25, 0.125, 0.0].into_iter().enumerate() {
            assert!((sample.position_mm[axis] - velocity * sample.time_us).abs() < 2e-4);
        }
    }
}

#[test]
fn gpu_hs1_stream_is_independent_of_batches() {
    let Some(gpu) =
        pollster::block_on(Gpu::new(Backends::VULKAN)).unwrap_or_else(|e| panic!("{e}"))
    else {
        eprintln!("SKIP GPU collisions: no Vulkan adapter");
        return;
    };
    assert_philox(&gpu);
    let e = empty_electric();
    let magnetic = MagneticField::default();
    let model = CollisionModel::Hs1(Hs1 {
        gas: Gas {
            pressure_pa: 53.0,
            ..Default::default()
        },
        ..Default::default()
    });
    let particles = vec![Particle::new(200.0, 1.0, [0.5; 3], [0.3, 0.1, 0.0], 0.0).unwrap(); 41];
    let options = FlightOptions {
        duration_us: 0.1,
        max_step_us: 0.01,
        ..Default::default()
    };
    let mut prepared = gpu
        .prepare_physics(&e, &[], &magnetic, Some(&model), 813)
        .unwrap();
    let complete = prepared.fly(&particles, &options).unwrap();
    let repeat = prepared.fly(&particles, &options).unwrap();
    compare(&complete.results, &repeat.results, 1e-15);
    prepared.params.physics_padding[2] = 1;
    let single_trial = prepared.fly(&particles, &options).unwrap();
    compare(&complete.results, &single_trial.results, 1e-15);
    prepared.params.physics_padding[2] = 32;
    let mut batched = Vec::new();
    for (batch, ions) in particles.chunks(7).enumerate() {
        batched.extend(
            prepared
                .fly_indexed(ions, &options, (batch * 7) as u64)
                .unwrap()
                .results,
        );
    }
    compare(&complete.results, &batched, 1e-15);
    assert!(
        complete
            .results
            .windows(2)
            .any(|w| w[0].final_state.velocity_mm_per_us != w[1].final_state.velocity_mm_per_us)
    );
}

#[test]
fn gpu_pending_physics_snapshots_instances_and_continues_seeded_streams() {
    let Some(gpu) = pollster::block_on(Gpu::new(Backends::VULKAN)).unwrap() else {
        eprintln!("SKIP pending GPU physics: no Vulkan adapter");
        return;
    };
    let mut array = PotentialArray::new(2, 2, 2);
    for (index, potential) in array.potentials.iter_mut().enumerate() {
        *potential = -0.1 * (index % 2) as f64;
    }
    let electric = InstanceField::new(
        GridField::new(array).unwrap(),
        [0.0; 3],
        [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        1.0,
        Some([[-100.0; 3], [100.0; 3]]),
        None,
    )
    .unwrap();
    let magnetic = MagneticField {
        uniform_gauss: [0.0, 0.0, 100.0],
        ..Default::default()
    };
    let model = CollisionModel::Hs1(Hs1 {
        gas: Gas {
            pressure_pa: 53.0,
            ..Default::default()
        },
        ..Default::default()
    });
    let particles = vec![Particle::new(200.0, 1.0, [0.5; 3], [0.3, 0.1, 0.0], 0.0).unwrap(); 17];
    let options = FlightOptions {
        duration_us: 0.1,
        max_step_us: 0.001,
        ..Default::default()
    };
    let mut prepared = gpu
        .prepare_physics(&electric, &[], &magnetic, Some(&model), 813)
        .unwrap();
    assert_eq!(prepared.max_ions_per_dispatch(), 4096);
    prepared.set_max_ions_per_dispatch(7).unwrap();
    prepared.params.physics_padding[2] = 1;
    let first = prepared.fly_indexed(&particles, &options, 100).unwrap();
    prepared.set_voltages(&[2.0]).unwrap();
    let second = prepared.fly_indexed(&particles, &options, 100).unwrap();
    assert_ne!(first.results[0].final_state, second.results[0].final_state);
    prepared.set_voltages(&[1.0]).unwrap();
    let pending_first = prepared.submit_indexed(&particles, &options, 100).unwrap();
    prepared.set_voltages(&[2.0]).unwrap();
    let pending_second = prepared.submit_indexed(&particles, &options, 100).unwrap();
    super::tests::assert_flight_bits(&second.results, &pending_second.collect().unwrap().results);
    super::tests::assert_flight_bits(&first.results, &pending_first.collect().unwrap().results);
    prepared.set_voltages(&[1.0]).unwrap();
    prepared.set_max_ions_per_dispatch(4096).unwrap();
    prepared.params.physics_padding[2] = 32;
    let combined = prepared.fly_indexed(&particles, &options, 100).unwrap();
    super::tests::assert_flight_bits(&first.results, &combined.results);
}

#[test]
fn gpu_sds_exact_damped_rotation_and_diffusion() {
    let Some(path) = std::env::var_os("SIMREP_GPU_MBM") else {
        eprintln!("SKIP GPU SDS: set SIMREP_GPU_MBM to the installed mbmr.dat");
        return;
    };
    let Some(gpu) =
        pollster::block_on(Gpu::new(Backends::VULKAN)).unwrap_or_else(|e| panic!("{e}"))
    else {
        eprintln!("SKIP GPU SDS: no Vulkan adapter");
        return;
    };
    let e = empty_electric();
    let magnetic = MagneticField {
        uniform_gauss: [0.0, 0.0, 100.0],
        ..Default::default()
    };
    let mut model = CollisionModel::Sds(Sds {
        gas: Gas {
            pressure_pa: 101325.0,
            temperature_k: 273.15,
            mass_amu: 28.94515,
            velocity_mm_per_us: [0.1, 0.0, 0.0],
        },
        gas_diameter_nm: 0.366,
        ion_diameter_nm: None,
        mobility_stp: None,
        diffusion: false,
        statistics: DiffusionStatistics::from_path(path).unwrap(),
    });
    let particles = vec![Particle::new(200.0, -1.0, [0.5; 3], [1.0, -0.1, 0.2], 0.0).unwrap(); 23];
    let options = FlightOptions {
        duration_us: 50.0,
        initial_step_us: 50.0,
        max_step_us: 50.0,
        ..Default::default()
    };
    let cpu =
        fly_many_collisional(&Combined(&e, &magnetic), &particles, &options, &model, 813).unwrap();
    let report = gpu
        .prepare_physics(&e, &[], &magnetic, Some(&model), 813)
        .unwrap()
        .fly(&particles, &options)
        .unwrap();
    compare(
        &cpu,
        &report.results,
        if gpu.has_f64() { 2e-10 } else { 1e-4 },
    );
    if let CollisionModel::Sds(sds) = &mut model {
        sds.diffusion = true;
    }
    let mut prepared = gpu
        .prepare_physics(&e, &[], &magnetic, Some(&model), 813)
        .unwrap();
    let all = prepared.fly(&particles, &options).unwrap();
    let tail = prepared.fly_indexed(&particles[7..], &options, 7).unwrap();
    compare(&all.results[7..], &tail.results, 1e-15);
    let options = FlightOptions {
        duration_us: 0.2,
        initial_step_us: 0.001,
        max_step_us: 0.05,
        ..options
    };
    let all = prepared.fly(&particles, &options).unwrap();
    prepared.params.physics_padding[2] = 1;
    let single_trial = prepared.fly(&particles, &options).unwrap();
    compare(&all.results, &single_trial.results, 1e-15);
}

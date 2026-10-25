//! Opt-in, interleaved kernel experiments on the solved live templates.
use super::*;

fn pipeline(gpu: &Gpu, source: &str) -> wgpu::ComputePipeline {
    let shader = gpu
        .device
        .create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("kernel experiment"),
            source: wgpu::ShaderSource::Wgsl(precision::shader(source, gpu.wide).into()),
        });
    let layout = gpu
        .device
        .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&gpu.layout)],
            immediate_size: 0,
        });
    gpu.device
        .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("kernel experiment"),
            layout: Some(&layout),
            module: &shader,
            entry_point: Some("fly"),
            compilation_options: Default::default(),
            cache: None,
        })
}

fn shader_metrics(label: &str, source: &str, wide: bool) {
    use wgpu::naga::{self, AddressSpace, TypeInner};
    let source = precision::shader(source, wide);
    let module = naga::front::wgsl::parse_str(&source).unwrap();
    let info = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .unwrap();
    let options = naga::back::spv::Options {
        flags: naga::back::spv::Options::default().flags | naga::back::spv::WriterFlags::DEBUG,
        ..Default::default()
    };
    let words = naga::back::spv::write_vec(
        &module,
        &info,
        &options,
        Some(&naga::back::spv::PipelineOptions {
            shader_stage: naga::ShaderStage::Compute,
            entry_point: "fly".into(),
        }),
    )
    .unwrap();
    let mut variables = [0; 3];
    let mut instructions = 0;
    let mut float_types = std::collections::HashMap::new();
    let mut names = std::collections::HashMap::new();
    let mut values = std::collections::HashMap::<u32, (usize, usize, u32)>::new();
    let mut function = None;
    let mut offset = 5;
    while offset < words.len() {
        let count = (words[offset] >> 16) as usize;
        let opcode = words[offset] & 65535;
        let operands = &words[offset + 1..offset + count];
        match opcode {
            5 => {
                let bytes: Vec<_> = operands[1..].iter().flat_map(|w| w.to_le_bytes()).collect();
                let name = String::from_utf8_lossy(&bytes)
                    .trim_end_matches('\0')
                    .to_owned();
                names.insert(operands[0], name);
            }
            22 => {
                float_types.insert(operands[0], operands[1] / 32);
            }
            23 => {
                if let Some(slots) = float_types.get(&operands[1]).copied() {
                    float_types.insert(operands[0], slots * operands[2]);
                }
            }
            30 => {
                let slots = operands[1..]
                    .iter()
                    .filter_map(|id| float_types.get(id))
                    .sum::<u32>();
                if slots != 0 {
                    float_types.insert(operands[0], slots);
                }
            }
            54 => {
                function = Some(operands[1]);
                values.clear();
            }
            56 => {
                if let Some(name) = function.and_then(|id| names.get(&id))
                    && ["rk45", "instance_electric", "fly"].contains(&name.as_str())
                {
                    let mut events = Vec::new();
                    for &(start, end, slots) in values.values() {
                        if end > start {
                            events.push((start, i64::from(slots)));
                            events.push((end + 1, -i64::from(slots)));
                        }
                    }
                    events.sort_unstable();
                    let mut live = 0;
                    let mut peak = 0;
                    for (_, delta) in events {
                        live += delta;
                        peak = peak.max(live);
                    }
                    eprintln!("PROFILE {label} {name} linear_ssa_peak_32bit_slots={peak}");
                }
                function = None;
            }
            _ => {}
        }
        if function.is_some() && opcode != 54 {
            let definition = (operands.len() >= 2)
                .then(|| float_types.get(&operands[0]).copied())
                .flatten()
                .map(|slots| (operands[1], slots));
            for &operand in operands {
                if definition.is_some_and(|(id, _)| id == operand) {
                    continue;
                }
                if let Some(value) = values.get_mut(&operand) {
                    value.1 = offset;
                }
            }
            if let Some((id, slots)) = definition {
                values.insert(id, (offset, offset, slots));
            }
        }
        if opcode == 59 {
            match words[offset + 3] {
                6 => variables[0] += 1, // Private.
                7 => variables[1] += 1, // Function.
                4 => variables[2] += 1, // Workgroup.
                _ => {}
            }
        }
        instructions += 1;
        offset += count;
    }
    eprintln!(
        "PROFILE {label} SPIRV wide={wide} words={} instructions={instructions} variable_private/function/workgroup={variables:?}",
        words.len()
    );
    for (_, global) in module.global_variables.iter() {
        if global.space == AddressSpace::Private
            && matches!(module.types[global.ty].inner, TypeInner::Array { .. })
        {
            eprintln!(
                "PROFILE {label} private_array {:?} {:?}",
                global.name, module.types[global.ty].inner
            );
        }
    }
    for (_, function) in module.functions.iter() {
        if function
            .name
            .as_deref()
            .is_some_and(|n| ["rk45", "instance_electric", "expand_cell"].contains(&n))
        {
            eprintln!(
                "PROFILE {label} function {:?} locals={} expressions={}",
                function.name,
                function.local_variables.len(),
                function.expressions.len()
            );
            for (_, local) in function.local_variables.iter() {
                if matches!(module.types[local.ty].inner, TypeInner::Array { .. }) {
                    eprintln!(
                        "PROFILE {label} local_array {:?} {:?}",
                        local.name, module.types[local.ty].inner
                    );
                }
            }
        }
    }
}

fn phase_replay(
    resident: &mut PreparedField<'_>,
    results: &[FlightResult],
    options: &FlightOptions,
    general: bool,
) {
    let replay: Vec<_> = results
        .iter()
        .filter_map(|r| r.trajectory.as_ref())
        .flat_map(|t| t.windows(2))
        .filter_map(|s| {
            let h = s[1].time_us - s[0].time_us;
            (h > 0.0).then(|| {
                Particle::new(100.0, 1.0, s[0].position_mm, s[0].velocity_mm_per_us, h).unwrap()
            })
        })
        .collect();
    let source = if general {
        include_str!("fly.wgsl").replace(
            "for (var x = i32(effective.x)",
            "for  (var x = i32(effective.x)",
        )
    } else {
        include_str!("fly.wgsl").to_owned()
    };
    let prefix = &source[..source
        .find("@compute @workgroup_size(64)\nfn fly(")
        .unwrap()];
    for (name, body) in [
        (
            "localization",
            "let at = moved(q,u*h); let r = reflected(at); result = physical(r);",
        ),
        ("field", "result = electric(q,0.0);"),
        (
            "rk_stages",
            "let trial = rk45(q,u,h,ion.metadata.x,0.0); result = trial.phase.d + trial.phase.u + vec3<f32>(trial.error);",
        ),
        (
            "step_control",
            "let error = max(length(u)*h,1e-8); let factor = clamp(0.9*pow(error,-0.2),0.2,5.0); result = vec3<f32>(clamp(h*factor,p.integration.z,p.integration.w));",
        ),
        (
            "termination",
            "let hit = intersection(q,u*h); result = vec3<f32>(hit.t,f32(hit.code),f32(hit.index));",
        ),
    ] {
        let source = format!(
            "{prefix}\n\
@compute @workgroup_size(64)\n\
fn fly(@builtin(global_invocation_id) id: vec3<u32>) {{\n\
    p = root; if id.x >= p.counts.x {{ return; }}\n\
    let ion = ions[id.x]; var q = Coordinate(ion.cell.xyz,ion.offset.xyz);\n\
    let u = ion.velocity.xyz; let h = ion.metadata.y;\n\
    var total = vec3<f32>(0.0);\n\
    for (var repeat=0u; repeat<8u; repeat++) {{\n\
        var result = vec3<f32>(0.0); {body}\n\
        total += result; q.offset.x += 1e-7;\n\
    }}\n\
    terminal.states[id.x] = EndState(vec4<i32>(q.cell,0),vec4<u32>(0u,0u,4u,0u),\n\
        vec4<f32>(q.offset,0.0),vec4<f32>(total,0.0));\n\
}}"
        );
        resident.waveform_pipeline = Some(pipeline(resident.gpu, &source));
        let mut times = Vec::new();
        for run in 0..5 {
            let report = resident.fly(&replay, options).unwrap();
            if run >= 2 {
                times.push(report.timings.fly.as_secs_f64() * 1e9 / (replay.len() * 8) as f64);
            }
        }
        times.sort_by(f64::total_cmp);
        eprintln!(
            "PROFILE {name} general={general} replay_states={} median_ns_per_call={:.6}",
            replay.len(),
            times[1]
        );
    }
    resident.waveform_pipeline = None;
}

#[test]
#[ignore = "requires SIMREP_GPU_KERNEL_BENCH (solved templates and hardware Vulkan timestamps)"]
fn kernel_experiments() {
    let directory =
        std::env::var("SIMREP_GPU_KERNEL_BENCH").expect("requires SIMREP_GPU_KERNEL_BENCH");
    let precision = if std::env::var_os("SIMREP_GPU_PREVIEW").is_some() {
        Precision::Preview
    } else {
        Precision::Precise
    };
    let gpu = pollster::block_on(Gpu::new_with_precision(Backends::VULKAN, precision))
        .unwrap()
        .expect("kernel benchmark requires Vulkan");
    assert_ne!(gpu.info.device_type, DeviceType::Cpu);
    assert!(gpu.has_timestamps());
    let source = include_str!("fly.wgsl");
    shader_metrics(
        "general",
        &source.replace(
            "for (var x = i32(effective.x)",
            "for  (var x = i32(effective.x)",
        ),
        gpu.wide,
    );
    shader_metrics("unroll_x", source, gpu.wide);
    let candidates = [
        // Whitespace bypasses the production loop specialization so the
        // unchanged general loop remains an independently compiled control.
        (
            "general",
            pipeline(
                &gpu,
                &source.replace(
                    "for (var x = i32(effective.x)",
                    "for  (var x = i32(effective.x)",
                ),
            ),
        ),
        ("unroll_x", gpu.fly.clone()),
    ];
    for (name, relative) in [
        ("einzel", "einzel/einzel.pa0"),
        ("quadrupole", "phase7b/quad/quadrupole.pa0"),
    ] {
        let path = std::path::Path::new(&directory).join(relative);
        let file = simrep_io::read_pa(&path).unwrap();
        let reference = file.pa0.as_ref().unwrap().reference_voltage;
        let bases: Vec<_> = (1..=3)
            .map(|i| {
                let mut array = simrep_io::read_pa(path.with_extension(format!("pa{i}")))
                    .unwrap()
                    .array;
                array.potentials.iter_mut().for_each(|v| *v /= reference);
                GridField::new_compact(array).unwrap()
            })
            .collect();
        let field = InstanceField::new(
            GridField::new_compact(file.array).unwrap(),
            [0.0; 3],
            [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            1.0,
            None,
            None,
        )
        .unwrap();
        let mut resident = gpu.prepare(&field, &bases).unwrap();
        resident.cache_basis_powers().unwrap();
        resident.set_voltages(&[1.0, 1.05, 0.0, 0.0]).unwrap();
        eprintln!(
            "KERNEL {name} precision={precision:?} dims={:?} degrees={:?} field_bytes={}",
            resident.params.dims,
            resident.params.degrees,
            resident.basis.size()
        );
        for count in [10_000, 100_000] {
            let golden = std::f64::consts::PI * (3.0 - 5.0_f64.sqrt());
            let particles: Vec<_> = (0..count)
                .map(|i| {
                    let radius = 3.0 * ((i as f64 + 0.5) / count as f64).sqrt();
                    let (sin, cos) = (i as f64 * golden).sin_cos();
                    Particle::from_relativistic_energy(
                        100.0,
                        1.0,
                        [1.0, radius * cos, radius * sin],
                        100.0,
                        [1.0, 0.0, 0.0],
                        0.0,
                    )
                    .unwrap()
                })
                .collect();
            let options = FlightOptions {
                relative_tolerance: 1e-8,
                position_tolerance_mm: 1e-8,
                velocity_tolerance_mm_per_us: 1e-8,
                initial_step_us: 0.001,
                max_step_us: 0.1,
                duration_us: 100.0,
                relativistic: true,
                ..Default::default()
            };
            let indices: Vec<_> = (0..300).map(|i| i * count / 300).collect();
            let sampling = TrajectorySampling {
                ion_indices: &indices,
                every: 1,
                max_samples_per_ion: 400,
            };
            resident.waveform_pipeline = Some(candidates[0].1.clone());
            let control = resident
                .fly_sampled(&particles, &options, &sampling)
                .unwrap();
            let localization_trials = if gpu.wide {
                format!("{}..{}", count * 27, count * 49)
            } else {
                (count * 27).to_string()
            };
            eprintln!(
                "PROFILE {name} accepted={} rejected={} terminal_localization_trials={}",
                control
                    .results
                    .iter()
                    .map(|r| r.accepted_steps)
                    .sum::<usize>(),
                control
                    .results
                    .iter()
                    .map(|r| r.rejected_steps)
                    .sum::<usize>(),
                localization_trials
            );
            if count == 100_000 && std::env::var_os("SIMREP_GPU_PHASES").is_some() {
                phase_replay(&mut resident, &control.results, &options, true);
                phase_replay(&mut resident, &control.results, &options, false);
            }
            let mut measurements = vec![Vec::new(); candidates.len()];
            for run in 0..8 {
                for index in 0..candidates.len() {
                    // Reverse alternate rounds to avoid a clock/order advantage.
                    let index = if run % 2 == 0 {
                        index
                    } else {
                        candidates.len() - 1 - index
                    };
                    let (candidate, shader) = &candidates[index];
                    resident.waveform_pipeline = Some(shader.clone());
                    let report = resident
                        .fly_sampled(&particles, &options, &sampling)
                        .unwrap();
                    let mut endpoint = 0.0_f64;
                    let mut tof = 0.0_f64;
                    let mut step_changes = 0;
                    for (a, b) in control.results.iter().zip(&report.results) {
                        assert_eq!(a.splat_reason, b.splat_reason, "{name} {candidate}");
                        endpoint = endpoint.max(
                            a.final_state
                                .position_mm
                                .iter()
                                .zip(b.final_state.position_mm)
                                .map(|(a, b)| (a - b).powi(2))
                                .sum::<f64>()
                                .sqrt(),
                        );
                        tof = tof.max((a.tof_us - b.tof_us).abs());
                        step_changes += usize::from(
                            a.accepted_steps != b.accepted_steps
                                || a.rejected_steps != b.rejected_steps,
                        );
                    }
                    super::tests::assert_flight_bits(&control.results, &report.results);
                    if run >= 5 {
                        let ms = report.timings.fly.as_secs_f64() * 1000.0;
                        measurements[index].push(ms);
                        eprintln!(
                            "KERNEL {name} ions={count} {candidate} run={} ms={ms:.6} endpoint_mm={endpoint:.6e} tof_us={tof:.6e} step_changes={step_changes}",
                            run - 5
                        );
                    }
                }
            }
            for ((candidate, _), mut times) in candidates.iter().zip(measurements) {
                times.sort_by(f64::total_cmp);
                eprintln!(
                    "KERNEL {name} ions={count} {candidate} median_ms={:.6}",
                    times[1]
                );
            }
        }
    }
}

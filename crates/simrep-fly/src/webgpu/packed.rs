//! Browser f32 flight ABI. Reconstruction/packing happens before GPU submission.
//! Layouts are the native Parameters<f32>, Ion<f32> and EndState<f32> layouts.
use serde::{Deserialize, Serialize};
use simrep_solve::gpu::GpuResult;
use simrep_solve::webgpu::{Gpu, Timer};
use wgpu::util::DeviceExt;

#[derive(Deserialize, Serialize)]
pub struct FlightInput {
    pub parameter_bytes: Vec<u8>,
    pub coefficient_f32: Vec<f32>,
    pub mask_u32: Vec<u32>,
    pub ion_bytes: Vec<u8>,
}
#[derive(Serialize)]
pub struct FlightOutput {
    pub end_state_bytes: Vec<u8>,
    pub compute_ms: Option<f64>,
}

pub async fn fly(gpu: &Gpu, input: &FlightInput) -> GpuResult<FlightOutput> {
    let p = &input.parameter_bytes;
    if p.len() != 640 {
        return Err("f32 flight parameters must contain exactly 640 bytes".into());
    }
    let word = |offset: usize| u32::from_le_bytes(p[offset..offset + 4].try_into().unwrap());
    let count = word(224) as usize;
    let coefficient_count = word(28) as usize;
    let bases = word(44) as usize;
    if count == 0
        || input.ion_bytes.len() != count.checked_mul(144).ok_or("ion size overflow")?
        || bases == 0
        || bases > 16
        || input.coefficient_f32.len()
            != coefficient_count
                .checked_mul(bases)
                .ok_or("basis size overflow")?
        || word(236) != 0
        || word(244) != 0
        || (608..640).step_by(4).any(|offset| word(offset) != 0)
        || !input.coefficient_f32.iter().all(|v| v.is_finite())
    {
        return Err("invalid packed static staircase f32 flight input".into());
    }
    let nodes = (0..12)
        .step_by(4)
        .try_fold(1usize, |n, offset| n.checked_mul(word(offset) as usize))
        .ok_or("node count overflow")?;
    let cells = (16..28)
        .step_by(4)
        .try_fold(1usize, |n, offset| n.checked_mul(word(offset) as usize))
        .ok_or("cell count overflow")?;
    if nodes == 0
        || cells == 0
        || input.mask_u32.len() != nodes.checked_add(cells).ok_or("mask size overflow")?
    {
        return Err("packed mask must contain all node flags and cell degrees".into());
    }
    let groups = count.div_ceil(64);
    if groups > gpu.device.limits().max_compute_workgroups_per_dimension as usize {
        return Err("packed ion count exceeds WebGPU dispatch limit".into());
    }
    let scope = gpu.device.push_error_scope(wgpu::ErrorFilter::Validation);
    let layout = gpu
        .device
        .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("packed f32 flight"),
            entries: &(0..9)
                .map(|binding| wgpu::BindGroupLayoutEntry {
                    binding,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: if binding == 0 {
                            wgpu::BufferBindingType::Uniform
                        } else {
                            wgpu::BufferBindingType::Storage {
                                read_only: [1, 3, 5, 8].contains(&binding),
                            }
                        },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                })
                .collect::<Vec<_>>(),
        });
    let pipeline_layout = gpu
        .device
        .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
    let shader = gpu
        .device
        .create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("portable staircase RK45"),
            source: wgpu::ShaderSource::Wgsl(shader_source().into()),
        });
    let pipeline = |name| {
        gpu.device
            .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(name),
                layout: Some(&pipeline_layout),
                module: &shader,
                entry_point: Some(name),
                compilation_options: Default::default(),
                cache: None,
            })
    };
    let combine = pipeline("combine");
    let flight = pipeline("fly");
    if let Some(error) = scope.pop().await {
        return Err(error.into());
    }
    let uniform = gpu
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("f32 parameters"),
            contents: p,
            usage: wgpu::BufferUsages::UNIFORM,
        });
    let basis = gpu.storage(&input.coefficient_f32, "field powers")?;
    let combined = gpu.storage(&vec![0f32; coefficient_count], "combined powers")?;
    let mask = gpu.storage(&input.mask_u32, "mask")?;
    let ions = gpu.storage(&input.ion_bytes, "ions")?;
    let instances = gpu.storage(p, "instances")?;
    let terminal = gpu.storage(&vec![0u32; 8 + count * 16], "terminal states")?;
    let samples = gpu.storage(&[0u32; 16], "unused samples")?;
    let physics = gpu.storage(&[0u32; 8], "unused physics")?;
    let bindings = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &layout,
        entries: [
            &uniform, &basis, &combined, &mask, &ions, &instances, &terminal, &samples, &physics,
        ]
        .into_iter()
        .enumerate()
        .map(|(binding, buffer)| wgpu::BindGroupEntry {
            binding: binding as u32,
            resource: buffer.as_entire_binding(),
        })
        .collect::<Vec<_>>()
        .as_slice(),
    });
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&combine);
        pass.set_bind_group(0, &bindings, &[]);
        let groups = coefficient_count.div_ceil(64) as u32;
        pass.dispatch_workgroups(groups.min(65535), groups.div_ceil(65535), 1);
    }
    let timer = Timer::new(gpu);
    {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("flight compute only"),
            timestamp_writes: timer
                .as_ref()
                .map(|timer| wgpu::ComputePassTimestampWrites {
                    query_set: &timer.queries,
                    beginning_of_pass_write_index: Some(0),
                    end_of_pass_write_index: Some(1),
                }),
        });
        pass.set_pipeline(&flight);
        pass.set_bind_group(0, &bindings, &[]);
        pass.dispatch_workgroups(groups as u32, 1, 1);
    }
    if let Some(timer) = &timer {
        timer.resolve(&mut encoder);
    }
    gpu.queue.submit([encoder.finish()]);
    let end_state_bytes = gpu.read::<u8>(&terminal, 32, count * 64).await?;
    for end in end_state_bytes.as_chunks::<64>().0 {
        let reason = u32::from_le_bytes(end[24..28].try_into().unwrap());
        if !(1..=7).contains(&reason)
            || end[32..]
                .as_chunks::<4>()
                .0
                .iter()
                .any(|v| !f32::from_le_bytes(*v).is_finite())
        {
            return Err("GPU returned unfinished or nonfinite terminal states".into());
        }
    }
    let compute_ms = match timer {
        Some(timer) => Some(timer.milliseconds(gpu).await?),
        None => None,
    };
    Ok(FlightOutput {
        end_state_bytes,
        compute_ms,
    })
}

pub fn shader_source() -> String {
    let source = include_str!("../gpu/fly.wgsl");
    let start = source.find("// SDF coordinates").unwrap();
    let end = source
        .find("// WGSL permits floating-point reassociation;")
        .unwrap();
    let source = format!("{}{}", &source[..start], &source[end..])
        .replace("const SURFACE_SDF: bool = false;\nconst SURFACE_SDF_REPLACE: bool = false;\n", "")
        .replace("// SDF_FUNCTIONS\n@group(0) @binding(9) var<storage, read> sdf_headers: array<sdf_Volume>;\n@group(0) @binding(10) var<storage, read> sdf_samples: array<f32>;\n\n", "")
        .replace("return Event(0.0,select((packed>>12u)&1u,0u,SURFACE_SDF_REPLACE),index);", "return Event(0.0,(packed>>12u)&1u,index);")
        .replace("return Event(0.0, select(metal[index + select(0u, p.counts.x, WORKBENCH)],0u,SURFACE_SDF_REPLACE), index);", "return Event(0.0, metal[index + select(0u, p.counts.x, WORKBENCH)], index);")
        .replace("fn base_status(", "fn status(").replace("fn base_intersection(", "fn intersection(");
    let mut physics = include_str!("../gpu/physics.wgsl").to_owned();
    let start = physics.find("fn imported_local(").unwrap();
    let end = physics.find("fn coil_value(").unwrap();
    physics.replace_range(start..end, "fn imported_status(point: vec3<f32>) -> Event { return Event(0.0,0u,0u); }\nfn imported_intersection(point: vec3<f32>, delta: vec3<f32>) -> Event { return Event(0.0,0u,0u); }\nfn imported_field(point: vec3<f32>) -> vec3<f32> { return vec3<f32>(0.0); }\n");
    // WGSL constant expressions cannot contain NaN. Keep unused native physics
    // helpers' failure values as runtime expressions for nonempty flights.
    source.replace("// PHYSICS_FUNCTIONS", &physics)
        .replace("bitcast<f32>(0x7fc00000u)", "bitcast<f32>(select(0u, 0x7fc00000u, root.counts.x != 0u))")
        .replace("96.48533215665327", &format!("{:.17e}", crate::ELECTRIC_ACCELERATION))
        .replace("0.009648533215665327", &format!("{:.17e}", crate::MAGNETIC_FREQUENCY))
        .replace("h = min(h, remaining);", "h = min(h, remaining);\n        if h == remaining && h > 0.0 {\n            let end = time_add(time, h);\n            let after = (p.integration.x - end.x) + (p.tolerance.w - end.y);\n            if after < 0.0 { h = bitcast<f32>(bitcast<u32>(h) - 1u); }\n        }")
        .replace("var<private> p: Parameters;", "").replace("p = root;", "").replace("p=root;", "")
        .replace("p = instances[i];", "").replace("p=instances[i];", "").replace("p.", "root.")
}

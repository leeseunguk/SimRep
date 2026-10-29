//! Resident sampled surfaces for the flight pipeline. The standalone segment
//! pipeline shares both scalar packing and the device intersection functions.
use super::*;

pub(super) struct SdfBuffers {
    pub(super) headers: wgpu::Buffer,
    pub(super) samples: wgpu::Buffer,
    pub(super) layout: wgpu::BindGroupLayout,
    pub(super) combine: Option<wgpu::ComputePipeline>,
}

impl Gpu {
    pub(super) fn prepare_sdf(&self, field: &InstanceField) -> GpuResult<Option<SdfBuffers>> {
        let Some(sdf) = &field.surface_sdf else {
            if field.surface.is_some() {
                return Err(
                    "surface collisions require --surface-backend sdf for GPU flying".into(),
                );
            }
            return Ok(None);
        };
        if sdf.volumes.is_empty() {
            return Ok(None);
        }
        if self.device.limits().max_storage_buffers_per_shader_stage < 10 {
            return Err("GPU SDF flight requires 10 storage buffers per shader stage".into());
        }
        let wide = self.has_f64();
        let size = sdf
            .volumes
            .iter()
            .map(|v| v.samples_f64.len() as u64)
            .sum::<u64>()
            * if wide { 8 } else { 4 };
        let header_size = sdf.volumes.len() as u64 * if wide { 128 } else { 80 };
        crate::sdf_hit::check_storage(size.max(header_size), &self.device.limits())?;
        let origin = field.surface.as_ref().unwrap().source_point([0.0; 3]);
        let packed = crate::sdf_hit::pack(sdf, wide, origin, field.scale)?;
        let buffer = |label, contents: &[u8]| {
            self.device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some(label),
                    contents,
                    usage: wgpu::BufferUsages::STORAGE,
                })
        };
        let headers = buffer("resident SDF headers", &packed.headers);
        let samples = buffer("resident SDF samples", &packed.samples);
        let layout = self
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("SDF flight buffers"),
                entries: &(0..11)
                    .map(|binding| wgpu::BindGroupLayoutEntry {
                        binding,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: if binding == 0 {
                                wgpu::BufferBindingType::Uniform
                            } else {
                                wgpu::BufferBindingType::Storage {
                                    read_only: matches!(binding, 1 | 3 | 5 | 8 | 9 | 10),
                                }
                            },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    })
                    .collect::<Vec<_>>(),
            });
        Ok(Some(SdfBuffers {
            headers,
            samples,
            layout,
            combine: None,
        }))
    }
}

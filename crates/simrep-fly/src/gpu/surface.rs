//! Resident sampled surfaces for the flight pipeline. The standalone segment
//! pipeline shares both scalar packing and the device intersection functions.
use super::*;

#[derive(Clone)]
pub(super) struct SdfBuffers {
    pub(super) headers: wgpu::Buffer,
    pub(super) samples: wgpu::Buffer,
    pub(super) layout: wgpu::BindGroupLayout,
    pub(super) combine: Option<wgpu::ComputePipeline>,
}

pub(super) struct SdfRemainder<'a> {
    pub(super) prepared: PreparedField<'a>,
    pub(super) particles: Vec<Particle>,
    pub(super) options: FlightOptions,
    pub(super) launch: bool,
    pub(super) indices: Vec<usize>,
    pub(super) sampling: Option<(usize, usize)>,
    pub(super) begin: usize,
    pub(super) ion_offset: u64,
    pub(super) ion_stride: u64,
}

impl Drop for SdfRemainder<'_> {
    fn drop(&mut self) {
        self.prepared.ion_buffers.lock().unwrap().bytes -=
            self.prepared.combined.size() + self.prepared.instances.size();
    }
}

pub(super) fn trace_dispatch(count: usize, elapsed: Duration) {
    if cfg!(feature = "precision-study") && std::env::var_os("SIMREP_SDF_DISPATCH_TRACE").is_some()
    {
        eprintln!(
            "sdf_dispatch ions={count} gpu_ms={:.6}",
            elapsed.as_secs_f64() * 1000.0
        );
    }
}

// Aim below 250 ms, leaving headroom for variation between independent chunks.
// Limit growth to 2x; a slow dispatch can shrink immediately to a single ion.
fn next_capacity(count: usize, elapsed: Duration, limit: usize) -> usize {
    let estimate = (count as f64 * 0.2 / elapsed.as_secs_f64().max(1e-9)) as usize;
    estimate.clamp(1, limit).min(count.saturating_mul(2))
}

impl SdfRemainder<'_> {
    pub(super) fn collect(
        mut self,
        mut count: usize,
        mut elapsed: Duration,
    ) -> GpuResult<FlightReport> {
        let mut report = FlightReport {
            results: Vec::with_capacity(self.particles.len()),
            timings: Timings::default(),
        };
        let mut offset = 0;
        while offset < self.particles.len() {
            let limit = self.prepared.max_ions_per_dispatch().min(
                if self.prepared.gpu.wide || self.prepared.gpu.info.device_type == DeviceType::Cpu {
                    MAX_IONS_PER_DISPATCH
                } else {
                    PREFERRED_IONS_PER_DISPATCH
                },
            );
            let capacity = next_capacity(count, elapsed, limit);
            let end = (offset + capacity).min(self.particles.len());
            count = end - offset;
            let begin = self.begin + offset;
            let indices: Vec<_> = self
                .indices
                .iter()
                .copied()
                .filter(|&i| i >= begin && i < begin + count)
                .map(|i| i - begin)
                .collect();
            let sampling = self
                .sampling
                .map(|(every, max_samples_per_ion)| TrajectorySampling {
                    ion_indices: &indices,
                    every,
                    max_samples_per_ion,
                });
            let mut batch = self.prepared.submit_batch(
                &self.particles[offset..end],
                &self.options,
                self.launch,
                sampling.as_ref(),
                self.ion_offset + begin as u64 * self.ion_stride,
                self.ion_stride,
            )?;
            let result = batch.collect(
                self.prepared.gpu,
                self.prepared.field,
                self.options.relativistic,
            );
            let longest_dispatch = batch.longest_dispatch;
            if batch.needs_unmap {
                batch.buffers.readback.unmap();
            }
            self.prepared
                .ion_buffers
                .lock()
                .unwrap()
                .idle
                .push(batch.buffers);
            let mut chunk = result?;
            elapsed = longest_dispatch;
            report.results.append(&mut chunk.results);
            report.timings.setup += chunk.timings.setup;
            report.timings.upload += chunk.timings.upload;
            report.timings.combine += chunk.timings.combine;
            report.timings.fly += chunk.timings.fly;
            report.timings.readback += chunk.timings.readback;
            offset = end;
        }
        Ok(report)
    }
}

impl<'a> PreparedField<'a> {
    // Deferred adaptive chunks must not see a later set_voltages/prepare call.
    // Basis and surface buffers are immutable; copy mutable instance parameters
    // and give the deferred flight its own adjusted-cell cache.
    pub(super) fn sdf_snapshot(&self) -> PreparedField<'a> {
        let gpu = self.gpu;
        let combined = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("adaptive SDF cell powers"),
            size: self.combined.size(),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let instances = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("adaptive SDF instance snapshot"),
            size: self.instances.size(),
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(&self.instances, 0, &instances, 0, self.instances.size());
        gpu.queue.submit([encoder.finish()]);
        self.ion_buffers.lock().unwrap().bytes += combined.size() + instances.size();
        PreparedField {
            gpu,
            field: self.field,
            params: self.params,
            basis: self.basis.clone(),
            combined,
            mask: self.mask.clone(),
            uniform: self.uniform.clone(),
            coefficient_count: self.coefficient_count,
            dirty: true,
            expressions: self.expressions.clone(),
            programs: self.programs.clone(),
            magnetic_expressions: self.magnetic_expressions.clone(),
            integrator: self.integrator,
            waveform_pipeline: self.waveform_pipeline.clone(),
            sdf: self.sdf.clone(),
            instances,
            instance_count: self.instance_count,
            physics_enabled: self.physics_enabled,
            nodal_physics: self.nodal_physics,
            imported_maps: self.imported_maps,
            collision: self.collision,
            physics_data: self.physics_data.clone(),
            ion_buffers: Arc::clone(&self.ion_buffers),
            dispatch_capacity: self.dispatch_capacity,
            timings: Timings::default(),
        }
    }
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
        if self.info.backend == wgpu::Backend::Dx12 {
            return Err("GPU SDF surfaces are not supported on the DX12 backend (FXC cannot compile the shader); use --backend vulkan (default) or --device cpu".into());
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adaptive_capacity_grows_shrinks_and_respects_limits() {
        assert_eq!(next_capacity(1024, Duration::from_millis(50), 32768), 2048);
        assert_eq!(next_capacity(1024, Duration::from_millis(400), 32768), 512);
        assert_eq!(next_capacity(1024, Duration::from_millis(50), 1200), 1200);
        assert_eq!(next_capacity(1, Duration::from_secs(1), 32768), 1);
        assert_eq!(next_capacity(1024, Duration::ZERO, 32768), 2048);
    }
}

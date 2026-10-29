//! Queued flights own their mutable GPU state until collection or drop.
use super::*;

#[derive(Default)]
pub(super) struct IonBufferPool {
    pub(super) idle: Vec<IonBuffers>,
    pub(super) bytes: u64,
}
impl IonBuffers {
    fn bytes(&self) -> u64 {
        [
            &self.ions,
            &self.terminal,
            &self.samples,
            &self.readback,
            &self.uniform,
            &self.instances,
        ]
        .iter()
        .map(|buffer| buffer.size())
        .sum::<u64>()
            + self
                .timestamps
                .as_ref()
                .map_or(0, |(_, buffer)| buffer.size())
    }
}
struct Sampling {
    ion_indices: Vec<usize>,
    max_samples_per_ion: usize,
}

/// Submitted GPU work. Collection waits, resumes bounded physics dispatches,
/// and decodes ordered results. No mutable PreparedField borrow is retained.
/// Dropping discards the result and returns scratch buffers for queue-ordered
/// reuse; it does not cancel work already submitted to the GPU.
#[must_use = "collect the submitted flight to retrieve its results"]
pub struct PendingFlight<'a> {
    gpu: &'a Gpu,
    field: &'a InstanceField,
    relativistic: bool,
    pool: Arc<Mutex<IonBufferPool>>,
    batches: Vec<PendingBatch>,
}
struct PendingBatch {
    buffers: IonBuffers,
    bindings: wgpu::BindGroup,
    pipeline: wgpu::ComputePipeline,
    combine_pipeline: wgpu::ComputePipeline,
    physics: bool,
    groups: u32,
    timestamp_count: u32,
    output_size: u64,
    sample_size: u64,
    birth_times: Vec<f64>,
    sampling: Option<Sampling>,
    submission: Option<wgpu::SubmissionIndex>,
    mapping: Option<mpsc::Receiver<Result<(), wgpu::BufferAsyncError>>>,
    needs_unmap: bool,
    started: Instant,
    fly_ticks: u64,
    timings: Timings,
}
impl PendingFlight<'_> {
    pub fn collect(mut self) -> GpuResult<FlightReport> {
        let mut results =
            Vec::with_capacity(self.batches.iter().map(|b| b.birth_times.len()).sum());
        let mut timings = Timings::default();
        for batch in &mut self.batches {
            let mut report = batch.collect(self.gpu, self.field, self.relativistic)?;
            results.append(&mut report.results);
            timings.setup += report.timings.setup;
            timings.upload += report.timings.upload;
            timings.combine += report.timings.combine;
            timings.fly += report.timings.fly;
            timings.readback += report.timings.readback;
        }
        Ok(FlightReport { results, timings })
    }
}
impl Drop for PendingFlight<'_> {
    fn drop(&mut self) {
        let mut pool = self.pool.lock().unwrap();
        for batch in self.batches.drain(..) {
            if batch.needs_unmap {
                batch.buffers.readback.unmap();
            }
            pool.idle.push(batch.buffers);
        }
    }
}
impl<'a> PreparedField<'a> {
    pub fn fly(
        &mut self,
        particles: &[Particle],
        options: &FlightOptions,
    ) -> GpuResult<FlightReport> {
        self.submit_internal(particles, options, false, None, 0, 1)?
            .collect()
    }
    /// Continue a seeded input stream across caller-controlled batches.
    /// `first_ion_index` is the index of `particles[0]` in the complete beam.
    pub fn fly_indexed(
        &mut self,
        particles: &[Particle],
        options: &FlightOptions,
        first_ion_index: u64,
    ) -> GpuResult<FlightReport> {
        self.submit_internal(particles, options, false, None, first_ion_index, 1)?
            .collect()
    }
    /// Fly an interleaved partition using absolute Philox stream identities.
    /// Sampling indices address the local particle slice, not the complete beam.
    pub fn fly_interleaved(
        &mut self,
        particles: &[Particle],
        options: &FlightOptions,
        first_ion_index: u64,
        ion_stride: u64,
        sampling: Option<&TrajectorySampling<'_>>,
    ) -> GpuResult<FlightReport> {
        self.submit_internal(
            particles,
            options,
            false,
            sampling,
            first_ion_index,
            ion_stride,
        )?
        .collect()
    }
    /// Explicit emission from initial metal, matching the CPU launch API.
    pub fn fly_from_electrode(
        &mut self,
        particles: &[Particle],
        options: &FlightOptions,
    ) -> GpuResult<FlightReport> {
        self.submit_internal(particles, options, true, None, 0, 1)?
            .collect()
    }
    pub fn fly_sampled(
        &mut self,
        particles: &[Particle],
        options: &FlightOptions,
        sampling: &TrajectorySampling<'_>,
    ) -> GpuResult<FlightReport> {
        self.submit_internal(particles, options, false, Some(sampling), 0, 1)?
            .collect()
    }
    pub fn fly_from_electrode_sampled(
        &mut self,
        particles: &[Particle],
        options: &FlightOptions,
        sampling: &TrajectorySampling<'_>,
    ) -> GpuResult<FlightReport> {
        self.submit_internal(particles, options, true, Some(sampling), 0, 1)?
            .collect()
    }
    /// Enqueue a flight without waiting for GPU completion. The returned handle
    /// owns its input metadata and scratch buffers; it does not borrow this
    /// prepared field, the particles, or options. Voltages and pipeline selection
    /// are snapshotted, so another request can be prepared/submitted immediately.
    pub fn submit(
        &mut self,
        particles: &[Particle],
        options: &FlightOptions,
    ) -> GpuResult<PendingFlight<'a>> {
        self.submit_internal(particles, options, false, None, 0, 1)
    }
    /// Nonblocking counterpart of fly_indexed, retaining absolute RNG indices.
    pub fn submit_indexed(
        &mut self,
        particles: &[Particle],
        options: &FlightOptions,
        first_ion_index: u64,
    ) -> GpuResult<PendingFlight<'a>> {
        self.submit_internal(particles, options, false, None, first_ion_index, 1)
    }
    pub fn submit_from_electrode(
        &mut self,
        particles: &[Particle],
        options: &FlightOptions,
    ) -> GpuResult<PendingFlight<'a>> {
        self.submit_internal(particles, options, true, None, 0, 1)
    }
    pub fn submit_sampled(
        &mut self,
        particles: &[Particle],
        options: &FlightOptions,
        sampling: &TrajectorySampling<'_>,
    ) -> GpuResult<PendingFlight<'a>> {
        self.submit_internal(particles, options, false, Some(sampling), 0, 1)
    }
    pub fn submit_from_electrode_sampled(
        &mut self,
        particles: &[Particle],
        options: &FlightOptions,
        sampling: &TrajectorySampling<'_>,
    ) -> GpuResult<PendingFlight<'a>> {
        self.submit_internal(particles, options, true, Some(sampling), 0, 1)
    }
    fn submit_internal(
        &mut self,
        particles: &[Particle],
        options: &FlightOptions,
        launch: bool,
        sampling: Option<&TrajectorySampling<'_>>,
        ion_offset: u64,
        ion_stride: u64,
    ) -> GpuResult<PendingFlight<'a>> {
        options.validate()?;
        if ion_stride == 0
            || (particles.len().saturating_sub(1) as u64)
                .checked_mul(ion_stride)
                .and_then(|n| ion_offset.checked_add(n))
                .is_none()
        {
            return Err("GPU ion indices require a positive stride without u64 overflow".into());
        }
        if self.integrator != options.integrator {
            self.integrator = options.integrator;
            let programs = if self.programs.is_empty() {
                vec![self.expressions.clone()]
            } else {
                self.programs.clone()
            };
            self.compile_expressions(&programs)?;
        }
        if precision::study_is("tof") && self.expressions.iter().any(Option::is_some) {
            return Err("the split-TOF rounding study is restricted to static fields".into());
        }
        if self.collision.is_some() && options.relativistic {
            return Err("neutral gas collision models require nonrelativistic dynamics".into());
        }
        if options.sample_every.is_some() {
            return Err(
                "use fly_sampled with TrajectorySampling for selected GPU trajectories".into(),
            );
        }
        if let Some(s) = sampling
            && (s.every == 0
                || s.every > u32::MAX as usize
                || s.max_samples_per_ion < 2
                || s.max_samples_per_ion > i32::MAX as usize
                || s.ion_indices.last().is_some_and(|&i| i >= particles.len())
                || s.ion_indices.windows(2).any(|w| w[0] >= w[1]))
        {
            return Err(
                "GPU sampling needs ordered unique indices, a positive interval and capacity >= 2"
                    .into(),
            );
        }
        let mut pending = PendingFlight {
            gpu: self.gpu,
            field: self.field,
            relativistic: options.relativistic,
            pool: Arc::clone(&self.ion_buffers),
            batches: Vec::new(),
        };
        for (batch_index, particles) in particles.chunks(self.max_ions_per_dispatch()).enumerate() {
            let begin = batch_index * self.max_ions_per_dispatch();
            let indices: Vec<_> = sampling
                .map(|s| {
                    s.ion_indices
                        .iter()
                        .copied()
                        .filter(|&i| i >= begin && i < begin + particles.len())
                        .map(|i| i - begin)
                        .collect()
                })
                .unwrap_or_default();
            let batch_sampling = sampling.map(|s| TrajectorySampling {
                ion_indices: &indices,
                every: s.every,
                max_samples_per_ion: s.max_samples_per_ion,
            });
            pending.batches.push(self.submit_batch(
                particles,
                options,
                launch,
                batch_sampling.as_ref(),
                ion_offset + begin as u64 * ion_stride,
                ion_stride,
            )?);
        }
        Ok(pending)
    }
    fn submit_batch(
        &mut self,
        particles: &[Particle],
        options: &FlightOptions,
        launch: bool,
        sampling: Option<&TrajectorySampling<'_>>,
        ion_offset: u64,
        ion_stride: u64,
    ) -> GpuResult<PendingBatch> {
        let mut timings = Timings::default();
        let start = Instant::now();
        let field = self.field;
        let array = field.grid.array();
        let spacing = [array.dx_mm, array.dy_mm, array.dz_mm];
        let mut ions = vec![Ion::zeroed(); particles.len()];
        ions.par_iter_mut()
            .zip(particles.par_iter())
            .enumerate()
            .try_for_each(|(index, (out, particle))| -> GpuResult<()> {
                particle.validate(options.relativistic)?;
                let local = field.local(particle.initial.position_mm);
                let v = particle.initial.velocity_mm_per_us;
                let gamma = if options.relativistic {
                    1.0 / (1.0 - dot(v, v) / SPEED_OF_LIGHT.powi(2)).sqrt()
                } else {
                    1.0
                };
                let mut ion = Ion::zeroed();
                ion.padding0[0] = u32::MAX;
                for a in 0..3 {
                    let q = local[a] / spacing[a];
                    if q.floor() < i32::MIN as f64 || q.floor() > i32::MAX as f64 {
                        return Err("particle position exceeds i32 cell range".into());
                    }
                    ion.cell[a] = q.floor() as i32;
                    ion.offset[a] = q - q.floor();
                    ion.velocity[a] = gamma * dot(field.axes[a], v);
                }
                ion.metadata[0] = particle.charge_e / particle.mass_amu;
                ion.metadata[1] = if self.gpu.wide {
                    particle.initial.time_us
                } else {
                    f64::from(particle.initial.time_us as f32)
                };
                ion.metadata[2] = particle.initial.time_us - ion.metadata[1];
                ion.metadata[3] = if launch { 1.0 } else { 0.0 };
                physics::prepare_ion(
                    &mut ion,
                    particle,
                    self.collision,
                    ion_offset + index as u64 * ion_stride,
                )?;
                if !ion.metadata[1].is_finite() || !ion.metadata[2].is_finite() {
                    return Err("particle birth time exceeds GPU f32 range".into());
                }
                *out = ion;
                Ok(())
            })?;
        if let Some(s) = sampling {
            for (slot, &index) in s.ion_indices.iter().enumerate() {
                ions[index].padding0 =
                    [slot as u32, s.every as u32, s.max_samples_per_ion as u32, 0];
            }
        }
        let narrow = (!self.gpu.wide).then(|| {
            ions.par_iter()
                .map(|i| i.map(|v| v as f32))
                .collect::<Vec<_>>()
        });
        let ion_bytes = narrow.as_ref().map_or_else(
            || bytemuck::cast_slice(&ions),
            |ions| bytemuck::cast_slice(ions),
        );
        let size = ion_bytes.len() as u64;
        self.gpu.check_size(size)?;
        let output_size = particles.len() as u64 * if self.gpu.wide { 96 } else { 64 };
        let sample_size = sampling.map_or(0, |s| {
            s.ion_indices.len() as u64
                * s.max_samples_per_ion as u64
                * if self.gpu.wide { 64 } else { 32 }
        });
        self.gpu.check_size(sample_size)?;
        let groups = (ions.len() as u32).div_ceil(64);
        if groups
            > self
                .gpu
                .device
                .limits()
                .max_compute_workgroups_per_dimension
        {
            return Err("too many particles for one GPU dispatch".into());
        }
        self.params.integration = [
            if self.gpu.wide && !precision::study_is("tof") {
                options.duration_us
            } else {
                f64::from(options.duration_us as f32)
            },
            options
                .initial_step_us
                .min(options.duration_us.max(options.min_step_us)),
            options.min_step_us,
            options
                .max_step_us
                .min(options.duration_us.max(options.min_step_us))
                .min(1e30),
        ];
        self.params.padding[3] = u32::from(options.max_step_us == f64::MAX);
        self.params.step_limits = [
            options.step_limits.max_grid_cells,
            options.step_limits.max_cyclotron_radians,
            self.expressions
                .iter()
                .flatten()
                .filter_map(VoltageExpression::time_scale_us)
                .fold(1e30, f64::min)
                * options.step_limits.max_voltage_phase_radians,
            f64::from(options.integrator == Integrator::Boris),
        ];
        // The embedded difference is evaluated without subtracting full
        // positions. A roundoff floor still prevents requesting f64 accuracy
        // from f32 field powers and velocities.
        let floor = if self.gpu.wide
            && ![
                "tolerance",
                "evaluation",
                "phase",
                "transforms",
                "position",
                "velocity",
            ]
            .iter()
            .any(|name| precision::study_is(name))
        {
            1e-13
        } else {
            2e-7
        };
        self.params.tolerance = [
            options.relative_tolerance.max(floor),
            options.position_tolerance_mm.max(floor),
            options.velocity_tolerance_mm_per_us.max(floor),
            if self.gpu.wide && !precision::study_is("tof") {
                0.0
            } else {
                f64::from((options.duration_us - self.params.integration[0]) as f32)
            },
        ];
        if self
            .params
            .integration
            .iter()
            .any(|v| !v.is_finite() || *v < 0.0)
            || self.params.integration[2] == 0.0
            || self.params.tolerance.iter().any(|v| !v.is_finite())
        {
            return Err("GPU flight limits exceed f32 range".into());
        }
        self.params.stop_plane = [0.0; 4];
        if let Some(p) = options.stop_plane {
            for a in 0..3 {
                self.params.stop_plane[a] = field.axes[a][p.axis];
            }
            self.params.stop_plane[3] = p.coordinate_mm - field.origin_mm[p.axis];
        }
        self.params.counts = [
            ions.len() as u32,
            options.max_steps.min(u32::MAX as usize) as u32,
            u32::from(options.relativistic),
            if self.instance_count > 1 {
                2
            } else {
                u32::from(self.waveform_pipeline.is_some())
            },
        ];
        timings.setup = start.elapsed();
        let start = Instant::now();
        let gpu = self.gpu;
        let buffers = self.take_buffers(particles.len(), sample_size.max(64))?;
        gpu.queue.write_buffer(&buffers.ions, 0, ion_bytes);
        gpu.queue.write_buffer(
            &buffers.uniform,
            0,
            &parameters_bytes(&[self.params], gpu.wide),
        );
        let bindings = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("flight snapshot"),
            layout: self.sdf.as_ref().map_or(&gpu.layout, |s| &s.layout),
            entries: &[
                &buffers.uniform,
                &self.basis,
                &self.combined,
                &self.mask,
                &buffers.ions,
                &buffers.instances,
                &buffers.terminal,
                &buffers.samples,
                &self.physics_data,
            ]
            .into_iter()
            .chain(self.sdf.iter().flat_map(|s| [&s.headers, &s.samples]))
            .enumerate()
            .map(|(binding, buffer)| wgpu::BindGroupEntry {
                binding: binding as u32,
                resource: buffer.as_entire_binding(),
            })
            .collect::<Vec<_>>(),
        });
        let combine = self.dirty
            && self.instance_count == 1
            && !self.physics_enabled
            && self.expressions.iter().all(Option::is_none);
        let combine_count = if self.params.padding[1] != 0 {
            self.params.cells[..3].iter().product()
        } else {
            self.coefficient_count
        };
        let mut batch = PendingBatch {
            buffers,
            bindings,
            pipeline: self.waveform_pipeline.as_ref().unwrap_or(&gpu.fly).clone(),
            combine_pipeline: self
                .sdf
                .as_ref()
                .and_then(|s| s.combine.as_ref())
                .unwrap_or(&gpu.combine)
                .clone(),
            physics: self.physics_enabled,
            groups,
            timestamp_count: if combine { 4 } else { 2 },
            output_size,
            sample_size,
            birth_times: particles.iter().map(|p| p.initial.time_us).collect(),
            sampling: sampling.map(|s| Sampling {
                ion_indices: s.ion_indices.to_vec(),
                max_samples_per_ion: s.max_samples_per_ion,
            }),
            submission: None,
            mapping: None,
            needs_unmap: false,
            started: Instant::now(),
            fly_ticks: 0,
            timings,
        };
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        // Physics continuation must not observe a later voltage update.
        encoder.copy_buffer_to_buffer(
            &self.instances,
            0,
            &batch.buffers.instances,
            0,
            self.instances.size(),
        );
        // A skipped write must not expose a previous flight's pooled result.
        // Continuations retain completed ions, which return before writing again.
        encoder.clear_buffer(&batch.buffers.terminal, 32, Some(output_size));
        batch.dispatch(gpu, encoder, combine.then_some(combine_count));
        batch.timings.upload = start.elapsed();
        self.dirty = false;
        Ok(batch)
    }
    fn take_buffers(&self, count: usize, sample_bytes: u64) -> GpuResult<IonBuffers> {
        let gpu = self.gpu;
        let mut pool = self.ion_buffers.lock().unwrap();
        if let Some(index) = pool
            .idle
            .iter()
            .rposition(|b| b.capacity >= count && b.sample_bytes >= sample_bytes)
        {
            return Ok(pool.idle.swap_remove(index));
        }
        let previous = pool.idle.last();
        let mut capacity = previous
            .as_ref()
            .map_or(count, |b| b.capacity.max(count))
            .next_power_of_two()
            .min(gpu.ion_capacity());
        let sample_bytes = previous
            .as_ref()
            .map_or(sample_bytes, |b| b.sample_bytes.max(sample_bytes));
        gpu.check_size(sample_bytes)?;
        let end_size = if gpu.wide { 96 } else { 64 };
        if capacity as u64 * end_size + sample_bytes + 32 > gpu.device.limits().max_buffer_size {
            capacity = count;
        }
        let read_size = capacity as u64 * end_size + sample_bytes + 32;
        if read_size > gpu.device.limits().max_buffer_size {
            return Err("GPU trajectory/result readback exceeds device buffer limit".into());
        }
        if let Some(previous) = pool.idle.pop() {
            pool.bytes -= previous.bytes();
        }
        let buffer = |label, size, usage| {
            gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage,
                mapped_at_creation: false,
            })
        };
        let buffers = IonBuffers {
            capacity,
            sample_bytes,
            ions: buffer(
                "reusable ions",
                capacity as u64 * if gpu.wide { 224 } else { 144 },
                wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            ),
            terminal: buffer(
                "compact terminal states",
                32 + capacity as u64 * end_size,
                wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_SRC
                    | wgpu::BufferUsages::COPY_DST,
            ),
            samples: buffer(
                "selected trajectories",
                sample_bytes,
                wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            ),
            readback: buffer(
                "reusable result readback",
                read_size,
                wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            ),
            uniform: buffer(
                "flight parameter snapshot",
                self.uniform.size(),
                wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            ),
            instances: buffer(
                "flight instance snapshot",
                self.instances.size(),
                wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            ),
            timestamps: gpu.has_timestamps().then(|| {
                (
                    gpu.device.create_query_set(&wgpu::QuerySetDescriptor {
                        label: Some("flight timestamps"),
                        ty: wgpu::QueryType::Timestamp,
                        count: 4,
                    }),
                    buffer(
                        "flight timestamp resolve",
                        32,
                        wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                    ),
                )
            }),
        };
        pool.bytes += buffers.bytes();
        Ok(buffers)
    }
}
impl PendingBatch {
    fn timestamp_bytes(&self) -> u64 {
        if self.buffers.timestamps.is_some() {
            u64::from(self.timestamp_count) * 8
        } else {
            0
        }
    }
    fn copy_results(&self, encoder: &mut wgpu::CommandEncoder) -> u64 {
        let readback = &self.buffers.readback;
        encoder.copy_buffer_to_buffer(&self.buffers.terminal, 32, readback, 0, self.output_size);
        if self.sample_size != 0 {
            encoder.copy_buffer_to_buffer(
                &self.buffers.samples,
                0,
                readback,
                self.output_size,
                self.sample_size,
            );
        }
        let timestamp_bytes = if self.physics {
            0
        } else {
            self.timestamp_bytes()
        };
        if let Some((_, resolve)) = &self.buffers.timestamps
            && !self.physics
        {
            encoder.copy_buffer_to_buffer(
                resolve,
                0,
                readback,
                self.output_size + self.sample_size,
                timestamp_bytes,
            );
        }
        self.output_size + self.sample_size + timestamp_bytes
    }
    fn map(&mut self, gpu: &Gpu, encoder: wgpu::CommandEncoder, size: u64) {
        self.started = Instant::now();
        self.submission = Some(gpu.queue.submit([encoder.finish()]));
        let (tx, rx) = mpsc::channel();
        self.buffers
            .readback
            .slice(..size)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = tx.send(result);
            });
        self.mapping = Some(rx);
        self.needs_unmap = true;
    }
    fn wait(&mut self, gpu: &Gpu) -> GpuResult<()> {
        gpu.device.poll(wgpu::PollType::Wait {
            submission_index: self.submission.take(),
            timeout: Some(gpu.poll_timeout()),
        })?;
        let result = self
            .mapping
            .take()
            .unwrap()
            .recv_timeout(gpu.poll_timeout())?;
        if result.is_err() {
            self.needs_unmap = false;
        }
        result?;
        Ok(())
    }
    fn dispatch(&mut self, gpu: &Gpu, mut encoder: wgpu::CommandEncoder, combine: Option<u32>) {
        if self.physics {
            encoder.clear_buffer(&self.buffers.terminal, 0, Some(4));
        }
        if let Some(count) = combine {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("basis combination and cell cache"),
                timestamp_writes: self.buffers.timestamps.as_ref().map(|(queries, _)| {
                    wgpu::ComputePassTimestampWrites {
                        query_set: queries,
                        beginning_of_pass_write_index: Some(0),
                        end_of_pass_write_index: Some(1),
                    }
                }),
            });
            pass.set_pipeline(&self.combine_pipeline);
            pass.set_bind_group(0, &self.bindings, &[]);
            let groups = count.div_ceil(64);
            pass.dispatch_workgroups(groups.min(65535), groups.div_ceil(65535), 1);
        }
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("bounded flight"),
                timestamp_writes: self.buffers.timestamps.as_ref().map(|(queries, _)| {
                    wgpu::ComputePassTimestampWrites {
                        query_set: queries,
                        beginning_of_pass_write_index: Some(self.timestamp_count - 2),
                        end_of_pass_write_index: Some(self.timestamp_count - 1),
                    }
                }),
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bindings, &[]);
            pass.dispatch_workgroups(self.groups, 1, 1);
        }
        if let Some((queries, resolve)) = &self.buffers.timestamps {
            encoder.resolve_query_set(queries, 0..self.timestamp_count, resolve, 0);
        }
        let read_size = if self.physics {
            encoder.copy_buffer_to_buffer(&self.buffers.terminal, 0, &self.buffers.readback, 0, 32);
            if let Some((_, resolve)) = &self.buffers.timestamps {
                encoder.copy_buffer_to_buffer(
                    resolve,
                    0,
                    &self.buffers.readback,
                    32,
                    self.timestamp_bytes(),
                );
            }
            32 + self.timestamp_bytes()
        } else {
            self.copy_results(&mut encoder)
        };
        self.map(gpu, encoder, read_size);
    }
    fn collect(
        &mut self,
        gpu: &Gpu,
        field: &InstanceField,
        relativistic: bool,
    ) -> GpuResult<FlightReport> {
        loop {
            self.wait(gpu)?;
            self.timings.fly += self.started.elapsed();
            if !self.physics {
                break;
            }
            let start = Instant::now();
            let size = 32 + self.timestamp_bytes();
            let mapped = self.buffers.readback.slice(..size).get_mapped_range()?;
            let active = bytemuck::cast_slice::<_, u32>(&mapped[..4])[0] != 0;
            if self.buffers.timestamps.is_some() {
                let ticks: &[u64] = bytemuck::cast_slice(&mapped[32..]);
                self.fly_ticks += ticks[(self.timestamp_count - 1) as usize]
                    - ticks[(self.timestamp_count - 2) as usize];
            }
            drop(mapped);
            self.buffers.readback.unmap();
            self.needs_unmap = false;
            self.timings.readback += start.elapsed();
            if !active {
                let mut encoder = gpu.device.create_command_encoder(&Default::default());
                let size = self.copy_results(&mut encoder);
                self.map(gpu, encoder, size);
                self.wait(gpu)?;
                break;
            }
            self.dispatch(
                gpu,
                gpu.device.create_command_encoder(&Default::default()),
                None,
            );
        }
        let start = Instant::now();
        let mut timings = std::mem::take(&mut self.timings);
        let output_size = self.output_size;
        let sample_size = self.sample_size;
        let read_size = output_size
            + sample_size
            + if self.physics {
                0
            } else {
                self.timestamp_bytes()
            };
        let readback = &self.buffers.readback;
        let mapped = readback.slice(..read_size).get_mapped_range()?;
        if self.buffers.timestamps.is_some() {
            let scale = f64::from(gpu.queue.get_timestamp_period()) * 1e-9;
            if self.physics {
                timings.fly = Duration::from_secs_f64(self.fly_ticks as f64 * scale);
            } else {
                let ticks: &[u64] =
                    bytemuck::cast_slice(&mapped[(output_size + sample_size) as usize..]);
                timings.fly = Duration::from_secs_f64(
                    (ticks[(self.timestamp_count - 1) as usize]
                        - ticks[(self.timestamp_count - 2) as usize]) as f64
                        * scale,
                );
                if self.timestamp_count == 4 {
                    timings.combine = Duration::from_secs_f64((ticks[1] - ticks[0]) as f64 * scale);
                }
            }
        }
        let array = field.grid.array();
        let spacing = [array.dx_mm, array.dy_mm, array.dz_mm];
        let sampling = self.sampling.as_ref();
        let output: Vec<EndState> = if gpu.wide {
            bytemuck::cast_slice(&mapped[..output_size as usize]).to_vec()
        } else {
            bytemuck::cast_slice::<_, EndState<f32>>(&mapped[..output_size as usize])
                .iter()
                .map(|i| i.map(f64::from))
                .collect()
        };
        // Failed trials retain the last finite state, and time advances only
        // with an accepted step. A valid reason alone does not prove that the
        // shader completed: driver failures can also leave partial output.
        if output.iter().any(|ion| {
            !(1..=7).contains(&ion.outcome[2])
                || ion
                    .offset
                    .iter()
                    .chain(&ion.velocity)
                    .any(|v| !v.is_finite())
                || ion.offset[3] + ion.velocity[3] < 0.0
                || (ion.outcome[0] == 0 && (ion.offset[3] != 0.0 || ion.velocity[3] != 0.0))
        }) {
            drop(mapped);
            readback.unmap();
            self.needs_unmap = false;
            return Err("GPU flight returned invalid or unfinished terminal states (possible device loss or shader failure); compute timestamps and results are invalid".into());
        }
        let mut results: Vec<_> = output
            .iter()
            .zip(&self.birth_times)
            .map(|(ion, &birth)| {
                let local: [f64; 3] = std::array::from_fn(|a| {
                    (f64::from(ion.cell[a]) + ion.offset[a]) * spacing[a] * field.scale
                });
                let u = ion.velocity;
                let gamma = if relativistic {
                    (1.0 + (u[0] * u[0] + u[1] * u[1] + u[2] * u[2]) / SPEED_OF_LIGHT.powi(2))
                        .sqrt()
                } else {
                    1.0
                };
                let tof = ion.offset[3] + ion.velocity[3];
                FlightResult {
                    final_state: ParticleState {
                        position_mm: std::array::from_fn(|a| {
                            field.origin_mm[a]
                                + (0..3).map(|b| field.axes[b][a] * local[b]).sum::<f64>()
                        }),
                        velocity_mm_per_us: std::array::from_fn(|a| {
                            (0..3).map(|b| field.axes[b][a] * u[b] / gamma).sum()
                        }),
                        time_us: birth + tof,
                    },
                    splat_reason: match ion.outcome[2] {
                        1 if ion.outcome[3] & 0x8000_0000 != 0 => SplatReason::SurfaceElectrode {
                            number: (ion.outcome[3] & 0x7fff_ffff) as usize,
                        },
                        1 => SplatReason::Electrode {
                            index: ion.outcome[3] as usize,
                        },
                        2 => SplatReason::OutsideGrid,
                        3 => SplatReason::StopPlane,
                        4 => SplatReason::TimeLimit,
                        5 => SplatReason::StepLimit,
                        6 => SplatReason::StepUnderflow,
                        _ => SplatReason::NonFinite,
                    },
                    tof_us: tof,
                    accepted_steps: ion.outcome[0] as usize,
                    rejected_steps: ion.outcome[1] as usize,
                    trajectory: None,
                }
            })
            .collect();
        let sample_output: Vec<Sample> = if gpu.wide {
            bytemuck::cast_slice(
                &mapped[output_size as usize..(output_size + sample_size) as usize],
            )
            .to_vec()
        } else {
            bytemuck::cast_slice::<_, Sample<f32>>(
                &mapped[output_size as usize..(output_size + sample_size) as usize],
            )
            .iter()
            .map(|s| s.map(f64::from))
            .collect()
        };
        let mut overflow = false;
        if let Some(s) = sampling {
            for (slot, &index) in s.ion_indices.iter().enumerate() {
                let count = output[index].cell[3] as usize;
                if count > s.max_samples_per_ion {
                    overflow = true;
                    continue;
                }
                let samples = &sample_output
                    [slot * s.max_samples_per_ion..slot * s.max_samples_per_ion + count];
                results[index].trajectory = Some(
                    samples
                        .iter()
                        .map(|s| {
                            let u = s.velocity;
                            let gamma = if relativistic {
                                (1.0 + (u[0] * u[0] + u[1] * u[1] + u[2] * u[2])
                                    / SPEED_OF_LIGHT.powi(2))
                                .sqrt()
                            } else {
                                1.0
                            };
                            ParticleState {
                                position_mm: std::array::from_fn(|a| {
                                    field.origin_mm[a]
                                        + (0..3)
                                            .map(|b| field.axes[b][a] * s.position[b])
                                            .sum::<f64>()
                                }),
                                velocity_mm_per_us: std::array::from_fn(|a| {
                                    (0..3).map(|b| field.axes[b][a] * u[b] / gamma).sum()
                                }),
                                time_us: self.birth_times[index] + s.position[3] + s.velocity[3],
                            }
                        })
                        .collect(),
                );
            }
        }
        drop(mapped);
        readback.unmap();
        self.needs_unmap = false;
        if overflow {
            return Err("GPU trajectory capacity exceeded; increase max_samples_per_ion or sampling interval".into());
        }
        timings.readback += start.elapsed();
        Ok(FlightReport { results, timings })
    }
}

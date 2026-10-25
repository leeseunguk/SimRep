//! Native RK45 flyer: f64 when supported, with the portable f32 fallback.
//! Positions retain integer cells and fractional offsets. The CPU's reconstructed
//! reconstruction is retained; 3D nodal derivatives upload once and expand into
//! a resident cell-power cache when voltages change.

use std::{
    error::Error,
    sync::{Arc, Mutex, mpsc},
    time::{Duration, Instant},
};

use bytemuck::Zeroable;
use rayon::prelude::*;
use simrep_core::Symmetry;
use wgpu::util::DeviceExt;

use crate::{
    FlightOptions, FlightResult, GridField, InstanceField, Integrator, Particle, ParticleState,
    SPEED_OF_LIGHT, SplatReason, VoltageExpression, WorkbenchField, dot,
};

pub use wgpu::{AdapterInfo, Backends, Device, DeviceType, Queue};
pub type GpuResult<T> = Result<T, Box<dyn Error + Send + Sync>>;
const TIMEOUT: Duration = Duration::from_secs(120);
const MAX_BASES: usize = 16;
const MAX_IONS_PER_DISPATCH: usize = 32_768;
const PREFERRED_IONS_PER_DISPATCH: usize = 131_072;
#[cfg(test)]
mod benchmark;
mod coil_map;
mod flight;
mod imported_map;
#[cfg(test)]
mod kernel_benchmark;
use flight::IonBufferPool;
pub use flight::PendingFlight;
mod physics;
#[cfg(test)]
mod physics_benchmark;
#[cfg(test)]
mod physics_tests;
mod precision;
#[cfg(test)]
mod quad_reference;
#[cfg(test)]
mod tests;
use precision::{EndState, Ion, Parameters, Sample, parameters_bytes};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Precision {
    /// Native f64 where enabled on the device, otherwise f32.
    #[default]
    Precise,
    /// Explicit interactive preview; not the native precision acceptance path.
    Preview,
}

#[derive(Debug, Default)]
pub struct Timings {
    pub setup: Duration,
    /// Host allocation/transfer enqueue time; submission does not wait for uploads.
    pub upload: Duration,
    /// GPU basis combination/cache expansion timestamps, when available.
    /// Zero for a clean resident field or a device without timestamp queries.
    pub combine: Duration,
    /// GPU pass timestamps when available; otherwise elapsed submit-to-collection.
    /// Field combination, upload, readback and shader compilation are excluded
    /// from timestamp measurements.
    pub fly: Duration,
    /// Host mapped-result/trajectory decoding and unmapping. Transfers are
    /// queued with compute, so their overlapped wait is not a separate timer.
    pub readback: Duration,
}

pub struct FlightReport {
    pub results: Vec<FlightResult>,
    pub timings: Timings,
}

/// GPU trajectories for selected, strictly increasing, zero-based particle indices.
/// Initial/final states and every `every`th accepted step are retained. Exceeding
/// capacity returns an error rather than silently dropping samples.
pub struct TrajectorySampling<'a> {
    pub ion_indices: &'a [usize],
    pub every: usize,
    pub max_samples_per_ion: usize,
}

struct IonBuffers {
    capacity: usize,
    sample_bytes: u64,
    ions: wgpu::Buffer,
    terminal: wgpu::Buffer,
    samples: wgpu::Buffer,
    readback: wgpu::Buffer,
    uniform: wgpu::Buffer,
    instances: wgpu::Buffer,
    timestamps: Option<(wgpu::QuerySet, wgpu::Buffer)>,
}

pub struct Gpu {
    pub info: AdapterInfo,
    device: wgpu::Device,
    queue: wgpu::Queue,
    layout: wgpu::BindGroupLayout,
    combine: wgpu::ComputePipeline,
    fly: wgpu::ComputePipeline,
    wide: bool,
}

/// The first basis is the instance's grid. Additional bases must have the same
/// geometry and reconstruction. Voltages are multipliers of these arrays:
/// `[1, V1, V2, ...]` represents a fixed background plus unit-voltage bases.
/// Updating them only writes the small uniform, never the basis arrays.
pub struct PreparedField<'a> {
    gpu: &'a Gpu,
    field: &'a InstanceField,
    params: Parameters,
    basis: wgpu::Buffer,
    combined: wgpu::Buffer,
    mask: wgpu::Buffer,
    uniform: wgpu::Buffer,
    coefficient_count: u32,
    dirty: bool,
    expressions: Vec<Option<VoltageExpression>>,
    programs: Vec<Vec<Option<VoltageExpression>>>,
    magnetic_expressions: Option<[String; 3]>,
    integrator: Integrator,
    waveform_pipeline: Option<wgpu::ComputePipeline>,
    instances: wgpu::Buffer,
    instance_count: usize,
    physics_enabled: bool,
    nodal_physics: bool,
    imported_maps: bool,
    collision: Option<&'a crate::physics::collision::CollisionModel>,
    physics_data: wgpu::Buffer,
    ion_buffers: Arc<Mutex<IonBufferPool>>,
    dispatch_capacity: usize,
    pub timings: Timings,
}

impl Gpu {
    /// Defaults to native Vulkan in callers; a software adapter is accepted for
    /// correctness tests and is identified by `info.device_type`.
    pub async fn new(backends: Backends) -> GpuResult<Option<Self>> {
        Self::new_with_precision(backends, Precision::Precise).await
    }

    pub async fn new_with_precision(
        backends: Backends,
        precision: Precision,
    ) -> GpuResult<Option<Self>> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = match instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                ..Default::default()
            })
            .await
        {
            Ok(a) => a,
            Err(wgpu::RequestAdapterError::NotFound { .. }) => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        let info = adapter.get_info();
        let wide = precision == Precision::Precise
            && adapter.features().contains(wgpu::Features::SHADER_F64)
            && !precision::study_is("all");
        let limits = adapter.limits();
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("simrep flyer"),
                required_features: (adapter.features() & wgpu::Features::TIMESTAMP_QUERY)
                    | if wide {
                        wgpu::Features::SHADER_F64
                    } else {
                        wgpu::Features::empty()
                    },
                required_limits: wgpu::Limits {
                    max_storage_buffer_binding_size: limits.max_storage_buffer_binding_size,
                    max_buffer_size: limits.max_buffer_size,
                    max_compute_workgroups_per_dimension: limits
                        .max_compute_workgroups_per_dimension,
                    ..Default::default()
                },
                ..Default::default()
            })
            .await?;
        Self::from_device_with_precision(device, queue, info, precision)
            .await
            .map(Some)
    }

    /// Build pipelines on an existing device. Cloned wgpu handles share the
    /// same device and queue; no adapter or additional device is created.
    pub async fn from_device(device: Device, queue: Queue, info: AdapterInfo) -> GpuResult<Self> {
        Self::from_device_with_precision(device, queue, info, Precision::Precise).await
    }

    /// A preview pipeline may share a device with the native f64 solver.
    pub async fn from_device_with_precision(
        device: Device,
        queue: Queue,
        info: AdapterInfo,
        precision: Precision,
    ) -> GpuResult<Self> {
        let wide = precision == Precision::Precise
            && device.features().contains(wgpu::Features::SHADER_F64)
            && !precision::study_is("all");
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("cell-local RK45"),
            source: wgpu::ShaderSource::Wgsl(
                precision::shader(include_str!("fly.wgsl"), wide).into(),
            ),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("flyer buffers"),
            entries: &(0..9)
                .map(|binding| wgpu::BindGroupLayoutEntry {
                    binding,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: if binding == 0 {
                            wgpu::BufferBindingType::Uniform
                        } else {
                            wgpu::BufferBindingType::Storage {
                                read_only: binding == 1
                                    || binding == 3
                                    || binding == 5
                                    || binding == 8,
                            }
                        },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                })
                .collect::<Vec<_>>(),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("flyer layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = |entry| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: Some(&pipeline_layout),
                module: &shader,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let combine = pipeline("combine");
        let fly = pipeline("fly");
        if let Some(e) = scope.pop().await {
            return Err(e.into());
        }
        Ok(Self {
            info,
            device,
            queue,
            layout,
            combine,
            fly,
            wide,
        })
    }

    pub fn device(&self) -> &Device {
        &self.device
    }

    pub fn queue(&self) -> &Queue {
        &self.queue
    }

    pub fn has_f64(&self) -> bool {
        self.wide
    }

    fn poll_timeout(&self) -> Duration {
        // Software Vulkan also JIT-compiles native physics during submission.
        // Hardware keeps its existing bounded completion wait.
        if self.info.device_type == DeviceType::Cpu {
            Duration::from_secs(600)
        } else {
            TIMEOUT
        }
    }

    pub fn has_timestamps(&self) -> bool {
        self.device
            .features()
            .contains(wgpu::Features::TIMESTAMP_QUERY)
    }

    pub fn prepare<'a>(
        &'a self,
        field: &'a InstanceField,
        additional_bases: &[GridField],
    ) -> GpuResult<PreparedField<'a>> {
        self.prepare_internal(field, additional_bases, true, true)
    }

    fn prepare_internal<'a>(
        &'a self,
        field: &'a InstanceField,
        additional_bases: &[GridField],
        nodal: bool,
        cache: bool,
    ) -> GpuResult<PreparedField<'a>> {
        let start = Instant::now();
        let additional_bases: Vec<_> = additional_bases
            .iter()
            .chain(field.voltage_bases.iter().map(|(g, _)| g))
            .collect();
        if additional_bases.len() >= MAX_BASES {
            return Err("GPU flyer supports at most 16 resident basis arrays".into());
        }
        let array = field.grid.array();
        let nodal = nodal && field.voltage_bases.is_empty();
        let (cells, degrees, mut coefficients, nodal) = if cache {
            field.grid.gpu_data(nodal)
        } else {
            field.grid.gpu_physics_data(nodal)
        };
        let count = coefficients.len();
        if !additional_bases.is_empty() {
            coefficients
                .to_mut()
                .reserve(additional_bases.len() * count);
        }
        for basis in &additional_bases {
            if !field.grid.compatible(basis) {
                return Err("GPU basis geometry differs from the instance grid".into());
            }
            let (c, d, values, n) = basis.gpu_data(nodal);
            if c != cells || d != degrees || n != nodal {
                return Err("GPU basis reconstruction differs from the instance grid".into());
            }
            coefficients.to_mut().extend(values.iter().copied());
        }
        if precision::study_is("coefficients") {
            for c in coefficients.to_mut() {
                *c = f64::from(*c as f32);
            }
        }
        if precision::study_is("trim") {
            for c in coefficients.to_mut() {
                if c.abs() < 1e-10 {
                    *c = 0.0;
                }
            }
        }
        if precision::study_is("profile") {
            let mut maxima = [[0.0_f64; 6]; 3];
            for (i, &c) in coefficients.iter().enumerate() {
                let mut j = i % degrees.iter().product::<usize>();
                for a in 0..3 {
                    maxima[a][j % degrees[a]] = maxima[a][j % degrees[a]].max(c.abs());
                    j /= degrees[a];
                }
            }
            eprintln!(
                "coefficient maxima {:?}: {maxima:?}",
                [array.nx, array.ny, array.nz]
            );
        }
        if coefficients
            .iter()
            .any(|v| !v.is_finite() || !self.wide && !(*v as f32).is_finite())
        {
            return Err("cell powers exceed the GPU scalar range".into());
        }
        self.check_size(coefficients.len() as u64 * if self.wide { 8 } else { 4 })?;
        let mut params = Parameters::zeroed();
        params.padding[0] = array.potentials.len() as u32;
        params.padding[1] = u32::from(nodal);
        params.padding[2] = u32::from(field.grid.uses_node_cell_collisions());
        params.dims = [array.nx as u32, array.ny as u32, array.nz as u32, 0];
        params.cells = [
            cells[0] as u32,
            cells[1] as u32,
            cells[2] as u32,
            count as u32,
        ];
        params.degrees = [
            degrees[0] as u32,
            degrees[1] as u32,
            degrees[2] as u32,
            (additional_bases.len() + 1) as u32,
        ];
        let bits = |b: [bool; 3]| {
            b.into_iter()
                .enumerate()
                .fold(0, |v, (a, x)| v | (u32::from(x) << a))
        };
        params.flags = [
            bits([array.mirrors.x, array.mirrors.y, array.mirrors.z]),
            bits([
                array.anti_mirrors.x,
                array.anti_mirrors.y,
                array.anti_mirrors.z,
            ]),
            u32::from(array.symmetry == Symmetry::Cylindrical),
            u32::from(field.bounds_mm.is_some()),
        ];
        for (a, h) in [array.dx_mm, array.dy_mm, array.dz_mm]
            .into_iter()
            .enumerate()
        {
            params.spacing[a] = h * field.scale;
            for b in 0..3 {
                params.axes[a][b] = field.axes[a][b];
            }
        }
        if let Some(bounds) = field.bounds_mm {
            for (side, point) in bounds.iter().enumerate() {
                for (a, coordinate) in point.iter().enumerate() {
                    params.bounds[side][a] = coordinate - field.origin_mm[a];
                }
            }
        }
        params.spacing[3] = (0..3)
            .filter(|&a| params.dims[a] > 1)
            .map(|a| params.spacing[a])
            .fold(1e30, f64::min);
        if let Some((normal, offset)) = field.plane {
            params.detector = [
                dot(normal, field.axes[0]),
                dot(normal, field.axes[1]),
                dot(normal, field.axes[2]),
                offset - dot(normal, field.origin_mm),
            ];
        }
        params.voltages[0][0] = 1.0;
        if params.spacing[..3]
            .iter()
            .any(|&h| !h.is_finite() || h <= 0.0)
            || params
                .bounds
                .iter()
                .flatten()
                .chain(params.detector.iter())
                .any(|v| !v.is_finite())
        {
            return Err("GPU placement exceeds f32 range".into());
        }
        let setup = start.elapsed();
        let start = Instant::now();
        let buffer = |label, bytes: &[u8], usage| {
            self.device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some(label),
                    contents: bytes,
                    usage,
                })
        };
        let narrow =
            (!self.wide).then(|| coefficients.iter().map(|&v| v as f32).collect::<Vec<_>>());
        let basis = buffer(
            "resident field bases",
            narrow.as_ref().map_or_else(
                || bytemuck::cast_slice(&coefficients),
                |v| bytemuck::cast_slice(v),
            ),
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        );
        let mut mask = field.grid.collision_mask();
        let patch_size = degrees.iter().product::<usize>();
        let mut active_degrees = vec![0u32; cells.iter().product::<usize>()];
        active_degrees
            .par_iter_mut()
            .enumerate()
            .for_each(|(cell, packed)| {
                let origin = [
                    cell % cells[0],
                    cell / cells[0] % cells[1],
                    cell / (cells[0] * cells[1]),
                ];
                let dims = [array.nx, array.ny, array.nz];
                let corners = dims.map(|n| if n > 1 { 2 } else { 1 });
                let solid = (0..corners[2]).all(|z| {
                    (0..corners[1]).all(|y| {
                        (0..corners[0]).all(|x| {
                            mask[((origin[2] + z) * dims[1] + origin[1] + y) * dims[0]
                                + origin[0]
                                + x]
                                != 0
                        })
                    })
                });
                *packed = u32::from(solid) << 12;
                if nodal {
                    *packed |= 0x666;
                    return;
                }
                let mut active = [1usize; 3];
                for b in 0..additional_bases.len() + 1 {
                    for j in 0..patch_size {
                        if coefficients[b * count + cell * patch_size + j] == 0.0 {
                            continue;
                        }
                        let mut k = j;
                        for a in 0..3 {
                            active[a] = active[a].max(k % degrees[a] + 1);
                            k /= degrees[a];
                        }
                    }
                }
                *packed |= (active[0] | (active[1] << 4) | (active[2] << 8)) as u32;
            });
        mask.extend(active_degrees);
        let mask = buffer(
            "electrode mask",
            bytemuck::cast_slice(&mask),
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        );
        let uniform = buffer(
            "flyer parameters and voltages",
            &parameters_bytes(&[params], self.wide),
            wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        );
        let instances = buffer(
            "instance parameters",
            &parameters_bytes(&[params], self.wide),
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        );
        let combined_size = if cache {
            cells.iter().product::<usize>() as u64
                * degrees.iter().product::<usize>() as u64
                * if self.wide { 8 } else { 4 }
        } else {
            32
        };
        self.check_size(combined_size)?;
        let combined = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("adjusted cell powers"),
            size: combined_size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let submission = self.queue.submit([]);
        self.device.poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: Some(self.poll_timeout()),
        })?;
        let expressions: Vec<_> =
            std::iter::repeat_n(None, 1 + additional_bases.len() - field.voltage_bases.len())
                .chain(field.voltage_bases.iter().map(|(_, e)| Some(e.clone())))
                .collect();
        let mut prepared = PreparedField {
            gpu: self,
            field,
            params,
            basis,
            combined,
            mask,
            uniform,
            coefficient_count: count as u32,
            dirty: true,
            expressions: vec![None; expressions.len()],
            programs: Vec::new(),
            magnetic_expressions: None,
            integrator: Integrator::Rk45,
            waveform_pipeline: None,
            instances,
            instance_count: 1,
            physics_enabled: false,
            nodal_physics: false,
            imported_maps: false,
            collision: None,
            physics_data: self
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("empty transport data"),
                    contents: &[0; 32],
                    usage: wgpu::BufferUsages::STORAGE,
                }),
            ion_buffers: Arc::default(),
            dispatch_capacity: self.ion_capacity().min(
                // Strict native-f64 electric flights can exceed Windows TDR
                // at 100k ions. Keep their conservative partition by default.
                if self.wide || self.info.device_type == DeviceType::Cpu {
                    MAX_IONS_PER_DISPATCH
                } else {
                    PREFERRED_IONS_PER_DISPATCH
                },
            ),
            timings: Timings {
                setup,
                upload: start.elapsed(),
                ..Default::default()
            },
        };
        if expressions.iter().any(Option::is_some) {
            prepared.set_voltage_expressions(&expressions)?;
        }
        Ok(prepared)
    }

    /// Resident, non-overlapping workbench instances in one GPU invocation per
    /// ion, including zero-field drift and a shared workbench boundary.
    pub fn prepare_workbench<'a>(
        &'a self,
        field: &'a WorkbenchField,
    ) -> GpuResult<PreparedField<'a>> {
        if field.instances.len() == 1 {
            return self.prepare(&field.instances[0], &[]);
        }
        let fields = field
            .instances
            .iter()
            .map(|f| self.prepare_internal(f, &[], false, false))
            .collect::<GpuResult<Vec<_>>>()?;
        self.join_fields(fields, false)
    }

    fn join_fields<'a>(
        &'a self,
        mut fields: Vec<PreparedField<'a>>,
        physics_enabled: bool,
    ) -> GpuResult<PreparedField<'a>> {
        let start = Instant::now();
        let count = fields.len();
        let size: u64 = fields.iter().map(|f| f.basis.size()).sum();
        self.check_size(size)?;
        let basis = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("workbench basis powers"),
            size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mask_size: u64 = fields.iter().map(|f| f.mask.size()).sum();
        self.check_size(mask_size)?;
        let mask = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("workbench electrode masks"),
            size: mask_size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        let mut offset = 0;
        let mut mask_offset = 0;
        let origin = fields[0].field.origin_mm;
        let mut params = Vec::new();
        let mut programs = Vec::new();
        for (n, f) in fields.iter_mut().enumerate() {
            encoder.copy_buffer_to_buffer(&f.basis, 0, &basis, offset, f.basis.size());
            encoder.copy_buffer_to_buffer(&f.mask, 0, &mask, mask_offset, f.mask.size());
            f.params.dims[3] = (offset / if self.wide { 8 } else { 4 }) as u32;
            f.params.cells[3] = f.coefficient_count;
            f.params.counts[0] = (mask_offset / 4) as u32;
            f.params.axes[0][3] = n as f64;
            f.params.counts[3] = 1;
            for (a, &coordinate) in origin.iter().enumerate() {
                f.params.origin[a] = f.field.origin_mm[a] - coordinate;
            }
            params.push(f.params);
            programs.push(f.expressions.clone());
            offset += f.basis.size();
            mask_offset += f.mask.size();
        }
        self.queue.submit([encoder.finish()]);
        let mut prepared = fields.remove(0);
        prepared.basis = basis;
        prepared.mask = mask;
        prepared.instances = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("placed workbench instances"),
                contents: &parameters_bytes(&params, self.wide),
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_DST
                    | wgpu::BufferUsages::COPY_SRC,
            });
        prepared.instance_count = count;
        prepared.physics_enabled = physics_enabled;
        prepared.params.spacing[3] = params.iter().map(|p| p.spacing[3]).fold(1e30, f64::min);
        prepared.params.dims[3] = if prepared.instance_count > 1 {
            prepared.instance_count as u32
        } else {
            0
        };
        if physics_enabled {
            prepared.programs = programs;
        } else {
            prepared.compile_expressions(&programs)?;
        }
        prepared.timings.setup += start.elapsed();
        Ok(prepared)
    }

    fn check_size(&self, size: u64) -> GpuResult<()> {
        if size > self.device.limits().max_storage_buffer_binding_size
            || size > self.device.limits().max_buffer_size
        {
            return Err(format!(
                "GPU flyer buffer needs {size} bytes; storage binding limit is {}",
                self.device.limits().max_storage_buffer_binding_size
            )
            .into());
        }
        Ok(())
    }

    fn ion_capacity(&self) -> usize {
        ion_capacity(&self.device.limits(), self.wide)
    }
}

fn ion_capacity(limits: &wgpu::Limits, wide: bool) -> usize {
    let binding = limits
        .max_storage_buffer_binding_size
        .min(limits.max_buffer_size);
    let ion_size = if wide { 224 } else { 144 };
    let end_size = if wide { 96 } else { 64 };
    (u64::from(limits.max_compute_workgroups_per_dimension) * 64)
        .min(binding / ion_size)
        .min(binding.saturating_sub(32) / end_size)
        // Readback includes terminal states, timestamps and a minimum sample buffer.
        .min(limits.max_buffer_size.saturating_sub(96) / end_size)
        .min(u64::from(u32::MAX)) as usize
}

impl PreparedField<'_> {
    /// Allocated GPU buffer bytes, including reusable scratch buffers and
    /// outstanding flights from this field. Excludes driver/pipeline/query-set
    /// overhead and CPU memory. Shared device resources are not counted.
    pub fn gpu_memory_bytes(&self) -> u64 {
        [
            &self.basis,
            &self.combined,
            &self.mask,
            &self.uniform,
            &self.instances,
            &self.physics_data,
        ]
        .iter()
        .map(|buffer| buffer.size())
        .sum::<u64>()
            + self.ion_buffers.lock().unwrap().bytes
    }

    /// Effective per-dispatch ion limit, clamped to this device's workgroup and
    /// buffer limits. Physics additionally retains its 4096-ion watchdog bound.
    pub fn max_ions_per_dispatch(&self) -> usize {
        if self.physics_enabled {
            self.dispatch_capacity.min(4096)
        } else {
            self.dispatch_capacity
        }
    }

    /// Select a partition size without changing per-ion arithmetic or RNG.
    /// Native f64 defaults to 32768; hardware Preview defaults to 131072.
    /// Zero is invalid; values above the device limits are clamped. Larger
    /// electric batches can exceed the operating system's watchdog budget.
    pub fn set_max_ions_per_dispatch(&mut self, capacity: usize) -> GpuResult<()> {
        if capacity == 0 {
            return Err("GPU dispatch capacity must be positive".into());
        }
        self.dispatch_capacity = capacity.min(self.gpu.ion_capacity());
        Ok(())
    }

    /// Replace selected resident basis factors by per-ion, per-stage voltages.
    /// `None` retains the factor supplied by `set_voltages`.
    pub fn set_voltage_expressions(
        &mut self,
        expressions: &[Option<VoltageExpression>],
    ) -> GpuResult<()> {
        if expressions.len() != self.params.degrees[3] as usize {
            return Err("provide one optional expression per resident basis".into());
        }
        if self.instance_count != 1 {
            return Err("update workbench voltages before prepare_workbench".into());
        }
        if self.params.padding[1] != 0 && expressions.iter().any(Option::is_some) {
            self.cache_basis_powers()?;
        }
        self.compile_expressions(&[expressions.to_vec()])?;
        self.expressions = expressions.to_vec();
        self.dirty = true;
        Ok(())
    }

    /// Trade compact GPU basis storage for faster repeated voltage updates.
    /// Expansion happens on the GPU without uploading CPU cell powers. This is
    /// also selected automatically for dynamic expressions. Noncompact fields
    /// are already cached, so repeated calls do nothing.
    pub fn cache_basis_powers(&mut self) -> GpuResult<()> {
        if self.params.padding[1] == 0 {
            return Ok(());
        }
        let start = Instant::now();
        let gpu = self.gpu;
        let count = self.params.cells[..3].iter().product::<u32>() * 216;
        let bytes = u64::from(count) * if gpu.wide { 8 } else { 4 };
        let size = bytes * u64::from(self.params.degrees[3]);
        gpu.check_size(size)?;
        let expanded = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("expression basis powers"),
            size,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let dummy = || {
            gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("unused combine binding"),
                size: std::mem::size_of::<Ion>() as u64,
                usage: wgpu::BufferUsages::STORAGE,
                mapped_at_creation: false,
            })
        };
        let (ions, terminal, samples) = (dummy(), dummy(), dummy());
        let bindings = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("materialize compact bases"),
            layout: &gpu.layout,
            entries: &[
                &self.uniform,
                &self.basis,
                &self.combined,
                &self.mask,
                &ions,
                &self.instances,
                &terminal,
                &samples,
                &self.physics_data,
            ]
            .iter()
            .enumerate()
            .map(|(binding, buffer)| wgpu::BindGroupEntry {
                binding: binding as u32,
                resource: buffer.as_entire_binding(),
            })
            .collect::<Vec<_>>(),
        });
        let original = self.params.voltages;
        for b in 0..self.params.degrees[3] as usize {
            self.params.voltages = [[0.0; 4]; MAX_BASES];
            self.params.voltages[b][0] = 1.0;
            gpu.queue.write_buffer(
                &self.uniform,
                0,
                &parameters_bytes(&[self.params], gpu.wide),
            );
            let mut encoder = gpu.device.create_command_encoder(&Default::default());
            {
                let mut pass = encoder.begin_compute_pass(&Default::default());
                pass.set_pipeline(&gpu.combine);
                pass.set_bind_group(0, &bindings, &[]);
                pass.dispatch_workgroups((count / 216).div_ceil(64), 1, 1);
            }
            encoder.copy_buffer_to_buffer(&self.combined, 0, &expanded, b as u64 * bytes, bytes);
            gpu.queue.submit([encoder.finish()]);
        }
        gpu.device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(gpu.poll_timeout()),
        })?;
        self.params.voltages = original;
        self.params.padding[1] = 0;
        self.params.cells[3] = count;
        self.coefficient_count = count;
        self.basis = expanded;
        self.dirty = true;
        self.timings.setup += start.elapsed();
        Ok(())
    }

    fn compile_expressions(
        &mut self,
        programs: &[Vec<Option<VoltageExpression>>],
    ) -> GpuResult<()> {
        let start = Instant::now();
        let dynamic = programs.iter().flatten().any(Option::is_some);
        self.waveform_pipeline = if programs.len() > 1
            || dynamic
            || self.physics_enabled
            || self.integrator == Integrator::Boris
        {
            let mut code = String::from("fn voltage(b: u32, t: f32) -> f32 { switch b {");
            if programs.len() > 1 {
                code =
                    String::from("fn voltage(b: u32, t: f32) -> f32 { switch u32(p.axes[0].w) {");
            }
            for (n, expressions) in programs.iter().enumerate() {
                if programs.len() > 1 {
                    code.push_str(&format!("case {n}u: {{ switch b {{"));
                }
                for (b, expression) in expressions.iter().enumerate() {
                    if let Some(expression) = expression {
                        code.push_str(&format!(
                            "case {b}u: {{ return {}; }}",
                            if self.gpu.wide {
                                expression.to_wgsl_f64()?
                            } else {
                                expression.to_wgsl()?
                            }
                        ));
                    }
                }
                if programs.len() > 1 {
                    code.push_str("default: { return p.voltages[b].x; } } }");
                }
            }
            code.push_str("default: { return p.voltages[b].x; } } }\n");
            let mut edges =
                String::from("fn next_voltage_event(t: f32) -> f32 { var edge = 3.402823e38;");
            for edge in programs
                .iter()
                .flatten()
                .flatten()
                .flat_map(VoltageExpression::events)
            {
                if !(edge as f32).is_finite() {
                    return Err("voltage event exceeds GPU f32 range".into());
                }
                edges.push_str(&format!(
                    "if t < {edge:.17e} {{ edge = min(edge,{edge:.17e}); }}"
                ));
            }
            edges.push_str("return edge; }");
            let mut source = include_str!("fly.wgsl")
                .replace(
                    "const DYNAMIC: bool = false;",
                    if dynamic {
                        "const DYNAMIC: bool = true;"
                    } else {
                        "const DYNAMIC: bool = false;"
                    },
                )
                .replace(
                    "const WORKBENCH: bool = false;",
                    if programs.len() > 1 || self.physics_enabled {
                        "const WORKBENCH: bool = true;"
                    } else {
                        "const WORKBENCH: bool = false;"
                    },
                )
                .replace(
                    "fn voltage(b: u32, t: f32) -> f32 { return p.voltages[b].x; }",
                    &code,
                )
                .replace(
                    "const PHYSICS: bool = false;",
                    if self.physics_enabled {
                        "const PHYSICS: bool = true;"
                    } else {
                        "const PHYSICS: bool = false;"
                    },
                )
                .replace(
                    "const COLLISION: u32 = 0u;",
                    &format!(
                        "const COLLISION: u32 = {}u;",
                        match self.collision {
                            Some(crate::physics::collision::CollisionModel::Hs1(_)) => 1,
                            Some(crate::physics::collision::CollisionModel::Sds(sds))
                                if sds.gas.pressure_pa > 0.0 =>
                                2,
                            Some(crate::physics::collision::CollisionModel::Sds(_)) | None => 0,
                        },
                    ),
                )
                .replace(
                    "const BORIS: bool = false;",
                    &format!(
                        "const BORIS: bool = {};",
                        self.integrator == Integrator::Boris
                    ),
                )
                .replace(
                    "const PHYSICS_NODAL: bool = false;",
                    &format!("const PHYSICS_NODAL: bool = {};", self.nodal_physics),
                )
                .replace(
                    "const IMPORTED_MAPS: bool = false;",
                    &format!("const IMPORTED_MAPS: bool = {};", self.imported_maps),
                )
                .replace(
                    "const MAGNETIC: bool = false;",
                    &format!(
                        "const MAGNETIC: bool = {};",
                        self.params.magnetic[..3].iter().any(|&v| v != 0.0)
                            || self.params.physics[1] != 0
                            || self.params.physics[3] != 0
                            || self.imported_maps
                            || self.magnetic_expressions.is_some(),
                    ),
                )
                .replace(
                    "const ELECTRIC: bool = true;",
                    &format!(
                        "const ELECTRIC: bool = {};",
                        !self.physics_enabled
                            || [
                                self.field.grid.array().nx,
                                self.field.grid.array().ny,
                                self.field.grid.array().nz
                            ] != [1, 1, 1],
                    ),
                )
                .replace(
                    "const COILS: u32 = 0u;",
                    &format!("const COILS: u32 = {}u;", self.params.physics[3],),
                )
                .replace(
                    "const MAGNETIC_PAS: u32 = 0u;",
                    &format!("const MAGNETIC_PAS: u32 = {}u;", self.params.physics[1],),
                )
                .replace(
                    "fn next_voltage_event(t: f32) -> f32 { return 3.402823e38; }",
                    &edges,
                );
            if let Some(expressions) = &self.magnetic_expressions {
                source = source.replace(
                    "fn analytic_magnetic(position: vec3<f32>, time: f32) -> vec3<f32> { return vec3<f32>(0.0); }",
                    &format!(
                        "fn analytic_magnetic(position: vec3<f32>, time: f32) -> vec3<f32> {{ return vec3<f32>({},{},{}); }}",
                        expressions[0], expressions[1], expressions[2],
                    ),
                );
            }
            let scope = self
                .gpu
                .device
                .push_error_scope(wgpu::ErrorFilter::Validation);
            let shader = self
                .gpu
                .device
                .create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some("compiled voltage expressions"),
                    source: wgpu::ShaderSource::Wgsl(
                        precision::shader(&source, self.gpu.wide).into(),
                    ),
                });
            let layout = self
                .gpu
                .device
                .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("voltage expression flyer"),
                    bind_group_layouts: &[Some(&self.gpu.layout)],
                    immediate_size: 0,
                });
            let pipeline =
                self.gpu
                    .device
                    .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                        label: Some("RF flyer"),
                        layout: Some(&layout),
                        module: &shader,
                        entry_point: Some("fly"),
                        compilation_options: Default::default(),
                        cache: None,
                    });
            if let Some(error) = pollster::block_on(scope.pop()) {
                return Err(error.into());
            }
            Some(pipeline)
        } else {
            None
        };
        self.timings.setup += start.elapsed();
        self.programs = programs.to_vec();
        self.dirty = true;
        Ok(())
    }

    pub fn set_voltages(&mut self, voltages: &[f64]) -> GpuResult<()> {
        if self.instance_count != 1 && !self.physics_enabled {
            return Err("update workbench voltages before prepare_workbench".into());
        }
        if voltages.len() != self.params.degrees[3] as usize
            || voltages.iter().any(|&v| !(v as f32).is_finite())
        {
            return Err("provide one finite f32-representable voltage per resident basis".into());
        }
        for (out, &v) in self.params.voltages.iter_mut().zip(voltages) {
            out[0] = v;
        }
        if self.physics_enabled {
            let mut instance = self.params;
            instance.dims[3] = 0;
            instance.origin[..3].fill(0.0);
            instance.counts = [0, 0, 0, 1];
            instance.stop_plane = [0.0; 4];
            self.gpu.queue.write_buffer(
                &self.instances,
                0,
                &parameters_bytes(&[instance], self.gpu.wide),
            );
        }
        self.dirty = true;
        Ok(())
    }
}

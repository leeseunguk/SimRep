//! Opt-in, nonblocking WebGPU compute. GPU arithmetic is always f32.
//!
//! The outer iterate and true residual remain CPU f64, as on native adapters
//! without SHADER_F64. No filesystem, blocking executor, or device wait is used.
use futures_channel::oneshot;
use wgpu::util::DeviceExt;

use crate::gpu::GpuResult;
use crate::mg::{Geometry, Grid, Options};

pub struct Gpu {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub info: wgpu::AdapterInfo,
    pub adapter_limits: wgpu::Limits,
}

pub struct SolveReport {
    pub converged: bool,
    pub cycles: usize,
    pub residual: f64,
    /// GPU pass timestamps, excluding compilation, uploads and readback.
    pub compute_ms: Option<f64>,
}

impl Gpu {
    pub async fn new() -> GpuResult<Option<Self>> {
        Self::with_backends(wgpu::Backends::BROWSER_WEBGPU).await
    }

    /// Vulkan runs the same portable kernels in native correctness tests.
    pub async fn with_backends(backends: wgpu::Backends) -> GpuResult<Option<Self>> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = match instance.request_adapter(&Default::default()).await {
            Ok(adapter) => adapter,
            Err(wgpu::RequestAdapterError::NotFound { .. }) => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let adapter_limits = adapter.limits();
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("SimRep portable f32"),
                required_features: adapter.features() & wgpu::Features::TIMESTAMP_QUERY,
                required_limits: wgpu::Limits {
                    max_storage_buffers_per_shader_stage: adapter_limits
                        .max_storage_buffers_per_shader_stage
                        .min(10),
                    max_storage_buffer_binding_size: adapter_limits.max_storage_buffer_binding_size,
                    max_buffer_size: adapter_limits.max_buffer_size,
                    ..Default::default()
                },
                ..Default::default()
            })
            .await?;
        Ok(Some(Self {
            device,
            queue,
            info: adapter.get_info(),
            adapter_limits,
        }))
    }

    pub fn require_precision(precision: &str) -> GpuResult<()> {
        if precision != "f32" {
            return Err("WebGPU supports only f32 kernels; use CPU wasm for f64".into());
        }
        Ok(())
    }

    pub fn storage<T: bytemuck::Pod>(&self, values: &[T], label: &str) -> GpuResult<wgpu::Buffer> {
        let bytes = bytemuck::cast_slice(values);
        self.check_size(bytes.len() as u64)?;
        Ok(self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents: bytes,
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_SRC
                    | wgpu::BufferUsages::COPY_DST,
            }))
    }

    pub fn check_size(&self, size: u64) -> GpuResult<()> {
        let limits = self.device.limits();
        if size == 0
            || size > limits.max_buffer_size
            || size > limits.max_storage_buffer_binding_size
        {
            return Err(format!("GPU buffer size {size} exceeds adapter storage limits").into());
        }
        Ok(())
    }

    /// Yield to the browser event loop until the mapping callback fires.
    pub async fn read<T: bytemuck::Pod + Copy>(
        &self,
        buffer: &wgpu::Buffer,
        offset: u64,
        count: usize,
    ) -> GpuResult<Vec<T>> {
        let bytes = std::mem::size_of::<T>()
            .checked_mul(count)
            .ok_or("readback size overflow")? as u64;
        if bytes == 0 {
            return Ok(Vec::new());
        }
        if offset
            .checked_add(bytes)
            .is_none_or(|end| end > buffer.size())
        {
            return Err("readback exceeds source buffer".into());
        }
        let staging = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("async readback"),
            size: bytes,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(buffer, offset, &staging, 0, bytes);
        self.queue.submit([encoder.finish()]);
        let (tx, rx) = oneshot::channel();
        staging
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = tx.send(result);
            });
        // Native tests must drive callbacks. This branch is absent in wasm.
        #[cfg(not(target_arch = "wasm32"))]
        self.device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(300)),
        })?;
        rx.await??;
        let mapped = staging.slice(..).get_mapped_range()?;
        let values = mapped
            .chunks_exact(std::mem::size_of::<T>())
            .map(bytemuck::pod_read_unaligned)
            .collect();
        drop(mapped);
        staging.unmap();
        Ok(values)
    }

    /// Cartesian staircase solve with native f32 Galerkin multigrid kernels.
    pub async fn solve(
        &self,
        potential: &mut [f64],
        mask: &[bool],
        grid: Grid,
        options: Options,
    ) -> GpuResult<SolveReport> {
        let count = grid
            .dims
            .iter()
            .try_fold(1usize, |n, d| n.checked_mul(*d))
            .ok_or("grid size overflow")?;
        if grid.geometry != Geometry::Planar
            || grid.dims[0] < 2
            || grid.dims[1] < 2
            || grid.dims[2] == 0
        {
            return Err("WebGPU staircase solve requires a Cartesian grid with nx,ny >= 2".into());
        }
        if potential.len() != count
            || mask.len() != count
            || count > u32::MAX as usize
            || !potential.iter().all(|v| v.is_finite())
            || !mask.iter().any(|v| *v)
            || !grid.spacing.iter().all(|h| {
                h.is_finite() && *h > 0.0 && (1.0 / h.powi(2)).is_finite() && 1.0 / h.powi(2) > 0.0
            })
            || !options.absolute_tolerance.is_finite()
            || options.absolute_tolerance < 0.0
            || !options.relative_tolerance.is_finite()
            || options.relative_tolerance < 0.0
            || options.absolute_tolerance + options.relative_tolerance <= 0.0
            || options.pre_smooth == 0
            || options.post_smooth == 0
        {
            return Err("invalid staircase grid, potentials, mask or solver options".into());
        }
        let origin = potential[mask.iter().position(|v| *v).unwrap()];
        let mut x: Vec<f64> = potential.iter().map(|v| v - origin).collect();
        let normalization = (0..3)
            .filter(|a| grid.dims[*a] > 1)
            .map(|a| 1.0 / grid.spacing[a].powi(2))
            .fold(0.0, f64::max);
        let mut axes = Vec::new();
        let mut offsets = [0u32; 4];
        for (a, offset) in offsets.iter_mut().enumerate().take(3) {
            *offset = axes.len() as u32;
            let n = grid.dims[a];
            let scale = 1.0 / grid.spacing[a].powi(2) / normalization;
            for i in 0..n {
                let (minus, plus, weight) = if n == 1 {
                    (0.0, 0.0, 1.0)
                } else if i == 0 {
                    (0.0, 2.0, 0.5)
                } else if i == n - 1 {
                    (2.0, 0.0, 0.5)
                } else {
                    (1.0, 1.0, 1.0)
                };
                axes.extend([minus * scale, plus * scale, weight]);
            }
        }
        offsets[3] = axes.len() as u32;
        let mut matrix: Vec<f32> = axes.iter().map(|v| *v as f32).collect();
        matrix.extend(mask.iter().map(|v| u8::from(*v) as f32));
        if !matrix.iter().all(|v| v.is_finite()) {
            return Err("stencil exceeds f32 range".into());
        }
        let kernels = Kernels::new(self).await?;
        let mut dims = vec![grid.dims];
        while dims.last().unwrap().iter().any(|n| *n > 4) {
            dims.push(
                dims.last()
                    .unwrap()
                    .map(|n| if n <= 2 { n } else { n / 2 + 1 }),
            );
        }
        let mut levels = Vec::new();
        for (i, d) in dims.iter().enumerate() {
            let child = dims.get(i + 1).unwrap_or(d);
            let words = [
                d[0] as u32,
                d[1] as u32,
                d[2] as u32,
                u32::from(i == 0),
                child[0] as u32,
                child[1] as u32,
                child[2] as u32,
                0,
                offsets[0],
                offsets[1],
                offsets[2],
                offsets[3],
            ];
            let n = d.iter().product::<usize>();
            levels.push(Level {
                dims: *d,
                params: self
                    .device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("level parameters"),
                        contents: bytemuck::cast_slice(&words),
                        usage: wgpu::BufferUsages::UNIFORM,
                    }),
                matrix: if i == 0 {
                    self.storage(&matrix, "fine stencil")?
                } else {
                    self.storage(&vec![0f32; n * 27], "Galerkin stencil")?
                },
                x: [
                    self.storage(&vec![0f32; n], "correction")?,
                    self.storage(&vec![0f32; n], "Jacobi work")?,
                ],
                rhs: self.storage(&vec![0f32; n], "rhs")?,
                current: 0,
            });
        }
        let mut encoder = self.device.create_command_encoder(&Default::default());
        for i in 0..levels.len() - 1 {
            kernels.dispatch(
                self,
                &mut encoder,
                4,
                &levels[i],
                [
                    &kernels.dummy,
                    &kernels.dummy,
                    &levels[i + 1].matrix,
                    &kernels.dummy,
                ],
                dims[i + 1].iter().product(),
            );
        }
        self.queue.submit([encoder.finish()]);
        let visit = |i: usize, value: &mut dyn FnMut(usize, f64)| {
            let c = [
                i % grid.dims[0],
                i / grid.dims[0] % grid.dims[1],
                i / (grid.dims[0] * grid.dims[1]),
            ];
            let stride = [1, grid.dims[0], grid.dims[0] * grid.dims[1]];
            let mut weight = 1.0;
            for a in 0..3 {
                let k = offsets[a] as usize + 3 * c[a];
                if c[a] > 0 {
                    value(i - stride[a], axes[k]);
                }
                if c[a] + 1 < grid.dims[a] {
                    value(i + stride[a], axes[k + 1]);
                }
                weight *= axes[k + 2];
            }
            weight
        };
        let mut scale = 0.0f64;
        for i in 0..count {
            if !mask[i] {
                let mut b = 0.0;
                visit(i, &mut |j, a| {
                    if mask[j] {
                        b += a * x[j];
                    }
                });
                scale = scale.max(b.abs());
            }
        }
        if scale == 0.0 {
            scale = 1.0;
        }
        let target =
            options.absolute_tolerance / normalization + options.relative_tolerance * scale;
        let mut rhs = vec![0f32; count];
        let mut cycles = 0;
        let mut compute_ms = self
            .device
            .features()
            .contains(wgpu::Features::TIMESTAMP_QUERY)
            .then_some(0.0);
        let mut residual;
        loop {
            residual = 0.0f64;
            for i in 0..count {
                rhs[i] = 0.0;
                if !mask[i] {
                    let mut r = 0.0;
                    let w = visit(i, &mut |j, a| r += a * (x[j] - x[i]));
                    rhs[i] = (r * w) as f32;
                    residual = residual.max(r.abs());
                }
            }
            if !residual.is_finite() || !rhs.iter().all(|v| v.is_finite()) {
                return Err("nonfinite f32 correction residual".into());
            }
            if residual <= target || cycles >= options.max_cycles {
                break;
            }
            self.queue
                .write_buffer(&levels[0].rhs, 0, bytemuck::cast_slice(&rhs));
            levels[0].current = 0;
            let mut encoder = self.device.create_command_encoder(&Default::default());
            encoder.clear_buffer(&levels[0].x[0], 0, None);
            let timer = Timer::new(self);
            if let Some(timer) = &timer {
                timer.mark(&mut encoder, true);
            }
            let batch = (options.max_cycles - cycles).min(4);
            for _ in 0..batch {
                cycle(self, &kernels, &mut levels, &mut encoder, options);
            }
            if let Some(timer) = &timer {
                timer.mark(&mut encoder, false);
                timer.resolve(&mut encoder);
            }
            self.queue.submit([encoder.finish()]);
            let correction = self
                .read::<f32>(&levels[0].x[levels[0].current], 0, count)
                .await?;
            if !correction.iter().all(|v| v.is_finite()) {
                return Err("nonfinite GPU correction".into());
            }
            for i in 0..count {
                if !mask[i] {
                    x[i] += f64::from(correction[i]);
                }
            }
            if let Some(timer) = timer {
                *compute_ms.as_mut().unwrap() += timer.milliseconds(self).await?;
            }
            cycles += batch;
        }
        for i in 0..count {
            if !mask[i] {
                potential[i] = x[i] + origin;
            }
        }
        Ok(SolveReport {
            converged: residual <= target,
            cycles,
            residual: residual * normalization,
            compute_ms,
        })
    }
}

/// GPU timestamps only; unsupported adapters report None rather than wall time.
pub struct Timer {
    pub queries: wgpu::QuerySet,
    buffer: wgpu::Buffer,
}
impl Timer {
    pub fn new(gpu: &Gpu) -> Option<Self> {
        gpu.device
            .features()
            .contains(wgpu::Features::TIMESTAMP_QUERY)
            .then(|| Self {
                queries: gpu.device.create_query_set(&wgpu::QuerySetDescriptor {
                    label: Some("compute timestamps"),
                    ty: wgpu::QueryType::Timestamp,
                    count: 2,
                }),
                buffer: gpu.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("timestamp resolve"),
                    size: 16,
                    usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                    mapped_at_creation: false,
                }),
            })
    }
    fn mark(&self, encoder: &mut wgpu::CommandEncoder, beginning: bool) {
        let _pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("compute interval boundary"),
            timestamp_writes: Some(wgpu::ComputePassTimestampWrites {
                query_set: &self.queries,
                beginning_of_pass_write_index: beginning.then_some(0),
                end_of_pass_write_index: (!beginning).then_some(1),
            }),
        });
    }
    pub fn resolve(&self, encoder: &mut wgpu::CommandEncoder) {
        encoder.resolve_query_set(&self.queries, 0..2, &self.buffer, 0);
    }
    pub async fn milliseconds(&self, gpu: &Gpu) -> GpuResult<f64> {
        let ticks = gpu.read::<u64>(&self.buffer, 0, 2).await?;
        Ok((ticks[1] - ticks[0]) as f64 * f64::from(gpu.queue.get_timestamp_period()) * 1e-6)
    }
}
struct Level {
    dims: [usize; 3],
    params: wgpu::Buffer,
    matrix: wgpu::Buffer,
    x: [wgpu::Buffer; 2],
    rhs: wgpu::Buffer,
    current: usize,
}
struct Kernels {
    layout: wgpu::BindGroupLayout,
    pipelines: Vec<wgpu::ComputePipeline>,
    dummy: wgpu::Buffer,
}
impl Kernels {
    async fn new(gpu: &Gpu) -> GpuResult<Self> {
        let scope = gpu.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let layout = gpu
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("portable multigrid"),
                entries: &(0..6)
                    .map(|binding| wgpu::BindGroupLayoutEntry {
                        binding,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: if binding == 0 {
                                wgpu::BufferBindingType::Uniform
                            } else {
                                wgpu::BufferBindingType::Storage {
                                    read_only: binding != 4,
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
        let mut source = include_str!("gpu/multigrid.wgsl")
            .replace("ASSEMBLY_INIT", "var row: array<f32,27>;")
            .replace("ASSEMBLY_ADD", "row[slot] += left*a*right;")
            .replace(
                "ASSEMBLY_STORE",
                "for(var s=0u;s<27u;s++) { out[s*count(p.coarse.xyz)+i]=row[s]; }",
            );
        // Browser WGSL rejects constant infinity even in unused entry points.
        // This API computes true residuals on the CPU, so omit native norm code.
        let start = source
            .find("// Physical (unweighted) correction residual;")
            .unwrap();
        let end = source
            .find("@compute @workgroup_size(128)\nfn jacobi_sweep")
            .unwrap();
        source.replace_range(start..end, "");
        let shader = gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("portable multigrid"),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            });
        let pipelines = [
            "jacobi_sweep",
            "residual",
            "restrict_rhs",
            "prolong",
            "galerkin",
        ]
        .iter()
        .map(|name| {
            gpu.device
                .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some(name),
                    layout: Some(&pipeline_layout),
                    module: &shader,
                    entry_point: Some(name),
                    compilation_options: Default::default(),
                    cache: None,
                })
        })
        .collect();
        if let Some(error) = scope.pop().await {
            return Err(error.into());
        }
        Ok(Self {
            layout,
            pipelines,
            dummy: gpu.storage(&[0u32], "dummy")?,
        })
    }
    fn dispatch(
        &self,
        gpu: &Gpu,
        encoder: &mut wgpu::CommandEncoder,
        kernel: usize,
        level: &Level,
        buffers: [&wgpu::Buffer; 4],
        count: usize,
    ) {
        let bindings = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.layout,
            entries: [&level.params, &level.matrix]
                .into_iter()
                .chain(buffers)
                .enumerate()
                .map(|(binding, buffer)| wgpu::BindGroupEntry {
                    binding: binding as u32,
                    resource: buffer.as_entire_binding(),
                })
                .collect::<Vec<_>>()
                .as_slice(),
        });
        let groups = count.div_ceil(128);
        let y = groups.div_ceil(65535);
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&self.pipelines[kernel]);
        pass.set_bind_group(0, &bindings, &[]);
        pass.dispatch_workgroups(groups.div_ceil(y) as u32, y as u32, 1);
    }
}
fn smooth(
    gpu: &Gpu,
    k: &Kernels,
    level: &mut Level,
    encoder: &mut wgpu::CommandEncoder,
    sweeps: usize,
) {
    for _ in 0..sweeps {
        k.dispatch(
            gpu,
            encoder,
            0,
            level,
            [
                &level.x[level.current],
                &level.rhs,
                &level.x[1 - level.current],
                &k.dummy,
            ],
            level.dims.iter().product(),
        );
        level.current = 1 - level.current;
    }
}
fn cycle(
    gpu: &Gpu,
    k: &Kernels,
    levels: &mut [Level],
    encoder: &mut wgpu::CommandEncoder,
    options: Options,
) {
    let (level, tail) = levels.split_first_mut().unwrap();
    if tail.is_empty() {
        smooth(gpu, k, level, encoder, 80);
        return;
    }
    smooth(gpu, k, level, encoder, options.pre_smooth);
    k.dispatch(
        gpu,
        encoder,
        1,
        level,
        [
            &level.x[level.current],
            &level.rhs,
            &level.x[1 - level.current],
            &k.dummy,
        ],
        level.dims.iter().product(),
    );
    k.dispatch(
        gpu,
        encoder,
        2,
        level,
        [
            &level.x[1 - level.current],
            &k.dummy,
            &tail[0].rhs,
            &k.dummy,
        ],
        tail[0].dims.iter().product(),
    );
    tail[0].current = 0;
    encoder.clear_buffer(&tail[0].x[0], 0, None);
    cycle(gpu, k, tail, encoder, options);
    k.dispatch(
        gpu,
        encoder,
        3,
        level,
        [
            &level.x[level.current],
            &k.dummy,
            &level.x[1 - level.current],
            &tail[0].x[tail[0].current],
        ],
        level.dims.iter().product(),
    );
    level.current = 1 - level.current;
    smooth(gpu, k, level, encoder, options.post_smooth);
}

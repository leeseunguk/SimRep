//! Native GPU foundation and a checked 64^3 residual kernel.
//!
//! The portable f32 kernel is WGSL. The optional f64 kernel is native GLSL
//! translated by Naga: WGSL itself has no f64 type.

use std::{borrow::Cow, error::Error, sync::mpsc, time::Duration};

use wgpu::util::DeviceExt;

mod solve;
pub use solve::{Poisson, RefinementFailure, SolveReport, Timings};

pub use wgpu::{AdapterInfo, Backends, Device, DeviceType, Features, Queue};

pub type GpuResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

pub const GRID_SIZE: usize = 64;
const CELL_COUNT: usize = GRID_SIZE * GRID_SIZE * GRID_SIZE;
const GPU_TIMEOUT: Duration = Duration::from_secs(30);

/// A compute device selected with a high-performance adapter preference.
pub struct Gpu {
    pub info: AdapterInfo,
    pub adapter_features: Features,
    device: wgpu::Device,
    queue: wgpu::Queue,
}

#[derive(Debug)]
pub struct KernelResult {
    pub precision: &'static str,
    /// GPU timestamp duration, excluding uploads, compilation and readback.
    /// None means the adapter does not expose TIMESTAMP_QUERY.
    pub kernel_time: Option<Duration>,
    /// Host time from queue submission through completed readback.
    pub submit_readback_time: Duration,
    pub max_abs_error: f64,
    pub tolerance: f64,
    pub cells_checked: usize,
}

impl Gpu {
    /// Returns None only when no adapter is available for the requested backends.
    /// Device creation errors are propagated rather than treated as a skip.
    pub async fn new(backends: Backends) -> GpuResult<Option<Self>> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = match instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: false,
                compatible_surface: None,
                ..Default::default()
            })
            .await
        {
            Ok(adapter) => adapter,
            Err(wgpu::RequestAdapterError::NotFound { .. }) => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let info = adapter.get_info();
        let adapter_features = adapter.features();
        let required_features =
            adapter_features & (Features::SHADER_F64 | Features::TIMESTAMP_QUERY);
        let limits = adapter.limits();
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("simrep GPU"),
                required_features,
                required_limits: wgpu::Limits {
                    max_buffer_size: limits.max_buffer_size,
                    max_storage_buffer_binding_size: limits.max_storage_buffer_binding_size,
                    ..Default::default()
                },
                ..Default::default()
            })
            .await?;
        let mut gpu = Self::from_device(device, queue, info);
        gpu.adapter_features = adapter_features;
        Ok(Some(gpu))
    }

    /// Reuse an application's compute device, including its enabled features.
    pub fn from_device(device: Device, queue: Queue, info: AdapterInfo) -> Self {
        Self {
            info,
            adapter_features: device.features(),
            device,
            queue,
        }
    }

    pub fn device(&self) -> &Device {
        &self.device
    }

    pub fn queue(&self) -> &Queue {
        &self.queue
    }

    pub fn has_f64(&self) -> bool {
        self.adapter_features.contains(Features::SHADER_F64)
    }

    /// Runs f32, then f64 when supported. Every result must be finite and within
    /// its absolute tolerance; a mismatch returns an error to the caller.
    pub fn smoke(&self) -> GpuResult<Vec<KernelResult>> {
        let (potential, rhs) = smoke_input();
        // Compare both precisions on the exact same f32-representable inputs,
        // isolating arithmetic error from input quantization.
        let potential64: Vec<_> = potential.iter().map(|&v| f64::from(v)).collect();
        let rhs64: Vec<_> = rhs.iter().map(|&v| f64::from(v)).collect();
        let reference = residual_cpu(&potential64, &rhs64, GRID_SIZE);
        let mut results = vec![self.run_kernel(
            "f32",
            wgpu::ShaderSource::Wgsl(include_str!("residual.wgsl").into()),
            &potential,
            &rhs,
            &reference,
            2.0e-6,
        )?];
        if self.has_f64() {
            results.push(self.run_kernel(
                "f64",
                wgpu::ShaderSource::Glsl {
                    shader: Cow::Borrowed(include_str!("residual_f64.comp")),
                    stage: wgpu::naga::ShaderStage::Compute,
                    defines: Default::default(),
                },
                &potential64,
                &rhs64,
                &reference,
                1.0e-12,
            )?);
        }
        Ok(results)
    }

    fn run_kernel<T: bytemuck::Pod + Copy + Into<f64>>(
        &self,
        precision: &'static str,
        source: wgpu::ShaderSource<'_>,
        potential: &[T],
        rhs: &[T],
        reference: &[f64],
        tolerance: f64,
    ) -> GpuResult<KernelResult> {
        let scope = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let shader = self
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("7-point residual"),
                source,
            });
        let pipeline = self
            .device
            .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("7-point residual"),
                layout: None,
                module: &shader,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                cache: None,
            });
        if let Some(error) = pollster::block_on(scope.pop()) {
            return Err(error.into());
        }
        let input = |label, values: &[T]| {
            self.device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some(label),
                    contents: bytemuck::cast_slice(values),
                    usage: wgpu::BufferUsages::STORAGE,
                })
        };
        let potential_buffer = input("potential", potential);
        let rhs_buffer = input("rhs", rhs);
        let byte_size = std::mem::size_of_val(potential) as u64;
        let output = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("residual"),
            size: byte_size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback = self.readback_buffer(byte_size, "residual readback");
        let bindings = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("residual inputs and output"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: potential_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: rhs_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: output.as_entire_binding(),
                },
            ],
        });
        let timestamps = self
            .device
            .features()
            .contains(Features::TIMESTAMP_QUERY)
            .then(|| {
                let queries = self.device.create_query_set(&wgpu::QuerySetDescriptor {
                    label: Some("residual timestamps"),
                    ty: wgpu::QueryType::Timestamp,
                    count: 2,
                });
                let resolve = self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("timestamp resolve"),
                    size: 16,
                    usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                    mapped_at_creation: false,
                });
                (
                    queries,
                    resolve,
                    self.readback_buffer(16, "timestamp readback"),
                )
            });
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("residual smoke"),
            });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("7-point residual"),
                timestamp_writes: timestamps.as_ref().map(|(queries, _, _)| {
                    wgpu::ComputePassTimestampWrites {
                        query_set: queries,
                        beginning_of_pass_write_index: Some(0),
                        end_of_pass_write_index: Some(1),
                    }
                }),
            });
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &bindings, &[]);
            pass.dispatch_workgroups(16, 16, 16);
        }
        encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, byte_size);
        if let Some((queries, resolve, readback)) = &timestamps {
            encoder.resolve_query_set(queries, 0..2, resolve, 0);
            encoder.copy_buffer_to_buffer(resolve, 0, readback, 0, 16);
        }
        let started = std::time::Instant::now();
        let submission = self.queue.submit([encoder.finish()]);
        let (sender, receiver) = mpsc::channel();
        readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                // The receiver is alive until after the bounded device poll below.
                let _ = sender.send(result);
            });
        let timestamp_receiver = timestamps.as_ref().map(|(_, _, buffer)| {
            let (sender, receiver) = mpsc::channel();
            buffer
                .slice(..)
                .map_async(wgpu::MapMode::Read, move |result| {
                    let _ = sender.send(result);
                });
            receiver
        });
        self.device.poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: Some(GPU_TIMEOUT),
        })?;
        receiver.recv_timeout(GPU_TIMEOUT)??;
        let submit_readback_time = started.elapsed();
        let data = readback.slice(..).get_mapped_range()?;
        let values: &[T] = bytemuck::cast_slice(&data);
        let mut max_abs_error = 0.0_f64;
        for (&actual, &expected) in values.iter().zip(reference) {
            let actual = actual.into();
            if !actual.is_finite() {
                return Err(
                    format!("{precision} kernel returned non-finite value: {actual}").into(),
                );
            }
            max_abs_error = max_abs_error.max((actual - expected).abs());
        }
        drop(data);
        readback.unmap();
        let kernel_time = if let Some((_, _, buffer)) = &timestamps {
            timestamp_receiver.unwrap().recv_timeout(GPU_TIMEOUT)??;
            let data = buffer.slice(..).get_mapped_range()?;
            let ticks: &[u64] = bytemuck::cast_slice(&data);
            let seconds = (ticks[1] - ticks[0]) as f64
                * f64::from(self.queue.get_timestamp_period())
                * 1.0e-9;
            drop(data);
            buffer.unmap();
            Some(Duration::from_secs_f64(seconds))
        } else {
            None
        };
        if max_abs_error > tolerance {
            return Err(format!(
                "{precision} residual failed: max abs error {max_abs_error:e} > {tolerance:e}"
            )
            .into());
        }
        Ok(KernelResult {
            precision,
            kernel_time,
            submit_readback_time,
            max_abs_error,
            tolerance,
            cells_checked: reference.len(),
        })
    }

    fn readback_buffer(&self, size: u64, label: &str) -> wgpu::Buffer {
        self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        })
    }
}

fn smoke_input() -> (Vec<f32>, Vec<f32>) {
    let mut state = 0x1234_5678_u32;
    let mut sample = || {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (state >> 8) as f32 / 16_777_216.0 - 0.5
    };
    let potential = (0..CELL_COUNT).map(|_| sample()).collect();
    let rhs = (0..CELL_COUNT).map(|_| sample()).collect();
    (potential, rhs)
}

fn residual_cpu(potential: &[f64], rhs: &[f64], n: usize) -> Vec<f64> {
    let mut residual = vec![0.0; n * n * n];
    for z in 1..n - 1 {
        for y in 1..n - 1 {
            for x in 1..n - 1 {
                let i = x + n * (y + n * z);
                let laplacian = potential[i - 1]
                    + potential[i + 1]
                    + potential[i - n]
                    + potential[i + n]
                    + potential[i - n * n]
                    + potential[i + n * n]
                    - 6.0 * potential[i];
                residual[i] = rhs[i] - laplacian;
            }
        }
    }
    residual
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quadratic_has_known_laplacian_and_zero_boundary_residual() {
        let n = 5;
        let potential: Vec<_> = (0..n * n * n)
            .map(|i| {
                let (x, y, z) = (i % n, i / n % n, i / (n * n));
                (x * x + y * y + z * z) as f64
            })
            .collect();
        let rhs = vec![2.0; potential.len()];
        let result = residual_cpu(&potential, &rhs, n);
        for z in 0..n {
            for y in 0..n {
                for x in 0..n {
                    let boundary = [x, y, z].iter().any(|&p| p == 0 || p == n - 1);
                    assert_eq!(
                        result[x + n * (y + n * z)],
                        if boundary { 0.0 } else { -4.0 }
                    );
                }
            }
        }
    }

    #[test]
    fn shader_sources_validate_without_a_gpu() {
        use wgpu::naga::{ShaderStage, front, valid};

        let wgsl = front::wgsl::parse_str(include_str!("residual.wgsl")).unwrap();
        let glsl = front::glsl::Frontend::default()
            .parse(
                &front::glsl::Options::from(ShaderStage::Compute),
                include_str!("residual_f64.comp"),
            )
            .unwrap();
        for module in [wgsl, glsl] {
            valid::Validator::new(valid::ValidationFlags::all(), valid::Capabilities::all())
                .validate(&module)
                .unwrap();
        }
    }
}

//! Standalone GPU sampled-SDF intersections; no flight-kernel integration.
//!
//! The host partitions by M = sum(|delta_a|/h_a) on linear active axes,
//! plus hypot(delta_y, delta_z)/h_r for cylindrical volumes, using the
//! tightest per-axis spacing across volumes. Each submitted piece has M <= 4
//! (we target 2 to leave rounding headroom). A linear axis contributes at
//! most 2(M_a+1) grid faces plus one mirror fold. A radial branch spans at
//! most M_r+1 radii, with two mirror branches and at most three roots/critical
//! points per circle. Including the radial mirror circle gives <= 6M+14
//! cuts in the cylindrical case, and <= 2M+11 in Cartesian geometry,
//! endpoints included. Thus 64 geometric slots suffice for M <= 4. Per-cell
//! quartic roots are streamed, requiring only 16 further slots, independent
//! of dims.
//!
//! `Precise` uses SHADER_F64 when available, otherwise the mixed f32 path,
//! as the existing flyer does. `Preview` always uses f32 storage/arithmetic.
use std::{sync::mpsc, time::Duration};

use wgpu::util::DeviceExt;

use crate::{
    FlyError, Vector,
    gpu::{Backends, Gpu, Precision},
    surface_sdf::SdfVolumes,
};

const BATCH: usize = 32_768;
const TIMEOUT: Duration = Duration::from_secs(600);
type Hits = Vec<Option<(f64, usize)>>;

/// Upload once and reuse the standalone pipeline for multiple segment batches.
pub struct GpuSdf {
    gpu: Gpu,
    pipeline: wgpu::ComputePipeline,
    headers: wgpu::Buffer,
    samples: wgpu::Buffer,
    linear_scale: Vector,
    radial_scale: f64,
    empty: bool,
}

/// Return the first sampled-SDF hit (including a start inside a volume).
pub fn gpu_first_hits(
    sdf: &SdfVolumes,
    segments: &[(Vector, Vector)],
    precision: Precision,
) -> Result<Hits, FlyError> {
    if segments.is_empty() || sdf.volumes.is_empty() {
        return Ok(vec![None; segments.len()]);
    }
    GpuSdf::new(sdf, precision)?.first_hits(segments)
}

impl GpuSdf {
    pub fn new(sdf: &SdfVolumes, precision: Precision) -> Result<Self, FlyError> {
        let gpu = pollster::block_on(Gpu::new_with_precision(Backends::VULKAN, precision))
            .map_err(|e| gpu_error("GPU SDF device creation failed", e))?
            .ok_or(FlyError("GPU SDF requires a Vulkan adapter"))?;
        let wide = gpu.has_f64();
        let scalar_bytes = if wide { 8 } else { 4 };
        let size = sdf
            .volumes
            .iter()
            .map(|v| v.samples_f64.len() as u64)
            .sum::<u64>()
            * scalar_bytes;
        let header_size = sdf.volumes.len() as u64 * if wide { 128 } else { 80 };
        check_storage(size.max(header_size), &gpu.device().limits())?;
        let (cylindrical, mirrors, tolerance) = sdf.gpu_geometry();
        let flags = u32::from(cylindrical)
            | mirrors
                .into_iter()
                .enumerate()
                .fold(0, |bits, (a, m)| bits | (u32::from(m) << (a + 1)));
        let mut headers = Vec::new();
        let mut samples = Vec::new();
        let mut offset = 0u32;
        let mut linear_scale = [0.0_f64; 3];
        let mut radial_scale = 0.0_f64;
        for v in &sdf.volumes {
            for value in v.dims.map(|n| n as u32).into_iter().chain([0]) {
                headers.extend(value.to_le_bytes());
            }
            for value in [v.number as u32, offset, flags, 0] {
                headers.extend(value.to_le_bytes());
            }
            for value in v
                .origin
                .into_iter()
                .chain([0.0])
                .chain(v.spacing)
                .chain([0.0])
                .chain([v.lipschitz, tolerance, 0.0, 0.0])
            {
                scalar(&mut headers, value, wide)?;
            }
            if wide {
                samples.extend_from_slice(bytemuck::cast_slice(&v.samples_f64));
            } else {
                samples.extend_from_slice(bytemuck::cast_slice(&v.samples_f32));
            }
            offset += v.samples_f64.len() as u32;
            for (a, scale) in linear_scale.iter_mut().enumerate() {
                if v.dims[a] > 1 && (!cylindrical || a == 0) {
                    *scale = scale.max(1.0 / v.spacing[a]);
                }
            }
            if cylindrical && v.dims[1] > 1 {
                radial_scale = radial_scale.max(1.0 / v.spacing[1]);
            }
        }
        // Wgpu bindings cannot be zero bytes. Empty input never dispatches.
        if headers.is_empty() {
            headers.resize(if wide { 128 } else { 80 }, 0);
        }
        if samples.is_empty() {
            samples.resize(scalar_bytes as usize, 0);
        }
        let buffer = |label, contents: &[u8]| {
            gpu.device()
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some(label),
                    contents,
                    usage: wgpu::BufferUsages::STORAGE,
                })
        };
        let headers = buffer("SDF volume headers", &headers);
        let samples = buffer("SDF samples", &samples);
        let scope = gpu.device().push_error_scope(wgpu::ErrorFilter::Validation);
        let shader = gpu
            .device()
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("standalone SDF segment hits"),
                source: wgpu::ShaderSource::Wgsl(shader_source(wide).into()),
            });
        let pipeline = gpu
            .device()
            .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("SDF first hits"),
                layout: None,
                module: &shader,
                entry_point: Some("first_hits"),
                compilation_options: Default::default(),
                cache: None,
            });
        if let Some(e) = pollster::block_on(scope.pop()) {
            return Err(gpu_error("GPU SDF shader validation failed", e));
        }
        Ok(Self {
            gpu,
            pipeline,
            headers,
            samples,
            linear_scale,
            radial_scale,
            empty: sdf.volumes.is_empty(),
        })
    }

    pub fn has_f64(&self) -> bool {
        self.gpu.has_f64()
    }

    pub fn adapter_info(&self) -> &wgpu::AdapterInfo {
        &self.gpu.info
    }

    fn metric(&self, delta: Vector) -> f64 {
        (0..3)
            .map(|a| delta[a].abs() * self.linear_scale[a])
            .sum::<f64>()
            + delta[1].hypot(delta[2]) * self.radial_scale
    }

    pub fn first_hits(&self, segments: &[(Vector, Vector)]) -> Result<Hits, FlyError> {
        self.first_hits_profiled(segments).map(|(hits, _)| hits)
    }

    /// GPU timestamps cover compute passes only, excluding upload, readback,
    /// host splitting/reduction and pipeline creation. None means this adapter
    /// lacks timestamp queries; wall time must not be called compute time.
    pub fn first_hits_profiled(
        &self,
        segments: &[(Vector, Vector)],
    ) -> Result<(Hits, Option<Duration>), FlyError> {
        let mut results = vec![None; segments.len()];
        let mut compute = self.gpu.has_timestamps().then_some(Duration::ZERO);
        if self.empty {
            return Ok((results, compute));
        }
        let wide = self.has_f64();
        let mut bytes = Vec::new();
        let mut owners = Vec::new();
        for (index, &(from, to)) in segments.iter().enumerate() {
            if from.into_iter().chain(to).any(|x| !x.is_finite()) {
                return Err(FlyError("GPU SDF segments must be finite"));
            }
            let delta = std::array::from_fn(|a| to[a] - from[a]);
            let pieces = (self.metric(delta) / 2.0).ceil().max(1.0) as usize;
            for piece in 0..pieces {
                let t0 = piece as f64 / pieces as f64;
                let t1 = (piece + 1) as f64 / pieces as f64;
                let a: Vector = std::array::from_fn(|axis| from[axis] + t0 * delta[axis]);
                let b: Vector = std::array::from_fn(|axis| from[axis] + t1 * delta[axis]);
                let actual_delta = std::array::from_fn(|axis| {
                    if wide {
                        b[axis] - a[axis]
                    } else {
                        f64::from(b[axis] as f32) - f64::from(a[axis] as f32)
                    }
                });
                if self.metric(actual_delta) > 4.0 {
                    return Err(FlyError(
                        "GPU SDF segment precision cannot resolve the cell spacing",
                    ));
                }
                for value in a.into_iter().chain([0.0]).chain(b).chain([0.0]) {
                    scalar(&mut bytes, value, wide)?;
                }
                owners.push((index, t0, t1 - t0));
                if owners.len() == BATCH {
                    let elapsed = self.collect(&bytes, &owners, &mut results)?;
                    if let (Some(sum), Some(elapsed)) = (&mut compute, elapsed) {
                        *sum += elapsed;
                    }
                    bytes.clear();
                    owners.clear();
                }
            }
        }
        if !owners.is_empty() {
            let elapsed = self.collect(&bytes, &owners, &mut results)?;
            if let (Some(sum), Some(elapsed)) = (&mut compute, elapsed) {
                *sum += elapsed;
            }
        }
        Ok((results, compute))
    }

    fn collect(
        &self,
        bytes: &[u8],
        owners: &[(usize, f64, f64)],
        results: &mut [Option<(f64, usize)>],
    ) -> Result<Option<Duration>, FlyError> {
        let device = self.gpu.device();
        let size = owners.len() as u64 * if self.has_f64() { 32 } else { 16 };
        check_storage((bytes.len() as u64).max(size), &device.limits())?;
        let input = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("SDF segments"),
            contents: bytes,
            usage: wgpu::BufferUsages::STORAGE,
        });
        let output = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("SDF first hits"),
            size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let timestamps = self.gpu.has_timestamps().then(|| {
            let query = device.create_query_set(&wgpu::QuerySetDescriptor {
                label: Some("SDF compute timestamps"),
                ty: wgpu::QueryType::Timestamp,
                count: 2,
            });
            let resolve = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("SDF timestamp resolve"),
                size: 16,
                usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            });
            (query, resolve)
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("SDF hit readback"),
            size: size + if timestamps.is_some() { 16 } else { 0 },
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bindings = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("SDF bindings"),
            layout: &self.pipeline.get_bind_group_layout(0),
            entries: &[&self.headers, &self.samples, &input, &output]
                .into_iter()
                .enumerate()
                .map(|(binding, buffer)| wgpu::BindGroupEntry {
                    binding: binding as u32,
                    resource: buffer.as_entire_binding(),
                })
                .collect::<Vec<_>>(),
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("SDF first hits"),
                timestamp_writes: timestamps.as_ref().map(|(query, _)| {
                    wgpu::ComputePassTimestampWrites {
                        query_set: query,
                        beginning_of_pass_write_index: Some(0),
                        end_of_pass_write_index: Some(1),
                    }
                }),
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bindings, &[]);
            pass.dispatch_workgroups((owners.len() as u32).div_ceil(64), 1, 1);
        }
        encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, size);
        if let Some((query, resolve)) = &timestamps {
            encoder.resolve_query_set(query, 0..2, resolve, 0);
            encoder.copy_buffer_to_buffer(resolve, 0, &readback, size, 16);
        }
        self.gpu.queue().submit([encoder.finish()]);
        let (tx, rx) = mpsc::channel();
        readback.map_async(wgpu::MapMode::Read, .., move |r| {
            let _ = tx.send(r);
        });
        device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(TIMEOUT),
            })
            .map_err(|e| gpu_error("GPU SDF execution failed", e))?;
        rx.recv_timeout(TIMEOUT)
            .map_err(|_| FlyError("GPU SDF readback timed out"))?
            .map_err(|e| gpu_error("GPU SDF readback failed", e))?;
        let mapped = readback
            .get_mapped_range(..)
            .map_err(|e| gpu_error("GPU SDF mapping failed", e))?;
        let values = &mapped[..size as usize];
        for (i, &(owner, t0, dt)) in owners.iter().enumerate() {
            let value: [f64; 4] = if self.has_f64() {
                bytemuck::cast_slice::<u8, [f64; 4]>(values)[i]
            } else {
                bytemuck::cast_slice::<u8, [f32; 4]>(values)[i].map(f64::from)
            };
            if value[2] == 2.0 {
                return Err(FlyError("GPU SDF geometric breakpoint bound exceeded"));
            }
            if value[2] == 1.0 {
                let t = t0 + dt * value[0];
                if results[owner].is_none_or(|hit| t < hit.0) {
                    results[owner] = Some((t, value[1] as usize));
                }
            }
        }
        let compute = timestamps.as_ref().map(|_| {
            let ticks: &[u64] = bytemuck::cast_slice(&mapped[size as usize..]);
            Duration::from_secs_f64(
                ticks[1].wrapping_sub(ticks[0]) as f64
                    * f64::from(self.gpu.queue().get_timestamp_period())
                    * 1e-9,
            )
        });
        drop(mapped);
        readback.unmap();
        Ok(compute)
    }
}

fn scalar(bytes: &mut Vec<u8>, value: f64, wide: bool) -> Result<(), FlyError> {
    if !value.is_finite() || !wide && !(value as f32).is_finite() {
        return Err(FlyError("SDF values exceed the GPU scalar range"));
    }
    if wide {
        bytes.extend(value.to_le_bytes());
    } else {
        bytes.extend((value as f32).to_le_bytes());
    }
    Ok(())
}

fn check_storage(size: u64, limits: &wgpu::Limits) -> Result<(), FlyError> {
    let limit = limits
        .max_storage_buffer_binding_size
        .min(limits.max_buffer_size);
    if size > limit {
        // FlyError's existing public representation requires a static message.
        // Retain the exceptional, size-dependent diagnostic for that lifetime.
        return Err(FlyError(Box::leak(format!(
            "SDF volumes exceed GPU storage limit ({:.3} MB > {:.3} MB); use a smaller --surface-sdf-resolution-divisor or --device cpu",
            size as f64 / 1_048_576.0, limit as f64 / 1_048_576.0
        ).into_boxed_str())));
    }
    Ok(())
}

fn gpu_error(message: &'static str, error: impl std::fmt::Display) -> FlyError {
    eprintln!("{message}: {error}");
    FlyError(message)
}

fn shader_source(wide: bool) -> String {
    let source = include_str!("sdf_hit.wgsl");
    if !wide {
        return source.to_owned();
    }
    let source = source.replace("f32", "f64");
    // Match precision.rs: Naga's `lf` suffix keeps inferred local variables
    // and literals in native f64 too, not just storage and function types.
    let bytes = source.as_bytes();
    let mut out = String::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit()
            && (i == 0 || !bytes[i - 1].is_ascii_alphanumeric() && bytes[i - 1] != b'_')
        {
            let start = i;
            while i < bytes.len()
                && (bytes[i].is_ascii_alphanumeric()
                    || bytes[i] == b'.'
                    || (bytes[i] == b'+' || bytes[i] == b'-') && i > start && bytes[i - 1] == b'e')
            {
                i += 1;
            }
            let token = &source[start..i];
            out.push_str(token);
            if !token.starts_with("0x") && (token.contains('.') || token.contains('e')) {
                out.push_str("lf");
            }
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn storage_limit_diagnostic() {
        let limits = wgpu::Limits {
            max_storage_buffer_binding_size: 1_048_576,
            ..Default::default()
        };
        assert_eq!(
            check_storage(2_097_152, &limits).unwrap_err().0,
            "SDF volumes exceed GPU storage limit (2.000 MB > 1.000 MB); use a smaller --surface-sdf-resolution-divisor or --device cpu"
        );
    }
}

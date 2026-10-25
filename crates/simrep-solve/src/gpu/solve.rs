//! Masked Galerkin multigrid corrections in f32 and an f64 outer iterate.
use std::{borrow::Cow, sync::mpsc, time::Instant};

use rayon::prelude::*;
use wgpu::util::DeviceExt;

use super::{GPU_TIMEOUT, Gpu, GpuResult};
use crate::mg::{Geometry, Grid, Options, Residual};
use std::time::Duration;

mod poisson;
pub use poisson::Poisson;

#[derive(Debug, Default)]
pub struct Timings {
    /// Validation, shader compilation, and GPU Galerkin hierarchy construction.
    pub setup: Duration,
    /// Initial potential, stencil, and mask upload, including completion.
    pub upload: Duration,
    /// V-cycles, f64 residuals, and convergence scalar readbacks.
    pub solve: Duration,
    /// Final potential download and host copy.
    pub readback: Duration,
}

#[derive(Debug)]
pub struct SolveReport {
    pub converged: bool,
    pub iterations: usize,
    pub levels: usize,
    pub residual_history: Vec<Residual>,
    pub elapsed: Duration,
    pub timings: Timings,
    /// False means each iteration transfers a full correction and residual.
    pub gpu_f64: bool,
}

struct HostFine {
    grid: Grid,
    axes: Vec<f64>,
    offsets: [u32; 4],
    normalization: f64,
    cuts: Option<(Vec<u32>, Vec<crate::cutcell::CutRow>, f64)>,
}

impl HostFine {
    fn new(grid: Grid) -> Self {
        let normalization = grid
            .spacing
            .iter()
            .enumerate()
            .filter(|(a, _)| grid.dims[*a] > 1)
            .map(|(_, h)| 1.0 / (h * h))
            .fold(0.0_f64, f64::max);
        let mut axes = Vec::new();
        let mut offsets = [0; 4];
        for (a, offset) in offsets.iter_mut().enumerate().take(3) {
            *offset = axes.len() as u32;
            let n = grid.dims[a];
            for i in 0..n {
                let (minus, plus, weight) = if n == 1 {
                    (0.0, 0.0, 1.0)
                } else if let Geometry::Cylindrical { radial_origin } = grid.geometry
                    && a == 0
                {
                    let r = radial_origin / grid.spacing[0] + i as f64;
                    if i == 0 && radial_origin == 0.0 {
                        (0.0, 4.0, 0.125)
                    } else if i == 0 {
                        (0.0, 2.0, (r + 0.5) * 0.5)
                    } else if i == n - 1 {
                        (2.0, 0.0, (r - 0.5) * 0.5)
                    } else {
                        (1.0 - 0.5 / r, 1.0 + 0.5 / r, r)
                    }
                } else if i == 0 {
                    (0.0, 2.0, 0.5)
                } else if i == n - 1 {
                    (2.0, 0.0, 0.5)
                } else {
                    (1.0, 1.0, 1.0)
                };
                let scale = 1.0 / (grid.spacing[a] * grid.spacing[a]) / normalization;
                axes.extend_from_slice(&[minus * scale, plus * scale, weight]);
            }
        }
        offsets[3] = axes.len() as u32;
        Self {
            grid,
            axes,
            offsets,
            normalization,
            cuts: None,
        }
    }

    fn visit(&self, i: usize, mut f: impl FnMut(usize, f64)) -> f64 {
        let [nx, ny, _] = self.grid.dims;
        let c = [i % nx, i / nx % ny, i / (nx * ny)];
        let strides = [1, nx, nx * ny];
        let mut w = 1.0;
        for a in 0..3 {
            let k = self.offsets[a] as usize + 3 * c[a];
            if c[a] > 0 {
                f(i - strides[a], self.axes[k]);
            }
            if c[a] + 1 < self.grid.dims[a] {
                f(i + strides[a], self.axes[k + 1]);
            }
            w *= self.axes[k + 2];
        }
        w
    }

    fn scale(&self, x: &[f64], mask: &[bool]) -> f64 {
        let scale = (0..x.len())
            .into_par_iter()
            .filter(|&i| !mask[i])
            .map(|i| {
                if let Some(row) = self.cut(i) {
                    let mut b = row.source(self.cuts.as_ref().unwrap().2) / self.normalization;
                    let strides = [1, self.grid.dims[0], self.grid.dims[0] * self.grid.dims[1]];
                    for s in 0..6 {
                        if row.neighbors[s] != 0.0 {
                            let j = i.wrapping_add_signed(
                                strides[s / 2] as isize * if s % 2 == 0 { -1 } else { 1 },
                            );
                            if mask[j] {
                                b += row.neighbors[s] / self.normalization * x[j];
                            }
                        }
                    }
                    return b.abs();
                }
                let mut b = 0.0;
                self.visit(i, |j, a| {
                    if mask[j] {
                        b += a * x[j];
                    }
                });
                b.abs()
            })
            .reduce(|| 0.0, f64::max);
        if scale > 0.0 {
            scale
        } else {
            (0..3)
                .map(|a| self.axes[self.offsets[a] as usize + 1])
                .sum::<f64>()
                .max(1.0 / self.normalization)
        }
    }

    fn residual(&self, x: &[f64], mask: &[bool], rhs: &mut [f32]) -> f64 {
        rhs.par_iter_mut()
            .enumerate()
            .map(|(i, out)| {
                if mask[i] {
                    *out = 0.0;
                    return 0.0;
                }
                if let Some(row) = self.cut(i) {
                    let [nx, ny, _] = self.grid.dims;
                    let r = row.residual(x, [1, nx, nx * ny], self.cuts.as_ref().unwrap().2)
                        / self.normalization;
                    let weight = self.visit(i, |_, _| {});
                    *out = (weight * r) as f32;
                    return if r.is_finite() {
                        r.abs()
                    } else {
                        f64::INFINITY
                    };
                }
                let mut r = 0.0;
                let w = self.visit(i, |j, a| r += a * (x[j] - x[i]));
                *out = (w * r) as f32;
                if r.is_finite() {
                    r.abs()
                } else {
                    f64::INFINITY
                }
            })
            .reduce(|| 0.0, f64::max)
    }

    fn cut(&self, i: usize) -> Option<&crate::cutcell::CutRow> {
        self.cuts
            .as_ref()
            .and_then(|(slots, rows, _)| (slots[i] > 0).then(|| &rows[slots[i] as usize - 1]))
    }

    fn cut_data(&self) -> Vec<f64> {
        let mut data = Vec::new();
        if let Some((_, rows, gauge)) = &self.cuts {
            for row in rows {
                data.extend(row.neighbors.map(|v| v / self.normalization));
                data.push(row.diagonal() / self.normalization);
                data.push(row.source(*gauge) / self.normalization);
            }
        }
        data
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
    smooth: wgpu::ComputePipeline,
    residual: wgpu::ComputePipeline,
    restrict: wgpu::ComputePipeline,
    prolong: wgpu::ComputePipeline,
    galerkin: wgpu::ComputePipeline,
    dummy: wgpu::Buffer,
}

fn dispatch_size(count: usize) -> [u32; 2] {
    let groups = count.div_ceil(128);
    let y = groups.div_ceil(65535);
    [groups.div_ceil(y) as u32, y as u32]
}

fn dispatch(
    encoder: &mut wgpu::CommandEncoder,
    pipeline: &wgpu::ComputePipeline,
    bindings: &wgpu::BindGroup,
    count: usize,
) {
    let [x, y] = dispatch_size(count);
    let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor::default());
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, bindings, &[]);
    pass.dispatch_workgroups(x, y, 1);
}

impl Gpu {
    fn storage(&self, size: u64, label: &str) -> GpuResult<wgpu::Buffer> {
        let limits = self.device.limits();
        if size > limits.max_storage_buffer_binding_size || size > limits.max_buffer_size {
            return Err(format!(
                "{label} needs {size} bytes; adapter storage binding limit is {}",
                limits.max_storage_buffer_binding_size
            )
            .into());
        }
        Ok(self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: size.max(4),
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }))
    }
    fn uploaded<T: bytemuck::Pod>(&self, values: &[T], label: &str) -> wgpu::Buffer {
        self.device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents: bytemuck::cast_slice(values),
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_SRC
                    | wgpu::BufferUsages::COPY_DST,
            })
    }
    fn finish(&self, encoder: wgpu::CommandEncoder) -> GpuResult<()> {
        let submission = self.queue.submit([encoder.finish()]);
        self.device.poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: Some(Duration::from_secs(300)),
        })?;
        Ok(())
    }
    fn download<T: bytemuck::Pod + Copy>(
        &self,
        buffer: &wgpu::Buffer,
        count: usize,
    ) -> GpuResult<Vec<T>> {
        let bytes = (count * std::mem::size_of::<T>()) as u64;
        let staging = self.readback_buffer(bytes, "solver readback");
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(buffer, 0, &staging, 0, bytes);
        self.queue.submit([encoder.finish()]);
        let (sender, receiver) = mpsc::channel();
        staging.slice(..).map_async(wgpu::MapMode::Read, move |r| {
            let _ = sender.send(r);
        });
        self.device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(Duration::from_secs(300)),
        })?;
        receiver.recv_timeout(GPU_TIMEOUT)??;
        let data = staging.slice(..).get_mapped_range()?;
        let result = bytemuck::cast_slice(&data).to_vec();
        drop(data);
        staging.unmap();
        Ok(result)
    }

    /// Solve the same discrete Laplace problem and residual criterion as `mg::solve`.
    /// A single f32 V-cycle approximates each f64 residual correction. If native
    /// shader f64 is absent, the outer iterate/residual stay on the CPU instead.
    pub fn solve(
        &self,
        potential: &mut [f64],
        mask: &[bool],
        grid: Grid,
        options: Options,
    ) -> GpuResult<SolveReport> {
        self.solve_with_precision(potential, mask, grid, options, self.has_f64())
    }

    /// Explicit CPU residual mode is useful for validating the fallback even on
    /// an f64-capable Vulkan adapter. `gpu_f64=true` requires SHADER_F64.
    pub fn solve_with_precision(
        &self,
        potential: &mut [f64],
        mask: &[bool],
        grid: Grid,
        options: Options,
        gpu_f64: bool,
    ) -> GpuResult<SolveReport> {
        self.solve_inner(potential, mask, grid, options, gpu_f64, None)
    }

    /// Symmetric linear ghost-fluid solve, sharing the CPU geometry preparation.
    pub fn solve_sdf(
        &self,
        potential: &mut [f64],
        cuts: &crate::cutcell::CutCells,
        options: Options,
    ) -> GpuResult<SolveReport> {
        self.solve_sdf_with_precision(potential, cuts, options, self.has_f64())
    }

    pub fn solve_sdf_with_precision(
        &self,
        potential: &mut [f64],
        cuts: &crate::cutcell::CutCells,
        options: Options,
        gpu_f64: bool,
    ) -> GpuResult<SolveReport> {
        if potential.len() != cuts.fixed.len() {
            return Err("SDF potential length mismatch".into());
        }
        for (i, value) in potential.iter_mut().enumerate() {
            if cuts.fixed[i] {
                *value = cuts.values[i];
            }
        }
        self.solve_inner(
            potential,
            &cuts.fixed,
            cuts.grid,
            options,
            gpu_f64,
            Some(cuts),
        )
    }

    pub(crate) fn solve_inner(
        &self,
        potential: &mut [f64],
        mask: &[bool],
        grid: Grid,
        options: Options,
        gpu_f64: bool,
        cuts: Option<&crate::cutcell::CutCells>,
    ) -> GpuResult<SolveReport> {
        let started = Instant::now();
        validate(potential, mask, grid, options)?;
        if gpu_f64 && !self.has_f64() {
            return Err("SHADER_F64 unavailable".into());
        }
        let origin = potential[mask.iter().position(|&m| m).unwrap()];
        let mut host_x: Vec<_> = potential.par_iter().map(|&v| v - origin).collect();
        let mut fine = HostFine::new(grid);
        if let Some(cuts) = cuts {
            if cuts.cut_nodes() > (1 << 24) {
                return Err("cut-row slots exceed the exact f32 integer range".into());
            }
            fine.cuts = Some((cuts.slots.clone(), cuts.rows.clone(), origin));
        }
        let cut_data = fine.cut_data();
        let scale = fine.scale(&host_x, mask);
        let kernels = Kernels::new(self)?;
        let mut dims = vec![grid.dims];
        while dims.last().unwrap().iter().any(|&n| n > 4) {
            dims.push(
                dims.last()
                    .unwrap()
                    .map(|n| if n <= 2 { n } else { n / 2 + 1 }),
            );
        }
        // Check every large binding before allocating/uploading any of them.
        for (i, d) in dims.iter().enumerate() {
            let count = d.iter().product::<usize>() as u64;
            let size = if i == 0 {
                (count + fine.axes.len() as u64 + cut_data.len() as u64) * 4
            } else {
                count * 108
            };
            if size > self.device.limits().max_storage_buffer_binding_size {
                return Err(format!(
                    "level {i} matrix needs {size} bytes; storage binding limit is {}",
                    self.device.limits().max_storage_buffer_binding_size
                )
                .into());
            }
        }
        if gpu_f64
            && potential.len() as u64 * 8 > self.device.limits().max_storage_buffer_binding_size
        {
            return Err("f64 potential exceeds adapter storage binding limit".into());
        }
        let mut timings = Timings {
            setup: started.elapsed(),
            ..Default::default()
        };
        let upload = Instant::now();
        let mut matrix: Vec<f32> = fine.axes.iter().map(|&v| v as f32).collect();
        matrix.extend(mask.iter().enumerate().map(|(i, &m)| {
            if m {
                1.0
            } else if let Some((slots, _, _)) = &fine.cuts {
                -(slots[i] as f32)
            } else {
                0.0
            }
        }));
        matrix.extend(cut_data.iter().map(|v| *v as f32));
        if !matrix.iter().all(|v| v.is_finite()) {
            return Err("grid coefficients cannot be represented in f32".into());
        }
        let mut levels = Vec::new();
        for (i, &d) in dims.iter().enumerate() {
            let count = d.iter().product::<usize>() as u64;
            let child = dims.get(i + 1).copied().unwrap_or(d);
            let words = [
                d[0] as u32,
                d[1] as u32,
                d[2] as u32,
                u32::from(i == 0),
                child[0] as u32,
                child[1] as u32,
                child[2] as u32,
                0,
                fine.offsets[0],
                fine.offsets[1],
                fine.offsets[2],
                fine.offsets[3],
            ];
            let params = self
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("level parameters"),
                    contents: bytemuck::cast_slice(&words),
                    usage: wgpu::BufferUsages::UNIFORM,
                });
            levels.push(Level {
                dims: d,
                params,
                matrix: if i == 0 {
                    self.uploaded(&matrix, "fine stencil and mask")
                } else {
                    self.storage(count * 108, "Galerkin matrix")?
                },
                x: [
                    self.storage(count * 4, "correction")?,
                    self.storage(count * 4, "Jacobi work")?,
                ],
                rhs: self.storage(count * 4, "correction rhs")?,
                current: 0,
            });
        }
        drop(matrix);
        let native = if gpu_f64 {
            Some(Native::new(self, &fine, &host_x)?)
        } else {
            None
        };
        self.finish(self.device.create_command_encoder(&Default::default()))?;
        timings.upload = upload.elapsed();
        if let Some(native) = &native {
            timings.upload -= native.compilation;
            timings.setup += native.compilation;
        }
        let setup = Instant::now();
        let mut encoder = self.device.create_command_encoder(&Default::default());
        for i in 0..levels.len() - 1 {
            let bindings = kernels.bind(
                self,
                &levels[i],
                &kernels.dummy,
                &kernels.dummy,
                &levels[i + 1].matrix,
                &kernels.dummy,
            );
            dispatch(
                &mut encoder,
                &kernels.galerkin,
                &bindings,
                dims[i + 1].iter().product(),
            );
        }
        self.finish(encoder)?;
        timings.setup += setup.elapsed();
        let solving = Instant::now();
        let count = potential.len();
        let mut cpu_rhs = if gpu_f64 {
            Vec::new()
        } else {
            vec![0.0; count]
        };
        let mut absolute = if let Some(native) = &native {
            native.residual(self, &levels[0])?
        } else {
            let norm = fine.residual(&host_x, mask, &mut cpu_rhs);
            self.queue
                .write_buffer(&levels[0].rhs, 0, bytemuck::cast_slice(&cpu_rhs));
            norm
        };
        let mut history = Vec::new();
        let mut iterations = 0;
        loop {
            if !absolute.is_finite() {
                return Err("GPU refinement produced non-finite residual".into());
            }
            history.push(Residual {
                absolute: absolute * fine.normalization,
                relative: absolute / scale,
            });
            if absolute
                <= options.absolute_tolerance / fine.normalization
                    + options.relative_tolerance * scale
                || iterations == options.max_cycles
            {
                break;
            }
            let mut encoder = self.device.create_command_encoder(&Default::default());
            levels[0].current = 0;
            encoder.clear_buffer(&levels[0].x[0], 0, None);
            cycle(self, &kernels, &mut levels, &mut encoder, options);
            if let Some(native) = &native {
                native.update(self, &levels[0], &mut encoder);
                self.queue.submit([encoder.finish()]);
                absolute = native.residual(self, &levels[0])?;
            } else {
                self.queue.submit([encoder.finish()]);
                let correction = self.download::<f32>(&levels[0].x[levels[0].current], count)?;
                host_x
                    .par_iter_mut()
                    .zip(correction)
                    .zip(mask)
                    .for_each(|((v, e), &fixed)| {
                        if !fixed {
                            *v += f64::from(e);
                        }
                    });
                absolute = fine.residual(&host_x, mask, &mut cpu_rhs);
                self.queue
                    .write_buffer(&levels[0].rhs, 0, bytemuck::cast_slice(&cpu_rhs));
            }
            iterations += 1;
        }
        timings.solve = solving.elapsed();
        let readback = Instant::now();
        if let Some(native) = &native {
            host_x = self.download::<f64>(&native.x, count)?;
        }
        potential
            .par_iter_mut()
            .zip(host_x)
            .zip(mask)
            .for_each(|((v, x), &fixed)| {
                if !fixed {
                    *v = x + origin;
                }
            });
        timings.readback = readback.elapsed();
        Ok(SolveReport {
            converged: absolute
                <= options.absolute_tolerance / fine.normalization
                    + options.relative_tolerance * scale,
            iterations,
            levels: levels.len(),
            residual_history: history,
            elapsed: started.elapsed(),
            timings,
            gpu_f64,
        })
    }
}

impl Kernels {
    fn new(gpu: &Gpu) -> GpuResult<Self> {
        let scope = gpu.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let entries: Vec<_> = (0..6)
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
            .collect();
        let layout = gpu
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("multigrid layout"),
                entries: &entries,
            });
        let pipeline_layout = gpu
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: None,
                bind_group_layouts: &[Some(&layout)],
                immediate_size: 0,
            });
        // FXC tries to unroll all gather loops around a dynamically indexed
        // writable local array, even inside a specialized-away branch. Give it
        // a source with no such array. Each invocation owns its storage row.
        let assembly = if gpu.info.backend == wgpu::Backend::Dx12 {
            [
                "for(var s=0u;s<27u;s++) { out[s*count(p.coarse.xyz)+i]=0.0; }",
                "out[slot*count(p.coarse.xyz)+i] += left*a*right;",
                "",
            ]
        } else {
            [
                "var row: array<f32,27>;",
                "row[slot] += left*a*right;",
                "for(var s=0u;s<27u;s++) { out[s*count(p.coarse.xyz)+i]=row[s]; }",
            ]
        };
        let source = include_str!("multigrid.wgsl")
            .replace("ASSEMBLY_INIT", assembly[0])
            .replace("ASSEMBLY_ADD", assembly[1])
            .replace("ASSEMBLY_STORE", assembly[2]);
        let module = gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("f32 Galerkin multigrid"),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            });
        let pipeline = |name| {
            gpu.device
                .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some(name),
                    layout: Some(&pipeline_layout),
                    module: &module,
                    entry_point: Some(name),
                    compilation_options: Default::default(),
                    cache: None,
                })
        };
        let result = Self {
            layout,
            smooth: pipeline("jacobi_sweep"),
            residual: pipeline("residual"),
            restrict: pipeline("restrict_rhs"),
            prolong: pipeline("prolong"),
            galerkin: pipeline("galerkin"),
            dummy: gpu.storage(4, "dummy read buffer")?,
        };
        if let Some(error) = pollster::block_on(scope.pop()) {
            return Err(error.into());
        }
        Ok(result)
    }
    fn bind(
        &self,
        gpu: &Gpu,
        level: &Level,
        x: &wgpu::Buffer,
        rhs: &wgpu::Buffer,
        out: &wgpu::Buffer,
        aux: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        let buffers = [&level.params, &level.matrix, x, rhs, out, aux];
        let entries: Vec<_> = buffers
            .iter()
            .enumerate()
            .map(|(i, b)| wgpu::BindGroupEntry {
                binding: i as u32,
                resource: b.as_entire_binding(),
            })
            .collect();
        gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.layout,
            entries: &entries,
        })
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
        let bindings = k.bind(
            gpu,
            level,
            &level.x[level.current],
            &level.rhs,
            &level.x[1 - level.current],
            &k.dummy,
        );
        dispatch(encoder, &k.smooth, &bindings, level.dims.iter().product());
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
    let bindings = k.bind(
        gpu,
        level,
        &level.x[level.current],
        &level.rhs,
        &level.x[1 - level.current],
        &k.dummy,
    );
    dispatch(encoder, &k.residual, &bindings, level.dims.iter().product());
    let bindings = k.bind(
        gpu,
        level,
        &level.x[1 - level.current],
        &k.dummy,
        &tail[0].rhs,
        &k.dummy,
    );
    dispatch(
        encoder,
        &k.restrict,
        &bindings,
        tail[0].dims.iter().product(),
    );
    tail[0].current = 0;
    encoder.clear_buffer(&tail[0].x[0], 0, None);
    cycle(gpu, k, tail, encoder, options);
    let bindings = k.bind(
        gpu,
        level,
        &level.x[level.current],
        &k.dummy,
        &level.x[1 - level.current],
        &tail[0].x[tail[0].current],
    );
    dispatch(encoder, &k.prolong, &bindings, level.dims.iter().product());
    level.current = 1 - level.current;
    smooth(gpu, k, level, encoder, options.post_smooth);
}

struct Native {
    compilation: Duration,
    x: wgpu::Buffer,
    axes: wgpu::Buffer,
    norms: Vec<wgpu::Buffer>,
    residual: wgpu::ComputePipeline,
    update: wgpu::ComputePipeline,
    reduce: wgpu::ComputePipeline,
}
impl Native {
    fn new(gpu: &Gpu, fine: &HostFine, x: &[f64]) -> GpuResult<Self> {
        let started = Instant::now();
        let scope = gpu.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let pipeline = |source: &'static str, update: bool| {
            let defines: &[(&str, &str)] = if update { &[("UPDATE", "1")] } else { &[] };
            let module = gpu
                .device
                .create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some("f64 refinement"),
                    source: wgpu::ShaderSource::Glsl {
                        shader: Cow::Borrowed(source),
                        stage: wgpu::naga::ShaderStage::Compute,
                        defines,
                    },
                });
            gpu.device
                .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: None,
                    layout: None,
                    module: &module,
                    entry_point: Some("main"),
                    compilation_options: Default::default(),
                    cache: None,
                })
        };
        let residual = pipeline(include_str!("solve_f64.comp"), false);
        let update = pipeline(include_str!("solve_f64.comp"), true);
        let reduce = pipeline(include_str!("reduce_f64.comp"), false);
        if let Some(error) = pollster::block_on(scope.pop()) {
            return Err(error.into());
        }
        let compilation = started.elapsed();
        let mut count = x.len().div_ceil(128);
        let mut norms = vec![gpu.storage((count * 8) as u64, "residual norm blocks")?];
        while count > 1 {
            count = count.div_ceil(128);
            norms.push(gpu.storage((count * 8) as u64, "residual norm reduction")?);
        }
        Ok(Self {
            compilation,
            x: gpu.uploaded(x, "f64 potential"),
            axes: {
                let mut data = fine.axes.clone();
                data.extend(fine.cut_data());
                gpu.uploaded(&data, "f64 axes and cut rows")
            },
            norms,
            residual,
            update,
            reduce,
        })
    }
    fn bindings(
        &self,
        gpu: &Gpu,
        pipeline: &wgpu::ComputePipeline,
        level: &Level,
        update: bool,
    ) -> wgpu::BindGroup {
        let entries = if update {
            vec![
                (0, &level.params),
                (2, &self.x),
                (5, &level.x[level.current]),
                (6, &level.matrix),
            ]
        } else {
            vec![
                (0, &level.params),
                (1, &self.axes),
                (2, &self.x),
                (3, &level.rhs),
                (4, &self.norms[0]),
                (6, &level.matrix),
            ]
        };
        let entries: Vec<_> = entries
            .into_iter()
            .map(|(binding, b)| wgpu::BindGroupEntry {
                binding,
                resource: b.as_entire_binding(),
            })
            .collect();
        gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &pipeline.get_bind_group_layout(0),
            entries: &entries,
        })
    }
    fn update(&self, gpu: &Gpu, level: &Level, encoder: &mut wgpu::CommandEncoder) {
        let bindings = self.bindings(gpu, &self.update, level, true);
        dispatch(
            encoder,
            &self.update,
            &bindings,
            level.dims.iter().product(),
        );
    }
    fn residual(&self, gpu: &Gpu, level: &Level) -> GpuResult<f64> {
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        let bindings = self.bindings(gpu, &self.residual, level, false);
        dispatch(
            &mut encoder,
            &self.residual,
            &bindings,
            (self.norms[0].size() / 8) as usize,
        );
        for pair in self.norms.windows(2) {
            let entries = [
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: pair[0].as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: pair[1].as_entire_binding(),
                },
            ];
            let bindings = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &self.reduce.get_bind_group_layout(0),
                entries: &entries,
            });
            dispatch(
                &mut encoder,
                &self.reduce,
                &bindings,
                (pair[1].size() / 8) as usize,
            );
        }
        gpu.queue.submit([encoder.finish()]);
        Ok(gpu.download::<f64>(self.norms.last().unwrap(), 1)?[0])
    }
}

fn validate(x: &[f64], mask: &[bool], grid: Grid, options: Options) -> GpuResult<()> {
    let [nx, ny, nz] = grid.dims;
    let count = nx.checked_mul(ny).and_then(|n| n.checked_mul(nz));
    if nx < 2
        || ny < 2
        || nz < 1
        || count != Some(x.len())
        || mask.len() != x.len()
        || x.len() > u32::MAX as usize
    {
        return Err(
            "grid dimensions and slice lengths must match (nx,ny >= 2, nz >= 1, count <= u32::MAX)"
                .into(),
        );
    }
    if !mask.iter().any(|&m| m) || !x.iter().all(|v| v.is_finite()) {
        return Err("finite potentials and at least one fixed electrode are required".into());
    }
    if !grid
        .spacing
        .iter()
        .all(|&h| h.is_finite() && h > 0.0 && (1.0 / (h * h)).is_finite() && 1.0 / (h * h) > 0.0)
    {
        return Err("spacing must be positive, finite and numerically representable".into());
    }
    if let Geometry::Cylindrical { radial_origin } = grid.geometry
        && (nz != 1 || !radial_origin.is_finite() || radial_origin < 0.0)
    {
        return Err("cylindrical grids require nz=1 and finite radial_origin >= 0".into());
    }
    if !options.relative_tolerance.is_finite()
        || options.relative_tolerance < 0.0
        || !options.absolute_tolerance.is_finite()
        || options.absolute_tolerance < 0.0
        || options.relative_tolerance + options.absolute_tolerance <= 0.0
        || options.pre_smooth == 0
        || options.post_smooth == 0
    {
        return Err(
            "tolerances must be finite, nonnegative (one positive), smoothing counts positive"
                .into(),
        );
    }
    Ok(())
}

//! Additive physical-RHS path; the Laplace and cut-cell solvers are unchanged.
use super::*;

/// Reusable masked Galerkin hierarchy for `-L phi = source`. The native f64
/// path and the CPU-residual fallback use the same physical source, row
/// weights and stopping criterion. Construction is preparation; all `solve`
/// work, including transfers and residuals, is computation.
pub struct Poisson {
    fine: HostFine,
    mask: Vec<bool>,
    options: Options,
    kernels: Kernels,
    levels: Vec<Level>,
    native: Option<Native>,
    source: wgpu::Buffer,
    residual: Option<wgpu::ComputePipeline>,
    max_source: f64,
}

impl Gpu {
    pub fn solve_poisson(
        &self,
        potential: &mut [f64],
        mask: &[bool],
        source: &[f64],
        grid: Grid,
        options: Options,
    ) -> GpuResult<SolveReport> {
        self.solve_poisson_with_precision(potential, mask, source, grid, options, self.has_f64())
    }

    pub fn solve_poisson_with_precision(
        &self,
        potential: &mut [f64],
        mask: &[bool],
        source: &[f64],
        grid: Grid,
        options: Options,
        gpu_f64: bool,
    ) -> GpuResult<SolveReport> {
        let started = Instant::now();
        let mut solver = Poisson::new(self, potential, mask, grid, options, gpu_f64)?;
        let setup = started.elapsed();
        let mut report = solver.solve(self, potential, source)?;
        report.timings.setup += setup;
        report.elapsed = started.elapsed();
        Ok(report)
    }
}

impl Poisson {
    pub fn new(
        gpu: &Gpu,
        potential: &[f64],
        mask: &[bool],
        grid: Grid,
        options: Options,
        gpu_f64: bool,
    ) -> GpuResult<Self> {
        Self::from_fine(gpu, potential, mask, HostFine::new(grid), options, gpu_f64)
    }

    /// Reuse an already prepared variable-coefficient operator. Fixed values
    /// remain the caller's potential values; the stencil mask cannot change.
    pub fn new_prepared(
        gpu: &Gpu,
        potential: &[f64],
        stencil: &crate::cutcell::CutCells,
        options: Options,
        gpu_f64: bool,
    ) -> GpuResult<Self> {
        if stencil.cut_nodes() > (1 << 24) {
            return Err("prepared-row slots exceed the exact f32 integer range".into());
        }
        let mut fine = HostFine::new(stencil.grid);
        fine.cuts = Some((stencil.slots.clone(), stencil.rows.clone(), 0.0));
        Self::from_fine(gpu, potential, &stencil.fixed, fine, options, gpu_f64)
    }

    fn from_fine(
        gpu: &Gpu,
        potential: &[f64],
        mask: &[bool],
        fine: HostFine,
        options: Options,
        gpu_f64: bool,
    ) -> GpuResult<Self> {
        let grid = fine.grid;
        validate(potential, mask, grid, options)?;
        if gpu_f64 && !gpu.has_f64() {
            return Err("SHADER_F64 unavailable".into());
        }
        let kernels = Kernels::new(gpu)?;
        let mut dims = vec![grid.dims];
        while dims.last().unwrap().iter().any(|&n| n > 4) {
            dims.push(
                dims.last()
                    .unwrap()
                    .map(|n| if n <= 2 { n } else { n / 2 + 1 }),
            );
        }
        let mut matrix: Vec<f32> = fine.axes.iter().map(|&v| v as f32).collect();
        matrix.extend(mask.iter().enumerate().map(|(i, &m)| {
            if m {
                1.0
            } else {
                fine.cuts
                    .as_ref()
                    .map_or(0.0, |(slots, _, _)| -(slots[i] as f32))
            }
        }));
        matrix.extend(fine.cut_data().iter().map(|&v| v as f32));
        if !matrix.iter().all(|v| v.is_finite()) {
            return Err("Poisson coefficients overflow f32".into());
        }
        // Check matrix sizes before allocating or uploading large buffers.
        for (i, d) in dims.iter().enumerate() {
            let size = if i == 0 {
                matrix.len() as u64 * 4
            } else {
                d.iter().product::<usize>() as u64 * 108
            };
            if size > gpu.device.limits().max_storage_buffer_binding_size {
                return Err("Poisson matrix exceeds adapter binding limit".into());
            }
        }
        let mut levels = Vec::new();
        for (i, &d) in dims.iter().enumerate() {
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
            levels.push(Level {
                dims: d,
                params: gpu
                    .device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("Poisson level"),
                        contents: bytemuck::cast_slice(&words),
                        usage: wgpu::BufferUsages::UNIFORM,
                    }),
                matrix: if i == 0 {
                    gpu.uploaded(&matrix, "Poisson stencil and mask")
                } else {
                    gpu.storage(
                        d.iter().product::<usize>() as u64 * 108,
                        "Poisson Galerkin matrix",
                    )?
                },
                x: [
                    gpu.storage(d.iter().product::<usize>() as u64 * 4, "Poisson correction")?,
                    gpu.storage(d.iter().product::<usize>() as u64 * 4, "Poisson work")?,
                ],
                rhs: gpu.storage(
                    d.iter().product::<usize>() as u64 * 4,
                    "Poisson weighted residual",
                )?,
                current: 0,
            });
        }
        let native = if gpu_f64 {
            Some(Native::new(gpu, &fine, potential)?)
        } else {
            None
        };
        let source = gpu.storage(
            potential.len() as u64 * 8,
            "normalized physical Poisson source",
        )?;
        let residual = if gpu_f64 {
            let scope = gpu.device.push_error_scope(wgpu::ErrorFilter::Validation);
            let shader = include_str!("../solve_f64.comp")
                .replace("void main() {","layout(set=0,binding=7,std430) readonly buffer Source { double source[]; };\nvoid main() {")
                .replace("rhs[i]=float(w*r);","r+=source[i];\n rhs[i]=float(w*r);");
            let module = gpu
                .device
                .create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some("physical f64 residual"),
                    source: wgpu::ShaderSource::Glsl {
                        shader: Cow::Owned(shader),
                        stage: wgpu::naga::ShaderStage::Compute,
                        defines: &[],
                    },
                });
            let pipeline = gpu
                .device
                .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some("physical f64 residual"),
                    layout: None,
                    module: &module,
                    entry_point: Some("main"),
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
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        for i in 0..levels.len() - 1 {
            let bindings = kernels.bind(
                gpu,
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
        gpu.finish(encoder)?;
        Ok(Self {
            fine,
            mask: mask.to_vec(),
            options,
            kernels,
            levels,
            native,
            source,
            residual,
            max_source: 0.0,
        })
    }

    /// Buffer values are source / normalization, where normalization is the
    /// largest active-axis 1/h². A producer such as GPU CIC may write it directly.
    pub fn normalized_source_buffer(&self) -> &wgpu::Buffer {
        &self.source
    }
    pub fn normalization(&self) -> f64 {
        self.fine.normalization
    }
    pub fn max_source(&self) -> f64 {
        self.max_source
    }
    /// Native iterate for device-side gathering, after `solve_device`.
    pub fn potential_buffer(&self) -> Option<&wgpu::Buffer> {
        self.native.as_ref().map(|n| &n.x)
    }

    pub fn solve(
        &mut self,
        gpu: &Gpu,
        potential: &mut [f64],
        source: &[f64],
    ) -> GpuResult<SolveReport> {
        if source.len() != self.mask.len() || source.iter().any(|v| !v.is_finite()) {
            return Err("physical Poisson source must be finite and match the grid".into());
        }
        let normalized: Vec<_> = source.iter().map(|v| v / self.fine.normalization).collect();
        gpu.queue
            .write_buffer(&self.source, 0, bytemuck::cast_slice(&normalized));
        self.solve_device(gpu, potential)
    }

    /// Solve a device-produced physical RHS. Scalar convergence and the mesh
    /// source readback (for scale validation/fallback) are included in timings.
    pub fn solve_device(&mut self, gpu: &Gpu, potential: &mut [f64]) -> GpuResult<SolveReport> {
        let started = Instant::now();
        validate(potential, &self.mask, self.fine.grid, self.options)?;
        let normalized = gpu.download::<f64>(&self.source, self.mask.len())?;
        if normalized.iter().any(|v| !v.is_finite()) {
            return Err("non-finite physical GPU source".into());
        }
        self.max_source = normalized
            .iter()
            .enumerate()
            .filter(|(i, _)| !self.mask[*i])
            .map(|(_, v)| v.abs())
            .fold(0.0, f64::max)
            * self.fine.normalization;
        // Keep the absolute gauge here: unlike Laplace refinement, a device
        // gather must see the physical potential rather than a shifted iterate.
        let mut host_x = potential.to_vec();
        if let Some(native) = &self.native {
            gpu.queue
                .write_buffer(&native.x, 0, bytemuck::cast_slice(&host_x));
        }
        let mut cpu_rhs = vec![0.0; host_x.len()];
        // CPU Poisson uses the gauge-shifted *combined* reduced RHS, not the
        // maximum of a boundary scale and a source scale. In particular, a
        // small nonzero PIC source must not inherit the unit-voltage fallback.
        let origin = host_x[self.mask.iter().position(|&m| m).unwrap()];
        let mut scale = normalized
            .par_iter()
            .enumerate()
            .filter(|(i, _)| !self.mask[*i])
            .map(|(i, &source)| {
                let mut rhs = source;
                if let Some(row) = self.fine.cut(i) {
                    rhs += row.source(origin) / self.fine.normalization;
                    let strides = [
                        1,
                        self.fine.grid.dims[0],
                        self.fine.grid.dims[0] * self.fine.grid.dims[1],
                    ];
                    for s in 0..6 {
                        if row.neighbors[s] != 0.0 {
                            let j = i.wrapping_add_signed(
                                strides[s / 2] as isize * if s % 2 == 0 { -1 } else { 1 },
                            );
                            if self.mask[j] {
                                rhs += row.neighbors[s] / self.fine.normalization
                                    * (host_x[j] - origin);
                            }
                        }
                    }
                    return rhs.abs();
                }
                self.fine.visit(i, |j, a| {
                    if self.mask[j] {
                        rhs += a * (host_x[j] - origin);
                    }
                });
                rhs.abs()
            })
            .reduce(|| 0.0, f64::max);
        if scale == 0.0 {
            scale = (0..3)
                .map(|a| self.fine.axes[self.fine.offsets[a] as usize + 1])
                .sum::<f64>()
                .max(1.0 / self.fine.normalization);
        }
        let mut absolute = self.physical_residual(gpu, &host_x, &normalized, &mut cpu_rhs)?;
        let mut history = Vec::new();
        let mut iterations = 0;
        loop {
            if !absolute.is_finite() {
                return Err("non-finite physical GPU residual".into());
            }
            history.push(Residual {
                absolute: absolute * self.fine.normalization,
                relative: absolute / scale,
            });
            if absolute
                <= self.options.absolute_tolerance / self.fine.normalization
                    + self.options.relative_tolerance * scale
                || iterations == self.options.max_cycles
            {
                break;
            }
            let mut encoder = gpu.device.create_command_encoder(&Default::default());
            self.levels[0].current = 0;
            encoder.clear_buffer(&self.levels[0].x[0], 0, None);
            cycle(
                gpu,
                &self.kernels,
                &mut self.levels,
                &mut encoder,
                self.options,
            );
            if let Some(native) = &self.native {
                native.update(gpu, &self.levels[0], &mut encoder);
                gpu.queue.submit([encoder.finish()]);
            } else {
                gpu.queue.submit([encoder.finish()]);
                let correction =
                    gpu.download::<f32>(&self.levels[0].x[self.levels[0].current], host_x.len())?;
                host_x
                    .par_iter_mut()
                    .zip(correction)
                    .zip(&self.mask)
                    .for_each(|((x, e), &fixed)| {
                        if !fixed {
                            *x += f64::from(e);
                        }
                    });
            }
            absolute = self.physical_residual(gpu, &host_x, &normalized, &mut cpu_rhs)?;
            iterations += 1;
        }
        if let Some(native) = &self.native {
            host_x = gpu.download::<f64>(&native.x, host_x.len())?;
        }
        potential.copy_from_slice(&host_x);
        Ok(SolveReport {
            converged: absolute
                <= self.options.absolute_tolerance / self.fine.normalization
                    + self.options.relative_tolerance * scale,
            iterations,
            levels: self.levels.len(),
            residual_history: history,
            elapsed: started.elapsed(),
            timings: Timings {
                solve: started.elapsed(),
                ..Default::default()
            },
            gpu_f64: self.native.is_some(),
        })
    }

    fn physical_residual(
        &self,
        gpu: &Gpu,
        x: &[f64],
        source: &[f64],
        rhs: &mut [f32],
    ) -> GpuResult<f64> {
        if let Some(native) = &self.native {
            let pipeline = self.residual.as_ref().unwrap();
            let entries: Vec<_> = [
                (0, &self.levels[0].params),
                (1, &native.axes),
                (2, &native.x),
                (3, &self.levels[0].rhs),
                (4, &native.norms[0]),
                (6, &self.levels[0].matrix),
                (7, &self.source),
            ]
            .into_iter()
            .map(|(binding, b)| wgpu::BindGroupEntry {
                binding,
                resource: b.as_entire_binding(),
            })
            .collect();
            let bindings = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &pipeline.get_bind_group_layout(0),
                entries: &entries,
            });
            let mut encoder = gpu.device.create_command_encoder(&Default::default());
            dispatch(
                &mut encoder,
                pipeline,
                &bindings,
                (native.norms[0].size() / 8) as usize,
            );
            for pair in native.norms.windows(2) {
                let bindings = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: None,
                    layout: &native.reduce.get_bind_group_layout(0),
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: pair[0].as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: pair[1].as_entire_binding(),
                        },
                    ],
                });
                dispatch(
                    &mut encoder,
                    &native.reduce,
                    &bindings,
                    (pair[1].size() / 8) as usize,
                );
            }
            gpu.queue.submit([encoder.finish()]);
            Ok(gpu.download::<f64>(native.norms.last().unwrap(), 1)?[0])
        } else {
            let absolute = rhs
                .par_iter_mut()
                .enumerate()
                .map(|(i, out)| {
                    if self.mask[i] {
                        *out = 0.0;
                        return 0.0;
                    }
                    let mut r = source[i];
                    let weight = if let Some(row) = self.fine.cut(i) {
                        let [nx, ny, _] = self.fine.grid.dims;
                        r += row.residual(x, [1, nx, nx * ny], 0.0) / self.fine.normalization;
                        self.fine.visit(i, |_, _| {})
                    } else {
                        self.fine.visit(i, |j, a| r += a * (x[j] - x[i]))
                    };
                    *out = (weight * r) as f32;
                    if r.is_finite() {
                        r.abs()
                    } else {
                        f64::INFINITY
                    }
                })
                .reduce(|| 0.0, f64::max);
            gpu.queue
                .write_buffer(&self.levels[0].rhs, 0, bytemuck::cast_slice(rhs));
            Ok(absolute)
        }
    }
}

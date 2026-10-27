//! f64 GPU CIC, Poisson field gathering and synchronized Boris pushes.
//! Integer atomics build cell lists; one invocation per node sums its eight
//! neighboring lists in descending source-index order in f64. No floating-point
//! atomics or f32 charge rounding.
use std::{borrow::Cow, sync::mpsc, time::Duration};

use simrep_solve::gpu::{Gpu, Poisson};
use simrep_solve::mg::{Geometry, Grid};
use wgpu::util::DeviceExt;

use super::space_charge::{EPSILON_0_F_PER_MM, PicMesh};
use crate::{
    ELECTRIC_ACCELERATION, Field, FlyError, MAGNETIC_FREQUENCY, Particle, ParticleState,
    SPEED_OF_LIGHT, Vector, dot,
};

type Result<T> = std::result::Result<T, FlyError>;

pub struct GpuPic {
    gpu: Gpu,
    pub grid: Grid,
    origin: Vector,
    fixed: wgpu::Buffer,
    parameters: wgpu::Buffer,
    parameter_values: Vec<f64>,
    particles: wgpu::Buffer,
    heads: wgpu::Buffer,
    links: wgpu::Buffer,
    cells: wgpu::Buffer,
    electric: wgpu::Buffer,
    inputs: wgpu::Buffer,
    output: wgpu::Buffer,
    bin: wgpu::ComputePipeline,
    cell: wgpu::ComputePipeline,
    deposit: wgpu::ComputePipeline,
    field: wgpu::ComputePipeline,
    gather: wgpu::ComputePipeline,
    push: wgpu::ComputePipeline,
    solver: Poisson,
    potential: Vec<f64>,
    capacity: usize,
    count: usize,
    plasma_frequency_per_us: f64,
    pub last_cycles: usize,
    pub last_residual: f64,
}

impl GpuPic {
    /// Prepare buffers and hierarchy once. Explicit GPU mode supports smaller
    /// correctness studies; CLI auto selection uses this path at N >= 100,000.
    pub fn new(gpu: Gpu, mesh: PicMesh, capacity: usize) -> Result<Self> {
        if !gpu.has_f64() {
            return Err(FlyError("GPU PIC requires SHADER_F64"));
        }
        if capacity == 0 || capacity > u32::MAX as usize {
            return Err(FlyError("invalid GPU PIC particle capacity"));
        }
        let bytes = capacity as u64 * 128;
        if bytes > gpu.device().limits().max_storage_buffer_binding_size {
            return Err(FlyError(
                "GPU PIC particles exceed adapter storage binding limit",
            ));
        }
        let potential = mesh.potentials().to_vec();
        let solver = Poisson::new(&gpu, &potential, &mesh.fixed, mesh.grid, mesh.options, true)
            .map_err(|_| FlyError("GPU PIC Poisson preparation failed"))?;
        let scope = gpu.device().push_error_scope(wgpu::ErrorFilter::Validation);
        let pipeline = |kind| {
            let defines = [(kind, "1")];
            let shader = gpu
                .device()
                .create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some("f64 CIC PIC"),
                    source: wgpu::ShaderSource::Glsl {
                        shader: Cow::Borrowed(include_str!("gpu_pic.comp")),
                        stage: wgpu::naga::ShaderStage::Compute,
                        defines: &defines,
                    },
                });
            gpu.device()
                .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some(kind),
                    layout: None,
                    module: &shader,
                    entry_point: Some("main"),
                    compilation_options: Default::default(),
                    cache: None,
                })
        };
        let cell = pipeline("CELL");
        let deposit = pipeline("DEPOSIT");
        let field = pipeline("FIELD");
        let gather = pipeline("GATHER");
        let push = pipeline("PUSH");
        let shader = gpu
            .device()
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("PIC atomic bin lists"),
                source: wgpu::ShaderSource::Wgsl(include_str!("gpu_pic_bin.wgsl").into()),
            });
        let bin = gpu
            .device()
            .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("PIC atomic bin lists"),
                layout: None,
                module: &shader,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                cache: None,
            });
        if let Some(error) = pollster::block_on(scope.pop()) {
            eprintln!("{error}");
            return Err(FlyError("GPU PIC shader validation failed"));
        }
        let upload = |values: &[u32]| {
            gpu.device()
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("PIC mask"),
                    contents: bytemuck::cast_slice(values),
                    usage: wgpu::BufferUsages::STORAGE,
                })
        };
        let buffer = |size, label| {
            gpu.device().create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_DST
                    | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            })
        };
        let count = potential.len() as u64;
        let fixed = upload(&mesh.fixed.iter().map(|&m| u32::from(m)).collect::<Vec<_>>());
        let parameters = buffer(20 * 8, "PIC parameters");
        let particles = buffer(capacity as u64 * 64, "PIC force-time particles");
        let heads = buffer(count * 4, "PIC cell heads");
        let links = buffer(capacity as u64 * 4, "PIC particle links");
        let cells = buffer((capacity as u64 + 1) * 4, "PIC f64 bin addresses");
        let electric = buffer(count * 24, "PIC nodal electric");
        let inputs = buffer(bytes, "PIC push inputs");
        let output = buffer(capacity as u64 * 64, "PIC particle output");
        let cylindrical = matches!(mesh.grid.geometry, Geometry::Cylindrical { .. });
        let radial = match mesh.grid.geometry {
            Geometry::Cylindrical { radial_origin } => radial_origin,
            _ => 0.0,
        };
        let parameter_values = vec![
            mesh.grid.dims[0] as f64,
            mesh.grid.dims[1] as f64,
            mesh.grid.dims[2] as f64,
            0.0,
            mesh.grid.spacing[0],
            mesh.grid.spacing[1],
            mesh.grid.spacing[2],
            solver.normalization(),
            mesh.origin_mm[0],
            mesh.origin_mm[1],
            mesh.origin_mm[2],
            0.0,
            0.0,
            EPSILON_0_F_PER_MM,
            ELECTRIC_ACCELERATION,
            MAGNETIC_FREQUENCY,
            SPEED_OF_LIGHT,
            f64::from(u8::from(cylindrical)),
            radial,
            std::f64::consts::PI,
        ];
        Ok(Self {
            gpu,
            grid: mesh.grid,
            origin: mesh.origin_mm,
            fixed,
            parameters,
            parameter_values,
            particles,
            heads,
            links,
            cells,
            electric,
            inputs,
            output,
            bin,
            cell,
            deposit,
            field,
            gather,
            push,
            solver,
            potential,
            capacity,
            count: 0,
            plasma_frequency_per_us: 0.0,
            last_cycles: 0,
            last_residual: 0.0,
        })
    }

    fn dispatch(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        pipeline: &wgpu::ComputePipeline,
        buffers: &[(u32, &wgpu::Buffer)],
        count: usize,
    ) {
        let entries: Vec<_> = buffers
            .iter()
            .map(|&(binding, b)| wgpu::BindGroupEntry {
                binding,
                resource: b.as_entire_binding(),
            })
            .collect();
        let bindings = self
            .gpu
            .device()
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &pipeline.get_bind_group_layout(0),
                entries: &entries,
            });
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &bindings, &[]);
        let groups = count.div_ceil(128);
        let y = groups.div_ceil(65535).max(1);
        pass.dispatch_workgroups(groups.div_ceil(y) as u32, y as u32, 1);
    }

    fn download(&self, buffer: &wgpu::Buffer, count: usize) -> Result<Vec<f64>> {
        let size = count as u64 * 8;
        let staging = self.gpu.device().create_buffer(&wgpu::BufferDescriptor {
            label: Some("PIC readback"),
            size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = self
            .gpu
            .device()
            .create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(buffer, 0, &staging, 0, size);
        self.gpu.queue().submit([encoder.finish()]);
        let (tx, rx) = mpsc::channel();
        staging.slice(..).map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        self.gpu
            .device()
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(Duration::from_secs(30)),
            })
            .map_err(|_| FlyError("GPU PIC readback timeout"))?;
        rx.recv_timeout(Duration::from_secs(30))
            .map_err(|_| FlyError("GPU PIC map timeout"))?
            .map_err(|_| FlyError("GPU PIC map failed"))?;
        let data = staging
            .slice(..)
            .get_mapped_range()
            .map_err(|_| FlyError("GPU PIC mapped range failed"))?;
        let values = bytemuck::cast_slice(&data).to_vec();
        drop(data);
        staging.unmap();
        Ok(values)
    }

    /// Deposit at synchronized force-time positions, solve physical Poisson,
    /// and differentiate the potential. The gathered force remains on device.
    pub fn evaluate(
        &mut self,
        positions: &[Vector],
        charges: &[f64],
        ratios: &[f64],
    ) -> Result<()> {
        if positions.is_empty()
            || positions.len() > self.capacity
            || positions.len() != charges.len()
            || positions.len() != ratios.len()
            || charges.iter().chain(ratios).any(|v| !v.is_finite())
        {
            return Err(FlyError("invalid GPU PIC inputs"));
        }
        let mut data = Vec::with_capacity(positions.len() * 8);
        for (&p, &q) in positions.iter().zip(charges) {
            let mut v: Vector = std::array::from_fn(|a| p[a] - self.origin[a]);
            if let Geometry::Cylindrical { radial_origin } = self.grid.geometry {
                v = [v[0].hypot(v[1]) - radial_origin, v[2], 0.0];
            }
            if (0..3).any(|a| {
                self.grid.dims[a] > 1
                    && (!v[a].is_finite()
                        || v[a] < 0.0
                        || v[a] > (self.grid.dims[a] - 1) as f64 * self.grid.spacing[a])
            }) {
                return Err(FlyError("live particle lies outside the GPU PIC mesh"));
            }
            data.extend_from_slice(&[p[0], p[1], p[2], q, 0.0, 0.0, 0.0, 0.0]);
        }
        self.count = positions.len();
        self.parameter_values[3] = self.count as f64;
        self.gpu.queue().write_buffer(
            &self.parameters,
            0,
            bytemuck::cast_slice(&self.parameter_values),
        );
        self.gpu
            .queue()
            .write_buffer(&self.particles, 0, bytemuck::cast_slice(&data));
        let mut encoder = self
            .gpu
            .device()
            .create_command_encoder(&Default::default());
        encoder.clear_buffer(&self.heads, 0, None);
        encoder.clear_buffer(&self.links, 0, None);
        self.dispatch(
            &mut encoder,
            &self.cell,
            &[
                (0, &self.parameters),
                (1, &self.particles),
                (9, &self.cells),
            ],
            self.count,
        );
        self.dispatch(
            &mut encoder,
            &self.bin,
            &[(0, &self.cells), (1, &self.heads), (2, &self.links)],
            self.count,
        );
        self.dispatch(
            &mut encoder,
            &self.deposit,
            &[
                (0, &self.parameters),
                (1, &self.particles),
                (2, &self.heads),
                (3, &self.links),
                (4, &self.fixed),
                (5, self.solver.normalized_source_buffer()),
            ],
            self.potential.len(),
        );
        self.gpu.queue().submit([encoder.finish()]);
        let report = self
            .solver
            .solve_device(&self.gpu, &mut self.potential)
            .map_err(|_| FlyError("GPU PIC physical Poisson failed"))?;
        self.last_cycles = report.iterations;
        self.last_residual = report.residual_history.last().unwrap().absolute;
        if !report.converged {
            return Err(FlyError("GPU PIC Poisson iteration budget exhausted"));
        }
        let max_ratio = ratios.iter().map(|v| v.abs()).fold(0.0, f64::max);
        self.plasma_frequency_per_us =
            (ELECTRIC_ACCELERATION * max_ratio * self.solver.max_source()).sqrt();
        let mut encoder = self
            .gpu
            .device()
            .create_command_encoder(&Default::default());
        self.dispatch(
            &mut encoder,
            &self.field,
            &[
                (0, &self.parameters),
                (4, &self.fixed),
                (5, self.solver.potential_buffer().unwrap()),
                (6, &self.electric),
            ],
            self.potential.len(),
        );
        self.gpu.queue().submit([encoder.finish()]);
        Ok(())
    }

    pub fn field(
        &mut self,
        positions: &[Vector],
        charges: &[f64],
        ratios: &[f64],
    ) -> Result<Vec<Vector>> {
        self.evaluate(positions, charges, ratios)?;
        let mut encoder = self
            .gpu
            .device()
            .create_command_encoder(&Default::default());
        self.dispatch(
            &mut encoder,
            &self.gather,
            &[
                (0, &self.parameters),
                (1, &self.particles),
                (6, &self.electric),
                (8, &self.output),
            ],
            self.count,
        );
        self.gpu.queue().submit([encoder.finish()]);
        Ok(self
            .download(&self.output, self.count * 8)?
            .as_chunks::<8>()
            .0
            .iter()
            .map(|v| [v[0], v[1], v[2]])
            .collect())
    }

    pub fn potentials(&self) -> &[f64] {
        &self.potential
    }

    pub(super) fn push(
        &self,
        field: &impl Field,
        particles: &[Particle],
        states: &[ParticleState],
        live: &[usize],
        h: f64,
        relativistic: bool,
    ) -> Result<Vec<ParticleState>> {
        let mut input = Vec::with_capacity(live.len() * 16);
        for &i in live {
            let s = states[i];
            let p = std::array::from_fn(|a| s.position_mm[a] + 0.5 * h * s.velocity_mm_per_us[a]);
            let sample = field.sample(p, s.time_us + 0.5 * h);
            input.extend_from_slice(&[
                s.position_mm[0],
                s.position_mm[1],
                s.position_mm[2],
                s.time_us,
                s.velocity_mm_per_us[0],
                s.velocity_mm_per_us[1],
                s.velocity_mm_per_us[2],
                particles[i].charge_e / particles[i].mass_amu,
                sample.electric_v_per_mm[0],
                sample.electric_v_per_mm[1],
                sample.electric_v_per_mm[2],
                h,
                sample.magnetic_gauss[0],
                sample.magnetic_gauss[1],
                sample.magnetic_gauss[2],
                f64::from(u8::from(relativistic)),
            ]);
        }
        self.gpu
            .queue()
            .write_buffer(&self.inputs, 0, bytemuck::cast_slice(&input));
        let mut encoder = self
            .gpu
            .device()
            .create_command_encoder(&Default::default());
        self.dispatch(
            &mut encoder,
            &self.push,
            &[
                (0, &self.parameters),
                (1, &self.particles),
                (6, &self.electric),
                (7, &self.inputs),
                (8, &self.output),
            ],
            self.count,
        );
        self.gpu.queue().submit([encoder.finish()]);
        Ok(self
            .download(&self.output, self.count * 8)?
            .as_chunks::<8>()
            .0
            .iter()
            .map(|v| ParticleState {
                position_mm: [v[0], v[1], v[2]],
                time_us: v[3],
                velocity_mm_per_us: [v[4], v[5], v[6]],
            })
            .collect())
    }

    pub(super) fn time_scale(&self, velocities: &[Vector]) -> f64 {
        let cell = self.grid.spacing.into_iter().fold(f64::INFINITY, f64::min);
        let speed = velocities
            .iter()
            .map(|&v| dot(v, v).sqrt())
            .fold(0.0, f64::max);
        (1.0 / self.plasma_frequency_per_us).min(cell / speed)
    }
}

use super::*;
use crate::Field;
use crate::physics::{
    collision::CollisionModel,
    magnetic::{Coil, MagneticField},
};

impl Gpu {
    /// Upload electric and magnetic reconstruction, analytic coils and optional
    /// neutral-gas transport. `seed` and absolute input indices key independent
    /// Philox streams; the internal dispatch size never changes an ion's stream.
    pub fn prepare_physics<'a>(
        &'a self,
        electric: &'a InstanceField,
        additional_bases: &[GridField],
        magnetic: &'a MagneticField,
        collision: Option<&'a CollisionModel>,
        seed: u64,
    ) -> GpuResult<PreparedField<'a>> {
        if !magnetic.vector_potentials.is_empty()
            || magnetic
                .arrays
                .iter()
                .chain(&magnetic.support)
                .any(|pa| pa.has_material_cells())
        {
            return Err(
                "vector potentials and magnetic material cell collisions require CPU physics"
                    .into(),
            );
        }
        if let Some(model) = collision {
            model.validate()?;
        }
        let mut fields = vec![self.prepare_internal(electric, additional_bases, false, false)?];
        let mut nodal_physics = false;
        for pa in magnetic.arrays.iter().chain(&magnetic.support) {
            let parts = pa.gpu_parts();
            let mut field = self.prepare_internal(parts.instance, &[], true, false)?;
            nodal_physics |= field.params.padding[1] != 0;
            // Metadata in otherwise unused vector padding travels with each
            // placement. Support volumes gate analytic B but never collide.
            field.params.magnetic = [
                parts.factor,
                parts.polarity_axis.map_or(0.0, |a| (a + 1) as f64),
                parts.sector.map_or(0.0, |(a, _, _)| (a + 1) as f64),
                0.0,
            ];
            if let Some((_, start, end)) = parts.sector {
                field.params.bounds[0][3] = start.to_radians();
                field.params.bounds[1][3] = end.to_radians();
            }
            fields.push(field);
        }
        let mut prepared = self.join_fields(fields, true)?;
        prepared.nodal_physics = nodal_physics;
        prepared.imported_maps = !magnetic.maps.is_empty();
        prepared.params.physics = [
            1,
            magnetic.arrays.len() as u32,
            magnetic
                .support
                .as_ref()
                .map_or(u32::MAX, |_| (1 + magnetic.arrays.len()) as u32),
            magnetic.coils.len() as u32,
        ];
        prepared.params.magnetic = [
            magnetic.uniform_gauss[0],
            magnetic.uniform_gauss[1],
            magnetic.uniform_gauss[2],
            dot(
                crate::cross(electric.axes[0], electric.axes[1]),
                electric.axes[2],
            )
            .signum(),
        ];
        // The root origin is otherwise unused: it restores absolute workbench
        // coordinates for angular apertures. Instance origins stay root-relative.
        prepared.params.origin[..3].copy_from_slice(&electric.origin_mm);
        let mut data = Vec::new();
        for coil in &magnetic.coils {
            let mut values = [0.0; 12];
            match coil {
                Coil::Segment {
                    first_mm,
                    last_mm,
                    current_a,
                } => {
                    for a in 0..3 {
                        values[a] = first_mm[a] - electric.origin_mm[a];
                        values[4 + a] = last_mm[a] - electric.origin_mm[a];
                    }
                    values[8] = *current_a;
                }
                Coil::Loop {
                    center_mm,
                    normal,
                    radius_mm,
                    current_a,
                } => {
                    for a in 0..3 {
                        values[a] = center_mm[a] - electric.origin_mm[a];
                        values[4 + a] = normal[a];
                    }
                    values[3] = 1.0;
                    values[8] = *current_a;
                    values[9] = *radius_mm;
                }
            }
            data.extend(values);
        }
        if let Some(model) = collision {
            let gas = match model {
                CollisionModel::Hs1(hs) => {
                    let sigma = (1.3806505e-23 * hs.gas.temperature_k
                        / (hs.gas.mass_amu * 1.6605402e-27))
                        .sqrt()
                        / 1000.0;
                    prepared.params.collision = [
                        1.0,
                        sigma,
                        hs.gas.pressure_pa * hs.cross_section_m2
                            / (1000.0 * 1.3806505e-23 * hs.gas.temperature_k),
                        hs.mean_free_path_mm.unwrap_or(0.0),
                    ];
                    prepared.params.collision_limits = [
                        2.0_f64.sqrt() * sigma,
                        hs.steps_per_mean_free_time,
                        f64::from(hs.gas.pressure_pa > 0.0),
                        0.0,
                    ];
                    &hs.gas
                }
                CollisionModel::Sds(sds) => {
                    prepared.params.collision = [2.0, 0.0, 0.0, 0.0];
                    prepared.params.collision_limits = [
                        0.0,
                        0.0,
                        f64::from(sds.diffusion && sds.gas.pressure_pa > 0.0),
                        data.len() as f64,
                    ];
                    data.extend_from_slice(sds.statistics.gpu_values());
                    &sds.gas
                }
            };
            for a in 0..3 {
                prepared.params.gas[a] = dot(electric.axes[a], gas.velocity_mm_per_us);
            }
            prepared.params.gas[3] = gas.mass_amu;
        }
        let map = super::coil_map::append(magnetic, electric, &mut data)?;
        let map = super::imported_map::append(magnetic, electric, &mut data, map)?;
        for imported in &magnetic.maps {
            if let Some(spacing) = imported.spatial_scale_mm() {
                prepared.params.spacing[3] = prepared.params.spacing[3].min(spacing);
            }
        }
        data.resize(data.len().max(4), 0.0);
        self.check_size(data.len() as u64 * if self.wide { 8 } else { 4 })?;
        let bytes = if self.wide {
            bytemuck::cast_slice(&data).to_vec()
        } else {
            bytemuck::cast_slice(&data.iter().map(|&v| v as f32).collect::<Vec<_>>()).to_vec()
        };
        prepared.physics_data = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("coils and SDS inverse distributions"),
                contents: &bytes,
                usage: wgpu::BufferUsages::STORAGE,
            });
        prepared.collision = collision;
        prepared.params.physics_padding = [seed as u32, (seed >> 32) as u32, 32, map];
        // A single electric instance also needs instance selection when B is
        // analytic or only a collision process is present.
        prepared.params.dims[3] = prepared.instance_count as u32;
        prepared.compile_expressions(&prepared.programs.clone())?;
        Ok(prepared)
    }
}

impl PreparedField<'_> {
    /// Additional analytic workbench B components in gauss, as WGSL scalar
    /// expressions using `position` (absolute workbench mm) and `time` (us).
    /// Native mode promotes expressions and math to f64. Rust field closures
    /// cannot be uploaded; callers provide the equivalent expressions here.
    pub fn set_magnetic_expressions(&mut self, expressions: [&str; 3]) -> GpuResult<()> {
        if !self.physics_enabled {
            return Err("analytic B requires prepare_physics".into());
        }
        self.magnetic_expressions = Some(expressions.map(|expression| {
            let mut source = String::new();
            let mut chars = expression.chars().peekable();
            while let Some(c) = chars.next() {
                if c.is_ascii_alphabetic() || c == '_' {
                    let mut word = String::from(c);
                    while chars
                        .peek()
                        .is_some_and(|c| c.is_ascii_alphanumeric() || *c == '_')
                    {
                        word.push(chars.next().unwrap());
                    }
                    let mut spaces = String::new();
                    while chars.peek().is_some_and(|c| c.is_ascii_whitespace()) {
                        spaces.push(chars.next().unwrap());
                    }
                    if chars.peek() == Some(&'(')
                        && matches!(
                            word.as_str(),
                            "sin" | "cos" | "exp" | "log" | "atan2" | "pow"
                        )
                    {
                        source.push_str(if word == "pow" { "voltage_pow" } else { &word });
                    } else {
                        source.push_str(&word);
                        source.push_str(&spaces);
                    }
                } else {
                    source.push(c);
                }
            }
            source
        }));
        self.compile_expressions(&self.programs.clone())
    }
}

pub(super) fn prepare_ion(
    ion: &mut Ion,
    particle: &Particle,
    collision: Option<&CollisionModel>,
    index: u64,
) -> GpuResult<()> {
    ion.offset[3] = particle.mass_amu;
    ion.padding1 = [index as u32, (index >> 32) as u32, 0, 0];
    if let Some(CollisionModel::Sds(sds)) = collision {
        if particle.charge_e == 0.0 {
            return Err("SDS mobility requires nonzero ion charge".into());
        }
        let transport = sds.transport(particle.mass_amu, particle.charge_e);
        ion.time = [
            transport.damping_per_us,
            transport.mean_free_path_mm,
            transport.mean_speed_mm_per_us,
            (particle.mass_amu / sds.gas.mass_amu).log10(),
        ];
    }
    Ok(())
}

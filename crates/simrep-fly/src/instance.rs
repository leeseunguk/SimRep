use crate::surface::SurfaceModel;
use crate::{Field, FieldSample, FlyError, GridField, SplatReason, Vector, VoltageExpression, dot};
use std::sync::Arc;

/// One placed electrostatic PA, with optional field-free workbench volume.
///
/// `axes` are the PA unit x/y/z directions in workbench coordinates. The PA
/// origin is placed at `origin_mm`; its physical spacing is multiplied by
/// `scale`. A plane is specified by `dot(normal, position_mm) = offset_mm`.
#[derive(Clone)]
pub struct InstanceField {
    pub(crate) grid: GridField,
    pub(crate) origin_mm: Vector,
    pub(crate) axes: [Vector; 3],
    pub(crate) scale: f64,
    pub(crate) bounds_mm: Option<[Vector; 2]>,
    pub(crate) plane: Option<(Vector, f64)>,
    pub(crate) voltage_bases: Vec<(GridField, VoltageExpression)>,
    surface: Option<Arc<SurfaceModel>>,
    surface_domain: Option<GridField>,
    adaptive: Option<Arc<crate::AdaptiveField>>,
}

/// Multiple non-overlapping PA instances with field-free drift between them.
/// The first containing instance supplies the field if array volumes overlap.
pub struct WorkbenchField {
    pub(crate) instances: Vec<InstanceField>,
    bounds_mm: [Vector; 2],
}

impl WorkbenchField {
    pub fn new(
        mut instances: Vec<InstanceField>,
        bounds_mm: [Vector; 2],
    ) -> Result<Self, FlyError> {
        if instances.is_empty()
            || (0..3).any(|a| {
                !bounds_mm[0][a].is_finite()
                    || !bounds_mm[1][a].is_finite()
                    || bounds_mm[0][a] >= bounds_mm[1][a]
            })
        {
            return Err(FlyError(
                "workbench needs instances and finite increasing bounds",
            ));
        }
        for instance in &mut instances {
            instance.bounds_mm = Some(bounds_mm);
        }
        Ok(Self {
            instances,
            bounds_mm,
        })
    }
}

impl Field for WorkbenchField {
    fn spatial_scale_mm(&self) -> Option<f64> {
        self.instances
            .iter()
            .filter_map(Field::spatial_scale_mm)
            .min_by(f64::total_cmp)
    }

    fn time_scale_us(&self) -> Option<f64> {
        self.instances
            .iter()
            .filter_map(Field::time_scale_us)
            .min_by(f64::total_cmp)
    }

    fn sample(&self, position: Vector, time: f64) -> FieldSample {
        self.instances
            .iter()
            .find(|f| f.grid.contains(f.local(position)))
            .map_or_else(FieldSample::default, |f| f.sample(position, time))
    }

    fn status(&self, position: Vector) -> Option<SplatReason> {
        if (0..3).any(|a| position[a] < self.bounds_mm[0][a] || position[a] > self.bounds_mm[1][a])
        {
            return Some(SplatReason::OutsideGrid);
        }
        self.instances
            .iter()
            .find(|f| f.grid.contains(f.local(position)))
            .and_then(|f| f.status(position))
    }

    fn intersection(&self, from: Vector, to: Vector) -> Option<(f64, SplatReason)> {
        self.instances
            .iter()
            .filter_map(|f| f.intersection(from, to))
            .min_by(|a, b| a.0.total_cmp(&b.0))
    }

    fn next_time_event(&self, time: f64) -> Option<f64> {
        self.instances
            .iter()
            .filter_map(|f| f.next_time_event(time))
            .min_by(f64::total_cmp)
    }

    fn launch_intersection(&self, from: Vector, to: Vector) -> Option<(f64, SplatReason)> {
        self.instances
            .iter()
            .filter_map(|f| f.launch_intersection(from, to))
            .min_by(|a, b| a.0.total_cmp(&b.0))
    }
}

impl InstanceField {
    pub fn new(
        grid: GridField,
        origin_mm: Vector,
        axes: [Vector; 3],
        scale: f64,
        bounds_mm: Option<[Vector; 2]>,
        plane: Option<(Vector, f64)>,
    ) -> Result<Self, FlyError> {
        if !scale.is_finite()
            || scale <= 0.0
            || origin_mm
                .iter()
                .chain(axes.iter().flatten())
                .any(|v| !v.is_finite())
            || (0..3)
                .any(|i| (0..3).any(|j| (dot(axes[i], axes[j]) - f64::from(i == j)).abs() > 1e-10))
            || bounds_mm.is_some_and(|b| {
                (0..3).any(|a| !b[0][a].is_finite() || !b[1][a].is_finite() || b[0][a] >= b[1][a])
            })
            || plane.is_some_and(|(n, d)| {
                !d.is_finite() || n.iter().any(|v| !v.is_finite()) || dot(n, n) == 0.0
            })
        {
            return Err(FlyError(
                "invalid PA placement, workbench bounds or detector plane",
            ));
        }
        Ok(Self {
            grid,
            origin_mm,
            axes,
            scale,
            bounds_mm,
            plane,
            voltage_bases: Vec::new(),
            surface: None,
            surface_domain: None,
            adaptive: None,
        })
    }

    /// Replace PA conductor collisions and rebuild background/basis samples
    /// with a vacuum continuation at exact-surface metal knots.
    /// The electrode-free geometry grid retains PA exits, mirrors and cylinders;
    /// workbench bounds and detector events are still merged below.
    pub fn with_surface(mut self, surface: Arc<SurfaceModel>) -> Result<Self, FlyError> {
        surface.validate_array(self.grid.array())?;
        let mut domain = self.grid.array().clone();
        domain.electrodes.fill(false);
        domain.potentials.fill(0.0);
        self.surface_domain = Some(GridField::new(domain)?);
        self.grid = self.grid.with_surface(&surface)?;
        for (basis, _) in &mut self.voltage_bases {
            *basis = basis.clone().with_surface(&surface)?;
        }
        if let Some(adaptive) = self.adaptive.take() {
            self.adaptive = Some(Arc::new(
                Arc::unwrap_or_clone(adaptive).with_surface(self.grid.clone(), &surface)?,
            ));
        }
        self.surface = Some(surface);
        Ok(self)
    }

    /// Add exact collisions without changing the PA field or its electrode mask.
    pub fn with_collision_surface(mut self, surface: Arc<SurfaceModel>) -> Result<Self, FlyError> {
        if self.surface_domain.is_some() {
            return Err(FlyError(
                "collision-only surface requires a staircase field",
            ));
        }
        surface.validate_array(self.grid.array())?;
        self.surface = Some(surface);
        Ok(self)
    }

    /// Preparation counts summed over the background and unit-voltage bases.
    pub fn ghost_fill_report(&self) -> Option<crate::GhostFillReport> {
        let mut report = if let Some(adaptive) = &self.adaptive {
            adaptive.ghost_fill_report()?
        } else {
            self.grid.ghost_fill_report()?.clone()
        };
        for (basis, _) in &self.voltage_bases {
            if let Some(basis_report) = basis.ghost_fill_report() {
                report.merge(basis_report);
            }
        }
        Some(report)
    }

    /// Add unit-voltage bases to the fixed background, sharing its placement.
    pub fn with_voltage_expressions(
        mut self,
        mut bases: Vec<(GridField, VoltageExpression)>,
    ) -> Result<Self, FlyError> {
        if bases.iter().any(|(grid, _)| !self.grid.compatible(grid)) {
            return Err(FlyError("voltage bases must share the instance geometry"));
        }
        if let Some(surface) = &self.surface
            && self.surface_domain.is_some()
        {
            for (basis, _) in &mut bases {
                *basis = basis.clone().with_surface(surface)?;
            }
        }
        self.voltage_bases = bases;
        Ok(self)
    }

    pub fn with_adaptive(mut self, field: crate::AdaptiveField) -> Self {
        let field = if let Some(surface) = &self.surface
            && self.surface_domain.is_some()
        {
            // AdaptiveField owns validated GridFields; patch domains are made
            // from those same grids, so this does not introduce new validation.
            field
                .with_surface(self.grid.clone(), surface)
                .expect("validated adaptive grids accept surface reconstruction")
        } else {
            field
        };
        self.adaptive = Some(Arc::new(field));
        self
    }

    pub(crate) fn local(&self, position: Vector) -> Vector {
        let p = std::array::from_fn(|a| (position[a] - self.origin_mm[a]) / self.scale);
        self.axes.map(|axis| dot(axis, p))
    }
}

impl Field for InstanceField {
    fn spatial_scale_mm(&self) -> Option<f64> {
        self.adaptive
            .as_ref()
            .map_or_else(
                || self.grid.spatial_scale_mm(),
                |field| field.spatial_scale_mm(),
            )
            .map(|h| h * self.scale)
    }

    fn time_scale_us(&self) -> Option<f64> {
        self.voltage_bases
            .iter()
            .filter_map(|(_, e)| e.time_scale_us())
            .min_by(f64::total_cmp)
    }

    fn sample(&self, position_mm: Vector, time_us: f64) -> FieldSample {
        let p = self.local(position_mm);
        if self.bounds_mm.is_some() && !self.grid.contains(p) {
            return FieldSample::default();
        }
        let mut sample = self.adaptive.as_ref().map_or_else(
            || self.grid.sample(p, time_us),
            |field| field.sample(p, time_us),
        );
        for (grid, expression) in &self.voltage_bases {
            let factor = expression.evaluate(time_us);
            let basis = grid.sample(p, time_us);
            sample.potential_v += factor * basis.potential_v;
            for a in 0..3 {
                sample.electric_v_per_mm[a] += factor * basis.electric_v_per_mm[a];
            }
        }
        FieldSample {
            potential_v: sample.potential_v,
            electric_v_per_mm: std::array::from_fn(|a| {
                (0..3)
                    .map(|i| self.axes[i][a] * sample.electric_v_per_mm[i])
                    .sum::<f64>()
                    / self.scale
            }),
            magnetic_gauss: [0.0; 3],
        }
    }

    fn next_time_event(&self, time_us: f64) -> Option<f64> {
        self.voltage_bases
            .iter()
            .filter_map(|(_, e)| e.next_event(time_us))
            .min_by(f64::total_cmp)
    }

    fn status(&self, position_mm: Vector) -> Option<SplatReason> {
        if self
            .bounds_mm
            .is_some_and(|b| (0..3).any(|a| position_mm[a] < b[0][a] || position_mm[a] > b[1][a]))
        {
            return Some(SplatReason::OutsideGrid);
        }
        let grid = self.surface_domain.as_ref().unwrap_or(&self.grid);
        let local = self.local(position_mm);
        let boundary = grid
            .status(local)
            .filter(|s| self.bounds_mm.is_none() || *s != SplatReason::OutsideGrid);
        boundary.or_else(|| {
            self.surface.as_ref().and_then(|s| {
                s.membership(s.source_point(local))
                    .map(|number| SplatReason::SurfaceElectrode { number })
            })
        })
    }

    fn intersection(&self, from: Vector, to: Vector) -> Option<(f64, SplatReason)> {
        self.intersection_impl(from, to, false)
    }

    fn launch_intersection(&self, from: Vector, to: Vector) -> Option<(f64, SplatReason)> {
        self.intersection_impl(from, to, true)
    }
}

impl InstanceField {
    fn intersection_impl(
        &self,
        from: Vector,
        to: Vector,
        launch: bool,
    ) -> Option<(f64, SplatReason)> {
        let grid = self.surface_domain.as_ref().unwrap_or(&self.grid);
        let local_from = self.local(from);
        let local_to = self.local(to);
        let mut first = if self.surface_domain.is_some()
            && (self.bounds_mm.is_some() || grid.contains(local_from) && grid.contains(local_to))
        {
            // The surface domain has no electrodes. With workbench bounds
            // the PA cannot splat; otherwise a segment inside this convex
            // domain cannot cross an edge. Avoid allocating/traversing cells.
            None
        } else {
            grid.intersect(local_from, local_to, self.bounds_mm.is_none(), launch)
        };
        let mut consider = |fraction, reason| {
            if (0.0..=1.0).contains(&fraction) && first.is_none_or(|(t, _)| fraction < t) {
                first = Some((fraction, reason));
            }
        };
        if let Some(surface) = &self.surface
            && let Some((fraction, number)) = surface.segment_hit(
                surface.source_point(local_from),
                surface.source_point(local_to),
                launch,
            )
        {
            consider(fraction, SplatReason::SurfaceElectrode { number });
        }
        if let Some(bounds) = self.bounds_mm {
            for a in 0..3 {
                if to[a] < bounds[0][a] || to[a] > bounds[1][a] {
                    let edge = if to[a] < bounds[0][a] {
                        bounds[0][a]
                    } else {
                        bounds[1][a]
                    };
                    consider(
                        (edge - from[a]) / (to[a] - from[a]),
                        SplatReason::OutsideGrid,
                    );
                }
            }
        }
        if let Some((normal, offset)) = self.plane {
            let distance = dot(normal, to) - dot(normal, from);
            if distance != 0.0 {
                consider(
                    (offset - dot(normal, from)) / distance,
                    SplatReason::StopPlane,
                );
            }
        }
        first
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FlightOptions, Particle, fly};
    use simrep_core::PotentialArray;

    const IDENTITY: [Vector; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

    #[test]
    fn collision_surface_preserves_grid_field_and_staircase_hits() {
        let mut array = PotentialArray::new(9, 5, 1);
        array.electrodes[6 + 2 * 9] = true;
        let grid = GridField::new(array)
            .unwrap()
            .with_nearest_node_collisions();
        let base = InstanceField::new(grid, [0.0; 3], IDENTITY, 1.0, None, None).unwrap();
        let surface = SurfaceModel::from_sdfs(
            vec![(4, Arc::new(|p: Vector| (p[0] - 3.0).abs() - 0.25))],
            1e-10,
        )
        .unwrap();
        let hybrid = base
            .clone()
            .with_collision_surface(Arc::new(surface))
            .unwrap();
        assert_eq!(
            base.sample([1.0, 2.0, 0.0], 0.0),
            hybrid.sample([1.0, 2.0, 0.0], 0.0)
        );
        assert!(matches!(
            hybrid
                .intersection([1.0, 2.0, 0.0], [8.0, 2.0, 0.0])
                .unwrap()
                .1,
            SplatReason::SurfaceElectrode { number: 4 }
        ));
        assert!(matches!(
            hybrid
                .intersection([4.0, 2.0, 0.0], [8.0, 2.0, 0.0])
                .unwrap()
                .1,
            SplatReason::Electrode { .. }
        ));
    }

    #[test]
    fn scaled_rotated_field_and_field_free_workbench() {
        let mut a = PotentialArray::new(5, 5, 1);
        for y in 0..5 {
            for x in 0..5 {
                a.potentials[y * 5 + x] = -(x as f64);
            }
        }
        let field = InstanceField::new(
            GridField::new(a).unwrap(),
            [10.0, 20.0, 0.0],
            [[0.0, 1.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
            2.0,
            Some([[-100.0; 3], [100.0; 3]]),
            None,
        )
        .unwrap();
        let e = field.sample([6.0, 24.0, 0.0], 0.0).electric_v_per_mm;
        assert!(e[0].abs() < 1e-12 && (e[1] - 0.5).abs() < 1e-12);
        assert_eq!(field.sample([50.0, 50.0, 0.0], 0.0), FieldSample::default());
        assert_eq!(field.status([50.0, 50.0, 0.0]), None);
    }

    #[test]
    fn entry_catches_electrode_and_drift_reaches_oblique_plane() {
        let mut a = PotentialArray::new(5, 5, 1);
        a.electrodes[2 + 2 * 5] = true;
        let field = InstanceField::new(
            GridField::new(a).unwrap().with_nearest_node_collisions(),
            [0.0; 3],
            IDENTITY,
            1.0,
            Some([[-10.0; 3], [10.0; 3]]),
            None,
        )
        .unwrap();
        let hit = field
            .intersection([-2.0, 2.0, 0.0], [5.0, 2.0, 0.0])
            .unwrap();
        assert!((hit.0 - 0.5).abs() < 1e-12);
        assert!(matches!(hit.1, SplatReason::Electrode { index: 12 }));
        let field = InstanceField::new(
            GridField::new(PotentialArray::new(5, 5, 1)).unwrap(),
            [0.0; 3],
            IDENTITY,
            1.0,
            Some([[-10.0; 3], [10.0; 3]]),
            Some(([1.0, 1.0, 0.0], 8.0)),
        )
        .unwrap();
        let p = Particle::new(1.0, 0.0, [-1.0, 2.0, 0.0], [1.0, 0.0, 0.0], 0.0).unwrap();
        let r = fly(&field, &p, &FlightOptions::default()).unwrap();
        assert_eq!(r.splat_reason, SplatReason::StopPlane);
        assert!((r.tof_us - 7.0).abs() < 1e-10);
        assert!((r.final_state.position_mm[0] - 6.0).abs() < 1e-10);
    }
}

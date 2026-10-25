//! Scalar magnetic PAs and thin-wire Biot-Savart fields, in mm, A and gauss.

use std::f64::consts::PI;

use simrep_core::{FieldType, PotentialArray};

use crate::{
    Field, FieldSample, FlyError, GridField, InstanceField, Reconstruction, SplatReason, Vector,
    cross, dot,
};

mod coil_map;
mod imported;
pub(crate) use coil_map::CoilMapPlan;
pub use coil_map::{CoilMapCalibration, PreparedMagneticField};
pub use imported::{DivergenceReport, ImportedMagneticMap, MapPlacement, OutsideMap};
mod vector_potential;
pub use vector_potential::VectorPotential;

/// A placed magnetic scalar PA. `ng` is the PA header's magnetic scale.
/// SIMION scales coordinates, but not magnetic field strength, on placement.
pub struct MagneticPa {
    instance: InstanceField,
    factor: f64,
    polarity_axis: Option<usize>,
    sector: Option<(usize, f64, f64)>,
}

pub(crate) struct MagneticPaView<'a> {
    pub instance: &'a InstanceField,
    pub factor: f64,
    pub polarity_axis: Option<usize>,
    pub sector: Option<(usize, f64, f64)>,
}

impl MagneticPa {
    /// Mark permeability/magnetization cells as solid for ions. In the
    /// remaining vacuum, B=-ng grad p is the complete physical B field.
    pub fn with_material_cells(mut self, cells: Vec<bool>) -> Result<Self, FlyError> {
        self.instance.grid = self.instance.grid.with_solid_cells(cells)?;
        Ok(self)
    }

    pub fn has_material_cells(&self) -> bool {
        self.instance.grid.has_solid_cells()
    }

    /// Read-only reconstruction and placement for native GPU upload.
    pub(crate) fn gpu_parts(&self) -> MagneticPaView<'_> {
        MagneticPaView {
            instance: &self.instance,
            factor: self.factor,
            polarity_axis: self.polarity_axis,
            sector: self.sector,
        }
    }

    pub fn new(array: PotentialArray, ng: f64) -> Result<Self, FlyError> {
        Self::placed(
            array,
            ng,
            [0.0; 3],
            [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            1.0,
            None,
        )
    }

    pub fn placed(
        mut array: PotentialArray,
        ng: f64,
        origin_mm: Vector,
        axes: [Vector; 3],
        scale: f64,
        bounds_mm: Option<[Vector; 2]>,
    ) -> Result<Self, FlyError> {
        if array.field_type != FieldType::Magnetic || !ng.is_finite() || ng <= 0.0 {
            return Err(FlyError(
                "magnetic PA requires magnetic metadata and positive ng",
            ));
        }
        // Scalar interpolation, cylindrical expansion and potential mirrors
        // are identical to their electric counterparts. Do not reinterpret
        // ordinary magnetic mirrors as anti-mirrors.
        array.field_type = FieldType::Electrostatic;
        let grid = GridField::with_reconstruction(array, Reconstruction::BoundaryAware)?
            .with_node_cell_collisions();
        Ok(Self {
            instance: InstanceField::new(grid, origin_mm, axes, scale, bounds_mm, None)?,
            factor: ng * scale,
            polarity_axis: None,
            sector: None,
        })
    }

    /// Reproduce a model's explicit mfield_adjust polarity switch, such as
    /// sector_3dp.lua. This is separate from the PA's ordinary scalar mirrors.
    pub fn with_polarity_flip_axis(mut self, axis: usize) -> Result<Self, FlyError> {
        if axis > 2 {
            return Err(FlyError("polarity axis must be 0, 1 or 2"));
        }
        self.polarity_axis = Some(axis);
        Ok(self)
    }

    pub fn contains(&self, position_mm: Vector) -> bool {
        self.instance
            .grid
            .contains(self.instance.local(position_mm))
    }

    /// Artificial angular field aperture, as in sector_2dc.lua.
    /// Around X, angle = atan2(y,z) in degrees modulo 360.
    /// Coordinates are workbench coordinates about its origin.
    pub fn with_angular_sector(
        mut self,
        axis: usize,
        start_deg: f64,
        end_deg: f64,
    ) -> Result<Self, FlyError> {
        if axis > 2
            || !start_deg.is_finite()
            || !end_deg.is_finite()
            || !(0.0..=360.0).contains(&start_deg)
            || !(start_deg..=360.0).contains(&end_deg)
        {
            return Err(FlyError(
                "sector requires axis 0..2 and 0<=start<=end<=360 degrees",
            ));
        }
        self.sector = Some((axis, start_deg, end_deg));
        Ok(self)
    }

    fn sample_local(&self, p: Vector, local: Vector, t: f64) -> FieldSample {
        if let Some((axis, start, end)) = self.sector {
            let angle = p[(axis + 1) % 3]
                .atan2(p[(axis + 2) % 3])
                .to_degrees()
                .rem_euclid(360.0);
            if angle < start || angle > end {
                return FieldSample::default();
            }
        }
        let sign = if self.polarity_axis.is_some_and(|a| local[a] < 0.0) {
            -1.0
        } else {
            1.0
        };
        let sample = self.instance.grid.sample(local, t);
        FieldSample {
            magnetic_gauss: std::array::from_fn(|a| {
                let e = (0..3)
                    .map(|i| self.instance.axes[i][a] * sample.electric_v_per_mm[i])
                    .sum::<f64>()
                    / self.instance.scale;
                e * self.factor * sign
            }),
            ..Default::default()
        }
    }
}

impl Field for MagneticPa {
    fn spatial_scale_mm(&self) -> Option<f64> {
        self.instance.spatial_scale_mm()
    }

    fn sample(&self, p: Vector, t: f64) -> FieldSample {
        let local = self.instance.local(p);
        if self.instance.bounds_mm.is_some() && !self.instance.grid.contains(local) {
            return FieldSample::default();
        }
        self.sample_local(p, local, t)
    }
    fn status(&self, p: Vector) -> Option<SplatReason> {
        self.instance.status(p)
    }
    fn intersection(&self, a: Vector, b: Vector) -> Option<(f64, SplatReason)> {
        self.instance.intersection(a, b)
    }
    fn launch_intersection(&self, a: Vector, b: Vector) -> Option<(f64, SplatReason)> {
        self.instance.launch_intersection(a, b)
    }
}

/// Thin filament fields. Points on a wire are singular (nonfinite field);
/// these are not finite-radius conductors or a magnetic material model.
#[derive(Clone, Debug)]
pub enum Coil {
    Segment {
        first_mm: Vector,
        last_mm: Vector,
        current_a: f64,
    },
    Loop {
        center_mm: Vector,
        normal: Vector,
        radius_mm: f64,
        current_a: f64,
    },
}

impl Coil {
    pub fn segment(first_mm: Vector, last_mm: Vector, current_a: f64) -> Result<Self, FlyError> {
        if first_mm.iter().chain(&last_mm).any(|x| !x.is_finite())
            || !current_a.is_finite()
            || first_mm == last_mm
        {
            return Err(FlyError(
                "wire endpoints must be finite and distinct; current finite",
            ));
        }
        Ok(Self::Segment {
            first_mm,
            last_mm,
            current_a,
        })
    }

    pub fn hoop(
        center_mm: Vector,
        normal: Vector,
        radius_mm: f64,
        current_a: f64,
    ) -> Result<Self, FlyError> {
        let length = dot(normal, normal).sqrt();
        if center_mm.iter().chain(&normal).any(|x| !x.is_finite())
            || !current_a.is_finite()
            || !radius_mm.is_finite()
            || radius_mm <= 0.0
            || !length.is_finite()
            || length == 0.0
        {
            return Err(FlyError(
                "loop requires finite center, nonzero normal and positive radius",
            ));
        }
        Ok(Self::Loop {
            center_mm,
            normal: normal.map(|n| n / length),
            radius_mm,
            current_a,
        })
    }

    pub fn field_gauss(&self, p: Vector) -> Vector {
        match *self {
            Self::Segment {
                first_mm,
                last_mm,
                current_a,
            } => {
                if current_a == 0.0 {
                    return [0.0; 3];
                }
                let direction = std::array::from_fn(|a| last_mm[a] - first_mm[a]);
                let length = dot(direction, direction).sqrt();
                let u = direction.map(|d| d / length);
                let r: Vector = std::array::from_fn(|a| p[a] - first_mm[a]);
                let along = dot(r, u);
                let radial: Vector = std::array::from_fn(|a| r[a] - along * u[a]);
                let r2 = dot(radial, radial);
                if r2 == 0.0 {
                    return if along < 0.0 || along > length {
                        [0.0; 3]
                    } else {
                        [f64::NAN; 3]
                    };
                }
                let factor = current_a / r2
                    * (along / (r2 + along * along).sqrt()
                        - (along - length) / (r2 + (along - length).powi(2)).sqrt());
                cross(u, radial).map(|b| b * factor)
            }
            Self::Loop {
                center_mm,
                normal,
                radius_mm: radius,
                current_a,
            } => {
                if current_a == 0.0 {
                    return [0.0; 3];
                }
                let r: Vector = std::array::from_fn(|a| p[a] - center_mm[a]);
                let z = dot(r, normal);
                let radial: Vector = std::array::from_fn(|a| r[a] - z * normal[a]);
                let rho = dot(radial, radial).sqrt();
                if rho < radius * 1e-8 {
                    let axial =
                        2.0 * PI * current_a * radius.powi(2) / (radius.powi(2) + z * z).powf(1.5);
                    // Leading radial limit avoids cancellation near the axis.
                    let radial_factor = 3.0 * PI * current_a * radius.powi(2) * z
                        / (radius.powi(2) + z * z).powf(2.5);
                    return std::array::from_fn(|a| axial * normal[a] + radial_factor * radial[a]);
                }
                let d = (radius + rho).powi(2) + z * z;
                let q = (radius - rho).powi(2) + z * z;
                if q == 0.0 {
                    return [f64::NAN; 3];
                }
                let (k, e) = elliptic_ke(4.0 * radius * rho / d);
                let factor = 2.0 * current_a / d.sqrt();
                let axial = factor * (k + (radius * radius - rho * rho - z * z) * e / q);
                let radial_factor =
                    factor * z / rho.powi(2) * (-k + (radius * radius + rho * rho + z * z) * e / q);
                std::array::from_fn(|a| axial * normal[a] + radial_factor * radial[a])
            }
        }
    }
}

// Complete elliptic integrals, parameter m=k^2; AGM converges quadratically.
fn elliptic_ke(m: f64) -> (f64, f64) {
    let mut a = 1.0;
    let mut b = (1.0 - m).sqrt();
    let mut sum = 0.5 * m;
    let mut weight = 1.0;
    for _ in 0..32 {
        let c = 0.5 * (a - b);
        sum += weight * c * c;
        let next = 0.5 * (a + b);
        b = (a * b).sqrt();
        a = next;
        weight *= 2.0;
        if c.abs() < 2e-16 * a {
            break;
        }
    }
    let k = PI / (2.0 * a);
    (k, k * (1.0 - sum))
}

/// Constant B plus current segments/loops and placed magnetic PAs.
/// As in a SIMION workbench, the first containing magnetic PA wins overlaps.
#[derive(Default)]
pub struct MagneticField {
    pub uniform_gauss: Vector,
    pub coils: Vec<Coil>,
    pub arrays: Vec<MagneticPa>,
    /// Imported vector maps superpose with every other magnetic source.
    pub maps: Vec<ImportedMagneticMap>,
    /// CPU curl-A sources. These add to each other and to scalar/coil sources.
    pub vector_potentials: Vec<VectorPotential>,
    /// Optional empty magnetic PA volume where analytic overrides apply,
    /// as in the SIMION MField examples. It does not create a workbench wall.
    pub support: Option<MagneticPa>,
}

impl MagneticField {
    /// Prepare calibrated C2 axial/Br-over-r maps for coaxial hoop groups.
    /// Preparation is separate from flight compute. The immutable borrow
    /// prevents coil edits from invalidating a prepared map. Calling `sample`
    /// on this unprepared field remains the exact Biot-Savart opt-out.
    pub fn prepare_coil_map(
        &self,
        bounds_mm: Option<[Vector; 2]>,
    ) -> Result<PreparedMagneticField<'_>, FlyError> {
        PreparedMagneticField::new(self, bounds_mm)
    }

    /// SIMION MField.solenoid_hoops: N hoops at the centers of N equal
    /// axial slices, not at the two endpoints.
    pub fn solenoid(
        first: Vector,
        last: Vector,
        radius: f64,
        current: f64,
        turns: usize,
    ) -> Result<Self, FlyError> {
        if turns == 0 || first == last {
            return Err(FlyError("solenoid needs distinct ends and nonzero turns"));
        }
        let normal = std::array::from_fn(|a| last[a] - first[a]);
        let coils = (0..turns)
            .map(|i| {
                let center =
                    std::array::from_fn(|a| first[a] + (i as f64 + 0.5) / turns as f64 * normal[a]);
                Coil::hoop(center, normal, radius, current)
            })
            .collect::<Result<_, _>>()?;
        Ok(Self {
            coils,
            ..Default::default()
        })
    }
}

fn first_hit(
    a: Option<(f64, SplatReason)>,
    b: Option<(f64, SplatReason)>,
) -> Option<(f64, SplatReason)> {
    match (a, b) {
        (Some(a), Some(b)) => Some(if a.0 <= b.0 { a } else { b }),
        (a, b) => a.or(b),
    }
}

impl Field for MagneticField {
    fn spatial_scale_mm(&self) -> Option<f64> {
        self.arrays
            .iter()
            .chain(&self.support)
            .filter_map(Field::spatial_scale_mm)
            .chain(self.maps.iter().filter_map(Field::spatial_scale_mm))
            .chain(
                self.vector_potentials
                    .iter()
                    .filter_map(Field::spatial_scale_mm),
            )
            .min_by(f64::total_cmp)
    }

    fn sample(&self, p: Vector, t: f64) -> FieldSample {
        self.sample_with_coils(p, t, None)
    }
    fn status(&self, p: Vector) -> Option<SplatReason> {
        if let Some(reason) = self.vector_potentials.iter().find_map(|f| f.status(p)) {
            return Some(reason);
        }
        match self.arrays.iter().find(|a| a.contains(p)) {
            Some(array) => array.status(p),
            None => self.arrays.first().and_then(|a| a.status(p)),
        }
        .or_else(|| self.maps.iter().find_map(|m| m.status(p)))
    }
    fn intersection(&self, a: Vector, b: Vector) -> Option<(f64, SplatReason)> {
        self.arrays
            .iter()
            .filter_map(|f| f.intersection(a, b))
            .chain(self.maps.iter().filter_map(|f| f.intersection(a, b)))
            .chain(
                self.vector_potentials
                    .iter()
                    .filter_map(|f| f.intersection(a, b)),
            )
            .reduce(|a, b| first_hit(Some(a), Some(b)).unwrap())
    }
}

impl MagneticField {
    fn sample_with_coils(&self, p: Vector, t: f64, mapped: Option<Vector>) -> FieldSample {
        let active = self.support.as_ref().is_none_or(|s| s.contains(p));
        let mut b = if active { self.uniform_gauss } else { [0.0; 3] };
        if active {
            if let Some(next) = mapped {
                for a in 0..3 {
                    b[a] += next[a];
                }
            } else {
                for coil in &self.coils {
                    let next = coil.field_gauss(p);
                    for a in 0..3 {
                        b[a] += next[a];
                    }
                }
            }
        }
        if let Some((array, local)) = self.arrays.iter().find_map(|array| {
            let local = array.instance.local(p);
            array
                .instance
                .grid
                .contains(local)
                .then_some((array, local))
        }) {
            let next = array.sample_local(p, local, t).magnetic_gauss;
            for a in 0..3 {
                b[a] += next[a];
            }
        }
        for potential in &self.vector_potentials {
            let next = potential.sample(p, t).magnetic_gauss;
            for a in 0..3 {
                b[a] += next[a];
            }
        }
        for map in &self.maps {
            let next = map.sample(p, t).magnetic_gauss;
            for a in 0..3 {
                b[a] += next[a];
            }
        }
        FieldSample {
            magnetic_gauss: b,
            ..Default::default()
        }
    }
}

/// Add electric and magnetic fields without losing either geometry or edges.
pub struct Electromagnetic<E, B> {
    pub electric: E,
    pub magnetic: B,
}

impl<E: Field, B: Field> Field for Electromagnetic<E, B> {
    fn spatial_scale_mm(&self) -> Option<f64> {
        self.electric
            .spatial_scale_mm()
            .into_iter()
            .chain(self.magnetic.spatial_scale_mm())
            .min_by(f64::total_cmp)
    }

    fn time_scale_us(&self) -> Option<f64> {
        self.electric
            .time_scale_us()
            .into_iter()
            .chain(self.magnetic.time_scale_us())
            .min_by(f64::total_cmp)
    }

    fn sample(&self, p: Vector, t: f64) -> FieldSample {
        let mut e = self.electric.sample(p, t);
        let b = self.magnetic.sample(p, t);
        e.potential_v += b.potential_v;
        for a in 0..3 {
            e.electric_v_per_mm[a] += b.electric_v_per_mm[a];
            e.magnetic_gauss[a] += b.magnetic_gauss[a];
        }
        e
    }
    fn status(&self, p: Vector) -> Option<SplatReason> {
        self.electric.status(p).or_else(|| self.magnetic.status(p))
    }
    fn intersection(&self, a: Vector, b: Vector) -> Option<(f64, SplatReason)> {
        first_hit(
            self.electric.intersection(a, b),
            self.magnetic.intersection(a, b),
        )
    }
    fn launch_intersection(&self, a: Vector, b: Vector) -> Option<(f64, SplatReason)> {
        first_hit(
            self.electric.launch_intersection(a, b),
            self.magnetic.launch_intersection(a, b),
        )
    }
    fn next_time_event(&self, t: f64) -> Option<f64> {
        self.electric
            .next_time_event(t)
            .into_iter()
            .chain(self.magnetic.next_time_event(t))
            .min_by(f64::total_cmp)
    }
}

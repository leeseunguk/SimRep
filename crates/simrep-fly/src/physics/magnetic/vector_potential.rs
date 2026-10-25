use simrep_core::{FieldType, PotentialArray, Symmetry};
use simrep_solve::magnetostatics::Formulation;

use crate::{
    Field, FieldSample, FlyError, GridField, InstanceField, Reconstruction, SplatReason, Vector,
    cross, dot,
};

/// CPU field reconstructed as the curl of one C2 potential, rather than
/// interpolating B components independently. PA coordinates are (x,y) for
/// Az, and (z,r) for psi=r*Aphi, with the physical symmetry axis along PA X.
pub struct VectorPotential {
    instance: InstanceField,
    formulation: Formulation,
    ng: f64,
    components: Option<[GridField; 2]>,
}

impl VectorPotential {
    pub fn new(array: PotentialArray, formulation: Formulation, ng: f64) -> Result<Self, FlyError> {
        Self::placed(
            array,
            formulation,
            ng,
            [0.0; 3],
            [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            1.0,
            None,
        )
    }

    pub fn placed(
        mut array: PotentialArray,
        formulation: Formulation,
        ng: f64,
        origin_mm: Vector,
        axes: [Vector; 3],
        scale: f64,
        bounds_mm: Option<[Vector; 2]>,
    ) -> Result<Self, FlyError> {
        if !ng.is_finite()
            || ng <= 0.0
            || array.nz != 1
            || array.nx.checked_mul(array.ny) != Some(array.potentials.len())
            || array.electrodes.len() != array.potentials.len()
            || !array.dy_mm.is_finite()
            || array.dy_mm <= 0.0
            || !matches!(
                (formulation, array.symmetry),
                (Formulation::PlanarAz, Symmetry::Planar)
                    | (Formulation::AxisymmetricPsi, Symmetry::Cylindrical)
            )
        {
            return Err(FlyError(
                "vector potential requires planar Az or axisymmetric psi, nz=1 and positive ng",
            ));
        }
        if formulation == Formulation::AxisymmetricPsi {
            if array.ny < 3 || array.anti_mirrors.y {
                return Err(FlyError(
                    "regular psi requires at least three radial nodes and no radial anti-mirror",
                ));
            }
            // Reconstruct the regular even function f=psi/r^2. Then
            // Bz=ng*(2f+r*df/dr), Br=-ng*r*df/dz, including r=0.
            // This removes cancellation and division by r near the axis.
            for x in 0..array.nx {
                if array.potentials[x] != 0.0 {
                    return Err(FlyError(
                        "axisymmetric psi must be zero on the regular axis",
                    ));
                }
                let one = array.potentials[array.nx + x] / array.dy_mm.powi(2);
                let two = array.potentials[2 * array.nx + x] / (2.0 * array.dy_mm).powi(2);
                array.potentials[x] = (4.0 * one - two) / 3.0;
                // A regularity constraint is not a physical conductor.
                array.electrodes[x] = false;
            }
            for y in 1..array.ny {
                let r2 = (y as f64 * array.dy_mm).powi(2);
                for x in 0..array.nx {
                    array.potentials[y * array.nx + x] /= r2;
                }
            }
        }
        array.field_type = FieldType::Electrostatic;
        let grid = GridField::with_reconstruction(array, Reconstruction::GlobalCubic)?;
        Ok(Self {
            instance: InstanceField::new(grid, origin_mm, axes, scale, bounds_mm, None)?,
            formulation,
            ng,
            components: None,
        })
    }

    /// Three C2 Cartesian components, each with its own scalar mirror parity
    /// and boundary mask. Component grids and physical spacing must agree.
    pub fn cartesian(
        arrays: [PotentialArray; 3],
        ng: f64,
        origin_mm: Vector,
        axes: [Vector; 3],
        scale: f64,
        bounds_mm: Option<[Vector; 2]>,
    ) -> Result<Self, FlyError> {
        let [mut ax, mut ay, mut az] = arrays;
        if !ng.is_finite()
            || ng <= 0.0
            || [&ax, &ay, &az]
                .iter()
                .any(|a| a.symmetry != Symmetry::Planar)
            || [&ay, &az].iter().any(|a| {
                [a.nx, a.ny, a.nz] != [ax.nx, ax.ny, ax.nz]
                    || [a.dx_mm, a.dy_mm, a.dz_mm] != [ax.dx_mm, ax.dy_mm, ax.dz_mm]
                    || a.mirrors != ax.mirrors
            })
        {
            return Err(FlyError(
                "Cartesian vector-potential components require matching planar grids, mirrors, spacing and positive ng",
            ));
        }
        ax.field_type = FieldType::Electrostatic;
        ay.field_type = FieldType::Electrostatic;
        az.field_type = FieldType::Electrostatic;
        let grid = |a| GridField::with_reconstruction(a, Reconstruction::GlobalCubic);
        Ok(Self {
            instance: InstanceField::new(grid(ax)?, origin_mm, axes, scale, bounds_mm, None)?,
            formulation: Formulation::CartesianA,
            ng,
            components: Some([grid(ay)?, grid(az)?]),
        })
    }

    pub fn with_material_cells(mut self, cells: Vec<bool>) -> Result<Self, FlyError> {
        self.instance.grid = self.instance.grid.with_solid_cells(cells)?;
        Ok(self)
    }
}

impl Field for VectorPotential {
    fn spatial_scale_mm(&self) -> Option<f64> {
        self.instance.spatial_scale_mm()
    }

    fn sample(&self, position_mm: Vector, time_us: f64) -> FieldSample {
        let p = self.instance.local(position_mm);
        if self.instance.bounds_mm.is_some() && !self.instance.grid.contains(p) {
            return FieldSample::default();
        }
        let s = self.instance.grid.sample(p, time_us);
        let e = s.electric_v_per_mm;
        let local = match self.formulation {
            Formulation::PlanarAz => [-self.ng * e[1], self.ng * e[0], 0.0],
            Formulation::AxisymmetricPsi => [
                self.ng * (2.0 * s.potential_v - p[1] * e[1] - p[2] * e[2]),
                self.ng * p[1] * e[0],
                self.ng * p[2] * e[0],
            ],
            Formulation::CartesianA => {
                let [ay, az] = self.components.as_ref().unwrap();
                let y = ay.sample(p, time_us).electric_v_per_mm;
                let z = az.sample(p, time_us).electric_v_per_mm;
                [
                    self.ng * (y[2] - z[1]),
                    self.ng * (z[0] - e[2]),
                    self.ng * (e[1] - y[0]),
                ]
            }
            Formulation::Scalar => unreachable!("constructor rejects scalar vector potentials"),
        };
        FieldSample {
            // As for magnetic PAs, placement changes lengths but not B.
            magnetic_gauss: std::array::from_fn(|a| {
                let handedness = dot(
                    cross(self.instance.axes[0], self.instance.axes[1]),
                    self.instance.axes[2],
                )
                .signum();
                handedness
                    * (0..3)
                        .map(|i| self.instance.axes[i][a] * local[i])
                        .sum::<f64>()
            }),
            ..Default::default()
        }
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

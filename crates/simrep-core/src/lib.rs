//! Grid, potential array, geometry and shared types.

/// Symmetry of a potential array.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Symmetry {
    /// Cartesian coordinates, including 2D arrays when `nz == 1`.
    #[default]
    Planar,
    /// Axial coordinate x and radial coordinate y; `nz == 1`.
    Cylindrical,
}

/// Reflection symmetry about the coordinate planes at the origin.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MirrorFlags {
    pub x: bool,
    pub y: bool,
    pub z: bool,
}

/// Physical meaning of the scalar potential.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum FieldType {
    /// Electric potential in volts.
    #[default]
    Electrostatic,
    /// Magnetic scalar potential; SIMION's ng scaling is file metadata.
    Magnetic,
}

/// A regular grid of scalar potentials and electrode membership.
///
/// Points are stored with x varying fastest: `(z * ny + y) * nx + x`.
/// Potentials are decoded values, without SIMION's electrode offset. In a
/// `.pa#` array, electrode potentials represent electrode numbers instead.
///
/// Both vectors must contain `nx * ny * nz` entries. Fields are public to allow
/// direct solver access; callers changing dimensions must resize both vectors.
#[derive(Clone, Debug, PartialEq)]
pub struct PotentialArray {
    pub nx: usize,
    pub ny: usize,
    pub nz: usize,
    pub symmetry: Symmetry,
    pub mirrors: MirrorFlags,
    /// Mirrored axes whose potentials change sign across the mirror plane.
    pub anti_mirrors: MirrorFlags,
    pub field_type: FieldType,
    /// Whether a field solver may refine this array (false for field maps).
    pub refinable: bool,
    /// Distance between neighboring x grid points, in millimeters.
    pub dx_mm: f64,
    /// Distance between neighboring y grid points, in millimeters.
    pub dy_mm: f64,
    /// Distance between neighboring z grid points, in millimeters.
    pub dz_mm: f64,
    pub potentials: Vec<f64>,
    pub electrodes: Vec<bool>,
}

impl PotentialArray {
    /// Create a planar, electrostatic array at zero potential with 1 mm cells.
    ///
    /// # Panics
    ///
    /// Panics if any dimension is zero or the point count overflows `usize`.
    pub fn new(nx: usize, ny: usize, nz: usize) -> Self {
        assert!(nx > 0 && ny > 0 && nz > 0, "dimensions must be positive");
        let len = nx
            .checked_mul(ny)
            .and_then(|n| n.checked_mul(nz))
            .expect("point count exceeds usize");
        Self {
            nx,
            ny,
            nz,
            symmetry: Symmetry::Planar,
            mirrors: MirrorFlags::default(),
            anti_mirrors: MirrorFlags::default(),
            field_type: FieldType::Electrostatic,
            refinable: true,
            dx_mm: 1.0,
            dy_mm: 1.0,
            dz_mm: 1.0,
            potentials: vec![0.0; len],
            electrodes: vec![false; len],
        }
    }

    /// Return the flat index, or `None` for coordinates outside this array.
    pub fn index(&self, x: usize, y: usize, z: usize) -> Option<usize> {
        if x < self.nx && y < self.ny && z < self.nz {
            Some((z * self.ny + y) * self.nx + x)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_x_fastest_indexing() {
        let pa = PotentialArray::new(3, 4, 2);
        assert_eq!(pa.potentials, vec![0.0; 24]);
        assert_eq!(pa.electrodes, vec![false; 24]);
        assert_eq!(pa.symmetry, Symmetry::Planar);
        assert_eq!(pa.field_type, FieldType::Electrostatic);
        assert_eq!((pa.dx_mm, pa.dy_mm, pa.dz_mm), (1.0, 1.0, 1.0));
        assert_eq!(pa.index(2, 3, 1), Some(23));
        assert_eq!(pa.index(0, 1, 0), Some(3));
        assert_eq!(pa.index(0, 0, 1), Some(12));
        assert_eq!(pa.index(3, 0, 0), None);
        assert_eq!(pa.index(0, 4, 0), None);
        assert_eq!(pa.index(0, 0, 2), None);
    }
}

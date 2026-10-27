//! CPU sampling of ghosted Cartesian patch PA families.
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use simrep_core::{PotentialArray, Symmetry};

use crate::{Field, FieldSample, FlyError, GridField, SplatReason, Vector};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdaptiveManifest {
    pub schema_version: u32,
    pub base: String,
    pub origin_mm: Vector,
    pub spacing_mm: Vector,
    pub reference_voltage: f64,
    pub electrode_voltages: BTreeMap<usize, f64>,
    pub patches: Vec<AdaptivePatchManifest>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdaptivePatchManifest {
    pub level: u32,
    pub factor: usize,
    pub bounds_mm: [Vector; 2],
    pub origin_mm: Vector,
    pub spacing_mm: Vector,
    pub file: String,
    pub ghost_layers: usize,
}

#[derive(Clone, Debug)]
struct Patch {
    bounds: [Vector; 2],
    offset: Vector,
    level: u32,
    field: GridField,
}

/// Geometry and collision ownership remain with the base; patches only
/// reconstruct the potential. Coordinates are relative to the base PA origin.
#[derive(Clone, Debug)]
pub struct AdaptiveField {
    base: GridField,
    patches: Vec<Patch>,
}

impl AdaptiveField {
    pub(crate) fn with_surface(
        mut self,
        base: GridField,
        surface: &crate::surface::SurfaceModel,
    ) -> Result<Self, FlyError> {
        self.base = base;
        for patch in &mut self.patches {
            let domain = surface.reconstruction_domain(patch.field.array(), patch.offset)?;
            patch.field = patch.field.clone().with_surface(&domain)?;
        }
        Ok(self)
    }

    pub(crate) fn ghost_fill_report(&self) -> Option<crate::GhostFillReport> {
        let mut report = self.base.ghost_fill_report()?.clone();
        for patch in &self.patches {
            if let Some(patch_report) = patch.field.ghost_fill_report() {
                report.merge(patch_report);
            }
        }
        Some(report)
    }

    pub fn new(
        base: GridField,
        manifest: &AdaptiveManifest,
        arrays: Vec<PotentialArray>,
    ) -> Result<Self, FlyError> {
        if manifest.schema_version != 1
            || base.array().symmetry != Symmetry::Planar
            || arrays.len() != manifest.patches.len()
            || arrays.is_empty()
            || [base.array().dx_mm, base.array().dy_mm, base.array().dz_mm] != manifest.spacing_mm
            || manifest.origin_mm.iter().any(|v| !v.is_finite())
            || manifest
                .spacing_mm
                .iter()
                .any(|v| !v.is_finite() || *v <= 0.0)
        {
            return Err(FlyError("invalid Cartesian adaptive manifest"));
        }
        let mut patches = Vec::new();
        let base_dims = [base.array().nx, base.array().ny, base.array().nz];
        for (description, array) in manifest.patches.iter().zip(arrays) {
            let dimensions = [array.nx, array.ny, array.nz];
            if ![2, 4].contains(&description.factor)
                || description.level == 0
                || description.ghost_layers < 2
                || array.symmetry != Symmetry::Planar
                || [array.dx_mm, array.dy_mm, array.dz_mm] != description.spacing_mm
                || (0..3).any(|a| {
                    let lo = description.bounds_mm[0][a];
                    let hi = description.bounds_mm[1][a];
                    let base_hi =
                        manifest.origin_mm[a] + (base_dims[a] - 1) as f64 * manifest.spacing_mm[a];
                    !lo.is_finite()
                        || !hi.is_finite()
                        || lo > hi
                        || !description.origin_mm[a].is_finite()
                        || lo < description.origin_mm[a] - 1e-9
                        || hi
                            > description.origin_mm[a]
                                + (dimensions[a] - 1) as f64 * description.spacing_mm[a]
                                + 1e-9
                        || lo < manifest.origin_mm[a] - 1e-9
                        || hi > base_hi + 1e-9
                        || (base_dims[a] == 1) != (dimensions[a] == 1)
                        || (base_dims[a] > 1
                            && (description.spacing_mm[a] * description.factor as f64
                                - manifest.spacing_mm[a])
                                .abs()
                                > 1e-10)
                })
            {
                return Err(FlyError("invalid adaptive patch bounds, spacing or ghosts"));
            }
            patches.push(Patch {
                bounds: description
                    .bounds_mm
                    .map(|p| std::array::from_fn(|a| p[a] - manifest.origin_mm[a])),
                offset: std::array::from_fn(|a| description.origin_mm[a] - manifest.origin_mm[a]),
                level: description.level,
                field: GridField::new(array)?,
            });
        }
        patches.sort_by_key(|p| std::cmp::Reverse(p.level));
        Ok(Self { base, patches })
    }
}

impl Field for AdaptiveField {
    fn sample(&self, position: Vector, time: f64) -> FieldSample {
        let (q, reflection, sign) = self.base.coordinates(position);
        if let Some(patch) = self.patches.iter().find(|p| {
            (0..3).all(|a| {
                let dims = [
                    self.base.array().nx,
                    self.base.array().ny,
                    self.base.array().nz,
                ];
                dims[a] == 1 || (q[a] >= p.bounds[0][a] && q[a] <= p.bounds[1][a])
            })
        }) {
            let mut sample = patch
                .field
                .sample(std::array::from_fn(|a| q[a] - patch.offset[a]), time);
            sample.potential_v *= sign;
            for (a, value) in sample.electric_v_per_mm.iter_mut().enumerate() {
                *value *= sign * reflection[a];
            }
            sample
        } else {
            self.base.sample(position, time)
        }
    }

    fn spatial_scale_mm(&self) -> Option<f64> {
        self.patches
            .iter()
            .filter_map(|p| p.field.spatial_scale_mm())
            .chain(self.base.spatial_scale_mm())
            .min_by(f64::total_cmp)
    }

    fn status(&self, position: Vector) -> Option<SplatReason> {
        self.base.status(position)
    }
    fn intersection(&self, from: Vector, to: Vector) -> Option<(f64, SplatReason)> {
        self.base.intersection(from, to)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn array(n: usize, spacing: f64, shift: f64, slope: f64) -> PotentialArray {
        let mut array = PotentialArray::new(n, if spacing < 1.0 { 9 } else { 5 }, 1);
        array.dx_mm = spacing;
        array.dy_mm = spacing;
        for i in 0..array.potentials.len() {
            array.potentials[i] = shift + slope * (i % n) as f64 * spacing;
        }
        array
    }

    #[test]
    fn core_selection_uses_ghosted_patch_but_geometry_remains_base_owned() {
        let mut base = array(9, 1.0, 0.0, 1.0);
        base.electrodes[0] = true;
        let base = GridField::new(base).unwrap();
        let manifest = AdaptiveManifest {
            schema_version: 1,
            base: "base.pa0".into(),
            origin_mm: [10.0, 0.0, 0.0],
            spacing_mm: [1.0; 3],
            reference_voltage: 10000.0,
            electrode_voltages: BTreeMap::new(),
            patches: vec![AdaptivePatchManifest {
                level: 1,
                factor: 2,
                bounds_mm: [[12.0, 0.0, 0.0], [16.0, 4.0, 0.0]],
                origin_mm: [11.0, 0.0, 0.0],
                spacing_mm: [0.5, 0.5, 1.0],
                file: "patch.pa0".into(),
                ghost_layers: 2,
            }],
        };
        let field =
            AdaptiveField::new(base.clone(), &manifest, vec![array(13, 0.5, 101.0, 2.0)]).unwrap();
        assert!((field.sample([4.0, 2.0, 0.0], 0.0).potential_v - 107.0).abs() < 1e-10);
        assert!((field.sample([1.5, 2.0, 0.0], 0.0).potential_v - 1.5).abs() < 1e-10);
        assert!((field.sample([2.0, 2.0, 0.0], 0.0).potential_v - 103.0).abs() < 1e-10);
        assert_eq!(field.status([0.0; 3]), base.status([0.0; 3]));
        assert_eq!(
            field.intersection([1.0, 0.0, 0.0], [-1.0, 0.0, 0.0]),
            base.intersection([1.0, 0.0, 0.0], [-1.0, 0.0, 0.0])
        );
        // Matching affine traces and composite-filled ghosts reproduce the
        // same potential and derivative on both sides of the interface.
        let continuous =
            AdaptiveField::new(base.clone(), &manifest, vec![array(13, 0.5, 1.0, 1.0)]).unwrap();
        for x in [2.0 - 1e-7, 2.0, 2.0 + 1e-7, 6.0 - 1e-7, 6.0 + 1e-7] {
            let s = continuous.sample([x, 2.0, 0.0], 0.0);
            assert!((s.potential_v - x).abs() < 1e-10);
            assert!((s.electric_v_per_mm[0] + 1.0).abs() < 1e-10);
        }
        assert_eq!(continuous.spatial_scale_mm(), Some(0.5));
        let mut mirror_base = base.array().clone();
        mirror_base.mirrors.x = true;
        let mirrored = AdaptiveField::new(
            GridField::new(mirror_base).unwrap(),
            &manifest,
            vec![array(13, 0.5, 1.0, 1.0)],
        )
        .unwrap()
        .sample([-4.0, 2.0, 0.0], 0.0);
        assert!((mirrored.potential_v - 4.0).abs() < 1e-10);
        assert!((mirrored.electric_v_per_mm[0] - 1.0).abs() < 1e-10);
    }
}

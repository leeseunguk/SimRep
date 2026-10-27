//! Continuous conductor membership and first accepted-segment intersections.
//! V3 retains source CSG or triangles; V1/V2 are approximate reconstructions.

use std::io::Read;
use std::sync::Arc;

use base64::{Engine, engine::general_purpose::STANDARD};
use bincode::Options;
use serde::{Deserialize, Serialize};
use simrep_core::{PotentialArray, Symmetry};
use simrep_geom::{SampledSdf, SignedDistance};

use crate::{FlyError, Vector};

/// Numbered source geometry, independent of the sampled fallback. Binary
/// payloads are base64 in JSON; their encodings are fixed by the v3 format.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SourceSurface {
    pub number: usize,
    pub geometry: SourceGeometry,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "encoding")]
pub enum SourceGeometry {
    #[serde(rename = "gem-csg-bincode1-base64")]
    Gem { data: String },
    #[serde(rename = "mesh-f64-u64-le-base64")]
    Mesh {
        data: String,
        /// CAD deflection; absent for triangle-file inputs.
        chordal_tolerance_mm: Option<f64>,
    },
}

impl SourceGeometry {
    pub fn gem(source: &simrep_gem::ElectrodeSdf) -> Result<Self, bincode::Error> {
        Ok(Self::Gem {
            data: STANDARD.encode(bincode::serialize(source)?),
        })
    }

    pub fn mesh(
        part: &simrep_mesh::Part,
        chordal_tolerance_mm: Option<f64>,
    ) -> std::io::Result<Self> {
        Ok(Self::Mesh {
            data: STANDARD.encode(part.source_bytes()?),
            chordal_tolerance_mm,
        })
    }

    fn electrode(
        &self,
        number: usize,
    ) -> Result<Electrode, Box<dyn std::error::Error + Send + Sync>> {
        match self {
            Self::Gem { data } => {
                let bytes = STANDARD.decode(data)?;
                let shape: simrep_gem::ElectrodeSdf = bincode::DefaultOptions::new()
                    .with_fixint_encoding()
                    .with_limit(bytes.len() as u64)
                    .reject_trailing_bytes()
                    .deserialize(&bytes)?;
                shape.validate()?;
                Ok(Electrode {
                    number,
                    shape: Arc::new(shape),
                    mesh: None,
                    lipschitz: 1.0,
                })
            }
            Self::Mesh {
                data,
                chordal_tolerance_mm,
            } => {
                if chordal_tolerance_mm.is_some_and(|t| !t.is_finite() || t <= 0.0) {
                    return Err(FlyError("invalid CAD chordal tolerance").into());
                }
                let part = simrep_mesh::Part::from_source_bytes(&STANDARD.decode(data)?)?;
                let mesh = Arc::new(simrep_mesh::Solid::new(&part)?);
                let shape = Arc::clone(&mesh);
                Ok(Electrode {
                    number,
                    shape: Arc::new(move |p| shape.signed_distance(p)),
                    mesh: Some(mesh),
                    lipschitz: 1.0,
                })
            }
        }
    }
}

/// Full x-fastest samples in mm, negative inside the numbered conductor.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SurfaceSamples {
    pub number: usize,
    pub dims: [usize; 3],
    pub origin_mm: Vector,
    pub spacing_mm: Vector,
    pub values: Vec<f64>,
}

impl SurfaceSamples {
    pub fn sample(
        number: usize,
        shape: &dyn SignedDistance,
        dims: [usize; 3],
        origin_mm: Vector,
        spacing_mm: Vector,
    ) -> Result<Self, FlyError> {
        let count = checked_grid(dims, origin_mm, spacing_mm)?;
        let values = (0..count)
            .map(|i| shape.distance(point(i, dims, origin_mm, spacing_mm)))
            .collect();
        SampledSdf::new(dims, origin_mm, spacing_mm, values)
            .map(|s| Self {
                number,
                dims,
                origin_mm,
                spacing_mm,
                values: s.values,
            })
            .map_err(FlyError)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SurfaceAccuracy {
    SourceGeometry,
    SampledSdf,
    ApproximateV1,
}

#[derive(Clone)]
struct Electrode {
    number: usize,
    shape: Arc<dyn SignedDistance>,
    lipschitz: f64,
    mesh: Option<Arc<simrep_mesh::Solid>>,
}

/// Geometry coordinates are the source's mm coordinates. Placed PA instances
/// add `origin_mm` after undoing the instance transform, so a nonzero source
/// grid origin is not applied twice.
pub struct SurfaceModel {
    electrodes: Vec<Electrode>,
    accuracy: SurfaceAccuracy,
    domain: Option<Domain>,
    tolerance_mm: f64,
    pub(crate) reconstruction_cuts: Option<ReconstructionCuts>,
}

pub(crate) struct ReconstructionCuts {
    pub(crate) nodes: Vec<Option<usize>>,
    pub(crate) links: Vec<LinkIntersections>,
}

#[derive(Clone)]
struct Domain {
    dims: [usize; 3],
    origin: Vector,
    spacing: Vector,
    cylindrical: bool,
    mirrors: [bool; 3],
}

#[derive(Deserialize)]
struct Sidecar {
    version: u32,
    dims: [usize; 3],
    origin_mm: Vector,
    spacing_mm: Vector,
    symmetry: String,
    #[serde(default)]
    mirrors: [bool; 3],
    #[serde(default)]
    electrode_numbers: Vec<usize>,
    #[serde(default)]
    sampled_sdfs: Vec<SurfaceSamples>,
    #[serde(default)]
    source_geometry: Vec<SourceSurface>,
    #[serde(default)]
    node_electrodes: Vec<Option<usize>>,
    node_electrode_runs: Option<Vec<(usize, usize, Option<usize>)>>,
    #[serde(default)]
    cut_fractions: Vec<[Option<f64>; 6]>,
    #[serde(default)]
    cut_electrodes: Vec<[Option<usize>; 6]>,
    link_intersections: Option<Vec<LinkIntersections>>,
}

#[derive(Deserialize)]
pub(crate) struct LinkIntersections {
    pub(crate) node: usize,
    pub(crate) fractions: [Option<f64>; 6],
    pub(crate) electrodes: [Option<usize>; 6],
}

impl SurfaceModel {
    /// Use source geometry directly, with its SignedDistance contract. No PA
    /// clipping, symmetry or sampling is imposed on this API.
    pub fn from_sdfs(
        electrodes: Vec<(usize, Arc<dyn SignedDistance>)>,
        tolerance_mm: f64,
    ) -> Result<Self, FlyError> {
        if electrodes.is_empty() || !tolerance_mm.is_finite() || tolerance_mm <= 0.0 {
            return Err(FlyError(
                "surface needs electrodes and a positive finite tolerance",
            ));
        }
        Ok(Self {
            electrodes: electrodes
                .into_iter()
                .map(|(number, shape)| Electrode {
                    number,
                    shape,
                    lipschitz: 1.0,
                    mesh: None,
                })
                .collect(),
            accuracy: SurfaceAccuracy::SourceGeometry,
            domain: None,
            tolerance_mm,
            reconstruction_cuts: None,
        })
    }

    /// Load the versioned cuts JSON. Unknown versions and malformed samples
    /// fail rather than silently reverting to staircase collisions.
    pub fn from_reader(
        reader: impl Read,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let data: Sidecar = serde_json::from_reader(reader)?;
        if !matches!(data.version, 1..=3) {
            return Err(FlyError("unsupported cuts sidecar version").into());
        }
        let count = checked_grid(data.dims, data.origin_mm, data.spacing_mm)?;
        let valid_nodes = if let Some(runs) = &data.node_electrode_runs {
            data.version == 3
                && data.node_electrodes.is_empty()
                && runs.first().is_some_and(|r| r.0 == 0)
                && runs.last().is_some_and(|r| r.1 == count)
                && runs.iter().all(|r| r.0 < r.1)
                && runs.windows(2).all(|pair| pair[0].1 == pair[1].0)
        } else {
            data.node_electrodes.len() == count
        };
        let valid_links =
            if let Some(links) = &data.link_intersections {
                data.version == 3
                    && data.cut_fractions.is_empty()
                    && data.cut_electrodes.is_empty()
                    && links.windows(2).all(|pair| pair[0].node < pair[1].node)
                    && links.iter().all(|link| {
                        link.node < count
                            && link.fractions.iter().any(Option::is_some)
                            && link.fractions.iter().zip(link.electrodes).all(|(t, n)| {
                                match (t, n) {
                                    (None, None) => true,
                                    (Some(t), Some(n)) => {
                                        t.is_finite()
                                            && (0.0..=1.0).contains(t)
                                            && data.electrode_numbers.contains(&n)
                                    }
                                    _ => false,
                                }
                            })
                    })
            } else {
                data.cut_fractions.len() == count && data.cut_electrodes.len() == count
            };
        if !matches!(data.symmetry.as_str(), "planar" | "cylindrical")
            || !valid_nodes
            || !valid_links
            || data.symmetry == "cylindrical"
                && (data.dims[2] != 1 || data.origin_mm[1] != 0.0 || data.origin_mm[2] != 0.0)
        {
            return Err(FlyError("invalid cuts sidecar geometry").into());
        }
        let h = (0..3)
            .filter(|&a| data.dims[a] > 1)
            .map(|a| data.spacing_mm[a])
            .fold(f64::INFINITY, f64::min);
        let accuracy = if data.version == 3 {
            SurfaceAccuracy::SourceGeometry
        } else if data.version == 1 {
            SurfaceAccuracy::ApproximateV1
        } else {
            SurfaceAccuracy::SampledSdf
        };
        let samples = if data.version == 1 {
            reconstruct_v1(&data, h)?
        } else if data.version == 3 && data.sampled_sdfs.is_empty() {
            Vec::new()
        } else {
            let mut numbers = data.electrode_numbers.clone();
            numbers.sort_unstable();
            numbers.dedup();
            if data.sampled_sdfs.is_empty()
                || data.electrode_numbers.len() != data.sampled_sdfs.len()
                || numbers.len() != data.electrode_numbers.len()
                || data
                    .node_electrodes
                    .iter()
                    .flatten()
                    .any(|n| !numbers.contains(n))
                || data
                    .sampled_sdfs
                    .iter()
                    .zip(&data.electrode_numbers)
                    .any(|(s, n)| {
                        s.number != *n
                            || s.origin_mm != data.origin_mm
                            || (0..3).any(|a| {
                                s.dims[a] == 0
                                    || ((s.dims[a] - 1) as f64 * s.spacing_mm[a]
                                        - (data.dims[a] - 1) as f64 * data.spacing_mm[a])
                                        .abs()
                                        > 1e-9 * h
                            })
                    })
            {
                return Err(FlyError("v2 SDF coverage or electrode mapping mismatch").into());
            }
            data.sampled_sdfs.clone()
        };
        let mut electrodes = samples
            .into_iter()
            .map(|s| {
                let shape = SampledSdf::new(s.dims, s.origin_mm, s.spacing_mm, s.values)
                    .map_err(FlyError)?;
                let lipschitz = sampled_lipschitz(&shape).max(1.0);
                Ok(Electrode {
                    number: s.number,
                    shape: Arc::new(shape),
                    lipschitz,
                    mesh: None,
                })
            })
            .collect::<Result<Vec<_>, FlyError>>()?;
        if data.version == 3 {
            let mut numbers: Vec<_> = data.source_geometry.iter().map(|s| s.number).collect();
            numbers.sort_unstable();
            numbers.dedup();
            let mut expected = data.electrode_numbers.clone();
            expected.sort_unstable();
            if numbers != expected
                || numbers.is_empty()
                || data
                    .node_electrode_runs
                    .iter()
                    .flatten()
                    .any(|r| r.2.is_some_and(|number| !numbers.contains(&number)))
                || data
                    .node_electrodes
                    .iter()
                    .flatten()
                    .any(|n| !numbers.contains(n))
            {
                return Err(FlyError("v3 source electrode mapping mismatch").into());
            }
            if data.symmetry == "cylindrical"
                && data
                    .source_geometry
                    .iter()
                    .any(|s| matches!(s.geometry, SourceGeometry::Mesh { .. }))
            {
                return Err(FlyError("triangle sources require a planar 3D domain").into());
            }
            electrodes = data
                .source_geometry
                .iter()
                .map(|s| s.geometry.electrode(s.number))
                .collect::<Result<Vec<_>, _>>()?;
        }
        if electrodes.is_empty() {
            return Err(FlyError("cuts sidecar contains no electrodes").into());
        }
        let nodes = if let Some(runs) = data.node_electrode_runs {
            runs.into_iter()
                .flat_map(|(lo, hi, number)| std::iter::repeat_n(number, hi - lo))
                .collect()
        } else {
            data.node_electrodes
        };
        let links = if let Some(links) = data.link_intersections {
            links
        } else {
            data.cut_fractions
                .into_iter()
                .zip(data.cut_electrodes)
                .enumerate()
                .filter_map(|(i, (fractions, numbers))| {
                    fractions
                        .iter()
                        .any(Option::is_some)
                        .then_some(LinkIntersections {
                            node: i,
                            fractions,
                            electrodes: numbers,
                        })
                })
                .collect()
        };
        Ok(Self {
            electrodes,
            accuracy,
            domain: Some(Domain {
                dims: data.dims,
                origin: data.origin_mm,
                spacing: data.spacing_mm,
                cylindrical: data.symmetry == "cylindrical",
                mirrors: data.mirrors,
            }),
            tolerance_mm: 1e-10 * h,
            reconstruction_cuts: Some(ReconstructionCuts { nodes, links }),
        })
    }

    pub(crate) fn electrode_numbers(&self) -> impl Iterator<Item = usize> + '_ {
        self.electrodes.iter().map(|e| e.number)
    }

    /// A refined patch has its own grid links, but the same source geometry.
    pub(crate) fn reconstruction_domain(
        &self,
        array: &PotentialArray,
        offset: Vector,
    ) -> Result<Self, FlyError> {
        Self {
            electrodes: self.electrodes.clone(),
            accuracy: self.accuracy,
            domain: None,
            tolerance_mm: self.tolerance_mm,
            reconstruction_cuts: None,
        }
        .with_pa_domain(array, self.source_point(offset))
    }

    pub fn accuracy(&self) -> SurfaceAccuracy {
        self.accuracy
    }

    /// Set root localization to the flight's source-coordinate position
    /// tolerance (workbench tolerance divided by instance scale).
    pub fn with_tolerance(mut self, tolerance_mm: f64) -> Result<Self, FlyError> {
        if !tolerance_mm.is_finite() || tolerance_mm <= 0.0 {
            return Err(FlyError("surface tolerance must be finite and positive"));
        }
        self.tolerance_mm = tolerance_mm;
        Ok(self)
    }

    /// Give in-memory source geometry the same source origin, finite PA extent,
    /// mirrors and cylindrical revolution used by a sidecar. The instance
    /// placement still specifies where PA node zero lies in the workbench.
    pub fn with_pa_domain(
        mut self,
        array: &PotentialArray,
        origin_mm: Vector,
    ) -> Result<Self, FlyError> {
        let dims = [array.nx, array.ny, array.nz];
        let spacing = [array.dx_mm, array.dy_mm, array.dz_mm];
        checked_grid(dims, origin_mm, spacing)?;
        let cylindrical = array.symmetry == Symmetry::Cylindrical;
        if cylindrical && (dims[2] != 1 || origin_mm[1] != 0.0 || origin_mm[2] != 0.0) {
            return Err(FlyError(
                "cylindrical surface domain requires nz=1 and zero radial/z origins",
            ));
        }
        self.domain = Some(Domain {
            dims,
            origin: origin_mm,
            spacing,
            cylindrical,
            mirrors: [array.mirrors.x, array.mirrors.y, array.mirrors.z],
        });
        Ok(self)
    }

    /// Number of the first containing conductor, including its surface.
    pub fn membership(&self, point_mm: Vector) -> Option<usize> {
        let (p, boundary) = self.coordinates(point_mm);
        if boundary > 0.0 {
            return None;
        }
        self.electrodes
            .iter()
            .find(|e| {
                e.mesh
                    .as_ref()
                    .map_or_else(|| e.shape.distance(p) <= 0.0, |m| m.contains(p))
            })
            .map(|e| e.number)
    }

    /// First conductor entry on a straight accepted-step segment. Conservative
    /// tracing also catches conductors between two vacuum endpoints; a sign
    /// bracket is localized with bisection by the shared solver root finder.
    pub fn first_hit(&self, from: Vector, to: Vector) -> Option<(f64, usize)> {
        self.segment_hit(from, to, false)
    }

    pub(crate) fn segment_hit(
        &self,
        from: Vector,
        to: Vector,
        launch: bool,
    ) -> Option<(f64, usize)> {
        if from == to {
            return (!launch)
                .then(|| self.membership(from))
                .flatten()
                .map(|n| (0.0, n));
        }
        let length = (0..3)
            .map(|a| (to[a] - from[a]).powi(2))
            .sum::<f64>()
            .sqrt();
        let (start_point, boundary) = self.coordinates(from);
        self.electrodes
            .iter()
            .filter_map(|e| {
                if let Some(mesh) = &e.mesh {
                    return self.mesh_hit(mesh, from, to, launch).map(|t| (t, e.number));
                }
                // The shared tracer makes the same conservative rejection,
                // but first evaluates its start point twice. Almost every
                // flight segment is far from every conductor.
                if e.shape.distance(start_point).max(boundary) / e.lipschitz > length {
                    return None;
                }
                let distance = |p| self.distance(e, p) / e.lipschitz;
                let start = if launch && distance(from) <= 0.0 {
                    let exit = |p| -distance(p);
                    let t = simrep_solve::cutcell::surface_fraction(
                        &exit,
                        from,
                        to,
                        self.tolerance_mm / e.lipschitz,
                    )?;
                    let length = (0..3)
                        .map(|a| (to[a] - from[a]).powi(2))
                        .sum::<f64>()
                        .sqrt();
                    let t = t + 4.0 * self.tolerance_mm / length;
                    if t >= 1.0 {
                        return None;
                    }
                    t
                } else {
                    0.0
                };
                simrep_solve::cutcell::surface_fraction(
                    &distance,
                    std::array::from_fn(|a| from[a] + start * (to[a] - from[a])),
                    to,
                    self.tolerance_mm / e.lipschitz,
                )
                .map(|t| (start + (1.0 - start) * t, e.number))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
    }

    fn mesh_hit(
        &self,
        mesh: &simrep_mesh::Solid,
        from: Vector,
        to: Vector,
        launch: bool,
    ) -> Option<f64> {
        // A mirrored segment is piecewise linear in source coordinates. Split
        // at every reflection plane before querying triangles, not afterwards.
        let mut splits = vec![0.0, 1.0];
        if let Some(domain) = &self.domain {
            for a in 0..3 {
                if domain.mirrors[a] && from[a] != to[a] {
                    let t = (domain.origin[a] - from[a]) / (to[a] - from[a]);
                    if t > 0.0 && t < 1.0 {
                        splits.push(t);
                    }
                }
            }
        }
        splits.sort_unstable_by(f64::total_cmp);
        splits.dedup();
        let at = |t: f64| std::array::from_fn(|a| from[a] + t * (to[a] - from[a]));
        let fold = |mut p: Vector| {
            if let Some(domain) = &self.domain {
                for (a, v) in p.iter_mut().enumerate() {
                    if domain.mirrors[a] {
                        *v = domain.origin[a] + (*v - domain.origin[a]).abs();
                    }
                }
            }
            p
        };
        let contains = |p| {
            let (p, boundary) = self.coordinates(p);
            boundary <= 0.0 && mesh.contains(p)
        };
        let inside = contains(from);
        if !launch && inside {
            return Some(0.0);
        }
        let mut hits = Vec::new();
        for pair in splits.windows(2) {
            let a = fold(at(pair[0]));
            let b = fold(at(pair[1]));
            let (mut low, mut high) = (0.0_f64, 1.0_f64);
            if let Some(domain) = &self.domain {
                for axis in 0..3 {
                    if domain.dims[axis] <= 1 {
                        continue;
                    }
                    let min = domain.origin[axis];
                    let max = min + (domain.dims[axis] - 1) as f64 * domain.spacing[axis];
                    let direction = b[axis] - a[axis];
                    if direction == 0.0 {
                        if a[axis] < min || a[axis] > max {
                            high = -1.0;
                        }
                    } else {
                        let x = (min - a[axis]) / direction;
                        let y = (max - a[axis]) / direction;
                        low = low.max(x.min(y));
                        high = high.min(x.max(y));
                    }
                }
            }
            if low > high {
                continue;
            }
            let low = pair[0] + low * (pair[1] - pair[0]);
            let high = pair[0] + high * (pair[1] - pair[0]);
            if contains(at(low)) {
                hits.push(low);
            }
            hits.extend(
                mesh.segment_intersections(fold(at(low)), fold(at(high)))
                    .into_iter()
                    .map(|t| low + t * (high - low)),
            );
        }
        hits.sort_unstable_by(f64::total_cmp);
        hits.dedup_by(|a, b| (*a - *b).abs() <= f64::EPSILON * 16.0);
        let mut leaving = launch && inside;
        for (i, &t) in hits.iter().enumerate() {
            if leaving {
                let next = hits.get(i + 1).copied().unwrap_or(1.0);
                leaving = contains(at(0.5 * (t + next)));
                continue;
            }
            return Some(t);
        }
        None
    }

    fn distance(&self, electrode: &Electrode, p: Vector) -> f64 {
        let (p, boundary) = self.coordinates(p);
        electrode.shape.distance(p).max(boundary)
    }

    fn coordinates(&self, mut p: Vector) -> (Vector, f64) {
        let Some(domain) = &self.domain else {
            return (p, f64::NEG_INFINITY);
        };
        if domain.cylindrical {
            p[1] = p[1].hypot(p[2]);
            p[2] = 0.0;
        }
        for (a, value) in p.iter_mut().enumerate() {
            if domain.mirrors[a] {
                *value = domain.origin[a] + (*value - domain.origin[a]).abs();
            }
        }
        let boundary = (0..3)
            .filter(|&a| domain.dims[a] > 1)
            .map(|a| {
                let low = if domain.mirrors[a] || domain.cylindrical && a == 1 {
                    f64::NEG_INFINITY
                } else {
                    domain.origin[a] - p[a]
                };
                low.max(p[a] - domain.origin[a] - (domain.dims[a] - 1) as f64 * domain.spacing[a])
            })
            .fold(f64::NEG_INFINITY, f64::max);
        (p, boundary)
    }

    pub(crate) fn source_point(&self, local: Vector) -> Vector {
        self.domain
            .as_ref()
            .map_or(local, |d| std::array::from_fn(|a| local[a] + d.origin[a]))
    }

    pub(crate) fn validate_array(&self, array: &PotentialArray) -> Result<(), FlyError> {
        if let Some(d) = &self.domain
            && (d.dims != [array.nx, array.ny, array.nz]
                || d.spacing != [array.dx_mm, array.dy_mm, array.dz_mm]
                || d.cylindrical != (array.symmetry == Symmetry::Cylindrical)
                || d.mirrors != [array.mirrors.x, array.mirrors.y, array.mirrors.z])
        {
            return Err(FlyError(
                "cuts sidecar does not match PA dimensions, spacing or symmetry",
            ));
        }
        Ok(())
    }
}

fn checked_grid(dims: [usize; 3], origin: Vector, spacing: Vector) -> Result<usize, FlyError> {
    if dims.contains(&0)
        || !dims.iter().any(|n| *n > 1)
        || origin.iter().any(|v| !v.is_finite())
        || spacing.iter().any(|v| !v.is_finite() || *v <= 0.0)
    {
        return Err(FlyError("invalid surface grid"));
    }
    dims.into_iter()
        .try_fold(1usize, usize::checked_mul)
        .ok_or(FlyError("surface grid size overflow"))
}

fn point(i: usize, dims: [usize; 3], origin: Vector, spacing: Vector) -> Vector {
    let c = [i % dims[0], i / dims[0] % dims[1], i / (dims[0] * dims[1])];
    std::array::from_fn(|a| origin[a] + c[a] as f64 * spacing[a])
}

fn sampled_lipschitz(s: &SampledSdf) -> f64 {
    let strides = [1, s.dims[0], s.dims[0] * s.dims[1]];
    (0..3)
        .map(|a| {
            s.values
                .iter()
                .enumerate()
                .filter(|(i, _)| i / strides[a] % s.dims[a] + 1 < s.dims[a])
                .map(|(i, d)| ((s.values[i + strides[a]] - d) / s.spacing[a]).abs())
                .fold(0.0, f64::max)
                .powi(2)
        })
        .sum::<f64>()
        .sqrt()
}

/// A single continuous trilinear sign field per numbered electrode gives a
/// watertight cell interface. Link distances constrain nodal magnitudes;
/// underresolved features and conflicting link constraints cannot be recovered.
fn reconstruct_v1(data: &Sidecar, h: f64) -> Result<Vec<SurfaceSamples>, FlyError> {
    let mut numbers: Vec<_> = data
        .node_electrodes
        .iter()
        .flatten()
        .copied()
        .chain(data.cut_electrodes.iter().flatten().flatten().copied())
        .collect();
    numbers.sort_unstable();
    numbers.dedup();
    let strides = [1, data.dims[0], data.dims[0] * data.dims[1]];
    numbers
        .into_iter()
        .map(|number| {
            let mut values = vec![h; data.node_electrodes.len()];
            for (i, links) in data.cut_fractions.iter().enumerate() {
                for (s, fraction) in links.iter().enumerate() {
                    if let Some(t) = fraction {
                        if !t.is_finite() || !(0.0..=1.0).contains(t) {
                            return Err(FlyError("invalid v1 cut fraction"));
                        }
                        let a = s / 2;
                        let c = i / strides[a] % data.dims[a];
                        if data.cut_electrodes[i][s] != Some(number) {
                            continue;
                        }
                        if s % 2 == 0 && c == 0 || s % 2 == 1 && c + 1 == data.dims[a] {
                            return Err(FlyError("v1 cut link leaves PA domain"));
                        }
                        let j = i.wrapping_add_signed(if s % 2 == 0 {
                            -(strides[a] as isize)
                        } else {
                            strides[a] as isize
                        });
                        values[i] = values[i].min(t * data.spacing_mm[a]);
                        values[j] = values[j].min((1.0 - t) * data.spacing_mm[a]);
                    }
                }
            }
            for (i, v) in values.iter_mut().enumerate() {
                if data.node_electrodes[i] == Some(number) {
                    *v = -*v;
                }
            }
            Ok(SurfaceSamples {
                number,
                dims: data.dims,
                origin_mm: data.origin_mm,
                spacing_mm: data.spacing_mm,
                values,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Field, FlightOptions, GridField, InstanceField, Particle, SplatReason, fly};
    use simrep_geom::Sdf;

    const AXES: [Vector; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

    #[test]
    fn distant_segment_evaluates_each_conductor_only_once() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let calls = Arc::new(AtomicUsize::new(0));
        let count = Arc::clone(&calls);
        let surface = SurfaceModel::from_sdfs(
            vec![(
                17,
                Arc::new(move |p: Vector| {
                    count.fetch_add(1, Ordering::Relaxed);
                    10.0 - p[0]
                }),
            )],
            1e-12,
        )
        .unwrap();
        assert_eq!(surface.first_hit([0.0; 3], [1.0, 0.0, 0.0]), None);
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        let (fraction, number) = surface
            .first_hit([9.0, 0.0, 0.0], [11.0, 0.0, 0.0])
            .unwrap();
        assert_eq!(number, 17);
        assert!((fraction - 0.5).abs() < 1e-12);
    }

    fn sampled(shape: &dyn SignedDistance, n: usize, supersampling: usize) -> SurfaceModel {
        let dims = [n; 3];
        let spacing = [2.0 / (n - 1) as f64; 3];
        let sample = SurfaceSamples::sample(
            17,
            shape,
            [(n - 1) * supersampling + 1; 3],
            [-1.0; 3],
            spacing.map(|h| h / supersampling as f64),
        )
        .unwrap();
        let owners: Vec<_> = (0..n.pow(3))
            .map(|i| (shape.distance(point(i, dims, [-1.0; 3], spacing)) <= 0.0).then_some(17))
            .collect();
        let json = serde_json::json!({
            "version": 2, "dims": dims, "origin_mm": ([-1.0; 3]), "spacing_mm": spacing,
            "symmetry": "planar", "mirrors": ([false; 3]), "electrode_numbers": [17],
            "sampled_sdfs": [sample], "node_electrodes": owners,
            "cut_fractions": vec![[None::<f64>; 6]; n.pow(3)],
            "cut_electrodes": vec![[None::<usize>; 6]; n.pow(3)],
        });
        SurfaceModel::from_reader(json.to_string().as_bytes()).unwrap()
    }

    #[test]
    fn sphere_and_cylinder_hits_converge_with_sdf_sampling() {
        for shape in [
            Sdf::Sphere {
                center: [0.023, -0.017, 0.031],
                radius: 0.63,
            },
            Sdf::Cylinder {
                center: [0.023, -0.017, 0.031],
                radius: 0.63,
                axis: 2,
            },
        ] {
            let from = [-0.95, 0.173, 0.114];
            let to = [0.95, from[1], from[2]];
            let exact =
                SurfaceModel::from_sdfs(vec![(17, Arc::new(shape.clone()))], 1e-12).unwrap();
            let expected = exact.first_hit(from, to).unwrap().0;
            let errors: Vec<_> = [1, 2, 4, 8]
                .map(|s| {
                    (sampled(&shape, 21, s).first_hit(from, to).unwrap().0 - expected).abs() * 1.9
                })
                .to_vec();
            println!("{shape:?}: sampling hit errors mm {errors:?}");
            assert!(errors[1] < 0.01 * 0.1, "{errors:?}");
            assert!(errors[0] / errors[3] > 20.0, "{errors:?}");
            assert!(errors.windows(2).all(|e| e[1] < e[0]), "{errors:?}");
            // Both accepted endpoints are vacuum: first entry must still splat.
            assert_eq!(sampled(&shape, 11, 2).membership(to), None);
        }
    }

    #[test]
    fn source_geometry_avoids_false_voxel_start_and_reintegrates_to_surface() {
        let mut a = PotentialArray::new(6, 2, 1);
        for i in 0..a.potentials.len() {
            a.potentials[i] = -((i % 6) as f64) * 1e-4;
            a.electrodes[i] = i % 6 >= 3;
        }
        let surface = Arc::new(
            SurfaceModel::from_sdfs(
                vec![(
                    91,
                    Arc::new(Sdf::Plane {
                        normal: [-1.0, 0.0, 0.0],
                        offset: -2.7,
                    }),
                )],
                1e-12,
            )
            .unwrap(),
        );
        let original = InstanceField::new(
            GridField::new(a).unwrap().with_nearest_node_collisions(),
            [0.0; 3],
            AXES,
            1.0,
            None,
            None,
        )
        .unwrap();
        assert!(matches!(
            original.status([2.6, 0.5, 0.0]),
            Some(SplatReason::Electrode { .. })
        ));
        let field = original.with_surface(surface).unwrap();
        assert_eq!(field.status([2.6, 0.5, 0.0]), None);
        let p = Particle::new(1.0, 1.0, [2.6, 0.5, 0.0], [1.0, 0.0, 0.0], 0.0).unwrap();
        let r = fly(
            &field,
            &p,
            &FlightOptions {
                initial_step_us: 0.5,
                max_step_us: 0.5,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(r.splat_reason, SplatReason::SurfaceElectrode { number: 91 });
        assert!((r.final_state.position_mm[0] - 2.7).abs() < 1e-10);
        assert!(r.tof_us > 0.0);
        assert!(r.final_state.velocity_mm_per_us[0] > 1.0);
    }

    #[test]
    fn surface_preserves_placement_detector_bounds_and_pa_exit() {
        let model = Arc::new(sampled(
            &Sdf::Sphere {
                center: [0.0; 3],
                radius: 0.63,
            },
            11,
            2,
        ));
        let mut a = PotentialArray::new(11, 11, 11);
        [a.dx_mm, a.dy_mm, a.dz_mm] = [0.2; 3];
        let base = InstanceField::new(
            GridField::new(a).unwrap().with_nearest_node_collisions(),
            [10.0, 20.0, 30.0],
            AXES,
            2.0,
            None,
            None,
        )
        .unwrap()
        .with_surface(model)
        .unwrap();
        // PA local zero corresponds to source [-1,-1,-1], not [0,0,0].
        assert_eq!(base.status([10.1, 22.0, 32.0]), None);
        assert_eq!(
            base.status([12.0, 22.0, 32.0]),
            Some(SplatReason::SurfaceElectrode { number: 17 })
        );
        let p = Particle::new(1.0, 0.0, [10.1, 20.1, 30.1], [-1.0, 0.0, 0.0], 0.0).unwrap();
        assert_eq!(
            fly(&base, &p, &FlightOptions::default())
                .unwrap()
                .splat_reason,
            SplatReason::OutsideGrid
        );
        let mut detector = base.clone();
        detector.plane = Some(([1.0, 0.0, 0.0], 10.3));
        let p = Particle::new(1.0, 0.0, [10.1, 22.0, 32.0], [1.0, 0.0, 0.0], 0.0).unwrap();
        let r = fly(&detector, &p, &FlightOptions::default()).unwrap();
        assert_eq!(r.splat_reason, SplatReason::StopPlane);
        assert!((r.final_state.position_mm[0] - 10.3).abs() < 1e-10);
        let mut drift = base;
        drift.bounds_mm = Some([[0.0; 3], [50.0; 3]]);
        drift.plane = Some(([1.0, 0.0, 0.0], 17.0));
        let p = Particle::new(1.0, 0.0, [15.0, 20.1, 30.1], [1.0, 0.0, 0.0], 0.0).unwrap();
        assert_eq!(
            fly(&drift, &p, &FlightOptions::default())
                .unwrap()
                .splat_reason,
            SplatReason::StopPlane
        );
        drift.plane = None;
        drift.bounds_mm = Some([[0.0; 3], [16.0, 50.0, 50.0]]);
        assert_eq!(
            fly(&drift, &p, &FlightOptions::default())
                .unwrap()
                .splat_reason,
            SplatReason::OutsideGrid
        );
    }

    #[test]
    fn v1_reconstruction_is_continuous_and_explicitly_approximate() {
        let owners: Vec<_> = (0..10).map(|i| (i % 5 >= 3).then_some(9)).collect();
        let mut fractions = vec![[None::<f64>; 6]; 10];
        let mut numbers = vec![[None::<usize>; 6]; 10];
        for i in [2, 7] {
            fractions[i][1] = Some(0.3);
            numbers[i][1] = Some(9);
        }
        let data = serde_json::json!({
            "version":1, "dims":[5,2,1], "origin_mm":([0.0;3]), "spacing_mm":([1.0;3]),
            "symmetry":"planar", "node_electrodes":owners,
            "cut_fractions":fractions, "cut_electrodes":numbers
        });
        let surface = SurfaceModel::from_reader(data.to_string().as_bytes()).unwrap();
        assert_eq!(surface.accuracy(), SurfaceAccuracy::ApproximateV1);
        for y in [0.0, 0.37, 0.99, 1.0] {
            let (t, n) = surface.first_hit([1.0, y, 0.0], [4.0, y, 0.0]).unwrap();
            assert_eq!(n, 9);
            assert!((1.0 + 3.0 * t - 2.3).abs() < 1e-8);
        }
    }

    #[test]
    fn cylindrical_revolution_and_axial_mirror() {
        let shape = Sdf::Plane {
            normal: [0.0, -1.0, 0.0],
            offset: -0.73,
        };
        let sample = SurfaceSamples::sample(7, &shape, [9, 9, 1], [0.0; 3], [0.25; 3]).unwrap();
        let data = serde_json::json!({
            "version":2, "dims":[5,5,1], "origin_mm":([0.0;3]), "spacing_mm":([0.5;3]),
            "symmetry":"cylindrical", "mirrors":[true,false,false],
            "electrode_numbers":[7], "sampled_sdfs":[sample],
            "node_electrodes":vec![None::<usize>;25],
            "cut_fractions":vec![[None::<f64>;6];25],
            "cut_electrodes":vec![[None::<usize>;6];25]
        });
        let surface = SurfaceModel::from_reader(data.to_string().as_bytes()).unwrap();
        let (t, n) = surface
            .first_hit([-0.5, 0.0, 0.0], [-0.5, 0.0, -1.5])
            .unwrap();
        assert_eq!(n, 7);
        assert!((t * 1.5 - 0.73).abs() < 1e-9);
        assert_eq!(surface.membership([-0.5, -0.8, 0.0]), Some(7));
    }

    #[test]
    fn explicit_surface_launch_leaves_initial_conductor_and_hits_the_next() {
        let source = SurfaceModel::from_sdfs(
            vec![
                (
                    3,
                    Arc::new(Sdf::Sphere {
                        center: [2.0, 1.0, 1.0],
                        radius: 0.3,
                    }),
                ),
                (
                    7,
                    Arc::new(Sdf::Plane {
                        normal: [-1.0, 0.0, 0.0],
                        offset: -4.0,
                    }),
                ),
            ],
            1e-12,
        )
        .unwrap();
        assert_eq!(
            source.segment_hit([2.0, 1.0, 1.0], [2.0, 1.0, 1.0], true),
            None
        );
        let field = InstanceField::new(
            GridField::new(PotentialArray::new(6, 3, 3)).unwrap(),
            [0.0; 3],
            AXES,
            1.0,
            None,
            None,
        )
        .unwrap()
        .with_surface(Arc::new(source))
        .unwrap();
        let p = Particle::new(1.0, 0.0, [2.0, 1.0, 1.0], [1.0, 0.0, 0.0], 0.0).unwrap();
        let ordinary = fly(&field, &p, &FlightOptions::default()).unwrap();
        assert_eq!(
            ordinary.splat_reason,
            SplatReason::SurfaceElectrode { number: 3 }
        );
        assert_eq!(ordinary.tof_us, 0.0);
        let launched =
            crate::fly_many_from_electrode(&field, &[p], &FlightOptions::default()).unwrap();
        assert_eq!(
            launched[0].splat_reason,
            SplatReason::SurfaceElectrode { number: 7 }
        );
        assert!((launched[0].tof_us - 2.0).abs() < 1e-9);
    }

    #[test]
    fn malformed_sidecars_are_rejected_instead_of_falling_back() {
        let shape = Sdf::Sphere {
            center: [0.0; 3],
            radius: 0.63,
        };
        let sample = SurfaceSamples::sample(7, &shape, [5; 3], [-1.0; 3], [0.5; 3]).unwrap();
        let data = serde_json::json!({
            "version":2, "dims":[3,3,3], "origin_mm":([-1.0;3]), "spacing_mm":([1.0;3]),
            "symmetry":"planar", "electrode_numbers":[7],"sampled_sdfs":[sample],
            "node_electrodes":vec![None::<usize>;27],
            "cut_fractions":vec![[None::<f64>;6];27],
            "cut_electrodes":vec![[None::<usize>;6];27]
        });
        assert!(SurfaceModel::from_reader(data.to_string().as_bytes()).is_ok());
        for (field, value) in [
            ("version", serde_json::json!(3)),
            ("electrode_numbers", serde_json::json!([8])),
            ("dims", serde_json::json!([4, 3, 3])),
        ] {
            let mut invalid = data.clone();
            invalid[field] = value;
            assert!(SurfaceModel::from_reader(invalid.to_string().as_bytes()).is_err());
        }
        let mut invalid = data;
        invalid["sampled_sdfs"][0]["spacing_mm"][0] = serde_json::json!(0.25);
        assert!(SurfaceModel::from_reader(invalid.to_string().as_bytes()).is_err());
    }

    #[test]
    fn source_geometry_supports_nonzero_origin_without_resampling() {
        let shape = Sdf::Sphere {
            center: [0.0; 3],
            radius: 0.63,
        };
        let mut a = PotentialArray::new(11, 11, 11);
        [a.dx_mm, a.dy_mm, a.dz_mm] = [0.2; 3];
        let surface = SurfaceModel::from_sdfs(vec![(4, Arc::new(shape))], 1e-12)
            .unwrap()
            .with_pa_domain(&a, [-1.0; 3])
            .unwrap();
        assert_eq!(surface.accuracy(), SurfaceAccuracy::SourceGeometry);
        let field =
            InstanceField::new(GridField::new(a).unwrap(), [-1.0; 3], AXES, 1.0, None, None)
                .unwrap()
                .with_surface(Arc::new(surface))
                .unwrap();
        let p = Particle::new(1.0, 0.0, [-0.9, 0.0, 0.0], [1.0, 0.0, 0.0], 0.0).unwrap();
        let r = fly(&field, &p, &FlightOptions::default()).unwrap();
        assert_eq!(r.splat_reason, SplatReason::SurfaceElectrode { number: 4 });
        assert!((r.final_state.position_mm[0] + 0.63).abs() < 1e-11);
    }

    #[test]
    fn v3_csg_retains_the_grazing_aperture_corner_lost_by_v2() {
        let gem = simrep_gem::parse_gem(
            "pa_define{nx=9,ny=9,nz=9,dx=0.25} e(1){fill{
                within{box3d(0.65,0,0,0.9,2,2)}
                notin{cylinder3d(0,0,0,2,0,0,0.8)}
            }}",
        )
        .unwrap();
        let source = gem.electrode_sdfs().unwrap().remove(0);
        let sample = SurfaceSamples::sample(17, &source, [9; 3], [0.0; 3], [0.25; 3]).unwrap();
        let mut data = serde_json::json!({
            "version":3,"dims":[9,9,9],"origin_mm":([0.0;3]),"spacing_mm":([0.25;3]),
            "symmetry":"planar","mirrors":[false,true,true],"electrode_numbers":[17],
            "sampled_sdfs":[sample],
            "source_geometry":[SourceSurface{number:17,geometry:SourceGeometry::gem(&source).unwrap()}],
            "node_electrodes":vec![None::<usize>;729],
            "cut_fractions":vec![[None::<f64>;6];729],
            "cut_electrodes":vec![[None::<usize>;6];729]
        });
        let exact = SurfaceModel::from_reader(data.to_string().as_bytes())
            .unwrap()
            .with_tolerance(1e-12)
            .unwrap();
        assert_eq!(exact.accuracy(), SurfaceAccuracy::SourceGeometry);
        // Enter just outside the circular hole and leave through the cap.
        // Sampling max(box,-cylinder) before interpolation rounds this away.
        let from = [0.6, -0.801, 0.0];
        let to = [0.7, -0.801, 0.0];
        let (t, number) = exact.first_hit(from, to).unwrap();
        assert_eq!(number, 17);
        assert!((from[0] + t * 0.1 - 0.65).abs() < 1e-10);
        assert_eq!(exact.membership(to), Some(17));
        let mut without_samples = data.clone();
        without_samples
            .as_object_mut()
            .unwrap()
            .remove("sampled_sdfs");
        without_samples
            .as_object_mut()
            .unwrap()
            .remove("cut_fractions");
        without_samples
            .as_object_mut()
            .unwrap()
            .remove("cut_electrodes");
        without_samples["link_intersections"] = serde_json::json!([{
            "node":0, "fractions":[null,0.25,null,null,null,null],
            "electrodes":[null,17,null,null,null,null]
        }]);
        let source_only = SurfaceModel::from_reader(without_samples.to_string().as_bytes())
            .unwrap()
            .with_tolerance(1e-12)
            .unwrap();
        assert_eq!(source_only.accuracy(), SurfaceAccuracy::SourceGeometry);
        assert_eq!(source_only.first_hit(from, to), exact.first_hit(from, to));
        assert_eq!(source_only.membership(to), Some(17));
        let mut runs = without_samples.clone();
        runs.as_object_mut().unwrap().remove("node_electrodes");
        runs["node_electrode_runs"] = serde_json::json!([[0, 729, null]]);
        let compact = SurfaceModel::from_reader(runs.to_string().as_bytes())
            .unwrap()
            .with_tolerance(1e-12)
            .unwrap();
        assert_eq!(compact.first_hit(from, to), exact.first_hit(from, to));
        for invalid_runs in [
            serde_json::json!([[0, 728, null]]),
            serde_json::json!([[0, 300, null], [301, 729, null]]),
            serde_json::json!([[0, 300, null], [299, 729, null]]),
            serde_json::json!([[0, 729, 18]]),
        ] {
            runs["node_electrode_runs"] = invalid_runs;
            assert!(SurfaceModel::from_reader(runs.to_string().as_bytes()).is_err());
        }
        let mut invalid = without_samples.clone();
        invalid["link_intersections"][0]["node"] = serde_json::json!(729);
        assert!(SurfaceModel::from_reader(invalid.to_string().as_bytes()).is_err());
        invalid = without_samples.clone();
        invalid["link_intersections"][0]["fractions"][1] = serde_json::json!(1.5);
        assert!(SurfaceModel::from_reader(invalid.to_string().as_bytes()).is_err());
        invalid = without_samples.clone();
        invalid["link_intersections"][0]["electrodes"][1] = serde_json::json!(18);
        assert!(SurfaceModel::from_reader(invalid.to_string().as_bytes()).is_err());
        without_samples["node_electrodes"][0] = serde_json::json!(18);
        assert!(SurfaceModel::from_reader(without_samples.to_string().as_bytes()).is_err());
        data["version"] = serde_json::json!(2);
        let approximate = SurfaceModel::from_reader(data.to_string().as_bytes()).unwrap();
        assert_eq!(approximate.membership(to), None);
        assert!(approximate.first_hit(from, to).is_none());
    }

    #[test]
    fn source_binary_retains_transforms_and_nonfinite_primitive_parameters() {
        let gem = simrep_gem::parse_gem(
            "pa_define(21,21,21,p,n,e,,0.5,,,gu)
             locate(5,5,5,2,90,0,0){e(2){cylinder3d(0,0,-inf,0,0,inf,1)}}",
        )
        .unwrap();
        let source = gem.electrode_sdfs().unwrap().remove(0);
        let encoded = SourceGeometry::gem(&source).unwrap();
        let json = serde_json::to_vec(&encoded).unwrap();
        let restored: SourceGeometry = serde_json::from_slice(&json).unwrap();
        let electrode = restored.electrode(73).unwrap();
        for p in [[2.5; 3], [0.0; 3], [3.0, 2.5, 2.5], [5.0, 3.0, 1.0]] {
            assert_eq!(source.distance(p), electrode.shape.distance(p));
        }
    }

    #[test]
    fn staircase_hit_error_tracks_pa_spacing_instead_of_sdf_sampling() {
        for spherical in [true, false] {
            let shape = if spherical {
                Sdf::Sphere {
                    center: [1.0; 3],
                    radius: 0.63,
                }
            } else {
                Sdf::Cylinder {
                    center: [1.0; 3],
                    radius: 0.63,
                    axis: 2,
                }
            };
            let mut errors = Vec::new();
            for n in [11usize, 21, 41] {
                let h = 2.0 / (n - 1) as f64;
                let mut array = PotentialArray::new(n, n, n);
                [array.dx_mm, array.dy_mm, array.dz_mm] = [h; 3];
                for (i, owner) in array.electrodes.iter_mut().enumerate() {
                    *owner = shape.distance(point(i, [n; 3], [0.0; 3], [h; 3])) <= 0.0;
                }
                let field = GridField::new(array)
                    .unwrap()
                    .with_nearest_node_collisions();
                let mut sum = 0.0;
                for j in 0..37 {
                    let dy = 0.037 + 0.4 * j as f64 / 37.0;
                    let dz = 0.041 + 0.21 * ((j * 17) % 37) as f64 / 37.0;
                    let from = [0.05, 1.0 + dy, 1.0 + dz];
                    let to = [1.95, from[1], from[2]];
                    let exact_x = 1.0
                        - (0.63_f64.powi(2) - dy * dy - if spherical { dz * dz } else { 0.0 })
                            .sqrt();
                    let t = field.intersection(from, to).unwrap().0;
                    sum += (from[0] + 1.9 * t - exact_x).powi(2);
                }
                errors.push((sum / 37.0).sqrt());
            }
            println!("spherical={spherical}: staircase RMS hit errors mm {errors:?}");
            assert!(
                errors[0] / errors[2] > 2.0 && errors[0] / errors[2] < 8.0,
                "{errors:?}"
            );
            assert!(errors.windows(2).all(|e| e[1] < e[0]), "{errors:?}");
        }
    }
}

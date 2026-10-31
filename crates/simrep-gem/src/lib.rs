//! SIMION GEM geometry, sandboxed Lua 5.1, ordered fills and PA rasterization.
//!
//! [`read_gem`] resolves relative includes. [`parse_gem`] parses an in-memory
//! source without includes. Rasterization is independent of PA serialization;
//! [`Gem::rasterize`] returns a `simrep_io::PaFile`.

pub mod geometry;
#[cfg(feature = "lua")]
mod lua;
#[cfg(not(feature = "lua"))]
#[path = "lua_disabled.rs"]
mod lua;
mod parser;
mod raster;
mod stl;
mod surface;
pub use surface::SurfaceOffsets;

use std::fmt;
use std::path::Path;
use std::time::Duration;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

use geometry::{Inclusion, RasterMode, Region, Shape, transform};
use parser::{Node, Parser, Value};
use serde::{Deserialize, Serialize};
use simrep_core::{FieldType, MirrorFlags, PotentialArray, Symmetry};
use simrep_io::PaFile;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Error(pub String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for Error {}
impl Error {
    /// A valid geometry fragment needs an enclosing array definition.
    pub fn is_include_fragment(&self) -> bool {
        self.0.starts_with("include fragment:")
    }
}
pub type Result<T> = std::result::Result<T, Error>;

/// Features intentionally rejected rather than silently rasterized incorrectly.
pub const UNSUPPORTED_FEATURES: &[&str] = &[
    "host Lua modules and filesystem/process access",
    "signed-distance export and serialization of Lua shape/potential callbacks",
    "multiple pa_define / workbench GEM",
    "include arguments",
    "infinite cylinder3d endpoints that do not define an axis",
];

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Fill {
    pub electrode: bool,
    pub potential: f64,
    pub edge: bool,
    pub shape: Shape,
}

#[derive(Clone, Debug)]
pub struct Gem {
    pub dimensions: [usize; 3],
    pub symmetry: Symmetry,
    pub mirrors: MirrorFlags,
    pub field_type: FieldType,
    pub refinable: bool,
    pub spacing_mm: [f64; 3],
    pub ng: u32,
    /// The ordered analytic CSG fills, also usable by future cut-cell consumers.
    pub fills: Vec<Fill>,
    pub surface_auto: bool,
    pub surface_fractional: bool,
    pub grid_units: bool,
}

/// Physical, ordered GEM CSG for a single electrode, evaluated before any
/// interpolation. The coordinate scale retains anisotropic GEM grid units.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ElectrodeSdf {
    pub voltage: f64,
    fills: Vec<Fill>,
    coordinate_scale: [f64; 3],
}

impl ElectrodeSdf {
    /// Conservative physical bounds of the electrode's positive fills.
    /// Vacuum/other-electrode overwrites can only shrink these bounds.
    /// Unbounded primitive axes remain infinite.
    pub fn bounds_mm(&self) -> [[f64; 3]; 2] {
        let mut bounds = [[f64::INFINITY; 3], [f64::NEG_INFINITY; 3]];
        for fill in self
            .fills
            .iter()
            .filter(|fill| fill.electrode && fill.potential == self.voltage)
        {
            let (min, max) = fill.shape.bounds([0.0; 3]);
            for a in 0..3 {
                bounds[0][a] = bounds[0][a].min(min[a] * self.coordinate_scale[a]);
                bounds[1][a] = bounds[1][a].max(max[a] * self.coordinate_scale[a]);
            }
        }
        bounds
    }

    pub fn try_distance(&self, point_mm: [f64; 3]) -> Result<f64> {
        let p = std::array::from_fn(|a| point_mm[a] / self.coordinate_scale[a]);
        let mut distance = f64::MAX;
        // Before the first fill of this electrode, max(MAX, -other) is
        // identically MAX. Avoid evaluating those irrelevant source trees.
        for fill in self
            .fills
            .iter()
            .skip_while(|fill| !fill.electrode || fill.potential != self.voltage)
        {
            let d = fill.shape.signed_distance(p)?;
            let d = if fill.edge { d.abs() } else { d };
            distance = if fill.electrode && fill.potential == self.voltage {
                distance.min(d)
            } else {
                distance.max(-d)
            };
        }
        Ok(distance
            * self
                .coordinate_scale
                .into_iter()
                .fold(f64::INFINITY, f64::min))
    }

    /// Validate decoded sources before exposing the infallible distance trait.
    pub fn validate(&self) -> Result<()> {
        if !self.voltage.is_finite()
            || self
                .coordinate_scale
                .iter()
                .any(|s| !s.is_finite() || *s <= 0.0)
            || !self
                .fills
                .iter()
                .any(|f| f.electrode && f.potential == self.voltage)
        {
            return Err(Error(
                "invalid source CSG electrode or coordinate scale".into(),
            ));
        }
        for fill in &self.fills {
            fill.shape.validate_sdf()?;
        }
        self.try_distance([0.0; 3])?;
        Ok(())
    }
}

impl simrep_geom::SignedDistance for ElectrodeSdf {
    fn distance(&self, point: [f64; 3]) -> f64 {
        self.try_distance(point).unwrap_or(f64::NAN)
    }

    fn segment_distance_bounds(&self, from: [f64; 3], to: [f64; 3]) -> Option<[f64; 2]> {
        let from = std::array::from_fn(|a| from[a] / self.coordinate_scale[a]);
        let to = std::array::from_fn(|a| to[a] / self.coordinate_scale[a]);
        let mut bounds = [f64::MAX; 2];
        for fill in self
            .fills
            .iter()
            .skip_while(|fill| !fill.electrode || fill.potential != self.voltage)
        {
            let mut next = fill.shape.distance_bounds(from, to).ok()?;
            if fill.edge {
                next = [
                    if next[0] <= 0.0 && next[1] >= 0.0 {
                        0.0
                    } else {
                        next[0].abs().min(next[1].abs())
                    },
                    next[0].abs().max(next[1].abs()),
                ];
            }
            bounds = if fill.electrode && fill.potential == self.voltage {
                [bounds[0].min(next[0]), bounds[1].min(next[1])]
            } else {
                [bounds[0].max(-next[1]), bounds[1].max(-next[0])]
            };
        }
        let scale = self
            .coordinate_scale
            .into_iter()
            .fold(f64::INFINITY, f64::min);
        Some(bounds.map(|d| d * scale))
    }

    fn segment_single_interval(&self, from: [f64; 3], to: [f64; 3], level: f64) -> bool {
        let scale = self
            .coordinate_scale
            .into_iter()
            .fold(f64::INFINITY, f64::min);
        let level = level / scale;
        let from = std::array::from_fn(|a| from[a] / self.coordinate_scale[a]);
        let to = std::array::from_fn(|a| to[a] / self.coordinate_scale[a]);
        let mut included = 0;
        for fill in self
            .fills
            .iter()
            .skip_while(|fill| !fill.electrode || fill.potential != self.voltage)
        {
            let Ok(bounds) = fill.shape.distance_bounds(from, to) else {
                return false;
            };
            if fill.electrode && fill.potential == self.voltage {
                if bounds[0] <= level {
                    included += 1;
                    if included > 1
                        || fill.edge
                        || !fill.shape.segment_single_interval(from, to, level)
                    {
                        return false;
                    }
                }
            } else if fill.edge || bounds[0] < -level {
                return false;
            }
        }
        true
    }
}

pub struct Rasterized {
    pub pa: PaFile,
    pub surfaces: Option<SurfaceOffsets>,
    /// Grid allocation is excluded, as are parsing and output serialization.
    pub compute_time: Duration,
}

/// Samples of one electrode's conservative SDF, in millimeters.
/// Negative values are conductor material; values use x-fastest indexing.
#[derive(Clone, Debug)]
pub struct SampledElectrodeSdf {
    /// GEM potential, or electrode number for a fast-adjustable PA#.
    pub voltage: f64,
    pub dims: [usize; 3],
    pub origin: [f64; 3],
    pub spacing: [f64; 3],
    pub values: Vec<f64>,
}

impl SampledElectrodeSdf {
    /// Trilinear interpolation inside the closed sample domain.
    pub fn sample(&self, point: [f64; 3]) -> Option<f64> {
        let mut lo = [0; 3];
        let mut hi = [0; 3];
        let mut fraction = [0.; 3];
        for i in 0..3 {
            let grid = (point[i] - self.origin[i]) / self.spacing[i];
            if !grid.is_finite() || grid < 0. || grid > (self.dims[i] - 1) as f64 {
                return None;
            }
            lo[i] = grid.floor() as usize;
            hi[i] = (lo[i] + 1).min(self.dims[i] - 1);
            fraction[i] = grid - lo[i] as f64;
        }
        let mut value = 0.;
        for z in 0..2 {
            for y in 0..2 {
                for x in 0..2 {
                    let select = [x, y, z];
                    let index: [usize; 3] =
                        std::array::from_fn(|i| if select[i] == 0 { lo[i] } else { hi[i] });
                    let weight = (0..3)
                        .map(|i| {
                            if select[i] == 0 {
                                1. - fraction[i]
                            } else {
                                fraction[i]
                            }
                        })
                        .product::<f64>();
                    value += weight
                        * self.values
                            [index[0] + self.dims[0] * (index[1] + self.dims[1] * index[2])];
                }
            }
        }
        Some(value)
    }
}

pub fn read_gem(path: impl AsRef<Path>) -> Result<Gem> {
    compile(Parser::new().file(path.as_ref())?)
}

/// Read literal GEM and relative includes from caller-owned bytes.
/// Executable Lua, when enabled, retains its native filesystem semantics.
pub fn read_gem_with(
    path: impl AsRef<Path>,
    load: impl Fn(&Path) -> Result<Vec<u8>> + Send + Sync + 'static,
) -> Result<Gem> {
    compile(Parser::with_loader(Box::new(load)).file(path.as_ref())?)
}

pub fn parse_gem(source: &str) -> Result<Gem> {
    compile(Parser::new().source(source, Path::new("."))?)
}

impl Gem {
    /// Retain source primitives, transforms and ordered overwrites in mm.
    pub fn electrode_sdfs(&self) -> Result<Vec<ElectrodeSdf>> {
        let mut sources = Vec::new();
        for fill in &self.fills {
            if fill.electrode
                && !sources
                    .iter()
                    .any(|s: &ElectrodeSdf| s.voltage == fill.potential)
            {
                let source = ElectrodeSdf {
                    voltage: fill.potential,
                    fills: self.fills.clone(),
                    coordinate_scale: if self.grid_units {
                        self.spacing_mm
                    } else {
                        [1.0; 3]
                    },
                };
                source.validate()?;
                sources.push(source);
            }
        }
        Ok(sources)
    }

    /// Sample physical surfaces, without GEM's half-grid raster expansion.
    /// Later fills, including non-electrode cuts, retain their overwrite order.
    pub fn sample_electrode_sdfs(
        &self,
        dims: [usize; 3],
        origin: [f64; 3],
        spacing: [f64; 3],
    ) -> Result<Vec<SampledElectrodeSdf>> {
        if dims.contains(&0)
            || origin.iter().any(|x| !x.is_finite())
            || spacing.iter().any(|x| !x.is_finite() || *x <= 0.)
        {
            return Err(Error(
                "SDF sampling requires positive dimensions/spacing and finite coordinates".into(),
            ));
        }
        let count = dims[0]
            .checked_mul(dims[1])
            .and_then(|n| n.checked_mul(dims[2]))
            .ok_or_else(|| Error("SDF sample count overflows usize".into()))?;
        let mut voltages = Vec::new();
        for fill in &self.fills {
            if fill.electrode && !voltages.contains(&fill.potential) {
                voltages.push(fill.potential);
            }
        }
        let mut samples = Vec::new();
        for voltage in voltages {
            let mut values = Vec::new();
            values
                .try_reserve_exact(count)
                .map_err(|e| Error(e.to_string()))?;
            for z in 0..dims[2] {
                for y in 0..dims[1] {
                    for x in 0..dims[0] {
                        let mut p =
                            std::array::from_fn(|i| origin[i] + [x, y, z][i] as f64 * spacing[i]);
                        if self.grid_units {
                            for (i, v) in p.iter_mut().enumerate() {
                                *v /= self.spacing_mm[i];
                            }
                        }
                        let mut d = f64::MAX;
                        for fill in &self.fills {
                            let shape = fill.shape.signed_distance(p)?;
                            let shape = if fill.edge { shape.abs() } else { shape };
                            if fill.electrode && fill.potential == voltage {
                                d = d.min(shape);
                            } else {
                                d = d.max(-shape);
                            }
                        }
                        if self.grid_units {
                            d *= self.spacing_mm.into_iter().fold(f64::INFINITY, f64::min);
                        }
                        if !d.is_finite() {
                            return Err(Error("SDF sampling produced a non-finite value".into()));
                        }
                        values.push(d);
                    }
                }
            }
            samples.push(SampledElectrodeSdf {
                voltage,
                dims,
                origin,
                spacing,
                values,
            });
        }
        Ok(samples)
    }

    pub fn rasterize(&self) -> Result<Rasterized> {
        let [nx, ny, nz] = self.dimensions;
        let mut array = PotentialArray::new(nx, ny, nz);
        array.symmetry = self.symmetry;
        array.mirrors = self.mirrors;
        array.field_type = self.field_type;
        array.refinable = self.refinable;
        [array.dx_mm, array.dy_mm, array.dz_mm] = self.spacing_mm;
        let coordinate_scale = if self.grid_units {
            [1.; 3]
        } else {
            self.spacing_mm
        };
        let margin = coordinate_scale.map(|s| {
            s * if self.surface_fractional {
                0.00005
            } else if self.surface_auto {
                0.35
            } else {
                0.5
            }
        });
        #[cfg(not(target_arch = "wasm32"))]
        let start = Instant::now();
        for fill in &self.fills {
            let mode = RasterMode {
                auto: self.surface_auto,
                fractional: self.surface_fractional,
                electrode: fill.electrode,
            };
            let (min, max) = fill.shape.bounds(margin);
            let lo: [usize; 3] = std::array::from_fn(|i| {
                (min[i] / coordinate_scale[i])
                    .ceil()
                    .clamp(0., self.dimensions[i] as f64) as usize
            });
            let hi: [usize; 3] = std::array::from_fn(|i| {
                ((max[i] / coordinate_scale[i]).floor() + 1.).clamp(0., self.dimensions[i] as f64)
                    as usize
            });
            if lo[2] >= hi[2] {
                continue;
            }
            let plane = nx * ny;
            let raster = |potentials: &mut [f64], electrodes: &mut [bool], zbase: usize| {
                for local_z in 0..potentials.len() / plane {
                    let z = zbase + local_z;
                    for y in lo[1]..hi[1] {
                        let p = [
                            0.,
                            y as f64 * coordinate_scale[1],
                            z as f64 * coordinate_scale[2],
                        ];
                        let spans = if fill.edge {
                            fill.shape.edge_spans(
                                p,
                                coordinate_scale,
                                margin,
                                mode,
                                self.dimensions,
                            )
                        } else {
                            fill.shape.spans(
                                p,
                                [coordinate_scale[0], 0., 0.],
                                margin,
                                false,
                                mode,
                                nx,
                            )
                        };
                        for (lo, hi) in spans {
                            let lo = lo.max((min[0] / coordinate_scale[0]).ceil().max(0.) as usize);
                            let hi =
                                hi.min(
                                    ((max[0] / coordinate_scale[0]).floor() + 1.).max(0.) as usize
                                );
                            if lo >= hi {
                                continue;
                            }
                            let base = (local_z * ny + y) * nx;
                            if fill.shape.has_potential() {
                                for (x, value) in
                                    potentials[base + lo..base + hi].iter_mut().enumerate()
                                {
                                    *value = fill
                                        .shape
                                        .potential([
                                            (lo + x) as f64 * coordinate_scale[0],
                                            p[1],
                                            p[2],
                                        ])
                                        .unwrap();
                                }
                            } else {
                                potentials[base + lo..base + hi].fill(fill.potential);
                            }
                            electrodes[base + lo..base + hi].fill(fill.electrode);
                        }
                    }
                }
            };
            let points = plane * (hi[2] - lo[2]);
            let potentials = &mut array.potentials[lo[2] * plane..hi[2] * plane];
            let electrodes = &mut array.electrodes[lo[2] * plane..hi[2] * plane];
            if !cfg!(target_arch = "wasm32") && points > 200_000 && hi[2] - lo[2] > 4 {
                let workers = std::thread::available_parallelism()
                    .map_or(1, usize::from)
                    .min(16);
                let depth = (hi[2] - lo[2]).div_ceil(workers);
                std::thread::scope(|scope| {
                    for (i, (v, e)) in potentials
                        .chunks_mut(plane * depth)
                        .zip(electrodes.chunks_mut(plane * depth))
                        .enumerate()
                    {
                        scope.spawn(move || raster(v, e, lo[2] + i * depth));
                    }
                });
            } else {
                raster(potentials, electrodes, lo[2]);
            }
        }
        let surfaces = self
            .surface_fractional
            .then(|| self.surface_offsets(&array));
        for fill in &self.fills {
            fill.shape.check_callbacks()?;
        }
        #[cfg(not(target_arch = "wasm32"))]
        let compute_time = start.elapsed();
        #[cfg(target_arch = "wasm32")]
        let compute_time = Duration::ZERO;
        let mut pa = PaFile::new(array);
        pa.ng = self.ng;
        Ok(Rasterized {
            pa,
            compute_time,
            surfaces,
        })
    }
}

fn compile(mut nodes: Vec<Node>) -> Result<Gem> {
    if nodes.iter().all(|n| n.name != "pa_define") {
        return Err(Error("include fragment: no pa_define; compile through a parent GEM to select the output grid".into()));
    }
    if nodes.first().is_none_or(|n| n.name != "pa_define") {
        return Err(Error("pa_define must precede geometry instructions".into()));
    }
    let definition = nodes.remove(0);
    let n = &definition;
    for key in n.named.keys() {
        if ![
            "nx",
            "ny",
            "nz",
            "gx",
            "gy",
            "gz",
            "wx",
            "wy",
            "wz",
            "symmetry",
            "mirror",
            "type",
            "ng",
            "dx",
            "dy",
            "dz",
            "unit",
            "surface",
            "refinable",
            "filename",
            "nz_use",
            "refine_convergence",
        ]
        .contains(&key.as_str())
        {
            return Err(Error(format!("unsupported pa_define parameter '{key}'")));
        }
    }
    let number = |key: &str, index, default| -> Result<f64> {
        match n.named.get(key) {
            Some(v) => v.number(),
            None => n.num(index, default),
        }
    };
    let string = |key: &str, index, default: &str| -> Result<String> {
        Ok(match n.named.get(key) {
            Some(v) => v.text()?,
            None => n.string(index, default)?,
        }
        .to_ascii_lowercase())
    };
    let dx = number("dx", 7, 1.)?;
    let dy = number("dy", 8, dx)?;
    let dz = number("dz", 9, dy)?;
    let spacing_mm = [dx, dy, dz];
    if spacing_mm.iter().any(|v| !v.is_finite() || *v <= 0.) {
        return Err(Error(
            "pa_define cell sizes must be positive finite numbers".into(),
        ));
    }
    let mut dimensions = [100, 20, 1];
    for i in 0..3 {
        let keys = [["nx", "gx", "wx"], ["ny", "gy", "wy"], ["nz", "gz", "wz"]][i];
        let v = if let Some(v) = n.named.get(keys[0]) {
            v.number()?
        } else if let Some(v) = n.named.get(keys[1]) {
            v.number()? + 1.
        } else if let Some(v) = n.named.get(keys[2]) {
            (v.number()? / spacing_mm[i]).ceil() + 1.
        } else if let Some(Value::Number(v, true)) = n.args.get(i) {
            (v / spacing_mm[i]).ceil() + 1.
        } else {
            n.num(i, dimensions[i] as f64)?
        };
        if !v.is_finite()
            || (v - v.round()).abs() > 1e-9
            || v < if i < 2 { 3. } else { 1. }
            || v > 90000.
        {
            return Err(Error(format!("invalid PA dimension {}={v}", keys[0])));
        }
        dimensions[i] = v.round() as usize;
    }
    dimensions[0]
        .checked_mul(dimensions[1])
        .and_then(|n| n.checked_mul(dimensions[2]))
        .filter(|&n| n <= isize::MAX as usize / 8)
        .ok_or_else(|| Error("PA point count overflows address space".into()))?;
    let sym_default = if n.table || dimensions[2] > 1 {
        "planar"
    } else {
        "cylindrical"
    };
    let sym_index = 3;
    let sym = string("symmetry", sym_index, sym_default)?;
    let symmetry = if sym.starts_with('p') {
        Symmetry::Planar
    } else if sym.starts_with('c') {
        Symmetry::Cylindrical
    } else {
        return Err(Error(format!("invalid PA symmetry '{sym}'")));
    };
    let mirror = if n.table {
        if let Some(v) = n.named.get("mirror") {
            v.text()?.to_string()
        } else {
            n.string(4, sym.split_whitespace().nth(1).unwrap_or(""))?
                .to_string()
        }
    } else {
        string(
            "mirror",
            4,
            if symmetry == Symmetry::Cylindrical {
                "y"
            } else {
                ""
            },
        )?
    };
    let mirrors = MirrorFlags {
        x: mirror.contains('x'),
        y: symmetry == Symmetry::Cylindrical || mirror.contains('y'),
        z: mirror.contains('z'),
    };
    if symmetry == Symmetry::Cylindrical && (dimensions[2] != 1 || dy != dz)
        || dimensions[2] == 1 && mirrors.z
    {
        return Err(Error(
            "invalid symmetry/mirroring/grid spacing combination".into(),
        ));
    }
    let type_index = if n.table { usize::MAX } else { 5 };
    let field = string("type", type_index, "electric")?;
    let field_type = if field.starts_with('e') {
        FieldType::Electrostatic
    } else if field.starts_with('m') {
        FieldType::Magnetic
    } else {
        return Err(Error(format!("invalid PA type '{field}'")));
    };
    let ng = number("ng", if n.table { usize::MAX } else { 6 }, 100.)?;
    if !ng.is_finite() || !(0. ..=131071.).contains(&ng) {
        return Err(Error("ng must be in 0..=131071".into()));
    }
    let surface = string("surface", 11, if n.table { "auto" } else { "none" })?;
    if !["none", "auto", "fractional"].contains(&surface.as_str()) {
        return Err(Error(format!("unsupported GEM feature surface={surface}")));
    }
    let refinable = match n.named.get("refinable") {
        Some(Value::Text(v)) if v == "true" => true,
        Some(Value::Text(v)) if v == "false" => false,
        Some(_) => return Err(Error("refinable must be true or false".into())),
        None => !field.ends_with('_'),
    };
    let unit = string("unit", 10, "")?;
    if !unit.is_empty() && unit != "gu" {
        return Err(Error(format!("unsupported GEM unit '{unit}'")));
    }
    let mut gem = Gem {
        dimensions,
        symmetry,
        mirrors,
        field_type,
        refinable,
        spacing_mm,
        ng: if ng < 1. || field_type == FieldType::Electrostatic {
            100
        } else {
            ng as u32
        },
        fills: Vec::new(),
        surface_auto: surface != "none",
        surface_fractional: surface == "fractional",
        grid_units: unit == "gu",
    };
    base(&nodes, true, 0., &mut gem.fills)?;
    Ok(gem)
}

fn electrode(name: &str) -> Option<bool> {
    match name {
        "electrode" | "e" | "p" | "elect" | "pole" | "electrode_points" | "pole_points" => {
            Some(true)
        }
        "non_electrode"
        | "n"
        | "non_e"
        | "non_p"
        | "non_pole"
        | "non_electrode_points"
        | "non_pole_points" => Some(false),
        _ => None,
    }
}

fn is_fill(name: &str) -> bool {
    matches!(
        name,
        "fill"
            | "fill_volume"
            | "edge_fill"
            | "edge_fill_volume"
            | "rotate_fill"
            | "rotate_fill_volume"
            | "rotate_edge_fill"
            | "rotate_edge_fill_volume"
    )
}

fn within(name: &str) -> Option<(bool, Inclusion)> {
    Some(match name {
        "within" | "intersect" => (false, Inclusion::Expanded),
        "notin" => (true, Inclusion::Expanded),
        "within_inside" => (false, Inclusion::Inside),
        "notin_inside" => (true, Inclusion::Inside),
        "within_inside_or_on" => (false, Inclusion::InsideOrOn),
        "notin_inside_or_on" => (true, Inclusion::InsideOrOn),
        _ => return None,
    })
}

fn base(nodes: &[Node], is_electrode: bool, potential: f64, fills: &mut Vec<Fill>) -> Result<()> {
    let mut pending = Vec::new();
    let mut has_notin = false;
    let flush = |pending: &mut Vec<Node>, fills: &mut Vec<Fill>| -> Result<()> {
        if !pending.is_empty() {
            fills.push(Fill {
                electrode: is_electrode,
                potential,
                edge: false,
                shape: region_shape(pending)?,
            });
            pending.clear();
        }
        Ok(())
    };
    for n in nodes {
        let structural = electrode(&n.name).is_some() || is_fill(&n.name);
        let t = transform(n)?;
        // Locates wrapping structural nodes retain fill ordering and scope.
        let t_base = t.is_some()
            && n.children
                .iter()
                .any(|c| electrode(&c.name).is_some() || is_fill(&c.name) || contains_base(c));
        if structural || t_base {
            flush(&mut pending, fills)?;
            has_notin = false;
            if let Some(e) = electrode(&n.name) {
                #[cfg(feature = "lua")]
                if let Some(Value::Callback(callback)) = n.args.first() {
                    let start = fills.len();
                    base(&n.children, e, 0., fills)?;
                    for fill in &mut fills[start..] {
                        fill.shape =
                            Shape::Potential(callback.clone(), Box::new(fill.shape.clone()));
                    }
                    continue;
                }
                let potential = n.num(0, 0.)?;
                if !potential.is_finite() {
                    return Err(Error(format!(
                        "line {}: electrode potential must be finite",
                        n.line
                    )));
                }
                base(&n.children, e, potential, fills)?;
            } else if let Some(t) = t {
                let start = fills.len();
                base(&n.children, is_electrode, potential, fills)?;
                for f in &mut fills[start..] {
                    f.shape = Shape::Transform(t, Box::new(f.shape.clone()));
                }
            } else {
                let mut shape = region_shape(&n.children)?;
                if n.name.starts_with("rotate_") {
                    let angle = n.num(0, 360.)?;
                    shape = Shape::Revolve {
                        shape: Box::new(shape),
                        start: 0f64.min(angle),
                        end: 0f64.max(angle),
                        axis: 0,
                    };
                }
                fills.push(Fill {
                    electrode: is_electrode,
                    potential,
                    edge: n.name.contains("edge"),
                    shape,
                });
            }
        } else {
            let exclusion = within(&n.name).is_some_and(|(e, _)| e);
            if !exclusion && has_notin {
                flush(&mut pending, fills)?;
                has_notin = false;
            }
            has_notin |= exclusion;
            pending.push(n.clone());
        }
    }
    flush(&mut pending, fills)
}

fn contains_base(n: &Node) -> bool {
    n.children
        .iter()
        .any(|n| electrode(&n.name).is_some() || is_fill(&n.name) || contains_base(n))
}

fn region_shape(nodes: &[Node]) -> Result<Shape> {
    if nodes.is_empty() {
        return Err(Error("fill/shape block cannot be empty".into()));
    }
    let mut regions = Vec::new();
    for n in nodes {
        if let Some((exclude, inclusion)) = within(&n.name) {
            let shapes = n
                .children
                .iter()
                .map(test_shape)
                .collect::<Result<Vec<_>>>()?;
            if shapes.is_empty() {
                return Err(Error(format!(
                    "line {}: '{}' block cannot be empty",
                    n.line, n.name
                )));
            }
            regions.push(Region {
                exclude,
                inclusion,
                shape: Shape::Intersection(shapes),
            });
        } else if let Some(t) = transform(n)? {
            // A locate in fill context changes the frame but not boolean grouping.
            let Shape::Region(children) = region_shape(&n.children)? else {
                unreachable!()
            };
            regions.extend(children.into_iter().map(|mut r| {
                r.shape = Shape::Transform(t, Box::new(r.shape));
                r
            }));
        } else {
            regions.push(Region {
                exclude: false,
                inclusion: Inclusion::Inherited,
                shape: test_shape(n)?,
            });
        }
    }
    Ok(Shape::Region(regions))
}

fn test_shape(n: &Node) -> Result<Shape> {
    #[cfg(feature = "lua")]
    if n.name == "shape"
        && let Some(Value::Callback(callback)) = n.args.first()
    {
        return Ok(Shape::Primitive(geometry::Primitive::Lua(callback.clone())));
    }
    if let Some(t) = transform(n)? {
        return Ok(Shape::Transform(
            t,
            Box::new(Shape::Intersection(
                n.children.iter().map(test_shape).collect::<Result<_>>()?,
            )),
        ));
    }
    if let Some(p) = geometry::primitive(n)? {
        return Ok(Shape::Primitive(p));
    }
    if within(&n.name).is_some() {
        return region_shape(std::slice::from_ref(n));
    }
    let name = n.name.as_str();
    if matches!(name, "revolve_xy" | "revolve_yz" | "revolve_zx") {
        let (start, end) = if n.args.len() > 1 {
            (n.num(0, 0.)?, n.num(1, 360.)?)
        } else {
            (0., n.num(0, 360.)?)
        };
        return Ok(Shape::Revolve {
            shape: Box::new(region_shape(&n.children)?),
            start: start.min(end),
            end: start.max(end),
            axis: match name {
                "revolve_xy" => 0,
                "revolve_yz" => 1,
                _ => 2,
            },
        });
    }
    if matches!(name, "extrude_xy" | "extrude_yz" | "extrude_zx") {
        let (a, b) = if n.args.is_empty() {
            (f64::NEG_INFINITY, f64::INFINITY)
        } else {
            (n.num(0, 0.)?, n.num(1, 0.)?)
        };
        return Ok(Shape::Extrude {
            shape: Box::new(region_shape(&n.children)?),
            min: a.min(b),
            max: a.max(b),
            axis: match name {
                "extrude_xy" => 2,
                "extrude_yz" => 0,
                _ => 1,
            },
        });
    }
    Err(n.unsupported())
}

#[cfg(test)]
mod sdf_tests {
    use super::*;
    use simrep_geom::SignedDistance;

    #[test]
    fn source_segment_bounds_enclose_ordered_csg_and_edge_fills() {
        for text in [
            "e(1){fill{within{cylinder3d(1,2,3,8,5,7,1.75)}}}
             e(2){fill{within{box3d(-2,-1,0,2,3,4)}}}
             e(1){fill{within{box3d(9,1,1,11,3,3)}}}",
            "e(1){fill{within{box3d(-inf,-4,-4,inf,4,4)}
                       notin{cylinder3d(-inf,0,0,inf,0,0,1.75)}}}",
            "e(1){edge_fill{within{box3d(0,0,0,3,3,3)}}}",
        ] {
            let gem = parse_gem(&format!("pa_define(13,13,13,planar)\n{text}")).unwrap();
            for sdf in gem.electrode_sdfs().unwrap() {
                for i in 0..97 {
                    let from = [
                        (i * 13 % 41) as f64 * 0.31 - 3.0,
                        (i * 19 % 43) as f64 * 0.23 - 4.0,
                        (i * 23 % 47) as f64 * 0.19 - 3.0,
                    ];
                    let to = [
                        (i * 17 % 31) as f64 * 0.37 - 2.0,
                        (i * 29 % 37) as f64 * 0.21 - 3.0,
                        (i * 31 % 41) as f64 * 0.17 - 2.0,
                    ];
                    let bounds = sdf.segment_distance_bounds(from, to).unwrap();
                    for step in 0..=64 {
                        let p = std::array::from_fn(|a| {
                            from[a] + (to[a] - from[a]) * step as f64 / 64.0
                        });
                        let d = sdf.distance(p);
                        assert!(
                            bounds[0] <= d + 1e-12 && d <= bounds[1] + 1e-12,
                            "{from:?}..{to:?}: {bounds:?} excludes {d}"
                        );
                    }
                }
            }
        }
    }
}

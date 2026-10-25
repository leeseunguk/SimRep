use crate::{GridSpec, Mesh, Part, Point, invalid, repair};
use mcubes::{MarchingCubes, MeshSide};
use simrep_core::{PotentialArray, Symmetry};
use std::collections::{BTreeMap, HashMap};
use std::io;

/// Tessellation and extent of surfaces made from a two-dimensional PA.
#[derive(Clone, Copy, Debug)]
pub struct SurfaceOptions {
    pub angular_segments: usize,
    /// Sweep from the positive y direction toward positive z, in (0, 360].
    pub sweep_degrees: f64,
    /// Planar extrusion centered on origin_mm[2].
    pub extrusion_depth_mm: f64,
}

impl Default for SurfaceOptions {
    fn default() -> Self {
        Self {
            angular_segments: 64,
            sweep_degrees: 360.0,
            extrusion_depth_mm: 1.0,
        }
    }
}

impl SurfaceOptions {
    pub(crate) fn validate(self) -> io::Result<()> {
        if self.angular_segments < 3
            || !self.sweep_degrees.is_finite()
            || self.sweep_degrees <= 0.0
            || self.sweep_degrees > 360.0
            || !self.extrusion_depth_mm.is_finite()
            || self.extrusion_depth_mm <= 0.0
        {
            return Err(invalid(
                "surfaces require >=3 angular segments, sweep in (0,360], and positive finite extrusion depth",
            ));
        }
        Ok(())
    }
}

pub(crate) fn pa_grid(pa: &PotentialArray, origin_mm: Point) -> io::Result<GridSpec> {
    let grid = GridSpec {
        dims: [pa.nx, pa.ny, pa.nz],
        origin_mm,
        spacing_mm: [pa.dx_mm, pa.dy_mm, pa.dz_mm],
    };
    let count = grid.validate()?;
    if pa.potentials.len() != count
        || pa.electrodes.len() != count
        || pa.potentials.iter().any(|v| !v.is_finite())
    {
        return Err(invalid("PA buffer lengths or potentials are invalid"));
    }
    Ok(grid)
}

/// Expand reflection planes through the first node of each mirrored axis.
/// Cylindrical y is a radius, not a second copy of the meridional plane.
pub(crate) fn expanded_pa(
    pa: &PotentialArray,
    origin_mm: Point,
) -> io::Result<(PotentialArray, GridSpec)> {
    let mut grid = pa_grid(pa, origin_mm)?;
    if pa.symmetry == Symmetry::Cylindrical && (pa.nz != 1 || pa.ny < 2 || pa.anti_mirrors.y) {
        return Err(invalid(
            "cylindrical export requires nz=1, ny>=2 and an even radial axis",
        ));
    }
    let mirrors = [pa.mirrors.x, pa.mirrors.y, pa.mirrors.z];
    let anti = [pa.anti_mirrors.x, pa.anti_mirrors.y, pa.anti_mirrors.z];
    if (0..3).any(|axis| anti[axis] && !mirrors[axis]) {
        return Err(invalid("anti-mirrors require the matching mirror flag"));
    }
    let source_dims = grid.dims;
    let mut offset = [0; 3];
    for axis in 0..3 {
        if mirrors[axis] && !(axis == 1 && pa.symmetry == Symmetry::Cylindrical) {
            offset[axis] = source_dims[axis] - 1;
            grid.dims[axis] = offset[axis]
                .checked_mul(2)
                .and_then(|n| n.checked_add(1))
                .ok_or_else(|| invalid("mirrored grid dimensions overflow"))?;
            grid.origin_mm[axis] -= offset[axis] as f64 * grid.spacing_mm[axis];
        }
    }
    let count = grid.validate()?;
    let mut result = pa.clone();
    [result.nx, result.ny, result.nz] = grid.dims;
    result.potentials = Vec::with_capacity(count);
    result.electrodes = Vec::with_capacity(count);
    for z in 0..result.nz {
        for y in 0..result.ny {
            for x in 0..result.nx {
                let coords = [x, y, z];
                let source: [usize; 3] = std::array::from_fn(|a| coords[a].abs_diff(offset[a]));
                let i = (source[2] * pa.ny + source[1]) * pa.nx + source[0];
                let sign = (0..3).filter(|&a| anti[a] && coords[a] < offset[a]).count();
                if (0..3).any(|a| anti[a] && source[a] == 0) && pa.potentials[i] != 0.0 {
                    return Err(invalid("anti-mirror potential must be zero on its plane"));
                }
                result.potentials.push(if sign % 2 == 0 {
                    pa.potentials[i]
                } else {
                    -pa.potentials[i]
                });
                result.electrodes.push(pa.electrodes[i]);
            }
        }
    }
    result.mirrors = Default::default();
    result.anti_mirrors = Default::default();
    Ok((result, grid))
}

/// Extract exposed faces of electrode voxels (half a cell around each PA
/// node), grouped by electrode potential/number. Internal equal-value faces
/// are omitted, and different electrode values retain their interface.
pub fn electrode_surface(
    pa: &PotentialArray,
    origin_mm: Point,
    electrode: Option<f64>,
) -> io::Result<Mesh> {
    electrode_surface_with_options(pa, origin_mm, electrode, SurfaceOptions::default())
}

/// Electrode voxels in 3D, revolved x-r voxels in cylindrical 2D, or
/// z-extruded planar 2D voxels. Mirror domains are expanded before extraction.
pub fn electrode_surface_with_options(
    pa: &PotentialArray,
    origin_mm: Point,
    electrode: Option<f64>,
    options: SurfaceOptions,
) -> io::Result<Mesh> {
    options.validate()?;
    if electrode.is_some_and(|e| !e.is_finite()) {
        return Err(invalid("electrode filter must be finite"));
    }
    let (pa, grid) = expanded_pa(pa, origin_mm)?;
    if pa.nz == 1 {
        return crate::sweep::electrodes(&pa, grid, electrode, options);
    }
    electrode_voxels(&pa, grid.origin_mm, electrode)
}

fn electrode_voxels(
    pa: &PotentialArray,
    origin_mm: Point,
    electrode: Option<f64>,
) -> io::Result<Mesh> {
    let grid = pa_grid(pa, origin_mm)?;
    if pa.symmetry != Symmetry::Planar || pa.nz < 2 {
        return Err(invalid(
            "electrode surface extraction requires a planar 3D PA",
        ));
    }
    if electrode.is_some_and(|e| !e.is_finite()) {
        return Err(invalid("electrode filter must be finite"));
    }
    type Builder = (Part, HashMap<[usize; 3], usize>);
    let mut parts: BTreeMap<u64, Builder> = BTreeMap::new();
    let faces = [
        [[0, 0, 0], [0, 0, 1], [0, 1, 1], [0, 1, 0]],
        [[1, 0, 0], [1, 1, 0], [1, 1, 1], [1, 0, 1]],
        [[0, 0, 0], [1, 0, 0], [1, 0, 1], [0, 0, 1]],
        [[0, 1, 0], [0, 1, 1], [1, 1, 1], [1, 1, 0]],
        [[0, 0, 0], [0, 1, 0], [1, 1, 0], [1, 0, 0]],
        [[0, 0, 1], [1, 0, 1], [1, 1, 1], [0, 1, 1]],
    ];
    for z in 0..pa.nz {
        for y in 0..pa.ny {
            for x in 0..pa.nx {
                let index = (z * pa.ny + y) * pa.nx + x;
                let value = pa.potentials[index];
                if !pa.electrodes[index] || electrode.is_some_and(|e| e != value) {
                    continue;
                }
                let key = if value == 0.0 { 0 } else { value.to_bits() };
                let (part, vertices) = parts.entry(key).or_insert_with(|| {
                    (
                        Part {
                            name: format!("electrode-{value}"),
                            ..Part::default()
                        },
                        HashMap::new(),
                    )
                });
                let coords = [x, y, z];
                for (face_index, corners) in faces.iter().enumerate() {
                    let axis = face_index / 2;
                    let mut neighbor = coords;
                    let coordinate = if face_index % 2 == 0 {
                        coords[axis].checked_sub(1)
                    } else {
                        coords[axis].checked_add(1)
                    };
                    if let Some(coordinate) = coordinate {
                        neighbor[axis] = coordinate;
                        if let Some(i) = pa.index(neighbor[0], neighbor[1], neighbor[2])
                            && pa.electrodes[i]
                            && pa.potentials[i] == value
                        {
                            continue;
                        }
                    }
                    let quad = corners.map(|corner| {
                        let lattice = std::array::from_fn(|i| coords[i] + corner[i]);
                        *vertices.entry(lattice).or_insert_with(|| {
                            part.vertices.push(std::array::from_fn(|i| {
                                grid.origin_mm[i] + (lattice[i] as f64 - 0.5) * grid.spacing_mm[i]
                            }));
                            part.vertices.len() - 1
                        })
                    });
                    part.triangles.push([quad[0], quad[1], quad[2]]);
                    part.triangles.push([quad[0], quad[2], quad[3]]);
                }
            }
        }
    }
    let result = Mesh {
        parts: parts.into_values().map(|(part, _)| part).collect(),
    };
    result.validate()?;
    Ok(result)
}

/// PA-aware isopotential extraction, including mirrors and two-dimensional
/// marching-squares contours revolved or extruded into 3D.
pub fn pa_isosurface(
    pa: &PotentialArray,
    origin_mm: Point,
    level: f64,
    options: SurfaceOptions,
) -> io::Result<Mesh> {
    options.validate()?;
    let (pa, grid) = expanded_pa(pa, origin_mm)?;
    if pa.nz == 1 {
        crate::sweep::isopotential(&pa, grid, level, options)
    } else {
        isosurface(&pa.potentials, grid, level)
    }
}

/// Classic marching cubes, with linear edge interpolation. Geometry is in
/// mm and clipped at the grid domain. Subtract the isovalue in f64 before
/// converting normalized differences to the algorithm's f32 samples.
pub fn isosurface(values: &[f64], grid: GridSpec, level: f64) -> io::Result<Mesh> {
    let count = grid.validate()?;
    if grid.dims.iter().any(|&n| n < 2)
        || values.len() != count
        || !level.is_finite()
        || values.iter().any(|v| !v.is_finite())
    {
        return Err(invalid(
            "isosurface requires finite samples and at least 2 nodes per axis",
        ));
    }
    let scale = values.iter().map(|v| (v - level).abs()).fold(0.0, f64::max);
    if !scale.is_finite() {
        return Err(invalid("potential range overflows f64"));
    }
    if scale == 0.0 {
        return Err(invalid("constant grid has no isosurface"));
    }
    let normalized: Vec<_> = values
        .iter()
        .map(|v| ((v - level) / scale) as f32)
        .collect();
    let [nx, ny, nz] = grid.dims;
    let marcher = MarchingCubes::new(
        (nx, ny, nz),
        (1.0, 1.0, 1.0),
        (1.0, 1.0, 1.0),
        Default::default(),
        normalized,
        0.0,
    )?;
    let generated = marcher.generate(MeshSide::OutsideOnly);
    if generated.indices.is_empty() {
        return Err(invalid("isovalue does not intersect this grid"));
    }
    let vertices = generated
        .vertices
        .iter()
        .map(|v| {
            let position = [v.posit.x, v.posit.y, v.posit.z];
            std::array::from_fn(|i| grid.origin_mm[i] + f64::from(position[i]) * grid.spacing_mm[i])
        })
        .collect();
    let mut part = Part {
        name: format!("isopotential-{level}"),
        vertices,
        triangles: generated
            .indices
            .chunks_exact(3)
            .map(|v| [v[0], v[1], v[2]])
            .collect(),
    };
    repair(&mut part)?;
    Ok(Mesh { parts: vec![part] })
}

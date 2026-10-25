//! SIMION sparse surface-offset sidecars (SIMEXP, FHDR version 1).
use std::io::{self, Write};
use std::path::Path;

use crate::geometry::RasterMode;
use crate::{Gem, Rasterized};

/// Fractional intersections of grid edges with electrode surfaces.
/// Each entry contains the lower endpoint's flat index and an offset in (0,1).
#[derive(Clone, Debug, Default)]
pub struct SurfaceOffsets {
    pub axes: [Vec<(u64, f64)>; 3],
}

impl SurfaceOffsets {
    pub fn write(&self, mut out: impl Write) -> io::Result<()> {
        out.write_all(b"SIMEXP\xff\x7f")?;
        out.write_all(&4u32.to_le_bytes())?;
        out.write_all(b"FHDR")?;
        out.write_all(&1u32.to_le_bytes())?;
        for (axis, entries) in self.axes.iter().enumerate() {
            out.write_all(&0u32.to_le_bytes())?;
            out.write_all(&u32::MAX.to_le_bytes())?;
            out.write_all(&(entries.len() as u64 * 16).to_le_bytes())?;
            out.write_all([b"XOFF", b"YOFF", b"ZOFF"][axis])?;
            for &(index, offset) in entries {
                out.write_all(&index.to_le_bytes())?;
                out.write_all(&offset.to_le_bytes())?;
            }
        }
        out.write_all(&0u32.to_le_bytes())?;
        out.flush()
    }
}

impl Rasterized {
    /// Write the fractional geometry companion, if this GEM requested it.
    pub fn write_surfaces(&self, pa_path: impl AsRef<Path>) -> io::Result<()> {
        let path = pa_path.as_ref().with_extension("pa-surf");
        if let Some(surface) = &self.surfaces
            && surface.axes.iter().any(|axis| !axis.is_empty())
        {
            surface.write(std::io::BufWriter::new(std::fs::File::create(path)?))?;
        } else {
            match std::fs::remove_file(path) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }
}

impl Gem {
    pub(crate) fn surface_offsets(&self, pa: &simrep_core::PotentialArray) -> SurfaceOffsets {
        let scale = if self.grid_units {
            [1.; 3]
        } else {
            self.spacing_mm
        };
        let mut surfaces = SurfaceOffsets::default();
        let [nx, ny, nz] = self.dimensions;
        for axis in 0..3 {
            let stride = [1, nx, nx * ny][axis];
            for z in 0..nz {
                for y in 0..ny {
                    for x in 0..nx {
                        let xyz = [x, y, z];
                        if xyz[axis] + 1 == self.dimensions[axis] {
                            continue;
                        }
                        let index = x + nx * (y + ny * z);
                        let left = pa.electrodes[index];
                        if left == pa.electrodes[index + stride] {
                            continue;
                        }
                        let p: [f64; 3] = std::array::from_fn(|i| xyz[i] as f64 * scale[i]);
                        let (mut low, mut high) = (0., 1.);
                        for _ in 0..10 {
                            let middle = (low + high) / 2.;
                            let mut q = p;
                            q[axis] += middle * scale[axis];
                            let inside = self
                                .fills
                                .iter()
                                .rev()
                                .find(|fill| {
                                    fill.shape.contains(
                                        q,
                                        scale.map(|s| s * 0.00005),
                                        true,
                                        RasterMode {
                                            auto: true,
                                            fractional: true,
                                            electrode: fill.electrode,
                                        },
                                    )
                                })
                                .is_some_and(|fill| fill.electrode);
                            if inside == left {
                                low = middle;
                            } else {
                                high = middle;
                            }
                        }
                        let offset = if left { low } else { high };
                        if offset > 0. && offset < 1. {
                            surfaces.axes[axis].push((index as u64, offset));
                        }
                    }
                }
            }
        }
        surfaces
    }
}

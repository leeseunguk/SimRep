use super::*;
use crate::physics::magnetic::{MagneticField, OutsideMap};

/// Header: coil-map pointer (two 16-bit words), imported-map count.
/// Per imported map: origin(3), axes(9), scale, dims(3), spacing(3),
/// cylindrical, outside policy, mirrors, anti-mirror bits(3), jet pointers(6).
pub(super) fn append(
    magnetic: &MagneticField,
    electric: &InstanceField,
    data: &mut Vec<f64>,
    coils: u32,
) -> GpuResult<u32> {
    if magnetic.maps.is_empty() && coils == 0 {
        return Ok(0);
    }
    data.resize(data.len().max(4), 0.0);
    let base = u32::try_from(data.len())?;
    let header = data.len();
    data.resize(header + 3 + 31 * magnetic.maps.len(), 0.0);
    data[header] = f64::from(coils & 0xffff);
    data[header + 1] = f64::from(coils >> 16);
    data[header + 2] = magnetic.maps.len() as f64;
    for (i, map) in magnetic.maps.iter().enumerate() {
        let meta = header + 3 + 31 * i;
        let placement = &map.placement;
        for a in 0..3 {
            data[meta + a] = placement.origin_mm[a] - electric.origin_mm[a];
            data[meta + 3 + 3 * a..meta + 6 + 3 * a].copy_from_slice(&placement.axes[a]);
        }
        data[meta + 12] = placement.scale;
        let pa = map.grids[0].array();
        for (a, d) in [pa.nx, pa.ny, pa.nz].into_iter().enumerate() {
            data[meta + 13 + a] = d as f64;
        }
        data[meta + 16..meta + 19].copy_from_slice(&[pa.dx_mm, pa.dy_mm, pa.dz_mm]);
        data[meta + 19] = f64::from(map.axisymmetric);
        data[meta + 20] = match placement.outside {
            OutsideMap::Error => 0.0,
            OutsideMap::Zero => 1.0,
            OutsideMap::Clamp => 2.0,
        };
        let mask = placement
            .mirrors
            .iter()
            .enumerate()
            .fold(0u32, |m, (a, p)| m | (u32::from(p.is_some()) << a));
        data[meta + 21] = f64::from(mask);
        for c in 0..3 {
            let anti = placement
                .mirrors
                .iter()
                .enumerate()
                .fold(0u32, |m, (a, p)| {
                    m | (u32::from(p.is_some_and(|p| p[c] < 0)) << a)
                });
            data[meta + 22 + c] = f64::from(anti);
            let (_, _, jets, _) = map.grids[c].gpu_physics_data(true);
            let offset = u32::try_from(data.len())?;
            data[meta + 25 + 2 * c] = f64::from(offset & 0xffff);
            data[meta + 26 + 2 * c] = f64::from(offset >> 16);
            data.extend_from_slice(&jets);
        }
    }
    Ok(base)
}

use super::*;
use crate::physics::magnetic::{CoilMapPlan, MagneticField};

/// Coaxial hoops admit a two-component x/r map of the full workbench vector.
/// Domain construction and calibration live in physics/magnetic/coil_map.rs;
/// here only the upload layout and the calibration report remain.
pub(super) fn append(
    magnetic: &MagneticField,
    electric: &InstanceField,
    data: &mut Vec<f64>,
) -> GpuResult<u32> {
    let Some(plan) = CoilMapPlan::new(magnetic, electric.bounds_mm) else {
        return Ok(0);
    };
    let spacing: f64 = 0.0625;
    #[cfg(test)]
    let spacing = std::env::var("SIMREP_GPU_COIL_MAP_STEP")
        .ok()
        .map(|v| v.parse::<f64>())
        .transpose()?
        .unwrap_or(spacing);
    if spacing <= 0.0 || !spacing.is_finite() {
        return Err("coil map spacing must be finite and positive".into());
    }
    let domains = plan.build(&magnetic.coils, spacing)?;
    let base = u32::try_from(data.len())?;
    let header = data.len();
    data.resize(header + 7 + 12 * domains.len(), 0.0);
    data[header] = domains.len() as f64;
    for a in 0..3 {
        data[header + 1 + a] = plan.origin[a] - electric.origin_mm[a];
        data[header + 4 + a] = plan.axis[a];
    }
    for (block, domain) in domains.iter().enumerate() {
        let meta = header + 7 + block * 12;
        data[meta..meta + 4].copy_from_slice(&[
            domain.lo,
            0.0,
            domain.spacing[0],
            domain.spacing[1],
        ]);
        let mut sampled = vec![[0.0; 2]; 512];
        for component in 0..2 {
            let (cells, degrees, coefficients) = domain.physics_data(component);
            data[meta + 4] = cells[0] as f64;
            data[meta + 5] = cells[1] as f64;
            data[meta + 6] = degrees[0] as f64;
            data[meta + 7] = degrees[1] as f64;
            let offset = u32::try_from(data.len())?;
            data[meta + 8 + component * 2] = f64::from(offset & 0xffff);
            data[meta + 9 + component * 2] = f64::from(offset >> 16);
            data.extend(coefficients.iter().copied());
            for (sample, q) in sampled.iter_mut().zip(domain.probes()) {
                sample[component] = domain.grid_sample(component, q);
            }
        }
        let calibration = domain.calibrate(&plan, &magnetic.coils, &sampled);
        let (dx, dr) = (domain.spacing[0], domain.spacing[1]);
        eprintln!(
            "COIL_MAP_CALIBRATION,block={block},dx_mm={dx:.9},dr_mm={dr:.9},nodes={},probes=512,sampled_max_abs_gauss={:.12e},sampled_max_relative={:.12e}",
            calibration.nodes, calibration.max_absolute_gauss, calibration.max_relative
        );
    }
    Ok(base)
}

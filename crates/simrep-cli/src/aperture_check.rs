use std::{fs::File, path::PathBuf};

use clap::Args;
use serde_json::{Value, json};
use simrep_core::{PotentialArray, Symmetry};
use simrep_fly::surface::{SurfaceAccuracy, SurfaceModel};
use simrep_solve::gpu::GpuResult;

#[derive(Args)]
pub struct Options {
    /// Unrefined PA#.
    #[arg(long)]
    pa: PathBuf,
    /// Exact surface sidecar (defaults to the PA sibling).
    #[arg(long)]
    cuts: Option<PathBuf>,
    /// Axis-aligned probe endpoints in source coordinates, x0,y0,z0,x1,y1,z1 (mm).
    #[arg(long, required = true, allow_hyphen_values = true)]
    line: Vec<String>,
    /// Maximum sampling step in mm (default: axis spacing / 64).
    #[arg(long)]
    step_mm: Option<f64>,
}

fn parse_line(s: &str) -> Result<[f64; 6], String> {
    let v: Vec<f64> = s
        .split(',')
        .map(|x| {
            x.parse::<f64>()
                .map_err(|_| "line needs six finite numbers".to_owned())
        })
        .collect::<Result<_, _>>()?;
    if v.len() != 6 || v.iter().any(|x| !x.is_finite()) {
        return Err("line needs six finite numbers".into());
    }
    Ok(v.try_into().unwrap())
}

fn measure(
    a: &PotentialArray,
    origin: [f64; 3],
    surface: &SurfaceModel,
    line: [f64; 6],
    step: Option<f64>,
) -> Result<Value, String> {
    let from = [line[0], line[1], line[2]];
    let to = [line[3], line[4], line[5]];
    let axes: Vec<_> = (0..3).filter(|&i| from[i] != to[i]).collect();
    if axes.len() != 1 {
        return Err("line must be nonzero and axis-aligned".into());
    }
    let axis = axes[0];
    let dims = [a.nx, a.ny, a.nz];
    let spacing = [a.dx_mm, a.dy_mm, a.dz_mm];
    if dims[axis] <= 1 {
        return Err("line axis must have at least two nodes".into());
    }
    if a.symmetry == Symmetry::Cylindrical
        && (from[2] != 0.0 || to[2] != 0.0 || from[1] < 0.0 || to[1] < 0.0)
    {
        return Err("cylindrical probes must use the stored x/r plane (z=0, r>=0)".into());
    }
    for i in 0..3 {
        let low = origin[i];
        let high = low + (dims[i] - 1) as f64 * spacing[i];
        if from[i].min(to[i]) < low - 1e-9 || from[i].max(to[i]) > high + 1e-9 {
            return Err("line must lie within the PA grid".into());
        }
        if i != axis
            && ((from[i] - low) / spacing[i] - ((from[i] - low) / spacing[i]).round()).abs() > 1e-8
        {
            return Err("line must pass through grid nodes on fixed axes".into());
        }
    }
    let h = spacing[axis];
    let step = step.unwrap_or(h / 64.0);
    if !step.is_finite() || step <= 0.0 {
        return Err("step-mm must be finite and positive".into());
    }
    let length = (to[axis] - from[axis]).abs();
    let n = (length / step).ceil().max(1.0);
    if n > 10_000_000.0 {
        return Err("probe requires more than 10000000 samples; increase step-mm".into());
    }
    let n = n as usize;
    let point = |t: f64| std::array::from_fn(|i| from[i] + t * (to[i] - from[i]));
    let open = |t: f64| surface.membership(point(t)).is_none();
    if !open(0.5) {
        return Err("line midpoint is on or inside an electrode".into());
    }
    let mut previous = open(0.0);
    let mut start = previous.then_some(0.0);
    let mut intervals = Vec::new();
    for i in 1..=n {
        let t = i as f64 / n as f64;
        let now = open(t);
        if now != previous {
            let mut lo = (i - 1) as f64 / n as f64;
            let mut hi = t;
            while (hi - lo) * length > 1e-6 {
                let mid = (lo + hi) * 0.5;
                if open(mid) == previous {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            let boundary = (lo + hi) * 0.5;
            if now {
                start = Some(boundary);
            } else {
                intervals.push((start.take().unwrap(), boundary));
            }
        }
        previous = now;
    }
    if let Some(start) = start {
        intervals.push((start, 1.0));
    }
    let &(start, end) = intervals
        .iter()
        .find(|&&(s, e)| s < 0.5 && e > 0.5)
        .ok_or("line midpoint is not inside a sampled vacuum interval")?;
    if start == 0.0 || end == 1.0 {
        return Err("midpoint vacuum interval must be bounded by surfaces on both sides".into());
    }
    let mut fixed = [0usize; 3];
    for i in 0..3 {
        if i != axis {
            fixed[i] = ((from[i] - origin[i]) / spacing[i]).round() as usize;
        }
    }
    // Select the staircase opening at the midpoint independently of the exact
    // interval: open grid nodes can lie outside the continuous aperture.
    let midpoint = (from[axis] + to[axis]) * 0.5;
    let center = ((midpoint - origin[axis]) / h).round() as usize;
    let is_open = |j: usize| {
        let coordinate = origin[axis] + j as f64 * h;
        if coordinate < from[axis].min(to[axis]) || coordinate > from[axis].max(to[axis]) {
            return false;
        }
        let mut index = fixed;
        index[axis] = j;
        !a.electrodes[a.index(index[0], index[1], index[2]).unwrap()]
    };
    let (staircase, effective) = if is_open(center) {
        let mut first = center;
        let mut last = center;
        while first > 0 && is_open(first - 1) {
            first -= 1;
        }
        while last + 1 < dims[axis] && is_open(last + 1) {
            last += 1;
        }
        let width = (last - first + 1) as f64 * h;
        (width, width)
    } else {
        (0.0, 0.0)
    };
    let exact = (end - start) * length;
    let diff = staircase - exact;
    let mut warnings = Vec::new();
    if (diff / h).abs() > 0.5 {
        warnings.push("staircase differs from exact width by more than 0.5 cell");
    }
    if staircase / h < 2.5 {
        warnings.push("under-resolved slit: staircase width is less than 2.5 cells");
    }
    let vacuum_intervals: Vec<_> = intervals
        .iter()
        .map(|&(s, e)| json!({"start": point(s), "end": point(e), "width_mm": (e-s)*length}))
        .collect();
    Ok(
        json!({"line":line,"staircase_mm":staircase,"effective_mm":effective,"exact_mm":exact,"diff_mm":diff,"diff_cells":diff/h,"vacuum_intervals":vacuum_intervals,"warnings":warnings}),
    )
}

pub fn run(options: Options) -> GpuResult<()> {
    let pa = simrep_io::read_pa(&options.pa)?;
    let cuts = options
        .cuts
        .unwrap_or_else(|| options.pa.with_extension("cuts.json"));
    let metadata: Value = serde_json::from_reader(File::open(&cuts)?)?;
    let surface = SurfaceModel::from_reader(File::open(&cuts)?)?;
    if surface.accuracy() != SurfaceAccuracy::SourceGeometry {
        return Err("cuts must contain source geometry for exact aperture measurements".into());
    }
    let dims = [pa.array.nx, pa.array.ny, pa.array.nz];
    let spacing = [pa.array.dx_mm, pa.array.dy_mm, pa.array.dz_mm];
    let origin: [f64; 3] = serde_json::from_value(metadata["origin_mm"].clone())?;
    let side_dims: [usize; 3] = serde_json::from_value(metadata["dims"].clone())?;
    let side_spacing: [f64; 3] = serde_json::from_value(metadata["spacing_mm"].clone())?;
    let symmetry = if pa.array.symmetry == Symmetry::Cylindrical {
        "cylindrical"
    } else {
        "planar"
    };
    let side_mirrors: [bool; 3] = match metadata.get("mirrors") {
        Some(value) => serde_json::from_value(value.clone())?,
        None => [false; 3],
    };
    let mirrors = [pa.array.mirrors.x, pa.array.mirrors.y, pa.array.mirrors.z];
    if dims != side_dims
        || spacing != side_spacing
        || metadata["symmetry"] != symmetry
        || mirrors != side_mirrors
    {
        return Err("cuts sidecar does not match PA dimensions, spacing or symmetry".into());
    }
    let mut apertures = Vec::new();
    for line in options.line {
        let result = measure(
            &pa.array,
            origin,
            &surface,
            parse_line(&line)?,
            options.step_mm,
        )?;
        println!(
            "line {line}: staircase {} mm, exact {} mm, difference {} mm ({} cells)",
            result["staircase_mm"], result["exact_mm"], result["diff_mm"], result["diff_cells"]
        );
        for warning in result["warnings"].as_array().unwrap() {
            eprintln!("warning: {}", warning.as_str().unwrap());
        }
        apertures.push(result);
    }
    crate::json::set_summary(json!({"apertures":apertures}));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn slit(cylindrical: bool) -> (PotentialArray, SurfaceModel) {
        let mut pa = PotentialArray::new(11, 3, 1);
        pa.dx_mm = 0.5;
        pa.dy_mm = 0.5;
        pa.dz_mm = 0.5;
        if cylindrical {
            pa.symmetry = Symmetry::Cylindrical;
        }
        for y in 0..3 {
            for x in 0..11 {
                let i = pa.index(x, y, 0).unwrap();
                let px = x as f64 * 0.5;
                pa.electrodes[i] = !(1.35..3.65).contains(&px);
            }
        }
        let surface = SurfaceModel::from_sdfs(
            vec![
                (1, Arc::new(|p: [f64; 3]| p[0] - 1.35)),
                (2, Arc::new(|p: [f64; 3]| 3.65 - p[0])),
            ],
            1e-8,
        )
        .unwrap();
        (pa, surface)
    }

    #[test]
    fn slit_widths_and_json_schema() {
        for cylindrical in [false, true] {
            let (pa, surface) = slit(cylindrical);
            let result = measure(
                &pa,
                [0.0; 3],
                &surface,
                [0.0, 0.5, 0.0, 5.0, 0.5, 0.0],
                None,
            )
            .unwrap();
            assert_eq!(result["staircase_mm"], 2.5);
            assert_eq!(result["effective_mm"], 2.5);
            assert!((result["exact_mm"].as_f64().unwrap() - 2.3).abs() < 1e-6);
            assert!((result["diff_mm"].as_f64().unwrap() - 0.2).abs() < 1e-6);
            assert!((result["diff_cells"].as_f64().unwrap() - 0.4).abs() < 2e-6);
            assert!(result["line"].is_array());
            assert_eq!(result["vacuum_intervals"].as_array().unwrap().len(), 1);
        }
    }

    #[test]
    fn rejects_diagonal_and_midpoint_in_electrode() {
        let (pa, surface) = slit(false);
        assert!(
            measure(
                &pa,
                [0.0; 3],
                &surface,
                [0.0, 0.0, 0.0, 5.0, 0.5, 0.0],
                None
            )
            .is_err()
        );
        assert!(
            measure(
                &pa,
                [0.0; 3],
                &surface,
                [0.0, 0.0, 0.0, 1.0, 0.0, 0.0],
                None
            )
            .is_err()
        );
    }
}

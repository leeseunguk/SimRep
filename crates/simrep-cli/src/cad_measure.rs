use std::path::PathBuf;

use clap::Args;
use simrep_solve::gpu::GpuResult;

#[derive(Args)]
pub struct MeasureOptions {
    /// STEP or IGES model; lengths are converted to millimeters.
    file: PathBuf,
    /// XCAF path or zero-based body index, optionally followed by :faces=0,1.
    #[arg(long, num_args = 2, value_names = ["SEL_A", "SEL_B"],
        required_unless_present = "all_pairs", conflicts_with = "all_pairs")]
    gap: Vec<String>,
    /// Measure every unique pair and return symmetric distance/touch matrices.
    #[arg(long)]
    all_pairs: bool,
    /// Absolute allowed exact/tessellation discrepancy in mm.
    #[arg(long, default_value = "0.01", value_parser = super::parse_tolerance)]
    cad_tolerance: f64,
    /// Touching means distance <= this many mm, not proof of electrical contact.
    #[arg(long, default_value = "1e-7", value_parser = parse_touch_tolerance)]
    touch_tolerance: f64,
}

fn parse_touch_tolerance(value: &str) -> Result<f64, String> {
    let tolerance = value
        .parse::<f64>()
        .map_err(|_| "touch tolerance must be a number")?;
    if !tolerance.is_finite() || tolerance < 0.0 {
        return Err("touch tolerance must be nonnegative and finite".into());
    }
    Ok(tolerance)
}

#[cfg(not(feature = "cad"))]
pub fn run(_args: MeasureOptions) -> GpuResult<()> {
    Err("CAD measurement requires cargo build -p simrep-cli --features cad; see docs/step-import.md".into())
}

#[cfg(feature = "cad")]
pub fn run(args: MeasureOptions) -> GpuResult<()> {
    use serde_json::json;
    use simrep_cad::{MeasureOptions as CadOptions, MeasurementModel};

    let model = MeasurementModel::read(&args.file)?;
    let options = CadOptions {
        cad_tolerance_mm: args.cad_tolerance,
        touch_tolerance_mm: args.touch_tolerance,
    };
    let measurements = if args.all_pairs {
        model.all_pairs(options)?
    } else {
        vec![model.gap(&args.gap[0], &args.gap[1], options)?]
    };
    let parts: Vec<_> = model
        .parts()
        .iter()
        .map(|p| {
            json!({
                "index": p.index, "name": p.name, "face_count": p.face_count,
            })
        })
        .collect();
    let (distance_matrix, touching_matrix) = if args.all_pairs {
        let n = model.parts().len();
        let mut distances = vec![vec![0.0; n]; n];
        let mut touching = vec![vec![true; n]; n];
        for gap in &measurements {
            distances[gap.part_a][gap.part_b] = gap.min_distance_mm;
            distances[gap.part_b][gap.part_a] = gap.min_distance_mm;
            touching[gap.part_a][gap.part_b] = gap.touching;
            touching[gap.part_b][gap.part_a] = gap.touching;
        }
        (json!(distances), json!(touching))
    } else {
        (serde_json::Value::Null, serde_json::Value::Null)
    };
    let measurements: Vec<_> = measurements.iter().map(|gap| {
        let name_a = &model.parts()[gap.part_a].name;
        let name_b = &model.parts()[gap.part_b].name;
        println!("{name_a:?} faces {:?} -> {name_b:?} faces {:?}: {:.12} mm, touching={}, tessellation={:.12} mm (error {:.3e} mm)",
            gap.faces_a, gap.faces_b, gap.min_distance_mm, gap.touching,
            gap.tessellated_distance_mm, gap.cross_check_error_mm);
        println!("  closest points (mm): {:?}; faces {:?}", gap.closest_points_mm, gap.closest_faces);
        json!({
            "part_a": gap.part_a, "part_b": gap.part_b,
            "name_a": name_a, "name_b": name_b,
            "faces_a": gap.faces_a, "faces_b": gap.faces_b,
            "min_distance_mm": gap.min_distance_mm,
            "closest_points_mm": gap.closest_points_mm,
            "closest_faces": gap.closest_faces,
            "touching": gap.touching,
            "tessellated_distance_mm": gap.tessellated_distance_mm,
            "cross_check_error_mm": gap.cross_check_error_mm,
        })
    }).collect();
    crate::json::set_summary(json!({
        "file": args.file,
        "parts": parts,
        "measurements": measurements,
        "distance_matrix_mm": distance_matrix,
        "touching_matrix": touching_matrix,
        "touch_tolerance_mm": args.touch_tolerance,
        "cad_tolerance_mm": args.cad_tolerance,
    }));
    Ok(())
}

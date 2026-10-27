//! Writes the simrep-cad fixture shapes (`simrep_cad::write_fixture`) as STEP and IGES into a
//! folder, then reads every file back with `read_cad` so a broken writer or reader shows here.
//!
//! The IGES files come from the same OCCT shapes as the STEP fixtures: the writer picks IGES
//! (B-rep mode, manifold solids) from the `.igs` extension.
//!
//! Usage: `cad-fixtures <out-dir>` (see tools/cad-fixtures/README.md).

use std::path::PathBuf;

use simrep_cad::{ImportOptions, MeasureOptions, MeasurementModel, read_cad, write_fixture};

/// (write_fixture shape, file stem)
const SHAPES: [(u8, &str); 5] = [
    (0, "sphere"),
    (1, "cylinder"),
    (2, "ring"),
    (3, "pair"),
    (4, "touching"),
];

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or("usage: cad-fixtures <out-dir>")?,
    );
    std::fs::create_dir_all(&out)?;
    for (shape, stem) in SHAPES {
        for ext in ["step", "igs"] {
            let path = out.join(format!("{stem}.{ext}"));
            write_fixture(&path, shape, "AP242DIS", "MM")?;
            let parts = read_cad(&path, ImportOptions::default())?;
            if shape >= 3 {
                let gap =
                    MeasurementModel::read(&path)?.gap("0", "1", MeasureOptions::default())?;
                let expected = if shape == 3 { 1.2 - 2.0 * 0.31 } else { 0.0 };
                assert!((gap.min_distance_mm - expected).abs() < 1e-9, "{gap:?}");
                assert_eq!(gap.touching, shape == 4);
                println!(
                    "  gap={:.12} mm, tessellation={:.12} mm, touching={}",
                    gap.min_distance_mm, gap.tessellated_distance_mm, gap.touching
                );
            }
            let size = std::fs::metadata(&path)?.len();
            println!("{} ({size} bytes): {} part(s)", path.display(), parts.len());
            for part in &parts {
                let [lo, hi] = part.bounds_mm;
                println!(
                    "  {:<14} {:?}  bounds [{:.3}, {:.3}, {:.3}]..[{:.3}, {:.3}, {:.3}] mm",
                    part.name, part.mode, lo[0], lo[1], lo[2], hi[0], hi[1], hi[2]
                );
            }
        }
    }
    Ok(())
}

//! Read-only robustness probe for locally available CAD, with no copied fixtures.
use std::error::Error;
use std::time::Instant;

use simrep_cad::{ImportOptions, Mode, read_cad};

fn main() -> Result<(), Box<dyn Error>> {
    for path in std::env::args().skip(1) {
        let started = Instant::now();
        let parts = read_cad(
            &path,
            ImportOptions {
                mode: Mode::Exact,
                ..Default::default()
            },
        )?;
        let preparation = started.elapsed();
        let started = Instant::now();
        let mut failures = 0;
        for part in &parts {
            for i in 0..100 {
                let p = std::array::from_fn(|a| {
                    part.bounds_mm[0][a]
                        + (part.bounds_mm[1][a] - part.bounds_mm[0][a])
                            * ((i * [37, 53, 71][a]) % 101) as f64
                            / 100.0
                });
                if part.try_distance(p).is_err() {
                    failures += 1;
                }
            }
            println!("body {:?} bounds_mm={:?}", part.name, part.bounds_mm);
        }
        println!(
            "{path}: bodies={} failures={failures} preparation_ms={:.3} query_ms={:.3}",
            parts.len(),
            preparation.as_secs_f64() * 1000.0,
            started.elapsed().as_secs_f64() * 1000.0
        );
        assert_eq!(failures, 0);
    }
    Ok(())
}

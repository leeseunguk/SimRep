//! Minimal conversion runner for the external SIMION oracle harness.
use std::io::Write;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let input = PathBuf::from(args.next().ok_or("expected input.gem")?);
    let output = PathBuf::from(args.next().ok_or("expected output.pa#")?);
    let gem = match simrep_gem::read_gem(input) {
        Ok(gem) => gem,
        Err(error) if error.is_include_fragment() => {
            println!("{error}");
            return Ok(());
        }
        Err(error) => return Err(error.into()),
    };
    let mode = args.next();
    if mode.as_ref().is_some_and(|s| s == "--check") {
        println!(
            "dimensions: {} {} {}",
            gem.dimensions[0], gem.dimensions[1], gem.dimensions[2]
        );
        return Ok(());
    }
    let result = gem.rasterize()?;
    if mode.is_none_or(|s| s != "--bench") {
        let mut writer =
            std::io::BufWriter::with_capacity(4 * 1024 * 1024, std::fs::File::create(&output)?);
        result.pa.write(&mut writer)?;
        writer.flush()?;
        result.write_surfaces(&output)?;
    }
    println!(
        "compute_ms: {:.6}",
        result.compute_time.as_secs_f64() * 1000.
    );
    Ok(())
}

use simrep_solve::gpu::{Backends, Gpu, GpuResult};

// A standalone harness makes runtime skips visible even without --nocapture.
// A missing adapter is not counted as a successful GPU validation.
fn main() -> GpuResult<()> {
    let mut executed = 0;
    let mut skipped = 0;
    for backend in [Backends::DX12, Backends::VULKAN] {
        let Some(gpu) = pollster::block_on(Gpu::new(backend))? else {
            println!("SKIP gpu_smoke {backend:?}: no adapter found");
            skipped += 1;
            continue;
        };
        println!("gpu_smoke {:?}: {}", gpu.info.backend, gpu.info.name);
        for result in gpu.smoke()? {
            println!(
                "PASS {}: max abs error = {:e}",
                result.precision, result.max_abs_error
            );
            executed += 1;
        }
        if !gpu.has_f64() {
            println!("SKIP f64: SHADER_F64 unavailable");
            skipped += 1;
        }
    }
    println!("GPU checks: {executed} executed, {skipped} skipped");
    Ok(())
}

//! Standalone release API timing, sharing the integration test's workload.
#[path = "../crates/simrep-fly/tests/default_sweep.rs"]
mod sweep;

fn main() {
    sweep::run_sweep();
}

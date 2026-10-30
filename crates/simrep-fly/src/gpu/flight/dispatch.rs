//! SDF dispatch budget and opt-in host timeline diagnostics.
use std::{cell::RefCell, time::Instant};

// Include the expensive surface phase in the first estimate instead of spending
// several submits doubling from 32 mostly cheap initial trials. The 640-ms goal
// leaves headroom below the 1.5-second measured dispatch acceptance limit.
pub(super) const INITIAL_IONS: usize = 16_384;
pub(super) const INITIAL_TRIALS: u32 = 128;
pub(super) const TRIAL_TARGET_SECONDS: f64 = 0.64;
pub(super) const MAX_TRIALS: u32 = 512;

#[derive(Default)]
struct Timeline {
    first_submit: Option<Instant>,
    last_complete: Option<Instant>,
    submits: usize,
    bytes: u64,
}
thread_local! {
    static TIMELINE: RefCell<Timeline> = RefCell::new(Timeline::default());
}
fn enabled() -> bool {
    cfg!(feature = "precision-study") && std::env::var_os("SIMREP_SDF_DISPATCH_TRACE").is_some()
}
pub(super) fn trace_submit(bytes: u64) {
    if !enabled() {
        return;
    }
    TIMELINE.with_borrow_mut(|t| {
        let now = Instant::now();
        let first = *t.first_submit.get_or_insert(now);
        t.submits += 1;
        t.bytes = bytes;
        eprintln!(
            "sdf_host submit={} since_first_ms={:.6} host_idle_ms={:.6} readback_bytes={bytes}",
            t.submits,
            now.duration_since(first).as_secs_f64() * 1000.0,
            t.last_complete
                .map_or(0.0, |c| now.duration_since(c).as_secs_f64() * 1000.0),
        );
    });
}
pub(super) fn trace_wait(started: Instant) {
    if !enabled() {
        return;
    }
    TIMELINE.with_borrow_mut(|t| {
        let now = Instant::now();
        eprintln!(
            "sdf_host complete={} since_first_ms={:.6} submit_map_wait_ms={:.6} readback_bytes={}",
            t.submits,
            now.duration_since(t.first_submit.unwrap()).as_secs_f64() * 1000.0,
            now.duration_since(started).as_secs_f64() * 1000.0,
            t.bytes,
        );
        t.last_complete = Some(now);
    });
}

//! Per-thread command output, shared by the executable and embedded Python CLI.
use std::cell::{Cell, RefCell};

use serde_json::{Value, json};

thread_local! {
    static ENABLED: Cell<bool> = const { Cell::new(false) };
    static SUMMARY: RefCell<Option<Value>> = const { RefCell::new(None) };
}

pub fn enabled() -> bool {
    ENABLED.get()
}

pub fn set_summary(summary: Value) {
    if enabled() {
        SUMMARY.set(Some(summary));
    }
}

pub fn capture(command: super::Cli) -> simrep_solve::gpu::GpuResult<Option<Value>> {
    let enabled = command.json;
    let previous = ENABLED.replace(enabled);
    let previous_summary = SUMMARY.take();
    let result = super::run(command);
    let summary = SUMMARY.replace(previous_summary);
    ENABLED.set(previous);
    result?;
    Ok(enabled.then(|| {
        let mut summary = summary.expect("JSON commands must provide a summary");
        summary["schema_version"] = json!(1);
        summary["simrep_version"] = json!(env!("CARGO_PKG_VERSION"));
        summary
    }))
}

pub fn error(message: &str, kind: &str) -> Value {
    json!({
        "schema_version": 1,
        "simrep_version": env!("CARGO_PKG_VERSION"),
        "error": {"message": message, "kind": kind},
    })
}

pub fn execution_error(failure: &(dyn std::error::Error + 'static), kind: &str) -> Value {
    let mut value = error(&failure.to_string(), kind);
    if let Some(failure) = failure.downcast_ref::<simrep_solve::gpu::RefinementFailure>() {
        value["converged"] = json!(false);
        value["reason"] = json!(failure.reason);
        value["fallback_reason"] = json!(failure.fallback_reason);
        value["cycles"] = json!(failure.iterations);
        value["final_residual"] = json!(failure.residual.relative);
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    use simrep_solve::{gpu::RefinementFailure, mg::Residual};

    #[test]
    fn gpu_nonconvergence_preserves_structured_diagnostics() {
        let failure = RefinementFailure {
            reason: "stagnation".into(),
            fallback_reason: Some("CPU residual"),
            iterations: 17,
            residual: Residual {
                absolute: 2e-15,
                relative: 1e-15,
            },
        };
        let value = execution_error(&failure, "command_failed");
        assert_eq!(value["converged"], false);
        assert_eq!(value["reason"], failure.reason);
        assert_eq!(value["fallback_reason"], failure.fallback_reason.unwrap());
        assert_eq!(value["cycles"], 17);
        assert_eq!(value["final_residual"], 1e-15);
        assert_eq!(value["error"]["kind"], "command_failed");
    }
}

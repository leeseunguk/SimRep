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

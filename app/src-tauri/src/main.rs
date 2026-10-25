// No console window behind the app in release builds on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
// Proving the sessions (which hold resident wgpu fields, src/live.rs) Send + Sync walks wgpu-core's
// debug-build types deeper than the default limit of 128.
#![recursion_limit = "256"]

use serde::Serialize;
use tauri::Manager;

mod cad;
mod export;
mod fly;
mod geometry;
mod iob;
mod live;
#[cfg(test)]
mod options;
mod pa;
mod project;
mod refine;
mod session;
mod templates;

/// Answer to the UI's start-up IPC check (mirrored by `BackendInfo` in src/runtime.ts).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BackendInfo {
    app_name: &'static str,
    app_version: &'static str,
    core_crate: &'static str,
}

#[tauri::command]
fn backend_info() -> BackendInfo {
    BackendInfo {
        app_name: env!("CARGO_PKG_NAME"),
        app_version: env!("CARGO_PKG_VERSION"),
        core_crate: "simrep-core",
    }
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(session::AppState::default())
        .setup(|app| {
            // Create the GPU device once, off the UI path: the multigrid solver's (refine), then the
            // flyer's pipelines on that same device (live re-fly). Both are kept for the whole session.
            let state = app.state::<session::AppState>().inner().clone();
            std::thread::spawn(move || {
                let solver = state.gpu.get_or_init(refine::probe_gpu);
                let _ = state.fly_gpu.get_or_init(|| live::probe(solver));
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            backend_info,
            pa::open_pa,
            pa::read_file,
            pa::path_info,
            pa::close_session,
            pa::electrode_labels,
            iob::open_iob,
            geometry::inspect_geometry,
            geometry::build_geometry,
            export::export_array,
            fly::export_trajectories,
            project::read_text,
            project::write_text,
            project::write_bytes,
            project::remove_file,
            project::recovery_path,
            templates::create_template,
            refine::refine_targets,
            refine::refine_start,
            refine::refine_cancel,
            fly::fly,
            fly::fast_adjust,
            fly::ion_trace,
            fly::flight_velocities,
            live::compute_device,
        ])
        .run(tauri::generate_context!())
        .expect("failed to start the SimRep window");
}

//! Project files (.simrep, JSON written by the UI) and the autosave recovery file.
//!
//! The UI owns the format (src/state/project.ts); these commands only move text to and from disk.
//! Writes go to a temporary sibling first and are then renamed over the target, so a crash never
//! leaves a half-written project.

use std::path::PathBuf;
use tauri::{AppHandle, Manager};

use crate::pa::PaError;

#[tauri::command]
pub async fn read_text(path: String) -> Result<String, PaError> {
    tauri::async_runtime::spawn_blocking(move || Ok(std::fs::read_to_string(path)?))
        .await
        .map_err(|e| PaError::new("unexpected", e.to_string()))?
}

fn write_atomic(path: String, contents: impl AsRef<[u8]>) -> Result<(), PaError> {
    let path = PathBuf::from(path);
    let mut tmp = path.clone().into_os_string();
    tmp.push(".tmp");
    std::fs::write(&tmp, contents)?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

#[tauri::command]
pub async fn write_text(path: String, contents: String) -> Result<(), PaError> {
    tauri::async_runtime::spawn_blocking(move || write_atomic(path, contents))
        .await
        .map_err(|e| PaError::new("unexpected", e.to_string()))?
}

/// `%XX` escapes (encodeURIComponent) back to UTF-8.
fn percent_decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// Exports (CSV, PNG, SVG): the raw request body goes to the file named by the `path` header
/// (percent-encoded), so large files cross IPC as bytes rather than JSON.
#[tauri::command]
pub async fn write_bytes(request: tauri::ipc::Request<'_>) -> Result<(), PaError> {
    let path = request
        .headers()
        .get("path")
        .and_then(|v| v.to_str().ok())
        .and_then(percent_decode)
        .ok_or_else(|| PaError::new("unexpected", "missing or malformed path header"))?;
    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else {
        return Err(PaError::new("unexpected", "expected a binary body"));
    };
    let bytes = bytes.clone();
    tauri::async_runtime::spawn_blocking(move || write_atomic(path, bytes))
        .await
        .map_err(|e| PaError::new("unexpected", e.to_string()))?
}

/// Deletes a file; a file that is already gone is fine.
#[tauri::command]
pub fn remove_file(path: String) -> Result<(), PaError> {
    match std::fs::remove_file(path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
        _ => Ok(()),
    }
}

/// Where the autosave of the open project lives (the app's local data folder, created on demand).
#[tauri::command]
pub fn recovery_path(app: AppHandle) -> Result<String, PaError> {
    let dir = app.path().app_local_data_dir().map_err(|e| PaError::new("unexpected", e.to_string()))?;
    std::fs::create_dir_all(&dir)?;
    Ok(dir.join("recovery.simrep").display().to_string())
}

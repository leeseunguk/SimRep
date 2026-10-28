//! The project data folder `X.simrep.d/` next to a project file `X.simrep`: stored flight runs
//! (`runs/<id>/`), and whatever else the UI keeps per project (src/state/projectDir.ts).
//!
//! Every command takes the project file path and a path *relative to the data folder*. Relative
//! paths are plain `/`-separated names: no absolute paths, no drive prefixes, no `.` / `..`, no
//! backslashes, and no step through a symbolic link (the folder itself included), so nothing outside
//! `X.simrep.d/` can be read, written, listed or deleted through them.

use std::io::Read;
use std::path::{Component, Path, PathBuf};

use serde::Serialize;
use sha2::{Digest, Sha256};
use tauri::ipc::Response;

use crate::pa::PaError;

const PROJECT_EXTENSION: &str = "simrep";

fn refused(detail: impl Into<String>) -> PaError {
    PaError::new("unexpected", detail)
}

/// `X.simrep` -> `X.simrep.d` (the project path must be absolute and end in `.simrep`).
pub fn data_dir(project: &str) -> Result<PathBuf, PaError> {
    let path = Path::new(project);
    let is_project = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case(PROJECT_EXTENSION));
    if !path.is_absolute() || !is_project {
        return Err(refused(format!(
            "not an absolute .{PROJECT_EXTENSION} project path: {project}"
        )));
    }
    let mut dir = path.as_os_str().to_owned();
    dir.push(".d");
    Ok(PathBuf::from(dir))
}

/// The names of a relative path inside the data folder (`""` = the folder itself, lists only).
fn rel_parts(rel: &str, allow_empty: bool) -> Result<Vec<String>, PaError> {
    if rel.is_empty() {
        return if allow_empty {
            Ok(Vec::new())
        } else {
            Err(refused("empty path inside the project folder"))
        };
    }
    if rel.contains('\\') || rel.contains(':') || rel.contains('\0') {
        return Err(refused(format!("invalid project folder path: {rel}")));
    }
    let trimmed = rel.strip_suffix('/').unwrap_or(rel);
    let mut parts = Vec::new();
    for s in trimmed.split('/') {
        let mut c = Path::new(s).components();
        match (c.next(), c.next()) {
            (Some(Component::Normal(n)), None) if n == s => parts.push(s.to_owned()),
            _ => return Err(refused(format!("path leaves the project folder: {rel}"))),
        }
    }
    Ok(parts)
}

fn is_symlink(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink())
}

/// The absolute path of `rel`, after checking that no existing step (the data folder included)
/// is a symbolic link.
fn resolve(project: &str, rel: &str, allow_empty: bool) -> Result<PathBuf, PaError> {
    let mut path = data_dir(project)?;
    if is_symlink(&path) {
        return Err(refused("the project data folder is a symbolic link"));
    }
    for part in rel_parts(rel, allow_empty)? {
        path.push(part);
        if is_symlink(&path) {
            return Err(refused(format!("path leaves the project folder: {rel}")));
        }
    }
    Ok(path)
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), PaError> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Writes `bytes` to `rel` inside the data folder, creating the folder and parents on demand.
pub fn write(project: &str, rel: &str, bytes: &[u8]) -> Result<(), PaError> {
    let path = resolve(project, rel, false)?;
    let root = data_dir(project)?;
    let parent = path.parent().unwrap_or(&root);
    std::fs::create_dir_all(parent)?;
    // create_dir_all may have created steps; check again that none is a link (a race with a
    // link created meanwhile is out of scope: the folder belongs to the user).
    resolve(project, rel, false)?;
    write_atomic(&path, bytes)
}

pub fn read(project: &str, rel: &str) -> Result<Vec<u8>, PaError> {
    let path = resolve(project, rel, false)?;
    Ok(std::fs::read(path)?)
}

#[derive(Debug, Serialize, PartialEq)]
pub struct Entry {
    /// Relative to the data folder, `/` separated.
    path: String,
    size: u64,
}

/// Every regular file under `prefix` (a folder inside the data folder, `""` = all), recursively,
/// sorted by path. Symbolic links are skipped. A missing folder lists nothing.
pub fn list(project: &str, prefix: &str) -> Result<Vec<Entry>, PaError> {
    let start = resolve(project, prefix, true)?;
    let base = rel_parts(prefix, true)?.join("/");
    let mut out = Vec::new();
    let mut stack = vec![(start, base)];
    while let Some((dir, rel)) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) if e.kind() == std::io::ErrorKind::NotADirectory => continue,
            Err(e) => return Err(e.into()),
        };
        for entry in entries {
            let entry = entry?;
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            let path = if rel.is_empty() {
                name.clone()
            } else {
                format!("{rel}/{name}")
            };
            let kind = entry.file_type()?;
            if kind.is_dir() {
                stack.push((entry.path(), path));
            } else if kind.is_file() && !name.ends_with(".tmp") {
                out.push(Entry {
                    path,
                    size: entry.metadata()?.len(),
                });
            }
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}

/// Deletes a file or a whole folder (a run) inside the data folder; a missing one is fine.
pub fn delete(project: &str, rel: &str) -> Result<(), PaError> {
    let path = resolve(project, rel, false)?;
    let result = match std::fs::symlink_metadata(&path) {
        Ok(m) if m.is_dir() => std::fs::remove_dir_all(&path),
        Ok(_) => std::fs::remove_file(&path),
        Err(e) => Err(e),
    };
    match result {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
        _ => Ok(()),
    }
}

/// Lowercase hex SHA-256 of a file (potential arrays: whether stored results still match).
pub fn sha256_file(path: &Path) -> Result<String, PaError> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, PaError> + Send + 'static,
) -> Result<T, PaError> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| PaError::new("unexpected", e.to_string()))?
}

/// Header value, percent-encoded by the UI (encodeURIComponent).
fn header(request: &tauri::ipc::Request<'_>, name: &str) -> Result<String, PaError> {
    request
        .headers()
        .get(name)
        .and_then(|v| v.to_str().ok())
        .and_then(crate::project::percent_decode)
        .ok_or_else(|| refused(format!("missing or malformed {name} header")))
}

/// Raw body (bytes or UTF-8 text) to `rel-path` of the project in `project-path` (both headers).
#[tauri::command]
pub async fn project_dir_write(request: tauri::ipc::Request<'_>) -> Result<(), PaError> {
    let project = header(&request, "project-path")?;
    let rel = header(&request, "rel-path")?;
    let bytes = match request.body() {
        tauri::ipc::InvokeBody::Raw(bytes) => bytes.clone(),
        tauri::ipc::InvokeBody::Json(serde_json::Value::String(s)) => s.clone().into_bytes(),
        _ => return Err(refused("expected a binary or text body")),
    };
    blocking(move || write(&project, &rel, &bytes)).await
}

#[tauri::command]
pub async fn project_dir_read(project_path: String, rel_path: String) -> Result<Response, PaError> {
    blocking(move || read(&project_path, &rel_path).map(Response::new)).await
}

#[tauri::command]
pub async fn project_dir_list(project_path: String, prefix: String) -> Result<Vec<Entry>, PaError> {
    blocking(move || list(&project_path, &prefix)).await
}

#[tauri::command]
pub async fn project_dir_delete(project_path: String, rel_path: String) -> Result<(), PaError> {
    blocking(move || delete(&project_path, &rel_path)).await
}

#[tauri::command]
pub async fn file_sha256(path: String) -> Result<String, PaError> {
    blocking(move || sha256_file(Path::new(&path))).await
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Temp(PathBuf);
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn temp(name: &str) -> (Temp, String) {
        let dir = std::env::temp_dir().join(format!(
            "simrep-projectdir-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let project = dir.join("lens.simrep").display().to_string();
        (Temp(dir), project)
    }

    #[test]
    fn data_dir_is_next_to_the_project() {
        let (_t, project) = temp("dir");
        assert_eq!(
            data_dir(&project).unwrap(),
            PathBuf::from(format!("{project}.d"))
        );
        assert!(data_dir("relative.simrep").is_err());
        assert!(data_dir(&project.replace(".simrep", ".txt")).is_err());
    }

    #[test]
    fn write_read_list_delete_round_trip() {
        let (_t, project) = temp("roundtrip");
        write(&project, "runs/r1/run.json", b"{}").unwrap();
        write(&project, "runs/r1/results.bin", &[1, 2, 3]).unwrap();
        write(&project, "runs/r2/run.json", b"{\"a\":1}").unwrap();
        assert_eq!(
            read(&project, "runs/r1/results.bin").unwrap(),
            vec![1, 2, 3]
        );
        let all: Vec<_> = list(&project, "runs")
            .unwrap()
            .into_iter()
            .map(|e| (e.path, e.size))
            .collect();
        assert_eq!(
            all,
            vec![
                ("runs/r1/results.bin".to_owned(), 3),
                ("runs/r1/run.json".to_owned(), 2),
                ("runs/r2/run.json".to_owned(), 7)
            ]
        );
        assert_eq!(list(&project, "").unwrap().len(), 3);
        assert!(list(&project, "nothing/here").unwrap().is_empty());
        delete(&project, "runs/r1").unwrap();
        assert_eq!(list(&project, "runs/").unwrap().len(), 1);
        delete(&project, "runs/r2/run.json").unwrap();
        delete(&project, "runs/missing").unwrap();
        assert!(list(&project, "runs").unwrap().is_empty());
        assert_eq!(
            read(&project, "runs/r1/run.json").unwrap_err().code(),
            "notFound"
        );
    }

    #[test]
    fn paths_outside_the_folder_are_refused() {
        let (t, project) = temp("escape");
        std::fs::write(t.0.join("secret.txt"), b"x").unwrap();
        for bad in [
            "../secret.txt",
            "runs/../../secret.txt",
            "/etc/passwd",
            "./run.json",
            "runs//run.json",
            "C:/Windows/win.ini",
            "runs\\..\\x",
            "",
        ] {
            assert!(write(&project, bad, b"x").is_err(), "write {bad:?}");
            assert!(read(&project, bad).is_err(), "read {bad:?}");
            assert!(delete(&project, bad).is_err(), "delete {bad:?}");
        }
        assert!(list(&project, "..").is_err());
        assert!(list(&project, "/tmp").is_err());
        assert_eq!(std::fs::read(t.0.join("secret.txt")).unwrap(), b"x");
    }

    #[cfg(unix)]
    #[test]
    fn symbolic_links_are_not_followed() {
        let (t, project) = temp("links");
        let outside = t.0.join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("f.txt"), b"keep").unwrap();
        write(&project, "runs/r1/run.json", b"{}").unwrap();
        let dir = data_dir(&project).unwrap();
        std::os::unix::fs::symlink(&outside, dir.join("link")).unwrap();
        assert!(read(&project, "link/f.txt").is_err());
        assert!(write(&project, "link/new.txt", b"x").is_err());
        assert!(delete(&project, "link").is_err());
        assert!(list(&project, "link").is_err());
        // Listing the folder skips the link instead of walking out through it.
        assert_eq!(list(&project, "").unwrap().len(), 1);
        assert!(!outside.join("new.txt").exists());
        assert_eq!(std::fs::read(outside.join("f.txt")).unwrap(), b"keep");

        // A data folder that is itself a link is refused as a whole.
        let (t2, project2) = temp("linkroot");
        std::os::unix::fs::symlink(&outside, data_dir(&project2).unwrap()).unwrap();
        assert!(read(&project2, "f.txt").is_err());
        assert!(list(&project2, "").is_err());
        drop(t2);
    }

    #[test]
    fn sha256_of_a_file() {
        let (t, _) = temp("sha");
        let f = t.0.join("a.pa0");
        std::fs::write(&f, b"abc").unwrap();
        assert_eq!(
            sha256_file(&f).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert!(sha256_file(&t.0.join("missing")).is_err());
    }
}

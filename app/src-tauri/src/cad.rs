//! STEP/IGES runtime check (cargo feature `cad`, crates/simrep-cad over OpenCascade).
//!
//! Without the feature every CAD request answers `cadUnavailable`. With it, Windows builds
//! delay-load the OCCT DLLs (build.rs), so the app starts even when they are missing; before the
//! first CAD call `ensure` finds the SDK and puts its `bin` folders on this process's PATH, the
//! same folders OCCT's `env.bat`/`custom.bat` add:
//!
//! - `<CASROOT>\win64\vc14\bin` and every bundled third-party `<CASROOT>\..\<name>\bin`
//!   (tbb, freetype, freeimage, openvr `bin\win64`, ffmpeg, draco);
//! - CASROOT candidates, in order: `SIMREP_OCCT_DIR` (a CASROOT or a folder holding the DLLs),
//!   `CASROOT`, an `occt` folder next to the executable, the executable folder (shared with
//!   the bundled CLI), the SDK the app was built against (`OCCT_LIB_DIR` at build time),
//!   `%USERPROFILE%\occt\opencascade-7.7.0`.
//!
//! It then loads every toolkit once, so a missing DLL becomes a clear message instead of a
//! delay-load crash. Linux links the system OCCT normally.

use crate::pa::{PaError, Reason};

pub const UNAVAILABLE: &str = "cadUnavailable";

#[cfg(not(feature = "cad"))]
pub fn ensure() -> Result<(), PaError> {
    Err(PaError::coded(UNAVAILABLE, Reason::new("cadNotBuilt")))
}

#[cfg(feature = "cad")]
pub fn ensure() -> Result<(), PaError> {
    static READY: std::sync::OnceLock<Result<(), Reason>> = std::sync::OnceLock::new();
    READY
        .get_or_init(load)
        .clone()
        .map_err(|reason| PaError::coded(UNAVAILABLE, reason))
}

#[cfg(all(feature = "cad", not(windows)))]
fn load() -> Result<(), Reason> {
    Ok(())
}

#[cfg(all(feature = "cad", windows))]
const TOOLKITS: [&str; 20] = [
    "TKXDESTEP",
    "TKXDEIGES",
    "TKSTEP",
    "TKIGES",
    "TKXSBase",
    "TKXCAF",
    "TKCAF",
    "TKLCAF",
    "TKCDF",
    "TKMesh",
    "TKShHealing",
    "TKPrim",
    "TKTopAlgo",
    "TKGeomAlgo",
    "TKBRep",
    "TKGeomBase",
    "TKG3d",
    "TKG2d",
    "TKMath",
    "TKernel",
];

#[cfg(all(feature = "cad", windows))]
fn load() -> Result<(), Reason> {
    use std::ffi::{OsStr, c_void};
    use std::os::windows::ffi::OsStrExt;
    use std::path::{Path, PathBuf};

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn LoadLibraryW(name: *const u16) -> *mut c_void;
    }

    let mut candidates: Vec<PathBuf> = Vec::new();
    candidates.extend(std::env::var_os("SIMREP_OCCT_DIR").map(PathBuf::from));
    candidates.extend(std::env::var_os("CASROOT").map(PathBuf::from));
    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
    {
        candidates.push(dir.join("occt"));
        candidates.push(dir);
    }
    if let Some(lib) = option_env!("SIMREP_OCCT_LIB_DIR") {
        candidates.push(Path::new(lib).with_file_name("bin"));
    }
    if let Some(home) = std::env::var_os("USERPROFILE") {
        candidates.push(PathBuf::from(home).join("occt").join("opencascade-7.7.0"));
    }
    // (folder holding TKernel.dll, CASROOT when it is an SDK layout)
    let found = candidates.iter().find_map(|c| {
        let sdk = c.join("win64").join("vc14").join("bin");
        if sdk.join("TKernel.dll").is_file() {
            Some((sdk, Some(c.clone())))
        } else if c.join("TKernel.dll").is_file() {
            // A bin folder (or bundled flat folder); its CASROOT is three levels up when it is win64\vc14\bin.
            let root = c
                .ancestors()
                .nth(3)
                .filter(|r| r.join("win64").is_dir())
                .map(Path::to_path_buf);
            Some((c.clone(), root))
        } else {
            None
        }
    });
    let Some((bin, root)) = found else {
        let searched: Vec<String> = candidates.iter().map(|c| c.display().to_string()).collect();
        return Err(Reason::new("cadNotFound").with("dirs", searched.join("; ")));
    };
    let mut dirs = vec![bin.clone()];
    if let Some(products) = root.as_deref().and_then(Path::parent)
        && let Ok(entries) = std::fs::read_dir(products)
    {
        for entry in entries.flatten() {
            for sub in [
                entry.path().join("bin"),
                entry.path().join("bin").join("win64"),
            ] {
                if sub.is_dir() {
                    dirs.push(sub);
                }
            }
        }
    }
    let path = std::env::var_os("PATH").unwrap_or_default();
    let joined = std::env::join_paths(dirs.iter().cloned().chain(std::env::split_paths(&path)))
        .map_err(|e| Reason::new("cadLoad").with("file", "PATH").detail(e))?;
    // SAFETY: called once (OnceLock), before any OCCT DLL is loaded; other threads only read PATH
    // through the loader, which copies it per call.
    unsafe { std::env::set_var("PATH", joined) };
    for toolkit in TOOLKITS {
        let file = bin.join(format!("{toolkit}.dll"));
        let wide: Vec<u16> = OsStr::new(&file).encode_wide().chain([0]).collect();
        // SAFETY: a NUL-terminated wide path; the module stays loaded for the process lifetime.
        if unsafe { LoadLibraryW(wide.as_ptr()) }.is_null() {
            let dirs = dirs
                .iter()
                .map(|d| d.display().to_string())
                .collect::<Vec<_>>()
                .join("; ");
            return Err(Reason::new("cadLoad")
                .with("file", file.display().to_string())
                .with("dirs", dirs)
                .detail(std::io::Error::last_os_error()));
        }
    }
    Ok(())
}

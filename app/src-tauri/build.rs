/// OCCT toolkits linked by crates/simrep-cad/build.rs (keep the two lists in step).
const OCCT_TOOLKITS: [&str; 20] = [
    "TKXDESTEP", "TKXDEIGES", "TKSTEP", "TKIGES", "TKXSBase", "TKXCAF", "TKCAF", "TKLCAF", "TKCDF", "TKMesh", "TKShHealing", "TKPrim", "TKTopAlgo",
    "TKGeomAlgo", "TKBRep", "TKGeomBase", "TKG3d", "TKG2d", "TKMath", "TKernel",
];

fn main() {
    println!("cargo:rerun-if-env-changed=OCCT_LIB_DIR");
    let cad = std::env::var_os("CARGO_FEATURE_CAD").is_some();
    if cad && std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        // Delay-load OCCT so the app starts (and shows a clear message) when the DLLs are not
        // found; src/cad.rs puts the SDK's bin folders on PATH before the first CAD call.
        for lib in OCCT_TOOLKITS {
            println!("cargo:rustc-link-arg-bins=/DELAYLOAD:{lib}.dll");
        }
        println!("cargo:rustc-link-arg-bins=delayimp.lib");
        // Where the SDK was at build time: the first place src/cad.rs looks for the DLLs.
        if let Ok(dir) = std::env::var("OCCT_LIB_DIR") {
            println!("cargo:rustc-env=SIMREP_OCCT_LIB_DIR={dir}");
        }
    }
    tauri_build::build()
}

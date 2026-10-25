fn main() {
    println!("cargo:rerun-if-changed=native/bridge.cpp");
    for key in ["OCCT_INCLUDE_DIR", "OCCT_LIB_DIR"] {
        println!("cargo:rerun-if-env-changed={key}");
    }
    #[cfg(feature = "occt")]
    build();
}

#[cfg(feature = "occt")]
fn build() {
    use std::{env, path::PathBuf};
    let windows = env::var("CARGO_CFG_TARGET_OS").unwrap() == "windows";
    let include = env::var_os("OCCT_INCLUDE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/usr/include/opencascade"));
    assert!(
        include.join("Standard_Version.hxx").is_file(),
        "CAD requires an OCCT development SDK. Set OCCT_INCLUDE_DIR and OCCT_LIB_DIR; see docs/step-import.md"
    );
    if let Some(lib) = env::var_os("OCCT_LIB_DIR") {
        println!(
            "cargo:rustc-link-search=native={}",
            PathBuf::from(lib).display()
        );
    } else {
        assert!(
            !windows,
            "Set OCCT_LIB_DIR to the MSVC OCCT import-library directory"
        );
    }
    cc::Build::new()
        .cpp(true)
        .std("c++17")
        .include(include)
        .file("native/bridge.cpp")
        .compile("simrep_cad_bridge");
    // OCCT 7.6/7.7 toolkit names. Use a matching SDK, not FreeCAD runtime DLLs.
    for lib in [
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
    ] {
        println!("cargo:rustc-link-lib=dylib={lib}");
    }
}

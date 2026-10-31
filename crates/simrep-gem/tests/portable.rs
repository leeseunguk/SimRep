use std::collections::BTreeMap;

use simrep_gem::{Error, parse_gem, read_gem_with};

#[test]
fn byte_loader_resolves_relative_includes_and_preserves_raster_bytes() {
    let files = BTreeMap::from([
        (
            "dir/main.gem",
            b"pa_define(9,9,1,planar)\ninclude('parts/../electrode')".to_vec(),
        ),
        (
            "dir/electrode.gem",
            b"e(1){fill{within{box(1,1,3,3)}}}".to_vec(),
        ),
    ]);
    let gem = read_gem_with("dir/main.gem", move |path| {
        files
            .get(path.to_str().unwrap())
            .cloned()
            .ok_or_else(|| Error("missing bytes".into()))
    })
    .unwrap();
    let expected = parse_gem("pa_define(9,9,1,planar)\ne(1){fill{within{box(1,1,3,3)}}}").unwrap();
    let mut actual_bytes = Vec::new();
    let mut expected_bytes = Vec::new();
    gem.rasterize()
        .unwrap()
        .pa
        .write(&mut actual_bytes)
        .unwrap();
    expected
        .rasterize()
        .unwrap()
        .pa
        .write(&mut expected_bytes)
        .unwrap();
    assert_eq!(actual_bytes, expected_bytes);
}

#[test]
fn embedded_stl_uses_scoped_bytes_and_restores_native_reads_after_errors() {
    let text = "pa_define(13,7,7,planar)\ne(1){fill{within{stl('two.stl')}}}";
    let gem = read_gem_with("virtual/main.gem", move |path| {
        match path.to_str().unwrap() {
            "virtual/main.gem" => Ok(text.as_bytes().to_vec()),
            "virtual/two.stl" => Ok(include_bytes!("fixtures/two_tetrahedra.stl").to_vec()),
            _ => Err(Error("unregistered file".into())),
        }
    })
    .unwrap();
    let native_text = format!(
        "pa_define(13,7,7,planar)\ne(1){{fill{{within{{stl('{}')}}}}}}",
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/two_tetrahedra.stl"
        )
    );
    let expected = parse_gem(&native_text).unwrap();
    let mut actual_bytes = Vec::new();
    let mut expected_bytes = Vec::new();
    gem.rasterize()
        .unwrap()
        .pa
        .write(&mut actual_bytes)
        .unwrap();
    expected
        .rasterize()
        .unwrap()
        .pa
        .write(&mut expected_bytes)
        .unwrap();
    assert_eq!(actual_bytes, expected_bytes);
    assert!(
        read_gem_with("main.gem", move |path| {
            if path.to_str() == Some("main.gem") {
                Ok(text.as_bytes().to_vec())
            } else {
                Err(Error("missing STL bytes".into()))
            }
        })
        .is_err()
    );
    assert!(parse_gem(&native_text).is_ok());
}

#[cfg(not(feature = "lua"))]
#[test]
fn executable_lua_is_rejected_without_misreading_strings_or_comments() {
    for source in [
        "# local n=5\npa_define(5,5,1,planar)",
        "pa_define{nx=5,ny=5,nz=1}\nfor i=1,3 do end",
        "pa_define(5,5,1,planar)\nprint('code')",
        "pa_define(5,5,1,planar)\nos.exit()",
    ] {
        assert!(
            parse_gem(source)
                .unwrap_err()
                .0
                .contains("unsupported in browser: GEM Lua")
        );
    }
    assert!(parse_gem("--[[ function for if end ]]\n; # local ignored\npa_define(5,5,1,planar)\ne(1){fill{within{box(1,1,2,2)}}}").is_ok());
}

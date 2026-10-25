use simrep_core::{FieldType, Symmetry};
use simrep_gem::{parse_gem, read_gem};

fn electrode_at(source: &str, x: usize, y: usize, z: usize) -> Option<f64> {
    let result = parse_gem(source).unwrap().rasterize().unwrap();
    let a = result.pa.array;
    let i = a.index(x, y, z).unwrap();
    a.electrodes[i].then_some(a.potentials[i])
}

#[test]
fn ordered_fills_and_non_electrodes() {
    let source = "pa_define(11,11,1,p,n)
        e(3){ fill{ within{box(1,1,9,9)} notin_inside{circle(5,5,2)} } }
        n(7){fill{within{box(1,1,2,2)}}}
        e(4){fill{within{box(7,7,9,9)}}}";
    assert_eq!(electrode_at(source, 1, 1, 0), None);
    assert_eq!(electrode_at(source, 5, 5, 0), None);
    assert_eq!(electrode_at(source, 5, 3, 0), Some(3.));
    assert_eq!(electrode_at(source, 8, 8, 0), Some(4.));
    assert_eq!(electrode_at(source, 0, 0, 0), None);
}

#[test]
fn boundary_modes_and_half_grid_expansion() {
    for (mode, on, outside) in [
        ("within", true, true),
        ("within_inside", false, false),
        ("within_inside_or_on", true, false),
    ] {
        let source = format!("pa_define(11,11,1,p,n) e(1){{fill{{{mode}{{circle(5,5,3)}}}}}}");
        assert_eq!(electrode_at(&source, 8, 5, 0).is_some(), on);
        assert_eq!(electrode_at(&source, 8, 6, 0).is_some(), outside);
    }
}

#[test]
fn intersections_unions_and_modern_order() {
    let source = "pa_define{11,11,1, surface='none'}
        e(1){ intersect{box(0,0,5,5) circle(0,0,5)} notin{box(1,1,4,4)}
               box(2,2,3,3) notin_inside{box(2,2,3,3)} }";
    assert_eq!(electrode_at(source, 5, 5, 0), None);
    assert_eq!(electrode_at(source, 2, 2, 0), Some(1.));
    assert_eq!(electrode_at(source, 4, 4, 0), None);
}

#[test]
fn scale_rotation_and_mm_grid_units() {
    let source = "pa_define(21,21,21,p,n,e,,0.5)
        locate(5,5,5,2,90,0,0){e(2){fill{within{cylinder(0,0,0,1,,2)}}}}";
    assert_eq!(electrode_at(source, 6, 10, 10), Some(2.));
    assert_eq!(electrode_at(source, 14, 10, 10), None);
    let source = "pa_define(11,11,1,p,n,e,,0.5,,,gu)
        e(1){fill{within{box(8,8,9,9)}}}";
    assert_eq!(electrode_at(source, 8, 8, 0), Some(1.));
}

#[test]
fn expressions_named_parameters_and_metadata() {
    let gem = parse_gem("-- comment\nlocal R=5\n# local d=0.5
        pa_define{wx=R*2,wy=R, dx=d, symmetry='cylindrical x', type='magnetic', ng=20, refinable=false}
        e(2^3){circle(R,0, math.sqrt(4))}").unwrap();
    assert_eq!(gem.dimensions, [21, 11, 1]);
    assert_eq!(gem.spacing_mm, [0.5; 3]);
    assert_eq!(gem.symmetry, Symmetry::Cylindrical);
    assert!(gem.mirrors.x && gem.mirrors.y);
    assert_eq!(gem.field_type, FieldType::Magnetic);
    assert!(!gem.refinable);
    assert_eq!(gem.ng, 20);
    assert_eq!(gem.fills[0].potential, 8.);
}

#[test]
fn polygon_revolution_and_edge_shell() {
    let cone = "pa_define(21,21,21,p,n) locate(0,10,10){e(1){rotate_fill(360){
        within{polyline(0,0, 0,5, 10,0)}
    }}}";
    assert_eq!(electrode_at(cone, 5, 12, 10), Some(1.));
    assert_eq!(electrode_at(cone, 5, 14, 10), None);
    let edge = "pa_define(11,11,1,p,n) e(1){edge_fill{within{box(2,2,8,8)}}}";
    assert_eq!(electrode_at(edge, 2, 5, 0), Some(1.));
    assert_eq!(electrode_at(edge, 5, 5, 0), None);
}

#[test]
fn explicit_unsupported_and_invalid_input_errors() {
    for (source, expected) in [
        ("pa_define(11,11,1,p,n,surface=invalid)", "surface=invalid"),
        ("pa_define{11,11} e(1){stl('missing.stl')}", "stl"),
        ("pa_define{11,11} unsupported_shape()", "unsupported"),
        ("# for i=1,3 do\n", "Lua"),
        ("pa_define(2,11,1,p,n)", "dimension"),
        ("pa_define{11,11} e(1){box()}", ""),
    ] {
        let result = parse_gem(source);
        if expected.is_empty() {
            assert!(result.is_ok());
        } else {
            let error = result.unwrap_err().to_string();
            assert!(error.contains(expected), "{source}: {error}");
        }
    }
}

#[test]
fn include_is_relative_to_containing_file() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/main.gem");
    let gem = read_gem(path).unwrap();
    assert_eq!(gem.fills.len(), 1);
    let pa = gem.rasterize().unwrap().pa;
    assert_eq!(pa.array.potentials[2 + 11 * 2], 4.);
}

#[test]
fn sampled_sdf_contract_units_order_and_overwrites() {
    let gem = parse_gem(
        "pa_define(11,11,1,p,n,e,,0.5,,,gu)
        e(3){fill{within{circle(4,4,2)}}}
        n(0){fill{within{box(4,4,5,5)}}}",
    )
    .unwrap();
    let grids = gem
        .sample_electrode_sdfs([11, 11, 1], [0.; 3], [0.5; 3])
        .unwrap();
    let sdf = &grids[0];
    assert_eq!(sdf.voltage, 3.);
    assert_eq!(sdf.dims, [11, 11, 1]);
    assert_eq!(sdf.origin, [0.; 3]);
    assert_eq!(sdf.spacing, [0.5; 3]);
    assert_eq!(sdf.values.len(), 121);
    assert!(sdf.values.iter().all(|v| v.is_finite()));
    assert!(sdf.values[3 + 11 * 4] < 0.);
    assert!(sdf.values[5 + 11 * 5] >= 0.);
    assert!((sdf.values[8 + 11 * 4] - 1.).abs() < 1e-12);
    assert!((sdf.sample([3.75, 2., 0.]).unwrap() - 0.75).abs() < 1e-12);
    assert!(sdf.sample([-0.1, 0., 0.]).is_none());
    for y in 0..11 {
        for x in 0..10 {
            assert!((sdf.values[x + 11 * y] - sdf.values[x + 1 + 11 * y]).abs() <= 0.5 + 1e-12);
        }
    }
}

#[test]
fn oracle_regressions_hyperbola_and_unclipped_edges() {
    let quad =
        "pa_define(101,101,1,p,xy) e(2){fill{within{locate(,,,,,90){hyperbola(0,0,50,50)}}}}";
    assert_eq!(electrode_at(quad, 54, 22, 0), Some(2.));
    let ring = "pa_define(51,51,1,p,xy) e(2){edge_fill{within{circle(0,0,50)}}}";
    assert_eq!(electrode_at(ring, 0, 0, 0), None);
    assert_eq!(electrode_at(ring, 50, 0, 0), Some(2.));
}

#[test]
fn oracle_polygon_offsets_and_revolution_bounds() {
    let cone = "pa_define(51,51,51,p,n)
        locate(10,25,25){e(1){rotate_fill(){within{polyline(0,0,0,15,30,0)}}}}";
    assert_eq!(electrode_at(cone, 12, 21, 11), Some(1.));
    assert_eq!(electrode_at(cone, 10, 21, 10), Some(1.));
    assert_eq!(electrode_at(cone, 41, 25, 25), None);
    let pierce = "pa_define(221,121,1,c,y,,,0.05)
        e(0){fill{within{polyline(0,0,0,2,1.656854,6,0,6)}}}";
    assert_eq!(electrode_at(pierce, 8, 58, 0), Some(0.));
}

#[test]
fn oracle_modern_inheritance_and_axis_half_spaces() {
    let source = "pa_define{11,11,1} e(1){box(0,0,10,10)}
        n(0){box(0,0,10,2)}";
    assert_eq!(electrode_at(source, 5, 0, 0), None);
    assert_eq!(electrode_at(source, 5, 2, 0), None);
    let source = "pa_define{11,11,1} e(1){half_space('-x',5)}";
    assert_eq!(electrode_at(source, 4, 5, 0), Some(1.));
    assert_eq!(electrode_at(source, 6, 5, 0), None);
}

#[test]
fn fractional_surface_sidecar_matches_oracle_plane() {
    let gem = parse_gem(
        "pa_define(11,11,1,p,n,surface=fractional)
        e(1){fill{within{box(-5,-5,0.3,20)}}}",
    )
    .unwrap();
    let result = gem.rasterize().unwrap();
    let surface = result.surfaces.unwrap();
    assert_eq!(
        surface.axes[0],
        (0..11).map(|y| (y * 11, 307. / 1024.)).collect::<Vec<_>>()
    );
    assert!(surface.axes[1].is_empty() && surface.axes[2].is_empty());
    let mut encoded = Vec::new();
    surface.write(&mut encoded).unwrap();
    assert_eq!(&encoded[..8], b"SIMEXP\xff\x7f");
    assert_eq!(encoded.len(), 260);
    assert_eq!(u64::from_le_bytes(encoded[28..36].try_into().unwrap()), 176);
    assert_eq!(&encoded[36..40], b"XOFF");
    assert_eq!(&encoded[232..236], b"YOFF");
    assert_eq!(&encoded[252..256], b"ZOFF");
}

#[test]
fn empty_macro_lines_roundoff_and_infinite_coordinates() {
    let gem = parse_gem(
        "# -- header\n#\n# local ny=20.2/0.1+1
        pa_define(3,$(ny),1,c,xy,e,0.1)
        e(1){fill{within{box(0,0,2,50)}}}",
    )
    .unwrap();
    assert_eq!(gem.dimensions, [3, 203, 1]);
    assert_eq!(gem.ng, 100);
    let source = "pa_define{21,21,21}
        e(1){cylinder3d(10,10,-inf,10,10,inf,3)}";
    assert_eq!(electrode_at(source, 10, 10, 0), Some(1.));
    assert_eq!(electrode_at(source, 10, 10, 20), Some(1.));
    assert_eq!(electrode_at(source, 0, 0, 10), None);
    let source = "pa_define{21,21,1}
        e(1){polyline(0,0,inf,0,0,inf)}";
    assert_eq!(electrode_at(source, 20, 20, 0), Some(1.));
}

#[test]
fn relative_stl_import_and_connected_component_selection() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/stl.gem");
    let gem = read_gem(path).unwrap();
    let result = gem.rasterize().unwrap();
    for (x, voltage) in [(3, 1.), (15, 2.)] {
        let i = result.pa.array.index(x, 3, 3).unwrap();
        assert!(result.pa.array.electrodes[i]);
        assert_eq!(result.pa.array.potentials[i], voltage);
    }
}

#[test]
fn polygon_vertex_clipping_and_inside_tolerance() {
    let source = "local c=cos(rad(30)) local s=sin(rad(30))
        local x=14*c+15*s-20 local y=sqrt((15*c-14*s)^2+36)
        pa_define(3,3,1,p,n)
        locate(-x,-y){e(1){fill{within{polyline(0,0,12,0,0,8)}}}}";
    assert_eq!(electrode_at(source, 0, 0, 0), None);
    let source = "pa_define(11,11,1,p,n) e(1){fill{within_inside{circle(5,5,3.00001)}}}";
    assert_eq!(electrode_at(source, 8, 5, 0), None);
    let source = "pa_define(11,11,1,p,n) e(1){fill{within_inside_or_on{circle(5,5,2.99999)}}}";
    assert_eq!(electrode_at(source, 8, 5, 0), Some(1.));
}

#[test]
fn lua51_preprocessing_statements_macros_and_balanced_substitutions() {
    let gem = parse_gem(
        "; legacy macros
        # local values = {2, 4, 6}
        $(local function volts(i) return values[i]^2 end)
        # function square(x)
          box($(x),1,$(x+1),2)
        # end
        pa_define(11,11,1,p,n)
        # for i,v in ipairs(values) do
          e($(volts(i))) { fill { within { $(square(v)) } } }
        # end
        # _put('e(99){fill{within{box(0,0,0,0)}}}')
        ",
    )
    .unwrap();
    let pa = gem.rasterize().unwrap().pa.array;
    for (x, potential) in [(0, 99.), (2, 4.), (4, 16.), (6, 36.)] {
        assert_eq!(
            pa.potentials[pa.index(x, if x == 0 { 0 } else { 1 }, 0).unwrap()],
            potential
        );
    }
}

#[test]
fn modern_lua_tables_functions_methods_and_argument_statements() {
    let source = "
        local values = {2,4,6}
        assert(_VERSION == 'Lua 5.1')
        local label = ('%s-%d'):format('shape', #values)
        assert(label == 'shape-3')
        pa_define{10*mm,10*mm,1,surface='none'}
        local function polygon(x)
          polyline(
            for i=1,3 do
              local y = i*2,
              x+y, 1
            end
          )
        end
        for i,v in ipairs(values) do
          e(v) { box(v,2,v,2) }
        end
        e(8){polygon(0)}
    ";
    assert_eq!(electrode_at(source, 4, 2, 0), Some(4.));
    assert_eq!(electrode_at(source, 4, 1, 0), Some(8.));
}

#[test]
fn lua_shape_and_potential_callbacks_preserve_coordinate_scopes() {
    let source = "
        local function voltage(x,y,z) return x+10*y end
        pa_define{11,11,1,surface='fractional'}
        locate(1,2,0) {
          e(voltage) {
            locate(1,0,0) { shape(function(x,y,z) return x>=1 and x<=3 and y==2 end) }
          }
        }
    ";
    assert_eq!(electrode_at(source, 3, 4, 0), Some(22.));
    assert_eq!(electrode_at(source, 5, 4, 0), Some(24.));
    assert_eq!(electrode_at(source, 6, 4, 0), None);
    let gem = parse_gem(source).unwrap();
    assert!(
        gem.electrode_sdfs()
            .unwrap_err()
            .to_string()
            .contains("Lua")
    );
}

#[test]
fn lua_sandbox_and_callback_errors_are_reported() {
    let source = "
        local file = assert(io.open('params.txt','w'))
        file:write('key=', 3, '\\n')
        file:close()
        assert(os==nil and package==nil and debug==nil and require==nil)
        assert(loadfile==nil and loadstring==nil and dofile==nil)
        assert(io.open('/etc/passwd','r')==nil)
        pa_define{3,3,1}
        e(1){shape(function(x,y,z) error('callback failed') end)}
    ";
    let error = parse_gem(source)
        .unwrap()
        .rasterize()
        .err()
        .unwrap()
        .to_string();
    assert!(error.contains("callback failed"), "{error}");
}

#[test]
fn standalone_include_fragment_has_a_distinct_status() {
    let error = parse_gem("e(1){box(0,0,1,1)}").unwrap_err();
    assert!(error.is_include_fragment());
    let error = parse_gem("box() pa_define{3,3,1}").unwrap_err();
    assert!(!error.is_include_fragment());
}

#[test]
fn expanded_sphere_endpoints_and_electric_ng_match_simion() {
    let gem = parse_gem("pa_define(11,11,1,p,n,e,1) e(1){fill{within{circle(5,5,2.5)}}}").unwrap();
    assert_eq!(gem.ng, 100);
    let array = gem.rasterize().unwrap().pa.array;
    assert!(array.electrodes[array.index(8, 5, 0).unwrap()]);
    let sphere = "pa_define(101,101,101,p,n)
        locate(50,50,50){e(43){fill{within{sphere(0,0,0,22.5)}}}}";
    assert_eq!(electrode_at(sphere, 50, 50, 27), Some(43.));
}

#[test]
fn lua_and_legacy_includes_share_recursion_detection() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/recursive_modern.gem");
    assert!(
        read_gem(path)
            .unwrap_err()
            .to_string()
            .contains("recursive include")
    );
}

#[test]
fn lua_substitution_preserves_numeric_precision() {
    let source = "# local value=math.cos(math.pi/2)\n\
        pa_define(3,3,1,p,n)\n\
        n($(value)){fill{within{box(0,0,2,2)}}}";
    let gem = parse_gem(source).unwrap();
    assert_eq!(gem.fills[0].potential, (std::f64::consts::PI / 2.).cos());
}

#[test]
fn empty_fractional_surfaces_remove_stale_companions() {
    let result = parse_gem("pa_define{3,3,1,surface='fractional'} e(1){box(-1,-1,4,4)}")
        .unwrap()
        .rasterize()
        .unwrap();
    let path = std::env::temp_dir().join(format!(
        "simrep-gem-empty-surface-{}.pa#",
        std::process::id()
    ));
    let companion = path.with_extension("pa-surf");
    std::fs::write(&companion, b"stale surface").unwrap();
    result.write_surfaces(&path).unwrap();
    assert!(!companion.exists());
}

use std::process::Command;

#[cfg(not(feature = "cad"))]
#[test]
fn cad_input_explains_the_optional_feature() {
    let dir = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_simrep"))
        .args([
            "refine-geometry",
            "sphere.step",
            "--electrode",
            "0=1:1",
            "--out",
            "sphere",
        ])
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("--features cad"));
}

#[cfg(feature = "cad")]
#[test]
fn cad_formats_and_mesh_fallback_write_fast_adjust_and_cuts() {
    let dir = tempfile::tempdir().unwrap();
    for extension in ["step", "stp", "iges", "igs"] {
        let input = format!("sphere.{extension}");
        simrep_cad::write_fixture(dir.path().join(&input), 0, "AP242DIS", "MM").unwrap();
        for mode in ["exact", "tessellated"] {
            let output = Command::new(env!("CARGO_BIN_EXE_simrep"))
                .args([
                    "refine-geometry",
                    &input,
                    "--grid",
                    "9,9,9",
                    "--spacing",
                    "0.2,0.2,0.2",
                    "--origin",
                    "-0.8,-0.8,-0.8",
                    "--electrode",
                    if extension == "step" {
                        "sphere=3:17"
                    } else {
                        "0=3:17"
                    },
                    "--cad-mode",
                    mode,
                    "--cad-tolerance",
                    "0.002",
                    "--sidecar-dense-sdf",
                    "--out",
                    "sphere",
                ])
                .current_dir(dir.path())
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            let pa =
                simrep_io::read_fast_adjust(dir.path().join("sphere.pa0"), &[(3, 7.0)]).unwrap();
            assert!(pa.electrodes[4 + 9 * (4 + 9 * 4)]);
            assert!(pa.potentials.iter().all(|v| (v - 7.0).abs() < 1e-7));
            let cuts: serde_json::Value = serde_json::from_slice(
                &std::fs::read(dir.path().join("sphere.cuts.json")).unwrap(),
            )
            .unwrap();
            let cut = cuts["cut_fractions"][6 + 9 * (4 + 9 * 4)][0]
                .as_f64()
                .unwrap();
            assert!((cut - 0.45).abs() < if mode == "exact" { 1e-7 } else { 0.01 });
            assert_eq!(cuts["version"], 3);
            assert_eq!(
                cuts["source_geometry"][0]["geometry"]["chordal_tolerance_mm"],
                0.002
            );
            let surface = simrep_fly::surface::SurfaceModel::from_reader(
                std::fs::File::open(dir.path().join("sphere.cuts.json")).unwrap(),
            )
            .unwrap();
            assert_eq!(surface.membership([0.0; 3]), Some(3));
            let (t, number) = surface
                .first_hit([-0.7, 0.0, 0.0], [0.7, 0.0, 0.0])
                .unwrap();
            assert_eq!(number, 3);
            assert!((-0.7 + 1.4 * t + 0.31).abs() < 0.002);
        }
    }
}

#[cfg(feature = "cad")]
#[test]
fn located_step_electrodes_refine_and_fly_with_fast_adjust() {
    let dir = tempfile::tempdir().unwrap();
    simrep_cad::write_fixture(dir.path().join("pair.step"), 3, "AP242DIS", "MM").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_simrep"))
        .args([
            "refine-geometry",
            "pair.step",
            "--grid",
            "17,17,17",
            "--spacing",
            "0.125,0.125,0.125",
            "--origin",
            "-1,-1,-1",
            "--electrode",
            "pair/left=1:10",
            "--electrode",
            "pair/right=2:-10",
            "--cad-mode",
            "exact",
            "--out",
            "pair",
        ])
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    for extension in ["pa0", "pa1", "pa2", "pa#", "cuts.json"] {
        assert!(dir.path().join(format!("pair.{extension}")).is_file());
    }
    let adjusted =
        simrep_io::read_fast_adjust(dir.path().join("pair.pa0"), &[(1, 5.0), (2, -5.0)]).unwrap();
    assert!(adjusted.potentials.iter().all(|v| v.is_finite()));
    assert!((adjusted.potentials[3 + 17 * (8 + 17 * 8)] - 5.0).abs() < 1e-8);
    assert!((adjusted.potentials[13 + 17 * (8 + 17 * 8)] + 5.0).abs() < 1e-8);
    std::fs::write(dir.path().join("probe.ion"), "0,1,1,0,-0.7,0,0,90,100\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_simrep"))
        .args([
            "fly",
            "--pa",
            "pair.pa0",
            "--particles",
            "probe.ion",
            "--voltage",
            "1=5,2=-5",
            "--origin",
            "-1,-1,-1",
            "--plane",
            "0,1,0,0.7",
            "--device",
            "cpu",
            "--output",
            "flight.csv",
        ])
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let csv = std::fs::read_to_string(dir.path().join("flight.csv")).unwrap();
    let rows: Vec<_> = csv.lines().skip(1).collect();
    assert_eq!(rows.len(), 1);
    let fields: Vec<_> = rows[0].split(',').collect();
    let x: f64 = fields[2].parse().unwrap();
    let y: f64 = fields[3].parse().unwrap();
    assert!(x > 0.001 && x < 0.2, "charged probe must deflect: {csv}");
    assert!((y - 0.7).abs() < 1e-8, "{csv}");
    assert_eq!(fields[6], "plane");
}

#[cfg(feature = "cad")]
#[test]
fn tessellated_pair_fixture_pins_cut_nodes_and_fraction() {
    let dir = tempfile::tempdir().unwrap();
    simrep_cad::write_fixture(dir.path().join("pair.step"), 3, "AP242DIS", "MM").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_simrep"))
        .args([
            "refine-geometry",
            "pair.step",
            "--grid",
            "17,17,17",
            "--spacing",
            "0.125,0.125,0.125",
            "--origin",
            "-1,-1,-1",
            "--electrode",
            "pair/left=1:10",
            "--electrode",
            "pair/right=2:-10",
            "--cad-mode",
            "tessellated",
            "--cad-tolerance",
            "0.001",
            "--device",
            "cpu",
            "--out",
            "pair",
        ])
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let cuts: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.path().join("pair.cuts.json")).unwrap()).unwrap();
    assert_eq!(cuts["version"], 3);
    let links = cuts["link_intersections"].as_array().unwrap();
    assert_eq!(links.len(), 156);
    assert_eq!(links[0]["node"], 1567);
    assert_eq!(links[0]["electrodes"][5], 1);
    let first = links[0]["fractions"][5].as_f64().unwrap();
    assert!((first - 0.7426782932745095).abs() < 1e-12, "{first}");
    assert_eq!(links[80]["node"], 2465);
    assert_eq!(links[80]["electrodes"][1], 1);
    let middle = links[80]["fractions"][1].as_f64().unwrap();
    assert!((middle - 0.9341038565408231).abs() < 1e-12, "{middle}");
    let node_sum: u64 = links
        .iter()
        .map(|link| link["node"].as_u64().unwrap())
        .sum();
    assert_eq!(node_sum, 383_136);
    let crossings = links
        .iter()
        .flat_map(|link| link["fractions"].as_array().unwrap())
        .filter(|value| !value.is_null())
        .count();
    assert_eq!(crossings, 252);
    assert_eq!(cuts["node_electrode_runs"][1][0], 1856);
    assert_eq!(cuts["node_electrode_runs"][1][1], 1858);
    assert_eq!(cuts["node_electrode_runs"][1][2], 1);
    assert_eq!(cuts["solver_fixed_runs"][1][0], 1855);
    assert_eq!(cuts["solver_fixed_runs"][1][1], 1858);
    assert_eq!(cuts["solver_fixed_runs"][1][2], true);

    // Potentials and mesh normals match the pre-optimization binary on this
    // fixture. Vertex bits fix the tessellation; the unit normal is the
    // cross product of triangle 0.
    let pa = simrep_io::read_pa(dir.path().join("pair.pa0")).unwrap();
    assert_eq!(pa.array.potentials[0].to_bits(), 4_615_274_635_331_296_253);
    assert_eq!(
        pa.array.potentials[2456].to_bits(),
        4_557_068_357_500_370_944
    );
    assert_eq!(pa.array.potentials[2451].to_bits(), 10.0f64.to_bits());
    assert_eq!(pa.array.potentials[2461].to_bits(), (-10.0f64).to_bits());
    let expected = [
        (
            14_512_501_023_914_503_539u64,
            [
                13_826_639_437_011_752_996u64,
                4_566_617_848_529_879_005,
                13_822_593_006_376_035_740,
            ],
        ),
        (
            12_513_857_739_664_593_252,
            [
                4_603_891_678_039_264_834,
                4_566_617_848_529_879_005,
                13_822_593_006_376_035_740,
            ],
        ),
    ];
    for (src, (checksum, vertex0)) in cuts["source_geometry"]
        .as_array()
        .unwrap()
        .iter()
        .zip(expected)
    {
        let bytes = decode_standard_b64(src["geometry"]["data"].as_str().unwrap());
        assert_eq!(fnv64(&bytes), checksum);
        let part = simrep_mesh::Part::from_source_bytes(&bytes).unwrap();
        assert_eq!(part.vertices[0].map(f64::to_bits), vertex0);
        let [ia, ib, ic] = part.triangles[0];
        let normal = unit_normal(part.vertices[ia], part.vertices[ib], part.vertices[ic]);
        assert!(
            (normal[0] - 0.05607037785263761).abs() < 1e-12,
            "{normal:?}"
        );
        assert!(
            (normal[1] - 0.0015731830354406052).abs() < 1e-12,
            "{normal:?}"
        );
        assert!((normal[2] + 0.9984255795113622).abs() < 1e-12, "{normal:?}");
    }
}

#[cfg(feature = "cad")]
fn decode_standard_b64(input: &str) -> Vec<u8> {
    fn value(byte: u8) -> u8 {
        match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => 0,
        }
    }
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    for chunk in bytes.chunks(4) {
        if chunk.len() < 4 {
            break;
        }
        let packed = ((value(chunk[0]) as u32) << 18)
            | ((value(chunk[1]) as u32) << 12)
            | ((value(chunk[2]) as u32) << 6)
            | value(chunk[3]) as u32;
        out.push((packed >> 16) as u8);
        if chunk[2] != b'=' {
            out.push((packed >> 8) as u8);
        }
        if chunk[3] != b'=' {
            out.push(packed as u8);
        }
    }
    out
}

#[cfg(feature = "cad")]
fn fnv64(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0u64, |acc, byte| {
        acc.wrapping_mul(0x100000001b3) ^ u64::from(*byte)
    })
}

#[cfg(feature = "cad")]
fn unit_normal(a: [f64; 3], b: [f64; 3], c: [f64; 3]) -> [f64; 3] {
    let e1 = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let e2 = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let n = [
        e1[1] * e2[2] - e1[2] * e2[1],
        e1[2] * e2[0] - e1[0] * e2[2],
        e1[0] * e2[1] - e1[1] * e2[0],
    ];
    let mag = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    [n[0] / mag, n[1] / mag, n[2] / mag]
}

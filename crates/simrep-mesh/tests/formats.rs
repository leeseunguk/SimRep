mod common;

use common::cube;
use simrep_mesh::{ImportOptions, Mesh, Solid, Transform, Unit, inspect, read_mesh, write_mesh};
use std::fs;
use std::io::Write;
use tempfile::tempdir;

#[test]
fn unknown_units_and_file_formats_list_supported_values() {
    let error = "typo".parse::<Unit>().unwrap_err().to_string();
    for unit in [
        "um",
        "micron",
        "mm",
        "millimeter",
        "cm",
        "centimeter",
        "m/meter",
        "in",
        "inch",
        "ft",
        "foot",
    ] {
        assert!(error.contains(unit), "{error}");
    }
    let dir = tempdir().unwrap();
    let path = dir.path().join("mesh.typo");
    let error = read_mesh(&path, ImportOptions::default())
        .unwrap_err()
        .to_string();
    for format in ["stl", "obj", "ply", "3mf", "gltf", "glb"] {
        assert!(error.contains(format), "{error}");
    }
    let mesh = Mesh {
        parts: vec![cube(-1.0, 1.0)],
    };
    let error = write_mesh(&path, &mesh).unwrap_err().to_string();
    for format in ["stl", "obj", "gltf", "glb"] {
        assert!(error.contains(format), "{error}");
    }
    assert!(!path.exists());
}

#[test]
fn stl_obj_gltf_glb_roundtrip_preserves_two_solids_and_units() {
    let dir = tempdir().unwrap();
    let mut second = cube(-1.0, 1.0);
    second.name = "second".into();
    for v in &mut second.vertices {
        v[0] += 5.0;
    }
    let original = Mesh {
        parts: vec![cube(-1.0, 1.0), second],
    };
    for extension in ["stl", "obj", "gltf", "glb"] {
        let path = dir.path().join(format!("mesh.{extension}"));
        write_mesh(&path, &original).unwrap();
        let mesh = read_mesh(&path, ImportOptions::default()).unwrap();
        assert_eq!(mesh.parts.len(), 2, "{extension}");
        assert_eq!(mesh.triangle_count(), 24);
        let solids: Vec<_> = mesh.parts.iter().map(|p| Solid::new(p).unwrap()).collect();
        for point in [[0.0; 3], [5.0, 0.0, 0.0]] {
            assert!(
                solids.iter().any(|s| s.contains(point)),
                "{extension}: {point:?}"
            );
        }
        assert!(solids.iter().all(|s| !s.contains([2.5, 0.0, 0.0])));
        for (part, expected) in mesh.parts.iter().zip(&original.parts) {
            let bounds = Solid::new(part).unwrap().bounds();
            let exact = Solid::new(expected).unwrap().bounds();
            assert!(
                bounds
                    .iter()
                    .flatten()
                    .zip(exact.iter().flatten())
                    .all(|(a, b)| (a - b).abs() < 1e-5),
                "{extension}"
            );
        }
    }
}

#[test]
fn ascii_stl_and_source_unit_affine_reflection() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("ascii.stl");
    let cube = cube(0.0, 1.0);
    let mut file = fs::File::create(&path).unwrap();
    writeln!(file, "solid fixture").unwrap();
    for (face, triangle) in cube.triangles.iter().enumerate() {
        writeln!(file, "facet normal 0 0 0\nouter loop").unwrap();
        for &i in triangle {
            let [x, y, z] =
                cube.vertices[i].map(|v| if v == 0.0 && face % 2 == 0 { -0.0 } else { v });
            writeln!(file, "vertex {x} {y} {z}").unwrap();
        }
        writeln!(file, "endloop\nendfacet").unwrap();
    }
    writeln!(file, "endsolid fixture").unwrap();
    let transform = Transform([
        [-1.0, 0.0, 0.0, 5.0],
        [0.0, 1.0, 0.0, 6.0],
        [0.0, 0.0, 1.0, 7.0],
        [0.0, 0.0, 0.0, 1.0],
    ]);
    let mesh = read_mesh(
        &path,
        ImportOptions {
            units: Some(Unit::Inch),
            transform,
        },
    )
    .unwrap();
    assert_eq!(mesh.parts.len(), 1);
    assert_eq!(inspect(&mesh.parts[0]).unwrap(), Default::default());
    let bounds = Solid::new(&mesh.parts[0]).unwrap().bounds();
    assert_eq!(bounds, [[-20.4, 6.0, 7.0], [5.0, 31.4, 32.4]]);
}

#[test]
fn ply_ascii_binary_both_endians_and_object_groups() {
    let dir = tempdir().unwrap();
    let cube = cube(0.0, 1.0);
    for format in ["ascii", "binary_little_endian", "binary_big_endian"] {
        let mut content = format!("ply\nformat {format} 1.0\ncomment fixture\nelement vertex 8\nproperty double x\nproperty double y\nproperty double z\nelement face 12\nproperty list uchar int vertex_indices\nproperty int object_id\nend_header\n").into_bytes();
        if format == "ascii" {
            for [x, y, z] in &cube.vertices {
                content.extend_from_slice(format!("{x} {y} {z}\n").as_bytes());
            }
            for [a, b, c] in &cube.triangles {
                content.extend_from_slice(format!("3 {a} {b} {c} 7\n").as_bytes());
            }
        } else {
            let little = format == "binary_little_endian";
            for &v in cube.vertices.iter().flatten() {
                content.extend_from_slice(&if little {
                    v.to_le_bytes()
                } else {
                    v.to_be_bytes()
                });
            }
            for face in &cube.triangles {
                content.push(3);
                for v in face.map(|i| i as i32).into_iter().chain([7]) {
                    content.extend_from_slice(&if little {
                        v.to_le_bytes()
                    } else {
                        v.to_be_bytes()
                    });
                }
            }
        }
        let path = dir.path().join(format!("{format}.ply"));
        fs::write(&path, content).unwrap();
        let mesh = read_mesh(&path, ImportOptions::default()).unwrap();
        assert_eq!(mesh.parts[0].name, "ply-7");
        assert_eq!(mesh.parts[0].vertices, cube.vertices);
        assert_eq!(mesh.parts[0].triangles, cube.triangles);
    }
}

#[test]
fn threemf_relationship_components_units_and_reflected_instances() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("instances.3mf");
    let cube = cube(0.0, 1.0);
    let mut xml = String::from(
        "<model unit=\"centimeter\" xmlns=\"http://schemas.microsoft.com/3dmanufacturing/core/2015/02\"><resources><object id=\"1\" name=\"cube &amp; lens\" type=\"model\"><mesh><vertices>",
    );
    for [x, y, z] in cube.vertices {
        xml.push_str(&format!("<vertex x=\"{x}\" y=\"{y}\" z=\"{z}\"/>"));
    }
    xml.push_str("</vertices><triangles>");
    for [a, b, c] in cube.triangles {
        xml.push_str(&format!("<triangle v1=\"{a}\" v2=\"{b}\" v3=\"{c}\"/>"));
    }
    xml.push_str("</triangles></mesh></object><object id=\"2\" name=\"assembly\" type=\"model\"><components><component objectid=\"1\" transform=\"1 0 0 0 1 0 0 0 1 2 0 0\"/></components></object></resources><build><item objectid=\"2\" transform=\"1 0 0 0 1 0 0 0 1 10 0 0\"/><item objectid=\"2\" transform=\"-1 0 0 0 1 0 0 0 1 -10 0 0\"/></build></model>");
    let mut archive = zip::ZipWriter::new(fs::File::create(&path).unwrap());
    let options = zip::write::SimpleFileOptions::default();
    archive.start_file("_rels/.rels", options).unwrap();
    archive.write_all(br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="r0" Type="http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel" Target="/3D/main.model"/></Relationships>"#).unwrap();
    archive.start_file("3D/main.model", options).unwrap();
    archive.write_all(xml.as_bytes()).unwrap();
    archive.finish().unwrap();
    let mesh = read_mesh(&path, ImportOptions::default()).unwrap();
    assert_eq!(mesh.parts.len(), 2);
    assert!(mesh.parts[0].name.ends_with("assembly/cube & lens"));
    assert_eq!(
        Solid::new(&mesh.parts[0]).unwrap().bounds(),
        [[120.0, 0.0, 0.0], [130.0, 10.0, 10.0]]
    );
    assert_eq!(
        Solid::new(&mesh.parts[1]).unwrap().bounds(),
        [[-130.0, 0.0, 0.0], [-120.0, 10.0, 10.0]]
    );
    assert_eq!(inspect(&mesh.parts[1]).unwrap(), Default::default());
    let override_units = read_mesh(
        &path,
        ImportOptions {
            units: Some(Unit::Millimeter),
            ..ImportOptions::default()
        },
    )
    .unwrap();
    assert_eq!(
        Solid::new(&override_units.parts[0]).unwrap().bounds()[0],
        [12.0, 0.0, 0.0]
    );
}

#[test]
fn gltf_nested_static_nodes_and_external_buffer() {
    use base64::Engine;
    let dir = tempdir().unwrap();
    let path = dir.path().join("scene.gltf");
    write_mesh(
        &path,
        &Mesh {
            parts: vec![cube(0.0, 1.0)],
        },
    )
    .unwrap();
    let mut json: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let uri = json["buffers"][0]["uri"].as_str().unwrap();
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(uri.split_once(',').unwrap().1)
        .unwrap();
    fs::write(dir.path().join("mesh data.bin"), bytes).unwrap();
    json["buffers"][0]["uri"] = serde_json::json!("mesh%20data.bin");
    json["nodes"][0]["translation"] = serde_json::json!([0.002, 0.003, 0.004]);
    json["nodes"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({"children":[0],"translation":[0.01,0,0],"scale":[-1,1,1]}));
    json["scenes"][0]["nodes"] = serde_json::json!([1]);
    fs::write(&path, serde_json::to_vec(&json).unwrap()).unwrap();
    let mesh = read_mesh(&path, ImportOptions::default()).unwrap();
    let bounds = Solid::new(&mesh.parts[0]).unwrap().bounds();
    for (a, b) in bounds
        .iter()
        .flatten()
        .zip([[7.0, 3.0, 4.0], [8.0, 4.0, 5.0]].iter().flatten())
    {
        assert!((a - b).abs() < 1e-5, "{bounds:?}");
    }
    assert_eq!(inspect(&mesh.parts[0]).unwrap(), Default::default());
}

#[test]
fn malformed_input_returns_errors() {
    let dir = tempdir().unwrap();
    for (name,bytes) in [
        ("broken.stl",b"solid broken".as_slice()),
        ("broken.ply",b"ply\nformat binary_little_endian 1.0\nelement vertex 1\nproperty float x\nproperty float y\nproperty float z\nend_header\n".as_slice()),
        ("broken.glb",b"glTF".as_slice()),
        ("broken.3mf",b"not zip".as_slice()),
    ] {
        let path = dir.path().join(name);
        fs::write(&path,bytes).unwrap();
        assert!(read_mesh(&path,ImportOptions::default()).is_err(),"{name}");
    }
}

#[test]
fn material_primitives_do_not_split_one_closed_object() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("materials.gltf");
    write_mesh(
        &path,
        &Mesh {
            parts: vec![cube(0.0, 1.0)],
        },
    )
    .unwrap();
    let mut document: serde_json::Value =
        serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    document["accessors"][1]["count"] = serde_json::json!(18);
    let mut second = document["accessors"][1].clone();
    second["byteOffset"] = serde_json::json!(72);
    document["accessors"].as_array_mut().unwrap().push(second);
    let mut primitive = document["meshes"][0]["primitives"][0].clone();
    primitive["indices"] = serde_json::json!(2);
    document["meshes"][0]["primitives"]
        .as_array_mut()
        .unwrap()
        .push(primitive);
    fs::write(&path, serde_json::to_vec(&document).unwrap()).unwrap();
    let mesh = read_mesh(&path, ImportOptions::default()).unwrap();
    assert_eq!(mesh.parts.len(), 1);
    assert_eq!(mesh.triangle_count(), 12);
    assert!(Solid::new(&mesh.parts[0]).unwrap().contains([0.5; 3]));

    let path = dir.path().join("materials.obj");
    write_mesh(
        &path,
        &Mesh {
            parts: vec![cube(0.0, 1.0)],
        },
    )
    .unwrap();
    let text = fs::read_to_string(&path).unwrap();
    let mut faces = 0;
    let mut material_obj = String::from("mtllib colors.mtl\n");
    for line in text.lines() {
        if line.starts_with("f ") {
            if faces == 0 {
                material_obj.push_str("usemtl first\n");
            }
            if faces == 6 {
                material_obj.push_str("usemtl second\n");
            }
            faces += 1;
        }
        material_obj.push_str(line);
        material_obj.push('\n');
    }
    fs::write(
        dir.path().join("colors.mtl"),
        "newmtl first\nKd 1 0 0\nnewmtl second\nKd 0 1 0\n",
    )
    .unwrap();
    fs::write(&path, material_obj).unwrap();
    let mesh = read_mesh(&path, ImportOptions::default()).unwrap();
    assert_eq!(mesh.parts.len(), 1);
    assert!(Solid::new(&mesh.parts[0]).unwrap().contains([0.5; 3]));
}

#[test]
fn concave_obj_and_ply_polygons_preserve_the_notch_and_winding() {
    let dir = tempdir().unwrap();
    let polygon = [
        [2.0, 1.0],
        [1.0, 1.0],
        [1.0, 2.0],
        [0.0, 2.0],
        [0.0, 0.0],
        [2.0, 0.0],
    ];
    let vertices: Vec<[f64; 3]> = [0.0, 1.0]
        .into_iter()
        .flat_map(|z| polygon.map(|[x, y]| [x, y, z]))
        .collect();
    let mut faces: Vec<Vec<usize>> = vec![(0..6).rev().collect(), (6..12).collect()];
    for i in 0..6 {
        faces.push(vec![i, (i + 1) % 6, (i + 1) % 6 + 6, i + 6]);
    }
    for extension in ["obj", "ply"] {
        let mut text = if extension == "obj" {
            "o concave\n".to_owned()
        } else {
            "ply\nformat ascii 1.0\nelement vertex 12\nproperty double x\nproperty double y\nproperty double z\nelement face 8\nproperty list uchar int vertex_indices\nend_header\n".to_owned()
        };
        for [x, y, z] in &vertices {
            text.push_str(&format!(
                "{}{x} {y} {z}\n",
                if extension == "obj" { "v " } else { "" }
            ));
        }
        for face in &faces {
            let prefix = if extension == "obj" {
                "f".to_owned()
            } else {
                face.len().to_string()
            };
            text.push_str(&prefix);
            for &i in face {
                text.push_str(&format!(" {}", if extension == "obj" { i + 1 } else { i }));
            }
            text.push('\n');
        }
        let path = dir.path().join(format!("concave.{extension}"));
        fs::write(&path, text).unwrap();
        let mesh = read_mesh(&path, ImportOptions::default()).unwrap();
        assert_eq!(inspect(&mesh.parts[0]).unwrap(), Default::default());
        let solid = Solid::new(&mesh.parts[0]).unwrap();
        assert!(!solid.contains([1.25, 1.25, 0.5]), "{extension}");
        assert!(solid.contains([0.5, 1.5, 0.5]), "{extension}");
        assert!(solid.contains([1.5, 0.5, 0.5]), "{extension}");
    }
}

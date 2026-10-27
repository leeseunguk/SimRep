use simrep_mesh::{Mesh, Part, Trajectory, TrajectoryPoint, write_mesh, write_usda};
use std::fs;
use std::process::Command;

fn mesh() -> Mesh {
    let part = Part {
        name: "electrode \"7\"\\source\nline".into(),
        vertices: vec![[0.0, 1.0, 2.0], [3.0, 1.0, 2.0], [0.0, 4.0, 2.0]],
        triangles: vec![[0, 1, 2]],
    };
    Mesh {
        parts: vec![part.clone(), part],
    }
}

fn trajectories() -> Vec<Trajectory> {
    let point = |time_us| TrajectoryPoint {
        time_us,
        position_mm: [time_us, 2.0 * time_us, -time_us],
        velocity_mm_per_us: [1.123456789012345, 2.0, -1.0],
    };
    vec![
        Trajectory {
            id: u64::MAX,
            points: vec![point(0.0), point(0.5)],
        },
        Trajectory {
            id: 2,
            points: vec![point(1.0), point(1.5), point(2.0)],
        },
    ]
}

// Decode the numeric USDA arrays, independently of their whitespace layout.
fn arrays(text: &str, name: &str) -> Vec<Vec<f64>> {
    text.lines()
        .filter_map(|line| {
            let (declaration, value) = line.split_once(" = [")?;
            (declaration.split_whitespace().last()? == name).then(|| {
                value
                    .split(']')
                    .next()
                    .unwrap()
                    .replace(['(', ')'], "")
                    .split(',')
                    .map(|v| v.trim().parse::<f64>().unwrap())
                    .collect()
            })
        })
        .collect()
}

#[test]
fn combined_usda_preserves_topology_units_names_and_sample_attributes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("scene.usda");
    let mesh = mesh();
    let paths = trajectories();
    write_usda(&path, Some(&mesh), &paths).unwrap();
    let text = fs::read_to_string(&path).unwrap();
    assert_eq!(text.lines().next(), Some("#usda 1.0"));
    assert!(text.contains("metersPerUnit = 0.001"));
    assert!(text.contains("upAxis = \"Z\""));
    assert!(text.contains("defaultPrim = \"SimRep\""));
    for (i, name) in text
        .lines()
        .filter_map(|s| s.trim().strip_prefix("def Mesh "))
        .enumerate()
    {
        assert_eq!(name, format!("\"part_{i}\" ("));
    }
    let names: Vec<String> = text
        .lines()
        .filter_map(|s| s.trim().strip_prefix("displayName = "))
        .map(|v| serde_json::from_str(v).unwrap())
        .collect();
    assert_eq!(
        names,
        mesh.parts
            .iter()
            .map(|p| p.name.clone())
            .collect::<Vec<_>>()
    );
    assert_eq!(arrays(&text, "faceVertexCounts"), [vec![3.0], vec![3.0]]);
    assert_eq!(
        arrays(&text, "faceVertexIndices"),
        [vec![0.0, 1.0, 2.0], vec![0.0, 1.0, 2.0]]
    );
    let positions = arrays(&text, "points");
    assert_eq!(positions.len(), 3);
    assert_eq!(
        positions[0],
        mesh.parts[0]
            .vertices
            .iter()
            .flatten()
            .copied()
            .collect::<Vec<_>>()
    );
    assert_eq!(positions[1], positions[0]);
    assert_eq!(
        positions[2],
        paths
            .iter()
            .flat_map(|t| &t.points)
            .flat_map(|p| p.position_mm)
            .collect::<Vec<_>>()
    );
    assert_eq!(arrays(&text, "curveVertexCounts"), [vec![2.0, 3.0]]);
    // Parse ids as integers, not f64, to prove all 64 bits survive.
    let ids = text
        .lines()
        .find_map(|s| s.trim().strip_prefix("uint64[] primvars:trajectory_id = "))
        .unwrap()
        .split(" (")
        .next()
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Vec<u64>>(ids).unwrap(),
        [u64::MAX, 2]
    );
    assert_eq!(
        arrays(&text, "primvars:time_us"),
        [vec![0.0, 0.5, 1.0, 1.5, 2.0]]
    );
    assert_eq!(
        arrays(&text, "primvars:velocity_mm_per_us")[0],
        paths
            .iter()
            .flat_map(|t| &t.points)
            .flat_map(|p| p.velocity_mm_per_us)
            .collect::<Vec<_>>()
    );
    assert_eq!(arrays(&text, "widths"), [vec![0.1]]);
    let bounds = arrays(&text, "extent");
    assert_eq!(bounds[0], [0.0, 1.0, 2.0, 3.0, 4.0, 2.0]);
    for p in positions[2].as_chunks::<3>().0 {
        for axis in 0..3 {
            assert!(bounds[2][axis] <= p[axis] - 0.049);
            assert!(bounds[2][axis + 3] >= p[axis] + 0.049);
        }
    }
    let repeated = dir.path().join("repeated.usda");
    write_usda(&repeated, Some(&mesh), &paths).unwrap();
    assert_eq!(fs::read(path).unwrap(), fs::read(repeated).unwrap());
}

#[test]
fn mesh_dispatch_and_curve_only_scenes_have_no_external_assets() {
    let dir = tempfile::tempdir().unwrap();
    let mesh_path = dir.path().join("mesh.USDA");
    write_mesh(&mesh_path, &mesh()).unwrap();
    let text = fs::read_to_string(mesh_path).unwrap();
    assert_eq!(arrays(&text, "points").len(), 2);
    assert!(arrays(&text, "curveVertexCounts").is_empty());
    assert!(!text.contains('@'));

    let path = dir.path().join("curves.usda");
    write_usda(&path, None, &trajectories()).unwrap();
    let text = fs::read_to_string(path).unwrap();
    assert!(arrays(&text, "faceVertexCounts").is_empty());
    assert_eq!(arrays(&text, "points").len(), 1);
    assert!(!text.contains('@'));
}

#[test]
fn invalid_usd_data_is_rejected_before_creating_an_output() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("invalid.usda");
    assert!(write_usda(&path, None, &[]).is_err());
    let mut mesh = mesh();
    mesh.parts[0].vertices[0][0] = f64::MAX;
    assert!(write_usda(&path, Some(&mesh), &[]).is_err());
    mesh.parts[0].vertices[0][0] = f64::NAN;
    assert!(write_usda(&path, Some(&mesh), &[]).is_err());
    mesh.parts[0].vertices[0][0] = 0.0;
    mesh.parts[0].triangles[0][0] = usize::MAX;
    assert!(write_usda(&path, Some(&mesh), &[]).is_err());

    let mut paths = trajectories();
    paths[1].id = paths[0].id;
    assert!(write_usda(&path, None, &paths).is_err());
    paths = trajectories();
    paths[0].points.reverse();
    assert!(write_usda(&path, None, &paths).is_err());
    paths = trajectories();
    paths[0].points.truncate(1);
    assert!(write_usda(&path, None, &paths).is_err());
    paths = trajectories();
    paths[0].points[0].velocity_mm_per_us[0] = f64::NAN;
    assert!(write_usda(&path, None, &paths).is_err());
    paths = trajectories();
    paths[0].points[0].position_mm[0] = f64::MAX;
    assert!(write_usda(&path, None, &paths).is_err());
    assert!(!path.exists());
}

#[test]
fn source_name_controls_use_usd_octal_escapes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("names.usda");
    let mut mesh = mesh();
    mesh.parts[0].name = "control\u{1}\0\u{7f}\u{03c6}".into();
    write_usda(&path, Some(&mesh), &[]).unwrap();
    let text = fs::read_to_string(path).unwrap();
    assert!(text.contains("displayName = \"control\\001\\000\\177\u{03c6}\""));
}

#[test]
fn openusd_round_trip_when_pxr_is_available() {
    let python = std::env::var("SIMREP_USD_PYTHON").unwrap_or_else(|_| "python3".into());
    let probe = Command::new(&python)
        .args([
            "-c",
            "import importlib.util; raise SystemExit(0 if importlib.util.find_spec('pxr') else 77)",
        ])
        .output();
    match probe {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
        Ok(output) if output.status.code() == Some(77) => return,
        Ok(output) => assert!(output.status.success(), "{output:?}"),
        Err(e) => panic!("cannot probe OpenUSD: {e}"),
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("scene.usda");
    let mut mesh = mesh();
    mesh.parts[1].name = "control\u{1}\0\u{7f}\u{03c6}".into();
    write_usda(&path, Some(&mesh), &trajectories()).unwrap();
    let output = Command::new(python)
        .args([
            "-c",
            r#"import sys
from pxr import Usd, UsdGeom
stage = Usd.Stage.Open(sys.argv[1])
assert stage.GetDefaultPrim().GetPath() == '/SimRep'
assert UsdGeom.GetStageMetersPerUnit(stage) == 0.001
assert UsdGeom.GetStageUpAxis(stage) == 'Z'
meshes = [UsdGeom.Mesh(p) for p in stage.Traverse() if p.IsA(UsdGeom.Mesh)]
assert len(meshes) == 2
for i, mesh in enumerate(meshes):
    assert list(mesh.GetFaceVertexCountsAttr().Get()) == [3]
    assert list(mesh.GetFaceVertexIndicesAttr().Get()) == [0, 1, 2]
    assert tuple(mesh.GetPointsAttr().Get()[1]) == (3, 1, 2)
    assert mesh.GetSubdivisionSchemeAttr().Get() == 'none'
    name = 'electrode "7"\\source\nline' if i == 0 else 'control'+chr(1)+chr(0)+chr(127)+chr(0x03c6)
    assert mesh.GetPrim().GetDisplayName() == name
curves = UsdGeom.BasisCurves(stage.GetPrimAtPath('/SimRep/Trajectories'))
assert curves.GetTypeAttr().Get() == 'linear'
assert curves.GetWrapAttr().Get() == 'nonperiodic'
assert list(curves.GetCurveVertexCountsAttr().Get()) == [2, 3]
assert len(curves.GetPointsAttr().Get()) == 5
assert curves.GetWidthsInterpolation() == 'constant'
primvars = UsdGeom.PrimvarsAPI(curves)
ids = primvars.GetPrimvar('trajectory_id')
assert ids.GetInterpolation() == 'uniform'
assert list(ids.Get()) == [2**64-1, 2]
times = primvars.GetPrimvar('time_us')
assert times.GetInterpolation() == 'vertex'
assert list(times.Get()) == [0, 0.5, 1, 1.5, 2]
velocity = primvars.GetPrimvar('velocity_mm_per_us').Get()
assert tuple(velocity[0]) == (1.123456789012345, 2, -1)
stage.Export(sys.argv[2])
assert Usd.Stage.Open(sys.argv[2])
"#,
        ])
        .arg(path)
        .arg(dir.path().join("roundtrip.usda"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "OpenUSD round trip failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

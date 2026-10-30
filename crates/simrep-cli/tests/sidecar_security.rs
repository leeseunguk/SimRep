//! A provenance sidecar must never follow an attacker-controlled link.
#[cfg(unix)]
#[test]
fn flight_refuses_existing_and_dangling_sidecar_links() {
    use std::{fs, process::Command};
    let dir = tempfile::tempdir().unwrap();
    let ions = dir.path().join("ions.ion");
    fs::write(&ions, b"0,1,1,0,0,0,0,0,1\n").unwrap();
    for dangling in [false, true] {
        let csv = dir.path().join(format!("flight-{dangling}.csv"));
        let sidecar = csv.with_extension("csv.json");
        let outside = dir.path().join(format!("outside-{dangling}.json"));
        if !dangling {
            fs::write(&outside, b"KEEP").unwrap();
        }
        std::os::unix::fs::symlink(&outside, &sidecar).unwrap();
        let result = Command::new(env!("CARGO_BIN_EXE_simrep"))
            .args(["--json", "fly", "--particles"])
            .arg(&ions)
            .args(["--plane", "1,0,0,1", "--device", "cpu", "--output"])
            .arg(&csv)
            .output()
            .unwrap();
        assert!(
            !result.status.success(),
            "sidecar link must be refused, stdout={}",
            String::from_utf8_lossy(&result.stdout)
        );
        assert!(
            fs::symlink_metadata(&sidecar)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        if dangling {
            assert!(!outside.exists());
        } else {
            assert_eq!(fs::read(&outside).unwrap(), b"KEEP");
        }
    }
}

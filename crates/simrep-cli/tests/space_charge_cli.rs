use simrep_core::PotentialArray;
use simrep_io::{PaFile, write_pa};
use std::{fs, process::Command};

#[test]
fn direct_and_pic_cli_carry_charge_weights_and_reach_the_detector() {
    let dir = tempfile::tempdir().unwrap();
    write_pa(
        dir.path().join("blank.pa"),
        &PaFile::new(PotentialArray::new(7, 7, 7)),
    )
    .unwrap();
    fs::write(
        dir.path().join("bunch.ion"),
        "0,100,1,1,2,3,0,0,1,1\n0,100,1,1,4,3,0,0,1,1\n",
    )
    .unwrap();
    let mut direct = Vec::new();
    for (method, mode, amount) in [
        ("direct", "factor", "100000"),
        ("direct", "coulombic", "3.204353268e-14"),
        ("pic", "coulombic", "3.204353268e-14"),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_simrep"))
            .current_dir(dir.path())
            .env("RAYON_NUM_THREADS", "4")
            .args([
                "fly",
                "--pa",
                "blank.pa",
                "--particles",
                "bunch.ion",
                "--bounds=0,0,0,6,6,6",
                "--plane=1,0,0,4",
                "--nonrelativistic",
                "--duration-us",
                "10",
                "--max-step-us",
                "0.2",
                "--space-charge",
                method,
                "--charge-mode",
                mode,
                "--charge-amount",
                amount,
                "--pic-dims",
                "9,9,9",
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let text = String::from_utf8(output.stdout).unwrap();
        let rows: Vec<Vec<_>> = text
            .lines()
            .skip(1)
            .map(|line| line.split(',').collect())
            .collect();
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|r| r[6] == "plane"));
        let y0: f64 = rows[0][3].parse().unwrap();
        let y1: f64 = rows[1][3].parse().unwrap();
        assert!(y0 < 2.0 && y1 > 4.0, "{text}");
        if method == "direct" {
            direct.push([y0, y1]);
        }
    }
    for (a, b) in direct[0].iter().zip(direct[1]) {
        assert!((a - b).abs() < 1e-12);
    }
}

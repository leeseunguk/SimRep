"""The Python merger shares the CLI's lossless provenance-checked assembly."""
import json
import os
from pathlib import Path
import shutil
import subprocess

import pytest
import simrep


def cli(*args):
    root = Path(__file__).resolve().parents[3]
    binary = Path(os.environ.get("SIMREP_BIN", root / "target/release/simrep"))
    return json.loads(subprocess.run(
        [str(binary), "--json", *map(str, args)], check=True, capture_output=True,
        text=True, stdin=subprocess.DEVNULL, timeout=120,
        env=dict(os.environ, RAYON_NUM_THREADS="2")).stdout)


@pytest.mark.parametrize("count", [1, 2, 3, 7])
def test_python_merges_real_cli_shards(tmp_path, count):
    ions = tmp_path / "ions.ion"
    ions.write_text("".join(f"{k % 3 * 0.01},40,1,0,0,0,0,0,{80+k}\n" for k in range(15)))
    base = ["fly", "--device=cpu", f"--particles={ions}", "--bounds=-1,-1,-1,3,1,1",
            "--plane=1,0,0,1", "--duration-us=10", "--sample-every=3"]
    full_csv = tmp_path / "full.csv"
    full_trajectory = tmp_path / "full-trajectories.csv"
    full = cli(*base, f"--output={full_csv}", f"--trajectories={full_trajectory}")
    shards = []
    times = []
    for index in range(1, count + 1):
        path = tmp_path / f"{index}.csv"
        report = cli(*base, f"--shard={index}/{count}", f"--output={path}",
                     f"--trajectories={tmp_path / f'{index}-trajectories.csv'}")
        shards.append(path)
        times.append(report["compute_ms"])
    output = tmp_path / "merged.csv"
    result = simrep.merge_flights(output, shards)
    assert output.read_bytes() == full_csv.read_bytes()
    assert (tmp_path / "merged.trajectories.csv").read_bytes() == full_trajectory.read_bytes()
    for key in ("arrival_tof_us", "arrivals", "splat_counts", "ion_count", "run_hash"):
        assert result[key] == full[key]
    # Rust accumulates in input order; Python's sum may use compensation.
    expected_sum = 0.0
    for value in times:
        expected_sum += value
    assert result["compute_ms_sum"] == expected_sum
    assert result["compute_ms_max"] == max(times)
    assert json.loads((tmp_path / "merged.csv.json").read_text())["summary"] == result
    with pytest.raises(ValueError, match="duplicate shard"):
        simrep.merge_flights(tmp_path / "bad.csv", [shards[0], shards[0]])
    assert not (tmp_path / "bad.csv").exists()
    if count > 1:
        with pytest.raises(ValueError, match="missing shard"):
            simrep.merge_flights(tmp_path / "bad.csv", shards[:-1])


def test_shards_and_nested_trajectories_can_move_to_another_machine_directory(tmp_path):
    source = tmp_path / "worker"
    source.mkdir()
    trajectories = source / "trajectories"
    trajectories.mkdir()
    ions = source / "ions.ion"
    ions.write_text("0,40,1,0,0,0,0,0,100\n0.01,40,1,0,0,0,0,0,110\n")
    base = ["fly", "--device=cpu", f"--particles={ions}", "--bounds=-1,-1,-1,3,1,1",
            "--plane=1,0,0,1", "--duration-us=10"]
    cli(*base, f"--output={source / 'full.csv'}", f"--trajectories={trajectories / 'full.csv'}")
    for index in (1, 2):
        cli(*base, f"--shard={index}/2", f"--output={source / f'{index}.csv'}",
            f"--trajectories={trajectories / f'{index}.csv'}")
    destination = tmp_path / "collector"
    shutil.copytree(source, destination)
    # Prove that merging cannot fall back to absolute paths on the worker.
    source.rename(tmp_path / "unavailable-worker")
    summary = simrep.merge_flights(destination / "merged.csv",
                                  [destination / "2.csv", destination / "1.csv"])
    assert summary["ion_count"] == 2
    assert (destination / "merged.csv").read_bytes() == (destination / "full.csv").read_bytes()
    assert (destination / "merged.trajectories.csv").read_bytes() == (destination / "trajectories/full.csv").read_bytes()

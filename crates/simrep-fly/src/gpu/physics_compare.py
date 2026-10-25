"""Compare CPU/GPU neutral-gas distributions with exact two-sample KS tests."""
import argparse
import csv
import json
from pathlib import Path

from scipy.stats import ks_2samp


def columns(path):
    with path.open(newline="") as stream:
        rows = list(csv.DictReader(stream))
    return {
        key: [float(row[key]) for row in rows]
        for key in ("x_mm", "y_mm", "z_mm", "tof_us", "ke_ev")
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("oracle_root", type=Path)
    parser.add_argument("--preview", action="store_true")
    args = parser.parse_args()
    output = {}
    for name in ("hs1", "sds"):
        cpu = columns(args.oracle_root / "gpu-physics" / f"{name}-cpu.csv")
        suffix = "-preview" if args.preview else ""
        gpu = columns(args.oracle_root / "gpu-physics" / f"{name}-gpu{suffix}.csv")
        results = {}
        for key in cpu:
            a, b = cpu[key], gpu[key]
            # TOF and SDS KE are degenerate atoms. Check the prescribed common
            # termination time before comparing its distribution.
            if key == "tof_us":
                assert max(abs(v - 50.0) for v in a + b) < 1e-10
                a = [50.0] * len(a)
                b = [50.0] * len(b)
            result = ks_2samp(a, b, method="exact")
            results[key] = {
                "D": float(result.statistic),
                "p_value": float(result.pvalue),
                "cpu_ions": len(a),
                "gpu_ions": len(b),
            }
            assert result.pvalue >= 0.05, (name, key, results[key])
        output[name] = results
    print(json.dumps(output, indent=2))


if __name__ == "__main__":
    main()

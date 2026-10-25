#!/usr/bin/env python3
"""Generate the deterministic 100 amu, 200 eV einzel benchmark beam."""

import argparse
from pathlib import Path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    parser.add_argument("--ions", type=int, default=1_000_000)
    args = parser.parse_args()
    if args.ions < 2:
        parser.error("--ions must be at least two")
    with args.output.open("w", newline="") as stream:
        stream.write("; TOB, MASS, CHARGE, X, Y, Z, AZ, EL, KE, CWF, COLOR\n")
        for i in range(args.ions):
            y = -15.0 + 30.0*i/(args.ions-1)
            stream.write(f"0,100,1,0,{y:.6f},0,0,0,200,1,1\n")
    print(f"{args.ions} ions: {args.output}")


if __name__ == "__main__":
    main()

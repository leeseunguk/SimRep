#!/usr/bin/env python3
"""Generate the exact deterministic 100 amu, 100 eV quadrupole template beam."""
import argparse
import math
from pathlib import Path

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("output", type=Path)
parser.add_argument("--ions", type=int, default=1000)
args = parser.parse_args()
if args.ions < 2:
    parser.error("--ions must be >= 2")
with args.output.open("w") as stream:
    stream.write("; TOB,MASS,CHARGE,X,Y,Z,AZ,EL,KE,CWF,COLOR\n")
    for i in range(args.ions):
        radius = 3.0 * math.sqrt(i / (args.ions - 1))
        angle = i * 2.399963229728653
        y, z = radius * math.cos(angle), radius * math.sin(angle)
        stream.write(f"0,100,1,1,{y:.17e},{z:.17e},0,0,100,1,1\n")

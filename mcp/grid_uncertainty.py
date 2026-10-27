#!/usr/bin/env python3
"""Fixed argv-only bridge to SimRep's native Celik helper; no alternate estimator."""
import json
import sys

from simrep import uncertainty


def main():
    data = json.loads(sys.argv[1])
    result = uncertainty.observed_order(data["h"], data["values"])
    gci = result["gci_fine"]
    result["u_num"] = None if gci is None else abs(data["values"][0]) * gci
    result["estimator"] = "Celik GCI (relative GCI converted to absolute u_num)"
    print(json.dumps(result, allow_nan=False))


if __name__ == "__main__":
    main()

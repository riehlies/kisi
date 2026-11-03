#!/usr/bin/env python3
"""Thin wrapper of compare.py: CST's attenuation (lossy port modes) against kisi's line model and the full-wave sweep.

    python3 validation/cst_xs/compare_loss.py [--solve] [--jobs 2] [--ids a,b] [--md out.md] [--json out.json]

--solve runs kisi's FDM solves of the line model first (compare.py --solve-loss → kisi_loss.json); the comparison
is compare.py's loss section (--loss-table: per case and frequency).
"""
import os, sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import compare  # noqa: E402

if __name__ == "__main__":
    argv = sys.argv[1:]
    if "--solve" in argv:
        argv.remove("--solve")
        keep = [a for a in argv if a not in ("--loss-table",)]
        compare.main(["--solve-loss"] + [a for i, a in enumerate(keep) if a in ("--jobs", "--ids") or (i and keep[i - 1] in ("--jobs", "--ids"))])
    compare.main(["--loss-table"] + argv)

#!/usr/bin/env python3
"""Thin wrapper of compare.py for the full-wave workflow's summary (.github/workflows/fullwave-xs.yml): the same
comparison with FW's diagnostics (run times, sweep warnings).

    python3 validation/cst_xs/fullwave_compare.py [--per-case] [--sweep-table] [--lossy-table] [--md out.md] [--json out.json]
        [any other option of compare.py]
"""
import os, sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import compare  # noqa: E402

if __name__ == "__main__":
    argv = ["--loss-table" if a == "--lossy-table" else a for a in sys.argv[1:]]
    compare.main(["--fw-diag"] + argv)

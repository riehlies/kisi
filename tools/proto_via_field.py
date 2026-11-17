#!/usr/bin/env python3
"""Prototype via field viewer (docs/research/fdtd-field-view.md, "Prototype viewer"): build the self-contained page
dist/kisi-via-field-proto.html. Not part of kisi's product bundles.

  CARGO_TARGET_DIR=… RUSTFLAGS="-C target-feature=+simd128" \\
    cargo build --release -p kisi-fdtd --lib --target wasm32-unknown-unknown --features proto-viewer
  cp $CARGO_TARGET_DIR/wasm32-unknown-unknown/release/kisi_fdtd.wasm /tmp/kisi_fdtd_proto.wasm
  cargo build --release -p kisi-fdtd                      # native, for the embedded example
  python3 tools/proto_via_field.py /tmp/kisi_fdtd_proto.wasm $CARGO_TARGET_DIR/release/kisi-fdtd \\
    dist/kisi-via-field-proto.html [--work DIR] [--freqs 10,20,30] [--mantissa 7]

The page embeds the WebAssembly module (gzip + base64; it needs the `proto-viewer` feature: progress import and
`fd_mesh`) and one precomputed example: the default single via (tools/proto_via_field/case.js) run by the native
kisi-fdtd (bit-identical to WebAssembly) at --freqs, port 1 excited, stored as KVF with f16 fields (--mantissa bits;
7: 0.4 % rounding). Needs node.
"""
import argparse
import base64
import datetime
import gzip
import json
import os
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SRC = os.path.join(ROOT, "tools", "proto_via_field")


def node(script, *args):
    return subprocess.run(["node", "-e", script, *args], check=True, capture_output=True, text=True).stdout


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("wasm")
    ap.add_argument("kisi_fdtd")
    ap.add_argument("out")
    ap.add_argument("--work", default=os.path.join(ROOT, "target", "proto_via_field"))
    ap.add_argument("--freqs", default="10,20,30", help="GHz, the example's field frequencies")
    ap.add_argument("--mantissa", type=int, default=7)
    a = ap.parse_args()
    os.makedirs(a.work, exist_ok=True)
    freqs = [float(f) * 1e9 for f in a.freqs.split(",")]

    # the example: default single via, the page's f_loss rule (geometric mean of the field frequencies)
    case_path = os.path.join(a.work, "example_case.json")
    node(
        "const vm=require('vm'),fs=require('fs');vm.runInThisContext(fs.readFileSync(process.argv[1],'utf8'));"
        "const fs_=JSON.parse(process.argv[2]);const c=KVF.makeCase({},fs_);"
        "c.f_loss=Math.exp(fs_.reduce((a,f)=>a+Math.log(f),0)/fs_.length);"
        "c.volume={freqs:fs_,stride:1,every:0};c.volume_port=0;fs.writeFileSync(process.argv[3],JSON.stringify(c));",
        os.path.join(SRC, "case.js"), json.dumps(freqs), case_path)
    blob = os.path.join(a.work, "example_p1.bin")
    key = json.dumps([open(case_path).read(), os.path.getmtime(a.kisi_fdtd)])
    stamp = os.path.join(a.work, "example_p1.key")
    if not (os.path.exists(blob) and os.path.exists(stamp) and open(stamp).read() == key):
        head = subprocess.run([a.kisi_fdtd, case_path, "--volume", blob], check=True, capture_output=True, text=True).stdout
        h = json.loads(head)
        print(f"example: {h['cells']} cells, {h['steps']} steps, {h['seconds']:.1f} s native", file=sys.stderr)
        open(stamp, "w").write(key)
    kvf = os.path.join(a.work, "example.kvf")
    made = f"native kisi-fdtd, {datetime.date.today().isoformat()}"
    print(subprocess.run(["node", os.path.join(SRC, "encode.mjs"), case_path, kvf, str(a.mantissa), made, blob],
                         check=True, capture_output=True, text=True).stdout.strip(), file=sys.stderr)

    # copper conductivity from data/thermal.json (IEC 60028 annealed copper)
    th = json.load(open(os.path.join(ROOT, "data", "thermal.json")))
    rho = th["copper"]["rho_20c_ohm_m"]
    sigma_src = f"1/ρ with ρ = {rho:g} Ω·m from data/thermal.json (IEC 60028 annealed copper, 20 °C)"

    wasm_gz = gzip.compress(open(a.wasm, "rb").read(), 9, mtime=0)
    try:
        sha = subprocess.run(["git", "-C", ROOT, "rev-parse", "--short", "HEAD"], capture_output=True, text=True).stdout.strip()
    except OSError:
        sha = ""
    data = {
        "wasm": base64.b64encode(wasm_gz).decode(),
        "example": base64.b64encode(open(kvf, "rb").read()).decode(),
        "sigmaCu": 1.0 / rho,
        "sigmaSource": sigma_src,
        "worker": open(os.path.join(SRC, "worker.js")).read(),
        "build": f"Built {datetime.date.today().isoformat()} from {sha or 'a working tree'}; kisi_fdtd.wasm "
                 f"{len(wasm_gz) / 1024:.0f} KB gzip; example {os.path.getsize(kvf) / 2 ** 20:.2f} MB "
                 f"(f16, {a.mantissa}-bit mantissa).",
    }
    scripts = "<script>window.KVF_DATA = " + json.dumps(data) + ";</script>\n"
    for f in ("case.js", "fields.js", "view.js", "app.js"):
        js = open(os.path.join(SRC, f)).read()
        assert "</script" not in js, f
        scripts += f"<script>\n{js}\n</script>\n"
    page = open(os.path.join(SRC, "page.html")).read().replace("<!--SCRIPTS-->", scripts)
    os.makedirs(os.path.dirname(os.path.abspath(a.out)), exist_ok=True)
    open(a.out, "w").write(page)
    print(f"{a.out}: {len(page.encode()) / 2 ** 20:.2f} MB", file=sys.stderr)


if __name__ == "__main__":
    main()

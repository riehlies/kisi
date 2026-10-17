#!/bin/sh
# Full field-solver run from the command line.
# Usage: tools/kisi_field.sh <board.kicad_pcb> [all]   (default: only nets with a target)
set -e
cd "$(dirname "$0")/.."
B="$1"; T=$(mktemp -d)
target/release/kisi "$B" --geoms $T/g.json > /dev/null
if [ "$2" != "all" ]; then
  node -e 'const fs=require("fs");const g=JSON.parse(fs.readFileSync(process.argv[1]));fs.writeFileSync(process.argv[1],JSON.stringify(g.filter(x=>x.target_length>0)))' $T/g.json
fi
node tools/solve_xs.mjs $T/g.json $T/r.json
target/release/kisi "$B" --field $T/r.json

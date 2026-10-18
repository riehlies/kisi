#!/bin/sh
# Validate the field-solver chain (geometry generator + backend) against
# references and the stored baseline. Exit code != 0 on failure.
set -e
cd "$(dirname "$0")/.."
T=$(mktemp -d)
for set in forsten_suite thunderscope_xs; do
  target/release/kisi-validate validation/$set.json --geoms $T/$set.g.json > /dev/null
  node tools/solve_xs.mjs $T/$set.g.json $T/$set.r.json 2> /dev/null
done
target/release/kisi-validate validation/forsten_suite.json --field $T/forsten_suite.r.json
target/release/kisi-validate validation/thunderscope_xs.json validation/thunderscope_xs.ref.json --field $T/thunderscope_xs.r.json
node tools/compare_baseline.mjs validation/field_baseline.json $T/forsten_suite.r.json $T/thunderscope_xs.r.json
# the bundled viewer runs a classic-script conversion of the same modules: must give identical results
if [ -f dist/kisi.html ]; then
  node tools/solve_xs.mjs $T/thunderscope_xs.g.json $T/classic.r.json --classic dist/kisi.html 2> /dev/null
  node -e 'const fs=require("fs");const a=JSON.parse(fs.readFileSync(process.argv[1])),b=JSON.parse(fs.readFileSync(process.argv[2]));const k=["ok","z0","eeff","zodd","zeven"];const bad=a.filter((x,i)=>k.some(f=>x[f]!==b[i][f]));if(bad.length){console.error("classic bundle differs: "+bad.map(x=>x.key).join(", "));process.exit(1)}console.log("classic bundle (viewer) identical to ES modules: "+a.length+" cases")' $T/thunderscope_xs.r.json $T/classic.r.json
fi

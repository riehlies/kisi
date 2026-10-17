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

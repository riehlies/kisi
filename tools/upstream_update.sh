#!/bin/sh
# Update the vendored copy of js_2d_fields (third_party/js_2d_fields) and re-validate.
# Usage: tools/upstream_update.sh [commit|branch]   (default: main)
# The upstream files are copied unchanged; the revision is recorded in third_party/UPSTREAM_REV.
set -e
cd "$(dirname "$0")/.."
REV="${1:-main}"
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
git clone -q https://github.com/Ttl/js_2d_fields.git "$TMP/js_2d_fields"
git -C "$TMP/js_2d_fields" checkout -q "$REV"
git -C "$TMP/js_2d_fields" rev-parse HEAD > third_party/UPSTREAM_REV
rm -rf third_party/js_2d_fields
mkdir -p third_party/js_2d_fields
(cd "$TMP/js_2d_fields" && git archive HEAD) | tar -x -C third_party/js_2d_fields
echo "upstream at $(cat third_party/UPSTREAM_REV)"
cargo build --release -q -p kisi-cli
cargo build --release -q -p kisi-sparse --target wasm32-unknown-unknown
sh tools/validate_field.sh
echo "OK – review and commit third_party/js_2d_fields and third_party/UPSTREAM_REV"

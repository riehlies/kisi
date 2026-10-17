#!/bin/sh
# Update the vendored js_2d_fields solver and re-validate.
# Usage: tools/upstream_update.sh [commit]   (default: origin main)
set -e
cd "$(dirname "$0")/.."
REV="${1:-origin/main}"
git -C third_party/js_2d_fields fetch -q origin
git -C third_party/js_2d_fields checkout -q "$REV"
git -C third_party/js_2d_fields rev-parse HEAD > third_party/UPSTREAM_REV
echo "upstream at $(cat third_party/UPSTREAM_REV)"
cargo build --release -q -p kisi-cli
cargo build --release -q -p kisi-sparse --target wasm32-unknown-unknown
tools/validate_field.sh
echo "OK – commit third_party/js_2d_fields and third_party/UPSTREAM_REV"

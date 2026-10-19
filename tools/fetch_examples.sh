#!/bin/sh
# Fetch the example boards from their projects at fixed commits, shrink them with
# strip_pcb.py and write a notice (source, licence, modification) next to each:
#   <out>/Thunderscope_Rev5.3.kicad_pcb/.kicad_pro/.notice.txt   (CERN-OHL-P-2.0)
#   <out>/VNA.kicad_pcb/.kicad_pro/.notice.txt                   (GPL-3.0)
# Usage: tools/fetch_examples.sh <out-dir>; then tools/bundle.py ... ts "ThunderScope Rev5.3" <out>/Thunderscope_Rev5.3.kicad_pcb
#        <out>/Thunderscope_Rev5.3.kicad_pro vna LibreVNA <out>/VNA.kicad_pcb <out>/VNA.kicad_pro
# The power trees of the examples are in web/samples/{ts,vna}.kisi.json.
set -e
out=${1:?usage: fetch_examples.sh <out-dir>}
tools=$(cd "$(dirname "$0")" && pwd)
mkdir -p "$out"
raw=https://raw.githubusercontent.com

# fetch <repo> <commit> <dir in repo> <stem> <licence file> <licence name> <author>
fetch() {
  repo=$1 rev=$2 dir=$3 stem=$4 lic=$5 licname=$6 author=$7
  base="$raw/$repo/$rev"
  curl -fsSL "$base/$dir/$stem.kicad_pcb" -o "$out/$stem.full.kicad_pcb"
  curl -fsSL "$base/$dir/$stem.kicad_pro" -o "$out/$stem.kicad_pro"
  python3 "$tools/strip_pcb.py" "$out/$stem.full.kicad_pcb" "$out/$stem.kicad_pcb"
  rm "$out/$stem.full.kicad_pcb"
  {
    echo "Example board $stem: $author, https://github.com/$repo (commit $rev), licence $licname."
    echo "Modified by kisi (tools/strip_pcb.py, $(date -u +%Y-%m-%d)): reduced to stackup, nets, copper (tracks, vias, pads,"
    echo "zone fills), footprint fields and board outline; other graphics, texts and 3D models removed. The .kicad_pro is unmodified."
    echo
    curl -fsSL "$base/$lic"
  } > "$out/$stem.notice.txt"
  echo "$out/$stem: $(wc -c < "$out/$stem.kicad_pcb") bytes"
}

fetch EEVengers/ThunderScope eb0c0b580fe2d811053e07580117b63424177499 Hardware/Thunderscope_Rev5.3 Thunderscope_Rev5.3 \
  Hardware/LICENSE CERN-OHL-P-2.0 "EEVengers"
fetch jankae/LibreVNA c47781910221ba2b02636229920a7c8d48aeb8f8 Hardware/Kicad VNA \
  LICENSE GPL-3.0 "Jan Käberich"

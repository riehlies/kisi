#!/usr/bin/env python3
"""Write a power tree (kisi JSON, e.g. web/samples/vna_tree.json) as PI_REG /
PI_LOAD properties into a copy of a .kicad_pcb.

Usage: tree_to_fields.py tree.json board.kicad_pcb out.kicad_pcb

Meant for migrating an existing tree into KiCad and for testing; in a real
project put the fields on the schematic symbols (they are copied to the
footprints on "Update PCB from schematic").
"""
import json, re, sys


def rail(n, volts):
    v = volts.get(n)
    named = re.match(r"^[+-]?[0-9]+V[0-9]*$", n.rsplit("/", 1)[-1], re.I)
    return n if named or v is None else f"{n}@{v:g}V"


def fields(tree):
    volts = {r["net"]: r["voltage"] for r in tree.get("rails", [])}
    by = {}
    for e in tree.get("elements", []):
        p = [f"{rail(e['from'], volts)} > {rail(e['to'], volts)}" if e.get("from") else rail(e["to"], volts), e.get("kind", "buck")]
        if e.get("eff") is not None: p.append(f"{e['eff'] * 100:g}%")
        if e.get("imax") is not None: p.append(f"{e['imax']:g}A")
        if e.get("iq"): p.append(f"iq={e['iq'] * 1e3:g}mA")
        if e.get("r"): p.append(f"r={e['r']:g}")
        if e.get("in_ref"): p.append(f"in={e['in_ref']}")
        if e.get("out_ref"): p.append(f"out={e['out_ref']}")
        if e.get("id") and e["id"] != e["refdes"]: p.append("id=" + re.sub(r"[\s;]+", "_", e["id"]))
        by.setdefault(e["refdes"], {}).setdefault("PI_REG", []).append(" ".join(p))
    for l in tree.get("loads", []):
        if l.get("current"):
            by.setdefault(l["refdes"], {}).setdefault("PI_LOAD", []).append(f"{l['net']}={l['current']:g}A")
    return {ref: {"PI_REG": "; ".join(f.get("PI_REG", [])), "PI_LOAD": ", ".join(f.get("PI_LOAD", []))} for ref, f in by.items()}


def main():
    tree = json.load(open(sys.argv[1], encoding="utf-8"))
    pcb = open(sys.argv[2], encoding="utf-8").read()
    f = fields(tree)
    done = set()

    def ins(m):
        ref = m.group(1)
        extra = ""
        for k, v in f.get(ref, {}).items():
            if v:
                extra += f'(property "{k}" "{v}" (at 0 0 0) (layer "F.Fab") (hide yes)) '
        if extra:
            done.add(ref)
        return extra + m.group(0)

    pcb = re.sub(r'\(property "Reference" "([^"]+)"', ins, pcb)
    open(sys.argv[3], "w", encoding="utf-8").write(pcb)
    missing = sorted(set(f) - done)
    print(f"{len(done)} Bauteile mit Feldern" + (f", nicht gefunden: {' '.join(missing)}" if missing else ""))


if __name__ == "__main__":
    main()

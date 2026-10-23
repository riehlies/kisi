#!/usr/bin/env python3
"""Write a power tree (kisi tree JSON, as accepted by `kisi power --tree`) as PI_REG /
PI_LOAD fields: into a `<board>.kisi.json` or into a copy of the .kicad_pcb.

Usage: tree_to_fields.py tree.json board.kicad_pcb out.kisi.json   board configuration
       tree.json board.kicad_pcb out.kicad_pcb                       fields in a board copy

The board configuration keeps the sources: loads get the tree's basis (datasheet /
estimate) and notes, regulators the kind of their values. In a real project the
fields can also go on the schematic symbols (copied to the footprints on "Update PCB
from schematic"); the kisi.json keeps the KiCad files untouched.
"""
import json, os, re, sys


def name(n):
    """A net name in a field: in quotes when it has spaces or separators (hierarchical names)."""
    return f'"{n}"' if re.search(r'[\s";,=@>→]', n) else n


def rail(n, volts):
    v = volts.get(n)
    named = re.match(r"^[+-]?[0-9]+V[0-9]*$", n.rsplit("/", 1)[-1], re.I)
    return name(n) if named or v is None else f"{name(n)}@{v:g}V"


def reg_entry(e, volts):
    p = [f"{rail(e['from'], volts)} > {rail(e['to'], volts)}" if e.get("from") else rail(e["to"], volts), e.get("kind", "buck")]
    if e.get("eff") is not None: p.append(f"{e['eff'] * 100:g}%")
    if e.get("imax") is not None: p.append(f"{e['imax']:g}A")
    if e.get("iq"): p.append(f"iq={e['iq'] * 1e3:g}mA")
    if e.get("r"): p.append(f"r={e['r']:g}")
    if e.get("in_ref"): p.append(f"in={e['in_ref']}")
    if e.get("out_ref"): p.append(f"out={e['out_ref']}")
    return " ".join(p)


def reg_source(e):
    """Kind and note of a regulator entry's values."""
    k, part, note = e.get("kind"), e.get("part", ""), e.get("note", "")
    text = lambda *x: ", ".join(s for s in x if s)
    if k == "input":
        return "assumption", text(part, "supply input")
    if k == "pass":
        if "shunt" in part.lower():
            return "assumption", text(part, "resistance = component value")
        return "estimate", text(part, "DC resistance estimated")
    if k == "ldo":
        return "datasheet", text(part, note, "quiescent and rated current from the datasheet")
    return "estimate", text(part, note, "efficiency estimated from typical curves")


def entries(tree):
    """(ref, field, value, source) per component and field."""
    volts = {r["net"]: r["voltage"] for r in tree.get("rails", [])}
    regs, loads = {}, {}
    for e in tree.get("elements", []):
        regs.setdefault(e["refdes"], []).append((reg_entry(e, volts), reg_source(e)))
    for l in tree.get("loads", []):
        if not l.get("current"):
            continue
        # load point on other parts (e.g. the inductors of an output stage): NET=I@R1/R2
        at = "@" + "/".join(a.strip() for a in l["at"].split(",")) if l.get("at") else ""
        loads.setdefault(l["refdes"], []).append((f"{name(l['net'])}={l['current']:g}A{at}", l.get("basis") or "estimate", l["net"], l.get("note", "")))
    out = []
    for ref, rs in regs.items():
        kinds = {s[0] for _, s in rs}
        kind = "estimate" if "estimate" in kinds else ("assumption" if "assumption" in kinds else "datasheet")
        notes = list(dict.fromkeys(s[1] for _, s in rs if s[1]))
        out.append((ref, "PI_REG", "; ".join(v for v, _ in rs), {"kind": kind, "note": "; ".join(notes)}))
    for ref, ls in loads.items():
        kinds = {b for _, b, _, _ in ls}
        kind = "datasheet" if kinds == {"datasheet"} else "estimate"
        parts = []
        for _, b, net, note in ls:
            t = f"{net}: {note}" if len(ls) > 1 and note else note
            if len(kinds) > 1 and b != "datasheet":
                t = (t + " " if t else f"{net}: ") + "(estimate)"
            if t: parts.append(t)
        src = {"kind": kind, "note": "; ".join(parts) or ("typical value" if kind == "datasheet" else "rough value, check the datasheet")}
        out.append((ref, "PI_LOAD", ", ".join(v for v, _, _, _ in ls), src))
    return out


def write_config(tree, board, out):
    pcb = open(board, encoding="utf-8").read()
    refs = set(re.findall(r'\(property "Reference" "([^"]+)"', pcb))
    es = entries(tree)
    missing = sorted({r for r, *_ in es} - refs)
    notes = [n for n in (tree.get("name", ""), tree.get("note", "")) if n]
    notes.append("Converted from a kisi power tree with tools/tree_to_fields.py.")
    cfg = {"kisi_config": 1, "board": os.path.basename(board), "notes": notes,
           "fields": [{"ref": r, "name": f, "value": v, "source": s} for r, f, v, s in es]}
    with open(out, "w", encoding="utf-8", newline="\n") as fh:
        fh.write("{\n")
        fh.write(f'  "kisi_config": 1,\n  "board": {json.dumps(cfg["board"])},\n')
        fh.write('  "notes": ' + json.dumps(notes, ensure_ascii=False, indent=4).replace("\n]", "\n  ]") + ",\n")
        fh.write('  "fields": [\n' + ",\n".join("    " + json.dumps(x, ensure_ascii=False) for x in cfg["fields"]) + "\n  ]\n}\n")
    print(f"{out}: {len(es)} fields" + (f", not on the board: {' '.join(missing)}" if missing else ""))


def write_board(tree, board, out):
    pcb = open(board, encoding="utf-8").read()
    f = {}
    for r, k, v, _ in entries(tree):
        f.setdefault(r, {})[k] = v
    done = set()

    def ins(m):
        ref = m.group(1)
        q = lambda v: v.replace("\\", "\\\\").replace('"', '\\"')  # s-expression string
        extra = "".join(f'(property "{k}" "{q(v)}" (at 0 0 0) (layer "F.Fab") (hide yes)) ' for k, v in f.get(ref, {}).items())
        if extra:
            done.add(ref)
        return extra + m.group(0)

    pcb = re.sub(r'\(property "Reference" "([^"]+)"', ins, pcb)
    open(out, "w", encoding="utf-8").write(pcb)
    missing = sorted(set(f) - done)
    print(f"{len(done)} components with fields" + (f", not found: {' '.join(missing)}" if missing else ""))


def main():
    tree = json.load(open(sys.argv[1], encoding="utf-8"))
    (write_config if sys.argv[3].endswith(".json") else write_board)(tree, sys.argv[2], sys.argv[3])


if __name__ == "__main__":
    main()

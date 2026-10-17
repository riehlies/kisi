#!/usr/bin/env python3
"""Strip a .kicad_pcb down to what kisi needs (copper, stackup, nets, outline).

Used to embed sample boards into the viewer at a reasonable size.
Usage: strip_pcb.py in.kicad_pcb out.kicad_pcb
"""
import sys

KEEP_TOP = {"version", "generator", "generator_version", "general", "layers", "setup", "net",
            "footprint", "segment", "arc", "via", "zone",
            "gr_line", "gr_arc", "gr_rect", "gr_circle", "gr_poly"}
KEEP_FP = {"layer", "at", "pad", "attr", "property"}
KEEP_PAD = {"at", "size", "drill", "layers", "net", "roundrect_rratio", "primitives"}
DROP_ZONE = {"polygon", "fill", "connect_pads", "hatch", "placement", "attr", "keepout"}


def tokens(s):
    i, n = 0, len(s)
    while i < n:
        c = s[i]
        if c in " \t\r\n":
            i += 1
        elif c in "()":
            yield c
            i += 1
        elif c == '"':
            j = i + 1
            while j < n:
                if s[j] == "\\":
                    j += 2
                    continue
                if s[j] == '"':
                    break
                j += 1
            yield s[i:j + 1]
            i = j + 1
        else:
            j = i
            while j < n and s[j] not in ' \t\r\n()"':
                j += 1
            yield s[i:j]
            i = j


def parse(s):
    stack = [[]]
    for t in tokens(s):
        if t == "(":
            stack.append([])
        elif t == ")":
            x = stack.pop()
            stack[-1].append(x)
        else:
            stack[-1].append(t)
    return stack[0][0]


def head(x):
    return x[0] if isinstance(x, list) and x and isinstance(x[0], str) else None


def emit(x, out):
    if isinstance(x, str):
        # round long decimals to 0.1 um to shrink the file
        if "." in x and len(x) > 8 and x[0] in "-0123456789":
            try:
                x = ("%.4f" % float(x)).rstrip("0").rstrip(".")
            except ValueError:
                pass
        out.append(x)
        return
    out.append("(")
    first = True
    for c in x:
        if not first:
            out.append(" ")
        emit(c, out)
        first = False
    out.append(")")


def strip(root):
    res = [root[0]]
    for it in root[1:]:
        h = head(it)
        if h not in KEEP_TOP:
            continue
        if h and h.startswith("gr_"):
            if not any(head(c) == "layer" and c[1] == '"Edge.Cuts"' for c in it[1:]):
                continue
        if h == "footprint":
            fp = [it[0], it[1]]
            for c in it[2:]:
                if head(c) in KEEP_FP:
                    if head(c) == "property":
                        fp.append(c[:3])
                    elif head(c) == "pad":
                        pad = [x for x in c if not isinstance(x, list) or head(x) in KEEP_PAD]
                        fp.append(pad)
                    else:
                        fp.append(c)
            it = fp
        if h == "zone":
            if not any(head(c) == "filled_polygon" for c in it[1:]):
                continue
            it = [c for c in it if not (isinstance(c, list) and head(c) in DROP_ZONE)]
        res.append(it)
    return res


def main():
    src = open(sys.argv[1], encoding="utf-8").read()
    root = parse(src)
    out = []
    emit(strip(root), out)
    open(sys.argv[2], "w", encoding="utf-8").write("".join(out))


if __name__ == "__main__":
    main()

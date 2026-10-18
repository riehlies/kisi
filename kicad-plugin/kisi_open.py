#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""KiCad IPC-API action: open the kisi viewer with the board that is open in
the PCB editor.

* the board is copied to a temporary folder with KiCad's "save a copy"
  (the user's files are neither saved nor changed); older KiCad versions fall
  back to the saved file on disk
* the copy (and the project file for the net classes) is embedded into a copy
  of kisi.html next to this script, which then opens in the default browser
* a selected track, via or pad pre-selects its net in the viewer
* IBIS files named in the footprints' simulation fields (Sim.Device = IBIS,
  Sim.Library) are embedded too, so the eye uses the device models

The viewer itself stays a standalone page; this script only hands it a board.
"""
import argparse
import base64
import gzip
import json
import os
import pathlib
import re
import sys
import tempfile
import webbrowser

HERE = os.path.dirname(os.path.abspath(__file__))
MARKER = "null; /*KISI_PRELOAD*/"


def b64gz(path):
    with open(path, "rb") as f:
        return base64.b64encode(gzip.compress(f.read(), 6)).decode("ascii")


def find_viewer():
    for p in (os.path.join(HERE, "kisi.html"), os.path.join(HERE, "..", "dist", "kisi.html")):
        if os.path.exists(p):
            return p
    raise FileNotFoundError("kisi.html not found (place it next to kisi_open.py)")


def board_files(board, tmp):
    """(pcb path, pro path or None, warning or None)"""
    name = board.name or "board.kicad_pcb"
    target = os.path.join(tmp, name)
    try:
        board.save_as(target, overwrite=True, include_project=True)
        pro = os.path.splitext(target)[0] + ".kicad_pro"
        return target, (pro if os.path.exists(pro) else None), None
    except Exception as e:  # KiCad without SaveCopyOfDocument: use the saved file
        proj = board.get_project()
        pcb = os.path.join(proj.path, name)
        pro = os.path.join(proj.path, proj.name + ".kicad_pro")
        return pcb, (pro if os.path.exists(pro) else None), f"could not save a copy ({e}); using the saved file – unsaved changes are missing."


IBIS_MAX = 64 * 1024 * 1024


def ibis_files(pcb, project_dir):
    """[{name, data}] of the IBIS files referenced by footprints (Sim.Library of
    Sim.Device = IBIS), resolved like KiCad: ${KIPRJMOD} and other variables,
    relative to the project; else the file name searched in the project folder."""
    text = open(pcb, encoding="utf-8", errors="replace").read()
    libs = []
    for fp in re.finditer(r'\(footprint "(?:[^"\\]|\\.)*"', text):
        end = text.find("(footprint ", fp.end())
        blk = text[fp.start(): end if end > 0 else len(text)]
        dev = re.search(r'\(property "Sim\.Device" "([^"]*)"', blk)
        lib = re.search(r'\(property "Sim\.Library" "([^"]*)"', blk)
        if dev and lib and dev.group(1).strip().lower() == "ibis" and lib.group(1).strip() and lib.group(1) not in libs:
            libs.append(lib.group(1).strip())
    out, warn = [], []
    for lib in libs:
        p = re.sub(r"\$\{(\w+)\}|\$\((\w+)\)", lambda m: project_dir if (m.group(1) or m.group(2)) == "KIPRJMOD" else os.environ.get(m.group(1) or m.group(2), ""), lib)
        p = p if os.path.isabs(p) else os.path.join(project_dir, p)
        if not os.path.isfile(p):
            base = os.path.basename(p.replace("\\", "/")).lower()
            p = next((os.path.join(r, f) for r, _, fs in os.walk(project_dir) for f in fs if f.lower() == base), None)
        if not p:
            warn.append(f"IBIS file {lib} not found")
        elif os.path.getsize(p) > IBIS_MAX:
            warn.append(f"IBIS file {lib} is larger than 64 MB, not embedded")
        else:
            out.append({"name": lib, "data": b64gz(p)})
    return out, warn


def project_dir_of(board, pcb):
    try:
        return board.get_project().path
    except Exception:
        return os.path.dirname(os.path.abspath(pcb))


def selected_net(board):
    try:
        for item in board.get_selection():
            net = getattr(item, "net", None)
            if net is not None and net.name:
                return net.name
    except Exception:
        pass
    return None


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--view", default="si", choices=["si", "ch", "pi"])
    ap.add_argument("--pcb", help="without KiCad: open this board file directly")
    ap.add_argument("--net", help="without KiCad: select this net")
    args, _ = ap.parse_known_args()

    tmp = tempfile.mkdtemp(prefix="kisi-")
    warn, net = None, None
    proj_dir = None
    if args.pcb:
        pcb = args.pcb
        pro = os.path.splitext(pcb)[0] + ".kicad_pro"
        pro = pro if os.path.exists(pro) else None
        name = os.path.basename(pcb)
        net = args.net
        proj_dir = os.path.dirname(os.path.abspath(pcb))
    else:
        from kipy import KiCad  # provided by kicad-python (requirements.txt)
        kicad = KiCad(client_name="kisi")
        board = kicad.get_board()
        pcb, pro, warn = board_files(board, tmp)
        net = selected_net(board)
        name = board.name
        proj_dir = project_dir_of(board, pcb)
    ibis, ibis_warn = ibis_files(pcb, proj_dir)

    preload = {"name": os.path.splitext(name)[0], "pcb": b64gz(pcb), "pro": b64gz(pro) if pro else None, "net": net, "view": args.view, "ibis": ibis}
    html = open(find_viewer(), encoding="utf-8").read()
    if MARKER not in html:
        raise RuntimeError("kisi.html has no hand-over marker (version too old?)")
    html = html.replace(MARKER, json.dumps(preload).replace("</", "<\\/") + ";", 1)
    out = os.path.join(tmp, f"kisi_{preload['name']}.html")
    with open(out, "w", encoding="utf-8") as f:
        f.write(html)
    for w in ([warn] if warn else []) + ibis_warn:
        print("kisi:", w, file=sys.stderr)
    if not os.environ.get("KISI_NO_BROWSER"):
        webbrowser.open(pathlib.Path(out).as_uri())
    print(out)


if __name__ == "__main__":
    main()

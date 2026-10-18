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

The viewer itself stays a standalone page; this script only hands it a board.
"""
import argparse
import base64
import gzip
import json
import os
import pathlib
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
    raise FileNotFoundError("kisi.html nicht gefunden (neben kisi_open.py ablegen)")


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
        return pcb, (pro if os.path.exists(pro) else None), f"Kopie nicht möglich ({e}); verwende die gespeicherte Datei – ungespeicherte Änderungen fehlen."


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
    ap.add_argument("--pcb", help="ohne KiCad: Board-Datei direkt öffnen")
    ap.add_argument("--net", help="ohne KiCad: dieses Netz auswählen")
    args, _ = ap.parse_known_args()

    tmp = tempfile.mkdtemp(prefix="kisi-")
    warn, net = None, None
    if args.pcb:
        pcb = args.pcb
        pro = os.path.splitext(pcb)[0] + ".kicad_pro"
        pro = pro if os.path.exists(pro) else None
        name = os.path.basename(pcb)
        net = args.net
    else:
        from kipy import KiCad  # provided by kicad-python (requirements.txt)
        kicad = KiCad(client_name="kisi")
        board = kicad.get_board()
        pcb, pro, warn = board_files(board, tmp)
        net = selected_net(board)
        name = board.name

    preload = {"name": os.path.splitext(name)[0], "pcb": b64gz(pcb), "pro": b64gz(pro) if pro else None, "net": net, "view": args.view}
    html = open(find_viewer(), encoding="utf-8").read()
    if MARKER not in html:
        raise RuntimeError("kisi.html ohne Übergabe-Marker (zu alte Version?)")
    html = html.replace(MARKER, json.dumps(preload).replace("</", "<\\/") + ";", 1)
    out = os.path.join(tmp, f"kisi_{preload['name']}.html")
    with open(out, "w", encoding="utf-8") as f:
        f.write(html)
    if warn:
        print("kisi:", warn, file=sys.stderr)
    if not os.environ.get("KISI_NO_BROWSER"):
        webbrowser.open(pathlib.Path(out).as_uri())
    print(out)


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""Draws kisi's plugin icons (standard library only; the PNGs are committed, run this after a change):

  kisi-24.png, kisi-48.png            toolbar button, light theme (plugin.json icons-light; 48 px for high DPI)
  kisi-24-dark.png, kisi-48-dark.png  toolbar button, dark theme
  ../resources/icon.png               64×64 package icon of KiCad's Plugin and Content Manager

The motif is an eye diagram (two bit transitions over a ground bar): signal integrity, without any logo.
"""
import math
import os
import struct
import zlib

HERE = os.path.dirname(os.path.abspath(__file__))
SS = 4  # supersampling per axis


def png(path, w, h, rgba):
    raw = b"".join(b"\x00" + bytes(rgba[y * w * 4:(y + 1) * w * 4]) for y in range(h))
    chunk = lambda t, d: struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d) & 0xFFFFFFFF)
    data = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b"")
    with open(path, "wb") as f:
        f.write(data)


def eye_paths(x0, x1, yc, amp):
    """Polylines of a two-UI eye: transitions centred at 1/4 and 3/4 of the width, flat rails between."""
    paths = []
    ui = (x1 - x0) / 2
    for c in (x0 + ui / 2, x0 + 1.5 * ui):
        for sgn in (1, -1):
            pts = []
            for i in range(41):
                x = c - ui / 2 + ui * i / 40
                pts.append((x, yc - sgn * amp * math.tanh(4.2 * (x - c) / ui)))
            paths.append(pts)
    for sgn in (1, -1):  # rails over the eye's middle and at the ends
        paths.append([(x0 + ui / 2 + ui * 0.42, yc - sgn * amp * 0.995), (x0 + 1.5 * ui - ui * 0.42, yc - sgn * amp * 0.995)])
    return paths


def seg_dist(px, py, a, b):
    (ax, ay), (bx, by) = a, b
    dx, dy = bx - ax, by - ay
    t = 0.0 if dx == dy == 0 else max(0.0, min(1.0, ((px - ax) * dx + (py - ay) * dy) / (dx * dx + dy * dy)))
    return math.hypot(px - (ax + t * dx), py - (ay + t * dy))


def render(n, layers, bg=None, radius=0.0):
    """layers: [(polylines, width, (r,g,b))] in pixel units of an n×n icon; bg: rounded square colour."""
    out = [0] * (n * n * 4)
    segs = [([(p[i], p[i + 1]) for p in polys for i in range(len(p) - 1)], wid, col) for polys, wid, col in layers]
    for y in range(n):
        for x in range(n):
            acc = [0.0, 0.0, 0.0, 0.0]
            for sy in range(SS):
                for sx in range(SS):
                    px, py = x + (sx + 0.5) / SS, y + (sy + 0.5) / SS
                    col, a = None, 0.0
                    if bg is not None:
                        # rounded square
                        qx, qy = min(px, n - px), min(py, n - py)
                        inside = not (qx < radius and qy < radius and math.hypot(radius - qx, radius - qy) > radius)
                        if inside:
                            col, a = bg, 1.0
                    for ss, wid, c in segs:
                        if any(seg_dist(px, py, s[0], s[1]) <= wid / 2 for s in ss):
                            col, a = c, 1.0
                    if col is not None:
                        for k in range(3):
                            acc[k] += col[k] * a
                        acc[3] += a
            if acc[3]:
                i = (y * n + x) * 4
                out[i:i + 4] = [round(acc[0] / acc[3]), round(acc[1] / acc[3]), round(acc[2] / acc[3]), round(255 * acc[3] / SS / SS)]
    return out


def toolbar(n, eye_col, bar_col):
    s = n / 24
    eye = eye_paths(1.6 * s, 22.4 * s, 10.0 * s, 6.2 * s)
    bar = [[(3.0 * s, 20.6 * s), (21.0 * s, 20.6 * s)]]
    return render(n, [(eye, 1.9 * s, eye_col), (bar, 1.9 * s, bar_col)])


def main():
    # KiCad's toolbar glyphs: dark grey with one accent colour (light theme), light grey (dark theme)
    for suffix, eye, bar in (("", (21, 101, 192), (84, 84, 84)), ("-dark", (100, 181, 246), (200, 200, 200))):
        for n in (24, 48):
            png(os.path.join(HERE, f"kisi-{n}{suffix}.png"), n, n, toolbar(n, eye, bar))
    n = 64
    eye = eye_paths(9, 55, 28, 13.5)
    bar = [[(13, 50.5), (51, 50.5)]]
    icon = render(n, [(eye, 4.2, (255, 255, 255)), (bar, 4.2, (144, 202, 249))], bg=(21, 74, 140), radius=11)
    os.makedirs(os.path.join(HERE, "..", "resources"), exist_ok=True)
    png(os.path.join(HERE, "..", "resources", "icon.png"), n, n, icon)


if __name__ == "__main__":
    main()

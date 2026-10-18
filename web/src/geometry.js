"use strict";
// ============================================================== geometry prep
function classTarget(net) {
  const ci = S.scene.net_class[net] ?? 0;
  const c = S.an.classes[ci];
  return c ? c.target : null;
}
function comparedZ(xs, t) {
  if (t && t.diff) {
    if (xs.zdiff != null) return { v: xs.zdiff, diff: true };
    if (xs.z0 != null) return { v: 2 * xs.z0, diff: false };
    return null;
  }
  return xs.z0 != null ? { v: xs.z0, diff: false } : null;
}
function preparePieces() {
  const tr = S.scene.tracks, P = S.an.pieces, n = P.length / 4;
  const pc = { n, ax: new Float32Array(n), ay: new Float32Array(n), bx: new Float32Array(n), by: new Float32Array(n),
    w: new Float32Array(n), layer: new Uint8Array(n), net: new Uint32Array(n), xs: new Uint32Array(n), track: new Uint32Array(n),
    t0: new Float32Array(n), t1: new Float32Array(n) };
  for (let i = 0; i < n; i++) {
    const t = P[i * 4], a = P[i * 4 + 1], b = P[i * 4 + 2], o = t * 7;
    const x1 = tr[o], y1 = tr[o + 1], x2 = tr[o + 2], y2 = tr[o + 3];
    pc.ax[i] = x1 + (x2 - x1) * a; pc.ay[i] = y1 + (y2 - y1) * a;
    pc.bx[i] = x1 + (x2 - x1) * b; pc.by[i] = y1 + (y2 - y1) * b;
    pc.w[i] = tr[o + 4]; pc.layer[i] = tr[o + 5]; pc.net[i] = tr[o + 6];
    pc.xs[i] = P[i * 4 + 3]; pc.track[i] = t; pc.t0[i] = a; pc.t1[i] = b;
  }
  S.pieces = pc;
  // spatial hash, 1 mm cells
  const grid = new Map();
  for (let i = 0; i < n; i++) {
    const x0 = Math.floor(Math.min(pc.ax[i], pc.bx[i]) - pc.w[i]), x1 = Math.floor(Math.max(pc.ax[i], pc.bx[i]) + pc.w[i]);
    const y0 = Math.floor(Math.min(pc.ay[i], pc.by[i]) - pc.w[i]), y1 = Math.floor(Math.max(pc.ay[i], pc.by[i]) + pc.w[i]);
    for (let cx = x0; cx <= x1; cx++) for (let cy = y0; cy <= y1; cy++) {
      const k = cx * 100003 + cy; let a = grid.get(k); if (!a) grid.set(k, a = []); a.push(i);
    }
  }
  S.grid = grid;
}

function pieceColor(i) {
  const xs = S.an.cross_sections[S.pieces.xs[i]], C = S.colors;
  // before step 2: line type from the layer position only
  if (S.an.skeleton && S.tab !== "ch") return xs.kind === "Stripline" ? C.sl : C.ms;
  if (S.tab === "ch") {
    // channel view: result of the channel check per net, grey if not checked
    const st = S.chStatus && S.chStatus.get(S.pieces.net[i]);
    return st ? C[st] : C.none;
  }
  if (S.mode === "type") {
    return { Microstrip: C.ms, Stripline: C.sl, EmbeddedMicrostrip: C.emb, NoReference: C.noref }[xs.kind];
  }
  if (xs.kind === "NoReference") return C.noref;
  if (S.mode === "abs") {
    const z = xs.z0;
    return z == null ? C.noref : turbo((z - ABS_MIN) / (ABS_MAX - ABS_MIN) * 0.9 + 0.05);
  }
  const t = classTarget(S.pieces.net[i]);
  if (!t) return C.none;
  const z = comparedZ(xs, t);
  if (!z) return C.noref;
  const d = (z.v - t.z) / t.z;
  if (Math.abs(d) <= 0.05) return C.ok;
  if (d < 0) return d < -0.10 ? C.low2 : C.low1;
  return d > 0.10 ? C.high2 : C.high1;
}

function buildPaths(onlyNet = -1) {
  // group by layer -> (color|width) -> Path2D
  const n = S.scene.stackup.copper.length, pc = S.pieces;
  const layers = Array.from({ length: n }, () => new Map());
  for (let i = 0; i < pc.n; i++) {
    if (onlyNet >= 0 && pc.net[i] !== onlyNet) continue;
    const col = pieceColor(i);
    const key = col + "|" + pc.w[i].toFixed(3);
    let g = layers[pc.layer[i]].get(key);
    if (!g) layers[pc.layer[i]].set(key, g = { col, w: pc.w[i], path: new Path2D(), neutral: col === S.colors.none });
    g.path.moveTo(pc.ax[i], pc.ay[i]);
    g.path.lineTo(pc.bx[i], pc.by[i]);
  }
  const out = layers.map((m) => [...m.values()].sort((a, b) => (b.neutral - a.neutral)));
  if (onlyNet >= 0) S.selPaths = out; else S.paths = out;
}

function buildStatic() {
  // zones per layer, pads, vias, outline as Path2D in board coordinates
  const n = S.scene.stackup.copper.length;
  const zp = Array.from({ length: n }, () => new Path2D());
  const { idx, pts } = S.zones;
  for (let i = 0; i < idx.length; i += 4) {
    const l = idx[i], st = idx[i + 2], c = idx[i + 3];
    const p = zp[l];
    p.moveTo(pts[st * 2], pts[st * 2 + 1]);
    for (let k = 1; k < c; k++) p.lineTo(pts[(st + k) * 2], pts[(st + k) * 2 + 1]);
    p.closePath();
  }
  const outline = new Path2D();
  for (const poly of S.scene.outline) {
    if (!poly.length) continue;
    outline.moveTo(poly[0][0], poly[0][1]);
    for (let k = 1; k < poly.length; k++) outline.lineTo(poly[k][0], poly[k][1]);
  }
  const pads = S.scene.pads, padsByLayer = Array.from({ length: n }, () => new Path2D());
  for (let i = 0; i < pads.length; i += 8) {
    const x = pads[i], y = pads[i + 1], sx = pads[i + 2], sy = pads[i + 3], ang = -pads[i + 4] * Math.PI / 180, shape = pads[i + 5], mask = pads[i + 6];
    const layer = (mask & 1) ? 0 : ((mask & (1 << (n - 1))) ? n - 1 : Math.log2(mask & -mask));
    const p = padsByLayer[layer] || padsByLayer[0];
    const m = new DOMMatrix().translate(x, y).rotate(ang * 180 / Math.PI);
    const r = new Path2D();
    if (shape === 1) r.arc(0, 0, sx / 2, 0, Math.PI * 2);
    else if (shape === 2 || shape === 3) r.roundRect(-sx / 2, -sy / 2, sx, sy, shape === 2 ? Math.min(sx, sy) / 2 : Math.min(sx, sy) * 0.25);
    else r.rect(-sx / 2, -sy / 2, sx, sy);
    p.addPath(r, m);
  }
  const vias = new Path2D(), v = S.scene.vias;
  for (let i = 0; i < v.length; i += 7) { vias.moveTo(v[i] + v[i + 2] / 2, v[i + 1]); vias.arc(v[i], v[i + 1], v[i + 2] / 2, 0, Math.PI * 2); }
  S.static = { zp, outline, pads: padsByLayer, vias };
}

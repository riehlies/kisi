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
    // channel view: result of the channel check per net; before the check the nets of the
    // channels to check; nets without a function in this assembly variant faint
    const net = S.pieces.net[i];
    const st = S.chStatus && S.chStatus.get(net);
    if (st) return C[st];
    if (netUnused(net)) return C.unused;
    return chActiveNets().has(net) ? C.lane : C.none;
  }
  // crosstalk: neutral board, the critical stretches are drawn over it (drawXtMap)
  if (S.mode === "xt") return C.none;
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

// outline of pad `i` as a Path2D in board coordinates (scene.pads: x, y, sx, sy, angle, shape,
// layer mask, net; corner radius from scene.pad_radius, custom pads from scene.pad_custom)
function padShape(i) {
  const pads = S.scene.pads, o = i * 8;
  const x = pads[o], y = pads[o + 1], sx = pads[o + 2], sy = pads[o + 3], shape = pads[o + 5];
  const m = new DOMMatrix().translate(x, y).rotate(-pads[o + 4]);
  const r = new Path2D();
  const custom = shape === 4 && S.padCustom ? S.padCustom.get(i) : null;
  if (custom) {
    for (const poly of custom) {
      if (!poly.length) continue;
      r.moveTo(poly[0][0], poly[0][1]);
      for (let k = 1; k < poly.length; k++) r.lineTo(poly[k][0], poly[k][1]);
      r.closePath();
    }
  } else if (shape === 1) r.arc(0, 0, sx / 2, 0, Math.PI * 2);
  else {
    const rad = S.scene.pad_radius ? S.scene.pad_radius[i] : (shape === 2 ? Math.min(sx, sy) / 2 : shape === 3 ? Math.min(sx, sy) * 0.25 : 0);
    if (rad > 0) r.roundRect(-sx / 2, -sy / 2, sx, sy, Math.min(rad, sx / 2, sy / 2)); else r.rect(-sx / 2, -sy / 2, sx, sy);
  }
  const p = new Path2D();
  p.addPath(r, m);
  return p;
}
// the copper of piece `i` of a synthetic pad track (two halves from the pad centre): its part of
// the pad as a rectangle (null for ordinary tracks); `whole`: the pad's own outline instead
function padPieceShape(i, whole = false) {
  const pc = S.pieces, pad = S.padOfTrack ? S.padOfTrack.get(pc.track[i]) : undefined;
  if (pad == null) return null;
  if (whole) return padShape(pad);
  const dx = pc.bx[i] - pc.ax[i], dy = pc.by[i] - pc.ay[i], l = Math.hypot(dx, dy) || 1, h = pc.w[i] / 2;
  const nx = -dy / l * h, ny = dx / l * h, p = new Path2D();
  p.moveTo(pc.ax[i] + nx, pc.ay[i] + ny); p.lineTo(pc.bx[i] + nx, pc.by[i] + ny); p.lineTo(pc.bx[i] - nx, pc.by[i] - ny); p.lineTo(pc.ax[i] - nx, pc.ay[i] - ny); p.closePath();
  return p;
}

// tracks grouped by layer -> (color|width) -> Path2D (stroked); pads of signal nets in `fill`
// (filled: their own outline); `only`: a set of nets (null: all)
function netPaths(only) {
  const n = S.scene.stackup.copper.length, pc = S.pieces;
  const layers = Array.from({ length: n }, () => new Map());
  // pads whose pieces all have one colour: drawn once with their own outline
  const padCol = new Map();
  for (let i = 0; i < pc.n; i++) {
    const pad = S.padOfTrack ? S.padOfTrack.get(pc.track[i]) : undefined;
    if (pad == null || (only && !only.has(pc.net[i]))) continue;
    const col = pieceColor(i), c = padCol.get(pad);
    padCol.set(pad, c === undefined || c === col ? col : null);
  }
  const drawn = new Set();
  for (let i = 0; i < pc.n; i++) {
    if (only && !only.has(pc.net[i])) continue;
    const col = pieceColor(i);
    const key = col + "|" + pc.w[i].toFixed(3);
    let g = layers[pc.layer[i]].get(key);
    if (!g) layers[pc.layer[i]].set(key, g = { col, w: pc.w[i], path: new Path2D(), fill: null, neutral: col === S.colors.none });
    const pad = S.padOfTrack ? S.padOfTrack.get(pc.track[i]) : undefined;
    if (pad != null) {
      const whole = padCol.get(pad) != null;
      if (whole && drawn.has(pad)) continue;
      drawn.add(pad);
      (g.fill || (g.fill = new Path2D())).addPath(padPieceShape(i, whole));
      continue;
    }
    g.path.moveTo(pc.ax[i], pc.ay[i]);
    g.path.lineTo(pc.bx[i], pc.by[i]);
  }
  return layers.map((m) => [...m.values()].sort((a, b) => (b.neutral - a.neutral)));
}
// all tracks (S.paths), or the selection (S.selPaths: one net, or a set of nets such as a lane)
function buildPaths(onlyNet = -1) {
  const only = onlyNet instanceof Set ? onlyNet : onlyNet >= 0 ? new Set([onlyNet]) : null;
  if (only) S.selPaths = netPaths(only); else S.paths = netPaths(null);
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
  for (let i = 0; i < pads.length / 8; i++) {
    const mask = pads[i * 8 + 6];
    const layer = (mask & 1) ? 0 : ((mask & (1 << (n - 1))) ? n - 1 : Math.log2(mask & -mask));
    (padsByLayer[layer] || padsByLayer[0]).addPath(padShape(i));
  }
  const vias = new Path2D(), v = S.scene.vias;
  for (let i = 0; i < v.length; i += 7) { vias.moveTo(v[i] + v[i + 2] / 2, v[i + 1]); vias.arc(v[i], v[i + 1], v[i + 2] / 2, 0, Math.PI * 2); }
  S.static = { zp, outline, pads: padsByLayer, vias };
}

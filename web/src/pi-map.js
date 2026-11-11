"use strict";
// ---- raster maps of the Power view (voltage drop, current density, losses, temperature): display only.
// The solver's cells stay as they are (the tooltips read them). The picture is made from them:
// * the values are extended MAP_DILATE cells into the empty cells around the copper, so that
//   interpolation and smoothing do not fade at the copper's edge,
// * interpolated bilinearly ×f (f by the cell count, MAP_PX) and then colour-mapped – the colour of
//   the interpolated value, not a blend of two colours –, the canvas smooths further when zoomed in,
// * clipped to the real copper of the net on the layer (zones, pads, vias, tracks as one path) or to the
//   board outline, so the edges follow the copper and not the cells,
// * tracks narrower than MAP_LINE_CELLS cells (the solver's line conductors) as strokes of their own
//   width, coloured along their length from the cells under their centre line,
// * optional isolines (marching squares on the extended values), clipped the same way.

const MAP_DILATE = 2;       // cells the values are extended beyond the copper
const MAP_PX = 3e6;         // pixel budget of one map image (4 bytes each): chooses the factor f
const MAP_LINE_CELLS = 3;   // tracks narrower than this many cells: strokes (kisi_pi::pi::LINE_CELLS)
const MAP_ALPHA = 235 / 255;
const MAP_BUCKETS = 128;    // colour steps of the track strokes
let mapLut = null;          // turbo colours of the scale 0 … 1 (1024 steps)
function mapLutGet() {
  if (mapLut) return mapLut;
  const n = 1024; mapLut = new Uint8Array(n * 3);
  for (let i = 0; i < n; i++) mapLut.set(turboRGB(0.05 + 0.9 * i / (n - 1)), i * 3);
  return mapLut;
}
// upsampling factor for a map of `cells` cells: 4, 2 or 1 within the pixel budget (S.mapUp: forced, for tests)
const mapUpFactor = (cells) => S.mapUp || [4, 2, 1].find((f) => cells * f * f <= MAP_PX) || 1;

// A colour map from the values `val` (nx × ny, NaN: no copper) of a raster at x0, y0 with square cells:
// colour position (v - lo) / (hi - lo). o.dilate: cells of extension (default MAP_DILATE); o.cover: a
// pixel is drawn where the bilinear weight of the corners with values reaches it (default: any; 0.5
// without extension gives a smooth edge half way between a cell with and one without a value); o.lut: a
// colour table of 1024 RGB steps instead of turbo (mapLutGet); o.f: the factor instead of the budget's.
// Returns the image (cropped to the cells with values), the extended values of the crop (for isolines)
// and which of them were cells of their own (`own`).
function mapImage(val, nx, ny, o) {
  const t0 = performance.now();
  // crop: the cells with values plus the extension
  let i0 = nx, i1 = -1, j0 = ny, j1 = -1;
  for (let j = 0; j < ny; j++) {
    const row = j * nx;
    for (let i = 0; i < nx; i++) if (val[row + i] === val[row + i]) { if (i < i0) i0 = i; if (i > i1) i1 = i; if (j < j0) j0 = j; j1 = j; }
  }
  if (i1 < 0) return null;
  const dil = o.dilate ?? MAP_DILATE, cover = o.cover ?? 1e-6, e = dil + 1;
  i0 = Math.max(0, i0 - e); j0 = Math.max(0, j0 - e); i1 = Math.min(nx - 1, i1 + e); j1 = Math.min(ny - 1, j1 + e);
  const w = i1 - i0 + 1, h = j1 - j0 + 1;
  let a = new Float32Array(w * h);
  const own = new Uint8Array(w * h);
  for (let j = 0; j < h; j++) for (let i = 0; i < w; i++) { const v = val[(j + j0) * nx + i + i0]; a[j * w + i] = v; own[j * w + i] = v === v ? 1 : 0; }
  // extension: an empty cell next to cells with values takes their mean (8 neighbours), `dil` times
  for (let pass = 0; pass < dil; pass++) {
    const b = a.slice();
    for (let j = 0; j < h; j++) for (let i = 0; i < w; i++) {
      const k = j * w + i;
      if (a[k] === a[k]) continue;
      let s = 0, n = 0;
      for (let dj = -1; dj <= 1; dj++) {
        const jj = j + dj; if (jj < 0 || jj >= h) continue;
        for (let di = -1; di <= 1; di++) {
          const ii = i + di; if (ii < 0 || ii >= w) continue;
          const v = a[jj * w + ii]; if (v === v) { s += v; n++; }
        }
      }
      if (n) b[k] = s / n;
    }
    a = b;
  }
  const t1 = performance.now();
  // bilinear ×f on the values, then the colour; corners without a value drop out of the weights
  const f = o.f || mapUpFactor(w * h), W = w * f, H = h * f, lut = o.lut || mapLutGet();
  const c = document.createElement("canvas"); c.width = W; c.height = H;
  const cx = c.getContext("2d"), img = cx.createImageData(W, H), d = img.data;
  const lo = o.lo, sc = 1023 / ((o.hi - lo) || 1);
  const ix = new Int32Array(W), tx = new Float32Array(W);
  for (let p = 0; p < W; p++) { const u = Math.min(w - 1, Math.max(0, (p + 0.5) / f - 0.5)); ix[p] = Math.min(w - 2, Math.floor(u)); tx[p] = u - ix[p]; }
  if (w < 2) { ix.fill(0); tx.fill(0); }
  for (let q = 0; q < H; q++) {
    const vq = Math.min(h - 1, Math.max(0, (q + 0.5) / f - 0.5)), jq = h < 2 ? 0 : Math.min(h - 2, Math.floor(vq)), ty = h < 2 ? 0 : vq - jq;
    const r0 = jq * w, r1 = h < 2 ? r0 : r0 + w;
    for (let p = 0; p < W; p++) {
      const i = ix[p], fx = tx[p], i2 = w < 2 ? i : i + 1;
      const v00 = a[r0 + i], v10 = a[r0 + i2], v01 = a[r1 + i], v11 = a[r1 + i2];
      let v;
      if (v00 === v00 && v10 === v10 && v01 === v01 && v11 === v11) v = (v00 * (1 - fx) + v10 * fx) * (1 - ty) + (v01 * (1 - fx) + v11 * fx) * ty;
      else {
        let s = 0, n = 0, wt;
        if (v00 === v00) { wt = (1 - fx) * (1 - ty); s += v00 * wt; n += wt; }
        if (v10 === v10) { wt = fx * (1 - ty); s += v10 * wt; n += wt; }
        if (v01 === v01) { wt = (1 - fx) * ty; s += v01 * wt; n += wt; }
        if (v11 === v11) { wt = fx * ty; s += v11 * wt; n += wt; }
        if (n < cover) continue; // transparent
        v = s / n;
      }
      let li = Math.round((v - lo) * sc); li = li < 0 ? 0 : li > 1023 ? 1023 : li;
      const po = (q * W + p) * 4, lo3 = li * 3;
      d[po] = lut[lo3]; d[po + 1] = lut[lo3 + 1]; d[po + 2] = lut[lo3 + 2]; d[po + 3] = 255;
    }
  }
  cx.putImageData(img, 0, 0);
  const cell = o.cell;
  return {
    canvas: c, f, x0: o.x0 + i0 * cell, y0: o.y0 + j0 * cell, cell, w, h, vals: a, own, lo: o.lo, hi: o.hi, lut: o.lut || null,
    bbox: [o.x0 + i0 * cell, o.y0 + j0 * cell, o.x0 + (i1 + 1) * cell, o.y0 + (j1 + 1) * cell],
    bytes: W * H * 4 + a.byteLength + own.byteLength, ms: { extend: t1 - t0, image: performance.now() - t1 },
  };
}
// the image in board coordinates: near 1:1 without smoothing (the values are interpolated already, and
// smoothing costs several times more in a software-rendered canvas), bilinear when magnified, the
// browser's minification filter when much smaller
function mapPaint(c, m) {
  if (!m) return;
  const s = c.getTransform().a * m.cell / m.f; // device pixels per image pixel
  c.imageSmoothingEnabled = s > 1.5 || s < 0.5;
  c.imageSmoothingQuality = s < 0.5 ? "medium" : "low";
  c.drawImage(m.canvas, m.x0, m.y0, m.w * m.cell, m.h * m.cell);
  c.imageSmoothingEnabled = true;
}

// A closed ring of points (flat x, y list) added to `p` turning the same way as the canvas's own rect,
// arc and roundRect (positive shoelace sum in board coordinates, y down): copper drawn that way joins
// under the non-zero rule, so one path of all copper is the union of its parts (a clip or a mask in one
// fill). KiCad saves zone fills fractured – the holes joined to the outline –, a fill is one ring.
function mapRing(p, xy) {
  const n = xy.length / 2;
  if (n < 3) return;
  let a = 0;
  for (let k = 0; k < n; k++) { const j = (k + 1) % n; a += xy[2 * k] * xy[2 * j + 1] - xy[2 * j] * xy[2 * k + 1]; }
  const at = (k) => (a >= 0 ? k : n - 1 - k);
  p.moveTo(xy[2 * at(0)], xy[2 * at(0) + 1]);
  for (let k = 1; k < n; k++) p.lineTo(xy[2 * at(k)], xy[2 * at(k) + 1]);
  p.closePath();
}
// a track as a ring: the capsule of width w around a–b (8 segments per round end)
function mapCapsule(p, ax, ay, bx, by, w) {
  const r = w / 2, L = Math.hypot(bx - ax, by - ay), ux = L > 1e-9 ? (bx - ax) / L : 1, uy = L > 1e-9 ? (by - ay) / L : 0, xy = [];
  const t0 = Math.atan2(uy, ux);
  for (let k = 0; k <= 8; k++) { const t = t0 - Math.PI / 2 + Math.PI * k / 8; xy.push(bx + r * Math.cos(t), by + r * Math.sin(t)); }
  for (let k = 0; k <= 8; k++) { const t = t0 + Math.PI / 2 + Math.PI * k / 8; xy.push(ax + r * Math.cos(t), ay + r * Math.sin(t)); }
  mapRing(p, xy);
}
// The copper of the nets `nets` (indices) on copper layer `l` as one path in board coordinates (`clip`,
// non-zero: zones, pads, vias, tracks; the synthetic pad tracks left out, the pad itself is there) and
// the layer's tracks (`list`, indices into scene.tracks); kept per board with the other static paths
function mapCopper(nets, l) {
  const st = S.static, key = nets.join(",") + "|" + l;
  st.cu = st.cu || new Map();
  if (st.cu.has(key)) return st.cu.get(key);
  const set = new Set(nets), sc = S.scene, bit = 1 << l, clip = new Path2D();
  const { idx, pts } = S.zones;
  for (let i = 0; i < idx.length; i += 4) if (idx[i] === l && set.has(idx[i + 1])) mapRing(clip, pts.subarray(idx[i + 2] * 2, (idx[i + 2] + idx[i + 3]) * 2));
  const P = sc.pads;
  for (let i = 0; i < P.length / 8; i++) {
    if (!set.has(P[i * 8 + 7]) || !(P[i * 8 + 6] & bit)) continue;
    const custom = P[i * 8 + 5] === 4 && S.padCustom ? S.padCustom.get(i) : null;
    if (!custom) { clip.addPath(padShape(i)); continue; } // rect, roundRect, arc: the canvas's own turn
    const m = new DOMMatrix().translate(P[i * 8], P[i * 8 + 1]).rotate(-P[i * 8 + 4]);
    for (const poly of custom) mapRing(clip, poly.flatMap((q) => { const t = m.transformPoint({ x: q[0], y: q[1] }); return [t.x, t.y]; }));
  }
  const V = sc.vias;
  for (let i = 0; i < V.length; i += 7) {
    if (!set.has(V[i + 6]) || l < Math.min(V[i + 4], V[i + 5]) || l > Math.max(V[i + 4], V[i + 5])) continue;
    clip.moveTo(V[i] + V[i + 2] / 2, V[i + 1]); clip.arc(V[i], V[i + 1], V[i + 2] / 2, 0, Math.PI * 2); clip.closePath();
  }
  const T = sc.tracks, list = [];
  for (let i = 0; i < T.length / 7; i++) {
    const o = i * 7;
    if (T[o + 5] !== l || !set.has(T[o + 6]) || (S.padOfTrack && S.padOfTrack.has(i))) continue;
    mapCapsule(clip, T[o], T[o + 1], T[o + 2], T[o + 3], T[o + 4]);
    list.push(i);
  }
  const r = { clip, list };
  st.cu.set(key, r);
  return r;
}
// The board outline as a fill (cut-outs even-odd): the Edge.Cuts pieces chained end to end into closed
// loops (ends within 5 µm). null when a loop stays open – the core closes gaps and finds panels
// (kisi_pi::thermal::outline), the viewer then uses the map's own cells as the edge.
function mapOutline() {
  const st = S.static;
  if (st.outlineFill !== undefined) return st.outlineFill;
  const key = (q) => Math.round(q[0] * 200) + "," + Math.round(q[1] * 200);
  const pieces = S.scene.outline.filter((p) => p.length >= 2), ends = new Map(), used = new Uint8Array(pieces.length);
  pieces.forEach((p, i) => { for (const e of [0, 1]) { const k = key(e ? p[p.length - 1] : p[0]); if (!ends.has(k)) ends.set(k, []); ends.get(k).push(i); } });
  const fill = new Path2D();
  let ok = pieces.length > 0;
  for (let i = 0; i < pieces.length && ok; i++) {
    if (used[i]) continue;
    used[i] = 1;
    const loop = pieces[i].slice(), start = key(loop[0]);
    for (let guard = 0; key(loop[loop.length - 1]) !== start && guard < pieces.length; guard++) {
      const k = key(loop[loop.length - 1]), nx = (ends.get(k) || []).find((j) => !used[j]);
      if (nx == null) { ok = false; break; }
      used[nx] = 1;
      const p = pieces[nx], fwd = key(p[0]) === k;
      for (const q of (fwd ? p : p.slice().reverse()).slice(1)) loop.push(q);
    }
    if (key(loop[loop.length - 1]) !== start) { ok = false; break; }
    fill.moveTo(loop[0][0], loop[0][1]);
    for (let k = 1; k < loop.length; k++) fill.lineTo(loop[k][0], loop[k][1]);
    fill.closePath();
  }
  st.outlineFill = ok ? fill : null;
  return st.outlineFill;
}
// Cut the map image to a path once, at the image's resolution (`destination-in`): the losses of one
// layer cut each rail's image to its own copper, so that its extension does not cover a neighbour's
function mapBake(m, path, rule = "nonzero") {
  if (!m) return;
  const x = m.canvas.getContext("2d"), k = m.f / m.cell;
  x.setTransform(k, 0, 0, k, -m.x0 * k, -m.y0 * k);
  x.globalCompositeOperation = "destination-in"; x.fillStyle = "#000"; x.fill(path, rule);
  x.globalCompositeOperation = "source-over"; x.setTransform(1, 0, 0, 1, 0, 0);
}

// Draw a map onto the board (ctx in board coordinates) with the maps' transparency: the image and the
// isolines or arrows (`atop`) within the clip path – the real copper, or the board outline –, the track
// strokes (`over`) after it, not clipped: thin tracks keep a visible width when zoomed out.
// layers: { clip?: Path2D, rule?, image(c), atop?(c), over?(c) }
function mapDraw(layers) {
  ctx.save();
  if (layers.clip) ctx.clip(layers.clip, layers.rule || "nonzero");
  ctx.globalAlpha = MAP_ALPHA;
  layers.image(ctx);
  if (layers.atop) layers.atop(ctx);
  ctx.restore();
  if (layers.over) { ctx.save(); ctx.globalAlpha = MAP_ALPHA; layers.over(ctx); ctx.restore(); }
}

// The tracks narrower than MAP_LINE_CELLS cells as strokes coloured along their length: samples every
// half cell, bilinear on the map's extended values (the same values as the picture around them; along a
// line conductor's chain of cells they are the chain's own), runs of one colour step as one butt-ended
// stroke, a round dot of the end's colour at both ends (joints), in the map's colours. Returns paths by
// colour step.
function mapStrokes(cu, m) {
  const T = S.scene.tracks, runs = new Map(), dots = new Map(), cell = m.cell, maxW = MAP_LINE_CELLS * cell, { w, h, vals: a } = m;
  const lo = m.lo, hi = m.hi;
  const step = (v) => Math.max(0, Math.min(MAP_BUCKETS - 1, Math.round((v - lo) / ((hi - lo) || 1) * (MAP_BUCKETS - 1))));
  const at = (x, y) => {
    const u = (x - m.x0) / cell - 0.5, v = (y - m.y0) / cell - 0.5, i = Math.floor(u), j = Math.floor(v), fx = u - i, fy = v - j;
    let s2 = 0, n = 0;
    for (const [di, dj, wt] of [[0, 0, (1 - fx) * (1 - fy)], [1, 0, fx * (1 - fy)], [0, 1, (1 - fx) * fy], [1, 1, fx * fy]]) {
      const ii = i + di, jj = j + dj;
      if (ii < 0 || jj < 0 || ii >= w || jj >= h) continue;
      const q = a[jj * w + ii]; if (q === q && wt > 0) { s2 += q * wt; n += wt; }
    }
    return n > 1e-6 ? s2 / n : NaN;
  };
  const add = (map, b, tw) => { const k = b + "|" + tw.toFixed(4); let p = map.get(k); if (!p) map.set(k, p = { b, w: tw, path: new Path2D() }); return p.path; };
  let n = 0;
  for (const i of cu.list) {
    const o = i * 7, tw = T[o + 4];
    if (tw >= maxW) continue;
    const ax = T[o], ay = T[o + 1], bx = T[o + 2], by = T[o + 3], L = Math.hypot(bx - ax, by - ay);
    const ns = Math.max(1, Math.ceil(L / (cell / 2)));
    let last = NaN, prev = -1, from = 0, first = -1;
    const flush = (to) => { if (prev < 0) return; const p = add(runs, prev, tw); p.moveTo(ax + (bx - ax) * from, ay + (by - ay) * from); p.lineTo(ax + (bx - ax) * to, ay + (by - ay) * to); };
    for (let k = 0; k < ns; k++) {
      const u = (k + 0.5) / ns;
      let v = at(ax + (bx - ax) * u, ay + (by - ay) * u);
      if (v !== v) v = last; // no cell next to the line here: keep the colour before
      if (v !== v) continue;
      last = v;
      const b = step(v);
      if (first < 0) first = b;
      if (b !== prev) { flush(k / ns); prev = b; from = k / ns; }
    }
    flush(1);
    if (first < 0) continue;
    n++;
    add(dots, first, tw).moveTo(ax + tw / 2, ay); add(dots, first, tw).arc(ax, ay, tw / 2, 0, Math.PI * 2);
    add(dots, prev, tw).moveTo(bx + tw / 2, by); add(dots, prev, tw).arc(bx, by, tw / 2, 0, Math.PI * 2);
  }
  const lut = m.lut, col = Array.from({ length: MAP_BUCKETS }, (_, b) => {
    if (!lut) return turbo(0.05 + 0.9 * b / (MAP_BUCKETS - 1));
    const i = Math.round(1023 * b / (MAP_BUCKETS - 1)) * 3;
    return `rgb(${lut[i]},${lut[i + 1]},${lut[i + 2]})`;
  });
  return { runs: [...runs.values()], dots: [...dots.values()], col, n };
}
function mapDrawStrokes(c, s, px) {
  if (!s) return;
  c.lineCap = "butt";
  for (const r of s.runs) { c.strokeStyle = s.col[r.b]; c.lineWidth = Math.max(r.w, 1.2 * px); c.stroke(r.path); }
  for (const r of s.dots) { if (r.w < 1.2 * px) continue; c.fillStyle = s.col[r.b]; c.fill(r.path); }
  c.lineCap = "round";
}

// a tidy step for 10 … 20 isolines up to `max`: 1, 2, 2.5 or 5 × 10^k
function mapIsoStep(max, n = 20) {
  const raw = max / n, e = 10 ** Math.floor(Math.log10(raw)), m = raw / e;
  return (m <= 1 ? 1 : m <= 2 ? 2 : m <= 2.5 ? 2.5 : m <= 5 ? 5 : 10) * e;
}
// Isolines of the extended values of a map (mapImage) every `step` up to its scale (marching squares,
// linear on the cell edges, saddles by the centre value), and label places: in squares of the
// copper's own cells, at most one per level and eighth of the map.
// segment table of marching squares: per corner code the edge pairs (edges 0 top v0-v1, 1 right v1-v2,
// 2 bottom v3-v2, 3 left v0-v3); the saddles 5 and 10 are decided by the centre value
const MAP_MS = [[], [3, 0], [0, 1], [3, 1], [1, 2], null, [0, 2], [3, 2], [3, 2], [0, 2], null, [1, 2], [3, 1], [0, 1], [3, 0], []];
function mapIso(m, step) {
  const t0 = performance.now();
  const { w, h, vals: a, own, cell } = m, X = m.x0 + cell / 2, Y = m.y0 + cell / 2;
  const path = new Path2D(), major = new Path2D(), cand = new Map();
  const span = Math.max(8, Math.ceil(Math.max(w, h) / 8)), top = m.hi, ex = [0, 0, 0, 0], ey = [0, 0, 0, 0];
  let segs = 0;
  for (let j = 0; j < h - 1; j++) for (let i = 0; i < w - 1; i++) {
    const k = j * w + i, v0 = a[k], v1 = a[k + 1], v2 = a[k + w + 1], v3 = a[k + w];
    if (!(v0 === v0 && v1 === v1 && v2 === v2 && v3 === v3)) continue;
    const mn = Math.min(v0, v1, v2, v3), mx = Math.max(v0, v1, v2, v3);
    let n = Math.max(1, Math.floor(mn / step) + 1);
    const inside = own[k] && own[k + 1] && own[k + w] && own[k + w + 1];
    for (; n * step < mx && n * step <= top; n++) {
      const L = n * step;
      const code = (v0 > L ? 1 : 0) | (v1 > L ? 2 : 0) | (v2 > L ? 4 : 0) | (v3 > L ? 8 : 0);
      if (code === 0 || code === 15) continue;
      // the four edge crossings (only the ones used are meaningful)
      ex[0] = i + (L - v0) / (v1 - v0); ey[0] = j;
      ex[1] = i + 1; ey[1] = j + (L - v1) / (v2 - v1);
      ex[2] = i + (L - v3) / (v2 - v3); ey[2] = j + 1;
      ex[3] = i; ey[3] = j + (L - v0) / (v3 - v0);
      let pr = MAP_MS[code];
      if (!pr) { const hi = (v0 + v1 + v2 + v3) / 4 > L; pr = (code === 5) === hi ? [3, 2, 0, 1] : [3, 0, 1, 2]; }
      const p = n % 5 === 0 ? major : path;
      for (let q = 0; q < pr.length; q += 2) {
        const e1 = pr[q], e2 = pr[q + 1];
        p.moveTo(X + ex[e1] * cell, Y + ey[e1] * cell); p.lineTo(X + ex[e2] * cell, Y + ey[e2] * cell);
        segs++;
        if (inside) {
          const ck = n * 4096 + Math.floor(i / span) * 64 + Math.floor(j / span);
          if (!cand.has(ck)) cand.set(ck, { n, x: X + (ex[e1] + ex[e2]) / 2 * cell, y: Y + (ey[e1] + ey[e2]) / 2 * cell });
        }
      }
    }
  }
  // label places: the levels in turn, so that every level gets one before any gets a second
  const by = new Map();
  for (const c of cand.values()) { if (!by.has(c.n)) by.set(c.n, []); by.get(c.n).push(c); }
  const labels = [], lists = [...by.values()];
  for (let r = 0; lists.some((l) => l.length > r); r++) for (const l of lists) if (l[r]) labels.push(l[r]);
  return { path, major, labels, step, segs, ms: performance.now() - t0 };
}
function mapDrawIso(c, iso, px) {
  if (!iso) return;
  c.strokeStyle = "rgba(0,0,0,0.7)";
  c.lineWidth = 1 * px; c.stroke(iso.path);
  c.lineWidth = 2 * px; c.stroke(iso.major);
}

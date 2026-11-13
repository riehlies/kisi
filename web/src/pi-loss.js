"use strict";
// ---- losses: power density map ("Losses" view) and power flow (Sankey in the power tree)
// The numbers come from kisi_core::losses (S.loss = engine.piLosses): heat per part, copper loss per rail
// (incl. ground return) and layer, every copper layer on the rails' own DC grids (W per mm² of copper), one
// raster per layer and of the parts (W per mm² of board), and the flow links. The via barrels' loss is not in
// the map planes: the vias are markers of their own (S.loss.vias, kisi_core::losses::loss_vias).

// colour scale: log over 2, 3 or 4 decades below its top, the 99th percentile of the shown values (as the
// current density: a few cells at via contacts would push everything else down); legend, remembered
const LOSS_DECADE_STEPS = [2, 3, 4];
function lossDecades() {
  if (S.lossDecades == null) { const v = lsGet("kisi.loss.decades"); S.lossDecades = LOSS_DECADE_STEPS.includes(v) ? v : 3; }
  return S.lossDecades;
}
// the 99th percentile of the positive values
function lossTop(vals) {
  let n = 0;
  for (let k = 0; k < vals.length; k++) if (vals[k] > 0) n++;
  const a = new Float32Array(n);
  n = 0;
  for (let k = 0; k < vals.length; k++) if (vals[k] > 0) a[n++] = vals[k];
  a.sort();
  return a.length ? a[Math.min(a.length - 1, Math.floor(a.length * 0.99))] : 0;
}
// The colours of the scale (0 … 1 = the bottom … the top): turbo from light blue up, so that the low end does
// not sink into the dark background (turbo's indigo start); over the lowest decade the colour fades out
// (opacity eased from 1 to 0 at the bottom) onto what is drawn underneath – on a layer the board's drawing of
// the copper (lossLayerCompose), for the sums the board – little reads as little, copper without current as plain
// board copper. Above the top: the top colour.
const LOSS_TURBO0 = 0.2;
function lossColor(t, decades) {
  const t0 = 1 / decades;
  if (t >= t0) return [...turboRGB(LOSS_TURBO0 + (0.95 - LOSS_TURBO0) * (t - t0) / (1 - t0)), 1];
  const u = Math.max(0, t) / t0;
  return [...turboRGB(LOSS_TURBO0), u * u * (3 - 2 * u)];
}
function lossLut() {
  const d = lossDecades(), key = String(d);
  if (S.lossLut && S.lossLut.key === key) return S.lossLut;
  const lut = new Uint8Array(1024 * 3), alpha = new Uint8Array(1024);
  for (let i = 0; i < 1024; i++) { const c = lossColor(i / 1023, d); lut.set(c.slice(0, 3), i * 3); alpha[i] = Math.round(255 * c[3]); }
  S.lossLut = { key, lut, alpha };
  return S.lossLut;
}

function lossPlanes() {
  const m = S.loss && S.loss.map;
  if (!m) return [];
  const su = S.scene.stackup.copper;
  return [{ id: "total", name: "Total" }, { id: "comp", name: "Parts" }, { id: "cu", name: "Copper, all layers" }]
    .concat(m.layers.map((l, i) => ({ id: "L" + i, name: su[l] ? su[l].name : "layer " + l })));
}
// heat source per mm² of board of the selected sum on the common raster, nx*ny cells: the copper layers
// (S.loss.buf: one plane per layer, then the parts) and the parts summed
function lossValues(plane) {
  const m = S.loss.map, per = m.nx * m.ny, nl = m.layers.length, b = S.loss.buf, out = new Float32Array(per);
  const add = (s) => { for (let k = 0; k < per; k++) out[k] += b[s * per + k]; };
  if (plane !== "comp") for (let l = 0; l < nl; l++) add(l);
  if (plane !== "cu") add(nl);
  return out;
}
// log10 of the values for the picture, at least the scale's bottom (zero included); NaN: no copper
function lossLog(vals, lo) {
  const lg = new Float32Array(vals.length), floor = Math.log10(lo);
  for (let k = 0; k < vals.length; k++) { const v = vals[k]; lg[k] = v === v ? (v > lo ? Math.log10(v) : floor) : NaN; }
  return lg;
}
// The sums are drawn smoothed: the common raster's cells (about 0.2 mm) step at every copper and hole edge, the
// heat spreads anyway. A Gaussian of LOSS_SIGMA cells on the linear values (normalised weights, renormalised
// at the raster's edge) – a narrow track keeps its heat as a softer, wider line, whereas smoothing the log
// values would sink it towards the bottom of the scale; one cell leaves holes and gaps of half a millimetre
// as dips, 1.5 fills them. Then interpolated ×4 (LOSS_PX: a larger pixel budget than the other maps, some
// 25 MB on a large board with the half-resolution copy for the whole-board view). The picture only: the scale (top, bottom) and the tooltip read the unsmoothed
// values (S.lossSigma: forced, for tests; 0: off).
const LOSS_SIGMA = 1, LOSS_PX = 5e6;
function lossSmooth(vals, nx, ny, sigma) {
  if (!(sigma > 0)) return vals;
  const r = Math.ceil(3 * sigma), g = new Float32Array(2 * r + 1);
  for (let k = -r; k <= r; k++) g[k + r] = Math.exp(-(k * k) / (2 * sigma * sigma));
  const pass = (src, dst, n, len, step, stride) => { // along lines of `len` cells (`step` apart), `n` lines `stride` apart
    for (let l = 0; l < n; l++) {
      const b = l * stride;
      for (let i = 0; i < len; i++) {
        const a0 = Math.max(0, i - r), a1 = Math.min(len - 1, i + r);
        let s = 0, w = 0;
        for (let a = a0; a <= a1; a++) { const v = src[b + a * step]; if (v === v) { const gw = g[a - i + r]; s += v * gw; w += gw; } }
        dst[b + i * step] = w > 0 ? s / w : NaN;
      }
    }
  };
  const t = new Float32Array(vals.length), out = new Float32Array(vals.length);
  pass(vals, t, ny, nx, 1, nx);
  pass(t, out, nx, ny, nx, 1);
  return out;
}
// a copper layer of the plane L<i>: its rails on their own DC grids (kisi_pi::losses::loss_layer)
const lossLayerOf = (pl) => pl[0] === "L" ? S.loss.layers[+pl.slice(1)] : null;
// a rail plane of a layer as a grid (NaN: no copper) from its copper cells
function lossDense(p) {
  const out = new Float32Array(p.nx * p.ny).fill(NaN), I = S.loss.layerIdx, V = S.loss.layerBuf;
  for (let q = p.offset; q < p.offset + p.n; q++) out[I[q]] = V[q];
  return out;
}
// the density of a rail plane at cell k of its grid (its copper cells are in order), NaN: no copper
function lossCell(p, k) {
  const I = S.loss.layerIdx;
  let a = p.offset, b = p.offset + p.n;
  while (a < b) { const c = (a + b) >> 1; if (I[c] < k) a = c + 1; else b = c; }
  return a < p.offset + p.n && I[a] === k ? S.loss.layerBuf[a] : NaN;
}
// The picture of the shown plane (pi-map.js) on a log scale. A layer: drawn as the current density – one
// image per rail on its own DC cells (factor by its own cell count), each clipped every frame to its rail's
// exact copper, the narrow tracks as strokes; the board's copper underneath. The sums (total, parts, all
// copper): a heat source per mm² of board on the common raster, drawn as the temperature – values smoothed
// (lossSmooth) and interpolated over the board, clipped to the board outline.
function lossImage() {
  const { lut, alpha } = lossLut(), key = S.lossPlane + "|" + S.lossLut.key;
  if (S.lossImg && S.lossImg.key === key) return S.lossImg;
  const m = S.loss.map, pl = S.lossPlane, ll = lossLayerOf(pl), decades = lossDecades();
  if (ll) {
    const nets = [...S.piRes.entries()].filter(([, r]) => r.res).map(([n]) => S.scene.nets.indexOf(n)).filter((n) => n >= 0);
    const clip = mapCopper(nets, ll.layer).clip;
    const top = lossTop(S.loss.layerBuf.subarray(ll.planes.length ? ll.planes[0].offset : 0, ll.planes.length ? ll.planes[ll.planes.length - 1].offset + ll.planes[ll.planes.length - 1].n : 0));
    const lo = top / 10 ** decades, sc = { lo: Math.log10(lo), hi: Math.log10(top), lut, alpha }, rails = [];
    for (const p of ll.planes) {
      const net = S.scene.nets.indexOf(p.net);
      if (!(p.n > 0) || net < 0 || !(top > 0)) continue;
      const map = mapImage(lossLog(lossDense(p), lo), p.nx, p.ny, { x0: p.x0, y0: p.y0, cell: p.cell, ...sc });
      if (!map) continue;
      const cu = mapCopper([net], ll.layer);
      rails.push({ p, map, cu, strokes: mapStrokes(cu, map) });
    }
    S.lossImgN = (S.lossImgN || 0) + 1; // the composed layer (lossLayerDraw) belongs to this image set
    S.lossImg = { key, id: S.lossImgN, plane: pl, ll, rails, clip, max: top, lo, bytes: rails.reduce((a, r) => a + r.map.bytes, 0) };
    return S.lossImg;
  }
  const vals = lossValues(pl), top = lossTop(vals), lo = top / 10 ** decades, ol = mapOutline();
  const o = { x0: m.x0, y0: m.y0, cell: m.cell, lo: Math.log10(lo), hi: Math.log10(top), lut, alpha, px: LOSS_PX, half: true };
  const sm = top > 0 ? lossSmooth(vals, m.nx, m.ny, S.lossSigma ?? LOSS_SIGMA) : null;
  const map = sm ? mapImage(lossLog(sm, lo), m.nx, m.ny, ol ? o : { ...o, dilate: 0, cover: 0.5 }) : null;
  S.lossImg = { key, plane: pl, map, clip: ol, max: top, lo, vals, bytes: map ? map.bytes : 0 };
  return S.lossImg;
}
// A layer as drawn: the board's drawing of its copper (the zone colour, as the board view draws zones) where
// the scale fades out, then every rail in view as the current density draws one – its image within its exact
// copper, its narrow tracks as strokes. Composed at screen resolution into an offscreen canvas (the view and a
// margin) once per zoom level and copied while panning – the clips of some thirty rails cost a few frames;
// after a zoom the old one is scaled until the view rests for LOSS_LAYER_MS, then composed anew.
const LOSS_LAYER_MARGIN = 0.15, LOSS_LAYER_MS = 150;
function lossLayerCompose(x, im, px, bx0, by0, bx1, by1) {
  x.fillStyle = S.colors.zone; x.fill(im.clip);
  for (const r of im.rails) {
    const b = r.map.bbox;
    if (b[2] < bx0 || b[0] > bx1 || b[3] < by0 || b[1] > by1) continue;
    x.save(); x.clip(r.cu.clip); x.globalAlpha = MAP_ALPHA; mapPaint(x, r.map); x.restore();
    x.save(); x.globalAlpha = MAP_ALPHA; mapDrawStrokes(x, r.strokes, px); x.restore();
  }
}
function lossLayerDraw(im, px) {
  const d = window.devicePixelRatio || 1, v = S.view, k = v.s * d, key = im.id + "|" + S.colors.zone;
  const bx0 = -v.x / v.s, by0 = -v.y / v.s, bw = cv.width / k, bh = cv.height / k;
  let u = S.lossCache;
  const same = u && u.key === key, fits = same && u.k === k && u.bx0 <= bx0 && u.by0 <= by0 && u.bx0 + u.bw >= bx0 + bw && u.by0 + u.bh >= by0 + bh;
  if (!fits) {
    if (same && u.k !== k && !S.lossCacheDue) {
      clearTimeout(S.lossCacheT);
      S.lossCacheT = setTimeout(() => { S.lossCacheDue = true; draw(); }, LOSS_LAYER_MS);
    } else {
      S.lossCacheDue = false;
      const mx = bw * LOSS_LAYER_MARGIN, my = bh * LOSS_LAYER_MARGIN, c = (u && u.canvas) || document.createElement("canvas");
      c.width = Math.ceil(cv.width * (1 + 2 * LOSS_LAYER_MARGIN)); c.height = Math.ceil(cv.height * (1 + 2 * LOSS_LAYER_MARGIN));
      const x = c.getContext("2d");
      x.setTransform(1, 0, 0, 1, 0, 0); x.clearRect(0, 0, c.width, c.height);
      x.setTransform(k, 0, 0, k, -(bx0 - mx) * k, -(by0 - my) * k);
      lossLayerCompose(x, im, px, bx0 - mx, by0 - my, bx0 + bw + mx, by0 + bh + my);
      u = S.lossCache = { key, k, bx0: bx0 - mx, by0: by0 - my, bw: c.width / k, bh: c.height / k, canvas: c };
    }
  }
  ctx.imageSmoothingEnabled = u.k !== k; // 1:1 while panning
  ctx.drawImage(u.canvas, u.bx0, u.by0, u.bw, u.bh);
  ctx.imageSmoothingEnabled = true;
}
// the value of the shown plane at a board point: {v, net} (a layer: the rail whose copper is there)
function lossAt(im, x, y) {
  if (im.ll) {
    for (const { p } of im.rails) {
      const i = Math.floor((x - p.x0) / p.cell), j = Math.floor((y - p.y0) / p.cell);
      if (i < 0 || j < 0 || i >= p.nx || j >= p.ny) continue;
      const v = lossCell(p, j * p.nx + i);
      if (v === v) return { v, net: p.net };
    }
    return { v: NaN, net: null };
  }
  const m = S.loss.map, i = Math.floor((x - m.x0) / m.cell), j = Math.floor((y - m.y0) / m.cell);
  return { v: i < 0 || j < 0 || i >= m.nx || j >= m.ny ? NaN : im.vals[j * m.nx + i], net: null };
}

// ---- vias: their barrels' loss as markers (not in the map's colours: half a barrel's I²R on its contact cell
// is a point loss whose W/mm² grows with 1/cell²). Shown: the vias with at least a share of the largest via loss
// (the legend's choice, remembered per viewer), on a layer those whose current flows through it, on the sums all;
// a disc at constant screen size, radius and colour by the loss (log over two decades, magenta – outside the
// map's turbo), a halo ring around it, the ring in the warning colour above the capacity estimate.
const LOSS_VIA_STEPS = [0.01, 0.02, 0.05, 0.1], LOSS_VIA_DEFAULT = 0.02, LOSS_VIA_LEVELS = 12;
function lossViaMin() {
  if (S.lossViaMin == null) { const v = lsGet("kisi.loss.viamin"); S.lossViaMin = LOSS_VIA_STEPS.includes(v) ? v : LOSS_VIA_DEFAULT; }
  return S.lossViaMin;
}
function lossVias() {
  const min = lossViaMin(), pl = S.lossPlane, src = S.loss;
  if (S.lossViaSel && S.lossViaSel.src === src && S.lossViaSel.min === min && S.lossViaSel.pl === pl) return S.lossViaSel;
  const all = (src.vias && src.vias.list) || [], wmax = all.length ? all[0].loss_w : 0, ll = lossLayerOf(pl);
  const list = pl === "comp" || !(wmax > 0) ? [] : all.filter((v) => v.loss_w >= min * wmax && (!ll || (v.from <= ll.layer && ll.layer <= v.to)));
  S.lossViaSel = { src, min, pl, list, wmax };
  return S.lossViaSel;
}
const lossViaLevel = (w, wmax) => Math.round(Math.min(1, Math.max(0, 1 + Math.log10(Math.max(w, 1e-30) / wmax) / 2)) * (LOSS_VIA_LEVELS - 1));
// light lilac to deep magenta
const lossViaFill = (k) => { const t = k / (LOSS_VIA_LEVELS - 1); return `rgb(${Math.round(246 - 96 * t)},${Math.round(196 - 196 * t)},${Math.round(240 - 100 * t)})`; };
const lossViaRadius = (k) => 3 + 5 * k / (LOSS_VIA_LEVELS - 1); // screen px
function drawLossVias(px) {
  const { list, wmax } = lossVias();
  if (!list.length) return;
  const d = window.devicePixelRatio || 1, x0 = -S.view.x / S.view.s, y0 = -S.view.y / S.view.s;
  const x1 = x0 + cv.width / d / S.view.s, y1 = y0 + cv.height / d / S.view.s;
  const fills = Array.from({ length: LOSS_VIA_LEVELS }, () => new Path2D()), halo = new Path2D(), over = new Path2D();
  for (let i = list.length - 1; i >= 0; i--) {
    const v = list[i], k = lossViaLevel(v.loss_w, wmax), r = lossViaRadius(k) * px, x = v.at[0], y = v.at[1];
    if (x + r < x0 || x - r > x1 || y + r < y0 || y - r > y1) continue;
    fills[k].moveTo(x + r, y); fills[k].arc(x, y, r, 0, Math.PI * 2);
    const rh = r + 1.5 * px;
    (v.load > 1 ? over : halo).moveTo(x + rh, y); (v.load > 1 ? over : halo).arc(x, y, rh, 0, Math.PI * 2);
  }
  ctx.fillStyle = S.colors.halo; ctx.fill(halo);
  ctx.fillStyle = getComputedStyle(document.documentElement).getPropertyValue("--z-high2").trim(); ctx.fill(over);
  fills.forEach((p, k) => { ctx.fillStyle = lossViaFill(k); ctx.fill(p); });
}
// a via marker under the mouse (screen distance), else null
function lossViaAt(mx, my) {
  const { list, wmax } = lossVias();
  let best = null, bd = 1e9;
  for (const v of list) {
    const d = Math.hypot(S.view.x + v.at[0] * S.view.s - mx, S.view.y + v.at[1] * S.view.s - my);
    if (d < lossViaRadius(lossViaLevel(v.loss_w, wmax)) + 3 && d < bd) { bd = d; best = v; }
  }
  return best;
}
function lossViaTip(v) {
  const vs = S.loss.vias;
  return `<div class="t1">${v.hole ? "Plated hole" : "Via"} · ${esc(netLabel(v.net))}</div><div class="kv">
    <span>Current</span><span>${fmtSI(v.current, "A")} <span class="muted">${esc(layerName(v.from))} → ${esc(layerName(v.to))}</span></span>
    <span>Barrel loss</span><span>${fmtSI(v.loss_w, "W")}</span>
    <span>Current density</span><span>${fmt(v.j_a_mm2, 1)} A/mm² <span class="muted">in ${fmt(v.area_mm2, 4)} mm² of plating</span></span>
    <span>Capacity</span><span>${fmtSI(v.capacity_a, "A")} at ΔT ${fmt(vs.delta_t_k, 0)} K, IPC-2152 · estimate</span>
    <span>Current / capacity</span><span${v.load > 1 ? ' class="warn"' : ""}>${fmt(100 * v.load, 0)} %</span>
    <span>Drill</span><span>${fmtSI(v.drill * 1e-3, "m")}</span></div>`;
}

function drawLoss(px) {
  if (!S.loss || !S.loss.map) return;
  const im = lossImage();
  if (im.rails) lossLayerDraw(im, px);
  else if (im.map) mapDraw({ clip: im.clip, rule: "evenodd", image: (c) => mapPaint(c, im.map) });
  drawLossVias(px);
  // the largest heat sources: labels at a constant screen size, the hottest first; a label that
  // would overlap a hotter one is left out (more appear when zoomed in)
  S.lossLabels = [];
  if (S.lossPlane === "total" || S.lossPlane === "comp") {
    const items = S.loss.summary.components.filter((c) => c.at && c.w > 0).slice(0, 25).map((c) => ({ x: c.at[0], y: c.at[1], text: `${c.refdes} ${fmtSI(c.w, "W")}` }));
    S.lossLabels = drawBoardLabels(items, { max: 12 });
  }
}

function renderLossChips() {
  const planes = lossPlanes();
  if (!planes.some((p) => p.id === S.lossPlane)) S.lossPlane = "total";
  $("layers").innerHTML = planes.map((p) => `<button class="chip" data-lp="${p.id}" aria-pressed="${p.id === S.lossPlane}"><i></i>${esc(p.name)}</button>`).join("");
  $("layers").querySelectorAll(".chip").forEach((b) => b.onclick = () => { S.lossPlane = b.dataset.lp; renderLossChips(); renderLossLegend(); draw(); });
}

function renderLossLegend() {
  const L = $("legend");
  if (!S.loss || !S.loss.map) { L.innerHTML = `<strong>Losses</strong><div class="row">${S.loss && S.loss.error ? esc(S.loss.error) : "after “Compute” in step 2"}</div>`; return; }
  const im = lossImage(), plane = lossPlanes().find((p) => p.id === S.lossPlane), d = lossDecades(), C = S.colors, layer = !!im.ll;
  // the scale over what lies underneath, as on the map: the board's copper on a layer, the board for the sums
  const stops = Array.from({ length: 25 }, (_, k) => { const c = lossColor(k / 24, d); return `rgba(${c[0]},${c[1]},${c[2]},${(c[3] * MAP_ALPHA).toFixed(3)})`; }).join(",");
  const v = (x) => fmtSI(x, "W/mm²", 2);
  L.innerHTML = `<strong>Power density · ${esc(plane ? plane.name : "")} · log scale</strong>
    <div class="grad" style="background:linear-gradient(90deg,${stops}),${layer ? `linear-gradient(${C.zone},${C.zone}),` : ""}${C.canvas}"></div>
    <div class="ticks units"><span>≤ ${v(im.lo)}</span><span>${v(im.lo * 10 ** (d / 2))}</span><span>≥ ${v(im.max)}</span></div>
    <div class="row"><span>Scale <select id="loss-dec" aria-label="decades of the log scale">${LOSS_DECADE_STEPS.map((n) => `<option value="${n}"${n === d ? " selected" : ""}>${n} decades</option>`).join("")}</select>
      below the 99th percentile; over the lowest decade the colour fades into the ${layer ? "board's copper (without current: plain copper)" : "board"}</span></div>
    <div class="row">${layer ? "Copper: I²R of every rail and the ground return per mm² of copper, on each rail's own cells; the via barrels' loss as markers, not in the colours."
      : S.lossPlane === "comp" ? "Heat source per mm² of board: the parts' heat over the area of their pads (estimate)."
      : `Heat source per mm² of board: the copper loss of the layers${S.lossPlane === "total" ? " and the parts' heat (over the area of their pads, estimate)" : ""} summed, as the temperature takes it; the via barrels' loss (heat for the temperature too) as markers.`}</div>
    ${lossViaLegend()}`;
  const sel = $("loss-dec");
  if (sel) sel.onchange = () => { S.lossDecades = +sel.value; lsSet("kisi.loss.decades", S.lossDecades); renderLossLegend(); draw(); };
  const vsel = $("loss-vmin");
  if (vsel) vsel.onchange = () => { S.lossViaMin = +vsel.value; lsSet("kisi.loss.viamin", S.lossViaMin); renderLossLegend(); draw(); };
}

function lossViaLegend() {
  const vs = S.loss.vias;
  if (!vs || !vs.list.length || S.lossPlane === "comp") return "";
  const { list, wmax } = lossVias(), n = list.filter((v) => v.load > 1).length;
  return `<div class="row" id="loss-vias"><span class="viadot" style="background:${lossViaFill(LOSS_VIA_LEVELS - 1)}"></span><span>${list.length} vias${lossLayerOf(S.lossPlane) ? " through this layer" : ""} with at least
      <select id="loss-vmin" aria-label="vias shown: share of the largest via loss">${LOSS_VIA_STEPS.map((f) => `<option value="${f}"${f === lossViaMin() ? " selected" : ""}>${Math.round(f * 100)} %</option>`).join("")}</select>
      of the largest barrel loss (${fmtSI(lossViaMin() * wmax, "W")} … ${fmtSI(wmax, "W")}): larger and darker = more (log from 1&nbsp;%); all vias ${fmtSI(vs.total_w, "W")}.
      ${n ? `<span class="warn">${n} above the capacity estimate (red ring)</span>` : ""} Hover for current and capacity (estimate).</span></div>`;
}

function showLossTip(mx, my) {
  const tip = $("tip");
  if (!S.loss || !S.loss.map) { tip.hidden = true; return; }
  const via = lossViaAt(mx, my);
  if (via) { tip.innerHTML = lossViaTip(via); tip.hidden = false; lossTipPlace(tip, mx, my); return; }
  const m = S.loss.map, im = lossImage();
  const x = (mx - S.view.x) / S.view.s, y = (my - S.view.y) / S.view.s, at = lossAt(im, x, y);
  const i = Math.floor((x - m.x0) / m.cell), j = Math.floor((y - m.y0) / m.cell);
  if (i < 0 || j < 0 || i >= m.nx || j >= m.ny) { tip.hidden = true; return; }
  const k = j * m.nx + i, per = m.nx * m.ny, nl = m.layers.length, b = S.loss.buf, su = S.scene.stackup.copper;
  const near = S.loss.summary.components.filter((c) => c.at).map((c) => ({ c, d: Math.hypot(c.at[0] - x, c.at[1] - y) })).sort((a, b2) => a.d - b2.d)[0];
  const d = (w) => w === w ? fmtSI(w, "W/mm²") : "–";
  let rows;
  if (im.ll) {
    if (!(at.v === at.v)) { tip.hidden = true; return; }
    rows = `<span>Shown</span><span>${d(at.v)} <span class="muted">per mm² of copper</span></span>
      <span>Rail</span><span>${esc(netLabel(at.net))}</span>`;
  } else {
    if (!(at.v > 0)) { tip.hidden = true; return; }
    // the sum and what it is made of: the parts and every layer's copper loss at this cell
    const per_l = m.layers.map((l, s2) => [su[l] ? su[l].name : l, b[s2 * per + k]]).filter((e) => e[1] > 0).sort((a2, b2) => b2[1] - a2[1]);
    rows = `<span>Shown</span><span>${d(at.v)} <span class="muted">per mm² of board</span></span>
      ${S.lossPlane !== "cu" ? `<span>Parts</span><span>${d(b[nl * per + k])}</span>` : ""}
      ${S.lossPlane !== "comp" ? per_l.slice(0, 6).map(([n, w]) => `<span>${esc(n)}</span><span>${d(w)}</span>`).join("") : ""}`;
  }
  tip.innerHTML = `<div class="t1">Power density</div><div class="kv">${rows}
    ${near && near.d < 3 ? `<span>Part</span><span>${esc(near.c.refdes)} ${esc(near.c.part)} · ${fmtSI(near.c.w, "W")}</span>` : ""}</div>`;
  tip.hidden = false;
  lossTipPlace(tip, mx, my);
}
function lossTipPlace(tip, mx, my) {
  const rs = $("stage").getBoundingClientRect(), tw = tip.offsetWidth, th = tip.offsetHeight;
  let tx = mx + 16, ty = my + 16;
  if (tx + tw > rs.width - 8) tx = mx - tw - 16;
  if (ty + th > rs.height - 8) ty = my - th - 16;
  tip.style.left = Math.max(8, tx) + "px"; tip.style.top = Math.max(8, ty) + "px";
}

// side panel of the Losses view: totals, per layer, per rail, hottest parts
function renderLossDetail() {
  const box = $("pi-detail");
  if (!S.loss || !S.loss.summary) { box.hidden = true; return; }
  const s = S.loss.summary, su = S.scene.stackup.copper;
  const bar = (w, max, cls) => `<div class="bar"><i class="${cls}" style="width:${max > 0 ? Math.max(1, 100 * w / max) : 0}%"></i></div>`;
  const maxL = Math.max(...s.layers.map((l) => l[2]), 1e-12);
  const layers = s.layers.map(([l, name, w]) => `<tr><td>${esc(name || (su[l] && su[l].name) || l)}</td><td class="n">${fmtSI(w, "W")}</td><td>${bar(w, maxL, "l")}</td></tr>`).join("");
  const rails = s.rails.filter((r) => r.w > 0).slice(0, 8);
  const maxR = Math.max(...rails.map((r) => r.w), 1e-12);
  const railRows = rails.map((r) => `<tr><td><button class="linkish" data-lrail="${esc(r.net)}">${esc(netShort(r.net))}</button>${r.ground ? " <span class=\"muted\">return</span>" : ""}</td><td class="n">${fmtSI(r.w, "W")}</td><td>${bar(r.w, maxR, "l")}</td></tr>`).join("");
  const comps = s.components.slice(0, 10), maxC = Math.max(...comps.map((c) => c.w), 1e-12);
  const compRows = comps.map((c) => `<tr><td>${esc(c.refdes)}</td><td>${esc(c.part)}</td><td class="n">${fmtSI(c.w, "W")}</td><td>${bar(c.w, maxC, c.role === "load" ? "u" : "l")}</td></tr>`).join("");
  box.innerHTML = `<header><div><h3>Losses · ${fmtSI(s.total_w, "W")}</h3><div class="muted">where power turns into heat: parts and copper (rails and ground return)</div></div><button class="x" id="closePi" aria-label="Close">×</button></header>
    <div class="dstats"><span>Parts <b>${fmtSI(s.components_w, "W")}</b></span><span>Copper, rails <b>${fmtSI(s.copper_w, "W")}</b></span><span>Copper, ground return <b>${fmtSI(s.ground_w, "W")}</b></span></div>
    <h4 class="sub">Copper per layer</h4>
    <div class="tablewrap"><table class="xs-table ctab" data-head="Layer|Copper loss"><tbody>${layers || `<tr><td>no copper losses yet</td></tr>`}</tbody></table></div>
    ${S.loss.vias && S.loss.vias.count ? `<p class="note">Of which via barrels ${fmtSI(S.loss.vias.total_w, "W")} in ${S.loss.vias.count} vias and plated holes: markers on the map, not in its colours.</p>` : ""}
    <h4 class="sub">Copper per rail</h4>
    <div class="tablewrap"><table class="xs-table ctab" data-head="Rail|Copper loss"><tbody>${railRows || `<tr><td>–</td></tr>`}</tbody></table></div>
    <h4 class="sub">Hottest parts</h4>
    <div class="tablewrap"><table class="xs-table ctab" data-head="Part|Value|Heat"><tbody>${compRows || `<tr><td>no power tree</td></tr>`}</tbody></table></div>
    <p class="note">${esc((s.notes || []).join(" "))} Shown is power (W, W/mm²), not temperature.</p>`;
  box.hidden = false;
  $("closePi").onclick = () => { box.hidden = true; };
  box.querySelectorAll("[data-lrail]").forEach((b) => b.onclick = () => { S.piRail = b.dataset.lrail; S.piLayer = -1; setPiMode("v"); fitRail(); });
}

// ---- power flow (Sankey) -------------------------------------------------------------
// nodes in columns (input, rails, regulators, …, loads and losses); band width ∝ W.
// Loads below 1.5 % of the input are merged into one node.
function sankeySVG(flow, opts = {}) {
  if (!flow || !flow.links.length) return "";
  const W = opts.width || 1000, nodeW = 10, gap = 6, padTop = 6;
  const nodes = new Map(flow.nodes.map((n) => [n.id, { ...n, in: 0, out: 0, inL: [], outL: [] }]));
  const input = flow.links.filter((l) => nodes.get(l.from) && nodes.get(l.from).kind === "input").reduce((a, l) => a + l.w, 0) || 1;
  // merge small loads
  const loadIn = new Map();
  for (const l of flow.links) if (nodes.get(l.to) && nodes.get(l.to).kind === "load") loadIn.set(l.to, (loadIn.get(l.to) || 0) + l.w);
  const small = new Set([...loadIn].filter(([, w]) => w < 0.015 * input).map(([id]) => id));
  let links = flow.links.map((l) => ({ ...l }));
  if (small.size > 1) {
    const depth = nodes.get([...small][0]).depth;
    nodes.set("load:other", { id: "load:other", label: `${small.size} smaller loads`, kind: "load", depth, members: [...small].map((id) => nodes.get(id).label), in: 0, out: 0, inL: [], outL: [] });
    for (const id of small) nodes.delete(id);
    const m = new Map();
    for (const l of links) {
      const to = small.has(l.to) ? "load:other" : l.to, k = l.from + ">" + to;
      if (m.has(k)) m.get(k).w += l.w; else m.set(k, { from: l.from, to, w: l.w });
    }
    links = [...m.values()];
  }
  links = links.filter((l) => nodes.has(l.from) && nodes.has(l.to));
  for (const l of links) { const a = nodes.get(l.from), b = nodes.get(l.to); a.out += l.w; b.in += l.w; a.outL.push(l); b.inL.push(l); }
  for (const n of nodes.values()) n.v = Math.max(n.in, n.out);
  const cols = [...new Set([...nodes.values()].map((n) => n.depth))].sort((a, b) => a - b);
  const colNodes = cols.map((d) => [...nodes.values()].filter((n) => n.depth === d));
  // vertical scale: the input column fills the base height; every node keeps room for its label
  const H0 = opts.height || 380, step = 14;
  const k = Math.min(...colNodes.map((c) => (H0 - padTop * 2 - gap * (c.length - 1)) / c.reduce((a, n) => a + n.v, 0)));
  const xOf = (i) => 4 + i * (W - 8 - nodeW - 200) / Math.max(1, cols.length - 1);
  // order: by the weighted position of the sources, losses at the bottom
  colNodes.forEach((c, ci) => {
    if (ci > 0) for (const n of c) {
      let s = 0, w = 0;
      for (const l of n.inL) { const a = nodes.get(l.from); if (a.y != null) { s += (a.y + a.h / 2) * l.w; w += l.w; } }
      n.key = w ? s / w : 1e9;
    }
    if (ci > 0) c.sort((a, b) => (a.kind === "loss") - (b.kind === "loss") || a.key - b.key);
    let y = padTop;
    for (const n of c) { n.h = Math.max(1, n.v * k); n.y = y; n.x = xOf(ci); y += Math.max(n.h + gap, step); }
  });
  const H = Math.ceil(Math.max(...[...nodes.values()].map((n) => n.y + Math.max(n.h, step))) + padTop);
  // band offsets
  for (const n of nodes.values()) {
    n.outL.sort((a, b) => nodes.get(a.to).y - nodes.get(b.to).y);
    n.inL.sort((a, b) => nodes.get(a.from).y - nodes.get(b.from).y);
    let o = n.y; for (const l of n.outL) { l.y0 = o; o += l.w * k; }
    let i2 = n.y; for (const l of n.inL) { l.y1 = i2; i2 += l.w * k; }
  }
  const val = (w) => fmtSI(w, "W");
  const kindCol = { input: "var(--ok, #3a8f5c)", rail: "var(--accent)", regulator: "var(--muted)", load: "var(--pw-use)", loss: "var(--pw-loss)" };
  const paths = links.map((l) => {
    const a = nodes.get(l.from), b = nodes.get(l.to), t = Math.max(0.6, l.w * k);
    const x0 = a.x + nodeW, x1 = b.x, xm = (x0 + x1) / 2;
    const loss = b.kind === "loss";
    return `<path d="M${x0},${l.y0 + t / 2} C${xm},${l.y0 + t / 2} ${xm},${l.y1 + t / 2} ${x1},${l.y1 + t / 2}" stroke="${loss ? "var(--pw-loss)" : "var(--pw-use)"}" stroke-opacity="${loss ? 0.55 : 0.28}" stroke-width="${t}" fill="none"><title>${esc(netLabel(a.label))} → ${esc(netLabel(b.label))}: ${val(l.w)}</title></path>`;
  }).join("");
  const last = cols.length - 1;
  // labels: full name and value in the last column, short names in between (details on hover)
  const shortLabel = (n) => {
    if (n.kind === "rail") return netLabel(n.label.replace(/ \(\+\d+\)$/, ""));
    if (n.kind === "regulator") return n.label.split(" ")[0];
    if (n.kind === "input") return n.label.split(" ")[0] + " · " + val(n.v);
    return n.label;
  };
  const rects = [...nodes.values()].map((n) => {
    const ci = cols.indexOf(n.depth);
    const full = netLabel(n.label);
    const text = ci === last ? `${full.length > 26 ? full.slice(0, 25) + "…" : full} · ${val(n.v)}` : shortLabel(n);
    const tip = `${n.label}: ${val(n.v)}${n.members && n.members.length > 1 ? "\n" + n.members.map((m) => netShort(m)).join(", ") : ""}`;
    const jump = n.kind === "rail" || ((n.kind === "load" || n.kind === "regulator" || n.kind === "input") && n.id !== "load:other");
    return `<g${jump ? ` data-node="${esc(n.id)}" tabindex="0"` : ""}><title>${esc(tip)}${jump ? `\n(click: show ${n.kind === "rail" ? "the rail" : "the part"} on the board)` : ""}</title><rect x="${n.x}" y="${n.y}" width="${nodeW}" height="${n.h}" fill="${kindCol[n.kind] || "var(--muted)"}" rx="1"></rect><text x="${n.x + nodeW + 4}" y="${n.y + Math.max(n.h, 1) / 2}" dy="0.32em" class="sk-l${n.kind === "loss" ? " sk-loss" : ""}">${esc(text)}</text></g>`;
  }).join("");
  return `<svg class="sankey" viewBox="0 0 ${W} ${H}" width="${W}" height="${H}" role="img" aria-label="Power flow from the input through regulators and rails to the loads and losses">${paths}${rects}</svg>`;
}

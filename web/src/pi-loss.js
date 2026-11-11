"use strict";
// ---- losses: power density map ("Losses" view) and power flow (Sankey in the power tree)
// The numbers come from kisi_core::losses (S.loss = engine.piLosses): heat per part, copper
// loss per rail (incl. ground return) and layer, a raster in W/mm² (copper per mm² of copper), every
// copper layer on the rails' own DC grids, and the flow links.

const LOSS_PX = 3e6;    // pixel budget of one layer's images together (one per rail, on its own DC grid)
// colour scale: log over 2, 3 or 4 decades below the largest value (legend; remembered per viewer)
const LOSS_DECADE_STEPS = [2, 3, 4];
function lossDecades() {
  if (S.lossDecades == null) { const v = lsGet("kisi.loss.decades"); S.lossDecades = LOSS_DECADE_STEPS.includes(v) ? v : 3; }
  return S.lossDecades;
}
// The colours of the scale (0 … 1 = the bottom … the largest value): turbo from light blue up, so that the
// low end does not sink into the dark background (turbo's indigo start); over the lowest decade the colour
// fades out (opacity eased from 1 to 0 at the bottom) onto the board's own drawing of the copper underneath
// (lossUnder: the zone colour) – little reads as little, copper without current as plain board copper.
// Only where there is no solved copper there is no copper drawing either.
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
// W/mm² of the selected plane of the common raster, nx*ny cells (NaN: neither solved copper nor a part):
// copper per mm² of copper, the layers summed; parts over the box of their pads
function lossValues(plane) {
  const m = S.loss.map, per = m.nx * m.ny, b = S.loss.buf, out = new Float32Array(per);
  for (let k = 0; k < per; k++) {
    const cu = b[k], cp = b[per + k];
    out[k] = plane === "comp" ? (cp > 0 ? cp : NaN) : plane === "cu" ? cu : cu === cu ? cu + cp : cp > 0 ? cp : NaN;
  }
  return out;
}
// log10 of the values for the picture, at least the scale's bottom (zero included); NaN: no copper
function lossLog(vals, lo) {
  const lg = new Float32Array(vals.length), floor = Math.log10(lo);
  for (let k = 0; k < vals.length; k++) { const v = vals[k]; lg[k] = v === v ? (v > lo ? Math.log10(v) : floor) : NaN; }
  return lg;
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
// The picture of the shown plane (pi-map.js) on a log scale. Total, parts and all copper: the common
// raster, cut once to the copper of the solved rails on the plane's layers and the parts' pad boxes. A
// layer: one image per rail on its own DC cells, each cut once to its rail's copper (so that its
// extension does not cover a neighbour), the rails' narrow tracks as strokes. `clip`: that copper as
// one path, for the exact edge when zoomed in (lossDrawClip).
function lossImage() {
  const { lut, alpha } = lossLut(), key = S.lossPlane + "|" + S.lossLut.key + "|" + S.colors.zone;
  if (S.lossImg && S.lossImg.key === key) return S.lossImg;
  const m = S.loss.map, pl = S.lossPlane, ll = lossLayerOf(pl), decades = lossDecades();
  const nets = [...S.piRes.entries()].filter(([, r]) => r.res).map(([n]) => S.scene.nets.indexOf(n)).filter((n) => n >= 0);
  const layers = pl === "comp" ? [] : ll ? [ll.layer] : m.layers, clip = new Path2D();
  for (const l of layers) clip.addPath(mapCopper(nets, l).clip);
  if (ll) {
    const max = ll.max, lo = max / 10 ** decades, sc = { lo: Math.log10(lo), hi: Math.log10(max), lut, alpha };
    // factors 4, lowered for the finest images first until all of them fit the budget
    const ps = ll.planes.filter((p) => p.n > 0).sort((a, b) => b.nx * b.ny - a.nx * a.ny), f = ps.map(() => S.mapUp || 4);
    for (;;) {
      if (ps.reduce((a, p, i) => a + p.nx * p.ny * f[i] * f[i], 0) <= LOSS_PX) break;
      let k = -1;
      ps.forEach((p, i) => { if (f[i] > 1 && (k < 0 || p.cell / f[i] < ps[k].cell / f[k])) k = i; });
      if (k < 0) break;
      f[k] /= 2;
    }
    const rails = [];
    ps.forEach((p, i) => {
      const map = max > 0 ? mapImage(lossLog(lossDense(p), lo), p.nx, p.ny, { x0: p.x0, y0: p.y0, cell: p.cell, ...sc, f: f[i] }) : null;
      const net = S.scene.nets.indexOf(p.net);
      if (!map || net < 0) return;
      const cu = mapCopper([net], ll.layer);
      lossUnder(map, cu.clip);
      mapBake(map, cu.clip);
      rails.push({ p, map, strokes: mapStrokes(cu, map) });
    });
    S.lossImg = { key, plane: pl, ll, rails, clip, max, lo, px: Math.max(0, ...rails.map((r) => r.map.cell / r.map.f)), bytes: rails.reduce((a, r) => a + r.map.bytes, 0) };
    return S.lossImg;
  }
  const vals = lossValues(pl);
  let max = 0;
  for (let i = 0; i < vals.length; i++) if (vals[i] > max) max = vals[i];
  const lo = max / 10 ** decades;
  const map = max > 0 ? mapImage(lossLog(vals, lo), m.nx, m.ny, { x0: m.x0, y0: m.y0, cell: m.cell, lo: Math.log10(lo), hi: Math.log10(max), lut, alpha }) : null;
  // parts: their heat is spread over the box of their pads (kisi_pi::losses), the box is part of the clip
  if (pl === "comp" || pl === "total") {
    const hot = new Set(S.loss.summary.components.filter((c) => c.w > 0).map((c) => c.refdes)), P = S.scene.pads, box = new Map();
    for (let i = 0; i < P.length / 8; i++) {
      const ref = S.scene.fp_refs[S.scene.pad_fp[i]];
      if (!hot.has(ref)) continue;
      const o = i * 8, r = Math.max(P[o + 2], P[o + 3]) / 2, b = box.get(ref) || [Infinity, Infinity, -Infinity, -Infinity];
      b[0] = Math.min(b[0], P[o] - r); b[1] = Math.min(b[1], P[o + 1] - r); b[2] = Math.max(b[2], P[o] + r); b[3] = Math.max(b[3], P[o + 1] + r);
      box.set(ref, b);
    }
    for (const b of box.values()) clip.rect(b[0], b[1], b[2] - b[0], b[3] - b[1]);
  }
  if (map) { lossUnder(map, clip); mapBake(map, clip); }
  S.lossImg = { key, plane: pl, map, clip, max, lo, vals, px: map ? map.cell / map.f : 0, bytes: map ? map.bytes : 0 };
  return S.lossImg;
}
// the board's drawing of the copper (the zone colour, as the board view draws zones) under the image's
// pixels, once at the image's resolution (a fill of all the copper each frame costs several frames)
function lossUnder(m, path) {
  const x = m.canvas.getContext("2d"), k = m.f / m.cell;
  x.setTransform(k, 0, 0, k, -m.x0 * k, -m.y0 * k);
  x.globalCompositeOperation = "destination-over"; x.fillStyle = S.colors.zone; x.fill(path);
  x.globalCompositeOperation = "source-over"; x.setTransform(1, 0, 0, 1, 0, 0);
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

// the copper as a clip each frame only when an image pixel covers more than 1.5 device pixels: then the
// edge cut into the images would show; otherwise the cut images alone (one clip of all the copper of a
// board costs several times the rest of the frame)
const lossDrawClip = (im) => S.view.s * (window.devicePixelRatio || 1) * im.px > 1.5 ? im.clip : null;
function drawLoss(px) {
  if (!S.loss || !S.loss.map) return;
  const im = lossImage(), clip = lossDrawClip(im);
  if (im.rails) mapDraw({ clip, image: (c) => { for (const r of im.rails) mapPaint(c, r.map); }, over: (c) => { for (const r of im.rails) mapDrawStrokes(c, r.strokes, px); } });
  else if (im.map) mapDraw({ clip, image: (c) => mapPaint(c, im.map) });
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
  const im = lossImage(), plane = lossPlanes().find((p) => p.id === S.lossPlane), d = lossDecades(), C = S.colors;
  // the scale over the board's copper drawing, as on the map
  const stops = Array.from({ length: 25 }, (_, k) => { const c = lossColor(k / 24, d); return `rgba(${c[0]},${c[1]},${c[2]},${(c[3] * MAP_ALPHA).toFixed(3)})`; }).join(",");
  const v = (x) => fmtSI(x, "W/mm²", 2);
  L.innerHTML = `<strong>Power density · ${esc(plane ? plane.name : "")} · log scale</strong>
    <div class="grad" style="background:linear-gradient(90deg,${stops}),linear-gradient(${C.zone},${C.zone}),${C.canvas}"></div>
    <div class="ticks units"><span>≤ ${v(im.lo)}</span><span>${v(im.lo * 10 ** (d / 2))}</span><span>${v(im.max)}</span></div>
    <div class="row"><span>Scale <select id="loss-dec" aria-label="decades of the log scale">${LOSS_DECADE_STEPS.map((n) => `<option value="${n}"${n === d ? " selected" : ""}>${n} decades</option>`).join("")}</select>
      below the largest value; over the lowest decade the colour fades into the board's copper (without current: plain copper)</span></div>
    <div class="row">${S.lossPlane === "comp" || S.lossPlane === "total" ? "Parts: heat over the area of their pads (estimate). " : ""}${S.lossPlane === "comp" ? "" : `Copper: I²R of every rail and the ground return per mm² of copper${S.lossPlane[0] === "L" ? ", on each rail's own cells" : ", the layers summed"}.`}</div>`;
  const sel = $("loss-dec");
  if (sel) sel.onchange = () => { S.lossDecades = +sel.value; lsSet("kisi.loss.decades", S.lossDecades); renderLossLegend(); draw(); };
}

function showLossTip(mx, my) {
  const tip = $("tip");
  if (!S.loss || !S.loss.map) { tip.hidden = true; return; }
  const m = S.loss.map, im = lossImage();
  const x = (mx - S.view.x) / S.view.s, y = (my - S.view.y) / S.view.s;
  const i = Math.floor((x - m.x0) / m.cell), j = Math.floor((y - m.y0) / m.cell);
  if (i < 0 || j < 0 || i >= m.nx || j >= m.ny) { tip.hidden = true; return; }
  const k = j * m.nx + i, per = m.nx * m.ny, b = S.loss.buf;
  const cu = b[k] === b[k] ? b[k] : 0, comp = b[per + k], at = lossAt(im, x, y);
  if (!(at.v > 0) && !(cu > 0) && !(comp > 0)) { tip.hidden = true; return; }
  const near = S.loss.summary.components.filter((c) => c.at).map((c) => ({ c, d: Math.hypot(c.at[0] - x, c.at[1] - y) })).sort((a, b2) => a.d - b2.d)[0];
  const d = (w) => w === w ? fmtSI(w, "W/mm²") : "–";
  tip.innerHTML = `<div class="t1">Power density</div><div class="kv">
    <span>Shown</span><span>${d(at.v)}</span>
    ${at.net ? `<span>Rail</span><span>${esc(netLabel(at.net))}</span>` : ""}
    <span>Parts</span><span>${d(comp)}</span>
    <span>Copper</span><span>${d(cu)} <span class="muted">all layers</span></span>
    ${near && near.d < 3 ? `<span>Part</span><span>${esc(near.c.refdes)} ${esc(near.c.part)} · ${fmtSI(near.c.w, "W")}</span>` : ""}</div>`;
  tip.hidden = false;
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
    <div class="tablewrap"><table class="xs-table ctab"><tbody>${layers || `<tr><td>no copper losses yet</td></tr>`}</tbody></table></div>
    <h4 class="sub">Copper per rail</h4>
    <div class="tablewrap"><table class="xs-table ctab"><tbody>${railRows || `<tr><td>–</td></tr>`}</tbody></table></div>
    <h4 class="sub">Hottest parts</h4>
    <div class="tablewrap"><table class="xs-table ctab"><tbody>${compRows || `<tr><td>no power tree</td></tr>`}</tbody></table></div>
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

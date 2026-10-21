"use strict";
// ---- losses: power density map ("Losses" view) and power flow (Sankey in the power tree)
// The numbers come from kisi_core::losses (S.loss = engine.piLosses): heat per part, copper
// loss per rail (incl. ground return) and layer, a raster in W/mm² and the flow links.

const LOSS_DECADES = 3; // colour scale: max … max/1000 (log)

function lossPlanes() {
  const m = S.loss && S.loss.map;
  if (!m) return [];
  const su = S.scene.stackup.copper;
  return [{ id: "total", name: "Total" }, { id: "comp", name: "Parts" }, { id: "cu", name: "Copper, all layers" }]
    .concat(m.layers.map((l, i) => ({ id: "L" + i, name: su[l] ? su[l].name : "layer " + l })));
}
// W/mm² of the selected plane, nx*ny cells
function lossValues(plane) {
  const m = S.loss.map, per = m.nx * m.ny, nl = m.layers.length, b = S.loss.buf, out = new Float32Array(per);
  const add = (k) => { for (let i = 0; i < per; i++) out[i] += b[k * per + i]; };
  if (plane === "comp") add(nl);
  else if (plane === "cu") for (let k = 0; k < nl; k++) add(k);
  else if (plane === "total") for (let k = 0; k <= nl; k++) add(k);
  else add(+plane.slice(1));
  return out;
}
function lossImage() {
  if (S.lossImg && S.lossImg.plane === S.lossPlane) return S.lossImg;
  const m = S.loss.map, vals = lossValues(S.lossPlane);
  let max = 0;
  for (let i = 0; i < vals.length; i++) if (vals[i] > max) max = vals[i];
  const lo = max / 10 ** LOSS_DECADES;
  const c = document.createElement("canvas"); c.width = m.nx; c.height = m.ny;
  const cx = c.getContext("2d"), img = cx.createImageData(m.nx, m.ny);
  for (let k = 0; k < vals.length; k++) {
    const v = vals[k], o = k * 4;
    if (!(v > lo) || max <= 0) { img.data[o + 3] = 0; continue; }
    const col = turboRGB(0.05 + 0.9 * Math.min(1, Math.log10(v / lo) / LOSS_DECADES));
    img.data[o] = col[0]; img.data[o + 1] = col[1]; img.data[o + 2] = col[2]; img.data[o + 3] = 235;
  }
  cx.putImageData(img, 0, 0);
  S.lossImg = { plane: S.lossPlane, canvas: c, max, lo, vals };
  return S.lossImg;
}

function drawLoss(px) {
  if (!S.loss || !S.loss.map) return;
  const m = S.loss.map, im = lossImage();
  ctx.imageSmoothingEnabled = false;
  ctx.drawImage(im.canvas, m.x0, m.y0, m.nx * m.cell, m.ny * m.cell);
  // the largest heat sources
  if (S.lossPlane === "total" || S.lossPlane === "comp") {
    const top = S.loss.summary.components.slice(0, 5);
    const cs = getComputedStyle(document.documentElement);
    ctx.font = `600 ${11 * px}px ${cs.getPropertyValue("--font-mono").trim() || "monospace"}`;
    ctx.fillStyle = cs.getPropertyValue("--fg").trim() || "#000"; ctx.strokeStyle = S.colors.canvas; ctx.lineWidth = 3 * px; ctx.lineJoin = "round";
    for (const c of top) {
      if (!c.at) continue;
      const t = `${c.refdes} ${mW(c.w)}`;
      ctx.strokeText(t, c.at[0] + 6 * px, c.at[1] - 6 * px); ctx.fillText(t, c.at[0] + 6 * px, c.at[1] - 6 * px);
    }
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
  const im = lossImage(), plane = lossPlanes().find((p) => p.id === S.lossPlane);
  const stops = Array.from({ length: 11 }, (_, k) => turbo(k / 10 * 0.9 + 0.05)).join(",");
  const v = (x) => { const m = x * 1e3; return m >= 100 ? fmt(m, 0) : m >= 1 ? fmt(m, 1) : m >= 0.01 ? fmt(m, 3) : m.toExponential(0); };
  L.innerHTML = `<strong>Power density · ${esc(plane ? plane.name : "")}</strong>
    <div class="grad" style="background:linear-gradient(90deg,${stops})"></div>
    <div class="ticks"><span>${v(im.lo)}</span><span>${v(im.lo * 10 ** (LOSS_DECADES / 2))}</span><span>${v(im.max)} mW/mm² (log)</span></div>
    <div class="row">${S.lossPlane === "comp" || S.lossPlane === "total" ? "Parts: heat over the area of their pads (estimate). " : ""}Copper: I²R of every rail and the ground return.</div>`;
}

function showLossTip(mx, my) {
  const tip = $("tip");
  if (!S.loss || !S.loss.map) { tip.hidden = true; return; }
  const m = S.loss.map, im = lossImage();
  const x = (mx - S.view.x) / S.view.s, y = (my - S.view.y) / S.view.s;
  const i = Math.floor((x - m.x0) / m.cell), j = Math.floor((y - m.y0) / m.cell);
  if (i < 0 || j < 0 || i >= m.nx || j >= m.ny) { tip.hidden = true; return; }
  const k = j * m.nx + i, per = m.nx * m.ny, nl = m.layers.length, b = S.loss.buf;
  let cu = 0;
  for (let l = 0; l < nl; l++) cu += b[l * per + k];
  const comp = b[nl * per + k];
  if (!(im.vals[k] > 0) && !(cu > 0) && !(comp > 0)) { tip.hidden = true; return; }
  const near = S.loss.summary.components.filter((c) => c.at).map((c) => ({ c, d: Math.hypot(c.at[0] - x, c.at[1] - y) })).sort((a, b2) => a.d - b2.d)[0];
  const d = (w) => `${fmt(w * 1e3, w * 1e3 >= 10 ? 1 : 3)} mW/mm²`;
  tip.innerHTML = `<div class="t1">Power density</div><div class="kv">
    <span>Shown</span><span>${d(im.vals[k])}</span>
    <span>Parts</span><span>${d(comp)}</span>
    <span>Copper</span><span>${d(cu)}</span>
    ${near && near.d < 3 ? `<span>Part</span><span>${esc(near.c.refdes)} ${esc(near.c.part)} · ${mW(near.c.w)}</span>` : ""}</div>`;
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
  const layers = s.layers.map(([l, name, w]) => `<tr><td>${esc(name || (su[l] && su[l].name) || l)}</td><td class="n">${fmt(w * 1e3, 2)}</td><td>${bar(w, maxL, "l")}</td></tr>`).join("");
  const rails = s.rails.filter((r) => r.w > 0).slice(0, 8);
  const maxR = Math.max(...rails.map((r) => r.w), 1e-12);
  const railRows = rails.map((r) => `<tr><td><button class="linkish" data-lrail="${esc(r.net)}">${esc(netShort(r.net))}</button>${r.ground ? " <span class=\"muted\">return</span>" : ""}</td><td class="n">${fmt(r.w * 1e3, 2)}</td><td>${bar(r.w, maxR, "l")}</td></tr>`).join("");
  const comps = s.components.slice(0, 10), maxC = Math.max(...comps.map((c) => c.w), 1e-12);
  const compRows = comps.map((c) => `<tr><td>${esc(c.refdes)}</td><td>${esc(c.part)}</td><td class="n">${fmt(c.w * 1e3, 0)}</td><td>${bar(c.w, maxC, c.role === "load" ? "u" : "l")}</td></tr>`).join("");
  box.innerHTML = `<header><div><h3>Losses · ${fmt(s.total_w, 2)} W</h3><div class="muted">where power turns into heat: parts and copper (rails and ground return)</div></div><button class="x" id="closePi" aria-label="Close">×</button></header>
    <div class="dstats"><span>Parts <b>${mW(s.components_w)}</b></span><span>Copper, rails <b>${fmt(s.copper_w * 1e3, 1)} mW</b></span><span>Copper, ground return <b>${fmt(s.ground_w * 1e3, 1)} mW</b></span></div>
    <h4 class="sub">Copper per layer <em>mW</em></h4>
    <div class="tablewrap"><table class="xs-table ctab"><tbody>${layers || `<tr><td>no copper losses yet</td></tr>`}</tbody></table></div>
    <h4 class="sub">Copper per rail <em>mW</em></h4>
    <div class="tablewrap"><table class="xs-table ctab"><tbody>${railRows || `<tr><td>–</td></tr>`}</tbody></table></div>
    <h4 class="sub">Hottest parts <em>mW</em></h4>
    <div class="tablewrap"><table class="xs-table ctab"><tbody>${compRows || `<tr><td>no power tree</td></tr>`}</tbody></table></div>
    <p class="note">${esc((s.notes || []).join(" "))} Shown is power (W, mW/mm²), not temperature.</p>`;
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
  const val = (w) => mW(w);
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

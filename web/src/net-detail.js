"use strict";
// ============================================================== tooltip & detail
function devChip(xs, net) {
  const t = classTarget(net);
  if (!t) return "";
  const z = comparedZ(xs, t);
  if (!z) return "";
  const d = (z.v - t.z) / t.z;
  const col = Math.abs(d) <= 0.05 ? S.colors.ok : (d < 0 ? (d < -0.1 ? S.colors.low2 : S.colors.low1) : (d > 0.1 ? S.colors.high2 : S.colors.high1));
  return `<div>Target ${fmt(t.z, 0)} Ω ${t.diff ? "diff" : "SE"}${z.diff || !t.diff ? "" : " (uncoupled: 2·Z₀)"} <span class="pill" style="background:${col}">${d >= 0 ? "+" : ""}${fmt(d * 100, 1)} %</span></div>`;
}
function xsRows(xs) {
  const L = S.scene.stackup.copper;
  const rows = [["Layer", `${L[xs.layer].name} · ${KIND_LABEL[xs.kind]}`], [xs.flags & 256 ? "Pad width" : "Width", `${fmt(xs.w * 1000, 0)} µm · Cu ${fmt(xs.t * 1000, 1)} µm`]];
  if (xs.ref_up != null) rows.push(["Ref. above", `${L[xs.ref_up].name} · h ${fmt(xs.h_up * 1000, 0)} µm · εr ${fmt(xs.er_up, 2)}`]);
  if (xs.ref_dn != null) rows.push(["Ref. below", `${L[xs.ref_dn].name} · h ${fmt(xs.h_dn * 1000, 0)} µm · εr ${fmt(xs.er_dn, 2)}`]);
  if (xs.s_left != null || xs.s_right != null) rows.push(["Side ground", `${xs.s_left != null ? fmt(xs.s_left * 1000, 0) + " µm" : "–"} / ${xs.s_right != null ? fmt(xs.s_right * 1000, 0) + " µm" : "–"}`]);
  if (xs.diff_gap != null) rows.push(["Pair gap", `${fmt(xs.diff_gap * 1000, 0)} µm`]);
  for (const [k, l, r] of xs.planes || []) {
    const isRef = k === xs.ref_up || k === xs.ref_dn;
    const side = (v) => v == null ? "–" : fmt(v, 2);
    rows.push([L[k].name, isRef ? `plane ends ${side(l)} / ${side(r)} mm from centre` : `cut-out, copper ${side(l)} / ${side(r)} mm from centre`]);
  }
  if (xs.mask) rows.push(["Solder mask", "yes"]);
  rows.push(["Z₀", xs.z0 != null ? `${fmt(xs.z0, 1)} Ω` + (xs.eeff ? ` · εeff ${fmt(xs.eeff, 2)}` : "") : "–"]);
  if (xs.zdiff != null) rows.push(["Z diff", `${fmt(xs.zdiff, 1)} Ω`]);
  if (xs.solver === "field") {
    const fz = xs.zdiff_formula != null ? xs.zdiff_formula : xs.z0_formula;
    rows.push(["Source", `field solver · formula ${fmt(fz, 1)} Ω`]);
  } else if (xs.kind !== "NoReference") {
    rows.push(["Source", xs.geom != null && S.fieldPending.has(S.an.geometries[xs.geom].id) ? "formula, field solver running" : "formula"]);
  }
  return rows;
}
function showTip(i, mx, my) {
  const tip = $("tip");
  if (i < 0) { tip.hidden = true; return; }
  const pc = S.pieces, xs = S.an.cross_sections[pc.xs[i]], net = pc.net[i];
  const cls = S.scene.netclasses[S.scene.net_class[net] ?? 0]?.name ?? "";
  const flags = FLAGS.filter(([b]) => xs.flags & b).map(([, t]) => `<div class="warn">${t}</div>`).join("");
  const isPad = S.padSet && S.padSet.has(pc.track[i]);
  tip.innerHTML = `<div class="t1">${esc(S.scene.nets[net] || "(no net)")}${isPad ? " · Pad" : ""}</div><div class="muted">${esc(cls)}</div>
    ${S.an.skeleton ? `<div class="kv"><span>Layer</span><span>${esc(S.scene.stackup.copper[pc.layer[i]].name)}</span><span>Width</span><span>${fmt(pc.w[i] * 1000, 0)} µm</span></div><div class="muted">impedance not estimated yet (step 2)</div>`
      : `<div class="kv">${xsRows(xs).map(([k, v]) => `<span>${k}</span><span>${v}</span>`).join("")}</div>${devChip(xs, net)}${flags}`}`;
  tip.hidden = false;
  const r = $("stage").getBoundingClientRect(), tw = tip.offsetWidth, th = tip.offsetHeight;
  let x = mx + 16, y = my + 16;
  if (x + tw > r.width - 8) x = mx - tw - 16;
  if (y + th > r.height - 8) y = my - th - 16;
  tip.style.left = Math.max(8, x) + "px"; tip.style.top = Math.max(8, y) + "px";
}

function netOrder(net) {
  // order pieces of a net along its path (DFS over endpoint graph)
  const tr = S.scene.tracks, pc = S.pieces;
  const tracks = new Map();
  for (let i = 0; i < pc.n; i++) if (pc.net[i] === net) { const t = pc.track[i]; if (!tracks.has(t)) tracks.set(t, []); tracks.get(t).push(i); }
  const key = (x, y) => Math.round(x * 200) + "," + Math.round(y * 200);
  const adj = new Map(), ends = new Map();
  for (const t of tracks.keys()) {
    const o = t * 7, a = key(tr[o], tr[o + 1]), b = key(tr[o + 2], tr[o + 3]);
    ends.set(t, [a, b]);
    for (const k of [a, b]) { if (!adj.has(k)) adj.set(k, []); adj.get(k).push(t); }
  }
  const seen = new Set(), order = [];
  const starts = [...adj.entries()].filter(([, v]) => v.length === 1).map(([k]) => k);
  const allKeys = starts.concat([...adj.keys()]);
  for (const s of allKeys) {
    const stack = [s];
    while (stack.length) {
      const k = stack.pop();
      for (const t of adj.get(k) || []) {
        if (seen.has(t)) continue;
        seen.add(t);
        const [a, b] = ends.get(t), fwd = a === k;
        order.push([t, fwd]);
        stack.push(fwd ? b : a);
      }
    }
  }
  const out = []; let s = 0;
  for (const [t, fwd] of order) {
    const ps = tracks.get(t).slice().sort((p, q) => pc.t0[p] - pc.t0[q]);
    if (!fwd) ps.reverse();
    for (const i of ps) {
      const L = Math.hypot(pc.bx[i] - pc.ax[i], pc.by[i] - pc.ay[i]);
      out.push({ i, s0: s, s1: s + L }); s += L;
    }
  }
  return out;
}

function selectNet(net, zoom = true) {
  S.selNet = net;
  renderFieldStatus();
  document.querySelectorAll(".net").forEach((b) => b.setAttribute("aria-current", String(+b.dataset.net === net)));
  if (net < 0) { S.selPaths = null; $("detail").hidden = true; $("ch-detail").hidden = true; draw(); return; }
  buildPaths(net);
  const st = S.an.nets.find((x) => x.net === net);
  const t = classTarget(net), pc = S.pieces;
  // bbox
  let bb = [Infinity, Infinity, -Infinity, -Infinity];
  const xsLen = new Map();
  for (let i = 0; i < pc.n; i++) if (pc.net[i] === net) {
    bb = [Math.min(bb[0], pc.ax[i], pc.bx[i]), Math.min(bb[1], pc.ay[i], pc.by[i]), Math.max(bb[2], pc.ax[i], pc.bx[i]), Math.max(bb[3], pc.ay[i], pc.by[i])];
    xsLen.set(pc.xs[i], (xsLen.get(pc.xs[i]) || 0) + Math.hypot(pc.bx[i] - pc.ax[i], pc.by[i] - pc.ay[i]));
  }
  const partner = st && st.partner != null ? S.scene.nets[st.partner] : null;
  const rowsXs = [...xsLen.entries()].sort((a, b) => b[1] - a[1]).slice(0, 6).map(([xi, l]) => {
    const xs = S.an.cross_sections[xi], L = S.scene.stackup.copper;
    const z = comparedZ(xs, t);
    return `<tr><td>${fmt(l, 1)}</td><td>${L[xs.layer].name}</td><td>${KIND_LABEL[xs.kind].split(",")[0]}</td><td>${fmt(xs.w * 1000, 0)}</td><td>${xs.diff_gap != null ? fmt(xs.diff_gap * 1000, 0) : "–"}</td><td>${fmt(xs.z0, 1)}</td><td>${fmt(xs.zdiff, 1)}</td><td>${z ? fmt(z.v, 1) : "–"}</td></tr>`;
  }).join("");
  const d = $("detail");
  d.innerHTML = `<header><div><h3>${esc(S.scene.nets[net])}</h3><div class="muted">${esc(st?.class ?? "")}${partner ? " · pair with " + esc(partner) : ""}</div></div><button class="x" id="closeDetail" aria-label="Clear selection">×</button></header>
    <div class="dstats"><span>Length <b>${fmt(st?.length, 1)} mm</b></span><span>Target <b>${t ? fmt(t.z, 0) + " Ω " + (t.diff ? "diff" : "SE") : "–"}</b></span>
    <span>min/mean/max <b>${fmt(st?.z_min, 1)} / ${fmt(st?.z_mean, 1)} / ${fmt(st?.z_max, 1)} Ω</b></span><span>±5 % <b>${pct(st?.within5)}</b></span></div>
    <div class="tabs" role="tablist" aria-label="Net view">
      ${[["z", "Impedance"], ["loss", "Loss"], ["tdr", "TDR"], ["via", "Vias"], ["xt", "Crosstalk"]].map(([k, n]) => `<button role="tab" data-tab="${k}" aria-selected="${S.detailTab === k}">${n}</button>`).join("")}
    </div>
    <div id="tab-body"></div>`;
  d.hidden = false;
  $("closeDetail").onclick = () => selectNet(-1);
  d.querySelectorAll("[data-tab]").forEach((b) => b.onclick = () => {
    S.detailTab = b.dataset.tab;
    d.querySelectorAll("[data-tab]").forEach((x) => x.setAttribute("aria-selected", String(x === b)));
    renderTab(net, t, rowsXs);
  });
  if (S.detailTab === "eye") S.detailTab = "z";
  if (S.tab === "ch") { d.hidden = true; renderChDetail(net); } else { $("ch-detail").hidden = true; renderTab(net, t, rowsXs); }
  if (zoom && isFinite(bb[0])) {
    const m = 1.5;
    fitView([bb[0] - m, bb[1] - m, bb[2] + m, bb[3] + m]);
  } else draw();
}

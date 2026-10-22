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
    ${S.an.skeleton ? `<div class="kv"><span>Layer</span><span>${esc(S.scene.stackup.copper[pc.layer[i]].name)}</span><span>Width</span><span>${fmt(pc.w[i] * 1000, 0)} µm</span><span>Type</span><span>${xs.kind === "Stripline" ? "stripline" : "microstrip"} (by layer)</span></div><div class="muted">impedance not estimated yet (step 2)</div>`
      : `<div class="kv">${xsRows(xs).map(([k, v]) => `<span>${k}</span><span>${v}</span>`).join("")}</div>${devChip(xs, net)}${flags}`}`;
  tip.hidden = false;
  const r = $("stage").getBoundingClientRect(), tw = tip.offsetWidth, th = tip.offsetHeight;
  let x = mx + 16, y = my + 16;
  if (x + tw > r.width - 8) x = mx - tw - 16;
  if (y + th > r.height - 8) y = my - th - 16;
  tip.style.left = Math.max(8, x) + "px"; tip.style.top = Math.max(8, y) + "px";
}

// ---- path of a net: one direction for the impedance profile, the TDR and the board marks
// (kisi_core::rf::net_path_info): start at the lane's transmitter side or the IC, the user can
// swap it per net (stored per board)
const flipKey = () => "kisi.pathflip.v1." + S.name;
function pathFlips() { try { return JSON.parse(localStorage.getItem(flipKey()) || "{}"); } catch (e) { return {}; } }
function pathFlipped(net) { return !!pathFlips()[S.scene.nets[net]]; }
function setPathFlip(net, v) {
  const f = pathFlips();
  if (v) f[S.scene.nets[net]] = true; else delete f[S.scene.nets[net]];
  try { localStorage.setItem(flipKey(), JSON.stringify(f)); } catch (e) { /* storage blocked */ }
}
// transmitter side of the lane the net belongs to (a point near the start)
function pathHint(net) {
  const l = typeof laneOfNet === "function" ? laneOfNet(net) : null;
  if (!l) return null;
  const rev = laneReversed(l);
  const h = l.hops.concat(l.partner_hops || []).find((x) => x.type === "net" && x.net === net);
  return h ? (rev ? h.to : h.from) : null;
}
function pathReq(net) { return { net, flip: pathFlipped(net), hint: pathHint(net) }; }
function netPath(net) {
  const q = pathReq(net), key = `${S.boardKey}|${S.anGen}|${JSON.stringify(q)}`;
  if (!S.pathCache || S.pathCache.board !== S.boardKey) S.pathCache = { board: S.boardKey, m: new Map() };
  if (S.pathCache.m.has(key)) return S.pathCache.m.get(key);
  let p = null;
  try { p = S.engine.netPath(q); } catch (e) { p = null; }
  S.pathCache.m.set(key, p);
  return p;
}
// board position at `s` mm along the path (or a branch: list of pieces with s0/s1)
function pathPoint(pieces, s) {
  if (!pieces || !pieces.length) return null;
  const pc = S.pieces;
  let q = pieces.find((x) => s >= x.s0 && s <= x.s1);
  if (!q) q = s < pieces[0].s0 ? pieces[0] : pieces[pieces.length - 1];
  const f = q.s1 > q.s0 ? Math.min(1, Math.max(0, (s - q.s0) / (q.s1 - q.s0))) : 0, i = q.piece;
  const [x0, y0, x1, y1] = q.rev ? [pc.bx[i], pc.by[i], pc.ax[i], pc.ay[i]] : [pc.ax[i], pc.ay[i], pc.bx[i], pc.by[i]];
  return { x: x0 + (x1 - x0) * f, y: y0 + (y1 - y0) * f };
}
function setPathCursor(net, s) {
  const p = s == null ? null : netPath(net);
  S.pathCursor = p ? pathPoint(p.pieces, Math.min(s, p.length_mm)) : null;
  draw();
}
const branchLabel = (b) => `⑂ ${fmt(b.len_mm, 1)} mm → ${b.end.label}`;
function pathMarks(p) { return (p ? p.branches : []).map((b) => ({ x: b.at_s, label: branchLabel(b) })); }
// start A, end B and branch junctions on the board, the chart cursor as a dot
function drawPathMarks(px) {
  if (S.tab !== "si" || S.selNet < 0 || $("detail").hidden) return;
  const p = netPath(S.selNet);
  if (!p) return;
  const cs = getComputedStyle(document.documentElement), acc = cs.getPropertyValue("--accent").trim(), panel = cs.getPropertyValue("--panel").trim();
  ctx.save();
  for (const b of p.branches) {
    const s = 4 * px;
    ctx.fillStyle = acc; ctx.strokeStyle = panel; ctx.lineWidth = 1.5 * px;
    ctx.beginPath(); ctx.moveTo(b.at[0], b.at[1] - s); ctx.lineTo(b.at[0] + s, b.at[1]); ctx.lineTo(b.at[0], b.at[1] + s); ctx.lineTo(b.at[0] - s, b.at[1]); ctx.closePath(); ctx.fill(); ctx.stroke();
  }
  for (const [e, letter] of [[p.start, "A"], [p.end, "B"]]) {
    const r = 7 * px;
    ctx.fillStyle = acc; ctx.strokeStyle = panel; ctx.lineWidth = 2 * px;
    ctx.beginPath(); ctx.arc(e.at[0], e.at[1], r, 0, Math.PI * 2); ctx.fill(); ctx.stroke();
    ctx.fillStyle = panel; ctx.font = `600 ${9 * px}px ${cs.getPropertyValue("--font-mono")}`; ctx.textAlign = "center"; ctx.textBaseline = "middle";
    ctx.fillText(letter, e.at[0], e.at[1] + 0.5 * px);
  }
  if (S.pathCursor) {
    ctx.fillStyle = cs.getPropertyValue("--fg").trim(); ctx.strokeStyle = panel; ctx.lineWidth = 2 * px;
    ctx.beginPath(); ctx.arc(S.pathCursor.x, S.pathCursor.y, 4.5 * px, 0, Math.PI * 2); ctx.fill(); ctx.stroke();
  }
  ctx.restore();
}

function pathLine(net) {
  const p = netPath(net);
  if (!p) return "";
  const nb = p.branches.length;
  return `<div class="pathline"><span class="pend">A</span><b>${esc(p.start.label)}</b><span class="muted">→</span><span class="pend">B</span><b>${esc(p.end.label)}</b>
    <span class="muted">${esc(p.flipped ? "swapped by you" : p.direction)}${nb ? ` · ${nb} branch${nb > 1 ? "es" : ""}` : ""}${p.parts > 1 ? ` · ${p.parts} copper parts, the longest path is shown` : ""}</span>
    <button class="btn small" id="path-flip" title="Swap start and end">⇄</button></div>`;
}

function selectNet(net, zoom = true) {
  S.selNet = net; S.pathCursor = null; S.stubPaths = null;
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
  // a differential pair: one mode switch for all tabs (differential first, common mode on request)
  const pair = !!(partner && t && t.diff);
  const comm = pair && S.pairMode === "c";
  const rowsXs = [...xsLen.entries()].sort((a, b) => b[1] - a[1]).slice(0, 6).map(([xi, l]) => {
    const xs = S.an.cross_sections[xi], L = S.scene.stackup.copper;
    const z = comm ? (xs.zeven != null ? { v: xs.zeven / 2 } : null) : comparedZ(xs, t);
    return `<tr><td>${fmt(l, 1)}</td><td>${L[xs.layer].name}</td><td>${KIND_LABEL[xs.kind].split(",")[0]}</td><td>${fmt(xs.w * 1000, 0)}</td><td>${xs.diff_gap != null ? fmt(xs.diff_gap * 1000, 0) : "–"}</td><td>${fmt(xs.z0, 1)}</td><td>${fmt(comm ? (xs.zeven != null ? xs.zeven / 2 : null) : xs.zdiff, 1)}</td><td>${z ? fmt(z.v, 1) : "–"}</td></tr>`;
  }).join("");
  const tabs = [["z", "Impedance"], ["loss", "Loss"], ["tdr", "TDR"]].concat(pair ? [["skew", "Skew"]] : [], [["via", "Vias"], ["xt", "Crosstalk"]]);
  if (!pair && S.detailTab === "skew") S.detailTab = "z";
  const d = $("detail");
  const modeSw = pair ? `<div class="seg pmode" role="group" aria-label="Mode of the pair"><button data-pm="d" aria-pressed="${!comm}">Differential</button><button data-pm="c" aria-pressed="${comm}">Common mode</button></div>` : "";
  d.innerHTML = `<header><div><h3>${esc(S.scene.nets[net])}</h3><div class="muted">${esc(st?.class ?? "")}${partner ? " · pair with " + esc(partner) : ""}</div></div>${modeSw}<button class="x" id="closeDetail" aria-label="Clear selection">×</button></header>
    <div class="dstats"><span>Length <b>${fmt(st?.length, 1)} mm</b></span><span>Target <b>${t ? fmt(t.z, 0) + " Ω " + (t.diff ? "diff" : "SE") : "–"}</b></span>
    <span>min/mean/max <b>${fmt(st?.z_min, 1)} / ${fmt(st?.z_mean, 1)} / ${fmt(st?.z_max, 1)} Ω</b></span><span>±5 % <b>${pct(st?.within5)}</b></span></div>
    ${pathLine(net)}
    <div class="tabs" role="tablist" aria-label="Net view">
      ${tabs.map(([k, n]) => `<button role="tab" data-tab="${k}" aria-selected="${S.detailTab === k}">${n}</button>`).join("")}
    </div>
    <div id="tab-body"></div>`;
  d.hidden = false;
  $("closeDetail").onclick = () => selectNet(-1);
  d.querySelectorAll("[data-pm]").forEach((b) => b.onclick = () => { S.pairMode = b.dataset.pm; selectNet(net, false); });
  const pf = $("path-flip");
  if (pf) pf.onclick = () => { setPathFlip(net, !pathFlipped(net)); S.rfCache = null; S.pathCursor = null; selectNet(net, false); };
  d.querySelectorAll("[data-tab]").forEach((b) => b.onclick = () => {
    S.detailTab = b.dataset.tab;
    d.querySelectorAll("[data-tab]").forEach((x) => x.setAttribute("aria-selected", String(x === b)));
    renderTab(net, t, rowsXs);
    draw();
  });
  if (S.detailTab === "eye") S.detailTab = "z";
  if (S.tab === "ch") { d.hidden = true; renderChDetail(net); } else { $("ch-detail").hidden = true; renderTab(net, t, rowsXs); }
  if (zoom && isFinite(bb[0])) {
    const m = 1.5;
    fitView([bb[0] - m, bb[1] - m, bb[2] + m, bb[3] + m]);
  } else draw();
}

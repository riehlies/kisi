"use strict";
// ---- AC impedance of a rail (PDN) -----------------------------------------------------
// nominal value, rated voltage and dielectric with where they came from (the core's cap_rating: PI_CAP, a field
// of the part, the value text; an assumed rating is an estimate)
function capRatingTitle(c) {
  const from = (f) => f === "value" ? "value text" : f || "";
  const rated = c.rated_v ? `, rated ${fmtSI(c.rated_v, "V")} (${c.rated_assumed ? "assumed – estimate" : from(c.rated_from)})` : "";
  const diel = c.dielectric ? `, ${c.dielectric}${c.dielectric_from ? ` (${from(c.dielectric_from)})` : ""}` : "";
  return `nominal ${c.value}${rated}${diel}`;
}

function renderPdn(body, setup) {
  let r;
  r = S.pdnRes && S.pdnRes.get(setup.net);
  if (!r) { try { r = S.engine.pdn({ setup, params: {} }); S.pdnRes.set(setup.net, r); } catch (e) { r = { error: e.message }; } }
  if (r.error) { body.innerHTML = `<p class="err">${esc(r.error)}</p>`; return; }
  const groups = new Map();
  for (const c of r.caps) {
    const k = `${c.c.toExponential(3)}|${c.package}`;
    const g = groups.get(k) || { c: c.c, pkg: c.package, n: 0, l: 0, srf: 0, refs: [] };
    g.n++; g.l += c.esl + c.l_mount + c.l_spread; g.srf += c.srf; g.refs.push(c.refdes); groups.set(k, g);
  }
  const rows = [...groups.values()].map((g) => `<tr title="${esc(g.refs.join(", "))}"><td>${g.n} ×</td><td>${capVal(g.c)}</td><td>${g.pkg}</td><td class="num">${fmtSI(g.l / g.n, "H")}</td><td class="num">${fHz(g.srf / g.n)}</td></tr>`).join("");
  const over = r.above_target.map(([a, b]) => `${fHz(a)} … ${fHz(b)}`).join(", ");
  body.innerHTML = `<div class="dstats"><span>Target <b>${r.target_ohm != null ? ohm(r.target_ohm) : "–"}</b>${r.target_ohm != null ? ` (3 % ripple at ${fmtSI(r.load_current * 0.5, "A")} load step)` : ""}</span><span>C total <b>${capVal(r.c_total)}</b></span><span>plane ${capVal(r.c_plane)}</span>${r.cavity_hz ? `<span>1st cavity resonance ≈ ${fHz(r.cavity_hz)}</span>` : ""}</div>
    <div class="clegend"><span><i style="background:var(--pw-use)"></i>|Z| lumped</span>${r.cav_z.length ? `<span><i style="background:var(--pw-loss)"></i>plane model (${r.cav_cells} cells)</span>` : ""}<span><i style="background:var(--muted)"></i>regulator and plane only</span>${r.target_ohm != null ? `<span><i style="background:${S.colors.high2}"></i>target</span>` : ""}</div>
    <div class="chart"><canvas id="ch-pdn" style="height:200px" aria-label="Rail impedance over frequency"></canvas></div>
    ${r.target_ohm != null ? `<div class="dstats"><span>${over ? `<b class="warn">above target: ${over}</b>` : `<b>below target up to ${fHz(r.f_eval)}</b>`}</span></div>` : ""}
    ${rows ? `<div class="tablewrap"><table class="xs-table"><thead><tr><th>Count</th><th>Value</th><th>Package</th><th class="num">L total</th><th class="num">Resonance</th></tr></thead><tbody>${rows}</tbody></table></div>` : ""}
    <p class="note">Lumped model: regulator (2 mΩ + 20 nH) ∥ plane capacitance ∥ capacitors with ESR/ESL by package (estimated), mounting inductance to the plane and spreading to the load centroid. ${r.cav_z.length ? ` Plane model from 1 MHz: main plane as a grid (${fmt(r.cav_h_mm, 2)} mm cells) over ideal ground; shows the shape and resonances of the plane, but copper on other layers and traces are not included. Above 1 MHz the traffic light uses the plane model.` : ""} Evaluated up to ${fHz(r.f_eval)} – above that, package and die capacitance take over.</p>
    ${r.warnings.map((w) => `<div class="warn">${esc(w)}</div>`).join("")}
    <div id="pdn-ic"></div>`;
  drawPdn($("ch-pdn"), r);
  renderPdnIc($("pdn-ic"), setup, r);
}

// ---- PDN seen from every IC (AC grid over all layers) -------------------------------------
const capVal = (c) => fmtSI(c, "F");
function pdnIcOf(net) { return S.pdnIc && S.pdnIc.get(net); }
function renderPdnIc(host, setup, lumped) {
  if (!S.pdnIc) S.pdnIc = new Map();
  const r = pdnIcOf(setup.net);
  if (!r) {
    host.innerHTML = `<h4 class="sub">Seen from the ICs</h4><p class="note">AC grid of the rail copper on all layers with vias, together with the nets behind series resistors, inductors and ferrite beads up to the regulators; every load IC as a port: Z(f) at each IC, the loop inductance of every capacitor to it, a target from the IC's own current step and recommendations. Takes a few seconds.</p><button class="btn small primary" id="pdn-ic-run">Compute per IC</button>`;
    $("pdn-ic-run").onclick = () => {
      $("pdn-ic-run").disabled = true; $("pdn-ic-run").textContent = "Computing …";
      setTimeout(() => {
        let res;
        const setups = S.piSetups ? [...S.piSetups.values()] : [];
        try { res = S.engine.pdnIc({ setup, setups, params: {} }); } catch (e) { res = { error: e.message }; }
        S.pdnIc.set(setup.net, res);
        S.pdnIcSel = res.ports && res.ports.length ? res.ports[0].label : null;
        renderPdnIc(host, setup, lumped); draw();
      }, 30);
    };
    return;
  }
  if (r.error) { host.innerHTML = `<h4 class="sub">Seen from the ICs</h4><p class="err">${esc(r.error)}</p>`; return; }
  let pt = r.ports.find((x) => x.label === S.pdnIcSel) || r.ports[0];
  S.pdnIcSel = pt.label;
  const own = pt.net === r.net, dn = (n) => r.domain.find((d) => d.net === n) || { voltage: r.voltage };
  const chips = r.ports.map((x) => `<button class="chip" data-ic="${esc(x.label)}" aria-pressed="${x.label === pt.label}" title="${esc(x.net)}"><i style="background:${x.above.length ? S.colors.high2 : S.colors.ok}"></i>${esc(x.label)}${x.net === r.net ? "" : ` <span class="muted">· ${esc(netShort(x.net))}</span>`}</button>`).join("");
  // the rail domain: nets behind series parts, up to the regulators
  const ser = r.series.map((x) => `<tr title="${esc(x.from)} → ${esc(x.to)}"><td>${esc(x.refdes)}</td><td>${esc(x.kind)}${x.value && x.value.toUpperCase() !== "FERRITE" ? ` <span class="muted">${esc(x.value)}</span>` : ""}</td><td>${esc(netShort(x.from))} → ${esc(netShort(x.to))}</td><td>${x.kind === "bead" ? `${ohm(x.rp)} ∥ ${fmtSI(x.lp, "H")}` : x.kind === "inductor" ? fmtSI(x.l, "H") : ohm(x.r)}</td><td>${ohm(x.r)}</td><td>${ohm(x.z100)}</td><td>${x.source === "estimate" ? "estimate" : esc(x.source)}</td></tr>`).join("");
  const domain = r.domain.length > 1 || r.boundary.length ? `<details class="acc"><summary>Rail domain: ${r.domain.length} net${r.domain.length === 1 ? "" : "s"}, ${r.series.length} series part${r.series.length === 1 ? "" : "s"}, regulator ${esc(r.regulators.join(", ") || "none")}</summary>
      <div class="tablewrap"><table class="xs-table"><thead><tr><th>Net</th><th>V</th><th>C</th><th>ICs</th><th>Regulator</th></tr></thead><tbody>${r.domain.map((d) => `<tr title="${esc(d.net)}"><td>${esc(netShort(d.net))}</td><td>${fmtSI(d.voltage, "V")}</td><td>${d.caps ? `${d.caps} · ${capVal(d.c_total)}` : "–"}</td><td>${esc(d.ics.join(", ") || "–")}</td><td>${esc(d.regulators.join(", ") || "")}</td></tr>`).join("")}</tbody></table></div>
      ${ser ? `<div class="tablewrap"><table class="xs-table"><thead><tr><th>Part</th><th>Kind</th><th>Between</th><th>Model</th><th>R dc</th><th>|Z| 100 MHz</th><th>Source</th></tr></thead><tbody>${ser}</tbody></table></div>` : ""}
      ${r.boundary.map((b) => `<p class="note">Ends: ${esc(b)}</p>`).join("")}
      <p class="note">The domain follows resistors up to 10 Ω, inductors and ferrite beads from this rail and stops at the regulators (their output modelled as 2 mΩ + 20 nH, estimate). Ferrites without an impedance rating in the value (e.g. 600R@100MHz) use a typical 220 Ω at 100 MHz (estimate): add a PI_SERIES field (z100, rdc) from the datasheet.</p></details>` : "";
  const over = pt.above.map(([a, b]) => `${fHz(a)} … ${fHz(b)}`).join(", ");
  const rows = pt.caps.slice(0, 30).map((l) => {
    const c = r.caps[l.cap];
    const fl = c.flags.length ? ` <span class="warn" title="${esc(c.flags.join("; "))}">!</span>` : "";
    return `<tr class="click" data-cap="${l.cap}"><td>${esc(l.refdes)}${fl}</td><td title="${esc(capRatingTitle(c))}">${capVal(c.c_eff)}${c.c_eff < 0.95 * c.c_nom ? ` <span class="muted">(${fmt(100 * c.c_eff / c.c_nom, 0)} %)</span>` : ""}</td><td>${c.package}</td><td class="num">${fmtSI(l.l_path, "H")}</td><td class="num">${fmtSI(l.l_total, "H")}</td><td class="num">${fHz(l.srf_hz)}</td><td class="num">${fmt(l.dist_mm, 1)}</td></tr>`;
  }).join("");
  const recs = pt.recommendations.map((x) => x.count
    ? `<li><b>${x.count} × ${esc(x.value)} ${esc(x.package)}</b> ${x.radius_mm >= 49.9 ? "anywhere on the rail" : `within <b>${fmt(x.radius_mm, 1)} mm</b> of ${esc(pt.refdes)}`} <span class="muted">(${fHz(x.f_lo)} … ${fHz(x.f_hi)}; ${esc(x.note)})</span></li>`
    : `<li class="warn">${fHz(x.f_lo)} … ${fHz(x.f_hi)}: ${esc(x.note)}</li>`).join("");
  const peaks = pt.peaks.map((k) => `<li${k.above_target ? ' class="warn"' : ""}>${fHz(k.f)}: ${ohm(k.z)} – ${esc(k.note)}</li>`).join("");
  const flagged = r.caps.filter((c) => c.flags.length).map((c) => `<li class="click" data-cap="${r.caps.indexOf(c)}"><b>${esc(c.refdes)}</b> ${esc(c.value)}: ${esc(c.flags.join("; "))}</li>`).join("");
  host.innerHTML = `<h4 class="sub">Seen from the ICs</h4>
    <div class="chips">${chips}</div>
    ${domain}
    <div class="dstats"><span>${esc(pt.refdes)} · ${fmtSI(pt.current, "A")} · ${pt.pins} pin${pt.pins === 1 ? "" : "s"}${own ? "" : ` on ${esc(netShort(pt.net))}`}</span><span>Target <b>${pt.target != null ? ohm(pt.target) : "–"}</b> <span class="muted">${esc(pt.target_source)}</span></span></div>
    <div class="clegend"><span><i style="background:var(--pw-use)"></i>|Z| at ${esc(pt.refdes)} (grid)</span>${own ? `<span><i class="dash" style="border-color:var(--muted)"></i>lumped (rail)</span>` : ""}${pt.target != null ? `<span><i style="background:${S.colors.high2}"></i>target</span>` : ""}<span>▼ anti-resonance</span></div>
    <div class="chart"><canvas id="ch-pdn-ic" style="height:200px" aria-label="Impedance at the IC over frequency"></canvas></div>
    ${pt.target != null ? `<div class="dstats"><span>${over ? `<b class="warn">above target: ${over}</b>` : `<b>below target up to ${fHz(pt.f_max)}</b>`}</span></div>` : ""}
    ${recs ? `<p class="note"><b>Recommendations</b> (dashed circles on the board):</p><ul class="plain">${recs}</ul>` : ""}
    ${peaks ? `<p class="note"><b>Anti-resonances</b></p><ul class="plain">${peaks}</ul>` : ""}
    <div class="tablewrap"><table class="xs-table"><thead><tr><th>Capacitor</th><th>C at ${fmtSI(Math.abs(dn(pt.net).voltage), "V")}</th><th>Package</th><th class="num" title="loop inductance from the IC through the planes and vias to the capacitor pad">L path</th><th class="num" title="L path + ground-side mounting + ESL">L total</th><th class="num">Resonance</th><th class="num">mm</th></tr></thead><tbody>${rows}</tbody></table></div>
    ${flagged ? `<p class="note"><b>Flags</b></p><ul class="plain">${flagged}</ul>` : ""}
    <p class="note">AC grid (${fmt(r.cell_mm, 2)} mm cells, ${r.nodes.toLocaleString("en-US")} nodes, ${r.layers.length} layers) of the rail domain copper against ground as ideal reference: plane capacitance and μ0·d per square, vias with their barrel inductance, the IC's rail pins shorted as one port. Capacitors with ESR, ESL and ground-side mounting by package and DC-bias derating (estimates unless a PI_CAP field gives them; the rated voltage also from the parts' rating fields or value – hover a capacitor for its source); regulator ${esc(r.regulators.join(", ") || "none")} as 2 mΩ + 20 nH (estimate). Targets from the IC's current step unless PI_TARGET. Recommendations add parts next to the IC until the target holds, then widen the radius as far as the result allows (estimate).</p>
    ${r.warnings.map((w) => `<div class="warn">${esc(w)}</div>`).join("")}`;
  host.querySelectorAll("[data-ic]").forEach((b) => b.onclick = () => { S.pdnIcSel = b.dataset.ic; renderPdnIc(host, setup, lumped); draw(); });
  const keepOpen = host.querySelector("details.acc");
  if (keepOpen) { keepOpen.open = !!S.pdnDomOpen; keepOpen.ontoggle = () => { S.pdnDomOpen = keepOpen.open; }; }
  host.querySelectorAll("[data-cap]").forEach((tr) => {
    tr.onclick = () => { const c = r.caps[+tr.dataset.cap]; S.pdnIcCap = +tr.dataset.cap; zoomTo(c.at[0], c.at[1], 6); };
    tr.onpointerenter = () => { S.pdnIcHover = +tr.dataset.cap; draw(); };
    tr.onpointerleave = () => { S.pdnIcHover = null; draw(); };
  });
  renderLegend();
  drawZf($("ch-pdn-ic"), { freq: r.freq, series: [{ z: pt.z, color: "--pw-use", name: pt.label }].concat(own ? [{ f: lumped.freq, z: lumped.z, color: "--muted", dash: [4, 3], name: "lumped" }] : []),
    target: pt.target, fEval: pt.f_max, marks: pt.peaks.map((k) => ({ f: k.f, label: "anti-resonance" })) });
}
// ---- outlined parts on the board (PDN per IC, jumps from the power flow) ------------------
// footprint outline from the scene's pads: [x0, y0, x1, y1]
function fpBBox(refdes) {
  const sc = S.scene, pads = sc.pads;
  let b = [Infinity, Infinity, -Infinity, -Infinity];
  for (let i = 0; i < pads.length / 8; i++) {
    if (sc.fp_refs[sc.pad_fp[i]] !== refdes) continue;
    const x = pads[i * 8], y = pads[i * 8 + 1], h = Math.max(pads[i * 8 + 2], pads[i * 8 + 3]) / 2;
    b = [Math.min(b[0], x - h), Math.min(b[1], y - h), Math.max(b[2], x + h), Math.max(b[3], y + h)];
  }
  return isFinite(b[0]) ? b : null;
}
// outline: at least 16 px on screen, halo, colour, light fill; the label is queued in `labels`
// (drawBoardLabels items: just above the outline's left edge, in the part's colour)
function partBox(b, col, lw, label, opt, labels, px) {
  if (!b) return;
  const m = Math.max(0.25, 4 * px), min = 16 * px;
  let [x0, y0, x1, y1] = [b[0] - m, b[1] - m, b[2] + m, b[3] + m];
  if (x1 - x0 < min) { const c = (x0 + x1) / 2; x0 = c - min / 2; x1 = c + min / 2; }
  if (y1 - y0 < min) { const c = (y0 + y1) / 2; y0 = c - min / 2; y1 = c + min / 2; }
  ctx.beginPath(); ctx.roundRect(x0, y0, x1 - x0, y1 - y0, Math.min(3 * px, (x1 - x0) / 4));
  if (opt.fill) { ctx.globalAlpha = opt.fill; ctx.fillStyle = col; ctx.fill(); ctx.globalAlpha = 1; }
  ctx.lineWidth = lw + 3 * px; ctx.strokeStyle = S.colors.halo; ctx.stroke();
  ctx.lineWidth = lw; ctx.strokeStyle = col; ctx.stroke();
  if (opt.ring) {
    const g = 5 * px;
    ctx.beginPath(); ctx.roundRect(x0 - g, y0 - g, x1 - x0 + 2 * g, y1 - y0 + 2 * g, 4 * px);
    ctx.setLineDash([4 * px, 3 * px]); ctx.lineWidth = 1.5 * px; ctx.strokeStyle = col; ctx.stroke(); ctx.setLineDash([]);
  }
  if (label) labels.push({ x: x0, y: y0 - (opt.ring ? 5 : 0) * px, text: label, color: col, bold: !!opt.bold, dx: 0, dy: -4 });
}
// the queued labels at a constant screen size (shared helper): the bold ones (the picked part, the
// IC, the selected capacitor) first, so a label that would overlap one of them is left out
function drawPartLabels(labels) {
  return drawBoardLabels(labels.slice().sort((a, b) => (b.bold ? 1 : 0) - (a.bold ? 1 : 0)));
}
// a part picked in the power flow
function drawPiMark(px, labels) {
  const m = S.piMark;
  if (!m || (m.rail && m.rail !== S.piRail)) return;
  const acc = getComputedStyle(document.documentElement).getPropertyValue("--accent").trim();
  partBox(m.bbox, acc, 3 * px, m.refdes, { fill: 0.15, bold: true, ring: true }, labels, px);
}

// recommendation circles, the IC, its capacitors (ranked) and the series parts on the board
// (rail detail, impedance tab): outlined footprints, their references queued in `labels`; true when drawn
const pdnOverlayOn = () => !!S.piRail && !$("pi-detail").hidden && !!$("ch-pdn-ic");
function drawPdnOverlay(px, labels) {
  if (!pdnOverlayOn()) return; // only while the per-IC view is open
  const r = pdnIcOf(S.piRail);
  if (!r || r.error) return;
  const pt = r.ports.find((x) => x.label === S.pdnIcSel);
  if (!pt) return;
  const C = S.colors, acc = getComputedStyle(document.documentElement).getPropertyValue("--accent").trim();
  // series parts of the domain
  for (const x of r.series) partBox(x.bbox, x.kind === "resistor" ? C.none : C.high1, 2 * px, x.refdes, {}, labels, px);
  // capacitors: colour by loop inductance to this IC (green best … red worst); the best five,
  // the selected and the hovered one labelled, all labelled when zoomed in
  const ls = pt.caps.map((l) => l.l_total), lo = Math.min(...ls), hi = Math.max(...ls);
  const zoomed = S.view.s > 14;
  let selCap = null;
  pt.caps.forEach((l, k) => {
    const c = r.caps[l.cap], t = hi > lo ? (l.l_total - lo) / (hi - lo) : 0;
    const col = t < 0.33 ? C.ok : t < 0.66 ? C.high1 : C.high2;
    const sel = S.pdnIcCap === l.cap || S.pdnIcHover === l.cap;
    if (sel) selCap = { c, col };
    else partBox(c.bbox, col, 2.5 * px, zoomed || k < 5 ? c.refdes : null, { fill: 0.22 }, labels, px);
  });
  // the IC
  partBox(pt.bbox, acc, 3 * px, pt.label, { fill: 0.12, bold: true }, labels, px);
  // the selected / hovered capacitor last, with a dashed line to the IC (the loop its inductance
  // belongs to)
  if (selCap) {
    const { c, col } = selCap;
    ctx.setLineDash([5 * px, 4 * px]); ctx.lineWidth = 4 * px; ctx.strokeStyle = C.halo;
    ctx.beginPath(); ctx.moveTo(pt.at[0], pt.at[1]); ctx.lineTo(c.at[0], c.at[1]); ctx.stroke();
    ctx.lineWidth = 2 * px; ctx.strokeStyle = col; ctx.stroke(); ctx.setLineDash([]);
    partBox(c.bbox, col, 3.5 * px, c.refdes, { fill: 0.35, bold: true, ring: true }, labels, px);
  }
  // recommendation radii
  ctx.setLineDash([6 * px, 4 * px]); ctx.lineWidth = 1.5 * px; ctx.strokeStyle = acc;
  for (const x of pt.recommendations.filter((x) => x.count && x.radius_mm > 0 && x.radius_mm < 49.9)) {
    ctx.beginPath(); ctx.arc(pt.at[0], pt.at[1], x.radius_mm, 0, Math.PI * 2); ctx.stroke();
    const lx = pt.at[0] + x.radius_mm * 0.71, ly = pt.at[1] - x.radius_mm * 0.71;
    labels.push({ x: lx, y: ly, text: `${x.count}× ${x.value}`, color: acc });
  }
  ctx.setLineDash([]);
  return true;
}
function renderPiLayerChips() {
  if (S.piMode === "l") { renderLossChips(); return; }
  if (S.piMode === "t") { renderThermChips(); return; }
  const su = S.scene.stackup, r = piCurrent();
  const ls = r ? r.res.layers.map((g) => g.layer) : [];
  if (r) piLayerIdx(r);
  $("layers").innerHTML = ls.map((l) => `<button class="chip" data-l="${l}" aria-pressed="${l === S.piLayer}"><i></i>${esc(su.copper[l].name)}</button>`).join("");
  $("layers").querySelectorAll(".chip").forEach((b) => b.onclick = () => { S.piLayer = +b.dataset.l; S.piImg = null; renderPiLayerChips(); draw(); });
}

function renderPiLegend() {
  if (S.piMode === "l") { renderLossLegend(); return; }
  if (S.piMode === "t") { renderThermLegend(); return; }
  const L = $("legend"), r = piCurrent();
  if (!r) { L.innerHTML = `<strong>Power</strong><div class="row">${S.piSolved ? "Rail without result: define source and load" : "Voltage drop not computed yet: “Compute voltage drop” in the Power card"}</div>`; return; }
  const li = piLayerIdx(r), im = piImage(r, li), sc = im.scale;
  const stops = Array.from({ length: 11 }, (_, k) => turbo(k / 10 * 0.9 + 0.05)).join(",");
  // the scale is in mV (drop, offset) or A/mm² (current density): ticks with SI prefixes
  const tick = (x) => sc.unit === "mV" ? fmtSI(x * 1e-3, "V", 2) : fmtSI(x, sc.unit, 2);
  L.innerHTML = `<strong>${S.piMode === "v" ? (r.res.ground ? "Ground offset" : "Voltage drop") : "Current density"} · ${esc(S.scene.stackup.copper[S.piLayer].name)}</strong>
    <div class="grad" style="background:linear-gradient(90deg,${stops})"></div>
    <div class="ticks units"><span>0</span><span>${tick(sc.max / 2)}</span><span>${tick(sc.max)}${S.piMode === "j" ? " (99th percentile)" : ""}</span></div>
    <div class="row"><span class="sw" style="background:${S.colors.ok}"></span>source ▲ · load ○ · circles: vias with the highest currents</div>
    <div class="row">Labels: ${pdnOverlayOn() ? "the IC, its capacitors and the series parts (impedance tab)" : S.piMode === "j" ? "sources and loads with their current (the largest first)" : r.res.ground ? "sources, offset at the parts (the largest first)" : "sources, drop at the loads (the largest first)"}</div>`;
}

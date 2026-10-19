"use strict";
// ---- AC impedance of a rail (PDN) -----------------------------------------------------
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
  const capVal = (c) => c >= 1e-6 ? fmt(c * 1e6, c < 1e-5 ? 1 : 0) + " µF" : c >= 1e-9 ? fmt(c * 1e9, 0) + " nF" : fmt(c * 1e12, 0) + " pF";
  const rows = [...groups.values()].map((g) => `<tr title="${esc(g.refs.join(", "))}"><td>${g.n} ×</td><td>${capVal(g.c)}</td><td>${g.pkg}</td><td>${fmt(g.l / g.n * 1e9, 2)}</td><td>${fHz(g.srf / g.n)}</td></tr>`).join("");
  const over = r.above_target.map(([a, b]) => `${fHz(a)} … ${fHz(b)}`).join(", ");
  body.innerHTML = `<div class="dstats"><span>Target <b>${r.target_ohm != null ? ohm(r.target_ohm) : "–"}</b>${r.target_ohm != null ? ` (3 % ripple at ${fmt(r.load_current * 0.5, 2)} A load step)` : ""}</span><span>C total <b>${capVal(r.c_total)}</b></span><span>plane ${fmt(r.c_plane * 1e9, 2)} nF</span>${r.cavity_hz ? `<span>1st cavity resonance ≈ ${fHz(r.cavity_hz)}</span>` : ""}</div>
    <div class="clegend"><span><i style="background:var(--pw-use)"></i>|Z| lumped</span>${r.cav_z.length ? `<span><i style="background:var(--pw-loss)"></i>plane model (${r.cav_cells} cells)</span>` : ""}<span><i style="background:var(--muted)"></i>regulator and plane only</span>${r.target_ohm != null ? `<span><i style="background:${S.colors.high2}"></i>target</span>` : ""}</div>
    <div class="chart"><canvas id="ch-pdn" style="height:200px" aria-label="Rail impedance over frequency"></canvas></div>
    ${r.target_ohm != null ? `<div class="dstats"><span>${over ? `<b class="warn">above target: ${over}</b>` : `<b>below target up to ${fHz(r.f_eval)}</b>`}</span></div>` : ""}
    ${rows ? `<div class="tablewrap"><table class="xs-table"><thead><tr><th>Count</th><th>Value</th><th>Package</th><th>L total nH</th><th>Resonance</th></tr></thead><tbody>${rows}</tbody></table></div>` : ""}
    <p class="note">Lumped model: regulator (2 mΩ + 20 nH) ∥ plane capacitance ∥ capacitors with ESR/ESL by package (estimated), mounting inductance to the plane and spreading to the load centroid. ${r.cav_z.length ? ` Plane model from 1 MHz: main plane as a grid (${fmt(r.cav_h_mm, 2)} mm) over ideal ground; shows the shape and resonances of the plane, but copper on other layers and traces are not included. Above 1 MHz the traffic light uses the plane model.` : ""} Evaluated up to ${fHz(r.f_eval)} – above that, package and die capacitance take over.</p>
    ${r.warnings.map((w) => `<div class="warn">${esc(w)}</div>`).join("")}
    <div id="pdn-ic"></div>`;
  drawPdn($("ch-pdn"), r);
  renderPdnIc($("pdn-ic"), setup, r);
}

// ---- PDN seen from every IC (AC grid over all layers) -------------------------------------
const capVal = (c) => c >= 1e-6 ? fmt(c * 1e6, c < 1e-5 ? 1 : 0) + " µF" : c >= 1e-9 ? fmt(c * 1e9, c < 1e-8 ? 1 : 0) + " nF" : fmt(c * 1e12, 0) + " pF";
function pdnIcOf(net) { return S.pdnIc && S.pdnIc.get(net); }
function renderPdnIc(host, setup, lumped) {
  if (!S.pdnIc) S.pdnIc = new Map();
  const r = pdnIcOf(setup.net);
  if (!r) {
    host.innerHTML = `<h4 class="sub">Seen from the ICs</h4><p class="note">AC grid of the rail copper on all layers with vias, every load IC as a port: Z(f) at each IC, the loop inductance of every capacitor to it, a target from the IC's own current step and recommendations. Takes a few seconds.</p><button class="btn small primary" id="pdn-ic-run">Compute per IC</button>`;
    $("pdn-ic-run").onclick = () => {
      $("pdn-ic-run").disabled = true; $("pdn-ic-run").textContent = "Computing …";
      setTimeout(() => {
        let res;
        try { res = S.engine.pdnIc({ setup, params: {} }); } catch (e) { res = { error: e.message }; }
        S.pdnIc.set(setup.net, res);
        S.pdnIcSel = res.ports && res.ports.length ? res.ports[0].refdes : null;
        renderPdnIc(host, setup, lumped); draw();
      }, 30);
    };
    return;
  }
  if (r.error) { host.innerHTML = `<h4 class="sub">Seen from the ICs</h4><p class="err">${esc(r.error)}</p>`; return; }
  let pt = r.ports.find((x) => x.refdes === S.pdnIcSel) || r.ports[0];
  S.pdnIcSel = pt.refdes;
  const chips = r.ports.map((x) => `<button class="chip" data-ic="${esc(x.refdes)}" aria-pressed="${x.refdes === pt.refdes}"><i style="background:${x.above.length ? S.colors.high2 : S.colors.ok}"></i>${esc(x.refdes)}</button>`).join("");
  const over = pt.above.map(([a, b]) => `${fHz(a)} … ${fHz(b)}`).join(", ");
  const rows = pt.caps.slice(0, 30).map((l) => {
    const c = r.caps[l.cap];
    const fl = c.flags.length ? ` <span class="warn" title="${esc(c.flags.join("; "))}">!</span>` : "";
    return `<tr class="click" data-cap="${l.cap}"><td>${esc(l.refdes)}${fl}</td><td title="nominal ${esc(c.value)}${c.rated_v ? `, rated ${fmt(c.rated_v, 1)} V${c.rated_assumed ? " (assumed)" : ""}` : ""}${c.dielectric ? " " + esc(c.dielectric) : ""}">${capVal(c.c_eff)}${c.c_eff < 0.95 * c.c_nom ? ` <span class="muted">(${fmt(100 * c.c_eff / c.c_nom, 0)} %)</span>` : ""}</td><td>${c.package}</td><td>${fmt(l.l_path * 1e9, 2)}</td><td>${fmt(l.l_total * 1e9, 2)}</td><td>${fHz(l.srf_hz)}</td><td>${fmt(l.dist_mm, 1)}</td></tr>`;
  }).join("");
  const recs = pt.recommendations.map((x) => x.count
    ? `<li><b>${x.count} × ${esc(x.value)} ${esc(x.package)}</b> ${x.radius_mm >= 49.9 ? "anywhere on the rail" : `within <b>${fmt(x.radius_mm, 1)} mm</b> of ${esc(pt.refdes)}`} <span class="muted">(${fHz(x.f_lo)} … ${fHz(x.f_hi)}; ${esc(x.note)})</span></li>`
    : `<li class="warn">${fHz(x.f_lo)} … ${fHz(x.f_hi)}: ${esc(x.note)}</li>`).join("");
  const peaks = pt.peaks.map((k) => `<li${k.above_target ? ' class="warn"' : ""}>${fHz(k.f)}: ${ohm(k.z)} – ${esc(k.note)}</li>`).join("");
  const flagged = r.caps.filter((c) => c.flags.length).map((c) => `<li class="click" data-cap="${r.caps.indexOf(c)}"><b>${esc(c.refdes)}</b> ${esc(c.value)}: ${esc(c.flags.join("; "))}</li>`).join("");
  host.innerHTML = `<h4 class="sub">Seen from the ICs</h4>
    <div class="chips">${chips}</div>
    <div class="dstats"><span>${esc(pt.refdes)} · ${fmt(pt.current, 3)} A · ${pt.pins} pin${pt.pins === 1 ? "" : "s"}</span><span>Target <b>${pt.target != null ? ohm(pt.target) : "–"}</b> <span class="muted">${esc(pt.target_source)}</span></span></div>
    <div class="clegend"><span><i style="background:var(--pw-use)"></i>|Z| at ${esc(pt.refdes)} (grid)</span><span><i class="dash" style="border-color:var(--muted)"></i>lumped (rail)</span>${pt.target != null ? `<span><i style="background:${S.colors.high2}"></i>target</span>` : ""}<span>▼ anti-resonance</span></div>
    <div class="chart"><canvas id="ch-pdn-ic" style="height:200px" aria-label="Impedance at the IC over frequency"></canvas></div>
    ${pt.target != null ? `<div class="dstats"><span>${over ? `<b class="warn">above target: ${over}</b>` : `<b>below target up to ${fHz(pt.f_max)}</b>`}</span></div>` : ""}
    ${recs ? `<p class="note"><b>Recommendations</b> (dashed circles on the board):</p><ul class="plain">${recs}</ul>` : ""}
    ${peaks ? `<p class="note"><b>Anti-resonances</b></p><ul class="plain">${peaks}</ul>` : ""}
    <div class="tablewrap"><table class="xs-table"><thead><tr><th>Capacitor</th><th>C at ${fmt(Math.abs(r.voltage), 2)} V</th><th>Package</th><th title="loop inductance from the IC through the planes and vias to the capacitor pad">L path nH</th><th title="L path + ground-side mounting + ESL">L total nH</th><th>Resonance</th><th>mm</th></tr></thead><tbody>${rows}</tbody></table></div>
    ${flagged ? `<p class="note"><b>Flags</b></p><ul class="plain">${flagged}</ul>` : ""}
    <p class="note">AC grid (${fmt(r.cell_mm, 2)} mm, ${r.nodes.toLocaleString("en-US")} nodes, ${r.layers.length} layers) of the rail copper against ground as ideal reference: plane capacitance and μ0·d per square, vias with their barrel inductance, the IC's rail pins shorted as one port. Capacitors with ESR, ESL and ground-side mounting by package and DC-bias derating (estimates unless a PI_CAP field gives them); regulator ${esc(r.regulators.join(", ") || "none")} as 2 mΩ + 20 nH (estimate). Targets from the IC's current step unless PI_TARGET. Recommendations add parts next to the IC until the target holds, then widen the radius as far as the result allows (estimate).</p>
    ${r.warnings.map((w) => `<div class="warn">${esc(w)}</div>`).join("")}`;
  host.querySelectorAll("[data-ic]").forEach((b) => b.onclick = () => { S.pdnIcSel = b.dataset.ic; renderPdnIc(host, setup, lumped); draw(); });
  host.querySelectorAll("[data-cap]").forEach((tr) => tr.onclick = () => { const c = r.caps[+tr.dataset.cap]; S.pdnIcCap = +tr.dataset.cap; zoomTo(c.at[0], c.at[1], 6); });
  drawZf($("ch-pdn-ic"), { freq: r.freq, series: [{ z: pt.z, color: "--pw-use", name: pt.refdes }, { f: lumped.freq, z: lumped.z, color: "--muted", dash: [4, 3], name: "lumped" }],
    target: pt.target, fEval: pt.f_max, marks: pt.peaks.map((k) => ({ f: k.f, label: "anti-resonance" })) });
}
// recommendation circles and capacitor ranking on the board (rail detail, impedance tab)
function drawPdnOverlay(px) {
  if (!S.piRail || $("pi-detail").hidden || !$("ch-pdn-ic")) return; // only while the per-IC view is open
  const r = pdnIcOf(S.piRail);
  if (!r || r.error) return;
  const pt = r.ports.find((x) => x.refdes === S.pdnIcSel);
  if (!pt) return;
  const C = S.colors;
  // capacitors: colour by loop inductance to this IC (green best … red worst)
  const ls = pt.caps.map((l) => l.l_total), lo = Math.min(...ls), hi = Math.max(...ls);
  for (const l of pt.caps) {
    const c = r.caps[l.cap], t = hi > lo ? (l.l_total - lo) / (hi - lo) : 0;
    ctx.fillStyle = t < 0.33 ? C.ok : t < 0.66 ? C.high1 || C.high2 : C.high2;
    ctx.strokeStyle = C.halo; ctx.lineWidth = 1.5 * px;
    ctx.beginPath(); ctx.arc(c.at[0], c.at[1], 3.5 * px, 0, Math.PI * 2); ctx.fill(); ctx.stroke();
    if (S.pdnIcCap === l.cap) { ctx.beginPath(); ctx.arc(c.at[0], c.at[1], 8 * px, 0, Math.PI * 2); ctx.stroke(); }
  }
  // recommendation radii
  ctx.setLineDash([6 * px, 4 * px]); ctx.lineWidth = 1.5 * px; ctx.strokeStyle = C.halo;
  ctx.font = `${11 * px}px ${getComputedStyle(document.documentElement).getPropertyValue("--font-mono")}`; ctx.fillStyle = C.halo; ctx.textAlign = "left";
  for (const x of pt.recommendations.filter((x) => x.count && x.radius_mm > 0 && x.radius_mm < 49.9)) {
    ctx.beginPath(); ctx.arc(pt.at[0], pt.at[1], x.radius_mm, 0, Math.PI * 2); ctx.stroke();
    ctx.fillText(`${x.count}× ${x.value}`, pt.at[0] + x.radius_mm * 0.71 + 2 * px, pt.at[1] - x.radius_mm * 0.71);
  }
  ctx.setLineDash([]);
  marker(pt.at[0], pt.at[1], px, "load");
}
function renderPiLayerChips() {
  if (S.piMode === "l") { renderLossChips(); return; }
  const su = S.scene.stackup, r = piCurrent();
  const ls = r ? r.res.layers.map((g) => g.layer) : [];
  if (r) piLayerIdx(r);
  $("layers").innerHTML = ls.map((l) => `<button class="chip" data-l="${l}" aria-pressed="${l === S.piLayer}"><i></i>${esc(su.copper[l].name)}</button>`).join("");
  $("layers").querySelectorAll(".chip").forEach((b) => b.onclick = () => { S.piLayer = +b.dataset.l; S.piImg = null; renderPiLayerChips(); draw(); });
}

function renderPiLegend() {
  if (S.piMode === "l") { renderLossLegend(); return; }
  const L = $("legend"), r = piCurrent();
  if (!r) { L.innerHTML = `<strong>Power</strong><div class="row">${S.piSolved ? "Rail without result: define source and load" : "Voltage drop not computed yet: “Compute voltage drop” in the Power card"}</div>`; return; }
  const li = piLayerIdx(r), im = piImage(r, li), sc = im.scale;
  const stops = Array.from({ length: 11 }, (_, k) => turbo(k / 10 * 0.9 + 0.05)).join(",");
  const d = sc.max < 1 ? 2 : sc.max < 10 ? 1 : 0;
  L.innerHTML = `<strong>${S.piMode === "v" ? (r.res.ground ? "Ground offset" : "Voltage drop") : "Current density"} · ${esc(S.scene.stackup.copper[S.piLayer].name)}</strong>
    <div class="grad" style="background:linear-gradient(90deg,${stops})"></div>
    <div class="ticks"><span>0</span><span>${fmt(sc.max / 2, d)}</span><span>${fmt(sc.max, d)} ${sc.unit}${S.piMode === "j" ? " (99th percentile)" : ""}</span></div>
    <div class="row"><span class="sw" style="background:${S.colors.ok}"></span>source ▲ · load ○ · circles: vias with the highest currents</div>`;
}

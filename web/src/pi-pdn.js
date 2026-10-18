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
    ${r.warnings.map((w) => `<div class="warn">${esc(w)}</div>`).join("")}`;
  drawPdn($("ch-pdn"), r);
}
function renderPiLayerChips() {
  const su = S.scene.stackup, r = piCurrent();
  const ls = r ? r.res.layers.map((g) => g.layer) : [];
  if (r) piLayerIdx(r);
  $("layers").innerHTML = ls.map((l) => `<button class="chip" data-l="${l}" aria-pressed="${l === S.piLayer}"><i></i>${esc(su.copper[l].name)}</button>`).join("");
  $("layers").querySelectorAll(".chip").forEach((b) => b.onclick = () => { S.piLayer = +b.dataset.l; S.piImg = null; renderPiLayerChips(); draw(); });
}

function renderPiLegend() {
  const L = $("legend"), r = piCurrent();
  if (!r) { L.innerHTML = `<strong>Power</strong><div class="row">${S.piSolved ? "Rail without result: define source and load" : "Voltage drop not computed yet: “Compute voltage drop” in the Power card"}</div>`; return; }
  const li = piLayerIdx(r), im = piImage(r, li), sc = im.scale;
  const stops = Array.from({ length: 11 }, (_, k) => turbo(k / 10 * 0.9 + 0.05)).join(",");
  const d = sc.max < 1 ? 2 : sc.max < 10 ? 1 : 0;
  L.innerHTML = `<strong>${S.piMode === "v" ? "Voltage drop" : "Current density"} · ${esc(S.scene.stackup.copper[S.piLayer].name)}</strong>
    <div class="grad" style="background:linear-gradient(90deg,${stops})"></div>
    <div class="ticks"><span>0</span><span>${fmt(sc.max / 2, d)}</span><span>${fmt(sc.max, d)} ${sc.unit}${S.piMode === "j" ? " (99th percentile)" : ""}</span></div>
    <div class="row"><span class="sw" style="background:${S.colors.ok}"></span>source ▲ · load ○ · circles: vias with the highest currents</div>`;
}

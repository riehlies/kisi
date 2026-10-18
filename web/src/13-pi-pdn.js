"use strict";
// ---- AC impedance of a rail (PDN) -----------------------------------------------------
const fHz = (f) => f >= 1e9 ? fmt(f / 1e9, f < 1e10 ? 1 : 0) + " GHz" : f >= 1e6 ? fmt(f / 1e6, f < 1e7 ? 1 : 0) + " MHz" : fmt(f / 1e3, f < 1e4 ? 1 : 0) + " kHz";
const ohm = (z) => z >= 10 ? fmt(z, 0) + " Ω" : z >= 1 ? fmt(z, z % 1 ? 2 : 0) + " Ω" : fmt(z * 1e3, z < 0.01 ? 1 : 0) + " mΩ";
function renderPdn(body, setup) {
  let r;
  try { r = S.engine.pdn({ setup, params: {} }); } catch (e) { body.innerHTML = `<p class="err">${esc(e.message)}</p>`; return; }
  const groups = new Map();
  for (const c of r.caps) {
    const k = `${c.c.toExponential(3)}|${c.package}`;
    const g = groups.get(k) || { c: c.c, pkg: c.package, n: 0, l: 0, srf: 0, refs: [] };
    g.n++; g.l += c.esl + c.l_mount + c.l_spread; g.srf += c.srf; g.refs.push(c.refdes); groups.set(k, g);
  }
  const capVal = (c) => c >= 1e-6 ? fmt(c * 1e6, c < 1e-5 ? 1 : 0) + " µF" : c >= 1e-9 ? fmt(c * 1e9, 0) + " nF" : fmt(c * 1e12, 0) + " pF";
  const rows = [...groups.values()].map((g) => `<tr title="${esc(g.refs.join(", "))}"><td>${g.n} ×</td><td>${capVal(g.c)}</td><td>${g.pkg}</td><td>${fmt(g.l / g.n * 1e9, 2)}</td><td>${fHz(g.srf / g.n)}</td></tr>`).join("");
  const over = r.above_target.map(([a, b]) => `${fHz(a)} … ${fHz(b)}`).join(", ");
  body.innerHTML = `<div class="dstats"><span>Ziel <b>${r.target_ohm != null ? ohm(r.target_ohm) : "–"}</b>${r.target_ohm != null ? ` (3 % Welligkeit bei ${fmt(r.load_current * 0.5, 2)} A Lastsprung)` : ""}</span><span>C gesamt <b>${capVal(r.c_total)}</b></span><span>Ebene ${fmt(r.c_plane * 1e9, 2)} nF</span>${r.cavity_hz ? `<span>1. Hohlraumresonanz ≈ ${fHz(r.cavity_hz)}</span>` : ""}</div>
    <div class="clegend"><span><i style="background:var(--pw-use)"></i>|Z| konzentriert</span>${r.cav_z.length ? `<span><i style="background:var(--pw-loss)"></i>Ebenenmodell (${r.cav_cells} Zellen)</span>` : ""}<span><i style="background:var(--muted)"></i>nur Regler und Ebene</span>${r.target_ohm != null ? `<span><i style="background:${S.colors.high2}"></i>Ziel</span>` : ""}</div>
    <div class="chart"><canvas id="ch-pdn" style="height:200px" aria-label="Impedanz der Schiene über der Frequenz"></canvas></div>
    ${r.target_ohm != null ? `<div class="dstats"><span>${over ? `<b class="warn">über dem Ziel: ${over}</b>` : `<b>bis ${fHz(r.f_eval)} unter dem Ziel</b>`}</span></div>` : ""}
    ${rows ? `<div class="tablewrap"><table class="xs-table"><thead><tr><th>Anzahl</th><th>Wert</th><th>Bauform</th><th>L gesamt nH</th><th>Resonanz</th></tr></thead><tbody>${rows}</tbody></table></div>` : ""}
    <p class="note">Konzentriertes Modell: Regler (2 mΩ + 20 nH) ∥ Ebenenkapazität ∥ Kondensatoren mit ESR/ESL nach Bauform (geschätzt), Montage-Induktivität bis zur Ebene und Ausbreitung bis zum Lastschwerpunkt. ${r.cav_z.length ? ` Ebenenmodell ab 1 MHz: Hauptebene als Gitter (${fmt(r.cav_h_mm, 2)} mm) über idealer Masse, zeigt Form und Resonanzen der Fläche; Kupfer auf anderen Lagen und Leiterbahnen fehlen dort. Die Ampel nutzt oberhalb 1 MHz das Ebenenmodell.` : ""} Bewertet bis ${fHz(r.f_eval)} – darüber übernehmen Gehäuse- und Chip-Kapazität.</p>
    ${r.warnings.map((w) => `<div class="warn">${esc(w)}</div>`).join("")}`;
  drawPdn($("ch-pdn"), r);
}
function drawPdn(c, r) {
  const d = window.devicePixelRatio || 1, rc = c.getBoundingClientRect();
  c.width = rc.width * d; c.height = rc.height * d;
  const g = c.getContext("2d");
  const cs = getComputedStyle(document.documentElement);
  const muted = cs.getPropertyValue("--muted").trim(), line = cs.getPropertyValue("--line").trim(), use = cs.getPropertyValue("--pw-use").trim();
  const lf = r.freq.map(Math.log10), lz = r.z.map((z) => Math.log10(Math.max(z, 1e-6))), lb = r.z_bare.map((z) => Math.log10(Math.max(z, 1e-6)));
  const cf = r.cav_freq.map(Math.log10), cz = r.cav_z.map((z) => Math.log10(Math.max(z, 1e-6))), loss = cs.getPropertyValue("--pw-loss").trim();
  const fmin = lf[0], fmax = lf[lf.length - 1];
  let lo = Math.min(...lz), hi = Math.max(...lz.filter((_, i) => r.freq[i] <= r.f_eval * 3), ...cz.filter((_, i) => r.cav_freq[i] <= r.f_eval * 3));
  if (r.target_ohm) { lo = Math.min(lo, Math.log10(r.target_ohm)); hi = Math.max(hi, Math.log10(r.target_ohm)); }
  lo = Math.floor(lo - 0.2); hi = Math.ceil(hi + 0.2);
  const padL = 50, padR = 10, padT = 8, padB = 20, W = rc.width - padL - padR, H = rc.height - padT - padB;
  const X = (v) => padL + (v - fmin) / (fmax - fmin) * W, Y = (v) => padT + (1 - (v - lo) / (hi - lo)) * H;
  const paint = (hi_i) => {
    g.setTransform(d, 0, 0, d, 0, 0); g.clearRect(0, 0, rc.width, rc.height);
    g.font = "10.5px " + cs.getPropertyValue("--font-mono"); g.fillStyle = muted; g.strokeStyle = line; g.lineWidth = 1;
    for (let v = lo; v <= hi; v++) { g.beginPath(); g.moveTo(padL, Y(v)); g.lineTo(padL + W, Y(v)); g.stroke(); g.textAlign = "right"; g.textBaseline = "middle"; g.fillText(ohm(10 ** v), padL - 6, Y(v)); }
    g.textAlign = "center"; g.textBaseline = "top";
    for (let v = Math.ceil(fmin); v <= fmax; v++) { g.beginPath(); g.moveTo(X(v), padT); g.lineTo(X(v), padT + H); g.stroke(); g.fillText(fHz(10 ** v), Math.min(Math.max(X(v), padL + 18), padL + W - 22), padT + H + 5); }
    // not evaluated above f_eval
    const xe = X(Math.log10(r.f_eval));
    if (xe < padL + W) { g.fillStyle = muted; g.globalAlpha = 0.1; g.fillRect(xe, padT, padL + W - xe, H); g.globalAlpha = 1; }
    g.save(); g.beginPath(); g.rect(padL, padT, W, H); g.clip();
    if (r.target_ohm) { g.strokeStyle = S.colors.high2; g.setLineDash([5, 4]); g.lineWidth = 1.5; g.beginPath(); g.moveTo(padL, Y(Math.log10(r.target_ohm))); g.lineTo(padL + W, Y(Math.log10(r.target_ohm))); g.stroke(); g.setLineDash([]); }
    for (const [xa, arr, colr, wdt] of [[lf, lb, muted, 1.5], [lf, lz, use, 2], [cf, cz, loss, 2]]) {
      g.strokeStyle = colr; g.lineWidth = wdt; g.beginPath();
      arr.forEach((v, i) => { const px = X(xa[i]), py = Y(v); if (i) g.lineTo(px, py); else g.moveTo(px, py); });
      g.stroke();
    }
    g.restore();
    if (hi_i != null) {
      g.strokeStyle = muted; g.setLineDash([3, 3]); g.beginPath(); g.moveTo(X(lf[hi_i]), padT); g.lineTo(X(lf[hi_i]), padT + H); g.stroke(); g.setLineDash([]);
      g.fillStyle = use; g.strokeStyle = cs.getPropertyValue("--panel").trim(); g.lineWidth = 2; g.beginPath(); g.arc(X(lf[hi_i]), Y(lz[hi_i]), 4, 0, Math.PI * 2); g.fill(); g.stroke();
    }
  };
  paint(null);
  const host = c.parentElement;
  let tip = host.querySelector(".ctip");
  if (!tip) { tip = document.createElement("div"); tip.className = "ctip"; tip.hidden = true; host.appendChild(tip); }
  c.onpointermove = (e) => {
    const px = e.offsetX; if (px < padL || px > padL + W) { tip.hidden = true; paint(null); return; }
    const lv = fmin + (px - padL) / W * (fmax - fmin);
    let bi = 0; for (let i = 0; i < lf.length; i++) if (Math.abs(lf[i] - lv) < Math.abs(lf[bi] - lv)) bi = i;
    paint(bi);
    let ci = -1; for (let i = 0; i < cf.length; i++) if (ci < 0 || Math.abs(cf[i] - lv) < Math.abs(cf[ci] - lv)) ci = i;
    const cav = ci >= 0 && Math.abs(cf[ci] - lv) < 0.06 ? `<br>Ebenenmodell ${ohm(r.cav_z[ci])}` : "";
    tip.innerHTML = `${fHz(r.freq[bi])}<br>konzentriert ${ohm(r.z[bi])}${cav}<br>ohne C ${ohm(r.z_bare[bi])}`;
    tip.hidden = false;
    tip.style.left = Math.min(X(lf[bi]) + 10, rc.width - tip.offsetWidth - 4) + "px"; tip.style.top = "6px";
  };
  c.onpointerleave = () => { tip.hidden = true; paint(null); };
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
  if (!r) { L.innerHTML = `<strong>Versorgung</strong><div class="row">Schiene ohne Ergebnis: Quelle und Last festlegen</div>`; return; }
  const li = piLayerIdx(r), im = piImage(r, li), sc = im.scale;
  const stops = Array.from({ length: 11 }, (_, k) => turbo(k / 10 * 0.9 + 0.05)).join(",");
  const d = sc.max < 1 ? 2 : sc.max < 10 ? 1 : 0;
  L.innerHTML = `<strong>${S.piMode === "v" ? "Spannungsabfall" : "Stromdichte"} · ${esc(S.scene.stackup.copper[S.piLayer].name)}</strong>
    <div class="grad" style="background:linear-gradient(90deg,${stops})"></div>
    <div class="ticks"><span>0</span><span>${fmt(sc.max / 2, d)}</span><span>${fmt(sc.max, d)} ${sc.unit}${S.piMode === "j" ? " (99. Perzentil)" : ""}</span></div>
    <div class="row"><span class="sw" style="background:${S.colors.ok}"></span>Quelle ▲ · Last ○ · Kreise: Vias mit den höchsten Strömen</div>`;
}

function setTab(t) {
  S.tab = t;
  $("view-si").setAttribute("aria-pressed", String(t === "si"));
  $("view-pi").setAttribute("aria-pressed", String(t === "pi"));
  document.querySelectorAll("[data-view]").forEach((el) => { el.hidden = el.dataset.view !== t; });
  $("detail").hidden = t !== "si" || S.selNet < 0;
  $("pi-detail").hidden = true;
  $("pi-balance").hidden = true; $("legend").hidden = false; $("layers").hidden = false;
  $("tip").hidden = true;
  if (t === "pi") { fitRail(); renderPi(); } else { fitView(); renderLayerChips(); renderLegend(); draw(); }
}
$("view-si").onclick = () => setTab("si");
$("view-pi").onclick = () => setTab("pi");
function setPiMode(m) {
  S.piMode = m; S.piImg = null;
  for (const k of ["v", "j", "b"]) $("pim-" + k).setAttribute("aria-pressed", String(k === m));
  renderPi();
}
$("pim-b").onclick = () => setPiMode("b");
$("pim-v").onclick = () => setPiMode("v");
$("pim-j").onclick = () => setPiMode("j");

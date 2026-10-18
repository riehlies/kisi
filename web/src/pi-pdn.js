"use strict";
// ---- AC impedance of a rail (PDN) -----------------------------------------------------
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

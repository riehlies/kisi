"use strict";
// ---- board-level channel check ------------------------------------------------------
function checkItems() {
  const out = [];
  for (const st of S.an.nets) {
    const t = classTarget(st.net);
    if (!t) continue;
    if (st.partner != null) {
      // one line per pair, preferably the positive line
      const neg = /(_N|N|-|_M)$/i.test(st.name), pneg = /(_N|N|-|_M)$/i.test(S.scene.nets[st.partner] || "");
      if (neg !== pneg ? neg : st.partner < st.net) continue;
    }
    out.push({ net: st.net, gbps: eyeRate(st.net).gbps, target: [t.z, t.diff] });
  }
  return out;
}
function checkStatus(r) {
  if (r.error || r.eye_height < 0.4) return ["high2", "rot"];
  const ui = 1000 / r.gbps;
  if (r.eye_height < 0.65 || r.rl_db < 10 || (r.skew_ps != null && !r.skew_uncertain && Math.abs(r.skew_ps) > 0.05 * ui) || (r.stub_ghz != null && r.stub_ghz < 2 * r.nyquist_ghz)) return ["high1", "gelb"];
  return ["ok", "grün"];
}
async function runCheck() {
  const items = checkItems(), rows = [], gen = S.anGen, p = rfParams();
  const btn = $("ck-start"); btn.disabled = true;
  for (let i = 0; i < items.length; i += 4) {
    if (gen !== S.anGen) break;
    $("ck-text").textContent = `Prüfe ${Math.min(i + 4, items.length)} von ${items.length} Kanälen …`;
    await new Promise((res) => setTimeout(res, 0));
    try {
      S.engine.call(S.engine.k.kisi_check, JSON.stringify({ rf: p, items: items.slice(i, i + 4) }));
      rows.push(...JSON.parse(S.engine.out()));
    } catch (e) { console.warn(e); }
  }
  btn.disabled = false;
  S.check = { gen, rows };
  renderCheck();
}
function renderCheck() {
  const box = $("ck-out");
  if (!S.check || !S.check.rows.length) {
    box.innerHTML = "";
    $("ck-text").textContent = "Alle Netze mit Zielimpedanz: Dämpfung bei Nyquist, Rückflussdämpfung, Auge ohne Entzerrung, Skew im Paar, Via-Stubs.";
    return;
  }
  const rows = S.check.rows.slice().map((r) => ({ r, st: checkStatus(r) }));
  const rank = { high2: 0, high1: 1, ok: 2 };
  rows.sort((a, b) => rank[a.st[0]] - rank[b.st[0]] || a.r.eye_height - b.r.eye_height);
  const n = { high2: 0, high1: 0, ok: 0 }; rows.forEach((x) => n[x.st[0]]++);
  const stale = S.check.gen !== S.anGen;
  $("ck-text").textContent = `${rows.length} Kanäle: ${n.ok} grün, ${n.high1} gelb, ${n.high2} rot${stale ? " · veraltet, neu prüfen" : ""}`;
  const shown = S.checkAll ? rows : rows.slice(0, 15);
  box.innerHTML = `<div class="tablewrap"><table class="xs-table ck-table"><thead><tr><th></th><th>Netz</th><th>Gbit/s</th><th>IL dB</th><th>Auge %</th><th>Skew ps</th></tr></thead><tbody>${shown.map(({ r, st }) => {
    const nm = r.name.split("/").pop();
    const sk = r.skew_ps == null ? "–" : fmt(Math.abs(r.skew_ps), 1) + (r.skew_uncertain ? "?" : "");
    return `<tr class="click" data-net="${r.net}" title="${esc(r.name)} · ${r.diff ? "Paar" : "einzeln"}, ${fmt(r.length_mm, 1)} mm, ${r.vias} Vias, RL ${fmt(r.rl_db, 1)} dB${r.stub_ghz != null ? ", Stub-Resonanz " + fmt(r.stub_ghz, 1) + " GHz" : ""}${r.error ? " · " + esc(r.error) : ""}"><td><span class="dot" style="background:${S.colors[st[0]]}" aria-label="${st[1]}"></span></td><td>${esc(nm)}</td><td>${fmt(r.gbps, r.gbps < 1 ? 2 : r.gbps % 1 ? 2 : 0)}</td><td>${fmt(r.il_nyq_db, 1)}</td><td>${r.error ? "–" : fmt(r.eye_height * 100, 0)}</td><td>${sk}</td></tr>`;
  }).join("")}</tbody></table></div>
  ${rows.length > 15 ? `<button class="btn small" id="ck-all">${S.checkAll ? "Nur die ersten 15" : `Alle ${rows.length} zeigen`}</button>` : ""}
  <p class="note">Rot: Auge unter 40 % des Hubs. Gelb: Auge unter 65 %, Rückflussdämpfung unter 10 dB bis 1,5 × Nyquist, Skew über 5 % UI oder Via-Stub-Resonanz unter 2 × Nyquist. Grenzen sind Richtwerte für ein Board ohne Entzerrung, keine Spezifikation. „?“ = Pfad verzweigt, Skew unsicher.</p>`;
  box.querySelectorAll("tr[data-net]").forEach((tr) => tr.onclick = () => { S.detailTab = "eye"; selectNet(+tr.dataset.net); });
  if ($("ck-all")) $("ck-all").onclick = () => { S.checkAll = !S.checkAll; renderCheck(); };
}

function renderEye(body, r) {
  const ch = r.channel, e = r.eye;
  if (!ch) { body.innerHTML = `<p class="err">${esc(r.error || "kein Kanal")}</p>`; return; }
  if (!e || e.error) { body.innerHTML = `<p class="err">${esc(e?.error || "kein Augendiagramm")}</p>`; return; }
  const g = r.rate.gbps, uiPct = (x) => fmt(100 * x / e.ui_ps, 0);
  let skew = "";
  if (r.skew) {
    const lim = 0.05 * e.ui_ps, sk = Math.abs(r.skew.skew_ps), ok = sk <= lim;
    skew = `<span>Skew P/N <b class="${ok ? "" : "warn"}">${fmt(sk, 1)} ps</b> (${fmt(Math.abs(r.skew.skew_mm), 2)} mm) · Richtwert ≤ ${fmt(lim, 1)} ps (5 % UI)${r.skew.uncertain ? " · Pfad verzweigt oder über Bauteile geteilt, Skew daher unsicher" : ""}</span>`;
  }
  const warn = (e.warnings || []).map((w) => `<div class="warn">${esc(w)}</div>`).join("");
  body.innerHTML = `<div class="dstats"><span>${fmt(g, g < 1 ? 2 : g % 1 ? 2 : 0)} Gbit/s NRZ, PRBS7${r.rate.guess ? " (aus Netzklasse geschätzt)" : ""}</span><span>UI ${fmt(e.ui_ps, 1)} ps</span></div>
    <div class="dstats"><span>Augenhöhe <b>${fmt(e.height * 100, 0)} %</b> des Hubs</span><span>Augenbreite <b>${fmt(e.width_ps, 1)} ps</b> (${uiPct(e.width_ps)} % UI)</span><span>ISI-Summe <b>${fmt(e.isi_sum * 100, 0)} %</b></span></div>
    ${skew ? `<div class="dstats">${skew}</div>` : ""}
    <div class="chart"><canvas id="ch-eye" style="height:220px" aria-label="Augendiagramm"></canvas></div>
    <p class="note">Lineare Überlagerung der Einzelbit-Antwort aus S21 (Sender-Flanke ${fmt(rfParams().rise * 1e12, 0)} ps, angepasste Abschlüsse). Ohne Entzerrung, Jitter und Rauschen – ein optimistischer Wert für das passive Board, kein Compliance-Test.${ch.diff ? " Differentiell: ungerade Mode, Gleichtakt-Umwandlung durch Skew nicht enthalten." : ""}</p>${warn}`;
  drawEye($("ch-eye"), e);
}

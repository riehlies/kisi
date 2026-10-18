"use strict";
// ---- high-frequency views of a net -----------------------------------------------
function rfParams() {
  return {
    fmax: parseFloat($("rf-fmax").value) * 1e9, npts: 500,
    rise: parseFloat($("rf-rise").value) * 1e-12,
    roughness: $("rf-rough").value, sigma: 5.8e7, vias: $("rf-vias").checked,
    backdrill: $("rf-bd").value === "" ? null : parseFloat($("rf-bd").value),
  };
}
// data rate for the eye: explicit setting or a guess from class / net name
function eyeRate(net) {
  const v = $("rf-rate").value;
  if (v !== "auto") return { gbps: parseFloat(v), guess: false };
  // class name plus the net name without its sheet path ("/TS-PCIe Components/SYNC2" → "SYNC2")
  const ci = S.scene.net_class[net] ?? 0, cls = (S.an.classes[ci]?.name || "") + " " + (S.scene.nets?.[net] || "").split("/").pop();
  const rules = [[/USB_?D[PM+-]|USB2|D[PM]$/i, 0.48], [/PCIE|PCI_E|PE[TR]\d/i, 16], [/DDR5/i, 6.4], [/DDR4/i, 3.2], [/DDR|DQ|DQS/i, 3.2],
    [/USB|SS[TR]X/i, 5], [/MGT|GTP|GTX|SERDES|SFP/i, 6.25], [/HDMI|TMDS/i, 3.0], [/SGMII|RGMII|ETH|RJ45/i, 1.25], [/LVDS|ADC|LCLK|FCLK/i, 1.0]];
  for (const [re, g] of rules) if (re.test(cls)) return { gbps: g, guess: true };
  return { gbps: 1.0, guess: true };
}
function rfFor(net) {
  const t = classTarget(net), p = rfParams(), er = eyeRate(net);
  const key = `${net}|${S.anGen}|${JSON.stringify(p)}|${er.gbps}`;
  if (S.rfCache && S.rfCache.key === key) return S.rfCache.res;
  let res;
  try { res = S.engine.rf({ net, rf: p, target: t ? [t.z, t.diff] : null, bitrate: er.gbps * 1e9 }); res.rate = er; }
  catch (e) { res = { channel: null, error: e.message, crosstalk: { aggressors: [], geoms: [] }, touchstone: "" }; }
  S.rfCache = { key, res, net };
  return res;
}
function renderTab(net, t, rowsXs) {
  const body = $("tab-body");
  if (!body) return;
  if (S.detailTab === "z") {
    body.innerHTML = `<canvas id="profile" aria-label="Impedanzverlauf entlang des Netzes"></canvas>
      <div class="tablewrap"><table class="xs-table"><thead><tr><th>mm</th><th>Lage</th><th>Typ</th><th>w µm</th><th>Abst. µm</th><th>Z₀</th><th>Z diff</th><th>vergl.</th></tr></thead><tbody>${rowsXs}</tbody></table></div>`;
    drawProfile(net, t);
    return;
  }
  const r = rfFor(net), ch = r.channel;
  if (S.detailTab === "xt") return renderXt(body, r, net);
  if (S.detailTab === "via") return renderVias(body, r);
  if (S.detailTab === "eye") return renderEye(body, r);
  if (!ch) { body.innerHTML = `<p class="err">${esc(r.error || "kein Kanal")}</p>`; return; }
  const src = ch.field_share > 0 ? `Feldlöser für ${Math.round(ch.field_share * 100)} % der Länge, sonst Formeln` : "Formeln (schnelle Analyse)";
  const head = `<div class="dstats"><span>${ch.diff ? "differentiell, Bezug " + fmt(2 * ch.z_ref, 0) + " Ω" : "Bezug " + fmt(ch.z_ref, 0) + " Ω"}</span><span>Pfad <b>${fmt(ch.length_mm, 1)} mm</b></span><span>Laufzeit <b>${fmt(ch.delay_ps, 0)} ps</b></span><span>Vias <b>${ch.vias}</b></span></div>`;
  const warn = (ch.warnings || []).map((w) => `<div class="warn">${esc(w)}</div>`).join("");
  if (S.detailTab === "loss") {
    const marks = ch.il_marks.map(([f, d]) => `<span>${fmt(f, f < 10 ? 1 : 0)} GHz <b>${fmt(d, 2)} dB</b></span>`).join("");
    const lpi = ch.loss_db_per_in_at.map(([f, c, d]) => `<tr><td>${fmt(f, 0)} GHz</td><td>${fmt(c, 3)}</td><td>${fmt(d, 3)}</td><td>${fmt(c + d, 3)}</td></tr>`).join("");
    body.innerHTML = `${head}
      <div class="clegend"><span><i style="background:var(--pw-use)"></i>${ch.diff ? "Sdd21" : "S21"} Einfügedämpfung</span><span><i style="background:var(--pw-loss)"></i>${ch.diff ? "Sdd11" : "S11"} Reflexion</span></div>
      <div class="chart"><canvas id="ch-s" aria-label="S-Parameter über der Frequenz"></canvas></div>
      <div class="marks">${marks}</div>
      <div class="tablewrap"><table class="xs-table"><thead><tr><th>Leitungsverlust</th><th>Leiter dB/in</th><th>Dielektrikum dB/in</th><th>gesamt</th></tr></thead><tbody>${lpi}</tbody></table></div>
      <div class="dstats"><span class="muted">${esc(src)}</span><button class="btn small" id="ts-copy">Touchstone kopieren</button></div>${warn}`;
    const fg = ch.freq.map((f) => f / 1e9);
    drawLines($("ch-s"), { x: fg, xUnit: "GHz", yUnit: "dB", series: [
      { y: ch.s21_db, color: "--pw-use", name: ch.diff ? "Sdd21" : "S21" },
      { y: ch.s11_db, color: "--pw-loss", name: ch.diff ? "Sdd11" : "S11" }], yMin: Math.max(-60, Math.min(...ch.s11_db, ...ch.s21_db) - 2), yMax: 0 });
    $("ts-copy").onclick = (e) => {
      navigator.clipboard.writeText(r.touchstone).then(() => { e.target.textContent = "Kopiert (.s2p)"; }).catch(() => { e.target.textContent = "Kopieren nicht erlaubt"; });
    };
    return;
  }
  if (S.detailTab === "tdr") {
    const scale = ch.diff ? 2 : 1;
    const inLine = ch.tdr_z.filter((_, i) => ch.tdr_x_mm[i] <= ch.length_mm);
    const zmin = Math.min(...inLine) * scale, zmax = Math.max(...inLine) * scale;
    body.innerHTML = `${head}
      <div class="clegend"><span><i style="background:var(--pw-use)"></i>Impedanz aus der Sprungantwort (${fmt(rfParams().rise * 1e12, 0)} ps Anstieg)</span>${t ? `<span><i style="background:${S.colors.ok}"></i>Ziel ±5 %</span>` : ""}</div>
      <div class="chart"><canvas id="ch-tdr" aria-label="TDR-Impedanz über dem Ort"></canvas></div>
      <div class="marks"><span>${ch.diff ? "Zdiff" : "Z"} min <b>${fmt(zmin, 1)} Ω</b></span><span>${ch.diff ? "Zdiff" : "Z"} max <b>${fmt(zmax, 1)} Ω</b></span><span class="muted">Ort aus der mittleren Laufzeit; nach dem Leitungsende folgt der Abschluss mit dem Bezugswert.</span></div>${warn}`;
    const zt = t ? (t.diff ? t.z / 2 : t.z) : null;
    drawLines($("ch-tdr"), { x: ch.tdr_x_mm, xUnit: "mm", yUnit: "Ω", series: [{ y: ch.tdr_z.map((z) => z * scale), color: "--pw-use", name: ch.diff ? "Z diff" : "Z" }],
      band: zt ? [zt * scale * 0.95, zt * scale * 1.05] : null, xMax: ch.length_mm * 1.25 });
  }
}

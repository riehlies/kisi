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
function rfFor(net) {
  const t = classTarget(net), p = rfParams();
  const key = `${net}|${S.anGen}|${JSON.stringify(p)}`;
  if (S.rfCache && S.rfCache.key === key) return S.rfCache.res;
  let res;
  try { res = S.engine.rf({ net, rf: p, target: t ? [t.z, t.diff] : null }); }
  catch (e) { res = { channel: null, error: e.message, crosstalk: { aggressors: [], geoms: [] }, touchstone: "" }; }
  S.rfCache = { key, res, net };
  return res;
}
function renderTab(net, t, rowsXs) {
  const body = $("tab-body");
  if (!body) return;
  if (S.detailTab === "z") {
    body.innerHTML = `<canvas id="profile" aria-label="Impedance profile along the net"></canvas>
      <div class="tablewrap"><table class="xs-table"><thead><tr><th>mm</th><th>Layer</th><th>Type</th><th>w µm</th><th>gap µm</th><th>Z₀</th><th>Z diff</th><th>compared</th></tr></thead><tbody>${rowsXs}</tbody></table></div>`;
    drawProfile(net, t);
    return;
  }
  const r = rfFor(net), ch = r.channel;
  if (S.detailTab === "xt") return renderXt(body, r, net);
  if (S.detailTab === "via") return renderVias(body, r);
  if (!ch) { body.innerHTML = `<p class="err">${esc(r.error || "no channel")}</p>`; return; }
  const src = ch.field_share > 0 ? `Field solver for ${Math.round(ch.field_share * 100)} % of the length, formulas elsewhere` : "Formulas (fast analysis)";
  const head = `<div class="dstats"><span>${ch.diff ? "differential, reference " + fmt(2 * ch.z_ref, 0) + " Ω" : "reference " + fmt(ch.z_ref, 0) + " Ω"}</span><span>Path <b>${fmt(ch.length_mm, 1)} mm</b></span><span>Delay <b>${fmt(ch.delay_ps, 0)} ps</b></span><span>Vias <b>${ch.vias}</b></span></div>`;
  const warn = (ch.warnings || []).map((w) => `<div class="warn">${esc(w)}</div>`).join("");
  if (S.detailTab === "loss") {
    const marks = ch.il_marks.map(([f, d]) => `<span>${fmt(f, f < 10 ? 1 : 0)} GHz <b>${fmt(d, 2)} dB</b></span>`).join("");
    const lpi = ch.loss_db_per_in_at.map(([f, c, d]) => `<tr><td>${fmt(f, 0)} GHz</td><td>${fmt(c, 3)}</td><td>${fmt(d, 3)}</td><td>${fmt(c + d, 3)}</td></tr>`).join("");
    body.innerHTML = `${head}
      <div class="clegend"><span><i style="background:var(--pw-use)"></i>${ch.diff ? "Sdd21" : "S21"} insertion loss</span><span><i style="background:var(--pw-loss)"></i>${ch.diff ? "Sdd11" : "S11"} return loss</span></div>
      <div class="chart"><canvas id="ch-s" aria-label="S-parameters over frequency"></canvas></div>
      <div class="marks">${marks}</div>
      <div class="tablewrap"><table class="xs-table"><thead><tr><th>Line loss</th><th>conductor dB/in</th><th>dielectric dB/in</th><th>total</th></tr></thead><tbody>${lpi}</tbody></table></div>
      <div class="dstats"><span class="muted">${esc(src)}</span><button class="btn small" id="ts-copy">Copy Touchstone</button></div>${warn}`;
    const fg = ch.freq.map((f) => f / 1e9);
    drawLines($("ch-s"), { x: fg, xUnit: "GHz", yUnit: "dB", series: [
      { y: ch.s21_db, color: "--pw-use", name: ch.diff ? "Sdd21" : "S21" },
      { y: ch.s11_db, color: "--pw-loss", name: ch.diff ? "Sdd11" : "S11" }], yMin: Math.max(-60, Math.min(...ch.s11_db, ...ch.s21_db) - 2), yMax: 0 });
    $("ts-copy").onclick = (e) => {
      navigator.clipboard.writeText(r.touchstone).then(() => { e.target.textContent = "Copied (.s2p)"; }).catch(() => { e.target.textContent = "Copying not allowed"; });
    };
    return;
  }
  if (S.detailTab === "tdr") {
    const scale = ch.diff ? 2 : 1;
    const inLine = ch.tdr_z.filter((_, i) => ch.tdr_x_mm[i] <= ch.length_mm);
    const zmin = Math.min(...inLine) * scale, zmax = Math.max(...inLine) * scale;
    body.innerHTML = `${head}
      <div class="clegend"><span><i style="background:var(--pw-use)"></i>Impedance from the step response (${fmt(rfParams().rise * 1e12, 0)} ps rise)</span>${t ? `<span><i style="background:${S.colors.ok}"></i>target ±5 %</span>` : ""}</div>
      <div class="chart"><canvas id="ch-tdr" aria-label="TDR impedance over position"></canvas></div>
      <div class="marks"><span>${ch.diff ? "Zdiff" : "Z"} min <b>${fmt(zmin, 1)} Ω</b></span><span>${ch.diff ? "Zdiff" : "Z"} max <b>${fmt(zmax, 1)} Ω</b></span><span class="muted">Position from the mean delay; after the end of the line comes the termination at the reference value.</span></div>${warn}`;
    const zt = t ? (t.diff ? t.z / 2 : t.z) : null;
    drawLines($("ch-tdr"), { x: ch.tdr_x_mm, xUnit: "mm", yUnit: "Ω", series: [{ y: ch.tdr_z.map((z) => z * scale), color: "--pw-use", name: ch.diff ? "Z diff" : "Z" }],
      band: zt ? [zt * scale * 0.95, zt * scale * 1.05] : null, xMax: ch.length_mm * 1.25 });
  }
}

function renderVias(body, r) {
  const ch = r.channel;
  if (!ch) { body.innerHTML = `<p class="err">${esc(r.error || "no channel")}</p>`; return; }
  const L = (k) => esc(S.scene.stackup.copper[k]?.name ?? k);
  const ret = { none: "not needed", pair: "via partner via", stitch: "stitching via", decap: "via capacitors", missing: "missing" };
  const fmax = rfParams().fmax / 1e9;
  const rows = ch.via_info.map((v, i) => {
    const res = v.stub_f_ghz != null && v.stub_f_ghz < fmax;
    return `<tr class="click" data-via="${i}"><td>${L(v.layer_in)} → ${L(v.layer_out)}</td><td>${fmt(v.drill_mm * 1000, 0)} / ${fmt(v.pad_mm * 1000, 0)}</td><td>${v.antipad_mm != null ? fmt(v.antipad_mm * 1000, 0) : "–"}</td><td>${fmt(v.z_via, 0)}</td><td>${fmt(v.barrel_mm, 2)}</td><td${res ? ' class="warn"' : ""}>${fmt(v.stub_mm, 2)}${v.stub_f_ghz != null && v.stub_mm > 0.01 ? ` (${fmt(v.stub_f_ghz, v.stub_f_ghz < 100 ? 1 : 0)} GHz)` : ""}${v.backdrilled ? " bd" : ""}</td><td>${ret[v.return_kind] || v.return_kind}${v.return_kind === "stitch" ? ` ${fmt(v.return_mm, 1)} mm` : ""}</td></tr>`;
  }).join("");
  body.innerHTML = ch.via_info.length ? `<div class="dstats"><span>${ch.via_info.length} Via${ch.via_info.length > 1 ? "s" : ""} in path</span><span class="muted">Z ${ch.diff ? "differential" : "single-ended"}, target ${fmt(ch.diff ? 2 * ch.z_ref : ch.z_ref, 0)} Ω</span></div>
    <div class="tablewrap"><table class="xs-table"><thead><tr><th>Layers</th><th>drill/pad µm</th><th>antipad µm</th><th>Z Ω</th><th>barrel mm</th><th>stub mm</th><th>return path</th></tr></thead><tbody>${rows}</tbody></table></div>
    <p class="note">Antipad = mean clearance in the planes the via passes through. A stub (unused via end) acts as an open line; its quarter-wave resonance causes a dip in S21. “bd” = back-drilled. Click a row to show the via on the board.</p>`
    : `<p class="note">The computed path does not change layers.</p>`;
  body.querySelectorAll("tr[data-via]").forEach((tr) => tr.onclick = () => {
    const v = ch.via_info[+tr.dataset.via];
    zoomTo(v.at[0], v.at[1], 3);
  });
}

function renderXt(body, r, net) {
  const x = r.crosstalk, rise = fmt(x.rise_ps, 0);
  const rows = x.aggressors.slice(0, 20).map((a) => `<tr class="click" data-net="${a.net}"><td>${esc(a.name.split("/").pop() || a.name)}</td><td>${esc(S.scene.stackup.copper[a.layer].name)}</td><td>${fmt(a.coupled_mm, 1)}</td><td>${fmt(a.gap_min_mm * 1000, 0)}</td><td>${fmt(a.next * 100, 2)}</td><td>${fmt(a.fext * 100, 2)}</td><td>${a.source === "field" ? "field" : "formula"}</td></tr>`).join("");
  const open = x.geoms.filter((g) => !S.field.has(g[0])).length;
  body.innerHTML = `<div class="dstats"><span>NEXT total <b>${fmt(x.next_total * 100, 2)} %</b></span><span>FEXT total <b>${fmt(x.fext_total * 100, 2)} %</b></span><span class="muted">at ${rise} ps rise, in % of the aggressor amplitude</span></div>
    ${rows ? `<div class="tablewrap"><table class="xs-table"><thead><tr><th>Aggressor</th><th>Layer</th><th>parallel mm</th><th>gap µm</th><th>NEXT %</th><th>FEXT %</th><th>Source</th></tr></thead><tbody>${rows}</tbody></table></div>` : `<p class="note">No parallel signal lines within coupling range (up to 5 h beside the line).</p>`}
    <p class="note">Computed per line against the nearest neighbour on the same layer; power nets are ignored. NEXT saturates beyond half the rise length, FEXT grows with length.${open ? ` ${open} coupled cross-sections can be computed more accurately with the field solver (“selected net”).` : ""}</p>`;
  body.querySelectorAll("tr[data-net]").forEach((tr) => tr.onclick = () => selectNet(+tr.dataset.net));
}

// line chart with crosshair tooltip

function drawProfile(net, t) {
  const c = $("profile"), d = window.devicePixelRatio || 1, r = c.getBoundingClientRect();
  c.width = r.width * d; c.height = r.height * d;
  const g = c.getContext("2d"); g.setTransform(d, 0, 0, d, 0, 0);
  const segs = netOrder(net);
  if (!segs.length) return;
  const vals = segs.map((s) => { const xs = S.an.cross_sections[S.pieces.xs[s.i]]; const z = comparedZ(xs, t); return z ? z.v : null; });
  const total = segs[segs.length - 1].s1;
  let lo = Infinity, hi = -Infinity;
  for (const v of vals) if (v != null) { lo = Math.min(lo, v); hi = Math.max(hi, v); }
  if (t) { lo = Math.min(lo, t.z * 0.9); hi = Math.max(hi, t.z * 1.1); }
  if (!isFinite(lo)) { lo = 0; hi = 100; }
  const padL = 40, padR = 8, padT = 8, padB = 20, W = r.width - padL - padR, H = r.height - padT - padB;
  const X = (s) => padL + s / total * W, Y = (v) => padT + (1 - (v - lo) / (hi - lo || 1)) * H;
  const cs = getComputedStyle(document.documentElement);
  const muted = cs.getPropertyValue("--muted").trim(), line = cs.getPropertyValue("--line").trim();
  g.font = "10.5px " + cs.getPropertyValue("--font-mono"); g.fillStyle = muted; g.strokeStyle = line; g.lineWidth = 1;
  // grid & ticks
  const step = niceStep((hi - lo) / 4);
  for (let v = Math.ceil(lo / step) * step; v <= hi; v += step) {
    g.beginPath(); g.moveTo(padL, Y(v)); g.lineTo(padL + W, Y(v)); g.stroke();
    g.textAlign = "right"; g.textBaseline = "middle"; g.fillText(Math.round(v), padL - 6, Y(v));
  }
  const sx = niceStep(total / 5);
  g.textAlign = "center"; g.textBaseline = "top";
  for (let s = 0; s <= total + 1e-9; s += sx) g.fillText(fmt(s, sx < 1 ? 1 : 0), X(s), padT + H + 5);
  g.textAlign = "left"; g.fillText("mm", padL + W - 14, padT + H + 5);
  if (t) {
    g.fillStyle = S.colors.ok; g.globalAlpha = 0.14;
    g.fillRect(padL, Y(t.z * 1.05), W, Y(t.z * 0.95) - Y(t.z * 1.05)); g.globalAlpha = 1;
    g.strokeStyle = S.colors.ok; g.setLineDash([4, 3]); g.beginPath(); g.moveTo(padL, Y(t.z)); g.lineTo(padL + W, Y(t.z)); g.stroke(); g.setLineDash([]);
  }
  g.lineWidth = 2.5; g.lineCap = "butt";
  segs.forEach((s, k) => {
    const v = vals[k]; if (v == null) return;
    g.strokeStyle = pieceColor(s.i);
    g.beginPath(); g.moveTo(X(s.s0), Y(v)); g.lineTo(X(s.s1), Y(v)); g.stroke();
  });
}

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
  const t = classTarget(net), p = rfParams(), q = pathReq(net);
  const key = `${net}|${S.anGen}|${JSON.stringify(p)}|${JSON.stringify(q)}`;
  if (S.rfCache && S.rfCache.key === key) return S.rfCache.res;
  let res;
  try { res = S.engine.rf({ net, rf: p, target: t ? [t.z, t.diff] : null, flip: q.flip, hint: q.hint, mixed: true }); }
  catch (e) { res = { channel: null, error: e.message, crosstalk: { aggressors: [], geoms: [] }, touchstone: "" }; }
  S.rfCache = { key, res, net };
  return res;
}
// shown instead of results that need step 2
function needEstimate(body) {
  body.innerHTML = `<p class="note">This needs the impedance estimate (step 2 in the “Signals” card).</p><button class="btn small primary" id="need-an">Estimate impedances</button>`;
  $("need-an").onclick = () => estimateImpedances();
}
function renderTab(net, t, rowsXs) {
  const body = $("tab-body");
  if (!body) return;
  if (S.an.skeleton) return needEstimate(body);
  if (S.detailTab === "z") {
    const pc = !!(t && t.diff && S.pairMode === "c");
    body.innerHTML = `<div class="chart"><canvas id="profile" aria-label="Impedance profile along the net, from A to B"></canvas></div>
      <div class="tablewrap"><table class="xs-table"><thead><tr><th>mm</th><th>Layer</th><th>Type</th><th>w µm</th><th>gap µm</th><th>Z₀</th><th>${pc ? "Z comm" : "Z diff"}</th><th>compared</th></tr></thead><tbody>${rowsXs}</tbody></table></div>
      ${pc ? `<p class="note">Common mode: Zcommon = Zeven/2 of each cross-section (field solver, or estimated from the formulas as Z₀²/Zodd/2).</p>` : ""}`;
    drawProfile(net, t);
    return;
  }
  const r = rfFor(net), ch = r.channel;
  if (S.detailTab === "xt") return renderXt(body, r, net);
  if (S.detailTab === "via") return renderVias(body, r);
  if (!ch) { body.innerHTML = `<p class="err">${esc(r.error || "no channel")}</p>`; return; }
  const src = ch.field_share > 0 ? `Field solver for ${Math.round(ch.field_share * 100)} % of the length, formulas elsewhere` : "Formulas (fast analysis)";
  // differential pair: 4-port model (both lines on their own paths, odd and even mode); the mode
  // switch in the header chooses differential or common mode for every tab
  const mx = ch.diff && r.mixed && !r.mixed.error ? r.mixed : null;
  const comm = !!(mx && S.pairMode === "c");
  if (S.detailTab === "skew") return mx ? renderSkew(body, r, net) : (body.innerHTML = `<p class="err">${esc((r.mixed && r.mixed.error) || "no 4-port model")}</p>`);
  const ref = !ch.diff ? `reference ${fmt(ch.z_ref, 0)} Ω` : comm ? `common mode, reference ${fmt(ch.z_ref / 2, 1)} Ω` : `differential, reference ${fmt(2 * ch.z_ref, 0)} Ω`;
  const delay = mx ? (comm ? mx.delay_comm_ps : mx.delay_diff_ps) : ch.delay_ps;
  const head = `<div class="dstats"><span>${ref}</span><span>Path <b>${fmt(ch.length_mm, 1)} mm</b> incl. vias</span><span>Delay <b>${fmt(delay, 0)} ps</b></span><span>Vias <b>${ch.vias}</b></span></div>`;
  const warn = (ch.warnings || []).concat(mx ? (mx.warnings || []).filter((x) => !(ch.warnings || []).includes(x)) : []).map((w) => `<div class="warn">${esc(w)}</div>`).join("");
  const note4 = mx ? `<p class="note">4-port model: ${esc(netShort(mx.p_net))} and ${esc(netShort(mx.n_net))} on their own paths, coupled sections from odd and even mode, via pairs with the partner via. ${esc(mx.source)}. Eye and channel check use the differential odd-mode model.</p>` : "";
  // file name: the net, for a pair both nets (P first)
  const fname = (n) => n.split("/").pop().replace(/[^\w.+-]+/g, "_") || "net";
  const base = mx ? `${fname(mx.p_net)}_${fname(mx.n_net)}` : fname(S.scene.nets[net]);
  if (S.detailTab === "loss") {
    // the same view for single lines and pairs: S21 = S12, S11 from A, S22 from B
    const m = comm ? "cc" : ch.diff ? "dd" : "";
    const sx = mx ? (comm ? [mx.scc21_db, mx.scc11_db, mx.scc22_db] : [mx.sdd21_db, mx.sdd11_db, mx.sdd22_db]) : [ch.s21_db, ch.s11_db, ch.s22_db];
    const fr = mx ? mx.freq : ch.freq;
    const at = (arr, g) => { const k = fr.findIndex((f) => f >= g * 1e9 - 1); return k < 0 ? null : arr[k]; };
    const marks = ch.il_marks.map(([f]) => [f, at(sx[0], f)]).filter((x) => x[1] != null).map(([f, d]) => `<span>${fmt(f, f < 10 ? 1 : 0)} GHz <b>${fmt(d, 2)} dB</b></span>`).join("");
    const lpi = ch.loss_db_per_in_at.map(([f, c, d]) => `<tr><td>${fmt(f, 0)} GHz</td><td>${fmt(c, 3)}</td><td>${fmt(d, 3)}</td><td>${fmt(c + d, 3)}</td></tr>`).join("");
    const pa = ch.path;
    const a = pa ? ` from A (${esc(pa.start.label)})` : " from port 1", bEnd = pa ? ` from B (${esc(pa.end.label)})` : " from port 2";
    body.innerHTML = `${head}
      <div class="clegend"><span><i style="background:var(--pw-use)"></i>S${m}21 = S${m}12 insertion loss</span><span><i style="background:var(--pw-loss)"></i>S${m}11 return loss${a}</span><span><i class="dash" style="border-color:var(--pw-loss)"></i>S${m}22 return loss${bEnd}</span></div>
      <div class="chart"><canvas id="ch-s" aria-label="S-parameters over frequency"></canvas></div>
      <div class="marks">${marks}</div>
      <div class="tablewrap"><table class="xs-table"><thead><tr><th>Line loss${ch.diff ? " (differential)" : ""}</th><th>conductor dB/in</th><th>dielectric dB/in</th><th>total</th></tr></thead><tbody>${lpi}</tbody></table></div>
      ${note4}
      <div class="dstats"><span class="muted">${esc(src)}</span></div>
      <details class="ts-export" id="ts-export"${S.tsOpen ? " open" : ""}><summary>Export S-parameters (Touchstone)</summary>${exportBlock("s2p", r, base)}${mx ? exportBlock("s4p", r, base) : ""}</details>${warn}`;
    drawLines($("ch-s"), { x: fr.map((f) => f / 1e9), xUnit: "GHz", yUnit: "dB", series: [
      { y: sx[0], color: "--pw-use", name: `S${m}21 = S${m}12` },
      { y: sx[1], color: "--pw-loss", name: `S${m}11 (A)` },
      { y: sx[2], color: "--pw-loss", name: `S${m}22 (B)`, dash: [5, 3] }], yMin: Math.max(-60, Math.min(...sx[0], ...sx[1], ...sx[2]) - 2), yMax: 0 });
    // export: a folding section with one block per file (ports, data order, download)
    $("ts-export").ontoggle = (e) => { S.tsOpen = e.target.open; };
    body.querySelectorAll("[data-dl]").forEach((b) => b.onclick = () => saveText(`${base}.${b.dataset.dl}`, b.dataset.dl === "s4p" ? r.touchstone4 : r.touchstone));
    return;
  }
  if (S.detailTab === "tdr") {
    // pairs: differential or common-mode trace of the 4-port model
    const scale = ch.diff && !mx ? 2 : 1;
    const tx = mx ? (comm ? mx.tdr_x_comm_mm : mx.tdr_x_diff_mm) : ch.tdr_x_mm;
    const tz = mx ? (comm ? mx.tdr_zcomm : mx.tdr_zdiff) : ch.tdr_z;
    const end = mx ? mx.tdr_end_mm : (ch.tdr_end_mm ?? ch.length_mm);
    const inLine = tz.filter((_, i) => tx[i] <= end);
    const zmin = Math.min(...inLine) * scale, zmax = Math.max(...inLine) * scale;
    const zr = comm ? ch.z_ref / 2 : ch.z_ref * (ch.diff ? 2 : 1);
    const zn = comm ? "Zcommon" : ch.diff ? "Zdiff" : "Z";
    body.innerHTML = `${head}
      <div class="clegend"><span><i style="background:var(--pw-use)"></i>${zn} from the step response (${fmt(rfParams().rise * 1e12, 0)} ps rise), seen from A</span>${t && !comm ? `<span><i style="background:${S.colors.ok}"></i>target ±5 %</span>` : ""}<span><i style="background:var(--muted);opacity:.35"></i>after B: termination ${fmt(zr, comm ? 1 : 0)} Ω</span></div>
      <div class="chart"><canvas id="ch-tdr" aria-label="TDR impedance over position, from A to B"></canvas></div>
      <div class="marks"><span>${zn} min <b>${fmt(zmin, 1)} Ω</b></span><span>${zn} max <b>${fmt(zmax, 1)} Ω</b></span><span class="muted">Position from the delay of every section (lines and vias), so it matches the board; the rise time (${fmt(rfParams().rise * 1e12, 0)} ps) blurs edges over about ${fmt(rfParams().rise * 1e12 * 0.075, 1)} mm. ▲ at the bottom: vias.${comm ? " Common mode: both lines driven in phase (Zcommon = Zeven/2)." : ""}</span></div>${note4}${warn}`;
    const zt = t && !comm ? (t.diff ? t.z / 2 : t.z) : null;
    const zs = ch.diff ? 2 : 1;
    const pa = ch.path;
    const vias = ch.segments.filter((x) => x.kind === "via").map((x) => ({ x: x.s_mm, kind: "via" }));
    drawLines($("ch-tdr"), { x: tx, xUnit: "mm", yUnit: "Ω", series: [{ y: tz.map((z) => z * scale), color: "--pw-use", name: zn }],
      band: zt ? [zt * zs * 0.95, zt * zs * 1.05] : null, xMax: end * 1.25,
      ends: pa ? { a: pa.start.label, b: pa.end.label, at: end } : null, marks: pathMarks(pa).concat(vias),
      vline: { x: end, label: "end" + (pa ? " · " + pa.end.label : "") }, shadeFrom: { x: end, label: `${fmt(zr, comm ? 1 : 0)} Ω` },
      tipExtra: (x) => segmentAt(ch, x, end), onHover: (x) => setPathCursor(net, x) });
  }
}

// what a Touchstone file contains: ports, reference, data order, and its download button
function exportBlock(kind, r, base) {
  const ch = r.channel, mx = ch.diff && r.mixed && !r.mixed.error ? r.mixed : null, pa = ch.path;
  const nm = (n) => esc(netShort(n));
  let title, rows, order;
  if (kind === "s4p") {
    title = `single-ended 4-port of the pair, reference ${fmt(mx.z_ref, 0)} Ω at every port`;
    rows = [[1, "P at A", nm(mx.p_net), mx.p_a], [2, "N at A", nm(mx.n_net), mx.n_a], [3, "P at B", nm(mx.p_net), mx.p_b], [4, "N at B", nm(mx.n_net), mx.n_b]];
    order = `Contains both lines with their coupling: differential and common mode, mode conversion (Scd, Sdc) and the skew between P and N (${fmt(mx.skew_ps, 1)} ps). Four lines per frequency, line i = Si1 Si2 Si3 Si4 (real, imaginary); mixed mode with this order: Sdd21 = (S31 − S32 − S41 + S42)/2.`;
  } else if (mx) {
    title = `differential 2-port (Sdd of the 4-port), reference ${fmt(2 * mx.z_ref, 0)} Ω differential`;
    rows = [[1, "A", `${nm(mx.p_net)} / ${nm(mx.n_net)}`, `${mx.p_a} / ${mx.n_a}`], [2, "B", `${nm(mx.p_net)} / ${nm(mx.n_net)}`, `${mx.p_b} / ${mx.n_b}`]];
    order = "Contains the differential mode only: Sdd11 Sdd21 Sdd12 Sdd22 per frequency (real, imaginary).";
  } else {
    title = `2-port, single-ended, reference ${fmt(ch.z_ref, 0)} Ω`;
    rows = [[1, "A", nm(ch.name), pa ? pa.start.label : "start"], [2, "B", nm(ch.name), pa ? pa.end.label : "end"]];
    order = "S11 S21 S12 S22 per frequency (real, imaginary); S12 = S21.";
  }
  const file = `${base}.${kind}`;
  return `<div class="ts-block"><div class="ts-title"><b>${esc(file)}</b> · ${title}</div>
    <table class="xs-table"><thead><tr><th>Port</th><th>Where</th><th>Net</th><th>Pad</th></tr></thead><tbody>${rows.map(([n, w, net, pad]) => `<tr><td><b>${n}</b></td><td>${w}</td><td>${net}</td><td>${esc(pad)}</td></tr>`).join("")}</tbody></table>
    <p class="note">${order} The port list is also written as comments at the top of the file.</p>
    <button class="btn small" data-dl="${kind}">Download ${esc(file)}</button></div>`;
}

// skew and mode conversion of a pair: where P and N differ, conversion into common mode,
// the TDR of each line alone
function renderSkew(body, r, net) {
  const ch = r.channel, mx = r.mixed, pa = ch.path;
  const P = netShort(mx.p_net), N = netShort(mx.n_net);
  const w = mx.scd21_worst;
  const rows = mx.imbalance.map((x, i) => `<tr class="click" data-im="${i}"><td>${x.s_to_mm - x.s_from_mm < 0.05 ? "at " + fmt(x.s_from_mm, 1) : fmt(x.s_from_mm, 1) + " … " + fmt(x.s_to_mm, 1)}</td><td>${x.dl_mm >= 0 ? esc(P) : esc(N)} <b>+${fmt(Math.abs(x.dl_mm), 2)} mm</b></td><td>${x.dt_ps >= 0 ? "+" : "−"}${fmt(Math.abs(x.dt_ps), 2)} ps</td><td>${x.vias_differ ? "vias differ" : ""}</td></tr>`).join("");
  body.innerHTML = `<div class="dstats"><span>Skew <b>${fmt(mx.skew_ps, 1)} ps</b> (${fmt(mx.skew_mm, 2)} mm, ${esc(P)} − ${esc(N)})</span><span>${esc(P)} ${fmt(mx.length_mm[0], 2)} mm / ${fmt(mx.delay_ps[0], 1)} ps</span><span>${esc(N)} ${fmt(mx.length_mm[1], 2)} mm / ${fmt(mx.delay_ps[1], 1)} ps</span><span>Coupled <b>${fmt(mx.coupled_share * 100, 0)} %</b></span></div>
    <h4 class="sub">Where ${esc(P)} and ${esc(N)} differ</h4>
    ${rows ? `<div class="tablewrap"><table class="xs-table"><thead><tr><th>mm from A (along ${esc(P)})</th><th>longer</th><th>delay ${esc(P)} − ${esc(N)}</th><th></th></tr></thead><tbody>${rows}</tbody></table></div><p class="note">Stretches between the coupled runs where the lines are not equal; click to show the place on the board. Together they make up the skew.</p>` : `<p class="note">No stretch where the lines differ by more than 0.05 mm.</p>`}
    <h4 class="sub">Mode conversion</h4>
    <div class="clegend"><span><i style="background:var(--pw-conv)"></i>Scd21: differential in at A, common mode out at B</span><span><i class="dash" style="border-color:var(--pw-conv)"></i>Scd11: common mode back at A</span></div>
    <div class="chart"><canvas id="ch-scd" aria-label="Mode conversion over frequency"></canvas></div>
    <div class="marks"><span>Scd21 max <b>${fmt(w[0], 1)} dB</b> at ${fmt(w[1] / 1e9, 1)} GHz</span><span class="muted">The share of the differential signal turned into common mode by unequal lengths, vias or coupling: it is lost to the receiver and radiates.</span></div>
    <h4 class="sub">TDR of each line alone</h4>
    <div class="clegend"><span><i style="background:var(--pw-use)"></i>${esc(P)}</span><span><i class="dash" style="border-color:var(--pw-loss)"></i>${esc(N)}</span><span class="muted">one line driven, the other terminated in ${fmt(mx.z_ref, 0)} Ω</span></div>
    <div class="chart"><canvas id="ch-tdr-pn" aria-label="TDR of P and N over position"></canvas></div>`;
  drawLines($("ch-scd"), { x: mx.freq.map((f) => f / 1e9), xUnit: "GHz", yUnit: "dB", series: [
    { y: mx.scd21_db, color: "--pw-conv", name: "Scd21" }, { y: mx.scd11_db, color: "--pw-conv", name: "Scd11", dash: [5, 3] }],
    yMin: Math.max(-80, Math.min(...mx.scd21_db, ...mx.scd11_db) - 2), yMax: Math.min(0, Math.max(...mx.scd21_db, ...mx.scd11_db) + 6) });
  // both lines on one position axis: N resampled onto P's TDR positions
  const end = Math.max(mx.length_mm[0], mx.length_mm[1]);
  const zn = mx.tdr_xp_mm.map((x) => { const k = mx.tdr_xn_mm.findIndex((v) => v >= x); return k < 0 ? mx.tdr_zn[mx.tdr_zn.length - 1] : mx.tdr_zn[k]; });
  drawLines($("ch-tdr-pn"), { x: mx.tdr_xp_mm, xUnit: "mm", yUnit: "Ω", series: [{ y: mx.tdr_zp, color: "--pw-use", name: P }, { y: zn, color: "--pw-loss", name: N, dash: [5, 3] }],
    xMax: end * 1.2, ends: pa ? { a: pa.start.label, b: pa.end.label, at: mx.length_mm[0] } : null,
    marks: mx.imbalance.map((x) => ({ x: 0.5 * (x.s_from_mm + x.s_to_mm), label: `${x.dl_mm >= 0 ? P : N} +${fmt(Math.abs(x.dl_mm), 2)} mm` })) });
  body.querySelectorAll("[data-im]").forEach((tr) => tr.onclick = () => { const x = mx.imbalance[+tr.dataset.im]; S.pathCursor = { x: x.at[0], y: x.at[1] }; zoomTo(x.at[0], x.at[1], 3); });
}

// what lies at position x (mm along the path) of a channel: via, branch, line section
function segmentAt(ch, x, end) {
  if (x > end + 0.3) return `<br><span class="muted">termination (after B)</span>`;
  const L = S.scene.stackup.copper, near = 0.6;
  const v = ch.segments.find((s) => s.kind === "via" && Math.abs(s.s_mm - x) < near);
  const b = ch.segments.find((s) => s.kind === "branch" && Math.abs(s.s_mm - x) < near);
  const ln = ch.segments.find((s) => s.kind === "line" && x >= s.s_mm && x <= s.s_mm + s.len_mm);
  const out = [];
  if (v) out.push(`via${v.z ? " " + fmt(v.z * (ch.diff ? 2 : 1), 0) + " Ω" : ""}${v.via_lc ? ` (${fmt(v.via_lc[0], 2)} nH, ${fmt(v.via_lc[1], 2)} pF)` : ""}`);
  if (b) out.push(esc(b.part || "branch"));
  if (ln) {
    const xs = ln.xs != null ? S.an.cross_sections[ln.xs] : null;
    out.push(`${esc(L[ln.layer] ? L[ln.layer].name : "")}${xs ? ` · ${fmt(xs.w * 1000, 0)} µm` : ""}${ln.z ? ` · line ${fmt(ln.z * (ch.diff ? 2 : 1), 1)} Ω` : ""}`);
  }
  return out.length ? `<br><span class="muted">${out.join("<br>")}</span>` : "";
}

// a text file for the user to save (Touchstone, …)
function saveText(name, text) {
  const a = document.createElement("a");
  a.href = URL.createObjectURL(new Blob([text], { type: "text/plain" }));
  a.download = name;
  document.body.appendChild(a); a.click(); a.remove();
  setTimeout(() => URL.revokeObjectURL(a.href), 1000);
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
  const g = c.getContext("2d");
  const p = netPath(net);
  if (!p || !p.pieces.length) return;
  // pairs in common mode: Zcommon = Zeven/2 of each cross-section, no target
  const comm = !!(t && t.diff && S.pairMode === "c" && S.an.nets.find((x) => x.net === net)?.partner != null);
  const zOf = (i) => { const xs = S.an.cross_sections[S.pieces.xs[i]]; if (comm) return xs.zeven != null ? xs.zeven / 2 : null; const z = comparedZ(xs, t); return z ? z.v : null; };
  if (comm) t = null;
  const segs = p.pieces.map((x) => ({ i: x.piece, s0: x.s0, s1: x.s1, v: zOf(x.piece) }));
  // branches: their own impedance from the junction on (thin, dashed)
  const bsegs = p.branches.flatMap((b) => b.pieces.map((x) => ({ i: x.piece, s0: b.at_s + x.s0, s1: b.at_s + x.s1, v: zOf(x.piece) })));
  const total = Math.max(p.length_mm, ...bsegs.map((x) => x.s1), 1e-6);
  let lo = Infinity, hi = -Infinity;
  for (const x of segs.concat(bsegs)) if (x.v != null) { lo = Math.min(lo, x.v); hi = Math.max(hi, x.v); }
  if (t) { lo = Math.min(lo, t.z * 0.9); hi = Math.max(hi, t.z * 1.1); }
  if (!isFinite(lo)) { lo = 0; hi = 100; }
  const padL = 40, padR = 8, padT = 8, padB = 34, W = r.width - padL - padR, H = r.height - padT - padB;
  const X = (s) => padL + s / total * W, Y = (v) => padT + (1 - (v - lo) / (hi - lo || 1)) * H;
  const cs = getComputedStyle(document.documentElement);
  const muted = cs.getPropertyValue("--muted").trim(), line = cs.getPropertyValue("--line").trim();
  const paint = (cur) => {
    g.setTransform(d, 0, 0, d, 0, 0); g.clearRect(0, 0, r.width, r.height);
    g.font = "10.5px " + cs.getPropertyValue("--font-mono"); g.fillStyle = muted; g.strokeStyle = line; g.lineWidth = 1;
    const step = niceStep((hi - lo) / 4);
    for (let v = Math.ceil(lo / step) * step; v <= hi; v += step) {
      g.beginPath(); g.moveTo(padL, Y(v)); g.lineTo(padL + W, Y(v)); g.stroke();
      g.textAlign = "right"; g.textBaseline = "middle"; g.fillText(Math.round(v), padL - 6, Y(v));
    }
    const sx = niceStep(total / 5);
    g.textAlign = "center"; g.textBaseline = "top";
    for (let s = 0; s <= total + 1e-9; s += sx) g.fillText(fmt(s, sx < 1 ? 1 : 0), X(s), padT + H + 5);
    g.textAlign = "right"; g.fillText("mm from A", padL + W, padT + H + 5 - 12);
    if (t) {
      g.fillStyle = S.colors.ok; g.globalAlpha = 0.14;
      g.fillRect(padL, Y(t.z * 1.05), W, Y(t.z * 0.95) - Y(t.z * 1.05)); g.globalAlpha = 1;
      g.strokeStyle = S.colors.ok; g.setLineDash([4, 3]); g.beginPath(); g.moveTo(padL, Y(t.z)); g.lineTo(padL + W, Y(t.z)); g.stroke(); g.setLineDash([]);
    }
    g.lineCap = "butt";
    g.lineWidth = 1.5; g.setLineDash([3, 2]); g.globalAlpha = 0.75;
    for (const x of bsegs) { if (x.v == null) continue; g.strokeStyle = pieceColor(x.i); g.beginPath(); g.moveTo(X(x.s0), Y(x.v)); g.lineTo(X(x.s1), Y(x.v)); g.stroke(); }
    g.setLineDash([]); g.globalAlpha = 1; g.lineWidth = 2.5;
    for (const x of segs) { if (x.v == null) continue; g.strokeStyle = pieceColor(x.i); g.beginPath(); g.moveTo(X(x.s0), Y(x.v)); g.lineTo(X(x.s1), Y(x.v)); g.stroke(); }
    const vias = [];
    for (let k = 1; k < segs.length; k++) if (S.pieces.layer[segs[k].i] !== S.pieces.layer[segs[k - 1].i]) vias.push({ x: segs[k].s0, kind: "via" });
    pathDecor(g, { ends: { a: p.start.label, b: p.end.label, at: p.length_mm }, marks: pathMarks(p).concat(vias), vline: total > p.length_mm + 1e-6 ? { x: p.length_mm, label: "end · " + p.end.label } : null },
      { X, padL, padT, W, H, muted, xMin: 0, xMax: total });
    if (cur != null) { g.strokeStyle = muted; g.setLineDash([3, 3]); g.beginPath(); g.moveTo(X(cur), padT); g.lineTo(X(cur), padT + H); g.stroke(); g.setLineDash([]); }
  };
  paint(null);
  const host = c.parentElement;
  let tip = host.querySelector(".ctip");
  if (!tip) { tip = document.createElement("div"); tip.className = "ctip"; tip.hidden = true; host.appendChild(tip); }
  c.onpointermove = (e) => {
    const px = e.offsetX;
    if (px < padL || px > padL + W) { tip.hidden = true; paint(null); setPathCursor(net, null); return; }
    const s = (px - padL) / W * total;
    const q = segs.find((x) => s >= x.s0 && s <= x.s1);
    paint(s);
    if (s <= p.length_mm) setPathCursor(net, s); else setPathCursor(net, null);
    const xs = q ? S.an.cross_sections[S.pieces.xs[q.i]] : null;
    tip.innerHTML = `${fmt(s, 2)} mm from A` + (q && q.v != null ? `<br>${fmt(q.v, 1)} Ω · ${esc(S.scene.stackup.copper[xs.layer].name)} · ${fmt(xs.w * 1000, 0)} µm` : "");
    tip.hidden = false;
    tip.style.left = Math.min(X(s) + 10, r.width - tip.offsetWidth - 4) + "px"; tip.style.top = "6px";
  };
  c.onpointerleave = () => { tip.hidden = true; paint(null); setPathCursor(net, null); };
}

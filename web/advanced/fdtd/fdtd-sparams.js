// kisi-advanced.html, 3D field view: the S-parameters and the TDR of a run (docs/reference.md "Field views"), drawn
// with kisi.html's own chart (web/src/charts.js drawLines, bundled as window.KisiCharts by tools/advanced_bundle.py)
// in the style of kisi's Loss and TDR tabs.
// * The S-parameters: the runs' broadband outgoing waves (each run is a time-domain run, so every excited port gives
//   a column of S from DC to the case's fmax), the missing ones by reciprocity (Sij = Sji); a pair's mixed mode with
//   ports 1, 2 at A and 3, 4 at B (100 Ω differential, 25 Ω common).
// * kisi's model of the same via (handed over by kisi.html with the case: rf::via_s, the via between kisi's line
//   models of the cut-out's feed lengths): dotted.
// * The TDR: the step response of S11 (Sdd11, Scc11) with a Gaussian edge of the viewer's rise time – the same
//   formula as kisi's TDR (kisi_si::rf::tdr_map).
// Plain script: KVF.sview (page) and KVF.sMatrix / KVF.modes / KVF.tdr / KVF.touchstone (also node, via vm).
(function (root) {
  "use strict";
  const KVF = (root.KVF = root.KVF || {});
  const LN2 = Math.LN2;

  // S of a data set: S(i, j) over the runs' frequencies (null when neither port i nor port j was excited)
  KVF.sMatrix = function (ds) {
    const runs = new Map(ds.runs.filter((r) => r.s && r.sfreq).map((r) => [r.port, r]));
    const r0 = runs.values().next().value;
    if (!r0) return null;
    const n = r0.s.length;
    const S = (i, j) => (runs.has(j) ? runs.get(j).s[i] : runs.has(i) && i !== j ? runs.get(i).s[j] : null);
    return { f: r0.sfreq, n, S, full: Array.from({ length: n }, (_, j) => runs.has(j)).every(Boolean), excited: [...runs.keys()] };
  };
  // 0.5·(S(i1,j1) + sj·S(i1,j2) + si·S(i2,j1) + si·sj·S(i2,j2)) per frequency
  function comb(m, i1, i2, j1, j2, si, sj) {
    const a = m.S(i1, j1), b = m.S(i1, j2), c = m.S(i2, j1), d = m.S(i2, j2);
    if (!a || !b || !c || !d) return null;
    return a.map((_, k) => [0, 1].map((q) => 0.5 * (a[k][q] + sj * b[k][q] + si * c[k][q] + si * sj * d[k][q])));
  }
  // a pair (4 ports: 1 P, 2 N at A; 3 P, 4 N at B): differential (dd), common (cc) and differential → common (cd)
  KVF.modes = (m) => ({
    dd11: comb(m, 0, 1, 0, 1, -1, -1), dd21: comb(m, 2, 3, 0, 1, -1, -1), dd22: comb(m, 2, 3, 2, 3, -1, -1),
    cc11: comb(m, 0, 1, 0, 1, 1, 1), cc21: comb(m, 2, 3, 0, 1, 1, 1), cc22: comb(m, 2, 3, 2, 3, 1, 1),
    cd21: comb(m, 2, 3, 0, 1, 1, -1),
  });
  const dB = (x) => 20 * Math.log10(Math.hypot(x[0], x[1]) || 1e-30);

  // step response of s (complex [re, im] on f_k = k·df) with a Gaussian of 10-90 % rise `rise` (s); t (ps), Z (Ω)
  KVF.tdr = function (freq, s, zref, rise, tmax, nt = 600) {
    const df = freq[0], f3 = 0.3394 / rise, s0 = s[0][0];
    const terms = [];
    for (let k = 0; k < freq.length; k++) {
      const h = Math.exp(-(LN2 / 2) * (freq[k] / f3) ** 2);
      if (h > 1e-6) terms.push([2 * Math.PI * freq[k], s[k][0] * h, s[k][1] * h]);
    }
    const F = (t) => {
      let acc = s0 * t;
      for (const [w, a, b] of terms) acc += (2 * (a * Math.sin(w * t) + b * Math.cos(w * t))) / w;
      return acc * df;
    };
    const f0 = F(-3 * rise), t = [], z = [];
    for (let i = 0; i < nt; i++) {
      const ti = (i * tmax) / nt, rho = Math.max(-0.99, Math.min(0.99, F(ti) - f0));
      t.push(ti * 1e12);
      z.push((zref * (1 + rho)) / (1 - rho));
    }
    return { t, z };
  };
  // one-way delay from the phase slope of a transmission (the lower quarter of the band), s
  function delayOf(f, s21) {
    const n = Math.max(4, Math.floor(f.length / 4));
    let ph = 0, prev = Math.atan2(s21[0][1], s21[0][0]);
    for (let k = 1; k < n; k++) {
      const p = Math.atan2(s21[k][1], s21[k][0]);
      let d = p - prev;
      while (d > Math.PI) d -= 2 * Math.PI;
      while (d < -Math.PI) d += 2 * Math.PI;
      ph += d; prev = p;
    }
    return Math.max(0, -ph / (2 * Math.PI * (f[n - 1] - f[0])));
  }
  // a complex series on another uniform grid: linear in re/im (kisi's model, 50 MHz steps, onto the run's grid)
  function onGrid(df, s, f) {
    return f.map((x) => {
      const u = x / df - 1, i = Math.floor(u), w = u - i;
      if (i < 0) return s[0];
      if (i >= s.length - 1) return null;
      return [s[i][0] * (1 - w) + s[i + 1][0] * w, s[i][1] * (1 - w) + s[i + 1][1] * w];
    });
  }

  // Touchstone (RI, 50 Ω) of a full matrix: 2-port S11 S21 S12 S22, n-port one row per line
  KVF.touchstone = function (m, head) {
    let t = head.map((h) => `! ${h}\n`).join("") + "# Hz S RI R 50\n";
    const num = (x) => `${x[0].toExponential(9)} ${x[1].toExponential(9)}`;
    for (let k = 0; k < m.f.length; k++) {
      const v = (i, j) => num(m.S(i, j)[k]);
      if (m.n === 2) t += `${m.f[k].toExponential(6)} ${v(0, 0)} ${v(1, 0)} ${v(0, 1)} ${v(1, 1)}\n`;
      else for (let i = 0; i < m.n; i++) t += (i === 0 ? m.f[k].toExponential(6) : "  ") + " " + Array.from({ length: m.n }, (_, j) => v(i, j)).join(" ") + "\n";
    }
    return t;
  };

  if (typeof document === "undefined") return;
  // ---- the page: the two sub-views -------------------------------------------------------------------------------
  const esc = (s) => String(s).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);
  const f1 = (v, d = 1) => (v == null || !isFinite(v) ? "–" : v.toFixed(d));
  const st = { pm: "d", open: false };
  KVF.SVIEW = st; // the last drawn curves, for tools/check_advanced.py

  // ctx: {ds, board: {model, rise, target, marks} | null (the hand-over of this case), fieldHz}
  function prep(ctx) {
    const ds = ctx.ds, m = ds && KVF.sMatrix(ds);
    if (!m) return null;
    const pair = m.n === 4, kase = ds.case || {}, meta = kase.meta || {};
    const names = (meta.ports || []).map((p) => p.name);
    const pads = (meta.ports || []).filter((p) => p.pad).map((p) => p.name);
    const bd = ctx.board && ctx.board.caseName === kase.name ? ctx.board : null;
    const model = bd && bd.model ? bd.model : null;
    return { ds, m, pair, names, pads, bd, model, meta };
  }
  function modeTabs(pair, el, redraw) {
    if (!pair) return "";
    setTimeout(() => el.querySelectorAll("[data-pm]").forEach((b) => (b.onclick = () => { st.pm = b.dataset.pm; redraw(); })));
    return `<div class="seg" role="group" aria-label="Mode of the pair"><button data-pm="d" class="${st.pm === "d" ? "on" : ""}">Differential</button><button data-pm="c" class="${st.pm === "c" ? "on" : ""}">Common mode</button></div>`;
  }
  const kisiCol = () => getComputedStyle(document.getElementById("kisi3d")).getPropertyValue("--z-ok").trim() || "#23a05a";

  function renderS(el, ctx) {
    const p = prep(ctx);
    if (!p) { el.innerHTML = `<p class="note">No run yet – run the case (or load a .kvf).</p>`; return; }
    const { m, pair, model, pads } = p, comm = pair && st.pm === "c", f = m.f;
    let s21, s11, s22, cd, lab, zref;
    if (pair) {
      const md = KVF.modes(m);
      [s21, s11, s22] = comm ? [md.cc21, md.cc11, md.cc22] : [md.dd21, md.dd11, md.dd22];
      cd = comm ? null : md.cd21;
      lab = comm ? "cc" : "dd";
      zref = comm ? 25 : 100;
    } else { s21 = m.S(1, 0); s11 = m.S(0, 0); s22 = m.n > 1 ? m.S(1, 1) : null; lab = ""; zref = 50; }
    // kisi's model: a single via, or the odd mode (differential) of a pair; no common mode
    const k = model && (!pair || !comm) ? model.s : null;
    const kg = k ? [0, 2, 4].map((o) => onGrid(model.df, k.map((r) => [r[o], r[o + 1]]), f)) : null;
    const series = [];
    const add = (y, color, name, dash) => { if (y) series.push({ y: y.map((x) => (x ? dB(x) : NaN)), color, name, dash }); };
    add(s21, "--pw-use", `S${lab}21 = S${lab}12 (3D)`);
    add(s11, "--pw-loss", `S${lab}11 (3D, port 1)`);
    add(s22, "--pw-loss", `S${lab}22 (3D, far end)`, [5, 3]);
    add(cd, "--pw-conv", "Scd21 (3D, differential → common)");
    if (kg) { add(kg[1], "--pw-use", `S${lab}21 kisi`, [2, 3]); add(kg[0], "--pw-loss", `S${lab}11 kisi`, [2, 3]); }
    // the chart ends where kisi's model ends (40 GHz) or at the run's top
    const top = f[f.length - 1] / 1e9;
    const ys = series.flatMap((s) => s.y.filter(isFinite));
    const fh = ctx.fieldHz;
    const marksF = [...new Set(((p.bd && p.bd.marks) || []).filter((g) => g * 1e9 <= f[f.length - 1]).concat(fh ? [fh / 1e9] : []))].sort((a, b) => a - b);
    const at = (y, g) => { if (!y) return null; let b = 0; for (let i = 0; i < f.length; i++) if (Math.abs(f[i] - g * 1e9) < Math.abs(f[b] - g * 1e9)) b = i; return y[b] ? dB(y[b]) : null; };
    const marks = marksF.map((g) => `<span>${fh && Math.abs(g * 1e9 - fh) < 1 ? "field " : ""}${f1(g, g < 10 ? 2 : 1)} GHz <b>${f1(at(s21, g), 2)} dB</b>${kg ? ` <span class="muted">kisi ${f1(at(kg[1], g), 2)} dB</span>` : ""}</span>`).join("");
    const miss = m.full ? "" : ` The far end's ${pair ? "return loss and the .s4p" : "S22 and the .s2p"} need its port${pair ? "s" : ""} excited too (Excite, then Run).`;
    const kisiNote = !model ? (p.bd ? "kisi's via model: not available for this via (it is not on its net's channel)." : "kisi's via model: only for a via kisi.html handed over (the built-in examples have none).")
      : comm ? "kisi's via model: differential mode only (the common mode of the via pair is not a model of its own in the channel)."
      : `kisi's via model (dotted): the via as kisi's channel models it (${esc(model.kind || "ladder")}, between kisi's line models of the cut-out's feed lengths, ${model.feeds_mm.map((x) => f1(x, 2)).join(" / ")} mm), 50 Ω per line, lossy with kisi's copper and dielectric.`;
    el.innerHTML = `<div class="k3-shead">${modeTabs(pair, el, () => renderS(el, ctx))}</div>
      <div class="clegend"><span><i style="background:var(--pw-use)"></i>S${lab}21 = S${lab}12 insertion loss</span><span><i style="background:var(--pw-loss)"></i>S${lab}11 return loss at port 1</span>${s22 ? `<span><i class="dash" style="border-color:var(--pw-loss)"></i>S${lab}22 return loss at the far end</span>` : ""}${cd ? `<span><i style="background:var(--pw-conv)"></i>Scd21 mode conversion</span>` : ""}${kg ? `<span><i class="dot" style="border-color:var(--pw-use)"></i>kisi's via model (dotted)</span>` : ""}</div>
      <div class="chart"><canvas id="k3-schart" aria-label="S-parameters over frequency"></canvas></div>
      <div class="marks">${marks}</div>
      <p class="note">The 3D S-parameters are those of the cut-out with its lumped 50 Ω ports${pads.length ? ` (pad ports at ${esc(pads.join(", "))})` : ""} and its feed lines – not de-embedded; reference ${zref} Ω${pair ? (comm ? " common mode" : " differential") : ""}. From the time-domain runs, DC to ${f1(top, 1)} GHz (the pulse's top). The dielectric loss is right at f<sub>loss</sub> = ${f1((p.ds.f_loss || 0) / 1e9, 2)} GHz only: the solver holds the conductivity of tanδ there, so below it the loss comes out too high (by f<sub>loss</sub>/f), above it too low – an <span class="est">estimate</span> off f<sub>loss</sub>.${miss} ${kisiNote}</p>
      <details class="ts-export" id="k3-ts"${st.open ? " open" : ""}><summary>Export S-parameters (Touchstone)</summary>${exportBlock(p)}</details>`;
    KisiCharts.colors.ok = kisiCol();
    KisiCharts.drawLines(document.getElementById("k3-schart"), { x: f.map((x) => x / 1e9), xUnit: "GHz", yUnit: "dB", series,
      yMin: Math.max(-60, Math.min(...ys) - 2), yMax: 0, vline: fh ? { x: fh / 1e9, label: "field" } : null });
    st.s = { series: series.map((s) => s.name), kisi: !!kg, full: m.full, mode: pair ? st.pm : "single", points: f.length };
    document.getElementById("k3-ts").ontoggle = (e) => { st.open = e.target.open; };
    const dl = el.querySelector("[data-k3dl]");
    if (dl) dl.onclick = () => { const t = fileText(p); st.file = t; save(t, `${(p.ds.case && p.ds.case.name) || p.ds.name}.s${m.n}p`); };
  }
  function fileText(p) {
    const m = p.m, kase = p.ds.case || {}, meta = kase.meta || {};
    return KVF.touchstone(m, [`${meta.title || kase.name || p.ds.name}: 3D (kisi-fdtd) of the cut-out with its lumped 50 Ω ports, not de-embedded`,
      `ports: ${p.names.map((n, i) => `${i + 1} = ${n}`).join(", ")}`, `kisi-advanced.html, ${new Date().toISOString().slice(0, 10)}`]);
  }
  function exportBlock(p) {
    const m = p.m, rows = p.names.map((n, i) => `<tr><td>${i + 1}</td><td>${esc(n)}</td><td>${esc(((p.meta.ports || [])[i] || {}).layer || "")}</td></tr>`).join("");
    return `<div class="ts-block"><div class="ts-title"><b>.s${m.n}p</b> – the ${m.n}-port of the cut-out, 50 Ω at every port, ${m.f.length} frequencies</div>
      <table class="k3-tbl"><tr><th>port</th><th>name</th><th>layer</th></tr>${rows}</table>
      <p class="note">${m.n === 2 ? "S11 S21 S12 S22 per frequency (real, imaginary)." : "Four lines per frequency, line i = Si1 Si2 Si3 Si4 (real, imaginary); mixed mode with this order: Sdd21 = (S31 − S32 − S41 + S42)/2."} The columns of ports that were not excited come from reciprocity (Sij = Sji).</p>
      ${m.full ? `<button class="btn" data-k3dl>Download .s${m.n}p</button>` : `<p class="note">Excite every port (one run each) for the file.</p>`}</div>`;
  }
  function save(text, name) {
    const a = document.createElement("a");
    a.href = URL.createObjectURL(new Blob([text], { type: "text/plain" }));
    a.download = name;
    document.body.appendChild(a); a.click(); a.remove();
    setTimeout(() => URL.revokeObjectURL(a.href), 5000);
  }

  function renderT(el, ctx) {
    const p = prep(ctx);
    if (!p) { el.innerHTML = `<p class="note">No run yet – run the case (or load a .kvf).</p>`; return; }
    const { m, pair, model, pads } = p, comm = pair && st.pm === "c", f = m.f;
    const md = pair ? KVF.modes(m) : null;
    const s11 = pair ? (comm ? md.cc11 : md.dd11) : m.S(0, 0), s21 = pair ? (comm ? md.cc21 : md.dd21) : m.S(1, 0);
    const zref = pair ? (comm ? 25 : 100) : 50, zn = pair ? (comm ? "Zcommon" : "Zdiff") : "Z";
    const rise = (p.bd && p.bd.rise) || 35e-12, given = !!(p.bd && p.bd.rise);
    const tau = s21 ? delayOf(f, s21) : 50e-12, tmax = Math.max(100e-12, 2.5 * 2 * tau + 4 * rise);
    const a = KVF.tdr(f, s11, zref, rise, tmax);
    const series = [{ y: a.z, color: "--pw-use", name: `${zn} (3D)` }];
    const k = model && (!pair || !comm) ? model.s.map((r) => [r[0], r[1]]) : null;
    if (k) {
      const kf = k.map((_, i) => (i + 1) * model.df);
      const b = KVF.tdr(kf, k, 50, rise, tmax);
      series.push({ y: b.z.map((z) => (pair ? 2 * z : z)), color: "--pw-use", name: `${zn} kisi`, dash: [2, 3] });
    }
    // the via region: round-trip times τ/2 … 1.5τ (the via sits about one feed's delay τ from port 1: there and back)
    const inside = a.t.map((t) => t * 1e-12 >= tau / 2 && t * 1e-12 <= 1.5 * tau);
    const zin = a.z.filter((_, i) => inside[i]);
    const zmin = zin.length ? Math.min(...zin) : null, zmax = zin.length ? Math.max(...zin) : null;
    const kz = k ? series[1].y.filter((_, i) => inside[i]) : null;
    const tg = p.bd && p.bd.target && !comm && (!!p.bd.target.diff === pair) ? p.bd.target.z : null;
    el.innerHTML = `<div class="k3-shead">${modeTabs(pair, el, () => renderT(el, ctx))}</div>
      <div class="clegend"><span><i style="background:var(--pw-use)"></i>${zn} of the 3D result from the step response (${f1(rise * 1e12, 0)} ps rise${given ? ", kisi's TDR setting" : ", default"}), seen from port 1</span>${k ? `<span><i class="dot" style="border-color:var(--pw-use)"></i>kisi's via model (dotted)</span>` : ""}${tg ? `<span><i style="background:var(--z-ok)"></i>target ±5 %</span>` : ""}</div>
      <div class="chart"><canvas id="k3-tchart" aria-label="TDR of the 3D result"></canvas></div>
      <div class="marks"><span>${zn} min <b>${f1(zmin, 1)} Ω</b></span><span>${zn} max <b>${f1(zmax, 1)} Ω</b></span>${kz && kz.length ? `<span class="muted">kisi ${f1(Math.min(...kz), 1)} … ${f1(Math.max(...kz), 1)} Ω</span>` : ""}<span class="muted">the via region: ${f1(tau * 0.5e12, 0)}–${f1(tau * 1.5e12, 0)} ps round trip (τ = ${f1(tau * 1e12, 0)} ps, from S21's phase); x is the round-trip time, not a position</span></div>
      <p class="note">The 3D TDR includes the cut-out's feed lines and its lumped 50 Ω ports${pads.length ? ` (pad ports at ${esc(pads.join(", "))})` : ""} – not de-embedded; reference ${zref} Ω. ${k ? `kisi's curve: the step response of its via model between its line models of the feed lengths (${model.feeds_mm.map((x) => f1(x, 2)).join(" / ")} mm), the same rise time.` : comm ? "kisi's via model: differential mode only." : "No kisi model for this case."}</p>`;
    KisiCharts.colors.ok = kisiCol();
    KisiCharts.drawLines(document.getElementById("k3-tchart"), { x: a.t, xUnit: "ps", yUnit: "Ω", series, band: tg ? [tg * 0.95, tg * 1.05] : null });
    st.t = { series: series.map((s) => s.name), kisi: !!k, zmin, zmax, rise, tau };
  }
  KVF.sview = { renderS, renderT };
})(typeof globalThis !== "undefined" ? globalThis : this);

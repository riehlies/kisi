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
function drawEye(c, e) {
  const d = window.devicePixelRatio || 1, r = c.getBoundingClientRect();
  c.width = r.width * d; c.height = r.height * d;
  const g = c.getContext("2d"); g.setTransform(d, 0, 0, d, 0, 0);
  const cs = getComputedStyle(document.documentElement);
  const muted = cs.getPropertyValue("--muted").trim(), line = cs.getPropertyValue("--line").trim();
  const padL = 42, padR = 10, padT = 8, padB = 20, W = r.width - padL - padR, H = r.height - padT - padB;
  // density image
  const off = document.createElement("canvas"); off.width = e.cols; off.height = e.rows;
  const og = off.getContext("2d"), img = og.createImageData(e.cols, e.rows);
  const max = Math.max(1, ...e.hist), lm = Math.log1p(max);
  const tmp = document.createElement("div"); tmp.style.color = cs.getPropertyValue("--pw-use"); document.body.appendChild(tmp);
  const rgb = getComputedStyle(tmp).color.match(/\d+/g).map(Number); tmp.remove();
  for (let i = 0; i < e.hist.length; i++) {
    const v = e.hist[i]; if (!v) continue;
    const a = 0.25 + 0.75 * Math.log1p(v) / lm;
    img.data[i * 4] = rgb[0]; img.data[i * 4 + 1] = rgb[1]; img.data[i * 4 + 2] = rgb[2]; img.data[i * 4 + 3] = Math.round(255 * a);
  }
  og.putImageData(img, 0, 0);
  g.clearRect(0, 0, r.width, r.height);
  const Y = (v) => padT + (e.v_hi - v) / (e.v_hi - e.v_lo) * H, X = (col) => padL + col / e.cols * W;
  g.font = "10.5px " + cs.getPropertyValue("--font-mono"); g.fillStyle = muted; g.strokeStyle = line; g.lineWidth = 1;
  const ys = niceStep((e.v_hi - e.v_lo) * 100 / 4);
  for (let v = Math.ceil(e.v_lo * 100 / ys) * ys; v <= e.v_hi * 100 + 1e-9; v += ys) {
    g.beginPath(); g.moveTo(padL, Y(v / 100)); g.lineTo(padL + W, Y(v / 100)); g.stroke();
    g.textAlign = "right"; g.textBaseline = "middle"; g.fillText(fmt(Math.abs(v) < 1e-9 ? 0 : v, 0), padL - 6, Y(v / 100));
  }
  g.textAlign = "center"; g.textBaseline = "top";
  for (const [col, lab] of [[0, "0"], [e.cols / 4, "½"], [e.cols / 2, "1"], [3 * e.cols / 4, "1½"], [e.cols, "2 UI"]]) g.fillText(lab, Math.min(X(col), padL + W - 14), padT + H + 5);
  g.textAlign = "left"; g.textBaseline = "top"; g.fillText("%", 2, 0);
  g.imageSmoothingEnabled = true; g.drawImage(off, padL, padT, W, H);
  // opening at the sampling phase
  if (e.height > 0) {
    const acc = cs.getPropertyValue("--fg").trim();
    g.strokeStyle = acc; g.setLineDash([4, 3]); g.lineWidth = 1;
    for (const cc of [e.cols / 4, 3 * e.cols / 4]) {
      const y0 = Y(e.threshold + e.height / 2), y1 = Y(e.threshold - e.height / 2);
      const hw = e.width_ps / e.ui_ps * e.cols / 4;
      g.strokeRect(X(cc - hw), y0, X(cc + hw) - X(cc - hw), y1 - y0);
    }
    g.setLineDash([]);
  }
}
function renderVias(body, r) {
  const ch = r.channel;
  if (!ch) { body.innerHTML = `<p class="err">${esc(r.error || "kein Kanal")}</p>`; return; }
  const L = (k) => esc(S.scene.stackup.copper[k]?.name ?? k);
  const ret = { none: "nicht nötig", pair: "über Partner-Via", stitch: "Stitching-Via", decap: "über Kondensatoren", missing: "fehlt" };
  const fmax = rfParams().fmax / 1e9;
  const rows = ch.via_info.map((v, i) => {
    const res = v.stub_f_ghz != null && v.stub_f_ghz < fmax;
    return `<tr class="click" data-via="${i}"><td>${L(v.layer_in)} → ${L(v.layer_out)}</td><td>${fmt(v.drill_mm * 1000, 0)} / ${fmt(v.pad_mm * 1000, 0)}</td><td>${v.antipad_mm != null ? fmt(v.antipad_mm * 1000, 0) : "–"}</td><td>${fmt(v.z_via, 0)}</td><td>${fmt(v.barrel_mm, 2)}</td><td${res ? ' class="warn"' : ""}>${fmt(v.stub_mm, 2)}${v.stub_f_ghz != null && v.stub_mm > 0.01 ? ` (${fmt(v.stub_f_ghz, v.stub_f_ghz < 100 ? 1 : 0)} GHz)` : ""}${v.backdrilled ? " rb" : ""}</td><td>${ret[v.return_kind] || v.return_kind}${v.return_kind === "stitch" ? ` ${fmt(v.return_mm, 1)} mm` : ""}</td></tr>`;
  }).join("");
  body.innerHTML = ch.via_info.length ? `<div class="dstats"><span>${ch.via_info.length} Via${ch.via_info.length > 1 ? "s" : ""} im Pfad</span><span class="muted">Z ${ch.diff ? "differentiell" : "einzeln"}, Ziel ${fmt(ch.diff ? 2 * ch.z_ref : ch.z_ref, 0)} Ω</span></div>
    <div class="tablewrap"><table class="xs-table"><thead><tr><th>Lagen</th><th>Bohrung/Pad µm</th><th>Antipad µm</th><th>Z Ω</th><th>Hülse mm</th><th>Stub mm</th><th>Rückstrom</th></tr></thead><tbody>${rows}</tbody></table></div>
    <p class="note">Antipad = mittlere Freistellung in den Ebenen, die das Via durchquert. Ein Stub (ungenutztes Via-Ende) wirkt als offene Leitung; seine Viertelwellen-Resonanz erzeugt einen Einbruch in S21. „rb“ = rückgebohrt. Klick auf eine Zeile zeigt das Via auf der Platine.</p>`
    : `<p class="note">Der berechnete Pfad wechselt die Lage nicht.</p>`;
  body.querySelectorAll("tr[data-via]").forEach((tr) => tr.onclick = () => {
    const v = ch.via_info[+tr.dataset.via];
    zoomTo(v.at[0], v.at[1], 3);
  });
}
function renderXt(body, r, net) {
  const x = r.crosstalk, rise = fmt(x.rise_ps, 0);
  const rows = x.aggressors.slice(0, 20).map((a) => `<tr class="click" data-net="${a.net}"><td>${esc(a.name.split("/").pop() || a.name)}</td><td>${esc(S.scene.stackup.copper[a.layer].name)}</td><td>${fmt(a.coupled_mm, 1)}</td><td>${fmt(a.gap_min_mm * 1000, 0)}</td><td>${fmt(a.next * 100, 2)}</td><td>${fmt(a.fext * 100, 2)}</td><td>${a.source === "field" ? "Feld" : "Formel"}</td></tr>`).join("");
  const open = x.geoms.filter((g) => !S.field.has(g[0])).length;
  body.innerHTML = `<div class="dstats"><span>NEXT Summe <b>${fmt(x.next_total * 100, 2)} %</b></span><span>FEXT Summe <b>${fmt(x.fext_total * 100, 2)} %</b></span><span class="muted">bei ${rise} ps Anstieg, in % der Aggressor-Amplitude</span></div>
    ${rows ? `<div class="tablewrap"><table class="xs-table"><thead><tr><th>Aggressor</th><th>Lage</th><th>parallel mm</th><th>Abstand µm</th><th>NEXT %</th><th>FEXT %</th><th>Quelle</th></tr></thead><tbody>${rows}</tbody></table></div>` : `<p class="note">Keine parallelen Signalleitungen in Kopplungsreichweite (bis 5 h neben der Leitung).</p>`}
    <p class="note">Gerechnet je Leitung gegen den nächsten Nachbarn auf derselben Lage, Versorgungsnetze zählen nicht. NEXT sättigt ab der halben Anstiegslänge, FEXT wächst mit der Länge.${open ? ` ${open} Kopplungsquerschnitte lassen sich mit dem Feldlöser („ausgewähltes Netz“) genauer rechnen.` : ""}</p>`;
  body.querySelectorAll("tr[data-net]").forEach((tr) => tr.onclick = () => selectNet(+tr.dataset.net));
}

// line chart with crosshair tooltip
function drawLines(c, o) {
  const d = window.devicePixelRatio || 1, r = c.getBoundingClientRect();
  c.width = r.width * d; c.height = r.height * d;
  const g = c.getContext("2d");
  const cs = getComputedStyle(document.documentElement);
  const col = (v) => v.startsWith("--") ? cs.getPropertyValue(v).trim() : v;
  const muted = cs.getPropertyValue("--muted").trim(), line = cs.getPropertyValue("--line").trim();
  const xs = o.x, n = xs.length;
  const xMax = o.xMax ? Math.min(o.xMax, xs[n - 1]) : xs[n - 1], xMin = o.xMin ?? 0;
  let lo = o.yMin, hi = o.yMax;
  if (lo == null || hi == null) {
    let a = Infinity, b = -Infinity;
    for (const s of o.series) s.y.forEach((v, i) => { if (xs[i] <= xMax) { a = Math.min(a, v); b = Math.max(b, v); } });
    if (o.band) { a = Math.min(a, o.band[0]); b = Math.max(b, o.band[1]); }
    const m = (b - a) * 0.1 || 1; lo = lo ?? a - m; hi = hi ?? b + m;
  }
  const padL = 42, padR = 10, padT = 8, padB = 20, W = r.width - padL - padR, H = r.height - padT - padB;
  const X = (v) => padL + (v - xMin) / (xMax - xMin || 1) * W, Y = (v) => padT + (1 - (v - lo) / (hi - lo || 1)) * H;
  const paint = (hi_i) => {
    g.setTransform(d, 0, 0, d, 0, 0); g.clearRect(0, 0, r.width, r.height);
    g.font = "10.5px " + cs.getPropertyValue("--font-mono"); g.fillStyle = muted; g.strokeStyle = line; g.lineWidth = 1;
    const ys = niceStep((hi - lo) / 4);
    for (let v = Math.ceil(lo / ys) * ys; v <= hi + 1e-9; v += ys) {
      g.beginPath(); g.moveTo(padL, Y(v)); g.lineTo(padL + W, Y(v)); g.stroke();
      g.textAlign = "right"; g.textBaseline = "middle"; g.fillText(fmt(Math.abs(v) < 1e-9 ? 0 : v, ys < 1 ? 1 : 0), padL - 6, Y(v));
    }
    const xsS = niceStep((xMax - xMin) / 5);
    g.textAlign = "center"; g.textBaseline = "top";
    for (let v = Math.ceil(xMin / xsS) * xsS; v <= xMax + 1e-9; v += xsS) g.fillText(fmt(v, xsS < 1 ? 1 : 0), X(v), padT + H + 5);
    g.textAlign = "right"; g.fillText(o.xUnit, padL + W, padT + H + 5 - 12);
    g.textAlign = "left"; g.textBaseline = "top"; g.fillText(o.yUnit, 2, 0);
    if (o.band) {
      g.fillStyle = S.colors.ok; g.globalAlpha = 0.14; g.fillRect(padL, Y(o.band[1]), W, Y(o.band[0]) - Y(o.band[1])); g.globalAlpha = 1;
    }
    g.save(); g.beginPath(); g.rect(padL, padT, W, H); g.clip();
    for (const s of o.series) {
      g.strokeStyle = col(s.color); g.lineWidth = 2; g.lineJoin = "round"; g.beginPath();
      let started = false;
      for (let i = 0; i < n; i++) { if (xs[i] > xMax * 1.001) break; const px = X(xs[i]), py = Y(s.y[i]); if (!started) { g.moveTo(px, py); started = true; } else g.lineTo(px, py); }
      g.stroke();
    }
    g.restore();
    if (hi_i != null) {
      g.strokeStyle = muted; g.setLineDash([3, 3]); g.beginPath(); g.moveTo(X(xs[hi_i]), padT); g.lineTo(X(xs[hi_i]), padT + H); g.stroke(); g.setLineDash([]);
      for (const s of o.series) { g.fillStyle = col(s.color); g.strokeStyle = cs.getPropertyValue("--panel").trim(); g.lineWidth = 2; g.beginPath(); g.arc(X(xs[hi_i]), Y(s.y[hi_i]), 4, 0, Math.PI * 2); g.fill(); g.stroke(); }
    }
  };
  paint(null);
  const host = c.parentElement;
  let tip = host.querySelector(".ctip");
  if (!tip) { tip = document.createElement("div"); tip.className = "ctip"; tip.hidden = true; host.appendChild(tip); }
  c.onpointermove = (e) => {
    const px = e.offsetX; if (px < padL || px > padL + W) { tip.hidden = true; paint(null); return; }
    const xv = xMin + (px - padL) / W * (xMax - xMin);
    let bi = 0, bd = Infinity;
    for (let i = 0; i < n; i++) { const dd = Math.abs(xs[i] - xv); if (dd < bd) { bd = dd; bi = i; } if (xs[i] > xMax) break; }
    paint(bi);
    tip.innerHTML = `${fmt(xs[bi], 2)} ${o.xUnit}<br>` + o.series.map((s) => `${esc(s.name)} ${fmt(s.y[bi], 2)} ${o.yUnit}`).join("<br>");
    tip.hidden = false;
    const tx = X(xs[bi]) + 10;
    tip.style.left = Math.min(tx, r.width - tip.offsetWidth - 4) + "px"; tip.style.top = "6px";
  };
  c.onpointerleave = () => { tip.hidden = true; paint(null); };
}

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
function niceStep(x) { const p = Math.pow(10, Math.floor(Math.log10(x || 1))); const m = x / p; return (m < 1.5 ? 1 : m < 3.5 ? 2 : m < 7.5 ? 5 : 10) * p; }

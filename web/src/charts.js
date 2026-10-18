"use strict";
// ---- charts: line chart with crosshair, eye density, log-log impedance plot
const fHz = (f) => f >= 1e9 ? fmt(f / 1e9, f < 1e10 ? 1 : 0) + " GHz" : f >= 1e6 ? fmt(f / 1e6, f < 1e7 ? 1 : 0) + " MHz" : fmt(f / 1e3, f < 1e4 ? 1 : 0) + " kHz";
const ohm = (z) => z >= 10 ? fmt(z, 0) + " Ω" : z >= 1 ? fmt(z, z % 1 ? 2 : 0) + " Ω" : fmt(z * 1e3, z < 0.01 ? 1 : 0) + " mΩ";

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

function niceStep(x) { const p = Math.pow(10, Math.floor(Math.log10(x || 1))); const m = x / p; return (m < 1.5 ? 1 : m < 3.5 ? 2 : m < 7.5 ? 5 : 10) * p; }

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
    const cav = ci >= 0 && Math.abs(cf[ci] - lv) < 0.06 ? `<br>plane model ${ohm(r.cav_z[ci])}` : "";
    tip.innerHTML = `${fHz(r.freq[bi])}<br>lumped ${ohm(r.z[bi])}${cav}<br>without C ${ohm(r.z_bare[bi])}`;
    tip.hidden = false;
    tip.style.left = Math.min(X(lf[bi]) + 10, rc.width - tip.offsetWidth - 4) + "px"; tip.style.top = "6px";
  };
  c.onpointerleave = () => { tip.hidden = true; paint(null); };
}

// kisi-advanced.html, 3D field view: drawing of a cut plane (colour map, arrows, metal/dielectric outline,
// conductor-loss overlay, colour bars, axes) and the small 3D locator. Plain script, KVF.view namespace.
(function (root) {
  "use strict";
  const KVF = (root.KVF = root.KVF || {});
  const V = (KVF.view = {});

  // ---- colour maps (perceptually uniform, readable with colour-vision deficiency) ---------------------------
  const ANCHORS = {
    // viridis (matplotlib, CC0)
    viridis: ["#440154", "#482878", "#3e4a89", "#31688e", "#26828e", "#1f9e89", "#35b779", "#6ece58", "#b5de2b", "#fde725"],
    // cividis (Nuñez et al. 2018, optimised for deuteranomaly)
    cividis: ["#00204c", "#00336f", "#39486b", "#575d6d", "#707173", "#8a8779", "#a69d75", "#c4b56c", "#e4cf5b", "#ffea46"],
    // inferno (matplotlib) for the dielectric loss
    inferno: ["#000004", "#1b0c41", "#4a0c6b", "#781c6d", "#a52c60", "#cf4446", "#ed6925", "#fb9b06", "#f7d13d", "#fcffa4"],
    // a blue-to-white ramp for the conductor estimate (lightness only)
    ice: ["#04082e", "#0b2a6b", "#1b4f9c", "#2f75bf", "#4f9bd6", "#7dbde4", "#acd8ee", "#d7eef8", "#ffffff"],
    // diverging blue – light – orange (PuOr-like, CVD-safe) for signed components
    diverging: ["#2d004b", "#542788", "#8073ac", "#b2abd2", "#d8daeb", "#f7f7f7", "#fee0b6", "#fdb863", "#e08214", "#b35806", "#7f3b08"],
  };
  const hex = (h) => [parseInt(h.slice(1, 3), 16), parseInt(h.slice(3, 5), 16), parseInt(h.slice(5, 7), 16)];
  const LUTS = {};
  V.lut = function (name) {
    if (LUTS[name]) return LUTS[name];
    const a = ANCHORS[name].map(hex), lut = new Uint8ClampedArray(256 * 3);
    for (let i = 0; i < 256; i++) {
      const t = (i / 255) * (a.length - 1), k = Math.min(a.length - 2, Math.floor(t)), f = t - k;
      for (let c = 0; c < 3; c++) lut[3 * i + c] = a[k][c] + f * (a[k + 1][c] - a[k][c]);
    }
    return (LUTS[name] = lut);
  };
  V.css = (name, t) => { const l = V.lut(name), i = Math.max(0, Math.min(255, Math.round(t * 255))); return `rgb(${l[3 * i]},${l[3 * i + 1]},${l[3 * i + 2]})`; };

  // value → [0, 1]; log: decades over `range` dB (20·log10 for fields, 10·log10 for powers handled by caller)
  V.scaler = function (opt) {
    const { max, log, rangeDb, signed, power } = opt;
    const dec = rangeDb / (power ? 10 : 20);
    if (signed) {
      return (v) => {
        const a = Math.abs(v) / max;
        const m = log ? Math.max(0, 1 + Math.log10(Math.max(a, 1e-30)) / dec) : Math.min(1, a);
        return 0.5 + 0.5 * Math.sign(v) * Math.min(1, m);
      };
    }
    return (v) => (log ? Math.max(0, Math.min(1, 1 + Math.log10(Math.max(v / max, 1e-30)) / dec)) : Math.max(0, Math.min(1, v / max)));
  };

  const fmt = (v, d) => {
    if (!isFinite(v)) return "–";
    const a = Math.abs(v);
    if (a !== 0 && (a >= 1e4 || a < 1e-2)) return v.toExponential(d == null ? 2 : d).replace("e+", "e");
    return String(Number(v.toPrecision(d == null ? 3 : d + 1)));
  };
  V.fmt = fmt;

  // ---- plane geometry: node coordinates, pixel mapping ------------------------------------------------------
  V.planeAxes = function (ds, axis) {
    const ua = axis === 0 ? 1 : 0, va = axis === 2 ? 1 : 2;
    const L = [ds.x, ds.y, ds.z];
    return { ua, va, u: L[ua], v: L[va] };
  };
  // boundaries of each node's dual cell (mm)
  const bounds = (a) => { const b = new Float64Array(a.length + 1); b[0] = a[0]; b[a.length] = a[a.length - 1]; for (let i = 1; i < a.length; i++) b[i] = 0.5 * (a[i - 1] + a[i]); return b; };
  const nearestIdx = (b, x) => { let lo = 0, hi = b.length - 1; while (hi - lo > 1) { const m = (lo + hi) >> 1; if (b[m] <= x) lo = m; else hi = m; } return lo; };
  V.nearest = (a, x) => nearestIdx(bounds(a), x);

  // layout of the canvas: plot rectangle (CSS px) with physical aspect
  V.layout = function (cssW, ds, axis, opts) {
    const { u, v } = V.planeAxes(ds, axis);
    const narrow = cssW < 560;
    const ml = narrow ? 36 : 48, mb = 30, mt = 8, mr = (opts.bars || 1) * (narrow ? 68 : 76) + 6;
    const ur = u[u.length - 1] - u[0], vr = v[v.length - 1] - v[0];
    let pw = Math.max(120, cssW - ml - mr), ph = (pw * vr) / ur;
    const maxH = opts.maxH || 560;
    if (ph > maxH) { ph = maxH; pw = (ph * ur) / vr; }
    if (ph < 120) ph = 120;
    return { ml, mt, mr, mb, pw, ph, W: cssW, H: ph + mt + mb, u0: u[0], u1: u[u.length - 1], v0: v[0], v1: v[v.length - 1] };
  };
  V.toPx = (L, uu, vv) => [L.ml + ((uu - L.u0) / (L.u1 - L.u0)) * L.pw, L.mt + L.ph - ((vv - L.v0) / (L.v1 - L.v0)) * L.ph];
  V.fromPx = (L, px, py) => [L.u0 + ((px - L.ml) / L.pw) * (L.u1 - L.u0), L.v0 + ((L.mt + L.ph - py) / L.ph) * (L.v1 - L.v0)];

  // ---- the plane ------------------------------------------------------------------------------------------
  // st: {ds, pl (KVF.plane), pv (KVF.planeValues), view, comp, scale {max, log, rangeDb, signed, power}, cmap,
  //      arrows, outline, cond (KVF.conductorLoss or null), condMax, labels {title, unit, condUnit}, theme}
  V.draw = function (cv, st) {
    const dpr = root.devicePixelRatio || 1;
    const cssW = cv.parentElement.clientWidth;
    const bars = st.cond ? 2 : 1;
    const L = V.layout(cssW, st.ds, st.pl.axis, { bars, maxH: st.maxH });
    cv.style.width = L.W + "px";
    cv.style.height = L.H + "px";
    cv.width = Math.round(L.W * dpr);
    cv.height = Math.round(L.H * dpr);
    const g = cv.getContext("2d");
    g.setTransform(dpr, 0, 0, dpr, 0, 0);
    const css = getComputedStyle(document.documentElement);
    const fg = css.getPropertyValue("--fg").trim() || "#222", muted = css.getPropertyValue("--muted").trim() || "#666";
    const bg = css.getPropertyValue("--panel").trim() || "#fff";
    g.fillStyle = bg;
    g.fillRect(0, 0, L.W, L.H);
    const { ds, pl, pv } = st;
    const { u, v } = V.planeAxes(ds, pl.axis);
    const bu = bounds(u), bv = bounds(v);
    // colour map into an image at device resolution
    const x0 = Math.round(L.ml * dpr), y0 = Math.round(L.mt * dpr), w = Math.max(1, Math.round(L.pw * dpr)), h = Math.max(1, Math.round(L.ph * dpr));
    const img = g.createImageData(w, h), px = img.data;
    const ui = new Int32Array(w), vi = new Int32Array(h);
    for (let i = 0; i < w; i++) ui[i] = nearestIdx(bu, L.u0 + ((i + 0.5) / w) * (L.u1 - L.u0));
    for (let j = 0; j < h; j++) vi[j] = nearestIdx(bv, L.v1 - ((j + 0.5) / h) * (L.v1 - L.v0));
    const lut = V.lut(st.cmap), sc = V.scaler(st.scale), nv = pl.nv;
    const colIdx = new Uint8Array(pl.nu * pl.nv);
    for (let p = 0; p < colIdx.length; p++) colIdx[p] = Math.round(sc(pv.val[p]) * 255);
    for (let j = 0; j < h; j++) {
      const vj = vi[j];
      for (let i = 0; i < w; i++) {
        const c = colIdx[ui[i] * nv + vj], o = 4 * (j * w + i);
        px[o] = lut[3 * c]; px[o + 1] = lut[3 * c + 1]; px[o + 2] = lut[3 * c + 2]; px[o + 3] = 255;
      }
    }
    g.save();
    g.setTransform(1, 0, 0, 1, 0, 0);
    g.putImageData(img, x0, y0);
    g.restore();
    g.save();
    g.beginPath();
    g.rect(L.ml, L.mt, L.pw, L.ph);
    g.clip();
    if (st.outline) V.outline(g, L, st);
    if (st.cond) V.condOverlay(g, L, st);
    if (st.arrows && pv.arrows) V.arrows(g, L, st, bu, bv);
    g.restore();
    // frame and axes
    g.strokeStyle = muted;
    g.lineWidth = 1;
    g.strokeRect(L.ml + 0.5, L.mt + 0.5, L.pw, L.ph);
    g.fillStyle = fg;
    g.font = "11px system-ui, sans-serif";
    const names = ["x", "y", "z"], { ua, va } = V.planeAxes(ds, pl.axis);
    const ticks = (a0, a1, n) => { const s = niceStep((a1 - a0) / n); const out = []; for (let t = Math.ceil(a0 / s) * s; t <= a1 + 1e-9; t += s) out.push(Math.round(t / s) * s); return out; };
    g.textAlign = "center";
    g.textBaseline = "top";
    for (const t of ticks(L.u0, L.u1, Math.max(3, Math.floor(L.pw / 70)))) {
      const [xx] = V.toPx(L, t, L.v0);
      g.fillRect(xx, L.mt + L.ph, 1, 4);
      g.fillText(fmtTick(t), xx, L.mt + L.ph + 6);
    }
    g.textAlign = "right";
    g.textBaseline = "middle";
    for (const t of ticks(L.v0, L.v1, Math.max(2, Math.floor(L.ph / 50)))) {
      const [, yy] = V.toPx(L, L.u0, t);
      g.fillRect(L.ml - 4, yy, 4, 1);
      g.fillText(fmtTick(t), L.ml - 6, yy);
    }
    g.textAlign = "left";
    g.textBaseline = "alphabetic";
    g.fillStyle = muted;
    g.fillText(`${names[ua]} (mm)`, L.ml + L.pw - 34, L.mt + L.ph + 26);
    g.save();
    g.translate(11, L.mt + 30);
    g.rotate(-Math.PI / 2);
    g.textAlign = "right";
    g.fillText(`${names[va]} (mm)`, 0, 0);
    g.restore();
    // colour bars
    V.colorbar(g, L.ml + L.pw + 12, L.mt, L.ph, st.cmap, st.scale, st.labels.unit, fg, muted);
    if (st.cond) V.colorbar(g, L.ml + L.pw + 12 + (L.W < 560 ? 68 : 76), L.mt, L.ph, "ice", { max: st.condMax, log: true, rangeDb: st.scale.rangeDb, power: true }, st.labels.condUnit, fg, muted);
    return L;
  };
  function niceStep(x) { const p = 10 ** Math.floor(Math.log10(x)), m = x / p; return (m < 1.5 ? 1 : m < 3.5 ? 2 : m < 7.5 ? 5 : 10) * p; }
  function fmtTick(t) { return Math.abs(t) < 1e-9 ? "0" : String(+t.toFixed(3)); }

  V.colorbar = function (g, x, y, h, cmap, scale, unit, fg, muted) {
    const bw = 12;
    for (let i = 0; i < h; i++) { g.fillStyle = V.css(cmap, 1 - i / h); g.fillRect(x, y + i, bw, 1.5); }
    g.strokeStyle = muted;
    g.strokeRect(x + 0.5, y + 0.5, bw, h);
    g.fillStyle = fg;
    g.font = "10px system-ui, sans-serif";
    g.textAlign = "left";
    g.textBaseline = "middle";
    const { max, log, rangeDb, signed, power } = scale;
    const lab = [];
    if (signed) {
      lab.push([0, fmt(max, 1)], [0.5, "0"], [1, fmt(-max, 1)]);
      if (log) lab.push([0.25, fmt(max * 10 ** (-rangeDb / (power ? 10 : 20) / 2), 0)], [0.75, "−" + fmt(max * 10 ** (-rangeDb / (power ? 10 : 20) / 2), 0)]);
    } else if (log) {
      const dec = rangeDb / (power ? 10 : 20);
      const n = Math.min(6, Math.max(2, Math.round(dec)));
      for (let k = 0; k <= n; k++) lab.push([k / n, fmt(max * 10 ** ((-dec * k) / n), 1)]);
    } else for (let k = 0; k <= 4; k++) lab.push([k / 4, fmt(max * (1 - k / 4), 1)]);
    for (const [t, s] of lab) { g.fillRect(x + bw, y + t * h, 3, 1); g.fillText(s, x + bw + 5, Math.min(y + h - 4, Math.max(y + 5, y + t * h))); }
    g.save();
    g.fillStyle = muted;
    g.font = "10px system-ui, sans-serif";
    g.translate(x + bw + 46, y + h);
    g.rotate(-Math.PI / 2);
    g.textAlign = "left";
    g.fillText(unit.length * 5.2 > h ? unit.slice(0, Math.floor(h / 5.2) - 1) + "…" : unit, 0, 0);
    g.restore();
  };

  // metal from the free fractions of the in-plane E edges (solid: metal, thin: partly metal on conformal
  // boundaries), metal faces filled; dielectric interfaces and the case's circles (dashed)
  V.outline = function (g, L, st) {
    const { ds, pl } = st;
    const { ua, va, u, v } = V.planeAxes(ds, pl.axis);
    const L1 = [ds.x1, ds.y1, ds.z1];
    const idx3 = (uu, vv) => { const t = [0, 0, 0]; t[pl.axis] = pl.ia; t[ua] = uu; t[va] = vv; return KVF.idx(ds, t[0], t[1], t[2]); };
    const dark = matchMedia && matchMedia("(prefers-color-scheme: dark)").matches && document.documentElement.dataset.theme !== "light";
    const metal = "rgba(235,235,235,0.95)", metalFill = "rgba(205,205,212,0.82)";
    g.lineCap = "round";
    // faces with four metal edges
    const face = new Uint8Array(pl.nu * pl.nv);
    g.fillStyle = metalFill;
    g.beginPath(); // one path, filled once: no seams where the translucent faces touch
    for (let a = 0; a < pl.nu - 1; a++) for (let b = 0; b < pl.nv - 1; b++) {
      const q = idx3(a, b);
      if (ds.free[ua][q] || ds.free[va][q] || ds.free[ua][idx3(a, b + 1)] || ds.free[va][idx3(a + 1, b)]) continue;
      face[a * pl.nv + b] = 1;
      const [x1, y1] = V.toPx(L, u[a], v[b]), [x2, y2] = V.toPx(L, L1[ua][a], L1[va][b]);
      g.rect(Math.min(x1, x2), Math.min(y1, y2), Math.abs(x2 - x1), Math.abs(y2 - y1));
    }
    g.fill("nonzero");
    const isFace = (a, b) => a >= 0 && b >= 0 && a < pl.nu && b < pl.nv && face[a * pl.nv + b] === 1;
    // metal edges, except those inside filled metal
    for (const [dir, comp] of [[0, ua], [1, va]]) {
      for (let a = 0; a < pl.nu; a++) for (let b = 0; b < pl.nv; b++) {
        if ((dir === 0 && a >= pl.nu - 1) || (dir === 1 && b >= pl.nv - 1)) continue;
        const fr = ds.free[comp][idx3(a, b)];
        if (fr === 255) continue;
        if (dir === 0 ? isFace(a, b) && isFace(a, b - 1) : isFace(a, b) && isFace(a - 1, b)) continue;
        const p1 = V.toPx(L, u[a], v[b]);
        const p2 = dir === 0 ? V.toPx(L, L1[ua][a], v[b]) : V.toPx(L, u[a], L1[va][b]);
        g.strokeStyle = fr === 0 ? metal : "rgba(235,235,235,0.55)";
        g.lineWidth = fr === 0 ? 2.2 : 1;
        g.beginPath(); g.moveTo(p1[0], p1[1]); g.lineTo(p2[0], p2[1]); g.stroke();
      }
    }
    const kase = ds.case;
    if (!kase) return;
    g.setLineDash([4, 3]);
    g.lineWidth = 1;
    g.strokeStyle = dark ? "rgba(255,255,255,0.7)" : "rgba(255,255,255,0.8)";
    const pos = [ds.x, ds.y, ds.z][pl.axis][pl.ia];
    if (pl.axis === 2) {
      const circ = (cx, cy, r) => { const [px, py] = V.toPx(L, cx, cy); const rr = (r / (L.u1 - L.u0)) * L.pw; g.beginPath(); g.arc(px, py, rr, 0, 2 * Math.PI); g.stroke(); };
      for (const vi of kase.vias) {
        if (pos >= vi.z0 - 1e-9 && pos <= vi.z1 + 1e-9) circ(vi.x, vi.y, vi.r);
        if ((vi.pads || []).some((z) => Math.abs(z - pos) < 1e-6)) circ(vi.x, vi.y, vi.pad_r);
      }
      for (const s of kase.sheets) if (Math.abs(s.z - pos) < 1e-6) for (const hh of s.holes || []) if (hh.length === 3) circ(hh[0], hh[1], hh[2]);
    } else {
      // dielectric interfaces
      g.strokeStyle = "rgba(255,255,255,0.45)";
      const zs = new Set();
      for (const l of kase.layers) { zs.add(l.z0); zs.add(l.z1); }
      for (const z of zs) { const [xa, ya] = V.toPx(L, L.u0, z), [xb] = V.toPx(L, L.u1, z); g.beginPath(); g.moveTo(xa, ya); g.lineTo(xb, ya); g.stroke(); }
      g.strokeStyle = "rgba(255,255,255,0.8)";
      for (const vi of kase.vias) {
        const d = pos - (pl.axis === 0 ? vi.x : vi.y);
        if (Math.abs(d) >= vi.r) continue;
        const c = pl.axis === 0 ? vi.y : vi.x, hw = Math.sqrt(vi.r * vi.r - d * d);
        const [xa, ya] = V.toPx(L, c - hw, vi.z1), [xb, yb] = V.toPx(L, c + hw, vi.z0);
        g.strokeRect(xa, ya, xb - xa, yb - ya);
      }
    }
    g.setLineDash([]);
  };

  // conductor-loss estimate in the plane (separate colour scale "ice")
  V.condOverlay = function (g, L, st) {
    const { ds, pl, cond } = st;
    const sc = V.scaler({ max: st.condMax, log: true, rangeDb: st.scale.rangeDb, power: true });
    const { ua, va } = V.planeAxes(ds, pl.axis);
    const pos = [ds.x, ds.y, ds.z][pl.axis][pl.ia];
    const thick = 4;
    if (pl.axis === 2) {
      const m = cond.sheets.get(pl.ia);
      if (m) for (let i = 0; i < ds.nx - 1; i++) for (let j = 0; j < ds.ny - 1; j++) {
        const val = m[i * ds.ny + j];
        if (!(val >= 0)) continue;
        const [x1, y1] = V.toPx(L, ds.x[i], ds.y[j]), [x2, y2] = V.toPx(L, ds.x1[i], ds.y1[j]);
        g.fillStyle = V.css("ice", sc(val));
        g.fillRect(Math.min(x1, x2), Math.min(y1, y2), Math.abs(x2 - x1) + 0.6, Math.abs(y2 - y1) + 0.6);
      }
      for (const b of cond.barrels) {
        const row = b.zs.findIndex(([za, zb]) => pos >= za - 1e-9 && pos <= zb + 1e-9);
        if (row < 0) continue;
        const [cx, cy] = V.toPx(L, b.via.x, b.via.y), rr = (b.via.r / (L.u1 - L.u0)) * L.pw;
        g.lineWidth = thick;
        for (let q = 0; q < b.nphi; q++) {
          g.strokeStyle = V.css("ice", sc(b.dens[row][q]));
          g.beginPath();
          g.arc(cx, cy, rr, -((q + 1) / b.nphi) * 2 * Math.PI, -(q / b.nphi) * 2 * Math.PI);
          g.stroke();
        }
      }
    } else {
      const ia = pl.ia;
      for (const [k, m] of cond.sheets) {
        for (let a = 0; a < (pl.axis === 0 ? ds.ny : ds.nx) - 1; a++) {
          const i = pl.axis === 0 ? Math.min(ia, ds.nx - 2) : a, j = pl.axis === 0 ? a : Math.min(ia, ds.ny - 2);
          const val = m[i * ds.ny + j];
          if (!(val >= 0)) continue;
          const ua0 = pl.axis === 0 ? ds.y[j] : ds.x[i], ua1 = pl.axis === 0 ? ds.y1[j] : ds.x1[i];
          const [x1, y1] = V.toPx(L, ua0, ds.z[k]), [x2] = V.toPx(L, ua1, ds.z[k]);
          g.fillStyle = V.css("ice", sc(val));
          g.fillRect(x1, y1 - thick / 2, x2 - x1 + 0.6, thick);
        }
      }
      for (const b of cond.barrels) {
        const d = pos - (pl.axis === 0 ? b.via.x : b.via.y);
        if (Math.abs(d) >= b.via.r) continue;
        const c = pl.axis === 0 ? b.via.y : b.via.x, hw = Math.sqrt(b.via.r ** 2 - d * d);
        for (const sgn of [-1, 1]) {
          // φ of the point on the barrel (x = cos φ, y = sin φ around the via)
          const dx = pl.axis === 0 ? d : sgn * hw, dy = pl.axis === 0 ? sgn * hw : d;
          let ph = Math.atan2(dy, dx); if (ph < 0) ph += 2 * Math.PI;
          const q = Math.min(b.nphi - 1, Math.floor((ph / (2 * Math.PI)) * b.nphi));
          b.zs.forEach(([za, zb], row) => {
            const [xx, ya] = V.toPx(L, c + sgn * hw, zb), [, yb] = V.toPx(L, c + sgn * hw, za);
            g.fillStyle = V.css("ice", sc(b.dens[row][q]));
            g.fillRect(xx - thick / 2, ya, thick, yb - ya + 0.6);
          });
        }
      }
    }
  };

  // arrows of the in-plane vector on a screen grid; length ∝ the colour scale's value (log or linear)
  V.arrows = function (g, L, st, bu, bv) {
    const { pl, pv } = st;
    const [au, av] = pv.arrows;
    let mx = 0;
    for (let p = 0; p < au.length; p++) mx = Math.max(mx, Math.hypot(au[p], av[p]));
    if (!mx) return;
    const step = L.pw < 400 ? 16 : 20;
    const sc = V.scaler({ max: mx, log: st.scale.log, rangeDb: Math.min(st.scale.rangeDb, 40), power: st.view === "S" });
    g.lineCap = "round";
    for (let py = L.mt + step / 2; py < L.mt + L.ph; py += step) for (let px = L.ml + step / 2; px < L.ml + L.pw; px += step) {
      const [uu, vv] = V.fromPx(L, px, py);
      const p = nearestIdx(bu, uu) * pl.nv + nearestIdx(bv, vv);
      const m = Math.hypot(au[p], av[p]);
      const t = sc(m);
      if (t < 0.15) continue;
      const len = 0.92 * step * t, dx = (au[p] / m) * len, dy = (-av[p] / m) * len;
      const x0 = px - dx / 2, y0 = py - dy / 2, x1 = px + dx / 2, y1 = py + dy / 2;
      const hl = Math.min(6, 0.36 * len), ang = Math.atan2(dy, dx);
      for (const [col, lw] of [["rgba(0,0,0,0.65)", 3], ["rgba(255,255,255,0.95)", 1.3]]) {
        g.strokeStyle = col;
        g.lineWidth = lw;
        g.beginPath();
        g.moveTo(x0, y0); g.lineTo(x1, y1);
        g.moveTo(x1, y1); g.lineTo(x1 - hl * Math.cos(ang - 0.45), y1 - hl * Math.sin(ang - 0.45));
        g.moveTo(x1, y1); g.lineTo(x1 - hl * Math.cos(ang + 0.45), y1 - hl * Math.sin(ang + 0.45));
        g.stroke();
      }
    }
  };

  // ---- 3D locator: isometric sketch of the case and the cut plane ---------------------------------------------
  V.locator = function (cv, ds, axis, pos) {
    const dpr = root.devicePixelRatio || 1, W = cv.parentElement.clientWidth || 240, H = Math.round(W * 0.62);
    cv.style.width = W + "px"; cv.style.height = H + "px";
    cv.width = W * dpr; cv.height = H * dpr;
    const g = cv.getContext("2d");
    g.setTransform(dpr, 0, 0, dpr, 0, 0);
    const css = getComputedStyle(document.documentElement);
    g.fillStyle = css.getPropertyValue("--panel").trim() || "#fff";
    g.fillRect(0, 0, W, H);
    const kase = ds.case;
    const d = kase ? kase.domain : [ds.x[0], ds.y[0], ds.z[0], ds.x[ds.nx - 1], ds.y[ds.ny - 1], ds.z[ds.nz - 1]];
    const ZX = 2; // z exaggerated
    const c30 = Math.cos(Math.PI / 6), s30 = 0.5;
    const pr = (x, y, z) => [(x - y) * c30, (x + y) * s30 - z * ZX];
    const pts = [];
    for (const x of [d[0], d[3]]) for (const y of [d[1], d[4]]) for (const z of [d[2], d[5]]) pts.push(pr(x, y, z));
    const minX = Math.min(...pts.map((p) => p[0])), maxX = Math.max(...pts.map((p) => p[0]));
    const minY = Math.min(...pts.map((p) => p[1])), maxY = Math.max(...pts.map((p) => p[1]));
    const s = Math.min((W - 16) / (maxX - minX), (H - 22) / (maxY - minY));
    const P = (x, y, z) => { const [a, b] = pr(x, y, z); return [8 + (a - minX) * s, 6 + (b - minY) * s]; };
    const poly = (ps, fill, stroke) => { g.beginPath(); ps.forEach((p, i) => (i ? g.lineTo(p[0], p[1]) : g.moveTo(p[0], p[1]))); g.closePath(); if (fill) { g.fillStyle = fill; g.fill(); } if (stroke) { g.strokeStyle = stroke; g.stroke(); } };
    const muted = css.getPropertyValue("--muted").trim() || "#888";
    g.lineWidth = 1;
    // domain box (back edges)
    const box = (z) => [P(d[0], d[1], z), P(d[3], d[1], z), P(d[3], d[4], z), P(d[0], d[4], z)];
    poly(box(d[2]), null, muted);
    if (kase) {
      // the return copper first (planes and pours, faint), the signal copper on top
      const sheets = kase.sheets.slice().sort((a, b) => (a.net === "SIG") - (b.net === "SIG") || a.z - b.z);
      for (const sh of sheets) {
        const r = sh.rect;
        const gnd = sh.net !== "SIG";
        const fill = gnd ? "rgba(150,150,160,0.16)" : "rgba(200,120,50,0.95)", line = gnd ? "rgba(150,150,160,0.45)" : null;
        if (sh.polys) for (const ring of sh.polys) poly(ring.map(([x, y]) => P(x, y, sh.z)), fill, line);
        else poly([P(r[0], r[1], sh.z), P(r[2], r[1], sh.z), P(r[2], r[3], sh.z), P(r[0], r[3], sh.z)], fill, line);
      }
      for (const v of kase.vias) {
        const [xa, ya] = P(v.x, v.y, v.z0), [xb, yb] = P(v.x, v.y, v.z1);
        g.strokeStyle = v.net !== "SIG" ? "rgba(140,140,150,0.9)" : "rgba(210,120,40,1)";
        g.lineWidth = v.net !== "SIG" ? 2 : 3;
        g.beginPath(); g.moveTo(xa, ya); g.lineTo(xb, yb); g.stroke();
      }
      g.lineWidth = 1;
      kase.ports.forEach((p, i) => {
        const [xa, ya] = KVF.alongY(p) ? P((p.x + p.x1) / 2, p.y0, Math.max(p.z0, p.z1)) : P(p.x, (p.y0 + p.y1) / 2, Math.max(p.z0, p.z1));
        g.fillStyle = muted; g.font = "9px system-ui"; g.fillText(String(i + 1), xa + 2, ya - 2);
      });
    }
    // the cut plane
    let q;
    if (axis === 0) q = [P(pos, d[1], d[2]), P(pos, d[4], d[2]), P(pos, d[4], d[5]), P(pos, d[1], d[5])];
    else if (axis === 1) q = [P(d[0], pos, d[2]), P(d[3], pos, d[2]), P(d[3], pos, d[5]), P(d[0], pos, d[5])];
    else q = [P(d[0], d[1], pos), P(d[3], d[1], pos), P(d[3], d[4], pos), P(d[0], d[4], pos)];
    poly(q, "rgba(40,130,230,0.25)", "rgba(40,130,230,0.95)");
    poly(box(d[5]), null, muted);
    g.fillStyle = muted;
    g.font = "9px system-ui, sans-serif";
    g.fillText("z ×2", W - 30, H - 4);
  };
})(typeof globalThis !== "undefined" ? globalThis : this);

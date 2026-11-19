// kisi-advanced.html, 3D field view (docs/reference.md "Field views"): the field data of kisi-fdtd's volume
// output (kisi_fdtd::simulate_volume, fd_run_volume) and the quantities derived from it. Plain script (page and
// node via vm, validation/fdtd/check_fields.mjs): attaches its functions to globalThis.KVF.
//
// Normalisation: kisi-fdtd writes peak phasors for 1 W incident (½|a|² = 1 W) at the excited port. On loading,
// every run is rescaled to a 1 V peak incident voltage wave at its excited port into the port's reference
// impedance Z0 (a = 1 V/√Z0, incident power 1/(2·Z0) = 10 mW at 50 Ω): factor 1/√(2·Z0). A mode of two runs
// (odd = (F1 − F2)/√2, even = (F1 + F2)/√2) has ±1/√2 V incident at the two ports and the same 10 mW.
(function (root) {
  "use strict";
  const KVF = (root.KVF = root.KVF || {});
  const MU0 = 4e-7 * Math.PI, EPS0 = 1 / (MU0 * 299792458 ** 2);
  KVF.MU0 = MU0;
  KVF.EPS0 = EPS0;
  const COMPS = ["Ex", "Ey", "Ez", "Hx", "Hy", "Hz"];
  KVF.COMPS = COMPS;

  // ---- f16 -----------------------------------------------------------------------------------------------
  const f32b = new Float32Array(1), u32b = new Uint32Array(f32b.buffer);
  function toHalf(v) {
    f32b[0] = v;
    const x = u32b[0], s = (x >>> 16) & 0x8000;
    let e = ((x >>> 23) & 0xff) - 127 + 15, m = x & 0x7fffff;
    if (e >= 31) return s | 0x7c00;
    if (e <= 0) {
      if (e < -10) return s;
      m = (m | 0x800000) >>> (1 - e);
      return s | ((m + 0x1000) >>> 13);
    }
    const h = s | (e << 10) | (m >>> 13);
    return m & 0x1000 ? h + 1 : h; // round half up (carries into the exponent correctly)
  }
  const HALF = new Float32Array(65536);
  for (let h = 0; h < 65536; h++) {
    const s = h & 0x8000 ? -1 : 1, e = (h >>> 10) & 0x1f, m = h & 0x3ff;
    HALF[h] = e === 0 ? s * m * 2 ** -24 : e === 31 ? (m ? NaN : s * Infinity) : s * (1 + m / 1024) * 2 ** (e - 15);
  }

  // ---- reading ---------------------------------------------------------------------------------------------
  // the blob of kisi-fdtd --volume / fd_run_volume: u32 header length, JSON header, arrays
  KVF.parseRaw = function (bytes) {
    const u8 = bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes);
    const dv = new DataView(u8.buffer, u8.byteOffset, u8.byteLength);
    const hl = dv.getUint32(0, true);
    const h = JSON.parse(new TextDecoder().decode(u8.subarray(4, 4 + hl)));
    const base = u8.byteOffset + 4 + hl;
    const buf = u8.buffer.slice(base, u8.byteOffset + u8.byteLength); // aligned copy
    const a = {};
    for (const d of h.arrays) a[d.name] = d.dtype === "f32" ? new Float32Array(buf, d.offset, d.len) : new Uint8Array(buf, d.offset, d.len);
    return { head: h, arrays: a };
  };

  function grid(h) {
    const g = { x: Float64Array.from(h.x), y: Float64Array.from(h.y), z: Float64Array.from(h.z),
      x1: Float64Array.from(h.x1), y1: Float64Array.from(h.y1), z1: Float64Array.from(h.z1) };
    g.nx = g.x.length;
    g.ny = g.y.length;
    g.nz = g.z.length;
    return g;
  }

  // dataset from one or more raw blobs of the same case and mesh (one per excited port); `kase` optional
  KVF.fromRaw = function (blobs, kase) {
    const parsed = blobs.map((b) => (b.head ? b : KVF.parseRaw(b)));
    const h0 = parsed[0].head;
    const ds = Object.assign(grid(h0), { case: kase || null, name: h0.extra.name, stride: h0.stride, every: h0.every,
      freqs: h0.freqs.slice(), runs: [] });
    ds.f_loss = kase ? (kase.f_loss > 0 ? kase.f_loss : kase.fmax / 2) : null;
    for (const { head, arrays } of parsed) {
      if (head.x.length !== ds.nx || head.y.length !== ds.ny || head.z.length !== ds.nz) throw new Error("blobs of different meshes");
      if (head.freqs.join() !== ds.freqs.join()) throw new Error("blobs with different frequencies");
      const port = head.extra.excited;
      const z0 = kase ? kase.ports[port].r : 50;
      const k = 1 / Math.sqrt(2 * z0);
      const F = ds.freqs.map((_, fi) => {
        const out = [];
        for (const c of COMPS) for (const p of ["re", "im"]) {
          const src = arrays[`${c}_${fi}_${p}`], dst = new Float32Array(src.length);
          for (let q = 0; q < src.length; q++) dst[q] = src[q] * k;
          out.push(dst);
        }
        return out;
      });
      ds.runs.push({ port, z0, s: head.extra.s, sfreq: head.extra.freq, steps: head.extra.steps, cells: head.extra.cells,
        dt: head.extra.dt, F });
      if (!ds.free) {
        ds.free = [0, 1, 2].map((c) => Uint8Array.from(arrays["free" + c]));
        ds.epsr = [0, 1, 2].map((c) => Float32Array.from(arrays["epsr" + c]));
        ds.sigma = [0, 1, 2].map((c) => Float32Array.from(arrays["sigma" + c]));
      }
    }
    ds.runs.sort((a, b) => a.port - b.port);
    return finish(ds);
  };

  function finish(ds) {
    ds.n = ds.nx * ds.ny * ds.nz;
    const sp = (a, a1) => {
      const p = new Float64Array(a.length), d = new Float64Array(a.length), m = new Float64Array(a.length);
      for (let i = 0; i < a.length; i++) { p[i] = (a1[i] - a[i]) * 1e-3; m[i] = 0.5 * (a[i] + a1[i]); }
      d[0] = p[0];
      for (let i = 1; i < a.length; i++) d[i] = 0.5 * (p[i - 1] + p[i]);
      return { p, d, m };
    };
    ds.sx = sp(ds.x, ds.x1);
    ds.sy = sp(ds.y, ds.y1);
    ds.sz = sp(ds.z, ds.z1);
    ds.cache = new Map();
    return ds;
  }

  // ---- KVF container: "KVF1", u32 header length, JSON header, arrays (f16 with a scale, f32, u8); gzip outside --
  // mantissaBits < 10 rounds the f16 mantissa (smaller after gzip; 7 bits: 0.4 % of the value)
  KVF.encode = function (ds, opt) {
    opt = opt || {};
    const mb = opt.mantissaBits == null ? 10 : opt.mantissaBits;
    const drop = 10 - mb;
    const arrays = [], parts = [];
    let off = 0;
    const add = (name, dtype, data, scale) => {
      const bytes = new Uint8Array(data.buffer, data.byteOffset, data.byteLength);
      arrays.push({ name, dtype, offset: off, len: data.length, scale });
      parts.push(bytes);
      off += bytes.length;
      const pad = (4 - (off % 4)) % 4;
      if (pad) { parts.push(new Uint8Array(pad)); off += pad; }
    };
    ds.runs.forEach((r, ri) => r.F.forEach((arrs, fi) => arrs.forEach((src, q) => {
      let mx = 0;
      for (let i = 0; i < src.length; i++) mx = Math.max(mx, Math.abs(src[i]));
      const s = mx > 0 ? mx : 1, out = new Uint16Array(src.length);
      for (let i = 0; i < src.length; i++) {
        let hv = toHalf(src[i] / s);
        if (drop) { const r0 = 1 << (drop - 1); const mm = hv & 0x8000; hv = mm | ((((hv & 0x7fff) + r0) >> drop) << drop); }
        out[i] = hv;
      }
      add(`r${ri}_${COMPS[q >> 1]}_${fi}_${q & 1 ? "im" : "re"}`, "f16", out, s);
    })));
    for (let c = 0; c < 3; c++) add("free" + c, "u8", ds.free[c]);
    for (let c = 0; c < 3; c++) add("epsr" + c, "f32", ds.epsr[c]);
    for (let c = 0; c < 3; c++) add("sigma" + c, "f32", ds.sigma[c]);
    const head = {
      format: "kisi-volume-field", version: 1, note: "kisi-advanced.html 3D field view; fields: peak phasors for 1 V incident at the excited port",
      name: ds.name, case: ds.case, stride: ds.stride, every: ds.every, freqs: ds.freqs, f_loss: ds.f_loss,
      x: Array.from(ds.x), y: Array.from(ds.y), z: Array.from(ds.z), x1: Array.from(ds.x1), y1: Array.from(ds.y1), z1: Array.from(ds.z1),
      runs: ds.runs.map((r) => ({ port: r.port, z0: r.z0, s: r.s, sfreq: r.sfreq, steps: r.steps, cells: r.cells, dt: r.dt,
        seconds: r.seconds, wasm_mb: r.wasm_mb })),
      mantissa_bits: mb, arrays, meta: opt.meta || ds.meta || null,
    };
    let hb = new TextEncoder().encode(JSON.stringify(head));
    const hl = Math.ceil(hb.length / 4) * 4;
    const out = new Uint8Array(8 + hl + off);
    out.set([75, 86, 70, 49], 0);
    new DataView(out.buffer).setUint32(4, hl, true);
    out.fill(32, 8, 8 + hl);
    out.set(hb, 8);
    let p = 8 + hl;
    for (const b of parts) { out.set(b, p); p += b.length; }
    return out;
  };

  KVF.isKVF = (u8) => u8[0] === 75 && u8[1] === 86 && u8[2] === 70 && u8[3] === 49;
  KVF.decode = function (bytes) {
    const u8 = bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes);
    if (!KVF.isKVF(u8)) throw new Error("not a KVF file");
    const hl = new DataView(u8.buffer, u8.byteOffset).getUint32(4, true);
    const h = JSON.parse(new TextDecoder().decode(u8.subarray(8, 8 + hl)));
    const buf = u8.buffer.slice(u8.byteOffset + 8 + hl, u8.byteOffset + u8.byteLength);
    const get = {};
    for (const d of h.arrays) {
      if (d.dtype === "f16") {
        const src = new Uint16Array(buf, d.offset, d.len), out = new Float32Array(d.len);
        for (let i = 0; i < d.len; i++) out[i] = HALF[src[i]] * d.scale;
        get[d.name] = out;
      } else get[d.name] = d.dtype === "f32" ? new Float32Array(buf.slice(d.offset, d.offset + 4 * d.len)) : new Uint8Array(buf.slice(d.offset, d.offset + d.len));
    }
    const ds = Object.assign(grid(h), { case: h.case, name: h.name, stride: h.stride, every: h.every, freqs: h.freqs,
      f_loss: h.f_loss, meta: h.meta, mantissaBits: h.mantissa_bits });
    ds.runs = h.runs.map((r, ri) => Object.assign({}, r, {
      F: h.freqs.map((_, fi) => { const o = []; for (const c of COMPS) for (const p of ["re", "im"]) o.push(get[`r${ri}_${c}_${fi}_${p}`]); return o; }),
    }));
    ds.free = [0, 1, 2].map((c) => get["free" + c]);
    ds.epsr = [0, 1, 2].map((c) => get["epsr" + c]);
    ds.sigma = [0, 1, 2].map((c) => get["sigma" + c]);
    return finish(ds);
  };

  // ---- excitations ---------------------------------------------------------------------------------------
  // available excitations of a dataset: single runs, and odd/even of ports 1/2 for a pair (both runs present)
  KVF.excitationsOf = function (ds) {
    const have = new Set(ds.runs.map((r) => r.port));
    const list = [];
    if (ds.case && ds.case.meta && ds.case.meta.kind === "pair" && have.has(0) && have.has(1)) {
      const s = Math.SQRT1_2;
      list.push({ id: "odd", label: "odd / differential (ports 1, 2)", w: [[0, s], [1, -s]] });
      list.push({ id: "even", label: "even / common (ports 1, 2)", w: [[0, s], [1, s]] });
    }
    for (const r of ds.runs) {
      const nm = ds.case && ds.case.meta && ds.case.meta.ports ? ds.case.meta.ports[r.port].name : String(r.port + 1);
      list.push({ id: "p" + (r.port + 1), label: `port ${nm}`, w: [[r.port, 1]] });
    }
    return list;
  };

  // weighted sum of runs (by excited port) at frequency index fi: 12 Float32Arrays (Ex re, Ex im, …, Hz im)
  KVF.fieldsOf = function (ds, fi, w) {
    const key = fi + ":" + JSON.stringify(w);
    if (ds.cache.has(key)) return ds.cache.get(key);
    const runs = w.map(([port, wt]) => [ds.runs.find((r) => r.port === port), wt]);
    if (runs.some(([r]) => !r)) throw new Error("excitation needs a run that is missing");
    let F;
    if (runs.length === 1 && runs[0][1] === 1) F = runs[0][0].F[fi];
    else {
      F = [];
      for (let q = 0; q < 12; q++) {
        const o = new Float32Array(ds.n);
        for (const [r, wt] of runs) { const s = r.F[fi][q]; for (let i = 0; i < o.length; i++) o[i] += wt * s[i]; }
        F.push(o);
      }
    }
    if (ds.cache.size > 8) ds.cache.delete(ds.cache.keys().next().value);
    ds.cache.set(key, F);
    return F;
  };

  // S column (b_k / incident) of an excitation at frequency f (nearest point of the S grid): complex per port,
  // and the S grid point used
  KVF.waves = function (ds, f, w) {
    const r0 = ds.runs[0];
    let k = 0;
    for (let i = 1; i < r0.sfreq.length; i++) if (Math.abs(r0.sfreq[i] - f) < Math.abs(r0.sfreq[k] - f)) k = i;
    const np = r0.s.length, b = [];
    for (let p = 0; p < np; p++) {
      let re = 0, im = 0;
      for (const [port, wt] of w) { const r = ds.runs.find((q) => q.port === port); re += wt * r.s[p][k][0]; im += wt * r.s[p][k][1]; }
      b.push([re, im]);
    }
    return { b, f: r0.sfreq[k] };
  };
  // incident power of an excitation (W): Σ w² · 1/(2 Z0)
  KVF.incident = (ds, w) => w.reduce((a, [port, wt]) => a + (wt * wt) / (2 * ds.runs.find((r) => r.port === port).z0), 0);

  // ---- positions and interpolation -------------------------------------------------------------------------
  const idx = (ds, i, j, k) => (i * ds.ny + j) * ds.nz + k;
  KVF.idx = idx;
  // coordinates (mm) of component c's samples along each axis
  KVF.compCoords = function (ds, c) {
    const X = [ds.x, ds.y, ds.z], M = [ds.sx.m, ds.sy.m, ds.sz.m];
    const half = c < 3 ? [c === 0, c === 1, c === 2] : [c !== 3, c !== 4, c !== 5];
    return [0, 1, 2].map((a) => (half[a] ? M[a] : X[a]));
  };
  function locate(arr, v) {
    let lo = 0, hi = arr.length - 1;
    if (v <= arr[0]) return [0, 0];
    if (v >= arr[hi]) return [hi - 1, 1];
    while (hi - lo > 1) { const mid = (lo + hi) >> 1; if (arr[mid] <= v) lo = mid; else hi = mid; }
    return [lo, (v - arr[lo]) / (arr[lo + 1] - arr[lo])];
  }
  // trilinear interpolation of component c (complex) at (x, y, z) mm
  KVF.interp = function (ds, F, c, x, y, z) {
    const cc = ds._cc || (ds._cc = [0, 1, 2, 3, 4, 5].map((q) => KVF.compCoords(ds, q)));
    const [X, Y, Z] = cc[c];
    const [i, fx] = locate(X, x), [j, fy] = locate(Y, y), [k, fz] = locate(Z, z);
    const re = F[2 * c], im = F[2 * c + 1];
    let sr = 0, si = 0;
    for (let a = 0; a < 2; a++) for (let b = 0; b < 2; b++) for (let d = 0; d < 2; d++) {
      const wt = (a ? fx : 1 - fx) * (b ? fy : 1 - fy) * (d ? fz : 1 - fz);
      if (!wt) continue;
      const q = idx(ds, Math.min(i + a, ds.nx - 1), Math.min(j + b, ds.ny - 1), Math.min(k + d, ds.nz - 1));
      sr += wt * re[q];
      si += wt * im[q];
    }
    return [sr, si];
  };

  // ---- cut planes ----------------------------------------------------------------------------------------
  // plane normal to `axis` (0 x, 1 y, 2 z) at node index `ia`; in-plane axes u, v (x: y,z; y: x,z; z: x,y).
  // Complex E and H averaged to the nodes, the dielectric loss density ½σ|E|² (W/m³) and metal flags.
  KVF.plane = function (ds, F, axis, ia) {
    const ua = axis === 0 ? 1 : 0, va = axis === 2 ? 1 : 2;
    const n = [ds.nx, ds.ny, ds.nz], nu = n[ua], nv = n[va];
    const out = { axis, ia, ua, va, nu, nv, E: [], H: [], loss: new Float32Array(nu * nv) };
    for (let q = 0; q < 6; q++) out.E.push(new Float32Array(nu * nv)), out.H.push(new Float32Array(nu * nv));
    const ijk = [0, 0, 0];
    const at = (o, a, d) => { const t = o.slice(); t[a] = Math.max(0, t[a] - d); return idx(ds, t[0], t[1], t[2]); };
    for (let u = 0; u < nu; u++) for (let v = 0; v < nv; v++) {
      ijk[axis] = ia; ijk[ua] = u; ijk[va] = v;
      const p = u * nv + v, q0 = idx(ds, ijk[0], ijk[1], ijk[2]);
      let loss = 0;
      for (let c = 0; c < 3; c++) {
        // E_c on the edges c−½ and c+½ around the node
        const qm = at(ijk, c, 1);
        for (let part = 0; part < 2; part++) out.E[2 * c + part][p] = 0.5 * (F[2 * c + part][qm] + F[2 * c + part][q0]);
        const sg = ds.sigma[c];
        loss += 0.25 * (sg[qm] * (F[2 * c][qm] ** 2 + F[2 * c + 1][qm] ** 2) + sg[q0] * (F[2 * c][q0] ** 2 + F[2 * c + 1][q0] ** 2));
        // H_c on the four faces around the node (the two axes other than c)
        const a1 = (c + 1) % 3, a2 = (c + 2) % 3;
        const qs = [q0, at(ijk, a1, 1), at(ijk, a2, 1)];
        { const t = ijk.slice(); t[a1] = Math.max(0, t[a1] - 1); t[a2] = Math.max(0, t[a2] - 1); qs.push(idx(ds, t[0], t[1], t[2])); }
        for (let part = 0; part < 2; part++) {
          const a = F[6 + 2 * c + part];
          out.H[2 * c + part][p] = 0.25 * (a[qs[0]] + a[qs[1]] + a[qs[2]] + a[qs[3]]);
        }
      }
      out.loss[p] = loss;
    }
    return out;
  };

  // values of a view on a plane: {val (Float32Array), arrows: [u comp, v comp] or null, signed}
  // view: "E" | "H" | "S" | "loss"; comp: "mag" | 0 | 1 | 2 (x, y, z); phase: null (phasor magnitude / time
  // average) or φ in rad (instantaneous Re{F e^{jφ}})
  KVF.planeValues = function (pl, view, comp, phase) {
    const N = pl.nu * pl.nv, val = new Float32Array(N);
    const au = new Float32Array(N), av = new Float32Array(N);
    const { ua, va } = pl;
    const cs = phase == null ? 1 : Math.cos(phase), sn = phase == null ? 0 : Math.sin(phase);
    let bound = new Float32Array(N); // max over the period of |value| (fixed colour scale of the animation)
    const vecRe = (A, p, c) => A[2 * c][p] * cs - A[2 * c + 1][p] * sn;
    const mag2 = (A, p) => A[0][p] ** 2 + A[1][p] ** 2 + A[2][p] ** 2 + A[3][p] ** 2 + A[4][p] ** 2 + A[5][p] ** 2;
    for (let p = 0; p < N; p++) {
      if (view === "E" || view === "H") {
        const A = view === "E" ? pl.E : pl.H;
        const inst = [vecRe(A, p, 0), vecRe(A, p, 1), vecRe(A, p, 2)];
        if (comp === "mag") {
          val[p] = phase == null ? Math.sqrt(mag2(A, p)) : Math.hypot(inst[0], inst[1], inst[2]);
          bound[p] = Math.sqrt(mag2(A, p));
        } else {
          val[p] = phase == null ? Math.hypot(A[2 * comp][p], A[2 * comp + 1][p]) : inst[comp];
          bound[p] = Math.hypot(A[2 * comp][p], A[2 * comp + 1][p]);
        }
        au[p] = inst[ua];
        av[p] = inst[va];
      } else if (view === "S") {
        const E = pl.E, H = pl.H;
        let s;
        if (phase == null) s = KVF.poyntingAvg(E, H, p);
        else {
          const e = [vecRe(E, p, 0), vecRe(E, p, 1), vecRe(E, p, 2)], h = [vecRe(H, p, 0), vecRe(H, p, 1), vecRe(H, p, 2)];
          s = [e[1] * h[2] - e[2] * h[1], e[2] * h[0] - e[0] * h[2], e[0] * h[1] - e[1] * h[0]];
        }
        val[p] = comp === "mag" ? Math.hypot(s[0], s[1], s[2]) : s[comp];
        bound[p] = Math.sqrt(mag2(E, p) * mag2(H, p));
        au[p] = s[ua];
        av[p] = s[va];
      } else {
        val[p] = pl.loss[p];
        bound[p] = pl.loss[p];
      }
    }
    const signed = comp !== "mag" && (phase != null || view === "S");
    return { val, bound, arrows: view === "loss" ? null : [au, av], signed };
  };
  KVF.poyntingAvg = function (E, H, p) {
    const d = (a, b) => E[2 * a][p] * H[2 * b][p] + E[2 * a + 1][p] * H[2 * b + 1][p]; // Re(E_a H_b*)
    return [0.5 * (d(1, 2) - d(2, 1)), 0.5 * (d(2, 0) - d(0, 2)), 0.5 * (d(0, 1) - d(1, 0))];
  };

  // ---- integrals (exact on the Yee grid for stride 1) -------------------------------------------------------
  // ∫ S·n dA (W) through the mesh plane normal to `a` at node index ia; rect: [b0, b1, c0, c1] node ranges of the
  // two tangential axes b = a+1, c = a+2 (cyclic), as validation/fdtd/volume_fields.py box_flux
  KVF.flux = function (ds, F, a, ia, rect) {
    const b = (a + 1) % 3, c = (a + 2) % 3, n = [ds.nx, ds.ny, ds.nz];
    const S = [ds.sx, ds.sy, ds.sz];
    if (ia < 1) ia = 1;
    const [b0, b1, c0, c1] = rect || [0, n[b] - 1, 1, n[c] - 2];
    const id = (i, j, k) => { const t = [0, 0, 0]; t[a] = i; t[b] = j; t[c] = k; return idx(ds, t[0], t[1], t[2]); };
    const Ebr = F[2 * b], Ebi = F[2 * b + 1], Ecr = F[2 * c], Eci = F[2 * c + 1];
    const Hbr = F[6 + 2 * b], Hbi = F[6 + 2 * b + 1], Hcr = F[6 + 2 * c], Hci = F[6 + 2 * c + 1];
    // an E edge partly in metal (conformal boundary) carries its field on the free part only
    const fb = ds.free[b], fc = ds.free[c];
    let tot = 0;
    const rb = rect ? [b0, b1] : [0, n[b] - 1], rc = rect ? [c0, c1 + 1] : [1, n[c] - 1];
    for (let j = rb[0]; j < rb[1]; j++) for (let k = rc[0]; k < rc[1]; k++) {
      const q = id(ia, j, k), qm = id(ia - 1, j, k);
      const hr = 0.5 * (Hcr[qm] + Hcr[q]), hi = 0.5 * (Hci[qm] + Hci[q]);
      tot += 0.5 * (Ebr[q] * hr + Ebi[q] * hi) * S[b].p[j] * S[c].d[k] * (fb[q] / 255);
    }
    const rb2 = rect ? [b0, b1 + 1] : [1, n[b] - 1], rc2 = rect ? [c0, c1] : [0, n[c] - 1];
    for (let j = rb2[0]; j < rb2[1]; j++) for (let k = rc2[0]; k < rc2[1]; k++) {
      const q = id(ia, j, k), qm = id(ia - 1, j, k);
      const hr = 0.5 * (Hbr[qm] + Hbr[q]), hi = 0.5 * (Hbi[qm] + Hbi[q]);
      tot -= 0.5 * (Ecr[q] * hr + Eci[q] * hi) * S[b].d[j] * S[c].p[k] * (fc[q] / 255);
    }
    return tot;
  };
  // outward flux through a box m nodes inside the sampled domain (power into the PML)
  KVF.boxOutflow = function (ds, F, m) {
    m = m == null ? 2 : m;
    return KVF.boxBalance(ds, F, [m, ds.nx - 1 - m, m, ds.ny - 1 - m, m, ds.nz - 1 - m], true).outflow;
  };
  // energy balance of the box of nodes [i0, i1] × [j0, j1] × [k0, k1]: outward ∫S·dA over its six faces and the
  // dielectric loss inside (edges on a face count with half their dual volume per face). For a box without
  // sources, outflow + loss = 0; with a driven port inside, = the power the port delivers.
  KVF.boxBalance = function (ds, F, box, fluxOnly) {
    const lo = [box[0], box[2], box[4]], hi = [box[1], box[3], box[5]];
    let outflow = 0;
    for (let a = 0; a < 3; a++) {
      const b = (a + 1) % 3, c = (a + 2) % 3;
      const rect = [lo[b], hi[b], lo[c], hi[c]];
      outflow += KVF.flux(ds, F, a, hi[a], rect) - KVF.flux(ds, F, a, lo[a], rect);
    }
    if (fluxOnly) return { outflow };
    const S = [ds.sx, ds.sy, ds.sz];
    let loss = 0;
    for (let c = 0; c < 3; c++) {
      const re = F[2 * c], im = F[2 * c + 1], sg = ds.sigma[c];
      const top = hi.slice();
      top[c] -= 1;
      for (let i = lo[0]; i <= top[0]; i++) for (let j = lo[1]; j <= top[1]; j++) for (let k = lo[2]; k <= top[2]; k++) {
        const q = idx(ds, i, j, k);
        if (!sg[q]) continue;
        const t = [i, j, k];
        let w = 1;
        for (let a = 0; a < 3; a++) if (a !== c && (t[a] === lo[a] || t[a] === hi[a])) w *= 0.5;
        const v = (c === 0 ? S[0].p[i] : S[0].d[i]) * (c === 1 ? S[1].p[j] : S[1].d[j]) * (c === 2 ? S[2].p[k] : S[2].d[k]);
        loss += w * 0.5 * sg[q] * (re[q] * re[q] + im[q] * im[q]) * v;
      }
    }
    return { outflow, loss };
  };
  // ∫ ½σ|E|² dV (W), each E edge with its dual volume, interior nodes; per dielectric layer of the case
  KVF.dielLoss = function (ds, F) {
    const S = [ds.sx, ds.sy, ds.sz];
    const layers = ds.case ? ds.case.layers : [];
    const per = new Float64Array(layers.length + 1);
    let tot = 0;
    for (let c = 0; c < 3; c++) {
      const re = F[2 * c], im = F[2 * c + 1], sg = ds.sigma[c];
      for (let i = 1; i < ds.nx - 1; i++) for (let j = 1; j < ds.ny - 1; j++) {
        const vxy = (c === 0 ? S[0].p[i] : S[0].d[i]) * (c === 1 ? S[1].p[j] : S[1].d[j]);
        for (let k = 1; k < ds.nz - 1; k++) {
          const q = idx(ds, i, j, k);
          if (!sg[q]) continue;
          const p = 0.5 * sg[q] * (re[q] * re[q] + im[q] * im[q]) * vxy * (c === 2 ? S[2].p[k] : S[2].d[k]);
          tot += p;
          const zc = c === 2 ? S[2].m[k] : ds.z[k];
          let li = layers.findIndex((l) => zc >= l.z0 - 1e-9 && zc <= l.z1 + 1e-9);
          per[li < 0 ? layers.length : li] += p;
        }
      }
    }
    return { total: tot, perLayer: Array.from(per) };
  };

  // port voltage V = φ(signal) − φ(reference) from Ez across a z-directed lumped port (average over its width);
  // a port along x sits at x across y0…y1, one along y (KVF.alongY) at y0 across x…x1
  KVF.portVoltage = function (ds, F, port) {
    const pr = ds.case.ports[port], ay = KVF.alongY(pr);
    const near = (a, v) => { let k = 0; for (let q = 1; q < a.length; q++) if (Math.abs(a[q] - v) < Math.abs(a[k] - v)) k = q; return k; };
    const [pos, lo, hi, A, B] = ay ? [pr.y0, pr.x, pr.x1, ds.y, ds.x] : [pr.x, pr.y0, pr.y1, ds.x, ds.y];
    const i0 = near(A, pos);
    let js = [];
    for (let j = 0; j < B.length; j++) if (B[j] >= lo - 1e-9 && B[j] <= hi + 1e-9) js.push(j);
    if (!js.length) js = [near(B, (lo + hi) / 2)];
    const zlo = Math.min(pr.z0, pr.z1), zhi = Math.max(pr.z0, pr.z1), sgn = pr.z1 > pr.z0 ? 1 : -1;
    let vr = 0, vi = 0;
    for (const j of js) for (let k = 0; k < ds.nz; k++) {
      if (ds.z[k] < zlo - 1e-9 || ds.z1[k] > zhi + 1e-9) continue;
      const q = ay ? idx(ds, j, i0, k) : idx(ds, i0, j, k);
      vr -= sgn * F[4][q] * ds.sz.p[k];
      vi -= sgn * F[5][q] * ds.sz.p[k];
    }
    return [vr / js.length, vi / js.length];
  };
  // a point in a sheet of the case: its rectangle and, with polygons, inside them (even-odd over all rings)
  KVF.inSheet = function (s, x, y) {
    const r = s.rect;
    if (x < r[0] - 1e-9 || x > r[2] + 1e-9 || y < r[1] - 1e-9 || y > r[3] + 1e-9) return false;
    if (s.holes && s.holes.some((h) => (h.length === 3 ? Math.hypot(x - h[0], y - h[1]) < h[2] : false))) return false;
    if (!s.polys) return true;
    let inside = false;
    for (const ring of s.polys) for (let a = 0, b = ring.length - 1; a < ring.length; b = a++) {
      const [xa, ya] = ring[a], [xb, yb] = ring[b];
      if ((ya > y) !== (yb > y) && x < ((xb - xa) * (y - ya)) / (yb - ya) + xa) inside = !inside;
    }
    return inside;
  };

  // ---- conductor loss estimate (perturbation of the PEC solution) ------------------------------------------
  // Sheets: every mesh face normal to z whose four edges are metal, ½·Rs·|H_t|² on both sides (H half a cell
  // above and below). Barrels (vias of the case): ½·Rs·(|H_φ|²·((r+δ)/r)² + |H_z|²) sampled at ρ = r + δ,
  // δ = 1.5 local cells, scaled to the surface by the 1/ρ decay. Rs = √(π f μ0 / σ_Cu), smooth copper.
  // Estimate: current crowding at trace edges is under-resolved, no roughness, no skin-depth effects on the fields.
  KVF.conductorLoss = function (ds, F, f, sigmaCu) {
    const Rs = Math.sqrt((Math.PI * f * MU0) / sigmaCu);
    const res = { Rs, sigmaCu, sheets: new Map(), barrels: [], regions: { "signal traces and pads": 0, "planes and other copper": 0,
      "signal via barrels": 0, "other via barrels (return vias)": 0 }, total: 0, exact: ds.stride === 1 };
    if (ds.stride !== 1) return res;
    const kase = ds.case;
    const fr = ds.free;
    // the signal copper is net "SIG" (examples and kisi's exporter); everything else is the return path
    const sigSheets = kase ? kase.sheets.filter((s) => s.net === "SIG") : [];
    const netAt = (x, y, z) => {
      if (!kase) return "other";
      for (const v of kase.vias) if (v.pads && v.pads.some((pz) => Math.abs(pz - z) < 1e-6) && Math.hypot(x - v.x, y - v.y) <= Math.max(v.pad_r, v.r) + 1e-6) return v.net === "SIG" ? "SIG" : "other";
      for (const s of sigSheets) if (Math.abs(s.z - z) < 1e-6 && KVF.inSheet(s, x, y)) return "SIG";
      return "other";
    };
    const Hx = [F[6], F[7]], Hy = [F[8], F[9]];
    for (let k = 0; k < ds.nz; k++) {
      let map = null;
      for (let i = 0; i < ds.nx - 1; i++) for (let j = 0; j < ds.ny - 1; j++) {
        if (fr[0][idx(ds, i, j, k)] || fr[0][idx(ds, i, j + 1, k)] || fr[1][idx(ds, i, j, k)] || fr[1][idx(ds, i + 1, j, k)]) continue;
        let h2 = 0, open = 0;
        for (const kk of [k, k - 1]) {
          if (kk < 0 || kk >= ds.nz - 1) continue;
          // this side is dielectric if one of the four Ez edges at the face's corners is not metal
          if (!(fr[2][idx(ds, i, j, kk)] || fr[2][idx(ds, i + 1, j, kk)] || fr[2][idx(ds, i, j + 1, kk)] || fr[2][idx(ds, i + 1, j + 1, kk)])) continue;
          open++;
          const a = idx(ds, i, j, kk), b = idx(ds, i + 1, j, kk), c = idx(ds, i, j + 1, kk);
          const hxr = 0.5 * (Hx[0][a] + Hx[0][b]), hxi = 0.5 * (Hx[1][a] + Hx[1][b]);
          const hyr = 0.5 * (Hy[0][a] + Hy[0][c]), hyi = 0.5 * (Hy[1][a] + Hy[1][c]);
          h2 += hxr * hxr + hxi * hxi + hyr * hyr + hyi * hyi;
        }
        if (!open) continue; // inside a solid barrel
        const ps = 0.5 * Rs * h2, A = ds.sx.p[i] * ds.sy.p[j];
        if (!map) map = new Float32Array(ds.nx * ds.ny).fill(NaN);
        map[i * ds.ny + j] = ps;
        const net = netAt(ds.sx.m[i], ds.sy.m[j], ds.z[k]);
        res.regions[net === "SIG" ? "signal traces and pads" : "planes and other copper"] += ps * A;
        res.total += ps * A;
      }
      if (map) res.sheets.set(k, map);
    }
    if (kase) for (const v of kase.vias) {
      if (v.outer) continue;
      const NPHI = 36;
      let i0 = 0;
      for (let q = 1; q < ds.nx; q++) if (Math.abs(ds.x[q] - (v.x + v.r)) < Math.abs(ds.x[i0] - (v.x + v.r))) i0 = q;
      const h = Math.max(ds.sx.p[Math.min(i0, ds.nx - 1)], 1e-6) * 1e3; // local cell (mm)
      const delta = 1.5 * h, rho = v.r + delta, corr = (rho / v.r) ** 2;
      const zs = [], dens = [];
      let P = 0;
      for (let k = 0; k < ds.nz; k++) {
        const za = ds.z[k], zb = ds.z1[k];
        if (za < v.z0 - 1e-9 || zb > v.z1 + 1e-9) continue;
        const zm = 0.5 * (za + zb), dz = (zb - za) * 1e-3;
        const row = new Float32Array(NPHI);
        for (let q = 0; q < NPHI; q++) {
          const ph = ((q + 0.5) / NPHI) * 2 * Math.PI, cx = Math.cos(ph), sy = Math.sin(ph);
          const px = v.x + rho * cx, py = v.y + rho * sy;
          const hx = KVF.interp(ds, F, 3, px, py, zm), hy = KVF.interp(ds, F, 4, px, py, zm), hz = KVF.interp(ds, F, 5, px, py, zm);
          const hpr = -hx[0] * sy + hy[0] * cx, hpi = -hx[1] * sy + hy[1] * cx;
          const ps = 0.5 * Rs * ((hpr * hpr + hpi * hpi) * corr + hz[0] * hz[0] + hz[1] * hz[1]);
          row[q] = ps;
          P += ps * v.r * 1e-3 * ((2 * Math.PI) / NPHI) * dz;
        }
        zs.push([za, zb]);
        dens.push(row);
      }
      res.barrels.push({ via: v, zs, dens, nphi: NPHI, P, delta });
      res.regions[(v.net || "SIG") === "SIG" ? "signal via barrels" : "other via barrels (return vias)"] += P;
      res.total += P;
    }
    return res;
  };

  // power summary of an excitation at frequency index fi (W): S-parameter powers, losses, outflow
  KVF.summary = function (ds, fi, w, sigmaCu) {
    const F = KVF.fieldsOf(ds, fi, w), f = ds.freqs[fi];
    const pin = KVF.incident(ds, w), { b, f: fs } = KVF.waves(ds, f, w);
    const meta = ds.case && ds.case.meta;
    const side = (p) => (meta && meta.ports && meta.ports[p] ? meta.ports[p].side : p === 0 ? -1 : 1);
    let refl = 0, trans = 0;
    b.forEach(([re, im], p) => { const pw = (re * re + im * im) * pin; if (side(p) < 0) refl += pw; else trans += pw; });
    const diel = KVF.dielLoss(ds, F), out = KVF.boxOutflow(ds, F, 2);
    const fl = ds.f_loss || f;
    const cond = sigmaCu ? KVF.conductorLoss(ds, F, f, sigmaCu) : null;
    return { f, fs, pin, b, refl, trans, diel, dielMaterial: (diel.total * f) / fl, f_loss: fl, out, cond,
      balance: pin - refl - trans };
  };
})(typeof globalThis !== "undefined" ? globalThis : this);

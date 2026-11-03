// kisi solver backend: Henrik Forstén's js_2d_fields (GPL-3.0), triangular FEM backend
// (src/tri_solver/: static P2 FEM, full-wave P2 Nédélec eigenmode above 100 MHz, MQS conductor
// loss, Djordjevic-Sarkar dielectrics), used unmodified through the custom-geometry text format.
//
// Validation only for now (validation/cst_xs/run_fullwave.py); not in kisi's bundles or CLI.
// Its WASM modules (wasm_solver/eigen_solver.js, wasm_solver/gmsh.js) are not in upstream's git:
// tools/build_forsten_fullwave.sh builds them, and the importer redirects upstream's imports of
// them (in node: tools/upstream_hooks_fw.mjs; a bundle would map them like
// tools/solver_bundle.py's REDIRECT). UPSTREAM.md, "Full-wave backend".
//
// Same contract as backend_forsten.js:
//   solveGeometry(text, {tol, maxNodes, f0, ...}) -> {ok, z0?, eeff?, zodd?, zeven?, eeff_odd?, eeff_even?,
//                                  r?, l?, g?, c?, (…_odd, …_even), mat_l?, mat_c?, f0, ms, error?}
// with these differences and additions:
//   z0 / zodd / zeven  the line impedance 1/(c0 C0 sqrt(eeff_mode)): full-wave dielectric dispersion, conductor
//                      loss and internal inductance excluded (PEC-like, as the FDM backend's static Z0)
//   eeff / eeff_odd …  the phase εeff (β/k0)² of the RLGC line: includes the internal inductance (as FDM)
//   eeff_mode …        εeff of the full-wave eigenmode with ideal conductors (static below 100 MHz)
//   z0_static, eeff_static …  static P2 FEM values on the same mesh (statics: true)
//   zc_re, zc_im …     complex characteristic impedance sqrt((R+jωL)/(G+jωC))
//   alpha_c, alpha_d … attenuation in dB/m
//   info               mesh size, eigen bias, loss method, warnings
// and a sweep entry point:
//   solveSweep(text, freqs, opts) -> {ok, f: [...], modes: {single|odd|even: {z0, zc_re, zc_im, eeff, eeff_mode,
//                                     r, l, g, c, alpha_c, alpha_d, l_int}}, zdiff?, zcomm?, mat?, info, ms}
import { CustomGeometrySolver } from "../../third_party/js_2d_fields/src/custom_geometry.js";

export const BACKEND = { id: "forsten-fem-fw", name: "js_2d_fields (Forstén), triangular FEM, full-wave" };
const F0 = 1e9;
const C0 = 299792458;

// opts: tol (energy and parameter tolerance of the adaptive mesh), maxNodes (triangle budget maxNodes/4,
// capped by upstream's eigensolve memory guard at about 24k triangles), maxIters, sigma (S/m), rq (m),
// causal (Djordjevic-Sarkar dielectrics, εr and tanδ hold at 1 GHz), lossMethod ('auto' = MQS, upstream's
// default; 'static' = static-field perturbation, cheap; 'perturbation'), certify (upstream's static
// error certificate, default on), triOpts (further TriBackend options).
function makeSolver(text, freq, o) {
  const s = new CustomGeometrySolver({ text, freq, sigma_cond: o.sigma ?? 5.8e7, rq: o.rq ?? 0, nx: 30, ny: 30,
    mesh_backend: "triangular" });
  s.use_causal_materials = !!o.causal;
  s.tri_opts = { lossMethod: o.lossMethod ?? "auto", ...(o.triOpts || {}) };
  return s;
}
const adaptive = (o, extra = {}) => ({ energy_tol: o.tol ?? 0.005, param_tol: o.tol ?? 0.005, max_iters: o.maxIters ?? 12,
  max_nodes: o.maxNodes ?? 60000, min_converged_passes: 2, ...(o.certify === undefined ? {} : { certify: o.certify }), ...extra });
const flat = (a) => (Array.isArray(a) && a.length === 2 ? [a[0][0], a[0][1], a[1][0], a[1][1]] : undefined);
const modeName = (m, i, n) => m.mode || (n === 1 ? "single" : ["odd", "even"][i]);
// key of a mode value in the contract: z0 / eeff for a single line, zodd / eeff_odd / … for a pair
const key = (k, name) => (name === "single" ? k : k === "z0" ? "z" + name : k + "_" + name);

function modeValues(m) {
  const x = m.RLGC || {};
  return { z0: m.Z0, eeff: m.eps_eff, eeff_mode: m.eps_eff_mode ?? m.eps_eff, r: x.R, l: x.L, g: x.G, c: x.C,
    zc_re: m.Zc && m.Zc.re, zc_im: m.Zc && m.Zc.im, alpha_c: m.alpha_c, alpha_d: m.alpha_d, l_int: m.L_internal,
    eigen_bias: m.eigen_bias, loss_via: m.lossVia };
}

function info(s, r) {
  const q = s.meshQuality || {};
  const tm = s.triMesh || (s._triBackend && s._triBackend.mesh) || {};
  return { n_tris: tm.nTris ?? null, n_nodes: tm.nodes ? tm.nodes.length / 2 : null,
    mesh_max_q: q.maxQ ?? null, certification: s.certification ? { pass: s.certification.pass, err: s.certification.err } : null,
    warnings: [...new Set(((r && r.warnings) || s.modeWarnings || []).map((w) => w.type + (w.reason ? ":" + w.reason : "")))] };
}

function quiet(fn) {
  return async (...a) => {
    const log = console.log, warn = console.warn;
    console.log = () => {}; console.warn = () => {};
    try { return await fn(...a); } finally { console.log = log; console.warn = warn; }
  };
}

export const solveGeometry = quiet(async (text, o = {}) => {
  const t0 = performance.now();
  const f0 = o.f0 ?? F0;
  try {
    const s = makeSolver(text, f0, o);
    const r = await s.solve_adaptive(adaptive(o));
    const msMesh = Math.round(performance.now() - t0);
    const out = { ok: true, f0 };
    const n = r.modes.length;
    const st = o.statics === false ? null : await s.computeAtFrequency(0, r);  // static P2 FEM, same mesh
    r.modes.forEach((m, i) => {
      const name = modeName(m, i, n);
      for (const [k, val] of Object.entries(modeValues(m))) if (val !== undefined && val !== null) out[key(k, name)] = val;
      if (st) {
        const ms = st.modes.find((x) => x.mode === m.mode) || st.modes[i];
        out[key("z0", name) + "_static"] = ms.Z0;
        out[key("eeff", name) + "_static"] = ms.eps_eff_mode ?? ms.eps_eff;
      }
    });
    if (n === 2) {
      // odd/even keys as in backend_forsten.js: zodd, zeven, eeff_odd, …
      const mat = r.RLGC_matrix || {};
      out.mat_l = flat(mat.L); out.mat_c = flat(mat.C);
      out.zdiff = r.Z_diff; out.zcomm = r.Z_common;
    }
    out.info = { ...info(s, r), ms_mesh_and_f0: msMesh };
    out.ms = Math.round(performance.now() - t0);
    return out;
  } catch (e) {
    return { ok: false, error: String((e && e.message) || e), ms: Math.round(performance.now() - t0) };
  }
});

// Frequency sweep on one mesh, as upstream's FieldSolver2D.solve_sweep (mesh refined at the highest frequency,
// then every frequency in ascending order on the cached mesh and statics), keeping the full-wave quantities
// solve_sweep drops (eeff_mode, internal inductance, warnings).
export const solveSweep = quiet(async (text, freqs, o = {}) => {
  const t0 = performance.now();
  try {
    const fs = [...freqs].sort((a, b) => a - b);
    const fmax = fs[fs.length - 1];
    const s = makeSolver(text, fmax, { causal: true, ...o });
    s._sweepFmax = fmax;  // upstream's hint (solve_sweep): size the MQS skin band for the whole sweep
    const init = await s.solve_adaptive(adaptive(o));
    const msMesh = Math.round(performance.now() - t0);
    const out = { ok: true, f: [], modes: {}, ms_point: [] };
    const pair = init.modes.length === 2;
    if (pair) { out.zdiff = []; out.zcomm = []; out.mat = { r: [], l: [], g: [], c: [] }; }
    const warn = new Set();
    for (const f of fs) {
      const t1 = performance.now();
      const r = await s.computeAtFrequency(f, init);
      out.f.push(f);
      r.modes.forEach((m, i) => {
        const name = modeName(m, i, r.modes.length);
        const dst = out.modes[name] || (out.modes[name] = {});
        for (const [k, v] of Object.entries(modeValues(m))) {
          if (k === "eigen_bias" || k === "loss_via") continue;
          (dst[k] || (dst[k] = [])).push(v ?? null);
        }
      });
      if (pair) {
        out.zdiff.push(r.Z_diff); out.zcomm.push(r.Z_common);
        const M = r.RLGC_matrix || {};
        for (const k of ["R", "L", "G", "C"]) out.mat[k.toLowerCase()].push(flat(M[k]) || null);
      }
      for (const w of r.warnings || []) warn.add(w.type + (w.reason ? ":" + w.reason : ""));
      out.ms_point.push(Math.round(performance.now() - t1));
    }
    s._sweepFmax = null;
    out.info = { ...info(s, init), warnings: [...warn], ms_mesh: msMesh };
    out.ms = Math.round(performance.now() - t0);
    return out;
  } catch (e) {
    return { ok: false, error: String((e && e.message) || e), ms: Math.round(performance.now() - t0) };
  }
});

// εeff of a line from its RLGC (phase): for checks against the eigenmode value
export const phaseEeff = (r, l, g, c, f) => {
  const w = 2 * Math.PI * f;
  const a = r * g - w * w * l * c, b = w * (l * g + r * c);  // (R+jωL)(G+jωC)
  const mag = Math.hypot(a, b), beta = Math.sqrt((mag - a) / 2);
  return (beta / (w / C0)) ** 2;
};

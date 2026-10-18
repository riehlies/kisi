// kisi solver backend: Henrik Forstén's js_2d_fields (GPL-3.0), quasi-static
// rectilinear FDM, used unmodified through its custom-geometry text format.
//
// Contract of every kisi solver backend:
//   solveGeometry(text, {tol}) -> {ok, z0?, eeff?, zodd?, zeven?, eeff_odd?, eeff_even?,
//                                  r?, l?, g?, c?, r_odd?, l_odd?, g_odd?, c_odd?,
//                                  mat_l?, mat_c?, f0, ms, error?}
// Per-unit-length values at f0 = 1 GHz, smooth copper (σ = 5.8e7 S/m);
// odd-mode values per line; mat_* = 2x2 matrices (row major) of a pair.
// `text` is the cross-section in js_2d_fields custom geometry syntax, which
// kisi-core generates (crates/kisi-core/src/xsgeom.rs).
import { CustomGeometrySolver } from "../../third_party/js_2d_fields/src/custom_geometry.js";

export const BACKEND = { id: "forsten-fdm", name: "js_2d_fields (Forstén), FDM quasi-static" };
const F0 = 1e9;
const rlgc = (m, suffix = "") => {
  const x = (m && m.RLGC) || {};
  const out = {};
  for (const k of ["R", "L", "G", "C"]) if (Number.isFinite(x[k])) out[k.toLowerCase() + suffix] = x[k];
  return out;
};
const flat = (a) => (Array.isArray(a) && a.length === 2 ? [a[0][0], a[0][1], a[1][0], a[1][1]] : undefined);

export async function solveGeometry(text, { tol = 0.005, maxNodes = 60000 } = {}) {
  const t0 = performance.now();
  const log = console.log, warn = console.warn;
  console.log = () => {}; console.warn = () => {};
  try {
    const s = new CustomGeometrySolver({ text, freq: F0, sigma_cond: 5.8e7, rq: 0, nx: 10, ny: 10, mesh_backend: "rectilinear" });
    const r = await s.solve_adaptive({ energy_tol: tol, param_tol: tol, max_iters: 10, max_nodes: maxNodes });
    const ms = Math.round(performance.now() - t0);
    if (r.modes.length >= 2) {
      const odd = r.modes.find((m) => m.mode === "odd"), even = r.modes.find((m) => m.mode === "even");
      const mat = r.RLGC_matrix || {};
      return { ok: true, zodd: odd.Z0, zeven: even.Z0, eeff_odd: odd.eps_eff, eeff_even: even.eps_eff, ...rlgc(odd, "_odd"),
        mat_l: flat(mat.L), mat_c: flat(mat.C), f0: F0, ms };
    }
    return { ok: true, z0: r.modes[0].Z0, eeff: r.modes[0].eps_eff, ...rlgc(r.modes[0]), f0: F0, ms };
  } catch (e) {
    return { ok: false, error: String(e && e.message || e), ms: Math.round(performance.now() - t0) };
  } finally {
    console.log = log; console.warn = warn;
  }
}

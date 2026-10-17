// kisi solver backend: Henrik Forstén's js_2d_fields (GPL-3.0), quasi-static
// rectilinear FDM, used unmodified through its custom-geometry text format.
//
// Contract of every kisi solver backend:
//   solveGeometry(text, {tol}) -> {ok, z0?, eeff?, zodd?, zeven?, eeff_odd?, eeff_even?, ms, error?}
// `text` is the cross-section in js_2d_fields custom geometry syntax, which
// kisi-core generates (crates/kisi-core/src/xsgeom.rs).
import { CustomGeometrySolver } from "../../third_party/js_2d_fields/src/custom_geometry.js";

export const BACKEND = { id: "forsten-fdm", name: "js_2d_fields (Forstén), FDM quasi-statisch" };

export async function solveGeometry(text, { tol = 0.005, maxNodes = 60000 } = {}) {
  const t0 = performance.now();
  const log = console.log, warn = console.warn;
  console.log = () => {}; console.warn = () => {};
  try {
    const s = new CustomGeometrySolver({ text, freq: 1e9, nx: 10, ny: 10, mesh_backend: "rectilinear" });
    const r = await s.solve_adaptive({ energy_tol: tol, param_tol: tol, max_iters: 10, max_nodes: maxNodes });
    const ms = Math.round(performance.now() - t0);
    if (r.modes.length >= 2) {
      const odd = r.modes.find((m) => m.mode === "odd"), even = r.modes.find((m) => m.mode === "even");
      return { ok: true, zodd: odd.Z0, zeven: even.Z0, eeff_odd: odd.eps_eff, eeff_even: even.eps_eff, ms };
    }
    return { ok: true, z0: r.modes[0].Z0, eeff: r.modes[0].eps_eff, ms };
  } catch (e) {
    return { ok: false, error: String(e && e.message || e), ms: Math.round(performance.now() - t0) };
  } finally {
    console.log = log; console.warn = warn;
  }
}

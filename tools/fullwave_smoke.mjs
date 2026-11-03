// Smoke test of the full-wave backend build: one microstrip (w 0.6, h 0.3 mm, εr 4.4, CST ≈ 50 Ω) static and
// full-wave at 1 GHz, and three sweep points. Exit code 1 on a failure or an implausible value.
// Usage: KISI_FW_WASM_DIR=<dist> node tools/fullwave_smoke.mjs
import { loadFullwave } from "./fullwave_node.mjs";
const { solveGeometry, solveSweep, build } = await loadFullwave();
console.log("build:", build && build.emcc, build && build.upstream_rev);
const text = `units mm
bounds gnd gnd gnd gnd
domain -3.9 3.9 0 3.97
diel x=-inf w=inf y=0.035 h=0.3 er=4.4 tand=0.02
gnd x=-inf w=inf y=0 h=0.035
sig+ x=-0.3 w=0.6 y=0.335 h=0.035
`;
const r = await solveGeometry(text.replace("tand=0.02", "tand=0"), { tol: 0.01, lossMethod: "static" });
console.log(JSON.stringify(r));
const s = await solveSweep(text, [1e7, 1e9, 1e10], { tol: 0.01 });
console.log(JSON.stringify({ ok: s.ok, error: s.error, f: s.f, eeff: s.modes?.single?.eeff, z0: s.modes?.single?.z0,
  alpha_c: s.modes?.single?.alpha_c, alpha_d: s.modes?.single?.alpha_d, ms: s.ms, info: s.info }));
const bad = !r.ok || !(r.z0 > 45 && r.z0 < 55) || !(Math.abs(r.eeff_mode / r.eeff_static - 1) < 0.01) || !s.ok;
process.exit(bad ? 1 : 0);

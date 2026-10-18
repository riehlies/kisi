// Solve cross-sections with the upstream quasi-static solver (unmodified).
// Usage: node tools/solve_xs.mjs <in.json> <out.json> [--sparse path.wasm] [--tol 0.005] [--classic dist/kisi.html]
// --classic: use the classic-script solver embedded in a bundled viewer (tools/esm_classic.py)
//            instead of the ES modules, to check that the conversion changes nothing
// in.json: [{key, text}]  text = js_2d_fields custom geometry text
// out.json: [{key, ok, z0?, eeff?, zodd?, zeven?, eeff_odd?, eeff_even?, ms, error?}]
import fs from "node:fs";
import { register } from "node:module";
import path from "node:path";
import { fileURLToPath } from "node:url";
const here = path.dirname(fileURLToPath(import.meta.url));
register(new URL("./upstream_hooks.mjs", import.meta.url));
const args = process.argv.slice(2);
const opt = (k, d) => { const i = args.indexOf(k); return i >= 0 ? args[i + 1] : d; };
const sparse = opt("--sparse", path.join(here, "../target/wasm32-unknown-unknown/release/kisi_sparse.wasm"));
globalThis.__KISI_SPARSE_WASM__ = fs.readFileSync(sparse);
let solveGeometry;
if (opt("--classic")) {
  const html = fs.readFileSync(opt("--classic"), "utf8");
  const m = html.match(/const SOLVER_SRC = ("(?:[^"\\\n]|\\.)*");/);
  if (!m) throw new Error("SOLVER_SRC not found in " + opt("--classic"));
  ({ solveGeometry } = (0, eval)(JSON.parse(m[1])));
} else {
  ({ solveGeometry } = await import("../web/solver/backend_forsten.js"));
}
const cases = JSON.parse(fs.readFileSync(args[0], "utf8"));
const tol = parseFloat(opt("--tol", "0.01"));
const out = [];
for (const c of cases) {
  const r = await solveGeometry(c.text, { tol });
  out.push({ key: c.key, ...r });
  process.stderr.write(`${c.key}: ${r.ok ? (r.z0 ?? 2 * r.zodd).toFixed(2) : r.error} (${r.ms} ms)\n`);
}
fs.writeFileSync(args[1], JSON.stringify(out, null, 1));

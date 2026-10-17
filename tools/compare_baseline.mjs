// Compare field-solver results with the stored baseline (relative tolerance).
// Usage: node compare_baseline.mjs <baseline.json> <result.json>... [--write]
import fs from "node:fs";
const args = process.argv.slice(2);
const write = args.includes("--write");
const [basePath, ...files] = args.filter((a) => a !== "--write");
const val = (r) => r.z0 ?? (r.zodd != null ? 2 * r.zodd : null);
const now = {};
for (const f of files) for (const r of JSON.parse(fs.readFileSync(f, "utf8"))) now[r.key] = val(r);
if (write || !fs.existsSync(basePath)) {
  fs.writeFileSync(basePath, JSON.stringify(now, null, 1));
  console.log(`baseline written: ${Object.keys(now).length} values`);
  process.exit(0);
}
const base = JSON.parse(fs.readFileSync(basePath, "utf8"));
const TOL = 0.005;
let bad = 0;
for (const [k, v] of Object.entries(base)) {
  const n = now[k];
  const d = n == null ? Infinity : Math.abs(n - v) / v;
  if (d > TOL) { bad++; console.log(`changed: ${k}: ${v.toFixed(2)} -> ${n == null ? "missing" : n.toFixed(2)}`); }
}
console.log(bad ? `${bad} values moved by more than ${TOL * 100} %` : `all ${Object.keys(base).length} values within ${TOL * 100} % of baseline`);
process.exit(bad ? 1 : 0);

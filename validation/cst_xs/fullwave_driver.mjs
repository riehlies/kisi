// One task of run_fullwave.py in its own node process (an aborted WASM heap cannot affect the next task):
//   node validation/cst_xs/fullwave_driver.mjs <task.json> <result.json>
// task: {wasm_dir, kind: "point" | "sweep", text, freqs?, opts}
// result: the backend's result (web/solver/backend_forsten_fw.js) plus node and build information.
import fs from "node:fs";
import { loadFullwave } from "../../tools/fullwave_node.mjs";

const [taskPath, outPath] = process.argv.slice(2);
const task = JSON.parse(fs.readFileSync(taskPath, "utf8"));
const fw = await loadFullwave(task.wasm_dir || undefined);
const t0 = performance.now();
const r = task.kind === "sweep"
  ? await fw.solveSweep(task.text, task.freqs, task.opts || {})
  : await fw.solveGeometry(task.text, task.opts || {});
r.wall_s = (performance.now() - t0) / 1000;
r.node = process.version;
r.peak_rss_mb = Math.round(process.resourceUsage().maxRSS / 1024);
fs.writeFileSync(outPath, JSON.stringify(r));
process.exit(0);

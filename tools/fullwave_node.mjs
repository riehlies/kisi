// Load the full-wave backend in node: module hooks for both upstream WASM paths (FDM: kisi-sparse,
// tools/upstream_hooks.mjs; full-wave: tools/upstream_hooks_fw.mjs), then web/solver/backend_forsten_fw.js.
// wasmDir: the build of tools/build_forsten_fullwave.sh (default $KISI_FW_WASM_DIR or target/forsten-fw/dist).
import fs from "node:fs";
import { register } from "node:module";
import path from "node:path";
import { fileURLToPath } from "node:url";
const here = path.dirname(fileURLToPath(import.meta.url));
export async function loadFullwave(wasmDir = process.env.KISI_FW_WASM_DIR || path.join(here, "../target/forsten-fw/dist")) {
  for (const f of ["eigen_solver.js", "eigen_solver.wasm", "gmsh.js", "gmsh.wasm"])
    if (!fs.existsSync(path.join(wasmDir, f))) throw new Error(`${f} missing in ${wasmDir} (tools/build_forsten_fullwave.sh)`);
  register(new URL("./upstream_hooks.mjs", import.meta.url));
  register(new URL("./upstream_hooks_fw.mjs", import.meta.url), { data: { dir: path.resolve(wasmDir) } });
  // the FDM sparse solver, lazily: the triangular path does not use it, but upstream code shared by both may
  const sparse = path.join(here, "../target/wasm32-unknown-unknown/release/kisi_sparse.wasm");
  globalThis.__KISI_SPARSE_WASM__ = () => {
    if (!fs.existsSync(sparse)) throw new Error("kisi_sparse.wasm needed but not built: " + sparse);
    return fs.readFileSync(sparse);
  };
  const info = path.join(wasmDir, "BUILD_INFO.json");
  return { ...(await import("../web/solver/backend_forsten_fw.js")), wasmDir, build: fs.existsSync(info) ? JSON.parse(fs.readFileSync(info, "utf8")) : null };
}

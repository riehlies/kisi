// Node module hooks for the full-wave backend (web/solver/backend_forsten_fw.js): route upstream's imports of
// its WASM modules (src/tri_solver/eigen_module.js → '../wasm_solver/eigen_solver.js', occ_to_mesh.js →
// new URL('../wasm_solver/gmsh.js', import.meta.url)) to a build of tools/build_forsten_fullwave.sh, so the
// vendored upstream tree stays untouched. Registered next to tools/upstream_hooks.mjs (the FDM path), e.g.
//   register(new URL("./upstream_hooks_fw.mjs", import.meta.url), { data: { dir: "/path/to/dist" } })
// dir defaults to target/forsten-fw/dist. emscripten finds the .wasm next to the redirected .js.
import { pathToFileURL } from "node:url";
let DIR = new URL("../target/forsten-fw/dist/", import.meta.url);
export async function initialize(data) {
  if (data && data.dir) DIR = pathToFileURL(data.dir.replace(/\/?$/, "/"));
}
const MODULES = ["eigen_solver.js", "gmsh.js"];
export async function resolve(specifier, context, next) {
  for (const m of MODULES) if (specifier.endsWith("wasm_solver/" + m)) return { url: new URL(m, DIR).href, shortCircuit: true };
  return next(specifier, context);
}

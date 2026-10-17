// Node module hooks: route upstream's './wasm_solver/solver.js' import to the
// kisi-sparse shim, so the vendored upstream tree stays untouched.
const SHIM = new URL("../web/solver/sparse_module.js", import.meta.url).href;
export async function resolve(specifier, context, next) {
  if (specifier.endsWith("wasm_solver/solver.js")) return { url: SHIM, shortCircuit: true };
  return next(specifier, context);
}

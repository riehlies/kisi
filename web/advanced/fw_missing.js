// kisi-advanced.html built without the full-wave WebAssembly modules (tools/advanced_bundle.py without --fw):
// stands in for the original's Emscripten loaders wasm_solver/{eigen_solver,gmsh,fft}.js, which are not in its git
// (tools/build_forsten_fullwave.sh builds the first two). The quasi-static backend (potential, |E|) works without them.
export default async function missingModule() {
  throw new Error("This kisi-advanced.html has no full-wave backend (gmsh / eigen_solver WebAssembly modules not " +
    "built in). Choose the Quasi-static solver, or use a build with them (tools/advanced_bundle.py --fw).");
}

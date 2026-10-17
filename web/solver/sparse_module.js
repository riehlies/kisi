// kisi: drop-in replacement for js_2d_fields' src/wasm_solver/solver.js.
// Upstream builds that file from Eigen with emscripten; it is not in the
// upstream repository. This module offers the same Module shape (_malloc,
// _free, HEAP32, HEAPF64, _solve_sparse_multi) backed by the kisi-sparse
// WebAssembly build (Rust, faer).
//
// The wasm bytes come from globalThis.__KISI_SPARSE_WASM__: a Uint8Array /
// ArrayBuffer, or a function returning one (sync or async).
export default async function createWASMModule() {
  let src = globalThis.__KISI_SPARSE_WASM__;
  if (typeof src === "function") src = await src();
  if (!src) throw new Error("kisi-sparse wasm not provided (globalThis.__KISI_SPARSE_WASM__)");
  const { instance } = await WebAssembly.instantiate(src, {});
  const e = instance.exports;
  return {
    _malloc: (n) => e.ks_alloc(n),
    _free: (p) => e.ks_free(p),
    get HEAP32() { return new Int32Array(e.memory.buffer); },
    get HEAPF64() { return new Float64Array(e.memory.buffer); },
    _solve_sparse_multi: (...args) => e.ks_solve_sparse_multi(...args),
  };
}

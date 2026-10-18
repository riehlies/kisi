"use strict";
// ============================================================== field solver pool
// The solver is one classic script (SOLVER_SRC, built by tools/esm_classic.py):
// module workers from blob: URLs do not work when the page is opened from disk.
const WORKER_SRC = () => `
let backend = null;
self.onmessage = async (e) => {
  const m = e.data;
  if (m.type === "init") {
    globalThis.__KISI_SPARSE_WASM__ = m.sparse;
    try { backend = ${SOLVER_SRC}; self.postMessage({ type: "ready", info: backend.BACKEND }); }
    catch (err) { self.postMessage({ type: "fail", error: String(err && err.message || err) }); }
  } else if (m.type === "solve") {
    const r = await backend.solveGeometry(m.text, { tol: m.tol });
    self.postMessage({ type: "result", id: m.id, gen: m.gen, r });
  }
};`;
function solverOnPage() { return (0, eval)(SOLVER_SRC); }

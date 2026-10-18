"use strict";
// ============================================================== field solver pool
function solverModuleUrls() {
  if (solverModuleUrls.urls) return solverModuleUrls.urls;
  const urls = [];
  SOLVER_MODULES.forEach((m, i) => {
    const src = m.src.replace(/__KISI_MOD_(\d+)__/g, (_, k) => urls[+k]) + `\n//# sourceURL=kisi/${m.name}`;
    urls[i] = URL.createObjectURL(new Blob([src], { type: "text/javascript" }));
  });
  return (solverModuleUrls.urls = urls);
}
const WORKER_SRC = `
let backend = null;
self.onmessage = async (e) => {
  const m = e.data;
  if (m.type === "init") {
    globalThis.__KISI_SPARSE_WASM__ = m.sparse;
    try { backend = await import(m.entry); self.postMessage({ type: "ready", info: backend.BACKEND }); }
    catch (err) { self.postMessage({ type: "fail", error: String(err && err.message || err) }); }
  } else if (m.type === "solve") {
    const r = await backend.solveGeometry(m.text, { tol: m.tol });
    self.postMessage({ type: "result", id: m.id, gen: m.gen, r });
  }
};`;

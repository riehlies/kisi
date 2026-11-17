// kisi-advanced.html runtime (tools/advanced_bundle.py puts it at the top of the page's <head>).
//
// The fork's page (riehlies/js_2d_fields, field_solver.html) is a set of ES modules, a module worker, Plotly and
// WebAssembly files loaded by URL. A page opened from disk (file://) can load none of these, so the bundle keeps
// every file inside the page (gzip + base64, decoded on first use) and this script serves them:
//   * the modules are one classic script (tools/esm_classic.py), evaluated for the page and for the worker;
//     `import.meta.url` of a module is FAKE + its path in the fork's src/, so the URLs the code builds from it
//     (new URL("solve_worker.js", import.meta.url), an Emscripten loader's .wasm) point below FAKE;
//   * fetch() of a URL below FAKE answers from the embedded files (page and worker);
//   * new Worker(<URL below FAKE>) starts the bundled worker from a blob as a classic script; the worker asks the
//     page for embedded files by message (lazy: the 12 MB mesher is decoded only when the full-wave backend runs);
//   * a DOMContentLoaded listener added after the event (the modules run after decoding) is called at once.
// Nothing of the fork's or the original's code is changed by this; see docs/research/advanced-solvers.md.
"use strict";
(() => {
  const FAKE = "https://kisi-advanced.invalid/src/";
  const B64 = JSON.parse(document.getElementById("kisi-adv-files").textContent);
  const cache = new Map();
  const unb64 = (s) => Uint8Array.from(atob(s), (c) => c.charCodeAt(0));
  async function gunzip(bytes) {
    return new Uint8Array(await new Response(new Blob([bytes]).stream().pipeThrough(new DecompressionStream("gzip"))).arrayBuffer());
  }
  // an embedded file by its path in the fork's src/ (or a kisi name), decoded once
  function getFile(name) {
    if (!cache.has(name)) {
      if (!(name in B64)) return Promise.reject(new Error(`kisi-advanced: ${name} is not in this build`));
      cache.set(name, gunzip(unb64(B64[name])));
    }
    return cache.get(name);
  }
  const text = async (name) => new TextDecoder().decode(await getFile(name));
  const mime = (n) => n.endsWith(".wasm") ? "application/wasm" : n.endsWith(".js") ? "text/javascript" : "application/octet-stream";
  const t0 = performance.now();
  window.__kisiAdv = { t0, files: Object.keys(B64), has: (n) => n in B64, getFile, text };

  // fetch: files below FAKE from the bundle
  const nativeFetch = window.fetch.bind(window);
  window.fetch = async (u, o) => {
    const s = String(u instanceof Request ? u.url : u);
    if (s.startsWith(FAKE)) {
      const name = s.slice(FAKE.length).split(/[?#]/)[0];
      return new Response(await getFile(name), { headers: { "Content-Type": mime(name) } });
    }
    return nativeFetch(u, o);
  };
  // the kisi-sparse module that stands in for the original's solver.wasm (web/solver/sparse_module.js)
  globalThis.__KISI_SPARSE_WASM__ = () => getFile("kisi_sparse.wasm");

  // the worker: worker runtime + the module bundle, started with the worker's entry
  const WORKER_RT = `"use strict";
const FAKE = ${JSON.stringify(FAKE)};
let reqId = 0; const pending = new Map();
self.addEventListener("message", (e) => {
  const d = e.data;
  if (d && d.__kisiFile) { e.stopImmediatePropagation(); const p = pending.get(d.id); pending.delete(d.id); if (d.error) p.rej(new Error(d.error)); else p.res(d.bytes); }
});
const getFile = (name) => new Promise((res, rej) => { const id = ++reqId; pending.set(id, { res, rej }); self.postMessage({ __kisiGet: name, id }); });
const nativeFetch = self.fetch.bind(self);
self.fetch = async (u, o) => {
  const s = String(u instanceof Request ? u.url : u);
  if (s.startsWith(FAKE)) { const n = s.slice(FAKE.length).split(/[?#]/)[0]; return new Response(await getFile(n), { headers: { "Content-Type": n.endsWith(".wasm") ? "application/wasm" : "application/octet-stream" } }); }
  return nativeFetch(u, o);
};
globalThis.__KISI_SPARSE_WASM__ = () => getFile("kisi_sparse.wasm");
`;
  let workerURL = null;
  const NativeWorker = window.Worker;
  function KisiWorker(url, opts) {
    const s = String(url);
    if (!s.startsWith(FAKE)) return new NativeWorker(url, opts);
    const entry = window.__kisiAdvBundle.entries[s.slice(FAKE.length)];
    if (entry == null) throw new Error(`kisi-advanced: no bundled worker ${s.slice(FAKE.length)}`);
    if (!workerURL) {
      workerURL = URL.createObjectURL(new Blob([WORKER_RT, "(", window.__kisiAdvBundle.src, ")([", entry, "]);\n"], { type: "text/javascript" }));
    }
    const w = new NativeWorker(workerURL, { name: opts && opts.name });
    w.addEventListener("message", async (e) => {
      const d = e.data;
      if (!d || d.__kisiGet == null) return;
      e.stopImmediatePropagation();
      try { w.postMessage({ __kisiFile: 1, id: d.id, bytes: await getFile(d.__kisiGet) }); }
      catch (err) { w.postMessage({ __kisiFile: 1, id: d.id, error: String(err && err.message || err) }); }
    });
    return w;
  }
  KisiWorker.prototype = NativeWorker.prototype;
  window.Worker = KisiWorker;

  // DOMContentLoaded listeners added after the event (the modules run after decoding) are called at once
  for (const target of [window, document]) {
    const add = target.addEventListener.bind(target);
    target.addEventListener = (type, fn, opts) => {
      if (type === "DOMContentLoaded" && document.readyState !== "loading") { setTimeout(() => (typeof fn === "function" ? fn : fn.handleEvent.bind(fn))(new Event(type)), 0); return; }
      return add(type, fn, opts);
    };
  }

  // opened from a blob: URL (the page built into kisi.html): the original drops the applied link from the address
  // with replaceState(pathname + search), which a blob: URL cannot resolve; the same without the fragment instead
  const replaceState = history.replaceState.bind(history);
  history.replaceState = (st, title, url) => {
    try { return replaceState(st, title, url); } catch (e) { return replaceState(st, title, location.href.split("#")[0]); }
  };

  // decode the module bundle and Plotly, then start: Plotly by the page's own loader (its source replaced by the
  // blob URL at bundle time), the modules here
  window.__kisiReady = (async () => {
    const [src, plotly] = await Promise.all([text("kisi/bundle.js"), getFile("plotly-3.3.0.min.js")]);
    window.__kisiPlotlyURL = URL.createObjectURL(new Blob([plotly], { type: "text/javascript" }));
    const meta = JSON.parse(await text("kisi/bundle.json"));
    window.__kisiAdvBundle = { src, entries: meta.entries };
    window.__kisiAdv.tDecoded = performance.now();
  })();
  window.__kisiStartPage = () => {
    const run = (0, eval)("(" + window.__kisiAdvBundle.src + ")");
    run(window.__kisiAdvBundle.entries["#page"]);
    window.__kisiAdv.tStarted = performance.now();
  };
})();

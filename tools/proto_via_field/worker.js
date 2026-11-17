// Prototype via field viewer: kisi-fdtd (WebAssembly, feature proto-viewer) in a Web Worker started from a blob URL.
// Message in: {wasm: WebAssembly.Module, op: "mesh" | "run", req: {case, mesh, volume, port}}.
// Out: {type: "progress", step, db, mb} every ~0.25 s; {type: "done", blob (ArrayBuffer) | mesh, seconds, mb};
// {type: "error", message}. Cancel: the page terminates the worker (the run is one synchronous call).
self.onmessage = async (ev) => {
  const { wasm, op, req } = ev.data;
  let inst = null, last = 0;
  const mb = () => (inst ? inst.exports.memory.buffer.byteLength / 2 ** 20 : 0);
  try {
    inst = await WebAssembly.instantiate(wasm, {
      env: {
        fd_progress: (step, db) => {
          const t = performance.now();
          if (t - last > 250) { last = t; self.postMessage({ type: "progress", step, db, mb: mb() }); }
        },
      },
    });
    const k = inst.exports;
    const bytes = new TextEncoder().encode(JSON.stringify(req));
    const p = k.fd_alloc(bytes.length);
    new Uint8Array(k.memory.buffer, p, bytes.length).set(bytes);
    const t0 = performance.now();
    const code = op === "mesh" ? k.fd_mesh(p, bytes.length) : k.fd_run_volume(p, bytes.length);
    const seconds = (performance.now() - t0) / 1000;
    const out = new Uint8Array(k.memory.buffer, k.fd_out_ptr(), k.fd_out_len()).slice();
    if (code !== 0) throw new Error(JSON.parse(new TextDecoder().decode(out)).error);
    if (op === "mesh") self.postMessage({ type: "done", mesh: JSON.parse(new TextDecoder().decode(out)), seconds, mb: mb() });
    else self.postMessage({ type: "done", blob: out.buffer, seconds, mb: mb() }, [out.buffer]);
  } catch (e) {
    self.postMessage({ type: "error", message: String((e && e.message) || e) });
  }
};

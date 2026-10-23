"use strict";
// ============================================================== check workers
// The channel check computes its rows on the main thread and in Web Workers, each with an engine
// of its own that replays the main engine's state (Engine.replay: board, project file, kisi.json,
// variant, Touchstone files, analysis, lanes). A row is a pure function of that state, so it is
// the same whichever engine computes it; rows are put back in the order of the items.
// Threads: 4 with the main thread (as the CLIs), at most the cores; S.checkThreads overrides it.
// A worker first replays the state (about as long as loading the board); the workers stay for the
// next check (another profile, a recompute) and are dropped when the engine's state changes.
const CHECK_THREADS = 4;
const CHECK_CHUNK = 2; // items per request; a worker has two requests at a time
const CHECK_WORKER_SRC = `
const enc = new TextEncoder(), dec = new TextDecoder();
let k = null, ready = null;
function call(name, text) {
  const b = enc.encode(text);
  const p = k.kisi_alloc(b.length);
  new Uint8Array(k.memory.buffer, p, b.length).set(b);
  const rc = k[name](p, b.length);
  k.kisi_free(p, b.length);
  const o = dec.decode(new Uint8Array(k.memory.buffer, k.kisi_out_ptr(), k.kisi_out_len()));
  if (rc !== 0) throw new Error(o);
  return o;
}
self.onmessage = async (e) => {
  const m = e.data;
  if (m.type === "init") {
    ready = (async () => {
      k = (await WebAssembly.instantiate(m.module, {})).exports;
      for (const [fn, text] of m.log) call(fn, text);
    })();
    try { await ready; } catch (err) { self.postMessage({ type: "dead", error: String(err && err.message || err) }); }
  } else if (m.type === "check") {
    try { await ready; self.postMessage({ type: "rows", id: m.id, text: call("kisi_check", m.text) }); }
    catch (err) { self.postMessage({ type: "fail", id: m.id, error: String(err && err.message || err) }); }
  }
};`;
const CK = { gen: -1, workers: [], id: 0, wake: null, url: null };

function checkWake() { const w = CK.wake; CK.wake = null; if (w) w(); }
function checkWorkerDead(wk, why) {
  if (wk.dead) return;
  wk.dead = true;
  wk.w.terminate();
  if (why) console.warn(`check worker: ${why}`);
  for (const j of wk.jobs.values()) j.fail();
  wk.jobs.clear();
  checkWake();
}
// the engine's state changed: the workers' copies are stale
function checkPoolDrop() {
  for (const wk of CK.workers) checkWorkerDead(wk, null);
  CK.workers = []; CK.gen = -1;
  if (CK.url) { URL.revokeObjectURL(CK.url); CK.url = null; }
}
// workers for a check of n items (none for a few items or a single core)
function checkPool(n) {
  const threads = Math.min(S.checkThreads || Math.min(CHECK_THREADS, navigator.hardwareConcurrency || 1), Math.ceil(n / (2 * CHECK_CHUNK)));
  if (CK.gen !== S.engine.stateGen) checkPoolDrop();
  CK.workers = CK.workers.filter((wk) => !wk.dead);
  if (threads - 1 > CK.workers.length && S.engine.mod) {
    const url = CK.url || (CK.url = URL.createObjectURL(new Blob([CHECK_WORKER_SRC], { type: "text/javascript" })));
    const log = S.engine.replay();
    while (CK.workers.length < threads - 1) {
      let w;
      try { w = new Worker(url); } catch (e) { console.warn(e); break; }
      const wk = { w, dead: false, jobs: new Map() };
      w.onmessage = (e) => {
        const m = e.data, j = m.id != null ? wk.jobs.get(m.id) : null;
        if (m.type === "dead") return checkWorkerDead(wk, m.error);
        if (!j) return;
        wk.jobs.delete(m.id);
        if (m.type === "rows") j.ok(m.text); else { console.warn(m.error); j.fail(); }
        checkWake();
      };
      w.onerror = (e) => { e.preventDefault(); checkWorkerDead(wk, e.message || "error"); };
      w.postMessage({ type: "init", module: S.engine.mod, log });
      CK.workers.push(wk);
    }
    CK.gen = S.engine.stateGen;
    S.engine.onState = checkPoolDrop;
  }
  return CK.workers.slice(0, Math.max(0, threads - 1));
}

// kisi_check for all items, CHECK_CHUNK at a time, on the main thread and in the workers;
// progress(done, n, threads). Stops when the analysis changes (gen). Returns the rows of the
// chunks done, in their order: [[index of the chunk's first item, rows], …].
async function checkChunks(items, rf, gen, progress) {
  const starts = [];
  for (let i = 0; i < items.length; i += CHECK_CHUNK) starts.push(i);
  const res = new Array(starts.length), queue = starts.map((_, c) => c);
  const input = (c) => JSON.stringify({ rf, items: items.slice(starts[c], starts[c] + CHECK_CHUNK) });
  const workers = checkPool(items.length);
  S.checkWorkers = workers.length;
  let done = 0;
  const finish = (c, rows) => { res[c] = rows; done += Math.min(CHECK_CHUNK, items.length - starts[c]); };
  const feed = (wk) => {
    while (!wk.dead && wk.jobs.size < 2 && queue.length && gen === S.anGen) {
      const c = queue.shift(), id = ++CK.id;
      wk.jobs.set(id, { ok: (text) => { finish(c, JSON.parse(text)); feed(wk); }, fail: () => queue.unshift(c) });
      wk.w.postMessage({ type: "check", id, text: input(c) });
    }
  };
  workers.forEach(feed);
  for (;;) {
    while (queue.length && gen === S.anGen) {
      progress(done, items.length, workers.filter((wk) => !wk.dead).length + 1);
      await new Promise((r) => requestAnimationFrame(() => setTimeout(r, 0)));
      const c = queue.shift();
      if (c == null || gen !== S.anGen) break;
      try { S.engine.call(S.engine.k.kisi_check, input(c)); finish(c, JSON.parse(S.engine.out())); }
      catch (e) { console.warn(e); finish(c, []); }
      workers.forEach(feed);
    }
    if (gen !== S.anGen || !workers.some((wk) => wk.jobs.size)) break;
    progress(done, items.length, workers.filter((wk) => !wk.dead).length + 1);
    await new Promise((r) => { CK.wake = r; });
  }
  return res.map((rows, c) => [starts[c], rows]).filter((x) => x[1]);
}

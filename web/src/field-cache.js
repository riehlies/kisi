"use strict";
// ---- persistent cache of field-solver results (per viewer, IndexedDB) -------
// Key: upstream revision + geometry id. Geometry ids are content hashes, so
// results are shared between boards and survive reloads.
const FIELD_TOL = 0.01;
const Cache = {
  db: null,
  key: (id) => `${UPSTREAM_REV}|${FIELD_TOL}|v2|${id}`,
  async open() {
    try {
      this.db = await new Promise((res, rej) => {
        const r = indexedDB.open("kisi", 1);
        r.onupgradeneeded = () => r.result.createObjectStore("field");
        r.onsuccess = () => res(r.result);
        r.onerror = () => rej(r.error);
      });
    } catch (e) { this.db = null; }
  },
  async loadAll() {
    if (!this.db) return 0;
    const prefix = `${UPSTREAM_REV}|${FIELD_TOL}|v2|`;
    try {
      return await new Promise((res) => {
        let n = 0;
        const tx = this.db.transaction("field", "readonly");
        const req = tx.objectStore("field").openCursor();
        req.onsuccess = () => {
          const c = req.result;
          if (!c) return res(n);
          if (String(c.key).startsWith(prefix)) { S.field.set(String(c.key).slice(prefix.length), c.value); n++; }
          c.continue();
        };
        req.onerror = () => res(n);
      });
    } catch (e) { return 0; }
  },
  put(id, r) {
    if (!this.db) return;
    try { this.db.transaction("field", "readwrite").objectStore("field").put(r, this.key(id)); } catch (e) { /* storage full or blocked */ }
  },
  async clear() {
    S.field.clear();
    if (!this.db) return;
    try { await new Promise((res) => { const tx = this.db.transaction("field", "readwrite"); tx.objectStore("field").clear(); tx.oncomplete = res; tx.onerror = res; }); } catch (e) { /* ignore */ }
  },
};

class FieldPool {
  constructor() {
    this.n = Math.max(1, Math.min(6, (navigator.hardwareConcurrency || 2) - 1));
    this.workers = []; this.idle = []; this.queue = []; this.running = false; this.failed = null;
    this.lastApply = 0; this.dirty = false; this.mainThread = false; this.total = 0; this.done_n = 0;
    const sparse = b64ToBytes(SPARSE_B64);
    const urls = solverModuleUrls(), entry = urls[urls.length - 1];
    const wurl = URL.createObjectURL(new Blob([WORKER_SRC], { type: "text/javascript" }));
    for (let i = 0; i < this.n; i++) {
      let w;
      try { w = new Worker(wurl, { type: "module" }); } catch (e) { this.useMainThread(entry, sparse, e); return; }
      w.onmessage = (e) => this.onMessage(w, e.data);
      w.onerror = (e) => { if (!this.mainThread) this.useMainThread(entry, sparse, e.message || "worker error"); };
      w.postMessage({ type: "init", sparse, entry });
      this.workers.push(w);
    }
  }
  async useMainThread(entry, sparse, why) {
    if (this.mainThread) return;
    this.mainThread = true;
    this.workers.forEach((w) => w.terminate()); this.workers = []; this.idle = [];
    try {
      globalThis.__KISI_SPARSE_WASM__ = sparse;
      this.backend = await import(entry);
      this.n = 1; this.pump();
    } catch (e) {
      this.failed = String(e && e.message || e) + (why ? " / " + why : "");
      renderFieldStatus();
    }
  }
  onMessage(w, m) {
    if (m.type === "ready") { this.idle.push(w); this.pump(); }
    else if (m.type === "fail") { const entry = solverModuleUrls().at(-1); this.useMainThread(entry, b64ToBytes(SPARSE_B64), m.error); }
    else if (m.type === "result") { this.done(m.id, m.gen, m.r); this.idle.push(w); this.pump(); }
  }
  done(id, gen, r) {
    S.fieldPending.delete(id);
    S.field.set(id, r);
    Cache.put(id, r);
    this.done_n++;
    if (gen !== S.gen) return;
    this.dirty = true;
    const now = performance.now();
    const finished = this.queue.length === 0 && S.fieldPending.size === 0;
    if (finished) this.running = false;
    if (now - this.lastApply > 2000 || finished) this.apply();
    else renderFieldStatus();
  }
  apply() { this.lastApply = performance.now(); this.dirty = false; runAnalysis(); }
  reset() { this.queue = []; this.running = false; this.total = 0; this.done_n = 0; }
  start(items) {
    const seen = new Set();
    const g = items.filter((x) => !S.field.has(x.id) && !S.fieldPending.has(x.id) && !seen.has(x.id) && seen.add(x.id));
    g.sort((a, b) => (b.prio || 0) - (a.prio || 0));
    this.queue = g.map((x) => ({ id: x.id, text: x.text, gen: S.gen }));
    this.total = this.queue.length; this.done_n = 0;
    this.running = this.queue.length > 0;
    renderFieldStatus();
    this.pump();
  }
  async pump() {
    if (!this.running) return;
    if (this.mainThread) {
      if (this.busy || !this.backend) return;
      this.busy = true;
      while (this.running && this.queue.length) {
        const job = this.queue.shift();
        S.fieldPending.add(job.id);
        await new Promise((r) => setTimeout(r, 0));
        const r = await this.backend.solveGeometry(job.text, { tol: FIELD_TOL });
        this.done(job.id, job.gen, r);
      }
      this.busy = false;
      return;
    }
    while (this.idle.length && this.queue.length) {
      const w = this.idle.pop(), job = this.queue.shift();
      S.fieldPending.add(job.id);
      w.postMessage({ type: "solve", id: job.id, text: job.text, tol: FIELD_TOL, gen: job.gen });
    }
    renderFieldStatus();
  }
  stop() { this.queue = []; this.running = false; renderFieldStatus(); }
}

// nets in a solver scope -> geometry ids used by them
function scopeNets(scope) {
  const nets = new Set();
  if (scope === "sel") {
    if (S.selNet >= 0) { nets.add(S.selNet); const st = S.an.nets.find((x) => x.net === S.selNet); if (st && st.partner != null) nets.add(st.partner); }
  } else if (scope.startsWith("class:")) {
    const ci = +scope.slice(6);
    S.scene.net_class.forEach((c, n) => { if (c === ci) nets.add(n); });
  } else if (scope === "target") {
    S.scene.net_class.forEach((c, n) => { if (S.an.classes[c] && S.an.classes[c].target) nets.add(n); });
  } else {
    for (let n = 0; n < S.scene.nets.length; n++) nets.add(n);
  }
  return nets;
}
function scopeGeoms(scope) {
  const nets = scopeNets(scope), idx = new Set(), pc = S.pieces;
  for (let i = 0; i < pc.n; i++) {
    if (!nets.has(pc.net[i])) continue;
    const g = S.an.cross_sections[pc.xs[i]].geom;
    if (g != null) idx.add(g);
  }
  const items = [...idx].map((g) => { const e = S.an.geometries[g]; return { id: e.id, text: e.text, prio: 1e6 + e.target_length * 10 + e.length }; });
  // the selected net also gets its coupling cross-sections (crosstalk)
  if (scope === "sel" && S.selNet >= 0) {
    const r = rfFor(S.selNet);
    for (const [id, text] of r.crosstalk.geoms) items.push({ id, text, prio: 1 });
  }
  return items;
}

function renderFieldStatus() {
  if (!S.an) return;
  const pool = S.pool, geos = S.an.geometries;
  // scope selector
  const sel = $("fs-scope"), prev = sel.value;
  const opts = [];
  if (S.selNet >= 0) opts.push(["sel", "selected net " + netLabel(S.scene.nets[S.selNet]).split("/").pop()]);
  S.an.classes.forEach((c, i) => { if (c.target && c.nets) opts.push(["class:" + i, "class " + c.name]); });
  opts.push(["target", "all nets with target"], ["all", "all nets"]);
  const html = opts.map(([v, t]) => `<option value="${esc(v)}">${esc(t)}</option>`).join("");
  if (sel.dataset.html !== html) { sel.innerHTML = html; sel.dataset.html = html; }
  sel.value = opts.some((o) => o[0] === prev) ? prev : (S.selNet >= 0 ? "sel" : "target");
  const ids = scopeGeoms(sel.value);
  const open = ids.filter((x) => !S.field.has(x.id)).length;
  $("fs-start").textContent = open ? `compute ${open}` : "computed";
  $("fs-start").disabled = !open || (pool && pool.running) || (pool && pool.failed);
  // coverage
  const tgt = geos.filter((g) => g.target_length > 0);
  const totalT = tgt.reduce((a, g) => a + g.target_length, 0);
  const doneT = tgt.filter((g) => S.field.get(g.id)?.ok).reduce((a, g) => a + g.target_length, 0);
  const doneAll = geos.filter((g) => S.field.has(g.id)).length;
  const share = totalT ? doneT / totalT : 0;
  $("si-source").textContent = doneAll
    ? `Field solver for ${Math.round(share * 100)} % of the length with target, formulas elsewhere.`
    : "Fast analysis with formulas. Start the field solver for more accurate values.";
  $("fs-cache").textContent = `${S.field.size} cross-sections cached in this browser`;
  if (pool && pool.failed) {
    $("fs-text").textContent = "Field solver unavailable";
    $("fs-note").textContent = "Error: " + pool.failed + ". The map shows formula values.";
    $("fs-toggle").hidden = true;
    return;
  }
  const busy = pool && (pool.running || S.fieldPending.size > 0);
  $("fs-bar").style.width = busy && pool.total ? Math.round((pool.done_n / pool.total) * 100) + "%" : "0%";
  $("fs-text").textContent = busy ? `${pool.done_n} of ${pool.total} cross-sections` : "";
  $("fs-note").textContent = busy ? `Running on ${pool.n} ${pool.n === 1 ? "thread" : "threads"}, most important cross-sections first.` : "";
  $("fs-toggle").hidden = !busy;
  $("fs-toggle").textContent = "Pause";
}
$("fs-toggle").onclick = () => S.pool && S.pool.stop();
$("fs-start").onclick = () => {
  if (!S.pool) S.pool = new FieldPool();
  S.pool.start(scopeGeoms($("fs-scope").value));
};
$("fs-scope").onchange = () => renderFieldStatus();
$("fs-clear").onclick = async () => { await Cache.clear(); runAnalysis(); };

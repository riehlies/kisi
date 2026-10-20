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

// worker threads: "auto" = one less than the logical cores (max. 6), or a fixed number
const CORES = navigator.hardwareConcurrency || 2;
const THREADS_KEY = "kisi.threads.v1";
const autoThreads = () => Math.max(1, Math.min(6, CORES - 1));
function threadCount() { const t = lsGet(THREADS_KEY); return t ? Math.max(1, Math.min(CORES, t)) : autoThreads(); }
function initThreadSelect() {
  const sel = $("fs-threads"), cur = lsGet(THREADS_KEY);
  sel.innerHTML = `<option value="">auto (${autoThreads()})</option>` +
    Array.from({ length: CORES }, (_, i) => `<option value="${i + 1}" ${cur === i + 1 ? "selected" : ""}>${i + 1}</option>`).join("");
  sel.title = `${CORES} logical cores reported by the browser`;
  sel.onchange = () => {
    lsSet(THREADS_KEY, sel.value ? +sel.value : null);
    // takes effect now if idle, otherwise for the next run
    if (S.pool && !S.pool.running) { S.pool.terminate(); S.pool = null; }
    renderFieldStatus();
  };
}
initThreadSelect();

class FieldPool {
  constructor() {
    this.n = threadCount();
    this.workers = []; this.idle = []; this.queue = []; this.running = false; this.failed = null;
    this.lastApply = 0; this.dirty = false; this.mainThread = false; this.total = 0; this.done_n = 0;
    const sparse = SPARSE_WASM;
    const wurl = URL.createObjectURL(new Blob([WORKER_SRC()], { type: "text/javascript" }));
    for (let i = 0; i < this.n; i++) {
      let w;
      try { w = new Worker(wurl); } catch (e) { this.useMainThread(sparse, e); return; }
      w.onmessage = (e) => this.onMessage(w, e.data);
      w.onerror = (e) => { if (!this.mainThread) this.useMainThread(sparse, e.message || "worker error"); };
      w.postMessage({ type: "init", sparse });
      this.workers.push(w);
    }
  }
  async useMainThread(sparse, why) {
    if (this.mainThread) return;
    this.mainThread = true;
    this.workers.forEach((w) => w.terminate()); this.workers = []; this.idle = [];
    try {
      globalThis.__KISI_SPARSE_WASM__ = sparse;
      if (why) console.warn("kisi: field solver runs on the page (" + why + ")");
      this.backend = solverOnPage();
      this.n = 1; this.pump();
    } catch (e) {
      this.failed = String(e && e.message || e) + (why ? " / " + why : "");
      renderFieldStatus();
    }
  }
  onMessage(w, m) {
    if (m.type === "ready") { this.idle.push(w); this.pump(); }
    else if (m.type === "fail") { this.useMainThread(SPARSE_WASM, m.error); }
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
  terminate() { this.workers.forEach((w) => w.terminate()); this.workers = []; this.idle = []; }
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
  // coupling cross-sections (crosstalk), after the impedance ones: all of the selected net's,
  // for the other scopes those of the critical nets in it (most important first)
  if ($("fs-xt").checked) {
    if (scope === "sel" && S.selNet >= 0) {
      const r = rfFor(S.selNet);
      for (const [id, text] of r.crosstalk.geoms) items.push({ id, text, prio: 1 });
    } else if (S.xtScan) {
      const cg = S.xtScan.critical_geoms, wmax = Math.max(1e-9, ...cg.map((g) => g[2]));
      for (const [id, text, w, vic] of cg) if (scope === "all" || vic.some((v) => nets.has(v))) items.push({ id, text, prio: w / wmax, xt: true });
    }
  }
  return items;
}

function renderSteps() {
  const an = S.an, cls = an.classes.filter((c) => c.target && c.nets);
  const nets = an.nets.filter((n) => n.target).length;
  $("st1-text").textContent = `${S.scene.stackup.copper.length} copper layers${S.scene.stackup.from_file ? "" : " (assumed stackup)"}; ${cls.length} net class${cls.length === 1 ? "" : "es"} with a target impedance, ${nets} nets.`;
  $("st2").classList.toggle("done", !!S.full);
  $("an-start").hidden = !!S.full;
  $("st2-text").textContent = S.full
    ? `Done in ${S.anMs} ms: ${an.unique_cross_sections} distinct cross-sections. Formula values are typically within 5 % for single lines, 5–15 % off for coupled and coplanar lines.`
    : "Closed-form formulas (Hammerstad-Jensen, Wheeler, Wadell, IPC-2141) for every track, about a second. Typically within 5 % for single lines, 5–15 % off for coupled and coplanar lines.";
  $("st3").classList.toggle("locked", !S.full);
  renderXtCard();
}
$("an-start").onclick = () => estimateImpedances();

function renderFieldStatus() {
  if (!S.an) return;
  renderSteps();
  if (!S.full) {
    $("fs-start").textContent = "Start"; $("fs-start").disabled = true;
    $("si-source").textContent = "Available after step 2. 2D field solver (js_2d_fields) for each distinct cross-section: within 2 % of HFSS, seconds to minutes. Results are kept in this browser.";
    $("fs-cache").textContent = `${S.field.size} cross-sections cached in this browser`;
    $("fs-text").textContent = ""; $("fs-note").textContent = ""; $("fs-toggle").hidden = true; $("fs-bar").style.width = "0%";
    return;
  }
  const pool = S.pool, geos = S.an.geometries;
  // scope selector
  const sel = $("fs-scope"), prev = sel.value;
  const opts = [];
  if (S.selNet >= 0) opts.push(["sel", "selected net " + netShort(S.scene.nets[S.selNet])]);
  S.an.classes.forEach((c, i) => { if (c.target && c.nets) opts.push(["class:" + i, "class " + c.name]); });
  opts.push(["target", "all nets with target"], ["all", "all nets"]);
  const html = opts.map(([v, t]) => `<option value="${esc(v)}">${esc(t)}</option>`).join("");
  if (sel.dataset.html !== html) { sel.innerHTML = html; sel.dataset.html = html; }
  sel.value = opts.some((o) => o[0] === prev) ? prev : (S.selNet >= 0 ? "sel" : "target");
  const ids = scopeGeoms(sel.value);
  const open = ids.filter((x) => !S.field.has(x.id)).length;
  const openXt = ids.filter((x) => x.xt && !S.field.has(x.id)).length;
  $("fs-xt-label").textContent = `Refine critical couplings (crosstalk)${openXt ? ` · ${openXt}` : ""}`;
  $("fs-start").textContent = open ? `Compute ${open}` : "Computed";
  $("fs-start").disabled = !open || (pool && pool.running) || (pool && pool.failed);
  // coverage
  const tgt = geos.filter((g) => g.target_length > 0);
  const totalT = tgt.reduce((a, g) => a + g.target_length, 0);
  const doneT = tgt.filter((g) => S.field.get(g.id)?.ok).reduce((a, g) => a + g.target_length, 0);
  const doneAll = geos.filter((g) => S.field.has(g.id)).length;
  const share = totalT ? doneT / totalT : 0;
  $("st3").classList.toggle("done", totalT > 0 && share > 0.999);
  $("si-source").textContent = doneAll
    ? `Field solver for ${Math.round(share * 100)} % of the length with target, formulas elsewhere. `
    : "2D field solver (js_2d_fields) for each distinct cross-section: within 2 % of HFSS, seconds to minutes. Results are kept in this browser.";
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
  if (S.pool && !S.pool.running && !S.pool.mainThread && S.pool.n !== threadCount()) { S.pool.terminate(); S.pool = null; }
  if (!S.pool) S.pool = new FieldPool();
  S.pool.start(scopeGeoms($("fs-scope").value));
};
$("fs-scope").onchange = () => renderFieldStatus();
// coupling refinement on by default; the choice is kept per viewer
const FSXT_KEY = "kisi.fsxt.v1";
$("fs-xt").checked = lsGet(FSXT_KEY) !== false;
$("fs-xt").onchange = () => { lsSet(FSXT_KEY, $("fs-xt").checked ? null : false); renderFieldStatus(); };
$("fs-clear").onclick = async () => { await Cache.clear(); runAnalysis(); };

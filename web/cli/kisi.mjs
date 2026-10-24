#!/usr/bin/env node
// SPDX-License-Identifier: GPL-3.0-only
// kisi command line for Node.js (≥ 18): the same engine as the viewer (WASM) plus
// Forstén's 2D field solver in worker threads. Arguments, analyses and JSON
// results come from kisi_core::commands, exactly as in the native CLI; this file
// reads files, runs the field solver with a disk cache, computes the rows of `check`
// in worker threads with engines of their own and writes the report.
//
// Built into one file by tools/bundle_cli.py (dist/kisi.mjs):
//   node kisi.mjs <command> <board.kicad_pcb> [options]      (node kisi.mjs help)
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { Worker } from "node:worker_threads";
import zlib from "node:zlib";

const VERSION = "__VERSION__";
const UPSTREAM_REV = "__UPSTREAM_REV__";
const FIELD_TOL = 0.01; // as in the viewer, so cached results are the same
// engine, field solver's sparse module and solver script: gzip + base64 (tools/bundle_cli.py)
const gunzip = (b64) => zlib.gunzipSync(Buffer.from(b64, "base64"));
const WASM_GZ = "__WASM_GZ__";
const SPARSE_GZ = "__SPARSE_GZ__";
const SOLVER_GZ = "__SOLVER_GZ__";

// ---- engine (WASM, C ABI as in the viewer)
class Engine {
  constructor() {
    this.mod = new WebAssembly.Module(gunzip(WASM_GZ)); // compiled once, posted to check workers
    this.k = new WebAssembly.Instance(this.mod, {}).exports;
  }
  out() { return Buffer.from(new Uint8Array(this.k.memory.buffer, this.k.kisi_out_ptr(), this.k.kisi_out_len())).toString("utf8"); }
  call(name, value) {
    const b = Buffer.from(typeof value === "string" ? value : JSON.stringify(value), "utf8");
    const p = this.k.kisi_alloc(b.length);
    new Uint8Array(this.k.memory.buffer, p, b.length).set(b);
    const rc = this.k[name](p, b.length);
    this.k.kisi_free(p, b.length);
    const o = this.out();
    if (rc !== 0) throw new Error(o);
    return o;
  }
  json(name, value) { return JSON.parse(this.call(name, value)); }
}

// ---- files
const isFile = (p) => { try { return fs.statSync(p).isFile(); } catch { return false; } };
const read = (p) => { try { return fs.readFileSync(p, "utf8"); } catch (e) { throw new Error(`${p}: ${e.code === "ENOENT" ? "not found" : e.message}`); } };
const readJson = (p) => { const t = read(p); try { return JSON.parse(t); } catch (e) { throw new Error(`${p}: ${e.message}`); } };
const stem = (p) => path.basename(p).replace(/\.kicad_pcb$/i, "");

// ---- field solver: worker threads running the classic-script solver, results cached on disk
const cacheFile = () => {
  const base = process.env.KISI_CACHE || (process.platform === "win32" ? path.join(process.env.LOCALAPPDATA || os.homedir(), "kisi") : path.join(process.env.XDG_CACHE_HOME || path.join(os.homedir(), ".cache"), "kisi"));
  return path.join(base, "field-cache.json");
};
const cacheKey = (id) => `${UPSTREAM_REV}|${FIELD_TOL}|v2|${id}`; // same key as the viewer's IndexedDB
function loadCache(f) { try { return JSON.parse(fs.readFileSync(f, "utf8")); } catch { return {}; } }
function saveCache(f, c) {
  try {
    fs.mkdirSync(path.dirname(f), { recursive: true });
    const tmp = f + "." + process.pid + ".tmp";
    fs.writeFileSync(tmp, JSON.stringify(c));
    fs.renameSync(tmp, f);
  } catch (e) { process.stderr.write(`kisi: field cache not written (${e.message})\n`); }
}

const WORKER = `
const { parentPort, workerData } = require("node:worker_threads");
globalThis.__KISI_SPARSE_WASM__ = workerData.sparse;
const backend = (0, eval)(workerData.src);
parentPort.on("message", async (m) => {
  let r;
  try { r = await backend.solveGeometry(m.text, { tol: m.tol }); } catch (e) { r = { ok: false, error: String(e && e.message || e) }; }
  parentPort.postMessage({ id: m.id, r });
});`;

// solver script and sparse module, unpacked only when the field solver runs
let solverPayload = null;
const solverParts = () => solverPayload || (solverPayload = { src: gunzip(SOLVER_GZ).toString("utf8"), sparse: new Uint8Array(gunzip(SPARSE_GZ)) });
async function solveField(plan, budgetS, quiet) {
  const file = cacheFile(), cache = loadCache(file);
  const field = {}, todo = [];
  let fromCache = 0;
  for (const [id, text, w] of plan) {
    const c = cache[cacheKey(id)];
    if (c) { if (c.ok) field[id] = c; fromCache++; } else todo.push({ id, text, w });
  }
  const t0 = Date.now(), deadline = t0 + budgetS * 1000;
  let solved = 0, failed = 0;
  if (todo.length && budgetS > 0) {
    const n = Math.max(1, Math.min(todo.length, (os.availableParallelism ? os.availableParallelism() : os.cpus().length) - 1 || 1, 8));
    const solver = solverParts();
    const workers = Array.from({ length: n }, () => new Worker(WORKER, { eval: true, workerData: { src: solver.src, sparse: solver.sparse } }));
    let next = 0, lastMsg = 0;
    const totalW = plan.reduce((a, x) => a + x[2], 0) || 1;
    let doneW = plan.filter(([id]) => field[id]).reduce((a, x) => a + x[2], 0);
    const run = (wk) => new Promise((resolve) => {
      let job = null;
      const feed = () => {
        if (next >= todo.length || Date.now() > deadline) { wk.terminate(); resolve(); return; }
        job = todo[next++];
        wk.postMessage({ id: job.id, text: job.text, tol: FIELD_TOL });
      };
      wk.on("message", (m) => {
        cache[cacheKey(job.id)] = m.r;
        if (m.r.ok) { field[job.id] = m.r; solved++; doneW += job.w; } else failed++;
        if (!quiet && Date.now() - lastMsg > 2000) {
          lastMsg = Date.now();
          process.stderr.write(`kisi: field solver ${solved + failed}/${todo.length} new cross-sections, ${Math.round(100 * doneW / totalW)} % of the length covered, ${Math.round((Date.now() - t0) / 1000)} s\n`);
        }
        feed();
      });
      wk.on("error", (e) => { failed++; process.stderr.write(`kisi: solver worker failed: ${e.message}\n`); resolve(); });
      feed();
    });
    await Promise.all(workers.map(run));
    saveCache(file, cache);
  }
  const missing = plan.filter(([id]) => !cache[cacheKey(id)]).length;
  return { field, info: { planned: plan.length, from_cache: fromCache, solved_now: solved, failed_now: failed, not_solved: missing, seconds: Math.round((Date.now() - t0) / 100) / 10, cache: file,
    note: missing ? `${missing} cross-section(s) not solved within the time budget (--field-time ${budgetS}): formula values there; run again to continue (results are cached)` : undefined } };
}

// ---- check: rows in worker threads, each with an engine and the project of its own
// (kisi_core::commands::check_rows), handed out a few at a time; the main thread computes rows
// too. Rows are put together by their index in the check's order (kisi_cli_run with
// check_rows), so the result is the same as computing them in one engine. Workers: 4 with the
// main thread (as the native CLI's threads), fewer on machines with fewer CPUs; KISI_THREADS
// sets the number (1: no workers). A worker sets the project up itself (about as long as the
// main thread did), in parallel to the main thread.
const CHECK_THREADS = 4;
const checkThreads = () => {
  const n = Math.floor(Number(process.env.KISI_THREADS));
  return n >= 1 ? n : Math.min(CHECK_THREADS, os.availableParallelism ? os.availableParallelism() : os.cpus().length);
};
const CHECK_WORKER = `
const { parentPort, workerData } = require("node:worker_threads");
const k = new WebAssembly.Instance(workerData.module, {}).exports;
function call(name, value) {
  const b = Buffer.from(JSON.stringify(value), "utf8");
  const p = k.kisi_alloc(b.length);
  new Uint8Array(k.memory.buffer, p, b.length).set(b);
  const rc = k[name](p, b.length);
  k.kisi_free(p, b.length);
  const o = Buffer.from(new Uint8Array(k.memory.buffer, k.kisi_out_ptr(), k.kisi_out_len())).toString("utf8");
  if (rc !== 0) throw new Error(o);
  return o;
}
let failed = null;
const setup = (inputs) => { try { call("kisi_cli_project", inputs); failed = null; } catch (e) { failed = String(e && e.message || e); } };
setup(workerData.inputs);
parentPort.on("message", (m) => {
  if (m.setup) return setup(m.setup);
  let r;
  try { if (failed) throw new Error(failed); r = { ok: true, text: call("kisi_cli_check_part", m) }; } catch (e) { r = { ok: false, error: String(e && e.message || e) }; }
  parentPort.postMessage(r);
});`;
function startCheckWorkers(eng, inputs) {
  const n = checkThreads() - 1;
  return Array.from({ length: Math.max(0, n) }, () => new Worker(CHECK_WORKER, { eval: true, workerData: { module: eng.mod, inputs } }));
}
const CHUNK = 2; // rows per request; a worker has two requests at a time, so it never waits for the main thread
async function checkRows(eng, workers, o) {
  const { count } = eng.json("kisi_cli_check_part", { opts: o, items: [] });
  const rows = [], queue = Array.from({ length: count }, (_, i) => i);
  if (count < 2 || !workers.length) return null;
  const take = (n) => queue.splice(0, n);
  const jobs = workers.map((wk) => new Promise((resolve) => {
    const out = []; // requests sent, in order
    let done = false;
    const stop = (why) => {
      if (done) return;
      done = true;
      if (why) process.stderr.write(`kisi: check worker stopped (${why}), its rows are computed here\n`);
      for (const it of out.splice(0)) queue.unshift(...it);
      wk.terminate();
      resolve();
    };
    const send = () => { const it = take(CHUNK); if (!it.length) return false; out.push(it); wk.postMessage({ opts: o, items: it }); return true; };
    wk.on("message", (m) => {
      const it = out.shift();
      if (!m.ok) { if (it) queue.unshift(...it); stop(m.error); return; }
      rows.push(...JSON.parse(m.text).rows);
      if (!send() && !out.length) stop(null);
    });
    wk.on("error", (e) => stop(e.message));
    send(); send();
  }));
  const tick = () => new Promise((r) => setImmediate(r));
  for (;;) {
    while (queue.length) {
      rows.push(...eng.json("kisi_cli_check_part", { opts: o, items: take(1) }).rows);
      await tick(); // answers of the workers: their next rows
    }
    await Promise.all(jobs);
    if (!queue.length) return rows;
  }
}

// ---- main
async function main(argv) {
  const eng = new Engine();
  if (!argv.length || argv[0] === "--version") {
    if (argv[0] === "--version") { console.log(`kisi ${VERSION} (node, field solver ${UPSTREAM_REV.slice(0, 7)})`); return 0; }
  }
  let parsed;
  try { parsed = eng.json("kisi_cli_parse", argv.length ? argv : ["help"]); }
  catch (e) { console.log(JSON.stringify({ error: e.message }, null, 2)); process.stderr.write(`kisi: ${e.message}\n`); return 1; }
  const o = parsed.opts;
  if (o.cmd === "help") { console.log(parsed.help); return 0; }
  try {
    let tree = null;
    if (o.tree) tree = readJson(o.tree);
    if (parsed.needs_board) {
      const pcbPath = path.resolve(o.pos[0]);
      const dir = path.dirname(pcbPath);
      const pcb = read(pcbPath);
      const proPath = o.pro ? path.resolve(o.pro) : path.join(dir, stem(pcbPath) + ".kicad_pro");
      const pro = isFile(proPath) ? read(proPath) : null;
      let config = null;
      if (!o.no_config && o.cmd !== "config-template") {
        const cp = o.config ? path.resolve(o.config) : path.join(dir, stem(pcbPath) + ".kisi.json");
        if (o.config && !isFile(cp)) throw new Error(`${o.config}: not found`);
        if (isFile(cp)) { try { config = { path: cp, text: fs.readFileSync(cp, "utf8") }; } catch (e) { config = { path: cp, error: e.message }; } }
      }
      const stackup = o.stackup ? readJson(o.stackup) : null;
      // files the configuration refers to (paths relative to it): BOMs of assembly variants,
      // Touchstone files of connector ends
      const files = {};
      if (config && config.text) {
        let c = null;
        try { c = JSON.parse(config.text); } catch { /* reported by the engine */ }
        const want = [
          ...Object.values((c && c.variants) || {}).map((v) => v && v.bom),
          ...Object.values((c && c.connector_ends) || {}).map((e) => e && e.touchstone && e.touchstone.file),
        ];
        for (const f of want) {
          if (typeof f !== "string" || !f || files[f]) continue;
          const bp = path.resolve(path.dirname(config.path), f);
          try { files[f] = { text: fs.readFileSync(bp, "utf8") }; } catch (e) { files[f] = { error: `${bp}: ${e.code === "ENOENT" ? "not found" : e.message}` }; }
        }
      }
      const inputs = { opts: o, pcb_path: pcbPath, pcb, pro, config, stackup, field: {}, files };
      // check workers set the project up while the main thread does
      if (o.cmd === "check") var checkWorkers = startCheckWorkers(eng, inputs);
      const { plan } = eng.json("kisi_cli_project", inputs);
      var fieldInfo = null;
      if (plan.length) {
        const budget = o.field_time ?? 120;
        const { field, info } = await solveField(plan, budget, false);
        fieldInfo = info;
        if (Object.keys(field).length) {
          for (const wk of checkWorkers || []) wk.postMessage({ setup: { ...inputs, field } });
          eng.json("kisi_cli_project", { ...inputs, field });
        }
      }
      if (checkWorkers) {
        try { var checkRowsDone = await checkRows(eng, checkWorkers, o); }
        finally { for (const wk of checkWorkers) wk.terminate(); }
      }
      var reportPath = o.cmd === "report" ? (o.out ? path.resolve(o.out) : path.join(dir, "analysis", "kisi.json")) : null;
    }
    let res = eng.json("kisi_cli_run", { opts: o, tree, report_path: reportPath || null, check_rows: checkRowsDone || null });
    if (o.cmd === "report") {
      fs.mkdirSync(path.dirname(res.write.path), { recursive: true });
      fs.writeFileSync(res.write.path, res.write.content);
      res = res.result;
    }
    if (res && res._write) { fs.writeFileSync(path.resolve(res._write.path), res._write.content); res.written = res._write.path; delete res._write; }
    if (res && res._files) { // `export`: the files into a folder
      for (const f of res._files.files) {
        const p = path.join(path.resolve(res._files.dir), f.path);
        fs.mkdirSync(path.dirname(p), { recursive: true });
        fs.writeFileSync(p, f.content);
      }
      res.written = res._files.dir; delete res._files;
    }
    if (fieldInfo && res && typeof res === "object" && !Array.isArray(res)) res.field_solver_run = fieldInfo;
    console.log(JSON.stringify(res, null, 2));
    return 0;
  } catch (e) {
    console.log(JSON.stringify({ error: e.message }, null, 2));
    process.stderr.write(`kisi ${o.cmd}: ${e.message}\n`);
    return 1;
  } finally {
    for (const wk of checkWorkers || []) wk.terminate();
  }
}

process.exitCode = await main(process.argv.slice(2));

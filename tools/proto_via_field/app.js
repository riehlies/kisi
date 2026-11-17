// Prototype via field viewer: the page (settings, runs in workers, the viewer). Needs case.js, fields.js, view.js
// and window.KVF_DATA {wasm, example (gzip + base64), sigmaCu, sigmaSource, worker (source text), build}.
(function () {
  "use strict";
  const V = KVF.view, D = window.KVF_DATA;
  const $ = (id) => document.getElementById(id);
  const SIGMA_CU = D.sigmaCu;
  const st = {
    ds: null, exc: 0, fi: 0, view: "E", comp: "mag", axis: 1, ia: [0, 0, 0], arrows: true, outline: true, log: true,
    rangeDb: 40, lock: null, tmode: "avg", phase: 0, playing: false, speed: 0.5, kind: "single", running: null,
  };
  window.KVF_STATE = st; // for the headless checks

  // ---- helpers --------------------------------------------------------------------------------------------------
  const b64 = (s) => { const bin = atob(s), u = new Uint8Array(bin.length); for (let i = 0; i < bin.length; i++) u[i] = bin.charCodeAt(i); return u; };
  async function gunzip(u8) {
    const ds = new DecompressionStream("gzip");
    return new Uint8Array(await new Response(new Blob([u8]).stream().pipeThrough(ds)).arrayBuffer());
  }
  async function gzip(u8) {
    const cs = new CompressionStream("gzip");
    return new Uint8Array(await new Response(new Blob([u8]).stream().pipeThrough(cs)).arrayBuffer());
  }
  let wasmMod = null;
  async function wasmModule() {
    if (!wasmMod) wasmMod = await WebAssembly.compile(await gunzip(b64(D.wasm)));
    return wasmMod;
  }
  const workerURL = URL.createObjectURL(new Blob([D.worker], { type: "text/javascript" }));
  function job(op, req, onProgress) {
    const w = new Worker(workerURL);
    let rej = null;
    const p = new Promise(async (resolve, reject) => {
      rej = reject;
      w.onmessage = (ev) => {
        const m = ev.data;
        if (m.type === "progress") onProgress && onProgress(m);
        else if (m.type === "done") { w.terminate(); resolve(m); }
        else { w.terminate(); reject(new Error(m.message)); }
      };
      w.onerror = (e) => { w.terminate(); reject(new Error(e.message || "worker error")); };
      w.postMessage({ wasm: await wasmModule(), op, req });
    });
    return { worker: w, promise: p, cancel: () => { w.terminate(); rej(new Error("cancelled")); } };
  }
  const mW = (w) => V.fmt(w * 1e3, 3);
  const pct = (w, ref) => { if (!ref) return ""; const v = (100 * w) / ref; return (Math.abs(v) < 0.005 ? "0.00" : v.toFixed(Math.abs(v) < 10 ? 2 : 1)) + " %"; };
  const db = ([re, im]) => (20 * Math.log10(Math.hypot(re, im) || 1e-30)).toFixed(2);
  const segs = (id, cb) => {
    const el = $(id);
    el.addEventListener("click", (e) => {
      const b = e.target.closest("button");
      if (!b || b.disabled) return;
      for (const x of el.querySelectorAll("button")) x.classList.toggle("on", x === b);
      cb(b.dataset.v);
    });
  };
  const setSeg = (id, v) => { for (const x of $(id).querySelectorAll("button")) x.classList.toggle("on", x.dataset.v === String(v)); };
  let RATE = 60e6; // cells·steps per second of one WebAssembly worker (default; replaced by this device's last run)
  try { const r = +localStorage.getItem("kvf-rate"); if (r > 1e6) RATE = r; } catch (e) { /* storage unavailable */ }

  // ---- settings -----------------------------------------------------------------------------------------------
  function params() {
    const n = (id) => +$(id).value;
    return { kind: st.kind, stackup: $("stackup").value, layers: n("layers"), thickness: n("thickness"), er: n("er"), tand: n("tand"),
      drill: n("drill"), pad: n("pad"), antipad: n("antipad"), pitch: n("pitch"), gnd: n("gnd"), gndDist: n("gndDist"),
      feed: n("feed"), mesh: $("mesh").value };
  }
  function freqs() {
    const f = [...document.querySelectorAll("#fchips .chip.on")].map((b) => +b.dataset.f * 1e9);
    for (const s of $("fother").value.split(/[,;\s]+/)) { const v = parseFloat(s); if (v > 0 && v <= 100) f.push(v * 1e9); }
    return [...new Set(f.map((x) => Math.round(x)))].sort((a, b) => a - b);
  }
  // dielectric loss reference: the geometric mean of the field frequencies (exact for one frequency)
  const fLoss = (fs) => (fs.length ? Math.exp(fs.reduce((a, f) => a + Math.log(f), 0) / fs.length) : 0);
  function excitePorts() {
    return [...document.querySelectorAll("#exciteBox input:checked")].map((x) => +x.value);
  }
  function renderExcite() {
    const pair = st.kind === "pair";
    const opts = pair ? [[0, "1 P near", true], [1, "2 N near", true], [2, "3 P far", false], [3, "4 N far", false]] : [[0, "port 1", true], [1, "port 2", false]];
    $("exciteBox").innerHTML = opts.map(([v, l, on]) => `<label style="margin-right:8px"><input type="checkbox" value="${v}"${on ? " checked" : ""}> ${l}</label>`).join("") +
      (pair ? `<div class="note">Ports 1 and 2 give the odd (differential) and even (common) mode by superposition.</div>` : "");
    $("exciteBox").onchange = estimate;
  }
  let estTimer = null, estSeq = 0;
  function estimate() {
    clearTimeout(estTimer);
    estTimer = setTimeout(doEstimate, 250);
    for (const el of document.querySelectorAll(".uni")) el.classList.toggle("hidden", $("stackup").value !== "uniform");
    for (const el of document.querySelectorAll(".pr")) el.classList.toggle("hidden", st.kind !== "pair");
    const fs = freqs();
    $("flossNote").innerHTML = fs.length ? `Dielectric loss: the solver's conductivity is set from tanδ at f<sub>loss</sub> = ${(fLoss(fs) / 1e9).toFixed(2)} GHz (geometric mean of the field frequencies; exact when one frequency is chosen).` : "Choose at least one frequency.";
  }
  async function doEstimate() {
    const seq = ++estSeq, fs = freqs(), ports = excitePorts();
    if (!fs.length || !ports.length) { $("estimate").textContent = !fs.length ? "Choose at least one frequency." : "Choose a port to excite."; $("run").disabled = true; return; }
    let kase;
    try { kase = KVF.makeCase(params(), fs); } catch (e) { $("estimate").textContent = e.message; return; }
    $("traceNote").textContent = `50 Ω feeds (closed form): ${kase.meta.trace_w[0]} mm on L1, ${kase.meta.trace_w[1]} mm on the bottom layer; f_max of the pulse ${kase.fmax / 1e9} GHz.`;
    try {
      const { mesh } = await job("mesh", { case: kase }).promise;
      if (seq !== estSeq) return;
      const E = estimateOf(kase, mesh, fs.length, ports.length);
      st.estimate = E;
      const par = Math.min(ports.length, Math.max(1, Math.min(2, navigator.hardwareConcurrency || 1)));
      $("estimate").innerHTML = `<b>${(E.cells / 1e3).toFixed(0)} k cells</b> (${(E.nodes / 1e3).toFixed(0)} k field nodes) · ≈${E.steps} steps<br>` +
        `memory ≈ <b>${E.mbWorker.toFixed(0)} MB</b> per worker (${ports.length} run${ports.length > 1 ? "s" : ""}, ${par} at a time) + ${E.mbPage.toFixed(0)} MB in the page<br>` +
        `time ≈ <b>${fmtS(E.secs * Math.ceil(ports.length / par))}</b> <span class="note">(estimate at ${(RATE / 1e6).toFixed(0)} Mcells/s${localStorageRate() ? ", measured on this device" : ""})</span>`;
      const mobile = /Mobi|Android|iPhone|iPad/.test(navigator.userAgent) || (window.matchMedia && matchMedia("(pointer: coarse)").matches);
      const big = E.mbWorker > 300 || E.mbWorker * par + E.mbPage > 600;
      $("warn").classList.toggle("hidden", !big);
      if (big) $("warn").innerHTML = `<b>Large run.</b> About ${(E.mbWorker * par + E.mbPage).toFixed(0)} MB in total. Phones and tablets (iOS Safari in particular, often 0.3–1 GB per page) may reload the page or stop the run${mobile ? " – this looks like such a device" : ""}. No limit is enforced; use a coarser mesh or fewer frequencies if it fails.`;
      $("run").disabled = !!st.running;
    } catch (e) { $("estimate").textContent = "Mesh failed: " + e.message; }
  }
  const localStorageRate = () => { try { return !!localStorage.getItem("kvf-rate"); } catch (e) { return false; } };
  const fmtS = (s) => (s < 90 ? `${Math.max(1, Math.round(s))} s` : `${(s / 60).toFixed(1)} min`);
  // cells, nodes, steps, memory and time of one run (calibrated on kisi-fdtd runs, docs/research/fdtd-field-view.md)
  function estimateOf(kase, mesh, nf, nruns) {
    const cells = mesh.x.length * mesh.y.length * mesh.z.length, d = kase.domain;
    const inside = (a, lo, hi) => a.filter((v) => v >= lo - 1e-9 && v <= hi + 1e-9).length;
    const nodes = inside(mesh.x, d[0], d[3]) * inside(mesh.y, d[1], d[4]) * inside(mesh.z, d[2], d[5]);
    const dmin = (a) => { let m = Infinity; for (let i = 1; i < a.length; i++) m = Math.min(m, a[i] - a[i - 1]); return m * 1e-3; };
    const dt = 0.95 / (299792458 * Math.sqrt(dmin(mesh.x) ** -2 + dmin(mesh.y) ** -2 + dmin(mesh.z) ** -2));
    const tsim = (0.107 + 3.33 / (kase.fmax / 1e9)) * 1e-9; // simulated time to −50 dB energy (fit of the cases here)
    const steps = Math.ceil(tsim / dt / 200) * 200 + 1;
    const mbWorker = (75 * cells + (96 * nf + 62) * nodes) / 2 ** 20 + 4;
    const mbPage = (nruns * nf * 48 * nodes + 27 * nodes) / 2 ** 20;
    return { cells, nodes, steps, dt, mbWorker, mbPage, secs: (cells * steps) / RATE + 0.5 };
  }

  // ---- runs -----------------------------------------------------------------------------------------------------
  async function run() {
    const fs = freqs(), ports = excitePorts();
    if (!fs.length || !ports.length) return;
    const kase = KVF.makeCase(params(), fs);
    kase.f_loss = fLoss(fs);
    const req = (port) => ({ case: kase, mesh: null, volume: { freqs: fs, stride: 1, every: 0 }, port });
    const par = Math.min(ports.length, Math.max(1, Math.min(2, navigator.hardwareConcurrency || 1)));
    const prog = $("progress");
    prog.innerHTML = ports.map((p) => `<div class="prog" id="pl${p}">port ${p + 1}: waiting</div><progress id="pb${p}" max="1" value="0"></progress>`).join("");
    const t0 = performance.now();
    st.running = { jobs: [], cancelled: false };
    $("run").disabled = true;
    $("cancel").disabled = false;
    const results = {};
    const estSteps = st.estimate ? st.estimate.steps : 3000;
    try {
      const queue = ports.slice();
      const worker = async () => {
        while (queue.length && !st.running.cancelled) {
          const port = queue.shift(), ts = performance.now();
          const j = job("run", req(port), (m) => {
            $("pb" + port).value = Math.min(0.98, m.step / estSteps);
            $("pl" + port).textContent = `port ${port + 1}: step ${m.step} · field energy ${m.db.toFixed(1)} dB (stops at ${kase.end_db} dB) · ${m.mb.toFixed(0)} MB · ${((performance.now() - ts) / 1000).toFixed(1)} s`;
          });
          st.running.jobs.push(j);
          $("pl" + port).textContent = `port ${port + 1}: setting up the mesh and the conformal faces…`;
          const r = await j.promise;
          results[port] = r;
          $("pb" + port).value = 1;
          $("pl" + port).textContent = `port ${port + 1}: done in ${r.seconds.toFixed(1)} s · WebAssembly memory ${r.mb.toFixed(0)} MB`;
        }
      };
      await Promise.all(Array.from({ length: par }, worker));
      if (st.running.cancelled) throw new Error("cancelled");
      const blobs = ports.map((p) => results[p].blob);
      const ds = KVF.fromRaw(blobs.map((b) => new Uint8Array(b)), kase);
      for (const r of ds.runs) { r.seconds = results[r.port].seconds; r.wasm_mb = results[r.port].mb; }
      const wall = (performance.now() - t0) / 1000;
      ds.meta = { source: "run in this browser", wall_s: wall, workers: par };
      // this device's rate for the next estimate
      const rr = ds.runs[0];
      if (rr.steps && rr.cells && rr.seconds > 1) { RATE = (rr.cells * rr.steps) / rr.seconds; try { localStorage.setItem("kvf-rate", String(RATE)); } catch (e) { /* ignore */ } }
      window.KVF_LAST_RUN = { wall_s: wall, runs: ds.runs.map((r) => ({ port: r.port, seconds: r.seconds, wasm_mb: r.wasm_mb, steps: r.steps, cells: r.cells })), nodes: ds.n, freqs: ds.freqs };
      setDataset(ds);
      prog.insertAdjacentHTML("beforeend", `<div class="prog">All runs: ${wall.toFixed(1)} s wall time.</div>`);
    } catch (e) {
      prog.insertAdjacentHTML("beforeend", `<div class="warn">${e.message === "cancelled" ? "Run cancelled." : "Run failed: " + e.message}</div>`);
      window.KVF_LAST_RUN = { error: e.message };
    } finally {
      st.running = null;
      $("run").disabled = false;
      $("cancel").disabled = true;
      estimate();
    }
  }
  function cancel() {
    if (!st.running) return;
    st.running.cancelled = true;
    for (const j of st.running.jobs) j.cancel(); // terminates the worker; run() reports "cancelled"
  }

  // ---- data sets ------------------------------------------------------------------------------------------------
  function setDataset(ds) {
    st.ds = ds;
    st.excs = KVF.excitationsOf(ds);
    st.exc = 0;
    st.fi = Math.min(st.fi, ds.freqs.length - 1);
    const k = ds.case;
    st.lock = null;
    $("dsTitle").textContent = (k && k.meta && k.meta.title) || ds.name;
    const src = ds.meta && ds.meta.source ? ds.meta.source : "";
    $("dsSub").textContent = `${src ? src + " · " : ""}${ds.nx}×${ds.ny}×${ds.nz} field nodes (${(ds.n / 1e3).toFixed(0)} k), ${ds.runs.length} run${ds.runs.length > 1 ? "s" : ""}` +
      `${ds.runs[0].cells ? `, ${(ds.runs[0].cells / 1e3).toFixed(0)} k cells, ${ds.runs[0].steps} steps` : ""}${ds.mantissaBits && ds.mantissaBits < 10 ? `, stored with ${ds.mantissaBits}-bit mantissa (≈${(100 * 2 ** -(ds.mantissaBits + 1)).toFixed(1)} %)` : ""}` +
      `${ds.f_loss ? ` · f_loss ${(ds.f_loss / 1e9).toFixed(2)} GHz` : ""}`;
    $("exc").innerHTML = st.excs.map((e, i) => `<option value="${i}">${e.label}</option>`).join("");
    $("freq").innerHTML = ds.freqs.map((f, i) => `<option value="${i}">${+(f / 1e9).toFixed(3)} GHz</option>`).join("");
    $("freq").value = st.fi;
    // starting plane: y through the (first) signal via; z in the middle; x at the via
    const sig = k ? k.vias.find((v) => (v.net || "SIG") === "SIG") : null;
    st.ia = [V.nearest(ds.x, sig ? sig.x : 0), V.nearest(ds.y, sig ? sig.y : 0), V.nearest(ds.z, (ds.z[0] + ds.z[ds.nz - 1]) / 2)];
    $("save").disabled = false;
    updatePlaneUI();
    draw();
  }
  async function loadFiles(files) {
    const blobs = [];
    let kase = null, ds = null;
    for (const f of files) {
      let u = new Uint8Array(await f.arrayBuffer());
      if (u[0] === 0x1f && u[1] === 0x8b) u = await gunzip(u);
      if (KVF.isKVF(u)) ds = KVF.decode(u);
      else if (u[0] === 0x7b) kase = JSON.parse(new TextDecoder().decode(u));
      else blobs.push(u);
    }
    if (!ds && blobs.length) { ds = KVF.fromRaw(blobs, kase && kase.case ? kase.case : kase); ds.meta = { source: "kisi-fdtd blobs" }; }
    if (!ds) throw new Error("no field data in the chosen files");
    ds.meta = Object.assign({}, ds.meta || {}, { source: "loaded from " + files.map((f) => f.name).join(", ") });
    setDataset(ds);
  }
  async function save() {
    if (!st.ds) return;
    const z = await gzip(KVF.encode(st.ds));
    const a = document.createElement("a");
    a.href = URL.createObjectURL(new Blob([z], { type: "application/octet-stream" }));
    a.download = `${st.ds.name}.kvf`;
    a.click();
    setTimeout(() => URL.revokeObjectURL(a.href), 5000);
  }
  async function loadExample() {
    const ds = KVF.decode(await gunzip(b64(D.example)));
    ds.meta = Object.assign({}, ds.meta || {}, { source: "precomputed example (" + (ds.meta && ds.meta.made ? ds.meta.made : "native kisi-fdtd") + ")" });
    setDataset(ds);
  }

  // ---- viewer ---------------------------------------------------------------------------------------------------
  function jumps(ds, axis) {
    const k = ds.case, out = [];
    if (!k) return out;
    if (axis === 2) {
      (k.meta.copper_z || []).forEach((z, i) => out.push([`L${i + 1} (copper)`, z]));
      k.layers.forEach((l, i) => out.push([`middle of dielectric ${i + 1}`, (l.z0 + l.z1) / 2]));
      out.sort((a, b) => b[1] - a[1]);
    } else if (axis === 0) {
      k.ports.forEach((p, i) => out.push([`port ${i + 1}`, p.x]));
      out.push(["signal via", 0], ["between port 1 and the via", k.ports[0].x / 2], ["between the via and the far port", k.ports[k.ports.length - 1].x / 2]);
    } else {
      for (const v of k.vias) if ((v.net || "SIG") === "SIG") out.push([`signal via at y = ${v.y}`, v.y]);
      if (k.meta.kind === "pair") out.push(["between the pair", 0]);
      for (const v of k.vias) if (v.net === "GND" && Math.abs(v.x) < 1e-9) out.push([`GND via at y = ${v.y}`, v.y]);
    }
    const seen = new Set();
    return out.map(([l, v]) => [l, +v.toFixed(6)]).filter(([, v]) => (seen.has(v) ? false : seen.add(v)));
  }
  function updatePlaneUI() {
    const ds = st.ds;
    if (!ds) return;
    const n = [ds.nx, ds.ny, ds.nz][st.axis];
    $("planeSlider").max = n - 1;
    $("planeSlider").value = st.ia[st.axis];
    setSeg("axis", st.axis);
    const js = jumps(ds, st.axis);
    $("jump").innerHTML = `<option value="">jump to…</option>` + js.map(([l, v]) => `<option value="${v}">${l}</option>`).join("");
  }
  function currentScale(pv) {
    let mx = 0;
    const src = st.tmode === "inst" || pv.signed ? pv.bound : pv.val;
    for (let p = 0; p < src.length; p++) { const a = Math.abs(src[p]); if (a > mx && isFinite(a)) mx = a; }
    const key = `${st.view}:${st.comp}:${st.tmode}`;
    if ($("lock").checked) { if (!st.lock || st.lock.key !== key) st.lock = { key, max: mx }; mx = st.lock.max; } else st.lock = null;
    return { max: mx || 1, log: st.log, rangeDb: st.rangeDb, signed: pv.signed, power: st.view === "S" || st.view === "loss" };
  }
  const UNITS = { E: "V/m", H: "A/m", S: "W/m²", loss: "W/m³" };
  function labelOf() {
    const c = st.comp === "mag" ? "" : "xyz"[st.comp];
    const inst = st.tmode === "inst";
    if (st.view === "loss") return "dielectric loss ½σ|E|² (W/m³)";
    if (st.view === "S") return inst ? (c ? `s${c}(t) = (e×h)${c}` : "|s(t)| = |e(t)×h(t)|") + " (W/m²)" : (c ? `⟨S⟩${c}` : "|⟨S⟩|") + " = ½Re(E×H*) (W/m²)";
    const F = st.view;
    return (inst ? (c ? `${F.toLowerCase()}${c}(t)` : `|${F.toLowerCase()}(t)|`) : (c ? `|${F}${c}|` : `|${F}|`)) + ` (${UNITS[F]}, peak)`;
  }
  let planeCache = { key: "" };
  function getPlane() {
    const ds = st.ds, e = st.excs[st.exc];
    const key = `${st.fi}:${st.exc}:${st.axis}:${st.ia[st.axis]}:${ds.name}:${ds.n}`;
    if (planeCache.key !== key) {
      const F = KVF.fieldsOf(ds, st.fi, e.w);
      planeCache = { key, F, pl: KVF.plane(ds, F, st.axis, st.ia[st.axis]) };
    }
    return planeCache;
  }
  const condCache = new Map(), sumCache = new Map();
  function condOf() {
    const key = `${st.ds.name}:${st.ds.n}:${st.fi}:${st.exc}`;
    if (!condCache.has(key)) {
      const F = KVF.fieldsOf(st.ds, st.fi, st.excs[st.exc].w);
      const c = KVF.conductorLoss(st.ds, F, st.ds.freqs[st.fi], SIGMA_CU);
      let mx = 0;
      for (const m of c.sheets.values()) for (const v of m) if (v > mx) mx = v;
      for (const b of c.barrels) for (const r of b.dens) for (const v of r) if (v > mx) mx = v;
      c.max = mx || 1;
      condCache.set(key, c);
      if (condCache.size > 6) condCache.delete(condCache.keys().next().value);
    }
    return condCache.get(key);
  }
  function summaryOf() {
    const key = `${st.ds.name}:${st.ds.n}:${st.fi}:${st.exc}`;
    if (!sumCache.has(key)) {
      const s = KVF.summary(st.ds, st.fi, st.excs[st.exc].w, null);
      s.cond = condOf();
      sumCache.set(key, s);
      if (sumCache.size > 6) sumCache.delete(sumCache.keys().next().value);
    }
    return sumCache.get(key);
  }
  let lastL = null, lastPV = null;
  function draw() {
    const ds = st.ds;
    if (!ds) return;
    const { pl, F } = getPlane();
    const phase = st.tmode === "inst" ? st.phase : null;
    const comp = st.comp === "mag" ? "mag" : +st.comp;
    const pv = KVF.planeValues(pl, st.view, st.view === "loss" ? "mag" : comp, st.view === "loss" ? null : phase);
    // arrows of E/H in the phasor view: the real vector at the phase slider's φ
    if (phase == null && (st.view === "E" || st.view === "H")) pv.arrows = KVF.planeValues(pl, st.view, "mag", st.phase).arrows;
    const scale = currentScale(pv);
    const cond = st.view === "loss" ? condOf() : null;
    const cmap = st.view === "loss" ? "inferno" : pv.signed ? "diverging" : "viridis";
    lastPV = pv;
    lastL = V.draw($("cv"), { ds, pl, pv, view: st.view, scale, cmap, arrows: st.arrows && st.view !== "loss", outline: st.outline, cond,
      condMax: cond ? cond.max : 1, maxH: Math.max(260, Math.min(560, window.innerHeight * 0.62)),
      labels: { unit: labelOf(), condUnit: "conductor ½Rs|Ht|² (W/m², estimate)" } });
    const pos = [ds.x, ds.y, ds.z][st.axis][st.ia[st.axis]];
    $("planePos").textContent = `${"xyz"[st.axis]} = ${pos.toFixed(3)} mm`;
    $("phaseLab").textContent = `${Math.round((st.phase * 180) / Math.PI) % 360}°`;
    V.locator($("loc"), ds, st.axis, pos);
    // flux through the plane
    const P = KVF.flux(ds, F, st.axis, Math.max(1, st.ia[st.axis]));
    const s = summaryOf();
    const arrowNote = st.arrows && st.view !== "loss" ? (st.view === "S" && st.tmode !== "inst" ? "arrows: direction of the time-averaged S in the plane" : `arrows: the real ${st.view === "S" ? "s(t)" : st.view.toLowerCase() + "(t) = Re{" + st.view + "·e^{jφ}}"} in the plane at φ = ${Math.round((st.phase * 180) / Math.PI) % 360}°`) : "";
    $("planeInfo").innerHTML = `<b>∫S·dA through this plane</b> (+${"xyz"[st.axis]} direction): <b>${mW(P)} mW</b> = ${pct(P, s.pin)} of the incident ${mW(s.pin)} mW` +
      `${ds.stride !== 1 ? " (stride > 1: approximate)" : ""}. ${arrowNote}${st.view === "loss" ? ' · metal coloured by the conductor-loss <span class="est">estimate</span> (blue scale)' : ""}`;
    totals(s, P);
    hover(null);
  }
  function totals(s, P) {
    const ds = st.ds, f = ds.freqs[st.fi], pin = s.pin, c = s.cond;
    const L = (name, w, cls, extra) => `<tr class="${cls || ""}"><td>${name}</td><td class="n">${mW(w)}</td><td class="n">${extra != null ? extra : pct(w, pin)}</td></tr>`;
    let h = `<tr class="head"><td>at ${+(f / 1e9).toFixed(3)} GHz, ${st.excs[st.exc].label}</td><td class="n">mW</td><td class="n">of incident</td></tr>`;
    h += L("incident", pin);
    h += L("reflected (near-end ports, from S)", s.refl);
    h += L("transmitted (far-end ports, from S)", s.trans);
    h += L("lost or radiated (incident − reflected − transmitted)", s.balance);
    h += L("dielectric loss ∫½σ|E|²dV", s.diel.total, "sub");
    if (ds.case) ds.case.layers.forEach((l, i) => { if (s.diel.perLayer[i] > 1e-3 * s.diel.total) h += `<tr class="sub"><td>&nbsp;&nbsp;dielectric ${i + 1} (z ${l.z0}–${l.z1} mm, εr ${l.er})</td><td class="n">${mW(s.diel.perLayer[i])}</td><td class="n">${pct(s.diel.perLayer[i], pin)}</td></tr>`; });
    h += L("leaves the box (into the PML: radiation)", s.out, "sub");
    h += L("sum of the two (balance check)", s.diel.total + s.out, "sub", `Δ ${pct(s.diel.total + s.out - s.balance, pin)}`);
    const fl = s.f_loss;
    if (Math.abs(f / fl - 1) > 1e-3) h += `<tr class="sub"><td>dielectric with tanδ at this f <span class="est">estimate</span> (the solver's σ is tanδ at f_loss = ${(fl / 1e9).toFixed(2)} GHz; ×f/f_loss)</td><td class="n">${mW(s.dielMaterial)}</td><td class="n">${pct(s.dielMaterial, pin)}</td></tr>`;
    h += `<tr class="head"><td>conductor loss <span class="est">estimate</span></td><td class="n">mW</td><td class="n"></td></tr>`;
    if (c && c.exact) {
      h += L("total, ½Rs|Ht|² on the PEC surfaces", c.total);
      for (const [k, v] of Object.entries(c.regions)) if (v > 0) h += L(k, v, "sub");
      h += `<tr class="sub"><td colspan="3">Rs = ${(c.Rs * 1e3).toFixed(1)} mΩ (smooth copper, σ = ${(SIGMA_CU / 1e7).toFixed(2)}·10⁷ S/m, ${D.sigmaSource}). Not in the S-parameters (PEC); edge current crowding under-resolved, no roughness.</td></tr>`;
    } else h += `<tr class="sub"><td colspan="3">needs stride 1</td></tr>`;
    h += `<tr class="head"><td>this plane</td><td class="n">mW</td><td></td></tr>` + L(`∫S·dA (+${"xyz"[st.axis]})`, P);
    $("totals").innerHTML = h;
    $("totHead").textContent = "Power balance";
    // S-parameters at the nearest S grid point
    const { b, f: fs } = KVF.waves(ds, f, st.excs[st.exc].w);
    const names = ds.case && ds.case.meta ? ds.case.meta.ports.map((p) => p.name) : b.map((_, i) => String(i + 1));
    let t = `<tr class="head"><td>outgoing waves b/a (${+(fs / 1e9).toFixed(2)} GHz)</td><td class="n">dB</td><td class="n">phase</td></tr>`;
    b.forEach((x, i) => { t += `<tr><td>port ${names[i]}</td><td class="n">${db(x)}</td><td class="n">${((Math.atan2(x[1], x[0]) * 180) / Math.PI).toFixed(0)}°</td></tr>`; });
    if (ds.case && ds.case.meta && ds.case.meta.kind === "pair" && ds.runs.some((r) => r.port === 0) && ds.runs.some((r) => r.port === 1) && b.length === 4) {
      const col = (port) => KVF.waves(ds, f, [[port, 1]]).b;
      const c1 = col(0), c2 = col(1);
      const mm = (r1, r2, sg1, sg2) => [0.5 * (c1[r1][0] + sg1 * c2[r1][0] + sg2 * (c1[r2][0] + sg1 * c2[r2][0])), 0.5 * (c1[r1][1] + sg1 * c2[r1][1] + sg2 * (c1[r2][1] + sg1 * c2[r2][1]))];
      t += `<tr class="head"><td>mixed mode (100 Ω / 25 Ω)</td><td class="n">dB</td><td></td></tr>`;
      t += `<tr><td>Sdd11</td><td class="n">${db(mm(0, 1, -1, -1))}</td><td></td></tr><tr><td>Sdd21</td><td class="n">${db(mm(2, 3, -1, -1))}</td><td></td></tr>`;
      t += `<tr><td>Scd21 (diff. → common)</td><td class="n">${db(mm(2, 3, -1, 1))}</td><td></td></tr><tr><td>Scc21</td><td class="n">${db(mm(2, 3, 1, 1))}</td><td></td></tr>`;
    }
    $("sparams").innerHTML = t;
    const e = st.excs[st.exc];
    $("normNote").innerHTML = `Fields are peak phasors for a <b>1 V peak incident voltage wave</b> at each driven port into its reference impedance (${ds.runs[0].z0} Ω): a = 1 V/√Z0, incident power 1/(2·Z0) = ${mW(1 / (2 * ds.runs[0].z0))} mW per port.` +
      (e.w.length > 1 ? ` This mode is (F₁ ${e.w[1][1] < 0 ? "−" : "+"} F₂)/√2 of the two single-port runs: ±1/√2 V at ports 1 and 2 (differential voltage √2 V into 100 Ω for the odd mode), the same incident power as one port.` : "");
  }
  let lastPointer = null;
  function hover(ev) {
    if (!st.ds || !lastL) return;
    if (ev) lastPointer = { clientX: ev.clientX, clientY: ev.clientY };
    if (!lastPointer) return;
    const r = $("cv").getBoundingClientRect(), px = lastPointer.clientX - r.left, py = lastPointer.clientY - r.top, L = lastL;
    if (px < L.ml || px > L.ml + L.pw || py < L.mt || py > L.mt + L.ph) return;
    const [uu, vv] = V.fromPx(L, px, py), ds = st.ds;
    const { ua, va, u, v } = V.planeAxes(ds, st.axis);
    const a = V.nearest(u, uu), b = V.nearest(v, vv), p = a * planeCache.pl.nv + b;
    const pos = [0, 0, 0];
    pos[st.axis] = [ds.x, ds.y, ds.z][st.axis][st.ia[st.axis]];
    pos[ua] = u[a];
    pos[va] = v[b];
    const pl = planeCache.pl;
    const mag = (A) => Math.sqrt(A.reduce((s, x) => s + x[p] * x[p], 0));
    const Sv = KVF.poyntingAvg(pl.E, pl.H, p);
    const val = lastPV.val[p];
    $("hover").textContent = `x ${pos[0].toFixed(3)}  y ${pos[1].toFixed(3)}  z ${pos[2].toFixed(3)} mm (node ${a}, ${b})\n` +
      `${labelOf().replace(/ \(.*$/, "")} = ${V.fmt(val)} ${st.view === "loss" ? "W/m³" : UNITS[st.view]}   |E| ${V.fmt(mag(pl.E))} V/m  |H| ${V.fmt(mag(pl.H))} A/m  ⟨S⟩ (${Sv.map((x) => V.fmt(x, 1)).join(", ")}) W/m²  ½σ|E|² ${V.fmt(pl.loss[p])} W/m³`;
  }

  // ---- animation ------------------------------------------------------------------------------------------------
  let lastT = 0;
  function frame(t) {
    if (!st.playing) return;
    const dt = lastT ? Math.min(0.1, (t - lastT) / 1000) : 0;
    lastT = t;
    st.phase = (st.phase + 2 * Math.PI * st.speed * dt) % (2 * Math.PI);
    $("phase").value = Math.round((st.phase * 180) / Math.PI) % 360;
    draw();
    requestAnimationFrame(frame);
  }
  function play(on) {
    st.playing = on == null ? !st.playing : on;
    $("play").textContent = st.playing ? "❚❚" : "▶";
    if (st.playing) {
      if (st.tmode !== "inst" && st.view !== "loss") { st.tmode = "inst"; setSeg("tmode", "inst"); }
      lastT = 0;
      requestAnimationFrame(frame);
    }
  }

  // ---- wiring ---------------------------------------------------------------------------------------------------
  // the GND via count is remembered per case kind (defaults: 4 around a single via, 2 beside a pair)
  const gndByKind = { single: "4", pair: "2" };
  $("gnd").addEventListener("change", () => { gndByKind[st.kind] = $("gnd").value; });
  segs("kind", (v) => { st.kind = v; renderExcite(); $("gnd").value = gndByKind[v]; estimate(); });
  for (const id of ["stackup", "layers", "thickness", "er", "tand", "drill", "pad", "antipad", "pitch", "gnd", "gndDist", "feed", "mesh", "fother"]) $(id).addEventListener("input", estimate);
  $("fchips").addEventListener("click", (e) => { const b = e.target.closest(".chip"); if (b) { b.classList.toggle("on"); estimate(); } });
  $("run").onclick = run;
  $("cancel").onclick = cancel;
  $("file").onchange = (e) => loadFiles([...e.target.files]).catch((er) => alert(er.message));
  $("save").onclick = save;
  $("example").onclick = () => loadExample();
  $("exc").onchange = (e) => { st.exc = +e.target.value; draw(); };
  $("freq").onchange = (e) => { st.fi = +e.target.value; draw(); };
  segs("view", (v) => { st.view = v; draw(); });
  segs("comp", (v) => { st.comp = v; draw(); });
  segs("axis", (v) => { st.axis = +v; updatePlaneUI(); draw(); });
  segs("scale", (v) => { st.log = v === "log"; draw(); });
  segs("tmode", (v) => { st.tmode = v; if (v === "avg") play(false); draw(); });
  $("range").onchange = (e) => { st.rangeDb = +e.target.value; draw(); };
  $("lock").onchange = () => draw();
  $("arrows").onchange = (e) => { st.arrows = e.target.checked; draw(); };
  $("outline").onchange = (e) => { st.outline = e.target.checked; draw(); };
  $("planeSlider").addEventListener("input", (e) => { st.ia[st.axis] = +e.target.value; draw(); });
  $("jump").onchange = (e) => { if (e.target.value === "") return; const ds = st.ds; st.ia[st.axis] = V.nearest([ds.x, ds.y, ds.z][st.axis], +e.target.value); $("planeSlider").value = st.ia[st.axis]; e.target.value = ""; draw(); };
  $("phase").addEventListener("input", (e) => { st.phase = (+e.target.value * Math.PI) / 180; draw(); });
  $("play").onclick = () => play();
  $("speed").onchange = (e) => { st.speed = +e.target.value; };
  $("cv").addEventListener("pointermove", hover);
  $("cv").addEventListener("pointerdown", hover);
  let rz = null;
  window.addEventListener("resize", () => { clearTimeout(rz); rz = setTimeout(draw, 120); });
  window.KVF_APP = { draw, run, cancel, loadExample, setDataset, play, st, estimateNow: doEstimate };
  $("buildNote").textContent = D.build || "";
  renderExcite();
  estimate();
  loadExample().catch((e) => { $("dsTitle").textContent = "Example could not be loaded: " + e.message; });
})();

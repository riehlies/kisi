"use strict";
// ============================================================== loading
// results of the boards opened in this session, kept in memory: switching to
// another board and back restores estimate, checks, voltage drop and Z(f)
const BOARD_CACHE_MAX = 3;
const BOARD_STATE = ["overrides", "full", "anMs", "mode", "chMode", "check", "laneRes", "chStatus", "selLane",
  "piRes", "piSolved", "piMs", "pdnRes", "pdnSolved", "pdnMs", "piRail", "piLayer", "piMode", "piTab", "piGround", "groundWarn", "loss", "lossPlane",
  "therm", "thermLayer", "piTree", "treeRes", "piSetups", "piWarn", "layerOn", "zoneLayer", "view"];
function boardKey(name, text) {
  // name, size and a sampled FNV hash of the file
  let h = 2166136261;
  const step = Math.max(1, Math.floor(text.length / 8192));
  for (let i = 0; i < text.length; i += step) { h ^= text.charCodeAt(i); h = Math.imul(h, 16777619); }
  return `${name}|${text.length}|${(h >>> 0).toString(16)}`;
}
function saveBoardState() {
  if (!S.scene || !S.boardKey) return;
  const snap = {};
  for (const k of BOARD_STATE) snap[k] = S[k];
  S.boardCache = S.boardCache || new Map();
  S.boardCache.delete(S.boardKey);
  S.boardCache.set(S.boardKey, snap);
  while (S.boardCache.size > BOARD_CACHE_MAX) S.boardCache.delete(S.boardCache.keys().next().value);
}

// small texts (the board configuration): hash every character
function textHash(t) { let h = 2166136261; for (let i = 0; i < t.length; i++) { h ^= t.charCodeAt(i); h = Math.imul(h, 16777619); } return (h >>> 0).toString(16); }

// cfg: {name, text} of a <board>.kisi.json (values with sources), applied over the board
async function loadFiles(pcbText, proText, name, piDefaults = null, cfg = null) {
  $("drop").hidden = true; $("drop").classList.remove("start");
  showBusy("Reading board and computing cross-sections…");
  await new Promise((r) => requestAnimationFrame(() => setTimeout(r, 0)));
  try {
    saveBoardState();
    // assembly variant chosen in this browser (wins over the configuration's)
    let vchoice = lsGet(variantKey(name));
    const key = boardKey(name, pcbText + "|" + (proText || "")) + (cfg ? "|" + textHash(cfg.text + JSON.stringify(cfg.csv || {})) : "") + "|v=" + (vchoice || "");
    const snap = S.boardCache && S.boardCache.get(key);
    const t0 = performance.now();
    S.engine.loadPro(proText || '{"net_settings":{}}');
    S.engine.loadPcb(pcbText);
    S.lastFiles = { pcbText, proText, name, piDefaults, cfg };
    S.config = null;
    S.variant = null;
    if (cfg) {
      try { S.config = { name: cfg.name, ...S.engine.config(cfg.text, name, cfgFiles(cfg), vchoice) }; S.variant = S.config.variant; }
      catch (e) { S.config = { name: cfg.name, check: { ok: false, errors: [e.message], warnings: [], entries: 0 }, targets: {}, profiles: {} }; }
    }
    try {
      if (!S.variant) S.variant = vchoice ? S.engine.variant(vchoice) : S.engine.variants();
      if (vchoice && !S.variant.chosen && S.variant.errors.length) { lsSet(variantKey(name), null); vchoice = null; S.variant = S.engine.variant(null); }
    } catch (e) { S.variant = null; console.warn(e); }
    S.scene = S.engine.scene();
    $("app").classList.remove("no-board");
    S.padSet = new Set(S.scene.pad_tracks || []);
    // synthetic pad track → its pad (drawn as the pad's outline, not as a stroke)
    S.padOfTrack = new Map((S.scene.pad_tracks || []).map((t, i) => [t, (S.scene.pad_track_pad || [])[i]]));
    S.padCustom = new Map((S.scene.pad_custom || []).map(([i, polys]) => [i, polys]));
    S.zones = S.engine.zones();
    S.name = name;
    S.overrides = {};
    S.selNet = -1; S.hover = -1; S.piMark = null; S.check = null; S.chStatus = null; S.chCache = null; S.laneRes = null; S.selLane = null; S.chMode = "o";
    S.full = false; S.mode = "type"; // step 2 (formula impedances) not run yet; line type first
    S.xtScan = null; S.xtMs = null; S.xtHover = -1;
    S.fieldPending = new Set(); S.gen++;
    if (S.pool) S.pool.reset();
    const n = S.scene.stackup.copper.length;
    S.layerOn = new Array(n).fill(true);
    // copper zones hidden at the start ("Show copper zones: none"); the choice is kept per board
    S.zoneLayer = -1;
    S.boardKey = key.replace(/\|v=.*$/, "") + "|v=" + (vchoice || "");
    const tParse = performance.now() - t0;
    if (snap) {
      // back to a board of this session: same inputs, re-run the (fast) analysis for
      // the engine, keep the slow results
      S.overrides = snap.overrides; S.full = snap.full; S.mode = snap.mode; S.chMode = snap.chMode; S.anMs = snap.anMs;
      if (snap.layerOn && snap.layerOn.length === n) S.layerOn = snap.layerOn;
      S.zoneLayer = snap.zoneLayer;
    }
    runAnalysis(tParse);
    buildStatic();
    initPi(piDefaults);
    if (snap) {
      for (const k of ["piRes", "piSolved", "piMs", "pdnRes", "pdnSolved", "pdnMs", "piRail", "piLayer", "piMode", "piTab", "piGround", "groundWarn", "loss", "lossPlane", "therm", "thermLayer", "piTree", "treeRes", "piSetups", "piWarn", "laneRes", "chStatus", "selLane"]) S[k] = snap[k];
      if (snap.check) S.check = { ...snap.check, gen: S.anGen, stale: false };
      syncModeButtons();
      setTab(S.tab);
      S.view = { ...snap.view };
      draw();
    } else fitView();
    hideBusy();
  } catch (e) {
    hideBusy();
    $("warnings").innerHTML = `<span class="err">Could not read file: ${esc(e.message)}</span>`;
    console.error(e);
  }
}

// stackup changes per board (viewer only; the file is not modified)
const stackKey = () => "kisi.stack.v1." + S.name;
function stackOv() {
  if (!S.stackOv || S.stackOv.board !== S.name) S.stackOv = { board: S.name, dielectrics: {}, ...(lsGet(stackKey()) || {}) };
  S.stackOv.board = S.name;
  return S.stackOv;
}
function stackEff(i) { return { ...S.scene.stackup.dielectrics[i], ...(stackOv().dielectrics[i] || {}) }; }
function stackEdited(i) { return !!stackOv().dielectrics[i]; }
function stackParam() {
  return { dielectrics: Object.entries(stackOv().dielectrics || {}).map(([i, o]) => ({ index: +i, ...o })) };
}

// step 1 (on load): board, net classes, targets – S.an is a skeleton without impedances
// step 2 (button): cross-sections and formula impedances, field results where available
function anParams() {
  // class targets: board configuration (kisi.json) first, edits in the viewer on top
  const targets = { ...((S.config && S.config.targets) || {}) };
  for (const [k, v] of Object.entries(S.overrides)) targets[k] = v;
  const field = {};
  for (const [id, r] of S.field) if (r.ok) field[id] = r;
  return { step: S.step, coplanar_factor: 3.0, solder_mask: S.mask, targets, field, stackup: stackParam() };
}
function runAnalysis(tParse) {
  const t0 = performance.now();
  const params = anParams(), targets = params.targets;
  S.an = S.full ? S.engine.analyze(params) : S.engine.skeleton(params);
  // device-to-device lanes (topology): board and targets only, cheap
  const bkey = JSON.stringify(targets);
  if (!S.buses || S.busesKey !== bkey || S.busesBoard !== S.name + S.gen) {
    try { S.buses = S.engine.buses(params); } catch (e) { S.buses = []; console.warn(e); }
    S.busesKey = bkey; S.busesBoard = S.name + S.gen;
  }
  S.chCache = null; S.rfCache = null;
  S.anGen++;
  setTimeout(() => { renderCheck(); if (S.tab === "ch") { renderIfcCard(); renderProfCard(); } }, 0);
  const tAn = performance.now() - t0;
  if (tParse != null) S.parseMs = Math.round(tParse);
  $("timing").textContent = S.parseMs != null ? `read ${S.parseMs} ms` : "";
  xtUpdate();
  syncModeButtons();
  preparePieces();
  buildPaths();
  renderSide();
  renderLegend();
  if (S.selNet >= 0) selectNet(S.selNet, false); else $("detail").hidden = true;
  draw();
  renderFieldStatus();
}

function estimateImpedances() {
  if (S.full) return;
  showBusy("Estimating impedances and couplings…");
  requestAnimationFrame(() => setTimeout(() => {
    const t0 = performance.now();
    try { S.full = true; runAnalysis(); } catch (e) { busyError(e.message); return; }
    S.anMs = Math.round(performance.now() - t0);
    setMode("dev"); // deviation from target is now available
    renderFieldStatus();
    hideBusy();
    if (S.tab === "ch" && S.chMode === "o") renderChOverview();
  }, 0));
}

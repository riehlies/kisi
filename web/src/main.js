"use strict";
// ---- files, samples, theme and start-up
// files
async function handleFiles(files) {
  files = [...files]; S.sample = null;
  // the KiCad card belongs to the files the plugin handed over (loadKicadBoard), not to files dropped later
  if (S.kicad) { S.kicad.on = !!S.kicad.loading; S.kicad.loading = false; }
  const pcb = files.find((f) => f.name.endsWith(".kicad_pcb"));
  const pro = files.find((f) => f.name.endsWith(".kicad_pro"));
  // <board>.kisi.json: board configuration with sources; other .json files are not read
  const kisiCfg = files.find((f) => /\.kisi\.json$/i.test(f.name));
  const other = files.find((f) => f.name.endsWith(".json") && f !== kisiCfg);
  // BOMs (CSV) that the configuration's variants name
  const csv = {};
  for (const f of files.filter((f) => /\.csv$/i.test(f.name))) csv[f.name] = await f.text();
  // Touchstone files (.s2p, .s4p) for the channel behind connectors: kept in memory
  const tsDropped = files.filter((f) => /\.s\d+p$/i.test(f.name));
  for (const f of tsDropped) loadTouchstone(f.name, await f.text());
  if (tsDropped.length && !pcb && !kisiCfg && !Object.keys(csv).length) {
    if (S.tab === "ch" && S.scene) { S.chCache = null; markCheckStale(); renderChOverview(); }
    return;
  }
  let kc = kisiCfg ? { name: kisiCfg.name, text: await kisiCfg.text(), csv } : null;
  if (!kc && Object.keys(csv).length && S.lastFiles && S.lastFiles.cfg) kc = { ...S.lastFiles.cfg, csv: { ...(S.lastFiles.cfg.csv || {}), ...csv } };
  if (!pcb && !kisiCfg && kc) {
    const l = S.lastFiles;
    await loadFiles(l.pcbText, l.proText, l.name, l.piDefaults, kc);
    return;
  }
  if (pcb) {
    for (const k of ["ts", "vna"]) $("sample-" + k).setAttribute("aria-pressed", "false");
    await loadFiles(await pcb.text(), pro ? await pro.text() : null, pcb.name.replace(/\.kicad_pcb$/, ""), null, kc);
  } else if (kc && S.lastFiles) {
    // configuration alone: load the current board again with it
    const l = S.lastFiles;
    await loadFiles(l.pcbText, l.proText, l.name, l.piDefaults, kc);
    return;
  }
  if (pcb) return;
  if (other) { $("warnings").innerHTML = `<span class="err">${esc(other.name)}: only a board configuration named &lt;board&gt;.kisi.json is read.</span>`; return; }
  if (!pcb && pro && S.scene) {
    try { S.engine.loadPro(await pro.text()); S.scene = S.engine.scene(); runAnalysis(); } catch (e) { $("warnings").innerHTML = `<span class="err">Project file: ${esc(e.message)}</span>`; }
    return;
  }
  if (!pcb) { $("warnings").innerHTML = `<span class="err">Please choose a .kicad_pcb file (optionally also the .kicad_pro for net classes and the board's .kisi.json).</span>`; return; }
  for (const k of ["ts", "vna"]) $("sample-" + k).setAttribute("aria-pressed", "false");
  await loadFiles(await pcb.text(), pro ? await pro.text() : null, pcb.name.replace(/\.kicad_pcb$/, ""));
}
// register a Touchstone file in the engine (memory only); the result or the error is kept
function loadTouchstone(name, text) {
  let r;
  try { r = { text, info: S.engine.touchstone(name, text) }; } catch (e) { r = { text, error: e.message }; }
  S.tsFiles[name] = r;
  return r;
}
$("file").onchange = (e) => { $("drop").hidden = true; $("drop").classList.remove("start"); S.fileHandles = null; $("reload").hidden = true; handleFiles(e.target.files); e.target.value = ""; };
// File System Access (Chromium): keep the file handles, so “Reload” reads the same
// files again after saving in KiCad; elsewhere the file input is used
async function rememberHandles(hs) {
  hs = (hs || []).filter((h) => h && h.kind === "file");
  if (!hs.some((h) => /\.kicad_pcb$/i.test(h.name))) return false;
  S.fileHandles = hs; $("reload").hidden = false;
  return true;
}
async function readHandles() { return Promise.all(S.fileHandles.map((h) => h.getFile())); }
if (window.showOpenFilePicker) {
  $("load-files").addEventListener("click", async (e) => {
    e.preventDefault();
    try {
      const hs = await window.showOpenFilePicker({ multiple: true });
      if (!(await rememberHandles(hs))) { S.fileHandles = null; $("reload").hidden = true; }
      await handleFiles(await Promise.all(hs.map((h) => h.getFile())));
    } catch (err) {
      if (err && err.name === "AbortError") return;
      $("file").click(); // picker not allowed here: plain file input
    }
  });
}
$("reload").onclick = async () => {
  if (!S.fileHandles) return;
  try { await handleFiles(await readHandles()); }
  catch (e) { $("warnings").innerHTML = `<span class="err">Reload failed: ${esc(e.message)} – load the files again.</span>`; S.fileHandles = null; $("reload").hidden = true; }
};
let dragDepth = 0;
window.addEventListener("dragenter", (e) => { e.preventDefault(); dragDepth++; $("drop").hidden = false; });
window.addEventListener("dragleave", () => { if (--dragDepth <= 0) { dragDepth = 0; if (!$("drop").classList.contains("start")) $("drop").hidden = true; } });
window.addEventListener("dragover", (e) => e.preventDefault());
window.addEventListener("drop", (e) => {
  e.preventDefault(); dragDepth = 0; $("drop").hidden = true; $("drop").classList.remove("start");
  // handles must be requested inside the event, before any await
  const hp = [...(e.dataTransfer.items || [])].map((it) => it.kind === "file" && it.getAsFileSystemHandle ? it.getAsFileSystemHandle().catch(() => null) : null);
  const files = [...e.dataTransfer.files];
  (async () => {
    if (hp.some(Boolean) && files.some((f) => /\.kicad_pcb$/i.test(f.name))) { if (!(await rememberHandles(await Promise.all(hp)))) { S.fileHandles = null; $("reload").hidden = true; } }
    else if (files.some((f) => /\.kicad_pcb$/i.test(f.name))) { S.fileHandles = null; $("reload").hidden = true; }
    else if (S.fileHandles && hp.some(Boolean)) {
      // a configuration file dropped later: reload reads it too
      const extra = (await Promise.all(hp)).filter((h) => h && h.kind === "file" && /\.kisi\.json$|\.csv$|\.s\d+p$/i.test(h.name));
      S.fileHandles = S.fileHandles.filter((h) => !extra.some((x) => x.name === h.name)).concat(extra);
    }
    await handleFiles(files);
  })();
});

async function loadSample(key) {
  S.fileHandles = null; $("reload").hidden = true; S.sample = key;
  if (S.kicad) S.kicad.on = false;
  const s = SAMPLES[key];
  for (const k of ["ts", "vna"]) $("sample-" + k).setAttribute("aria-pressed", String(k === key));
  showBusy("Unpacking example board…");
  const [pcb, pro] = await Promise.all([gunzipB64(s.pcb), gunzipB64(s.pro)]);
  // name = file name, so the example's kisi.json matches the board like a dropped one
  await loadFiles(pcb, pro, s.file || s.name, null, s.config || null);
}
// ---- a board from the KiCad plugin (KICAD_BOARD, kicad-plugin/): its files take the way of dropped files
async function loadKicadBoard(k) {
  showBusy("Unpacking the board from KiCad…");
  const files = await Promise.all(k.files.map(async (f) => new File([await gunzipB64(f.gz)], f.name)));
  S.kicad = { ...k, files: undefined, on: false, loading: true, dirHandle: null, dirStored: false, dirNote: "" };
  // the board's folder chosen in an earlier session of this browser (Chromium; kdirChoose)
  S.kicad.dirHandle = await kdirGet(k.pcb_path);
  S.kicad.dirStored = !!S.kicad.dirHandle;
  await handleFiles(files);
  kicadCard();
}
const kicadActive = () => !!(S.kicad && S.kicad.on && S.scene && S.name === S.kicad.board);
// where the board's kisi.json belongs (the plugin's path, with the platform's separators)
// the plugin's kisi.mjs and Node.js (kisi_kicad.find_cli, find_node): the agent card's command
const kicadCli = () => (kicadActive() && S.kicad.cli && S.kicad.cli.path ? S.kicad.cli : null);
const kicadCfgPath = () => S.kicad.config_path || S.kicad.dir + (S.kicad.dir.includes("\\") ? "\\" : "/") + S.kicad.board + ".kisi.json";
function kicadCard() {
  const on = kicadActive(), k = S.kicad;
  $("kicad-card").classList.toggle("off", !on);
  if (!on) return;
  $("kicad-saved").textContent = k.saved ? `saved ${k.saved}` : "";
  const name = esc(k.board + ".kisi.json"), prep = `<a href="#" data-kicad-prep>Prepare PCB</a>`, imp = k.import || {};
  const cfg = {
    loaded: `${name} loaded from next to the board.`,
    imported: `${name} taken over from the downloads folder and loaded.`,
    created: `There was no ${name}: an empty one was created next to the board. Fill it on the ${prep} page.`,
    saved: `${name} saved from the ${prep} page and loaded.`,
    downloaded: `${name} downloaded and loaded here. Click kisi in KiCad again: the plugin takes it over from the downloads folder (or move it next to the board yourself).`,
  }[k.config] || `No ${name} next to the board.`;
  // a kisi.json the viewer downloaded earlier: taken over on this click, or why not (kisi_kicad.import_download)
  const impHtml = imp.state === "taken"
    ? `<p class="note">Taken over from the downloads folder: <code class="kicad-dir">${esc(imp.from)}</code> (${esc(imp.time)}${imp.checked ? ", " + esc(imp.checked) : ""}).${imp.backup ? ` The previous file is kept as <code class="kicad-dir">${esc(imp.backup)}</code>.` : ""}${(imp.reasons || []).map((r) => " " + esc(r)).join("")}</p>`
    : ["ambiguous", "invalid"].includes(imp.state)
      ? `<p class="warn">Not taken over from the downloads folder${imp.from ? ` (<code class="kicad-dir">${esc(imp.from)}</code>)` : ""}: ${esc((imp.reasons || []).join("; "))}. The kisi.json next to the board is unchanged.</p>` : "";
  // where this page lies: analysis/<board>.kisi.html next to the board, or the temporary folder
  const pg = k.page || {};
  const pageHtml = pg.where === "temp"
    ? `<p class="warn">This page is in the temporary folder (<code class="kicad-dir">${esc(pg.path)}</code>): ${esc(pg.reason || "analysis/ next to the board could not be written")}.</p>`
    : pg.path ? `<p class="note">This page: <code class="kicad-dir">${esc(pg.path)}</code> – each click writes it anew.</p>` : "";
  const unsaved = k.modified === "modified"
    ? `<p class="warn">KiCad reports unsaved changes: this is the last saved version of the board. Save in KiCad and click kisi again.</p>`
    : k.modified === "unmodified" ? "" : `<p class="note">Read from the saved file: changes not yet saved in KiCad are not included – save there and click kisi again.</p>`;
  $("kicad-body").innerHTML = `<p class="note"><b>${esc(k.board)}.kicad_pcb</b> in <code class="kicad-dir">${esc(k.dir)}</code></p>${unsaved}${impHtml}<p class="note">${cfg}</p>${pageHtml}`
    + (k.notes || []).map((n) => `<p class="note warn">${esc(n)}</p>`).join("");
}
document.addEventListener("click", (e) => { if (e.target.closest && e.target.closest("[data-kicad-prep]")) { e.preventDefault(); prepOpen(true); } });

// ---- the board's folder, chosen once (Chromium: showDirectoryPicker), so that "Save kisi.json" writes there without
// a dialog. Its handle is kept in IndexedDB per board path (handles can be stored there; the browser asks for write
// permission again once per session). Elsewhere, or when the handle cannot be stored, it lasts for this page.
const KDIR_DB = "kisi-kicad-folders";
function kdirDb() {
  return new Promise((res, rej) => {
    const r = indexedDB.open(KDIR_DB, 1);
    r.onupgradeneeded = () => r.result.createObjectStore("dirs");
    r.onsuccess = () => res(r.result); r.onerror = () => rej(r.error);
  });
}
async function kdirGet(key) {
  try {
    const db = await kdirDb();
    return await new Promise((res) => { const q = db.transaction("dirs").objectStore("dirs").get(key); q.onsuccess = () => res(q.result || null); q.onerror = () => res(null); });
  } catch (e) { return null; }
}
async function kdirSet(key, h) {
  try {
    const db = await kdirDb();
    await new Promise((res, rej) => { const t = db.transaction("dirs", "readwrite"); t.objectStore("dirs").put(h, key); t.oncomplete = res; t.onerror = () => rej(t.error); t.onabort = () => rej(t.error); });
    return true;
  } catch (e) { return false; }
}
// the chosen folder is the board's: its name is the board folder's and it holds <board>.kicad_pcb
async function kdirCheck(h) {
  const base = String(S.kicad.dir).split(/[\\/]/).filter(Boolean).pop() || "";
  if (String(h.name).toLowerCase() !== base.toLowerCase()) return `the chosen folder “${h.name}” is not the board's folder “${base}”`;
  try { await h.getFileHandle(S.kicad.board + ".kicad_pcb"); } catch (e) { return `the chosen folder “${h.name}” has no ${S.kicad.board}.kicad_pcb`; }
  return "";
}
async function kdirChoose() {
  let h;
  try { h = await window.showDirectoryPicker({ id: "kisi-dir-" + textHash(S.kicad.dir), mode: "readwrite", startIn: S.kicad.dirHandle || "documents" }); }
  catch (e) { if (!e || e.name !== "AbortError") { S.kicad.dirNote = `the folder could not be chosen (${e && e.message})`; prepRender(false); } return; }
  const bad = await kdirCheck(h);
  if (bad) { S.kicad.dirNote = bad + " – choose the folder shown above."; prepRender(false); return; }
  S.kicad.dirHandle = h; S.kicad.dirNote = "";
  S.kicad.dirStored = await kdirSet(S.kicad.pcb_path, h);
  prepRender(false);
}
// write <board>.kisi.json into the chosen folder: true, or false with the reason in dirNote (then the save dialog)
async function kdirWrite(name, text) {
  const h = S.kicad.dirHandle;
  try {
    let p = h.queryPermission ? await h.queryPermission({ mode: "readwrite" }) : "granted";
    if (p !== "granted" && h.requestPermission) p = await h.requestPermission({ mode: "readwrite" });
    if (p !== "granted") { S.kicad.dirNote = "no permission to write into the board's folder – saved through the dialog instead"; return false; }
    const bad = await kdirCheck(h);
    if (bad) { S.kicad.dirNote = bad + " – not written there; choose the board's folder again"; return false; }
    const fh = await h.getFileHandle(name, { create: true }), w = await fh.createWritable();
    await w.write(text); await w.close();
    S.kicad.dirNote = "";
    return true;
  } catch (e) { S.kicad.dirNote = `writing into the board's folder failed (${e.message}) – saved through the dialog instead`; return false; }
}
// the agent guide shipped with the plugin (KISI-AGENT-GUIDE.md): path and text for the agent card
const kicadGuide = () => (kicadActive() && S.kicad.guide && S.kicad.guide.text ? S.kicad.guide : null);
$("sample-ts").onclick = () => loadSample("ts");
$("sample-vna").onclick = () => loadSample("vna");

let busySince = 0;
function showBusy(t) { $("busy").textContent = t; $("busy").hidden = false; busySince = performance.now(); }
function hideBusy() { $("busy").hidden = true; }
// show uncaught errors on the overlay instead of leaving a stale "busy" message
function busyError(msg) {
  console.error(msg);
  const b = $("busy");
  if (b.hidden) { $("warnings").innerHTML = `<span class="err">${esc(msg)}</span>`; return; }
  b.innerHTML = `<div class="busy-err"><b>Error</b><br>${esc(msg)}<br><small>Details in the browser console (F12).</small></div>`;
}
window.addEventListener("error", (e) => busyError(e.message || String(e.error)));
window.addEventListener("unhandledrejection", (e) => busyError(e.reason && e.reason.message ? e.reason.message : String(e.reason)));
setInterval(() => {
  const b = $("busy");
  if (!b.hidden && !b.querySelector(".busy-err, small") && performance.now() - busySince > 30000)
    b.innerHTML = `<div>${esc(b.textContent)}<br><small>still working after 30 s – if nothing changes, please check the browser console (F12)</small></div>`;
}, 5000);

// without a board, controls that work on it do nothing (they are hidden on the start screen as well)
for (const ev of ["click", "change", "input"]) {
  document.addEventListener(ev, (e) => {
    if (!S.scene && e.target.closest && e.target.closest(".side, .bar > .seg:not(.samples), .overlay-tr, .detail")) { e.stopImmediatePropagation(); e.preventDefault(); }
  }, true);
}

// theme changes
const reTheme = () => { readColors(); if (S.scene) { buildPaths(); if (S.selNet >= 0) selectNet(S.selNet, false); renderLegend(); renderNets(); } draw(); };
matchMedia("(prefers-color-scheme: dark)").addEventListener("change", reTheme);
new MutationObserver(reTheme).observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme"] });
new ResizeObserver(resize).observe(cv);

(async () => {
  readColors();
  try {
    S.engine = new Engine();
    await S.engine.init();
    $("up-rev").textContent = UPSTREAM_REV.slice(0, 7);
    await Cache.open();
    await Cache.loadAll();
    // example boards are only in builds that embed them (they are not in the repository)
    for (const k of ["ts", "vna"]) {
      $("sample-" + k).hidden = !SAMPLES[k];
      if (SAMPLES[k] && SAMPLES[k].notice) $("sample-" + k).title = SAMPLES[k].notice.split("\n")[0]; // credit and licence
    }
    $("sample-ts").parentElement.hidden = !SAMPLES.ts && !SAMPLES.vna;
    if (KICAD_BOARD) await loadKicadBoard(KICAD_BOARD);
    else if (SAMPLES.ts) await loadSample("ts");
    else { hideBusy(); $("drop").hidden = false; $("drop").classList.add("start"); }
  } catch (e) {
    showBusy("Engine failed to start: " + e.message);
  }
})();

"use strict";
// ---- files, samples, theme and start-up
// files
async function handleFiles(files) {
  files = [...files];
  const pcb = files.find((f) => f.name.endsWith(".kicad_pcb"));
  const pro = files.find((f) => f.name.endsWith(".kicad_pro"));
  // <board>.kisi.json: board configuration with sources; other .json files are not read
  const kisiCfg = files.find((f) => /\.kisi\.json$/i.test(f.name));
  const other = files.find((f) => f.name.endsWith(".json") && f !== kisiCfg);
  const kc = kisiCfg ? { name: kisiCfg.name, text: await kisiCfg.text() } : null;
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
      const extra = (await Promise.all(hp)).filter((h) => h && h.kind === "file" && /\.kisi\.json$/i.test(h.name));
      S.fileHandles = S.fileHandles.filter((h) => !extra.some((x) => x.name === h.name)).concat(extra);
    }
    await handleFiles(files);
  })();
});

async function loadSample(key) {
  S.fileHandles = null; $("reload").hidden = true;
  const s = SAMPLES[key];
  for (const k of ["ts", "vna"]) $("sample-" + k).setAttribute("aria-pressed", String(k === key));
  showBusy("Unpacking example board…");
  const [pcb, pro] = await Promise.all([gunzipB64(s.pcb), gunzipB64(s.pro)]);
  // name = file name, so the example's kisi.json matches the board like a dropped one
  await loadFiles(pcb, pro, s.file || s.name, null, s.config || null);
}
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
    if (SAMPLES.ts) await loadSample("ts");
    else { hideBusy(); $("drop").hidden = false; $("drop").classList.add("start"); }
  } catch (e) {
    showBusy("Engine failed to start: " + e.message);
  }
})();

"use strict";
// ---- files, samples, theme and start-up
// files
async function handleFiles(files) {
  files = [...files];
  const pcb = files.find((f) => f.name.endsWith(".kicad_pcb"));
  const pro = files.find((f) => f.name.endsWith(".kicad_pro"));
  const cfg = files.find((f) => f.name.endsWith(".json"));
  if (pcb) {
    for (const k of ["ts", "vna"]) $("sample-" + k).setAttribute("aria-pressed", "false");
    await loadFiles(await pcb.text(), pro ? await pro.text() : null, pcb.name.replace(/\.kicad_pcb$/, ""));
  }
  if (cfg && S.scene) { importKipida(await cfg.text()); return; }
  if (pcb) return;
  if (!pcb && pro && S.scene) {
    try { S.engine.loadPro(await pro.text()); S.scene = S.engine.scene(); runAnalysis(); } catch (e) { $("warnings").innerHTML = `<span class="err">Project file: ${esc(e.message)}</span>`; }
    return;
  }
  if (!pcb) { $("warnings").innerHTML = `<span class="err">Please choose a .kicad_pcb file (optionally also the .kicad_pro for net classes).</span>`; return; }
  for (const k of ["ts", "vna"]) $("sample-" + k).setAttribute("aria-pressed", "false");
  await loadFiles(await pcb.text(), pro ? await pro.text() : null, pcb.name.replace(/\.kicad_pcb$/, ""));
}
$("file").onchange = (e) => handleFiles(e.target.files);
let dragDepth = 0;
window.addEventListener("dragenter", (e) => { e.preventDefault(); dragDepth++; $("drop").hidden = false; });
window.addEventListener("dragleave", () => { if (--dragDepth <= 0) { dragDepth = 0; $("drop").hidden = true; } });
window.addEventListener("dragover", (e) => e.preventDefault());
window.addEventListener("drop", (e) => { e.preventDefault(); dragDepth = 0; $("drop").hidden = true; handleFiles(e.dataTransfer.files); });

async function loadSample(key) {
  const s = SAMPLES[key];
  for (const k of ["ts", "vna"]) $("sample-" + k).setAttribute("aria-pressed", String(k === key));
  showBusy("Unpacking example board…");
  const [pcb, pro] = await Promise.all([gunzipB64(s.pcb), gunzipB64(s.pro)]);
  await loadFiles(pcb, pro, s.name, s.pi || null);
}
// board handed over by the KiCad plugin (tools/kicad-plugin): {name, pcb, pro (gzip+base64), net?}
async function loadPreload(pl) {
  for (const k of ["ts", "vna"]) $("sample-" + k).setAttribute("aria-pressed", "false");
  showBusy("Unpacking board from KiCad…");
  // gzip+base64 from the KiCad plugin, plain text from kisi-mcp
  const pcb = pl.pcb_text != null ? pl.pcb_text : await gunzipB64(pl.pcb);
  const pro = pl.pro_text != null ? pl.pro_text : pl.pro ? await gunzipB64(pl.pro) : null;
  await loadFiles(pcb, pro, pl.name);
  if (pl.net) {
    const id = S.scene.nets.indexOf(pl.net);
    if (id > 0) { if (pl.view === "ch") setTab("ch"); selectNet(id); }
  }
}
$("sample-ts").onclick = () => loadSample("ts");
$("sample-vna").onclick = () => loadSample("vna");

function showBusy(t) { $("busy").textContent = t; $("busy").hidden = false; }
function hideBusy() { $("busy").hidden = true; }

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
    if (typeof PRELOAD !== "undefined" && PRELOAD) await loadPreload(PRELOAD);
    else await loadSample("ts");
  } catch (e) {
    showBusy("Engine failed to start: " + e.message);
  }
})();

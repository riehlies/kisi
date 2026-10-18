"use strict";
// ============================================================== loading
async function loadFiles(pcbText, proText, name, piDefaults = null) {
  showBusy("Lese Board und berechne Querschnitte…");
  await new Promise((r) => requestAnimationFrame(() => setTimeout(r, 0)));
  try {
    const t0 = performance.now();
    S.engine.loadPro(proText || '{"net_settings":{}}');
    S.engine.loadPcb(pcbText);
    S.scene = S.engine.scene();
    S.padSet = new Set(S.scene.pad_tracks || []);
    S.zones = S.engine.zones();
    S.name = name;
    S.overrides = {};
    S.selNet = -1; S.hover = -1; S.check = null; S.chStatus = null; S.chCache = null;
    S.fieldPending = new Set(); S.gen++;
    if (S.pool) S.pool.reset();
    const n = S.scene.stackup.copper.length;
    S.layerOn = new Array(n).fill(true);
    // default plane layer: first inner layer with most zone area, else none
    S.zoneLayer = guessPlaneLayer();
    const tParse = performance.now() - t0;
    runAnalysis(tParse);
    buildStatic();
    initPi(piDefaults);
    fitView();
    hideBusy();
  } catch (e) {
    hideBusy();
    $("warnings").innerHTML = `<span class="err">Datei konnte nicht gelesen werden: ${esc(e.message)}</span>`;
    console.error(e);
  }
}

function guessPlaneLayer() {
  const n = S.scene.stackup.copper.length, idx = S.zones.idx, pts = S.zones.pts;
  const area = new Array(n).fill(0);
  for (let i = 0; i < idx.length; i += 4) {
    const l = idx[i], st = idx[i + 2], c = idx[i + 3];
    let a = 0;
    for (let k = 0; k < c; k++) {
      const j = (st + k) * 2, j2 = (st + (k + 1) % c) * 2;
      a += pts[j] * pts[j2 + 1] - pts[j2] * pts[j + 1];
    }
    area[l] += Math.abs(a) / 2;
  }
  let best = -1, bestA = 0;
  for (let l = 1; l < n - 1; l++) if (area[l] > bestA) { bestA = area[l]; best = l; }
  return best;
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

function runAnalysis(tParse) {
  const t0 = performance.now();
  const targets = {};
  for (const [k, v] of Object.entries(S.overrides)) targets[k] = v;
  const field = {};
  for (const [id, r] of S.field) if (r.ok) field[id] = r;
  S.an = S.engine.analyze({ step: S.step, coplanar_factor: 3.0, solder_mask: S.mask, targets, field, stackup: stackParam() });
  S.chCache = null; S.rfCache = null;
  S.anGen++;
  setTimeout(() => { renderCheck(); if (S.tab === "ch") { renderIfcCard(); renderProfCard(); } }, 0);
  const tAn = performance.now() - t0;
  $("timing").textContent = (tParse != null ? `Lesen ${Math.round(tParse)} ms · ` : "") + `Analyse ${Math.round(tAn)} ms`;
  preparePieces();
  buildPaths();
  renderSide();
  renderLegend();
  if (S.selNet >= 0) selectNet(S.selNet, false); else $("detail").hidden = true;
  draw();
  renderFieldStatus();
}

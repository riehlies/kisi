"use strict";
// ---- top-level views: impedance / channel / power, power display modes
function setTab(t) {
  S.tab = t;
  for (const k of ["si", "ch", "pi"]) $("view-" + k).setAttribute("aria-pressed", String(k === t));
  if (!S.scene) return; // no board yet: the chosen view opens when one is loaded (load.js)
  document.querySelectorAll("[data-view]").forEach((el) => { el.hidden = !el.dataset.view.split(" ").includes(t); });
  $("detail").hidden = true; $("ch-detail").hidden = true;
  $("pi-detail").hidden = true;
  $("pi-balance").hidden = true; $("ch-overview").hidden = true; $("legend").hidden = false; $("layers").hidden = false;
  $("tip").hidden = true;
  if (t === "pi") { fitRail(); renderPi(); return; }
  if (t === "ch") { renderIfcCard(); renderProfCard(); renderCheck(); setChMode(S.chMode || "o"); if (S.chMode === "o") return; }
  buildPaths(); renderLayerChips(); renderLegend();
  if (S.selNet >= 0) selectNet(S.selNet, false); else fitView();
}
$("view-si").onclick = () => setTab("si");
$("view-ch").onclick = () => setTab("ch");
$("view-pi").onclick = () => setTab("pi");
function setPiMode(m) {
  const toLoss = m === "l" && S.piMode !== "l";
  S.piMode = m; S.piImg = null;
  if (toLoss) fitView(); // the loss map covers the whole board
  for (const k of ["v", "j", "l", "b"]) $("pim-" + k).setAttribute("aria-pressed", String(k === m));
  renderPi();
}
$("pim-b").onclick = () => setPiMode("b");
$("pim-v").onclick = () => setPiMode("v");
$("pim-j").onclick = () => setPiMode("j");
$("pim-l").onclick = () => setPiMode("l");

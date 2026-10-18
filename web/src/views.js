"use strict";
// ---- top-level views: Impedanz / Kanal / Versorgung, supply display modes
function setTab(t) {
  S.tab = t;
  for (const k of ["si", "ch", "pi"]) $("view-" + k).setAttribute("aria-pressed", String(k === t));
  document.querySelectorAll("[data-view]").forEach((el) => { el.hidden = !el.dataset.view.split(" ").includes(t); });
  $("detail").hidden = true; $("ch-detail").hidden = true;
  $("pi-detail").hidden = true;
  $("pi-balance").hidden = true; $("legend").hidden = false; $("layers").hidden = false;
  $("tip").hidden = true;
  if (t === "pi") { fitRail(); renderPi(); return; }
  if (t === "ch") { renderIfcCard(); renderProfCard(); renderCheck(); }
  buildPaths(); renderLayerChips(); renderLegend();
  if (S.selNet >= 0) selectNet(S.selNet, false); else fitView();
}
$("view-si").onclick = () => setTab("si");
$("view-ch").onclick = () => setTab("ch");
$("view-pi").onclick = () => setTab("pi");
function setPiMode(m) {
  S.piMode = m; S.piImg = null;
  for (const k of ["v", "j", "b"]) $("pim-" + k).setAttribute("aria-pressed", String(k === m));
  renderPi();
}
$("pim-b").onclick = () => setPiMode("b");
$("pim-v").onclick = () => setPiMode("v");
$("pim-j").onclick = () => setPiMode("j");

"use strict";
// ---- top-level views: impedance / channel / power, power display modes
function setTab(t) {
  S.tab = t;
  for (const k of ["si", "ch", "pi"]) $("view-" + k).setAttribute("aria-pressed", String(k === t));
  if (!S.scene) return; // no board yet: the chosen view opens when one is loaded (load.js)
  document.querySelectorAll("[data-view]").forEach((el) => { el.hidden = !el.dataset.view.split(" ").includes(t); });
  placeSharedSteps(t);
  $("detail").hidden = true; $("ch-detail").hidden = true;
  $("pi-detail").hidden = true;
  $("pi-balance").hidden = true; $("ch-overview").hidden = true; $("legend").hidden = false; $("layers").hidden = false;
  $("tip").hidden = true;
  if (t === "pi") { fitRail(); renderPi(); return; }
  if (t === "ch") { renderIfcCard(); renderProfCard(); renderCheck(); setChMode(S.chMode || "o"); if (S.chMode === "o") return; }
  buildPaths(); renderLayerChips(); renderLegend();
  if (S.selNet >= 0) selectNet(S.selNet, false); else fitView();
}
// the field solver step and the accuracy notes exist once: in the "Signals" card of the
// impedance view, moved into the "Channel check" card in the channel view (same controls,
// same state, the same look)
function placeSharedSteps(t) {
  const st3 = $("st3"), acc = $("acc");
  if (t === "ch") { $("cs3").before(st3); $("ck-out").after(acc); }
  else { $("st2").after(st3); $("cfg-status").after(acc); }
}
$("view-si").onclick = () => setTab("si");
$("view-ch").onclick = () => setTab("ch");
$("view-pi").onclick = () => setTab("pi");
function setPiMode(m) {
  const toMap = (m === "l" || m === "t") && S.piMode !== m;
  S.piMode = m; S.piImg = null;
  if (toMap) fitView(); // the loss and temperature maps cover the whole board
  for (const k of ["v", "j", "l", "t", "b"]) $("pim-" + k).setAttribute("aria-pressed", String(k === m));
  renderPi();
}
$("pim-b").onclick = () => setPiMode("b");
$("pim-v").onclick = () => setPiMode("v");
$("pim-j").onclick = () => setPiMode("j");
$("pim-l").onclick = () => setPiMode("l");
$("pim-t").onclick = () => setPiMode("t");

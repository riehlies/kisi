"use strict";
// ---- top-level views: Impedanz / Kanal / Versorgung, supply display modes
function setTab(t) {
  S.tab = t;
  $("view-si").setAttribute("aria-pressed", String(t === "si"));
  $("view-pi").setAttribute("aria-pressed", String(t === "pi"));
  document.querySelectorAll("[data-view]").forEach((el) => { el.hidden = el.dataset.view !== t; });
  $("detail").hidden = t !== "si" || S.selNet < 0;
  $("pi-detail").hidden = true;
  $("pi-balance").hidden = true; $("legend").hidden = false; $("layers").hidden = false;
  $("tip").hidden = true;
  if (t === "pi") { fitRail(); renderPi(); } else { fitView(); renderLayerChips(); renderLegend(); draw(); }
}
$("view-si").onclick = () => setTab("si");
$("view-pi").onclick = () => setTab("pi");
function setPiMode(m) {
  S.piMode = m; S.piImg = null;
  for (const k of ["v", "j", "b"]) $("pim-" + k).setAttribute("aria-pressed", String(k === m));
  renderPi();
}
$("pim-b").onclick = () => setPiMode("b");
$("pim-v").onclick = () => setPiMode("v");
$("pim-j").onclick = () => setPiMode("j");

"use strict";
// ============================================================== interaction
// colour modes: line type from the start, deviation / Z0 / crosstalk after step 2
function syncModeButtons() {
  if (S.mode === "xt" && !S.full) S.mode = "type";
  for (const k of ["dev", "abs", "type", "xt"]) $("mode-" + k).setAttribute("aria-pressed", String(k === S.mode));
  for (const k of ["dev", "abs"]) { $("mode-" + k).disabled = !S.full; $("mode-" + k).title = S.full ? "" : "after step 2 “Impedance estimate”"; }
  $("mode-xt").disabled = !S.full; $("mode-xt").title = S.full ? "" : "after step 2 “Impedance estimate”";
}
function setMode(m) {
  S.mode = m;
  syncModeButtons();
  buildPaths(); if (S.selNet >= 0) selectNet(S.selNet, false);
  renderLegend(); draw();
}
$("mode-dev").onclick = () => setMode("dev");
$("mode-abs").onclick = () => setMode("abs");
$("mode-type").onclick = () => setMode("type");
$("fit").onclick = () => fitView();
$("search").oninput = () => renderNets();
$("opt-mask").onchange = (e) => { S.mask = e.target.checked; runAnalysis(); };
$("opt-step").onchange = (e) => { S.step = parseFloat(e.target.value); runAnalysis(); };
$("ck-start").onclick = () => runCheck();
for (const id of ["rf-fmax", "rf-rise", "rf-rough", "rf-vias", "rf-bd"]) $(id).onchange = () => {
  markCheckStale();
  if (S.full) { xtUpdate(); renderFieldStatus(); draw(); }
  if (S.selNet >= 0) selectNet(S.selNet, false);
};
$("opt-zone").onchange = (e) => { S.zoneLayer = parseInt(e.target.value, 10); draw(); };

const ptrs = new Map(); let pinch = null, dragMoved = false;
cv.addEventListener("pointerdown", (e) => {
  cv.setPointerCapture(e.pointerId);
  ptrs.set(e.pointerId, { x: e.offsetX, y: e.offsetY });
  dragMoved = false;
  if (ptrs.size === 2) { const [a, b] = [...ptrs.values()]; pinch = { d: Math.hypot(a.x - b.x, a.y - b.y), s: S.view.s, mx: (a.x + b.x) / 2, my: (a.y + b.y) / 2 }; }
});
cv.addEventListener("pointermove", (e) => {
  const p = ptrs.get(e.pointerId);
  if (p) {
    if (ptrs.size === 1) {
      const dx = e.offsetX - p.x, dy = e.offsetY - p.y;
      if (Math.abs(dx) + Math.abs(dy) > 2) dragMoved = true;
      S.view.x += dx; S.view.y += dy; p.x = e.offsetX; p.y = e.offsetY; $("tip").hidden = true; draw(); return;
    }
    p.x = e.offsetX; p.y = e.offsetY;
    if (pinch && ptrs.size === 2) {
      const [a, b] = [...ptrs.values()]; const d = Math.hypot(a.x - b.x, a.y - b.y);
      zoomAt(pinch.mx, pinch.my, (pinch.s * d / pinch.d) / S.view.s); dragMoved = true;
    }
    return;
  }
  if (S.tab === "pi") { showPiTip(e.offsetX, e.offsetY); return; }
  const i = pick(e.offsetX, e.offsetY);
  if (i !== S.hover) { S.hover = i; draw(); }
  showTip(i, e.offsetX, e.offsetY);
});
const endPtr = (e) => {
  const wasDrag = dragMoved;
  ptrs.delete(e.pointerId); if (ptrs.size < 2) pinch = null;
  // click on a track: the net (impedance view) or its channel (channel view, board map)
  if (e.type === "pointerup" && !wasDrag && ptrs.size === 0 && (S.tab === "si" || (S.tab === "ch" && S.chMode === "m"))) {
    const i = pick(e.offsetX, e.offsetY);
    if (S.tab === "ch") S.selLane = null;
    if (i >= 0) selectNet(S.pieces.net[i], false); else if (S.selNet >= 0) selectNet(-1);
  }
};
cv.addEventListener("pointerup", endPtr);
cv.addEventListener("pointercancel", endPtr);
cv.addEventListener("pointerleave", () => { if (S.hover >= 0) { S.hover = -1; draw(); } $("tip").hidden = true; });
function zoomAt(mx, my, f) {
  const s = Math.max(0.5, Math.min(S.view.s * f, 2000)); f = s / S.view.s;
  S.view.x = mx - (mx - S.view.x) * f; S.view.y = my - (my - S.view.y) * f; S.view.s = s; draw();
}
cv.addEventListener("wheel", (e) => { e.preventDefault(); zoomAt(e.offsetX, e.offsetY, Math.exp(-e.deltaY * (e.deltaMode ? 0.05 : 0.0015))); }, { passive: false });

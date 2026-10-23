"use strict";
// ============================================================== view / drawing
const cv = $("pcb"), ctx = cv.getContext("2d");
function resize() {
  const r = cv.getBoundingClientRect(), d = window.devicePixelRatio || 1;
  cv.width = Math.max(1, Math.round(r.width * d)); cv.height = Math.max(1, Math.round(r.height * d));
  draw();
}
function fitView(bb) {
  if (!S.scene) return;
  const b = bb || S.scene.bbox, r = cv.getBoundingClientRect();
  const w = b[2] - b[0], h = b[3] - b[1];
  const s = Math.min((r.width - 40) / Math.max(w, 0.1), (r.height - 90) / Math.max(h, 0.1));
  S.view.s = Math.max(0.5, Math.min(s, 400));
  S.view.x = r.width / 2 - (b[0] + w / 2) * S.view.s;
  S.view.y = (r.height - 30) / 2 - (b[1] + h / 2) * S.view.s;
  draw();
}
let drawQueued = false;
function draw() { if (!drawQueued) { drawQueued = true; requestAnimationFrame(drawNow); } }
function drawNow() {
  drawQueued = false;
  if (S.tab === "pi") return drawPi();
  const d = window.devicePixelRatio || 1, C = S.colors;
  ctx.setTransform(1, 0, 0, 1, 0, 0);
  ctx.fillStyle = C.canvas; ctx.fillRect(0, 0, cv.width, cv.height);
  if (!S.scene || !S.paths) return;
  const v = S.view, n = S.scene.stackup.copper.length;
  ctx.setTransform(v.s * d, 0, 0, v.s * d, v.x * d, v.y * d);
  const px = 1 / v.s; // one css pixel in mm
  ctx.lineCap = "round"; ctx.lineJoin = "round";
  // reference plane fill
  if (S.zoneLayer >= 0) {
    ctx.fillStyle = C.zone; ctx.fill(S.static.zp[S.zoneLayer], "evenodd");
    ctx.strokeStyle = C.zoneEdge; ctx.lineWidth = px; ctx.stroke(S.static.zp[S.zoneLayer]);
  }
  ctx.strokeStyle = C.edge; ctx.lineWidth = 1.5 * px; ctx.stroke(S.static.outline);
  const dim = S.selNet >= 0;
  // tracks: bottom layer first
  for (let l = n - 1; l >= 0; l--) {
    if (!S.layerOn[l]) continue;
    ctx.fillStyle = C.pad; ctx.fill(S.static.pads[l]);
    ctx.globalAlpha = dim ? 0.18 : 1;
    for (const g of S.paths[l]) drawGroup(g, px);
    ctx.globalAlpha = 1;
  }
  ctx.fillStyle = C.via; ctx.fill(S.static.vias);
  // branches of a selected lane (the other path of a fork): dashed, under the lane itself
  if (dim && S.stubPaths) {
    ctx.setLineDash([3 * px, 3 * px]);
    for (let l = n - 1; l >= 0; l--) {
      if (!S.layerOn[l]) continue;
      for (const g of S.stubPaths[l]) drawGroup(g, px);
    }
    ctx.setLineDash([]);
  }
  if (dim && S.selPaths) {
    for (let l = n - 1; l >= 0; l--) {
      if (!S.layerOn[l]) continue;
      for (const g of S.selPaths[l]) drawGroup(g, px, 3);
      for (const g of S.selPaths[l]) drawGroup(g, px);
    }
  }
  if (S.hover >= 0) {
    const i = S.hover, pc = S.pieces, pad = padPieceShape(i, true);
    if (pad) {
      ctx.strokeStyle = C.halo; ctx.lineWidth = 4 * px; ctx.stroke(pad);
      ctx.fillStyle = pieceColor(i); ctx.fill(pad);
    } else {
      ctx.strokeStyle = C.halo; ctx.lineWidth = Math.max(pc.w[i], 1.2 * px) + 4 * px;
      ctx.beginPath(); ctx.moveTo(pc.ax[i], pc.ay[i]); ctx.lineTo(pc.bx[i], pc.by[i]); ctx.stroke();
      ctx.strokeStyle = pieceColor(i); ctx.lineWidth = Math.max(pc.w[i], 1.2 * px);
      ctx.stroke();
    }
  }
  drawXtMap(px);
  if (S.mark && S.selNet === S.mark.net) {
    ctx.strokeStyle = C.halo; ctx.lineWidth = 5 * px; ctx.beginPath(); ctx.arc(S.mark.x, S.mark.y, 9 * px, 0, Math.PI * 2); ctx.stroke();
    ctx.strokeStyle = getComputedStyle(document.documentElement).getPropertyValue("--accent").trim(); ctx.lineWidth = 2 * px; ctx.stroke();
  }
  drawXtMarks(px);
  drawPathMarks(px);
}
// one colour group of netPaths: tracks stroked, pads filled with their outline; `halo` (css px):
// the outline drawn wider in the halo colour instead
// Labels on the board at a constant screen size: the font is set in CSS pixels on a screen-space
// transform, so the text is crisp at any zoom and devicePixelRatio (text drawn in board units at
// size N·px gets blocky when zoomed in). items: [{x, y, text, color?, bold?, dx?, dy?}] with the
// anchor in board mm, most important first; each label sits right of and above its anchor (dx, dy
// in CSS px, per item or in opt) and is skipped when it would overlap one already drawn or lie
// outside the view. opt.center: the anchor is the text's centre (a letter in a marker). Returns the
// drawn labels' screen boxes ([x0, y0, x1, y1] in CSS px). Restores the caller's transform. Used by
// every board view (voltage drop, current density, losses, temperature, PDN, net A/B marks).
function drawBoardLabels(items, opt = {}) {
  const d = window.devicePixelRatio || 1, v = S.view, size = opt.size || 11;
  const dx0 = opt.dx ?? (opt.center ? 0 : 6), dy0 = opt.dy ?? (opt.center ? 0 : -6);
  const cs = getComputedStyle(document.documentElement), mono = cs.getPropertyValue("--font-mono").trim() || "monospace";
  const W = cv.width / d, H = cv.height / d, boxes = [];
  ctx.save();
  ctx.setTransform(d, 0, 0, d, 0, 0);
  ctx.textAlign = "left"; ctx.textBaseline = "alphabetic"; ctx.lineJoin = "round"; ctx.lineWidth = 3;
  ctx.strokeStyle = opt.halo || S.colors.canvas;
  const fg = cs.getPropertyValue("--fg").trim() || "#000";
  for (const it of items) {
    if (opt.max && boxes.length >= opt.max) break;
    ctx.font = `${it.bold ? 700 : 600} ${size}px ${mono}`;
    const w = ctx.measureText(it.text).width;
    let sx = v.x + it.x * v.s + (it.dx ?? dx0), sy = v.y + it.y * v.s + (it.dy ?? dy0);
    if (opt.center) { sx -= w / 2; sy += 0.36 * size; }
    const b = [sx - 2, sy - size, sx + w + 2, sy + 0.3 * size];
    if (b[2] < 0 || b[0] > W || b[3] < 0 || b[1] > H) continue;
    if (boxes.some((q) => b[0] < q[2] && b[2] > q[0] && b[1] < q[3] && b[3] > q[1])) continue;
    boxes.push(b);
    ctx.strokeText(it.text, sx, sy);
    ctx.fillStyle = it.color || opt.color || fg; ctx.fillText(it.text, sx, sy);
  }
  ctx.restore();
  return boxes;
}

function drawGroup(g, px, halo = 0) {
  const C = S.colors;
  ctx.strokeStyle = halo ? C.halo : g.col; ctx.lineWidth = Math.max(g.w, 1.2 * px) + halo * px; ctx.stroke(g.path);
  if (!g.fill) return;
  if (halo) { ctx.lineWidth = halo * px; ctx.stroke(g.fill); } else { ctx.fillStyle = g.col; ctx.fill(g.fill); }
}
function zoomTo(x, y, half) {
  S.mark = { x, y, net: S.selNet };
  fitView([x - half, y - half, x + half, y + half]);
}

function pick(mx, my) {
  if (!S.pieces) return -1;
  const v = S.view, x = (mx - v.x) / v.s, y = (my - v.y) / v.s, tol = 5 / v.s;
  let best = -1, bestD = Infinity, bestL = 99;
  const pc = S.pieces;
  const r = Math.ceil(tol);
  for (let cx = Math.floor(x) - r; cx <= Math.floor(x) + r; cx++) for (let cy = Math.floor(y) - r; cy <= Math.floor(y) + r; cy++) {
    const a = S.grid.get(cx * 100003 + cy); if (!a) continue;
    for (const i of a) {
      if (!S.layerOn[pc.layer[i]]) continue;
      const dx = pc.bx[i] - pc.ax[i], dy = pc.by[i] - pc.ay[i], L2 = dx * dx + dy * dy || 1e-12;
      let t = ((x - pc.ax[i]) * dx + (y - pc.ay[i]) * dy) / L2; t = Math.max(0, Math.min(1, t));
      const ex = pc.ax[i] + t * dx - x, ey = pc.ay[i] + t * dy - y;
      const dd = Math.sqrt(ex * ex + ey * ey) - pc.w[i] / 2;
      if (dd > tol) continue;
      const score = Math.max(dd, 0);
      if (score < bestD - 1e-6 || (Math.abs(score - bestD) < 1e-6 && pc.layer[i] < bestL)) { best = i; bestD = score; bestL = pc.layer[i]; }
    }
  }
  return best;
}

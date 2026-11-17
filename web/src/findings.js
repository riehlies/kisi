"use strict";
// ============================================================== findings: jump to a place on the board
// A place comes from Rust (kisi_si::locate::Spot – the report's findings, kisi_locate for a channel
// row): {at: [x, y] mm (KiCad board coordinates, y down), layer, layers, net, item, refs, what, bbox}.
// showSpot() opens the board view there: the item's layers on (the others off), its net selected for
// tracks, pads and nets, the view zoomed to it with a marker. Sources: a channel check row (channel.js),
// a finding of a report opened from the viewer (export.js), a link #finding=<id>&at=x,y&layer=…&net=…
// (&board=…) in the page's address, also before a board is loaded (applied once it is).

// the link's place: {id, board, spot} or null
function findingFromHash(h) {
  const q = String(h || "").replace(/^#/, "");
  if (!/(^|&)finding=/.test(q)) return null;
  const v = {};
  for (const kv of q.split("&")) {
    const i = kv.indexOf("=");
    if (i > 0) { try { v[kv.slice(0, i)] = decodeURIComponent(kv.slice(i + 1)); } catch (e) { v[kv.slice(0, i)] = kv.slice(i + 1); } }
  }
  const at = (v.at || "").split(",").map(Number);
  if (at.length !== 2 || !at.every(isFinite)) return { id: v.finding, board: v.board || "", spot: null };
  const layers = v.layers ? v.layers.split(",").map((x) => { try { return decodeURIComponent(x); } catch (e) { return x; } }) : v.layer ? [v.layer] : [];
  return { id: v.finding, board: v.board || "", spot: { at, layer: v.layers ? "" : v.layer || "", layers, net: v.net || "", item: v.item || (v.net ? "track" : "part"), refs: [], what: "", bbox: [at[0] - 3, at[1] - 3, at[0] + 3, at[1] + 3] } };
}

// open the board view at a place; opt.tab: "ch" keeps the channel view's board map (a check row),
// opt.keepNet: leave the selection as it is; opt.label: the text for the status bar
function showSpot(sp, opt = {}) {
  if (!S.scene || !sp || !Array.isArray(sp.at)) return false;
  const L = S.scene.stackup.copper.map((c) => c.name);
  const tab = opt.tab || "si";
  if (S.tab !== tab) setTab(tab);
  if (tab === "ch" && S.chMode !== "m") setChMode("m");
  // the item's layers: a track or SMD pad its layer, a via the layers it joins, a net its layers
  const ls = (sp.layer ? [sp.layer] : sp.layers || []).map((n) => L.indexOf(n)).filter((i) => i >= 0);
  if (ls.length) { S.layerOn = L.map((_, i) => ls.includes(i)); renderLayerChips(); }
  const net = sp.net ? S.scene.nets.indexOf(sp.net) : -1;
  if (!opt.keepNet && net >= 0 && ["track", "pad", "net"].includes(sp.item) && S.an && S.an.nets.some((n) => n.net === net)) selectNet(net, false);
  const b = sp.bbox && sp.bbox.length === 4 ? sp.bbox : [sp.at[0] - 3, sp.at[1] - 3, sp.at[0] + 3, sp.at[1] + 3];
  const half = Math.max(5, Math.min(12, Math.max(b[2] - b[0], b[3] - b[1]) / 2));
  S.mark = { x: sp.at[0], y: sp.at[1], net: null };
  fitView([sp.at[0] - half, sp.at[1] - half, sp.at[0] + half, sp.at[1] + half]);
  // the place in the part of the board left free by an open detail panel
  const d = [$("detail"), $("ch-detail")].find((e) => e && !e.hidden), cr = cv.getBoundingClientRect();
  if (d) {
    const dr = d.getBoundingClientRect(), over = Math.max(0, cr.right - dr.left);
    if (dr.top < cr.bottom - 0.4 * cr.height && over < 0.7 * cr.width) { S.view.x -= over / 2; draw(); }
  }
  const where = [sp.layer || (sp.layers || []).join("/"), `(${sp.at[0].toFixed(2)}, ${sp.at[1].toFixed(2)}) mm`, (sp.refs || []).join(", ")].filter(Boolean).join(" · ");
  xpStatus(`<span>${opt.label ? esc(opt.label) + ": " : ""}${esc(where)}${sp.what && sp.what !== opt.label ? ` – ${esc(sp.what)}` : ""}</span> <button class="linklike" id="xp-cancel" aria-label="Close">×</button>`);
  $("xp-cancel").onclick = () => { xpStatus(""); S.mark = null; draw(); };
  return true;
}

// a link #finding=…: now, or once a board is loaded
let JUMP_PENDING = null;
function jumpFromHash() {
  const f = findingFromHash(location.hash);
  if (!f || !f.spot) return;
  JUMP_PENDING = f;
  if (S.scene) jumpPending();
  else xpStatus(`<span>Finding ${esc(f.id)}${f.board ? ` on ${esc(f.board)}` : ""}: load the board to show it.</span>`);
}
function jumpPending() {
  const f = JUMP_PENDING;
  if (!f || !S.scene) return;
  JUMP_PENDING = null;
  showSpot(f.spot, { label: `Finding ${f.id}${f.board && f.board !== S.name ? ` (of ${f.board}, not this board)` : ""}` });
}
window.addEventListener("hashchange", jumpFromHash);
jumpFromHash();

// a channel check row: where its worst criterion applies (kisi_locate, the report's rule), or null
function rowSpot(r) {
  const lane = r.lane, p = lane ? laneProfile(lane) : profileOf(r.net), t = classTarget(lane ? laneNet(lane) : r.net);
  const st = lane ? null : S.an.nets.find((x) => x.net === r.net);
  const nets = lane ? [...lane.nets, ...lane.partner_nets] : [r.net, ...(st && st.partner != null ? [st.partner] : [])];
  const rx = lane && lane.ends.length >= 2 ? laneRx(lane) : null;
  try {
    return S.engine.locate({ metrics: rowMetrics(r, lane), profile: p, limits: p.limits, nets, target: t ? [t.z, t.diff] : null,
      rx: rx ? { refdes: rx.refdes, pin: rx.pin } : null, stub_at: r.stub_at || null });
  } catch (e) { console.warn(e); return null; }
}

// the report window opened from the viewer: its "show in the viewer" links jump here
function hookReportWindow(w) {
  try {
    const h = w.document.querySelector(".hint");
    if (h) h.textContent = "Opened from the kisi viewer: “show in the viewer” at a finding shows its place on the board there. Print this page or save it as PDF with the browser's print dialog (Ctrl+P, on a Mac ⌘P).";
    w.document.addEventListener("click", (e) => {
      const a = e.target.closest && e.target.closest("a.jump");
      if (!a) return;
      e.preventDefault();
      const f = findingFromHash(a.getAttribute("href"));
      if (f && f.spot && showSpot(f.spot, { label: `Finding ${f.id}` })) window.focus();
    });
  } catch (e) { console.warn(e); }
}

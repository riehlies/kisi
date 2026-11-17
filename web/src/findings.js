"use strict";
// ============================================================== findings: the list and the jump to a place
// A place comes from Rust (kisi_si::locate::Spot – the report's findings, kisi_locate for a channel
// row): {at: [x, y] mm (KiCad board coordinates, y down), layer, layers, net, item, refs, what, bbox,
// channel (a channel's whole box)}. jumpFinding() opens the view that shows a finding:
// * a channel (lane or net without devices): the channel view's board map with the whole channel selected
//   (all its nets, both lines of a pair), every layer on, the view on the channel, the marker at the place;
// * a net class's impedance: the impedance view, the net selected, the place's layer;
// * a rail (vias over their estimate, a rail warning): the Power view's voltage drop map of that rail on
//   the place's layer; a part's temperature: the temperature map – the DC solution (and the temperature)
//   computed first where missing, with the progress of their steps;
// * a gap's parts: the board view with the marker.
// Sources: the findings list of the Findings & export page (xpFindings), a channel check row
// (channel.js, kisi_locate), "show in the viewer" in a report opened from the viewer, a link
// #finding=<id>&at=x,y&layer=…&layers=…&net=…&item=…&box=…&board=… (also before a board is loaded).

// the link's finding: {id, area, item, board, location} or null
function findingFromHash(h) {
  const q = String(h || "").replace(/^#/, "");
  if (!/(^|&)finding=/.test(q)) return null;
  const v = {};
  for (const kv of q.split("&")) {
    const i = kv.indexOf("=");
    if (i > 0) { try { v[kv.slice(0, i)] = decodeURIComponent(kv.slice(i + 1)); } catch (e) { v[kv.slice(0, i)] = kv.slice(i + 1); } }
  }
  const id = v.finding || "", k = id.indexOf(":");
  const f = { id, area: k > 0 ? id.slice(0, k) : "", item: k > 0 ? id.slice(k + 1).replace(/#\d+$/, "") : id, board: v.board || "", location: null };
  const at = (v.at || "").split(",").map(Number);
  if (at.length !== 2 || !at.every(isFinite)) return f;
  const layers = v.layers ? v.layers.split(",") : v.layer ? [v.layer] : [];
  const box = (v.box || "").split(",").map(Number);
  f.location = { at, layer: v.layers ? "" : v.layer || "", layers, net: v.net || "", item: v.item || (v.net ? "track" : "part"), refs: [], what: "",
    bbox: [at[0] - 3, at[1] - 3, at[0] + 3, at[1] + 3], channel: box.length === 4 && box.every(isFinite) ? box : null };
  return f;
}

// open the board at a place: opt.tab the view ("si" board view, "ch" the channel view's board map, "pi" the
// Power view's map as set up by the caller), opt.keepNet: leave the selection, opt.keepLayers: leave the
// layer switches, opt.label: for the top bar. A channel's place (sp.channel): every layer on, the view on
// the whole channel; else the item's layers and the view around it.
function showSpot(sp, opt = {}) {
  if (!S.scene || !sp || !Array.isArray(sp.at)) return false;
  const L = S.scene.stackup.copper.map((c) => c.name);
  const tab = opt.tab || "si";
  if (S.tab !== tab) setTab(tab);
  if (tab === "ch" && S.chMode !== "m") setChMode("m");
  const box = Array.isArray(sp.channel) && sp.channel.length === 4 ? sp.channel : null;
  if (!opt.keepLayers && tab !== "pi") {
    // a channel: all its layers; an item: its layers (a track or SMD pad its layer, a via the layers it joins)
    const ls = box ? L.map((_, i) => i) : (sp.layer ? [sp.layer] : sp.layers || []).map((n) => L.indexOf(n)).filter((i) => i >= 0);
    if (ls.length) { S.layerOn = L.map((_, i) => ls.includes(i)); renderLayerChips(); }
  }
  const net = sp.net ? S.scene.nets.indexOf(sp.net) : -1;
  if (!opt.keepNet && tab !== "pi" && net >= 0 && ["track", "pad", "net"].includes(sp.item) && S.an && S.an.nets.some((n) => n.net === net)) selectNet(net, false);
  S.mark = { x: sp.at[0], y: sp.at[1], net: null };
  if (box) {
    const m = Math.max(1.5, 0.08 * Math.max(box[2] - box[0], box[3] - box[1]));
    fitView([Math.min(box[0], sp.at[0]) - m, Math.min(box[1], sp.at[1]) - m, Math.max(box[2], sp.at[0]) + m, Math.max(box[3], sp.at[1]) + m]);
  } else {
    const b = sp.bbox && sp.bbox.length === 4 ? sp.bbox : [sp.at[0] - 3, sp.at[1] - 3, sp.at[0] + 3, sp.at[1] + 3];
    const half = Math.max(5, Math.min(12, Math.max(b[2] - b[0], b[3] - b[1]) / 2));
    fitView([sp.at[0] - half, sp.at[1] - half, sp.at[0] + half, sp.at[1] + half]);
  }
  // the view in the part of the board left free by an open detail panel
  const d = [$("detail"), $("ch-detail"), $("pi-detail")].find((e) => e && !e.hidden && e.offsetParent), cr = cv.getBoundingClientRect();
  if (d) {
    const dr = d.getBoundingClientRect(), over = Math.max(0, cr.right - dr.left);
    if (dr.top < cr.bottom - 0.4 * cr.height && over < 0.7 * cr.width) { S.view.x -= over / 2; draw(); }
  }
  const where = [sp.layer || (sp.layers || []).join("/"), `(${sp.at[0].toFixed(2)}, ${sp.at[1].toFixed(2)}) mm`, (sp.refs || []).join(", ")].filter(Boolean).join(" · ");
  xpStatus(`<span>${opt.label ? esc(opt.label) + ": " : ""}${esc(where)}${sp.what && sp.what !== opt.label ? ` – ${esc(sp.what)}` : ""}</span> <button class="linklike" id="xp-cancel" aria-label="Close">×</button>`);
  $("xp-cancel").onclick = () => { xpStatus(""); S.mark = null; draw(); };
  return true;
}
// the marker of a finding's place on any view (draw.js and the Power view's maps), at a constant screen size
function drawJumpMark() {
  if (!S.mark || S.mark.net != null) return;
  const d = window.devicePixelRatio || 1, v = S.view, px = 1 / v.s;
  ctx.save();
  ctx.setTransform(v.s * d, 0, 0, v.s * d, v.x * d, v.y * d);
  const m = S.mark, r = 9 * px;
  ctx.beginPath(); ctx.arc(m.x, m.y, r, 0, Math.PI * 2);
  ctx.moveTo(m.x - 2 * r, m.y); ctx.lineTo(m.x + 2 * r, m.y); ctx.moveTo(m.x, m.y - 2 * r); ctx.lineTo(m.x, m.y + 2 * r);
  ctx.strokeStyle = S.colors.halo; ctx.lineWidth = 5 * px; ctx.stroke();
  ctx.strokeStyle = getComputedStyle(document.documentElement).getPropertyValue("--accent").trim(); ctx.lineWidth = 2 * px; ctx.stroke();
  ctx.restore();
}

// the DC solution (and with `thermal` the temperature) computed where missing, with the progress of their
// steps in the Power view; false without a power tree
async function piEnsure(thermal) {
  if (!S.piSetups || !S.piSetups.size) return false;
  if (!S.piSolved) {
    xpStatus("<span>Computing the DC drop of every rail for this finding …</span>");
    if (S.tab !== "pi") setTab("pi");
    await solveAllRails();
  }
  if (thermal && !thermOk()) {
    xpStatus("<span>Computing the temperature for this finding …</span>");
    await solveThermal();
  }
  return S.piSolved && (!thermal || thermOk());
}

// open the view that shows a finding (see above)
async function jumpFinding(f) {
  const sp = f && f.location;
  const label = `${f.severity === "fail" ? "fail" : f.severity === "warn" ? "warning" : f.severity === "gap" ? "gap" : "finding"} ${f.id}`;
  if (!sp || !S.scene) { xpStatus(`<span>${esc(f ? f.id : "")}: no single place on the board.</span> <button class="linklike" id="xp-cancel">×</button>`); $("xp-cancel").onclick = () => xpStatus(""); return false; }
  const L = S.scene.stackup.copper.map((c) => c.name), li = (n) => L.indexOf(n);
  if (f.area === "channel") {
    // the lane named so, else the net without devices: the whole channel selected in the board map
    const lane = S.buses ? allLanes().find((l) => l.name === f.item) : null;
    const net = lane ? laneNet(lane) : S.scene.nets.indexOf(f.item);
    setTab("ch"); if (S.chMode !== "m") setChMode("m");
    S.selLane = lane ? laneKey(lane) : null;
    if (net >= 0) selectNet(net, false);
    return showSpot(sp, { tab: "ch", keepNet: true, label });
  }
  if (f.area === "vias" || f.area === "power" || f.area === "temperature") {
    const th = f.area === "temperature";
    if (await piEnsure(th)) {
      setTab("pi");
      const layer = li(sp.layer || (sp.layers || [])[0] || "");
      if (th) { setPiMode("t"); thermJump(sp.at, layer >= 0 ? layer : S.thermLayer, f.item); }
      else {
        if (sp.net && (S.piSetups.has(sp.net) || (S.piGround && S.piGround.has(sp.net)))) S.piRail = sp.net;
        S.piLayer = layer; S.piImg = null; setPiMode("v");
      }
      return showSpot(sp, { tab: "pi", keepNet: true, keepLayers: true, label });
    }
  }
  return showSpot(sp, { label });
}

// a link #finding=…: now, or once a board is loaded
let JUMP_PENDING = null;
function jumpFromHash() {
  const f = findingFromHash(location.hash);
  if (!f || !f.location) return;
  JUMP_PENDING = f;
  if (S.scene) jumpPending();
  else xpStatus(`<span>Finding ${esc(f.id)}${f.board ? ` on ${esc(f.board)}` : ""}: load the board to show it.</span>`);
}
function jumpPending() {
  const f = JUMP_PENDING;
  if (!f || !S.scene) return;
  JUMP_PENDING = null;
  if (f.board && f.board !== S.name) f.id += ` (of ${f.board}, not this board)`;
  jumpFinding(f);
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
      const full = f && XP.done && XP.done.counts && XP.done.counts.findings.find((x) => x.id === f.id);
      if (f) { window.focus(); jumpFinding(full || f); }
    });
  } catch (e) { console.warn(e); }
}

// ---- the findings list of the Findings & export page: per net (part, rail, gap) its findings, from the
// report (kisi report in workers, as Results and Report); a click jumps to the finding
const XP_KIND = { net: "net", rail: "rail", part: "part", gap: "" };
const XP_SEV = { fail: "high2", warn: "high1", gap: "none" };
function xpFindings() {
  const box = $("xp-find");
  if (!box) return;
  const last = S.scene ? xpLast() : null, old = !last && XP.done && XP.done.board === S.name ? XP.done : null, use = last || old;
  const c = use && use.counts;
  // the badge on the top bar's button: the findings of the state shown
  const nc = last && last.counts;
  $("xp-count").hidden = !nc || !nc.n;
  if (nc) { $("xp-count").textContent = String(nc.n); $("xp-count").className = "count" + (nc.fail ? "" : nc.warn ? " m" : " z"); }
  $("xp-find-n").textContent = c ? `${c.n} · ${c.fail} fail, ${xpPlural(c.warn, "warning")}, ${xpPlural(c.gap, "gap")}` : "";
  const run = `<button class="btn small xp-run" id="xp-find-run" ${XP.run || !S.scene ? "disabled" : ""}>${c ? "Find again" : "Find the problems"}</button>`;
  if (!c) {
    box.innerHTML = `<p class="note">Every result over or near a limit and every open gap, per net, each with its place on the board – click one to show it there. Runs kisi report in the background first (the channel check, the DC drop, the temperature; about half a minute${S.name ? ` on ${esc(S.name)}` : ""}).</p>${run}`;
  } else {
    const byId = new Map(c.findings.map((f) => [f.id, f]));
    const groups = c.groups_list.length ? c.groups_list : c.findings.map((f) => ({ id: f.id, title: f.item, kind: "", severity: f.severity, findings: [f.id] }));
    box.innerHTML = (old ? `<p class="note warn">Computed before the last change (other settings, a field-solver result, another variant): the places still hold for the board, the values may differ – “Find again”.</p>` : "")
      + groups.map((g) => `<div class="xp-grp${old ? " xp-old" : ""}"><div class="xp-gh"><span class="dot" style="background:${S.colors[XP_SEV[g.severity]] || "var(--muted)"}"></span><span>${esc(g.title)}</span><span class="muted">${esc(XP_KIND[g.kind] || "")}${g.findings.length > 1 ? ` · ${g.findings.length}` : ""}</span></div>`
        + g.findings.map((id) => byId.get(id)).filter(Boolean).map((f) => `<button class="xp-fd" data-fid="${esc(f.id)}" ${f.location ? "" : "disabled"}><span class="dot" style="background:${S.colors[XP_SEV[f.severity]] || "var(--muted)"}" aria-label="${f.severity === "warn" ? "warning" : f.severity}"></span><span>${esc(f.what)}<span class="w">${esc(f.location ? xpWhere(f.location) : "board-wide")}</span></span></button>`).join("") + "</div>").join("")
      + `<p class="note">Coordinates in mm as KiCad shows them (x →, y ↓).</p>${run}`;
    box.querySelectorAll("[data-fid]").forEach((b) => b.onclick = () => { const f = byId.get(b.dataset.fid); xpMenu(false); jumpFinding(f); });
  }
  const r = $("xp-find-run");
  if (r) r.onclick = () => xpRun(() => { xpMenu(true); }, null, true);
}
// a place in one line, as the report's "Where"
function xpWhere(l) {
  return [l.layer || (l.layers || []).join("/"), Array.isArray(l.at) ? `(${l.at[0].toFixed(2)}, ${l.at[1].toFixed(2)}) mm` : "", (l.refs || []).join(", ")].filter(Boolean).join(" · ");
}

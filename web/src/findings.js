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
// Sources: the findings list of the Findings & export page (xpFindings, below), a channel check row
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
  // from either page: back to the board
  if (XP.open) xpOpen(false);
  if (S.prep.open) prepOpen(false);
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


// ---- the Findings & export page: a page of its own like Prepare PCB's – it takes the place of the board
// views; the top bar's button opens and closes it, a view button or Escape goes back to the board, opening one
// of the two pages closes the other. Tab "Findings": the report's findings per net (part, rail, gap), from
// kisi report (run in workers, export.js; also filled by Report and Results), filtered by severity, area and
// text; a click closes the page and jumps. Tab "Export": the four exports as cards (export.js, xpSync).
const XP_KIND = { net: "net", rail: "rail", part: "part", gap: "" };
const XP_SEV = { fail: "high2", warn: "high1", gap: "none" };
const XP_SEVN = { fail: "fail", warn: "warning", gap: "gap" };
// the filter's areas and the findings' areas in each
const XP_AREA = { channel: ["channel", ["channel"]], impedance: ["impedance", ["impedance"]], power: ["power", ["vias", "power"]], thermal: ["thermal", ["temperature"]], gap: ["gaps", ["gap"]] };
const xpAreaOf = (f) => Object.keys(XP_AREA).find((k) => XP_AREA[k][1].includes(f.area)) || "gap";
Object.assign(XP, { open: false, tab: "find", f: { sev: new Set(), area: new Set(), q: "" }, byId: new Map(), scroll: [0, 0] });

function xpOpen(on) {
  if (on && !S.scene) return;
  if (on && S.prep.open) prepOpen(false);
  if (!on && XP.open) XP.scroll = [$("xp").scrollTop, window.scrollY];
  XP.open = on;
  $("app").classList.toggle("xp-open", on);
  $("xp").hidden = !on;
  $("xp-btn").setAttribute("aria-pressed", String(on));
  for (const k of ["stk", "si", "ch", "pi"]) $("view-" + k).setAttribute("aria-pressed", String(!on && S.tab === k));
  if (on) { xpRender(); $("xp").scrollTop = XP.scroll[0]; window.scrollTo(0, XP.scroll[1]); }
}
// the report's findings for the state shown (last), else the board's older ones (old, greyed)
function xpState() {
  const last = S.scene ? xpLast() : null, old = !last && XP.done && XP.done.board === S.name ? XP.done : null;
  return { last, old, c: (last || old || {}).counts || null };
}
// the badge on the top bar's button: the findings of the state shown
function xpBadge() {
  const l = S.scene ? xpLast() : null, c = l && l.counts, b = $("xp-count");
  b.hidden = !c || !c.n;
  if (c) { b.textContent = String(c.n); b.className = "count" + (c.fail ? "" : c.warn ? " m" : " z"); }
}
function xpRender() {
  xpBadge();
  if (!XP.open || !S.scene) return;
  const st = xpState(), c = st.c, find = XP.tab === "find";
  $("xp-board").textContent = S.name || "";
  xpSync(); // the export cards (export.js)
  const sev = c ? `${c.fail} fail, ${xpPlural(c.warn, "warning")}, ${xpPlural(c.gap, "gap")}` : "not computed yet";
  const mk = c ? (c.fail ? ["open h", c.n] : c.warn ? ["open m", c.n] : c.n ? ["open s", c.n] : ["done", "✓"]) : ["open s", "–"];
  const ex = ["xp-html", "xp-csv", "xp-json", "xp-cfg"].filter((i) => !$(i).disabled).length;
  $("xp-ready").innerHTML = [["find", "Findings", mk, (c ? `${c.n} · ` : "") + sev + (st.old ? " (before the last change)" : "")], ["export", "Export", ["open s", ex], `${ex} of 4 ready`]]
    .map(([t, n, m, s]) => `<button data-xp-tab="${t}" aria-current="${XP.tab === t}"><span class="mk ${m[0]}">${m[1]}</span><b>${n}</b><span>${esc(s)}</span></button>`).join("");
  $("xp-tab-find").setAttribute("aria-selected", String(find)); $("xp-tab-export").setAttribute("aria-selected", String(!find));
  $("xp-tab-find-n").textContent = c ? `${c.n}${st.old ? " · old" : ""}` : "";
  $("xp-pane-find").hidden = !find; $("xp-pane-export").hidden = find;
  // the summary: findings by severity, what they are on, whether they hold for what is shown
  const kpi = (v, l, col) => `<div class="kpi"><span class="v"${v && col ? ` style="color:${S.colors[col]}"` : ""}>${v}</span><span class="l">${l}</span></div>`;
  $("xp-sum").innerHTML = c ? kpi(c.fail, "fail", "high2") + kpi(c.warn, c.warn === 1 ? "warning" : "warnings", "high1") + kpi(c.gap, c.gap === 1 ? "open gap" : "open gaps") : kpi("–", "fail") + kpi("–", "warnings") + kpi("–", "open gaps");
  const kinds = c ? ["net", "rail", "part"].map((k) => [k, c.groups_list.filter((g) => g.kind === k).length]).filter((x) => x[1]).map(([k, n]) => xpPlural(n, k)) : [];
  $("xp-scope").textContent = c ? (c.n ? `${xpPlural(c.n, "finding")}${kinds.length ? ` on ${kinds.join(", ")}` : ""}${c.gap ? `${kinds.length ? " and" : ""} ${xpPlural(c.gap, "gap")}` : ""}.` : "No finding: every check within its limits, no open gap.") : `Checks ${xpScope() || "the board"}.`;
  $("xp-state").className = "note" + (st.old ? " warn" : "");
  $("xp-state").textContent = XP.run ? "" : st.last ? "Current: computed for the settings, variant and stackup shown." : st.old ? "Computed before the last change (other settings, a field-solver result, another variant): the places still hold, the values may differ – “Find again”." : "Not computed yet for this board.";
  $("xp-find-run").textContent = c ? "Find again" : "Find the problems";
  xpFindings(st);
  xpProg();
}
// the run's progress on the page (the top bar shows it too)
function xpProg() {
  if (!XP.open) return;
  const p = XP.prog, run = !!XP.run;
  $("xp-barbox").hidden = !run;
  $("xp-bar").style.width = p && p.n ? `${Math.round(100 * p.d / p.n)}%` : "3%";
  $("xp-prog").textContent = run && p ? `kisi report: ${p.what}${p.n ? ` · ${Math.round(100 * p.d / p.n)} %` : ""} · ${p.s} s` : "";
  $("xp-find-run").disabled = run || !S.scene; $("xp-find-stop").hidden = !run;
}
// the list: per net (part, rail, gap) its findings that pass the filter
function xpFindings(st) {
  const box = $("xp-find"), c = st.c, F = XP.f;
  $("xp-filter").hidden = !c || !c.n;
  if (!c) {
    box.innerHTML = `<p class="prep-empty">No findings yet. “Find the problems” runs kisi report on ${esc(S.name)} and lists every result over or near a limit and every open gap, per net, each with its place on the board.</p>`;
    return;
  }
  if (!c.n) { box.innerHTML = `<p class="prep-empty">kisi report finds nothing over or near a limit and no open gap.</p>`; return; }
  const byId = XP.byId = new Map(c.findings.map((f) => [f.id, f]));
  const groups = c.groups_list.length ? c.groups_list : c.findings.map((f) => ({ id: f.id, title: f.item, kind: "", severity: f.severity, findings: [f.id] }));
  const q = F.q.trim().toLowerCase(), dot = (s) => `<span class="dot" style="background:${S.colors[XP_SEV[s]] || "var(--muted)"}"></span>`;
  const pass = (f, g, skip) => (skip === "sev" || !F.sev.size || F.sev.has(f.severity)) && (skip === "area" || !F.area.size || F.area.has(xpAreaOf(f)))
    && (!q || [f.id, f.what, f.detail, g.title, ...(g.nets || []), f.location ? xpWhere(f.location) + " " + f.location.net : ""].join(" ").toLowerCase().includes(q));
  const gOf = new Map(groups.flatMap((g) => g.findings.map((id) => [id, g])));
  // each chip counts what it would show with the other filters kept
  const chip = (attr, k, label, on, n) => `<button data-${attr}="${k}" aria-pressed="${on}" ${n || on ? "" : "disabled"}>${attr === "xp-sev" ? dot(k) : ""}${label} <b>${n}</b></button>`;
  $("xp-sev").innerHTML = Object.entries(XP_SEVN).map(([k, l]) => chip("xp-sev", k, l, F.sev.has(k), c.findings.filter((f) => f.severity === k && pass(f, gOf.get(f.id) || {}, "sev")).length)).join("");
  $("xp-area").innerHTML = Object.entries(XP_AREA).map(([k, [l]]) => chip("xp-area", k, l, F.area.has(k), c.findings.filter((f) => xpAreaOf(f) === k && pass(f, gOf.get(f.id) || {}, "area")).length)).join("");
  let shown = 0;
  const html = groups.map((g) => {
    const fs = g.findings.map((id) => byId.get(id)).filter((f) => f && pass(f, g));
    if (!fs.length) return "";
    shown += fs.length;
    return `<div class="xp-grp${st.old ? " xp-old" : ""}"><div class="xp-gh">${dot(g.severity)}<span>${esc(g.title)}</span><span class="muted">${esc(XP_KIND[g.kind] || "")}${g.findings.length > 1 ? ` · ${g.findings.length}` : ""}</span></div>`
      + fs.map((f) => `<button class="xp-fd" data-fid="${esc(f.id)}" aria-current="${f.id === XP.lastJump}" ${f.location ? "" : "disabled"}>${dot(f.severity).replace("></span>", ` aria-label="${XP_SEVN[f.severity] || f.severity}"></span>`)}<span>${esc(f.what)}${f.estimate ? ` <em class="est">estimate</em>` : ""}<span class="w">${esc(f.location ? xpWhere(f.location) : "board-wide: no single place")}</span>${f.detail ? `<span class="d">${esc(f.detail)}</span>` : ""}</span></button>`).join("") + "</div>";
  }).join("");
  $("xp-shown").textContent = shown === c.n ? xpPlural(c.n, "finding") : `${shown} of ${c.n} shown`;
  box.innerHTML = (st.old ? `<p class="note warn">Computed before the last change – the values may differ; “Find again” for the state shown.</p>` : "")
    + (html || `<p class="prep-empty">No finding matches the filter. <button class="linklike" id="xp-clear">Show all</button></p>`)
    + `<p class="note">Coordinates in mm as KiCad shows them (x →, y ↓). A click shows the finding on the board.</p>`;
}
// a place in one line, as the report's "Where"
function xpWhere(l) {
  return [l.layer || (l.layers || []).join("/"), Array.isArray(l.at) ? `(${l.at[0].toFixed(2)}, ${l.at[1].toFixed(2)}) mm` : "", (l.refs || []).join(", ")].filter(Boolean).join(" · ");
}
$("xp-btn").onclick = () => xpOpen(!XP.open);
for (const k of ["stk", "si", "ch", "pi"]) $("view-" + k).addEventListener("click", () => { if (XP.open) xpOpen(false); });
// Escape outside a form field: from either page back to the board
document.addEventListener("keydown", (e) => {
  if (e.key !== "Escape" || /^(INPUT|SELECT|TEXTAREA)$/.test((e.target && e.target.tagName) || "")) return;
  if (XP.open) { xpOpen(false); $("xp-btn").focus(); } else if (S.prep.open) { prepOpen(false); $("prep-btn").focus(); }
});
$("xp-q").oninput = () => { XP.f.q = $("xp-q").value; xpFindings(xpState()); };
$("xp").addEventListener("click", (ev) => {
  const t = ev.target.closest("button");
  if (!t) return;
  const d = t.dataset;
  if (d.xpTab) { XP.tab = d.xpTab; xpRender(); return; }
  if (d.xpSev || d.xpArea) { const s = d.xpSev ? XP.f.sev : XP.f.area, k = d.xpSev || d.xpArea; if (!s.delete(k)) s.add(k); xpFindings(xpState()); return; }
  if (t.id === "xp-clear") { XP.f.sev.clear(); XP.f.area.clear(); XP.f.q = $("xp-q").value = ""; xpFindings(xpState()); return; }
  if (d.fid) { const f = XP.byId.get(d.fid); if (f) { XP.lastJump = f.id; jumpFinding(f); } return; }
  if (t.id === "xp-find-run") xpRun(() => xpRender());
  else if (t.id === "xp-find-stop" && XP.run) XP.run.cancel.on = true;
});

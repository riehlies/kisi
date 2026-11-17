"use strict";
// ============================================================== Advanced solvers (prototype)
// Only in a build with `tools/bundle.py --advanced` (docs/research/advanced-solvers.md); the default kisi.html has
// none of this. The option "Advanced solvers" (Solver options of the field solver, off by default) adds "Fields…" to
// a net's detail: the cross-section of the clicked segment (or another of the net's) opens in the field views of the
// js_2d_fields fork (potential, |E|, |H|, current density, power flow with the ∫S dA check, losses split by
// conductor and dielectric), solved there:
//   local   kisi-advanced.html next to this page (tools/advanced_bundle.py; offline, also from disk)
//   hosted  the fork's hosted page (needs the internet; the cross-section goes along in the URL fragment, which is
//           never sent to the server)
//   inline  the page built into this file (bundle.py --advanced-inline), opened from a blob: URL
// The hand-over is the original's link format (#params=, settings as deflate-raw + base64url) plus the keys of
// web/advanced/handover.js (solve, view, title, editor); the next cross-section changes only the fragment of the
// open window (no reload). kisi writes the geometry text it also solves with (xsgeom.rs); the page uses its own
// defaults for everything else (smooth copper 5.8e7 S/m, the materials' εr and tanδ from the text).
const ADV_KEY = "kisi.advanced.v1";
const ADV_LOCAL = "kisi-advanced.html";
const ADV_HOSTED = "https://advpcb.davidriehl.de/tl/field_solver.html";
const ADV_FREQS = [[0.1, "100 MHz"], [1, "1 GHz"], [5, "5 GHz"], [10, "10 GHz"]];
const ADV_VIEWS = [["sfield", "Power flow S"], ["efield", "|E|"], ["potential", "Potential"], ["hfield", "|H|"], ["jfield", "Current J"], ["lossfield", "Losses"]];
const ADV = Object.assign({ on: false, target: "local", backend: "fullwave_mqs", freq: 1, view: "sfield" }, lsGet(ADV_KEY) || {});
ADV.win = null; ADV.n = 0; ADV.piece = -1; ADV.blob = null;
const advInline = () => typeof ADV_INLINE_GZ === "string" && ADV_INLINE_GZ.length > 0;
if (ADV.target === "inline" && !advInline()) ADV.target = "local";
const advSave = () => lsSet(ADV_KEY, { on: ADV.on, target: ADV.target, backend: ADV.backend, freq: ADV.freq, view: ADV.view });
const advOpts = (list, cur) => list.map(([v, t]) => `<option value="${esc(v)}" ${String(v) === String(cur) ? "selected" : ""}>${esc(t)}</option>`).join("");

// ---- the option and its settings (Solver options of step 3)
function renderAdvOption() {
  let box = $("adv-opts");
  if (!box) {
    box = document.createElement("div");
    box.id = "adv-opts";
    box.className = "fs-settings";
    $("fs-settings").append(box);
  }
  const targets = [["local", "kisi-advanced.html next to this page"], ["hosted", "the fork's hosted page (online)"]].concat(advInline() ? [["inline", "the page built into this file"]] : []);
  box.innerHTML = `<label class="prep-chk" for="adv-on" title="Off by default. On: “Fields…” in a net's detail opens the cross-section in the field views of js_2d_fields (fork): potential, |E|, |H|, current density, power flow, losses"><input type="checkbox" id="adv-on" ${ADV.on ? "checked" : ""}> Advanced solvers (field views)</label>` +
    (ADV.on ? `<label class="opt" for="adv-target"><span>Open in</span><select id="adv-target">${advOpts(targets, ADV.target)}</select></label>
      <label class="opt" for="adv-backend"><span>Solver there</span><select id="adv-backend">${advOpts([["fullwave_mqs", "full-wave (all views)"], ["rectilinear", "quasi-static (potential, |E|)"]], ADV.backend)}</select></label>
      <label class="opt" for="adv-freq"><span>Frequency</span><select id="adv-freq">${advOpts(ADV_FREQS, ADV.freq)}</select></label>
      <label class="opt" for="adv-view"><span>First view</span><select id="adv-view">${advOpts(ADV_VIEWS, ADV.view)}</select></label>` : "");
  $("adv-on").onchange = (e) => { ADV.on = e.target.checked; advSave(); renderAdvOption(); if (S.selNet >= 0) selectNet(S.selNet, false); };
  for (const [id, key, num] of [["adv-target", "target"], ["adv-backend", "backend"], ["adv-freq", "freq", true], ["adv-view", "view"]]) {
    const el = $(id);
    if (el) el.onchange = () => { ADV[key] = num ? +el.value : el.value; advSave(); };
  }
}
renderAdvOption();

// ---- the clicked segment (before the click selects its net: capture phase on the board canvas)
cv.addEventListener("pointerup", (e) => {
  if (!ADV.on || dragMoved || S.tab !== "si" || !S.pieces) return;
  ADV.piece = pick(e.offsetX, e.offsetY);
}, true);

// cross-sections of a net with a geometry, longest first; the clicked segment's first
function advNetXs(net) {
  const pc = S.pieces, len = new Map();
  for (let i = 0; i < pc.n; i++) {
    if (pc.net[i] !== net || S.an.cross_sections[pc.xs[i]].geom == null) continue;
    len.set(pc.xs[i], (len.get(pc.xs[i]) || 0) + Math.hypot(pc.bx[i] - pc.ax[i], pc.by[i] - pc.ay[i]));
  }
  const list = [...len.entries()].sort((a, b) => b[1] - a[1]);
  const hit = ADV.piece >= 0 && pc.net[ADV.piece] === net ? pc.xs[ADV.piece] : null;
  if (hit != null) list.sort((a, b) => (b[0] === hit) - (a[0] === hit));
  return list.map(([xi, l]) => ({ xi, len: l, clicked: xi === hit }));
}
function advXsLabel(xs, len) {
  const L = S.scene.stackup.copper[xs.layer].name;
  const pair = xs.diff_gap != null ? ` / gap ${fmt(xs.diff_gap * 1000, 0)} µm, pair` : "";
  return `${L} · ${KIND_LABEL[xs.kind].split(",")[0]} · w ${fmt(xs.w * 1000, 0)} µm${pair} · ${fmt(len, 1)} mm`;
}

// ---- "Fields…" in the net detail
const advSelectNet = selectNet;
selectNet = function (net, zoom = true) {
  advSelectNet(net, zoom);
  if (!ADV.on || net < 0 || S.tab !== "si" || !S.an || S.an.skeleton) return;
  const d = $("detail"), stats = d.querySelector(".dstats");
  if (!stats || d.hidden) return;
  const list = advNetXs(net);
  if (!list.length) return;
  const row = document.createElement("div");
  row.className = "fs-row adv-row";
  row.innerHTML = `<label class="opt" for="adv-xs" style="flex:1 1 auto;min-width:0"><span>Fields</span><select id="adv-xs" style="min-width:0;max-width:100%">${
    list.map((x) => `<option value="${x.xi}">${esc((x.clicked ? "clicked: " : "") + advXsLabel(S.an.cross_sections[x.xi], x.len))}</option>`).join("")}</select></label><button class="btn small" id="adv-go" title="Open this cross-section in the field views (Advanced solvers)">Fields…</button>`;
  stats.after(row);
  $("adv-go").onclick = () => advOpen(net, +$("adv-xs").value);
};
const advShowTip = showTip;
showTip = function (i, mx, my) {
  advShowTip(i, mx, my);
  if (!ADV.on || i < 0 || S.tab !== "si" || !S.an || S.an.skeleton) return;
  const tip = $("tip");
  if (!tip.hidden && S.an.cross_sections[S.pieces.xs[i]].geom != null) tip.insertAdjacentHTML("beforeend", `<div class="muted">Click: “Fields…” in the net's detail shows this cross-section's fields</div>`);
};

// ---- hand-over
async function advDeflate(text) {
  const bytes = new Uint8Array(await new Response(new Blob([text]).stream().pipeThrough(new CompressionStream("deflate-raw"))).arrayBuffer());
  let bin = "";
  for (let i = 0; i < bytes.length; i += 0x8000) bin += String.fromCharCode.apply(null, bytes.subarray(i, i + 0x8000));
  return "z." + btoa(bin).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}
// the URL fragment for one cross-section (also used by tools/measure_advanced.py)
async function advFragment(net, xi, o = ADV) {
  const xs = S.an.cross_sections[xi], g = S.an.geometries[xs.geom];
  const settings = { tl_type: "custom", custom_geom: g.text, mesh_backend: o.backend, freq_start: o.freq, freq_stop: o.freq, freq_points: 1, custom_sigma: 5.8e7 };
  const quasi = o.backend === "rectilinear";
  const view = quasi && !["potential", "efield"].includes(o.view) ? "efield" : o.view;
  const title = `${S.scene.nets[net]} · ${advXsLabel(xs, 0).replace(/ · [^·]*mm$/, "")} · from kisi (${S.name})`;
  return `#params=${await advDeflate(JSON.stringify(settings))}&solve=1&view=${view}&editor=hide&title=${encodeURIComponent(title)}&n=${++ADV.n}`;
}
async function advBase() {
  if (ADV.target === "hosted") return ADV.hostedURL || ADV_HOSTED;  // hostedURL: a test hook (tools/measure_advanced.py)
  if (ADV.target === "inline") {
    if (!ADV.blob) ADV.blob = URL.createObjectURL(new Blob([await gunzipB64(ADV_INLINE_GZ)], { type: "text/html" }));
    return ADV.blob;
  }
  return new URL(ADV_LOCAL, location.href).href;
}
async function advOpen(net, xi) {
  // the window first (in the click, so no popup blocker), the address once the fragment is encoded
  let w = ADV.win && !ADV.win.closed ? ADV.win : window.open("", "kisi-advanced");
  if (!w) { $("adv-go").textContent = "Pop-up blocked"; $("adv-go").title = "The browser blocked the new window – allow pop-ups for this page."; return; }
  ADV.win = w;
  const [base, frag] = await Promise.all([advBase(), advFragment(net, xi)]);
  try { w.location.href = base + frag; w.focus(); } catch (e) { ADV.win = window.open(base + frag, "kisi-advanced"); }
}

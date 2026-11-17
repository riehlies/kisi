"use strict";
// ============================================================== field views (option "Advanced solvers")
// "Fields…" in a net's detail opens a cross-section in the field page next to this file (kisi-advanced.html, else
// kisi-advanced-lite.html): the fork's link hand-over in the URL fragment, the page's steps back by postMessage.
// What goes along and how the page is found: docs/reference.md "Field views"; values: data/field_views.json.
const FV_FILES = ["kisi-advanced.html", "kisi-advanced-lite.html"];
const FV_VIEWS = [["sfield", "Power flow S"], ["efield", "|E|"], ["potential", "Potential"], ["hfield", "|H|"], ["jfield", "Current J"], ["lossfield", "Losses"]];
const FV_LITE = ["potential", "efield"];
const FVD = DATA.field_views;
// per browser: on, first view, the file that answered last; per session: the window, its page and build
const FV = Object.assign({ on: false, view: "sfield", file: null }, lsGet("kisi.fields.v1") || {},
  { win: null, base: null, build: null, n: 0, msg: "", err: false, note: "", probeMs: 6000, waiters: [] });
const fvSave = () => lsSet("kisi.fields.v1", { on: FV.on, view: FV.view, file: FV.file });
const fvOn = () => FV.on && S.tab === "si" && S.an && !S.an.skeleton;
const fvWeb = () => /^https?:$/.test(location.protocol);
$("fv-on").checked = FV.on;
$("fv-on").onchange = (e) => { FV.on = e.target.checked; fvSave(); if (S.selNet >= 0) selectNet(S.selNet, false); };

// the net's cross-sections with a geometry, longest first, the clicked segment's (interaction.js) first
function fvNetXs(net) {
  const pc = S.pieces, len = new Map(), p = S.fvPiece ?? -1;
  for (let i = 0; i < pc.n; i++) if (pc.net[i] === net && S.an.cross_sections[pc.xs[i]].geom != null)
    len.set(pc.xs[i], (len.get(pc.xs[i]) || 0) + Math.hypot(pc.bx[i] - pc.ax[i], pc.by[i] - pc.ay[i]));
  const hit = p >= 0 && pc.net[p] === net ? pc.xs[p] : null;
  return [...len.entries()].sort((a, b) => (b[0] === hit) - (a[0] === hit) || b[1] - a[1]).map(([xi, l]) => ({ xi, len: l, hit: xi === hit }));
}
function fvXsLabel(xs) {
  return `${S.scene.stackup.copper[xs.layer].name} · ${KIND_LABEL[xs.kind].split(",")[0]} · w ${fmt(xs.w * 1000, 0)} µm` +
    (xs.diff_gap != null ? ` / gap ${fmt(xs.diff_gap * 1000, 0)} µm, pair` : "");
}
// the row in the net detail (net-detail.js) and the tooltip line
function fvRow(net) {
  const list = fvOn() ? fvNetXs(net) : [];
  if (!list.length) return "";
  const lite = FV.build === "lite";
  return `<div class="fv-row"><span class="fv-l">Fields</span><select id="fv-xs" aria-label="Cross-section">${list.map((x) =>
    `<option value="${x.xi}">${esc((x.hit ? "clicked: " : "") + fvXsLabel(S.an.cross_sections[x.xi]) + ` · ${fmt(x.len, 1)} mm`)}</option>`).join("")}</select>` +
    `<select id="fv-view" aria-label="First view">${FV_VIEWS.map(([v, t]) => `<option value="${v}"${v === FV.view ? " selected" : ""}>${t}${lite && !FV_LITE.includes(v) ? " (not in lite)" : ""}</option>`).join("")}</select>` +
    `<button class="btn small" id="fv-go" title="The cross-section's fields in the field page next to this file (a new window, reused)">Fields…</button></div>` +
    `<p class="note fv-msg${FV.err ? " err" : ""}" id="fv-msg"${FV.msg ? "" : " hidden"}>${FV.msg}</p>`;
}
function fvWire(net) {
  if (!$("fv-go")) return;
  $("fv-go").onclick = () => fvOpen(net, +$("fv-xs").value);
  $("fv-view").onchange = (e) => { FV.view = e.target.value; fvSave(); };
}
const fvTip = (i) => fvOn() && S.an.cross_sections[S.pieces.xs[i]].geom != null ? `<div class="muted">Click: “Fields…” in the net's detail shows its fields</div>` : "";
function fvStatus(html, err = false) {
  Object.assign(FV, { msg: html, err });
  const el = $("fv-msg");
  if (el) { el.innerHTML = html; el.hidden = !html; el.classList.toggle("err", err); }
}

// the hand-over of one cross-section to a page of this build: frequency, excitation, copper, theme, title
const fvHz = (g) => g >= 1 ? `${fmt(g, g % 1 ? 2 : 0)} GHz` : `${fmt(g * 1000, 0)} MHz`;
async function fvB64(text) {
  const b = new Uint8Array(await new Response(new Blob([text]).stream().pipeThrough(new CompressionStream("deflate-raw"))).arrayBuffer());
  let s = "";
  for (let i = 0; i < b.length; i += 0x8000) s += String.fromCharCode.apply(null, b.subarray(i, i + 0x8000));
  return "z." + btoa(s).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}
async function fvHandover(net, xi, lite) {
  const xs = S.an.cross_sections[xi], pair = xs.diff_gap != null, even = pair && S.pairMode === "c", ex = FVD.excitation;
  const id = profileIdOf(net), p = id && profile(id), ghz = p && p.rate_gbps > 0 ? p.rate_gbps / 2 : FVD.frequency.fallback_ghz;
  const why = p && p.rate_gbps > 0 ? `Nyquist of ${p.name || id}` : "no interface profile";
  const rough = $("rf-rough"), rq = FVD.copper.rq_um[rough.value] || 0;
  // kisi's roughness preset as the page's RMS roughness on every conductor (the text is in mm)
  const text = rq > 0 && /^units mm$/m.test(S.an.geometries[xs.geom].text)
    ? S.an.geometries[xs.geom].text.replace(/^((?:gnd|sig[+-]?)\s.*?)\s*$/gm, `$1 rq=${+(rq / 1000).toPrecision(4)}`) : S.an.geometries[xs.geom].text;
  const view = lite && !FV_LITE.includes(FV.view) ? "efield" : FV.view, act = fabOn() && stkActiveLabel();
  const set = { tl_type: "custom", custom_geom: text, mesh_backend: lite ? "rectilinear" : "fullwave_mqs", freq_start: ghz, freq_stop: ghz, freq_points: 1, custom_sigma: FVD.copper.sigma_s_m };
  const title = `${S.scene.nets[net]} · ${fvXsLabel(xs)} · ${fvHz(ghz)} (${why}) · ` +
    (!pair ? `${ex.single_v} V` : even ? `common mode ${ex.pair_even_vcm} V` : `${ex.pair_odd_vdiff} V differential`) +
    ` · copper ${rough.selectedOptions[0].textContent}${rq > 0 ? ` (Rq ${fmt(rq, 2)} µm, estimate)` : ""}${act ? ` · stackup ${act} (what-if)` : ""} · kisi, ${S.name}`;
  const t = document.documentElement.dataset.theme, theme = t || (matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light");
  const exc = !pair ? `V:${ex.single_v}:peak` : even ? `V:${ex.pair_even_vcm}:peak` : `Vd:${ex.pair_odd_vdiff}:peak`;
  FV.note = view !== FV.view ? " – the lite page shows |E| instead" : "";
  return { at: `${fvHz(ghz)} (${esc(why)})`, frag: `#params=${await fvB64(JSON.stringify(set))}&solve=1&view=${view}${pair ? `&mode=${even ? "even" : "odd"}` : ""}` +
    `&exc=${exc}&editor=hide&theme=${theme}&title=${encodeURIComponent(title)}&n=${++FV.n}` };
}

// the field window: its first script says it is there and which build ("kisi-advanced"), then the hand-over's steps
window.addEventListener("message", (e) => {
  const d = e.data;
  if (!FV.win || e.source !== FV.win || !d) return;
  if (d.type === "kisi-advanced") FV.waiters.splice(0).forEach((w) => w(d));
  else if (d.type === "field-handover" && d.phase === "shown") fvStatus(`Shown in the field window, solved in ${fmt(d.solveMs / 1000, 1)} s${FV.note}.`);
  else if (d.type === "field-handover" && d.phase === "error") fvStatus(`The field page reports: ${esc(d.error)}`, true);
  else if (d.type === "field-handover" && d.phase === "solving") fvStatus(`Solving in ${esc(FV.file)}…`);
});
const fvHello = (ms) => new Promise((res) => {
  const w = (d) => { clearTimeout(t); res(d); }, t = setTimeout(() => { FV.waiters = FV.waiters.filter((x) => x !== w); res(null); }, ms);
  FV.waiters.push(w);
});
// the file that answered last time first, then the full page, then the lite one; over http(s) only those there
async function fvConnect(w, reload) {
  for (const name of new Set([FV.file, ...FV_FILES].filter((n) => FV_FILES.includes(n)))) {
    const url = new URL(name, location.href).href.split("#")[0];
    if (fvWeb() && !(await fetch(url, { method: "HEAD", cache: "no-store" }).then((r) => r.ok, () => false))) continue;
    fvStatus(`Opening ${name}…`);
    const base = reload ? `${url}?kisi=${Date.now()}` : url, hello = fvHello(fvWeb() ? 30000 : FV.probeMs);
    w.location.href = base + "#kisi";
    const d = await hello;
    reload = false;
    if (d) {
      Object.assign(FV, { base, build: d.build, file: name });
      fvSave();
      return true;
    }
  }
  return false;
}
async function fvOpen(net, xi) {
  // the window inside the click (pop-up blockers): a new one is about:blank, one of an earlier session shows a page
  let w = FV.win && !FV.win.closed ? FV.win : null, fresh = false;
  if (!w) {
    if (!(w = window.open("", "kisi-advanced"))) return fvStatus("The browser blocked the new window – allow pop-ups for this page.", true);
    try { fresh = w.location.href === "about:blank"; } catch (e) { fresh = false; }
    Object.assign(FV, { win: w, base: null, build: null });
  }
  if (!FV.base) {
    if (!(await fvConnect(w, !fresh))) {
      w.close();
      FV.win = null;
      return fvStatus(`No field page next to this file: put <b>kisi-advanced.html</b> (every view, 9 MB) or <b>kisi-advanced-lite.html</b> (potential and |E|, 3 MB) from <a href="https://github.com/riehlies/kisi/releases/latest" target="_blank" rel="noopener">the release</a> into its folder.`, true);
    }
    if (S.selNet >= 0) selectNet(S.selNet, false);   // the view list marks what the lite page lacks
  }
  const h = await fvHandover(net, xi, FV.build === "lite");
  fvStatus(`Handing over at ${h.at}${FV.note}…`);
  w.location.href = FV.base + h.frag;
  w.focus();
}

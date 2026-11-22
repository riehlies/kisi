"use strict";
// ======== field views (option "Advanced solvers"): "Fields…" opens kisi-advanced.html next to this file in a reused
// window – 2D: a segment's cross-section by the fork's link hand-over (URL fragment, kisi's adaptive mesh); 3D: a via
// cut out of the board (kisi_via_case) by postMessage. docs/reference.md "Field views"; data/field_views.json.
const FV_FILE = "kisi-advanced.html";
const FVD = DATA.field_views;
// on by default; a stored choice (turned off) is kept
const FV = Object.assign({ on: true }, lsGet("kisi.fields.v1") || {},
  { win: null, base: null, build: null, n: 0, msg: "", msgNet: -1, err: false, probeMs: 6000, waiters: [], seq3: 0 });
const fvSave = () => lsSet("kisi.fields.v1", { on: FV.on });
const fvOn = () => FV.on && (S.tab === "si" || S.tab === "ch") && S.an && !S.an.skeleton;
const fvWeb = () => /^https?:$/.test(location.protocol);
$("fv-on").checked = FV.on;
$("fv-on").onchange = (e) => { FV.on = e.target.checked; fvSave(); if (S.selNet >= 0) selectNet(S.selNet, false); };

function fvXsLabel(xs) {
  return `${S.scene.stackup.copper[xs.layer].name} · ${KIND_LABEL[xs.kind].split(",")[0]} · w ${fmt(xs.w * 1000, 0)} µm` +
    (xs.diff_gap != null ? ` / gap ${fmt(xs.diff_gap * 1000, 0)} µm, pair` : "");
}
const fvTip = (i) => fvOn() && S.an.cross_sections[S.pieces.xs[i]].geom != null ? `<div class="muted">Click: “Fields…” in ${S.tab === "si" ? "the net's Impedance tab" : "the channel's path"} shows its fields</div>` : "";
// the field-view column of a table (Impedance tab, Vias tab, a channel's path): a cross-section's (2D) or a via's (3D)
// "Fields…", the column head and the status line under the table (of the table's net)
const fv2Cell = (xi, also = "") => !fvOn() ? "" : xi != null && S.an.cross_sections[xi].geom != null
  ? `<td><button class="btn small" data-fv2="${xi}" title="This cross-section in 2D (js_2d_fields)${also}">Fields…</button></td>` : "<td></td>";
const fv3Cell = (i) => fvOn() ? `<td><button class="btn small" data-fv3="${i}" title="This via in 3D (kisi-fdtd)">Fields…</button></td>` : "";
const fvHead = (h) => fvOn() ? `<th>${h}</th>` : "";
const fvMsg = (net) => fvOn() ? `<p class="note fv-msg${FV.err ? " err" : ""}" id="fv-msg" data-net="${net}"${FV.msg && FV.msgNet === net ? "" : " hidden"}>${FV.msgNet === net ? FV.msg : ""}</p>` : "";
function fvWire(root, net, infos = []) {
  root.querySelectorAll("button[data-fv2]").forEach((b) => b.onclick = (e) => { e.stopPropagation(); fvOpen(net, +b.dataset.fv2); });
  root.querySelectorAll("button[data-fv3]").forEach((b) => b.onclick = (e) => { e.stopPropagation(); fv3Open(net, infos[+b.dataset.fv3]); });
}
function fvStatus(html, err = false) {
  Object.assign(FV, { msg: html, err });
  const el = $("fv-msg");
  if (el && +el.dataset.net === FV.msgNet) { el.innerHTML = html; el.hidden = !html; el.classList.toggle("err", err); }
}

// the Nyquist frequency of the net's profile, else the fallback
const fvHz = (g) => g >= 1 ? `${fmt(g, g % 1 ? 2 : 0)} GHz` : `${fmt(g * 1000, 0)} MHz`;
function fvFreq(net) {
  const id = profileIdOf(net), p = id && profile(id);
  return p && p.rate_gbps > 0 ? { ghz: p.rate_gbps / 2, why: `Nyquist of ${p.name || id}` } : { ghz: FVD.frequency.fallback_ghz, why: "no interface profile" };
}
const fvTheme = () => { const t = document.documentElement.dataset.theme; return t || (matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light"); };

async function fvB64(text) {
  const b = new Uint8Array(await new Response(new Blob([text]).stream().pipeThrough(new CompressionStream("deflate-raw"))).arrayBuffer());
  let s = "";
  for (let i = 0; i < b.length; i += 0x8000) s += String.fromCharCode.apply(null, b.subarray(i, i + 0x8000));
  return "z." + btoa(s).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}
async function fvHandover(net, xi) {
  const xs = S.an.cross_sections[xi], pair = xs.diff_gap != null, even = pair && S.pairMode === "c", ex = FVD.excitation, ad = FVD.adaptive;
  const { ghz, why } = fvFreq(net);
  const rough = $("rf-rough"), rq = FVD.copper.rq_um[rough.value] || 0;
  // kisi's roughness preset as the page's RMS roughness (mm)
  const text = rq > 0 && /^units mm$/m.test(S.an.geometries[xs.geom].text)
    ? S.an.geometries[xs.geom].text.replace(/^((?:gnd|sig[+-]?)\s.*?)\s*$/gm, `$1 rq=${+(rq / 1000).toPrecision(4)}`) : S.an.geometries[xs.geom].text;
  const qs = FV.build !== "full", act = fabOn() && stkActiveLabel();
  // kisi's adaptive mesh: FIELD_TOL as energy and parameter tolerance, initial grid, node cap, passes
  const set = { tl_type: "custom", custom_geom: text, mesh_backend: qs ? "rectilinear" : "fullwave_mqs", freq_start: ghz, freq_stop: ghz, freq_points: 1,
    custom_sigma: FVD.copper.sigma_s_m, tolerance: FIELD_TOL, param_tolerance: FIELD_TOL, initial_grid: ad.initial_grid, max_nodes: ad.max_nodes / 1000,
    min_converged_passes: ad.min_converged_passes, estimate_error: ad.estimate_error ? 1 : 0 };
  const title = `${S.scene.nets[net]} · ${fvXsLabel(xs)} · ${fvHz(ghz)} (${why}) · ` +
    (!pair ? `${ex.single_v} V` : even ? `common mode ${ex.pair_even_vcm} V` : `${ex.pair_odd_vdiff} V differential`) +
    ` · copper ${rough.selectedOptions[0].textContent}${rq > 0 ? ` (Rq ${fmt(rq, 2)} µm, estimate)` : ""}${act ? ` · stackup ${act} (what-if)` : ""} · kisi's adaptive mesh · kisi, ${S.name}`;
  const exc = !pair ? `V:${ex.single_v}:peak` : even ? `V:${ex.pair_even_vcm}:peak` : `Vd:${ex.pair_odd_vdiff}:peak`;
  return { at: `${fvHz(ghz)} (${esc(why)})`, frag: `#params=${await fvB64(JSON.stringify(set))}&view=${qs ? "efield" : "sfield"}${pair ? `&mode=${even ? "even" : "odd"}` : ""}` +
    `&exc=${exc}&editor=hide&theme=${fvTheme()}&title=${encodeURIComponent(title)}&n=${++FV.n}` };
}

// the field page's hello ("kisi-advanced", its build) and the steps of each hand-over
window.addEventListener("message", (e) => {
  const d = e.data;
  if (!FV.win || e.source !== FV.win || !d) return;
  if (d.type === "kisi-advanced") FV.waiters.splice(0).forEach((w) => w(d));
  else if (d.type === "field-handover" && d.phase === "shown") fvStatus(`Shown in the field window, solved in ${fmt(d.solveMs / 1000, 1)} s.`);
  else if (d.type === "field-handover" && d.phase === "error") fvStatus(`The field page reports: ${esc(d.error)}`, true);
  else if (d.type === "field-handover" && d.phase === "solving") fvStatus(`Solving in ${FV_FILE}…`);
  else if (d.type === "field-handover" && d.phase === "ready") fvStatus("Loaded in the field window: check the settings there, then Solve.");
  else if (d.type === "field3d" && d.seq === FV.seq3) {
    if (d.phase === "received") fvStatus("Loaded in the field window: check the settings there, then Run.");
    else if (d.phase === "running") fvStatus(`Running kisi-fdtd in the field window (${d.ports} run${d.ports > 1 ? "s" : ""})…`);
    else if (d.phase === "shown") fvStatus(`Shown in the field window, computed in ${fmt(d.seconds, 1)} s.`);
    else if (d.phase === "cancelled") fvStatus("The run was cancelled in the field window.");
    else if (d.phase === "error") fvStatus(`The field page reports: ${esc(d.error)}`, true);
  }
});
const fvHello = (ms) => new Promise((res) => {
  const w = (d) => { clearTimeout(t); res(d); }, t = setTimeout(() => { FV.waiters = FV.waiters.filter((x) => x !== w); res(null); }, ms);
  FV.waiters.push(w);
});
// the page next to this file (over http(s) only when it is there)
async function fvConnect(w, reload, status) {
  const url = new URL(FV_FILE, location.href).href.split("#")[0];
  if (fvWeb() && !(await fetch(url, { method: "HEAD", cache: "no-store" }).then((r) => r.ok, () => false))) return false;
  status(`Opening ${FV_FILE}…`);
  const base = reload ? `${url}?kisi=${Date.now()}` : url, hello = fvHello(fvWeb() ? 30000 : FV.probeMs);
  w.location.href = base + "#kisi";
  const d = await hello;
  if (!d) return false;
  Object.assign(FV, { base, build: d.build });
  return true;
}
const FV_MISSING = `No field page next to this file: put <b>${FV_FILE}</b> (2D cross-section and 3D via views, about 9.5 MB) from <a href="https://github.com/riehlies/kisi/releases/latest" target="_blank" rel="noopener">the release</a> into its folder, or turn the option off (Solver options).`;
// the window, opened in the click (pop-up blockers); a fresh one is about:blank
async function fvWindow(status) {
  let w = FV.win && !FV.win.closed ? FV.win : null, fresh = false;
  if (!w) {
    if (!(w = window.open("", "kisi-advanced"))) { status("The browser blocked the new window – allow pop-ups for this page.", true); return null; }
    try { fresh = w.location.href === "about:blank"; } catch (e) { fresh = false; }
    Object.assign(FV, { win: w, base: null, build: null });
  }
  if (!FV.base && !(await fvConnect(w, !fresh, status))) {
    w.close();
    FV.win = null;
    status(FV_MISSING, true);
    return null;
  }
  return w;
}
async function fvOpen(net, xi) {
  FV.msgNet = net;
  const w = await fvWindow(fvStatus);
  if (!w) return;
  const h = await fvHandover(net, xi);
  fvStatus(`Handing over at ${h.at}…`);
  w.location.href = FV.base + h.frag;
  w.focus();
}

// a via in 3D: kisi's exporter on the board as analysed
async function fv3Open(net0, info) {
  FV.msgNet = net0;
  const sv = S.scene.vias;
  let best = -1, bd = Infinity;
  for (let k = 0; k < sv.length / 7; k++) { const d = Math.hypot(sv[7 * k] - info.at[0], sv[7 * k + 1] - info.at[1]); if (d < bd) { bd = d; best = k; } }
  if (best < 0 || bd > 0.05) return fvStatus("This via is not on the board as loaded.", true);
  const net = sv[7 * best + 6], L = (k) => S.scene.stackup.copper[k].name, fb = FVD.fdtd.board;
  let r;
  try {
    r = S.engine.viaCase({ net: S.scene.nets[net], at: info.at, layers: [L(info.layer_in), L(info.layer_out)], board_file: S.name, with_config: !!S.config,
      keep: fb.keep_mm, feed: fb.feed_mm, margin: fb.margin_mm });
  } catch (e) { return fvStatus(`The via cannot be cut out: ${esc(e.message)}`, true); }
  const c = r.case;
  c.meta.copper_names = S.scene.stackup.copper.map((x) => x.name);
  const { ghz, why } = fvFreq(net), act = fabOn() && stkActiveLabel();
  const title = `${c.meta.title}${act ? ` · stackup ${act} (what-if)` : ""} · kisi, ${S.name}`;
  const from = `Cut out of ${S.name} by kisi's exporter (kisi-validate via-case): ${c.sheets.length} copper pieces, ${c.vias.length} vias, ports ${c.ports.length}; ` +
    `${fvHz(ghz)}: ${why}.${r.notes.filter((n) => !/^board_|sheets \(/.test(n)).map((n) => ` ${n[0].toUpperCase()}${n.slice(1)}${/[.)]$/.test(n) ? "" : "."}`).join("")}`;
  // kisi's model of the via between its lines of the case's feed lengths (rf::via_s), the TDR's rise time, the
  // target and the loss marks: the page's S-parameter and TDR comparison
  const t = classTarget(net), q = pathReq(net);
  let mo = null;
  try { mo = S.engine.rf({ net, rf: rfParams(), target: t ? [t.z, t.diff] : null, flip: q.flip, hint: q.hint, via_at: info.at, via_feeds: r.feeds_mm }); } catch (e) { mo = null; }
  const model = mo && mo.via_model ? Object.assign(mo.via_model, { kind: info.model }) : null;
  const w = await fvWindow(fvStatus);
  if (!w) return;
  FV.seq3 = ++FV.n;
  w.postMessage({ type: "kisi-3d-case", seq: FV.seq3, case: c, title, from, freqs: [{ ghz, label: why }], theme: fvTheme(),
    model, rise: rfParams().rise, target: t, marks: mo && mo.channel ? mo.channel.il_marks.map((x) => x[0]) : null }, "*");
  fvStatus(`Handed over at ${fvHz(ghz)} (${esc(why)})…`);
  w.focus();
}

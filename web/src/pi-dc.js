"use strict";
// ============================================================== supply (DC IR drop)
const PI_PARAMS = { cell: null, plating_um: 25, rho: 1.72e-8, max_cells: 250000 };
const piKey = () => "kisi.pi.v1." + S.name;
function piLoadSaved() { try { const t = localStorage.getItem(piKey()); return t ? JSON.parse(t) : null; } catch (e) { return null; } }
function piSave() {
  if (S.pdnRes) { S.pdnRes.clear(); S.pdnSolved = false; } // loads changed: Z(f) targets are stale
  try {
    if (S.piTree) localStorage.setItem(treeKey(), JSON.stringify(S.piTree));
    else localStorage.setItem(piKey(), JSON.stringify([...S.piSetups.values()]));
  } catch (e) { /* storage blocked */ }
}
const treeKey = () => "kisi.tree.v2." + S.name;
function treeLoadSaved() { try { const t = localStorage.getItem(treeKey()); return t ? JSON.parse(t) : null; } catch (e) { return null; } }

// power tree: solve the chain, derive the IR-drop setups of every rail
function computeTree() {
  try {
    S.treeRes = S.engine.piTree(S.piTree);
    S.piSetups = new Map(S.treeRes.setups.map((r) => [r.net, r]));
    S.piWarn = S.treeRes.warnings || [];
  } catch (e) {
    S.treeRes = null; S.piSetups = new Map(); S.piWarn = ["Power tree: " + e.message];
  }
  computeLosses(); // power flow of the tree (copper losses join after step 2)
}

// ground return of all parts (needs the power tree), then losses and power flow
function solveGround() {
  for (const n of S.piGround.keys()) S.piRes.delete(n);
  S.piGround = new Map(); S.groundWarn = [];
  if (!S.piTree) return;
  try {
    const g = S.engine.piGround();
    S.groundWarn = g.warnings || [];
    for (const s of g.setups) {
      S.piGround.set(s.net, s);
      try { S.piRes.set(s.net, S.engine.piSolve(s, PI_PARAMS)); } catch (e) { S.piRes.set(s.net, { error: e.message }); }
    }
  } catch (e) { S.groundWarn = ["Ground return: " + e.message]; }
}
function computeLosses() {
  try { S.loss = S.engine.piLosses(null); } catch (e) { S.loss = { error: e.message }; }
  S.lossImg = null;
}
const piSetupOf = (net) => S.piSetups.get(net) || (S.piGround && S.piGround.get(net));

function initPi(defaults) {
  S.piRes = new Map(); S.piSetups = new Map(); S.piImg = null; S.piWarn = []; S.piDefaults = defaults;
  S.piGround = new Map(); S.groundWarn = []; S.loss = null; S.lossImg = null; S.lossPlane = "total";
  try { S.engine.piClear(); } catch (e) { /* nothing stored */ }
  S.piTree = null; S.treeRes = null; $("pi-balance").scrollTop = 0;
  S.piSolved = false; // IR drop runs on request (button), it is the slowest part of loading
  S.piMode = "b"; // the power tree is the first view; drop / density after step 2
  S.pdnRes = new Map(); S.pdnSolved = false; // Z(f) per rail, on request
  try { S.piNets = S.engine.piNets(); } catch (e) { S.piNets = []; }
  // regulator chain from PI_REG fields in the board wins over the example tree
  let tf = null;
  try { tf = S.engine.piTreeFields(); } catch (e) { tf = null; }
  if (tf && tf.spec && tf.spec.elements.length) { defaults = tf.spec; S.piDefaults = defaults; }
  if (defaults && !Array.isArray(defaults) && defaults.elements) {
    S.piTree = treeLoadSaved() || JSON.parse(JSON.stringify(defaults));
    computeTree();
    if (tf && tf.warnings) S.piWarn = tf.warnings.concat(S.piWarn);
    $("pi-timing").textContent = "";
    S.piRail = [...S.piSetups.keys()][0] || null; S.piLayer = -1;
    renderPi();
    return;
  }
  const saved = piLoadSaved();
  let fromFields = { rails: [], warnings: [] };
  try { fromFields = S.engine.piFields(); } catch (e) { /* no fields */ }
  S.piWarn = fromFields.warnings || [];
  if (saved) for (const r of saved) S.piSetups.set(r.net, r);
  else {
    for (const r of defaults || []) S.piSetups.set(r.net, r);
    for (const r of fromFields.rails) S.piSetups.set(r.net, r); // fields win over examples
  }
  $("pi-timing").textContent = "";
  S.piRail = [...S.piSetups.keys()][0] || (S.piNets.find((n) => !n.ground) || {}).name || null;
  S.piLayer = -1;
  renderPi();
}

// Z(f) of all rails with a source (step 3)
// inline progress of a power step (bar and "i of n rails" below its Compute button)
const frame = () => new Promise((res) => requestAnimationFrame(() => setTimeout(res, 0)));
function stepProgress(key, i, n) {
  const run = i != null;
  $(key + "-barbox").hidden = !run; $(key + "-prog").hidden = !run;
  S[key + "Running"] = run;
  $(key === "pi" ? "pi-run" : "pdn-run").disabled = run;
  if (run) { $(key + "-bar").style.width = Math.round(100 * i / n) + "%"; $(key + "-prog").textContent = `${i} of ${n} rails`; }
}

async function solveAllPdn() {
  const rails = [...S.piSetups.values()].filter((r) => r.sources.length);
  const t0 = performance.now();
  S.pdnRes = new Map();
  for (let i = 0; i < rails.length; i++) {
    stepProgress("pdn", i, rails.length);
    await frame();
    try { S.pdnRes.set(rails[i].net, S.engine.pdn({ setup: rails[i], params: {} })); } catch (e) { S.pdnRes.set(rails[i].net, { error: e.message }); }
  }
  stepProgress("pdn", null);
  S.pdnSolved = true; S.pdnMs = Math.round(performance.now() - t0);
  if (S.piRail && S.pdnRes.has(S.piRail)) S.piTab = "z";
  renderPi();
}
$("pdn-run").onclick = () => solveAllPdn();

// IR drop of all configured rails (step 2)
async function solveAllRails() {
  const rails = [...S.piSetups.values()];
  const t0 = performance.now(), n = rails.length + (S.piTree ? 1 : 0);
  try { S.engine.piClear(); } catch (e) { /* nothing stored */ }
  for (let i = 0; i < rails.length; i++) {
    stepProgress("pi", i, n);
    await frame();
    try { solvePiRail(rails[i].net, false, true, false); } catch (e) { stepProgress("pi", null); S.piWarn = [e.message]; renderPi(); return; }
  }
  if (S.piTree) { stepProgress("pi", rails.length, n); await frame(); solveGround(); }
  computeLosses();
  stepProgress("pi", null);
  S.piSolved = true; S.piMs = Math.round(performance.now() - t0);
  $("pi-timing").textContent = "";
  let best = null;
  for (const [net, r] of S.piRes) if (r.res && !r.res.ground && (!best || (r.res.max_drop_mv || 0) > (S.piRes.get(best).res.max_drop_mv || 0))) best = net;
  if (best) S.piRail = best;
  S.piLayer = -1; S.piImg = null; S.piMode = "v"; // show the result
  if (S.tab === "pi") fitRail();
  renderPi();
}

function solvePiRail(net, redraw = true, force = false, losses = true) {
  if (!force && !S.piSolved) { if (redraw) renderPi(); return; } // before step 2: only re-render
  const setup = S.piSetups.get(net);
  S.piImg = null;
  if (!setup || !setup.sources.length || !setup.loads.some((l) => l.current)) { S.piRes.delete(net); if (redraw) renderPi(); return; }
  try {
    const r = S.engine.piSolve(setup, PI_PARAMS);
    S.piRes.set(net, r);
  } catch (e) {
    S.piRes.set(net, { error: e.message });
  }
  if (losses) computeLosses();
  if (redraw) renderPi();
}

function piRailInfo(net) { return S.piNets.find((n) => n.name === net); }
function piSetup(net) {
  if (!S.piSetups.has(net)) {
    const info = piRailInfo(net);
    S.piSetups.set(net, { net, voltage: info && info.voltage != null ? info.voltage : 0, sources: [], loads: [] });
  }
  return S.piSetups.get(net);
}

function renderPi() {
  for (const k of ["v", "j", "l", "b"]) $("pim-" + k).setAttribute("aria-pressed", String(k === S.piMode));
  for (const k of ["v", "j", "l"]) { $("pim-" + k).disabled = !S.piSolved; $("pim-" + k).title = S.piSolved ? "" : "after “Compute” in step 2"; }
  renderPiSide();
  const bal = S.tab === "pi" && S.piMode === "b";
  $("pi-balance").hidden = !bal;
  $("legend").hidden = bal; $("layers").hidden = bal;
  if (S.tab !== "pi") return;
  if (bal) { $("pi-detail").hidden = true; renderBalance(); return; }
  if (S.piMode === "l") { renderLossDetail(); renderLayerChips(); renderLegend(); draw(); return; }
  renderPiDetail();
  renderLayerChips();
  renderLegend();
  draw();
}

function renderPiSide() {
  // KPIs
  let worst = null, maxJ = 0, n = 0;
  for (const [net, r] of S.piRes) {
    if (!r.res) continue;
    n++;
    if (r.res.ground) continue;
    const d = r.res.max_drop_mv;
    if (d != null && (!worst || d > worst.d)) worst = { d, net, pct: d / 10 / Math.abs(r.res.voltage) };
    maxJ = Math.max(maxJ, r.res.max_j);
  }
  $("pk-drop").innerHTML = worst ? `${fmt(worst.d, 1)}<small> mV</small>` : "–";
  const nReady = [...S.piSetups.values()].filter((r) => r.sources.length && r.loads.some((l) => l.current)).length;
  $("pi-run").textContent = S.piSolved ? "Recompute" : "Compute";
  $("pi-run").disabled = !nReady || !!S.piRunning;
  $("ps2").classList.toggle("done", !!S.piSolved);
  $("pi-step").textContent = S.piSolved
    ? `Done in ${S.piMs} ms for ${n} rail${n === 1 ? "" : "s"}. Voltage and current density per layer, drop at every load.` + (worst ? ` Largest drop: ${fmt(worst.d, 1)} mV on ${netLabel(worst.net)}.` : "")
    : nReady ? `DC voltage drop on the copper of ${nReady} rail${nReady === 1 ? "" : "s"}: zones, tracks and vias between sources and loads; a few seconds.`
    : "No rail has both a source and a load yet.";
  const nZ = [...S.piSetups.values()].filter((r) => r.sources.length).length;
  $("pdn-run").textContent = S.pdnSolved ? "Recompute" : "Compute";
  $("pdn-run").disabled = !nZ || !!S.pdnRunning;
  $("ps3").classList.toggle("done", !!S.pdnSolved);
  if (S.pdnSolved) {
    const over = [...S.pdnRes.entries()].filter(([, r]) => r && r.above_target && r.above_target.length).map(([net]) => netShort(net));
    $("pdn-step").textContent = `Done in ${S.pdnMs} ms for ${S.pdnRes.size} rail${S.pdnRes.size === 1 ? "" : "s"}. ` +
      (over.length ? `${over.length} rail${over.length === 1 ? "" : "s"} above the target impedance: ${over.slice(0, 5).join(", ")}${over.length > 5 ? ` and ${over.length - 5} more` : ""}.` : "All rails stay below their target impedance.") + " Select a rail for the curve.";
  } else {
    $("pdn-step").textContent = nZ ? `Impedance of ${nZ} rail${nZ === 1 ? "" : "s"} over frequency: regulator, plane capacitance and decoupling capacitors (ESR/ESL estimated by package), plus a plane model from 1 MHz.`
      : "No rail has a source yet.";
  }
  $("pk-rails").textContent = n;
  $("pk-j").innerHTML = n ? `${fmt(maxJ, 0)}<small> A/mm²</small>` : "–";
  const origins = {};
  for (const r of S.piSetups.values()) for (const l of r.loads) origins[l.origin || "user"] = (origins[l.origin || "user"] || 0) + 1;
  const o = Object.entries(origins).map(([k, v]) => `${v}× ${k === "example" ? "example value (assumption)" : k === "field" ? "KiCad field" : k}`).join(", ");
  if (S.treeRes) {
    const t = S.treeRes, cu = piCopperLoss();
    $("pi-origin").textContent = `Power tree ${t.name}: ${t.rails.length} rails, input ${fmt(t.p_input + cu, 2)} W, of which ${fmt(t.p_losses + cu, 2)} W losses${S.piSolved ? "" : " (copper losses after step 2)"}. Details in the “Power tree” view.`;
  } else $("pi-origin").textContent = S.piSetups.size
    ? `${S.piSetups.size} rail${S.piSetups.size === 1 ? "" : "s"} configured. Loads: ${o || "none"}. Each rail needs a source and a load.`
    : "No loads defined. Set PI_LOAD/PI_SOURCE (or PI_REG) fields in KiCad, load a kipida_config.json or enter them in the rail editor.";
  $("pi-warn").innerHTML = S.piWarn.concat(S.groundWarn || []).map((w) => `<span class="err">${esc(w)}</span>`).join("<br>");
  // rails list
  const rank = (r) => (S.piRes.get(r.name) && S.piRes.get(r.name).res ? 0 : S.piSetups.has(r.name) ? 1 : r.ground ? 3 : 2);
  const known = new Set(S.piNets.map((r) => r.name));
  const extra = [...S.piSetups.keys()].filter((n) => !known.has(n)).map((n) => ({ name: n, voltage: S.piSetups.get(n).voltage, ground: false, components: [] }));
  const rails = S.piNets.concat(extra).sort((a, b) => rank(a) - rank(b));
  $("rail-count").textContent = `${rails.filter((r) => !r.ground).length} rails`;
  $("rails").innerHTML = rails.map((r) => {
    const res = S.piRes.get(r.name), st = S.piSetups.get(r.name);
    let status;
    if (r.ground && res && res.res) status = `return ${fmt(-res.res.total_load, 2)} A · offset ${fmt(res.res.max_drop_mv, 2)} mV · ${mW(res.res.power_loss_w)}`;
    else if (r.ground && res && res.error) status = "ground return: " + res.error;
    else if (r.ground) status = S.piTree ? "ground return after step 2" : "ground";
    else if (res && res.error) status = "error: " + res.error;
    else if (res && res.res) status = `drop ${fmt(res.res.max_drop_mv, 1)} mV · ${fmt(res.res.total_load, 2)} A`;
    else if (st && st.loads.length && !st.sources.length) status = "source missing";
    else if (st && st.loads.length && !S.piSolved) status = `${fmt(st.loads.reduce((a, l) => a + (l.current || 0), 0), 2)} A · drop not computed yet`;
    else status = "no load";
    const v = st && st.voltage ? st.voltage : r.voltage;
    if (r.ground) return `<button class="rail" data-net="${esc(r.name)}" aria-current="${r.name === S.piRail}">
      <span class="rn">${esc(netLabel(r.name))}</span><span class="rv">0 V</span><span class="rs">${esc(status)}</span></button>`;
    return `<button class="rail" data-net="${esc(r.name)}" aria-current="${r.name === S.piRail}">
      <span class="rn">${esc(netLabel(r.name))}</span><span class="rv">${v != null && v !== 0 ? fmt(v, 2) + " V" : "? V"}</span>
      <span class="rs">${esc(status)}</span></button>`;
  }).join("");
  $("rails").querySelectorAll(".rail").forEach((b) => b.onclick = () => { S.piRail = b.dataset.net; S.piLayer = -1; S.piImg = null; fitRail(); renderPi(); renderRailEditor(); });
  renderRailEditor();
}

function renderRailEditor() {
  const net = S.piRail, box = $("rail-edit");
  $("rail-name").textContent = net ? netLabel(net) : "";
  if (!net) { box.innerHTML = `<p class="note">Select a rail in the list.</p>`; return; }
  const gs = S.piGround && S.piGround.get(net);
  if (gs) {
    const rows = gs.loads.slice().sort((a, b) => Math.abs(b.current) - Math.abs(a.current));
    box.innerHTML = `<div class="rail-edit">
      <p class="note">Ground return: every part returns the current it draws from the supply rails minus what it feeds into them (a buck converter takes current from ground). Reference 0 V: ${esc(gs.sources.map((x) => x.refdes).join(", "))} (supply input).</p>
      <div class="re-list"><span>Return currents (+ into ground)</span>${rows.map((l) => `<div class="re-load"><span>${esc(l.refdes)}</span><span class="mono">${fmt(-l.current, 3)} A</span><span></span></div>`).join("")}</div></div>`;
    return;
  }
  if (S.piTree) {
    const st = S.piSetups.get(net);
    const rf = S.treeRes && S.treeRes.rails.find((r) => r.net === net);
    box.innerHTML = `<div class="rail-edit">
      <p class="note">Sources and loads come from the power tree. Change currents in the <button class="btn small" id="re-tobal">power tree</button>.</p>
      ${rf ? `<div class="re-row"><span>Voltage</span><span class="mono">${fmt(rf.voltage, 2)} V · ${fmt(rf.i_total, 3)} A</span></div>` : ""}
      ${st ? `<div class="re-row"><span>Source</span><span class="mono">${esc(st.sources.map((x) => x.refdes).join(", "))}${rf && rf.fed_by ? " (" + esc(rf.fed_by) + ")" : ""}</span></div>
      <div class="re-list"><span>Loads on the net</span>${st.loads.map((l) => `<div class="re-load"><span>${esc(l.refdes)} <span class="o">${esc(l.origin)}</span></span><span class="mono">${fmt(l.current, 3)} A</span><span></span></div>`).join("")}</div>` : `<p class="note">No element feeds this rail.</p>`}
    </div>`;
    const b = $("re-tobal"); if (b) b.onclick = () => setPiMode("b");
    return;
  }
  const info = piRailInfo(net), st = piSetup(net);
  const comps = (info ? info.components : []).slice().sort((a, b) => a.localeCompare(b, "en", { numeric: true }));
  const srcSet = new Set(st.sources.map((x) => x.refdes)), loadSet = new Set(st.loads.map((x) => x.refdes));
  const opt = (list) => list.map((c) => `<option value="${esc(c)}">${esc(c)}</option>`).join("");
  box.innerHTML = `<div class="rail-edit">
    <div class="re-row"><label for="re-v">Nominal voltage</label><input id="re-v" type="number" step="0.01" value="${st.voltage || ""}" placeholder="V"></div>
    <div class="re-row"><span>Sources</span><div class="chips">${st.sources.map((x, i) => `<span class="chip">${esc(x.refdes)}<button class="xbtn" data-rs="${i}" aria-label="Remove source ${esc(x.refdes)}">×</button></span>`).join("")}
      <select id="re-addsrc" aria-label="Add source"><option value="">+ source</option>${opt(comps.filter((c) => !srcSet.has(c)))}</select></div></div>
    <div class="re-list"><span>Loads (current in A)</span>
      ${st.loads.map((l, i) => `<div class="re-load"><span>${esc(l.refdes)} <span class="o">${esc(l.origin === "example" ? "assumption" : l.origin || "")}</span></span>
        <input type="number" step="0.001" min="0" value="${l.current}" data-li="${i}" aria-label="Current ${esc(l.refdes)} in A"><button class="xbtn" data-rl="${i}" aria-label="Remove load ${esc(l.refdes)}">×</button></div>`).join("")}
      <select id="re-addload" aria-label="Add load"><option value="">+ load</option>${opt(comps.filter((c) => !loadSet.has(c)))}</select>
    </div>
    <div class="re-actions"><button class="btn small primary" id="re-all">Recompute all rails</button><button class="btn small" id="re-reset">Reset inputs</button></div>
    <details><summary>As KiCad fields</summary><pre class="fields" id="re-fields">${esc(fieldsText())}</pre><button class="btn small" id="re-copy">Copy</button></details>
  </div>`;
  const changed = () => { piSave(); solvePiRail(net); };
  $("re-v").onchange = (e) => { st.voltage = parseFloat(e.target.value) || 0; changed(); };
  $("re-addsrc").onchange = (e) => { if (e.target.value) { st.sources.push({ refdes: e.target.value, pads: [] }); changed(); } };
  $("re-addload").onchange = (e) => { if (e.target.value) { st.loads.push({ refdes: e.target.value, current: 0.1, pads: [], origin: "user" }); changed(); } };
  box.querySelectorAll("[data-rs]").forEach((b) => b.onclick = () => { st.sources.splice(+b.dataset.rs, 1); changed(); });
  box.querySelectorAll("[data-rl]").forEach((b) => b.onclick = () => { st.loads.splice(+b.dataset.rl, 1); changed(); });
  box.querySelectorAll("[data-li]").forEach((inp) => inp.onchange = () => { const l = st.loads[+inp.dataset.li]; l.current = Math.max(0, parseFloat(inp.value) || 0); l.origin = "user"; changed(); });
  $("re-all").onclick = () => solveAllRails();
  $("re-reset").onclick = () => { try { localStorage.removeItem(piKey()); } catch (e) { /* ignore */ } initPi(S.piDefaults); };
  $("re-copy").onclick = (e) => {
    const t = $("re-fields").textContent;
    navigator.clipboard.writeText(t).then(() => { e.target.textContent = "Copied"; }).catch(() => {
      const r = document.createRange(); r.selectNodeContents($("re-fields")); const sel = getSelection(); sel.removeAllRanges(); sel.addRange(r);
    });
  };
}

// current configuration as KiCad field values, per component
function fieldsText() {
  if (S.piTree) return treeFieldsText(S.piTree);
  const by = new Map();
  const add = (ref, key, val) => { if (!by.has(ref)) by.set(ref, { PI_LOAD: [], PI_SOURCE: [] }); by.get(ref)[key].push(val); };
  for (const r of S.piSetups.values()) {
    for (const s of r.sources) add(s.refdes, "PI_SOURCE", `${netLabel(r.net)}=${r.voltage}V`);
    for (const l of r.loads) if (l.current) add(l.refdes, "PI_LOAD", `${netLabel(r.net)}=${+l.current.toFixed(4)}A`);
  }
  const lines = [];
  for (const [ref, f] of [...by.entries()].sort((a, b) => a[0].localeCompare(b[0], "en", { numeric: true }))) {
    if (f.PI_SOURCE.length) lines.push(`${ref}  PI_SOURCE = ${f.PI_SOURCE.join(", ")}`);
    if (f.PI_LOAD.length) lines.push(`${ref}  PI_LOAD = ${f.PI_LOAD.join(", ")}`);
  }
  return lines.join("\n") || "No sources or loads yet.";
}

// power tree as KiCad fields: PI_REG on regulators / inputs / pass elements, PI_LOAD on loads
function treeFieldsText(t) {
  const by = new Map(), add = (ref, key, val) => { if (!by.has(ref)) by.set(ref, { PI_REG: [], PI_LOAD: [] }); by.get(ref)[key].push(val); };
  const volt = new Map(t.rails.map((r) => [r.net, r.voltage]));
  const rn = (n) => {
    const v = volt.get(n), named = /^[+-]?[0-9]+V[0-9]*$/i.test(netShort(n));
    return named || v == null ? netLabel(n) : `${netLabel(n)}@${+v.toFixed(3)}V`;
  };
  for (const e of t.elements) {
    const parts = [e.from ? `${rn(e.from)} > ${rn(e.to)}` : `${rn(e.to)}`, e.kind];
    if (e.eff != null) parts.push(`${+(e.eff * 100).toFixed(1)}%`);
    if (e.imax != null) parts.push(`${+e.imax.toFixed(3)}A`);
    if (e.iq) parts.push(`iq=${+(e.iq * 1e3).toFixed(3)}mA`);
    if (e.r) parts.push(`r=${+e.r.toFixed(4)}`);
    if (e.in_ref) parts.push(`in=${e.in_ref}`);
    if (e.out_ref) parts.push(`out=${e.out_ref}`);
    if (e.id && e.id !== e.refdes) parts.push(`id=${e.id.replace(/[\s;]+/g, "_")}`);
    add(e.refdes, "PI_REG", parts.join(" "));
  }
  for (const l of t.loads) if (l.current) add(l.refdes, "PI_LOAD", `${netLabel(l.net)}=${+l.current.toFixed(4)}A${l.at ? "@" + l.at.split(",").map((x) => x.trim()).join("/") : ""}`);
  const lines = [];
  for (const [ref, f] of [...by.entries()].sort((a, b) => a[0].localeCompare(b[0], "en", { numeric: true }))) {
    if (f.PI_REG.length) lines.push(`${ref}  PI_REG = ${f.PI_REG.join("; ")}`);
    if (f.PI_LOAD.length) lines.push(`${ref}  PI_LOAD = ${f.PI_LOAD.join(", ")}`);
  }
  return lines.join("\n");
}

function importKipida(text) {
  try {
    const r = S.engine.piKipida(text);
    for (const rail of r.rails) S.piSetups.set(rail.net, rail);
    S.piWarn = r.warnings || [];
    piSave();
    for (const rail of r.rails) solvePiRail(rail.net, false);
    if (r.rails.length) S.piRail = r.rails[0].net;
    setTab("pi");
    renderPi();
  } catch (e) {
    S.piWarn = ["KiPIDA file: " + e.message];
    renderPi();
  }
}

function fitRail() {
  const r = piCurrent();
  if (!r) return;
  const g = r.res.layers[0], c = r.res.cell;
  fitView([g.x0, g.y0, g.x0 + g.nx * c, g.y0 + g.ny * c]);
}
function piCurrent() { const r = S.piRes.get(S.piRail); return r && r.res ? r : null; }
function piLayerIdx(r) {
  const ls = r.res.layers;
  if (S.piLayer >= 0) { const i = ls.findIndex((g) => g.layer === S.piLayer); if (i >= 0) return i; }
  // default: layer with most copper cells
  let best = 0, bestN = -1;
  ls.forEach((g, i) => { let c = 0; for (let k = g.offset; k < g.offset + g.nx * g.ny; k++) if (!isNaN(r.v[k])) c++; if (c > bestN) { bestN = c; best = i; } });
  S.piLayer = ls[best].layer;
  return best;
}
function piScale(r) {
  if (S.piMode === "v" && r.res.ground) {
    let mx = 0;
    for (let k = 0; k < r.v.length; k++) { const a = Math.abs(r.v[k]); if (a > mx) mx = a; }
    return { max: Math.max(mx * 1e3, 0.001), unit: "mV", val: (k) => Math.abs(r.v[k]) * 1e3 };
  }
  if (S.piMode === "v") {
    const v0 = Math.abs(r.res.voltage);
    let mn = Infinity;
    for (let k = 0; k < r.v.length; k++) { const a = Math.abs(r.v[k]); if (a < mn) mn = a; }
    return { max: Math.max((v0 - mn) * 1e3, 0.01), unit: "mV", val: (k) => (v0 - Math.abs(r.v[k])) * 1e3 };
  }
  const js = [];
  for (let k = 0; k < r.j.length; k++) if (!isNaN(r.j[k])) js.push(r.j[k]);
  js.sort((a, b) => a - b);
  const p99 = js.length ? js[Math.floor(js.length * 0.99)] : 1;
  return { max: Math.max(p99, 1e-3), unit: "A/mm²", val: (k) => r.j[k] };
}
function piImage(r, li) {
  const key = `${S.piRail}|${li}|${S.piMode}`;
  if (S.piImg && S.piImg.key === key) return S.piImg;
  const g = r.res.layers[li], sc = piScale(r);
  const c = document.createElement("canvas"); c.width = g.nx; c.height = g.ny;
  const cx = c.getContext("2d"), img = cx.createImageData(g.nx, g.ny);
  for (let j = 0; j < g.ny; j++) for (let i = 0; i < g.nx; i++) {
    const k = g.offset + j * g.nx + i, o = (j * g.nx + i) * 4;
    if (isNaN(r.v[k])) { img.data[o + 3] = 0; continue; }
    const col = turboRGB(0.05 + 0.9 * Math.min(1, sc.val(k) / sc.max));
    img.data[o] = col[0]; img.data[o + 1] = col[1]; img.data[o + 2] = col[2]; img.data[o + 3] = 235;
  }
  cx.putImageData(img, 0, 0);
  S.piImg = { key, canvas: c, g, scale: sc };
  return S.piImg;
}

function drawPi() {
  const d = window.devicePixelRatio || 1, C = S.colors;
  ctx.setTransform(1, 0, 0, 1, 0, 0);
  ctx.fillStyle = C.canvas; ctx.fillRect(0, 0, cv.width, cv.height);
  if (!S.scene || !S.static) return;
  const v = S.view, n = S.scene.stackup.copper.length;
  ctx.setTransform(v.s * d, 0, 0, v.s * d, v.x * d, v.y * d);
  const px = 1 / v.s;
  ctx.strokeStyle = C.edge; ctx.lineWidth = 1.5 * px; ctx.stroke(S.static.outline);
  ctx.globalAlpha = 0.25;
  for (let l = n - 1; l >= 0; l--) { ctx.fillStyle = C.pad; ctx.fill(S.static.pads[l]); }
  ctx.globalAlpha = 1;
  if (S.piMode === "l") { drawLoss(px); return; }
  const r = piCurrent();
  if (r) {
    const li = piLayerIdx(r), im = piImage(r, li), g = im.g, cell = r.res.cell;
    ctx.imageSmoothingEnabled = false;
    ctx.drawImage(im.canvas, g.x0, g.y0, g.nx * cell, g.ny * cell);
    // sources and loads
    const net = S.scene.nets.indexOf(S.piRail), pads = S.scene.pads, setup = piSetupOf(S.piRail);
    const srcs = new Set(setup.sources.map((x) => x.refdes)), lds = new Set(setup.loads.map((x) => x.refdes));
    for (let i = 0; i < pads.length / 8; i++) {
      if (pads[i * 8 + 7] !== net) continue;
      const ref = S.scene.fp_refs[S.scene.pad_fp[i]];
      const x = pads[i * 8], y = pads[i * 8 + 1];
      if (srcs.has(ref)) { marker(x, y, px, "src"); } else if (lds.has(ref)) { marker(x, y, px, "load"); }
    }
    // vias with the highest currents
    const vmax = r.res.vias.length ? r.res.vias[0].current : 0;
    ctx.strokeStyle = C.halo; ctx.lineWidth = 1.5 * px;
    for (const via of r.res.vias.slice(0, 10)) {
      const rad = (3 + 7 * Math.sqrt(via.current / (vmax || 1))) * px;
      ctx.beginPath(); ctx.arc(via.at[0], via.at[1], rad, 0, Math.PI * 2); ctx.stroke();
    }
  } else {
    // nothing solved: show the rail's copper zones on all layers lightly
    ctx.fillStyle = C.zone;
    const net = S.scene.nets.indexOf(S.piRail), idx = S.zones.idx, pts = S.zones.pts;
    ctx.beginPath();
    for (let i = 0; i < idx.length; i += 4) {
      if (idx[i + 1] !== net) continue;
      const st = idx[i + 2], c = idx[i + 3];
      ctx.moveTo(pts[st * 2], pts[st * 2 + 1]);
      for (let k = 1; k < c; k++) ctx.lineTo(pts[(st + k) * 2], pts[(st + k) * 2 + 1]);
      ctx.closePath();
    }
    ctx.fill("evenodd");
  }
  drawPdnOverlay(px);
  drawPiMark(px);
}
function marker(x, y, px, kind) {
  const C = S.colors, s = 5 * px;
  ctx.lineWidth = 1.5 * px; ctx.strokeStyle = C.halo;
  ctx.beginPath();
  if (kind === "src") { ctx.moveTo(x, y - s); ctx.lineTo(x + s, y + s * 0.8); ctx.lineTo(x - s, y + s * 0.8); ctx.closePath(); ctx.fillStyle = C.ok; }
  else { ctx.arc(x, y, s * 0.7, 0, Math.PI * 2); ctx.fillStyle = C.canvas; }
  ctx.fill(); ctx.stroke();
}

function showPiTip(mx, my) {
  if (S.piMode === "l") { showLossTip(mx, my); return; }
  const tip = $("tip"), r = piCurrent();
  if (!r) { tip.hidden = true; return; }
  const li = piLayerIdx(r), g = r.res.layers[li], cell = r.res.cell;
  const x = (mx - S.view.x) / S.view.s, y = (my - S.view.y) / S.view.s;
  const i = Math.floor((x - g.x0) / cell), j = Math.floor((y - g.y0) / cell);
  if (i < 0 || j < 0 || i >= g.nx || j >= g.ny) { tip.hidden = true; return; }
  const k = g.offset + j * g.nx + i;
  if (isNaN(r.v[k])) { tip.hidden = true; return; }
  const L = S.scene.stackup.copper;
  const drop = (Math.abs(r.res.voltage) - Math.abs(r.v[k])) * 1e3;
  tip.innerHTML = `<div class="t1">${esc(netLabel(S.piRail))}${r.res.ground ? " · ground return" : ""}</div><div class="kv">
    <span>Layer</span><span>${esc(L[g.layer].name)}</span>
    ${r.res.ground ? `<span>Offset</span><span>${fmt(r.v[k] * 1e3, 3)} mV vs. supply input</span>` : `<span>Voltage</span><span>${fmt(r.v[k], 4)} V</span>
    <span>Drop</span><span>${fmt(drop, 2)} mV</span>`}
    <span>Current density</span><span>${fmt(r.j[k], 1)} A/mm²</span></div>`;
  tip.hidden = false;
  const rs = $("stage").getBoundingClientRect(), tw = tip.offsetWidth, th = tip.offsetHeight;
  let tx = mx + 16, ty = my + 16;
  if (tx + tw > rs.width - 8) tx = mx - tw - 16;
  if (ty + th > rs.height - 8) ty = my - th - 16;
  tip.style.left = Math.max(8, tx) + "px"; tip.style.top = Math.max(8, ty) + "px";
}

function renderPiDetail() {
  const box = $("pi-detail"), r = S.piRes.get(S.piRail);
  if (!r || !r.res) {
    if (S.pdnRes && S.pdnRes.has(S.piRail)) {
      // Z(f) computed, DC not (yet): impedance view only
      const st = S.piSetups.get(S.piRail);
      box.innerHTML = `<header><div><h3>${esc(netLabel(S.piRail))} · ${fmt(st ? st.voltage : 0, 2)} V</h3><div class="muted">${r && r.error ? esc(r.error) : "DC voltage drop: step 2"}</div></div><button class="x" id="closePi" aria-label="Close">×</button></header><div id="pdn-body"></div>`;
      box.hidden = false; $("closePi").onclick = () => { box.hidden = true; };
      renderPdn($("pdn-body"), st);
      return;
    }
    box.hidden = !(r && r.error);
    if (r && r.error) box.innerHTML = `<header><h3>${esc(netLabel(S.piRail))}</h3></header><p class="err">${esc(r.error)}</p>`;
    return;
  }
  const x = r.res, v0 = Math.abs(x.voltage);
  if (x.ground) { renderGroundDetail(box, x); return; }
  // ground offset at each part (ground return, step 2): adds to the rail's drop
  const gOff = new Map();
  for (const g of S.piRes.values()) if (g.res && g.res.ground) for (const l of g.res.loads) if (l.v_max != null) gOff.set(l.refdes, l.v_max * 1e3);
  const withG = gOff.size > 0;
  const loads = x.loads.map((l) => {
    const drop = l.v_min != null ? (v0 - Math.abs(l.v_min)) * 1e3 : null;
    const g = gOff.get(l.refdes);
    return `<tr><td>${esc(l.refdes)}</td><td>${fmt(l.current, 3)}</td><td>${l.v_min != null ? fmt(l.v_min, 4) : "–"}</td><td>${drop != null ? fmt(drop, 2) : "not connected"}</td><td>${drop != null ? fmt(drop / 10 / v0, 2) : "–"}</td>${withG ? `<td>${drop != null && g != null ? fmt(drop + g, 2) : "–"}</td>` : ""}</tr>`;
  }).join("");
  const srcs = x.sources.map((s) => `${esc(s.refdes)} ${fmt(s.current, 3)} A`).join(" · ");
  const vias = x.vias.slice(0, 3).map((v) => `${fmt(v.current, 3)} A (drill ${fmt(v.drill, 2)} mm)`).join(" · ");
  const tabs = `<div class="tabs" role="tablist" aria-label="Rail view">${[["dc", "DC"], ["z", "Impedance Z(f)"]].map(([k, n]) => `<button role="tab" data-pt="${k}" aria-selected="${(S.piTab || "dc") === k}">${n}</button>`).join("")}</div>`;
  const head = `<header><div><h3>${esc(netLabel(x.net))} · ${fmt(x.voltage, 2)} V</h3><div class="muted">grid ${fmt(x.cell, 3)} mm · ${x.nodes.toLocaleString("en-US")} nodes</div></div><button class="x" id="closePi" aria-label="Close">×</button></header>${tabs}`;
  const wire = () => {
    box.hidden = false;
    $("closePi").onclick = () => { box.hidden = true; };
    box.querySelectorAll("[data-pt]").forEach((b) => b.onclick = () => { S.piTab = b.dataset.pt; renderPiDetail(); });
  };
  if (S.piTab === "z") { box.innerHTML = head + `<div id="pdn-body"></div>`; wire(); renderPdn($("pdn-body"), S.piSetups.get(S.piRail)); return; }
  box.innerHTML = head + `
    <div class="dstats"><span>Load <b>${fmt(x.total_load, 3)} A</b></span><span>Loss <b>${fmt(x.power_loss_w * 1e3, 1)} mW</b></span><span>max. current density <b>${fmt(x.max_j, 1)} A/mm²</b></span></div>
    <div class="tablewrap"><table class="xs-table"><thead><tr><th>Load</th><th>A</th><th>V min</th><th>drop mV</th><th>%</th>${withG ? `<th title="rail drop plus the ground offset at the part (ground return)">incl. GND mV</th>` : ""}</tr></thead><tbody>${loads}</tbody></table></div>
    <div class="dstats"><span>Sources: ${srcs || "–"}</span></div>
    <div class="dstats"><span>Highest-current vias: ${vias || "–"}</span></div>
    ${x.warnings.length ? `<div>${x.warnings.map((w) => `<div class="warn">${esc(w)}</div>`).join("")}</div>` : ""}`;
  wire();
}

function renderGroundDetail(box, x) {
  const rows = x.loads.slice().sort((a, b) => Math.max(Math.abs(b.v_min || 0), Math.abs(b.v_max || 0)) - Math.max(Math.abs(a.v_min || 0), Math.abs(a.v_max || 0))).map((l) =>
    `<tr><td>${esc(l.refdes)}</td><td>${fmt(-l.current, 3)}</td><td>${l.v_max != null ? fmt(l.v_max * 1e3, 3) : "not connected"}</td><td>${l.v_min != null ? fmt(l.v_min * 1e3, 3) : "–"}</td></tr>`).join("");
  const vias = x.vias.slice(0, 3).map((v) => `${fmt(v.current, 3)} A (drill ${fmt(v.drill, 2)} mm)`).join(" · ");
  box.innerHTML = `<header><div><h3>${esc(netLabel(x.net))} · ground return</h3><div class="muted">grid ${fmt(x.cell, 3)} mm · ${x.nodes.toLocaleString("en-US")} nodes · reference 0 V: ${esc(x.sources.map((s) => s.refdes).join(", "))}</div></div><button class="x" id="closePi" aria-label="Close">×</button></header>
    <div class="dstats"><span>Return current <b>${fmt(-x.total_load, 3)} A</b></span><span>Loss <b>${fmt(x.power_loss_w * 1e3, 2)} mW</b></span><span>largest offset <b>${fmt(x.max_drop_mv, 3)} mV</b></span><span>max. current density <b>${fmt(x.max_j, 1)} A/mm²</b></span></div>
    <div class="tablewrap"><table class="xs-table"><thead><tr><th>Part</th><th>return A</th><th>offset max mV</th><th>offset min mV</th></tr></thead><tbody>${rows}</tbody></table></div>
    <div class="dstats"><span>Highest-current vias: ${vias || "–"}</span></div>
    <p class="note">Every part returns the current it draws from the supply rails minus what it feeds into them; a negative return (buck converters) takes current from ground. The offset is the ground potential at the part relative to the supply input; it adds to the drop of the part's supply rails.</p>
    ${x.warnings.length ? `<div>${x.warnings.map((w) => `<div class="warn">${esc(w)}</div>`).join("")}</div>` : ""}`;
  box.hidden = false;
  $("closePi").onclick = () => { box.hidden = true; };
}

$("pi-run").onclick = () => solveAllRails();

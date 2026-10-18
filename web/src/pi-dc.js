"use strict";
// ============================================================== supply (DC IR drop)
const PI_PARAMS = { cell: null, plating_um: 25, rho: 1.72e-8, max_cells: 250000 };
const piKey = () => "kisi.pi.v1." + S.name;
function piLoadSaved() { try { const t = localStorage.getItem(piKey()); return t ? JSON.parse(t) : null; } catch (e) { return null; } }
function piSave() {
  try {
    if (S.piTree) localStorage.setItem(treeKey(), JSON.stringify(S.piTree));
    else localStorage.setItem(piKey(), JSON.stringify([...S.piSetups.values()]));
  } catch (e) { /* storage blocked */ }
}
const treeKey = () => "kisi.tree.v1." + S.name;
function treeLoadSaved() { try { const t = localStorage.getItem(treeKey()); return t ? JSON.parse(t) : null; } catch (e) { return null; } }

// power tree: solve the chain, derive the IR-drop setups of every rail
function computeTree() {
  try {
    S.treeRes = S.engine.piTree(S.piTree);
    S.piSetups = new Map(S.treeRes.setups.map((r) => [r.net, r]));
    S.piWarn = S.treeRes.warnings || [];
  } catch (e) {
    S.treeRes = null; S.piSetups = new Map(); S.piWarn = ["Versorgungsbaum: " + e.message];
  }
}

function initPi(defaults) {
  S.piRes = new Map(); S.piSetups = new Map(); S.piImg = null; S.piWarn = []; S.piDefaults = defaults;
  S.piTree = null; S.treeRes = null; $("pi-balance").scrollTop = 0;
  try { S.piNets = S.engine.piNets(); } catch (e) { S.piNets = []; }
  // regulator chain from PI_REG fields in the board wins over the example tree
  let tf = null;
  try { tf = S.engine.piTreeFields(); } catch (e) { tf = null; }
  if (tf && tf.spec && tf.spec.elements.length) { defaults = tf.spec; S.piDefaults = defaults; }
  if (defaults && !Array.isArray(defaults) && defaults.elements) {
    S.piTree = treeLoadSaved() || JSON.parse(JSON.stringify(defaults));
    computeTree();
    if (tf && tf.warnings) S.piWarn = tf.warnings.concat(S.piWarn);
    const t0 = performance.now();
    for (const r of S.piSetups.values()) solvePiRail(r.net, false);
    $("pi-timing").textContent = `${S.piSetups.size} Schienen · ${Math.round(performance.now() - t0)} ms`;
    let best = null;
    for (const [net, r] of S.piRes) if (r.res && (!best || (r.res.max_drop_mv || 0) > (S.piRes.get(best).res.max_drop_mv || 0))) best = net;
    S.piRail = best; S.piLayer = -1;
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
  const t0 = performance.now();
  for (const r of S.piSetups.values()) solvePiRail(r.net, false);
  $("pi-timing").textContent = S.piSetups.size ? `${S.piSetups.size} Schienen · ${Math.round(performance.now() - t0)} ms` : "";
  // default rail: worst drop, else largest supply net
  let best = null;
  for (const [net, r] of S.piRes) if (r.res && (!best || (r.res.max_drop_mv || 0) > (S.piRes.get(best).res.max_drop_mv || 0))) best = net;
  S.piRail = best || (S.piNets.find((n) => !n.ground) || {}).name || null;
  S.piLayer = -1;
  renderPi();
}

function solvePiRail(net, redraw = true) {
  const setup = S.piSetups.get(net);
  S.piImg = null;
  if (!setup || !setup.sources.length || !setup.loads.some((l) => l.current)) { S.piRes.delete(net); if (redraw) renderPi(); return; }
  try {
    const r = S.engine.piSolve(setup, PI_PARAMS);
    S.piRes.set(net, r);
  } catch (e) {
    S.piRes.set(net, { error: e.message });
  }
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
  renderPiSide();
  const bal = S.tab === "pi" && S.piMode === "b";
  $("pi-balance").hidden = !bal;
  $("legend").hidden = bal; $("layers").hidden = bal;
  if (S.tab !== "pi") return;
  if (bal) { $("pi-detail").hidden = true; renderBalance(); return; }
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
    const d = r.res.max_drop_mv;
    if (d != null && (!worst || d > worst.d)) worst = { d, net, pct: d / 10 / Math.abs(r.res.voltage) };
    maxJ = Math.max(maxJ, r.res.max_j);
  }
  $("pk-drop").innerHTML = worst ? `${fmt(worst.d, 1)}<small> mV</small>` : "–";
  $("pk-rails").textContent = n;
  $("pk-j").innerHTML = n ? `${fmt(maxJ, 0)}<small> A/mm²</small>` : "–";
  const origins = {};
  for (const r of S.piSetups.values()) for (const l of r.loads) origins[l.origin || "Benutzer"] = (origins[l.origin || "Benutzer"] || 0) + 1;
  const o = Object.entries(origins).map(([k, v]) => `${v}× ${k === "Beispiel" ? "Beispielwert (Annahme)" : k === "Feld" ? "KiCad-Feld" : k}`).join(", ");
  if (S.treeRes) {
    const t = S.treeRes, cu = piCopperLoss();
    $("pi-origin").textContent = `Versorgungsbaum ${t.name}: Eingang ${fmt(t.p_input + cu, 2)} W, davon ${fmt(t.p_losses + cu, 2)} W Verluste.` +
      (worst ? ` Größter Abfall auf ${netLabel(worst.net)} (${fmt(worst.pct, 2)} %).` : "");
  } else $("pi-origin").textContent = worst
    ? `Größter Abfall auf ${netLabel(worst.net)} (${fmt(worst.pct, 2)} %). Lasten: ${o}.`
    : (S.piSetups.size ? `Lasten: ${o || "keine"}. Für eine Berechnung braucht jede Schiene eine Quelle und eine Last.` : "Keine Lasten festgelegt. Felder PI_LOAD/PI_SOURCE in KiCad setzen, eine kipida_config.json laden oder unten eintragen.");
  $("pi-warn").innerHTML = S.piWarn.map((w) => `<span class="err">${esc(w)}</span>`).join("<br>");
  // rails list
  const rank = (r) => (S.piRes.get(r.name) && S.piRes.get(r.name).res ? 0 : S.piSetups.has(r.name) ? 1 : r.ground ? 3 : 2);
  const known = new Set(S.piNets.map((r) => r.name));
  const extra = [...S.piSetups.keys()].filter((n) => !known.has(n)).map((n) => ({ name: n, voltage: S.piSetups.get(n).voltage, ground: false, components: [] }));
  const rails = S.piNets.concat(extra).sort((a, b) => rank(a) - rank(b));
  $("rail-count").textContent = `${rails.filter((r) => !r.ground).length} Schienen`;
  $("rails").innerHTML = rails.map((r) => {
    const res = S.piRes.get(r.name), st = S.piSetups.get(r.name);
    let status;
    if (r.ground) status = "Masse";
    else if (res && res.error) status = "Fehler: " + res.error;
    else if (res && res.res) status = `Abfall ${fmt(res.res.max_drop_mv, 1)} mV · ${fmt(res.res.total_load, 2)} A`;
    else if (st && st.loads.length && !st.sources.length) status = "Quelle fehlt";
    else status = "keine Last";
    const v = st && st.voltage ? st.voltage : r.voltage;
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
  if (!net) { box.innerHTML = `<p class="note">Schiene in der Liste auswählen.</p>`; return; }
  if (S.piTree) {
    const st = S.piSetups.get(net);
    const rf = S.treeRes && S.treeRes.rails.find((r) => r.net === net);
    box.innerHTML = `<div class="rail-edit">
      <p class="note">Quellen und Lasten kommen aus dem Versorgungsbaum. Ströme in der <button class="btn small" id="re-tobal">Leistungsbilanz</button> ändern.</p>
      ${rf ? `<div class="re-row"><span>Spannung</span><span class="mono">${fmt(rf.voltage, 2)} V · ${fmt(rf.i_total, 3)} A</span></div>` : ""}
      ${st ? `<div class="re-row"><span>Quelle</span><span class="mono">${esc(st.sources.map((x) => x.refdes).join(", "))}${rf && rf.fed_by ? " (" + esc(rf.fed_by) + ")" : ""}</span></div>
      <div class="re-list"><span>Lasten am Netz</span>${st.loads.map((l) => `<div class="re-load"><span>${esc(l.refdes)} <span class="o">${esc(l.origin)}</span></span><span class="mono">${fmt(l.current, 3)} A</span><span></span></div>`).join("")}</div>` : `<p class="note">Kein Element speist diese Schiene.</p>`}
    </div>`;
    const b = $("re-tobal"); if (b) b.onclick = () => setPiMode("b");
    return;
  }
  const info = piRailInfo(net), st = piSetup(net);
  const comps = (info ? info.components : []).slice().sort((a, b) => a.localeCompare(b, "de", { numeric: true }));
  const srcSet = new Set(st.sources.map((x) => x.refdes)), loadSet = new Set(st.loads.map((x) => x.refdes));
  const opt = (list) => list.map((c) => `<option value="${esc(c)}">${esc(c)}</option>`).join("");
  box.innerHTML = `<div class="rail-edit">
    <div class="re-row"><label for="re-v">Nennspannung</label><input id="re-v" type="number" step="0.01" value="${st.voltage || ""}" placeholder="V"></div>
    <div class="re-row"><span>Quellen</span><div class="chips">${st.sources.map((x, i) => `<span class="chip">${esc(x.refdes)}<button class="xbtn" data-rs="${i}" aria-label="Quelle ${esc(x.refdes)} entfernen">×</button></span>`).join("")}
      <select id="re-addsrc" aria-label="Quelle hinzufügen"><option value="">+ Quelle</option>${opt(comps.filter((c) => !srcSet.has(c)))}</select></div></div>
    <div class="re-list"><span>Lasten (Strom in A)</span>
      ${st.loads.map((l, i) => `<div class="re-load"><span>${esc(l.refdes)} <span class="o">${esc(l.origin === "Beispiel" ? "Annahme" : l.origin || "")}</span></span>
        <input type="number" step="0.001" min="0" value="${l.current}" data-li="${i}" aria-label="Strom ${esc(l.refdes)} in A"><button class="xbtn" data-rl="${i}" aria-label="Last ${esc(l.refdes)} entfernen">×</button></div>`).join("")}
      <select id="re-addload" aria-label="Last hinzufügen"><option value="">+ Last</option>${opt(comps.filter((c) => !loadSet.has(c)))}</select>
    </div>
    <div class="re-actions"><button class="btn small primary" id="re-all">Alle Schienen neu rechnen</button><button class="btn small" id="re-reset">Eingaben zurücksetzen</button></div>
    <details><summary>Als KiCad-Felder</summary><pre class="fields" id="re-fields">${esc(fieldsText())}</pre><button class="btn small" id="re-copy">Kopieren</button></details>
  </div>`;
  const changed = () => { piSave(); solvePiRail(net); };
  $("re-v").onchange = (e) => { st.voltage = parseFloat(e.target.value) || 0; changed(); };
  $("re-addsrc").onchange = (e) => { if (e.target.value) { st.sources.push({ refdes: e.target.value, pads: [] }); changed(); } };
  $("re-addload").onchange = (e) => { if (e.target.value) { st.loads.push({ refdes: e.target.value, current: 0.1, pads: [], origin: "Benutzer" }); changed(); } };
  box.querySelectorAll("[data-rs]").forEach((b) => b.onclick = () => { st.sources.splice(+b.dataset.rs, 1); changed(); });
  box.querySelectorAll("[data-rl]").forEach((b) => b.onclick = () => { st.loads.splice(+b.dataset.rl, 1); changed(); });
  box.querySelectorAll("[data-li]").forEach((inp) => inp.onchange = () => { const l = st.loads[+inp.dataset.li]; l.current = Math.max(0, parseFloat(inp.value) || 0); l.origin = "Benutzer"; changed(); });
  $("re-all").onclick = () => { for (const r of S.piSetups.values()) solvePiRail(r.net, false); renderPi(); };
  $("re-reset").onclick = () => { try { localStorage.removeItem(piKey()); } catch (e) { /* ignore */ } initPi(S.piDefaults); };
  $("re-copy").onclick = (e) => {
    const t = $("re-fields").textContent;
    navigator.clipboard.writeText(t).then(() => { e.target.textContent = "Kopiert"; }).catch(() => {
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
  for (const [ref, f] of [...by.entries()].sort((a, b) => a[0].localeCompare(b[0], "de", { numeric: true }))) {
    if (f.PI_SOURCE.length) lines.push(`${ref}  PI_SOURCE = ${f.PI_SOURCE.join(", ")}`);
    if (f.PI_LOAD.length) lines.push(`${ref}  PI_LOAD = ${f.PI_LOAD.join(", ")}`);
  }
  return lines.join("\n") || "Noch keine Quellen oder Lasten.";
}

// power tree as KiCad fields: PI_REG on regulators / inputs / pass elements, PI_LOAD on loads
function treeFieldsText(t) {
  const by = new Map(), add = (ref, key, val) => { if (!by.has(ref)) by.set(ref, { PI_REG: [], PI_LOAD: [] }); by.get(ref)[key].push(val); };
  const volt = new Map(t.rails.map((r) => [r.net, r.voltage]));
  const rn = (n) => {
    const v = volt.get(n), named = /^[+-]?[0-9]+V[0-9]*$/i.test(netLabel(n).split("/").pop());
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
  for (const l of t.loads) if (l.current) add(l.refdes, "PI_LOAD", `${netLabel(l.net)}=${+l.current.toFixed(4)}A`);
  const lines = [];
  for (const [ref, f] of [...by.entries()].sort((a, b) => a[0].localeCompare(b[0], "de", { numeric: true }))) {
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
    S.piWarn = ["KiPIDA-Datei: " + e.message];
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
  const r = piCurrent();
  if (r) {
    const li = piLayerIdx(r), im = piImage(r, li), g = im.g, cell = r.res.cell;
    ctx.imageSmoothingEnabled = false;
    ctx.drawImage(im.canvas, g.x0, g.y0, g.nx * cell, g.ny * cell);
    // sources and loads
    const net = S.scene.nets.indexOf(S.piRail), pads = S.scene.pads, setup = S.piSetups.get(S.piRail);
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
  tip.innerHTML = `<div class="t1">${esc(netLabel(S.piRail))}</div><div class="kv">
    <span>Lage</span><span>${esc(L[g.layer].name)}</span>
    <span>Spannung</span><span>${fmt(r.v[k], 4)} V</span>
    <span>Abfall</span><span>${fmt(drop, 2)} mV</span>
    <span>Stromdichte</span><span>${fmt(r.j[k], 1)} A/mm²</span></div>`;
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
    box.hidden = !(r && r.error);
    if (r && r.error) box.innerHTML = `<header><h3>${esc(netLabel(S.piRail))}</h3></header><p class="err">${esc(r.error)}</p>`;
    return;
  }
  const x = r.res, v0 = Math.abs(x.voltage);
  const loads = x.loads.map((l) => {
    const drop = l.v_min != null ? (v0 - Math.abs(l.v_min)) * 1e3 : null;
    return `<tr><td>${esc(l.refdes)}</td><td>${fmt(l.current, 3)}</td><td>${l.v_min != null ? fmt(l.v_min, 4) : "–"}</td><td>${drop != null ? fmt(drop, 2) : "keine Verbindung"}</td><td>${drop != null ? fmt(drop / 10 / v0, 2) : "–"}</td></tr>`;
  }).join("");
  const srcs = x.sources.map((s) => `${esc(s.refdes)} ${fmt(s.current, 3)} A`).join(" · ");
  const vias = x.vias.slice(0, 3).map((v) => `${fmt(v.current, 3)} A (Bohrung ${fmt(v.drill, 2)} mm)`).join(" · ");
  const tabs = `<div class="tabs" role="tablist" aria-label="Ansicht der Schiene">${[["dc", "Gleichstrom"], ["z", "Impedanz Z(f)"]].map(([k, n]) => `<button role="tab" data-pt="${k}" aria-selected="${(S.piTab || "dc") === k}">${n}</button>`).join("")}</div>`;
  const head = `<header><div><h3>${esc(netLabel(x.net))} · ${fmt(x.voltage, 2)} V</h3><div class="muted">Raster ${fmt(x.cell, 3)} mm · ${x.nodes.toLocaleString("de-DE")} Knoten</div></div><button class="x" id="closePi" aria-label="Schließen">×</button></header>${tabs}`;
  const wire = () => {
    box.hidden = false;
    $("closePi").onclick = () => { box.hidden = true; };
    box.querySelectorAll("[data-pt]").forEach((b) => b.onclick = () => { S.piTab = b.dataset.pt; renderPiDetail(); });
  };
  if (S.piTab === "z") { box.innerHTML = head + `<div id="pdn-body"></div>`; wire(); renderPdn($("pdn-body"), S.piSetups.get(S.piRail)); return; }
  box.innerHTML = head + `
    <div class="dstats"><span>Last <b>${fmt(x.total_load, 3)} A</b></span><span>Verlust <b>${fmt(x.power_loss_w * 1e3, 1)} mW</b></span><span>max. Stromdichte <b>${fmt(x.max_j, 1)} A/mm²</b></span></div>
    <div class="tablewrap"><table class="xs-table"><thead><tr><th>Last</th><th>A</th><th>V min</th><th>Abfall mV</th><th>%</th></tr></thead><tbody>${loads}</tbody></table></div>
    <div class="dstats"><span>Quellen: ${srcs || "–"}</span></div>
    <div class="dstats"><span>Stärkste Vias: ${vias || "–"}</span></div>
    ${x.warnings.length ? `<div>${x.warnings.map((w) => `<div class="warn">${esc(w)}</div>`).join("")}</div>` : ""}`;
  wire();
}

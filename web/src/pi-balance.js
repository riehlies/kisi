"use strict";
// ---- power balance view ------------------------------------------------------
function piCopperLoss() { let w = 0; for (const r of S.piRes.values()) if (r.res) w += r.res.power_loss_w; return w; }
const KIND_NAME = { input: "input", buck: "buck", boost: "boost", ldo: "LDO", inverter: "inverter", pass: "pass-through" };
const watts = (w) => fmtSI(w, "W");
const COMP_COLS_KEY = "kisi.componentColumns";
// thermal columns of one part (components table, "Thermal"): Tj, case top, Tj max, the θ values as
// used by the model and where they come from
function thermCells(p) {
  if (!p) return `<td class="n">–</td><td class="n">–</td><td class="n">–</td><td>–</td><td class="src">no heat in step 3</td>`;
  const theta = p.model === "theta_jb" ? [`θJB ${fmt(p.theta_jb, 1)}`].concat(p.theta_jc ? [`θJC ${fmt(p.theta_jc, 1)}`] : [])
    : p.model === "psi_jb" ? [`ψJB ${fmt(p.psi_jb, 1)}`] : [`θJA ${fmt(p.theta_ja, 1)}`];
  if (p.psi_jt != null) theta.push(`ψJT ${fmt(p.psi_jt, 2)}`);
  const field = p.data_source.startsWith("PI_THERMAL");
  const kind = field ? ((p.data_source.match(/^PI_THERMAL \(([^:),]+)/) || [])[1] || "PI_THERMAL") : `${p.family} default`;
  const sub = field ? (p.source_document || (p.data_source.match(/^PI_THERMAL \([^:]+: (.*)\)$/) || [])[1] || "footprint field") + (p.model === "theta_ja" ? " · θJA of the JEDEC board (estimate)" : "")
    : "data/components.json · estimate";
  const tjmDef = p.tj_max_source.startsWith("default");
  const full = `${theta.join(", ")} K/W – ${field ? p.data_source + (p.source_document ? `; ${p.source_document}` : "") : `package default ${p.family} (data/components.json, estimate)`}; Tj max ${fmt(p.tj_max_c, 0)} °C: ${p.tj_max_source}${p.case_model ? `; case top from ${p.case_model === "psi_jt" ? "ψJT" : "the two-resistor model"}` : ""}`;
  return `<td class="n" title="${esc(full)}">${tCell(p.tj_c, p.estimate ? "*" : "")}</td><td class="n">${fmtTn(p.t_case_c)}</td>
    <td class="n" title="${esc(p.tj_max_source)}">${fmt(p.tj_max_c, 0)}${tjmDef ? "*" : ""}</td><td class="th">${theta.map(esc).join("<br>")}</td>
    <td class="src" title="${esc(full)}"><b>${esc(kind)}</b><span>${esc(sub)}</span></td>`;
}

function renderBalance() {
  const box = $("pi-balance"), t = S.treeRes;
  if (!t) {
    box.innerHTML = `<div class="bal-head"><h3>Power tree</h3><p>No power tree is defined for this board. Add PI_REG fields to the regulators in KiCad; the regulator chains of the example boards ThunderScope and LibreVNA are modelled.</p></div>`;
    return;
  }
  const cu = piCopperLoss();
  const pin = t.p_input + cu, loss = t.p_losses + cu;
  const regLoss = t.elements.filter((e) => e.kind !== "pass" && e.kind !== "input").reduce((a, e) => a + e.p_loss, 0);
  const passLoss = t.p_losses - regLoss;
  const kpi = (v, u, l) => `<div class="bal-kpi"><span class="v">${v}<small> ${u}</small></span><span class="l">${l}</span></div>`;
  const kpiW = (w, l) => kpi(...siParts(w, "W"), l);
  // tree, depth-first from the inputs
  const kids = (net) => t.elements.filter((e) => e.from === net);
  const loadsOn = (net) => S.piTree.loads.map((l, i) => ({ l, i })).filter((x) => t.load_nets[x.i] === net);
  const rows = [];
  // a part reference that shows the part on the board (with the rail it feeds / draws from)
  const part = (refdes, rail, bold) => `<button class="t-part${bold ? " b" : ""}" data-part="${esc(refdes)}" data-prail="${esc(rail || "")}" title="Show ${esc(refdes)} on the board">${esc(refdes)}</button>`;
  const walk = (net, d) => {
    const r = t.rails.find((x) => x.net === net);
    if (!r) return;
    const pad = `style="padding-left:${d * 16}px"`;
    rows.push(`<div class="t-rail" ${pad}><button data-rail="${esc(net)}" title="Show voltage drop of this rail">${esc(netShort(net))}</button><span class="t-num">${fmtSI(r.voltage, "V")} · ${fmtSI(r.i_total, "A")} · ${watts(Math.abs(r.voltage) * r.i_total)}</span></div>`);
    for (const { l, i } of loadsOn(net)) {
      rows.push(`<div class="t-load" style="padding-left:${d * 16 + 12}px"><span>${part(l.refdes, net)} <span class="b">${esc(l.part)} · ${esc(l.basis || "")}${l.note ? " · " + esc(l.note) : ""}</span></span>
        <input type="text" inputmode="decimal" class="si-in" value="${esc(fmtSI(l.current, "A", 4))}" data-load="${i}" title="Current, e.g. 12 mA, 12m, 0.012 or 1.2e-5 (A)" aria-label="Current ${esc(l.refdes)} on ${esc(netLabel(net))}"></div>`);
    }
    for (const e of kids(net)) {
      // efficiency of the element (with its quiescent current); kisi's default when PI_REG gives none: an estimate
      const effEst = e.eff_estimate ? ` <span class="muted" title="No efficiency in the PI_REG field: kisi assumes ${fmt(100 * e.eff, 0)} %. Give the datasheet value at the expected load, e.g. “buck 90%”.">(estimate)</span>` : "";
      const eff = e.kind === "pass" ? "" : e.p_in > 0 ? ` · η ${fmt(100 * e.p_out / e.p_in, 0)} %${effEst}` : e.eff_estimate ? ` · η ${fmt(100 * e.eff, 0)} %${effEst}` : "";
      rows.push(`<div class="t-el" style="padding-left:${d * 16 + 12}px"><span>└ ${part(e.refdes, e.to, true)}${e.id !== e.refdes ? ` <span class="b">${esc(e.id)}</span>` : ""} ${esc(KIND_NAME[e.kind] || e.kind)} · ${esc(e.part)}${e.overload ? `<span class="chip-warn">${fmtSI(e.i_out, "A")} &gt; ${fmtSI(e.imax, "A")}</span>` : ""}</span><span class="t-num">loss ${watts(e.p_loss)}${eff}</span></div>`);
      walk(e.to, d + 1);
    }
  };
  for (const e of t.elements.filter((e) => e.kind === "input")) {
    rows.push(`<div class="t-el"><span>${part(e.refdes, e.to, true)} ${esc(e.part)}</span><span class="t-num">${watts(e.p_out)}</span></div>`);
    walk(e.to, 0);
  }
  // components
  const comps = t.components.filter((c) => c.consumption + c.loss >= 0.0005);
  const maxW = Math.max(...comps.map((c) => c.consumption + c.loss), 1e-9);
  const effEst = new Set(t.elements.filter((e) => e.eff_estimate).map((e) => e.refdes));
  const basis = (c) => { const b = [...new Set(c.rails.map((r) => r[2]))]; return c.rails.length ? (b.length > 1 ? "mixed" : b[0]) : effEst.has(c.refdes) ? "estimate (η)" : "computed"; };
  // columns: "Power" (consumption, loss, basis) or, once step 3 has run, "Thermal" (Tj, case top, Tj max,
  // the thermal resistances used and their source); the choice is remembered per browser
  const th = typeof thermOk === "function" && thermOk() ? new Map(S.therm.parts.map((p) => [p.refdes, p])) : null;
  let want = "p";
  try { want = localStorage.getItem(COMP_COLS_KEY) === "t" ? "t" : "p"; } catch (e) { /* no storage */ }
  const thermal = !!th && want === "t";
  const crow = (c) => {
    const el = t.elements.find((e) => e.refdes === c.refdes);
    const rail = c.rails.length ? c.rails[0][0] : el ? el.to : "";
    const head = `<td>${part(c.refdes, rail)}</td><td>${esc(c.part)}</td>`;
    if (thermal) return `<tr>${head}${thermCells(th.get(c.refdes))}</tr>`;
    const wu = 100 * c.consumption / maxW, wl = 100 * c.loss / maxW;
    return `<tr title="${esc(c.refdes)}: consumption ${watts(c.consumption)}, loss ${watts(c.loss)}">${head}
      <td class="n">${c.consumption ? watts(c.consumption) : "–"}</td><td class="n">${c.loss ? watts(c.loss) : "–"}</td>
      <td><div class="bar">${wu > 0 ? `<i class="u" style="width:${wu}%"></i>` : ""}${wl > 0 ? `<i class="l" style="width:${wl}%"></i>` : ""}</div></td><td class="b">${esc(basis(c))}</td></tr>`;
  };
  const colsSw = `<span class="seg cmode" role="group" aria-label="Columns of the components table"><button data-cm="p" aria-pressed="${!thermal}">Power</button><button data-cm="t" aria-pressed="${thermal}"${th ? ` title="junction and case temperature, θ values and their source"` : ` disabled title="available after step 3 (Temperature)"`}>Thermal</button></span>`;
  const thead = thermal
    ? `<th>Component</th><th>Part</th><th class="n" title="junction temperature from step 3 (°C); * = estimated package data">Tj °C</th><th class="n" title="top of the package (°C): Tj − ψJT·P, else the two-resistor model">case top °C</th><th class="n" title="maximum junction temperature (°C); * = assumed default">Tj max °C</th><th title="thermal resistances as used (K/W): θJB junction–board, θJC junction–case top, ψJB/ψJT characterisation parameters, θJA (JEDEC test board only)">θ K/W</th><th title="where the θ values come from: PI_THERMAL with its source document, or the package default of the family (data/components.json, estimate)">source</th>`
    : `<th>Component</th><th>Part</th><th class="n">consumption</th><th class="n">loss</th><th></th><th>Basis</th>`;
  const tnote = thermal
    ? `<p class="note">Temperatures from step 3 (estimate): ${esc(S.therm.cooling.airflow_m_s ? `air flow ${S.therm.cooling.airflow_m_s} m/s` : "still air")}, ambient ${fmtT(S.therm.ambient_c)}. * estimate: package default θ by family and size or an assumed Tj max. Hover a row for all values and sources.</p>`
    : th ? "" : `<p class="note">“Thermal” (junction and case temperature, θ values and their source) after step 3 “Temperature”.</p>`;
  const warn = (t.warnings || []).map((w) => `<div class="warn">${esc(w)}</div>`).join("");
  box.innerHTML = `
    <div class="bal-head"><h3>Power tree · ${esc(t.name)}</h3><p>${esc(t.note)}</p></div>
    <div class="bal-kpis">
      ${kpiW(pin, "input power")}
      ${kpiW(t.p_loads, "consumed by components")}
      ${kpiW(regLoss, "regulator losses")}
      ${kpiW(passLoss + cu, "shunts, ferrites and copper")}
      ${kpi(fmt(100 * t.p_loads / pin, 0), "%", "overall efficiency")}
    </div>
    ${warn}
    ${S.loss && S.loss.flow ? `<div class="bal-card" style="margin-bottom:12px"><h4>Power flow <em>band width ∝ power · hover for values</em></h4>
      <div class="sankey-wrap">${sankeySVG(S.loss.flow)}</div>
      <div class="bal-legend"><span><i style="background:var(--pw-use)"></i>delivered</span><span><i style="background:var(--pw-loss)"></i>losses</span></div>
      <p class="note">Rails behind shunts and ferrites are merged into the rail before them. ${S.piSolved ? "Copper losses of the rails and of the ground return come from step 2." : "Copper losses (rails and ground return) join after step 2."}</p></div>` : ""}
    <div class="bal-cols">
      <div class="bal-card"><h4>Power tree <em>currents are editable: 12 mA, 12m, 0.012</em></h4><div class="tree">${rows.join("")}</div>
        <div class="bal-actions" style="margin-top:8px"><button class="btn small" id="bal-reset">Reset values</button><span class="note">Changes are stored in this browser.</span></div></div>
      <div class="bal-card bal-comp"><h4>Components ${colsSw}<em>${comps.length} with ≥ 0.5 mW</em></h4>
        ${thermal ? "" : `<div class="bal-legend"><span><i style="background:var(--pw-use)"></i>consumption</span><span><i style="background:var(--pw-loss)"></i>loss (regulator, shunt, ferrite)</span></div>`}
        <div class="tablewrap"><table class="ctab${thermal ? " therm" : ""}"><thead><tr>${thead}</tr></thead><tbody>${comps.map(crow).join("")}</tbody></table></div>
        ${tnote}
        <p class="note">${S.piSolved ? `Copper losses of the layers (${watts(cu)}) come from the voltage-drop solution and are not assigned to any component.` : "Copper losses of the layers are not included yet: they come from the voltage-drop solution (“Compute voltage drop”)."}</p></div>
    </div>`;
  box.querySelectorAll("[data-load]").forEach((inp) => inp.onchange = () => {
    // with or without prefix and unit: "12 mA", "12m", "0.012", "1.2e-5" (a plain number is A)
    const v = parseSI(inp.value, "A");
    if (!(v >= 0)) { inp.setAttribute("aria-invalid", "true"); return; }
    const l = S.piTree.loads[+inp.dataset.load];
    l.current = v;
    l.basis = "user";
    piSave(); refreshTree();
  });
  box.querySelectorAll("[data-rail]").forEach((b) => b.onclick = () => { S.piRail = b.dataset.rail; S.piLayer = -1; setPiMode("v"); fitRail(); });
  box.querySelectorAll("[data-part]").forEach((b) => b.onclick = () => showPart(b.dataset.part, b.dataset.prail));
  box.querySelectorAll("[data-cm]").forEach((b) => b.onclick = () => {
    try { localStorage.setItem(COMP_COLS_KEY, b.dataset.cm); } catch (e) { /* not remembered */ }
    renderBalance();
  });
  // power flow: a rail opens the rail on the board, a regulator, input or load shows the part
  box.querySelectorAll(".sankey [data-node]").forEach((g) => {
    const go = () => sankeyJump(g.dataset.node);
    g.onclick = go;
    g.onkeydown = (e) => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); go(); } };
  });
  $("bal-reset").onclick = () => { try { localStorage.removeItem(treeKey()); } catch (e) { /* ignore */ } initPi(S.piDefaults); setPiMode("b"); };
}
function sankeyJump(id) {
  const flow = S.loss && S.loss.flow;
  if (!flow) return;
  const node = flow.nodes.find((n) => n.id === id);
  if (!node) return;
  const railOf = (rid) => rid && rid.startsWith("rail:") ? rid.slice(5) : null;
  const known = (net) => net && S.piSetups.has(net) ? net : null;
  if (node.kind === "rail") {
    S.piMark = null;
    S.piRail = known(railOf(id)) || S.piRail; S.piLayer = -1;
    setPiMode("v"); fitRail();
    return;
  }
  // a part: its reference is the load id or the first word of the label; show it with the rail it
  // feeds (regulator, input) or draws from (load)
  const refdes = node.kind === "load" ? id.slice(5) : node.label.split(" ")[0];
  const link = node.kind === "load" ? flow.links.find((l) => l.to === id) : flow.links.find((l) => l.from === id && l.to.startsWith("rail:"));
  showPart(refdes, railOf(link && (node.kind === "load" ? link.from : link.to)));
}
// show a part outlined on the board, with a rail (the one it feeds or draws from) in the voltage
// drop view; from the power flow, the power tree and the component list
function showPart(refdes, rail) {
  if (rail && S.piSetups.has(rail)) { S.piRail = rail; S.piLayer = -1; }
  const bbox = fpBBox(refdes);
  S.piMark = bbox ? { refdes, bbox, rail: S.piRail } : null;
  setPiMode("v");
  if (bbox) { const c = [(bbox[0] + bbox[2]) / 2, (bbox[1] + bbox[3]) / 2], h = Math.max(6, (bbox[2] - bbox[0]), (bbox[3] - bbox[1])); fitView([c[0] - h, c[1] - h, c[0] + h, c[1] + h]); }
}
function refreshTree() {
  const keep = S.piRail;
  computeTree();
  S.piRes = new Map();
  try { S.engine.piClear(); } catch (e) { /* nothing stored */ }
  for (const r of S.piSetups.values()) solvePiRail(r.net, false, false, false);
  if (S.piSolved) solveGround();
  computeLosses();
  S.piRail = keep;
  renderPi();
}

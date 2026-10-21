"use strict";
// ---- power balance view ------------------------------------------------------
function piCopperLoss() { let w = 0; for (const r of S.piRes.values()) if (r.res) w += r.res.power_loss_w; return w; }
const KIND_NAME = { input: "input", buck: "buck", boost: "boost", ldo: "LDO", inverter: "inverter", pass: "pass-through" };
const mW = (w) => (w >= 1 ? fmt(w, 2) + " W" : fmt(w * 1e3, 0) + " mW");

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
    rows.push(`<div class="t-rail" ${pad}><button data-rail="${esc(net)}" title="Show voltage drop of this rail">${esc(netShort(net))}</button><span class="t-num">${fmt(r.voltage, 2)} V · ${fmt(r.i_total, 3)} A · ${mW(Math.abs(r.voltage) * r.i_total)}</span></div>`);
    for (const { l, i } of loadsOn(net)) {
      rows.push(`<div class="t-load" style="padding-left:${d * 16 + 12}px"><span>${part(l.refdes, net)} <span class="b">${esc(l.part)} · ${esc(l.basis || "")}${l.note ? " · " + esc(l.note) : ""}</span></span>
        <input type="number" min="0" step="0.001" value="${+l.current.toFixed(4)}" data-load="${i}" aria-label="Current ${esc(l.refdes)} on ${esc(netLabel(net))} in A"></div>`);
    }
    for (const e of kids(net)) {
      const eff = e.kind === "pass" ? "" : e.p_in > 0 ? ` · η ${fmt(100 * e.p_out / e.p_in, 0)} %` : "";
      rows.push(`<div class="t-el" style="padding-left:${d * 16 + 12}px"><span>└ ${part(e.refdes, e.to, true)}${e.id !== e.refdes ? ` <span class="b">${esc(e.id)}</span>` : ""} ${esc(KIND_NAME[e.kind] || e.kind)} · ${esc(e.part)}${e.overload ? `<span class="chip-warn">${fmt(e.i_out, 2)} A &gt; ${fmt(e.imax, 2)} A</span>` : ""}</span><span class="t-num">loss ${mW(e.p_loss)}${eff}</span></div>`);
      walk(e.to, d + 1);
    }
  };
  for (const e of t.elements.filter((e) => e.kind === "input")) {
    rows.push(`<div class="t-el"><span>${part(e.refdes, e.to, true)} ${esc(e.part)}</span><span class="t-num">${mW(e.p_out)}</span></div>`);
    walk(e.to, 0);
  }
  // components
  const comps = t.components.filter((c) => c.consumption + c.loss >= 0.0005);
  const maxW = Math.max(...comps.map((c) => c.consumption + c.loss), 1e-9);
  const basis = (c) => { const b = [...new Set(c.rails.map((r) => r[2]))]; return c.rails.length ? (b.length > 1 ? "mixed" : b[0]) : "computed"; };
  const crow = (c) => {
    const wu = 100 * c.consumption / maxW, wl = 100 * c.loss / maxW;
    const el = t.elements.find((e) => e.refdes === c.refdes);
    const rail = c.rails.length ? c.rails[0][0] : el ? el.to : "";
    return `<tr title="${esc(c.refdes)}: consumption ${mW(c.consumption)}, loss ${mW(c.loss)}"><td>${part(c.refdes, rail)}</td><td>${esc(c.part)}</td>
      <td class="n">${c.consumption ? fmt(c.consumption * 1e3, 0) : "–"}</td><td class="n">${c.loss ? fmt(c.loss * 1e3, 0) : "–"}</td>
      <td><div class="bar">${wu > 0 ? `<i class="u" style="width:${wu}%"></i>` : ""}${wl > 0 ? `<i class="l" style="width:${wl}%"></i>` : ""}</div></td><td class="b">${esc(basis(c))}</td></tr>`;
  };
  const warn = (t.warnings || []).map((w) => `<div class="warn">${esc(w)}</div>`).join("");
  box.innerHTML = `
    <div class="bal-head"><h3>Power tree · ${esc(t.name)}</h3><p>${esc(t.note)}</p></div>
    <div class="bal-kpis">
      ${kpi(fmt(pin, 2), "W", "input power")}
      ${kpi(fmt(t.p_loads, 2), "W", "consumed by components")}
      ${kpi(fmt(regLoss, 2), "W", "regulator losses")}
      ${kpi(fmt((passLoss + cu) * 1e3, 0), "mW", "shunts, ferrites and copper")}
      ${kpi(fmt(100 * t.p_loads / pin, 0), "%", "overall efficiency")}
    </div>
    ${warn}
    ${S.loss && S.loss.flow ? `<div class="bal-card" style="margin-bottom:12px"><h4>Power flow <em>band width ∝ power · hover for values</em></h4>
      <div class="sankey-wrap">${sankeySVG(S.loss.flow)}</div>
      <div class="bal-legend"><span><i style="background:var(--pw-use)"></i>delivered</span><span><i style="background:var(--pw-loss)"></i>losses</span></div>
      <p class="note">Rails behind shunts and ferrites are merged into the rail before them. ${S.piSolved ? "Copper losses of the rails and of the ground return come from step 2." : "Copper losses (rails and ground return) join after step 2."}</p></div>` : ""}
    <div class="bal-cols">
      <div class="bal-card"><h4>Power tree <em>currents in A are editable</em></h4><div class="tree">${rows.join("")}</div>
        <div class="bal-actions" style="margin-top:8px"><button class="btn small" id="bal-reset">Reset values</button><span class="note">Changes are stored in this browser.</span></div></div>
      <div class="bal-card bal-comp"><h4>Components <em>${comps.length} with ≥ 0.5 mW</em></h4>
        <div class="bal-legend"><span><i style="background:var(--pw-use)"></i>consumption</span><span><i style="background:var(--pw-loss)"></i>loss (regulator, shunt, ferrite)</span></div>
        <div class="tablewrap"><table class="ctab"><thead><tr><th>Component</th><th>Part</th><th>consumption mW</th><th>loss mW</th><th></th><th>Basis</th></tr></thead><tbody>${comps.map(crow).join("")}</tbody></table></div>
        <p class="note">${S.piSolved ? `Copper losses of the layers (${fmt(cu * 1e3, 0)} mW) come from the voltage-drop solution and are not assigned to any component.` : "Copper losses of the layers are not included yet: they come from the voltage-drop solution (“Compute voltage drop”)."}</p></div>
    </div>`;
  box.querySelectorAll("[data-load]").forEach((inp) => inp.onchange = () => {
    const l = S.piTree.loads[+inp.dataset.load];
    l.current = Math.max(0, parseFloat(inp.value) || 0);
    l.basis = "user";
    piSave(); refreshTree();
  });
  box.querySelectorAll("[data-rail]").forEach((b) => b.onclick = () => { S.piRail = b.dataset.rail; S.piLayer = -1; setPiMode("v"); fitRail(); });
  box.querySelectorAll("[data-part]").forEach((b) => b.onclick = () => showPart(b.dataset.part, b.dataset.prail));
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

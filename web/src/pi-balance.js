"use strict";
// ---- power balance view ------------------------------------------------------
function piCopperLoss() { let w = 0; for (const r of S.piRes.values()) if (r.res) w += r.res.power_loss_w; return w; }
const KIND_NAME = { input: "input", buck: "buck", boost: "boost", ldo: "LDO", inverter: "inverter", pass: "pass-through" };
const mW = (w) => (w >= 1 ? fmt(w, 2) + " W" : fmt(w * 1e3, 0) + " mW");

function renderBalance() {
  const box = $("pi-balance"), t = S.treeRes;
  if (!t) {
    box.innerHTML = `<div class="bal-head"><h3>Power budget</h3><p>No power tree is defined for this board. Add PI_REG fields to the regulators in KiCad; the regulator chains of the example boards ThunderScope and LibreVNA are modelled.</p></div>`;
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
  const walk = (net, d) => {
    const r = t.rails.find((x) => x.net === net);
    if (!r) return;
    const pad = `style="padding-left:${d * 16}px"`;
    rows.push(`<div class="t-rail" ${pad}><button data-rail="${esc(net)}" title="Show voltage drop of this rail">${esc(netLabel(net).split("/").pop())}</button><span class="t-num">${fmt(r.voltage, 2)} V · ${fmt(r.i_total, 3)} A · ${mW(Math.abs(r.voltage) * r.i_total)}</span></div>`);
    for (const { l, i } of loadsOn(net)) {
      rows.push(`<div class="t-load" style="padding-left:${d * 16 + 12}px"><span>${esc(l.refdes)} <span class="b">${esc(l.part)} · ${esc(l.basis || "")}${l.note ? " · " + esc(l.note) : ""}</span></span>
        <input type="number" min="0" step="0.001" value="${+l.current.toFixed(4)}" data-load="${i}" aria-label="Current ${esc(l.refdes)} on ${esc(netLabel(net))} in A"></div>`);
    }
    for (const e of kids(net)) {
      const eff = e.kind === "pass" ? "" : e.p_in > 0 ? ` · η ${fmt(100 * e.p_out / e.p_in, 0)} %` : "";
      rows.push(`<div class="t-el" style="padding-left:${d * 16 + 12}px"><span>└ <b>${esc(e.id)}</b> ${esc(KIND_NAME[e.kind] || e.kind)} · ${esc(e.part)}${e.overload ? `<span class="chip-warn">${fmt(e.i_out, 2)} A &gt; ${fmt(e.imax, 2)} A</span>` : ""}</span><span class="t-num">loss ${mW(e.p_loss)}${eff}</span></div>`);
      walk(e.to, d + 1);
    }
  };
  for (const e of t.elements.filter((e) => e.kind === "input")) {
    rows.push(`<div class="t-el"><span><b>${esc(e.refdes)}</b> ${esc(e.part)}</span><span class="t-num">${mW(e.p_out)}</span></div>`);
    walk(e.to, 0);
  }
  // components
  const comps = t.components.filter((c) => c.consumption + c.loss >= 0.0005);
  const maxW = Math.max(...comps.map((c) => c.consumption + c.loss), 1e-9);
  const basis = (c) => { const b = [...new Set(c.rails.map((r) => r[2]))]; return c.rails.length ? (b.length > 1 ? "mixed" : b[0]) : "computed"; };
  const crow = (c) => {
    const wu = 100 * c.consumption / maxW, wl = 100 * c.loss / maxW;
    return `<tr title="${esc(c.refdes)}: consumption ${mW(c.consumption)}, loss ${mW(c.loss)}"><td>${esc(c.refdes)}</td><td>${esc(c.part)}</td>
      <td class="n">${c.consumption ? fmt(c.consumption * 1e3, 0) : "–"}</td><td class="n">${c.loss ? fmt(c.loss * 1e3, 0) : "–"}</td>
      <td><div class="bar">${wu > 0 ? `<i class="u" style="width:${wu}%"></i>` : ""}${wl > 0 ? `<i class="l" style="width:${wl}%"></i>` : ""}</div></td><td class="b">${esc(basis(c))}</td></tr>`;
  };
  const warn = (t.warnings || []).map((w) => `<div class="warn">${esc(w)}</div>`).join("");
  box.innerHTML = `
    <div class="bal-head"><h3>Power budget · ${esc(t.name)}</h3><p>${esc(t.note)}</p></div>
    <div class="bal-kpis">
      ${kpi(fmt(pin, 2), "W", "input power")}
      ${kpi(fmt(t.p_loads, 2), "W", "consumed by components")}
      ${kpi(fmt(regLoss, 2), "W", "regulator losses")}
      ${kpi(fmt((passLoss + cu) * 1e3, 0), "mW", "shunts, ferrites and copper")}
      ${kpi(fmt(100 * t.p_loads / pin, 0), "%", "overall efficiency")}
    </div>
    ${warn}
    <div class="bal-cols">
      <div class="bal-card"><h4>Power tree <em>currents in A are editable</em></h4><div class="tree">${rows.join("")}</div>
        <div class="bal-actions" style="margin-top:8px"><button class="btn small" id="bal-reset">Reset values</button><span class="note">Changes are stored in this browser.</span></div></div>
      <div class="bal-card bal-comp"><h4>Components <em>${comps.length} with ≥ 0.5 mW</em></h4>
        <div class="bal-legend"><span><i style="background:var(--pw-use)"></i>consumption</span><span><i style="background:var(--pw-loss)"></i>loss (regulator, shunt, ferrite)</span></div>
        <div class="tablewrap"><table class="ctab"><thead><tr><th>Component</th><th>Part</th><th>consumption mW</th><th>loss mW</th><th></th><th>Basis</th></tr></thead><tbody>${comps.map(crow).join("")}</tbody></table></div>
        <p class="note">Copper losses of the layers (${fmt(cu * 1e3, 0)} mW) come from the voltage-drop solution and are not assigned to any component.</p></div>
    </div>`;
  box.querySelectorAll("[data-load]").forEach((inp) => inp.onchange = () => {
    const l = S.piTree.loads[+inp.dataset.load];
    l.current = Math.max(0, parseFloat(inp.value) || 0);
    l.basis = "user";
    piSave(); refreshTree();
  });
  box.querySelectorAll("[data-rail]").forEach((b) => b.onclick = () => { S.piRail = b.dataset.rail; S.piLayer = -1; setPiMode("v"); fitRail(); });
  $("bal-reset").onclick = () => { try { localStorage.removeItem(treeKey()); } catch (e) { /* ignore */ } initPi(S.piDefaults); setPiMode("b"); };
}
function refreshTree() {
  const keep = S.piRail;
  computeTree();
  S.piRes = new Map();
  for (const r of S.piSetups.values()) solvePiRail(r.net, false);
  S.piRail = keep;
  renderPi();
}

"use strict";
// ---- power balance view ------------------------------------------------------
function piCopperLoss() { let w = 0; for (const r of S.piRes.values()) if (r.res) w += r.res.power_loss_w; return w; }
const KIND_NAME = { input: "Eingang", buck: "Buck", boost: "Boost", ldo: "LDO", inverter: "Inverter", pass: "Durchgang" };
const mW = (w) => (w >= 1 ? fmt(w, 2) + " W" : fmt(w * 1e3, 0) + " mW");

function renderBalance() {
  const box = $("pi-balance"), t = S.treeRes;
  if (!t) {
    box.innerHTML = `<div class="bal-head"><h3>Leistungsbilanz</h3><p>Für dieses Board ist kein Versorgungsbaum hinterlegt. Die Reglerketten sind für die Beispielboards ThunderScope und LibreVNA modelliert.</p></div>`;
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
    rows.push(`<div class="t-rail" ${pad}><button data-rail="${esc(net)}" title="Spannungsabfall dieser Schiene zeigen">${esc(netLabel(net).split("/").pop())}</button><span class="t-num">${fmt(r.voltage, 2)} V · ${fmt(r.i_total, 3)} A · ${mW(Math.abs(r.voltage) * r.i_total)}</span></div>`);
    for (const { l, i } of loadsOn(net)) {
      rows.push(`<div class="t-load" style="padding-left:${d * 16 + 12}px"><span>${esc(l.refdes)} <span class="b">${esc(l.part)} · ${esc(l.basis || "")}${l.note ? " · " + esc(l.note) : ""}</span></span>
        <input type="number" min="0" step="0.001" value="${+l.current.toFixed(4)}" data-load="${i}" aria-label="Strom ${esc(l.refdes)} an ${esc(netLabel(net))} in A"></div>`);
    }
    for (const e of kids(net)) {
      const eff = e.kind === "pass" ? "" : e.p_in > 0 ? ` · η ${fmt(100 * e.p_out / e.p_in, 0)} %` : "";
      rows.push(`<div class="t-el" style="padding-left:${d * 16 + 12}px"><span>└ <b>${esc(e.id)}</b> ${esc(KIND_NAME[e.kind] || e.kind)} · ${esc(e.part)}${e.overload ? `<span class="chip-warn">${fmt(e.i_out, 2)} A &gt; ${fmt(e.imax, 2)} A</span>` : ""}</span><span class="t-num">Verlust ${mW(e.p_loss)}${eff}</span></div>`);
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
  const basis = (c) => { const b = [...new Set(c.rails.map((r) => r[2]))]; return c.rails.length ? (b.length > 1 ? "gemischt" : b[0]) : "berechnet"; };
  const crow = (c) => {
    const wu = 100 * c.consumption / maxW, wl = 100 * c.loss / maxW;
    return `<tr title="${esc(c.refdes)}: Verbrauch ${mW(c.consumption)}, Verlust ${mW(c.loss)}"><td>${esc(c.refdes)}</td><td>${esc(c.part)}</td>
      <td class="n">${c.consumption ? fmt(c.consumption * 1e3, 0) : "–"}</td><td class="n">${c.loss ? fmt(c.loss * 1e3, 0) : "–"}</td>
      <td><div class="bar">${wu > 0 ? `<i class="u" style="width:${wu}%"></i>` : ""}${wl > 0 ? `<i class="l" style="width:${wl}%"></i>` : ""}</div></td><td class="b">${esc(basis(c))}</td></tr>`;
  };
  const warn = (t.warnings || []).map((w) => `<div class="warn">${esc(w)}</div>`).join("");
  box.innerHTML = `
    <div class="bal-head"><h3>Leistungsbilanz · ${esc(t.name)}</h3><p>${esc(t.note)}</p></div>
    <div class="bal-kpis">
      ${kpi(fmt(pin, 2), "W", "Eingangsleistung")}
      ${kpi(fmt(t.p_loads, 2), "W", "Verbrauch der Bauteile")}
      ${kpi(fmt(regLoss, 2), "W", "Verluste in Reglern")}
      ${kpi(fmt((passLoss + cu) * 1e3, 0), "mW", "Shunts, Ferrite und Kupfer")}
      ${kpi(fmt(100 * t.p_loads / pin, 0), "%", "Gesamtwirkungsgrad")}
    </div>
    ${warn}
    <div class="bal-cols">
      <div class="bal-card"><h4>Versorgungsbaum <em>Ströme in A editierbar</em></h4><div class="tree">${rows.join("")}</div>
        <div class="bal-actions" style="margin-top:8px"><button class="btn small" id="bal-reset">Werte zurücksetzen</button><span class="note">Änderungen werden in diesem Browser gespeichert.</span></div></div>
      <div class="bal-card bal-comp"><h4>Bauteile <em>${comps.length} mit ≥ 0,5 mW</em></h4>
        <div class="bal-legend"><span><i style="background:var(--pw-use)"></i>Verbrauch</span><span><i style="background:var(--pw-loss)"></i>Verlust (Regler, Shunt, Ferrit)</span></div>
        <div class="tablewrap"><table class="ctab"><thead><tr><th>Bauteil</th><th>Typ</th><th>Verbrauch mW</th><th>Verlust mW</th><th></th><th>Grundlage</th></tr></thead><tbody>${comps.map(crow).join("")}</tbody></table></div>
        <p class="note">Kupferverluste der Lagen (${fmt(cu * 1e3, 0)} mW) stammen aus dem Spannungsabfall und sind keinem Bauteil zugeordnet.</p></div>
    </div>`;
  box.querySelectorAll("[data-load]").forEach((inp) => inp.onchange = () => {
    const l = S.piTree.loads[+inp.dataset.load];
    l.current = Math.max(0, parseFloat(inp.value) || 0);
    l.basis = "Benutzer";
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

"use strict";
// ---- device-to-device lanes (M7): overview of buses, lane channel and eye
//
// Data:   S.buses = S.engine.buses(params): buses → lanes (topology through series
//         parts and terminations, Rust kisi_si::topology). A lane is identified by
//         its first net and its far end device (laneKey).
// Engine: S.engine.laneRf(...) for one lane, kisi_check items {lane, to, reversed}.

const laneSwapKey = () => "kisi.laneswap.v1." + S.name;
const laneKey = (l) => `${l.nets[0]}>${l.ends.length ? l.ends[l.ends.length - 1].refdes : ""}`;
const CHECKABLE = new Set(["ic-ic", "ic-connector"]);
function laneSwaps() {
  if (!S.laneSw || S.laneSw.board !== S.name) S.laneSw = { board: S.name, map: lsGet(laneSwapKey()) || {} };
  return S.laneSw.map;
}
function allLanes() { return (S.buses || []).flatMap((b) => b.lanes.map((l) => (l.bus = b, l))); }
// transmit from ends[1] to ends[0]?
function laneReversed(l) { return (l.rx_end === 0) !== !!laneSwaps()[laneKey(l)]; }
function laneTx(l) { return l.ends[laneReversed(l) ? 1 : 0]; }
function laneRx(l) { return l.ends[laneReversed(l) ? 0 : 1]; }
function laneOfNet(net) {
  if (!S.buses) return null;
  const ls = allLanes().filter((l) => l.nets.includes(net) || l.partner_nets.includes(net));
  return ls.find((l) => CHECKABLE.has(l.kind)) || ls[0] || null;
}
// the net that carries the target impedance (for profile and target): the one the lane is
// named after (its own path), else its first net with a target
function laneNet(l) { return l.name_net != null && classTarget(l.name_net) ? l.name_net : (l.nets.find((n) => classTarget(n)) ?? l.nets[0]); }
// the other paths of a fork (open branches): {to, through, nets} – not part of the lane's path
function laneBranches(l) {
  const own = new Set([...l.nets, ...l.partner_nets]);
  return l.branches.map((b, i) => ({
    to: (l.branch_ends || [])[i] || "",
    through: (b.find((h) => h.type === "series") || {}).part,
    nets: [...new Set(b.filter((h) => h.type === "net" && !own.has(h.net)).map((h) => h.net))],
  }));
}
// zoom to the lane's own path (both lines of a pair)
function fitLane(l) {
  const own = new Set([...l.nets, ...l.partner_nets]), pc = S.pieces;
  let bb = [Infinity, Infinity, -Infinity, -Infinity];
  for (let i = 0; i < pc.n; i++) if (own.has(pc.net[i])) bb = [Math.min(bb[0], pc.ax[i], pc.bx[i]), Math.min(bb[1], pc.ay[i], pc.by[i]), Math.max(bb[2], pc.ax[i], pc.bx[i]), Math.max(bb[3], pc.ay[i], pc.by[i])];
  if (isFinite(bb[0])) fitView([bb[0] - 1.5, bb[1] - 1.5, bb[2] + 1.5, bb[3] + 1.5]);
}
// board: the lane's own nets highlighted, its branches dashed
function highlightLane(l) {
  buildPaths(new Set([...l.nets, ...l.partner_nets]));
  const br = new Set(laneBranches(l).flatMap((b) => b.nets));
  S.stubPaths = br.size ? netPaths(br) : null;
  draw();
}
function laneProfile(l) { return profileOf(laneNet(l)); }
function laneZStats(l) {
  let len = 0, zs = 0, lo = Infinity, hi = -Infinity;
  for (const n of l.nets) {
    const st = S.an.nets.find((x) => x.net === n);
    if (!st || st.z_mean == null) continue;
    len += st.length; zs += st.z_mean * st.length; lo = Math.min(lo, st.z_min); hi = Math.max(hi, st.z_max);
  }
  return len ? { z_mean: zs / len, z_min: lo, z_max: hi } : {};
}
const shortNet = (n) => netShort(S.scene.nets[n] || "");
const pinLabel = (e) => e ? `${e.refdes}.${e.pin}` : "–";

function chForLane(l) {
  const net = laneNet(l), t = classTarget(net), p = laneProfile(l), rp = rfParams(), rev = laneReversed(l);
  rp.fmax = Math.max(rp.fmax, p.rate_gbps * 1.25e9);
  const key = `lane|${laneKey(l)}|${rev}|${S.anGen}|${JSON.stringify(rp)}|${JSON.stringify(p)}`;
  if (S.chCache && S.chCache.key === key) return S.chCache.res;
  let res;
  try {
    res = S.engine.laneRf({ lane: l.nets[0], to: l.ends.length ? l.ends[l.ends.length - 1].refdes : null, reversed: rev, rf: rp, target: t ? [t.z, t.diff] : null,
      bitrate: p.rate_gbps * 1e9, tx: p.tx, rx: p.rx, eye_rise: p.tx.rise_ps * 1e-12 });
  } catch (e) { res = { channel: null, error: e.message, crosstalk: { aggressors: [], geoms: [] } }; }
  res.profile = p; res.lane = l;
  S.chCache = { key, res };
  return res;
}

// path of a lane as chips: pin – parts – pin, terminations and branches
function laneChain(l) {
  const parts = l.hops.filter((h) => h.type === "series").map((h) => `<span class="lc-part" title="${esc(h.part.package)}">${esc(h.part.refdes)} ${esc(h.part.value)}</span>`);
  const rev = laneReversed(l);
  const a = laneTx(l), b = laneRx(l);
  const mid = rev ? parts.reverse() : parts;
  const sh = l.shunts.map((s) => `<span class="lc-shunt" title="${s.across ? "across the pair" : "to " + esc(shortNet(s.to))}">∥ ${esc(s.part.refdes)} ${esc(s.part.value)}</span>`);
  const br = laneBranches(l).map((b) => `<span class="lc-branch" title="the other path of a fork, not part of this lane: an open branch from the junction (${esc(b.nets.map(shortNet).join(", "))})">⑂ ${b.through ? esc(b.through.refdes) + " → " : ""}${esc(b.to || "branch")}</span>`).join("");
  return `<span class="lc-pin">${esc(pinLabel(a))}</span>${mid.map((x) => `<i>→</i>${x}`).join("")}<i>→</i><span class="lc-pin">${esc(pinLabel(b))}</span>${sh.join("")}${br}`;
}

function laneRow(l) { return S.laneRes ? S.laneRes.get(laneKey(l)) : null; }
function laneStatus(l) {
  const r = laneRow(l);
  return r ? judge(rowMetrics(r, l), laneProfile(l)).status : null;
}

// ---- overview (stage panel like the power tree)
const KIND_TITLE = {
  "ic-ic": ["Device to device", "transmitter and receiver on this board: eye, loss, return loss, skew"],
  "ic-connector": ["Device to connector", "the channel continues off the board: loss, return loss and skew of the board share, no eye"],
  other: ["Multi-drop and open lines", "more than two devices on one line or no second device: listed, not checked"],
};
function renderChOverview() {
  const box = $("ch-overview");
  if (variantNeeded()) { box.innerHTML = variantPrompt(); bindVariantButtons(box); return; }
  if (!S.buses) { box.innerHTML = ""; return; }
  const groups = { "ic-ic": [], "ic-connector": [], other: [] };
  for (const b of S.buses) {
    const k = CHECKABLE.has(b.kind) ? b.kind : "other";
    groups[k].push(b);
  }
  const nl = (k) => groups[k].reduce((a, b) => a + b.lanes.length, 0);
  const res = S.laneRes;
  const counts = { ok: 0, high1: 0, high2: 0 };
  if (res) for (const l of allLanes()) { const st = laneStatus(l); if (st) counts[st]++; }
  const stale = S.check && (S.check.gen !== S.anGen || S.check.stale);
  const kpi = (v, l) => `<div class="bal-kpi"><span class="v">${v}</span><span class="l">${l}</span></div>`;
  const busCard = (b) => {
    const l0 = b.lanes[0], p = laneProfile(l0);
    const sts = b.lanes.map(laneStatus).filter(Boolean);
    const rank = { high2: 0, high1: 1, ok: 2 };
    const worst = sts.length ? sts.sort((x, y) => rank[x] - rank[y])[0] : null;
    const tx = laneTx(l0), rx = laneRx(l0);
    const oneDir = l0.ends.length === 2 && b.lanes.every((l) => l.ends.length === 2 && laneTx(l).refdes === tx.refdes);
    const dir = oneDir ? `${esc(tx.refdes)} <span class="muted">${esc(tx.value)}</span> → ${esc(rx.refdes)} <span class="muted">${esc(rx.value)}</span>`
      : b.devices.map((d, i) => `${esc(d)} <span class="muted">${esc(b.values[i] || "")}</span>`).join(" ↔ ") + (l0.ends.length === 2 ? ' <span class="muted">· direction per lane</span>' : "");
    const checkable = CHECKABLE.has(b.kind);
    const rows = b.lanes.map((l) => {
      const r = laneRow(l), st = r ? laneStatus(l) : null;
      const cells = r ? `<td class="n">${fmt(r.length_mm, 1)}</td><td class="n">${fmt(r.il_nyq_db, 1)}</td><td class="n">${fmt(r.rl_db, 1)}</td><td class="n">${r.eye && !r.error ? fmt(r.eye_height * 100, 0) : "–"}</td><td class="n">${r.skew_ps == null ? "–" : fmt(Math.abs(r.skew_ps), 1) + (r.skew_uncertain ? "?" : "")}</td>`
        : `<td class="n" colspan="5">${checkable ? '<span class="muted">not checked yet</span>' : `<span class="muted">${esc(l.kind)}</span>`}</td>`;
      return `<tr class="click" data-lane="${esc(laneKey(l))}"><td>${st ? `<span class="dot" style="background:${S.colors[st]}" aria-label="${ST_NAME[st]}"></span>` : ""}</td><td class="ln">${esc(netShort(l.name))}${l.diff ? ' <span class="muted">pair</span>' : ""}</td><td class="lc">${laneChain(l)}</td>${cells}</tr>`;
    }).join("");
    return `<div class="bal-card bus-card">
      <h4><span>${worst ? `<span class="dot" style="background:${S.colors[worst]}"></span> ` : ""}${esc(b.name)}</span><em>${esc(b.class)} · ${esc(p.name)} · ${b.lanes.length} lane${b.lanes.length === 1 ? "" : "s"}</em></h4>
      <div class="bus-meta"><span>${dir}</span>${checkable && l0.ends.length === 2 ? `<span class="muted">${esc(laneSwaps()[laneKey(l0)] ? "direction swapped by you" : oneDir ? l0.direction : "")}</span><button class="btn small" data-swap="${esc(b.name)}|${esc(b.class)}">Swap direction</button>` : ""}</div>
      <div class="tablewrap"><table class="ctab lanes"><thead><tr><th></th><th>Lane</th><th>Path</th><th>mm</th><th>IL dB</th><th>RL dB</th><th>Eye %</th><th>Skew ps</th></tr></thead><tbody>${rows}</tbody></table></div>
    </div>`;
  };
  const section = (k) => groups[k].length ? `<h3 class="ov-h">${KIND_TITLE[k][0]} <em>${groups[k].length} bus${groups[k].length === 1 ? "" : "es"}, ${nl(k)} lane${nl(k) === 1 ? "" : "s"} · ${KIND_TITLE[k][1]}</em></h3>${groups[k].map(busCard).join("")}` : "";
  box.innerHTML = `
    <div class="bal-head"><h3>Interfaces · ${esc(S.name)}</h3>
      <p>Signals with a target impedance, followed from device to device through series resistors and AC-coupling capacitors. Terminations (∥), branches (⑂) and side stubs (unused pads, parts not fitted) are part of the channel. Profiles come from the net class (“Interfaces per net class” card).${variantShown() ? ` Assembly variant: <b>${esc(variantShown())}</b>.` : ""}${variantUnused() ? ` ${variantUnused().size} net${variantUnused().size === 1 ? "" : "s"} only connect parts not fitted: not used in this variant, faint on the board map.` : ""} On the board map, a click on a channel opens it.</p></div>
    <div class="bal-kpis">
      ${kpi(S.buses.length, "buses")}
      ${kpi(nl("ic-ic"), "device to device")}
      ${kpi(nl("ic-connector"), "device to connector")}
      ${kpi(nl("other"), "not checked")}
      ${kpi(res ? `<span style="color:${S.colors.ok}">${counts.ok}</span> / <span style="color:${S.colors.high1}">${counts.high1}</span> / <span style="color:${S.colors.high2}">${counts.high2}</span>` : "–", res ? `green / yellow / red${stale ? " · outdated" : ""}` : "after “Check”")}
    </div>
    ${S.an.skeleton ? `<div class="ov-actions"><span class="note">Loss, eye and skew need the impedance estimate (step 2 in the impedance view).</span><button class="btn small primary" id="ov-est">Estimate impedances</button></div>` : ""}
    <div class="ov-actions"><button class="btn small primary" id="ov-check" ${S.an.skeleton || S.checkRunning ? "disabled" : ""}>${res ? "Check again" : "Check all lanes"}</button><span class="note" id="ov-check-text"></span></div>
    ${section("ic-ic")}${section("ic-connector")}${section("other")}`;
  $("ov-check").onclick = () => runCheck();
  if ($("ov-est")) $("ov-est").onclick = () => estimateImpedances();
  box.querySelectorAll("tr[data-lane]").forEach((tr) => tr.onclick = () => {
    const l = allLanes().find((x) => laneKey(x) === tr.dataset.lane);
    if (!l) return;
    S.selLane = laneKey(l); S.chTab = "check";
    setChMode("m"); selectNet(l.nets[0], false); fitLane(l);
  });
  box.querySelectorAll("[data-swap]").forEach((bt) => bt.onclick = () => {
    const [name, cls] = bt.dataset.swap.split("|");
    const b = S.buses.find((x) => x.name === name && x.class === cls);
    const m = laneSwaps();
    for (const l of b.lanes) { const k = laneKey(l); if (m[k]) delete m[k]; else m[k] = true; }
    lsSet(laneSwapKey(), m); S.chCache = null; markCheckStale(); renderChOverview();
  });
}

// ---- detail of one lane (stage panel "ch-detail")
function renderLaneDetail(l) {
  if (variantNeeded()) {
    const d = $("ch-detail");
    d.innerHTML = `<header><div><h3>${esc(netShort(l.name))}</h3><div class="muted">${esc(l.class)}</div></div><button class="x" id="closeCh" aria-label="Clear selection">×</button></header><div id="ch-body">${variantPrompt()}</div>`;
    d.hidden = false; $("closeCh").onclick = () => { S.selLane = null; selectNet(-1); }; bindVariantButtons(d);
    return;
  }
  const d = $("ch-detail"), r = chForLane(l), p = r.profile;
  const tab = S.chTab || "check";
  const tx = laneTx(l), rx = laneRx(l);
  const tabs = [["check", "Check"], ...(l.kind === "ic-ic" ? [["eye", "Eye"]] : []), ["path", "Path"]];
  const t = tabs.some((x) => x[0] === tab) ? tab : "check";
  d.innerHTML = `<header><div><h3>${esc(netShort(l.name))}${l.diff ? ` <span class="muted">pair</span>` : ""}</h3><div class="muted">${esc(pinLabel(tx))} ${esc(tx?.value || "")} → ${esc(pinLabel(rx))} ${esc(rx?.value || "")} · ${esc(l.class)}</div></div><button class="x" id="closeCh" aria-label="Clear selection">×</button></header>
    <div class="dstats"><span>Profile <b>${esc(p.name)}</b>${p.edited ? " *" : ""}${variantShown() ? ` · variant ${esc(variantShown())}` : ""}</span><span><button class="btn small" id="ch-edit">Edit profile</button> <button class="btn small" id="ch-swap">Swap direction</button></span></div>
    <div class="dstats muted"><span>Transmitter ${esc(tx?.refdes || "")}: ${fmt(p.tx.r, 0)} Ω${p.tx.c_pf ? ", " + fmt(p.tx.c_pf, 1) + " pF" : ""}, ${fmt(p.tx.rise_ps, 0)} ps, ${fmt(p.tx.swing_mv, 0)} mV</span>
      <span>Receiver ${esc(rx?.refdes || "")}: ${r.rx_used ? (r.rx_used.r > 0 ? fmt(r.rx_used.r, 0) + " Ω" : "high impedance") : (p.rx.r > 0 ? fmt(p.rx.r, 0) + " Ω" : "high impedance")}${p.rx.c_pf ? ", " + fmt(p.rx.c_pf, 1) + " pF" : ""}</span><span>per line</span></div>
    ${r.rx_note ? `<div class="dstats muted"><span>${esc(r.rx_note)}</span></div>` : ""}
    <div class="tabs" role="tablist" aria-label="Lane">${tabs.map(([k, n]) => `<button role="tab" data-ct="${k}" aria-selected="${t === k}">${n}</button>`).join("")}</div>
    <div id="ch-body"></div>`;
  d.hidden = false;
  highlightLane(l);
  $("closeCh").onclick = () => { S.selLane = null; selectNet(-1); };
  $("ch-edit").onclick = () => { S.profEdit = p.id; renderProfCard(); $("h-prof").scrollIntoView({ behavior: "smooth", block: "start" }); };
  $("ch-swap").onclick = () => {
    const m = laneSwaps(), k = laneKey(l);
    if (m[k]) delete m[k]; else m[k] = true;
    lsSet(laneSwapKey(), m); S.chCache = null; markCheckStale(); renderLaneDetail(l);
  };
  d.querySelectorAll("[data-ct]").forEach((b) => b.onclick = () => { S.chTab = b.dataset.ct; renderLaneDetail(l); });
  const body = $("ch-body");
  if (t === "eye") return renderEye(body, r);
  if (t === "path") return renderLanePath(body, l, r);
  const m = metricsOf(r, laneNet(l), l), j = judge(m, p);
  const xt = r.crosstalk && r.crosstalk.aggressors ? `<tr><td></td><td>Crosstalk (total)</td><td>NEXT ${fmt(r.crosstalk.next_total * 100, 2)} %, FEXT ${fmt(r.crosstalk.fext_total * 100, 2)} %</td><td>information only</td></tr>` : "";
  body.innerHTML = `<div class="dstats"><span>Result <b style="color:${S.colors[j.status]}">${ST_NAME[j.status]}</b></span><span>${fmt(p.rate_gbps, p.rate_gbps < 1 ? 2 : p.rate_gbps % 1 ? 2 : 0)} Gbit/s</span>${r.channel ? `<span>Path ${fmt(r.channel.length_mm, 1)} mm, ${r.channel.vias} vias, ${l.hops.filter((h) => h.type === "series").length} series part(s)</span>` : ""}</div>
    <div class="tablewrap"><table class="xs-table"><thead><tr><th></th><th>Criterion</th><th>Value</th><th>Limit</th></tr></thead><tbody>
    ${j.rows.map((x) => `<tr title="${esc(x.note || "")}"><td><span class="dot" style="background:${S.colors[x.st]}" aria-label="${ST_NAME[x.st]}"></span></td><td>${x.name}</td><td>${esc(String(x.value))}</td><td>${x.limit}</td></tr>`).join("")}${xt}
    </tbody></table></div>
    <p class="note">${l.kind === "ic-ic" ? "From device pin to device pin, including series parts (package parasitics estimated, data/components.json). Loss, return loss and TDR describe the interconnect without terminations; the eye uses the profile's transmitter and receiver, with a termination next to the receiver as its termination." : "From the device to the connector: the board share of the channel. No eye, because the rest of the channel is off the board."} No equalisation, jitter or noise.</p>
    ${(r.channel?.warnings || []).map((w) => `<div class="warn">${esc(w)}</div>`).join("")}`;
}

function renderLanePath(body, l, r) {
  const ch = r.channel;
  if (!ch) { body.innerHTML = `<p class="err">${esc(r.error || "no channel")}</p>`; return; }
  const L = (k) => esc(S.scene.stackup.copper[k]?.name ?? k);
  // consecutive line pieces on the same layer with (almost) the same impedance form one row
  const segs = [];
  for (const s of ch.segments) {
    const last = segs[segs.length - 1];
    if (s.kind === "line" && last && last.kind === "line" && last.layer === s.layer && Math.abs((last.z ?? -1) - (s.z ?? -1)) < 0.5 && last.source === s.source) { last.len_mm += s.len_mm; continue; }
    segs.push({ ...s });
  }
  const rows = segs.map((s) => {
    if (s.kind === "line") return `<tr><td>line</td><td>${L(s.layer)}</td><td class="n">${fmt(s.len_mm, 1)} mm</td><td class="n">${s.z != null ? fmt(ch.diff ? 2 * s.z : s.z, 1) + " Ω" : "–"}</td><td class="muted">${esc(s.source)}</td></tr>`;
    if (s.kind === "via") return `<tr><td>via</td><td>${L(s.layer)}</td><td class="n">${fmt(s.len_mm, 2)} mm</td><td class="n">${s.z != null ? fmt(ch.diff ? 2 * s.z : s.z, 0) + " Ω" : "–"}</td><td class="muted">via model</td></tr>`;
    if (s.kind === "branch") return `<tr class="warnrow"><td>${/^stub/.test(s.part || "") ? "stub" : "branch"}</td><td colspan="2">${esc(s.part || "")}</td><td></td><td class="muted">open${/not fitted/.test(s.part || "") ? ", pad C estimated" : ""}</td></tr>`;
    if (/ pad$/.test(s.part || "")) return "";
    return `<tr><td>${s.kind === "series" ? "series" : "shunt"}</td><td colspan="3"><b>${esc(s.part || "")}</b></td><td class="muted">estimated parasitics</td></tr>`;
  }).join("");
  const sh = l.shunts.map((s) => `${esc(s.part.refdes)} ${esc(s.part.value)} ${s.across ? "across the pair" : "to " + esc(shortNet(s.to))}`).join(", ");
  body.innerHTML = `<div class="dstats"><span>${esc(pinLabel(laneTx(l)))} → ${esc(pinLabel(laneRx(l)))}</span><span>${fmt(ch.length_mm, 1)} mm, delay ${fmt(ch.delay_ps, 0)} ps</span>${r.skew ? `<span>skew to ${esc(netShort(r.skew.partner))} ${fmt(r.skew.skew_ps, 1)} ps${r.skew.uncertain ? " (uncertain)" : ""}</span>` : ""}</div>
    <div class="tablewrap"><table class="xs-table"><thead><tr><th>Element</th><th>Layer / part</th><th>Length</th><th>Z</th><th>Source</th></tr></thead><tbody>${rows}</tbody></table></div>
    <p class="note">Nets: ${l.nets.map((n) => esc(shortNet(n))).join(" → ")}${l.diff ? ` · partner ${l.partner_nets.map((n) => esc(shortNet(n))).join(" → ")}` : ""}.${laneBranches(l).map((b) => ` Branch (open, dashed on the board): ${b.through ? esc(b.through.refdes) + " → " : ""}${esc(b.nets.map(shortNet).join(" → "))} → ${esc(b.to)}.`).join("")}${sh ? ` Terminations: ${sh}.` : ""}${l.others.length ? ` Also connected (not modelled): ${esc(l.others.join(", "))}.` : ""} Direction: ${esc(laneSwaps()[laneKey(l)] ? "swapped by you" : l.direction)}.</p>`;
}

// ---- channel view modes: interfaces overview / board map
function setChMode(m) {
  S.chMode = m;
  for (const k of ["o", "m"]) $("chm-" + k).setAttribute("aria-pressed", String(k === m));
  if (S.tab !== "ch") return;
  const ov = m === "o";
  $("ch-overview").hidden = !ov;
  $("legend").hidden = ov; $("layers").hidden = ov;
  if (ov) { $("ch-detail").hidden = true; renderChOverview(); }
  else { buildPaths(); renderLayerChips(); renderLegend(); if (S.selNet >= 0) selectNet(S.selNet, false); draw(); }
}
$("chm-o").onclick = () => setChMode("o");
$("chm-m").onclick = () => setChMode("m");

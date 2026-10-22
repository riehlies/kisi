"use strict";
// ============================================================== side panels
// board configuration (<board>.kisi.json): what was applied and the check result
function renderCfgStatus() {
  const box = $("cfg-status"), c = S.config;
  box.hidden = !c;
  if (!c) return;
  const ck = c.check || { ok: true, errors: [], warnings: [], entries: 0 };
  const parts = [c.dielectrics ? `${c.dielectrics} stackup` : "", Object.keys(c.targets || {}).length ? `${Object.keys(c.targets).length} target(s)` : "", Object.keys(c.profiles || {}).length ? `${Object.keys(c.profiles).length} profile(s)` : "", c.fields ? `${c.fields} field(s)` : ""].filter(Boolean);
  const list = (xs, cls) => xs.map((x) => `<li class="${cls}">${esc(x)}</li>`).join("");
  box.innerHTML = `<b>${esc(c.name)}</b>: ${parts.join(", ") || "nothing"} applied · ${ck.ok ? `<span class="ok">check passed</span>` : `<span class="err">${ck.errors.length} error(s)</span>`}${ck.warnings.length ? ` · ${ck.warnings.length} warning(s)` : ""}
    ${ck.errors.length || ck.warnings.length ? `<details><summary>Details</summary><ul>${list(ck.errors, "err")}${list(ck.warnings, "")}</ul></details>` : ""}`;
}

function renderSide() {
  const sc = S.scene, an = S.an, su = sc.stackup;
  $("boardName").textContent = S.name;
  $("boardMeta").textContent = `${su.copper.length} layers · ${sc.tracks.length / 7 - (sc.pad_tracks || []).length} track segments · ${(sc.pad_tracks || []).length} signal pads · ${sc.vias.length / 7} vias`;
  // KPIs
  let lenT = 0, lenOk = 0, nets = 0;
  for (const n of an.nets) if (n.target) { nets++; lenT += n.length; lenOk += (n.within5 || 0) * n.length; }
  let noref = 0; const pc = S.pieces;
  for (let i = 0; i < pc.n; i++) if (an.cross_sections[pc.xs[i]].kind === "NoReference") noref += Math.hypot(pc.bx[i] - pc.ax[i], pc.by[i] - pc.ay[i]);
  $("k-ok").textContent = lenT && !an.skeleton ? Math.round(lenOk / lenT * 100) + " %" : "–";
  $("k-nets").textContent = nets;
  $("k-noref").textContent = an.skeleton ? "–" : fmt(noref, 0);
  $("warnings").innerHTML = (sc.warnings || []).map((w) => `<span class="err">${esc(w)}</span>`).join("<br>");
  renderCfgStatus();
  renderVariantCard();
  // stackup
  let total = 0; su.copper.forEach((c) => total += c.thickness); su.dielectrics.forEach((d, i) => total += stackEff(i).thickness);
  $("stackTotal").textContent = fmt(total, 2) + " mm";
  const rows = [];
  su.copper.forEach((c, i) => {
    rows.push(`<div class="st-row st-cu${S.layerOn[i] ? "" : " off"}"><label for="lay-${i}"><input type="checkbox" id="lay-${i}" data-l="${i}" ${S.layerOn[i] ? "checked" : ""}><span class="st-name">${esc(c.name)}</span></label><div class="st-bar"></div><span class="st-meta">${fmt(c.thickness * 1000, 1)} µm</span></div>`);
    const d0 = su.dielectrics[i];
    if (d0) {
      const d = stackEff(i), ed = stackEdited(i);
      const hpx = Math.max(6, Math.min(40, d.thickness * 60));
      const meta = S.stackEdit
        ? `<span class="st-edit"><input type="number" step="0.01" data-di="${i}" data-k="epsilon_r" value="${d.epsilon_r}" aria-label="εr ${esc(d0.name)}" title="εr"><input type="number" step="0.001" data-di="${i}" data-k="loss_tangent" value="${d.loss_tangent}" aria-label="tanδ ${esc(d0.name)}" title="tanδ"><input type="number" step="1" data-di="${i}" data-k="thickness" value="${Math.round(d.thickness * 1000)}" aria-label="Thickness µm ${esc(d0.name)}" title="Thickness µm"></span>`
        : `${esc(d0.material)} · ${fmt(d.thickness * 1000, 0)} µm · εr ${fmt(d.epsilon_r, 2)} · tanδ ${fmt(d.loss_tangent, 3)}${ed ? " *" : ""}`;
      rows.push(`<div class="st-row st-di${ed ? " edited" : ""}"><span class="st-name muted">${esc(d0.kind || "")}</span><div class="st-bar" style="height:${hpx}px"></div><span class="st-meta">${meta}</span></div>`);
    }
  });
  if (!su.from_file) rows.unshift(`<p class="err">No stackup in the file – assumed values</p>`);
  const nEd = Object.keys(stackOv().dielectrics || {}).length;
  rows.push(`<div class="fs-row"><span class="note">${S.stackEdit ? "Columns: εr · tanδ · thickness µm. Affects impedance, loss, field solver and PDN." : nEd ? `${nEd} layer(s) changed (*), stored in this browser.` : (S.config && S.config.dielectrics ? `Values from the file and ${esc(S.config.name)}.` : "Values from the file.")}</span><span>${nEd && !S.stackEdit ? `<button class="btn small" id="st-reset">File values</button> ` : ""}<button class="btn small" id="st-edit">${S.stackEdit ? "Done" : "Edit"}</button></span></div>`);
  $("stack").innerHTML = rows.join("");
  $("stack").querySelectorAll("input[data-l]").forEach((el) => el.onchange = () => { S.layerOn[+el.dataset.l] = el.checked; renderLayerChips(); renderSide(); draw(); });
  $("st-edit").onclick = () => { S.stackEdit = !S.stackEdit; renderSide(); };
  if ($("st-reset")) $("st-reset").onclick = () => { lsSet(stackKey(), null); S.stackOv = null; runAnalysis(); };
  $("stack").querySelectorAll("input[data-di]").forEach((el) => el.onchange = () => {
    const i = +el.dataset.di, k = el.dataset.k, f = S.scene.stackup.dielectrics[i];
    let v = parseFloat(el.value);
    if (!isFinite(v)) return;
    if (k === "thickness") v /= 1000;
    const ov = stackOv(), o = ov.dielectrics[i] || (ov.dielectrics[i] = {});
    if (Math.abs(v - f[k]) < 1e-9) delete o[k]; else o[k] = v;
    if (!Object.keys(o).length) delete ov.dielectrics[i];
    lsSet(stackKey(), ov);
    runAnalysis();
  });
  renderLayerChips();
  // targets
  const tgRows = [`<span class="h">Class</span><span class="h">Target Ω</span><span class="h">Type</span>`];
  an.classes.forEach((c, i) => {
    if (!c.nets && !c.target) return;
    const t = c.target;
    tgRows.push(`<label class="cn" for="tz-${i}">${esc(c.name)}<small>${c.nets} nets${c.target_source ? " · " + esc(c.target_source) : ""}</small></label>
      <input id="tz-${i}" data-c="${esc(c.name)}" type="number" min="10" max="200" step="1" value="${t ? t.z : ""}" placeholder="–" inputmode="decimal">
      <select id="tk-${i}" data-c="${esc(c.name)}" aria-label="Target impedance type for ${esc(c.name)}"><option value="se" ${t && !t.diff ? "selected" : ""}>SE</option><option value="diff" ${t && t.diff ? "selected" : ""}>Diff</option></select>`);
  });
  if (!$("targets").contains(document.activeElement)) $("targets").innerHTML = tgRows.join("");
  const onTarget = (name) => {
    const i = an.classes.findIndex((c) => c.name === name);
    const z = parseFloat($("tz-" + i).value), diff = $("tk-" + i).value === "diff";
    if (isFinite(z) && z > 0) S.overrides[name] = { z, diff }; else S.overrides[name] = { z: 0, diff };
    if (!(isFinite(z) && z > 0)) delete S.overrides[name];
    runAnalysis();
  };
  $("targets").querySelectorAll("input,select").forEach((el) => el.onchange = () => onTarget(el.dataset.c));
  // zone layer select
  $("opt-zone").innerHTML = `<option value="-1">none</option>` + su.copper.map((c, i) => `<option value="${i}" ${i === S.zoneLayer ? "selected" : ""}>${esc(c.name)}</option>`).join("");
  renderNets();
}

function renderLayerChips() {
  const su = S.scene.stackup;
  if (S.tab === "pi") return renderPiLayerChips();
  $("layers").innerHTML = su.copper.map((c, i) => `<button class="chip" data-l="${i}" aria-pressed="${S.layerOn[i]}"><i></i>${esc(c.name)}</button>`).join("");
  $("layers").querySelectorAll(".chip").forEach((b) => b.onclick = (e) => {
    const l = +b.dataset.l;
    if (e.altKey || e.shiftKey) { S.layerOn = S.layerOn.map((_, k) => k === l); } else S.layerOn[l] = !S.layerOn[l];
    renderSide(); draw();
  });
}

function renderNets() {
  const q = $("search").value.trim().toLowerCase();
  let list = S.an.nets.filter((n) => q ? (n.name.toLowerCase().includes(q) || n.class.toLowerCase().includes(q)) : n.target);
  list.sort((a, b) => (a.within5 ?? 2) - (b.within5 ?? 2) || b.length - a.length);
  $("netCount").textContent = q ? `${list.length} matches` : `${list.length} with target${S.an.skeleton ? "" : " · worst first"}`;
  list = list.slice(0, 300);
  $("nets").innerHTML = list.map((n) => {
    const w5 = n.within5;
    const col = w5 == null ? S.colors.none : w5 >= 0.9 ? S.colors.ok : w5 >= 0.6 ? S.colors.high1 : S.colors.high2;
    const short = n.name.split("/").pop() || n.name;
    return `<button class="net" data-net="${n.net}" aria-current="${n.net === S.selNet}" title="${esc(n.name)}">
      <span class="nm"><bdi>${esc(n.name.length > short.length ? "…/" + short : short)}</bdi></span><span class="zz">${n.target ? fmt(n.z_mean, 1) + " / " + fmt(n.target.z, 0) + " Ω" : fmt(n.z_mean, 1) + " Ω"}</span>
      <span class="cl">${esc(n.class)} · ${fmt(n.length, 1)} mm</span><span class="meter" style="width:64px">${w5 != null ? `<i style="width:${Math.round(w5 * 100)}%;background:${col}"></i>` : ""}</span></button>`;
  }).join("");
  $("nets").querySelectorAll(".net").forEach((b) => b.onclick = () => selectNet(+b.dataset.net));
}

function renderLegend() {
  const C = S.colors, L = $("legend");
  if (S.tab === "pi") return renderPiLegend();
  if (S.tab === "ch") {
    const it = [[C.ok, "channel OK"], [C.high1, "limit exceeded (yellow)"], [C.high2, "red / error"], [C.none, "not checked"]];
    L.innerHTML = `<strong>Channel check</strong>` + it.map(([c, t]) => `<div class="row"><span class="sw" style="background:${c}"></span>${t}</div>`).join("");
    return;
  }
  if (S.an && S.an.skeleton) {
    const it = [[C.ms, "Microstrip (outer layer)"], [C.sl, "Stripline (inner layer)"]];
    L.innerHTML = `<strong>Line type (by layer)</strong>` + it.map(([c, t]) => `<div class="row"><span class="sw" style="background:${c}"></span>${t}</div>`).join("") +
      `<div class="row muted">reference planes and impedances: step 2</div>`;
    return;
  }
  if (S.mode === "xt") {
    const it = [[C.high2, "> 3 %"], [C.high1, "1 … 3 %"], [C.none, "below 1 % or no target"]];
    L.innerHTML = `<strong>Crosstalk per stretch</strong>` + it.map(([c, t]) => `<div class="row"><span class="sw" style="background:${c}"></span>${t}</div>`).join("") +
      `<div class="row muted">NEXT or FEXT of each stretch, % of the aggressor swing${S.xtScan && S.xtScan.rows.some((r) => r.source === "formula") ? " (estimate)" : ""}</div>`;
  } else if (S.mode === "dev") {
    const it = [[C.low2, "< −10 %"], [C.low1, "−10 … −5 %"], [C.ok, "±5 % (on target)"], [C.high1, "+5 … +10 %"], [C.high2, "> +10 %"], [C.none, "no target"], [C.noref, "no reference"]];
    L.innerHTML = `<strong>Deviation from target</strong>` + it.map(([c, t]) => `<div class="row"><span class="sw" style="background:${c}"></span>${t}</div>`).join("");
  } else if (S.mode === "abs") {
    const stops = Array.from({ length: 11 }, (_, k) => turbo(k / 10 * 0.9 + 0.05)).join(",");
    L.innerHTML = `<strong>Z₀ (single-ended)</strong><div class="grad" style="background:linear-gradient(90deg,${stops})"></div><div class="ticks"><span>${ABS_MIN}</span><span>50</span><span>75</span><span>100</span><span>${ABS_MAX} Ω</span></div>`;
  } else {
    const it = [[C.ms, "Microstrip (outer layer)"], [C.sl, "Stripline"], [C.emb, "inner, one reference"], [C.noref, "no reference"]];
    L.innerHTML = `<strong>Line type</strong>` + it.map(([c, t]) => `<div class="row"><span class="sw" style="background:${c}"></span>${t}</div>`).join("");
  }
}

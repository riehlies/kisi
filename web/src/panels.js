"use strict";
// ============================================================== side panels
function renderSide() {
  const sc = S.scene, an = S.an, su = sc.stackup;
  $("boardName").textContent = S.name;
  $("boardMeta").textContent = `${su.copper.length} Lagen · ${sc.tracks.length / 7 - (sc.pad_tracks || []).length} Leiterbahnstücke · ${(sc.pad_tracks || []).length} Signal-Pads · ${sc.vias.length / 7} Vias`;
  // KPIs
  let lenT = 0, lenOk = 0, nets = 0;
  for (const n of an.nets) if (n.target) { nets++; lenT += n.length; lenOk += (n.within5 || 0) * n.length; }
  let noref = 0; const pc = S.pieces;
  for (let i = 0; i < pc.n; i++) if (an.cross_sections[pc.xs[i]].kind === "NoReference") noref += Math.hypot(pc.bx[i] - pc.ax[i], pc.by[i] - pc.ay[i]);
  $("k-ok").textContent = lenT ? Math.round(lenOk / lenT * 100) + " %" : "–";
  $("k-nets").textContent = nets;
  $("k-noref").textContent = fmt(noref, 0);
  $("warnings").innerHTML = (sc.warnings || []).map((w) => `<span class="err">${esc(w)}</span>`).join("<br>");
  // stackup
  let total = 0; su.copper.forEach((c) => total += c.thickness); su.dielectrics.forEach((d) => total += d.thickness);
  $("stackTotal").textContent = fmt(total, 2) + " mm";
  const rows = [];
  su.copper.forEach((c, i) => {
    rows.push(`<div class="st-row st-cu${S.layerOn[i] ? "" : " off"}"><label for="lay-${i}"><input type="checkbox" id="lay-${i}" data-l="${i}" ${S.layerOn[i] ? "checked" : ""}><span class="st-name">${esc(c.name)}</span></label><div class="st-bar"></div><span class="st-meta">${fmt(c.thickness * 1000, 1)} µm</span></div>`);
    const d = su.dielectrics[i];
    if (d) {
      const hpx = Math.max(6, Math.min(40, d.thickness * 60));
      rows.push(`<div class="st-row st-di"><span class="st-name muted">${esc(d.kind || "")}</span><div class="st-bar" style="height:${hpx}px"></div><span class="st-meta">${esc(d.material)} · ${fmt(d.thickness * 1000, 0)} µm · εr ${fmt(d.epsilon_r, 2)}</span></div>`);
    }
  });
  if (!su.from_file) rows.unshift(`<p class="err">Kein Lagenaufbau in der Datei – Annahmen</p>`);
  $("stack").innerHTML = rows.join("");
  $("stack").querySelectorAll("input[data-l]").forEach((el) => el.onchange = () => { S.layerOn[+el.dataset.l] = el.checked; renderLayerChips(); renderSide(); draw(); });
  renderLayerChips();
  // targets
  const tgRows = [`<span class="h">Klasse</span><span class="h">Ziel Ω</span><span class="h">Art</span>`];
  an.classes.forEach((c, i) => {
    if (!c.nets && !c.target) return;
    const t = c.target;
    tgRows.push(`<label class="cn" for="tz-${i}">${esc(c.name)}<small>${c.nets} Netze${c.target_source ? " · " + esc(c.target_source) : ""}</small></label>
      <input id="tz-${i}" data-c="${esc(c.name)}" type="number" min="10" max="200" step="1" value="${t ? t.z : ""}" placeholder="–" inputmode="decimal">
      <select id="tk-${i}" data-c="${esc(c.name)}" aria-label="Art der Zielimpedanz für ${esc(c.name)}"><option value="se" ${t && !t.diff ? "selected" : ""}>SE</option><option value="diff" ${t && t.diff ? "selected" : ""}>Diff</option></select>`);
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
  $("opt-zone").innerHTML = `<option value="-1">keine</option>` + su.copper.map((c, i) => `<option value="${i}" ${i === S.zoneLayer ? "selected" : ""}>${esc(c.name)}</option>`).join("");
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
  $("netCount").textContent = q ? `${list.length} Treffer` : `${list.length} mit Ziel · schlechteste zuerst`;
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
  if (S.mode === "dev") {
    const it = [[C.low2, "< −10 %"], [C.low1, "−10 … −5 %"], [C.ok, "±5 % (im Ziel)"], [C.high1, "+5 … +10 %"], [C.high2, "> +10 %"], [C.none, "kein Ziel"], [C.noref, "keine Referenz"]];
    L.innerHTML = `<strong>Abweichung vom Ziel</strong>` + it.map(([c, t]) => `<div class="row"><span class="sw" style="background:${c}"></span>${t}</div>`).join("");
  } else if (S.mode === "abs") {
    const stops = Array.from({ length: 11 }, (_, k) => turbo(k / 10 * 0.9 + 0.05)).join(",");
    L.innerHTML = `<strong>Z₀ (single-ended)</strong><div class="grad" style="background:linear-gradient(90deg,${stops})"></div><div class="ticks"><span>${ABS_MIN}</span><span>50</span><span>75</span><span>100</span><span>${ABS_MAX} Ω</span></div>`;
  } else {
    const it = [[C.ms, "Microstrip (Außenlage)"], [C.sl, "Stripline"], [C.emb, "Innen, eine Referenz"], [C.noref, "keine Referenz"]];
    L.innerHTML = `<strong>Leitungstyp</strong>` + it.map(([c, t]) => `<div class="row"><span class="sw" style="background:${c}"></span>${t}</div>`).join("");
  }
}

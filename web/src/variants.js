"use strict";
// ---- assembly variants: which parts are fitted (Rust kisi_core::variant)
//
// Data:   S.variant = VariantState from the engine: {variants: [{name, description, defined_in}],
//         chosen, chosen_by ("viewer" | "kisi.json"), needs_choice, message, forks, not_fitted,
//         fitted, errors}. Precedence: board file < kisi.json ("variant") < the choice here,
//         stored per board in this browser (variantKey()).
// Rule:   with variants (KiCad 10 or kisi.json) or forks where all alternatives are fitted and
//         no variant chosen, lanes are not computed: the channel view asks first.

const VARIANT_DEFAULT = "default";
const variantKey = (name) => "kisi.variant.v1." + (name ?? S.name);
const variantNeeded = () => !!(S.variant && S.variant.needs_choice);
function variantRelevant() {
  const v = S.variant;
  return !!v && (v.variants.length > 0 || v.forks.length > 0 || (v.chosen && v.chosen !== VARIANT_DEFAULT) || v.errors.length > 0);
}
// the variant <board>.kisi.json chooses (for the "back to kisi.json" button)
function cfgVariant() {
  const t = S.lastFiles && S.lastFiles.cfg && S.lastFiles.cfg.text;
  if (!t) return null;
  try { return JSON.parse(t).variant || null; } catch (e) { return null; }
}
// BOMs that the configuration's variants name, from CSV files dropped with it (matched by name)
function cfgFiles(cfg) {
  const out = {};
  if (!cfg) return out;
  let c = null;
  try { c = JSON.parse(cfg.text); } catch (e) { return out; }
  for (const v of Object.values((c && c.variants) || {})) {
    if (!v || typeof v.bom !== "string" || !cfg.csv) continue;
    const f = cfg.csv[v.bom.split(/[\\/]/).pop()] ?? cfg.csv[v.bom];
    if (f != null) out[v.bom] = f;
  }
  // Touchstone files of the connector ends: dropped with it or loaded in a bus card
  for (const e of Object.values((c && c.connector_ends) || {})) {
    const p = e && e.touchstone && e.touchstone.file;
    if (typeof p !== "string" || !p) continue;
    const f = S.tsFiles[p.split(/[\\/]/).pop()];
    if (f) out[p] = f.text;
  }
  return out;
}
// nets that only connect parts not fitted (board DNP and the variant): no function in this
// assembly – no lane, not checked, drawn faint in the channel view
function variantUnused() {
  const v = S.variant;
  if (!v || !v.unused_nets || !v.unused_nets.length) return null;
  if (!v._unused) v._unused = new Set(v.unused_nets);
  return v._unused;
}
function netUnused(n) { const u = variantUnused(); return !!u && u.has(n); }
const forkText = (f) => `${f.parts.join("/")} on ${netShort(f.net_name)} (→ ${f.towards.map((t) => t.join(", ")).join(" / ")})`;
function variantShown() {
  const v = S.variant;
  if (!v || !v.chosen) return "";
  const by = v.chosen_by === "viewer" ? "your choice" : v.chosen_by === "kisi.json" ? (S.config ? S.config.name : "kisi.json") : v.chosen_by || "";
  return `${v.chosen === VARIANT_DEFAULT ? "as in the board file" : v.chosen}${by ? ` (${by})` : ""}`;
}

// side card (all views; hidden when the board has no variants)
function renderVariantCard() {
  const card = $("variant-card"), box = $("variant-body"), v = S.variant;
  card.classList.toggle("off", !variantRelevant());
  if (!variantRelevant()) { box.innerHTML = ""; return; }
  $("variant-name").textContent = v.chosen ? (v.chosen === VARIANT_DEFAULT ? "board file" : v.chosen) : "not chosen";
  const opts = v.variants.map((x) => `<option value="${esc(x.name)}" ${x.name === v.chosen ? "selected" : ""}>${esc(x.name)}${x.description ? " – " + esc(x.description) : ""}</option>`).join("")
    + `<option value="${VARIANT_DEFAULT}" ${v.chosen === VARIANT_DEFAULT ? "selected" : ""}>as in the board file</option>`;
  const by = v.chosen_by === "viewer" ? "Your choice, stored in this browser." : v.chosen_by === "kisi.json" ? `From ${esc(S.config ? S.config.name : "kisi.json")}.` : "";
  const cv = cfgVariant();
  const ask = !variantNeeded() ? "" : v.variants.length
    ? `This board has ${v.variants.length} assembly variant${v.variants.length === 1 ? "" : "s"}. Lanes depend on the fitted parts: choose one before they are computed.`
    : `kisi found ${v.forks.length} fork${v.forks.length === 1 ? "" : "s"} where all alternative parts are fitted – possibly assembly variants. Define them in &lt;board&gt;.kisi.json (“variants”: by sheet, references or BOM), mark the unused parts DNP in KiCad, or analyse the board as it is.`;
  const refs = (xs) => esc(xs.slice(0, 60).join(", ") + (xs.length > 60 ? ` … ${xs.length - 60} more` : ""));
  box.innerHTML = `${ask ? `<p class="var-ask">${ask}</p>` : ""}
    <label class="opt" for="variant-sel"><span>Variant</span><select id="variant-sel" aria-label="Assembly variant">${variantNeeded() ? '<option value="" selected>– choose –</option>' : ""}${opts}</select></label>
    ${by || (v.chosen_by === "viewer" && cv) ? `<div class="fs-row"><span class="note">${by}</span>${v.chosen_by === "viewer" && cv ? `<button class="btn small" id="variant-reset" title="Use the variant of the board configuration">kisi.json: ${esc(cv)}</button>` : ""}</div>` : ""}
    ${v.not_fitted.length || v.fitted.length ? `<details><summary>${v.not_fitted.length} part${v.not_fitted.length === 1 ? "" : "s"} not fitted${v.fitted.length ? `, ${v.fitted.length} fitted` : ""} in this variant</summary><p class="note mono">${v.not_fitted.length ? "not fitted: " + refs(v.not_fitted) : ""}${v.fitted.length ? `<br>fitted: ${refs(v.fitted)}` : ""}</p></details>` : ""}
    ${v.forks.length ? `<details${variantNeeded() ? " open" : ""}><summary>${v.forks.length} fork${v.forks.length === 1 ? "" : "s"} with all alternatives fitted</summary><ul class="src">${v.forks.map((f) => `<li>${esc(forkText(f))}</li>`).join("")}</ul></details>` : ""}
    ${v.errors.map((e) => `<div class="warn">${esc(e)}</div>`).join("")}
    <p class="note">Parts not fitted connect nothing; their pads stay as copper and form stubs on the lanes (pad capacitance estimated).</p>`;
  $("variant-sel").onchange = (e) => { if (e.target.value) setVariant(e.target.value); };
  if ($("variant-reset")) $("variant-reset").onclick = () => setVariant(null);
}

// prompt in place of lane results (channel overview and lane detail)
function variantPrompt() {
  const v = S.variant;
  const btns = v.variants.map((x) => `<button class="btn small primary" data-variant="${esc(x.name)}" title="${esc(x.description || "")}">${esc(x.name)}</button>`).join("")
    + `<button class="btn small" data-variant="${VARIANT_DEFAULT}">As in the board file</button>`;
  const forks = v.forks.length ? `<ul class="src">${v.forks.slice(0, 12).map((f) => `<li>${esc(forkText(f))}</li>`).join("")}${v.forks.length > 12 ? `<li>… ${v.forks.length - 12} more</li>` : ""}</ul>` : "";
  return `<div class="bal-head"><h3>Which assembly variant?</h3>
      <p>${v.variants.length ? "This board can be built in several ways. Lanes depend on which parts are fitted, so kisi computes them once a variant is chosen. The choice is stored in this browser; “variant” in &lt;board&gt;.kisi.json sets it for everyone." : "All alternative parts of these forks are fitted in the board file – possibly assembly variants. A lane would carry the unused path as an open branch. Define the variants in &lt;board&gt;.kisi.json (“variants”), mark the unused parts DNP in KiCad, or analyse the board as it is."}</p></div>
    <div class="ov-actions var-choices">${btns}</div>${forks}`;
}
function bindVariantButtons(box) {
  box.querySelectorAll("[data-variant]").forEach((b) => b.onclick = () => setVariant(b.dataset.variant));
}

// choose a variant (null: back to the board configuration's); the board is rebuilt in the
// engine, lanes and power results are computed again
function setVariant(name) {
  lsSet(variantKey(), name);
  showBusy("Applying the assembly variant…");
  requestAnimationFrame(() => setTimeout(() => {
    try { S.variant = S.engine.variant(name); } catch (e) { busyError(e.message); return; }
    S.boardKey = (S.boardKey || "").replace(/\|v=.*$/, "") + "|v=" + (name || "");
    S.buses = null; S.check = null; S.laneRes = null; S.chStatus = null; S.chCache = null; S.rfCache = null; S.selLane = null;
    initPi(S.lastFiles ? S.lastFiles.piDefaults : null);
    runAnalysis();
    renderVariantCard();
    if (S.tab === "ch") { renderIfcCard(); renderProfCard(); renderCheck(); setChMode(S.chMode || "o"); }
    else if (S.tab === "pi") renderPi();
    hideBusy();
  }, 0));
}

"use strict";
// ============================================================== state
const $ = (id) => document.getElementById(id);
const S = {
  engine: null, scene: null, an: null, zones: null, name: "",
  mode: "type", layerOn: [], zoneLayer: -1, overrides: {}, mask: true, step: 0.5,
  pieces: null, grid: null, selNet: -1, hover: -1,
  view: { s: 1, x: 0, y: 0 }, paths: null, selPaths: null, colors: {},
  field: new Map(), fieldPending: new Set(), pool: null, gen: 0,
  tab: "si", piMode: "b", piNets: [], piSetups: new Map(), piRes: new Map(), piRail: null, piLayer: -1,
  piImg: null, piWarn: [], piDefaults: null, piTree: null, treeRes: null,
  detailTab: "z", rfCache: null, anGen: 0,
};
const KIND_LABEL = { Microstrip: "Microstrip", Stripline: "Stripline", EmbeddedMicrostrip: "Microstrip, embedded (one reference)", NoReference: "No reference plane" };
const FLAGS = [
  [1, "No reference plane found"],
  [2, "Reference plane ends close to the trace"],
  [4, "Nearest reference is not on the adjacent layer"],
  [8, "Inner layer with only one reference plane"],
  [16, "Coplanar ground next to stripline (not modelled)"],
  [32, "Differential pair not coupled here"],
  [64, "Reference plane perforated here (e.g. via clearances)"],
  [128, "Cut-out in a layer between trace and reference (e.g. under a pad)"],
];
const netLabel = (n) => String(n).replace(/\{slash\}/g, "/");
// last part of a hierarchical net name ("/FPGA/+1V0" → "+1V0"; an escaped slash stays part of the name)
const netShort = (n) => netLabel(String(n).split("/").pop());
const fmt = (v, d = 1) => (v == null || !isFinite(v)) ? "–" : v.toLocaleString("en-US", { minimumFractionDigits: d, maximumFractionDigits: d });
const pct = (v) => v == null ? "–" : Math.round(v * 100) + " %";
const esc = (s) => String(s).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" }[c]));

function readColors() {
  const cs = getComputedStyle(document.documentElement);
  const g = (n) => cs.getPropertyValue(n).trim();
  S.colors = {
    canvas: g("--canvas"), edge: g("--canvas-edge"), pad: g("--canvas-pad"), via: g("--canvas-via"),
    zone: g("--canvas-zone"), zoneEdge: g("--canvas-zone-edge"), halo: g("--halo"),
    low2: g("--z-low2"), low1: g("--z-low1"), ok: g("--z-ok"), high1: g("--z-high1"), high2: g("--z-high2"),
    none: g("--z-none"), noref: g("--z-noref"), ms: g("--k-ms"), sl: g("--k-sl"), emb: g("--k-emb"),
  };
}

// Turbo colormap (Google, polynomial approximation)
function turbo(x) {
  x = Math.min(1, Math.max(0, x));
  const r = 0.13572138 + x * (4.6153926 + x * (-42.66032258 + x * (132.13108234 + x * (-152.94239396 + x * 59.28637943))));
  const g = 0.09140261 + x * (2.19418839 + x * (4.84296658 + x * (-14.18503333 + x * (4.27729857 + x * 2.82956604))));
  const b = 0.1066733 + x * (12.64194608 + x * (-60.58204836 + x * (110.36276771 + x * (-89.90310912 + x * 27.34824973))));
  const c = (v) => Math.round(255 * Math.min(1, Math.max(0, v)));
  return `rgb(${c(r)},${c(g)},${c(b)})`;
}
function turboRGB(x) {
  x = Math.min(1, Math.max(0, x));
  const r = 0.13572138 + x * (4.6153926 + x * (-42.66032258 + x * (132.13108234 + x * (-152.94239396 + x * 59.28637943))));
  const g = 0.09140261 + x * (2.19418839 + x * (4.84296658 + x * (-14.18503333 + x * (4.27729857 + x * 2.82956604))));
  const b = 0.1066733 + x * (12.64194608 + x * (-60.58204836 + x * (110.36276771 + x * (-89.90310912 + x * 27.34824973))));
  const c = (v) => Math.round(255 * Math.min(1, Math.max(0, v)));
  return [c(r), c(g), c(b)];
}
const ABS_MIN = 25, ABS_MAX = 125;
// browser storage (per viewer; may be unavailable)
const lsGet = (k) => { try { const t = localStorage.getItem(k); return t ? JSON.parse(t) : null; } catch (e) { return null; } };
const lsSet = (k, v) => { try { if (v == null) localStorage.removeItem(k); else localStorage.setItem(k, JSON.stringify(v)); } catch (e) { /* storage unavailable */ } };

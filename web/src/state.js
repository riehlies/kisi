"use strict";
// ============================================================== state
const $ = (id) => document.getElementById(id);
const S = {
  engine: null, scene: null, an: null, zones: null, name: "",
  mode: "dev", layerOn: [], zoneLayer: -1, overrides: {}, mask: true, step: 0.5,
  pieces: null, grid: null, selNet: -1, hover: -1,
  view: { s: 1, x: 0, y: 0 }, paths: null, selPaths: null, colors: {},
  field: new Map(), fieldPending: new Set(), pool: null, gen: 0,
  tab: "si", piMode: "v", piNets: [], piSetups: new Map(), piRes: new Map(), piRail: null, piLayer: -1,
  piImg: null, piWarn: [], piDefaults: null, piTree: null, treeRes: null,
  detailTab: "z", rfCache: null, anGen: 0,
};
const KIND_DE = { Microstrip: "Microstrip", Stripline: "Stripline", EmbeddedMicrostrip: "Microstrip, eingebettet (eine Referenz)", NoReference: "Keine Referenzebene" };
const FLAGS = [
  [1, "Keine Referenzebene gefunden"],
  [2, "Referenzebene endet nahe der Leiterbahn"],
  [4, "Nächste Referenz liegt nicht auf der Nachbarlage"],
  [8, "Innenlage mit nur einer Referenzebene"],
  [16, "Koplanare Masse neben Stripline (in M1 nicht modelliert)"],
  [32, "Diff-Paar hier nicht gekoppelt"],
  [64, "Referenzebene hier perforiert (z. B. Via-Freistellungen)"],
  [128, "Aussparung in einer Lage zwischen Leitung und Referenz (z. B. unter einem Pad)"],
];
const netLabel = (n) => String(n).replace(/\{slash\}/g, "/");
const fmt = (v, d = 1) => (v == null || !isFinite(v)) ? "–" : v.toLocaleString("de-DE", { minimumFractionDigits: d, maximumFractionDigits: d });
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

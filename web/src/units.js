"use strict";
// ============================================================== units
// Numbers with SI prefixes, shared by all views (and by tools/test_units.mjs). No dependencies.
//
//   fmtSI(1.2e-5, "A")        → "12 µA"      values switch their prefix (p n µ m – k M G) so that the
//   fmtSI(0.0234, "W")        → "23.4 mW"    mantissa lies in 1 … 999; 3 significant digits by default,
//   fmtSI(3.3, "V")           → "3.3 V"      trailing zeros dropped (opts.zeros keeps them: "3.30 V");
//   fmtSI(4.7e-6, "F", 2)     → "4.7 µF"     null, NaN, ±∞ → "–"; 0 → "0 A". Value and unit are joined
//   fmtSI(1.5e9, "Hz")        → "1.5 GHz"    by a no-break space.
//   parseSI("12u", "A")       → 1.2e-5       input with or without prefix and unit: "12 µA", "0.012 mA",
//   parseSI("1.2e-5")         → 1.2e-5       "12uA", "1,5m"; a plain number is the base unit; NaN if
//   parseSI("12 mV", "A")     → NaN          the text is no such value (a different unit, garbage).
//   fieldSI(1.2e-5, "A")      → "12uA"       for KiCad fields kisi parses (PI_LOAD, PI_REG): ASCII "u",
//                                            no space, prefixes µ, m, k only, 4 significant digits.
const SI_PREFIX = { "-12": "p", "-9": "n", "-6": "µ", "-3": "m", "0": "", "3": "k", "6": "M", "9": "G" };
const SI_FACTOR = { p: 1e-12, n: 1e-9, u: 1e-6, "µ": 1e-6, "μ": 1e-6, m: 1e-3, k: 1e3, K: 1e3, M: 1e6, G: 1e9 };

// value → [mantissa rounded to `digits` significant digits, exponent (multiple of 3)], or null
function siSplit(v, digits = 3, minExp = -12, maxExp = 9) {
  if (v == null || typeof v !== "number" || !isFinite(v)) return null;
  if (v === 0) return [0, 0];
  let e = Math.min(maxExp, Math.max(minExp, Math.floor(Math.log10(Math.abs(v)) / 3) * 3));
  let m = Number((v / 10 ** e).toPrecision(digits));
  // rounding can carry over to the next prefix: 999.7 µA → 1 mA
  if (Math.abs(m) >= 1000 && e < maxExp) { e += 3; m = Number((v / 10 ** e).toPrecision(digits)); }
  return [m, e];
}

function siMantissa(m, digits, zeros) {
  return m.toLocaleString("en-US", { maximumSignificantDigits: digits, minimumSignificantDigits: zeros ? digits : 1, useGrouping: false });
}

// "12 µA": value with a prefix and the unit; `digits` significant digits (default 3)
function fmtSI(v, unit = "", digits = 3, opts = {}) {
  const s = siSplit(v, digits);
  if (!s) return "–";
  const [m, e] = s;
  return `${siMantissa(m, digits, opts.zeros && m !== 0)} ${SI_PREFIX[e]}${unit}`;
}

// text typed by a person → value in the base unit (NaN if not readable)
function parseSI(text, unit = "") {
  let s = String(text == null ? "" : text).trim().replace(/\s+/g, "").replace(",", ".");
  const names = unit === "Ω" ? ["Ω", "ohms", "ohm", "R"] : unit ? [unit] : [];
  for (const n of names) {
    if (s.length > n.length && s.toLowerCase().endsWith(n.toLowerCase())) { s = s.slice(0, -n.length); break; }
  }
  const m = s.match(/^([+-]?(?:\d+(?:\.\d*)?|\.\d+)(?:[eE][+-]?\d+)?)([pnuµμmkKMG]?)$/);
  if (!m) return NaN;
  return Number(m[1]) * (m[2] ? SI_FACTOR[m[2]] : 1);
}

// "12uA": a value for a KiCad field that kisi's field parsers read (prefixes µ as "u", m, k)
function fieldSI(v, unit = "", digits = 4, minExp = -6, maxExp = 3) {
  const s = siSplit(v, digits, minExp, maxExp);
  if (!s) return "";
  const [m, e] = s;
  return `${siMantissa(m, digits, false)}${SI_PREFIX[e] === "µ" ? "u" : SI_PREFIX[e]}${unit}`;
}

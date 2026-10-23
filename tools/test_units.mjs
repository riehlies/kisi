// Tests of the viewer's unit formatter and parser (web/src/units.js): node tools/test_units.mjs
// The viewer's scripts share one classic scope, so the file is run in a VM context.
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import vm from "node:vm";
import assert from "node:assert/strict";

const src = readFileSync(fileURLToPath(new URL("../web/src/units.js", import.meta.url)), "utf8");
const ctx = vm.createContext({});
vm.runInContext(src + "\n;globalThis.U = { fmtSI, parseSI, fieldSI };", ctx);
const { fmtSI, parseSI, fieldSI } = ctx.U;
const nb = (s) => s.replace(/ /g, " ");
let n = 0;
const eq = (a, b, what) => { assert.equal(a, b, what); n++; };
const near = (a, b, what) => { assert.ok(Math.abs(a - b) <= 1e-12 * Math.max(1, Math.abs(b)), `${what}: ${a} vs ${b}`); n++; };

// formatter: prefixes, 3 significant digits, trailing zeros dropped
for (const [v, u, want] of [
  [1.2e-5, "A", "12 µA"], [0.000012, "A", "12 µA"], [0.0234, "W", "23.4 mW"], [3.3, "V", "3.3 V"], [5, "V", "5 V"],
  [0.1, "W", "100 mW"], [1, "A", "1 A"], [999.7e-6, "A", "1 mA"], [0.9996, "W", "1 W"], [12345, "W", "12.3 kW"],
  [2.5e6, "Hz", "2.5 MHz"], [1.5e9, "Hz", "1.5 GHz"], [4.7e-6, "F", "4.7 µF"], [1e-7, "F", "100 nF"], [2.2e-12, "F", "2.2 pF"],
  [0.0025, "Ω", "2.5 mΩ"], [600, "Ω", "600 Ω"], [4700, "Ω", "4.7 kΩ"], [-0.00132, "A", "-1.32 mA"], [0, "A", "0 A"],
  [1.23456e-9, "s", "1.23 ns"], [0.000171, "V", "171 µV"], [3.29983, "V", "3.3 V"],
]) eq(fmtSI(v, u), nb(want), `fmtSI(${v}, ${u})`);
eq(fmtSI(3.29983, "V", 5), nb("3.2998 V"), "5 digits");
eq(fmtSI(3.3, "V", 3, { zeros: true }), nb("3.30 V"), "zeros kept");
eq(fmtSI(0, "V", 3, { zeros: true }), nb("0 V"), "zero with zeros");
for (const v of [null, undefined, NaN, Infinity, "3"]) eq(fmtSI(v, "A"), "–", `fmtSI(${v})`);
// no "0.000"-style output anywhere in the range
for (let e = -12; e <= 9; e += 0.37) {
  const t = fmtSI(1.234 * 10 ** e, "W");
  assert.ok(!/^0\.0/.test(t) && !/e/.test(t), `${e}: ${t}`); n++;
}

// parser: with or without prefix and unit; a plain number is the base unit
for (const [t, u, want] of [
  ["12u", "A", 1.2e-5], ["12 µA", "A", 1.2e-5], ["12 μA", "A", 1.2e-5], ["12uA", "A", 1.2e-5], ["0.012 mA", "A", 1.2e-5],
  ["1.2e-5", "A", 1.2e-5], ["0.5", "A", 0.5], ["2 A", "A", 2], ["2a", "A", 2], ["1,5m", "A", 1.5e-3], [" 300 mA ", "A", 0.3],
  [".5m", "A", 5e-4], ["3k3", "Ω", NaN], ["4.7k", "Ω", 4700], ["10 mΩ", "Ω", 0.01], ["10 mohm", "Ω", 0.01], ["100n", "F", 1e-7],
  ["1M", "Ω", 1e6], ["2.5 GHz", "Hz", 2.5e9],
]) {
  const v = parseSI(t, u);
  if (Number.isNaN(want)) { assert.ok(Number.isNaN(v), `${t}: ${v}`); n++; } else near(v, want, `parseSI(${t}, ${u})`);
}
for (const t of ["", "abc", "12 mV", "12 µ A B", "1.2.3", "m", "12 x"]) { assert.ok(Number.isNaN(parseSI(t, "A")), `parseSI(${t})`); n++; }
// what the formatter writes, the parser reads back
for (const v of [1.2e-5, 0.0234, 3.3, 0.75e-3, 2.4e-6, 1.5, 47e-9]) near(parseSI(fmtSI(v, "A", 4), "A"), v, `round trip ${v}`);

// KiCad field values: ASCII "u", no space, only µ / m / – / k
for (const [v, u, want] of [
  [1.2e-5, "A", "12uA"], [0.012, "A", "12mA"], [0.75e-3, "A", "750uA"], [2.5, "A", "2.5A"], [0.6, "A", "600mA"], [5e-9, "A", "0.005uA"],
  [1.2345e-4, "A", "123.5uA"], [0.08, "", "80m"], [0, "A", "0A"],
]) eq(fieldSI(v, u), want, `fieldSI(${v})`);
eq(fieldSI(0.0005, "", 4, -3, 3), "0.5m", "ohms: milli at most");
for (const v of [1.2e-5, 0.012, 0.75e-3, 2.5]) near(parseSI(fieldSI(v, "A"), "A"), v, `field round trip ${v}`);

console.log(`units: ${n} checks passed`);

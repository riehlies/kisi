// Physics checks of the 3D field view of kisi-advanced.html: the quantities the page derives from kisi-fdtd's volume
// fields (web/advanced/fdtd/fdtd-fields.js) on runs of the native kisi-fdtd (bit-identical to its WebAssembly build),
// with the cases the page builds (web/advanced/fdtd/fdtd-case.js: KVF.makeCase, KVF.setRun).
//   node validation/fdtd/check_fields.mjs <kisi-fdtd> <work dir> [--json out.json] [--board case.json …]
// Cases: the page's default single via (10/20/30 GHz, ports 1 and 2 excited), the default differential pair (20 GHz,
// ports 1 and 2 excited) and board vias cut out by kisi's exporter (default: validation/fdtd/board_cases/
// BV_ts_refinout2.json and BV_vna_clkin_pad.json – its out side a pad port – on the page's mesh at 10 GHz, port 1). Runs are cached in the work dir by the case and
// the solver binary. Exit code 1 if a check fails. Tolerances and what limits them: next to each check;
// validation/vias.md "3D field view".
import vm from "node:vm";
import fs from "node:fs";
import path from "node:path";
import crypto from "node:crypto";
import { execFileSync } from "node:child_process";
const dir = path.dirname(new URL(import.meta.url).pathname);
const ROOT = path.join(dir, "..", "..");
for (const f of ["fdtd-case.js", "fdtd-fields.js", "fdtd-sparams.js"]) vm.runInThisContext(fs.readFileSync(path.join(ROOT, "web", "advanced", "fdtd", f), "utf8"), { filename: f });
const KVF = globalThis.KVF;
const args = process.argv.slice(2);
const [exe, work] = args;
const opt = (k) => { const i = args.indexOf(k); return i > 0 ? args[i + 1] : null; };
fs.mkdirSync(work, { recursive: true });
const SIGMA_CU = 1 / JSON.parse(fs.readFileSync(path.join(ROOT, "data", "thermal.json"), "utf8")).copper.rho_20c_ohm_m;

// a case as the page runs it (KVF.setRun: mesh, pulse to fmax, f_loss), each excited port natively
function runCase(p, freqs, ports, given) {
  const c = KVF.setRun(given ? KVF.prepareCase(given) : KVF.makeCase(p, freqs), freqs, (p && p.mesh) || "standard");
  const blobs = ports.map((port) => {
    const cc = Object.assign({}, c, { volume: { freqs, stride: 1, every: 0 }, volume_port: port });
    const txt = JSON.stringify(cc), h = crypto.createHash("sha1").update(txt + fs.statSync(exe).mtimeMs).digest("hex").slice(0, 12);
    const cp = path.join(work, `${c.name}_p${port}_${h}.json`), bp = cp.replace(/\.json$/, ".bin");
    if (!fs.existsSync(bp)) {
      fs.writeFileSync(cp, txt);
      const t0 = Date.now();
      execFileSync(exe, [cp, "--volume", bp], { stdio: ["ignore", "ignore", "ignore"] });
      console.log(`  ran ${path.basename(cp)} natively in ${((Date.now() - t0) / 1000).toFixed(1)} s`);
    }
    return new Uint8Array(fs.readFileSync(bp));
  });
  return KVF.fromRaw(blobs, c);
}

const results = [];
let failed = 0;
function check(name, value, ref, tol, unit, how) {
  const err = ref === 0 ? Math.abs(value) : Math.abs(value - ref) / Math.abs(ref);
  const ok = err <= tol;
  if (!ok) failed++;
  results.push({ name, value, ref, rel_err: err, tol, ok, how });
  console.log(`${ok ? "ok  " : "FAIL"} ${name}: ${fmt(value)} vs ${fmt(ref)} ${unit || ""} (${ref === 0 ? "abs" : "rel"} ${err.toExponential(2)}, tol ${tol})`);
}
const fmt = (v) => (Math.abs(v) < 1e-3 || Math.abs(v) >= 1e4 ? v.toExponential(4) : v.toFixed(5));
const nearest = (a, v) => { let k = 0; for (let i = 1; i < a.length; i++) if (Math.abs(a[i] - v) < Math.abs(a[k] - v)) k = i; return k; };
// ∫S·dA through x = x_i over the trace's side of its reference plane only (z ≥ zRef above, or z ≤ zRef below):
// the full plane also cuts the radial wave in the plane-pair cavity, which the S-parameters do not see at the ports
function traceFlux(ds, F, xi, zRef, above) {
  const kr = nearest(ds.z, zRef);
  const rect = above ? [1, ds.ny - 2, kr, ds.nz - 2] : [1, ds.ny - 2, 1, kr];
  return KVF.flux(ds, F, 0, nearest(ds.x, xi), rect);
}

// ---------------------------------------------------------------------------------------------------------------
console.log("single via (default parameters), 10/20/30 GHz, ports 1 and 2");
const sv = runCase({}, [10e9, 20e9, 30e9], [0, 1]);
const xIn = sv.case.ports[0].x / 2, xOut = sv.case.ports[1].x / 2;
for (let fi = 0; fi < sv.freqs.length; fi++) {
  const f = sv.freqs[fi], tag = `${f / 1e9} GHz`;
  const w = [[0, 1]], F = KVF.fieldsOf(sv, fi, w), s = KVF.summary(sv, fi, w, SIGMA_CU);
  // 1 V normalisation: port voltages V1 = 1 + S11 (1 V incident + reflected), V2 = S21
  const v1 = KVF.portVoltage(sv, F, 0), v2 = KVF.portVoltage(sv, F, 1), b = s.b;
  check(`${tag} |V port 1| = |1 + S11|`, Math.hypot(...v1), Math.hypot(1 + b[0][0], b[0][1]), 0.01, "V");
  check(`${tag} |V port 2| = |S21|`, Math.hypot(...v2), Math.hypot(...b[1]), 0.01, "V");
  check(`${tag} phase V port 2 − phase S21`, Math.atan2(v2[1], v2[0]) - Math.atan2(b[1][1], b[1][0]), 0, 0.01, "rad");
  // ∫S·dA through planes between the ports against the S-parameter powers
  const zt = sv.case.ports[0].z0, zb = sv.case.ports[1].z0; // the reference planes of the two feeds
  const p1 = traceFlux(sv, F, xIn, zt, true), p2 = traceFlux(sv, F, xOut, zb, false);
  check(`${tag} ∫S·dA at x = ${xIn} mm (z ≥ ${zt}) vs (|a1|² − |b1|²)·P`, p1, s.pin - s.refl, f > 25e9 ? 0.05 : 0.03, "W",
    "1 mm after the lumped port: its near field radiates 0.8 % (10 GHz) to 2.6 % (30 GHz) into the air, the feed loses 1.4 %/mm");
  check(`${tag} ∫S·dA at x = ${xOut} mm (z ≤ ${zb}) vs Σ|b_far|²·P`, p2, s.trans, 0.03, "W");
  const pf1 = KVF.flux(sv, F, 0, nearest(sv.x, xIn)), pf2 = KVF.flux(sv, F, 0, nearest(sv.x, xOut));
  console.log(`  whole planes x = ${xIn} / ${xOut} mm: ${fmt(pf1)} / ${fmt(pf2)} W (they also cut the cavity wave; outflow into the PML ${fmt(s.out)} W)`);
  // energy balance: incident − reflected − transmitted = dielectric loss + outflow into the PML
  check(`${tag} 1 − |S11|² − |S21|² vs dielectric loss + outflow`, s.diel.total + s.out, s.balance, 0.06, "W", `dielectric ${fmt(s.diel.total)} W alone; the rest leaves the box`);
  check(`${tag} the same, relative to the incident power`, (s.diel.total + s.out - s.balance) / s.pin, 0, 0.01, "");
  // conductor estimate: GND plane under the top feed against the image-current formula Rs·|I|²/(4πh) per length
  if (fi === 1) {
    const cl = s.cond, kz = nearest(sv.z, 1.4), m = cl.sheets.get(kz);
    let P = 0, Lx = 0;
    for (let i = 0; i < sv.nx - 1; i++) if (sv.sx.m[i] > -1.6 && sv.sx.m[i] < -0.8) {
      Lx += sv.sx.p[i];
      for (let j = 0; j < sv.ny - 1; j++) { const v = m[i * sv.ny + j]; if (v >= 0) P += v * sv.sx.p[i] * sv.sy.p[j]; }
    }
    // the trace current from Ampère's law around the top trace at x = −1.2 mm
    const xi = sv.x[nearest(sv.x, -1.2)];
    let Ir = 0, Ii = 0;
    const loop = [[-0.4, 1.5], [0.4, 1.5], [0.4, 1.7], [-0.4, 1.7], [-0.4, 1.5]];
    for (let s0 = 0; s0 < 4; s0++) for (let n = 0; n < 400; n++) {
      const [ya, za] = loop[s0], [yb, zb] = loop[s0 + 1], t = (n + 0.5) / 400;
      const y = ya + (yb - ya) * t, z = za + (zb - za) * t;
      const hy = KVF.interp(sv, F, 4, xi, y, z), hz = KVF.interp(sv, F, 5, xi, y, z);
      Ir += (hy[0] * (yb - ya) + hz[0] * (zb - za)) * 1e-3 / 400;
      Ii += (hy[1] * (yb - ya) + hz[1] * (zb - za)) * 1e-3 / 400;
    }
    const I = Math.hypot(Ir, Ii), h = 0.2e-3;
    const ref = (cl.Rs * I * I) / (4 * Math.PI * h);
    check(`${tag} conductor estimate: GND plane loss per length under the feed vs Rs·I²/(4πh)`, P / Lx, ref, 0.3, "W/m",
      `I = ${(I * 1e3).toFixed(2)} mA peak from ∮H·dl; the line-current image formula is an upper bound for a 0.39 mm strip`);
    const bar = cl.barrels.find((x) => x.via.net === "SIG");
    const refB = (cl.Rs * I * I) / (2 * Math.PI * bar.via.r * 1e-3) * ((bar.via.z1 - bar.via.z0) * 1e-3) / 2;
    check(`${tag} conductor estimate: signal barrel vs ½·Rs·I²·L/(2πr)`, bar.P, refB, 0.5, "W", "I taken equal to the feed current over the whole barrel (rough)");
    console.log(`  conductor loss estimate ${fmt(cl.total * 1e3)} mW of ${fmt(s.pin * 1e3)} mW (Rs ${(cl.Rs * 1e3).toFixed(1)} mΩ): ` +
      Object.entries(cl.regions).map(([k, v]) => `${k} ${fmt(v * 1e3)}`).join(", "));
  }
}
// local energy balance: the half box above a z plane through the core (port 1 inside, port 2 outside):
// power delivered by port 1 = outflow through the six faces (downward through the antipads included) + loss inside;
// the z planes cut the conformal barrels and antipads (flux weighted by the free part of each E edge)
for (let fi = 0; fi < sv.freqs.length; fi++) {
  const w = [[0, 1]], F = KVF.fieldsOf(sv, fi, w), s = KVF.summary(sv, fi, w, null), m = 2;
  for (const z0 of [0.8, 1.4]) {
    const k0 = nearest(sv.z, z0);
    const bb = KVF.boxBalance(sv, F, [m, sv.nx - 1 - m, m, sv.ny - 1 - m, k0, sv.nz - 1 - m]);
    check(`${sv.freqs[fi] / 1e9} GHz half box z ≥ ${sv.z[k0].toFixed(2)} mm: outflow + loss vs (1 − |S11|²)·P`, bb.outflow + bb.loss, s.pin - s.refl, 0.01, "W",
      `down through the plane ${fmt(-KVF.flux(sv, F, 2, k0))} W`);
  }
}
// animation: the period average of the instantaneous s(t) = e(t)×h(t) equals ½Re(E×H*) on a plane
{
  const F = KVF.fieldsOf(sv, 1, [[0, 1]]);
  for (const [axis, pos] of [[1, 0], [2, 0.8]]) {
    const pl = KVF.plane(sv, F, axis, nearest([sv.x, sv.y, sv.z][axis], pos));
    const avg = KVF.planeValues(pl, "S", 2, null).val;
    for (const N of [3, 8, 36]) {
      const acc = new Float64Array(avg.length);
      for (let n = 0; n < N; n++) { const v = KVF.planeValues(pl, "S", 2, (2 * Math.PI * n) / N).val; for (let p = 0; p < v.length; p++) acc[p] += v[p] / N; }
      let num = 0, den = 0;
      for (let p = 0; p < avg.length; p++) { num += (acc[p] - avg[p]) ** 2; den += avg[p] ** 2; }
      check(`animation: mean of s_z(t) over ${N} phases = ⟨S⟩z (plane ${"xyz"[axis]} = ${pos})`, Math.sqrt(num / den), 0, 1e-5, "(rel. rms)");
    }
    // |e(t)| never exceeds the fixed colour scale of the animation (the phasor magnitude)
    let over = 0;
    for (let n = 0; n < 24; n++) { const pv = KVF.planeValues(pl, "E", "mag", (2 * Math.PI * n) / 24); for (let p = 0; p < pv.val.length; p++) over = Math.max(over, pv.val[p] / (pv.bound[p] || 1) - 1); }
    check(`animation: |e(t)| ≤ |E| on plane ${"xyz"[axis]} = ${pos}`, Math.max(0, over), 0, 1e-6, "");
  }
}
// excitation of port 2 alone: the same checks backwards (reciprocity S12 = S21)
{
  const fi = 1, s2 = KVF.waves(sv, sv.freqs[fi], [[1, 1]]).b, s1 = KVF.waves(sv, sv.freqs[fi], [[0, 1]]).b;
  check("reciprocity |S12| = |S21| (20 GHz)", Math.hypot(...s2[0]), Math.hypot(...s1[1]), 0.005, "");
}

// the fallback frequency without an interface profile (1 GHz): the pulse to fmax = 10 GHz still carries it
{
  console.log("single via at 1 GHz");
  const lo = runCase({}, [1e9], [0]);
  const w = [[0, 1]], F = KVF.fieldsOf(lo, 0, w), s = KVF.summary(lo, 0, w, SIGMA_CU);
  check("1 GHz |V port 2| = |S21|", Math.hypot(...KVF.portVoltage(lo, F, 1)), Math.hypot(...s.b[1]), 0.01, "V");
  check("1 GHz 1 − |S11|² − |S21|² vs dielectric loss + outflow (rel. to incident)", (s.diel.total + s.out - s.balance) / s.pin, 0, 0.01, "");
}

// ---------------------------------------------------------------------------------------------------------------
console.log("differential pair (default parameters), 20 GHz, ports 1 and 2");
const pr = runCase({ kind: "pair", gnd: 2 }, [20e9], [0, 1]);
const ex = KVF.excitationsOf(pr), odd = ex.find((e) => e.id === "odd"), even = ex.find((e) => e.id === "even");
const sums = {};
for (const e of [odd, even, ex.find((e) => e.id === "p1"), ex.find((e) => e.id === "p2")]) {
  const F = KVF.fieldsOf(pr, 0, e.w), s = KVF.summary(pr, 0, e.w, SIGMA_CU);
  const p1 = traceFlux(pr, F, pr.case.ports[0].x / 2, pr.case.ports[0].z0, true), p2 = traceFlux(pr, F, pr.case.ports[2].x / 2, pr.case.ports[2].z0, false);
  console.log(`  ${e.id}: outflow into the PML ${fmt(s.out)} W, dielectric ${fmt(s.diel.total)} W, balance ${fmt(s.balance)} W`);
  sums[e.id] = { diel: s.diel.total, out: s.out, p1, p2, cond: s.cond.total };
  check(`pair ${e.id}: incident power`, s.pin, 0.01, 1e-12, "W", "1 V peak per single-ended port; modes (F1 ± F2)/√2");
  check(`pair ${e.id}: ∫S·dA between ports 1/2 and the vias vs P·(1 − |b1|² − |b2|²)`, p1, s.pin - s.refl, 0.03, "W");
  check(`pair ${e.id}: ∫S·dA between the vias and ports 3/4 vs P·(|b3|² + |b4|²)`, p2, s.trans, 0.03, "W");
  check(`pair ${e.id}: 1 − Σ|b|² vs dielectric loss + outflow (rel. to incident)`, (s.diel.total + s.out - s.balance) / s.pin, 0, 0.015, "",
    "the even mode sends 35 % into the plane pair; ending at −70 dB instead of −50 dB brings its closure from 1.2 % to 0.75 %");
  if (e.id === "odd") {
    const F2 = F, v1 = KVF.portVoltage(pr, F2, 0), v2 = KVF.portVoltage(pr, F2, 1);
    const b = s.b;
    check("pair odd: V port 1 = 1/√2 + b1 (|·|)", Math.hypot(...v1), Math.hypot(Math.SQRT1_2 + b[0][0], b[0][1]), 0.01, "V");
    check("pair odd: V port 2 = −1/√2 + b2 (|·|)", Math.hypot(...v2), Math.hypot(-Math.SQRT1_2 + b[1][0], b[1][1]), 0.01, "V");
  }
}
// odd/even are an orthonormal change of the two port excitations: quadratic totals add up the same
for (const k of ["diel", "out", "p1", "p2", "cond"]) check(`pair: ${k}(odd) + ${k}(even) = ${k}(port 1) + ${k}(port 2)`, sums.odd[k] + sums.even[k], sums.p1[k] + sums.p2[k], 1e-4, "W");

// ---------------------------------------------------------------------------------------------------------------
// KVF storage: f16 with 10 and 7 mantissa bits against f32
// the S-parameter and TDR views (fdtd-sparams.js): reciprocity of the broadband S of the two single-port runs, a
// lossless via passes nearly all power at low frequency, the TDR of a matched port stays at its reference
console.log("S-parameters and TDR");
{
  const m = KVF.sMatrix(sv), k = m.f.findIndex((f) => f >= 5e9), d = (a, b) => Math.hypot(a[0] - b[0], a[1] - b[1]);
  check("single: S21 = S12 of the two runs (reciprocity, 5 GHz)", d(m.S(1, 0)[k], m.S(0, 1)[k]), 0, 0.01, "", "time-domain runs of each port; the difference is the runs' truncation");
  // passive at every frequency (the loss off f_loss is not tanδ's: a fixed conductivity, see fdtd-sparams.js)
  const p = (x) => x[0] ** 2 + x[1] ** 2;
  check("single: max over f of |S11|² + |S21|² (passive)", Math.max(1, ...m.f.map((_, k) => p(m.S(0, 0)[k]) + p(m.S(1, 0)[k]))), 1, 1e-3, "");
  const z = KVF.tdr(m.f, m.f.map(() => [0, 0]), 50, 35e-12, 200e-12).z;
  check("TDR of S11 = 0 stays at the reference", Math.max(...z.map((x) => Math.abs(x - 50))), 0, 1e-9, "Ω");
  const ts = KVF.touchstone(m, ["check"]).trim().split("\n");
  check("the .s2p has one line per frequency", ts.length - 2, m.f.length, 0, "", "nine numbers each");
}
console.log("KVF round trip");
for (const mb of [10, 7]) {
  const back = KVF.decode(KVF.encode(sv, { mantissaBits: mb }));
  const F0 = KVF.fieldsOf(sv, 1, [[0, 1]]), F1 = KVF.fieldsOf(back, 1, [[0, 1]]);
  const a = KVF.flux(sv, F0, 0, nearest(sv.x, xOut)), b = KVF.flux(back, F1, 0, nearest(back.x, xOut));
  check(`KVF ${mb}-bit mantissa: ∫S·dA unchanged`, b, a, mb === 10 ? 2e-4 : 3e-3, "W");
  const d0 = KVF.dielLoss(sv, F0).total, d1 = KVF.dielLoss(back, F1).total;
  check(`KVF ${mb}-bit mantissa: dielectric loss unchanged`, d1, d0, mb === 10 ? 2e-4 : 3e-3, "W");
}
// ---------------------------------------------------------------------------------------------------------------
// board vias (kisi's exporter, kisi_core::viacase): port voltages against S, the closed balance and the conductor
// estimate's split (signal copper, return copper) on the page's mesh for a single via (coarse, finer for narrow traces,
// KVF.boardMesh) at 10 GHz, port 1 excited
const boards = args.flatMap((a, i) => (a === "--board" ? [args[i + 1]] : []));
// the second: a via whose trace ends at a capacitor pad inside the keep box (a pad port, kisi_core::viacase)
if (!boards.length) boards.push(...["BV_ts_refinout2.json", "BV_vna_clkin_pad.json"].map((f) => path.join(dir, "board_cases", f)));
for (const bp of boards) {
  const name = path.basename(bp, ".json");
  // the page's mesh for it: coarse, or finer for narrow traces (KVF.boardMesh)
  const given = KVF.prepareCase(JSON.parse(fs.readFileSync(bp, "utf8"))), mesh = KVF.boardMesh(given, "coarse");
  console.log(`board via ${name} (${mesh} mesh, 10 GHz, port 1)`);
  const bv = runCase({ mesh }, [10e9], [0], given);
  const w = [[0, 1]], F = KVF.fieldsOf(bv, 0, w), s = KVF.summary(bv, 0, w, SIGMA_CU), b = s.b;
  const far = bv.case.meta.ports.findIndex((q) => q.side > 0);
  const v1 = KVF.portVoltage(bv, F, 0), v2 = KVF.portVoltage(bv, F, far);
  check(`${name}: |V port 1| = |1 + S11|`, Math.hypot(...v1), Math.hypot(1 + b[0][0], b[0][1]), 0.01, "V");
  check(`${name}: |V far port| = |S21|`, Math.hypot(...v2), Math.hypot(...b[far]), 0.01, "V", "ports along x or y (KVF.alongY)");
  check(`${name}: 1 − Σ|b|² vs dielectric loss + outflow (rel. to incident)`, (s.diel.total + s.out - s.balance) / s.pin, 0, 0.015, "",
    "coarse mesh (0.07 mm) at the lumped ports; not the stop level: −50 / −60 dB instead of −40 dB give 1.2 / 1.0 % on BV_ts_refinout2");
  const cl = s.cond;
  check(`${name}: conductor estimate split into signal and return copper adds up`, Object.values(cl.regions).reduce((a, x) => a + x, 0), cl.total, 1e-9, "W");
  check(`${name}: the signal copper carries a share of the conductor estimate`, cl.regions["signal traces and pads"] > 0 && cl.regions["planes and other copper"] > 0 ? 1 : 0, 1, 0, "",
    `signal ${fmt(cl.regions["signal traces and pads"] * 1e3)} mW, return ${fmt(cl.regions["planes and other copper"] * 1e3)} mW of ${fmt(cl.total * 1e3)} mW`);
  results.push({ name: `${name}: run`, value: bv.runs[0].cells, ref: bv.runs[0].steps, ok: true, how: "cells, steps" });
}

const out = opt("--json");
if (out) fs.writeFileSync(out, JSON.stringify({ sigma_cu: SIGMA_CU, single: { nodes: sv.n, cells: sv.runs[0].cells, steps: sv.runs[0].steps }, pair: { nodes: pr.n, cells: pr.runs[0].cells, steps: pr.runs[0].steps }, checks: results }, null, 1));
console.log(`${results.length - failed} of ${results.length} checks passed`);
process.exit(failed ? 1 : 0);

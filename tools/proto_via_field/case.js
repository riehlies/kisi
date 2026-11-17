// Prototype (docs/research/fdtd-field-view.md, "Prototype viewer"): kisi-fdtd cases of a single via or a
// differential via pair from a few parameters, following validation/fdtd/via_cases.py (planes through the PML,
// zero-thickness copper, solid barrels, 50 Ω microstrip feeds from 50 Ω lumped ports). Plain script: attaches
// KVF.makeCase etc. to globalThis (page, worker and node via vm).
(function (root) {
  "use strict";
  const KVF = (root.KVF = root.KVF || {});
  // copper layers top (L1) to bottom, dielectrics between them (thickness mm, εr, tanδ); planes: copper indices
  const STACKUPS = {
    s4: { title: "4-layer 1.6 mm FR-4", diel: [[0.2, 4.3, 0.02], [1.2, 4.5, 0.02], [0.2, 4.3, 0.02]], planes: [1, 2] },
    s8: {
      title: "8-layer 1.6 mm FR-4 (generic)",
      diel: [[0.1, 3.8, 0.02], [0.2, 4.2, 0.02], [0.15, 3.8, 0.02], [0.7, 4.2, 0.02], [0.15, 3.8, 0.02], [0.2, 4.2, 0.02], [0.1, 3.8, 0.02]],
      planes: [1, 3, 4, 6],
    },
  };
  KVF.STACKUPS = STACKUPS;
  // a uniform stack: n copper layers, every inner layer a GND plane
  function uniformStack(n, t, er, tand) {
    n = Math.max(2, Math.round(n));
    const d = t / (n - 1);
    const diel = Array.from({ length: n - 1 }, () => [d, er, tand]);
    const planes = n > 2 ? Array.from({ length: n - 2 }, (_, i) => i + 1) : [1];
    return { title: `${n}-layer ${t} mm, εr ${er}, tanδ ${tand} (uniform)`, diel, planes, through: n > 2 ? n - 1 : 0 };
  }
  const r6 = (v) => Math.round(v * 1e6) / 1e6;
  function copperZ(st) {
    const z = [st.diel.reduce((a, d) => a + d[0], 0)];
    for (const d of st.diel) z.push(z[z.length - 1] - d[0]);
    return z.map(r6);
  }
  // Hammerstad–Jensen microstrip, zero thickness (as via_cases.py)
  function zMicrostrip(w, h, er) {
    const u = w / h;
    const a = 1 + Math.log((u ** 4 + (u / 52) ** 2) / (u ** 4 + 0.432)) / 49 + Math.log(1 + (u / 18.1) ** 3) / 18.7;
    const b = 0.564 * ((er - 0.9) / (er + 3)) ** 0.053;
    const ee = (er + 1) / 2 + ((er - 1) / 2) * (1 + 10 / u) ** (-a * b);
    const f = 6 + (2 * Math.PI - 6) * Math.exp(-((30.666 / u) ** 0.7528));
    return (60 / Math.sqrt(ee)) * Math.log(f / u + Math.sqrt(1 + (2 / u) ** 2));
  }
  function widthFor(z0, fz) {
    let lo = 0.01, hi = 3.0;
    for (let i = 0; i < 60; i++) {
      const mid = Math.sqrt(lo * hi);
      if (fz(mid) > z0) lo = mid;
      else hi = mid;
    }
    return Math.round(Math.sqrt(lo * hi) * 1000) / 1000;
  }
  KVF.zMicrostrip = zMicrostrip;

  // default parameters (single via: like validation case A_s4_d200_ap600 with a shorter feed)
  KVF.DEFAULTS = {
    kind: "single", stackup: "s4", layers: 4, thickness: 1.6, er: 4.3, tand: 0.02,
    drill: 0.2, pad: 0.5, antipad: 0.6, pitch: 1.0, gnd: 4, gndDist: 1.0, feed: 2.0, mesh: "standard",
  };
  KVF.MESH = { coarse: 0.08, standard: 0.05, fine: 0.035 };

  // p: parameters (KVF.DEFAULTS), freqs (Hz): the case for kisi-fdtd with a `meta` block for the viewer
  KVF.makeCase = function (p, freqs) {
    p = Object.assign({}, KVF.DEFAULTS, p);
    const st = p.stackup === "uniform" ? uniformStack(p.layers, p.thickness, p.er, p.tand) : STACKUPS[p.stackup];
    if (!st) throw new Error("unknown stackup " + p.stackup);
    const zc = copperZ(st), t = zc[0], nl = zc.length;
    const lin = 0, lout = nl - 1;
    const refIn = Math.min(...st.planes), refOut = Math.max(...st.planes);
    const fmaxSel = Math.max(...freqs);
    // the pulse reaches fmax; the mesh is λ/20 there (make_mesh): a little above the highest field frequency
    const fmax = Math.max(20e9, Math.ceil((1.25 * fmaxSel) / 5e9) * 5e9);
    const pair = p.kind === "pair";
    const vy = pair ? [p.pitch / 2, -p.pitch / 2] : [0];
    const air = 0.8, hx = p.feed + 1.0;
    const hy = r6(Math.max(2.0, (pair ? p.pitch / 2 : 0) + Math.max(p.gndDist, p.antipad / 2) + 1.2));
    const dom = [-hx, -hy, -air, hx, hy, r6(t + air)];
    const board = [-hx, -hy, hx, hy];
    const layers = [];
    {
      let z = t;
      for (const d of st.diel) {
        layers.push({ z0: r6(z - d[0]), z1: r6(z), er: d[1], tand: d[2] });
        z -= d[0];
      }
      layers.reverse();
    }
    const dielBetween = (a, b) => {
      let s = 0, w = 0;
      for (let i = Math.min(a, b); i < Math.max(a, b); i++) { s += st.diel[i][0] * st.diel[i][1]; w += st.diel[i][0]; }
      return s / w;
    };
    const wIn = widthFor(50, (w) => zMicrostrip(w, Math.abs(zc[lin] - zc[refIn]), dielBetween(lin, refIn)));
    const wOut = widthFor(50, (w) => zMicrostrip(w, Math.abs(zc[lout] - zc[refOut]), dielBetween(lout, refOut)));
    const dr = KVF.MESH[p.mesh] || +p.mesh || 0.05;
    const c = {
      name: (pair ? "pair" : "via") + `_${p.stackup}_d${Math.round(p.drill * 1000)}_ap${Math.round(p.antipad * 1000)}` +
        (pair ? `_p${Math.round(p.pitch * 1000)}` : ""),
      domain: dom.map(r6), board: board.map(r6), layers, sheets: [], vias: [], ports: [], fmax,
      f_loss: 0, mesh: { dfine: dr, ratio: 1.4, thirds: true }, boundary: "pml", pml_cells: 8, conformal: true,
      end_db: -40, max_steps: 60000, nf: 200,
    };
    const ports = [];
    const holes = [];
    for (const y of vy) {
      c.vias.push({ x: 0, y: r6(y), r: r6(p.drill / 2), z0: zc[lout], z1: zc[lin], pad_r: r6(p.pad / 2), pads: [zc[lin], zc[lout]], net: "SIG" });
      holes.push([0, r6(y), r6(p.antipad / 2)]);
    }
    // GND stitching vias (all planes connected)
    const gv = [];
    if (pair) {
      const yo = p.pitch / 2 + p.gndDist;
      if (p.gnd >= 2) gv.push([0, yo], [0, -yo]);
      if (p.gnd >= 4) gv.push([-p.gndDist, 0], [p.gndDist, 0]);
    } else {
      const d = p.gndDist / Math.SQRT2;
      if (p.gnd === 2) gv.push([0, p.gndDist], [0, -p.gndDist]);
      if (p.gnd >= 4) gv.push([d, d], [-d, d], [-d, -d], [d, -d]);
    }
    for (const [x, y] of gv) c.vias.push({ x: r6(x), y: r6(y), r: 0.15, z0: 0, z1: t, pad_r: 0.3, pads: [], net: "GND", dmesh: 0.1 });
    // feeds: left on L1 (ports 1[, 2]), right on the bottom layer (ports 2 or 3, 4)
    const names = pair ? ["1 P near", "2 N near", "3 P far", "4 N far"] : ["1", "2"];
    let pi = 0;
    for (const [side, layer, ref, w] of [[-1, lin, refIn, wIn], [1, lout, refOut, wOut]]) {
      for (const y of vy) {
        const xp = side * p.feed;
        c.sheets.push({ z: zc[layer], rect: [Math.min(0, xp), r6(y - w / 2), Math.max(0, xp), r6(y + w / 2)], net: "SIG" });
        c.ports.push({ x: r6(xp), y0: r6(y - w / 2), y1: r6(y + w / 2), z0: zc[ref], z1: zc[layer], r: 50.0 });
        ports.push({ name: names[pi++], layer: `L${layer + 1}`, ref: `L${ref + 1}`, side });
      }
    }
    for (const pl of st.planes) c.sheets.push({ z: zc[pl], rect: c.board.slice(), holes: holes.map((h) => h.slice()), net: "GND" });
    c.meta = {
      kind: p.kind, params: p, stackup: p.stackup, stackup_title: st.title, copper_z: zc,
      planes: st.planes.map((q) => `L${q + 1}`), ports, trace_w: [wIn, wOut],
      title: (pair ? `Differential via pair, pitch ${p.pitch} mm` : "Single via") +
        ` L1→L${nl}, drill ⌀${p.drill}, pad ⌀${p.pad}, antipad ⌀${p.antipad} mm, ${gv.length} GND vias (${st.title})`,
    };
    return c;
  };

  // excitations of a field run: [{label, ports: [[port index, weight]]}] (weights of the 1 V single-port runs)
  KVF.excitations = function (c) {
    if (c.meta && c.meta.kind === "pair") {
      const s = Math.SQRT1_2;
      return [
        { id: "odd", label: "odd (differential) mode at ports 1/2", ports: [[0, s], [1, -s]] },
        { id: "even", label: "even (common) mode at ports 1/2", ports: [[0, s], [1, s]] },
        { id: "p1", label: "port 1 (P near) alone", ports: [[0, 1]] },
        { id: "p2", label: "port 2 (N near) alone", ports: [[1, 1]] },
      ];
    }
    return c.ports.map((_, i) => ({ id: "p" + (i + 1), label: `port ${i + 1}`, ports: [[i, 1]] }));
  };
})(typeof globalThis !== "undefined" ? globalThis : this);

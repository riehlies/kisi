// Reference values from Henrik Forstén's 2D field solver (GPL-3.0), quasi-static backend.
// Usage: node ref.mjs <path-to-js_2d_fields> <cases.json>
// cases: [{name, type: 'microstrip'|'diff_microstrip'|'stripline'|'diff_stripline',
//          w, h, t, er, s?, er_top?, top_h?, sm_t?, sm_er?}]  (mm)
const [, , repo, casesPath] = process.argv;
const { buildSolverFromParams } = await import(repo + '/src/solver_factory.js');
const fs = await import('node:fs');
const cases = JSON.parse(fs.readFileSync(casesPath, 'utf8'));
const mm = 1e-3;
const out = [];
for (const c of cases) {
    const p = {
        tl_type: c.type,
        w: c.w * mm, h: c.h * mm, t: c.t * mm, er: c.er, tand: 0.0, sigma: 5.8e7,
        freq: 1e9, nx: 40, ny: 40, rq: 0, mesh_backend: 'rectilinear',
        trace_spacing: c.s ? c.s * mm : undefined,
        er_top: c.er_top ?? c.er, tand_top: 0, stripline_top_h: (c.top_h ?? 0) * mm,
        use_sm: !!c.sm_t, sm_t_sub: (c.sm_t ?? 0) * mm, sm_t_trace: (c.sm_t ?? 0) * mm,
        sm_t_side: (c.sm_t ?? 0) * mm, sm_er: c.sm_er ?? 3.3, sm_tand: 0,
    };
    const solver = buildSolverFromParams(p, (e) => console.error(c.name, e));
    if (!solver) { out.push({ name: c.name, error: true }); continue; }
    const log = console.log; console.log = () => {};
    let r;
    try {
        r = await solver.solve_adaptive({ max_iters: 8, energy_tol: 0.005, param_tol: 0.005, max_nodes: 40000, min_converged_passes: 2 });
    } finally { console.log = log; }
    const res = { name: c.name };
    if (r.modes.length === 2) {
        const odd = r.modes.find(m => m.mode === 'odd');
        const even = r.modes.find(m => m.mode === 'even');
        res.zdiff = 2 * odd.Z0; res.zodd = odd.Z0; res.zeven = even.Z0;
    } else {
        res.z0 = r.modes[0].Z0; res.eeff = r.modes[0].eps_eff;
    }
    out.push(res);
    console.log(JSON.stringify(res));
}
fs.writeFileSync(casesPath.replace(/\.json$/, '.out.json'), JSON.stringify(out, null, 1));

//! 3D FDTD (Yee scheme) for via transitions and small cut-outs of a PCB.
//!
//! A case (`Case`, JSON) describes a box: dielectric layers of the board, zero-thickness copper
//! sheets (traces, planes with antipads, pads), vias as solid cylinders and lumped ports. The
//! mesh is rectilinear and non-uniform (`mesh`), metal is staircased (an edge is metal when it
//! lies in a sheet or inside a cylinder), dielectric loss is a conductivity fixed at `f_loss`.
//! The outer boundary is Mur's first-order absorbing condition. Every port is excited in turn
//! with a Gaussian pulse up to `fmax`; voltages and currents at the ports give the S-parameters.
//!
//! This is the feasibility version: the reference is openEMS on the same mesh (same metal
//! rules, same boundary), see `validation/fdtd`.

#![allow(clippy::needless_range_loop, clippy::type_complexity, clippy::ptr_arg)]

use serde::{Deserialize, Serialize};

pub const C0: f64 = 299_792_458.0;
pub const MU0: f64 = 4e-7 * std::f64::consts::PI;
pub const EPS0: f64 = 1.0 / (MU0 * C0 * C0);

// --- case --------------------------------------------------------------------------------

/// Dielectric slab between z0 and z1 (mm), inside `Case::board`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Layer {
    pub z0: f64,
    pub z1: f64,
    pub er: f64,
    #[serde(default)]
    pub tand: f64,
}

/// Zero-thickness copper at height z: a rectangle with circular holes (antipads).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Sheet {
    pub z: f64,
    pub rect: [f64; 4],
    #[serde(default)]
    pub holes: Vec<[f64; 3]>,
}

/// Solid cylinder along z (via barrel); `pads`: circular pads (radius) at these heights.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Via {
    pub x: f64,
    pub y: f64,
    pub r: f64,
    pub z0: f64,
    pub z1: f64,
    #[serde(default)]
    pub pad_r: f64,
    #[serde(default)]
    pub pads: Vec<f64>,
}

/// Lumped port along z at x, over y0..y1, from z0 (reference) to z1 (signal).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Port {
    pub x: f64,
    pub y0: f64,
    pub y1: f64,
    pub z0: f64,
    pub z1: f64,
    #[serde(default = "r50")]
    pub r: f64,
}
fn r50() -> f64 {
    50.0
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MeshSpec {
    /// largest cell (mm); default λ/20 in the densest dielectric at fmax
    #[serde(default)]
    pub dmax: f64,
    /// cell at metal edges, vias and ports (mm)
    pub dfine: f64,
    #[serde(default = "ratio")]
    pub ratio: f64,
}
fn ratio() -> f64 {
    1.4
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Case {
    pub name: String,
    /// simulation box [x0, y0, z0, x1, y1, z1] (mm)
    pub domain: [f64; 6],
    /// extent of the dielectric layers [x0, y0, x1, y1] (mm)
    pub board: [f64; 4],
    pub layers: Vec<Layer>,
    #[serde(default)]
    pub sheets: Vec<Sheet>,
    #[serde(default)]
    pub vias: Vec<Via>,
    pub ports: Vec<Port>,
    pub fmax: f64,
    /// frequency where tanδ gives the conductivity (Hz); default fmax/2
    #[serde(default)]
    pub f_loss: f64,
    pub mesh: MeshSpec,
    /// stop when the field energy fell this far below its maximum (dB)
    #[serde(default = "end_db")]
    pub end_db: f64,
    #[serde(default = "max_steps")]
    pub max_steps: usize,
    /// ports to excite (default: all)
    #[serde(default)]
    pub excite: Option<Vec<usize>>,
    /// frequency points of the result
    #[serde(default = "nf")]
    pub nf: usize,
    /// threads for the field updates (native; 1 in the browser)
    #[serde(default = "one")]
    pub threads: usize,
}
fn one() -> usize {
    1
}
fn end_db() -> f64 {
    -50.0
}
fn max_steps() -> usize {
    400_000
}
fn nf() -> usize {
    201
}

// --- mesh --------------------------------------------------------------------------------

/// Mesh lines in mm, sorted.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mesh {
    pub x: Vec<f64>,
    pub y: Vec<f64>,
    pub z: Vec<f64>,
}

/// Lines between fixed lines: spacing `fine` at fine lines (`dmax` at the others), growing by
/// `ratio` towards the middle of each gap, at most `dmax`. Fixed lines closer than 0.6·fine merge.
fn grade(fixed: &mut Vec<(f64, bool)>, fine: f64, dmax: f64, ratio: f64) -> Vec<f64> {
    fixed.sort_by(|a, b| a.0.total_cmp(&b.0));
    // clusters of lines closer than 0.6·fine become one line at their mean (keeps symmetry);
    // the domain bounds (first and last) stay where they are
    let (lo, hi) = (fixed[0].0, fixed[fixed.len() - 1].0);
    // (first, last, sum, count, fine); a cluster of equal values keeps the exact value (a sheet
    // must lie exactly on its line, the mean of 1.4 three times is not 1.4)
    let mut groups: Vec<(f64, f64, f64, usize, bool)> = Vec::new();
    for &(x, is_fine) in fixed.iter() {
        match groups.last_mut() {
            Some(g) if x - g.0 < 0.6 * fine => {
                g.1 = x;
                g.2 += x;
                g.3 += 1;
                g.4 |= is_fine;
            }
            _ => groups.push((x, x, x, 1, is_fine)),
        }
    }
    let n = groups.len();
    let f: Vec<(f64, bool)> = groups
        .iter()
        .enumerate()
        .map(|(i, g)| {
            let v = if i == 0 {
                lo
            } else if i == n - 1 {
                hi
            } else if g.1 - g.0 < 1e-9 {
                g.0
            } else {
                g.2 / g.3 as f64
            };
            (v, g.4)
        })
        .collect();
    let mut out = vec![f[0].0];
    for w in f.windows(2) {
        let (a, b) = (w[0].0, w[1].0);
        let len = b - a;
        let (mut sl, mut sr) = (if w[0].1 { fine } else { dmax }, if w[1].1 { fine } else { dmax });
        let (mut left, mut right) = (Vec::new(), Vec::new());
        let mut covered = 0.0;
        while covered + sl.min(sr) < len {
            if sl <= sr {
                left.push(sl);
                covered += sl;
                sl = (sl * ratio).min(dmax);
            } else {
                right.push(sr);
                covered += sr;
                sr = (sr * ratio).min(dmax);
            }
        }
        let rem = len - covered;
        let mut cells: Vec<f64> = left;
        if rem > 0.5 * sl.min(sr) || cells.len() + right.len() == 0 {
            cells.push(rem);
        }
        cells.extend(right.iter().rev());
        let sum: f64 = cells.iter().sum();
        let mut x = a;
        for c in &cells[..cells.len() - 1] {
            x += c * len / sum;
            out.push(x);
        }
        out.push(b);
    }
    out
}

pub fn make_mesh(c: &Case) -> Mesh {
    let d = &c.domain;
    let er_max = c.layers.iter().map(|l| l.er).fold(1.0, f64::max);
    let dmax = if c.mesh.dmax > 0.0 { c.mesh.dmax } else { C0 / c.fmax / er_max.sqrt() / 20.0 * 1e3 };
    let fine = c.mesh.dfine;
    let mut fx: Vec<(f64, bool)> = vec![(d[0], false), (d[3], false), (c.board[0], true), (c.board[2], true)];
    let mut fy: Vec<(f64, bool)> = vec![(d[1], false), (d[4], false), (c.board[1], true), (c.board[3], true)];
    let mut fz: Vec<(f64, bool)> = vec![(d[2], false), (d[5], false)];
    for l in &c.layers {
        fz.push((l.z0, true));
        fz.push((l.z1, true));
    }
    for s in &c.sheets {
        fz.push((s.z, true));
        fx.push((s.rect[0], true));
        fx.push((s.rect[2], true));
        fy.push((s.rect[1], true));
        fy.push((s.rect[3], true));
        for h in &s.holes {
            for dd in [-h[2], 0.0, h[2]] {
                fx.push((h[0] + dd, true));
                fy.push((h[1] + dd, true));
            }
        }
    }
    for v in &c.vias {
        for dd in [-v.pad_r, -v.r, 0.0, v.r, v.pad_r] {
            if dd.abs() > 0.0 || dd == 0.0 {
                fx.push((v.x + dd, true));
                fy.push((v.y + dd, true));
            }
        }
        fz.push((v.z0, true));
        fz.push((v.z1, true));
    }
    for p in &c.ports {
        fx.push((p.x, true));
        fy.push((p.y0, true));
        fy.push((p.y1, true));
        fz.push((p.z0, true));
        fz.push((p.z1, true));
    }
    let clip = |v: &mut Vec<(f64, bool)>, lo: f64, hi: f64| v.retain(|x| x.0 >= lo - 1e-9 && x.0 <= hi + 1e-9);
    clip(&mut fx, d[0], d[3]);
    clip(&mut fy, d[1], d[4]);
    clip(&mut fz, d[2], d[5]);
    Mesh { x: grade(&mut fx, fine, dmax, c.mesh.ratio), y: grade(&mut fy, fine, dmax, c.mesh.ratio), z: grade(&mut fz, fine, dmax, c.mesh.ratio) }
}

// --- simulation --------------------------------------------------------------------------

struct Grid {
    nx: usize,
    ny: usize,
    nz: usize,
    // primary spacings (m), length n-1, and dual spacings (m), length n (ends: half cells)
    dx: Vec<f64>,
    dy: Vec<f64>,
    dz: Vec<f64>,
    dxd: Vec<f64>,
    dyd: Vec<f64>,
    dzd: Vec<f64>,
}

impl Grid {
    fn new(m: &Mesh) -> Self {
        let prim = |v: &[f64]| v.windows(2).map(|w| (w[1] - w[0]) * 1e-3).collect::<Vec<f64>>();
        let dual = |p: &[f64]| {
            let n = p.len() + 1;
            (0..n)
                .map(|i| {
                    let a = if i > 0 { p[i - 1] } else { 0.0 };
                    let b = if i < p.len() { p[i] } else { 0.0 };
                    0.5 * (a + b)
                })
                .collect::<Vec<f64>>()
        };
        let (dx, dy, dz) = (prim(&m.x), prim(&m.y), prim(&m.z));
        Grid { nx: m.x.len(), ny: m.y.len(), nz: m.z.len(), dxd: dual(&dx), dyd: dual(&dy), dzd: dual(&dz), dx, dy, dz }
    }
    #[inline]
    fn id(&self, i: usize, j: usize, k: usize) -> usize {
        (i * self.ny + j) * self.nz + k
    }
}

/// Per-port result of one excitation run.
#[derive(Debug, Clone, Serialize)]
pub struct RunResult {
    pub excited: usize,
    pub steps: usize,
    pub dt: f64,
    pub seconds: f64,
    /// S[i] = b_i / a_excited over `freq`, as (re, im)
    pub s: Vec<Vec<(f64, f64)>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Result {
    pub name: String,
    pub mesh: Mesh,
    pub cells: usize,
    pub freq: Vec<f64>,
    pub runs: Vec<RunResult>,
}

fn gauss(t: f64, f0: f64, fc: f64) -> f64 {
    // openEMS' Gaussian excitation: modulated, centred at 9 / (2π fc)
    let t0 = 9.0 / (2.0 * std::f64::consts::PI * fc);
    let s = 3.0 / (2.0 * std::f64::consts::PI * fc);
    (2.0 * std::f64::consts::PI * f0 * (t - t0)).cos() * (-((t - t0) / s).powi(2)).exp()
}

struct PortEdges {
    /// (index of Ez, Δz m, dual area m², ε)
    edges: Vec<(usize, f64, f64, f64)>,
    /// +1: signal above the reference, -1: below
    s: f64,
    nser: usize,
    npar: usize,
    r: f64,
}

/// One run with port `ex` excited; returns V(t), I(t) for all ports.
fn run(c: &Case, m: &Mesh, ex: usize, on_progress: &mut dyn FnMut(usize, f64)) -> (f64, usize, Vec<Vec<f64>>, Vec<Vec<f64>>, Vec<f64>) {
    let g = Grid::new(m);
    let (nx, ny, nz) = (g.nx, g.ny, g.nz);
    let n = nx * ny * nz;
    let dmin = |v: &[f64]| v.iter().cloned().fold(f64::INFINITY, f64::min);
    let dt = 0.99 / (C0 * (1.0 / dmin(&g.dx).powi(2) + 1.0 / dmin(&g.dy).powi(2) + 1.0 / dmin(&g.dz).powi(2)).sqrt());
    let f_loss = if c.f_loss > 0.0 { c.f_loss } else { c.fmax / 2.0 };
    // relative permittivity and conductivity of the cell (i, j, k) (between lines)
    let in_board = |x: f64, y: f64| x >= c.board[0] && x <= c.board[2] && y >= c.board[1] && y <= c.board[3];
    let cell_mat = |x: f64, y: f64, z: f64| -> (f64, f64) {
        if in_board(x, y) {
            for l in &c.layers {
                if z >= l.z0 && z <= l.z1 {
                    return (l.er, 2.0 * std::f64::consts::PI * f_loss * EPS0 * l.er * l.tand);
                }
            }
        }
        (1.0, 0.0)
    };
    let cx = |i: usize| 0.5 * (m.x[i] + m.x[i + 1]);
    let cy = |j: usize| 0.5 * (m.y[j] + m.y[j + 1]);
    let cz = |k: usize| 0.5 * (m.z[k] + m.z[k + 1]);
    // average of the cells around an edge, weighted by the dual area each one contributes
    let avg = |pts: &[(f64, f64, f64, f64)]| -> (f64, f64) {
        let (mut e, mut s, mut w) = (0.0, 0.0, 0.0);
        for &(x, y, z, a) in pts {
            let (er, sg) = cell_mat(x, y, z);
            e += er * a;
            s += sg * a;
            w += a;
        }
        if w > 0.0 {
            (e / w, s / w)
        } else {
            (1.0, 0.0)
        }
    };
    // metal tests
    let in_sheet = |x: f64, y: f64, z: f64| -> bool {
        c.sheets.iter().any(|s| {
            (s.z - z).abs() < 1e-9 && x >= s.rect[0] - 1e-9 && x <= s.rect[2] + 1e-9 && y >= s.rect[1] - 1e-9 && y <= s.rect[3] + 1e-9 && !s.holes.iter().any(|h| (x - h[0]).hypot(y - h[1]) < h[2] - 1e-9)
        }) || c.vias.iter().any(|v| v.pads.iter().any(|&pz| (pz - z).abs() < 1e-9) && (x - v.x).hypot(y - v.y) <= v.pad_r + 1e-9)
    };
    let in_via = |x: f64, y: f64, z: f64| c.vias.iter().any(|v| z >= v.z0 - 1e-9 && z <= v.z1 + 1e-9 && (x - v.x).hypot(y - v.y) <= v.r + 1e-9);
    // coefficients
    let mut ca = [vec![0f32; n], vec![0f32; n], vec![0f32; n]];
    let mut cb = [vec![0f32; n], vec![0f32; n], vec![0f32; n]];
    let mut epsr = [vec![1f32; n], vec![1f32; n], vec![1f32; n]];
    let coef = |er: f64, sg: f64| -> (f32, f32) {
        let eps = EPS0 * er;
        let a = sg * dt / (2.0 * eps);
        (((1.0 - a) / (1.0 + a)) as f32, ((dt / eps) / (1.0 + a)) as f32)
    };
    for i in 0..nx {
        for j in 0..ny {
            for k in 0..nz {
                let id = g.id(i, j, k);
                let (x, y, z) = (m.x[i], m.y[j], m.z[k]);
                // Ex: edge (i..i+1, j, k)
                if i + 1 < nx && j > 0 && j + 1 < ny && k > 0 && k + 1 < nz {
                    let xm = cx(i);
                    let metal = in_sheet(xm, y, z) || in_via(xm, y, z);
                    if !metal {
                        let pts = [(xm, cy(j - 1), cz(k - 1), g.dy[j - 1] * g.dz[k - 1]), (xm, cy(j), cz(k - 1), g.dy[j] * g.dz[k - 1]), (xm, cy(j - 1), cz(k), g.dy[j - 1] * g.dz[k]), (xm, cy(j), cz(k), g.dy[j] * g.dz[k])];
                        let (er, sg) = avg(&pts);
                        let (a, b) = coef(er, sg);
                        ca[0][id] = a;
                        cb[0][id] = b;
                        epsr[0][id] = er as f32;
                    }
                }
                if j + 1 < ny && i > 0 && i + 1 < nx && k > 0 && k + 1 < nz {
                    let ym = cy(j);
                    let metal = in_sheet(x, ym, z) || in_via(x, ym, z);
                    if !metal {
                        let pts = [(cx(i - 1), ym, cz(k - 1), g.dx[i - 1] * g.dz[k - 1]), (cx(i), ym, cz(k - 1), g.dx[i] * g.dz[k - 1]), (cx(i - 1), ym, cz(k), g.dx[i - 1] * g.dz[k]), (cx(i), ym, cz(k), g.dx[i] * g.dz[k])];
                        let (er, sg) = avg(&pts);
                        let (a, b) = coef(er, sg);
                        ca[1][id] = a;
                        cb[1][id] = b;
                        epsr[1][id] = er as f32;
                    }
                }
                if k + 1 < nz && i > 0 && i + 1 < nx && j > 0 && j + 1 < ny {
                    let zm = cz(k);
                    let metal = in_via(x, y, zm);
                    if !metal {
                        let pts = [(cx(i - 1), cy(j - 1), zm, g.dx[i - 1] * g.dy[j - 1]), (cx(i), cy(j - 1), zm, g.dx[i] * g.dy[j - 1]), (cx(i - 1), cy(j), zm, g.dx[i - 1] * g.dy[j]), (cx(i), cy(j), zm, g.dx[i] * g.dy[j])];
                        let (er, sg) = avg(&pts);
                        let (a, b) = coef(er, sg);
                        ca[2][id] = a;
                        cb[2][id] = b;
                        epsr[2][id] = er as f32;
                    }
                }
            }
        }
    }
    // ports: Ez edges, lumped resistor (and source) per edge
    let near = |v: &[f64], x: f64| v.iter().enumerate().min_by(|a, b| (a.1 - x).abs().total_cmp(&(b.1 - x).abs())).unwrap().0;
    let ports: Vec<PortEdges> = c
        .ports
        .iter()
        .map(|p| {
            let i = near(&m.x, p.x);
            let (j0, j1) = (near(&m.y, p.y0), near(&m.y, p.y1));
            let (k0, k1) = (near(&m.z, p.z0.min(p.z1)), near(&m.z, p.z0.max(p.z1)));
            let (nser, npar) = (k1 - k0, j1 - j0 + 1);
            let mut edges = Vec::new();
            for j in j0..=j1 {
                for k in k0..k1 {
                    let id = g.id(i, j, k);
                    edges.push((id, g.dz[k], g.dxd[i] * g.dyd[j], EPS0 * epsr[2][id] as f64));
                }
            }
            PortEdges { edges, s: if p.z1 >= p.z0 { 1.0 } else { -1.0 }, nser, npar, r: p.r }
        })
        .collect();
    // port edges: their own update (ca, cb set to 0 here, handled below)
    for p in &ports {
        for &(id, ..) in &p.edges {
            ca[2][id] = 0.0;
            cb[2][id] = 0.0;
        }
    }
    let mut e = [vec![0f32; n], vec![0f32; n], vec![0f32; n]];
    let mut h = [vec![0f32; n], vec![0f32; n], vec![0f32; n]];
    let inv = |v: &[f64]| v.iter().map(|x| (1.0 / x) as f32).collect::<Vec<f32>>();
    let (idx, idy, idz) = (inv(&g.dx), inv(&g.dy), inv(&g.dz));
    let (idxd, idyd, idzd) = (inv(&g.dxd), inv(&g.dyd), inv(&g.dzd));
    let hc = (dt / MU0) as f32;
    let fc = c.fmax / 2.0;
    let f0 = c.fmax / 2.0;
    let mut vs: Vec<Vec<f64>> = vec![Vec::new(); ports.len()];
    let mut is: Vec<Vec<f64>> = vec![Vec::new(); ports.len()];
    let mut exc = Vec::new();
    // Mur: previous boundary and neighbour values
    let t_pulse = 2.0 * 9.0 / (2.0 * std::f64::consts::PI * fc);
    let mut emax = 0.0f64;
    let mut steps = 0;
    let (sy, sx) = (nz, ny * nz);
    let mut prev: Vec<f32> = Vec::new();
    let threads = if cfg!(target_arch = "wasm32") { 1 } else { c.threads.max(1) };
    for step in 0..c.max_steps {
        let t = step as f64 * dt;
        // --- H update
        par_update(&g, threads, &mut h, |hs, i0, i1| update_h(&g, hs, i0, i1, &e, hc, &idx, &idy, &idz));
        // --- save E next to the boundary for Mur
        prev.clear();
        mur_save(&g, &e, &mut prev);
        // --- E update (interior); the port edges keep their old value for their own update
        let saved: Vec<Vec<f32>> = ports.iter().map(|p| p.edges.iter().map(|x| e[2][x.0]).collect()).collect();
        par_update(&g, threads, &mut e, |es, i0, i1| update_e(&g, es, i0, i1, &h, &ca, &cb, &idxd, &idyd, &idzd));
        // --- lumped ports
        let src = gauss(t + dt, f0, fc);
        exc.push(src);
        for (pi, p) in ports.iter().enumerate() {
            // nser edges in series per column, npar columns in parallel: R_edge·nser/npar = R
            let re = p.r * p.npar as f64 / p.nser as f64;
            let vse = if pi == ex { src / p.nser as f64 } else { 0.0 };
            let (mut v, mut cur) = (0.0, 0.0);
            for (n_e, &(id, dz, a, eps)) in p.edges.iter().enumerate() {
                let i_ = id / sx;
                let j_ = (id / sy) % ny;
                let curl = ((h[1][id] - h[1][id - sx]) * idxd[i_] - (h[0][id] - h[0][id - sy]) * idyd[j_]) as f64;
                let g1 = eps / dt;
                let g2 = dz / (2.0 * re * a);
                let old = saved[pi][n_e] as f64;
                // source in series with the resistor, from the reference to the signal side:
                // J_z = (s·Vs + E·Δz) / (R·A), V = −s·ΣE·Δz, current into the network s·J_z·A
                let sv = p.s * vse;
                let new = (old * (g1 - g2) + curl - sv / (re * a)) / (g1 + g2);
                e[2][id] = new as f32;
                v += -p.s * new * dz;
                cur += p.s * (sv + 0.5 * (new + old) * dz) / re;
            }
            vs[pi].push(v / p.npar as f64);
            is[pi].push(cur / p.nser as f64);
        }
        // --- Mur boundaries
        mur_apply(&g, &mut e, &prev, &epsr, dt);
        steps = step + 1;
        // --- energy check
        if step % 200 == 0 {
            // field energy ~ Σ ε E² + μ0 H² (without the cell volumes; only the decay matters)
            let mut en = 0.0f64;
            for c3 in 0..3 {
                en += e[c3].iter().zip(&epsr[c3]).map(|(x, r)| EPS0 * (*r as f64) * (*x as f64).powi(2)).sum::<f64>();
                en += h[c3].iter().map(|x| MU0 * (*x as f64).powi(2)).sum::<f64>();
            }
            emax = emax.max(en);
            on_progress(step, if emax > 0.0 { 10.0 * (en / emax).max(1e-30).log10() } else { 0.0 });
            if t > t_pulse && emax > 0.0 && 10.0 * (en / emax).max(1e-30).log10() < c.end_db {
                break;
            }
        }
    }
    (dt, steps, vs, is, exc)
}

/// Split the three field arrays into slabs of whole i-planes and update them on `threads`
/// threads (scoped; the other field is only read).
fn par_update<F>(g: &Grid, threads: usize, f3: &mut [Vec<f32>; 3], f: F)
where
    F: Fn([&mut [f32]; 3], usize, usize) + Sync,
{
    let sx = g.ny * g.nz;
    let t = threads.clamp(1, g.nx);
    let [a, b, c] = f3;
    if t == 1 {
        f([&mut a[..], &mut b[..], &mut c[..]], 0, g.nx);
        return;
    }
    let bounds: Vec<usize> = (0..=t).map(|k| k * g.nx / t).collect();
    let (mut ra, mut rb, mut rc) = (&mut a[..], &mut b[..], &mut c[..]);
    let mut parts = Vec::new();
    for k in 0..t {
        let len = (bounds[k + 1] - bounds[k]) * sx;
        let (pa, qa) = ra.split_at_mut(len);
        let (pb, qb) = rb.split_at_mut(len);
        let (pc, qc) = rc.split_at_mut(len);
        parts.push(([pa, pb, pc], bounds[k], bounds[k + 1]));
        ra = qa;
        rb = qb;
        rc = qc;
    }
    std::thread::scope(|s| {
        for (sl, i0, i1) in parts {
            let f = &f;
            s.spawn(move || f(sl, i0, i1));
        }
    });
}

/// H from the curl of E on every face of the primary grid (row slices along z for vectorising).
#[allow(clippy::too_many_arguments)]
fn update_h(g: &Grid, h: [&mut [f32]; 3], i0: usize, i1: usize, e: &[Vec<f32>; 3], hc: f32, idx: &[f32], idy: &[f32], idz: &[f32]) {
    let (nx, ny, nz) = (g.nx, g.ny, g.nz);
    let (sy, sx) = (nz, ny * nz);
    let [hx, hy, hz] = h;
    let [ex, ey, ez] = e;
    let off = i0 * sx;
    for i in i0..i1 {
        for j in 0..ny {
            let b = g.id(i, j, 0);
            let o = b - off;
            let (ix, iy) = (if i + 1 < nx { idx[i] } else { 0.0 }, if j + 1 < ny { idy[j] } else { 0.0 });
            if i + 1 < nx && j + 1 < ny {
                // Hz(i+½, j+½, k)
                let (hzr, eyr, eyx, exr, exy) = (&mut hz[o..o + nz], &ey[b..b + nz], &ey[b + sx..b + sx + nz], &ex[b..b + nz], &ex[b + sy..b + sy + nz]);
                for k in 0..nz {
                    hzr[k] -= hc * ((eyx[k] - eyr[k]) * ix - (exy[k] - exr[k]) * iy);
                }
            }
            if j + 1 < ny {
                // Hx(i, j+½, k+½)
                let (hxr, ezr, ezy, eyr) = (&mut hx[o..o + nz], &ez[b..b + nz], &ez[b + sy..b + sy + nz], &ey[b..b + nz]);
                for k in 0..nz - 1 {
                    hxr[k] -= hc * ((ezy[k] - ezr[k]) * iy - (eyr[k + 1] - eyr[k]) * idz[k]);
                }
            }
            if i + 1 < nx {
                // Hy(i+½, j, k+½)
                let (hyr, exr, ezr, ezx) = (&mut hy[o..o + nz], &ex[b..b + nz], &ez[b..b + nz], &ez[b + sx..b + sx + nz]);
                for k in 0..nz - 1 {
                    hyr[k] -= hc * ((exr[k + 1] - exr[k]) * idz[k] - (ezx[k] - ezr[k]) * ix);
                }
            }
        }
    }
}

/// E from the curl of H on the interior edges (ports and boundaries are updated separately).
#[allow(clippy::too_many_arguments)]
fn update_e(g: &Grid, e: [&mut [f32]; 3], i0: usize, i1: usize, h: &[Vec<f32>; 3], ca: &[Vec<f32>; 3], cb: &[Vec<f32>; 3], idxd: &[f32], idyd: &[f32], idzd: &[f32]) {
    let (nx, ny, nz) = (g.nx, g.ny, g.nz);
    let (sy, sx) = (nz, ny * nz);
    let [ex, ey, ez] = e;
    let [hx, hy, hz] = h;
    let off = i0 * sx;
    for i in i0..i1 {
        for j in 0..ny {
            let b = g.id(i, j, 0);
            let o = b - off;
            if i + 1 < nx && j > 0 && j + 1 < ny {
                let iy = idyd[j];
                let (er, a, c, hzr, hzm, hyr) = (&mut ex[o..o + nz], &ca[0][b..b + nz], &cb[0][b..b + nz], &hz[b..b + nz], &hz[b - sy..b - sy + nz], &hy[b..b + nz]);
                for k in 1..nz - 1 {
                    let curl = (hzr[k] - hzm[k]) * iy - (hyr[k] - hyr[k - 1]) * idzd[k];
                    er[k] = a[k] * er[k] + c[k] * curl;
                }
            }
            if j + 1 < ny && i > 0 && i + 1 < nx {
                let ix = idxd[i];
                let (er, a, c, hxr, hzr, hzm) = (&mut ey[o..o + nz], &ca[1][b..b + nz], &cb[1][b..b + nz], &hx[b..b + nz], &hz[b..b + nz], &hz[b - sx..b - sx + nz]);
                for k in 1..nz - 1 {
                    let curl = (hxr[k] - hxr[k - 1]) * idzd[k] - (hzr[k] - hzm[k]) * ix;
                    er[k] = a[k] * er[k] + c[k] * curl;
                }
            }
            if i > 0 && i + 1 < nx && j > 0 && j + 1 < ny {
                let (ix, iy) = (idxd[i], idyd[j]);
                let (er, a, c, hyr, hym, hxr, hxm) = (&mut ez[o..o + nz], &ca[2][b..b + nz], &cb[2][b..b + nz], &hy[b..b + nz], &hy[b - sx..b - sx + nz], &hx[b..b + nz], &hx[b - sy..b - sy + nz]);
                for k in 0..nz - 1 {
                    let curl = (hyr[k] - hym[k]) * ix - (hxr[k] - hxm[k]) * iy;
                    er[k] = a[k] * er[k] + c[k] * curl;
                }
            }
        }
    }
}

/// Neighbour values of the tangential E fields on the six faces (before the E update).
fn mur_save(g: &Grid, e: &[Vec<f32>; 3], out: &mut Vec<f32>) {
    let (nx, ny, nz) = (g.nx, g.ny, g.nz);
    // x faces: Ey, Ez at i=0 (neighbour 1) and i=nx-1 (neighbour nx-2); store boundary and neighbour
    for &(i, inn) in &[(0usize, 1usize), (nx - 1, nx - 2)] {
        for j in 0..ny {
            for k in 0..nz {
                out.push(e[1][g.id(i, j, k)]);
                out.push(e[1][g.id(inn, j, k)]);
                out.push(e[2][g.id(i, j, k)]);
                out.push(e[2][g.id(inn, j, k)]);
            }
        }
    }
    for &(j, inn) in &[(0usize, 1usize), (ny - 1, ny - 2)] {
        for i in 0..nx {
            for k in 0..nz {
                out.push(e[0][g.id(i, j, k)]);
                out.push(e[0][g.id(i, inn, k)]);
                out.push(e[2][g.id(i, j, k)]);
                out.push(e[2][g.id(i, inn, k)]);
            }
        }
    }
    for &(k, inn) in &[(0usize, 1usize), (nz - 1, nz - 2)] {
        for i in 0..nx {
            for j in 0..ny {
                out.push(e[0][g.id(i, j, k)]);
                out.push(e[0][g.id(i, j, inn)]);
                out.push(e[1][g.id(i, j, k)]);
                out.push(e[1][g.id(i, j, inn)]);
            }
        }
    }
}

/// Mur's first-order condition: E0(n+1) = E1(n) + q (E1(n+1) − E0(n)), q = (vΔt − Δ)/(vΔt + Δ).
fn mur_apply(g: &Grid, e: &mut [Vec<f32>; 3], prev: &[f32], epsr: &[Vec<f32>; 3], dt: f64) {
    let (nx, ny, nz) = (g.nx, g.ny, g.nz);
    let q = |d: f64, er: f32| -> f32 {
        let v = C0 / (er.max(1.0) as f64).sqrt();
        ((v * dt - d) / (v * dt + d)) as f32
    };
    let mut p = 0;
    for &(i, inn, d) in &[(0usize, 1usize, g.dx[0]), (nx - 1, nx - 2, g.dx[nx - 2])] {
        for j in 0..ny {
            for k in 0..nz {
                let (b, nb) = (g.id(i, j, k), g.id(inn, j, k));
                for comp in [1usize, 2] {
                    let (e0, e1) = (prev[p], prev[p + 1]);
                    p += 2;
                    let ok = if comp == 1 { j + 1 < ny } else { k + 1 < nz };
                    if ok {
                        e[comp][b] = e1 + q(d, epsr[comp][nb]) * (e[comp][nb] - e0);
                    }
                }
            }
        }
    }
    for &(j, inn, d) in &[(0usize, 1usize, g.dy[0]), (ny - 1, ny - 2, g.dy[ny - 2])] {
        for i in 0..nx {
            for k in 0..nz {
                let (b, nb) = (g.id(i, j, k), g.id(i, inn, k));
                for comp in [0usize, 2] {
                    let (e0, e1) = (prev[p], prev[p + 1]);
                    p += 2;
                    let ok = if comp == 0 { i + 1 < nx } else { k + 1 < nz };
                    if ok {
                        e[comp][b] = e1 + q(d, epsr[comp][nb]) * (e[comp][nb] - e0);
                    }
                }
            }
        }
    }
    for &(k, inn, d) in &[(0usize, 1usize, g.dz[0]), (nz - 1, nz - 2, g.dz[nz - 2])] {
        for i in 0..nx {
            for j in 0..ny {
                let (b, nb) = (g.id(i, j, k), g.id(i, j, inn));
                for comp in [0usize, 1] {
                    let (e0, e1) = (prev[p], prev[p + 1]);
                    p += 2;
                    let ok = if comp == 0 { i + 1 < nx } else { j + 1 < ny };
                    if ok {
                        e[comp][b] = e1 + q(d, epsr[comp][nb]) * (e[comp][nb] - e0);
                    }
                }
            }
        }
    }
}

fn dft(x: &[f64], dt: f64, f: f64, t_off: f64) -> (f64, f64) {
    let w = 2.0 * std::f64::consts::PI * f;
    let (mut re, mut im) = (0.0, 0.0);
    for (n, v) in x.iter().enumerate() {
        let t = (n as f64 + 1.0) * dt + t_off;
        re += v * (w * t).cos();
        im -= v * (w * t).sin();
    }
    (re * dt, im * dt)
}

/// Run every excitation and return the S-parameters.
pub fn simulate(c: &Case, mesh: Option<Mesh>, on_progress: &mut dyn FnMut(usize, usize, f64)) -> Result {
    let m = mesh.unwrap_or_else(|| make_mesh(c));
    let freq: Vec<f64> = (0..c.nf).map(|i| c.fmax * (i as f64 + 1.0) / c.nf as f64).collect();
    let exc_list = c.excite.clone().unwrap_or_else(|| (0..c.ports.len()).collect());
    let mut runs = Vec::new();
    for &ex in &exc_list {
        let t0 = now();
        let (dt, steps, vs, is, _) = run(c, &m, ex, &mut |s, db| on_progress(ex, s, db));
        let secs = now() - t0;
        let mut s = Vec::new();
        let z = c.ports[ex].r;
        let wave = |pi: usize, f: f64| -> ((f64, f64), (f64, f64)) {
            let (v, i) = (dft(&vs[pi], dt, f, 0.0), dft(&is[pi], dt, f, -0.5 * dt));
            let zr = c.ports[pi].r;
            let k = 1.0 / (2.0 * zr.sqrt());
            (((v.0 + zr * i.0) * k, (v.1 + zr * i.1) * k), ((v.0 - zr * i.0) * k, (v.1 - zr * i.1) * k))
        };
        let _ = z;
        for pi in 0..c.ports.len() {
            let row: Vec<(f64, f64)> = freq
                .iter()
                .map(|&f| {
                    let (a, _) = wave(ex, f);
                    let (_, b) = wave(pi, f);
                    let den = a.0 * a.0 + a.1 * a.1;
                    ((b.0 * a.0 + b.1 * a.1) / den, (b.1 * a.0 - b.0 * a.1) / den)
                })
                .collect();
            s.push(row);
        }
        runs.push(RunResult { excited: ex, steps, dt, seconds: secs, s });
    }
    Result { name: c.name.clone(), cells: m.x.len() * m.y.len() * m.z.len(), mesh: m, freq, runs }
}

#[cfg(not(target_arch = "wasm32"))]
fn now() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}
#[cfg(target_arch = "wasm32")]
fn now() -> f64 {
    0.0
}

// --- wasm interface (feasibility: one run per call) --------------------------------------
#[cfg(target_arch = "wasm32")]
mod wasm {
    use std::cell::RefCell;
    thread_local! {
        static OUT: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    }
    #[no_mangle]
    pub extern "C" fn fd_alloc(n: usize) -> *mut u8 {
        let mut v = Vec::<u8>::with_capacity(n.max(1));
        let p = v.as_mut_ptr();
        std::mem::forget(v);
        p
    }
    #[no_mangle]
    pub extern "C" fn fd_free(p: *mut u8, n: usize) {
        unsafe { drop(Vec::from_raw_parts(p, 0, n.max(1))) }
    }
    #[no_mangle]
    pub extern "C" fn fd_out_ptr() -> *const u8 {
        OUT.with(|o| o.borrow().as_ptr())
    }
    #[no_mangle]
    pub extern "C" fn fd_out_len() -> usize {
        OUT.with(|o| o.borrow().len())
    }
    /// Input: {"case": Case, "mesh": Mesh | null}. Output: Result (JSON) or {"error"}.
    #[no_mangle]
    pub extern "C" fn fd_run(p: *const u8, n: usize) -> i32 {
        let txt = String::from_utf8_lossy(unsafe { std::slice::from_raw_parts(p, n) }).into_owned();
        let res = (|| -> std::result::Result<String, String> {
            let v: serde_json::Value = serde_json::from_str(&txt).map_err(|e| e.to_string())?;
            let case: super::Case = serde_json::from_value(v["case"].clone()).map_err(|e| e.to_string())?;
            let mesh: Option<super::Mesh> = serde_json::from_value(v["mesh"].clone()).ok();
            let r = super::simulate(&case, mesh, &mut |_, _, _| {});
            Ok(serde_json::to_string(&r).unwrap())
        })();
        let (code, s) = match res {
            Ok(s) => (0, s),
            Err(e) => (1, serde_json::json!({ "error": e }).to_string()),
        };
        OUT.with(|o| *o.borrow_mut() = s.into_bytes());
        code
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 6 mm microstrip, 50 Ω ports: low reflection, small loss, |S11|² + |S21|² ≤ 1.
    #[test]
    fn short_microstrip() {
        let w = 0.35;
        let c: Case = serde_json::from_value(serde_json::json!({
            "name": "t", "domain": [-4.0, -2.5, -1.0, 4.0, 2.5, 2.2], "board": [-3.5, -2.0, 3.5, 2.0],
            "layers": [{"z0": 0.0, "z1": 0.2, "er": 4.3, "tand": 0.0}],
            "sheets": [{"z": 0.0, "rect": [-3.5, -2.0, 3.5, 2.0]}, {"z": 0.2, "rect": [-3.0, -w / 2.0, 3.0, w / 2.0]}],
            "ports": [{"x": -3.0, "y0": -w / 2.0, "y1": w / 2.0, "z0": 0.0, "z1": 0.2}, {"x": 3.0, "y0": -w / 2.0, "y1": w / 2.0, "z0": 0.0, "z1": 0.2}],
            "fmax": 10e9, "mesh": {"dfine": 0.05}, "end_db": -35, "max_steps": 20000, "excite": [0], "nf": 20
        }))
        .unwrap();
        let r = simulate(&c, None, &mut |_, _, _| {});
        let s = &r.runs[0].s;
        let db = |x: (f64, f64)| 20.0 * x.0.hypot(x.1).log10();
        for k in [1, 9, 19] {
            let (s11, s21) = (db(s[0][k]), db(s[1][k]));
            eprintln!("{:.1} GHz: S11 {s11:.1} dB, S21 {s21:.2} dB", r.freq[k] / 1e9);
            assert!(s11 < -12.0 && s21 > -0.6 && s21 < 0.05, "{s11} {s21}");
        }
    }
}

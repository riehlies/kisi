//! 3D FDTD (Yee scheme) for via transitions and small cut-outs of a PCB.
//!
//! A case (`Case`, JSON) describes a box: dielectric layers of the board, zero-thickness copper
//! sheets (traces, planes with round or slot-shaped antipads, pads), vias as solid cylinders and
//! lumped ports. The mesh is rectilinear and non-uniform (`make_mesh`). Dielectric loss is a
//! conductivity fixed at `f_loss`.
//!
//! * Metal: conformal (Dey–Mittra) on round boundaries – the H update of every face that touches a
//!   partially free edge uses the free edge lengths and an effective face area that keeps the
//!   time step stable – or staircased (`"conformal": false`, as openEMS).
//! * Boundary: convolutional PML (Roden & Gedney, CFS) in `pml_cells` cells outside the domain;
//!   whatever touches the domain boundary continues through it (infinite planes). Mur's first
//!   order condition and PEC are options.
//! * Every port is excited in turn with a Gaussian pulse up to `fmax`; voltages and currents at
//!   the ports give the S-parameters.
//!
//! Validation: `validation/fdtd` (analytic cavity, mesh convergence, openEMS on the same mesh).

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

/// Zero-thickness copper at height z: a rectangle with holes (antipads). A hole is a circle
/// [cx, cy, r] or a slot [x0, y0, x1, y1, r] (all points within r of the segment).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Sheet {
    pub z: f64,
    pub rect: [f64; 4],
    #[serde(default)]
    pub holes: Vec<Vec<f64>>,
}

/// segment and radius of a hole
fn hole_seg(h: &[f64]) -> (f64, f64, f64, f64, f64) {
    if h.len() >= 5 {
        (h[0], h[1], h[2], h[3], h[4])
    } else {
        (h[0], h[1], h[0], h[1], h[2])
    }
}

/// distance from (x, y) to the segment (x0, y0)–(x1, y1)
fn seg_dist(x: f64, y: f64, x0: f64, y0: f64, x1: f64, y1: f64) -> f64 {
    let (dx, dy) = (x1 - x0, y1 - y0);
    let l2 = dx * dx + dy * dy;
    let t = if l2 > 0.0 { (((x - x0) * dx + (y - y0) * dy) / l2).clamp(0.0, 1.0) } else { 0.0 };
    (x - x0 - t * dx).hypot(y - y0 - t * dy)
}

/// Solid cylinder along z (via barrel); `pads`: circular pads (radius) at these heights.
/// `outer`: metal outside the radius instead (a cylindrical wall, for cavity tests).
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
    #[serde(default)]
    pub outer: bool,
    /// cell size near this via (mm); default the case's `dfine` (stitching vias can be coarser)
    #[serde(default)]
    pub dmesh: f64,
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

/// Lumped series R–L–C along z at x over y0..y1 between z0 and z1 (a capacitor with its ESR and
/// ESL, a resistor, a short). `l` = 0: no inductance, `c` = 0: no capacitor (a short in series).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Lumped {
    pub x: f64,
    pub y0: f64,
    pub y1: f64,
    pub z0: f64,
    pub z1: f64,
    #[serde(default)]
    pub r: f64,
    #[serde(default)]
    pub l: f64,
    #[serde(default)]
    pub c: f64,
}

/// Field monitor: the plane normal to `axis` ("x", "y" or "z") nearest to `pos` (mm); the DFT of
/// the E components `comps` (0 = x, 1 = y, 2 = z) at `freqs`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Monitor {
    pub axis: String,
    pub pos: f64,
    pub freqs: Vec<f64>,
    #[serde(default = "all_comps")]
    pub comps: Vec<usize>,
}
fn all_comps() -> Vec<usize> {
    vec![0, 1, 2]
}

/// DFT of one E component in a monitor plane, per volt of the incident wave at the excited port
/// (V/m per V): values on the mesh nodes u × v (row-major, v fastest), u, v the in-plane axes.
#[derive(Debug, Clone, Serialize)]
pub struct FieldPlane {
    pub axis: String,
    pub pos: f64,
    pub freq: f64,
    pub comp: usize,
    pub u: Vec<f64>,
    pub v: Vec<f64>,
    pub re: Vec<f32>,
    pub im: Vec<f32>,
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
    #[serde(default)]
    pub lumped: Vec<Lumped>,
    /// field monitors, recorded during the first excitation only
    #[serde(default)]
    pub monitors: Vec<Monitor>,
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
    /// outer boundary: "pml" (convolutional PML in `pml_cells` cells outside the domain),
    /// "mur" (first order, on the domain faces) or "pec"
    #[serde(default = "pml")]
    pub boundary: String,
    #[serde(default = "pml_cells")]
    pub pml_cells: usize,
    /// conformal metal (Dey–Mittra) on round vias, pads and antipads; false: staircase
    #[serde(default = "yes")]
    pub conformal: bool,
}
fn one() -> usize {
    1
}
fn yes() -> bool {
    true
}
fn pml() -> String {
    "pml".into()
}
fn pml_cells() -> usize {
    8
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

/// Lines between fixed lines (position, cell size there, soft), growing by `ratio` towards the
/// middle of each gap, at most `dmax`. Hard lines closer than 0.6 × their smaller cell size merge
/// (to their mean; equal values keep the exact value). Soft lines (circle extremes with conformal
/// metal: they only ask for fine cells there) go into any line within 0.9 cells and take its
/// position. A remainder of at least half a cell becomes a cell of its own and the gap is
/// rescaled, so no sliver cell sets the time step.
fn grade(fixed: &mut Vec<(f64, f64, bool)>, dmax: f64, ratio: f64) -> Vec<f64> {
    fixed.sort_by(|a, b| a.0.total_cmp(&b.0));
    let (lo, hi) = (fixed[0].0, fixed[fixed.len() - 1].0);
    // group: (first, last, sum of hard, count of hard, sum of soft, count of soft, cell)
    let mut groups: Vec<(f64, f64, f64, usize, f64, usize, f64)> = Vec::new();
    for &(x, cell, soft) in fixed.iter() {
        let join = match groups.last() {
            Some(g) => {
                let thr = if soft || g.3 == 0 { 0.9 } else { 0.6 } * g.6.min(cell);
                x - g.0 < thr
            }
            None => false,
        };
        if join {
            let g = groups.last_mut().unwrap();
            g.1 = x;
            if soft {
                g.4 += x;
                g.5 += 1;
            } else {
                g.2 += x;
                g.3 += 1;
            }
            g.6 = g.6.min(cell);
        } else if soft {
            groups.push((x, x, 0.0, 0, x, 1, cell));
        } else {
            groups.push((x, x, x, 1, 0.0, 0, cell));
        }
    }
    let n = groups.len();
    let f: Vec<(f64, f64)> = groups
        .iter()
        .enumerate()
        .map(|(i, g)| {
            let v = if i == 0 {
                lo
            } else if i == n - 1 {
                hi
            } else if g.3 > 0 {
                // hard lines decide; a cluster of equal values keeps the exact value
                let m = g.2 / g.3 as f64;
                let first_hard = fixed.iter().find(|x| !x.2 && x.0 >= g.0 - 1e-12 && x.0 <= g.1 + 1e-12).map(|x| x.0).unwrap_or(m);
                if (m - first_hard).abs() < 1e-9 {
                    first_hard
                } else {
                    m
                }
            } else {
                g.4 / g.5 as f64
            };
            (v, g.6)
        })
        .collect();
    let mut out = vec![f[0].0];
    for w in f.windows(2) {
        let (a, b) = (w[0].0, w[1].0);
        let len = b - a;
        if len <= 1e-12 {
            continue;
        }
        let (mut sl, mut sr) = (w[0].1.min(dmax), w[1].1.min(dmax));
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
            // a full cell instead of the sliver; the rescaling below shrinks all cells evenly
            cells.push(if cells.len() + right.len() == 0 { rem } else { sl.min(sr) });
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
    let mut fx: Vec<(f64, f64, bool)> = vec![(d[0], dmax, false), (d[3], dmax, false), (c.board[0], fine, false), (c.board[2], fine, false)];
    let mut fy: Vec<(f64, f64, bool)> = vec![(d[1], dmax, false), (d[4], dmax, false), (c.board[1], fine, false), (c.board[3], fine, false)];
    let mut fz: Vec<(f64, f64, bool)> = vec![(d[2], dmax, false), (d[5], dmax, false)];
    for l in &c.layers {
        fz.push((l.z0, fine, false));
        fz.push((l.z1, fine, false));
    }
    for s in &c.sheets {
        fz.push((s.z, fine, false));
        fx.push((s.rect[0], fine, false));
        fx.push((s.rect[2], fine, false));
        fy.push((s.rect[1], fine, false));
        fy.push((s.rect[3], fine, false));
        for h in &s.holes {
            let (x0, y0, x1, y1, r) = hole_seg(h);
            for dd in [-r, 0.0, r] {
                let soft = c.conformal && dd != 0.0;
                fx.push((x0 + dd, fine, soft));
                fy.push((y0 + dd, fine, soft));
                fx.push((x1 + dd, fine, soft));
                fy.push((y1 + dd, fine, soft));
            }
        }
    }
    for v in &c.vias {
        let cell = if v.dmesh > 0.0 { v.dmesh } else { fine };
        // conformal: the circle needs no lines of its own, only its centre
        let offs: &[f64] = if c.conformal && v.dmesh > 0.0 { &[0.0] } else { &[0.0, -1.0, 1.0, -2.0, 2.0] };
        for &k in offs {
            let dd = match k as i32 {
                -1 => -v.r,
                1 => v.r,
                -2 => -v.pad_r,
                2 => v.pad_r,
                _ => 0.0,
            };
            let soft = c.conformal && dd != 0.0;
            fx.push((v.x + dd, cell, soft));
            fy.push((v.y + dd, cell, soft));
        }
        fz.push((v.z0, fine, false));
        fz.push((v.z1, fine, false));
    }
    for p in &c.ports {
        fx.push((p.x, fine, false));
        fy.push((p.y0, fine, false));
        fy.push((p.y1, fine, false));
        fz.push((p.z0, fine, false));
        fz.push((p.z1, fine, false));
    }
    let clip = |v: &mut Vec<(f64, f64, bool)>, lo: f64, hi: f64| v.retain(|x| x.0 >= lo - 1e-9 && x.0 <= hi + 1e-9);
    clip(&mut fx, d[0], d[3]);
    clip(&mut fy, d[1], d[4]);
    clip(&mut fz, d[2], d[5]);
    let mut m = Mesh { x: grade(&mut fx, dmax, c.mesh.ratio), y: grade(&mut fy, dmax, c.mesh.ratio), z: grade(&mut fz, dmax, c.mesh.ratio) };
    if c.boundary == "pml" {
        // PML cells outside the domain, as wide as the outermost cell
        for v in [&mut m.x, &mut m.y, &mut m.z] {
            let (d0, d1) = (v[1] - v[0], v[v.len() - 1] - v[v.len() - 2]);
            let (a, b) = (v[0], v[v.len() - 1]);
            let mut lo: Vec<f64> = (1..=c.pml_cells).rev().map(|k| a - k as f64 * d0).collect();
            lo.extend(v.iter());
            lo.extend((1..=c.pml_cells).map(|k| b + k as f64 * d1));
            *v = lo;
        }
    }
    m
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
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<FieldPlane>,
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

/// A round boundary where metal starts or ends: segment (x0, y0)–(x1, y1) (a point for a circle),
/// radius, z0, z1; z0 == z1 for one in a sheet plane.
type Circle = (f64, f64, f64, f64, f64, f64, f64);

/// Geometry tests of a case. Points outside the domain (in the PML) are clamped to it, so
/// whatever touches the domain boundary continues straight through the PML.
struct Geo<'a> {
    c: &'a Case,
    circles: Vec<Circle>,
}

impl<'a> Geo<'a> {
    fn new(c: &'a Case) -> Self {
        let mut circles = Vec::new();
        for s in &c.sheets {
            for h in &s.holes {
                let (x0, y0, x1, y1, r) = hole_seg(h);
                circles.push((x0, y0, x1, y1, r, s.z, s.z));
            }
        }
        for v in &c.vias {
            circles.push((v.x, v.y, v.x, v.y, v.r, v.z0, v.z1));
            if !v.outer {
                for &pz in &v.pads {
                    circles.push((v.x, v.y, v.x, v.y, v.pad_r, pz, pz));
                }
            }
        }
        Geo { c, circles }
    }
    fn clamp(&self, x: f64, y: f64, z: f64) -> (f64, f64, f64) {
        let d = &self.c.domain;
        (x.clamp(d[0], d[3]), y.clamp(d[1], d[4]), z.clamp(d[2], d[5]))
    }
    fn metal(&self, x: f64, y: f64, z: f64) -> bool {
        let (x, y, z) = self.clamp(x, y, z);
        let c = self.c;
        c.sheets.iter().any(|s| {
            (s.z - z).abs() < 1e-9 && x >= s.rect[0] - 1e-9 && x <= s.rect[2] + 1e-9 && y >= s.rect[1] - 1e-9 && y <= s.rect[3] + 1e-9 && !s.holes.iter().any(|h| {
                    let (x0, y0, x1, y1, r) = hole_seg(h);
                    seg_dist(x, y, x0, y0, x1, y1) < r - 1e-9
                })
        }) || c.vias.iter().any(|v| {
            let d = (x - v.x).hypot(y - v.y);
            let inz = z >= v.z0 - 1e-9 && z <= v.z1 + 1e-9;
            if v.outer {
                inz && d >= v.r - 1e-9
            } else {
                (inz && d <= v.r + 1e-9) || (v.pads.iter().any(|&pz| (pz - z).abs() < 1e-9) && d <= v.pad_r + 1e-9)
            }
        })
    }
    /// relative permittivity and conductivity at a point (dielectric loss at f_loss)
    fn mat(&self, x: f64, y: f64, z: f64, f_loss: f64) -> (f64, f64) {
        let (x, y, z) = self.clamp(x, y, z);
        let c = self.c;
        if x >= c.board[0] && x <= c.board[2] && y >= c.board[1] && y <= c.board[3] {
            for l in &c.layers {
                if z >= l.z0 && z <= l.z1 {
                    return (l.er, 2.0 * std::f64::consts::PI * f_loss * EPS0 * l.er * l.tand);
                }
            }
        }
        (1.0, 0.0)
    }
    /// does a circle boundary pass within `reach` of (x, y) at height z (z range for faces)?
    fn near_circle(&self, x: f64, y: f64, z0: f64, z1: f64, reach: f64) -> bool {
        self.circles.iter().any(|&(x0, y0, x1, y1, r, a, b)| {
            let zok = if a == b { z0 <= a + 1e-9 && z1 >= a - 1e-9 && (z1 - z0) < 1e-9 } else { z1 >= a - 1e-9 && z0 <= b + 1e-9 };
            zok && (seg_dist(x, y, x0, y0, x1, y1) - r).abs() < reach
        })
    }
    /// free (non-metal) fraction of the segment p0 → p1
    fn free_len(&self, p0: [f64; 3], p1: [f64; 3]) -> f64 {
        const N: usize = 64;
        let mut free = 0;
        for s in 0..N {
            let t = (s as f64 + 0.5) / N as f64;
            if !self.metal(p0[0] + t * (p1[0] - p0[0]), p0[1] + t * (p1[1] - p0[1]), p0[2] + t * (p1[2] - p0[2])) {
                free += 1;
            }
        }
        free as f64 / N as f64
    }
    /// free fraction of the axis-aligned face spanned by p0, p0 + u, p0 + v
    fn free_area(&self, p0: [f64; 3], u: [f64; 3], v: [f64; 3]) -> f64 {
        const N: usize = 24;
        let mut free = 0;
        for a in 0..N {
            for b in 0..N {
                let (s, t) = ((a as f64 + 0.5) / N as f64, (b as f64 + 0.5) / N as f64);
                if !self.metal(p0[0] + s * u[0] + t * v[0], p0[1] + s * u[1] + t * v[1], p0[2] + s * u[2] + t * v[2]) {
                    free += 1;
                }
            }
        }
        free as f64 / (N * N) as f64
    }
}

/// Convolutional PML (Roden & Gedney, CFS): coefficients per axis at the nodes (E-type
/// derivatives) and at the cell centres (H-type), and the auxiliary fields ψ of the twelve
/// (field component, derivative axis) pairs.
struct Cpml {
    /// per axis: (node, b, c) for E updates and (cell, b, c) for H updates, only where σ > 0
    e: [Vec<(usize, f32, f32)>; 3],
    h: [Vec<(usize, f32, f32)>; 3],
    /// ψ of E[comp] / H[comp] for the derivative along axis a: [comp][slot] with slot 0/1
    pe: [[Vec<f32>; 2]; 3],
    ph: [[Vec<f32>; 2]; 3],
}

/// (derivative axis, source component, sign) of the curl terms of component c:
/// (curl F)_c = Σ sign · ∂F_src/∂axis
const CURL: [[(usize, usize, f32); 2]; 3] = [[(1, 2, 1.0), (2, 1, -1.0)], [(2, 0, 1.0), (0, 2, -1.0)], [(0, 1, 1.0), (1, 0, -1.0)]];

/// index ranges of a component along each axis: E (edges, interior) or H (faces)
fn ranges(g: &Grid, comp: usize, is_e: bool) -> [(usize, usize); 3] {
    let n = [g.nx, g.ny, g.nz];
    let mut r = [(0, 0); 3];
    for d in 0..3 {
        r[d] = if is_e {
            if d == comp {
                (0, n[d] - 1)
            } else {
                (1, n[d] - 1)
            }
        } else if d == comp {
            (0, n[d])
        } else {
            (0, n[d] - 1)
        };
    }
    r
}

impl Cpml {
    fn new(g: &Grid, m: &Mesh, ncell: usize, dt: f64, fmax: f64, kappa_max: f64) -> (Self, [Vec<f64>; 3], [Vec<f64>; 3]) {
        let order = 3.0;
        let lines = [&m.x, &m.y, &m.z];
        let mut e: [Vec<(usize, f32, f32)>; 3] = Default::default();
        let mut h: [Vec<(usize, f32, f32)>; 3] = Default::default();
        // κ at the nodes and the cell centres (scales the inverse spacings of the main update)
        let mut ke: [Vec<f64>; 3] = Default::default();
        let mut kh: [Vec<f64>; 3] = Default::default();
        let eta0 = (MU0 / EPS0).sqrt();
        let alpha_max = 2.0 * std::f64::consts::PI * EPS0 * fmax / 20.0;
        for a in 0..3 {
            let v = lines[a];
            let n = v.len();
            let (lo, hi) = (v[ncell], v[n - 1 - ncell]);
            let (dlo, dhi) = (lo - v[0], v[n - 1] - hi);
            let sig_lo = 0.8 * (order + 1.0) / (eta0 * dlo / ncell as f64 * 1e-3);
            let sig_hi = 0.8 * (order + 1.0) / (eta0 * dhi / ncell as f64 * 1e-3);
            let coef = |x: f64| -> Option<(f64, f32, f32)> {
                let (rho, smax) = if x < lo - 1e-12 {
                    ((lo - x) / dlo, sig_lo)
                } else if x > hi + 1e-12 {
                    ((x - hi) / dhi, sig_hi)
                } else {
                    return None;
                };
                let sig = smax * rho.powf(order);
                let kap = 1.0 + (kappa_max - 1.0) * rho.powf(order);
                let alp = alpha_max * (1.0 - rho);
                let b = (-(sig / kap + alp) * dt / EPS0).exp();
                let c = sig * (b - 1.0) / (sig * kap + kap * kap * alp);
                Some((kap, b as f32, c as f32))
            };
            ke[a] = vec![1.0; n];
            kh[a] = vec![1.0; n - 1];
            for i in 1..n - 1 {
                if let Some((k, b, c)) = coef(v[i]) {
                    ke[a][i] = k;
                    e[a].push((i, b, c));
                }
            }
            for i in 0..n - 1 {
                if let Some((k, b, c)) = coef(0.5 * (v[i] + v[i + 1])) {
                    kh[a][i] = k;
                    h[a].push((i, b, c));
                }
            }
        }
        let size = |comp: usize, a: usize, is_e: bool, list: &Vec<(usize, f32, f32)>| -> usize {
            let r = ranges(g, comp, is_e);
            (0..3).map(|d| if d == a { list.iter().filter(|x| x.0 >= r[d].0 && x.0 < r[d].1).count() } else { r[d].1 - r[d].0 }).product()
        };
        let mut pe: [[Vec<f32>; 2]; 3] = Default::default();
        let mut ph: [[Vec<f32>; 2]; 3] = Default::default();
        for c in 0..3 {
            for (s, &(a, _, _)) in CURL[c].iter().enumerate() {
                pe[c][s] = vec![0.0; size(c, a, true, &e[a])];
                ph[c][s] = vec![0.0; size(c, a, false, &h[a])];
            }
        }
        (Cpml { e, h, pe, ph }, ke, kh)
    }
}

/// ψ update and correction for one (component, slot) pair. E: f += sign·cb·ψ with
/// ψ = bψ + c·(src[n] − src[n−1])/Δdual; H: f −= hc·sign·ψ with ψ = bψ + c·(src[n+1] − src[n])/Δ.
/// ψ is stored in the loop order i, j, k with the PML axis restricted to its nodes.
#[allow(clippy::too_many_arguments)]
fn cpml_pass(g: &Grid, comp: usize, a: usize, sign: f32, is_e: bool, list: &[(usize, f32, f32)], psi: &mut [f32], f: &mut [f32], src: &[f32], cb: Option<&[f32]>, hc: f32, inv: &[f32]) {
    let r = ranges(g, comp, is_e);
    let st = [g.ny * g.nz, g.nz, 1][a];
    let sel: Vec<(usize, f32, f32)> = list.iter().cloned().filter(|x| x.0 >= r[a].0 && x.0 < r[a].1).collect();
    let id = |i: usize, j: usize, k: usize| (i * g.ny + j) * g.nz + k;
    let mut p = 0;
    if a == 2 {
        for i in r[0].0..r[0].1 {
            for j in r[1].0..r[1].1 {
                let base = id(i, j, 0);
                for &(k, b, c) in &sel {
                    let x = base + k;
                    if is_e {
                        let d = (src[x] - src[x - st]) * inv[k];
                        psi[p] = b * psi[p] + c * d;
                        f[x] += sign * cb.unwrap()[x] * psi[p];
                    } else {
                        let d = (src[x + st] - src[x]) * inv[k];
                        psi[p] = b * psi[p] + c * d;
                        f[x] -= hc * sign * psi[p];
                    }
                    p += 1;
                }
            }
        }
        return;
    }
    let (k0, nk) = (r[2].0, r[2].1 - r[2].0);
    let full = |d: usize| -> Vec<(usize, f32, f32)> { (r[d].0..r[d].1).map(|i| (i, 0.0, 0.0)).collect() };
    let (li, lj) = if a == 0 { (sel, full(1)) } else { (full(0), sel) };
    let hs = hc * sign;
    for &(i, bi, ci) in &li {
        for &(j, bj, cj) in &lj {
            let (b, c, n) = if a == 0 { (bi, ci, i) } else { (bj, cj, j) };
            let w = c * inv[n];
            let x = id(i, j, k0);
            let ps = &mut psi[p..p + nk];
            p += nk;
            let fr = &mut f[x..x + nk];
            let s0 = &src[x..x + nk];
            if is_e {
                let s1 = &src[x - st..x - st + nk];
                let cr = &cb.unwrap()[x..x + nk];
                for k in 0..nk {
                    ps[k] = b * ps[k] + w * (s0[k] - s1[k]);
                    fr[k] += sign * cr[k] * ps[k];
                }
            } else {
                let s1 = &src[x + st..x + st + nk];
                for k in 0..nk {
                    ps[k] = b * ps[k] + w * (s1[k] - s0[k]);
                    fr[k] -= hs * ps[k];
                }
            }
        }
    }
}

/// One run with port `ex` excited; returns V(t), I(t) for all ports.
#[allow(clippy::type_complexity)]
fn run(c: &Case, m: &Mesh, ex: usize, record: bool, on_progress: &mut dyn FnMut(usize, f64)) -> (f64, usize, Vec<Vec<f64>>, Vec<Vec<f64>>, Vec<f64>, Vec<FieldPlane>) {
    let g = Grid::new(m);
    let (nx, ny, nz) = (g.nx, g.ny, g.nz);
    let n = nx * ny * nz;
    let geo = Geo::new(c);
    let dmin = |v: &[f64]| v.iter().cloned().fold(f64::INFINITY, f64::min);
    let courant = if c.conformal { 0.95 } else { 0.99 };
    let dt = courant / (C0 * (1.0 / dmin(&g.dx).powi(2) + 1.0 / dmin(&g.dy).powi(2) + 1.0 / dmin(&g.dz).powi(2)).sqrt());
    let f_loss = if c.f_loss > 0.0 { c.f_loss } else { c.fmax / 2.0 };
    let cx = |i: usize| 0.5 * (m.x[i] + m.x[i + 1]);
    let cy = |j: usize| 0.5 * (m.y[j] + m.y[j + 1]);
    let cz = |k: usize| 0.5 * (m.z[k] + m.z[k + 1]);
    // average of the cells around an edge, weighted by the dual area each one contributes
    let avg = |pts: &[(f64, f64, f64, f64)]| -> (f64, f64) {
        let (mut e, mut s, mut w) = (0.0, 0.0, 0.0);
        for &(x, y, z, a) in pts {
            let (er, sg) = geo.mat(x, y, z, f_loss);
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
    let lines = [&m.x, &m.y, &m.z];
    // free fraction of every edge: 0 metal, 1 free; partial only near a circle in conformal mode
    let edge_free = |comp: usize, i: usize, j: usize, k: usize| -> f64 {
        let p0 = [m.x[i], m.y[j], m.z[k]];
        let mut p1 = p0;
        let idx = [i, j, k];
        p1[comp] = lines[comp][idx[comp] + 1];
        let mid = [0.5 * (p0[0] + p1[0]), 0.5 * (p0[1] + p1[1]), 0.5 * (p0[2] + p1[2])];
        if c.conformal && comp < 2 && geo.near_circle(mid[0], mid[1], mid[2], mid[2], p1[comp] - p0[comp]) {
            let f = geo.free_len(p0, p1);
            if f < 0.02 {
                0.0
            } else if f > 0.98 {
                1.0
            } else {
                f
            }
        } else if geo.metal(mid[0], mid[1], mid[2]) {
            0.0
        } else {
            1.0
        }
    };
    // coefficients
    let mut ca = [vec![0f32; n], vec![0f32; n], vec![0f32; n]];
    let mut cb = [vec![0f32; n], vec![0f32; n], vec![0f32; n]];
    let mut epsr = [vec![1f32; n], vec![1f32; n], vec![1f32; n]];
    let mut free = [vec![0f32; n], vec![0f32; n], vec![0f32; n]];
    let coef = |er: f64, sg: f64| -> (f32, f32) {
        let eps = EPS0 * er;
        let a = sg * dt / (2.0 * eps);
        (((1.0 - a) / (1.0 + a)) as f32, ((dt / eps) / (1.0 + a)) as f32)
    };
    for i in 0..nx {
        for j in 0..ny {
            for k in 0..nz {
                let id = g.id(i, j, k);
                // Ex: edge (i..i+1, j, k)
                if i + 1 < nx && j > 0 && j + 1 < ny && k > 0 && k + 1 < nz {
                    let fr = edge_free(0, i, j, k);
                    free[0][id] = fr as f32;
                    if fr > 0.0 {
                        let xm = cx(i);
                        let (y, z) = (m.y[j], m.z[k]);
                        let pts = [(xm, cy(j - 1), cz(k - 1), g.dy[j - 1] * g.dz[k - 1]), (xm, cy(j), cz(k - 1), g.dy[j] * g.dz[k - 1]), (xm, cy(j - 1), cz(k), g.dy[j - 1] * g.dz[k]), (xm, cy(j), cz(k), g.dy[j] * g.dz[k])];
                        let _ = (y, z);
                        let (er, sg) = avg(&pts);
                        let (a, b) = coef(er, sg);
                        ca[0][id] = a;
                        cb[0][id] = b;
                        epsr[0][id] = er as f32;
                    }
                }
                if j + 1 < ny && i > 0 && i + 1 < nx && k > 0 && k + 1 < nz {
                    let fr = edge_free(1, i, j, k);
                    free[1][id] = fr as f32;
                    if fr > 0.0 {
                        let ym = cy(j);
                        let pts = [(cx(i - 1), ym, cz(k - 1), g.dx[i - 1] * g.dz[k - 1]), (cx(i), ym, cz(k - 1), g.dx[i] * g.dz[k - 1]), (cx(i - 1), ym, cz(k), g.dx[i - 1] * g.dz[k]), (cx(i), ym, cz(k), g.dx[i] * g.dz[k])];
                        let (er, sg) = avg(&pts);
                        let (a, b) = coef(er, sg);
                        ca[1][id] = a;
                        cb[1][id] = b;
                        epsr[1][id] = er as f32;
                    }
                }
                if k + 1 < nz && i > 0 && i + 1 < nx && j > 0 && j + 1 < ny {
                    let fr = edge_free(2, i, j, k);
                    free[2][id] = fr as f32;
                    if fr > 0.0 {
                        let zm = cz(k);
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
    // conformal faces (Dey–Mittra): H −= Δt/μ · Σ ±E·l_free / A_eff on every face that has a
    // partially free edge or is cut by a round boundary. The regular update has run with the full
    // lengths; the correction adds the difference. Every face touching a partial edge must use its
    // free length: then the update operator is similar to a symmetric positive semidefinite one
    // (real eigenvalues, stable). A_eff ≥ A · (weighted free fraction of the edges) keeps the
    // face's local frequency at most that of the full cell.
    let mut conf: Vec<(usize, usize, [(usize, usize, f32); 4])> = Vec::new();
    if c.conformal && !geo.circles.is_empty() {
        let (sy, sx) = (nz, ny * nz);
        let st = [sx, sy, 1];
        let n3 = [nx, ny, nz];
        // faces to treat: mark[comp][id] (face normal to comp at its lower corner id)
        let mut mark = [vec![false; n], vec![false; n], vec![false; n]];
        for a in 0..3 {
            for id in 0..n {
                let f = free[a][id];
                if f > 0.0 && f < 1.0 {
                    let idx = [id / sx, (id / sy) % ny, id % nz];
                    for b in 0..3 {
                        if b == a {
                            continue;
                        }
                        // faces normal to b containing the edge: their other in-plane axis c
                        let cax = 3 - a - b;
                        mark[b][id] = true;
                        if idx[cax] > 0 {
                            mark[b][id - st[cax]] = true;
                        }
                    }
                }
            }
        }
        for i in 0..nx {
            for j in 0..ny {
                for k in 0..nz {
                    let id = g.id(i, j, k);
                    let idx = [i, j, k];
                    for comp in 0..3 {
                        // face normal to comp, spanned by axes u, v (u < v)
                        let (u, v) = match comp {
                            0 => (1, 2),
                            1 => (0, 2),
                            _ => (0, 1),
                        };
                        if idx[u] + 1 >= n3[u] || idx[v] + 1 >= n3[v] {
                            continue;
                        }
                        let p0 = [m.x[i], m.y[j], m.z[k]];
                        let (du, dv) = (lines[u][idx[u] + 1] - lines[u][idx[u]], lines[v][idx[v] + 1] - lines[v][idx[v]]);
                        if !mark[comp][id] {
                            let mut ctr = p0;
                            ctr[u] += 0.5 * du;
                            ctr[v] += 0.5 * dv;
                            let (z0, z1) = if comp == 2 { (p0[2], p0[2]) } else { (p0[2], p0[2] + dv) };
                            if !geo.near_circle(ctr[0], ctr[1], z0, z1, du.hypot(dv)) {
                                continue;
                            }
                        }
                        // edges: along u at v = 0 / dv, along v at u = 0 / du
                        let (e_u0, e_u1, e_v0, e_v1) = (id, id + st[v], id, id + st[u]);
                        let (fu0, fu1, fv0, fv1) = (free[u][e_u0] as f64, free[u][e_u1] as f64, free[v][e_v0] as f64, free[v][e_v1] as f64);
                        if fu0 + fu1 + fv0 + fv1 == 0.0 {
                            continue;
                        }
                        let mut uu = [0.0; 3];
                        let mut vv = [0.0; 3];
                        uu[u] = du;
                        vv[v] = dv;
                        let af = geo.free_area(p0, uu, vv);
                        // weights 1/Δperp²: edges along u have the perpendicular extent dv
                        let (wu, wv) = (1.0 / (dv * dv), 1.0 / (du * du));
                        let ratio = (wu * (fu0 + fu1) + wv * (fv0 + fv1)) / (2.0 * wu + 2.0 * wv);
                        let a_eff = af.max(ratio);
                        if fu0 == 1.0 && fu1 == 1.0 && fv0 == 1.0 && fv1 == 1.0 && a_eff > 0.999 {
                            continue;
                        }
                        // (curl E)_comp = Σ sign·∂E_src/∂axis; regular weight of an edge 1/Δperp,
                        // conformal l_free / A_eff; the correction adds hc·sign·(regular − conformal)
                        let mut terms = [(0usize, 0usize, 0f32); 4];
                        let mut t = 0;
                        let a_full = du * dv * 1e-6;
                        for &(_, srcc, sign) in &CURL[comp] {
                            let (dperp, lfull, f0, f1, i0, i1) = if srcc == u { (dv, du, fu0, fu1, e_u0, e_u1) } else { (du, dv, fv0, fv1, e_v0, e_v1) };
                            let wstd = 1.0 / (dperp * 1e-3);
                            let w0 = lfull * 1e-3 * f0 / (a_full * a_eff);
                            let w1 = lfull * 1e-3 * f1 / (a_full * a_eff);
                            terms[t] = (srcc, i0, (-(sign as f64) * (wstd - w0)) as f32);
                            terms[t + 1] = (srcc, i1, ((sign as f64) * (wstd - w1)) as f32);
                            t += 2;
                        }
                        conf.push((comp, id, terms));
                    }
                }
            }
        }
    }
    drop(free);
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
    // lumped elements: Ez edges with a series R–L–C each (R, L scaled by npar/nser, C by nser/npar);
    // state per edge: current I and charge q
    struct LumpEdge {
        id: usize,
        dz: f64,
        a: f64,
        eps: f64,
        r: f64,
        l: f64,
        inv_c: f64,
        i: f64,
        q: f64,
    }
    let mut lumps: Vec<LumpEdge> = Vec::new();
    for el in &c.lumped {
        let i = near(&m.x, el.x);
        let (j0, j1) = (near(&m.y, el.y0), near(&m.y, el.y1));
        let (k0, k1) = (near(&m.z, el.z0.min(el.z1)), near(&m.z, el.z0.max(el.z1)));
        let (nser, npar) = ((k1 - k0).max(1) as f64, (j1 - j0 + 1) as f64);
        for j in j0..=j1 {
            for k in k0..k1 {
                let id = g.id(i, j, k);
                lumps.push(LumpEdge {
                    id,
                    dz: g.dz[k],
                    a: g.dxd[i] * g.dyd[j],
                    eps: EPS0 * epsr[2][id] as f64,
                    r: el.r * npar / nser,
                    l: el.l * npar / nser,
                    inv_c: if el.c > 0.0 { 1.0 / (el.c * nser / npar) } else { 0.0 },
                    i: 0.0,
                    q: 0.0,
                });
                ca[2][id] = 0.0;
                cb[2][id] = 0.0;
            }
        }
    }
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
    // raw inverse spacings (ψ terms and the ports) and, in the PML, the ones divided by κ
    let raw_p = [inv(&g.dx), inv(&g.dy), inv(&g.dz)];
    let raw_d = [inv(&g.dxd), inv(&g.dyd), inv(&g.dzd)];
    let (idxd, idyd) = (raw_d[0].clone(), raw_d[1].clone());
    let mut kp = raw_p.clone();
    let mut kd = raw_d.clone();
    let mut cpml = None;
    if c.boundary == "pml" {
        let (p, ke, kh) = Cpml::new(&g, m, c.pml_cells, dt, c.fmax, 1.0);
        for a in 0..3 {
            for (i, x) in kp[a].iter_mut().enumerate() {
                *x /= kh[a][i] as f32;
            }
            for (i, x) in kd[a].iter_mut().enumerate() {
                *x /= ke[a][i] as f32;
            }
        }
        cpml = Some(p);
    }
    let hc = (dt / MU0) as f32;
    let fc = c.fmax / 2.0;
    let f0 = c.fmax / 2.0;
    let mut vs: Vec<Vec<f64>> = vec![Vec::new(); ports.len()];
    let mut is: Vec<Vec<f64>> = vec![Vec::new(); ports.len()];
    let mut exc = Vec::new();
    let t_pulse = 2.0 * 9.0 / (2.0 * std::f64::consts::PI * fc);
    let mut emax = 0.0f64;
    let mut steps = 0;
    let (sy, sx) = (nz, ny * nz);
    let mut prev: Vec<f32> = Vec::new();
    let threads = if cfg!(target_arch = "wasm32") { 1 } else { c.threads.max(1) };
    let mur = c.boundary == "mur";
    // field monitors: (plane index along the axis, axis, accumulators per (comp, freq))
    let mut mons: Vec<(usize, usize, Vec<FieldPlane>)> = Vec::new();
    if record {
        for mo in &c.monitors {
            let a = match mo.axis.as_str() {
                "x" => 0,
                "y" => 1,
                _ => 2,
            };
            let idx = near(lines[a], mo.pos);
            let (u, v) = match a {
                0 => (1, 2),
                1 => (0, 2),
                _ => (0, 1),
            };
            let mut planes = Vec::new();
            for &f in &mo.freqs {
                for &comp in &mo.comps {
                    let nn = lines[u].len() * lines[v].len();
                    planes.push(FieldPlane { axis: mo.axis.clone(), pos: lines[a][idx], freq: f, comp, u: lines[u].clone(), v: lines[v].clone(), re: vec![0.0; nn], im: vec![0.0; nn] });
                }
            }
            mons.push((idx, a, planes));
        }
    }
    for step in 0..c.max_steps {
        let t = step as f64 * dt;
        // --- H update
        par_update(&g, threads, &mut h, |hs, i0, i1| update_h(&g, hs, i0, i1, &e, hc, &kp[0], &kp[1], &kp[2]));
        if let Some(p) = cpml.as_mut() {
            for comp in 0..3 {
                for (slot, &(a, src, sign)) in CURL[comp].iter().enumerate() {
                    cpml_pass(&g, comp, a, sign, false, &p.h[a], &mut p.ph[comp][slot], &mut h[comp], &e[src], None, hc, &raw_p[a]);
                }
            }
        }
        for (comp, id, terms) in &conf {
            let mut s = 0.0;
            for &(ec, eid, w) in terms {
                s += w * e[ec][eid];
            }
            h[*comp][*id] += hc * s;
        }
        // --- save E next to the boundary for Mur
        if mur {
            prev.clear();
            mur_save(&g, &e, &mut prev);
        }
        // --- E update (interior); the port edges keep their old value for their own update
        let saved: Vec<Vec<f32>> = ports.iter().map(|p| p.edges.iter().map(|x| e[2][x.0]).collect()).collect();
        let saved_l: Vec<f32> = lumps.iter().map(|x| e[2][x.id]).collect();
        par_update(&g, threads, &mut e, |es, i0, i1| update_e(&g, es, i0, i1, &h, &ca, &cb, &kd[0], &kd[1], &kd[2]));
        if let Some(p) = cpml.as_mut() {
            for comp in 0..3 {
                for (slot, &(a, src, sign)) in CURL[comp].iter().enumerate() {
                    cpml_pass(&g, comp, a, sign, true, &p.e[a], &mut p.pe[comp][slot], &mut e[comp], &h[src], Some(&cb[comp]), hc, &raw_d[a]);
                }
            }
        }
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
        // --- lumped R–L–C (trapezoidal, implicit in E, I, q): with Ī = (Iⁿ⁺¹ + Iⁿ)/2,
        // L(Iⁿ⁺¹ − Iⁿ)/Δt + R·Ī + (qⁿ + Δt·Ī/2)/C = Δz(Eⁿ⁺¹ + Eⁿ)/2, ε(Eⁿ⁺¹ − Eⁿ)/Δt = curl H − Ī/A
        for (n_e, el) in lumps.iter_mut().enumerate() {
            let id = el.id;
            let i_ = id / sx;
            let j_ = (id / sy) % ny;
            let curl = ((h[1][id] - h[1][id - sx]) * idxd[i_] - (h[0][id] - h[0][id - sy]) * idyd[j_]) as f64;
            let old = saved_l[n_e] as f64;
            let den = 2.0 * el.l / dt + el.r + dt * el.inv_c / 2.0 + el.dz * dt / (2.0 * el.eps * el.a);
            let ib = (el.dz * old + el.dz * dt * curl / (2.0 * el.eps) + 2.0 * el.l * el.i / dt - el.q * el.inv_c) / den;
            e[2][id] = (old + dt / el.eps * (curl - ib / el.a)) as f32;
            el.i = 2.0 * ib - el.i;
            el.q += dt * ib;
        }
        // --- Mur boundaries
        if mur {
            mur_apply(&g, &mut e, &prev, &epsr, dt);
        }
        steps = step + 1;
        // --- field monitors (E at t = (step + 1)·Δt)
        for (idx, a, planes) in mons.iter_mut() {
            let tt = (step as f64 + 1.0) * dt;
            for fp in planes.iter_mut() {
                let w = 2.0 * std::f64::consts::PI * fp.freq * tt;
                let (cr, ci) = (w.cos() as f32, -w.sin() as f32);
                let ec = &e[fp.comp];
                let nv = fp.v.len();
                let nu = fp.u.len();
                for iu in 0..nu {
                    for iv in 0..nv {
                        let id = match *a {
                            0 => g.id(*idx, iu, iv),
                            1 => g.id(iu, *idx, iv),
                            _ => g.id(iu, iv, *idx),
                        };
                        let x = ec[id];
                        let q = iu * nv + iv;
                        fp.re[q] += x * cr;
                        fp.im[q] += x * ci;
                    }
                }
            }
        }
        // --- energy check
        if step % 200 == 0 {
            // field energy ~ Σ ε E² + μ0 H² (without the cell volumes; only the decay matters)
            let mut en = 0.0f64;
            for c3 in 0..3 {
                en += e[c3].iter().zip(&epsr[c3]).map(|(x, r)| EPS0 * (*r as f64) * (*x as f64).powi(2)).sum::<f64>();
                en += h[c3].iter().map(|x| MU0 * (*x as f64).powi(2)).sum::<f64>();
            }
            if !en.is_finite() {
                break;
            }
            emax = emax.max(en);
            on_progress(step, if emax > 0.0 { 10.0 * (en / emax).max(1e-30).log10() } else { 0.0 });
            if t > t_pulse && emax > 0.0 && 10.0 * (en / emax).max(1e-30).log10() < c.end_db {
                break;
            }
        }
    }
    let fields: Vec<FieldPlane> = mons.into_iter().flat_map(|m| m.2).collect();
    (dt, steps, vs, is, exc, fields)
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
        let record = !c.monitors.is_empty() && ex == exc_list[0];
        let (dt, steps, vs, is, _, mut fields) = run(c, &m, ex, record, &mut |s, db| on_progress(ex, s, db));
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
        // fields per volt of the incident wave: DFT·Δt / (a·√Z)
        for fp in fields.iter_mut() {
            let (a, _) = wave(ex, fp.freq);
            let zr = c.ports[ex].r.sqrt();
            let (ar, ai) = (a.0 * zr, a.1 * zr);
            let den = ar * ar + ai * ai;
            for q in 0..fp.re.len() {
                let (xr, xi) = (fp.re[q] as f64 * dt, fp.im[q] as f64 * dt);
                fp.re[q] = ((xr * ar + xi * ai) / den) as f32;
                fp.im[q] = ((xi * ar - xr * ai) / den) as f32;
            }
        }
        runs.push(RunResult { excited: ex, steps, dt, seconds: secs, s, fields });
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

    /// A lone pad (a disc in a plane) with conformal metal: the energy must decay, not grow
    /// (every face touching a partially free edge needs its free length).
    #[test]
    fn conformal_pad_stable() {
        let c: Case = serde_json::from_value(serde_json::json!({
            "name": "pad", "domain": [-1.5, -1.5, 0.0, 1.5, 1.5, 1.0], "board": [-1.5, -1.5, 1.5, 1.5],
            "layers": [{"z0": 0.0, "z1": 1.0, "er": 4.0}],
            "vias": [{"x": 0.0, "y": 0.0, "r": 0.0, "z0": 0.5, "z1": 0.5, "pad_r": 0.33, "pads": [0.5]}],
            "ports": [{"x": 1.0, "y0": 0.0, "y1": 0.0, "z0": 0.0, "z1": 0.5}],
            "fmax": 30e9, "mesh": {"dfine": 0.05, "dmax": 0.1}, "boundary": "pec", "end_db": -300, "max_steps": 12001, "nf": 4
        }))
        .unwrap();
        let mut last = 0.0;
        simulate(&c, None, &mut |_, s, db| {
            if s > 4000 {
                last = db
            }
        });
        assert!(last < -20.0, "energy {last} dB");
    }

    /// Cylindrical cavity R = 3 mm, εr 4, 0.1 mm cells, conformal wall: TM010 within 0.2 % of
    /// the analytic 2.404826·c / (2π R √εr) = 19.124 GHz.
    #[test]
    fn cavity_resonance_conformal() {
        let m = 3.4;
        let c: Case = serde_json::from_value(serde_json::json!({
            "name": "cav", "domain": [-m, -m, 0.0, m, m, 0.5], "board": [-m, -m, m, m],
            "layers": [{"z0": 0.0, "z1": 0.5, "er": 4.0, "tand": 0.03}],
            "sheets": [{"z": 0.0, "rect": [-m, -m, m, m]}, {"z": 0.5, "rect": [-m, -m, m, m]}],
            "vias": [{"x": 0.0, "y": 0.0, "r": 3.0, "z0": 0.0, "z1": 0.5, "outer": true}],
            "ports": [{"x": 1.5, "y0": 0.0, "y1": 0.0, "z0": 0.0, "z1": 0.5}],
            "fmax": 30e9, "mesh": {"dfine": 0.1, "dmax": 0.1}, "boundary": "pec", "end_db": -60, "max_steps": 60000, "nf": 3000
        }))
        .unwrap();
        let r = simulate(&c, None, &mut |_, _, _| {});
        let re: Vec<f64> = r.runs[0].s[0]
            .iter()
            .map(|&(a, b)| {
                // Z = 50 (1 + S) / (1 − S)
                let (nr, ni, dr, di) = (1.0 + a, b, 1.0 - a, -b);
                50.0 * (nr * dr + ni * di) / (dr * dr + di * di)
            })
            .collect();
        let (lo, hi) = (r.freq.iter().position(|&f| f > 17e9).unwrap(), r.freq.iter().position(|&f| f > 21e9).unwrap());
        let k = (lo..hi).max_by(|&a, &b| re[a].total_cmp(&re[b])).unwrap();
        let fa = 2.404826 * C0 / (2.0 * std::f64::consts::PI * 3e-3 * 2.0);
        let err = (r.freq[k] / fa - 1.0).abs();
        eprintln!("cavity {:.3} GHz, analytic {:.3} GHz", r.freq[k] / 1e9, fa / 1e9);
        assert!(err < 0.002, "{err}");
    }

    /// Port at a 1 mm microstrip, a series 10 Ω + 1 nH + 1 pF to ground at its end: the series
    /// resonance with the line's own 0.35 nH lies at 1/(2π√(1.35 nH · 1 pF)) = 4.33 GHz, Re Z ≈ 10 Ω.
    #[test]
    fn lumped_rlc() {
        let w = 0.35;
        let c: Case = serde_json::from_value(serde_json::json!({
            "name": "rlc", "domain": [-1.5, -1.5, -0.5, 1.5, 1.5, 1.2], "board": [-1.5, -1.5, 1.5, 1.5],
            "layers": [{"z0": 0.0, "z1": 0.2, "er": 4.3}],
            "sheets": [{"z": 0.0, "rect": [-1.5, -1.5, 1.5, 1.5]}, {"z": 0.2, "rect": [-0.5, -w / 2.0, 0.5, w / 2.0]}],
            "ports": [{"x": -0.5, "y0": -w / 2.0, "y1": w / 2.0, "z0": 0.0, "z1": 0.2}],
            "lumped": [{"x": 0.5, "y0": -w / 2.0, "y1": w / 2.0, "z0": 0.0, "z1": 0.2, "r": 10.0, "l": 1e-9, "c": 1e-12}],
            "fmax": 15e9, "mesh": {"dfine": 0.05}, "end_db": -50, "max_steps": 100000, "nf": 300
        }))
        .unwrap();
        let r = simulate(&c, None, &mut |_, _, _| {});
        let z: Vec<(f64, f64)> = r.runs[0].s[0]
            .iter()
            .map(|&(a, b)| {
                let (nr, ni, dr, di) = (1.0 + a, b, 1.0 - a, -b);
                let d = dr * dr + di * di;
                (50.0 * (nr * dr + ni * di) / d, 50.0 * (ni * dr - nr * di) / d)
            })
            .collect();
        // first sign change of Im Z from − to +
        let k = (1..z.len()).find(|&k| z[k - 1].1 < 0.0 && z[k].1 >= 0.0).unwrap();
        eprintln!("series resonance {:.3} GHz, Re {:.2} Ω", r.freq[k] / 1e9, z[k].0);
        assert!((r.freq[k] / 4.33e9 - 1.0).abs() < 0.03 && (z[k].0 - 10.0).abs() < 1.5);
    }
}

//! Physics-based reduced via model (ported from validation/fdtd/via_pbm.py, checked
//! against the kisi-fdtd 3D references, see validation/fdtd/TESTS.md).
//!
//! A via is a ladder along its barrel:
//! * nodes: the planes it crosses (with an antipad) and the layers where the traces join
//!   it; inside a plane pair extra nodes every ≤ 0.2 mm;
//! * shunt at every node: the via–plate capacitance from an axisymmetric r–z Laplace solve
//!   (barrel and pads at 1 V, planes with their antipads at 0 V, layered εr); the barrel
//!   charge goes to the nearest node;
//! * series between two nodes inside a plane pair: the radial (parallel-plate) impedance
//!   Z = ωμd/4 · H0⁽²⁾(k a) with the return vias of that plane pair as shorted ports
//!   (Z_in = Z00 − z0r Zrr⁻¹ zr0). A trace that joins inside the pair weights the pair by the
//!   height of its group, (d_g/d)²;
//! * between a surface layer and the outermost plane: a short coax with the antipad as
//!   outer radius;
//! * stubs: the ladder beyond the entry and exit nodes ends open;
//! * the trace over the antipad opening as a short microstrip over the next plane, and the
//!   feed trace removed over max(pad radius, antipad radius) (the traces run to the via
//!   centre in the layout and in the 2×thru fixtures).
//!
//! Pairs: the partner via is a third port of the plane-pair matrix; the odd mode uses
//! Z_PP − Z_PN and scales the barrel capacitance inside the plane pairs by
//! ln(R/a)/ln(p/a).
//!
//! Return groups (`Post::joins`, option `--via-returns`, validation/fdtd/BOARD_RETURNS.md): a
//! return via shorts only the pairs whose two planes it joins. Elsewhere it is a small ladder of
//! its own – a span with its own current in every pair it does not short, a node with its
//! capacitance to the plane at every plane it passes through an opening ([`post_plane_cap`]) –
//! coupled to the via through the pairs' matrices: a GND via across a supply plane is a floating
//! return, a GND via that ends at a supply plane a capacitive one. Capacitors between two planes
//! nearby (`CapPost`) are ports closed by their impedance. The returns are reduced onto the via's
//! current moment in each pair ([`PbmVia::abcd`] then solves the ladder with one unknown per such
//! pair).
//!
//! Geometry in mm (z upwards), electrical quantities in SI.

use crate::num::{Cx, C0, EPS0, MU0};
use crate::rf::Abcd;
use serde::{Deserialize, Serialize};
use std::f64::consts::PI;

/// largest span between ladder nodes inside a plane pair (mm)
const SUB: f64 = 0.2;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Slab {
    pub z0: f64,
    pub z1: f64,
    pub er: f64,
    pub tand: f64,
}

/// Plane (reference copper) at height `z` with the antipad radius around the via;
/// `None`: not opened at the via.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Plane {
    pub z: f64,
    pub antipad: Option<f64>,
}

/// A second barrel near the via (return via or the partner of a pair): offset (mm),
/// radius (mm) and the z range it spans.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Post {
    pub dx: f64,
    pub dy: f64,
    pub r: f64,
    pub z0: f64,
    pub z1: f64,
    /// return groups: heights (mm) of the planes this return via is joined to (the planes of its
    /// net). It shorts a plane pair only when it joins both of its planes; elsewhere it is a ladder
    /// of its own, with a node and its capacitance at every plane it passes through an opening –
    /// between two planes it joins a floating return that ties the pairs between them together,
    /// beyond its last joined plane a capacitive return. None: a short in every pair it spans (the
    /// ladder before return groups)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub joins: Option<Vec<f64>>,
    /// return groups: the radius (mm) of the post's opening in each plane it passes without joining
    /// it, (plane height, radius); a plane not listed: post radius + 0.2 mm (an estimate)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub open_r: Vec<(f64, f64)>,
}

/// What a return post is in the plane pair between the planes at `za` > `zb` (mm).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Role {
    /// joins both planes: a short of the pair
    Short,
    /// joins a plane above (`.0`, ≥ za) and one below (`.1`, ≤ zb) but not both of this pair: a
    /// floating return of the segment between those planes
    Float(f64, f64),
    /// joins planes on one side only: closes the pair through its capacitance to the planes it
    /// passes on the other side
    Cap,
    /// no current in this pair
    Open,
}

impl Post {
    /// the post's role in the plane pair between the planes at `za` > `zb` (mm)
    pub fn role(&self, za: f64, zb: f64) -> Role {
        let spans = |a: f64, b: f64| self.z0 <= b + 1e-9 && self.z1 >= a - 1e-9;
        match &self.joins {
            None => {
                if spans(za, zb) {
                    Role::Short
                } else {
                    Role::Open
                }
            }
            Some(js) => {
                let has = |z: f64| js.iter().any(|&j| (j - z).abs() < 1e-6);
                if has(za) && has(zb) && spans(za, zb) {
                    return Role::Short;
                }
                let up = js.iter().copied().filter(|&j| j >= za - 1e-6).fold(f64::INFINITY, f64::min);
                let dn = js.iter().copied().filter(|&j| j <= zb + 1e-6).fold(f64::NEG_INFINITY, f64::max);
                if up.is_finite() && dn.is_finite() && spans(up, dn) {
                    Role::Float(up, dn)
                } else if (up.is_finite() || dn.is_finite()) && spans(za, zb) {
                    Role::Cap
                } else {
                    Role::Open
                }
            }
        }
    }
}

/// A capacitor between two planes of the via's plane pairs (decoupling) as a lossy post: offset
/// (mm) and radius (mm) of its post, the heights of the two planes it joins (mm, `z_top` >
/// `z_bot`) and its impedance R + jωL + 1/(jωC) (ESR in Ω, ESL with the mounting in H, C in F;
/// None: no series capacitance). On infinite plates it is a further port of the pair, closed by
/// this impedance; a finite cavity (`via_cavity`) has its capacitors in its environment instead.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CapPost {
    pub dx: f64,
    pub dy: f64,
    pub r: f64,
    pub z_top: f64,
    pub z_bot: f64,
    pub r_ohm: f64,
    pub l: f64,
    pub c: Option<f64>,
}

impl CapPost {
    fn z(&self, w: f64) -> Cx {
        Cx::new(self.r_ohm, w * self.l - self.c.map_or(0.0, |c| 1.0 / (w * c)))
    }
}

/// Trace joining the via: layer height, height of its reference plane, width (mm) and the
/// line parameters of the trace (for removing the part that the ladder models itself).
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Feed {
    pub z: f64,
    pub z_ref: f64,
    pub w: f64,
    pub zc: f64,
    pub eeff: f64,
    /// length of this trace next to the via that the caller has already left out (mm): kisi's
    /// channel leaves the unreferenced pieces over the antipad to the ladder
    #[serde(default)]
    pub dropped: f64,
    /// model the trace over the antipad opening (pad edge → antipad edge)
    pub antipad_line: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViaGeom {
    /// dielectric slabs; the first slab that contains a height wins on shared faces
    pub slabs: Vec<Slab>,
    pub planes: Vec<Plane>,
    /// barrel radius and z range (mm)
    pub r: f64,
    pub z0: f64,
    pub z1: f64,
    /// pad heights and pad radius (mm)
    pub pads: Vec<f64>,
    pub pad_r: f64,
    pub returns: Vec<Post>,
    pub partner: Option<Post>,
    /// entry and exit trace
    pub feeds: [Feed; 2],
    /// loss frequency: with Some(f_l) the loss tangent is scaled by f_l/f (the 3D
    /// references fix σ at f_l); None: constant tanδ
    pub f_loss: Option<f64>,
    /// junction model fitted to the 3D references (default): series inductance μ0/(2π)·pad
    /// radius where each trace enters the via, the trace over the antipad scaled from its own
    /// line, striplines kept up to the via centre. false: the Python prototype (via_pbm.py).
    #[serde(default = "yes")]
    pub junction: bool,
    /// conductivity of barrel and planes (S/m) for their skin-effect loss; None: lossless metal
    /// (as the 3D references)
    #[serde(default)]
    pub sigma: Option<f64>,
    /// capacitors between two planes near the via (return groups: the return of a pair of two nets)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub caps: Vec<CapPost>,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Mode {
    Single,
    Odd,
    Even,
}

/// Grid of the r–z Laplace solve.
#[derive(Debug, Clone, Copy)]
pub enum Grid {
    /// uniform cells of `dr` (mm) up to `rmax` (the Python prototype: 0.01, 3.0)
    Uniform { dr: f64, rmax: f64 },
    /// graded: `fine` (mm) at conductor edges, growing by `ratio` up to `coarse` (radial) and
    /// `zcoarse` (axial); the charge of a grid row whose cell straddles the boundary between two
    /// ladder nodes is split by overlap
    Graded { fine: f64, coarse: f64, zcoarse: f64, ratio: f64, rmax: f64 },
}

impl Default for Grid {
    fn default() -> Self {
        Grid::Graded { fine: 0.005, coarse: 0.1, zcoarse: 0.05, ratio: 1.3, rmax: 3.0 }
    }
}

#[derive(Debug, Clone, Serialize)]
enum SpanKind {
    /// inductance of the short coax outside the plane pairs (H)
    Outer(f64),
    /// plane pair: group index and d·(shared-cavity factor) in m
    Pair { group: usize, scale: f64 },
}

#[derive(Debug, Clone, Serialize)]
struct Group {
    er: f64,
    tand: f64,
    /// points: signal, optional partner, returns that short the pair, capacitors (the last
    /// `loads.len()`): (x m, y m, radius m)
    pts: Vec<(f64, f64, f64)>,
    partner: bool,
    /// height of the plane pair (m), for the conductor loss of the plates
    d: f64,
    /// radius on the diagonal, distances off it (m), row-major
    rr: Vec<f64>,
    /// capacitors between the planes (the trailing points), on infinite plates only
    #[serde(skip)]
    loads: Vec<CapPost>,
    /// finite cavity: index into `PbmVia::envs` and the environment's port of every point
    #[serde(skip)]
    env: Option<(usize, Vec<usize>)>,
    /// heights of the two planes (m)
    #[serde(skip)]
    pair: (f64, f64),
}

/// A plane pair with returns that have spans of their own in it (return groups): what couples the
/// via's spans to them, and them to each other.
#[derive(Debug, Clone)]
struct PairM {
    /// heights of the two planes (m)
    pair: (f64, f64),
    er: f64,
    tand: f64,
    /// points: signal and partner (`nsig`: 0 if the barrel does not cross the pair, 1, or 2 with
    /// the partner), the returns that short the pair (`nshort`), capacitors (`loads`), then the
    /// returns with a span of their own in this pair (`segs`)
    pts: Vec<(f64, f64, f64)>,
    nsig: usize,
    nshort: usize,
    loads: Vec<CapPost>,
    /// the return span (`PbmVia::rspans`) of every return with a span of its own here
    segs: Vec<usize>,
    rr: Vec<f64>,
    /// finite cavity: index into `PbmVia::envs` and the environment's port of every point but the
    /// capacitors (the environment has its own)
    env: Option<(usize, Vec<usize>)>,
}

/// The coupling terms of the pairs with return spans at one frequency, per pair: via ← return (per
/// metre of the span), return ← via (per metre of the span), return × return (per metre of the
/// pair's height; q × q row-major)
type PairZ = Vec<(Vec<Cx>, Vec<Cx>, Vec<Cx>)>;

/// A finite plane-pair cavity for the ladder (`via_cavity`): the plane pair between the ladder's
/// planes at `z_top` > `z_bot` (mm), its environment (posts, edges, loads) and the via's position
/// in the environment's frame (m). The environment's ports must contain the via (and the partner
/// and return vias the ladder uses in that pair) at their positions; the ladder then takes the
/// pair's impedance matrix from it instead of the infinite radial line.
#[derive(Clone)]
pub struct CavityLink {
    pub z_top: f64,
    pub z_bot: f64,
    pub env: std::rc::Rc<crate::via_cavity::PairEnv>,
    pub origin: (f64, f64),
}

impl std::fmt::Debug for CavityLink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "CavityLink {{ z_top: {}, z_bot: {}, ports: {}, origin: {:?} }}", self.z_top, self.z_bot, self.env.ports().len(), self.origin)
    }
}

/// Built model; evaluate with [`PbmVia::abcd`].
#[derive(Debug, Clone, Serialize)]
pub struct PbmVia {
    pub nodes: Vec<f64>,
    /// node capacitances (F)
    pub cap: Vec<f64>,
    node_tand: Vec<f64>,
    spans: Vec<SpanKind>,
    groups: Vec<Group>,
    i_in: usize,
    i_out: usize,
    mode: Mode,
    f_loss: Option<f64>,
    /// trace over the antipad per side: (Z, εr, length m)
    ap_line: [Option<(f64, f64, f64)>; 2],
    /// feed removal per side: (Z, εeff, length m, negative)
    feed_cut: [(f64, f64, f64); 2],
    /// junction inductance per side (H)
    l_port: [f64; 2],
    /// conductor loss: conductivity (S/m), barrel radius (m) and the barrel length of every span (m)
    sigma: Option<f64>,
    a_m: f64,
    span_len: Vec<f64>,
    /// unknowns of the Laplace solve
    pub laplace_n: usize,
    /// plane-pair impedances per metre divided by f on the grid k·`DF_GRID` (filled on demand)
    #[serde(skip)]
    zgrid: std::cell::RefCell<Vec<Option<Vec<Cx>>>>,
    /// finite cavities of the plane pairs (empty: infinite plates)
    #[serde(skip)]
    envs: Vec<CavityLink>,
    /// evaluate without the cavities (the local, infinite-plate ladder): stub notch, impedance
    #[serde(skip)]
    local: std::cell::Cell<bool>,
    /// return groups: every return post that does not short a pair is a ladder of its own – nodes
    /// at the planes it passes without joining them (post, plane height mm, its capacitance to the
    /// plane F), spans in the pairs it does not short (post, top node, bottom node; None: a plane it
    /// joins); the pairs with such spans, and for every span of the via its pair in `pairs`
    #[serde(skip)]
    rnodes: Vec<(usize, f64, f64)>,
    #[serde(skip)]
    rspans: Vec<(usize, Option<usize>, Option<usize>)>,
    #[serde(skip)]
    pairs: Vec<PairM>,
    #[serde(skip)]
    span_pair: Vec<Option<usize>>,
    /// the returns reduced onto the via's current moments in the pairs it crosses (`return_w`),
    /// divided by f on the grid k·`DF_GRID` (filled on demand)
    #[serde(skip)]
    wgrid: std::cell::RefCell<Vec<Option<Vec<Cx>>>>,
}

/// frequency step of the plane-pair impedance grid (Hz): Z/f is smooth in f (no resonance of
/// the return-via cage below ~50 GHz for returns within 1 mm), so it is interpolated linearly
const DF_GRID: f64 = 0.25e9;

fn er_at(slabs: &[Slab], z: f64) -> (f64, f64) {
    for s in slabs {
        if s.z0 - 1e-9 <= z && z <= s.z1 + 1e-9 {
            return (s.er, s.tand);
        }
    }
    (1.0, 0.0)
}

fn round6(x: f64) -> f64 {
    (x * 1e6).round() / 1e6
}

fn sorted_unique(mut v: Vec<f64>, tol: f64) -> Vec<f64> {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v.dedup_by(|a, b| (*a - *b).abs() < tol);
    v
}

/// thickness-weighted εr between za > zb
fn er_weighted(slabs: &[Slab], za: f64, zb: f64) -> Option<f64> {
    let (mut num, mut den) = (0.0, 0.0);
    for s in slabs {
        let o = (s.z1.min(za) - s.z0.max(zb)).max(0.0);
        num += o * s.er;
        den += o;
    }
    if den > 0.0 {
        Some(num / den)
    } else {
        None
    }
}

// --- Bessel / Hankel --------------------------------------------------------------------

fn cln(z: Cx) -> Cx {
    Cx::new(0.5 * norm2(z).ln(), z.arg())
}

/// |z|² (no hypot: in WebAssembly hypot and its fused multiply-adds run in software)
fn norm2(z: Cx) -> f64 {
    z.re * z.re + z.im * z.im
}

/// H0⁽²⁾(z) = J0(z) − j Y0(z) for complex z (Re z > 0): power series for |z| < 12, Hankel
/// asymptotic expansion beyond.
pub fn hankel2_0(z: Cx) -> Cx {
    if norm2(z) < 144.0 {
        let q = (z * z).scale(0.25);
        let mut term = Cx::new(1.0, 0.0); // (−q)^k/(k!)²
        let mut j0 = term;
        let mut ysum = Cx::new(0.0, 0.0);
        let mut h = 0.0;
        for k in 1..80 {
            let kf = k as f64;
            term = Cx::new(0.0, 0.0) - term * q.scale(1.0 / (kf * kf));
            h += 1.0 / kf;
            j0 = j0 + term;
            // Y0 series: (2/π) Σ (−1)^{k+1} H_k q^k/(k!)² = −(2/π) Σ H_k term_k
            ysum = ysum - term.scale(h);
            if k > 4 && norm2(term) < 1e-34 * norm2(j0).max(1e-300) {
                break;
            }
        }
        let gamma = 0.577_215_664_901_532_9;
        let lz = cln(z.scale(0.5)) + Cx::new(gamma, 0.0);
        let y0 = (lz * j0 + ysum).scale(2.0 / PI);
        j0 - Cx::new(0.0, 1.0) * y0
    } else {
        // H0⁽²⁾(z) ~ √(2/(πz)) e^{−j(z−π/4)} Σ (−j)^k a_k / z^k, a_k = a_{k−1}·(−(2k−1)²)/(8k)
        let mut sum = Cx::new(1.0, 0.0);
        let mut a = 1.0;
        let mut zp = Cx::new(1.0, 0.0);
        let mj = Cx::new(0.0, -1.0);
        let mut mjk = Cx::new(1.0, 0.0);
        let mut last = f64::INFINITY;
        for k in 1..40 {
            let kf = k as f64;
            a *= -(2.0 * kf - 1.0).powi(2) / (8.0 * kf);
            zp = zp * z;
            mjk = mjk * mj;
            let t = mjk.scale(a) / zp;
            let ta = t.abs();
            if ta > last {
                break;
            }
            sum = sum + t;
            last = ta;
            if ta < 1e-17 {
                break;
            }
        }
        let pre = (Cx::new(2.0 / PI, 0.0) / z).sqrt();
        let ph = (Cx::new(0.0, -1.0) * (z - Cx::new(PI / 4.0, 0.0))).exp();
        pre * ph * sum
    }
}

/// Solve the complex system a x = b (small, dense, partial pivoting); a is n×n row-major.
fn csolve(mut a: Vec<Cx>, mut b: Vec<Cx>, n: usize, nrhs: usize) -> Vec<Cx> {
    // b is n×nrhs row-major
    for c in 0..n {
        let p = (c..n).max_by(|&i, &j| norm2(a[i * n + c]).partial_cmp(&norm2(a[j * n + c])).unwrap()).unwrap();
        if p != c {
            for k in 0..n {
                a.swap(c * n + k, p * n + k);
            }
            for k in 0..nrhs {
                b.swap(c * nrhs + k, p * nrhs + k);
            }
        }
        let inv = a[c * n + c].inv();
        for i in c + 1..n {
            let f = a[i * n + c] * inv;
            if f.re == 0.0 && f.im == 0.0 {
                continue;
            }
            for k in c..n {
                a[i * n + k] = a[i * n + k] - f * a[c * n + k];
            }
            for k in 0..nrhs {
                b[i * nrhs + k] = b[i * nrhs + k] - f * b[c * nrhs + k];
            }
        }
    }
    for c in (0..n).rev() {
        let inv = a[c * n + c].inv();
        for k in 0..nrhs {
            let mut s = b[c * nrhs + k];
            for j in c + 1..n {
                s = s - a[c * n + j] * b[j * nrhs + k];
            }
            b[c * nrhs + k] = s * inv;
        }
    }
    b
}

/// Schur complement of a matrix (n×n row-major): the matrix over the points `keep` with the
/// points `elim` closed (their diagonal increased by `add`, zero for a short),
/// Z_kk − Z_ke (Z_ee + diag(add))⁻¹ Z_ek.
fn schur(z: &[Cx], n: usize, keep: &[usize], elim: &[usize], add: &[Cx]) -> Vec<Cx> {
    let (nk, ne) = (keep.len(), elim.len());
    let mut out: Vec<Cx> = (0..nk * nk).map(|q| z[keep[q / nk] * n + keep[q % nk]]).collect();
    if ne == 0 {
        return out;
    }
    let mut ee: Vec<Cx> = (0..ne * ne).map(|q| z[elim[q / ne] * n + elim[q % ne]]).collect();
    for i in 0..ne {
        ee[i * ne + i] = ee[i * ne + i] + add[i];
    }
    let ek: Vec<Cx> = (0..ne * nk).map(|q| z[elim[q / nk] * n + keep[q % nk]]).collect();
    let x = csolve(ee, ek, ne, nk);
    for a in 0..nk {
        for c in 0..nk {
            let mut s = Cx::new(0.0, 0.0);
            for q in 0..ne {
                s = s + z[keep[a] * n + elim[q]] * x[q * nk + c];
            }
            out[a * nk + c] = out[a * nk + c] - s;
        }
    }
    out
}

// --- r–z Laplace -------------------------------------------------------------------------

fn uniform(a: f64, b: f64, d: f64) -> Vec<f64> {
    let n = (((b - a) / d) - 1e-9).ceil().max(1.0) as usize;
    (1..=n).map(|k| a + (b - a) * k as f64 / n as f64).collect()
}

/// graded points between fixed lines: `fine` next to every fixed line, growing by `ratio`
/// up to `coarse` (symmetric in each interval)
fn graded(fixed: &[f64], fine: f64, coarse: f64, ratio: f64) -> Vec<f64> {
    let mut out = vec![fixed[0]];
    for w in fixed.windows(2) {
        let (a, b) = (w[0], w[1]);
        let len = b - a;
        // half-interval cell sizes
        let mut cells = vec![];
        let mut s = fine;
        let mut tot = 0.0;
        while tot + s < len / 2.0 {
            cells.push(s);
            tot += s;
            s = (s * ratio).min(coarse);
        }
        let mut sizes: Vec<f64> = cells.clone();
        let rest = len - 2.0 * tot;
        let mut mid = vec![];
        if rest > 1e-12 {
            let n = (rest / s).ceil().max(1.0) as usize;
            mid = vec![rest / n as f64; n];
        }
        sizes.extend(mid);
        sizes.extend(cells.iter().rev());
        let mut x = a;
        for (k, d) in sizes.iter().enumerate() {
            x += d;
            out.push(if k + 1 == sizes.len() { b } else { x });
        }
    }
    out
}

/// z, charge and z extent of every conductor-1 grid node, number of unknowns
type Charges = std::rc::Rc<(Vec<f64>, Vec<f64>, usize, Vec<(f64, f64)>)>;

thread_local! {
    /// Laplace results by geometry: the vias of a board come in few types
    static CACHE: std::cell::RefCell<std::collections::HashMap<String, Charges>> = Default::default();
}

/// Barrel charge per grid row (z mm, F per volt), for diagnostics.
pub fn charge_rows(g: &ViaGeom, grid: Grid) -> Vec<(f64, f64)> {
    let (zq, qq, _, _) = rz_solve(g, grid);
    let mut rows: Vec<(f64, f64)> = vec![];
    for (&z, &q) in zq.iter().zip(&qq) {
        match rows.iter_mut().find(|r| (r.0 - z).abs() < 1e-9) {
            Some(r) => r.1 += q,
            None => rows.push((z, q)),
        }
    }
    rows
}

/// Empty the Laplace cache (for timing measurements).
pub fn clear_cache() {
    CACHE.with(|c| c.borrow_mut().clear());
}

/// [`rz_solve`] through a per-thread cache keyed by everything the solve depends on.
fn rz_charges(g: &ViaGeom, grid: Grid) -> Charges {
    let q = |x: f64| (x * 1e6).round() as i64;
    let mut key = format!("{grid:?}|{}|{}|{}|{}|", q(g.r), q(g.z0), q(g.z1), q(g.pad_r));
    for s in &g.slabs {
        key += &format!("s{},{},{};", q(s.z0), q(s.z1), q(s.er));
    }
    for p in &g.planes {
        key += &format!("p{},{};", q(p.z), p.antipad.map_or(-1, q));
    }
    for z in &g.pads {
        key += &format!("d{};", q(*z));
    }
    if let Some(c) = CACHE.with(|c| c.borrow().get(&key).cloned()) {
        return c;
    }
    let r = std::rc::Rc::new(rz_solve(g, grid));
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        if c.len() > 512 {
            c.clear();
        }
        c.insert(key, r.clone());
    });
    r
}

/// z and charge (F per volt) of every conductor-1 grid node, and the number of unknowns.
fn rz_solve(g: &ViaGeom, grid: Grid) -> (Vec<f64>, Vec<f64>, usize, Vec<(f64, f64)>) {
    let mut zc: Vec<f64> = g.planes.iter().map(|p| round6(p.z)).collect();
    zc.extend(g.pads.iter().map(|&z| round6(z)));
    zc.push(round6(g.z0));
    zc.push(round6(g.z1));
    for s in &g.slabs {
        zc.push(round6(s.z0));
        zc.push(round6(s.z1));
    }
    let zc = sorted_unique(zc, 1e-9);
    let (zlo, zhi) = (zc[0] - 0.5, zc[zc.len() - 1] + 0.5);
    let mut fixed = zc.clone();
    fixed.push(zlo);
    fixed.push(zhi);
    let fixed = sorted_unique(fixed, 1e-9);
    // radial fixed lines: barrel, pad and antipad edges
    let (z, r): (Vec<f64>, Vec<f64>) = match grid {
        Grid::Uniform { dr, rmax } => {
            let mut zs = vec![fixed[0]];
            for w in fixed.windows(2) {
                zs.extend(uniform(w[0], w[1], dr));
            }
            let nr = (rmax / dr).round() as usize;
            (zs, (0..=nr).map(|i| i as f64 * dr).collect())
        }
        Grid::Graded { fine, coarse, zcoarse, ratio, rmax } => {
            let zs = graded(&fixed, fine, zcoarse, ratio);
            let mut rf = vec![0.0, g.r, g.pad_r, rmax];
            rf.extend(g.planes.iter().filter_map(|p| p.antipad).filter(|&b| b < rmax));
            let rf = sorted_unique(rf.into_iter().filter(|&x| x >= 0.0 && x <= rmax).collect(), 1e-6);
            (zs, graded(&rf, fine, coarse, ratio))
        }
    };
    let (nr, nz) = (r.len(), z.len());
    let idx = |i: usize, j: usize| j * nr + i;
    let mut cond = vec![-1i8; nr * nz];
    for (j, &zz) in z.iter().enumerate() {
        let on_pad = g.pads.iter().any(|&pz| (zz - pz).abs() < 1e-6);
        for (i, &rr) in r.iter().enumerate() {
            let k = idx(i, j);
            if g.z0 - 1e-9 <= zz && zz <= g.z1 + 1e-9 && rr <= g.r + 1e-9 {
                cond[k] = 1;
            }
            if on_pad && rr <= g.pad_r + 1e-9 {
                cond[k] = 1;
            }
            for p in &g.planes {
                if (zz - p.z).abs() < 1e-6 && p.antipad.map_or(true, |b| rr >= b - 1e-9) {
                    cond[k] = 0;
                }
            }
        }
    }
    let ecell: Vec<f64> = (0..nz - 1).map(|j| er_at(&g.slabs, 0.5 * (z[j] + z[j + 1])).0).collect();
    let mut links: Vec<(usize, usize, f64)> = Vec::with_capacity(2 * nr * nz);
    let rl = |i: usize| if i == 0 { 0.0 } else { 0.5 * (r[i - 1] + r[i]) };
    let rh = |i: usize| if i + 1 == nr { r[i] } else { 0.5 * (r[i] + r[i + 1]) };
    for j in 0..nz {
        let dzh = 0.5 * if j + 1 < nz { z[j + 1] - z[j] } else { 0.0 } + 0.5 * if j > 0 { z[j] - z[j - 1] } else { 0.0 };
        let e = {
            let (mut s, mut n) = (0.0, 0.0);
            for jj in [j as isize - 1, j as isize] {
                if jj >= 0 && (jj as usize) < nz - 1 {
                    s += ecell[jj as usize];
                    n += 1.0;
                }
            }
            s / n
        };
        for i in 0..nr {
            let p = idx(i, j);
            if i + 1 < nr {
                let g_ = EPS0 * e * 2.0 * PI * 0.5 * (r[i] + r[i + 1]) * dzh / (r[i + 1] - r[i]) * 1e-3;
                links.push((p, idx(i + 1, j), g_));
            }
            if j + 1 < nz {
                let area = PI * (rh(i).powi(2) - rl(i).powi(2));
                let g_ = EPS0 * ecell[j] * area / (z[j + 1] - z[j]) * 1e-3;
                links.push((p, idx(i, j + 1), g_));
            }
        }
    }
    let mut fmap = vec![usize::MAX; nr * nz];
    let mut nf = 0;
    for k in 0..nr * nz {
        if cond[k] < 0 {
            fmap[k] = nf;
            nf += 1;
        }
    }
    let fix = |k: usize| if cond[k] == 1 { 1.0 } else { 0.0 };
    let mut rhs = vec![0.0; nf];
    let mut diag = vec![0.0; nf];
    let mut trip: Vec<(usize, usize, f64)> = Vec::with_capacity(links.len() * 2 + nf);
    for &(p, q, gg) in &links {
        for (u, w) in [(p, q), (q, p)] {
            if fmap[u] != usize::MAX {
                diag[fmap[u]] += gg;
                if fmap[w] != usize::MAX {
                    trip.push((fmap[u], fmap[w], -gg));
                } else {
                    rhs[fmap[u]] += gg * fix(w);
                }
            }
        }
    }
    for (k, d) in diag.iter().enumerate() {
        trip.push((k, k, *d));
    }
    let sol = kisi_sparse::solve_spd_triplets(nf, &trip, &rhs).unwrap_or_else(|_| vec![0.0; nf]);
    let phi = |k: usize| if fmap[k] != usize::MAX { sol[fmap[k]] } else { fix(k) };
    let mut q = vec![0.0; nr * nz];
    for &(p, qq, gg) in &links {
        let d = gg * (phi(p) - phi(qq));
        q[p] += d;
        q[qq] -= d;
    }
    let mut zq = vec![];
    let mut qq = vec![];
    let mut ext = vec![];
    for k in 0..nr * nz {
        if cond[k] == 1 {
            let j = k / nr;
            zq.push(z[j]);
            qq.push(q[k]);
            ext.push((if j > 0 { 0.5 * (z[j - 1] + z[j]) } else { z[j] }, if j + 1 < nz { 0.5 * (z[j] + z[j + 1]) } else { z[j] }));
        }
    }
    (zq, qq, nf, ext)
}

/// Total charge (F per volt) on the conductors at 1 V of an axisymmetric r–z Laplace problem: a
/// barrel (radius, z range; mm) at 1 V if given, planes at height z with a round opening (None:
/// solid) at 1 V or 0 V, layered εr, Neumann at the domain's edges (graded grid as `rz_solve`).
fn rz_total(slabs: &[Slab], barrel: Option<(f64, f64, f64)>, planes: &[(f64, Option<f64>, bool)], rmax: f64) -> f64 {
    let (fine, coarse, zcoarse, ratio) = (0.015, 0.1, 0.05, 1.4);
    let mut zf: Vec<f64> = planes.iter().map(|p| round6(p.0)).collect();
    if let Some((_, z0, z1)) = barrel {
        zf.push(round6(z0));
        zf.push(round6(z1));
    }
    let (lo, hi) = zf.iter().fold((f64::MAX, f64::MIN), |(l, h), &z| (l.min(z), h.max(z)));
    for s in slabs {
        for z in [s.z0, s.z1] {
            if z > lo - 0.3 && z < hi + 0.3 {
                zf.push(round6(z));
            }
        }
    }
    zf.push(round6(lo - 0.3));
    zf.push(round6(hi + 0.3));
    let zs = graded(&sorted_unique(zf, 1e-9), fine, zcoarse, ratio);
    let mut rf = vec![0.0, rmax];
    if let Some((a, _, _)) = barrel {
        rf.push(a);
    }
    rf.extend(planes.iter().filter_map(|p| p.1).filter(|&b| b < rmax));
    let rs = graded(&sorted_unique(rf, 1e-6), fine, coarse, ratio);
    let (nr, nz) = (rs.len(), zs.len());
    let idx = |i: usize, j: usize| j * nr + i;
    let mut cond = vec![-1i8; nr * nz];
    for (j, &zz) in zs.iter().enumerate() {
        for (i, &rr) in rs.iter().enumerate() {
            if let Some((a, z0, z1)) = barrel {
                if z0 - 1e-9 <= zz && zz <= z1 + 1e-9 && rr <= a + 1e-9 {
                    cond[idx(i, j)] = 1;
                }
            }
            for p in planes {
                if (zz - p.0).abs() < 1e-6 && p.1.map_or(true, |b| rr >= b - 1e-9) {
                    cond[idx(i, j)] = if p.2 { 1 } else { 0 };
                }
            }
        }
    }
    let ecell: Vec<f64> = (0..nz - 1).map(|j| er_at(slabs, 0.5 * (zs[j] + zs[j + 1])).0).collect();
    let rl = |i: usize| if i == 0 { 0.0 } else { 0.5 * (rs[i - 1] + rs[i]) };
    let rh = |i: usize| if i + 1 == nr { rs[i] } else { 0.5 * (rs[i] + rs[i + 1]) };
    let mut links: Vec<(usize, usize, f64)> = Vec::with_capacity(2 * nr * nz);
    for j in 0..nz {
        let dzh = 0.5 * if j + 1 < nz { zs[j + 1] - zs[j] } else { 0.0 } + 0.5 * if j > 0 { zs[j] - zs[j - 1] } else { 0.0 };
        let (mut s, mut n) = (0.0, 0.0);
        for jj in [j as isize - 1, j as isize] {
            if jj >= 0 && (jj as usize) < nz - 1 {
                s += ecell[jj as usize];
                n += 1.0;
            }
        }
        let e = s / n;
        for i in 0..nr {
            if i + 1 < nr {
                links.push((idx(i, j), idx(i + 1, j), EPS0 * e * 2.0 * PI * 0.5 * (rs[i] + rs[i + 1]) * dzh / (rs[i + 1] - rs[i]) * 1e-3));
            }
            if j + 1 < nz {
                links.push((idx(i, j), idx(i, j + 1), EPS0 * ecell[j] * PI * (rh(i).powi(2) - rl(i).powi(2)) / (zs[j + 1] - zs[j]) * 1e-3));
            }
        }
    }
    let mut fmap = vec![usize::MAX; nr * nz];
    let mut nf = 0;
    for k in 0..nr * nz {
        if cond[k] < 0 {
            fmap[k] = nf;
            nf += 1;
        }
    }
    let fix = |k: usize| if cond[k] == 1 { 1.0 } else { 0.0 };
    let mut rhs = vec![0.0; nf];
    let mut diag = vec![0.0; nf];
    let mut trip: Vec<(usize, usize, f64)> = Vec::with_capacity(links.len() * 2 + nf);
    for &(p, q, gg) in &links {
        for (u, w) in [(p, q), (q, p)] {
            if fmap[u] != usize::MAX {
                diag[fmap[u]] += gg;
                if fmap[w] != usize::MAX {
                    trip.push((fmap[u], fmap[w], -gg));
                } else {
                    rhs[fmap[u]] += gg * fix(w);
                }
            }
        }
    }
    for (k, d) in diag.iter().enumerate() {
        trip.push((k, k, *d));
    }
    let sol = kisi_sparse::solve_spd_triplets(nf, &trip, &rhs).unwrap_or_else(|_| vec![0.0; nf]);
    let phi = |k: usize| if fmap[k] != usize::MAX { sol[fmap[k]] } else { fix(k) };
    // charge on the 1-V conductors: the flux leaving them through their links
    let mut q = 0.0;
    for &(p, qq, gg) in &links {
        if cond[p] == 1 && cond[qq] != 1 {
            q += gg * (phi(p) - phi(qq));
        } else if cond[qq] == 1 && cond[p] != 1 {
            q += gg * (phi(qq) - phi(p));
        }
    }
    q
}

thread_local! {
    static POST_CAP: std::cell::RefCell<std::collections::HashMap<String, f64>> = Default::default();
}

/// Capacitance (F) between a return post and a plane it passes through an opening without joining
/// it (return groups): post of radius `a` from `z0` to `z1` (mm), the planes within its range
/// top-down as (z, opening radius; None where it joins the plane), `node` the plane in question. Everything but
/// that plane is at the post's potential (the planes it joins are; the others are its neighbours'
/// own nodes); the charge with the post minus the charge without it, so that the planes' own
/// capacitance cancels. Cached by geometry (to 1 µm).
pub fn post_plane_cap(slabs: &[Slab], a: f64, z0: f64, z1: f64, planes: &[(f64, Option<f64>)], node: usize) -> f64 {
    let q = |x: f64| (x * 1e3).round() as i64;
    let mut key = format!("{},{},{},{node}|", q(a), q(z0), q(z1));
    for p in planes {
        key += &format!("{},{};", q(p.0), p.1.map_or(-1, q));
    }
    for s in slabs {
        key += &format!("s{},{},{};", q(s.z0), q(s.z1), q(s.er));
    }
    if let Some(c) = POST_CAP.with(|c| c.borrow().get(&key).copied()) {
        return c;
    }
    // only the node's plane and its neighbours: the planes at the post's potential shield the rest,
    // and the post ends at them
    let (lo, hi) = (node.saturating_sub(1), (node + 1).min(planes.len() - 1));
    let pl: Vec<(f64, Option<f64>, bool)> = (lo..=hi).map(|k| (planes[k].0, if k == node { Some(planes[k].1.unwrap_or(a + 0.2)) } else { planes[k].1 }, k != node)).collect();
    // planes top-down: the neighbour above is node − 1, below node + 1
    let zt = if node > 0 { z1.min(planes[node - 1].0) } else { z1 };
    let zb = if node + 1 < planes.len() { z0.max(planes[node + 1].0) } else { z0 };
    let rmax = 1.2f64.max(4.0 * planes[node].1.unwrap_or(a + 0.2));
    let with = rz_total(slabs, Some((a, zb, zt)), &pl, rmax);
    let without = rz_total(slabs, None, &pl, rmax);
    let c = (with - without).max(0.0);
    POST_CAP.with(|m| {
        let mut m = m.borrow_mut();
        if m.len() > 4096 {
            m.clear();
        }
        m.insert(key, c);
    });
    c
}

/// Radius of the round antipad with the same in-plane capacitance as a non-round opening.
///
/// 2D Laplace between the barrel (circle of radius `a`, mm) and the opening, given as its
/// distance from the via centre `rb[j]` (mm) at `rb.len()` equally spaced angles (star-shaped
/// around the via). Solved in log-polar coordinates u = ln ρ, θ, where the Laplace equation
/// keeps its form; the irregular outer boundary uses Shortley–Weller differences. For a round
/// concentric opening the result is its radius; for an eccentric one it matches the eccentric
/// coax, b_eq = a·exp(acosh((a² + R² − d²)/(2aR))).
pub fn equivalent_radius(a: f64, rb: &[f64]) -> f64 {
    let nt = rb.len();
    let (rmin, rmax) = rb.iter().fold((f64::MAX, f64::MIN), |(lo, hi), &r| (lo.min(r), hi.max(r)));
    if nt < 8 || rmin <= a * 1.0001 {
        return rmin.max(a * 1.05);
    }
    if rmax / rmin < 1.002 {
        return rb.iter().map(|r| r.ln()).sum::<f64>().exp().powf(1.0 / nt as f64);
    }
    let h = 2.0 * std::f64::consts::PI / nt as f64;
    let u0 = a.ln();
    let ub: Vec<f64> = rb.iter().map(|r| r.ln()).collect();
    let nu = ((rmax.ln() - u0) / h).ceil() as usize + 1;
    let inside = |i: usize, j: usize| (u0 + i as f64 * h) < ub[j] - 1e-12;
    let mut phi = vec![0.0f64; (nu + 1) * nt];
    for j in 0..nt {
        phi[j] = 1.0;
    }
    // neighbour weights: (coefficient towards each neighbour, the neighbour index or None for the
    // outer boundary where φ = 0), Shortley–Weller in both directions
    let mut nodes: Vec<(usize, [(f64, Option<usize>); 4], f64)> = vec![];
    for i in 1..=nu {
        for j in 0..nt {
            if !inside(i, j) {
                continue;
            }
            let ui = u0 + i as f64 * h;
            // u direction
            let su = if inside(i + 1, j) { 1.0 } else { ((ub[j] - ui) / h).clamp(1e-3, 1.0) };
            let up = if inside(i + 1, j) { Some((i + 1) * nt + j) } else { None };
            let dn = Some((i - 1) * nt + j);
            // θ directions: the boundary between θ_j and θ_j±1 where ub crosses u_i (linear)
            let side = |jn: usize| -> (f64, Option<usize>) {
                if inside(i, jn) {
                    (1.0, Some(i * nt + jn))
                } else {
                    let t = ((ub[j] - ui) / (ub[j] - ub[jn]).max(1e-12)).clamp(1e-3, 1.0);
                    (t, None)
                }
            };
            let (sl, l) = side((j + nt - 1) % nt);
            let (sr, r) = side((j + 1) % nt);
            // second differences with unequal arms: 2/(s1 s2 (s1+s2)) weights
            let wu = [2.0 / (su * (1.0 + su)), 2.0 / (1.0 + su)];
            let wt = [2.0 / (sl * (sl + sr)), 2.0 / (sr * (sl + sr))];
            let diag = 2.0 / su + 2.0 / (sl * sr);
            nodes.push((i * nt + j, [(wu[0], up), (wu[1], dn), (wt[0], l), (wt[1], r)], diag));
        }
    }
    // direct sparse solve (Shortley–Weller rows are not symmetric: LU)
    let mut id = vec![usize::MAX; phi.len()];
    for (q, (k, _, _)) in nodes.iter().enumerate() {
        id[*k] = q;
    }
    let n = nodes.len();
    let (mut rp, mut ci, mut vals) = (vec![0i32], vec![], vec![]);
    let mut rhs = vec![0.0; n];
    for (q, (_, nb, diag)) in nodes.iter().enumerate() {
        ci.push(q as i32);
        vals.push(*diag);
        for (w, nq) in nb {
            match nq {
                Some(nq) if id[*nq] != usize::MAX => {
                    ci.push(id[*nq] as i32);
                    vals.push(-w);
                }
                Some(nq) => rhs[q] += w * phi[*nq], // the barrel (φ = 1)
                None => {}                          // the outer boundary (φ = 0)
            }
        }
        rp.push(ci.len() as i32);
    }
    match kisi_sparse::solve_csr_multi(n, &rp, &ci, &vals, 1, &rhs, true) {
        Ok(x) => {
            for (q, (k, _, _)) in nodes.iter().enumerate() {
                phi[*k] = x[q];
            }
        }
        Err(_) => return rmin.max(a * 1.05),
    }
    // flux through the barrel: Σ −∂φ/∂u Δθ (second order where three nodes exist)
    let mut flux = 0.0;
    for j in 0..nt {
        let g = if inside(2, j) {
            (3.0 * phi[j] - 4.0 * phi[nt + j] + phi[2 * nt + j]) / (2.0 * h)
        } else if inside(1, j) {
            (phi[j] - phi[nt + j]) / h
        } else {
            1.0 / (ub[j] - u0)
        };
        flux += g * h;
    }
    a * (2.0 * std::f64::consts::PI / flux).exp()
}

thread_local! {
    static EQ_CACHE: std::cell::RefCell<std::collections::HashMap<Vec<i32>, f64>> = Default::default();
}

/// [`equivalent_radius`] with a per-thread cache (profiles to 10 µm) and a short cut for nearly
/// round openings (max/min < 1.03: the log mean of the profile, as polygonised circles are).
pub fn equivalent_radius_cached(a: f64, rb: &[f64]) -> f64 {
    let (lo, hi) = rb.iter().fold((f64::MAX, f64::MIN), |(l, h), &r| (l.min(r), h.max(r)));
    if hi / lo < 1.03 {
        return (rb.iter().map(|r| r.ln()).sum::<f64>() / rb.len() as f64).exp();
    }
    // 48 angles are enough for the solve (eccentric coax within 1.5 %)
    let step = (rb.len() / 48).max(1);
    let rb: Vec<f64> = rb.iter().step_by(step).copied().collect();
    let rb = &rb[..];
    let mut key: Vec<i32> = rb.iter().map(|r| (r / 0.01).round() as i32).collect();
    key.push((a / 0.001).round() as i32);
    if let Some(b) = EQ_CACHE.with(|c| c.borrow().get(&key).copied()) {
        return b;
    }
    let b = equivalent_radius(a, rb);
    EQ_CACHE.with(|c| {
        let mut c = c.borrow_mut();
        if c.len() > 4096 {
            c.clear();
        }
        c.insert(key, b);
    });
    b
}

/// Distance from `p` to the edge of an opening along `nt` rays, the opening being the union of
/// `inside(x, y)` regions (mm), up to `rmax`: marched in `step` and refined by bisection.
pub fn ray_profile(p: [f64; 2], nt: usize, rmax: f64, step: f64, inside: &dyn Fn(f64, f64) -> bool) -> Vec<f64> {
    (0..nt)
        .map(|j| {
            let th = 2.0 * std::f64::consts::PI * j as f64 / nt as f64;
            let (c, s) = (th.cos(), th.sin());
            let mut t = 0.0;
            while t < rmax && inside(p[0] + t * c, p[1] + t * s) {
                t += step;
            }
            if t >= rmax {
                return rmax;
            }
            let (mut lo, mut hi) = ((t - step).max(0.0), t);
            for _ in 0..30 {
                let m = 0.5 * (lo + hi);
                if inside(p[0] + m * c, p[1] + m * s) {
                    lo = m;
                } else {
                    hi = m;
                }
            }
            0.5 * (lo + hi)
        })
        .collect()
}

/// zero-thickness Hammerstad–Jensen microstrip impedance (as the case generator)
fn z_ms(w: f64, h: f64, er: f64) -> f64 {
    let u = w / h;
    let a = 1.0 + ((u.powi(4) + (u / 52.0).powi(2)) / (u.powi(4) + 0.432)).ln() / 49.0 + (1.0 + (u / 18.1).powi(3)).ln() / 18.7;
    let b = 0.564 * ((er - 0.9) / (er + 3.0)).powf(0.053);
    let ee = (er + 1.0) / 2.0 + (er - 1.0) / 2.0 * (1.0 + 10.0 / u).powf(-a * b);
    let f = 6.0 + (2.0 * PI - 6.0) * (-(30.666 / u).powf(0.7528)).exp();
    60.0 / ee.sqrt() * (f / u + (1.0 + (2.0 / u).powi(2)).sqrt()).ln()
}

fn tline(z: f64, beta_l: f64) -> Abcd {
    let (s, c) = beta_l.sin_cos();
    Abcd { a: Cx::new(c, 0.0), b: Cx::new(0.0, z * s), c: Cx::new(0.0, s / z), d: Cx::new(c, 0.0) }
}

impl PbmVia {
    pub fn build(g: &ViaGeom, mode: Mode, grid: Grid) -> PbmVia {
        Self::build_with(g, mode, grid, vec![])
    }

    /// As [`build`](Self::build), with finite cavities for some plane pairs (`CavityLink`).
    pub fn build_with(g: &ViaGeom, mode: Mode, grid: Grid, envs: Vec<CavityLink>) -> PbmVia {
        let mode = if g.partner.is_none() { Mode::Single } else { mode };
        let (z_in, z_out) = (g.feeds[0].z, g.feeds[1].z);
        let crossed: Vec<&Plane> = g.planes.iter().filter(|p| g.z0 - 1e-9 <= p.z && p.z <= g.z1 + 1e-9).collect();
        let mut nodes: Vec<f64> = crossed.iter().filter(|p| p.antipad.is_some()).map(|p| round6(p.z)).collect();
        nodes.push(round6(z_in));
        nodes.push(round6(z_out));
        let mut nodes = sorted_unique(nodes, 1e-9);
        nodes.reverse();
        let plane_z: Vec<f64> = g.planes.iter().map(|p| p.z).collect();
        let mut extra = vec![];
        for w in nodes.windows(2) {
            let (za, zb) = (w[0], w[1]);
            if plane_z.iter().any(|&pz| pz >= za - 1e-9) && plane_z.iter().any(|&pz| pz <= zb + 1e-9) {
                let n = ((za - zb) / SUB - 1e-9).ceil() as usize;
                for k in 1..n {
                    extra.push(round6(za - (za - zb) * k as f64 / n as f64));
                }
            }
        }
        nodes.extend(extra);
        let mut nodes = sorted_unique(nodes, 1e-9);
        nodes.reverse();
        let graded_grid = matches!(grid, Grid::Graded { .. });
        let ch = rz_charges(g, grid);
        let (zq, qq, laplace_n) = (&ch.0, &ch.1, ch.2);
        let mut cap = vec![0.0; nodes.len()];
        // node regions in z: between the midpoints to the neighbouring nodes (nodes descend)
        let bound = |k: usize| -> (f64, f64) {
            let hi = if k == 0 { f64::INFINITY } else { 0.5 * (nodes[k - 1] + nodes[k]) };
            let lo = if k + 1 == nodes.len() { f64::NEG_INFINITY } else { 0.5 * (nodes[k] + nodes[k + 1]) };
            (lo, hi)
        };
        for (i, (&zz, &q)) in zq.iter().zip(qq).enumerate() {
            let mut best = 0;
            for k in 1..nodes.len() {
                if (zz - nodes[k]).abs() < (zz - nodes[best]).abs() {
                    best = k;
                }
            }
            // graded grid: a grid row whose cell straddles a node boundary is split by overlap
            // (the uniform grid keeps the prototype's nearest-node rule)
            let (c0, c1) = ch.3[i];
            if graded_grid && c1 - c0 > 1e-12 {
                for k in best.saturating_sub(1)..(best + 2).min(nodes.len()) {
                    let (lo, hi) = bound(k);
                    let o = (c1.min(hi) - c0.max(lo)).max(0.0);
                    cap[k] += q * o / (c1 - c0);
                }
            } else {
                cap[best] += q;
            }
        }
        if let (Some(pt), Mode::Odd) = (&g.partner, mode) {
            let pitch = pt.dx.hypot(pt.dy);
            // barrel charge per row with the row's cell height (mm)
            let mut rows: Vec<(f64, f64, f64)> = vec![];
            for (i, (&zz, &q)) in zq.iter().zip(qq).enumerate() {
                let key = (zz * 1e5).round() / 1e5;
                match rows.iter_mut().find(|(z, _, _)| (*z - key).abs() < 1e-9) {
                    Some(r) => r.1 += q,
                    None => rows.push((key, q, ch.3[i].1 - ch.3[i].0)),
                }
            }
            for k in 0..nodes.len() {
                let zk = nodes[k];
                let inside = crossed.iter().any(|p| p.z > zk + 1e-9) && crossed.iter().any(|p| p.z < zk - 1e-9);
                if !inside {
                    continue;
                }
                // charge per length of the barrel around this node: rows within SUB, away from the
                // planes. The prototype divided by the spread of the rows, which overstates the length
                // when the rows fall into two groups on both sides of a plane node (thin dielectrics:
                // capacitances ×10 on the ThunderScope stack); here the rows' own cell heights are summed.
                let near: Vec<&(f64, f64, f64)> = rows
                    .iter()
                    .filter(|(z, _, _)| (z - zk).abs() < SUB && g.planes.iter().all(|p| (z - p.z).abs() > 0.05))
                    .collect();
                let len: f64 = near.iter().map(|r| r.2).sum::<f64>() * 1e-3;
                if near.len() < 3 || len <= 0.0 {
                    continue;
                }
                let cpl = near.iter().map(|r| r.1).sum::<f64>() / len;
                let er = er_at(&g.slabs, zk).0;
                let reff = g.r * (2.0 * PI * EPS0 * er / cpl).exp();
                if pitch < reff {
                    cap[k] *= (reff / g.r).ln() / (pitch / g.r).ln();
                }
            }
        }
        let n = nodes.len();
        let node_tand: Vec<f64> = (0..n).map(|k| if k + 1 < n { er_at(&g.slabs, nodes[k] - 1e-4).1 } else { er_at(&g.slabs, nodes[k] + 1e-4).1 }).collect();
        let use_partner = mode != Mode::Single && g.partner.is_some();
        let pt = |p: &Post| (p.dx * 1e-3, p.dy * 1e-3, p.r * 1e-3);
        let cpt = |c: &CapPost| (c.dx * 1e-3, c.dy * 1e-3, c.r * 1e-3);
        let joins = |c: &CapPost, pa: f64, pb: f64| (c.z_top - pa).abs() < 1e-6 && (c.z_bot - pb).abs() < 1e-6;
        // the environment of a finite cavity for the pair (pa, pb) if every point is one of its ports
        let env_for = |pa: f64, pb: f64, pts: &[(f64, f64, f64)]| {
            envs.iter().position(|e| (e.z_top - pa).abs() < 1e-6 && (e.z_bot - pb).abs() < 1e-6).and_then(|ei| {
                let e = &envs[ei];
                let ports = e.env.ports();
                let idx: Option<Vec<usize>> = pts
                    .iter()
                    .map(|&(x, y, _)| ports.iter().position(|q| (q.x - (e.origin.0 + x)).abs() < 1e-7 && (q.y - (e.origin.1 + y)).abs() < 1e-7))
                    .collect();
                idx.map(|i| (ei, i))
            })
        };
        let dist = |pts: &[(f64, f64, f64)]| -> Vec<f64> {
            let n = pts.len();
            (0..n * n).map(|q| if q / n == q % n { pts[q / n].2 } else { (pts[q / n].0 - pts[q % n].0).hypot(pts[q / n].1 - pts[q % n].1) }).collect()
        };
        // return groups: every return post that does not short all pairs it spans is a ladder of its
        // own – a node at each plane it passes without joining it (with its capacitance to that
        // plane), a span in each pair whose two planes it does not both join – and the pairs with
        // such spans get the matrices that couple them to the via
        let mut rnodes: Vec<(usize, f64, f64)> = vec![];
        let mut rspans: Vec<(usize, Option<usize>, Option<usize>)> = vec![];
        let mut pairs: Vec<PairM> = vec![];
        {
            let mut pl = sorted_unique(plane_z.iter().map(|&z| round6(z)).collect(), 1e-9);
            pl.reverse();
            // the spans of the returns in every pair (index of the pair's upper plane in `pl`)
            let mut active: Vec<Vec<(usize, usize)>> = vec![vec![]; pl.len().saturating_sub(1)];
            for (i, p) in g.returns.iter().enumerate() {
                let Some(js) = &p.joins else { continue };
                let has = |z: f64| js.iter().any(|&j| (j - z).abs() < 1e-6);
                let ks: Vec<usize> = (0..pl.len()).filter(|&k| p.z0 <= pl[k] + 1e-9 && pl[k] <= p.z1 + 1e-9).collect();
                if !ks.iter().any(|&k| has(pl[k])) {
                    continue;
                }
                let planes_r: Vec<(f64, Option<f64>)> = ks
                    .iter()
                    .map(|&k| (pl[k], if has(pl[k]) { None } else { Some(p.open_r.iter().find(|o| (o.0 - pl[k]).abs() < 1e-6).map_or(p.r + 0.2, |o| o.1)) }))
                    .collect();
                let node: Vec<Option<usize>> = ks
                    .iter()
                    .enumerate()
                    .map(|(q, &k)| {
                        if has(pl[k]) {
                            None
                        } else {
                            rnodes.push((i, pl[k], post_plane_cap(&g.slabs, p.r, p.z0, p.z1, &planes_r, q)));
                            Some(rnodes.len() - 1)
                        }
                    })
                    .collect();
                for q in 0..ks.len().saturating_sub(1) {
                    if ks[q + 1] != ks[q] + 1 || (node[q].is_none() && node[q + 1].is_none()) {
                        continue;
                    }
                    rspans.push((i, node[q], node[q + 1]));
                    active[ks[q]].push((i, rspans.len() - 1));
                }
            }
            for (q, act) in active.iter().enumerate() {
                if act.is_empty() {
                    continue;
                }
                let (pa, pb) = (pl[q], pl[q + 1]);
                let crossed_pair = g.z0 <= pb + 1e-9 && g.z1 >= pa - 1e-9;
                let mut pts = vec![];
                if crossed_pair {
                    pts.push((0.0, 0.0, g.r * 1e-3));
                    if use_partner {
                        pts.push(pt(g.partner.as_ref().unwrap()));
                    }
                }
                let nsig = pts.len();
                pts.extend(g.returns.iter().filter(|p| p.joins.is_some() && p.role(pa, pb) == Role::Short).map(pt));
                let nshort = pts.len() - nsig;
                pts.extend(act.iter().map(|a| pt(&g.returns[a.0])));
                let env = env_for(pa, pb, &pts);
                let loads: Vec<CapPost> = if env.is_some() { vec![] } else { g.caps.iter().copied().filter(|c| joins(c, pa, pb)).collect() };
                // capacitors before the returns with spans
                let at = nsig + nshort;
                for (k, c) in loads.iter().enumerate() {
                    pts.insert(at + k, cpt(c));
                }
                let mid = 0.5 * (pa + pb);
                let er = er_weighted(&g.slabs, pa, pb).unwrap_or(er_at(&g.slabs, mid).0);
                let rr = dist(&pts);
                pairs.push(PairM { pair: (pa * 1e-3, pb * 1e-3), er, tand: er_at(&g.slabs, mid).1, pts, nsig, nshort, loads, segs: act.iter().map(|a| a.1).collect(), rr, env });
            }
        }
        // spans
        let mut groups: Vec<Group> = vec![];
        let mut spans = vec![];
        let mut span_pair = vec![];
        for k in 0..n.saturating_sub(1) {
            let (za, zb) = (nodes[k], nodes[k + 1]);
            let above: Vec<f64> = plane_z.iter().copied().filter(|&pz| pz >= za - 1e-9).collect();
            let below: Vec<f64> = plane_z.iter().copied().filter(|&pz| pz <= zb + 1e-9).collect();
            if above.is_empty() || below.is_empty() {
                let pl = if !below.is_empty() { below.iter().copied().fold(f64::MAX, f64::min) } else { above.iter().copied().fold(f64::MIN, f64::max) };
                let b = g.planes.iter().find(|p| (p.z - pl).abs() < 1e-9 && p.antipad.map_or(false, |b| b > 0.0)).and_then(|p| p.antipad).unwrap_or(2.0 * g.r);
                spans.push(SpanKind::Outer(MU0 * (za - zb) * 1e-3 / (2.0 * PI) * (b / g.r).max(1.01).ln()));
                span_pair.push(None);
                continue;
            }
            let mid = 0.5 * (za + zb);
            let (er0, tand) = er_at(&g.slabs, mid);
            let er = er_weighted(&g.slabs, za, zb).unwrap_or(er0);
            let pa = above.iter().copied().fold(f64::MAX, f64::min);
            let pb = below.iter().copied().fold(f64::MIN, f64::max);
            let mut pts = vec![(0.0, 0.0, g.r * 1e-3)];
            if use_partner {
                pts.push(pt(g.partner.as_ref().unwrap()));
            }
            for p in &g.returns {
                let short = match p.joins {
                    // the ladder before return groups: every return that spans this span
                    None => p.z0 <= zb + 1e-9 && p.z1 >= za - 1e-9,
                    Some(_) => p.role(round6(pa), round6(pb)) == Role::Short,
                };
                if short {
                    pts.push(pt(p));
                }
            }
            span_pair.push(pairs.iter().position(|q| (q.pair.0 - round6(pa) * 1e-3).abs() < 1e-12 && (q.pair.1 - round6(pb) * 1e-3).abs() < 1e-12));
            let mut scale = (za - zb) * 1e-3;
            if let Some(&zj) = [z_in, z_out].iter().find(|&&zj| pb + 1e-9 < zj && zj < pa - 1e-9) {
                let dg = if za <= pa + 1e-9 && zb >= zj - 1e-9 { pa - zj } else { zj - pb };
                scale *= dg / (pa - pb);
            }
            let d_pair = (pa - pb) * 1e-3;
            // a finite cavity for this plane pair: every point must be a port of its environment
            let env = env_for(pa, pb, &pts);
            // capacitors between the two planes (on infinite plates; a cavity has its own)
            let loads: Vec<CapPost> = if env.is_some() { vec![] } else { g.caps.iter().copied().filter(|c| joins(c, round6(pa), round6(pb))).collect() };
            pts.extend(loads.iter().map(cpt));
            let gi = match groups.iter().position(|q| q.er == er && q.tand == tand && q.pts == pts && q.partner == use_partner && q.d == d_pair && q.env == env && q.loads == loads) {
                Some(i) => i,
                None => {
                    let rr = dist(&pts);
                    groups.push(Group { er, tand, pts, partner: use_partner, d: d_pair, rr, loads, env, pair: (pa * 1e-3, pb * 1e-3) });
                    groups.len() - 1
                }
            };
            spans.push(SpanKind::Pair { group: gi, scale });
        }
        let idx = |z: f64| nodes.iter().position(|&x| (x - round6(z)).abs() < 1e-9).unwrap();
        let (i_in, i_out) = (idx(z_in), idx(z_out));
        // where the traces join: the trace over the antipad, the removal of the feed that the
        // fixture (or the channel) runs to the via centre, and the junction inductance
        let mut ap_line = [None, None];
        let mut feed_cut = [(50.0, 1.0, 0.0); 2];
        let mut l_port = [0.0; 2];
        for side in 0..2 {
            let fd = g.feeds[side];
            let (zt, ref_z) = (fd.z, fd.z_ref);
            let h_ref = (zt - ref_z).abs();
            let hb = g.planes.iter().find(|p| (p.z - ref_z).abs() < 1e-6 && p.antipad.map_or(false, |b| b > 0.0)).and_then(|p| p.antipad).unwrap_or(0.0);
            let strip = plane_z.iter().any(|&pz| pz > zt + 1e-9) && plane_z.iter().any(|&pz| pz < zt - 1e-9);
            if g.junction {
                // fitted to the 3D references (TESTS.md section 7): where the trace current turns
                // into the pad and the barrel, μ0/(2π)·pad radius (≈ 50–60 pH)
                l_port[side] = MU0 / (2.0 * PI) * g.pad_r * 1e-3;
            }
            if g.junction && strip {
                // a stripline keeps its line up to the via centre: between two planes the
                // openings change it little (3D: removing it misses ~3 ps on L1→L3)
                feed_cut[side] = (fd.zc, fd.eeff, fd.dropped * 1e-3);
                continue;
            }
            if fd.antipad_line && hb > g.pad_r {
                let ln = (hb - g.pad_r) * 1e-3;
                ap_line[side] = Some(if g.junction {
                    // its return runs around the edge of the opening: the feed scaled to the height
                    // plus the mean lateral way to the edge
                    let he = h_ref + (hb - g.pad_r);
                    (fd.zc * z_ms(fd.w, he, 4.0) / z_ms(fd.w, h_ref, 4.0), fd.eeff, ln)
                } else {
                    // prototype: a microstrip over the next plane beyond the reference
                    let nxt: Vec<f64> = plane_z.iter().copied().filter(|&pz| if ref_z < zt { pz < ref_z - 1e-6 } else { pz > ref_z + 1e-6 }).collect();
                    let h2 = if nxt.is_empty() {
                        h_ref * 3.0
                    } else if ref_z < zt {
                        (zt - nxt.iter().copied().fold(f64::MIN, f64::max)).abs()
                    } else {
                        (zt - nxt.iter().copied().fold(f64::MAX, f64::min)).abs()
                    };
                    let er = er_at(&g.slabs, 0.5 * (zt + ref_z)).0;
                    (z_ms(fd.w, h2, er), er, ln)
                });
            }
            feed_cut[side] = (fd.zc, fd.eeff, -(g.pad_r.max(hb) - fd.dropped) * 1e-3);
        }
        let span_len: Vec<f64> = nodes.windows(2).map(|w| (w[0] - w[1]) * 1e-3).collect();
        PbmVia {
            nodes,
            cap,
            node_tand,
            spans,
            groups,
            i_in,
            i_out,
            mode,
            f_loss: g.f_loss,
            ap_line,
            feed_cut,
            l_port,
            sigma: g.sigma,
            a_m: g.r * 1e-3,
            span_len,
            laplace_n,
            zgrid: Default::default(),
            envs,
            local: Default::default(),
            rnodes,
            rspans,
            pairs,
            span_pair,
            wgrid: Default::default(),
        }
    }

    fn tan_eff(&self, tand: f64, f: f64) -> f64 {
        match self.f_loss {
            Some(fl) => tand * fl / f,
            None => tand,
        }
    }

    /// How much each plane pair still couples to the via with its returns in place: the real part
    /// (the power the via cluster launches into the radial line) of the reduced impedance against
    /// that of a lone via, Re Z_red / (ωμd/4 · J0(ka)), on infinite plates at f. A cluster of
    /// returns close around the via gives a small ratio: what lies further out (edges, other
    /// posts) can hardly change the via. (z_top, z_bot) of the pair in mm and the ratio.
    pub fn pair_leakage(&self, f: f64) -> Vec<(f64, f64, f64)> {
        let z = self.group_z_exact(f);
        let w = 2.0 * PI * f;
        self.groups
            .iter()
            .zip(&z)
            .map(|(g, zr)| {
                let k = Cx::new(1.0, -self.tan_eff(g.tand, f)).sqrt().scale(w * (MU0 * EPS0 * g.er).sqrt());
                let lone = (hankel2_0(k.scale(g.pts[0].2)).scale(w * MU0 / 4.0)).re;
                (g.pair.0 * 1e3, g.pair.1 * 1e3, (zr.re / lone).abs())
            })
            .collect()
    }

    /// whether some plane pair is a finite cavity
    pub fn has_cavity(&self) -> bool {
        self.groups.iter().any(|g| g.env.is_some())
    }

    /// impedance of every plane-pair group per metre of height: the infinite radial line
    /// interpolated in Z/f (`group_z_hankel`), the finite cavities exact at f
    fn group_z(&self, f: f64) -> Vec<Cx> {
        let mut z = self.group_z_hankel(f);
        if self.local.get() {
            return z;
        }
        for (gi, g) in self.groups.iter().enumerate() {
            if let Some((ei, idx)) = &g.env {
                let e = &self.envs[*ei];
                let gm = e.env.green(f);
                let ne = e.env.ports().len();
                let n = idx.len();
                // per metre of height: Z = jωμd·G
                let jwm = Cx::new(0.0, 2.0 * PI * f * MU0);
                let zz: Vec<Cx> = (0..n * n).map(|q| jwm * gm[idx[q / n] * ne + idx[q % n]]).collect();
                z[gi] = self.reduce(zz, n, g.partner);
            }
        }
        z
    }

    /// returns of a plane-pair matrix (per metre) reduced as shorts; single, odd or even mode
    fn reduce(&self, z: Vec<Cx>, n: usize, partner: bool) -> Cx {
        self.reduce_loaded(z, n, partner, &[])
    }

    /// as [`reduce`](Self::reduce), the last `zload.len()` points closed by these impedances
    /// (per metre of the pair's height: Z/d) instead of shorts
    fn reduce_loaded(&self, z: Vec<Cx>, n: usize, partner: bool, zload: &[Cx]) -> Cx {
        let kk = if partner { 2 } else { 1 };
        let mut zr: Vec<Cx> = (0..kk * kk).map(|q| z[(q / kk) * n + q % kk]).collect();
        if n > kk {
            let m = n - kk;
            let mut bb: Vec<Cx> = (0..m * m).map(|q| z[(kk + q / m) * n + kk + q % m]).collect();
            for (i, zl) in zload.iter().enumerate() {
                let q = m - zload.len() + i;
                bb[q * m + q] = bb[q * m + q] + *zl;
            }
            let ba: Vec<Cx> = (0..m * kk).map(|q| z[(kk + q / kk) * n + q % kk]).collect();
            let x = csolve(bb, ba, m, kk);
            for a in 0..kk {
                for c in 0..kk {
                    let mut s = Cx::new(0.0, 0.0);
                    for q in 0..m {
                        s = s + z[a * n + kk + q] * x[q * kk + c];
                    }
                    zr[a * kk + c] = zr[a * kk + c] - s;
                }
            }
        }
        match (partner, self.mode) {
            (true, Mode::Odd) => zr[0] - zr[1],
            (true, _) => zr[0] + zr[1],
            _ => zr[0],
        }
    }

    /// infinite radial line of every group per metre: linear interpolation of Z/f between grid
    /// points (exact below the first grid point)
    fn group_z_hankel(&self, f: f64) -> Vec<Cx> {
        if self.groups.is_empty() {
            return vec![];
        }
        let x = f / DF_GRID;
        if x < 1.0 {
            return self.group_z_exact(f);
        }
        let k = x.floor() as usize;
        let t = x - k as f64;
        let at = |k: usize| -> Vec<Cx> {
            let mut g = self.zgrid.borrow_mut();
            if g.len() <= k {
                g.resize(k + 1, None);
            }
            if g[k].is_none() {
                let fk = k as f64 * DF_GRID;
                g[k] = Some(self.group_z_exact(fk).into_iter().map(|z| z.scale(1.0 / fk)).collect());
            }
            g[k].clone().unwrap()
        };
        let a = at(k);
        if t < 1e-9 {
            return a.into_iter().map(|z| z.scale(f)).collect();
        }
        let b = at(k + 1);
        a.iter().zip(&b).map(|(p, q)| (p.scale(1.0 - t) + q.scale(t)).scale(f)).collect()
    }

    /// impedance of every plane-pair group per metre of height, evaluated at `f`
    fn group_z_exact(&self, f: f64) -> Vec<Cx> {
        let w = 2.0 * PI * f;
        self.groups
            .iter()
            .map(|g| {
                let mut k = Cx::new(1.0, -self.tan_eff(g.tand, f)).sqrt().scale(w * (MU0 * EPS0 * g.er).sqrt());
                if let Some(sig) = self.sigma {
                    // plates with surface impedance (1+j)Rs: k² (1 + 2Rs/(ωμd) − j 2Rs/(ωμd))
                    let x = 2.0 * (PI * f * MU0 / sig).sqrt() / (w * MU0 * g.d);
                    k = k * Cx::new(1.0 + x, -x).sqrt();
                }
                let pre = w * MU0 / 4.0;
                let n = g.pts.len();
                let mut z = vec![Cx::new(0.0, 0.0); n * n];
                for i in 0..n {
                    for j in i..n {
                        let h = hankel2_0(k.scale(g.rr[i * n + j])).scale(pre);
                        z[i * n + j] = h;
                        z[j * n + i] = h;
                    }
                }
                // reduce the shorted returns: Zr = Z_aa − Z_ab Z_bb⁻¹ Z_ba (capacitors: closed by
                // their impedance)
                if g.loads.is_empty() {
                    self.reduce(z, n, g.partner)
                } else {
                    let zl: Vec<Cx> = g.loads.iter().map(|c| c.z(w).scale(1.0 / g.d)).collect();
                    self.reduce_loaded(z, n, g.partner, &zl)
                }
            })
            .collect()
    }

    /// wavenumber of a plane pair at f (dielectric loss, plates of conductivity `sigma`)
    fn pair_k(&self, er: f64, tand: f64, d: f64, f: f64) -> Cx {
        let w = 2.0 * PI * f;
        let mut k = Cx::new(1.0, -self.tan_eff(tand, f)).sqrt().scale(w * (MU0 * EPS0 * er).sqrt());
        if let Some(sig) = self.sigma {
            let x = 2.0 * (PI * f * MU0 / sig).sqrt() / (w * MU0 * d);
            k = k * Cx::new(1.0 + x, -x).sqrt();
        }
        k
    }

    /// whether some plane pair has capacitors between its planes (return groups, infinite plates)
    pub fn has_loads(&self) -> bool {
        self.groups.iter().any(|g| !g.loads.is_empty()) || self.pairs.iter().any(|p| !p.loads.is_empty())
    }

    /// whether some plane pair has floating returns (return groups)
    pub fn has_floats(&self) -> bool {
        !self.pairs.is_empty()
    }

    /// plane pairs with return spans of their own (floating or capacitive returns), (z_top, z_bot)
    /// in mm, and the number of return spans
    pub fn floating_pairs(&self) -> (Vec<(f64, f64)>, usize) {
        (self.pairs.iter().map(|p| (p.pair.0 * 1e3, p.pair.1 * 1e3)).collect(), self.rspans.len())
    }

    /// the coupling terms of every pair with floating returns at f: infinite plates (or the pair's
    /// finite cavity unless `local`); the returns that short the pair and the capacitors reduced
    fn pair_z_exact(&self, f: f64, cavities: bool) -> PairZ {
        let w = 2.0 * PI * f;
        self.pairs
            .iter()
            .map(|p| {
                let d = p.pair.0 - p.pair.1;
                let n = p.pts.len();
                let nl = p.loads.len();
                // the full matrix over the points (per metre) and the impedances of the eliminated ones
                let (z, n_all, elim, add): (Vec<Cx>, usize, Vec<usize>, Vec<Cx>) = match (&p.env, cavities) {
                    (Some((ei, idx)), true) => {
                        let e = &self.envs[*ei];
                        let gm = e.env.green(f);
                        let ne = e.env.ports().len();
                        let m = idx.len();
                        let jwm = Cx::new(0.0, w * MU0);
                        ((0..m * m).map(|q| jwm * gm[idx[q / m] * ne + idx[q % m]]).collect(), m, (p.nsig..p.nsig + p.nshort).collect(), vec![Cx::new(0.0, 0.0); p.nshort])
                    }
                    _ => {
                        let k = self.pair_k(p.er, p.tand, d, f);
                        let pre = w * MU0 / 4.0;
                        let mut z = vec![Cx::new(0.0, 0.0); n * n];
                        for i in 0..n {
                            for j in i..n {
                                let h = hankel2_0(k.scale(p.rr[i * n + j])).scale(pre);
                                z[i * n + j] = h;
                                z[j * n + i] = h;
                            }
                        }
                        let mut add = vec![Cx::new(0.0, 0.0); p.nshort];
                        add.extend(p.loads.iter().map(|c| c.z(w).scale(1.0 / d)));
                        (z, n, (p.nsig..p.nsig + p.nshort + nl).collect(), add)
                    }
                };
                // with a cavity the capacitors are not points
                let nf = p.segs.len();
                let keep: Vec<usize> = (0..p.nsig).chain(n_all - nf..n_all).collect();
                let kz = schur(&z, n_all, &keep, &elim, &add);
                let nk = keep.len();
                let ns = p.nsig;
                let kq = |i: usize, j: usize| kz[i * nk + j];
                // the via's row (with the partner: the voltage on the via with the partner carrying
                // ∓ its current, as the groups' odd and even mode Z_PP ∓ Z_PN), so that a segment in
                // one pair is the groups' short; the return's excitation by both barrels
                let (mut c, mut b) = (vec![], vec![]);
                for q in 0..nf {
                    let fq = ns + q;
                    match (ns, self.mode) {
                        (0, _) => {}
                        (1, _) | (_, Mode::Single) => {
                            c.push(kq(0, fq));
                            b.push(kq(fq, 0));
                        }
                        (_, Mode::Odd) => {
                            c.push(kq(0, fq));
                            b.push(kq(fq, 0) - kq(fq, 1));
                        }
                        (_, Mode::Even) => {
                            c.push(kq(0, fq));
                            b.push(kq(fq, 0) + kq(fq, 1));
                        }
                    }
                }
                let dm: Vec<Cx> = (0..nf * nf).map(|q| kq(ns + q / nf, ns + q % nf)).collect();
                (c, b, dm)
            })
            .collect()
    }

    /// The returns with spans of their own reduced onto the via (return groups): W (P × P, P the
    /// pairs with such returns that the via crosses), the voltage on a span k of the via in pair p
    /// being ℓ_k Σ_p' W_pp' M_p' with M_p' = Σ_{l in p'} ℓ_l I_l the via's current moment in pair p'.
    /// The returns' own system – a span per pair it does not short, u_top − u_bot − Σ_r' D_rr'·d·I_r'
    /// = b·M (the pair's voltage at the return, zero at a plane it joins), a node per plane it
    /// passes, I_above − I_below = jωC·u – solved for the moments: W = C·A⁻¹·B. Finite cavities with
    /// `cavities`.
    fn return_w_exact(&self, f: f64, cavities: bool) -> Vec<Cx> {
        let w = 2.0 * PI * f;
        let pz = self.pair_z_exact(f, cavities);
        let (q, m) = (self.rspans.len(), self.rnodes.len());
        let nt = q + m;
        let ps: Vec<usize> = (0..self.pairs.len()).filter(|&i| self.pairs[i].nsig > 0).collect();
        let np = ps.len();
        let (one, mone) = (Cx::new(1.0, 0.0), Cx::new(-1.0, 0.0));
        let mut a = vec![Cx::new(0.0, 0.0); nt * nt];
        let mut add = |r: usize, c: usize, v: Cx| a[r * nt + c] = a[r * nt + c] + v;
        for (sp, &(_, top, bot)) in self.rspans.iter().enumerate() {
            if let Some(t) = top {
                add(sp, q + t, one);
            }
            if let Some(t) = bot {
                add(sp, q + t, mone);
            }
        }
        for (pi, p) in self.pairs.iter().enumerate() {
            let d = p.pair.0 - p.pair.1;
            let nf = p.segs.len();
            for f1 in 0..nf {
                for f2 in 0..nf {
                    add(p.segs[f1], p.segs[f2], pz[pi].2[f1 * nf + f2].scale(-d));
                }
            }
        }
        for (t, &(_, _, c)) in self.rnodes.iter().enumerate() {
            add(q + t, q + t, Cx::new(0.0, -w * c));
            for (sp, &(_, top, bot)) in self.rspans.iter().enumerate() {
                if bot == Some(t) {
                    add(q + t, sp, one);
                }
                if top == Some(t) {
                    add(q + t, sp, mone);
                }
            }
        }
        // B: the via's moment in pair p drives the return spans in p by b
        let mut rhs = vec![Cx::new(0.0, 0.0); nt * np];
        for (j, &pi) in ps.iter().enumerate() {
            for (fq, &sp) in self.pairs[pi].segs.iter().enumerate() {
                rhs[sp * np + j] = pz[pi].1[fq];
            }
        }
        let x = csolve(a, rhs, nt, np);
        // C: the voltage on the via in pair p from the return spans in p
        let mut wm = vec![Cx::new(0.0, 0.0); np * np];
        for (i, &pi) in ps.iter().enumerate() {
            for (fq, &sp) in self.pairs[pi].segs.iter().enumerate() {
                for j in 0..np {
                    wm[i * np + j] = wm[i * np + j] + pz[pi].0[fq] * x[sp * np + j];
                }
            }
        }
        wm
    }

    /// [`return_w_exact`](Self::return_w_exact) on infinite plates interpolated linearly in W/f
    /// between the grid points k·`DF_GRID` (as the plane-pair impedances), with finite cavities
    /// exact at f
    fn return_w(&self, f: f64) -> Vec<Cx> {
        if !self.local.get() && self.pairs.iter().any(|p| p.env.is_some()) {
            return self.return_w_exact(f, true);
        }
        let x = f / DF_GRID;
        if x < 1.0 {
            return self.return_w_exact(f, false);
        }
        let k = x.floor() as usize;
        let t = x - k as f64;
        let at = |k: usize| -> Vec<Cx> {
            let mut g = self.wgrid.borrow_mut();
            if g.len() <= k {
                g.resize(k + 1, None);
            }
            if g[k].is_none() {
                let fk = k as f64 * DF_GRID;
                g[k] = Some(self.return_w_exact(fk, false).into_iter().map(|z| z.scale(1.0 / fk)).collect());
            }
            g[k].clone().unwrap()
        };
        let a = at(k);
        if t < 1e-9 {
            return a.into_iter().map(|z| z.scale(f)).collect();
        }
        let b = at(k + 1);
        a.iter().zip(&b).map(|(p, q)| (p.scale(1.0 - t) + q.scale(t)).scale(f)).collect()
    }

    /// ABCD of the via between the two feed traces (ports at the via centre).
    pub fn abcd(&self, f: f64) -> Abcd {
        let w = 2.0 * PI * f;
        let gz = self.group_z(f);
        // skin-effect resistance and internal inductance of the barrel per metre
        let zb = self.sigma.map(|sig| {
            let rs = (PI * f * MU0 / sig).sqrt();
            Cx::new(rs, rs).scale(1.0 / (2.0 * PI * self.a_m))
        });
        let zs = |k: usize| -> Cx {
            let z = match self.spans[k] {
                SpanKind::Outer(l) => Cx::new(0.0, w * l),
                SpanKind::Pair { group, scale } => gz[group].scale(scale),
            };
            match zb {
                Some(zb) => z + zb.scale(self.span_len[k]),
                None => z,
            }
        };
        let yc = |k: usize| -> Cx { Cx::new(0.0, w * self.cap[k]) * Cx::new(1.0, -self.tan_eff(self.node_tand[k], f)) };
        let mut m = if self.pairs.is_empty() { self.core_chain(&zs, &yc) } else { self.core_coupled(f, &zs, &yc) };
        for side in 0..2 {
            if self.l_port[side] != 0.0 {
                let t = Abcd::series(Cx::new(0.0, w * self.l_port[side]));
                m = if side == 0 { t.mul(&m) } else { m.mul(&t) };
            }
        }
        for side in 0..2 {
            if let Some((z, er, ln)) = self.ap_line[side] {
                let t = tline(z, w * er.sqrt() / C0 * ln);
                m = if side == 0 { t.mul(&m) } else { m.mul(&t) };
            }
        }
        for side in 0..2 {
            let (z, ee, ln) = self.feed_cut[side];
            if ln != 0.0 {
                let t = tline(z, w * ee.sqrt() / C0 * ln);
                m = if side == 0 { t.mul(&m) } else { m.mul(&t) };
            }
        }
        m
    }

    /// the barrel between the entry and the exit node with its open stubs: the ladder as a chain of
    /// shunt and series elements (`zs` of every span, `yc` of every node)
    fn core_chain(&self, zs: &dyn Fn(usize) -> Cx, yc: &dyn Fn(usize) -> Cx) -> Abcd {
        let n = self.nodes.len();
        // chain from entry to exit
        let (a, b) = (self.i_in, self.i_out);
        let mut m = Abcd::identity();
        if a != b {
            let step: isize = if b > a { 1 } else { -1 };
            let mut k = a as isize;
            while k != b as isize {
                let kn = k + step;
                m = m.mul(&Abcd::shunt(yc(k as usize))).mul(&Abcd::series(zs(k.min(kn) as usize)));
                k = kn;
            }
        }
        m = m.mul(&Abcd::shunt(yc(b)));
        // open stubs beyond the chain
        let stub_y = |from: usize, end: usize| -> Cx {
            if from == end {
                return Cx::new(0.0, 0.0);
            }
            let step: isize = if end > from { 1 } else { -1 };
            let mut y = yc(end);
            let mut k = end as isize;
            while k != from as isize {
                let kp = k - step;
                let z = zs(k.min(kp) as usize);
                y = if norm2(y) > 0.0 { (z + y.inv()).inv() } else { Cx::new(0.0, 0.0) };
                if kp != from as isize {
                    y = y + yc(kp as usize);
                }
                k = kp;
            }
            y
        };
        let (lo, hi) = (a.min(b), a.max(b));
        let y_top = if lo != 0 { stub_y(lo, 0) } else { Cx::new(0.0, 0.0) };
        let y_bot = if hi != n - 1 { stub_y(hi, n - 1) } else { Cx::new(0.0, 0.0) };
        if a < b {
            Abcd::shunt(y_top).mul(&m).mul(&Abcd::shunt(y_bot))
        } else {
            Abcd::shunt(y_bot).mul(&m).mul(&Abcd::shunt(y_top))
        }
    }

    /// The core of [`core_chain`](Self::core_chain) with return ladders (return groups): the via's
    /// node voltages and span currents solved together with its current moment M_p in every pair
    /// where returns have spans of their own; a span k in such a pair gets the voltage
    /// ℓ_k Σ_p' W_pp' M_p' of the reduced returns ([`return_w`](Self::return_w)). Without the returns'
    /// capacitances a return joined above and below is a floating return (one current, the pairs'
    /// voltages adding up to zero), which over pairs of equal εr that the via crosses with one
    /// current is the same as shorts in each; a return joined at both planes of one pair is a short
    /// (the reduction of the groups).
    fn core_coupled(&self, f: f64, zs: &dyn Fn(usize) -> Cx, yc: &dyn Fn(usize) -> Cx) -> Abcd {
        let n = self.nodes.len();
        let ns = n.saturating_sub(1);
        let ps: Vec<usize> = (0..self.pairs.len()).filter(|&i| self.pairs[i].nsig > 0).collect();
        let np = ps.len();
        let wm = self.return_w(f);
        let m0 = n + ns;
        let nt = m0 + np;
        let mut a = vec![Cx::new(0.0, 0.0); nt * nt];
        let (one, mone) = (Cx::new(1.0, 0.0), Cx::new(-1.0, 0.0));
        let mut add = |r: usize, c: usize, v: Cx| a[r * nt + c] = a[r * nt + c] + v;
        for k in 0..n {
            add(k, k, yc(k));
            if k > 0 {
                add(k, n + k - 1, mone);
            }
            if k + 1 < n {
                add(k, n + k, one);
            }
        }
        for k in 0..ns {
            let r = n + k;
            add(r, k, one);
            add(r, k + 1, mone);
            add(r, r, zs(k).scale(-1.0));
            if let Some(i) = self.span_pair[k].and_then(|pi| ps.iter().position(|&x| x == pi)) {
                let l = self.span_len[k];
                for j in 0..np {
                    add(r, m0 + j, wm[i * np + j].scale(-l));
                }
                // the moment of pair i
                add(m0 + i, r, Cx::new(-l, 0.0));
            }
        }
        for i in 0..np {
            add(m0 + i, m0 + i, one);
        }
        let (ia, ib) = (self.i_in, self.i_out);
        let mut rhs = vec![Cx::new(0.0, 0.0); nt * 2];
        rhs[ia * 2] = one;
        rhs[ib * 2 + 1] = rhs[ib * 2 + 1] + one;
        let x = csolve(a, rhs, nt, 2);
        let (z11, z21, z12, z22) = (x[ia * 2], x[ib * 2], x[ia * 2 + 1], x[ib * 2 + 1]);
        Abcd { a: z11 / z21, b: (z11 * z22 - z12 * z21) / z21, c: z21.inv(), d: z22 / z21 }
    }

    /// group delay of the transition (s) from the S21 phase slope at low frequency (the via
    /// itself: without the cavities)
    pub fn delay(&self) -> f64 {
        let was = self.local.replace(true);
        let t = self.delay_inner();
        self.local.set(was);
        t
    }

    fn delay_inner(&self) -> f64 {
        let (f1, f2) = (0.5e9, 1.5e9);
        let p = |f: f64| {
            let s = self.abcd(f).s_params(50.0).1;
            s.arg()
        };
        let mut dp = p(f2) - p(f1);
        while dp > PI {
            dp -= 2.0 * PI;
        }
        while dp < -PI {
            dp += 2.0 * PI;
        }
        (-dp / (2.0 * PI * (f2 - f1))).max(0.0)
    }

    /// impedance of the barrel between entry and exit, √(ΣL/ΣC) at 100 MHz (per line; Ω)
    pub fn z_barrel(&self) -> Option<f64> {
        let (lo, hi) = (self.i_in.min(self.i_out), self.i_in.max(self.i_out));
        if lo == hi {
            return None;
        }
        let f = 1e8;
        let w = 2.0 * PI * f;
        if self.has_floats() {
            // floating returns couple the spans: the series inductance between entry and exit from
            // the coupled core on infinite plates, Im(Z11 + Z22 − Z12 − Z21)/ω (a π of the node
            // capacitances and the series path; the stubs are capacitances at 100 MHz)
            let was = self.local.replace(true);
            let gz = self.group_z_hankel(f);
            let zs = |k: usize| -> Cx {
                match self.spans[k] {
                    SpanKind::Outer(l) => Cx::new(0.0, w * l),
                    SpanKind::Pair { group, scale } => gz[group].scale(scale),
                }
            };
            let yc = |k: usize| -> Cx { Cx::new(0.0, w * self.cap[k]) };
            let m = self.core_coupled(f, &zs, &yc);
            self.local.set(was);
            // Z from ABCD: Z11 = A/C, Z22 = D/C, Z12 = det/C, Z21 = 1/C
            let ci = m.c.inv();
            let det = m.a * m.d - m.b * m.c;
            let l = ((m.a + m.d - det - Cx::new(1.0, 0.0)) * ci).im / w;
            let c: f64 = self.cap[lo..=hi].iter().sum();
            return if l > 0.0 && c > 0.0 { Some((l / c).sqrt()) } else { None };
        }
        // the via's own impedance: infinite plates (a finite cavity without a return via is a
        // capacitor at low frequency)
        let gz = self.group_z_hankel(f);
        let l: f64 = (lo..hi)
            .map(|k| match self.spans[k] {
                SpanKind::Outer(l) => l,
                SpanKind::Pair { group, scale } => gz[group].scale(scale).im / w,
            })
            .sum();
        let c: f64 = self.cap[lo..=hi].iter().sum();
        if l > 0.0 && c > 0.0 {
            Some((l / c).sqrt())
        } else {
            None
        }
    }

    /// whether open stubs hang off the chain
    pub fn has_stub(&self) -> bool {
        self.i_in.min(self.i_out) != 0 || self.i_in.max(self.i_out) != self.nodes.len() - 1
    }

    /// first transmission dip below `fmax` (Hz): local minimum of |S21| below −3 dB
    pub fn notch(&self, fmax: f64) -> Option<f64> {
        if !self.has_stub() {
            return None;
        }
        // the stub's own notch: without the cavities (their resonances are not stub notches)
        let was = self.local.replace(true);
        let r = self.notch_inner(fmax);
        self.local.set(was);
        r
    }

    fn notch_inner(&self, fmax: f64) -> Option<f64> {
        let s21 = |f: f64| self.abcd(f).s_params(50.0).1.abs();
        let df = 0.5e9;
        let mut prev = (df, s21(df));
        let mut f = 2.0 * df;
        let mut falling = false;
        while f <= fmax {
            let v = s21(f);
            if v < prev.1 {
                falling = true;
            } else if falling && prev.1 < 0.708 {
                // golden-section refine between f−2df and f
                let (mut a, mut b) = (f - 2.0 * df, f);
                for _ in 0..30 {
                    let c = b - (b - a) * 0.618;
                    let d = a + (b - a) * 0.618;
                    if s21(c) < s21(d) {
                        b = d;
                    } else {
                        a = c;
                    }
                }
                return Some(0.5 * (a + b));
            } else {
                falling = false;
            }
            prev = (f, v);
            f += df;
        }
        None
    }
}

/// S-parameters of N vias that share plane-pair cavities (`CavityLink`s with the same
/// environment, whose ports contain every via): their ladders solved together, the plane pairs
/// coupling them through the environment's transfer terms. Ports 2v (entry) and 2v + 1 (exit) of
/// via v, reference `z0`; 2N × 2N row-major.
///
/// In a shared cavity the returns of all vias are shorts (the matrix over the vias' barrels with
/// every return reduced): the self term of each via's spans, as in its own ladder, and the
/// mutual term between a span k of via v and a span l of via w, Z_vw·s_k·s_l/d (the z-uniform
/// field of the pair: the voltage of a span is its height's share of the pair's voltage, driven
/// by the height-weighted mean current of the other barrel). Pairs (odd mode) are not coupled.
/// Floating returns (return groups) are left open here (the validation cases share one pair of
/// one net).
pub fn coupled_s(vias: &[&PbmVia], f: f64, z0: f64) -> Vec<Cx> {
    let w = 2.0 * PI * f;
    let nv = vias.len();
    let jwm = Cx::new(0.0, w * MU0);
    // shared environments: key → (env, [(via, group)])
    let mut shared: Vec<(std::rc::Rc<crate::via_cavity::PairEnv>, Vec<(usize, usize)>)> = vec![];
    for (v, m) in vias.iter().enumerate() {
        for (gi, g) in m.groups.iter().enumerate() {
            if let Some((ei, _)) = &g.env {
                let e = &m.envs[*ei].env;
                match shared.iter_mut().find(|(x, _)| std::rc::Rc::ptr_eq(x, e)) {
                    Some((_, l)) => l.push((v, gi)),
                    None => shared.push((e.clone(), vec![(v, gi)])),
                }
            }
        }
    }
    // per shared environment: the matrix over the vias' barrels (per metre) with all returns
    // reduced, and the barrel index of every (via, group)
    let mut zsh: Vec<(Vec<Cx>, Vec<usize>, usize)> = vec![];
    for (env, users) in &shared {
        let gm = env.green(f);
        let ne = env.ports().len();
        let mut sig: Vec<usize> = vec![];
        let mut ret: Vec<usize> = vec![];
        for &(v, gi) in users {
            let (_, idx) = vias[v].groups[gi].env.as_ref().unwrap();
            if !sig.contains(&idx[0]) {
                sig.push(idx[0]);
            }
            let kk = if vias[v].groups[gi].partner { 2 } else { 1 };
            for &q in &idx[kk..] {
                if !ret.contains(&q) {
                    ret.push(q);
                }
            }
        }
        ret.retain(|q| !sig.contains(q));
        let (ns, nr) = (sig.len(), ret.len());
        let mut zs: Vec<Cx> = (0..ns * ns).map(|q| jwm * gm[sig[q / ns] * ne + sig[q % ns]]).collect();
        if nr > 0 {
            let bb: Vec<Cx> = (0..nr * nr).map(|q| jwm * gm[ret[q / nr] * ne + ret[q % nr]]).collect();
            let ba: Vec<Cx> = (0..nr * ns).map(|q| jwm * gm[ret[q / ns] * ne + sig[q % ns]]).collect();
            let x = csolve(bb, ba, nr, ns);
            for a in 0..ns {
                for c in 0..ns {
                    let mut acc = Cx::new(0.0, 0.0);
                    for q in 0..nr {
                        acc = acc + jwm * gm[sig[a] * ne + ret[q]] * x[q * ns + c];
                    }
                    zs[a * ns + c] = zs[a * ns + c] - acc;
                }
            }
        }
        let bidx: Vec<usize> = users.iter().map(|&(v, gi)| sig.iter().position(|&q| q == vias[v].groups[gi].env.as_ref().unwrap().1[0]).unwrap()).collect();
        zsh.push((zs, bidx, ns));
    }
    let env_of = |v: usize, gi: usize| -> Option<(usize, usize)> {
        shared.iter().enumerate().find_map(|(k, (_, users))| users.iter().position(|&u| u == (v, gi)).map(|p| (k, zsh[k].1[p])))
    };
    // unknowns: per via its node voltages, then its span currents
    let mut off = vec![0usize; nv + 1];
    for v in 0..nv {
        let n = vias[v].nodes.len();
        off[v + 1] = off[v] + n + n.saturating_sub(1);
    }
    let nt = off[nv];
    let mut a = vec![Cx::new(0.0, 0.0); nt * nt];
    // spans in shared cavities: (via, span, env, barrel, scale, pair height)
    let mut cav_spans: Vec<(usize, usize, usize, usize, f64, f64)> = vec![];
    for (v, m) in vias.iter().enumerate() {
        let n = m.nodes.len();
        let gz = m.group_z(f);
        let zb = m.sigma.map(|sig| {
            let rs = (PI * f * MU0 / sig).sqrt();
            Cx::new(rs, rs).scale(1.0 / (2.0 * PI * m.a_m))
        });
        for k in 0..n {
            let yc = Cx::new(0.0, w * m.cap[k]) * Cx::new(1.0, -m.tan_eff(m.node_tand[k], f));
            let r = off[v] + k;
            a[r * nt + off[v] + k] = a[r * nt + off[v] + k] + yc;
            if k > 0 {
                a[r * nt + off[v] + n + k - 1] = a[r * nt + off[v] + n + k - 1] - Cx::new(1.0, 0.0);
            }
            if k + 1 < n {
                a[r * nt + off[v] + n + k] = a[r * nt + off[v] + n + k] + Cx::new(1.0, 0.0);
            }
        }
        for k in 0..n.saturating_sub(1) {
            let r = off[v] + n + k;
            a[r * nt + off[v] + k] = Cx::new(1.0, 0.0);
            a[r * nt + off[v] + k + 1] = Cx::new(-1.0, 0.0);
            let mut zk = match m.spans[k] {
                SpanKind::Outer(l) => Cx::new(0.0, w * l),
                SpanKind::Pair { group, scale } => match env_of(v, group) {
                    Some((e, b)) => {
                        cav_spans.push((v, k, e, b, scale, m.groups[group].pair.0 - m.groups[group].pair.1));
                        zsh[e].0[b * zsh[e].2 + b].scale(scale)
                    }
                    None => gz[group].scale(scale),
                },
            };
            if let Some(zb) = zb {
                zk = zk + zb.scale(m.span_len[k]);
            }
            a[r * nt + r] = a[r * nt + r] - zk;
        }
    }
    // mutual terms between spans of different vias in the same cavity
    for &(v, k, e, b, sk, d) in &cav_spans {
        for &(v2, k2, e2, b2, sl, _) in &cav_spans {
            if v2 == v || e2 != e {
                continue;
            }
            let zm = zsh[e].0[b * zsh[e].2 + b2].scale(sk * sl / d);
            let (r, c) = (off[v] + vias[v].nodes.len() + k, off[v2] + vias[v2].nodes.len() + k2);
            a[r * nt + c] = a[r * nt + c] - zm;
        }
    }
    // port nodes, unit currents → Z of the core
    let pn: Vec<usize> = (0..nv).flat_map(|v| [off[v] + vias[v].i_in, off[v] + vias[v].i_out]).collect();
    let np = pn.len();
    let mut rhs = vec![Cx::new(0.0, 0.0); nt * np];
    for (p, &node) in pn.iter().enumerate() {
        rhs[node * np + p] = Cx::new(1.0, 0.0);
    }
    let x = csolve(a, rhs, nt, np);
    let zc: Vec<Cx> = (0..np * np).map(|q| x[pn[q / np] * np + q % np]).collect();
    // end blocks (feed removal, trace over the antipad, junction), outer port → core
    let blocks: Vec<Abcd> = (0..nv)
        .flat_map(|v| {
            let m = vias[v];
            (0..2).map(move |side| {
                let mut t = Abcd::identity();
                let (z, ee, ln) = m.feed_cut[side];
                if ln != 0.0 {
                    t = t.mul(&tline(z, w * ee.sqrt() / C0 * ln));
                }
                if let Some((z, er, ln)) = m.ap_line[side] {
                    t = t.mul(&tline(z, w * er.sqrt() / C0 * ln));
                }
                if m.l_port[side] != 0.0 {
                    t = t.mul(&Abcd::series(Cx::new(0.0, w * m.l_port[side])));
                }
                t
            })
        })
        .collect();
    // Z = (A Zc + B)(C Zc + D)⁻¹ with diagonal A, B, C, D
    let mut num = vec![Cx::new(0.0, 0.0); np * np];
    let mut den = vec![Cx::new(0.0, 0.0); np * np];
    for p in 0..np {
        for q in 0..np {
            num[p * np + q] = blocks[p].a * zc[p * np + q];
            den[p * np + q] = blocks[p].c * zc[p * np + q];
        }
        num[p * np + p] = num[p * np + p] + blocks[p].b;
        den[p * np + p] = den[p * np + p] + blocks[p].d;
    }
    // Z = num · den⁻¹ → solve den^T Z^T = num^T
    let tr = |m: &[Cx]| -> Vec<Cx> { (0..np * np).map(|q| m[(q % np) * np + q / np]).collect() };
    let zt = csolve(tr(&den), tr(&num), np, np);
    let z = tr(&zt);
    // S = (Z − z0)(Z + z0)⁻¹ → solve (Z + z0)^T S^T = (Z − z0)^T
    let mut zp = z.clone();
    let mut zm = z;
    for p in 0..np {
        zp[p * np + p] = zp[p * np + p] + Cx::new(z0, 0.0);
        zm[p * np + p] = zm[p * np + p] - Cx::new(z0, 0.0);
    }
    tr(&csolve(tr(&zp), tr(&zm), np, np))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hankel_matches_scipy() {
        let r = [
            (0.0001, -1e-06, 0.9936340116576532, 5.937257240363565),
            (0.01, -0.0002, 0.9872409881024125, 3.00532936544549),
            (0.5, -0.01, 0.9237793482402868, 0.4468165518905931),
            (3.0, -0.05, -0.24427677897034264, -0.36022918303821055),
            (8.5, -0.1, 0.03937239126803667, -0.24421532790079736),
            (11.9, -0.2, 0.018925562459763456, 0.18829076178096354),
            (12.1, -0.2, 0.05554830924465755, 0.1792640623882569),
            (20.0, -0.3, 0.12406062130440366, -0.045470008184599287),
            (40.0, -1.0, 0.0032879478180926213, -0.04628110795114858),
        ];
        for (x, y, hr, hi) in r {
            let h = hankel2_0(Cx::new(x, y));
            let e = (h - Cx::new(hr, hi)).abs() / Cx::new(hr, hi).abs();
            assert!(e < 1e-9, "z = {x}{y:+}j: {h:?} vs {hr} {hi}, rel {e:e}");
        }
    }

    /// 8-layer style through via, L1 → L8, two GND planes inside: the transition is close to
    /// 50 Ω at low frequency and has no stub.
    fn sample() -> ViaGeom {
        let slabs = vec![Slab { z0: 0.0, z1: 1.6, er: 4.2, tand: 0.0 }];
        ViaGeom {
            slabs,
            planes: vec![Plane { z: 1.4, antipad: Some(0.4) }, Plane { z: 0.2, antipad: Some(0.4) }],
            r: 0.15,
            z0: 0.0,
            z1: 1.6,
            pads: vec![1.6, 0.0],
            pad_r: 0.25,
            returns: vec![Post { dx: 1.0, dy: 0.0, r: 0.15, z0: 0.0, z1: 1.6, joins: None, open_r: vec![] }],
            partner: None,
            junction: true,
            sigma: None,
            caps: vec![],
            feeds: [Feed { z: 1.6, z_ref: 1.4, w: 0.35, zc: 50.0, eeff: 3.3, dropped: 0.0, antipad_line: true }, Feed { z: 0.0, z_ref: 0.2, w: 0.35, zc: 50.0, eeff: 3.3, dropped: 0.0, antipad_line: true }],
            f_loss: None,
        }
    }

    #[test]
    fn through_via_is_short_and_matched() {
        let m = PbmVia::build(&sample(), Mode::Single, Grid::default());
        assert!(!m.has_stub());
        let (s11, s21, _) = m.abcd(1e9).s_params(50.0);
        assert!(s11.abs() < 0.1 && s21.abs() > 0.99, "{} {}", s11.abs(), s21.abs());
        let t = m.delay();
        assert!(t > 2e-12 && t < 20e-12, "{t}");
    }

    #[test]
    fn stub_notch() {
        // exit on an inner layer just below the top plane: long stub to the bottom
        let mut g = sample();
        g.planes.push(Plane { z: 1.0, antipad: Some(0.4) });
        g.feeds[1] = Feed { z: 1.2, z_ref: 1.0, w: 0.15, zc: 50.0, eeff: 4.2, dropped: 0.0, antipad_line: true };
        let m = PbmVia::build(&g, Mode::Single, Grid::default());
        assert!(m.has_stub());
        let f = m.notch(80e9).expect("notch");
        // quarter wave of ≈ 1.0 mm at εr 4.2 → ≈ 37 GHz; the plate capacitances lower it
        assert!(f > 15e9 && f < 30e9, "{f}");
        assert!(m.abcd(f).s_params(50.0).1.abs() < 0.5);
    }

    #[test]
    fn equivalent_radius_round_and_eccentric() {
        let nt = 128;
        let a = 0.1;
        // concentric: the radius itself
        let b = equivalent_radius(a, &vec![0.4; nt]);
        assert!((b - 0.4).abs() < 1e-9, "{b}");
        // eccentric circle R = 0.4 with the via 0.15 off centre
        let (r, d) = (0.4f64, 0.15f64);
        let rb: Vec<f64> = (0..nt)
            .map(|j| {
                let th = 2.0 * PI * j as f64 / nt as f64;
                // ray from the via (at x = d) to the circle centred at the origin
                let bx = d * th.cos();
                -bx + (bx * bx - d * d + r * r).sqrt()
            })
            .collect();
        let exact = a * ((a * a + r * r - d * d) / (2.0 * a * r)).acosh().exp();
        let b = equivalent_radius(a, &rb);
        assert!((b / exact - 1.0).abs() < 0.01, "{b} vs {exact}");
        // and the nearest-edge circle is pessimistic
        assert!(b > r - d);
        // the cached variant solves on 48 of the angles
        let bc = equivalent_radius_cached(a, &rb);
        assert!((bc / exact - 1.0).abs() < 0.015, "{bc} vs {exact}");
    }

    /// the coupled solver with a single via gives the ladder's own S-parameters
    #[test]
    fn coupled_single_matches_abcd() {
        let mut g = sample();
        g.planes.push(Plane { z: 1.0, antipad: Some(0.4) });
        g.feeds[1] = Feed { z: 1.2, z_ref: 1.0, w: 0.15, zc: 50.0, eeff: 4.2, dropped: 0.0, antipad_line: true };
        g.sigma = Some(5.8e7);
        let m = PbmVia::build(&g, Mode::Single, Grid::default());
        for f in [1e9, 9e9, 21e9] {
            let (s11, s21, s22) = m.abcd(f).s_params(50.0);
            let s = coupled_s(&[&m], f, 50.0);
            for (a, b) in [(s[0], s11), (s[2], s21), (s[3], s22)] {
                assert!((a - b).abs() < 1e-9, "{f}: {a:?} {b:?}");
            }
        }
    }

    /// a finite cavity much larger than the wavelength (lossy) gives the infinite ladder
    #[test]
    fn large_cavity_is_the_infinite_ladder() {
        use crate::via_cavity::{EnvPort, PairEnv, PairMedium, RectEnv};
        let mut g = sample();
        g.f_loss = None;
        g.slabs[0].tand = 0.1;
        let m0 = PbmVia::build(&g, Mode::Single, Grid::default());
        let (x0, y0) = (0.3, 0.25);
        let ports = vec![EnvPort { x: x0, y: y0, r: 0.15e-3 }, EnvPort { x: x0 + 1e-3, y: y0, r: 0.15e-3 }];
        let med = PairMedium { d: 1.2e-3, er: 4.2, tand: 0.1, f_loss: None, sigma: None, area: 0.0, edges: vec![] };
        let env = std::rc::Rc::new(PairEnv::Rect(RectEnv::new(0.6, 0.5, med, ports, vec![], 20e9)));
        let m1 = PbmVia::build_with(&g, Mode::Single, Grid::default(), vec![CavityLink { z_top: 1.4, z_bot: 0.2, env, origin: (x0, y0) }]);
        assert!(m1.has_cavity());
        for f in [2e9, 10e9, 20e9] {
            let (a, b) = (m0.abcd(f).s_params(50.0).1, m1.abcd(f).s_params(50.0).1);
            assert!((a - b).abs() < 2e-3, "{f}: {a:?} {b:?}");
        }
    }

    /// three planes (1.4, 0.8, 0.2 mm) in one dielectric, a through via with one return
    fn three_planes(joins: Option<Vec<f64>>) -> ViaGeom {
        let mut g = sample();
        g.planes = vec![Plane { z: 1.4, antipad: Some(0.4) }, Plane { z: 0.8, antipad: Some(0.4) }, Plane { z: 0.2, antipad: Some(0.4) }];
        g.returns[0].joins = joins;
        g
    }

    /// the ladder solved with its nodes and spans together (the core with floating returns)
    /// equals the chain when there are none
    #[test]
    fn coupled_core_equals_chain() {
        let mut g = sample();
        g.planes.push(Plane { z: 1.0, antipad: Some(0.4) });
        g.feeds[1] = Feed { z: 1.2, z_ref: 1.0, w: 0.15, zc: 50.0, eeff: 4.2, dropped: 0.0, antipad_line: true };
        g.sigma = Some(5.8e7);
        let m = PbmVia::build(&g, Mode::Single, Grid::default());
        assert!(!m.has_floats());
        for f in [0.3e9, 7e9, 23e9] {
            let w = 2.0 * PI * f;
            let gz = m.group_z(f);
            let zs = |k: usize| match m.spans[k] {
                SpanKind::Outer(l) => Cx::new(0.0, w * l),
                SpanKind::Pair { group, scale } => gz[group].scale(scale),
            };
            let yc = |k: usize| Cx::new(0.0, w * m.cap[k]);
            let (a, b) = (m.core_chain(&zs, &yc), m.core_coupled(f, &zs, &yc));
            for (x, y) in [(a.a, b.a), (a.b, b.b), (a.c, b.c), (a.d, b.d)] {
                assert!((x - y).abs() <= 1e-9 * (1.0 + x.abs()), "{f}: {x:?} {y:?}");
            }
        }
    }

    /// return groups: a return joined to the top and the bottom plane only floats across the middle
    /// one; in one dielectric, crossed by the barrel with one current, that is the same as shorts
    /// in both pairs (the middle plane carries no field of its own); joined to all three it is the
    /// ladder before return groups
    #[test]
    fn floating_return_across_a_plane() {
        let old = PbmVia::build(&three_planes(None), Mode::Single, Grid::default());
        let all = PbmVia::build(&three_planes(Some(vec![1.4, 0.8, 0.2])), Mode::Single, Grid::default());
        // the return's opening in the middle plane as large as the via's: no capacitance to speak of
        let mut gf = three_planes(Some(vec![1.4, 0.2]));
        gf.returns[0].open_r = vec![(0.8, 0.4)];
        let float = PbmVia::build(&gf, Mode::Single, Grid::default());
        // joined to the top plane only: capacitive in both pairs
        let cap = PbmVia::build(&three_planes(Some(vec![1.4])), Mode::Single, Grid::default());
        let mut gn = three_planes(None);
        gn.returns.clear();
        let none = PbmVia::build(&gn, Mode::Single, Grid::default());
        assert!(!all.has_floats() && float.has_floats() && cap.has_floats());
        assert_eq!(float.floating_pairs(), (vec![(1.4, 0.8), (0.8, 0.2)], 2));
        for f in [1e9, 5e9, 15e9] {
            let s = |m: &PbmVia| m.abcd(f).s_params(50.0).1;
            assert!((s(&old) - s(&all)).abs() < 1e-12, "{f}");
            // the barrel's own capacitances make its current non-uniform, and the return couples a
            // little to the middle plane: a small difference
            assert!((s(&float) - s(&all)).abs() < 2e-3 * f / 1e9, "{f}: {:?} {:?}", s(&float), s(&all));
            // a return joined to one plane is a poorer return than shorts, closer to none than to
            // shorts below the resonance of its post with its capacitance (here ~30 GHz)
            let (sn, sc, sa) = (s(&none).abs(), s(&cap).abs(), s(&all).abs());
            assert!(sc < sa - 1e-3 && (sc - sn).abs() < (sa - sc), "{f}: {sn} {sc} {sa}");
        }
    }

    /// a capacitor post with no impedance is a short; the roles of a return
    #[test]
    fn cap_post_and_roles() {
        let mut g = three_planes(Some(vec![1.4]));
        let p = &g.returns[0];
        assert_eq!(p.role(1.4, 0.8), Role::Cap);
        assert_eq!(Post { joins: Some(vec![]), ..p.clone() }.role(1.4, 0.8), Role::Open);
        let q = Post { joins: Some(vec![1.4, 0.2]), ..p.clone() };
        assert_eq!(q.role(1.4, 0.8), Role::Float(1.4, 0.2));
        assert_eq!(q.role(0.8, 0.2), Role::Float(1.4, 0.2));
        let q = Post { joins: Some(vec![1.4, 0.8, 0.2]), ..p.clone() };
        assert_eq!(q.role(0.8, 0.2), Role::Short);
        assert_eq!(Post { joins: None, z0: 0.5, ..p.clone() }.role(0.8, 0.2), Role::Open);
        let short = PbmVia::build(&three_planes(Some(vec![1.4, 0.8, 0.2])), Mode::Single, Grid::default());
        // the return ends at the middle plane; the capacitor post at its place shorts the lower pair
        g.returns[0].joins = Some(vec![1.4, 0.8]);
        g.returns[0].z0 = 0.8;
        g.caps = vec![CapPost { dx: 1.0, dy: 0.0, r: 0.15, z_top: 0.8, z_bot: 0.2, r_ohm: 0.0, l: 0.0, c: None }];
        let cap = PbmVia::build(&g, Mode::Single, Grid::default());
        for f in [1e9, 9e9] {
            let (a, b) = (short.abcd(f).s_params(50.0).1, cap.abcd(f).s_params(50.0).1);
            assert!((a - b).abs() < 1e-9, "{f}: {a:?} {b:?}");
        }
        // a real capacitor (1 nH) is a poorer return than the via at 10 GHz
        g.caps[0].l = 1e-9;
        g.caps[0].c = Some(100e-9);
        let lc = PbmVia::build(&g, Mode::Single, Grid::default());
        assert!(lc.abcd(10e9).s_params(50.0).1.abs() < short.abcd(10e9).s_params(50.0).1.abs());
    }

    /// a return post's capacitance to a plane it passes: the coaxial stub below the plane (the
    /// RG_s4_pwr_n4 GND via: Ø0.3 in a Ø0.7 opening of L3, 0.2 mm on to L4, εr 4.3) plus the
    /// fringe of the opening; larger openings and shorter stubs give less
    #[test]
    fn post_capacitance() {
        let slabs = vec![Slab { z0: 0.0, z1: 0.2, er: 4.3, tand: 0.0 }, Slab { z0: 0.2, z1: 1.4, er: 4.5, tand: 0.0 }, Slab { z0: 1.4, z1: 1.6, er: 4.3, tand: 0.0 }];
        let planes = [(1.4, None), (0.2, Some(0.35))];
        let c = post_plane_cap(&slabs, 0.15, 0.0, 1.6, &planes, 1);
        let coax = 2.0 * PI * EPS0 * 4.3 * 0.2e-3 / (0.35f64 / 0.15).ln();
        assert!(c > coax && c < 3.0 * coax, "{c} {coax}");
        assert!(post_plane_cap(&slabs, 0.15, 0.0, 1.6, &[(1.4, None), (0.2, Some(0.5))], 1) < c);
        assert!(post_plane_cap(&slabs, 0.15, 0.1, 1.6, &planes, 1) < c);
    }

    #[test]
    fn graded_grid_close_to_uniform() {
        let g = sample();
        let u = PbmVia::build(&g, Mode::Single, Grid::Uniform { dr: 0.01, rmax: 3.0 });
        let d = PbmVia::build(&g, Mode::Single, Grid::default());
        let (cu, cd): (f64, f64) = (u.cap.iter().sum(), d.cap.iter().sum());
        assert!((cd / cu - 1.0).abs() < 0.02, "{cu} {cd}");
        assert!(d.laplace_n * 4 < u.laplace_n, "{} {}", d.laplace_n, u.laplace_n);
    }
}

//! Via model for the channel (milestone M4).
//!
//! A signal via is modelled as a stack of short coaxial line sections, one per
//! dielectric layer it crosses. The outer conductor of each section is the
//! antipad measured in the filled planes of the bounding copper layers; for a
//! differential pair the neighbouring via of the partner net adds the odd-mode
//! coupling. Unused barrel ends (stubs) hang off the entry/exit layer as open
//! line sections, so their quarter-wave resonance appears in S21. A change of
//! reference planes adds the inductance of the return path to the nearest
//! stitching via. All of this is a design-check model (±10 … 20 % on the via
//! impedance), not a 3D solution.
//!
//! When the channel passes the feed traces (`ViaCtx::feeds`) and the via crosses at least
//! one plane, the physics-based ladder of `via_pbm` replaces the coax stack in the cascade
//! (checked against kisi-fdtd: TDR median 1.5 %, |S21| median 0.14 dB up to 20 GHz); the
//! coax sections stay for the L/C summary and as the fallback.

use crate::board::{Board, Via};
use crate::geom::{self, P};
use crate::rf::{Abcd, Cx, C0, MU0};
use serde::Serialize;
use std::f64::consts::PI;

/// One coaxial section of the barrel.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct Sec {
    /// characteristic impedance (Ω)
    pub z: f64,
    pub er: f64,
    pub tand: f64,
    /// length (m)
    pub len: f64,
    /// barrel radius (m), for the plating resistance
    pub a: f64,
}

impl Sec {
    /// series impedance per metre at f: the barrel inductance and the plating's surface impedance
    /// (1 + j)·Rs over the circumference (resistance and its internal inductance)
    pub fn z(&self, f: f64, sigma: f64) -> Cx {
        let w = 2.0 * PI * f;
        let l = self.z * self.er.sqrt() / C0;
        let r = (PI * f * MU0 / sigma).sqrt() / (2.0 * PI * self.a);
        Cx::new(r, w * l + r)
    }

    /// shunt admittance per metre at f: jωC·ε(f)/εr with the Djordjevic-Sarkar dielectric (ε′ and
    /// ε″ of one causal model)
    pub fn y(&self, f: f64) -> Cx {
        let w = 2.0 * PI * f;
        let c = self.er.sqrt() / (self.z * C0);
        Cx::new(0.0, w * c) * crate::rf::eps_c(self.er, self.tand, f).scale(1.0 / self.er)
    }

    fn abcd(&self, f: f64, sigma: f64) -> Abcd {
        let w = 2.0 * PI * f;
        let (z, y) = (self.z(f, sigma), self.y(f));
        let l = if w > 0.0 { z.im / w } else { self.z * self.er.sqrt() / C0 };
        let c = if w > 0.0 { y.im / w } else { self.er.sqrt() / (self.z * C0) * crate::rf::eps_c(self.er, self.tand, f).re / self.er };
        Abcd::line(z.re, l, y.re, c, w, self.len)
    }
}

/// Electrical model of one via transition, in the direction of travel.
#[derive(Debug, Clone, Serialize)]
pub struct ViaModel {
    pub barrel: Vec<Sec>,
    /// open stub at the entry layer (sections from the junction outwards)
    pub stub_in: Vec<Sec>,
    /// open stub at the exit layer
    pub stub_out: Vec<Sec>,
    /// return-path inductance (H), in series at the middle of the barrel
    pub l_ret: f64,
    /// physics-based ladder; replaces the coax sections in `abcd` when present
    #[serde(skip)]
    pub pbm: Option<Box<crate::via_pbm::PbmVia>>,
}

fn open_stub_y(secs: &[Sec], f: f64, sigma: f64) -> Cx {
    if secs.is_empty() {
        return Cx::new(0.0, 0.0);
    }
    let mut m = Abcd::identity();
    for s in secs {
        m = m.mul(&s.abcd(f, sigma));
    }
    // open end: I2 = 0 → Yin = C / A
    m.c / m.a
}

impl ViaModel {
    pub fn abcd(&self, f: f64, sigma: f64) -> Abcd {
        if let Some(p) = &self.pbm {
            return p.abcd(f);
        }
        let w = 2.0 * PI * f;
        let mut m = Abcd::shunt(open_stub_y(&self.stub_in, f, sigma));
        let half = self.barrel.len() / 2;
        for (i, s) in self.barrel.iter().enumerate() {
            if i == half && self.l_ret > 0.0 {
                m = m.mul(&Abcd::series(Cx::new(0.0, w * self.l_ret)));
            }
            m = m.mul(&s.abcd(f, sigma));
        }
        if self.barrel.len() < 2 && self.l_ret > 0.0 {
            m = m.mul(&Abcd::series(Cx::new(0.0, w * self.l_ret)));
        }
        m.mul(&Abcd::shunt(open_stub_y(&self.stub_out, f, sigma)))
    }
    /// time delay through the barrel (s)
    pub fn delay(&self) -> f64 {
        if let Some(p) = &self.pbm {
            return p.delay();
        }
        self.barrel.iter().map(|s| s.len * s.er.sqrt() / C0).sum()
    }
}

/// Report of one via in the channel (lengths in mm, impedance as seen by the
/// mode of the channel: differential vias report Zdiff).
#[derive(Debug, Clone, Serialize)]
pub struct ViaInfo {
    pub at: P,
    pub layer_in: u16,
    pub layer_out: u16,
    pub drill_mm: f64,
    pub pad_mm: f64,
    /// mean antipad diameter over the planes the barrel passes (None: no plane)
    pub antipad_mm: Option<f64>,
    pub barrel_mm: f64,
    pub stub_mm: f64,
    /// quarter-wave resonance of the longer stub (GHz)
    pub stub_f_ghz: Option<f64>,
    pub z_via: f64,
    pub backdrilled: bool,
    /// partner via pitch for differential pairs (mm)
    pub pitch_mm: Option<f64>,
    /// distance to the nearest stitching via used for the return path (mm)
    pub return_mm: Option<f64>,
    /// "none" | "pair" | "stitch" | "decap" | "missing"
    pub return_kind: &'static str,
    pub l_ret_nh: f64,
    /// "ladder" (physics-based, `via_pbm`) or "coax" (section stack)
    pub model: &'static str,
    /// plane pairs of two nets (GND/supply) in the via's path that no return via within 3 mm joins
    /// or bridges (`open_pairs`): the return current crosses the pair's cavity
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub open_pairs: Vec<OpenPair>,
    /// plane pairs of two nets in the via's path (open or not)
    #[serde(skip)]
    pub two_net_pairs: usize,
}

/// A plane pair of two nets (a GND and a supply plane) in a via's path that no return via joins
/// or bridges (validation/fdtd/BOARD_RETURNS.md): no via within 3 mm joins both planes, nor
/// planes above and below the pair. The ladder before return groups shorts such a pair with the
/// GND vias and calls the transition nearly ideal, while the board's plane cavity can resonate.
#[derive(Debug, Clone, Serialize)]
pub struct OpenPair {
    /// copper layers of the two planes (upper first)
    pub layers: [u16; 2],
    /// their nets
    pub nets: [u32; 2],
    /// capacitors between the two nets within 3 mm of the via
    pub caps: usize,
}

/// Opening around `at` in the plane copper of `layer`: (radius mm, net) of the
/// nearest plane fill edge if `at` lies in a hole of a plane fill within 2 mm.
pub fn plane_clearance(board: &Board, at: P, layer: u16, own: u32, partner: Option<u32>) -> Option<(f64, u32)> {
    let mut best: Option<(f64, u32)> = None;
    for z in &board.zones {
        if z.layer != layer || z.net == 0 || z.net == own || Some(z.net) == partner {
            continue;
        }
        if !board.plane_net.get(z.net as usize).copied().unwrap_or(false) {
            continue;
        }
        let bb = &z.bbox;
        if at[0] < bb.min[0] - 2.0 || at[0] > bb.max[0] + 2.0 || at[1] < bb.min[1] - 2.0 || at[1] > bb.max[1] + 2.0 {
            continue;
        }
        let pts = &z.pts;
        let n = pts.len();
        let mut inside = false;
        let mut dmin = f64::INFINITY;
        for i in 0..n {
            let a = pts[i];
            let b = pts[(i + 1) % n];
            if (a[1] > at[1]) != (b[1] > at[1]) {
                let x = a[0] + (at[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0]);
                if x > at[0] {
                    inside = !inside;
                }
            }
            let d = geom::sub(b, a);
            let l2 = geom::dot(d, d);
            if l2 < 1e-18 {
                continue; // zero-length bridge of a fractured polygon
            }
            let u = (geom::dot(geom::sub(at, a), d) / l2).clamp(0.0, 1.0);
            let dd = geom::len(geom::sub(at, geom::add(a, geom::scale(d, u))));
            dmin = dmin.min(dd);
        }
        if inside || dmin > 2.0 {
            continue;
        }
        if best.map_or(true, |(r, _)| dmin < r) {
            best = Some((dmin, z.net));
        }
    }
    best
}

/// Per-line impedance of a via section of barrel radius `a` in an antipad of
/// radius `b` (coax). With a partner via at `pitch` (mm) the odd mode of a
/// balanced shielded pair is used (Wheeler/ITT): Zdiff = 120/√εr ·
/// ln(2ν(1−σ²)/(1+σ²)), ν = pitch/(2a), σ = pitch/D, with the shield diameter D
/// taken as pitch + 2b (two round antipads or one oval).
fn z_section(a: f64, b: f64, er: f64, pitch: Option<f64>) -> f64 {
    let b = b.max(a * 1.05);
    match pitch {
        None => 60.0 / er.sqrt() * (b / a).ln(),
        Some(p) => {
            let nu = p / (2.0 * a);
            let sg = p / (p + 2.0 * b);
            let arg = (2.0 * nu * (1.0 - sg * sg) / (1.0 + sg * sg)).max(1.05);
            60.0 / er.sqrt() * arg.ln()
        }
    }
}

pub struct ViaCtx<'a> {
    pub board: &'a Board,
    pub net: u32,
    pub partner: Option<u32>,
    /// reference planes (layer indices) of the trace before and after the via
    pub refs_in: Vec<u16>,
    pub refs_out: Vec<u16>,
    /// residual stub after back-drilling for every via (mm); None: as in the layout
    pub backdrill: Option<f64>,
    /// entry and exit trace (enables the ladder model)
    pub feeds: Option<[FeedSpec; 2]>,
    /// copper conductivity for the ladder's conductor loss (S/m)
    pub sigma: Option<f64>,
    /// plane pairs as finite cavities (`via_cavity`; also with the environment variable
    /// KISI_VIA_CAVITY=1)
    pub cavity: bool,
    /// return groups per plane pair (also with KISI_VIA_RETURNS=1): a return via shorts only the
    /// pairs whose two planes it joins, floats across planes of other nets, and capacitors close
    /// pairs of two nets (validation/fdtd/BOARD_RETURNS.md)
    pub returns: bool,
}

/// Whether a via of `net` at `at` with pad radius `r` (mm) is joined to the copper of its net on
/// `layer`: a fill of that net covers it or reaches its pad (thermal spokes).
pub fn joined_to_fill(board: &Board, at: P, r: f64, layer: u16, net: u32) -> bool {
    for z in &board.zones {
        if z.layer != layer || z.net != net {
            continue;
        }
        let bb = &z.bbox;
        if at[0] < bb.min[0] - r || at[0] > bb.max[0] + r || at[1] < bb.min[1] - r || at[1] > bb.max[1] + r {
            continue;
        }
        let pts = &z.pts;
        let n = pts.len();
        let mut inside = false;
        let mut dmin = f64::INFINITY;
        for i in 0..n {
            let a = pts[i];
            let b = pts[(i + 1) % n];
            if (a[1] > at[1]) != (b[1] > at[1]) {
                let x = a[0] + (at[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0]);
                if x > at[0] {
                    inside = !inside;
                }
            }
            let d = geom::sub(b, a);
            let l2 = geom::dot(d, d);
            if l2 < 1e-18 {
                continue;
            }
            let u = (geom::dot(geom::sub(at, a), d) / l2).clamp(0.0, 1.0);
            dmin = dmin.min(geom::len(geom::sub(at, geom::add(a, geom::scale(d, u)))));
        }
        if inside || dmin <= r {
            return true;
        }
    }
    false
}

/// return groups on (`ViaCtx::returns` or KISI_VIA_RETURNS=1)
pub fn returns_on(ctx: &ViaCtx) -> bool {
    ctx.returns || std::env::var("KISI_VIA_RETURNS").is_ok_and(|x| x != "0")
}

/// Trace joining a via, as the channel models it.
#[derive(Debug, Clone, Copy)]
pub struct FeedSpec {
    /// trace width (mm)
    pub w: f64,
    /// line model of the referenced trace next to the via
    pub zc: f64,
    pub eeff: f64,
    /// length of unreferenced trace (over the antipad) next to the via that the channel left
    /// out because the ladder models it (mm)
    pub dropped: f64,
}

/// Distance from `at` to the edge of the plane opening it lies in, along 96 rays (mm, at most
/// 1.5 mm): the nearest crossing with an edge of a plane fill on `layer` (`plane`: the net found
/// by `plane_clearance`).
fn hole_profile(board: &Board, at: P, layer: u16, own: u32, partner: Option<u32>, plane: u32) -> Vec<f64> {
    const NT: usize = 96;
    const RMAX: f64 = 1.5;
    let mut edges: Vec<(P, P)> = vec![];
    for zf in &board.zones {
        // every plane fill on the layer (a split plane can bound the opening with two nets)
        if zf.layer != layer || zf.net == 0 || zf.net == own || Some(zf.net) == partner {
            continue;
        }
        if zf.net != plane && !board.plane_net.get(zf.net as usize).copied().unwrap_or(false) {
            continue;
        }
        let bb = &zf.bbox;
        if at[0] < bb.min[0] - RMAX || at[0] > bb.max[0] + RMAX || at[1] < bb.min[1] - RMAX || at[1] > bb.max[1] + RMAX {
            continue;
        }
        let n = zf.pts.len();
        for i in 0..n {
            let (p, q) = (zf.pts[i], zf.pts[(i + 1) % n]);
            // edges that can come within RMAX of the via
            if p[0].min(q[0]) > at[0] + RMAX || p[0].max(q[0]) < at[0] - RMAX || p[1].min(q[1]) > at[1] + RMAX || p[1].max(q[1]) < at[1] - RMAX {
                continue;
            }
            edges.push((p, q));
        }
    }
    if edges.is_empty() {
        return vec![];
    }
    (0..NT)
        .map(|j| {
            let th = 2.0 * PI * j as f64 / NT as f64;
            let e = [th.cos(), th.sin()];
            let mut t_min = RMAX;
            for &(p, q) in &edges {
                // at + t e = p + u (q − p)
                let d = geom::sub(q, p);
                let den = e[0] * d[1] - e[1] * d[0];
                if den.abs() < 1e-14 {
                    continue;
                }
                let w = geom::sub(p, at);
                let t = (w[0] * d[1] - w[1] * d[0]) / den;
                let u = (w[0] * e[1] - w[1] * e[0]) / den;
                if t > 1e-6 && (0.0..=1.0).contains(&u) && t < t_min {
                    t_min = t;
                }
            }
            t_min
        })
        .collect()
}

/// z of every copper layer (mm, bottom copper at 0): dielectrics plus half the copper
/// thickness on both sides, as the coax sections
fn copper_z(su: &crate::board::Stackup) -> Vec<f64> {
    let n = su.n();
    let mut z = vec![0.0; n];
    for k in (0..n.saturating_sub(1)).rev() {
        z[k] = z[k + 1] + su.dielectrics[k].thickness + 0.5 * (su.copper[k].thickness + su.copper[k + 1].thickness);
    }
    z
}

/// The return vias of the ladder: plane-net posts (`plane_posts`) within 3 mm of `v`, the nearest
/// `RETURN_POSTS`, as posts
/// relative to it (heights `z` of the copper layers, mm). With return groups (`groups`) each post
/// carries the planes of the ladder it joins (`clear`: plane opening and net per layer at the via;
/// a plane whose net is the via's and whose fill reaches it) and its openings in the others, and
/// only vias that join one of them are taken. A pour on one of the via's own signal layers `sig`
/// (the copper around its trace, which the ladder takes as a plane – BOARD_VIAS.md) counts as joined
/// by every return that spans it, as before return groups: it is the trace's coplanar return, not
/// a plane of its own (the finite cavities leave such pairs alone for the same reason); a via joined
/// only to such a pour is not taken.
#[allow(clippy::too_many_arguments)]
pub fn return_posts(board: &Board, v: &Via, net: u32, partner: Option<u32>, clear: &[Option<(f64, u32)>], z: &[f64], groups: bool, sig: [u16; 2]) -> Vec<crate::via_pbm::Post> {
    let joined = |w: &Via| post_joins(board, w, clear, z, sig);
    let posts = plane_posts(board);
    let mut cand: Vec<(f64, &Via)> = posts
        .iter()
        .filter(|w| w.net != net && Some(w.net) != partner && w.net != 0)
        .map(|w| (geom::len(geom::sub(w.at, v.at)), w))
        .filter(|&(d, _)| d > 0.01 && d < 3.0)
        .collect();
    // nearest first (stable: equal distances keep the board's order), the joins only until 8 are taken
    cand.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    let rets: Vec<(f64, crate::via_pbm::Post)> = cand
        .into_iter()
        .filter_map(|(d, w)| {
            let (joins, open_r) = if groups {
                let j = joined(w);
                // a via joined only to the pour around the trace carries no return between planes
                if !j.iter().any(|&zj| !sig.iter().any(|&k| (z[k as usize] - zj).abs() < 1e-9)) {
                    return None;
                }
                // its openings in the planes it passes without joining them (to 0.01 mm, as the
                // antipads: returns of one type then share the capacitance solve)
                let o: Vec<(f64, f64)> = (0..clear.len())
                    .filter(|&k| clear[k].is_some() && w.from as usize <= k && k <= w.to as usize && !j.iter().any(|&zj| (zj - z[k]).abs() < 1e-9))
                    .filter_map(|k| plane_clearance(board, w.at, k as u16, w.net, None).map(|c| (z[k], (c.0.max(w.drill / 2.0 + 0.02) * 100.0).round() / 100.0)))
                    .collect();
                (Some(j), o)
            } else {
                (None, vec![])
            };
            Some((d, crate::via_pbm::Post { dx: w.at[0] - v.at[0], dy: w.at[1] - v.at[1], r: (w.drill / 2.0).max(0.025), z0: z[w.to as usize], z1: z[w.from as usize], joins, open_r }))
        })
        .take(ret_cap())
        .collect();
    rets.into_iter().map(|r| r.1).collect()
}

/// The posts that can carry a via's return current: the vias of plane nets and the plated holes
/// of plane-net pads (connector pins, mounting holes) as vias spanning the pad's layers. Before
/// Block 6 only the vias (validation switch KISI_RET_PADS=0): a connector's GND pin 3 mm from the
/// ThunderScope REFINOUT2 via closes the open side of its return-via cage, without it the ladder
/// radiated 12 % of the power into the plane pairs at 20 GHz (BOARD_VIAS.md section 6).
pub fn plane_posts(board: &Board) -> Vec<Via> {
    let mut out: Vec<Via> = board.vias.iter().filter(|w| w.net != 0 && board.plane_net.get(w.net as usize).copied().unwrap_or(false)).cloned().collect();
    if !std::env::var("KISI_RET_PADS").is_ok_and(|x| x == "0") {
        for p in &board.pads {
            if !p.plated || p.drill <= 0.0 || p.net == 0 || !board.plane_net.get(p.net as usize).copied().unwrap_or(false) {
                continue;
            }
            let (Some(&from), Some(&to)) = (p.layers.iter().min(), p.layers.iter().max()) else { continue };
            if from == to {
                continue;
            }
            out.push(Via { at: p.at, size: p.size[0].min(p.size[1]).max(p.drill), drill: p.drill, from, to, net: p.net, backdrill: None, remove_unused: None });
        }
    }
    out
}

/// the most return posts the ladder takes, nearest first (8 before Block 6; validation switch
/// KISI_RET_CAP)
pub const RETURN_POSTS: usize = 12;

fn ret_cap() -> usize {
    std::env::var("KISI_RET_CAP").ok().and_then(|x| x.parse().ok()).unwrap_or(RETURN_POSTS)
}

/// The heights (`z`) of the via's planes (`clear`) that the return via `w` joins: the plane net at
/// the signal via on that layer is `w`'s net and a fill of that net reaches `w`'s pad, the layer is
/// not back-drilled; a pour on one of the signal via's own layers `sig` counts as joined (see
/// `return_posts`).
fn post_joins(board: &Board, w: &Via, clear: &[Option<(f64, u32)>], z: &[f64], sig: [u16; 2]) -> Vec<f64> {
    (0..clear.len())
        .filter(|&k| clear[k].is_some() && w.from as usize <= k && k <= w.to as usize)
        .filter(|&k| !w.backdrill.is_some_and(|(b0, b1)| b0 as usize <= k && k <= b1 as usize))
        .filter(|&k| sig.contains(&(k as u16)) || (clear[k].is_some_and(|c| c.1 == w.net) && joined_to_fill(board, w.at, w.size / 2.0 + 0.05, k as u16, w.net)))
        .map(|k| z[k])
        .collect()
}

/// The plane pairs of two nets between the via's layers `lo` and `hi` (its path) that no return
/// via joins or bridges (`OpenPair`), with the return vias as the ladder with return groups takes
/// them (`return_posts`), and the number of plane pairs of two nets in the path. Cheap when the
/// path crosses no pair of two nets: the return vias' joins are only computed for vias that do,
/// nearest first, until every pair is bridged.
#[allow(clippy::too_many_arguments)]
pub fn open_pairs(board: &Board, v: &Via, net: u32, partner: Option<u32>, clear: &[Option<(f64, u32)>], lo: u16, hi: u16, sig: [u16; 2]) -> (usize, Vec<OpenPair>) {
    let plane_k: Vec<usize> = (0..clear.len()).filter(|&k| clear[k].is_some()).collect();
    let pairs: Vec<(usize, usize)> = plane_k
        .windows(2)
        .filter(|w| w[0] >= lo as usize && w[1] <= hi as usize)
        .filter(|w| clear[w[0]].map(|c| c.1) != clear[w[1]].map(|c| c.1))
        .map(|w| (w[0], w[1]))
        .collect();
    let n_pairs = pairs.len();
    if pairs.is_empty() {
        return (0, vec![]);
    }
    let z = copper_z(board.stackup());
    // the return vias as `return_posts` takes them (the nearest `RETURN_POSTS` within 3 mm that join one of the
    // planes), their joins computed lazily: most pairs are bridged by the nearest one
    let posts = plane_posts(board);
    let mut cand: Vec<(f64, &Via)> = posts
        .iter()
        .filter(|w| w.net != net && Some(w.net) != partner && w.net != 0)
        .map(|w| (geom::len(geom::sub(w.at, v.at)), w))
        .filter(|&(d, _)| d > 0.01 && d < 3.0)
        .collect();
    cand.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    let mut open = pairs;
    let mut taken = 0;
    for (_, w) in cand {
        if open.is_empty() || taken == ret_cap() {
            break;
        }
        let j = post_joins(board, w, clear, &z, sig);
        if !j.iter().any(|&zj| !sig.iter().any(|&k| (z[k as usize] - zj).abs() < 1e-9)) {
            continue;
        }
        taken += 1;
        let p = crate::via_pbm::Post { dx: 0.0, dy: 0.0, r: 0.1, z0: z[w.to as usize], z1: z[w.from as usize], joins: Some(j), open_r: vec![] };
        open.retain(|&(ka, kb)| !matches!(p.role(z[ka], z[kb]), crate::via_pbm::Role::Short | crate::via_pbm::Role::Float(..)));
    }
    if open.is_empty() {
        return (n_pairs, vec![]);
    }
    let caps = cap_posts(board, v.at, &plane_k, clear, &z);
    let open = open
        .into_iter()
        .map(|(ka, kb)| OpenPair {
            layers: [ka as u16, kb as u16],
            nets: [clear[ka].unwrap().1, clear[kb].unwrap().1],
            caps: caps.iter().filter(|c| (c.z_top - z[ka]).abs() < 1e-6 && (c.z_bot - z[kb]).abs() < 1e-6).count(),
        })
        .collect();
    (n_pairs, open)
}

/// One line for a channel's warnings about a via with open plane pairs (`ViaInfo::open_pairs`);
/// `modelled`: the via model has return groups and finite cavities (`--via-returns --via-cavity`).
pub fn open_pair_warning(board: &Board, info: &ViaInfo, modelled: bool) -> Option<String> {
    if info.open_pairs.is_empty() {
        return None;
    }
    let su = board.stackup();
    let ln = |k: u16| su.copper.get(k as usize).map_or(format!("L{k}"), |c| c.name.clone());
    let pairs: Vec<String> = info
        .open_pairs
        .iter()
        .map(|o| {
            let caps = if o.caps > 0 { format!(", {} capacitor(s) within 3 mm", o.caps) } else { String::new() };
            format!("{}/{} ({}–{}{caps})", board.net_name(o.nets[0]), board.net_name(o.nets[1]), ln(o.layers[0]), ln(o.layers[1]))
        })
        .collect();
    let what = if modelled {
        "modelled with return groups and finite cavities (experimental)"
    } else {
        "the via model counts the GND vias as its return and may show the transition as nearly ideal while the board resonates (estimate); add a stitching via or a decoupling capacitor next to the via, or check with --via-cavity --via-returns"
    };
    let pair = if info.pitch_mm.is_some() { " (a pair: its partner via carries the differential return, so mainly the common mode is affected)" } else { "" };
    Some(format!(
        "via at ({:.2}, {:.2}) mm {} → {} crosses the plane pair {} that no via within 3 mm joins or bridges: its return current crosses the plane cavity{pair} – {what}",
        info.at[0],
        info.at[1],
        ln(info.layer_in),
        ln(info.layer_out),
        pairs.join(", ")
    ))
}

/// One line for a check row about the vias of its channel with open plane pairs
/// (`ViaInfo::open_pairs`), None without such vias; `modelled` as in `open_pair_warning`.
pub fn open_pairs_line(board: &Board, vias: &[ViaInfo], modelled: bool) -> Option<String> {
    if vias.is_empty() {
        return None;
    }
    let su = board.stackup();
    let ln = |k: u16| su.copper.get(k as usize).map_or(format!("L{k}"), |c| c.name.clone());
    let mut seen: Vec<P> = vec![];
    let mut items: Vec<String> = vec![];
    for v in vias {
        if seen.iter().any(|s| geom::len(geom::sub(*s, v.at)) < 0.01) {
            continue;
        }
        seen.push(v.at);
        let pairs: Vec<String> = v.open_pairs.iter().map(|o| format!("{}/{} {}–{}", board.net_name(o.nets[0]), board.net_name(o.nets[1]), ln(o.layers[0]), ln(o.layers[1]))).collect();
        items.push(format!("({:.2}, {:.2}) {}→{} across {}", v.at[0], v.at[1], ln(v.layer_in), ln(v.layer_out), pairs.join(", ")));
    }
    let tail = if modelled { "modelled with return groups and finite cavities (experimental)" } else { "the via model may show these transitions as nearly ideal while the board resonates (estimate); add a stitching via or a decoupling capacitor next to the via, or check with --via-cavity --via-returns" };
    Some(format!("{} via(s) cross a GND/supply plane pair that no via within 3 mm joins or bridges: {} – {tail}", items.len(), items.join("; ")))
}

/// The return group of every plane pair (neighbouring planes) of a via with return groups: layers,
/// plane nets, the return posts that short it, those that float across it, those that join planes
/// on one side only (capacitive), and the capacitors (`via_pbm::CapPost`) between its planes within
/// 3 mm (for surveys; the ladder builds the same).
pub fn return_groups(board: &Board, v: &Via, net: u32, partner: Option<u32>, sig: [u16; 2]) -> Vec<(u16, u16, u32, u32, usize, usize, usize, usize)> {
    let su = board.stackup();
    let z = copper_z(su);
    let clear: Vec<Option<(f64, u32)>> = (0..su.n() as u16).map(|k| plane_clearance(board, v.at, k, net, partner)).collect();
    let plane_k: Vec<usize> = (0..su.n()).filter(|&k| clear[k].is_some()).collect();
    let rets = return_posts(board, v, net, partner, &clear, &z, true, sig);
    let caps = cap_posts(board, v.at, &plane_k, &clear, &z);
    plane_k
        .windows(2)
        .map(|w| {
            let (za, zb) = (z[w[0]], z[w[1]]);
            let roles: Vec<crate::via_pbm::Role> = rets.iter().map(|p| p.role(za, zb)).collect();
            let short = roles.iter().filter(|r| **r == crate::via_pbm::Role::Short).count();
            let float = roles.iter().filter(|r| matches!(r, crate::via_pbm::Role::Float(..))).count();
            let capr = roles.iter().filter(|r| **r == crate::via_pbm::Role::Cap).count();
            let ncap = caps.iter().filter(|c| (c.z_top - za).abs() < 1e-6 && (c.z_bot - zb).abs() < 1e-6).count();
            (w[0] as u16, w[1] as u16, clear[w[0]].unwrap().1, clear[w[1]].unwrap().1, short, float, capr, ncap)
        })
        .collect()
}

/// Ladder model of the via, None without a plane or without feed information.
#[allow(clippy::too_many_arguments)]
fn ladder(ctx: &ViaCtx, v: &Via, l_in: u16, l_out: u16, clear: &[Option<(f64, u32)>], partner_via: Option<&Via>) -> Option<crate::via_pbm::PbmVia> {
    use crate::via_pbm::{Feed, Grid, Mode, PbmVia, Plane, Post, Slab, ViaGeom};
    let feeds = ctx.feeds?;
    // validation switch: the section-stack model only (compare_kisi.py, old against new)
    if std::env::var("KISI_VIA_MODEL").is_ok_and(|m| m == "coax") || clear.iter().all(|c| c.is_none()) {
        return None;
    }
    let board = ctx.board;
    let su = board.stackup();
    let z = copper_z(su);
    let slabs: Vec<Slab> = (0..su.n() - 1).map(|k| Slab { z0: z[k + 1], z1: z[k], er: su.dielectrics[k].epsilon_r, tand: su.dielectrics[k].loss_tangent }).collect();
    // antipads as the round opening of the same in-plane capacitance (offset, oval and shared
    // openings), to 0.01 mm: vias of one type then share the Laplace solve
    let a = (v.drill / 2.0).max(0.025);
    #[cfg(not(target_arch = "wasm32"))]
    let tp = std::time::Instant::now();
    let planes: Vec<Plane> = (0..su.n())
        .filter_map(|k| {
            clear[k].map(|c| {
                let rb = hole_profile(board, v.at, k as u16, ctx.net, ctx.partner, c.1);
                let b = if rb.is_empty() { c.0 } else { crate::via_pbm::equivalent_radius_cached(a, &rb) };
                Plane { z: z[k], antipad: Some((b * 100.0).round() / 100.0) }
            })
        })
        .collect();
    // barrel: layout range minus a back-drilled end; a residual stub length shortens it further
    let (mut top, mut bot) = (v.from as usize, v.to as usize);
    if let Some((b0, b1)) = v.backdrill {
        let (b0, b1) = (b0 as usize, b1 as usize);
        if b1 >= bot && b0 > top {
            bot = b0 - 1;
        } else if b0 <= top && b1 < bot {
            top = b1 + 1;
        }
    }
    let (lo, hi) = (l_in.min(l_out) as usize, l_in.max(l_out) as usize);
    if lo < top || hi > bot {
        return None;
    }
    let (mut z1, mut z0) = (z[top], z[bot]);
    if let Some(res) = ctx.backdrill {
        z1 = z1.min(z[lo] + res);
        z0 = z0.max(z[hi] - res);
    }
    // annular rings: on every layer of the barrel (KiCad default), or only on the connected
    // layers (and the ends with keep_end_layers)
    let mut pads = vec![z[l_in as usize], z[l_out as usize]];
    match v.remove_unused {
        None => pads.extend((top..=bot).map(|k| z[k]).filter(|&zk| zk <= z1 + 1e-9 && zk >= z0 - 1e-9)),
        Some(keep_ends) => {
            if keep_ends && (z1 - z[top]).abs() < 1e-9 {
                pads.push(z1);
            }
            if keep_ends && (z0 - z[bot]).abs() < 1e-9 {
                pads.push(z0);
            }
        }
    }
    // return groups: the planes of the ladder each return via joins (the plane net at the signal
    // via is its net and its fill reaches it), and the capacitors between two planes nearby
    let groups = returns_on(ctx);
    let plane_k: Vec<usize> = (0..su.n()).filter(|&k| clear[k].is_some()).collect();
    let rets = return_posts(board, v, ctx.net, ctx.partner, clear, &z, groups, [l_in, l_out]);
    let caps = if groups { cap_posts(board, v.at, &plane_k, clear, &z) } else { vec![] };
    let feed = |side: usize, l: u16, refs: &[u16]| -> Option<Feed> {
        let fs = feeds[side];
        // reference plane: the listed reference nearest in height (a stripline lists both), else
        // the nearest plane layer
        let dz = |k: &u16| (z[*k as usize] - z[l as usize]).abs();
        let nearest = |it: &mut dyn Iterator<Item = u16>| it.min_by(|a, b| dz(a).partial_cmp(&dz(b)).unwrap());
        let rl = nearest(&mut refs.iter().copied()).or_else(|| nearest(&mut (0..su.n() as u16).filter(|&k| clear[k as usize].is_some() && k != l)))?;
        Some(Feed { z: z[l as usize], z_ref: z[rl as usize], w: fs.w, zc: fs.zc, eeff: fs.eeff, dropped: fs.dropped, antipad_line: true })
    };
    let geom = ViaGeom {
        slabs,
        planes,
        r: (v.drill / 2.0).max(0.025),
        z0,
        z1,
        pads,
        pad_r: v.size / 2.0,
        returns: rets,
        partner: partner_via.map(|p| Post { dx: p.at[0] - v.at[0], dy: p.at[1] - v.at[1], r: (p.drill / 2.0).max(0.025), z0: z[p.to as usize], z1: z[p.from as usize], joins: None, open_r: vec![] }),
        feeds: [feed(0, l_in, &ctx.refs_in)?, feed(1, l_out, &ctx.refs_out)?],
        f_loss: None,
        junction: true,
        sigma: ctx.sigma,
        caps,
        // pours on the via's own signal layers (their planes in `planes`); validation switch
        // KISI_VIA_POURS=planes: as planes, the ladder before Block 5 (BOARD_VIAS.md)
        coplanar: if std::env::var("KISI_VIA_POURS").is_ok_and(|x| x == "planes") {
            vec![]
        } else {
            [l_in, l_out].iter().filter(|&&k| clear[k as usize].is_some()).map(|&k| z[k as usize]).collect()
        },
    };
    let mode = if partner_via.is_some() { Mode::Odd } else { Mode::Single };
    let debug = std::env::var("KISI_DEBUG_VIA").is_ok();
    // validation: the 3D references' loss model (dielectric conductivity fixed at this frequency,
    // lossless metal), as `kisi-validate via-geom` uses for the comparison
    let mut geom = geom;
    if let Some(fl) = std::env::var("KISI_VIA_3DLOSS").ok().and_then(|x| x.parse::<f64>().ok()) {
        geom.f_loss = Some(fl);
        geom.sigma = None;
    }
    #[cfg(not(target_arch = "wasm32"))]
    if debug {
        eprintln!("# planes {:.2} ms", tp.elapsed().as_secs_f64() * 1e3);
    }
    if debug {
        eprintln!("{}", serde_json::to_string(&geom).unwrap_or_default());
    }
    if debug && groups {
        // the return group of every plane pair: shorts, floating returns, capacitors
        for w in plane_k.windows(2) {
            let (za, zb) = (z[w[0]], z[w[1]]);
            let roles: Vec<crate::via_pbm::Role> = geom.returns.iter().map(|p| p.role(za, zb)).collect();
            let n_short = roles.iter().filter(|r| **r == crate::via_pbm::Role::Short).count();
            let n_float = roles.iter().filter(|r| matches!(r, crate::via_pbm::Role::Float(..))).count();
            let n_capr = roles.iter().filter(|r| **r == crate::via_pbm::Role::Cap).count();
            let n_cap = geom.caps.iter().filter(|c| (c.z_top - za).abs() < 1e-6 && (c.z_bot - zb).abs() < 1e-6).count();
            let net = |k: usize| clear[k].map_or(String::new(), |c| board.nets.get(c.1 as usize).cloned().unwrap_or_default());
            eprintln!("# return group {}-{} {}/{}: {n_short} shorts, {n_float} floating, {n_capr} capacitive, {n_cap} capacitors", w[0], w[1], net(w[0]), net(w[1]));
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    let t0 = std::time::Instant::now();
    let mut m = PbmVia::build(&geom, mode, Grid::default());
    if ctx.cavity || std::env::var("KISI_VIA_CAVITY").is_ok_and(|x| x != "0") {
        #[cfg(not(target_arch = "wasm32"))]
        let tc = std::time::Instant::now();
        let plane_layers: Vec<(u16, f64)> = (0..su.n()).filter(|&k| clear[k].is_some()).map(|k| (k as u16, z[k])).collect();
        let mut ctx2 = ViaCtx { refs_in: ctx.refs_in.clone(), refs_out: ctx.refs_out.clone(), ..*ctx };
        ctx2.sigma = geom.sigma;
        #[cfg(not(target_arch = "wasm32"))]
        let tl = std::time::Instant::now();
        let links = cavity_links(&ctx2, v.at, &geom, &plane_layers, &m, [l_in, l_out]);
        #[cfg(not(target_arch = "wasm32"))]
        CAV_TIME.with(|c| c.set(c.get() + tl.elapsed().as_secs_f64()));
        if !links.is_empty() {
            m = PbmVia::build_with(&geom, mode, Grid::default(), links);
        }
        #[cfg(not(target_arch = "wasm32"))]
        if debug {
            eprintln!("# cavities {:.1} ms", tc.elapsed().as_secs_f64() * 1e3);
        }
    }
    // validation: the via's S-parameters, 25 MHz … 25 GHz, 50 Ω, between feed lines that run to the
    // via centre (without the trace the channel left out for the ladder: the reference plane of the
    // de-embedded 3D board vias)
    #[cfg(not(target_arch = "wasm32"))]
    if let Ok(dir) = std::env::var("KISI_DUMP_VIA") {
        let mut t = String::from("# Hz S RI R 50\n");
        for k in 1..=1000 {
            let f = k as f64 * 25e6;
            let mm = m.abcd_centre(f);
            let (s11, s21, s22) = mm.s_params(50.0);
            t += &format!("{f:.6e} {:.9e} {:.9e} {:.9e} {:.9e} {:.9e} {:.9e} {:.9e} {:.9e}\n", s11.re, s11.im, s21.re, s21.im, s21.re, s21.im, s22.re, s22.im);
        }
        let tag = if m.has_cavity() { "_cav" } else { "" };
        let _ = std::fs::write(format!("{dir}/via_{:.4}_{:.4}_{}{}{tag}.s2p", v.at[0], v.at[1], l_in, l_out), t);
    }
    #[cfg(not(target_arch = "wasm32"))]
    if debug {
        let s21: Vec<String> = [2e9, 4e9, 8e9, 12e9, 16e9].iter().map(|&f| format!("{:.2}", 20.0 * m.abcd(f).s_params(50.0).1.abs().log10())).collect();
        eprintln!("# ladder build {:.2} ms, {} Laplace unknowns, nodes {:?}, C fF {:?}, |S21| dB at 2/4/8/12/16 GHz {}", t0.elapsed().as_secs_f64() * 1e3, m.laplace_n,
            m.nodes, m.cap.iter().map(|c| (c * 1e16).round() / 10.0).collect::<Vec<_>>(), s21.join(" "));
        if m.has_floats() {
            eprintln!("# return nodes (plane z mm, C fF) {:?}", m.return_nodes().iter().map(|&(z, c)| (z, (c * 1e16).round() / 10.0)).collect::<Vec<_>>());
        }
    }
    Some(m)
}

/// radius around the via within which capacitors between two of its planes enter the ladder (mm):
/// the reach of its return vias
const CAP_REACH: f64 = 3.0;

/// The capacitors between two neighbouring planes of the via of two nets (a supply and GND) within
/// `CAP_REACH` (return groups): fitted two-pad parts with one pad on each net, ESR and ESL with its
/// mounting from data/components.json ("decoupling", by package; estimates), the capacitance from
/// the footprint value, else 100 nF (an estimate), as a post of `CAVITY_CAP_POST` between the
/// pads. A capacitor joins every neighbouring pair of these two nets (as in `via_cavity`).
fn cap_posts(board: &Board, at: P, plane_k: &[usize], clear: &[Option<(f64, u32)>], z: &[f64]) -> Vec<crate::via_pbm::CapPost> {
    use crate::via_cavity::{parse_cap_value, DecapData};
    let pairs: Vec<(usize, usize, u32, u32)> = plane_k
        .windows(2)
        .filter_map(|w| {
            let (na, nb) = (clear[w[0]]?.1, clear[w[1]]?.1);
            (na != nb).then_some((w[0], w[1], na, nb))
        })
        .collect();
    if pairs.is_empty() {
        return vec![];
    }
    let mut by_fp: std::collections::HashMap<u32, Vec<usize>> = Default::default();
    for (i, p) in board.pads.iter().enumerate() {
        by_fp.entry(p.fp).or_default().push(i);
    }
    let mut out = vec![];
    let mut fps: Vec<(&u32, &Vec<usize>)> = by_fp.iter().collect();
    fps.sort();
    for (&fp, ids) in fps {
        if ids.len() != 2 || board.footprints.get(fp as usize).map_or(true, |f| f.dnp) {
            continue;
        }
        let (p0, p1) = (&board.pads[ids[0]], &board.pads[ids[1]]);
        let c = geom::lerp(p0.at, p1.at, 0.5);
        if geom::len(geom::sub(c, at)) > CAP_REACH {
            continue;
        }
        for &(ka, kb, na, nb) in &pairs {
            if (p0.net == na && p1.net == nb) || (p0.net == nb && p1.net == na) {
                let cv = parse_cap_value(&board.footprints[fp as usize].value).unwrap_or(CAVITY_CAP_C);
                let pkg = crate::topology::package_of(board, fp as usize, ids[0], ids[1]);
                let (r_ohm, l) = DecapData::builtin().rl(pkg, cv);
                out.push(crate::via_pbm::CapPost { dx: c[0] - at[0], dy: c[1] - at[1], r: CAVITY_CAP_POST, z_top: z[ka], z_bot: z[kb], r_ohm, l, c: Some(cv) });
            }
        }
    }
    out
}

/// leakage ratio below which a plane pair stays an infinite radial line (`PbmVia::pair_leakage`
/// at 10 GHz): four returns at 1 mm give 0.006 and change the via by ≤ 0.2 dB in 3D up to 25 GHz,
/// one return at 2 mm gives 0.84 (validation/fdtd/BOARD_RESONANCES.md)
const CAVITY_LEAKAGE: f64 = 0.02;
/// half-width of the window around the via for a plane pair of one net (mm) and its cell (mm)
const CAVITY_WINDOW: (f64, f64) = (5.0, 0.3);
/// frequency step (Hz) of the window's samples (open, no high-Q resonance: interpolated) and of a
/// closed region's (the channel's own step at 25 GHz / 500 points, so its points are exact)
const CAVITY_WINDOW_DF: f64 = 0.25e9;
const CAVITY_REGION_DF: f64 = 0.05e9;
/// cells of the whole region of a plane pair, least cell (mm); at most this many cells to keep
/// them at λ/6
const CAVITY_REGION: (usize, f64) = (2500, 0.3);
const CAVITY_REGION_MAX: usize = 12000;
/// mean distance of the posts of a pair of one net below which only a window around the via
/// is modelled (mm): the stitching cuts the pair off a few rows of posts away
const CAVITY_DENSE: f64 = 3.0;
/// a capacitor between the planes of a pair: ESR and ESL with its mounting from
/// data/components.json ("decoupling", by package; estimates), the capacitance from the footprint
/// value, else 100 nF (an estimate), and the radius of its post in the plane pair (mm)
const CAVITY_CAP_C: f64 = 100e-9;
const CAVITY_CAP_POST: f64 = 0.15;

/// The finite cavities of the via's plane pairs (`via_cavity`). A pair whose returns shield the
/// via (leakage below `CAVITY_LEAKAGE`) and a pair bounded by a pour on the via's own signal
/// layers stay infinite radial lines. Every other pair: its whole region on a coarse grid with the
/// posts joining both planes as shorts and the capacitors between two nets as loads (estimated
/// parasitics), magnetic walls with their radiation at its edges; a pair of one net stitched
/// densely (`CAVITY_DENSE`): a window of `CAVITY_WINDOW` around the via with every post in it, the
/// window's cuts open (matched to the outgoing wave). The ladder's returns stay its ports
/// (shorted in every pair they span, as on infinite plates; with return groups the returns that
/// short the pair or close it through their capacitance, and a pair a floating return crosses –
/// a supply plane between two planes the return vias join – stays an infinite radial line).
fn cavity_links(ctx: &ViaCtx, at: P, geom: &crate::via_pbm::ViaGeom, planes: &[(u16, f64)], m: &crate::via_pbm::PbmVia, signal: [u16; 2]) -> Vec<crate::via_pbm::CavityLink> {
    use crate::via_cavity::{pair_cavity, pair_window, DecapData, EnvPort, GridEnv, Load, PairEnv, PairMedium};
    let leak = m.pair_leakage(10e9);
    let mut out = vec![];
    for w in planes.windows(2) {
        let ((ka, za), (kb, zb)) = (w[0], w[1]);
        // a pour on the via's own signal layer is a coplanar return around its pad, not one side of
        // a plane pair (BOARD_VIAS.md): such pairs stay infinite radial lines
        if signal.contains(&ka) || signal.contains(&kb) {
            continue;
        }
        let Some(&(_, _, ratio)) = leak.iter().find(|l| (l.0 - za).abs() < 1e-6 && (l.1 - zb).abs() < 1e-6) else { continue };
        if ratio < CAVITY_LEAKAGE {
            continue;
        }
        // return groups: a pair with floating returns (a plane of another net between two planes
        // the return vias join) stays an infinite radial line with its floating returns – they
        // short the pairs' common field, and the stitching beyond them is not in a cavity of one
        // pair (BOARD_RETURNS.md)
        if geom.returns.iter().any(|p| p.joins.is_some() && matches!(p.role(za, zb), crate::via_pbm::Role::Float(..))) {
            continue;
        }
        let na = plane_clearance(ctx.board, at, ka, ctx.net, ctx.partner).map(|c| c.1);
        let nb = plane_clearance(ctx.board, at, kb, ctx.net, ctx.partner).map(|c| c.1);
        // the whole region; a pair of one net stitched densely (posts closer than
        // `CAVITY_DENSE` on average) only matters near the via: a window
        let Some(mut pc) = pair_cavity(ctx.board, at, ka, kb, ctx.net, ctx.partner, CAVITY_REGION.0, CAVITY_REGION.1) else { continue };
        // cells no coarser than λ/6 at 25 GHz (grid dispersion), up to `CAVITY_REGION_MAX` cells
        let h_lambda = crate::num::C0 / (25e9 * pc.er.sqrt()) / 6.0 * 1e3;
        if pc.mask.h > h_lambda {
            let bb = (pc.mask.nx as f64 * pc.mask.h) * (pc.mask.ny as f64 * pc.mask.h);
            let n = ((bb / (h_lambda * h_lambda)) as usize).min(CAVITY_REGION_MAX);
            if let Some(p2) = pair_cavity(ctx.board, at, ka, kb, ctx.net, ctx.partner, n, CAVITY_REGION.1) {
                pc = p2;
            }
        }
        if na == nb && !pc.shorts.is_empty() && (pc.area_mm2 / pc.shorts.len() as f64).sqrt() < CAVITY_DENSE {
            match pair_window(ctx.board, at, ka, kb, ctx.net, ctx.partner, CAVITY_WINDOW.0, CAVITY_WINDOW.1) {
                Some(w) => pc = w,
                None => continue,
            }
        }
        if pc.mask.count() < 4 {
            continue;
        }
        // ports: the via, the partner and the returns spanning the pair, at their board positions
        let mut ports = vec![EnvPort { x: at[0], y: at[1], r: geom.r }];
        if let (Some(p), true) = (&geom.partner, ctx.partner.is_some()) {
            ports.push(EnvPort { x: at[0] + p.dx, y: at[1] + p.dy, r: p.r });
        }
        for p in &geom.returns {
            // the returns that short the pair (with return groups: those joining both planes, and
            // those joining one, closing the pair through their capacitance to the other)
            let short = match p.joins {
                None => p.z0 <= zb + 1e-9 && p.z1 >= za - 1e-9,
                Some(_) => matches!(p.role(za, zb), crate::via_pbm::Role::Short | crate::via_pbm::Role::Cap),
            };
            if short {
                ports.push(EnvPort { x: at[0] + p.dx, y: at[1] + p.dy, r: p.r });
            }
        }
        let decap = DecapData::builtin();
        let loads: Vec<Load> = pc
            .caps
            .iter()
            .map(|cp| {
                let c = cp.c.unwrap_or(CAVITY_CAP_C);
                let (r_ohm, l) = decap.rl(cp.pkg, c);
                Load { at: EnvPort { x: cp.at[0], y: cp.at[1], r: CAVITY_CAP_POST }, r_ohm, l, c: Some(c) }
            })
            .collect();
        let medium = PairMedium {
            d: pc.d_mm * 1e-3,
            er: pc.er,
            tand: pc.tand,
            f_loss: geom.f_loss,
            sigma: ctx.sigma,
            area: pc.area_mm2 * 1e-6,
            edges: pc.mask.edge_runs().iter().map(|l| l * 1e-3).collect(),
        };
        // the same via and pair in another channel build (pair, partner, eye) reuses the environment
        let key = format!("{:.4},{:.4},{ka},{kb},{:?},{:?},{:?},{:?}", at[0], at[1], na, nb, ports.iter().map(|p| ((p.x * 1e4).round() as i64, (p.y * 1e4).round() as i64)).collect::<Vec<_>>(), ctx.sigma);
        if let Some(env) = ENV_CACHE.with(|c| c.borrow().get(&key).cloned()) {
            out.push(crate::via_pbm::CavityLink { z_top: za, z_bot: zb, env, origin: (at[0] * 1e-3, at[1] * 1e-3) });
            continue;
        }
        let mut env = GridEnv::new_cut(&pc.mask, medium, &pc.shorts, &loads, &ports, pc.cut);
        env.df = Some(if pc.cut.iter().any(|&c| c) { CAVITY_WINDOW_DF } else { CAVITY_REGION_DF });
        // the vias that cross the same region share its factorisations (a window is centred on its
        // via and is its own)
        env.share();
        if std::env::var("KISI_DEBUG_VIA").is_ok() {
            eprintln!(
                "# cavity at ({:.4}, {:.4}) {}-{} nets {:?}: leakage {ratio:.3}, {} cells of {:.2} mm, {} shorts, {} capacitors, {} ports, window cut {:?}",
                at[0],
                at[1],
                ka, kb, (na, nb), env.cells(), pc.mask.h, pc.shorts.len(), loads.len(), ports.len(), pc.cut
            );
        }
        let env = std::rc::Rc::new(PairEnv::Grid(env));
        ENV_CACHE.with(|c| {
            let mut c = c.borrow_mut();
            if c.len() > 256 {
                c.clear();
            }
            c.insert(key, env.clone());
        });
        out.push(crate::via_pbm::CavityLink { z_top: za, z_bot: zb, env, origin: (at[0] * 1e-3, at[1] * 1e-3) });
    }
    out
}

thread_local! {
    /// seconds spent finding the cavities of the vias (`cavity_links`), for run-time reports
    pub static CAV_TIME: std::cell::Cell<f64> = const { std::cell::Cell::new(0.0) };
    /// cavity environments by via, pair and ports (with their frequency samples)
    static ENV_CACHE: std::cell::RefCell<std::collections::HashMap<String, std::rc::Rc<crate::via_cavity::PairEnv>>> = Default::default();
}

/// Build the via model for a transition from `l_in` to `l_out` through `v`.
pub fn build(ctx: &ViaCtx, v: &Via, l_in: u16, l_out: u16) -> (ViaModel, ViaInfo) {
    let board = ctx.board;
    let su = board.stackup();
    let diff = ctx.partner.is_some();
    let a_mm = (v.drill / 2.0).max(0.025);
    let (lo, hi) = (l_in.min(l_out).max(v.from), l_in.max(l_out).min(v.to));
    // per-layer antipad radius (mm) and plane net
    let nl = su.n();
    let clear: Vec<Option<(f64, u32)>> = (0..nl as u16).map(|k| plane_clearance(board, v.at, k, ctx.net, ctx.partner)).collect();
    // partner via
    let partner_via = ctx.partner.and_then(|pn| {
        board
            .vias
            .iter()
            .filter(|w| w.net == pn)
            .map(|w| (geom::len(geom::sub(w.at, v.at)), w))
            .filter(|&(d, _)| d < 2.0 && d > v.drill)
            .min_by(|x, y| x.0.partial_cmp(&y.0).unwrap())
    });
    let pitch = partner_via.map(|p| p.0);
    // nearest stitching via of a plane net reaching the given layers
    let stitch = |la: u16, lb: u16, net: Option<u32>| -> Option<f64> {
        board
            .vias
            .iter()
            .filter(|w| w.net != ctx.net && Some(w.net) != ctx.partner && w.net != 0)
            .filter(|w| board.plane_net.get(w.net as usize).copied().unwrap_or(false))
            .filter(|w| net.map_or(true, |n| w.net == n))
            .filter(|w| w.from <= la.min(lb) && w.to >= la.max(lb))
            .map(|w| geom::len(geom::sub(w.at, v.at)))
            .min_by(|x, y| x.partial_cmp(y).unwrap())
    };
    let any_stitch = stitch(v.from, v.to, None);
    let r_fallback = any_stitch.unwrap_or(1.0).clamp(v.size / 2.0 + 0.1, 1.5);
    let sec = |k: usize| -> Sec {
        // dielectric k lies between copper k and k+1
        let d = &su.dielectrics[k];
        let len = d.thickness + 0.5 * (su.copper[k].thickness + su.copper[k + 1].thickness);
        let rs: Vec<f64> = [clear[k], clear[k + 1]].iter().flatten().map(|c| c.0).collect();
        let b = if rs.is_empty() { r_fallback } else { rs.iter().sum::<f64>() / rs.len() as f64 };
        let b = b.max(v.size / 2.0 + 0.02);
        Sec { z: z_section(a_mm, b, d.epsilon_r, pitch), er: d.epsilon_r, tand: d.loss_tangent, len: len * 1e-3, a: a_mm * 1e-3 }
    };
    let mut barrel: Vec<Sec> = (lo as usize..hi as usize).map(sec).collect();
    if l_in > l_out {
        barrel.reverse();
    }
    // stubs (sections from the junction outwards)
    let mut stub_lo: Vec<Sec> = (v.from as usize..lo as usize).rev().map(sec).collect();
    let mut stub_hi: Vec<Sec> = (hi as usize..v.to as usize).map(sec).collect();
    let mut backdrilled = false;
    if let Some((b0, b1)) = v.backdrill {
        // the drilled range removes copper; keep the part of the stub outside it
        let keep = |s: &mut Vec<Sec>, layers: Vec<usize>| {
            let n = layers.iter().take_while(|&&k| !(k as u16 >= b0 && k as u16 <= b1)).count();
            s.truncate(n);
        };
        keep(&mut stub_hi, (hi as usize..v.to as usize).map(|k| k + 1).collect());
        keep(&mut stub_lo, (v.from as usize..lo as usize).rev().collect());
        backdrilled = true;
    }
    if let Some(res) = ctx.backdrill {
        for s in [&mut stub_lo, &mut stub_hi] {
            let total: f64 = s.iter().map(|x| x.len).sum();
            if total * 1e3 > res {
                let mut left = res * 1e-3;
                let mut out = Vec::new();
                for x in s.iter() {
                    if left <= 0.0 {
                        break;
                    }
                    let mut y = *x;
                    y.len = y.len.min(left);
                    left -= y.len;
                    out.push(y);
                }
                *s = out;
                backdrilled = true;
            }
        }
    }
    let (stub_in, stub_out) = if l_in <= l_out { (stub_lo, stub_hi) } else { (stub_hi, stub_lo) };

    // return path: reference planes shared before and after → nothing to do
    let mut return_kind = "none";
    let mut return_mm = None;
    let mut l_ret = 0.0;
    let net_at = |k: u16| clear.get(k as usize).copied().flatten().map(|c| c.1);
    let shared = ctx.refs_in.iter().any(|r| ctx.refs_out.contains(r));
    if !shared && diff {
        // odd mode: the return current flows mostly on the partner via
        return_kind = "pair";
    }
    if !shared && !ctx.refs_in.is_empty() && !ctx.refs_out.is_empty() && !diff {
        // nearest pair of reference planes
        let (mut ra, mut rb, mut best) = (ctx.refs_in[0], ctx.refs_out[0], u16::MAX);
        for &x in &ctx.refs_in {
            for &y in &ctx.refs_out {
                let d = x.abs_diff(y);
                if d < best {
                    best = d;
                    ra = x;
                    rb = y;
                }
            }
        }
        let h_cav = su.gap_between(ra as usize, rb as usize) * 1e-3;
        let r_anti = clear[ra as usize].map(|c| c.0).unwrap_or(v.size / 2.0 + 0.2);
        let (na, nb) = (net_at(ra), net_at(rb));
        let s = if na.is_some() && na == nb {
            match stitch(ra, rb, na) {
                Some(d) if d < 5.0 => {
                    return_kind = "stitch";
                    d
                }
                _ => {
                    return_kind = "missing";
                    5.0
                }
            }
        } else {
            return_kind = "decap";
            5.0
        };
        return_mm = Some(s);
        l_ret = MU0 * h_cav / (2.0 * PI) * (s / r_anti).max(1.0).ln();
    }

    let barrel_mm: f64 = barrel.iter().map(|s| s.len).sum::<f64>() * 1e3;
    let stub_len = |s: &[Sec]| s.iter().map(|x| x.len).sum::<f64>();
    let (sl_in, sl_out) = (stub_len(&stub_in), stub_len(&stub_out));
    let longest = if sl_in >= sl_out { &stub_in } else { &stub_out };
    let stub_f = {
        let t: f64 = longest.iter().map(|x| x.len * x.er.sqrt()).sum();
        if t > 1e-6 { Some(C0 / (4.0 * t) / 1e9) } else { None }
    };
    let z_mean = if barrel.is_empty() {
        0.0
    } else {
        // impedance from total L and C of the barrel
        let (mut l, mut c) = (0.0, 0.0);
        for s in &barrel {
            let v = C0 / s.er.sqrt();
            l += s.z / v * s.len;
            c += s.len / (s.z * v);
        }
        (l / c).sqrt()
    };
    let rs: Vec<f64> = (lo..=hi).filter_map(|k| clear[k as usize].map(|c| c.0)).collect();
    let mut info = ViaInfo {
        at: v.at,
        layer_in: l_in,
        layer_out: l_out,
        drill_mm: v.drill,
        pad_mm: v.size,
        antipad_mm: if rs.is_empty() { None } else { Some(2.0 * rs.iter().sum::<f64>() / rs.len() as f64) },
        barrel_mm,
        stub_mm: sl_in.max(sl_out) * 1e3 + 0.0,
        stub_f_ghz: stub_f,
        z_via: if diff { 2.0 * z_mean } else { z_mean },
        backdrilled,
        pitch_mm: pitch,
        return_mm,
        return_kind,
        l_ret_nh: l_ret * 1e9,
        model: "coax",
        open_pairs: vec![],
        two_net_pairs: 0,
    };
    (info.two_net_pairs, info.open_pairs) = open_pairs(board, v, ctx.net, ctx.partner, &clear, lo, hi, [l_in, l_out]);
    let pbm = ladder(ctx, v, l_in, l_out, &clear, partner_via.map(|p| p.1)).map(Box::new);
    if let Some(p) = &pbm {
        info.model = "ladder";
        if let Some(zb) = p.z_barrel() {
            info.z_via = if diff { 2.0 * zb } else { zb };
        }
        if p.has_stub() {
            // first transmission dip of the ladder (searched up to 1.2 × the quarter-wave estimate,
            // which the plate capacitances only lower); the estimate itself if none is found
            let fmax = info.stub_f_ghz.map_or(80e9, |q| (1.2 * q * 1e9).min(80e9));
            info.stub_f_ghz = p.notch(fmax).map(|f| f / 1e9).or(info.stub_f_ghz);
        }
    }
    (ViaModel { barrel, stub_in, stub_out, l_ret, pbm }, info)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A plane fill (−10…10 mm) with round holes, fractured as KiCad writes it: each hole joined by
    /// a cut to the bottom edge (holes at y ≤ 0) or the top edge (KiCad's y points down: mirrored).
    fn fill(net: u32, name: &str, layer: &str, holes: &[(f64, f64, f64)]) -> String {
        let ring = |h: &(f64, f64, f64), top: bool| -> Vec<(f64, f64)> {
            let n = 24;
            let k0 = if top { n / 4 } else { 3 * n / 4 };
            (0..=n).map(|k| {
                let a = 2.0 * PI * ((k0 + n - k) % n) as f64 / n as f64;
                (h.0 + h.2 * a.cos(), h.1 + h.2 * a.sin())
            }).collect()
        };
        let mut low: Vec<&(f64, f64, f64)> = holes.iter().filter(|h| h.1 <= 0.0).collect();
        let mut high: Vec<&(f64, f64, f64)> = holes.iter().filter(|h| h.1 > 0.0).collect();
        low.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        high.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
        let mut pts = vec![(-10.0, -10.0)];
        for h in low {
            pts.push((h.0, -10.0));
            pts.extend(ring(h, false));
            pts.push((h.0, -10.0));
        }
        pts.extend([(10.0, -10.0), (10.0, 10.0)]);
        for h in high {
            pts.push((h.0, 10.0));
            pts.extend(ring(h, true));
            pts.push((h.0, 10.0));
        }
        pts.push((-10.0, 10.0));
        let xy: Vec<String> = pts.iter().map(|p| format!("(xy {:.4} {:.4})", p.0, -p.1)).collect();
        format!("  (zone (net {net}) (net_name \"{name}\") (layer \"{layer}\") (filled_polygon (layer \"{layer}\") (pts {})))\n", xy.join(" "))
    }

    /// The 4-layer board of the 3D case RG_s4_pwr_n4 (BOARD_RETURNS.md): In1 GND, In2 PWR, a via
    /// F.Cu → B.Cu whose B.Cu trace refers to the PWR plane, four GND vias at 1 mm that pass the
    /// PWR plane through openings (`joined`: they join it, i.e. it is a GND plane), optionally a
    /// capacitor between the planes 2 mm away.
    fn rg_board(pwr: bool, cap: bool) -> Board {
        crate::board::load_pcb(&rg_pcb(pwr, cap, "")).unwrap()
    }

    /// `rg_board`'s file with `extra` items
    fn rg_pcb(pwr: bool, cap: bool, extra: &str) -> String {
        let g: Vec<(f64, f64)> = (0..4).map(|k| (std::f64::consts::FRAC_1_SQRT_2 * if k % 2 == 0 { 1.0 } else { -1.0 }, std::f64::consts::FRAC_1_SQRT_2 * if k < 2 { 1.0 } else { -1.0 })).collect();
        let mut t = String::from("(kicad_pcb (version 20240108) (generator \"test\")\n  (layers (0 \"F.Cu\" signal) (1 \"In1.Cu\" signal) (2 \"In2.Cu\" signal) (31 \"B.Cu\" signal))\n  (setup (stackup (layer \"F.Cu\" (type \"copper\") (thickness 0.035)) (layer \"d1\" (type \"core\") (thickness 0.2) (epsilon_r 4.3) (loss_tangent 0.02)) (layer \"In1.Cu\" (type \"copper\") (thickness 0.035)) (layer \"d2\" (type \"core\") (thickness 1.2) (epsilon_r 4.5) (loss_tangent 0.02)) (layer \"In2.Cu\" (type \"copper\") (thickness 0.035)) (layer \"d3\" (type \"core\") (thickness 0.2) (epsilon_r 4.3) (loss_tangent 0.02)) (layer \"B.Cu\" (type \"copper\") (thickness 0.035))))\n  (net 0 \"\") (net 1 \"SIG\") (net 2 \"GND\") (net 3 \"PWR\")\n");
        t += "  (segment (start -5 0) (end 0 0) (width 0.35) (layer \"F.Cu\") (net 1))\n  (segment (start 0 0) (end 5 0) (width 0.35) (layer \"B.Cu\") (net 1))\n";
        t += "  (via (at 0 0) (size 0.6) (drill 0.3) (layers \"F.Cu\" \"B.Cu\") (net 1))\n";
        for p in &g {
            t += &format!("  (via (at {:.4} {:.4}) (size 0.6) (drill 0.3) (layers \"F.Cu\" \"B.Cu\") (net 2))\n", p.0, -p.1);
        }
        t += &fill(2, "GND", "In1.Cu", &[(0.0, 0.0, 0.4)]);
        let mut holes = vec![(0.0, 0.0, 0.4)];
        if pwr {
            holes.extend(g.iter().map(|p| (p.0, p.1, 0.35)));
        }
        t += &fill(if pwr { 3 } else { 2 }, if pwr { "PWR" } else { "GND" }, "In2.Cu", &holes);
        if cap {
            t += "  (footprint \"C_0402\" (layer \"F.Cu\") (at 0 -2) (property \"Reference\" \"C1\") (property \"Value\" \"100n\")\n    (pad \"1\" smd rect (at -0.48 0) (size 0.56 0.62) (layers \"F.Cu\") (net 2 \"GND\"))\n    (pad \"2\" smd rect (at 0.48 0) (size 0.56 0.62) (layers \"F.Cu\") (net 3 \"PWR\")))\n";
        }
        t += extra;
        t += ")\n";
        t
    }

    #[test]
    fn open_plane_pair_warning() {
        for (pwr, cap, open) in [(true, false, true), (true, true, true), (false, false, false)] {
            let b = rg_board(pwr, cap);
            let sig = b.nets.iter().position(|n| n == "SIG").unwrap() as u32;
            let v = b.vias.iter().find(|v| v.net == sig).unwrap();
            let ctx = ViaCtx { board: &b, net: sig, partner: None, refs_in: vec![1], refs_out: vec![2], backdrill: None, feeds: None, sigma: None, cavity: false, returns: false };
            let (_, info) = build(&ctx, v, 0, 3);
            assert_eq!(!info.open_pairs.is_empty(), open, "pwr {pwr} cap {cap}: {:?}", info.open_pairs);
            assert_eq!(info.two_net_pairs, usize::from(pwr));
            let w = open_pair_warning(&b, &info, false);
            assert_eq!(w.is_some(), open);
            if open {
                let o = &info.open_pairs[0];
                assert_eq!((o.layers, o.caps), ([1, 2], usize::from(cap)));
                assert!(w.unwrap().contains("GND/PWR (In1.Cu–In2.Cu"));
                assert!(open_pairs_line(&b, std::slice::from_ref(&info), false).unwrap().starts_with("1 via(s) cross a GND/supply plane pair"));
            }
        }
    }

    /// Block 6: a connector's plated GND pin is a return post over its layers; a non-plated hole,
    /// an SMD pad and a signal pin are not; KISI_RET_CAP is not set in tests (the nearest 12).
    #[test]
    fn plated_pads_are_return_posts() {
        let src = {
            let mut s = String::new();
            s += "  (footprint \"SMA\" (layer \"F.Cu\") (at 2 0) (property \"Reference\" \"J1\") (property \"Value\" \"SMA\")\n";
            s += "    (pad \"2\" thru_hole circle (at 0 0) (size 1.6 1.6) (drill 1.0) (layers \"*.Cu\" \"*.Mask\") (net 2 \"GND\"))\n";
            s += "    (pad \"\" np_thru_hole circle (at 0.5 1.5) (size 1.0 1.0) (drill 1.0) (layers \"*.Cu\" \"*.Mask\"))\n";
            s += "    (pad \"3\" smd rect (at -0.5 -1.5) (size 0.6 0.6) (layers \"F.Cu\") (net 2 \"GND\"))\n";
            s += "    (pad \"1\" thru_hole circle (at 0.5 -1.0) (size 1.0 1.0) (drill 0.5) (layers \"*.Cu\" \"*.Mask\") (net 1 \"SIG\")))\n";
            s
        };
        // rg_board without the PWR plane, with the connector 2 mm from the via
        let b = crate::board::load_pcb(&rg_pcb(false, false, &src)).unwrap();
        let gnd = b.nets.iter().position(|n| n == "GND").unwrap() as u32;
        let posts = plane_posts(&b);
        let pads: Vec<&Via> = posts.iter().filter(|w| (w.at[0] - 2.0).abs() < 1e-9 && w.at[1].abs() < 1e-9).collect();
        assert_eq!(pads.len(), 1, "{posts:?}");
        assert_eq!((pads[0].drill, pads[0].size, pads[0].net, pads[0].from, pads[0].to), (1.0, 1.6, gnd, 0, b.stackup().copper.len() as u16 - 1));
        assert_eq!(posts.len(), 5, "4 GND vias and the plated GND pin: {posts:?}");
        let sig = b.nets.iter().position(|n| n == "SIG").unwrap() as u32;
        let v = b.vias.iter().find(|v| v.net == sig).unwrap();
        let z = copper_z(b.stackup());
        let clear = vec![None; z.len()];
        let rets = return_posts(&b, v, sig, None, &clear, &z, false, [0, 3]);
        assert_eq!(rets.len(), 5);
        assert!(rets.iter().any(|p| (p.dx - 2.0).abs() < 1e-9 && (p.r - 0.5).abs() < 1e-9));
    }

    #[test]
    fn coax_section() {
        // 0.3 mm drill, 0.8 mm antipad, εr 4: 60/2·ln(0.4/0.15) ≈ 29.4 Ω
        let z = z_section(0.15, 0.4, 4.0, None);
        assert!((z - 29.43).abs() < 0.1, "{z}");
        // pair 0.2 mm drill at 0.7 mm pitch, 1.0 mm antipads, εr 3.7 → ≈ 100 Ω diff
        let zd = 2.0 * z_section(0.1, 0.5, 3.7, Some(0.7));
        assert!(zd > 85.0 && zd < 110.0, "{zd}");
    }

    #[test]
    fn stub_resonance_notch() {
        // 50 Ω through path, 1.5 mm open stub at εr 4 → notch near 25 GHz
        let s = Sec { z: 50.0, er: 4.0, tand: 0.0, len: 1.5e-3, a: 1e-4 };
        let m = ViaModel { barrel: vec![], stub_in: vec![], stub_out: vec![s], l_ret: 0.0, pbm: None };
        let f_res = C0 / (4.0 * 1.5e-3 * 2.0);
        let s21 = |f: f64| m.abcd(f, 5.8e7).s_params(50.0).1.abs();
        assert!(s21(f_res) < 0.1, "{}", s21(f_res));
        assert!(s21(f_res * 0.3) > 0.9);
    }
}

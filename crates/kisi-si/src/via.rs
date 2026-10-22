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
    fn abcd(&self, f: f64, sigma: f64) -> Abcd {
        let w = 2.0 * PI * f;
        let v = C0 / self.er.sqrt();
        let l = self.z / v;
        let c = 1.0 / (self.z * v);
        let rs = (PI * f * MU0 / sigma).sqrt();
        let r = rs / (2.0 * PI * self.a);
        Abcd::line(r, l, w * c * self.tand, c, w, self.len)
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
    let mut rets: Vec<(f64, Post)> = board
        .vias
        .iter()
        .filter(|w| w.net != ctx.net && Some(w.net) != ctx.partner && w.net != 0)
        .filter(|w| board.plane_net.get(w.net as usize).copied().unwrap_or(false))
        .map(|w| (geom::len(geom::sub(w.at, v.at)), w))
        .filter(|&(d, _)| d > 0.01 && d < 3.0)
        .map(|(d, w)| (d, Post { dx: w.at[0] - v.at[0], dy: w.at[1] - v.at[1], r: (w.drill / 2.0).max(0.025), z0: z[w.to as usize], z1: z[w.from as usize] }))
        .collect();
    rets.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    rets.truncate(8);
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
        returns: rets.into_iter().map(|r| r.1).collect(),
        partner: partner_via.map(|p| Post { dx: p.at[0] - v.at[0], dy: p.at[1] - v.at[1], r: (p.drill / 2.0).max(0.025), z0: z[p.to as usize], z1: z[p.from as usize] }),
        feeds: [feed(0, l_in, &ctx.refs_in)?, feed(1, l_out, &ctx.refs_out)?],
        f_loss: None,
        junction: true,
        sigma: ctx.sigma,
    };
    let mode = if partner_via.is_some() { Mode::Odd } else { Mode::Single };
    let debug = std::env::var("KISI_DEBUG_VIA").is_ok();
    #[cfg(not(target_arch = "wasm32"))]
    if debug {
        eprintln!("# planes {:.2} ms", tp.elapsed().as_secs_f64() * 1e3);
    }
    if debug {
        eprintln!("{}", serde_json::to_string(&geom).unwrap_or_default());
    }
    #[cfg(not(target_arch = "wasm32"))]
    let t0 = std::time::Instant::now();
    let m = PbmVia::build(&geom, mode, Grid::default());
    #[cfg(not(target_arch = "wasm32"))]
    if debug {
        let s21: Vec<String> = [2e9, 4e9, 8e9, 12e9, 16e9].iter().map(|&f| format!("{:.2}", 20.0 * m.abcd(f).s_params(50.0).1.abs().log10())).collect();
        eprintln!("# ladder build {:.2} ms, {} Laplace unknowns, nodes {:?}, C fF {:?}, |S21| dB at 2/4/8/12/16 GHz {}", t0.elapsed().as_secs_f64() * 1e3, m.laplace_n,
            m.nodes, m.cap.iter().map(|c| (c * 1e16).round() / 10.0).collect::<Vec<_>>(), s21.join(" "));
    }
    Some(m)
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
    let info = ViaInfo {
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
    };
    let mut info = info;
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

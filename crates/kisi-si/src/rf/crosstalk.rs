//! Crosstalk to neighbouring traces on the same layer (NEXT/FEXT estimate).

use crate::analytic;
#[allow(unused_imports)]
use super::*;
use crate::board::Board;
use crate::extract::{Analysis, FieldResult, Kind};
use crate::geom::{self};
use crate::xsgeom::{self, Extra, PlaneCut, XsDesc};
use serde::Serialize;
use std::collections::HashMap;

// --- crosstalk ----------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct Aggressor {
    pub net: u32,
    pub name: String,
    /// differential aggressor: the second net of the pair (noise of the pair driven differentially)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pair: Option<String>,
    pub layer: u16,
    pub coupled_mm: f64,
    pub gap_min_mm: f64,
    pub gap_mean_mm: f64,
    /// backward coupling coefficient (saturated NEXT, fraction)
    pub kb: f64,
    /// NEXT and FEXT at the given rise time (fraction of aggressor swing)
    pub next: f64,
    pub fext: f64,
    pub source: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct Crosstalk {
    pub net: u32,
    pub rise_ps: f64,
    /// the victim is a pair: NEXT/FEXT are differential noise (P − N) where the partner runs alongside
    pub differential: bool,
    pub aggressors: Vec<Aggressor>,
    /// geometries (id, text) a field solver can refine
    pub geoms: Vec<(String, String)>,
    pub next_total: f64,
    pub fext_total: f64,
}

/// Coupling of two lines from their modal parameters:
/// (Kb, Kf in s/m) with FEXT = Kf·len/tr.
pub fn coupling_from_modes(ze: f64, zo: f64, ee: f64, eo: f64) -> (f64, f64) {
    let ve = C0 / ee.sqrt();
    let vo = C0 / eo.sqrt();
    let (lp, lm_) = (ze / ve, zo / vo); // L+Lm, L-Lm
    let (cm_, cp) = (1.0 / (ze * ve), 1.0 / (zo * vo)); // C-Cm, C+Cm
    let l = (lp + lm_) / 2.0;
    let lm = (lp - lm_) / 2.0;
    let c = (cp + cm_) / 2.0;
    let cm = (cp - cm_) / 2.0;
    coupling_from_lc(l, lm, c, cm)
}

pub fn coupling_from_lc(l: f64, lm: f64, c: f64, cm: f64) -> (f64, f64) {
    let kb = 0.25 * (lm / l + cm / c);
    let kf = 0.5 * (cm / c - lm / l) * (l * c).sqrt();
    (kb, kf)
}

/// One victim or aggressor line of a coupling key: centre offset from the sampled victim
/// trace (mm, positive = +normal side), width (mm), weight in the noise sum.
#[derive(Clone, Copy)]
struct Line {
    d: f64,
    w: f64,
    k: f64,
}

/// Nearest parallel track of `nets` (same layer, gap ≤ gmax) at sample `p` of a trace with
/// direction `dir`: (net, gap, width, side +normal).
#[allow(clippy::too_many_arguments)]
fn near_tracks(board: &Board, grid: &HashMap<(i64, i64), Vec<usize>>, layer: u16, p: geom::P, dir: geom::P, w: f64, gmax: f64, out: &mut HashMap<u32, (f64, f64, bool)>) {
    out.clear();
    let nrm = [-dir[1], dir[0]];
    let r = (gmax + 1.0).ceil() as i64;
    let (cx, cy) = (p[0].floor() as i64, p[1].floor() as i64);
    for x in cx - r..=cx + r {
        for y in cy - r..=cy + r {
            for &oi in grid.get(&(x, y)).map(|v| v.as_slice()).unwrap_or(&[]) {
                let o = &board.tracks[oi];
                if o.layer != layer {
                    continue;
                }
                let od = geom::sub(o.b, o.a);
                let ol = geom::len(od);
                if ol < 1e-6 {
                    continue;
                }
                let odir = geom::scale(od, 1.0 / ol);
                if geom::cross(dir, odir).abs() > 0.26 {
                    continue;
                }
                let ap = geom::sub(p, o.a);
                let along = geom::dot(ap, odir);
                if along < 0.0 || along > ol {
                    continue;
                }
                let dist = geom::cross(odir, ap).abs();
                let gap = dist - (w + o.width) / 2.0;
                if gap <= 0.0 || gap > gmax {
                    continue;
                }
                let side = geom::dot(geom::sub(geom::add(o.a, geom::scale(odir, along)), p), nrm) > 0.0;
                let e = out.entry(o.net).or_insert((f64::INFINITY, o.width, side));
                if gap < e.0 {
                    *e = (gap, o.width, side);
                }
            }
        }
    }
}

fn track_grid(board: &Board, keep: impl Fn(u32) -> bool) -> HashMap<(i64, i64), Vec<usize>> {
    let mut grid: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
    for (ti, t) in board.tracks.iter().enumerate() {
        if !keep(t.net) {
            continue;
        }
        let (x0, x1) = (t.a[0].min(t.b[0]).floor() as i64, t.a[0].max(t.b[0]).floor() as i64);
        let (y0, y1) = (t.a[1].min(t.b[1]).floor() as i64, t.a[1].max(t.b[1]).floor() as i64);
        for x in x0..=x1 {
            for y in y0..=y1 {
                grid.entry((x, y)).or_default().push(ti);
            }
        }
    }
    grid
}

/// Coupling key: victim cross-section, victim partner (gap q, width q, side), aggressor
/// (net, width q, gap q, side) and, for an aggressor pair, its second line (net, width q,
/// gap between the aggressor lines q).
type Key = (u32, Option<(i64, i64, bool)>, u32, i64, i64, bool, Option<(u32, i64, i64)>);

/// Gaps are quantised in 3 % steps (fewer distinct coupling geometries; the coupling changes by
/// a few per cent per step).
fn gq(v: f64) -> i64 {
    ((v.max(0.001) / 0.001).ln() / 1.03f64.ln()).round() as i64
}
fn gv(q: i64) -> f64 {
    0.001 * 1.03f64.powi(q as i32)
}

pub fn crosstalk(board: &Board, an: &Analysis, net: u32, field: &HashMap<String, FieldResult>, rp: &RfParams) -> Crosstalk {
    let su = board.stackup();
    let partners = crate::extract::diff_partners(&board.nets);
    let partner = partners.get(net as usize).copied().flatten();
    let step = 0.25;
    // other signal tracks (supply / ground nets are quiet: not counted as aggressors)
    let grid = track_grid(board, |n| n != net && Some(n) != partner && n != 0 && !board.plane_net.get(n as usize).copied().unwrap_or(false));
    let pgrid = partner.map(|pn| track_grid(board, |n| n == pn));
    let mut acc: HashMap<Key, f64> = HashMap::new();
    let mut per_net: HashMap<u32, (f64, f64, f64, u16)> = HashMap::new(); // len, gap min, gap*len, layer
    let (mut best, mut pbest) = (HashMap::new(), HashMap::new());
    let q = |v: f64, u: f64| (v / u).round() as i64;
    for pc in an.pieces.chunks_exact(4) {
        let tr = &board.tracks[pc[0] as usize];
        if tr.net != net || tr.pad {
            continue;
        }
        let xs = &an.cross_sections[pc[3] as usize];
        let hmin = [xs.h_up, xs.h_dn].into_iter().flatten().fold(f64::INFINITY, f64::min);
        if !hmin.is_finite() {
            continue;
        }
        // beyond ~5 h the backward coupling is below ~1 %
        let gmax = (5.0 * hmin).max(3.0 * tr.width).min(2.0);
        let d = geom::sub(tr.b, tr.a);
        let len = geom::len(d);
        if len < 1e-6 {
            continue;
        }
        let dir = geom::scale(d, 1.0 / len);
        let l0 = len * pc[1];
        let l1 = len * pc[2];
        let ns = (((l1 - l0) / step).ceil() as usize).max(1);
        let seg = (l1 - l0) / ns as f64;
        for k in 0..ns {
            let p = geom::add(tr.a, geom::scale(dir, l0 + (k as f64 + 0.5) * seg));
            near_tracks(board, &grid, tr.layer, p, dir, tr.width, gmax, &mut best);
            if best.is_empty() {
                continue;
            }
            // the victim's own partner beside it (coupled section of a pair)
            let vp = match (&pgrid, partner) {
                (Some(g), Some(pn)) => {
                    near_tracks(board, g, tr.layer, p, dir, tr.width, 3.0 * tr.width + 0.5, &mut pbest);
                    pbest.get(&pn).map(|&(g, w, s)| (gq(g), q(w, 0.001), s))
                }
                _ => None,
            };
            let mut done: Vec<u32> = Vec::new();
            for (&onet, &(gap, ow, side)) in &best {
                if done.contains(&onet) {
                    continue;
                }
                // an aggressor pair: both lines on the same side
                let op = partners.get(onet as usize).copied().flatten();
                let pair = op.and_then(|o| best.get(&o).map(|&b| (o, b))).filter(|(_, b)| b.2 == side);
                let (near, ng, nw, far) = match pair {
                    Some((o, (g2, w2, _))) => {
                        done.push(o);
                        if g2 < gap {
                            (o, g2, w2, Some((onet, q(ow, 0.001), gq(gap - g2 - w2))))
                        } else {
                            (onet, gap, ow, Some((o, q(w2, 0.001), gq(g2 - gap - ow))))
                        }
                    }
                    None => (onet, gap, ow, None),
                };
                done.push(onet);
                *acc.entry((pc[3] as u32, vp, near, q(nw, 0.001), gq(ng), side, far)).or_default() += seg;
                let e = per_net.entry(near).or_insert((0.0, f64::INFINITY, 0.0, tr.layer));
                e.0 += seg;
                e.1 = e.1.min(ng);
                e.2 += ng * seg;
            }
        }
    }
    // coupling per key: noise = Σ over victim lines i and aggressor lines j of s_i·t_j·k_ij, with
    // k_ij from the two lines (others quiet at 0 V in the field geometry)
    let mut geoms: Vec<(String, String)> = Vec::new();
    let mut agg: HashMap<u32, (f64, f64, &'static str, Option<u32>)> = HashMap::new(); // next, fext, source, pair partner
    let mut victim_pair = false;
    for (&(xi, vp, near, wq, gq_, side, far), &len) in &acc {
        let xs = &an.cross_sections[xi as usize];
        let sgn = |s: bool| if s { 1.0 } else { -1.0 };
        let mut vic = vec![Line { d: 0.0, w: xs.w, k: 1.0 }];
        if let Some((pg, pw, ps)) = vp {
            let (pg, pw) = (gv(pg), pw as f64 * 0.001);
            vic.push(Line { d: sgn(ps) * (xs.w / 2.0 + pg + pw / 2.0), w: pw, k: -1.0 });
            victim_pair = true;
        }
        let (g, w) = (gv(gq_), wq as f64 * 0.001);
        let mut ag = vec![Line { d: sgn(side) * (xs.w / 2.0 + g + w / 2.0), w, k: 1.0 }];
        if let Some((_, w2q, g2q)) = far {
            let (w2, g2) = (w2q as f64 * 0.001, gv(g2q));
            ag[0].k = 0.5;
            ag.push(Line { d: sgn(side) * (xs.w / 2.0 + g + w + g2 + w2 / 2.0), w: w2, k: -0.5 });
        }
        let all: Vec<Line> = vic.iter().chain(&ag).copied().collect();
        let (mut kb_sum, mut kf_sum, mut any_field) = (0.0, 0.0, false);
        for (iv, vi) in vic.iter().enumerate() {
            for (ja, aj) in ag.iter().enumerate() {
                // field geometry in the frame of the sampled victim trace: line i = sig+, line j = sig-,
                // every other line a quiet conductor
                let mut extra = Vec::new();
                for (n, l) in all.iter().enumerate() {
                    if n == 0 {
                        continue;
                    }
                    let kind = if n == iv { "sig+" } else if n == vic.len() + ja { "sig-" } else { "gnd" };
                    extra.push(Extra { d: l.d, w: l.w, kind });
                }
                // coplanar ground only on sides without further lines
                let left_free = all.iter().all(|l| l.d <= 0.0);
                let right_free = all.iter().all(|l| l.d >= 0.0);
                let desc = XsDesc {
                    layer: xs.layer as usize,
                    w: xs.w,
                    ref_up: xs.ref_up.map(|x| x as usize),
                    ref_dn: xs.ref_dn.map(|x| x as usize),
                    s_left: if left_free { xs.s_left } else { None },
                    s_right: if right_free { xs.s_right } else { None },
                    partner: None,
                    mask: if xs.mask { su.mask_top.as_ref().or(su.mask_bottom.as_ref()).map(|m| (m.thickness, m.epsilon_r)) } else { None },
                    planes: xs.planes.iter().map(|&(k, l, r)| PlaneCut { layer: k as usize, left: l, right: r }).collect(),
                    extra,
                    trace_gnd: iv != 0,
                };
                let gtext = xsgeom::build(su, &desc);
                let gid = gtext.as_ref().map(|t| xsgeom::geom_id(t));
                let fr = gid.as_ref().and_then(|id| field.get(id));
                let edge = (aj.d - vi.d).abs() - (aj.w + vi.w) / 2.0;
                let (kb, kf) = match fr {
                    Some(f) if f.mat_l.is_some() && f.mat_c.is_some() => {
                        any_field = true;
                        let (ml, mc) = (f.mat_l.unwrap(), f.mat_c.unwrap());
                        // asymmetric lines: self terms as geometric means
                        coupling_from_lc((ml[0] * ml[3]).sqrt(), ml[1], (mc[0] * mc[3]).sqrt(), -mc[1])
                    }
                    _ => {
                        let (er, _) = xs_material(su, xs);
                        let (ze, zo, ee, eo) = match xs.kind {
                            Kind::Stripline => {
                                let b = xs.h_up.unwrap_or(0.1) + xs.h_dn.unwrap_or(0.1) + xs.t;
                                let (ze, zo) = analytic::cohn_coupled(vi.w, edge.max(0.01), b, er);
                                (ze, zo, er, er)
                            }
                            _ => {
                                let h = xs.h_dn.or(xs.h_up).unwrap_or(0.1);
                                analytic::kirschning_jansen(vi.w, edge.max(0.01), h, er)
                            }
                        };
                        // lines in between shield part of the field (estimate: 1 / (1 + w/h), w the width
                        // of the conductors in between, h the height over the nearest plane; the field
                        // solver gives 0.56 where this gives 0.4 for 0.15 mm over 0.1 mm)
                        let (lo, hi) = (vi.d.min(aj.d), vi.d.max(aj.d));
                        let wb: f64 = all.iter().filter(|l| l.d > lo + 1e-9 && l.d < hi - 1e-9).map(|l| l.w).sum();
                        let hmin = [xs.h_up, xs.h_dn].into_iter().flatten().fold(f64::INFINITY, f64::min).max(0.02);
                        let (kb, kf) = coupling_from_modes(ze, zo, ee, eo);
                        let f = 1.0 / (1.0 + wb / hmin);
                        (kb * f, kf * f)
                    }
                };
                if let (Some(id), Some(t)) = (gid, gtext) {
                    if fr.is_none() && !geoms.iter().any(|g| g.0 == id) {
                        geoms.push((id, t));
                    }
                }
                kb_sum += vi.k * aj.k * kb;
                kf_sum += vi.k * aj.k * kf;
            }
        }
        let eeff = xs.eeff.unwrap_or(3.0);
        let v = C0 / eeff.sqrt();
        let lm = len * 1e-3;
        let next = kb_sum.abs() * (2.0 * lm / (v * rp.rise)).min(1.0);
        let fext = kf_sum.abs() * lm / rp.rise;
        let e = agg.entry(near).or_insert((0.0, 0.0, "formula", None));
        e.0 = e.0.max(next);
        e.1 += fext;
        if any_field {
            e.2 = "field";
        }
        if let Some((o, _, _)) = far {
            e.3 = Some(o);
        }
    }
    let mut aggressors: Vec<Aggressor> = agg
        .into_iter()
        .map(|(onet, (next, fext, src, pair))| {
            let pn = per_net[&onet];
            Aggressor {
                net: onet,
                name: board.net_name(onet).to_string(),
                pair: pair.map(|o| board.net_name(o).to_string()),
                layer: pn.3,
                coupled_mm: pn.0,
                gap_min_mm: pn.1,
                gap_mean_mm: pn.2 / pn.0.max(1e-9),
                kb: next,
                next,
                fext,
                source: src,
            }
        })
        .filter(|a| a.coupled_mm >= 0.5)
        .collect();
    aggressors.sort_by(|a, b| (b.next + b.fext).partial_cmp(&(a.next + a.fext)).unwrap().then(a.net.cmp(&b.net)).then(a.layer.cmp(&b.layer)));
    let next_total = aggressors.iter().map(|a| a.next).fold(0.0, |x, y| x + y);
    let fext_total = aggressors.iter().map(|a| a.fext).fold(0.0, |x, y| x + y);
    Crosstalk { net, rise_ps: rp.rise * 1e12, differential: victim_pair, aggressors, geoms, next_total, fext_total }
}

/// Coupling geometries of several victim nets not yet in `field`: (id, text, coupled mm of
/// the aggressors that need it), longest first. A pair counts once.
pub fn crosstalk_geoms(board: &Board, an: &Analysis, nets: &[u32], field: &HashMap<String, FieldResult>, rp: &RfParams) -> Vec<(String, String, f64)> {
    let partners = crate::extract::diff_partners(&board.nets);
    let mut seen: std::collections::HashSet<u32> = std::collections::HashSet::new();
    let mut out: HashMap<String, (String, f64)> = HashMap::new();
    for &n in nets {
        if !seen.insert(n) {
            continue;
        }
        if let Some(p) = partners.get(n as usize).copied().flatten() {
            seen.insert(p);
        }
        let x = crosstalk(board, an, n, field, rp);
        let w: f64 = x.aggressors.iter().map(|a| a.coupled_mm).sum::<f64>() / x.geoms.len().max(1) as f64;
        for (id, t) in x.geoms {
            out.entry(id).or_insert((t, 0.0)).1 += w;
        }
    }
    let mut v: Vec<(String, String, f64)> = out.into_iter().map(|(id, (t, w))| (id, t, w)).collect();
    v.sort_by(|a, b| b.2.total_cmp(&a.2).then(a.0.cmp(&b.0)));
    v
}

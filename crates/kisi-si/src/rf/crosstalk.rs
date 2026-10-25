//! Crosstalk to neighbouring traces on the same layer (NEXT/FEXT estimate).

use super::*;
use crate::analytic;
use crate::board::Board;
use crate::extract::{Analysis, FieldResult, Kind};
use crate::geom::{self, P};
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
    /// NEXT and FEXT at the given rise time (fraction of aggressor swing); for a pair victim the
    /// differential noise P − N
    pub next: f64,
    pub fext: f64,
    /// pair victim: common-mode noise (P + N)/2
    pub next_c: f64,
    pub fext_c: f64,
    /// "field" (≥ 95 % of the coupling from field-solver cross-sections), "partly field" or "formula"
    pub source: &'static str,
}

/// Coupling along the victim's path for the chart: positions (mm from A) and, per aggressor,
/// the near-end noise seen at A from the sections around each position (window of half the
/// rise length) and the far-end noise summed up to each position.
#[derive(Debug, Clone, Serialize)]
pub struct XtProfile {
    pub s_mm: Vec<f64>,
    pub length_mm: f64,
    /// half the rise length (mm): NEXT sections within it add up
    pub window_mm: f64,
    pub items: Vec<XtProfileItem>,
}

#[derive(Debug, Clone, Serialize)]
pub struct XtProfileItem {
    pub net: u32,
    pub name: String,
    pub pair: Option<String>,
    pub next: Vec<f64>,
    pub fext: Vec<f64>,
    pub next_c: Vec<f64>,
    pub fext_c: Vec<f64>,
}

/// A coupled stretch of 0.25 mm for the board: victim piece (va → vb), aggressor piece (aa → ab),
/// layer, aggressor net and the local backward coupling coefficient.
#[derive(Debug, Clone, Serialize)]
pub struct XtSeg {
    pub net: u32,
    pub layer: u16,
    pub va: P,
    pub vb: P,
    pub aa: P,
    pub ab: P,
    pub kb: f64,
}

/// A stretch of the victim (0.25 mm) with the crosstalk that the coupling within half the rise
/// length around it causes on its own: NEXT at the near end, FEXT at the far end.
#[derive(Debug, Clone, Serialize)]
pub struct XtHot {
    pub net: u32,
    pub layer: u16,
    pub a: P,
    pub b: P,
    pub w: f64,
    pub next: f64,
    pub fext: f64,
}

/// Tracks over or under the victim on a neighbouring layer (broadside): listed, not computed.
#[derive(Debug, Clone, Serialize)]
pub struct Broadside {
    pub net: u32,
    pub name: String,
    pub layer: u16,
    pub overlap_mm: f64,
    /// vertical distance between the layers and smallest lateral offset of the centres (mm)
    pub dz_mm: f64,
    pub offset_mm: f64,
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
    /// per geometry: its share of NEXT + FEXT (the order in which refining pays off)
    #[serde(skip)]
    pub geom_weight: Vec<f64>,
    pub next_total: f64,
    pub fext_total: f64,
    pub next_total_c: f64,
    pub fext_total_c: f64,
    /// along the path and on the board (detail of one net only)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<XtProfile>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub segments: Vec<XtSeg>,
    pub broadside: Vec<Broadside>,
    /// stretches of the victim with their local crosstalk (with the path, `crosstalk_dir`)
    #[serde(skip)]
    pub hot: Vec<XtHot>,
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

/// Nearest parallel track per net at a sample: (gap, width, side +normal, foot point on that track,
/// direction of that track).
type Near = (f64, f64, bool, geom::P, geom::P);

/// Nearest parallel track of `nets` (same layer, gap ≤ gmax) at sample `p` of a trace with
/// direction `dir`: (net, gap, width, side +normal).
fn near_tracks(
    board: &Board,
    grid: &HashMap<(i64, i64), Vec<usize>>,
    layer: u16,
    p: geom::P,
    dir: geom::P,
    w: f64,
    gmax: f64,
    out: &mut HashMap<u32, Near>,
) {
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
                let foot = geom::add(o.a, geom::scale(odir, along));
                let side = geom::dot(geom::sub(foot, p), nrm) > 0.0;
                let e = out.entry(o.net).or_insert((f64::INFINITY, o.width, side, foot, odir));
                if gap < e.0 {
                    *e = (gap, o.width, side, foot, odir);
                }
            }
        }
    }
}

/// Tracks on another layer `layer2` that run over or under the sample (parallel, lateral offset
/// below half the widths plus `reach`): net → (lateral offset of the centres, width).
fn near_broadside(
    board: &Board,
    grid: &HashMap<(i64, i64), Vec<usize>>,
    layer2: u16,
    p: geom::P,
    dir: geom::P,
    w: f64,
    reach: f64,
    out: &mut HashMap<u32, (f64, f64)>,
) {
    out.clear();
    let r = (reach + w + 1.0).ceil() as i64;
    let (cx, cy) = (p[0].floor() as i64, p[1].floor() as i64);
    for x in cx - r..=cx + r {
        for y in cy - r..=cy + r {
            for &oi in grid.get(&(x, y)).map(|v| v.as_slice()).unwrap_or(&[]) {
                let o = &board.tracks[oi];
                if o.layer != layer2 {
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
                if dist > (w + o.width) / 2.0 + reach {
                    continue;
                }
                let e = out.entry(o.net).or_insert((f64::INFINITY, o.width));
                if dist < e.0 {
                    *e = (dist, o.width);
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

pub fn crosstalk(
    board: &Board,
    an: &Analysis,
    net: u32,
    field: &HashMap<String, FieldResult>,
    rp: &RfParams,
) -> Crosstalk {
    crosstalk_impl(board, an, net, field, rp, None)
}

/// As `crosstalk`, with the coupling along the path (start as for the channel: `flip`, `hint`)
/// and the coupled stretches for the board.
pub fn crosstalk_dir(
    board: &Board,
    an: &Analysis,
    net: u32,
    field: &HashMap<String, FieldResult>,
    rp: &RfParams,
    flip: bool,
    hint: Option<P>,
) -> Crosstalk {
    crosstalk_impl(board, an, net, field, rp, Some((flip, hint)))
}

/// One sample of the victim with coupling to an aggressor.
struct Smp {
    piece: usize,
    frac: f64,
    ds: f64,
    key: usize,
    near: u32,
    layer: u16,
    va: P,
    vb: P,
    aa: P,
    ab: P,
    v: f64,
    w: f64,
}

fn crosstalk_impl(
    board: &Board,
    an: &Analysis,
    net: u32,
    field: &HashMap<String, FieldResult>,
    rp: &RfParams,
    detail: Option<(bool, Option<P>)>,
) -> Crosstalk {
    let su = board.stackup();
    let partners = crate::extract::diff_partners(&board.nets);
    let partner = partners.get(net as usize).copied().flatten();
    let step = 0.25;
    let quiet = |n: u32| n == 0 || board.plane_net.get(n as usize).copied().unwrap_or(false);
    // other signal tracks (supply / ground nets are quiet: not counted as aggressors)
    let grid = track_grid(board, |n| n != net && Some(n) != partner && !quiet(n));
    let pgrid = partner.map(|pn| track_grid(board, |n| n == pn));
    let mut keys: Vec<Key> = Vec::new();
    let mut key_of: HashMap<Key, usize> = HashMap::new();
    let mut key_len: Vec<f64> = Vec::new();
    let mut smps: Vec<Smp> = Vec::new();
    let mut per_net: HashMap<u32, (f64, f64, f64, u16)> = HashMap::new(); // len, gap min, gap*len, layer
    let mut bside: HashMap<(u32, u16), (f64, f64, f64)> = HashMap::new(); // overlap, dz, min offset
    let (mut best, mut pbest, mut bbest) = (HashMap::new(), HashMap::new(), HashMap::new());
    let q = |v: f64, u: f64| (v / u).round() as i64;
    for (pi, pc) in an.pieces.as_chunks::<4>().0.iter().enumerate() {
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
        let v = C0 / xs.eeff.unwrap_or(3.0).sqrt();
        // neighbouring signal layers without a plane in between (the reference planes of this
        // cross-section are the bounds): broadside candidates
        let lay = tr.layer as usize;
        let nbs: Vec<usize> = [lay.wrapping_sub(1), lay + 1]
            .into_iter()
            .filter(|&k| k < su.n() && Some(k as u16) != xs.ref_up && Some(k as u16) != xs.ref_dn)
            .collect();
        for k in 0..ns {
            let t = l0 + (k as f64 + 0.5) * seg;
            let p = geom::add(tr.a, geom::scale(dir, t));
            for &k2 in &nbs {
                let dz = su.gap_between(lay, k2);
                near_broadside(board, &grid, k2 as u16, p, dir, tr.width, dz, &mut bbest);
                for (&on, &(off, _)) in &bbest {
                    let e = bside.entry((on, k2 as u16)).or_insert((0.0, dz, f64::INFINITY));
                    e.0 += seg;
                    e.2 = e.2.min(off);
                }
            }
            // the victim's own partner beside it (coupled section of a pair)
            let vp = match (&pgrid, partner) {
                (Some(g), Some(pn)) => {
                    near_tracks(board, g, tr.layer, p, dir, tr.width, 3.0 * tr.width + 0.5, &mut pbest);
                    pbest.get(&pn).map(|&(g, w, s, _, _)| (gq(g), q(w, 0.001), s))
                }
                _ => None,
            };
            // on the partner's side the reach counts from the partner (a neighbour of the pair
            // couples to the partner line as well)
            let reach = gmax + vp.map_or(0.0, |(g, w, _)| gv(g) + w as f64 * 0.001);
            near_tracks(board, &grid, tr.layer, p, dir, tr.width, reach, &mut best);
            if let Some((_, _, ps)) = vp {
                best.retain(|_, b| b.0 <= gmax || b.2 == ps);
            }
            if best.is_empty() {
                continue;
            }
            let mut done: Vec<u32> = Vec::new();
            for (&onet, &(gap, ow, side, foot, odir)) in &best {
                if done.contains(&onet) {
                    continue;
                }
                // an aggressor pair: both lines on the same side
                let op = partners.get(onet as usize).copied().flatten();
                let pair = op.and_then(|o| best.get(&o).map(|&b| (o, b))).filter(|(_, b)| b.2 == side);
                let (near, ng, nw, far, nfoot, ndir) = match pair {
                    Some((o, (g2, w2, _, f2, d2))) => {
                        done.push(o);
                        if g2 < gap {
                            (o, g2, w2, Some((onet, q(ow, 0.001), gq(gap - g2 - w2))), f2, d2)
                        } else {
                            (onet, gap, ow, Some((o, q(w2, 0.001), gq(g2 - gap - ow))), foot, odir)
                        }
                    }
                    None => (onet, gap, ow, None, foot, odir),
                };
                done.push(onet);
                let key: Key = (pc[3] as u32, vp, near, q(nw, 0.001), gq(ng), side, far);
                let ki = *key_of.entry(key).or_insert_with(|| {
                    keys.push(key);
                    key_len.push(0.0);
                    keys.len() - 1
                });
                key_len[ki] += seg;
                if detail.is_some() {
                    let h = geom::scale(dir, seg / 2.0);
                    // the aggressor stretch runs the same way as the victim's
                    let oh = geom::scale(ndir, if geom::dot(ndir, dir) >= 0.0 { seg / 2.0 } else { -seg / 2.0 });
                    let frac = if pc[2] > pc[1] { ((t / len - pc[1]) / (pc[2] - pc[1])).clamp(0.0, 1.0) } else { 0.5 };
                    smps.push(Smp {
                        piece: pi,
                        frac,
                        ds: seg,
                        key: ki,
                        near,
                        layer: tr.layer,
                        va: geom::sub(p, h),
                        vb: geom::add(p, h),
                        aa: geom::sub(nfoot, oh),
                        ab: geom::add(nfoot, oh),
                        v,
                        w: tr.width,
                    });
                }
                let e = per_net.entry(near).or_insert((0.0, f64::INFINITY, 0.0, tr.layer));
                e.0 += seg;
                e.1 = e.1.min(ng);
                e.2 += ng * seg;
            }
        }
    }
    // coupling per key: noise = Σ over victim lines i and aggressor lines j of s_i·t_j·k_ij, with
    // k_ij from the two lines (others quiet at 0 V in the field geometry); a pair victim gives the
    // differential noise (s = +1, −1) and the common-mode noise (s = ½, ½)
    let mut geoms: Vec<(String, String)> = Vec::new();
    let mut geom_weight: Vec<f64> = Vec::new();
    let mut agg: HashMap<u32, (f64, f64, f64, f64, (f64, f64), Option<u32>)> = HashMap::new(); // next, fext, next_c, fext_c, (field, all) contribution, pair partner
    let mut victim_pair = false;
    let mut kcoef: Vec<(f64, f64, f64, f64)> = Vec::with_capacity(keys.len()); // kb, kf, kb_c, kf_c
    for (ki, &(xi, vp, near, wq, gq_, side, far)) in keys.iter().enumerate() {
        let len = key_len[ki];
        let xs = &an.cross_sections[xi as usize];
        let sgn = |s: bool| if s { 1.0 } else { -1.0 };
        let mut vic = vec![Line { d: 0.0, w: xs.w, k: 1.0 }];
        let mut kc = vec![1.0];
        if let Some((pg, pw, ps)) = vp {
            let (pg, pw) = (gv(pg), pw as f64 * 0.001);
            vic.push(Line { d: sgn(ps) * (xs.w / 2.0 + pg + pw / 2.0), w: pw, k: -1.0 });
            kc = vec![0.5, 0.5];
            victim_pair = true;
        }
        let (g, w) = (gv(gq_), wq as f64 * 0.001);
        // one line of a differential aggressor (its partner out of reach or on the other side)
        // carries half the differential swing
        let half = far.is_none() && partners.get(near as usize).copied().flatten().is_some();
        let mut ag = vec![Line { d: sgn(side) * (xs.w / 2.0 + g + w / 2.0), w, k: if half { 0.5 } else { 1.0 } }];
        if let Some((_, w2q, g2q)) = far {
            let (w2, g2) = (w2q as f64 * 0.001, gv(g2q));
            ag[0].k = 0.5;
            ag.push(Line { d: sgn(side) * (xs.w / 2.0 + g + w + g2 + w2 / 2.0), w: w2, k: -0.5 });
        }
        let all: Vec<Line> = vic.iter().chain(&ag).copied().collect();
        let v = C0 / xs.eeff.unwrap_or(3.0).sqrt();
        let lm = len * 1e-3;
        let sat = (2.0 * lm / (v * rp.rise)).min(1.0);
        let (mut kb_sum, mut kf_sum, mut kb_c, mut kf_c) = (0.0, 0.0, 0.0, 0.0);
        let (mut w_field, mut w_all) = (0.0, 0.0); // contribution from field solutions / in all
        for (iv, vi) in vic.iter().enumerate() {
            for (ja, aj) in ag.iter().enumerate() {
                // field geometry in the frame of the sampled victim trace: line i = sig+, line j = sig-,
                // every other line a quiet conductor
                let mut extra = Vec::new();
                for (n, l) in all.iter().enumerate() {
                    if n == 0 {
                        continue;
                    }
                    let kind = if n == iv {
                        "sig+"
                    } else if n == vic.len() + ja {
                        "sig-"
                    } else {
                        "gnd"
                    };
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
                    mask: if xs.mask {
                        su.mask_top.as_ref().or(su.mask_bottom.as_ref()).map(|m| (m.thickness, m.epsilon_r))
                    } else {
                        None
                    },
                    planes: xs
                        .planes
                        .iter()
                        .map(|&(k, l, r)| PlaneCut { layer: k as usize, left: l, right: r })
                        .collect(),
                    extra,
                    trace_gnd: iv != 0,
                };
                let gtext = xsgeom::build(su, &desc);
                let gid = gtext.as_ref().map(|t| xsgeom::geom_id(t));
                let fr = gid.as_ref().and_then(|id| field.get(id));
                let edge = (aj.d - vi.d).abs() - (aj.w + vi.w) / 2.0;
                let (kb, kf) = match fr {
                    Some(f) if f.mat_l.is_some() && f.mat_c.is_some() => {
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
                let wt = (vi.k * aj.k).abs() * (kb.abs() * sat + kf.abs() * lm / rp.rise);
                w_all += wt;
                if fr.is_some_and(|f| f.mat_l.is_some() && f.mat_c.is_some()) {
                    w_field += wt;
                }
                if let (Some(id), Some(t)) = (gid, gtext) {
                    if fr.is_none() {
                        match geoms.iter().position(|g| g.0 == id) {
                            Some(i) => geom_weight[i] += wt,
                            None => {
                                geoms.push((id, t));
                                geom_weight.push(wt);
                            }
                        }
                    }
                }
                kb_sum += vi.k * aj.k * kb;
                kf_sum += vi.k * aj.k * kf;
                kb_c += kc[iv] * aj.k * kb;
                kf_c += kc[iv] * aj.k * kf;
            }
        }
        kcoef.push((kb_sum, kf_sum, kb_c, kf_c));
        let e = agg.entry(near).or_insert((0.0, 0.0, 0.0, 0.0, (0.0, 0.0), None));
        e.0 = e.0.max(kb_sum.abs() * sat);
        e.1 += kf_sum.abs() * lm / rp.rise;
        e.2 = e.2.max(kb_c.abs() * sat);
        e.3 += kf_c.abs() * lm / rp.rise;
        e.4 .0 += w_field;
        e.4 .1 += w_all;
        if let Some((o, _, _)) = far {
            e.5 = Some(o);
        }
    }
    let mut aggressors: Vec<Aggressor> = agg
        .into_iter()
        .map(|(onet, (next, fext, next_c, fext_c, src, pair))| {
            let pn = per_net[&onet];
            Aggressor {
                net: onet,
                name: board.net_name(onet).to_string(),
                pair: pair
                    .or_else(|| partners.get(onet as usize).copied().flatten())
                    .map(|o| board.net_name(o).to_string()),
                layer: pn.3,
                coupled_mm: pn.0,
                gap_min_mm: pn.1,
                gap_mean_mm: pn.2 / pn.0.max(1e-9),
                kb: next,
                next,
                fext,
                next_c,
                fext_c,
                source: if src.0 >= 0.95 * src.1 && src.1 > 0.0 {
                    "field"
                } else if src.0 > 0.0 {
                    "partly field"
                } else {
                    "formula"
                },
            }
        })
        .filter(|a| a.coupled_mm >= 0.5)
        .collect();
    aggressors.sort_by(|a, b| {
        (b.next + b.fext).partial_cmp(&(a.next + a.fext)).unwrap().then(a.net.cmp(&b.net)).then(a.layer.cmp(&b.layer))
    });
    let sum = |f: &dyn Fn(&Aggressor) -> f64| aggressors.iter().map(f).fold(0.0, |x, y| x + y);
    let (next_total, fext_total, next_total_c, fext_total_c) =
        (sum(&|a| a.next), sum(&|a| a.fext), sum(&|a| a.next_c), sum(&|a| a.fext_c));
    let mut broadside: Vec<Broadside> = bside
        .into_iter()
        .filter(|(_, b)| b.0 >= 0.5)
        .map(|((on, l), b)| Broadside {
            net: on,
            name: board.net_name(on).to_string(),
            layer: l,
            overlap_mm: b.0,
            dz_mm: b.1,
            offset_mm: b.2,
        })
        .collect();
    broadside.sort_by(|a, b| b.overlap_mm.total_cmp(&a.overlap_mm).then(a.net.cmp(&b.net)));
    // profile along the path and the stretches for the board
    let (mut profile, mut segments, mut hot) = (None, Vec::new(), Vec::new());
    if let Some((flip, hint)) = detail {
        if let Some(path) = net_path_info(board, an, net, flip, hint) {
            // piece → position on the path (main path, branches from their junction)
            let mut at: HashMap<usize, (f64, f64, bool)> = HashMap::new();
            for pp in &path.pieces {
                at.insert(pp.piece, (pp.s0, pp.s1, pp.rev));
            }
            for b in &path.branches {
                for pp in &b.pieces {
                    at.entry(pp.piece).or_insert((b.at_s + pp.s0, b.at_s + pp.s1, pp.rev));
                }
            }
            let length = at.values().map(|x| x.1).fold(path.length_mm, f64::max);
            let nb = ((length / step).ceil() as usize).max(1);
            let shown: Vec<u32> = aggressors.iter().take(5).map(|a| a.net).collect();
            let mut dens: HashMap<u32, [Vec<f64>; 4]> =
                shown.iter().map(|&n| (n, [vec![0.0; nb], vec![0.0; nb], vec![0.0; nb], vec![0.0; nb]])).collect();
            let mut vsum = (0.0, 0.0);
            // all aggressors (differential noise for a pair): local crosstalk per stretch
            let (mut tb, mut tf) = (vec![0.0; nb], vec![0.0; nb]);
            let bin_of = |m: &Smp| {
                at.get(&m.piece).map(|&(s0, s1, rev)| {
                    let sp = if rev { s1 - m.frac * (s1 - s0) } else { s0 + m.frac * (s1 - s0) };
                    ((sp / step) as usize).min(nb - 1)
                })
            };
            for m in &smps {
                let (kb, kf, kbc, kfc) = kcoef[m.key];
                segments.push(XtSeg {
                    net: m.near,
                    layer: m.layer,
                    va: m.va,
                    vb: m.vb,
                    aa: m.aa,
                    ab: m.ab,
                    kb: kb.abs(),
                });
                vsum.0 += m.v * m.ds;
                vsum.1 += m.ds;
                let Some(b) = bin_of(m) else { continue };
                tb[b] += kb * m.ds;
                tf[b] += kf * m.ds;
                let Some(d) = dens.get_mut(&m.near) else { continue };
                d[0][b] += kb * m.ds;
                d[1][b] += kf * m.ds;
                d[2][b] += kbc * m.ds;
                d[3][b] += kfc * m.ds;
            }
            let v = if vsum.1 > 0.0 { vsum.0 / vsum.1 } else { C0 / 3f64.sqrt() };
            // NEXT at A from the sections within half the rise length around each position;
            // FEXT reaching B, summed from A up to each position
            let window = (v * rp.rise / 2.0 * 1e3).max(step);
            let nw = ((window / step).round() as usize).max(1);
            // sum over the window centred on each bin
            let win = |d: &Vec<f64>| -> Vec<f64> {
                let mut c = vec![0.0; nb + 1];
                for i in 0..nb {
                    c[i + 1] = c[i] + d[i];
                }
                (0..nb)
                    .map(|i| {
                        let lo = i.saturating_sub(nw / 2);
                        let hi = (lo + nw).min(nb);
                        c[hi] - c[lo]
                    })
                    .collect()
            };
            let next_of = |d: &Vec<f64>| -> Vec<f64> { win(d).into_iter().map(|x| (x / window).abs()).collect() };
            let fext_of = |d: &Vec<f64>| -> Vec<f64> {
                let mut acc = 0.0;
                d.iter()
                    .map(|x| {
                        acc += x * 1e-3 / rp.rise;
                        acc.abs()
                    })
                    .collect()
            };
            let items = shown
                .iter()
                .map(|&n| {
                    let d = &dens[&n];
                    let a = aggressors.iter().find(|a| a.net == n).unwrap();
                    XtProfileItem {
                        net: n,
                        name: a.name.clone(),
                        pair: a.pair.clone(),
                        next: next_of(&d[0]),
                        fext: fext_of(&d[1]),
                        next_c: next_of(&d[2]),
                        fext_c: fext_of(&d[3]),
                    }
                })
                .collect();
            // local crosstalk of each sampled stretch (once per stretch, not per aggressor)
            let (wb, wf) = (next_of(&tb), win(&tf));
            let mut seen = std::collections::HashSet::new();
            for m in &smps {
                let Some(b) = bin_of(m) else { continue };
                if !seen.insert((m.piece, (m.frac * 1e6) as i64)) {
                    continue;
                }
                let (nx, fx) = (wb[b], (wf[b] * 1e-3 / rp.rise).abs());
                if nx.max(fx) >= 1e-3 {
                    hot.push(XtHot { net, layer: m.layer, a: m.va, b: m.vb, w: m.w, next: nx, fext: fx });
                }
            }
            profile = Some(XtProfile {
                s_mm: (0..nb).map(|i| (i as f64 + 0.5) * step).collect(),
                length_mm: length,
                window_mm: window,
                items,
            });
        }
    }
    Crosstalk {
        net,
        rise_ps: rp.rise * 1e12,
        differential: victim_pair,
        aggressors,
        geoms,
        geom_weight,
        next_total,
        fext_total,
        next_total_c,
        fext_total_c,
        profile,
        segments,
        broadside,
        hot,
    }
}

/// Stage 1 of the coupling analysis for many nets (formulas, or the field solver where it has
/// already solved): one row per victim (a pair once), sorted by the larger of NEXT and FEXT.
#[derive(Debug, Clone, Serialize)]
pub struct XtRow {
    pub net: u32,
    pub partner: Option<u32>,
    pub name: String,
    pub differential: bool,
    pub next: f64,
    pub fext: f64,
    pub next_c: f64,
    pub fext_c: f64,
    pub worst: Option<String>,
    pub worst_pair: Option<String>,
    /// "field", "formula" or "partly field"
    pub source: &'static str,
    /// coupling cross-sections not solved yet that carry at least a tenth of the threshold
    pub open: usize,
    pub broadside: usize,
    pub critical: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct XtScan {
    pub rows: Vec<XtRow>,
    pub threshold: f64,
    /// coupling cross-sections of the critical victims not solved yet: (id, text, weight, victims)
    pub critical_geoms: Vec<(String, String, f64, Vec<u32>)>,
    /// stretches of the critical victims (both lines of a pair) whose local crosstalk reaches the
    /// threshold, for the board
    pub map: Vec<XtHot>,
}

pub fn crosstalk_scan(
    board: &Board,
    an: &Analysis,
    nets: &[u32],
    field: &HashMap<String, FieldResult>,
    rp: &RfParams,
    threshold: f64,
) -> XtScan {
    let partners = crate::extract::diff_partners(&board.nets);
    let mut seen: std::collections::HashSet<u32> = std::collections::HashSet::new();
    let mut rows = Vec::new();
    let mut geo: HashMap<String, (String, f64, Vec<u32>)> = HashMap::new();
    let mut map = Vec::new();
    for &n in nets {
        if !seen.insert(n) {
            continue;
        }
        let partner = partners.get(n as usize).copied().flatten();
        if let Some(p) = partner {
            seen.insert(p);
        }
        let x = crosstalk_dir(board, an, n, field, rp, false, None);
        let worst = x.aggressors.first();
        let nf = x.aggressors.iter().filter(|a| a.source == "field").count();
        let critical = x.next_total.max(x.fext_total) >= threshold;
        if critical {
            // the cross-sections that carry the coupling (from a tenth of the threshold on)
            for ((id, t), &w) in x.geoms.iter().zip(&x.geom_weight) {
                if w >= 0.1 * threshold {
                    let e = geo.entry(id.clone()).or_insert((t.clone(), 0.0, Vec::new()));
                    e.1 += w;
                    if !e.2.contains(&n) {
                        e.2.push(n);
                    }
                }
            }
            // the stretches that make it critical, on both lines of a pair
            map.extend(x.hot.iter().filter(|h| h.next.max(h.fext) >= threshold).cloned());
            if let Some(p) = partner.filter(|_| x.differential) {
                map.extend(
                    crosstalk_dir(board, an, p, field, rp, false, None)
                        .hot
                        .into_iter()
                        .filter(|h| h.next.max(h.fext) >= threshold),
                );
            }
        }
        rows.push(XtRow {
            net: n,
            partner,
            name: board.net_name(n).to_string(),
            differential: x.differential,
            next: x.next_total,
            fext: x.fext_total,
            next_c: x.next_total_c,
            fext_c: x.fext_total_c,
            worst: worst.map(|a| a.name.clone()),
            worst_pair: worst.and_then(|a| a.pair.clone()),
            source: if nf > 0 && nf == x.aggressors.len() {
                "field"
            } else if nf > 0 || x.aggressors.iter().any(|a| a.source != "formula") {
                "partly field"
            } else {
                "formula"
            },
            open: x.geom_weight.iter().filter(|&&w| w >= 0.1 * threshold).count(),
            broadside: x.broadside.len(),
            critical,
        });
    }
    rows.sort_by(|a, b| b.next.max(b.fext).total_cmp(&a.next.max(a.fext)).then(a.net.cmp(&b.net)));
    let mut critical_geoms: Vec<(String, String, f64, Vec<u32>)> =
        geo.into_iter().map(|(id, (t, w, v))| (id, t, w, v)).collect();
    critical_geoms.sort_by(|a, b| b.2.total_cmp(&a.2).then(a.0.cmp(&b.0)));
    XtScan { rows, threshold, critical_geoms, map }
}

/// Coupling cross-sections of the critical victims among `nets` (stage 2): see `crosstalk_scan`.
pub fn crosstalk_geoms(
    board: &Board,
    an: &Analysis,
    nets: &[u32],
    field: &HashMap<String, FieldResult>,
    rp: &RfParams,
) -> Vec<(String, String, f64)> {
    crosstalk_scan(board, an, nets, field, rp, XT_CRITICAL)
        .critical_geoms
        .into_iter()
        .map(|(id, t, w, _)| (id, t, w))
        .collect()
}

/// Victims with NEXT or FEXT from this fraction of the aggressor swing on are critical (kisi
/// guide value): their coupling cross-sections go to the field solver in stage 2.
pub const XT_CRITICAL: f64 = 0.01;

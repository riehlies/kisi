//! Crosstalk to neighbouring traces on the same layer (NEXT/FEXT estimate).

use crate::analytic;
#[allow(unused_imports)]
use super::*;
use crate::board::Board;
use crate::extract::{Analysis, FieldResult, Kind};
use crate::geom::{self};
use crate::xsgeom::{self, PlaneCut, XsDesc};
use serde::Serialize;
use std::collections::HashMap;

// --- crosstalk ----------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct Aggressor {
    pub net: u32,
    pub name: String,
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

pub fn crosstalk(board: &Board, an: &Analysis, net: u32, field: &HashMap<String, FieldResult>, rp: &RfParams) -> Crosstalk {
    let su = board.stackup();
    let partner = crate::extract::diff_partners(&board.nets).get(net as usize).copied().flatten();
    let step = 0.25;
    // index of other-net tracks (grid 1 mm)
    let mut grid: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
    for (ti, t) in board.tracks.iter().enumerate() {
        // supply / ground nets are quiet: not counted as aggressors
        if t.net == net || Some(t.net) == partner || t.net == 0 || board.plane_net.get(t.net as usize).copied().unwrap_or(false) {
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
    // (aggressor net, xs, aggressor width q, gap q, side) -> length
    let mut acc: HashMap<(u32, u32, i64, i64, bool), f64> = HashMap::new();
    let mut per_net: HashMap<u32, (f64, f64, f64, u16)> = HashMap::new(); // len, gap min, gap*len, layer
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
        let nrm = [-dir[1], dir[0]];
        let l0 = len * pc[1];
        let l1 = len * pc[2];
        let ns = (((l1 - l0) / step).ceil() as usize).max(1);
        let seg = (l1 - l0) / ns as f64;
        for k in 0..ns {
            let p = geom::add(tr.a, geom::scale(dir, l0 + (k as f64 + 0.5) * seg));
            let mut best: HashMap<u32, (f64, f64, bool)> = HashMap::new(); // net -> (gap, width, side)
            let r = (gmax + 1.0).ceil() as i64;
            let (cx, cy) = (p[0].floor() as i64, p[1].floor() as i64);
            for x in cx - r..=cx + r {
                for y in cy - r..=cy + r {
                    for &oi in grid.get(&(x, y)).map(|v| v.as_slice()).unwrap_or(&[]) {
                        let o = &board.tracks[oi];
                        if o.layer != tr.layer {
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
                        let gap = dist - (tr.width + o.width) / 2.0;
                        if gap <= 0.0 || gap > gmax {
                            continue;
                        }
                        let side = geom::dot(geom::sub(geom::add(o.a, geom::scale(odir, along)), p), nrm) > 0.0;
                        let e = best.entry(o.net).or_insert((f64::INFINITY, o.width, side));
                        if gap < e.0 {
                            *e = (gap, o.width, side);
                        }
                    }
                }
            }
            for (onet, (gap, ow, side)) in best {
                let gq = (gap / 0.01).round() as i64;
                let wq = (ow / 0.001).round() as i64;
                *acc.entry((onet, pc[3] as u32, wq, gq, side)).or_default() += seg;
                let e = per_net.entry(onet).or_insert((0.0, f64::INFINITY, 0.0, tr.layer));
                e.0 += seg;
                e.1 = e.1.min(gap);
                e.2 += gap * seg;
            }
        }
    }
    // coupling per key
    let mut geoms: Vec<(String, String)> = Vec::new();
    let mut agg: HashMap<u32, (f64, f64, &'static str)> = HashMap::new(); // next, fext, source
    for (&(onet, xi, wq, gq, side), &len) in &acc {
        let xs = &an.cross_sections[xi as usize];
        let gap = gq as f64 * 0.01;
        let ow = wq as f64 * 0.001;
        // field geometry: victim with the aggressor as partner
        let desc = XsDesc {
            layer: xs.layer as usize,
            w: xs.w,
            ref_up: xs.ref_up.map(|x| x as usize),
            ref_dn: xs.ref_dn.map(|x| x as usize),
            s_left: if side { None } else { xs.s_left },
            s_right: if side { xs.s_right } else { None },
            partner: Some((gap, ow, true)),
            mask: if xs.mask { su.mask_top.as_ref().or(su.mask_bottom.as_ref()).map(|m| (m.thickness, m.epsilon_r)) } else { None },
            planes: xs.planes.iter().map(|&(k, l, r)| PlaneCut { layer: k as usize, left: l, right: r }).collect(),
        };
        let gtext = xsgeom::build(su, &desc);
        let gid = gtext.as_ref().map(|t| xsgeom::geom_id(t));
        let fr = gid.as_ref().and_then(|id| field.get(id));
        let (kb, kf, src) = match fr {
            Some(f) if f.mat_l.is_some() && f.mat_c.is_some() => {
                let (ml, mc) = (f.mat_l.unwrap(), f.mat_c.unwrap());
                let (kb, kf) = coupling_from_lc(ml[0], ml[1], mc[0], -mc[1]);
                (kb, kf, "field")
            }
            Some(FieldResult { zodd: Some(zo), zeven: Some(ze), eeff_odd: Some(eo), eeff_even: Some(ee), .. }) => {
                let (kb, kf) = coupling_from_modes(*ze, *zo, *ee, *eo);
                (kb, kf, "field")
            }
            _ => {
                let (er, _) = xs_material(su, xs);
                let (ze, zo, ee, eo) = match xs.kind {
                    Kind::Stripline => {
                        let b = xs.h_up.unwrap_or(0.1) + xs.h_dn.unwrap_or(0.1) + xs.t;
                        let (ze, zo) = analytic::cohn_coupled(xs.w, gap, b, er);
                        (ze, zo, er, er)
                    }
                    _ => {
                        let h = xs.h_dn.or(xs.h_up).unwrap_or(0.1);
                        analytic::kirschning_jansen(xs.w, gap, h, er)
                    }
                };
                let (kb, kf) = coupling_from_modes(ze, zo, ee, eo);
                (kb, kf, "formula")
            }
        };
        if let (Some(id), Some(t)) = (gid, gtext) {
            if fr.is_none() && !geoms.iter().any(|g| g.0 == id) {
                geoms.push((id, t));
            }
        }
        let eeff = xs.eeff.unwrap_or(3.0);
        let v = C0 / eeff.sqrt();
        let lm = len * 1e-3;
        let next = kb * (2.0 * lm / (v * rp.rise)).min(1.0);
        let fext = kf.abs() * lm / rp.rise;
        let e = agg.entry(onet).or_insert((0.0, 0.0, "formula"));
        e.0 = e.0.max(next);
        e.1 += fext;
        if src == "field" {
            e.2 = "field";
        }
    }
    let mut aggressors: Vec<Aggressor> = agg
        .into_iter()
        .map(|(onet, (next, fext, src))| {
            let pn = per_net[&onet];
            let kb_sat = next; // already limited by length
            Aggressor {
                net: onet,
                name: board.net_name(onet).to_string(),
                layer: pn.3,
                coupled_mm: pn.0,
                gap_min_mm: pn.1,
                gap_mean_mm: pn.2 / pn.0.max(1e-9),
                kb: kb_sat,
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
    Crosstalk { net, rise_ps: rp.rise * 1e12, aggressors, geoms, next_total, fext_total }
}


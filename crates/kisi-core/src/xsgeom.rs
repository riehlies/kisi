//! Cross-section geometry for field solvers.
//!
//! Builds the 2D cross-section of a trace (or diff pair) from the stackup and
//! the extracted surroundings, and writes it in the custom-geometry text format
//! of js_2d_fields (H. Forstén, GPL-3.0). That text is the interface between
//! kisi and any field-solver backend: a backend receives the text and returns
//! impedances, so a new upstream solver version or another backend can be
//! plugged in without touching the extraction.
//!
//! Orientation: y up; the trace is always placed with its free side up, i.e. a
//! bottom-layer trace is mirrored so that solder mask and air are on top.

use crate::board::Stackup;
use std::fmt::Write;

/// Copper extent on one layer of the modelled stack, measured from the
/// sampled trace centre along the cross-section.
/// * reference layer: copper ends `left`/`right` away (None = continues)
/// * any other layer (opening, e.g. an antipad under a pad): copper starts
///   `left`/`right` away (None = no copper on that side within the window)
#[derive(Debug, Clone, PartialEq)]
pub struct PlaneCut {
    pub layer: usize,
    pub left: Option<f64>,
    pub right: Option<f64>,
}

/// Everything the geometry needs about one cross-section.
#[derive(Debug, Clone)]
pub struct XsDesc {
    pub layer: usize,
    pub w: f64,
    pub ref_up: Option<usize>,
    pub ref_dn: Option<usize>,
    /// coplanar gaps measured from the sampled trace edge, on its +normal
    /// ("left") and -normal ("right") side
    pub s_left: Option<f64>,
    pub s_right: Option<f64>,
    /// diff partner: (gap edge-to-edge, partner width, partner on the left side)
    pub partner: Option<(f64, f64, bool)>,
    /// solder mask (thickness, er) if the trace is on an outer layer
    pub mask: Option<(f64, f64)>,
    /// plane edges and openings on the layers of the modelled stack
    pub planes: Vec<PlaneCut>,
}

struct Band {
    y0: f64,
    y1: f64,
}

/// Returns the geometry text, or None if the trace has no reference at all.
pub fn build(su: &Stackup, d: &XsDesc) -> Option<String> {
    let n = su.n();
    let li = d.layer;
    if d.ref_up.is_none() && d.ref_dn.is_none() {
        return None;
    }
    // natural coordinates: B.Cu bottom at y = 0, F.Cu on top
    let mut cu: Vec<Band> = (0..n).map(|_| Band { y0: 0.0, y1: 0.0 }).collect();
    let mut y = 0.0;
    for k in (0..n).rev() {
        cu[k].y0 = y;
        y += su.copper[k].thickness;
        cu[k].y1 = y;
        if k > 0 {
            y += su.dielectrics[k - 1].thickness;
        }
    }
    let total = y;

    // the trace must face up: mirror if the only/outer free side is below
    let flip = match (d.ref_up, d.ref_dn) {
        (Some(_), None) => true, // reference above only: free side is below
        _ => false,
    };
    let fy = |y0: f64, y1: f64| -> (f64, f64) {
        if flip {
            (total - y1, total - y0)
        } else {
            (y0, y1)
        }
    };

    // vertical extent of the modelled stack (copper indices, natural order)
    let k_top = match (d.ref_up, d.ref_dn) {
        (Some(j), _) => j,
        (None, Some(_)) => 0, // up to the board surface above the trace
        _ => unreachable!(),
    };
    let k_bot = match (d.ref_up, d.ref_dn) {
        (_, Some(j)) => j,
        (Some(_), None) => n - 1,
        _ => unreachable!(),
    };
    let has_up_plane = d.ref_up.is_some();
    let has_dn_plane = d.ref_dn.is_some();

    let mut out = String::new();
    let _ = writeln!(out, "units mm");
    let _ = writeln!(out, "bounds open open open open");

    // dielectric layers between k_top and k_bot
    for k in k_top..k_bot {
        let die = &su.dielectrics[k];
        let (y0, y1) = fy(cu[k + 1].y1, cu[k].y0);
        let _ = writeln!(out, "diel x=-inf w=inf y={} h={} er={}", f(y0), f(y1 - y0), f(die.epsilon_r));
    }
    // copper bands of all non-plane layers in range: resin fill
    for k in k_top..=k_bot {
        let is_plane = (has_up_plane && k == k_top) || (has_dn_plane && k == k_bot);
        if is_plane {
            continue;
        }
        let outer = k == 0 || k == n - 1;
        if outer && k == li {
            continue; // surface trace: air beside it (mask added below)
        }
        let er = fill_er(su, k);
        let (y0, y1) = fy(cu[k].y0, cu[k].y1);
        let _ = writeln!(out, "diel x=-inf w=inf y={} h={} er={}", f(y0), f(y1 - y0), f(er));
    }

    // trace placement (x): pair centred on 0
    let t = su.copper[li].thickness;
    let (ty0, _) = fy(cu[li].y0, cu[li].y1);
    // after a flip the trace band is still [ty0, ty0 + t]
    let mut traces: Vec<(f64, f64, &str)> = Vec::new(); // (x_left, width, kind)
    let (x_tr, x_partner): (f64, Option<(f64, f64)>);
    match d.partner {
        Some((g, wp, left)) => {
            let tot = d.w + g + wp;
            if left {
                x_partner = Some((-tot / 2.0, wp));
                x_tr = -tot / 2.0 + wp + g;
            } else {
                x_tr = -tot / 2.0;
                x_partner = Some((-tot / 2.0 + d.w + g, wp));
            }
        }
        None => {
            x_tr = -d.w / 2.0;
            x_partner = None;
        }
    }
    match x_partner {
        Some((xp, wp)) => {
            if xp < x_tr {
                traces.push((xp, wp, "sig-"));
                traces.push((x_tr, d.w, "sig+"));
            } else {
                traces.push((x_tr, d.w, "sig-"));
                traces.push((xp, wp, "sig+"));
            }
        }
        None => traces.push((x_tr, d.w, "sig+")),
    }

    // solder mask on surface traces
    let surface = li == 0 || li == n - 1;
    if surface {
        if let Some((tm, em)) = d.mask {
            let _ = writeln!(out, "diel x=-inf w=inf y={} h={} er={} thin=1", f(ty0), f(tm), f(em));
            for &(x, w, _) in &traces {
                let _ = writeln!(out, "diel x={} w={} y={} h={} er={}", f(x - tm), f(w + 2.0 * tm), f(ty0), f(t + tm), f(em));
            }
        }
    }

    // reference planes (finite where the plane ends near the trace) and
    // copper beside openings on the layers in between
    let xc = x_tr + d.w / 2.0; // sampled trace centre
    let cut = |k: usize| d.planes.iter().find(|c| c.layer == k);
    for j in [d.ref_up, d.ref_dn].into_iter().flatten() {
        let (y0, y1) = fy(cu[j].y0, cu[j].y1);
        let (yy, hh) = (f(y0), f(y1 - y0));
        match cut(j).map(|c| (c.left, c.right)).unwrap_or((None, None)) {
            (None, None) => {
                let _ = writeln!(out, "gnd x=-inf w=inf y={yy} h={hh}");
            }
            (Some(l), None) => {
                let _ = writeln!(out, "gnd x={} w=inf y={yy} h={hh}", f(xc - l));
            }
            (None, Some(r)) => {
                let _ = writeln!(out, "gnd x={} w=-inf y={yy} h={hh}", f(xc + r));
            }
            (Some(l), Some(r)) => {
                let _ = writeln!(out, "gnd x={} w={} y={yy} h={hh}", f(xc - l), f(l + r));
            }
        }
    }
    for c in &d.planes {
        if Some(c.layer) == d.ref_up || Some(c.layer) == d.ref_dn || c.layer == li {
            continue;
        }
        if c.layer < k_top || c.layer > k_bot {
            continue;
        }
        let (y0, y1) = fy(cu[c.layer].y0, cu[c.layer].y1);
        if let Some(l) = c.left {
            let _ = writeln!(out, "gnd x={} w=-inf y={} h={}", f(xc - l), f(y0), f(y1 - y0));
        }
        if let Some(r) = c.right {
            let _ = writeln!(out, "gnd x={} w=inf y={} h={}", f(xc + r), f(y0), f(y1 - y0));
        }
    }

    // coplanar ground on the trace layer. "left" = trace's +normal side = -x.
    let x_l_edge = traces.first().map(|t| t.0).unwrap();
    let x_r_edge = traces.last().map(|t| t.0 + t.1).unwrap();
    let side_gap = |s: Option<f64>, partner_here: bool| -> Option<f64> {
        let s = s?;
        if partner_here {
            let (g, wp, _) = d.partner.unwrap();
            let r = s - g - wp;
            (r > 0.005).then_some(r)
        } else {
            Some(s)
        }
    };
    let p_left = d.partner.map(|p| p.2);
    // gaps are measured from the sampled trace; translate to the pair's outer edges
    if let Some(s) = side_gap(d.s_left, p_left == Some(true)) {
        let _ = writeln!(out, "gnd x={} w=-inf y={} h={}", f(x_l_edge - s), f(ty0), f(t));
    }
    if let Some(s) = side_gap(d.s_right, p_left == Some(false)) {
        let _ = writeln!(out, "gnd x={} w=inf y={} h={}", f(x_r_edge + s), f(ty0), f(t));
    }

    for (x, w, kind) in traces {
        let _ = writeln!(out, "{} x={} w={} y={} h={}", kind, f(x), f(w), f(ty0), f(t));
    }
    Some(out)
}

/// Permittivity of the resin around copper on layer k: an adjacent prepreg if
/// there is one (it flows around the copper), else the dielectric above.
fn fill_er(su: &Stackup, k: usize) -> f64 {
    let above = if k > 0 { su.dielectrics.get(k - 1) } else { None };
    let below = su.dielectrics.get(k);
    let is_pp = |d: Option<&crate::board::Dielectric>| d.map(|d| d.kind.eq_ignore_ascii_case("prepreg")).unwrap_or(false);
    if is_pp(above) {
        above.unwrap().epsilon_r
    } else if is_pp(below) {
        below.unwrap().epsilon_r
    } else {
        above.or(below).map(|d| d.epsilon_r).unwrap_or(1.0)
    }
}

fn f(v: f64) -> String {
    let s = format!("{:.5}", v);
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.to_string() }
}

/// Stable 64-bit FNV-1a hash of the geometry text, as hex (cache key).
pub fn geom_id(text: &str) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in text.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::{CopperLayer, Dielectric, SolderMask, Stackup};

    fn ts() -> Stackup {
        let cu = |n: &str, t: f64| CopperLayer { name: n.into(), thickness: t };
        let di = |k: &str, h: f64, er: f64| Dielectric {
            name: "d".into(),
            kind: k.into(),
            material: "m".into(),
            thickness: h,
            epsilon_r: er,
            loss_tangent: 0.02,
        };
        Stackup {
            copper: vec![cu("F.Cu", 0.0432), cu("In1.Cu", 0.0175), cu("In2.Cu", 0.0175), cu("In3.Cu", 0.0175), cu("In4.Cu", 0.0175), cu("B.Cu", 0.0432)],
            dielectrics: vec![di("core", 0.1107, 3.23), di("prepreg", 0.1016, 3.66), di("core", 0.9256, 4.5), di("prepreg", 0.1107, 3.66), di("core", 0.1, 3.23)],
            mask_top: Some(SolderMask { thickness: 0.0152, epsilon_r: 3.3 }),
            mask_bottom: Some(SolderMask { thickness: 0.0152, epsilon_r: 3.3 }),
            from_file: true,
        }
    }

    #[test]
    fn microstrip_top() {
        let su = ts();
        let g = build(&su, &XsDesc { layer: 0, w: 0.213, ref_up: None, ref_dn: Some(1), s_left: None, s_right: None, partner: None, mask: Some((0.0152, 3.3)), planes: vec![] }).unwrap();
        // substrate 0.1107 thick directly under the trace, plane below it
        assert!(g.contains("h=0.1107 er=3.23"), "{g}");
        assert!(g.contains("sig+ x=-0.1065 w=0.213"), "{g}");
        assert!(g.contains("thin=1"));
        assert!(!g.contains("sig-"));
    }

    #[test]
    fn bottom_trace_is_flipped() {
        let su = ts();
        let g = build(&su, &XsDesc { layer: 5, w: 0.2, ref_up: Some(4), ref_dn: None, s_left: None, s_right: None, partner: None, mask: Some((0.0152, 3.3)), planes: vec![] }).unwrap();
        // the plane must be below the trace after the flip
        let y_of = |prefix: &str| -> f64 {
            let line = g.lines().find(|l| l.starts_with(prefix)).unwrap();
            line.split_whitespace().find(|t| t.starts_with("y=")).unwrap()[2..].parse().unwrap()
        };
        assert!(y_of("gnd") < y_of("sig+"), "{g}");
    }

    #[test]
    fn stripline_inner_pair() {
        let su = ts();
        let g = build(&su, &XsDesc { layer: 3, w: 0.127, ref_up: Some(2), ref_dn: Some(4), s_left: None, s_right: None, partner: Some((0.23, 0.127, true)), mask: None, planes: vec![] }).unwrap();
        assert_eq!(g.matches("gnd x=-inf").count(), 2, "{g}");
        assert!(g.contains("sig-") && g.contains("sig+"));
        // In3 copper band filled with the prepreg below (3.66)
        assert!(g.contains("h=0.0175 er=3.66"), "{g}");
    }

    #[test]
    fn antipad_under_pad() {
        // trace on F.Cu, In1 cut out 0.4/0.5 mm around it, reference In2
        let su = ts();
        let g = build(&su, &XsDesc {
            layer: 0, w: 0.5, ref_up: None, ref_dn: Some(2), s_left: None, s_right: None, partner: None, mask: None,
            planes: vec![PlaneCut { layer: 1, left: Some(0.4), right: Some(0.5) }, PlaneCut { layer: 2, left: None, right: None }],
        }).unwrap();
        assert!(g.contains("gnd x=-0.4 w=-inf"), "{g}");
        assert!(g.contains("gnd x=0.5 w=inf"), "{g}");
        assert!(g.contains("gnd x=-inf w=inf"), "{g}");
    }

    #[test]
    fn plane_edge() {
        let su = ts();
        let g = build(&su, &XsDesc {
            layer: 0, w: 0.2, ref_up: None, ref_dn: Some(1), s_left: None, s_right: None, partner: None, mask: None,
            planes: vec![PlaneCut { layer: 1, left: None, right: Some(0.3) }],
        }).unwrap();
        // plane continues to the left, ends 0.3 mm right of the trace centre
        assert!(g.contains("gnd x=0.3 w=-inf"), "{g}");
    }

    #[test]
    fn coplanar_ground_beyond_partner() {
        let su = ts();
        let g = build(&su, &XsDesc { layer: 0, w: 0.1, ref_up: None, ref_dn: Some(1), s_left: Some(0.5), s_right: Some(0.2), partner: Some((0.1, 0.1, true)), mask: None, planes: vec![] }).unwrap();
        // left: ground 0.5 from sampled trace = 0.3 beyond partner's outer edge
        assert!(g.contains("w=-inf"), "{g}");
        assert!(g.contains("gnd x=-0.45 w=-inf"), "{g}");
        assert!(g.contains("gnd x=0.35 w=inf"), "{g}");
    }
}

//! Where a result applies on the board: one `Spot` – layer(s), coordinates, net and the part, pad,
//! via or track – for the findings of `kisi report` (`kisi_core::findings`), the location of a
//! channel row's worst criterion (`kisi check`, the report) and the viewer's jump to it (WASM
//! `kisi_locate`). Coordinates are KiCad board coordinates in mm as KiCad shows them (x to the right,
//! y down, the page origin); layers by their KiCad names.

use crate::board::{Board, Pad};
use crate::extract::{compared_z, Analysis, Target};
use crate::geom::{self, P};
use crate::profile::{Criterion, Status};
use serde::{Deserialize, Serialize};

/// A place on the board.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Spot {
    /// the point (mm, KiCad board coordinates)
    pub at: P,
    /// the copper layer it applies to ("" when it applies to several or none)
    pub layer: String,
    /// every copper layer of the item (a via: the layers it joins; a net: the layers it runs on)
    pub layers: Vec<String>,
    /// the net, as in the board file ("" for none)
    pub net: String,
    /// "track", "via", "pad", "part" or "net"
    pub item: String,
    /// parts and pads: `U3`, `U3.12`
    pub refs: Vec<String>,
    /// the item in one line, e.g. "track 0.127 mm on In1.Cu from (12.30, 45.10) to (15.00, 45.10) mm"
    pub what: String,
    /// the area to show (x0, y0, x1, y1 in mm): the item with its surroundings
    pub bbox: [f64; 4],
}

/// Coordinates rounded to 0.01 mm for files (`+ 0.0`: no negative zero).
#[inline]
pub fn r2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0 + 0.0
}

fn pt(p: P) -> String {
    format!("({:.2}, {:.2})", p[0], p[1])
}

fn lname(b: &Board, l: u16) -> String {
    b.stackup().copper.get(l as usize).map(|c| c.name.clone()).unwrap_or_default()
}

fn bbox_of(pts: impl IntoIterator<Item = P>, margin: f64) -> [f64; 4] {
    let mut bb = geom::BBox::empty();
    for p in pts {
        bb.add(p);
    }
    if !bb.is_valid() {
        return [0.0; 4];
    }
    [r2(bb.min[0] - margin), r2(bb.min[1] - margin), r2(bb.max[0] + margin), r2(bb.max[1] + margin)]
}

fn finish(mut s: Spot) -> Spot {
    s.at = [r2(s.at[0]), r2(s.at[1])];
    s
}

/// The layers of `nets`' tracks, top to bottom.
fn net_layers(b: &Board, nets: &[u32]) -> Vec<String> {
    let mut ls: Vec<u16> = b.tracks.iter().filter(|t| nets.contains(&t.net)).map(|t| t.layer).collect();
    for v in b.vias.iter().filter(|v| nets.contains(&v.net)) {
        ls.extend([v.from, v.to]);
    }
    ls.sort_unstable();
    ls.dedup();
    ls.into_iter().map(|l| lname(b, l)).collect()
}

/// A pad's name as people write it: `U3.12` (the part's reference, the pad number).
pub fn pad_ref(b: &Board, p: &Pad) -> String {
    let r = b.footprints.get(p.fp as usize).map(|f| f.reference.as_str()).unwrap_or("?");
    if p.number.is_empty() {
        r.to_string()
    } else {
        format!("{r}.{}", p.number)
    }
}

/// The track of `nets` whose impedance (`extract::compared_z`, as the net statistics count it) is
/// furthest from `target`. `mean`: the track that moves the nets' mean impedance most towards the side
/// it is on – its mean deviation times its length (the impedance criterion; a track through an SMD pad
/// stands for the pad); else the largest deviation of a piece (the largest discontinuity: return
/// loss). Tracks shorter than 0.3 mm only when no other has an impedance. None without impedances.
pub fn worst_track(b: &Board, an: &Analysis, nets: &[u32], target: Option<Target>, mean: bool) -> Option<Spot> {
    let t = target?;
    // per track: (track, length with an impedance, Σ z·length, the middle of the piece with the largest
    // deviation, that deviation, all pieces from the field solver)
    let mut per: Vec<(usize, f64, f64, f64, f64, bool)> = Vec::new();
    for pc in an.pieces.as_chunks::<4>().0 {
        let ti = pc[0] as usize;
        let Some(tr) = b.tracks.get(ti) else { continue };
        if !nets.contains(&tr.net) {
            continue;
        }
        let Some(xs) = an.cross_sections.get(pc[3] as usize) else { continue };
        // a track inside an SMD pad: the pad's own synthetic track stands for it (as the statistics)
        let counted = tr.pad || xs.flags & crate::extract::F_PAD == 0;
        let Some((z, _)) = compared_z(xs, Some(t)).filter(|_| counted) else { continue };
        let l = geom::len(geom::sub(tr.b, tr.a)) * (pc[2] - pc[1]);
        let dev = (z - t.z).abs();
        let field = xs.solver == "field";
        match per.iter_mut().find(|x| x.0 == ti) {
            Some(x) => {
                x.1 += l;
                x.2 += z * l;
                x.5 &= field;
                if dev > x.4 {
                    x.3 = 0.5 * (pc[1] + pc[2]);
                    x.4 = dev;
                }
            }
            None => per.push((ti, l, z * l, 0.5 * (pc[1] + pc[2]), dev, field)),
        }
    }
    type Tr = (usize, f64, f64, f64, f64, bool);
    // the side the nets' mean is on (above or below the target): a track counts by how far it pulls that way
    let (l_all, zl_all) = per.iter().fold((0.0, 0.0), |a, x| (a.0 + x.1, a.1 + x.2));
    let side = if zl_all / l_all.max(1e-12) >= t.z { 1.0 } else { -1.0 };
    let score = |x: &Tr| if mean { side * (x.2 / x.1.max(1e-12) - t.z) * x.1 } else { x.4 };
    let long = per.iter().any(|x| x.1 >= 0.3);
    let best = per.iter().filter(|x| x.1 > 0.0 && (!long || x.1 >= 0.3)).fold(None::<&Tr>, |a, x| match a {
        Some(a) if score(a) >= score(x) => Some(a),
        _ => Some(x),
    })?;
    let tr = &b.tracks[best.0];
    let z = best.2 / best.1;
    let layer = lname(b, tr.layer);
    let at = geom::lerp(tr.a, tr.b, best.3);
    let kind = if tr.pad {
        // the synthetic track through an SMD pad: name the pad
        let pad = b.pads.iter().find(|p| {
            p.net == tr.net
                && p.layers.contains(&tr.layer)
                && geom::len(geom::sub(p.at, at)) <= 0.5 * p.size[0].max(p.size[1])
        });
        format!("pad {}", pad.map(|p| pad_ref(b, p)).unwrap_or_default())
    } else if tr.is_arc {
        "arc".into()
    } else {
        "track".into()
    };
    let what = format!(
        "{kind} {:.3} mm wide on {layer} from {} to {} mm, Z {:.1} Ω{} against {:.0} Ω{}",
        tr.width,
        pt(tr.a),
        pt(tr.b),
        z,
        if best.5 { "" } else { " (formula, estimate)" },
        t.z,
        if t.diff { " (differential)" } else { "" }
    );
    let refs: Vec<String> = if tr.pad {
        b.pads
            .iter()
            .filter(|p| {
                p.net == tr.net
                    && p.layers.contains(&tr.layer)
                    && geom::len(geom::sub(p.at, at)) <= 0.5 * p.size[0].max(p.size[1])
            })
            .take(1)
            .map(|p| pad_ref(b, p))
            .collect()
    } else {
        vec![]
    };
    Some(finish(Spot {
        at,
        layers: vec![layer.clone()],
        layer,
        net: b.net_name(tr.net).to_string(),
        item: if tr.pad { "pad" } else { "track" }.into(),
        refs,
        what,
        bbox: bbox_of([tr.a, tr.b], 1.5),
    }))
}

/// A pad by part reference and pad number.
pub fn pad_spot(b: &Board, refdes: &str, pin: &str) -> Option<Spot> {
    let p = b
        .pads
        .iter()
        .find(|p| p.number == pin && b.footprints.get(p.fp as usize).is_some_and(|f| f.reference == refdes))?;
    let layers: Vec<String> = p.layers.iter().map(|&l| lname(b, l)).collect();
    let r = pad_ref(b, p);
    Some(finish(Spot {
        at: p.at,
        layer: if layers.len() == 1 { layers[0].clone() } else { String::new() },
        what: format!(
            "pad {r} ({}) at {} mm on {}",
            if p.drill > 0.0 { "through hole" } else { "SMD" },
            pt(p.at),
            if layers.len() > 2 { "all layers".to_string() } else { layers.join(", ") }
        ),
        layers,
        net: b.net_name(p.net).to_string(),
        item: "pad".into(),
        refs: vec![r],
        bbox: bbox_of([p.at], 3.0),
    }))
}

/// A part: its pads (on `net` when given, else all), the point at their centre.
pub fn part_spot(b: &Board, refdes: &str, net: Option<&str>) -> Option<Spot> {
    let fi = b.footprints.iter().position(|f| f.reference == refdes)?;
    let f = &b.footprints[fi];
    let all: Vec<&Pad> = b.pads.iter().filter(|p| p.fp as usize == fi).collect();
    let on: Vec<&Pad> = match net {
        Some(n) => all.iter().copied().filter(|p| b.net_name(p.net) == n).collect(),
        None => vec![],
    };
    let pads = if on.is_empty() { &all } else { &on };
    let at = if pads.is_empty() {
        f.at
    } else {
        let n = pads.len() as f64;
        [pads.iter().map(|p| p.at[0]).sum::<f64>() / n, pads.iter().map(|p| p.at[1]).sum::<f64>() / n]
    };
    // the side: the layers of its SMD pads, else every layer of its pads
    let mut ls: Vec<u16> = all.iter().filter(|p| p.drill <= 0.0).flat_map(|p| p.layers.iter().copied()).collect();
    if ls.is_empty() {
        ls = all.iter().flat_map(|p| p.layers.iter().copied()).collect();
    }
    ls.sort_unstable();
    ls.dedup();
    let layers: Vec<String> = ls.iter().map(|&l| lname(b, l)).collect();
    let side = match layers.as_slice() {
        [one] => one.clone(),
        _ => String::new(),
    };
    let net_txt = match (net, on.is_empty()) {
        (Some(n), false) => format!(", its {} pad{} on {n}", on.len(), if on.len() == 1 { "" } else { "s" }),
        _ => String::new(),
    };
    Some(finish(Spot {
        at,
        what: format!(
            "part {refdes} {} at {} mm{}{net_txt}",
            f.part_label(),
            pt(at),
            if side.is_empty() { String::new() } else { format!(" on {side}") }
        ),
        layer: side,
        layers,
        net: if on.is_empty() { String::new() } else { net.unwrap_or("").to_string() },
        item: "part".into(),
        refs: if on.len() == 1 { vec![pad_ref(b, on[0])] } else { vec![refdes.to_string()] },
        bbox: bbox_of(all.iter().map(|p| p.at).chain([f.at]), 3.0),
    }))
}

/// A via (or plated hole) at `at` joining copper layers `from`–`to`; the board's via there names its
/// net and size.
pub fn via_spot(b: &Board, at: P, from: &str, to: &str, drill: f64, plated_hole: bool) -> Spot {
    let near = |p: P| geom::len(geom::sub(p, at)) < 0.05;
    let v = b.vias.iter().find(|v| near(v.at));
    let pad = if plated_hole { b.pads.iter().find(|p| p.drill > 0.0 && near(p.at)) } else { None };
    let (fi, ti) = {
        let su = b.stackup();
        (su.layer_index(from).unwrap_or(0), su.layer_index(to).unwrap_or(0))
    };
    let (lo, hi) = (fi.min(ti), fi.max(ti));
    let layers: Vec<String> = (lo..=hi).map(|l| lname(b, l as u16)).collect();
    let net = v.map(|v| v.net).or(pad.map(|p| p.net)).map(|n| b.net_name(n).to_string()).unwrap_or_default();
    let what = match (pad, v) {
        (Some(p), _) => format!("plated hole of pad {} ({drill:.2} mm drill) at {} mm", pad_ref(b, p), pt(at)),
        (None, Some(v)) => {
            format!(
                "via {:.2}/{:.2} mm (drill/pad) at {} mm, {} – {}",
                v.drill,
                v.size,
                pt(at),
                lname(b, v.from),
                lname(b, v.to)
            )
        }
        _ => format!("via {drill:.2} mm drill at {} mm, {from} – {to}", pt(at)),
    };
    finish(Spot {
        at,
        layer: String::new(),
        layers,
        net,
        item: "via".into(),
        refs: pad.map(|p| vec![pad_ref(b, p)]).unwrap_or_default(),
        what,
        bbox: bbox_of([at], 3.0),
    })
}

/// The route of `nets`: every layer it runs on, the point in the middle of its longest track.
pub fn route_spot(b: &Board, nets: &[u32]) -> Option<Spot> {
    let tr = b
        .tracks
        .iter()
        .filter(|t| nets.contains(&t.net) && !t.pad)
        .max_by(|x, y| geom::len(geom::sub(x.b, x.a)).total_cmp(&geom::len(geom::sub(y.b, y.a))))?;
    let pts = b.tracks.iter().filter(|t| nets.contains(&t.net)).flat_map(|t| [t.a, t.b]);
    let name: Vec<&str> = nets.iter().map(|&n| b.net_name(n)).collect();
    Some(finish(Spot {
        at: geom::lerp(tr.a, tr.b, 0.5),
        layer: String::new(),
        layers: net_layers(b, nets),
        net: name.first().map(|s| s.to_string()).unwrap_or_default(),
        item: "net".into(),
        refs: vec![],
        what: format!("the route of {}", name.join(", ")),
        bbox: bbox_of(pts, 2.0),
    }))
}

/// The worst criterion of a verdict: the worst status, then the least margin relative to its limit
/// (an error first). The same order as the report's tables.
pub fn worst_criterion(cs: &[Criterion]) -> Option<&Criterion> {
    let rel = |c: &Criterion| {
        let v = c.value?;
        let lo = c.lo.map(|l| (v - l) / l.abs().max(1e-9));
        let hi = c.hi.map(|h| (h - v) / h.abs().max(1e-9));
        match (lo, hi) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    };
    let key = |c: &Criterion| (c.status, if c.key == "error" { -1.0 } else { rel(c).unwrap_or(f64::INFINITY) });
    let mut best: Option<(&Criterion, (Status, f64))> = None;
    for c in cs {
        let k = key(c);
        if best.as_ref().is_none_or(|b| k.0 < b.1 .0 || (k.0 == b.1 .0 && k.1 < b.1 .1)) {
            best = Some((c, k));
        }
    }
    best.map(|b| b.0)
}

/// Where a channel row's criterion `key` applies: the impedance (`z`) at the track that moves the mean
/// most, the return loss (`rl`) at the largest deviation from the target, a via stub (`stub`) at the via with the lowest stub resonance (`stub_at`, the
/// check row's), everything measured at the receiver (eye, loss, return loss, skew, an error) at the
/// receiver's pad (`rx`: reference, pad) – else the route of the row's nets.
pub fn channel_spot(
    b: &Board,
    an: &Analysis,
    nets: &[u32],
    target: Option<Target>,
    key: &str,
    rx: Option<(&str, &str)>,
    stub_at: Option<P>,
) -> Option<Spot> {
    let route = route_spot(b, nets);
    let spot = match key {
        "z" => worst_track(b, an, nets, target, true),
        "rl" => worst_track(b, an, nets, target, false),
        "stub" => stub_at.and_then(|at| {
            let v = b.vias.iter().find(|v| nets.contains(&v.net) && geom::len(geom::sub(v.at, at)) < 0.05)?;
            Some(via_spot(b, v.at, &lname(b, v.from), &lname(b, v.to), v.drill, false))
        }),
        _ => rx.and_then(|(r, p)| pad_spot(b, r, p)),
    };
    let mut s = spot.or_else(|| route.clone())?;
    if let (Some(r), true) = (&route, s.net.is_empty()) {
        s.net = r.net.clone();
    }
    Some(s)
}

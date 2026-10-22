//! Board → kisi-fdtd case: the 3D check of one via (or the via pair of a differential pair)
//! cut out of the layout.
//!
//! ```text
//! kisi-validate via-case <board.kicad_pcb> --net NAME --out DIR
//!     [--at X,Y | --via K] [--layers IN,OUT] [--pro F] [--config F | --no-config]
//!     [--keep 1.2] [--keep-cell 0.05] [--rotate DEG] [--feed 4.0] [--margin 2.5] [--air 1.0]
//!     [--fmax 40e9] [--dfine 0.035] [--name ID] [--list] [--scan]
//! ```
//!
//! Writes `DIR/<name>.json` (a kisi-fdtd case with a `meta` block like
//! validation/fdtd/via_cases.py) and the 2×thru fixture of each side, `DIR/thru_<name>_in.json`
//! and `DIR/thru_<name>_out.json`.
//!
//! What the case holds (coordinates in mm, origin at the via or the middle of the pair, y up):
//! * the stackup with kisi's copper heights (dielectrics plus half the copper on each side, as
//!   the ladder `kisi_si::via`), zero-thickness copper, air above and below;
//! * the signal via (and the partner via of a pair): barrel, annular rings by KiCad's rules
//!   (every layer, or only the connected ones with `remove_unused_layers`), back-drilling;
//! * on every layer without a feed trace: the zone fills of all other nets as polygons (their
//!   real outlines, so the antipads are the real openings), pads and other tracks, within the
//!   window;
//! * on the two feed layers: the same inside the keep box (±keep around the origin), the own
//!   trace(s) as routed up to the box edge, then a straight trace (or pair) along the nearest
//!   axis to a 50 Ω lumped port `feed` mm from the origin; nothing else outside the box, so the
//!   fixture is a clean line;
//! * every other via in the window: return vias (plane nets) and other nets' vias as barrels;
//!   plated holes of pads as barrels.
//!
//! The 2×thru of each side: the straight trace (or pair) of the feed, 2·feed long, with solid
//! sheets on the layers that have copper at the port in the case.

use kisi_core::board::{pad_polygon, Board, Via};
use kisi_core::geom::{self, P};
use kisi_core::service::Project;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

fn die(msg: &str) -> ! {
    eprintln!("via-case: {msg}");
    std::process::exit(2);
}

struct Opt {
    pcb: PathBuf,
    pro: Option<PathBuf>,
    config: Option<PathBuf>,
    no_config: bool,
    net: String,
    at: Option<P>,
    via_k: Option<usize>,
    layers: Option<(String, String)>,
    keep: f64,
    keep_cell: f64,
    /// rotation of the cut-out (degrees, counter-clockwise); None: 0° or 45°, whichever puts
    /// the exit directions of the feeds closer to the axes
    rotate: Option<f64>,
    feed: f64,
    margin: f64,
    air: f64,
    fmax: f64,
    dfine: f64,
    name: Option<String>,
    out: Option<PathBuf>,
    list: bool,
    scan: bool,
    /// diagnostics: the real copper of every layer in the whole window (own tracks included,
    /// no extensions); the case is not meant to be run
    preview: bool,
}

fn parse(args: &[String]) -> Opt {
    let mut o = Opt {
        pcb: PathBuf::new(),
        pro: None,
        config: None,
        no_config: false,
        net: String::new(),
        at: None,
        via_k: None,
        layers: None,
        keep: 1.2,
        keep_cell: 0.05,
        rotate: None,
        feed: 4.0,
        margin: 2.5,
        air: 1.0,
        fmax: 40e9,
        dfine: 0.035,
        name: None,
        out: None,
        list: false,
        scan: false,
        preview: false,
    };
    let mut i = 0;
    let num = |s: &str| -> f64 { s.parse().unwrap_or_else(|_| die(&format!("not a number: {s}"))) };
    while i < args.len() {
        let a = args[i].as_str();
        let val = || args.get(i + 1).cloned().unwrap_or_else(|| die(&format!("{a} needs a value")));
        match a {
            "--net" => o.net = val(),
            "--pro" => o.pro = Some(val().into()),
            "--config" => o.config = Some(val().into()),
            "--no-config" => {
                o.no_config = true;
                i += 1;
                continue;
            }
            "--list" => {
                o.list = true;
                i += 1;
                continue;
            }
            "--scan" => {
                o.scan = true;
                i += 1;
                continue;
            }
            "--preview" => {
                o.preview = true;
                i += 1;
                continue;
            }
            "--at" => {
                let v: Vec<f64> = val().split(',').map(num).collect();
                o.at = Some([v[0], v[1]]);
            }
            "--via" => o.via_k = Some(num(&val()) as usize),
            "--layers" => {
                let v = val();
                let (a, b) = v.split_once(',').unwrap_or_else(|| die("--layers IN,OUT"));
                o.layers = Some((a.to_string(), b.to_string()));
            }
            "--keep" => o.keep = num(&val()),
            "--keep-cell" => o.keep_cell = num(&val()),
            "--rotate" => o.rotate = Some(num(&val())),
            "--feed" => o.feed = num(&val()),
            "--margin" => o.margin = num(&val()),
            "--air" => o.air = num(&val()),
            "--fmax" => o.fmax = num(&val()),
            "--dfine" => o.dfine = num(&val()),
            "--name" => o.name = Some(val()),
            "--out" => o.out = Some(val().into()),
            _ if o.pcb.as_os_str().is_empty() && !a.starts_with("--") => {
                o.pcb = a.into();
                i += 1;
                continue;
            }
            _ => die(&format!("unknown argument {a}")),
        }
        i += 2;
    }
    if o.pcb.as_os_str().is_empty() || (o.net.is_empty() && !o.scan) {
        die("usage: kisi-validate via-case <board.kicad_pcb> --net NAME --out DIR [options], see viacase.rs");
    }
    o
}

/// z of every copper layer (mm, bottom copper at 0): dielectrics plus half the copper thickness
/// on both sides – the heights kisi's ladder (`kisi_si::via`) uses.
fn copper_z(b: &Board) -> Vec<f64> {
    let su = b.stackup();
    let n = su.n();
    let mut z = vec![0.0; n];
    for k in (0..n.saturating_sub(1)).rev() {
        z[k] = z[k + 1] + su.dielectrics[k].thickness + 0.5 * (su.copper[k].thickness + su.copper[k + 1].thickness);
    }
    z
}

fn r6(x: f64) -> f64 {
    (x * 1e6).round() / 1e6
}

/// Sutherland–Hodgman clip of a ring (even-odd copper, also fractured outlines) to a rectangle.
fn clip_ring(ring: &[P], r: [f64; 4]) -> Vec<P> {
    let mut out = ring.to_vec();
    for edge in 0..4 {
        let inp = std::mem::take(&mut out);
        if inp.is_empty() {
            break;
        }
        let inside = |p: &P| match edge {
            0 => p[0] >= r[0],
            1 => p[0] <= r[2],
            2 => p[1] >= r[1],
            _ => p[1] <= r[3],
        };
        let isect = |a: P, b: P| -> P {
            let t = match edge {
                0 => (r[0] - a[0]) / (b[0] - a[0]),
                1 => (r[2] - a[0]) / (b[0] - a[0]),
                2 => (r[1] - a[1]) / (b[1] - a[1]),
                _ => (r[3] - a[1]) / (b[1] - a[1]),
            };
            [a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1])]
        };
        let n = inp.len();
        for i in 0..n {
            let (prev, cur) = (inp[(i + n - 1) % n], inp[i]);
            match (inside(&prev), inside(&cur)) {
                (true, true) => out.push(cur),
                (true, false) => out.push(isect(prev, cur)),
                (false, true) => {
                    out.push(isect(prev, cur));
                    out.push(cur);
                }
                _ => {}
            }
        }
    }
    out
}

fn area(ring: &[P]) -> f64 {
    let n = ring.len();
    (0..n).map(|i| geom::cross(ring[i], ring[(i + 1) % n])).sum::<f64>() / 2.0
}

fn bbox(ring: &[P]) -> [f64; 4] {
    let mut b = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
    for p in ring {
        b = [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])];
    }
    b
}

fn inside_ring(ring: &[P], p: P) -> bool {
    let n = ring.len();
    let mut odd = false;
    for i in 0..n {
        let (a, b) = (ring[i], ring[(i + 1) % n]);
        if (a[1] > p[1]) != (b[1] > p[1]) && a[0] + (p[1] - a[1]) * (b[0] - a[0]) / (b[1] - a[1]) > p[0] {
            odd = !odd;
        }
    }
    odd
}

/// net of the copper (polygon or rectangle sheet, not the signal) at q in the plane z
fn fill_net_at_any(sheets: &[Value], z: f64, q: P) -> Option<String> {
    sheets.iter().find_map(|s| {
        if (s["z"].as_f64()? - z).abs() > 1e-9 || s["net"].as_str() == Some("SIG") {
            return None;
        }
        let r: Vec<f64> = s["rect"].as_array()?.iter().filter_map(|x| x.as_f64()).collect();
        if q[0] < r[0] || q[0] > r[2] || q[1] < r[1] || q[1] > r[3] {
            return None;
        }
        let inside = match s["polys"].as_array() {
            None => true,
            Some(ps) => ps.iter().filter(|ring| inside_ring(&ring.as_array().unwrap().iter().map(|pt| [pt[0].as_f64().unwrap(), pt[1].as_f64().unwrap()]).collect::<Vec<P>>(), q)).count() % 2 == 1,
        };
        inside.then(|| s["net"].as_str().unwrap_or("").to_string())
    })
}

/// Liang–Barsky: the part of a→b inside the rectangle
fn clip_seg(a: P, b: P, r: [f64; 4]) -> Option<(P, P)> {
    let d = geom::sub(b, a);
    let (mut t0, mut t1) = (0.0f64, 1.0f64);
    for (p, q) in [(-d[0], a[0] - r[0]), (d[0], r[2] - a[0]), (-d[1], a[1] - r[1]), (d[1], r[3] - a[1])] {
        if p.abs() < 1e-15 {
            if q < 0.0 {
                return None;
            }
        } else {
            let t = q / p;
            if p < 0.0 {
                t0 = t0.max(t);
            } else {
                t1 = t1.min(t);
            }
        }
    }
    (t0 < t1 - 1e-9).then(|| (geom::lerp(a, b, t0), geom::lerp(a, b, t1)))
}

/// Outline of a trace segment with round ends (KiCad's track shape).
fn capsule(a: P, b: P, w: f64) -> Vec<P> {
    let d = geom::sub(b, a);
    let th = d[1].atan2(d[0]);
    let r = w / 2.0;
    let mut v = Vec::new();
    for (c, a0) in [(b, th - std::f64::consts::FRAC_PI_2), (a, th + std::f64::consts::FRAC_PI_2)] {
        for k in 0..=8 {
            let t = a0 + std::f64::consts::PI * k as f64 / 8.0;
            v.push([r6(c[0] + r * t.cos()), r6(c[1] + r * t.sin())]);
        }
    }
    v
}

/// Ordered segments (a, b, width) of the trace of `net` that leaves the via at `at` on `layer`,
/// starting at the via; None if no track of the net ends at the via on this layer.
fn chain(b: &Board, net: u32, at: P, r: f64, layer: u16) -> Option<Vec<(P, P, f64)>> {
    let near = |p: P, q: P, tol: f64| geom::len(geom::sub(p, q)) < tol;
    let mut used = vec![false; b.tracks.len()];
    // the track whose end is nearest to the via centre (a short piece into the via may be
    // followed by one that ends inside the pad as well)
    let dist = |i: usize| {
        let t = &b.tracks[i];
        geom::len(geom::sub(t.a, at)).min(geom::len(geom::sub(t.b, at)))
    };
    let mut starts: Vec<usize> = (0..b.tracks.len())
        .filter(|&i| {
            let t = &b.tracks[i];
            t.net == net && t.layer == layer && !t.pad && dist(i) < r && geom::len(geom::sub(t.b, t.a)) > 1e-6
        })
        .collect();
    starts.sort_by(|&x, &y| dist(x).total_cmp(&dist(y)));
    let &s0 = starts.first()?;
    if starts.len() > 1 && (dist(starts[1]) - dist(s0)).abs() < 1e-3 {
        eprintln!("via-case: {} tracks of {} leave the via on layer {layer}; following one", starts.len(), b.net_name(net));
    }
    let t = &b.tracks[s0];
    used[s0] = true;
    let mut segs = vec![if geom::len(geom::sub(t.a, at)) <= geom::len(geom::sub(t.b, at)) { (t.a, t.b, t.width) } else { (t.b, t.a, t.width) }];
    let mut total = geom::len(geom::sub(t.b, t.a));
    while total < 40.0 {
        let end = segs.last().unwrap().1;
        let nx = (0..b.tracks.len()).find(|&i| {
            let t = &b.tracks[i];
            !used[i] && t.net == net && t.layer == layer && !t.pad && (near(t.a, end, 1e-3) || near(t.b, end, 1e-3))
        });
        let Some(i) = nx else { break };
        used[i] = true;
        let t = &b.tracks[i];
        segs.push(if near(t.a, end, 1e-3) { (t.a, t.b, t.width) } else { (t.b, t.a, t.width) });
        total += geom::len(geom::sub(t.b, t.a));
    }
    if std::env::var("KISI_DEBUG_VIACASE").is_ok() {
        eprintln!("chain {} layer {layer}: {:?}", b.net_name(net), segs.iter().map(|s| (s.0, s.1)).collect::<Vec<_>>());
    }
    Some(segs)
}

/// Barrel range (top, bottom layer) of a via without its back-drilled part.
fn barrel(v: &Via) -> (usize, usize) {
    let (mut top, mut bot) = (v.from as usize, v.to as usize);
    if let Some((b0, b1)) = v.backdrill {
        let (b0, b1) = (b0 as usize, b1 as usize);
        if b1 >= bot && b0 > top {
            bot = b0 - 1;
        } else if b0 <= top && b1 < bot {
            top = b1 + 1;
        }
    }
    (top, bot)
}

/// One feed side: the routed trace(s) inside the keep box and the straight extension.
struct Side {
    layer: u16,
    /// axis (0 = x, 1 = y) and direction (±1) of the extension
    axis: usize,
    sign: f64,
    /// per via (signal, partner): kept segments in case coordinates, width, lateral position
    /// of the extension
    kept: Vec<(Vec<(P, P)>, f64, f64)>,
    /// reference layer of the feed
    ref_layer: u16,
}

pub fn run(args: &[String]) {
    let o = parse(args);
    let cfg = if o.no_config { None } else { o.config.clone().or_else(|| Some(kisi_core::service::config_path_for(&o.pcb)).filter(|p| p.is_file())) };
    let p = Project::load_with(&o.pcb, o.pro.as_deref(), None, cfg.as_deref()).unwrap_or_else(|e| die(&e));
    let b = &p.board;
    let su = b.stackup();
    let nl = su.n();
    let z = copper_z(b);
    let lname = |k: u16| su.copper[k as usize].name.clone();
    if o.scan {
        // every via of a signal net with the layers its tracks join: net class, net, position
        for v in &b.vias {
            if v.net == 0 || b.plane_net.get(v.net as usize).copied().unwrap_or(false) {
                continue;
            }
            let conn: Vec<String> = (v.from..=v.to).filter(|&l| chain(b, v.net, v.at, v.size / 2.0 + 1e-3, l).is_some()).map(lname).collect();
            println!("{}\t{}\t{:.4},{:.4}\t{}\t{:.3}/{:.3}", b.class_of(v.net).name, b.net_name(v.net), v.at[0], v.at[1], conn.join(">"), v.drill, v.size);
        }
        return;
    }
    let net = p.net_id(&o.net).unwrap_or_else(|| die(&format!("net {} not found", o.net)));
    let partner = kisi_core::extract::diff_partners(&b.nets).get(net as usize).copied().flatten();
    let vias: Vec<&Via> = b.vias.iter().filter(|v| v.net == net).collect();
    if vias.is_empty() {
        die("the net has no via");
    }
    if o.list {
        for (k, v) in vias.iter().enumerate() {
            println!("{k}: at ({:.4}, {:.4}) {}–{} drill {:.3} size {:.3}", v.at[0], v.at[1], lname(v.from), lname(v.to), v.drill, v.size);
        }
        return;
    }
    let v: &Via = match (o.at, o.via_k) {
        (Some(at), _) => vias.iter().min_by(|a, c| geom::len(geom::sub(a.at, at)).total_cmp(&geom::len(geom::sub(c.at, at)))).unwrap(),
        (None, Some(k)) => vias.get(k).unwrap_or_else(|| die("--via index out of range")),
        (None, None) => {
            if vias.len() > 1 {
                eprintln!("via-case: the net has {} vias, taking the first (--list, --via K, --at X,Y)", vias.len());
            }
            vias[0]
        }
    };
    // partner via: as kisi's via model (nearest via of the partner net within 2 mm)
    let pv: Option<&Via> = partner.and_then(|pn| {
        b.vias.iter().filter(|w| w.net == pn).map(|w| (geom::len(geom::sub(w.at, v.at)), w)).filter(|&(d, _)| d < 2.0 && d > v.drill).min_by(|x, y| x.0.total_cmp(&y.0)).map(|x| x.1)
    });
    let sig: Vec<(&Via, u32)> = std::iter::once((v, net)).chain(pv.map(|w| (w, partner.unwrap()))).collect();
    let c0: P = if let Some(w) = pv { geom::scale(geom::add(v.at, w.at), 0.5) } else { v.at };
    // case frame before the rotation: origin c0, y up
    let tc0 = |q: P| -> P { [q[0] - c0[0], -(q[1] - c0[1])] };
    let own = |n: u32| n == net || Some(n) == partner;

    // connected layers and feed sides
    let conn: Vec<u16> = (v.from..=v.to).filter(|&l| chain(b, net, v.at, v.size / 2.0 + 1e-3, l).is_some()).collect();
    let (l_in, l_out) = match &o.layers {
        Some((a, c)) => {
            let f = |s: &str| su.layer_index(s).map(|i| i as u16).unwrap_or_else(|| die(&format!("unknown layer {s}")));
            (f(a), f(c))
        }
        None => {
            if conn.len() != 2 {
                die(&format!("the via connects tracks on {} layers {:?}; choose two with --layers IN,OUT", conn.len(), conn.iter().map(|&l| lname(l)).collect::<Vec<_>>()));
            }
            (conn[0], conn[1])
        }
    };
    // the traces of both sides and their exit directions: from the origin to the first point at
    // least `keep` away, summed over the vias of a pair
    let all_chains: Vec<Vec<Vec<(P, P, f64)>>> = [l_in, l_out]
        .iter()
        .map(|&l| {
            sig.iter()
                .map(|&(w, n)| chain(b, n, w.at, w.size / 2.0 + 1e-3, l).unwrap_or_else(|| die(&format!("no track of {} at the via on {}", b.net_name(n), lname(l)))))
                .collect()
        })
        .collect();
    let dirs: Vec<P> = all_chains
        .iter()
        .zip([l_in, l_out])
        .map(|(chains, l)| {
            let mut dir = [0.0, 0.0];
            for ch in chains {
                let q = ch.iter().map(|s| tc0(s.1)).find(|&q| geom::len(q) >= o.keep).unwrap_or_else(|| die(&format!("the trace on {} ends within {} mm of the via; lower --keep", lname(l), o.keep)));
                dir = geom::add(dir, geom::scale(q, 1.0 / geom::len(q)));
            }
            dir
        })
        .collect();
    // rotation: 0° or 45°, whichever brings the exit directions closer to the axes
    let dev = |d: P, rho: f64| -> f64 {
        let th = d[1].atan2(d[0]).to_degrees() + rho;
        ((th % 90.0 + 90.0 + 45.0) % 90.0 - 45.0).abs()
    };
    let rho = o.rotate.unwrap_or_else(|| {
        let worst = |r: f64| dirs.iter().map(|&d| dev(d, r)).fold(0.0, f64::max);
        if worst(45.0) + 1.0 < worst(0.0) {
            45.0
        } else {
            0.0
        }
    });
    let (sr, cr) = rho.to_radians().sin_cos();
    let tc = |q: P| -> P {
        let a = tc0(q);
        [r6(a[0] * cr - a[1] * sr), r6(a[0] * sr + a[1] * cr)]
    };
    let mut sides: Vec<Side> = Vec::new();
    for (si, &l) in [l_in, l_out].iter().enumerate() {
        let chains = &all_chains[si];
        let d0 = dirs[si];
        let dir = [d0[0] * cr - d0[1] * sr, d0[0] * sr + d0[1] * cr];
        let axis = if dir[0].abs() >= dir[1].abs() { 0 } else { 1 };
        let sign = dir[axis].signum();
        let mut kept = Vec::new();
        let mut ref_layer = None;
        for (vi, ch) in chains.iter().enumerate() {
            let mut segs = Vec::new();
            let mut done = None;
            for &(a, c, w) in ch {
                let (pa, pc) = (tc(a), tc(c));
                let (sa, sc) = (sign * pa[axis], sign * pc[axis]);
                if sc >= o.keep && sa < o.keep {
                    let t = (o.keep - sa) / (sc - sa);
                    let q = geom::lerp(pa, pc, t);
                    segs.push((pa, q));
                    done = Some((w, q[1 - axis], a, c, t));
                    break;
                }
                segs.push((pa, pc));
            }
            let (w, u, a, c, t) = done.unwrap_or_else(|| die(&format!("the trace on {} does not leave the keep box along its exit axis", lname(l))));
            // reference planes of the trace piece at the cut (kisi's extraction)
            let q = geom::lerp(a, c, t);
            let refs = piece_refs(&p, sig[vi].1, l, q);
            let rl = refs.into_iter().min_by(|x, y| (z[*x as usize] - z[l as usize]).abs().total_cmp(&(z[*y as usize] - z[l as usize]).abs()));
            if ref_layer.is_none() {
                ref_layer = rl;
            }
            kept.push((segs, w, u));
        }
        let ref_layer = ref_layer.unwrap_or_else(|| die(&format!("the trace on {} has no reference plane at the keep-box edge", lname(l))));
        sides.push(Side { layer: l, axis, sign, kept, ref_layer });
    }
    for s in &sides {
        if s.ref_layer == l_in || s.ref_layer == l_out {
            die("a feed's reference plane is the other feed layer; not supported");
        }
    }

    // window: keep box and the extensions, a margin around them, 1.5 mm beyond the ports
    let keep_box = [-o.keep, -o.keep, o.keep, o.keep];
    let mut win = keep_box;
    for s in &sides {
        for &(_, w, u) in &s.kept {
            let (lo, hi) = (u - w / 2.0, u + w / 2.0);
            let la = 1 - s.axis;
            win[la] = win[la].min(lo);
            win[la + 2] = win[la + 2].max(hi);
        }
    }
    win = [win[0] - o.margin, win[1] - o.margin, win[2] + o.margin, win[3] + o.margin];
    for s in &sides {
        let k = if s.sign > 0.0 { s.axis + 2 } else { s.axis };
        win[k] = s.sign * (o.feed + 1.5);
    }
    let win = win.map(r6);
    let region = [win[0] - 1.0, win[1] - 1.0, win[2] + 1.0, win[3] + 1.0];
    let feed_layer = |l: u16| l == l_in || l == l_out;
    let reg_of = |l: u16| if feed_layer(l) && !o.preview { keep_box } else { region };
    let in_win = |q: P| q[0] >= win[0] && q[0] <= win[2] && q[1] >= win[1] && q[1] <= win[3];

    let mut sheets: Vec<Value> = Vec::new();
    let mut feed_fills: Vec<(u16, String, Vec<P>)> = Vec::new();
    let mut refine: Vec<[f64; 5]> = vec![[keep_box[0], keep_box[1], keep_box[2], keep_box[3], o.keep_cell]];
    let mut nverts = 0usize;
    // zone fills of the other nets
    for zf in &b.zones {
        if own(zf.net) {
            continue;
        }
        let ring: Vec<P> = zf.pts.iter().map(|&q| tc(q)).collect();
        let bb = bbox(&ring);
        let rg = reg_of(zf.layer);
        if bb[0] > rg[2] || bb[2] < rg[0] || bb[1] > rg[3] || bb[3] < rg[1] {
            continue;
        }
        let cl: Vec<P> = clip_ring(&ring, rg).into_iter().map(|q| [r6(q[0]), r6(q[1])]).collect();
        if cl.len() < 3 || area(&cl).abs() < 1e-6 {
            continue;
        }
        nverts += cl.len();
        let bb = bbox(&cl);
        if feed_layer(zf.layer) {
            feed_fills.push((zf.layer, b.net_name(zf.net).to_string(), cl.clone()));
        }
        sheets.push(json!({"z": r6(z[zf.layer as usize]), "rect": bb, "polys": [cl], "net": b.net_name(zf.net)}));
    }
    // coplanar copper next to each feed at the keep-box edge: gap and net per lateral side
    // (low u, high u); a pour closer than 3 h (at most 0.6 mm) continues along the extension
    // as a sheet with a via fence (the same in the 2×thru), so the fixture is the same line
    let fill_net_at = |l: u16, q: P| -> Option<String> { feed_fills.iter().find(|f| f.0 == l && inside_ring(&f.2, q)).map(|f| f.1.clone()) };
    let cpws: Vec<[Option<(f64, String)>; 2]> = sides
        .iter()
        .map(|s| {
            let h = (z[s.layer as usize] - z[s.ref_layer as usize]).abs();
            let gmax = (3.0 * h).clamp(0.1, 0.6);
            let lo = s.kept.iter().map(|k| k.2 - k.1 / 2.0).fold(f64::INFINITY, f64::min);
            let hi = s.kept.iter().map(|k| k.2 + k.1 / 2.0).fold(f64::NEG_INFINITY, f64::max);
            let st = s.sign * (o.keep - 0.03);
            let probe = |edge: f64, dir: f64| -> Option<(f64, String)> {
                (1..=((gmax / 0.005) as usize)).find_map(|k| {
                    let d = k as f64 * 0.005;
                    let q = if s.axis == 0 { [st, edge + dir * d] } else { [edge + dir * d, st] };
                    fill_net_at(s.layer, q).map(|n| (r6(d - 0.0025), n))
                })
            };
            [probe(lo, -1.0), probe(hi, 1.0)]
        })
        .collect();
    // pads of other nets (plated holes as barrels)
    let mut extra_vias: Vec<Value> = Vec::new();
    for pd in &b.pads {
        if own(pd.net) || pd.layers.is_empty() {
            continue;
        }
        let poly: Vec<P> = pad_polygon(pd).into_iter().map(tc).collect();
        let c = tc(pd.at);
        for &l in &pd.layers {
            let rg = reg_of(l);
            if c[0] < rg[0] || c[0] > rg[2] || c[1] < rg[1] || c[1] > rg[3] {
                continue;
            }
            let cl = clip_ring(&poly, rg);
            if cl.len() >= 3 && area(&cl).abs() > 1e-8 {
                sheets.push(json!({"z": r6(z[l as usize]), "rect": bbox(&cl), "polys": [cl], "net": b.net_name(pd.net)}));
            }
        }
        if pd.drill > 0.0 && in_win(c) {
            let (l0, l1) = (*pd.layers.iter().min().unwrap() as usize, *pd.layers.iter().max().unwrap() as usize);
            extra_vias.push(json!({"x": c[0], "y": c[1], "r": r6(pd.drill / 2.0), "z0": r6(z[l1]), "z1": r6(z[l0]), "pad_r": 0.0, "pads": [], "dmesh": 0.1, "net": b.net_name(pd.net)}));
        }
    }
    // tracks of other nets: polygons (no mesh lines of their own)
    for t in &b.tracks {
        if (own(t.net) && !o.preview) || t.pad {
            continue;
        }
        let Some((a, c)) = clip_seg(tc(t.a), tc(t.b), reg_of(t.layer)) else { continue };
        if geom::len(geom::sub(c, a)) < 1e-6 {
            continue;
        }
        sheets.push(json!({"z": r6(z[t.layer as usize]), "rect": bbox(&capsule(a, c, t.width)), "polys": [capsule(a, c, t.width)], "net": b.net_name(t.net)}));
    }
    // the feeds: routed pieces inside the keep box and the straight extensions
    let mut ports: Vec<Value> = Vec::new();
    let mut port_meta: Vec<Value> = Vec::new();
    let mut ext_meta: Vec<Value> = Vec::new();
    let pair = pv.is_some();
    for (si, s) in sides.iter().enumerate() {
        let zl = r6(z[s.layer as usize]);
        let zr = r6(z[s.ref_layer as usize]);
        for (vi, (segs, w, u)) in s.kept.iter().enumerate() {
            let (w, u) = (*w, *u);
            for (k, &(a, c)) in segs.iter().enumerate() {
                if geom::len(geom::sub(c, a)) < 1e-6 {
                    continue;
                }
                let aligned = (a[0] - c[0]).abs() < 1e-6 || (a[1] - c[1]).abs() < 1e-6;
                if aligned {
                    let r = [a[0].min(c[0]), a[1].min(c[1]), a[0].max(c[0]), a[1].max(c[1])];
                    let r = if (a[0] - c[0]).abs() < 1e-6 { [r[0] - w / 2.0, r[1], r[2] + w / 2.0, r[3]] } else { [r[0], r[1] - w / 2.0, r[2], r[3] + w / 2.0] };
                    sheets.push(json!({"z": zl, "rect": r.map(r6), "net": "SIG"}));
                    // round end where it meets the next piece at an angle
                    if k + 1 < segs.len() {
                        sheets.push(json!({"z": zl, "rect": bbox(&capsule(c, c, w)), "polys": [capsule(c, geom::add(c, [1e-4, 0.0]), w)], "net": "SIG"}));
                    }
                } else {
                    let cap = capsule(a, c, w);
                    let bb = bbox(&cap);
                    refine.push([r6(bb[0] - o.dfine), r6(bb[1] - o.dfine), r6(bb[2] + o.dfine), r6(bb[3] + o.dfine), o.dfine]);
                    sheets.push(json!({"z": zl, "rect": bb, "polys": [cap], "net": "SIG"}));
                }
            }
            // extension from the keep-box edge to the port
            let (s0, s1) = (s.sign * o.keep, s.sign * o.feed);
            let (lo, hi) = (s0.min(s1), s0.max(s1));
            let r = if s.axis == 0 { [lo, u - w / 2.0, hi, u + w / 2.0] } else { [u - w / 2.0, lo, u + w / 2.0, hi] };
            sheets.push(json!({"z": zl, "rect": r.map(r6), "net": "SIG"}));
            let port = if s.axis == 0 {
                json!({"x": r6(s1), "y0": r6(u - w / 2.0), "y1": r6(u + w / 2.0), "z0": zr, "z1": zl, "r": 50.0})
            } else {
                json!({"x": r6(u - w / 2.0), "x1": r6(u + w / 2.0), "y0": r6(s1), "y1": r6(s1), "z0": zr, "z1": zl, "r": 50.0})
            };
            let num = si * sig.len() + vi + 1;
            let pn = if pair { format!("{num} {} {}", if vi == 0 { "P" } else { "N" }, if si == 0 { "in" } else { "out" }) } else { format!("{num} {}", if si == 0 { "in" } else { "out" }) };
            ports.push(port);
            port_meta.push(json!({"name": pn, "layer": lname(s.layer), "ref": lname(s.ref_layer)}));
            ext_meta.push(json!({"side": if si == 0 { "in" } else { "out" }, "layer": lname(s.layer), "w": r6(w), "u": r6(u), "axis": if s.axis == 0 { "x" } else { "y" }, "sign": s.sign}));
        }
    }
    // pair ports in kisi's order: 1 = P in, 2 = N in, 3 = P out, 4 = N out (already so)

    // layers with copper at each port, and its net (for the fixtures, the fences and the meta)
    let planes_at_port: Vec<Vec<(u16, String)>> = sides
        .iter()
        .map(|s| {
            let (_, _, u) = s.kept[0];
            let q = if s.axis == 0 { [s.sign * o.feed, u] } else { [u, s.sign * o.feed] };
            let pl: Vec<(u16, String)> = (0..nl as u16).filter(|&l| l != s.layer).filter_map(|l| fill_net_at_any(&sheets, z[l as usize], q).map(|n| (l, n))).collect();
            if !pl.iter().any(|x| x.0 == s.ref_layer) {
                eprintln!("via-case: warning: the reference {} of the {} feed has no copper at the port", lname(s.ref_layer), lname(s.layer));
            }
            pl
        })
        .collect();
    // coplanar sheets and via fences along the extensions; the corridor of every extension
    // (up to its fence, or 0.35 mm beyond the traces) is cleared of the board's own vias. The
    // fence joins the coplanar sheet to every plane of its net at the port (planes of other nets
    // in between get a clearance hole), as the board's stitching vias do.
    let fence_r = 0.15;
    let mut fence_holes: Vec<(f64, f64, f64, f64, String)> = Vec::new(); // x, y, z0, z1, net
    let mut fences: Vec<Value> = Vec::new();
    let mut corridors: Vec<(usize, f64, [f64; 2])> = Vec::new(); // axis, sign, lateral range
    let mut cpw_meta: Vec<Value> = Vec::new();
    for (si, s) in sides.iter().enumerate() {
        let la = 1 - s.axis;
        let lo = s.kept.iter().map(|k| k.2 - k.1 / 2.0).fold(f64::INFINITY, f64::min);
        let hi = s.kept.iter().map(|k| k.2 + k.1 / 2.0).fold(f64::NEG_INFINITY, f64::max);
        let (a0, a1) = if s.sign > 0.0 { (o.keep, win[s.axis + 2]) } else { (win[s.axis], -o.keep) };
        let ref_net = planes_at_port[si].iter().find(|x| x.0 == s.ref_layer).map(|x| x.1.clone());
        let zl = z[s.layer as usize];
        let mut cor = [lo - 0.35, hi + 0.35];
        for (k, dir) in [(0usize, -1.0f64), (1, 1.0)] {
            let Some((g, net)) = &cpws[si][k] else { continue };
            let edge = if k == 0 { lo - g } else { hi + g };
            let (l0, l1) = if k == 0 { (win[la], edge) } else { (edge, win[la + 2]) };
            let r = if s.axis == 0 { [a0, l0, a1, l1] } else { [l0, a0, l1, a1] };
            sheets.push(json!({"z": r6(zl), "rect": r.map(r6), "net": net, "strip": true}));
            let fence = ref_net.as_deref() == Some(net.as_str());
            if fence {
                let zs: Vec<f64> = planes_at_port[si].iter().filter(|x| &x.1 == net).map(|x| z[x.0 as usize]).chain([zl]).collect();
                let (fz0, fz1) = (zs.iter().cloned().fold(f64::INFINITY, f64::min), zs.iter().cloned().fold(f64::NEG_INFINITY, f64::max));
                let off = edge + dir * (fence_r + 0.15);
                let mut t = o.keep + 0.5;
                while t < o.feed + 1.3 {
                    let q = if s.axis == 0 { [s.sign * t, off] } else { [off, s.sign * t] };
                    fences.push(json!({"x": r6(q[0]), "y": r6(q[1]), "r": fence_r, "z0": r6(fz0), "z1": r6(fz1), "pad_r": 0.0, "pads": [], "dmesh": 0.1, "net": net}));
                    fence_holes.push((q[0], q[1], fz0, fz1, net.clone()));
                    t += 1.0;
                }
                cor[k] = off + dir * (fence_r + 0.15);
            } else {
                cor[k] = edge + dir * 0.1;
            }
            cpw_meta.push(json!({"side": if si == 0 { "in" } else { "out" }, "lateral": if k == 0 { "low" } else { "high" }, "gap_mm": g, "net": net, "fence": fence}));
        }
        corridors.push((s.axis, s.sign, cor));
    }
    let in_corridor = |q: P| corridors.iter().any(|&(ax, sg, cor)| sg * q[ax] > o.keep && q[1 - ax] > cor[0] && q[1 - ax] < cor[1]);

    // vias: signal (and partner), then every other via in the window
    let mut vias_out: Vec<Value> = Vec::new();
    for &(w, n) in &sig {
        let (top, bot) = barrel(w);
        let conn_w: Vec<usize> = (w.from..=w.to).filter(|&l| chain(b, n, w.at, w.size / 2.0 + 1e-3, l).is_some()).map(|l| l as usize).collect();
        let mut pads: Vec<usize> = conn_w.clone();
        match w.remove_unused {
            None => pads.extend(top..=bot),
            Some(keep_ends) => {
                if keep_ends {
                    pads.push(top);
                    pads.push(bot);
                }
            }
        }
        pads.sort();
        pads.dedup();
        let pads: Vec<f64> = pads.into_iter().filter(|&k| k >= top && k <= bot).map(|k| r6(z[k])).collect();
        let c = tc(w.at);
        vias_out.push(json!({"x": c[0], "y": c[1], "r": r6((w.drill / 2.0).max(0.025)), "z0": r6(z[bot]), "z1": r6(z[top]), "pad_r": r6(w.size / 2.0), "pads": pads, "net": "SIG"}));
    }
    let mut n_ret = 0;
    for w in &b.vias {
        if own(w.net) {
            continue;
        }
        let c = tc(w.at);
        if !in_win(c) || in_corridor(c) {
            continue;
        }
        let (top, bot) = barrel(w);
        let plane = b.plane_net.get(w.net as usize).copied().unwrap_or(false);
        if plane {
            n_ret += 1;
        }
        let pads: Vec<f64> = if plane || w.remove_unused.is_some() { vec![] } else { (top..=bot).map(|k| r6(z[k])).collect() };
        vias_out.push(json!({"x": c[0], "y": c[1], "r": r6((w.drill / 2.0).max(0.025)), "z0": r6(z[bot]), "z1": r6(z[top]), "pad_r": if pads.is_empty() { 0.0 } else { r6(w.size / 2.0) }, "pads": pads, "dmesh": 0.1, "net": b.net_name(w.net)}));
    }
    vias_out.extend(extra_vias.into_iter().filter(|v| !in_corridor([v["x"].as_f64().unwrap(), v["y"].as_f64().unwrap()])));
    vias_out.extend(fences);
    // the coplanar sheets of the extensions get a clearance around vias of other nets
    for sh in sheets.iter_mut().filter(|sh| sh["strip"].as_bool() == Some(true)) {
        let (zs, net) = (sh["z"].as_f64().unwrap(), sh["net"].as_str().unwrap_or("").to_string());
        let r: Vec<f64> = sh["rect"].as_array().unwrap().iter().map(|x| x.as_f64().unwrap()).collect();
        let holes: Vec<Value> = vias_out
            .iter()
            .filter(|v| v["net"].as_str() != Some(net.as_str()) && v["z0"].as_f64().unwrap() <= zs + 1e-9 && v["z1"].as_f64().unwrap() >= zs - 1e-9)
            .filter_map(|v| {
                let (x, y) = (v["x"].as_f64().unwrap(), v["y"].as_f64().unwrap());
                let rr = v["r"].as_f64().unwrap().max(v["pad_r"].as_f64().unwrap()) + 0.15;
                (x > r[0] - rr && x < r[2] + rr && y > r[1] - rr && y < r[3] + rr).then(|| json!([x, y, r6(rr)]))
            })
            .collect();
        sh["holes"] = json!(holes);
    }

    for sh in sheets.iter_mut() {
        let (zs, net) = (sh["z"].as_f64().unwrap(), sh["net"].as_str().unwrap_or("").to_string());
        let r: Vec<f64> = sh["rect"].as_array().unwrap().iter().map(|x| x.as_f64().unwrap()).collect();
        let add: Vec<Value> = fence_holes
            .iter()
            .filter(|f| f.4 != net && zs > f.2 + 1e-9 && zs < f.3 - 1e-9 && f.0 > r[0] - 0.35 && f.0 < r[2] + 0.35 && f.1 > r[1] - 0.35 && f.1 < r[3] + 0.35)
            .map(|f| json!([r6(f.0), r6(f.1), 0.35]))
            .collect();
        if !add.is_empty() {
            let mut h = sh["holes"].as_array().cloned().unwrap_or_default();
            h.extend(add);
            sh["holes"] = json!(h);
        }
    }

    // dielectric slabs, bottom first
    let layers: Vec<Value> = (0..nl - 1).rev().map(|k| json!({"z0": r6(z[k + 1]), "z1": r6(z[k]), "er": su.dielectrics[k].epsilon_r, "tand": su.dielectrics[k].loss_tangent})).collect();
    let t_board = z[0];
    let domain = [win[0], win[1], r6(-o.air), win[2], win[3], r6(t_board + o.air)];
    // monitors: xy plane in the thickest dielectric the signal via crosses, plane through port 1
    let (top, bot) = barrel(v);
    let kthick = (top..bot).max_by(|&a, &c| su.dielectrics[a].thickness.total_cmp(&su.dielectrics[c].thickness)).unwrap_or(0);
    let zmon = r6(0.5 * (z[kthick] + z[kthick + 1]));
    let s0 = &sides[0];
    let mons = json!([{"axis": "z", "pos": zmon, "freqs": [10e9, 20e9, 30e9], "comps": [2]},
                      {"axis": if s0.axis == 0 { "y" } else { "x" }, "pos": r6(s0.kept[0].2), "freqs": [10e9, 20e9, 30e9], "comps": [s0.axis, 2]}]);

    let stem = o.pcb.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let name = o.name.clone().unwrap_or_else(|| {
        let nn: String = b.net_name(net).rsplit('/').next().unwrap_or("").chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
        format!("board_{stem}_{nn}")
    });
    // kisi's return vias for this via (plane-net vias within 3 mm, the nearest 8), to match the
    // ladder geometry kisi prints with KISI_DEBUG_VIA
    let mut rets: Vec<(f64, P)> = b
        .vias
        .iter()
        .filter(|w| !own(w.net) && w.net != 0 && b.plane_net.get(w.net as usize).copied().unwrap_or(false))
        .map(|w| (geom::len(geom::sub(w.at, v.at)), geom::sub(w.at, v.at)))
        .filter(|&(d, _)| d > 0.01 && d < 3.0)
        .collect();
    rets.sort_by(|a, c| a.0.total_cmp(&c.0));
    rets.truncate(8);
    let planes_meta: Vec<String> = planes_at_port.iter().flatten().map(|x| lname(x.0)).collect::<std::collections::BTreeSet<_>>().into_iter().collect();
    let approx = vec![
        "copper has zero thickness; layer heights are kisi's (dielectric plus half the copper on each side)",
        "fills, pads and tracks outside the window are left out; copper that reaches the window edge continues through the PML",
        "on the feed layers only the keep box is taken from the board: outside it the own trace(s) run straight along the nearest axis to the port, without pour or other copper",
        "traces of other nets are cut at the keep box on the feed layers (floating ends)",
        "other vias and traces of the own net (and of the partner net) are left out",
        "solder mask, plating thickness and the barrel's hole wall are not modelled; the barrel is a solid cylinder of the drill radius",
        "dielectric loss: a conductivity fixed at fmax/2 (tanδ exact there); lossless copper",
    ];
    let title = format!("{} via {}→{} at ({:.3}, {:.3}){}", b.net_name(net), lname(l_in), lname(l_out), v.at[0], v.at[1], pv.map(|_| " with its partner").unwrap_or(""));
    let su_title = format!("{} from the board file{}", stem, if cfg.is_some() { " and its kisi.json" } else { "" });
    let thru_names = [format!("thru_{name}_in"), format!("thru_{name}_out")];
    let mut case = json!({
        "name": name, "domain": domain, "board": [win[0], win[1], win[2], win[3]], "layers": layers,
        "sheets": sheets, "vias": vias_out, "ports": ports, "monitors": mons,
        "fmax": o.fmax, "mesh": {"dfine": o.dfine, "ratio": 1.4, "thirds": true, "refine": refine},
        "boundary": "pml", "pml_cells": 8, "conformal": true, "end_db": -50, "max_steps": 150000, "nf": 400,
    });
    case["meta"] = json!({
        "group": "board", "title": title, "stackup": stem, "stackup_title": su_title,
        "copper_z": z.iter().map(|&x| r6(x)).collect::<Vec<_>>(), "planes": planes_meta, "pwr_planes": [],
        "ports": port_meta, "lumped": [],
        "params": {"board": o.pcb.file_name().map(|f| f.to_string_lossy().to_string()), "net": b.net_name(net), "partner": partner.map(|pn| b.net_name(pn).to_string()),
                   "via_at": v.at, "partner_at": pv.map(|w| w.at), "pitch_mm": pv.map(|w| r6(geom::len(geom::sub(w.at, v.at)))),
                   "layers": [lname(v.from), lname(v.to)], "l_in": lname(l_in), "l_out": lname(l_out), "drill_mm": v.drill, "pad_mm": v.size,
                   "remove_unused_layers": v.remove_unused.is_some(), "backdrill": v.backdrill.map(|(a, c)| [lname(a), lname(c)]),
                   "return_vias_in_window": n_ret, "keep_mm": o.keep, "feed_mm": o.feed, "window": win, "origin": c0, "y": "up (board y flipped)",
                   "rotation_deg": rho,
                   "fill_vertices": nverts},
        "extensions": ext_meta,
        "coplanar": cpw_meta,
        "kisi_returns": rets.iter().map(|r| [r6(r.1[0]), r6(r.1[1])]).collect::<Vec<_>>(),
        "thru": thru_names,
        "approximations": approx,
    });
    if pair {
        case["meta"]["excite"] = json!([0, 1, 2, 3]);
    }
    let out = o.out.clone().unwrap_or_else(|| PathBuf::from("."));
    std::fs::create_dir_all(&out).unwrap_or_else(|e| die(&e.to_string()));
    write(&out, &name, &case);
    // 2×thru fixtures
    for (si, s) in sides.iter().enumerate() {
        let th = thru_case(&thru_names[si], s, &planes_at_port[si], &cpws[si], &z, b, &o, &lname);
        write(&out, &thru_names[si], &th);
    }
    eprintln!(
        "via-case: {name}: {} sheets ({} fill vertices), {} vias ({} return vias), ports {}, window {:.2} × {:.2} mm",
        case["sheets"].as_array().unwrap().len(),
        nverts,
        case["vias"].as_array().unwrap().len(),
        n_ret,
        case["ports"].as_array().unwrap().len(),
        win[2] - win[0],
        win[3] - win[1]
    );
}

/// Reference layers (kisi's extraction) of the track piece of `net` on `layer` nearest to `q`.
fn piece_refs(p: &Project, net: u32, layer: u16, q: P) -> Vec<u16> {
    let (b, an) = (&p.board, &p.an);
    let mut best: Option<(f64, usize)> = None;
    for k in 0..an.pieces.len() / 4 {
        let t = &b.tracks[an.pieces[4 * k] as usize];
        if t.layer != layer || t.net != net {
            continue;
        }
        let (t0, t1) = (an.pieces[4 * k + 1], an.pieces[4 * k + 2]);
        let m = geom::lerp(t.a, t.b, 0.5 * (t0 + t1));
        let d = geom::len(geom::sub(m, q));
        if best.is_none_or(|x| d < x.0) {
            best = Some((d, k));
        }
    }
    best.map(|(_, k)| {
        let x = &an.cross_sections[an.pieces[4 * k + 3] as usize];
        [x.ref_up, x.ref_dn].into_iter().flatten().collect()
    })
    .unwrap_or_default()
}

/// 2×thru of one feed side: the straight trace (or pair) along x, 2·feed long, solid sheets on
/// the layers that have copper at the port in the case.
#[allow(clippy::too_many_arguments)]
fn thru_case(name: &str, s: &Side, planes: &[(u16, String)], cpw: &[Option<(f64, String)>; 2], z: &[f64], b: &Board, o: &Opt, lname: &dyn Fn(u16) -> String) -> Value {
    let su = b.stackup();
    let nl = su.n();
    let us: Vec<f64> = s.kept.iter().map(|k| k.2).collect();
    let umid = 0.5 * (us.iter().cloned().fold(f64::INFINITY, f64::min) + us.iter().cloned().fold(f64::NEG_INFINITY, f64::max));
    let spread = us.iter().map(|u| (u - umid).abs()).fold(0.0, f64::max);
    let hx = o.feed + 1.5;
    let hy = r6(o.margin.max(3.0 - spread) + spread + s.kept[0].1);
    let board = [-hx, -hy, hx, hy];
    let layers: Vec<Value> = (0..nl - 1).rev().map(|k| json!({"z0": r6(z[k + 1]), "z1": r6(z[k]), "er": su.dielectrics[k].epsilon_r, "tand": su.dielectrics[k].loss_tangent})).collect();
    let mut sheets: Vec<Value> = Vec::new();
    let mut ports: Vec<Value> = Vec::new();
    let mut pm: Vec<Value> = Vec::new();
    let zl = r6(z[s.layer as usize]);
    let zr = r6(z[s.ref_layer as usize]);
    // lateral order as in the case (the pair keeps its pitch); the case's x/y orientation does
    // not matter for a straight line
    let n = s.kept.len();
    for (side, base) in [(-1.0, 0), (1.0, n)] {
        for (vi, k) in s.kept.iter().enumerate() {
            let (w, u) = (k.1, r6(k.2 - umid));
            if side < 0.0 {
                sheets.push(json!({"z": zl, "rect": [r6(-o.feed), r6(u - w / 2.0), r6(o.feed), r6(u + w / 2.0)], "net": "SIG"}));
            }
            ports.push(json!({"x": r6(side * o.feed), "y0": r6(u - w / 2.0), "y1": r6(u + w / 2.0), "z0": zr, "z1": zl, "r": 50.0}));
            pm.push(json!({"name": format!("{} {}", base + vi + 1, if side < 0.0 { "near" } else { "far" }), "layer": lname(s.layer), "ref": lname(s.ref_layer)}));
        }
    }
    // fences (as in the case): from the feed layer to every plane of the coplanar net
    let mut fence_y: Vec<(f64, f64, f64, String)> = Vec::new();
    // coplanar sheets and fences as along the case's extension (fence pitch 1 mm, symmetric)
    let lo = s.kept.iter().map(|k| k.2 - umid - k.1 / 2.0).fold(f64::INFINITY, f64::min);
    let hi = s.kept.iter().map(|k| k.2 - umid + k.1 / 2.0).fold(f64::NEG_INFINITY, f64::max);
    let mut vias: Vec<Value> = Vec::new();
    let ref_net = planes.iter().find(|x| x.0 == s.ref_layer).map(|x| x.1.clone());
    for (k, dir) in [(0usize, -1.0f64), (1, 1.0)] {
        let Some((g, net)) = &cpw[k] else { continue };
        let edge = if k == 0 { lo - g } else { hi + g };
        let (l0, l1) = if k == 0 { (-hy, edge) } else { (edge, hy) };
        sheets.push(json!({"z": zl, "rect": [-hx, r6(l0), hx, r6(l1)], "net": net}));
        if ref_net.as_deref() == Some(net.as_str()) {
            let zs: Vec<f64> = planes.iter().filter(|x| &x.1 == net).map(|x| z[x.0 as usize]).chain([zl]).collect();
            let (fz0, fz1) = (zs.iter().cloned().fold(f64::INFINITY, f64::min), zs.iter().cloned().fold(f64::NEG_INFINITY, f64::max));
            let off = edge + dir * 0.3;
            fence_y.push((off, fz0, fz1, net.clone()));
            let mut t = 0.5;
            while t < hx - 0.2 {
                for sg in [-1.0, 1.0] {
                    vias.push(json!({"x": r6(sg * t), "y": r6(off), "r": 0.15, "z0": r6(fz0), "z1": r6(fz1), "pad_r": 0.0, "pads": [], "dmesh": 0.1, "net": net}));
                }
                t += 1.0;
            }
        }
    }
    let _ = zr;
    // solid planes (their own net), with a clearance where a fence of another net crosses
    for (l, net) in planes {
        let zp = z[*l as usize];
        let mut holes: Vec<Value> = Vec::new();
        for (off, fz0, fz1, fnet) in &fence_y {
            if fnet != net && zp > fz0 + 1e-9 && zp < fz1 - 1e-9 {
                let mut t = 0.5;
                while t < hx - 0.2 {
                    holes.push(json!([r6(t), r6(*off), 0.35]));
                    holes.push(json!([r6(-t), r6(*off), 0.35]));
                    t += 1.0;
                }
            }
        }
        sheets.push(json!({"z": r6(zp), "rect": board, "holes": holes, "net": net}));
    }
    let domain = [-hx, -hy, r6(-o.air), hx, hy, r6(z[0] + o.air)];
    let mut c = json!({
        "name": name, "domain": domain, "board": board, "layers": layers, "sheets": sheets, "vias": vias, "ports": ports,
        "fmax": o.fmax, "mesh": {"dfine": o.dfine, "ratio": 1.4, "thirds": true}, "boundary": "pml", "pml_cells": 8, "conformal": true,
        "end_db": -50, "max_steps": 150000, "nf": 400, "monitors": [],
    });
    let (excite, mirror) = if n == 1 { (json!([0]), json!([1, 0])) } else { (json!([0, 1]), json!([2, 3, 0, 1])) };
    c["meta"] = json!({"group": "thru", "title": format!("2×thru {}, {} mm{}", lname(s.layer), 2.0 * o.feed, if n > 1 { ", pair" } else { "" }),
        "stackup": "board", "stackup_title": "as the board case", "copper_z": z.iter().map(|&x| r6(x)).collect::<Vec<_>>(),
        "planes": planes.iter().map(|x| lname(x.0)).collect::<Vec<_>>(), "plane_nets": planes.iter().map(|x| x.1.clone()).collect::<Vec<_>>(), "pwr_planes": [], "ports": pm, "lumped": [],
        "params": {"layer": lname(s.layer), "width_mm": s.kept[0].1, "length_mm": 2.0 * o.feed, "offsets_mm": s.kept.iter().map(|k| r6(k.2 - umid)).collect::<Vec<_>>()},
        "excite": excite, "mirror": mirror});
    c
}

fn write(dir: &Path, name: &str, v: &Value) {
    let p = dir.join(format!("{name}.json"));
    std::fs::write(&p, serde_json::to_string(v).unwrap()).unwrap_or_else(|e| die(&format!("{}: {e}", p.display())));
    eprintln!("via-case: wrote {}", p.display());
}

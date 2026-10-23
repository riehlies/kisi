//! `kisi-validate cavity-model <case.json> <out dir> [--grid H] [--cap-r R] [--no-radiation] [--no-grid] [--suffix S]`
//!
//! kisi's via ladder on a finite-board case of validation/fdtd/board_resonances.py (group R): the
//! signal vias between the two planes of the case, read as `models.rs` reads the reference cases
//! (same feed lines, the 3D loss model: dielectric conductivity fixed at fmax/2, lossless metal),
//! with the plane pair as
//! * the infinite radial line (kisi's ladder as it is)        → `<out>/ladder_inf.s2p`
//! * the rectangular cavity (`via_cavity::RectEnv`)            → `<out>/ladder_rect.s2p`
//! * the grid of the plane outline (`via_cavity::GridEnv`, cell H mm, default 0.25)
//!                                                             → `<out>/ladder_grid.s2p`
//! Two signal vias: the coupled 4-port (`via_pbm::coupled_s`, ports A in, A out, B in, B out) →
//! `ladder_rect.s4p`, `ladder_grid.s4p`. `--cap-r`: the radius of the post of a capacitor between
//! the planes (the 3D lumped element is one cell wide: Peaceman's radius of its cell);
//! `--no-radiation`: magnetic walls without the edges' radiation loss; `--no-grid`: the rectangle
//! only; `--suffix S`: files `ladder_<kind>_S.*`. Prints the run times as JSON.

use crate::models::{feed_line, freqs, planes_at, s_of, write_s2p};
use kisi_core::geom::BBox;
use kisi_core::num::Cx;
use kisi_core::via_cavity::{EnvPort, GridEnv, Load, Mask, PairEnv, PairMedium, RectEnv};
use kisi_core::via_pbm::{coupled_s, CavityLink, Feed, Grid, Mode, PbmVia, Post, Slab, ViaGeom};
use serde_json::{json, Value};
use std::rc::Rc;
use std::time::Instant;

fn num(v: &Value) -> f64 {
    v.as_f64().unwrap()
}

/// the ladder geometry of signal via k (ports 2k, 2k + 1)
fn via_geom_k(case: &Value, k: usize) -> ViaGeom {
    let vias = case["vias"].as_array().unwrap();
    let sigs: Vec<&Value> = vias.iter().filter(|v| v["net"].as_str().unwrap_or("SIG") == "SIG").collect();
    let v = sigs[k];
    let (vx, vy) = (num(&v["x"]), num(&v["y"]));
    let planes = planes_at(case, vx, vy, None);
    let layers = case["layers"].as_array().unwrap();
    let ports = case["ports"].as_array().unwrap();
    let feed = |p: &Value| {
        let (w, z, zr) = (num(&p["y1"]) - num(&p["y0"]), num(&p["z1"]), num(&p["z0"]));
        let (zc, eeff) = feed_line(layers, &planes, w, z, zr);
        Feed { z, z_ref: zr, w, zc, eeff, dropped: 0.0, antipad_line: true }
    };
    let post = |w: &Value| Post { dx: num(&w["x"]) - vx, dy: num(&w["y"]) - vy, r: num(&w["r"]), z0: num(&w["z0"]), z1: num(&w["z1"]) };
    ViaGeom {
        slabs: layers.iter().map(|l| Slab { z0: num(&l["z0"]), z1: num(&l["z1"]), er: num(&l["er"]), tand: l["tand"].as_f64().unwrap_or(0.0) }).collect(),
        planes: planes.clone(),
        r: num(&v["r"]),
        z0: num(&v["z0"]),
        z1: num(&v["z1"]),
        pads: v["pads"].as_array().unwrap().iter().map(num).collect(),
        pad_r: num(&v["pad_r"]),
        returns: vias.iter().filter(|w| w["net"].as_str() == Some("GND")).map(post).collect(),
        partner: None,
        feeds: [feed(&ports[2 * k]), feed(&ports[2 * k + 1])],
        f_loss: Some(case["f_loss"].as_f64().unwrap_or(num(&case["fmax"]) / 2.0)),
        junction: true,
        sigma: None,
    }
}

fn write_s4p(path: &std::path::Path, title: &str, f: &[f64], s: &[Vec<Cx>], z0: f64) {
    let mut t = format!("! {title}\n# Hz S RI R {z0}\n");
    for (fk, m) in f.iter().zip(s) {
        for i in 0..4 {
            t += &if i == 0 { format!("{fk:.6e}") } else { " ".to_string() };
            for j in 0..4 {
                t += &format!(" {:.9e} {:.9e}", m[i * 4 + j].re, m[i * 4 + j].im);
            }
            t += "\n";
        }
    }
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, t).unwrap();
}

pub fn run(args: &[String]) {
    let case: Value = serde_json::from_str(&std::fs::read_to_string(&args[0]).unwrap()).unwrap();
    let out = std::path::Path::new(&args[1]);
    let opt = |k: &str| args.iter().position(|a| a == k).and_then(|i| args.get(i + 1)).and_then(|s| s.parse::<f64>().ok());
    let h_grid = opt("--grid").unwrap_or(0.25);
    let cid = case["name"].as_str().unwrap_or("case").to_string();
    let meta = &case["meta"];
    let f = freqs(&case);
    let fmax = num(&case["fmax"]);
    let bx: Vec<f64> = meta["board"].as_array().unwrap().iter().map(num).collect();
    let (a, b) = ((bx[2] - bx[0]) * 1e-3, (bx[3] - bx[1]) * 1e-3);
    let cav = &meta["cavity"];
    // the two planes (sheets of plane nets) and the medium between them
    let mut zp: Vec<f64> = case["sheets"].as_array().unwrap().iter().filter(|s| matches!(s["net"].as_str(), Some("GND") | Some("PWR"))).map(|s| num(&s["z"])).collect();
    zp.sort_by(|x, y| y.partial_cmp(x).unwrap());
    zp.dedup_by(|x, y| (*x - *y).abs() < 1e-9);
    let (z_top, z_bot) = (zp[0], zp[zp.len() - 1]);
    let no_rad = args.iter().any(|x| x == "--no-radiation");
    let suffix = args.iter().position(|a| a == "--suffix").and_then(|i| args.get(i + 1)).map(|s| format!("_{s}")).unwrap_or_default();
    let skip_grid = args.iter().any(|x| x == "--no-grid");
    let mut medium = PairMedium { d: (z_top - z_bot) * 1e-3, er: num(&cav["er"]), tand: num(&cav["tand"]), f_loss: Some(case["f_loss"].as_f64().unwrap_or(fmax / 2.0)), sigma: None, area: 0.0, edges: vec![] };
    // ports of the cavity: every barrel between the planes (signal vias first, then returns)
    let vias = case["vias"].as_array().unwrap();
    let sigs: Vec<&Value> = vias.iter().filter(|v| v["net"].as_str().unwrap_or("SIG") == "SIG").collect();
    let rets: Vec<&Value> = vias.iter().filter(|v| v["net"].as_str() == Some("GND")).collect();
    let ports_mm: Vec<EnvPort> = sigs.iter().chain(rets.iter()).map(|v| EnvPort { x: num(&v["x"]) - bx[0], y: num(&v["y"]) - bx[1], r: num(&v["r"]) }).collect();
    let ports_m: Vec<EnvPort> = ports_mm.iter().map(|p| EnvPort { x: p.x * 1e-3, y: p.y * 1e-3, r: p.r * 1e-3 }).collect();
    let cap_r = opt("--cap-r").unwrap_or(0.05);
    let caps: Vec<Load> = meta["caps"]
        .as_array()
        .map(|cs| cs.iter().map(|c| Load { at: EnvPort { x: num(&c["at"][0]) - bx[0], y: num(&c["at"][1]) - bx[1], r: cap_r }, r_ohm: num(&c["r"]), l: num(&c["l"]), c: c["c"].as_f64() }).collect())
        .unwrap_or_default();
    let caps_m: Vec<Load> = caps.iter().map(|l| Load { at: EnvPort { x: l.at.x * 1e-3, y: l.at.y * 1e-3, r: l.at.r * 1e-3 }, ..*l }).collect();
    let geoms: Vec<ViaGeom> = (0..sigs.len()).map(|k| via_geom_k(&case, k)).collect();
    let origin = |k: usize| (ports_m[k].x, ports_m[k].y);
    let mut timing = serde_json::Map::new();
    let title = meta["title"].as_str().unwrap_or("");
    // infinite plates (kisi's ladder as it is), via 0
    let t0 = Instant::now();
    let m0 = PbmVia::build(&geoms[0], Mode::Single, Grid::default());
    let leakage: Vec<Value> = [2e9, 5e9, 10e9, 20e9].iter().map(|&fl| json!({"ghz": fl / 1e9, "pairs": m0.pair_leakage(fl).iter().map(|l| json!([l.0, l.1, l.2])).collect::<Vec<_>>()})).collect();
    let s0: Vec<[Cx; 4]> = f.iter().map(|&fk| s_of(&m0.abcd(fk), 50.0)).collect();
    write_s2p(&out.join("ladder_inf.s2p"), &format!("{cid}: kisi's ladder, infinite plates ({title})"), &f, &s0, 50.0);
    timing.insert("infinite".into(), json!({"ms": t0.elapsed().as_secs_f64() * 1e3}));
    // the cavity environments
    let outline: Option<Vec<[f64; 2]>> = meta["plane_outline"].as_array().map(|r| r.iter().map(|p| [num(&p[0]) - bx[0], num(&p[1]) - bx[1]]).collect());
    // the region (outline or board) for the edge radiation
    let mut region = Mask::new(&BBox { min: [0.0, 0.0], max: [bx[2] - bx[0], bx[3] - bx[1]] }, h_grid);
    match &outline {
        Some(ring) => region.fill_rings(&[&ring[..]]),
        None => region.on.iter_mut().for_each(|x| *x = true),
    }
    if !no_rad {
        medium.area = region.count() as f64 * h_grid * h_grid * 1e-6;
        medium.edges = region.edge_runs().iter().map(|l| l * 1e-3).collect();
    }
    let mut envs: Vec<(&str, Rc<PairEnv>, f64)> = vec![];
    if outline.is_none() {
        let t = Instant::now();
        let e = RectEnv::new(a, b, medium.clone(), ports_m.clone(), caps_m.clone(), fmax);
        envs.push(("rect", Rc::new(PairEnv::Rect(e)), t.elapsed().as_secs_f64() * 1e3));
    }
    if !skip_grid {
        let t = Instant::now();
        let e = GridEnv::new(&region, medium.clone(), &[], &caps, &ports_mm);
        envs.push(("grid", Rc::new(PairEnv::Grid(e)), t.elapsed().as_secs_f64() * 1e3));
    }
    for (tag, env, t_env) in envs {
        let t = Instant::now();
        let ms: Vec<PbmVia> = geoms
            .iter()
            .enumerate()
            .map(|(k, g)| PbmVia::build_with(g, Mode::Single, Grid::default(), vec![CavityLink { z_top, z_bot, env: env.clone(), origin: origin(k) }]))
            .collect();
        assert!(ms.iter().all(|m| m.has_cavity()), "{cid}: the cavity was not linked");
        if ms.len() == 1 {
            let s: Vec<[Cx; 4]> = f.iter().map(|&fk| s_of(&ms[0].abcd(fk), 50.0)).collect();
            write_s2p(&out.join(format!("ladder_{tag}{suffix}.s2p")), &format!("{cid}: kisi's ladder with the plane pair as a finite cavity ({tag}) ({title})"), &f, &s, 50.0);
        } else {
            let refs: Vec<&PbmVia> = ms.iter().collect();
            let s: Vec<Vec<Cx>> = f.iter().map(|&fk| coupled_s(&refs, fk, 50.0)).collect();
            write_s4p(&out.join(format!("ladder_{tag}{suffix}.s4p")), &format!("{cid}: kisi's ladders coupled through the finite cavity ({tag}) ({title})"), &f, &s, 50.0);
            // via A alone in the cavity (B's barrel open), as a 2-port
            let s: Vec<[Cx; 4]> = f.iter().map(|&fk| s_of(&ms[0].abcd(fk), 50.0)).collect();
            write_s2p(&out.join(format!("ladder_{tag}{suffix}.s2p")), &format!("{cid}: via A with the finite cavity ({tag}), B open"), &f, &s, 50.0);
        }
        let cells = match &*env {
            PairEnv::Grid(g) => json!(g.cells()),
            PairEnv::Rect(r) => json!(r.modes()),
        };
        timing.insert(tag.into(), json!({"env_ms": t_env, "ladder_ms": t.elapsed().as_secs_f64() * 1e3, "nf": f.len(), "cells_or_modes": cells}));
    }
    println!("{}", serde_json::to_string_pretty(&json!({"case": cid, "timing": timing, "leakage": leakage})).unwrap());
}

//! `kisi-validate cavity-model <case.json> <out dir> [--grid H] [--cap-r R] [--no-radiation] [--no-grid] [--suffix S]`
//!
//! kisi's via ladder on a finite-board case of validation/fdtd/board_resonances.py (group R): the
//! signal vias between the two planes of the case, read as `models.rs` reads the reference cases
//! (same feed lines, the 3D loss model: dielectric conductivity fixed at fmax/2, lossless metal),
//! with the plane pair as
//! * the infinite radial line (kisi's ladder as it is)        → `<out>/ladder_inf.s2p`
//! * the rectangular cavity (`via_cavity::RectEnv`)            → `<out>/ladder_rect.s2p`
//! * the grid of the plane outline (`via_cavity::GridEnv`, cell H mm, default 0.25)
//!   → `<out>/ladder_grid.s2p`
//!
//! Two signal vias: the coupled 4-port (`via_pbm::coupled_s`, ports A in, A out, B in, B out) →
//! `ladder_rect.s4p`, `ladder_grid.s4p`. `--cap-r`: the radius of the post of a capacitor between
//! the planes (the 3D lumped element is one cell wide: Peaceman's radius of its cell);
//! `--no-radiation`: magnetic walls without the edges' radiation loss; `--no-grid`: the rectangle
//! only; `--suffix S`: files `ladder_<kind>_S.*`; `--returns`: return groups (each GND via joins
//! the GND sheets with copper at it, the capacitors are posts between the planes of their lumped
//! element, validation/fdtd/BOARD_RETURNS.md). A case with several plane pairs gets a cavity for
//! each (the same outline). Prints the run times as JSON.

use crate::models::{feed_line, freqs, planes_at, s_of, write_s2p};
use kisi_core::geom::BBox;
use kisi_core::num::Cx;
use kisi_core::via_cavity::{EnvPort, GridEnv, Load, Mask, PairEnv, PairMedium, RectEnv};
use kisi_core::via_pbm::{coupled_s, CapPost, CavityLink, Feed, Grid, Mode, PbmVia, Post, Slab, ViaGeom};
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
    let post = |w: &Value| Post {
        dx: num(&w["x"]) - vx,
        dy: num(&w["y"]) - vy,
        r: num(&w["r"]),
        z0: num(&w["z0"]),
        z1: num(&w["z1"]),
        joins: None,
        open_r: vec![],
    };
    ViaGeom {
        slabs: layers
            .iter()
            .map(|l| Slab {
                z0: num(&l["z0"]),
                z1: num(&l["z1"]),
                er: num(&l["er"]),
                tand: l["tand"].as_f64().unwrap_or(0.0),
            })
            .collect(),
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
        caps: vec![],
        coplanar: vec![],
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

/// whether the sheet has copper at (x, y): inside its rectangle (and outline polygons) and in none
/// of its holes (circles [x, y, r] and slots [x0, y0, x1, y1, r])
fn metal_at(s: &Value, x: f64, y: f64) -> bool {
    let r: Vec<f64> = s["rect"].as_array().map(|a| a.iter().map(num).collect()).unwrap_or_default();
    if r.len() == 4 && !(r[0] - 1e-9 <= x && x <= r[2] + 1e-9 && r[1] - 1e-9 <= y && y <= r[3] + 1e-9) {
        return false;
    }
    if let Some(polys) = s["polys"].as_array() {
        let mut inside = false;
        for ring in polys {
            let pts: Vec<(f64, f64)> = ring.as_array().unwrap().iter().map(|p| (num(&p[0]), num(&p[1]))).collect();
            let n = pts.len();
            for i in 0..n {
                let (a, b) = (pts[i], pts[(i + 1) % n]);
                if (a.1 > y) != (b.1 > y) && x < a.0 + (y - a.1) / (b.1 - a.1) * (b.0 - a.0) {
                    inside = !inside;
                }
            }
        }
        if !inside {
            return false;
        }
    }
    let holes: Vec<Vec<f64>> = s["holes"]
        .as_array()
        .map(|a| a.iter().map(|h| h.as_array().unwrap().iter().map(num).collect()).collect())
        .unwrap_or_default();
    !holes.iter().any(|h| {
        if h.len() < 5 {
            (x - h[0]).hypot(y - h[1]) < h[2]
        } else {
            let (dx, dy) = (h[2] - h[0], h[3] - h[1]);
            let l2 = dx * dx + dy * dy;
            let t = if l2 > 0.0 { (((x - h[0]) * dx + (y - h[1]) * dy) / l2).clamp(0.0, 1.0) } else { 0.0 };
            (x - h[0] - t * dx).hypot(y - h[1] - t * dy) < h[4]
        }
    })
}

pub fn run(args: &[String]) {
    let case: Value = serde_json::from_str(&std::fs::read_to_string(&args[0]).unwrap()).unwrap();
    let out = std::path::Path::new(&args[1]);
    let opt =
        |k: &str| args.iter().position(|a| a == k).and_then(|i| args.get(i + 1)).and_then(|s| s.parse::<f64>().ok());
    let h_grid = opt("--grid").unwrap_or(0.25);
    let cid = case["name"].as_str().unwrap_or("case").to_string();
    let meta = &case["meta"];
    let f = freqs(&case);
    let fmax = num(&case["fmax"]);
    let f_loss = case["f_loss"].as_f64().unwrap_or(fmax / 2.0);
    let bx: Vec<f64> = meta["board"].as_array().unwrap().iter().map(num).collect();
    let (a, b) = ((bx[2] - bx[0]) * 1e-3, (bx[3] - bx[1]) * 1e-3);
    // the planes (sheets of plane nets) by height, and the plane pairs between neighbours
    let plane_sheets: Vec<&Value> = case["sheets"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|s| matches!(s["net"].as_str(), Some("GND") | Some("PWR")))
        .collect();
    let mut zp: Vec<f64> = plane_sheets.iter().map(|s| num(&s["z"])).collect();
    zp.sort_by(|x, y| y.partial_cmp(x).unwrap());
    zp.dedup_by(|x, y| (*x - *y).abs() < 1e-9);
    let pairs: Vec<(f64, f64)> = zp.windows(2).map(|w| (w[0], w[1])).collect();
    let no_rad = args.iter().any(|x| x == "--no-radiation");
    let suffix = args
        .iter()
        .position(|a| a == "--suffix")
        .and_then(|i| args.get(i + 1))
        .map(|s| format!("_{s}"))
        .unwrap_or_default();
    let skip_grid = args.iter().any(|x| x == "--no-grid");
    let returns = args.iter().any(|x| x == "--returns");
    let layers = case["layers"].as_array().unwrap();
    // the medium of a pair: thickness-weighted εr and tanδ of the layers between its planes (the
    // case's "cavity" for a single pair)
    let medium_of = |za: f64, zb: f64| -> PairMedium {
        let (mut d, mut e, mut t) = (0.0, 0.0, 0.0);
        for l in layers {
            let o = (num(&l["z1"]).min(za) - num(&l["z0"]).max(zb)).max(0.0);
            d += o;
            e += o * num(&l["er"]);
            t += o * l["tand"].as_f64().unwrap_or(0.0);
        }
        let cav = &meta["cavity"];
        let (er, tand) =
            if pairs.len() == 1 && cav.is_object() { (num(&cav["er"]), num(&cav["tand"])) } else { (e / d, t / d) };
        PairMedium { d: (za - zb) * 1e-3, er, tand, f_loss: Some(f_loss), sigma: None, area: 0.0, edges: vec![] }
    };
    // ports of the cavities: every barrel between the planes (signal vias first, then returns)
    let vias = case["vias"].as_array().unwrap();
    let sigs: Vec<&Value> = vias.iter().filter(|v| v["net"].as_str().unwrap_or("SIG") == "SIG").collect();
    let rets: Vec<&Value> = vias.iter().filter(|v| v["net"].as_str() == Some("GND")).collect();
    let ports_mm: Vec<EnvPort> = sigs
        .iter()
        .chain(rets.iter())
        .map(|v| EnvPort { x: num(&v["x"]) - bx[0], y: num(&v["y"]) - bx[1], r: num(&v["r"]) })
        .collect();
    let ports_m: Vec<EnvPort> =
        ports_mm.iter().map(|p| EnvPort { x: p.x * 1e-3, y: p.y * 1e-3, r: p.r * 1e-3 }).collect();
    let cap_r = opt("--cap-r").unwrap_or(0.05);
    // capacitors: meta "caps" with the planes from the case's lumped elements (same order)
    let lumped = case["lumped"].as_array().cloned().unwrap_or_default();
    let caps: Vec<(Load, f64, f64)> = meta["caps"]
        .as_array()
        .map(|cs| {
            cs.iter()
                .enumerate()
                .map(|(i, c)| {
                    let (z1, z0) = lumped.get(i).map_or((zp[0], zp[zp.len() - 1]), |l| {
                        (num(&l["z0"]).max(num(&l["z1"])), num(&l["z0"]).min(num(&l["z1"])))
                    });
                    (
                        Load {
                            at: EnvPort { x: num(&c["at"][0]) - bx[0], y: num(&c["at"][1]) - bx[1], r: cap_r },
                            r_ohm: num(&c["r"]),
                            l: num(&c["l"]),
                            c: c["c"].as_f64(),
                        },
                        z1,
                        z0,
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    let caps_in = |za: f64, zb: f64| -> Vec<Load> {
        caps.iter().filter(|c| (c.1 - za).abs() < 1e-6 && (c.2 - zb).abs() < 1e-6).map(|c| c.0).collect()
    };
    let mut geoms: Vec<ViaGeom> = (0..sigs.len()).map(|k| via_geom_k(&case, k)).collect();
    if returns {
        // return groups: every GND via joins the GND sheets that have copper at it; the capacitors
        // are posts between their two planes
        for (k, g) in geoms.iter_mut().enumerate() {
            let (vx, vy) = (num(&sigs[k]["x"]), num(&sigs[k]["y"]));
            for (p, w) in g.returns.iter_mut().zip(&rets) {
                let (x, y) = (num(&w["x"]), num(&w["y"]));
                let js: Vec<f64> = zp
                    .iter()
                    .copied()
                    .filter(|&z| z >= num(&w["z0"]) - 1e-9 && z <= num(&w["z1"]) + 1e-9)
                    .filter(|&z| {
                        plane_sheets.iter().any(|s| {
                            (num(&s["z"]) - z).abs() < 1e-9 && s["net"].as_str() == Some("GND") && metal_at(s, x, y)
                        })
                    })
                    .collect();
                // its opening in the planes it passes: the hole of that sheet centred on it
                p.open_r = zp
                    .iter()
                    .copied()
                    .filter(|&z| {
                        z >= num(&w["z0"]) - 1e-9
                            && z <= num(&w["z1"]) + 1e-9
                            && !js.iter().any(|&j| (j - z).abs() < 1e-9)
                    })
                    .filter_map(|z| {
                        plane_sheets.iter().filter(|s| (num(&s["z"]) - z).abs() < 1e-9).find_map(|s| {
                            s["holes"].as_array()?.iter().find_map(|h| {
                                let h: Vec<f64> = h.as_array()?.iter().map(num).collect();
                                (h.len() == 3 && (h[0] - x).hypot(h[1] - y) < 1e-6).then_some((z, h[2]))
                            })
                        })
                    })
                    .collect();
                p.joins = Some(js);
            }
            g.caps = caps
                .iter()
                .map(|(l, z1, z0)| CapPost {
                    dx: l.at.x + bx[0] - vx,
                    dy: l.at.y + bx[1] - vy,
                    r: cap_r,
                    z_top: *z1,
                    z_bot: *z0,
                    r_ohm: l.r_ohm,
                    l: l.l,
                    c: l.c,
                })
                .collect();
        }
    }
    let origin = |k: usize| (ports_m[k].x, ports_m[k].y);
    let mut timing = serde_json::Map::new();
    let title = meta["title"].as_str().unwrap_or("");
    // infinite plates (kisi's ladder as it is), via 0
    let t0 = Instant::now();
    let m0 = PbmVia::build(&geoms[0], Mode::Single, Grid::default());
    let leakage: Vec<Value> = [2e9, 5e9, 10e9, 20e9].iter().map(|&fl| json!({"ghz": fl / 1e9, "pairs": m0.pair_leakage(fl).iter().map(|l| json!([l.0, l.1, l.2])).collect::<Vec<_>>()})).collect();
    let s0: Vec<[Cx; 4]> = f.iter().map(|&fk| s_of(&m0.abcd(fk), 50.0)).collect();
    write_s2p(
        &out.join(format!("ladder_inf{suffix}.s2p")),
        &format!("{cid}: kisi's ladder, infinite plates{} ({title})", if returns { ", return groups" } else { "" }),
        &f,
        &s0,
        50.0,
    );
    timing.insert(
        "infinite".into(),
        json!({"ms": t0.elapsed().as_secs_f64() * 1e3, "return_pairs": m0.floating_pairs().0, "return_spans": m0.floating_pairs().1,
            "return_nodes_z_mm_c_ff": m0.return_nodes().iter().map(|&(z, c)| json!([z, (c * 1e16).round() / 10.0])).collect::<Vec<_>>()}),
    );
    // the cavity environments of every pair
    let outline: Option<Vec<[f64; 2]>> =
        meta["plane_outline"].as_array().map(|r| r.iter().map(|p| [num(&p[0]) - bx[0], num(&p[1]) - bx[1]]).collect());
    // the region (outline or board) for the edge radiation
    let mut region = Mask::new(&BBox { min: [0.0, 0.0], max: [bx[2] - bx[0], bx[3] - bx[1]] }, h_grid);
    match &outline {
        Some(ring) => region.fill_rings(&[&ring[..]]),
        None => region.on.iter_mut().for_each(|x| *x = true),
    }
    let with_rad = |mut m: PairMedium| {
        if !no_rad {
            m.area = region.count() as f64 * h_grid * h_grid * 1e-6;
            m.edges = region.edge_runs().iter().map(|l| l * 1e-3).collect();
        }
        m
    };
    let mut kinds: Vec<(&str, Vec<(f64, f64, Rc<PairEnv>)>, f64)> = vec![];
    if outline.is_none() {
        let t = Instant::now();
        let envs = pairs
            .iter()
            .map(|&(za, zb)| {
                let ld: Vec<Load> = caps_in(za, zb)
                    .iter()
                    .map(|l| Load { at: EnvPort { x: l.at.x * 1e-3, y: l.at.y * 1e-3, r: l.at.r * 1e-3 }, ..*l })
                    .collect();
                (
                    za,
                    zb,
                    Rc::new(PairEnv::Rect(RectEnv::new(a, b, with_rad(medium_of(za, zb)), ports_m.clone(), ld, fmax))),
                )
            })
            .collect();
        kinds.push(("rect", envs, t.elapsed().as_secs_f64() * 1e3));
    }
    if !skip_grid {
        let t = Instant::now();
        let envs = pairs
            .iter()
            .map(|&(za, zb)| {
                (
                    za,
                    zb,
                    Rc::new(PairEnv::Grid(GridEnv::new(
                        &region,
                        with_rad(medium_of(za, zb)),
                        &[],
                        &caps_in(za, zb),
                        &ports_mm,
                    ))),
                )
            })
            .collect();
        kinds.push(("grid", envs, t.elapsed().as_secs_f64() * 1e3));
    }
    for (tag, envs, t_env) in kinds {
        let t = Instant::now();
        let links = |k: usize| -> Vec<CavityLink> {
            envs.iter()
                .map(|(za, zb, e)| CavityLink { z_top: *za, z_bot: *zb, env: e.clone(), origin: origin(k) })
                .collect()
        };
        let ms: Vec<PbmVia> = geoms
            .iter()
            .enumerate()
            .map(|(k, g)| PbmVia::build_with(g, Mode::Single, Grid::default(), links(k)))
            .collect();
        assert!(ms.iter().all(|m| m.has_cavity()), "{cid}: the cavity was not linked");
        if ms.len() == 1 {
            let s: Vec<[Cx; 4]> = f.iter().map(|&fk| s_of(&ms[0].abcd(fk), 50.0)).collect();
            write_s2p(
                &out.join(format!("ladder_{tag}{suffix}.s2p")),
                &format!(
                    "{cid}: kisi's ladder with the plane {} ({tag}) ({title})",
                    if pairs.len() == 1 { "pair as a finite cavity" } else { "pairs as finite cavities" }
                ),
                &f,
                &s,
                50.0,
            );
        } else {
            let refs: Vec<&PbmVia> = ms.iter().collect();
            let s: Vec<Vec<Cx>> = f.iter().map(|&fk| coupled_s(&refs, fk, 50.0)).collect();
            write_s4p(
                &out.join(format!("ladder_{tag}{suffix}.s4p")),
                &format!("{cid}: kisi's ladders coupled through the finite cavity ({tag}) ({title})"),
                &f,
                &s,
                50.0,
            );
            // via A alone in the cavity (B's barrel open), as a 2-port
            let s: Vec<[Cx; 4]> = f.iter().map(|&fk| s_of(&ms[0].abcd(fk), 50.0)).collect();
            write_s2p(
                &out.join(format!("ladder_{tag}{suffix}.s2p")),
                &format!("{cid}: via A with the finite cavity ({tag}), B open"),
                &f,
                &s,
                50.0,
            );
        }
        let cells: Vec<Value> = envs
            .iter()
            .map(|(_, _, env)| match &**env {
                PairEnv::Grid(g) => json!(g.cells()),
                PairEnv::Rect(r) => json!(r.modes()),
            })
            .collect();
        timing.insert(tag.into(), json!({"env_ms": t_env, "ladder_ms": t.elapsed().as_secs_f64() * 1e3, "nf": f.len(), "cells_or_modes": cells}));
    }
    println!("{}", serde_json::to_string_pretty(&json!({"case": cid, "timing": timing, "leakage": leakage})).unwrap());
}

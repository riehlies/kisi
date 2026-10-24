//! `kisi-validate models <via_cases dir> <out dir> [--grid uniform|graded|both] [--only k1,k2]`
//!
//! Runs the Rust reduced models on the kisi-fdtd reference cases (validation/fdtd/via_cases):
//! * groups A–D (2 ports): via ladder, single mode → `<out>/<case>/rust_pbm[_graded].s2p`
//! * group E (pairs): via ladder, odd mode (per line, 50 Ω)
//! * group G: cavity model Z matrix → `<out>/<case>/rust_model.s2p` (1 Ω reference)
//! * group H: PDN loop inductance at 50 MHz
//!
//! The case geometry is read the way validation/fdtd/via_pbm.py reads it (same feed-line
//! formulas, same loss treatment), so the Rust and Python results can be compared directly.
//! Prints one JSON object with timings and the per-case results.

use kisi_core::cavity_modal::{loop_inductance, Cavity};
use kisi_core::num::Cx;
use kisi_core::rf::Abcd;
use kisi_core::via_pbm::{clear_cache, equivalent_radius, ray_profile, Feed, Grid, Mode, PbmVia, Plane, Post, Slab, ViaGeom};
use serde_json::{json, Value};
use std::f64::consts::PI;
use std::time::Instant;

fn er_at(layers: &[Value], z: f64) -> (f64, f64) {
    for l in layers {
        let (z0, z1) = (l["z0"].as_f64().unwrap(), l["z1"].as_f64().unwrap());
        if z0 - 1e-9 <= z && z <= z1 + 1e-9 {
            return (l["er"].as_f64().unwrap(), l["tand"].as_f64().unwrap_or(0.0));
        }
    }
    (1.0, 0.0)
}

fn z_microstrip(w: f64, h: f64, er: f64) -> f64 {
    let u = w / h;
    let a = 1.0 + ((u.powi(4) + (u / 52.0).powi(2)) / (u.powi(4) + 0.432)).ln() / 49.0 + (1.0 + (u / 18.1).powi(3)).ln() / 18.7;
    let b = 0.564 * ((er - 0.9) / (er + 3.0)).powf(0.053);
    let ee = (er + 1.0) / 2.0 + (er - 1.0) / 2.0 * (1.0 + 10.0 / u).powf(-a * b);
    let f = 6.0 + (2.0 * PI - 6.0) * (-(30.666 / u).powf(0.7528)).exp();
    60.0 / ee.sqrt() * (f / u + (1.0 + (2.0 / u).powi(2)).sqrt()).ln()
}

/// Cohn, zero thickness, exact elliptic ratio (as via_cases.py)
fn z_stripline_sym(w: f64, b: f64, er: f64) -> f64 {
    let k = 1.0 / (PI * w / (2.0 * b)).cosh();
    let kp = (1.0 - k * k).sqrt();
    let r = if k < 0.7071 { PI / (2.0 * (1.0 + kp.sqrt()) / (1.0 - kp.sqrt())).ln() } else { (2.0 * (1.0 + k.sqrt()) / (1.0 - k.sqrt())).ln() / PI };
    30.0 * PI / er.sqrt() * r
}

fn feed_eeff(w: f64, h: f64, er: f64, layers: &[Value], zt: f64, zref: f64) -> f64 {
    let z0 = |l: &Value| l["z0"].as_f64().unwrap();
    let above = layers.iter().any(|l| z0(l) >= zt.max(zref) - 1e-9);
    if (above && zt < zref) || (zt > zref && layers.iter().any(|l| z0(l) >= zt - 1e-9)) {
        return er;
    }
    let u = w / h;
    let a = 1.0 + ((u.powi(4) + (u / 52.0).powi(2)) / (u.powi(4) + 0.432)).ln() / 49.0 + (1.0 + (u / 18.1).powi(3)).ln() / 18.7;
    let b = 0.564 * ((er - 0.9) / (er + 3.0)).powf(0.053);
    (er + 1.0) / 2.0 + (er - 1.0) / 2.0 * (1.0 + 10.0 / u).powf(-a * b)
}

pub(crate) fn feed_line(layers: &[Value], planes: &[Plane], w: f64, zt: f64, zref: f64) -> (f64, f64) {
    let up: Vec<f64> = planes.iter().map(|p| p.z).filter(|&z| z > zt + 1e-9).collect();
    let dn: Vec<f64> = planes.iter().map(|p| p.z).filter(|&z| z < zt - 1e-9).collect();
    if !up.is_empty() && !dn.is_empty() {
        let za = up.iter().copied().fold(f64::MAX, f64::min);
        let zb = dn.iter().copied().fold(f64::MIN, f64::max);
        let (mut num, mut den) = (0.0, 0.0);
        for l in layers {
            let o = (l["z1"].as_f64().unwrap().min(za) - l["z0"].as_f64().unwrap().max(zb)).max(0.0);
            num += o * l["er"].as_f64().unwrap();
            den += o;
        }
        let er = num / den;
        let (z1, z2) = (z_stripline_sym(w, 2.0 * (za - zt), er), z_stripline_sym(w, 2.0 * (zt - zb), er));
        return (2.0 * z1 * z2 / (z1 + z2), er);
    }
    let h = (zt - zref).abs();
    let er = er_at(layers, 0.5 * (zt + zref)).0;
    (z_microstrip(w, h, er), feed_eeff(w, h, er, layers, zt, zref))
}

/// via_pbm.py geometry(): planes (GND/PWR sheets) with the antipad radius around the via
/// `equivalent`: the antipad as the round opening with the same in-plane capacitance
/// (`via_pbm::equivalent_radius` around a barrel of radius `a`); else the prototype's rule (the
/// hole centred on the via or the slot it lies on, else the nearest edge).
pub(crate) fn planes_at(case: &Value, vx: f64, vy: f64, equivalent: Option<f64>) -> Vec<Plane> {
    let mut planes = vec![];
    for s in case["sheets"].as_array().unwrap() {
        let net = s["net"].as_str().unwrap_or("");
        if net != "GND" && net != "PWR" {
            continue;
        }
        let holes: Vec<Vec<f64>> = s["holes"].as_array().map(|a| a.iter().map(|h| h.as_array().unwrap().iter().map(|x| x.as_f64().unwrap()).collect()).collect()).unwrap_or_default();
        let mut b = None;
        for h in &holes {
            if h.len() < 5 && (h[0] - vx).hypot(h[1] - vy) < 1e-6 {
                b = Some(h[2]);
            } else if h.len() >= 5 {
                let (x0, y0, x1, y1, r) = (h[0], h[1], h[2], h[3], h[4]);
                let (dx, dy) = (x1 - x0, y1 - y0);
                let l2 = dx * dx + dy * dy;
                let t = if l2 > 0.0 { (((vx - x0) * dx + (vy - y0) * dy) / l2).clamp(0.0, 1.0) } else { 0.0 };
                if (vx - x0 - t * dx).hypot(vy - y0 - t * dy) < 1e-6 {
                    b = Some(r);
                }
            }
        }
        if b.is_none() {
            for h in &holes {
                if h.len() < 5 {
                    let d = (h[0] - vx).hypot(h[1] - vy);
                    if d < h[2] {
                        b = Some(h[2] - d);
                    }
                }
            }
        }
        if let (Some(a), Some(_)) = (equivalent, b) {
            let inside = |x: f64, y: f64| {
                holes.iter().any(|h| {
                    if h.len() < 5 {
                        (x - h[0]).hypot(y - h[1]) < h[2]
                    } else {
                        let (dx, dy) = (h[2] - h[0], h[3] - h[1]);
                        let l2 = dx * dx + dy * dy;
                        let t = if l2 > 0.0 { (((x - h[0]) * dx + (y - h[1]) * dy) / l2).clamp(0.0, 1.0) } else { 0.0 };
                        (x - h[0] - t * dx).hypot(y - h[1] - t * dy) < h[4]
                    }
                })
            };
            let rb = ray_profile([vx, vy], 96, 3.0, 0.005, &inside);
            b = Some(equivalent_radius(a, &rb));
        }
        planes.push(Plane { z: s["z"].as_f64().unwrap(), antipad: b });
    }
    planes.sort_by(|a, b| b.z.partial_cmp(&a.z).unwrap());
    planes
}

fn via_geom(case: &Value, mode: Mode, equivalent: bool) -> ViaGeom {
    let vias = case["vias"].as_array().unwrap();
    let sigs: Vec<&Value> = vias.iter().filter(|v| v["net"].as_str().unwrap_or("SIG") == "SIG").collect();
    let v = sigs[0];
    let g = |v: &Value, k: &str| v[k].as_f64().unwrap();
    let (vx, vy) = (g(v, "x"), g(v, "y"));
    let planes = planes_at(case, vx, vy, if equivalent { Some(g(v, "r")) } else { None });
    let layers = case["layers"].as_array().unwrap();
    let ports = case["ports"].as_array().unwrap();
    let ports: Vec<&Value> = if ports.len() == 4 { vec![&ports[0], &ports[2]] } else { ports.iter().collect() };
    let feed = |p: &Value| {
        let (w, z, zr) = (g(p, "y1") - g(p, "y0"), g(p, "z1"), g(p, "z0"));
        let (zc, eeff) = feed_line(layers, &planes, w, z, zr);
        Feed { z, z_ref: zr, w, zc, eeff, dropped: 0.0, antipad_line: true }
    };
    let post = |w: &Value| Post { dx: g(w, "x") - vx, dy: g(w, "y") - vy, r: g(w, "r"), z0: g(w, "z0"), z1: g(w, "z1"), joins: None, open_r: vec![] };
    ViaGeom {
        slabs: layers.iter().map(|l| Slab { z0: g(l, "z0"), z1: g(l, "z1"), er: g(l, "er"), tand: l["tand"].as_f64().unwrap_or(0.0) }).collect(),
        planes: planes.clone(),
        r: g(v, "r"),
        z0: g(v, "z0"),
        z1: g(v, "z1"),
        pads: v["pads"].as_array().unwrap().iter().map(|x| x.as_f64().unwrap()).collect(),
        pad_r: g(v, "pad_r"),
        returns: vias.iter().filter(|w| w["net"].as_str() == Some("GND")).map(post).collect(),
        partner: if mode != Mode::Single && sigs.len() > 1 { Some(post(sigs[1])) } else { None },
        feeds: [feed(ports[0]), feed(ports[1])],
        f_loss: Some(case["f_loss"].as_f64().unwrap_or(case["fmax"].as_f64().unwrap() / 2.0)),
        junction: true,
        sigma: None,
        caps: vec![],
        coplanar: vec![],
    }
}

pub(crate) fn freqs(case: &Value) -> Vec<f64> {
    let fmax = case["fmax"].as_f64().unwrap();
    let nf = case["nf"].as_u64().unwrap_or(400) as usize;
    (1..=nf).map(|k| fmax * k as f64 / nf as f64).collect()
}

pub(crate) fn write_s2p(path: &std::path::Path, title: &str, f: &[f64], s: &[[Cx; 4]], z0: f64) {
    let mut t = format!("! {title}\n# Hz S RI R {z0}\n");
    for (fk, m) in f.iter().zip(s) {
        t += &format!("{fk:.6e}");
        // Touchstone 2-port order: S11 S21 S12 S22
        for c in [m[0], m[2], m[1], m[3]] {
            t += &format!(" {:.9e} {:.9e}", c.re, c.im);
        }
        t += "\n";
    }
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, t).unwrap();
}

pub(crate) fn s_of(m: &Abcd, z0: f64) -> [Cx; 4] {
    let (s11, s21, s22) = m.s_params(z0);
    // S12 from the determinant (= S21 for a reciprocal network)
    let det = m.a * m.d - m.b * m.c;
    let den = m.a + m.b.scale(1.0 / z0) + m.c.scale(z0) + m.d;
    [s11, Cx::new(2.0, 0.0) * det / den, s21, s22]
}

pub fn run(args: &[String]) {
    let dir = std::path::Path::new(&args[0]);
    let out = std::path::Path::new(&args[1]);
    let opt = |k: &str| args.iter().position(|a| a == k).and_then(|i| args.get(i + 1)).cloned();
    let grids: Vec<(&str, Grid)> = match opt("--grid").as_deref() {
        Some("uniform") => vec![("rust_pbm", Grid::Uniform { dr: 0.01, rmax: 3.0 })],
        Some("graded") => vec![("rust_pbm_graded", Grid::default())],
        _ => vec![("rust_pbm", Grid::Uniform { dr: 0.01, rmax: 3.0 }), ("rust_pbm_graded", Grid::default())],
    };
    let only: Vec<String> = opt("--only").map(|s| s.split(',').map(String::from).collect()).unwrap_or_default();
    let mut names: Vec<String> = std::fs::read_dir(dir).unwrap().filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().to_string()).filter(|n| n.ends_with(".json")).collect();
    names.sort();
    let mut res = serde_json::Map::new();
    for fname in names {
        let cid = fname.trim_end_matches(".json").to_string();
        if !only.is_empty() && !only.iter().any(|k| cid.contains(k.as_str())) {
            continue;
        }
        let case: Value = serde_json::from_str(&std::fs::read_to_string(dir.join(&fname)).unwrap()).unwrap();
        let group = case["meta"]["group"].as_str().unwrap_or("");
        let np = case["ports"].as_array().map_or(0, |a| a.len());
        let f = freqs(&case);
        match group {
            "A" | "B" | "C" | "D" | "E" | "X" => {
                if group != "E" && (np != 2 || cid.contains("cap")) {
                    continue;
                }
                let mode = if group == "E" { Mode::Odd } else { Mode::Single };
                let geom0 = via_geom(&case, mode, false);
                let geom_kisi = via_geom(&case, mode, true);
                if std::env::var("KISI_DEBUG_VIA").is_ok() {
                    eprintln!("{}", serde_json::to_string(&geom_kisi).unwrap());
                }
                let mut r = serde_json::Map::new();
                for (tag, grid) in &grids {
                    // the uniform grid runs the prototype's model (parity with via_pbm.py), the
                    // graded grid kisi's (with the junction model)
                    let mut geom = if matches!(grid, Grid::Uniform { .. }) { geom0.clone() } else { geom_kisi.clone() };
                    geom.junction = !matches!(grid, Grid::Uniform { .. });
                    clear_cache();
                    let t0 = Instant::now();
                    let m = PbmVia::build(&geom, mode, *grid);
                    let t_build = t0.elapsed().as_secs_f64();
                    let t1 = Instant::now();
                    let s: Vec<[Cx; 4]> = f.iter().map(|&fk| s_of(&m.abcd(fk), 50.0)).collect();
                    let t_eval = t1.elapsed().as_secs_f64();
                    write_s2p(&out.join(&cid).join(format!("{tag}.s2p")), &format!("{cid}: reduced via model (Rust, {tag})"), &f, &s, 50.0);
                    r.insert(
                        tag.to_string(),
                        json!({"build_ms": t_build * 1e3, "eval_ms": t_eval * 1e3, "nf": f.len(), "laplace_unknowns": m.laplace_n,
                               "nodes": m.nodes.len(), "C_fF": m.cap.iter().map(|c| (c * 1e16).round() / 10.0).collect::<Vec<_>>(),
                               "notch_ghz": m.notch(f[f.len() - 1]).map(|x| x / 1e9), "z_barrel": m.z_barrel(), "delay_ps": m.delay() * 1e12}),
                    );
                }
                res.insert(cid, Value::Object(r));
            }
            "G" => {
                let bx: Vec<f64> = case["board"].as_array().unwrap().iter().map(|x| x.as_f64().unwrap()).collect();
                let lay = &case["layers"][0];
                let d = (lay["z1"].as_f64().unwrap() - lay["z0"].as_f64().unwrap()) * 1e-3;
                let c = Cavity {
                    a: (bx[2] - bx[0]) * 1e-3,
                    b: (bx[3] - bx[1]) * 1e-3,
                    d,
                    er: lay["er"].as_f64().unwrap(),
                    tand: lay["tand"].as_f64().unwrap_or(0.0),
                    f_loss: Some(case["f_loss"].as_f64().unwrap_or(case["fmax"].as_f64().unwrap() / 2.0)),
                    fringe: 0.0,
                    modes: (120, 80),
                };
                let ports: Vec<(f64, f64)> = case["ports"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|p| ((p["x"].as_f64().unwrap() - bx[0]) * 1e-3, ((p["y0"].as_f64().unwrap() + p["y1"].as_f64().unwrap()) / 2.0 - bx[1]) * 1e-3))
                    .collect();
                let w = case["mesh"]["dfine"].as_f64().unwrap() * 1e-3;
                let t0 = Instant::now();
                let p = c.ports(&ports, w);
                let t_build = t0.elapsed().as_secs_f64();
                let t1 = Instant::now();
                let zs: Vec<Vec<Cx>> = f.iter().map(|&fk| c.z(&p, fk)).collect();
                let t_eval = t1.elapsed().as_secs_f64();
                // S = (Z − I)(Z + I)⁻¹ for two ports, 1 Ω
                let s: Vec<[Cx; 4]> = zs
                    .iter()
                    .map(|z| {
                        let one = Cx::new(1.0, 0.0);
                        let (a, b, cc, dd) = (z[0] + one, z[1], z[2], z[3] + one);
                        let det = a * dd - b * cc;
                        let (ia, ib, ic, id) = (dd / det, Cx::new(0.0, 0.0) - b / det, Cx::new(0.0, 0.0) - cc / det, a / det);
                        let (m0, m1, m2, m3) = (z[0] - one, z[1], z[2], z[3] - one);
                        [m0 * ia + m1 * ic, m0 * ib + m1 * id, m2 * ia + m3 * ic, m2 * ib + m3 * id]
                    })
                    .collect();
                write_s2p(&out.join(&cid).join("rust_model.s2p"), &format!("{cid}: cavity model (Rust), 1 ohm"), &f, &s, 1.0);
                res.insert(cid, json!({"cavity": {"build_ms": t_build * 1e3, "eval_ms": t_eval * 1e3, "nf": f.len(), "modes": 120 * 80}}));
            }
            "H" => {
                let bx: Vec<f64> = case["board"].as_array().unwrap().iter().map(|x| x.as_f64().unwrap()).collect();
                let zc: Vec<f64> = case["meta"]["copper_z"].as_array().unwrap().iter().map(|x| x.as_f64().unwrap()).collect();
                let (d1, d2) = (zc[1] - zc[3], zc[3] - zc[4]);
                let d_eff = d1 * d2 / (d1 + d2);
                let c = Cavity { a: (bx[2] - bx[0]) * 1e-3, b: (bx[3] - bx[1]) * 1e-3, d: d_eff * 1e-3, er: 4.0, tand: 0.0, f_loss: None, fringe: 0.0, modes: (200, 60) };
                let lx = case["lumped"][0]["x"].as_f64().unwrap();
                let t0 = Instant::now();
                let l = loop_inductance(&c, ((0.0 - bx[0]) * 1e-3, (0.0 - bx[1]) * 1e-3), 2.4e-3, ((lx + 0.5 - bx[0]) * 1e-3, (0.0 - bx[1]) * 1e-3), 0.2e-3, 50e6);
                let t = t0.elapsed().as_secs_f64();
                res.insert(cid, json!({"pdn_loop": {"L_plane_model_nH": l * 1e9, "ms": t * 1e3}}));
            }
            _ => {}
        }
    }
    println!("{}", serde_json::to_string_pretty(&Value::Object(res)).unwrap());
}

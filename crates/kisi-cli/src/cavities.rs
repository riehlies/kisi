//! `kisi-validate cavities <board.kicad_pcb> [--config F | --no-config] [--variant NAME] [--json FILE]`
//!
//! Survey of the plane-pair cavities the channel vias cross (validation/fdtd/BOARD_RESONANCES.md):
//! for every via of every checked lane, the plane pairs between its entry and exit layer (and in
//! its stub), the region of each pair (connected overlap of both planes), its size and the lowest
//! resonances of its bounding rectangle, the posts that short the pair (stitching vias and plated
//! holes of the plane net) and how close they are, against the return vias the ladder takes
//! (plane-net vias within 3 mm, the nearest 8), and the return group of every pair (the return vias
//! that short it, that float across it, capacitors; validation/fdtd/BOARD_RETURNS.md). Prints a
//! table; `--json` writes the details.

use kisi_core::geom::{self, P};
use kisi_core::profile::Profiles;
use kisi_core::rf::RfParams;
use kisi_core::service::{self, Project};
use kisi_core::via::plane_clearance;
use kisi_core::via_cavity::pair_cavity;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;

fn die(msg: &str) -> ! {
    eprintln!("cavities: {msg}");
    std::process::exit(2);
}

pub fn run(args: &[String]) {
    let pcb = PathBuf::from(&args[0]);
    let opt = |k: &str| args.iter().position(|a| a == k).and_then(|i| args.get(i + 1)).cloned();
    let no_config = args.iter().any(|a| a == "--no-config");
    let cfg = if no_config { None } else { opt("--config").map(PathBuf::from).or_else(|| Some(service::config_path_for(&pcb)).filter(|p| p.is_file())) };
    let variant = opt("--variant");
    let p = Project::load_with(&pcb, None, None, cfg.as_deref(), variant.as_deref()).unwrap_or_else(|e| die(&e));
    let t0 = std::time::Instant::now();
    let rp = RfParams::default();
    let check = service::channel_check(&p, &Profiles::builtin(), &HashMap::new(), None, &rp);
    let board = &p.board;
    let su = board.stackup();
    let lname = |k: u16| su.copper[k as usize].name.clone();
    let mut out_vias: Vec<Value> = vec![];
    let mut seen: Vec<P> = vec![];
    let mut pairs_seen: HashMap<(u16, u16, u32, u32, i64, i64), Value> = HashMap::new();
    let rows = check["rows"].as_array().cloned().unwrap_or_default();
    for row in &rows {
        let lane = row["lane"].as_str().unwrap_or("").to_string();
        let diff = row["diff"].as_bool().unwrap_or(false);
        for nn in row["nets"].as_array().cloned().unwrap_or_default() {
            let Some(name) = nn.as_str() else { continue };
            let Some(net) = p.net_id(name) else { continue };
            let ch = match kisi_core::rf::channel_dir(board, &p.an, net, &p.params.field, p.target(net), &rp, false, None) {
                Ok(c) => c,
                Err(_) => continue,
            };
            for vi in &ch.via_info {
                if seen.iter().any(|s| geom::len(geom::sub(*s, vi.at)) < 0.01) {
                    continue;
                }
                seen.push(vi.at);
                let Some(v) = board.vias.iter().find(|v| geom::len(geom::sub(v.at, vi.at)) < 0.01) else { continue };
                let partner = None;
                let planes: Vec<u16> = (v.from..=v.to).filter(|&k| plane_clearance(board, v.at, k, net, partner).is_some()).collect();
                let (lo, hi) = (vi.layer_in.min(vi.layer_out), vi.layer_in.max(vi.layer_out));
                // ladder returns: plane-net vias within 3 mm, the nearest 8
                let mut rets: Vec<(f64, &kisi_core::board::Via)> = board
                    .vias
                    .iter()
                    .filter(|w| w.net != net && w.net != 0 && board.plane_net.get(w.net as usize).copied().unwrap_or(false))
                    .map(|w| (geom::len(geom::sub(w.at, v.at)), w))
                    .filter(|&(d, _)| d > 0.01 && d < 3.0)
                    .collect();
                rets.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
                rets.truncate(8);
                let mut pv = vec![];
                for w in planes.windows(2) {
                    let (ka, kb) = (w[0], w[1]);
                    let in_path = ka >= lo && kb <= hi;
                    let Some(c) = pair_cavity(board, v.at, ka, kb, net, partner, 60000, 0.25) else { continue };
                    let (a, b) = c.rect();
                    let near: Vec<f64> = {
                        let mut d: Vec<f64> = c.shorts.iter().map(|s| geom::len(geom::sub(s.at, v.at))).filter(|&d| d > 0.01).collect();
                        d.sort_by(|x, y| x.partial_cmp(y).unwrap());
                        d
                    };
                    let ladder_shorts = rets.iter().filter(|(_, w)| w.net == c.nets.0 && w.net == c.nets.1 && w.from <= ka && w.to >= kb).count();
                    let modes: Vec<f64> = c.rect_modes(3).iter().map(|m| (m.0 / 1e7).round() / 100.0).collect();
                    let key = (ka, kb, c.nets.0, c.nets.1, (c.bbox.min[0] * 10.0) as i64, (c.bbox.min[1] * 10.0) as i64);
                    let pitch = if c.shorts.is_empty() { None } else { Some((c.area_mm2 / c.shorts.len() as f64).sqrt()) };
                    pairs_seen.entry(key).or_insert_with(|| {
                        json!({"layers": [lname(ka), lname(kb)], "nets": [board.net_name(c.nets.0), board.net_name(c.nets.1)], "d_mm": c.d_mm, "er": c.er,
                            "rect_mm": [a, b], "area_mm2": c.area_mm2, "fill_ratio": c.area_mm2 / (a * b), "shorts": c.shorts.len(), "caps": c.caps.len(),
                            "short_pitch_mm": pitch, "rect_modes_ghz": modes, "cell_mm": c.mask.h, "vias": 0})
                    });
                    if let Some(e) = pairs_seen.get_mut(&key) {
                        e["vias"] = json!(e["vias"].as_u64().unwrap_or(0) + 1);
                    }
                    pv.push(json!({"layers": [lname(ka), lname(kb)], "in_path": in_path, "nets": [board.net_name(c.nets.0), board.net_name(c.nets.1)],
                        "d_mm": c.d_mm, "rect_mm": [a, b], "area_mm2": c.area_mm2, "shorts": c.shorts.len(), "caps": c.caps.len(),
                        "nearest_short_mm": near.first(), "shorts_within_3mm": near.iter().filter(|&&d| d < 3.0).count(),
                        "shorts_within_10mm": near.iter().filter(|&&d| d < 10.0).count(), "ladder_returns_that_short_it": ladder_shorts,
                        "rect_modes_ghz": modes}));
                }
                // return groups (BOARD_RETURNS.md): per plane pair the return vias that short it (join both
                // planes), that float across it (join planes beyond it on both sides), capacitors within 3 mm
                let groups: Vec<Value> = kisi_core::via::return_groups(board, v, net, partner, [vi.layer_in, vi.layer_out])
                    .iter()
                    .map(|&(ka, kb, na, nb, short, float, capr, caps)| {
                        let (lo2, hi2) = (vi.layer_in.min(vi.layer_out), vi.layer_in.max(vi.layer_out));
                        let kind = if short > 0 { "shorted" } else if float > 0 { "bridged" } else { "open" };
                        json!({"layers": [lname(ka), lname(kb)], "nets": [board.net_name(na), board.net_name(nb)], "in_path": ka >= lo2 && kb <= hi2,
                            "same_net": na == nb, "shorts": short, "floating": float, "capacitive": capr, "caps": caps, "kind": kind})
                    })
                    .collect();
                out_vias.push(json!({"lane": lane, "net": name, "diff": diff, "at": vi.at, "from": lname(vi.layer_in), "to": lname(vi.layer_out),
                    "barrel": [lname(v.from), lname(v.to)], "return_kind": vi.return_kind, "return_mm": vi.return_mm,
                    "ladder_returns": rets.len(), "nearest_return_mm": rets.first().map(|r| r.0), "pairs": pv, "return_groups": groups}));
            }
        }
    }
    let mut cav: Vec<Value> = pairs_seen.into_values().collect();
    cav.sort_by(|a, b| b["vias"].as_u64().cmp(&a["vias"].as_u64()));
    let res = json!({"board": pcb.file_name().map(|s| s.to_string_lossy().to_string()), "lanes": rows.len(), "vias": out_vias.len(),
        "seconds": t0.elapsed().as_secs_f64(), "cavities": cav, "via_list": out_vias});
    // table for people
    println!("{} lanes, {} channel vias, {:.1} s", rows.len(), res["vias"], t0.elapsed().as_secs_f64());
    println!("cavities (plane pairs crossed by channel vias):");
    for c in res["cavities"].as_array().unwrap() {
        println!(
            "  {:>7}-{:<7} {:>10}/{:<10} d {:.3} mm  rect {:5.1} × {:5.1} mm  fill {:.2}  shorts {:4} (pitch {})  caps {:3}  f10/f01/f11 {:?} GHz  vias {}",
            c["layers"][0].as_str().unwrap(),
            c["layers"][1].as_str().unwrap(),
            c["nets"][0].as_str().unwrap(),
            c["nets"][1].as_str().unwrap(),
            c["d_mm"].as_f64().unwrap(),
            c["rect_mm"][0].as_f64().unwrap(),
            c["rect_mm"][1].as_f64().unwrap(),
            c["fill_ratio"].as_f64().unwrap(),
            c["shorts"],
            c["short_pitch_mm"].as_f64().map_or("-".into(), |x| format!("{x:.1} mm")),
            c["caps"],
            c["rect_modes_ghz"].as_array().unwrap().iter().map(|x| x.as_f64().unwrap()).collect::<Vec<_>>(),
            c["vias"]
        );
    }
    println!("vias:");
    for v in res["via_list"].as_array().unwrap() {
        let pairs: Vec<String> = v["pairs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|q| {
                format!(
                    "{}{}-{} near {} ({}≤3mm, {}≤10mm, ladder {})",
                    if q["in_path"].as_bool().unwrap() { "" } else { "stub " },
                    q["layers"][0].as_str().unwrap(),
                    q["layers"][1].as_str().unwrap(),
                    q["nearest_short_mm"].as_f64().map_or("-".into(), |x| format!("{x:.2}")),
                    q["shorts_within_3mm"],
                    q["shorts_within_10mm"],
                    q["ladder_returns_that_short_it"]
                )
            })
            .collect();
        println!("  {:<40} {:>6}>{:<6} ret {:<8} {}", v["net"].as_str().unwrap(), v["from"].as_str().unwrap(), v["to"].as_str().unwrap(), v["return_kind"].as_str().unwrap(), pairs.join("; "));
    }
    // return groups over all channel vias: pairs of one net / of two nets by kind, in the path or a stub
    let mut tally: std::collections::BTreeMap<String, usize> = Default::default();
    for v in res["via_list"].as_array().unwrap() {
        for g in v["return_groups"].as_array().unwrap() {
            let k = format!(
                "{} {} ({})",
                if g["same_net"].as_bool().unwrap() { "pair of one net," } else { "pair of two nets," },
                g["kind"].as_str().unwrap(),
                if g["in_path"].as_bool().unwrap() { "path" } else { "stub" }
            );
            *tally.entry(k).or_default() += 1;
        }
    }
    println!("return groups (via × plane pair):");
    for (k, n) in &tally {
        println!("  {k:<48} {n}");
    }
    let mut res = res;
    res["return_group_tally"] = json!(tally);
    if let Some(f) = opt("--json") {
        std::fs::write(&f, serde_json::to_string_pretty(&res).unwrap()).unwrap();
    }
}

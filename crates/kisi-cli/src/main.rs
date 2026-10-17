//! kisi – command line front end for kisi-core.
//!
//! Usage: kisi <board.kicad_pcb> [--pro <file.kicad_pro>] [--json <out.json>] [--step <mm>] [--nets <filter>]

use kisi_core::extract::{compared_z, Kind};
use kisi_core::{analyze, apply_project, load_pcb, Params};
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("usage: kisi <board.kicad_pcb> [--pro file] [--json out] [--step mm] [--nets filter]");
        std::process::exit(2);
    }
    let pcb_path = &args[1];
    let mut pro_path: Option<String> = None;
    let mut json_out: Option<String> = None;
    let mut filter: Option<String> = None;
    let mut geoms_out: Option<String> = None;
    let mut pi_mode = false;
    let mut pi_config: Option<String> = None;
    let mut list_rails = false;
    let mut field_in: Option<String> = None;
    let mut params = Params::default();
    let mut i = 2;
    while i < args.len() {
        match args[i].as_str() {
            "--pro" => {
                pro_path = args.get(i + 1).cloned();
                i += 1;
            }
            "--json" => {
                json_out = args.get(i + 1).cloned();
                i += 1;
            }
            "--step" => {
                params.step = args.get(i + 1).and_then(|s| s.parse().ok()).unwrap_or(0.5);
                i += 1;
            }
            "--nets" => {
                filter = args.get(i + 1).cloned();
                i += 1;
            }
            "--no-mask" => params.solder_mask = false,
            "--pi" => pi_mode = true,
            "--rails" => list_rails = true,
            "--pi-config" => {
                pi_config = args.get(i + 1).cloned();
                pi_mode = true;
                i += 1;
            }
            "--geoms" => {
                geoms_out = args.get(i + 1).cloned();
                i += 1;
            }
            "--field" => {
                field_in = args.get(i + 1).cloned();
                i += 1;
            }
            _ => {}
        }
        i += 1;
    }
    if pro_path.is_none() {
        let guess = pcb_path.trim_end_matches(".kicad_pcb").to_string() + ".kicad_pro";
        if std::path::Path::new(&guess).exists() {
            pro_path = Some(guess);
        }
    }

    let t0 = Instant::now();
    let text = std::fs::read_to_string(pcb_path).expect("cannot read board file");
    let mut board = match load_pcb(&text) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("parse error: {e}");
            std::process::exit(1);
        }
    };
    let t_parse = t0.elapsed();
    if let Some(p) = &pro_path {
        let s = std::fs::read_to_string(p).expect("cannot read project file");
        if let Err(e) = apply_project(&mut board, &s) {
            eprintln!("warning: project file: {e}");
        }
    }
    if let Some(p) = &field_in {
        // [{key|id, z0?, zodd?, zeven?, eeff?, ...}] as written by tools/solve_xs.mjs
        let v: Vec<serde_json::Value> = serde_json::from_str(&std::fs::read_to_string(p).expect("read field results")).unwrap();
        for r in v {
            let id = r["key"].as_str().or(r["id"].as_str()).unwrap_or("").to_string();
            if r["ok"].as_bool() == Some(false) {
                continue;
            }
            if let Ok(fr) = serde_json::from_value(r) {
                params.field.insert(id, fr);
            }
        }
    }
    if list_rails {
        println!("Versorgungsnetze:");
        for p in kisi_core::pi::power_nets(&board) {
            let ls: Vec<&str> = p.zone_layers.iter().map(|&l| board.stackup().copper[l as usize].name.as_str()).collect();
            println!(
                "  {:<28} {:>6} V  {:>6.0} mm² Zonen auf {:<24} {:>4} Pads  {}",
                p.name,
                p.voltage.map(|v| format!("{v}")).unwrap_or("?".into()),
                p.zone_area,
                ls.join(","),
                p.pads,
                p.components.iter().take(12).cloned().collect::<Vec<_>>().join(" ")
            );
        }
        return;
    }
    if pi_mode {
        let (rails, warns) = match &pi_config {
            Some(p) => {
                let txt = std::fs::read_to_string(p).expect("read config");
                if txt.trim_start().starts_with('[') {
                    // kisi format: [RailSetup, ...]
                    (serde_json::from_str(&txt).expect("rail setup JSON"), vec![])
                } else {
                    kisi_core::pi::rails_from_kipida(&board, &txt).expect("KiPIDA config")
                }
            }
            None => kisi_core::pi::rails_from_fields(&board),
        };
        for w in warns {
            println!("WARNUNG: {w}");
        }
        if rails.is_empty() {
            println!("Keine Versorgung definiert (Felder PI_SOURCE / PI_LOAD oder --pi-config kipida_config.json).");
        }
        for r in &rails {
            let t = Instant::now();
            match kisi_core::pi::solve_rail(&board, r, &kisi_core::pi::PiParams::default()) {
                Ok(res) => {
                    println!(
                        "\n{} {:.3} V: {} Knoten, Raster {} mm, {:.0} ms",
                        res.net, res.voltage, res.nodes, res.cell, t.elapsed().as_secs_f64() * 1e3
                    );
                    println!("  Last gesamt {:.3} A, Verlust {:.1} mW, max. Stromdichte {:.1} A/mm²", res.total_load, res.power_loss_w * 1e3, res.max_j);
                    for s in &res.sources {
                        println!("  Quelle {:<8} {:.3} A ({} Pads)", s.refdes, s.current, s.pads);
                    }
                    for l in &res.loads {
                        println!(
                            "  Last   {:<8} {:.3} A  Vmin {}  Abfall {}",
                            l.refdes,
                            l.current,
                            l.v_min.map(|v| format!("{v:.4} V")).unwrap_or("-".into()),
                            l.v_min.map(|v| format!("{:.1} mV", (res.voltage - v) * 1e3)).unwrap_or("nicht verbunden".into())
                        );
                    }
                    if let Some(v) = res.vias.first() {
                        println!("  höchster Via-Strom {:.3} A bei ({:.2}, {:.2})", v.current, v.at[0], v.at[1]);
                    }
                    for w in &res.warnings {
                        println!("  Hinweis: {w}");
                    }
                }
                Err(e) => println!("\n{}: {}", r.net, e),
            }
        }
        return;
    }
    let t1 = Instant::now();
    let an = analyze(&board, &params);
    if let Some(out) = &geoms_out {
        // geometries sorted by routed length of nets with a target, then total length
        let mut g: Vec<_> = an.geometries.iter().collect();
        g.sort_by(|a, b| b.target_length.partial_cmp(&a.target_length).unwrap().then(b.length.partial_cmp(&a.length).unwrap()));
        let list: Vec<serde_json::Value> = g
            .iter()
            .map(|e| serde_json::json!({"key": e.id, "text": e.text, "length": e.length, "target_length": e.target_length}))
            .collect();
        std::fs::write(out, serde_json::to_string_pretty(&list).unwrap()).expect("write geoms");
        let with_t = an.geometries.iter().filter(|e| e.target_length > 0.0).count();
        println!("{} geometries ({} used by nets with a target) written to {out}", an.geometries.len(), with_t);
    }
    let field_used = an.cross_sections.iter().filter(|x| x.solver == "field").count();
    if field_in.is_some() {
        println!("field results applied to {field_used} of {} cross-sections", an.cross_sections.len());
    }
    let t_an = t1.elapsed();

    let su = board.stackup();
    println!("Board: {}  (KiCad {} / {})", pcb_path, board.version, board.generator);
    println!(
        "  {} tracks, {} vias, {} pads, {} zone fills, {} nets",
        board.tracks.len(),
        board.vias.len(),
        board.pads.len(),
        board.zones.len(),
        board.nets.len()
    );
    println!("  parse {:.0} ms, analysis {:.0} ms, {} samples -> {} unique cross-sections",
        t_parse.as_secs_f64() * 1e3, t_an.as_secs_f64() * 1e3, an.samples, an.unique_cross_sections);
    println!("Stackup{}:", if su.from_file { "" } else { " (DEFAULT)" });
    for (k, c) in su.copper.iter().enumerate() {
        println!("  {:<8} Cu {:.4} mm", c.name, c.thickness);
        if let Some(d) = su.dielectrics.get(k) {
            println!("    {:<10} {:<12} h={:.4} mm  er={:.2}  tand={:.4}", d.kind, d.material, d.thickness, d.epsilon_r, d.loss_tangent);
        }
    }
    for w in &board.warnings {
        println!("WARNING: {w}");
    }
    println!("\nNet classes:");
    for c in &an.classes {
        let t = c
            .target
            .map(|t| format!("{:.0} Ω {}", t.z, if t.diff { "diff" } else { "SE" }))
            .unwrap_or("-".into());
        println!("  {:<16} {:>4} nets  target {:<12} {}", c.name, c.nets, t, c.target_source);
    }

    println!("\nNets with target (compared value, length-weighted):");
    println!("  {:<44} {:<14} {:>7} {:>6} {:>6} {:>6} {:>6} {:>6}", "net", "class", "len mm", "target", "min", "mean", "max", "±5%");
    for n in &an.nets {
        if n.target.is_none() {
            continue;
        }
        if let Some(f) = &filter {
            if !n.name.contains(f.as_str()) && !n.class.contains(f.as_str()) {
                continue;
            }
        }
        let f = |v: Option<f64>| v.map(|x| format!("{x:.1}")).unwrap_or("-".into());
        println!(
            "  {:<44} {:<14} {:>7.1} {:>6.0} {:>6} {:>6} {:>6} {:>5}%",
            truncate(&n.name, 44),
            truncate(&n.class, 14),
            n.length,
            n.target.unwrap().z,
            f(n.z_min),
            f(n.z_mean),
            f(n.z_max),
            n.within5.map(|x| format!("{:.0}", x * 100.0)).unwrap_or("-".into())
        );
    }

    // Distribution of cross-section kinds
    let mut kinds = std::collections::BTreeMap::new();
    for pc in an.pieces.chunks_exact(4) {
        let tr = &board.tracks[pc[0] as usize];
        let l = kisi_core::geom::len(kisi_core::geom::sub(tr.b, tr.a)) * (pc[2] - pc[1]);
        let xs = &an.cross_sections[pc[3] as usize];
        *kinds.entry(format!("{:?}", xs.kind)).or_insert(0.0) += l;
        let _ = compared_z(xs, None);
    }
    println!("\nRouted length by cross-section type:");
    for (k, l) in kinds {
        println!("  {:<20} {:>9.1} mm", k, l);
    }
    let _ = Kind::Microstrip;

    if let Some(f) = &filter {
        // distinct cross-sections (by routed length) of matching nets
        let mut m: std::collections::HashMap<usize, f64> = Default::default();
        for pc in an.pieces.chunks_exact(4) {
            let tr = &board.tracks[pc[0] as usize];
            let name = board.net_name(tr.net);
            if !name.contains(f.as_str()) && !board.class_of(tr.net).name.contains(f.as_str()) {
                continue;
            }
            let l = kisi_core::geom::len(kisi_core::geom::sub(tr.b, tr.a)) * (pc[2] - pc[1]);
            *m.entry(pc[3] as usize).or_default() += l;
        }
        let mut v: Vec<_> = m.into_iter().collect();
        v.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        println!("\nCross-sections for '{f}' (top 12 by length):");
        for (xi, l) in v.iter().take(12) {
            let x = &an.cross_sections[*xi];
            let o = |v: Option<f64>| v.map(|x| format!("{x:.4}")).unwrap_or("-".into());
            println!(
                "  {:>7.1} mm  {:?} L{} w={:.4} up={:?}/{} dn={:?}/{} erU={} erD={} sL={} sR={} gap={} mask={} Z0={} Zd={} flags={}",
                l, x.kind, x.layer, x.w, x.ref_up, o(x.h_up), x.ref_dn, o(x.h_dn), o(x.er_up), o(x.er_dn),
                o(x.s_left), o(x.s_right), o(x.diff_gap), x.mask, o(x.z0), o(x.zdiff), x.flags
            );
        }
    }

    if let Some(out) = json_out {
        let s = serde_json::to_string(&an).unwrap();
        std::fs::write(&out, s).expect("write json");
        println!("\nJSON written to {out}");
    }
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let k: String = s.chars().rev().take(n - 1).collect::<Vec<_>>().into_iter().rev().collect();
        format!("…{k}")
    }
}

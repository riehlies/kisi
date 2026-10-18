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
    let mut pi_tree: Option<String> = None;
    let mut rf_net: Option<String> = None;
    let mut s2p_out: Option<String> = None;
    let mut eye_gbps: Option<f64> = None;
    let mut pdn_net: Option<String> = None;
    let mut stackup_ov: Option<String> = None;
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
            "--rf" => {
                rf_net = args.get(i + 1).cloned();
                i += 1;
            }
            "--stackup" => {
                stackup_ov = args.get(i + 1).cloned();
                i += 1;
            }
            "--pdn" => {
                pdn_net = args.get(i + 1).cloned();
                i += 1;
            }
            "--eye" => {
                eye_gbps = args.get(i + 1).and_then(|x| x.parse::<f64>().ok());
                i += 1;
            }
            "--s2p" => {
                s2p_out = args.get(i + 1).cloned();
                i += 1;
            }
            "--pi-tree" => {
                pi_tree = args.get(i + 1).cloned();
                i += 1;
            }
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
    if let Some(p) = &stackup_ov {
        // {"dielectrics": [{"index": 0, "epsilon_r": 3.6, "loss_tangent": 0.009, "thickness": 0.11}], "mask_epsilon_r": 3.8}
        let ov: kisi_core::board::StackupOverride = serde_json::from_str(&std::fs::read_to_string(p).expect("read stackup overrides")).expect("stackup JSON");
        let su = board.stackup().with_overrides(&ov);
        board.stackup = Some(su);
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
    if let Some(name) = &pdn_net {
        let setups: Vec<kisi_core::pi::RailSetup> = match &pi_tree {
            Some(p) => {
                let spec: kisi_core::tree::TreeSpec = serde_json::from_str(&std::fs::read_to_string(p).expect("read tree")).expect("tree JSON");
                kisi_core::tree::solve_tree(&spec, Some(&board)).map(|t| t.setups).unwrap_or_default()
            }
            None => kisi_core::pi::rails_from_fields(&board).0,
        };
        let setup = setups.iter().find(|r| r.net == *name || r.net.rsplit('/').next() == Some(name.as_str())).cloned().unwrap_or_else(|| kisi_core::pi::RailSetup {
            net: name.clone(),
            voltage: kisi_core::pi::voltage_from_name(name).unwrap_or(0.0),
            ..Default::default()
        });
        match kisi_core::pdn::pdn(&board, &setup, &kisi_core::pdn::PdnParams::default()) {
            Ok(r) => {
                println!("PDN {} {:+.2} V, Last {:.3} A, Ziel {}", r.net, r.voltage, r.load_current,
                    r.target_ohm.map(|t| format!("{:.1} mΩ", t * 1e3)).unwrap_or("–".into()));
                for pl in &r.planes {
                    println!("  Ebene L{}↔L{} {:.0} mm², Abstand {:.3} mm, {:.2} nF", pl.layer, pl.gnd_layer, pl.area_mm2, pl.d_mm, pl.c * 1e9);
                }
                if let Some(f) = r.cavity_hz {
                    println!("  erste Hohlraumresonanz ≈ {:.0} MHz (darüber gilt das konzentrierte Modell nicht)", f / 1e6);
                }
                let mut groups: Vec<(String, &str, usize, f64, f64, f64)> = Vec::new();
                for c in &r.caps {
                    let key = format!("{:.3e}", c.c);
                    let lt = c.esl + c.l_mount + c.l_spread;
                    match groups.iter_mut().find(|g| g.0 == key && g.1 == c.package) {
                        Some(g) => {
                            g.2 += 1;
                            g.3 += lt;
                            g.5 += c.srf;
                        }
                        None => groups.push((key, c.package, 1, lt, c.c, c.srf)),
                    }
                }
                for g in &groups {
                    println!("  {:>3} × {:>9.3} µF {:<5} L gesamt Ø {:.2} nH, SRF Ø {:.1} MHz", g.2, g.4 * 1e6, g.1, g.3 / g.2 as f64 * 1e9, g.5 / g.2 as f64 / 1e6);
                }
                println!("  C gesamt {:.1} µF (Ebene {:.2} nF)", r.c_total * 1e6, r.c_plane * 1e9);
                let at = |f: f64| r.freq.iter().position(|&x| x >= f).map(|i| r.z[i]).unwrap_or(f64::NAN);
                println!("  |Z|: {}", [1e3, 1e4, 1e5, 1e6, 1e7, 1e8, 1e9].iter().map(|&f| format!("{} {:.1} mΩ", if f >= 1e6 { format!("{:.0} MHz", f / 1e6) } else { format!("{:.0} kHz", f / 1e3) }, at(f) * 1e3)).collect::<Vec<_>>().join(", "));
                if !r.cav_z.is_empty() {
                    let at = |f: f64| r.cav_freq.iter().position(|&x| x >= f).map(|i| r.cav_z[i]).unwrap_or(f64::NAN);
                    let (k, zmax) = r.cav_z.iter().enumerate().fold((0, 0.0), |a, (i, &z)| if z > a.1 { (i, z) } else { a });
                    println!("  Ebenenmodell ({} Zellen à {:.2} mm): {}; Maximum {:.1} mΩ bei {:.0} MHz", r.cav_cells, r.cav_h_mm,
                        [1e6, 1e7, 1e8, 1e9].iter().map(|&f| format!("{:.0} MHz {:.1} mΩ", f / 1e6, at(f) * 1e3)).collect::<Vec<_>>().join(", "),
                        zmax * 1e3, r.cav_freq[k] / 1e6);
                }
                for (a, b) in &r.above_target {
                    println!("  über Ziel: {:.2} … {:.2} MHz", a / 1e6, b / 1e6);
                }
                for w in &r.warnings {
                    println!("  Hinweis: {w}");
                }
            }
            Err(e) => eprintln!("PDN: {e}"),
        }
        return;
    }
    if let Some(p) = &pi_tree {
        let spec: kisi_core::tree::TreeSpec = if p == "fields" {
            match kisi_core::tree::tree_from_fields(&board) {
                Some((s, w)) => {
                    for x in w {
                        println!("Hinweis: {x}");
                    }
                    s
                }
                None => {
                    eprintln!("keine PI_REG-Felder auf der Platine");
                    std::process::exit(1);
                }
            }
        } else {
            serde_json::from_str(&std::fs::read_to_string(p).expect("read tree")).expect("tree JSON")
        };
        let t = match kisi_core::tree::solve_tree(&spec, Some(&board)) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("Versorgungsbaum: {e}");
                std::process::exit(1);
            }
        };
        let short = |n: &str| n.rsplit('/').next().unwrap_or(n).replace("{slash}", "/");
        println!("Versorgungsbaum {}", t.name);
        // tree walk from the inputs
        fn walk(net: &str, t: &kisi_core::tree::TreeResult, ind: usize, short: &dyn Fn(&str) -> String) {
            let r = t.rails.iter().find(|r| r.net == net).unwrap();
            println!("{:ind$}{} {:+.2} V  {:.3} A ({:.3} A Lasten)", "", short(net), r.voltage, r.i_total, r.i_loads, ind = ind);
            for e in t.elements.iter().filter(|e| e.from.as_deref() == Some(net)) {
                println!(
                    "{:ind$}  └ {:<10} {:<8} {:<14} {:.3} A → {:.3} A, Verlust {:.0} mW{}",
                    "",
                    e.id,
                    e.kind,
                    e.part,
                    e.i_in,
                    e.i_out,
                    e.p_loss * 1e3,
                    if e.overload { "  ÜBERLAST" } else { "" },
                    ind = ind
                );
                walk(&e.to, t, ind + 6, short);
            }
        }
        for e in t.elements.iter().filter(|e| e.kind == "input") {
            println!("Eingang {} ({})", e.refdes, e.part);
            walk(&e.to, &t, 2, &short);
        }
        println!("\nBauteile (Verbrauch / Verlust in mW):");
        for c in &t.components {
            if c.consumption + c.loss < 0.0005 {
                continue;
            }
            println!("  {:<10} {:<16} {:>7.0} {:>7.0}  {}", c.refdes, c.part, c.consumption * 1e3, c.loss * 1e3, c.role);
        }
        println!(
            "\nEingang {:.2} W = Lasten {:.2} W + Verluste {:.2} W  (Wirkungsgrad {:.0} %)",
            t.p_input,
            t.p_loads,
            t.p_losses,
            100.0 * t.p_loads / t.p_input.max(1e-9)
        );
        for w in &t.warnings {
            println!("WARNUNG: {w}");
        }
        let mut cu = 0.0;
        let mut worst: Option<(String, f64)> = None;
        let mut fails = Vec::new();
        for s in &t.setups {
            match kisi_core::pi::solve_rail(&board, s, &kisi_core::pi::PiParams::default()) {
                Ok(r) => {
                    cu += r.power_loss_w;
                    if let Some(d) = r.max_drop_mv {
                        if worst.as_ref().is_none_or(|w| d > w.1) {
                            worst = Some((r.net.clone(), d));
                        }
                    }
                    for w in &r.warnings {
                        if w.contains("Pads") || w.contains("keine Verbindung") {
                            fails.push(format!("{}: {w}", short(&r.net)));
                        }
                    }
                }
                Err(e) => fails.push(format!("{}: {e}", short(&s.net))),
            }
        }
        println!("Spannungsabfall: {} Schienen, Kupferverluste {:.0} mW, größter Abfall {}", t.setups.len(), cu * 1e3,
            worst.map(|w| format!("{:.1} mV auf {}", w.1, short(&w.0))).unwrap_or("-".into()));
        for f in fails {
            println!("  Hinweis: {f}");
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
    if let Some(pat) = &rf_net {
        let rp = kisi_core::rf::RfParams::default();
        let nets: Vec<u32> = (0..board.nets.len() as u32).filter(|&n| board.net_name(n).ends_with(pat.as_str()) || board.net_name(n) == pat).collect();
        if nets.is_empty() {
            eprintln!("kein Netz endet auf '{pat}'");
        }
        for net in nets {
            let cls = board.net_class.get(net as usize).copied().unwrap_or(0) as usize;
            let target = an.classes.get(cls).and_then(|c| c.target).map(|t| (t.z, t.diff));
            match kisi_core::rf::channel(&board, &an, net, &params.field, target, &rp) {
                Ok(ch) => {
                    println!(
                        "\nKanal {}{}: {:.1} mm, {} Vias, Laufzeit {:.0} ps, Bezug {:.1} Ω, Feldlöser-Anteil {:.0} %",
                        ch.name,
                        if ch.diff { " (differentiell, ungerade Mode)" } else { "" },
                        ch.length_mm,
                        ch.vias,
                        ch.delay_ps,
                        ch.z_ref,
                        ch.field_share * 100.0
                    );
                    println!("  Einfügedämpfung: {}", ch.il_marks.iter().map(|(f, d)| format!("{f} GHz {d:.2} dB")).collect::<Vec<_>>().join(", "));
                    let rl_max = ch.s11_db.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
                    println!("  Rückflussdämpfung schlechtester Wert: {:.1} dB", -rl_max);
                    for (f, c, d) in &ch.loss_db_per_in_at {
                        println!("  Leitungsverlust {f} GHz: Leiter {c:.3} dB/in, Dielektrikum {d:.3} dB/in");
                    }
                    let (zmin, zmax) = ch.tdr_z.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |a, &z| (a.0.min(z), a.1.max(z)));
                    println!("  TDR ({:.0} ps Anstieg): Z min {:.1} Ω, max {:.1} Ω", rp.rise * 1e12, zmin, zmax);
                    println!("  Abschnitte: {}", ch.segments.iter().map(|s| if s.kind == "via" { "Via".to_string() } else { format!("{:.1}mm@{:.0}Ω", s.len_mm, s.z.unwrap_or(0.0)) }).collect::<Vec<_>>().join(" "));
                    let ln = |k: u16| board.stackup().copper.get(k as usize).map(|c| c.name.clone()).unwrap_or_default();
                    if let Some(g) = eye_gbps {
                        let ep = kisi_core::eye::EyeParams { bitrate: g * 1e9, rise: rp.rise, ..Default::default() };
                        match kisi_core::eye::eye(&ch, &ep) {
                            Ok(e) => println!(
                                "  Auge {:.2} Gbit/s PRBS7: Höhe {:.0} % des Hubs, Breite {:.1} ps ({:.0} % UI), Hauptcursor {:.2}, ISI-Summe {:.2}{}",
                                g, e.height * 100.0, e.width_ps, e.width_ps / e.ui_ps * 100.0, e.main_cursor, e.isi_sum,
                                e.warnings.iter().map(|w| format!(" – {w}")).collect::<String>()
                            ),
                            Err(e) => println!("  Auge: {e}"),
                        }
                    }
                    for v in &ch.via_info {
                        println!(
                            "  Via ({:.2},{:.2}) {}→{}  Bohrung {:.2}/{:.2} mm, Antipad {}  Z {:.1} Ω{}  Hülse {:.2} mm, Stub {:.2} mm{}{}  Rückstrom {}",
                            v.at[0], v.at[1], ln(v.layer_in), ln(v.layer_out), v.drill_mm, v.pad_mm,
                            v.antipad_mm.map(|a| format!("{a:.2} mm")).unwrap_or("–".into()),
                            v.z_via, if ch.diff { " diff" } else { "" },
                            v.barrel_mm, v.stub_mm,
                            v.stub_f_ghz.map(|f| format!(" ({f:.1} GHz)")).unwrap_or_default(),
                            if v.backdrilled { " rückgebohrt" } else { "" },
                            match v.return_kind { "stitch" => format!("Stitching {:.2} mm, {:.2} nH", v.return_mm.unwrap_or(0.0), v.l_ret_nh), k => k.to_string() }
                        );
                    }
                    for w in &ch.warnings {
                        println!("  Hinweis: {w}");
                    }
                    if let Some(out) = &s2p_out {
                        let fname = if out.contains("{}") { out.replace("{}", &ch.name.replace('/', "_")) } else { out.clone() };
                        std::fs::write(&fname, kisi_core::rf::touchstone(&ch)).expect("write s2p");
                        println!("  Touchstone: {fname}");
                    }
                    let xt = kisi_core::rf::crosstalk(&board, &an, net, &params.field, &rp);
                    println!("  Übersprechen ({:.0} ps): NEXT Summe {:.2} %, FEXT Summe {:.2} %", xt.rise_ps, xt.next_total * 100.0, xt.fext_total * 100.0);
                    for a in xt.aggressors.iter().take(6) {
                        println!(
                            "    {:<36} {:>6.1} mm parallel, Abstand min {:.3} mm, NEXT {:.2} %, FEXT {:.2} % ({})",
                            a.name.rsplit('/').next().unwrap_or(&a.name),
                            a.coupled_mm,
                            a.gap_min_mm,
                            a.next * 100.0,
                            a.fext * 100.0,
                            a.source
                        );
                    }
                }
                Err(e) => println!("{}: {e}", board.net_name(net)),
            }
        }
        return;
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

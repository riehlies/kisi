//! `kisi-validate via-case`: a board via as a kisi-fdtd case with its 2×thru fixtures – the
//! exporter is `kisi_core::viacase` (shared with the 3D field view of the viewer's "Fields…").
//!
//! ```text
//! kisi-validate via-case <board.kicad_pcb> --net NAME --out DIR
//!     [--at X,Y | --via K] [--layers IN,OUT] [--pro F] [--config F | --no-config] [--variant NAME]
//!     [--keep 1.2] [--keep-cell 0.05] [--rotate DEG] [--feed 4.0] [--margin 2.5] [--air 1.0]
//!     [--fmax 40e9] [--dfine 0.035] [--name ID] [--list] [--scan] [--sweep]
//! ```
//!
//! `--sweep`: the exporter on every via of a signal net, as the viewer's "Fields…" calls it (the layers of kisi's
//! via model in the net's channel where the via is on it, else the two layers its tracks join; keep 1.2, feed 2.5,
//! margin 1.5 mm unless given): one JSON line per via with the result or the reason (`tools/golden.py viacase`).
//!
//! Writes `DIR/<name>.json` (the case) and the 2×thru fixture of each side,
//! `DIR/thru_<name>_in.json` and `DIR/thru_<name>_out.json`.

use kisi_core::service::Project;
use kisi_core::viacase::{self, Opts};
use serde_json::Value;
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
    /// assembly variant (KiCad 10 / kisi.json) to load the board with
    variant: Option<String>,
    out: Option<PathBuf>,
    list: bool,
    scan: bool,
    /// every signal via with the viewer's layers and options: one JSON line per via (tools/golden.py viacase)
    sweep: bool,
    /// what to cut out (kisi_core::viacase)
    v: Opts,
}

fn parse(args: &[String]) -> Opt {
    let mut o = Opt {
        pcb: PathBuf::new(),
        pro: None,
        config: None,
        no_config: false,
        variant: None,
        out: None,
        list: false,
        scan: false,
        sweep: false,
        v: Opts::default(),
    };
    let mut i = 0;
    let num = |s: &str| -> f64 { s.parse().unwrap_or_else(|_| die(&format!("not a number: {s}"))) };
    while i < args.len() {
        let a = args[i].as_str();
        let val = || args.get(i + 1).cloned().unwrap_or_else(|| die(&format!("{a} needs a value")));
        match a {
            "--net" => o.v.net = val(),
            "--pro" => o.pro = Some(val().into()),
            "--config" => o.config = Some(val().into()),
            "--variant" => o.variant = Some(val()),
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
            "--sweep" => {
                o.sweep = true;
                i += 1;
                continue;
            }
            "--preview" => {
                o.v.preview = true;
                i += 1;
                continue;
            }
            "--at" => {
                let v: Vec<f64> = val().split(',').map(num).collect();
                o.v.at = Some([v[0], v[1]]);
            }
            "--via" => o.v.via_k = Some(num(&val()) as usize),
            "--layers" => {
                let v = val();
                let (a, b) = v.split_once(',').unwrap_or_else(|| die("--layers IN,OUT"));
                o.v.layers = Some((a.to_string(), b.to_string()));
            }
            "--keep" => o.v.keep = num(&val()),
            "--keep-cell" => o.v.keep_cell = num(&val()),
            "--rotate" => o.v.rotate = Some(num(&val())),
            "--feed" => o.v.feed = num(&val()),
            "--margin" => o.v.margin = num(&val()),
            "--air" => o.v.air = num(&val()),
            "--fmax" => o.v.fmax = num(&val()),
            "--dfine" => o.v.dfine = num(&val()),
            "--name" => o.v.name = Some(val()),
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
    if o.pcb.as_os_str().is_empty() || (o.v.net.is_empty() && !o.scan && !o.sweep) {
        die("usage: kisi-validate via-case <board.kicad_pcb> --net NAME --out DIR [options], see viacase.rs");
    }
    o
}

pub fn run(args: &[String]) {
    let mut o = parse(args);
    let cfg = if o.no_config {
        None
    } else {
        o.config.clone().or_else(|| Some(kisi_core::service::config_path_for(&o.pcb)).filter(|p| p.is_file()))
    };
    let p = Project::load_with(&o.pcb, o.pro.as_deref(), None, cfg.as_deref(), o.variant.as_deref())
        .unwrap_or_else(|e| die(&e));
    if o.scan {
        viacase::scan(&p.board).iter().for_each(|l| println!("{l}"));
        return;
    }
    if o.list {
        viacase::list(&p.board, &o.v.net).unwrap_or_else(|e| die(&e)).iter().for_each(|l| println!("{l}"));
        return;
    }
    o.v.board_file = o.pcb.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
    o.v.with_config = cfg.is_some();
    if o.sweep {
        return sweep(&p, &o);
    }
    let r = viacase::export(&p.board, &p.an, &o.v).unwrap_or_else(|e| die(&e.to_string()));
    let out = o.out.clone().unwrap_or_else(|| PathBuf::from("."));
    std::fs::create_dir_all(&out).unwrap_or_else(|e| die(&e.to_string()));
    write(&out, &r.name, &r.case);
    for (name, th) in &r.fixtures(&p.board, &o.v) {
        write(&out, name, th);
    }
    for n in &r.notes {
        eprintln!("via-case: {n}");
    }
}

fn write(dir: &Path, name: &str, v: &Value) {
    let p = dir.join(format!("{name}.json"));
    std::fs::write(&p, serde_json::to_string(v).unwrap()).unwrap_or_else(|e| die(&format!("{}: {e}", p.display())));
    eprintln!("via-case: wrote {}", p.display());
}

/// `--sweep`: every signal via, its layers from kisi's channel of its net where it lies on it (the viewer's "Fields…"
/// passes those), the export's result or reason as one JSON line.
fn sweep(p: &Project, o: &Opt) {
    use kisi_core::{geom, rf};
    let b = &p.board;
    let su = b.stackup();
    let lname = |k: u16| su.copper[k as usize].name.clone();
    // the viewer's options (data/field_views.json "fdtd.board") unless given
    let given = |a: &str| std::env::args().any(|x| x == a);
    let mut v = o.v.clone();
    if !given("--feed") {
        v.feed = 2.5;
    }
    if !given("--margin") {
        v.margin = 1.5;
    }
    v.board_file = o.pcb.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
    v.with_config = p.config_path.is_some();
    // kisi's via model on each net's channel: at → (layer in, layer out)
    let rp = rf::RfParams::default();
    let mut model: Vec<(geom::P, u16, u16)> = Vec::new();
    let mut nets: Vec<u32> = b.vias.iter().map(|w| w.net).filter(|&n| n != 0).collect();
    nets.sort();
    nets.dedup();
    for &n in &nets {
        if b.plane_net.get(n as usize).copied().unwrap_or(false) {
            continue;
        }
        if let Ok(ch) = rf::channel(b, &p.an, n, &p.params.field, p.target(n), &rp) {
            model.extend(ch.via_info.iter().map(|i| (i.at, i.layer_in, i.layer_out)));
        }
    }
    for w in &b.vias {
        if w.net == 0 || b.plane_net.get(w.net as usize).copied().unwrap_or(false) {
            continue;
        }
        let m = model.iter().find(|m| geom::len(geom::sub(m.0, w.at)) < 1e-3);
        let mut vo = v.clone();
        vo.net = b.net_name(w.net).to_string();
        vo.at = Some(w.at);
        vo.layers = m.map(|m| (lname(m.1), lname(m.2)));
        let r = viacase::export(b, &p.an, &vo);
        let line = serde_json::json!({
            "net": vo.net, "class": b.class_of(w.net).name, "at": [w.at[0], w.at[1]],
            "layers": vo.layers.as_ref().map(|l| [l.0.clone(), l.1.clone()]), "model": m.is_some(),
            "ok": r.is_ok(), "code": r.as_ref().err().map(|e| e.code), "reason": r.as_ref().err().map(|e| e.msg.clone()),
            "notes": r.as_ref().map(|x| x.notes.clone()).unwrap_or_default(),
            "pad_ports": r.as_ref().map(|x| x.case["meta"]["ports"].as_array().map(|p| p.iter().filter(|q| q.get("pad").is_some()).count()).unwrap_or(0)).unwrap_or(0),
            "sheets": r.as_ref().map(|x| x.case["sheets"].as_array().map(|a| a.len()).unwrap_or(0)).unwrap_or(0),
        });
        println!("{line}");
    }
}

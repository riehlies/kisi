//! `kisi-validate via-case`: a board via as a kisi-fdtd case with its 2×thru fixtures – the
//! exporter is `kisi_core::viacase` (shared with the 3D field view of the viewer's "Fields…").
//!
//! ```text
//! kisi-validate via-case <board.kicad_pcb> --net NAME --out DIR
//!     [--at X,Y | --via K] [--layers IN,OUT] [--pro F] [--config F | --no-config] [--variant NAME]
//!     [--keep 1.2] [--keep-cell 0.05] [--rotate DEG] [--feed 4.0] [--margin 2.5] [--air 1.0]
//!     [--fmax 40e9] [--dfine 0.035] [--name ID] [--list] [--scan]
//! ```
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
    if o.pcb.as_os_str().is_empty() || (o.v.net.is_empty() && !o.scan) {
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
    let r = viacase::export(&p.board, &p.an, &o.v).unwrap_or_else(|e| die(&e));
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

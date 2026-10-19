//! `kisi <command> <board> …`: the analyses as JSON on stdout – the interface for
//! scripts and agents (see the skills in claude-plugin/). Every result says which
//! models and which `<board>.kisi.json` were used.

use kisi_core::profile::Profiles;
use kisi_core::rf::{RfParams, Roughness};
use kisi_core::service::{self, Project};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub const COMMANDS: &[&str] = &["overview", "interfaces", "net", "check", "profiles", "power", "report", "gaps", "config-check", "config-template", "help"];

pub const HELP: &str = "kisi <command> <board.kicad_pcb> [options] – JSON on stdout

  overview         stackup, impedance per net class (share within ±5 %, worst nets)
  interfaces       buses and device-to-device lanes with profiles   [--class CLASS=PROFILE]…
  net <NET>        one net: impedance along the path, loss, TDR, vias, crosstalk
                   [--rise-ps 35] [--roughness smooth|hvlp|standard]
  check            channel check of all lanes against interface profiles
                   [--class CLASS=PROFILE]… [--nets A,B] [--all] [--max 30]
  profiles [ID]    interface profiles (no board needed)
  power            regulator chain, DC IR drop per rail   [--tree tree.json] [--pdn RAIL]
  report           write analysis/kisi.json next to the board   [--out FILE]
  gaps             what the analyses miss, where to find it, how to write it into <board>.kisi.json
  config-check     check <board>.kisi.json (format, ranges, references, sources)
  config-template  a <board>.kisi.json skeleton with this board's dielectrics and net classes

common options: --pro FILE (default: board.kicad_pro), --config FILE (default: board.kisi.json
next to the board), --no-config, --stackup FILE (stackup changes, as in the viewer)
  kisi file.ibs    linear models of an IBIS file (experimental, not working yet)";

#[derive(Clone)]
struct Opts {
    pos: Vec<String>,
    pro: Option<PathBuf>,
    config: Option<PathBuf>,
    no_config: bool,
    stackup: Option<PathBuf>,
    classes: HashMap<String, String>,
    nets: Option<Vec<String>>,
    all: bool,
    max: usize,
    rise_ps: Option<f64>,
    roughness: Option<String>,
    tree: Option<PathBuf>,
    pdn: Option<String>,
    out: Option<PathBuf>,
}

fn parse_opts(args: &[String]) -> Result<Opts, String> {
    let mut o = Opts { pos: vec![], pro: None, config: None, no_config: false, stackup: None, classes: HashMap::new(), nets: None, all: false, max: 30, rise_ps: None, roughness: None, tree: None, pdn: None, out: None };
    let mut i = 0;
    let val = |i: &mut usize, name: &str| -> Result<String, String> {
        *i += 1;
        args.get(*i).cloned().ok_or(format!("{name} needs a value"))
    };
    while i < args.len() {
        let a = args[i].as_str();
        match a {
            "--pro" => o.pro = Some(val(&mut i, a)?.into()),
            "--config" => o.config = Some(val(&mut i, a)?.into()),
            "--no-config" => o.no_config = true,
            "--stackup" => o.stackup = Some(val(&mut i, a)?.into()),
            "--class" => {
                let v = val(&mut i, a)?;
                let (c, p) = v.split_once('=').ok_or("--class needs CLASS=PROFILE")?;
                o.classes.insert(c.to_string(), p.to_string());
            }
            "--nets" => o.nets = Some(val(&mut i, a)?.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()),
            "--all" => o.all = true,
            "--max" => o.max = val(&mut i, a)?.parse().map_err(|_| "--max needs a number")?,
            "--rise-ps" => o.rise_ps = Some(val(&mut i, a)?.parse().map_err(|_| "--rise-ps needs a number")?),
            "--roughness" => o.roughness = Some(val(&mut i, a)?),
            "--tree" => o.tree = Some(val(&mut i, a)?.into()),
            "--pdn" => o.pdn = Some(val(&mut i, a)?),
            "--out" => o.out = Some(val(&mut i, a)?.into()),
            _ if a.starts_with("--") => return Err(format!("unknown option {a} (see `kisi help`)")),
            _ => o.pos.push(a.to_string()),
        }
        i += 1;
    }
    Ok(o)
}

fn load(o: &Opts) -> Result<Project, String> {
    let pcb = PathBuf::from(o.pos.first().ok_or("board file missing (kisi <command> <board.kicad_pcb>)")?);
    let ov = match &o.stackup {
        Some(f) => Some(serde_json::from_str::<kisi_core::board::StackupOverride>(&std::fs::read_to_string(f).map_err(|e| format!("{}: {e}", f.display()))?).map_err(|e| format!("{}: {e}", f.display()))?),
        None => None,
    };
    let cfg = if o.no_config {
        None
    } else {
        match &o.config {
            Some(c) if !c.is_file() => return Err(format!("{}: not found", c.display())),
            Some(c) => Some(c.clone()),
            None => Some(service::config_path_for(&pcb)).filter(|c| c.is_file()),
        }
    };
    Project::load_with(&pcb, o.pro.as_deref(), ov.as_ref(), cfg.as_deref())
}

fn check_classes(o: &Opts, profiles: &Profiles) -> Result<(), String> {
    for (c, id) in &o.classes {
        if profiles.get(id).is_none() {
            return Err(format!("unknown profile '{id}' for class '{c}' (see `kisi profiles`)"));
        }
    }
    Ok(())
}

fn run(cmd: &str, o: &Opts) -> Result<Value, String> {
    let profiles = Profiles::builtin();
    match cmd {
        "help" => Ok(json!({"help": HELP})),
        "overview" => Ok(service::overview(&load(o)?)),
        "interfaces" => {
            check_classes(o, &profiles)?;
            Ok(service::interfaces(&load(o)?, &profiles, &o.classes))
        }
        "net" => {
            let net = o.pos.get(1).ok_or("net name missing (kisi net <board> <NET>)")?;
            let mut rp = RfParams::default();
            if let Some(r) = o.rise_ps {
                rp.rise = r * 1e-12;
            }
            if let Some(r) = &o.roughness {
                rp.roughness = match r.to_ascii_lowercase().as_str() {
                    "smooth" => Roughness::Smooth,
                    "hvlp" => Roughness::Hvlp,
                    "standard" => Roughness::Standard,
                    _ => return Err(format!("--roughness {r}: use smooth, hvlp or standard")),
                };
            }
            service::net_detail(&load(o)?, net, &rp)
        }
        "check" => {
            check_classes(o, &profiles)?;
            let p = load(o)?;
            let mut v = service::channel_check(&p, &profiles, &o.classes, o.nets.as_deref(), &RfParams::default());
            if let Some(rows) = v["rows"].as_array_mut() {
                if !o.all {
                    rows.retain(|r| r["status"] != "green");
                }
                let n = rows.len();
                rows.truncate(o.max);
                v["rows_omitted"] = json!(n.saturating_sub(o.max));
                v["rows_shown"] = json!(if o.all { "all" } else { "yellow and red (use --all for green too)" });
            }
            Ok(v)
        }
        "profiles" => {
            if let Some(id) = o.pos.first() {
                let p = profiles.get(id).ok_or(format!("unknown profile '{id}'"))?;
                let mut v = serde_json::to_value(p).unwrap();
                v["effective_limits"] = serde_json::to_value(profiles.limits(p)).unwrap();
                return Ok(v);
            }
            Ok(json!({"profiles": profiles.profiles.iter().map(|p| json!({"id": p.id, "name": p.name, "family": p.family, "rate_gbps": p.rate_gbps, "signal": p.signal, "z_ohm": p.z, "z_tol_pct": p.z_tol_pct})).collect::<Vec<_>>(),
                "default_limits": profiles.defaults.limits, "note": "z_ohm per line (differential: half the differential impedance). Limits marked 'kisi' are kisi guide values, not values from a standard."}))
        }
        "power" => {
            let tree = match &o.tree {
                Some(t) => Some(serde_json::from_str(&std::fs::read_to_string(t).map_err(|e| format!("{}: {e}", t.display()))?).map_err(|e| format!("{}: {e}", t.display()))?),
                None => None,
            };
            let p = load(o)?;
            let mut v = service::power(&p, tree, o.pdn.as_deref())?;
            v["config"] = p.config_summary();
            Ok(v)
        }
        "report" => {
            let p = load(o)?;
            let rep = service::report(&p, &profiles);
            let path = o.out.clone().unwrap_or_else(|| p.pcb_path.parent().unwrap_or(Path::new(".")).join("analysis").join("kisi.json"));
            if let Some(d) = path.parent() {
                std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
            }
            std::fs::write(&path, serde_json::to_string_pretty(&rep).unwrap()).map_err(|e| format!("{}: {e}", path.display()))?;
            Ok(json!({"written": path.display().to_string(), "channels": rep["channels"]["summary"], "classes": rep["overview"]["classes"].as_array().map(|a| a.len()), "config": rep["config"]}))
        }
        "gaps" => Ok(kisi_core::gaps::gaps(&load(o)?, &profiles)),
        "config-check" => {
            let p = load(o)?;
            match (&p.config_path, &p.config_check) {
                (None, _) => Err(format!("no configuration: {} does not exist (create it, see `kisi config-template`)", service::config_path_for(&p.pcb_path).display())),
                (Some(f), Some(c)) => Ok(json!({"file": f.display().to_string(), "ok": c.ok, "entries": c.entries, "errors": c.errors, "warnings": c.warnings})),
                (Some(f), None) => Ok(json!({"file": f.display().to_string(), "ok": true})),
            }
        }
        "config-template" => {
            let o2 = Opts { no_config: true, ..o.clone() };
            let p = load(&o2)?;
            let su = p.board.stackup();
            let diel: Vec<Value> = su
                .dielectrics
                .iter()
                .enumerate()
                .map(|(i, d)| json!({"index": i, "_layer": format!("{} / {}", su.copper[i].name, su.copper.get(i + 1).map(|c| c.name.as_str()).unwrap_or("")), "_now": {"material": d.material, "thickness": d.thickness, "epsilon_r": d.epsilon_r, "loss_tangent": d.loss_tangent}, "source": {"kind": "datasheet", "document": null, "url": null, "page": null, "note": null}}))
                .collect();
            let classes: serde_json::Map<String, Value> = p
                .an
                .classes
                .iter()
                .filter(|c| c.nets > 0)
                .map(|c| (c.name.clone(), json!({"target_ohm": c.target.map(|t| t.z), "diff": c.target.map(|t| t.diff), "profile": null, "source": {"kind": "datasheet", "document": null, "note": null}})))
                .collect();
            Ok(json!({
                "kisi_config": kisi_core::config::VERSION,
                "board": p.pcb_path.file_name().map(|f| f.to_string_lossy().to_string()),
                "stackup": {"dielectrics": diel, "solder_mask": {"epsilon_r": null, "source": {"kind": "datasheet", "document": null}}},
                "net_classes": classes,
                "fields": [{"ref": "U1", "name": "PI_LOAD", "value": "+3V3=100mA", "source": {"kind": "datasheet", "url": null, "page": null, "note": "typical"}}],
                "notes": ["Template: keep only entries you fill in; keys starting with '_' are information and ignored. Every value needs a source (kind: datasheet, standard, application_note, measurement, simulation, estimate, assumption). Save as <board>.kisi.json next to the board and run `kisi config-check`."]
            }))
        }
        _ => Err(format!("unknown command {cmd}")),
    }
}

/// Run a command; prints JSON (or `{"error": …}`) and returns the exit code.
pub fn main(cmd: &str, args: &[String]) -> i32 {
    if cmd == "help" {
        println!("{HELP}");
        return 0;
    }
    let r = parse_opts(args).and_then(|o| run(cmd, &o));
    match r {
        Ok(v) => {
            println!("{}", serde_json::to_string_pretty(&v).unwrap());
            0
        }
        Err(e) => {
            println!("{}", serde_json::to_string_pretty(&json!({"error": e})).unwrap());
            eprintln!("kisi {cmd}: {e}");
            1
        }
    }
}

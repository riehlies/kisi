//! `kisi <command> <board> …`: the analyses as JSON on stdout. Parsing and results
//! come from `kisi_core::commands` (shared with the Node CLI kisi.mjs); this file
//! only reads and writes files. The native CLI has no field solver: impedances are
//! formula values (kisi.mjs adds the field solver).

use kisi_core::commands::{self, Opts};
use kisi_core::service::{self, Project};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

pub use commands::HELP;

fn read(p: &str) -> Result<String, String> {
    std::fs::read_to_string(p).map_err(|e| format!("{p}: {e}"))
}

fn load(o: &Opts, with_config: bool) -> Result<Project, String> {
    let pcb = PathBuf::from(&o.pos[0]);
    let ov = match &o.stackup {
        Some(f) => Some(
            serde_json::from_str::<kisi_core::board::StackupOverride>(&read(f)?).map_err(|e| format!("{f}: {e}"))?,
        ),
        None => None,
    };
    let cfg = if o.no_config || !with_config {
        None
    } else {
        match &o.config {
            Some(c) if !Path::new(c).is_file() => return Err(format!("{c}: not found")),
            Some(c) => Some(PathBuf::from(c)),
            None => Some(service::config_path_for(&pcb)).filter(|c| c.is_file()),
        }
    };
    Project::load_preset(
        &pcb,
        o.pro.as_deref().map(Path::new),
        ov.as_ref(),
        cfg.as_deref(),
        o.variant.as_deref(),
        o.stackup_preset.as_deref(),
        commands::tol_arg(o).as_deref(),
    )
}

fn run(o: &Opts) -> Result<Value, String> {
    let p = if commands::wants_board(o) { Some(load(o, o.cmd != "config-template")?) } else { None };
    let tree = match &o.tree {
        Some(t) => Some(serde_json::from_str::<Value>(&read(t)?).map_err(|e| format!("{t}: {e}"))?),
        None => None,
    };
    let mut v = commands::run(o, p.as_ref(), tree)?;
    if o.cmd == "report" {
        let p = p.as_ref().unwrap();
        let path = o
            .out
            .clone()
            .map(PathBuf::from)
            .unwrap_or_else(|| p.pcb_path.parent().unwrap_or(Path::new(".")).join("analysis").join("kisi.json"));
        // the report's date: SOURCE_DATE_EPOCH (reproducible files), else today (UTC)
        let secs =
            std::env::var("SOURCE_DATE_EPOCH").ok().and_then(|s| s.trim().parse::<i64>().ok()).unwrap_or_else(|| {
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0)
            });
        let files = commands::report_files(o, &path.display().to_string(), &v, &commands::utc_date(secs));
        for (f, text) in &files {
            if let Some(d) = Path::new(f).parent().filter(|d| !d.as_os_str().is_empty()) {
                std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
            }
            std::fs::write(f, text).map_err(|e| format!("{f}: {e}"))?;
        }
        v = commands::report_summary(&files, o, &v);
    }
    // files a command asks to write (e.g. `net --s2p`)
    if let Some(w) = v.as_object_mut().and_then(|m| m.remove("_write")) {
        let (path, content) = (w["path"].as_str().unwrap_or(""), w["content"].as_str().unwrap_or(""));
        std::fs::write(path, content).map_err(|e| format!("{path}: {e}"))?;
        v["written"] = json!(path);
    }
    // the files of `export`: into a folder
    if let Some(w) = v.as_object_mut().and_then(|m| m.remove("_files")) {
        let dir = PathBuf::from(w["dir"].as_str().unwrap_or("."));
        for f in w["files"].as_array().map(|a| a.as_slice()).unwrap_or(&[]) {
            let path = dir.join(f["path"].as_str().unwrap_or("x"));
            if let Some(d) = path.parent() {
                std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
            }
            std::fs::write(&path, f["content"].as_str().unwrap_or(""))
                .map_err(|e| format!("{}: {e}", path.display()))?;
        }
        v["written"] = json!(dir.display().to_string());
    }
    if commands::uses_field(&o.cmd) && !o.formula {
        v["field_solver_note"] =
            json!("native kisi: formula impedances only; the Node CLI kisi.mjs adds the 2D field solver");
    }
    Ok(v)
}

/// Run a command line (`args[0]` = command); prints JSON (or `{"error": …}`) and returns the exit code.
pub fn main(args: &[String]) -> i32 {
    let o = match commands::parse_args(args) {
        Ok(o) => o,
        Err(e) => {
            println!("{}", serde_json::to_string_pretty(&json!({"error": e})).unwrap());
            eprintln!("kisi: {e}");
            return 1;
        }
    };
    if o.cmd == "help" {
        println!("{HELP}");
        return 0;
    }
    match run(&o) {
        Ok(v) => {
            println!("{}", serde_json::to_string_pretty(&v).unwrap());
            0
        }
        Err(e) => {
            println!("{}", serde_json::to_string_pretty(&json!({"error": e})).unwrap());
            eprintln!("kisi {}: {e}", o.cmd);
            1
        }
    }
}

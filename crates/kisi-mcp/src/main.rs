//! kisi-mcp: Model Context Protocol server (stdio, JSON-RPC 2.0) for kisi.
//!
//! Gives AI agents a small, stable set of tools for the signal- and power-
//! integrity analysis of KiCad boards. Boards are read from disk (the path can
//! come from the user, from Konnect or from any other KiCad tool), so the
//! server works next to KiCad-editing MCP servers without talking to KiCad
//! itself. Results are compact JSON; `kisi_report` writes
//! `analysis/kisi.json` next to the board for file-based review tools such as
//! kicad-happy.
//!
//! Transport: one JSON-RPC message per line on stdin/stdout, logs on stderr.

use kisi_core::profile::Profiles;
use kisi_core::rf::{RfParams, Roughness};
use kisi_core::service::{self, Project};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

const PROTOCOLS: &[&str] = &["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

const INSTRUCTIONS: &str = "kisi analyses KiCad boards (.kicad_pcb, with the .kicad_pro next to it for net classes). \
Start with kisi_overview to see stackup and impedance compliance per net class, then kisi_net for single nets \
(impedance along the path, losses, TDR, vias, crosstalk) or kisi_channel_check for eye/loss/skew checks against \
interface profiles (PCIe, USB, DDR, LVDS, ... see kisi_profiles). kisi_power covers regulator chains (PI_REG/PI_LOAD \
fields), DC IR drop and PDN impedance. Impedances in these tools come from closed-form formulas; the 2D field solver \
runs only in the browser viewer (kisi_open_viewer). Values are design-check estimates, not signoff results; always \
state the models and assumptions reported in the 'models' fields.";

struct Server {
    cache: HashMap<String, (SystemTime, Project)>,
    profiles: Profiles,
}

fn tools() -> Value {
    let pcb = json!({"type": "string", "description": "Path to the .kicad_pcb file (the .kicad_pro next to it is read for net classes)"});
    let stackup = json!({"type": "object", "description": "Optional stackup changes: {\"dielectrics\": [{\"index\": 0, \"epsilon_r\": 3.6, \"loss_tangent\": 0.009, \"thickness\": 0.11}]} (thickness in mm, index counts dielectrics from the top)"});
    json!([
        {
            "name": "kisi_overview",
            "title": "Board overview (stackup, impedance per net class)",
            "description": "Stackup and, per net class with a target impedance, the share of routed length within ±5 % and the worst nets. Run this first.",
            "inputSchema": {"type": "object", "properties": {"pcb": pcb, "stackup": stackup}, "required": ["pcb"]},
            "annotations": {"readOnlyHint": true}
        },
        {
            "name": "kisi_net",
            "title": "Single net: impedance, losses, TDR, vias, crosstalk",
            "description": "Impedance segments along the routed path, insertion loss at typical Nyquist frequencies, worst return loss, TDR extremes, via details (antipad, stub resonance, return path) and the strongest crosstalk aggressors of one net. Differential pairs are evaluated in the odd mode (TDR reported as differential impedance).",
            "inputSchema": {"type": "object", "properties": {
                "pcb": pcb, "net": {"type": "string", "description": "Net name, with or without sheet path"},
                "rise_ps": {"type": "number", "description": "10-90 % rise time for TDR and crosstalk (default 35)"},
                "roughness": {"type": "string", "enum": ["Smooth", "Hvlp", "Standard"], "description": "Copper roughness preset (default Standard)"},
                "stackup": stackup}, "required": ["pcb", "net"]},
            "annotations": {"readOnlyHint": true}
        },
        {
            "name": "kisi_channel_check",
            "title": "Channel check against interface profiles",
            "description": "Eye height/width (PRBS7 with the profile's linear transmitter/receiver, no equalisation), loss at Nyquist, return loss, intra-pair skew and via-stub resonance for every net with a target impedance, judged green/yellow/red against the profile limits. Profiles are assigned automatically from net and class names unless given per class.",
            "inputSchema": {"type": "object", "properties": {
                "pcb": pcb,
                "classes": {"type": "object", "additionalProperties": {"type": "string"}, "description": "Net class → profile id (see kisi_profiles), e.g. {\"PCIe\": \"pcie3\"}"},
                "nets": {"type": "array", "items": {"type": "string"}, "description": "Only these nets"},
                "all_rows": {"type": "boolean", "description": "Return green rows too (default: only yellow/red plus counts)"},
                "max_rows": {"type": "integer", "description": "Row limit (default 30)"},
                "stackup": stackup}, "required": ["pcb"]},
            "annotations": {"readOnlyHint": true}
        },
        {
            "name": "kisi_profiles",
            "title": "List interface profiles",
            "description": "Interface profiles (PCIe 1-5, USB 2/3, DDR4/5, RGMII, SGMII, HDMI, LVDS, MIPI, generic) with data rate, target impedance, transmitter/receiver models, limits and sources.",
            "inputSchema": {"type": "object", "properties": {"id": {"type": "string", "description": "Return only this profile in full"}}},
            "annotations": {"readOnlyHint": true}
        },
        {
            "name": "kisi_power",
            "title": "Power: regulator chain, DC IR drop, PDN impedance",
            "description": "Power balance of the regulator chain (from PI_REG/PI_LOAD fields or a kisi tree JSON), DC IR drop of every rail and optionally the AC impedance of one rail against its target impedance.",
            "inputSchema": {"type": "object", "properties": {
                "pcb": pcb, "tree": {"type": "string", "description": "Path to a kisi power-tree JSON; default: PI_REG/PI_LOAD fields of the board"},
                "pdn_rail": {"type": "string", "description": "Rail for the PDN impedance (e.g. +1V0)"}}, "required": ["pcb"]},
            "annotations": {"readOnlyHint": true}
        },
        {
            "name": "kisi_report",
            "title": "Write analysis/kisi.json",
            "description": "Runs overview, channel check and power analysis and writes them to analysis/kisi.json next to the board (or to 'out'), for file-based review tools. Returns the path and the counts.",
            "inputSchema": {"type": "object", "properties": {"pcb": pcb, "out": {"type": "string", "description": "Output path (default <board dir>/analysis/kisi.json)"}}, "required": ["pcb"]},
            "annotations": {"readOnlyHint": false, "destructiveHint": false}
        },
        {
            "name": "kisi_open_viewer",
            "title": "Open the kisi viewer",
            "description": "Opens the standalone kisi viewer in the browser with this board (field solver, charts, editing of profiles and stackup). Optionally pre-selects a net and a view.",
            "inputSchema": {"type": "object", "properties": {
                "pcb": pcb, "net": {"type": "string"}, "view": {"type": "string", "enum": ["si", "ch", "pi"], "description": "Impedanz / Kanal / Versorgung"}}, "required": ["pcb"]},
            "annotations": {"readOnlyHint": true, "openWorldHint": true}
        }
    ])
}

impl Server {
    fn project(&mut self, args: &Value) -> Result<&Project, String> {
        let pcb = args["pcb"].as_str().ok_or("argument 'pcb' missing")?;
        let path = PathBuf::from(pcb);
        let mtime = std::fs::metadata(&path).and_then(|m| m.modified()).map_err(|e| format!("{pcb}: {e}"))?;
        let ov: Option<kisi_core::board::StackupOverride> = args.get("stackup").and_then(|v| serde_json::from_value(v.clone()).ok());
        let key = format!("{}|{}", path.display(), args.get("stackup").map(|v| v.to_string()).unwrap_or_default());
        let fresh = self.cache.get(&key).is_some_and(|(t, _)| *t == mtime);
        if !fresh {
            let p = Project::load(&path, None, ov.as_ref())?;
            self.cache.insert(key.clone(), (mtime, p));
        }
        Ok(&self.cache[&key].1)
    }

    fn call(&mut self, name: &str, args: &Value) -> Result<Value, String> {
        match name {
            "kisi_overview" => Ok(service::overview(self.project(args)?)),
            "kisi_net" => {
                let mut rp = RfParams::default();
                if let Some(r) = args["rise_ps"].as_f64() {
                    rp.rise = r * 1e-12;
                }
                if let Some(r) = args["roughness"].as_str() {
                    rp.roughness = match r {
                        "Smooth" => Roughness::Smooth,
                        "Hvlp" => Roughness::Hvlp,
                        _ => Roughness::Standard,
                    };
                }
                let net = args["net"].as_str().ok_or("argument 'net' missing")?.to_string();
                service::net_detail(self.project(args)?, &net, &rp)
            }
            "kisi_channel_check" => {
                let assign: HashMap<String, String> = args.get("classes").and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or_default();
                let nets: Option<Vec<String>> = args.get("nets").and_then(|v| serde_json::from_value(v.clone()).ok());
                let all = args["all_rows"].as_bool().unwrap_or(false);
                let max = args["max_rows"].as_u64().unwrap_or(30) as usize;
                let profiles = self.profiles.clone();
                for (c, id) in &assign {
                    if profiles.get(id).is_none() {
                        return Err(format!("unknown profile '{id}' for class '{c}' (see kisi_profiles)"));
                    }
                }
                let p = self.project(args)?;
                let mut v = service::channel_check(p, &profiles, &assign, nets.as_deref(), &RfParams::default());
                if let Some(rows) = v["rows"].as_array_mut() {
                    if !all {
                        rows.retain(|r| r["status"] != "green");
                    }
                    let n = rows.len();
                    rows.truncate(max);
                    v["rows_omitted"] = json!(n.saturating_sub(max));
                }
                Ok(v)
            }
            "kisi_profiles" => {
                if let Some(id) = args["id"].as_str() {
                    let p = self.profiles.get(id).ok_or(format!("unknown profile '{id}'"))?;
                    let mut v = serde_json::to_value(p).unwrap();
                    v["effective_limits"] = serde_json::to_value(self.profiles.limits(p)).unwrap();
                    return Ok(v);
                }
                Ok(json!({"profiles": self.profiles.profiles.iter().map(|p| json!({"id": p.id, "name": p.name, "family": p.family, "rate_gbps": p.rate_gbps, "signal": p.signal, "z_ohm": p.z, "z_tol_pct": p.z_tol_pct})).collect::<Vec<_>>(),
                    "default_limits": self.profiles.defaults.limits, "note": "Values per line (differential: half the differential resistance). Limits marked 'kisi' are kisi guide values, not standard values."}))
            }
            "kisi_power" => {
                let tree = match args["tree"].as_str() {
                    Some(t) if t != "fields" => Some(serde_json::from_str(&std::fs::read_to_string(t).map_err(|e| format!("{t}: {e}"))?).map_err(|e| format!("{t}: {e}"))?),
                    _ => None,
                };
                let rail = args["pdn_rail"].as_str().map(str::to_string);
                service::power(self.project(args)?, tree, rail.as_deref())
            }
            "kisi_report" => {
                let profiles = self.profiles.clone();
                let out = args["out"].as_str().map(PathBuf::from);
                let p = self.project(args)?;
                let rep = service::report(p, &profiles);
                let path = out.unwrap_or_else(|| p.pcb_path.parent().unwrap_or(Path::new(".")).join("analysis").join("kisi.json"));
                if let Some(d) = path.parent() {
                    std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
                }
                std::fs::write(&path, serde_json::to_string_pretty(&rep).unwrap()).map_err(|e| format!("{}: {e}", path.display()))?;
                Ok(json!({"written": path.display().to_string(), "channels": rep["channels"]["summary"], "classes": rep["overview"]["classes"].as_array().map(|a| a.len())}))
            }
            "kisi_open_viewer" => open_viewer(args),
            _ => Err(format!("unknown tool '{name}'")),
        }
    }
}

/// Copy of the viewer with the board embedded (same hand-over as the KiCad plugin).
fn open_viewer(args: &Value) -> Result<Value, String> {
    let pcb = PathBuf::from(args["pcb"].as_str().ok_or("argument 'pcb' missing")?);
    let viewer = viewer_path().ok_or("kisi.html not found (set KISI_VIEWER or put kisi.html next to kisi-mcp)")?;
    let html = std::fs::read_to_string(&viewer).map_err(|e| e.to_string())?;
    let marker = "null; /*KISI_PRELOAD*/";
    if !html.contains(marker) {
        return Err("kisi.html has no hand-over marker (old version?)".into());
    }
    let text = std::fs::read_to_string(&pcb).map_err(|e| format!("{}: {e}", pcb.display()))?;
    let pro = std::fs::read_to_string(pcb.with_extension("kicad_pro")).ok();
    let name = pcb.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let pl = json!({"name": name, "pcb_text": text, "pro_text": pro, "net": args["net"], "view": args["view"].as_str().unwrap_or("si")});
    let out = std::env::temp_dir().join(format!("kisi_{name}.html"));
    let html = html.replacen(marker, &(pl.to_string().replace("</", "<\\/") + ";"), 1);
    std::fs::write(&out, html).map_err(|e| e.to_string())?;
    let url = format!("file://{}", out.display());
    let opened = if std::env::var_os("KISI_NO_BROWSER").is_some() {
        false
    } else if cfg!(target_os = "windows") {
        std::process::Command::new("cmd").args(["/C", "start", "", &url]).spawn().is_ok()
    } else if cfg!(target_os = "macos") {
        std::process::Command::new("open").arg(&url).spawn().is_ok()
    } else {
        std::process::Command::new("xdg-open").arg(&url).spawn().is_ok()
    };
    Ok(json!({"file": out.display().to_string(), "opened": opened}))
}

fn viewer_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("KISI_VIEWER") {
        return Some(PathBuf::from(p));
    }
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    [dir.join("kisi.html"), dir.join("../share/kisi/kisi.html"), dir.join("../../dist/kisi.html")].into_iter().find(|p| p.exists())
}

fn handle(srv: &mut Server, msg: &Value) -> Option<Value> {
    let id = msg.get("id").cloned();
    let method = msg["method"].as_str().unwrap_or("");
    let params = &msg["params"];
    let result: Result<Value, (i64, String)> = match method {
        "initialize" => {
            let want = params["protocolVersion"].as_str().unwrap_or(PROTOCOLS[1]);
            let ver = if PROTOCOLS.contains(&want) { want } else { PROTOCOLS[1] };
            Ok(json!({
                "protocolVersion": ver,
                "capabilities": {"tools": {"listChanged": false}},
                "serverInfo": {"name": "kisi", "title": "kisi SI/PI for KiCad", "version": env!("CARGO_PKG_VERSION")},
                "instructions": INSTRUCTIONS
            }))
        }
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({"tools": tools()})),
        "resources/list" => Ok(json!({"resources": []})),
        "prompts/list" => Ok(json!({"prompts": []})),
        "tools/call" => {
            let name = params["name"].as_str().unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or(json!({}));
            let t0 = std::time::Instant::now();
            let r = srv.call(name, &args);
            eprintln!("kisi-mcp: {name} {:?} {}", t0.elapsed(), if r.is_ok() { "ok" } else { "error" });
            Ok(match r {
                Ok(v) => json!({"content": [{"type": "text", "text": v.to_string()}], "structuredContent": v, "isError": false}),
                // tool errors are results with isError, so the model can react
                Err(e) => json!({"content": [{"type": "text", "text": e}], "isError": true}),
            })
        }
        m if m.starts_with("notifications/") => return None,
        _ => Err((-32601, format!("method not found: {method}"))),
    };
    let id = id?; // notifications get no response
    Some(match result {
        Ok(r) => json!({"jsonrpc": "2.0", "id": id, "result": r}),
        Err((code, m)) => json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": m}}),
    })
}

fn main() {
    if std::env::args().any(|a| a == "--version") {
        println!("kisi-mcp {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    let mut srv = Server { cache: HashMap::new(), profiles: Profiles::builtin() };
    if let Some(p) = std::env::var_os("KISI_PROFILES") {
        match std::fs::read_to_string(&p).map_err(|e| e.to_string()).and_then(|t| serde_json::from_str(&t).map_err(|e| e.to_string())) {
            Ok(ps) => srv.profiles = ps,
            Err(e) => eprintln!("kisi-mcp: KISI_PROFILES: {e} – using built-in profiles"),
        }
    }
    let stdin = std::io::stdin();
    let mut out = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(msg) => handle(&mut srv, &msg),
            Err(e) => Some(json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": format!("parse error: {e}")}})),
        };
        if let Some(r) = reply {
            let _ = writeln!(out, "{}", r);
            let _ = out.flush();
        }
    }
}

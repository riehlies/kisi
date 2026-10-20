//! The `kisi <command> <board>` commands: argument parsing and the JSON results.
//!
//! Shared by the native CLI (`kisi-cli`) and the Node CLI (`kisi.mjs`, through
//! WASM), so both accept the same arguments and print the same JSON. The front ends
//! only read files, run the field solver (Node) and write `analysis/kisi.json`.

use crate::profile::Profiles;
use crate::rf::{RfParams, Roughness};
use crate::service::{self, Project};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

pub const COMMANDS: &[&str] = &["overview", "interfaces", "net", "check", "crosstalk", "profiles", "power", "report", "gaps", "config-check", "config-template", "help"];

pub const HELP: &str = "kisi <command> <board.kicad_pcb> [options] – JSON on stdout

  overview         stackup, impedance per net class (share within ±5 %, worst nets)
  interfaces       buses and device-to-device lanes with profiles   [--class CLASS=PROFILE]…
  net <NET>        one net: impedance along the path, loss, TDR, vias, crosstalk;
                   differential pairs also as 4-port (mixed mode, skew)
                   [--rise-ps 35] [--roughness smooth|hvlp|standard] [--s2p FILE | --s4p FILE (Touchstone)]
  check            channel check of all lanes against interface profiles
                   [--class CLASS=PROFILE]… [--nets A,B] [--all] [--max 30]
  crosstalk        coupling scan (NEXT/FEXT from neighbouring tracks) of all nets with a target,
                   critical from 1 %; kisi.mjs refines the critical couplings with the field solver
                   [--nets A,B] [--all (all rows, not only critical)] [--max 30] [--rise-ps 35]
  profiles [ID]    interface profiles (no board needed)
  power            regulator chain, DC IR drop per rail and ground return, losses, power flow
                                                          [--tree tree.json] [--pdn RAIL]
  report           write analysis/kisi.json next to the board   [--out FILE]
  gaps             what the analyses miss, where to find it, how to write it into <board>.kisi.json
  config-check     check <board>.kisi.json (format, ranges, references, sources)
  config-template  a <board>.kisi.json skeleton with this board's dielectrics and net classes

common options: --pro FILE (default: board.kicad_pro), --config FILE (default: board.kisi.json
next to the board), --no-config, --stackup FILE (stackup changes, as in the viewer)
field solver (kisi.mjs only; overview, net, check, report): used by default for the cross-sections
of nets with a target, results cached on disk; --formula (no field solver), --field-time S
(time budget for new solves, default 120), --field-all (all cross-sections, not only targets),
--field-xt (also the coupling cross-sections of critical crosstalk victims, NEXT or FEXT from 1 %;
net: always all of the net's)";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Opts {
    pub cmd: String,
    /// positional arguments after the command: board, then e.g. the net name or profile id
    pub pos: Vec<String>,
    pub pro: Option<String>,
    pub config: Option<String>,
    pub no_config: bool,
    pub stackup: Option<String>,
    pub classes: HashMap<String, String>,
    pub nets: Option<Vec<String>>,
    pub all: bool,
    pub max: Option<usize>,
    pub rise_ps: Option<f64>,
    pub roughness: Option<String>,
    pub tree: Option<String>,
    pub pdn: Option<String>,
    pub out: Option<String>,
    /// `net`: write the channel's S-parameters as Touchstone .s2p
    pub s2p: Option<String>,
    /// `net`: write the pair's single-ended 4-port as Touchstone .s4p
    pub s4p: Option<String>,
    pub formula: bool,
    pub field_time: Option<f64>,
    pub field_all: bool,
    /// field solver: also the coupling cross-sections of the crosstalk estimate
    pub field_xt: bool,
}

/// Parse `<command> [args…]`.
pub fn parse_args(args: &[String]) -> Result<Opts, String> {
    let mut o = Opts { cmd: args.first().cloned().unwrap_or_else(|| "help".into()), ..Default::default() };
    if o.cmd == "--help" || o.cmd == "-h" {
        o.cmd = "help".into();
    }
    if !COMMANDS.contains(&o.cmd.as_str()) {
        return Err(format!("unknown command '{}' (see `kisi help`)", o.cmd));
    }
    let mut i = 1;
    let val = |i: &mut usize, name: &str| -> Result<String, String> {
        *i += 1;
        args.get(*i).cloned().ok_or(format!("{name} needs a value"))
    };
    let num = |s: String, name: &str| -> Result<f64, String> { s.parse().map_err(|_| format!("{name} needs a number")) };
    while i < args.len() {
        let a = args[i].as_str();
        match a {
            "--pro" => o.pro = Some(val(&mut i, a)?),
            "--config" => o.config = Some(val(&mut i, a)?),
            "--no-config" => o.no_config = true,
            "--stackup" => o.stackup = Some(val(&mut i, a)?),
            "--class" => {
                let v = val(&mut i, a)?;
                let (c, p) = v.split_once('=').ok_or("--class needs CLASS=PROFILE")?;
                o.classes.insert(c.to_string(), p.to_string());
            }
            "--nets" => o.nets = Some(val(&mut i, a)?.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()),
            "--all" => o.all = true,
            "--max" => o.max = Some(num(val(&mut i, a)?, a)? as usize),
            "--rise-ps" => o.rise_ps = Some(num(val(&mut i, a)?, a)?),
            "--roughness" => o.roughness = Some(val(&mut i, a)?),
            "--tree" => o.tree = Some(val(&mut i, a)?),
            "--pdn" => o.pdn = Some(val(&mut i, a)?),
            "--out" => o.out = Some(val(&mut i, a)?),
            "--s2p" => o.s2p = Some(val(&mut i, a)?),
            "--s4p" => o.s4p = Some(val(&mut i, a)?),
            "--formula" => o.formula = true,
            "--field-time" => o.field_time = Some(num(val(&mut i, a)?, a)?),
            "--field-all" => o.field_all = true,
            "--field-xt" => o.field_xt = true,
            _ if a.starts_with("--") => return Err(format!("unknown option {a} (see `kisi help`)")),
            _ => o.pos.push(a.to_string()),
        }
        i += 1;
    }
    let profiles = Profiles::builtin();
    for (c, id) in &o.classes {
        if profiles.get(id).is_none() {
            return Err(format!("unknown profile '{id}' for class '{c}' (see `kisi profiles`)"));
        }
    }
    if needs_board(&o.cmd) && o.pos.is_empty() {
        return Err(format!("board file missing (kisi {} <board.kicad_pcb>)", o.cmd));
    }
    if o.cmd == "net" && o.pos.len() < 2 {
        return Err("net name missing (kisi net <board> <NET>)".into());
    }
    Ok(o)
}

pub fn needs_board(cmd: &str) -> bool {
    !matches!(cmd, "help" | "profiles")
}

/// Whether the command benefits from field-solver impedances.
pub fn uses_field(cmd: &str) -> bool {
    matches!(cmd, "overview" | "net" | "check" | "crosstalk" | "report")
}

/// Cross-sections the field solver should solve for this command, most important first:
/// (geometry id, geometry text, weight = routed length in mm).
pub fn field_plan(p: &Project, o: &Opts) -> Vec<(String, String, f64)> {
    if o.formula || !uses_field(&o.cmd) {
        return vec![];
    }
    let g = &p.an.geometries;
    let mut plan: Vec<(String, String, f64)> = if o.cmd == "crosstalk" {
        // stage 2: only the coupling cross-sections of the critical victims
        let Ok(nets) = service::xt_nets(p, o.nets.as_deref()) else { return vec![] };
        xt_plan(p, &nets, false)
    } else if o.cmd == "net" {
        // only the geometries along this net (and its partner)
        let Some(net) = o.pos.get(1).and_then(|n| p.net_id(n)) else { return vec![] };
        let partner = p.an.nets.iter().find(|s| s.net == net).and_then(|s| s.partner);
        let nets: HashSet<u32> = std::iter::once(net).chain(partner).collect();
        let mut w: HashMap<u32, f64> = HashMap::new();
        for c in p.an.pieces.chunks(4) {
            let (ti, t0, t1, xi) = (c[0] as usize, c[1], c[2], c[3] as usize);
            let Some(tr) = p.board.tracks.get(ti) else { continue };
            if !nets.contains(&tr.net) {
                continue;
            }
            if let Some(gi) = p.an.cross_sections.get(xi).and_then(|x| x.geom) {
                *w.entry(gi).or_default() += crate::geom::len(crate::geom::sub(tr.b, tr.a)) * (t1 - t0).abs();
            }
        }
        let mut v: Vec<(String, String, f64)> = w.into_iter().filter_map(|(gi, l)| g.get(gi as usize).map(|e| (e.id.clone(), e.text.clone(), l))).collect();
        v.extend(xt_plan(p, &[net], true));
        v
    } else {
        let mut v: Vec<(String, String, f64)> = g.iter().filter(|e| o.field_all || e.target_length > 0.0).map(|e| (e.id.clone(), e.text.clone(), if o.field_all { e.length } else { e.target_length })).collect();
        if o.field_xt {
            let nets: Vec<u32> = p.an.nets.iter().filter(|s| o.field_all || s.target.is_some()).map(|s| s.net).collect();
            v.extend(xt_plan(p, &nets, false));
        }
        v
    };
    plan.sort_by(|a, b| b.2.total_cmp(&a.2));
    plan
}

/// Coupling cross-sections of the crosstalk estimate, after the impedance cross-sections
/// (weight scaled below any routed length): all of one net's (`all`), else only those of the
/// critical victims (stage 2 of the coupling analysis).
fn xt_plan(p: &Project, nets: &[u32], all: bool) -> Vec<(String, String, f64)> {
    let (f, rp) = (HashMap::new(), RfParams::default());
    if all {
        nets.iter().flat_map(|&n| crate::rf::crosstalk(&p.board, &p.an, n, &f, &rp).geoms).map(|(id, t)| (id, t, 1e-3)).collect()
    } else {
        crate::rf::crosstalk_geoms(&p.board, &p.an, nets, &f, &rp).into_iter().map(|(id, t, w)| (id, t, 1e-3 * w)).collect()
    }
}

fn rf_params(o: &Opts) -> Result<RfParams, String> {
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
    Ok(rp)
}

/// Run a command. `p` is the project (None for `help`/`profiles`; for `config-template`
/// load it without configuration), `tree` the parsed `--tree` file. `report` returns the
/// full report; the caller writes it and prints `report_summary`.
pub fn run(o: &Opts, p: Option<&Project>, tree: Option<Value>) -> Result<Value, String> {
    let profiles = Profiles::builtin();
    let proj = || p.ok_or_else(|| "no board loaded".to_string());
    match o.cmd.as_str() {
        "help" => Ok(json!({"help": HELP})),
        "profiles" => {
            if let Some(id) = o.pos.first() {
                let pr = profiles.get(id).ok_or(format!("unknown profile '{id}'"))?;
                let mut v = serde_json::to_value(pr).unwrap();
                v["effective_limits"] = serde_json::to_value(profiles.limits(pr)).unwrap();
                return Ok(v);
            }
            Ok(json!({"profiles": profiles.profiles.iter().map(|p| json!({"id": p.id, "name": p.name, "family": p.family, "rate_gbps": p.rate_gbps, "signal": p.signal, "z_ohm": p.z, "z_tol_pct": p.z_tol_pct})).collect::<Vec<_>>(),
                "default_limits": profiles.defaults.limits, "note": "z_ohm per line (differential: half the differential impedance). Limits marked 'kisi' are kisi guide values, not values from a standard."}))
        }
        "overview" => Ok(service::overview(proj()?)),
        "interfaces" => Ok(service::interfaces(proj()?, &profiles, &o.classes)),
        "net" => {
            let (pr, rp) = (proj()?, rf_params(o)?);
            let mut v = service::net_detail(pr, &o.pos[1], &rp)?;
            // the front end writes the file and replaces "_write" by "written"
            match (&o.s2p, &o.s4p) {
                (Some(_), Some(_)) => return Err("--s2p and --s4p: give one of them".into()),
                (Some(path), None) => v["_write"] = json!({"path": path, "content": service::net_touchstone(pr, &o.pos[1], &rp)?}),
                (None, Some(path)) => v["_write"] = json!({"path": path, "content": service::net_touchstone4(pr, &o.pos[1], &rp)?}),
                _ => {}
            }
            Ok(v)
        }
        "check" => {
            let mut v = service::channel_check(proj()?, &profiles, &o.classes, o.nets.as_deref(), &rf_params(o)?);
            let max = o.max.unwrap_or(30);
            if let Some(rows) = v["rows"].as_array_mut() {
                if !o.all {
                    rows.retain(|r| r["status"] != "green");
                }
                let n = rows.len();
                rows.truncate(max);
                v["rows_omitted"] = json!(n.saturating_sub(max));
                v["rows_shown"] = json!(if o.all { "all" } else { "yellow and red (use --all for green too)" });
            }
            Ok(v)
        }
        "crosstalk" => {
            let mut v = service::crosstalk_scan(proj()?, o.nets.as_deref(), &rf_params(o)?)?;
            let max = o.max.unwrap_or(30);
            if let Some(rows) = v["rows"].as_array_mut() {
                if !o.all {
                    rows.retain(|r| r["critical"] == true);
                }
                let n = rows.len();
                rows.truncate(max);
                v["rows_omitted"] = json!(n.saturating_sub(max));
                v["rows_shown"] = json!(if o.all { "all" } else { "critical (use --all for all)" });
            }
            Ok(v)
        }
        "power" => {
            let tree = match tree {
                Some(t) => Some(serde_json::from_value(t).map_err(|e| format!("--tree: {e}"))?),
                None => None,
            };
            let pr = proj()?;
            let mut v = service::power(pr, tree, o.pdn.as_deref())?;
            v["config"] = pr.config_summary();
            Ok(v)
        }
        "report" => Ok(service::report(proj()?, &profiles)),
        "gaps" => Ok(crate::gaps::gaps(proj()?, &profiles)),
        "config-check" => {
            let pr = proj()?;
            match (&pr.config_path, &pr.config_check) {
                (None, _) => Err(format!("no configuration: {} does not exist (create it, see `kisi config-template`)", service::config_path_for(&pr.pcb_path).display())),
                (Some(f), Some(c)) => Ok(json!({"file": f.display().to_string(), "ok": c.ok, "entries": c.entries, "errors": c.errors, "warnings": c.warnings})),
                (Some(f), None) => Ok(json!({"file": f.display().to_string(), "ok": true})),
            }
        }
        "config-template" => Ok(config_template(proj()?)),
        c => Err(format!("unknown command {c}")),
    }
}

/// What `report` prints after writing the file.
pub fn report_summary(path: &str, rep: &Value) -> Value {
    json!({"written": path, "channels": rep["channels"]["summary"], "classes": rep["overview"]["classes"].as_array().map(|a| a.len()),
        "config": rep["config"], "field_solver": rep["overview"]["field_solver"]})
}

/// A `<board>.kisi.json` skeleton with this board's dielectrics and net classes.
pub fn config_template(p: &Project) -> Value {
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
    json!({
        "kisi_config": crate::config::VERSION,
        "board": p.pcb_path.file_name().map(|f| f.to_string_lossy().to_string()),
        "stackup": {"dielectrics": diel, "solder_mask": {"epsilon_r": null, "source": {"kind": "datasheet", "document": null}}},
        "net_classes": classes,
        "fields": [{"ref": "U1", "name": "PI_LOAD", "value": "+3V3=100mA", "source": {"kind": "datasheet", "url": null, "page": null, "note": "typical"}}],
        "notes": ["Template: keep only entries you fill in; keys starting with '_' are information and ignored. Every value needs a source (kind: datasheet, standard, application_note, measurement, simulation, estimate, assumption). Save as <board>.kisi.json next to the board and run `kisi config-check`."]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn args() {
        let a = |s: &str| s.split_whitespace().map(String::from).collect::<Vec<_>>();
        let o = parse_args(&a("check b.kicad_pcb --class PCIe=pcie3 --nets A,B --all --max 5 --formula")).unwrap();
        assert_eq!((o.cmd.as_str(), o.pos.len(), o.classes["PCIe"].as_str(), o.all, o.max, o.formula), ("check", 1, "pcie3", true, Some(5), true));
        assert_eq!(o.nets.unwrap(), vec!["A", "B"]);
        assert!(parse_args(&a("check b.kicad_pcb --class PCIe=pcie9")).is_err());
        assert!(parse_args(&a("net b.kicad_pcb")).is_err());
        assert!(parse_args(&a("overview")).is_err());
        assert!(parse_args(&a("bogus x")).is_err());
        assert_eq!(parse_args(&a("profiles")).unwrap().cmd, "profiles");
        assert_eq!(parse_args(&a("--help")).unwrap().cmd, "help");
    }
}

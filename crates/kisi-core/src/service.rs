//! Front-end services: load a project from disk and produce compact JSON
//! summaries of the analyses. Used by the MCP server and the CLI's JSON
//! report; the viewer calls the analyses directly through WASM.
//!
//! Every summary states which models were used (formula vs. field solver,
//! assumed profiles) so an agent can report it truthfully.

use crate::board::{Board, StackupOverride};
use crate::eye;
use crate::extract::{analyze, Analysis, Params};
use crate::profile::{judge, Metrics, Profiles, Status};
use crate::rf::{self, RfParams};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub struct Project {
    pub pcb_path: PathBuf,
    pub name: String,
    pub board: Board,
    pub an: Analysis,
    pub params: Params,
}

impl Project {
    /// Load `board.kicad_pcb` (and `board.kicad_pro` next to it if `pro` is None).
    pub fn load(pcb: &Path, pro: Option<&Path>, stackup: Option<&StackupOverride>) -> Result<Project, String> {
        let text = std::fs::read_to_string(pcb).map_err(|e| format!("{}: {e}", pcb.display()))?;
        let mut board = crate::load_pcb(&text).map_err(|e| e.to_string())?;
        let pro_path = pro.map(Path::to_path_buf).unwrap_or_else(|| pcb.with_extension("kicad_pro"));
        if let Ok(p) = std::fs::read_to_string(&pro_path) {
            if let Err(e) = crate::apply_project(&mut board, &p) {
                board.warnings.push(format!("project file: {e}"));
            }
        }
        if let Some(ov) = stackup {
            let su = board.stackup().with_overrides(ov);
            board.stackup = Some(su);
        }
        let params = Params { step: 0.5, coplanar_factor: 3.0, solder_mask: true, ..Default::default() };
        let an = analyze(&board, &params);
        let name = pcb.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        Ok(Project { pcb_path: pcb.to_path_buf(), name, board, an, params })
    }

    pub fn net_id(&self, name: &str) -> Option<u32> {
        crate::pi::find_net(&self.board, name)
    }

    fn target(&self, net: u32) -> Option<(f64, bool)> {
        let ci = self.board.net_class.get(net as usize).copied().unwrap_or(0) as usize;
        self.an.classes.get(ci).and_then(|c| c.target).map(|t| (t.z, t.diff))
    }

    fn class_name(&self, net: u32) -> String {
        self.an.nets.iter().find(|n| n.net == net).map(|n| n.class.clone()).unwrap_or_default()
    }
}

const MODELS: &str = "Impedance by formula (Hammerstad-Jensen, Wheeler, Wadell, IPC-2141); the 2D field solver runs only in the viewer. Losses: Wheeler/Huray/Djordjevic-Sarkar.";

/// Board overview: stackup, classes with targets, impedance compliance per class.
pub fn overview(p: &Project) -> Value {
    let su = p.board.stackup();
    let stack: Vec<Value> = su
        .copper
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let d = su.dielectrics.get(i);
            json!({"layer": c.name, "cu_um": c.thickness * 1e3,
                   "dielectric_below": d.map(|d| json!({"material": d.material, "thickness_um": d.thickness * 1e3, "er": d.epsilon_r, "tand": d.loss_tangent}))})
        })
        .collect();
    let mut classes = Vec::new();
    for c in &p.an.classes {
        let Some(t) = c.target else { continue };
        let nets: Vec<_> = p.an.nets.iter().filter(|n| n.class == c.name && n.target.is_some()).collect();
        if nets.is_empty() {
            continue;
        }
        let len: f64 = nets.iter().map(|n| n.length).sum();
        let ok: f64 = nets.iter().map(|n| n.length * n.within5.unwrap_or(0.0)).sum();
        let mut worst: Vec<_> = nets.iter().filter(|n| n.z_mean.is_some()).collect();
        worst.sort_by(|a, b| {
            let da = (a.z_mean.unwrap() - t.z).abs();
            let db = (b.z_mean.unwrap() - t.z).abs();
            db.partial_cmp(&da).unwrap()
        });
        classes.push(json!({
            "class": c.name, "target_ohm": t.z, "diff": t.diff, "target_source": c.target_source, "nets": nets.len(),
            "length_mm": len, "within_5pct": if len > 0.0 { ok / len } else { 0.0 },
            "worst": worst.iter().take(5).map(|n| json!({"net": n.name, "z_mean": n.z_mean, "z_min": n.z_min, "z_max": n.z_max, "within_5pct": n.within5})).collect::<Vec<_>>()
        }));
    }
    json!({
        "board": p.name, "file": p.pcb_path.display().to_string(), "layers": su.n(), "stackup_from_file": su.from_file,
        "stackup": stack, "classes": classes, "warnings": p.board.warnings, "models": MODELS
    })
}

/// One net: impedance along the path, losses, TDR extremes, vias, crosstalk.
pub fn net_detail(p: &Project, net_name: &str, rp: &RfParams) -> Result<Value, String> {
    let net = p.net_id(net_name).ok_or_else(|| format!("net '{net_name}' not found"))?;
    let st = p.an.nets.iter().find(|n| n.net == net);
    let field = HashMap::new();
    let ch = rf::channel(&p.board, &p.an, net, &field, p.target(net), rp)?;
    let xt = rf::crosstalk(&p.board, &p.an, net, &field, rp);
    let su = p.board.stackup();
    let tdr_in: Vec<f64> = ch.tdr_z.iter().zip(&ch.tdr_x_mm).filter(|(_, x)| **x <= ch.length_mm).map(|(z, _)| *z).collect();
    let k = if ch.diff { 2.0 } else { 1.0 };
    Ok(json!({
        "net": ch.name, "class": st.map(|s| s.class.clone()), "target": st.and_then(|s| s.target).map(|t| json!({"ohm": t.z, "diff": t.diff})),
        "differential": ch.diff, "length_mm": ch.length_mm, "delay_ps": ch.delay_ps, "vias": ch.vias,
        "impedance": st.map(|s| json!({"mean": s.z_mean, "min": s.z_min, "max": s.z_max, "within_5pct": s.within5})),
        "segments": ch.segments.iter().filter(|s| s.kind == "line").map(|s| json!({"layer": su.copper[s.layer as usize].name, "len_mm": s.len_mm, "z_ohm_per_line": s.z})).collect::<Vec<_>>(),
        "insertion_loss_db": ch.il_marks.iter().map(|(f, d)| json!({"ghz": f, "db": d})).collect::<Vec<_>>(),
        "return_loss_worst_db": -ch.s11_db.iter().cloned().fold(f64::NEG_INFINITY, f64::max),
        "tdr": {"rise_ps": rp.rise * 1e12, "z_min": tdr_in.iter().cloned().fold(f64::INFINITY, f64::min) * k, "z_max": tdr_in.iter().cloned().fold(f64::NEG_INFINITY, f64::max) * k, "mode": if ch.diff { "differential" } else { "single-ended" }},
        "via_details": ch.via_info.iter().map(|v| json!({"from": su.copper[v.layer_in as usize].name, "to": su.copper[v.layer_out as usize].name, "drill_mm": v.drill_mm, "antipad_mm": v.antipad_mm, "z_ohm": v.z_via, "stub_mm": v.stub_mm, "stub_resonance_ghz": v.stub_f_ghz, "return_path": v.return_kind})).collect::<Vec<_>>(),
        "crosstalk": {"next_total": xt.next_total, "fext_total": xt.fext_total, "rise_ps": xt.rise_ps,
            "aggressors": xt.aggressors.iter().take(5).map(|a| json!({"net": a.name, "parallel_mm": a.coupled_mm, "gap_min_mm": a.gap_min_mm, "next": a.next, "fext": a.fext})).collect::<Vec<_>>()},
        "warnings": ch.warnings, "models": MODELS
    }))
}

/// Channel check of all nets with a target (one row per pair) against their profiles.
/// `assign`: class name → profile id (others automatic).
pub fn channel_check(p: &Project, profiles: &Profiles, assign: &HashMap<String, String>, only: Option<&[String]>, rp: &RfParams) -> Value {
    let field = HashMap::new();
    let mut rows = Vec::new();
    for st in &p.an.nets {
        let Some(t) = st.target else { continue };
        if let Some(o) = only {
            if !o.iter().any(|n| n == &st.name || st.name.rsplit('/').next() == Some(n.as_str())) {
                continue;
            }
        }
        if let Some(pn) = st.partner {
            let pname = p.board.net_name(pn);
            let neg = |s: &str| s.ends_with('N') || s.ends_with('-') || s.ends_with("_M");
            if if neg(&st.name) != neg(pname) { neg(&st.name) } else { pn < st.net } {
                continue;
            }
        }
        let class = p.class_name(st.net);
        let pid = assign.get(&class).cloned().unwrap_or_else(|| profiles.auto(&st.name, &class, t.diff).to_string());
        let Some(prof) = profiles.get(&pid) else { continue };
        let lim = profiles.limits(prof);
        let row = eye::check_net(&p.board, &p.an, &field, st.net, Some((t.z, t.diff)), rp, prof.rate_gbps, (Some(prof.tx_model()), Some(prof.rx), Some(prof.tx.rise_ps * 1e-12)));
        let m = Metrics {
            z_mean: st.z_mean,
            z_min: st.z_min,
            z_max: st.z_max,
            il_nyq_db: Some(row.il_nyq_db),
            rl_db: Some(row.rl_db),
            eye_h: row.error.is_none().then_some(row.eye_height),
            eye_w_ui: row.error.is_none().then_some(row.eye_width_ui),
            skew_ps: row.skew_ps,
            skew_uncertain: row.skew_uncertain,
            stub_ghz: row.stub_ghz,
            error: row.error.clone(),
        };
        let v = judge(&m, prof, &lim);
        rows.push((v.status, json!({"net": st.name, "class": class, "profile": prof.id, "rate_gbps": prof.rate_gbps, "status": v.status, "criteria": v.criteria,
            "length_mm": row.length_mm, "vias": row.vias, "metrics": m})));
    }
    rows.sort_by(|a, b| a.0.cmp(&b.0));
    let count = |s: Status| rows.iter().filter(|r| r.0 == s).count();
    json!({
        "summary": {"green": count(Status::Green), "yellow": count(Status::Yellow), "red": count(Status::Red)},
        "rows": rows.into_iter().map(|r| r.1).collect::<Vec<_>>(),
        "models": format!("{MODELS} Eye: PRBS7, linear transmitter/receiver from the profile, without equalisation, jitter and noise.")
    })
}

/// Power: tree balance (PI_REG fields or a JSON tree), DC drop of every rail,
/// optional PDN impedance of one rail.
pub fn power(p: &Project, tree: Option<crate::tree::TreeSpec>, pdn_rail: Option<&str>) -> Result<Value, String> {
    let (spec, mut warnings, source) = match tree {
        Some(t) => (Some(t), vec![], "JSON"),
        None => match crate::tree::tree_from_fields(&p.board) {
            Some((t, w)) => (Some(t), w, "PI_REG fields"),
            None => (None, vec![], "PI_LOAD/PI_SOURCE fields"),
        },
    };
    let (setups, balance) = match &spec {
        Some(s) => {
            let t = crate::tree::solve_tree(s, Some(&p.board))?;
            warnings.extend(t.warnings.iter().cloned());
            let bal = json!({"input_w": t.p_input, "loads_w": t.p_loads, "losses_w": t.p_losses,
                "elements": t.elements.iter().map(|e| json!({"id": e.id, "kind": e.kind, "part": e.part, "i_in": e.i_in, "i_out": e.i_out, "loss_w": e.p_loss, "overload": e.overload})).collect::<Vec<_>>()});
            (t.setups, Some(bal))
        }
        None => {
            let (s, w) = crate::pi::rails_from_fields(&p.board);
            warnings.extend(w);
            (s, None)
        }
    };
    let mut rails = Vec::new();
    for s in &setups {
        match crate::pi::solve_rail(&p.board, s, &crate::pi::PiParams::default()) {
            Ok(r) => rails.push(json!({"rail": r.net, "voltage": r.voltage, "load_a": r.total_load, "max_drop_mv": r.max_drop_mv, "copper_loss_mw": r.power_loss_w * 1e3, "max_j_a_mm2": r.max_j, "warnings": r.warnings})),
            Err(e) => rails.push(json!({"rail": s.net, "error": e})),
        }
    }
    let pdn = match pdn_rail {
        Some(name) => {
            let setup = setups.iter().find(|r| r.net == name || r.net.rsplit('/').next() == Some(name)).cloned().unwrap_or_else(|| crate::pi::RailSetup {
                net: name.to_string(),
                voltage: crate::pi::voltage_from_name(name).unwrap_or(0.0),
                ..Default::default()
            });
            let r = crate::pdn::pdn(&p.board, &setup, &crate::pdn::PdnParams::default())?;
            let at = |f: f64| r.freq.iter().position(|&x| x >= f).map(|i| r.z[i]);
            Some(json!({"rail": r.net, "target_ohm": r.target_ohm, "above_target_hz": r.above_target, "c_total_f": r.c_total, "c_plane_f": r.c_plane,
                "cavity_first_resonance_hz": r.cavity_hz, "z_ohm": {"1k": at(1e3), "100k": at(1e5), "1M": at(1e6), "10M": at(1e7), "100M": at(1e8)},
                "caps": r.caps.len(), "warnings": r.warnings}))
        }
        None => None,
    };
    Ok(json!({"tree_source": source, "balance": balance, "rails": rails, "pdn": pdn, "warnings": warnings}))
}

/// Everything in one document, written as `analysis/kisi.json` next to the board
/// (layout of kicad-happy's analysis folder).
pub fn report(p: &Project, profiles: &Profiles) -> Value {
    let rp = RfParams::default();
    json!({
        "tool": "kisi", "version": env!("CARGO_PKG_VERSION"),
        "overview": overview(p),
        "channels": channel_check(p, profiles, &HashMap::new(), None, &rp),
        "power": power(p, None, None).unwrap_or_else(|e| json!({"error": e})),
    })
}

//! Front-end services: load a project from disk and produce compact JSON
//! summaries of the analyses. Used by the CLI's JSON commands (the interface for
//! agents, see the skills in claude-plugin/); the viewer calls the analyses
//! directly through WASM.
//!
//! Every summary states which models were used (formula vs. field solver,
//! assumed profiles) so an agent can report it truthfully.

use crate::board::{Board, StackupOverride};
use crate::eye;
use crate::extract::{analyze, Analysis, Params};
use crate::profile::{judge, Metrics, Profiles, Status};
use crate::rf::{self, RfParams};
use crate::topology;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub struct Project {
    pub pcb_path: PathBuf,
    pub name: String,
    pub board: Board,
    pub an: Analysis,
    pub params: Params,
    /// `<board>.kisi.json` (values with sources), if present
    pub config_path: Option<PathBuf>,
    pub config: Option<crate::config::BoardConfig>,
    /// check of the configuration against the board, or the parse error
    pub config_check: Option<crate::config::CheckResult>,
    /// net class → profile id from the configuration
    pub config_profiles: HashMap<String, String>,
}

/// File contents a project is built from (see `Project::from_texts`).
pub struct Inputs {
    /// path of the board (name, location of analysis/ and kisi.json; not read here)
    pub pcb_path: PathBuf,
    pub pcb: String,
    pub pro: Option<String>,
    /// (path, contents or read error) of `<board>.kisi.json`
    pub config: Option<(PathBuf, Result<String, String>)>,
    pub stackup: Option<StackupOverride>,
    /// field-solver results by geometry id (`Analysis::geometries`)
    pub field: HashMap<String, crate::extract::FieldResult>,
}

/// `<board>.kisi.json` next to `board.kicad_pcb`.
pub fn config_path_for(pcb: &Path) -> PathBuf {
    let stem = pcb.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    pcb.with_file_name(format!("{stem}.kisi.json"))
}

impl Project {
    /// Load `board.kicad_pcb` (and `board.kicad_pro` next to it if `pro` is None), with
    /// `board.kisi.json` next to it if present.
    pub fn load(pcb: &Path, pro: Option<&Path>, stackup: Option<&StackupOverride>) -> Result<Project, String> {
        let cp = config_path_for(pcb);
        Self::load_with(pcb, pro, stackup, cp.is_file().then_some(cp.as_path()))
    }

    /// Like `load`, with an explicit configuration file (None: no configuration).
    pub fn load_with(pcb: &Path, pro: Option<&Path>, stackup: Option<&StackupOverride>, config: Option<&Path>) -> Result<Project, String> {
        let text = std::fs::read_to_string(pcb).map_err(|e| format!("{}: {e}", pcb.display()))?;
        let pro_path = pro.map(Path::to_path_buf).unwrap_or_else(|| pcb.with_extension("kicad_pro"));
        let pro_text = std::fs::read_to_string(&pro_path).ok();
        let config = match config {
            Some(cp) => Some((cp.to_path_buf(), std::fs::read_to_string(cp).map_err(|e| format!("{}: {e}", cp.display())))),
            None => None,
        };
        Self::from_texts(Inputs { pcb_path: pcb.to_path_buf(), pcb: text, pro: pro_text, config, stackup: stackup.cloned(), field: HashMap::new() })
    }

    /// Build a project from file contents (no file system access: also used in WASM).
    pub fn from_texts(inp: Inputs) -> Result<Project, String> {
        let mut board = crate::load_pcb(&inp.pcb).map_err(|e| e.to_string())?;
        if let Some(p) = &inp.pro {
            if let Err(e) = crate::apply_project(&mut board, p) {
                board.warnings.push(format!("project file: {e}"));
            }
        }
        // configuration: checked against the board as read, then laid over it
        let mut cfg = None;
        let mut cfg_check = None;
        let mut applied = crate::config::Applied::default();
        if let Some((_, text)) = &inp.config {
            match text.clone().and_then(|t| crate::config::parse(&t)) {
                Ok(c) => {
                    let an0 = crate::extract::skeleton(&board, &Params::default());
                    let classes: Vec<String> = an0.classes.iter().map(|c| c.name.clone()).collect();
                    let bn = inp.pcb_path.file_name().map(|f| f.to_string_lossy().to_string());
                    cfg_check = Some(crate::config::check(&c, &board, &classes, &Profiles::builtin(), bn.as_deref()));
                    applied = c.apply(&mut board);
                    cfg = Some(c);
                }
                Err(e) => cfg_check = Some(crate::config::CheckResult { ok: false, errors: vec![e], ..Default::default() }),
            }
        }
        if let Some(ov) = &inp.stackup {
            let su = board.stackup().with_overrides(ov);
            board.stackup = Some(su);
        }
        let params = Params { step: 0.5, coplanar_factor: 3.0, solder_mask: true, targets: applied.targets.clone(), field: inp.field, ..Default::default() };
        let an = analyze(&board, &params);
        let name = inp.pcb_path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        Ok(Project { pcb_path: inp.pcb_path, name, board, an, params, config_path: inp.config.map(|c| c.0), config: cfg, config_check: cfg_check, config_profiles: applied.profiles })
    }

    /// Field-solver coverage of the analysis (share of the routed length with a target).
    pub fn field_summary(&self) -> Value {
        let g = &self.an.geometries;
        let total: f64 = g.iter().map(|e| e.target_length).sum();
        let solved: Vec<&crate::extract::GeomEntry> = g.iter().filter(|e| self.params.field.contains_key(&e.id)).collect();
        let cov: f64 = solved.iter().map(|e| e.target_length).sum();
        json!({"cross_sections_field": solved.len(), "cross_sections": g.len(),
            "target_length_coverage_pct": if total > 0.0 { (1000.0 * cov / total).round() / 10.0 + 0.0 } else { 0.0 }})
    }

    /// Model description for the summaries, depending on field-solver coverage.
    pub fn models(&self) -> String {
        let n = self.params.field.len();
        let imp = if n == 0 {
            "Impedance by formula (Hammerstad-Jensen, Wheeler, Wadell, IPC-2141); differential and coplanar lines can be off by 5–15 %.".to_string()
        } else {
            format!("Impedance from Forstén's 2D field solver for {n} cross-sections (see field_solver: coverage), formula (Hammerstad-Jensen, Wheeler, Wadell, IPC-2141) for the rest.")
        };
        format!("{imp} Losses: Wheeler/Huray/Djordjevic-Sarkar.")
    }

    pub fn net_id(&self, name: &str) -> Option<u32> {
        crate::pi::find_net(&self.board, name)
    }

    fn target(&self, net: u32) -> Option<(f64, bool)> {
        let ci = self.board.net_class.get(net as usize).copied().unwrap_or(0) as usize;
        self.an.classes.get(ci).and_then(|c| c.target).map(|t| (t.z, t.diff))
    }

    /// Profile assignment: the configuration's, overridden by `extra`.
    pub fn assign(&self, extra: &HashMap<String, String>) -> HashMap<String, String> {
        let mut m = self.config_profiles.clone();
        m.extend(extra.iter().map(|(k, v)| (k.clone(), v.clone())));
        m
    }

    /// Which configuration was used (for every summary).
    pub fn config_summary(&self) -> Value {
        match (&self.config_path, &self.config_check) {
            (None, _) => json!({"file": null, "note": "no <board>.kisi.json next to the board: values from the board file only"}),
            (Some(p), Some(c)) => json!({"file": p.display().to_string(), "entries": c.entries, "ok": c.ok, "errors": c.errors, "warnings": c.warnings}),
            (Some(p), None) => json!({"file": p.display().to_string()}),
        }
    }

    /// Lanes grouped into buses.
    pub fn buses(&self) -> Vec<topology::Bus> {
        topology::buses(&self.board, &self.an, &topology::ComponentData::builtin())
    }

    fn class_name(&self, net: u32) -> String {
        self.an.nets.iter().find(|n| n.net == net).map(|n| n.class.clone()).unwrap_or_default()
    }
}


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
                   "dielectric_below": d.map(|d| json!({"material": d.material, "thickness_um": d.thickness * 1e3, "er": d.epsilon_r, "tand": d.loss_tangent, "assumed": d.assumed}))})
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
        "stackup": stack, "classes": classes, "warnings": p.board.warnings, "config": p.config_summary(), "field_solver": p.field_summary(), "models": p.models()
    })
}

/// S-parameters of one net's channel as Touchstone (.s2p) text.
pub fn net_touchstone(p: &Project, net_name: &str, rp: &RfParams) -> Result<String, String> {
    let net = p.net_id(net_name).ok_or_else(|| format!("net '{net_name}' not found"))?;
    let ch = rf::channel_dir(&p.board, &p.an, net, &p.params.field, p.target(net), rp, false, lane_hint(p, net))?;
    Ok(rf::touchstone(&ch))
}

/// Start hint of a net's path: the transmitter side of the lane the net belongs to.
fn lane_hint(p: &Project, net: u32) -> Option<crate::geom::P> {
    for b in p.buses() {
        for l in &b.lanes {
            let rev = l.rx_end == 0;
            for h in l.hops.iter().chain(&l.partner_hops) {
                if let topology::Hop::Net { net: n, from, to } = h {
                    if *n == net {
                        return Some(if rev { *to } else { *from });
                    }
                }
            }
        }
    }
    None
}

fn path_json(ch: &rf::Channel) -> Value {
    match &ch.path {
        Some(pa) => json!({"start": pa.start.label, "end": pa.end.label, "direction": pa.direction, "length_mm": pa.length_mm,
            "branches": pa.branches.iter().map(|b| json!({"at_mm": b.at_s, "len_mm": b.len_mm, "to": b.end.label, "modelled": b.modelled})).collect::<Vec<_>>()}),
        None => Value::Null,
    }
}

/// One net: impedance along the path, losses, TDR extremes, vias, crosstalk.
pub fn net_detail(p: &Project, net_name: &str, rp: &RfParams) -> Result<Value, String> {
    let net = p.net_id(net_name).ok_or_else(|| format!("net '{net_name}' not found"))?;
    let st = p.an.nets.iter().find(|n| n.net == net);
    let field = &p.params.field;
    let ch = rf::channel_dir(&p.board, &p.an, net, field, p.target(net), rp, false, lane_hint(p, net))?;
    let xt = rf::crosstalk(&p.board, &p.an, net, field, rp);
    let su = p.board.stackup();
    let tdr_in: Vec<f64> = ch.tdr_z.iter().zip(&ch.tdr_x_mm).filter(|(_, x)| **x <= ch.length_mm).map(|(z, _)| *z).collect();
    let k = if ch.diff { 2.0 } else { 1.0 };
    Ok(json!({
        "net": ch.name, "class": st.map(|s| s.class.clone()), "target": st.and_then(|s| s.target).map(|t| json!({"ohm": t.z, "diff": t.diff})),
        "differential": ch.diff, "length_mm": ch.length_mm, "delay_ps": ch.delay_ps, "vias": ch.vias, "path": path_json(&ch),
        "impedance": st.map(|s| json!({"mean": s.z_mean, "min": s.z_min, "max": s.z_max, "within_5pct": s.within5})),
        "segments": ch.segments.iter().filter(|s| s.kind == "line").map(|s| json!({"layer": su.copper[s.layer as usize].name, "len_mm": s.len_mm, "z_ohm_per_line": s.z})).collect::<Vec<_>>(),
        "insertion_loss_db": ch.il_marks.iter().map(|(f, d)| json!({"ghz": f, "db": d})).collect::<Vec<_>>(),
        "return_loss_worst_db": -ch.s11_db.iter().cloned().fold(f64::NEG_INFINITY, f64::max),
        "return_loss_worst_db_b": -ch.s22_db.iter().cloned().fold(f64::NEG_INFINITY, f64::max),
        "tdr": {"rise_ps": rp.rise * 1e12, "z_min": tdr_in.iter().cloned().fold(f64::INFINITY, f64::min) * k, "z_max": tdr_in.iter().cloned().fold(f64::NEG_INFINITY, f64::max) * k, "mode": if ch.diff { "differential" } else { "single-ended" }},
        "via_details": ch.via_info.iter().map(|v| json!({"from": su.copper[v.layer_in as usize].name, "to": su.copper[v.layer_out as usize].name, "drill_mm": v.drill_mm, "antipad_mm": v.antipad_mm, "z_ohm": v.z_via, "stub_mm": v.stub_mm, "stub_resonance_ghz": v.stub_f_ghz, "return_path": v.return_kind})).collect::<Vec<_>>(),
        "crosstalk": {"next_total": xt.next_total, "fext_total": xt.fext_total, "rise_ps": xt.rise_ps,
            "aggressors": xt.aggressors.iter().take(5).map(|a| json!({"net": a.name, "parallel_mm": a.coupled_mm, "gap_min_mm": a.gap_min_mm, "next": a.next, "fext": a.fext})).collect::<Vec<_>>()},
        "warnings": ch.warnings, "field_solver": p.field_summary(), "models": p.models()
    }))
}

fn pin(e: &topology::EndPin) -> String {
    format!("{}.{}", e.refdes, e.pin)
}

fn lane_json(p: &Project, l: &topology::Lane) -> Value {
    let rev = l.rx_end == 0;
    let (tx, rx) = if l.ends.len() == 2 { (Some(&l.ends[if rev { 1 } else { 0 }]), Some(&l.ends[if rev { 0 } else { 1 }])) } else { (None, None) };
    json!({
        "lane": l.name, "kind": l.kind, "diff": l.diff,
        "from": tx.map(pin), "to": rx.map(pin), "direction": l.direction,
        "ends": l.ends.iter().map(pin).collect::<Vec<_>>(),
        "nets": l.nets.iter().map(|&n| p.board.net_name(n)).collect::<Vec<_>>(),
        "series_parts": l.hops.iter().filter_map(|h| match h { topology::Hop::Series { part } => Some(format!("{} {} ({})", part.refdes, part.value, part.package)), _ => None }).collect::<Vec<_>>(),
        "terminations": l.shunts.iter().map(|s| format!("{} {} {}", s.part.refdes, s.part.value, if s.across { "across the pair".to_string() } else { format!("to {}", p.board.net_name(s.to)) })).collect::<Vec<_>>(),
        "branches": l.branches.len(),
        "not_modelled": l.others,
    })
}

/// Device-to-device lanes grouped into buses, with the profile each bus gets.
pub fn interfaces(p: &Project, profiles: &Profiles, assign: &HashMap<String, String>) -> Value {
    let assign = &p.assign(assign);
    let buses = p.buses();
    let list: Vec<Value> = buses
        .iter()
        .map(|b| {
            let l0 = &b.lanes[0];
            let tnet = l0.nets.iter().copied().find(|&n| p.target(n).is_some()).unwrap_or(l0.nets[0]);
            let pid = assign.get(&b.class).cloned().unwrap_or_else(|| profiles.auto(p.board.net_name(tnet), &b.class, l0.diff).to_string());
            json!({"bus": b.name, "kind": b.kind, "class": b.class, "profile": pid, "devices": b.devices, "values": b.values,
                "lanes": b.lanes.iter().map(|l| lane_json(p, l)).collect::<Vec<_>>()})
        })
        .collect();
    json!({
        "buses": list,
        "config": p.config_summary(),
        "notes": "Lanes follow nets through series resistors (≤ 100 Ω), AC-coupling capacitors (≥ 1 nF) and small inductors; DNP parts are skipped. kind: ic-ic (eye checked), ic-connector (board share only, no eye), multi-drop/open (not checked). Direction is a guess (termination next to the receiver), parasitics of passives are estimates from data/components.json."
    })
}

/// Channel check of all device-to-device lanes (plus nets with a target that are
/// not on any lane), judged against the interface profiles.
pub fn channel_check(p: &Project, profiles: &Profiles, assign: &HashMap<String, String>, only: Option<&[String]>, rp: &RfParams) -> Value {
    let assign = &p.assign(assign);
    let field = &p.params.field;
    let cd = topology::ComponentData::builtin();
    let buses = p.buses();
    let wanted = |names: &mut dyn Iterator<Item = &str>| -> bool {
        match only {
            None => true,
            Some(o) => {
                for nm in names {
                    if o.iter().any(|n| n == nm || nm.rsplit('/').next() == Some(n.as_str())) {
                        return true;
                    }
                }
                false
            }
        }
    };
    let mut rows = Vec::new();
    let mut on_lane: std::collections::HashSet<u32> = std::collections::HashSet::new();
    for b in &buses {
        for l in &b.lanes {
            on_lane.extend(l.nets.iter().chain(&l.partner_nets).copied());
            if !matches!(l.kind.as_str(), "ic-ic" | "ic-connector") {
                continue;
            }
            if !wanted(&mut l.nets.iter().map(|&n| p.board.net_name(n))) {
                continue;
            }
            let tnet = l.nets.iter().copied().find(|&n| p.target(n).is_some()).unwrap_or(l.nets[0]);
            let class = p.class_name(tnet);
            let pid = assign.get(&class).cloned().unwrap_or_else(|| profiles.auto(p.board.net_name(tnet), &class, l.diff).to_string());
            let Some(prof) = profiles.get(&pid) else { continue };
            let lim = profiles.limits(prof);
            let row = topology::check_lane(&p.board, &p.an, field, l, p.target(tnet), rp, prof.rate_gbps, (Some(prof.tx_model()), Some(prof.rx), Some(prof.tx.rise_ps * 1e-12)), l.rx_end == 0, &cd);
            // length-weighted impedance of the lane's nets
            let (mut len, mut zs, mut lo, mut hi) = (0.0, 0.0, f64::INFINITY, f64::NEG_INFINITY);
            for st in p.an.nets.iter().filter(|s| l.nets.contains(&s.net)) {
                if let (Some(z), Some(a), Some(c)) = (st.z_mean, st.z_min, st.z_max) {
                    len += st.length;
                    zs += z * st.length;
                    lo = lo.min(a);
                    hi = hi.max(c);
                }
            }
            let m = Metrics {
                z_mean: (len > 0.0).then(|| zs / len),
                z_min: (len > 0.0).then_some(lo),
                z_max: (len > 0.0).then_some(hi),
                il_nyq_db: Some(row.il_nyq_db),
                rl_db: Some(row.rl_db),
                eye_h: (row.eye && row.error.is_none()).then_some(row.eye_height),
                eye_w_ui: (row.eye && row.error.is_none()).then_some(row.eye_width_ui),
                skew_ps: row.skew_ps,
                skew_uncertain: row.skew_uncertain,
                stub_ghz: row.stub_ghz,
                error: row.error.clone(),
            };
            let v = judge(&m, prof, &lim);
            let mut j = lane_json(p, l);
            j["bus"] = json!(b.name);
            j["class"] = json!(class);
            j["profile"] = json!(prof.id);
            j["rate_gbps"] = json!(prof.rate_gbps);
            j["status"] = json!(v.status);
            j["criteria"] = json!(v.criteria);
            j["length_mm"] = json!(row.length_mm);
            j["vias"] = json!(row.vias);
            j["eye_computed"] = json!(row.eye);
            j["metrics"] = json!(m);
            rows.push((v.status, j));
        }
    }
    // nets with a target that are on no lane (no devices found at their ends)
    for st in &p.an.nets {
        let Some(t) = st.target else { continue };
        if on_lane.contains(&st.net) || !wanted(&mut std::iter::once(st.name.as_str())) {
            continue;
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
        let row = eye::check_net(&p.board, &p.an, field, st.net, Some((t.z, t.diff)), rp, prof.rate_gbps, (Some(prof.tx_model()), Some(prof.rx), Some(prof.tx.rise_ps * 1e-12)));
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
        rows.push((v.status, json!({"net": st.name, "kind": "net (no devices found)", "class": class, "profile": prof.id, "rate_gbps": prof.rate_gbps, "status": v.status, "criteria": v.criteria,
            "length_mm": row.length_mm, "vias": row.vias, "metrics": m})));
    }
    rows.sort_by(|a, b| a.0.cmp(&b.0));
    let count = |s: Status| rows.iter().filter(|r| r.0 == s).count();
    json!({
        "summary": {"green": count(Status::Green), "yellow": count(Status::Yellow), "red": count(Status::Red)},
        "config": p.config_summary(),
        "rows": rows.into_iter().map(|r| r.1).collect::<Vec<_>>(),
        "field_solver": p.field_summary(),
        "models": format!("{} Lanes run from device pin to device pin through series parts (estimated parasitics). Eye only from IC to IC: PRBS7, linear transmitter/receiver from the profile (a termination next to the receiver acts as its termination), without equalisation, jitter and noise. Lanes ending at a connector: board share only (loss, return loss, skew).", p.models())
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
    let mut tree_res = None;
    let (setups, balance) = match &spec {
        Some(s) => {
            let t = crate::tree::solve_tree(s, Some(&p.board))?;
            warnings.extend(t.warnings.iter().cloned());
            let bal = json!({"input_w": t.p_input, "loads_w": t.p_loads, "losses_w": t.p_losses,
                "elements": t.elements.iter().map(|e| json!({"id": e.id, "kind": e.kind, "part": e.part, "i_in": e.i_in, "i_out": e.i_out, "loss_w": e.p_loss, "overload": e.overload})).collect::<Vec<_>>()});
            let setups = t.setups.clone();
            tree_res = Some(t);
            (setups, Some(bal))
        }
        None => {
            let (s, w) = crate::pi::rails_from_fields(&p.board);
            warnings.extend(w);
            (s, None)
        }
    };
    let params = crate::pi::PiParams::default();
    let mut rails = Vec::new();
    let mut solved = Vec::new();
    let rail_json = |r: &crate::pi::PiResult| {
        if r.ground {
            json!({"rail": r.net, "ground_return": true, "return_a": -r.total_load, "max_offset_mv": r.max_drop_mv, "copper_loss_mw": r.power_loss_w * 1e3, "max_j_a_mm2": r.max_j, "warnings": r.warnings})
        } else {
            json!({"rail": r.net, "voltage": r.voltage, "load_a": r.total_load, "max_drop_mv": r.max_drop_mv, "copper_loss_mw": r.power_loss_w * 1e3, "max_j_a_mm2": r.max_j, "warnings": r.warnings})
        }
    };
    for s in &setups {
        match crate::pi::solve_rail(&p.board, s, &params) {
            Ok(r) => {
                rails.push(rail_json(&r));
                solved.push(r);
            }
            Err(e) => rails.push(json!({"rail": s.net, "error": e})),
        }
    }
    // ground return (needs the regulator chain: who returns how much current)
    if let Some(t) = &tree_res {
        let (gs, gw) = crate::losses::ground_setups(&p.board, t);
        warnings.extend(gw);
        for s in &gs {
            match crate::pi::solve_rail(&p.board, s, &params) {
                Ok(r) => {
                    rails.push(rail_json(&r));
                    solved.push(r);
                }
                Err(e) => rails.push(json!({"rail": s.net, "ground_return": true, "error": e})),
            }
        }
    }
    let refs: Vec<&crate::pi::PiResult> = solved.iter().collect();
    let ls = crate::losses::loss_summary(&p.board, tree_res.as_ref(), &refs);
    let losses = json!({
        "total_w": ls.total_w, "components_w": ls.components_w, "copper_rails_w": ls.copper_w, "copper_ground_w": ls.ground_w,
        "per_layer": ls.layers.iter().map(|(_, n, w)| json!({"layer": n, "copper_w": w})).collect::<Vec<_>>(),
        "per_rail": ls.rails.iter().map(|r| json!({"rail": r.net, "ground_return": r.ground, "copper_w": r.w})).collect::<Vec<_>>(),
        "components": ls.components.iter().take(25).map(|c| json!({"ref": c.refdes, "part": c.part, "heat_w": c.w, "role": c.role})).collect::<Vec<_>>(),
        "notes": ls.notes,
    });
    let flow = tree_res.as_ref().map(|t| {
        let copper: Vec<(String, f64)> = ls.rails.iter().filter(|r| !r.ground).map(|r| (r.net.clone(), r.w)).collect();
        crate::losses::power_flow(t, &copper, ls.ground_w)
    });
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
    Ok(json!({"tree_source": source, "balance": balance, "rails": rails, "losses": losses, "flow": flow, "pdn": pdn, "warnings": warnings}))
}

/// Everything in one document, written as `analysis/kisi.json` next to the board
/// (layout of kicad-happy's analysis folder).

pub fn report(p: &Project, profiles: &Profiles) -> Value {
    let rp = RfParams::default();
    json!({
        "tool": "kisi", "version": env!("CARGO_PKG_VERSION"),
        "config": p.config_summary(),
        "overview": overview(p),
        "interfaces": interfaces(p, profiles, &HashMap::new()),
        "channels": channel_check(p, profiles, &HashMap::new(), None, &rp),
        "power": power(p, None, None).unwrap_or_else(|e| json!({"error": e})),
    })
}

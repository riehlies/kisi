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
    /// IBIS files named in the footprints' simulation fields, loaded from disk
    pub ibis: topology::IbisLibs,
    /// (Sim.Library, reason) of referenced IBIS files that could not be read
    pub ibis_missing: Vec<(String, String)>,
    /// `<board>.kisi.json` (values with sources), if present
    pub config_path: Option<PathBuf>,
    pub config: Option<crate::config::BoardConfig>,
    /// check of the configuration against the board, or the parse error
    pub config_check: Option<crate::config::CheckResult>,
    /// net class → profile id from the configuration
    pub config_profiles: HashMap<String, String>,
}

/// `<board>.kisi.json` next to `board.kicad_pcb`.
pub fn config_path_for(pcb: &Path) -> PathBuf {
    let stem = pcb.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    pcb.with_file_name(format!("{stem}.kisi.json"))
}

/// Find an IBIS file named in Sim.Library: `${KIPRJMOD}` and other `${VAR}`s are
/// expanded; relative paths are taken from the board's folder; if that fails the
/// file name is searched in the board's folder and up to three levels below.
pub fn find_library(lib: &str, dir: &Path) -> Option<PathBuf> {
    let mut s = lib.trim().to_string();
    for (a, b) in [("${", "}"), ("$(", ")")] {
        while let Some(i) = s.find(a) {
            let Some(j) = s[i..].find(b).map(|j| i + j) else { break };
            let var = &s[i + 2..j];
            let val = if var == "KIPRJMOD" { dir.to_string_lossy().to_string() } else { std::env::var(var).unwrap_or_default() };
            s.replace_range(i..=j, &val);
        }
    }
    let p = PathBuf::from(&s);
    let cands = if p.is_absolute() { vec![p.clone()] } else { vec![dir.join(&p)] };
    if let Some(c) = cands.into_iter().find(|c| c.is_file()) {
        return Some(c);
    }
    let key = topology::IbisLibs::key(&s);
    fn walk(d: &Path, key: &str, depth: u32) -> Option<PathBuf> {
        let rd = std::fs::read_dir(d).ok()?;
        let mut dirs = vec![];
        for e in rd.flatten() {
            let p = e.path();
            if p.is_file() && p.file_name().is_some_and(|n| n.to_string_lossy().to_ascii_lowercase() == key) {
                return Some(p);
            }
            if p.is_dir() && depth > 0 && !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.')) {
                dirs.push(p);
            }
        }
        dirs.into_iter().find_map(|p| walk(&p, key, depth - 1))
    }
    walk(dir, &key, 3)
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
        let mut board = crate::load_pcb(&text).map_err(|e| e.to_string())?;
        let pro_path = pro.map(Path::to_path_buf).unwrap_or_else(|| pcb.with_extension("kicad_pro"));
        if let Ok(p) = std::fs::read_to_string(&pro_path) {
            if let Err(e) = crate::apply_project(&mut board, &p) {
                board.warnings.push(format!("project file: {e}"));
            }
        }
        // configuration: checked against the board as read, then laid over it
        let mut cfg = None;
        let mut cfg_check = None;
        let mut applied = crate::config::Applied::default();
        if let Some(cp) = config {
            let parsed = std::fs::read_to_string(cp).map_err(|e| format!("{}: {e}", cp.display())).and_then(|t| crate::config::parse(&t));
            match parsed {
                Ok(c) => {
                    let an0 = crate::extract::skeleton(&board, &Params::default());
                    let classes: Vec<String> = an0.classes.iter().map(|c| c.name.clone()).collect();
                    cfg_check = Some(crate::config::check(&c, &board, &classes, &Profiles::builtin()));
                    applied = c.apply(&mut board);
                    cfg = Some(c);
                }
                Err(e) => cfg_check = Some(crate::config::CheckResult { ok: false, errors: vec![e], ..Default::default() }),
            }
        }
        if let Some(ov) = stackup {
            let su = board.stackup().with_overrides(ov);
            board.stackup = Some(su);
        }
        let params = Params { step: 0.5, coplanar_factor: 3.0, solder_mask: true, targets: applied.targets.clone(), ..Default::default() };
        let an = analyze(&board, &params);
        let name = pcb.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        let dir = pcb.parent().map(Path::to_path_buf).unwrap_or_default();
        let mut ibis = topology::IbisLibs::default();
        let mut ibis_missing = vec![];
        for (lib, _) in topology::ibis_libraries(&board) {
            match find_library(&lib, &dir).map(|f| std::fs::read_to_string(&f).map_err(|e| format!("{}: {e}", f.display()))) {
                Some(Ok(t)) => {
                    let info = ibis.add(&lib, &t);
                    if let Some(e) = info.error {
                        ibis_missing.push((lib, e));
                    }
                }
                Some(Err(e)) => ibis_missing.push((lib, e)),
                None => ibis_missing.push((lib, "file not found".into())),
            }
        }
        Ok(Project { pcb_path: pcb.to_path_buf(), name, board, an, params, ibis, ibis_missing, config_path: config.map(Path::to_path_buf), config: cfg, config_check: cfg_check, config_profiles: applied.profiles })
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

    /// Lanes grouped into buses; IBIS model types fix the direction where they can.
    pub fn buses(&self) -> Vec<topology::Bus> {
        let cd = topology::ComponentData::builtin();
        let mut b = topology::buses(&self.board, &self.an, &cd);
        topology::apply_ibis_direction(&mut b, &self.ibis);
        b
    }

    /// IBIS files referenced by the board and their state.
    pub fn ibis_summary(&self) -> Value {
        let libs: Vec<Value> = topology::ibis_libraries(&self.board)
            .into_iter()
            .map(|(lib, devs)| {
                let err = self.ibis_missing.iter().find(|m| m.0 == lib).map(|m| m.1.clone());
                let comps = self.ibis.info().into_iter().find(|i| topology::IbisLibs::key(&i.name) == topology::IbisLibs::key(&lib)).map(|i| i.components).unwrap_or_default();
                json!({"library": lib, "devices": devs, "loaded": err.is_none(), "error": err, "components": comps})
            })
            .collect();
        json!(libs)
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
        "stackup": stack, "classes": classes, "warnings": p.board.warnings, "config": p.config_summary(), "models": MODELS
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

fn pin(e: &topology::EndPin) -> String {
    format!("{}.{}", e.refdes, e.pin)
}

fn ibis_json(e: &topology::IbisEnd) -> Value {
    match &e.linear {
        Some(l) => json!({"pin": format!("{}.{}", e.refdes, e.pin), "status": e.status, "component": l.component, "model": l.model, "model_type": l.model_type,
            "transmitter": l.tx.map(|t| json!({"r_ohm": t.r, "c_pf": t.c_pf, "l_nh": t.l_nh, "swing_open_v": l.v_swing, "swing_into_50_ohm_v": l.v_swing.map(|v| v * 50.0 / (t.r + 50.0)), "rise_10_90_ps": l.rise.map(|r| r * 1e12)})),
            "receiver": {"r_ohm": if l.rx.r > 0.0 { json!(l.rx.r) } else { json!("high impedance") }, "c_pf": l.rx.c_pf, "l_nh": l.rx.l_nh},
            "notes": l.notes, "estimate": "linearised IBIS (typical corner) – EXPERIMENTAL, not working yet: do not rely on it"}),
        None => json!({"pin": format!("{}.{}", e.refdes, e.pin), "status": e.status, "library": e.library, "message": e.message}),
    }
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
        "ibis": l.ends.iter().filter_map(|e| topology::ibis_end(e, &p.ibis)).map(|e| ibis_json(&e)).collect::<Vec<_>>(),
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
        "ibis_files": p.ibis_summary(),
        "notes": "Lanes follow nets through series resistors (≤ 100 Ω), AC-coupling capacitors (≥ 1 nF) and small inductors; DNP parts are skipped. kind: ic-ic (eye checked), ic-connector (board share only, no eye), multi-drop/open (not checked). Direction comes from IBIS model types where both ends have models, else it is a guess (termination next to the receiver). IBIS ends (footprint fields Sim.Device=IBIS, Sim.Library, Sim.Name, Sim.Ibis.Model) are EXPERIMENTAL and not working yet (linearised, typical corner): treat results with IBIS ends as unvalidated. Parasitics of passives are estimates from data/components.json."
    })
}

/// Channel check of all device-to-device lanes (plus nets with a target that are
/// not on any lane), judged against the interface profiles.
pub fn channel_check(p: &Project, profiles: &Profiles, assign: &HashMap<String, String>, only: Option<&[String]>, rp: &RfParams) -> Value {
    let assign = &p.assign(assign);
    let field = HashMap::new();
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
            let row = topology::check_lane(&p.board, &p.an, &field, l, p.target(tnet), rp, prof.rate_gbps, (Some(prof.tx_model()), Some(prof.rx), Some(prof.tx.rise_ps * 1e-12)), l.rx_end == 0, &cd, &p.ibis);
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
            if let Some(e) = &row.ends {
                j["end_models"] = json!(e);
            }
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
        rows.push((v.status, json!({"net": st.name, "kind": "net (no devices found)", "class": class, "profile": prof.id, "rate_gbps": prof.rate_gbps, "status": v.status, "criteria": v.criteria,
            "length_mm": row.length_mm, "vias": row.vias, "metrics": m})));
    }
    rows.sort_by(|a, b| a.0.cmp(&b.0));
    let count = |s: Status| rows.iter().filter(|r| r.0 == s).count();
    json!({
        "summary": {"green": count(Status::Green), "yellow": count(Status::Yellow), "red": count(Status::Red)},
        "config": p.config_summary(),
        "ibis_files": p.ibis_summary(),
        "rows": rows.into_iter().map(|r| r.1).collect::<Vec<_>>(),
        "models": format!("{MODELS} Lanes run from device pin to device pin through series parts (estimated parasitics). Eye only from IC to IC: PRBS7, linear transmitter/receiver from IBIS where the devices have models (EXPERIMENTAL, not working yet; see end_models), otherwise from the profile (a termination next to the receiver acts as its termination), without equalisation, jitter and noise. Lanes ending at a connector: board share only (loss, return loss, skew).")
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
        "config": p.config_summary(),
        "overview": overview(p),
        "interfaces": interfaces(p, profiles, &HashMap::new()),
        "channels": channel_check(p, profiles, &HashMap::new(), None, &rp),
        "power": power(p, None, None).unwrap_or_else(|e| json!({"error": e})),
    })
}

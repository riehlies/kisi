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
    /// the assembly variant in use (`crate::variant`)
    pub variant: crate::variant::VariantState,
    /// Touchstone files of the configuration's connector ends, by the path written in it
    pub touchstones: HashMap<String, Result<crate::rf::Touchstone, String>>,
}

/// Parse the Touchstone files the configuration's connector ends name (from `files`).
pub fn touchstones_of(cfg: Option<&crate::config::BoardConfig>, files: &crate::variant::Files) -> HashMap<String, Result<crate::rf::Touchstone, String>> {
    let mut out = HashMap::new();
    for e in cfg.map(|c| c.connector_ends.values().collect::<Vec<_>>()).unwrap_or_default() {
        let Some(tr) = &e.end.touchstone else { continue };
        if tr.file.is_empty() || out.contains_key(&tr.file) {
            continue;
        }
        let name = tr.file.rsplit(['/', '\\']).next().unwrap_or(&tr.file).to_string();
        let r = match files.get(&tr.file) {
            Some(Ok(text)) => crate::rf::parse_touchstone(text, &name),
            Some(Err(e)) => Err(e.clone()),
            None => Err(format!("{} not loaded", tr.file)),
        };
        out.insert(tr.file.clone(), r);
    }
    out
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
    /// assembly variant chosen on the command line (wins over the configuration's)
    pub variant: Option<String>,
    /// files the configuration refers to (BOMs of variants), by the path written in it
    pub files: crate::variant::Files,
}

/// Read the files a configuration refers to (BOMs of its variants, Touchstone files of its
/// connector ends; relative to the configuration file).
pub fn config_files(config_path: &Path, text: &str) -> crate::variant::Files {
    let mut files = crate::variant::Files::new();
    let Ok(c) = crate::config::parse(text) else { return files };
    let dir = config_path.parent().unwrap_or(Path::new("."));
    for v in c.variants.values() {
        if let Some(b) = &v.bom {
            let p = dir.join(b);
            files.insert(b.clone(), std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display())));
        }
    }
    // Touchstone files of the connector ends (channel behind a connector)
    for e in c.connector_ends.values() {
        if let Some(tr) = e.end.touchstone.as_ref().filter(|t| !t.file.is_empty()) {
            let p = dir.join(&tr.file);
            files.insert(tr.file.clone(), std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display())));
        }
    }
    files
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
        Self::load_with(pcb, pro, stackup, cp.is_file().then_some(cp.as_path()), None)
    }

    /// Like `load`, with an explicit configuration file (None: no configuration) and an
    /// assembly variant chosen on the command line.
    pub fn load_with(pcb: &Path, pro: Option<&Path>, stackup: Option<&StackupOverride>, config: Option<&Path>, variant: Option<&str>) -> Result<Project, String> {
        let text = std::fs::read_to_string(pcb).map_err(|e| format!("{}: {e}", pcb.display()))?;
        let pro_path = pro.map(Path::to_path_buf).unwrap_or_else(|| pcb.with_extension("kicad_pro"));
        let pro_text = std::fs::read_to_string(&pro_path).ok();
        let config = match config {
            Some(cp) => Some((cp.to_path_buf(), std::fs::read_to_string(cp).map_err(|e| format!("{}: {e}", cp.display())))),
            None => None,
        };
        let files = match &config {
            Some((cp, Ok(t))) => config_files(cp, t),
            _ => Default::default(),
        };
        Self::from_texts(Inputs { pcb_path: pcb.to_path_buf(), pcb: text, pro: pro_text, config, stackup: stackup.cloned(), field: HashMap::new(), variant: variant.map(str::to_string), files })
    }

    /// Build a project from file contents (no file system access: also used in WASM).
    pub fn from_texts(inp: Inputs) -> Result<Project, String> {
        let mut board = crate::load_pcb(&inp.pcb).map_err(|e| e.to_string())?;
        if let Some(p) = &inp.pro {
            if let Err(e) = crate::apply_project(&mut board, p) {
                board.warnings.push(format!("project file: {e}"));
            }
        }
        // configuration: checked against the board as read, then laid over it – after the
        // assembly variant (the command line's choice, else the configuration's), so that its
        // fields win over the board's
        let mut cfg = None;
        let mut cfg_check = None;
        if let Some((_, text)) = &inp.config {
            match text.clone().and_then(|t| crate::config::parse(&t)) {
                Ok(c) => {
                    let an0 = crate::extract::skeleton(&board, &Params::default());
                    let classes: Vec<String> = an0.classes.iter().map(|c| c.name.clone()).collect();
                    let bn = inp.pcb_path.file_name().map(|f| f.to_string_lossy().to_string());
                    cfg_check = Some(crate::config::check(&c, &board, &classes, &Profiles::builtin(), bn.as_deref(), &inp.files));
                    cfg = Some(c);
                }
                Err(e) => cfg_check = Some(crate::config::CheckResult { ok: false, errors: vec![e], ..Default::default() }),
            }
        }
        let variant = crate::variant::apply(&mut board, cfg.as_ref(), inp.variant.as_deref().map(|v| (v, "command line")), &inp.files);
        let applied = cfg.as_ref().map(|c| c.apply(&mut board)).unwrap_or_default();
        if let Some(ov) = &inp.stackup {
            let su = board.stackup().with_overrides(ov);
            board.stackup = Some(su);
        }
        let params = Params { step: 0.5, coplanar_factor: 3.0, solder_mask: true, targets: applied.targets.clone(), field: inp.field, ..Default::default() };
        let an = analyze(&board, &params);
        let name = inp.pcb_path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        let touchstones = touchstones_of(cfg.as_ref(), &inp.files);
        Ok(Project { pcb_path: inp.pcb_path, name, board, an, params, config_path: inp.config.map(|c| c.0), config: cfg, config_check: cfg_check, config_profiles: applied.profiles, variant, touchstones })
    }

    /// Commands that compute lanes call this first: an error with the list of variants when
    /// the board has variants (or forks with all alternatives fitted) and none is chosen.
    pub fn require_variant(&self) -> Result<(), String> {
        match &self.variant.message {
            Some(m) if self.variant.needs_choice => Err(m.clone()),
            _ => Ok(()),
        }
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
        // length whose formula impedance is a rough estimate (over a cut-out between trace and reference)
        let rough: f64 = p
            .an
            .pieces
            .chunks_exact(4)
            .filter(|pc| p.an.cross_sections[pc[3] as usize].flags & crate::extract::F_FORMULA_ROUGH != 0)
            .filter_map(|pc| p.board.tracks.get(pc[0] as usize).map(|t| (t, pc)))
            .filter(|(t, _)| nets.iter().any(|n| n.net == t.net))
            .map(|(t, pc)| crate::geom::len(crate::geom::sub(t.b, t.a)) * (pc[2] - pc[1]))
            .sum::<f64>()
            + 0.0; // no -0.0 for an empty sum
        let mut worst: Vec<_> = nets.iter().filter(|n| n.z_mean.is_some()).collect();
        worst.sort_by(|a, b| {
            let da = (a.z_mean.unwrap() - t.z).abs();
            let db = (b.z_mean.unwrap() - t.z).abs();
            db.partial_cmp(&da).unwrap()
        });
        classes.push(json!({
            "class": c.name, "target_ohm": t.z, "diff": t.diff, "target_source": c.target_source, "nets": nets.len(),
            "length_mm": len, "within_5pct": if len > 0.0 { ok / len } else { 0.0 },
            "rough_formula_mm": rough,
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
    net_touchstone2(p, net, rp, &ch)
}

/// Single-ended 4-port of a differential pair as Touchstone (.s4p) text.
pub fn net_touchstone4(p: &Project, net_name: &str, rp: &RfParams) -> Result<String, String> {
    let net = p.net_id(net_name).ok_or_else(|| format!("net '{net_name}' not found"))?;
    let m = rf::mixed_mode(&p.board, &p.an, net, &p.params.field, p.target(net), rp, false, lane_hint(p, net))?;
    Ok(rf::touchstone4(&m))
}

/// Touchstone (.s2p) of a net: the channel, for a differential pair Sdd of its 4-port model.
fn net_touchstone2(p: &Project, net: u32, rp: &RfParams, ch: &rf::Channel) -> Result<String, String> {
    if ch.diff {
        if let Ok(m) = rf::mixed_mode(&p.board, &p.an, net, &p.params.field, p.target(net), rp, false, lane_hint(p, net)) {
            return Ok(rf::touchstone_dd(&m));
        }
    }
    Ok(rf::touchstone(ch))
}

/// Summary of a pair's 4-port model (mixed-mode S-parameters, modal TDR, skew).
pub fn mixed_json(m: &rf::MixedMode, rise: f64) -> Value {
    let at = |v: &[f64], g: f64| -> Option<f64> { m.freq.iter().position(|&f| f >= g * 1e9 - 1.0).map(|k| v[k]) };
    let marks = |v: &[f64]| [1.0, 2.5, 5.0, 8.0, 12.5, 16.0, 25.0].iter().filter_map(|&g| at(v, g).map(|d| json!({"ghz": g, "db": d}))).collect::<Vec<_>>();
    let inside = |z: &[f64], x: &[f64]| -> (f64, f64) {
        z.iter().zip(x).filter(|(_, x)| **x <= m.tdr_end_mm).fold((f64::INFINITY, f64::NEG_INFINITY), |a, (z, _)| (a.0.min(*z), a.1.max(*z)))
    };
    let (dmin, dmax) = inside(&m.tdr_zdiff, &m.tdr_x_diff_mm);
    let (cmin, cmax) = inside(&m.tdr_zcomm, &m.tdr_x_comm_mm);
    json!({
        "p": m.p_net, "n": m.n_net, "z_ref_diff": 2.0 * m.z_ref, "z_ref_common": m.z_ref / 2.0,
        "length_mm": {"p": m.length_mm.0, "n": m.length_mm.1}, "delay_ps": {"p": m.delay_ps.0, "n": m.delay_ps.1},
        "skew_ps": m.skew_ps, "skew_mm": m.skew_mm, "coupled_mm": m.coupled_mm, "coupled_share": m.coupled_share,
        "where_p_and_n_differ": m.imbalance.iter().map(|x| json!({"from_mm": x.s_from_mm, "to_mm": x.s_to_mm, "p_minus_n_mm": x.dl_mm, "p_minus_n_ps": x.dt_ps, "vias_differ": x.vias_differ, "at": x.at})).collect::<Vec<_>>(),
        "delay_ps_diff": m.delay_diff_ps, "delay_ps_common": m.delay_comm_ps,
        "sdd21_db": marks(&m.sdd21_db), "scd21_db": marks(&m.scd21_db),
        "scd21_worst": {"db": m.scd21_worst.0, "ghz": m.scd21_worst.1 / 1e9},
        "return_loss_diff_worst_db": -m.sdd11_db.iter().cloned().fold(f64::NEG_INFINITY, f64::max),
        "return_loss_common_worst_db": -m.scc11_db.iter().cloned().fold(f64::NEG_INFINITY, f64::max),
        "tdr": {"rise_ps": rise * 1e12, "zdiff_min": dmin, "zdiff_max": dmax, "zcommon_min": cmin, "zcommon_max": cmax},
        "source": m.source, "warnings": m.warnings
    })
}

/// Start hint of a net's path: the transmitter side of the lane the net belongs to.
fn lane_hint(p: &Project, net: u32) -> Option<crate::geom::P> {
    for b in p.buses() {
        for l in &b.lanes {
            let rev = l.rx_end == 0;
            for h in l.hops.iter().chain(&l.partner_hops) {
                if let topology::Hop::Net { net: n, from, to, .. } = h {
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
    // formula results over a cut-out between the trace and its reference: rough estimates
    let rough = |x: Option<u32>| x.is_some_and(|x| p.an.cross_sections[x as usize].flags & crate::extract::F_FORMULA_ROUGH != 0);
    let rough_mm: f64 = ch.segments.iter().filter(|s| s.kind == "line" && rough(s.xs)).map(|s| s.len_mm).sum();
    let mut warnings = ch.warnings.clone();
    if rough_mm > 0.0 {
        warnings.push(format!("{rough_mm:.2} mm of the path lie over a cut-out in a layer between the trace and its reference (a launch or pad over a plane opening): the formula impedance there is a rough estimate, it ignores the cut-out's edges and can read 25 % high – the field solver (kisi.mjs) models them"));
    }
    let mixed = if ch.diff {
        match rf::mixed_mode(&p.board, &p.an, net, field, p.target(net), rp, false, lane_hint(p, net)) {
            Ok(m) => mixed_json(&m, rp.rise),
            Err(e) => json!({"error": e}),
        }
    } else {
        Value::Null
    };
    Ok(json!({
        "net": ch.name, "class": st.map(|s| s.class.clone()), "target": st.and_then(|s| s.target).map(|t| json!({"ohm": t.z, "diff": t.diff})),
        "differential": ch.diff, "length_mm": ch.length_mm, "delay_ps": ch.delay_ps, "vias": ch.vias, "path": path_json(&ch),
        "impedance": st.map(|s| json!({"mean": s.z_mean, "min": s.z_min, "max": s.z_max, "within_5pct": s.within5})),
        "segments": ch.segments.iter().filter(|s| s.kind == "line").map(|s| {
            let mut v = json!({"layer": su.copper[s.layer as usize].name, "len_mm": s.len_mm, "z_ohm_per_line": s.z});
            if rough(s.xs) {
                v["estimate"] = json!("rough: formula over a cut-out between trace and reference");
            }
            v
        }).collect::<Vec<_>>(),
        // lumped parts in the channel (shunt parts of the net at their pads; estimated parasitics)
        "parts": ch.segments.iter().filter(|s| matches!(s.kind, "shunt" | "series")).map(|s| json!({"part": s.part, "kind": s.kind, "at_mm": s.s_mm, "parasitics": "estimate"})).collect::<Vec<_>>(),
        "insertion_loss_db": ch.il_marks.iter().map(|(f, d)| json!({"ghz": f, "db": d})).collect::<Vec<_>>(),
        "return_loss_worst_db": -ch.s11_db.iter().cloned().fold(f64::NEG_INFINITY, f64::max),
        "return_loss_worst_db_b": -ch.s22_db.iter().cloned().fold(f64::NEG_INFINITY, f64::max),
        "tdr": {"rise_ps": rp.rise * 1e12, "z_min": tdr_in.iter().cloned().fold(f64::INFINITY, f64::min) * k, "z_max": tdr_in.iter().cloned().fold(f64::NEG_INFINITY, f64::max) * k, "mode": if ch.diff { "differential" } else { "single-ended" }},
        "via_details": ch.via_info.iter().map(|v| json!({"from": su.copper[v.layer_in as usize].name, "to": su.copper[v.layer_out as usize].name, "drill_mm": v.drill_mm, "antipad_mm": v.antipad_mm, "z_ohm": v.z_via, "stub_mm": v.stub_mm, "stub_resonance_ghz": v.stub_f_ghz, "return_path": v.return_kind, "model": v.model})).collect::<Vec<_>>(),
        "crosstalk": {"next_total": xt.next_total, "fext_total": xt.fext_total, "rise_ps": xt.rise_ps, "differential": xt.differential,
            "next_total_common": if xt.differential { json!(xt.next_total_c) } else { Value::Null },
            "fext_total_common": if xt.differential { json!(xt.fext_total_c) } else { Value::Null },
            "aggressors": xt.aggressors.iter().take(5).map(|a| {
                let mut v = json!({"net": a.name, "pair": a.pair, "parallel_mm": a.coupled_mm, "gap_min_mm": a.gap_min_mm, "next": a.next, "fext": a.fext, "source": a.source});
                if xt.differential {
                    v["next_common"] = json!(a.next_c);
                    v["fext_common"] = json!(a.fext_c);
                }
                v
            }).collect::<Vec<_>>(),
            "broadside": xt.broadside.iter().map(|b| json!({"net": b.name, "layer": su.copper[b.layer as usize].name, "overlap_mm": b.overlap_mm, "dz_mm": b.dz_mm, "lateral_offset_mm": b.offset_mm, "computed": false})).collect::<Vec<_>>(),
            "note": "fractions of the aggressor swing; source 'formula' is an estimate. Differential victims: differential noise, common-mode noise in *_common. Broadside overlaps (tracks on the adjacent layer) are listed but not computed yet."},
        "mixed_mode": mixed,
        "warnings": warnings, "field_solver": p.field_summary(), "models": p.models()
    }))
}

/// Victims of the coupling scan: the named nets, else all nets with a target.
pub fn xt_nets(p: &Project, names: Option<&[String]>) -> Result<Vec<u32>, String> {
    match names {
        Some(ns) => ns.iter().map(|n| p.net_id(n).ok_or_else(|| format!("net '{n}' not found"))).collect(),
        None => Ok(p.an.nets.iter().filter(|s| s.target.is_some()).map(|s| s.net).collect()),
    }
}

/// Coupling scan (stage 1 of the viewer's step 4; with field-solver results the critical
/// couplings are refined): one row per victim, a pair once.
pub fn crosstalk_scan(p: &Project, names: Option<&[String]>, rp: &RfParams) -> Result<Value, String> {
    let nets = xt_nets(p, names)?;
    let sc = rf::crosstalk_scan(&p.board, &p.an, &nets, &p.params.field, rp, rf::XT_CRITICAL);
    let rows: Vec<Value> = sc
        .rows
        .iter()
        .map(|r| {
            json!({"net": r.name, "partner": r.partner.map(|n| p.board.net_name(n).to_string()), "differential": r.differential,
                "next": r.next, "fext": r.fext,
                "next_common": if r.differential { json!(r.next_c) } else { Value::Null },
                "fext_common": if r.differential { json!(r.fext_c) } else { Value::Null },
                "worst_aggressor": r.worst, "worst_aggressor_pair": r.worst_pair, "source": r.source,
                "unsolved_cross_sections": r.open, "broadside_overlaps": r.broadside, "critical": r.critical})
        })
        .collect();
    Ok(json!({
        "victims": rows.len(), "critical": sc.rows.iter().filter(|r| r.critical).count(), "threshold": sc.threshold,
        "rise_ps": rp.rise * 1e12, "unsolved_critical_cross_sections": sc.critical_geoms.len(),
        "rows": rows,
        "note": "NEXT (backward, seen at the near end) and FEXT (forward, at the far end) as fractions of the aggressor swing, all aggressors summed; critical from 1 % (kisi guide value). Source 'formula' is an estimate; kisi.mjs solves the critical coupling cross-sections with the field solver. Broadside overlaps (tracks on the adjacent layer) are counted but not computed yet. Details per net: `kisi net`.",
        "field_solver": p.field_summary()
    }))
}

fn pin(e: &topology::EndPin) -> String {
    format!("{}.{}", e.refdes, e.pin)
}

/// The net a lane's target, class and profile come from: the one it is named after (its own
/// path), else its first net with a target.
fn lane_net(p: &Project, l: &topology::Lane) -> u32 {
    if p.target(l.name_net).is_some() {
        return l.name_net;
    }
    l.nets.iter().copied().find(|&n| p.target(n).is_some()).unwrap_or(l.nets[0])
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
        // the other paths of a fork: open branches, not part of the lane's own path
        "branches": l.branches.iter().enumerate().map(|(i, b)| json!({
            "to": l.branch_ends.get(i),
            "through": b.iter().find_map(|h| match h { topology::Hop::Series { part } => Some(format!("{} {}", part.refdes, part.value)), _ => None }),
            "nets": b.iter().filter_map(|h| match h { topology::Hop::Net { net, .. } if !l.nets.contains(net) => Some(p.board.net_name(*net)), _ => None }).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
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
            let tnet = lane_net(p, l0);
            let pid = assign.get(&b.class).cloned().unwrap_or_else(|| profiles.auto(p.board.net_name(tnet), &b.class, l0.diff).to_string());
            let mut v = json!({"bus": b.name, "kind": b.kind, "class": b.class, "profile": pid, "devices": b.devices, "values": b.values,
                "lanes": b.lanes.iter().map(|l| lane_json(p, l)).collect::<Vec<_>>()});
            // manufacturer part numbers where the Value field is not one (a name, a description)
            let mpns: Vec<Option<&str>> = b.devices.iter().map(|d| p.board.footprints.iter().find(|f| &f.reference == d).and_then(|f| f.mpn_if_not_value())).collect();
            if mpns.iter().any(|m| m.is_some()) {
                v["mpns"] = json!(mpns);
            }
            v
        })
        .collect();
    json!({
        "buses": list,
        "variant": p.variant.summary(),
        "config": p.config_summary(),
        "notes": "Lanes follow nets through series resistors (≤ 100 Ω), AC-coupling capacitors (≥ 1 nF) and small inductors; parts not fitted (DNP in KiCad or not in the chosen assembly variant) connect nothing, their pads stay as stubs. kind: ic-ic (eye checked), connector-connector (board share only, no eye), ic-connector (board share: loss, return loss, skew judged; the eye with the connector end model – connector_ends.<connector> in kisi.json, default the profile's receiver or transmitter; information unless a channel behind the connector is given), multi-drop/open (not checked). Direction is a guess (termination next to the receiver), parasitics of passives are estimates from data/components.json. mpns (when present): the devices' manufacturer part numbers where the Value field does not show one."
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
            if !topology::checked_kind(&l.kind) {
                continue;
            }
            if !wanted(&mut l.nets.iter().map(|&n| p.board.net_name(n))) {
                continue;
            }
            let tnet = lane_net(p, l);
            let class = p.class_name(tnet);
            let pid = assign.get(&class).cloned().unwrap_or_else(|| profiles.auto(p.board.net_name(tnet), &class, l.diff).to_string());
            let Some(prof) = profiles.get(&pid) else { continue };
            let lim = profiles.limits(prof);
            // a lane ending at a connector: the end model there (kisi.json connector_ends)
            let cend = l.ends.iter().find(|e| e.role == "connector").and_then(|e| p.config.as_ref()?.connector_ends.get(&e.refdes)).map(|e| &e.end);
            let cts = cend.and_then(|c| c.touchstone.as_ref()).and_then(|tr| p.touchstones.get(&tr.file)?.as_ref().ok());
            let row = topology::check_lane(&p.board, &p.an, field, l, p.target(tnet), rp, prof.rate_gbps, (Some(prof.tx_model()), Some(prof.rx), Some(prof.tx.rise_ps * 1e-12)), l.rx_end == 0, cend, cts, &cd);
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
                // the eye of a lane ending at a connector covers the board share only: shown, not judged
                eye_h: (row.eye && !row.eye_board_share && row.error.is_none()).then_some(row.eye_height),
                eye_w_ui: (row.eye && !row.eye_board_share && row.error.is_none()).then_some(row.eye_width_ui),
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
            if row.eye_board_share && row.eye && row.error.is_none() {
                j["eye_board_share"] = json!({"height": row.eye_height, "width_ui": row.eye_width_ui, "connector_end": cend.map(|e| e.model.as_str()).filter(|m| !m.is_empty()).unwrap_or("profile"),
                    "note": "eye of the board share with the connector end model (termination or driver at the connector); information, not judged"});
            }
            j["stubs"] = json!(row.stubs);
            j["metrics"] = json!(m);
            rows.push((v.status, j));
        }
    }
    // nets with a target that are on no lane (no devices found at their ends); nets that only
    // connect parts not fitted have no function in this assembly and are left out
    let unused: std::collections::HashSet<u32> = p.variant.unused_nets.iter().copied().collect();
    let mut skipped_unused = 0usize;
    for st in &p.an.nets {
        let Some(t) = st.target else { continue };
        if on_lane.contains(&st.net) || !wanted(&mut std::iter::once(st.name.as_str())) {
            continue;
        }
        if unused.contains(&st.net) {
            skipped_unused += 1;
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
        "variant": p.variant.summary(),
        "config": p.config_summary(),
        "not_checked_unused": (skipped_unused > 0).then(|| format!("{skipped_unused} net(s) with a target only connect parts not fitted in this assembly (variant or DNP): not checked")),
        "rows": rows.into_iter().map(|r| r.1).collect::<Vec<_>>(),
        "field_solver": p.field_summary(),
        "models": format!("{} Lanes run from device pin to device pin through series parts (estimated parasitics); side stubs of the lane nets are open stubs (unused pads, parts not fitted with their pad capacitance – estimates –, the other path of a fork). Eye: PRBS7, linear transmitter/receiver from the profile (a termination next to the receiver acts as its termination), without equalisation, jitter and noise. Shunt parts on the lane's nets (capacitors to ground, …) are in the channel at their pads. Lanes from connector to connector: board share only (loss, return loss, skew, TDR), no eye. Lanes ending at a connector: board share – loss, return loss and skew judged; the eye with the connector end model (kisi.json connector_ends.<connector>, default the profile's receiver or transmitter at the connector) – information (eye_board_share), or judged when a channel behind the connector (rest_loss_db) is given.", p.models())
    })
}

/// The DC part of the power analysis, shared by `power`, `thermal` and `report`: the tree (PI_REG
/// fields or a JSON tree), the rail setups and the DC solution of every rail and the ground return.
pub struct PowerRun {
    pub source: &'static str,
    pub tree: Option<crate::tree::TreeResult>,
    pub balance: Option<Value>,
    pub setups: Vec<crate::pi::RailSetup>,
    /// solved rails, then the ground return, with the setup each came from
    pub solved: Vec<(crate::pi::RailSetup, crate::pi::PiResult)>,
    /// JSON row per rail (also the ones that failed)
    pub rails: Vec<Value>,
    pub warnings: Vec<String>,
    pub params: crate::pi::PiParams,
}

pub fn power_run(p: &Project, tree: Option<crate::tree::TreeSpec>) -> Result<PowerRun, String> {
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
            let mut bal = json!({"input_w": t.p_input, "loads_w": t.p_loads, "losses_w": t.p_losses,
                "elements": t.elements.iter().map(|e| json!({"id": e.id, "kind": e.kind, "part": e.part, "i_in": e.i_in, "i_out": e.i_out, "loss_w": e.p_loss, "overload": e.overload,
                    "efficiency": e.eff, "efficiency_estimate": e.eff_estimate})).collect::<Vec<_>>()});
            let est: Vec<&str> = t.elements.iter().filter(|e| e.eff_estimate).map(|e| e.id.as_str()).collect();
            if !est.is_empty() {
                bal["note"] = json!(format!(
                    "efficiency_estimate: {} without an efficiency in PI_REG – kisi's default {:.0} % is used (estimate); give the datasheet value at the expected load (e.g. 'buck 90%').",
                    est.join(", "),
                    crate::tree::DEFAULT_EFF * 100.0
                ));
            }
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
                solved.push((s.clone(), r));
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
                    solved.push((s.clone(), r));
                }
                Err(e) => rails.push(json!({"rail": s.net, "ground_return": true, "error": e})),
            }
        }
    }
    Ok(PowerRun { source, tree: tree_res, balance, setups, solved, rails, warnings, params })
}

/// Power: tree balance (PI_REG fields or a JSON tree), DC drop of every rail,
/// optional PDN impedance of one rail.
pub fn power(p: &Project, tree: Option<crate::tree::TreeSpec>, pdn_rail: Option<&str>) -> Result<Value, String> {
    power_from(p, &power_run(p, tree)?, pdn_rail)
}

pub fn power_from(p: &Project, run: &PowerRun, pdn_rail: Option<&str>) -> Result<Value, String> {
    let (setups, tree_res) = (&run.setups, &run.tree);
    let refs: Vec<&crate::pi::PiResult> = run.solved.iter().map(|x| &x.1).collect();
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
            let per_ic = match crate::pdn_ic::pdn_ic(&p.board, &setup, setups, &crate::pdn::PdnParams::default()) {
                Ok(ic) => pdn_ic_json(&ic),
                Err(e) => json!({"error": e}),
            };
            Some(json!({"rail": r.net, "target_ohm": r.target_ohm, "above_target_hz": r.above_target, "c_total_f": r.c_total, "c_plane_f": r.c_plane,
                "cavity_first_resonance_hz": r.cavity_hz, "z_ohm": {"1k": at(1e3), "100k": at(1e5), "1M": at(1e6), "10M": at(1e7), "100M": at(1e8)},
                "caps": r.caps.len(), "warnings": r.warnings, "per_ic": per_ic}))
        }
        None => None,
    };
    Ok(json!({"tree_source": run.source, "balance": run.balance, "rails": run.rails, "losses": losses, "flow": flow, "pdn": pdn, "warnings": run.warnings}))
}

/// Temperature estimate: the power analysis, the board's temperature from its losses
/// (`thermal::solve`) and the IR drop of every rail at that temperature.
pub fn thermal(p: &Project, tree: Option<crate::tree::TreeSpec>, prm: &crate::thermal::ThermalParams) -> Result<Value, String> {
    thermal_from(p, &power_run(p, tree)?, prm)
}

pub fn thermal_from(p: &Project, run: &PowerRun, prm: &crate::thermal::ThermalParams) -> Result<Value, String> {
    let refs: Vec<&crate::pi::PiResult> = run.solved.iter().map(|x| &x.1).collect();
    let ls = crate::losses::loss_summary(&p.board, run.tree.as_ref(), &refs);
    let mut th = crate::thermal::solve(&p.board, &refs, &ls.components, prm)?;
    th.rails = run.solved.iter().map(|(s, r)| crate::thermal::rail_at_temperature(&p.board, s, r, &th, &run.params)).collect();
    let mut warnings = run.warnings.clone();
    warnings.extend(th.warnings.iter().cloned());
    Ok(thermal_json(p, &th, prm, run.source, warnings))
}

/// The JSON of `kisi thermal` (temperatures in °C rounded to 0.01, powers to 0.1 mW).
pub fn thermal_json(p: &Project, th: &crate::thermal::ThermalResult, prm: &crate::thermal::ThermalParams, tree_source: &str, warnings: Vec<String>) -> Value {
    let su = p.board.stackup();
    let lname = |l: u16| su.copper.get(l as usize).map(|c| c.name.clone()).unwrap_or_default();
    let c2 = |x: f64| (x * 100.0).round() / 100.0 + 0.0;
    let w4 = |x: f64| (x * 1e4).round() / 1e4 + 0.0;
    let xy = |p: Option<crate::geom::P>| p.map(|p| json!([c2(p[0]), c2(p[1])]));
    let opt = |x: Option<f64>| x.map(c2);
    let co = &th.cooling;
    let parts: Vec<Value> = th
        .parts
        .iter()
        .take(30)
        .map(|x| {
            let mut v = json!({
                "ref": x.refdes, "part": x.part, "heat_w": w4(x.w), "heat_source": x.w_source, "side": x.side,
                "t_board_c": c2(x.t_board_c), "tj_c": c2(x.tj_c), "t_case_top_c": opt(x.t_case_c), "case_model": x.case_model, "tj_max_c": x.tj_max_c, "tj_max_source": x.tj_max_source, "margin_k": c2(x.margin_k), "status": x.status,
                "model": x.model, "theta_jb": opt(x.theta_jb), "theta_jc": opt(x.theta_jc), "psi_jb": x.psi_jb, "theta_ja": x.theta_ja,
                "to_board_share": x.to_board.map(|s| (s * 1000.0).round() / 1000.0), "package_family": x.family, "data_source": x.data_source, "estimate": x.estimate,
            });
            v["under_part"] = json!(x.vias.as_ref().map(|v| json!({"vias": v.count, "thermal_pad": v.thermal_pad, "into_board_w": w4(v.to_board_w), "through_vias_w": w4(v.via_w), "through_laminate_w": w4(v.laminate_w), "sideways_in_pad_layer_w": w4(v.lateral_w),
                "dt_pad_layer_to_next_k": c2(v.dt_next_k), "dt_pad_layer_to_other_side_k": c2(v.dt_through_k), "hint": v.hint})));
            v
        })
        .collect();
    let vias = json!({
        "total": th.vias_total,
        "most_heat": th.vias.iter().take(20).map(|v| json!({"at": xy(Some(v.at)), "from": lname(v.from), "to": lname(v.to), "drill_mm": v.drill, "net": v.net, "plated_hole": v.hole,
            "heat_w": (v.q_w * 1e6).round() / 1e6, "dt_k": c2(v.dt_k), "t_c": c2(v.t_c)})).collect::<Vec<_>>(),
        "note": "heat_w: the largest heat flow through one layer step of the barrel (positive downwards), dt_k: temperature difference between its end layers; open barrels with 25 µm plating (estimate)",
    });
    let hottest = th.parts.iter().max_by(|a, b| a.tj_c.total_cmp(&b.tj_c));
    json!({
        "estimate": "Temperatures are estimates: steady state, board in air at the ambient temperature, heat only through convection and radiation; package data from PI_THERMAL or estimated package defaults (see parts[].data_source).",
        "summary": {
            "t_board_max_c": c2(th.t_max_c), "at": xy(th.t_max_at), "layer": lname(th.t_max_layer),
            "hottest_part": hottest.map(|h| json!({"ref": h.refdes, "part": h.part, "tj_c": c2(h.tj_c), "tj_max_c": h.tj_max_c, "estimate": h.estimate})),
            "parts_over_tj_max": th.parts.iter().filter(|x| x.status == "over").map(|x| x.refdes.clone()).collect::<Vec<_>>(),
            "parts_within_10k_of_tj_max": th.parts.iter().filter(|x| x.status == "near").map(|x| x.refdes.clone()).collect::<Vec<_>>(),
            "ir_drop_increase_max_mv": th.rails.iter().filter(|r| !r.ground).filter_map(|r| Some(r.drop_mv_hot? - r.drop_mv_20c?)).fold(None, |a: Option<f64>, x| Some(a.map_or(x, |a| a.max(x)))).map(|x| (x * 1e3).round() / 1e3),
        },
        "ambient_c": th.ambient_c,
        "ambient_source": if prm.ambient_c.is_some() { "--ambient" } else { "default (assumption)" },
        "cooling": {
            "orientation": co.orientation, "airflow_m_s": co.airflow_m_s,
            "h_top_w_m2k": c2(co.h_top), "h_bottom_w_m2k": c2(co.h_bottom), "h_convection_top": c2(co.h_conv_top), "h_convection_bottom": c2(co.h_conv_bottom), "h_radiation": c2(co.h_rad),
            "mean_rise_k": c2(co.dt_mean_k),
            "source": if co.estimated { "estimate: natural (or forced) convection from flat-plate correlations and linearised radiation (emissivity 0.9) at the board's mean temperature rise, data/thermal.json" } else { "--h (convection and radiation per side)" },
        },
        "heat_w": {"total": w4(th.parts_w + th.copper_w), "parts": w4(th.parts_w), "copper": w4(th.copper_w)},
        "energy_balance": {"in_w": w4(th.balance.in_w), "convection_w": w4(th.balance.convection_w), "radiation_w": w4(th.balance.radiation_w), "error_rel": if th.balance.error_rel < 1e-6 { 0.0 } else { th.balance.error_rel }},
        "board": {
            "t_max_c": c2(th.t_max_c), "at": xy(th.t_max_at), "layer": lname(th.t_max_layer),
            "per_layer": th.layers.iter().map(|l| json!({"layer": l.name, "t_max_c": c2(l.t_max_c), "t_mean_c": c2(l.t_mean_c)})).collect::<Vec<_>>(),
        },
        "hot_spots": th.hot_spots.iter().map(|h| json!({"t_c": c2(h.t_c), "layer": lname(h.layer), "at": xy(Some(h.at)), "near": h.near})).collect::<Vec<_>>(),
        "parts": parts,
        "parts_omitted": th.parts.len().saturating_sub(30),
        "rails": th.rails.iter().map(|r| json!({
            "rail": r.net, "ground_return": r.ground, "t_copper_max_c": opt(r.t_max_c), "copper_max_at": xy(r.max_at), "copper_max_layer": r.max_layer.map(lname), "t_copper_mean_c": opt(r.t_mean_c),
            "max_drop_mv_20c": r.drop_mv_20c.map(|x| (x * 1e3).round() / 1e3), "max_drop_mv_at_temperature": r.drop_mv_hot.map(|x| (x * 1e3).round() / 1e3),
            "copper_loss_mw_20c": (r.loss_w_20c * 1e4).round() / 10.0, "copper_loss_mw_at_temperature": r.loss_w_hot.map(|x| (x * 1e4).round() / 10.0), "error": r.error,
        })).collect::<Vec<_>>(),
        "vias": vias,
        "parts_hints": th.parts.iter().filter_map(|x| x.vias.as_ref().and_then(|v| v.hint.as_ref()).map(|h| json!({"ref": x.refdes, "hint": h, "estimate": true}))).collect::<Vec<_>>(),
        "grid": {"cell_mm": th.cell, "nodes": th.nodes, "layers": th.layers.len(), "board_area_mm2": th.area_mm2.round()},
        "tree_source": tree_source,
        "config": p.config_summary(),
        "variant": p.variant.summary(),
        "warnings": warnings,
        "notes": th.notes,
        "models": "Board: one sheet per copper layer on a uniform grid over the outline (copper fill of every net, laminate in plane and through the thickness, open via barrels), convection and radiation from both sides. Heat: copper loss of every rail and the ground return (DC solution at 20 °C), part heat from the power tree (load power V·I, regulator and pass losses) or PI_THERMAL p. Parts: JEDEC two-resistor model (θJB, θJC) or ψJB / θJA. Rails: copper temperature from the map; IR drop again with ρ(T) = ρ₂₀(1 + 0.00393/K·(T − 20 °C)) (one electro-thermal step). Case top: Tj − ψJT·P with ψJT from PI_THERMAL, else the case node of the two-resistor model (Tj − θJC·heat through the top). Vias: heat through each barrel from the temperatures of the layers it joins; under_part splits a part's heat into vias, laminate and sideways in the pad layer; hints (no via under a thermal pad, 5 K or more to the next layer, under 25 % through vias with over 50 % sideways) are kisi guide values, estimates.",
    })
}

/// Summary of the PDN seen from every IC (`pdn_ic`): Z at decades, target, bands above it,
/// anti-resonances, the most effective capacitors, recommendations, capacitor flags.
pub fn pdn_ic_json(r: &crate::pdn_ic::IcPdnResult) -> Value {
    let nh = |l: f64| (l * 1e12).round() / 1e3;
    let ports: Vec<Value> = r.ports.iter().map(|pt| {
        let at = |f: f64| r.freq.iter().position(|&x| x >= f * 0.999).map(|i| pt.z[i]);
        json!({
            "ic": pt.refdes, "net": pt.net, "current_a": pt.current, "pins": pt.pins,
            "target_ohm": pt.target, "target_source": pt.target_source, "target_fmax_hz": pt.f_max,
            "z_ohm": {"10k": at(1e4), "100k": at(1e5), "1M": at(1e6), "10M": at(1e7), "100M": at(1e8), "1G": at(1e9)},
            "above_target_hz": pt.above,
            "anti_resonances": pt.peaks.iter().map(|k| json!({"hz": k.f, "z_ohm": k.z, "above_target": k.above_target, "between": k.note})).collect::<Vec<_>>(),
            "capacitors_most_effective": pt.caps.iter().take(8).map(|l| json!({"ref": l.refdes, "l_total_nh": nh(l.l_total), "l_path_nh": nh(l.l_path), "srf_mhz": (l.srf_hz / 1e4).round() / 100.0, "dist_mm": (l.dist_mm * 10.0).round() / 10.0})).collect::<Vec<_>>(),
            "recommendations": pt.recommendations.iter().map(|x| json!({"hz": [x.f_lo, x.f_hi], "value": x.value, "package": x.package, "count": x.count, "within_mm": (x.radius_mm * 10.0).round() / 10.0, "note": x.note})).collect::<Vec<_>>(),
        })
    }).collect();
    let flagged: Vec<Value> = r.caps.iter().filter(|c| !c.flags.is_empty()).map(|c| json!({"ref": c.refdes, "value": c.value, "flags": c.flags})).collect();
    json!({
        "ics": ports, "capacitors": r.caps.len(), "capacitor_flags": flagged, "regulators": r.regulators,
        "domain": r.domain.iter().map(|d| json!({"net": d.net, "voltage": d.voltage, "capacitors": d.caps, "c_total_uf": (d.c_total * 1e8).round() / 100.0, "ics": d.ics, "regulators": d.regulators})).collect::<Vec<_>>(),
        "series_parts": r.series.iter().map(|x| json!({"ref": x.refdes, "value": x.value, "kind": x.kind, "from": x.from, "to": x.to, "r_ohm": x.r, "l_nh": nh(x.l), "lp_nh": x.lp.map(nh), "rp_ohm": x.rp, "z_100mhz_ohm": (x.z100 * 10.0).round() / 10.0, "source": x.source})).collect::<Vec<_>>(),
        "domain_ends": r.boundary,
        "grid": {"cell_mm": r.cell_mm, "nodes": r.nodes, "layers": r.layers.len()},
        "note": "AC grid of the rail domain (the rail and the nets behind series resistors, inductors and ferrite beads, up to the regulators) over all layers against ground (ideal reference); series parts with R, L and the bead's lossy part (estimates unless PI_SERIES or the value gives them); capacitors with ESR, ESL, mounting and DC-bias-derated capacitance (estimates unless PI_CAP); targets from each IC's current step unless PI_TARGET. Design check, not a signoff PDN simulation.",
        "warnings": r.warnings
    })
}

/// Everything in one document, written as `analysis/kisi.json` next to the board
/// (layout of kicad-happy's analysis folder).

pub fn report(p: &Project, profiles: &Profiles) -> Value {
    let rp = RfParams::default();
    let run = power_run(p, None);
    json!({
        "tool": "kisi", "version": env!("CARGO_PKG_VERSION"),
        "variant": p.variant.summary(),
        "config": p.config_summary(),
        "overview": overview(p),
        "interfaces": interfaces(p, profiles, &HashMap::new()),
        "channels": channel_check(p, profiles, &HashMap::new(), None, &rp),
        "power": run.as_ref().map_err(|e| e.clone()).and_then(|r| power_from(p, r, None)).unwrap_or_else(|e| json!({"error": e})),
        "thermal": run.as_ref().map_err(|e| e.clone()).and_then(|r| thermal_from(p, r, &crate::thermal::ThermalParams::default())).unwrap_or_else(|e| json!({"error": e})),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A CAPZ coupon: a 50 Ω line from connector J1 to connector J2 (class 50Z) with a 100 nF
    /// capacitor (KiCad's 0805) to GND at 20 mm.
    const PCB: &str = r#"(kicad_pcb (version 20241229) (generator "pcbnew")
  (layers (0 "F.Cu" signal) (4 "In1.Cu" signal) (2 "B.Cu" signal))
  (setup (stackup
    (layer "F.Cu" (type "copper") (thickness 0.035))
    (layer "dielectric 1" (type "prepreg") (thickness 0.2) (material "FR4") (epsilon_r 4.3) (loss_tangent 0.02))
    (layer "In1.Cu" (type "copper") (thickness 0.035))
    (layer "dielectric 2" (type "core") (thickness 1.2) (material "FR4") (epsilon_r 4.5) (loss_tangent 0.02))
    (layer "B.Cu" (type "copper") (thickness 0.035))))
  (net 0 "") (net 1 "SE_CAPZ_0805") (net 2 "GND")
  (footprint "J" (layer "F.Cu") (at 0 0) (property "Reference" "J1") (property "Value" "SMA") (attr smd)
    (pad "1" smd rect (at 0 0) (size 0.6 0.35) (layers "F.Cu") (net 1 "SE_CAPZ_0805")))
  (footprint "J" (layer "F.Cu") (at 40 0) (property "Reference" "J2") (property "Value" "SMA") (attr smd)
    (pad "1" smd rect (at 0 0) (size 0.6 0.35) (layers "F.Cu") (net 1 "SE_CAPZ_0805")))
  (footprint "Capacitor_SMD:C_0805_2012Metric" (layer "F.Cu") (at 20 0.95) (property "Reference" "C3") (property "Value" "100nF 16V X7R") (attr smd)
    (pad "1" smd rect (at 0 -0.95) (size 1.45 1.0) (layers "F.Cu") (net 1 "SE_CAPZ_0805"))
    (pad "2" smd rect (at 0 0.95) (size 1.45 1.0) (layers "F.Cu") (net 2 "GND")))
  (segment (start 0 0) (end 40 0) (width 0.35) (layer "F.Cu") (net 1))
  (zone (net 2) (net_name "GND") (layer "In1.Cu") (filled_polygon (layer "In1.Cu") (pts (xy -5 -5) (xy 45 -5) (xy 45 8) (xy -5 8)))))
"#;
    const PRO: &str = r#"{"net_settings": {"classes": [{"name": "Default"}, {"name": "50Z"}], "netclass_assignments": {"SE_CAPZ_0805": "50Z"}}}"#;

    fn project() -> Project {
        Project::from_texts(Inputs { pcb_path: "capz.kicad_pcb".into(), pcb: PCB.into(), pro: Some(PRO.into()), config: None, stackup: None, field: HashMap::new(), variant: None, files: Default::default() }).unwrap()
    }

    /// `check --all` judges the connector–connector lane (board share: impedance, loss, return
    /// loss; no eye), and `net` models the shunt capacitor at its pad.
    #[test]
    fn coupon_lane_is_checked_and_net_has_the_capacitor() {
        let p = project();
        let o = crate::commands::parse_args(&["check".into(), "capz.kicad_pcb".into(), "--all".into()]).unwrap();
        let v = crate::commands::run(&o, Some(&p), None).unwrap();
        let rows = v["rows"].as_array().unwrap();
        assert_eq!(rows.len(), 1, "{v:#}");
        let r = &rows[0];
        assert_eq!((r["kind"].as_str(), r["lane"].as_str(), r["eye_computed"].as_bool()), (Some("connector-connector"), Some("SE_CAPZ_0805"), Some(false)));
        let keys: Vec<&str> = r["criteria"].as_array().unwrap().iter().filter_map(|c| c["key"].as_str()).collect();
        assert!(keys.contains(&"z") && keys.contains(&"il") && keys.contains(&"rl") && !keys.contains(&"eye_h"), "{keys:?}");
        assert_eq!(r["terminations"][0].as_str(), Some("C3 100nF 16V X7R to GND"));
        // the capacitor costs return loss: the row is not green
        assert_ne!(r["status"].as_str(), Some("green"), "{r:#}");
        // net: the capacitor at 20 mm, named in the warnings
        let n = net_detail(&p, "SE_CAPZ_0805", &RfParams::default()).unwrap();
        let parts = n["parts"].as_array().unwrap();
        assert!(parts.len() == 1 && parts[0]["part"].as_str().unwrap().starts_with("C3 100nF") && (parts[0]["at_mm"].as_f64().unwrap() - 20.0).abs() < 0.6, "{parts:?}");
        assert!(n["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("C3 100nF 16V X7R (0805)")));
    }
}

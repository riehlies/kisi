//! Fab stackup templates (`data/stackups.json`): the library, whether a template fits a board
//! (copper layer count, board thickness), and a template as stackup changes – for `--stackup-preset`
//! – or as the `stackup` entry of `<board>.kisi.json` with its sources (the viewer's “Use a fab
//! stackup”, `kisi stackups <board> --preset ID`).

use crate::board::{CopperOverride, DielectricOverride, Stackup, StackupOverride};
use crate::config::{CopperEntry, DielectricEntry, MaskEntry, Source, StackupEntry};
pub use crate::study::Tolerances;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

pub const STACKUPS_JSON: &str = include_str!("../../../data/stackups.json");

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DataSource {
    pub kind: String,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PresetDielectric {
    pub kind: String,
    pub material: String,
    /// mm
    pub thickness: f64,
    pub epsilon_r: f64,
    pub loss_tangent: f64,
    /// another published εr of the same material (a second source), for comparisons
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub epsilon_r_alt: Option<f64>,
    /// where εr and tanδ come from (assumptions named)
    pub note: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PresetMask {
    pub thickness: f64,
    pub epsilon_r: f64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Preset {
    pub id: String,
    pub fab: String,
    /// the template's name in the fab's order form
    pub template: String,
    pub title: String,
    pub layers: usize,
    /// nominal board thickness (mm)
    pub nominal_mm: f64,
    /// pressed thickness of the template (mm)
    pub pressed_mm: f64,
    /// finished copper thickness per copper layer, top to bottom (mm)
    pub copper: Vec<f64>,
    pub dielectrics: Vec<PresetDielectric>,
    pub solder_mask: PresetMask,
    pub sources: Vec<DataSource>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BoardThickness {
    pub tolerance_pct: f64,
    pub tolerance_below_1mm_mm: f64,
    pub sources: Vec<DataSource>,
}

/// Default tolerances of the impedance band and the default target window (`data/stackups.json`).
#[derive(Debug, Clone, Deserialize)]
pub struct ToleranceDefaults {
    pub about: String,
    pub dielectric_thickness_pct: f64,
    pub er_pct: f64,
    pub copper_pct: f64,
    pub width_mm: f64,
    pub window_pct: f64,
    /// the values that are kisi estimates
    pub estimates: Vec<String>,
    pub sources: Vec<DataSource>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Library {
    pub about: String,
    pub board_thickness: BoardThickness,
    pub tolerances: ToleranceDefaults,
    pub presets: Vec<Preset>,
}

/// The built-in library (`data/stackups.json`).
pub fn library() -> &'static Library {
    static L: OnceLock<Library> = OnceLock::new();
    L.get_or_init(|| serde_json::from_str(STACKUPS_JSON).expect("data/stackups.json"))
}

impl Library {
    pub fn get(&self, id: &str) -> Option<&Preset> {
        self.presets.iter().find(|p| p.id.eq_ignore_ascii_case(id) || p.template.eq_ignore_ascii_case(id))
    }
    /// The preset `id` (or its template name), or an error that lists the library.
    pub fn find(&self, id: &str) -> Result<&Preset, String> {
        self.get(id).ok_or_else(|| {
            format!(
                "unknown stackup preset '{id}' (presets: {}; see `kisi stackups`)",
                self.presets.iter().map(|p| p.id.as_str()).collect::<Vec<_>>().join(", ")
            )
        })
    }
}

/// Whether a preset fits a board's stackup.
#[derive(Debug, Clone, Serialize)]
pub struct Fit {
    pub ok: bool,
    pub board_layers: usize,
    /// the board's thickness from its stackup (copper and dielectrics), when the stackup is in the file
    pub board_mm: Option<f64>,
    /// why it does not fit
    #[serde(skip_serializing_if = "Option::is_none")]
    pub problem: Option<String>,
}

impl Preset {
    /// The fab's board thickness tolerance around the template's nominal thickness (mm).
    pub fn thickness_tolerance_mm(&self) -> f64 {
        let t = &library().board_thickness;
        if self.nominal_mm >= 1.0 {
            self.nominal_mm * t.tolerance_pct / 100.0
        } else {
            t.tolerance_below_1mm_mm
        }
    }

    /// Does the preset fit the board's stackup: the same number of copper layers, and – when the
    /// stackup is in the board file – a board thickness within the fab's tolerance of the
    /// template's nominal thickness.
    pub fn fit(&self, su: &Stackup) -> Fit {
        let board_mm = su.from_file.then(|| (su.total_thickness() * 1e4).round() / 1e4);
        let mut problem = None;
        if su.n() != self.layers {
            problem = Some(format!(
                "{} has {} copper layers, the board {} – a template applies only to a board with the same layer count",
                self.id,
                self.layers,
                su.n()
            ));
        } else if let Some(b) = board_mm {
            let tol = self.thickness_tolerance_mm();
            if (b - self.nominal_mm).abs() > tol {
                problem = Some(format!(
                    "{} is a {} mm board, the board's stackup is {} mm thick (more than the fab's ±{} mm) – choose a template of the board's thickness or change the thickness in KiCad's stackup",
                    self.id,
                    num(self.nominal_mm),
                    num(b),
                    num(tol)
                ));
            }
        }
        Fit { ok: problem.is_none(), board_layers: su.n(), board_mm, problem }
    }

    /// The preset as stackup changes (all dielectrics, copper and mask). The caller checks `fit`.
    pub fn to_override(&self) -> StackupOverride {
        StackupOverride {
            dielectrics: self
                .dielectrics
                .iter()
                .enumerate()
                .map(|(i, d)| DielectricOverride {
                    index: i,
                    epsilon_r: Some(d.epsilon_r),
                    loss_tangent: Some(d.loss_tangent),
                    thickness: Some(d.thickness),
                    material: Some(d.material.clone()),
                })
                .collect(),
            mask_epsilon_r: Some(self.solder_mask.epsilon_r),
            copper: self
                .copper
                .iter()
                .enumerate()
                .map(|(i, &t)| CopperOverride { index: i, thickness: Some(t) })
                .collect(),
            mask_thickness: Some(self.solder_mask.thickness),
        }
    }

    /// Apply the preset to a stackup: its values replace the board's; the dielectrics' values
    /// count as given (no longer KiCad defaults).
    pub fn apply(&self, su: &Stackup) -> Stackup {
        let mut s = su.with_overrides(&self.to_override());
        for (d, p) in s.dielectrics.iter_mut().zip(&self.dielectrics) {
            d.assumed.clear();
            d.kind = p.kind.clone();
        }
        s
    }

    /// The document line of the sources kisi writes for this preset.
    pub fn document(&self) -> String {
        format!("{} impedance template {} (kisi stackup preset {})", self.fab, self.template, self.id)
    }

    fn url(&self, which: &str) -> Option<String> {
        self.sources.iter().filter_map(|s| s.url.clone()).find(|u| u.contains(which))
    }

    /// The preset as the `stackup` entry of `<board>.kisi.json`: every dielectric, every copper
    /// layer and the solder mask, each with a source that names the template and what is assumed.
    pub fn to_entry(&self) -> StackupEntry {
        let doc = self.document();
        let data_url = self.url("raw_json").or_else(|| self.sources.first().and_then(|s| s.url.clone()));
        let caps = self.url("capabilities");
        let guide = self.url("impedance-calculator");
        StackupEntry {
            dielectrics: self
                .dielectrics
                .iter()
                .enumerate()
                .map(|(i, d)| DielectricEntry {
                    index: i,
                    material: Some(d.material.clone()),
                    epsilon_r: Some(d.epsilon_r),
                    loss_tangent: Some(d.loss_tangent),
                    thickness: Some(d.thickness),
                    source: Some(Source {
                        kind: "datasheet".into(),
                        document: Some(doc.clone()),
                        url: data_url.clone(),
                        page: None,
                        note: Some(format!("{}: thickness from the template; {}", d.material, d.note)),
                    }),
                })
                .collect(),
            copper: self
                .copper
                .iter()
                .enumerate()
                .map(|(i, &t)| CopperEntry {
                    index: i,
                    thickness: Some(t),
                    source: Some(Source {
                        kind: "datasheet".into(),
                        document: Some(doc.clone()),
                        url: caps.clone(),
                        page: None,
                        note: Some(if i == 0 || i + 1 == self.copper.len() {
                            "finished outer copper (1 oz) as in the template".into()
                        } else {
                            "finished inner copper (0.5 oz) as JLC's impedance calculator takes it (0.6 mil)".into()
                        }),
                    }),
                })
                .collect(),
            solder_mask: Some(MaskEntry {
                epsilon_r: Some(self.solder_mask.epsilon_r),
                thickness: Some(self.solder_mask.thickness),
                source: Some(Source {
                    kind: "datasheet".into(),
                    document: Some(doc),
                    url: guide.or(caps),
                    page: None,
                    note: Some("solder mask as the fab's impedance calculator takes it (on laminate and between traces); thinner on copper".into()),
                }),
            }),
        }
    }
}

/// The default tolerances (`data/stackups.json`).
pub fn default_tolerances() -> Tolerances {
    let t = &library().tolerances;
    Tolerances {
        dielectric_thickness_pct: t.dielectric_thickness_pct,
        er_pct: t.er_pct,
        copper_pct: t.copper_pct,
        width_mm: t.width_mm,
        width_pct: None,
    }
}

/// `--tol thickness=10,er=5,copper=10,width=0.02mm` (or `width=20%`): tolerances in %, the width
/// in mm or %; keys not given keep their defaults.
pub fn parse_tol(spec: &str) -> Result<Tolerances, String> {
    let mut t = default_tolerances();
    for part in spec.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        let (k, v) = part.split_once('=').ok_or_else(|| format!("--tol: '{part}' is not KEY=VALUE"))?;
        let v = v.trim();
        let (num_txt, unit) = if let Some(x) = v.strip_suffix('%') {
            (x, "%")
        } else if let Some(x) = v.strip_suffix("mm") {
            (x, "mm")
        } else {
            (v, "")
        };
        let x: f64 = num_txt.trim().parse().map_err(|_| format!("--tol {k}: '{v}' is not a number"))?;
        if !(0.0..=50.0).contains(&x) {
            return Err(format!("--tol {k}: {v} outside 0 … 50"));
        }
        match (k.trim(), unit) {
            ("thickness" | "dielectric", "" | "%") => t.dielectric_thickness_pct = x,
            ("er" | "epsilon_r", "" | "%") => t.er_pct = x,
            ("copper", "" | "%") => t.copper_pct = x,
            ("width", "%") => t.width_pct = Some(x),
            ("width", "" | "mm") if x <= 0.5 => {
                t.width_mm = x;
                t.width_pct = None;
            }
            (k, u) => {
                return Err(format!(
                    "--tol: unknown key or unit '{k}' ({u}) – thickness, er, copper in %, width in mm or %"
                ))
            }
        }
    }
    Ok(t)
}

/// Where each stackup value comes from: per dielectric, per copper layer and for the solder mask
/// ("board file", "kisi.json (…)", "preset jlc6_3313", "--stackup FILE", "viewer edit").
#[derive(Debug, Clone, Default, Serialize)]
pub struct Provenance {
    pub dielectrics: Vec<String>,
    pub copper: Vec<String>,
    pub mask: String,
}

/// The provenance of a stackup built in kisi's order: the board file (or kisi's default), the
/// configuration's entries, a template, then stackup changes (`ov` with its label).
pub fn provenance(
    su: &Stackup,
    cfg: Option<&crate::config::BoardConfig>,
    preset: Option<&str>,
    ov: Option<(&StackupOverride, &str)>,
) -> Provenance {
    let base = if su.from_file { "board file" } else { "kisi's default (no stackup in the board file)" };
    let mut p = Provenance {
        dielectrics: vec![base.to_string(); su.dielectrics.len()],
        copper: vec![base.to_string(); su.copper.len()],
        mask: base.to_string(),
    };
    let cite = |s: &Option<Source>| match s {
        Some(s) => format!("kisi.json ({}{})", s.kind, s.citation().map(|c| format!(": {c}")).unwrap_or_default()),
        None => "kisi.json".to_string(),
    };
    if let Some(c) = cfg {
        for d in &c.stackup.dielectrics {
            if let Some(x) = p.dielectrics.get_mut(d.index) {
                *x = cite(&d.source);
            }
        }
        for d in &c.stackup.copper {
            if let Some(x) = p.copper.get_mut(d.index) {
                *x = cite(&d.source);
            }
        }
        if let Some(m) = &c.stackup.solder_mask {
            p.mask = cite(&m.source);
        }
    }
    if let Some(id) = preset.and_then(|id| library().get(id)) {
        let t = format!("preset {} ({} {})", id.id, id.fab, id.template);
        p.dielectrics.iter_mut().chain(p.copper.iter_mut()).for_each(|x| *x = t.clone());
        p.mask = t;
    }
    if let Some((o, label)) = ov {
        for d in &o.dielectrics {
            if let Some(x) = p.dielectrics.get_mut(d.index) {
                *x = label.to_string();
            }
        }
        for d in &o.copper {
            if let Some(x) = p.copper.get_mut(d.index) {
                *x = label.to_string();
            }
        }
        if o.mask_epsilon_r.is_some() || o.mask_thickness.is_some() {
            p.mask = label.to_string();
        }
    }
    p
}

/// Text in ASCII (SPICE and Touchstone comments): µ → u, ε → e, δ → d, other non-ASCII → ?.
pub fn ascii(t: &str) -> String {
    t.chars()
        .map(|c| match c {
            'µ' | 'μ' => "u".to_string(),
            'ε' => "e".to_string(),
            'δ' => "d".to_string(),
            '–' | '—' | '−' => "-".to_string(),
            '±' => "+-".to_string(),
            '↔' => "<->".to_string(),
            '…' => "...".to_string(),
            '’' | '‘' => "'".to_string(),
            '“' | '”' => "\"".to_string(),
            c if c.is_ascii() => c.to_string(),
            _ => "?".to_string(),
        })
        .collect()
}

fn g4(v: f64) -> String {
    let s = format!("{:.4}", v);
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" {
        "0".into()
    } else {
        s.to_string()
    }
}

/// The comment block of an export: the stackup the export used, layer by layer with its source
/// (ASCII; the caller adds the comment character).
pub fn header_lines(su: &Stackup, prov: &Provenance) -> Vec<String> {
    let mut v = vec![format!(
        "stackup used ({} copper layers, {} mm; er and tand at 1 GHz, the line models follow Djordjevic-Sarkar from there):",
        su.n(),
        g4(su.total_thickness())
    )];
    let src = |s: Option<&String>| s.map(|s| format!(" [{s}]")).unwrap_or_default();
    if let Some(m) = su.mask_top.as_ref() {
        v.push(format!(
            "  solder mask (top): {} um, er {}{}",
            g4(m.thickness * 1e3),
            g4(m.epsilon_r),
            src(Some(&prov.mask))
        ));
    }
    for (i, c) in su.copper.iter().enumerate() {
        v.push(format!("  {}: copper {} um{}", c.name, g4(c.thickness * 1e3), src(prov.copper.get(i))));
        if let Some(d) = su.dielectrics.get(i) {
            // the kind before the material, unless the material names it ("prepreg 7628x1", "core 0.4 mm")
            let kind = if d.kind.trim().is_empty() { "dielectric" } else { d.kind.trim() };
            let m = d.material.to_ascii_lowercase();
            let what = if m.contains("prepreg") || m.contains("core") || m.contains(&kind.to_ascii_lowercase()) {
                d.material.clone()
            } else {
                format!("{kind} {}", d.material)
            };
            v.push(format!(
                "    {}: {} um, er {}, tand {}{}{}",
                what,
                g4(d.thickness * 1e3),
                g4(d.epsilon_r),
                g4(d.loss_tangent),
                if d.assumed.is_empty() {
                    String::new()
                } else {
                    format!(" (KiCad default: {})", d.assumed.join(", "))
                },
                src(prov.dielectrics.get(i))
            ));
        }
    }
    if let Some(m) = su.mask_bottom.as_ref() {
        v.push(format!(
            "  solder mask (bottom): {} um, er {}{}",
            g4(m.thickness * 1e3),
            g4(m.epsilon_r),
            src(Some(&prov.mask))
        ));
    }
    v.into_iter().map(|l| ascii(&l)).collect()
}

/// The tolerance part of an export's comment block, from a study with a band (`study_json`): the
/// tolerances and, per row of the given classes (all when None), the band against the window.
pub fn tolerance_lines(study: &serde_json::Value, classes: Option<&[String]>) -> Vec<String> {
    let t = &study["tolerances"];
    if t.is_null() {
        return vec![];
    }
    let est: Vec<&str> =
        t["estimates"].as_array().map(|a| a.iter().filter_map(|x| x.as_str()).collect()).unwrap_or_default();
    let mark = |k: &str| if est.contains(&k) { " (estimate)" } else { "" };
    let f = |v: &serde_json::Value| v.as_f64().map(g4).unwrap_or_else(|| "-".into());
    let width = if t["width_pct"].is_null() {
        format!("+-{} mm", f(&t["width_mm"]))
    } else {
        format!("+-{} %", f(&t["width_pct"]))
    };
    let mut v = vec![format!(
        "manufacturing tolerances (kisi overview --tolerance): dielectric thickness +-{} %{}, er +-{} %{}, copper +-{} %{}, trace width {}{}; one-at-a-time sensitivities, worst case and RSS (information, not a verdict):",
        f(&t["dielectric_thickness_pct"]),
        mark("dielectric_thickness_pct"),
        f(&t["er_pct"]),
        mark("er_pct"),
        f(&t["copper_pct"]),
        mark("copper_pct"),
        width,
        mark("width_mm")
    )];
    for r in study["rows"].as_array().map(|a| a.as_slice()).unwrap_or(&[]) {
        let class = r["class"].as_str().unwrap_or("");
        if classes.is_some_and(|c| !c.iter().any(|x| x == class)) {
            continue;
        }
        let b = &r["tolerance"];
        if b.is_null() {
            continue;
        }
        v.push(format!(
            "  {} {}: nominal {} ohm ({}), RSS {} - {} ohm, worst case {} - {} ohm, window {} - {} ohm: {}",
            class,
            r["layer_name"].as_str().unwrap_or(""),
            f(&b["nominal_ohm"]),
            b["level"].as_str().unwrap_or(""),
            f(&b["rss"]["min_ohm"]),
            f(&b["rss"]["max_ohm"]),
            f(&b["worst_case"]["min_ohm"]),
            f(&b["worst_case"]["max_ohm"]),
            f(&r["window_ohm"][0]),
            f(&r["window_ohm"][1]),
            b["hint"].as_str().unwrap_or("")
        ));
    }
    v.into_iter().map(|l| ascii(&l)).collect()
}

/// The tolerances as used, with which are kisi's estimates (defaults) and the sources.
pub fn tolerances_json(t: &Tolerances) -> serde_json::Value {
    let d = &library().tolerances;
    let est = |k: &str, given: bool| !given && d.estimates.iter().any(|e| e == k);
    let width_given = t.width_pct.is_some() || t.width_mm != d.width_mm;
    let estimates: Vec<&str> = [
        ("dielectric_thickness_pct", t.dielectric_thickness_pct != d.dielectric_thickness_pct),
        ("er_pct", t.er_pct != d.er_pct),
        ("copper_pct", t.copper_pct != d.copper_pct),
        ("width_mm", width_given),
    ]
    .iter()
    .filter(|(k, g)| est(k, *g))
    .map(|(k, _)| *k)
    .collect();
    serde_json::json!({
        "dielectric_thickness_pct": t.dielectric_thickness_pct, "er_pct": t.er_pct, "copper_pct": t.copper_pct,
        "width_mm": if t.width_pct.is_some() { serde_json::Value::Null } else { serde_json::json!(t.width_mm) },
        "width_pct": t.width_pct,
        "estimates": estimates,
        "about": d.about, "sources": d.sources,
    })
}

/// One stackup of a study: the board's (as analysed) or another one.
#[derive(Debug, Clone)]
pub struct Column {
    pub id: String,
    pub label: String,
    /// where its values come from ("board file", "kisi.json", "preset jlc6_3313", "viewer edits")
    pub source: String,
    pub stackup: Stackup,
}

/// The target window of each class (± %, and where it comes from): the class's interface profile's
/// `z_tol_pct`, else the default window (`data/stackups.json`).
pub fn windows(
    classes: &[String],
    assign: &std::collections::HashMap<String, String>,
    profiles: &crate::profile::Profiles,
) -> std::collections::HashMap<String, (f64, String)> {
    let d = library().tolerances.window_pct;
    classes
        .iter()
        .map(|c| {
            let w = assign
                .get(c)
                .and_then(|id| profiles.get(id))
                .filter(|p| p.z_tol_pct > 0.0)
                .map(|p| (p.z_tol_pct, format!("profile {}", p.id)))
                .unwrap_or((d, "default: the fab's standard impedance tolerance (JLCPCB ±10 %)".into()));
            (c.clone(), w)
        })
        .collect()
}

fn r3(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0 + 0.0
}

/// The study: per row (a class's main cross-section on a layer) its impedance under every column's
/// stackup, and with `tol` the tolerance band under column `tol_col`. Returns the JSON and the
/// geometries the field solver would need (id, text, weight: the row's length, less for other
/// columns and tolerances).
pub fn study_json(
    board: &crate::Board,
    an: &crate::Analysis,
    field: &std::collections::HashMap<String, crate::extract::FieldResult>,
    cols: &[Column],
    tol: Option<(&Tolerances, usize)>,
    windows: &std::collections::HashMap<String, (f64, String)>,
) -> (serde_json::Value, Vec<(String, String, f64)>) {
    use crate::study;
    use serde_json::json;
    let rows = study::rows(board, an);
    let acc = crate::accuracy::table();
    let mut need: Vec<(String, String, f64)> = Vec::new();
    let mut add = |id: &str, text: &str, w: f64| {
        if !field.contains_key(id) && !need.iter().any(|x| x.0 == id) {
            need.push((id.to_string(), text.to_string(), w));
        }
    };
    let default_w = (library().tolerances.window_pct, "default".to_string());
    let mut out = Vec::new();
    for r in &rows {
        let (win, win_src) = windows.get(&r.class).cloned().unwrap_or_else(|| default_w.clone());
        let t = r.target;
        let (lo, hi) = (t.z * (1.0 - win / 100.0), t.z * (1.0 + win / 100.0));
        let formula_unc =
            r.xs.as_ref()
                .and_then(|x| {
                    let mut x = x.clone();
                    x.solver = "formula";
                    crate::accuracy::impedance_of(&x)
                })
                .map(|a| a.1)
                .unwrap_or(acc.impedance.formula_coupled_pct);
        let mut cells = Vec::new();
        let mut nominal = None;
        for (ci, c) in cols.iter().enumerate() {
            let e = study::eval(&c.stackup, &r.desc, t, field);
            if let Some(g) = e.as_ref().and_then(|e| e.geom.as_ref()) {
                add(&g.0, &g.1, if ci == 0 { r.length_mm } else { 0.5 * r.length_mm });
            }
            cells.push(match &e {
                Some(e) => json!({"z_ohm": r3(e.z), "dev_pct": r3(100.0 * (e.z - t.z) / t.z), "in_window": e.z >= lo && e.z <= hi,
                    "level": if e.field { "field solver" } else { "formula" },
                    "uncertainty_pct": if e.field { acc.impedance.field_pct } else { formula_unc }}),
                None => json!({"z_ohm": null}),
            });
            if tol.is_some_and(|(_, k)| k == ci) {
                nominal = e;
            }
        }
        let mut row = json!({
            "class": r.class, "target_ohm": t.z, "diff": t.diff, "layer": r.layer, "layer_name": r.layer_name,
            "kind": r.kind, "w_mm": r3(r.w_mm * 1000.0) / 1000.0, "gap_mm": r.gap_mm.map(|g| r3(g * 1000.0) / 1000.0), "coplanar": r.coplanar,
            "length_mm": r3(r.length_mm), "share_of_layer": r3(r.share_of_layer), "share_of_class": r3(r.share_of_class),
            "window_pct": win, "window_source": win_src, "window_ohm": [r3(lo), r3(hi)], "cells": cells,
        });
        if let (Some((tl, k)), Some(n)) = (tol, nominal.as_ref()) {
            let (b, need_b) = study::band(&cols[k].stackup, &r.desc, t, tl, field, n);
            for (id, text) in need_b {
                add(&id, &text, 1e-3 * r.length_mm);
            }
            if let Some(b) = b {
                let span = |pm: f64| {
                    let (a, z) = (n.z - pm, n.z + pm);
                    json!({"plus_minus_ohm": r3(pm), "min_ohm": r3(a), "max_ohm": r3(z),
                        "min_dev_pct": r3(100.0 * (a - t.z) / t.z), "max_dev_pct": r3(100.0 * (z - t.z) / t.z), "in_window": a >= lo && z <= hi})
                };
                let nin = n.z >= lo && n.z <= hi;
                let (win_w, win_r) = (
                    n.z - b.worst_ohm >= lo && n.z + b.worst_ohm <= hi,
                    n.z - b.rss_ohm >= lo && n.z + b.rss_ohm <= hi,
                );
                let (status, hint) = match (nin, win_r, win_w) {
                    (false, _, _) => ("nominal_outside", "the nominal value is outside the window"),
                    (true, true, true) => ("inside", "the tolerance band stays in the window"),
                    (true, true, false) => (
                        "worst_case_leaves",
                        "nominal in the window; the worst-case band leaves it, the RSS band stays in",
                    ),
                    (true, false, _) => {
                        ("leaves", "nominal in the window, but the tolerance band leaves it (RSS and worst case)")
                    }
                };
                row["tolerance"] = json!({
                    "column": cols[k].id, "nominal_ohm": r3(n.z), "level": b.level,
                    "sensitivities": b.sens.iter().map(|s| json!({"kind": s.kind, "what": s.what, "nominal": s.nominal, "delta": r3(s.delta * 1e6) / 1e6, "dz_ohm": r3(s.dz)})).collect::<Vec<_>>(),
                    "by_kind_ohm": b.by_kind.iter().map(|(k, v)| (k.to_string(), json!(r3(*v)))).collect::<serde_json::Map<_, _>>(),
                    "worst_case": span(b.worst_ohm), "rss": span(b.rss_ohm), "band_status": status, "hint": hint,
                });
            }
        }
        out.push(row);
    }
    let mut v = json!({
        "columns": cols.iter().map(|c| json!({"id": c.id, "label": c.label, "source": c.source, "layers": c.stackup.n(),
            "thickness_mm": (c.stackup.total_thickness() * 1e4).round() / 1e4,
            "dielectrics": c.stackup.dielectrics.iter().map(|d| json!({"kind": d.kind, "material": d.material, "thickness_mm": d.thickness, "er": d.epsilon_r, "tand": d.loss_tangent})).collect::<Vec<_>>(),
            "copper_um": c.stackup.copper.iter().map(|x| (x.thickness * 1e4).round() / 10.0).collect::<Vec<_>>()})).collect::<Vec<_>>(),
        "rows": out,
        "rows_note": format!("one row per net class and layer with at least {} % of the class's length: the cross-section with the most length there (pads left out; for a differential target a coupled one), evaluated under each stackup with the same references, coplanar copper and partner", (study::MIN_LAYER_SHARE * 100.0).round()),
    });
    if let Some((t, k)) = tol {
        v["tolerances"] = tolerances_json(t);
        v["tolerance_column"] = json!(cols.get(k).map(|c| c.id.clone()));
        v["method"] = json!("one-at-a-time sensitivities: central differences dZ = (Z(x+d) - Z(x-d))/2 per dielectric thickness, dielectric er, the trace layer's copper thickness and the trace width (symmetric: gaps shrink as widths grow; no etch factor), with the solver of the nominal value where the field solver has every perturbed geometry, else the formulas scaled to the nominal value; worst case = sum |dZ|, RSS = sqrt(sum dZ^2). A linear estimate; information only – the channel verdict is unchanged.");
    }
    need.sort_by(|a, b| b.2.total_cmp(&a.2));
    (v, need)
}

fn num(v: f64) -> String {
    let s = format!("{:.3}", v);
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// A preset's summary for `kisi stackups` and the viewer, with its fit to a board's stackup.
pub fn preset_json(p: &Preset, su: Option<&Stackup>) -> serde_json::Value {
    let mut v = serde_json::json!({
        "id": p.id, "fab": p.fab, "template": p.template, "title": p.title, "layers": p.layers,
        "nominal_mm": p.nominal_mm, "pressed_mm": p.pressed_mm,
        "copper_um": p.copper.iter().map(|c| (c * 1e4).round() / 10.0).collect::<Vec<_>>(),
        "dielectrics": p.dielectrics, "solder_mask": p.solder_mask, "sources": p.sources,
    });
    if let Some(su) = su {
        v["fit"] = serde_json::to_value(p.fit(su)).unwrap_or_default();
    }
    v
}

/// `kisi stackups`: the library; with a board, which presets fit it.
pub fn list_json(su: Option<&Stackup>) -> serde_json::Value {
    let lib = library();
    let mut v = serde_json::json!({
        "presets": lib.presets.iter().map(|p| preset_json(p, su)).collect::<Vec<_>>(),
        "about": lib.about,
        "board_thickness_tolerance": {"pct": lib.board_thickness.tolerance_pct, "below_1mm_mm": lib.board_thickness.tolerance_below_1mm_mm, "sources": lib.board_thickness.sources},
        "tolerance_defaults": tolerances_json(&default_tolerances()),
    });
    if let Some(su) = su {
        v["board"] = serde_json::json!({"layers": su.n(), "thickness_mm": su.from_file.then(|| (su.total_thickness() * 1e4).round() / 1e4), "stackup_from_file": su.from_file});
        v["fitting"] =
            serde_json::json!(lib.presets.iter().filter(|p| p.fit(su).ok).map(|p| p.id.clone()).collect::<Vec<_>>());
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stack(n: usize, d: f64) -> Stackup {
        use crate::board::{CopperLayer, Dielectric, SolderMask};
        Stackup {
            copper: (0..n).map(|i| CopperLayer { name: format!("L{i}"), thickness: 0.035 }).collect(),
            dielectrics: (0..n - 1)
                .map(|_| Dielectric {
                    name: "d".into(),
                    kind: "core".into(),
                    material: "FR4".into(),
                    thickness: d,
                    epsilon_r: 4.5,
                    loss_tangent: 0.02,
                    assumed: vec!["epsilon_r".into()],
                })
                .collect(),
            mask_top: Some(SolderMask { thickness: 0.01, epsilon_r: 3.3 }),
            mask_bottom: Some(SolderMask { thickness: 0.01, epsilon_r: 3.3 }),
            from_file: true,
        }
    }

    #[test]
    fn library_reads() {
        let l = library();
        assert!(l.presets.len() >= 6);
        for p in &l.presets {
            assert_eq!(p.copper.len(), p.layers, "{}", p.id);
            assert_eq!(p.dielectrics.len() + 1, p.layers, "{}", p.id);
            // the pressed thickness is the sum of the layers (within the template's rounding)
            let sum: f64 = p.copper.iter().sum::<f64>() + p.dielectrics.iter().map(|d| d.thickness).sum::<f64>();
            assert!((sum - p.pressed_mm).abs() < 0.01, "{}: {sum} vs {}", p.id, p.pressed_mm);
            assert!((p.pressed_mm - p.nominal_mm).abs() <= p.thickness_tolerance_mm(), "{}", p.id);
        }
        assert_eq!(l.find("JLC04161H-7628").unwrap().id, "jlc4_7628");
        assert!(l.find("nope").unwrap_err().contains("jlc4_7628"));
    }

    #[test]
    fn fit_and_apply() {
        let p = library().find("jlc4_7628").unwrap();
        // 4 layers, 1.6 mm: fits
        let su = stack(4, (1.6 - 4.0 * 0.035) / 3.0);
        assert!(p.fit(&su).ok, "{:?}", p.fit(&su));
        // a 6-layer board: no
        let f = p.fit(&stack(6, 0.27));
        assert!(!f.ok && f.problem.unwrap().contains("6"));
        // 4 layers but 2.0 mm thick: no
        let f = p.fit(&stack(4, (2.0 - 0.14) / 3.0));
        assert!(!f.ok && f.problem.unwrap().contains("thick"));
        // a default stackup (not in the file): the thickness is not known, the layer count decides
        let mut d = stack(4, 0.6);
        d.from_file = false;
        assert!(p.fit(&d).ok);
        // applied: every value of the template, the defaults no longer assumed
        let a = p.apply(&su);
        assert_eq!(a.dielectrics[0].thickness, 0.2104);
        assert_eq!(a.dielectrics[1].epsilon_r, 4.6);
        assert_eq!(a.dielectrics[1].material, "core 1.1 mm H/HOZ (NP-155F)");
        assert_eq!((a.dielectrics[0].kind.as_str(), a.dielectrics[1].kind.as_str()), ("prepreg", "core"));
        assert!(a.dielectrics.iter().all(|d| d.assumed.is_empty()));
        assert_eq!(a.copper[1].thickness, 0.0152);
        assert_eq!(a.mask_top.as_ref().unwrap().thickness, 0.0305);
        assert_eq!(a.mask_bottom.as_ref().unwrap().epsilon_r, 3.8);
        assert!((a.total_thickness() - p.pressed_mm).abs() < 1e-6);
    }

    /// The kisi.json entry of a preset passes `config::check` and gives the same stackup as the
    /// preset applied directly.
    #[test]
    fn entry_is_valid_config() {
        let p = library().find("jlc4_3313").unwrap();
        let cfg = crate::config::BoardConfig {
            kisi_config: crate::config::VERSION,
            stackup: p.to_entry(),
            ..Default::default()
        };
        let text = serde_json::to_string(&crate::config::to_json(&cfg)).unwrap();
        let back = crate::config::parse(&text).unwrap();
        assert_eq!(back.stackup.copper.len(), 4);
        let su = stack(4, 0.48);
        let mut board = crate::Board { stackup: Some(su.clone()), ..Default::default() };
        let r =
            crate::config::check(&back, &board, &[], &crate::profile::Profiles::builtin(), None, &Default::default());
        assert!(r.ok, "{:?}", r.errors);
        assert_eq!(r.entries, 3 + 4 + 1);
        back.apply(&mut board);
        let (a, b) = (board.stackup(), p.apply(&su));
        for (x, y) in a.dielectrics.iter().zip(&b.dielectrics) {
            assert_eq!(
                (x.thickness, x.epsilon_r, x.loss_tangent, &x.material),
                (y.thickness, y.epsilon_r, y.loss_tangent, &y.material)
            );
        }
        for (x, y) in a.copper.iter().zip(&b.copper) {
            assert_eq!(x.thickness, y.thickness);
        }
        assert_eq!(a.mask_top.as_ref().unwrap().thickness, 0.0305);
        // the sources name the template, and the assumed tanδ
        let s = back.stackup.dielectrics[0].source.as_ref().unwrap();
        assert!(s.document.as_deref().unwrap().contains("JLC04161H-3313"));
        assert!(s.note.as_deref().unwrap().contains("assumption"));
    }

    #[test]
    fn tolerance_spec() {
        let d = default_tolerances();
        assert_eq!((d.dielectric_thickness_pct, d.er_pct, d.copper_pct, d.width_mm), (10.0, 5.0, 10.0, 0.02));
        let t = parse_tol("thickness=8, er=3,width=15%").unwrap();
        assert_eq!((t.dielectric_thickness_pct, t.er_pct, t.copper_pct, t.width_pct), (8.0, 3.0, 10.0, Some(15.0)));
        let t = parse_tol("width=0.03mm,copper=0").unwrap();
        assert_eq!((t.width_mm, t.width_pct, t.copper_pct), (0.03, None, 0.0));
        assert!(parse_tol("width=2").is_err());
        assert!(parse_tol("er=60").is_err());
        assert!(parse_tol("foo=1").is_err());
        assert!(parse_tol("er").is_err());
        // defaults are reported as estimates, given values are not
        let j = tolerances_json(&parse_tol("er=4").unwrap());
        let est: Vec<&str> = j["estimates"].as_array().unwrap().iter().filter_map(|x| x.as_str()).collect();
        assert_eq!(est, ["dielectric_thickness_pct", "copper_pct", "width_mm"]);
    }

    /// A microstrip pair and a single line with targets: the study's rows, a template column and
    /// the band; a thinner prepreg lowers the impedance.
    #[test]
    fn study_on_a_board() {
        let pcb = r#"(kicad_pcb (version 20241229) (generator "pcbnew")
  (layers (0 "F.Cu" signal) (4 "In1.Cu" signal) (6 "In2.Cu" signal) (2 "B.Cu" signal))
  (setup (stackup
    (layer "F.Mask" (type "Top Solder Mask") (thickness 0.01))
    (layer "F.Cu" (type "copper") (thickness 0.035))
    (layer "dielectric 1" (type "prepreg") (thickness 0.2) (material "7628") (epsilon_r 4.4) (loss_tangent 0.02))
    (layer "In1.Cu" (type "copper") (thickness 0.0175))
    (layer "dielectric 2" (type "core") (thickness 1.1) (material "FR4") (epsilon_r 4.6) (loss_tangent 0.02))
    (layer "In2.Cu" (type "copper") (thickness 0.0175))
    (layer "dielectric 3" (type "prepreg") (thickness 0.2) (material "7628") (epsilon_r 4.4) (loss_tangent 0.02))
    (layer "B.Cu" (type "copper") (thickness 0.035))
    (layer "B.Mask" (type "Bottom Solder Mask") (thickness 0.01))))
  (net 0 "") (net 1 "SIG") (net 2 "GND") (net 3 "D_P") (net 4 "D_N")
  (segment (start 0 0) (end 30 0) (width 0.35) (layer "F.Cu") (net 1))
  (segment (start 0 5) (end 30 5) (width 0.2) (layer "F.Cu") (net 3))
  (segment (start 0 5.35) (end 30 5.35) (width 0.2) (layer "F.Cu") (net 4))
  (zone (net 2) (net_name "GND") (layer "In1.Cu") (filled_polygon (layer "In1.Cu") (pts (xy -5 -5) (xy 35 -5) (xy 35 10) (xy -5 10)))))
"#;
        let cfg = r#"{"kisi_config": 1, "net_classes": {
            "SE50": {"target_ohm": 50, "diff": false, "nets": ["SIG"], "source": {"kind": "design", "note": "t"}},
            "D100": {"target_ohm": 100, "diff": true, "nets": ["D_*"], "source": {"kind": "design", "note": "t"}}}}"#;
        let p = crate::service::Project::from_texts(crate::service::Inputs {
            pcb_path: "t.kicad_pcb".into(),
            pcb: pcb.into(),
            pro: None,
            config: Some(("t.kisi.json".into(), Ok(cfg.into()))),
            stackup: None,
            stackup_preset: None,
            field: Default::default(),
            variant: None,
            files: Default::default(),
        })
        .unwrap();
        let su = p.board.stackup().clone();
        let pr = library().find("jlc4_3313").unwrap();
        let cols = vec![
            Column { id: "board".into(), label: "board".into(), source: "board file".into(), stackup: su.clone() },
            Column { id: pr.id.clone(), label: pr.template.clone(), source: pr.document(), stackup: pr.apply(&su) },
        ];
        let w = windows(&["SE50".into(), "D100".into()], &Default::default(), &crate::profile::Profiles::builtin());
        let tol = default_tolerances();
        let (v, need) = study_json(&p.board, &p.an, &p.params.field, &cols, Some((&tol, 0)), &w);
        let rows = v["rows"].as_array().unwrap();
        assert_eq!(rows.len(), 2, "{v}");
        let d = rows.iter().find(|r| r["class"] == "D100").unwrap();
        assert!(d["gap_mm"].as_f64().is_some() && d["diff"] == true);
        for r in rows {
            let (a, b) = (r["cells"][0]["z_ohm"].as_f64().unwrap(), r["cells"][1]["z_ohm"].as_f64().unwrap());
            // the 3313 template's prepreg is half as thick: lower impedance
            assert!(b < a - 3.0, "{r}");
            let t = &r["tolerance"];
            let (wc, rss) =
                (t["worst_case"]["plus_minus_ohm"].as_f64().unwrap(), t["rss"]["plus_minus_ohm"].as_f64().unwrap());
            assert!(rss > 0.5 && rss < wc, "{t}");
            // the core and the bottom prepreg are outside the microstrip: not varied
            assert!(
                t["sensitivities"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|s| s["what"].as_str().unwrap().contains("F.Cu") || s["kind"] == "width"),
                "{t}"
            );
            assert_eq!(r["window_pct"], 10.0);
        }
        // the field solver would need the nominal geometries of both columns and the perturbed ones
        assert!(need.len() > 4);
        assert!(need[0].2 >= need.last().unwrap().2);
        assert_eq!(v["tolerances"]["estimates"].as_array().unwrap().len(), 4);
    }

    /// Where each value comes from, the header lines (ASCII) and how each export format carries them.
    #[test]
    fn provenance_and_stamp() {
        let su = stack(4, 0.5);
        let cfg = crate::config::parse(
            r#"{"kisi_config": 1, "stackup": {"dielectrics": [{"index": 1, "epsilon_r": 4.2,
            "source": {"kind": "datasheet", "document": "laminate X"}}]}}"#,
        )
        .unwrap();
        let ov =
            StackupOverride { copper: vec![CopperOverride { index: 3, thickness: Some(0.02) }], ..Default::default() };
        let p = provenance(&su, Some(&cfg), None, Some((&ov, "viewer edit")));
        assert_eq!(p.dielectrics, ["board file", "kisi.json (datasheet: laminate X)", "board file"]);
        assert_eq!((p.copper[3].as_str(), p.copper[0].as_str()), ("viewer edit", "board file"));
        let pp = provenance(&su, Some(&cfg), Some("jlc4_7628"), None);
        assert!(pp.dielectrics.iter().all(|x| x.starts_with("preset jlc4_7628")));
        let mut su2 = su.clone();
        su2.dielectrics[0].material = "Fancy µ-laminate".into();
        let lines = header_lines(&su2, &p);
        assert!(lines.iter().all(|l| l.is_ascii()), "{lines:?}");
        assert!(lines[0].contains("4 copper layers") && lines.iter().any(|l| l.contains("Fancy u-laminate")));
        assert!(
            lines
                .iter()
                .any(|l| l.contains("tand 0.02 (KiCad default: epsilon_r) [kisi.json (datasheet: laminate X)]")),
            "{lines:?}"
        );
        use crate::export::files::ExportFile;
        let f = |p: &str, c: &str| ExportFile { path: p.into(), content: c.into() };
        let mut files = vec![
            f("x.lib", "* kisi: lane\n* board b\n.subckt a 1 2\n.ends\n"),
            f("bench_ngspice.cir", "* title\nR1 1 0 50\n.end\n"),
            f("x.s2p", "! kisi\n# HZ S RI R 50\n1e9 0 0 1 0 1 0 0 0\n"),
            f("README.md", "# x\n"),
            f("x.kicad_sch", "(kicad_sch\n  (title_block\n    (title \"t\")\n    (comment 1 \"c\")\n  )\n)\n"),
            f("x.kicad_pro", "{}"),
        ];
        let head = vec!["stackup used (2 copper layers):".to_string(), "  F.Cu: copper 35 um".to_string()];
        crate::channel_export::stamp(&mut files, &head);
        assert_eq!(files[0].content, "* kisi: lane\n* board b\n* stackup used (2 copper layers):\n*   F.Cu: copper 35 um\n.subckt a 1 2\n.ends\n");
        assert!(files[1].content.starts_with("* title\n* stackup used"));
        assert!(files[2]
            .content
            .starts_with("! kisi\n! stackup used (2 copper layers):\n!   F.Cu: copper 35 um\n# HZ"));
        assert!(files[3]
            .content
            .ends_with("## Stackup\n\n```text\nstackup used (2 copper layers):\n  F.Cu: copper 35 um\n```\n"));
        assert!(files[4].content.contains(
            "    (comment 1 \"c\")\n    (comment 2 \"stackup used (2 copper layers) - layers in the README\")\n"
        ));
        assert_eq!(files[5].content, "{}");
    }
}

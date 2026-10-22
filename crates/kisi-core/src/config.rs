//! Board configuration file `<board>.kisi.json`: values that the board file does
//! not contain (or contains only as defaults), each with its source.
//!
//! Written by a person or an agent (see the kisi-prepare skill), read by the CLI
//! and the viewer. kisi never writes into the KiCad files; fields given here are
//! laid over the footprints' fields when the board is loaded.
//!
//! ```json
//! {
//!   "kisi_config": 1,
//!   "board": "ts.kicad_pcb",
//!   "stackup": {
//!     "dielectrics": [{"index": 0, "epsilon_r": 3.65, "loss_tangent": 0.0095,
//!                      "source": {"kind": "datasheet", "document": "Isola FR408HR", "url": "…", "page": "2"}}],
//!     "solder_mask": {"epsilon_r": 3.8, "source": {"kind": "assumption", "note": "typical LPI mask"}}
//!   },
//!   "net_classes": {"PCIe": {"target_ohm": 85, "diff": true, "profile": "pcie3", "source": {…}}},
//!   "fields": [{"ref": "U5", "name": "PI_LOAD", "value": "+1V0=2.5A, +1V8=300mA", "source": {…}}],
//!   "variant": "TS-PCIe",
//!   "variants": {"TS-PCIe": {"not_fitted_sheets": ["/TS-USB4 Components/"], "fitted": ["C101"], "source": {…}},
//!                "TS-USB4": {"bom": "TS-USB4_BOM.csv", "bom_exceptions": ["P1"], "source": {…}}},
//!   "notes": ["…"]
//! }
//! ```

use crate::board::{Board, DielectricOverride, StackupOverride};
use crate::extract::Target;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

pub const VERSION: u32 = 1;
/// kinds of sources; "estimate" and "assumption" are reported as such; "design" = the
/// project's own files (BOM, position files, schematic)
pub const SOURCE_KINDS: &[&str] = &["datasheet", "standard", "application_note", "measurement", "simulation", "design", "estimate", "assumption"];

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Source {
    pub kind: String,
    pub document: Option<String>,
    pub url: Option<String>,
    pub page: Option<String>,
    /// e.g. "typical", "maximum", "at 10 GHz"
    pub note: Option<String>,
}

impl Source {
    fn is_empty(&self) -> bool {
        self.document.is_none() && self.url.is_none() && self.note.is_none()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DielectricEntry {
    pub index: usize,
    pub material: Option<String>,
    pub epsilon_r: Option<f64>,
    pub loss_tangent: Option<f64>,
    /// mm
    pub thickness: Option<f64>,
    pub source: Option<Source>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct MaskEntry {
    pub epsilon_r: Option<f64>,
    pub source: Option<Source>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct StackupEntry {
    pub dielectrics: Vec<DielectricEntry>,
    pub solder_mask: Option<MaskEntry>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ClassEntry {
    /// single-ended impedance, or differential impedance when `diff`
    pub target_ohm: Option<f64>,
    pub diff: Option<bool>,
    /// interface profile id (data/interfaces.json)
    pub profile: Option<String>,
    pub source: Option<Source>,
}

/// `connector_ends.<connector reference>`: what sits at a connector where lanes leave the
/// board – the termination when it receives, the driver when it sends – and the channel
/// behind it: {"model": "profile" | "matched" | "high_impedance" | "custom", "r_ohm", "c_pf",
/// "l_nh", "rest_loss_db", "rest_delay_ps", "source"}.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ConnectorEndEntry {
    #[serde(flatten)]
    pub end: kisi_si::eye::ConnEnd,
    pub source: Option<Source>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct FieldEntry {
    #[serde(rename = "ref")]
    pub refdes: String,
    pub name: String,
    pub value: String,
    pub source: Option<Source>,
}

/// An assembly variant for boards without KiCad 10 variants (or on top of them): which
/// parts are fitted. Applied in this order, later rules win: the board file (DNP, and the
/// KiCad 10 variant of the same name), `bom`, the sheets, the reference lists.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct VariantEntry {
    pub description: Option<String>,
    /// hierarchical sheets whose parts are fitted / not fitted ("/Power/"; a sheet covers
    /// its sub-sheets)
    pub fitted_sheets: Vec<String>,
    pub not_fitted_sheets: Vec<String>,
    /// reference designators fitted / not fitted (wildcards `*` and `?` allowed)
    pub fitted: Vec<String>,
    pub not_fitted: Vec<String>,
    /// a BOM (CSV, path relative to the kisi.json): parts in it are fitted, parts not in it
    /// are not – except parts marked "exclude from BOM" in KiCad and `bom_exceptions`
    pub bom: Option<String>,
    /// parts never listed in a BOM that stay as in the board file (gold fingers, test points)
    pub bom_exceptions: Vec<String>,
    pub source: Option<Source>,
}

impl VariantEntry {
    pub fn is_empty(&self) -> bool {
        self.fitted_sheets.is_empty() && self.not_fitted_sheets.is_empty() && self.fitted.is_empty() && self.not_fitted.is_empty() && self.bom.is_none()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct BoardConfig {
    pub kisi_config: u32,
    pub board: Option<String>,
    pub stackup: StackupEntry,
    pub net_classes: BTreeMap<String, ClassEntry>,
    /// lanes ending at a connector: the end model there and the channel behind it, per
    /// connector reference ("P1", "J8")
    pub connector_ends: BTreeMap<String, ConnectorEndEntry>,
    pub fields: Vec<FieldEntry>,
    /// the assembly variant to analyse (a KiCad 10 variant of the board, one of `variants`,
    /// or "default" for the board file as it is)
    pub variant: Option<String>,
    pub variants: BTreeMap<String, VariantEntry>,
    pub notes: Vec<String>,
}

/// Fields kisi evaluates (others are laid over too, but have no effect).
pub const KNOWN_FIELDS: &[&str] = &["PI_LOAD", "PI_SOURCE", "PI_REG", "PI_CAP", "PI_TARGET", "PI_SERIES"];

pub fn parse(text: &str) -> Result<BoardConfig, String> {
    serde_json::from_str(text).map_err(|e| format!("kisi.json: {e}"))
}

/// What the configuration changes for the analyses.
#[derive(Debug, Clone, Default)]
pub struct Applied {
    pub targets: HashMap<String, Target>,
    /// net class → profile id
    pub profiles: HashMap<String, String>,
    pub fields: usize,
    pub dielectrics: usize,
}

impl BoardConfig {
    pub fn stackup_override(&self) -> StackupOverride {
        StackupOverride {
            dielectrics: self
                .stackup
                .dielectrics
                .iter()
                .map(|d| DielectricOverride { index: d.index, epsilon_r: d.epsilon_r, loss_tangent: d.loss_tangent, thickness: d.thickness })
                .collect(),
            mask_epsilon_r: self.stackup.solder_mask.as_ref().and_then(|m| m.epsilon_r),
        }
    }

    /// Lay the configuration over the board: stackup values and footprint fields.
    /// Returns the class targets and profile assignments for the analyses.
    pub fn apply(&self, board: &mut Board) -> Applied {
        let mut a = Applied::default();
        if !self.stackup.dielectrics.is_empty() || self.stackup.solder_mask.is_some() {
            let mut su = board.stackup().with_overrides(&self.stackup_override());
            for d in &self.stackup.dielectrics {
                if let Some(x) = su.dielectrics.get_mut(d.index) {
                    if let Some(m) = &d.material {
                        x.material = m.clone();
                    }
                    let given: Vec<&str> = [("material", d.material.is_some()), ("epsilon_r", d.epsilon_r.is_some()), ("loss_tangent", d.loss_tangent.is_some()), ("thickness", d.thickness.is_some())]
                        .iter()
                        .filter(|g| g.1)
                        .map(|g| g.0)
                        .collect();
                    x.assumed.retain(|p| !given.contains(&p.as_str()));
                    a.dielectrics += 1;
                }
            }
            board.stackup = Some(su);
        }
        for f in &self.fields {
            if let Some(fp) = board.footprints.iter_mut().find(|fp| fp.reference == f.refdes) {
                let mut set = |k: &str, v: String| match fp.props.iter_mut().find(|(n, _)| n.eq_ignore_ascii_case(k)) {
                    Some(p) => p.1 = v,
                    None => fp.props.push((k.to_string(), v)),
                };
                set(&f.name, f.value.clone());
                // the source travels with the field, so the power tree can show it per load
                if let Some(src) = &f.source {
                    set(&source_key(&f.name), format!("{}\t{}", src.kind, src.note.as_deref().or(src.document.as_deref()).unwrap_or("")));
                }
                a.fields += 1;
            }
        }
        for (cls, c) in &self.net_classes {
            if let Some(z) = c.target_ohm {
                a.targets.insert(cls.clone(), Target { z, diff: c.diff.unwrap_or(false) });
            }
            if let Some(p) = &c.profile {
                a.profiles.insert(cls.clone(), p.clone());
            }
        }
        a
    }
}

/// Footprint property that carries the kisi.json source of field `name`
/// ("kind\tnote"); read by the power tree (`kisi_pi::tree::field_source`).
pub fn source_key(name: &str) -> String {
    format!("{}#source", name.to_ascii_uppercase())
}

/// Result of `check`.
#[derive(Debug, Clone, Default, Serialize)]
pub struct CheckResult {
    pub ok: bool,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    pub entries: usize,
}

/// Board names match when their stems (without path and `.kicad_pcb`) are equal, ignoring case.
pub fn same_board(a: &str, b: &str) -> bool {
    let stem = |s: &str| s.trim().rsplit(['/', '\\']).next().unwrap_or("").trim_end_matches(".kicad_pcb").to_ascii_lowercase();
    stem(a) == stem(b)
}

/// Check a configuration against the board (as loaded from the file, before `apply`):
/// format, ranges, references, sources, and whether kisi can read the field values.
/// `board_name`: the loaded board (file name or viewer name), compared with `cfg.board`;
/// `files`: the files the configuration refers to (BOMs of variants).
pub fn check(cfg: &BoardConfig, board: &Board, class_names: &[String], profiles: &crate::profile::Profiles, board_name: Option<&str>, files: &crate::variant::Files) -> CheckResult {
    let mut r = CheckResult::default();
    let src = |what: String, s: &Option<Source>, r: &mut CheckResult| match s {
        None => r.errors.push(format!("{what}: source missing (every value needs a source)")),
        Some(s) if s.is_empty() => r.errors.push(format!("{what}: source needs a document, url or note")),
        Some(s) if !SOURCE_KINDS.contains(&s.kind.as_str()) => r.errors.push(format!("{what}: source kind '{}' unknown (use {})", s.kind, SOURCE_KINDS.join(", "))),
        _ => {}
    };
    if cfg.kisi_config != VERSION {
        r.errors.push(format!("kisi_config must be {VERSION} (found {})", cfg.kisi_config));
    }
    if let (Some(want), Some(have)) = (cfg.board.as_deref(), board_name) {
        if !same_board(want, have) {
            r.errors.push(format!("this kisi.json was written for '{want}', but the loaded board is '{have}' – wrong file or wrong board? (the errors below follow from that)"));
        }
    }
    let su = board.stackup();
    for d in &cfg.stackup.dielectrics {
        r.entries += 1;
        let what = format!("stackup.dielectrics[index {}]", d.index);
        if d.index >= su.dielectrics.len() {
            r.errors.push(format!("{what}: the board has {} dielectrics (index 0 = below F.Cu)", su.dielectrics.len()));
        }
        if d.epsilon_r.is_some_and(|v| !(1.0..20.0).contains(&v)) {
            r.errors.push(format!("{what}: epsilon_r outside 1 … 20"));
        }
        if d.loss_tangent.is_some_and(|v| !(0.0..0.2).contains(&v)) {
            r.errors.push(format!("{what}: loss_tangent outside 0 … 0.2"));
        }
        if d.thickness.is_some_and(|v| !(0.005..5.0).contains(&v)) {
            r.errors.push(format!("{what}: thickness in mm outside 0.005 … 5"));
        }
        if d.epsilon_r.is_none() && d.loss_tangent.is_none() && d.thickness.is_none() && d.material.is_none() {
            r.warnings.push(format!("{what}: no value given"));
        }
        src(what, &d.source, &mut r);
    }
    if let Some(m) = &cfg.stackup.solder_mask {
        r.entries += 1;
        if m.epsilon_r.is_some_and(|v| !(1.0..20.0).contains(&v)) {
            r.errors.push("stackup.solder_mask: epsilon_r outside 1 … 20".into());
        }
        src("stackup.solder_mask".into(), &m.source, &mut r);
    }
    for (cls, c) in &cfg.net_classes {
        r.entries += 1;
        let what = format!("net_classes.{cls}");
        if !class_names.iter().any(|n| n == cls) {
            r.errors.push(format!("{what}: no net class '{cls}' in the project (classes: {})", class_names.join(", ")));
        }
        if c.target_ohm.is_some_and(|z| !(10.0..200.0).contains(&z)) {
            r.errors.push(format!("{what}: target_ohm outside 10 … 200"));
        }
        if c.target_ohm.is_some() && c.diff.is_none() {
            r.warnings.push(format!("{what}: diff not given (taken as single-ended)"));
        }
        if let Some(p) = &c.profile {
            match profiles.get(p) {
                None => r.errors.push(format!("{what}: unknown profile '{p}' (see `kisi profiles`)")),
                Some(pr) => {
                    if let (Some(d), true) = (c.diff, !pr.signal.is_empty()) {
                        if d != (pr.signal == "diff") {
                            r.warnings.push(format!("{what}: profile '{p}' is {} but diff = {d}", pr.signal));
                        }
                    }
                }
            }
        }
        if c.target_ohm.is_none() && c.profile.is_none() {
            r.warnings.push(format!("{what}: neither target_ohm nor profile given"));
        }
        src(what, &c.source, &mut r);
    }
    for (refdes, e) in &cfg.connector_ends {
        r.entries += 1;
        let w = format!("connector_ends.{refdes}");
        if !board.footprints.iter().any(|f| &f.reference == refdes) {
            r.errors.push(format!("{w}: no part '{refdes}' on the board"));
        }
        let m = e.end.model.as_str();
        if !m.is_empty() && !kisi_si::eye::ConnEnd::MODELS.contains(&m) {
            r.errors.push(format!("{w}: unknown model '{m}' ({})", kisi_si::eye::ConnEnd::MODELS.join(", ")));
        }
        if m == "custom" && e.end.r_ohm.is_none() {
            r.errors.push(format!("{w}: model 'custom' needs r_ohm (Ω per line; 0 = high impedance)"));
        }
        if e.end.r_ohm.is_some_and(|v| !(0.0..=10000.0).contains(&v)) || e.end.c_pf.is_some_and(|v| !(0.0..=100.0).contains(&v)) || e.end.l_nh.is_some_and(|v| !(0.0..=100.0).contains(&v)) {
            r.errors.push(format!("{w}: r_ohm 0 … 10000, c_pf and l_nh 0 … 100"));
        }
        if e.end.rest_loss_db.is_some_and(|v| !(0.0..=100.0).contains(&v)) || e.end.rest_delay_ps.is_some_and(|v| !(0.0..=100000.0).contains(&v)) {
            r.errors.push(format!("{w}: rest_loss_db 0 … 100 dB, rest_delay_ps 0 … 100000"));
        }
        if (e.end.r_ohm.is_some() || e.end.c_pf.is_some()) && !matches!(m, "custom" | "high_impedance") {
            r.warnings.push(format!("{w}: r_ohm / c_pf are only used with model 'custom' (or c_pf with 'high_impedance')"));
        }
        if let Some(tr) = &e.end.touchstone {
            if tr.port_order.as_deref().is_some_and(|o| !kisi_si::rf::PORT_ORDERS.contains(&o)) {
                r.errors.push(format!("{w}.touchstone: unknown port_order ({})", kisi_si::rf::PORT_ORDERS.join(", ")));
            }
            if e.end.rest_loss_db.is_some() {
                r.warnings.push(format!("{w}: touchstone and rest_loss_db given – the Touchstone file is used"));
            }
            let name = tr.file.rsplit(['/', '\\']).next().unwrap_or(&tr.file);
            match files.get(&tr.file) {
                None => r.errors.push(format!("{w}.touchstone: {} not loaded", tr.file)),
                Some(Err(err)) => r.errors.push(format!("{w}.touchstone: {err}")),
                Some(Ok(text)) => match kisi_si::rf::parse_touchstone(text, name) {
                    Err(err) => r.errors.push(format!("{w}.touchstone: {err}")),
                    Ok(ts) => {
                        if ts.nports == 4 && tr.port_order.is_none() {
                            r.warnings.push(format!("{w}.touchstone: 4-port without port_order – \"1-3,2-4\" (P 1→3, N 2→4) assumed"));
                        }
                        r.warnings.extend(ts.warnings.iter().map(|x| format!("{w}.touchstone: {x}")));
                    }
                },
            }
        }
        src(w, &e.source, &mut r);
    }
    let mut seen = std::collections::HashSet::new();
    for f in &cfg.fields {
        r.entries += 1;
        let what = format!("fields[{} {}]", f.refdes, f.name);
        if !board.footprints.iter().any(|fp| fp.reference == f.refdes) {
            r.errors.push(format!("{what}: no footprint {} on the board", f.refdes));
        }
        if !seen.insert((f.refdes.clone(), f.name.to_ascii_uppercase())) {
            r.errors.push(format!("{what}: given twice"));
        }
        if f.value.trim().is_empty() {
            r.errors.push(format!("{what}: empty value"));
        }
        if !KNOWN_FIELDS.iter().any(|k| k.eq_ignore_ascii_case(&f.name)) {
            r.warnings.push(format!("{what}: kisi does not evaluate this field (known: {})", KNOWN_FIELDS.join(", ")));
        }
        // PDN fields: readable values
        if f.name.eq_ignore_ascii_case("PI_CAP") {
            if let Err(e) = crate::pdn_ic::parse_pi_cap(&f.value) {
                r.errors.push(format!("{what}: {e}"));
            }
        } else if f.name.eq_ignore_ascii_case("PI_SERIES") {
            if let Err(e) = crate::pdn_ic::parse_pi_series(&f.value) {
                r.errors.push(format!("{what}: {e}"));
            }
        } else if f.name.eq_ignore_ascii_case("PI_TARGET") {
            // any rail name: parse with an impossible net so that only the syntax is checked
            for part in f.value.split([';', ',']).map(str::trim).filter(|p| !p.is_empty()) {
                if let Err(e) = crate::pdn_ic::parse_pi_target(part, part.split_whitespace().next().and_then(|t| t.rsplit_once('=').map(|x| x.0)).unwrap_or("")) {
                    r.errors.push(format!("{what}: {e}"));
                }
            }
        }
        src(what, &f.source, &mut r);
    }
    check_variants(cfg, board, files, &mut r);
    // can kisi read the power fields? apply to a copy and parse them like the analyses do
    if cfg.fields.iter().any(|f| KNOWN_FIELDS.iter().any(|k| k.eq_ignore_ascii_case(&f.name))) {
        let mut b = board.clone();
        cfg.apply(&mut b);
        let refs: Vec<&str> = cfg.fields.iter().map(|f| f.refdes.as_str()).collect();
        let mine = |w: &String| refs.iter().any(|r| w.starts_with(&format!("{r}:")));
        let (_, w1) = crate::pi::rails_from_fields(&b);
        let w2 = crate::tree::tree_from_fields(&b).map(|t| t.1).unwrap_or_default();
        for w in w1.iter().chain(&w2).filter(|w| mine(w)) {
            if !r.errors.contains(w) {
                r.errors.push(w.clone());
            }
        }
    }
    r.ok = r.errors.is_empty();
    r
}

/// The `variants` section and the `variant` choice.
fn check_variants(cfg: &BoardConfig, board: &Board, files: &crate::variant::Files, r: &mut CheckResult) {
    use crate::variant::{bom_refs, in_sheet, norm_sheet, DEFAULT};
    let mut sheets: Vec<String> = board.footprints.iter().map(|f| norm_sheet(&f.sheet)).filter(|s| !s.is_empty()).collect();
    sheets.sort();
    sheets.dedup();
    let refs: std::collections::HashSet<&str> = board.footprints.iter().map(|f| f.reference.as_str()).collect();
    let known_ref = |p: &str| if p.contains(['*', '?']) { board.footprints.iter().any(|f| crate::board::wildcard_match(p, &f.reference)) } else { refs.contains(p) };
    for (name, v) in &cfg.variants {
        r.entries += 1;
        let what = format!("variants.{name}");
        if name.trim().eq_ignore_ascii_case(DEFAULT) {
            r.errors.push(format!("{what}: \"{DEFAULT}\" is reserved (the board file as it is)"));
        }
        if v.is_empty() && !board.variants.iter().any(|b| &b.name == name) {
            r.warnings.push(format!("{what}: defines nothing (fitted_sheets, not_fitted_sheets, fitted, not_fitted or bom)"));
        }
        for s in v.fitted_sheets.iter().chain(&v.not_fitted_sheets) {
            if sheets.is_empty() {
                r.errors.push(format!("{what}: sheet '{s}' – the board file has no sheet names (re-save it with KiCad 6 or later, or select by references)"));
            } else if !board.footprints.iter().any(|f| in_sheet(&f.sheet, s)) {
                r.errors.push(format!("{what}: no part on sheet '{s}' (sheets: {}{})", sheets.iter().take(12).cloned().collect::<Vec<_>>().join(", "), if sheets.len() > 12 { ", …" } else { "" }));
            }
        }
        for p in v.fitted.iter().chain(&v.not_fitted).chain(&v.bom_exceptions) {
            if !known_ref(p.trim()) {
                r.errors.push(format!("{what}: no footprint '{p}' on the board"));
            }
        }
        for p in &v.fitted {
            if v.not_fitted.contains(p) {
                r.errors.push(format!("{what}: '{p}' is listed as fitted and as not fitted"));
            }
        }
        if !v.bom_exceptions.is_empty() && v.bom.is_none() {
            r.warnings.push(format!("{what}: bom_exceptions without a bom"));
        }
        if let Some(path) = &v.bom {
            match files.get(path) {
                None => r.errors.push(format!("{what}: BOM {path} not loaded (path relative to the kisi.json; in the viewer, drop it together with the board)")),
                Some(Err(e)) => r.errors.push(format!("{what}: BOM {path}: {e}")),
                Some(Ok(t)) => match bom_refs(t) {
                    Err(e) => r.errors.push(format!("{what}: BOM {path}: {e}")),
                    Ok(b) => {
                        let n = b.fitted.iter().filter(|x| !refs.contains(x.as_str())).count();
                        if n > 0 {
                            r.warnings.push(format!("{what}: {n} reference(s) of the BOM are not on the board"));
                        }
                    }
                },
            }
        }
        if v.source.is_none() && v.is_empty() {
            continue; // only a description of a KiCad 10 variant
        }
        match &v.source {
            None => r.errors.push(format!("{what}: source missing (e.g. kind \"design\" with the BOM or assembly files)")),
            Some(s) if s.is_empty() => r.errors.push(format!("{what}: source needs a document, url or note")),
            Some(s) if !SOURCE_KINDS.contains(&s.kind.as_str()) => r.errors.push(format!("{what}: source kind '{}' unknown (use {})", s.kind, SOURCE_KINDS.join(", "))),
            _ => {}
        }
    }
    let names: Vec<&str> = board.variants.iter().map(|v| v.name.as_str()).chain(cfg.variants.keys().map(|k| k.as_str())).collect();
    match &cfg.variant {
        Some(sel) if !sel.trim().eq_ignore_ascii_case(DEFAULT) && !names.iter().any(|n| n.eq_ignore_ascii_case(sel.trim())) => r.errors.push(format!(
            "variant: '{sel}' is not defined ({}\"{DEFAULT}\" = the board file as it is)",
            if names.is_empty() { String::new() } else { format!("variants: {}; ", names.join(", ")) }
        )),
        None if !names.is_empty() => r.warnings.push(format!("variant: none chosen (variants: {}) – commands that compute lanes stop and ask for one", names.join(", "))),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PCB: &str = r#"(kicad_pcb (version 20240108) (generator "pcbnew")
  (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
  (setup (stackup (layer "F.Cu" (type "copper") (thickness 0.035))
    (layer "dielectric 1" (type "core") (thickness 1.5) (material "FR4") (epsilon_r 4.5) (loss_tangent 0.02))
    (layer "B.Cu" (type "copper") (thickness 0.035))))
  (net 0 "") (net 1 "+3V3") (net 2 "GND")
  (footprint "R" (layer "F.Cu") (at 10 10) (property "Reference" "U1") (property "Value" "MCU")
    (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net 1 "+3V3"))))
"#;

    #[test]
    fn check_and_apply() {
        let board = crate::load_pcb(PCB).unwrap();
        let profiles = crate::profile::Profiles::builtin();
        let classes = vec!["Default".to_string(), "50Z".to_string()];
        let cfg = parse(
            r#"{"kisi_config": 1,
            "stackup": {"dielectrics": [{"index": 0, "epsilon_r": 4.2, "loss_tangent": 0.015, "source": {"kind": "datasheet", "document": "laminate X"}}]},
            "net_classes": {"50Z": {"target_ohm": 50, "diff": false, "profile": "se50", "source": {"kind": "assumption", "note": "class name"}}},
            "fields": [{"ref": "U1", "name": "PI_LOAD", "value": "+3V3=150mA", "source": {"kind": "datasheet", "url": "https://example.com/ds.pdf", "page": "12", "note": "typical"}}]}"#,
        )
        .unwrap();
        let r = check(&cfg, &board, &classes, &profiles, None, &Default::default());
        assert!(r.ok, "{:?}", r.errors);
        assert_eq!(r.entries, 3);
        let mut b = board.clone();
        let a = cfg.apply(&mut b);
        assert_eq!(b.footprints[0].prop("PI_LOAD"), Some("+3V3=150mA"));
        // the source travels with the field into the power tree
        b.footprints[0].props.push(("PI_REG".into(), "+3V3 input".into()));
        let (t, _) = crate::tree::tree_from_fields(&b).unwrap();
        assert_eq!((t.loads[0].basis.as_str(), t.loads[0].note.as_str()), ("datasheet", "typical"));
        assert!(t.name.contains("kisi.json"));
        assert!((b.stackup().dielectrics[0].loss_tangent - 0.015).abs() < 1e-12);
        assert_eq!(a.targets["50Z"].z, 50.0);
        assert_eq!(a.profiles["50Z"], "se50");

        let bad = parse(
            r#"{"kisi_config": 1,
            "stackup": {"dielectrics": [{"index": 3, "epsilon_r": 40}]},
            "net_classes": {"PCIe": {"profile": "pcie9", "source": {"kind": "guess", "note": "x"}}},
            "fields": [{"ref": "U9", "name": "PI_LOAD", "value": "+3V3=2A", "source": {"kind": "datasheet", "document": "d"}},
                       {"ref": "U1", "name": "PI_LOAD", "value": "+3V3=lots", "source": {"kind": "datasheet", "document": "d"}}]}"#,
        )
        .unwrap();
        let r = check(&bad, &board, &classes, &profiles, None, &Default::default());
        let named = BoardConfig { board: Some("mine.kicad_pcb".into()), ..cfg.clone() };
        assert!(check(&named, &board, &classes, &profiles, Some("other.kicad_pcb"), &Default::default()).errors[0].contains("written for"));
        assert!(check(&named, &board, &classes, &profiles, Some("/x/Mine.kicad_pcb"), &Default::default()).ok);
        assert!(same_board("x/LibreVNA.kicad_pcb", "librevna"));
        assert!(!r.ok);
        let all = r.errors.join("\n");
        for frag in ["1 dielectrics", "epsilon_r outside", "source missing", "no net class 'PCIe'", "unknown profile 'pcie9'", "kind 'guess'", "no footprint U9", "not readable"] {
            assert!(all.contains(frag), "missing '{frag}' in\n{all}");
        }
    }

    #[test]
    fn connector_ends() {
        let board = crate::load_pcb(PCB).unwrap();
        let profiles = crate::profile::Profiles::builtin();
        let classes = vec!["Default".to_string()];
        let ok = parse(r#"{"kisi_config": 1, "connector_ends": {"U1": {"model": "matched", "rest_loss_db": 3, "source": {"kind": "assumption", "note": "slot"}}}}"#).unwrap();
        let r = check(&ok, &board, &classes, &profiles, None, &Default::default());
        assert!(r.ok, "{:?}", r.errors);
        let e = &ok.connector_ends["U1"].end;
        assert!(e.model == "matched" && e.has_rest());
        let bad = parse(r#"{"kisi_config": 1, "connector_ends": {"J9": {"model": "custom"}, "U1": {"model": "lossy", "rest_loss_db": 300}}}"#).unwrap();
        let all = check(&bad, &board, &classes, &profiles, None, &Default::default()).errors.join("\n");
        for frag in ["no part 'J9'", "needs r_ohm", "unknown model 'lossy'", "rest_loss_db 0 … 100", "source missing"] {
            assert!(all.contains(frag), "missing '{frag}' in\n{all}");
        }
        // a Touchstone file behind the connector: checked when loaded, an error when not
        let ts = parse(r#"{"kisi_config": 1, "connector_ends": {"U1": {"touchstone": {"file": "cab/c.s2p"}, "source": {"kind": "measurement", "note": "VNA"}}}}"#).unwrap();
        assert!(check(&ts, &board, &classes, &profiles, None, &Default::default()).errors[0].contains("cab/c.s2p not loaded"));
        let files: crate::variant::Files = HashMap::from([("cab/c.s2p".to_string(), Ok("# GHz S MA R 50\n1 0 0 0.9 -10 0.9 -10 0 0\n2 0 0 0.8 -20 0.8 -20 0 0\n".to_string()))]);
        let r = check(&ts, &board, &classes, &profiles, None, &files);
        assert!(r.ok, "{:?}", r.errors);
        let po = parse(r#"{"kisi_config": 1, "connector_ends": {"U1": {"touchstone": {"file": "cab/c.s2p", "port_order": "2-4"}, "source": {"kind": "measurement", "note": "VNA"}}}}"#).unwrap();
        assert!(check(&po, &board, &classes, &profiles, None, &files).errors.iter().any(|e| e.contains("unknown port_order")));
        assert!(crate::service::touchstones_of(Some(&ts), &files)["cab/c.s2p"].as_ref().is_ok_and(|t| t.nports == 2 && t.name == "c.s2p"));
    }
}

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
//!   "notes": ["…"]
//! }
//! ```

use crate::board::{Board, DielectricOverride, StackupOverride};
use crate::extract::Target;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

pub const VERSION: u32 = 1;
/// kinds of sources; "estimate" and "assumption" are reported as such
pub const SOURCE_KINDS: &[&str] = &["datasheet", "standard", "application_note", "measurement", "simulation", "estimate", "assumption"];

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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct FieldEntry {
    #[serde(rename = "ref")]
    pub refdes: String,
    pub name: String,
    pub value: String,
    pub source: Option<Source>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct BoardConfig {
    pub kisi_config: u32,
    pub board: Option<String>,
    pub stackup: StackupEntry,
    pub net_classes: BTreeMap<String, ClassEntry>,
    pub fields: Vec<FieldEntry>,
    pub notes: Vec<String>,
}

/// Fields kisi evaluates (others are laid over too, but have no effect).
pub const KNOWN_FIELDS: &[&str] = &["PI_LOAD", "PI_SOURCE", "PI_REG"];

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
/// `board_name`: the loaded board (file name or viewer name), compared with `cfg.board`.
pub fn check(cfg: &BoardConfig, board: &Board, class_names: &[String], profiles: &crate::profile::Profiles, board_name: Option<&str>) -> CheckResult {
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
        src(what, &f.source, &mut r);
    }
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
        let r = check(&cfg, &board, &classes, &profiles, None);
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
        let r = check(&bad, &board, &classes, &profiles, None);
        let named = BoardConfig { board: Some("mine.kicad_pcb".into()), ..cfg.clone() };
        assert!(check(&named, &board, &classes, &profiles, Some("other.kicad_pcb")).errors[0].contains("written for"));
        assert!(check(&named, &board, &classes, &profiles, Some("/x/Mine.kicad_pcb")).ok);
        assert!(same_board("x/LibreVNA.kicad_pcb", "librevna"));
        assert!(!r.ok);
        let all = r.errors.join("\n");
        for frag in ["1 dielectrics", "epsilon_r outside", "source missing", "no net class 'PCIe'", "unknown profile 'pcie9'", "kind 'guess'", "no footprint U9", "not readable"] {
            assert!(all.contains(frag), "missing '{frag}' in\n{all}");
        }
    }
}

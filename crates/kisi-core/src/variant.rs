//! Assembly variants: which parts are fitted.
//!
//! A board can be built in several ways (ThunderScope Rev5.3: as a PCIe card or for the M.2 to
//! Thunderbolt adapter). Lanes depend on it: a part that is not fitted connects nothing, and the
//! copper up to its pads becomes a stub. Sources of variants:
//! * KiCad 10 board files: `(variants …)` and per-footprint overrides (`kisi_board::board`);
//! * `<board>.kisi.json` → `variants`: by sheet, by reference lists or from a BOM (boards from
//!   KiCad 8/9, or on top of a KiCad 10 variant of the same name).
//!
//! The variant is chosen in `<board>.kisi.json` (`"variant"`), on the command line (`--variant`)
//! or in the viewer; precedence board file < kisi.json < command line / viewer. "default" is the
//! board file as it is. When a board has variants – or forks where all alternative parts are
//! fitted (`topology::forks`) – and none is chosen, the commands that compute lanes stop and ask
//! for one (maintainer's decision, `docs/research/channel-simulation-ibis.md` section 8).

use crate::board::{wildcard_match, Board};
use crate::config::{BoardConfig, VariantEntry};
use crate::topology::{self, ComponentData, Fork};
use serde::Serialize;
use std::collections::{HashMap, HashSet};

/// The board file as it is (KiCad's base state, without variant overrides).
pub const DEFAULT: &str = "default";

/// Files a configuration refers to (BOMs), by the path written in it: contents or read error.
pub type Files = HashMap<String, Result<String, String>>;

#[derive(Debug, Clone, Serialize)]
pub struct VariantInfo {
    pub name: String,
    pub description: String,
    /// "board file", "kisi.json" or "board file, kisi.json"
    pub defined_in: String,
}

/// The variant in use and what it changed.
#[derive(Debug, Clone, Default, Serialize)]
pub struct VariantState {
    /// variants the board file and the configuration define
    pub variants: Vec<VariantInfo>,
    pub chosen: Option<String>,
    /// "command line", "viewer" or "kisi.json"
    pub chosen_by: Option<String>,
    pub description: Option<String>,
    /// parts not fitted in this variant although fitted in the board file, and the reverse
    pub not_fitted: Vec<String>,
    pub fitted: Vec<String>,
    /// parts not fitted in total (board file DNP and variant)
    pub not_fitted_total: usize,
    /// field values set by a KiCad 10 variant: "R1 Value = 22"
    pub fields: Vec<String>,
    /// forks with all alternative parts fitted (in the board as analysed)
    pub forks: Vec<Fork>,
    /// a variant must be chosen before lanes are computed
    pub needs_choice: bool,
    pub message: Option<String>,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

impl VariantState {
    /// Short form for the JSON results.
    pub fn summary(&self) -> serde_json::Value {
        let cap = |v: &[String]| -> serde_json::Value {
            if v.len() <= 40 {
                serde_json::json!(v)
            } else {
                serde_json::json!(v[..40].iter().cloned().chain(std::iter::once(format!("… {} more", v.len() - 40))).collect::<Vec<_>>())
            }
        };
        serde_json::json!({
            "name": self.chosen, "chosen_by": self.chosen_by, "description": self.description,
            "available": self.variants.iter().map(|v| &v.name).collect::<Vec<_>>(),
            "not_fitted_by_variant": cap(&self.not_fitted), "fitted_by_variant": cap(&self.fitted),
            "not_fitted_total": self.not_fitted_total, "field_overrides": self.fields,
            "forks_all_fitted": self.forks.len(), "needs_choice": self.needs_choice,
            "errors": self.errors, "warnings": self.warnings,
        })
    }
}

/// Variants of the board file and the configuration.
pub fn available(board: &Board, cfg: Option<&BoardConfig>) -> Vec<VariantInfo> {
    let mut out: Vec<VariantInfo> = board.variants.iter().map(|v| VariantInfo { name: v.name.clone(), description: v.description.clone(), defined_in: "board file".into() }).collect();
    if let Some(c) = cfg {
        for (name, e) in &c.variants {
            match out.iter_mut().find(|v| &v.name == name) {
                Some(v) => {
                    v.defined_in = "board file, kisi.json".into();
                    if let Some(d) = &e.description {
                        v.description = d.clone();
                    }
                }
                None => out.push(VariantInfo { name: name.clone(), description: e.description.clone().unwrap_or_default(), defined_in: "kisi.json".into() }),
            }
        }
    }
    out
}

/// "/Power" → "/Power/"; empty stays empty.
pub fn norm_sheet(s: &str) -> String {
    let t = s.trim();
    if t.is_empty() {
        return String::new();
    }
    let mut o = String::new();
    if !t.starts_with('/') {
        o.push('/');
    }
    o.push_str(t);
    if !t.ends_with('/') {
        o.push('/');
    }
    o
}

/// Whether a footprint's sheet lies in `sheet` (or one of its sub-sheets).
pub fn in_sheet(fp_sheet: &str, sheet: &str) -> bool {
    let (f, s) = (norm_sheet(fp_sheet), norm_sheet(sheet));
    !f.is_empty() && !s.is_empty() && f.starts_with(&s)
}

fn ref_match(pattern: &str, refdes: &str) -> bool {
    let p = pattern.trim();
    if p.contains(['*', '?']) {
        wildcard_match(p, refdes)
    } else {
        p == refdes
    }
}

/// References of a BOM (CSV).
#[derive(Debug, Clone, Default)]
pub struct BomRefs {
    pub fitted: HashSet<String>,
    /// rows marked not fitted (a DNP / "Fitted" column, or the value "DNP", "DNF")
    pub not_fitted: HashSet<String>,
}

/// One CSV line split into cells (quotes, doubled quotes inside quotes).
fn csv_cells(line: &str, delim: char) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if quoted && chars.peek() == Some(&'"') => {
                cur.push('"');
                chars.next();
            }
            '"' => quoted = !quoted,
            c if c == delim && !quoted => out.push(std::mem::take(&mut cur)),
            c => cur.push(c),
        }
    }
    out.push(cur);
    out.into_iter().map(|c| c.trim().to_string()).collect()
}

/// "R1-R4" → R1, R2, R3, R4; anything else as it is.
fn expand_range(tok: &str) -> Vec<String> {
    let split = |s: &str| -> Option<(String, u32)> {
        let p: String = s.chars().take_while(|c| !c.is_ascii_digit()).collect();
        let n: u32 = s[p.len()..].parse().ok()?;
        Some((p, n))
    };
    if let Some((a, b)) = tok.split_once('-') {
        if let (Some((pa, na)), Some((pb, nb))) = (split(a), split(b)) {
            if !pa.is_empty() && (pb.is_empty() || pb == pa) && nb >= na && nb - na < 1000 {
                return (na..=nb).map(|n| format!("{pa}{n}")).collect();
            }
        }
    }
    vec![tok.to_string()]
}

/// Read the references of a BOM: a CSV file (separator `,`, `;` or tab) with a column
/// "Reference(s)" / "Designator(s)" / "Ref" whose cells list references separated by commas
/// or spaces ("R1-R4" ranges allowed). Rows marked not fitted (a column "DNP", "Do not
/// populate", "Fitted"/"Populate" = no, or the value "DNP"/"DNF") count as not fitted.
pub fn bom_refs(text: &str) -> Result<BomRefs, String> {
    let lines: Vec<&str> = text.lines().map(|l| l.trim_start_matches('\u{feff}')).filter(|l| !l.trim().is_empty()).collect();
    let first = lines.first().ok_or("empty file")?;
    let delim = [',', ';', '\t'].into_iter().max_by_key(|d| first.matches(*d).count()).unwrap_or(',');
    let norm = |s: &str| s.trim().to_ascii_lowercase();
    const REF: &[&str] = &["reference", "references", "ref", "refs", "refdes", "designator", "designators", "reference designator", "reference designators", "ref des"];
    let (hi, header) = lines
        .iter()
        .take(20)
        .enumerate()
        .map(|(i, l)| (i, csv_cells(l, delim)))
        .find(|(_, cells)| cells.iter().any(|c| REF.contains(&norm(c).as_str())))
        .ok_or("no column with the references (\"Reference\", \"Designator\" or \"Ref\") in the first 20 lines")?;
    let col = |names: &[&str]| header.iter().position(|c| names.contains(&norm(c).as_str()));
    let rc = col(REF).unwrap();
    let dnp_col = col(&["dnp", "do not populate", "do not place", "not fitted", "dnf", "nofit"]);
    let fit_col = col(&["fitted", "populate", "populated", "mount", "mounted", "place", "fit"]);
    let val_col = col(&["value", "val", "comment"]);
    let yes = |s: &str| matches!(norm(s).as_str(), "x" | "yes" | "y" | "1" | "true" | "dnp" | "dnf" | "not fitted");
    let no = |s: &str| matches!(norm(s).as_str(), "no" | "n" | "0" | "false" | "dnp" | "dnf" | "not fitted" | "do not populate");
    let mut out = BomRefs::default();
    for l in &lines[hi + 1..] {
        let cells = csv_cells(l, delim);
        let Some(refs) = cells.get(rc) else { continue };
        let cell = |c: Option<usize>| c.and_then(|i| cells.get(i)).map(String::as_str).unwrap_or("");
        let not_fitted = (dnp_col.is_some() && yes(cell(dnp_col)))
            || (fit_col.is_some() && no(cell(fit_col)))
            || matches!(norm(cell(val_col)).as_str(), "dnp" | "dnf" | "nf" | "not fitted" | "do not populate");
        for tok in refs.split([',', ' ', ';']).map(str::trim).filter(|t| !t.is_empty()) {
            for r in expand_range(tok) {
                if not_fitted {
                    out.not_fitted.insert(r);
                } else {
                    out.fitted.insert(r);
                }
            }
        }
    }
    if out.fitted.is_empty() && out.not_fitted.is_empty() {
        return Err("no references found".into());
    }
    Ok(out)
}

/// Apply the KiCad 10 overrides of variant `name` (exact name).
fn apply_board_variant(board: &mut Board, name: &str, st: &mut VariantState) {
    for fp in &mut board.footprints {
        let Some(v) = fp.variants.iter().find(|v| v.name == name).cloned() else { continue };
        if let Some(d) = v.dnp {
            fp.dnp = d;
        }
        if let Some(e) = v.exclude_from_bom {
            fp.exclude_from_bom = e;
        }
        for (k, val) in &v.fields {
            fp.set_prop(k, val);
            st.fields.push(format!("{} {k} = {val}", fp.reference));
        }
    }
}

/// Apply a kisi.json variant definition: BOM, then sheets (the most specific sheet wins), then
/// the reference lists.
fn apply_entry(board: &mut Board, e: &VariantEntry, files: &Files, st: &mut VariantState) {
    if let Some(path) = &e.bom {
        match files.get(path) {
            None => st.errors.push(format!("BOM {path} not loaded (give it next to the kisi.json; in the viewer, drop it together with the board)")),
            Some(Err(err)) => st.errors.push(format!("BOM {path}: {err}")),
            Some(Ok(text)) => match bom_refs(text) {
                Err(err) => st.errors.push(format!("BOM {path}: {err}")),
                Ok(b) => {
                    let on_board: HashSet<&str> = board.footprints.iter().map(|f| f.reference.as_str()).collect();
                    let missing: Vec<&String> = b.fitted.iter().filter(|r| !on_board.contains(r.as_str())).collect();
                    if !missing.is_empty() {
                        let mut m: Vec<&str> = missing.iter().map(|s| s.as_str()).collect();
                        m.sort_unstable();
                        st.warnings.push(format!("BOM {path}: {} reference(s) not on the board ({}{})", m.len(), m.iter().take(8).cloned().collect::<Vec<_>>().join(", "), if m.len() > 8 { ", …" } else { "" }));
                    }
                    for fp in &mut board.footprints {
                        if fp.exclude_from_bom || e.bom_exceptions.iter().any(|p| ref_match(p, &fp.reference)) {
                            continue;
                        }
                        fp.dnp = !b.fitted.contains(&fp.reference) || b.not_fitted.contains(&fp.reference);
                    }
                }
            },
        }
    }
    if !e.fitted_sheets.is_empty() || !e.not_fitted_sheets.is_empty() {
        for fp in &mut board.footprints {
            let best = e
                .fitted_sheets
                .iter()
                .map(|s| (s, false))
                .chain(e.not_fitted_sheets.iter().map(|s| (s, true)))
                .filter(|(s, _)| in_sheet(&fp.sheet, s))
                .max_by_key(|(s, _)| norm_sheet(s).len());
            if let Some((_, dnp)) = best {
                fp.dnp = dnp;
            }
        }
    }
    for fp in &mut board.footprints {
        if e.fitted.iter().any(|p| ref_match(p, &fp.reference)) {
            fp.dnp = false;
        }
        if e.not_fitted.iter().any(|p| ref_match(p, &fp.reference)) {
            fp.dnp = true;
        }
    }
}

/// Apply the chosen variant to the board (`choice` = (name, chosen by) from the command line or
/// the viewer; else the configuration's `variant`) and find forks with all alternatives fitted.
pub fn apply(board: &mut Board, cfg: Option<&BoardConfig>, choice: Option<(&str, &str)>, files: &Files) -> VariantState {
    let mut st = VariantState { variants: available(board, cfg), ..Default::default() };
    let sel = choice.map(|(n, by)| (n.to_string(), by.to_string())).or_else(|| cfg.and_then(|c| c.variant.clone()).map(|v| (v, "kisi.json".to_string())));
    let before: Vec<bool> = board.footprints.iter().map(|f| f.dnp).collect();
    if let Some((name, by)) = sel.filter(|(n, _)| !n.trim().is_empty()) {
        let name = name.trim();
        let canon = if name.eq_ignore_ascii_case(DEFAULT) {
            Some(DEFAULT.to_string())
        } else {
            st.variants.iter().find(|v| v.name == name).or_else(|| st.variants.iter().find(|v| v.name.eq_ignore_ascii_case(name))).map(|v| v.name.clone())
        };
        match canon {
            None => st.errors.push(format!(
                "unknown assembly variant '{name}' ({by}); {}",
                if st.variants.is_empty() { "the board defines none (\"default\" = the board file as it is)".to_string() } else { format!("variants: {}, or \"default\" (the board file as it is)", st.variants.iter().map(|v| v.name.as_str()).collect::<Vec<_>>().join(", ")) }
            )),
            Some(c) => {
                if c != DEFAULT {
                    apply_board_variant(board, &c, &mut st);
                    if let Some(e) = cfg.and_then(|cf| cf.variants.get(&c)) {
                        apply_entry(board, e, files, &mut st);
                    }
                    st.description = st.variants.iter().find(|v| v.name == c).map(|v| v.description.clone()).filter(|d| !d.is_empty());
                } else {
                    st.description = Some("the board file as it is".into());
                }
                st.chosen = Some(c);
                st.chosen_by = Some(by);
            }
        }
    }
    for (fp, was) in board.footprints.iter().zip(&before) {
        if fp.dnp && !was {
            st.not_fitted.push(fp.reference.clone());
        } else if !fp.dnp && *was {
            st.fitted.push(fp.reference.clone());
        }
    }
    let natural = |a: &String, b: &String| topology::natural_cmp(a, b);
    st.not_fitted.sort_by(natural);
    st.fitted.sort_by(natural);
    st.not_fitted_total = board.footprints.iter().filter(|f| f.dnp).count();
    st.forks = topology::forks(board, &ComponentData::builtin());
    st.needs_choice = st.chosen.is_none() && (!st.variants.is_empty() || !st.forks.is_empty());
    if st.needs_choice {
        st.message = Some(message(&st));
    }
    st
}

/// Forks in a few words: "C41/C43 on /FPGA/MGT_TX3_P (→ P1 / J8)".
pub fn fork_text(f: &Fork) -> String {
    let short = |n: &str| n.rsplit('/').next().unwrap_or(n).to_string();
    format!("{} on {} (→ {})", f.parts.join("/"), short(&f.net_name), f.towards.iter().map(|t| t.join(", ")).collect::<Vec<_>>().join(" / "))
}

/// Why a variant must be chosen and how.
pub fn message(st: &VariantState) -> String {
    let how = "choose one with --variant NAME or \"variant\": \"NAME\" in <board>.kisi.json (\"default\" = the board file as it is)";
    let mut m = if !st.variants.is_empty() {
        let list: Vec<String> = st.variants.iter().map(|v| if v.description.is_empty() { v.name.clone() } else { format!("{} ({})", v.name, v.description) }).collect();
        format!("This board has assembly variants and none is chosen: {}. Lanes depend on which parts are fitted: {how}.", list.join(", "))
    } else {
        let list: Vec<String> = st.forks.iter().take(4).map(fork_text).collect();
        format!(
            "kisi found {} fork(s) where all alternative series parts are fitted – possibly assembly variants: {}{}. A lane would carry the unused path as an open branch. Define the variants in <board>.kisi.json (\"variants\": by sheet, references or BOM) and choose one (--variant NAME or \"variant\": \"NAME\"), mark the unused parts DNP in KiCad, or compute the board as it is with --variant default.",
            st.forks.len(),
            list.join("; "),
            if st.forks.len() > 4 { "; …" } else { "" }
        )
    };
    if !st.errors.is_empty() {
        m = format!("{} {m}", st.errors.join(" "));
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bom_and_sheets() {
        let csv = "\u{feff}\"Line Item\",\"Qty\",\"Value\",\"Reference\"\n\"1\",\"3\",\"1uF\",\"C1,C2,C5\"\n\"2\",\"1\",\"DNP\",\"R9\"\n\"3\",\"4\",\"0\",\"R1-R4\"\n";
        let b = bom_refs(csv).unwrap();
        for r in ["C1", "C2", "C5", "R1", "R3", "R4"] {
            assert!(b.fitted.contains(r), "{r}");
        }
        assert!(b.not_fitted.contains("R9") && !b.fitted.contains("C3"));
        let semi = "Designator;Comment;Fitted\nU1 U2;MCU;yes\nJ1;conn;no\n";
        let b = bom_refs(semi).unwrap();
        assert!(b.fitted.contains("U2") && b.not_fitted.contains("J1"));
        assert!(bom_refs("a,b\n1,2\n").is_err());
        assert!(in_sheet("/TS-PCIe Components/", "/TS-PCIe Components") && in_sheet("/FPGA/IO/", "/FPGA/") && !in_sheet("", "/") && !in_sheet("/FPGA2/", "/FPGA/"));
        assert_eq!(expand_range("C10-C12"), vec!["C10", "C11", "C12"]);
    }

    /// Two options on sheets /PCIe/ and /USB/, a shared capacitor on /USB/, edge fingers P1
    /// (excluded from the BOM) and one part outside the option sheets.
    const PCB: &str = r#"(kicad_pcb (version 20241229) (generator "pcbnew")
  (layers (0 "F.Cu" signal) (2 "B.Cu" signal))
  (net 0 "") (net 1 "A") (net 2 "B") (net 3 "C")
  (footprint "R" (layer "F.Cu") (at 0 0) (property "Reference" "R1") (property "Value" "0") (sheetname "/PCIe/") (attr smd)
    (pad "1" smd rect (at -0.5 0) (size 0.6 0.6) (layers "F.Cu") (net 1 "A")) (pad "2" smd rect (at 0.5 0) (size 0.6 0.6) (layers "F.Cu") (net 2 "B")))
  (footprint "R" (layer "B.Cu") (at 0 0) (property "Reference" "R2") (property "Value" "0") (sheetname "/USB/") (attr smd)
    (pad "1" smd rect (at -0.5 0) (size 0.6 0.6) (layers "B.Cu") (net 1 "A")) (pad "2" smd rect (at 0.5 0) (size 0.6 0.6) (layers "B.Cu") (net 3 "C")))
  (footprint "C" (layer "F.Cu") (at 5 0) (property "Reference" "C9") (property "Value" "100n") (sheetname "/USB/Power/") (attr smd)
    (pad "1" smd rect (at -0.5 0) (size 0.6 0.6) (layers "F.Cu") (net 2 "B")))
  (footprint "P" (layer "F.Cu") (at 9 0) (property "Reference" "P1") (property "Value" "edge") (sheetname "/PCIe/") (attr smd exclude_from_bom)
    (pad "1" smd rect (at 0 0) (size 0.6 3) (layers "F.Cu") (net 2 "B")))
  (footprint "U" (layer "F.Cu") (at -9 0) (property "Reference" "U1") (property "Value" "FPGA") (sheetname "/") (attr smd)
    (pad "1" smd rect (at 0 0) (size 0.6 0.6) (layers "F.Cu") (net 1 "A"))))
"#;

    fn project(cfg: Option<&str>, variant: Option<&str>, files: Files) -> crate::service::Project {
        crate::service::Project::from_texts(crate::service::Inputs {
            pcb_path: "b.kicad_pcb".into(),
            pcb: PCB.into(),
            pro: None,
            config: cfg.map(|c| ("b.kisi.json".into(), Ok(c.to_string()))),
            stackup: None,
            field: HashMap::new(),
            variant: variant.map(str::to_string),
            files,
        })
        .unwrap()
    }
    fn dnp(p: &crate::service::Project, r: &str) -> bool {
        p.board.footprints.iter().find(|f| f.reference == r).unwrap().dnp
    }

    #[test]
    fn variants_by_sheet_refs_and_bom() {
        let cfg = r#"{"kisi_config": 1, "variant": "PCIe",
          "variants": {
            "PCIe": {"not_fitted_sheets": ["/USB/"], "fitted_sheets": ["/USB/Power"], "source": {"kind": "design", "document": "BOM"}},
            "USB": {"not_fitted": ["R1", "P*"], "source": {"kind": "design", "document": "BOM"}},
            "USB-BOM": {"bom": "usb.csv", "source": {"kind": "design", "document": "usb.csv"}}}}"#;
        let files: Files = HashMap::from([("usb.csv".to_string(), Ok("Reference,Value\nR2,0\nU1,FPGA\n".to_string()))]);
        // kisi.json chooses PCIe: R2 (sheet /USB/) out, C9 (sub-sheet /USB/Power/, more specific) stays
        let p = project(Some(cfg), None, files.clone());
        assert_eq!((p.variant.chosen.as_deref(), p.variant.chosen_by.as_deref()), (Some("PCIe"), Some("kisi.json")));
        assert!(!dnp(&p, "R1") && dnp(&p, "R2") && !dnp(&p, "C9") && !dnp(&p, "P1"));
        assert_eq!(p.variant.not_fitted, vec!["R2"]);
        assert!(p.config_check.as_ref().unwrap().ok, "{:?}", p.config_check);
        assert!(p.require_variant().is_ok() && p.variant.forks.is_empty());
        // the command line wins: references with a wildcard
        let p = project(Some(cfg), Some("usb"), files.clone());
        assert_eq!(p.variant.chosen.as_deref(), Some("USB"));
        assert!(dnp(&p, "R1") && !dnp(&p, "R2") && dnp(&p, "P1"));
        // a BOM: parts not in it are not fitted, parts excluded from BOMs stay as in the board
        let p = project(Some(cfg), Some("USB-BOM"), files.clone());
        assert!(dnp(&p, "R1") && !dnp(&p, "R2") && dnp(&p, "C9") && !dnp(&p, "P1") && !dnp(&p, "U1"));
        // a BOM that is not loaded: an error in config-check and in the variant
        let p = project(Some(cfg), Some("USB-BOM"), Files::new());
        assert!(p.variant.errors[0].contains("usb.csv not loaded") && !p.config_check.as_ref().unwrap().ok);
        // unknown names
        let p = project(Some(cfg), Some("Lite"), files.clone());
        assert!(p.variant.chosen.is_none() && p.variant.errors[0].contains("unknown assembly variant 'Lite'"));
        let bad = cfg.replace("\"variant\": \"PCIe\"", "\"variant\": \"Lite\"");
        assert!(project(Some(&bad), None, files.clone()).config_check.unwrap().errors.iter().any(|e| e.contains("'Lite' is not defined")));
    }

    #[test]
    fn must_choose() {
        // variants defined, none chosen: lane commands stop and list them
        let cfg = r#"{"kisi_config": 1, "variants": {"PCIe": {"not_fitted_sheets": ["/USB/"], "source": {"kind": "design", "note": "x"}}, "USB": {"not_fitted_sheets": ["/PCIe/"], "source": {"kind": "design", "note": "x"}}}}"#;
        let p = project(Some(cfg), None, Files::new());
        assert!(p.variant.needs_choice);
        let e = p.require_variant().unwrap_err();
        assert!(e.contains("PCIe") && e.contains("USB") && e.contains("--variant"), "{e}");
        assert!(p.config_check.as_ref().unwrap().warnings.iter().any(|w| w.contains("none chosen")));
        let o = crate::commands::parse_args(&["check".to_string(), "b.kicad_pcb".to_string()]).unwrap();
        assert!(crate::commands::run(&o, Some(&p), None).is_err());
        let o = crate::commands::parse_args(&["overview".to_string(), "b.kicad_pcb".to_string()]).unwrap();
        assert!(crate::commands::run(&o, Some(&p), None).is_ok(), "overview does not depend on the parts");
        let gaps = crate::commands::run(&crate::commands::parse_args(&["gaps".to_string(), "b.kicad_pcb".to_string()]).unwrap(), Some(&p), None).unwrap();
        assert!(gaps["gaps"].as_array().unwrap().iter().any(|g| g["id"] == "assembly-variant"));
        // "default" is an informed choice: the board file as it is
        let p = project(Some(cfg), Some("default"), Files::new());
        assert!(!p.variant.needs_choice && p.require_variant().is_ok() && !dnp(&p, "R2"));
        // forks with all alternatives fitted (R1/R2 back to back, 0 Ω, to different nets) – here
        // without devices behind them, so no fork; a board without variants needs no choice
        let p = project(None, None, Files::new());
        assert!(p.variant.forks.is_empty() && !p.variant.needs_choice);
    }

    #[test]
    fn kicad10_variant_overrides() {
        let pcb = PCB.replace("(net 0 \"\")", "(variants (variant (name \"Lite\") (description \"no USB\"))) (net 0 \"\")").replace(
            "(net 3 \"C\")))\n  (footprint \"C\"",
            "(net 3 \"C\")) (variant (name \"Lite\") (dnp yes) (field (name \"Value\") (value \"DNP\"))))\n  (footprint \"C\"",
        );
        let mut b = crate::load_pcb(&pcb).unwrap();
        assert_eq!(b.variants.len(), 1);
        let st = apply(&mut b.clone(), None, None, &Files::new());
        assert!(st.needs_choice && st.message.as_deref().unwrap().contains("Lite (no USB)"));
        let st = apply(&mut b, None, Some(("Lite", "command line")), &Files::new());
        let r2 = b.footprints.iter().find(|f| f.reference == "R2").unwrap();
        assert!(r2.dnp && r2.value == "DNP" && st.not_fitted == vec!["R2"] && st.fields == vec!["R2 Value = DNP"]);
    }
}

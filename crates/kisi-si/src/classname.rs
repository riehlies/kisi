//! Target impedance from a net class name (`50Z`, `SE50`, `Diff_100`, `SE55DE100`, `ZDIFF=100`, …).
//! The naming conventions live in `data/netclass_names.json`; the rules are described there and in
//! `docs/reference.md`.

use crate::extract::Target;
use serde::Deserialize;
use std::sync::OnceLock;

pub const NETCLASS_NAMES_JSON: &str = include_str!("../../../data/netclass_names.json");

#[derive(Debug, Deserialize)]
struct Range {
    min: f64,
    max: f64,
}

#[derive(Debug, Deserialize)]
struct Kinds {
    single: Vec<String>,
    diff: Vec<String>,
    #[serde(default)]
    plain: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct Hint {
    min_ohm: f64,
    keywords: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct Interface {
    keyword: String,
    z: f64,
    diff: bool,
}

#[derive(Debug, Deserialize)]
struct Rules {
    range_ohm: Range,
    units: Vec<String>,
    prefixes: Kinds,
    kind_words: Kinds,
    diff_keywords: Vec<String>,
    diff_hint: Hint,
    interfaces: Vec<Interface>,
}

fn rules() -> &'static Rules {
    static R: OnceLock<Rules> = OnceLock::new();
    R.get_or_init(|| serde_json::from_str(NETCLASS_NAMES_JSON).expect("data/netclass_names.json"))
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Single,
    Diff,
    /// a unit or a plain prefix: the rest of the name decides
    Open,
}

#[derive(Debug)]
enum Tok {
    Word(String),
    Num(f64),
    Sep(char),
}

fn tokens(up: &str) -> Vec<Tok> {
    let mut out = Vec::new();
    let cs: Vec<char> = up.chars().collect();
    let mut i = 0;
    while i < cs.len() {
        let c = cs[i];
        if c.is_ascii_digit() {
            let st = i;
            while i < cs.len() && (cs[i].is_ascii_digit() || cs[i] == '.') {
                i += 1;
            }
            out.push(Tok::Num(cs[st..i].iter().collect::<String>().trim_end_matches('.').parse().unwrap_or(f64::NAN)));
        } else if c.is_alphabetic() {
            let st = i;
            while i < cs.len() && cs[i].is_alphabetic() {
                i += 1;
            }
            out.push(Tok::Word(cs[st..i].iter().collect()));
        } else {
            out.push(Tok::Sep(c));
            i += 1;
        }
    }
    out
}

/// Whether `word` carries `prefix` at its end: short prefixes (up to two letters) must be the
/// whole word, longer ones may end it.
fn ends_with_prefix(word: &str, prefix: &str) -> bool {
    if prefix.chars().count() <= 2 {
        word == prefix
    } else {
        word.ends_with(prefix)
    }
}

fn diff_keyword(up: &str, k: &str) -> bool {
    if let Some(k) = k.strip_prefix('^') {
        up.starts_with(k)
    } else if let Some(k) = k.strip_suffix('$') {
        up.ends_with(k)
    } else {
        up.contains(k)
    }
}

/// Derive a target impedance from a net class name. Returns the target and its source: "class
/// name" (with a note for combined names such as `SE55DE100`) or "interface assumption" (a keyword
/// without a number). None for names without a target and for ambiguous ones (`SE50_SE60`).
pub fn target_from_class_name(name: &str) -> Option<(Target, String)> {
    let r = rules();
    let up = name.to_uppercase();
    let toks = tokens(&up);
    let sep_ok = |t: Option<&Tok>| matches!(t, Some(Tok::Sep('_' | '-' | '=' | ' ')));
    // the word right before / after token i, across at most one separator
    let word_before = |i: usize| -> Option<&str> {
        match i.checked_sub(1).map(|j| &toks[j]) {
            Some(Tok::Word(w)) => Some(w.as_str()),
            t if sep_ok(t) => match i.checked_sub(2).map(|j| &toks[j]) {
                Some(Tok::Word(w)) => Some(w.as_str()),
                _ => None,
            },
            _ => None,
        }
    };
    let word_after = |i: usize, seps: &[char]| -> Option<&str> {
        match toks.get(i + 1) {
            Some(Tok::Word(w)) => Some(w.as_str()),
            Some(Tok::Sep(c)) if seps.contains(c) => match toks.get(i + 2) {
                Some(Tok::Word(w)) => Some(w.as_str()),
                _ => None,
            },
            _ => None,
        }
    };
    let mut found: Vec<(f64, Kind)> = Vec::new();
    for (i, t) in toks.iter().enumerate() {
        let Tok::Num(v) = *t else { continue };
        if !(r.range_ohm.min..=r.range_ohm.max).contains(&v) {
            continue;
        }
        let before = word_before(i);
        let has = |list: &[String]| before.is_some_and(|w| list.iter().any(|p| ends_with_prefix(w, p)));
        let kind = if has(&r.prefixes.single) {
            Some(Kind::Single)
        } else if has(&r.prefixes.diff) {
            Some(Kind::Diff)
        } else if has(&r.prefixes.plain)
            || word_after(i, &['_', ' ']).is_some_and(|w| r.units.iter().any(|u| w.starts_with(u.as_str())))
        {
            Some(Kind::Open)
        } else {
            match word_after(i, &['_', '-', ' ']) {
                Some(w) if r.kind_words.single.iter().any(|k| k == w) => Some(Kind::Single),
                Some(w) if r.kind_words.diff.iter().any(|k| k == w) => Some(Kind::Diff),
                _ => None,
            }
        };
        if let Some(k) = kind {
            if !found.contains(&(v, k)) {
                found.push((v, k));
            }
        }
    }
    let diff_kw = r.diff_keywords.iter().any(|k| diff_keyword(&up, k));
    let resolve = |v: f64, k: Kind| match k {
        Kind::Single => false,
        Kind::Diff => true,
        Kind::Open => {
            diff_kw || v >= r.diff_hint.min_ohm && r.diff_hint.keywords.iter().any(|k| up.contains(k.as_str()))
        }
    };
    let mut single: Vec<f64> = found.iter().filter(|(v, k)| !resolve(*v, *k)).map(|x| x.0).collect();
    let mut diff: Vec<f64> = found.iter().filter(|(v, k)| resolve(*v, *k)).map(|x| x.0).collect();
    for v in [&mut single, &mut diff] {
        v.sort_by(f64::total_cmp);
        v.dedup();
    }
    match (single.as_slice(), diff.as_slice()) {
        ([], []) => {}
        ([z], []) => return Some((Target { z: *z, diff: false }, "class name".into())),
        ([], [z]) => return Some((Target { z: *z, diff: true }, "class name".into())),
        ([s], [d]) => {
            return Some((
                Target { z: *d, diff: true },
                format!("class name ({d} Ω differential; its single-ended {s} Ω is not checked)"),
            ))
        }
        // two values of one kind: which one is meant is not clear
        _ => return None,
    }
    for it in &r.interfaces {
        if up.split(|c: char| !c.is_ascii_alphanumeric() && c != '_').any(|tok| tok.contains(it.keyword.as_str())) {
            return Some((Target { z: it.z, diff: it.diff }, "interface assumption".into()));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn class_names() {
        let t = |s| target_from_class_name(s).map(|x| (x.0.z, x.0.diff));
        let src = |s| target_from_class_name(s).map(|x| x.1).unwrap_or_default();
        // number + unit, Z + number (the forms before)
        assert_eq!(t("50Z"), Some((50.0, false)));
        assert_eq!(t("FE_100Z_Diff"), Some((100.0, true)));
        assert_eq!(t("FE_50Z_INPUT"), Some((50.0, false)));
        assert_eq!(t("USB_90R"), Some((90.0, true)));
        assert_eq!(t("Z85_PCIe"), Some((85.0, true)));
        assert_eq!(t("Diff100ohm"), Some((100.0, true)));
        assert_eq!(t("50_Ohm"), Some((50.0, false)));
        // prefixes of the kicad-happy corpus boards
        assert_eq!(t("Diff_100"), Some((100.0, true)));
        assert_eq!(t("Diff_90"), Some((90.0, true)));
        assert_eq!(t("SE_50"), Some((50.0, false)));
        assert_eq!(t("diff100"), Some((100.0, true)));
        assert_eq!(t("diff90"), Some((90.0, true)));
        assert_eq!(t("SE50"), Some((50.0, false)));
        assert_eq!(t("ZDIFF=100"), Some((100.0, true)));
        assert_eq!(t("ZSINGLE=50"), Some((50.0, false)));
        assert_eq!(t("S50"), Some((50.0, false)));
        assert_eq!(src("S50"), "class name");
        // single-ended and differential in one name: the differential target, the other named
        assert_eq!(t("SE55DE100"), Some((100.0, true)));
        assert_eq!(t("SE60DE100"), Some((100.0, true)));
        assert!(src("SE55DE100").contains("single-ended 55 Ω"), "{}", src("SE55DE100"));
        // kind word after the number
        assert_eq!(t("LVDS_100_DIFF"), Some((100.0, true)));
        assert_eq!(t("RF_50_SE"), Some((50.0, false)));
        // interface keywords without a number
        assert_eq!(t("LVDS_ADC"), Some((100.0, true)));
        assert_eq!(t("PCIe"), Some((85.0, true)));
        assert_eq!(src("PCIe"), "interface assumption");
        // no target: no impedance in the name, ambiguous, numbers that are no impedances
        assert_eq!(t("PWR"), None);
        assert_eq!(t("Default"), None);
        assert_eq!(t("3V3"), None);
        assert_eq!(t("SE50_SE60"), None);
        assert_eq!(t("Diff_90_Diff_100"), None);
        assert_eq!(t("BUS50"), None);
        assert_eq!(t("CLK_100M"), None);
        assert_eq!(t("DDR4_2400"), None);
        assert_eq!(t("W0.2_S0.15"), None);
        assert_eq!(t("Power_1000mA"), None);
        assert_eq!(t("MODE100"), None);
    }
}

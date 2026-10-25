//! Accuracy levels (ROADMAP block 2; design: docs/research/accuracy-levels.md): which level a result
//! used – formula, 2D field solver, or a model of its own (via ladder, coaxial fallback, the no-plane
//! estimate) – and how uncertain it is, with the numbers of kisi's validation from
//! `data/accuracy.json` (sources there).

use crate::extract::{CrossSection, Kind, F_FORMULA_ROUGH};
use serde::Deserialize;
use std::sync::OnceLock;

pub const ACCURACY_JSON: &str = include_str!("../../../data/accuracy.json");

#[derive(Debug, Clone, Deserialize)]
pub struct Impedance {
    pub field_pct: f64,
    pub formula_microstrip_pct: f64,
    pub formula_coupled_pct: f64,
    pub formula_stripline_pct: f64,
    pub formula_rough_pct: f64,
    pub no_plane_pct: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Vias {
    pub ladder_median_pct: f64,
    pub ladder_p90_pct: f64,
    pub pair_median_pct: f64,
    pub pair_p90_pct: f64,
    pub coax_median_pct: f64,
    pub coax_p90_pct: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Accuracy {
    pub impedance: Impedance,
    pub vias: Vias,
}

/// The built-in table (`data/accuracy.json`).
pub fn table() -> &'static Accuracy {
    static T: OnceLock<Accuracy> = OnceLock::new();
    T.get_or_init(|| serde_json::from_str(ACCURACY_JSON).expect("data/accuracy.json"))
}

/// Level and typical uncertainty (% of the value) of a cross-section's impedance: "field solver",
/// "formula" or "no plane" (the coplanar estimate); None without an impedance.
pub fn impedance_of(xs: &CrossSection) -> Option<(&'static str, f64)> {
    xs.z0?;
    let t = &table().impedance;
    Some(if xs.solver == "field" {
        ("field solver", t.field_pct)
    } else if xs.kind == Kind::NoReference {
        ("no plane", t.no_plane_pct)
    } else if xs.flags & F_FORMULA_ROUGH != 0 {
        ("formula", t.formula_rough_pct)
    } else if xs.kind == Kind::Stripline {
        ("formula", t.formula_stripline_pct)
    } else if xs.diff_gap.is_some() || xs.s_left.is_some() || xs.s_right.is_some() {
        ("formula", t.formula_coupled_pct)
    } else {
        ("formula", t.formula_microstrip_pct)
    })
}

/// The level of a set of cross-sections from the share of their length the field solver covers:
/// "field solver", "formula" or "mixed".
pub fn level_of(field_share: f64) -> &'static str {
    if field_share >= 0.999 {
        "field solver"
    } else if field_share <= 0.001 {
        "formula"
    } else {
        "mixed"
    }
}

/// The via models of a channel and their validated TDR error (median, 90th percentile, %):
/// `ladder` and `coax` counts, `pair`: the channel is a pair (its vias as via pairs).
pub fn vias_of(ladder: usize, coax: usize, pair: bool) -> Option<(&'static str, f64, f64)> {
    let t = &table().vias;
    match (ladder, coax) {
        (0, 0) => None,
        (_, 0) if pair => Some(("ladder (pair)", t.pair_median_pct, t.pair_p90_pct)),
        (_, 0) => Some(("ladder", t.ladder_median_pct, t.ladder_p90_pct)),
        (0, _) => Some(("coax", t.coax_median_pct, t.coax_p90_pct)),
        _ => Some(("mixed", t.coax_median_pct.max(t.ladder_median_pct), t.coax_p90_pct.max(t.ladder_p90_pct))),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn table_reads() {
        let t = super::table();
        assert!(t.impedance.field_pct > 0.0 && t.vias.coax_p90_pct > t.vias.ladder_p90_pct);
        assert_eq!(super::level_of(0.5), "mixed");
    }
}

//! Interface profiles (data/interfaces.json) and the judgement of a channel
//! against a profile's limits. Shared by the viewer (through WASM) and the CLI
//! so every front end shows the same verdict.

use crate::eye::EndModel;
use serde::{Deserialize, Serialize};

/// The profile file shipped with kisi (can be replaced at run time).
pub const BUILTIN_JSON: &str = include_str!("../../../data/interfaces.json");

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Limits {
    pub eye_h_red: Option<f64>,
    pub eye_h_yellow: Option<f64>,
    pub eye_w_min_ui: Option<f64>,
    pub skew_ps: Option<f64>,
    pub rl_min_db: Option<f64>,
    pub stub_factor: Option<f64>,
    pub il_board_db: Option<f64>,
}

impl Limits {
    /// `self` with every unset value taken from `base`
    pub fn or(&self, base: &Limits) -> Limits {
        Limits {
            eye_h_red: self.eye_h_red.or(base.eye_h_red),
            eye_h_yellow: self.eye_h_yellow.or(base.eye_h_yellow),
            eye_w_min_ui: self.eye_w_min_ui.or(base.eye_w_min_ui),
            skew_ps: self.skew_ps.or(base.skew_ps),
            rl_min_db: self.rl_min_db.or(base.rl_min_db),
            stub_factor: self.stub_factor.or(base.stub_factor),
            il_board_db: self.il_board_db.or(base.il_board_db),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Source {
    pub kind: String,
    pub text: String,
    pub url: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TxSpec {
    pub swing_mv: f64,
    pub r: f64,
    pub c_pf: f64,
    pub l_nh: f64,
    pub rise_ps: f64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub family: String,
    pub rate_gbps: f64,
    /// "diff" or "se"
    pub signal: String,
    pub z: f64,
    pub z_tol_pct: f64,
    pub tx: TxSpec,
    pub rx: EndModel,
    pub limits: Limits,
    #[serde(rename = "match")]
    pub match_: Vec<String>,
    pub sources: Vec<Source>,
    pub il_total: Option<serde_json::Value>,
}

impl Profile {
    pub fn tx_model(&self) -> EndModel {
        EndModel { r: self.tx.r, c_pf: self.tx.c_pf, l_nh: self.tx.l_nh }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Defaults {
    pub limits: Limits,
    pub sources: Vec<Source>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Profiles {
    pub version: u32,
    pub defaults: Defaults,
    pub profiles: Vec<Profile>,
}

impl Profiles {
    pub fn builtin() -> Profiles {
        serde_json::from_str(BUILTIN_JSON).expect("data/interfaces.json")
    }
    pub fn get(&self, id: &str) -> Option<&Profile> {
        self.profiles.iter().find(|p| p.id == id)
    }
    /// Effective limits of a profile (profile values over the defaults).
    pub fn limits(&self, p: &Profile) -> Limits {
        p.limits.or(&self.defaults.limits)
    }
    /// Automatic assignment: first profile whose rule matches the net name
    /// (without sheet path), then the class name; else generic by target.
    pub fn auto(&self, net_name: &str, class_name: &str, diff: bool) -> &str {
        let short = net_name.rsplit('/').next().unwrap_or(net_name);
        for s in [short, class_name] {
            for p in &self.profiles {
                for m in &p.match_ {
                    if regex_lite::RegexBuilder::new(m).case_insensitive(true).build().map(|re| re.is_match(s)).unwrap_or(false) {
                        return &p.id;
                    }
                }
            }
        }
        if diff {
            "diff100"
        } else {
            "se50"
        }
    }
}

/// Measured values of one channel (any field may be missing).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Metrics {
    pub z_mean: Option<f64>,
    pub z_min: Option<f64>,
    pub z_max: Option<f64>,
    pub il_nyq_db: Option<f64>,
    pub rl_db: Option<f64>,
    pub eye_h: Option<f64>,
    pub eye_w_ui: Option<f64>,
    pub skew_ps: Option<f64>,
    pub skew_uncertain: bool,
    pub stub_ghz: Option<f64>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Red,
    Yellow,
    Green,
}

/// One criterion: value with its limits (lo ≤ value ≤ hi; `red_below` for the eye).
#[derive(Debug, Clone, Serialize)]
pub struct Criterion {
    pub key: &'static str,
    pub status: Status,
    pub value: Option<f64>,
    pub lo: Option<f64>,
    pub hi: Option<f64>,
    pub red_below: Option<f64>,
    pub note: Option<&'static str>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Verdict {
    pub status: Status,
    pub criteria: Vec<Criterion>,
}

pub fn judge(m: &Metrics, p: &Profile, l: &Limits) -> Verdict {
    let nyq = p.rate_gbps / 2.0;
    let ui = 1000.0 / p.rate_gbps.max(1e-6);
    let mut c = Vec::new();
    let mut add = |key, status, value, lo, hi, red_below, note| c.push(Criterion { key, status, value, lo, hi, red_below, note });
    if m.error.is_some() {
        add("error", Status::Red, None, None, None, None, None);
    }
    if let Some(z) = m.z_mean {
        let (lo, hi) = (p.z * (1.0 - p.z_tol_pct / 100.0), p.z * (1.0 + p.z_tol_pct / 100.0));
        add("z", if z < lo || z > hi { Status::Yellow } else { Status::Green }, Some(z), Some(lo), Some(hi), None, Some("mean along the line from the impedance analysis"));
    }
    if let Some(h) = m.eye_h {
        let (red, yel) = (l.eye_h_red.unwrap_or(0.4), l.eye_h_yellow.unwrap_or(0.65));
        let st = if h < red { Status::Red } else if h < yel { Status::Yellow } else { Status::Green };
        add("eye_h", st, Some(h), Some(yel), None, Some(red), None);
    }
    if let Some(w) = m.eye_w_ui {
        let lo = l.eye_w_min_ui.unwrap_or(0.5);
        add("eye_w", if w < lo { Status::Yellow } else { Status::Green }, Some(w), Some(lo), None, None, None);
    }
    if let Some(il) = m.il_nyq_db {
        add("il", if l.il_board_db.is_some_and(|b| il < b) { Status::Yellow } else { Status::Green }, Some(il), l.il_board_db, None, None, None);
    }
    if let Some(rl) = m.rl_db {
        let lo = l.rl_min_db.unwrap_or(10.0);
        add("rl", if rl < lo { Status::Yellow } else { Status::Green }, Some(rl), Some(lo), None, None, None);
    }
    if let Some(s) = m.skew_ps {
        let hi = l.skew_ps.unwrap_or(0.05 * ui);
        let bad = !m.skew_uncertain && s.abs() > hi;
        add("skew", if bad { Status::Yellow } else { Status::Green }, Some(s.abs()), None, Some(hi), None, if m.skew_uncertain { Some("path branches or is split by components") } else { None });
    }
    if let Some(f) = m.stub_ghz {
        let lo = l.stub_factor.unwrap_or(2.0) * nyq;
        add("stub", if f < lo { Status::Yellow } else { Status::Green }, Some(f), Some(lo), None, None, None);
    }
    let status = c.iter().map(|x| x.status).min().unwrap_or(Status::Green);
    Verdict { status, criteria: c }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_profiles_load_and_match() {
        let ps = Profiles::builtin();
        assert!(ps.profiles.len() > 15);
        assert_eq!(ps.auto("/TS-PCIe Components/PCIe_PET0_P", "PCIe", true), "pcie4");
        assert_eq!(ps.auto("PCIe_REFCLK_P", "PCIe", true), "pcie_refclk");
        assert_eq!(ps.auto("/FPGA/USRIO_3_P", "LVDS_USRIO", true), "lvds");
        assert_eq!(ps.auto("FOO", "Default", false), "se50");
        let p = ps.get("pcie4").unwrap();
        let l = ps.limits(p);
        assert_eq!(l.skew_ps, Some(5.0));
        assert_eq!(l.eye_h_red, Some(0.4));
    }

    #[test]
    fn verdict() {
        let ps = Profiles::builtin();
        let p = ps.get("pcie4").unwrap();
        let l = ps.limits(p);
        let m = Metrics { eye_h: Some(0.3), z_mean: Some(86.0), ..Default::default() };
        assert_eq!(judge(&m, p, &l).status, Status::Red);
        let m = Metrics { eye_h: Some(0.8), z_mean: Some(86.0), skew_ps: Some(7.0), ..Default::default() };
        assert_eq!(judge(&m, p, &l).status, Status::Yellow);
        let m = Metrics { eye_h: Some(0.8), z_mean: Some(86.0), skew_ps: Some(7.0), skew_uncertain: true, ..Default::default() };
        assert_eq!(judge(&m, p, &l).status, Status::Green);
    }
}

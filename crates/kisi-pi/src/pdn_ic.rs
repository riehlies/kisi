//! PDN seen from every IC (PI-AC, second version).
//!
//! The rail copper on all layers is rasterised (as for the DC drop) into an AC grid: every
//! cell carries its capacitance to the nearest ground copper above and below, neighbouring
//! cells are joined by the plane-pair inductance μ0·d per square plus the sheet resistance of
//! rail and return, vias join the layers with their barrel inductance. Ground is the ideal
//! reference. Decoupling capacitors (ESR, ESL, ground-side mounting, DC-bias-derated C) and
//! the regulators (R + L) are shunts at their pads; every load IC is a port with its rail
//! pins shorted together (pin group).
//!
//! Results per IC: Z(f), the effective loop inductance of every capacitor seen from the IC
//! (path through the plane and vias + mounting + ESL) with a ranking, a target impedance
//! from the IC's own current step (or PI_TARGET), anti-resonances and recommendations
//! (value, package, count, placement radius). Design check, not a signoff PDN simulation.

use crate::board::{Board, Pad};
use crate::geom::{self, BBox, P};
use crate::num::{Cx, EPS0, MU0};
use crate::pdn::{parse_cap, PdnParams};
use crate::pi::{fill_polygon, fill_track, find_net, is_ground_name, pad_cells, Grid, RailSetup};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::f64::consts::PI;

const COMPONENTS_JSON: &str = include_str!("../../../data/components.json");
const RHO: f64 = 1.72e-8;
/// pin-group short and regulator connection (Ω)
const R_SHORT: f64 = 1e-4;

// --- data (data/components.json, "decoupling") ---------------------------------------------

#[derive(Debug, Clone, Deserialize)]
struct EsrModel {
    k_ohm: f64,
    c_ref_f: f64,
    exp: f64,
    min_ohm: f64,
    max_ohm: f64,
    min_bulk_ohm: f64,
}
#[derive(Debug, Clone, Deserialize)]
struct Bias {
    high_cv: Vec<(f64, f64)>,
    standard: Vec<(f64, f64)>,
    high_cv_from_f: BTreeMap<String, f64>,
}
#[derive(Debug, Clone, Deserialize)]
struct Rated {
    series: Vec<f64>,
    margin: f64,
}
#[derive(Debug, Clone, Deserialize)]
struct Cand {
    value: String,
    package: String,
}
#[derive(Debug, Clone, Deserialize)]
struct Mount {
    value: f64,
}
#[derive(Debug, Clone, Deserialize)]
pub struct DecapData {
    esl_nh: BTreeMap<String, f64>,
    esr: EsrModel,
    mount_nh: Mount,
    dc_bias: Bias,
    rated_v: Rated,
    candidates: Vec<Cand>,
}
#[derive(Debug, Clone, Deserialize)]
struct BeadData {
    z100_ohm: f64,
    rdc_ohm: f64,
    rp_factor: f64,
    cp_pf: f64,
}
/// Series parts in a supply path (data/components.json, "series").
#[derive(Debug, Clone, Deserialize)]
pub struct SeriesData {
    r_max_ohm: f64,
    zero_ohm: f64,
    max_nets: usize,
    bead: BeadData,
    inductor_dcr_ohm: BTreeMap<String, f64>,
}
#[derive(Deserialize)]
struct Wrap {
    decoupling: DecapData,
    series: SeriesData,
}

impl SeriesData {
    pub fn builtin() -> Self {
        serde_json::from_str::<Wrap>(COMPONENTS_JSON).expect("data/components.json: series").series
    }
}

fn interp(curve: &[(f64, f64)], x: f64) -> f64 {
    if curve.is_empty() {
        return 1.0;
    }
    if x <= curve[0].0 {
        return curve[0].1;
    }
    for w in curve.windows(2) {
        let ((x0, y0), (x1, y1)) = (w[0], w[1]);
        if x <= x1 {
            return y0 + (y1 - y0) * (x - x0) / (x1 - x0).max(1e-12);
        }
    }
    // beyond the rated voltage: continue the last slope, at least 10 %
    let n = curve.len();
    let ((x0, y0), (x1, y1)) = (curve[n - 2], curve[n - 1]);
    (y1 + (y1 - y0) / (x1 - x0).max(1e-12) * (x - x1)).max(0.1)
}

impl DecapData {
    pub fn builtin() -> Self {
        serde_json::from_str::<Wrap>(COMPONENTS_JSON).expect("data/components.json: decoupling").decoupling
    }
    fn esl(&self, pkg: &str) -> f64 {
        self.esl_nh.get(pkg).or_else(|| self.esl_nh.get("large")).copied().unwrap_or(1.5) * 1e-9
    }
    fn esr(&self, pkg: &str, c: f64, dielectric: &str) -> f64 {
        let e = &self.esr;
        let mut r = (e.k_ohm * (e.c_ref_f / c).powf(e.exp)).clamp(e.min_ohm, e.max_ohm);
        if is_bulk(dielectric) || (c >= 47e-6 && pkg == "large") {
            r = r.max(e.min_bulk_ohm);
        }
        r
    }
    fn class2(dielectric: &str, c: f64) -> bool {
        match dielectric {
            "" => c >= 1e-9, // unknown: class II from 1 nF on (assumption)
            d => matches!(d, "X5R" | "X7R" | "X6S" | "X7S" | "X8R" | "X7T" | "X6T" | "Y5V" | "Z5U"),
        }
    }
    /// C(V)/C(0) of a class II MLCC at `v` with rated voltage `vr`.
    fn retention(&self, pkg: &str, c: f64, v: f64, vr: f64, dielectric: &str) -> f64 {
        if !Self::class2(dielectric, c) || vr <= 0.0 {
            return 1.0;
        }
        let from = self.dc_bias.high_cv_from_f.get(pkg).or_else(|| self.dc_bias.high_cv_from_f.get("large")).copied().unwrap_or(1e-5);
        let curve = if c >= from * 0.999 { &self.dc_bias.high_cv } else { &self.dc_bias.standard };
        interp(curve, v.abs() / vr)
    }
    fn rated_assumed(&self, v: f64) -> f64 {
        let need = v.abs() * self.rated_v.margin;
        self.rated_v.series.iter().copied().find(|&r| r >= need).unwrap_or(*self.rated_v.series.last().unwrap_or(&100.0))
    }
}

fn is_bulk(d: &str) -> bool {
    matches!(d, "POLYMER" | "TANTALUM" | "ELECTROLYTIC")
}

// --- parsing ---------------------------------------------------------------------------------

/// Number with an SI prefix and an optional unit: "5m", "0.45nH", "6.3V", "2.2u", "50MHz", "10mOhm".
pub fn si(s: &str) -> Option<f64> {
    let t = s.trim().replace(',', ".").replace('µ', "u").replace('μ', "u");
    let lower = t.to_ascii_lowercase();
    let mut body = t.as_str();
    for unit in ["ohm", "Ω", "hz", "f", "h", "v", "a"] {
        if lower.ends_with(unit) && body.len() > unit.len() {
            let cut = body.len() - unit.len();
            if body.is_char_boundary(cut) {
                body = &body[..cut];
            }
            break;
        }
    }
    let body = body.trim();
    let (num, mult) = match body.chars().last()? {
        'p' => (&body[..body.len() - 1], 1e-12),
        'n' => (&body[..body.len() - 1], 1e-9),
        'u' => (&body[..body.len() - 1], 1e-6),
        'm' => (&body[..body.len() - 1], 1e-3),
        'k' | 'K' => (&body[..body.len() - 1], 1e3),
        'M' => (&body[..body.len() - 1], 1e6),
        'G' => (&body[..body.len() - 1], 1e9),
        _ => (body, 1.0),
    };
    num.trim().parse::<f64>().ok().map(|v| v * mult)
}

/// Rated voltage and dielectric from a value text such as "10uF 6.3V X5R" or "100n/16V/X7R".
pub fn value_details(value: &str) -> (Option<f64>, String) {
    let up = value.to_ascii_uppercase();
    let mut rated = None;
    let mut diel = String::new();
    for tok in up.split(|c: char| c.is_whitespace() || c == '/' || c == '_' || c == ',' || c == ';') {
        let t = tok.trim();
        if t.is_empty() {
            continue;
        }
        for d in ["X5R", "X7R", "X6S", "X7S", "X8R", "X7T", "X6T", "Y5V", "Z5U", "C0G", "NP0", "COG", "NPO"] {
            if t == d {
                diel = match d {
                    "COG" | "NPO" | "NP0" => "C0G".into(),
                    x => x.into(),
                };
            }
        }
        if t.contains("POLY") {
            diel = "POLYMER".into();
        } else if t.starts_with("TANT") {
            diel = "TANTALUM".into();
        }
        // "6.3V", "6V3", "16V"
        if let Some(n) = t.strip_suffix('V') {
            if let Ok(v) = n.parse::<f64>() {
                rated = Some(v);
            }
        } else if let Some((a, b)) = t.split_once('V') {
            if !a.is_empty() && a.chars().all(|c| c.is_ascii_digit()) && !b.is_empty() && b.chars().all(|c| c.is_ascii_digit()) {
                rated = format!("{a}.{b}").parse().ok();
            }
        }
    }
    (rated, diel)
}

/// Capacitor data from a PI_CAP field: "esr=5m esl=0.45n rated=6.3V X5R c@3.3V=2.1u c=10u".
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CapField {
    pub c: Option<f64>,
    pub esr: Option<f64>,
    pub esl: Option<f64>,
    pub rated: Option<f64>,
    pub dielectric: Option<String>,
    /// capacitance at a DC bias: (V, F)
    pub c_at: Vec<(f64, f64)>,
}

pub fn parse_pi_cap(s: &str) -> Result<CapField, String> {
    let mut f = CapField::default();
    for tok in s.split(|c: char| c.is_whitespace() || c == ',' || c == ';').filter(|t| !t.is_empty()) {
        let bad = || format!("PI_CAP: cannot read '{tok}'");
        if let Some((k, v)) = tok.split_once('=') {
            let kl = k.to_ascii_lowercase();
            match kl.as_str() {
                "esr" => f.esr = Some(si(v).ok_or_else(bad)?),
                "esl" => f.esl = Some(si(v).ok_or_else(bad)?),
                "rated" | "v" | "vr" => f.rated = Some(si(v).ok_or_else(bad)?),
                "c" => f.c = Some(si(v).ok_or_else(bad)?),
                _ if kl.starts_with("c@") => f.c_at.push((si(&k[2..]).ok_or_else(bad)?, si(v).ok_or_else(bad)?)),
                _ => return Err(bad()),
            }
        } else {
            let (_, d) = value_details(tok);
            if d.is_empty() {
                return Err(bad());
            }
            f.dielectric = Some(d);
        }
    }
    f.c_at.sort_by(|a, b| a.0.total_cmp(&b.0));
    Ok(f)
}

/// Resistance from a value text: "0R", "4R7", "49.9R", "2k2", "1k", "10", "0.05", "10mR", "0 Ohm".
pub fn parse_resistance(v: &str) -> Option<f64> {
    let t: String = v.chars().filter(|c| !c.is_whitespace()).collect::<String>().replace(',', ".").to_lowercase();
    let t = t.trim_end_matches("ohms").trim_end_matches("ohm").trim_end_matches('ω');
    // "10mR" → 10 mΩ
    let (t, pre) = match t.strip_suffix("mr") {
        Some(x) => (x, 1e-3),
        None => (t, 1.0),
    };
    // "4R7", "2k2", "1m5": the letter is the decimal point and the multiplier
    for (ch, mult) in [('r', 1.0), ('k', 1e3), ('m', 1e-3)] {
        if let Some((a, b)) = t.split_once(ch) {
            if !b.chars().all(|c| c.is_ascii_digit()) {
                return None;
            }
            let a = if a.is_empty() { "0" } else { a };
            let num = if b.is_empty() { a.to_string() } else { format!("{a}.{b}") };
            return num.parse::<f64>().ok().map(|x| x * mult * pre);
        }
    }
    t.parse::<f64>().ok().map(|x| x * pre)
}

/// Impedance of a ferrite bead at 100 MHz from its value: "600R@100MHz", "120 Ohm @ 100 MHz",
/// or a Murata BLM part number ("BLM18PG221SN1": 22·10¹ = 220 Ω).
pub fn bead_z100(v: &str) -> Option<f64> {
    let u = v.trim().to_ascii_uppercase();
    if let Some((a, _)) = u.split_once('@') {
        return parse_resistance(a);
    }
    if u.starts_with("BLM") && u.len() >= 10 {
        let d: Vec<u32> = u[7..10].chars().filter_map(|c| c.to_digit(10)).collect();
        if d.len() == 3 {
            return Some((d[0] * 10 + d[1]) as f64 * 10f64.powi(d[2] as i32));
        }
    }
    None
}

/// Series part data from a PI_SERIES field: "dcr=50m l=2.2u", "z100=600 rdc=0.15", "r=10m",
/// "rdc=0.1 lp=1u rp=800 cp=1p".
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SeriesField {
    pub r: Option<f64>,
    pub l: Option<f64>,
    pub z100: Option<f64>,
    pub rp: Option<f64>,
    pub lp: Option<f64>,
    pub cp: Option<f64>,
}

pub fn parse_pi_series(s: &str) -> Result<SeriesField, String> {
    let mut f = SeriesField::default();
    for tok in s.split(|c: char| c.is_whitespace() || c == ',' || c == ';').filter(|t| !t.is_empty()) {
        let bad = || format!("PI_SERIES: cannot read '{tok}'");
        let (k, v) = tok.split_once('=').ok_or_else(bad)?;
        let x = match k.to_ascii_lowercase().as_str() {
            "r" | "dcr" | "rdc" | "rp" | "z100" => parse_resistance(v).or_else(|| si(v)),
            _ => si(v),
        }
        .ok_or_else(bad)?;
        match k.to_ascii_lowercase().as_str() {
            "r" | "dcr" | "rdc" => f.r = Some(x),
            "l" => f.l = Some(x),
            "z100" => f.z100 = Some(x),
            "rp" => f.rp = Some(x),
            "lp" => f.lp = Some(x),
            "cp" => f.cp = Some(x),
            _ => return Err(bad()),
        }
    }
    Ok(f)
}

/// Target from a PI_TARGET field for `net`: "VCCINT=10m fmax=50M" or "15mOhm" (any rail).
/// Returns (Ω, fmax Hz if given).
pub fn parse_pi_target(s: &str, net: &str) -> Result<Option<(f64, Option<f64>)>, String> {
    for part in s.split([';', ',']).map(str::trim).filter(|p| !p.is_empty()) {
        let mut z = None;
        let mut fmax = None;
        let mut for_net = true;
        for tok in part.split_whitespace() {
            if let Some(v) = tok.strip_prefix("fmax=") {
                fmax = Some(si(v).ok_or_else(|| format!("PI_TARGET: cannot read '{tok}'"))?);
            } else if let Some((n, v)) = tok.rsplit_once('=') {
                for_net = n == net || n.trim_start_matches('/') == net.trim_start_matches('/') || net.rsplit('/').next() == Some(n);
                z = Some(si(v).ok_or_else(|| format!("PI_TARGET: cannot read '{tok}'"))?);
            } else {
                z = Some(si(tok).ok_or_else(|| format!("PI_TARGET: cannot read '{tok}'"))?);
            }
        }
        if let (true, Some(z)) = (for_net, z) {
            return Ok(Some((z, fmax)));
        }
    }
    Ok(None)
}

// --- results -------------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct CapResult {
    pub refdes: String,
    /// net of the capacitor's supply pad
    pub net: String,
    pub value: String,
    pub package: String,
    pub at: P,
    /// footprint outline from its pads: [x0, y0, x1, y1] (mm)
    pub bbox: [f64; 4],
    pub layer: u16,
    /// nominal and effective (DC-bias derated) capacitance (F)
    pub c_nom: f64,
    pub c_eff: f64,
    pub dielectric: String,
    pub rated_v: Option<f64>,
    pub rated_assumed: bool,
    pub esr: f64,
    pub esl: f64,
    /// ground-side mounting inductance: pad → via → ground (H)
    pub l_gnd: f64,
    /// "PI_CAP" (kisi.json / field) or "estimate"
    pub source: String,
    /// smallest path inductance to any IC (H) and that IC
    pub l_path_min: Option<f64>,
    pub nearest_ic: Option<String>,
    pub flags: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CapLink {
    /// index into `IcPdnResult::caps`
    pub cap: usize,
    pub refdes: String,
    /// loop inductance IC → plane / vias → capacitor pad (H), without the capacitor itself
    pub l_path: f64,
    pub r_path: f64,
    /// l_path + ground-side mounting + ESL (H)
    pub l_total: f64,
    /// series resonance of the capacitor with l_total (Hz)
    pub srf_hz: f64,
    pub dist_mm: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Peak {
    pub f: f64,
    pub z: f64,
    pub above_target: bool,
    pub note: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Recommendation {
    pub f_lo: f64,
    pub f_hi: f64,
    pub z_max: f64,
    pub value: String,
    pub package: String,
    pub count: usize,
    /// place within this distance of the IC (mm)
    pub radius_mm: f64,
    /// inductance per capacitor allowed at that radius (H)
    pub l_each: f64,
    pub note: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PortResult {
    pub refdes: String,
    /// net of the IC's supply pins (the rail or a net behind series parts)
    pub net: String,
    /// refdes, with the net when the IC draws from several nets of the domain
    pub label: String,
    pub at: P,
    /// footprint outline from its pads: [x0, y0, x1, y1] (mm)
    pub bbox: [f64; 4],
    pub current: f64,
    pub pins: usize,
    pub z: Vec<f64>,
    pub target: Option<f64>,
    pub target_source: String,
    /// upper end of the band the target applies to (Hz)
    pub f_max: f64,
    pub above: Vec<(f64, f64)>,
    pub caps: Vec<CapLink>,
    pub peaks: Vec<Peak>,
    pub recommendations: Vec<Recommendation>,
    /// fit of the path inductance over distance: l_path ≈ a + b·ln(r / 1 mm) (H)
    pub spread_fit: Option<(f64, f64)>,
}

#[derive(Debug, Clone, Serialize)]
pub struct IcPdnResult {
    pub net: String,
    pub voltage: f64,
    pub freq: Vec<f64>,
    pub ports: Vec<PortResult>,
    pub caps: Vec<CapResult>,
    pub cell_mm: f64,
    pub layers: Vec<u16>,
    pub nodes: usize,
    pub regulators: Vec<String>,
    /// nets of the rail domain: the rail and the nets behind series parts, up to the regulators
    pub domain: Vec<DomainNet>,
    /// series parts inside the domain
    pub series: Vec<SeriesResult>,
    /// where the domain stops (switch nodes, size limit)
    pub boundary: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DomainNet {
    pub net: String,
    pub voltage: f64,
    pub caps: usize,
    pub c_total: f64,
    pub ics: Vec<String>,
    pub regulators: Vec<String>,
}

/// A resistor, inductor or ferrite bead between two nets of the domain.
#[derive(Debug, Clone, Serialize)]
pub struct SeriesResult {
    pub refdes: String,
    pub value: String,
    /// "resistor", "inductor" or "bead"
    pub kind: &'static str,
    pub package: String,
    pub at: P,
    /// footprint outline from its pads: [x0, y0, x1, y1] (mm)
    pub bbox: [f64; 4],
    pub from: String,
    pub to: String,
    /// series resistance (Ω) and inductance (H)
    pub r: f64,
    pub l: f64,
    /// bead: parallel R (Ω), L (H), C (F) of the lossy part
    pub rp: Option<f64>,
    pub lp: Option<f64>,
    pub cp: Option<f64>,
    /// |Z| at 100 MHz (Ω)
    pub z100: f64,
    /// "PI_SERIES", "PI_REG pass", "value" or "estimate"
    pub source: String,
}

impl SeriesResult {
    fn link(&self) -> Link {
        Link::Series { r: self.r, l: self.l, rp: self.rp.unwrap_or(0.0), lp: self.lp.unwrap_or(0.0), cp: self.cp.unwrap_or(0.0) }
    }
    /// inductance below the bead's peak (H): what resonates with the capacitors behind it
    fn l_low(&self) -> f64 {
        self.l + self.lp.unwrap_or(0.0)
    }
}

// --- model -----------------------------------------------------------------------------------

/// Branch between two nodes of the AC grid.
#[derive(Clone, Copy)]
enum Link {
    /// plane / trace: sheet resistance of `t_m` copper (rail and return), μ0·d per square, ÷ width factor
    Sheet { t_m: f64, d_m: f64, wf: f64 },
    /// via barrel: L (H), R (Ω)
    Via { l: f64, r: f64 },
    /// pin-group short / regulator tap
    Short,
    /// series part: R + jωL, with a ferrite's lossy part Lp ∥ Rp ∥ Cp (rp = 0: none)
    Series { r: f64, l: f64, rp: f64, lp: f64, cp: f64 },
}

/// Shunt from a node to the ground reference.
#[derive(Clone, Copy)]
enum Shunt {
    /// plane capacitance of a cell (F) with dielectric loss
    Plane { c: f64, tand: f64 },
    /// capacitor: ESR, total series L, C
    Cap { r: f64, l: f64, c: f64 },
    /// regulator: R + L
    Vrm { r: f64, l: f64 },
}

impl Link {
    fn y(&self, f: f64) -> Cx {
        let w = 2.0 * PI * f;
        match *self {
            Link::Sheet { t_m, d_m, wf } => {
                let rs = (RHO / t_m).max((PI * f * MU0 * RHO).sqrt());
                Cx::new(2.0 * rs / wf, w * MU0 * d_m / wf).inv()
            }
            Link::Via { l, r } => Cx::new(r, w * l).inv(),
            Link::Short => Cx::new(1.0 / R_SHORT, 0.0),
            Link::Series { r, l, rp, lp, cp } => {
                let mut z = Cx::new(r.max(R_SHORT), w * l);
                if rp > 0.0 && lp > 0.0 {
                    let y = Cx::new(1.0 / rp, w * cp - 1.0 / (w * lp));
                    z = z + y.inv();
                }
                z.inv()
            }
        }
    }
}

impl Shunt {
    fn y(&self, f: f64) -> Cx {
        let w = 2.0 * PI * f;
        match *self {
            Shunt::Plane { c, tand } => Cx::new(w * c * tand, w * c),
            Shunt::Cap { r, l, c } => Cx::new(r, w * l - 1.0 / (w * c)).inv(),
            Shunt::Vrm { r, l } => Cx::new(r, w * l).inv(),
        }
    }
}

struct Model {
    n: usize,
    links: Vec<(usize, usize, Link)>,
    shunts: Vec<(usize, Shunt)>,
}

impl Model {
    /// Admittance matrix at f; `plane`, `caps`, `vrm` include those shunts; `ground` adds a
    /// near-ideal tie to the reference at these nodes.
    fn triplets(&self, f: f64, plane: bool, others: bool, ground: &[usize]) -> Vec<(usize, usize, f64, f64)> {
        let mut t = Vec::with_capacity(self.links.len() * 4 + self.shunts.len() + ground.len());
        for &(a, b, l) in &self.links {
            let y = l.y(f);
            t.push((a, a, y.re, y.im));
            t.push((b, b, y.re, y.im));
            t.push((a, b, -y.re, -y.im));
            t.push((b, a, -y.re, -y.im));
        }
        for &(i, s) in &self.shunts {
            let keep = match s {
                Shunt::Plane { .. } => plane,
                _ => others,
            };
            if keep {
                let y = s.y(f);
                t.push((i, i, y.re, y.im));
            }
        }
        for &g in ground {
            t.push((g, g, 1e9, 0.0));
        }
        t
    }
}

fn polygon_area(p: &[P]) -> f64 {
    let n = p.len();
    let mut a = 0.0;
    for i in 0..n {
        a += p[i][0] * p[(i + 1) % n][1] - p[(i + 1) % n][0] * p[i][1];
    }
    (a / 2.0).abs()
}

fn fmt_c(c: f64) -> String {
    let (v, u) = if c >= 1e-6 { (c * 1e6, "µF") } else if c >= 1e-9 { (c * 1e9, "nF") } else { (c * 1e12, "pF") };
    format!("{}{}", if v >= 10.0 { format!("{v:.0}") } else { format!("{v:.2}").trim_end_matches('0').trim_end_matches('.').to_string() }, u)
}

/// PDN of `setup` seen from every load IC.
pub fn pdn_ic(board: &Board, setup: &RailSetup, others: &[RailSetup], p: &PdnParams) -> Result<IcPdnResult, String> {
    let data = DecapData::builtin();
    let sdata = SeriesData::builtin();
    let su = board.stackup();
    let ncu = su.n();
    let net = find_net(board, &setup.net).ok_or_else(|| format!("net {} not found", setup.net))?;
    let vrail0 = setup.voltage.abs();
    let mut warnings: Vec<String> = Vec::new();
    let is_gnd = |n: u32| n != 0 && is_ground_name(board.net_name(n));

    // --- rail domain: the rail and the nets behind series parts (resistors up to r_max_ohm,
    // inductors, ferrite beads), up to the regulators. A net at a regulator pin without a rail of
    // its own and without a capacitor to ground (a switch node) ends the domain.
    let setups: Vec<&RailSetup> = std::iter::once(setup).chain(others.iter().filter(|s| s.net != setup.net)).collect();
    let setup_of = |n: u32| -> Option<&RailSetup> { setups.iter().copied().find(|s| find_net(board, &s.net) == Some(n)) };
    let mut fp_all: HashMap<u32, Vec<&Pad>> = HashMap::new();
    for pd in board.pads.iter().filter(|q| !q.layers.is_empty()) {
        fp_all.entry(pd.fp).or_default().push(pd);
    }
    // outline of a footprint from all its pads (for the board highlight)
    let fp_bbox = |fi: u32| -> [f64; 4] {
        let mut b = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
        for q in board.pads.iter().filter(|q| q.fp == fi) {
            let h = 0.5 * q.size[0].max(q.size[1]);
            b = [b[0].min(q.at[0] - h), b[1].min(q.at[1] - h), b[2].max(q.at[0] + h), b[3].max(q.at[1] + h)];
        }
        if b[0].is_finite() {
            b
        } else {
            let a = board.footprints[fi as usize].at;
            [a[0] - 0.5, a[1] - 0.5, a[0] + 0.5, a[1] + 0.5]
        }
    };
    let prefix = |r: &str| r.chars().take_while(|c| c.is_ascii_alphabetic()).collect::<String>().to_ascii_uppercase();
    // candidates: (footprint, net a, net b, kind)
    let mut cands: Vec<(u32, u32, u32, &'static str)> = Vec::new();
    for (fi, f) in board.footprints.iter().enumerate() {
        if f.dnp {
            continue;
        }
        let Some(pads) = fp_all.get(&(fi as u32)) else { continue };
        if pads.len() != 2 {
            continue;
        }
        let (a, b) = (pads[0].net, pads[1].net);
        if a == 0 || b == 0 || a == b || is_gnd(a) || is_gnd(b) {
            continue;
        }
        let field_r = f.prop("PI_SERIES").and_then(|v| parse_pi_series(v).ok()).and_then(|x| x.r);
        let kind = match prefix(&f.reference).as_str() {
            "R" => match field_r.or_else(|| parse_resistance(&f.value)) {
                Some(r) if r <= sdata.r_max_ohm => "resistor",
                _ => continue,
            },
            "FB" => "bead",
            "L" => {
                let up = f.value.to_ascii_uppercase();
                if up.contains("FERRITE") || up.contains("BEAD") || bead_z100(&f.value).is_some() {
                    "bead"
                } else if si(&f.value).is_some() || f.prop("PI_SERIES").and_then(|v| parse_pi_series(v).ok()).is_some_and(|x| x.l.is_some()) {
                    "inductor"
                } else {
                    "bead"
                }
            }
            _ => continue,
        };
        cands.push((fi as u32, a, b, kind));
    }
    let cand_fp: std::collections::HashSet<u32> = cands.iter().map(|c| c.0).collect();
    let reg_fp: Vec<bool> = board
        .footprints
        .iter()
        .enumerate()
        .map(|(fi, f)| !cand_fp.contains(&(fi as u32)) && (f.prop("PI_REG").is_some() || f.prop("PI_SOURCE").is_some() || setups.iter().any(|s| s.sources.iter().any(|x| x.refdes == f.reference))))
        .collect();
    let has_cap = |n: u32| -> bool {
        fp_all.iter().any(|(fi, pads)| {
            pads.len() == 2 && board.footprints[*fi as usize].reference.starts_with('C') && ((pads[0].net == n && is_gnd(pads[1].net)) || (pads[1].net == n && is_gnd(pads[0].net)))
        })
    };
    let mut dom: Vec<u32> = vec![net];
    let mut parts: Vec<(u32, u32, u32, &'static str)> = Vec::new();
    let mut boundary: Vec<String> = Vec::new();
    let mut qi = 0;
    while qi < dom.len() {
        let u = dom[qi];
        qi += 1;
        for &(fi, a, b, kind) in &cands {
            if (a != u && b != u) || parts.iter().any(|x| x.0 == fi) {
                continue;
            }
            let v = if a == u { b } else { a };
            if !dom.contains(&v) {
                // switch node: at a regulator pin, no rail of its own and no capacitor to ground
                if setup_of(v).is_none() && !has_cap(v) {
                    if let Some(q) = board.pads.iter().find(|q| q.net == v && reg_fp[q.fp as usize]) {
                        let note = format!(
                            "{} ({kind}) ends at {}: switch node or pin of the regulator {}",
                            board.footprints[fi as usize].reference,
                            board.net_name(v),
                            board.footprints[q.fp as usize].reference
                        );
                        if !boundary.contains(&note) {
                            boundary.push(note);
                        }
                        continue;
                    }
                }
                if dom.len() >= sdata.max_nets {
                    boundary.push(format!("domain limited to {} nets: {} not followed", sdata.max_nets, board.net_name(v)));
                    continue;
                }
                dom.push(v);
            }
            parts.push((fi, a, b, kind));
        }
    }
    let nd = dom.len();
    let dom_idx: HashMap<u32, usize> = dom.iter().enumerate().map(|(i, &n)| (n, i)).collect();
    let volt: Vec<f64> = dom
        .iter()
        .map(|&n| setup_of(n).map(|s| s.voltage.abs()).filter(|v| *v > 0.0).or_else(|| crate::pi::voltage_from_name(board.net_name(n)).map(f64::abs)).unwrap_or(vrail0))
        .collect();

    // copper of the domain
    let mut bb = BBox::empty();
    let mut used = vec![false; ncu];
    let mut used_n = vec![vec![false; ncu]; nd];
    let mut cu_area = 0.0;
    for z in board.zones.iter() {
        let Some(&ni) = dom_idx.get(&z.net) else { continue };
        bb.add(z.bbox.min);
        bb.add(z.bbox.max);
        used[z.layer as usize] = true;
        used_n[ni][z.layer as usize] = true;
        cu_area += polygon_area(&z.pts);
    }
    for t in board.tracks.iter().filter(|t| !t.pad) {
        let Some(&ni) = dom_idx.get(&t.net) else { continue };
        for q in [t.a, t.b] {
            bb.add([q[0] - t.width, q[1] - t.width]);
            bb.add([q[0] + t.width, q[1] + t.width]);
        }
        used[t.layer as usize] = true;
        used_n[ni][t.layer as usize] = true;
        cu_area += geom::len(geom::sub(t.b, t.a)) * t.width;
    }
    for pd in board.pads.iter() {
        let Some(&ni) = dom_idx.get(&pd.net) else { continue };
        let r = pd.size[0].max(pd.size[1]);
        bb.add([pd.at[0] - r, pd.at[1] - r]);
        bb.add([pd.at[0] + r, pd.at[1] + r]);
        for &l in &pd.layers {
            used[l as usize] = true;
            used_n[ni][l as usize] = true;
        }
        cu_area += pd.size[0] * pd.size[1] * pd.layers.len() as f64;
    }
    if !bb.is_valid() {
        return Err(format!("{}: no copper found", setup.net));
    }
    let layers: Vec<usize> = (0..ncu).filter(|&l| used[l]).collect();
    let raw = (cu_area / p.ic_cells.max(500) as f64).sqrt();
    let cell = [0.1, 0.15, 0.2, 0.25, 0.3, 0.4, 0.5, 0.6, 0.75, 1.0, 1.25, 1.5, 2.0, 2.5, 3.0].into_iter().find(|&v| v >= raw).unwrap_or(raw);
    let g = Grid {
        x0: bb.min[0] - cell,
        y0: bb.min[1] - cell,
        cell,
        nx: (((bb.max[0] - bb.min[0]) / cell).ceil() as usize) + 2,
        ny: (((bb.max[1] - bb.min[1]) / cell).ceil() as usize) + 2,
    };
    let per = g.nx * g.ny;
    let n_lay: usize = used_n.iter().map(|u| u.iter().filter(|x| **x).count()).sum();
    if per * (n_lay + ncu) > 40_000_000 {
        return Err(format!("{}: grid too large", setup.net));
    }
    // copper weight per net and layer, ground copper per layer (all layers)
    let mut w: Vec<Vec<Vec<f32>>> = vec![vec![Vec::new(); ncu]; nd];
    for (ni, &dn) in dom.iter().enumerate() {
        for l in (0..ncu).filter(|&l| used_n[ni][l]) {
            let mut wl = vec![0f32; per];
            for z in board.zones.iter().filter(|z| z.net == dn && z.layer as usize == l) {
                fill_polygon(&g, &z.pts, &mut wl);
            }
            for t in board.tracks.iter().filter(|t| t.net == dn && !t.pad && t.layer as usize == l) {
                fill_track(&g, t.a, t.b, t.width, &mut wl);
            }
            for pd in board.pads.iter().filter(|q| q.net == dn && q.layers.contains(&(l as u16))) {
                for k in pad_cells(&g, pd) {
                    wl[k] = 1.0;
                }
            }
            w[ni][l] = wl;
        }
    }
    let mut gw: Vec<Vec<bool>> = vec![vec![false; per]; ncu];
    let mut gnd_any = vec![false; ncu];
    for z in board.zones.iter().filter(|z| is_gnd(z.net)) {
        let mut tmp = vec![0f32; per];
        if fill_polygon(&g, &z.pts, &mut tmp) > 0 {
            gnd_any[z.layer as usize] = true;
            for (k, v) in tmp.iter().enumerate() {
                if *v > 0.0 {
                    gw[z.layer as usize][k] = true;
                }
            }
        }
    }
    if !gnd_any.iter().any(|x| *x) {
        warnings.push("no ground copper (plane or pour) under the rail: plane capacitance and inductance from the nearest layer, rough".into());
    }
    // nearest ground layer above / below a cell: (layer, distance mm)
    let ref_of = |l: usize, k: usize| -> (Option<(usize, f64)>, Option<(usize, f64)>) {
        let up = (0..l).rev().find(|&j| gw[j][k]).map(|j| (j, su.gap_between(l, j)));
        let dn = (l + 1..ncu).find(|&j| gw[j][k]).map(|j| (j, su.gap_between(l, j)));
        (up, dn)
    };
    let fallback_ref = |l: usize| -> (usize, f64) {
        (0..ncu).filter(|&j| j != l && gnd_any[j]).map(|j| (j, su.gap_between(l, j))).min_by(|a, b| a.1.total_cmp(&b.1)).unwrap_or((if l == 0 { ncu - 1 } else { 0 }, su.gap_between(l, if l == 0 { ncu - 1 } else { 0 }).max(0.1)))
    };
    let tand_between = |a: usize, b: usize| -> f64 {
        let (a, b) = (a.min(b), a.max(b));
        if b == a {
            return 0.02;
        }
        su.dielectrics[a..b].iter().map(|d| d.loss_tangent).sum::<f64>() / (b - a) as f64
    };

    // nodes: copper cells per net and layer
    let mut node_of: Vec<Vec<Vec<u32>>> = vec![vec![Vec::new(); ncu]; nd];
    let mut node_pos: Vec<(usize, usize, usize)> = Vec::new();
    for ni in 0..nd {
        for l in (0..ncu).filter(|&l| used_n[ni][l]) {
            let mut m = vec![u32::MAX; per];
            for k in 0..per {
                if w[ni][l][k] > 0.0 {
                    m[k] = node_pos.len() as u32;
                    node_pos.push((ni, l, k));
                }
            }
            node_of[ni][l] = m;
        }
    }
    let ncell = node_pos.len();
    let mut model = Model { n: ncell, links: Vec::new(), shunts: Vec::new() };
    let mut d_eff = vec![0.0f64; ncell];
    let mut no_ref = 0usize;
    let a_cell = (cell * 1e-3).powi(2);
    for (i, &(ni, l, k)) in node_pos.iter().enumerate() {
        let (up, dn) = ref_of(l, k);
        let refs: Vec<(usize, f64)> = match (up, dn) {
            (None, None) => {
                no_ref += 1;
                vec![fallback_ref(l)]
            }
            (a, b) => [a, b].into_iter().flatten().collect(),
        };
        let mut c = 0.0;
        let mut inv_d = 0.0;
        let mut tand = 0.0;
        for &(j, d) in &refs {
            let d = d.max(0.01);
            let cj = EPS0 * su.er_between(l, j) * a_cell * w[ni][l][k] as f64 / (d * 1e-3);
            tand += tand_between(l, j) * cj;
            c += cj;
            inv_d += 1.0 / d;
        }
        d_eff[i] = 1.0 / inv_d;
        model.shunts.push((i, Shunt::Plane { c, tand: if c > 0.0 { tand / c } else { 0.02 } }));
    }
    if no_ref > 0 && no_ref * 10 > ncell {
        warnings.push(format!("{}% of the rail copper has no ground copper above or below: nearest ground layer used", no_ref * 100 / ncell));
    }
    // in-layer links
    for ni in 0..nd {
        for l in (0..ncu).filter(|&l| used_n[ni][l]) {
            let t_m = su.copper[l].thickness * 1e-3;
            let (wl, m) = (&w[ni][l], &node_of[ni][l]);
            for j in 0..g.ny {
                for i in 0..g.nx {
                    let k = j * g.nx + i;
                    if m[k] == u32::MAX {
                        continue;
                    }
                    for (xi, xj) in [(i + 1, j), (i, j + 1)] {
                        if xi >= g.nx || xj >= g.ny {
                            continue;
                        }
                        let k2 = xj * g.nx + xi;
                        if m[k2] == u32::MAX {
                            continue;
                        }
                        let (a, b) = (wl[k] as f64, wl[k2] as f64);
                        let wf = 2.0 * a * b / (a + b);
                        let (na, nb) = (m[k] as usize, m[k2] as usize);
                        let d_m = 0.5 * (d_eff[na] + d_eff[nb]) * 1e-3;
                        model.links.push((na, nb, Link::Sheet { t_m, d_m, wf }));
                    }
                }
            }
        }
    }
    // vias and plated holes of the domain nets
    let gnd_vias: Vec<P> = board.vias.iter().filter(|v| is_gnd(v.net)).map(|v| v.at).chain(board.pads.iter().filter(|q| is_gnd(q.net) && q.drill > 0.0).map(|q| q.at)).collect();
    let near_gnd_via = |at: P| -> Option<f64> { gnd_vias.iter().map(|q| geom::len(geom::sub(*q, at))).filter(|&d| d < 3.0).min_by(|a, b| a.total_cmp(b)) };
    let mut holes: Vec<(usize, P, f64, u16, u16)> = board.vias.iter().filter_map(|v| dom_idx.get(&v.net).map(|&ni| (ni, v.at, v.drill, v.from, v.to))).collect();
    for q in board.pads.iter().filter(|q| q.drill > 0.0 && q.layers.len() > 1) {
        if let Some(&ni) = dom_idx.get(&q.net) {
            holes.push((ni, q.at, q.drill, *q.layers.iter().min().unwrap(), *q.layers.iter().max().unwrap()));
        }
    }
    for &(ni, at, drill, from, to) in &holes {
        let Some((ci, cj)) = g.cell_of(at) else { continue };
        // the node of the net on a layer at the hole: its cell, else a neighbour (a pour around
        // a via with thermal relief can leave the centre cell empty)
        let node_at = |l: usize| -> Option<u32> {
            let m = &node_of[ni][l];
            let mut best: Option<(i64, u32)> = None;
            for dj in -1i64..=1 {
                for di in -1i64..=1 {
                    let (x, y) = (ci as i64 + di, cj as i64 + dj);
                    if x < 0 || y < 0 || x >= g.nx as i64 || y >= g.ny as i64 {
                        continue;
                    }
                    let n = m[y as usize * g.nx + x as usize];
                    let d = di * di + dj * dj;
                    if n != u32::MAX && best.is_none_or(|b| d < b.0) {
                        best = Some((d, n));
                    }
                }
            }
            best.map(|b| b.1)
        };
        let on: Vec<(usize, u32)> = (0..ncu).filter(|&l| used_n[ni][l] && l as u16 >= from && l as u16 <= to).filter_map(|l| node_at(l).map(|n| (l, n))).collect();
        let r = (drill / 2.0).max(0.05);
        let s = near_gnd_via(at).unwrap_or(1.5).clamp(2.0 * r + 0.1, 3.0);
        for pair in on.windows(2) {
            let h = su.gap_between(pair[0].0, pair[1].0).max(0.01) * 1e-3;
            let l = MU0 * h / (2.0 * PI) * (s / r).ln().max(0.2);
            let rv = RHO * h / (PI * drill * 1e-3 * 25e-6);
            model.links.push((pair[0].1 as usize, pair[1].1 as usize, Link::Via { l, r: rv }));
        }
    }

    // footprints → pads on the domain nets
    let mut fp_pads: HashMap<u32, Vec<&Pad>> = HashMap::new();
    for pd in board.pads.iter().filter(|q| dom_idx.contains_key(&q.net)) {
        fp_pads.entry(pd.fp).or_default().push(pd);
    }
    let fp_by_ref: HashMap<&str, u32> = board.footprints.iter().enumerate().map(|(i, f)| (f.reference.as_str(), i as u32)).collect();
    let pad_nodes = |pd: &Pad| -> Vec<usize> {
        let mut out = Vec::new();
        let Some(&ni) = dom_idx.get(&pd.net) else { return out };
        for &l in &pd.layers {
            if !used_n[ni][l as usize] {
                continue;
            }
            for k in pad_cells(&g, pd) {
                let n = node_of[ni][l as usize][k];
                if n != u32::MAX {
                    out.push(n as usize);
                }
            }
        }
        out.sort();
        out.dedup();
        out
    };
    // a port node joined to a set of pad nodes
    let add_group = |model: &mut Model, nodes: &[usize]| -> usize {
        let id = model.n;
        model.n += 1;
        for &k in nodes {
            model.links.push((id, k, Link::Short));
        }
        id
    };
    let one_node = |model: &mut Model, nodes: &[usize]| -> Option<usize> {
        match nodes {
            [] => None,
            [x] => Some(*x),
            _ => Some(add_group(model, nodes)),
        }
    };

    // series parts inside the domain
    let mut series: Vec<SeriesResult> = Vec::new();
    let mut series_ni: Vec<(usize, usize)> = Vec::new();
    for &(fi, a, b, kind) in &parts {
        let f = &board.footprints[fi as usize];
        let pads = &fp_all[&fi];
        let pkg = crate::package::chip_package(&f.lib, pads[0], pads[1]);
        let field = match f.prop("PI_SERIES").map(parse_pi_series) {
            Some(Ok(x)) => Some(x),
            Some(Err(e)) => {
                warnings.push(format!("{}: {e}", f.reference));
                None
            }
            None => None,
        };
        let fd = field.clone().unwrap_or_default();
        // DC resistance of a PI_REG pass stage ("A > B pass r=0.1")
        let pass_r = f.prop("PI_REG").filter(|v| v.contains("pass")).and_then(|v| v.split_whitespace().find_map(|t| t.strip_prefix("r=")).and_then(|x| parse_resistance(x).or_else(|| si(x))));
        let esl = data.esl(pkg);
        let (r, l, bead, source) = match kind {
            "resistor" => {
                let r = fd.r.or_else(|| parse_resistance(&f.value)).unwrap_or(0.0).max(sdata.zero_ohm);
                (r, fd.l.unwrap_or(esl), None, if fd.r.is_some() { "PI_SERIES" } else { "value" })
            }
            "inductor" => {
                let l = fd.l.or_else(|| si(&f.value)).unwrap_or(1e-6);
                let dcr = fd.r.or(pass_r).unwrap_or_else(|| sdata.inductor_dcr_ohm.get(pkg).or_else(|| sdata.inductor_dcr_ohm.get("large")).copied().unwrap_or(0.05));
                (dcr, l, None, if field.is_some() { "PI_SERIES" } else if pass_r.is_some() { "PI_REG pass" } else { "value" })
            }
            _ => {
                let z_val = bead_z100(&f.value);
                let z = fd.z100.or(z_val).unwrap_or(sdata.bead.z100_ohm);
                let lp = fd.lp.unwrap_or(z / (2.0 * PI * 1e8));
                let rp = fd.rp.unwrap_or(sdata.bead.rp_factor * z);
                let cp = fd.cp.unwrap_or(sdata.bead.cp_pf * 1e-12);
                let rdc = fd.r.or(pass_r).unwrap_or(sdata.bead.rdc_ohm);
                let src = if fd.z100.is_some() || fd.lp.is_some() { "PI_SERIES" } else if z_val.is_some() { "value" } else { "estimate" };
                (rdc, esl, Some((rp, lp, cp)), src)
            }
        };
        let (ia, ib) = (dom_idx[&a], dom_idx[&b]);
        let (ia, ib, pa, pb) = if ia <= ib { (ia, ib, pads[0], pads[1]) } else { (ib, ia, pads[1], pads[0]) };
        let mut sr = SeriesResult {
            refdes: f.reference.clone(),
            value: f.value.clone(),
            kind,
            package: pkg.into(),
            at: [(pads[0].at[0] + pads[1].at[0]) / 2.0, (pads[0].at[1] + pads[1].at[1]) / 2.0],
            bbox: fp_bbox(fi),
            from: board.net_name(dom[ia]).to_string(),
            to: board.net_name(dom[ib]).to_string(),
            r,
            l,
            rp: bead.map(|x| x.0),
            lp: bead.map(|x| x.1),
            cp: bead.map(|x| x.2),
            z100: 0.0,
            source: source.into(),
        };
        sr.z100 = sr.link().y(1e8).inv().abs();
        let (na, nb) = (pad_nodes(pa), pad_nodes(pb));
        let (Some(na), Some(nb)) = (one_node(&mut model, &na), one_node(&mut model, &nb)) else {
            warnings.push(format!("{}: no copper at its pads, not connected", f.reference));
            continue;
        };
        model.links.push((na, nb, sr.link()));
        series.push(sr);
        series_ni.push((ia, ib));
    }

    // regulators: the sources of the domain nets that are not series parts inside the domain
    let part_refs: std::collections::HashSet<&str> = series.iter().map(|x| x.refdes.as_str()).collect();
    let mut regulators = Vec::new();
    let mut reg_by_net: Vec<Vec<String>> = vec![Vec::new(); nd];
    for (ni, &dn) in dom.iter().enumerate() {
        let Some(s) = setup_of(dn) else { continue };
        for src in &s.sources {
            if part_refs.contains(src.refdes.as_str()) {
                continue;
            }
            let Some(&fi) = fp_by_ref.get(src.refdes.as_str()) else { continue };
            let pads: Vec<&Pad> = fp_pads.get(&fi).map(|v| v.iter().copied().filter(|q| q.net == dn && (src.pads.is_empty() || src.pads.contains(&q.number))).collect()).unwrap_or_default();
            let nodes: Vec<usize> = pads.iter().flat_map(|q| pad_nodes(q)).collect();
            if nodes.is_empty() {
                continue;
            }
            let id = add_group(&mut model, &nodes);
            model.shunts.push((id, Shunt::Vrm { r: p.vrm_r, l: p.vrm_l }));
            if !regulators.contains(&src.refdes) {
                regulators.push(src.refdes.clone());
            }
            reg_by_net[ni].push(src.refdes.clone());
        }
    }
    if regulators.is_empty() {
        warnings.push("no regulator / source in the rail domain: the low-frequency end is open (capacitors only)".into());
    }

    // load ICs (ports): one per component and net, currents summed
    // ports are the ICs: load points on series parts (resistors, beads, inductors), connectors,
    // diodes and test points are passed through, not decoupled
    let passive = |r: &str| {
        let p: String = r.chars().take_while(|c| c.is_ascii_alphabetic()).collect::<String>().to_ascii_uppercase();
        matches!(p.as_str(), "R" | "RN" | "FB" | "L" | "F" | "J" | "P" | "CN" | "X" | "TP" | "D" | "SW" | "K" | "BT" | "C")
    };
    let mut loads: Vec<(String, usize, f64)> = Vec::new();
    for (ni, &dn) in dom.iter().enumerate() {
        let Some(s) = setup_of(dn) else { continue };
        for l in s.loads.iter().filter(|l| !passive(&l.refdes)) {
            match loads.iter_mut().find(|x| x.0 == l.refdes && x.1 == ni) {
                Some(x) => x.2 += l.current.abs(),
                None => loads.push((l.refdes.clone(), ni, l.current.abs())),
            }
        }
    }
    struct Port {
        refdes: String,
        fi: u32,
        ni: usize,
        node: usize,
        at: P,
        current: f64,
        pins: usize,
        target: Option<(f64, Option<f64>, String)>,
    }
    let mut ports: Vec<Port> = Vec::new();
    for (refdes, ni, current) in &loads {
        let Some(&fi) = fp_by_ref.get(refdes.as_str()) else { continue };
        let dn = dom[*ni];
        let pads: Vec<&Pad> = fp_pads.get(&fi).map(|v| v.iter().copied().filter(|q| q.net == dn).collect()).unwrap_or_default();
        let nodes: Vec<usize> = pads.iter().flat_map(|q| pad_nodes(q)).collect();
        if nodes.is_empty() {
            continue;
        }
        let at = [pads.iter().map(|q| q.at[0]).sum::<f64>() / pads.len() as f64, pads.iter().map(|q| q.at[1]).sum::<f64>() / pads.len() as f64];
        let fp = &board.footprints[fi as usize];
        let target = match fp.prop("PI_TARGET").map(|v| parse_pi_target(v, board.net_name(dn))) {
            Some(Ok(Some((z, fm)))) => {
                let src = crate::tree::field_source(fp, "PI_TARGET").map(|(k, n)| format!("PI_TARGET ({k}{}{n})", if n.is_empty() { "" } else { ", " })).unwrap_or_else(|| "PI_TARGET".into());
                Some((z, fm, src))
            }
            Some(Err(e)) => {
                warnings.push(format!("{refdes}: {e}"));
                None
            }
            _ => None,
        };
        let id = add_group(&mut model, &nodes);
        ports.push(Port { refdes: refdes.clone(), fi, ni: *ni, node: id, at, current: *current, pins: pads.len(), target });
    }
    if ports.is_empty() {
        return Err(format!("{}: no load IC in the rail domain (its loads are series parts, connectors or missing PI_LOAD): nothing to decouple here", setup.net));
    }

    // decoupling capacitors
    let mut caps: Vec<CapResult> = Vec::new();
    let mut cap_node: Vec<usize> = Vec::new();
    let mut cap_ni: Vec<usize> = Vec::new();
    let mut unparsed = 0;
    let mut assumed_rating = 0;
    let gnd_via_of = |pd: &Pad| -> Option<(f64, f64)> {
        board.vias.iter().filter(|v| v.net == pd.net).map(|v| (geom::len(geom::sub(v.at, pd.at)), v.drill)).filter(|x| x.0 < 2.0).min_by(|x, y| x.0.total_cmp(&y.0))
    };
    for (fi, f) in board.footprints.iter().enumerate() {
        if f.dnp || !f.reference.starts_with('C') {
            continue;
        }
        let all: Vec<&Pad> = board.pads.iter().filter(|q| q.fp == fi as u32 && !q.layers.is_empty()).collect();
        if all.len() != 2 {
            continue;
        }
        let (pr, pg) = if dom_idx.contains_key(&all[0].net) && is_gnd(all[1].net) {
            (all[0], all[1])
        } else if dom_idx.contains_key(&all[1].net) && is_gnd(all[0].net) {
            (all[1], all[0])
        } else {
            continue;
        };
        let ni = dom_idx[&pr.net];
        let vrail = volt[ni];
        let field = match f.prop("PI_CAP").map(parse_pi_cap) {
            Some(Ok(x)) => Some(x),
            Some(Err(e)) => {
                warnings.push(format!("{}: {e}", f.reference));
                None
            }
            None => None,
        };
        let Some(c_nom) = field.as_ref().and_then(|x| x.c).or_else(|| parse_cap(&f.value)) else {
            unparsed += 1;
            continue;
        };
        let pitch = geom::len(geom::sub(pr.at, pg.at));
        let pkg = crate::package::chip_package(&f.lib, pr, pg);
        let (vr_text, diel_text) = value_details(&f.value);
        let dielectric = field.as_ref().and_then(|x| x.dielectric.clone()).unwrap_or(diel_text);
        let rated = field.as_ref().and_then(|x| x.rated).or(vr_text);
        let class2 = DecapData::class2(&dielectric, c_nom);
        let (rated_v, rated_assumed) = match rated {
            Some(v) => (Some(v), false),
            None if class2 => {
                assumed_rating += 1;
                (Some(data.rated_assumed(vrail)), true)
            }
            None => (None, false),
        };
        // capacitance at the rail voltage: datasheet points from PI_CAP, else the derating curve
        let c_eff = match field.as_ref().filter(|x| !x.c_at.is_empty()) {
            Some(x) => {
                let mut pts = vec![(0.0, c_nom)];
                pts.extend(x.c_at.iter().copied());
                pts.sort_by(|a, b| a.0.total_cmp(&b.0));
                let r: Vec<(f64, f64)> = pts.iter().map(|&(v, c)| (v, c / c_nom)).collect();
                c_nom * interp(&r, vrail).max(0.05)
            }
            None => c_nom * data.retention(pkg, c_nom, vrail, rated_v.unwrap_or(0.0), &dielectric),
        };
        let esl = field.as_ref().and_then(|x| x.esl).unwrap_or_else(|| data.esl(pkg));
        let esr = field.as_ref().and_then(|x| x.esr).unwrap_or_else(|| data.esr(pkg, c_nom, &dielectric));
        // ground side: pad → via trace, via down to the nearest ground copper
        let pl = pg.layers[0] as usize;
        let k_pg = g.cell_of(pg.at).map(|(i, j)| j * g.nx + i);
        let h_g = match k_pg {
            Some(k) if gw[pl][k] => 0.0,
            Some(k) => {
                let (up, dn) = ref_of(pl, k);
                [up, dn].into_iter().flatten().map(|x| x.1).fold(f64::INFINITY, f64::min)
            }
            None => f64::INFINITY,
        };
        let h_g = if h_g.is_finite() { h_g } else { fallback_ref(pl).1 };
        let gv = gnd_via_of(pg);
        let mut l_gnd = 0.0;
        if h_g > 0.0 {
            let r = gv.map_or(0.15, |v| (v.1 / 2.0).max(0.05));
            let s = pitch.max(0.5).max(2.0 * r + 0.1);
            l_gnd += MU0 * h_g * 1e-3 / (2.0 * PI) * (s / r).ln().max(0.2);
            l_gnd += match gv {
                Some((d, _)) => 0.4e-9 * d,
                None => 1.0e-9, // no via next to the pad: longer path, rough
            };
        }
        let nodes = pad_nodes(pr);
        let Some(&node) = nodes.first() else { continue };
        let node = if nodes.len() > 1 { add_group(&mut model, &nodes) } else { node };
        model.shunts.push((node, Shunt::Cap { r: esr, l: esl + l_gnd, c: c_eff }));
        let mut flags = Vec::new();
        if c_eff < 0.5 * c_nom {
            flags.push(format!("DC bias: {:.0} % of the nominal value at {:.2} V{}", 100.0 * c_eff / c_nom, vrail, if rated_assumed { " (rating assumed)" } else { "" }));
        }
        cap_node.push(node);
        cap_ni.push(ni);
        caps.push(CapResult {
            refdes: f.reference.clone(),
            net: board.net_name(pr.net).to_string(),
            value: f.value.clone(),
            package: pkg.into(),
            at: [(pr.at[0] + pg.at[0]) / 2.0, (pr.at[1] + pg.at[1]) / 2.0],
            bbox: fp_bbox(fi as u32),
            layer: pr.layers[0],
            c_nom,
            c_eff,
            dielectric,
            rated_v,
            rated_assumed,
            esr,
            esl,
            l_gnd,
            source: if field.is_some() { "PI_CAP".into() } else { "estimate".into() },
            l_path_min: None,
            nearest_ic: None,
            flags,
        });
    }
    if unparsed > 0 {
        warnings.push(format!("{unparsed} capacitors with unreadable value skipped"));
    }
    if assumed_rating > 0 {
        warnings.push(format!("{assumed_rating} class II capacitors without rated voltage: the smallest standard rating ≥ {:.1} × the net voltage assumed for the DC-bias derating (add it to the value or a PI_CAP field)", data.rated_v.margin));
    }
    if caps.is_empty() {
        warnings.push("no decoupling capacitors found between the rail domain and ground".into());
    }

    // --- Z(f) at every port ---
    let nf = p.ic_npts.max(20);
    let freq: Vec<f64> = (0..nf).map(|i| p.fmin.max(1e3) * (p.fmax / p.fmin.max(1e3)).powf(i as f64 / (nf - 1) as f64)).collect();
    let n = model.n;
    let mut zs: Vec<Vec<f64>> = vec![Vec::with_capacity(nf); ports.len()];
    let mut zcx: Vec<Vec<Cx>> = vec![Vec::with_capacity(nf); ports.len()];
    let rhs: Vec<Vec<(f64, f64)>> = ports
        .iter()
        .map(|pt| {
            let mut b = vec![(0.0, 0.0); n];
            b[pt.node] = (1.0, 0.0);
            b
        })
        .collect();
    let mut sym = None;
    for &f in &freq {
        let (lu, s) = kisi_sparse::ComplexLu::with_symbolic(n, &model.triplets(f, true, true, &[]), sym.take()).map_err(|e| format!("AC grid: solver error {e}"))?;
        sym = Some(s);
        let x = lu.solve(&rhs).map_err(|e| format!("AC grid: solver error {e}"))?;
        for (k, pt) in ports.iter().enumerate() {
            let v = x[k][pt.node];
            zs[k].push((v.0 * v.0 + v.1 * v.1).sqrt());
            zcx[k].push(Cx::new(v.0, v.1));
        }
    }

    // --- path inductance capacitor ↔ IC (inductive network, ground tied once per island) ---
    let f_l = 10e6;
    let wl = 2.0 * PI * f_l;
    let mut comp = vec![usize::MAX; n];
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
    for &(a, b, _) in &model.links {
        adj[a].push(b);
        adj[b].push(a);
    }
    let mut roots = Vec::new();
    for s in 0..n {
        if comp[s] != usize::MAX {
            continue;
        }
        comp[s] = roots.len();
        let mut q = VecDeque::from([s]);
        while let Some(u) = q.pop_front() {
            for &v in &adj[u] {
                if comp[v] == usize::MAX {
                    comp[v] = roots.len();
                    q.push_back(v);
                }
            }
        }
        roots.push(s);
    }
    // ICs whose pins are not joined to a capacitor or regulator through the copper
    {
        let fed: std::collections::HashSet<usize> = model.shunts.iter().filter(|(_, sh)| !matches!(sh, Shunt::Plane { .. })).map(|(i, _)| comp[*i]).collect();
        let size = |c: usize| comp.iter().filter(|&&x| x == c).count();
        for pt in &ports {
            if !fed.contains(&comp[pt.node]) {
                warnings.push(format!(
                    "{} on {}: its supply pins are not joined to a capacitor or regulator through the copper of the domain ({} grid nodes): check the connection (thin tracks below the {:.2} mm cell?)",
                    pt.refdes,
                    board.net_name(dom[pt.ni]),
                    size(comp[pt.node]),
                    cell
                ));
            }
        }
    }
    let lu = kisi_sparse::ComplexLu::new(n, &model.triplets(f_l, false, false, &roots)).map_err(|e| format!("AC grid: solver error {e}"))?;
    // G at the port and capacitor nodes, in batches
    let mut probe: Vec<usize> = ports.iter().map(|pt| pt.node).collect();
    probe.extend(cap_node.iter().copied());
    let mut gm: Vec<Vec<Cx>> = Vec::with_capacity(probe.len()); // gm[i][j] = V at probe j for 1 A into probe i
    for chunk in probe.chunks(32) {
        let b: Vec<Vec<(f64, f64)>> = chunk
            .iter()
            .map(|&node| {
                let mut v = vec![(0.0, 0.0); n];
                v[node] = (1.0, 0.0);
                v
            })
            .collect();
        let x = lu.solve(&b).map_err(|e| format!("AC grid: solver error {e}"))?;
        for col in x {
            gm.push(probe.iter().map(|&j| Cx::new(col[j].0, col[j].1)).collect());
        }
    }
    let np = ports.len();
    let mut out_ports = Vec::new();
    // capacitance per net (for the filter resonances of series inductors and beads)
    let mut c_net = vec![0.0f64; nd];
    for (ci, c) in caps.iter().enumerate() {
        c_net[cap_ni[ci]] += c.c_eff;
    }
    for (pi_, pt) in ports.iter().enumerate() {
        let vrail = volt[pt.ni];
        let mut links = Vec::new();
        for (ci, cap) in caps.iter().enumerate() {
            if cap_ni[ci] != pt.ni || comp[cap_node[ci]] != comp[pt.node] {
                continue;
            }
            let (a, b) = (pi_, np + ci);
            let z2 = gm[a][a] + gm[b][b] - gm[a][b] - gm[b][a];
            let l_path = (z2.im / wl).max(0.0);
            let l_total = l_path + cap.l_gnd + cap.esl;
            links.push(CapLink {
                cap: ci,
                refdes: cap.refdes.clone(),
                l_path,
                r_path: z2.re.max(0.0),
                l_total,
                srf_hz: 1.0 / (2.0 * PI * (l_total * cap.c_eff).sqrt()),
                dist_mm: geom::len(geom::sub(cap.at, pt.at)),
            });
        }
        links.sort_by(|x, y| x.l_total.total_cmp(&y.l_total));
        // spreading fit l_path = a + b·ln(r)
        let pts: Vec<(f64, f64)> = links.iter().map(|l| (l.dist_mm.max(0.5).ln(), l.l_path)).collect();
        let spread_fit = if pts.len() >= 3 {
            let m = pts.len() as f64;
            let (sx, sy) = (pts.iter().map(|q| q.0).sum::<f64>(), pts.iter().map(|q| q.1).sum::<f64>());
            let sxx = pts.iter().map(|q| q.0 * q.0).sum::<f64>();
            let sxy = pts.iter().map(|q| q.0 * q.1).sum::<f64>();
            let den = m * sxx - sx * sx;
            if den.abs() > 1e-12 {
                let b = ((m * sxy - sx * sy) / den).max(20e-12);
                Some(((sy - b * sx) / m, b))
            } else {
                None
            }
        } else {
            None
        };
        // target
        let (target, target_source, f_max) = match &pt.target {
            Some((z, fm, src)) => (Some(*z), src.clone(), fm.unwrap_or(crate::pdn::F_BOARD)),
            None if pt.current > 0.0 && vrail > 0.0 => (
                Some(vrail * p.ripple / (p.step * pt.current)),
                format!("{:.0} % ripple at a {:.0} % step of {:.0} mA (estimate)", p.ripple * 100.0, p.step * 100.0, pt.current * 1e3),
                crate::pdn::F_BOARD,
            ),
            None => (None, "no load current".into(), crate::pdn::F_BOARD),
        };
        let z = &zs[pi_];
        let mut above = Vec::new();
        if let Some(t) = target {
            let mut start: Option<usize> = None;
            for i in 0..nf {
                let bad = z[i] > t && freq[i] <= f_max;
                match (bad, start) {
                    (true, None) => start = Some(i),
                    (false, Some(s0)) => {
                        above.push((freq[s0], freq[i - 1]));
                        start = None;
                    }
                    _ => {}
                }
            }
            if let Some(s0) = start {
                above.push((freq[s0], freq.iter().copied().filter(|&f| f <= f_max).last().unwrap_or(freq[nf - 1])));
            }
        }
        // anti-resonances: local maxima with a prominence of ≥ 1.5 below f_max
        let mut peaks = Vec::new();
        for i in 1..nf - 1 {
            if freq[i] > f_max || !(z[i] > z[i - 1] && z[i] >= z[i + 1]) {
                continue;
            }
            // prominence against the deepest valley on either side, up to a higher point
            let valley = |it: &mut dyn Iterator<Item = f64>| it.take_while(|&v| v <= z[i]).fold(z[i], f64::min);
            let left = valley(&mut z[..i].iter().rev().copied());
            let right = valley(&mut z[i + 1..].iter().copied());
            if z[i] < 1.5 * left.max(right) {
                continue;
            }
            // capacitors resonating just below (inductive there) and above (still capacitive)
            let below = links.iter().filter(|l| l.srf_hz < freq[i]).max_by(|a, b| a.srf_hz.total_cmp(&b.srf_hz));
            let over = links.iter().filter(|l| l.srf_hz > freq[i]).min_by(|a, b| a.srf_hz.total_cmp(&b.srf_hz));
            let name = |l: &CapLink| format!("{} {} (series resonance {:.1} MHz)", l.refdes, fmt_c(caps[l.cap].c_eff), l.srf_hz / 1e6);
            let note = match (below, over) {
                (Some(a), Some(b)) => format!("between {} and {}", name(a), name(b)),
                (Some(a), None) => format!("inductance of the decoupling against the plane capacitance; highest series resonance: {}", name(a)),
                (None, Some(b)) => format!("regulator inductance against the capacitors; lowest series resonance: {}", name(b)),
                _ => "regulator against the plane capacitance".into(),
            };
            // an inductor or bead next to this net resonating with its capacitors (LC filter)
            let mut note = note;
            for (sr, &(ia, ib)) in series.iter().zip(&series_ni) {
                if sr.kind == "resistor" || (ia != pt.ni && ib != pt.ni) || c_net[pt.ni] <= 0.0 {
                    continue;
                }
                let f_lc = 1.0 / (2.0 * PI * (sr.l_low() * c_net[pt.ni]).sqrt());
                if (freq[i] / f_lc).ln().abs() < 1.6f64.ln() {
                    note = format!("filter resonance of {} ({}, {:.2} µH below its peak) with the {} on this net; {note}", sr.refdes, sr.kind, sr.l_low() * 1e6, fmt_c(c_net[pt.ni]));
                    break;
                }
            }
            peaks.push(Peak { f: freq[i], z: z[i], above_target: target.is_some_and(|t| z[i] > t), note });
        }
        // recommendations per band above the target
        let mut recs = Vec::new();
        if let Some(t) = target {
            let d_plane = d_eff.iter().copied().fold(0.0, f64::max).max(0.05);
            let l_sp = |r: f64| -> f64 {
                match spread_fit {
                    Some((a, b)) => (a + b * r.max(0.5).ln()).max(0.0),
                    None => MU0 * d_plane * 1e-3 / PI * (r / 0.15).max(1.0).ln(),
                }
            };
            // greedy: add the part that lowers the worst point most, one at a time, with the parts
            // seen at the IC in parallel to the computed Z (their own path at r_place); then the
            // largest distance each group may have without losing the result
            let ra = data.rated_assumed(vrail);
            let mount = data.mount_nh.value * 1e-9;
            let cands: Vec<(f64, &str, f64, f64, f64)> = data
                .candidates
                .iter()
                .filter_map(|cd| {
                    let c0 = parse_cap(&cd.value)?;
                    let pkg = cd.package.as_str();
                    Some((c0, pkg, c0 * data.retention(pkg, c0, vrail, ra, ""), data.esr(pkg, c0, ""), data.esl(pkg) + mount))
                })
                .collect();
            let band: Vec<usize> = (0..nf).filter(|&i| freq[i] <= f_max).collect();
            let y_part = |k: usize, i: usize, r_mm: f64| -> Cx {
                let (_, _, c, r, l) = cands[k];
                let w = 2.0 * PI * freq[i];
                Cx::new(r, w * (l + l_sp(r_mm)) - 1.0 / (w * c)).inv()
            };
            let r_place = 3.0; // mm: a placement next to the IC
            let z0: &Vec<Cx> = &zcx[pi_];
            let eval = |groups: &[(usize, usize, f64)]| -> Vec<f64> {
                band.iter()
                    .map(|&i| {
                        let mut y = z0[i].inv();
                        for &(k, n, r_mm) in groups {
                            y = y + y_part(k, i, r_mm).scale(n as f64);
                        }
                        y.inv().abs()
                    })
                    .collect()
            };
            let mut groups: Vec<(usize, usize, f64)> = Vec::new(); // (candidate, count, radius)
            let mut cur = eval(&groups);
            let mut parts = 0;
            while parts < 48 {
                let (wi, worst) = cur.iter().enumerate().fold((0, 0.0), |a, (i, &v)| if v > a.1 { (i, v) } else { a });
                if worst <= t {
                    break;
                }
                let mut best: Option<(usize, f64)> = None;
                for k in 0..cands.len() {
                    let mut g2 = groups.clone();
                    match g2.iter_mut().find(|g| g.0 == k) {
                        Some(g) => g.1 += 1,
                        None => g2.push((k, 1, r_place)),
                    }
                    let i = band[wi];
                    let mut y = z0[i].inv();
                    for &(kk, n, r_mm) in &g2 {
                        y = y + y_part(kk, i, r_mm).scale(n as f64);
                    }
                    let v = y.inv().abs();
                    if best.is_none_or(|b| v < b.1 - 1e-12) {
                        best = Some((k, v));
                    }
                }
                let Some((k, v)) = best else { break };
                if v >= worst * 0.999 {
                    break; // no part helps at this point any more
                }
                match groups.iter_mut().find(|g| g.0 == k) {
                    Some(g) => g.1 += 1,
                    None => groups.push((k, 1, r_place)),
                }
                parts += 1;
                cur = eval(&groups);
            }
            let reached = cur.iter().copied().fold(0.0, f64::max);
            // radius per group: as far as possible while the worst point stays within 5 %
            let limit = reached.max(t) * 1.05;
            for gi in 0..groups.len() {
                let (mut lo, mut hi) = (r_place, 50.0);
                let ok = |r: f64, groups: &Vec<(usize, usize, f64)>| {
                    let mut g2 = groups.clone();
                    g2[gi].2 = r;
                    eval(&g2).iter().copied().fold(0.0, f64::max) <= limit
                };
                if ok(hi, &groups) {
                    lo = hi;
                } else {
                    for _ in 0..12 {
                        let mid = 0.5 * (lo + hi);
                        if ok(mid, &groups) {
                            lo = mid;
                        } else {
                            hi = mid;
                        }
                    }
                }
                groups[gi].2 = lo;
            }
            groups.sort_by(|a, b| cands[b.0].0.total_cmp(&cands[a.0].0));
            for &(k, n, r_mm) in &groups {
                let (c0, pkg, c, _, l) = cands[k];
                let srf = 1.0 / (2.0 * PI * ((l + l_sp(r_mm)) * c).sqrt());
                let worst_in = |lo: f64, hi: f64| band.iter().zip(&cur).filter(|(&i, _)| freq[i] >= lo && freq[i] <= hi).map(|(_, &v)| v).fold(0.0, f64::max);
                let (fa, fb) = ((srf / 3.0).max(freq[0]), (srf * 3.0).min(f_max));
                recs.push(Recommendation {
                    f_lo: fa,
                    f_hi: fb,
                    z_max: worst_in(fa, fb),
                    value: fmt_c(c0),
                    package: pkg.into(),
                    count: n,
                    radius_mm: r_mm,
                    l_each: l + l_sp(r_mm),
                    note: format!("{} effective each at {:.1} V (DC bias, rating {:.1} V assumed), series resonance {:.1} MHz at that distance", fmt_c(c), vrail, ra, srf / 1e6),
                });
            }
            if reached > t {
                let bad: Vec<f64> = band.iter().zip(&cur).filter(|(_, &v)| v > t).map(|(&i, _)| freq[i]).collect();
                let (fa, fb) = (bad[0], *bad.last().unwrap());
                recs.push(Recommendation {
                    f_lo: fa,
                    f_hi: fb,
                    z_max: reached,
                    value: String::new(),
                    package: String::new(),
                    count: 0,
                    radius_mm: 0.0,
                    l_each: 0.0,
                    note: if fb < 1e6 {
                        format!("still {:.1} mΩ above the target after {parts} parts: bulk capacitance (polymer / electrolytic) or the regulator's control loop covers this range", (reached - t) * 1e3)
                    } else {
                        format!("still {:.1} mΩ above the target after {parts} parts: plane capacitance, package / die capacitance, or check the target (current step)", (reached - t) * 1e3)
                    },
                });
            }
        }
        out_ports.push(PortResult {
            refdes: pt.refdes.clone(),
            net: board.net_name(dom[pt.ni]).to_string(),
            label: pt.refdes.clone(),
            at: pt.at,
            bbox: fp_bbox(pt.fi),
            current: pt.current,
            pins: pt.pins,
            z: zs[pi_].clone(),
            target,
            target_source,
            f_max,
            above,
            caps: links,
            peaks,
            recommendations: recs,
            spread_fit,
        });
    }
    // capacitor flags: nearest IC, far from every IC (high-frequency parts whose path is more than
    // twice their own ESL + mounting and more than twice the best path on this rail), not connected
    // the same IC on several nets of the domain: label with the net
    for i in 0..out_ports.len() {
        if out_ports.iter().filter(|x| x.refdes == out_ports[i].refdes).count() > 1 {
            let short = out_ports[i].net.rsplit('/').next().unwrap_or("").to_string();
            out_ports[i].label = format!("{} ({short})", out_ports[i].refdes);
        }
    }
    let best_path: HashMap<String, f64> = out_ports.iter().map(|pt| (pt.label.clone(), pt.caps.iter().map(|l| l.l_path).fold(f64::INFINITY, f64::min))).collect();
    for (ci, cap) in caps.iter_mut().enumerate() {
        let best = out_ports.iter().filter_map(|pt| pt.caps.iter().find(|l| l.cap == ci).map(|l| (l.l_path, pt.label.clone()))).min_by(|a, b| a.0.total_cmp(&b.0));
        match best {
            Some((l, ic)) => {
                let own = cap.esl + cap.l_gnd;
                let ref_l = best_path.get(&ic).copied().unwrap_or(0.0);
                if cap.c_eff < 1e-6 && l > 2.0 * own && l > 2.0 * ref_l && l > 0.5e-9 {
                    cap.flags.push(format!(
                        "far from every IC: path inductance {:.2} nH to {ic} (best capacitor at that IC {:.2} nH, own ESL + mounting {:.2} nH); its series resonance drops to {:.1} MHz",
                        l * 1e9,
                        ref_l * 1e9,
                        own * 1e9,
                        1.0 / (2.0 * PI * ((l + own) * cap.c_eff).sqrt()) / 1e6
                    ));
                }
                cap.l_path_min = Some(l);
                cap.nearest_ic = Some(ic);
            }
            // no IC on its net: a filter capacitor (e.g. at a regulator output before a bead)
            None if out_ports.iter().any(|pt| pt.net == cap.net) => cap.flags.push("not connected to a load IC through the rail copper".into()),
            None => {}
        }
    }
    Ok(IcPdnResult {
        net: board.net_name(net).to_string(),
        voltage: setup.voltage,
        freq,
        ports: out_ports,
        caps,
        cell_mm: cell,
        layers: layers.iter().map(|&l| l as u16).collect(),
        nodes: n,
        regulators,
        domain: dom
            .iter()
            .enumerate()
            .map(|(ni, &dn)| DomainNet {
                net: board.net_name(dn).to_string(),
                voltage: volt[ni],
                caps: cap_ni.iter().filter(|&&x| x == ni).count(),
                c_total: c_net[ni],
                ics: ports.iter().filter(|pt| pt.ni == ni).map(|pt| pt.refdes.clone()).collect(),
                regulators: reg_by_net[ni].clone(),
            })
            .collect(),
        series,
        boundary,
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_fields() {
        let near = |a: Option<f64>, b: f64| a.is_some_and(|a| (a - b).abs() <= 1e-9 * b.abs());
        assert!(near(si("5m"), 5e-3));
        assert!(near(si("0.45nH"), 0.45e-9));
        assert!(near(si("6.3V"), 6.3));
        assert!(near(si("50MHz"), 50e6));
        assert!((si("10mOhm").unwrap() - 0.01).abs() < 1e-12);
        let f = parse_pi_cap("esr=5m esl=0.45n rated=6.3V X5R c@3.3V=2.1u").unwrap();
        assert!(near(f.esr, 5e-3) && near(f.esl, 0.45e-9) && near(f.rated, 6.3));
        assert_eq!(f.dielectric.as_deref(), Some("X5R"));
        assert!(f.c_at.len() == 1 && near(Some(f.c_at[0].0), 3.3) && near(Some(f.c_at[0].1), 2.1e-6));
        assert!(parse_pi_cap("foo").is_err());
        let t = parse_pi_target("VCCINT=10m fmax=50M; +3V3=30m", "+3V3").unwrap().unwrap();
        assert!(near(Some(t.0), 0.03) && t.1.is_none());
        let t = parse_pi_target("VCCINT=10m fmax=50M", "VCCINT").unwrap().unwrap();
        assert!(near(Some(t.0), 0.01) && near(t.1, 50e6));
        assert!(near(parse_pi_target("15mOhm", "X").unwrap().map(|t| t.0), 0.015));
        assert!(parse_pi_target("VCCINT=10m", "+3V3").unwrap().is_none());
        assert_eq!(value_details("10uF 6.3V X5R"), (Some(6.3), "X5R".into()));
        assert_eq!(value_details("100n/16V/X7R"), (Some(16.0), "X7R".into()));
        assert_eq!(value_details("4u7 6V3"), (Some(6.3), String::new()));
    }

    #[test]
    fn derating() {
        let d = DecapData::builtin();
        // 10 µF 0805 6.3 V at 3.3 V: high-CV curve, 52 % of rating → about 0.68 (data: 0.68–0.88)
        let r = d.retention("0805", 10e-6, 3.3, 6.3, "X5R");
        assert!(r > 0.6 && r < 0.75, "{r}");
        assert_eq!(d.retention("0402", 100e-12, 3.3, 50.0, "C0G"), 1.0);
        assert!(d.retention("0402", 100e-9, 3.3, 16.0, "X7R") > 0.9);
        assert_eq!(d.rated_assumed(3.3), 6.3);
        assert_eq!(d.rated_assumed(1.0), 4.0);
    }
}

#[cfg(test)]
mod grid_tests {
    use super::*;
    use crate::pi::{LoadSpec, SourceSpec};

    /// VCC pour on F.Cu over a GND plane (In1, 0.2 mm), 100 × 60 mm; U1 in the middle, C1 5 mm
    /// and C2 20 mm away, regulator U2 at the edge.
    fn board(extra: &str) -> Board {
        let cap = |r: &str, x: f64, v: &str| {
            format!(
                r#"(footprint "C" (layer "F.Cu") (at {x} 30) (property "Reference" "{r}") (property "Value" "{v}")
    (pad "1" smd rect (at -0.5 0) (size 0.5 0.6) (layers "F.Cu") (net 1 "+3V3"))
    (pad "2" smd rect (at 0.5 0) (size 0.5 0.6) (layers "F.Cu") (net 2 "GND")))
  (via (at {} 30) (size 0.45) (drill 0.2) (layers "F.Cu" "B.Cu") (net 2))"#,
                x + 0.9
            )
        };
        let pcb = format!(
            r#"(kicad_pcb (version 20240108) (generator "pcbnew")
  (general (thickness 1.6))
  (layers (0 "F.Cu" signal) (1 "In1.Cu" signal) (31 "B.Cu" signal))
  (setup (stackup
    (layer "F.Cu" (type "copper") (thickness 0.035))
    (layer "dielectric 1" (type "prepreg") (thickness 0.2) (material "FR4") (epsilon_r 4.3) (loss_tangent 0.02))
    (layer "In1.Cu" (type "copper") (thickness 0.035))
    (layer "dielectric 2" (type "core") (thickness 1.2) (material "FR4") (epsilon_r 4.5) (loss_tangent 0.02))
    (layer "B.Cu" (type "copper") (thickness 0.035))))
  (net 0 "") (net 1 "+3V3") (net 2 "GND")
  (footprint "U" (layer "F.Cu") (at 50 30) (property "Reference" "U1") (property "Value" "FPGA") (property "PI_LOAD" "+3V3=1A") {extra}
    (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net 1 "+3V3")))
  (footprint "U" (layer "F.Cu") (at 2 30) (property "Reference" "U2") (property "Value" "REG") (property "PI_SOURCE" "+3V3=3.3V")
    (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net 1 "+3V3")))
  {}
  {}
  (zone (net 1) (net_name "+3V3") (layer "F.Cu") (filled_polygon (layer "F.Cu") (pts (xy -50 -30) (xy 150 -30) (xy 150 90) (xy -50 90))))
  (zone (net 2) (net_name "GND") (layer "In1.Cu") (filled_polygon (layer "In1.Cu") (pts (xy -50 -30) (xy 150 -30) (xy 150 90) (xy -50 90)))))
"#,
            cap("C1", 55.0, "10uF 6.3V X5R"),
            cap("C2", 70.0, "100nF 16V X7R")
        );
        crate::load_pcb(&pcb).unwrap()
    }

    fn setup() -> RailSetup {
        RailSetup {
            net: "+3V3".into(),
            voltage: 3.3,
            sources: vec![SourceSpec { refdes: "U2".into(), pads: vec![] }],
            loads: vec![LoadSpec { refdes: "U1".into(), current: 1.0, pads: vec![], origin: "field".into() }],
            ground: false,
        }
    }

    #[test]
    fn plane_inductance_and_ranking() {
        let b = board("");
        let r = pdn_ic(&b, &setup(), &[], &PdnParams { ic_cells: 40000, ..Default::default() }).unwrap();
        let pt = &r.ports[0];
        let l1 = pt.caps.iter().find(|l| l.refdes == "C1").unwrap();
        let l2 = pt.caps.iter().find(|l| l.refdes == "C2").unwrap();
        eprintln!("cell {} mm, nodes {}, C1 {:.1} pH at {:.1} mm, C2 {:.1} pH at {:.1} mm", r.cell_mm, r.nodes, l1.l_path * 1e12, l1.dist_mm, l2.l_path * 1e12, l2.dist_mm);
        // plane pair: ΔL = μ0·d/π·ln(D2/D1) for points far from the edges (8 % above it on this grid;
        // on the 100 × 60 mm plane the open edges add another 20 %)
        let dl = MU0 * 0.2e-3 / PI * (l2.dist_mm / l1.dist_mm).ln();
        assert!(((l2.l_path - l1.l_path) - dl).abs() < 0.15 * dl, "ΔL {} vs {}", l2.l_path - l1.l_path, dl);
        assert_eq!(pt.caps[0].refdes, "C1", "nearest first");
        // derating: 10 µF 0805? pitch 1 mm → 0402, high CV at 3.3 V of 6.3 V
        let c1 = &r.caps[pt.caps[0].cap];
        assert!(c1.c_eff < 0.75 * c1.c_nom && !c1.rated_assumed, "{c1:?}");
        // Z at 1 kHz ≈ regulator R (2 mΩ) + plane; target 3.3·3 %/(0.5·1 A) = 0.198 Ω
        assert!((pt.target.unwrap() - 0.198).abs() < 1e-6);
        assert!(pt.z[0] < 0.01, "{}", pt.z[0]);
        // a tight target gives a recommendation
        let b2 = board(r#"(property "PI_TARGET" "+3V3=5m fmax=100M")"#);
        let r2 = pdn_ic(&b2, &setup(), &[], &PdnParams::default()).unwrap();
        let p2 = &r2.ports[0];
        assert!(p2.target_source.starts_with("PI_TARGET"));
        assert!(!p2.above.is_empty() && !p2.recommendations.is_empty(), "{:?}", p2.above);
        for rc in &p2.recommendations {
            eprintln!("{:.2}-{:.2} MHz: {} × {} {} within {:.1} mm ({})", rc.f_lo / 1e6, rc.f_hi / 1e6, rc.count, rc.value, rc.package, rc.radius_mm, rc.note);
        }
        for pk in &p2.peaks {
            eprintln!("peak {:.2} MHz {:.1} mΩ: {}", pk.f / 1e6, pk.z * 1e3, pk.note);
        }
    }

    /// +3V3 (regulator U2, switch node SW through L1) feeds +3V3A through the ferrite FB1; the
    /// load U1 and its capacitors sit on +3V3A.
    fn filtered(bead_value: &str) -> Board {
        let cap = |r: &str, x: f64, v: &str, net: (u32, &str)| {
            format!(
                r#"(footprint "C" (layer "F.Cu") (at {x} 30) (property "Reference" "{r}") (property "Value" "{v}")
    (pad "1" smd rect (at -0.5 0) (size 0.5 0.6) (layers "F.Cu") (net {} "{}"))
    (pad "2" smd rect (at 0.5 0) (size 0.5 0.6) (layers "F.Cu") (net 2 "GND")))
  (via (at {} 30) (size 0.45) (drill 0.2) (layers "F.Cu" "B.Cu") (net 2))"#,
                net.0,
                net.1,
                x + 0.9
            )
        };
        let pcb = format!(
            r#"(kicad_pcb (version 20240108) (generator "pcbnew")
  (general (thickness 1.6))
  (layers (0 "F.Cu" signal) (1 "In1.Cu" signal) (31 "B.Cu" signal))
  (setup (stackup
    (layer "F.Cu" (type "copper") (thickness 0.035))
    (layer "dielectric 1" (type "prepreg") (thickness 0.2) (material "FR4") (epsilon_r 4.3) (loss_tangent 0.02))
    (layer "In1.Cu" (type "copper") (thickness 0.035))
    (layer "dielectric 2" (type "core") (thickness 1.2) (material "FR4") (epsilon_r 4.5) (loss_tangent 0.02))
    (layer "B.Cu" (type "copper") (thickness 0.035))))
  (net 0 "") (net 1 "+3V3") (net 2 "GND") (net 3 "+3V3A") (net 4 "SW")
  (footprint "U" (layer "F.Cu") (at 70 30) (property "Reference" "U1") (property "Value" "ADC")
    (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net 3 "+3V3A")))
  (footprint "U" (layer "F.Cu") (at 2 30) (property "Reference" "U2") (property "Value" "REG") (property "PI_REG" "x")
    (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net 1 "+3V3"))
    (pad "2" smd rect (at 0 -3) (size 1 1) (layers "F.Cu") (net 4 "SW")))
  (footprint "L" (layer "F.Cu") (at 6 27) (property "Reference" "L1") (property "Value" "2.2uH")
    (pad "1" smd rect (at -1 0) (size 1 1) (layers "F.Cu") (net 4 "SW"))
    (pad "2" smd rect (at 1 0) (size 1 1) (layers "F.Cu") (net 1 "+3V3")))
  (footprint "L" (layer "F.Cu") (at 42.5 30) (property "Reference" "FB1") (property "Value" "{bead_value}")
    (pad "1" smd rect (at -0.8 0) (size 0.8 0.9) (layers "F.Cu") (net 1 "+3V3"))
    (pad "2" smd rect (at 0.8 0) (size 0.8 0.9) (layers "F.Cu") (net 3 "+3V3A")))
  {}
  {}
  {}
  (zone (net 1) (net_name "+3V3") (layer "F.Cu") (filled_polygon (layer "F.Cu") (pts (xy -20 10) (xy 41.5 10) (xy 41.5 50) (xy -20 50))))
  (zone (net 3) (net_name "+3V3A") (layer "F.Cu") (filled_polygon (layer "F.Cu") (pts (xy 43.5 10) (xy 100 10) (xy 100 50) (xy 43.5 50))))
  (zone (net 2) (net_name "GND") (layer "In1.Cu") (filled_polygon (layer "In1.Cu") (pts (xy -20 10) (xy 100 10) (xy 100 50) (xy -20 50)))))
"#,
            cap("C10", 10.0, "22uF 10V X5R", (1, "+3V3")),
            cap("C1", 74.0, "1uF 10V X7R", (3, "+3V3A")),
            cap("C2", 66.0, "100nF 16V X7R", (3, "+3V3A"))
        );
        crate::load_pcb(&pcb).unwrap()
    }

    fn rails() -> (RailSetup, RailSetup) {
        let main = RailSetup { net: "+3V3".into(), voltage: 3.3, sources: vec![SourceSpec { refdes: "U2".into(), pads: vec!["1".into()] }], loads: vec![], ground: false };
        let filt = RailSetup {
            net: "+3V3A".into(),
            voltage: 3.3,
            sources: vec![SourceSpec { refdes: "FB1".into(), pads: vec![] }],
            loads: vec![LoadSpec { refdes: "U1".into(), current: 0.2, pads: vec![], origin: "field".into() }],
            ground: false,
        };
        (main, filt)
    }

    #[test]
    fn series_parts_up_to_the_regulator() {
        assert_eq!(parse_resistance("4R7"), Some(4.7));
        assert_eq!(parse_resistance("0R"), Some(0.0));
        assert_eq!(parse_resistance("2k2"), Some(2200.0));
        assert_eq!(parse_resistance("10mR"), Some(0.01));
        assert_eq!(parse_resistance("FERRITE"), None);
        assert_eq!(bead_z100("600R@100MHz"), Some(600.0));
        assert_eq!(bead_z100("BLM18PG221SN1"), Some(220.0));
        assert_eq!(parse_pi_series("z100=600 rdc=0.15").unwrap().z100, Some(600.0));
        let (main, filt) = rails();
        let b = filtered("FERRITE");
        let r = pdn_ic(&b, &filt, &[main.clone(), filt.clone()], &PdnParams::default()).unwrap();
        // domain: +3V3A, then +3V3 through FB1; SW is a switch node (pin of U2, no rail)
        assert_eq!(r.domain.iter().map(|d| d.net.as_str()).collect::<Vec<_>>(), ["+3V3A", "+3V3"]);
        assert!(r.boundary.iter().any(|x| x.contains("L1") && x.contains("SW")), "{:?}", r.boundary);
        assert_eq!(r.regulators, ["U2"], "FB1 is a series part, not the source");
        let fb = &r.series[0];
        assert!(fb.refdes == "FB1" && fb.kind == "bead" && fb.source == "estimate" && (fb.z100 - 220.0).abs() < 60.0, "{fb:?}");
        // the regulator is reached through the bead: low impedance at 1 kHz (bead Rdc + regulator)
        let pt = &r.ports[0];
        assert_eq!((pt.refdes.as_str(), pt.net.as_str()), ("U1", "+3V3A"));
        assert!(pt.z[0] < 0.2, "Z(1 kHz) {}", pt.z[0]);
        // only the capacitors of its own net are ranked; the LC resonance of the bead is named
        assert!(pt.caps.iter().all(|l| l.refdes != "C10"));
        let lc = pt.peaks.iter().find(|k| k.note.contains("FB1")).unwrap_or_else(|| panic!("{:?}", pt.peaks));
        eprintln!("filter resonance {:.0} kHz {:.2} Ω: {}", lc.f / 1e3, lc.z, lc.note);
        assert!(lc.f > 100e3 && lc.f < 1e6);
        // without the other rails the domain still follows FB1, but has no regulator
        let r0 = pdn_ic(&b, &filt, &[], &PdnParams::default()).unwrap();
        assert_eq!(r0.domain.len(), 2);
        // from the main rail: the IC behind the bead is a port too
        let rm = pdn_ic(&b, &main, &[main.clone(), filt.clone()], &PdnParams::default()).unwrap();
        assert!(rm.ports.iter().any(|x| x.refdes == "U1" && x.net == "+3V3A"));
        // a rated bead: impedance from the value
        let r6 = pdn_ic(&filtered("600R@100MHz"), &filt, &[main, filt.clone()], &PdnParams::default()).unwrap();
        assert!(r6.series[0].source == "value" && (r6.series[0].z100 - 600.0).abs() < 150.0, "{:?}", r6.series[0]);
    }
}

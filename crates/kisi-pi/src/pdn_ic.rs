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
#[derive(Deserialize)]
struct Wrap {
    decoupling: DecapData,
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
    pub value: String,
    pub package: String,
    pub at: P,
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
    pub at: P,
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
    pub warnings: Vec<String>,
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

fn package_of(pitch: f64) -> &'static str {
    match pitch {
        p if p < 0.65 => "0201",
        p if p < 1.25 => "0402",
        p if p < 1.95 => "0603",
        p if p < 2.7 => "0805",
        p if p < 3.8 => "1206",
        p if p < 4.6 => "1210",
        _ => "large",
    }
}

fn fmt_c(c: f64) -> String {
    let (v, u) = if c >= 1e-6 { (c * 1e6, "µF") } else if c >= 1e-9 { (c * 1e9, "nF") } else { (c * 1e12, "pF") };
    format!("{}{}", if v >= 10.0 { format!("{v:.0}") } else { format!("{v:.2}").trim_end_matches('0').trim_end_matches('.').to_string() }, u)
}

/// PDN of `setup` seen from every load IC.
pub fn pdn_ic(board: &Board, setup: &RailSetup, p: &PdnParams) -> Result<IcPdnResult, String> {
    let data = DecapData::builtin();
    let su = board.stackup();
    let ncu = su.n();
    let net = find_net(board, &setup.net).ok_or_else(|| format!("net {} not found", setup.net))?;
    let vrail = setup.voltage.abs();
    let mut warnings: Vec<String> = Vec::new();
    let is_gnd = |n: u32| n != 0 && is_ground_name(board.net_name(n));

    // copper of the rail
    let mut bb = BBox::empty();
    let mut used = vec![false; ncu];
    let mut cu_area = 0.0;
    for z in board.zones.iter().filter(|z| z.net == net) {
        bb.add(z.bbox.min);
        bb.add(z.bbox.max);
        used[z.layer as usize] = true;
        cu_area += polygon_area(&z.pts);
    }
    for t in board.tracks.iter().filter(|t| t.net == net && !t.pad) {
        for q in [t.a, t.b] {
            bb.add([q[0] - t.width, q[1] - t.width]);
            bb.add([q[0] + t.width, q[1] + t.width]);
        }
        used[t.layer as usize] = true;
        cu_area += geom::len(geom::sub(t.b, t.a)) * t.width;
    }
    for pd in board.pads.iter().filter(|q| q.net == net) {
        let r = pd.size[0].max(pd.size[1]);
        bb.add([pd.at[0] - r, pd.at[1] - r]);
        bb.add([pd.at[0] + r, pd.at[1] + r]);
        for &l in &pd.layers {
            used[l as usize] = true;
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
    if per * ncu > 40_000_000 {
        return Err(format!("{}: grid too large", setup.net));
    }
    // rail copper weight per layer, ground copper per layer (all layers)
    let mut w: Vec<Vec<f32>> = vec![Vec::new(); ncu];
    for &l in &layers {
        let mut wl = vec![0f32; per];
        for z in board.zones.iter().filter(|z| z.net == net && z.layer as usize == l) {
            fill_polygon(&g, &z.pts, &mut wl);
        }
        for t in board.tracks.iter().filter(|t| t.net == net && !t.pad && t.layer as usize == l) {
            fill_track(&g, t.a, t.b, t.width, &mut wl);
        }
        for pd in board.pads.iter().filter(|q| q.net == net && q.layers.contains(&(l as u16))) {
            for k in pad_cells(&g, pd) {
                wl[k] = 1.0;
            }
        }
        w[l] = wl;
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

    // nodes: rail cells
    let mut node_of: Vec<Vec<u32>> = vec![Vec::new(); ncu];
    let mut node_pos: Vec<(usize, usize)> = Vec::new();
    for &l in &layers {
        let mut m = vec![u32::MAX; per];
        for k in 0..per {
            if w[l][k] > 0.0 {
                m[k] = node_pos.len() as u32;
                node_pos.push((l, k));
            }
        }
        node_of[l] = m;
    }
    let ncell = node_pos.len();
    let mut model = Model { n: ncell, links: Vec::new(), shunts: Vec::new() };
    let mut d_eff = vec![0.0f64; ncell];
    let mut no_ref = 0usize;
    let a_cell = (cell * 1e-3).powi(2);
    for (i, &(l, k)) in node_pos.iter().enumerate() {
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
            let cj = EPS0 * su.er_between(l, j) * a_cell * w[l][k] as f64 / (d * 1e-3);
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
    for &l in &layers {
        let t_m = su.copper[l].thickness * 1e-3;
        let (wl, m) = (&w[l], &node_of[l]);
        for j in 0..g.ny {
            for i in 0..g.nx {
                let k = j * g.nx + i;
                if m[k] == u32::MAX {
                    continue;
                }
                for (ni, nj) in [(i + 1, j), (i, j + 1)] {
                    if ni >= g.nx || nj >= g.ny {
                        continue;
                    }
                    let k2 = nj * g.nx + ni;
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
    // vias and plated holes of the rail
    let gnd_vias: Vec<P> = board.vias.iter().filter(|v| is_gnd(v.net)).map(|v| v.at).chain(board.pads.iter().filter(|q| is_gnd(q.net) && q.drill > 0.0).map(|q| q.at)).collect();
    let near_gnd_via = |at: P| -> Option<f64> { gnd_vias.iter().map(|q| geom::len(geom::sub(*q, at))).filter(|&d| d < 3.0).min_by(|a, b| a.total_cmp(b)) };
    let mut holes: Vec<(P, f64, u16, u16)> = board.vias.iter().filter(|v| v.net == net).map(|v| (v.at, v.drill, v.from, v.to)).collect();
    for q in board.pads.iter().filter(|q| q.net == net && q.drill > 0.0 && q.layers.len() > 1) {
        holes.push((q.at, q.drill, *q.layers.iter().min().unwrap(), *q.layers.iter().max().unwrap()));
    }
    let mut n_via_links = 0;
    for &(at, drill, from, to) in &holes {
        let Some((ci, cj)) = g.cell_of(at) else { continue };
        let k = cj * g.nx + ci;
        let on: Vec<usize> = layers.iter().copied().filter(|&l| l as u16 >= from && l as u16 <= to && node_of[l][k] != u32::MAX).collect();
        let r = (drill / 2.0).max(0.05);
        let s = near_gnd_via(at).unwrap_or(1.5).clamp(2.0 * r + 0.1, 3.0);
        for pair in on.windows(2) {
            let h = su.gap_between(pair[0], pair[1]).max(0.01) * 1e-3;
            let l = MU0 * h / (2.0 * PI) * (s / r).ln().max(0.2);
            let rv = RHO * h / (PI * drill * 1e-3 * 25e-6);
            model.links.push((node_of[pair[0]][k] as usize, node_of[pair[1]][k] as usize, Link::Via { l, r: rv }));
            n_via_links += 1;
        }
    }
    let _ = n_via_links;

    // footprints → pads on the rail
    let mut fp_pads: HashMap<u32, Vec<&Pad>> = HashMap::new();
    for pd in board.pads.iter().filter(|q| q.net == net) {
        fp_pads.entry(pd.fp).or_default().push(pd);
    }
    let fp_by_ref: HashMap<&str, u32> = board.footprints.iter().enumerate().map(|(i, f)| (f.reference.as_str(), i as u32)).collect();
    let pad_nodes = |pd: &Pad| -> Vec<usize> {
        let mut out = Vec::new();
        for &l in &pd.layers {
            if !used[l as usize] {
                continue;
            }
            for k in pad_cells(&g, pd) {
                let n = node_of[l as usize][k];
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

    // regulators
    let mut regulators = Vec::new();
    for src in &setup.sources {
        let Some(&fi) = fp_by_ref.get(src.refdes.as_str()) else { continue };
        let pads: Vec<&Pad> = fp_pads.get(&fi).map(|v| v.iter().copied().filter(|q| src.pads.is_empty() || src.pads.contains(&q.number)).collect()).unwrap_or_default();
        let nodes: Vec<usize> = pads.iter().flat_map(|q| pad_nodes(q)).collect();
        if nodes.is_empty() {
            continue;
        }
        let id = add_group(&mut model, &nodes);
        model.shunts.push((id, Shunt::Vrm { r: p.vrm_r, l: p.vrm_l }));
        regulators.push(src.refdes.clone());
    }
    if regulators.is_empty() {
        warnings.push("no regulator / source on this rail: the low-frequency end is open (capacitors only)".into());
    }

    // load ICs (ports): one per component, currents summed
    let mut loads: Vec<(String, f64)> = Vec::new();
    // ports are the ICs: load points on series parts (resistors, beads, inductors), connectors,
    // diodes and test points are passed through, not decoupled
    let passive = |r: &str| {
        let p: String = r.chars().take_while(|c| c.is_ascii_alphabetic()).collect::<String>().to_ascii_uppercase();
        matches!(p.as_str(), "R" | "RN" | "FB" | "L" | "F" | "J" | "P" | "CN" | "X" | "TP" | "D" | "SW" | "K" | "BT" | "C")
    };
    for l in setup.loads.iter().filter(|l| !passive(&l.refdes)) {
        match loads.iter_mut().find(|x| x.0 == l.refdes) {
            Some(x) => x.1 += l.current.abs(),
            None => loads.push((l.refdes.clone(), l.current.abs())),
        }
    }
    struct Port {
        refdes: String,
        node: usize,
        at: P,
        current: f64,
        pins: usize,
        target: Option<(f64, Option<f64>, String)>,
    }
    let mut ports: Vec<Port> = Vec::new();
    for (refdes, current) in &loads {
        let Some(&fi) = fp_by_ref.get(refdes.as_str()) else { continue };
        let pads: Vec<&Pad> = fp_pads.get(&fi).cloned().unwrap_or_default();
        let nodes: Vec<usize> = pads.iter().flat_map(|q| pad_nodes(q)).collect();
        if nodes.is_empty() {
            continue;
        }
        let at = [pads.iter().map(|q| q.at[0]).sum::<f64>() / pads.len() as f64, pads.iter().map(|q| q.at[1]).sum::<f64>() / pads.len() as f64];
        let fp = &board.footprints[fi as usize];
        let target = match fp.prop("PI_TARGET").map(|v| parse_pi_target(v, &setup.net)) {
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
        ports.push(Port { refdes: refdes.clone(), node: id, at, current: *current, pins: pads.len(), target });
    }
    if ports.is_empty() {
        return Err(format!("{}: no load IC on the rail (its loads are series parts, connectors or missing PI_LOAD): nothing to decouple here", setup.net));
    }

    // decoupling capacitors
    let mut caps: Vec<CapResult> = Vec::new();
    let mut cap_node: Vec<usize> = Vec::new();
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
        let (pr, pg) = if all[0].net == net && is_gnd(all[1].net) {
            (all[0], all[1])
        } else if all[1].net == net && is_gnd(all[0].net) {
            (all[1], all[0])
        } else {
            continue;
        };
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
        let pkg = package_of(pitch);
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
        caps.push(CapResult {
            refdes: f.reference.clone(),
            value: f.value.clone(),
            package: pkg.into(),
            at: [(pr.at[0] + pg.at[0]) / 2.0, (pr.at[1] + pg.at[1]) / 2.0],
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
        warnings.push(format!("{assumed_rating} class II capacitors without rated voltage: {:.1} V assumed for the DC-bias derating (add it to the value or a PI_CAP field)", data.rated_assumed(vrail)));
    }
    if caps.is_empty() {
        warnings.push("no decoupling capacitors found between rail and ground".into());
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
    for (pi_, pt) in ports.iter().enumerate() {
        let mut links = Vec::new();
        for (ci, cap) in caps.iter().enumerate() {
            if comp[cap_node[ci]] != comp[pt.node] {
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
            at: pt.at,
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
    let best_path: HashMap<String, f64> = out_ports.iter().map(|pt| (pt.refdes.clone(), pt.caps.iter().map(|l| l.l_path).fold(f64::INFINITY, f64::min))).collect();
    for (ci, cap) in caps.iter_mut().enumerate() {
        let best = out_ports.iter().filter_map(|pt| pt.caps.iter().find(|l| l.cap == ci).map(|l| (l.l_path, pt.refdes.clone()))).min_by(|a, b| a.0.total_cmp(&b.0));
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
            None => cap.flags.push("not connected to a load IC through the rail copper".into()),
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
        let r = pdn_ic(&b, &setup(), &PdnParams { ic_cells: 40000, ..Default::default() }).unwrap();
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
        let r2 = pdn_ic(&b2, &setup(), &PdnParams::default()).unwrap();
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
}

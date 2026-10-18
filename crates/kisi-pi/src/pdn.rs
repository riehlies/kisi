//! AC impedance of a supply rail (PI-AC, first version): lumped PDN model.
//!
//! Z(f) seen by the loads = VRM ∥ plane capacitance ∥ every decoupling
//! capacitor, each capacitor with ESR, ESL, mounting inductance (pad → via →
//! plane pair) and spreading inductance in the plane pair up to the load
//! centroid. Capacitor values come from the footprint value, ESR/ESL from
//! package-size estimates. Valid below the first cavity resonance of the plane,
//! which is reported. Design check, not a signoff PDN simulation.

use crate::board::Board;
use crate::geom::{self, P};
use crate::pi::{find_net, is_ground_name, RailSetup};
use crate::num::{Cx, MU0, EPS0, C0};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::f64::consts::PI;

/// upper end of the range the board decoupling is judged in (Hz)
pub const F_BOARD: f64 = 100e6;

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct PdnParams {
    pub fmin: f64,
    pub fmax: f64,
    pub npts: usize,
    /// regulator output: series R (Ω) and L (H)
    pub vrm_r: f64,
    pub vrm_l: f64,
    /// allowed ripple (fraction of the rail voltage) for the target impedance
    pub ripple: f64,
    /// load step as a fraction of the total load current
    pub step: f64,
    /// cells of the plane grid model (0 = off)
    pub max_cells: usize,
}

impl Default for PdnParams {
    fn default() -> Self {
        PdnParams { fmin: 1e3, fmax: 1e9, npts: 241, vrm_r: 2e-3, vrm_l: 20e-9, ripple: 0.03, step: 0.5, max_cells: 2500 }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CapInfo {
    pub refdes: String,
    pub value: String,
    pub c: f64,
    pub package: &'static str,
    pub layer: u16,
    pub at: P,
    pub esr: f64,
    /// body ESL, mounting and spreading inductance (H)
    pub esl: f64,
    pub l_mount: f64,
    pub l_spread: f64,
    /// series resonance with the total inductance (Hz)
    pub srf: f64,
    /// distance to the load centroid (mm)
    pub dist_mm: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlanePair {
    pub layer: u16,
    pub gnd_layer: u16,
    pub area_mm2: f64,
    pub d_mm: f64,
    pub c: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct PdnResult {
    pub net: String,
    pub voltage: f64,
    pub freq: Vec<f64>,
    pub z: Vec<f64>,
    /// |Z| without decoupling capacitors (VRM + plane only)
    pub z_bare: Vec<f64>,
    pub caps: Vec<CapInfo>,
    pub planes: Vec<PlanePair>,
    pub c_plane: f64,
    pub c_total: f64,
    /// first cavity resonance estimate of the largest plane (Hz)
    pub cavity_hz: Option<f64>,
    pub target_ohm: Option<f64>,
    /// frequency range where |Z| exceeds the target (Hz)
    pub above_target: Vec<(f64, f64)>,
    /// upper end of the evaluated range (Hz)
    pub f_eval: f64,
    /// grid model of the main plane (from 1 MHz): frequencies, |Z|, cells, cell size (mm)
    pub cav_freq: Vec<f64>,
    pub cav_z: Vec<f64>,
    pub cav_cells: usize,
    pub cav_h_mm: f64,
    pub load_current: f64,
    pub load_at: Option<P>,
    pub warnings: Vec<String>,
}

/// Capacitance from a footprint value: "100n", "100nF", "0.1uF 16V", "4u7", "10µF/6.3V", "1p5".
pub fn parse_cap(v: &str) -> Option<f64> {
    let s = v.trim().replace(',', ".").replace('µ', "u").replace('μ', "u");
    let tok = s.split(|c: char| c.is_whitespace() || c == '/' || c == '_').next()?;
    let t = tok.trim_end_matches(['F', 'f']);
    let mult = |c: char| match c {
        'p' | 'P' => Some(1e-12),
        'n' | 'N' => Some(1e-9),
        'u' | 'U' => Some(1e-6),
        'm' => Some(1e-3),
        _ => None,
    };
    // "4u7" style
    if let Some(pos) = t.find(|c: char| mult(c).is_some()) {
        let (a, b) = t.split_at(pos);
        let m = mult(b.chars().next()?)?;
        let rest = &b[1..];
        if a.is_empty() || !a.chars().all(|c| c.is_ascii_digit() || c == '.') {
            return None;
        }
        if !rest.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        let num: f64 = if rest.is_empty() { a.parse().ok()? } else { format!("{a}.{rest}").parse().ok()? };
        return Some(num * m);
    }
    // plain number with an explicit F
    if tok.ends_with(['F', 'f']) {
        return t.parse().ok();
    }
    None
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

/// (body ESL, ESR) estimates for an MLCC of this size and value; large values
/// in big packages are treated as polymer/tantalum.
fn esl_esr(pkg: &str, c: f64) -> (f64, f64) {
    let esl = match pkg {
        "0201" => 0.2e-9,
        "0402" => 0.35e-9,
        "0603" => 0.5e-9,
        "0805" => 0.6e-9,
        "1206" => 0.8e-9,
        "1210" => 0.9e-9,
        _ => 1.5e-9,
    };
    let mut esr = (0.025 * (1e-7 / c).powf(0.4)).clamp(0.002, 0.5);
    if c >= 47e-6 && pkg == "large" {
        esr = esr.max(0.015);
    }
    (esl, esr)
}

fn polygon_area(p: &[P]) -> f64 {
    let n = p.len();
    let mut a = 0.0;
    for i in 0..n {
        let (x0, y0) = (p[i][0], p[i][1]);
        let (x1, y1) = (p[(i + 1) % n][0], p[(i + 1) % n][1]);
        a += x0 * y1 - x1 * y0;
    }
    (a / 2.0).abs()
}

pub fn pdn(board: &Board, setup: &RailSetup, p: &PdnParams) -> Result<PdnResult, String> {
    let su = board.stackup();
    let net = find_net(board, &setup.net).ok_or_else(|| format!("net {} not found", setup.net))?;
    let mut warnings = Vec::new();
    let is_gnd = |n: u32| n != 0 && is_ground_name(board.net_name(n));

    // copper area of the rail and of ground per layer
    let nl = su.n();
    let mut area = vec![0.0; nl];
    let mut gnd_area = vec![0.0; nl];
    let mut bb_rail: Option<(P, P)> = None;
    for z in &board.zones {
        let a = polygon_area(&z.pts);
        if z.net == net {
            area[z.layer as usize] += a;
            let b = bb_rail.get_or_insert((z.bbox.min, z.bbox.max));
            b.0 = [b.0[0].min(z.bbox.min[0]), b.0[1].min(z.bbox.min[1])];
            b.1 = [b.1[0].max(z.bbox.max[0]), b.1[1].max(z.bbox.max[1])];
        } else if is_gnd(z.net) {
            gnd_area[z.layer as usize] += a;
        }
    }
    // plane pairs: each rail layer with ≥ 20 mm² against its nearest ground layer
    let mut planes = Vec::new();
    for k in 0..nl {
        if area[k] < 20.0 {
            continue;
        }
        let g = (0..nl).filter(|&j| j != k && gnd_area[j] > 0.3 * area[k]).min_by_key(|&j| (j as i64 - k as i64).abs());
        if let Some(j) = g {
            let d = su.gap_between(k, j);
            let er = su.er_between(k, j);
            let c = EPS0 * er * area[k] * 1e-6 / (d * 1e-3);
            planes.push(PlanePair { layer: k as u16, gnd_layer: j as u16, area_mm2: area[k], d_mm: d, c });
        }
    }
    let c_plane: f64 = planes.iter().map(|x| x.c).fold(0.0, |a, b| a + b);
    let main = planes.iter().max_by(|a, b| a.area_mm2.partial_cmp(&b.area_mm2).unwrap()).cloned();
    let tand = main.as_ref().map(|m| {
        let (a, b) = (m.layer.min(m.gnd_layer) as usize, m.layer.max(m.gnd_layer) as usize);
        su.dielectrics[a..b].iter().map(|d| d.loss_tangent).sum::<f64>() / (b - a) as f64
    });
    let cavity_hz = match (&main, bb_rail) {
        (Some(m), Some((lo, hi))) => {
            let lmax = (hi[0] - lo[0]).max(hi[1] - lo[1]) * 1e-3;
            let er = su.er_between(m.layer as usize, m.gnd_layer as usize);
            if lmax > 0.0 { Some(C0 / (2.0 * er.sqrt() * lmax)) } else { None }
        }
        _ => None,
    };
    if planes.is_empty() {
        warnings.push("rail without a plane over ground: no plane capacitance, spreading inductance over traces roughly estimated".into());
    }

    // load centroid (current-weighted pads of the load components)
    let mut fp_pads: HashMap<u32, Vec<usize>> = HashMap::new();
    for (i, pd) in board.pads.iter().enumerate() {
        fp_pads.entry(pd.fp).or_default().push(i);
    }
    let fp_by_ref: HashMap<&str, u32> = board.footprints.iter().enumerate().map(|(i, f)| (f.reference.as_str(), i as u32)).collect();
    let mut wsum = 0.0;
    let mut cen = [0.0, 0.0];
    let mut load_current = 0.0;
    for l in &setup.loads {
        load_current += l.current.abs();
        let Some(&fi) = fp_by_ref.get(l.refdes.as_str()) else { continue };
        let pads: Vec<&crate::board::Pad> = fp_pads.get(&fi).map(|v| v.iter().map(|&i| &board.pads[i]).filter(|pd| pd.net == net).collect()).unwrap_or_default();
        if pads.is_empty() {
            continue;
        }
        let w = l.current.abs().max(1e-6);
        for pd in &pads {
            cen[0] += pd.at[0] * w / pads.len() as f64;
            cen[1] += pd.at[1] * w / pads.len() as f64;
        }
        wsum += w;
    }
    let load_at = if wsum > 0.0 { Some([cen[0] / wsum, cen[1] / wsum]) } else { None };
    if load_at.is_none() {
        warnings.push("no loads with a position on the board: spreading inductance not included".into());
    }

    // decoupling capacitors: two-pad parts between the rail and ground
    let mut caps = Vec::new();
    let mut unparsed = 0;
    for (fi, f) in board.footprints.iter().enumerate() {
        let Some(pi) = fp_pads.get(&(fi as u32)) else { continue };
        let cu: Vec<&crate::board::Pad> = pi.iter().map(|&i| &board.pads[i]).filter(|pd| !pd.layers.is_empty()).collect();
        if cu.len() != 2 {
            continue;
        }
        let (a, b) = (cu[0], cu[1]);
        let (pr, pg) = if a.net == net && is_gnd(b.net) {
            (a, b)
        } else if b.net == net && is_gnd(a.net) {
            (b, a)
        } else {
            continue;
        };
        if !f.reference.starts_with('C') {
            continue;
        }
        let Some(c) = parse_cap(&f.value) else {
            unparsed += 1;
            continue;
        };
        let pitch = geom::len(geom::sub(pr.at, pg.at));
        let pkg = package_of(pitch);
        let (esl, esr) = esl_esr(pkg, c);
        let layer = pr.layers[0];
        // mounting: nearest via of each pad's net, loop down to the plane pair
        let near_via = |pd: &crate::board::Pad| -> Option<(f64, f64)> {
            board
                .vias
                .iter()
                .filter(|v| v.net == pd.net)
                .map(|v| (geom::len(geom::sub(v.at, pd.at)), v.drill))
                .filter(|x| x.0 < 2.0)
                .min_by(|x, y| x.0.partial_cmp(&y.0).unwrap())
        };
        let depth = |target: Option<usize>| -> f64 { target.map_or(0.2, |t| su.gap_between(layer as usize, t).max(0.05)) };
        let rail_layer = main.as_ref().map(|m| m.layer as usize);
        let gnd_layer = main.as_ref().map(|m| m.gnd_layer as usize);
        let h = depth(rail_layer).max(depth(gnd_layer));
        let (vr, vg) = (near_via(pr), near_via(pg));
        let on_plane = |pd: &crate::board::Pad| main.as_ref().is_some_and(|m| (m.layer == layer && pd.net == net) || (m.gnd_layer == layer && is_gnd(pd.net)));
        let mut l_mount = 0.0;
        let trace = |d: f64| 0.4e-9 * d; // pad → via, ~0.4 nH/mm
        let s = match (vr, vg) {
            (Some(x), Some(y)) => {
                l_mount += trace(x.0) + trace(y.0);
                pitch.max(x.0 + y.0).max(0.3)
            }
            _ => pitch.max(0.3),
        };
        let r = vr.or(vg).map_or(0.1, |v| v.1 / 2.0);
        if !(on_plane(pr) && on_plane(pg)) {
            l_mount += MU0 * h * 1e-3 / PI * (s / r).max(1.5).ln();
        }
        if vr.is_none() && !on_plane(pr) || vg.is_none() && !on_plane(pg) {
            l_mount += 1.0e-9; // no via next to the pad: longer path, rough
        }
        let (dist, l_spread) = match load_at {
            Some(lp) => {
                let dist = geom::len(geom::sub(lp, pr.at));
                let ls = match &main {
                    Some(m) => MU0 * m.d_mm * 1e-3 / PI * (dist / 0.15).max(1.0).ln(),
                    None => 0.5e-9 * dist,
                };
                (dist, ls)
            }
            None => (0.0, 0.0),
        };
        let lt = esl + l_mount + l_spread;
        caps.push(CapInfo {
            refdes: f.reference.clone(),
            value: f.value.clone(),
            c,
            package: pkg,
            layer,
            at: pr.at,
            esr,
            esl,
            l_mount,
            l_spread,
            srf: 1.0 / (2.0 * PI * (lt * c).sqrt()),
            dist_mm: dist,
        });
    }
    if unparsed > 0 {
        warnings.push(format!("{unparsed} capacitors with unreadable value skipped"));
    }
    if caps.is_empty() {
        warnings.push("no decoupling capacitors found between rail and ground".into());
    }
    caps.sort_by(|a, b| b.c.partial_cmp(&a.c).unwrap());

    // frequency sweep (log)
    let n = p.npts.max(20);
    let freq: Vec<f64> = (0..n).map(|i| p.fmin * (p.fmax / p.fmin).powf(i as f64 / (n - 1) as f64)).collect();
    let mut z = Vec::with_capacity(n);
    let mut z_bare = Vec::with_capacity(n);
    for &f in &freq {
        let w = 2.0 * PI * f;
        let y_vrm = Cx::new(p.vrm_r, w * p.vrm_l).inv();
        let y_plane = Cx::new(w * c_plane * tand.unwrap_or(0.02), w * c_plane);
        let mut y = y_vrm + y_plane;
        z_bare.push(y.inv().abs());
        for c in &caps {
            let zc = Cx::new(c.esr, w * (c.esl + c.l_mount + c.l_spread) - 1.0 / (w * c.c));
            y = y + zc.inv();
        }
        z.push(y.inv().abs());
    }
    // grid model of the main plane above 1 MHz
    let (mut cav_freq, mut cav_z, mut cav_cells, mut cav_h) = (Vec::new(), Vec::new(), 0, 0.0);
    if let (Some(m), Some(lp), true) = (&main, load_at, p.max_cells > 0) {
        let fq: Vec<f64> = freq.iter().cloned().filter(|&f| f >= 1e6).step_by(2).collect();
        let mut shunts: Vec<Shunt> = caps.iter().map(|c| Shunt { at: c.at, r: c.esr, l: c.esl + c.l_mount, c: Some(c.c) }).collect();
        for src in &setup.sources {
            if let Some(&fi) = fp_by_ref.get(src.refdes.as_str()) {
                let pads: Vec<P> = fp_pads.get(&fi).map(|v| v.iter().map(|&i| &board.pads[i]).filter(|pd| pd.net == net).map(|pd| pd.at).collect()).unwrap_or_default();
                if !pads.is_empty() {
                    let c = [pads.iter().map(|q| q[0]).sum::<f64>() / pads.len() as f64, pads.iter().map(|q| q[1]).sum::<f64>() / pads.len() as f64];
                    shunts.push(Shunt { at: c, r: p.vrm_r, l: p.vrm_l, c: None });
                }
            }
        }
        let er = su.er_between(m.layer as usize, m.gnd_layer as usize);
        if let Some(cr) = cavity(board, net, m.layer, m.d_mm, er, tand.unwrap_or(0.02), &shunts, lp, &fq, p.max_cells) {
            cav_freq = cr.freq;
            cav_z = cr.z;
            cav_cells = cr.cells;
            cav_h = cr.h_mm;
        }
    }
    let target = if load_current > 0.0 && setup.voltage.abs() > 0.0 {
        Some(setup.voltage.abs() * p.ripple / (p.step * load_current))
    } else {
        None
    };
    let mut above = Vec::new();
    if let Some(t) = target {
        let mut start: Option<f64> = None;
        let comb: Vec<f64> = freq
            .iter()
            .zip(&z)
            .map(|(&f, &zl)| {
                if cav_freq.is_empty() || f < cav_freq[0] {
                    return zl;
                }
                // interpolate the grid curve (log-spaced, every second point)
                let k = cav_freq.iter().position(|&x| x >= f).unwrap_or(cav_freq.len() - 1);
                if k == 0 || cav_freq[k] == f {
                    cav_z[k]
                } else {
                    let t = (f / cav_freq[k - 1]).ln() / (cav_freq[k] / cav_freq[k - 1]).ln();
                    cav_z[k - 1] * (cav_z[k] / cav_z[k - 1]).powf(t)
                }
            })
            .collect();
        for (i, &zi) in comb.iter().enumerate() {
            // board decoupling matters up to ~100 MHz; above, package and die capacitance take over
            let fcap = if cav_freq.is_empty() { cavity_hz.unwrap_or(f64::INFINITY).min(F_BOARD) } else { F_BOARD };
            let bad = zi > t && freq[i] <= fcap;
            match (bad, start) {
                (true, None) => start = Some(freq[i]),
                (false, Some(s0)) => {
                    above.push((s0, freq[i - 1]));
                    start = None;
                }
                _ => {}
            }
        }
        if let Some(s0) = start {
            above.push((s0, freq[n - 1].min(if cav_freq.is_empty() { cavity_hz.unwrap_or(f64::INFINITY) } else { f64::INFINITY }).min(F_BOARD)));
        }
    }
    let c_total = caps.iter().map(|c| c.c).fold(c_plane, |a, b| a + b);
    Ok(PdnResult {
        net: board.net_name(net).to_string(),
        voltage: setup.voltage,
        freq,
        z,
        z_bare,
        caps,
        planes,
        c_plane,
        c_total,
        cavity_hz,
        target_ohm: target,
        above_target: above,
        f_eval: if cav_freq.is_empty() { cavity_hz.unwrap_or(f64::INFINITY).min(F_BOARD) } else { F_BOARD },
        cav_freq,
        cav_z,
        cav_cells,
        cav_h_mm: cav_h,
        load_current,
        load_at,
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cap_values() {
        let cases = [("100n", 100e-9), ("100nF", 100e-9), ("0.1uF 16V", 0.1e-6), ("4u7", 4.7e-6), ("10µF/6.3V", 10e-6), ("1p5", 1.5e-12), ("22uF_X5R", 22e-6)];
        for (s, v) in cases {
            let c = parse_cap(s).unwrap_or(0.0);
            assert!((c - v).abs() < v * 1e-9, "{s}: {c}");
        }
        assert!(parse_cap("BLM18PG").is_none());
        assert!(parse_cap("DNP").is_none());
    }

    #[test]
    fn series_resonance() {
        let (esl, esr) = esl_esr("0402", 100e-9);
        assert!((esl - 0.35e-9).abs() < 1e-12);
        assert!(esr > 0.01 && esr < 0.05);
    }
}

// --- plane-pair cavity model (grid) -------------------------------------------------------
//
// The rail plane over an ideal ground plane is meshed into square cells of
// size h. Neighbouring cells are joined by the plane-pair inductance μ0·d per
// square (plus the sheet resistance of both planes); every cell carries its
// capacitance ε·h²/d with dielectric loss. Decoupling capacitors (without the
// lumped spreading term – the grid models it) and the regulator are shunt
// branches at their cells. 1 A is injected at the load cell; |V| = |Z|.
// The complex banded system is solved per frequency by LU with partial
// pivoting. Resonances of the real plane shape appear naturally.

/// Complex banded matrix with `kl` sub- and `ku` super-diagonals, stored by
/// rows over columns [i − kl, i + ku + kl] (room for pivoting fill-in).
struct Band {
    n: usize,
    kl: usize,
    ku: usize,
    w: usize,
    a: Vec<Cx>,
}

impl Band {
    fn new(n: usize, kl: usize, ku: usize) -> Self {
        let w = 2 * kl + ku + 1;
        Band { n, kl, ku, w, a: vec![Cx::new(0.0, 0.0); n * w] }
    }
    #[inline]
    fn idx(&self, i: usize, j: usize) -> Option<usize> {
        let off = j as isize - i as isize + self.kl as isize;
        if off < 0 || off as usize >= self.w {
            None
        } else {
            Some(i * self.w + off as usize)
        }
    }
    fn add(&mut self, i: usize, j: usize, v: Cx) {
        let k = self.idx(i, j).expect("outside band");
        self.a[k] = self.a[k] + v;
    }
    fn get(&self, i: usize, j: usize) -> Cx {
        self.idx(i, j).map_or(Cx::new(0.0, 0.0), |k| self.a[k])
    }
    fn set(&mut self, i: usize, j: usize, v: Cx) {
        if let Some(k) = self.idx(i, j) {
            self.a[k] = v;
        }
    }
    /// Solve A x = b in place (A is destroyed).
    fn solve(mut self, b: &mut [Cx]) -> Result<(), String> {
        let n = self.n;
        let (kl, ku) = (self.kl, self.ku);
        for k in 0..n {
            // pivot among rows k..k+kl
            let last = (k + kl).min(n - 1);
            let mut p = k;
            let mut best = self.get(k, k).abs();
            for i in k + 1..=last {
                let v = self.get(i, k).abs();
                if v > best {
                    best = v;
                    p = i;
                }
            }
            if best == 0.0 {
                return Err("singular matrix".into());
            }
            let jmax = (k + ku + kl).min(n - 1);
            if p != k {
                for j in k..=jmax {
                    let (x, y) = (self.get(k, j), self.get(p, j));
                    self.set(k, j, y);
                    self.set(p, j, x);
                }
                b.swap(k, p);
            }
            let piv = self.get(k, k).inv();
            let w = self.w;
            for i in k + 1..=last {
                let f = self.get(i, k) * piv;
                if f.re == 0.0 && f.im == 0.0 {
                    continue;
                }
                self.set(i, k, Cx::new(0.0, 0.0));
                // row_i[j] -= f · row_k[j] for j in k+1..=jmax (contiguous in both rows)
                let len = jmax - k;
                let ko = k * w + kl + 1; // column k+1 in row k
                let io = i * w + (k + 1 + kl - i); // column k+1 in row i
                let (head, tail) = self.a.split_at_mut(i * w);
                let rk = &head[ko..ko + len];
                let ri = &mut tail[io - i * w..io - i * w + len];
                for (x, &u) in ri.iter_mut().zip(rk) {
                    x.re -= f.re * u.re - f.im * u.im;
                    x.im -= f.re * u.im + f.im * u.re;
                }
                b[i] = b[i] - f * b[k];
            }
        }
        for k in (0..n).rev() {
            let jmax = (k + ku + kl).min(n - 1);
            let mut s = b[k];
            for j in k + 1..=jmax {
                s = s - self.get(k, j) * b[j];
            }
            b[k] = s * self.get(k, k).inv();
        }
        Ok(())
    }
}

/// Shunt element at a point of the plane: (position, branch impedance function input).
pub struct Shunt {
    pub at: P,
    pub r: f64,
    pub l: f64,
    /// capacitance (F); infinite for a regulator
    pub c: Option<f64>,
}

pub struct CavityResult {
    pub freq: Vec<f64>,
    pub z: Vec<f64>,
    pub cells: usize,
    pub h_mm: f64,
}

/// Grid model of the rail plane `layer` of net `net` over ground at distance `d_mm`.
pub fn cavity(board: &Board, net: u32, layer: u16, d_mm: f64, er: f64, tand: f64, shunts: &[Shunt], port: P, freq: &[f64], max_cells: usize) -> Option<CavityResult> {
    let polys: Vec<&crate::board::ZoneFill> = board.zones.iter().filter(|z| z.net == net && z.layer == layer).collect();
    if polys.is_empty() {
        return None;
    }
    let mut lo = [f64::INFINITY; 2];
    let mut hi = [f64::NEG_INFINITY; 2];
    let mut area = 0.0;
    for z in &polys {
        lo = [lo[0].min(z.bbox.min[0]), lo[1].min(z.bbox.min[1])];
        hi = [hi[0].max(z.bbox.max[0]), hi[1].max(z.bbox.max[1])];
        area += polygon_area(&z.pts);
    }
    let h = (area / max_cells as f64).sqrt().max(0.2);
    let nx = ((hi[0] - lo[0]) / h).ceil() as usize + 1;
    let ny = ((hi[1] - lo[1]) / h).ceil() as usize + 1;
    // inside test per cell centre (even-odd per polygon, union of polygons)
    let mut inside = vec![false; nx * ny];
    for z in &polys {
        let pts = &z.pts;
        let m = pts.len();
        for iy in 0..ny {
            let y = lo[1] + (iy as f64 + 0.5) * h;
            if y < z.bbox.min[1] || y > z.bbox.max[1] {
                continue;
            }
            let mut xs: Vec<f64> = Vec::new();
            for i in 0..m {
                let (a, b) = (pts[i], pts[(i + 1) % m]);
                if (a[1] > y) != (b[1] > y) {
                    xs.push(a[0] + (y - a[1]) / (b[1] - a[1]) * (b[0] - a[0]));
                }
            }
            xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
            for pair in xs.chunks_exact(2) {
                let i0 = ((pair[0] - lo[0]) / h - 0.5).ceil().max(0.0) as usize;
                let i1 = ((pair[1] - lo[0]) / h - 0.5).floor();
                if i1 < 0.0 {
                    continue;
                }
                for ix in i0..=(i1 as usize).min(nx - 1) {
                    inside[iy * nx + ix] = true;
                }
            }
        }
    }
    // numbering along the shorter side for a narrow band
    let by_rows = nx <= ny;
    let (na, nb) = if by_rows { (nx, ny) } else { (ny, nx) };
    let cell = |a: usize, b: usize| if by_rows { b * nx + a } else { a * nx + b };
    let mut id = vec![usize::MAX; nx * ny];
    let mut n = 0;
    for b in 0..nb {
        for a in 0..na {
            if inside[cell(a, b)] {
                id[cell(a, b)] = n;
                n += 1;
            }
        }
    }
    if n < 4 {
        return None;
    }
    // branches and bandwidth
    let mut branches = Vec::new();
    let mut bw = 1;
    for iy in 0..ny {
        for ix in 0..nx {
            let i = id[iy * nx + ix];
            if i == usize::MAX {
                continue;
            }
            for (jx, jy) in [(ix + 1, iy), (ix, iy + 1)] {
                if jx < nx && jy < ny {
                    let j = id[jy * nx + jx];
                    if j != usize::MAX {
                        branches.push((i, j));
                        bw = bw.max(j.abs_diff(i));
                    }
                }
            }
        }
    }
    let pos_of = |i: usize| -> P {
        // centre of the cell with number i (linear search is fine for a few lookups)
        let k = id.iter().position(|&x| x == i).unwrap();
        [lo[0] + ((k % nx) as f64 + 0.5) * h, lo[1] + ((k / nx) as f64 + 0.5) * h]
    };
    let nearest = |p: P| -> usize {
        let ix = (((p[0] - lo[0]) / h) as isize).clamp(0, nx as isize - 1) as usize;
        let iy = (((p[1] - lo[1]) / h) as isize).clamp(0, ny as isize - 1) as usize;
        if id[iy * nx + ix] != usize::MAX {
            return id[iy * nx + ix];
        }
        let mut best = (f64::INFINITY, 0);
        for (k, &v) in id.iter().enumerate() {
            if v != usize::MAX {
                let c = [lo[0] + ((k % nx) as f64 + 0.5) * h, lo[1] + ((k / nx) as f64 + 0.5) * h];
                let d = geom::len(geom::sub(c, p));
                if d < best.0 {
                    best = (d, v);
                }
            }
        }
        best.1
    };
    let _ = pos_of;
    let port_i = nearest(port);
    let sh: Vec<(usize, &Shunt)> = shunts.iter().map(|s| (nearest(s.at), s)).collect();
    let t_cu = board.stackup().copper[layer as usize].thickness * 1e-3;
    let rho = 1.72e-8;
    let c_cell = EPS0 * er * (h * 1e-3).powi(2) / (d_mm * 1e-3);
    let l_sq = MU0 * d_mm * 1e-3;
    let mut z = Vec::with_capacity(freq.len());
    for &f in freq {
        let w = 2.0 * PI * f;
        let rs = (rho / t_cu).max((PI * f * MU0 * rho).sqrt());
        let yb = Cx::new(2.0 * rs, w * l_sq).inv();
        let yc = Cx::new(w * c_cell * tand, w * c_cell);
        let mut m = Band::new(n, bw, bw);
        for i in 0..n {
            m.add(i, i, yc);
        }
        for &(i, j) in &branches {
            m.add(i, i, yb);
            m.add(j, j, yb);
            m.add(i, j, Cx::new(-yb.re, -yb.im));
            m.add(j, i, Cx::new(-yb.re, -yb.im));
        }
        for (i, s) in &sh {
            let x = w * s.l - s.c.map_or(0.0, |c| 1.0 / (w * c));
            m.add(*i, *i, Cx::new(s.r, x).inv());
        }
        let mut b = vec![Cx::new(0.0, 0.0); n];
        b[port_i] = Cx::new(1.0, 0.0);
        if m.solve(&mut b).is_err() {
            return None;
        }
        z.push(b[port_i].abs());
    }
    Some(CavityResult { freq: freq.to_vec(), z, cells: n, h_mm: h })
}

#[cfg(test)]
mod cavity_tests {
    use super::*;
    use crate::board::{Board, CopperLayer, Dielectric, Stackup, ZoneFill};
    use crate::geom::BBox;

    #[test]
    fn band_solver_matches_dense() {
        // tridiagonal complex system
        let n = 6;
        let mut m = Band::new(n, 1, 1);
        for i in 0..n {
            m.add(i, i, Cx::new(2.0, 0.5));
            if i + 1 < n {
                m.add(i, i + 1, Cx::new(-1.0, 0.1));
                m.add(i + 1, i, Cx::new(-1.0, 0.1));
            }
        }
        let a: Vec<Vec<Cx>> = (0..n).map(|i| (0..n).map(|j| m.get(i, j)).collect()).collect();
        let x_true: Vec<Cx> = (0..n).map(|i| Cx::new(i as f64, 1.0)).collect();
        let mut b: Vec<Cx> = (0..n).map(|i| (0..n).fold(Cx::new(0.0, 0.0), |s, j| s + a[i][j] * x_true[j])).collect();
        m.solve(&mut b).unwrap();
        for i in 0..n {
            assert!((b[i] - x_true[i]).abs() < 1e-9);
        }
    }

    #[test]
    fn rectangular_plane_first_resonance() {
        // 50 × 30 mm plane, d = 0.1 mm, εr 4.3, port in a corner: first
        // resonance (1,0) at c/(2·50 mm·√4.3) ≈ 1.446 GHz
        let pts = vec![[0.0, 0.0], [50.0, 0.0], [50.0, 30.0], [0.0, 30.0]];
        let su = Stackup {
            copper: vec![CopperLayer { name: "F.Cu".into(), thickness: 0.035 }, CopperLayer { name: "B.Cu".into(), thickness: 0.035 }],
            dielectrics: vec![Dielectric { name: "d".into(), kind: "core".into(), material: "FR4".into(), thickness: 0.1, epsilon_r: 4.3, loss_tangent: 0.02 }],
            mask_top: None,
            mask_bottom: None,
            from_file: true,
        };
        let board = Board {
            stackup: Some(su),
            nets: vec!["".into(), "VCC".into()],
            zones: vec![ZoneFill { layer: 0, net: 1, bbox: BBox::of(&pts), pts }],
            ..Default::default()
        };
        let freq: Vec<f64> = (0..121).map(|i| 0.5e9 + i as f64 * 0.01e9).collect();
        let r = cavity(&board, 1, 0, 0.1, 4.3, 0.02, &[], [1.0, 1.0], &freq, 1500).unwrap();
        let (k, _) = r.z.iter().enumerate().max_by(|a, b| a.1.partial_cmp(b.1).unwrap()).unwrap();
        let f_pk = freq[k];
        assert!((f_pk - 1.446e9).abs() < 0.05e9, "peak at {:.3} GHz", f_pk / 1e9);
    }
}

//! IBIS models as linear channel ends (milestone M7b).
//!
//! STATUS: EXPERIMENTAL – NOT WORKING YET. Model selection per pin and the interplay
//! with the interface profiles are not validated; see README (M7b).
//!
//! Reads the parts of an IBIS file (.ibs, versions 1.1 to 7.x) that a linear
//! eye analysis needs – [Component] with [Package] and [Pin], [Model Selector],
//! [Model] with C_comp, the I-V tables, [Ramp] and on-die terminators – and
//! reduces a pin to the linear end model of `eye::EndModel`:
//!
//! * transmitter: the pull-up and pull-down I-V curves (plus clamps) are
//!   linearised at the operating point of the [Ramp] load (R_load to ground for
//!   the high level, to the supply for the low level); the tangent gives the
//!   Thevenin resistance and open-circuit level per state. Edge rate from
//!   [Ramp] (20-80 %), converted to 10-90 % for a Gaussian edge.
//! * receiver: small-signal conductance of the clamps (and [Rgnd]/[Rpower]) at
//!   half the supply – an on-die termination modelled as linear clamp curves
//!   appears as a resistor, plain CMOS inputs as high impedance.
//! * both: C_comp plus C_pin (or C_pkg), L_pin (or L_pkg); R_pin/R_pkg adds to
//!   the transmitter resistance.
//!
//! Only the typical column is used. Waveform tables, IBIS-AMI and package
//! models ([Package Model], .pkg/.ebd) are not evaluated; the result is an
//! estimate and is labelled as such.

use crate::eye::EndModel;
use serde::Serialize;

/// A value from an IBIS number: "3.0pF", "50m", "-5.0V", "1.2e-3", "NA" → None.
/// Scale suffixes per the IBIS spec (case sensitive): T G M(ega) k m u n p f.
pub fn num(tok: &str) -> Option<f64> {
    let s = tok.trim();
    if s.eq_ignore_ascii_case("na") || s.is_empty() {
        return None;
    }
    let b = s.as_bytes();
    let mut i = 0;
    if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
        i += 1;
    }
    let ds = i;
    while i < b.len() && (b[i].is_ascii_digit() || b[i] == b'.') {
        i += 1;
    }
    if i == ds {
        return None;
    }
    // exponent only when digits follow
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        let mut j = i + 1;
        if j < b.len() && (b[j] == b'+' || b[j] == b'-') {
            j += 1;
        }
        if j < b.len() && b[j].is_ascii_digit() {
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            i = j;
        }
    }
    let v: f64 = s[..i].parse().ok()?;
    let m = match b.get(i) {
        Some(b'T') => 1e12,
        Some(b'G') => 1e9,
        Some(b'M') => 1e6,
        Some(b'k') => 1e3,
        Some(b'm') => 1e-3,
        Some(b'u') => 1e-6,
        Some(b'n') => 1e-9,
        Some(b'p') => 1e-12,
        Some(b'f') => 1e-15,
        _ => 1.0,
    };
    Some(v * m)
}

/// I-V table: (V, I typ), sorted by V.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Table(pub Vec<(f64, f64)>);

impl Table {
    /// Linear interpolation, linear extrapolation beyond the ends.
    pub fn at(&self, v: f64) -> f64 {
        let t = &self.0;
        match t.len() {
            0 => 0.0,
            1 => t[0].1,
            n => {
                let k = t.partition_point(|p| p.0 < v).clamp(1, n - 1);
                let (a, b) = (t[k - 1], t[k]);
                if b.0 == a.0 {
                    return b.1;
                }
                a.1 + (b.1 - a.1) * (v - a.0) / (b.0 - a.0)
            }
        }
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct PinEntry {
    pub pin: String,
    pub signal: String,
    pub model: String,
    pub r: Option<f64>,
    pub l: Option<f64>,
    pub c: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Component {
    pub name: String,
    pub r_pkg: Option<f64>,
    pub l_pkg: Option<f64>,
    pub c_pkg: Option<f64>,
    pub pins: Vec<PinEntry>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Model {
    pub name: String,
    pub model_type: String,
    pub c_comp: Option<f64>,
    /// sum of C_comp_pullup/pulldown/power_clamp/gnd_clamp (IBIS 4+)
    pub c_comp_parts: Option<f64>,
    pub v_range: Option<f64>,
    pub pullup_ref: Option<f64>,
    pub pulldown_ref: Option<f64>,
    pub power_clamp_ref: Option<f64>,
    pub gnd_clamp_ref: Option<f64>,
    pub pullup: Option<Table>,
    pub pulldown: Option<Table>,
    pub gnd_clamp: Option<Table>,
    pub power_clamp: Option<Table>,
    /// [Ramp] typ: (dV, dt) of the 20-80 % edge
    pub ramp_r: Option<(f64, f64)>,
    pub ramp_f: Option<(f64, f64)>,
    pub r_load: Option<f64>,
    pub rgnd: Option<f64>,
    pub rpower: Option<f64>,
    pub ami: bool,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct IbisFile {
    pub version: String,
    pub components: Vec<Component>,
    pub models: Vec<Model>,
    /// [Model Selector] name → model names
    pub selectors: Vec<(String, Vec<String>)>,
}

fn norm_kw(s: &str) -> String {
    s.trim().to_ascii_lowercase().replace('_', " ").split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Parse an IBIS file. Unknown keywords are skipped; only a file without any
/// [Component] or [Model] is an error.
pub fn parse(text: &str) -> Result<IbisFile, String> {
    #[derive(PartialEq)]
    enum Ctx {
        None,
        Component,
        Model,
        Selector,
    }
    let mut f = IbisFile::default();
    let mut cc = '|';
    let mut kw = String::new();
    let mut ctx = Ctx::None;
    for raw in text.lines() {
        let line = raw.trim_end();
        let lt = line.trim_start();
        // [Comment Char] changes the comment character (before stripping comments)
        if lt.starts_with('[') && norm_kw(lt.get(1..lt.find(']').unwrap_or(1)).unwrap_or("")) == "comment char" {
            if let Some(c) = lt[lt.find(']').unwrap_or(0) + 1..].trim().chars().next() {
                cc = c;
            }
            kw = "comment char".into();
            continue;
        }
        let line = match line.find(cc) {
            Some(p) => &line[..p],
            None => line,
        };
        let lt = line.trim();
        if lt.is_empty() {
            continue;
        }
        if lt.starts_with('[') {
            let Some(end) = lt.find(']') else { continue };
            kw = norm_kw(&lt[1..end]);
            let rest = lt[end + 1..].trim();
            let vals = || rest.split_whitespace().next().and_then(num);
            match kw.as_str() {
                "ibis ver" => f.version = rest.to_string(),
                "component" => {
                    f.components.push(Component { name: rest.to_string(), ..Default::default() });
                    ctx = Ctx::Component;
                }
                "model selector" => {
                    f.selectors.push((rest.to_string(), vec![]));
                    ctx = Ctx::Selector;
                }
                "model" => {
                    f.models.push(Model { name: rest.to_string(), ..Default::default() });
                    ctx = Ctx::Model;
                }
                // a submodel or package model has its own tables: leave the model
                "submodel" | "define package model" | "external model" | "external circuit" => ctx = Ctx::None,
                "end" => break,
                _ => {}
            }
            if ctx == Ctx::Model {
                let m = f.models.last_mut().unwrap();
                match kw.as_str() {
                    "voltage range" => m.v_range = vals(),
                    "pullup reference" => m.pullup_ref = vals(),
                    "pulldown reference" => m.pulldown_ref = vals(),
                    "power clamp reference" => m.power_clamp_ref = vals(),
                    "gnd clamp reference" => m.gnd_clamp_ref = vals(),
                    "rgnd" => m.rgnd = vals(),
                    "rpower" => m.rpower = vals(),
                    "pullup" => m.pullup = Some(Table::default()),
                    "pulldown" => m.pulldown = Some(Table::default()),
                    "gnd clamp" => m.gnd_clamp = Some(Table::default()),
                    "power clamp" => m.power_clamp = Some(Table::default()),
                    "algorithmic model" => m.ami = true,
                    _ => {}
                }
            }
            continue;
        }
        // data line of the current keyword
        let tok: Vec<&str> = lt.split_whitespace().collect();
        match (kw.as_str(), &ctx) {
            ("package", Ctx::Component) => {
                let c = f.components.last_mut().unwrap();
                let v = tok.get(1).and_then(|t| num(t));
                match tok[0].to_ascii_lowercase().as_str() {
                    "r_pkg" => c.r_pkg = v,
                    "l_pkg" => c.l_pkg = v,
                    "c_pkg" => c.c_pkg = v,
                    _ => {}
                }
            }
            ("pin", Ctx::Component) if tok.len() >= 3 => {
                let c = f.components.last_mut().unwrap();
                c.pins.push(PinEntry {
                    pin: tok[0].to_string(),
                    signal: tok[1].to_string(),
                    model: tok[2].to_string(),
                    r: tok.get(3).and_then(|t| num(t)),
                    l: tok.get(4).and_then(|t| num(t)),
                    c: tok.get(5).and_then(|t| num(t)),
                });
            }
            ("model selector", Ctx::Selector) => f.selectors.last_mut().unwrap().1.push(tok[0].to_string()),
            ("model", Ctx::Model) => {
                // "Model_type I/O", "C_comp 1p 0.8p 1.2p", "Vinl = 0.8V"
                let m = f.models.last_mut().unwrap();
                let joined = lt.replace('=', " ");
                let t2: Vec<&str> = joined.split_whitespace().collect();
                let v = t2.get(1).and_then(|t| num(t));
                match t2[0].to_ascii_lowercase().as_str() {
                    "model_type" => m.model_type = t2.get(1).unwrap_or(&"").to_string(),
                    "c_comp" => m.c_comp = v,
                    "c_comp_pullup" | "c_comp_pulldown" | "c_comp_power_clamp" | "c_comp_gnd_clamp" => {
                        if let Some(x) = v {
                            m.c_comp_parts = Some(m.c_comp_parts.unwrap_or(0.0) + x);
                        }
                    }
                    _ => {}
                }
            }
            ("pullup" | "pulldown" | "gnd clamp" | "power clamp", Ctx::Model) if tok.len() >= 2 => {
                // typ column; a missing typ value (NA) falls back to min/max
                let i = num(tok[1]).or_else(|| {
                    let mm: Vec<f64> = tok.iter().skip(2).take(2).filter_map(|t| num(t)).collect();
                    (!mm.is_empty()).then(|| mm.iter().sum::<f64>() / mm.len() as f64)
                });
                let (Some(v), Some(i)) = (num(tok[0]), i) else { continue };
                let m = f.models.last_mut().unwrap();
                let t = match kw.as_str() {
                    "pullup" => &mut m.pullup,
                    "pulldown" => &mut m.pulldown,
                    "gnd clamp" => &mut m.gnd_clamp,
                    _ => &mut m.power_clamp,
                };
                if let Some(t) = t {
                    t.0.push((v, i));
                }
            }
            ("ramp", Ctx::Model) => {
                let m = f.models.last_mut().unwrap();
                let joined = lt.replace('=', " ");
                let t2: Vec<&str> = joined.split_whitespace().collect();
                let ratio = |s: &str| -> Option<(f64, f64)> {
                    let (a, b) = s.split_once('/')?;
                    Some((num(a)?, num(b)?))
                };
                match t2[0].to_ascii_lowercase().as_str() {
                    "dv/dt_r" => m.ramp_r = t2.get(1).and_then(|s| ratio(s)),
                    "dv/dt_f" => m.ramp_f = t2.get(1).and_then(|s| ratio(s)),
                    "r_load" => m.r_load = t2.get(1).and_then(|s| num(s)),
                    _ => {}
                }
            }
            _ => {}
        }
    }
    for m in &mut f.models {
        for t in [&mut m.pullup, &mut m.pulldown, &mut m.gnd_clamp, &mut m.power_clamp].into_iter().flatten() {
            t.0.sort_by(|a, b| a.0.total_cmp(&b.0));
            t.0.dedup_by(|a, b| a.0 == b.0);
        }
        // an empty table is no table
        for t in [&mut m.pullup, &mut m.pulldown, &mut m.gnd_clamp, &mut m.power_clamp] {
            if t.as_ref().is_some_and(|x| x.0.is_empty()) {
                *t = None;
            }
        }
    }
    if f.components.is_empty() && f.models.is_empty() {
        return Err("no [Component] or [Model] found – not an IBIS file?".into());
    }
    Ok(f)
}

/// Linear end model of one pin.
#[derive(Debug, Clone, Serialize)]
pub struct Linear {
    pub component: String,
    pub pin: String,
    pub signal: String,
    pub model: String,
    pub model_type: String,
    /// transmitter model (None: input only, or no usable pull-up/pull-down)
    pub tx: Option<EndModel>,
    /// open-circuit swing of the linearised driver (V)
    pub v_swing: Option<f64>,
    /// 10-90 % edge (s), from [Ramp]
    pub rise: Option<f64>,
    /// receiver model
    pub rx: EndModel,
    pub notes: Vec<String>,
}

impl IbisFile {
    pub fn model(&self, name: &str) -> Option<&Model> {
        self.models.iter().find(|m| m.name.eq_ignore_ascii_case(name))
    }
    fn selector(&self, name: &str) -> Option<&Vec<String>> {
        self.selectors.iter().find(|s| s.0.eq_ignore_ascii_case(name)).map(|s| &s.1)
    }
    pub fn component(&self, name: &str) -> Option<&Component> {
        if name.is_empty() && self.components.len() == 1 {
            return self.components.first();
        }
        self.components.iter().find(|c| c.name.eq_ignore_ascii_case(name))
    }

    /// Linear model of `pin` of `component`. `model` (KiCad's Sim.Ibis.Model)
    /// picks the entry of a [Model Selector]; otherwise the first entry is used.
    pub fn linear(&self, component: &str, pin: &str, model: Option<&str>) -> Result<Linear, String> {
        let comp = self.component(component).ok_or_else(|| {
            format!("component \"{component}\" not in the IBIS file (has: {})", self.components.iter().map(|c| c.name.as_str()).collect::<Vec<_>>().join(", "))
        })?;
        let pe = comp.pins.iter().find(|p| p.pin.eq_ignore_ascii_case(pin)).ok_or_else(|| format!("pin {pin} not in [Pin] of {}", comp.name))?;
        let mut notes = Vec::new();
        let mname = match self.selector(&pe.model) {
            Some(list) => {
                let pick = model.and_then(|m| list.iter().find(|x| x.eq_ignore_ascii_case(m))).or(list.first()).ok_or_else(|| format!("empty model selector {}", pe.model))?;
                notes.push(format!("model selector {}: {pick}", pe.model));
                pick.clone()
            }
            None => pe.model.clone(),
        };
        if ["power", "gnd", "nc"].iter().any(|x| mname.eq_ignore_ascii_case(x)) {
            return Err(format!("pin {pin} is {mname} in the IBIS file"));
        }
        let m = self.model(&mname).ok_or_else(|| format!("model {mname} not in the IBIS file"))?;
        if m.ami {
            notes.push("IBIS-AMI equalisation not evaluated (analog part only)".into());
        }
        let c_comp = m.c_comp.or(m.c_comp_parts).unwrap_or(0.0);
        let c = c_comp + pe.c.or(comp.c_pkg).unwrap_or(0.0);
        let l = pe.l.or(comp.l_pkg).unwrap_or(0.0);
        let r_pkg = pe.r.or(comp.r_pkg).unwrap_or(0.0);

        let vr = m.v_range.or(m.pullup_ref).unwrap_or(0.0);
        let vpu = m.pullup_ref.unwrap_or(vr);
        let vpd = m.pulldown_ref.unwrap_or(0.0);
        let vpc = m.power_clamp_ref.unwrap_or(vr);
        let vgc = m.gnd_clamp_ref.unwrap_or(0.0);
        let vcc = (vpu - vpd).abs().max(0.1);
        let tab = |t: &Option<Table>, v: f64| t.as_ref().map_or(0.0, |t| t.at(v));
        // current into the pin
        let clamp = |v: f64| tab(&m.gnd_clamp, v - vgc) + tab(&m.power_clamp, vpc - v);
        let hi = |v: f64| tab(&m.pullup, vpu - v) + clamp(v);
        let lo = |v: f64| tab(&m.pulldown, v - vpd) + clamp(v);
        let h = 0.05 * vcc;
        let slope = |f: &dyn Fn(f64) -> f64, v: f64| (f(v + h) - f(v - h)) / (2.0 * h);

        let t = m.model_type.to_ascii_lowercase();
        let drives = !(t.starts_with("input") || t == "terminator" || t.starts_with("series"));
        let mut tx = None;
        let mut v_swing = None;
        let mut rise = None;
        if drives {
            if m.pullup.is_none() || m.pulldown.is_none() {
                notes.push(format!(
                    "{} output: the {} level comes from outside the IC, no linear transmitter",
                    m.model_type,
                    if m.pullup.is_none() { "high" } else { "low" }
                ));
            } else {
                let rl = m.r_load.filter(|r| *r > 0.0).unwrap_or(50.0);
                // operating point: KCL i(v) = (vt - v)/rl, bisection
                let op = |f: &dyn Fn(f64) -> f64, vt: f64| -> f64 {
                    let g = |v: f64| f(v) - (vt - v) / rl;
                    let (mut a, mut b) = (vpd - vcc, vpu + vcc);
                    for _ in 0..80 {
                        let mid = 0.5 * (a + b);
                        if g(mid) > 0.0 {
                            b = mid;
                        } else {
                            a = mid;
                        }
                    }
                    0.5 * (a + b)
                };
                let thev = |f: &dyn Fn(f64) -> f64, vt: f64| -> Option<(f64, f64)> {
                    let v0 = op(f, vt);
                    let s = slope(f, v0);
                    (s > 1e-6).then(|| (1.0 / s, v0 - f(v0) / s))
                };
                match (thev(&hi, vpd), thev(&lo, vpu)) {
                    (Some((rh, vh)), Some((rlo, vl))) if vh > vl => {
                        tx = Some(EndModel { r: 0.5 * (rh + rlo) + r_pkg, c_pf: c * 1e12, l_nh: l * 1e9 });
                        v_swing = Some(vh - vl);
                        if (rh - rlo).abs() > 0.3 * (rh + rlo) {
                            notes.push(format!("pull-up {rh:.0} Ω / pull-down {rlo:.0} Ω averaged"));
                        }
                    }
                    _ => notes.push("I-V tables give no usable linear driver".into()),
                }
                let dts: Vec<f64> = [m.ramp_r, m.ramp_f].iter().flatten().map(|r| r.1).filter(|d| *d > 0.0).collect();
                if !dts.is_empty() {
                    // 20-80 % → 10-90 % for a Gaussian edge: 2·1.2816 / (2·0.8416)
                    rise = Some(dts.iter().sum::<f64>() / dts.len() as f64 * 1.5227);
                } else {
                    notes.push("no [Ramp]: edge rate from the profile".into());
                }
            }
        }
        // receiver: small-signal conductance of the clamps / terminators at mid supply
        let vm = vpd + 0.5 * vcc;
        let mut g = slope(&clamp, vm).max(0.0);
        if let Some(r) = m.rgnd.filter(|r| *r > 0.0) {
            g += 1.0 / r;
        }
        if let Some(r) = m.rpower.filter(|r| *r > 0.0) {
            g += 1.0 / r;
        }
        let r_rx = if g > 1.0 / 5000.0 { 1.0 / g } else { 0.0 };
        if r_rx > 0.0 {
            notes.push(format!("on-die termination {r_rx:.0} Ω (from clamp curves / terminator)"));
        }
        Ok(Linear {
            component: comp.name.clone(),
            pin: pe.pin.clone(),
            signal: pe.signal.clone(),
            model: m.name.clone(),
            model_type: m.model_type.clone(),
            tx,
            v_swing,
            rise,
            rx: EndModel { r: r_rx, c_pf: c * 1e12, l_nh: l * 1e9 },
            notes,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // a small file in the style of the IBIS specification examples
    const SAMPLE: &str = "[IBIS Ver]      3.2
[Comment char]  #_char
[File name]     test.ibs
[Component]     TESTCHIP
[Manufacturer]  kisi
[Package]
R_pkg           50m     40m     60m
L_pkg           2n      NA      NA
C_pkg           0.5p    NA      NA
[Pin]   signal_name     model_name      R_pin   L_pin   C_pin
  A1    VCC             POWER
  A2    DIN             IN33            55m     NA      0.2p
  A3    DOUT            OUT33           0.1     1.5n    0.3p
  B1    DQ0             DQ_SEL
[Model Selector] DQ_SEL
DQ_ODT60   with 60 ohm termination
DQ_NOODT   without termination
[Model]         IN33
Model_type      Input
Vinl = 0.8V
C_comp          1.0pF   0.5pF   2.0pF
[Voltage range] 3.3V    3.0V    3.6V
[GND_clamp]
   -3.3V    -50.0m     NA        NA
   -0.7V      0         NA        NA
   3.3V      0         NA        NA
[Model]         OUT33
Model_type      Output
C_comp          3.0pF   2.0pF   4.0pF
[Voltage range] 3.3V    3.0V    3.6V
[Pulldown]
   -3.3V    -132.0m   NA   NA
   0.0V      0        NA   NA
   3.3V      132.0m   NA   NA # 25 ohm, linear
[Pullup]
   -3.3V     132.0m   NA   NA
   0.0V      0        NA   NA
   3.3V     -132.0m   NA   NA
[Ramp]
dV/dt_r         1.98/0.5n     NA     NA
dV/dt_f         1.98/0.5n     NA     NA
R_load = 50
[Model]         DQ_ODT60
Model_type      I/O
C_comp          1.2p
[Voltage range] 1.2V
[Pulldown]
   -1.2V    -0.034    NA   NA
   1.2V      0.034    NA   NA
[Pullup]
   -1.2V     0.034    NA   NA
   1.2V     -0.034    NA   NA
[GND clamp]
   -1.2V    -0.01     NA   NA
   1.2V      0.01     NA   NA
[POWER clamp]
   -1.2V     0.01     NA   NA
   1.2V     -0.01     NA   NA
[Model]         DQ_NOODT
Model_type      I/O
C_comp          1.2p
[Voltage range] 1.2V
[Pulldown]
   -1.2V    -0.034    NA   NA
   1.2V      0.034    NA   NA
[Pullup]
   -1.2V     0.034    NA   NA
   1.2V     -0.034    NA   NA
[END]
";

    #[test]
    fn numbers() {
        assert_eq!(num("3.0pF"), Some(3.0e-12));
        assert_eq!(num("50m"), Some(0.05));
        assert_eq!(num("-5.0V"), Some(-5.0));
        assert_eq!(num("1.2e-3"), Some(1.2e-3));
        assert_eq!(num("2.5E3"), Some(2500.0));
        assert_eq!(num("150Ohm"), Some(150.0));
        assert_eq!(num("1M"), Some(1e6));
        assert_eq!(num("NA"), None);
    }

    #[test]
    fn linear_ends() {
        let f = parse(SAMPLE).unwrap();
        assert_eq!(f.components.len(), 1);
        assert_eq!(f.models.len(), 4);
        // driver: 25 Ω linear pull-up/pull-down + 0.1 Ω pin, 0.5 ns 20-80 %
        let o = f.linear("TESTCHIP", "A3", None).unwrap();
        let tx = o.tx.unwrap();
        assert!((tx.r - 25.1).abs() < 0.2, "r {}", tx.r);
        assert!((o.v_swing.unwrap() - 3.3).abs() < 0.05, "swing {:?}", o.v_swing);
        assert!((o.rise.unwrap() - 0.761e-9).abs() < 0.01e-9);
        assert!((tx.c_pf - 3.3).abs() < 1e-9 && (tx.l_nh - 1.5).abs() < 1e-9);
        // input: high impedance, C_comp + C_pin, L from the package
        let i = f.linear("testchip", "a2", None).unwrap();
        assert!(i.tx.is_none());
        assert_eq!(i.rx.r, 0.0);
        assert!((i.rx.c_pf - 1.2).abs() < 1e-9 && (i.rx.l_nh - 2.0).abs() < 1e-9);
        // model selector: ODT from the linear clamps (2 × 120 Ω → 60 Ω)
        let d = f.linear("TESTCHIP", "B1", None).unwrap();
        assert_eq!(d.model, "DQ_ODT60");
        assert!((d.rx.r - 60.0).abs() < 0.5, "odt {}", d.rx.r);
        let d2 = f.linear("TESTCHIP", "B1", Some("dq_noodt")).unwrap();
        assert_eq!(d2.model, "DQ_NOODT");
        assert_eq!(d2.rx.r, 0.0);
        assert!(f.linear("TESTCHIP", "A1", None).is_err());
        assert!(f.linear("TESTCHIP", "Z9", None).is_err());
        assert!(f.linear("OTHER", "A2", None).is_err());
    }
}

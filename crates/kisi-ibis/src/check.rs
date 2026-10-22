//! Checks in the spirit of IBISCHK, where they are cheap: required keywords, references
//! between sections, table sanity, [Ramp] against the waveforms, waveform end points against
//! the I-V tables, table coverage of the voltage range.
//!
//! Tolerances are kisi's choices (marked in the messages where they matter): monotonicity
//! ignores reversals below 0.01 % of a column's largest current, end points may differ from
//! the DC solution by 3 % of the supply, [Ramp] from the matching waveform by 25 %.

use crate::derive::{self, State};
use crate::num::{Corner, Tmm};
use crate::types::*;
use std::collections::{BTreeMap, HashMap, HashSet};

/// IBIS versions (ratified).
pub const VERSIONS: &[&str] = &["1.1", "2.0", "2.1", "3.0", "3.1", "3.2", "4.0", "4.1", "4.2", "5.0", "5.1", "6.0", "6.1", "7.0", "7.1", "7.2", "8.0"];
/// Newest version whose keywords this parser covers.
pub const COVERED: f64 = 7.2;

const MONO_TOL: f64 = 1e-4;
const ENDPOINT_TOL: f64 = 0.03;
const RAMP_TOL: f64 = 0.25;

struct Out {
    v: Vec<Issue>,
}

impl Out {
    fn push(&mut self, severity: Severity, line: Option<usize>, code: &'static str, at: Option<String>, message: String) {
        self.v.push(Issue { severity, line, code, at, message });
    }
}

fn si(v: f64, unit: &str) -> String {
    let a = v.abs();
    let (k, p) = if a == 0.0 {
        (1.0, "")
    } else if a >= 1e6 {
        (1e-6, "M")
    } else if a >= 1e3 {
        (1e-3, "k")
    } else if a >= 1.0 {
        (1.0, "")
    } else if a >= 1e-3 {
        (1e3, "m")
    } else if a >= 1e-6 {
        (1e6, "µ")
    } else if a >= 1e-9 {
        (1e9, "n")
    } else if a >= 1e-12 {
        (1e12, "p")
    } else {
        (1e15, "f")
    };
    let x = v * k;
    let s = if x.abs() >= 100.0 { format!("{x:.0}") } else if x.abs() >= 10.0 { format!("{x:.1}") } else { format!("{x:.3}") };
    let s = if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_string() } else { s };
    format!("{s} {p}{unit}")
}

/// All checks of a parsed file. `file_name`: the name of the file as stored (compared with
/// [File Name]).
pub fn check(f: &IbisFile, file_name: Option<&str>) -> Vec<Issue> {
    let mut o = Out { v: vec![] };
    header(f, file_name, &mut o);
    let mut seen: HashMap<String, usize> = HashMap::new();
    for c in &f.components {
        if let Some(l) = seen.insert(c.name.to_ascii_lowercase(), c.line) {
            o.push(Severity::Error, Some(c.line), "duplicate-name", Some(format!("component {}", c.name)), format!("component name given twice (also line {l})"));
        }
        component(f, c, &mut o);
    }
    selectors(f, &mut o);
    let mut seen: HashMap<String, usize> = HashMap::new();
    for m in &f.models {
        if let Some(l) = seen.insert(m.name.clone(), m.line) {
            o.push(Severity::Error, Some(m.line), "duplicate-name", Some(format!("model {}", m.name)), format!("model name given twice (also line {l})"));
        }
        model(f, m, &mut o);
    }
    for p in &f.package_models {
        let at = Some(format!("package model {}", p.name));
        if let Some(n) = p.number_of_pins {
            if n != p.pins.len() {
                o.push(Severity::Warning, Some(p.line), "package-model", at.clone(), format!("[Number Of Pins] {n} but {} entries in [Pin Numbers]", p.pins.len()));
            }
        }
        for (n, mx) in [("resistance", &p.r), ("inductance", &p.l), ("capacitance", &p.c)] {
            let Some(mx) = mx else { continue };
            let mut seen = HashSet::new();
            if let Some(e) = mx.entries.iter().find(|e| !seen.insert((e.0.min(e.1), e.0.max(e.1)))) {
                let pin = |i: usize| p.pins.get(i).cloned().unwrap_or_default();
                o.push(Severity::Warning, Some(mx.line), "package-model", at.clone(), format!("{n} matrix: entry ({}, {}) given twice", pin(e.0), pin(e.1)));
            }
        }
        if p.r.is_none() && p.l.is_none() && p.c.is_none() {
            o.push(Severity::Warning, Some(p.line), "package-model", at, "no resistance, inductance or capacitance matrix".into());
        }
    }
    o.v
}

fn header(f: &IbisFile, file_name: Option<&str>, o: &mut Out) {
    let h = &f.header;
    let at = || Some("header".to_string());
    match &h.ibis_ver {
        None => o.push(Severity::Error, None, "missing-keyword", at(), "[IBIS Ver] missing".into()),
        Some(v) => {
            if !VERSIONS.contains(&v.as_str()) {
                o.push(Severity::Warning, h.ibis_ver_line, "version", at(), format!("[IBIS Ver] {v} is not a released IBIS version ({})", VERSIONS.join(", ")));
            }
            if h.version().is_some_and(|x| x > COVERED + 1e-9) {
                o.push(Severity::Note, h.ibis_ver_line, "version", at(), format!("IBIS {v}: kisi reads the keywords up to IBIS {COVERED}; newer ones are reported as unknown"));
            }
        }
    }
    match &h.file_name {
        None => o.push(Severity::Error, None, "missing-keyword", at(), "[File Name] missing".into()),
        Some(n) => {
            let ext = n.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
            if !n.contains('.') || !["ibs", "pkg", "ebd", "ims"].contains(&ext.as_str()) {
                o.push(Severity::Warning, None, "file-name", at(), format!("[File Name] '{n}' has no .ibs, .pkg, .ebd or .ims extension"));
            }
            if let Some(actual) = file_name {
                let base = actual.rsplit(['/', '\\']).next().unwrap_or(actual);
                if !base.eq_ignore_ascii_case(n) {
                    o.push(Severity::Warning, None, "file-name", at(), format!("[File Name] '{n}' differs from the file's name '{base}' (renamed or a different file?)"));
                }
            }
        }
    }
    if h.file_rev.is_none() {
        o.push(Severity::Error, None, "missing-keyword", at(), "[File Rev] missing".into());
    }
    let pkg_only = h.file_name.as_deref().is_some_and(|n| !n.to_ascii_lowercase().ends_with(".ibs"));
    if f.components.is_empty() && !pkg_only && h.ibis_ver.is_some() {
        o.push(Severity::Error, None, "missing-keyword", None, "no [Component] in the file".into());
    }
}

fn range_check(o: &mut Out, line: Option<usize>, at: &Option<String>, what: &str, v: Option<f64>, lo: f64, hi: f64, unit: &str) {
    if let Some(x) = v {
        if !(lo..=hi).contains(&x) {
            o.push(Severity::Warning, line, "out-of-range", at.clone(), format!("{what} = {} is outside {} … {} (scaling factor missing or wrong?)", si(x, unit), si(lo, unit), si(hi, unit)));
        }
    }
}

fn component(f: &IbisFile, c: &Component, o: &mut Out) {
    let at = Some(format!("component {}", c.name));
    if c.manufacturer.as_deref().map_or(true, |m| m.trim().is_empty()) {
        o.push(Severity::Error, Some(c.line), "missing-keyword", at.clone(), "[Manufacturer] missing".into());
    }
    let pl = c.package.line.or(Some(c.line));
    if c.package.line.is_none() {
        o.push(Severity::Error, Some(c.line), "missing-keyword", at.clone(), "[Package] missing".into());
    } else {
        for (n, v, hi, unit) in [("R_pkg", &c.package.r, 10.0, "Ω"), ("L_pkg", &c.package.l, 100e-9, "H"), ("C_pkg", &c.package.c, 100e-12, "F")] {
            match v {
                None => o.push(Severity::Error, pl, "missing-keyword", at.clone(), format!("[Package] {n} missing")),
                Some(t) => {
                    for x in t.values() {
                        range_check(o, pl, &at, &format!("[Package] {n}"), Some(x), 0.0, hi, unit);
                    }
                }
            }
        }
    }
    if c.pins.is_empty() {
        o.push(Severity::Error, Some(c.line), "missing-keyword", at.clone(), "[Pin] missing or empty".into());
    }
    let mut names: HashMap<&str, usize> = HashMap::new();
    let mut missing: BTreeMap<&str, Vec<&Pin>> = BTreeMap::new();
    for p in &c.pins {
        if let Some(l) = names.insert(p.name.as_str(), p.line) {
            o.push(Severity::Error, Some(p.line), "duplicate-pin", at.clone(), format!("pin {} listed twice (also line {l})", p.name));
        }
        if p.reserved().is_none() && f.model(&p.model).is_none() && f.selector(&p.model).is_none() {
            missing.entry(p.model.as_str()).or_default().push(p);
        }
        range_check(o, Some(p.line), &at, &format!("pin {} R_pin", p.name), p.r, 0.0, 10.0, "Ω");
        range_check(o, Some(p.line), &at, &format!("pin {} L_pin", p.name), p.l, 0.0, 100e-9, "H");
        range_check(o, Some(p.line), &at, &format!("pin {} C_pin", p.name), p.c, 0.0, 100e-12, "F");
    }
    for (m, pins) in missing {
        let list: Vec<String> = pins.iter().take(8).map(|p| p.name.clone()).collect();
        o.push(
            Severity::Error,
            Some(pins[0].line),
            "unknown-model",
            at.clone(),
            format!("model '{m}' of {} pin(s) ({}{}) is not defined in this file (neither [Model] nor [Model Selector])", pins.len(), list.join(", "), if pins.len() > 8 { ", …" } else { "" }),
        );
    }
    let known = |n: &str| c.pin(n).is_some();
    let mut in_pair: HashMap<&str, usize> = HashMap::new();
    for d in &c.diff_pins {
        for n in [&d.pin, &d.inv_pin] {
            if !known(n) {
                o.push(Severity::Error, Some(d.line), "unknown-pin", at.clone(), format!("[Diff Pin] pin {n} is not in [Pin]"));
            }
            if let Some(l) = in_pair.insert(n.as_str(), d.line) {
                o.push(Severity::Warning, Some(d.line), "duplicate-pin", at.clone(), format!("[Diff Pin] pin {n} is in two pairs (also line {l})"));
            }
        }
        range_check(o, Some(d.line), &at, &format!("[Diff Pin] {} vdiff", d.pin), d.vdiff, 0.0, 2.0, "V");
    }
    for m in &c.pin_mapping {
        if !known(&m.pin) {
            o.push(Severity::Warning, Some(m.line), "unknown-pin", at.clone(), format!("[Pin Mapping] pin {} is not in [Pin]", m.pin));
        }
    }
    for s in &c.series_pins {
        for n in [&s.pin1, &s.pin2] {
            if !known(n) {
                o.push(Severity::Error, Some(s.line), "unknown-pin", at.clone(), format!("[Series Pin Mapping] pin {n} is not in [Pin]"));
            }
        }
        match f.model(&s.model) {
            None => o.push(Severity::Error, Some(s.line), "unknown-model", at.clone(), format!("[Series Pin Mapping] model '{}' is not defined", s.model)),
            Some(m) if !(m.kind().series || m.kind().series_switch) => {
                o.push(Severity::Error, Some(s.line), "model-type", at.clone(), format!("[Series Pin Mapping] model '{}' is {}, not Series or Series_switch", s.model, m.model_type.as_deref().unwrap_or("untyped")))
            }
            _ => {}
        }
    }
    if let Some(pm) = &c.package_model {
        if f.package_model(pm).is_none() {
            o.push(Severity::Warning, Some(c.line), "unknown-package-model", at, format!("[Package Model] '{pm}' is not defined in this file (a separate .pkg file is not read): the [Pin] / [Package] values apply"));
        }
    }
}

fn selectors(f: &IbisFile, o: &mut Out) {
    let models: HashSet<&str> = f.models.iter().map(|m| m.name.as_str()).collect();
    let mut seen: HashMap<String, usize> = HashMap::new();
    for s in &f.model_selectors {
        let at = Some(format!("model selector {}", s.name));
        if let Some(l) = seen.insert(s.name.to_ascii_lowercase(), s.line) {
            o.push(Severity::Error, Some(s.line), "duplicate-name", at.clone(), format!("[Model Selector] name given twice (also line {l})"));
        }
        if models.contains(s.name.as_str()) {
            o.push(Severity::Warning, Some(s.line), "duplicate-name", at.clone(), "a [Model] has the same name as this [Model Selector]".into());
        }
        if s.entries.is_empty() {
            o.push(Severity::Error, Some(s.line), "empty", at.clone(), "[Model Selector] lists no model".into());
        }
        for e in &s.entries {
            if f.model(&e.model).is_none() {
                o.push(Severity::Error, Some(e.line), "unknown-model", at.clone(), format!("model '{}' is not defined in this file", e.model));
            }
        }
    }
}

/// Expected direction of a table's current with rising x: +1 increasing, −1 decreasing.
fn direction(which: &str) -> f64 {
    match which {
        "[Pullup]" | "[POWER Clamp]" => -1.0,
        _ => 1.0,
    }
}

fn table_checks(which: &str, t: &Table, at: &Option<String>, o: &mut Out) {
    if t.rows.is_empty() {
        o.push(Severity::Error, Some(t.line), "empty-table", at.clone(), format!("{which} has no rows"));
        return;
    }
    if let Some(w) = t.rows.windows(2).find(|w| w[1].x <= w[0].x) {
        o.push(Severity::Warning, Some(w[1].line), "non-monotonic", at.clone(), format!("{which}: voltages not increasing ({} after {})", si(w[1].x, "V"), si(w[0].x, "V")));
    }
    for c in [Corner::Min, Corner::Max] {
        let na = t.rows.iter().filter(|r| (if c == Corner::Min { r.y.min } else { r.y.max }).is_none()).count();
        if na > 0 && na < t.rows.len() {
            o.push(Severity::Warning, Some(t.line), "partial-na", at.clone(), format!("{which}: the {} column has NA in {na} of {} rows (typ is used there)", c.name(), t.rows.len()));
        }
    }
    let dir = direction(which);
    for c in Corner::ALL {
        if !t.has_corner(c) {
            continue;
        }
        let pts: Vec<(f64, usize)> = t.rows.iter().filter_map(|r| r.y.get(c).map(|y| (y, r.line))).collect();
        let big = pts.iter().map(|p| p.0.abs()).fold(0.0, f64::max);
        let tol = MONO_TOL * big;
        let rev: Vec<usize> = pts.windows(2).filter(|w| dir * (w[1].0 - w[0].0) < -tol).map(|w| w[1].1).collect();
        if !rev.is_empty() {
            o.push(
                Severity::Warning,
                Some(rev[0]),
                "non-monotonic",
                at.clone(),
                format!("{which} ({}): current not monotonic, {} reversal(s), first at line {} (expected {} with the voltage; kisi tolerance 0.01 % of the largest current)", c.name(), rev.len(), rev[0], if dir > 0.0 { "rising" } else { "falling" }),
            );
        }
    }
}

/// Pin-voltage range a table covers (typ references).
fn coverage(which: &str, t: &Table, r: &derive::Rails) -> Option<(f64, f64)> {
    let (lo, hi) = t.x_range()?;
    Some(match which {
        "[Pulldown]" => (lo + r.pulldown, hi + r.pulldown),
        "[GND Clamp]" => (lo + r.gnd_clamp, hi + r.gnd_clamp),
        "[Pullup]" => (r.pullup - hi, r.pullup - lo),
        _ => (r.power_clamp - hi, r.power_clamp - lo),
    })
}

fn model(f: &IbisFile, m: &Model, o: &mut Out) {
    let at = Some(format!("model {}", m.name));
    let ml = Some(m.line);
    let k = m.kind();
    if m.model_type.is_none() && m.line_of("model_type").is_none() {
        o.push(Severity::Error, ml, "missing-keyword", at.clone(), "Model_type missing".into());
    }
    // C_comp
    if m.c_comp.is_none() && m.c_comp_pullup.is_none() && m.c_comp_pulldown.is_none() && m.c_comp_power_clamp.is_none() && m.c_comp_gnd_clamp.is_none() {
        o.push(Severity::Error, ml, "missing-keyword", at.clone(), "C_comp missing (neither C_comp nor C_comp_pullup/_pulldown/_power_clamp/_gnd_clamp)".into());
    }
    if let Some((c, x)) = Corner::ALL.iter().filter_map(|&c| m.c_comp_at(c).map(|x| (c, x))).find(|(_, x)| !(0.0..=100e-12).contains(x)) {
        range_check(o, m.line_of("c_comp").or(ml), &at, &format!("C_comp ({})", c.name()), Some(x), 0.0, 100e-12, "F");
    }
    // supply
    let series_like = k.series || k.series_switch;
    let r_typ = derive::rails(m, Corner::Typ);
    if !series_like && r_typ.is_none() {
        o.push(Severity::Error, ml, "missing-keyword", at.clone(), "neither [Voltage Range] nor [Pullup Reference]: the supply voltage is unknown".into());
    }
    if let (Some(vr), false) = (&m.voltage_range, k.ecl) {
        for x in vr.values() {
            range_check(o, m.line_of("voltage_range"), &at, "[Voltage Range]", Some(x), 0.0, 60.0, "V");
        }
    }
    if let Some(t) = &m.temperature_range {
        for x in t.values() {
            if !(-100.0..=250.0).contains(&x) {
                o.push(Severity::Warning, m.line_of("temperature_range"), "out-of-range", at.clone(), format!("[Temperature Range] {x} °C is outside −100 … 250 °C"));
            }
        }
    }
    // receiver thresholds
    if k.input {
        let vinl = m.vinl.or_else(|| m.spec("Vinl").and_then(|t| t.typ));
        let vinh = m.vinh.or_else(|| m.spec("Vinh").and_then(|t| t.typ));
        let vth = m.receiver_thresholds.iter().any(|r| r.name.eq_ignore_ascii_case("Vth"));
        if (vinl.is_none() || vinh.is_none()) && !vth {
            o.push(Severity::Warning, ml, "missing-threshold", at.clone(), format!("{} missing: simulators then assume defaults (0.8 V / 2.0 V for non-ECL inputs)", if vinl.is_none() && vinh.is_none() { "Vinl and Vinh" } else if vinl.is_none() { "Vinl" } else { "Vinh" }));
        }
        if let (Some(l), Some(h)) = (vinl, vinh) {
            if l >= h {
                o.push(Severity::Error, m.line_of("vinl").or(ml), "threshold", at.clone(), format!("Vinl {} is not below Vinh {}", si(l, "V"), si(h, "V")));
            }
        }
    }
    // tables by Model_type
    if k.has_pulldown && m.pulldown.is_none() {
        o.push(Severity::Error, ml, "missing-keyword", at.clone(), format!("[Pulldown] missing (required for {})", m.model_type.as_deref().unwrap_or("")));
    }
    if k.has_pullup && m.pullup.is_none() {
        o.push(Severity::Error, ml, "missing-keyword", at.clone(), format!("[Pullup] missing (required for {})", m.model_type.as_deref().unwrap_or("")));
    }
    let term = m.rgnd.is_some() || m.rpower.is_some() || m.rac.is_some() || m.cac.is_some();
    if term && !k.terminator {
        o.push(Severity::Error, ml, "model-type", at.clone(), "[Rgnd], [Rpower], [Rac] or [Cac] given, but Model_type is not Terminator".into());
    }
    if k.terminator && !term && m.gnd_clamp.is_none() && m.power_clamp.is_none() {
        o.push(Severity::Warning, ml, "missing-keyword", at.clone(), "Terminator without [Rgnd], [Rpower], [Rac]/[Cac] or clamp tables".into());
    }
    if k.series && m.series.is_empty() {
        o.push(Severity::Error, ml, "missing-keyword", at.clone(), "Series model without series elements ([R Series], [L Series], [C Series], [Series Current], [Series MOSFET])".into());
    }
    if !k.series_switch && (m.on.is_some() || m.off.is_some()) {
        o.push(Severity::Error, ml, "model-type", at.clone(), "[On] / [Off] are only allowed in Series_switch models".into());
    }
    if k.series_switch {
        if m.on.is_none() || m.off.is_none() {
            o.push(Severity::Error, ml, "missing-keyword", at.clone(), "Series_switch model needs [On] and [Off]".into());
        }
        if !m.series.is_empty() {
            o.push(Severity::Error, ml, "model-type", at.clone(), "Series_switch model has series elements outside [On] / [Off]".into());
        }
    }
    for (which, t) in [("[Pulldown]", &m.pulldown), ("[Pullup]", &m.pullup), ("[GND Clamp]", &m.gnd_clamp), ("[POWER Clamp]", &m.power_clamp)] {
        if let Some(t) = t {
            table_checks(which, t, &at, o);
        }
    }
    // coverage of the voltage range: driver tables −Vcc … 2·Vcc each, clamps together
    if let (Some(r), false) = (r_typ, k.ecl) {
        let vcc = r.span();
        let tol = 0.02 * vcc.abs();
        let (want_lo, want_hi) = (r.pulldown - vcc, r.pullup + vcc);
        let need = |lo: f64, hi: f64| lo > want_lo + tol || hi < want_hi - tol;
        for (which, t) in [("[Pulldown]", &m.pulldown), ("[Pullup]", &m.pullup)] {
            if let Some((lo, hi)) = t.as_ref().filter(|t| !t.is_empty()).and_then(|t| coverage(which, t, &r)) {
                if need(lo, hi) {
                    o.push(Severity::Warning, t.as_ref().map(|t| t.line), "coverage", at.clone(), format!("{which} covers pin voltages {} … {}; IBIS asks for −Vcc … 2·Vcc ({} … {})", si(lo, "V"), si(hi, "V"), si(want_lo, "V"), si(want_hi, "V")));
                }
            }
        }
        let cl: Vec<(f64, f64)> = [("[GND Clamp]", &m.gnd_clamp), ("[POWER Clamp]", &m.power_clamp)].iter().filter_map(|(w, t)| t.as_ref().filter(|t| !t.is_empty()).and_then(|t| coverage(w, t, &r))).collect();
        if !cl.is_empty() {
            let lo = cl.iter().map(|c| c.0).fold(f64::INFINITY, f64::min);
            let hi = cl.iter().map(|c| c.1).fold(f64::NEG_INFINITY, f64::max);
            if need(lo, hi) {
                let l = m.gnd_clamp.as_ref().or(m.power_clamp.as_ref()).map(|t| t.line);
                o.push(Severity::Warning, l, "coverage", at.clone(), format!("the clamp tables cover pin voltages {} … {}; IBIS asks for −Vcc … 2·Vcc ({} … {})", si(lo, "V"), si(hi, "V"), si(want_lo, "V"), si(want_hi, "V")));
            }
        }
    }
    ramp_checks(m, k, &at, o);
    waveform_checks(m, k, &at, o);
    for s in &m.add_submodel {
        if !f.submodels.iter().any(|x| x.name == s.name) {
            o.push(Severity::Error, Some(s.line), "unknown-submodel", at.clone(), format!("[Add Submodel] '{}' is not defined", s.name));
        }
    }
    for d in &m.driver_schedule {
        if f.model(&d.model).is_none() {
            o.push(Severity::Error, Some(d.line), "unknown-model", at.clone(), format!("[Driver Schedule] model '{}' is not defined", d.model));
        }
    }
}

fn ramp_checks(m: &Model, k: ModelKind, at: &Option<String>, o: &mut Out) {
    let Some(r) = &m.ramp else {
        if k.output {
            o.push(Severity::Error, Some(m.line), "missing-keyword", at.clone(), "[Ramp] missing (required for driver models)".into());
        }
        return;
    };
    let rl = Some(r.line);
    if r.r_load.is_some_and(|x| x <= 0.0) {
        o.push(Severity::Error, rl, "ramp", at.clone(), "[Ramp] R_load must be positive".into());
    }
    for (name, e) in [("dV/dt_r", &r.rising), ("dV/dt_f", &r.falling)] {
        let Some(e) = e else {
            if k.output {
                o.push(Severity::Warning, rl, "ramp", at.clone(), format!("[Ramp] {name} missing"));
            }
            continue;
        };
        for c in Corner::ALL {
            let Some(d) = (match c {
                Corner::Typ => e.typ,
                Corner::Min => e.min,
                Corner::Max => e.max,
            }) else {
                continue;
            };
            if d.dv <= 0.0 || d.dt <= 0.0 {
                o.push(Severity::Error, rl, "ramp-sign", at.clone(), format!("[Ramp] {name} ({}): dV and dt must both be positive (found {}/{})", c.name(), si(d.dv, "V"), si(d.dt, "s")));
            }
            if let Some(v) = derive::rails(m, c).map(|x| x.span()).filter(|v| *v > 0.0) {
                if d.dv > 1.05 * v {
                    o.push(Severity::Warning, rl, "ramp", at.clone(), format!("[Ramp] {name} ({}): dV {} is larger than the supply {} (dV is 60 % of the swing)", c.name(), si(d.dv, "V"), si(v, "V")));
                }
            }
        }
    }
    // against the waveform with the same load: rising into R_load at the pulldown reference,
    // falling into R_load at the pullup reference
    let Some(rails) = derive::rails(m, Corner::Typ) else { return };
    for (rising, e) in [(true, &r.rising), (false, &r.falling)] {
        let Some(d) = e.as_ref().and_then(|e| e.typ).filter(|d| d.dv > 0.0 && d.dt > 0.0) else { continue };
        let v_target = if rising { rails.pulldown } else { rails.pullup };
        let tol_v = 0.05 * rails.span().abs().max(0.1);
        let w = m.waveforms.iter().find(|w| w.rising == rising && w.r_fixture.is_some_and(|x| (x - r.r_load()).abs() <= 0.01 * r.r_load()) && w.v_fixture.is_some_and(|v| (v - v_target).abs() <= tol_v));
        let Some(w) = w else { continue };
        let Some(ed) = derive::edge(w, Corner::Typ) else { continue };
        let dv_w = 0.6 * (ed.v_end - ed.v_start).abs();
        let off_v = (dv_w - d.dv).abs() / d.dv.max(1e-12);
        let off_t = ed.t20_80.map(|t| (t - d.dt).abs() / d.dt.max(1e-15));
        if off_v > RAMP_TOL || off_t.is_some_and(|x| x > RAMP_TOL) {
            o.push(
                Severity::Warning,
                Some(r.line),
                "ramp-waveform",
                at.clone(),
                format!(
                    "[Ramp] {} {}/{} differs from the {} into {} to {} (line {}): {}/{} for 20–80 % (kisi tolerance 25 %)",
                    if rising { "dV/dt_r" } else { "dV/dt_f" },
                    si(d.dv, "V"),
                    si(d.dt, "s"),
                    if rising { "[Rising Waveform]" } else { "[Falling Waveform]" },
                    si(w.r_fixture.unwrap_or(0.0), "Ω"),
                    si(w.v_fixture.unwrap_or(0.0), "V"),
                    w.line,
                    si(dv_w, "V"),
                    ed.t20_80.map(|t| si(t, "s")).unwrap_or_else(|| "–".into())
                ),
            );
        }
    }
}

fn waveform_checks(m: &Model, k: ModelKind, at: &Option<String>, o: &mut Out) {
    let rails = derive::rails(m, Corner::Typ);
    for w in &m.waveforms {
        let kind = if w.rising { "[Rising Waveform]" } else { "[Falling Waveform]" };
        let wl = Some(w.line);
        if w.r_fixture.is_none() {
            o.push(Severity::Error, wl, "missing-parameter", at.clone(), format!("{kind}: R_fixture missing"));
        } else if w.r_fixture.is_some_and(|r| r <= 0.0) {
            o.push(Severity::Error, wl, "out-of-range", at.clone(), format!("{kind}: R_fixture must be positive"));
        }
        if w.v_fixture.is_none() {
            o.push(Severity::Error, wl, "missing-parameter", at.clone(), format!("{kind}: V_fixture missing"));
        }
        if w.table.rows.is_empty() {
            o.push(Severity::Error, wl, "empty-table", at.clone(), format!("{kind} has no rows"));
            continue;
        }
        if let Some(x) = w.table.rows.windows(2).find(|x| x[1].x <= x[0].x) {
            o.push(Severity::Warning, Some(x[1].line), "non-monotonic", at.clone(), format!("{kind}: time not increasing ({} after {})", si(x[1].x, "s"), si(x[0].x, "s")));
        }
        let Some(e) = derive::edge(w, Corner::Typ) else { continue };
        if (w.rising && e.v_end <= e.v_start) || (!w.rising && e.v_end >= e.v_start) {
            o.push(Severity::Error, wl, "waveform-direction", at.clone(), format!("{kind} goes from {} to {}: wrong direction", si(e.v_start, "V"), si(e.v_end, "V")));
            continue;
        }
        // end points against the DC solution of the I-V tables into the fixture
        let (Some(r_fix), Some(rails)) = (w.r_fixture, rails) else { continue };
        if w.r_dut.is_some() || r_fix <= 0.0 || !k.output {
            continue;
        }
        let low = if k.has_pulldown { State::Low } else { State::Off };
        let high = if k.has_pullup { State::High } else { State::Off };
        let (s0, s1) = if w.rising { (low, high) } else { (high, low) };
        let tol = ENDPOINT_TOL * rails.span().abs();
        let mut worst: Option<(f64, String)> = None;
        for c in Corner::ALL {
            if !w.table.has_corner(c) {
                continue;
            }
            let (Some(ec), Some(vf)) = (derive::edge(w, c), w.v_fixture_at(c)) else { continue };
            for (state, v, which) in [(s0, ec.v_start, "starts"), (s1, ec.v_end, "ends")] {
                let Some(dc) = derive::dc_level(m, c, state, r_fix, vf) else { continue };
                let d = (v - dc).abs();
                if d > tol.max(0.01) && worst.as_ref().map_or(true, |x| d > x.0) {
                    worst = Some((d, format!("{kind} into {} to {} {which} at {} ({}), the I-V tables give {} (kisi tolerance 3 % of the supply)", si(r_fix, "Ω"), si(vf, "V"), si(v, "V"), c.name(), si(dc, "V"))));
                }
            }
        }
        if let Some((_, msg)) = worst {
            o.push(Severity::Warning, wl, "waveform-endpoint", at.clone(), msg);
        }
    }
    if k.output && k.has_pullup && k.has_pulldown && !k.ecl {
        let (nr, nf) = (m.rising().count(), m.falling().count());
        if nr < 2 || nf < 2 {
            o.push(
                Severity::Note,
                Some(m.line),
                "waveforms",
                at.clone(),
                format!("{nr} rising and {nf} falling waveform(s): two per edge (loads to ground and to the supply) give the best switching coefficients (IBIS Cookbook)"),
            );
        }
    }
}

/// Keep at most `keep` findings of the same code at the same place; a note says how many more.
pub fn condense(mut v: Vec<Issue>, keep: usize) -> Vec<Issue> {
    v.sort_by_key(|i| i.line.unwrap_or(usize::MAX));
    let mut count: HashMap<(&'static str, Option<String>), usize> = HashMap::new();
    let mut out = Vec::new();
    for i in v {
        if i.code == "unsupported-keyword" {
            out.push(i);
            continue;
        }
        let n = count.entry((i.code, i.at.clone())).or_default();
        *n += 1;
        if *n <= keep {
            out.push(i);
        }
    }
    let mut more: Vec<((&'static str, Option<String>), usize)> = count.into_iter().filter(|(_, n)| *n > keep).collect();
    more.sort_by(|a, b| a.0.cmp(&b.0));
    for ((code, at), n) in more {
        out.push(Issue { severity: Severity::Note, line: None, code: "more", at: at.clone(), message: format!("{} more '{code}' finding(s) not listed", n - keep) });
    }
    out
}

/// Typical value helper for summaries.
pub fn typ(t: &Option<Tmm>) -> Option<f64> {
    t.as_ref().and_then(|t| t.typ)
}

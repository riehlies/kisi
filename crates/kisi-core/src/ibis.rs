//! IBIS models in kisi: the summary of `kisi ibis <file.ibs>` and the model mapping of
//! `<board>.kisi.json` (`"ibis": {"U18": {…}}`, see `config::IbisEntry`).
//!
//! Summaries never print the tables of a file (vendor models are usually private): sizes,
//! single values and values kisi derives from the tables, each marked as derived.

use crate::board::Board;
use crate::topology::Bus;
use crate::Analysis;
use crate::config::BoardConfig;
use crate::ibis_parser::{self as ip, derive, Corner, IbisFile, Severity, Tmm};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

/// Model files the configuration's `ibis` section names, by the path written there: the bytes
/// or why they could not be read. A path missing from the map was not loaded (viewer).
pub type IbisFiles = HashMap<String, Result<Vec<u8>, String>>;

/// A parsed model file: data (with the checks' findings), SHA-256 and size.
#[derive(Debug, Clone)]
pub struct Parsed {
    pub file: IbisFile,
    pub sha256: String,
    pub bytes: usize,
}

/// Parsed model files by the path written in kisi.json (CLI) or by file name (viewer): the data
/// or why the file could not be read. A path missing from the map was not loaded.
pub type ParsedSet = HashMap<String, Result<Parsed, String>>;

/// Parse a file's bytes (`name`: as stored, compared with [File Name]).
pub fn parse_one(name: &str, bytes: &[u8]) -> Parsed {
    Parsed { file: ip::read(bytes, Some(base(name))), sha256: ip::sha256::hex(bytes), bytes: bytes.len() }
}

/// Parse the files the front end read.
pub fn parse_set(files: &IbisFiles) -> ParsedSet {
    files.iter().map(|(k, r)| (k.clone(), r.as_ref().map(|b| parse_one(k, b)).map_err(|e| e.clone()))).collect()
}

/// The file a mapping names: by the path written, else by its file name (files loaded in the
/// viewer are known by name only).
pub fn find<'a>(set: &'a ParsedSet, path: &str) -> Option<&'a Result<Parsed, String>> {
    if let Some(r) = set.get(path) {
        return Some(r);
    }
    let b = base(path);
    let mut keys: Vec<&String> = set.keys().filter(|k| base(k) == b).collect();
    keys.sort();
    keys.first().and_then(|k| set.get(*k))
}

pub const SUMMARY_NOTE: &str = "Summary only: tables are not printed, only their sizes. Values under \"derived\" are kisi's estimates from the tables, not values of the file. Units: Ω, nH, pF, V, ps, °C; [Model Spec] and [Receiver Thresholds] in SI units as written. Profiles remain the channel end models in this version.";

/// Round to 6 significant digits (no 1.4900000000000002).
fn sig(x: f64) -> f64 {
    if x == 0.0 || !x.is_finite() {
        return x;
    }
    let d = 10f64.powi(5 - x.abs().log10().floor() as i32);
    let r = (x * d).round() / d;
    if r.is_finite() {
        r
    } else {
        x
    }
}

fn num(x: Option<f64>, k: f64) -> Value {
    x.filter(|v| v.is_finite()).map(|v| json!(sig(v * k))).unwrap_or(Value::Null)
}

/// typ/min/max as written (NA = null), scaled.
fn tmm(t: &Option<Tmm>, k: f64) -> Value {
    match t {
        None => Value::Null,
        Some(t) => json!({"typ": num(t.typ, k), "min": num(t.min, k), "max": num(t.max, k)}),
    }
}

/// A value per corner from a function (null where it cannot be computed).
fn corners(f: impl Fn(Corner) -> Option<f64>, k: f64) -> Value {
    let v: Vec<Option<f64>> = Corner::ALL.iter().map(|&c| f(c)).collect();
    if v.iter().all(Option::is_none) {
        return Value::Null;
    }
    json!({"typ": num(v[0], k), "min": num(v[1], k), "max": num(v[2], k)})
}

fn put(o: &mut Map<String, Value>, k: &str, v: Value) {
    if !v.is_null() {
        o.insert(k.to_string(), v);
    }
}

fn base(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

/// What a pin's model name refers to.
fn pin_kind(f: &IbisFile, p: &ip::Pin) -> &'static str {
    if let Some(r) = p.reserved() {
        r
    } else if f.model(&p.model).is_some() {
        "model"
    } else if f.selector(&p.model).is_some() {
        "selector"
    } else {
        "undefined"
    }
}

fn pin_json(f: &IbisFile, c: &ip::Component, p: &ip::Pin) -> Value {
    let [r, l, cc] = c.pin_rlc(p, Corner::Typ);
    let own = [r.1, l.1, cc.1].iter().filter(|x| **x).count();
    json!({"pin": p.name, "signal": p.signal, "model": p.model, "kind": pin_kind(f, p),
        "r_ohm": num(r.0, 1.0), "l_nh": num(l.0, 1e9), "c_pf": num(cc.0, 1e12),
        "rlc_from": match own { 3 => "pin", 0 => "package", _ => "pin+package" }})
}

fn component_json(f: &IbisFile, c: &ip::Component, pins: bool, only_model: Option<&str>) -> Value {
    let mut by_kind: BTreeMap<&str, usize> = BTreeMap::new();
    for p in &c.pins {
        *by_kind.entry(pin_kind(f, p)).or_default() += 1;
    }
    let mut o = Map::new();
    o.insert("name".into(), json!(c.name));
    o.insert("line".into(), json!(c.line));
    o.insert("manufacturer".into(), json!(c.manufacturer));
    o.insert("package".into(), json!({"r_ohm": tmm(&c.package.r, 1.0), "l_nh": tmm(&c.package.l, 1e9), "c_pf": tmm(&c.package.c, 1e12)}));
    if let Some(pm) = &c.package_model {
        o.insert("package_model".into(), json!({"name": pm, "in_this_file": f.package_model(pm).is_some()}));
    }
    o.insert("pins_total".into(), json!(c.pins.len()));
    o.insert("pins_by_kind".into(), json!(by_kind));
    if pins {
        o.insert("pins".into(), Value::Array(c.pins.iter().map(|p| pin_json(f, c, p)).collect()));
    }
    if let Some(m) = only_model {
        let uses = |p: &ip::Pin| p.model == m || f.selector(&p.model).is_some_and(|s| s.entries.iter().any(|e| e.model == m));
        o.insert("pins_with_model".into(), json!(c.pins.iter().filter(|p| uses(p)).map(|p| p.name.clone()).collect::<Vec<_>>()));
    }
    o.insert(
        "diff_pins".into(),
        Value::Array(c.diff_pins.iter().map(|d| json!({"pin": d.pin, "inv_pin": d.inv_pin, "vdiff_v": num(d.vdiff, 1.0), "tdelay_ps": tmm(&Some(d.tdelay), 1e12), "line": d.line})).collect()),
    );
    if !c.pin_mapping.is_empty() {
        let buses: BTreeSet<&str> =
            c.pin_mapping.iter().flat_map(|m| [Some(m.pulldown_ref.as_str()), Some(m.pullup_ref.as_str()), m.gnd_clamp_ref.as_deref(), m.power_clamp_ref.as_deref(), m.ext_ref.as_deref()]).flatten().filter(|b| !b.eq_ignore_ascii_case("NC")).collect();
        o.insert("pin_mapping".into(), json!({"rows": c.pin_mapping.len(), "buses": buses, "note": "kept as data (power-aware use is out of scope)"}));
    }
    if !c.series_pins.is_empty() {
        o.insert("series_pins".into(), Value::Array(c.series_pins.iter().map(|s| json!({"pin_1": s.pin1, "pin_2": s.pin2, "model": s.model, "group": s.group})).collect()));
    }
    Value::Object(o)
}

fn waveform_json(w: &ip::Waveform) -> Value {
    let mut o = Map::new();
    o.insert("edge".into(), json!(if w.rising { "rising" } else { "falling" }));
    o.insert("line".into(), json!(w.line));
    o.insert("rows".into(), json!(w.table.len()));
    put(&mut o, "r_fixture_ohm", num(w.r_fixture, 1.0));
    if w.v_fixture.is_some() || w.v_fixture_min.is_some() || w.v_fixture_max.is_some() {
        o.insert("v_fixture_v".into(), json!({"typ": num(w.v_fixture, 1.0), "min": num(w.v_fixture_min, 1.0), "max": num(w.v_fixture_max, 1.0)}));
    }
    put(&mut o, "l_fixture_nh", num(w.l_fixture, 1e9));
    put(&mut o, "c_fixture_pf", num(w.c_fixture, 1e12));
    put(&mut o, "r_dut_ohm", num(w.r_dut, 1.0));
    put(&mut o, "l_dut_nh", num(w.l_dut, 1e9));
    put(&mut o, "c_dut_pf", num(w.c_dut, 1e12));
    if let Some(cc) = &w.composite_current {
        o.insert("composite_current_rows".into(), json!(cc.len()));
    }
    Value::Object(o)
}

fn dvdt_json(e: &Option<ip::DvdtTmm>) -> Value {
    let Some(e) = e else { return Value::Null };
    let one = |d: Option<ip::Dvdt>| d.map(|d| json!({"dv_v": sig(d.dv), "dt_ps": sig(d.dt * 1e12)})).unwrap_or(Value::Null);
    json!({"typ": one(e.typ), "min": one(e.min), "max": one(e.max)})
}

fn series_json(s: &ip::SeriesData) -> Value {
    let mut o = Map::new();
    put(&mut o, "r_ohm", tmm(&s.r, 1.0));
    put(&mut o, "l_nh", tmm(&s.l, 1e9));
    put(&mut o, "c_pf", tmm(&s.c, 1e12));
    put(&mut o, "rl_ohm", tmm(&s.rl, 1.0));
    put(&mut o, "rc_ohm", tmm(&s.rc, 1.0));
    put(&mut o, "lc_nh", tmm(&s.lc, 1e9));
    if let Some(t) = &s.current {
        o.insert("series_current_rows".into(), json!(t.len()));
    }
    if !s.mosfet.is_empty() {
        o.insert("series_mosfet".into(), Value::Array(s.mosfet.iter().map(|(vds, t)| json!({"vds_v": num(*vds, 1.0), "rows": t.len()})).collect()));
    }
    Value::Object(o)
}

fn derived_json(m: &ip::Model) -> Value {
    let mut d = Map::new();
    if m.pulldown.is_some() {
        put(&mut d, "ron_pulldown_ohm", corners(|c| derive::ron_pulldown(m, c), 1.0));
    }
    if m.pullup.is_some() {
        put(&mut d, "ron_pullup_ohm", corners(|c| derive::ron_pullup(m, c), 1.0));
    }
    if m.gnd_clamp.is_some() || m.power_clamp.is_some() {
        put(&mut d, "roff_ohm", corners(|c| derive::roff(m, c).flatten(), 1.0));
        if Corner::ALL.iter().any(|&c| derive::roff(m, c) == Some(None)) {
            d.insert("roff_note".into(), json!("the clamps do not conduct at Vcc/2 in some corner (> 1 GΩ): high impedance there"));
        }
    }
    if let Some(r) = &m.ramp {
        for (key, e) in [("rising", &r.rising), ("falling", &r.falling)] {
            let Some(e) = e else { continue };
            let raw = |c: Corner| match c {
                Corner::Typ => e.typ,
                Corner::Min => e.min,
                Corner::Max => e.max,
            };
            put(&mut d, &format!("slew_{key}_v_per_ns"), corners(|c| raw(c).filter(|x| x.dt > 0.0).map(|x| x.dv / x.dt), 1e-9));
            put(&mut d, &format!("{}_20_80_ps", if key == "rising" { "rise" } else { "fall" }), corners(|c| raw(c).map(|x| x.dt), 1e12));
        }
    }
    let waves: Vec<Value> = m
        .waveforms
        .iter()
        .filter_map(|w| {
            let e = derive::edge(w, Corner::Typ)?;
            Some(json!({"edge": if w.rising { "rising" } else { "falling" }, "line": w.line,
                "into": format!("{} Ω to {} V", w.r_fixture.map(sig).map(|x| x.to_string()).unwrap_or("?".into()), w.v_fixture.map(sig).map(|x| x.to_string()).unwrap_or("?".into())),
                "v_start_v": sig(e.v_start), "v_end_v": sig(e.v_end), "t20_80_ps": num(e.t20_80, 1e12)}))
        })
        .collect();
    if !waves.is_empty() {
        d.insert("waveforms_typ".into(), Value::Array(waves));
    }
    if d.is_empty() {
        return Value::Null;
    }
    d.insert(
        "note".into(),
        json!("derived by kisi from the tables – estimates, not values of the file: Ron = (Vcc/2) / I at Vcc/2 of [Pulldown] / [Pullup]; Roff = small-signal resistance of the clamp tables at Vcc/2 (receiver or high-impedance state, finite with on-die termination); slew and 20–80 % times from [Ramp]; waveform start, end and 20–80 % time from the typ column. Corners with NA fall back to typ, as simulators do."),
    );
    Value::Object(d)
}

fn model_json(m: &ip::Model) -> Value {
    let k = m.kind();
    let mut o = Map::new();
    o.insert("name".into(), json!(m.name));
    o.insert("line".into(), json!(m.line));
    o.insert("type".into(), json!(m.model_type));
    put(&mut o, "polarity", json!(m.polarity));
    put(&mut o, "enable", json!(m.enable));
    o.insert("c_comp_pf".into(), corners(|c| m.c_comp_at(c), 1e12));
    let mut parts = Map::new();
    put(&mut parts, "pullup", tmm(&m.c_comp_pullup, 1e12));
    put(&mut parts, "pulldown", tmm(&m.c_comp_pulldown, 1e12));
    put(&mut parts, "power_clamp", tmm(&m.c_comp_power_clamp, 1e12));
    put(&mut parts, "gnd_clamp", tmm(&m.c_comp_gnd_clamp, 1e12));
    if !parts.is_empty() {
        o.insert("c_comp_parts_pf".into(), Value::Object(parts));
    }
    let mut th = Map::new();
    put(&mut th, "vinl_v", num(m.vinl, 1.0));
    put(&mut th, "vinh_v", num(m.vinh, 1.0));
    put(&mut th, "vmeas_v", num(m.vmeas, 1.0));
    put(&mut th, "vref_v", num(m.vref, 1.0));
    put(&mut th, "rref_ohm", num(m.rref, 1.0));
    put(&mut th, "cref_pf", num(m.cref, 1e12));
    if !th.is_empty() {
        o.insert("thresholds".into(), Value::Object(th));
    }
    if !m.params.is_empty() {
        o.insert("other_parameters".into(), Value::Object(m.params.iter().map(|(n, v)| (n.clone(), json!(sig(*v)))).collect()));
    }
    put(&mut o, "voltage_range_v", tmm(&m.voltage_range, 1.0));
    let mut refs = Map::new();
    put(&mut refs, "pullup_v", tmm(&m.pullup_ref, 1.0));
    put(&mut refs, "pulldown_v", tmm(&m.pulldown_ref, 1.0));
    put(&mut refs, "power_clamp_v", tmm(&m.power_clamp_ref, 1.0));
    put(&mut refs, "gnd_clamp_v", tmm(&m.gnd_clamp_ref, 1.0));
    put(&mut refs, "external_v", tmm(&m.ext_ref, 1.0));
    if !refs.is_empty() {
        o.insert("references".into(), Value::Object(refs));
    }
    put(&mut o, "temperature_range_c", tmm(&m.temperature_range, 1.0));
    let rows = |t: &Option<ip::Table>| t.as_ref().map(|t| json!(t.len())).unwrap_or(Value::Null);
    let mut tables = Map::new();
    put(&mut tables, "pulldown", rows(&m.pulldown));
    put(&mut tables, "pullup", rows(&m.pullup));
    put(&mut tables, "gnd_clamp", rows(&m.gnd_clamp));
    put(&mut tables, "power_clamp", rows(&m.power_clamp));
    for (n, t) in &m.extra_tables {
        tables.insert(n.to_ascii_lowercase().replace(' ', "_"), json!(t.len()));
    }
    let all: Vec<&ip::Table> = [&m.pulldown, &m.pullup, &m.gnd_clamp, &m.power_clamp].into_iter().flatten().chain(m.waveforms.iter().map(|w| &w.table)).collect();
    if !all.is_empty() {
        let cs: Vec<&str> = Corner::ALL.iter().filter(|&&c| all.iter().any(|t| t.has_corner(c))).map(|c| c.name()).collect();
        tables.insert("corners_with_data".into(), json!(cs));
    }
    if !tables.is_empty() {
        o.insert("tables_rows".into(), Value::Object(tables));
    }
    let mut term = Map::new();
    put(&mut term, "rgnd_ohm", tmm(&m.rgnd, 1.0));
    put(&mut term, "rpower_ohm", tmm(&m.rpower, 1.0));
    put(&mut term, "rac_ohm", tmm(&m.rac, 1.0));
    put(&mut term, "cac_pf", tmm(&m.cac, 1e12));
    if !term.is_empty() {
        o.insert("terminator".into(), Value::Object(term));
    }
    if k.series || k.series_switch || !m.series.is_empty() {
        let mut s = Map::new();
        if !m.series.is_empty() {
            s.insert("elements".into(), series_json(&m.series));
        }
        if let Some(on) = &m.on {
            s.insert("on".into(), series_json(on));
        }
        if let Some(off) = &m.off {
            s.insert("off".into(), series_json(off));
        }
        o.insert("series".into(), Value::Object(s));
    }
    if let Some(r) = &m.ramp {
        o.insert("ramp".into(), json!({"line": r.line, "r_load_ohm": sig(r.r_load()), "r_load_given": r.r_load.is_some(), "rising": dvdt_json(&r.rising), "falling": dvdt_json(&r.falling)}));
    }
    if !m.waveforms.is_empty() {
        o.insert("waveforms".into(), Value::Array(m.waveforms.iter().map(waveform_json).collect()));
    }
    if !m.model_spec.is_empty() {
        o.insert("model_spec".into(), Value::Object(m.model_spec.iter().map(|s| (s.name.clone(), tmm(&Some(s.value), 1.0))).collect()));
    }
    if !m.receiver_thresholds.is_empty() {
        o.insert("receiver_thresholds".into(), Value::Object(m.receiver_thresholds.iter().map(|r| (r.name.clone(), r.value.map(|v| json!(sig(v))).unwrap_or(json!(r.text)))).collect()));
    }
    if !m.driver_schedule.is_empty() {
        o.insert(
            "driver_schedule".into(),
            Value::Array(m.driver_schedule.iter().map(|d| json!({"model": d.model, "rise_on_ps": num(d.rise_on, 1e12), "rise_off_ps": num(d.rise_off, 1e12), "fall_on_ps": num(d.fall_on, 1e12), "fall_off_ps": num(d.fall_off, 1e12)})).collect()),
        );
    }
    if !m.add_submodel.is_empty() {
        o.insert("submodels".into(), Value::Array(m.add_submodel.iter().map(|s| json!({"name": s.name, "mode": s.mode})).collect()));
    }
    if !m.extra_values.is_empty() {
        o.insert("other_values".into(), Value::Object(m.extra_values.iter().map(|v| (v.name.clone(), tmm(&Some(v.value), 1.0))).collect()));
    }
    put(&mut o, "derived", derived_json(m));
    Value::Object(o)
}

/// The JSON of `kisi ibis <file>`: header, components (pins with models and package R/L/C),
/// model selectors, models (with derived values), submodels, package models, findings.
/// `component` / `model`: show only that component (and the models its pins use) / that model.
pub fn summary(f: &IbisFile, name: &str, bytes: &[u8], component: Option<&str>, model: Option<&str>) -> Result<Value, String> {
    let comp = match component {
        Some(c) => Some(f.component(c).ok_or_else(|| format!("no [Component] '{c}' in {name} (components: {})", f.components.iter().map(|c| c.name.as_str()).collect::<Vec<_>>().join(", ")))?),
        None => None,
    };
    let mdl = match model {
        Some(m) => Some(f.model(m).ok_or_else(|| format!("no [Model] '{m}' in {name} ({} models{})", f.models.len(), if f.models.len() <= 30 { format!(": {}", f.models.iter().map(|m| m.name.as_str()).collect::<Vec<_>>().join(", ")) } else { String::new() }))?),
        None => None,
    };
    // which models and selectors to show
    let mut sel_names: Option<HashSet<&str>> = None;
    let mut model_names: Option<HashSet<&str>> = None;
    if let Some(c) = comp {
        let s: HashSet<&str> = c.pins.iter().filter_map(|p| f.selector(&p.model)).map(|s| s.name.as_str()).collect();
        let mut m: HashSet<&str> = c.pins.iter().filter_map(|p| f.model(&p.model)).chain(c.series_pins.iter().filter_map(|s| f.model(&s.model))).map(|m| m.name.as_str()).collect();
        for s in s.iter().filter_map(|n| f.selector(n)) {
            m.extend(s.entries.iter().map(|e| e.model.as_str()));
        }
        sel_names = Some(s);
        model_names = Some(m);
    }
    if let Some(m) = mdl {
        model_names = Some(HashSet::from([m.name.as_str()]));
        let s: HashSet<&str> = f.model_selectors.iter().filter(|s| s.entries.iter().any(|e| e.model == m.name)).map(|s| s.name.as_str()).collect();
        sel_names = Some(match sel_names {
            Some(old) => old.intersection(&s).copied().collect(),
            None => s,
        });
    }
    let components: Vec<Value> = f.components.iter().filter(|c| comp.map_or(true, |x| std::ptr::eq(x, *c))).map(|c| component_json(f, c, mdl.is_none() || comp.is_some(), mdl.map(|m| m.name.as_str()))).collect();
    let selectors: Vec<Value> = f
        .model_selectors
        .iter()
        .filter(|s| sel_names.as_ref().map_or(true, |n| n.contains(s.name.as_str())))
        .map(|s| json!({"name": s.name, "line": s.line, "default": s.entries.first().map(|e| e.model.clone()), "models": s.entries.iter().map(|e| json!({"model": e.model, "description": e.description, "defined": f.model(&e.model).is_some()})).collect::<Vec<_>>()}))
        .collect();
    let models: Vec<Value> = f.models.iter().filter(|m| model_names.as_ref().map_or(true, |n| n.contains(m.name.as_str()))).map(model_json).collect();
    let h = &f.header;
    let mat = |m: &Option<ip::Matrix>| m.as_ref().map(|m| json!({"kind": m.kind, "entries": m.entries.len(), "bandwidth": m.bandwidth})).unwrap_or(Value::Null);
    let filtered = comp.is_some() || mdl.is_some();
    // with a filter: only the findings about what is shown (and the file as a whole)
    let shown: HashSet<String> = components
        .iter()
        .filter_map(|c| c["name"].as_str().map(|n| format!("component {n}")))
        .chain(selectors.iter().filter_map(|s| s["name"].as_str().map(|n| format!("model selector {n}"))))
        .chain(models.iter().filter_map(|m| m["name"].as_str().map(|n| format!("model {n}"))))
        .collect();
    let issues: Vec<&ip::Issue> = f.issues.iter().filter(|i| !filtered || i.at.as_deref().map_or(true, |a| a == "header" || shown.contains(a))).collect();
    Ok(json!({
        "file": name,
        "sha256": ip::sha256::hex(bytes),
        "bytes": bytes.len(),
        "lines": f.lines,
        "header": {"ibis_ver": h.ibis_ver, "file_name": h.file_name, "file_rev": h.file_rev, "date": h.date, "source": h.source, "notes": h.notes, "disclaimer": h.disclaimer, "copyright": h.copyright},
        "counts": {"components": f.components.len(), "model_selectors": f.model_selectors.len(), "models": f.models.len(), "submodels": f.submodels.len(), "package_models": f.package_models.len(),
                   "errors": f.count(Severity::Error), "warnings": f.count(Severity::Warning), "notes": f.count(Severity::Note), "complete": f.ended},
        "filter": if filtered { json!({"component": comp.map(|c| c.name.clone()), "model": mdl.map(|m| m.name.clone())}) } else { Value::Null },
        "components": components,
        "model_selectors": selectors,
        "models": models,
        "submodels": if filtered { Value::Null } else { Value::Array(f.submodels.iter().map(|s| json!({"name": s.name, "line": s.line, "type": s.submodel_type, "tables": s.tables.iter().map(|t| json!({"keyword": t.0, "rows": t.1.len()})).collect::<Vec<_>>(), "waveforms": s.waveforms.len(), "note": "parsed, not used by kisi"})).collect()) },
        "package_models": if filtered { Value::Null } else { Value::Array(f.package_models.iter().map(|p| json!({"name": p.name, "line": p.line, "manufacturer": p.manufacturer, "oem": p.oem, "description": p.description, "number_of_pins": p.number_of_pins, "pins_listed": p.pins.len(), "sections": p.sections,
            "matrices": {"resistance": mat(&p.r), "inductance": mat(&p.l), "capacitance": mat(&p.c)}, "note": "stored; channel models use the uncoupled [Pin] / [Package] R/L/C so far"})).collect()) },
        "issues": issues,
        "issues_filtered": filtered && issues.len() < f.issues.len(),
        "note": SUMMARY_NOTE,
    }))
}

/// Nets whose pads need an IBIS pin with a model: nets with a target impedance and the nets of
/// device-to-device lanes, without supply and ground nets (value: why).
pub fn signal_nets(board: &Board, an: &Analysis, buses: &[Bus]) -> HashMap<u32, String> {
    let supply: HashSet<u32> = crate::pi::power_nets(board).into_iter().map(|n| n.net).collect();
    let mut m = HashMap::new();
    for st in an.nets.iter().filter(|s| s.target.is_some() && !supply.contains(&s.net)) {
        m.insert(st.net, "target impedance".to_string());
    }
    for b in buses {
        for l in &b.lanes {
            for &n in l.nets.iter().chain(&l.partner_nets).filter(|n| !supply.contains(n)) {
                m.entry(n).or_insert_with(|| format!("lane {}", l.name));
            }
        }
    }
    m
}

/// Result of the mapping check with the model files.
#[derive(Debug, Clone, Default)]
pub struct FileCheck {
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    /// per mapped part: file, component, hash state, pads checked / covered, uncovered pads,
    /// selectors, models used
    pub parts: Vec<Value>,
}

/// Check the `ibis` section with the model files: file present, hash, component, selectors,
/// and that every pad of the part on a signal net (`signal_nets`) has an IBIS pin with a model.
pub fn check_files(cfg: &BoardConfig, board: &Board, signal: &HashMap<u32, String>, files: &ParsedSet) -> FileCheck {
    let mut out = FileCheck::default();
    for (refdes, e) in cfg.ibis.iter().filter(|(_, e)| !e.file.trim().is_empty()) {
        let what = format!("ibis.{refdes}");
        let Some(fi) = board.footprints.iter().position(|f| &f.reference == refdes) else { continue };
        let fp = &board.footprints[fi];
        let mut part = json!({"ref": refdes, "value": fp.value, "file": e.file, "component": e.component, "corner": e.corner.as_deref().unwrap_or("typ"), "pin_map": e.pin_map.as_deref().unwrap_or("by-number"), "loaded": false});
        if fp.dnp {
            out.warnings.push(format!("{what}: {refdes} is not fitted (DNP or not in the chosen assembly variant): the mapping is not used"));
            part["fitted"] = json!(false);
        }
        let parsed = match find(files, &e.file) {
            None => {
                out.warnings.push(format!("{what}: {} not loaded – in the viewer, drop the .ibs file together with the board (it stays in memory, nothing is stored or uploaded)", e.file));
                out.parts.push(part);
                continue;
            }
            Some(Err(err)) => {
                out.errors.push(format!("{what}: {err}"));
                part["error"] = json!(err);
                out.parts.push(part);
                continue;
            }
            Some(Ok(p)) => p,
        };
        let (f, sha) = (&parsed.file, &parsed.sha256);
        part["loaded"] = json!(true);
        part["sha256"] = json!(sha);
        match &e.sha256 {
            Some(h) if !h.eq_ignore_ascii_case(sha) => {
                out.errors.push(format!("{what}: {} has sha256 {sha}, the kisi.json says {h}: the file changed – check the mapping, then update sha256", e.file));
                part["sha256_ok"] = json!(false);
            }
            Some(_) => part["sha256_ok"] = json!(true),
            None => out.warnings.push(format!("{what}: no sha256 – add \"sha256\": \"{sha}\" so that kisi notices when {} changes", e.file)),
        }
        let (ne, nw) = (f.count(Severity::Error), f.count(Severity::Warning));
        part["file_issues"] = json!({"errors": ne, "warnings": nw});
        if ne > 0 {
            out.warnings.push(format!("{what}: {} has {ne} error(s) and {nw} warning(s) of its own (see `kisi ibis {}`)", e.file, e.file));
        }
        let names = || f.components.iter().map(|c| c.name.as_str()).collect::<Vec<_>>().join(", ");
        let comp = match &e.component {
            Some(n) => match f.component(n) {
                Some(c) => c,
                None => {
                    out.errors.push(format!("{what}: no [Component] '{n}' in {} (components: {})", e.file, names()));
                    out.parts.push(part);
                    continue;
                }
            },
            None if f.components.len() == 1 => &f.components[0],
            None => {
                out.errors.push(format!("{what}: {} has {} components – name one with \"component\" ({})", e.file, f.components.len(), names()));
                out.parts.push(part);
                continue;
            }
        };
        part["component"] = json!(comp.name);
        for (sel, model) in &e.selectors {
            match f.selector(sel) {
                None => out.errors.push(format!("{what}: selectors: no [Model Selector] '{sel}' in {} (selectors: {})", e.file, f.model_selectors.iter().map(|s| s.name.as_str()).collect::<Vec<_>>().join(", "))),
                Some(s) if !s.entries.iter().any(|x| x.model.eq_ignore_ascii_case(model)) => {
                    out.errors.push(format!("{what}: selectors.{sel}: '{model}' is not one of its models ({})", s.entries.iter().map(|x| x.model.as_str()).collect::<Vec<_>>().join(", ")))
                }
                _ => {}
            }
        }
        for (pad, pin) in &e.pins {
            if comp.pin(pin).is_none() {
                out.errors.push(format!("{what}: pins.{pad}: no pin '{pin}' in component {}", comp.name));
            }
        }
        for (pad, m) in &e.models {
            if f.model(m).is_none() {
                out.errors.push(format!("{what}: models.{pad}: no [Model] '{m}' in {}", e.file));
            }
        }
        // pads of the footprint (board order), with their nets and pin names
        let mut pads: Vec<(&str, Vec<u32>, &str)> = Vec::new();
        for p in board.pads.iter().filter(|p| p.fp as usize == fi) {
            match pads.iter_mut().find(|x| x.0 == p.number) {
                Some(x) => x.1.push(p.net),
                None => pads.push((p.number.as_str(), vec![p.net], p.pinfunction.as_str())),
            }
        }
        let by_name = e.pin_map.as_deref() == Some("by-name");
        let choice = |s: &str| e.selectors.iter().find(|(k, _)| k.eq_ignore_ascii_case(s)).map(|(_, v)| v.clone());
        let (mut checked, mut covered) = (0usize, 0usize);
        let mut uncovered: Vec<Value> = Vec::new();
        let mut unchosen: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut models: BTreeSet<String> = BTreeSet::new();
        for (number, nets, pinfn) in &pads {
            let Some((net, why)) = nets.iter().find_map(|n| signal.get(n).map(|w| (*n, w))) else { continue };
            checked += 1;
            let ibis_pin = if let Some(p) = e.pins.get(*number) {
                comp.pin(p)
            } else if by_name {
                comp.pins.iter().find(|p| p.signal == *pinfn).or_else(|| comp.pins.iter().find(|p| !pinfn.is_empty() && p.signal.eq_ignore_ascii_case(pinfn)))
            } else {
                comp.pin(number)
            };
            if let Some(m) = e.models.get(*number).and_then(|m| f.model(m)) {
                models.insert(m.name.clone());
                covered += 1;
                continue;
            }
            let reason = match ibis_pin {
                None if by_name && pinfn.is_empty() => Some("the pad has no pin name (pinfunction) in the board file".to_string()),
                None if by_name => Some(format!("no IBIS pin with signal name '{pinfn}'")),
                None => Some("no such pin in the IBIS file".to_string()),
                Some(p) => {
                    if let Some(r) = p.reserved() {
                        Some(format!("IBIS pin {} is {r}", p.name))
                    } else if let Some(m) = f.model(&p.model) {
                        models.insert(m.name.clone());
                        None
                    } else if let Some(s) = f.selector(&p.model) {
                        match choice(&s.name) {
                            Some(c) => {
                                models.insert(c);
                            }
                            None => {
                                unchosen.entry(s.name.clone()).or_default().push(number.to_string());
                                if let Some(first) = s.entries.first() {
                                    models.insert(format!("{} (first entry of {}, not chosen)", first.model, s.name));
                                }
                            }
                        }
                        None
                    } else {
                        Some(format!("model '{}' of IBIS pin {} is not defined in the file", p.model, p.name))
                    }
                }
            };
            match reason {
                None => covered += 1,
                Some(r) => uncovered.push(json!({"pad": number, "net": board.net_name(net), "needed_for": why, "reason": r})),
            }
        }
        if !uncovered.is_empty() {
            let list: Vec<String> = uncovered.iter().take(10).map(|u| format!("{} ({}: {})", u["pad"].as_str().unwrap_or(""), u["net"].as_str().unwrap_or(""), u["reason"].as_str().unwrap_or(""))).collect();
            let hint = if !by_name && covered == 0 && pads.iter().any(|p| !p.2.is_empty()) { " – none matches by number: try \"pin_map\": \"by-name\"" } else { "" };
            out.warnings.push(format!(
                "{what}: {} of {checked} pad(s) on nets with a target or a lane have no IBIS pin with a model: {}{}{hint}",
                uncovered.len(),
                list.join(", "),
                if uncovered.len() > 10 { ", …" } else { "" }
            ));
        }
        for (sel, pads) in &unchosen {
            let s = f.selector(sel);
            out.warnings.push(format!(
                "{what}: pad(s) {} use [Model Selector] {sel} without a choice – add \"selectors\": {{\"{sel}\": \"<model>\"}} ({}); otherwise the first entry, {}, applies",
                pads.join(", "),
                s.map(|s| s.entries.iter().map(|e| e.model.as_str()).collect::<Vec<_>>().join(", ")).unwrap_or_default(),
                s.and_then(|s| s.entries.first()).map(|e| e.model.as_str()).unwrap_or("?")
            ));
        }
        part["pads_checked"] = json!(checked);
        part["pads_with_model"] = json!(covered);
        part["uncovered"] = json!(uncovered);
        part["selectors"] = json!(e.selectors);
        part["selectors_without_choice"] = json!(unchosen.keys().collect::<Vec<_>>());
        part["models"] = json!(models);
        out.parts.push(part);
    }
    out
}

// --- linearised IBIS ends for the channel check and the eye ------------------------------

/// 10–90 % over 20–80 % time of a Gaussian edge – the edge shape of kisi's eye
/// (`eye::step_response`): 2·1.2816 / (2·0.8416).
pub const RISE_10_90_PER_20_80: f64 = 1.5228;
/// Clamp-table resistance at Vcc/2 up to which a receiver counts as terminated (on-die
/// termination); above it the input is taken as high impedance.
pub const ODT_MAX_OHM: f64 = 2000.0;
pub const LINEAR_NOTE: &str = "linearised IBIS model (estimate)";

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Driver,
    Receiver,
}

/// The IBIS model at one end of a lane, linearised per line for kisi's linear channel: a
/// driver as source resistance (Ron + R_pin), capacitance (C_comp + C_pin) and package L with
/// an edge from the waveforms or [Ramp]; a receiver as C_comp + C_pin, package L and its
/// termination (terminator or on-die termination in the clamp tables), else high impedance.
#[derive(Debug, Clone, serde::Serialize)]
pub struct IbisEnd {
    pub refdes: String,
    pub pad: String,
    pub role: Role,
    pub file: String,
    pub sha256: Option<String>,
    pub component: Option<String>,
    pub ibis_pin: Option<String>,
    pub signal: Option<String>,
    pub model: Option<String>,
    pub model_type: Option<String>,
    /// "pin", "override", "[Model Selector] X (chosen)", "[Model Selector] X: first entry (not chosen)"
    pub model_from: Option<String>,
    /// the [Model Selector] the IBIS pin uses (and its models)
    pub selector: Option<String>,
    pub selector_models: Vec<String>,
    pub corner: String,
    /// per line: driver source resistance, receiver termination (0 = high impedance)
    pub r_ohm: f64,
    pub c_pf: f64,
    pub l_nh: f64,
    /// driver: 10–90 % rise for the eye, and where it comes from
    pub rise_ps: Option<f64>,
    pub rise_from: Option<String>,
    /// supply span of the model (Vcc)
    pub vcc_v: Option<f64>,
    pub termination: Option<String>,
    /// the parts of the linear model: Ron, R/C/L of the pin, C_comp, 20–80 % time
    pub details: Value,
    /// the [Diff Pin] partner of a differential lane end
    pub diff_pin: Option<String>,
    /// models of the component that fit the role (for an override)
    pub candidates: Vec<String>,
    pub warnings: Vec<String>,
    /// set when the mapping cannot be used: the profile's end applies
    pub error: Option<String>,
    pub note: &'static str,
}

impl IbisEnd {
    fn new(refdes: &str, pad: &str, role: Role, file: &str) -> IbisEnd {
        IbisEnd {
            refdes: refdes.into(),
            pad: pad.into(),
            role,
            file: file.into(),
            sha256: None,
            component: None,
            ibis_pin: None,
            signal: None,
            model: None,
            model_type: None,
            model_from: None,
            selector: None,
            selector_models: vec![],
            corner: "typ".into(),
            r_ohm: 0.0,
            c_pf: 0.0,
            l_nh: 0.0,
            rise_ps: None,
            rise_from: None,
            vcc_v: None,
            termination: None,
            details: Value::Null,
            diff_pin: None,
            candidates: vec![],
            warnings: vec![],
            error: None,
            note: LINEAR_NOTE,
        }
    }
    /// The linear end model (None when the mapping cannot be used).
    pub fn end_model(&self) -> Option<crate::eye::EndModel> {
        self.error.is_none().then_some(crate::eye::EndModel { r: self.r_ohm, c_pf: self.c_pf, l_nh: self.l_nh })
    }
    fn fail(mut self, e: impl Into<String>) -> IbisEnd {
        self.error = Some(e.into());
        self
    }
}

fn fits(k: ip::ModelKind, role: Role) -> bool {
    match role {
        Role::Driver => k.output,
        Role::Receiver => k.input || k.terminator,
    }
}

/// Models a component's pins use, directly or through a [Model Selector].
fn component_models<'a>(f: &'a IbisFile, c: &ip::Component) -> BTreeSet<&'a str> {
    let mut out = BTreeSet::new();
    for p in &c.pins {
        if let Some(m) = f.model(&p.model) {
            out.insert(m.name.as_str());
        } else if let Some(s) = f.selector(&p.model) {
            out.extend(s.entries.iter().filter_map(|e| f.model(&e.model)).map(|m| m.name.as_str()));
        }
    }
    out
}

/// The IBIS end of pad `pad` of part `refdes` in the role it has on a lane, from a mapping
/// (`ibis` of kisi.json, or the viewer's choice). None: no mapping for the part; an end with
/// `error` set: the mapping exists but cannot be used here (the profile's end applies).
#[allow(clippy::too_many_arguments)]
pub fn resolve_end(map: &BTreeMap<String, crate::config::IbisEntry>, set: &ParsedSet, board: &Board, refdes: &str, pad: &str, role: Role, diff: bool) -> Option<IbisEnd> {
    let e = map.get(refdes).filter(|e| !e.file.trim().is_empty())?;
    let mut out = IbisEnd::new(refdes, pad, role, base(&e.file));
    let corner = e.corner.as_deref().and_then(Corner::parse).unwrap_or(Corner::Typ);
    out.corner = corner.name().into();
    let parsed = match find(set, &e.file) {
        None => return Some(out.fail(format!("{} not loaded", base(&e.file)))),
        Some(Err(err)) => return Some(out.fail(err.clone())),
        Some(Ok(p)) => p,
    };
    out.sha256 = Some(parsed.sha256.clone());
    let f = &parsed.file;
    let comp = match &e.component {
        Some(n) => match f.component(n) {
            Some(c) => c,
            None => return Some(out.fail(format!("no [Component] '{n}' in {}", base(&e.file)))),
        },
        None if f.components.len() == 1 => &f.components[0],
        None => return Some(out.fail(format!("{} has {} components: choose one", base(&e.file), f.components.len()))),
    };
    out.component = Some(comp.name.clone());
    out.candidates = component_models(f, comp).into_iter().filter(|m| f.model(m).is_some_and(|m| fits(m.kind(), role))).map(str::to_string).collect();
    // the IBIS pin of the pad: an exception in `pins`, by pin name, or by number
    let pinfn = board.footprints.iter().position(|x| x.reference == refdes).and_then(|fi| board.pads.iter().find(|p| p.fp as usize == fi && p.number == pad)).map(|p| p.pinfunction.as_str()).unwrap_or("");
    let by_name = e.pin_map.as_deref() == Some("by-name");
    let pin = if let Some(pn) = e.pins.get(pad) {
        comp.pin(pn)
    } else if by_name {
        comp.pins.iter().find(|p| !pinfn.is_empty() && p.signal == pinfn).or_else(|| comp.pins.iter().find(|p| !pinfn.is_empty() && p.signal.eq_ignore_ascii_case(pinfn)))
    } else {
        comp.pin(pad)
    };
    let Some(pin) = pin else {
        let why = if by_name { format!("no IBIS pin with signal name '{pinfn}' (pad {pad})") } else { format!("pad {pad} is not a pin of {}", comp.name) };
        return Some(out.fail(why));
    };
    out.ibis_pin = Some(pin.name.clone());
    out.signal = Some(pin.signal.clone());
    if let Some(s) = f.selector(&pin.model) {
        out.selector = Some(s.name.clone());
        out.selector_models = s.entries.iter().map(|x| x.model.clone()).collect();
    }
    // the model: a per-pad override, the pin's model, or its selector's choice / first entry
    let choice = |s: &str| e.selectors.iter().find(|(k, _)| k.eq_ignore_ascii_case(s)).map(|(_, v)| v.clone());
    let (mname, from) = if let Some(m) = e.models.get(pad) {
        (m.clone(), "override".to_string())
    } else if let Some(r) = pin.reserved() {
        return Some(out.fail(format!("IBIS pin {} is {r}", pin.name)));
    } else if f.model(&pin.model).is_some() {
        (pin.model.clone(), "pin".to_string())
    } else if let Some(s) = f.selector(&pin.model) {
        match (choice(&s.name), s.entries.first()) {
            (Some(c), _) => (c, format!("[Model Selector] {} (chosen)", s.name)),
            (None, Some(first)) => (first.model.clone(), format!("[Model Selector] {}: first entry (not chosen)", s.name)),
            (None, None) => return Some(out.fail(format!("[Model Selector] {} lists no model", s.name))),
        }
    } else {
        return Some(out.fail(format!("model '{}' of IBIS pin {} is not defined in the file", pin.model, pin.name)));
    };
    let Some(m) = f.model(&mname) else { return Some(out.fail(format!("model '{mname}' is not defined in {}", base(&e.file)))) };
    out.model = Some(m.name.clone());
    out.model_type = m.model_type.clone();
    out.model_from = Some(from);
    let k = m.kind();
    if !fits(k, role) {
        let what = if role == Role::Driver { "drive" } else { "receive" };
        return Some(out.fail(format!("model {} ({}) cannot {what}", m.name, m.model_type.as_deref().unwrap_or("no Model_type"))));
    }
    let [rp, lp, cp] = comp.pin_rlc(pin, corner);
    let (r_pin, l_pin, c_pin) = (rp.0.unwrap_or(0.0), lp.0.unwrap_or(0.0), cp.0.unwrap_or(0.0));
    let c_comp = m.c_comp_at(corner).unwrap_or(0.0);
    out.c_pf = (c_comp + c_pin) * 1e12;
    out.l_nh = l_pin * 1e9;
    out.vcc_v = derive::rails(m, corner).map(|r| r.span());
    let mut det = json!({"c_comp_pf": sig(c_comp * 1e12), "c_pin_pf": sig(c_pin * 1e12), "l_pin_nh": sig(l_pin * 1e9), "r_pin_ohm": sig(r_pin), "pin_rlc_from": if rp.1 || lp.1 || cp.1 { "pin" } else { "package" }});
    match role {
        Role::Driver => {
            let (pu, pd) = (derive::ron_pullup(m, corner), derive::ron_pulldown(m, corner));
            let ron = match (pu, pd) {
                (Some(a), Some(b)) => 0.5 * (a + b),
                (Some(a), None) | (None, Some(a)) => {
                    out.warnings.push(format!("{}: one driver transistor only – linearised with it; the pull resistor is outside the model", m.model_type.as_deref().unwrap_or("")));
                    a
                }
                (None, None) => return Some(out.fail(format!("model {}: no [Pullup] / [Pulldown] resistance at Vcc/2", m.name))),
            };
            out.r_ohm = ron + r_pin;
            det["ron_pullup_ohm"] = num(pu, 1.0);
            det["ron_pulldown_ohm"] = num(pd, 1.0);
            // edge: mean 20–80 % time of the waveforms of the corner, else [Ramp]
            let wave: Vec<f64> = m.waveforms.iter().filter(|w| w.table.has_corner(corner)).filter_map(|w| derive::edge(w, corner).and_then(|x| x.t20_80)).filter(|t| *t > 0.0).collect();
            let ramp: Vec<f64> = m.ramp.iter().flat_map(|r| [r.rising, r.falling]).flatten().filter_map(|d| d.get(corner)).map(|d| d.dt).filter(|t| *t > 0.0).collect();
            let (t, from) = if !wave.is_empty() {
                (Some(wave.iter().sum::<f64>() / wave.len() as f64), format!("{} waveform(s), mean 20–80 %", wave.len()))
            } else if !ramp.is_empty() {
                (Some(ramp.iter().sum::<f64>() / ramp.len() as f64), "[Ramp] dt, mean of rising and falling (20–80 %)".to_string())
            } else {
                out.warnings.push("no waveforms and no [Ramp]: the profile's edge is used".into());
                (None, String::new())
            };
            if let Some(t) = t {
                det["t20_80_ps"] = json!(sig(t * 1e12));
                out.rise_ps = Some(t * RISE_10_90_PER_20_80 * 1e12);
                out.rise_from = Some(format!("{from} × {RISE_10_90_PER_20_80} (10–90 % of a Gaussian edge)"));
            }
        }
        Role::Receiver => {
            if k.terminator {
                let rs: Vec<f64> = [&m.rgnd, &m.rpower, &m.rac].iter().filter_map(|t| t.as_ref().and_then(|t| t.get(corner))).filter(|r| *r > 0.0).collect();
                if !rs.is_empty() {
                    out.r_ohm = 1.0 / rs.iter().map(|r| 1.0 / r).sum::<f64>();
                    out.termination = Some(format!("terminator: [Rgnd] / [Rpower] / [Rac] in parallel to AC ground{}", if m.cac.is_some() { " ([Cac] taken as a short at the signal frequencies)" } else { "" }));
                }
            } else if let Some(Some(r)) = derive::roff(m, corner) {
                if r <= ODT_MAX_OHM {
                    out.r_ohm = r;
                    out.termination = Some(format!("on-die termination from the clamp tables: {} Ω small-signal at Vcc/2", sig(r)));
                }
            }
            if out.termination.is_none() {
                out.termination = Some("none (high impedance)".into());
            }
            if r_pin > 0.0 {
                out.warnings.push(format!("R_pin {} Ω of the receiver is left out (series R is not part of the receiver model)", sig(r_pin)));
            }
        }
    }
    out.details = det;
    out.r_ohm = sig(out.r_ohm);
    out.c_pf = sig(out.c_pf);
    out.l_nh = sig(out.l_nh);
    out.rise_ps = out.rise_ps.map(sig);
    if diff {
        match comp.diff_pins.iter().find(|d| d.pin == pin.name || d.inv_pin == pin.name) {
            Some(d) => out.diff_pin = Some(format!("{} / {}{}", d.pin, d.inv_pin, d.vdiff.map(|v| format!(", vdiff {} V", sig(v))).unwrap_or_default())),
            None => out.warnings.push(format!("IBIS pin {} is not in [Diff Pin]: each line is modelled as a single-ended buffer", pin.name)),
        }
    }
    Some(out)
}

/// The IBIS ends of a lane: [transmitting end, receiving end] (IC ends with a mapping only;
/// connectors keep their connector-end model). `reversed`: transmit from `ends[1]`.
pub fn lane_ends(map: &BTreeMap<String, crate::config::IbisEntry>, set: &ParsedSet, board: &Board, lane: &crate::topology::Lane, reversed: bool) -> [Option<IbisEnd>; 2] {
    if lane.ends.len() != 2 || map.is_empty() {
        return [None, None];
    }
    let (ti, ri) = if reversed { (1, 0) } else { (0, 1) };
    let one = |i: usize, role: Role| {
        let e = &lane.ends[i];
        (e.role == "ic").then(|| resolve_end(map, set, board, &e.refdes, &e.pin, role, lane.diff)).flatten()
    };
    [one(ti, Role::Driver), one(ri, Role::Receiver)]
}

/// The ends a lane check uses: the IBIS ends where they are usable, else the profile's.
#[derive(Debug, Clone)]
pub struct AppliedEnds {
    pub tx: crate::eye::EndModel,
    pub rx: crate::eye::EndModel,
    /// 10–90 % rise of the transmitter (s)
    pub rise_s: f64,
    /// supply span of an IBIS driver (for the eye in mV)
    pub tx_vcc_v: Option<f64>,
    /// what each end is: the IBIS end (JSON) or {"model": "profile"} (with the reason if a
    /// mapping could not be used)
    pub tx_info: Value,
    pub rx_info: Value,
    /// an IBIS mapping concerns at least one end
    pub mapped: bool,
    pub ibis_used: bool,
}

pub fn apply_ends(ends: &[Option<IbisEnd>; 2], tx: crate::eye::EndModel, rx: crate::eye::EndModel, rise_s: f64) -> AppliedEnds {
    let info = |e: &Option<IbisEnd>| match e {
        None => json!({"model": "profile"}),
        Some(x) if x.error.is_some() => json!({"model": "profile", "ibis": x}),
        Some(x) => serde_json::to_value(x).unwrap_or(Value::Null),
    };
    let t = ends[0].as_ref().and_then(|e| e.end_model());
    let r = ends[1].as_ref().and_then(|e| e.end_model());
    let rise = ends[0].as_ref().filter(|e| e.error.is_none()).and_then(|e| e.rise_ps).map(|p| p * 1e-12);
    AppliedEnds {
        tx: t.unwrap_or(tx),
        rx: r.unwrap_or(rx),
        rise_s: rise.unwrap_or(rise_s),
        tx_vcc_v: ends[0].as_ref().filter(|e| e.error.is_none()).and_then(|e| e.vcc_v),
        tx_info: info(&ends[0]),
        rx_info: info(&ends[1]),
        mapped: ends.iter().any(|e| e.is_some()),
        ibis_used: t.is_some() || r.is_some(),
    }
}

/// Swing of an IBIS driver at the receiver for the eye's 100 % (mV): the open-circuit swing
/// (Vcc) divided by source and receiver termination, doubled for a pair (odd mode, per line).
pub fn eye_swing_mv(vcc_v: f64, tx: &crate::eye::EndModel, rx: &crate::eye::EndModel, diff: bool) -> f64 {
    let div = if rx.r > 0.0 { rx.r / (tx.r.max(0.0) + rx.r) } else { 1.0 };
    sig(vcc_v * 1e3 * div * if diff { 2.0 } else { 1.0 })
}

/// What the viewer shows of a loaded model file: components (pins, selectors they use),
/// selectors, models with their types, findings.
pub fn file_info(name: &str, p: &Parsed) -> Value {
    let f = &p.file;
    json!({
        "name": name, "sha256": p.sha256, "bytes": p.bytes,
        "components": f.components.iter().map(|c| {
            let sels: BTreeSet<&str> = c.pins.iter().filter_map(|x| f.selector(&x.model)).map(|s| s.name.as_str()).collect();
            json!({"name": c.name, "pins": c.pins.len(), "selectors": sels})
        }).collect::<Vec<_>>(),
        "selectors": f.model_selectors.iter().map(|s| json!({"name": s.name, "models": s.entries.iter().map(|e| e.model.clone()).collect::<Vec<_>>()})).collect::<Vec<_>>(),
        "models": f.models.iter().map(|m| json!({"name": m.name, "type": m.model_type})).collect::<Vec<_>>(),
        "errors": f.count(Severity::Error), "warnings": f.count(Severity::Warning),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::{Inputs, Project};

    fn synth() -> Vec<u8> {
        std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../kisi-ibis/tests/data/synthetic_full.ibs")).unwrap()
    }

    fn args(s: &str) -> crate::commands::Opts {
        crate::commands::parse_args(&s.split_whitespace().map(String::from).collect::<Vec<_>>()).unwrap()
    }

    #[test]
    fn ibis_command_json() {
        let b = synth();
        let v = crate::commands::run_file(&args("ibis synthetic_full.ibs"), "dir/synthetic_full.ibs", &b).unwrap();
        assert_eq!(v["file"], "synthetic_full.ibs");
        assert_eq!(v["sha256"].as_str().unwrap(), crate::ibis_parser::sha256::hex(&b));
        assert_eq!((v["counts"]["errors"].as_u64(), v["counts"]["models"].as_u64(), v["counts"]["complete"].as_bool()), (Some(0), Some(8), Some(true)));
        let c = &v["components"][0];
        assert_eq!(c["pins_total"], 16);
        assert_eq!(c["pins_by_kind"]["selector"], 3);
        let pin = |n: &str| c["pins"].as_array().unwrap().iter().find(|p| p["pin"] == n).unwrap().clone();
        let p4 = pin("4");
        assert_eq!((p4["kind"].as_str(), p4["r_ohm"].as_f64(), p4["l_nh"].as_f64(), p4["c_pf"].as_f64(), p4["rlc_from"].as_str()), (Some("selector"), Some(0.08), Some(1.8), Some(0.45), Some("pin")));
        assert_eq!((pin("12")["rlc_from"].as_str(), pin("12")["r_ohm"].as_f64()), (Some("pin+package"), Some(0.1)));
        assert_eq!(pin("15")["rlc_from"], "package");
        assert_eq!(c["diff_pins"][0]["vdiff_v"], 0.1);
        assert_eq!(c["package"]["c_pf"]["min"], Value::Null);
        let m = |n: &str| v["models"].as_array().unwrap().iter().find(|m| m["name"] == n).unwrap().clone();
        let io = m("IO_50");
        assert_eq!(io["derived"]["ron_pulldown_ohm"], json!({"typ": 50.0, "min": 60.0, "max": 40.0}));
        assert_eq!(io["derived"]["ron_pullup_ohm"]["min"], 60.0);
        assert!(io["derived"]["roff_note"].is_string());
        assert_eq!(io["c_comp_pf"], json!({"typ": 2.0, "min": 1.8, "max": 2.2}));
        assert_eq!(io["tables_rows"]["pulldown"], 37);
        assert_eq!(io["waveforms"].as_array().unwrap().len(), 4);
        assert!((io["derived"]["slew_rising_v_per_ns"]["typ"].as_f64().unwrap() - 0.99 / 0.411042).abs() < 1e-3);
        assert_eq!(m("IN_ODT")["derived"]["roff_ohm"]["typ"], 50.0);
        assert_eq!(m("SW")["series"]["off"]["r_ohm"]["typ"], 1e6);
        assert_eq!(m("TERM")["terminator"]["rpower_ohm"]["typ"], 1e12);
        assert!(v["issues"].as_array().unwrap().iter().all(|i| i["code"] == "unsupported-keyword" || i["code"] == "after-end"));
        // the tables themselves are never printed
        assert!(!v.to_string().contains("\"rows\":[") && !v.to_string().contains("0.0198"));
        // filters
        let v = crate::commands::run_file(&args("ibis x.ibs --component kisi_synth"), "x.ibs", &b).unwrap();
        let names: BTreeSet<&str> = v["models"].as_array().unwrap().iter().map(|m| m["name"].as_str().unwrap()).collect();
        assert_eq!(names, BTreeSet::from(["IO_50", "IO_34", "DRV", "IN_ODT", "OD_OUT", "TERM", "SER", "SW"]));
        assert!(v["submodels"].is_null());
        let v = crate::commands::run_file(&args("ibis x.ibs --model IO_34"), "x.ibs", &b).unwrap();
        assert_eq!(v["models"].as_array().unwrap().len(), 1);
        assert_eq!(v["model_selectors"][0]["name"], "IO_SEL");
        assert_eq!(v["components"][0]["pins_with_model"], json!(["4", "5", "6"]));
        assert!(v["components"][0]["pins"].is_null());
        assert!(crate::commands::run_file(&args("ibis x.ibs --model NOPE"), "x.ibs", &b).unwrap_err().contains("no [Model] 'NOPE'"));
        assert!(crate::commands::run_file(&args("ibis x.ibs --component NOPE"), "x.ibs", &b).unwrap_err().contains("KISI_SYNTH"));
    }

    const PCB: &str = r#"(kicad_pcb (version 20240108) (generator "pcbnew")
  (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
  (setup (stackup (layer "F.Cu" (type "copper") (thickness 0.035))
    (layer "dielectric 1" (type "core") (thickness 1.5) (material "FR4") (epsilon_r 4.5) (loss_tangent 0.02))
    (layer "B.Cu" (type "copper") (thickness 0.035))))
  (net 0 "") (net 1 "+3V3") (net 2 "GND") (net 3 "SIG3") (net 4 "SIG4") (net 5 "TX_P") (net 6 "TX_N") (net 7 "RXODT") (net 8 "SIG99") (net 9 "SIG12")
  (footprint "BGA" (layer "F.Cu") (at 10 10) (property "Reference" "U1") (property "Value" "SYNTH")
    (pad "1" smd rect (at 0 0) (size 0.5 0.5) (layers "F.Cu") (net 1 "+3V3") (pinfunction "VCC"))
    (pad "2" smd rect (at 0 1) (size 0.5 0.5) (layers "F.Cu") (net 2 "GND") (pinfunction "GND"))
    (pad "3" smd rect (at 0 2) (size 0.5 0.5) (layers "F.Cu") (net 3 "SIG3") (pinfunction "NC"))
    (pad "4" smd rect (at 0 3) (size 0.5 0.5) (layers "F.Cu") (net 4 "SIG4") (pinfunction "IO0"))
    (pad "5" smd rect (at 0 4) (size 0.5 0.5) (layers "F.Cu") (net 5 "TX_P") (pinfunction "TX_P"))
    (pad "6" smd rect (at 0 5) (size 0.5 0.5) (layers "F.Cu") (net 6 "TX_N") (pinfunction "TX_N"))
    (pad "11" smd rect (at 0 6) (size 0.5 0.5) (layers "F.Cu") (net 7 "RXODT") (pinfunction "RX_ODT"))
    (pad "12" smd rect (at 0 7) (size 0.5 0.5) (layers "F.Cu") (net 9 "SIG12") (pinfunction "OD0"))
    (pad "99" smd rect (at 0 8) (size 0.5 0.5) (layers "F.Cu") (net 8 "SIG99") (pinfunction "IO1")))
  (footprint "QFN" (layer "F.Cu") (at 30 10) (property "Reference" "U2") (property "Value" "RX")
    (pad "1" smd rect (at 0 3) (size 0.5 0.5) (layers "F.Cu") (net 4 "SIG4"))
    (pad "2" smd rect (at 0 6) (size 0.5 0.5) (layers "F.Cu") (net 7 "RXODT")))
  (segment (start 10 12) (end 20 12) (width 0.2) (layer "F.Cu") (net 3))
  (segment (start 10 13) (end 30 13) (width 0.2) (layer "F.Cu") (net 4))
  (segment (start 10 14) (end 20 14) (width 0.2) (layer "F.Cu") (net 5))
  (segment (start 10 15) (end 20 15) (width 0.2) (layer "F.Cu") (net 6))
  (segment (start 10 16) (end 30 16) (width 0.2) (layer "F.Cu") (net 7))
  (segment (start 10 17) (end 20 17) (width 0.2) (layer "F.Cu") (net 9))
  (segment (start 10 18) (end 20 18) (width 0.2) (layer "F.Cu") (net 8)))
"#;

    fn project(ibis: &str, files: Option<IbisFiles>) -> Project {
        let cfg = format!(
            r#"{{"kisi_config": 1, "net_classes": {{"Default": {{"target_ohm": 50, "diff": false, "source": {{"kind": "assumption", "note": "test"}}}}}},
                "ibis": {ibis}}}"#
        );
        Project::from_texts(Inputs {
            pcb_path: "b.kicad_pcb".into(),
            pcb: PCB.into(),
            pro: None,
            config: Some(("b.kisi.json".into(), Ok(cfg))),
            stackup: None,
            field: HashMap::new(),
            variant: None,
            files: Default::default(),
            ibis_files: files,
        })
        .unwrap()
    }

    fn files() -> IbisFiles {
        HashMap::from([("m/synth.ibs".to_string(), Ok(synth()))])
    }

    #[test]
    fn mapping_check() {
        let sha = crate::ibis_parser::sha256::hex(&synth());
        // without the files (any command but config-check): only the static checks
        let p = project(r#"{"U1": {"file": "m/synth.ibs", "component": "KISI_SYNTH"}}"#, None);
        assert!(p.ibis.is_none());
        let ck = p.config_check.as_ref().unwrap();
        assert!(ck.ok && ck.warnings.iter().any(|w| w.contains("ibis.U1: source missing")), "{ck:?}");
        // with the files, pads by number: pin 3 is NC, pad 99 has no IBIS pin, IO_SEL not chosen
        let p = project(r#"{"U1": {"file": "m/synth.ibs", "component": "KISI_SYNTH", "source": {"kind": "simulation", "note": "synthetic"}}}"#, Some(files()));
        let ck = p.config_check.as_ref().unwrap();
        assert!(ck.ok, "{:?}", ck.errors);
        let w = ck.warnings.join("\n");
        assert!(w.contains("2 of 7 pad(s)") && w.contains("3 (SIG3: IBIS pin 3 is NC)") && w.contains("99 (SIG99: no such pin in the IBIS file)"), "{w}");
        assert!(w.contains("pad(s) 4, 5, 6 use [Model Selector] IO_SEL without a choice") && w.contains("first entry, IO_50"), "{w}");
        assert!(w.contains(&format!("\"sha256\": \"{sha}\"")), "{w}");
        let part = &p.ibis.as_ref().unwrap()[0];
        assert_eq!((part["pads_checked"].as_u64(), part["pads_with_model"].as_u64(), part["loaded"].as_bool()), (Some(7), Some(5), Some(true)));
        // the choice, an exception for pad 99 and the right hash: only pad 3 is left
        let p = project(&format!(r#"{{"U1": {{"file": "m/synth.ibs", "sha256": "{sha}", "selectors": {{"IO_SEL": "IO_34"}}, "pins": {{"99": "16"}}, "source": {{"kind": "simulation", "note": "x"}}}}}}"#), Some(files()));
        let ck = p.config_check.as_ref().unwrap();
        assert!(ck.ok, "{:?}", ck.errors);
        let ibis_w: Vec<&String> = ck.warnings.iter().filter(|w| w.starts_with("ibis.")).collect();
        assert_eq!(ibis_w.len(), 1, "{ibis_w:?}");
        assert!(ibis_w[0].contains("1 of 7") && ibis_w[0].contains("3 (SIG3"));
        let part = &p.ibis.as_ref().unwrap()[0];
        assert_eq!((part["sha256_ok"].as_bool(), part["component"].as_str()), (Some(true), Some("KISI_SYNTH")));
        assert_eq!(part["models"], json!(["IN_ODT", "IO_34", "IO_50", "OD_OUT"]));
        // by pin name: pad 99 (IO1) finds IBIS pin 16
        let p = project(r#"{"U1": {"file": "m/synth.ibs", "pin_map": "by-name", "selectors": {"io_sel": "IO_50"}, "source": {"kind": "simulation", "note": "x"}}}"#, Some(files()));
        assert_eq!(p.ibis.as_ref().unwrap()[0]["pads_with_model"], 6);
        // errors: changed file, unknown component, selector choice, IBIS pin, static ones
        let bad = project(
            r#"{"U1": {"file": "m/synth.ibs", "sha256": "0000000000000000000000000000000000000000000000000000000000000000", "component": "OTHER", "source": {"kind": "simulation", "note": "x"}},
                "U2": {"file": "m/synth.ibs", "selectors": {"IO_SEL": "IO_99", "NOSEL": "A"}, "pins": {"1": "77", "8": "4"}, "pin_map": "by-magic", "corner": "fast", "source": {"kind": "guess"}},
                "U9": {"file": "", "sha256": "xyz"}}"#,
            Some(files()),
        );
        let ck = bad.config_check.as_ref().unwrap();
        let e = ck.errors.join("\n");
        for frag in ["the file changed", "no [Component] 'OTHER'", "'IO_99' is not one of its models", "no [Model Selector] 'NOSEL'", "pins.1: no pin '77'", "U2 has no pad '8'", "pin_map 'by-magic'", "corner 'fast'", "kind 'guess'", "no footprint U9", "\"file\" missing", "64 hex digits"] {
            assert!(e.contains(frag), "missing '{frag}' in\n{e}");
        }
        // a file that cannot be read (CLI) is an error, one not dropped (viewer) a warning
        let p = project(r#"{"U1": {"file": "m/gone.ibs", "source": {"kind": "simulation", "note": "x"}}}"#, Some(HashMap::from([("m/gone.ibs".to_string(), Err("/x/m/gone.ibs: not found".to_string()))])));
        assert!(p.config_check.as_ref().unwrap().errors.iter().any(|e| e.contains("gone.ibs: not found")));
        let p = project(r#"{"U1": {"file": "m/synth.ibs", "source": {"kind": "simulation", "note": "x"}}}"#, Some(IbisFiles::new()));
        let ck = p.config_check.as_ref().unwrap();
        assert!(ck.ok && ck.warnings.iter().any(|w| w.contains("not loaded") && w.contains("nothing is stored or uploaded")));
        // config-check shows the parts
        let v = crate::commands::run(&args("config-check b.kicad_pcb"), Some(&p), None).unwrap();
        assert_eq!(v["ibis"][0]["ref"], "U1");
    }

    #[test]
    fn linear_ends_from_the_synthetic_file() {
        let set: ParsedSet = HashMap::from([("m/synth.ibs".to_string(), Ok(parse_one("synth.ibs", &synth())))]);
        let board = crate::load_pcb(PCB).unwrap();
        let entry: crate::config::IbisEntry = serde_json::from_str(r#"{"file": "m/synth.ibs", "pins": {"99": "11"}}"#).unwrap();
        let map = BTreeMap::from([("U1".to_string(), entry)]);
        // pad 4: IBIS pin 4 (R 80 mΩ, L 1.8 nH, C 0.45 pF), [Model Selector] IO_SEL → first entry IO_50
        let d = resolve_end(&map, &set, &board, "U1", "4", Role::Driver, false).unwrap();
        assert!(d.error.is_none(), "{:?}", d.error);
        assert_eq!((d.model.as_deref(), d.selector.as_deref()), (Some("IO_50"), Some("IO_SEL")));
        assert!(d.model_from.as_deref().unwrap().contains("first entry"));
        // Ron 50 Ω (pullup and pulldown) + R_pin; C_comp 2.0 pF + C_pin; L_pin
        assert!((d.r_ohm - 50.08).abs() < 1e-6 && (d.c_pf - 2.45).abs() < 1e-9 && (d.l_nh - 1.8).abs() < 1e-9, "{} {} {}", d.r_ohm, d.c_pf, d.l_nh);
        // edge: the raised-cosine waveforms have 411.042 ps 20–80 %, × 1.5228 for 10–90 %
        assert!((d.rise_ps.unwrap() - 411.042 * RISE_10_90_PER_20_80).abs() < 0.1, "{:?}", d.rise_ps);
        assert_eq!(d.vcc_v, Some(3.3));
        assert_eq!(d.candidates, ["DRV", "IO_34", "IO_50", "OD_OUT"]);
        // corner max: Ron 40 Ω, faster edge
        let mut m2 = map.clone();
        m2.get_mut("U1").unwrap().corner = Some("max".into());
        let dx = resolve_end(&m2, &set, &board, "U1", "4", Role::Driver, false).unwrap();
        assert!((dx.r_ohm - 40.08).abs() < 1e-6 && dx.rise_ps.unwrap() < d.rise_ps.unwrap());
        // the receiver with on-die termination (pad 99 → IBIS pin 11, IN_ODT: 2 × 100 Ω)
        let r = resolve_end(&map, &set, &board, "U1", "99", Role::Receiver, false).unwrap();
        assert!(r.error.is_none(), "{:?}", r.error);
        assert!((r.r_ohm - 50.0).abs() < 1e-6 && (r.c_pf - 1.9).abs() < 1e-9 && (r.l_nh - 1.2).abs() < 1e-9, "{} {} {}", r.r_ohm, r.c_pf, r.l_nh);
        assert!(r.termination.as_deref().unwrap().contains("on-die"));
        // an input cannot drive; an override; a selector choice; NC; differential pins
        let bad = resolve_end(&map, &set, &board, "U1", "99", Role::Driver, false).unwrap();
        assert!(bad.error.as_deref().unwrap().contains("cannot drive") && bad.end_model().is_none());
        let mut m3 = map.clone();
        m3.get_mut("U1").unwrap().models.insert("4".into(), "OD_OUT".into());
        let od = resolve_end(&m3, &set, &board, "U1", "4", Role::Driver, false).unwrap();
        assert_eq!((od.model_from.as_deref(), od.r_ohm), (Some("override"), 25.08));
        assert!(!od.warnings.is_empty());
        m3.get_mut("U1").unwrap().selectors.insert("IO_SEL".into(), "IO_34".into());
        let s34 = resolve_end(&m3, &set, &board, "U1", "5", Role::Driver, true).unwrap();
        assert!(s34.model.as_deref() == Some("IO_34") && s34.diff_pin.as_deref().unwrap().starts_with("5 / 6"));
        assert!(resolve_end(&map, &set, &board, "U1", "3", Role::Receiver, false).unwrap().error.unwrap().contains("NC"));
        assert!(resolve_end(&map, &set, &board, "U2", "1", Role::Receiver, false).is_none());
        // not loaded: an end with the reason
        let none = resolve_end(&map, &ParsedSet::new(), &board, "U1", "4", Role::Driver, false).unwrap();
        assert!(none.error.unwrap().contains("not loaded"));
        // applied to a lane: IBIS where usable, else the profile's
        let prof = crate::eye::EndModel { r: 50.0, c_pf: 1.0, l_nh: 0.0 };
        let ap = apply_ends(&[Some(d.clone()), Some(bad)], prof, prof, 100e-12);
        assert!(ap.ibis_used && ap.mapped && (ap.tx.r - 50.08).abs() < 1e-9 && ap.rx.r == 50.0 && ap.rx.c_pf == 1.0);
        assert!((ap.rise_s - d.rise_ps.unwrap() * 1e-12).abs() < 1e-18);
        assert_eq!(ap.rx_info["model"], "profile");
        // eye swing: 3.3 V into 50 Ω from 50.08 Ω, high impedance: 3.3 V; a pair doubles it
        assert!((eye_swing_mv(3.3, &ap.tx, &crate::eye::EndModel { r: 50.0, ..prof }, false) - 3300.0 * 50.0 / 100.08).abs() < 0.1);
        assert_eq!(eye_swing_mv(3.3, &ap.tx, &crate::eye::EndModel { r: 0.0, ..prof }, true), 6600.0);
    }

    #[test]
    fn lane_check_uses_ibis_ends() {
        let ent = r#"{"U1": {"file": "m/synth.ibs", "selectors": {"IO_SEL": "IO_50"}, "source": {"kind": "simulation", "note": "x"}},
                      "U2": {"file": "m/synth.ibs", "pins": {"1": "11", "2": "11"}, "source": {"kind": "simulation", "note": "x"}}}"#;
        let with = project(ent, Some(files()));
        let without = project(ent, None);
        let rows = |p: &Project| {
            let v = crate::commands::run(&args("check b.kicad_pcb --all"), Some(p), None).unwrap();
            v["rows"].as_array().unwrap().iter().find(|r| r["lane"].as_str() == Some("SIG4")).cloned().expect("SIG4 lane")
        };
        let (a, b) = (rows(&with), rows(&without));
        let (tx, rx) = (&a["ends"]["tx"], &a["ends"]["rx"]);
        // U1 drives through IO_50, U2 receives through IN_ODT (50 Ω on-die termination)
        assert_eq!((tx["refdes"].as_str(), tx["model"].as_str(), rx["refdes"].as_str(), rx["model"].as_str()), (Some("U1"), Some("IO_50"), Some("U2"), Some("IN_ODT")), "{}", a["ends"]);
        assert_eq!(rx["r_ohm"], 50.0);
        assert!(a["ends"]["note"].as_str().unwrap().contains("linearised IBIS model (estimate)"));
        // without the files: the profile's ends, with the reason
        assert_eq!(b["ends"]["tx"]["model"], "profile");
        assert!(b["ends"]["tx"]["ibis"]["error"].as_str().unwrap().contains("not loaded"));
        // the eye changes (slower IBIS edge and capacitances), loss and return loss do not
        let eye = |r: &Value| r["metrics"]["eye_h"].as_f64().unwrap();
        assert!((eye(&a) - eye(&b)).abs() > 1e-4, "{} vs {}", eye(&a), eye(&b));
        assert_eq!(a["metrics"]["il_nyq_db"], b["metrics"]["il_nyq_db"]);
        // interfaces names the ends too
        let v = crate::commands::run(&args("interfaces b.kicad_pcb"), Some(&with), None).unwrap();
        let lanes: Vec<&Value> = v["buses"].as_array().unwrap().iter().flat_map(|b| b["lanes"].as_array().unwrap()).collect();
        let l = lanes.iter().find(|l| l["lane"] == "SIG4").unwrap();
        assert_eq!(l["ibis_ends"]["tx"]["model"], "IO_50");
    }

    #[test]
    fn gaps_lists_lane_ends_without_model() {
        let p = project(r#"{"U1": {"file": "m/synth.ibs", "source": {"kind": "simulation", "note": "x"}}}"#, None);
        let g = crate::commands::run(&args("gaps b.kicad_pcb"), Some(&p), None).unwrap();
        let gap = g["gaps"].as_array().unwrap().iter().find(|g| g["id"] == "ibis-models").expect("ibis-models gap");
        assert_eq!(gap["priority"], "low");
        let refs: Vec<&str> = gap["items"].as_array().unwrap().iter().map(|i| i["ref"].as_str().unwrap()).collect();
        assert_eq!(refs, ["U2"]);
        assert_eq!(gap["items"][0]["lanes"], 2);
    }
}

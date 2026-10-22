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
        "parser": ip::NOTICE,
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
pub fn check_files(cfg: &BoardConfig, board: &Board, signal: &HashMap<u32, String>, files: &IbisFiles) -> FileCheck {
    let mut out = FileCheck::default();
    let mut parsed: HashMap<&str, (IbisFile, String)> = HashMap::new();
    for (refdes, e) in cfg.ibis.iter().filter(|(_, e)| !e.file.trim().is_empty()) {
        let what = format!("ibis.{refdes}");
        let Some(fi) = board.footprints.iter().position(|f| &f.reference == refdes) else { continue };
        let fp = &board.footprints[fi];
        let mut part = json!({"ref": refdes, "value": fp.value, "file": e.file, "component": e.component, "corner": e.corner.as_deref().unwrap_or("typ"), "pin_map": e.pin_map.as_deref().unwrap_or("by-number"), "loaded": false});
        if fp.dnp {
            out.warnings.push(format!("{what}: {refdes} is not fitted (DNP or not in the chosen assembly variant): the mapping is not used"));
            part["fitted"] = json!(false);
        }
        let bytes = match files.get(&e.file) {
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
            Some(Ok(b)) => b,
        };
        let (f, sha) = &*parsed.entry(e.file.as_str()).or_insert_with(|| (ip::read(bytes, Some(base(&e.file))), ip::sha256::hex(bytes)));
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

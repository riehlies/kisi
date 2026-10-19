//! Gap list: which data the analyses miss or only have as defaults, where it is
//! usually found and how to write it into `<board>.kisi.json`.
//!
//! This is the deterministic half of preparing a board; the research half (reading
//! datasheets, choosing values with a source) is done by a person or an agent
//! following the kisi-prepare skill, which then runs `kisi config-check`.

use crate::profile::Profiles;
use crate::service::Project;
use crate::topology::{self, ComponentData};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, HashSet};

const GENERIC: &[&str] = &["se50", "diff100"];

fn gap(id: &str, priority: &str, area: &str, issue: String, affects: &[&str], source: &str, write: Value, items: Vec<Value>) -> Value {
    json!({"id": id, "priority": priority, "area": area, "issue": issue, "affects": affects, "where_to_find": source, "write": write, "items": items})
}

/// All gaps of a project (after its `<board>.kisi.json` was applied).
pub fn gaps(p: &Project, profiles: &Profiles) -> Value {
    let b = &p.board;
    let su = b.stackup();
    let cd = ComponentData::builtin();
    let cfg = p.config.clone().unwrap_or_default();
    let mut out: Vec<Value> = Vec::new();

    // --- stackup ---------------------------------------------------------------------
    if !su.from_file {
        out.push(gap(
            "stackup-missing",
            "high",
            "stackup",
            "The board file has no stackup: kisi uses 1.6 mm FR4 (εr 4.5, tanδ 0.02) with equal dielectric thicknesses.".into(),
            &["impedance", "loss", "eye", "crosstalk"],
            "The fabricator's stackup for this board (layer thicknesses, laminate per layer); better: enter it in KiCad (Board Setup → Physical Stackup).",
            json!({"to": "stackup.dielectrics[]", "example": {"index": 0, "material": "…", "thickness": 0.2, "epsilon_r": 4.2, "loss_tangent": 0.015, "source": {"kind": "datasheet", "document": "…", "page": "…"}}}),
            vec![],
        ));
    }
    let configured = |i: usize, what: &str| {
        cfg.stackup.dielectrics.iter().any(|d| {
            d.index == i
                && match what {
                    "epsilon_r" => d.epsilon_r.is_some(),
                    "loss_tangent" => d.loss_tangent.is_some(),
                    _ => d.material.is_some(),
                }
        })
    };
    let mut diel_items = Vec::new();
    for (i, d) in su.dielectrics.iter().enumerate() {
        let mut why = Vec::new();
        let generic_mat = d.material.is_empty() || d.material.eq_ignore_ascii_case("fr4") || d.material.contains("assumed");
        if !configured(i, "loss_tangent") && (d.assumed.iter().any(|a| a == "loss_tangent") || (d.loss_tangent - 0.02).abs() < 1e-9) {
            why.push("tanδ 0.02 is KiCad's default value");
        }
        if !configured(i, "epsilon_r") && (d.assumed.iter().any(|a| a == "epsilon_r") || ((d.epsilon_r - 4.5).abs() < 1e-9 && generic_mat)) {
            why.push("εr 4.5 is KiCad's default value");
        }
        if !configured(i, "material") && d.material.trim().is_empty() {
            why.push("no material named");
        }
        if !why.is_empty() {
            let above = su.copper.get(i).map(|c| c.name.clone()).unwrap_or_default();
            let below = su.copper.get(i + 1).map(|c| c.name.clone()).unwrap_or_default();
            diel_items.push(json!({"index": i, "between": format!("{above} / {below}"), "kind": d.kind, "material": d.material, "thickness_mm": d.thickness,
                "epsilon_r": d.epsilon_r, "loss_tangent": d.loss_tangent, "why": why}));
        }
    }
    if su.from_file && !diel_items.is_empty() {
        out.push(gap(
            "stackup-defaults",
            "medium",
            "stackup",
            format!("{} dielectric layer(s) carry default or missing material data.", diel_items.len()),
            &["impedance (εr)", "loss and eye (tanδ)"],
            "Laminate datasheet of the named material (εr and tanδ near the signal frequencies, e.g. 1–10 GHz, for the resin content / glass style of this layer), or the fabricator's stackup report. Mark typical values as such.",
            json!({"to": "stackup.dielectrics[]", "example": {"index": diel_items[0]["index"], "epsilon_r": 3.65, "loss_tangent": 0.0095, "source": {"kind": "datasheet", "document": "laminate datasheet", "url": "…", "page": "…", "note": "at 10 GHz, typical"}}}),
            diel_items,
        ));
    }

    // --- impedance targets and interface profiles -------------------------------------
    // by name only, so skip what cannot be such a signal: supply nets, stubs, diff profiles without a partner
    let supply: HashSet<String> = crate::pi::power_nets(b).into_iter().map(|n| n.name).collect();
    let mut untargeted: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    for st in p.an.nets.iter().filter(|n| n.target.is_none() && n.length >= 5.0) {
        if supply.contains(&st.name) || crate::pi::voltage_from_name(&st.name).is_some() {
            continue;
        }
        let diff = st.partner.is_some();
        let pid = profiles.auto(&st.name, "", diff);
        if GENERIC.contains(&pid) || (!diff && profiles.get(pid).is_some_and(|p| p.signal == "diff")) {
            continue;
        }
        untargeted.entry(st.class.clone()).or_default().push(json!({"net": st.name, "looks_like": pid, "length_mm": (st.length * 10.0).round() / 10.0}));
    }
    if !untargeted.is_empty() {
        let n: usize = untargeted.values().map(|v| v.len()).sum();
        let items: Vec<Value> = untargeted.into_iter().map(|(c, nets)| json!({"class": c, "nets": nets})).collect();
        out.push(gap(
            "nets-without-target",
            "high",
            "impedance",
            format!("{n} routed net(s) look like high-speed interfaces by their name but have no target impedance (their net class has none). Name-based: confirm before acting."),
            &["impedance check", "channel check", "eye"],
            "Interface standard or the device datasheets (e.g. 85/90/100 Ω differential). Nets in the class “Default” need their own net class in KiCad first (Board Setup → Net Classes, or net class directives in the schematic) – kisi.json can only set values of existing classes.",
            json!({"to": "net_classes.<class>", "example": {"target_ohm": 90, "diff": true, "profile": "usb2", "source": {"kind": "standard", "document": "USB 2.0 specification", "note": "90 Ω differential"}}}),
            items,
        ));
    }
    let mut unclear = Vec::new();
    for c in p.an.classes.iter().filter(|c| c.target.is_some() && c.nets > 0) {
        if p.config_profiles.contains_key(&c.name) {
            continue;
        }
        let nets: Vec<&crate::extract::NetStat> = p.an.nets.iter().filter(|n| n.class == c.name && n.length > 0.0).collect();
        let diff = c.target.is_some_and(|t| t.diff);
        let ids: BTreeSet<&str> = nets.iter().map(|n| profiles.auto(&n.name, &c.name, diff)).collect();
        if ids.iter().all(|i| GENERIC.contains(i)) {
            unclear.push(json!({"class": c.name, "target_ohm": c.target.map(|t| t.z), "diff": diff, "nets": nets.len(),
                "examples": nets.iter().take(4).map(|n| n.name.clone()).collect::<Vec<_>>(), "profile_now": ids.iter().next()}));
        }
    }
    if !unclear.is_empty() {
        out.push(gap(
            "classes-without-standard",
            "medium",
            "interfaces",
            format!("{} net class(es) with a target impedance get only a generic profile (no data rate / standard): the channel check uses 1 Gbit/s and generic limits.", unclear.len()),
            &["channel check", "eye"],
            "What runs on these nets: schematic (bus names, the devices at the ends: `kisi interfaces`) and the device datasheets (interface, data rate). Available profiles: `kisi profiles`.",
            json!({"to": "net_classes.<class>.profile", "example": {"profile": "lvds", "source": {"kind": "datasheet", "document": "ADC datasheet", "page": "…", "note": "LVDS outputs, 1 Gbit/s per lane"}}}),
            unclear,
        ));
    }

    // --- channel parts with unreadable values ------------------------------------------
    let buses = p.buses();
    let mut bad_parts: BTreeMap<String, Value> = BTreeMap::new();
    for l in buses.iter().flat_map(|b| &b.lanes) {
        let series = l.hops.iter().filter_map(|h| match h {
            topology::Hop::Series { part } => Some(part),
            _ => None,
        });
        for part in series.chain(l.shunts.iter().map(|s| &s.part)) {
            if part.val.is_none() {
                bad_parts.entry(part.refdes.clone()).or_insert_with(|| json!({"ref": part.refdes, "value": part.value, "lane": l.name}));
            }
        }
    }
    if !bad_parts.is_empty() {
        out.push(gap(
            "channel-part-values",
            "medium",
            "interfaces",
            format!("{} part(s) in signal channels have a value kisi cannot read (e.g. “DNP”, a part number): modelled as open / very large.", bad_parts.len()),
            &["channel check", "eye"],
            "Schematic / BOM: give the Value field a plain value (22, 4k7, 100n) or mark the part as DNP in KiCad. This is a fix in KiCad, not in kisi.json.",
            json!({"to": "KiCad Value field (not kisi.json)"}),
            bad_parts.into_values().collect(),
        ));
    }

    // --- power ---------------------------------------------------------------------------
    let pnets: Vec<crate::pi::PowerNet> = crate::pi::power_nets(b).into_iter().filter(|n| !n.ground && n.voltage.is_some()).collect();
    let (rails, _) = crate::pi::rails_from_fields(b);
    let tree = crate::tree::tree_from_fields(b);
    let reg_out: HashSet<String> = tree.as_ref().map(|t| t.0.elements.iter().map(|e| e.to.clone()).collect()).unwrap_or_default();
    let regs: HashSet<String> = tree.as_ref().map(|t| t.0.elements.iter().map(|e| e.refdes.clone()).collect()).unwrap_or_default();
    let short = |s: &str| s.rsplit('/').next().unwrap_or(s).to_string();
    let sourced: HashSet<String> = rails.iter().filter(|r| !r.sources.is_empty()).map(|r| short(&r.net)).chain(reg_out.iter().map(|n| short(n))).collect();
    let no_source: Vec<Value> = pnets
        .iter()
        .filter(|n| !sourced.contains(&short(&n.name)) && n.components.len() > 1)
        .map(|n| json!({"rail": n.name, "voltage": n.voltage, "components": n.components.len()}))
        .collect();
    if tree.is_none() {
        out.push(gap(
            "power-tree",
            "medium",
            "power",
            "No regulator chain (field PI_REG): no power budget, no regulator load check.".into(),
            &["power budget", "regulator loading"],
            "Regulator datasheets: topology (buck/LDO/…), efficiency at the expected load, rated output current; input and output rails from the schematic.",
            json!({"to": "fields[] (name PI_REG on the regulator)", "example": {"ref": "U3", "name": "PI_REG", "value": "+12V > +3V3 buck 90% 3A", "source": {"kind": "datasheet", "document": "regulator datasheet", "page": "…", "note": "efficiency at 1 A, typical"}}}),
            vec![],
        ));
    }
    if !no_source.is_empty() {
        out.push(gap(
            "rails-without-source",
            "medium",
            "power",
            format!("{} supply rail(s) have no source (PI_SOURCE on a connector/regulator output, or PI_REG): no IR drop for them.", no_source.len()),
            &["DC IR drop", "power budget"],
            "Schematic / netlist: which connector or regulator feeds the rail (the pad nets of the regulator show input and output).",
            json!({"to": "fields[]", "example": {"ref": "J1", "name": "PI_REG", "value": "+5V input", "source": {"kind": "assumption", "note": "barrel jack J1 feeds +5V (schematic / netlist)"}},
                "note": "supply inputs: PI_REG '<rail> input' (counts in the power budget and as IR-drop source); regulator outputs: PI_REG on the regulator (add out=<part> when the output current leaves through an inductor or diode); PI_SOURCE only marks IR-drop source pads without a power tree"}),
            no_source,
        ));
    }
    let loaded: HashSet<&str> = b.footprints.iter().filter(|f| f.prop("PI_LOAD").is_some()).map(|f| f.reference.as_str()).collect();
    let mut on_rails: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let rail_names: HashSet<&str> = pnets.iter().map(|n| n.name.as_str()).collect();
    for pad in &b.pads {
        let Some(f) = b.footprints.get(pad.fp as usize) else { continue };
        if f.dnp || cd.role(&f.reference) != "ic" || loaded.contains(f.reference.as_str()) || regs.contains(&f.reference) {
            continue;
        }
        let net = b.net_name(pad.net);
        if rail_names.contains(net) {
            on_rails.entry(f.reference.clone()).or_default().insert(short(net));
        }
    }
    if !on_rails.is_empty() {
        let items: Vec<Value> = on_rails
            .iter()
            .map(|(r, rails)| {
                let f = b.footprints.iter().find(|f| &f.reference == r);
                json!({"ref": r, "value": f.map(|f| f.value.clone()), "rails": rails})
            })
            .collect();
        out.push(gap(
            "ic-loads",
            "medium",
            "power",
            format!("{} IC(s) on supply rails have no load current (field PI_LOAD): their current is missing from IR drop and budget.", items.len()),
            &["DC IR drop", "power budget", "PDN target impedance"],
            "Device datasheets: supply current per rail in the expected operating mode (state typical or maximum in the source note); FPGAs/SoCs: the vendor's power estimator.",
            json!({"to": "fields[] (name PI_LOAD)", "example": {"ref": items[0]["ref"], "name": "PI_LOAD", "value": "+3V3=120mA", "source": {"kind": "datasheet", "url": "…", "page": "…", "note": "typical, all channels active"}}}),
            items,
        ));
    }
    let mut caps = Vec::new();
    let mut seen = HashSet::new();
    for pad in &b.pads {
        let Some(f) = b.footprints.get(pad.fp as usize) else { continue };
        if f.dnp || cd.role(&f.reference) != "capacitor" || !rail_names.contains(b.net_name(pad.net)) || !seen.insert(f.reference.clone()) {
            continue;
        }
        if crate::pdn::parse_cap(&f.value).is_none() {
            caps.push(json!({"ref": f.reference, "value": f.value, "rail": short(b.net_name(pad.net))}));
        }
    }
    if !caps.is_empty() {
        out.push(gap(
            "cap-values",
            "low",
            "power",
            format!("{} capacitor(s) on supply rails have a value kisi cannot read: left out of Z(f).", caps.len()),
            &["PDN impedance"],
            "Schematic / BOM: plain capacitance in the Value field (100n, 4u7, 10µF/6.3V). A fix in KiCad, not in kisi.json.",
            json!({"to": "KiCad Value field (not kisi.json)"}),
            caps,
        ));
    }

    let rank = |v: &Value| match v["priority"].as_str() {
        Some("high") => 0,
        Some("medium") => 1,
        _ => 2,
    };
    out.sort_by_key(rank);
    let count = |pr: &str| out.iter().filter(|g| g["priority"] == pr).count();
    json!({
        "board": p.name,
        "config": p.config_summary(),
        "summary": {"high": count("high"), "medium": count("medium"), "low": count("low")},
        "gaps": out,
        "how": "Fill the gaps in <board>.kisi.json next to the board (format: see `kisi config-template`), every value with a source; then run `kisi config-check <board>` and `kisi gaps <board>` again. Gaps marked 'not kisi.json' are fixes in KiCad."
    })
}

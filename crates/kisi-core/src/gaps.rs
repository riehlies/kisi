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
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

const GENERIC: &[&str] = &["se50", "diff100"];

/// What each gap affects (in words), per gap id: the source of the gap's analysis categories
/// (`views`, via `AFFECT_VIEWS`).
pub const GAP_AFFECTS: &[(&str, &[&str])] = &[
    ("stackup-missing", &["impedance", "loss", "eye", "crosstalk"]),
    ("stackup-defaults", &["impedance (εr)", "loss and eye (tanδ)"]),
    ("nets-without-target", &["impedance check", "channel check", "eye"]),
    ("classes-without-standard", &["channel check", "eye"]),
    ("assembly-variant", &["lanes", "channel check", "eye", "PDN (fitted capacitors)"]),
    ("assembly-variant-forks", &["lanes", "channel check", "eye"]),
    ("channel-part-values", &["channel check", "eye"]),
    ("power-tree", &["power budget", "regulator loading"]),
    ("rails-without-voltage", &["power budget", "DC IR drop", "PDN"]),
    ("rails-without-source", &["DC IR drop", "power budget"]),
    ("regulator-efficiency", &["power budget", "regulator losses", "input current"]),
    ("ic-loads", &["DC IR drop", "power budget", "PDN target impedance"]),
    ("cap-ratings", &["PDN impedance", "decoupling recommendations"]),
    ("bead-ratings", &["PDN impedance per IC", "filter resonances"]),
    ("cap-values", &["PDN impedance"]),
    ("thermal-data", &["temperature estimate", "junction temperature"]),
];

/// The analysis category (viewer view) of each word in `GAP_AFFECTS`: impedance, channel or power
/// (the temperature estimate is part of power). A test checks that every word has one.
pub const AFFECT_VIEWS: &[(&str, &str)] = &[
    ("impedance", "impedance"),
    ("impedance (εr)", "impedance"),
    ("impedance check", "impedance"),
    ("crosstalk", "impedance"),
    ("loss", "channel"),
    ("eye", "channel"),
    ("loss and eye (tanδ)", "channel"),
    ("channel check", "channel"),
    ("lanes", "channel"),
    ("PDN (fitted capacitors)", "power"),
    ("power budget", "power"),
    ("regulator loading", "power"),
    ("regulator losses", "power"),
    ("input current", "power"),
    ("DC IR drop", "power"),
    ("PDN", "power"),
    ("PDN target impedance", "power"),
    ("PDN impedance", "power"),
    ("PDN impedance per IC", "power"),
    ("decoupling recommendations", "power"),
    ("filter resonances", "power"),
    ("temperature estimate", "power"),
    ("junction temperature", "power"),
];

/// The analysis categories, in the viewer's order.
pub const VIEWS: &[&str] = &["impedance", "channel", "power"];

/// What gap `id` affects (words) and its categories (`VIEWS` order).
pub fn gap_views(id: &str) -> (&'static [&'static str], Vec<&'static str>) {
    let affects = GAP_AFFECTS.iter().find(|g| g.0 == id).map(|g| g.1).unwrap_or(&[]);
    let v: Vec<&str> = VIEWS.iter().copied().filter(|v| affects.iter().any(|a| AFFECT_VIEWS.iter().any(|(w, x)| w == a && x == v))).collect();
    (affects, v)
}

fn gap(id: &str, priority: &str, area: &str, issue: String, source: &str, write: Value, items: Vec<Value>) -> Value {
    let (affects, views) = gap_views(id);
    json!({"id": id, "priority": priority, "area": area, "issue": issue, "affects": affects, "views": views, "where_to_find": source, "write": write, "items": items})
}

/// A part in a gap item: the manufacturer part number added when the footprint has one that the
/// Value field does not show (Value `100n`, a manufacturer name, …), so that the datasheet can be found.
fn part_json(mut v: Value, f: Option<&crate::board::Footprint>) -> Value {
    if let Some(m) = f.and_then(|f| f.mpn_if_not_value()) {
        v["mpn"] = json!(m);
        if let Some(mf) = f.and_then(|f| f.prop("Manufacturer")).filter(|x| !x.trim().is_empty() && x.trim() != "~") {
            v["manufacturer"] = json!(mf.trim());
        }
    }
    v
}

/// A supply rail as the gap list sees it.
#[derive(Default)]
struct SupplyRail {
    voltage: Option<f64>,
    /// why it counts as a supply: a voltage in its name, a field, its name, a pin name
    found_by: Vec<String>,
    /// fitted parts with pads on it, and the ICs among them
    components: Vec<String>,
    ics: Vec<String>,
    /// for rails without a voltage: series parts (ferrite, inductor, link ≤ 1 Ω) to a rail with a
    /// voltage – (part, that rail)
    fed_through: Vec<(String, String)>,
}

/// Supply rails of the board: nets with a voltage in the name (not signal names such as PG_1V0),
/// the rails of PI_REG / PI_LOAD /
/// PI_SOURCE (with the voltages given there, `NAME@<V>V`), and nets that look like supplies by
/// their name or a pin name (`pi::supply_name_hint`: VBUS, VSYS, VBAT, VDD, …). A net found only
/// by name needs two connecting parts (not capacitors; fitted, or pads without a part such as a
/// battery pad), so that the decoupling node of an IC's internal regulator is not taken for a rail.
fn supply_rails(b: &crate::board::Board, cd: &ComponentData) -> BTreeMap<String, SupplyRail> {
    let mut m: BTreeMap<String, SupplyRail> = BTreeMap::new();
    let add = |m: &mut BTreeMap<String, SupplyRail>, name: &str, v: Option<f64>, why: String| {
        let r = m.entry(name.to_string()).or_default();
        if r.voltage.is_none() {
            r.voltage = v.filter(|x| *x != 0.0);
        }
        if !r.found_by.contains(&why) {
            r.found_by.push(why);
        }
    };
    let mut parts: HashMap<u32, Vec<usize>> = HashMap::new();
    for pad in b.pads.iter().filter(|p| p.net != 0) {
        let v = parts.entry(pad.net).or_default();
        if !v.contains(&(pad.fp as usize)) {
            v.push(pad.fp as usize);
        }
    }
    // a voltage in the name, but not a signal at that level (PG_1V0, TRIM_SCL_5V)
    for n in crate::pi::power_nets(b).into_iter().filter(|n| !n.ground && n.voltage.is_some() && !crate::pi::signal_name(&n.name)) {
        add(&mut m, &n.name, n.voltage, "voltage in the name".into());
    }
    if let Some((spec, _)) = crate::tree::tree_from_fields(b) {
        for r in &spec.rails {
            if let Some(id) = crate::pi::find_net(b, &r.net) {
                add(&mut m, b.net_name(id), Some(r.voltage), "PI_REG".into());
            }
        }
    }
    for s in crate::pi::rails_from_fields(b).0 {
        add(&mut m, &s.net, Some(s.voltage), "PI_LOAD / PI_SOURCE".into());
    }
    let connecting = |id: u32| {
        parts.get(&id).map_or(0, |v| v.iter().filter(|&&fi| b.footprints.get(fi).is_some_and(|f| cd.role(&f.reference) != "capacitor" && (!f.dnp || f.exclude_from_bom))).count())
    };
    let known: HashSet<String> = m.keys().cloned().collect();
    for (i, n) in b.nets.iter().enumerate().skip(1) {
        if !known.contains(n) && crate::pi::supply_name_hint(n) && connecting(i as u32) >= 2 {
            add(&mut m, n, crate::pi::voltage_from_name(n), "name".into());
        }
    }
    for pad in &b.pads {
        let n = b.net_name(pad.net);
        if pad.net == 0 || known.contains(n) || m.get(n).is_some_and(|r| !r.found_by.is_empty()) || crate::pi::is_ground_name(n) {
            continue;
        }
        // a pin typed as a signal (input, output, …) is not a supply pin, whatever its name
        if pad.pinfunction.is_empty() || !matches!(pad.pintype.as_str(), "" | "power_in" | "power_out" | "passive" | "unspecified") || !crate::pi::supply_name_hint(&pad.pinfunction) {
            continue;
        }
        if connecting(pad.net) >= 2 {
            let r = b.footprints.get(pad.fp as usize).map(|f| f.reference.as_str()).unwrap_or("");
            add(&mut m, n, crate::pi::voltage_from_name(n), format!("pin {r} {}", pad.pinfunction));
        }
    }
    // parts on each rail; series parts that feed a rail without a voltage from one with a voltage
    let ids: HashMap<&str, u32> = b.nets.iter().enumerate().map(|(i, n)| (n.as_str(), i as u32)).collect();
    let with_v: HashSet<String> = m.iter().filter(|(_, r)| r.voltage.is_some()).map(|(n, _)| n.clone()).collect();
    for (n, r) in m.iter_mut() {
        let Some(id) = ids.get(n.as_str()) else { continue };
        for &fi in parts.get(id).map(|v| v.as_slice()).unwrap_or(&[]) {
            let f = &b.footprints[fi];
            if f.dnp {
                continue;
            }
            r.components.push(f.reference.clone());
            match cd.role(&f.reference) {
                "ic" => r.ics.push(f.reference.clone()),
                role @ ("inductor" | "resistor") if r.voltage.is_none() => {
                    if role == "resistor" && !crate::topology::parse_value(&f.value).is_some_and(|v| v <= 1.0) {
                        continue;
                    }
                    let mut nets: Vec<u32> = b.pads.iter().filter(|p| p.fp as usize == fi && p.net != 0).map(|p| p.net).collect();
                    nets.sort();
                    nets.dedup();
                    if let [x, y] = nets[..] {
                        let other = b.net_name(if x == *id { y } else { x });
                        if with_v.contains(other) {
                            r.fed_through.push((f.reference.clone(), other.to_string()));
                        }
                    }
                }
                _ => {}
            }
        }
    }
    m
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
            "Laminate datasheet of the named material (εr and tanδ near the signal frequencies, e.g. 1–10 GHz, for the resin content / glass style of this layer), or the fabricator's stackup report. Mark typical values as such.",
            json!({"to": "stackup.dielectrics[]", "example": {"index": diel_items[0]["index"], "epsilon_r": 3.65, "loss_tangent": 0.0095, "source": {"kind": "datasheet", "document": "laminate datasheet", "url": "…", "page": "…", "note": "at 10 GHz, typical"}}}),
            diel_items,
        ));
    }

    // --- impedance targets and interface profiles -------------------------------------
    // by name only, so skip what cannot be such a signal: supply nets, stubs, diff profiles without a partner
    let supplies = supply_rails(b, &cd);
    let dis = Dismissed::from(&cfg);
    let mut untargeted = untargeted_nets(p, profiles, &supplies);
    for nets in untargeted.values_mut() {
        nets.retain(|n| !dis.applies("nets-without-target", n["net"].as_str().unwrap_or("")));
    }
    untargeted.retain(|_, nets| !nets.is_empty());
    if !untargeted.is_empty() {
        let n: usize = untargeted.values().map(|v| v.len()).sum();
        let items: Vec<Value> = untargeted.into_iter().map(|(c, nets)| json!({"class": c, "nets": nets})).collect();
        out.push(gap(
            "nets-without-target",
            "high",
            "impedance",
            format!("{n} routed net(s) look like high-speed interfaces by their name but have no target impedance (their net class has none). Name-based: confirm before acting."),
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
            "What runs on these nets: schematic (bus names, the devices at the ends: `kisi interfaces`) and the device datasheets (interface, data rate). Available profiles: `kisi profiles`.",
            json!({"to": "net_classes.<class>.profile", "example": {"profile": "lvds", "source": {"kind": "datasheet", "document": "ADC datasheet", "page": "…", "note": "LVDS outputs, 1 Gbit/s per lane"}}}),
            unclear,
        ));
    }

    // --- assembly variant ----------------------------------------------------------------
    let vs = &p.variant;
    if vs.needs_choice || !vs.errors.is_empty() {
        let mut items: Vec<Value> = vs.variants.iter().map(|v| json!({"variant": v.name, "description": v.description, "defined_in": v.defined_in})).collect();
        items.extend(vs.forks.iter().map(|f| json!({"fork": crate::variant::fork_text(f), "net": f.net_name, "partner": f.partner, "parts": f.parts, "value": f.value, "towards": f.towards})));
        let issue = if !vs.variants.is_empty() {
            format!("The board has {} assembly variant(s) and none is chosen: interfaces, check and report stop until one is.", vs.variants.len())
        } else {
            format!("{} fork(s) where all alternative series parts are fitted – possibly assembly variants; none is chosen: interfaces, check and report stop until one is (\"default\" computes the board as it is, with the other paths as open branches).", vs.forks.len())
        };
        let issue = if vs.errors.is_empty() { issue } else { format!("{} {issue}", vs.errors.join(" ")) };
        out.push(gap(
            "assembly-variant",
            "high",
            "interfaces",
            issue,
            "The project's assembly outputs: BOM and position files per variant, the schematic's variant or DNP marks, the sheets that hold the parts of each option. KiCad 10: variants in the board file. Alternatively mark the unused parts DNP in KiCad.",
            json!({"to": "variant (and variants.<name> for boards without KiCad 10 variants)", "example": {"variant": "TS-PCIe", "variants": {"TS-PCIe": {"not_fitted_sheets": ["/TS-USB4 Components/"], "fitted": ["C101"], "source": {"kind": "design", "document": "board_TS-PCIe_BOM.csv"}}, "TS-USB4": {"bom": "board_TS-USB4_BOM.csv", "bom_exceptions": ["P1"], "source": {"kind": "design", "document": "board_TS-USB4_BOM.csv"}}}},
                "note": "--variant NAME on the command line chooses for one run; \"default\" = the board file as it is"}),
            items,
        ));
    } else if !vs.forks.is_empty() {
        out.push(gap(
            "assembly-variant-forks",
            "medium",
            "interfaces",
            format!("Variant '{}' still has {} fork(s) where all alternative series parts are fitted: the other paths stay in the lanes as open branches.", vs.chosen.clone().unwrap_or_default(), vs.forks.len()),
            "BOM / position files of the variant: which of the alternatives is fitted.",
            json!({"to": "variants.<name>.not_fitted (or not_fitted_sheets, bom)"}),
            vs.forks.iter().map(|f| json!({"fork": crate::variant::fork_text(f), "parts": f.parts, "towards": f.towards})).collect(),
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
                bad_parts.entry(part.refdes.clone()).or_insert_with(|| part_json(json!({"ref": part.refdes, "value": part.value, "lane": l.name}), b.footprints.iter().find(|f| f.reference == part.refdes)));
            }
        }
    }
    if !bad_parts.is_empty() {
        out.push(gap(
            "channel-part-values",
            "medium",
            "interfaces",
            format!("{} part(s) in signal channels have a value kisi cannot read (e.g. “DNP”, a part number): modelled as open / very large.", bad_parts.len()),
            "Schematic / BOM: give the Value field a plain value (22, 4k7, 100n) or mark the part as DNP in KiCad. This is a fix in KiCad, not in kisi.json.",
            json!({"to": "KiCad Value field (not kisi.json)"}),
            bad_parts.into_values().collect(),
        ));
    }

    // --- power ---------------------------------------------------------------------------
    let (rails, _) = crate::pi::rails_from_fields(b);
    let tree = crate::tree::tree_from_fields(b);
    let supplies = &supplies;
    let regs: HashSet<String> = tree.as_ref().map(|t| t.0.elements.iter().map(|e| e.refdes.clone()).collect()).unwrap_or_default();
    let short = |s: &str| s.rsplit('/').next().unwrap_or(s).to_string();
    let board_name = |n: &str| crate::pi::find_net(b, n).map(|i| b.net_name(i).to_string()).unwrap_or_else(|| n.to_string());
    let sourced: HashSet<String> = rails
        .iter()
        .filter(|r| !r.sources.is_empty())
        .map(|r| r.net.clone())
        .chain(tree.iter().flat_map(|t| t.0.elements.iter().map(|e| board_name(&e.to))))
        .collect();
    let no_source: Vec<Value> = supplies
        .iter()
        .filter(|(n, r)| r.voltage.is_some() && !sourced.contains(*n) && r.components.len() > 1)
        .map(|(n, r)| json!({"rail": n, "voltage": r.voltage, "components": r.components.len()}))
        .collect();
    if tree.is_none() {
        out.push(gap(
            "power-tree",
            "medium",
            "power",
            "No regulator chain (field PI_REG): no power budget, no regulator load check.".into(),
            "Regulator datasheets: topology (buck/LDO/…), efficiency at the expected load, rated output current; input and output rails from the schematic.",
            json!({"to": "fields[] (name PI_REG on the regulator)", "example": {"ref": "U3", "name": "PI_REG", "value": "+12V > +3V3 buck 90% 3A", "source": {"kind": "datasheet", "document": "regulator datasheet", "page": "…", "note": "efficiency at 1 A, typical"}}}),
            vec![],
        ));
    }
    // rails found by name, pin name or a field, without a voltage
    let no_voltage: Vec<Value> = rails_without_voltage(supplies).into_iter().filter(|v| !(v["dismissable"] == true && dis.applies("rails-without-voltage", v["rail"].as_str().unwrap_or("")))).collect();
    if !no_voltage.is_empty() {
        out.push(gap(
            "rails-without-voltage",
            "medium",
            "power",
            format!("{} supply rail(s) without a known voltage (found by their name, a pin name such as VBUS, VSYS, VBAT, VDD, or a field): kisi needs the voltage to put their loads and sources into the power tree, the budget and the IR drop. Found by name: confirm that each one is a supply.", no_voltage.len()),
            "Schematic: the regulator, load switch or connector that feeds the rail and its output voltage (regulator datasheet / voltage setting, USB VBUS 5 V, battery nominal voltage). A rail behind a ferrite, a 0 Ω link or a shunt takes the voltage of the rail before it.",
            json!({"to": "fields[] – the rail with its voltage as NAME@<V>V in a PI_REG entry", "example": {"ref": "U2", "name": "PI_REG", "value": "VBUS > VSYS@5V pass", "source": {"kind": "datasheet", "document": "PMIC datasheet", "note": "VSYS follows VBUS while USB is present"}},
                "note": "Write the voltage where the rail appears in PI_REG: as the output of the part that feeds it (`+5V > VSYS@5V buck 90%`) or as a supply input (`VBAT@3.7V input`). Behind a ferrite, 0 Ω link or shunt use a pass element (`+3V3 > IMU_VDD pass r=0.5`: the voltage carries over; items[].write suggests it when kisi finds such a part). Net names with spaces in double quotes (`\"/Power/VDD CORE\"@0.9V`). Without a power tree: PI_SOURCE `NAME=<V>V` on the source part. Nets that are not supplies: ignore."}),
            no_voltage,
        ));
    }
    if !no_source.is_empty() {
        out.push(gap(
            "rails-without-source",
            "medium",
            "power",
            format!("{} supply rail(s) have no source (PI_SOURCE on a connector/regulator output, or PI_REG): no IR drop for them.", no_source.len()),
            "Schematic / netlist: which connector or regulator feeds the rail (the pad nets of the regulator show input and output).",
            json!({"to": "fields[]", "example": {"ref": "J1", "name": "PI_REG", "value": "+5V input", "source": {"kind": "assumption", "note": "barrel jack J1 feeds +5V (schematic / netlist)"}},
                "note": "supply inputs: PI_REG '<rail> input' (counts in the power budget and as IR-drop source); regulator outputs: PI_REG on the regulator (add out=<part> when the output current leaves through an inductor or diode); PI_SOURCE only marks IR-drop source pads without a power tree"}),
            no_source,
        ));
    }
    // switching regulators without an efficiency: kisi's default (an estimate)
    let no_eff: Vec<Value> = tree
        .iter()
        .flat_map(|t| t.0.elements.iter())
        .filter(|e| crate::tree::is_switching(&e.kind) && e.eff.is_none())
        .map(|e| {
            let f = b.footprints.iter().find(|f| f.reference == e.refdes);
            part_json(json!({"ref": e.refdes, "id": if e.id.is_empty() { &e.refdes } else { &e.id }, "kind": e.kind, "from": e.from, "to": e.to}), f)
        })
        .collect();
    if !no_eff.is_empty() {
        out.push(gap(
            "regulator-efficiency",
            "low",
            "power",
            format!("{} switching regulator(s) have no efficiency in PI_REG: kisi assumes {:.0} % (an estimate, marked as such in the power tree).", no_eff.len(), crate::tree::DEFAULT_EFF * 100.0),
            "Regulator datasheet: efficiency at the expected load current and input voltage (often only a curve – read it there and say so in the note, kind datasheet with 'read from Figure …', or estimate).",
            json!({"to": "fields[] (PI_REG on the regulator: the efficiency after the type)", "example": {"ref": no_eff[0]["ref"], "name": "PI_REG", "value": "+5V > +3V3 buck 88% 600mA", "source": {"kind": "datasheet", "document": "regulator datasheet", "note": "efficiency at 50 mA, 5 V in, read from the efficiency curve"}}}),
            no_eff,
        ));
    }
    let loaded: HashSet<&str> = b.footprints.iter().filter(|f| f.prop("PI_LOAD").is_some()).map(|f| f.reference.as_str()).collect();
    let mut on_rails: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let rail_names: HashSet<&str> = supplies.keys().map(|n| n.as_str()).collect();
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
                part_json(json!({"ref": r, "value": f.map(|f| f.value.clone()), "rails": rails}), f)
            })
            .collect();
        out.push(gap(
            "ic-loads",
            "medium",
            "power",
            format!("{} IC(s) on supply rails have no load current (field PI_LOAD): their current is missing from IR drop and budget.", items.len()),
            "Device datasheets (look up the mpn when an item has one: the Value field is not always the part number): supply current per rail in the expected operating mode (state typical or maximum in the source note); FPGAs/SoCs: the vendor's power estimator.",
            json!({"to": "fields[] (name PI_LOAD)", "example": {"ref": items[0]["ref"], "name": "PI_LOAD", "value": "+3V3=120mA", "source": {"kind": "datasheet", "url": "…", "page": "…", "note": "typical, all channels active"}}}),
            items,
        ));
    }
    let mut caps = Vec::new();
    let mut unrated = Vec::new();
    let mut seen = HashSet::new();
    for pad in &b.pads {
        let Some(f) = b.footprints.get(pad.fp as usize) else { continue };
        if f.dnp || cd.role(&f.reference) != "capacitor" || !rail_names.contains(b.net_name(pad.net)) || !seen.insert(f.reference.clone()) {
            continue;
        }
        match crate::pdn::parse_cap(&f.value) {
            None => caps.push(part_json(json!({"ref": f.reference, "value": f.value, "rail": short(b.net_name(pad.net))}), Some(f))),
            // class II range without a rated voltage in the value or a PI_CAP field: DC-bias derating guessed
            Some(c) if c >= 1e-7 && f.prop("PI_CAP").is_none() && crate::pdn_ic::value_details(&f.value).0.is_none() => {
                unrated.push(part_json(json!({"ref": f.reference, "value": f.value, "rail": short(b.net_name(pad.net))}), Some(f)));
            }
            _ => {}
        }
    }
    if !unrated.is_empty() {
        out.push(gap(
            "cap-ratings",
            "low",
            "power",
            format!("{} decoupling capacitor(s) of 100 nF and more have no rated voltage or dielectric: the DC-bias derating in Z(f) uses an assumed rating.", unrated.len()),
            "BOM / capacitor datasheet: rated voltage, dielectric (X5R/X7R/C0G), ideally the capacitance at the rail voltage from the vendor's DC-bias curve, ESR and ESL.",
            json!({"to": "fields[] (name PI_CAP) or the KiCad Value field (e.g. 10u 6.3V X5R)", "example": {"ref": unrated[0]["ref"], "name": "PI_CAP", "value": "rated=6.3V X5R c@3.3V=4.1u esl=0.45n", "source": {"kind": "datasheet", "document": "capacitor datasheet / vendor DC-bias curve", "note": "capacitance at 3.3 V, 25 °C"}}}),
            unrated,
        ));
    }
    // ferrite beads on supply rails without an impedance rating: typical 220 Ω at 100 MHz assumed
    let mut beads = Vec::new();
    let mut seen_b = HashSet::new();
    for pad in &b.pads {
        let Some(f) = b.footprints.get(pad.fp as usize) else { continue };
        let pre: String = f.reference.chars().take_while(|c| c.is_ascii_alphabetic()).collect::<String>().to_ascii_uppercase();
        let up = f.value.to_ascii_uppercase();
        let bead = pre == "FB" || (pre == "L" && (up.contains("FERRITE") || up.contains("BEAD")));
        if f.dnp || !bead || !rail_names.contains(b.net_name(pad.net)) || f.prop("PI_SERIES").is_some() || crate::pdn_ic::bead_z100(&f.value).is_some() || !seen_b.insert(f.reference.clone()) {
            continue;
        }
        beads.push(part_json(json!({"ref": f.reference, "value": f.value, "rail": short(b.net_name(pad.net))}), Some(f)));
    }
    if !beads.is_empty() {
        out.push(gap(
            "bead-ratings",
            "low",
            "power",
            format!("{} ferrite bead(s) on supply rails have no impedance rating: the PDN per IC assumes a typical 220 Ω at 100 MHz and 0.1 Ω DC resistance.", beads.len()),
            "Bead datasheet: impedance at 100 MHz and DC resistance (or the part number in the Value field, e.g. 600R@100MHz or BLM18PG221SN1).",
            json!({"to": "fields[] (name PI_SERIES) or the KiCad Value field", "example": {"ref": beads[0]["ref"], "name": "PI_SERIES", "value": "z100=600 rdc=0.15", "source": {"kind": "datasheet", "document": "ferrite bead datasheet", "note": "impedance at 100 MHz, DC resistance"}}}),
            beads,
        ));
    }
    if !caps.is_empty() {
        out.push(gap(
            "cap-values",
            "low",
            "power",
            format!("{} capacitor(s) on supply rails have a value kisi cannot read: left out of Z(f).", caps.len()),
            "Schematic / BOM: plain capacitance in the Value field (100n, 4u7, 10µF/6.3V). A fix in KiCad, not in kisi.json.",
            json!({"to": "KiCad Value field (not kisi.json)"}),
            caps,
        ));
    }
    out.extend(thermal_gap(p));

    let rank = |v: &Value| match v["priority"].as_str() {
        Some("high") => 0,
        Some("medium") => 1,
        _ => 2,
    };
    out.sort_by_key(rank);
    let count = |pr: &str| out.iter().filter(|g| g["priority"] == pr).count();
    // per category: is there anything to check, and is it ready (no open high or medium gap)
    let targets = p.an.nets.iter().filter(|n| n.target.is_some()).count();
    let lanes: usize = buses.iter().map(|b| b.lanes.len()).sum();
    let applicable = |v: &str| -> (bool, String) {
        match v {
            "impedance" if targets == 0 => (false, "no net with a target impedance (net classes such as 50Z or 90Z_Diff, or net_classes in kisi.json)".into()),
            "impedance" => (true, format!("{targets} net(s) with a target impedance")),
            "channel" if lanes == 0 => (false, "no buses or lanes between devices".into()),
            "channel" => (true, format!("{lanes} lane(s) in {} bus(es)", buses.len())),
            _ if supplies.is_empty() => (false, "no supply rails".into()),
            _ => (true, format!("{} supply rail(s)", supplies.len())),
        }
    };
    let mut views = serde_json::Map::new();
    for v in VIEWS {
        let (ok, why) = applicable(v);
        let n = |pr: &str| out.iter().filter(|g| g["priority"] == pr && g["views"].as_array().is_some_and(|a| a.iter().any(|x| x == v))).count();
        let (h, m, l) = (n("high"), n("medium"), n("low"));
        let state = if !ok { "nothing to check" } else if h + m == 0 { "ready" } else { "open" };
        views.insert(v.to_string(), json!({"applicable": ok, "why": why, "state": state, "high": h, "medium": m, "low": l}));
    }
    json!({
        "board": p.name,
        "variant": p.variant.summary(),
        "config": p.config_summary(),
        "summary": {"high": count("high"), "medium": count("medium"), "low": count("low")},
        "views": views,
        "gaps": out,
        "dismissed": dis.report(),
        "how": "Fill the gaps in <board>.kisi.json next to the board (format: see `kisi config-template`), every value with a source; then run `kisi config-check <board>` and `kisi gaps <board>` again. Gaps marked 'not kisi.json' are fixes in KiCad."
    })
}

/// Gaps whose items can be dismissed in `<board>.kisi.json` (`gaps.dismissed`): items found only by
/// a name heuristic, which may be wrong (`"dismissable": true` on the item).
pub const DISMISSABLE: &[&str] = &["nets-without-target", "rails-without-voltage"];

/// Routed nets that look like interfaces by their name but have no target, per net class (all of
/// them name-based, so dismissable).
fn untargeted_nets(p: &Project, profiles: &Profiles, supplies: &BTreeMap<String, SupplyRail>) -> BTreeMap<String, Vec<Value>> {
    let b = &p.board;
    let supply: HashSet<String> = crate::pi::power_nets(b).into_iter().map(|n| n.name).chain(supplies.keys().cloned()).collect();
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
        untargeted.entry(st.class.clone()).or_default().push(json!({"net": st.name, "looks_like": pid, "length_mm": (st.length * 10.0).round() / 10.0, "dismissable": true}));
    }
    untargeted
}

/// Supply rails without a voltage; those found only by their name are dismissable.
fn rails_without_voltage(supplies: &BTreeMap<String, SupplyRail>) -> Vec<Value> {
    supplies
        .iter()
        .filter(|(_, r)| r.voltage.is_none())
        .map(|(n, r)| {
            let feed: Vec<Value> = r.fed_through.iter().map(|(part, from)| json!({"part": part, "from": from})).collect();
            let write = match r.fed_through.first() {
                Some((part, from)) => format!("PI_REG on {part}: {} > {} pass", crate::field_syntax::quote_name(from), crate::field_syntax::quote_name(n)),
                None => format!("{}@<V>V where a PI_REG entry names it (regulator output or supply input)", crate::field_syntax::quote_name(n)),
            };
            let by_name = !r.found_by.is_empty() && r.found_by.iter().all(|f| f == "name");
            let mut v = json!({"rail": n, "found_by": r.found_by, "components": r.components.len(), "ics": r.ics, "write": write});
            if !feed.is_empty() {
                v["fed_through"] = json!(feed);
            }
            if by_name {
                v["dismissable"] = json!(true);
            }
            v
        })
        .collect()
}

/// The dismissals of the configuration that `kisi gaps` applies: a known gap and a reason (the
/// check reports the others).
struct Dismissed<'a> {
    list: Vec<&'a crate::config::Dismissal>,
    used: std::cell::RefCell<HashSet<(String, String)>>,
}

impl<'a> Dismissed<'a> {
    fn from(cfg: &'a crate::config::BoardConfig) -> Self {
        let list = cfg.gaps.dismissed.iter().filter(|d| DISMISSABLE.contains(&d.gap.as_str()) && !d.reason.trim().is_empty()).collect();
        Dismissed { list, used: Default::default() }
    }
    /// Is `item` of gap `gap` dismissed? (remembers it for the report)
    fn applies(&self, gap: &str, item: &str) -> bool {
        let hit = self.list.iter().any(|d| d.gap == gap && d.items.iter().any(|i| i == item));
        if hit {
            self.used.borrow_mut().insert((gap.to_string(), item.to_string()));
        }
        hit
    }
    /// The dismissals with the items they left out (`items`) and the ones that did not apply.
    fn report(&self) -> Value {
        let used = self.used.borrow();
        json!(self
            .list
            .iter()
            .map(|d| {
                let (a, n): (Vec<&String>, Vec<&String>) = d.items.iter().partition(|i| used.contains(&(d.gap.clone(), (*i).clone())));
                json!({"gap": d.gap, "reason": d.reason, "items": a, "not_applied": n})
            })
            .collect::<Vec<_>>())
    }
}

/// Dismissals that cannot apply: (errors, warnings). An item that this gap reports but not as a
/// name-based guess is an error (only guesses can be dismissed); an item the gap no longer reports
/// is a warning (the dismissal is no longer needed). Part of the configuration check.
pub fn dismissal_problems(p: &Project) -> (Vec<String>, Vec<String>) {
    let (mut errors, mut warnings) = (vec![], vec![]);
    let Some(cfg) = &p.config else { return (errors, warnings) };
    if cfg.gaps.dismissed.is_empty() {
        return (errors, warnings);
    }
    let supplies = supply_rails(&p.board, &ComponentData::builtin());
    let nets: HashSet<String> = untargeted_nets(p, &Profiles::builtin(), &supplies).into_values().flatten().filter_map(|n| n["net"].as_str().map(str::to_string)).collect();
    let rails: Vec<Value> = rails_without_voltage(&supplies);
    for d in cfg.gaps.dismissed.iter().filter(|d| DISMISSABLE.contains(&d.gap.as_str())) {
        let what = format!("gaps.dismissed[{}]", d.gap);
        for it in &d.items {
            match d.gap.as_str() {
                "nets-without-target" if !nets.contains(it) => warnings.push(format!("{what}: '{it}' is no longer reported (dismissal not needed)")),
                "rails-without-voltage" => match rails.iter().find(|r| r["rail"] == it.as_str()) {
                    None => warnings.push(format!("{what}: '{it}' is no longer reported (dismissal not needed)")),
                    Some(r) if r["dismissable"] != true => errors.push(format!("{what}: '{it}' was found by a pin name or a field, not only by its name – it cannot be dismissed (give its voltage)")),
                    _ => {}
                },
                _ => {}
            }
        }
    }
    (errors, warnings)
}

/// `kisi gaps --for <view>`: only the gaps of one category, with its summary.
pub fn only_view(v: &mut Value, view: &str) {
    if let Some(gs) = v["gaps"].as_array_mut() {
        gs.retain(|g| g["views"].as_array().is_some_and(|a| a.iter().any(|x| x == view)));
    }
    let count = |pr: &str| v["gaps"].as_array().map(|a| a.iter().filter(|g| g["priority"] == pr).count()).unwrap_or(0);
    v["summary"] = json!({"high": count("high"), "medium": count("medium"), "low": count("low")});
    let one = v["views"].get(view).cloned().unwrap_or(Value::Null);
    v["views"] = json!({ view: one });
    if let Some(ds) = v["dismissed"].as_array_mut() {
        ds.retain(|d| gap_views(d["gap"].as_str().unwrap_or("")).1.contains(&view));
    }
    v["for"] = json!(view);
}

/// Parts that dissipate 0.1 W or more (power tree: load power, regulator and pass losses) without a
/// PI_THERMAL field: `kisi thermal` gives them package defaults (estimates).
fn thermal_gap(p: &Project) -> Option<Value> {
    let b = &p.board;
    let (spec, _) = crate::tree::tree_from_fields(b)?;
    let t = crate::tree::solve_tree(&spec, Some(b)).ok()?;
    let mut items: Vec<(f64, Value)> = t
        .components
        .iter()
        .filter(|c| c.consumption + c.loss >= 0.1)
        .filter_map(|c| {
            let fi = b.footprints.iter().position(|f| f.reference == c.refdes)?;
            let f = &b.footprints[fi];
            if f.dnp || f.prop("PI_THERMAL").is_some() {
                return None;
            }
            let w = c.consumption + c.loss;
            let fam = crate::thermal::package_defaults(b, fi).0;
            Some((w, json!({"ref": c.refdes, "value": f.value, "heat_w": (w * 1e3).round() / 1e3, "package_default": fam})))
        })
        .collect();
    if items.is_empty() {
        return None;
    }
    items.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| a.1["ref"].as_str().cmp(&b.1["ref"].as_str())));
    let items: Vec<Value> = items.into_iter().map(|x| x.1).collect();
    Some(gap(
        "thermal-data",
        "low",
        "power",
        format!("{} part(s) dissipate 0.1 W or more without thermal data (field PI_THERMAL): `kisi thermal` estimates their junction temperature from package defaults and assumes Tj max 125 °C.", items.len()),
        "Device datasheet: the Thermal Information table (RθJB, RθJC(top) – or ψJB – for the JEDEC 2s2p board) and the maximum junction temperature (Absolute Maximum Ratings / Recommended Operating Conditions). Heat that leaves the board (connectors, LEDs, RF output): p=<W>.",
        json!({"to": "fields[] (name PI_THERMAL)", "example": {"ref": items[0]["ref"], "name": "PI_THERMAL", "value": "theta_jb=17.4 theta_jc=53.6 tj_max=125", "source": {"kind": "datasheet", "document": "…", "page": "Thermal Information", "note": "RθJB, RθJC(top), JEDEC 2s2p board"}}}),
        items,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::Inputs;

    /// A small PMIC board: VSYS and VBAT have no voltage in their names; VSYS is fed from +5V
    /// through a 0 Ω link, VBAT has a battery pad (a test point without a part). The VBUSOUT node
    /// (the PMIC and its capacitor only) and the VBUS_DET signal are not rails.
    const PCB: &str = r#"(kicad_pcb (version 20241229) (generator "pcbnew")
  (layers (0 "F.Cu" signal) (2 "B.Cu" signal))
  (net 0 "") (net 1 "+5V") (net 2 "VSYS") (net 3 "VBAT") (net 4 "Net-(U1-VBUSOUT)") (net 5 "VBUS_DET") (net 6 "GND") (net 7 "+3V3")
  (footprint "J" (layer "F.Cu") (at 0 0) (property "Reference" "J1") (property "Value" "USB-C") PROPJ
    (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net 1 "+5V") (pinfunction "VBUS") (pintype "passive"))
    (pad "2" smd rect (at 0 2) (size 1 1) (layers "F.Cu") (net 6 "GND")))
  (footprint "R" (layer "F.Cu") (at 4 0) (property "Reference" "R1") (property "Value" "0R") PROPR
    (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net 1 "+5V"))
    (pad "2" smd rect (at 1 0) (size 1 1) (layers "F.Cu") (net 2 "VSYS")))
  (footprint "U" (layer "F.Cu") (at 10 0) (property "Reference" "U1") (property "Value" "英联") (property "MPN" "UM3204H") (property "Manufacturer" "英联")
    (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net 2 "VSYS") (pinfunction "VSYS") (pintype "power_in"))
    (pad "2" smd rect (at 0 1) (size 1 1) (layers "F.Cu") (net 3 "VBAT") (pinfunction "VBAT") (pintype "power_in"))
    (pad "3" smd rect (at 0 2) (size 1 1) (layers "F.Cu") (net 4 "Net-(U1-VBUSOUT)") (pinfunction "VBUSOUT") (pintype "power_out"))
    (pad "4" smd rect (at 0 3) (size 1 1) (layers "F.Cu") (net 5 "VBUS_DET") (pinfunction "VBUS_DET") (pintype "input"))
    (pad "5" smd rect (at 0 4) (size 1 1) (layers "F.Cu") (net 6 "GND")))
  (footprint "U" (layer "F.Cu") (at 20 0) (property "Reference" "U3") (property "Value" "TPS62843") PROPU3
    (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net 2 "VSYS"))
    (pad "2" smd rect (at 0 1) (size 1 1) (layers "F.Cu") (net 7 "+3V3"))
    (pad "3" smd rect (at 0 2) (size 1 1) (layers "F.Cu") (net 6 "GND")))
  (footprint "TP" (layer "B.Cu") (at 12 6) (property "Reference" "TP1") (property "Value" "BAT") (attr smd exclude_from_bom dnp)
    (pad "1" smd circle (at 0 0) (size 1 1) (layers "B.Cu") (net 3 "VBAT")))
  (footprint "C" (layer "F.Cu") (at 12 3) (property "Reference" "C1") (property "Value" "1u")
    (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net 4 "Net-(U1-VBUSOUT)"))
    (pad "2" smd rect (at 0 1) (size 1 1) (layers "F.Cu") (net 6 "GND")))
  (footprint "R" (layer "F.Cu") (at 14 3) (property "Reference" "R2") (property "Value" "100k")
    (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net 5 "VBUS_DET"))
    (pad "2" smd rect (at 0 1) (size 1 1) (layers "F.Cu") (net 1 "+5V"))))"#;

    pub(super) fn pcb_plain() -> String {
        PCB.replace("PROPJ", "").replace("PROPR", "").replace("PROPU3", "")
    }
    fn run(j: &str, r: &str, u3: &str) -> Value {
        let pcb = PCB.replace("PROPJ", j).replace("PROPR", r).replace("PROPU3", u3);
        let p = Project::from_texts(Inputs { pcb_path: "pmic.kicad_pcb".into(), pcb, pro: None, config: None, stackup: None, field: HashMap::new(), variant: None, files: Default::default() }).unwrap();
        gaps(&p, &Profiles::builtin())
    }
    fn gap_of<'a>(v: &'a Value, id: &str) -> Option<&'a Value> {
        v["gaps"].as_array().unwrap().iter().find(|g| g["id"] == id)
    }

    #[test]
    fn rails_without_voltage_and_mpn() {
        let v = run("", "", "");
        let g = gap_of(&v, "rails-without-voltage").expect("gap");
        let rails: Vec<&str> = g["items"].as_array().unwrap().iter().map(|i| i["rail"].as_str().unwrap()).collect();
        assert_eq!(rails, vec!["VBAT", "VSYS"], "{g:#}");
        let vsys = &g["items"][1];
        assert_eq!(vsys["fed_through"][0]["part"], "R1");
        assert_eq!(vsys["write"], "PI_REG on R1: +5V > VSYS pass");
        // the ICs on the new rails are loads without a current; the MPN where the value is none
        let ic = gap_of(&v, "ic-loads").unwrap();
        let u1 = ic["items"].as_array().unwrap().iter().find(|i| i["ref"] == "U1").unwrap();
        assert_eq!((u1["mpn"].as_str(), u1["manufacturer"].as_str()), (Some("UM3204H"), Some("英联")));
        assert_eq!(u1["rails"], json!(["VBAT", "VSYS"]));
        assert!(gap_of(&v, "regulator-efficiency").is_none());

        // the voltage from PI_REG: VSYS behind the link takes +5V's; a buck without efficiency
        let v = run(r#"(property "PI_REG" "+5V input")"#, r#"(property "PI_REG" "+5V > VSYS pass")"#, r#"(property "PI_REG" "VSYS > +3V3 buck 600mA")"#);
        let rails: Vec<&str> = gap_of(&v, "rails-without-voltage").unwrap()["items"].as_array().unwrap().iter().map(|i| i["rail"].as_str().unwrap()).collect();
        assert_eq!(rails, vec!["VBAT"]);
        let eff = gap_of(&v, "regulator-efficiency").expect("efficiency gap");
        assert_eq!(eff["items"][0]["ref"], "U3");
        // with the battery as an input: no rail without a voltage left
        let v = run(r#"(property "PI_REG" "+5V input")"#, r#"(property "PI_REG" "+5V > VSYS pass")"#, r#"(property "PI_REG" "VSYS > +3V3 buck 90% 600mA")"#);
        let v2 = run(r#"(property "PI_REG" "+5V input; VBAT@3.7V input")"#, r#"(property "PI_REG" "+5V > VSYS pass")"#, r#"(property "PI_REG" "VSYS > +3V3 buck 90% 600mA")"#);
        assert!(gap_of(&v, "regulator-efficiency").is_none());
        assert!(gap_of(&v2, "rails-without-voltage").is_none(), "{:#}", v2["gaps"]);
    }

    /// Every word of GAP_AFFECTS has a category, and every gap id in this file has an entry.
    #[test]
    fn affects_words_map() {
        for (id, words) in GAP_AFFECTS {
            for w in *words {
                assert!(AFFECT_VIEWS.iter().any(|(x, _)| x == w), "gap {id}: '{w}' has no category in AFFECT_VIEWS");
            }
            assert!(!gap_views(id).1.is_empty(), "gap {id}: no category");
        }
        let src = include_str!("gaps.rs");
        for (i, _) in src.match_indices("gap(\n") {
            let id = src[i + 4..].trim_start().trim_start_matches('"').split('"').next().unwrap();
            assert!(GAP_AFFECTS.iter().any(|g| g.0 == id), "gap '{id}' missing in GAP_AFFECTS");
        }
        assert_eq!(gap_views("stackup-defaults").1, vec!["impedance", "channel"]);
        assert_eq!(gap_views("assembly-variant").1, vec!["channel", "power"]);
        assert_eq!(gap_views("thermal-data").1, vec!["power"]);
    }

    /// Categories: nothing to check without targets and lanes; `--for` keeps one category.
    #[test]
    fn views_and_for() {
        let mut v = run("", "", "");
        assert_eq!(v["views"]["impedance"]["state"], "nothing to check");
        assert_eq!(v["views"]["channel"]["applicable"], false);
        assert_eq!((v["views"]["power"]["state"].as_str(), v["views"]["power"]["applicable"].as_bool()), (Some("open"), Some(true)));
        assert_eq!(gap_of(&v, "rails-without-voltage").unwrap()["views"], json!(["power"]));
        only_view(&mut v, "power");
        assert!(v["gaps"].as_array().unwrap().iter().all(|g| g["views"].as_array().unwrap().iter().any(|x| x == "power")));
        assert_eq!(v["views"].as_object().unwrap().len(), 1);
        assert_eq!(v["for"], "power");
    }

    fn with_config(cfg: &str) -> Project {
        Project::from_texts(Inputs { pcb_path: "pmic.kicad_pcb".into(), pcb: pcb_plain(), pro: None, config: Some(("pmic.kisi.json".into(), Ok(cfg.to_string()))), stackup: None, field: HashMap::new(), variant: None, files: Default::default() }).unwrap()
    }

    /// Dismissals: only name-based gaps, with a reason; `gaps` leaves the items out and lists them.
    #[test]
    fn dismissals() {
        let p = with_config(r#"{"kisi_config": 1, "gaps": {"dismissed": [{"gap": "rails-without-voltage", "items": ["VBAT"], "reason": "battery case not modelled"}]}}"#);
        assert!(p.config_check.as_ref().unwrap().ok, "{:?}", p.config_check);
        let v = gaps(&p, &Profiles::builtin());
        let rails: Vec<&str> = gap_of(&v, "rails-without-voltage").unwrap()["items"].as_array().unwrap().iter().map(|i| i["rail"].as_str().unwrap()).collect();
        assert_eq!(rails, vec!["VSYS"]);
        assert_eq!(v["dismissed"][0]["items"], json!(["VBAT"]));
        assert_eq!(v["dismissed"][0]["reason"], "battery case not modelled");
        // both rails dismissed: the gap is closed
        let p = with_config(r#"{"kisi_config": 1, "gaps": {"dismissed": [{"gap": "rails-without-voltage", "items": ["VBAT", "VSYS"], "reason": "not supplies in this case"}]}}"#);
        assert!(gap_of(&gaps(&p, &Profiles::builtin()), "rails-without-voltage").is_none());
        // no reason, a gap that cannot be dismissed, an unknown net, an item no longer reported
        let p = with_config(r#"{"kisi_config": 1, "gaps": {"dismissed": [{"gap": "rails-without-voltage", "items": ["VBAT"]},
            {"gap": "power-tree", "items": ["+5V"], "reason": "x"}, {"gap": "rails-without-voltage", "items": ["NOPE", "+5V"], "reason": "y"}]}}"#);
        let c = p.config_check.as_ref().unwrap();
        assert!(!c.ok);
        for want in ["reason missing", "gap 'power-tree' cannot be dismissed", "no net 'NOPE'"] {
            assert!(c.errors.iter().any(|e| e.contains(want)), "{want}: {:?}", c.errors);
        }
        assert!(c.warnings.iter().any(|w| w.contains("'+5V' is no longer reported")), "{:?}", c.warnings);
        // without a reason the dismissal does not apply
        assert_eq!(gap_of(&gaps(&p, &Profiles::builtin()), "rails-without-voltage").unwrap()["items"].as_array().unwrap().len(), 2);
    }

    /// One category of a configuration ("save only this category").
    #[test]
    fn subset_per_view() {
        let cfg = crate::config::parse(r#"{"kisi_config": 1, "board": "b.kicad_pcb",
            "stackup": {"dielectrics": [{"index": 0, "epsilon_r": 4.2, "source": {"kind": "estimate", "note": "x"}}]},
            "net_classes": {"A": {"target_ohm": 50, "diff": false, "profile": "se50", "source": {"kind": "estimate", "note": "x"}}, "B": {"profile": "lvds", "source": {"kind": "estimate", "note": "x"}}},
            "fields": [{"ref": "U1", "name": "PI_LOAD", "value": "+3V3=1mA", "source": {"kind": "estimate", "note": "x"}}],
            "variant": "default",
            "gaps": {"dismissed": [{"gap": "nets-without-target", "items": ["X"], "reason": "r"}]}}"#).unwrap();
        let i = crate::config::subset(&cfg, "impedance");
        assert_eq!((i.stackup.dielectrics.len(), i.net_classes.len(), i.net_classes["A"].profile.clone(), i.fields.len(), i.variant.clone(), i.gaps.dismissed.len()), (1, 1, None, 0, None, 1));
        let c = crate::config::subset(&cfg, "channel");
        assert_eq!((c.net_classes.len(), c.net_classes["B"].profile.as_deref(), c.variant.as_deref(), c.fields.len()), (2, Some("lvds"), Some("default"), 0));
        let w = crate::config::subset(&cfg, "power");
        assert_eq!((w.stackup.dielectrics.len(), w.net_classes.len(), w.fields.len(), w.variant.as_deref(), w.gaps.dismissed.len()), (0, 0, 1, Some("default"), 0));
        assert_eq!(crate::config::message_views(&cfg, "net_classes.B: source missing"), vec!["channel"]);
        assert_eq!(crate::config::message_views(&cfg, "U1: PI_LOAD value 'x' not readable"), vec!["power"]);
    }
}

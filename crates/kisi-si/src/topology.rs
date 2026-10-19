//! Device-to-device channels (milestone M7).
//!
//! A signal often runs through several nets: from an IC pin, over a series
//! resistor or an AC-coupling capacitor, to the pin of another IC or a connector.
//! This module follows nets through two-pin passives, collects terminations
//! (passives to ground/supply or across a pair) and the end pins, and groups the
//! resulting lanes into buses (same pair of devices). Parasitics of the passives
//! come from `data/components.json`.

use crate::board::Board;
use crate::extract::{diff_partners, Analysis};
use crate::geom::{self, P};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};

pub const COMPONENTS_JSON: &str = include_str!("../../../data/components.json");

#[derive(Debug, Clone, Deserialize)]
pub struct PackageData {
    pub esl_nh: f64,
    pub pad_c_pf: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ComponentData {
    pub packages: BTreeMap<String, PackageData>,
    pub capacitor: CapData,
    pub roles: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CapData {
    pub esr_ohm: f64,
}

impl ComponentData {
    pub fn builtin() -> Self {
        serde_json::from_str(COMPONENTS_JSON).expect("data/components.json")
    }
    /// Role of a reference designator by its letter prefix (longest match followed by a digit).
    pub fn role(&self, refdes: &str) -> &str {
        let letters: String = refdes.chars().take_while(|c| c.is_ascii_alphabetic()).collect::<String>().to_uppercase();
        if letters.is_empty() || !refdes[letters.len()..].starts_with(|c: char| c.is_ascii_digit()) {
            return "other";
        }
        let mut best: Option<(&str, usize)> = None;
        for (role, prefixes) in &self.roles {
            for p in prefixes {
                if letters == p.to_uppercase() && best.map_or(true, |b| p.len() > b.1) {
                    best = Some((role.as_str(), p.len()));
                }
            }
        }
        best.map(|b| b.0).unwrap_or("other")
    }
    pub fn package(&self, pkg: &str) -> &PackageData {
        self.packages.get(pkg).or_else(|| self.packages.get("large")).expect("package table")
    }
}

/// Case size from the pad pitch (centre to centre, mm).
pub fn package_of(pitch: f64) -> &'static str {
    match pitch {
        p if p < 0.65 => "0201",
        p if p < 1.25 => "0402",
        p if p < 1.95 => "0603",
        p if p < 2.7 => "0805",
        p if p < 3.8 => "1206",
        _ => "large",
    }
}

/// Parse a component value: "0", "22", "4k7", "49.9R", "1M", "220nF", "100n", "4u7", "10µF/6.3V".
pub fn parse_value(v: &str) -> Option<f64> {
    let s = v.trim().replace(',', ".").replace(['µ', 'μ'], "u");
    let tok = s.split(|c: char| c.is_whitespace() || c == '/' || c == '_').next()?;
    let t = tok.trim_end_matches(['F', 'f', 'H', 'h', 'Ω']).trim_end_matches("Ohm").trim_end_matches("ohm");
    let mult = |c: char| match c {
        'p' | 'P' => Some(1e-12),
        'n' | 'N' => Some(1e-9),
        'u' | 'U' => Some(1e-6),
        'm' => Some(1e-3),
        'R' | 'r' => Some(1.0),
        'k' | 'K' => Some(1e3),
        'M' => Some(1e6),
        _ => None,
    };
    if t.is_empty() {
        return None;
    }
    if let Some(pos) = t.find(|c: char| mult(c).is_some()) {
        let (a, b) = t.split_at(pos);
        let m = mult(b.chars().next()?)?;
        let rest = &b[1..];
        if !a.chars().all(|c| c.is_ascii_digit() || c == '.') || !rest.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        let a = if a.is_empty() { "0" } else { a };
        let num: f64 = if rest.is_empty() { a.parse().ok()? } else { format!("{a}.{rest}").parse().ok()? };
        return Some(num * m);
    }
    t.parse().ok()
}

#[derive(Debug, Clone, Serialize)]
pub struct EndPin {
    pub refdes: String,
    pub pin: String,
    pub value: String,
    /// "ic" or "connector"
    pub role: String,
    pub net: u32,
    pub at: P,
}

/// A two-pin passive in or at the channel.
#[derive(Debug, Clone, Serialize)]
pub struct Passive {
    pub refdes: String,
    pub value: String,
    /// 'R', 'C' or 'L'
    pub kind: char,
    /// parsed value in Ω, F or H (None: unreadable, e.g. "DNP")
    pub val: Option<f64>,
    pub package: String,
    pub esl_nh: f64,
    pub pad_c_pf: f64,
    /// pad positions (in the order of the nets they connect)
    pub pads: [P; 2],
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Hop {
    /// a piece of copper of one net, from point to point
    Net { net: u32, from: P, to: P },
    /// a part in series between two nets
    Series { part: Passive },
}

/// Termination: a passive from the line to ground / a supply (`to` = that net),
/// or across a differential pair (`across` = true; the odd-mode model uses R/2 per line).
#[derive(Debug, Clone, Serialize)]
pub struct Shunt {
    pub part: Passive,
    pub net: u32,
    pub to: u32,
    pub across: bool,
    pub at: P,
}

#[derive(Debug, Clone, Serialize)]
pub struct Lane {
    /// name of the (positive) line, e.g. the first net with a target
    pub name: String,
    /// nets of the line in order from `ends[0]` to `ends[1]`
    pub nets: Vec<u32>,
    /// nets of the partner line (differential pairs), same order
    pub partner_nets: Vec<u32>,
    pub diff: bool,
    pub hops: Vec<Hop>,
    /// hops and end pins of the partner line (differential pairs), for skew
    pub partner_hops: Vec<Hop>,
    pub partner_ends: Vec<EndPin>,
    /// open branches hanging off the line at its first end (the rest of a
    /// multi-drop line seen from a device in the middle), each from the junction
    pub branches: Vec<Vec<Hop>>,
    pub shunts: Vec<Shunt>,
    /// two ends if the lane is point to point; more for multi-drop, fewer if open
    pub ends: Vec<EndPin>,
    /// "ic-ic", "ic-connector", "connector-connector", "multi-drop", "open"
    pub kind: String,
    /// index into `ends` of the receiving end (guess, see `direction`)
    pub rx_end: usize,
    /// how the direction was found
    pub direction: String,
    /// other parts touching the line (ESD diodes, test points, …): refdes
    pub others: Vec<String>,
    pub class: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Bus {
    pub name: String,
    /// device reference designators at the ends (sorted)
    pub devices: Vec<String>,
    pub values: Vec<String>,
    pub kind: String,
    pub class: String,
    pub lanes: Vec<Lane>,
}

fn is_supply(board: &Board, net: u32) -> bool {
    if board.plane_net.get(net as usize).copied().unwrap_or(false) {
        return true;
    }
    let n = board.net_name(net).rsplit('/').next().unwrap_or("").to_uppercase();
    n.is_empty()
        || n.contains("GND")
        || n.starts_with("VCC")
        || n.starts_with("VDD")
        || n.starts_with("VSS")
        || n.starts_with("VBUS")
        || n.starts_with("+")
        || n.starts_with("-")
        || (n.len() <= 6 && n.chars().next().is_some_and(|c| c.is_ascii_digit()) && n.contains('V'))
}

struct PartInfo<'a> {
    fp: u32,
    refdes: &'a str,
    role: String,
    /// (pad index, net)
    pads: Vec<(usize, u32)>,
}

/// All device-to-device lanes of nets that have a target impedance (or a diff
/// partner), grouped into buses.
pub fn buses(board: &Board, an: &Analysis, cd: &ComponentData) -> Vec<Bus> {
    let partners = diff_partners(&board.nets);
    let target_net: HashSet<u32> = an.nets.iter().filter(|n| n.target.is_some()).map(|n| n.net).collect();
    // parts and their pads on nets
    let mut parts: Vec<PartInfo> = board
        .footprints
        .iter()
        .enumerate()
        .map(|(i, f)| PartInfo { fp: i as u32, refdes: f.reference.as_str(), role: cd.role(&f.reference).to_string(), pads: vec![] })
        .collect();
    for (pi, p) in board.pads.iter().enumerate() {
        if p.net == 0 || (p.fp as usize) >= parts.len() {
            continue;
        }
        parts[p.fp as usize].pads.push((pi, p.net));
    }
    let two_pin = |pt: &PartInfo| -> Option<((usize, u32), (usize, u32))> {
        let passive = matches!(pt.role.as_str(), "resistor" | "capacitor" | "inductor");
        if !passive || pt.pads.len() != 2 || pt.pads[0].1 == pt.pads[1].1 {
            return None;
        }
        Some((pt.pads[0], pt.pads[1]))
    };
    let mk_passive = |pt: &PartInfo, a: usize, b: usize| -> Passive {
        let f = &board.footprints[pt.fp as usize];
        let (pa, pb) = (board.pads[a].at, board.pads[b].at);
        let pkg = package_of(geom::len(geom::sub(pa, pb)));
        let pd = cd.package(pkg);
        let kind = match pt.role.as_str() {
            "capacitor" => 'C',
            "inductor" => 'L',
            _ => 'R',
        };
        Passive { refdes: f.reference.clone(), value: f.value.clone(), kind, val: parse_value(&f.value), package: pkg.to_string(), esl_nh: pd.esl_nh, pad_c_pf: pd.pad_c_pf, pads: [pa, pb] }
    };
    // series links between signal nets, shunts and other attachments
    let mut links: HashMap<u32, Vec<(u32, usize, usize, usize)>> = HashMap::new(); // net -> (other net, part, pad here, pad there)
    let mut shunts: HashMap<u32, Vec<Shunt>> = HashMap::new();
    let mut ends: HashMap<u32, Vec<EndPin>> = HashMap::new();
    let mut others: HashMap<u32, Vec<String>> = HashMap::new();
    for (k, pt) in parts.iter().enumerate() {
        if board.footprints[pt.fp as usize].dnp {
            continue; // not populated
        }
        if let Some(((pa, na), (pb, nb))) = two_pin(pt) {
            let (sa, sb) = (is_supply(board, na), is_supply(board, nb));
            let f = &board.footprints[pt.fp as usize];
            let val = parse_value(&f.value);
            if val.is_none() || f.dnp {
                // DNP / NC / unreadable: not mounted or unknown, ignore
                continue;
            }
            // only parts that pass a signal on: small series resistors (0–100 Ω, damping /
            // links), AC-coupling capacitors (≥ 1 nF), small inductors; larger resistors
            // and small capacitors are gain, filter or bias networks and end the channel
            let passes = match pt.role.as_str() {
                "resistor" => val.unwrap() <= 100.0,
                "capacitor" => val.unwrap() >= 1e-9,
                _ => val.unwrap() <= 100e-9,
            };
            if !sa && !sb && !passes && partners.get(na as usize).copied().flatten() != Some(nb) {
                others.entry(na).or_default().push(pt.refdes.to_string());
                others.entry(nb).or_default().push(pt.refdes.to_string());
                continue;
            }
            if !sa && !sb {
                if partners.get(na as usize).copied().flatten() == Some(nb) {
                    // across a differential pair
                    let part = mk_passive(pt, pa, pb);
                    shunts.entry(na).or_default().push(Shunt { part: part.clone(), net: na, to: nb, across: true, at: board.pads[pa].at });
                    let mut p2 = part;
                    p2.pads.swap(0, 1);
                    shunts.entry(nb).or_default().push(Shunt { part: p2, net: nb, to: na, across: true, at: board.pads[pb].at });
                } else {
                    links.entry(na).or_default().push((nb, k, pa, pb));
                    links.entry(nb).or_default().push((na, k, pb, pa));
                }
            } else if sa != sb {
                let (sig, pad, sup, other_pad) = if sa { (nb, pb, na, pa) } else { (na, pa, nb, pb) };
                shunts.entry(sig).or_default().push(Shunt { part: mk_passive(pt, pad, other_pad), net: sig, to: sup, across: false, at: board.pads[pad].at });
            }
            continue;
        }
        for &(pi, net) in &pt.pads {
            if is_supply(board, net) {
                continue;
            }
            match pt.role.as_str() {
                "ic" | "connector" => {
                    let f = &board.footprints[pt.fp as usize];
                    ends.entry(net).or_default().push(EndPin { refdes: f.reference.clone(), pin: board.pads[pi].number.clone(), value: f.value.clone(), role: pt.role.clone(), net, at: board.pads[pi].at });
                }
                _ => others.entry(net).or_default().push(pt.refdes.to_string()),
            }
        }
    }
    // chains: connected components of signal nets over series links, starting from
    // nets with a target and their partners
    struct Chain {
        order: Vec<u32>,
        hops: Vec<Hop>,
        ends: Vec<EndPin>,
        kind: &'static str,
        has_target: bool,
    }
    let mut seen: HashSet<u32> = HashSet::new();
    let mut chains: Vec<Chain> = Vec::new();
    let mut starts: Vec<u32> = target_net.iter().copied().chain(target_net.iter().filter_map(|&n| partners.get(n as usize).copied().flatten())).collect();
    starts.sort_unstable();
    starts.dedup();
    for &n0 in &starts {
        if seen.contains(&n0) || is_supply(board, n0) {
            continue;
        }
        let mut comp = vec![n0];
        let mut st = vec![n0];
        seen.insert(n0);
        while let Some(u) = st.pop() {
            for &(v, ..) in links.get(&u).map(|x| x.as_slice()).unwrap_or(&[]) {
                if seen.insert(v) {
                    comp.push(v);
                    st.push(v);
                }
            }
        }
        let comp_set: HashSet<u32> = comp.iter().copied().collect();
        let mut e: Vec<EndPin> = comp.iter().flat_map(|n| ends.get(n).cloned().unwrap_or_default()).collect();
        e.sort_by(|a, b| a.refdes.cmp(&b.refdes).then(natural_cmp(&a.pin, &b.pin)));
        // order the nets along the chain, starting at an end net with at most one link
        let degree = |n: u32| links.get(&n).map(|x| x.iter().filter(|l| comp_set.contains(&l.0)).count()).unwrap_or(0);
        let first = e
            .iter()
            .map(|p| p.net)
            .find(|&n| degree(n) <= 1)
            .or_else(|| comp.iter().copied().filter(|&n| degree(n) <= 1).min())
            .unwrap_or(n0);
        let mut order = vec![first];
        let mut hops: Vec<Hop> = Vec::new();
        let mut used_parts: HashSet<usize> = HashSet::new();
        let mut cur = first;
        let mut entry: P = e.iter().find(|p| p.net == first).map(|p| p.at).unwrap_or([0.0, 0.0]);
        let mut simple = true;
        loop {
            let next: Vec<&(u32, usize, usize, usize)> =
                links.get(&cur).map(|x| x.iter().filter(|l| comp_set.contains(&l.0) && !used_parts.contains(&l.1)).collect()).unwrap_or_default();
            if next.len() > 1 {
                simple = false;
            }
            let Some(&&(v, k, pa, pb)) = next.first() else { break };
            hops.push(Hop::Net { net: cur, from: entry, to: board.pads[pa].at });
            hops.push(Hop::Series { part: mk_passive(&parts[k], pa, pb) });
            used_parts.insert(k);
            entry = board.pads[pb].at;
            if order.contains(&v) {
                simple = false;
                break;
            }
            cur = v;
            order.push(v);
        }
        let start_pin = e.iter().find(|p| p.net == first).cloned();
        let last_end = e
            .iter()
            .rev()
            .find(|p| p.net == cur && start_pin.as_ref().map_or(true, |s| s.refdes != p.refdes || s.pin != p.pin))
            .map(|p| p.at)
            .unwrap_or(entry);
        hops.push(Hop::Net { net: cur, from: entry, to: last_end });
        if order.len() != comp.len() {
            simple = false;
        }
        e.sort_by_key(|p| order.iter().position(|&n| n == p.net).unwrap_or(usize::MAX));
        let devices: HashSet<&str> = e.iter().map(|p| p.refdes.as_str()).collect();
        let kind = if !simple || e.len() > 2 {
            "multi-drop"
        } else if e.len() < 2 || devices.len() < 2 {
            "open"
        } else {
            match (e[0].role.as_str(), e[1].role.as_str()) {
                ("ic", "ic") => "ic-ic",
                ("connector", "connector") => "connector-connector",
                _ => "ic-connector",
            }
        };
        let has_target = order.iter().any(|n| target_net.contains(n));
        chains.push(Chain { order, hops, ends: e, kind, has_target });
    }
    // pair chains of differential lines: a chain whose nets are partners of another
    // chain's nets (names ending in P/N or +/-, also inside "Net-(…+)" names)
    let pmap = pair_map(&board.nets);
    let chain_of: HashMap<u32, usize> = chains.iter().enumerate().flat_map(|(i, c)| c.order.iter().map(move |&n| (n, i))).collect();
    let mut consumed = vec![false; chains.len()];
    let mut lanes: Vec<Lane> = Vec::new();
    for i in 0..chains.len() {
        if consumed[i] {
            continue;
        }
        let j = chains[i].order.iter().filter_map(|&n| pmap.get(&n)).filter_map(|p| chain_of.get(p)).copied().find(|&j| j != i && !consumed[j]);
        consumed[i] = true;
        let (pi, partner) = match j {
            Some(j) => {
                consumed[j] = true;
                // the positive line leads
                let pos = |c: &Chain| c.order.iter().any(|&n| is_positive(board.net_name(n)));
                if !pos(&chains[i]) && pos(&chains[j]) {
                    (j, Some(i))
                } else {
                    (i, Some(j))
                }
            }
            None => (i, None),
        };
        let c = &chains[pi];
        if c.ends.is_empty() || !(c.has_target || partner.is_some_and(|q| chains[q].has_target)) {
            continue; // no device at either end, or not an impedance-controlled line
        }
        let mut sh: Vec<Shunt> = c.order.iter().flat_map(|n| shunts.get(n).cloned().unwrap_or_default()).collect();
        sh.sort_by(|a, b| a.part.refdes.cmp(&b.part.refdes));
        let mut oth: Vec<String> = c.order.iter().flat_map(|n| others.get(n).cloned().unwrap_or_default()).collect();
        oth.sort();
        oth.dedup();
        let name_net = c.order.iter().copied().find(|n| target_net.contains(n)).unwrap_or(c.order[0]);
        lanes.push(Lane {
            name: board.net_name(name_net).to_string(),
            class: board.class_of(name_net).name.clone(),
            nets: c.order.clone(),
            partner_nets: partner.map(|q| chains[q].order.clone()).unwrap_or_default(),
            diff: partner.is_some(),
            hops: c.hops.clone(),
            partner_hops: partner.map(|q| chains[q].hops.clone()).unwrap_or_default(),
            partner_ends: partner.map(|q| chains[q].ends.clone()).unwrap_or_default(),
            branches: vec![],
            shunts: sh,
            ends: c.ends.clone(),
            kind: c.kind.to_string(),
            rx_end: 1,
            direction: String::new(),
            others: oth,
        });
    }
    let lanes: Vec<Lane> = lanes.into_iter().flat_map(split_multidrop).map(guess_direction).collect();
    // buses: same set of end devices and class
    let mut groups: BTreeMap<(Vec<String>, String), Vec<Lane>> = BTreeMap::new();
    for l in lanes {
        let mut devs: Vec<String> = l.ends.iter().map(|p| p.refdes.clone()).collect();
        devs.sort();
        devs.dedup();
        groups.entry((devs, l.class.clone())).or_default().push(l);
    }
    let mut out: Vec<Bus> = groups
        .into_iter()
        .map(|((devices, class), mut lanes)| {
            lanes.sort_by(|a, b| natural_cmp(&a.name, &b.name));
            let values = devices
                .iter()
                .map(|d| lanes.iter().flat_map(|l| &l.ends).find(|p| &p.refdes == d).map(|p| p.value.clone()).unwrap_or_default())
                .collect();
            let kind = if lanes.iter().all(|l| l.kind == lanes[0].kind) { lanes[0].kind.clone() } else { "mixed".into() };
            let name = if devices.is_empty() { format!("{class} (no end devices)") } else { devices.join(" ↔ ") };
            Bus { name, devices, values, kind, class, lanes }
        })
        .collect();
    out.sort_by(|a, b| kind_rank(&a.kind).cmp(&kind_rank(&b.kind)).then(a.name.cmp(&b.name)));
    out
}

/// Terminations (shunt resistors up to 200 Ω, to a supply or across the pair).
pub fn terminations(lane: &Lane) -> impl Iterator<Item = &Shunt> {
    lane.shunts.iter().filter(|s| s.part.kind == 'R' && s.part.val.is_some_and(|v| v <= 200.0))
}

/// Receiving end: next to a termination resistor if there is one, else the second end.
fn guess_direction(mut l: Lane) -> Lane {
    l.rx_end = 1.min(l.ends.len().saturating_sub(1));
    l.direction = "default order".into();
    if l.ends.len() == 2 {
        let term = terminations(&l).next().map(|t| (t.at, t.part.refdes.clone()));
        if let Some((at, refdes)) = term {
            let d = |e: &EndPin| geom::len(geom::sub(e.at, at));
            l.rx_end = if d(&l.ends[0]) < d(&l.ends[1]) { 0 } else { 1 };
            l.direction = format!("termination {} next to {}", refdes, l.ends[l.rx_end].refdes);
        }
    }
    l
}

fn rev_hop(h: &Hop) -> Hop {
    match h {
        Hop::Net { net, from, to } => Hop::Net { net: *net, from: *to, to: *from },
        Hop::Series { part } => {
            let mut p = part.clone();
            p.pads.swap(0, 1);
            Hop::Series { part: p }
        }
    }
}

/// Hops reversed (the same copper walked from the other end).
pub fn reverse_hops(h: &[Hop]) -> Vec<Hop> {
    h.iter().rev().map(rev_hop).collect()
}

/// Split `hops` at point `p` on the net hop with index `k`: (towards the start, towards the end),
/// both beginning at `p`.
fn split_at(hops: &[Hop], k: usize, p: P) -> (Vec<Hop>, Vec<Hop>) {
    let Hop::Net { net, from, to } = &hops[k] else { return (vec![], hops.to_vec()) };
    let mut left = vec![Hop::Net { net: *net, from: p, to: *from }];
    left.extend(reverse_hops(&hops[..k]));
    let mut right = vec![Hop::Net { net: *net, from: p, to: *to }];
    right.extend(hops[k + 1..].iter().cloned());
    (left, right)
}

/// A line with several devices where one IC sits between the others (e.g. an FPGA
/// pin feeding two connectors through AC-coupling capacitors, one of them a
/// second population option): one lane from the IC to each other end, the other
/// side as an open branch.
fn split_multidrop(l: Lane) -> Vec<Lane> {
    if l.kind != "multi-drop" || l.ends.len() < 3 {
        return vec![l];
    }
    let ics: Vec<&EndPin> = l.ends.iter().filter(|e| e.role == "ic").collect();
    let devs: HashSet<&str> = l.ends.iter().map(|e| e.refdes.as_str()).collect();
    if ics.len() != 1 || devs.len() != l.ends.len() {
        return vec![l];
    }
    let ic = ics[0].clone();
    let Some(k) = l.hops.iter().position(|h| matches!(h, Hop::Net { net, .. } if *net == ic.net)) else { return vec![l] };
    let (left, right) = split_at(&l.hops, k, ic.at);
    let first = l.ends.first().cloned();
    let last = l.ends.last().cloned();
    let mut out = Vec::new();
    for (hops, branch, end) in [(right.clone(), left.clone(), last), (left, right, first)] {
        let Some(end) = end else { continue };
        if end.refdes == ic.refdes {
            continue;
        }
        let kind = if end.role == "ic" { "ic-ic" } else { "ic-connector" };
        let mut nl = l.clone();
        nl.ends = vec![ic.clone(), end];
        nl.kind = kind.to_string();
        nl.hops = hops;
        nl.branches = vec![branch];
        nl.partner_hops = vec![]; // partner split below
        if !l.partner_hops.is_empty() {
            // the partner line has the same structure: split it at the partner pin of the IC
            if let Some(pk) = l.partner_hops.iter().position(|h| matches!(h, Hop::Net { net, .. } if l.partner_nets.contains(net) && l.partner_nets.iter().position(|n| n == net) == l.nets.iter().position(|n| *n == ic.net))) {
                if let Hop::Net { from, to, net } = &l.partner_hops[pk] {
                    // junction: the IC's pin on the partner net, else the closer end of that hop
                    let pin = l.partner_ends.iter().find(|e| e.refdes == ic.refdes && e.net == *net).map(|e| e.at);
                    let jp = pin.unwrap_or(if geom::len(geom::sub(*from, ic.at)) < geom::len(geom::sub(*to, ic.at)) { *from } else { *to });
                    let (pl, pr) = split_at(&l.partner_hops, pk, jp);
                    nl.partner_hops = if out.is_empty() { pr } else { pl };
                }
            }
        }
        out.push(nl);
    }
    if out.is_empty() {
        vec![l]
    } else {
        out
    }
}

fn is_positive(name: &str) -> bool {
    let n = name.trim_end_matches(')');
    n.ends_with('P') || n.ends_with('p') || n.ends_with('+')
}

/// Differential partners by name: `diff_partners` plus names with the polarity
/// inside parentheses, as KiCad generates them ("Net-(U12-OUT+)" / "Net-(U12-OUT-)").
fn pair_map(nets: &[String]) -> HashMap<u32, u32> {
    let mut m: HashMap<u32, u32> = HashMap::new();
    for (i, p) in diff_partners(nets).into_iter().enumerate() {
        if let Some(p) = p {
            m.insert(i as u32, p);
        }
    }
    let by_name: HashMap<&str, u32> = nets.iter().enumerate().map(|(i, n)| (n.as_str(), i as u32)).collect();
    for (i, n) in nets.iter().enumerate() {
        if let Some(inner) = n.strip_suffix(')') {
            let (base, last) = inner.split_at(inner.len().saturating_sub(1));
            let other = match last {
                "+" => format!("{base}-)"),
                "-" => format!("{base}+)"),
                "P" => format!("{base}N)"),
                "N" => format!("{base}P)"),
                _ => continue,
            };
            if let Some(&j) = by_name.get(other.as_str()) {
                m.insert(i as u32, j);
            }
        }
    }
    m
}

fn kind_rank(k: &str) -> u8 {
    match k {
        "ic-ic" => 0,
        "ic-connector" => 1,
        "connector-connector" => 2,
        "mixed" => 3,
        "multi-drop" => 4,
        _ => 5,
    }
}

/// Compare strings with embedded numbers numerically ("LANE2" < "LANE10").
pub fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let (mut x, mut y) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (x.peek(), y.peek()) {
            (None, None) => return std::cmp::Ordering::Equal,
            (None, _) => return std::cmp::Ordering::Less,
            (_, None) => return std::cmp::Ordering::Greater,
            (Some(c), Some(d)) if c.is_ascii_digit() && d.is_ascii_digit() => {
                let mut na = String::new();
                while let Some(c) = x.peek().filter(|c| c.is_ascii_digit()) {
                    na.push(*c);
                    x.next();
                }
                let mut nb = String::new();
                while let Some(c) = y.peek().filter(|c| c.is_ascii_digit()) {
                    nb.push(*c);
                    y.next();
                }
                let o = na.parse::<u64>().unwrap_or(0).cmp(&nb.parse::<u64>().unwrap_or(0));
                if o != std::cmp::Ordering::Equal {
                    return o;
                }
            }
            (Some(c), Some(d)) => {
                let o = c.cmp(d);
                if o != std::cmp::Ordering::Equal {
                    return o;
                }
                x.next();
                y.next();
            }
        }
    }
}

// --- channel of a lane ------------------------------------------------------------------

use crate::extract::FieldResult;
use crate::rf::{Build, Channel, LumpedPart, NetGraph, RfParams};

fn lumped(p: &Passive, series: bool, scale: f64, esr: f64) -> LumpedPart {
    let v = p.val.unwrap_or(0.0);
    let esl = p.esl_nh * 1e-9;
    let (r, l, c) = match p.kind {
        'C' => (esr, esl, Some(v.max(1e-15))),
        'L' => (0.01, v + esl, None),
        _ => (v, esl, None),
    };
    let label = format!("{} {}{}", p.refdes, p.value, if scale < 1.0 { " (across pair, odd mode)" } else { "" });
    LumpedPart { label, series, r: r * scale, l: l * scale, c: c.map(|c| c / scale) }
}

fn pad_cap(p: &Passive) -> LumpedPart {
    LumpedPart { label: format!("{} pad", p.refdes), series: false, r: 0.0, l: 0.0, c: Some(p.pad_c_pf.max(1e-3) * 1e-12) }
}

/// Add the copper and parts of `hops` to the build; shunts on these nets are placed
/// at the closest point of the path.
fn add_hops(b: &mut Build, board: &Board, an: &Analysis, hops: &[Hop], shunts: &[Shunt], esr: f64) -> Result<(), String> {
    for h in hops {
        match h {
            Hop::Net { net, from, to } => {
                let g = NetGraph::build(board, an, *net).ok_or_else(|| format!("net {} has no copper", board.net_name(*net)))?;
                let (na, nb) = (g.nearest(*from).unwrap_or(0), g.nearest(*to).unwrap_or(0));
                let path = if na == nb { vec![] } else { g.path(an, na, nb) };
                if g.has_branches() {
                    b.warnings.push(format!("{} branches: side stubs (test points, unused pads) are not included", board.net_name(*net)));
                }
                // piece end points to place the shunts of this net
                let ends: Vec<P> = path
                    .iter()
                    .map(|&(pi, rev)| {
                        let pc = &an.pieces[pi * 4..pi * 4 + 4];
                        let tr = &board.tracks[pc[0] as usize];
                        geom::lerp(tr.a, tr.b, if rev { pc[1] } else { pc[2] })
                    })
                    .collect();
                let mut at: Vec<(usize, LumpedPart)> = Vec::new();
                let mut before: Vec<LumpedPart> = Vec::new();
                for sh in shunts.iter().filter(|s| s.net == *net) {
                    let lp = lumped(&sh.part, false, if sh.across { 0.5 } else { 1.0 }, esr);
                    let d0 = geom::len(geom::sub(*from, sh.at));
                    match ends.iter().enumerate().min_by(|a, c| geom::len(geom::sub(*a.1, sh.at)).partial_cmp(&geom::len(geom::sub(*c.1, sh.at))).unwrap()) {
                        Some((i, e)) if geom::len(geom::sub(*e, sh.at)) < d0 => at.push((i, lp)),
                        _ => before.push(lp),
                    }
                }
                for lp in before {
                    b.add_part(lp);
                }
                let mut k = 0usize;
                b.add_path(*net, &path, |b, _| {
                    for (_, lp) in at.iter().filter(|(i, _)| *i == k) {
                        b.add_part(lp.clone());
                    }
                    k += 1;
                });
            }
            Hop::Series { part } => {
                b.add_part(pad_cap(part));
                b.add_part(lumped(part, true, 1.0, esr));
                b.add_part(pad_cap(part));
            }
        }
    }
    Ok(())
}

/// Channel of a lane from `ends[0]` to `ends[1]` (or reversed), with series parts,
/// terminations and open branches.
/// `with_terminations`: include termination resistors (for the eye; loss, return
/// loss and TDR describe the interconnect without them, the termination belongs to
/// the receiving end).
pub fn lane_channel(board: &Board, an: &Analysis, lane: &Lane, field: &HashMap<String, FieldResult>, target: Option<(f64, bool)>, rp: &RfParams, reversed: bool, with_terminations: bool, cd: &ComponentData) -> Result<Channel, String> {
    let diff = lane.diff && target.map(|t| t.1).unwrap_or(true);
    let esr = cd.capacitor.esr_ohm;
    let term: HashSet<&str> = terminations(lane).map(|t| t.part.refdes.as_str()).collect();
    let shunts: Vec<Shunt> = lane.shunts.iter().filter(|s| with_terminations || !term.contains(s.part.refdes.as_str())).cloned().collect();
    let hops = if reversed { reverse_hops(&lane.hops) } else { lane.hops.clone() };
    let branches: Vec<Channel> = lane
        .branches
        .iter()
        .map(|bh| {
            let mut bb = Build::new(board, an, field, rp, diff);
            add_hops(&mut bb, board, an, bh, &shunts, esr)?;
            Ok(bb.finish(lane.nets[0], "branch".into(), None, target))
        })
        .collect::<Result<_, String>>()?;
    let label = |c: &Channel| format!("open branch {:.1} mm", c.length_mm);
    let mut b = Build::new(board, an, field, rp, diff);
    if !reversed {
        for c in &branches {
            b.add_branch(label(c), c);
        }
    }
    add_hops(&mut b, board, an, &hops, &shunts, esr)?;
    if reversed {
        for c in &branches {
            b.add_branch(label(c), c);
        }
    }
    if !lane.branches.is_empty() {
        b.warnings.push(format!("the line also continues to another device: modelled as {} open branch(es) at {}", lane.branches.len(), lane.ends[0].refdes));
    }
    let partner = lane.partner_nets.first().copied();
    Ok(b.finish(lane.nets[0], lane.name.clone(), partner, target))
}

/// Result of evaluating a lane: the interconnect channel (without terminations),
/// the eye (IC to IC only) and the skew to the partner line.
pub struct LaneEval {
    pub ch: Channel,
    pub eye: Option<Result<crate::eye::Eye, String>>,
    /// receiver model used for the eye (after merging terminations at the receiver)
    pub rx_used: Option<crate::eye::EndModel>,
    pub rx_note: Option<String>,
    /// (skew ps, skew mm, partner name, uncertain)
    pub skew: Option<(f64, f64, String, bool)>,
}

/// Evaluate a lane. `reversed`: transmit from `ends[1]` to `ends[0]` (use
/// `lane.rx_end == 0` for the guessed direction). `eye`: bit rate, rise, PRBS
/// order, transmitter, receiver of the profile.
pub fn lane_eval(
    board: &Board,
    an: &Analysis,
    field: &HashMap<String, FieldResult>,
    lane: &Lane,
    target: Option<(f64, bool)>,
    rp: &RfParams,
    eye: Option<(f64, f64, u32, Option<crate::eye::EndModel>, Option<crate::eye::EndModel>)>,
    reversed: bool,
    cd: &ComponentData,
) -> Result<LaneEval, String> {
    let ch = lane_channel(board, an, lane, field, target, rp, reversed, false, cd)?;
    let mut out = LaneEval { ch, eye: None, rx_used: None, rx_note: None, skew: None };
    if let (Some((bitrate, rise, prbs, tx, rx)), true) = (eye, lane.kind == "ic-ic") {
        // a termination resistor next to the receiver acts as the receiver's
        // termination (R/2 per line in odd mode when it sits across the pair);
        // terminations elsewhere (source side) stay in the channel
        let rx_end = if reversed { 0 } else { 1 };
        let (rx_pos, tx_pos) = (lane.ends[rx_end].at, lane.ends[1 - rx_end].at);
        let at_rx: Vec<&Shunt> = terminations(lane).filter(|t| geom::len(geom::sub(t.at, rx_pos)) < geom::len(geom::sub(t.at, tx_pos))).collect();
        let others = terminations(lane).count() > at_rx.len();
        let mut rx_model = rx;
        if !at_rx.is_empty() {
            let g: f64 = at_rx.iter().map(|t| 1.0 / (t.part.val.unwrap_or(1e9).max(1e-3) * if t.across { 0.5 } else { 1.0 })).sum();
            rx_model = Some(crate::eye::EndModel { r: 1.0 / g, ..rx.unwrap_or_default() });
            out.rx_note = Some(format!(
                "termination {} at {} used as the receiver termination ({:.0} Ω per line)",
                at_rx.iter().map(|t| t.part.refdes.as_str()).collect::<Vec<_>>().join(", "),
                lane.ends[rx_end].refdes,
                1.0 / g
            ));
        }
        let eye_ch = if others {
            let rx_refs: HashSet<&str> = at_rx.iter().map(|t| t.part.refdes.as_str()).collect();
            let mut l2 = lane.clone();
            l2.shunts.retain(|s| !rx_refs.contains(s.part.refdes.as_str()));
            lane_channel(board, an, &l2, field, target, rp, reversed, true, cd)
        } else {
            Ok(out.ch.clone())
        };
        out.rx_used = rx_model;
        out.eye = Some(eye_ch.and_then(|c| crate::eye::eye(&c, &crate::eye::EyeParams { bitrate, rise, prbs, tx, rx: rx_model, ..Default::default() })));
    }
    if out.ch.diff && !lane.partner_hops.is_empty() {
        let rq = RfParams { npts: 10, ..rp.clone() };
        let pl = Lane { hops: lane.partner_hops.clone(), shunts: vec![], branches: vec![], ..lane.clone() };
        if let Ok(pc) = lane_channel(board, an, &pl, field, target, &rq, false, false, cd) {
            let uncertain = out.ch.warnings.iter().chain(&pc.warnings).any(|w| w.contains(" branches: "));
            let pname = lane.partner_nets.iter().map(|&n| board.net_name(n)).find(|_| true).unwrap_or("").to_string();
            out.skew = Some((out.ch.delay_ps - pc.delay_ps, out.ch.length_mm - pc.length_mm, pname, uncertain));
        }
    }
    Ok(out)
}

/// Channel check of one lane: loss, return loss, stubs, skew; the eye only from IC to IC.
/// `reversed`: transmit from `ends[1]` to `ends[0]` (use `lane.rx_end == 0` for the guessed direction).
pub fn check_lane(
    board: &Board,
    an: &Analysis,
    field: &HashMap<String, FieldResult>,
    lane: &Lane,
    target: Option<(f64, bool)>,
    rp: &RfParams,
    gbps: f64,
    ends: (Option<crate::eye::EndModel>, Option<crate::eye::EndModel>, Option<f64>),
    reversed: bool,
    cd: &ComponentData,
) -> crate::eye::CheckRow {
    let mut row = crate::eye::empty_row(lane.nets[0], lane.name.clone(), gbps);
    let rp = RfParams { fmax: rp.fmax.max(gbps * 1e9 * 1.25), ..rp.clone() };
    let (tx, rx, rise) = ends;
    match lane_eval(board, an, field, lane, target, &rp, Some((gbps * 1e9, rise.unwrap_or(rp.rise), 7, tx, rx)), reversed, cd) {
        Ok(ev) => {
            crate::eye::fill_row(&mut row, &ev.ch, gbps, rise.unwrap_or(rp.rise), (tx, rx), false);
            match ev.eye {
                Some(Ok(e)) => {
                    row.eye = true;
                    row.eye_height = e.height;
                    row.eye_width_ui = e.width_ps / e.ui_ps;
                }
                Some(Err(e)) => {
                    row.eye = true;
                    row.error = Some(e);
                }
                None => row.eye = false,
            }
            if let Some((sk, _, _, unc)) = ev.skew {
                row.skew_ps = Some(sk);
                row.skew_uncertain = unc;
            }
        }
        Err(e) => row.error = Some(e),
    }
    row
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_and_roles() {
        for (s, v) in [("0", 0.0), ("22", 22.0), ("4k7", 4700.0), ("49.9R", 49.9), ("220nF", 220e-9), ("100n", 100e-9), ("4u7", 4.7e-6), ("10µF/6.3V", 10e-6), ("1M", 1e6), ("R10", 0.1)] {
            let p = parse_value(s).unwrap_or(f64::NAN);
            assert!((p - v).abs() <= v.abs() * 1e-9 + 1e-15, "{s}: {p}");
        }
        assert!(parse_value("DNP").is_none());
        let cd = ComponentData::builtin();
        assert_eq!(cd.role("R38"), "resistor");
        assert_eq!(cd.role("U18"), "ic");
        assert_eq!(cd.role("USB1"), "connector");
        assert_eq!(cd.role("FB13"), "inductor");
        assert_eq!(cd.role("TP4"), "test_point");
        assert_eq!(cd.role("P1"), "connector");
        assert_eq!(cd.role("MH1"), "other");
        assert_eq!(natural_cmp("PET2", "PET10"), std::cmp::Ordering::Less);
    }
}

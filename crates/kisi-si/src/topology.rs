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
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

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
    /// copper layers of the pad (bit per layer, 0 = all: through-hole)
    #[serde(skip)]
    pub layers: u64,
}

/// Copper layers of a pad as a bit mask (0 = any layer: through-hole pads).
pub fn pad_layers(board: &Board, pad: usize) -> u64 {
    let p = &board.pads[pad];
    if p.drill > 0.0 {
        return 0;
    }
    p.layers.iter().fold(0, |m, &l| m | 1u64 << l.min(63))
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
    /// a piece of copper of one net, from point to point (pads on the copper layers
    /// `from_layers` / `to_layers`, bit masks, 0 = any)
    Net {
        net: u32,
        from: P,
        to: P,
        #[serde(skip)]
        from_layers: u64,
        #[serde(skip)]
        to_layers: u64,
    },
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
    /// name of the (positive) line: a net of its own path with a target – at the far end of a
    /// lane from an IC (the connector, or the device behind the fork), else the first one
    pub name: String,
    /// the net the name, class and profile come from
    pub name_net: u32,
    /// nets of the line's own path in order from `ends[0]` to `ends[1]` (not those of its
    /// branches: the other paths of a fork are in `branches`)
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
    /// device at the far end of each branch
    pub branch_ends: Vec<String>,
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
                    ends.entry(net).or_default().push(EndPin { refdes: f.reference.clone(), pin: board.pads[pi].number.clone(), value: f.value.clone(), role: pt.role.clone(), net, at: board.pads[pi].at, layers: pad_layers(board, pi) });
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
        let mut entry_l: u64 = e.iter().find(|p| p.net == first).map(|p| p.layers).unwrap_or(0);
        let mut simple = true;
        loop {
            let next: Vec<&(u32, usize, usize, usize)> =
                links.get(&cur).map(|x| x.iter().filter(|l| comp_set.contains(&l.0) && !used_parts.contains(&l.1)).collect()).unwrap_or_default();
            if next.len() > 1 {
                simple = false;
            }
            let Some(&&(v, k, pa, pb)) = next.first() else { break };
            hops.push(Hop::Net { net: cur, from: entry, to: board.pads[pa].at, from_layers: entry_l, to_layers: pad_layers(board, pa) });
            hops.push(Hop::Series { part: mk_passive(&parts[k], pa, pb) });
            used_parts.insert(k);
            entry = board.pads[pb].at;
            entry_l = pad_layers(board, pb);
            if order.contains(&v) {
                simple = false;
                break;
            }
            cur = v;
            order.push(v);
        }
        let start_pin = e.iter().find(|p| p.net == first).cloned();
        let (last_end, last_l) = e
            .iter()
            .rev()
            .find(|p| p.net == cur && start_pin.as_ref().map_or(true, |s| s.refdes != p.refdes || s.pin != p.pin))
            .map(|p| (p.at, p.layers))
            .unwrap_or((entry, entry_l));
        hops.push(Hop::Net { net: cur, from: entry, to: last_end, from_layers: entry_l, to_layers: last_l });
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
            name_net,
            class: board.class_of(name_net).name.clone(),
            nets: c.order.clone(),
            partner_nets: partner.map(|q| chains[q].order.clone()).unwrap_or_default(),
            diff: partner.is_some(),
            hops: c.hops.clone(),
            partner_hops: partner.map(|q| chains[q].hops.clone()).unwrap_or_default(),
            partner_ends: partner.map(|q| chains[q].ends.clone()).unwrap_or_default(),
            branches: vec![],
            branch_ends: vec![],
            shunts: sh,
            ends: c.ends.clone(),
            kind: c.kind.to_string(),
            rx_end: 1,
            direction: String::new(),
            others: oth,
        });
    }
    // the path from a pin to an end pin over the series links (breadth first)
    let route = |start: (u32, P, u64), to: &EndPin| -> Option<(Vec<u32>, Vec<Hop>)> {
        let mut prev: HashMap<u32, (u32, usize, usize, usize)> = HashMap::new();
        let mut seen: HashSet<u32> = HashSet::from([start.0]);
        let mut q = std::collections::VecDeque::from([start.0]);
        while let Some(u) = q.pop_front() {
            if u == to.net {
                break;
            }
            for &(v, k, pa, pb) in links.get(&u).map(|x| x.as_slice()).unwrap_or(&[]) {
                if seen.insert(v) {
                    prev.insert(v, (u, k, pa, pb));
                    q.push_back(v);
                }
            }
        }
        if !seen.contains(&to.net) {
            return None;
        }
        let mut steps = Vec::new();
        let mut cur = to.net;
        while cur != start.0 {
            let st = prev[&cur];
            steps.push(st);
            cur = st.0;
        }
        steps.reverse();
        let mut nets = vec![start.0];
        let mut hops = Vec::new();
        let (mut entry, mut entry_l) = (start.1, start.2);
        for &(u, k, pa, pb) in &steps {
            hops.push(Hop::Net { net: u, from: entry, to: board.pads[pa].at, from_layers: entry_l, to_layers: pad_layers(board, pa) });
            hops.push(Hop::Series { part: mk_passive(&parts[k], pa, pb) });
            entry = board.pads[pb].at;
            entry_l = pad_layers(board, pb);
            nets.push(board.pads[pb].net);
        }
        hops.push(Hop::Net { net: to.net, from: entry, to: to.at, from_layers: entry_l, to_layers: to.layers });
        Some((nets, hops))
    };
    let lanes: Vec<Lane> = lanes.into_iter().flat_map(|l| split_multidrop(l, &route)).map(|l| name_lane(board, &target_net, l)).map(guess_direction).collect();
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

/// A signal net that forks into alternative series parts which are all fitted – the mark of
/// an assembly variant (ThunderScope Rev5.3: AC-coupling capacitors and 0-Ω links back to back
/// on F.Cu and B.Cu, leading to the PCIe edge connector and to the M.2 connector).
#[derive(Debug, Clone, Serialize)]
pub struct Fork {
    pub net: u32,
    pub net_name: String,
    /// partner net when the fork is on a differential pair (listed once, the positive line)
    pub partner: Option<String>,
    /// the alternative parts, all fitted
    pub parts: Vec<String>,
    pub value: String,
    /// devices reached through each part
    pub towards: Vec<Vec<String>>,
    pub at: P,
}

/// Parts closer than this (centre to centre, mm) count as placed together (back to back on
/// F.Cu and B.Cu, overlapping, or side by side at the smallest pitch) – a sign of alternatives.
pub const FORK_MM: f64 = 2.0;

/// Forks with all alternatives fitted (see `Fork`): a net with two or more fitted series parts
/// of the same kind and value (series resistors ≤ 100 Ω, AC-coupling capacitors, small
/// inductors), placed together (within `FORK_MM`), that lead to different devices (no device
/// reached through both). A heuristic: "possible assembly variant". On the example boards it
/// finds the twelve variant forks of ThunderScope Rev5.3 and nothing on LibreVNA (0-Ω fan-outs
/// there are 2.4–4 mm apart; a supply split reaches one device through both parts).
pub fn forks(board: &Board, cd: &ComponentData) -> Vec<Fork> {
    let partners = pair_map(&board.nets);
    let mut fp_pads: Vec<Vec<usize>> = vec![vec![]; board.footprints.len()];
    for (pi, p) in board.pads.iter().enumerate() {
        if p.net != 0 && (p.fp as usize) < fp_pads.len() {
            fp_pads[p.fp as usize].push(pi);
        }
    }
    // fitted series parts between signal nets, and the devices on every net
    let mut links: BTreeMap<u32, Vec<(u32, usize, f64)>> = BTreeMap::new();
    let mut devices: HashMap<u32, BTreeSet<String>> = HashMap::new();
    for (i, f) in board.footprints.iter().enumerate() {
        if f.dnp {
            continue;
        }
        let role = cd.role(&f.reference);
        let pads = &fp_pads[i];
        match role {
            "resistor" | "capacitor" | "inductor" => {
                if pads.len() != 2 {
                    continue;
                }
                let (na, nb) = (board.pads[pads[0]].net, board.pads[pads[1]].net);
                let Some(v) = parse_value(&f.value) else { continue };
                let passes = match role {
                    "resistor" => v <= 100.0,
                    "capacitor" => v >= 1e-9,
                    _ => v <= 100e-9,
                };
                if na == nb || !passes || is_supply(board, na) || is_supply(board, nb) || partners.get(&na) == Some(&nb) {
                    continue;
                }
                links.entry(na).or_default().push((nb, i, v));
                links.entry(nb).or_default().push((na, i, v));
            }
            "ic" | "connector" => {
                for &pi in pads {
                    let n = board.pads[pi].net;
                    if !is_supply(board, n) {
                        devices.entry(n).or_default().insert(f.reference.clone());
                    }
                }
            }
            _ => {}
        }
    }
    // devices reached from `start` over series parts, without entering `block`
    let reach = |start: u32, block: u32| -> BTreeSet<String> {
        let mut seen: HashSet<u32> = HashSet::from([start, block]);
        let mut st = vec![start];
        let mut out = BTreeSet::new();
        while let Some(u) = st.pop() {
            out.extend(devices.get(&u).into_iter().flatten().cloned());
            for &(v, ..) in links.get(&u).map(|x| x.as_slice()).unwrap_or(&[]) {
                if seen.insert(v) {
                    st.push(v);
                }
            }
        }
        out
    };
    let mut out: Vec<Fork> = Vec::new();
    for (&n, ls) in &links {
        if ls.len() < 2 {
            continue;
        }
        // alternatives: same kind and value
        let mut groups: BTreeMap<(String, String), Vec<(u32, usize, f64)>> = BTreeMap::new();
        for &(o, i, v) in ls {
            groups.entry((cd.role(&board.footprints[i].reference).to_string(), format!("{v:.4e}"))).or_default().push((o, i, v));
        }
        for (_, g) in groups {
            let first = board.footprints[g[0].1].at;
            let alt: Vec<&(u32, usize, f64)> = g.iter().filter(|x| geom::len(geom::sub(board.footprints[x.1].at, first)) <= FORK_MM).collect();
            let others: HashSet<u32> = alt.iter().map(|x| x.0).collect();
            if alt.len() < 2 || others.len() < 2 {
                continue;
            }
            let towards: Vec<Vec<String>> = alt.iter().map(|x| reach(x.0, n).into_iter().collect()).collect();
            let mut all: Vec<&String> = towards.iter().flatten().collect();
            let count = all.len();
            all.sort();
            all.dedup();
            if towards.iter().any(|t| t.is_empty()) || all.len() < count {
                continue; // an alternative leads nowhere, or a device is reached through two of them
            }
            out.push(Fork {
                net: n,
                net_name: board.net_name(n).to_string(),
                partner: partners.get(&n).map(|&p| board.net_name(p).to_string()),
                parts: alt.iter().map(|x| board.footprints[x.1].reference.clone()).collect(),
                value: board.footprints[alt[0].1].value.clone(),
                towards,
                at: first,
            });
        }
    }
    // a differential pair once: the positive line (else the lower net id)
    let listed: HashSet<u32> = out.iter().map(|f| f.net).collect();
    let by_name: HashMap<&str, u32> = board.nets.iter().enumerate().map(|(i, n)| (n.as_str(), i as u32)).collect();
    out.retain(|f| {
        let Some(p) = f.partner.as_deref().and_then(|p| by_name.get(p).copied()).filter(|p| listed.contains(p)) else { return true };
        let (me, other) = (is_positive(&f.net_name), is_positive(board.net_name(p)));
        if me != other { me } else { f.net < p }
    });
    out.sort_by(|a, b| natural_cmp(&a.net_name, &b.net_name));
    out
}

/// Name and class of a lane from its own path: towards the far end of a lane that leaves an
/// IC at a fork (the device behind its own branch of the fork, not the shared IC net) and
/// towards the connector of an IC–connector lane; else the first net with a target.
fn name_lane(board: &Board, target_net: &HashSet<u32>, mut l: Lane) -> Lane {
    let has_target = |n: &&u32| target_net.contains(n);
    let role = |i: usize| l.ends.get(i).map(|e| e.role.as_str()).unwrap_or("");
    let backwards = !l.branches.is_empty() || (l.ends.len() == 2 && role(0) == "ic" && role(1) == "connector");
    let pick = if backwards { l.nets.iter().rev().find(has_target) } else { l.nets.iter().find(has_target) };
    if let Some(&n) = pick.or(l.nets.first()) {
        l.name = board.net_name(n).to_string();
        l.name_net = n;
        l.class = board.class_of(n).name.clone();
    }
    l
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
        Hop::Net { net, from, to, from_layers, to_layers } => Hop::Net { net: *net, from: *to, to: *from, from_layers: *to_layers, to_layers: *from_layers },
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

/// Route from a pin to an end pin over the series links of a chain: the nets in order and the
/// hops (see `buses`).
type Route<'r> = dyn Fn((u32, P, u64), &EndPin) -> Option<(Vec<u32>, Vec<Hop>)> + 'r;

/// Number of series parts two routes from the same pin share before they part (the fork).
fn common_steps(a: &[Hop], b: &[Hop]) -> usize {
    let parts = |h: &[Hop]| h.iter().filter_map(|x| if let Hop::Series { part } = x { Some(part.refdes.clone()) } else { None }).collect::<Vec<_>>();
    parts(a).iter().zip(parts(b).iter()).take_while(|(x, y)| x == y).count()
}

/// A line with several devices and one IC (e.g. an FPGA pin feeding two connectors through
/// AC-coupling capacitors, one of them a second population option, or an IC behind a series
/// resistor and a fork): one lane from the IC to each other end along its own path, the other
/// paths as open branches from where they leave it (`route`: the path from a pin to an end).
fn split_multidrop(l: Lane, route: &Route) -> Vec<Lane> {
    if l.kind != "multi-drop" || l.ends.len() < 3 {
        return vec![l];
    }
    let ics: Vec<&EndPin> = l.ends.iter().filter(|e| e.role == "ic").collect();
    let devs: HashSet<&str> = l.ends.iter().map(|e| e.refdes.as_str()).collect();
    if ics.len() != 1 || devs.len() != l.ends.len() {
        return vec![l];
    }
    let ic = ics[0].clone();
    let pin = |e: &EndPin| (e.net, e.at, e.layers);
    let others: Vec<&EndPin> = l.ends.iter().filter(|e| e.refdes != ic.refdes).collect();
    let routes: Vec<Option<(Vec<u32>, Vec<Hop>)>> = others.iter().map(|e| route(pin(&ic), e)).collect();
    // the partner line: from the IC's partner pin to the end's partner pin
    let p_ic = l.partner_ends.iter().find(|e| e.refdes == ic.refdes);
    let mut out = Vec::new();
    for (i, end) in others.iter().enumerate() {
        let Some((nets, hops)) = routes[i].clone() else { continue };
        let mut nl = l.clone();
        nl.kind = if end.role == "ic" { "ic-ic" } else { "ic-connector" }.to_string();
        nl.ends = vec![ic.clone(), (*end).clone()];
        nl.nets = nets;
        nl.hops = hops.clone();
        // the other paths, from the net where they part from this one (a branch must leave
        // through a series part; a device on a shared net is a side stub of that net)
        nl.branches = vec![];
        nl.branch_ends = vec![];
        for (k, other) in others.iter().enumerate() {
            let Some((_, oh)) = routes[k].as_ref().filter(|_| k != i) else { continue };
            let c = common_steps(&hops, oh);
            let b = oh[2 * c..].to_vec();
            if b.iter().any(|h| matches!(h, Hop::Series { .. })) {
                nl.branches.push(b);
                nl.branch_ends.push(other.refdes.clone());
            }
        }
        let p_end = l.partner_ends.iter().find(|e| e.refdes == end.refdes);
        match p_ic.zip(p_end).and_then(|(a, b)| route(pin(a), b)) {
            Some((pn, ph)) => {
                nl.partner_nets = pn;
                nl.partner_hops = ph;
            }
            None => {
                nl.partner_nets = nl.nets.iter().filter_map(|n| l.nets.iter().position(|x| x == n).and_then(|k| l.partner_nets.get(k).copied())).collect();
                nl.partner_hops = vec![];
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
use crate::rf::{Build, Channel, LumpedPart, RfParams};

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

/// Pad capacitance of a part that is not fitted (its pads stay as copper): the value per
/// package from `data/components.json` for two-pin parts (an estimate), None for others.
fn unfitted_pad_cap(board: &Board, fp: usize, cd: &ComponentData) -> Option<(String, LumpedPart)> {
    let pads: Vec<&crate::board::Pad> = board.pads.iter().filter(|p| p.fp as usize == fp).collect();
    if pads.len() != 2 {
        return None;
    }
    let pkg = package_of(geom::len(geom::sub(pads[0].at, pads[1].at)));
    let c = cd.package(pkg).pad_c_pf;
    let r = &board.footprints[fp].reference;
    Some((format!("{pkg}, {c:.2} pF estimated"), LumpedPart { label: format!("{r} pad (not fitted, estimate)"), series: false, r: 0.0, l: 0.0, c: Some(c.max(1e-3) * 1e-12) }))
}

/// What a lane's cascade needs besides its hops: the parts' data, the target (reference
/// impedance of stub channels) and the other paths of a fork (lane branches) that continue
/// beyond a side stub.
struct HopCtx<'c> {
    cd: &'c ComponentData,
    esr: f64,
    target: Option<(f64, bool)>,
    /// (series part next to the junction, hops beyond it, far-end device); consumed when a
    /// side stub ends at that part
    conts: Vec<(String, Vec<Hop>, String)>,
    used: Vec<bool>,
    notes: Vec<String>,
}

/// Add the copper and parts of `hops` to the build. Shunts on these nets are placed at the
/// closest point of the path, or at the end of the side stub that leads to them. Side stubs
/// (branches of a hop's net off its path) are open stubs: to unused pads and test points, to
/// the pads of parts that are not fitted (with their pad capacitance), and through fitted
/// parts on to the other path of a fork (a lane branch in `ctx.conts`).
fn add_hops(b: &mut Build, board: &Board, an: &Analysis, hops: &[Hop], shunts: &[Shunt], ctx: &mut HopCtx) -> Result<(), String> {
    let esr = ctx.esr;
    for h in hops {
        match h {
            Hop::Net { net, from, to, from_layers, to_layers } => {
                let info = crate::rf::hop_path(board, an, *net, (*from, *from_layers), (*to, *to_layers)).ok_or_else(|| format!("net {} has no copper", board.net_name(*net)))?;
                let path: Vec<(usize, bool)> = info.pieces.iter().map(|p| (p.piece, p.rev)).collect();
                // layer, end point and cross-section after every piece (junctions of the stubs)
                let at_piece: Vec<(u16, P, u32)> = path
                    .iter()
                    .map(|&(pi, rev)| {
                        let pc = &an.pieces[pi * 4..pi * 4 + 4];
                        let tr = &board.tracks[pc[0] as usize];
                        (tr.layer, geom::lerp(tr.a, tr.b, if rev { pc[1] } else { pc[2] }), pc[3] as u32)
                    })
                    .collect();
                // side stubs: (piece index, after the via in front of that piece?, label, channel)
                let mut stubs: Vec<(usize, bool, String, Channel)> = Vec::new();
                let mut at_stub_end: HashSet<String> = HashSet::new();
                let mut short = 0usize;
                for br in &info.branches {
                    let fp = br.end.refdes.as_ref().and_then(|r| board.footprints.iter().position(|f| &f.reference == r));
                    let refdes = br.end.refdes.clone().unwrap_or_default();
                    let cont = ctx.conts.iter().enumerate().position(|(i, c)| !ctx.used[i] && c.0 == refdes);
                    let shunt = shunts.iter().find(|s| s.net == *net && s.part.refdes == refdes && !at_stub_end.contains(&refdes));
                    let unfitted = fp.is_some_and(|i| board.footprints[i].dnp);
                    if !(br.modelled || cont.is_some() || unfitted || shunt.is_some()) {
                        short += 1;
                        continue;
                    }
                    // which side of a via at the junction the stub hangs on: the stub's layer
                    // against the layers before (prev) and after (next) the junction; a stub on
                    // another layer leaves through the via (its barrel added to the stub)
                    let bpath: Vec<(usize, bool)> = br.pieces.iter().map(|p| (p.piece, p.rev)).collect();
                    let k = br.after_piece;
                    let next = at_piece.get(k).copied();
                    let prev = if k == 0 { next.map(|n| (n.0, *from, n.2)) } else { at_piece.get(k - 1).copied() };
                    let s_layer = bpath.first().map(|&(pi, _)| board.tracks[an.pieces[pi * 4] as usize].layer);
                    let (after_via, junction) = match (prev, next, s_layer) {
                        (Some(p), Some(n), Some(sl)) if p.0 != n.0 => {
                            let (pl, nl, sl) = (p.0 as i32, n.0 as i32, sl as i32);
                            let beyond_next = (sl - pl).signum() == (nl - pl).signum() && (sl - pl).abs() >= (nl - pl).abs();
                            let between = (sl - pl).signum() == (nl - pl).signum() && (sl - pl).abs() < (nl - pl).abs() && sl != pl;
                            if beyond_next {
                                (true, Some((n.0, p.1, n.2)))
                            } else if between {
                                (false, Some((sl as u16, p.1, p.2)))
                            } else {
                                (false, Some(p))
                            }
                        }
                        (p, _, _) => (false, p),
                    };
                    let mut bb = b.sub();
                    bb.add_path_with(*net, &bpath, junction, |_, _| {}, |_, _| {});
                    // what the stub ends in; its label after the build (length, vias)
                    let what = if let Some(i) = cont {
                        ctx.used[i] = true;
                        let (part, rest, far) = ctx.conts[i].clone();
                        add_hops(&mut bb, board, an, &rest, shunts, ctx)?;
                        ctx.notes.push(format!("possible assembly variant: the line forks into {part} and on to {far}, all parts fitted – modelled as an open branch; choose the assembly variant (\"variant\" in <board>.kisi.json or --variant)"));
                        format!("{} and on through {part} to {far} (open, all alternatives fitted)", br.end.label)
                    } else if unfitted {
                        match unfitted_pad_cap(board, fp.unwrap(), ctx.cd) {
                            Some((pad, lp)) => {
                                bb.add_part(lp);
                                format!("{} (not fitted; pad {pad})", br.end.label)
                            }
                            None => format!("{} (not fitted)", br.end.label),
                        }
                    } else if let Some(sh) = shunt {
                        at_stub_end.insert(refdes.clone());
                        bb.add_part(lumped(&sh.part, false, if sh.across { 0.5 } else { 1.0 }, esr));
                        format!("{} {}", sh.part.refdes, sh.part.value)
                    } else {
                        br.end.label.clone()
                    };
                    let ch = bb.finish(*net, "branch".into(), None, ctx.target);
                    let via = match ch.vias {
                        0 => String::new(),
                        1 => " and a via".to_string(),
                        n => format!(" and {n} vias"),
                    };
                    let label = format!("{} {:.1} mm{via} to {what}", if cont.is_some() { "branch" } else { "stub" }, br.len_mm);
                    stubs.push((k, after_via, label, ch));
                }
                if !stubs.is_empty() || short > 0 {
                    let mut w = format!("{} branches: {} side stub(s) included as open stubs", board.net_name(*net), stubs.len());
                    if short > 0 {
                        w.push_str(&format!(", {short} shorter than {} mm not modelled", crate::rf::BRANCH_MIN_MM));
                    }
                    b.warnings.push(w);
                }
                let mut at: Vec<(usize, LumpedPart)> = Vec::new();
                let mut before: Vec<LumpedPart> = Vec::new();
                for sh in shunts.iter().filter(|s| s.net == *net && !at_stub_end.contains(&s.part.refdes)) {
                    let lp = lumped(&sh.part, false, if sh.across { 0.5 } else { 1.0 }, esr);
                    let d0 = geom::len(geom::sub(*from, sh.at));
                    match at_piece.iter().enumerate().min_by(|a, c| geom::len(geom::sub(a.1 .1, sh.at)).partial_cmp(&geom::len(geom::sub(c.1 .1, sh.at))).unwrap()) {
                        Some((i, e)) if geom::len(geom::sub(e.1, sh.at)) < d0 => at.push((i, lp)),
                        _ => before.push(lp),
                    }
                }
                for lp in before {
                    b.add_part(lp);
                }
                for (_, _, label, ch) in stubs.iter().filter(|s| s.0 == 0 && !s.1) {
                    b.add_branch(label.clone(), ch);
                }
                let mut k = 0usize;
                b.add_path_with(
                    *net,
                    &path,
                    None,
                    |b, step| {
                        for (_, _, label, ch) in stubs.iter().filter(|s| s.1 && s.0 == step) {
                            b.add_branch(label.clone(), ch);
                        }
                    },
                    |b, _| {
                        for (_, lp) in at.iter().filter(|(i, _)| *i == k) {
                            b.add_part(lp.clone());
                        }
                        k += 1;
                        for (_, _, label, ch) in stubs.iter().filter(|s| !s.1 && s.0 == k) {
                            b.add_branch(label.clone(), ch);
                        }
                    },
                );
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
/// terminations, side stubs and open branches.
/// `with_terminations`: include termination resistors (for the eye; loss, return
/// loss and TDR describe the interconnect without them, the termination belongs to
/// the receiving end).
pub fn lane_channel(board: &Board, an: &Analysis, lane: &Lane, field: &HashMap<String, FieldResult>, target: Option<(f64, bool)>, rp: &RfParams, reversed: bool, with_terminations: bool, cd: &ComponentData) -> Result<Channel, String> {
    let diff = lane.diff && target.map(|t| t.1).unwrap_or(true);
    let esr = cd.capacitor.esr_ohm;
    let term: HashSet<&str> = terminations(lane).map(|t| t.part.refdes.as_str()).collect();
    let shunts: Vec<Shunt> = lane.shunts.iter().filter(|s| with_terminations || !term.contains(s.part.refdes.as_str())).cloned().collect();
    let hops = if reversed { reverse_hops(&lane.hops) } else { lane.hops.clone() };
    // the other paths of a fork (lane branches): continued where a side stub reaches their
    // first series part; a branch whose junction is not found hangs at the shared end
    let conts: Vec<(usize, String, Vec<Hop>, String)> = lane
        .branches
        .iter()
        .enumerate()
        .filter_map(|(i, bh)| match (bh.first(), bh.get(1)) {
            (Some(Hop::Net { .. }), Some(Hop::Series { part })) => Some((i, part.refdes.clone(), bh[1..].to_vec(), lane.branch_ends.get(i).cloned().unwrap_or_default())),
            _ => None,
        })
        .collect();
    let build = |skip: &HashSet<usize>| -> Result<(Channel, Vec<usize>), String> {
        let mine: Vec<&(usize, String, Vec<Hop>, String)> = conts.iter().filter(|c| !skip.contains(&c.0)).collect();
        let mut ctx = HopCtx { cd, esr, target, conts: mine.iter().map(|c| (c.1.clone(), c.2.clone(), c.3.clone())).collect(), used: vec![false; mine.len()], notes: vec![] };
        let mut fixed: Vec<Channel> = Vec::new();
        for (i, bh) in lane.branches.iter().enumerate() {
            if conts.iter().any(|c| c.0 == i) && !skip.contains(&i) {
                continue;
            }
            let mut bb = Build::new(board, an, field, rp, diff);
            add_hops(&mut bb, board, an, bh, &shunts, &mut HopCtx { cd, esr, target, conts: vec![], used: vec![], notes: vec![] })?;
            fixed.push(bb.finish(lane.nets[0], "branch".into(), None, target));
        }
        let label = |c: &Channel| format!("open branch {:.1} mm", c.length_mm);
        let mut b = Build::new(board, an, field, rp, diff);
        if !reversed {
            for c in &fixed {
                b.add_branch(label(c), c);
            }
        }
        add_hops(&mut b, board, an, &hops, &shunts, &mut ctx)?;
        if reversed {
            for c in &fixed {
                b.add_branch(label(c), c);
            }
        }
        if !fixed.is_empty() {
            b.warnings.push(format!("the line also continues to another device: modelled as {} open branch(es) at {}", fixed.len(), lane.ends[0].refdes));
        }
        b.warnings.extend(ctx.notes.iter().cloned());
        let unused: Vec<usize> = mine.iter().zip(&ctx.used).filter(|(_, u)| !**u).map(|(c, _)| c.0).collect();
        let partner = lane.partner_nets.first().copied();
        Ok((b.finish(lane.nets[0], lane.name.clone(), partner, target), unused))
    };
    let (ch, unused) = build(&HashSet::new())?;
    if unused.is_empty() {
        return Ok(ch);
    }
    // a fork whose junction is not on the copper of this lane: the old way, at the shared end
    Ok(build(&unused.into_iter().collect())?.0)
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

    /// U1 drives SIG; at 19 mm it forks to R1 (→ net A → J1) and, 1.5 mm away, R2 (→ net B →
    /// J2), both 0 Ω – two population options. Over a ground plane on In1.
    const FORK_PCB_TEMPLATE: &str = r#"(kicad_pcb (version 20241229) (generator "pcbnew")
  (layers (0 "F.Cu" signal) (4 "In1.Cu" signal) (2 "B.Cu" signal))
  (setup (stackup
    (layer "F.Cu" (type "copper") (thickness 0.035))
    (layer "dielectric 1" (type "prepreg") (thickness 0.2) (material "FR4") (epsilon_r 4.3) (loss_tangent 0.02))
    (layer "In1.Cu" (type "copper") (thickness 0.035))
    (layer "dielectric 2" (type "core") (thickness 1.2) (material "FR4") (epsilon_r 4.5) (loss_tangent 0.02))
    (layer "B.Cu" (type "copper") (thickness 0.035))))
  (net 0 "") (net 1 "SIG") (net 2 "A") (net 3 "B") (net 4 "GND")
  (footprint "U" (layer "F.Cu") (at 0 0) (property "Reference" "U1") (property "Value" "FPGA") (attr smd)
    (pad "1" smd rect (at 0 0) (size 0.5 0.5) (layers "F.Cu") (net 1 "SIG")))
  (footprint "R" (layer "F.Cu") (at 21 0) (property "Reference" "R1") (property "Value" "0") (attr smd)
    (pad "1" smd rect (at -0.5 0) (size 0.6 0.6) (layers "F.Cu") (net 1 "SIG"))
    (pad "2" smd rect (at 0.5 0) (size 0.6 0.6) (layers "F.Cu") (net 2 "A")))
  (footprint "R" (layer "F.Cu") (at 21 1.5) (property "Reference" "R2") (property "Value" "0") {attr}
    (pad "1" smd rect (at -0.5 0) (size 0.6 0.6) (layers "F.Cu") (net 1 "SIG"))
    (pad "2" smd rect (at 0.5 0) (size 0.6 0.6) (layers "F.Cu") (net 3 "B")))
  (footprint "J" (layer "F.Cu") (at 40 0) (property "Reference" "J1") (property "Value" "conn") (attr smd)
    (pad "1" smd rect (at 0 0) (size 0.6 0.6) (layers "F.Cu") (net 2 "A")))
  (footprint "J" (layer "F.Cu") (at 40 3) (property "Reference" "J2") (property "Value" "conn") (attr smd)
    (pad "1" smd rect (at 0 0) (size 0.6 0.6) (layers "F.Cu") (net 3 "B")))
  (segment (start 0 0) (end 19 0) (width 0.35) (layer "F.Cu") (net 1))
  (segment (start 19 0) (end 20.5 0) (width 0.35) (layer "F.Cu") (net 1))
  (segment (start 19 0) (end 19 1.5) (width 0.35) (layer "F.Cu") (net 1))
  (segment (start 19 1.5) (end 20.5 1.5) (width 0.35) (layer "F.Cu") (net 1))
  (segment (start 21.5 0) (end 40 0) (width 0.35) (layer "F.Cu") (net 2))
  (segment (start 21.5 1.5) (end 40 3) (width 0.35) (layer "F.Cu") (net 3))
  (zone (net 4) (net_name "GND") (layer "In1.Cu") (filled_polygon (layer "In1.Cu") (pts (xy -5 -5) (xy 45 -5) (xy 45 8) (xy -5 8)))))
"#;

    fn fork_board(r2_dnp: bool) -> Board {
        let attr = if r2_dnp { "(attr smd dnp)" } else { "(attr smd)" };
        crate::board::load_pcb(&FORK_PCB_TEMPLATE.replace("{attr}", attr)).unwrap()
    }

    /// The same fork with U1 behind a series resistor R0 (net SRC → R0 → SIG → fork).
    fn fork_board_behind() -> Board {
        let pcb = FORK_PCB_TEMPLATE.replace("{attr}", "(attr smd)").replace(
            "(footprint \"U\" (layer \"F.Cu\") (at 0 0) (property \"Reference\" \"U1\") (property \"Value\" \"FPGA\") (attr smd)\n    (pad \"1\" smd rect (at 0 0) (size 0.5 0.5) (layers \"F.Cu\") (net 1 \"SIG\")))",
            "(footprint \"U\" (layer \"F.Cu\") (at -6 0) (property \"Reference\" \"U1\") (property \"Value\" \"FPGA\") (attr smd)\n    (pad \"1\" smd rect (at 0 0) (size 0.5 0.5) (layers \"F.Cu\") (net 5 \"SRC\")))\n  (footprint \"R\" (layer \"F.Cu\") (at -1.5 0) (property \"Reference\" \"R0\") (property \"Value\" \"22\") (attr smd)\n    (pad \"1\" smd rect (at -0.5 0) (size 0.6 0.6) (layers \"F.Cu\") (net 5 \"SRC\"))\n    (pad \"2\" smd rect (at 0.5 0) (size 0.6 0.6) (layers \"F.Cu\") (net 1 \"SIG\")))",
        ).replace("(net 4 \"GND\")", "(net 4 \"GND\") (net 5 \"SRC\")").replace(
            "  (segment (start 0 0) (end 19 0)",
            "  (segment (start -6 0) (end -2 0) (width 0.35) (layer \"F.Cu\") (net 5))\n  (segment (start -1 0) (end 0 0) (width 0.35) (layer \"F.Cu\") (net 1))\n  (segment (start 0 0) (end 19 0)",
        );
        crate::board::load_pcb(&pcb).unwrap()
    }

    fn lanes_of(board: &Board) -> (crate::extract::Analysis, Vec<Lane>) {
        let mut params = crate::extract::Params::default();
        params.targets.insert("Default".into(), crate::extract::Target { z: 50.0, diff: false });
        let an = crate::analyze(board, &params);
        let lanes = buses(board, &an, &ComponentData::builtin()).into_iter().flat_map(|b| b.lanes).collect();
        (an, lanes)
    }

    #[test]
    fn unfitted_part_leaves_a_side_stub() {
        let cd = ComponentData::builtin();
        let board = fork_board(true);
        assert!(forks(&board, &cd).is_empty(), "R2 is not fitted: no fork");
        let (an, lanes) = lanes_of(&board);
        let l = lanes.iter().find(|l| l.ends.iter().any(|e| e.refdes == "J1")).expect("lane U1–J1");
        assert_eq!(l.kind, "ic-connector");
        let reaches = |l: &Lane, r: &str| l.ends.iter().any(|e| e.refdes == r);
        assert!(l.branches.is_empty() && !lanes.iter().any(|l| reaches(l, "U1") && reaches(l, "J2")), "R2 connects nothing");
        let ch = lane_channel(&board, &an, l, &HashMap::new(), Some((50.0, false)), &RfParams::default(), false, false, &cd).unwrap();
        let stubs: Vec<&str> = ch.segments.iter().filter(|s| s.kind == "branch").filter_map(|s| s.part.as_deref()).collect();
        assert_eq!(stubs.len(), 1, "{stubs:?}");
        assert!(stubs[0].starts_with("stub 3.") && stubs[0].contains(" mm to R2.1 (not fitted; pad 0402, 0.10 pF"), "{}", stubs[0]);
        // the stub costs a little: compared with the same lane without it (R2 removed)
        let mut bare = board.clone();
        for p in bare.pads.iter_mut().filter(|p| board.footprints[p.fp as usize].reference == "R2") {
            p.net = 3;
        }
        bare.tracks.retain(|t| !(t.net == 1 && (t.a[1] > 0.1 || t.b[1] > 0.1)));
        let (an2, lanes2) = lanes_of(&bare);
        let l2 = lanes2.iter().find(|l| l.ends.iter().any(|e| e.refdes == "J1")).unwrap();
        let ch2 = lane_channel(&bare, &an2, l2, &HashMap::new(), Some((50.0, false)), &RfParams::default(), false, false, &cd).unwrap();
        assert!(!ch2.segments.iter().any(|s| s.kind == "branch"));
        let k = ch.freq.iter().position(|&f| f >= 10e9).unwrap();
        assert!(ch.s21_db[k] < ch2.s21_db[k] - 0.01, "stub {} vs bare {}", ch.s21_db[k], ch2.s21_db[k]);
    }

    #[test]
    fn fork_with_all_alternatives_fitted() {
        let cd = ComponentData::builtin();
        let board = fork_board(false);
        let f = forks(&board, &cd);
        assert_eq!(f.len(), 1, "{f:?}");
        assert_eq!((f[0].net_name.as_str(), f[0].parts.clone(), f[0].towards.clone()), ("SIG", vec!["R1".to_string(), "R2".to_string()], vec![vec!["J1".to_string()], vec!["J2".to_string()]]));
        // two lanes from U1 that share the fork, each named after and made of its own path, with
        // the other path as a branch – never J1 ↔ J2
        let (an, lanes) = lanes_of(&board);
        let id = |n: &str| board.nets.iter().position(|x| x == n).unwrap() as u32;
        fn ends(l: &Lane) -> Vec<&str> {
            l.ends.iter().map(|e| e.refdes.as_str()).collect()
        }
        assert!(!lanes.iter().any(|l| ends(l).contains(&"J1") && ends(l).contains(&"J2")));
        for (far, own, other, part) in [("J1", "A", "B", "R2"), ("J2", "B", "A", "R1")] {
            let l = lanes.iter().find(|l| ends(l) == vec!["U1", far]).unwrap_or_else(|| panic!("lane U1–{far}"));
            assert_eq!((l.name.as_str(), l.name_net, l.kind.as_str()), (own, id(own), "ic-connector"));
            assert_eq!(l.nets, vec![id("SIG"), id(own)], "{far}: only the own path");
            assert_eq!(l.branch_ends, vec![if far == "J1" { "J2" } else { "J1" }]);
            let br_nets: Vec<u32> = l.branches[0].iter().filter_map(|h| if let Hop::Net { net, .. } = h { Some(*net) } else { None }).collect();
            assert!(br_nets.contains(&id(other)) && l.branches[0].iter().any(|h| matches!(h, Hop::Series { part: p } if p.refdes == part)));
        }
        // the lane to J1 carries the other path as a branch from the junction, through R2 to J2
        let l = lanes.iter().find(|l| l.ends.iter().any(|e| e.refdes == "J1")).unwrap();
        let ch = lane_channel(&board, &an, l, &HashMap::new(), Some((50.0, false)), &RfParams::default(), false, false, &cd).unwrap();
        let br: Vec<&str> = ch.segments.iter().filter(|s| s.kind == "branch").filter_map(|s| s.part.as_deref()).collect();
        assert_eq!(br.len(), 1, "{br:?}");
        assert!(br[0].contains("on through R2 to J2"), "{}", br[0]);
        assert!(ch.warnings.iter().any(|w| w.starts_with("possible assembly variant")), "{:?}", ch.warnings);
        // the IC behind a series part: each lane runs through R0 and its own branch of the fork,
        // the other branch leaves after R0 (not at the IC)
        let board = fork_board_behind();
        let (an, lanes) = lanes_of(&board);
        let id = |n: &str| board.nets.iter().position(|x| x == n).unwrap() as u32;
        for (far, own, other) in [("J1", "A", "R2"), ("J2", "B", "R1")] {
            let l = lanes.iter().find(|l| ends(l) == vec!["U1", far]).unwrap_or_else(|| panic!("lane U1–{far}: {:?}", lanes.iter().map(ends).collect::<Vec<_>>()));
            assert_eq!(l.nets, vec![id("SRC"), id("SIG"), id(own)]);
            assert_eq!(l.name, own);
            assert!(matches!(&l.branches[0][..], [Hop::Net { net, .. }, Hop::Series { part }, ..] if *net == id("SIG") && part.refdes == other), "{far}");
            let ch = lane_channel(&board, &an, l, &HashMap::new(), Some((50.0, false)), &RfParams::default(), false, false, &cd).unwrap();
            assert!(ch.segments.iter().filter_map(|s| s.part.as_deref()).any(|p| p.contains(&format!("on through {other} to"))), "{far}: {:?}", ch.warnings);
        }
        // the junction is at 19 mm, not at U1: the branch starts after the first 19 mm of line
        let before: f64 = ch.segments.iter().take_while(|s| s.kind != "branch").filter(|s| s.kind == "line").map(|s| s.len_mm).sum();
        assert!((before - 19.0).abs() < 0.6, "{before}");
    }
}

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
                if letters == p.to_uppercase() && best.is_none_or(|b| p.len() > b.1) {
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

/// Case size of a two-pad part from its footprint name and pads (`kisi_board::package`).
pub fn package_of(board: &Board, fp: usize, a: usize, b: usize) -> &'static str {
    let name = board.footprints.get(fp).map(|f| f.lib.as_str()).unwrap_or("");
    crate::package::chip_package(name, &board.pads[a], &board.pads[b])
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
    /// the pads' indices in `Board::pads` (same order)
    #[serde(skip)]
    pub pad_idx: [usize; 2],
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
    /// copper layers of the pad on `net` (bit per layer, 0 = through-hole)
    #[serde(skip)]
    pub layers: u64,
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
    /// shunts on the partner line's nets (pairs; the N line of the 4-port model)
    #[serde(skip)]
    pub partner_shunts: Vec<Shunt>,
    /// the other paths of a fork on the partner line, from where they leave it, and their devices
    #[serde(skip)]
    pub partner_branches: Vec<Vec<Hop>>,
    #[serde(skip)]
    pub partner_branch_ends: Vec<String>,
    /// two ends if the lane is point to point; more for multi-drop, fewer if open
    pub ends: Vec<EndPin>,
    /// multi-drop lines along one path: the receivers beyond `ends[0]` and `ends[1]` (pins
    /// "U3.5", `ends[2..]`), reached through side stubs of the path – checked per receiver
    pub taps: Vec<String>,
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

/// What a fitted two-pin passive is for a signal channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PartUse {
    /// in series between two signal nets
    Series,
    /// across a differential pair (partner nets)
    Across,
    /// from the signal net at pad `0` or `1` to ground or a supply
    Shunt(usize),
    /// on signal nets without passing the signal on (gain, filter, bias networks)
    Other,
    /// value unreadable ("DNP", "NC"), or between supplies
    Ignore,
}

/// Only parts that pass a signal on are series parts: small series resistors (0–100 Ω, damping /
/// links), AC-coupling capacitors (≥ 1 nF), small inductors; larger resistors and small
/// capacitors are gain, filter or bias networks and end the channel.
fn part_use(board: &Board, partners: &[Option<u32>], role: &str, value: &str, na: u32, nb: u32) -> PartUse {
    let Some(v) = parse_value(value) else { return PartUse::Ignore };
    let (sa, sb) = (is_supply(board, na), is_supply(board, nb));
    let pair = partners.get(na as usize).copied().flatten() == Some(nb);
    let passes = match role {
        "resistor" => v <= 100.0,
        "capacitor" => v >= 1e-9,
        _ => v <= 100e-9,
    };
    match (sa, sb) {
        (false, false) if pair => PartUse::Across,
        (false, false) if passes => PartUse::Series,
        (false, false) => PartUse::Other,
        (true, false) => PartUse::Shunt(1),
        (false, true) => PartUse::Shunt(0),
        (true, true) => PartUse::Ignore,
    }
}

/// The passive of footprint `fp` (role "resistor", "capacitor" or "inductor") between its pads
/// `a` and `b`, with the package parasitics from `data/components.json`.
fn passive_of(board: &Board, cd: &ComponentData, fp: usize, role: &str, a: usize, b: usize) -> Passive {
    let f = &board.footprints[fp];
    let pkg = package_of(board, fp, a, b);
    let pd = cd.package(pkg);
    let kind = match role {
        "capacitor" => 'C',
        "inductor" => 'L',
        _ => 'R',
    };
    Passive {
        refdes: f.reference.clone(),
        value: f.value.clone(),
        kind,
        val: parse_value(&f.value),
        package: pkg.to_string(),
        esl_nh: pd.esl_nh,
        pad_c_pf: pd.pad_c_pf,
        pads: [board.pads[a].at, board.pads[b].at],
        pad_idx: [a, b],
    }
}

/// How a channel of one net treats a part across a differential pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartMode {
    /// single-ended channel: not modelled (noted)
    Single,
    /// odd-mode channel of a pair: R/2, 2·C per line
    Odd,
    /// one line of the pair's 4-port model: not modelled (noted)
    PairLine,
}

/// Shunt parts of one net for its own channel (`kisi net`, nets checked without a lane): fitted
/// two-pin passives from the net to ground or a supply – capacitors with ESR and ESL, resistors,
/// inductors, as in the lanes (`buses`), with the package parasitics from `data/components.json`
/// (estimates) – each at its pad on the net, and parts across the pair by `mode`. Terminations
/// (resistors up to 200 Ω, see `terminations`) are left out as in the lanes: loss, return loss
/// and TDR describe the interconnect. Returns the parts and notes for the channel's warnings.
pub fn net_parts(board: &Board, net: u32, mode: PartMode) -> (Vec<crate::rf::NetPart>, Vec<String>) {
    let cd = ComponentData::builtin();
    let partners = diff_partners(&board.nets);
    let mut fp_pads: Vec<Vec<usize>> = vec![vec![]; board.footprints.len()];
    for (pi, p) in board.pads.iter().enumerate() {
        if p.net != 0 && (p.fp as usize) < fp_pads.len() {
            fp_pads[p.fp as usize].push(pi);
        }
    }
    let (mut parts, mut terms, mut skipped) = (Vec::new(), Vec::new(), Vec::new());
    for (i, f) in board.footprints.iter().enumerate() {
        let role = cd.role(&f.reference);
        if f.dnp || !matches!(role, "resistor" | "capacitor" | "inductor") || fp_pads[i].len() != 2 {
            continue;
        }
        let (pa, pb) = (fp_pads[i][0], fp_pads[i][1]);
        let (na, nb) = (board.pads[pa].net, board.pads[pb].net);
        if na == nb || (na != net && nb != net) {
            continue;
        }
        let (here, there, across) = match part_use(board, &partners, role, &f.value, na, nb) {
            PartUse::Shunt(k) => {
                let (sig, other) = if k == 0 { (pa, pb) } else { (pb, pa) };
                if board.pads[sig].net != net {
                    continue;
                }
                (sig, other, false)
            }
            PartUse::Across if na == net => (pa, pb, true),
            PartUse::Across => (pb, pa, true),
            _ => continue,
        };
        let p = passive_of(board, &cd, i, role, here, there);
        let label = format!("{} {}", p.refdes, p.value);
        if p.kind == 'R' && p.val.is_some_and(|v| v <= 200.0) {
            terms.push(label);
            continue;
        }
        if across && mode != PartMode::Odd {
            skipped.push(label);
            continue;
        }
        let note = format!("{label} ({}{})", p.package, if across { ", across the pair: odd mode" } else { "" });
        parts.push(crate::rf::NetPart {
            refdes: p.refdes.clone(),
            note,
            at: board.pads[here].at,
            layers: pad_layers(board, here),
            part: lumped(&p, false, if across { 0.5 } else { 1.0 }, cd.capacitor.esr_ohm),
        });
    }
    let mut notes = Vec::new();
    if !terms.is_empty() {
        notes.push(format!("termination(s) {} not in the channel: loss, return loss and TDR describe the interconnect, the termination belongs to the receiving end", terms.join(", ")));
    }
    if !skipped.is_empty() {
        notes.push(format!(
            "{} across the pair: not part of the {} model",
            skipped.join(", "),
            if mode == PartMode::PairLine { "4-port" } else { "single-ended" }
        ));
    }
    (parts, notes)
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
        .map(|(i, f)| PartInfo {
            fp: i as u32,
            refdes: f.reference.as_str(),
            role: cd.role(&f.reference).to_string(),
            pads: vec![],
        })
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
    let mk_passive =
        |pt: &PartInfo, a: usize, b: usize| -> Passive { passive_of(board, cd, pt.fp as usize, &pt.role, a, b) };
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
            match part_use(board, &partners, &pt.role, &board.footprints[pt.fp as usize].value, na, nb) {
                // DNP / NC / unreadable: not mounted or unknown, ignore
                PartUse::Ignore => {}
                PartUse::Other => {
                    others.entry(na).or_default().push(pt.refdes.to_string());
                    others.entry(nb).or_default().push(pt.refdes.to_string());
                }
                PartUse::Across => {
                    let part = mk_passive(pt, pa, pb);
                    shunts.entry(na).or_default().push(Shunt {
                        part: part.clone(),
                        net: na,
                        to: nb,
                        across: true,
                        at: board.pads[pa].at,
                        layers: pad_layers(board, pa),
                    });
                    let mut p2 = part;
                    p2.pads.swap(0, 1);
                    shunts.entry(nb).or_default().push(Shunt {
                        part: p2,
                        net: nb,
                        to: na,
                        across: true,
                        at: board.pads[pb].at,
                        layers: pad_layers(board, pb),
                    });
                }
                PartUse::Series => {
                    links.entry(na).or_default().push((nb, k, pa, pb));
                    links.entry(nb).or_default().push((na, k, pb, pa));
                }
                PartUse::Shunt(i) => {
                    let (sig, pad, sup, other_pad) = if i == 1 { (nb, pb, na, pa) } else { (na, pa, nb, pb) };
                    shunts.entry(sig).or_default().push(Shunt {
                        part: mk_passive(pt, pad, other_pad),
                        net: sig,
                        to: sup,
                        across: false,
                        at: board.pads[pad].at,
                        layers: pad_layers(board, pad),
                    });
                }
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
                    ends.entry(net).or_default().push(EndPin {
                        refdes: f.reference.clone(),
                        pin: board.pads[pi].number.clone(),
                        value: f.value.clone(),
                        role: pt.role.clone(),
                        net,
                        at: board.pads[pi].at,
                        layers: pad_layers(board, pi),
                    });
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
        taps: Vec<String>,
    }
    let mut seen: HashSet<u32> = HashSet::new();
    let mut chains: Vec<Chain> = Vec::new();
    let mut starts: Vec<u32> = target_net
        .iter()
        .copied()
        .chain(target_net.iter().filter_map(|&n| partners.get(n as usize).copied().flatten()))
        .collect();
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
            let next: Vec<&(u32, usize, usize, usize)> = links
                .get(&cur)
                .map(|x| x.iter().filter(|l| comp_set.contains(&l.0) && !used_parts.contains(&l.1)).collect())
                .unwrap_or_default();
            if next.len() > 1 {
                simple = false;
            }
            let Some(&&(v, k, pa, pb)) = next.first() else { break };
            hops.push(Hop::Net {
                net: cur,
                from: entry,
                to: board.pads[pa].at,
                from_layers: entry_l,
                to_layers: pad_layers(board, pa),
            });
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
        // the end of the path: of the pins on the last net, the one farthest along the copper
        let cands: Vec<&EndPin> = e
            .iter()
            .filter(|p| p.net == cur && start_pin.as_ref().is_none_or(|s| s.refdes != p.refdes || s.pin != p.pin))
            .collect();
        let last_pin = if cands.len() > 1 {
            let dist = crate::rf::copper_distances(
                board,
                an,
                cur,
                (entry, entry_l),
                &cands.iter().map(|p| (p.at, p.layers)).collect::<Vec<_>>(),
            );
            cands
                .iter()
                .zip(&dist)
                .max_by(|a, b| a.1.unwrap_or(-1.0).total_cmp(&b.1.unwrap_or(-1.0)))
                .map(|x| (*x.0).clone())
        } else {
            cands.first().map(|p| (*p).clone())
        };
        let (last_end, last_l) = last_pin.as_ref().map(|p| (p.at, p.layers)).unwrap_or((entry, entry_l));
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
        // a multi-drop line along one path (no tree through series parts): the start and the end
        // of the path first, the pins of other devices are taps (receivers on side stubs of the
        // path); more pins of the devices at the ends are not receivers of their own
        let mut taps = Vec::new();
        if let (true, true, Some(sp), Some(lp)) = (simple, e.len() > 2, start_pin.as_ref(), last_pin.as_ref()) {
            let same = |a: &EndPin, b: &EndPin| a.refdes == b.refdes && a.pin == b.pin;
            let rest: Vec<EndPin> = e.iter().filter(|p| !same(p, sp) && !same(p, lp)).cloned().collect();
            taps = rest
                .iter()
                .filter(|p| p.refdes != sp.refdes && p.refdes != lp.refdes)
                .map(|p| format!("{}.{}", p.refdes, p.pin))
                .collect();
            e = std::iter::once(sp.clone()).chain(std::iter::once(lp.clone())).chain(rest).collect();
        }
        chains.push(Chain { order, hops, ends: e, kind, has_target, taps });
    }
    // pair chains of differential lines: a chain whose nets are partners of another
    // chain's nets (names ending in P/N or +/-, also inside "Net-(…+)" names)
    let pmap = pair_map(&board.nets);
    let chain_of: HashMap<u32, usize> =
        chains.iter().enumerate().flat_map(|(i, c)| c.order.iter().map(move |&n| (n, i))).collect();
    let mut consumed = vec![false; chains.len()];
    let mut lanes: Vec<Lane> = Vec::new();
    for i in 0..chains.len() {
        if consumed[i] {
            continue;
        }
        let j = chains[i]
            .order
            .iter()
            .filter_map(|&n| pmap.get(&n))
            .filter_map(|p| chain_of.get(p))
            .copied()
            .find(|&j| j != i && !consumed[j]);
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
        let mut psh: Vec<Shunt> = partner
            .map(|q| chains[q].order.iter().flat_map(|n| shunts.get(n).cloned().unwrap_or_default()).collect())
            .unwrap_or_default();
        psh.sort_by(|a, b| a.part.refdes.cmp(&b.part.refdes));
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
            partner_shunts: psh,
            partner_branches: vec![],
            partner_branch_ends: vec![],
            ends: c.ends.clone(),
            taps: c.taps.clone(),
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
            hops.push(Hop::Net {
                net: u,
                from: entry,
                to: board.pads[pa].at,
                from_layers: entry_l,
                to_layers: pad_layers(board, pa),
            });
            hops.push(Hop::Series { part: mk_passive(&parts[k], pa, pb) });
            entry = board.pads[pb].at;
            entry_l = pad_layers(board, pb);
            nets.push(board.pads[pb].net);
        }
        hops.push(Hop::Net { net: to.net, from: entry, to: to.at, from_layers: entry_l, to_layers: to.layers });
        Some((nets, hops))
    };
    let lanes: Vec<Lane> = lanes
        .into_iter()
        .flat_map(|l| split_multidrop(l, &route))
        .map(|l| name_lane(board, &target_net, l))
        .map(guess_direction)
        .collect();
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
                .map(|d| {
                    lanes
                        .iter()
                        .flat_map(|l| &l.ends)
                        .find(|p| &p.refdes == d)
                        .map(|p| p.value.clone())
                        .unwrap_or_default()
                })
                .collect();
            let kind =
                if lanes.iter().all(|l| l.kind == lanes[0].kind) { lanes[0].kind.clone() } else { "mixed".into() };
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
                if na == nb || !passes || is_supply(board, na) || is_supply(board, nb) || partners.get(&na) == Some(&nb)
                {
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
            groups
                .entry((cd.role(&board.footprints[i].reference).to_string(), format!("{v:.4e}")))
                .or_default()
                .push((o, i, v));
        }
        for (_, g) in groups {
            let first = board.footprints[g[0].1].at;
            let alt: Vec<&(u32, usize, f64)> =
                g.iter().filter(|x| geom::len(geom::sub(board.footprints[x.1].at, first)) <= FORK_MM).collect();
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
        let Some(p) = f.partner.as_deref().and_then(|p| by_name.get(p).copied()).filter(|p| listed.contains(p)) else {
            return true;
        };
        let (me, other) = (is_positive(&f.net_name), is_positive(board.net_name(p)));
        if me != other {
            me
        } else {
            f.net < p
        }
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

/// Lane kinds the check judges: IC to IC (with the eye), and lanes ending at a
/// connector – IC to connector and connector to connector (coupons, pass-through boards) – on
/// the board share only (loss, return loss, skew, TDR; no eye). Open lines are listed, not
/// checked; multi-drop lines only along one path (`checked`). Lanes exist only for nets with a
/// target impedance (`buses`).
pub fn checked_kind(kind: &str) -> bool {
    matches!(kind, "ic-ic" | "ic-connector" | "connector-connector")
}

/// Lanes the check judges: the kinds of `checked_kind`, and multi-drop lines along one path with
/// their receivers on side stubs (`Lane::taps`) – the eye at every receiver, the worst counts.
pub fn checked(lane: &Lane) -> bool {
    checked_kind(&lane.kind) || (lane.kind == "multi-drop" && !lane.taps.is_empty())
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
        Hop::Net { net, from, to, from_layers, to_layers } => {
            Hop::Net { net: *net, from: *to, to: *from, from_layers: *to_layers, to_layers: *from_layers }
        }
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
    let parts = |h: &[Hop]| {
        h.iter()
            .filter_map(|x| if let Hop::Series { part } = x { Some(part.refdes.clone()) } else { None })
            .collect::<Vec<_>>()
    };
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
        nl.taps = vec![];
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
                nl.partner_nets = nl
                    .nets
                    .iter()
                    .filter_map(|n| l.nets.iter().position(|x| x == n).and_then(|k| l.partner_nets.get(k).copied()))
                    .collect();
                nl.partner_hops = vec![];
            }
        }
        // the partner's other paths, as for the line itself (the N line of the 4-port model)
        nl.partner_branches = vec![];
        nl.partner_branch_ends = vec![];
        if let (Some(pic), false) = (p_ic, nl.partner_hops.is_empty()) {
            for (k, other) in others.iter().enumerate() {
                let Some(pe) = l.partner_ends.iter().find(|e| e.refdes == other.refdes).filter(|_| k != i) else {
                    continue;
                };
                let Some((_, oh)) = route(pin(pic), pe) else { continue };
                let c = common_steps(&nl.partner_hops, &oh);
                let b = oh[2 * c..].to_vec();
                if b.iter().any(|h| matches!(h, Hop::Series { .. })) {
                    nl.partner_branches.push(b);
                    nl.partner_branch_ends.push(other.refdes.clone());
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
use crate::rf::{Build, Channel, Cut, LumpedPart, RfParams};

fn lumped(p: &Passive, series: bool, scale: f64, esr: f64) -> LumpedPart {
    let v = p.val.unwrap_or(0.0);
    let esl = p.esl_nh * 1e-9;
    let (r, l, c) = match p.kind {
        'C' => (esr, esl, Some(v.max(1e-15))),
        'L' => (0.01, v + esl, None),
        _ => (v, esl, None),
    };
    let label = format!("{} {}{}", p.refdes, p.value, if scale < 1.0 { " (across pair, odd mode)" } else { "" });
    LumpedPart { label, series, kind: p.kind, r: r * scale, l: l * scale, c: c.map(|c| c / scale) }
}

/// Capacitance (F) of the half of SMD pad `pad` that no line of the cascade covers: a track runs to
/// the pad's centre, and the cross-sections of the track inside the pad already have the pad's
/// width (`extract`); the other half – towards the part's body (`toward`: the other pad), or
/// beyond the track's end – is a shunt C = Σ C′·ℓ over the cross-sections of the pad's synthetic
/// track there (C′ = √εeff/(Z0·c0): the 2D field solve or formula of the pad with whatever lies
/// below it – a cut-out in the plane under the pad, no plane at all). None when the pad has no
/// such track (no track enters it) or no impedance.
pub fn pad_shunt_c(board: &Board, an: &Analysis, pad: usize, toward: Option<P>) -> Option<f64> {
    let pd = board.pads.get(pad)?;
    let halves: Vec<usize> =
        (0..board.tracks.len()).filter(|&i| board.tracks[i].pad && board.tracks[i].item as usize == pad).collect();
    if halves.is_empty() {
        return None;
    }
    // a half is covered when a track of the pad's net runs along its middle
    let covered = |ti: usize| {
        let t = &board.tracks[ti];
        let m = geom::lerp(t.a, t.b, 0.5);
        board
            .tracks
            .iter()
            .any(|o| !o.pad && o.net == t.net && o.layer == t.layer && seg_covers(m, o.a, o.b, o.width / 2.0 + 0.02))
    };
    let free: Vec<usize> = halves.iter().copied().filter(|&ti| !covered(ti)).collect();
    let pick = match (free.len(), toward) {
        (0, _) => return Some(0.0),
        (1, _) => free[0],
        (_, Some(q)) => *free.iter().max_by(|&&a, &&b| {
            let d = |ti: usize| geom::dot(geom::sub(board.tracks[ti].b, pd.at), geom::sub(q, pd.at));
            d(a).total_cmp(&d(b))
        })?,
        (_, None) => free[0],
    };
    let len = geom::len(geom::sub(board.tracks[pick].b, board.tracks[pick].a));
    let mut c = 0.0;
    let mut cover = 0.0;
    for pc in an.pieces.chunks_exact(4) {
        if pc[0] as usize != pick {
            continue;
        }
        let xs = &an.cross_sections[pc[3] as usize];
        let (z, e) = (xs.z0?, xs.eeff?);
        let l = (pc[2] - pc[1]) * len * 1e-3;
        c += e.sqrt() / (z * crate::rf::C0) * l;
        cover += pc[2] - pc[1];
    }
    (cover > 0.99).then_some(c)
}

/// Whether segment a → b runs past point `p` within `r` (its foot point inside the segment, not
/// beyond either end).
fn seg_covers(p: P, a: P, b: P, r: f64) -> bool {
    let d = geom::sub(b, a);
    let t = geom::dot(geom::sub(p, a), d) / geom::dot(d, d).max(1e-18);
    (0.0..=1.0).contains(&t) && geom::len(geom::sub(p, geom::add(a, geom::scale(d, t)))) <= r
}

/// The two pad capacitances of a series part: the geometry's ([`pad_shunt_c`], the mean of both
/// pads), else the estimate per package from `data/components.json`.
fn pad_cap(board: &Board, an: &Analysis, p: &Passive) -> (LumpedPart, &'static str) {
    let [a, b] = p.pad_idx;
    let geo =
        match (pad_shunt_c(board, an, a, Some(board.pads[b].at)), pad_shunt_c(board, an, b, Some(board.pads[a].at))) {
            (Some(x), Some(y)) => Some(0.5 * (x + y)),
            _ => None,
        };
    match geo {
        Some(c) => (
            LumpedPart {
                label: format!("{} pad ({:.3} pF, pad geometry)", p.refdes, c * 1e12),
                series: false,
                kind: 'C',
                r: 0.0,
                l: 0.0,
                c: Some(c.max(1e-18)),
            },
            "pad geometry",
        ),
        None => (
            LumpedPart {
                label: format!("{} pad", p.refdes),
                series: false,
                kind: 'C',
                r: 0.0,
                l: 0.0,
                c: Some(p.pad_c_pf.max(1e-3) * 1e-12),
            },
            "estimate",
        ),
    }
}

/// Pad capacitance of a part that is not fitted (its pads stay as copper), at the end of a stub
/// that runs to pad `at_pad`: from the pad's geometry ([`pad_shunt_c`]), else the value per package
/// from `data/components.json` for two-pin parts (an estimate); None for others.
fn unfitted_pad_cap(
    board: &Board,
    an: &Analysis,
    fp: usize,
    at_pad: Option<usize>,
    cd: &ComponentData,
) -> Option<(String, LumpedPart)> {
    let pads: Vec<usize> = (0..board.pads.len()).filter(|&i| board.pads[i].fp as usize == fp).collect();
    if pads.len() != 2 {
        return None;
    }
    let pkg = package_of(board, fp, pads[0], pads[1]);
    let r = &board.footprints[fp].reference;
    if let Some(pi) = at_pad.filter(|p| pads.contains(p)) {
        let other = if pads[0] == pi { pads[1] } else { pads[0] };
        if let Some(c) = pad_shunt_c(board, an, pi, Some(board.pads[other].at)) {
            return Some((
                format!("{pkg}, {:.2} pF from the pad geometry", c * 1e12),
                LumpedPart {
                    label: format!("{r} pad (not fitted)"),
                    series: false,
                    kind: 'C',
                    r: 0.0,
                    l: 0.0,
                    c: Some(c.max(1e-18)),
                },
            ));
        }
    }
    let c = cd.package(pkg).pad_c_pf;
    Some((
        format!("{pkg}, {c:.2} pF estimated"),
        LumpedPart {
            label: format!("{r} pad (not fitted, estimate)"),
            series: false,
            kind: 'C',
            r: 0.0,
            l: 0.0,
            c: Some(c.max(1e-3) * 1e-12),
        },
    ))
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
    /// receivers on side stubs (multi-drop): pins "U3.5"
    taps: HashSet<String>,
}

/// Add the copper and parts of `hops` to the build. Shunts on these nets are placed at the
/// closest point of the path, or at the end of the side stub that leads to them. Side stubs
/// (branches of a hop's net off its path) are open stubs: to unused pads and test points, to
/// the pads of parts that are not fitted (with their pad capacitance), and through fitted
/// parts on to the other path of a fork (a lane branch in `ctx.conts`).
fn add_hops(
    b: &mut Build,
    board: &Board,
    an: &Analysis,
    hops: &[Hop],
    shunts: &[Shunt],
    ctx: &mut HopCtx,
) -> Result<(), String> {
    let esr = ctx.esr;
    for h in hops {
        match h {
            Hop::Net { net, from, to, from_layers, to_layers } => {
                let info = crate::rf::hop_path(board, an, *net, (*from, *from_layers), (*to, *to_layers))
                    .ok_or_else(|| format!("net {} has no copper", board.net_name(*net)))?;
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
                // side stubs: (piece index, after the via in front of that piece?, label, channel,
                // the same stub in the odd mode for a line of a pair's 4-port, the receiver at its
                // end for a tap of a multi-drop line)
                let mut stubs: Vec<(usize, bool, String, Channel, Option<Channel>, Option<String>)> = Vec::new();
                let mut at_stub_end: HashSet<String> = HashSet::new();
                let mut short = 0usize;
                for br in &info.branches {
                    let fp =
                        br.end.refdes.as_ref().and_then(|r| board.footprints.iter().position(|f| &f.reference == r));
                    let refdes = br.end.refdes.clone().unwrap_or_default();
                    let cont = ctx.conts.iter().enumerate().position(|(i, c)| !ctx.used[i] && c.0 == refdes);
                    let shunt = shunts
                        .iter()
                        .find(|s| s.net == *net && s.part.refdes == refdes && !at_stub_end.contains(&refdes));
                    let unfitted = fp.is_some_and(|i| board.footprints[i].dnp);
                    let tap = (br.end.kind == "pad" && ctx.taps.contains(&br.end.label)).then(|| br.end.label.clone());
                    if !(br.modelled || cont.is_some() || unfitted || shunt.is_some() || tap.is_some()) {
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
                            let beyond_next =
                                (sl - pl).signum() == (nl - pl).signum() && (sl - pl).abs() >= (nl - pl).abs();
                            let between = (sl - pl).signum() == (nl - pl).signum()
                                && (sl - pl).abs() < (nl - pl).abs()
                                && sl != pl;
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
                    // a line of a pair's 4-port: the stub also with the pair's odd-mode models (it
                    // pairs up with the partner's stub); not for the other path of a fork
                    let mut odd = (b.pair && cont.is_none()).then(|| {
                        let mut bo = Build::new(board, an, b.field, b.rp, true);
                        bo.add_path_with(*net, &bpath, junction, |_, _| {}, |_, _| {});
                        bo
                    });
                    // what the stub ends in; its label after the build (length, vias)
                    let what = if let Some(i) = cont {
                        ctx.used[i] = true;
                        let (part, rest, far) = ctx.conts[i].clone();
                        add_hops(&mut bb, board, an, &rest, shunts, ctx)?;
                        ctx.notes.push(format!("possible assembly variant: the line forks into {part} and on to {far}, all parts fitted – modelled as an open branch; choose the assembly variant (\"variant\" in <board>.kisi.json or --variant)"));
                        format!("{} and on through {part} to {far} (open, all alternatives fitted)", br.end.label)
                    } else if unfitted {
                        match unfitted_pad_cap(
                            board,
                            an,
                            fp.unwrap(),
                            fp.and_then(|f| {
                                (0..board.pads.len()).find(|&i| {
                                    board.pads[i].fp as usize == f && Some(&board.pads[i].number) == br.end.pad.as_ref()
                                })
                            }),
                            ctx.cd,
                        ) {
                            Some((pad, lp)) => {
                                if let Some(bo) = odd.as_mut() {
                                    bo.add_part(lp.clone());
                                }
                                bb.add_part(lp);
                                format!("{} (not fitted; pad {pad})", br.end.label)
                            }
                            None => format!("{} (not fitted)", br.end.label),
                        }
                    } else if let Some(sh) = shunt {
                        at_stub_end.insert(refdes.clone());
                        let lp = lumped(&sh.part, false, if sh.across { 0.5 } else { 1.0 }, esr);
                        if let Some(bo) = odd.as_mut() {
                            bo.add_part(lp.clone());
                        }
                        bb.add_part(lp);
                        format!("{} {}", sh.part.refdes, sh.part.value)
                    } else {
                        br.end.label.clone()
                    };
                    let ch = bb.finish(*net, "branch".into(), None, ctx.target);
                    let odd = odd.map(|bo| bo.finish(*net, "branch".into(), None, ctx.target));
                    let via = match ch.vias {
                        0 => String::new(),
                        1 => " and a via".to_string(),
                        n => format!(" and {n} vias"),
                    };
                    let label = format!(
                        "{} {:.1} mm{via} to {what}{}",
                        if cont.is_some() { "branch" } else { "stub" },
                        br.len_mm,
                        if tap.is_some() { " (receiver)" } else { "" }
                    );
                    stubs.push((k, after_via, label, ch, odd, tap));
                }
                if !stubs.is_empty() || short > 0 {
                    let mut w = format!(
                        "{} branches: {} side stub(s) included as open stubs",
                        board.net_name(*net),
                        stubs.len()
                    );
                    if short > 0 {
                        w.push_str(&format!(", {short} shorter than {} mm not modelled", crate::rf::BRANCH_MIN_MM));
                    }
                    b.warnings.push(w);
                }
                // shunts on this net: where their pad lies on the path (inside a piece)
                let mut cuts: Vec<Cut> = Vec::new();
                let mut before: Vec<LumpedPart> = Vec::new();
                for sh in shunts.iter().filter(|s| s.net == *net && !at_stub_end.contains(&s.part.refdes)) {
                    let lp = lumped(&sh.part, false, if sh.across { 0.5 } else { 1.0 }, esr);
                    match crate::rf::locate_on_path(board, an, &path, sh.at, sh.layers) {
                        Some((step, frac, _)) => cuts.push(Cut { step, frac, part: lp }),
                        None => before.push(lp),
                    }
                }
                for lp in before {
                    b.add_part(lp);
                }
                let add = |b: &mut Build, label: &String, ch: &Channel, odd: &Option<Channel>, tap: &Option<String>| {
                    b.add_branch_pair(label.clone(), ch, odd.as_ref());
                    if let Some(t) = tap {
                        b.mark_tap(t.clone());
                    }
                };
                for (_, _, label, ch, odd, tap) in stubs.iter().filter(|s| s.0 == 0 && !s.1) {
                    add(b, label, ch, odd, tap);
                }
                let mut k = 0usize;
                b.add_path_cuts(
                    *net,
                    &path,
                    None,
                    &cuts,
                    |b, step| {
                        for (_, _, label, ch, odd, tap) in stubs.iter().filter(|s| s.1 && s.0 == step) {
                            add(b, label, ch, odd, tap);
                        }
                    },
                    |b, _| {
                        k += 1;
                        for (_, _, label, ch, odd, tap) in stubs.iter().filter(|s| !s.1 && s.0 == k) {
                            add(b, label, ch, odd, tap);
                        }
                    },
                );
            }
            Hop::Series { part } => {
                let (pc, src) = pad_cap(board, an, part);
                b.add_part_src(pc.clone(), src);
                b.add_part(lumped(part, true, 1.0, esr));
                b.add_part_src(pc, src);
            }
        }
    }
    Ok(())
}

/// One line of a lane for its cascade: the hops, the shunts on its nets and the other paths of
/// a fork.
struct LineSpec<'l> {
    hops: Vec<Hop>,
    shunts: Vec<Shunt>,
    branches: &'l [Vec<Hop>],
    branch_ends: &'l [String],
    /// device at the lane's first end (where a branch without a junction on the copper hangs)
    end0: &'l str,
    /// receivers on side stubs (multi-drop)
    taps: HashSet<String>,
}

impl Lane {
    /// Terminations of the line (`partner`: the N line of a pair): shunt resistors up to 200 Ω.
    fn line_terms(&self, partner: bool) -> impl Iterator<Item = &Shunt> {
        (if partner { &self.partner_shunts } else { &self.shunts })
            .iter()
            .filter(|s| s.part.kind == 'R' && s.part.val.is_some_and(|v| v <= 200.0))
    }

    /// The line (or its partner) for `line_build`, from `ends[0]` towards `ends[1]`; without the
    /// terminations unless `with_terminations`, and without the shunts in `drop` (by reference).
    fn line_spec(&self, partner: bool, with_terminations: bool, drop: &HashSet<String>) -> LineSpec<'_> {
        let term: HashSet<&str> = self.line_terms(partner).map(|t| t.part.refdes.as_str()).collect();
        let all = if partner { &self.partner_shunts } else { &self.shunts };
        let shunts: Vec<Shunt> = all
            .iter()
            .filter(|s| (with_terminations || !term.contains(s.part.refdes.as_str())) && !drop.contains(&s.part.refdes))
            .cloned()
            .collect();
        let mut hops = if partner { self.partner_hops.clone() } else { self.hops.clone() };
        // the partner's chain may run from the other device: then it is walked backwards
        if partner && self.ends.len() >= 2 {
            if let Some(Hop::Net { from, .. }) = hops.first() {
                let d = |e: &EndPin| geom::len(geom::sub(e.at, *from));
                if d(&self.ends[1]) < d(&self.ends[0]) {
                    hops = reverse_hops(&hops);
                }
            }
        }
        let (branches, branch_ends) = if partner {
            (&self.partner_branches, &self.partner_branch_ends)
        } else {
            (&self.branches, &self.branch_ends)
        };
        let taps = if partner { HashSet::new() } else { self.taps.iter().cloned().collect() };
        LineSpec {
            hops,
            shunts,
            branches,
            branch_ends,
            end0: self.ends.first().map(|e| e.refdes.as_str()).unwrap_or(""),
            taps,
        }
    }
}

/// The cascade of one line of a lane (not finished): series parts, shunts, side stubs and the
/// other paths of a fork as open branches. `diff`: odd-mode line models (the 2-port of a pair);
/// `pair`: also the via models with the partner via (one line of the 4-port).
fn line_build<'a>(
    board: &'a Board,
    an: &'a Analysis,
    spec: &LineSpec,
    field: &'a HashMap<String, FieldResult>,
    target: Option<(f64, bool)>,
    rp: &'a RfParams,
    reversed: bool,
    cd: &ComponentData,
    diff: bool,
    pair: bool,
) -> Result<Build<'a>, String> {
    let esr = cd.capacitor.esr_ohm;
    let shunts = &spec.shunts;
    let hops = if reversed { reverse_hops(&spec.hops) } else { spec.hops.clone() };
    // the other paths of a fork (lane branches): continued where a side stub reaches their
    // first series part; a branch whose junction is not found hangs at the shared end
    let conts: Vec<(usize, String, Vec<Hop>, String)> = spec
        .branches
        .iter()
        .enumerate()
        .filter_map(|(i, bh)| match (bh.first(), bh.get(1)) {
            (Some(Hop::Net { .. }), Some(Hop::Series { part })) => {
                Some((i, part.refdes.clone(), bh[1..].to_vec(), spec.branch_ends.get(i).cloned().unwrap_or_default()))
            }
            _ => None,
        })
        .collect();
    let build = |skip: &HashSet<usize>| -> Result<(Build<'a>, Vec<usize>), String> {
        let mine: Vec<&(usize, String, Vec<Hop>, String)> = conts.iter().filter(|c| !skip.contains(&c.0)).collect();
        let mut ctx = HopCtx {
            cd,
            esr,
            target,
            conts: mine.iter().map(|c| (c.1.clone(), c.2.clone(), c.3.clone())).collect(),
            used: vec![false; mine.len()],
            notes: vec![],
            taps: spec.taps.clone(),
        };
        let mut fixed: Vec<Channel> = Vec::new();
        for (i, bh) in spec.branches.iter().enumerate() {
            if conts.iter().any(|c| c.0 == i) && !skip.contains(&i) {
                continue;
            }
            let mut bb = Build::new(board, an, field, rp, diff);
            add_hops(
                &mut bb,
                board,
                an,
                bh,
                shunts,
                &mut HopCtx { cd, esr, target, conts: vec![], used: vec![], notes: vec![], taps: HashSet::new() },
            )?;
            fixed.push(bb.finish(0, "branch".into(), None, target));
        }
        let label = |c: &Channel| format!("open branch {:.1} mm", c.length_mm);
        let mut b = Build::new(board, an, field, rp, diff);
        b.pair = pair;
        if !reversed {
            for c in &fixed {
                b.add_branch(label(c), c);
            }
        }
        add_hops(&mut b, board, an, &hops, shunts, &mut ctx)?;
        if reversed {
            for c in &fixed {
                b.add_branch(label(c), c);
            }
        }
        if !fixed.is_empty() {
            b.warnings.push(format!(
                "the line also continues to another device: modelled as {} open branch(es) at {}",
                fixed.len(),
                spec.end0
            ));
        }
        b.warnings.extend(ctx.notes.iter().cloned());
        let unused: Vec<usize> = mine.iter().zip(&ctx.used).filter(|(_, u)| !**u).map(|(c, _)| c.0).collect();
        Ok((b, unused))
    };
    let (b, unused) = build(&HashSet::new())?;
    if unused.is_empty() {
        return Ok(b);
    }
    // a fork whose junction is not on the copper of this lane: the old way, at the shared end
    Ok(build(&unused.into_iter().collect())?.0)
}

/// Channel of a lane from `ends[0]` to `ends[1]` (or reversed), with series parts,
/// terminations, side stubs and open branches; a pair in the odd mode (its 4-port: `lane_pair`).
/// `with_terminations`: include termination resistors (for the eye; loss, return
/// loss and TDR describe the interconnect without them, the termination belongs to
/// the receiving end).
pub fn lane_channel(
    board: &Board,
    an: &Analysis,
    lane: &Lane,
    field: &HashMap<String, FieldResult>,
    target: Option<(f64, bool)>,
    rp: &RfParams,
    reversed: bool,
    with_terminations: bool,
    cd: &ComponentData,
) -> Result<Channel, String> {
    let diff = lane.diff && target.map(|t| t.1).unwrap_or(true);
    let spec = lane.line_spec(false, with_terminations, &HashSet::new());
    let b = line_build(board, an, &spec, field, target, rp, reversed, cd, diff, false)?;
    Ok(b.finish(lane.nets[0], lane.name.clone(), lane.partner_nets.first().copied(), target))
}

/// The 4-port of a pair's lane: the P line along the lane's hops, the N line along the
/// partner's, each with its own parts, stubs and vias, aligned into coupled sections with the
/// modal line models and via pairs (`rf::mixed`). `drop`: shunts left out by reference (the
/// terminations used as the receiver's). An error when the partner's path is not known.
pub fn lane_pair(
    board: &Board,
    an: &Analysis,
    lane: &Lane,
    field: &HashMap<String, FieldResult>,
    target: Option<(f64, bool)>,
    rp: &RfParams,
    reversed: bool,
    with_terminations: bool,
    drop: &HashSet<String>,
    cd: &ComponentData,
) -> Result<crate::lanenet::PairNet, String> {
    if !lane.diff || lane.partner_hops.is_empty() {
        return Err("the partner line's path is not known".into());
    }
    let bp = line_build(
        board,
        an,
        &lane.line_spec(false, with_terminations, drop),
        field,
        target,
        rp,
        reversed,
        cd,
        false,
        true,
    )?;
    let bn = line_build(
        board,
        an,
        &lane.line_spec(true, with_terminations, drop),
        field,
        target,
        rp,
        reversed,
        cd,
        false,
        true,
    )?;
    Ok(crate::rf::pair_net(board, an, field, target, rp, bp, bn))
}

/// The termination resistors of a lane next to its receiver: they act as the receiver's
/// termination in the eye (and in the export's test bench) and are left out of the board's block.
pub struct RxTerms {
    /// the receiver's termination per line (Ω) when there are such resistors
    pub r: Option<f64>,
    pub note: Option<String>,
    /// their references (P line and, for a pair's 4-port, N line)
    pub drop: HashSet<String>,
    /// terminations elsewhere on the lane (source side): the board's block includes them
    pub others: bool,
}

/// Terminations of `lane` next to its receiver (`reversed`: transmit from `ends[1]`); `pair`: the
/// N line's own terminations count too (the 4-port).
pub fn rx_terminations(lane: &Lane, reversed: bool, pair: bool) -> RxTerms {
    let rx_end = if reversed { 0 } else { 1 };
    let (rx_pos, tx_pos) = (lane.ends[rx_end].at, lane.ends[1 - rx_end].at);
    let near_rx = |t: &&Shunt| geom::len(geom::sub(t.at, rx_pos)) < geom::len(geom::sub(t.at, tx_pos));
    let at_rx: Vec<&Shunt> = terminations(lane).filter(near_rx).collect();
    let at_rx_n: Vec<&Shunt> = if pair { lane.line_terms(true).filter(near_rx).collect() } else { vec![] };
    let others = terminations(lane).count() > at_rx.len() || (pair && lane.line_terms(true).count() > at_rx_n.len());
    let (mut r, mut note) = (None, None);
    if !at_rx.is_empty() {
        let g: f64 =
            at_rx.iter().map(|t| 1.0 / (t.part.val.unwrap_or(1e9).max(1e-3) * if t.across { 0.5 } else { 1.0 })).sum();
        r = Some(1.0 / g);
        note = Some(format!(
            "termination {} at {} used as the receiver termination ({:.0} Ω per line)",
            at_rx.iter().map(|t| t.part.refdes.as_str()).collect::<Vec<_>>().join(", "),
            lane.ends[rx_end].refdes,
            1.0 / g
        ));
    }
    let drop = at_rx.iter().chain(&at_rx_n).map(|t| t.part.refdes.clone()).collect();
    RxTerms { r, note, drop, others }
}

/// The builds of a lane's board block as the eye's network uses it (terminations next to the
/// receiver left out, `rx_terminations`): both lines of a pair, or one line (a pair without its
/// partner's path in the odd mode).
// a handful of short-lived values: boxing the large variant would only add indirection
#[allow(clippy::large_enum_variant)]
pub(crate) enum LaneBuilds<'a> {
    Single { b: Build<'a>, odd: bool },
    Pair { bp: Build<'a>, bn: Build<'a>, z_ref: f64 },
}

pub(crate) struct LaneBuildSet<'a> {
    pub builds: LaneBuilds<'a>,
    pub notes: Vec<String>,
    /// one-way static delay of the lane (ps; a pair: its odd mode)
    pub delay_ps: f64,
}

/// `LaneBuilds` of a lane, for the circuit export (`kisi_si::export`).
pub(crate) fn lane_builds<'a>(
    board: &'a Board,
    an: &'a Analysis,
    field: &'a HashMap<String, FieldResult>,
    lane: &Lane,
    target: Option<(f64, bool)>,
    rp: &'a RfParams,
    reversed: bool,
    cd: &ComponentData,
) -> Result<LaneBuildSet<'a>, String> {
    if lane.ends.len() < 2 {
        return Err("the lane has no two ends".into());
    }
    let diff = lane.diff && target.map(|t| t.1).unwrap_or(true);
    let pair = diff && !lane.partner_hops.is_empty();
    let rt = rx_terminations(lane, reversed, pair);
    let mut notes: Vec<String> = rt.note.iter().cloned().collect();
    let builds = if pair {
        let bp = line_build(
            board,
            an,
            &lane.line_spec(false, rt.others, &rt.drop),
            field,
            target,
            rp,
            reversed,
            cd,
            false,
            true,
        )?;
        let bn = line_build(
            board,
            an,
            &lane.line_spec(true, rt.others, &rt.drop),
            field,
            target,
            rp,
            reversed,
            cd,
            false,
            true,
        )?;
        let z_ref = rp.z_ref.unwrap_or(match target {
            Some((z, true)) => z / 2.0,
            Some((z, false)) => z,
            _ => 50.0,
        });
        LaneBuilds::Pair { bp, bn, z_ref }
    } else {
        if diff {
            notes.push("pair computed in the odd mode (2-port): the partner line's path is not known".into());
        }
        LaneBuilds::Single {
            b: line_build(
                board,
                an,
                &lane.line_spec(false, rt.others, &rt.drop),
                field,
                target,
                rp,
                reversed,
                cd,
                diff,
                false,
            )?,
            odd: diff,
        }
    };
    let delay_ps =
        lane_channel(board, an, lane, field, target, &RfParams { npts: 10, ..rp.clone() }, reversed, false, cd)
            .map(|c| c.delay_ps)
            .unwrap_or(0.0);
    Ok(LaneBuildSet { builds, notes, delay_ps })
}

/// Result of evaluating a lane: the interconnect channel (without terminations; a pair as the
/// differential mode of its 4-port), the eye (IC to IC, and to a connector with the connector end
/// model) and the skew to the partner line.
pub struct LaneEval {
    pub ch: Channel,
    pub eye: Option<Result<crate::eye::Eye, String>>,
    /// receiver model used for the eye (after merging terminations at the receiver)
    pub rx_used: Option<crate::eye::EndModel>,
    pub rx_note: Option<String>,
    /// transmitter model used for the eye (the connector end model when the connector sends)
    pub tx_used: Option<crate::eye::EndModel>,
    /// the lane ends at a connector: what sits there in the eye (termination or driver)
    pub end_note: Option<String>,
    /// a channel behind the connector (lossy line or Touchstone file) is in the eye
    pub rest_applied: bool,
    /// (skew ps, skew mm, partner name, uncertain)
    pub skew: Option<(f64, f64, String, bool)>,
    /// how the channel is modelled: "single-ended", "4-port" (a pair: both lines on their own
    /// paths, coupling, mode conversion) or "odd mode" (a pair whose partner path is not known)
    pub model: &'static str,
    /// pairs with the 4-port: mode conversion Scd21 per frequency (dB, differential in at the
    /// transmitter, common mode out at the receiver)
    pub scd21_db: Option<Vec<f64>>,
    /// multi-drop lines: the eye at every receiver, the lane's own receiver first (`eye` is the
    /// worst of them)
    pub receivers: Vec<crate::eye::RxEye>,
}

/// Evaluate a lane. `reversed`: transmit from `ends[1]` to `ends[0]` (use
/// `lane.rx_end == 0` for the guessed direction). `eye`: bit rate, rise, PRBS
/// order, transmitter, receiver of the profile. `conn_end`: for a lane ending at a
/// connector, the model there (termination when the connector receives, driver when it
/// sends; `None` = the profile's receiver / transmitter) and the channel behind it (a lossy
/// line, or the Touchstone file `rest_ts` that `conn_end.touchstone` names).
///
/// The channel and the eye come from the lane's network (`lanenet`): a pair as the 4-port of
/// both lines (P and N on their own paths, with their own parts, stubs and vias, coupled where
/// they run together), driven differentially; the channel behind a connector and the ends are
/// blocks of the same network.
pub fn lane_eval(
    board: &Board,
    an: &Analysis,
    field: &HashMap<String, FieldResult>,
    lane: &Lane,
    target: Option<(f64, bool)>,
    rp: &RfParams,
    eye: Option<(f64, f64, u32, Option<crate::eye::EndModel>, Option<crate::eye::EndModel>)>,
    reversed: bool,
    conn_end: Option<&crate::eye::ConnEnd>,
    rest_ts: Option<&crate::rf::Touchstone>,
    xt_lanes: &[crate::lanenet::XtLane],
    cd: &ComponentData,
) -> Result<LaneEval, String> {
    use crate::lanenet::{apply_pair, receivers_transfer, rest_block, Block};
    let mut ch = lane_channel(board, an, lane, field, target, rp, reversed, false, cd)?;
    let none = HashSet::new();
    let (mut model, mut scd21_db, mut pair) = (if ch.diff { "odd mode" } else { "single-ended" }, None, None);
    if !ch.diff {
        let s2: Vec<Vec<crate::rf::Cx>> =
            (0..ch.freq.len()).map(|k| vec![ch.s11[k], ch.s21[k], ch.s21[k], ch.s22[k]]).collect();
        ch.warnings.extend(crate::lanenet::passivity_check(&ch.freq, &s2, 2));
    } else {
        // a pair: the 4-port of both lines
        match lane_pair(board, an, lane, field, target, rp, reversed, false, &none, cd) {
            Ok(p) => {
                let mm = p.mixed();
                let s4: Vec<Vec<crate::rf::Cx>> =
                    p.t.iter().map(|t| crate::rf::s_of(t, p.z_ref).iter().flatten().copied().collect()).collect();
                ch.warnings.extend(crate::lanenet::passivity_check(&ch.freq, &s4, 4));
                apply_pair(&mut ch, &mm, rp.rise);
                scd21_db = Some(mm.iter().map(|m| 20.0 * m[3][0].abs().max(1e-12).log10()).collect());
                for w in &p.warnings {
                    if !ch.warnings.contains(w) {
                        ch.warnings.push(w.clone());
                    }
                }
                model = "4-port";
                pair = Some(p);
            }
            Err(e) => ch.warnings.push(format!("pair computed in the odd mode (2-port): {e}")),
        }
    }
    // multi-drop: receivers that are not on a side stub of the path do not load it
    let missing: Vec<&str> =
        lane.taps.iter().filter(|t| !ch.taps.iter().any(|c| &c.label == *t)).map(|t| t.as_str()).collect();
    if !missing.is_empty() {
        ch.warnings.push(format!(
            "receiver(s) {} not found on a side stub of the path: not loading the line",
            missing.join(", ")
        ));
    }
    let mut out = LaneEval {
        ch,
        eye: None,
        rx_used: None,
        rx_note: None,
        tx_used: None,
        end_note: None,
        rest_applied: false,
        skew: None,
        model,
        scd21_db,
        receivers: vec![],
    };
    let with_eye =
        matches!(lane.kind.as_str(), "ic-ic" | "ic-connector") || (lane.kind == "multi-drop" && !lane.taps.is_empty());
    if let (Some((bitrate, rise, prbs, mut tx, mut rx)), true) = (eye, with_eye) {
        let rx_end = if reversed { 0 } else { 1 };
        // a lane ending at a connector: the rest of the channel is off the board; the eye
        // uses the connector end model in place of the receiver or the transmitter there
        let mut conn_side: Option<bool> = None; // the connector receives (true) or sends
        let mut conn_pins: Vec<String> = Vec::new(); // the lane's pins there (P, N)
        if lane.kind == "ic-connector" {
            if let Some(ci) = lane.ends.iter().position(|e| e.role == "connector") {
                let receives = ci == rx_end;
                conn_side = Some(receives);
                conn_pins = connector_pins(lane, &lane.ends[ci].refdes);
                let prof = if receives { rx } else { tx };
                let m = match conn_end.map(|c| c.resolve(prof, target)) {
                    Some(Ok(m)) => m,
                    Some(Err(e)) => {
                        out.end_note = Some(e);
                        prof
                    }
                    None => prof,
                };
                let what = match conn_end.map(|c| c.model.as_str()) {
                    None | Some("") | Some("profile") => {
                        format!("the profile's {}", if receives { "receiver" } else { "transmitter" })
                    }
                    Some("matched") => "matched to the target".to_string(),
                    Some("high_impedance") => {
                        if receives {
                            "high impedance".to_string()
                        } else {
                            "ideal source".to_string()
                        }
                    }
                    Some(_) => "custom".to_string(),
                };
                let mm = m.unwrap_or_default();
                let err = out.end_note.take().map(|e| format!(" – {e}; the profile's end is used")).unwrap_or_default();
                out.end_note = Some(
                    format!(
                        "{} at connector {}: {} ({}{}{})",
                        if receives { "termination" } else { "driver" },
                        lane.ends[ci].refdes,
                        what,
                        if mm.r > 0.0 {
                            format!("{:.0} Ω per line", mm.r)
                        } else if receives {
                            "no termination".into()
                        } else {
                            "0 Ω".into()
                        },
                        if mm.c_pf > 0.0 { format!(", {:.2} pF", mm.c_pf) } else { String::new() },
                        if mm.l_nh > 0.0 { format!(", {:.2} nH", mm.l_nh) } else { String::new() },
                    ) + &err,
                );
                if receives {
                    rx = m;
                } else {
                    tx = m;
                }
            }
        }
        out.tx_used = tx;
        // a termination resistor next to the receiver acts as the receiver's termination (R/2 per
        // line in odd mode when it sits across the pair); terminations elsewhere (source side)
        // stay in the channel
        let rt = rx_terminations(lane, reversed, pair.is_some());
        let mut rx_model = rx;
        if let Some(r) = rt.r {
            rx_model = Some(crate::eye::EndModel { r, ..rx.unwrap_or_default() });
            out.rx_note = rt.note.clone();
        }
        out.rx_used = rx_model;
        let (others, drop) = (rt.others, rt.drop);
        // the board's block: the pair's 4-port, else the cascade – with the terminations that do
        // not sit at the receiver (source side)
        let lines = if pair.is_some() { 2 } else { 1 };
        let board_block: Result<Block, String> = match (&pair, others) {
            (Some(p), false) => Ok(p.block()),
            (Some(_), true) => {
                lane_pair(board, an, lane, field, target, rp, reversed, true, &drop, cd).map(|p| p.block())
            }
            (None, false) => Ok(line_block(&out.ch)),
            (None, true) => {
                let mut l2 = lane.clone();
                l2.shunts.retain(|s| !drop.contains(&s.part.refdes));
                lane_channel(board, an, &l2, field, target, rp, reversed, true, cd).map(|c| line_block(&c))
            }
        };
        // the channel behind the connector (a Touchstone file or a simple lossy line), on the
        // connector's side: after the board when the connector receives, before it when it sends
        let rest = conn_end.filter(|c| c.has_rest()).zip(conn_side);
        let mut rest_note = String::new();
        let z0 = out.ch.z_ref;
        let freq = out.ch.freq.clone();
        let blocks = board_block.map(|bb| {
            let Some((ce, receives)) = rest else { return vec![bb] };
            if let (Some(tr), None) = (&ce.touchstone, rest_ts) {
                rest_note = format!(
                    "; behind the connector: Touchstone file {} not loaded – the eye covers the board share",
                    tr.file
                );
                return vec![bb];
            }
            match rest_block(
                ce,
                rest_ts,
                &freq,
                lines,
                out.ch.diff,
                bitrate / 2.0,
                z0,
                &conn_pins[..lines.min(conn_pins.len())],
                xt_lanes,
            ) {
                Ok((rb, note)) => {
                    rest_note = match (&ce.touchstone, rest_ts) {
                        (Some(_), Some(ts)) => format!(
                            "; behind the connector {} ({}-port{})",
                            ts.name,
                            ts.nports,
                            if note.is_empty() { String::new() } else { format!("; {note}") }
                        ),
                        _ => format!("; behind the connector {note}"),
                    };
                    out.rest_applied = true;
                    if receives {
                        vec![bb, rb]
                    } else {
                        vec![rb.flipped(), bb]
                    }
                }
                Err(e) => {
                    rest_note = format!("; behind the connector: {e} – not used, the eye covers the board share");
                    vec![bb]
                }
            }
        });
        if let Some(n) = out.end_note.as_mut() {
            n.push_str(&rest_note);
        }
        let d = crate::eye::EndModel { r: z0, c_pf: 0.0, l_nh: 0.0 };
        let (txm, rxm) = (tx.unwrap_or(d), rx_model.unwrap_or(d));
        let ep = crate::eye::EyeParams { bitrate, rise, prbs, tx, rx: rx_model, ..Default::default() };
        let delay = out.ch.delay_ps;
        // every receiver (multi-drop: the taps as well, all loading the line); the eye of the lane
        // is the worst one
        let k_nyq = ((bitrate / 2.0 / freq[0]).round() as usize).clamp(1, freq.len()) - 1;
        let rx_pin = format!("{}.{}", lane.ends[rx_end].refdes, lane.ends[rx_end].pin);
        let res: Result<Vec<(String, crate::eye::Eye, f64)>, String> = blocks.and_then(|bl| {
            receivers_transfer(&freq, &bl, lines, &txm, &rxm)?
                .into_iter()
                .map(|r| {
                    let g = 20.0 * r.h[k_nyq].abs().max(1e-12).log10();
                    crate::eye::eye_with_xt(&freq, &r.h, Some(r.dc), delay, &ep, &r.xt)
                        .map(|e| (if r.label.is_empty() { rx_pin.clone() } else { r.label }, e, g))
                })
                .collect()
        });
        out.eye = Some(res.map(|all| {
            if all.len() > 1 {
                out.receivers = all
                    .iter()
                    .map(|(pin, e, g)| crate::eye::RxEye {
                        pin: pin.clone(),
                        eye_height: e.height,
                        eye_width_ui: e.width_ps / e.ui_ps,
                        gain_nyq_db: *g,
                    })
                    .collect();
            }
            let worst = all.into_iter().min_by(|a, b| a.1.height.total_cmp(&b.1.height)).unwrap();
            let mut e = worst.1;
            if !out.receivers.is_empty() {
                e.warnings.insert(0, format!("eye at {} – the worst of {} receivers", worst.0, out.receivers.len()));
            }
            e
        }));
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

/// The lane's pins at connector `refdes`: the line's, then the partner's (pairs).
pub fn connector_pins(lane: &Lane, refdes: &str) -> Vec<String> {
    let own = lane.ends.iter().find(|e| e.refdes == refdes).map(|e| e.pin.clone());
    let partner = lane.partner_ends.iter().find(|e| e.refdes == refdes).map(|e| e.pin.clone());
    own.into_iter().chain(partner.filter(|_| lane.diff)).collect()
}

/// The lanes ending at connector `refdes` (possible aggressors in one N-port file behind it):
/// their pins there and whether the connector is their transmitting end (the guessed direction,
/// `Lane::rx_end`).
pub fn connector_lanes<'l>(lanes: impl IntoIterator<Item = &'l Lane>, refdes: &str) -> Vec<crate::lanenet::XtLane> {
    lanes
        .into_iter()
        .filter(|l| l.kind == "ic-connector")
        .filter_map(|l| {
            let ci = l.ends.iter().position(|e| e.refdes == refdes && e.role == "connector")?;
            Some(crate::lanenet::XtLane {
                label: l.name.clone(),
                pins: connector_pins(l, refdes),
                sends: ci != l.rx_end,
            })
        })
        .collect()
}

/// The board's block of a single line: the cascade, or with receivers on side stubs the path in
/// pieces with the stubs (multi-drop).
fn line_block(c: &Channel) -> crate::lanenet::Block {
    if c.taps.is_empty() {
        crate::lanenet::Block::Two { m: c.abcd.clone(), dc: c.abcd_dc }
    } else {
        crate::lanenet::Block::Line { pieces: c.pieces.clone(), taps: c.taps.clone() }
    }
}

/// Channel check of one lane: loss, return loss, stubs, skew and the eye – from IC to IC with
/// the profile's ends, to a connector with the connector end model (`conn_end`; the eye of the
/// board share, `eye_board_share`). `reversed`: transmit from `ends[1]` to `ends[0]` (use
/// `lane.rx_end == 0` for the guessed direction).
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
    conn_end: Option<&crate::eye::ConnEnd>,
    rest_ts: Option<&crate::rf::Touchstone>,
    xt_lanes: &[crate::lanenet::XtLane],
    cd: &ComponentData,
) -> crate::eye::CheckRow {
    let mut row = crate::eye::empty_row(lane.nets[0], lane.name.clone(), gbps);
    row.eye_board_share = lane.kind == "ic-connector";
    let rp = RfParams { fmax: rp.fmax.max(gbps * 1e9 * 1.25), ..rp.clone() };
    let (tx, rx, rise) = ends;
    match lane_eval(
        board,
        an,
        field,
        lane,
        target,
        &rp,
        Some((gbps * 1e9, rise.unwrap_or(rp.rise), 7, tx, rx)),
        reversed,
        conn_end,
        rest_ts,
        xt_lanes,
        cd,
    ) {
        Ok(ev) => {
            // without a channel model behind the connector the eye covers the board share only
            row.eye_board_share = lane.kind == "ic-connector" && !ev.rest_applied;
            crate::eye::fill_row(
                &mut row,
                &ev.ch,
                gbps,
                rise.unwrap_or(rp.rise),
                (ev.tx_used.or(tx), ev.rx_used.or(rx)),
                false,
            );
            row.model = ev.model;
            row.mode_conversion_db =
                ev.scd21_db.as_ref().map(|c| crate::eye::worst_up_to(&ev.ch.freq, c, 1.5 * gbps * 0.5e9));
            row.receivers = ev.receivers.clone();
            match ev.eye {
                Some(Ok(e)) => {
                    row.eye = true;
                    row.eye_height = e.height;
                    row.eye_width_ui = e.width_ps / e.ui_ps;
                    row.xt_peak = e.xt.as_ref().map(|x| x.peak);
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
        for (s, v) in [
            ("0", 0.0),
            ("22", 22.0),
            ("4k7", 4700.0),
            ("49.9R", 49.9),
            ("220nF", 220e-9),
            ("100n", 100e-9),
            ("4u7", 4.7e-6),
            ("10µF/6.3V", 10e-6),
            ("1M", 1e6),
            ("R10", 0.1),
        ] {
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

    /// A 40 mm 50 Ω line from connector J1 to connector J2 with a 100 nF capacitor C1 (KiCad's
    /// 0805 land pattern) from the line to GND at 20 mm – a CAPZ coupon.
    fn capz_board(with_cap: bool) -> Board {
        let cap = r#"  (footprint "Capacitor_SMD:C_0805_2012Metric" (layer "F.Cu") (at 20 0.95) (property "Reference" "C1") (property "Value" "100nF 16V X7R") (attr smd)
    (pad "1" smd roundrect (at 0 -0.95) (size 1.45 1.0) (roundrect_rratio 0.25) (layers "F.Cu" "F.Paste" "F.Mask") (net 1 "SIG"))
    (pad "2" smd roundrect (at 0 0.95) (size 1.45 1.0) (roundrect_rratio 0.25) (layers "F.Cu" "F.Paste" "F.Mask") (net 4 "GND")))
"#;
        let pcb = format!(
            r#"(kicad_pcb (version 20241229) (generator "pcbnew")
  (layers (0 "F.Cu" signal) (4 "In1.Cu" signal) (2 "B.Cu" signal))
  (setup (stackup
    (layer "F.Cu" (type "copper") (thickness 0.035))
    (layer "dielectric 1" (type "prepreg") (thickness 0.2) (material "FR4") (epsilon_r 4.3) (loss_tangent 0.02))
    (layer "In1.Cu" (type "copper") (thickness 0.035))
    (layer "dielectric 2" (type "core") (thickness 1.2) (material "FR4") (epsilon_r 4.5) (loss_tangent 0.02))
    (layer "B.Cu" (type "copper") (thickness 0.035))))
  (net 0 "") (net 1 "SIG") (net 4 "GND")
  (footprint "J" (layer "F.Cu") (at 0 0) (property "Reference" "J1") (property "Value" "SMA") (attr smd)
    (pad "1" smd rect (at 0 0) (size 0.6 0.35) (layers "F.Cu") (net 1 "SIG")))
  (footprint "J" (layer "F.Cu") (at 40 0) (property "Reference" "J2") (property "Value" "SMA") (attr smd)
    (pad "1" smd rect (at 0 0) (size 0.6 0.35) (layers "F.Cu") (net 1 "SIG")))
{}  (segment (start 0 0) (end 40 0) (width 0.35) (layer "F.Cu") (net 1))
  (zone (net 4) (net_name "GND") (layer "In1.Cu") (filled_polygon (layer "In1.Cu") (pts (xy -5 -5) (xy 45 -5) (xy 45 8) (xy -5 8)))))
"#,
            if with_cap { cap } else { "" }
        );
        crate::board::load_pcb(&pcb).unwrap()
    }

    /// The shunt capacitor is in the net's channel (`kisi net`) and in the connector–connector
    /// lane, at its pad (20 mm), with the 0805 parasitics (ESL 0.6 nH); without it S21 is flat.
    #[test]
    fn shunt_capacitor_on_a_signal_net() {
        let cd = ComponentData::builtin();
        let rp = RfParams { fmax: 10e9, npts: 100, ..RfParams::default() };
        let board = capz_board(true);
        let (an, lanes) = lanes_of(&board);
        let l = lanes.iter().find(|l| l.ends.len() == 2).expect("lane J1–J2");
        assert_eq!(l.kind, "connector-connector");
        assert_eq!(l.shunts.len(), 1);
        let p = &l.shunts[0].part;
        assert_eq!(
            (p.refdes.as_str(), p.package.as_str(), p.esl_nh),
            ("C1", "0805", 0.6),
            "KiCad's 0805 (pitch 1.9 mm) is an 0805"
        );
        let shunt_at = |ch: &Channel| -> Vec<(String, f64)> {
            ch.segments
                .iter()
                .filter(|s| s.kind == "shunt")
                .map(|s| (s.part.clone().unwrap_or_default(), s.s_mm))
                .collect()
        };
        // the lane: C1 at 20 mm (inside the 40 mm piece, not at its end)
        let lc = lane_channel(&board, &an, l, &HashMap::new(), Some((50.0, false)), &rp, false, false, &cd).unwrap();
        let sh = shunt_at(&lc);
        assert!(sh.iter().any(|(n, s)| n.starts_with("C1 100nF") && (s - 20.0).abs() < 0.6), "{sh:?}");
        // the net
        let net = board.nets.iter().position(|n| n == "SIG").unwrap() as u32;
        let ch =
            crate::rf::channel_dir(&board, &an, net, &HashMap::new(), Some((50.0, false)), &rp, false, None).unwrap();
        let sh = shunt_at(&ch);
        assert!(sh.len() == 1 && sh[0].0.starts_with("C1 100nF") && (sh[0].1 - 20.0).abs() < 0.6, "{sh:?}");
        assert!(
            ch.warnings.iter().any(|w| w.starts_with("shunt part(s) modelled at their pads: C1 100nF 16V X7R (0805)")),
            "{:?}",
            ch.warnings
        );
        // against the bare line: 100 nF with 0.6 nH is about 19 Ω at 5 GHz: S21 well below
        let bare = capz_board(false);
        let (an0, _) = lanes_of(&bare);
        let ch0 =
            crate::rf::channel_dir(&bare, &an0, net, &HashMap::new(), Some((50.0, false)), &rp, false, None).unwrap();
        let k = ch.freq.iter().position(|&f| f >= 5e9).unwrap();
        assert!(ch.s21_db[k] < ch0.s21_db[k] - 3.0, "with C1 {} dB, without {} dB", ch.s21_db[k], ch0.s21_db[k]);
        assert!(ch0.segments.iter().all(|s| s.kind != "shunt"));
        // the 4-port model of a pair would take it per line; a 49.9 Ω resistor instead of C1 is a
        // termination: not in the net's channel, but named
        let pcb_r = capz_board(true);
        let mut b2 = pcb_r.clone();
        b2.footprints[2].value = "49.9".into();
        b2.footprints[2].reference = "R1".into();
        let (an2, _) = lanes_of(&b2);
        let ch2 =
            crate::rf::channel_dir(&b2, &an2, net, &HashMap::new(), Some((50.0, false)), &rp, false, None).unwrap();
        assert!(
            ch2.segments.iter().all(|s| s.kind != "shunt")
                && ch2.warnings.iter().any(|w| w.starts_with("termination(s) R1 49.9 not in the channel")),
            "{:?}",
            ch2.warnings
        );
    }

    /// A board with the CAPZ stackup, footprints `fps` and segments `segs` (nets SIG 1, GND 4).
    fn line_board(fps: &str, segs: &str) -> Board {
        let pcb = format!(
            r#"(kicad_pcb (version 20241229) (generator "pcbnew")
  (layers (0 "F.Cu" signal) (4 "In1.Cu" signal) (2 "B.Cu" signal))
  (setup (stackup
    (layer "F.Cu" (type "copper") (thickness 0.035))
    (layer "dielectric 1" (type "prepreg") (thickness 0.2) (material "FR4") (epsilon_r 4.3) (loss_tangent 0.02))
    (layer "In1.Cu" (type "copper") (thickness 0.035))
    (layer "dielectric 2" (type "core") (thickness 1.2) (material "FR4") (epsilon_r 4.5) (loss_tangent 0.02))
    (layer "B.Cu" (type "copper") (thickness 0.035))))
  (net 0 "") (net 1 "SIG") (net 4 "GND")
  (footprint "J" (layer "F.Cu") (at 0 0) (property "Reference" "J1") (property "Value" "SMA") (attr smd)
    (pad "1" smd rect (at 0 0) (size 0.6 0.35) (layers "F.Cu") (net 1 "SIG")))
{fps}{segs}  (zone (net 4) (net_name "GND") (layer "In1.Cu") (filled_polygon (layer "In1.Cu") (pts (xy -5 -5) (xy 50 -5) (xy 50 8) (xy -5 8)))))
"#
        );
        crate::board::load_pcb(&pcb).unwrap()
    }

    /// A capacitor to GND whose pad ends the net's path (an AC termination, the grounded leg of
    /// a balun) loads that port: named, not put into the channel (it would short the port).
    #[test]
    fn shunt_capacitor_at_the_end_of_the_path() {
        let board = line_board(
            r#"  (footprint "Capacitor_SMD:C_0402_1005Metric" (layer "F.Cu") (at 40.5 0) (property "Reference" "C9") (property "Value" "100nF") (attr smd)
    (pad "1" smd roundrect (at -0.48 0) (size 0.56 0.62) (roundrect_rratio 0.25) (layers "F.Cu") (net 1 "SIG"))
    (pad "2" smd roundrect (at 0.48 0) (size 0.56 0.62) (roundrect_rratio 0.25) (layers "F.Cu") (net 4 "GND")))
"#,
            "  (segment (start 0 0) (end 40.02 0) (width 0.35) (layer \"F.Cu\") (net 1))\n",
        );
        let (an, _) = lanes_of(&board);
        let net = board.nets.iter().position(|n| n == "SIG").unwrap() as u32;
        let rp = RfParams { fmax: 10e9, npts: 50, ..RfParams::default() };
        let ch =
            crate::rf::channel_dir(&board, &an, net, &HashMap::new(), Some((50.0, false)), &rp, false, None).unwrap();
        let ends = ch.path.as_ref().map(|p| (p.start.label.clone(), p.end.label.clone())).unwrap();
        assert!(ends.0 == "C9.1" || ends.1 == "C9.1", "{ends:?}");
        assert!(ch.segments.iter().all(|s| s.kind != "shunt"), "not in the channel");
        assert!(
            ch.warnings.iter().any(|w| w.starts_with("C9 100nF (0402) at an end of the computed path")),
            "{:?}",
            ch.warnings
        );
        // modelled, 100 nF would short the port (about -15 dB at 1 GHz)
        assert!(
            ch.freq.iter().zip(&ch.s21_db).filter(|(f, _)| **f <= 1e9).all(|(_, &d)| d > -0.5),
            "the port is not shorted"
        );
    }

    /// A pad is one node of the net's path: what lies on its copper joins its centre, and a
    /// track that leaves the pad keeps its full length even when it ends just beyond the pad
    /// (a 4.2 mm card-edge finger whose track runs from its centre to 0.25 mm past its end).
    #[test]
    fn track_leaving_a_long_pad_stays_in_the_path() {
        let board = line_board(
            r#"  (footprint "Edge" (layer "F.Cu") (at 42.1 0) (property "Reference" "P1") (property "Value" "PCIe") (attr smd)
    (pad "A16" smd rect (at 0 0 90) (size 0.7 4.2) (layers "F.Cu") (net 1 "SIG")))
"#,
            "  (segment (start 0 0) (end 39.75 0) (width 0.19) (layer \"F.Cu\") (net 1))\n  (segment (start 39.75 0) (end 42.1 0) (width 0.19) (layer \"F.Cu\") (net 1))\n",
        );
        let pt: Vec<&crate::board::Track> = board.tracks.iter().filter(|t| t.pad && t.item == 1).collect();
        assert!(pt.len() == 2 && (pt[0].width - 0.7).abs() < 1e-9, "the finger is a pad track of 0.7 mm");
        let (an, _) = lanes_of(&board);
        let net = board.nets.iter().position(|n| n == "SIG").unwrap() as u32;
        let info = crate::rf::net_path_info(&board, &an, net, false, None).unwrap();
        assert!(
            (info.length_mm - 42.1).abs() < 0.01,
            "path {} mm: the 2.35 mm track through the finger is kept",
            info.length_mm
        );
        assert_eq!(info.branches.len(), 0);
    }

    /// A pair from U1 to U2 through AC-coupling capacitors C1 (P) and C2 (N): coupled for 10 mm,
    /// fanned out to the capacitors, coupled again for 10 mm; B_P optionally with a jog of
    /// 2 × `jog` mm.
    fn pair_lane_board(jog: f64) -> Board {
        let bp = if jog > 0.0 {
            format!(
                "  (segment (start 20 -0.15) (end 22 -0.15) (width 0.15) (layer \"F.Cu\") (net 3))\n  (segment (start 22 -0.15) (end 22 {y}) (width 0.15) (layer \"F.Cu\") (net 3))\n  (segment (start 22 {y}) (end 23 {y}) (width 0.15) (layer \"F.Cu\") (net 3))\n  (segment (start 23 {y}) (end 23 -0.15) (width 0.15) (layer \"F.Cu\") (net 3))\n  (segment (start 23 -0.15) (end 30 -0.15) (width 0.15) (layer \"F.Cu\") (net 3))\n",
                y = -0.15 - jog
            )
        } else {
            "  (segment (start 20 -0.15) (end 30 -0.15) (width 0.15) (layer \"F.Cu\") (net 3))\n".to_string()
        };
        let pcb = format!(
            r#"(kicad_pcb (version 20241229) (generator "pcbnew")
  (layers (0 "F.Cu" signal) (4 "In1.Cu" signal) (2 "B.Cu" signal))
  (setup (stackup
    (layer "F.Cu" (type "copper") (thickness 0.035))
    (layer "dielectric 1" (type "prepreg") (thickness 0.1) (material "FR4") (epsilon_r 4.0) (loss_tangent 0.02))
    (layer "In1.Cu" (type "copper") (thickness 0.035))
    (layer "dielectric 2" (type "core") (thickness 1.3) (material "FR4") (epsilon_r 4.5) (loss_tangent 0.02))
    (layer "B.Cu" (type "copper") (thickness 0.035))))
  (net 0 "") (net 1 "A_P") (net 2 "A_N") (net 3 "B_P") (net 4 "B_N") (net 5 "GND")
  (footprint "U" (layer "F.Cu") (at 0 0) (property "Reference" "U1") (property "Value" "TX") (attr smd)
    (pad "1" smd rect (at 0 -0.15) (size 0.15 0.15) (layers "F.Cu") (net 1 "A_P"))
    (pad "2" smd rect (at 0 0.15) (size 0.15 0.15) (layers "F.Cu") (net 2 "A_N")))
  (footprint "C" (layer "F.Cu") (at 15 -0.5) (property "Reference" "C1") (property "Value" "100nF") (attr smd)
    (pad "1" smd rect (at -0.5 0) (size 0.5 0.5) (layers "F.Cu") (net 1 "A_P"))
    (pad "2" smd rect (at 0.5 0) (size 0.5 0.5) (layers "F.Cu") (net 3 "B_P")))
  (footprint "C" (layer "F.Cu") (at 15 0.5) (property "Reference" "C2") (property "Value" "100nF") (attr smd)
    (pad "1" smd rect (at -0.5 0) (size 0.5 0.5) (layers "F.Cu") (net 2 "A_N"))
    (pad "2" smd rect (at 0.5 0) (size 0.5 0.5) (layers "F.Cu") (net 4 "B_N")))
  (footprint "U" (layer "F.Cu") (at 30 0) (property "Reference" "U2") (property "Value" "RX") (attr smd)
    (pad "1" smd rect (at 0 -0.15) (size 0.15 0.15) (layers "F.Cu") (net 3 "B_P"))
    (pad "2" smd rect (at 0 0.15) (size 0.15 0.15) (layers "F.Cu") (net 4 "B_N")))
  (segment (start 0 -0.15) (end 10 -0.15) (width 0.15) (layer "F.Cu") (net 1))
  (segment (start 10 -0.15) (end 11 -0.5) (width 0.15) (layer "F.Cu") (net 1))
  (segment (start 11 -0.5) (end 14.5 -0.5) (width 0.15) (layer "F.Cu") (net 1))
  (segment (start 0 0.15) (end 10 0.15) (width 0.15) (layer "F.Cu") (net 2))
  (segment (start 10 0.15) (end 11 0.5) (width 0.15) (layer "F.Cu") (net 2))
  (segment (start 11 0.5) (end 14.5 0.5) (width 0.15) (layer "F.Cu") (net 2))
  (segment (start 15.5 -0.5) (end 19 -0.5) (width 0.15) (layer "F.Cu") (net 3))
  (segment (start 19 -0.5) (end 20 -0.15) (width 0.15) (layer "F.Cu") (net 3))
  (segment (start 15.5 0.5) (end 19 0.5) (width 0.15) (layer "F.Cu") (net 4))
  (segment (start 19 0.5) (end 20 0.15) (width 0.15) (layer "F.Cu") (net 4))
  (segment (start 20 0.15) (end 30 0.15) (width 0.15) (layer "F.Cu") (net 4))
{bp}  (zone (net 5) (net_name "GND") (layer "In1.Cu") (filled_polygon (layer "In1.Cu") (pts (xy -5 -5) (xy 35 -5) (xy 35 5) (xy -5 5)))))
"#
        );
        crate::board::load_pcb(&pcb).unwrap()
    }

    /// The pair's lane as a 4-port: symmetric, it gives the odd-mode cascade (loss, return loss
    /// and eye) without mode conversion; with a jog on P the mode conversion shows and the
    /// differential loss and eye differ from P's odd mode.
    #[test]
    fn pair_lane_as_four_port() {
        let cd = ComponentData::builtin();
        let rp = RfParams { fmax: 20e9, npts: 200, ..RfParams::default() };
        let ends = (
            Some(crate::eye::EndModel { r: 50.0, c_pf: 0.5, l_nh: 0.0 }),
            Some(crate::eye::EndModel { r: 50.0, c_pf: 0.5, l_nh: 0.0 }),
        );
        let mut results = Vec::new();
        for jog in [0.0, 1.0] {
            let board = pair_lane_board(jog);
            let mut params = crate::extract::Params::default();
            params.targets.insert("Default".into(), crate::extract::Target { z: 100.0, diff: true });
            let an = crate::analyze(&board, &params);
            let lanes: Vec<Lane> = buses(&board, &an, &cd).into_iter().flat_map(|b| b.lanes).collect();
            let l = lanes.iter().find(|l| l.kind == "ic-ic").expect("lane U1–U2");
            assert!(l.diff && !l.partner_hops.is_empty(), "{l:?}");
            assert_eq!(l.partner_shunts.len(), 0);
            let target = Some((100.0, true));
            let odd = lane_channel(&board, &an, l, &HashMap::new(), target, &rp, false, false, &cd).unwrap();
            let ev = lane_eval(
                &board,
                &an,
                &HashMap::new(),
                l,
                target,
                &rp,
                Some((5e9, 40e-12, 7, ends.0, ends.1)),
                false,
                None,
                None,
                &[],
                &cd,
            )
            .unwrap();
            assert_eq!(ev.model, "4-port");
            let scd = ev.scd21_db.as_ref().unwrap().iter().cloned().fold(f64::NEG_INFINITY, f64::max);
            let eye4 = ev.eye.unwrap().unwrap();
            let eye_odd = crate::eye::eye(
                &odd,
                &crate::eye::EyeParams { bitrate: 5e9, rise: 40e-12, tx: ends.0, rx: ends.1, ..Default::default() },
            )
            .unwrap();
            let dmax = ev.ch.s21_db.iter().zip(&odd.s21_db).map(|(a, b)| (a - b).abs()).fold(0.0, f64::max);
            let rl = |c: &Channel| -c.s11_db.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
            eprintln!("jog {jog}: max |Sdd21 − S21 odd| {dmax:.3} dB, RL {:.1} / {:.1} dB, Scd21 worst {scd:.1} dB, eye {:.3} / {:.3}, td {:?}", rl(&ev.ch), rl(&odd), eye4.height, eye_odd.height, eye4.td);
            results.push((dmax, scd, eye4.height, eye_odd.height));
            if jog == 0.0 {
                assert!(dmax < 0.02, "symmetric pair: Sdd21 = odd-mode S21 ({dmax} dB)");
                assert!((rl(&ev.ch) - rl(&odd)).abs() < 0.5, "{} {}", rl(&ev.ch), rl(&odd));
                assert!(scd < -60.0, "{scd}");
                assert!((eye4.height - eye_odd.height).abs() < 0.005, "{} {}", eye4.height, eye_odd.height);
                assert_eq!(eye4.td.dc_source, "network at 0 Hz");
            } else {
                assert!(scd > -25.0, "a 2 mm jog converts modes: {scd}");
            }
        }
        // the jog costs eye height in the 4-port (part of the signal goes to the common mode)
        assert!(results[1].2 < results[0].2, "{results:?}");
    }

    /// A multi-drop line: U1 drives U2 at the end of a 40 mm line and U3 on a 5 mm stub at 20 mm.
    /// The lane is checked per receiver: the stub is a tap with the receiver at its end, the eye is
    /// the worst of the two, and the loaded tap takes signal from the main receiver.
    #[test]
    fn multi_drop_per_receiver() {
        let board = line_board(
            r#"  (footprint "U" (layer "F.Cu") (at 40 0) (property "Reference" "U2") (property "Value" "RX") (attr smd)
    (pad "1" smd rect (at 0 0) (size 0.6 0.35) (layers "F.Cu") (net 1 "SIG")))
  (footprint "U" (layer "F.Cu") (at 20 5) (property "Reference" "U3") (property "Value" "RX") (attr smd)
    (pad "1" smd rect (at 0 0) (size 0.6 0.35) (layers "F.Cu") (net 1 "SIG")))
"#,
            "  (segment (start 0 0) (end 20 0) (width 0.35) (layer \"F.Cu\") (net 1))\n  (segment (start 20 0) (end 40 0) (width 0.35) (layer \"F.Cu\") (net 1))\n  (segment (start 20 0) (end 20 5) (width 0.35) (layer \"F.Cu\") (net 1))\n",
        );
        // U1 instead of the connector J1 at the start
        let mut board = board;
        board.footprints[0].reference = "U1".into();
        let cd = ComponentData::builtin();
        let (an, lanes) = lanes_of(&board);
        let l = lanes.iter().find(|l| l.kind == "multi-drop").expect("multi-drop lane");
        let names: Vec<&str> = l.ends.iter().map(|e| e.refdes.as_str()).collect();
        assert_eq!(names, vec!["U1", "U2", "U3"], "transmitter, end of the path, tap");
        assert_eq!(l.taps, vec!["U3.1".to_string()]);
        assert!(checked(l));
        let rp = RfParams { fmax: 15e9, npts: 300, ..RfParams::default() };
        let end = Some(crate::eye::EndModel { r: 50.0, c_pf: 1.0, l_nh: 0.0 });
        let target = Some((50.0, false));
        let ev = lane_eval(
            &board,
            &an,
            &HashMap::new(),
            l,
            target,
            &rp,
            Some((2e9, 100e-12, 7, end, end)),
            false,
            None,
            None,
            &[],
            &cd,
        )
        .unwrap();
        assert_eq!(ev.ch.taps.len(), 1);
        assert!(
            ev.ch
                .segments
                .iter()
                .any(|s| s.kind == "branch" && s.part.as_deref().is_some_and(|p| p.ends_with("to U3.1 (receiver)"))),
            "{:?}",
            ev.ch.segments
        );
        let pins: Vec<&str> = ev.receivers.iter().map(|r| r.pin.as_str()).collect();
        assert_eq!(pins, vec!["U2.1", "U3.1"]);
        let eye = ev.eye.unwrap().unwrap();
        let worst = ev.receivers.iter().map(|r| r.eye_height).fold(f64::INFINITY, f64::min);
        assert!((eye.height - worst).abs() < 1e-12 && eye.warnings[0].starts_with("eye at "), "{:?}", eye.warnings);
        // the same line with the stub open (the interconnect's cascade): the main receiver sees more
        let open = crate::eye::eye(
            &ev.ch,
            &crate::eye::EyeParams { bitrate: 2e9, rise: 100e-12, tx: end, rx: end, ..Default::default() },
        )
        .unwrap();
        eprintln!("receivers {:?}, open stub {:.3}", ev.receivers, open.height);
        assert!(ev.receivers[0].eye_height < open.height - 0.05, "{} vs {}", ev.receivers[0].eye_height, open.height);
        // the check row carries every receiver
        let row = check_lane(
            &board,
            &an,
            &HashMap::new(),
            l,
            target,
            &rp,
            2.0,
            (end, end, Some(100e-12)),
            false,
            None,
            None,
            &[],
            &cd,
        );
        assert_eq!(row.receivers.len(), 2);
        assert!((row.eye_height - worst).abs() < 1e-9);
    }

    /// Two single-ended lanes U1 → J1 (pins 1 and 2) with one 4-port file behind J1 for both
    /// (port map per pin): each lane's eye has the other lane as aggressor through the file.
    #[test]
    fn n_port_file_behind_a_connector() {
        let pcb = r#"(kicad_pcb (version 20241229) (generator "pcbnew")
  (layers (0 "F.Cu" signal) (4 "In1.Cu" signal) (2 "B.Cu" signal))
  (setup (stackup
    (layer "F.Cu" (type "copper") (thickness 0.035))
    (layer "dielectric 1" (type "prepreg") (thickness 0.2) (material "FR4") (epsilon_r 4.3) (loss_tangent 0.02))
    (layer "In1.Cu" (type "copper") (thickness 0.035))
    (layer "dielectric 2" (type "core") (thickness 1.2) (material "FR4") (epsilon_r 4.5) (loss_tangent 0.02))
    (layer "B.Cu" (type "copper") (thickness 0.035))))
  (net 0 "") (net 1 "A") (net 2 "B") (net 4 "GND")
  (footprint "U" (layer "F.Cu") (at 0 0) (property "Reference" "U1") (property "Value" "IC") (attr smd)
    (pad "1" smd rect (at 0 0) (size 0.4 0.4) (layers "F.Cu") (net 1 "A"))
    (pad "2" smd rect (at 0 2) (size 0.4 0.4) (layers "F.Cu") (net 2 "B")))
  (footprint "J" (layer "F.Cu") (at 30 0) (property "Reference" "J1") (property "Value" "conn") (attr smd)
    (pad "1" smd rect (at 0 0) (size 0.4 0.4) (layers "F.Cu") (net 1 "A"))
    (pad "2" smd rect (at 0 2) (size 0.4 0.4) (layers "F.Cu") (net 2 "B")))
  (segment (start 0 0) (end 30 0) (width 0.35) (layer "F.Cu") (net 1))
  (segment (start 0 2) (end 30 2) (width 0.35) (layer "F.Cu") (net 2))
  (zone (net 4) (net_name "GND") (layer "In1.Cu") (filled_polygon (layer "In1.Cu") (pts (xy -5 -5) (xy 35 -5) (xy 35 8) (xy -5 8)))))
"#;
        let board = crate::board::load_pcb(pcb).unwrap();
        let cd = ComponentData::builtin();
        let (an, lanes) = lanes_of(&board);
        let la = lanes.iter().find(|l| l.name == "A").expect("lane A");
        assert_eq!(la.kind, "ic-connector");
        let xt = connector_lanes(&lanes, "J1");
        assert_eq!(xt.len(), 2);
        assert_eq!(connector_pins(la, "J1"), vec!["1".to_string()]);
        // the file: two coupled 50 Ω lines (even 56, odd 44.5 Ω), 50 mm
        let rp = RfParams { fmax: 15e9, npts: 150, ..RfParams::default() };
        let freq: Vec<f64> = (1..=150).map(|k| k as f64 * 0.1e9).collect();
        let mut text = String::from("# Hz S RI R 50\n");
        for &f in &freq {
            let ln = |z: f64, e: f64| {
                crate::rf::Abcd::line(
                    2.0,
                    z * e.sqrt() / crate::rf::C0,
                    1e-4,
                    e.sqrt() / (z * crate::rf::C0),
                    2.0 * std::f64::consts::PI * f,
                    0.05,
                )
            };
            let sm = crate::rf::s_of(&crate::rf::coupled(&ln(56.0, 3.4), &ln(44.5, 3.0)), 50.0);
            text += &format!("{f}");
            for r in sm {
                for c in r {
                    text += &format!(" {:.12e} {:.12e}", c.re, c.im);
                }
            }
            text += "\n";
        }
        let ts = crate::rf::parse_touchstone(&text, "cable.s4p").unwrap();
        let mut ports = std::collections::BTreeMap::new();
        ports.insert("1".to_string(), [1, 3]);
        ports.insert("2".to_string(), [2, 4]);
        let ce = crate::eye::ConnEnd {
            model: "matched".into(),
            touchstone: Some(crate::eye::TsRef { file: "cable.s4p".into(), ports, ..Default::default() }),
            ..Default::default()
        };
        let end = Some(crate::eye::EndModel { r: 50.0, c_pf: 0.5, l_nh: 0.0 });
        let ev = lane_eval(
            &board,
            &an,
            &HashMap::new(),
            la,
            Some((50.0, false)),
            &rp,
            Some((5e9, 50e-12, 7, end, end)),
            la.rx_end == 0,
            Some(&ce),
            Some(&ts),
            &xt,
            &cd,
        )
        .unwrap();
        assert!(ev.rest_applied, "{:?}", ev.end_note);
        assert!(ev.end_note.as_deref().unwrap_or("").contains("1 aggressor lane"), "{:?}", ev.end_note);
        let e = ev.eye.unwrap().unwrap();
        let x = e.xt.clone().expect("crosstalk in the eye");
        eprintln!("eye {:.3} (alone {:.3}), crosstalk {:?}", e.height, x.height_alone, x.aggressors);
        assert_eq!(x.aggressors.len(), 1);
        assert_eq!(x.aggressors[0].0, "B");
        assert!(x.peak > 0.005 && e.height < x.height_alone);
        // without the port map the 4-port of two single lines is refused for a single-ended lane
        let ce2 = crate::eye::ConnEnd {
            touchstone: Some(crate::eye::TsRef { file: "cable.s4p".into(), ..Default::default() }),
            ..ce.clone()
        };
        let ev2 = lane_eval(
            &board,
            &an,
            &HashMap::new(),
            la,
            Some((50.0, false)),
            &rp,
            Some((5e9, 50e-12, 7, end, end)),
            la.rx_end == 0,
            Some(&ce2),
            Some(&ts),
            &xt,
            &cd,
        )
        .unwrap();
        assert!(!ev2.rest_applied && ev2.end_note.unwrap().contains("needs a differential lane"));
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
        assert!(
            l.branches.is_empty() && !lanes.iter().any(|l| reaches(l, "U1") && reaches(l, "J2")),
            "R2 connects nothing"
        );
        let ch =
            lane_channel(&board, &an, l, &HashMap::new(), Some((50.0, false)), &RfParams::default(), false, false, &cd)
                .unwrap();
        let stubs: Vec<&str> =
            ch.segments.iter().filter(|s| s.kind == "branch").filter_map(|s| s.part.as_deref()).collect();
        assert_eq!(stubs.len(), 1, "{stubs:?}");
        assert!(
            stubs[0].starts_with("stub 3.")
                && stubs[0].contains(" mm to R2.1 (not fitted; pad 0402, 0.05 pF from the pad geometry"),
            "{}",
            stubs[0]
        );
        // the stub costs a little: compared with the same lane without it (R2 removed)
        let mut bare = board.clone();
        for p in bare.pads.iter_mut().filter(|p| board.footprints[p.fp as usize].reference == "R2") {
            p.net = 3;
        }
        bare.tracks.retain(|t| !(t.net == 1 && (t.a[1] > 0.1 || t.b[1] > 0.1)));
        let (an2, lanes2) = lanes_of(&bare);
        let l2 = lanes2.iter().find(|l| l.ends.iter().any(|e| e.refdes == "J1")).unwrap();
        let ch2 = lane_channel(
            &bare,
            &an2,
            l2,
            &HashMap::new(),
            Some((50.0, false)),
            &RfParams::default(),
            false,
            false,
            &cd,
        )
        .unwrap();
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
        assert_eq!(
            (f[0].net_name.as_str(), f[0].parts.clone(), f[0].towards.clone()),
            ("SIG", vec!["R1".to_string(), "R2".to_string()], vec![vec!["J1".to_string()], vec!["J2".to_string()]])
        );
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
            let br_nets: Vec<u32> = l.branches[0]
                .iter()
                .filter_map(|h| if let Hop::Net { net, .. } = h { Some(*net) } else { None })
                .collect();
            assert!(
                br_nets.contains(&id(other))
                    && l.branches[0].iter().any(|h| matches!(h, Hop::Series { part: p } if p.refdes == part))
            );
        }
        // the lane to J1 carries the other path as a branch from the junction, through R2 to J2
        let l = lanes.iter().find(|l| l.ends.iter().any(|e| e.refdes == "J1")).unwrap();
        let ch =
            lane_channel(&board, &an, l, &HashMap::new(), Some((50.0, false)), &RfParams::default(), false, false, &cd)
                .unwrap();
        let br: Vec<&str> =
            ch.segments.iter().filter(|s| s.kind == "branch").filter_map(|s| s.part.as_deref()).collect();
        assert_eq!(br.len(), 1, "{br:?}");
        assert!(br[0].contains("on through R2 to J2"), "{}", br[0]);
        assert!(ch.warnings.iter().any(|w| w.starts_with("possible assembly variant")), "{:?}", ch.warnings);
        // the IC behind a series part: each lane runs through R0 and its own branch of the fork,
        // the other branch leaves after R0 (not at the IC)
        let board = fork_board_behind();
        let (an, lanes) = lanes_of(&board);
        let id = |n: &str| board.nets.iter().position(|x| x == n).unwrap() as u32;
        for (far, own, other) in [("J1", "A", "R2"), ("J2", "B", "R1")] {
            let l = lanes
                .iter()
                .find(|l| ends(l) == vec!["U1", far])
                .unwrap_or_else(|| panic!("lane U1–{far}: {:?}", lanes.iter().map(ends).collect::<Vec<_>>()));
            assert_eq!(l.nets, vec![id("SRC"), id("SIG"), id(own)]);
            assert_eq!(l.name, own);
            assert!(
                matches!(&l.branches[0][..], [Hop::Net { net, .. }, Hop::Series { part }, ..] if *net == id("SIG") && part.refdes == other),
                "{far}"
            );
            let ch = lane_channel(
                &board,
                &an,
                l,
                &HashMap::new(),
                Some((50.0, false)),
                &RfParams::default(),
                false,
                false,
                &cd,
            )
            .unwrap();
            assert!(
                ch.segments
                    .iter()
                    .filter_map(|s| s.part.as_deref())
                    .any(|p| p.contains(&format!("on through {other} to"))),
                "{far}: {:?}",
                ch.warnings
            );
        }
        // the junction is at 19 mm, not at U1: the branch starts after the first 19 mm of line
        let before: f64 =
            ch.segments.iter().take_while(|s| s.kind != "branch").filter(|s| s.kind == "line").map(|s| s.len_mm).sum();
        assert!((before - 19.0).abs() < 0.6, "{before}");
    }

    /// J1 – 9.5 mm – 0402 capacitor C1 – 9.5 mm – J2 on the CAPZ stackup; `cut`: the In1 plane cut
    /// out under C1 (next plane B.Cu, 1.4 mm). `finger`: a 0.7 × 2.1 mm finger pad on the A line at
    /// 3 mm with GND fingers 0.3 mm beside it and no plane under it.
    fn pad_board(cut: bool, finger: bool) -> Board {
        let zone = if cut {
            // the plane with a 1.0 × 1.2 mm cut-out under C1, fractured to the bottom edge
            "(xy -5 -5) (xy 9.5 -5) (xy 9.5 -0.6) (xy 9.5 0.6) (xy 10.5 0.6) (xy 10.5 -0.6) (xy 9.5 -0.6) (xy 9.5 -5) (xy 25 -5) (xy 25 8) (xy -5 8)"
        } else if finger {
            // the plane ends at x = 4.5 under the finger region (x < 4.5 has no plane)
            "(xy 4.5 -5) (xy 25 -5) (xy 25 8) (xy 4.5 8)"
        } else {
            "(xy -5 -5) (xy 25 -5) (xy 25 8) (xy -5 8)"
        };
        // B.Cu: a plane everywhere (under the cut-out the next reference), not under the finger
        let bottom = if finger {
            "(xy 4.5 -5) (xy 25 -5) (xy 25 8) (xy 4.5 8)"
        } else {
            "(xy -5 -5) (xy 25 -5) (xy 25 8) (xy -5 8)"
        };
        let fing = if finger {
            r#"  (footprint "Edge" (layer "F.Cu") (at 3 0) (property "Reference" "P1") (property "Value" "edge") (attr smd)
    (pad "1" smd rect (at 0 0) (size 2.1 0.7) (layers "F.Cu") (net 1 "A")))
  (zone (net 4) (net_name "GND") (layer "F.Cu") (filled_polygon (layer "F.Cu") (pts (xy 1.95 0.65) (xy 4.05 0.65) (xy 4.05 1.35) (xy 1.95 1.35))))
  (zone (net 4) (net_name "GND") (layer "F.Cu") (filled_polygon (layer "F.Cu") (pts (xy 1.95 -0.65) (xy 4.05 -0.65) (xy 4.05 -1.35) (xy 1.95 -1.35))))
"#
        } else {
            ""
        };
        let pcb = format!(
            r#"(kicad_pcb (version 20241229) (generator "pcbnew")
  (layers (0 "F.Cu" signal) (4 "In1.Cu" signal) (2 "B.Cu" signal))
  (setup (stackup
    (layer "F.Cu" (type "copper") (thickness 0.035))
    (layer "dielectric 1" (type "prepreg") (thickness 0.2) (material "FR4") (epsilon_r 4.3) (loss_tangent 0.02))
    (layer "In1.Cu" (type "copper") (thickness 0.035))
    (layer "dielectric 2" (type "core") (thickness 1.2) (material "FR4") (epsilon_r 4.5) (loss_tangent 0.02))
    (layer "B.Cu" (type "copper") (thickness 0.035))))
  (net 0 "") (net 1 "A") (net 2 "B") (net 4 "GND")
  (footprint "J" (layer "F.Cu") (at 0 0) (property "Reference" "J1") (property "Value" "SMA") (attr smd)
    (pad "1" smd rect (at 0 0) (size 0.6 0.35) (layers "F.Cu") (net 1 "A")))
  (footprint "J" (layer "F.Cu") (at 20 0) (property "Reference" "J2") (property "Value" "SMA") (attr smd)
    (pad "1" smd rect (at 0 0) (size 0.6 0.35) (layers "F.Cu") (net 2 "B")))
  (footprint "Capacitor_SMD:C_0402_1005Metric" (layer "F.Cu") (at 10 0) (property "Reference" "C1") (property "Value" "100nF") (attr smd)
    (pad "1" smd rect (at -0.48 0) (size 0.56 0.62) (layers "F.Cu") (net 1 "A"))
    (pad "2" smd rect (at 0.48 0) (size 0.56 0.62) (layers "F.Cu") (net 2 "B")))
{fing}  (segment (start 0 0) (end 3 0) (width 0.35) (layer "F.Cu") (net 1))
  (segment (start 3 0) (end 9.52 0) (width 0.35) (layer "F.Cu") (net 1))
  (segment (start 10.48 0) (end 20 0) (width 0.35) (layer "F.Cu") (net 2))
  (zone (net 4) (net_name "GND") (layer "In1.Cu") (filled_polygon (layer "In1.Cu") (pts {zone})))
  (zone (net 4) (net_name "GND") (layer "B.Cu") (filled_polygon (layer "B.Cu") (pts {bottom})))
)
"#
        );
        crate::board::load_pcb(&pcb).unwrap()
    }

    /// Pads: the track inside a pad has the pad's width, exactly from the pad's edge; the pads of a
    /// series capacitor are shunts from their geometry – smaller over a cut-out in the plane; a
    /// finger without a plane under it, between GND fingers, has the coplanar impedance.
    #[test]
    fn pads_from_their_geometry() {
        let cd = ComponentData::builtin();
        let rp = RfParams { fmax: 10e9, npts: 50, ..RfParams::default() };
        let pad_c = |board: &Board| -> (f64, Vec<(f64, f64)>) {
            let (an, lanes) = lanes_of(board);
            let l = lanes.iter().find(|l| l.kind == "connector-connector").expect("lane J1–J2");
            let ch = lane_channel(board, &an, l, &HashMap::new(), Some((50.0, false)), &rp, false, false, &cd).unwrap();
            let pads: Vec<&crate::rf::Segment> = ch
                .segments
                .iter()
                .filter(|s| s.kind == "shunt" && s.part.as_deref().is_some_and(|p| p.starts_with("C1 pad")))
                .collect();
            assert!(
                pads.len() == 2 && pads.iter().all(|s| s.source == "pad geometry"),
                "{:?}",
                ch.segments.iter().map(|s| (s.kind, s.source, s.part.clone())).collect::<Vec<_>>()
            );
            let c: f64 =
                pads[0].part.as_deref().unwrap().split('(').nth(1).unwrap().split(' ').next().unwrap().parse().unwrap();
            // the line sections in the pads: 0.62 mm wide, 0.28 mm long
            let wide: Vec<(f64, f64)> = ch
                .segments
                .iter()
                .filter(|s| s.kind == "line")
                .filter_map(|s| Some((an.cross_sections[s.xs? as usize].w, s.len_mm)))
                .filter(|(w, _)| *w > 0.5)
                .collect();
            (c, wide)
        };
        let (c0, wide) = pad_c(&pad_board(false, false));
        assert!(
            wide.len() == 2 && wide.iter().all(|&(w, l)| (w - 0.62).abs() < 1e-9 && (l - 0.28).abs() < 1e-6),
            "{wide:?}"
        );
        assert!((0.03..0.09).contains(&c0), "pad over the plane: {c0} pF");
        let (c1, _) = pad_c(&pad_board(true, false));
        assert!(c1 < 0.5 * c0, "over the cut-out {c1} pF, over the plane {c0} pF");
        // the finger
        let b = pad_board(false, true);
        let (an, _) = lanes_of(&b);
        let xs: Vec<&crate::extract::CrossSection> = an
            .pieces
            .chunks_exact(4)
            .filter(|p| !b.tracks[p[0] as usize].pad)
            .map(|p| &an.cross_sections[p[3] as usize])
            .filter(|x| (x.w - 0.7).abs() < 1e-9)
            .collect();
        assert!(
            !xs.is_empty()
                && xs
                    .iter()
                    .all(|x| x.kind == crate::extract::Kind::NoReference
                        && x.z0.is_some_and(|z| (40.0..100.0).contains(&z))),
            "{:?}",
            xs.iter().map(|x| (x.kind, x.z0, x.s_left, x.s_right)).collect::<Vec<_>>()
        );
    }
}

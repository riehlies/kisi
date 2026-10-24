//! Power tree: regulator chains, load currents and the power balance.
//!
//! A tree is described by rails (nominal voltages), elements that convert or
//! pass power from one rail to another (input connector, buck, boost, LDO,
//! inverter, pass element such as a shunt or ferrite) and loads (component
//! current on a rail). Solving it from the leaves to the inputs gives the
//! current of every rail and element, the losses of every regulator, the
//! power of every component and, as a by-product, the source/load setup of
//! every rail for the IR-drop solver (`pi::solve_rail`).

use crate::board::Board;
use crate::pi::{find_net, LoadSpec, RailSetup, SourceSpec};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct TreeSpec {
    pub name: String,
    pub note: String,
    pub rails: Vec<RailV>,
    pub elements: Vec<ElemSpec>,
    pub loads: Vec<LoadDef>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct RailV {
    pub net: String,
    pub voltage: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ElemSpec {
    /// unique id (defaults to refdes; needed when one IC has several channels)
    pub id: String,
    pub refdes: String,
    /// input | buck | boost | ldo | inverter | pass
    pub kind: String,
    pub from: Option<String>,
    pub to: String,
    /// efficiency of switching converters (0..1); `None`: kisi's default `DEFAULT_EFF` (an estimate)
    pub eff: Option<f64>,
    /// quiescent / ground current drawn from the input (A)
    pub iq: f64,
    /// series resistance of pass elements (Ω)
    pub r: f64,
    /// rated output current (A)
    pub imax: Option<f64>,
    /// component whose pads carry the input / output current, if not `refdes`
    /// (e.g. the inductor of a buck converter)
    pub in_ref: Option<String>,
    pub out_ref: Option<String>,
    pub part: String,
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct LoadDef {
    pub refdes: String,
    pub net: String,
    pub current: f64,
    pub part: String,
    /// "datasheet" or "estimate"
    pub basis: String,
    pub note: String,
    /// components whose pads are the load point, comma separated (default refdes)
    pub at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RailFlow {
    pub net: String,
    pub voltage: f64,
    /// current of loads directly on this rail
    pub i_loads: f64,
    /// current into elements fed by this rail
    pub i_children: f64,
    pub i_total: f64,
    pub fed_by: Option<String>,
    pub depth: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct ElemFlow {
    pub id: String,
    pub refdes: String,
    pub kind: String,
    pub part: String,
    pub from: Option<String>,
    pub to: String,
    pub v_in: f64,
    pub v_out: f64,
    pub i_in: f64,
    pub i_out: f64,
    pub p_in: f64,
    pub p_out: f64,
    pub p_loss: f64,
    pub imax: Option<f64>,
    pub overload: bool,
    /// efficiency used for a switching converter (None for inputs, LDOs and pass elements)
    pub eff: Option<f64>,
    /// the efficiency is kisi's default (none given): an estimate
    pub eff_estimate: bool,
    pub note: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CompPower {
    pub refdes: String,
    pub part: String,
    pub consumption: f64,
    pub loss: f64,
    /// (rail, current A, basis) for loads
    pub rails: Vec<(String, f64, String)>,
    pub role: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct TreeResult {
    pub name: String,
    pub note: String,
    pub rails: Vec<RailFlow>,
    pub elements: Vec<ElemFlow>,
    pub components: Vec<CompPower>,
    pub p_input: f64,
    pub p_loads: f64,
    pub p_losses: f64,
    /// source/load setup per rail for the IR-drop solver
    pub setups: Vec<RailSetup>,
    /// board spelling of the net of every spec load (same order as `TreeSpec::loads`)
    pub load_nets: Vec<String>,
    pub warnings: Vec<String>,
}

/// Efficiency of a switching converter without a value in its PI_REG field or tree entry: an
/// estimate (typical for small bucks at moderate load), marked as such in CLI and viewer.
pub const DEFAULT_EFF: f64 = 0.85;

/// Whether an element kind is a switching converter (efficiency applies).
pub fn is_switching(kind: &str) -> bool {
    !matches!(kind, "input" | "pass" | "ldo")
}

fn elem_id(e: &ElemSpec) -> String {
    if e.id.is_empty() { e.refdes.clone() } else { e.id.clone() }
}

/// Map a net name of the spec to the board's spelling (if a board is given).
fn canon(board: Option<&Board>, net: &str) -> String {
    match board.and_then(|b| find_net(b, net).map(|n| b.net_name(n).to_string())) {
        Some(n) => n,
        None => net.to_string(),
    }
}

pub fn solve_tree(spec: &TreeSpec, board: Option<&Board>) -> Result<TreeResult, String> {
    let mut warnings = Vec::new();
    // canonical net names
    let mut missing = Vec::new();
    let mut cn = |n: &str| -> String {
        let c = canon(board, n);
        if board.is_some() && board.and_then(|b| find_net(b, n)).is_none() && !missing.contains(&n.to_string()) {
            missing.push(n.to_string());
        }
        c
    };
    let rails_v: Vec<(String, f64)> = spec.rails.iter().map(|r| (cn(&r.net), r.voltage)).collect();
    let elems: Vec<(ElemSpec, Option<String>, String)> =
        spec.elements.iter().map(|e| (e.clone(), e.from.as_deref().map(&mut cn), cn(&e.to))).collect();
    let loads: Vec<(LoadDef, String)> = spec.loads.iter().map(|l| (l.clone(), cn(&l.net))).collect();
    for m in &missing {
        warnings.push(format!("net '{m}' not on the board"));
    }

    // voltages: explicit, then inherited through pass elements
    let mut volt: HashMap<String, f64> = rails_v.iter().cloned().collect();
    for _ in 0..elems.len() + 1 {
        let mut changed = false;
        for (e, from, to) in &elems {
            if e.kind == "pass" {
                if let Some(f) = from {
                    if let (Some(&v), false) = (volt.get(f), volt.contains_key(to)) {
                        volt.insert(to.clone(), v);
                        changed = true;
                    }
                }
            }
        }
        if !changed {
            break;
        }
    }
    let v_of = |n: &str| -> Result<f64, String> { volt.get(n).copied().ok_or_else(|| format!("voltage of '{n}' missing")) };

    // feeding element per rail, children per rail
    let mut fed: HashMap<String, usize> = HashMap::new();
    let mut children: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, (e, from, to)) in elems.iter().enumerate() {
        if let Some(prev) = fed.insert(to.clone(), i) {
            warnings.push(format!("{} is fed by {} and {}; only {} is computed", to, elem_id(&elems[prev].0), elem_id(e), elem_id(e)));
        }
        if let Some(f) = from {
            children.entry(f.clone()).or_default().push(i);
        } else if e.kind != "input" {
            return Err(format!("{}: input ('from') missing", elem_id(e)));
        }
    }
    let mut direct: HashMap<String, f64> = HashMap::new();
    for (l, net) in &loads {
        *direct.entry(net.clone()).or_default() += l.current;
    }

    // element input current from its output current
    let i_in_of = |e: &ElemSpec, v_in: f64, v_out: f64, i_out: f64| -> f64 {
        match e.kind.as_str() {
            "input" => i_out,
            // the I²R loss of a pass element shows up as extra input current
            // at the nominal voltage, so the balance stays exact
            "pass" => i_out + i_out * i_out * e.r / v_in.abs().max(1e-9),
            "ldo" => i_out + e.iq,
            _ => {
                let eff = e.eff.unwrap_or(DEFAULT_EFF).clamp(0.05, 1.0);
                v_out.abs() * i_out / (eff * v_in.abs().max(1e-9)) + e.iq
            }
        }
    };

    // recursive rail current with cycle detection
    fn rail_current(
        net: &str,
        elems: &[(ElemSpec, Option<String>, String)],
        children: &HashMap<String, Vec<usize>>,
        direct: &HashMap<String, f64>,
        volt: &HashMap<String, f64>,
        i_in_of: &dyn Fn(&ElemSpec, f64, f64, f64) -> f64,
        memo: &mut HashMap<String, f64>,
        stack: &mut Vec<String>,
        elem_in: &mut HashMap<usize, f64>,
    ) -> Result<f64, String> {
        if let Some(&v) = memo.get(net) {
            return Ok(v);
        }
        if stack.iter().any(|s| s == net) {
            return Err(format!("loop in the power tree at {net}"));
        }
        stack.push(net.to_string());
        let mut total = direct.get(net).copied().unwrap_or(0.0);
        for &ci in children.get(net).map(|v| v.as_slice()).unwrap_or(&[]) {
            let (e, _, to) = &elems[ci];
            let i_out = rail_current(to, elems, children, direct, volt, i_in_of, memo, stack, elem_in)?;
            let v_in = volt.get(net).copied().ok_or_else(|| format!("voltage of '{net}' missing"))?;
            let v_out = volt.get(to).copied().ok_or_else(|| format!("voltage of '{to}' missing"))?;
            let i_in = i_in_of(e, v_in, v_out, i_out);
            elem_in.insert(ci, i_in);
            total += i_in;
        }
        stack.pop();
        memo.insert(net.to_string(), total);
        Ok(total)
    }

    let mut memo = HashMap::new();
    let mut elem_in: HashMap<usize, f64> = HashMap::new();
    let mut stack = Vec::new();
    let mut nets: Vec<String> = Vec::new();
    for (e, from, to) in &elems {
        for n in from.iter().chain(std::iter::once(to)) {
            if !nets.contains(n) {
                nets.push(n.clone());
            }
        }
        let _ = e;
    }
    for (_, n) in &loads {
        if !nets.contains(n) {
            nets.push(n.clone());
            if !fed.contains_key(n) {
                warnings.push(format!("load on {n}, which no element feeds"));
            }
        }
    }
    for n in &nets {
        rail_current(n, &elems, &children, &direct, &volt, &i_in_of, &mut memo, &mut stack, &mut elem_in)?;
    }

    // element flows
    let mut eflows = Vec::new();
    let mut p_input = 0.0;
    for (i, (e, from, to)) in elems.iter().enumerate() {
        let v_out = v_of(to)?;
        let i_out = memo.get(to).copied().unwrap_or(0.0);
        let (v_in, i_in) = match from {
            Some(f) => (v_of(f)?, elem_in.get(&i).copied().unwrap_or_else(|| i_in_of(e, v_of(f).unwrap_or(v_out), v_out, i_out))),
            None => (v_out, i_out),
        };
        let p_out = v_out.abs() * i_out;
        let p_in = if e.kind == "input" { p_out } else { v_in.abs() * i_in };
        let p_loss = (p_in - p_out).max(0.0);
        if e.kind == "input" {
            p_input += p_out;
        }
        let overload = e.imax.map(|m| i_out > m).unwrap_or(false);
        if overload {
            warnings.push(format!(
                "{} ({}) delivers {:.2} A, rated {:.2} A",
                elem_id(e),
                e.part,
                i_out,
                e.imax.unwrap()
            ));
        }
        eflows.push(ElemFlow {
            id: elem_id(e),
            refdes: e.refdes.clone(),
            kind: e.kind.clone(),
            part: e.part.clone(),
            from: from.clone(),
            to: to.clone(),
            v_in,
            v_out,
            i_in,
            i_out,
            p_in,
            p_out,
            p_loss,
            imax: e.imax,
            overload,
            eff: is_switching(&e.kind).then(|| e.eff.unwrap_or(DEFAULT_EFF).clamp(0.05, 1.0)),
            eff_estimate: is_switching(&e.kind) && e.eff.is_none(),
            note: e.note.clone(),
        });
    }
    // rails with depth from the inputs
    let mut depth: HashMap<String, usize> = HashMap::new();
    fn set_depth(net: &str, d: usize, children: &HashMap<String, Vec<usize>>, elems: &[(ElemSpec, Option<String>, String)], depth: &mut HashMap<String, usize>) {
        if depth.get(net).is_some_and(|&x| x <= d) {
            return;
        }
        depth.insert(net.to_string(), d);
        for &ci in children.get(net).map(|v| v.as_slice()).unwrap_or(&[]) {
            set_depth(&elems[ci].2, d + 1, children, elems, depth);
        }
    }
    for (e, from, to) in &elems {
        if from.is_none() && e.kind == "input" {
            set_depth(to, 0, &children, &elems, &mut depth);
        }
    }
    let rails: Vec<RailFlow> = nets
        .iter()
        .map(|n| {
            let il = direct.get(n).copied().unwrap_or(0.0);
            let it = memo.get(n).copied().unwrap_or(0.0);
            RailFlow {
                net: n.clone(),
                voltage: volt.get(n).copied().unwrap_or(0.0),
                i_loads: il,
                i_children: it - il,
                i_total: it,
                fed_by: fed.get(n).map(|&i| elem_id(&elems[i].0)),
                depth: depth.get(n).copied().unwrap_or(99),
            }
        })
        .collect();

    // per component
    let mut comps: Vec<CompPower> = Vec::new();
    let mut idx: HashMap<String, usize> = HashMap::new();
    let mut get = |comps: &mut Vec<CompPower>, r: &str, part: &str, role: &str| -> usize {
        if let Some(&i) = idx.get(r) {
            return i;
        }
        comps.push(CompPower { refdes: r.to_string(), part: part.to_string(), consumption: 0.0, loss: 0.0, rails: vec![], role: role.to_string() });
        idx.insert(r.to_string(), comps.len() - 1);
        comps.len() - 1
    };
    let mut p_loads = 0.0;
    for (l, net) in &loads {
        let p = volt.get(net).copied().unwrap_or(0.0).abs() * l.current;
        p_loads += p;
        let i = get(&mut comps, &l.refdes, &l.part, "load");
        comps[i].consumption += p;
        comps[i].rails.push((net.clone(), l.current, l.basis.clone()));
    }
    let mut p_losses = 0.0;
    for ef in &eflows {
        p_losses += ef.p_loss;
        let role = match ef.kind.as_str() {
            "input" => "input",
            "pass" => "pass-through",
            _ => "regulator",
        };
        let i = get(&mut comps, &ef.refdes, &ef.part, role);
        comps[i].loss += ef.p_loss;
        if comps[i].role == "load" && role != "input" {
            comps[i].role = "load + regulator".into();
        }
    }
    comps.sort_by(|a, b| (b.consumption + b.loss).partial_cmp(&(a.consumption + a.loss)).unwrap());

    // IR-drop setups per rail
    let mut setups: Vec<RailSetup> = Vec::new();
    for n in &nets {
        let Some(&fi) = fed.get(n) else { continue };
        let fe = &elems[fi].0;
        let src = fe.out_ref.clone().unwrap_or_else(|| fe.refdes.clone());
        let mut ls: Vec<LoadSpec> = Vec::new();
        let add = |ls: &mut Vec<LoadSpec>, r: &str, i: f64, origin: &str| {
            if let Some(x) = ls.iter_mut().find(|x| x.refdes == r) {
                x.current += i;
            } else {
                ls.push(LoadSpec { refdes: r.to_string(), current: i, pads: vec![], origin: origin.to_string() });
            }
        };
        for (l, net) in loads.iter().filter(|(_, net)| net == n) {
            let _ = net;
            let at: Vec<String> = match &l.at {
                Some(a) => a.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect(),
                None => vec![l.refdes.clone()],
            };
            for r in &at {
                add(&mut ls, r, l.current / at.len() as f64, if l.basis.is_empty() { "estimate" } else { &l.basis });
            }
        }
        for &ci in children.get(n).map(|v| v.as_slice()).unwrap_or(&[]) {
            let e = &elems[ci].0;
            let r = e.in_ref.clone().unwrap_or_else(|| e.refdes.clone());
            add(&mut ls, &r, elem_in.get(&ci).copied().unwrap_or(0.0), if e.kind == "pass" { "pass-through" } else { "regulator" });
        }
        if ls.is_empty() {
            continue;
        }
        setups.push(RailSetup { net: n.clone(), voltage: volt.get(n).copied().unwrap_or(0.0), sources: vec![SourceSpec { refdes: src, pads: vec![] }], loads: ls, ground: false });
    }

    Ok(TreeResult {
        name: spec.name.clone(),
        note: spec.note.clone(),
        rails,
        elements: eflows,
        components: comps,
        p_input,
        p_loads,
        p_losses,
        setups,
        load_nets: loads.iter().map(|(_, n)| n.clone()).collect(),
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> TreeSpec {
        serde_json::from_str(
            r#"{
            "rails": [{"net":"VIN","voltage":12},{"net":"5V","voltage":5},{"net":"3V3","voltage":3.3},{"net":"-5V","voltage":-5}],
            "elements": [
              {"refdes":"J1","kind":"input","to":"VIN"},
              {"refdes":"R1","kind":"pass","from":"VIN","to":"VIN_R","r":0.01},
              {"refdes":"U1","kind":"buck","from":"VIN_R","to":"5V","eff":0.9,"imax":1.0},
              {"refdes":"U2","kind":"ldo","from":"5V","to":"3V3","iq":0.001},
              {"refdes":"U3","kind":"inverter","from":"5V","to":"-5V","eff":0.8}
            ],
            "loads": [
              {"refdes":"A","net":"3V3","current":0.5},
              {"refdes":"B","net":"5V","current":0.2},
              {"refdes":"C","net":"-5V","current":0.1}
            ]}"#,
        )
        .unwrap()
    }

    #[test]
    fn balance() {
        let r = solve_tree(&spec(), None).unwrap();
        // energy conservation: input = loads + losses
        assert!((r.p_input - r.p_loads - r.p_losses).abs() < 1e-9, "{} vs {} + {}", r.p_input, r.p_loads, r.p_losses);
        // LDO: input current = output + iq
        let u2 = r.elements.iter().find(|e| e.id == "U2").unwrap();
        assert!((u2.i_in - 0.501).abs() < 1e-12);
        assert!((u2.p_loss - (5.0 * 0.501 - 3.3 * 0.5)).abs() < 1e-12);
        // inverter: |V| used
        let u3 = r.elements.iter().find(|e| e.id == "U3").unwrap();
        assert!((u3.p_in - 0.5 / 0.8).abs() < 1e-12);
        // 5 V rail: 0.2 + 0.501 + 0.125 A
        let r5 = r.rails.iter().find(|x| x.net == "5V").unwrap();
        assert!((r5.i_total - (0.2 + 0.501 + 0.125)).abs() < 1e-12);
        assert!(!r.elements.iter().find(|e| e.id == "U1").unwrap().overload);
        // efficiency: given (U1, U3) or kisi's default as an estimate (none here); LDOs have none
        assert!(r.elements.iter().all(|e| !e.eff_estimate));
        assert_eq!(u2.eff, None);
        let mut s2 = spec();
        s2.elements[2].eff = None;
        let r2 = solve_tree(&s2, None).unwrap();
        let u1 = r2.elements.iter().find(|e| e.id == "U1").unwrap();
        assert!(u1.eff_estimate && u1.eff == Some(DEFAULT_EFF));
        // setups: the 5 V rail is fed by U1 and loads B, U2, U3
        let s5 = r.setups.iter().find(|s| s.net == "5V").unwrap();
        assert_eq!(s5.sources[0].refdes, "U1");
        assert_eq!(s5.loads.len(), 3);
        // pass element inherits the voltage
        assert_eq!(r.rails.iter().find(|x| x.net == "VIN_R").unwrap().voltage, 12.0);
    }

    #[test]
    fn overload_and_cycle() {
        let mut s = spec();
        s.loads[1].current = 2.0;
        let r = solve_tree(&s, None).unwrap();
        assert!(r.elements.iter().find(|e| e.id == "U1").unwrap().overload);
        s.elements.push(ElemSpec { refdes: "X".into(), kind: "pass".into(), from: Some("3V3".into()), to: "VIN".into(), ..Default::default() });
        assert!(solve_tree(&s, None).is_err());
    }
}

// --- power tree from KiCad fields (PI_REG + PI_LOAD) -------------------------------------
//
// PI_REG on a regulator / input / pass element, entries separated by ';':
//
//   +12V > +5V buck 90% 3A out=L3        switching converter, efficiency, rated current,
//                                         component that carries the output current
//   +5V > +3V3 ldo iq=2mA 1A             linear regulator with ground current
//   +3V3 > +3V3A pass r=0.1              ferrite / shunt (series resistance in Ω)
//   +5V > -5V inverter 70%
//   +12V input                            input connector (no source rail)
//   +5V > VCORE@0.9V buck 85%            rail without a voltage in its name: @<V>V
//   +5V > +1V8 buck 88% 2A id=U4A        one IC with several channels: unique id
//   +3V3 > "/Sensors/IMU VDD" pass r=0.5  net name with spaces (hierarchical name): in quotes
//
// '>' may also be written '->' or '→'. Values: 90% / 0.9, 3A / 500mA, iq=1mA,
// r=10m (Ω), in=/out=<refdes>, part=<text>. Loads come from PI_LOAD
// (`+1V0=2.5A, +1V8=300mA` or `+1V0=2W`; `NET=60mA@L32/L33`: the current enters the board at
// these parts' pads, e.g. the inductors of an output stage; `"/Sensors/IMU VDD"=1mA` for a name
// with spaces), the rails from all named nets. Quoting: `field_syntax`.

const KINDS: &[&str] = &["input", "buck", "boost", "ldo", "inverter", "pass", "buck-boost", "sepic"];

fn parse_amps(s: &str) -> Option<f64> {
    let t = s.trim().trim_end_matches(['A', 'a']);
    let (n, m) = if let Some(x) = t.strip_suffix('m') { (x, 1e-3) } else if let Some(x) = t.strip_suffix('u').or_else(|| t.strip_suffix('µ')) { (x, 1e-6) } else { (t, 1.0) };
    n.replace(',', ".").parse::<f64>().ok().map(|v| v * m)
}
fn parse_ohms(s: &str) -> Option<f64> {
    let t = s.trim().trim_end_matches(['Ω', 'R']).trim_end_matches("Ohm").trim_end_matches("ohm");
    let (n, m) = if let Some(x) = t.strip_suffix('m') { (x, 1e-3) } else if let Some(x) = t.strip_suffix('k') { (x, 1e3) } else { (t, 1.0) };
    n.replace(',', ".").parse::<f64>().ok().map(|v| v * m)
}

/// One PI_REG entry → (element, rail voltages given with '@').
pub fn parse_reg_entry(refdes: &str, part: &str, entry: &str) -> Result<(ElemSpec, Vec<(String, f64)>), String> {
    use crate::field_syntax as fs;
    let (lhs, rhs) = fs::split_arrow(entry).unwrap_or(("", entry.trim()));
    if entry.matches('"').count() % 2 == 1 {
        return Err("unbalanced quotes (a net name with spaces goes in \"…\")".into());
    }
    let mut toks = fs::tokens(rhs).into_iter();
    let mut volts = Vec::new();
    let mut rail = |t: &str| -> String {
        let (n, v) = fs::rail_token(t);
        if let Some(x) = v {
            volts.push((n.clone(), x));
        }
        n
    };
    let to_tok = toks.next().ok_or("output rail missing")?;
    let to = rail(to_tok);
    let from = if lhs.is_empty() { None } else { Some(rail(lhs)) };
    let mut el = ElemSpec { refdes: refdes.to_string(), part: part.to_string(), to, from, ..Default::default() };
    let mut kind = None;
    for t in toks {
        let tl = t.to_ascii_lowercase();
        if KINDS.contains(&tl.as_str()) {
            kind = Some(tl);
        } else if let Some(v) = t.strip_suffix('%') {
            el.eff = v.replace(',', ".").parse::<f64>().ok().map(|x| x / 100.0);
        } else if let Some((k, v)) = t.split_once('=') {
            match k.to_ascii_lowercase().as_str() {
                "iq" => el.iq = parse_amps(v).ok_or(format!("iq '{v}' not readable"))?,
                "r" => el.r = parse_ohms(v).ok_or(format!("r '{v}' not readable"))?,
                "imax" => el.imax = parse_amps(v),
                "eff" => el.eff = v.trim_end_matches('%').replace(',', ".").parse::<f64>().ok().map(|x| if x > 1.0 { x / 100.0 } else { x }),
                "out" => el.out_ref = Some(v.to_string()),
                "in" => el.in_ref = Some(v.to_string()),
                "id" => el.id = fs::unquote(v),
                "part" => el.part = fs::unquote(v),
                _ => return Err(format!("unknown key '{k}'")),
            }
        } else if t.ends_with(['A', 'a']) && parse_amps(t).is_some() {
            el.imax = parse_amps(t);
        } else {
            return Err(format!("'{t}' not understood"));
        }
    }
    el.kind = kind.unwrap_or_else(|| if el.from.is_none() { "input".into() } else { "buck".into() });
    if el.kind != "input" && el.from.is_none() {
        return Err("input rail missing (form INPUT > OUTPUT type)".into());
    }
    Ok((el, volts))
}

/// Source of a field laid over the footprint from `<board>.kisi.json`
/// (property `<FIELD>#source` = "kind\tnote"): (kind, note).
pub fn field_source<'a>(fp: &'a crate::board::Footprint, field: &str) -> Option<(&'a str, &'a str)> {
    let v = fp.prop(&format!("{}#source", field.to_ascii_uppercase()))?;
    Some(v.split_once('\t').unwrap_or((v, "")))
}

/// The document (with page) or url of a field's kisi.json source (property `<FIELD>#document`).
pub fn field_document<'a>(fp: &'a crate::board::Footprint, field: &str) -> Option<&'a str> {
    fp.prop(&format!("{}#document", field.to_ascii_uppercase()))
}

/// Power tree from PI_REG / PI_LOAD fields. Returns None if no PI_REG field exists.
pub fn tree_from_fields(board: &Board) -> Option<(TreeSpec, Vec<String>)> {
    if !board.footprints.iter().any(|f| f.prop("PI_REG").is_some()) {
        return None;
    }
    let from_cfg = board.footprints.iter().any(|f| field_source(f, "PI_REG").is_some() || field_source(f, "PI_LOAD").is_some());
    let name = if from_cfg { "from kisi.json / KiCad fields (PI_REG, PI_LOAD)" } else { "from KiCad fields (PI_REG, PI_LOAD)" };
    let mut spec = TreeSpec { name: name.into(), ..Default::default() };
    let mut warnings = Vec::new();
    let mut volts: HashMap<String, f64> = HashMap::new();
    for fp in &board.footprints {
        let Some(v) = fp.prop("PI_REG") else { continue };
        let entries: Vec<&str> = crate::field_syntax::split_unquoted(v, &[';']);
        for (k, entry) in entries.iter().enumerate() {
            match parse_reg_entry(&fp.reference, &fp.part_label(), entry) {
                Ok((mut el, vs)) => {
                    if let Some((_, note)) = field_source(fp, "PI_REG") {
                        el.note = note.to_string();
                    }
                    if el.id.is_empty() && entries.len() > 1 {
                        el.id = format!("{}.{}", fp.reference, k + 1);
                    }
                    for (n, x) in vs {
                        volts.insert(n, x);
                    }
                    spec.elements.push(el);
                }
                Err(e) => warnings.push(format!("{}: PI_REG '{}': {}", fp.reference, entry, e)),
            }
        }
    }
    for fp in &board.footprints {
        let Some(v) = fp.prop("PI_LOAD") else { continue };
        for item in crate::field_syntax::split_unquoted(v, &[',', ';']) {
            let Some((name, q)) = crate::field_syntax::split_once_unquoted(item, '=') else {
                warnings.push(format!("{}: PI_LOAD '{}' without value", fp.reference, item));
                continue;
            };
            let name = crate::field_syntax::unquote(name);
            let name = name.as_str();
            // load point on other parts: "NET=60mA@L32/L33" (current split over them)
            let (q, at) = match q.split_once('@') {
                Some((q, a)) => {
                    let refs: Vec<&str> = a.split(['/', '+']).map(str::trim).filter(|s| !s.is_empty()).collect();
                    for r in &refs {
                        if !board.footprints.iter().any(|f| f.reference == *r) {
                            warnings.push(format!("{}: PI_LOAD load point {} not on the board", fp.reference, r));
                        }
                    }
                    (q, Some(refs.join(",")))
                }
                None => (q, None),
            };
            let vr = volts.get(name).copied().or_else(|| crate::pi::voltage_from_name(name));
            let current = match crate::pi::parse_quantity(q) {
                Some((a, 'A')) => a,
                Some((w, 'W')) => match vr {
                    Some(x) if x.abs() > 0.0 => w / x.abs(),
                    _ => {
                        warnings.push(format!("{}: power for {} without known voltage", fp.reference, name));
                        continue;
                    }
                },
                _ => {
                    warnings.push(format!("{}: PI_LOAD value '{}' not readable", fp.reference, q.trim()));
                    continue;
                }
            };
            // basis: the kisi.json source kind (datasheet, estimate, …) or "field" for a KiCad field
            let (basis, note) = field_source(fp, "PI_LOAD").unwrap_or(("field", ""));
            spec.loads.push(LoadDef { refdes: fp.reference.clone(), net: name.to_string(), current, part: fp.part_label(), basis: basis.into(), note: note.into(), at });
        }
    }
    // rails: every net named by an element or a load
    let mut names: Vec<String> = Vec::new();
    for e in &spec.elements {
        for n in e.from.iter().chain(std::iter::once(&e.to)) {
            if !names.contains(n) {
                names.push(n.clone());
            }
        }
    }
    for l in &spec.loads {
        if !names.contains(&l.net) {
            names.push(l.net.clone());
        }
    }
    // pass elements (ferrite, shunt) carry the input voltage to their output rail
    for _ in 0..4 {
        for e in &spec.elements {
            if e.kind != "pass" {
                continue;
            }
            let Some(f) = &e.from else { continue };
            let vin = volts.get(f).copied().or_else(|| crate::pi::voltage_from_name(f));
            if let (Some(v), false) = (vin, volts.contains_key(&e.to) || crate::pi::voltage_from_name(&e.to).is_some()) {
                volts.insert(e.to.clone(), v);
            }
        }
    }
    for n in names {
        let v = volts.get(&n).copied().or_else(|| crate::pi::voltage_from_name(&n));
        if v.is_none() {
            warnings.push(format!("rail {n}: voltage unknown (put it in the name or write {n}@1.0V)"));
        }
        if crate::pi::find_net(board, &n).is_none() {
            warnings.push(format!("rail {n}: net not on the board"));
        }
        spec.rails.push(RailV { net: n, voltage: v.unwrap_or(0.0) });
    }
    Some((spec, warnings))
}

#[cfg(test)]
mod field_tests {
    use super::*;

    #[test]
    fn reg_entries() {
        let (e, _) = parse_reg_entry("U3", "TPS62130", "+12V > +5V buck 90% 3A out=L3").unwrap();
        assert_eq!((e.from.as_deref(), e.to.as_str(), e.kind.as_str()), (Some("+12V"), "+5V", "buck"));
        assert!((e.eff.unwrap() - 0.9).abs() < 1e-9 && (e.imax.unwrap() - 3.0).abs() < 1e-9);
        assert_eq!(e.out_ref.as_deref(), Some("L3"));
        let (e, _) = parse_reg_entry("U5", "", "+5V -> +3V3 ldo iq=2mA 500mA").unwrap();
        assert_eq!(e.kind, "ldo");
        assert!((e.iq - 0.002).abs() < 1e-12 && (e.imax.unwrap() - 0.5).abs() < 1e-12);
        let (e, _) = parse_reg_entry("FB1", "", "+3V3 → +3V3A pass r=50m").unwrap();
        assert!((e.r - 0.05).abs() < 1e-12);
        let (e, _) = parse_reg_entry("J1", "", "+12V input").unwrap();
        assert!(e.from.is_none() && e.kind == "input");
        let (e, v) = parse_reg_entry("U7", "", "+5V > VCORE@0.9V buck 85%").unwrap();
        assert_eq!(e.to, "VCORE");
        assert_eq!(v, vec![("VCORE".to_string(), 0.9)]);
        // net names with spaces in quotes, also with a voltage; the old forms unchanged
        let (e, v) = parse_reg_entry("FB3", "", "IMU&MIC_3V3 > \"/06 Peripherals/IMU_VDD\" pass r=0.5").unwrap();
        assert_eq!((e.from.as_deref(), e.to.as_str(), e.kind.as_str()), (Some("IMU&MIC_3V3"), "/06 Peripherals/IMU_VDD", "pass"));
        assert!(v.is_empty() && (e.r - 0.5).abs() < 1e-12);
        let (e, v) = parse_reg_entry("U4", "", "\"/Power/VIN MAIN\"@12V -> \"/Power/VDD CORE\"@0.9V buck 88% id=U4A part=\"TPS 1\"").unwrap();
        assert_eq!((e.from.as_deref(), e.to.as_str(), e.id.as_str(), e.part.as_str()), (Some("/Power/VIN MAIN"), "/Power/VDD CORE", "U4A", "TPS 1"));
        assert_eq!(v, vec![("/Power/VDD CORE".to_string(), 0.9), ("/Power/VIN MAIN".to_string(), 12.0)]);
        let (e, _) = parse_reg_entry("J1", "", "\"/In/VBUS 1\"@5V input").unwrap();
        assert!(e.from.is_none() && e.to == "/In/VBUS 1" && e.kind == "input");
        assert!(parse_reg_entry("FB3", "", "+3V3 > \"/06 Peripherals/IMU_VDD pass").is_err());
        assert!(parse_reg_entry("U8", "", "+5V > +3V3 ldo foo").is_err());
        assert!(parse_reg_entry("U9", "", "+3V3 ldo").is_err());
    }

    /// Hierarchical net names with spaces, quoted in PI_REG, PI_LOAD and PI_SOURCE: the tree,
    /// its loads and the rail setups find the board's nets.
    #[test]
    fn quoted_net_names() {
        let pcb = r#"(kicad_pcb (version 20241229) (generator "pcbnew")
  (layers (0 "F.Cu" signal) (2 "B.Cu" signal))
  (net 0 "") (net 1 "+3V3") (net 2 "/06 Peripherals/IMU_VDD") (net 3 "/P/VDD CORE")
  (footprint "R" (layer "F.Cu") (at 0 0) (property "Reference" "J1") (property "PI_REG" "+3V3 input")
    (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net 1 "+3V3")))
  (footprint "R" (layer "F.Cu") (at 5 0) (property "Reference" "FB3")
    (property "PI_REG" "+3V3 > \"/06 Peripherals/IMU_VDD\" pass r=0.5; +3V3 > \"/P/VDD CORE\"@0.9V ldo id=X")
    (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net 1 "+3V3"))
    (pad "2" smd rect (at 2 0) (size 1 1) (layers "F.Cu") (net 2 "/06 Peripherals/IMU_VDD")))
  (footprint "R" (layer "F.Cu") (at 10 0) (property "Reference" "U11")
    (property "PI_LOAD" "\"/06 Peripherals/IMU_VDD\"=0.9mA, \"/P/VDD CORE\"=10mA")
    (property "PI_SOURCE" "\"/P/VDD CORE\"=0.9V")
    (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net 2 "/06 Peripherals/IMU_VDD"))
    (pad "2" smd rect (at 2 0) (size 1 1) (layers "F.Cu") (net 3 "/P/VDD CORE"))))"#;
        let b = crate::load_pcb(pcb).unwrap();
        let (spec, w) = tree_from_fields(&b).unwrap();
        assert!(w.is_empty(), "{w:?}");
        let imu = spec.rails.iter().find(|r| r.net == "/06 Peripherals/IMU_VDD").unwrap();
        assert_eq!(imu.voltage, 3.3);
        assert_eq!(spec.rails.iter().find(|r| r.net == "/P/VDD CORE").unwrap().voltage, 0.9);
        assert_eq!(spec.loads.iter().map(|l| (l.net.as_str(), (l.current * 1e6).round())).collect::<Vec<_>>(), vec![("/06 Peripherals/IMU_VDD", 900.0), ("/P/VDD CORE", 10000.0)]);
        let t = solve_tree(&spec, Some(&b)).unwrap();
        assert!(t.warnings.is_empty(), "{:?}", t.warnings);
        assert!((t.rails.iter().find(|r| r.net == "+3V3").unwrap().i_total - (0.9e-3 + 0.01)).abs() < 1e-6);
        let (rails, w) = crate::pi::rails_from_fields(&b);
        assert!(w.is_empty(), "{w:?}");
        let core = rails.iter().find(|r| r.net == "/P/VDD CORE").unwrap();
        assert_eq!((core.voltage, core.sources.len(), core.loads.len()), (0.9, 1, 1));
        assert_eq!(rails.iter().find(|r| r.net == "/06 Peripherals/IMU_VDD").unwrap().loads[0].refdes, "U11");
    }
}

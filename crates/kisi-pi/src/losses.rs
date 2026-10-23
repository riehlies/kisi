//! Ground return, losses and power flow.
//!
//! * `ground_setups` – the return current of every component as a ground "rail" for
//!   `pi::solve_rail`: a component returns what it draws from the supply rails minus what it
//!   feeds into them (a load its full current, a buck converter takes current from ground);
//!   the supply input is the reference (0 V). Ground nets without the input (AGND, …) are
//!   joined through the parts that touch both.
//! * `loss_summary` – heat per component (consumption of loads, losses of regulators and pass
//!   elements) and copper loss per rail and layer (from the DC solutions incl. ground).
//! * `loss_map` – the same as power density (W/mm²) on one raster: per copper layer and
//!   a components plane.
//! * `power_flow` – nodes and links (W) from the input through regulators and rails to the
//!   loads, with the losses as their own sinks (for a Sankey diagram).
use crate::board::Board;
use crate::geom::{BBox, P};
use crate::pi::{is_ground_name, LoadSpec, PiResult, RailSetup, SourceSpec};
use crate::tree::TreeResult;
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};

/// Current each component returns into ground (A, + = into the ground net), and the
/// supply inputs (refdes).
fn return_currents(tree: &TreeResult) -> (BTreeMap<String, f64>, Vec<String>) {
    let volt: HashMap<&str, f64> = tree.rails.iter().map(|r| (r.net.as_str(), r.voltage)).collect();
    let sign = |v: f64| if v < 0.0 { -1.0 } else { 1.0 };
    let mut inj: BTreeMap<String, f64> = BTreeMap::new();
    let mut inputs = Vec::new();
    for e in &tree.elements {
        match e.kind.as_str() {
            "input" => {
                *inj.entry(e.refdes.clone()).or_default() -= e.i_out * sign(e.v_out);
                if !inputs.contains(&e.refdes) {
                    inputs.push(e.refdes.clone());
                }
            }
            "pass" => {} // no ground pin: in = out
            _ => *inj.entry(e.refdes.clone()).or_default() += e.i_in * sign(e.v_in) - e.i_out * sign(e.v_out),
        }
    }
    for c in &tree.components {
        for (rail, i, _) in &c.rails {
            *inj.entry(c.refdes.clone()).or_default() += i * sign(volt.get(rail.as_str()).copied().unwrap_or(1.0));
        }
    }
    (inj, inputs)
}

/// Ground return setups (one per ground net that carries return current).
pub fn ground_setups(board: &Board, tree: &TreeResult) -> (Vec<RailSetup>, Vec<String>) {
    let mut warnings = Vec::new();
    let (inj, inputs) = return_currents(tree);
    let gnets: Vec<u32> = (0..board.nets.len() as u32).filter(|&n| !board.nets[n as usize].is_empty() && is_ground_name(&board.nets[n as usize])).collect();
    if gnets.is_empty() {
        return (vec![], vec!["no ground net (GND, AGND, VSS, …) on the board: ground return not computed".into()]);
    }
    let fp_of: HashMap<&str, usize> = board.footprints.iter().enumerate().map(|(i, f)| (f.reference.as_str(), i)).collect();
    // ground pads per component and ground net (ordered: the part a ground net hangs on must not
    // depend on hash order – native and WASM runs chose different parts)
    let mut gpads: BTreeMap<usize, BTreeMap<u32, usize>> = BTreeMap::new();
    for p in &board.pads {
        if gnets.contains(&p.net) {
            *gpads.entry(p.fp as usize).or_default().entry(p.net).or_default() += 1;
        }
    }
    let nets_of = |r: &str| -> Vec<(u32, usize)> {
        fp_of.get(r).and_then(|i| gpads.get(i)).map(|m| m.iter().map(|(&n, &c)| (n, c)).collect()).unwrap_or_default()
    };
    // reference: the supply inputs; other ground nets hang on a part that touches a resolved one
    let mut source: BTreeMap<u32, (String, Option<u32>)> = BTreeMap::new(); // net -> (refdes, parent net)
    for r in &inputs {
        for (n, _) in nets_of(r) {
            source.entry(n).or_insert((r.clone(), None));
        }
    }
    if source.is_empty() {
        warnings.push(format!("ground return not computed: the supply input ({}) has no ground pads", if inputs.is_empty() { "PI_REG … input".to_string() } else { inputs.join(", ") }));
        return (vec![], warnings);
    }
    let mut order: Vec<u32> = source.keys().copied().collect();
    loop {
        let mut added = None;
        'find: for &g in &gnets {
            if source.contains_key(&g) {
                continue;
            }
            for (fi, m) in &gpads {
                if m.contains_key(&g) {
                    if let Some(&h) = m.keys().find(|h| source.contains_key(h)) {
                        added = Some((g, board.footprints[*fi].reference.clone(), h));
                        break 'find;
                    }
                }
            }
        }
        match added {
            Some((g, r, h)) => {
                source.insert(g, (r, Some(h)));
                order.push(g);
            }
            None => break,
        }
    }
    // return current per net and part: split over the part's ground nets by pad count;
    // a part that is the reference of a net puts its share on the parent net
    let mut per_net: BTreeMap<u32, BTreeMap<String, f64>> = BTreeMap::new();
    for (r, &i) in &inj {
        if i.abs() < 1e-12 {
            continue;
        }
        let ns = nets_of(r);
        if ns.is_empty() {
            if !inputs.contains(r) {
                warnings.push(format!("{r}: no ground pads, its return current ({:.3} A) is not placed", i));
            }
            continue;
        }
        let total: usize = ns.iter().map(|x| x.1).sum();
        for (n, c) in ns {
            let mut net = n;
            while let Some((sr, Some(parent))) = source.get(&net) {
                if sr == r { net = *parent; } else { break; }
            }
            *per_net.entry(net).or_default().entry(r.clone()).or_default() += i * c as f64 / total as f64;
        }
    }
    // child nets (deepest first) hand their total to their reference part on the parent net
    for &g in order.iter().rev() {
        if let Some((r, Some(parent))) = source.get(&g).cloned() {
            let sum: f64 = per_net.get(&g).map(|m| m.values().sum()).unwrap_or(0.0);
            if sum.abs() > 0.0 {
                *per_net.entry(parent).or_default().entry(r).or_default() += sum;
            }
        }
    }
    for g in &gnets {
        if !source.contains_key(g) && per_net.get(g).is_some_and(|m| !m.is_empty()) {
            warnings.push(format!("{}: not connected to the supply input's ground, its return current is not computed", board.net_name(*g)));
        }
    }
    let mut setups = Vec::new();
    for (g, (sr, _)) in &source {
        let Some(m) = per_net.get(g) else { continue };
        let loads: Vec<LoadSpec> = m
            .iter()
            .filter(|(r, i)| i.abs() > 1e-9 && *r != sr)
            .map(|(r, &i)| LoadSpec { refdes: r.clone(), current: -i, pads: vec![], origin: "return".into() })
            .collect();
        if loads.is_empty() {
            continue;
        }
        setups.push(RailSetup { net: board.net_name(*g).to_string(), voltage: 0.0, sources: vec![SourceSpec { refdes: sr.clone(), pads: vec![] }], loads, ground: true });
    }
    (setups, warnings)
}

// --- losses ------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct CompLoss {
    pub refdes: String,
    pub part: String,
    /// heat in the part (W): consumption of a load, loss of a regulator / pass element
    pub w: f64,
    pub role: String,
    pub at: Option<P>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RailLoss {
    pub net: String,
    pub ground: bool,
    pub w: f64,
    /// (copper layer index, W)
    pub layers: Vec<(u16, f64)>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct LossSummary {
    pub components: Vec<CompLoss>,
    pub rails: Vec<RailLoss>,
    /// copper loss per layer (layer index, name, W), all rails and ground
    pub layers: Vec<(u16, String, f64)>,
    pub components_w: f64,
    pub copper_w: f64,
    pub ground_w: f64,
    pub total_w: f64,
    pub notes: Vec<String>,
}

fn fp_center(board: &Board, refdes: &str) -> Option<P> {
    let i = board.footprints.iter().position(|f| f.reference == refdes)?;
    let mut bb = BBox::empty();
    for p in board.pads.iter().filter(|p| p.fp as usize == i) {
        bb.add(p.at);
    }
    if bb.is_valid() { Some([(bb.min[0] + bb.max[0]) / 2.0, (bb.min[1] + bb.max[1]) / 2.0]) } else { Some(board.footprints[i].at) }
}

pub fn loss_summary(board: &Board, tree: Option<&TreeResult>, rails: &[&PiResult]) -> LossSummary {
    let su = board.stackup();
    let mut s = LossSummary::default();
    if let Some(t) = tree {
        for c in &t.components {
            let w = c.consumption + c.loss;
            if w > 0.0 {
                s.components.push(CompLoss { refdes: c.refdes.clone(), part: c.part.clone(), w, role: c.role.clone(), at: fp_center(board, &c.refdes) });
            }
        }
        s.components.sort_by(|a, b| b.w.partial_cmp(&a.w).unwrap());
        s.notes.push("Component heat: the power a load draws is taken as heat in the part (estimate: power leaving the board, e.g. through connectors or LEDs, is included); regulator losses are put on the regulator.".into());
    }
    let mut per_layer: BTreeMap<u16, f64> = BTreeMap::new();
    for r in rails {
        let mut layers = Vec::new();
        for g in &r.layers {
            let w: f64 = r.loss[g.offset..g.offset + g.nx * g.ny].iter().map(|&x| x as f64).sum();
            layers.push((g.layer, w));
            *per_layer.entry(g.layer).or_default() += w;
        }
        s.rails.push(RailLoss { net: r.net.clone(), ground: r.ground, w: r.power_loss_w, layers });
        if r.ground { s.ground_w += r.power_loss_w } else { s.copper_w += r.power_loss_w }
    }
    s.rails.sort_by(|a, b| b.w.partial_cmp(&a.w).unwrap());
    s.layers = per_layer.into_iter().map(|(l, w)| (l, su.copper.get(l as usize).map(|c| c.name.clone()).unwrap_or_default(), w)).collect();
    s.components_w = s.components.iter().map(|c| c.w).sum();
    s.total_w = s.components_w + s.copper_w + s.ground_w;
    s
}

// --- loss map ----------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct LossMap {
    pub x0: f64,
    pub y0: f64,
    pub cell: f64,
    pub nx: usize,
    pub ny: usize,
    /// copper layer indices of the planes in `copper` (in this order)
    pub layers: Vec<u16>,
    /// highest density per copper plane and of the components plane (W/mm²)
    pub max_layer: Vec<f64>,
    pub max_components: f64,
    /// W/mm², `layers.len()` planes of nx*ny cells
    #[serde(skip)]
    pub copper: Vec<f32>,
    /// W/mm² of the components (heat over the area of their pads)
    #[serde(skip)]
    pub components: Vec<f32>,
}

/// Power density on one raster over the board. `cell` in mm (None: about 600 cells across).
pub fn loss_map(board: &Board, rails: &[&PiResult], comps: &[CompLoss], cell: Option<f64>) -> Result<LossMap, String> {
    let bb = board.bbox.ok_or("board without outline or copper")?;
    let (w, h) = (bb.max[0] - bb.min[0], bb.max[1] - bb.min[1]);
    let cell = cell.unwrap_or_else(|| (w.max(h) / 600.0).max(0.05));
    let nx = (w / cell).ceil() as usize + 1;
    let ny = (h / cell).ceil() as usize + 1;
    let per = nx * ny;
    let ncu = board.stackup().n();
    let mut layers: Vec<u16> = rails.iter().flat_map(|r| r.layers.iter().map(|g| g.layer)).collect();
    layers.sort();
    layers.dedup();
    let slot: HashMap<u16, usize> = layers.iter().enumerate().map(|(i, &l)| (l, i)).collect();
    let mut cu = vec![0f32; per * layers.len()];
    // spread W over the map cells a source rectangle covers
    let spread = |buf: &mut [f32], x0: f64, y0: f64, x1: f64, y1: f64, wv: f64| {
        let i0 = (((x0 - bb.min[0]) / cell).floor().max(0.0) as usize).min(nx - 1);
        let j0 = (((y0 - bb.min[1]) / cell).floor().max(0.0) as usize).min(ny - 1);
        let i1 = (((x1 - bb.min[0]) / cell).ceil().max(1.0) as usize - 1).clamp(i0, nx - 1);
        let j1 = (((y1 - bb.min[1]) / cell).ceil().max(1.0) as usize - 1).clamp(j0, ny - 1);
        let n = ((i1 - i0 + 1) * (j1 - j0 + 1)) as f64;
        for j in j0..=j1 {
            for i in i0..=i1 {
                buf[j * nx + i] += (wv / n) as f32;
            }
        }
    };
    for r in rails {
        for g in &r.layers {
            let s = slot[&g.layer] * per;
            let plane = &mut cu[s..s + per];
            for j in 0..g.ny {
                for i in 0..g.nx {
                    let wv = r.loss[g.offset + j * g.nx + i];
                    if wv > 0.0 {
                        let (x, y) = (g.x0 + i as f64 * r.cell, g.y0 + j as f64 * r.cell);
                        if r.cell <= cell {
                            let cx = (((x + r.cell / 2.0 - bb.min[0]) / cell).floor().max(0.0) as usize).min(nx - 1);
                            let cy = (((y + r.cell / 2.0 - bb.min[1]) / cell).floor().max(0.0) as usize).min(ny - 1);
                            plane[cy * nx + cx] += wv;
                        } else {
                            spread(plane, x, y, x + r.cell, y + r.cell, wv as f64);
                        }
                    }
                }
            }
        }
    }
    let mut cp = vec![0f32; per];
    for c in comps {
        let Some(fi) = board.footprints.iter().position(|f| f.reference == c.refdes) else { continue };
        let mut pb = BBox::empty();
        for p in board.pads.iter().filter(|p| p.fp as usize == fi) {
            let r = p.size[0].max(p.size[1]) / 2.0;
            pb.add([p.at[0] - r, p.at[1] - r]);
            pb.add([p.at[0] + r, p.at[1] + r]);
        }
        if !pb.is_valid() {
            let a = board.footprints[fi].at;
            pb.add(a);
            pb.add([a[0] + cell, a[1] + cell]);
        }
        spread(&mut cp, pb.min[0], pb.min[1], pb.max[0], pb.max[1], c.w);
    }
    let area = cell * cell;
    for x in cu.iter_mut().chain(cp.iter_mut()) {
        *x /= area as f32;
    }
    let max_layer = (0..layers.len()).map(|k| cu[k * per..(k + 1) * per].iter().fold(0f32, |a, &b| a.max(b)) as f64).collect();
    let max_components = cp.iter().fold(0f32, |a, &b| a.max(b)) as f64;
    let _ = ncu;
    Ok(LossMap { x0: bb.min[0], y0: bb.min[1], cell, nx, ny, layers, max_layer, max_components, copper: cu, components: cp })
}

// --- power flow ----------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct FlowNode {
    pub id: String,
    pub label: String,
    /// input | rail | regulator | load | loss
    pub kind: String,
    /// column from the left (input 0)
    pub depth: usize,
    /// rails merged into this node (rails behind shunts and ferrites belong to the rail before them)
    pub members: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FlowLink {
    pub from: String,
    pub to: String,
    pub w: f64,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct PowerFlow {
    pub nodes: Vec<FlowNode>,
    pub links: Vec<FlowLink>,
}

/// Flow of power through the tree (W). `copper`: copper loss per rail (after the DC solution),
/// `ground_w`: copper loss of the ground return. Rails behind pass elements (shunts, ferrites)
/// are merged into the rail before them; the pass loss becomes a loss link.
pub fn power_flow(tree: &TreeResult, copper: &[(String, f64)], ground_w: f64) -> PowerFlow {
    let short = |n: &str| n.rsplit('/').next().unwrap_or(n).to_string();
    let volt: HashMap<&str, f64> = tree.rails.iter().map(|r| (r.net.as_str(), r.voltage)).collect();
    // rail -> group (the first rail upstream that is not behind a pass element)
    let mut up: HashMap<&str, &str> = HashMap::new();
    for e in &tree.elements {
        if e.kind == "pass" {
            if let Some(f) = &e.from {
                up.insert(e.to.as_str(), f.as_str());
            }
        }
    }
    let group = |n: &str| -> String {
        let mut g = n;
        for _ in 0..64 {
            match up.get(g) {
                Some(p) => g = p,
                None => break,
            }
        }
        g.to_string()
    };
    let mut members: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for r in &tree.rails {
        members.entry(group(&r.net)).or_default().push(r.net.clone());
    }
    // columns: input 0, rail group = feeding regulator + 1, regulator = its input group + 1
    let mut gdepth: HashMap<String, usize> = HashMap::new();
    let regs: Vec<&crate::tree::ElemFlow> = tree.elements.iter().filter(|e| e.kind != "pass").collect();
    for e in regs.iter().filter(|e| e.kind == "input") {
        gdepth.insert(group(&e.to), 1);
    }
    for _ in 0..regs.len() + 1 {
        let mut changed = false;
        for e in regs.iter().filter(|e| e.kind != "input") {
            if let Some(&d) = e.from.as_deref().and_then(|f| gdepth.get(&group(f))) {
                let g = group(&e.to);
                if gdepth.get(&g).is_none_or(|&x| x < d + 2) {
                    gdepth.insert(g, d + 2);
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }
    let mut f = PowerFlow::default();
    let mut links: BTreeMap<(String, String), f64> = BTreeMap::new();
    let mut link = |a: String, b: String, w: f64| {
        if w > 0.0 {
            *links.entry((a, b)).or_default() += w;
        }
    };
    let node = |f: &mut PowerFlow, id: String, label: String, kind: &str, depth: usize, m: Vec<String>| {
        if !f.nodes.iter().any(|x| x.id == id) {
            f.nodes.push(FlowNode { id, label, kind: kind.into(), depth, members: m });
        }
    };
    for (g, m) in &members {
        let label = if m.len() > 1 { format!("{} (+{})", short(g), m.len() - 1) } else { short(g) };
        node(&mut f, format!("rail:{g}"), label, "rail", gdepth.get(g).copied().unwrap_or(1), m.clone());
    }
    for e in &tree.elements {
        let to = format!("rail:{}", group(&e.to));
        match e.kind.as_str() {
            "input" => {
                let id = format!("in:{}", e.id);
                node(&mut f, id.clone(), format!("{} {}", e.refdes, e.part).trim().to_string(), "input", 0, vec![]);
                link(id, to, e.p_out);
            }
            "pass" => {
                if let Some(fr) = &e.from {
                    node(&mut f, "loss:pass".into(), "Shunts and ferrites".into(), "loss", 0, vec![]);
                    link(format!("rail:{}", group(fr)), "loss:pass".into(), e.p_loss);
                }
            }
            _ => {
                let id = format!("el:{}", e.id);
                let d = e.from.as_deref().and_then(|x| gdepth.get(&group(x))).map(|d| d + 1).unwrap_or(1);
                node(&mut f, id.clone(), format!("{} {}", e.refdes, e.part).trim().to_string(), "regulator", d, vec![]);
                if let Some(fr) = &e.from {
                    link(format!("rail:{}", group(fr)), id.clone(), e.p_in);
                }
                link(id.clone(), to, e.p_out);
                node(&mut f, "loss:reg".into(), "Regulator losses".into(), "loss", 0, vec![]);
                link(id, "loss:reg".into(), e.p_loss);
            }
        }
    }
    for c in &tree.components {
        for (rail, i, _) in &c.rails {
            let w = i * volt.get(rail.as_str()).copied().unwrap_or(0.0).abs();
            if w > 0.0 {
                let id = format!("load:{}", c.refdes);
                node(&mut f, id.clone(), format!("{} {}", c.refdes, c.part).trim().to_string(), "load", 0, vec![]);
                link(format!("rail:{}", group(rail)), id, w);
            }
        }
    }
    for (net, w) in copper {
        if *w > 0.0 && volt.contains_key(net.as_str()) {
            node(&mut f, "loss:copper".into(), "Copper (rails)".into(), "loss", 0, vec![]);
            link(format!("rail:{}", group(net)), "loss:copper".into(), *w);
        }
    }
    if ground_w > 0.0 {
        if let Some(inp) = f.nodes.iter().find(|n| n.kind == "input").map(|n| n.id.clone()) {
            node(&mut f, "loss:ground".into(), "Copper (ground return)".into(), "loss", 0, vec![]);
            link(inp, "loss:ground".into(), ground_w);
        }
    }
    // loads and losses in the last column
    let last = f.nodes.iter().filter(|n| n.kind != "load" && n.kind != "loss").map(|n| n.depth).max().unwrap_or(0) + 1;
    for n in f.nodes.iter_mut() {
        if n.kind == "load" || n.kind == "loss" {
            n.depth = last;
        }
    }
    f.links = links.into_iter().map(|((a, b), w)| FlowLink { from: a, to: b, w }).collect();
    f
}

#[cfg(test)]
mod tests {
    use super::*;

    const PCB: &str = r#"(kicad_pcb (version 20240108) (generator "pcbnew")
  (general (thickness 1.6))
  (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
  (setup (stackup
    (layer "F.Cu" (type "copper") (thickness 0.035))
    (layer "dielectric 1" (type "core") (thickness 1.5) (material "FR4") (epsilon_r 4.5) (loss_tangent 0.02))
    (layer "B.Cu" (type "copper") (thickness 0.035))))
  (net 0 "") (net 1 "+5V") (net 2 "GND") (net 3 "+3V3")
  (footprint "J" (layer "F.Cu") (at 0 0) (property "Reference" "J1") (property "Value" "jack")
    (property "PI_REG" "+5V input")
    (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net 1 "+5V"))
    (pad "2" smd rect (at 0 2) (size 1 1) (layers "F.Cu") (net 2 "GND")))
  (footprint "U" (layer "F.Cu") (at 10 0) (property "Reference" "U1") (property "Value" "buck")
    (property "PI_REG" "+5V > +3V3 buck 80%")
    (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net 1 "+5V"))
    (pad "2" smd rect (at 0 2) (size 1 1) (layers "F.Cu") (net 2 "GND"))
    (pad "3" smd rect (at 2 0) (size 1 1) (layers "F.Cu") (net 3 "+3V3")))
  (footprint "U" (layer "F.Cu") (at 20 0) (property "Reference" "U2") (property "Value" "MCU")
    (property "PI_LOAD" "+3V3=1A")
    (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net 3 "+3V3"))
    (pad "2" smd rect (at 0 2) (size 1 1) (layers "F.Cu") (net 2 "GND")))
  (segment (start 0 0) (end 10 0) (width 1) (layer "F.Cu") (net 1))
  (segment (start 12 0) (end 20 0) (width 1) (layer "F.Cu") (net 3))
  (segment (start 0 2) (end 20 2) (width 1) (layer "F.Cu") (net 2)))
"#;

    #[test]
    fn ground_return_and_losses() {
        let board = crate::load_pcb(PCB).unwrap();
        let (spec, w) = crate::tree::tree_from_fields(&board).unwrap();
        assert!(w.is_empty(), "{w:?}");
        let t = crate::tree::solve_tree(&spec, Some(&board)).unwrap();
        let (gs, gw) = ground_setups(&board, &t);
        assert!(gw.is_empty(), "{gw:?}");
        assert_eq!(gs.len(), 1);
        let g = &gs[0];
        assert!(g.ground && g.net == "GND" && g.sources[0].refdes == "J1");
        // U2 returns its 1 A; the buck takes I_out - I_in = 1 - 3.3/(5·0.8) from ground
        let i = |r: &str| g.loads.iter().find(|l| l.refdes == r).unwrap().current;
        assert!((i("U2") + 1.0).abs() < 1e-9);
        assert!((i("U1") - (1.0 - 3.3 / 4.0)).abs() < 1e-9);
        let p = crate::pi::PiParams::default();
        let gr = crate::pi::solve_rail(&board, g, &p).unwrap();
        // ground rises towards U2, the far end
        let u2 = gr.loads.iter().find(|l| l.refdes == "U2").unwrap();
        assert!(u2.v_max.unwrap() > 0.0 && gr.max_drop_mv.unwrap() > 0.0);
        let cell_sum: f64 = gr.loss.iter().map(|&x| x as f64).sum();
        assert!((cell_sum - gr.power_loss_w).abs() < 1e-6 * gr.power_loss_w.max(1e-9));
        let rails: Vec<PiResult> = t.setups.iter().map(|s| crate::pi::solve_rail(&board, s, &p).unwrap()).collect();
        let mut all: Vec<&PiResult> = rails.iter().collect();
        all.push(&gr);
        let s = loss_summary(&board, Some(&t), &all);
        assert!((s.components_w - (3.3 + (5.0 * 3.3 / 4.0 - 3.3))).abs() < 1e-9, "{}", s.components_w);
        assert!(s.ground_w > 0.0 && s.copper_w > 0.0);
        let layer_sum: f64 = s.layers.iter().map(|l| l.2).sum();
        assert!((layer_sum - s.copper_w - s.ground_w).abs() < 1e-6);
        let m = loss_map(&board, &all, &s.components, Some(0.5)).unwrap();
        let tot: f64 = m.copper.iter().chain(m.components.iter()).map(|&x| x as f64 * 0.25).sum();
        assert!((tot - s.total_w).abs() < 1e-3 * s.total_w, "{tot} vs {}", s.total_w);
        let f = power_flow(&t, &[("+3V3".into(), 0.01)], s.ground_w);
        assert!(f.nodes.iter().any(|n| n.id == "loss:ground") && f.links.iter().any(|l| l.to == "loss:reg"));
    }
}

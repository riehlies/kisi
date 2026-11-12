//! Ground return, losses and power flow.
//!
//! * `ground_setups` – the return current of every component as a ground "rail" for
//!   `pi::solve_rail`: a component returns what it draws from the supply rails minus what it
//!   feeds into them (a load its full current, a buck converter takes current from ground);
//!   the supply input is the reference (0 V). Ground nets without the input (AGND, …) are
//!   joined through the parts that touch both.
//! * `loss_summary` – heat per component (consumption of loads, losses of regulators and pass
//!   elements) and copper loss per rail and layer (from the DC solutions incl. ground).
//! * `loss_map` – the same as power density (W per mm² of copper) on one raster over the board, all
//!   layers summed, and a components plane; `loss_layer` – one copper layer on the rails' own DC grids
//!   (display only: the viewer's Losses view), both without the via barrels' loss; `loss_vias` – that
//!   loss per via and plated hole, with its current and an IPC-2152 capacity estimate (the view's markers).
//! * `power_flow` – nodes and links (W) from the input through regulators and rails to the
//!   loads, with the losses as their own sinks (for a Sankey diagram).
use crate::board::Board;
use crate::geom::{BBox, P};
use crate::pi::{is_ground_name, LoadSpec, PiResult, RailSetup, SourceSpec};
use crate::tree::TreeResult;
use serde::Serialize;
use std::borrow::Cow;
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
    let gnets: Vec<u32> = (0..board.nets.len() as u32)
        .filter(|&n| !board.nets[n as usize].is_empty() && is_ground_name(&board.nets[n as usize]))
        .collect();
    if gnets.is_empty() {
        return (vec![], vec!["no ground net (GND, AGND, VSS, …) on the board: ground return not computed".into()]);
    }
    let fp_of: HashMap<&str, usize> =
        board.footprints.iter().enumerate().map(|(i, f)| (f.reference.as_str(), i)).collect();
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
        warnings.push(format!(
            "ground return not computed: the supply input ({}) has no ground pads",
            if inputs.is_empty() { "PI_REG … input".to_string() } else { inputs.join(", ") }
        ));
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
                if sr == r {
                    net = *parent;
                } else {
                    break;
                }
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
            warnings.push(format!(
                "{}: not connected to the supply input's ground, its return current is not computed",
                board.net_name(*g)
            ));
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
        setups.push(RailSetup {
            net: board.net_name(*g).to_string(),
            voltage: 0.0,
            sources: vec![SourceSpec { refdes: sr.clone(), pads: vec![] }],
            loads,
            ground: true,
        });
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
    if bb.is_valid() {
        Some([(bb.min[0] + bb.max[0]) / 2.0, (bb.min[1] + bb.max[1]) / 2.0])
    } else {
        Some(board.footprints[i].at)
    }
}

pub fn loss_summary(board: &Board, tree: Option<&TreeResult>, rails: &[&PiResult]) -> LossSummary {
    let su = board.stackup();
    let mut s = LossSummary::default();
    if let Some(t) = tree {
        for c in &t.components {
            let w = c.consumption + c.loss;
            if w > 0.0 {
                s.components.push(CompLoss {
                    refdes: c.refdes.clone(),
                    part: c.part.clone(),
                    w,
                    role: c.role.clone(),
                    at: fp_center(board, &c.refdes),
                });
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
        if r.ground {
            s.ground_w += r.power_loss_w
        } else {
            s.copper_w += r.power_loss_w
        }
    }
    s.rails.sort_by(|a, b| b.w.partial_cmp(&a.w).unwrap());
    s.layers = per_layer
        .into_iter()
        .map(|(l, w)| (l, su.copper.get(l as usize).map(|c| c.name.clone()).unwrap_or_default(), w))
        .collect();
    s.components_w = s.components.iter().map(|c| c.w).sum();
    s.total_w = s.components_w + s.copper_w + s.ground_w;
    s
}

// --- loss map ----------------------------------------------------------------------
//
// Display data (the viewer's Losses view); the sums above and the temperature (`thermal`, from
// `PiResult::loss` per cell) do not use it. One layer on the rails' own DC cells as W per mm² of the
// cell's copper (`PiResult::cu`: a cell half covered by copper is not half as dark), all layers and the
// parts on one raster as a heat source per mm² of board.

#[derive(Debug, Clone, Serialize)]
pub struct LossMap {
    pub x0: f64,
    pub y0: f64,
    pub cell: f64,
    pub nx: usize,
    pub ny: usize,
    /// copper layer indices with solved copper
    pub layers: Vec<u16>,
    /// highest density of the copper (all layers) and of the components plane (W per mm² of board)
    pub max_copper: f64,
    pub max_components: f64,
    /// W per mm² of board, `layers.len()` planes of nx*ny cells (copper loss of every rail on the layer)
    #[serde(skip)]
    pub copper: Vec<f32>,
    /// W/mm² of the components (heat over the area of their pads)
    #[serde(skip)]
    pub components: Vec<f32>,
}

/// The parts of the interval a0..a1 in the cells of a raster (origin, cell, n cells): (index, length).
/// The lengths add up to a1 - a0: a part beyond the raster goes to the cells at its edge (copper can
/// reach beyond the board's box).
fn overlaps(a0: f64, a1: f64, origin: f64, cell: f64, n: usize, out: &mut Vec<(usize, f64)>) {
    out.clear();
    let end = origin + n as f64 * cell;
    let (b0, b1) = (a0.clamp(origin, end), a1.clamp(origin, end));
    if b1 - b0 <= 1e-12 {
        out.push((if a0 >= end { n - 1 } else { 0 }, a1 - a0));
        return;
    }
    let scale = (a1 - a0) / (b1 - b0);
    let i0 = (((b0 - origin) / cell).floor().max(0.0) as usize).min(n - 1);
    let i1 = (((b1 - origin) / cell).ceil().max(1.0) as usize - 1).clamp(i0, n - 1);
    for i in i0..=i1 {
        let (c0, c1) = (origin + i as f64 * cell, origin + (i + 1) as f64 * cell);
        let len = b1.min(c1) - b0.max(c0);
        if len > 0.0 {
            out.push((i, len * scale));
        }
    }
}

/// A layer grid's loss per cell (W) for the display: `PiResult::loss` without the via barrels' halves
/// (`PiResult::barrels`: shown as vias of their own, `loss_vias` – half a barrel's I²R on one cell is a
/// point loss whose density grows with 1/cell²), and the cells `pi::solve_rail` joins into one node
/// around a via or plated hole wider than 1.4 cells (an equipotential disc, its loss booked on one of
/// them) share the rest evenly. Cells without copper (`cu` 0) carry none.
fn display_loss<'a>(board: &Board, r: &'a PiResult, g: &crate::pi::LayerGrid) -> Cow<'a, [f32]> {
    let per = g.nx * g.ny;
    let mut own = Cow::Borrowed(&r.loss[g.offset..g.offset + per]);
    for b in &r.barrels {
        for &(l, k) in &b.ends {
            if l == g.layer && (k as usize) < per {
                let o = own.to_mut();
                o[k as usize] = (o[k as usize] - (b.loss_w / 2.0) as f32).max(0.0);
            }
        }
    }
    let Some(net) = crate::pi::find_net(board, &r.net) else { return own };
    let grid = crate::pi::Grid { x0: g.x0, y0: g.y0, cell: r.cell, nx: g.nx, ny: g.ny };
    let l = g.layer;
    let discs: Vec<(P, f64)> = board
        .vias
        .iter()
        .filter(|v| v.net == net && v.from.min(v.to) <= l && l <= v.from.max(v.to))
        .map(|v| (v.at, v.drill / 2.0))
        .chain(
            board
                .pads
                .iter()
                .filter(|p| p.net == net && p.drill > 0.0 && p.layers.contains(&l))
                .map(|p| (p.at, p.drill / 2.0)),
        )
        .filter(|&(_, rw)| rw >= 0.7 * r.cell)
        .collect();
    if discs.is_empty() {
        return own;
    }
    let mut out = own.into_owned();
    // discs that share a cell are one node (union of their cells)
    let mut up: Vec<usize> = (0..discs.len()).collect();
    fn root(up: &mut [usize], mut x: usize) -> usize {
        while up[x] != x {
            up[x] = up[up[x]];
            x = up[x];
        }
        x
    }
    let mut owner: HashMap<usize, usize> = HashMap::new();
    let mut cells: Vec<(usize, usize)> = Vec::new(); // (cell, disc)
    for (d, &(at, rw)) in discs.iter().enumerate() {
        for k in grid.near(at, rw + 0.35 * r.cell) {
            if r.cu[g.offset + k] == 0 {
                continue;
            }
            match owner.get(&k) {
                None => {
                    owner.insert(k, d);
                    cells.push((k, d));
                }
                Some(&o) => {
                    let (a, b) = (root(&mut up, o), root(&mut up, d));
                    up[a.max(b)] = a.min(b);
                }
            }
        }
    }
    let mut sum: HashMap<usize, (f64, usize)> = HashMap::new();
    for &(k, d) in &cells {
        let e = sum.entry(root(&mut up, d)).or_default();
        e.0 += out[k] as f64;
        e.1 += 1;
    }
    for &(k, d) in &cells {
        let (w, n) = sum[&root(&mut up, d)];
        out[k] = (w / n as f64) as f32;
    }
    Cow::Owned(out)
}

/// Heat source per mm² of board on one raster over the board, per copper layer and of the parts. `cell` in
/// mm (None: about 600 cells across). Every DC cell's loss goes to the map cells its square overlaps, by
/// the overlapping area (no beat between the two rasters).
pub fn loss_map(board: &Board, rails: &[&PiResult], comps: &[CompLoss], cell: Option<f64>) -> Result<LossMap, String> {
    let bb = board.bbox.ok_or("board without outline or copper")?;
    let (w, h) = (bb.max[0] - bb.min[0], bb.max[1] - bb.min[1]);
    let cell = cell.unwrap_or_else(|| (w.max(h) / 600.0).max(0.05));
    let nx = (w / cell).ceil() as usize + 1;
    let ny = (h / cell).ceil() as usize + 1;
    let per = nx * ny;
    let mut layers: Vec<u16> = rails.iter().flat_map(|r| r.layers.iter().map(|g| g.layer)).collect();
    layers.sort();
    layers.dedup();
    let area = cell * cell;
    let mut cu = vec![0f32; per * layers.len()];
    let mut wl = vec![0f64; per];
    let (mut ox, mut oy) = (Vec::new(), Vec::new());
    let mut sum = vec![0f32; per];
    for (s, &l) in layers.iter().enumerate() {
        layer_raster(board, rails, l, (bb.min[0], bb.min[1], cell, nx, ny), &mut wl, &mut ox, &mut oy);
        for k in 0..per {
            let d = (wl[k] / area) as f32;
            cu[s * per + k] = d;
            sum[k] += d;
        }
    }
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
    for x in cp.iter_mut() {
        *x /= area as f32;
    }
    let max_copper = sum.iter().fold(0f32, |a, &b| a.max(b)) as f64;
    let max_components = cp.iter().fold(0f32, |a, &b| a.max(b)) as f64;
    Ok(LossMap {
        x0: bb.min[0],
        y0: bb.min[1],
        cell,
        nx,
        ny,
        layers,
        max_copper,
        max_components,
        copper: cu,
        components: cp,
    })
}

/// One layer of all rails on a raster (x0, y0, cell, nx, ny): loss (W) per map cell into `wl`
/// (overwritten), by area overlap with the DC cells. Returns its sum.
fn layer_raster(
    board: &Board,
    rails: &[&PiResult],
    l: u16,
    (x0, y0, cell, nx, ny): (f64, f64, f64, usize, usize),
    wl: &mut [f64],
    ox: &mut Vec<(usize, f64)>,
    oy: &mut Vec<(usize, f64)>,
) -> f64 {
    wl.fill(0.0);
    let mut ws = 0.0;
    for r in rails {
        let a = r.cell * r.cell;
        for g in r.layers.iter().filter(|g| g.layer == l) {
            if r.cu.len() < g.offset + g.nx * g.ny {
                continue;
            }
            let loss = display_loss(board, r, g);
            for j in 0..g.ny {
                let gy = g.y0 + j as f64 * r.cell;
                overlaps(gy, gy + r.cell, y0, cell, ny, oy);
                for i in 0..g.nx {
                    let k = j * g.nx + i;
                    let wv = loss[k] as f64;
                    if r.cu[g.offset + k] == 0 || wv == 0.0 {
                        continue;
                    }
                    let gx = g.x0 + i as f64 * r.cell;
                    overlaps(gx, gx + r.cell, x0, cell, nx, ox);
                    ws += wv;
                    for &(jj, ly) in oy.iter() {
                        for &(ii, lx) in ox.iter() {
                            wl[jj * nx + ii] += wv * lx * ly / a;
                        }
                    }
                }
            }
        }
    }
    ws
}

/// Current capacity (A) of a barrel with plating cross-section `area_mm2` at the temperature rise of
/// `data/thermal.json` (`via_current`): the IPC-2152 conductor chart fit solved for I. An estimate.
pub fn via_capacity(area_mm2: f64) -> f64 {
    let c = &crate::thermal::data().via_current;
    let dt = c.delta_t_k;
    let a_mil2 = area_mm2 / (0.0254 * 0.0254);
    let k = c.a[0] * dt.powf(c.a[1]) + c.a[2];
    let e = c.b[0] * dt.powf(c.b[1]) + c.b[2];
    (a_mil2 / k).powf(1.0 / e)
}

/// The barrel loss of one via or plated hole (all its steps of one rail).
#[derive(Debug, Clone, Serialize)]
pub struct ViaLoss {
    pub net: String,
    pub at: P,
    pub drill: f64,
    pub hole: bool,
    /// the contact layers its current flows between (outermost)
    pub from: u16,
    pub to: u16,
    /// plating cross-section π·(d − t)·t (mm²)
    pub area_mm2: f64,
    /// the largest current of its steps (A), its density in the plating (A/mm²)
    pub current: f64,
    pub j_a_mm2: f64,
    /// I²R of its steps (W)
    pub loss_w: f64,
    /// capacity estimate (`via_capacity`) and the current against it
    pub capacity_a: f64,
    pub load: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ViaLosses {
    /// barrel loss of every via and plated hole (W) and how many carry current
    pub total_w: f64,
    pub count: usize,
    /// the temperature rise of the capacity estimate (K)
    pub delta_t_k: f64,
    /// the vias with at least `VIA_LIST_MIN` of the largest loss, the most first
    pub list: Vec<ViaLoss>,
}

/// Share of the largest via loss down to which `loss_vias` lists vias (the viewer's lowest filter).
pub const VIA_LIST_MIN: f64 = 0.01;

/// The via barrels' loss of the rails, one entry per via or plated hole and rail: the loss the layer
/// maps (`loss_layer`, `loss_map`) leave out. Layer display loss + `total_w` = the rails' copper loss.
pub fn loss_vias(rails: &[&PiResult]) -> ViaLosses {
    let mut all: Vec<ViaLoss> = Vec::new();
    let mut total_w = 0.0;
    for r in rails {
        let mut by: HashMap<(i64, i64), usize> = HashMap::new();
        let first = all.len();
        for b in &r.barrels {
            total_w += b.loss_w;
            let key = ((b.at[0] * 1e4).round() as i64, (b.at[1] * 1e4).round() as i64);
            let i = *by.entry(key).or_insert_with(|| {
                all.push(ViaLoss {
                    net: r.net.clone(),
                    at: b.at,
                    drill: b.drill,
                    hole: b.hole,
                    from: b.from,
                    to: b.to,
                    area_mm2: b.area_mm2,
                    current: 0.0,
                    j_a_mm2: 0.0,
                    loss_w: 0.0,
                    capacity_a: 0.0,
                    load: 0.0,
                });
                all.len() - 1
            });
            let v = &mut all[i];
            v.from = v.from.min(b.from);
            v.to = v.to.max(b.to);
            v.current = v.current.max(b.current);
            v.loss_w += b.loss_w;
        }
        for v in &mut all[first..] {
            v.j_a_mm2 = v.current / v.area_mm2.max(1e-12);
            v.capacity_a = via_capacity(v.area_mm2);
            v.load = v.current / v.capacity_a.max(1e-12);
        }
    }
    let count = all.len();
    all.sort_by(|a, b| b.loss_w.total_cmp(&a.loss_w));
    let max = all.first().map_or(0.0, |v| v.loss_w);
    all.retain(|v| v.loss_w >= VIA_LIST_MIN * max);
    ViaLosses { total_w, count, delta_t_k: crate::thermal::data().via_current.delta_t_k, list: all }
}

/// The rails' copper on one layer on their own DC grids (the Losses view of a layer).
#[derive(Debug, Clone, Serialize)]
pub struct RailPlane {
    pub net: String,
    pub ground: bool,
    /// the rail's DC cell (mm) and the cropped grid: its copper on the layer plus `LOSS_LAYER_MARGIN`
    /// cells
    pub cell: f64,
    pub x0: f64,
    pub y0: f64,
    pub nx: usize,
    pub ny: usize,
    /// its copper cells: `LayerLoss::cells` / `density` from `offset`, `n` of them
    pub offset: usize,
    pub n: usize,
    /// highest density (W/mm²) and the rail's loss on the layer (W)
    pub max: f64,
    pub w: f64,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct LayerLoss {
    pub layer: u16,
    pub planes: Vec<RailPlane>,
    pub max: f64,
    /// the copper cells of the planes, row by row: index in the plane's grid (j·nx + i)
    #[serde(skip)]
    pub cells: Vec<u32>,
    /// and W per mm² of copper there
    #[serde(skip)]
    pub density: Vec<f32>,
}

/// Cells kept around a rail's copper in `loss_layer` (the viewer extends its values two cells and
/// interpolates with the next one).
pub const LOSS_LAYER_MARGIN: usize = 3;

/// The copper loss of every rail on copper layer `layer` as W per mm² of copper on the rail's own
/// DC cells (no resampling), its copper cells only.
pub fn loss_layer(board: &Board, rails: &[&PiResult], layer: u16) -> LayerLoss {
    let mut out = LayerLoss { layer, ..Default::default() };
    for r in rails {
        let a = r.cell * r.cell;
        for g in r.layers.iter().filter(|g| g.layer == layer) {
            if r.cu.len() < g.offset + g.nx * g.ny {
                continue;
            }
            let loss = display_loss(board, r, g);
            let cu = &r.cu[g.offset..g.offset + g.nx * g.ny];
            let (mut i0, mut i1, mut j0, mut j1) = (usize::MAX, 0, usize::MAX, 0);
            for j in 0..g.ny {
                for i in 0..g.nx {
                    if cu[j * g.nx + i] > 0 {
                        (i0, i1, j0, j1) = (i0.min(i), i1.max(i), j0.min(j), j1.max(j));
                    }
                }
            }
            if i0 == usize::MAX {
                continue;
            }
            let m = LOSS_LAYER_MARGIN;
            let (i0, j0, i1, j1) =
                (i0.saturating_sub(m), j0.saturating_sub(m), (i1 + m).min(g.nx - 1), (j1 + m).min(g.ny - 1));
            let (w, h) = (i1 - i0 + 1, j1 - j0 + 1);
            let offset = out.density.len();
            let (mut max, mut sum) = (0f64, 0f64);
            for j in j0..=j1 {
                for i in i0..=i1 {
                    let k = j * g.nx + i;
                    if cu[k] == 0 {
                        continue;
                    }
                    let d = loss[k] as f64 / (cu[k] as f64 / crate::pi::CU_STEPS as f64 * a);
                    max = max.max(d);
                    sum += loss[k] as f64;
                    out.cells.push(((j - j0) * w + (i - i0)) as u32);
                    out.density.push(d as f32);
                }
            }
            out.max = out.max.max(max);
            out.planes.push(RailPlane {
                net: r.net.clone(),
                ground: r.ground,
                cell: r.cell,
                x0: g.x0 + i0 as f64 * r.cell,
                y0: g.y0 + j0 as f64 * r.cell,
                nx: w,
                ny: h,
                offset,
                n: out.density.len() - offset,
                max,
                w: sum,
            });
        }
    }
    out
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
        let comp: f64 = m.components.iter().map(|&x| x as f64 * 0.25).sum();
        assert!((comp - s.components_w).abs() < 1e-6 * s.components_w, "{comp} vs {}", s.components_w);
        // the map raster keeps every layer's loss (area overlap; W per mm² of board), and the layer planes on
        // the DC grids carry the same loss
        let per = m.nx * m.ny;
        for &(l, _, w) in &s.layers {
            let slot = m.layers.iter().position(|&x| x == l).unwrap();
            let mw: f64 = m.copper[slot * per..(slot + 1) * per].iter().map(|&x| x as f64 * 0.25).sum();
            assert!((mw - w).abs() < 1e-5 * w, "layer {l}: {mw} vs {w}");
            let ll = loss_layer(&board, &all, l);
            let lw: f64 = ll.planes.iter().map(|p| p.w).sum();
            assert!((lw - w).abs() < 1e-5 * w, "layer {l}: {lw} vs {w}");
        }
        let f = power_flow(&t, &[("+3V3".into(), 0.01)], s.ground_w);
        assert!(f.nodes.iter().any(|n| n.id == "loss:ground") && f.links.iter().any(|l| l.to == "loss:reg"));
    }

    /// A strip 20 × 6 mm carrying 1 A from end to end: uniform density I²·Rs / W² in its middle, on the
    /// DC grid and on a map raster whose cell (0.2042 mm) is not a multiple of the DC cell (0.125 mm) –
    /// binning the DC cells by their centres gave map cells with one or two of them (a beat of ×2). The
    /// strip covers the map cells of its middle, so there the density per board area is the same.
    #[test]
    fn loss_map_without_beat() {
        let board = crate::load_pcb(
            r#"(kicad_pcb (version 20241229) (generator "pcbnew")
  (layers (0 "F.Cu" signal) (2 "B.Cu" signal))
  (setup (stackup
    (layer "F.Cu" (type "copper") (thickness 0.035))
    (layer "dielectric 1" (type "core") (thickness 1.5) (material "FR4") (epsilon_r 4.5) (loss_tangent 0.02))
    (layer "B.Cu" (type "copper") (thickness 0.035))))
  (net 0 "") (net 1 "+3V3")
  (footprint "R" (layer "F.Cu") (at 0 0) (property "Reference" "J1") (property "PI_SOURCE" "+3V3")
    (pad "1" smd rect (at 0.5 3) (size 1 6) (layers "F.Cu") (net 1 "+3V3")))
  (footprint "R" (layer "F.Cu") (at 0 0) (property "Reference" "U1") (property "PI_LOAD" "+3V3=1A")
    (pad "1" smd rect (at 19.5 3) (size 1 6) (layers "F.Cu") (net 1 "+3V3")))
  (zone (net 1) (net_name "+3V3") (layer "F.Cu") (filled_polygon (layer "F.Cu") (pts (xy 0 0) (xy 20 0) (xy 20 6) (xy 0 6)))))"#,
        )
        .unwrap();
        let (setups, _) = crate::pi::rails_from_fields(&board);
        let p = crate::pi::PiParams { cell: Some(0.125), ..Default::default() };
        let r = crate::pi::solve_rail(&board, &setups[0], &p).unwrap();
        let expect = 1.72e-8 / 35e-6 / 36.0; // W/mm²
        let spread = |v: &[f64]| {
            let mean = v.iter().sum::<f64>() / v.len() as f64;
            let sd = (v.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / v.len() as f64).sqrt();
            (mean, sd / mean)
        };
        let middle = |x: f64, y: f64| (5.0..15.0).contains(&x) && (1.0..5.0).contains(&y);
        let ll = loss_layer(&board, &[&r], 0);
        let pl = &ll.planes[0];
        let own: Vec<f64> = (pl.offset..pl.offset + pl.n)
            .filter(|&q| {
                let k = ll.cells[q] as usize;
                middle(pl.x0 + ((k % pl.nx) as f64 + 0.5) * pl.cell, pl.y0 + ((k / pl.nx) as f64 + 0.5) * pl.cell)
            })
            .map(|q| ll.density[q] as f64)
            .collect();
        let (mean, cv) = spread(&own);
        assert!((mean / expect - 1.0).abs() < 0.02 && cv < 0.01, "DC grid: {mean} vs {expect}, spread {cv}");
        let m = loss_map(&board, &[&r], &[], Some(0.2042)).unwrap();
        let map: Vec<f64> = (0..m.nx * m.ny)
            .filter(|&k| middle(m.x0 + ((k % m.nx) as f64 + 0.5) * m.cell, m.y0 + ((k / m.nx) as f64 + 0.5) * m.cell))
            .map(|k| m.copper[k] as f64)
            .collect();
        let (mean, cv) = spread(&map);
        assert!((mean / expect - 1.0).abs() < 0.02 && cv < 0.01, "map: {mean} vs {expect}, spread {cv}");
    }

    /// 1 A from a pour on F.Cu through one via (0.3 mm drill) into a pour on B.Cu: the barrel's I²R
    /// (ρ·Δz/A, A = π·(d − t)·t) is the via's loss, the layer maps leave it out, and layers + vias give the
    /// rail's copper loss; `PiResult::loss` (the temperature's heat) keeps it on the contact cells.
    #[test]
    fn via_barrels_out_of_the_layer_maps() {
        let board = crate::load_pcb(
            r#"(kicad_pcb (version 20241229) (generator "pcbnew")
  (layers (0 "F.Cu" signal) (2 "B.Cu" signal))
  (setup (stackup
    (layer "F.Cu" (type "copper") (thickness 0.035))
    (layer "dielectric 1" (type "core") (thickness 1.5) (material "FR4") (epsilon_r 4.5) (loss_tangent 0.02))
    (layer "B.Cu" (type "copper") (thickness 0.035))))
  (net 0 "") (net 1 "+3V3")
  (footprint "R" (layer "F.Cu") (at 0 0) (property "Reference" "J1") (property "PI_SOURCE" "+3V3")
    (pad "1" smd rect (at 0.5 2) (size 1 4) (layers "F.Cu") (net 1 "+3V3")))
  (footprint "R" (layer "B.Cu") (at 0 0) (property "Reference" "U1") (property "PI_LOAD" "+3V3=1A")
    (pad "1" smd rect (at 17.5 2) (size 1 4) (layers "B.Cu") (net 1 "+3V3")))
  (via (at 9 2) (size 0.6) (drill 0.3) (layers "F.Cu" "B.Cu") (net 1))
  (zone (net 1) (net_name "+3V3") (layer "F.Cu") (filled_polygon (layer "F.Cu") (pts (xy 0 0) (xy 10 0) (xy 10 4) (xy 0 4))))
  (zone (net 1) (net_name "+3V3") (layer "B.Cu") (filled_polygon (layer "B.Cu") (pts (xy 8 0) (xy 18 0) (xy 18 4) (xy 8 4)))))"#,
        )
        .unwrap();
        let (setups, _) = crate::pi::rails_from_fields(&board);
        for cell in [0.1, 0.25] {
            let p = crate::pi::PiParams { cell: Some(cell), ..Default::default() };
            let r = crate::pi::solve_rail(&board, &setups[0], &p).unwrap();
            assert_eq!(r.barrels.len(), 1);
            let b = &r.barrels[0];
            let (d, t) = (0.3, 0.025);
            let area = std::f64::consts::PI * (d - t) * t;
            let r_barrel = 1.72e-8 * 1.535e-3 / (area * 1e-6);
            assert!((b.area_mm2 / area - 1.0).abs() < 1e-9 && (b.current - 1.0).abs() < 1e-6, "{b:?}");
            assert!((b.loss_w / r_barrel - 1.0).abs() < 1e-6, "{} vs {r_barrel}", b.loss_w);
            assert!(b.from == 0 && b.to == 1 && !b.hole);
            // the temperature's heat per cell still holds the barrel
            let cells: f64 = r.loss.iter().map(|&x| x as f64).sum();
            assert!((cells / r.power_loss_w - 1.0).abs() < 1e-6);
            // the layer planes and the map without it, the vias with it
            let vl = loss_vias(&[&r]);
            assert_eq!((vl.count, vl.list.len()), (1, 1));
            let v = &vl.list[0];
            assert!((vl.total_w - b.loss_w).abs() < 1e-12 && (v.j_a_mm2 - 1.0 / area).abs() < 1e-6);
            assert!(v.capacity_a > 0.5 && v.capacity_a < 3.0 && (v.load - 1.0 / v.capacity_a).abs() < 1e-6, "{v:?}");
            let layers: f64 =
                (0..2).map(|l| loss_layer(&board, &[&r], l).planes.iter().map(|p| p.w).sum::<f64>()).sum();
            assert!(
                ((layers + vl.total_w) / r.power_loss_w - 1.0).abs() < 1e-5,
                "{layers} + {} vs {}",
                vl.total_w,
                r.power_loss_w
            );
            assert!(vl.total_w > 0.05 * r.power_loss_w, "the barrel is a visible part of this rail");
            let m = loss_map(&board, &[&r], &[], Some(0.2)).unwrap();
            let mw: f64 = m.copper.iter().map(|&x| x as f64 * 0.04).sum();
            assert!(
                ((mw + vl.total_w) / r.power_loss_w - 1.0).abs() < 1e-5,
                "{mw} + {} vs {}",
                vl.total_w,
                r.power_loss_w
            );
            // the contact cells keep only the plane's spreading loss (about 10× the strip's density next to a
            // 0.15 mm barrel); with the barrel's half on them they would show 330× (0.25 mm cells) to 2000×
            // (0.1 mm), growing with 1/cell²
            let ll = loss_layer(&board, &[&r], 0);
            let far = 1.72e-8 / 35e-6 / 16.0; // W/mm² of 1 A in the 4 mm strip
            let raw = r.loss.iter().fold(0f32, |a, &x| a.max(x)) as f64 / (cell * cell);
            assert!(ll.max < 20.0 * far && raw > 200.0 * far, "cell {cell}: {} / {raw} vs {far}", ll.max);
        }
        // IPC-2152 fit at 10 K: about 1.5 A for 0.3 mm / 25 µm (0.0216 mm²), within the chart's 3 %
        let c = via_capacity(std::f64::consts::PI * 0.275 * 0.025);
        assert!((c - 1.55).abs() < 0.05, "{c}");
    }
}

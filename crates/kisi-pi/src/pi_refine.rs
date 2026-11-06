//! Experimental refinement of the DC IR-drop raster (research, docs/research/pi-quadtree.md).
//!
//! `pi::solve_rail` rasterises every copper layer of a rail into square cells of one size. This module
//! solves the same rail on a refined raster to find out whether local refinement pays:
//!
//! * **Quadtree per base cell.** The base grid of `solve_rail` (same cell, same origin) is kept; a base
//!   cell of level k is split into 2^k × 2^k square leaves. Levels are chosen per base cell (around vias
//!   and drilled pads, on load and source pads, at necks, at copper edges, or everywhere for a uniform
//!   finer grid aligned with the base grid) and balanced 2:1 between neighbouring base cells. Leaves are
//!   rasterised like the base grid at their own size (`fill_polygon`, `fill_track`, pads). Neighbouring
//!   leaves are joined by the sheet conductance × face length / centre distance (two-point flux: a leaf
//!   and its half-size neighbour get 2/3 of a square's conductance; the tangential offset of the centres
//!   is ignored, a known first-order inconsistency of this scheme).
//! * **Via contacts.** `Contact::Cell` as `solve_rail` (the barrel at the leaf that contains its centre:
//!   a contact of equivalent radius ≈ 0.2 leaf, Peaceman 1983); `Contact::Disc` the barrel as an
//!   equipotential circle of radius drill/2 (all copper leaves whose centres lie inside become one node,
//!   the contact leaf if none does).
//! * **Peaceman-type correction.** A barrel of radius r_w smaller than about 0.7 leaf contacts its leaf
//!   alone; the leaf's links to full-copper neighbours are scaled by
//!   f = 1 / (1 − (2/π)·ln(r_w / (0.198·h))), so that the leaf's potential is the one of the barrel's rim
//!   instead of the one at 0.198·h from a point source (derivation in the write-up; f > 1 when the barrel
//!   is larger than 0.198·h, all conductances stay positive). Larger barrels use the disc contact.
//!
//! With no refinement, `Contact::Cell` and no correction the solution is the one of `solve_rail` (test).

use crate::board::{pad_polygon, Board, Pad};
use crate::geom::{self, BBox, P};
use crate::pi::{
    fill_polygon, fill_track, pad_cells, rail_frame, Frame, Grid, LayerGrid, LoadResult, PiParams, PiResult, RailSetup,
    SourceResult, ViaResult,
};
use serde::Deserialize;
use std::collections::{HashMap, VecDeque};

/// The refinement `PiParams::refine` asks for (experimental; default none).
#[derive(Debug, Clone, Copy, PartialEq, Default, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Refine {
    #[default]
    None,
    /// uniform raster, via contacts with the Peaceman-type correction (disc contact for large barrels)
    Peaceman,
    /// `levels` halvings within `radius` × drill diameter of vias and drilled pads and on load/source pads,
    /// barrels as equipotential discs
    Quadtree { levels: u8, radius: f64 },
    /// quadtree and the Peaceman-type correction at the leaves
    Both { levels: u8, radius: f64 },
    /// tracks narrower than 3 cells as line conductors
    Lines,
    /// line conductors and the Peaceman-type correction (the cheap combination)
    LinesPeaceman,
    /// quadtree, Peaceman-type correction and line conductors
    All { levels: u8, radius: f64 },
}

/// Tracks narrower than this many cells are line conductors in `Refine::Lines…`/`All`.
pub const LINE_CELLS: f64 = 3.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Contact {
    /// the barrel at the leaf containing its centre (`solve_rail`)
    Cell,
    /// the barrel as an equipotential circle of radius drill/2
    Disc,
}

/// Mesh and contact options of the experimental solver.
#[derive(Debug, Clone)]
pub struct Mesh {
    /// level of every base cell with copper (a uniform grid of cell / 2^uniform, aligned with the base grid)
    pub uniform: u8,
    /// level within `via_radius` × drill diameter of vias and drilled pads
    pub via_levels: u8,
    pub via_radius: f64,
    /// level on the pads of loads and sources
    pub pad_levels: u8,
    /// level at necks: full copper with at most 3 base cells across in x or y
    pub neck_levels: u8,
    /// level at copper edges (a copper base cell next to one without copper)
    pub edge_levels: u8,
    pub contact: Contact,
    pub peaceman: bool,
    /// disc contact: the leaves whose centres lie within r_w + `disc_grow` × leaf size
    pub disc_grow: f64,
    /// cut-cell weights: a leaf's weight is the copper fraction of its area, sampled on `cut_cell` ×
    /// `cut_cell` points (0 or 1: copper at the centre, as `solve_rail`)
    pub cut_cell: u8,
    /// tracks narrower than `track_lines` base cells as line conductors (0: rasterised)
    pub track_lines: f64,
}

impl Default for Mesh {
    fn default() -> Self {
        Mesh {
            uniform: 0,
            via_levels: 0,
            via_radius: 3.0,
            pad_levels: 0,
            neck_levels: 0,
            edge_levels: 0,
            contact: Contact::Cell,
            peaceman: false,
            disc_grow: 0.0,
            cut_cell: 0,
            track_lines: 0.0,
        }
    }
}

impl From<Refine> for Mesh {
    fn from(r: Refine) -> Self {
        let d = Mesh::default();
        match r {
            Refine::None => d,
            Refine::Peaceman => Mesh { contact: Contact::Disc, peaceman: true, disc_grow: DISC_GROW, ..d },
            Refine::Quadtree { levels, radius } => Mesh {
                via_levels: levels,
                via_radius: radius,
                pad_levels: levels,
                contact: Contact::Disc,
                disc_grow: DISC_GROW,
                ..d
            },
            Refine::Both { levels, radius } => Mesh {
                via_levels: levels,
                via_radius: radius,
                pad_levels: levels,
                contact: Contact::Disc,
                peaceman: true,
                disc_grow: DISC_GROW,
                ..d
            },
            Refine::Lines => Mesh { track_lines: LINE_CELLS, ..d },
            Refine::LinesPeaceman => Mesh { track_lines: LINE_CELLS, ..Mesh::from(Refine::Peaceman) },
            Refine::All { levels, radius } => {
                Mesh { track_lines: LINE_CELLS, ..Mesh::from(Refine::Both { levels, radius }) }
            }
        }
    }
}

/// Disc contact: leaves within r_w + 0.35 leaf (the equivalent radius of a block of tied leaves extends about a
/// third of a leaf beyond their centres; two-vias case within ±1.5 % for r_w/h = 1 … 5).
pub const DISC_GROW: f64 = 0.35;

/// Peaceman's equivalent radius of a single cell node of the 5-point stencil, in cells.
pub const PEACEMAN_REQ: f64 = 0.198;

/// Link factor of a contact leaf of size `h` for a barrel of radius `rw` (see the module notes).
pub fn peaceman_factor(rw: f64, h: f64) -> f64 {
    let d = 1.0 - 2.0 / std::f64::consts::PI * (rw / (PEACEMAN_REQ * h)).ln();
    (1.0 / d.max(0.05)).clamp(0.05, 20.0)
}

/// The solution on the leaves.
pub struct Field {
    g: Grid,
    /// level of every base cell
    pub lvl: Vec<u8>,
    /// copper layer of every slot
    pub layers: Vec<usize>,
    /// first leaf of every base cell (the same for every layer), leaves per layer
    first: Vec<u32>,
    nl: usize,
    /// node of every leaf (slot × nl + leaf), u32::MAX: no copper
    node: Vec<u32>,
    /// voltage of every node (NaN: not connected), sign of the rail applied
    pub v: Vec<f64>,
}

impl Field {
    /// The base grid: origin, cell (mm), cells in x and y.
    pub fn base(&self) -> (P, f64, usize, usize) {
        ([self.g.x0, self.g.y0], self.g.cell, self.g.nx, self.g.ny)
    }
    /// Voltage of a node (sign of the rail applied).
    pub fn node_v(&self, n: u32) -> Option<f64> {
        (n != u32::MAX).then(|| self.v[n as usize]).filter(|x| x.is_finite())
    }
    fn leaf_of(&self, p: P) -> Option<(usize, f64)> {
        let (ci, cj) = self.g.cell_of(p)?;
        let c = cj * self.g.nx + ci;
        let k = self.lvl[c];
        let n = 1usize << k;
        let h = self.g.cell / n as f64;
        let cx = self.g.x0 + ci as f64 * self.g.cell;
        let cy = self.g.y0 + cj as f64 * self.g.cell;
        let a = (((p[0] - cx) / h).floor().max(0.0) as usize).min(n - 1);
        let b = (((p[1] - cy) / h).floor().max(0.0) as usize).min(n - 1);
        Some((self.first[c] as usize + b * n + a, h))
    }
    /// Voltage at a point of a layer slot (None: no connected copper there).
    pub fn at(&self, slot: usize, p: P) -> Option<f64> {
        let (leaf, _) = self.leaf_of(p)?;
        let n = self.node[slot * self.nl + leaf];
        (n != u32::MAX).then(|| self.v[n as usize]).filter(|x| x.is_finite())
    }
    /// Every leaf: (slot, centre, size, voltage or None).
    pub fn for_each_leaf(&self, mut f: impl FnMut(usize, P, f64, Option<f64>)) {
        let g = &self.g;
        for slot in 0..self.layers.len() {
            for c in 0..g.nx * g.ny {
                let k = self.lvl[c];
                let n = 1usize << k;
                let h = g.cell / n as f64;
                let (ci, cj) = (c % g.nx, c / g.nx);
                for s in 0..n * n {
                    let (a, b) = (s % n, s / n);
                    let p = [
                        g.x0 + ci as f64 * g.cell + (a as f64 + 0.5) * h,
                        g.y0 + cj as f64 * g.cell + (b as f64 + 0.5) * h,
                    ];
                    let nd = self.node[slot * self.nl + self.first[c] as usize + s];
                    let v = (nd != u32::MAX).then(|| self.v[nd as usize]).filter(|x| x.is_finite());
                    f(slot, p, h, v);
                }
            }
        }
    }
}

/// A barrel's contact on one layer: (vertical index, layer, node).
pub type ContactNode = (usize, usize, u32);

/// The experimental solution of a rail.
pub struct Detail {
    pub result: PiResult,
    pub field: Field,
    /// unknowns of the linear system
    pub unknowns: usize,
    /// nonzeros of the system matrix (upper and lower)
    pub nnz: usize,
    /// leaves with copper (all layers)
    pub copper_leaves: usize,
    /// barrel contacts and the verticals (position, drill) in `solve_rail`'s order
    pub contacts: Vec<ContactNode>,
    pub verticals: Vec<(P, f64)>,
}

/// Which tracks of a layer become line conductors: narrower than `line_w` and not overlapping another track
/// of the net along their length (parallel or stacked tracks are copper the raster joins; as lines their
/// conductances would add up). Overlap near a track's ends – junctions – does not count.
fn line_tracks(ts: &[&crate::board::Track], line_w: f64) -> Vec<bool> {
    if line_w <= 0.0 {
        return vec![false; ts.len()];
    }
    const B: f64 = 1.0; // hash cell, mm
    let key = |p: P| ((p[0] / B).floor() as i64, (p[1] / B).floor() as i64);
    let mut hash: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
    for (i, t) in ts.iter().enumerate() {
        let r = t.width / 2.0;
        let (a, b) = (
            key([t.a[0].min(t.b[0]) - r, t.a[1].min(t.b[1]) - r]),
            key([t.a[0].max(t.b[0]) + r, t.a[1].max(t.b[1]) + r]),
        );
        for x in a.0..=b.0 {
            for y in a.1..=b.1 {
                hash.entry((x, y)).or_default().push(i);
            }
        }
    }
    let dist = |p: P, t: &crate::board::Track| {
        let d = geom::sub(t.b, t.a);
        let u = (geom::dot(geom::sub(p, t.a), d) / geom::dot(d, d).max(1e-18)).clamp(0.0, 1.0);
        geom::len(geom::sub(p, geom::add(t.a, geom::scale(d, u))))
    };
    ts.iter()
        .enumerate()
        .map(|(i, t)| {
            if t.width >= line_w {
                return false;
            }
            let len = geom::len(geom::sub(t.b, t.a));
            let step = (t.width / 2.0).clamp(0.01, 0.1);
            let n = (len / step).ceil() as usize;
            let mut cand: Vec<usize> = Vec::new();
            for k in 0..=n {
                let p = geom::add(t.a, geom::scale(geom::sub(t.b, t.a), k as f64 / n.max(1) as f64));
                if let Some(v) = hash.get(&key(p)) {
                    cand.extend(v.iter().filter(|&&j| j != i));
                }
            }
            cand.sort_unstable();
            cand.dedup();
            for j in cand {
                let t2 = ts[j];
                let reach = 0.75 * (t.width + t2.width) / 2.0;
                let end = t.width + t2.width;
                let mut over = 0.0;
                for k in 0..=n {
                    let s = len * k as f64 / n.max(1) as f64;
                    if s < end || s > len - end {
                        continue;
                    }
                    let p = geom::add(t.a, geom::scale(geom::sub(t.b, t.a), s / len.max(1e-12)));
                    if dist(p, t2) < reach {
                        over += len / n.max(1) as f64;
                    }
                }
                if over > end {
                    return false;
                }
            }
            true
        })
        .collect()
}

/// Weights of a grid (or a tile of leaves): with `ss` = 1 copper at the centre (`fill_polygon`, `fill_track`,
/// a pad's leaves or the one at its centre); with `ss` > 1 the copper fraction of every cell from `ss` × `ss`
/// samples, tracks narrower than a cell with `fill_track`'s weight (the larger of the two).
fn raster(
    g: &Grid,
    ss: usize,
    zones: &[&crate::board::ZoneFill],
    tracks: &[&crate::board::Track],
    pads: &[&Pad],
) -> Vec<f32> {
    let mut w = vec![0f32; g.nx * g.ny];
    let pad_fill = |gg: &Grid, p: &Pad, out: &mut [f32], centre: &mut [f32]| {
        let mut tmp = vec![0f32; gg.nx * gg.ny];
        if fill_polygon(gg, &pad_polygon(p), &mut tmp) == 0 {
            // a pad smaller than a cell: the cell at its centre
            if let Some((i, j)) = g.cell_of(p.at) {
                centre[j * g.nx + i] = 1.0;
            }
        }
        for (o, t) in out.iter_mut().zip(&tmp) {
            if *t > 0.0 {
                *o = 1.0;
            }
        }
    };
    if ss <= 1 {
        for z in zones {
            fill_polygon(g, &z.pts, &mut w);
        }
        for t in tracks {
            fill_track(g, t.a, t.b, t.width, &mut w);
        }
        let mut c = vec![0f32; g.nx * g.ny];
        for p in pads {
            pad_fill(g, p, &mut w, &mut c);
        }
        for (x, y) in w.iter_mut().zip(&c) {
            *x = x.max(*y);
        }
        return w;
    }
    let f = Grid { x0: g.x0, y0: g.y0, cell: g.cell / ss as f64, nx: g.nx * ss, ny: g.ny * ss };
    let mut wf = vec![0f32; f.nx * f.ny];
    for z in zones {
        fill_polygon(&f, &z.pts, &mut wf);
    }
    for t in tracks {
        if t.width < g.cell * 1.05 {
            fill_track(g, t.a, t.b, t.width, &mut w);
        } else {
            fill_track(&f, t.a, t.b, t.width, &mut wf);
        }
    }
    let mut c = vec![0f32; g.nx * g.ny];
    for p in pads {
        pad_fill(&f, p, &mut wf, &mut c);
    }
    let inv = 1.0 / (ss * ss) as f32;
    for j in 0..g.ny {
        for i in 0..g.nx {
            let mut a = 0f32;
            for b in 0..ss {
                let row = (j * ss + b) * f.nx + i * ss;
                a += wf[row..row + ss].iter().sum::<f32>();
            }
            let k = j * g.nx + i;
            w[k] = w[k].max(a * inv).max(c[k]);
        }
    }
    w
}

/// Two link factors of one leaf or of a link's two ends: 1 is "none", otherwise the larger.
fn combine(a: f32, b: f32) -> f32 {
    if a == 1.0 {
        b
    } else if b == 1.0 {
        a
    } else {
        a.max(b)
    }
}

struct UnionFind(Vec<u32>);
impl UnionFind {
    fn find(&mut self, mut x: u32) -> u32 {
        while self.0[x as usize] != x {
            let p = self.0[x as usize];
            self.0[x as usize] = self.0[p as usize];
            x = p;
        }
        x
    }
    fn union(&mut self, a: u32, b: u32) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra != rb {
            let (lo, hi) = if ra < rb { (ra, rb) } else { (rb, ra) };
            self.0[hi as usize] = lo;
        }
    }
}

pub fn solve(
    board: &Board,
    setup: &RailSetup,
    params: &PiParams,
    mesh: &Mesh,
    scale: Option<&dyn Fn(usize, P) -> f64>,
) -> Result<Detail, String> {
    let Frame { net, sign, used, layers, cell, g } = rail_frame(board, setup, params)?;
    let su = board.stackup();
    let ncu = su.n();
    let mut warnings = Vec::new();
    let per = g.nx * g.ny;
    let nsl = layers.len();
    let mut slot_of = vec![usize::MAX; ncu];
    for (s, &l) in layers.iter().enumerate() {
        slot_of[l] = s;
    }

    // base weights per layer, as `solve_rail`
    let zones: Vec<Vec<&crate::board::ZoneFill>> = layers
        .iter()
        .map(|&l| board.zones.iter().filter(|z| z.net == net && z.layer as usize == l).collect())
        .collect();
    let tracks: Vec<Vec<&crate::board::Track>> = layers
        .iter()
        .map(|&l| board.tracks.iter().filter(|t| t.net == net && !t.pad && t.layer as usize == l).collect())
        .collect();
    let pads_l: Vec<Vec<&Pad>> = layers
        .iter()
        .map(|&l| board.pads.iter().filter(|p| p.net == net && p.layers.contains(&(l as u16))).collect())
        .collect();
    // tracks as line conductors (narrower than `track_lines` base cells) and rasterised ones
    let line_w = mesh.track_lines * g.cell;
    let is_line: Vec<Vec<bool>> = tracks.iter().map(|ts| line_tracks(ts, line_w)).collect();
    let rtracks: Vec<Vec<&crate::board::Track>> = tracks
        .iter()
        .zip(&is_line)
        .map(|(ts, il)| ts.iter().zip(il).filter(|x| !*x.1).map(|x| *x.0).collect())
        .collect();
    let ss = mesh.cut_cell.max(1) as usize;
    let plain = ss == 1 && line_w <= 0.0;
    let mut w0: Vec<Vec<f32>> = Vec::with_capacity(nsl);
    for s in 0..nsl {
        if plain {
            let mut wl = vec![0f32; per];
            for z in &zones[s] {
                fill_polygon(&g, &z.pts, &mut wl);
            }
            for t in &tracks[s] {
                fill_track(&g, t.a, t.b, t.width, &mut wl);
            }
            for p in &pads_l[s] {
                for k in pad_cells(&g, p) {
                    wl[k] = 1.0;
                }
            }
            w0.push(wl);
        } else {
            w0.push(raster(&g, ss, &zones[s], &rtracks[s], &pads_l[s]));
        }
    }

    // verticals: vias and drilled pads of the net, in `solve_rail`'s order
    let mut verticals: Vec<(P, f64, Vec<usize>)> = Vec::new();
    for v in board.vias.iter().filter(|v| v.net == net) {
        verticals.push((v.at, v.drill, (v.from as usize..=v.to as usize).collect()));
    }
    for p in board.pads.iter().filter(|p| p.net == net && p.drill > 0.0) {
        verticals.push((p.at, p.drill, p.layers.iter().map(|&l| l as usize).collect()));
    }

    // --- levels per base cell ---
    let copper0: Vec<bool> = (0..per).map(|k| w0.iter().any(|w| w[k] > 0.0)).collect();
    let mut active = copper0.clone();
    for j in 0..g.ny {
        for i in 0..g.nx {
            if !copper0[j * g.nx + i] {
                continue;
            }
            for dj in -1isize..=1 {
                for di in -1isize..=1 {
                    let (ii, jj) = (i as isize + di, j as isize + dj);
                    if ii >= 0 && jj >= 0 && (ii as usize) < g.nx && (jj as usize) < g.ny {
                        active[jj as usize * g.nx + ii as usize] = true;
                    }
                }
            }
        }
    }
    let mut lvl: Vec<u8> = (0..per).map(|k| if active[k] { mesh.uniform } else { 0 }).collect();
    let raise = |lvl: &mut Vec<u8>, c: usize, k: u8| {
        if active[c] && lvl[c] < k {
            lvl[c] = k;
        }
    };
    let cells_near = |at: P, r: f64| -> Vec<usize> {
        // base cells whose square comes within r of `at`
        let mut out = Vec::new();
        let i0 = ((at[0] - r - g.x0) / g.cell).floor().max(0.0) as usize;
        let j0 = ((at[1] - r - g.y0) / g.cell).floor().max(0.0) as usize;
        let i1 = (((at[0] + r - g.x0) / g.cell).floor().max(0.0) as usize).min(g.nx - 1);
        let j1 = (((at[1] + r - g.y0) / g.cell).floor().max(0.0) as usize).min(g.ny - 1);
        for j in j0..=j1 {
            for i in i0..=i1 {
                let (cx0, cy0) = (g.x0 + i as f64 * g.cell, g.y0 + j as f64 * g.cell);
                let dx = (cx0 - at[0]).max(0.0).max(at[0] - cx0 - g.cell);
                let dy = (cy0 - at[1]).max(0.0).max(at[1] - cy0 - g.cell);
                if dx * dx + dy * dy <= r * r {
                    out.push(j * g.nx + i);
                }
            }
        }
        out
    };
    if mesh.via_levels > 0 {
        for (at, drill, _) in &verticals {
            for c in cells_near(*at, mesh.via_radius * drill) {
                raise(&mut lvl, c, mesh.via_levels);
            }
        }
    }
    if mesh.pad_levels > 0 {
        let parts: Vec<&str> = setup
            .sources
            .iter()
            .map(|s| s.refdes.as_str())
            .chain(setup.loads.iter().map(|l| l.refdes.as_str()))
            .collect();
        for p in board
            .pads
            .iter()
            .filter(|p| p.net == net && parts.contains(&board.footprints[p.fp as usize].reference.as_str()))
        {
            let bb = BBox::of(&pad_polygon(p));
            let r = 0.5 * (bb.max[0] - bb.min[0]).max(bb.max[1] - bb.min[1]);
            for c in cells_near(p.at, r) {
                raise(&mut lvl, c, mesh.pad_levels);
            }
        }
    }
    if mesh.neck_levels > 0 || mesh.edge_levels > 0 {
        for s in 0..nsl {
            let full = |i: usize, j: usize| w0[s][j * g.nx + i] >= 1.0;
            let any = |i: isize, j: isize| {
                i >= 0
                    && j >= 0
                    && (i as usize) < g.nx
                    && (j as usize) < g.ny
                    && w0[s][j as usize * g.nx + i as usize] > 0.0
            };
            // runs of full copper along x and y through every cell
            let mut rx = vec![0u16; per];
            let mut ry = vec![0u16; per];
            for j in 0..g.ny {
                let mut i = 0;
                while i < g.nx {
                    if !full(i, j) {
                        i += 1;
                        continue;
                    }
                    let st = i;
                    while i < g.nx && full(i, j) {
                        i += 1;
                    }
                    for x in st..i {
                        rx[j * g.nx + x] = (i - st).min(u16::MAX as usize) as u16;
                    }
                }
            }
            for i in 0..g.nx {
                let mut j = 0;
                while j < g.ny {
                    if !full(i, j) {
                        j += 1;
                        continue;
                    }
                    let st = j;
                    while j < g.ny && full(i, j) {
                        j += 1;
                    }
                    for y in st..j {
                        ry[y * g.nx + i] = (j - st).min(u16::MAX as usize) as u16;
                    }
                }
            }
            for j in 0..g.ny {
                for i in 0..g.nx {
                    let c = j * g.nx + i;
                    if w0[s][c] <= 0.0 {
                        continue;
                    }
                    if mesh.neck_levels > 0 && full(i, j) && rx[c].min(ry[c]) <= 3 && rx[c].max(ry[c]) >= 2 {
                        raise(&mut lvl, c, mesh.neck_levels);
                    }
                    let (ii, jj) = (i as isize, j as isize);
                    if mesh.edge_levels > 0
                        && !(any(ii - 1, jj) && any(ii + 1, jj) && any(ii, jj - 1) && any(ii, jj + 1))
                    {
                        raise(&mut lvl, c, mesh.edge_levels);
                    }
                }
            }
        }
    }
    // 2:1 balance between face neighbours (active cells)
    loop {
        let mut changed = false;
        for j in 0..g.ny {
            for i in 0..g.nx {
                let c = j * g.nx + i;
                if !active[c] {
                    continue;
                }
                let mut m = lvl[c];
                for (di, dj) in [(-1isize, 0isize), (1, 0), (0, -1), (0, 1)] {
                    let (ii, jj) = (i as isize + di, j as isize + dj);
                    if ii >= 0 && jj >= 0 && (ii as usize) < g.nx && (jj as usize) < g.ny {
                        let c2 = jj as usize * g.nx + ii as usize;
                        if active[c2] {
                            m = m.max(lvl[c2].saturating_sub(1));
                        }
                    }
                }
                if m > lvl[c] {
                    lvl[c] = m;
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }

    // --- leaves and their weights ---
    let mut first = vec![0u32; per];
    let mut nl = 0usize;
    for c in 0..per {
        first[c] = nl as u32;
        nl += 1usize << (2 * lvl[c]);
    }
    if nl * nsl > 400_000_000 {
        return Err(format!("{}: refined raster too large ({} leaves)", setup.net, nl * nsl));
    }
    let mut w = vec![0f32; nl * nsl];
    for s in 0..nsl {
        for c in 0..per {
            if lvl[c] == 0 {
                w[s * nl + first[c] as usize] = w0[s][c];
            }
        }
    }
    let kmax = lvl.iter().copied().max().unwrap_or(0);
    const T: usize = 16;
    for k in 1..=kmax {
        let n = 1usize << k;
        let hk = g.cell / n as f64;
        let mut tiles: Vec<(usize, usize)> = Vec::new();
        for c in 0..per {
            if lvl[c] == k {
                tiles.push(((c % g.nx) / T, (c / g.nx) / T));
            }
        }
        tiles.sort_unstable();
        tiles.dedup();
        for (ti, tj) in tiles {
            let (ci0, cj0) = (ti * T, tj * T);
            let (cw, ch) = (T.min(g.nx - ci0), T.min(g.ny - cj0));
            let sub = Grid {
                x0: g.x0 + ci0 as f64 * g.cell,
                y0: g.y0 + cj0 as f64 * g.cell,
                cell: hk,
                nx: cw * n,
                ny: ch * n,
            };
            let tb = BBox { min: [sub.x0, sub.y0], max: [sub.x0 + cw as f64 * g.cell, sub.y0 + ch as f64 * g.cell] };
            let hit = |a: &BBox| {
                a.min[0] <= tb.max[0] && a.max[0] >= tb.min[0] && a.min[1] <= tb.max[1] && a.max[1] >= tb.min[1]
            };
            for s in 0..nsl {
                let zs: Vec<&crate::board::ZoneFill> = zones[s].iter().copied().filter(|z| hit(&z.bbox)).collect();
                let ts: Vec<&crate::board::Track> = rtracks[s]
                    .iter()
                    .copied()
                    .filter(|t| {
                        let r = t.width.max(g.cell);
                        hit(&BBox {
                            min: [t.a[0].min(t.b[0]) - r, t.a[1].min(t.b[1]) - r],
                            max: [t.a[0].max(t.b[0]) + r, t.a[1].max(t.b[1]) + r],
                        })
                    })
                    .collect();
                let ps: Vec<&Pad> = pads_l[s].iter().copied().filter(|p| hit(&BBox::of(&pad_polygon(p)))).collect();
                let buf = raster(&sub, ss, &zs, &ts, &ps);
                for cj in cj0..cj0 + ch {
                    for ci in ci0..ci0 + cw {
                        let c = cj * g.nx + ci;
                        if lvl[c] != k {
                            continue;
                        }
                        let base = s * nl + first[c] as usize;
                        for b in 0..n {
                            let row = ((cj - cj0) * n + b) * sub.nx + (ci - ci0) * n;
                            w[base + b * n..base + b * n + n].copy_from_slice(&buf[row..row + n]);
                        }
                    }
                }
            }
        }
    }
    drop(w0);

    // leaf geometry
    let leaf_h = |c: usize| g.cell / (1usize << lvl[c]) as f64;
    let leaf_center = |c: usize, s: usize| -> P {
        let n = 1usize << lvl[c];
        let h = g.cell / n as f64;
        [
            g.x0 + (c % g.nx) as f64 * g.cell + ((s % n) as f64 + 0.5) * h,
            g.y0 + (c / g.nx) as f64 * g.cell + ((s / n) as f64 + 0.5) * h,
        ]
    };
    // leaf (layer-local index) → base cell, built once
    let mut base_of = vec![0u32; nl];
    for c in 0..per {
        let n = 1usize << (2 * lvl[c]);
        for s in 0..n {
            base_of[first[c] as usize + s] = c as u32;
        }
    }

    let leaf_at = |p: P| -> Option<usize> {
        let (ci, cj) = g.cell_of(p)?;
        let c = cj * g.nx + ci;
        let n = 1usize << lvl[c];
        let h = g.cell / n as f64;
        let a = (((p[0] - (g.x0 + ci as f64 * g.cell)) / h).floor().max(0.0) as usize).min(n - 1);
        let b = (((p[1] - (g.y0 + cj as f64 * g.cell)) / h).floor().max(0.0) as usize).min(n - 1);
        Some(first[c] as usize + b * n + a)
    };
    // --- tracks as line conductors: the leaves along the centre line in order, with the mean position of
    // the line's samples in each ---
    let h_min = g.cell / (1usize << lvl.iter().copied().max().unwrap_or(0)) as f64;
    let mut chains: Vec<(usize, f64, Vec<(usize, P)>, [P; 2])> = Vec::new();
    if line_w > 0.0 {
        for s in 0..nsl {
            for t in tracks[s].iter().zip(&is_line[s]).filter(|x| *x.1).map(|x| *x.0) {
                let len = geom::len(geom::sub(t.b, t.a));
                let n = ((len / (h_min / 4.0)).ceil() as usize).max(1);
                let mut ch: Vec<(usize, P, usize)> = Vec::new();
                for k in 0..=n {
                    let p = geom::add(t.a, geom::scale(geom::sub(t.b, t.a), k as f64 / n as f64));
                    let Some(lf) = leaf_at(p) else { continue };
                    match ch.last_mut() {
                        Some(last) if last.0 == lf => {
                            last.1 = geom::add(last.1, p);
                            last.2 += 1;
                        }
                        _ => ch.push((lf, p, 1)),
                    }
                }
                let ch = ch.into_iter().map(|(lf, sum, c)| (lf, geom::scale(sum, 1.0 / c as f64))).collect();
                chains.push((s, t.width, ch, [t.a, t.b]));
            }
        }
    }

    // --- nodes: leaves with copper or on a line track ---
    let mut node = vec![u32::MAX; nl * nsl];
    for (s, _, ch, _) in &chains {
        for &(lf, _) in ch {
            node[s * nl + lf] = 0;
        }
    }
    let mut nn = 0usize;
    for (i, x) in w.iter().enumerate() {
        if *x > 0.0 || node[i] == 0 {
            node[i] = nn as u32;
            nn += 1;
        }
    }
    let copper_leaves = nn;

    // --- barrel contacts ---
    // the contact leaf as `solve_rail::find_node`: the leaf at the centre, else the nearest copper leaf
    // within r
    let find_leaf = |s: usize, at: P, r: f64| -> Option<usize> {
        let lf = leaf_at(at)?;
        if node[s * nl + lf] != u32::MAX {
            return Some(lf);
        }
        let (ci, cj) = g.cell_of(at)?;
        let rc = (r / g.cell).ceil() as isize;
        let mut best: Option<(f64, usize)> = None;
        for dj in -rc..=rc {
            for di in -rc..=rc {
                let (ii, jj) = (ci as isize + di, cj as isize + dj);
                if ii < 0 || jj < 0 || ii as usize >= g.nx || jj as usize >= g.ny {
                    continue;
                }
                let c = jj as usize * g.nx + ii as usize;
                for sub in 0..1usize << (2 * lvl[c]) {
                    let lf = first[c] as usize + sub;
                    if node[s * nl + lf] != u32::MAX {
                        let d = geom::len(geom::sub(leaf_center(c, sub), at));
                        if d <= r && best.is_none_or(|b| d < b.0) {
                            best = Some((d, lf));
                        }
                    }
                }
            }
        }
        best.map(|b| b.1)
    };
    let disc_leaves = |s: usize, at: P, r: f64| -> Vec<usize> {
        let mut out = Vec::new();
        for c in cells_near(at, r) {
            for sub in 0..1usize << (2 * lvl[c]) {
                let lf = first[c] as usize + sub;
                if node[s * nl + lf] != u32::MAX && geom::len(geom::sub(leaf_center(c, sub), at)) <= r {
                    out.push(lf);
                }
            }
        }
        out
    };
    let mut uf = UnionFind((0..nn as u32).collect());
    let mut pfac = vec![1f32; nn];
    // per vertical and layer: a representative leaf (merged later)
    let mut contact_leaf: Vec<Vec<(usize, usize)>> = Vec::with_capacity(verticals.len());
    for (at, drill, ls) in &verticals {
        let mut cl = Vec::new();
        for &l in ls {
            if l >= ncu || !used[l] {
                continue;
            }
            let s = slot_of[l];
            let Some(lf) = find_leaf(s, *at, drill / 2.0 + g.cell) else { continue };
            let rw = drill / 2.0;
            let h = leaf_h(base_of[lf] as usize);
            let use_pm = mesh.peaceman && rw < 0.7 * h;
            let set = if !use_pm && (mesh.contact == Contact::Disc || mesh.peaceman) {
                disc_leaves(s, *at, rw + mesh.disc_grow * h)
            } else {
                Vec::new()
            };
            if set.is_empty() {
                if use_pm {
                    let n = node[s * nl + lf] as usize;
                    pfac[n] = combine(pfac[n], peaceman_factor(rw, h) as f32);
                }
                cl.push((l, lf));
            } else {
                let a = node[s * nl + set[0]];
                for &x in &set[1..] {
                    uf.union(a, node[s * nl + x]);
                }
                cl.push((l, set[0]));
            }
        }
        contact_leaf.push(cl);
    }
    // line track ends on other copper: the end's leaf is one node with the other track's leaf at the end's
    // projection (the end lies on that track) or with the nearest raster leaf within half the track's width
    if !chains.is_empty() {
        let mut by_cell: HashMap<(usize, usize), Vec<usize>> = HashMap::new();
        for (ci, (s, _, ch, _)) in chains.iter().enumerate() {
            for &(lf, _) in ch {
                let v = by_cell.entry((*s, base_of[lf] as usize)).or_default();
                if v.last() != Some(&ci) {
                    v.push(ci);
                }
            }
        }
        for (ci, (s, wd, ch, ends)) in chains.iter().enumerate() {
            for (e, end) in ends.iter().enumerate() {
                let le = if e == 0 { ch[0].0 } else { ch[ch.len() - 1].0 };
                let ne = node[s * nl + le];
                let Some((bi, bj)) = g.cell_of(*end) else { continue };
                let mut others: Vec<usize> = Vec::new();
                for dj in -1isize..=1 {
                    for di in -1isize..=1 {
                        let (ii, jj) = (bi as isize + di, bj as isize + dj);
                        if ii < 0 || jj < 0 || ii as usize >= g.nx || jj as usize >= g.ny {
                            continue;
                        }
                        if let Some(v) = by_cell.get(&(*s, jj as usize * g.nx + ii as usize)) {
                            others.extend(v.iter().filter(|&&o| o != ci));
                        }
                    }
                }
                others.sort_unstable();
                others.dedup();
                for o in others {
                    let (_, w2, ch2, e2) = &chains[o];
                    let d = geom::sub(e2[1], e2[0]);
                    let t = (geom::dot(geom::sub(*end, e2[0]), d) / geom::dot(d, d).max(1e-18)).clamp(0.0, 1.0);
                    let q = geom::add(e2[0], geom::scale(d, t));
                    if geom::len(geom::sub(*end, q)) > w2 / 2.0 + 1e-6 {
                        continue;
                    }
                    if let Some(&(lf2, _)) = ch2
                        .iter()
                        .min_by(|a, b| geom::len(geom::sub(a.1, q)).partial_cmp(&geom::len(geom::sub(b.1, q))).unwrap())
                    {
                        uf.union(ne, node[s * nl + lf2]);
                    }
                }
                if w[s * nl + le] <= 0.0 {
                    let mut best: Option<(f64, usize)> = None;
                    for c in cells_near(*end, wd / 2.0 + h_min) {
                        for sub in 0..1usize << (2 * lvl[c]) {
                            let lf = first[c] as usize + sub;
                            if w[s * nl + lf] > 0.0 {
                                let dd = geom::len(geom::sub(leaf_center(c, sub), *end));
                                if dd <= wd / 2.0 + 0.5 * leaf_h(c) && best.is_none_or(|b| dd < b.0) {
                                    best = Some((dd, lf));
                                }
                            }
                        }
                    }
                    if let Some((_, lf)) = best {
                        uf.union(ne, node[s * nl + lf]);
                    }
                }
            }
        }
    }
    // merge: renumber the nodes by their representatives (identity without discs)
    let merged = (0..nn as u32).any(|i| uf.find(i) != i);
    let mut rep_pos: Vec<(usize, usize)> = Vec::new(); // (slot, leaf) of every node
    if merged {
        let mut newid = vec![u32::MAX; nn];
        let mut m = 0u32;
        for i in 0..nn as u32 {
            let r = uf.find(i);
            if newid[r as usize] == u32::MAX {
                newid[r as usize] = m;
                m += 1;
            }
            newid[i as usize] = newid[r as usize];
        }
        let mut pf2 = vec![1f32; m as usize];
        for i in 0..nn {
            let k = newid[i] as usize;
            pf2[k] = combine(pf2[k], pfac[i]);
        }
        pfac = pf2;
        for x in node.iter_mut() {
            if *x != u32::MAX {
                *x = newid[*x as usize];
            }
        }
        nn = m as usize;
    }
    rep_pos.resize(nn, (usize::MAX, 0));
    for (i, &x) in node.iter().enumerate() {
        if x != u32::MAX && rep_pos[x as usize].0 == usize::MAX {
            rep_pos[x as usize] = (i / nl, i % nl);
        }
    }

    let div: Vec<f64> = match scale {
        Some(f) => rep_pos
            .iter()
            .map(|&(s, lf)| {
                let c = base_of[lf] as usize;
                f(layers[s], leaf_center(c, lf - first[c] as usize)).max(1e-3)
            })
            .collect(),
        None => Vec::new(),
    };
    let dv = |a: u32, b: u32| -> f64 {
        if div.is_empty() {
            1.0
        } else {
            0.5 * (div[a as usize] + div[b as usize])
        }
    };

    // --- edges: (a, b, conductance S); meta: (axis, centre distance in mm) ---
    let mut edges: Vec<(u32, u32, f64)> = Vec::with_capacity(nn * 2);
    let mut meta: Vec<(u8, f64)> = Vec::with_capacity(nn * 2);
    for s in 0..nsl {
        let gs = su.copper[layers[s]].thickness * 1e-3 / params.rho;
        let off = s * nl;
        let mut link = |la: usize, lb: usize, fac: f64, axis: u8, dist: f64| {
            let (na, nb) = (node[off + la], node[off + lb]);
            if na == u32::MAX || nb == u32::MAX || na == nb {
                return;
            }
            let (a, b) = (w[off + la] as f64, w[off + lb] as f64);
            if a <= 0.0 || b <= 0.0 {
                return; // a leaf on a line track only
            }
            let mut wf = 2.0 * a * b / (a + b);
            let pf = combine(pfac[na as usize], pfac[nb as usize]);
            if pf != 1.0 && a >= 1.0 && b >= 1.0 {
                wf *= pf as f64;
            }
            edges.push((na, nb, gs * (fac * wf) / dv(na, nb)));
            meta.push((axis, dist));
        };
        for j in 0..g.ny {
            for i in 0..g.nx {
                let c = j * g.nx + i;
                if !active[c] && lvl[c] == 0 && node[off + first[c] as usize] == u32::MAX {
                    continue;
                }
                let k = lvl[c];
                let n = 1usize << k;
                let h = g.cell / n as f64;
                let f0 = first[c] as usize;
                // inside the base cell
                for b in 0..n {
                    for a in 0..n {
                        if a + 1 < n {
                            link(f0 + b * n + a, f0 + b * n + a + 1, 1.0, 0, h);
                        }
                        if b + 1 < n {
                            link(f0 + b * n + a, f0 + (b + 1) * n + a, 1.0, 1, h);
                        }
                    }
                }
                // to the east and north neighbours
                for (axis, ni, nj) in [(0u8, i + 1, j), (1u8, i, j + 1)] {
                    if ni >= g.nx || nj >= g.ny {
                        continue;
                    }
                    let c2 = nj * g.nx + ni;
                    let k2 = lvl[c2];
                    let n2 = 1usize << k2;
                    let h2 = g.cell / n2 as f64;
                    let f2 = first[c2] as usize;
                    let fac = h.min(h2) / (0.5 * (h + h2));
                    let dist = 0.5 * (h + h2);
                    // leaves along the shared face: index t along the face, from the finer side
                    let nf = n.max(n2);
                    for t in 0..nf {
                        let ta = t * n / nf;
                        let tb = t * n2 / nf;
                        let (la, lb) = if axis == 0 {
                            (f0 + ta * n + (n - 1), f2 + tb * n2)
                        } else {
                            (f0 + (n - 1) * n + ta, f2 + tb)
                        };
                        link(la, lb, fac, axis, dist);
                    }
                }
            }
        }
    }
    // line tracks: the track's conductance between the mean positions in consecutive leaves (none where
    // both leaves are full raster copper: that carries it)
    for (s, wd, ch, _) in &chains {
        let gs = su.copper[layers[*s]].thickness * 1e-3 / params.rho;
        for pair in ch.windows(2) {
            let ((la, pa), (lb, pb)) = (pair[0], pair[1]);
            let (na, nb) = (node[s * nl + la], node[s * nl + lb]);
            if na == nb || (w[s * nl + la] >= 1.0 && w[s * nl + lb] >= 1.0) {
                continue;
            }
            let l = geom::len(geom::sub(pb, pa)).max(1e-6);
            edges.push((na, nb, gs * wd / l / dv(na, nb)));
            meta.push((3, wd * su.copper[layers[*s]].thickness));
        }
    }
    // vertical: vias and through-hole pads, as `solve_rail`
    let layer_z: Vec<f64> = {
        let mut z = vec![0.0; ncu];
        let mut acc = 0.0;
        for k in 0..ncu {
            z[k] = acc + su.copper[k].thickness / 2.0;
            acc += su.copper[k].thickness + su.dielectrics.get(k).map(|d| d.thickness).unwrap_or(0.0);
        }
        z
    };
    let barrel = |drill: f64| -> f64 {
        let r = drill / 2.0 * 1e-3;
        let tp = (params.plating_um * 1e-6).min(r);
        std::f64::consts::PI * (r * r - (r - tp) * (r - tp))
    };
    let mut via_edges: Vec<(usize, P, f64)> = Vec::new();
    let mut contacts: Vec<ContactNode> = Vec::new();
    for (vi, (at, drill, _)) in verticals.iter().enumerate() {
        let mut prev: Option<(usize, u32)> = None;
        for &(l, lf) in &contact_leaf[vi] {
            let nid = node[slot_of[l] * nl + lf];
            contacts.push((vi, l, nid));
            if let Some((pl, pn)) = prev {
                if pn != nid {
                    let len = (layer_z[l] - layer_z[pl]).abs() * 1e-3;
                    let gv = barrel(*drill) / (params.rho * len.max(1e-6)) / dv(pn, nid);
                    via_edges.push((edges.len(), *at, *drill));
                    edges.push((pn, nid, gv));
                    meta.push((2, 0.0));
                }
            }
            prev = Some((l, nid));
        }
    }

    // --- pads of sources and loads ---
    let pad_leaves = |p: &Pad| -> Vec<usize> {
        let mut out: Vec<usize> =
            pad_cells(&g, p).into_iter().filter(|&c| lvl[c] == 0).map(|c| first[c] as usize).collect();
        let poly = pad_polygon(p);
        let bb = BBox::of(&poly);
        let i0 = ((bb.min[0] - g.x0) / g.cell).floor().max(0.0) as usize;
        let j0 = ((bb.min[1] - g.y0) / g.cell).floor().max(0.0) as usize;
        let i1 = (((bb.max[0] - g.x0) / g.cell).floor().max(0.0) as usize).min(g.nx - 1);
        let j1 = (((bb.max[1] - g.y0) / g.cell).floor().max(0.0) as usize).min(g.ny - 1);
        let mut ks: Vec<u8> = Vec::new();
        for j in j0..=j1 {
            for i in i0..=i1 {
                let k = lvl[j * g.nx + i];
                if k > 0 && !ks.contains(&k) {
                    ks.push(k);
                }
            }
        }
        for k in ks {
            let n = 1usize << k;
            let sub = Grid {
                x0: g.x0 + i0 as f64 * g.cell,
                y0: g.y0 + j0 as f64 * g.cell,
                cell: g.cell / n as f64,
                nx: (i1 - i0 + 1) * n,
                ny: (j1 - j0 + 1) * n,
            };
            let mut tmp = vec![0f32; sub.nx * sub.ny];
            fill_polygon(&sub, &poly, &mut tmp);
            for fj in 0..sub.ny {
                for fi in 0..sub.nx {
                    if tmp[fj * sub.nx + fi] > 0.0 {
                        let c = (j0 + fj / n) * g.nx + i0 + fi / n;
                        if lvl[c] == k {
                            out.push(first[c] as usize + (fj % n) * n + fi % n);
                        }
                    }
                }
            }
        }
        if out.is_empty() {
            if let Some(lf) = leaf_at(p.at) {
                out.push(lf);
            }
        }
        out
    };
    let pads_of = |refdes: &str, numbers: &[String]| -> Vec<&Pad> {
        board
            .pads
            .iter()
            .filter(|p| {
                p.net == net
                    && board.footprints[p.fp as usize].reference == refdes
                    && (numbers.is_empty() || numbers.contains(&p.number))
            })
            .collect()
    };
    // nodes of a pad with the area of their leaves
    let pad_nodes = |p: &Pad| -> Vec<(u32, f64)> {
        let mut v = Vec::new();
        let lv = pad_leaves(p);
        for &l in &p.layers {
            let l = l as usize;
            if l >= ncu || slot_of[l] == usize::MAX {
                continue;
            }
            for &lf in &lv {
                let n = node[slot_of[l] * nl + lf];
                if n != u32::MAX {
                    let h = leaf_h(base_of[lf] as usize);
                    v.push((n, h * h));
                }
            }
        }
        v
    };
    let mut fixed = vec![false; nn];
    let mut src_nodes: Vec<(String, Vec<u32>, usize)> = Vec::new();
    for s in &setup.sources {
        let pads = pads_of(&s.refdes, &s.pads);
        if pads.is_empty() {
            warnings.push(format!("source {}: no pads on {}", s.refdes, setup.net));
            continue;
        }
        let mut ns = Vec::new();
        for p in &pads {
            ns.extend(pad_nodes(p).into_iter().map(|x| x.0));
        }
        for &n in &ns {
            fixed[n as usize] = true;
        }
        src_nodes.push((s.refdes.clone(), ns, pads.len()));
    }
    if !fixed.iter().any(|&f| f) {
        return Err(format!("{}: no source pads", setup.net));
    }
    let mut inject = vec![0f64; nn];
    let mut load_nodes: Vec<(usize, Vec<u32>, usize)> = Vec::new();
    let mut load_pad_nodes: Vec<Vec<Vec<u32>>> = Vec::new();
    for (li, ld) in setup.loads.iter().enumerate() {
        let pads = pads_of(&ld.refdes, &ld.pads);
        if pads.is_empty() {
            warnings.push(format!("load {}: no pads on {}", ld.refdes, setup.net));
            load_nodes.push((li, vec![], 0));
            load_pad_nodes.push(vec![]);
            continue;
        }
        let mut per_pad_nodes = Vec::new();
        let per_pad = ld.current / pads.len() as f64;
        let mut all = Vec::new();
        for p in &pads {
            let ns = pad_nodes(p);
            if ns.is_empty() {
                continue;
            }
            let same = ns.iter().all(|x| x.1 == ns[0].1);
            let area: f64 = ns.iter().map(|x| x.1).sum();
            for &(n, a) in &ns {
                // equal leaves: as `solve_rail`; mixed sizes: by area
                inject[n as usize] -= if same { per_pad / ns.len() as f64 } else { per_pad * a / area };
            }
            let ids: Vec<u32> = ns.iter().map(|x| x.0).collect();
            all.extend(ids.iter().copied());
            per_pad_nodes.push(ids);
        }
        load_nodes.push((li, all, pads.len()));
        load_pad_nodes.push(per_pad_nodes);
    }

    // --- connectivity, system, solve: as `solve_rail` ---
    let mut adj_start = vec![0usize; nn + 1];
    for &(a, b, _) in &edges {
        adj_start[a as usize + 1] += 1;
        adj_start[b as usize + 1] += 1;
    }
    for i in 0..nn {
        adj_start[i + 1] += adj_start[i];
    }
    let mut adj = vec![0u32; adj_start[nn]];
    let mut fillp = adj_start.clone();
    for &(a, b, _) in &edges {
        adj[fillp[a as usize]] = b;
        fillp[a as usize] += 1;
        adj[fillp[b as usize]] = a;
        fillp[b as usize] += 1;
    }
    let mut reach = vec![false; nn];
    let mut q: VecDeque<u32> = VecDeque::new();
    for i in 0..nn {
        if fixed[i] {
            reach[i] = true;
            q.push_back(i as u32);
        }
    }
    while let Some(n) = q.pop_front() {
        for &m in &adj[adj_start[n as usize]..adj_start[n as usize + 1]] {
            if !reach[m as usize] {
                reach[m as usize] = true;
                q.push_back(m);
            }
        }
    }
    drop(adj);
    let dropped = reach.iter().filter(|r| !**r).count();
    if dropped > 0 {
        warnings.push(format!("{} cells without connection to a source (copper islands) ignored", dropped));
    }
    let mut uidx = vec![u32::MAX; nn];
    let mut nu = 0usize;
    for i in 0..nn {
        if reach[i] && !fixed[i] {
            uidx[i] = nu as u32;
            nu += 1;
        }
    }
    let v_src = setup.voltage.abs();
    let mut trip: Vec<(usize, usize, f64)> = Vec::with_capacity(edges.len() * 4);
    let mut rhs = vec![0f64; nu];
    for i in 0..nn {
        if uidx[i] != u32::MAX {
            rhs[uidx[i] as usize] += inject[i];
        }
    }
    for &(a, b, gv) in &edges {
        let (ua, ub) = (uidx[a as usize], uidx[b as usize]);
        if !reach[a as usize] {
            continue;
        }
        match (ua != u32::MAX, ub != u32::MAX) {
            (true, true) => {
                let (x, y) = (ua as usize, ub as usize);
                trip.push((x, x, gv));
                trip.push((y, y, gv));
                trip.push((x, y, -gv));
                trip.push((y, x, -gv));
            }
            (true, false) => {
                trip.push((ua as usize, ua as usize, gv));
                rhs[ua as usize] += gv * v_src;
            }
            (false, true) => {
                trip.push((ub as usize, ub as usize, gv));
                rhs[ub as usize] += gv * v_src;
            }
            _ => {}
        }
    }
    let nnz = trip.len();
    let x = if nu > 0 {
        kisi_sparse::solve_spd_triplets(nu, &trip, &rhs).map_err(|c| format!("linear solver: error {c}"))?
    } else {
        vec![]
    };
    drop(trip);
    let mut v = vec![f64::NAN; nn];
    for i in 0..nn {
        if fixed[i] {
            v[i] = v_src;
        } else if uidx[i] != u32::MAX {
            v[i] = x[uidx[i] as usize];
        }
    }

    // --- results on the base grid: mean voltage of a base cell's copper leaves, the highest current
    // density of its leaves, the sum of their losses ---
    let mut gsum = vec![[0f64; 4]; nn]; // Σ slope x, n x, Σ slope y, n y
    let mut jline = vec![0f64; nn]; // A/mm² in line tracks
    for (e, &(a, b, gv)) in edges.iter().enumerate() {
        let (axis, dist) = meta[e];
        if axis == 3 {
            let (va, vb) = (v[a as usize], v[b as usize]);
            if va.is_finite() && vb.is_finite() {
                let j = (gv * (va - vb)).abs() / dist;
                jline[a as usize] = jline[a as usize].max(j);
                jline[b as usize] = jline[b as usize].max(j);
            }
            continue;
        }
        if axis > 1 {
            continue;
        }
        let (va, vb) = (v[a as usize], v[b as usize]);
        if !(va.is_finite() && vb.is_finite()) {
            continue;
        }
        let sl = (vb - va) / (dist * 1e-3);
        for n in [a, b] {
            gsum[n as usize][2 * axis as usize] += sl;
            gsum[n as usize][2 * axis as usize + 1] += 1.0;
        }
    }
    let mut vbuf = vec![f32::NAN; per * nsl];
    let mut jbuf = vec![f32::NAN; per * nsl];
    let mut lbuf = vec![0f32; per * nsl];
    let mut max_j = 0.0f64;
    let mut max_j_at = None;
    let mut grids = Vec::new();
    for s in 0..nsl {
        let off = s * nl;
        for c in 0..per {
            let nsub = 1usize << (2 * lvl[c]);
            let (mut sv, mut sa, mut jm) = (0.0, 0.0, f64::NAN);
            for sub in 0..nsub {
                let nd = node[off + first[c] as usize + sub];
                if nd == u32::MAX || !v[nd as usize].is_finite() {
                    continue;
                }
                let h = leaf_h(c);
                sv += v[nd as usize] * h * h;
                sa += h * h;
                let gg = gsum[nd as usize];
                let gx = if gg[1] > 0.0 { gg[0] / gg[1] } else { 0.0 };
                let gy = if gg[3] > 0.0 { gg[2] / gg[3] } else { 0.0 };
                let dn = if div.is_empty() { 1.0 } else { div[nd as usize] };
                let j = ((gx * gx + gy * gy).sqrt() / (params.rho * dn) * 1e-6).max(jline[nd as usize]);
                jm = if jm.is_nan() { j } else { jm.max(j) };
                if j > max_j {
                    max_j = j;
                    max_j_at = Some(leaf_center(c, sub));
                }
            }
            if sa > 0.0 {
                vbuf[s * per + c] = (sv / sa) as f32;
                jbuf[s * per + c] = jm as f32;
            }
        }
        grids.push(LayerGrid { layer: layers[s] as u16, x0: g.x0, y0: g.y0, nx: g.nx, ny: g.ny, offset: s * per });
    }
    let mut loads = Vec::new();
    for (k, (li, ns, npads)) in load_nodes.iter().enumerate() {
        let ld = &setup.loads[*li];
        let open = load_pad_nodes[k].iter().filter(|pn| pn.iter().all(|&n| !reach[n as usize])).count();
        if open > 0 && open < *npads {
            warnings.push(format!(
                "load {}: {} of {} pads not connected, their current is missing from the result",
                ld.refdes, open, npads
            ));
        }
        let vals: Vec<f64> = ns.iter().map(|&n| v[n as usize]).filter(|x| x.is_finite()).collect();
        let connected = !vals.is_empty();
        loads.push(LoadResult {
            refdes: ld.refdes.clone(),
            current: ld.current,
            origin: ld.origin.clone(),
            connected,
            v_min: vals.iter().cloned().reduce(f64::min),
            v_mean: connected.then(|| vals.iter().sum::<f64>() / vals.len() as f64),
            v_max: vals.iter().cloned().reduce(f64::max),
            pads: *npads,
        });
        if !connected && ld.current != 0.0 {
            warnings.push(format!("load {} is not connected to a source", ld.refdes));
        }
    }
    let mut src_cur = vec![0f64; src_nodes.len()];
    let mut owner: HashMap<u32, usize> = HashMap::new();
    for (si, (_, ns, _)) in src_nodes.iter().enumerate() {
        for &n in ns {
            owner.insert(n, si);
        }
    }
    let mut loss = 0.0;
    for &(a, b, gv) in &edges {
        let (va, vb2) = (v[a as usize], v[b as usize]);
        if !(va.is_finite() && vb2.is_finite()) {
            continue;
        }
        let pe = gv * (va - vb2) * (va - vb2);
        loss += pe;
        for n in [a, b] {
            let (s, lf) = rep_pos[n as usize];
            lbuf[s * per + base_of[lf] as usize] += (pe / 2.0) as f32;
        }
        if fixed[a as usize] && !fixed[b as usize] {
            if let Some(&s) = owner.get(&a) {
                src_cur[s] += gv * (va - vb2);
            }
        } else if fixed[b as usize] && !fixed[a as usize] {
            if let Some(&s) = owner.get(&b) {
                src_cur[s] += gv * (vb2 - va);
            }
        }
    }
    let mut vias: Vec<ViaResult> = via_edges
        .iter()
        .filter_map(|&(ei, at, drill)| {
            let (a, b, gv) = edges[ei];
            let d = v[a as usize] - v[b as usize];
            d.is_finite().then(|| ViaResult { at, current: (gv * d).abs(), drill })
        })
        .collect();
    vias.sort_by(|a, b| b.current.partial_cmp(&a.current).unwrap());
    vias.truncate(30);
    let v_min = loads.iter().filter_map(|l| l.v_min).reduce(f64::min);
    let max_drop_mv = if setup.ground {
        loads.iter().flat_map(|l| [l.v_min, l.v_max]).flatten().map(f64::abs).reduce(f64::max).map(|x| x * 1e3)
    } else {
        v_min.map(|m| (v_src - m) * 1e3)
    };
    if sign < 0.0 {
        for x in vbuf.iter_mut() {
            *x = -*x;
        }
        for x in v.iter_mut() {
            *x = -*x;
        }
        for l in loads.iter_mut() {
            (l.v_min, l.v_max) = (l.v_max.map(|v| -v), l.v_min.map(|v| -v));
            l.v_mean = l.v_mean.map(|v| -v);
        }
    }
    let v_min = v_min.map(|v| v * sign);
    let result = PiResult {
        net: setup.net.clone(),
        voltage: setup.voltage,
        ground: setup.ground,
        cell,
        layers: grids,
        loads,
        sources: src_nodes
            .iter()
            .zip(src_cur)
            .map(|((r, _, np), c)| SourceResult { refdes: r.clone(), current: c, pads: *np })
            .collect(),
        vias,
        nodes: nn,
        v_min,
        max_drop_mv,
        max_j,
        max_j_at,
        power_loss_w: loss,
        total_load: setup.loads.iter().map(|l| l.current).sum(),
        warnings,
        v: vbuf,
        j: jbuf,
        loss: lbuf,
    };
    Ok(Detail {
        result,
        field: Field { g, lvl, layers, first, nl, node, v },
        unknowns: nu,
        nnz,
        copper_leaves,
        contacts,
        verticals: verticals.iter().map(|(at, d, _)| (*at, *d)).collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::load_pcb;
    use crate::pi::{rails_from_fields, solve_rail};
    use std::f64::consts::PI;

    const RS: f64 = 1.72e-8 / 35e-6; // Ω per square, 35 µm copper

    /// A two-layer board (35 µm copper, 1.5 mm core) with net 1 "+3V3" and the given items.
    fn board(items: &str) -> crate::board::Board {
        load_pcb(&format!(
            r#"(kicad_pcb (version 20241229) (generator "pcbnew")
  (layers (0 "F.Cu" signal) (2 "B.Cu" signal))
  (setup (stackup
    (layer "F.Cu" (type "copper") (thickness 0.035))
    (layer "dielectric 1" (type "core") (thickness 1.5) (material "FR4") (epsilon_r 4.5) (loss_tangent 0.02))
    (layer "B.Cu" (type "copper") (thickness 0.035))))
  (net 0 "") (net 1 "+3V3")
{items}
)"#
        ))
        .unwrap()
    }
    fn part(r: &str, field: &str, pads: &str) -> String {
        format!(r#"  (footprint "X" (layer "F.Cu") (at 0 0) (property "Reference" "{r}") {field} {pads})"#)
    }
    fn circle_pad(n: usize, x: f64, y: f64, d: f64, layer: &str) -> String {
        format!(r#"(pad "{n}" smd circle (at {x} {y}) (size {d} {d}) (layers "{layer}") (net 1 "+3V3"))"#)
    }
    fn rect_pad(n: usize, x: f64, y: f64, w: f64, h: f64) -> String {
        format!(r#"(pad "{n}" smd rect (at {x} {y}) (size {w} {h}) (layers "F.Cu") (net 1 "+3V3"))"#)
    }
    fn zone(pts: &[(f64, f64)], layer: &str) -> String {
        let p: String = pts.iter().map(|(x, y)| format!("(xy {x} {y}) ")).collect();
        format!(r#"  (zone (net 1) (net_name "+3V3") (layer "{layer}") (filled_polygon (layer "{layer}") (pts {p})))"#)
    }
    fn via(x: f64, y: f64, drill: f64) -> String {
        format!(r#"  (via (at {x} {y}) (size {}) (drill {drill}) (layers "F.Cu" "B.Cu") (net 1))"#, 2.0 * drill)
    }
    fn solve_with(b: &crate::board::Board, cell: f64, mesh: &Mesh) -> Detail {
        let (rails, warn) = rails_from_fields(b);
        assert!(warn.is_empty(), "{warn:?}");
        solve(b, &rails[0], &PiParams { cell: Some(cell), ..Default::default() }, mesh, None).unwrap()
    }
    /// Voltage of a barrel's contact on copper layer `l`.
    fn barrel_v(d: &Detail, vertical: usize, l: usize) -> f64 {
        let c = d.contacts.iter().find(|c| c.0 == vertical && c.1 == l).unwrap();
        d.field.node_v(c.2).unwrap()
    }
    fn qt(levels: u8, peaceman: bool) -> Mesh {
        Mesh { peaceman, ..Mesh::from(Refine::Quadtree { levels, radius: 3.0 }) }
    }
    fn fine(levels: u8) -> Mesh {
        Mesh { uniform: levels, contact: Contact::Disc, disc_grow: DISC_GROW, ..Mesh::default() }
    }

    /// Without refinement and with cell contacts the solver is `solve_rail`: the same numbers.
    #[test]
    fn unrefined_is_solve_rail() {
        let items = [
            zone(&[(0.0, 0.0), (12.0, 0.0), (12.0, 8.0), (0.0, 8.0)], "F.Cu"),
            via(3.0, 3.0, 0.3),
            via(9.1, 5.3, 0.3),
            r#"  (segment (start 3 3) (end 9.1 5.3) (width 0.2) (layer "B.Cu") (net 1))"#.to_string(),
            part("J1", r#"(property "PI_SOURCE" "+3V3")"#, &rect_pad(1, 1.0, 1.0, 1.0, 1.5)),
            part(
                "U1",
                r#"(property "PI_LOAD" "+3V3=2A")"#,
                &(circle_pad(1, 10.0, 6.0, 0.5, "F.Cu") + &circle_pad(2, 9.1, 5.3, 0.6, "B.Cu")),
            ),
        ]
        .join("\n");
        let b = board(&items);
        let (rails, _) = rails_from_fields(&b);
        for cell in [None, Some(0.25), Some(0.15)] {
            let p = PiParams { cell, ..Default::default() };
            let a = solve_rail(&b, &rails[0], &p).unwrap();
            let d = solve(&b, &rails[0], &p, &Mesh::default(), None).unwrap();
            assert_eq!(a.nodes, d.result.nodes);
            assert_eq!(a.max_drop_mv, d.result.max_drop_mv, "cell {cell:?}");
            assert_eq!(a.power_loss_w, d.result.power_loss_w);
            for (x, y) in a.loads.iter().zip(&d.result.loads) {
                assert_eq!((x.v_min, x.v_mean, x.v_max), (y.v_min, y.v_mean, y.v_max));
            }
            for (x, y) in a.v.iter().zip(&d.result.v) {
                assert!(x == y || (x.is_nan() && y.is_nan()));
            }
        }
    }

    /// `PiParams::refine` from JSON (the viewer's and `kisi_pi_solve`'s form) reaches this solver through
    /// `solve_rail`; absent it is the uniform raster.
    #[test]
    fn refine_from_json() {
        let p: PiParams = serde_json::from_str("{}").unwrap();
        assert_eq!(p.refine, Refine::None);
        for (j, r) in [
            (r#"{"refine": {"kind": "peaceman"}}"#, Refine::Peaceman),
            (r#"{"refine": {"kind": "lines_peaceman"}}"#, Refine::LinesPeaceman),
            (
                r#"{"refine": {"kind": "quadtree", "levels": 2, "radius": 3}}"#,
                Refine::Quadtree { levels: 2, radius: 3.0 },
            ),
        ] {
            let p: PiParams = serde_json::from_str(j).unwrap();
            assert_eq!(p.refine, r);
        }
        let items = [
            zone(&[(0.0, 0.0), (20.0, 0.0), (20.0, 20.0), (0.0, 20.0)], "F.Cu"),
            via(7.0, 10.0, 0.3),
            via(13.0, 10.0, 0.3),
            part("J1", r#"(property "PI_SOURCE" "+3V3")"#, &circle_pad(1, 7.0, 10.0, 0.6, "B.Cu")),
            part("U1", r#"(property "PI_LOAD" "+3V3=1A")"#, &circle_pad(1, 13.0, 10.0, 0.6, "B.Cu")),
        ]
        .join("\n");
        let b = board(&items);
        let (rails, _) = rails_from_fields(&b);
        let p: PiParams = serde_json::from_str(r#"{"cell": 0.25, "refine": {"kind": "peaceman"}}"#).unwrap();
        let a = solve_rail(&b, &rails[0], &p).unwrap();
        let d = solve(&b, &rails[0], &p, &Mesh::from(Refine::Peaceman), None).unwrap();
        assert_eq!(a.max_drop_mv, d.result.max_drop_mv);
        let u = solve_rail(&b, &rails[0], &PiParams { cell: Some(0.25), ..Default::default() }).unwrap();
        assert!(a.max_drop_mv.unwrap() < u.max_drop_mv.unwrap());
    }

    /// Potential difference between the rims (radius a) of a source at (x1, y) and a sink at (x2, y) in a
    /// w × h sheet with insulating edges (method of images), per ampere and Ω per square.
    fn sheet_pair(w: f64, h: f64, x1: f64, x2: f64, y: f64, a: f64) -> f64 {
        let pot = |px: f64, py: f64| {
            let mut v = 0.0;
            for i in -30i32..=30 {
                for j in -30i32..=30 {
                    for (sx, sign) in [(x1, -1.0), (x2, 1.0)] {
                        for (mx, my) in [(1.0, 1.0), (-1.0, 1.0), (1.0, -1.0), (-1.0, -1.0)] {
                            let ix = 2.0 * w * i as f64 + mx * sx;
                            let iy = 2.0 * h * j as f64 + my * y;
                            v += sign / (4.0 * PI) * ((px - ix).powi(2) + (py - iy).powi(2)).ln();
                        }
                    }
                }
            }
            v
        };
        let rim = |cx: f64| {
            (0..16).map(|k| pot(cx + a * (k as f64 * PI / 8.0).cos(), y + a * (k as f64 * PI / 8.0).sin())).sum::<f64>()
                / 16.0
        };
        rim(x1) - rim(x2)
    }

    /// Two vias (0.3 mm drill) 6 mm apart in a 20 mm × 20 mm plane: the plane's resistance between the two
    /// barrels is the one between two discs of the barrel radius (images for the edges). Today's cell contact
    /// is a disc of 0.198 cell and overestimates it; the Peaceman-type correction, the quadtree with disc
    /// contacts and the fine reference get it.
    #[test]
    fn two_vias_in_a_sheet() {
        let items = [
            zone(&[(0.0, 0.0), (20.0, 0.0), (20.0, 20.0), (0.0, 20.0)], "F.Cu"),
            via(7.0, 10.0, 0.3),
            via(13.0, 10.0, 0.3),
            part("J1", r#"(property "PI_SOURCE" "+3V3")"#, &circle_pad(1, 7.0, 10.0, 0.6, "B.Cu")),
            part("U1", r#"(property "PI_LOAD" "+3V3=1A")"#, &circle_pad(1, 13.0, 10.0, 0.6, "B.Cu")),
        ]
        .join("\n");
        let b = board(&items);
        let expect = RS * sheet_pair(20.0, 20.0, 7.0, 13.0, 10.0, 0.15);
        let infinite = RS / PI * (6.0f64 / 0.3).acosh();
        let mut errs = HashMap::new();
        for (name, cell, mesh) in [
            ("uniform", 0.25, Mesh::default()),
            ("peaceman", 0.25, Mesh::from(Refine::Peaceman)),
            ("qt2", 0.25, qt(2, false)),
            ("qt2_pm", 0.25, qt(2, true)),
            ("fine_l3", 0.25, fine(3)),
            ("uniform_0.1", 0.1, Mesh::default()),
            ("peaceman_0.1", 0.1, Mesh::from(Refine::Peaceman)),
            ("peaceman_0.5", 0.5, Mesh::from(Refine::Peaceman)),
        ] {
            let d = solve_with(&b, cell, &mesh);
            let r = barrel_v(&d, 0, 0) - barrel_v(&d, 1, 0);
            let e = (r - expect) / expect;
            eprintln!(
                "two vias {name:13} cell {cell}: {:.4} mΩ vs {:.4} mΩ ({:.4} infinite sheet) {:+.2} %",
                r * 1e3,
                expect * 1e3,
                infinite * 1e3,
                e * 100.0
            );
            errs.insert(name, e);
        }
        assert!(errs["uniform"] > 0.15, "the cell contact overestimates the spreading");
        for k in ["peaceman", "qt2", "qt2_pm", "fine_l3", "peaceman_0.1", "peaceman_0.5"] {
            assert!(errs[k].abs() < 0.03, "{k}: {:+.2} %", errs[k] * 100.0);
        }
    }

    /// A via in the middle of a round plane fed from a ring of pads: V = Rs/2π · ln(R/r_w) between the ring's
    /// inner edge and the barrel.
    #[test]
    fn radial_via() {
        let (ro, d) = (5.0, 0.8);
        let n = 64;
        let ring: String = (0..n)
            .map(|k| {
                let t = 2.0 * PI * k as f64 / n as f64;
                circle_pad(k + 1, 10.0 + ro * t.cos(), 10.0 + ro * t.sin(), d, "F.Cu")
            })
            .collect();
        let disc: Vec<(f64, f64)> = (0..96)
            .map(|k| {
                let t = 2.0 * PI * k as f64 / 96.0;
                (10.0 + (ro + 0.6) * t.cos(), 10.0 + (ro + 0.6) * t.sin())
            })
            .collect();
        let items = [
            zone(&disc, "F.Cu"),
            via(10.0, 10.0, 0.3),
            part("J1", r#"(property "PI_SOURCE" "+3V3")"#, &ring),
            part("U1", r#"(property "PI_LOAD" "+3V3=1A")"#, &circle_pad(1, 10.0, 10.0, 0.6, "B.Cu")),
        ]
        .join("\n");
        let b = board(&items);
        // the ring's inner edge: pads of diameter d every 2πR/64 ≈ 0.49 mm, scalloped; its mean radius
        let gap = 2.0 * PI * ro / n as f64 / 2.0;
        let r_in = ro - ((d / 2.0).powi(2) - gap * gap / 3.0).sqrt();
        let expect = RS / (2.0 * PI) * (r_in / 0.15).ln();
        let mut errs = HashMap::new();
        for (name, cell, mesh) in [
            ("uniform", 0.25, Mesh::default()),
            ("peaceman", 0.25, Mesh::from(Refine::Peaceman)),
            ("qt2", 0.25, qt(2, false)),
            ("fine_l3", 0.25, fine(3)),
        ] {
            let dd = solve_with(&b, cell, &mesh);
            let r = 3.3 - barrel_v(&dd, 0, 0);
            let e = (r - expect) / expect;
            eprintln!("radial {name:9} cell {cell}: {:.4} mΩ vs {:.4} mΩ {:+.2} %", r * 1e3, expect * 1e3, e * 100.0);
            errs.insert(name, e);
        }
        assert!(errs["uniform"] > 0.15);
        for k in ["peaceman", "qt2", "fine_l3"] {
            assert!(errs[k].abs() < 0.03, "{k}: {:+.2} %", errs[k] * 100.0);
        }
    }

    /// A 4 mm strip, 20 mm long, with a 1 mm × 2 mm neck in the middle: the strip's squares plus the
    /// constriction resistance of the two steps (conformal mapping, ratio α = w/W):
    /// ΔR = Rs/π · [(1+α²)/(2α) · ln((1+α)/(1−α)) − ln(4α/(1−α²))].
    #[test]
    fn strip_with_neck() {
        let pts = [
            (0.0, 0.0),
            (9.0, 0.0),
            (9.0, 1.5),
            (11.0, 1.5),
            (11.0, 0.0),
            (20.0, 0.0),
            (20.0, 4.0),
            (11.0, 4.0),
            (11.0, 2.5),
            (9.0, 2.5),
            (9.0, 4.0),
            (0.0, 4.0),
        ];
        let items = [
            zone(&pts, "F.Cu"),
            part("J1", r#"(property "PI_SOURCE" "+3V3")"#, &rect_pad(1, 0.5, 2.0, 1.0, 4.0)),
            part("U1", r#"(property "PI_LOAD" "+3V3=1A")"#, &rect_pad(1, 19.5, 2.0, 1.0, 4.0)),
        ]
        .join("\n");
        let b = board(&items);
        let a: f64 = 0.25;
        let step =
            1.0 / PI * ((1.0 + a * a) / (2.0 * a) * ((1.0 + a) / (1.0 - a)).ln() - (4.0 * a / (1.0 - a * a)).ln());
        // 16 mm of the wide strip (between the pads), the neck, half of the load pad (current drawn evenly)
        let expect = RS * (16.0 / 4.0 + 2.0 / 1.0 + 0.5 / 4.0 + 2.0 * step);
        let mut errs = HashMap::new();
        for (name, cell, mesh) in [
            ("uniform", 0.25, Mesh::default()),
            ("neck1", 0.25, Mesh { neck_levels: 1, ..Mesh::default() }),
            ("edge1", 0.25, Mesh { edge_levels: 1, ..Mesh::default() }),
            ("fine_l3", 0.25, fine(3)),
        ] {
            let d = solve_with(&b, cell, &mesh);
            let r = d.result.max_drop_mv.unwrap() * 1e-3;
            let e = (r - expect) / expect;
            eprintln!(
                "neck {name:8} cell {cell}: {:.4} mΩ vs {:.4} mΩ (steps {:.4}) {:+.2} %",
                r * 1e3,
                expect * 1e3,
                2.0 * step * RS * 1e3,
                e * 100.0
            );
            errs.insert(name, e);
        }
        for (k, tol) in [("uniform", 0.03), ("neck1", 0.02), ("edge1", 0.02), ("fine_l3", 0.01)] {
            assert!(errs[k].abs() < tol, "{k}: {:+.2} %", errs[k] * 100.0);
        }
    }

    #[test]
    #[ignore]
    fn disc_grow_scan() {
        let items = [
            zone(&[(0.0, 0.0), (20.0, 0.0), (20.0, 20.0), (0.0, 20.0)], "F.Cu"),
            via(7.0, 10.0, 0.3),
            via(13.03, 10.01, 0.3),
            part("J1", r#"(property "PI_SOURCE" "+3V3")"#, &circle_pad(1, 7.0, 10.0, 0.6, "B.Cu")),
            part("U1", r#"(property "PI_LOAD" "+3V3=1A")"#, &circle_pad(1, 13.03, 10.01, 0.6, "B.Cu")),
        ]
        .join("\n");
        let b = board(&items);
        let expect = RS * sheet_pair(20.0, 20.0, 7.0, 13.03, 10.0, 0.15);
        for grow in [0.0, 0.2, 0.3, 0.4, 0.5] {
            for (cell, lv) in [(0.25, 1u8), (0.25, 2), (0.25, 3), (0.2, 2), (0.3, 2), (0.15, 0), (0.1, 0)] {
                let m = Mesh { uniform: lv, contact: Contact::Disc, disc_grow: grow, ..Mesh::default() };
                let d = solve_with(&b, cell, &m);
                let r = barrel_v(&d, 0, 0) - barrel_v(&d, 1, 0);
                eprintln!(
                    "grow {grow} h {:.4} (r_w/h {:.2}): {:+.2} %",
                    cell / (1 << lv) as f64,
                    0.15 / (cell / (1 << lv) as f64),
                    (r - expect) / expect * 100.0
                );
            }
        }
    }

    /// Tracks at 0°, 30° and 45°, 0.15 … 1 mm wide, on a 0.2 mm raster: the raster's chain of cells is longer
    /// than a slanted track and its staircase edges carry little current; tracks between one and a few cells
    /// wide are rounded to whole cells. Line conductors (`track_lines`) give R = Rs · L / w at any angle.
    #[test]
    fn track_direction() {
        let len = 10.0;
        let mut rows = vec![];
        for w in [0.15, 0.3, 0.5] {
            for deg in [0.0f64, 30.0, 45.0] {
                let (c, s) = (deg.to_radians().cos(), deg.to_radians().sin());
                let (x1, y1) = (1.0, 1.0);
                let (x2, y2) = (x1 + len * c, y1 + len * s);
                let items = [
                    format!(r#"  (segment (start {x1} {y1}) (end {x2} {y2}) (width {w}) (layer "F.Cu") (net 1))"#),
                    part("J1", r#"(property "PI_SOURCE" "+3V3")"#, &circle_pad(1, x1, y1, 0.05, "F.Cu")),
                    part("U1", r#"(property "PI_LOAD" "+3V3=1A")"#, &circle_pad(1, x2, y2, 0.05, "F.Cu")),
                    // fixes the grid origin off the track's diagonal (see `diagonal_tie`)
                    part("TP1", "", &circle_pad(1, -1.013, -1.029, 0.05, "F.Cu")),
                ]
                .join("\n");
                let b = board(&items);
                // the track from its start to its end; the source and the load are points
                let expect = RS * len / w;
                for (name, mesh) in [
                    ("uniform", Mesh::default()),
                    ("x2", Mesh { uniform: 1, ..Mesh::default() }),
                    ("cut4", Mesh { cut_cell: 4, ..Mesh::default() }),
                    ("lines", Mesh { track_lines: 8.0, ..Mesh::default() }),
                    ("fine_l3", fine(3)),
                ] {
                    let d = solve_with(&b, 0.2, &mesh);
                    let Some(r) = d.result.max_drop_mv.map(|x| x * 1e-3) else {
                        panic!("{w} mm {deg}° {name}: not connected {:?}", d.result.warnings);
                    };
                    let e = (r - expect) / expect;
                    eprintln!(
                        "track {w:4} mm {deg:4}° {name:8}: {:8.3} mΩ vs {:8.3} mΩ {:+6.1} %",
                        r * 1e3,
                        expect * 1e3,
                        e * 100.0
                    );
                    rows.push((w, deg, name, e));
                }
            }
        }
        for (w, deg, name, e) in rows {
            if name == "lines" {
                assert!(e.abs() < 0.03, "{w} mm {deg}° {name}: {:+.1} %", e * 100.0);
            }
            if name == "uniform" && deg == 0.0 && w < 0.2 {
                assert!(e.abs() < 0.03, "{w} mm {deg}° {name}: {:+.1} %", e * 100.0);
            }
        }
    }

    /// 45° tracks that today's raster can break apart: a thin track whose centre line runs through cell
    /// centres (its side cells sit exactly at the thin-track reach, half the cell diagonal, and rounding may
    /// drop them) and a track 1.05 … 1.41 cells wide (filled by its width alone: one diagonal row of cells
    /// where the band holds no second row, not 4-connected). Line conductors keep them whole.
    #[test]
    fn diagonal_breaks() {
        for (w, x0) in [(0.15, 1.0), (0.25, 1.0), (0.25, 1.05)] {
            let items = [
                format!(r#"  (segment (start {x0} 1) (end {} 8.071) (width {w}) (layer "F.Cu") (net 1))"#, x0 + 7.071),
                part("J1", r#"(property "PI_SOURCE" "+3V3")"#, &circle_pad(1, x0, 1.0, 0.1, "F.Cu")),
                part("U1", r#"(property "PI_LOAD" "+3V3=1A")"#, &circle_pad(1, x0 + 7.071, 8.071, 0.1, "F.Cu")),
            ]
            .join("\n");
            let b = board(&items);
            let (rails, _) = rails_from_fields(&b);
            let r = solve_rail(&b, &rails[0], &PiParams { cell: Some(0.2), ..Default::default() }).unwrap();
            let d = solve_with(&b, 0.2, &Mesh { track_lines: 8.0, ..Mesh::default() });
            let expect = RS * 10.0 / w * 1e3;
            eprintln!(
                "45° track {w} mm from x {x0}: raster drop {:?} mV {:?}; line track {:.2} mV (expected {expect:.2})",
                r.max_drop_mv,
                r.warnings,
                d.result.max_drop_mv.unwrap()
            );
            assert!((d.result.max_drop_mv.unwrap() / expect - 1.0).abs() < 0.03);
        }
    }
}

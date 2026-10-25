//! Plane-pair cavities of finite boards for the via ladder (validation/fdtd/BOARD_RESONANCES.md).
//!
//! The ladder (`via_pbm`) treats every plane pair a via crosses as an infinite radial line with
//! the nearby return vias as shorts: Z_ij = ωμd/4 · H0⁽²⁾(kρij). A real plane pair is finite: its
//! edges (open, magnetic walls) reflect the wave the via launches, so the pair resonates, and
//! every via that crosses it sees the others (via-to-via coupling). This module gives the
//! plane-pair Green's function of a finite cavity in place of the Hankel function:
//!
//! * [`RectEnv`]: a rectangle a × b with magnetic walls, the modal series summed in closed form
//!   along one side (single series) and accelerated by subtracting its asymptotic terms, whose
//!   sum is known (Kummer): the logarithmic singularity at the source comes out exactly, so a
//!   via's self term is the Green's function at its radius, as H0⁽²⁾(ka) in the ladder.
//! * [`GridEnv`]: any plane shape on a square grid (five-point Helmholtz operator, one sparse
//!   LU per frequency shared by every port), the shorting vias of the pair as the inductance of a
//!   post in its cell, with the near field of the ports taken from the exact free-space function
//!   (the grid supplies the reflected part only: Peaceman's equivalent radius of a grid node, and
//!   the exact lattice Green's function next to the port). A window of a large
//!   pair ([`GridEnv::new_cut`]) ends in an absorbing boundary on the sides where it cuts the
//!   copper.
//! * Both: capacitors between the two planes as series R–L–C loads, and the radiation of the open
//!   board edges as a slot conductance per unit length ([`slot_conductance`]).
//!
//! The geometry side ([`pair_cavity`], [`pair_window`]) finds the region of a plane pair around a
//! via on the board: the copper of both planes, their overlap, the connected part that contains
//! the via, the vias and plated holes that join both planes inside it, and the capacitors between
//! them.
//!
//! Lengths in mm in the geometry, SI in the electrical functions.

use crate::board::Board;
use crate::geom::{self, BBox, P};
use crate::num::{Cx, EPS0, MU0};
use std::f64::consts::PI;

// --- geometry ------------------------------------------------------------------------------

/// Cells of a square raster: cell (i, j) has its centre at (x0 + (i + ½)h, y0 + (j + ½)h).
#[derive(Debug, Clone)]
pub struct Mask {
    pub x0: f64,
    pub y0: f64,
    pub h: f64,
    pub nx: usize,
    pub ny: usize,
    pub on: Vec<bool>,
}

impl Mask {
    pub fn new(bb: &BBox, h: f64) -> Mask {
        let nx = (((bb.max[0] - bb.min[0]) / h).ceil() as usize).max(1);
        let ny = (((bb.max[1] - bb.min[1]) / h).ceil() as usize).max(1);
        Mask { x0: bb.min[0], y0: bb.min[1], h, nx, ny, on: vec![false; nx * ny] }
    }
    pub fn centre(&self, i: usize, j: usize) -> P {
        [self.x0 + (i as f64 + 0.5) * self.h, self.y0 + (j as f64 + 0.5) * self.h]
    }
    /// cell containing p (clamped to the raster)
    pub fn cell_of(&self, p: P) -> (usize, usize) {
        let i = ((p[0] - self.x0) / self.h).floor().clamp(0.0, self.nx as f64 - 1.0) as usize;
        let j = ((p[1] - self.y0) / self.h).floor().clamp(0.0, self.ny as f64 - 1.0) as usize;
        (i, j)
    }
    pub fn get(&self, i: usize, j: usize) -> bool {
        self.on[j * self.nx + i]
    }
    pub fn count(&self) -> usize {
        self.on.iter().filter(|&&b| b).count()
    }
    /// add the polygon rings (each even-odd on its own, KiCad's fractured fills) by scanlines
    pub fn fill_rings(&mut self, rings: &[&[P]]) {
        for pts in rings {
            let m = pts.len();
            if m < 3 {
                continue;
            }
            for j in 0..self.ny {
                let y = self.y0 + (j as f64 + 0.5) * self.h;
                let mut xs: Vec<f64> = Vec::new();
                for i in 0..m {
                    let (a, b) = (pts[i], pts[(i + 1) % m]);
                    if (a[1] > y) != (b[1] > y) {
                        xs.push(a[0] + (y - a[1]) / (b[1] - a[1]) * (b[0] - a[0]));
                    }
                }
                xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
                for pair in xs.chunks_exact(2) {
                    let i0 = ((pair[0] - self.x0) / self.h - 0.5).ceil().max(0.0) as usize;
                    let i1 = ((pair[1] - self.x0) / self.h - 0.5).floor();
                    if i1 < 0.0 {
                        continue;
                    }
                    for i in i0..=(i1 as usize).min(self.nx - 1) {
                        self.on[j * self.nx + i] = true;
                    }
                }
            }
        }
    }
    /// close isolated one-cell holes (antipads of single vias smaller than a cell)
    pub fn close_pinholes(&mut self) {
        let mut add = vec![];
        for j in 1..self.ny.saturating_sub(1) {
            for i in 1..self.nx.saturating_sub(1) {
                if !self.get(i, j)
                    && self.get(i - 1, j)
                    && self.get(i + 1, j)
                    && self.get(i, j - 1)
                    && self.get(i, j + 1)
                {
                    add.push(j * self.nx + i);
                }
            }
        }
        for k in add {
            self.on[k] = true;
        }
    }
    /// keep only the 4-connected part that contains cell `seed`
    pub fn keep_component(&mut self, seed: (usize, usize)) {
        let mut keep = vec![false; self.on.len()];
        let mut stack = vec![seed];
        while let Some((i, j)) = stack.pop() {
            let k = j * self.nx + i;
            if keep[k] || !self.on[k] {
                continue;
            }
            keep[k] = true;
            if i > 0 {
                stack.push((i - 1, j));
            }
            if i + 1 < self.nx {
                stack.push((i + 1, j));
            }
            if j > 0 {
                stack.push((i, j - 1));
            }
            if j + 1 < self.ny {
                stack.push((i, j + 1));
            }
        }
        self.on = keep;
    }
    /// bounding box of the cells that are on
    pub fn bbox(&self) -> BBox {
        let mut bb = BBox::empty();
        for j in 0..self.ny {
            for i in 0..self.nx {
                if self.get(i, j) {
                    let c = self.centre(i, j);
                    bb.add([c[0] - 0.5 * self.h, c[1] - 0.5 * self.h]);
                    bb.add([c[0] + 0.5 * self.h, c[1] + 0.5 * self.h]);
                }
            }
        }
        bb
    }
    /// lengths of the straight runs of the region's boundary (cell faces between an on and an
    /// off cell, or the raster's border), in mm
    pub fn edge_runs(&self) -> Vec<f64> {
        let on = |i: isize, j: isize| {
            i >= 0 && j >= 0 && (i as usize) < self.nx && (j as usize) < self.ny && self.get(i as usize, j as usize)
        };
        let mut runs = vec![];
        // faces normal to y (between rows j−1 and j), along x; then faces normal to x, along y
        for horizontal in [true, false] {
            let (n_outer, n_inner) = if horizontal { (self.ny + 1, self.nx) } else { (self.nx + 1, self.ny) };
            for o in 0..n_outer as isize {
                for side in [false, true] {
                    let mut len = 0usize;
                    for q in 0..=n_inner as isize {
                        let (a, b) = if horizontal { (on(q, o - 1), on(q, o)) } else { (on(o - 1, q), on(o, q)) };
                        let edge = q < n_inner as isize && if side { a && !b } else { !a && b };
                        if edge {
                            len += 1;
                        } else if len > 0 {
                            runs.push(len as f64 * self.h);
                            len = 0;
                        }
                    }
                }
            }
        }
        runs
    }

    /// boundary faces of the region: (cell column, cell row, length of the straight run the face
    /// belongs to (mm), side of the raster's border the face lies on: 0 x0, 1 x1, 2 y0, 3 y1, or
    /// none inside); every face is one cell side long
    pub fn edge_faces(&self) -> Vec<(usize, usize, f64, Option<u8>)> {
        let on = |i: isize, j: isize| {
            i >= 0 && j >= 0 && (i as usize) < self.nx && (j as usize) < self.ny && self.get(i as usize, j as usize)
        };
        let mut out = vec![];
        for horizontal in [true, false] {
            let (n_outer, n_inner) = if horizontal { (self.ny + 1, self.nx) } else { (self.nx + 1, self.ny) };
            for o in 0..n_outer as isize {
                for side in [false, true] {
                    // the border side of this line of faces, if it is one
                    let border = match (horizontal, side) {
                        (true, true) if o as usize == n_outer - 1 => Some(3u8),
                        (true, false) if o == 0 => Some(2u8),
                        (false, true) if o as usize == n_outer - 1 => Some(1u8),
                        (false, false) if o == 0 => Some(0u8),
                        _ => None,
                    };
                    let mut run: Vec<(usize, usize)> = vec![];
                    for q in 0..=n_inner as isize {
                        let (a, b) = if horizontal { (on(q, o - 1), on(q, o)) } else { (on(o - 1, q), on(o, q)) };
                        let edge = q < n_inner as isize && if side { a && !b } else { !a && b };
                        if edge {
                            // the cell of the region next to the face
                            let c = match (horizontal, side) {
                                (true, true) => (q as usize, (o - 1) as usize),
                                (true, false) => (q as usize, o as usize),
                                (false, true) => ((o - 1) as usize, q as usize),
                                (false, false) => (o as usize, q as usize),
                            };
                            run.push(c);
                        } else if !run.is_empty() {
                            let l = run.len() as f64 * self.h;
                            out.extend(run.drain(..).map(|(i, j)| (i, j, l, border)));
                        }
                    }
                }
            }
        }
        out
    }

    /// nearest cell that is on, within `rmax` (mm) of p
    pub fn nearest_on(&self, p: P, rmax: f64) -> Option<(usize, usize)> {
        let (ci, cj) = self.cell_of(p);
        let r = (rmax / self.h).ceil() as isize;
        let mut best: Option<(f64, (usize, usize))> = None;
        for dj in -r..=r {
            for di in -r..=r {
                let (i, j) = (ci as isize + di, cj as isize + dj);
                if i < 0 || j < 0 || i >= self.nx as isize || j >= self.ny as isize {
                    continue;
                }
                let (i, j) = (i as usize, j as usize);
                if !self.get(i, j) {
                    continue;
                }
                let d = geom::len(geom::sub(self.centre(i, j), p));
                if d <= rmax && best.map_or(true, |b| d < b.0) {
                    best = Some((d, (i, j)));
                }
            }
        }
        best.map(|b| b.1)
    }
}

/// A post that joins both planes of a pair (a stitching via or a plated hole of their net).
#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct Short {
    pub at: P,
    /// barrel radius (mm)
    pub r: f64,
}

/// The plane pair between copper layers `layers.0` (upper) and `layers.1` (lower) around a via.
#[derive(Debug, Clone)]
pub struct PairCavity {
    pub layers: (u16, u16),
    pub nets: (u32, u32),
    /// distance between the planes (mm), thickness-weighted εr and tanδ of the dielectric
    pub d_mm: f64,
    pub er: f64,
    pub tand: f64,
    /// the connected overlap of both planes that contains the via
    pub mask: Mask,
    pub area_mm2: f64,
    /// bounding box of the region
    pub bbox: BBox,
    /// posts joined to both planes inside the region
    pub shorts: Vec<Short>,
    /// capacitors with one pad on each plane net inside the region (pair of different nets)
    pub caps: Vec<CavCap>,
    /// sides of the raster (x0, x1, y0, y1) where a window cut the region (the planes go on)
    pub cut: [bool; 4],
}

/// A capacitor between the two planes of a pair.
#[derive(Debug, Clone)]
pub struct CavCap {
    /// centre between its pads (mm)
    pub at: P,
    /// capacitance from the footprint value, if it reads as one
    pub c: Option<f64>,
    /// chip package from the footprint and the pads ("0402", …, "large")
    pub pkg: &'static str,
}

/// Decoupling-capacitor parasitics from `data/components.json` ("decoupling", the same data as
/// the PDN model): body ESL by package, the ESR(C) trend and the mounting inductance (vias at the
/// pads), all typical values (estimates).
#[derive(Debug, Clone, serde::Deserialize)]
pub struct DecapData {
    esl_nh: std::collections::BTreeMap<String, f64>,
    esr: DecapEsr,
    mount_nh: DecapMount,
}
#[derive(Debug, Clone, serde::Deserialize)]
struct DecapEsr {
    k_ohm: f64,
    c_ref_f: f64,
    exp: f64,
    min_ohm: f64,
    max_ohm: f64,
}
#[derive(Debug, Clone, serde::Deserialize)]
struct DecapMount {
    value: f64,
}

impl DecapData {
    pub fn builtin() -> &'static DecapData {
        #[derive(serde::Deserialize)]
        struct Wrap {
            decoupling: DecapData,
        }
        static DATA: std::sync::OnceLock<DecapData> = std::sync::OnceLock::new();
        DATA.get_or_init(|| {
            serde_json::from_str::<Wrap>(crate::topology::COMPONENTS_JSON)
                .expect("data/components.json: decoupling")
                .decoupling
        })
    }
    /// (ESR Ω, ESL + mounting inductance H) of a capacitor c (F) in package pkg
    pub fn rl(&self, pkg: &str, c: f64) -> (f64, f64) {
        let esl = self.esl_nh.get(pkg).or_else(|| self.esl_nh.get("large")).copied().unwrap_or(1.5);
        let e = &self.esr;
        let r = (e.k_ohm * (e.c_ref_f / c).powf(e.exp)).clamp(e.min_ohm, e.max_ohm);
        (r, (esl + self.mount_nh.value) * 1e-9)
    }
}

impl PairCavity {
    /// rectangle with the region's bounding box (the "rectangular equivalent")
    pub fn rect(&self) -> (f64, f64) {
        (self.bbox.max[0] - self.bbox.min[0], self.bbox.max[1] - self.bbox.min[1])
    }
    /// resonances (Hz) of the bounding rectangle with magnetic walls, lowest first
    pub fn rect_modes(&self, n: usize) -> Vec<(f64, usize, usize)> {
        let (a, b) = self.rect();
        let mut v = vec![];
        for m in 0..12 {
            for q in 0..12 {
                if m + q == 0 {
                    continue;
                }
                let f = crate::num::C0 / (2.0 * self.er.sqrt())
                    * ((m as f64 / a).powi(2) + (q as f64 / b).powi(2)).sqrt()
                    * 1e3;
                v.push((f, m, q));
            }
        }
        v.sort_by(|x, y| x.0.partial_cmp(&y.0).unwrap());
        v.truncate(n);
        v
    }
}

/// thickness-weighted εr and tanδ between copper layers i < j
fn diel_between(board: &Board, i: usize, j: usize) -> (f64, f64, f64) {
    let su = board.stackup();
    let (mut d, mut e, mut t) = (0.0, 0.0, 0.0);
    for k in i..j {
        let dk = su.dielectrics[k].thickness + 0.5 * (su.copper[k].thickness + su.copper[k + 1].thickness);
        d += dk;
        e += dk * su.dielectrics[k].epsilon_r;
        t += dk * su.dielectrics[k].loss_tangent;
    }
    (d, e / d.max(1e-12), t / d.max(1e-12))
}

/// Capacitance (F) from a footprint value: "100n", "0.1uF", "4u7", "10µF/6.3V", "1p5", "2.2 nF".
pub fn parse_cap_value(v: &str) -> Option<f64> {
    let v = v.trim().replace('µ', "u").replace("μ", "u");
    let t: String = v.chars().take_while(|c| !c.is_whitespace() && *c != '/' && *c != ',').collect();
    let t = t.to_lowercase();
    let t = t.trim_end_matches('f');
    let mult = |c: char| match c {
        'p' => Some(1e-12),
        'n' => Some(1e-9),
        'u' => Some(1e-6),
        'm' => Some(1e-3),
        _ => None,
    };
    if let Some(pos) = t.find(|c: char| mult(c).is_some()) {
        let c = t[pos..].chars().next().unwrap();
        let (a, b) = (&t[..pos], &t[pos + 1..]);
        let x: f64 = if b.is_empty() { a.parse().ok()? } else { format!("{a}.{b}").parse().ok()? };
        return Some(x * mult(c)?);
    }
    t.parse::<f64>().ok().filter(|x| *x > 0.0 && *x < 1.0)
}

/// Region of the plane pair between layers `la` < `lb` around `at`: the copper of the plane net
/// on each layer (the net whose fill opens around the via, `via::plane_clearance`), their overlap
/// on a raster of at most `max_cells` cells (cell ≥ `h_min` mm), the connected part next to the
/// via, and the posts that join both planes in it. None if a layer has no plane at the via.
pub fn pair_cavity(
    board: &Board,
    at: P,
    la: u16,
    lb: u16,
    own: u32,
    partner: Option<u32>,
    max_cells: usize,
    h_min: f64,
) -> Option<PairCavity> {
    pair_region(board, at, la, lb, own, partner, None, max_cells, h_min)
}

/// As [`pair_cavity`] within a square window of half-width `radius` (mm) around the via, on cells
/// of `h` mm; the sides where the window cuts the region are marked (`PairCavity::cut`).
pub fn pair_window(
    board: &Board,
    at: P,
    la: u16,
    lb: u16,
    own: u32,
    partner: Option<u32>,
    radius: f64,
    h: f64,
) -> Option<PairCavity> {
    pair_region(board, at, la, lb, own, partner, Some(radius), usize::MAX, h)
}

#[allow(clippy::too_many_arguments)]
fn pair_region(
    board: &Board,
    at: P,
    la: u16,
    lb: u16,
    own: u32,
    partner: Option<u32>,
    window: Option<f64>,
    max_cells: usize,
    h_min: f64,
) -> Option<PairCavity> {
    let (la, lb) = (la.min(lb), la.max(lb));
    let (open_a, na) = crate::via::plane_clearance(board, at, la, own, partner)?;
    let (open_b, nb) = crate::via::plane_clearance(board, at, lb, own, partner)?;
    let fills = |l: u16, n: u32| board.zones.iter().filter(move |z| z.layer == l && z.net == n);
    let mut bba = BBox::empty();
    for z in fills(la, na) {
        bba.add(z.bbox.min);
        bba.add(z.bbox.max);
    }
    let mut bbb = BBox::empty();
    for z in fills(lb, nb) {
        bbb.add(z.bbox.min);
        bbb.add(z.bbox.max);
    }
    if !bba.is_valid() || !bbb.is_valid() {
        return None;
    }
    let mut bb = BBox {
        min: [bba.min[0].max(bbb.min[0]), bba.min[1].max(bbb.min[1])],
        max: [bba.max[0].min(bbb.max[0]), bba.max[1].min(bbb.max[1])],
    };
    let mut cut = [false; 4];
    if let Some(r) = window {
        let w = [at[0] - r, at[0] + r, at[1] - r, at[1] + r];
        cut = [w[0] > bb.min[0], w[1] < bb.max[0], w[2] > bb.min[1], w[3] < bb.max[1]];
        bb = BBox { min: [bb.min[0].max(w[0]), bb.min[1].max(w[2])], max: [bb.max[0].min(w[1]), bb.max[1].min(w[3])] };
    }
    if bb.max[0] <= bb.min[0] || bb.max[1] <= bb.min[1] {
        return None;
    }
    let h = if window.is_some() {
        h_min
    } else {
        (((bb.max[0] - bb.min[0]) * (bb.max[1] - bb.min[1]) / max_cells as f64).sqrt()).max(h_min)
    };
    let overlap = || {
        let mut ma = Mask::new(&bb, h);
        let ra: Vec<&[P]> = fills(la, na).map(|z| &z.pts[..]).collect();
        ma.fill_rings(&ra);
        let mut mb = Mask::new(&bb, h);
        let rb: Vec<&[P]> = fills(lb, nb).map(|z| &z.pts[..]).collect();
        mb.fill_rings(&rb);
        for k in 0..ma.on.len() {
            ma.on[k] = ma.on[k] && mb.on[k];
        }
        ma.close_pinholes();
        ma
    };
    // a via's openings in both planes belong to the region (filled up to the nearest copper of
    // either plane and a cell beyond): the ladder models the openings, the cavity sees the plane
    // pair around them
    let fill_at = |ma: &mut Mask, p: P, open: f64| {
        let rfill = open.max(0.5) + 1.5 * h;
        let (ci, cj) = ma.cell_of(p);
        let rc = (rfill / h).ceil() as isize;
        for dj in -rc..=rc {
            for di in -rc..=rc {
                let (i, j) = (ci as isize + di, cj as isize + dj);
                if i >= 0
                    && j >= 0
                    && (i as usize) < ma.nx
                    && (j as usize) < ma.ny
                    && geom::len(geom::sub(ma.centre(i as usize, j as usize), p)) <= rfill
                {
                    ma.on[j as usize * ma.nx + i as usize] = true;
                }
            }
        }
    };
    // the overlap is the same for every via of the pair: computed once (process-wide)
    let key =
        format!("{la},{lb},{na},{nb},{:?},{:?},{}", bb.min.map(f64::to_bits), bb.max.map(f64::to_bits), h.to_bits());
    let cached = OVERLAPS.lock().unwrap_or_else(|e| e.into_inner()).get(&key).cloned();
    let base = match cached {
        Some(m) => m,
        None => {
            let m = std::sync::Arc::new(overlap());
            let mut c = OVERLAPS.lock().unwrap_or_else(|e| e.into_inner());
            if c.len() > 256 {
                c.clear();
            }
            c.insert(key, m.clone());
            m
        }
    };
    let mut ma = (*base).clone();
    ma.nearest_on(at, 3.0)?;
    fill_at(&mut ma, at, open_a.max(open_b));
    ma.keep_component(ma.cell_of(at));
    let area = ma.count() as f64 * h * h;
    let bbox = ma.bbox();
    let inside = |p: P| -> bool {
        if p[0] < ma.x0 || p[1] < ma.y0 || p[0] >= ma.x0 + ma.nx as f64 * h || p[1] >= ma.y0 + ma.ny as f64 * h {
            return false;
        }
        let (i, j) = ma.cell_of(p);
        ma.get(i, j)
    };
    // a post with a thermal relief sits in its own clearance: next to the region counts
    let near = |p: P| -> bool {
        p[0] >= ma.x0 - 0.6
            && p[1] >= ma.y0 - 0.6
            && p[0] <= ma.x0 + ma.nx as f64 * h + 0.6
            && p[1] <= ma.y0 + ma.ny as f64 * h + 0.6
            && (inside(p) || ma.nearest_on(p, 0.6).is_some())
    };
    let mut shorts = vec![];
    if na == nb {
        for v in &board.vias {
            if v.net == na && v.from <= la && v.to >= lb && near(v.at) {
                // a back-drilled range that cuts the barrel between the planes
                if let Some((b0, b1)) = v.backdrill {
                    if b0 <= lb && b1 >= la {
                        continue;
                    }
                }
                shorts.push(Short { at: v.at, r: (v.drill / 2.0).max(0.05) });
            }
        }
        for p in &board.pads {
            if p.net == na && p.drill > 0.0 && p.layers.contains(&la) && p.layers.contains(&lb) && near(p.at) {
                shorts.push(Short { at: p.at, r: (p.drill / 2.0).max(0.05) });
            }
        }
    }
    // capacitors between the two plane nets (a pair of different nets: GND and a supply)
    let mut caps = vec![];
    if na != nb {
        let mut by_fp: Vec<Vec<usize>> = vec![vec![]; board.footprints.len()];
        for (i, p) in board.pads.iter().enumerate() {
            if let Some(v) = by_fp.get_mut(p.fp as usize) {
                v.push(i);
            }
        }
        for (fp, ids) in by_fp.iter().enumerate() {
            if board.footprints[fp].dnp || ids.len() != 2 {
                continue;
            }
            let (p0, p1) = (&board.pads[ids[0]], &board.pads[ids[1]]);
            if (p0.net == na && p1.net == nb) || (p0.net == nb && p1.net == na) {
                let c = geom::lerp(p0.at, p1.at, 0.5);
                if inside(c) || inside(p0.at) || inside(p1.at) {
                    let pkg = crate::topology::package_of(board, fp, ids[0], ids[1]);
                    caps.push(CavCap { at: c, c: parse_cap_value(&board.footprints[fp].value), pkg });
                }
            }
        }
    }
    let (d, er, tand) = diel_between(board, la as usize, lb as usize);
    Some(PairCavity {
        layers: (la, lb),
        nets: (na, nb),
        d_mm: d,
        er,
        tand,
        mask: ma,
        area_mm2: area,
        bbox,
        shorts,
        caps,
        cut,
    })
}

// --- medium ----------------------------------------------------------------------------------

/// Dielectric and plates of a plane pair.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PairMedium {
    /// distance between the plates (m)
    pub d: f64,
    pub er: f64,
    pub tand: f64,
    /// Some(f_l): tanδ scaled by f_l/f (a conductivity fixed at f_l, as the FDTD references)
    pub f_loss: Option<f64>,
    /// conductivity of the plates (S/m) for their skin-effect loss; None: lossless
    pub sigma: Option<f64>,
    /// radiation from the open edges: the region's area (m²) and the lengths of its straight
    /// edges (m); area 0: none
    #[serde(default)]
    pub area: f64,
    #[serde(default)]
    pub edges: Vec<f64>,
}

/// Radiation conductance (S) of the open edge of a plane pair, `w` (m) long, at wavenumber `k0`
/// in free space, the gap small against the wavelength: half that of the same slot in a ground
/// plane (Balanis, Antenna Theory, the radiating edge of a patch: G = I/(120π²),
/// I = ∫₀^π sin²(k0 w cosθ / 2)/cos²θ · sin³θ dθ; (w/λ)²/90 for a short slot, w/(120λ) for a long
/// one) – a board edge has no flange: its magnetic current radiates into the full space without
/// the image a ground plane adds, which doubles the radiated power.
pub fn slot_conductance(w: f64, k0: f64) -> f64 {
    let x = 0.5 * k0 * w;
    let n = 256;
    let mut s = 0.0;
    for i in 0..n {
        let th = PI * (i as f64 + 0.5) / n as f64;
        let c = th.cos();
        let v = if c.abs() < 1e-9 { x * x } else { ((x * c).sin() / c).powi(2) };
        s += v * th.sin().powi(3);
    }
    0.5 * s * PI / n as f64 / (120.0 * PI * PI)
}

impl PairMedium {
    /// equivalent loss tangent of the edge radiation for the modal series: 1/Q = 1.5·d·ΣG/(ωεA)
    /// (a cosine mode has twice its mean |V|² on the walls across which it varies and its mean on
    /// the others: 1.4–1.6 averaged over the perimeter for the low modes)
    pub fn tand_rad(&self, f: f64) -> f64 {
        if self.area <= 0.0 || self.edges.is_empty() {
            return 0.0;
        }
        let w = 2.0 * PI * f;
        let k0 = w / crate::num::C0;
        let g: f64 = self.edges.iter().map(|&l| slot_conductance(l, k0)).sum();
        1.5 * self.d * g / (w * EPS0 * self.er * self.area)
    }

    /// complex k² (1/m²) at f: k² = ω²μ0ε0·ε(f) (Djordjevic-Sarkar), with plates of surface impedance (1 + j)Rs:
    /// k²(1 + 2Rs/(ωμd) − j 2Rs/(ωμd)) (as the ladder's radial line), and the edge radiation
    /// as a further loss tangent
    pub fn k2(&self, f: f64) -> Cx {
        self.k2_with(f, self.tand_rad(f))
    }

    /// k² without the edge radiation (the grid puts it on its boundary cells)
    pub fn k2_bulk(&self, f: f64) -> Cx {
        self.k2_with(f, 0.0)
    }

    fn k2_with(&self, f: f64, extra: f64) -> Cx {
        let w = 2.0 * PI * f;
        // the dielectric as the ladder's: Djordjevic-Sarkar, or a conductivity fixed at f_l
        let e = match self.f_loss {
            Some(fl) => Cx::new(self.er, -self.er * self.tand * fl / f),
            None => crate::rf::eps_c(self.er, self.tand, f),
        } - Cx::new(0.0, self.er * extra);
        let mut k2 = e.scale(w * w * MU0 * EPS0);
        if let Some(sig) = self.sigma {
            let x = 2.0 * (PI * f * MU0 / sig).sqrt() / (w * MU0 * self.d);
            k2 = k2 * Cx::new(1.0 + x, -x);
        }
        k2
    }
}

/// free-space plane-pair Green's function (−j/4)·H0⁽²⁾(kρ) (the ladder's radial line: Z = jωμd·G)
pub fn green_free(k2: Cx, rho: f64) -> Cx {
    let mut k = k2.sqrt();
    if k.im > 0.0 {
        k = Cx::new(-k.re, -k.im);
    }
    Cx::new(0.0, -0.25) * crate::via_pbm::hankel2_0(k.scale(rho))
}

/// A port of a cavity environment: a post of radius `r` at (x, y) (m).
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct EnvPort {
    pub x: f64,
    pub y: f64,
    pub r: f64,
}

/// A post inside the cavity closed by an impedance R + jωL + 1/(jωC) (a capacitor with its
/// mounting); `c: None` and R = L = 0 is a short (a stitching via).
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct Load {
    pub at: EnvPort,
    pub r_ohm: f64,
    pub l: f64,
    pub c: Option<f64>,
}

impl Load {
    fn z(&self, w: f64) -> Cx {
        Cx::new(self.r_ohm, w * self.l - self.c.map_or(0.0, |c| 1.0 / (w * c)))
    }
}

fn csolve_small(a: &[Cx], b: &[Cx], n: usize, nrhs: usize) -> Vec<Cx> {
    // Gaussian elimination with partial pivoting (n small)
    let mut a = a.to_vec();
    let mut b = b.to_vec();
    let n2 = |z: Cx| z.re * z.re + z.im * z.im;
    for c in 0..n {
        let p = (c..n).max_by(|&i, &j| n2(a[i * n + c]).partial_cmp(&n2(a[j * n + c])).unwrap()).unwrap();
        if p != c {
            for k in 0..n {
                a.swap(c * n + k, p * n + k);
            }
            for k in 0..nrhs {
                b.swap(c * nrhs + k, p * nrhs + k);
            }
        }
        let inv = a[c * n + c].inv();
        for i in c + 1..n {
            let f = a[i * n + c] * inv;
            for k in c..n {
                a[i * n + k] = a[i * n + k] - f * a[c * n + k];
            }
            for k in 0..nrhs {
                b[i * nrhs + k] = b[i * nrhs + k] - f * b[c * nrhs + k];
            }
        }
    }
    for c in (0..n).rev() {
        let inv = a[c * n + c].inv();
        for k in 0..nrhs {
            let mut s = b[c * nrhs + k];
            for j in c + 1..n {
                s = s - a[c * n + j] * b[j * nrhs + k];
            }
            b[c * nrhs + k] = s * inv;
        }
    }
    b
}

/// Reduce the loads (indices n..n+nl of the full matrix `g`, size (n+nl)²) out of a Green's
/// function matrix: G = G_pp − G_pl (G_ll + Z_l/(jωμd))⁻¹ G_lp.
fn reduce_loads(g: &[Cx], n: usize, loads: &[Load], w: f64, d: f64) -> Vec<Cx> {
    let nl = loads.len();
    let nt = n + nl;
    let mut out: Vec<Cx> = (0..n * n).map(|q| g[(q / n) * nt + q % n]).collect();
    if nl == 0 {
        return out;
    }
    let jwmd = Cx::new(0.0, w * MU0 * d);
    let mut gll: Vec<Cx> = (0..nl * nl).map(|q| g[(n + q / nl) * nt + n + q % nl]).collect();
    for (k, l) in loads.iter().enumerate() {
        gll[k * nl + k] = gll[k * nl + k] + l.z(w) / jwmd;
    }
    let glp: Vec<Cx> = (0..nl * n).map(|q| g[(n + q / n) * nt + q % n]).collect();
    let x = csolve_small(&gll, &glp, nl, n);
    for i in 0..n {
        for j in 0..n {
            let mut s = Cx::new(0.0, 0.0);
            for k in 0..nl {
                s = s + g[i * nt + n + k] * x[k * n + j];
            }
            out[i * n + j] = out[i * n + j] - s;
        }
    }
    out
}

// --- rectangular cavity ----------------------------------------------------------------------

/// Σ_{m≥1} cos(mθ) e^{−mτ} / m = −½ ln(1 − 2e^{−τ} cos θ + e^{−2τ})
fn log_sum(theta: f64, tau: f64) -> f64 {
    let q = (-tau).exp();
    // 1 − 2q cos θ + q² = (1 − q)² + 4q sin²(θ/2)
    let s = (0.5 * theta).sin();
    let v = (1.0 - q).powi(2) + 4.0 * q * s * s;
    -0.5 * v.max(1e-300).ln()
}

/// Rectangular plane pair [0, a] × [0, b] (m) with magnetic walls (open board edges).
///
/// Green's function of (∇² + k²)G = −δ with ∂G/∂n = 0 on the walls, Z_ij = jωμd·G(r_i, r_j):
///
/// G = (1/a) Σ_m εm cos(αm x) cos(αm x') gm(y, y'), αm = mπ/a, ε0 = 1, εm = 2,
/// gm = [e^{−γ|y−y'|} + e^{−γ(y+y')} + e^{−γ(2b−y−y')} + e^{−γ(2b−|y−y'|)}] / (2γ(1 − e^{−2γb})),
/// γ = √(αm² − k²): the double cosine series summed in closed form along y. For large m the
/// terms approach (1/(2πm)) Σ_u Σ_v cos(mπu/a) e^{−mπv/a} (u = x ∓ x'; v = |y − y'|, y + y',
/// 2b − y − y'), whose sum is [`log_sum`] (Kummer): it carries the logarithmic singularity, so
/// a post's self term is G at its radius, as H0⁽²⁾(ka) in the ladder. The k² terms of the
/// expansion beyond the last mode M are added from frequency-independent sums, so the remainder
/// falls as 1/m⁵.
#[derive(Debug, Clone)]
pub struct RectEnv {
    pub a: f64,
    pub b: f64,
    /// x and y exchanged against the caller's frame (the series runs along the shorter side)
    pub swap: bool,
    pub medium: PairMedium,
    pub ports: Vec<EnvPort>,
    pub loads: Vec<Load>,
    modes: usize,
    /// all posts (ports, then loads) in the internal frame: x, y, r
    pts: Vec<(f64, f64, f64)>,
    /// per pair (row-major, n_all²): closed-form asymptotic sum minus its first M terms
    cconst: Vec<f64>,
    /// per pair: coefficient of k² of the second-order tail beyond M
    tail2: Vec<f64>,
    cache: std::cell::RefCell<std::collections::HashMap<u64, Vec<Cx>>>,
}

impl RectEnv {
    /// `a` × `b` (m) in the caller's frame, ports and loads in it; `fmax` sets the number of
    /// modes of the exact part.
    pub fn new(a: f64, b: f64, medium: PairMedium, ports: Vec<EnvPort>, loads: Vec<Load>, fmax: f64) -> RectEnv {
        let swap = a > b;
        let (aa, bb) = if swap { (b, a) } else { (a, b) };
        let tr = |p: &EnvPort| if swap { (p.y, p.x, p.r) } else { (p.x, p.y, p.r) };
        let pts: Vec<(f64, f64, f64)> = ports.iter().map(tr).chain(loads.iter().map(|l| tr(&l.at))).collect();
        let kmax = medium.k2(fmax).abs().sqrt();
        let modes = ((3.0 * kmax * aa / PI) as usize + 40).min(6000);
        let n = pts.len();
        let mut cconst = vec![0.0; n * n];
        let mut tail2 = vec![0.0; n * n];
        for i in 0..n {
            for j in i..n {
                let sides: &[f64] = if i == j { &[1.0, -1.0] } else { &[1.0] };
                let mut cc_sum = 0.0;
                let mut t2_sum = 0.0;
                for &sd in sides {
                    let (us, vs) = Self::uv(&pts, i, j, bb, sd);
                    let mut closed = 0.0;
                    for &u in &us {
                        for &v in &vs {
                            closed += log_sum(PI * u / aa, PI * v / aa);
                        }
                    }
                    closed /= 2.0 * PI;
                    let mut first = 0.0;
                    for m in 1..=modes {
                        let al = m as f64 * PI / aa;
                        let mut s = 0.0;
                        for &u in &us {
                            let cu = (al * u).cos();
                            for &v in &vs {
                                s += cu * (-al * v).exp();
                            }
                        }
                        first += s / (2.0 * PI * m as f64);
                    }
                    // second-order tail Σ_{m>M} Re(z^m)·[v a/(4π² m²) + a²/(4π³ m³)], z = e^{−τ + iθ}
                    let last = modes + 20 * modes + 4000;
                    let mut t2 = 0.0;
                    for &u in &us {
                        for &v in &vs {
                            let (th, ta) = (PI * u / aa, PI * v / aa);
                            if ta * (modes as f64) > 40.0 {
                                continue;
                            }
                            let z = Cx::new(-ta, th).exp();
                            let mut zm = Cx::new(-ta * (modes as f64 + 1.0), th * (modes as f64 + 1.0)).exp();
                            let (c2, c3) = (v * aa / (4.0 * PI * PI), aa * aa / (4.0 * PI * PI * PI));
                            for m in modes + 1..=last {
                                let mf = m as f64;
                                t2 += zm.re * (c2 / (mf * mf) + c3 / (mf * mf * mf));
                                zm = zm * z;
                                if (m & 63) == 0 && zm.re * zm.re + zm.im * zm.im < 1e-30 {
                                    break;
                                }
                            }
                        }
                    }
                    cc_sum += closed - first;
                    t2_sum += t2;
                }
                let ns = sides.len() as f64;
                cconst[i * n + j] = cc_sum / ns;
                cconst[j * n + i] = cc_sum / ns;
                tail2[i * n + j] = t2_sum / ns;
                tail2[j * n + i] = t2_sum / ns;
            }
        }
        RectEnv { a: aa, b: bb, swap, medium, ports, loads, modes, pts, cconst, tail2, cache: Default::default() }
    }

    /// the u and v of a pair; a post's self term is taken at its radius, at y + s·r (s = ±1:
    /// the self term is the mean of both sides, which removes the gradient of the regular part
    /// across the barrel – near a resonance worth up to a few per cent)
    fn uv(pts: &[(f64, f64, f64)], i: usize, j: usize, b: f64, s: f64) -> ([f64; 2], [f64; 3]) {
        let (x, y, r) = pts[i];
        let (xp, mut yp, _) = pts[j];
        if i == j {
            yp = y + s * r;
        }
        let dy = (y - yp).abs();
        ([x - xp, x + xp], [dy, y + yp, 2.0 * b - y - yp])
    }

    /// Green's function matrix between the ports (loads reduced), n×n row-major
    pub fn green(&self, f: f64) -> Vec<Cx> {
        if let Some(g) = self.cache.borrow().get(&f.to_bits()) {
            return g.clone();
        }
        let k2 = self.medium.k2(f);
        let (a, b) = (self.a, self.b);
        let n = self.pts.len();
        let m_n = self.modes;
        let gams: Vec<Cx> = (0..=m_n)
            .map(|m| {
                let al = m as f64 * PI / a;
                let g = (Cx::new(al * al, 0.0) - k2).sqrt();
                if g.re < 0.0 {
                    Cx::new(-g.re, -g.im)
                } else {
                    g
                }
            })
            .collect();
        // 1/(2γ(1 − e^{−2γb})) and e^{−2γb}
        let (pre, q2b): (Vec<Cx>, Vec<Cx>) = gams
            .iter()
            .map(|&g| {
                let e = if g.re * 2.0 * b < 60.0 { g.scale(-2.0 * b).exp() } else { Cx::new(0.0, 0.0) };
                ((g.scale(2.0) * (Cx::new(1.0, 0.0) - e)).inv(), e)
            })
            .collect();
        let mut g = vec![Cx::new(0.0, 0.0); n * n];
        for i in 0..n {
            for j in i..n {
                let (x, y, r) = self.pts[i];
                let (xp, _, _) = self.pts[j];
                let sides: &[f64] = if i == j { &[1.0, -1.0] } else { &[0.0] };
                let mut sum = Cx::new(0.0, 0.0);
                for &sd in sides {
                    let yp = if i == j { y + sd * r } else { self.pts[j].1 };
                    let dy = (y - yp).abs();
                    let (s1, s2) = (y + yp, 2.0 * b - y - yp);
                    let wside = 1.0 / sides.len() as f64;
                    for m in 0..=m_n {
                        let gm = gams[m];
                        let al = m as f64 * PI / a;
                        let e = |v: f64| gm.scale(-v).exp();
                        let mut s = e(dy);
                        if gm.re * s1 < 60.0 {
                            s = s + e(s1);
                        }
                        if gm.re * s2 < 60.0 {
                            s = s + e(s2);
                        }
                        if gm.re * (2.0 * b - dy) < 60.0 {
                            s = s + q2b[m] * gm.scale(dy).exp();
                        }
                        let w = if m == 0 { 1.0 / a } else { 2.0 / a * (al * x).cos() * (al * xp).cos() };
                        sum = sum + (s * pre[m]).scale(w * wside);
                    }
                }
                let v = sum + Cx::new(self.cconst[i * n + j], 0.0) + k2.scale(self.tail2[i * n + j]);
                g[i * n + j] = v;
                g[j * n + i] = v;
            }
        }
        let out = reduce_loads(&g, self.ports.len(), &self.loads, 2.0 * PI * f, self.medium.d);
        let mut c = self.cache.borrow_mut();
        if c.len() > 4096 {
            c.clear();
        }
        c.insert(f.to_bits(), out.clone());
        out
    }

    /// number of modes in the exact part
    pub fn modes(&self) -> usize {
        self.modes
    }
}

// --- any plane shape: grid -------------------------------------------------------------------

/// Peaceman's equivalent radius of a node of the five-point Laplacian on square cells of side h:
/// the grid's node value equals the continuous field of a point source at this distance,
/// h·e^{−γ}/(2√2) ≈ 0.1985·h
pub fn peaceman_radius(h: f64) -> f64 {
    h * (-0.577_215_664_901_532_9f64).exp() / (2.0 * std::f64::consts::SQRT_2)
}

/// A plane pair of any shape on square cells (the region of a [`PairCavity`]): each cell joined
/// to its neighbours by the plane-pair inductance μd per square, each carrying ε·h²/d (with the
/// dielectric loss), the shorting posts of the pair as inductances to the plates (Peaceman's
/// radius against the post radius), loads (capacitors) as their impedance in series with the
/// same local inductance, magnetic walls where the region ends, and on them – if the medium has
/// edges – the radiation conductance of a slot as long as their straight run, per length. One sparse LU per frequency,
/// one solve per port.
///
/// The ports' near field comes from the free-space function: the grid supplies only what the
/// region adds to it (edges, shorts, loads). A port is spread over the four cells around it
/// (bilinear weights w), so the smooth part is taken at its true position, and
///   G_ij = Σ w_a w_b G_grid(a, b) + G0(|r_i − r_j|) − Σ w_a w_b G0_lattice(a − b),
/// G0 = [`green_free`], G0_lattice the free-space function of the five-point lattice: G0 at
/// Peaceman's radius for the same cell, G0 at the cells' distance plus the static difference
/// between lattice and continuum (exact lattice values, half the resistance between nodes of an
/// infinite square resistor lattice) for offsets up to three cells, G0 beyond; a port's self term
/// uses its radius. Ports closer than a cell are as accurate as the
/// free-space ladder, and the region acts on them through its smooth reflected field.
pub struct GridEnv {
    pub medium: PairMedium,
    pub h: f64,
    pub ports: Vec<EnvPort>,
    /// every port spread over the cells around it: (unknown, weight, cell column, cell row)
    port_cells: Vec<Vec<(usize, f64, i64, i64)>>,
    /// the grid's system matrix (everything but the ports)
    sys: GridSys,
    sym: std::cell::RefCell<Option<kisi_sparse::SymbolicLu>>,
    cache: std::cell::RefCell<std::collections::HashMap<u64, Vec<Cx>>>,
    /// sample on a grid of this step (Hz) and interpolate linearly (an open window without
    /// high-Q resonances); None: exact at every frequency
    pub df: Option<f64>,
    /// the group of environments with the same system matrix (`share`) and this one's index in it
    group: Option<(std::sync::Arc<GridGroup>, usize)>,
    /// the system's sparsity pattern (the same at every frequency)
    pattern: std::sync::OnceLock<Option<kisi_sparse::ComplexPattern>>,
}

/// The system matrix of a grid environment, without its ports: environments of different vias
/// in the same region share it (`GridEnv::share`).
#[derive(Debug, Clone)]
struct GridSys {
    medium: PairMedium,
    h: f64,
    n: usize,
    branches: Vec<(usize, usize)>,
    /// per unknown: shorting inductance to the plates (H, parallel of the posts in the cell)
    shorts: Vec<(usize, f64)>,
    loads: Vec<(usize, Load, f64)>,
    /// boundary faces for the edge radiation: (unknown, face length m, run length m)
    edge_faces: Vec<(usize, f64, f64)>,
    /// faces where a window cut the region, matched to the outgoing wave of the window's centre:
    /// (unknown, face length m, cosine between the face normal and the radius, radius m)
    cut_faces: Vec<(usize, f64, f64, f64)>,
}

impl GridSys {
    /// the node equations at `f` as triplets (row, column, re, im)
    fn triplets(&self, f: f64) -> Vec<(usize, usize, f64, f64)> {
        let w = 2.0 * PI * f;
        let d = self.medium.d;
        let k2 = self.medium.k2_bulk(f);
        let jwmd = Cx::new(0.0, w * MU0 * d);
        // node equation (per cell): Σ (V_i − V_j)/(jωμd) − k² h²/(jωμd) V_i + Y_load V_i = I_i
        let yb = jwmd.inv();
        let yc = (Cx::new(0.0, 0.0) - k2.scale(self.h * self.h)) * yb;
        let mut trip: Vec<(usize, usize, f64, f64)> =
            Vec::with_capacity(self.n + 4 * self.branches.len() + self.shorts.len() + self.loads.len());
        for i in 0..self.n {
            trip.push((i, i, yc.re, yc.im));
        }
        for &(a, b) in &self.branches {
            trip.push((a, a, yb.re, yb.im));
            trip.push((b, b, yb.re, yb.im));
            trip.push((a, b, -yb.re, -yb.im));
            trip.push((b, a, -yb.re, -yb.im));
        }
        for &(c, l) in &self.shorts {
            let y = Cx::new(0.0, w * l).inv();
            trip.push((c, c, y.re, y.im));
        }
        for (c, ld, l) in &self.loads {
            let y = (ld.z(w) + Cx::new(0.0, w * l)).inv();
            trip.push((*c, *c, y.re, y.im));
        }
        // edge radiation: the slot conductance of each straight run, per length of its faces
        let k0 = w / crate::num::C0;
        let mut gcache: Vec<(f64, f64)> = vec![];
        for &(c, hf, run) in &self.edge_faces {
            let g = match gcache.iter().find(|x| (x.0 - run).abs() < 1e-12) {
                Some(x) => x.1,
                None => {
                    let g = slot_conductance(run, k0) / run;
                    gcache.push((run, g));
                    g
                }
            };
            trip.push((c, c, g * hf, 0.0));
        }
        if !self.cut_faces.is_empty() {
            let mut k = k2.sqrt();
            if k.im > 0.0 {
                k = Cx::new(-k.re, -k.im);
            }
            let y0 = k.scale(1.0 / (w * MU0 * d));
            for &(c, hf, cs, rho) in &self.cut_faces {
                let y = y0 * (Cx::new(1.0, 0.0) - Cx::new(0.0, 0.5) / k.scale(rho));
                trip.push((c, c, y.re * hf * cs, y.im * hf * cs));
            }
        }
        trip
    }

    /// LU of the system at `f`, reusing and returning the symbolic analysis (counted in
    /// `lu_stats`)
    fn lu(
        &self,
        f: f64,
        sym: Option<kisi_sparse::SymbolicLu>,
        pattern: &std::sync::OnceLock<Option<kisi_sparse::ComplexPattern>>,
    ) -> Result<(kisi_sparse::ComplexLu, kisi_sparse::SymbolicLu), i32> {
        let trip = self.triplets(f);
        #[cfg(not(target_arch = "wasm32"))]
        let t_lu = std::time::Instant::now();
        // the triplets' positions are the same at every frequency: their pattern once
        let pat = pattern.get_or_init(|| {
            kisi_sparse::ComplexPattern::new(self.n, &trip.iter().map(|t| (t.0, t.1)).collect::<Vec<_>>()).ok()
        });
        let lu = match pat {
            Some(p) => p.lu(&trip.iter().map(|t| (t.2, t.3)).collect::<Vec<_>>(), sym),
            None => kisi_sparse::ComplexLu::with_symbolic(self.n, &trip, sym),
        };
        #[cfg(not(target_arch = "wasm32"))]
        LU_STATS.with(|s| {
            let mut s = s.borrow_mut();
            s.0 += 1;
            s.1 += t_lu.elapsed().as_secs_f64();
            s.2 += self.n;
        });
        lu
    }
}

/// The ports' responses: zg[i·np + j] = Σ over the cells of port i of the solution for a unit
/// current spread over the cells of port j (None when the solve fails)
fn port_responses(lu: &kisi_sparse::ComplexLu, n: usize, cells: &[Vec<(usize, f64)>]) -> Option<Vec<Cx>> {
    let np = cells.len();
    let rhs: Vec<Vec<(f64, f64)>> = (0..np)
        .map(|j| {
            let mut b = vec![(0.0, 0.0); n];
            for &(c, w) in &cells[j] {
                b[c].0 += w;
            }
            b
        })
        .collect();
    let x = lu.solve(&rhs).ok()?;
    let mut zg = vec![Cx::new(0.0, 0.0); np * np];
    for i in 0..np {
        for j in 0..np {
            let mut z = Cx::new(0.0, 0.0);
            for &(ca, wa) in &cells[i] {
                let v = x[j][ca];
                z = z + Cx::new(v.0, v.1).scale(wa);
            }
            zg[i * np + j] = z;
        }
    }
    Some(zg)
}

/// Grid environments with the same system matrix – the same region, posts and loads, crossed by
/// different vias, which differ only in their ports: one factorisation per frequency serves them
/// all. An environment registers its ports when it joins (`GridEnv::share`); at a frequency not
/// computed yet the group factorises once and solves for every registered environment that
/// lacks it (each in a solve of its own, so a result does not depend on which others were
/// registered). `CAVITY_REGISTER_ONLY` lets a first pass over a check register them all.
struct GridGroup {
    sys: GridSys,
    pattern: std::sync::OnceLock<Option<kisi_sparse::ComplexPattern>>,
    state: std::sync::Mutex<GroupState>,
}

#[derive(Default)]
struct GroupState {
    sym: Option<kisi_sparse::SymbolicLu>,
    /// the distinct ports of the registered environments (cells and weights) and their index
    ports: Vec<Vec<(usize, f64)>>,
    port_index: std::collections::HashMap<String, usize>,
    /// every registered environment as its ports
    sets: Vec<Vec<usize>>,
    set_index: std::collections::HashMap<Vec<usize>, usize>,
    done: std::collections::HashMap<(usize, u64), Option<Vec<Cx>>>,
}

impl GridGroup {
    fn register(&self, cells: Vec<Vec<(usize, f64)>>) -> usize {
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let ids: Vec<usize> = cells
            .into_iter()
            .map(|c| {
                let key = format!("{c:?}");
                if let Some(&i) = st.port_index.get(&key) {
                    return i;
                }
                st.ports.push(c);
                let i = st.ports.len() - 1;
                st.port_index.insert(key, i);
                i
            })
            .collect();
        if let Some(&i) = st.set_index.get(&ids) {
            return i;
        }
        st.sets.push(ids.clone());
        let i = st.sets.len() - 1;
        st.set_index.insert(ids, i);
        i
    }

    /// the port responses (`port_responses`) of environment `id` at `f`: one factorisation per
    /// frequency, one solve per distinct port for every registered environment that lacks `f`
    /// (vias of one region share return vias), each port in a solve of its own
    fn responses(&self, id: usize, f: f64) -> Option<Vec<Cx>> {
        let fb = f.to_bits();
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(r) = st.done.get(&(id, fb)) {
            return r.clone();
        }
        let sym = st.sym.take();
        let lu = self.sys.lu(f, sym, &self.pattern);
        let todo: Vec<usize> = (0..st.sets.len()).filter(|&s| !st.done.contains_key(&(s, fb))).collect();
        let lu = match lu {
            Ok((lu, sym)) => {
                st.sym = Some(sym);
                lu
            }
            Err(_) => {
                for s in todo {
                    st.done.insert((s, fb), None);
                }
                return None;
            }
        };
        let n = self.sys.n;
        let mut need: Vec<usize> = todo.iter().flat_map(|&s| st.sets[s].iter().copied()).collect();
        need.sort_unstable();
        need.dedup();
        let mut x: std::collections::HashMap<usize, Option<Vec<(f64, f64)>>> = Default::default();
        for &p in &need {
            let mut b = vec![(0.0, 0.0); n];
            for &(c, w) in &st.ports[p] {
                b[c].0 += w;
            }
            x.insert(p, lu.solve(&[b]).ok().and_then(|mut v| v.pop()));
        }
        for s in todo {
            let set = &st.sets[s];
            let np = set.len();
            let mut zg = Some(vec![Cx::new(0.0, 0.0); np * np]);
            for j in 0..np {
                let Some(xj) = x[&set[j]].as_ref() else {
                    zg = None;
                    break;
                };
                let z = zg.as_mut().unwrap();
                for i in 0..np {
                    let mut v = Cx::new(0.0, 0.0);
                    for &(ca, wa) in &st.ports[set[i]] {
                        let q = xj[ca];
                        v = v + Cx::new(q.0, q.1).scale(wa);
                    }
                    z[i * np + j] = v;
                }
            }
            st.done.insert((s, fb), zg);
        }
        st.done.get(&(id, fb)).cloned().flatten()
    }
}

/// the groups of grid environments by system matrix (process-wide: the threads of a parallel
/// check share them)
static GROUPS: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<String, std::sync::Arc<GridGroup>>>> =
    std::sync::LazyLock::new(Default::default);

/// the overlap of a plane pair's two planes on its grid (`pair_region`), by layers, nets, box and cell
static OVERLAPS: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<String, std::sync::Arc<Mask>>>> =
    std::sync::LazyLock::new(Default::default);

/// a first pass that only registers the grid environments of a check (`set_register_only`)
static REGISTER_ONLY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// While on, grid environments answer zero without computing (and keep nothing): a pass over a
/// check with it registers every environment in its group, so the real pass factorises each
/// region once per frequency for all vias that cross it. The results of that pass are wrong and
/// must be dropped.
pub fn set_register_only(on: bool) {
    REGISTER_ONLY.store(on, std::sync::atomic::Ordering::SeqCst);
}

/// whether the registering pass is on (`set_register_only`)
pub fn register_only() -> bool {
    REGISTER_ONLY.load(std::sync::atomic::Ordering::Relaxed)
}

impl GridEnv {
    /// Region from a mask (mm), shorting posts and loads (positions in mm, radius in mm; loads
    /// with their impedance), ports (mm). Posts on a port's position (within 1 µm) are left out
    /// (the port is that post).
    pub fn new(mask: &Mask, medium: PairMedium, shorts: &[Short], loads: &[Load], ports: &[EnvPort]) -> GridEnv {
        Self::new_cut(mask, medium, shorts, loads, ports, [false; 4])
    }

    /// As [`new`](Self::new), the region cut by a window on the raster sides `cut` (x0, x1, y0,
    /// y1): the plane pair goes on there, so these faces carry the admittance of the cylindrical
    /// wave leaving the window's centre (the first port), per length
    /// k/(ωμd)·(1 − j/(2kρ))·cos θ (the large-argument ratio H1⁽²⁾/H0⁽²⁾ ≈ j + 1/(2kρ), θ between the face
    /// normal and the radius).
    pub fn new_cut(
        mask: &Mask,
        medium: PairMedium,
        shorts: &[Short],
        loads: &[Load],
        ports: &[EnvPort],
        cut: [bool; 4],
    ) -> GridEnv {
        let (nx, ny, h) = (mask.nx, mask.ny, mask.h);
        let mut id = vec![usize::MAX; nx * ny];
        let mut n = 0;
        for k in 0..nx * ny {
            if mask.on[k] {
                id[k] = n;
                n += 1;
            }
        }
        let cell_of = |x: f64, y: f64| -> usize {
            let (i, j) = mask.cell_of([x, y]);
            let k = j * nx + i;
            if id[k] != usize::MAX {
                return id[k];
            }
            let (i, j) = mask.nearest_on([x, y], 50.0).unwrap_or((i, j));
            id[j * nx + i]
        };
        // a port over the four cells around it (bilinear), else the cell it lies in or the nearest
        let spread = |x: f64, y: f64| -> Vec<(usize, f64, i64, i64)> {
            let (u, v) = ((x - mask.x0) / h - 0.5, (y - mask.y0) / h - 0.5);
            let (i0, j0) = (u.floor() as i64, v.floor() as i64);
            let (tu, tv) = (u - i0 as f64, v - j0 as f64);
            let mut out = vec![];
            for (di, dj, w) in
                [(0, 0, (1.0 - tu) * (1.0 - tv)), (1, 0, tu * (1.0 - tv)), (0, 1, (1.0 - tu) * tv), (1, 1, tu * tv)]
            {
                let (i, j) = (i0 + di, j0 + dj);
                if i < 0 || j < 0 || i >= nx as i64 || j >= ny as i64 || id[j as usize * nx + i as usize] == usize::MAX
                {
                    out.clear();
                    break;
                }
                if w > 1e-12 {
                    out.push((id[j as usize * nx + i as usize], w, i, j));
                }
            }
            if out.is_empty() {
                let c = cell_of(x, y);
                let k = id.iter().position(|&q| q == c).unwrap_or(0);
                out.push((c, 1.0, (k % nx) as i64, (k / nx) as i64));
            }
            out
        };
        let mut branches = vec![];
        for j in 0..ny {
            for i in 0..nx {
                let a = id[j * nx + i];
                if a == usize::MAX {
                    continue;
                }
                if i + 1 < nx && id[j * nx + i + 1] != usize::MAX {
                    branches.push((a, id[j * nx + i + 1]));
                }
                if j + 1 < ny && id[(j + 1) * nx + i] != usize::MAX {
                    branches.push((a, id[(j + 1) * nx + i]));
                }
            }
        }
        let req = peaceman_radius(h);
        let dloc = |r: f64| MU0 * medium.d / (2.0 * PI) * (req / r.max(1e-4)).ln().max(0.05);
        let is_port = |x: f64, y: f64| ports.iter().any(|p| (p.x - x).abs() < 1e-3 && (p.y - y).abs() < 1e-3);
        let mut sh: std::collections::HashMap<usize, f64> = Default::default();
        for s in shorts {
            if is_port(s.at[0], s.at[1]) {
                continue;
            }
            let c = cell_of(s.at[0], s.at[1]);
            // parallel inductances: 1/L = Σ 1/L_k
            *sh.entry(c).or_insert(0.0) += 1.0 / dloc(s.r);
        }
        // by cell: the order of the triplets decides the order of the sums in the matrix (a hash
        // map's order differed from run to run in the last digits)
        let mut shorts_v: Vec<(usize, f64)> = sh.into_iter().map(|(c, y)| (c, 1.0 / y)).collect();
        shorts_v.sort_by_key(|x| x.0);
        let loads_v: Vec<(usize, Load, f64)> =
            loads.iter().map(|l| (cell_of(l.at.x, l.at.y), *l, dloc(l.at.r))).collect();
        let port_cells: Vec<Vec<(usize, f64, i64, i64)>> = ports.iter().map(|p| spread(p.x, p.y)).collect();
        // the edge radiation (when the medium asks for it) on the boundary faces
        let faces = mask.edge_faces();
        let is_cut = |b: Option<u8>| b.is_some_and(|b| cut[b as usize]);
        let edge_faces: Vec<(usize, f64, f64)> = if medium.area > 0.0 && !medium.edges.is_empty() {
            faces.iter().filter(|f| !is_cut(f.3)).map(|&(i, j, l, _)| (id[j * nx + i], h * 1e-3, l * 1e-3)).collect()
        } else {
            vec![]
        };
        let centre = ports.first().map_or([mask.x0, mask.y0], |p| [p.x, p.y]);
        let cut_faces: Vec<(usize, f64, f64, f64)> = faces
            .iter()
            .filter(|f| is_cut(f.3))
            .map(|&(i, j, _, b)| {
                let c = mask.centre(i, j);
                let (fc, nrm) = match b.unwrap() {
                    0 => ([c[0] - 0.5 * h, c[1]], [-1.0, 0.0]),
                    1 => ([c[0] + 0.5 * h, c[1]], [1.0, 0.0]),
                    2 => ([c[0], c[1] - 0.5 * h], [0.0, -1.0]),
                    _ => ([c[0], c[1] + 0.5 * h], [0.0, 1.0]),
                };
                let (dx, dy) = (fc[0] - centre[0], fc[1] - centre[1]);
                let rho = dx.hypot(dy).max(h);
                (id[j * nx + i], h * 1e-3, ((dx * nrm[0] + dy * nrm[1]) / rho).max(0.0), rho * 1e-3)
            })
            .collect();
        // ports in metres from here on
        let ports_m: Vec<EnvPort> =
            ports.iter().map(|p| EnvPort { x: p.x * 1e-3, y: p.y * 1e-3, r: p.r * 1e-3 }).collect();
        let sys = GridSys {
            medium: medium.clone(),
            h: h * 1e-3,
            n,
            branches,
            shorts: shorts_v,
            loads: loads_v,
            edge_faces,
            cut_faces,
        };
        GridEnv {
            medium,
            h: h * 1e-3,
            ports: ports_m,
            port_cells,
            sys,
            sym: Default::default(),
            cache: Default::default(),
            df: None,
            group: None,
            pattern: Default::default(),
        }
    }

    /// number of unknowns
    pub fn cells(&self) -> usize {
        self.sys.n
    }

    /// Join the group of environments with the same system matrix (the same region, posts and
    /// loads; `GridGroup`): one factorisation per frequency for all of them. The results are the
    /// same as without.
    pub fn share(&mut self) {
        let key = format!("{:?}", self.sys);
        let group = {
            let mut g = GROUPS.lock().unwrap_or_else(|e| e.into_inner());
            if g.len() > 512 {
                g.clear();
            }
            g.entry(key)
                .or_insert_with(|| {
                    std::sync::Arc::new(GridGroup {
                        sys: self.sys.clone(),
                        pattern: Default::default(),
                        state: Default::default(),
                    })
                })
                .clone()
        };
        let id = group.register(self.port_cells.iter().map(|p| p.iter().map(|c| (c.0, c.1)).collect()).collect());
        self.group = Some((group, id));
    }

    /// free-space function of the five-point lattice between cells `di`, `dj` apart (k² of the
    /// medium): G0(Peaceman radius) − ½ R(di, dj), R the resistance between nodes of an infinite
    /// lattice of unit resistors (exact values up to three cells), G0 at the distance beyond
    fn g0_lattice(&self, k2: Cx, di: i64, dj: i64) -> Cx {
        let (a, b) = (di.unsigned_abs().max(dj.unsigned_abs()), di.unsigned_abs().min(dj.unsigned_abs()));
        let r = match (a, b) {
            (0, 0) => Some(0.0),
            (1, 0) => Some(0.5),
            (1, 1) => Some(2.0 / PI),
            (2, 0) => Some(2.0 - 4.0 / PI),
            (2, 1) => Some(4.0 / PI - 0.5),
            (2, 2) => Some(8.0 / (3.0 * PI)),
            (3, 0) => Some(8.5 - 24.0 / PI),
            (3, 1) => Some(46.0 / (3.0 * PI) - 4.0),
            (3, 2) => Some(0.5 + 4.0 / (3.0 * PI)),
            (3, 3) => Some(46.0 / (15.0 * PI)),
            _ => None,
        };
        let req = peaceman_radius(self.h);
        let rho = self.h * ((di * di + dj * dj) as f64).sqrt();
        match r {
            Some(_) if a == 0 => green_free(k2, req),
            // the continuous function at the distance (with its k-dependent part, e.g. the
            // radiation term −J0(kρ)/4) plus the static difference lattice − continuum,
            // (1/2π) ln(ρ/r_eq) − R/2
            Some(r) => green_free(k2, rho) + Cx::new((rho / req).ln() / (2.0 * PI) - 0.5 * r, 0.0),
            None => green_free(k2, rho),
        }
    }

    /// Green's function matrix between the ports, n×n row-major
    pub fn green(&self, f: f64) -> Vec<Cx> {
        match self.df {
            Some(df) if f > df => {
                let x = f / df;
                let k = x.floor();
                let t = x - k;
                let a = self.green_exact(k * df);
                if t < 1e-9 {
                    return a;
                }
                let b = self.green_exact((k + 1.0) * df);
                a.iter().zip(&b).map(|(p, q)| p.scale(1.0 - t) + q.scale(t)).collect()
            }
            _ => self.green_exact(f),
        }
    }

    fn green_exact(&self, f: f64) -> Vec<Cx> {
        if let Some(g) = self.cache.borrow().get(&f.to_bits()) {
            return g.clone();
        }
        let np = self.ports.len();
        let w = 2.0 * PI * f;
        let k2 = self.medium.k2_bulk(f);
        let jwmd = Cx::new(0.0, w * MU0 * self.medium.d);
        let rho = |i: usize, j: usize| {
            let (pi, pj) = (self.ports[i], self.ports[j]);
            if i == j {
                pi.r
            } else {
                (pi.x - pj.x).hypot(pi.y - pj.y).max(1e-7)
            }
        };
        if REGISTER_ONLY.load(std::sync::atomic::Ordering::Relaxed) {
            // a first pass that only registers: the infinite plates' function (cheap, regular)
            return (0..np * np).map(|q| green_free(k2, rho(q / np, q % np))).collect();
        }
        let zg = match &self.group {
            Some((g, id)) => g.responses(*id, f),
            None => {
                let sym = self.sym.borrow_mut().take();
                match self.sys.lu(f, sym, &self.pattern) {
                    Ok((lu, sym)) => {
                        *self.sym.borrow_mut() = Some(sym);
                        let cells: Vec<Vec<(usize, f64)>> =
                            self.port_cells.iter().map(|p| p.iter().map(|c| (c.0, c.1)).collect()).collect();
                        port_responses(&lu, self.sys.n, &cells)
                    }
                    Err(_) => None,
                }
            }
        };
        let mut out = vec![Cx::new(0.0, 0.0); np * np];
        if let Some(zg) = zg {
            // the lattice function depends on the cell offset only: once per offset
            let mut g0: std::collections::HashMap<(u64, u64), Cx> = Default::default();
            for i in 0..np {
                for j in 0..np {
                    let mut lat = Cx::new(0.0, 0.0);
                    for &(_, wa, ia, ja) in &self.port_cells[i] {
                        for &(_, wb, ib, jb) in &self.port_cells[j] {
                            let (di, dj) = ((ia - ib).unsigned_abs(), (ja - jb).unsigned_abs());
                            let g = *g0
                                .entry((di.max(dj), di.min(dj)))
                                .or_insert_with(|| self.g0_lattice(k2, ia - ib, ja - jb));
                            lat = lat + g.scale(wa * wb);
                        }
                    }
                    out[i * np + j] = zg[i * np + j] / jwmd + green_free(k2, rho(i, j)) - lat;
                }
            }
        }
        let mut c = self.cache.borrow_mut();
        if c.len() > 4096 {
            c.clear();
        }
        c.insert(f.to_bits(), out.clone());
        out
    }
}

thread_local! {
    /// grid factorisations so far: count, seconds, cells (for run-time reports)
    static LU_STATS: std::cell::RefCell<(usize, f64, usize)> = const { std::cell::RefCell::new((0, 0.0, 0)) };
}

/// Groups of shared grid environments: (groups, environments in them).
pub fn group_stats() -> (usize, usize) {
    let g = GROUPS.lock().unwrap_or_else(|e| e.into_inner());
    (g.len(), g.values().map(|x| x.state.lock().map(|s| s.sets.len()).unwrap_or(0)).sum())
}

/// Grid factorisations so far on this thread: (count, seconds, total cells).
pub fn lu_stats() -> (usize, f64, usize) {
    LU_STATS.with(|s| *s.borrow())
}

/// A cavity environment of a plane pair.
pub enum PairEnv {
    Rect(RectEnv),
    Grid(GridEnv),
}

impl PairEnv {
    pub fn ports(&self) -> &[EnvPort] {
        match self {
            PairEnv::Rect(e) => &e.ports,
            PairEnv::Grid(e) => &e.ports,
        }
    }
    pub fn medium(&self) -> &PairMedium {
        match self {
            PairEnv::Rect(e) => &e.medium,
            PairEnv::Grid(e) => &e.medium,
        }
    }
    /// Green's function matrix between the ports (n×n): Z_ij = jωμd·G_ij
    pub fn green(&self, f: f64) -> Vec<Cx> {
        match self {
            PairEnv::Rect(e) => e.green(f),
            PairEnv::Grid(e) => e.green(f),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn med(er: f64, tand: f64) -> PairMedium {
        PairMedium { d: 0.2e-3, er, tand, f_loss: None, sigma: None, area: 0.0, edges: vec![] }
    }

    /// double cosine series between two separate points (converges with enough modes)
    fn double_series(a: f64, b: f64, k2: Cx, p: (f64, f64), q: (f64, f64)) -> Cx {
        let mut d = Cx::new(0.0, 0.0);
        for m in 0..600 {
            for n in 0..400 {
                let (kx, ky) = (m as f64 * PI / a, n as f64 * PI / b);
                let chi = if m == 0 { 1.0 } else { 2.0 } * if n == 0 { 1.0 } else { 2.0 };
                let num = chi * (kx * p.0).cos() * (kx * q.0).cos() * (ky * p.1).cos() * (ky * q.1).cos();
                d = d + (Cx::new(kx * kx + ky * ky, 0.0) - k2).inv().scale(num);
            }
        }
        d.scale(1.0 / (a * b))
    }

    #[test]
    fn rect_green_matches_double_series() {
        for (a, b) in [(0.05, 0.03), (0.03, 0.05)] {
            let m = med(4.3, 0.02);
            let ports = vec![EnvPort { x: 0.011, y: 0.007, r: 1e-4 }, EnvPort { x: 0.021, y: 0.019, r: 1e-4 }];
            let e = RectEnv::new(a, b, m.clone(), ports.clone(), vec![], 10e9);
            let f = 3.1e9;
            let g = e.green(f)[1];
            let d = double_series(a, b, m.k2(f), (ports[0].x, ports[0].y), (ports[1].x, ports[1].y));
            assert!((g - d).abs() / d.abs() < 2e-3, "{a} {b}: {g:?} {d:?}");
        }
    }

    /// a large lossy cavity approaches the free-space function, also for a post's self term
    #[test]
    fn rect_green_large_cavity_is_free_space() {
        let m = med(4.0, 0.1);
        let f = 20e9;
        let k2 = m.k2(f);
        let ports = vec![
            EnvPort { x: 0.3, y: 0.25, r: 0.15e-3 },
            EnvPort { x: 0.301, y: 0.25, r: 0.15e-3 },
            EnvPort { x: 0.302, y: 0.252, r: 0.1e-3 },
        ];
        let e = RectEnv::new(0.6, 0.5, m, ports.clone(), vec![], f);
        let g = e.green(f);
        for (i, j) in [(0, 0), (0, 1), (0, 2), (2, 2)] {
            let rho = if i == j { ports[i].r } else { (ports[i].x - ports[j].x).hypot(ports[i].y - ports[j].y) };
            let gf = green_free(k2, rho);
            let v = g[i * 3 + j];
            assert!((v - gf).abs() / gf.abs() < 1e-3, "{i}{j}: {v:?} {gf:?}");
        }
    }

    /// low frequency: the cavity is a capacitor, Z = 1/(jωC), C = ε a b / d
    #[test]
    fn rect_green_low_frequency_capacitance() {
        let m = PairMedium { d: 0.2e-3, er: 4.5, tand: 0.0, f_loss: None, sigma: None, area: 0.0, edges: vec![] };
        let e = RectEnv::new(0.04, 0.02, m.clone(), vec![EnvPort { x: 0.01, y: 0.005, r: 2e-4 }], vec![], 1e9);
        let f = 1e6;
        let z = Cx::new(0.0, 2.0 * PI * f * MU0 * m.d) * e.green(f)[0];
        let cpl = EPS0 * 4.5 * 0.04 * 0.02 / m.d;
        let cz = -1.0 / (2.0 * PI * f * z.im);
        assert!((cz / cpl - 1.0).abs() < 0.01, "{cz} {cpl}");
    }

    /// a shorting post as a load: the same as reducing it as a port with Z = 0
    #[test]
    fn rect_short_load_reduces() {
        let m = med(4.3, 0.02);
        let p = vec![EnvPort { x: 0.01, y: 0.01, r: 1.5e-4 }];
        let s = EnvPort { x: 0.011, y: 0.01, r: 1.5e-4 };
        let e1 = RectEnv::new(0.03, 0.02, m.clone(), p.clone(), vec![Load { at: s, r_ohm: 0.0, l: 0.0, c: None }], 5e9);
        let e2 = RectEnv::new(0.03, 0.02, m, vec![p[0], s], vec![], 5e9);
        let f = 4e9;
        let g2 = e2.green(f);
        let red = g2[0] - g2[1] * g2[2] / g2[3];
        let g1 = e1.green(f)[0];
        assert!((g1 - red).abs() / red.abs() < 1e-9, "{g1:?} {red:?}");
    }

    #[test]
    fn slot_conductance_limits_and_edges() {
        let lam = 0.03;
        let k0 = 2.0 * PI / lam;
        // short edge (w/λ)²/90, long edge w/(120λ)
        let gs = 2.0 * slot_conductance(0.002, k0);
        assert!((gs / ((0.002f64 / lam).powi(2) / 90.0) - 1.0).abs() < 0.02, "{gs}");
        let gl = 2.0 * slot_conductance(0.6, k0);
        assert!((gl / (0.6 / (120.0 * lam)) - 1.0).abs() < 0.05, "{gl}");
        // an L-shaped region has six straight edges with the right lengths
        let bb = BBox { min: [0.0, 0.0], max: [24.0, 16.0] };
        let mut m = Mask::new(&bb, 0.5);
        m.fill_rings(&[&[[0.0, 0.0], [24.0, 0.0], [24.0, 10.0], [16.0, 10.0], [16.0, 16.0], [0.0, 16.0]][..]]);
        let mut e = m.edge_runs();
        e.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert_eq!(e, vec![6.0, 8.0, 10.0, 16.0, 16.0, 24.0]);
    }

    /// a window in an infinite plane pair, its cut sides matched: close to the free-space function
    /// (the edges reflect a little at oblique incidence; lossy medium)
    #[test]
    fn grid_window_is_open() {
        let m = PairMedium { d: 0.2e-3, er: 4.3, tand: 0.05, f_loss: None, sigma: None, area: 0.0, edges: vec![] };
        let bb = BBox { min: [-8.0, -8.0], max: [8.0, 8.0] };
        let mut mask = Mask::new(&bb, 0.25);
        mask.on.iter_mut().for_each(|x| *x = true);
        let ports = vec![EnvPort { x: 0.1, y: 0.05, r: 0.15 }, EnvPort { x: 1.1, y: 0.05, r: 0.15 }];
        let ge = GridEnv::new_cut(&mask, m.clone(), &[], &[], &ports, [true; 4]);
        for f in [5e9, 15e9] {
            let g = ge.green(f);
            let k2 = m.k2(f);
            for (q, rho) in [(0, 0.15e-3), (1, 1e-3)] {
                let gf = green_free(k2, rho);
                let e = (g[q] - gf).abs() / gf.abs();
                assert!(e < 0.05, "f {f}: {q}: {:?} free {:?} ({e})", g[q], gf);
            }
        }
    }

    /// environments with the same system matrix share its factorisations and give the same
    /// results as on their own
    #[test]
    fn shared_grid_groups() {
        let m = PairMedium { d: 0.3e-3, er: 4.2, tand: 0.02, f_loss: None, sigma: None, area: 0.0, edges: vec![] };
        let bb = BBox { min: [0.0, 0.0], max: [11.0, 7.0] };
        let mut mask = Mask::new(&bb, 0.5);
        mask.on.iter_mut().for_each(|x| *x = true);
        let loads = [Load { at: EnvPort { x: 5.0, y: 5.0, r: 0.15 }, r_ohm: 0.02, l: 0.6e-9, c: Some(100e-9) }];
        let pa = vec![EnvPort { x: 2.1, y: 2.2, r: 0.1 }, EnvPort { x: 3.0, y: 2.2, r: 0.1 }];
        let pb = vec![
            EnvPort { x: 7.3, y: 4.1, r: 0.1 },
            EnvPort { x: 3.0, y: 2.2, r: 0.1 },
            EnvPort { x: 8.0, y: 4.4, r: 0.1 },
        ];
        let own: Vec<GridEnv> = [&pa, &pb].iter().map(|p| GridEnv::new(&mask, m.clone(), &[], &loads, p)).collect();
        let shared: Vec<GridEnv> = [&pa, &pb]
            .iter()
            .map(|p| {
                let mut e = GridEnv::new(&mask, m.clone(), &[], &loads, p);
                e.share();
                e
            })
            .collect();
        let g = &shared[0].group.as_ref().unwrap().0;
        assert!(std::sync::Arc::ptr_eq(g, &shared[1].group.as_ref().unwrap().0));
        // the port at (3.0, 2.2) is solved once for both
        assert_eq!(g.state.lock().unwrap().ports.len(), 4);
        for f in [0.7e9, 4.1e9] {
            for (a, b) in own.iter().zip(&shared) {
                assert_eq!(a.green(f), b.green(f));
            }
        }
        // (the registering pass, a process-wide switch, is checked on the boards: a check gives
        // the same rows with and without it, BOARD_RESONANCES.md section 9)
    }

    #[test]
    fn decap_parasitics_from_data() {
        // data/components.json: 0402 body ESL 0.35 nH + 0.4 nH mounting; ESR 25 mΩ at 100 nF
        let (r, l) = DecapData::builtin().rl("0402", 100e-9);
        assert!((l - 0.75e-9).abs() < 1e-12, "{l}");
        assert!((r - 0.025).abs() < 1e-9, "{r}");
        let (r, l) = DecapData::builtin().rl("unknown", 10e-6);
        assert!(l > 1.5e-9 && r < 0.025);
    }

    #[test]
    fn cap_values() {
        for (s, v) in [
            ("100n", 100e-9),
            ("0.1uF", 0.1e-6),
            ("4u7", 4.7e-6),
            ("10µF/6.3V", 10e-6),
            ("1p5", 1.5e-12),
            ("2.2nF 50V", 2.2e-9),
        ] {
            let x = parse_cap_value(s).unwrap();
            assert!((x / v - 1.0).abs() < 1e-9, "{s}: {x}");
        }
        assert!(parse_cap_value("DNP").is_none());
    }

    /// the grid of a rectangle against the series: self and transfer terms
    #[test]
    fn grid_matches_rect() {
        let m = med(4.3, 0.02);
        let (a, b) = (24.0, 16.0); // mm
        let bb = BBox { min: [0.0, 0.0], max: [a, b] };
        let mut mask = Mask::new(&bb, 0.25);
        mask.on.iter_mut().for_each(|x| *x = true);
        let ports_mm = vec![
            EnvPort { x: 8.7, y: 6.1, r: 0.15 },
            EnvPort { x: 8.7, y: 8.1, r: 0.15 },
            EnvPort { x: 17.2, y: 11.1, r: 0.15 },
        ];
        let ge = GridEnv::new(&mask, m.clone(), &[], &[], &ports_mm);
        let pm: Vec<EnvPort> =
            ports_mm.iter().map(|p| EnvPort { x: p.x * 1e-3, y: p.y * 1e-3, r: p.r * 1e-3 }).collect();
        let re = RectEnv::new(a * 1e-3, b * 1e-3, m, pm, vec![], 10e9);
        for f in [1e9, 3.3e9, 7e9] {
            let (g1, g2) = (ge.green(f), re.green(f));
            for q in [0, 1, 2, 4, 5] {
                let e = (g1[q] - g2[q]).abs() / g2[q].abs();
                assert!(e < 0.01, "f {f}: entry {q}: grid {:?} series {:?}", g1[q], g2[q]);
            }
        }
    }
}

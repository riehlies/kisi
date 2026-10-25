//! Cross-section extraction along every track and impedance evaluation.
//!
//! For each track we sample points along its length. At every sample we look
//! perpendicular to the track and up/down through the stackup to build a
//! cross-section description (reference planes, coplanar ground, diff-pair
//! partner). Identical cross-sections are deduplicated and evaluated once –
//! the same cache will feed the 2D field solver in M2.

use crate::analytic;
use crate::board::Board;
use crate::geom::{self, P};
use crate::xsgeom::{self, PlaneCut, XsDesc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// --- flags -------------------------------------------------------------------
pub const F_NO_REF: u32 = 1; // no reference plane found at all
pub const F_PARTIAL_REF: u32 = 2; // reference plane ends close to the trace
pub const F_REF_SKIPS_LAYER: u32 = 4; // nearest plane is not on the adjacent layer
pub const F_ONE_SIDED: u32 = 8; // inner layer with only one reference plane
pub const F_COPLANAR_IGNORED: u32 = 16; // coplanar ground near a stripline (not modelled in M1)
pub const F_DIFF_UNCOUPLED: u32 = 32; // diff-pair net but partner not adjacent here
pub const F_REF_PERFORATED: u32 = 64; // sample lies in a small plane opening (e.g. via antipad)
pub const F_PLANE_OPENING: u32 = 128; // opening in a layer between trace and reference (e.g. antipad under a pad)
pub const F_PAD: u32 = 256; // cross-section through an SMD pad
/// formula result over a cut-out in a layer between the trace and its reference, with the
/// cut-out's edge laterally closer to the trace than the reference is below it: the closed-form
/// models know only the reference plane (and coplanar copper on the trace's layer), not the
/// cut-out's edges, which then carry much of the return current – a rough estimate (the 3313
/// launch over a 0.465 mm opening in In1 reads 25 % high); the field solver models the cut-out
pub const F_FORMULA_ROUGH: u32 = 512;

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Params {
    /// sampling step along tracks (mm)
    pub step: f64,
    /// coplanar ground considered if gap < factor * h
    pub coplanar_factor: f64,
    /// include solder mask correction for outer layers
    pub solder_mask: bool,
    /// user overrides: netclass name -> target
    pub targets: HashMap<String, Target>,
    /// field-solver results by geometry id (see `Analysis::geometries`)
    pub field: HashMap<String, FieldResult>,
}

/// Result of a field-solver backend for one cross-section geometry.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct FieldResult {
    pub z0: Option<f64>,
    pub eeff: Option<f64>,
    pub zodd: Option<f64>,
    pub zeven: Option<f64>,
    pub eeff_odd: Option<f64>,
    pub eeff_even: Option<f64>,
    /// per-unit-length parameters at `f0` (Ω/m, H/m, S/m, F/m), smooth copper;
    /// single line, or the odd mode of a pair (per line)
    pub r: Option<f64>,
    pub l: Option<f64>,
    pub g: Option<f64>,
    pub c: Option<f64>,
    pub r_odd: Option<f64>,
    pub l_odd: Option<f64>,
    pub g_odd: Option<f64>,
    pub c_odd: Option<f64>,
    /// even mode of a pair (per line)
    pub r_even: Option<f64>,
    pub l_even: Option<f64>,
    pub g_even: Option<f64>,
    pub c_even: Option<f64>,
    /// 2x2 matrices of a two-conductor geometry, row major (H/m, F/m Maxwell)
    pub mat_l: Option<[f64; 4]>,
    pub mat_c: Option<[f64; 4]>,
    pub f0: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GeomEntry {
    /// stable hash of `text`
    pub id: String,
    /// cross-section in js_2d_fields custom-geometry syntax
    pub text: String,
    /// routed length using this geometry (mm), for prioritising solves
    pub length: f64,
    /// routed length of nets with a target impedance
    pub target_length: f64,
}

impl Default for Params {
    fn default() -> Self {
        Params { step: 0.5, coplanar_factor: 3.0, solder_mask: true, targets: HashMap::new(), field: HashMap::new() }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct Target {
    pub z: f64,
    pub diff: bool,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, Hash)]
pub enum Kind {
    Microstrip,
    Stripline,
    EmbeddedMicrostrip,
    NoReference,
}

#[derive(Debug, Clone, Serialize)]
pub struct CrossSection {
    pub kind: Kind,
    pub layer: u16,
    pub w: f64,
    pub t: f64,
    pub ref_up: Option<u16>,
    pub ref_dn: Option<u16>,
    /// dielectric distance to the reference above / below (mm)
    pub h_up: Option<f64>,
    pub h_dn: Option<f64>,
    pub er_up: Option<f64>,
    pub er_dn: Option<f64>,
    /// coplanar gaps (mm) on the left / right side, if a conductor is close
    pub s_left: Option<f64>,
    pub s_right: Option<f64>,
    /// gap to diff-pair partner (edge to edge, mm)
    pub diff_gap: Option<f64>,
    pub partner_w: Option<f64>,
    pub mask: bool,
    pub z0: Option<f64>,
    pub eeff: Option<f64>,
    pub zdiff: Option<f64>,
    /// even-mode impedance of one line of the pair (Zcommon = Zeven / 2) and the modal
    /// effective permittivities (field solver, or estimated from the formulas)
    pub zeven: Option<f64>,
    pub eeff_odd: Option<f64>,
    pub eeff_even: Option<f64>,
    /// closed-form values (kept for comparison when a field result is used)
    pub z0_formula: Option<f64>,
    pub zdiff_formula: Option<f64>,
    /// "field" if z0/zdiff come from a field solver, else "formula"
    pub solver: &'static str,
    /// index into `Analysis::geometries`
    pub geom: Option<u32>,
    /// plane edges / openings: (layer, left, right), see xsgeom::PlaneCut
    pub planes: Vec<(u16, Option<f64>, Option<f64>)>,
    pub flags: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct NetStat {
    pub net: u32,
    pub name: String,
    pub class: String,
    pub length: f64,
    pub target: Option<Target>,
    pub partner: Option<u32>,
    /// length-weighted statistics of the compared impedance
    pub z_min: Option<f64>,
    pub z_max: Option<f64>,
    pub z_mean: Option<f64>,
    /// fraction of routed length within ±5 % / ±10 % of target
    pub within5: Option<f64>,
    pub within10: Option<f64>,
    pub flags: u32,
    /// accuracy of the impedance (`accuracy`): length-weighted typical uncertainty (%) and the share
    /// of the length from the field solver
    pub z_unc_pct: Option<f64>,
    pub field_share: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ClassInfo {
    pub name: String,
    pub target: Option<Target>,
    pub target_source: String,
    pub nets: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct Analysis {
    pub cross_sections: Vec<CrossSection>,
    pub geometries: Vec<GeomEntry>,
    /// flat: track index, t0, t1, xs index (4 numbers per piece)
    pub pieces: Vec<f64>,
    pub nets: Vec<NetStat>,
    pub classes: Vec<ClassInfo>,
    pub samples: usize,
    pub unique_cross_sections: usize,
    /// true for `skeleton()`: no cross-sections or impedances yet
    pub skeleton: bool,
}

// --- spatial index for zone fills ------------------------------------------------

struct Edge {
    a: P,
    b: P,
    poly: u32,
}

/// Copper that can act as reference or coplanar ground on one layer:
/// zone fills plus pads of plane nets (GND, supplies).
struct LayerZones {
    edges: Vec<Edge>,
    poly_net: Vec<u32>,
    y0: f64,
    band_h: f64,
    bands: Vec<Vec<u32>>,
    x0: f64,
    cell: f64,
    nx: usize,
    ny: usize,
    cells: Vec<Vec<u32>>,
}

impl LayerZones {
    fn build(board: &Board, layer: u16) -> Self {
        let mut edges = Vec::new();
        let mut poly_net = Vec::new();
        let mut add_poly = |pts: &[P], net: u32, edges: &mut Vec<Edge>| {
            let pi = poly_net.len() as u32;
            poly_net.push(net);
            let n = pts.len();
            for k in 0..n {
                let a = pts[k];
                let b = pts[(k + 1) % n];
                if a != b {
                    edges.push(Edge { a, b, poly: pi });
                }
            }
        };
        for z in board.zones.iter().filter(|z| z.layer == layer) {
            add_poly(&z.pts, z.net, &mut edges);
        }
        for p in &board.pads {
            if p.layers.contains(&layer) && board.plane_net.get(p.net as usize).copied().unwrap_or(false) {
                add_poly(&crate::board::pad_polygon(p), p.net, &mut edges);
            }
        }
        let bb = board.bbox.unwrap_or(geom::BBox { min: [0.0, 0.0], max: [1.0, 1.0] });
        let (x0, y0) = (bb.min[0] - 1.0, bb.min[1] - 1.0);
        let (x1, y1) = (bb.max[0] + 1.0, bb.max[1] + 1.0);
        let band_h = 0.1;
        let nb = (((y1 - y0) / band_h).ceil() as usize).max(1);
        let mut bands = vec![Vec::new(); nb];
        let cell = 0.5;
        let nx = (((x1 - x0) / cell).ceil() as usize).max(1);
        let ny = (((y1 - y0) / cell).ceil() as usize).max(1);
        let mut cells = vec![Vec::new(); nx * ny];
        let clampi = |v: f64, n: usize| -> usize { (v.max(0.0) as usize).min(n - 1) };
        for (ei, e) in edges.iter().enumerate() {
            let ylo = e.a[1].min(e.b[1]);
            let yhi = e.a[1].max(e.b[1]);
            // horizontal edges never cross a horizontal scanline -> skip for bands
            if yhi > ylo {
                let b0 = clampi((ylo - y0) / band_h, nb);
                let b1 = clampi((yhi - y0) / band_h, nb);
                for band in &mut bands[b0..=b1] {
                    band.push(ei as u32);
                }
            }
            let cx0 = clampi((e.a[0].min(e.b[0]) - x0) / cell, nx);
            let cx1 = clampi((e.a[0].max(e.b[0]) - x0) / cell, nx);
            let cy0 = clampi((ylo - y0) / cell, ny);
            let cy1 = clampi((yhi - y0) / cell, ny);
            // walk only the cells the edge actually passes (approx: bbox cells;
            // edges are short in practice)
            for cy in cy0..=cy1 {
                for cx in cx0..=cx1 {
                    cells[cy * nx + cx].push(ei as u32);
                }
            }
        }
        LayerZones { edges, poly_net, y0, band_h, bands, x0, cell, nx, ny, cells }
    }

    /// Zone polygons (indices into board.zones) that contain point p.
    fn covering(&self, p: P, out: &mut Vec<u32>) {
        out.clear();
        if self.edges.is_empty() {
            return;
        }
        let bi = ((p[1] - self.y0) / self.band_h).floor();
        if bi < 0.0 || bi as usize >= self.bands.len() {
            return;
        }
        let mut counts: Vec<(u32, u32)> = Vec::new();
        for &ei in &self.bands[bi as usize] {
            let e = &self.edges[ei as usize];
            if (e.a[1] > p[1]) != (e.b[1] > p[1]) {
                let x = e.a[0] + (p[1] - e.a[1]) * (e.b[0] - e.a[0]) / (e.b[1] - e.a[1]);
                if x > p[0] {
                    match counts.iter_mut().find(|c| c.0 == e.poly) {
                        Some(c) => c.1 += 1,
                        None => counts.push((e.poly, 1)),
                    }
                }
            }
        }
        out.extend(counts.into_iter().filter(|c| c.1 % 2 == 1).map(|c| c.0));
    }

    /// Distance along ray p + t*d (|d| = 1) to the first copper edge whose
    /// net differs from `exclude_net` (u32::MAX: any), up to `maxd`.
    fn ray(&self, p: P, d: P, maxd: f64, exclude_net: u32) -> Option<f64> {
        if self.edges.is_empty() {
            return None;
        }
        let q = geom::add(p, geom::scale(d, maxd));
        let clampi = |v: f64, n: usize| -> usize { (v.max(0.0) as usize).min(n - 1) };
        let cx0 = clampi((p[0].min(q[0]) - self.x0) / self.cell, self.nx);
        let cx1 = clampi((p[0].max(q[0]) - self.x0) / self.cell, self.nx);
        let cy0 = clampi((p[1].min(q[1]) - self.y0) / self.cell, self.ny);
        let cy1 = clampi((p[1].max(q[1]) - self.y0) / self.cell, self.ny);
        let mut best: Option<f64> = None;
        for cy in cy0..=cy1 {
            for cx in cx0..=cx1 {
                for &ei in &self.cells[cy * self.nx + cx] {
                    let e = &self.edges[ei as usize];
                    if self.poly_net[e.poly as usize] == exclude_net {
                        continue;
                    }
                    if let Some(t) = geom::ray_seg(p, d, e.a, e.b) {
                        if t <= maxd && best.is_none_or(|b| t < b) {
                            best = Some(t);
                        }
                    }
                }
            }
        }
        best
    }
}

// --- spatial index for tracks (diff partner search) ----------------------------

/// Width of the copper at `p` of track `tr` when `p` lies on an SMD pad of the track's own net and
/// layer that is wider across the track than the track (`board.tracks` holds every such pad as two
/// synthetic pad tracks, `Track::pad`).
fn pad_width_at(
    board: &Board,
    tgrid: &TrackGrid,
    near: &mut Vec<u32>,
    tr: &crate::board::Track,
    p: P,
    nrm: P,
) -> Option<f64> {
    tgrid.near(p, 0.05, near);
    near.iter()
        .map(|&i| &board.tracks[i as usize])
        .filter(|t| t.pad && t.net == tr.net && t.layer == tr.layer)
        .map(|t| &board.pads[t.item as usize])
        .filter(|pd| crate::board::pad_contains(pd, p, 0.0))
        .map(|pd| crate::board::pad_chord(pd, p, nrm))
        .fold(None, |m: Option<f64>, x| Some(m.map_or(x, |m| m.max(x))))
        .filter(|&x| x > tr.width + 1e-6)
}

/// Parameters t ∈ (0, 1) on track `tr` where it crosses the edge of an SMD pad of its own net and
/// layer (the pads it touches: those with synthetic pad tracks near either end).
fn pad_edges_on(board: &Board, tgrid: &TrackGrid, near: &mut Vec<u32>, tr: &crate::board::Track) -> Vec<f64> {
    let mut out = Vec::new();
    let mut seen: Vec<u32> = Vec::new();
    for e in [tr.a, tr.b] {
        tgrid.near(e, 0.05, near);
        for &i in near.iter() {
            let t = &board.tracks[i as usize];
            if !(t.pad && t.net == tr.net && t.layer == tr.layer) || seen.contains(&t.item) {
                continue;
            }
            seen.push(t.item);
            if let Some((t0, t1)) = crate::board::pad_segment_span(&board.pads[t.item as usize], tr.a, tr.b) {
                out.extend([t0, t1].into_iter().filter(|&x| x > 1e-6 && x < 1.0 - 1e-6));
            }
        }
    }
    out
}

struct TrackGrid {
    x0: f64,
    y0: f64,
    cell: f64,
    nx: usize,
    ny: usize,
    cells: Vec<Vec<u32>>,
}

impl TrackGrid {
    fn build(board: &Board) -> Self {
        let bb = board.bbox.unwrap_or(geom::BBox { min: [0.0, 0.0], max: [1.0, 1.0] });
        let (x0, y0) = (bb.min[0] - 1.0, bb.min[1] - 1.0);
        let cell = 1.0;
        let nx = (((bb.max[0] + 1.0 - x0) / cell).ceil() as usize).max(1);
        let ny = (((bb.max[1] + 1.0 - y0) / cell).ceil() as usize).max(1);
        let mut cells = vec![Vec::new(); nx * ny];
        let clampi = |v: f64, n: usize| -> usize { (v.max(0.0) as usize).min(n - 1) };
        for (ti, t) in board.tracks.iter().enumerate() {
            let cx0 = clampi((t.a[0].min(t.b[0]) - x0) / cell, nx);
            let cx1 = clampi((t.a[0].max(t.b[0]) - x0) / cell, nx);
            let cy0 = clampi((t.a[1].min(t.b[1]) - y0) / cell, ny);
            let cy1 = clampi((t.a[1].max(t.b[1]) - y0) / cell, ny);
            for cy in cy0..=cy1 {
                for cx in cx0..=cx1 {
                    cells[cy * nx + cx].push(ti as u32);
                }
            }
        }
        TrackGrid { x0, y0, cell, nx, ny, cells }
    }
    fn near(&self, p: P, r: f64, out: &mut Vec<u32>) {
        out.clear();
        let clampi = |v: f64, n: usize| -> usize { (v.max(0.0) as usize).min(n - 1) };
        let cx0 = clampi((p[0] - r - self.x0) / self.cell, self.nx);
        let cx1 = clampi((p[0] + r - self.x0) / self.cell, self.nx);
        let cy0 = clampi((p[1] - r - self.y0) / self.cell, self.ny);
        let cy1 = clampi((p[1] + r - self.y0) / self.cell, self.ny);
        for cy in cy0..=cy1 {
            for cx in cx0..=cx1 {
                out.extend_from_slice(&self.cells[cy * self.nx + cx]);
            }
        }
        out.sort_unstable();
        out.dedup();
    }
}

// --- diff pairs & targets -------------------------------------------------------

/// KiCad convention: names ending in `+`/`-` or `P`/`N` with a common base.
pub fn diff_partners(nets: &[String]) -> Vec<Option<u32>> {
    let by_name: HashMap<&str, u32> = nets.iter().enumerate().map(|(i, n)| (n.as_str(), i as u32)).collect();
    let mut out = vec![None; nets.len()];
    for (i, n) in nets.iter().enumerate() {
        if n.len() < 2 {
            continue;
        }
        let (base, last) = n.split_at(n.len() - 1);
        let other = match last {
            "+" => format!("{base}-"),
            "P" => format!("{base}N"),
            "p" => format!("{base}n"),
            _ => continue,
        };
        if let Some(&j) = by_name.get(other.as_str()) {
            out[i] = Some(j);
            out[j as usize] = Some(i as u32);
        }
    }
    out
}

/// Derive a target impedance from a net class name, e.g. `50Z`, `FE_100Z_Diff`,
/// `USB_90R`, `Z85_PCIe`. Falls back to interface keywords.
pub fn target_from_class_name(name: &str) -> Option<(Target, &'static str)> {
    let up = name.to_uppercase();
    let is_diff_kw = up.contains("DIFF") || up.contains("_DP") || up.starts_with("DP_") || up.ends_with("DP");
    // number followed by Z / R / OHM / Ω, or Z followed by number
    let chars: Vec<char> = up.chars().collect();
    let mut num: Option<f64> = None;
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_ascii_digit() {
            let st = i;
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                i += 1;
            }
            let v: f64 = chars[st..i].iter().collect::<String>().parse().unwrap_or(0.0);
            let mut j = i;
            while j < chars.len() && (chars[j] == '_' || chars[j] == ' ') {
                j += 1;
            }
            let rest: String = chars[j..].iter().collect();
            let before_z = st > 0 && chars[st - 1] == 'Z';
            if (rest.starts_with('Z')
                || rest.starts_with('R')
                || rest.starts_with("OHM")
                || rest.starts_with('Ω')
                || before_z)
                && (20.0..=200.0).contains(&v)
            {
                num = Some(v);
                break;
            }
        } else {
            i += 1;
        }
    }
    if let Some(z) = num {
        let diff = is_diff_kw || z >= 80.0 && (up.contains("USB") || up.contains("LVDS") || up.contains("PCI"));
        return Some((Target { z, diff }, "class name"));
    }
    let kw: &[(&str, f64, bool)] = &[
        ("USB", 90.0, true),
        ("PCIE", 85.0, true),
        ("PCI_E", 85.0, true),
        ("LVDS", 100.0, true),
        ("HDMI", 100.0, true),
        ("TMDS", 100.0, true),
        ("SATA", 100.0, true),
        ("MDI", 100.0, true),
        ("ETH", 100.0, true),
        ("RF", 50.0, false),
    ];
    for &(k, z, diff) in kw {
        if up.split(|c: char| !c.is_ascii_alphanumeric() && c != '_').any(|tok| tok.contains(k)) {
            return Some((Target { z, diff }, "interface assumption"));
        }
    }
    None
}

// --- the extraction ----------------------------------------------------------------

#[derive(Hash, PartialEq, Eq, Clone)]
struct Key {
    layer: u16,
    up: i32,
    dn: i32,
    w: i64,
    sl: i64,
    sr: i64,
    gap: i64,
    pw: i64,
    pleft: bool,
    planes: Vec<(u16, i64, i64)>,
    flags: u32,
}

/// Snap a lateral distance `d` (gap, plane edge) relative to the height `h`
/// that sets the field spread: fine steps close to the trace, coarse steps
/// far away where the impedance hardly depends on it (< 1 % per step).
fn snap_rel(d: f64, h: f64) -> f64 {
    let x = d / h.max(1e-3);
    let ratio = if x < 0.5 {
        1.03
    } else if x < 1.5 {
        1.08
    } else {
        1.2
    };
    snap_ratio(d, ratio)
}

/// Snap a distance (mm) to a geometric grid with the given step ratio.
fn snap_ratio(s: f64, ratio: f64) -> f64 {
    let base = 0.005;
    if s <= base {
        return base;
    }
    base * ratio.powf(((s / base).ln() / ratio.ln()).round())
}

/// Probe distance around a sample point in an opening of a plane (mm).
const OPENING_PROBE_MM: f64 = 0.3;
/// Openings longer than this along the track are never small (mm): antipads are smaller.
const OPENING_SMALL_MM: f64 = 1.0;

/// Whether the plane opening at sample `p` is small (a via antipad, a hole in a BGA field) so
/// that the plane still acts as the reference: copper within `OPENING_PROBE_MM` in three of the
/// four directions (along and across the track), and along the track in both directions within
/// `OPENING_SMALL_MM`. An opening that continues along the track – a slot, a cut-out under a
/// launch – is not small, also within `OPENING_PROBE_MM` of its end: the trace has no reference
/// over it.
fn small_opening(z: &LayerZones, p: P, dir: P, nrm: P, cover: &mut Vec<u32>) -> bool {
    let mut covered = |d: P, r: f64| {
        z.covering(geom::add(p, geom::scale(d, r)), cover);
        !cover.is_empty()
    };
    let back = geom::scale(dir, -1.0);
    let (f, b) = (covered(dir, OPENING_PROBE_MM), covered(back, OPENING_PROBE_MM));
    let n = [f, b, covered(nrm, OPENING_PROBE_MM), covered(geom::scale(nrm, -1.0), OPENING_PROBE_MM)]
        .iter()
        .filter(|x| **x)
        .count();
    n >= 3 && (f || covered(dir, OPENING_SMALL_MM)) && (b || covered(back, OPENING_SMALL_MM))
}

fn q(v: Option<f64>, res: f64) -> i64 {
    match v {
        Some(x) => (x / res).round() as i64,
        None => -1,
    }
}

pub fn analyze(board: &Board, params: &Params) -> Analysis {
    let su = board.stackup();
    let ncu = su.n();
    let zones: Vec<LayerZones> = (0..ncu as u16).map(|l| LayerZones::build(board, l)).collect();
    let tgrid = TrackGrid::build(board);
    let partners = diff_partners(&board.nets);

    let class_targets = class_targets(board, params);

    let mut cache: HashMap<Key, u32> = HashMap::new();
    let mut geoms = GeomReg::default();
    let mut xss: Vec<CrossSection> = Vec::new();
    let mut pieces: Vec<f64> = Vec::new();
    let mut cover = Vec::new();
    let mut near = Vec::new();
    let mut samples = 0usize;

    for (ti, tr) in board.tracks.iter().enumerate() {
        let d = geom::sub(tr.b, tr.a);
        let len = geom::len(d);
        if len < 1e-6 {
            continue;
        }
        let dir = geom::scale(d, 1.0 / len);
        let nrm = [-dir[1], dir[0]];
        let li = tr.layer as usize;
        let partner = partners.get(tr.net as usize).copied().flatten();
        // sample intervals: steps of at most `params.step`, split where the track enters or leaves
        // an SMD pad of its own net (the pad's cross-section starts exactly at its edge)
        let mut cuts = vec![0.0, 1.0];
        if !tr.pad {
            cuts.extend(pad_edges_on(board, &tgrid, &mut near, tr));
            cuts.sort_by(f64::total_cmp);
            cuts.dedup_by(|a, b| (*a - *b).abs() * len < 1e-4);
        }
        let mut spans: Vec<(f64, f64)> = Vec::new();
        for c in cuts.windows(2) {
            let m = (((c[1] - c[0]) * len / params.step).ceil() as usize).max(1);
            spans.extend((0..m).map(|k| {
                (c[0] + (c[1] - c[0]) * k as f64 / m as f64, c[0] + (c[1] - c[0]) * (k + 1) as f64 / m as f64)
            }));
        }
        let mut prev: Option<(u32, f64)> = None; // (xs, t0)
        for &(t_lo, t_hi) in &spans {
            samples += 1;
            let tm = 0.5 * (t_lo + t_hi);
            let p = geom::lerp(tr.a, tr.b, tm);
            let mut flags = 0u32;
            // a trace inside an SMD pad of its own net (a track that runs to the pad's centre, a
            // card-edge finger): the copper there is the pad, as wide as the pad across the track
            let w =
                if tr.pad { tr.width } else { pad_width_at(board, &tgrid, &mut near, tr, p, nrm).unwrap_or(tr.width) };
            if w > tr.width {
                flags |= F_PAD;
            }

            // reference planes
            let mut perforated = false;
            let mut perf_layers: Vec<usize> = Vec::new();
            let mut find_ref = |range: &mut dyn Iterator<Item = usize>| -> Option<(usize, f64, bool)> {
                for j in range {
                    zones[j].covering(p, &mut cover);
                    let mut hit = !cover.is_empty();
                    if !hit && small_opening(&zones[j], p, dir, nrm, &mut cover) {
                        // small openings (via antipads in a BGA field) do not
                        // remove the reference
                        hit = true;
                        perforated = true;
                        perf_layers.push(j);
                    }
                    if hit {
                        let h = su.gap_between(li, j);
                        let off = w / 2.0 + h;
                        let mut partial = false;
                        for s in [-1.0, 1.0] {
                            let pp = geom::add(p, geom::scale(nrm, s * off));
                            zones[j].covering(pp, &mut cover);
                            if cover.is_empty() {
                                partial = true;
                            }
                        }
                        return Some((j, h, partial));
                    }
                }
                None
            };
            let up = if li > 0 { find_ref(&mut (0..li).rev()) } else { None };
            let dn = if li + 1 < ncu { find_ref(&mut (li + 1..ncu)) } else { None };
            if perforated {
                flags |= F_REF_PERFORATED;
            }
            for (j, _, partial) in [&up, &dn].into_iter().flatten() {
                if *partial {
                    flags |= F_PARTIAL_REF;
                }
                if (*j as isize - li as isize).abs() > 1 {
                    flags |= F_REF_SKIPS_LAYER;
                }
            }
            let hmin = [up.map(|r| r.1), dn.map(|r| r.1)].into_iter().flatten().fold(f64::INFINITY, f64::min);
            if tr.pad {
                flags |= F_PAD;
            }

            // plane edges and openings on all layers of the modelled stack
            let mut planes: Vec<PlaneCut> = Vec::new();
            if up.is_some() || dn.is_some() {
                // the field spreads laterally about as far as the nearest
                // reference is away; copper beyond ~4 h_min changes Z by < 1 %
                let win = (w / 2.0 + 4.0 * hmin).min(2.0);
                let k_top = up.map(|r| r.0).unwrap_or(0);
                let k_bot = dn.map(|r| r.0).unwrap_or(ncu - 1);
                let neg = geom::scale(nrm, -1.0);
                for k in k_top..=k_bot {
                    if k == li {
                        continue;
                    }
                    let is_ref = up.map(|r| r.0) == Some(k) || dn.map(|r| r.0) == Some(k);
                    if is_ref && perf_layers.contains(&k) {
                        continue; // perforated plane: treat as continuous
                    }
                    let mut edge = |d: P| -> Option<f64> {
                        let t = zones[k].ray(p, d, win, u32::MAX)?;
                        if is_ref {
                            // a small hole (via antipad) is not a plane edge:
                            // copper resumes right behind it
                            zones[k].covering(geom::add(p, geom::scale(d, t + 0.6)), &mut cover);
                            if !cover.is_empty() {
                                return None;
                            }
                        }
                        Some(snap_rel((t - w / 2.0).max(0.005), hmin) + w / 2.0)
                    };
                    let (l, r) = (edge(nrm), edge(neg));
                    if l.is_some() || r.is_some() {
                        if !is_ref {
                            flags |= F_PLANE_OPENING;
                        }
                        planes.push(PlaneCut { layer: k, left: l, right: r });
                    }
                }
            }

            // coplanar conductors on the same layer
            // (without a plane: the coplanar copper within 2 mm is the return, `analytic::no_plane`)
            let maxd = w / 2.0 + if hmin.is_finite() { (params.coplanar_factor * hmin).min(2.0) } else { 2.0 };
            let s = |dist: Option<f64>| dist.map(|x| (x - w / 2.0).max(0.005));
            let mut s_left = s(zones[li].ray(p, nrm, maxd, tr.net));
            let mut s_right = s(zones[li].ray(p, geom::scale(nrm, -1.0), maxd, tr.net));

            // diff partner
            let mut diff_gap = None;
            let mut partner_geo: Option<(f64, f64, bool)> = None;
            if let Some(pn) = partner {
                let r = 1.5f64.min(w * 2.0 + 4.0 * hmin.min(0.5));
                tgrid.near(p, r + w, &mut near);
                for &oi in &near {
                    let o = &board.tracks[oi as usize];
                    if o.net != pn || o.layer != tr.layer {
                        continue;
                    }
                    let od = geom::sub(o.b, o.a);
                    let ol = geom::len(od);
                    if ol < 1e-6 {
                        continue;
                    }
                    let odir = geom::scale(od, 1.0 / ol);
                    if geom::cross(dir, odir).abs() > 0.26 {
                        continue; // not parallel (> ~15°)
                    }
                    let ap = geom::sub(p, o.a);
                    let along = geom::dot(ap, odir);
                    if along < -0.05 || along > ol + 0.05 {
                        continue;
                    }
                    let dist = geom::cross(odir, ap).abs();
                    let gap = dist - (w + o.width) / 2.0;
                    if gap > 0.0 && gap < r && diff_gap.is_none_or(|g| gap < g) {
                        diff_gap = Some(gap);
                        // partner on the +normal ("left") side?
                        let left = geom::dot(geom::sub(geom::add(o.a, geom::scale(odir, along)), p), nrm) > 0.0;
                        partner_geo = Some((gap, o.width, left));
                    }
                }
                if diff_gap.is_none() {
                    flags |= F_DIFF_UNCOUPLED;
                }
            }

            // coplanar gaps snap to a 3 % geometric grid: Z changes by well
            // under 0.5 % within a step, and far fewer distinct cross-sections
            // need a field solve
            s_left = s_left.map(|s| snap_rel(s, hmin));
            s_right = s_right.map(|s| snap_rel(s, hmin));
            // canonical orientation: partner always on the left (mirror image
            // is the same cross-section), halves the number of diff geometries
            if let Some((g, pw, false)) = partner_geo {
                partner_geo = Some((g, pw, true));
                std::mem::swap(&mut s_left, &mut s_right);
                for c in planes.iter_mut() {
                    std::mem::swap(&mut c.left, &mut c.right);
                }
            }
            let key = Key {
                layer: tr.layer,
                up: up.map(|r| r.0 as i32).unwrap_or(-1),
                dn: dn.map(|r| r.0 as i32).unwrap_or(-1),
                w: q(Some(w), 1e-4),
                sl: q(s_left, 1e-4),
                sr: q(s_right, 1e-4),
                gap: q(diff_gap, 5e-3),
                pw: q(partner_geo.map(|x| x.1), 1e-4),
                pleft: partner_geo.map(|x| x.2).unwrap_or(false),
                planes: planes.iter().map(|c| (c.layer as u16, q(c.left, 1e-4), q(c.right, 1e-4))).collect(),
                flags,
            };
            let xi = match cache.get(&key) {
                Some(&i) => i,
                None => {
                    let xs = evaluate(
                        board,
                        params,
                        &mut geoms,
                        tr.layer,
                        w,
                        up.map(|r| (r.0, r.1)),
                        dn.map(|r| (r.0, r.1)),
                        s_left,
                        s_right,
                        partner_geo,
                        planes,
                        flags,
                    );
                    let i = xss.len() as u32;
                    xss.push(xs);
                    cache.insert(key, i);
                    i
                }
            };
            let t0 = t_lo;
            match prev {
                Some((px, _)) if px == xi => {}
                Some((px, pt0)) => {
                    pieces.extend_from_slice(&[ti as f64, pt0, t0, px as f64]);
                    prev = Some((xi, t0));
                }
                None => prev = Some((xi, t0)),
            }
        }
        if let Some((px, pt0)) = prev {
            pieces.extend_from_slice(&[ti as f64, pt0, 1.0, px as f64]);
        }
    }

    let (nets, classes) = net_stats(board, &class_targets, &partners, &xss, &pieces);

    // routed length per geometry
    let mut geometries = geoms.list;
    for pc in pieces.as_chunks::<4>().0 {
        let xs = &xss[pc[3] as usize];
        if let Some(g) = xs.geom {
            let tr = &board.tracks[pc[0] as usize];
            let l = geom::len(geom::sub(tr.b, tr.a)) * (pc[2] - pc[1]);
            geometries[g as usize].length += l;
            let cls = board.net_class.get(tr.net as usize).copied().unwrap_or(0) as usize;
            if class_targets.get(cls).and_then(|t| t.0).is_some() {
                geometries[g as usize].target_length += l;
            }
        }
    }

    Analysis {
        unique_cross_sections: xss.len(),
        cross_sections: xss,
        geometries,
        pieces,
        nets,
        classes,
        samples,
        skeleton: false,
    }
}

fn class_targets(board: &Board, params: &Params) -> Vec<(Option<Target>, String)> {
    board
        .netclasses
        .iter()
        .map(|c| {
            if let Some(t) = params.targets.get(&c.name) {
                (Some(*t), "user".to_string())
            } else if let Some((t, src)) = target_from_class_name(&c.name) {
                (Some(t), src.to_string())
            } else {
                (None, String::new())
            }
        })
        .collect()
}

/// Per-net statistics and class summary from the pieces of an analysis.
fn net_stats(
    board: &Board,
    class_targets: &[(Option<Target>, String)],
    partners: &[Option<u32>],
    xss: &[CrossSection],
    pieces: &[f64],
) -> (Vec<NetStat>, Vec<ClassInfo>) {
    // (length, impedance, flags, counted in the impedance statistics, uncertainty %, field solver)
    let mut acc: HashMap<u32, Vec<(f64, Option<f64>, u32, bool, f64, bool)>> = HashMap::new();
    for pc in pieces.as_chunks::<4>().0 {
        let tr = &board.tracks[pc[0] as usize];
        let l = geom::len(geom::sub(tr.b, tr.a)) * (pc[2] - pc[1]);
        let xs = &xss[pc[3] as usize];
        // a track inside an SMD pad has the pad's cross-section; the pad's own synthetic track
        // stands for it in the impedance statistics (not counted twice), its length stays the net's
        let counted = tr.pad || xs.flags & F_PAD == 0;
        let cls = board.net_class.get(tr.net as usize).copied().unwrap_or(0) as usize;
        let target = class_targets.get(cls).and_then(|t| t.0);
        let z = compared_z(xs, target);
        let unc = crate::accuracy::impedance_of(xs).map(|u| u.1).unwrap_or(0.0);
        acc.entry(tr.net).or_default().push((
            l,
            z.map(|v| v.0).filter(|_| counted),
            xs.flags,
            counted,
            unc,
            xs.solver == "field",
        ));
    }
    let mut nets: Vec<NetStat> = Vec::new();
    let mut class_counts = vec![0usize; board.netclasses.len()];
    for (&net, v) in &acc {
        let cls = board.net_class.get(net as usize).copied().unwrap_or(0) as usize;
        if cls < class_counts.len() {
            class_counts[cls] += 1;
        }
        let target = class_targets.get(cls).and_then(|t| t.0);
        let length: f64 = v.iter().map(|x| x.0).sum();
        let l_counted: f64 = v.iter().filter(|x| x.3).map(|x| x.0).sum::<f64>().max(1e-12);
        let zl: Vec<(f64, f64)> = v.iter().filter_map(|x| x.1.map(|z| (x.0, z))).collect();
        let lz: f64 = zl.iter().map(|x| x.0).sum();
        let (mut z_min, mut z_max, mut z_mean) = (None, None, None);
        // accuracy: length-weighted uncertainty and field-solver share of the pieces with an impedance
        let zp: Vec<&(f64, Option<f64>, u32, bool, f64, bool)> = v.iter().filter(|x| x.1.is_some()).collect();
        let (z_unc_pct, field_share) = if lz > 0.0 {
            (
                Some(zp.iter().map(|x| x.0 * x.4).sum::<f64>() / lz),
                Some(zp.iter().filter(|x| x.5).map(|x| x.0).sum::<f64>() / lz),
            )
        } else {
            (None, None)
        };
        let (mut w5, mut w10) = (None, None);
        if lz > 0.0 {
            z_min = zl.iter().map(|x| x.1).reduce(f64::min);
            z_max = zl.iter().map(|x| x.1).reduce(f64::max);
            z_mean = Some(zl.iter().map(|x| x.0 * x.1).sum::<f64>() / lz);
            if let Some(t) = target {
                w5 = Some(
                    zl.iter().filter(|x| (x.1 - t.z).abs() <= 0.05 * t.z).map(|x| x.0).sum::<f64>() / l_counted + 0.0,
                );
                w10 = Some(
                    zl.iter().filter(|x| (x.1 - t.z).abs() <= 0.10 * t.z).map(|x| x.0).sum::<f64>() / l_counted + 0.0,
                );
            }
        }
        nets.push(NetStat {
            net,
            name: board.net_name(net).to_string(),
            class: board.netclasses.get(cls).map(|c| c.name.clone()).unwrap_or_default(),
            length,
            target,
            partner: partners.get(net as usize).copied().flatten(),
            z_min,
            z_max,
            z_mean,
            within5: w5,
            within10: w10,
            flags: v.iter().fold(0, |a, x| a | x.2),
            z_unc_pct,
            field_share,
        });
    }
    nets.sort_by(|a, b| a.name.cmp(&b.name));

    let classes: Vec<ClassInfo> = board
        .netclasses
        .iter()
        .enumerate()
        .map(|(i, c)| ClassInfo {
            name: c.name.clone(),
            target: class_targets[i].0,
            target_source: class_targets[i].1.clone(),
            nets: class_counts[i],
        })
        .collect();
    (nets, classes)
}

/// Board summary without impedances: net/class data, a preliminary line type per
/// layer (outer = microstrip, inner = stripline) and one piece per track, so the
/// viewer can show the board, net classes and targets before the (slower)
/// cross-section analysis runs. `skeleton` is set.
pub fn skeleton(board: &Board, params: &Params) -> Analysis {
    let class_targets = class_targets(board, params);
    let partners = diff_partners(&board.nets);
    let ncu = board.stackup().n();
    let xss: Vec<CrossSection> = (0..ncu as u16)
        .map(|layer| CrossSection {
            // reference planes are only checked by analyze()
            kind: if layer == 0 || layer as usize + 1 == ncu { Kind::Microstrip } else { Kind::Stripline },
            layer,
            w: 0.0,
            t: 0.0,
            ref_up: None,
            ref_dn: None,
            h_up: None,
            h_dn: None,
            er_up: None,
            er_dn: None,
            s_left: None,
            s_right: None,
            diff_gap: None,
            partner_w: None,
            mask: false,
            z0: None,
            eeff: None,
            zdiff: None,
            zeven: None,
            eeff_odd: None,
            eeff_even: None,
            z0_formula: None,
            zdiff_formula: None,
            solver: "none",
            geom: None,
            planes: vec![],
            flags: 0,
        })
        .collect();
    let mut pieces = Vec::with_capacity(board.tracks.len() * 4);
    for (i, tr) in board.tracks.iter().enumerate() {
        pieces.extend_from_slice(&[i as f64, 0.0, 1.0, tr.layer.min(ncu.saturating_sub(1) as u16) as f64]);
    }
    let (nets, classes) = net_stats(board, &class_targets, &partners, &xss, &pieces);
    Analysis {
        unique_cross_sections: 0,
        cross_sections: xss,
        geometries: vec![],
        pieces,
        nets,
        classes,
        samples: 0,
        skeleton: true,
    }
}

/// The impedance that is compared against a target:
/// Zdiff for coupled diff segments with a differential target,
/// 2·Z0 for uncoupled segments of a differential net, else Z0.
/// Returns (value comparable to target.z, is_differential_value).
pub fn compared_z(xs: &CrossSection, target: Option<Target>) -> Option<(f64, bool)> {
    match target {
        Some(t) if t.diff => match (xs.zdiff, xs.z0) {
            (Some(zd), _) => Some((zd, true)),
            (None, Some(z0)) => Some((2.0 * z0, false)),
            _ => None,
        },
        _ => xs.z0.map(|z| (z, false)),
    }
}

#[derive(Default)]
struct GeomReg {
    list: Vec<GeomEntry>,
    by_text: HashMap<String, u32>,
}

impl GeomReg {
    fn add(&mut self, text: String) -> u32 {
        if let Some(&i) = self.by_text.get(&text) {
            return i;
        }
        let i = self.list.len() as u32;
        self.list.push(GeomEntry { id: xsgeom::geom_id(&text), text: text.clone(), length: 0.0, target_length: 0.0 });
        self.by_text.insert(text, i);
        i
    }
}

fn evaluate(
    board: &Board,
    params: &Params,
    geoms: &mut GeomReg,
    layer: u16,
    w: f64,
    up: Option<(usize, f64)>,
    dn: Option<(usize, f64)>,
    s_left: Option<f64>,
    s_right: Option<f64>,
    partner: Option<(f64, f64, bool)>,
    planes: Vec<PlaneCut>,
    mut flags: u32,
) -> CrossSection {
    let diff_gap = partner.map(|p| p.0);
    // an opening in a layer between the trace and its reference whose edge is laterally closer
    // to the trace than the reference is below it: the edge carries much of the return current
    let h_ref = [up, dn].into_iter().flatten().map(|r| r.1).fold(f64::INFINITY, f64::min);
    let near_opening = planes
        .iter()
        .filter(|c| Some(c.layer) != up.map(|r| r.0) && Some(c.layer) != dn.map(|r| r.0))
        .filter_map(|c| [c.left, c.right].into_iter().flatten().reduce(f64::min))
        .any(|d| d - w / 2.0 < h_ref);
    let su = board.stackup();
    let li = layer as usize;
    let ncu = su.n();
    let t = su.copper[li].thickness;
    let er_up = up.map(|(j, _)| su.er_between(li, j));
    let er_dn = dn.map(|(j, _)| su.er_between(li, j));
    let outer_top = li == 0;
    let outer_bot = li + 1 == ncu;
    let mask = if outer_top {
        su.mask_top.as_ref()
    } else if outer_bot {
        su.mask_bottom.as_ref()
    } else {
        None
    }
    .filter(|_| params.solder_mask);

    let mut kind = Kind::NoReference;
    let mut z0 = None;
    let mut eeff = None;
    let mut zdiff = None;

    let mask_p = mask.map(|m| (m.thickness, m.epsilon_r));
    let ms = |h: f64, er: f64| -> (f64, f64) {
        let e = match mask {
            Some(m) => analytic::microstrip_masked(w, h, t, er, m.thickness, m.epsilon_r).1,
            None => analytic::microstrip(w, h, t, er).1,
        };
        (analytic::microstrip_coplanar(w, h, t, er, mask_p, s_left, s_right), e)
    };

    if outer_top || outer_bot {
        let r = if outer_top { dn } else { up };
        let er = if outer_top { er_dn } else { er_up };
        if let (Some((_, h)), Some(er)) = (r, er) {
            kind = Kind::Microstrip;
            let (z, e) = ms(h, er);
            z0 = Some(z);
            eeff = Some(e);
            if let Some(g) = diff_gap {
                zdiff = Some(analytic::diff_microstrip_coplanar(w, g, h, t, er, mask_p, s_left, s_right));
            }
        }
    } else {
        match (up, dn) {
            (Some((_, h1)), Some((_, h2))) => {
                kind = Kind::Stripline;
                let (e1, e2) = (er_up.unwrap(), er_dn.unwrap());
                let z = analytic::stripline_asym(w, h1, h2, t, e1, e2);
                z0 = Some(z);
                // effective permittivity: capacitance weighted
                eeff = Some((e1 / h1 + e2 / h2) / (1.0 / h1 + 1.0 / h2));
                if let Some(g) = diff_gap {
                    zdiff = Some(analytic::diff_stripline(z, g, h1 + h2 + t));
                }
                let smin = [s_left, s_right].into_iter().flatten().fold(f64::INFINITY, f64::min);
                if smin < h1.min(h2) {
                    flags |= F_COPLANAR_IGNORED;
                }
            }
            (Some((j, h)), None) | (None, Some((j, h))) => {
                kind = Kind::EmbeddedMicrostrip;
                flags |= F_ONE_SIDED;
                let er = su.er_between(li, j);
                // dielectric covering the trace on the other side: up to the
                // neighbouring copper layer
                let other = if j < li { li + 1 } else { li - 1 };
                let d = su.gap_between(li, other);
                let (z, e) = analytic::embedded_microstrip(w, h, t, d, er);
                z0 = Some(z);
                eeff = Some(e);
                if let Some(g) = diff_gap {
                    zdiff = Some(analytic::diff_microstrip(z, g, h));
                }
            }
            (None, None) => {}
        }
    }
    // no plane at all: the return in the coplanar copper beside the trace and in a pair's partner
    // (analytic::no_plane, an estimate); without either, no impedance (the channel's fallback)
    let mut no_plane_even = None;
    if kind == Kind::NoReference {
        flags |= F_NO_REF;
        let (h, er) = if outer_top {
            (su.gap_between(li, ncu - 1), su.er_between(li, ncu - 1))
        } else if outer_bot {
            (su.gap_between(0, li), su.er_between(0, li))
        } else {
            (f64::INFINITY, su.er_between(0, ncu - 1))
        };
        // only a close coplanar return carries the field (a card-edge finger between its ground
        // fingers); a conductor further away – a trace crossing a via's antipads on every layer –
        // leaves the return to plane edges the cross-section does not see: no impedance
        let near = |s: Option<f64>| s.filter(|&x| x <= (3.0 * w).max(0.5));
        if let Some((z, e, zd, ze)) = analytic::no_plane(w, h, er, near(s_left), near(s_right), near(diff_gap)) {
            z0 = Some(z);
            eeff = Some(e);
            zdiff = zd;
            no_plane_even = Some(ze);
        }
    }

    // geometry for field solvers, and a field result if one is available
    let desc = XsDesc {
        layer: li,
        w,
        ref_up: up.map(|r| r.0),
        ref_dn: dn.map(|r| r.0),
        s_left,
        s_right,
        partner,
        mask: mask.map(|m| (m.thickness, m.epsilon_r)),
        planes: planes.clone(),
        ..Default::default()
    };
    let (z0_formula, zdiff_formula) = (z0, zdiff);
    // even mode from the formulas: Z0 ≈ √(Zeven·Zodd); modal permittivities of microstrip from
    // Kirschning-Jansen (the ratio around the single-line value), stripline: homogeneous
    let mut zeven = match (no_plane_even, z0, zdiff) {
        (Some(ze), _, _) => ze,
        (None, Some(z), Some(zd)) if zd > 0.0 => Some(z * z / (zd / 2.0)),
        _ => None,
    };
    let (mut eeff_odd, mut eeff_even) = (eeff, eeff);
    if let (Some(g), Some(e)) = (diff_gap, eeff) {
        if matches!(kind, Kind::Microstrip | Kind::EmbeddedMicrostrip) {
            if let Some((h, er)) = (if outer_top { dn } else { up }).or(up).or(dn).map(|r| r.1).zip(er_dn.or(er_up)) {
                let (_, _, ee, eo) = analytic::kirschning_jansen(w, g, h, er);
                let em = 0.5 * (ee + eo);
                if em > 0.0 {
                    eeff_odd = Some(e * eo / em);
                    eeff_even = Some(e * ee / em);
                }
            }
        }
    }
    let mut solver = "formula";
    let geom_idx = xsgeom::build(su, &desc).map(|text| geoms.add(text));
    if let Some(gi) = geom_idx {
        let id = &geoms.list[gi as usize].id;
        if let Some(fr) = params.field.get(id) {
            if let (Some(zo), Some(ze)) = (fr.zodd, fr.zeven) {
                zdiff = Some(2.0 * zo);
                zeven = Some(ze);
                // single-ended impedance of one trace of the pair (uncoupled
                // estimate): geometric mean of the modal impedances
                z0 = Some((zo * ze).sqrt());
                if let (Some(a), Some(b)) = (fr.eeff_odd, fr.eeff_even) {
                    eeff = Some(0.5 * (a + b));
                    eeff_odd = Some(a);
                    eeff_even = Some(b);
                }
                solver = "field";
            } else if let Some(z) = fr.z0 {
                z0 = Some(z);
                eeff = fr.eeff.or(eeff);
                solver = "field";
            }
            if solver == "field" {
                flags &= !F_COPLANAR_IGNORED;
            }
        }
    }
    if solver == "formula" && kind != Kind::NoReference && flags & F_PLANE_OPENING != 0 && near_opening {
        flags |= F_FORMULA_ROUGH;
    }

    CrossSection {
        kind,
        layer,
        w,
        t,
        ref_up: up.map(|r| r.0 as u16),
        ref_dn: dn.map(|r| r.0 as u16),
        h_up: up.map(|r| r.1),
        h_dn: dn.map(|r| r.1),
        er_up,
        er_dn,
        s_left,
        s_right,
        diff_gap,
        partner_w: partner.map(|p| p.1),
        mask: mask.is_some(),
        z0,
        eeff,
        zdiff,
        zeven,
        eeff_odd,
        eeff_even,
        z0_formula,
        zdiff_formula,
        solver,
        geom: geom_idx,
        planes: planes.iter().map(|c| (c.layer as u16, c.left, c.right)).collect(),
        flags,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn targets() {
        let t = |s| target_from_class_name(s).map(|x| x.0);
        assert_eq!(t("50Z"), Some(Target { z: 50.0, diff: false }));
        assert_eq!(t("FE_100Z_Diff"), Some(Target { z: 100.0, diff: true }));
        assert_eq!(t("FE_50Z_INPUT"), Some(Target { z: 50.0, diff: false }));
        assert_eq!(t("LVDS_ADC"), Some(Target { z: 100.0, diff: true }));
        assert_eq!(t("PCIe"), Some(Target { z: 85.0, diff: true }));
        assert_eq!(t("USB_90R"), Some(Target { z: 90.0, diff: true }));
        assert_eq!(t("PWR"), None);
        assert_eq!(t("Default"), None);
        assert_eq!(t("3V3"), None);
    }
    /// A 2.75 mm launch on F.Cu over a cut-out in In1 (0.465 mm wide) that ends exactly where the
    /// launch track ends (the 3313 coupons): the reference is In2 along the whole track – the last
    /// sample, within 0.3 mm of the cut-out's end, was taken as a small antipad (reference In1).
    /// The line after it (over solid In1) passes a 0.5 mm square antipad: still referenced to In1.
    #[test]
    fn opening_ending_at_track_end() {
        let pcb = r#"(kicad_pcb (version 20241229) (generator "pcbnew")
  (layers (0 "F.Cu" signal) (4 "In1.Cu" signal) (6 "In2.Cu" signal) (2 "B.Cu" signal))
  (setup (stackup
    (layer "F.Cu" (type "copper") (thickness 0.035))
    (layer "dielectric 1" (type "prepreg") (thickness 0.0994) (material "3313") (epsilon_r 4.1) (loss_tangent 0.02))
    (layer "In1.Cu" (type "copper") (thickness 0.0175))
    (layer "dielectric 2" (type "core") (thickness 1.265) (material "FR4") (epsilon_r 4.6) (loss_tangent 0.02))
    (layer "In2.Cu" (type "copper") (thickness 0.0175))
    (layer "dielectric 3" (type "prepreg") (thickness 0.0994) (material "3313") (epsilon_r 4.1) (loss_tangent 0.02))
    (layer "B.Cu" (type "copper") (thickness 0.035))))
  (net 0 "") (net 1 "SIG") (net 2 "GND")
  (segment (start 0.05 0) (end 2.8 0) (width 0.255) (layer "F.Cu") (net 1))
  (segment (start 2.8 0) (end 10 0) (width 0.15) (layer "F.Cu") (net 1))
  (zone (net 2) (net_name "GND") (layer "In1.Cu")
    (filled_polygon (layer "In1.Cu") (pts (xy 0 -5) (xy 5.67 -5) (xy 5.67 0.3) (xy 0 0.3) (xy 0 0.2325) (xy 2.8 0.2325) (xy 2.8 -0.2325) (xy 0 -0.2325)))
    (filled_polygon (layer "In1.Cu") (pts (xy 0 0.29) (xy 5.67 0.29) (xy 5.67 5) (xy 0 5)))
    (filled_polygon (layer "In1.Cu") (pts (xy 5.6 -5) (xy 20 -5) (xy 20 -0.25) (xy 5.6 -0.25)))
    (filled_polygon (layer "In1.Cu") (pts (xy 5.6 0.25) (xy 20 0.25) (xy 20 5) (xy 5.6 5)))
    (filled_polygon (layer "In1.Cu") (pts (xy 6.17 -5) (xy 20 -5) (xy 20 5) (xy 6.17 5))))
  (zone (net 2) (net_name "GND") (layer "In2.Cu") (filled_polygon (layer "In2.Cu") (pts (xy -1 -5) (xy 20 -5) (xy 20 5) (xy -1 5)))))
"#;
        let board = crate::board::load_pcb(pcb).unwrap();
        let an = analyze(&board, &Params::default());
        let pieces: Vec<(usize, f64, f64, &CrossSection)> =
            an.pieces.chunks(4).map(|c| (c[0] as usize, c[1], c[2], &an.cross_sections[c[3] as usize])).collect();
        // the launch: reference In2 (index 2) over its whole length, In1 only as a cut-out
        let launch: Vec<_> = pieces.iter().filter(|p| p.0 == 0).collect();
        assert!(!launch.is_empty());
        for (_, t0, t1, xs) in &launch {
            assert_eq!((xs.ref_dn, xs.ref_up), (Some(2), None), "launch {t0:.3}–{t1:.3}: reference {:?}", xs.ref_dn);
            assert!(xs.flags & F_PLANE_OPENING != 0 && xs.flags & F_REF_PERFORATED == 0, "flags {:#x}", xs.flags);
            // the formula sees only In2 below: CPW-like and high, marked as a rough estimate
            assert!(
                xs.solver == "formula" && xs.flags & F_FORMULA_ROUGH != 0 && xs.z0.unwrap() > 55.0,
                "{:?} {:#x}",
                xs.z0,
                xs.flags
            );
        }
        // with a field-solver result for the launch's cross-section the mark goes away
        let gid = launch[0].3.geom.expect("launch geometry");
        let mut params = Params::default();
        params.field.insert(
            an.geometries[gid as usize].id.clone(),
            FieldResult { z0: Some(50.05), eeff: Some(2.9), ..Default::default() },
        );
        let an2 = analyze(&board, &params);
        let xs2 = &an2.cross_sections[an2.pieces[3] as usize];
        assert!(xs2.solver == "field" && xs2.flags & F_FORMULA_ROUGH == 0 && xs2.z0 == Some(50.05), "{:?}", xs2.z0);
        // the line: In1 everywhere, the antipad (sample at its centre, x = 5.92) is perforation
        let line: Vec<_> = pieces.iter().filter(|p| p.0 == 1).collect();
        assert!(
            line.iter().all(|p| p.3.ref_dn == Some(1)),
            "{:?}",
            line.iter().map(|p| (p.1, p.3.ref_dn)).collect::<Vec<_>>()
        );
        assert!(line.iter().any(|p| p.3.flags & F_REF_PERFORATED != 0), "the antipad is a small opening");
    }

    #[test]
    fn partners() {
        let nets: Vec<String> =
            ["", "/USB_D+", "/USB_D-", "CLK_P", "CLK_N", "VCCP"].iter().map(|s| s.to_string()).collect();
        let p = diff_partners(&nets);
        assert_eq!(p[1], Some(2));
        assert_eq!(p[4], Some(3));
        assert_eq!(p[5], None);
    }
}

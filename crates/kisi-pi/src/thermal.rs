//! Temperature estimate (steady state) from the losses of the power analysis.
//!
//! The board is a stack of 2D sheets, one per copper layer, on a uniform square grid over the
//! board outline – the building blocks of the DC voltage-drop solver (`pi`: rasterisation,
//! conductances between neighbouring cells, vias between layers, a sparse Cholesky solve):
//!
//! * in plane per layer: k_Cu·t·f (f = copper fill of the cell, all nets, from a finer raster)
//!   plus the laminate in plane (half of the dielectric above and below);
//! * between layers: the laminate through the thickness (k_z·c²/h) and the barrels of vias and
//!   plated holes (k_Cu·A/Δz);
//! * to the air: convection (natural or forced, one mean coefficient per side from the board's
//!   mean temperature rise) and linearised radiation on the outer layers, the solder mask in series;
//! * sources: the copper loss of every DC cell (rails and ground return) on its layer, and per part
//!   a junction node (JEDEC two-resistor model): θJB to the cells under its pads, θJC plus the
//!   top surface to the air. Only ψJB: heat into the pads, Tj = T_board + ψJB·P; only θJA: heat
//!   into the pads, Tj = T_amb + θJA·P (JEDEC board, flagged).
//!
//! Material constants and correlations come from `data/thermal.json`, package defaults from
//! `data/components.json` (`thermal`), per-part data from the footprint field `PI_THERMAL`.
//! `rail_at_temperature` solves a rail again with the copper resistance at the estimated
//! temperature (one electro-thermal step). Concept and validation: docs/research/thermal.md.

use crate::board::{Board, Footprint, Pad};
use crate::geom::{BBox, P};
use crate::losses::CompLoss;
use crate::pi::{fill_polygon, fill_track, pad_cells, Grid, PiParams, PiResult, RailSetup};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::OnceLock;

const THERMAL_JSON: &str = include_str!("../../../data/thermal.json");
const COMPONENTS_JSON: &str = include_str!("../../../data/components.json");
const SIGMA: f64 = 5.670374419e-8;

// --- data (data/thermal.json, data/components.json "thermal") ----------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct Data {
    pub copper: Copper,
    pub laminate: Laminate,
    pub solder_mask: Mask,
    pub air: Air,
    pub convection: Convection,
    pub defaults: Defaults,
}
#[derive(Debug, Clone, Deserialize)]
pub struct Copper {
    pub k_w_mk: f64,
    pub alpha_per_k: f64,
    pub rho_20c_ohm_m: f64,
}
#[derive(Debug, Clone, Deserialize)]
pub struct Laminate {
    pub k_inplane_w_mk: f64,
    pub k_through_w_mk: f64,
    pub mot_c: f64,
}
#[derive(Debug, Clone, Deserialize)]
pub struct Mask {
    pub k_w_mk: f64,
    pub thickness_mm: f64,
    pub emissivity: f64,
}
#[derive(Debug, Clone, Deserialize)]
pub struct Air {
    pub k_w_mk: f64,
    pub nu_m2_s: f64,
    pub pr: f64,
}
#[derive(Debug, Clone, Deserialize)]
pub struct Coef {
    pub c: f64,
    #[serde(default)]
    pub n: f64,
    #[serde(default)]
    pub ra_turbulent: Option<f64>,
    #[serde(default)]
    pub c_turbulent: f64,
    #[serde(default)]
    pub n_turbulent: f64,
}
#[derive(Debug, Clone, Deserialize)]
pub struct Convection {
    pub horizontal_up: Coef,
    pub horizontal_down: Coef,
    pub forced_laminar: Coef,
}
#[derive(Debug, Clone, Deserialize)]
pub struct Defaults {
    pub ambient_c: f64,
    pub airflow_m_s: f64,
    pub max_nodes: usize,
    pub min_cell_mm: f64,
    pub via_plating_um: f64,
}

pub fn data() -> &'static Data {
    static D: OnceLock<Data> = OnceLock::new();
    D.get_or_init(|| serde_json::from_str(THERMAL_JSON).expect("data/thermal.json"))
}

/// Package family defaults: θ = θ_ref·(A_ref/A)^exp, A = pad bounding box (mm²).
#[derive(Debug, Clone, Deserialize)]
pub struct Family {
    pub family: String,
    #[serde(rename = "match")]
    pub patterns: Vec<String>,
    pub ref_area_mm2: f64,
    pub theta_jb: f64,
    pub exp_jb: f64,
    pub theta_jc: Option<f64>,
    pub exp_jc: f64,
    pub source: String,
}
#[derive(Debug, Clone, Deserialize)]
pub struct PackageData {
    pub tj_max_c: f64,
    pub tj_max_note: String,
    pub families: Vec<Family>,
}

pub fn packages() -> &'static PackageData {
    static D: OnceLock<PackageData> = OnceLock::new();
    D.get_or_init(|| {
        #[derive(Deserialize)]
        struct Wrap {
            thermal: PackageData,
        }
        serde_json::from_str::<Wrap>(COMPONENTS_JSON).expect("data/components.json: thermal").thermal
    })
}

// --- the PI_THERMAL field ----------------------------------------------------------------------

/// Thermal data of a part from its PI_THERMAL field:
/// `theta_jb=17.4 theta_jc=53.6 tj_max=125`, `psi_jb=11.9`, `theta_ja=45`, `p=0.5W`.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ThermalField {
    pub theta_jb: Option<f64>,
    pub theta_jc: Option<f64>,
    pub psi_jb: Option<f64>,
    pub theta_ja: Option<f64>,
    pub tj_max: Option<f64>,
    /// heat dissipated in the part (W), replaces the power tree's value
    pub p: Option<f64>,
}

pub const PI_THERMAL_KEYS: &str = "theta_jb, theta_jc, psi_jb, theta_ja (K/W), tj_max (°C), p (W)";

pub fn parse_pi_thermal(s: &str) -> Result<ThermalField, String> {
    let mut f = ThermalField::default();
    let num = |v: &str, units: &[&str]| -> Option<f64> {
        let mut t = v.trim();
        for u in units {
            if let Some(x) = t.strip_suffix(u) {
                t = x.trim();
                break;
            }
        }
        t.parse::<f64>().ok().filter(|x| x.is_finite())
    };
    for tok in s.split(|c: char| c.is_whitespace() || c == ',' || c == ';').filter(|t| !t.is_empty()) {
        let bad = || format!("PI_THERMAL: cannot read '{tok}' (form theta_jb=17.4 theta_jc=53.6 tj_max=125; keys {PI_THERMAL_KEYS})");
        let (k, v) = tok.split_once('=').ok_or_else(bad)?;
        let k = k.trim().to_ascii_lowercase();
        match k.as_str() {
            "theta_jb" | "theta_jc" | "psi_jb" | "theta_ja" => {
                let x = num(v, &["°C/W", "C/W", "K/W"]).filter(|x| *x > 0.0).ok_or_else(bad)?;
                *match k.as_str() {
                    "theta_jb" => &mut f.theta_jb,
                    "theta_jc" => &mut f.theta_jc,
                    "psi_jb" => &mut f.psi_jb,
                    _ => &mut f.theta_ja,
                } = Some(x);
            }
            "tj_max" => f.tj_max = Some(num(v, &["°C", "C"]).filter(|x| *x > -60.0 && *x < 400.0).ok_or_else(bad)?),
            "p" => {
                let x = match crate::pi::parse_quantity(v) {
                    Some((w, 'W')) => w,
                    Some((w, 'A')) if !v.trim().ends_with(['A', 'a']) => w, // a bare number
                    _ => return Err(bad()),
                };
                if x < 0.0 {
                    return Err(bad());
                }
                f.p = Some(x);
            }
            _ => return Err(format!("PI_THERMAL: unknown key '{k}' (known: {PI_THERMAL_KEYS})")),
        }
    }
    Ok(f)
}

// --- parameters and results --------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ThermalParams {
    /// ambient air temperature (°C); None: the default of data/thermal.json (25 °C)
    pub ambient_c: Option<f64>,
    /// surface coefficient per side, convection and radiation together (W/m²K); None: estimated
    pub h: Option<f64>,
    /// forced air flow along the board (m/s); 0: natural convection
    pub airflow_m_s: f64,
    /// board stands vertical (natural convection as a vertical plate on both sides)
    pub vertical: bool,
    /// cell size (mm); None: from the board size and `max_nodes`
    pub cell: Option<f64>,
    pub max_nodes: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Cooling {
    /// "horizontal" | "vertical"
    pub orientation: String,
    pub airflow_m_s: f64,
    /// W/m²K per side, convection + radiation
    pub h_top: f64,
    pub h_bottom: f64,
    pub h_conv_top: f64,
    pub h_conv_bottom: f64,
    pub h_rad: f64,
    /// false when `h` was given
    pub estimated: bool,
    /// mean surface temperature rise the coefficients were taken at (K)
    pub dt_mean_k: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct LayerTemp {
    pub layer: u16,
    pub name: String,
    pub t_max_c: f64,
    pub t_mean_c: f64,
    pub max_at: Option<P>,
}

#[derive(Debug, Clone, Serialize)]
pub struct HotSpot {
    pub at: P,
    pub layer: u16,
    pub t_c: f64,
    /// nearest part (within a few mm)
    pub near: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PartTemp {
    pub refdes: String,
    pub part: String,
    /// heat in the part (W) and where it came from: "power tree" | "PI_THERMAL p"
    pub w: f64,
    pub w_source: String,
    /// centre of the pads
    pub at: Option<P>,
    /// "top" | "bottom"
    pub side: String,
    /// package family of the defaults (qfn, so, …) and the pad bounding box (mm²)
    pub family: String,
    pub area_mm2: f64,
    /// "theta_jb" (two-resistor network), "psi_jb", "theta_ja" (JEDEC board, flagged)
    pub model: String,
    pub theta_jb: Option<f64>,
    pub theta_jc: Option<f64>,
    pub psi_jb: Option<f64>,
    pub theta_ja: Option<f64>,
    /// "PI_THERMAL (datasheet: …)" or "package default <family> (estimate)"
    pub data_source: String,
    pub estimate: bool,
    /// board temperature under the part (pad cells, weighted by pad area) and the junction
    pub t_board_c: f64,
    pub tj_c: f64,
    /// share of the heat that goes into the board (two-resistor network)
    pub to_board: Option<f64>,
    pub tj_max_c: f64,
    pub tj_max_source: String,
    pub margin_k: f64,
    /// "ok" | "near" (within 10 K) | "over"
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct RailThermal {
    pub net: String,
    pub ground: bool,
    /// temperature of the copper that carries the rail's current: maximum and loss-weighted mean
    pub t_max_c: Option<f64>,
    pub t_mean_c: Option<f64>,
    /// largest drop (ground: offset) and copper loss at 20 °C and at the estimated temperature
    pub drop_mv_20c: Option<f64>,
    pub drop_mv_hot: Option<f64>,
    pub loss_w_20c: f64,
    pub loss_w_hot: Option<f64>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Balance {
    /// heat into the board (parts and copper) and out by convection and radiation (W)
    pub in_w: f64,
    pub convection_w: f64,
    pub radiation_w: f64,
    /// |in − out| / in
    pub error_rel: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ThermalResult {
    pub ambient_c: f64,
    pub cooling: Cooling,
    pub cell: f64,
    pub x0: f64,
    pub y0: f64,
    pub nx: usize,
    pub ny: usize,
    /// one plane per copper layer (index = copper layer) in `t`
    pub layers: Vec<LayerTemp>,
    pub nodes: usize,
    pub area_mm2: f64,
    pub parts_w: f64,
    pub copper_w: f64,
    pub balance: Balance,
    pub t_max_c: f64,
    pub t_max_at: Option<P>,
    pub t_max_layer: u16,
    pub hot_spots: Vec<HotSpot>,
    /// sorted by junction temperature, hottest first
    pub parts: Vec<PartTemp>,
    /// copper temperature and IR drop at temperature per rail (`rail_at_temperature`)
    pub rails: Vec<RailThermal>,
    pub warnings: Vec<String>,
    pub notes: Vec<String>,
    /// °C, `layers.len()` planes of nx·ny cells, NaN outside the board
    #[serde(skip)]
    pub t: Vec<f32>,
}

impl ThermalResult {
    /// Temperature (°C) at a point on a copper layer; ambient outside the board.
    pub fn temp_at(&self, layer: usize, p: P) -> f64 {
        let i = ((p[0] - self.x0) / self.cell).floor();
        let j = ((p[1] - self.y0) / self.cell).floor();
        if layer >= self.layers.len() || i < 0.0 || j < 0.0 || i as usize >= self.nx || j as usize >= self.ny {
            return self.ambient_c;
        }
        let v = self.t[layer * self.nx * self.ny + j as usize * self.nx + i as usize];
        if v.is_finite() { v as f64 } else { self.ambient_c }
    }
}

// --- board outline -----------------------------------------------------------------------------

/// Closed loops of the board outline: Edge.Cuts pieces (lines, arcs, polygons) chained at
/// matching ends (0.01 mm). Open chains are dropped.
pub fn outline_loops(board: &Board) -> Vec<Vec<P>> {
    let tol = 0.01;
    let close = |a: P, b: P| (a[0] - b[0]).abs() <= tol && (a[1] - b[1]).abs() <= tol;
    let mut pieces: Vec<Vec<P>> = board.outline.iter().filter(|p| p.len() >= 2).cloned().collect();
    let mut loops = Vec::new();
    while let Some(mut cur) = pieces.pop() {
        loop {
            let (s, e) = (cur[0], *cur.last().unwrap());
            if cur.len() > 2 && close(s, e) {
                cur.pop();
                if cur.len() >= 3 {
                    loops.push(cur);
                }
                break;
            }
            let Some(k) = pieces.iter().position(|q| close(q[0], e) || close(*q.last().unwrap(), e)) else { break };
            let mut q = pieces.swap_remove(k);
            if !close(q[0], e) {
                q.reverse();
            }
            cur.extend_from_slice(&q[1..]);
        }
    }
    loops
}

fn polygon_area(p: &[P]) -> f64 {
    let n = p.len();
    (0..n).map(|i| p[i][0] * p[(i + 1) % n][1] - p[(i + 1) % n][0] * p[i][1]).sum::<f64>().abs() / 2.0
}
fn perimeter(p: &[P]) -> f64 {
    let n = p.len();
    (0..n).map(|i| ((p[(i + 1) % n][0] - p[i][0]).powi(2) + (p[(i + 1) % n][1] - p[i][1]).powi(2)).sqrt()).sum()
}

// --- package thermal data ----------------------------------------------------------------------

/// Family, θJB, θJC and pad bounding box area (mm²) of a footprint from the package defaults.
pub fn package_defaults(board: &Board, fi: usize) -> (String, f64, Option<f64>, f64, String) {
    let pk = packages();
    let fp = &board.footprints[fi];
    let pads: Vec<&Pad> = board.pads.iter().filter(|p| p.fp as usize == fi).collect();
    let mut bb = BBox::empty();
    for p in &pads {
        let r = [p.size[0] / 2.0, p.size[1] / 2.0];
        let (s, c) = (p.angle.to_radians().sin().abs(), p.angle.to_radians().cos().abs());
        let (hx, hy) = (r[0] * c + r[1] * s, r[0] * s + r[1] * c);
        bb.add([p.at[0] - hx, p.at[1] - hy]);
        bb.add([p.at[0] + hx, p.at[1] + hy]);
    }
    let area = if bb.is_valid() { ((bb.max[0] - bb.min[0]) * (bb.max[1] - bb.min[1])).max(0.1) } else { 1.0 };
    let name = fp.lib.rsplit(':').next().unwrap_or(&fp.lib).to_ascii_uppercase();
    let by_name = pk.families.iter().find(|f| f.patterns.iter().any(|m| name.contains(&m.to_ascii_uppercase())));
    let fam = by_name.or_else(|| {
        // pad geometry: an exposed centre pad, a grid of round pads, two pads
        let key = if pads.len() == 2 {
            "chip"
        } else if pads.len() >= 5 && bb.is_valid() {
            let c = [(bb.min[0] + bb.max[0]) / 2.0, (bb.min[1] + bb.max[1]) / 2.0];
            let big = pads.iter().any(|p| p.size[0] * p.size[1] > 0.15 * area && (p.at[0] - c[0]).abs() < 0.2 * (bb.max[0] - bb.min[0]) && (p.at[1] - c[1]).abs() < 0.2 * (bb.max[1] - bb.min[1]));
            let round = pads.len() >= 16 && pads.iter().all(|p| p.shape == "circle" && p.drill == 0.0);
            if big { "qfn" } else if round { "bga" } else { "generic" }
        } else {
            "generic"
        };
        pk.families.iter().find(|f| f.family == key)
    });
    let Some(f) = fam.or_else(|| pk.families.last()) else { return ("generic".into(), 40.0, None, area, "kisi estimate".into()) };
    let ratio = (f.ref_area_mm2 / area).max(1e-3);
    let jb = f.theta_jb * ratio.powf(f.exp_jb);
    let jc = f.theta_jc.map(|t| t * ratio.powf(f.exp_jc));
    (f.family.clone(), jb, jc, area, f.source.clone())
}

// --- the model ---------------------------------------------------------------------------------

struct Mesh {
    g: Grid,
    nl: usize,
    /// grid cell → compact cell index (u32::MAX: outside the board)
    idx: Vec<u32>,
    /// compact → grid cell
    cells: Vec<usize>,
    /// in-plane sheet conductance per layer and compact cell (W/K per square)
    kxy: Vec<Vec<f64>>,
    /// vertical conductance per dielectric (between layer l and l+1) and compact cell (W/K)
    gz: Vec<Vec<f64>>,
    area_m2: f64,
    perim_m: f64,
    long_m: f64,
    /// mask resistance per area on top / bottom (m²K/W)
    r_mask: [f64; 2],
}

impl Mesh {
    fn node(&self, l: usize, c: usize) -> usize {
        l * self.cells.len() + c
    }
    /// compact cell of a point (None outside)
    fn cell_at(&self, p: P) -> Option<usize> {
        let (i, j) = self.g.cell_of(p)?;
        let k = self.idx[j * self.g.nx + i];
        (k != u32::MAX).then_some(k as usize)
    }
    /// compact cell of a point, or the nearest one inside the board within `r` mm
    fn cell_near(&self, p: P, r: f64) -> Option<usize> {
        if let Some(c) = self.cell_at(p) {
            return Some(c);
        }
        let ci = ((p[0] - self.g.x0) / self.g.cell).floor() as isize;
        let cj = ((p[1] - self.g.y0) / self.g.cell).floor() as isize;
        let rc = (r / self.g.cell).ceil() as isize;
        let mut best: Option<(f64, usize)> = None;
        for dj in -rc..=rc {
            for di in -rc..=rc {
                let (i, j) = (ci + di, cj + dj);
                if i < 0 || j < 0 || i as usize >= self.g.nx || j as usize >= self.g.ny {
                    continue;
                }
                let k = self.idx[j as usize * self.g.nx + i as usize];
                if k == u32::MAX {
                    continue;
                }
                let c = self.g.center(i as usize, j as usize);
                let d = (c[0] - p[0]).hypot(c[1] - p[1]);
                if best.is_none_or(|b| d < b.0) {
                    best = Some((d, k as usize));
                }
            }
        }
        best.map(|b| b.1)
    }
}

/// A plated hole (via or pad) is a thermal conductor; a pad no larger than its drill is an
/// unplated hole (no copper).
fn plated(p: &Pad) -> bool {
    p.drill > 0.0 && p.size[0].max(p.size[1]) > p.drill + 1e-6
}

fn build_mesh(board: &Board, prm: &ThermalParams, warnings: &mut Vec<String>) -> Result<Mesh, String> {
    let d = data();
    let su = board.stackup();
    let nl = su.n().max(1);
    let mut loops = outline_loops(board);
    let mut bb = BBox::empty();
    for l in &loops {
        for &p in l {
            bb.add(p);
        }
    }
    // an outline drawn partly in footprints leaves only cut-outs closed: if copper reaches more than
    // 1 mm beyond the closed loops, take the copper's bounding box instead
    let mut cu = BBox::empty();
    for p in &board.pads {
        cu.add(p.at);
    }
    for z in &board.zones {
        cu.add(z.bbox.min);
        cu.add(z.bbox.max);
    }
    for t in board.tracks.iter().filter(|t| !t.pad) {
        cu.add(t.a);
        cu.add(t.b);
    }
    if !loops.is_empty() && cu.is_valid() && (cu.min[0] < bb.min[0] - 1.0 || cu.min[1] < bb.min[1] - 1.0 || cu.max[0] > bb.max[0] + 1.0 || cu.max[1] > bb.max[1] + 1.0) {
        warnings.push("the closed board outline (Edge.Cuts) does not contain all copper – parts of it may be drawn in footprints: the bounding box of the copper is taken as the board".into());
        loops.clear();
        bb = BBox::empty();
    }
    let (area_mm2, perim_mm) = if loops.is_empty() {
        if !bb.is_valid() {
            bb = if cu.is_valid() { cu } else { board.bbox.ok_or("board without outline or copper")? };
            if warnings.is_empty() {
                warnings.push("no closed board outline (Edge.Cuts): the bounding box of the copper is taken as the board".into());
            }
        }
        let (w, h) = (bb.max[0] - bb.min[0], bb.max[1] - bb.min[1]);
        (w * h, 2.0 * (w + h))
    } else {
        // the largest loop is the outline, the others cut-outs
        let mut a: Vec<(f64, f64)> = loops.iter().map(|l| (polygon_area(l), perimeter(l))).collect();
        a.sort_by(|x, y| y.0.total_cmp(&x.0));
        (a[0].0 - a[1..].iter().map(|x| x.0).sum::<f64>(), a[0].1)
    };
    if !bb.is_valid() || area_mm2 <= 0.0 {
        return Err("board outline has no area".into());
    }
    let max_nodes = prm.max_nodes.unwrap_or(d.defaults.max_nodes).max(1000);
    let cell = prm.cell.unwrap_or_else(|| {
        let c = (area_mm2 * nl as f64 / max_nodes as f64).sqrt().max(d.defaults.min_cell_mm);
        [0.25, 0.3, 0.4, 0.5, 0.6, 0.75, 1.0, 1.25, 1.5, 2.0, 2.5, 3.0, 4.0, 5.0].into_iter().find(|&v| v >= c).unwrap_or(c)
    });
    let g = Grid {
        x0: bb.min[0],
        y0: bb.min[1],
        cell,
        nx: (((bb.max[0] - bb.min[0]) / cell).ceil() as usize).max(1),
        ny: (((bb.max[1] - bb.min[1]) / cell).ceil() as usize).max(1),
    };
    let per = g.nx * g.ny;
    // inside: even-odd over all loops
    let mut inside = vec![false; per];
    if loops.is_empty() {
        inside.iter_mut().for_each(|x| *x = true);
    } else {
        let mut w = vec![0f32; per];
        for l in &loops {
            w.iter_mut().for_each(|x| *x = 0.0);
            fill_polygon(&g, l, &mut w);
            for k in 0..per {
                if w[k] > 0.0 {
                    inside[k] = !inside[k];
                }
            }
        }
    }
    let mut idx = vec![u32::MAX; per];
    let mut cells = Vec::new();
    for k in 0..per {
        if inside[k] {
            idx[k] = cells.len() as u32;
            cells.push(k);
        }
    }
    if cells.is_empty() {
        return Err("no grid cell inside the board outline".into());
    }
    let nc = cells.len();
    // copper fill per layer from a finer raster (≈ 0.1 mm)
    let mut s = ((cell / 0.1).ceil() as usize).clamp(1, 8);
    while s > 1 && per * s * s > 4_000_000 {
        s -= 1;
    }
    let gf = Grid { x0: g.x0, y0: g.y0, cell: cell / s as f64, nx: g.nx * s, ny: g.ny * s };
    let (kcu, kxy_l, kz_l) = (d.copper.k_w_mk, d.laminate.k_inplane_w_mk, d.laminate.k_through_w_mk);
    let diel = |k: usize| su.dielectrics.get(k).map(|x| x.thickness).unwrap_or(0.0);
    let mut kxy = Vec::with_capacity(nl);
    let mut wf = vec![0f32; gf.nx * gf.ny];
    for l in 0..nl {
        wf.iter_mut().for_each(|x| *x = 0.0);
        for z in board.zones.iter().filter(|z| z.layer as usize == l) {
            fill_polygon(&gf, &z.pts, &mut wf);
        }
        for t in board.tracks.iter().filter(|t| t.layer as usize == l && !t.pad) {
            fill_track(&gf, t.a, t.b, t.width, &mut wf);
        }
        for p in board.pads.iter().filter(|p| p.layers.contains(&(l as u16)) && (p.drill == 0.0 || plated(p))) {
            for k in pad_cells(&gf, p) {
                wf[k] = 1.0;
            }
        }
        let t_cu = su.copper.get(l).map(|c| c.thickness).unwrap_or(0.035) * 1e-3;
        // laminate share of the layer node: half of the dielectric above and below (m)
        let t_lam = (if l > 0 { diel(l - 1) } else { 0.0 } + diel(l)) / 2.0 * 1e-3 + if nl == 1 { 1.5e-3 } else { 0.0 };
        let mut kl = vec![0.0; nc];
        for (c, &k) in cells.iter().enumerate() {
            let (i, j) = (k % g.nx, k / g.nx);
            let mut sum = 0.0;
            for jj in 0..s {
                let row = (j * s + jj) * gf.nx + i * s;
                for ii in 0..s {
                    sum += wf[row + ii] as f64;
                }
            }
            let f = (sum / (s * s) as f64).min(1.0);
            kl[c] = kcu * t_cu * f + kxy_l * (t_cu * (1.0 - f) + t_lam);
        }
        kxy.push(kl);
    }
    // between layers: the dielectric and the barrels of vias and plated holes
    let a_cell = (cell * 1e-3).powi(2);
    let mut gz: Vec<Vec<f64>> = (0..nl.saturating_sub(1)).map(|l| vec![kz_l * a_cell / (diel(l).max(1e-3) * 1e-3); nc]).collect();
    let layer_z: Vec<f64> = {
        let mut z = vec![0.0; nl];
        let mut acc = 0.0;
        for (k, zk) in z.iter_mut().enumerate() {
            let t = su.copper.get(k).map(|c| c.thickness).unwrap_or(0.035);
            *zk = acc + t / 2.0;
            acc += t + diel(k);
        }
        z
    };
    let tp = d.defaults.via_plating_um * 1e-6;
    let barrel = |drill: f64| {
        let r = drill / 2.0 * 1e-3;
        let t = tp.min(r);
        std::f64::consts::PI * (r * r - (r - t) * (r - t))
    };
    let add_barrel = |at: P, drill: f64, lo: usize, hi: usize, gz: &mut Vec<Vec<f64>>, idx: &Vec<u32>| {
        let Some((i, j)) = g.cell_of(at) else { return };
        let c = idx[j * g.nx + i];
        if c == u32::MAX {
            return;
        }
        for l in lo..hi.min(nl - 1) {
            let dz = ((layer_z[l + 1] - layer_z[l]) * 1e-3).max(1e-6);
            gz[l][c as usize] += kcu * barrel(drill) / dz;
        }
    };
    for v in &board.vias {
        let (lo, hi) = (v.from.min(v.to) as usize, v.from.max(v.to) as usize);
        add_barrel(v.at, v.drill, lo, hi, &mut gz, &idx);
    }
    for p in board.pads.iter().filter(|p| plated(p)) {
        let (lo, hi) = (p.layers.iter().min().copied().unwrap_or(0) as usize, p.layers.iter().max().copied().unwrap_or(0) as usize);
        add_barrel(p.at, p.drill, lo, hi, &mut gz, &idx);
    }
    let mask_r = |m: &Option<crate::board::SolderMask>| m.as_ref().map(|m| m.thickness).unwrap_or(d.solder_mask.thickness_mm) * 1e-3 / d.solder_mask.k_w_mk;
    let (w, h) = (bb.max[0] - bb.min[0], bb.max[1] - bb.min[1]);
    Ok(Mesh {
        g,
        nl,
        idx,
        cells,
        kxy,
        gz,
        area_m2: area_mm2 * 1e-6,
        perim_m: perim_mm * 1e-3,
        long_m: w.max(h) * 1e-3,
        r_mask: [mask_r(&su.mask_top), mask_r(&su.mask_bottom)],
    })
}

/// Surface coefficients (convection top, convection bottom, radiation) in W/m²K at a mean
/// surface rise `dt` over `ta` (K).
pub fn surface_coefficients(prm: &ThermalParams, area_m2: f64, perim_m: f64, long_m: f64, dt: f64, ta_k: f64) -> (f64, f64, f64) {
    let d = data();
    let air = &d.air;
    let dt = dt.max(0.5);
    let beta = 1.0 / (ta_k + dt / 2.0);
    let alpha_air = air.nu_m2_s / air.pr;
    let ra = |l: f64| 9.81 * beta * dt * l.powi(3) / (air.nu_m2_s * alpha_air);
    let (mut up, mut down) = if prm.vertical {
        let l = long_m.max(1e-3);
        let r = ra(l);
        let nu = (0.825 + 0.387 * r.powf(1.0 / 6.0) / (1.0 + (0.492 / air.pr).powf(9.0 / 16.0)).powf(8.0 / 27.0)).powi(2);
        (nu * air.k_w_mk / l, nu * air.k_w_mk / l)
    } else {
        let l = (area_m2 / perim_m.max(1e-6)).max(1e-4);
        let r = ra(l);
        let cu = &d.convection.horizontal_up;
        let nu_up = if cu.ra_turbulent.is_some_and(|rt| r > rt) { cu.c_turbulent * r.powf(cu.n_turbulent) } else { cu.c * r.powf(cu.n) };
        let cd = &d.convection.horizontal_down;
        (nu_up * air.k_w_mk / l, cd.c * r.powf(cd.n) * air.k_w_mk / l)
    };
    if prm.airflow_m_s > 0.0 {
        let l = long_m.max(1e-3);
        let re = prm.airflow_m_s * l / air.nu_m2_s;
        let hf = d.convection.forced_laminar.c * re.sqrt() * air.pr.powf(1.0 / 3.0) * air.k_w_mk / l;
        up = up.max(hf);
        down = down.max(hf);
    }
    let ts = ta_k + dt;
    let hr = d.solder_mask.emissivity * SIGMA * (ts * ts + ta_k * ta_k) * (ts + ta_k);
    (up, down, hr)
}

/// A heat source on a part: its node connections.
enum PartModel {
    /// junction node: θJB to the pad cells, top path to the air (W/K)
    Network { g_jb: f64, g_top: f64 },
    /// heat into the pad cells, Tj = T_board + ψJB·P
    Psi(f64),
    /// heat into the pad cells, Tj = T_amb + θJA·P
    Ja(f64),
}

struct Part {
    res: PartTemp,
    /// (node, weight), weights sum to 1
    cells: Vec<(usize, f64)>,
    model: PartModel,
}

/// Parts with heat: the losses' parts (power tree), replaced or added by `PI_THERMAL p=…`.
fn heat_parts(board: &Board, comps: &[CompLoss], warnings: &mut Vec<String>) -> Vec<(usize, f64, String, ThermalField, Option<(String, String)>)> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let field = |fp: &Footprint, warnings: &mut Vec<String>| -> ThermalField {
        match fp.prop("PI_THERMAL").map(parse_pi_thermal) {
            Some(Ok(f)) => f,
            Some(Err(e)) => {
                warnings.push(format!("{}: {e}", fp.reference));
                ThermalField::default()
            }
            None => ThermalField::default(),
        }
    };
    let src = |fp: &Footprint| crate::tree::field_source(fp, "PI_THERMAL").map(|(k, n)| (k.to_string(), n.to_string()));
    for c in comps {
        let Some(fi) = board.footprints.iter().position(|f| f.reference == c.refdes) else { continue };
        let f = field(&board.footprints[fi], warnings);
        let (w, ws) = match f.p {
            Some(p) => (p, "PI_THERMAL p".to_string()),
            None => (c.w, "power tree".to_string()),
        };
        seen.insert(fi);
        out.push((fi, w, ws, f, src(&board.footprints[fi])));
    }
    for (fi, fp) in board.footprints.iter().enumerate() {
        if seen.contains(&fi) || fp.dnp {
            continue;
        }
        if fp.prop("PI_THERMAL").is_none() {
            continue;
        }
        let f = field(fp, warnings);
        if let Some(p) = f.p {
            out.push((fi, p, "PI_THERMAL p".to_string(), f, src(fp)));
        }
    }
    out
}

/// Steady-state temperature of the board from the DC results (copper loss per cell) and the heat
/// of the parts (`losses::loss_summary(..).components`).
pub fn solve(board: &Board, rails: &[&PiResult], comps: &[CompLoss], prm: &ThermalParams) -> Result<ThermalResult, String> {
    let d = data();
    let pk = packages();
    let su = board.stackup();
    let mut warnings = Vec::new();
    let mut notes = Vec::new();
    let m = build_mesh(board, prm, &mut warnings)?;
    let (nl, nc) = (m.nl, m.cells.len());
    let nb = nl * nc;
    let ta = prm.ambient_c.unwrap_or(d.defaults.ambient_c);
    let ta_k = ta + 273.15;
    let a_cell = (m.g.cell * 1e-3).powi(2);

    // sources: copper loss per DC cell into the thermal cell under its centre
    let mut q = vec![0f64; nb];
    let (mut copper_w, mut unplaced) = (0.0, 0.0);
    for r in rails {
        for lg in &r.layers {
            let l = lg.layer as usize;
            for j in 0..lg.ny {
                for i in 0..lg.nx {
                    let w = r.loss[lg.offset + j * lg.nx + i] as f64;
                    if w <= 0.0 {
                        continue;
                    }
                    copper_w += w;
                    let c = [lg.x0 + (i as f64 + 0.5) * r.cell, lg.y0 + (j as f64 + 0.5) * r.cell];
                    match (l < nl).then(|| m.cell_near(c, 2.0 * m.g.cell)).flatten() {
                        Some(k) => q[m.node(l, k)] += w,
                        None => unplaced += w,
                    }
                }
            }
        }
    }
    if unplaced > 1e-6 {
        warnings.push(format!("{:.1} mW of copper loss lie outside the board outline and are left out", unplaced * 1e3));
    }

    // parts
    let mut parts: Vec<Part> = Vec::new();
    let mut parts_w = 0.0;
    for (fi, w, w_source, f, fsrc) in heat_parts(board, comps, &mut warnings) {
        if w <= 0.0 {
            continue;
        }
        let fp = &board.footprints[fi];
        let pads: Vec<&Pad> = board.pads.iter().filter(|p| p.fp as usize == fi).collect();
        let last = (nl - 1) as u16;
        let smd_bottom = pads.iter().any(|p| p.drill == 0.0 && p.layers == [last]) && !pads.iter().any(|p| p.drill == 0.0 && p.layers.contains(&0));
        let (side_l, side) = if smd_bottom && nl > 1 { (nl - 1, "bottom") } else { (0, "top") };
        let mut wsum: HashMap<usize, f64> = HashMap::new();
        for p in pads.iter().filter(|p| p.layers.contains(&(side_l as u16))) {
            let mut ks: Vec<usize> = pad_cells(&m.g, p).into_iter().filter_map(|k| (m.idx[k] != u32::MAX).then_some(m.idx[k] as usize)).collect();
            if ks.is_empty() {
                ks.extend(m.cell_near(p.at, 3.0 * m.g.cell));
            }
            let a = (p.size[0] * p.size[1]).max(1e-4) / ks.len().max(1) as f64;
            for k in ks {
                *wsum.entry(m.node(side_l, k)).or_default() += a;
            }
        }
        if wsum.is_empty() {
            match m.cell_near(fp.at, 5.0) {
                Some(k) => {
                    wsum.insert(m.node(side_l, k), 1.0);
                }
                None => {
                    warnings.push(format!("{}: outside the board outline, its {:.0} mW are left out", fp.reference, w * 1e3));
                    continue;
                }
            }
        }
        let tot: f64 = wsum.values().sum();
        let mut pb = BBox::empty();
        for p in &pads {
            pb.add(p.at);
        }
        let at = if pb.is_valid() { Some([(pb.min[0] + pb.max[0]) / 2.0, (pb.min[1] + pb.max[1]) / 2.0]) } else { Some(fp.at) };
        let mut cells: Vec<(usize, f64)> = wsum.into_iter().map(|(n, a)| (n, a / tot)).collect();
        cells.sort_by_key(|c| c.0);
        let (family, def_jb, def_jc, area, _) = package_defaults(board, fi);
        let field_src = match &fsrc {
            Some((k, n)) if !n.is_empty() => format!("PI_THERMAL ({k}: {n})"),
            Some((k, _)) => format!("PI_THERMAL ({k})"),
            None => "PI_THERMAL".to_string(),
        };
        let (model, theta_jb, theta_jc, psi_jb, theta_ja, data_source, estimate) = if let Some(jb) = f.theta_jb {
            (PartModel::Network { g_jb: 1.0 / jb, g_top: 0.0 }, Some(jb), f.theta_jc, None, None, field_src.clone(), false)
        } else if let Some(psi) = f.psi_jb {
            (PartModel::Psi(psi), None, None, Some(psi), None, field_src.clone(), false)
        } else if let Some(ja) = f.theta_ja {
            (PartModel::Ja(ja), None, None, None, Some(ja), format!("{field_src}, θJA of the JEDEC test board – not this board"), true)
        } else {
            (PartModel::Network { g_jb: 1.0 / def_jb, g_top: 0.0 }, Some(def_jb), def_jc, None, None, format!("package default {family} (estimate)"), true)
        };
        let (tj_max, tj_src) = match f.tj_max {
            Some(t) => (t, field_src.clone()),
            None => (pk.tj_max_c, "default (assumption)".to_string()),
        };
        parts_w += w;
        parts.push(Part {
            res: PartTemp {
                refdes: fp.reference.clone(),
                part: fp.value.clone(),
                w,
                w_source,
                at,
                side: side.into(),
                family,
                area_mm2: area,
                model: match model {
                    PartModel::Network { .. } => "theta_jb",
                    PartModel::Psi(_) => "psi_jb",
                    PartModel::Ja(_) => "theta_ja",
                }
                .into(),
                theta_jb,
                theta_jc,
                psi_jb,
                theta_ja,
                data_source,
                estimate,
                t_board_c: ta,
                tj_c: ta,
                to_board: None,
                tj_max_c: tj_max,
                tj_max_source: tj_src,
                margin_k: 0.0,
                status: String::new(),
            },
            cells,
            model,
        });
    }

    // surface coefficients from the mean rise (energy balance on the board's area)
    let p_total = copper_w + parts_w;
    if p_total <= 0.0 {
        warnings.push("no heat: neither a power tree with loads (PI_REG / PI_LOAD, step 1) nor PI_THERMAL p=… – the board stays at the ambient temperature".into());
    }
    let (hc_t, hc_b, h_rad, dt_mean) = match prm.h {
        Some(h) => (h, h, 0.0, p_total / (2.0 * h * m.area_m2)),
        None => {
            let mut dt = 10.0;
            let mut hs = (0.0, 0.0, 0.0);
            for _ in 0..50 {
                hs = surface_coefficients(prm, m.area_m2, m.perim_m, m.long_m, dt, ta_k);
                let nd = p_total / ((hs.0 + hs.1 + 2.0 * hs.2) * m.area_m2);
                if (nd - dt).abs() < 1e-4 {
                    dt = nd;
                    break;
                }
                dt = 0.5 * (dt + nd);
            }
            (hs.0, hs.1, hs.2, dt)
        }
    };
    let (h_top, h_bot) = (hc_t + h_rad, hc_b + h_rad);
    let g_surf = [a_cell / (1.0 / h_top + m.r_mask[0]), a_cell / (1.0 / h_bot + m.r_mask[1])];
    for p in parts.iter_mut() {
        if let PartModel::Network { g_top, .. } = &mut p.model {
            if let Some(jc) = p.res.theta_jc {
                let h = if p.res.side == "bottom" { h_bot } else { h_top };
                *g_top = 1.0 / (jc + 1.0 / (h * (p.res.area_mm2 * 1e-6).max(1e-8)));
            }
        }
    }

    // the system: board nodes, then one junction node per network part
    let nj = parts.iter().filter(|p| matches!(p.model, PartModel::Network { .. })).count();
    let n = nb + nj;
    let mut rhs = vec![0f64; n];
    rhs[..nb].copy_from_slice(&q);
    let mut trip: Vec<(usize, usize, f64)> = Vec::with_capacity(nb * 7 + nj * 16);
    let edge = |trip: &mut Vec<(usize, usize, f64)>, a: usize, b: usize, gv: f64| {
        trip.push((a, a, gv));
        trip.push((b, b, gv));
        trip.push((a, b, -gv));
        trip.push((b, a, -gv));
    };
    let g = &m.g;
    for l in 0..nl {
        let kl = &m.kxy[l];
        for (c, &k) in m.cells.iter().enumerate() {
            let (i, j) = (k % g.nx, k / g.nx);
            for (ni, nj2) in [(i + 1, j), (i, j + 1)] {
                if ni >= g.nx || nj2 >= g.ny {
                    continue;
                }
                let c2 = m.idx[nj2 * g.nx + ni];
                if c2 == u32::MAX {
                    continue;
                }
                let (a, b) = (kl[c], kl[c2 as usize]);
                edge(&mut trip, m.node(l, c), m.node(l, c2 as usize), 2.0 * a * b / (a + b));
            }
        }
        if l + 1 < nl {
            for c in 0..nc {
                edge(&mut trip, m.node(l, c), m.node(l + 1, c), m.gz[l][c]);
            }
        }
    }
    for c in 0..nc {
        trip.push((m.node(0, c), m.node(0, c), g_surf[0]));
        trip.push((m.node(nl - 1, c), m.node(nl - 1, c), g_surf[1]));
    }
    let mut jn = nb;
    let mut junction: Vec<Option<usize>> = Vec::with_capacity(parts.len());
    for p in &parts {
        match p.model {
            PartModel::Network { g_jb, g_top } => {
                for &(node, wt) in &p.cells {
                    edge(&mut trip, jn, node, g_jb * wt);
                }
                trip.push((jn, jn, g_top));
                rhs[jn] += p.res.w;
                junction.push(Some(jn));
                jn += 1;
            }
            PartModel::Psi(_) | PartModel::Ja(_) => {
                for &(node, wt) in &p.cells {
                    rhs[node] += p.res.w * wt;
                }
                junction.push(None);
            }
        }
    }
    let x = kisi_sparse::solve_spd_triplets(n, &trip, &rhs).map_err(|c| format!("thermal solver: error {c}"))?;

    // energy balance: out through the surfaces and the part tops
    let mut out_top = 0.0;
    let mut out_bot = 0.0;
    for c in 0..nc {
        out_top += g_surf[0] * x[m.node(0, c)];
        out_bot += g_surf[1] * x[m.node(nl - 1, c)];
    }
    let mut out_parts = [0.0, 0.0];
    for (p, jn) in parts.iter_mut().zip(&junction) {
        let tb: f64 = p.cells.iter().map(|&(node, wt)| wt * x[node]).sum();
        p.res.t_board_c = ta + tb;
        p.res.tj_c = match (&p.model, jn) {
            (PartModel::Network { g_jb, g_top }, Some(j)) => {
                out_parts[(p.res.side == "bottom") as usize] += g_top * x[*j];
                p.res.to_board = Some((g_jb * (x[*j] - tb) / p.res.w).clamp(0.0, 1.0));
                ta + x[*j]
            }
            (PartModel::Psi(psi), _) => ta + tb + psi * p.res.w,
            (PartModel::Ja(ja), _) => ta + ja * p.res.w,
            _ => ta + tb,
        };
        p.res.margin_k = p.res.tj_max_c - p.res.tj_c;
        p.res.status = if p.res.margin_k < 0.0 { "over" } else if p.res.margin_k < 10.0 { "near" } else { "ok" }.into();
    }
    let out = out_top + out_bot + out_parts[0] + out_parts[1];
    let rad = (out_top + out_parts[0]) * h_rad / h_top + (out_bot + out_parts[1]) * h_rad / h_bot;
    let balance = Balance { in_w: p_total, convection_w: out - rad, radiation_w: rad, error_rel: if p_total > 0.0 { (p_total - out).abs() / p_total } else { 0.0 } };

    // temperature planes, per-layer figures, hot spots
    let per = g.nx * g.ny;
    let mut t = vec![f32::NAN; nl * per];
    let mut layers = Vec::with_capacity(nl);
    let (mut tmax, mut tmax_at, mut tmax_l) = (f64::NEG_INFINITY, None, 0u16);
    let mut proj = vec![(f64::NEG_INFINITY, 0u16); nc];
    for l in 0..nl {
        let (mut mx, mut at, mut sum) = (f64::NEG_INFINITY, None, 0.0);
        for (c, &k) in m.cells.iter().enumerate() {
            let v = ta + x[m.node(l, c)];
            t[l * per + k] = v as f32;
            sum += v;
            if v > mx {
                mx = v;
                at = Some(g.center(k % g.nx, k / g.nx));
            }
            if v > proj[c].0 {
                proj[c] = (v, l as u16);
            }
        }
        if mx > tmax {
            (tmax, tmax_at, tmax_l) = (mx, at, l as u16);
        }
        layers.push(LayerTemp { layer: l as u16, name: su.copper.get(l).map(|c| c.name.clone()).unwrap_or_default(), t_max_c: mx, t_mean_c: sum / nc as f64, max_at: at });
    }
    let centres: Vec<(String, P)> = board
        .footprints
        .iter()
        .enumerate()
        .filter_map(|(fi, f)| {
            let mut bb = BBox::empty();
            for p in board.pads.iter().filter(|p| p.fp as usize == fi) {
                bb.add(p.at);
            }
            bb.is_valid().then(|| (f.reference.clone(), [(bb.min[0] + bb.max[0]) / 2.0, (bb.min[1] + bb.max[1]) / 2.0]))
        })
        .collect();
    let mut order: Vec<usize> = (0..nc).collect();
    order.sort_by(|&a, &b| proj[b].0.total_cmp(&proj[a].0));
    let sep = (5.0f64).max(4.0 * g.cell);
    let mut hot_spots: Vec<HotSpot> = Vec::new();
    for c in order {
        if hot_spots.len() >= 8 || proj[c].0 - ta < 0.05 * (tmax - ta) {
            break;
        }
        let k = m.cells[c];
        let at = g.center(k % g.nx, k / g.nx);
        if hot_spots.iter().any(|h| (h.at[0] - at[0]).hypot(h.at[1] - at[1]) < sep) {
            continue;
        }
        let near = centres.iter().map(|(r, p)| (r, (p[0] - at[0]).hypot(p[1] - at[1]))).filter(|x| x.1 < 4.0).min_by(|a, b| a.1.total_cmp(&b.1)).map(|x| x.0.clone());
        hot_spots.push(HotSpot { at, layer: proj[c].1, t_c: proj[c].0, near });
    }
    if tmax > d.laminate.mot_c {
        warnings.push(format!("the board reaches {tmax:.0} °C, above the laminate's typical maximum operating temperature of {:.0} °C (FR-4, estimate)", d.laminate.mot_c));
    }
    let mut parts: Vec<PartTemp> = parts.into_iter().map(|p| p.res).collect();
    parts.sort_by(|a, b| b.tj_c.total_cmp(&a.tj_c));
    let n_default = parts.iter().filter(|p| p.data_source.starts_with("package default")).count();
    let n_ja = parts.iter().filter(|p| p.model == "theta_ja").count();
    notes.push("Estimate: steady state of the board in air at the ambient temperature; heat leaves by convection and radiation from both sides and the part tops – not through mounting holes, connectors, card guides or heat sinks. Copper fill of every layer, laminate (in plane 0.8, through 0.3 W/m·K) and open via barrels conduct.".into());
    notes.push("Parts: JEDEC two-resistor model (θJB to the board under the pads, θJC and the top surface to the air); Tj is as good as the θ values – package defaults are estimates by family and size, PI_THERMAL gives datasheet values.".into());
    if n_default > 0 {
        notes.push(format!("{n_default} part(s) with package default θ values (estimate): add PI_THERMAL from their datasheets' Thermal Information tables."));
    }
    if n_ja > 0 {
        notes.push(format!("{n_ja} part(s) with θJA only: Tj = T_amb + θJA·P on the JEDEC test board, without this board's heat from other parts (fallback)."));
    }
    if parts.iter().any(|p| p.tj_max_source.starts_with("default")) {
        notes.push(format!("Tj max {:.0} °C assumed where PI_THERMAL gives none ({}).", pk.tj_max_c, pk.tj_max_note));
    }
    Ok(ThermalResult {
        ambient_c: ta,
        cooling: Cooling {
            orientation: if prm.vertical { "vertical" } else { "horizontal" }.into(),
            airflow_m_s: prm.airflow_m_s,
            h_top,
            h_bottom: h_bot,
            h_conv_top: hc_t,
            h_conv_bottom: hc_b,
            h_rad,
            estimated: prm.h.is_none(),
            dt_mean_k: dt_mean,
        },
        cell: g.cell,
        x0: g.x0,
        y0: g.y0,
        nx: g.nx,
        ny: g.ny,
        layers,
        nodes: n,
        area_mm2: m.area_m2 * 1e6,
        parts_w,
        copper_w: copper_w - unplaced,
        balance,
        t_max_c: tmax,
        t_max_at: tmax_at,
        t_max_layer: tmax_l,
        hot_spots,
        parts,
        rails: vec![],
        warnings,
        notes,
        t,
    })
}

/// Copper temperature of a rail (from its loss cells) and one electro-thermal step: the rail
/// solved again with ρ(T) = ρ₂₀(1 + α(T − 20 °C)) at the estimated temperature.
pub fn rail_at_temperature(board: &Board, setup: &RailSetup, base: &PiResult, th: &ThermalResult, params: &PiParams) -> RailThermal {
    let alpha = data().copper.alpha_per_k;
    let (mut tmax, mut sw, mut st) = (f64::NEG_INFINITY, 0.0, 0.0);
    for lg in &base.layers {
        for j in 0..lg.ny {
            for i in 0..lg.nx {
                let w = base.loss[lg.offset + j * lg.nx + i] as f64;
                if w <= 0.0 {
                    continue;
                }
                let tc = th.temp_at(lg.layer as usize, [lg.x0 + (i as f64 + 0.5) * base.cell, lg.y0 + (j as f64 + 0.5) * base.cell]);
                tmax = tmax.max(tc);
                sw += w;
                st += w * tc;
            }
        }
    }
    let mut r = RailThermal {
        net: base.net.clone(),
        ground: base.ground,
        t_max_c: tmax.is_finite().then_some(tmax),
        t_mean_c: (sw > 0.0).then(|| st / sw),
        drop_mv_20c: base.max_drop_mv,
        loss_w_20c: base.power_loss_w,
        ..Default::default()
    };
    let f = |l: usize, p: P| 1.0 + alpha * (th.temp_at(l, p) - 20.0);
    match crate::pi::solve_rail_scaled(board, setup, params, Some(&f)) {
        Ok(h) => {
            r.drop_mv_hot = h.max_drop_mv;
            r.loss_w_hot = Some(h.power_loss_w);
        }
        Err(e) => r.error = Some(e),
    }
    r
}

#[cfg(test)]
mod tests;

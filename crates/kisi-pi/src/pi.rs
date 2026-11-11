//! DC power integrity: IR drop of supply nets.
//!
//! Each copper layer of a net is rasterised into square cells; neighbouring
//! cells are connected by the sheet conductance of the layer, vias and
//! through-hole pads connect the layers. Source pads are held at the rail
//! voltage, load pads draw their current. The resulting SPD system is solved
//! with a sparse Cholesky factorisation (kisi-sparse / faer).
//!
//! Three corrections keep the raster's answer close to the converged one at any cell size
//! (docs/research/pi-quadtree.md):
//! * A cell's weight is its copper fraction (cut cells, `fill_fraction`), not copper at its centre.
//! * Tracks narrower than `LINE_CELLS` cells are line conductors (`line_tracks`): the cells along the centre
//!   line in order, joined by the track's conductance between the mean positions of the line in consecutive
//!   cells – exact for a straight track at any angle, where a staircase of cells is up to √2 longer and
//!   rounds the width to whole cells.
//! * Via contacts (Peaceman 1983): a barrel joined to the one cell around its centre acts like a disc of
//!   0.198 cell, so the spreading resistance into a pour would grow with ln(1/cell). A barrel smaller than
//!   0.7 cell scales its contact cell's links to full-copper neighbours by `peaceman_factor`, so that the
//!   cell holds the potential of the barrel's rim; a larger one joins every cell within r_w + 0.35 cell into
//!   one node (an equipotential disc).
//!
//! Configuration comes from footprint fields in the board (PI_SOURCE,
//! PI_LOAD), the board configuration, or the viewer.

use crate::board::{pad_polygon, pad_polygons, Board, Pad, Track};
use crate::geom::{self, BBox, P};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};

// --- configuration -------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct SourceSpec {
    pub refdes: String,
    /// pad numbers; empty = all pads of the component on the rail
    pub pads: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct LoadSpec {
    pub refdes: String,
    /// current drawn from the rail (A)
    pub current: f64,
    pub pads: Vec<String>,
    /// where the value came from: "field", "user", "regulator"
    pub origin: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct RailSetup {
    pub net: String,
    pub voltage: f64,
    pub sources: Vec<SourceSpec>,
    pub loads: Vec<LoadSpec>,
    /// ground return (`losses::ground_setups`): reference 0 V at the sources (the supply
    /// input), loads with negative current feed their return current into the net
    pub ground: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct PiParams {
    /// cell size in mm; None = automatic
    pub cell: Option<f64>,
    /// via barrel plating thickness (µm)
    pub plating_um: f64,
    /// copper resistivity (Ω·m), 1.72e-8 at 20 °C
    pub rho: f64,
    /// upper bound for the number of cells (all layers)
    pub max_cells: usize,
    /// tracks narrower than this (mm) are line conductors; None: `LINE_CELLS` cells
    pub line_width: Option<f64>,
}

impl Default for PiParams {
    fn default() -> Self {
        PiParams { cell: None, plating_um: 25.0, rho: 1.72e-8, max_cells: 250_000, line_width: None }
    }
}

// --- results ----------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct LayerGrid {
    pub layer: u16,
    pub x0: f64,
    pub y0: f64,
    pub nx: usize,
    pub ny: usize,
    /// offset of this layer's cells in the flat value buffers
    pub offset: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct LoadResult {
    pub refdes: String,
    pub current: f64,
    pub origin: String,
    pub connected: bool,
    pub v_min: Option<f64>,
    pub v_mean: Option<f64>,
    pub v_max: Option<f64>,
    pub pads: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct SourceResult {
    pub refdes: String,
    pub current: f64,
    pub pads: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct ViaResult {
    pub at: P,
    pub current: f64,
    pub drill: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct PiResult {
    pub net: String,
    pub voltage: f64,
    /// ground return: `max_drop_mv` is the largest ground offset at a component
    pub ground: bool,
    pub cell: f64,
    pub layers: Vec<LayerGrid>,
    pub loads: Vec<LoadResult>,
    pub sources: Vec<SourceResult>,
    /// vias with the highest currents (top 30)
    pub vias: Vec<ViaResult>,
    pub nodes: usize,
    pub v_min: Option<f64>,
    pub max_drop_mv: Option<f64>,
    /// highest current density (A/mm²) and where
    pub max_j: f64,
    pub max_j_at: Option<P>,
    pub power_loss_w: f64,
    pub total_load: f64,
    pub warnings: Vec<String>,
    /// flat buffers, `layers[i].offset .. + nx*ny`: voltage (NaN = no copper)
    #[serde(skip)]
    pub v: Vec<f32>,
    /// current density magnitude (A/mm²)
    #[serde(skip)]
    pub j: Vec<f32>,
    /// copper loss per cell (W): half of every conductance's I²R to each of its two cells
    #[serde(skip)]
    pub loss: Vec<f32>,
    /// copper area per cell in cell areas, in steps of 1/`CU_STEPS` (0: no copper or not connected; a line
    /// conductor's cell: its width / cell, up to `LINE_CELLS`) – the copper area the loss of a cell is spread
    /// over in the loss maps
    #[serde(skip)]
    pub cu: Vec<u8>,
}

// --- fields & helpers -------------------------------------------------------------

/// Nominal voltage from a net name: "+3V3" 3.3, "1V0" 1.0, "+5V" 5, "VCC_1V8" 1.8,
/// "+1.2V" 1.2, "12V_IN" 12. None if no voltage is recognisable.
pub fn voltage_from_name(name: &str) -> Option<f64> {
    let n = name.rsplit('/').next().unwrap_or(name).to_uppercase();
    let c: Vec<char> = n.chars().collect();
    let mut i = 0;
    while i < c.len() {
        if c[i].is_ascii_digit()
            && (i == 0 || !c[i - 1].is_ascii_alphanumeric() || c[i - 1] == 'V' || c[i - 1] == 'C' || c[i - 1] == 'D')
        {
            let st = i;
            while i < c.len() && (c[i].is_ascii_digit() || c[i] == '.') {
                i += 1;
            }
            if i < c.len() && c[i] == 'V' {
                let int: String = c[st..i].iter().collect();
                let mut j = i + 1;
                while j < c.len() && c[j].is_ascii_digit() {
                    j += 1;
                }
                let frac: String = c[i + 1..j].iter().collect();
                let s = if frac.is_empty() { int } else { format!("{int}.{frac}") };
                if let Ok(v) = s.parse::<f64>() {
                    if v > 0.0 && v <= 60.0 {
                        let neg = st > 0 && c[st - 1] == '-';
                        return Some(if neg { -v } else { v });
                    }
                }
            }
        } else {
            i += 1;
        }
    }
    None
}

/// Whether a net or pin name says it is a signal about a supply or a logic signal: a word such
/// as SENSE, DET, ADC, EN, PG, FB, SCL, SDA (VBUS_DET, VBAT_SENSE, VCC_EN, PG_1V0, TRIM_SCL_5V).
pub fn signal_name(name: &str) -> bool {
    const SIGNAL: &[&str] = &[
        "SENSE", "SNS", "SEN", "DET", "DETECT", "ADC", "MON", "MEAS", "EN", "ENA", "ENABLE", "PG", "PGOOD", "POK",
        "PWRGD", "OK", "GOOD", "FB", "CTRL", "CTL", "DIV", "ON", "OFF", "INT", "IRQ", "ALERT", "FLT", "FAULT", "OC",
        "OV", "UV", "ID", "SEL", "STAT", "STATUS", "DIS", "RST", "RESET", "REF", "PWM", "GATE", "DRV", "SIG", "SCL",
        "SDA", "SCK", "SCLK", "CLK", "MISO", "MOSI", "CS", "TX", "RX", "TXD", "RXD", "VINP", "VINN", "VINM",
    ];
    words(name).iter().any(|w| SIGNAL.contains(&w.as_str()))
}

fn words(name: &str) -> Vec<String> {
    name.replace("{slash}", "/")
        .to_uppercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(String::from)
        .collect()
}

/// Whether a net name or pin name looks like a supply without a voltage in it: a word that
/// starts with VCC, VDD, VIN, VBUS, VSYS, VBAT or VEE, or ends with VCC, VDD or VIN (VDDIO,
/// AVDD, PVIN, VBUSOUT, VINLDO1, `Net-(IC22-VCC)`, …), or is VIO – and not a signal name
/// (`signal_name`: VBUS_DET, VBAT_SENSE, VCC_EN, …) or an analog input (VINP, VIN+). Ground names
/// are not supplies. A hint, not a proof: the gap list says so.
pub fn supply_name_hint(name: &str) -> bool {
    const PREFIX: &[&str] = &["VCC", "VDD", "VIN", "VBUS", "VSYS", "VBAT", "VEE"];
    const SUFFIX: &[&str] = &["VCC", "VDD", "VIN"];
    let n = name.replace("{slash}", "/").to_uppercase();
    if is_ground_name(&n)
        || n.starts_with("UNCONNECTED")
        || n.contains("VIN+")
        || n.contains("VIN-)")
        || n.ends_with("VIN-")
        || signal_name(&n)
    {
        return false;
    }
    words(&n)
        .iter()
        .any(|w| w == "VIO" || PREFIX.iter().any(|p| w.starts_with(p)) || SUFFIX.iter().any(|s| w.ends_with(s)))
}

pub fn is_ground_name(name: &str) -> bool {
    let n = name.rsplit('/').next().unwrap_or(name).to_uppercase();
    n.contains("GND") || n.starts_with("VSS") || n == "0V" || n.contains("EARTH")
}

/// Value with unit: "2.5A", "300mA", "2W", "500 mW", "3.3V". Returns (value in base unit, unit char).
pub fn parse_quantity(s: &str) -> Option<(f64, char)> {
    let s = s.trim().replace(',', ".").replace(' ', "");
    let unit = s.chars().last()?.to_ascii_uppercase();
    if !matches!(unit, 'A' | 'W' | 'V') {
        let v: f64 = s.parse().ok()?;
        return Some((v, 'A'));
    }
    let body = &s[..s.len() - 1];
    let (num, mult) = match body.chars().last() {
        Some('m') => (&body[..body.len() - 1], 1e-3),
        Some('u') | Some('µ') => (&body[..body.len() - body.chars().last().unwrap().len_utf8()], 1e-6),
        Some('k') => (&body[..body.len() - 1], 1e3),
        _ => (body, 1.0),
    };
    Some((num.parse::<f64>().ok()? * mult, unit))
}

/// Resolve a net name written by a user (field / config) to a net id.
pub fn find_net(board: &Board, name: &str) -> Option<u32> {
    let want = name.trim();
    let strip = |s: &str| s.trim_start_matches('/').to_string();
    for (i, n) in board.nets.iter().enumerate() {
        if n == want {
            return Some(i as u32);
        }
    }
    for (i, n) in board.nets.iter().enumerate() {
        if strip(n).eq_ignore_ascii_case(&strip(want)) || n.rsplit('/').next() == Some(want) {
            return Some(i as u32);
        }
    }
    None
}

#[derive(Debug, Clone, Serialize)]
pub struct PowerNet {
    pub net: u32,
    pub name: String,
    pub voltage: Option<f64>,
    pub ground: bool,
    /// copper layers with zones of this net
    pub zone_layers: Vec<u16>,
    pub zone_area: f64,
    pub pads: usize,
    /// components with pads on this net
    pub components: Vec<String>,
}

/// Supply nets: nets with zones, or a voltage in the name, or named in a PI field.
pub fn power_nets(board: &Board) -> Vec<PowerNet> {
    let mut by: HashMap<u32, PowerNet> = HashMap::new();
    let add = |board: &Board, id: u32| -> PowerNet {
        let name = board.net_name(id).to_string();
        PowerNet {
            net: id,
            voltage: voltage_from_name(&name),
            ground: is_ground_name(&name),
            name,
            zone_layers: vec![],
            zone_area: 0.0,
            pads: 0,
            components: vec![],
        }
    };
    for z in &board.zones {
        if z.net == 0 {
            continue;
        }
        let e = by.entry(z.net).or_insert_with(|| add(board, z.net));
        if !e.zone_layers.contains(&z.layer) {
            e.zone_layers.push(z.layer);
        }
        e.zone_area += polygon_area(&z.pts);
    }
    for (i, n) in board.nets.iter().enumerate() {
        if i == 0 || by.contains_key(&(i as u32)) {
            continue;
        }
        if voltage_from_name(n).is_some() && !n.to_uppercase().contains("SENSE") {
            by.insert(i as u32, add(board, i as u32));
        }
    }
    for pad in &board.pads {
        if let Some(e) = by.get_mut(&pad.net) {
            e.pads += 1;
            let r = &board.footprints[pad.fp as usize].reference;
            if !r.is_empty() && !e.components.contains(r) {
                e.components.push(r.clone());
            }
        }
    }
    let mut v: Vec<PowerNet> =
        by.into_values().filter(|p| p.pads > 0 && (p.zone_area > 1.0 || p.voltage.is_some() || p.ground)).collect();
    v.sort_by(|a, b| a.ground.cmp(&b.ground).then(b.zone_area.partial_cmp(&a.zone_area).unwrap()));
    v
}

fn polygon_area(p: &[P]) -> f64 {
    let n = p.len();
    let mut a = 0.0;
    for i in 0..n {
        let (x1, y1) = (p[i][0], p[i][1]);
        let (x2, y2) = (p[(i + 1) % n][0], p[(i + 1) % n][1]);
        a += x1 * y2 - x2 * y1;
    }
    (a / 2.0).abs()
}

/// Rail setups from footprint fields:
///   PI_SOURCE = "+3V3" or "+3V3=3.3V" (several separated by ',' or ';')
///   PI_LOAD   = "+1V0=2.5A, +1V8=300mA" or "+1V0=2W" (power -> current via rail voltage)
/// Net names with spaces in double quotes (`"/Sensors/IMU VDD"=1mA`, see `field_syntax`).
pub fn rails_from_fields(board: &Board) -> (Vec<RailSetup>, Vec<String>) {
    let mut rails: Vec<RailSetup> = Vec::new();
    let mut warnings = Vec::new();
    let rail_idx = |rails: &mut Vec<RailSetup>, net: &str| -> usize {
        if let Some(i) = rails.iter().position(|r| r.net == net) {
            return i;
        }
        rails.push(RailSetup {
            net: net.to_string(),
            voltage: voltage_from_name(net).unwrap_or(0.0),
            ..Default::default()
        });
        rails.len() - 1
    };
    // sources first (they may define the voltage)
    for fp in &board.footprints {
        let Some(v) = fp.prop("PI_SOURCE") else { continue };
        for item in crate::field_syntax::split_unquoted(v, &[',', ';']) {
            let (name, val) = match crate::field_syntax::split_once_unquoted(item, '=') {
                Some((n, q)) => (crate::field_syntax::unquote(n), Some(q)),
                None => (crate::field_syntax::unquote(item), None),
            };
            let name = name.as_str();
            let Some(net) = find_net(board, name) else {
                warnings.push(format!("{}: PI_SOURCE names unknown net '{}'", fp.reference, name));
                continue;
            };
            let net_name = board.net_name(net).to_string();
            let i = rail_idx(&mut rails, &net_name);
            if let Some((volts, 'V')) = val.and_then(parse_quantity) {
                rails[i].voltage = volts;
            }
            rails[i].sources.push(SourceSpec { refdes: fp.reference.clone(), pads: vec![] });
        }
    }
    for fp in &board.footprints {
        let Some(v) = fp.prop("PI_LOAD") else { continue };
        for item in crate::field_syntax::split_unquoted(v, &[',', ';']) {
            let Some((name, q)) = crate::field_syntax::split_once_unquoted(item, '=') else {
                warnings.push(format!("{}: PI_LOAD '{}' without value (form NET=2A)", fp.reference, item));
                continue;
            };
            let name = crate::field_syntax::unquote(name);
            let Some(net) = find_net(board, &name) else {
                warnings.push(format!("{}: PI_LOAD names unknown net '{}'", fp.reference, name));
                continue;
            };
            let net_name = board.net_name(net).to_string();
            let i = rail_idx(&mut rails, &net_name);
            // "NET=60mA@L32/L33": the current enters at these parts' pads (split evenly)
            let (q, at): (&str, Vec<String>) = match q.split_once('@') {
                Some((q, a)) => {
                    (q, a.split(['/', '+']).map(|r| r.trim().to_string()).filter(|r| !r.is_empty()).collect())
                }
                None => (q, vec![fp.reference.clone()]),
            };
            let current = match parse_quantity(q) {
                Some((a, 'A')) => a,
                Some((w, 'W')) => {
                    if rails[i].voltage > 0.0 {
                        w / rails[i].voltage
                    } else {
                        warnings.push(format!("{}: power for {} without known voltage", fp.reference, net_name));
                        continue;
                    }
                }
                _ => {
                    warnings.push(format!("{}: PI_LOAD value '{}' not readable", fp.reference, q.trim()));
                    continue;
                }
            };
            for r in &at {
                rails[i].loads.push(LoadSpec {
                    refdes: r.clone(),
                    current: current / at.len() as f64,
                    pads: vec![],
                    origin: "field".into(),
                });
            }
        }
    }
    (rails, warnings)
}

/// Rail setups (sources and loads chosen per rail in the viewer, without a power tree) as footprint
/// fields (ref, field name, value): PI_SOURCE `NET=<V>V` on every source part, PI_LOAD `NET=<I>`
/// on every load part – the inverse of `rails_from_fields`. Returns the fields and what fields cannot
/// say: pads chosen on a part (fields name parts; their pads on the rail are all used).
pub fn setups_to_fields(setups: &[RailSetup]) -> (Vec<(String, String, String)>, Vec<String>) {
    use crate::field_syntax::quote_name;
    let mut out: Vec<(String, String, String)> = Vec::new();
    let mut lost = Vec::new();
    let push = |r: &str, name: &str, v: String, out: &mut Vec<(String, String, String)>| match out
        .iter_mut()
        .find(|x| x.0 == r && x.1 == name)
    {
        Some(x) => x.2 = format!("{}, {v}", x.2),
        None => out.push((r.to_string(), name.to_string(), v)),
    };
    for s in setups.iter().filter(|s| !s.ground) {
        let net = quote_name(&s.net);
        for src in &s.sources {
            let v = if s.voltage != 0.0 && voltage_from_name(&s.net).is_none_or(|x| (x - s.voltage).abs() > 1e-9) {
                format!("{net}={}V", crate::tree::field_num(s.voltage))
            } else {
                net.clone()
            };
            push(&src.refdes, "PI_SOURCE", v, &mut out);
            if !src.pads.is_empty() {
                lost.push(format!(
                    "{}: source pads {} on {} (the field uses all its pads on the rail)",
                    src.refdes,
                    src.pads.join(" "),
                    s.net
                ));
            }
        }
        for l in s.loads.iter().filter(|l| l.current != 0.0 && l.origin != "regulator") {
            push(&l.refdes, "PI_LOAD", format!("{net}={}", crate::tree::field_amps(l.current)), &mut out);
            if !l.pads.is_empty() {
                lost.push(format!(
                    "{}: load pads {} on {} (the field uses all its pads on the rail)",
                    l.refdes,
                    l.pads.join(" "),
                    s.net
                ));
            }
        }
    }
    (out, lost)
}

// --- rasterisation ------------------------------------------------------------------

pub(crate) struct Grid {
    pub(crate) x0: f64,
    pub(crate) y0: f64,
    pub(crate) cell: f64,
    pub(crate) nx: usize,
    pub(crate) ny: usize,
}

impl Grid {
    pub(crate) fn cell_of(&self, p: P) -> Option<(usize, usize)> {
        let i = ((p[0] - self.x0) / self.cell).floor();
        let j = ((p[1] - self.y0) / self.cell).floor();
        if i < 0.0 || j < 0.0 || i as usize >= self.nx || j as usize >= self.ny {
            return None;
        }
        Some((i as usize, j as usize))
    }
    pub(crate) fn center(&self, i: usize, j: usize) -> P {
        [self.x0 + (i as f64 + 0.5) * self.cell, self.y0 + (j as f64 + 0.5) * self.cell]
    }
    /// Cells (indices) whose centres lie within `r` of `p`.
    pub(crate) fn near(&self, p: P, r: f64) -> Vec<usize> {
        let rc = (r / self.cell).ceil() as isize + 1;
        let Some((ci, cj)) = self.cell_of(p) else { return vec![] };
        let mut out = vec![];
        for j in (cj as isize - rc).max(0)..=(cj as isize + rc).min(self.ny as isize - 1) {
            for i in (ci as isize - rc).max(0)..=(ci as isize + rc).min(self.nx as isize - 1) {
                if geom::len(geom::sub(self.center(i as usize, j as usize), p)) <= r {
                    out.push(j as usize * self.nx + i as usize);
                }
            }
        }
        out
    }
}

/// Distance of `p` from the segment a–b and the nearest point on it.
fn seg_dist(p: P, a: P, b: P) -> (f64, P) {
    let d = geom::sub(b, a);
    let q = geom::add(a, geom::scale(d, (geom::dot(geom::sub(p, a), d) / geom::dot(d, d).max(1e-18)).clamp(0.0, 1.0)));
    (geom::len(geom::sub(p, q)), q)
}

/// Fill a polygon (even-odd) into `w` with weight 1 at cell centres.
pub(crate) fn fill_polygon(g: &Grid, pts: &[P], w: &mut [f32]) -> usize {
    let n = pts.len();
    if n < 3 {
        return 0;
    }
    let bb = BBox::of(pts);
    let j0 = (((bb.min[1] - g.y0) / g.cell - 0.5).ceil().max(0.0)) as usize;
    let j1 = (((bb.max[1] - g.y0) / g.cell - 0.5).floor()).min(g.ny as f64 - 1.0);
    if j1 < 0.0 || j0 as f64 > j1 {
        return 0;
    }
    let j1 = j1 as usize;
    let mut rows: Vec<Vec<f64>> = vec![Vec::new(); j1 - j0 + 1];
    for k in 0..n {
        let a = pts[k];
        let b = pts[(k + 1) % n];
        if a[1] == b[1] {
            continue;
        }
        let (lo, hi) = if a[1] < b[1] { (a, b) } else { (b, a) };
        // rows whose centre y is in [lo.y, hi.y)
        let ja = (((lo[1] - g.y0) / g.cell - 0.5).ceil().max(j0 as f64)) as usize;
        let jb_f = ((hi[1] - g.y0) / g.cell - 0.5).ceil() - 1.0;
        if jb_f < ja as f64 {
            continue;
        }
        let jb = (jb_f as usize).min(j1);
        for j in ja..=jb {
            let y = g.y0 + (j as f64 + 0.5) * g.cell;
            let x = lo[0] + (y - lo[1]) * (hi[0] - lo[0]) / (hi[1] - lo[1]);
            rows[j - j0].push(x);
        }
    }
    let mut count = 0;
    for (r, xs) in rows.iter_mut().enumerate() {
        xs.sort_by(f64::total_cmp);
        let j = j0 + r;
        for pair in xs.as_chunks::<2>().0 {
            let i0 = (((pair[0] - g.x0) / g.cell - 0.5).ceil().max(0.0)) as usize;
            let i1 = ((pair[1] - g.x0) / g.cell - 0.5).floor();
            if i1 < i0 as f64 {
                continue;
            }
            let i1 = (i1 as usize).min(g.nx - 1);
            for i in i0..=i1 {
                w[j * g.nx + i] = 1.0;
                count += 1;
            }
        }
    }
    count
}

/// Rasterise a track (capsule). Tracks narrower than a cell keep their
/// conductance through a fractional weight width/cell on the centre-line cells.
pub(crate) fn fill_track(g: &Grid, a: P, b: P, width: f64, w: &mut [f32]) {
    let r = width / 2.0;
    // thin tracks: half the cell diagonal (with a margin: a 45° centre line through cell centres puts the
    // side cells exactly there) keeps the chain of cells 4-connected for any direction; a band narrower
    // than √2 cells can hold a single diagonal row of cells, which is not, so up to 1.5 cells
    let thin = width < g.cell * 1.5;
    let reach = if thin { g.cell * (std::f64::consts::FRAC_1_SQRT_2 + 1e-6) } else { r };
    let frac = if thin { (width / g.cell).min(1.0) as f32 } else { 1.0 };
    let bb_min = [a[0].min(b[0]) - reach, a[1].min(b[1]) - reach];
    let bb_max = [a[0].max(b[0]) + reach, a[1].max(b[1]) + reach];
    let (Some((i0, j0)), Some((i1, j1))) = (
        g.cell_of([bb_min[0].max(g.x0), bb_min[1].max(g.y0)]),
        g.cell_of([
            bb_max[0].min(g.x0 + g.nx as f64 * g.cell - 1e-9),
            bb_max[1].min(g.y0 + g.ny as f64 * g.cell - 1e-9),
        ]),
    ) else {
        return;
    };
    for j in j0..=j1 {
        for i in i0..=i1 {
            if seg_dist(g.center(i, j), a, b).0 <= reach {
                let k = j * g.nx + i;
                if w[k] < frac {
                    w[k] = frac;
                }
            }
        }
    }
}

/// Copper fractions of the cells from the union of `polys` (each even-odd): `ss` sample lines per row of
/// cells, the copper along each exactly (spans); fractions below 1/16 are no copper. Raised into `w`.
pub(crate) fn fill_fraction(g: &Grid, polys: &[&[P]], ss: usize, w: &mut [f32]) {
    let h = g.cell / ss as f64;
    // per sample line: crossings of the current polygon, starts and ends of the spans of all
    let mut xs: Vec<Vec<f64>> = vec![Vec::new(); g.ny * ss];
    let (mut st, mut en): (Vec<Vec<f64>>, Vec<Vec<f64>>) = (vec![Vec::new(); g.ny * ss], vec![Vec::new(); g.ny * ss]);
    for pts in polys {
        let mut rows = Vec::new();
        for k in 0..pts.len() {
            let (a, b) = (pts[k], pts[(k + 1) % pts.len()]);
            let (lo, hi) = if a[1] < b[1] { (a, b) } else { (b, a) };
            // sample lines with y in [lo.y, hi.y)
            let r0 = ((lo[1] - g.y0) / h - 0.5).ceil().max(0.0) as usize;
            let r1 = (((hi[1] - g.y0) / h - 0.5).ceil().max(0.0) as usize).min(g.ny * ss);
            for r in r0..r1 {
                let y = g.y0 + (r as f64 + 0.5) * h;
                xs[r].push(lo[0] + (y - lo[1]) * (hi[0] - lo[0]) / (hi[1] - lo[1]));
                rows.push(r);
            }
        }
        for r in rows {
            let x = &mut xs[r];
            x.sort_by(f64::total_cmp);
            for p in x.as_chunks::<2>().0 {
                st[r].push(p[0]);
                en[r].push(p[1]);
            }
            x.clear();
        }
    }
    let (mut part, mut diff) = (vec![0f64; g.nx], vec![0i32; g.nx + 1]);
    for j in 0..g.ny {
        part.fill(0.0);
        diff.fill(0);
        for r in j * ss..(j + 1) * ss {
            let (s, e) = (&mut st[r], &mut en[r]);
            s.sort_by(f64::total_cmp);
            e.sort_by(f64::total_cmp);
            // the union of the spans (a sweep over starts and ends; touching spans merge), in cell units:
            // partial end cells, full cells between
            let (mut i, mut k, mut depth, mut a) = (0, 0, 0, 0.0);
            while k < e.len() {
                if i < s.len() && s[i] <= e[k] {
                    if depth == 0 {
                        a = s[i];
                    }
                    (depth, i) = (depth + 1, i + 1);
                    continue;
                }
                (depth, k) = (depth - 1, k + 1);
                if depth > 0 {
                    continue;
                }
                let (u, v) = (((a - g.x0) / g.cell).max(0.0), ((e[k - 1] - g.x0) / g.cell).min(g.nx as f64));
                if u >= v {
                    continue;
                }
                let (iu, iv) = (u as usize, (v as usize).min(g.nx - 1));
                if iu == iv {
                    part[iu] += v - u;
                } else {
                    part[iu] += (iu + 1) as f64 - u;
                    part[iv] += v - iv as f64;
                    diff[iu + 1] += 1;
                    diff[iv] -= 1;
                }
            }
        }
        let mut run = 0;
        for i in 0..g.nx {
            run += diff[i];
            let f = (run as f64 + part[i]) / ss as f64;
            let k = j * g.nx + i;
            if f >= 1.0 / 16.0 {
                w[k] = w[k].max(if f > 0.999 { 1.0 } else { f as f32 });
            }
        }
    }
}

/// A track's outline (capsule; caps as half 8-gons).
fn capsule(t: &Track) -> Vec<P> {
    let d = geom::sub(t.b, t.a);
    let th = d[1].atan2(d[0]);
    let r = t.width / 2.0;
    (0..=8)
        .map(|k| (t.b, th - std::f64::consts::FRAC_PI_2 + k as f64 * std::f64::consts::PI / 8.0))
        .chain((0..=8).map(|k| (t.a, th + std::f64::consts::FRAC_PI_2 + k as f64 * std::f64::consts::PI / 8.0)))
        .map(|(c, a)| [c[0] + r * a.cos(), c[1] + r * a.sin()])
        .collect()
}

/// Cells (layer-local indices) covered by a pad; at least the cell at its centre.
pub(crate) fn pad_cells(g: &Grid, pad: &Pad) -> Vec<usize> {
    let mut tmp = vec![0f32; 0];
    let poly = pad_polygon(pad);
    let bb = BBox::of(&poly);
    let mut out = Vec::new();
    // small local fill
    if let (Some((i0, j0)), Some((i1, j1))) = (g.cell_of(bb.min), g.cell_of(bb.max)) {
        let sub = Grid {
            x0: g.x0 + i0 as f64 * g.cell,
            y0: g.y0 + j0 as f64 * g.cell,
            cell: g.cell,
            nx: i1 - i0 + 1,
            ny: j1 - j0 + 1,
        };
        tmp.resize(sub.nx * sub.ny, 0.0);
        fill_polygon(&sub, &poly, &mut tmp);
        for jj in 0..sub.ny {
            for ii in 0..sub.nx {
                if tmp[jj * sub.nx + ii] > 0.0 {
                    out.push((j0 + jj) * g.nx + (i0 + ii));
                }
            }
        }
    }
    if out.is_empty() {
        if let Some((i, j)) = g.cell_of(pad.at) {
            out.push(j * g.nx + i);
        }
    }
    out
}

/// Tracks narrower than this many cells are line conductors (`line_tracks`).
pub(crate) const LINE_CELLS: f64 = 3.0;
/// steps per cell area of `PiResult::cu` (u8: up to 255 / 80 = 3.19 cell areas, a line conductor's width)
pub const CU_STEPS: f32 = 80.0;

/// A track as a line conductor: the cells along its centre line in order with the mean position of the
/// line in each, its width and its ends.
pub(crate) struct Line {
    pub(crate) cells: Vec<(usize, P)>,
    pub(crate) width: f64,
    pub(crate) ends: [P; 2],
}

/// The tracks of one layer of a net as line conductors (None: rasterised): narrower than `max_w` and not
/// overlapping another track of the net along their length – parallel or stacked tracks are copper that the
/// raster joins; as lines their conductances would add up. Overlap near the ends (junctions) does not count.
pub(crate) fn line_tracks(g: &Grid, ts: &[&Track], max_w: f64) -> Vec<Option<Line>> {
    // 1 mm buckets (a collision only adds candidates)
    let key = |x: i64, y: i64| (x as usize).wrapping_mul(73_856_093) ^ y as usize;
    let at_key = |p: P| key(p[0].floor() as i64, p[1].floor() as i64);
    let mut hash: HashMap<usize, Vec<usize>> = HashMap::new();
    for (i, t) in ts.iter().enumerate() {
        let r = t.width / 2.0;
        let (x0, x1) = ((t.a[0].min(t.b[0]) - r).floor() as i64, (t.a[0].max(t.b[0]) + r).floor() as i64);
        let (y0, y1) = ((t.a[1].min(t.b[1]) - r).floor() as i64, (t.a[1].max(t.b[1]) + r).floor() as i64);
        for x in x0..=x1 {
            for y in y0..=y1 {
                hash.entry(key(x, y)).or_default().push(i);
            }
        }
    }
    let line = |i: usize, t: &Track| -> Option<Line> {
        if t.width >= max_w {
            return None;
        }
        let len = geom::len(geom::sub(t.b, t.a));
        let at = |u: f64| geom::lerp(t.a, t.b, u);
        let n = (len / (t.width / 2.0).clamp(0.01, 0.1)).ceil().max(1.0) as usize;
        let mut cand: Vec<usize> =
            (0..=n).filter_map(|k| hash.get(&at_key(at(k as f64 / n as f64)))).flatten().copied().collect();
        cand.sort_unstable();
        cand.dedup();
        for j in cand.into_iter().filter(|&j| j != i) {
            let end = t.width + ts[j].width;
            let over = (0..=n)
                .filter(|&k| {
                    let u = k as f64 / n as f64;
                    (end..=len - end).contains(&(u * len)) && seg_dist(at(u), ts[j].a, ts[j].b).0 < 0.375 * end
                })
                .count();
            if over as f64 * len / n as f64 > end {
                return None;
            }
        }
        // the cells along the centre line, sampled every quarter cell
        let m = ((len / (g.cell / 4.0)).ceil() as usize).max(1);
        let mut cells: Vec<(usize, P, f64)> = Vec::new();
        for k in 0..=m {
            let p = at(k as f64 / m as f64);
            let Some((ci, cj)) = g.cell_of(p) else { continue };
            match cells.last_mut() {
                Some(l) if l.0 == cj * g.nx + ci => (l.1, l.2) = (geom::add(l.1, p), l.2 + 1.0),
                _ => cells.push((cj * g.nx + ci, p, 1.0)),
            }
        }
        (!cells.is_empty()).then(|| Line {
            cells: cells.into_iter().map(|(c, s, k)| (c, geom::scale(s, 1.0 / k))).collect(),
            width: t.width,
            ends: [t.a, t.b],
        })
    };
    ts.iter().enumerate().map(|(i, t)| line(i, t)).collect()
}

/// Cells to join at the ends of line tracks: an end lying on another line track (within half its width)
/// with that track's nearest cell; an end off the raster's copper with the nearest copper cell within half
/// the track's width plus half a cell.
pub(crate) fn line_joins(g: &Grid, lines: &[Line], w: &[f32]) -> Vec<(usize, usize)> {
    let mut by_cell: HashMap<usize, Vec<usize>> = HashMap::new();
    for (i, l) in lines.iter().enumerate() {
        for &(c, _) in &l.cells {
            by_cell.entry(c).or_default().push(i);
        }
    }
    let mut out = vec![];
    for (i, l) in lines.iter().enumerate() {
        for (e, end) in l.ends.into_iter().enumerate() {
            let ce = if e == 0 { l.cells[0].0 } else { l.cells[l.cells.len() - 1].0 };
            let mut others: Vec<usize> =
                g.near(end, 2.5 * g.cell).iter().filter_map(|c| by_cell.get(c)).flatten().copied().collect();
            others.sort_unstable();
            others.dedup();
            for o in others.into_iter().filter(|&o| o != i) {
                let (d, q) = seg_dist(end, lines[o].ends[0], lines[o].ends[1]);
                let nearest = lines[o]
                    .cells
                    .iter()
                    .min_by(|a, b| geom::len(geom::sub(a.1, q)).total_cmp(&geom::len(geom::sub(b.1, q))));
                if let Some(&(c, _)) = nearest.filter(|_| d <= lines[o].width / 2.0 + 1e-6) {
                    out.push((ce, c));
                }
            }
            if w[ce] <= 0.0 {
                let best =
                    g.near(end, l.width / 2.0 + 0.5 * g.cell).into_iter().filter(|&c| w[c] > 0.0).min_by(|&a, &b| {
                        let d = |c: usize| geom::len(geom::sub(g.center(c % g.nx, c / g.nx), end));
                        d(a).total_cmp(&d(b))
                    });
                out.extend(best.map(|c| (ce, c)));
            }
        }
    }
    out
}

/// Link factor of a barrel's contact cell (Peaceman 1983): a cell node of the 5-point stencil fed by a point
/// source holds the potential at 0.198 cell from it; with each of its four links to full-copper neighbours
/// scaled by f = 1 / (1 − (2/π)·ln(r_w / 0.198 h)) it holds the potential of the barrel's rim (radius r_w).
/// Used for r_w < 0.7 h (f ≤ 5); larger barrels are discs. The textbook correction, a resistance
/// Rs/2π · ln(0.198 h / r_w) in series, is negative for every barrel larger than 0.198 h.
pub(crate) fn peaceman_factor(rw: f64, h: f64) -> f32 {
    (1.0 / (1.0 - std::f64::consts::FRAC_2_PI * (rw / (0.198 * h)).ln()).max(0.05)).min(20.0) as f32
}

/// Two link factors (1: none): the larger.
pub(crate) fn combine(a: f32, b: f32) -> f32 {
    if a == 1.0 {
        b
    } else if b == 1.0 {
        a
    } else {
        a.max(b)
    }
}

/// Representative of `x` in a union-find forest (smallest index of its set).
fn root(uf: &mut [u32], mut x: u32) -> u32 {
    while uf[x as usize] != x {
        let p = uf[x as usize];
        uf[x as usize] = uf[p as usize];
        x = p;
    }
    x
}

// --- the solver -------------------------------------------------------------------------

pub fn solve_rail(board: &Board, setup: &RailSetup, params: &PiParams) -> Result<PiResult, String> {
    solve_rail_scaled(board, setup, params, None)
}

/// `solve_rail` with the resistivity scaled per place: `scale(layer, point)` = ρ/ρ₂₀ there (e.g.
/// 1 + α(T − 20 °C) from a temperature map, `thermal::rail_at_temperature`). Each conductance uses
/// the mean factor of its two cells.
pub fn solve_rail_scaled(
    board: &Board,
    setup: &RailSetup,
    params: &PiParams,
    scale: Option<&dyn Fn(usize, P) -> f64>,
) -> Result<PiResult, String> {
    let su = board.stackup();
    let ncu = su.n();
    let net = find_net(board, &setup.net).ok_or_else(|| format!("net '{}' not found", setup.net))?;
    if setup.voltage == 0.0 && !setup.ground {
        return Err(format!("{}: no nominal voltage given", setup.net));
    }
    // negative rails are solved with the magnitude; the load current then
    // flows the same way relative to |V| and the result is mirrored back
    let sign = if setup.ground { 1.0 } else { setup.voltage.signum() };
    let mut warnings = Vec::new();

    // copper of the net per layer and its bounding box
    let mut bb = BBox::empty();
    let mut used = vec![false; ncu];
    for z in board.zones.iter().filter(|z| z.net == net) {
        bb.add(z.bbox.min);
        bb.add(z.bbox.max);
        used[z.layer as usize] = true;
    }
    for t in board.tracks.iter().filter(|t| t.net == net && !t.pad) {
        for p in [t.a, t.b] {
            bb.add([p[0] - t.width, p[1] - t.width]);
            bb.add([p[0] + t.width, p[1] + t.width]);
        }
        used[t.layer as usize] = true;
    }
    for p in board.pads.iter().filter(|p| p.net == net) {
        let r = p.size[0].max(p.size[1]);
        bb.add([p.at[0] - r, p.at[1] - r]);
        bb.add([p.at[0] + r, p.at[1] + r]);
        for &l in &p.layers {
            used[l as usize] = true;
        }
    }
    if !bb.is_valid() {
        return Err(format!("{}: no copper found", setup.net));
    }
    let layers: Vec<usize> = (0..ncu).filter(|&l| used[l]).collect();
    let area = (bb.max[0] - bb.min[0]) * (bb.max[1] - bb.min[1]);
    // copper area estimate: decides how many cells carry copper
    let mut cu_area: f64 = board.zones.iter().filter(|z| z.net == net).map(|z| polygon_area(&z.pts)).sum();
    let mut min_w = f64::INFINITY;
    for t in board.tracks.iter().filter(|t| t.net == net && !t.pad) {
        cu_area += geom::len(geom::sub(t.b, t.a)) * t.width;
        min_w = min_w.min(t.width);
    }
    for p in board.pads.iter().filter(|p| p.net == net) {
        cu_area += p.size[0] * p.size[1] * p.layers.len() as f64;
    }
    // limits: copper cells (solve size) and grid cells (memory)
    let by_copper = (cu_area / params.max_cells as f64).sqrt();
    let by_grid = (area * layers.len() as f64 / (params.max_cells as f64 * 8.0)).sqrt();
    let auto = by_copper.max(by_grid).max(0.05);
    let _ = min_w; // tracks narrower than a cell keep their conductance (fill_track)
    let cell = params.cell.unwrap_or_else(|| {
        let c = auto.max(0.05);
        // round up to a tidy value
        [0.05, 0.075, 0.1, 0.125, 0.15, 0.2, 0.25, 0.3, 0.4, 0.5, 0.75, 1.0].into_iter().find(|&v| v >= c).unwrap_or(c)
    });
    let g = Grid {
        x0: bb.min[0] - cell,
        y0: bb.min[1] - cell,
        cell,
        nx: (((bb.max[0] - bb.min[0]) / cell).ceil() as usize) + 2,
        ny: (((bb.max[1] - bb.min[1]) / cell).ceil() as usize) + 2,
    };
    let per = g.nx * g.ny;
    if per * layers.len() > params.max_cells * 16 {
        return Err(format!("{}: grid too large ({} cells)", setup.net, per * layers.len()));
    }

    // weights per layer: the copper fraction of every cell (cut cells); tracks narrower than `LINE_CELLS`
    // cells are line conductors, rasterised ones narrower than 1.5 cells chains of cells (`fill_track`)
    let max_w = params.line_width.unwrap_or(LINE_CELLS * cell);
    let mut w: Vec<Vec<f32>> = vec![Vec::new(); ncu];
    let mut lines: Vec<Vec<Line>> = (0..ncu).map(|_| Vec::new()).collect();
    for &l in &layers {
        let mut wl = vec![0f32; per];
        let mut polys: Vec<Vec<P>> = Vec::new();
        let ts: Vec<&Track> = board.tracks.iter().filter(|t| t.net == net && !t.pad && t.layer as usize == l).collect();
        for (t, line) in ts.iter().zip(line_tracks(&g, &ts, max_w)) {
            match line {
                Some(x) => lines[l].push(x),
                None if t.width < 1.5 * cell => fill_track(&g, t.a, t.b, t.width, &mut wl),
                None => polys.push(capsule(t)),
            }
        }
        let pads: Vec<&Pad> = board.pads.iter().filter(|p| p.net == net && p.layers.contains(&(l as u16))).collect();
        polys.extend(pads.iter().flat_map(|p| pad_polygons(p)));
        let zs = board.zones.iter().filter(|z| z.net == net && z.layer as usize == l).map(|z| z.pts.as_slice());
        fill_fraction(&g, &zs.chain(polys.iter().map(|p| p.as_slice())).collect::<Vec<_>>(), 4, &mut wl);
        // a pad smaller than a cell: its cell
        for k in pads.iter().flat_map(|p| pad_cells(&g, p)) {
            if wl[k] <= 0.0 {
                wl[k] = 1.0;
            }
        }
        w[l] = wl;
    }

    // node numbering: copper cells and the cells of line tracks
    let mut node_of: Vec<Vec<u32>> = vec![Vec::new(); ncu];
    let mut node_pos: Vec<(usize, usize)> = Vec::new(); // (layer, cell)
    for &l in &layers {
        let mut m = vec![u32::MAX; per];
        for &(k, _) in lines[l].iter().flat_map(|x| &x.cells) {
            m[k] = 0;
        }
        for k in 0..per {
            if w[l][k] > 0.0 || m[k] == 0 {
                m[k] = node_pos.len() as u32;
                node_pos.push((l, k));
            }
        }
        node_of[l] = m;
    }

    // vias and through-hole pads of the net: the contact cell on every layer of the barrel
    let mut verticals: Vec<(P, f64, Vec<usize>)> = Vec::new();
    for v in board.vias.iter().filter(|v| v.net == net) {
        verticals.push((v.at, v.drill, (v.from as usize..=v.to as usize).collect()));
    }
    for p in board.pads.iter().filter(|p| p.net == net && p.drill > 0.0) {
        verticals.push((p.at, p.drill, p.layers.iter().map(|&l| l as usize).collect()));
    }
    // the cell at a point, else the nearest copper cell within r
    let find_cell = |l: usize, at: P, r: f64| -> Option<usize> {
        let m = &node_of[l];
        let (ci, cj) = g.cell_of(at)?;
        if m.is_empty() || m[cj * g.nx + ci] != u32::MAX {
            return (!m.is_empty()).then_some(cj * g.nx + ci);
        }
        let d = |k: usize| geom::len(geom::sub(g.center(k % g.nx, k / g.nx), at));
        g.near(at, r).into_iter().filter(|&k| m[k] != u32::MAX).min_by(|&a, &b| d(a).total_cmp(&d(b)))
    };
    let mut uf: Vec<u32> = (0..node_pos.len() as u32).collect();
    let join = |uf: &mut [u32], a: u32, b: u32| {
        let (ra, rb) = (root(uf, a), root(uf, b));
        uf[ra.max(rb) as usize] = ra.min(rb);
    };
    let mut pfac = vec![1f32; node_pos.len()];
    let mut contacts: Vec<Vec<(usize, usize)>> = Vec::new(); // per vertical: (layer, cell)
    for (at, drill, ls) in &verticals {
        let rw = drill / 2.0;
        let mut cl = Vec::new();
        for &l in ls.iter().filter(|&&l| l < ncu && used[l]) {
            let Some(k) = find_cell(l, *at, rw + g.cell) else { continue };
            let n = node_of[l][k];
            if rw < 0.7 * g.cell {
                pfac[n as usize] = combine(pfac[n as usize], peaceman_factor(rw, g.cell));
            } else {
                // an equipotential disc: the cells within r_w + 0.35 cell (cells tied at their centres
                // short about a third of a cell beyond them)
                for k2 in g.near(*at, rw + 0.35 * g.cell) {
                    if node_of[l][k2] != u32::MAX {
                        join(&mut uf, n, node_of[l][k2]);
                    }
                }
            }
            cl.push((l, k));
        }
        contacts.push(cl);
    }
    for &l in &layers {
        for (a, b) in line_joins(&g, &lines[l], &w[l]) {
            join(&mut uf, node_of[l][a], node_of[l][b]);
        }
    }
    // merged nodes: renumbered (their position is the first cell, their link factor the larger)
    let mut newid = vec![u32::MAX; node_pos.len()];
    let mut pos = Vec::new();
    let mut pf = Vec::new();
    for i in 0..node_pos.len() {
        let r = root(&mut uf, i as u32) as usize;
        if newid[r] == u32::MAX {
            newid[r] = pos.len() as u32;
            pos.push(node_pos[i]);
            pf.push(1.0);
        }
        newid[i] = newid[r];
        pf[newid[i] as usize] = combine(pf[newid[i] as usize], pfac[i]);
    }
    for x in node_of.iter_mut().flatten().filter(|x| **x != u32::MAX) {
        *x = newid[*x as usize];
    }
    let (node_pos, pfac) = (pos, pf);
    let nn = node_pos.len();
    // resistivity factor per node (empty: 1 everywhere) and the mean of an edge's two nodes
    let div: Vec<f64> = match scale {
        Some(f) => node_pos.iter().map(|&(l, k)| f(l, g.center(k % g.nx, k / g.nx)).max(1e-3)).collect(),
        None => Vec::new(),
    };
    let dv = |a: u32, b: u32| -> f64 {
        if div.is_empty() {
            1.0
        } else {
            0.5 * (div[a as usize] + div[b as usize])
        }
    };

    // edges: (a, b, conductance S)
    let mut edges: Vec<(u32, u32, f64)> = Vec::with_capacity(nn * 2);
    let mut line_edges: Vec<(usize, f64)> = Vec::new(); // (edge, cross-section mm²)
    for &l in &layers {
        let gs = su.copper[l].thickness * 1e-3 / params.rho; // sheet conductance (S per square)
        let (wl, m) = (&w[l], &node_of[l]);
        for j in 0..g.ny {
            for i in 0..g.nx {
                let k = j * g.nx + i;
                for (ni, nj) in [(i + 1, j), (i, j + 1)] {
                    let k2 = nj * g.nx + ni;
                    if ni >= g.nx || nj >= g.ny || wl[k] <= 0.0 || wl[k2] <= 0.0 || m[k] == m[k2] {
                        continue;
                    }
                    let (a, b) = (wl[k] as f64, wl[k2] as f64);
                    let mut wf = 2.0 * a * b / (a + b);
                    let f = combine(pfac[m[k] as usize], pfac[m[k2] as usize]);
                    if f != 1.0 && a >= 1.0 && b >= 1.0 {
                        wf *= f as f64; // a barrel's contact cell
                    }
                    edges.push((m[k], m[k2], gs * wf / dv(m[k], m[k2])));
                }
            }
        }
        // line tracks: between consecutive cells, not where both are full copper (that carries it)
        for x in &lines[l] {
            for c in x.cells.windows(2) {
                let ((ka, pa), (kb, pb)) = (c[0], c[1]);
                let (na, nb) = (m[ka], m[kb]);
                if na != nb && (wl[ka] < 1.0 || wl[kb] < 1.0) {
                    line_edges.push((edges.len(), x.width * su.copper[l].thickness));
                    let len = geom::len(geom::sub(pb, pa)).max(1e-6);
                    edges.push((na, nb, gs * x.width / len / dv(na, nb)));
                }
            }
        }
    }
    // vertical: the barrels between their contact cells
    let mut via_edges: Vec<(usize, P, f64)> = Vec::new(); // (edge index, position, drill)
    let layer_z: Vec<f64> = {
        // centre height of each copper layer (mm from top)
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
    for ((at, drill, _), cl) in verticals.iter().zip(&contacts) {
        for pair in cl.windows(2) {
            let ((pl, pk), (l, k)) = (pair[0], pair[1]);
            let (pn, nid) = (node_of[pl][pk], node_of[l][k]);
            if pn != nid {
                let len = (layer_z[l] - layer_z[pl]).abs() * 1e-3;
                let gv = barrel(*drill) / (params.rho * len.max(1e-6)) / dv(pn, nid);
                via_edges.push((edges.len(), *at, *drill));
                edges.push((pn, nid, gv));
            }
        }
    }

    // sources and loads
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
    let pad_nodes = |p: &Pad| -> Vec<u32> {
        let mut v = Vec::new();
        for &l in &p.layers {
            let l = l as usize;
            if node_of[l].is_empty() {
                continue;
            }
            for k in pad_cells(&g, p) {
                let n = node_of[l][k];
                if n != u32::MAX {
                    v.push(n);
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
            ns.extend(pad_nodes(p));
        }
        for &n in &ns {
            fixed[n as usize] = true;
        }
        src_nodes.push((s.refdes.clone(), ns, pads.len()));
    }
    if !fixed.iter().any(|&f| f) {
        return Err(if setup.ground {
            format!("{}: the supply input (PI_REG input) has no pads on this net", setup.net)
        } else {
            format!("{}: no source defined (field PI_SOURCE or selection in the viewer)", setup.net)
        });
    }
    let mut inject = vec![0f64; nn];
    let mut load_nodes: Vec<(usize, Vec<u32>, usize)> = Vec::new(); // (load index, nodes, pads)
    let mut load_pad_nodes: Vec<Vec<Vec<u32>>> = Vec::new(); // per load, per pad
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
            for &n in &ns {
                inject[n as usize] -= per_pad / ns.len() as f64;
            }
            all.extend(ns.iter().copied());
            per_pad_nodes.push(ns);
        }
        load_nodes.push((li, all, pads.len()));
        load_pad_nodes.push(per_pad_nodes);
    }

    // connectivity from the sources
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
    let dropped = reach.iter().filter(|r| !**r).count();
    if dropped > 0 {
        warnings.push(format!("{} cells without connection to a source (copper islands) ignored", dropped));
    }

    // unknowns: reachable, not fixed
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
    let x = if nu > 0 {
        kisi_sparse::solve_spd_triplets(nu, &trip, &rhs).map_err(|c| format!("linear solver: error {c}"))?
    } else {
        vec![]
    };
    let mut v = vec![f64::NAN; nn];
    for i in 0..nn {
        if fixed[i] {
            v[i] = v_src;
        } else if uidx[i] != u32::MAX {
            v[i] = x[uidx[i] as usize];
        }
    }

    // per-layer buffers, current density (in a cell of a line track: the track's, I / (w·t))
    let mut jline = vec![0f64; nn];
    for &(e, area) in &line_edges {
        let (a, b, gv) = edges[e];
        let j = (gv * (v[a as usize] - v[b as usize])).abs() / area;
        if j.is_finite() {
            jline[a as usize] = jline[a as usize].max(j);
            jline[b as usize] = jline[b as usize].max(j);
        }
    }
    let mut grids = Vec::new();
    let mut vb: Vec<f32> = Vec::with_capacity(per * layers.len());
    let mut jb: Vec<f32> = Vec::with_capacity(per * layers.len());
    let mut cb: Vec<u8> = Vec::with_capacity(per * layers.len());
    let mut max_j = 0.0f64;
    let mut max_j_at = None;
    let cm = g.cell * 1e-3;
    for &l in &layers {
        let off = vb.len();
        let (m, wl) = (&node_of[l], &w[l]);
        // copper area in cell areas: the cell's own fraction; a line conductor's cell its width / cell (the
        // track's copper along its step, which can exceed the cell) – not in full raster copper, where the
        // raster carries the current and the line has no conductance (see the line edges)
        let mut fr: Vec<f32> = wl.clone();
        for x in &lines[l] {
            let f = (x.width / g.cell) as f32;
            for &(k, _) in &x.cells {
                if wl[k] < 1.0 {
                    fr[k] = fr[k].max(f);
                }
            }
        }
        // the voltage of a raster cell (not of a cell on a line track only)
        let val = |i: isize, j: isize| -> Option<f64> {
            if i < 0 || j < 0 || i as usize >= g.nx || j as usize >= g.ny || wl[j as usize * g.nx + i as usize] <= 0.0 {
                return None;
            }
            let n = m[j as usize * g.nx + i as usize];
            let x = v[n as usize];
            x.is_finite().then_some(x)
        };
        for j in 0..g.ny as isize {
            for i in 0..g.nx as isize {
                let n = m[j as usize * g.nx + i as usize];
                match (n != u32::MAX).then(|| v[n as usize]).filter(|x| x.is_finite()) {
                    None => {
                        vb.push(f32::NAN);
                        jb.push(f32::NAN);
                        cb.push(0);
                    }
                    Some(c) => {
                        vb.push(c as f32);
                        let f = fr[j as usize * g.nx + i as usize].clamp(0.0, LINE_CELLS as f32);
                        cb.push(if f > 0.0 { ((f * CU_STEPS).round() as u8).max(1) } else { 0 });
                        let d = |a: Option<f64>, b: Option<f64>| -> f64 {
                            match (a, b) {
                                (Some(a), Some(b)) => (b - a) / (2.0 * cm),
                                (Some(a), None) => (c - a) / cm,
                                (None, Some(b)) => (b - c) / cm,
                                _ => 0.0,
                            }
                        };
                        let (gx, gy) = if wl[j as usize * g.nx + i as usize] > 0.0 {
                            (d(val(i - 1, j), val(i + 1, j)), d(val(i, j - 1), val(i, j + 1)))
                        } else {
                            (0.0, 0.0)
                        };
                        let dn = if div.is_empty() { 1.0 } else { div[n as usize] };
                        let jm = ((gx * gx + gy * gy).sqrt() / (params.rho * dn) * 1e-6).max(jline[n as usize]); // A/mm²
                        if jm > max_j {
                            max_j = jm;
                            max_j_at = Some(g.center(i as usize, j as usize));
                        }
                        jb.push(jm as f32);
                    }
                }
            }
        }
        grids.push(LayerGrid { layer: l as u16, x0: g.x0, y0: g.y0, nx: g.nx, ny: g.ny, offset: off });
    }

    // loads
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
    // source currents and losses
    let mut src_cur = vec![0f64; src_nodes.len()];
    let mut owner: HashMap<u32, usize> = HashMap::new();
    for (si, (_, ns, _)) in src_nodes.iter().enumerate() {
        for &n in ns {
            owner.insert(n, si);
        }
    }
    let mut loss = 0.0;
    // loss per cell, in the same layout as the voltage buffer
    let layer_slot: Vec<usize> = {
        let mut ls = vec![usize::MAX; ncu];
        for (i, &l) in layers.iter().enumerate() {
            ls[l] = i;
        }
        ls
    };
    let mut lb = vec![0f32; per * layers.len()];
    for &(a, b, gv) in &edges {
        let (va, vb2) = (v[a as usize], v[b as usize]);
        if !(va.is_finite() && vb2.is_finite()) {
            continue;
        }
        let pe = gv * (va - vb2) * (va - vb2);
        loss += pe;
        for n in [a, b] {
            let (l, k) = node_pos[n as usize];
            lb[layer_slot[l] * per + k] += (pe / 2.0) as f32;
        }
        // current leaving fixed (source) nodes into the rest of the copper
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
        // largest ground offset (either sign) at a component
        loads.iter().flat_map(|l| [l.v_min, l.v_max]).flatten().map(f64::abs).reduce(f64::max).map(|x| x * 1e3)
    } else {
        v_min.map(|m| (v_src - m) * 1e3)
    };
    if sign < 0.0 {
        for x in vb.iter_mut() {
            *x = -*x;
        }
        for l in loads.iter_mut() {
            (l.v_min, l.v_max) = (l.v_max.map(|v| -v), l.v_min.map(|v| -v));
            l.v_mean = l.v_mean.map(|v| -v);
        }
    }
    let v_min = v_min.map(|v| v * sign);
    Ok(PiResult {
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
        v: vb,
        j: jb,
        loss: lb,
        cu: cb,
    })
}

/// Potential difference between the rims (radius a) of a source at (x1, y) and a sink at (x2, y) in a
/// w × h sheet with insulating edges (method of images), per ampere and Ω per square.
#[cfg(test)]
pub(crate) fn sheet_pair(w: f64, h: f64, x1: f64, x2: f64, y: f64, a: f64) -> f64 {
    use std::f64::consts::PI;
    let pot = |px: f64, py: f64| {
        let mut v = 0.0;
        for i in -30i32..=30 {
            for j in -30i32..=30 {
                for (sx, sign) in [(x1, -1.0), (x2, 1.0)] {
                    for (mx, my) in [(1.0, 1.0), (-1.0, 1.0), (1.0, -1.0), (-1.0, -1.0)] {
                        let (ix, iy) = (2.0 * w * i as f64 + mx * sx, 2.0 * h * j as f64 + my * y);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::load_pcb;

    #[test]
    fn names_and_quantities() {
        assert_eq!(voltage_from_name("+3V3"), Some(3.3));
        assert_eq!(voltage_from_name("/Power/VCC_1V8"), Some(1.8));
        assert_eq!(voltage_from_name("+5V"), Some(5.0));
        assert_eq!(voltage_from_name("+1.2V"), Some(1.2));
        assert_eq!(voltage_from_name("12V_IN"), Some(12.0));
        assert_eq!(voltage_from_name("GND"), None);
        assert_eq!(voltage_from_name("-5V"), Some(-5.0));
        assert_eq!(voltage_from_name("SPI_CLK"), None);
        assert_eq!(parse_quantity("300mA"), Some((0.3, 'A')));
        assert_eq!(parse_quantity("2W"), Some((2.0, 'W')));
        assert_eq!(parse_quantity("3,3V"), Some((3.3, 'V')));
        assert_eq!(parse_quantity("1.5"), Some((1.5, 'A')));
    }

    #[test]
    fn supply_names() {
        for n in [
            "VBUS",
            "VSYS",
            "VBAT",
            "/Power/VCC",
            "Net-(IC22-VCC)",
            "Net-(U8A-VBUS)",
            "VDDIO",
            "AVDD",
            "/06 Peripherals/IMU_VDD",
            "LSIN1/VINLDO1_B4",
            "PVIN",
            "VCC_IN",
            "VIO",
            "VDD_8",
        ] {
            assert!(supply_name_hint(n), "{n}");
        }
        for n in [
            "GND",
            "VBUS_DET",
            "VBAT_SENSE",
            "VCC_EN",
            "VDD_PG",
            "VINP",
            "VIN+",
            "Net-(U3-VIN-)",
            "VREF",
            "SDA",
            "unconnected-(U2-VBUS-Pad1)",
            "DECA",
            "VSS",
            "VOUT",
            "P1.17{slash}PMIC_SCL",
            "LDO1_IN",
        ] {
            assert!(!supply_name_hint(n), "{n}");
        }
        assert!(
            signal_name("/FPGA/FPGA Voltage Regs/PG_1V0")
                && signal_name("TRIM_SCL_5V")
                && !signal_name("+3V3")
                && !signal_name("3V3_HBLB_SWITCH")
        );
    }

    /// A 1 mm x 20 mm strip of 35 µm copper with a source pad at one end and a
    /// 1 A load at the other: R = rho L / (w t) between the pad centres.
    fn strip_board(width: f64) -> String {
        format!(
            r#"(kicad_pcb (version 20241229) (generator "pcbnew")
  (layers (0 "F.Cu" signal) (2 "B.Cu" signal))
  (setup (stackup
    (layer "F.Cu" (type "copper") (thickness 0.035))
    (layer "dielectric 1" (type "core") (thickness 1.5) (material "FR4") (epsilon_r 4.5) (loss_tangent 0.02))
    (layer "B.Cu" (type "copper") (thickness 0.035))))
  (net 0 "") (net 1 "+3V3")
  (footprint "R" (layer "F.Cu") (at 0 0)
    (property "Reference" "J1") (property "PI_SOURCE" "+3V3")
    (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net 1 "+3V3")))
  (footprint "R" (layer "F.Cu") (at 20 0)
    (property "Reference" "U1") (property "PI_LOAD" "+3V3=1A")
    (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net 1 "+3V3")))
  (segment (start 0 0) (end 20 0) (width {width}) (layer "F.Cu") (net 1))
)"#
        )
    }

    #[test]
    fn strip_resistance() {
        for (width, cell) in [(1.0, 0.05), (1.0, 0.1), (0.2, 0.1)] {
            let b = load_pcb(&strip_board(width)).unwrap();
            let (rails, warn) = rails_from_fields(&b);
            assert!(warn.is_empty(), "{warn:?}");
            assert_eq!(rails.len(), 1);
            assert_eq!(rails[0].voltage, 3.3);
            let r = solve_rail(&b, &rails[0], &PiParams { cell: Some(cell), ..Default::default() }).unwrap();
            // pads are 1 mm squares: the strip between the pad edges is 19 mm
            // long, the pads themselves add ~2 x 0.5 squares of the same width
            let l = if width >= 1.0 { 20.0 } else { 19.0 };
            let expect = 1.72e-8 * (l * 1e-3) / (width * 1e-3 * 35e-6); // ohm, 1 A
            let drop = r.max_drop_mv.unwrap() * 1e-3;
            let err = (drop - expect) / expect;
            eprintln!(
                "strip w={width} cell={cell}: {:.2} mV vs {:.2} mV ({:+.1} %)",
                drop * 1e3,
                expect * 1e3,
                err * 100.0
            );
            assert!(err.abs() < 0.08, "w={width} cell={cell}: drop {drop} V vs {expect} V ({:.1} %)", err * 100.0);
            assert!((r.sources[0].current - 1.0).abs() < 1e-6, "source current {}", r.sources[0].current);
        }
    }

    /// Spreading resistance of a plane: a 100 mm x 100 mm pour of 35 µm copper, a source pad and a 1 A load pad
    /// (round, 2 mm) 20 mm apart in the middle. Reference: two discs in a sheet, R = Rs/π · acosh(d / 2a), with the
    /// pour's insulating edges as image sources (method of images, Neumann boundary), plus the load pad's own
    /// spreading (current drawn evenly from the pad).
    #[test]
    fn plane_spreading_resistance() {
        let (w, h, a, x1, x2, y) = (100.0, 100.0, 1.0, 40.0, 60.0, 50.0);
        let text = format!(
            r#"(kicad_pcb (version 20241229) (generator "pcbnew")
  (layers (0 "F.Cu" signal) (2 "B.Cu" signal))
  (setup (stackup
    (layer "F.Cu" (type "copper") (thickness 0.035))
    (layer "dielectric 1" (type "core") (thickness 1.5) (material "FR4") (epsilon_r 4.5) (loss_tangent 0.02))
    (layer "B.Cu" (type "copper") (thickness 0.035))))
  (net 0 "") (net 1 "+3V3")
  (footprint "R" (layer "F.Cu") (at {x1} {y})
    (property "Reference" "J1") (property "PI_SOURCE" "+3V3")
    (pad "1" smd circle (at 0 0) (size {d} {d}) (layers "F.Cu") (net 1 "+3V3")))
  (footprint "R" (layer "F.Cu") (at {x2} {y})
    (property "Reference" "U1") (property "PI_LOAD" "+3V3=1A")
    (pad "1" smd circle (at 0 0) (size {d} {d}) (layers "F.Cu") (net 1 "+3V3")))
  (zone (net 1) (net_name "+3V3") (layer "F.Cu") (filled_polygon (layer "F.Cu") (pts (xy 0 0) (xy {w} 0) (xy {w} {h}) (xy 0 {h}))))
)"#,
            d = 2.0 * a
        );
        let rs = 1.72e-8 / 35e-6; // ohm per square
                                  // potential of the source/sink pair with all images of the rectangle (Neumann edges), per ampere
        let pot = |px: f64, py: f64| {
            let mut v = 0.0;
            for i in -40i32..=40 {
                for j in -40i32..=40 {
                    for (sx, sign) in [(x1, -1.0), (x2, 1.0)] {
                        for (mx, my) in [(1.0, 1.0), (-1.0, 1.0), (1.0, -1.0), (-1.0, -1.0)] {
                            let ix = 2.0 * w * i as f64 + mx * sx;
                            let iy = 2.0 * h * j as f64 + my * y;
                            let r2 = (px - ix).powi(2) + (py - iy).powi(2);
                            v += sign * rs / (4.0 * std::f64::consts::PI) * r2.ln();
                        }
                    }
                }
            }
            v
        };
        // the contacts' rims (averaged); kisi holds the source pad at the source voltage and draws the load
        // current evenly from the load pad's copper, which adds Rs/4π between the load pad's rim and its centre
        let rim = |cx: f64| {
            (0..16)
                .map(|k| {
                    let t = k as f64 * std::f64::consts::PI / 8.0;
                    pot(cx + a * t.cos(), y + a * t.sin())
                })
                .sum::<f64>()
                / 16.0
        };
        let expect = rim(x1) - rim(x2) + rs / (4.0 * std::f64::consts::PI);
        let infinite = rs / std::f64::consts::PI * ((x2 - x1) / (2.0 * a)).acosh();
        let b = load_pcb(&text).unwrap();
        let (rails, warn) = rails_from_fields(&b);
        assert!(warn.is_empty(), "{warn:?}");
        let r = solve_rail(&b, &rails[0], &PiParams { cell: Some(0.25), ..Default::default() }).unwrap();
        let drop = r.max_drop_mv.unwrap() * 1e-3;
        let err = (drop - expect) / expect;
        eprintln!(
            "plane: {:.4} mV vs {:.4} mV with edges ({:.4} mV infinite sheet), {:+.1} %",
            drop * 1e3,
            expect * 1e3,
            infinite * 1e3,
            err * 100.0
        );
        assert!(err.abs() < 0.05, "plane spreading: {drop} V vs {expect} V ({:+.1} %)", err * 100.0);
    }
    // --- analytic cases of the raster corrections (docs/research/pi-quadtree.md) ---

    const RS: f64 = 1.72e-8 / 35e-6; // Ω per square, 35 µm copper

    /// A two-layer board (35 µm copper, 1.5 mm core) with net 1 "+3V3" and the given items.
    fn board(items: &str) -> Board {
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
    fn zone(pts: &[(f64, f64)]) -> String {
        let p: String = pts.iter().map(|(x, y)| format!("(xy {x} {y}) ")).collect();
        format!(r#"  (zone (net 1) (net_name "+3V3") (layer "F.Cu") (filled_polygon (layer "F.Cu") (pts {p})))"#)
    }
    fn via(x: f64, y: f64, drill: f64) -> String {
        format!(r#"  (via (at {x} {y}) (size {}) (drill {drill}) (layers "F.Cu" "B.Cu") (net 1))"#, 2.0 * drill)
    }
    fn segment(a: (f64, f64), b: (f64, f64), w: f64) -> String {
        format!(r#"  (segment (start {} {}) (end {} {}) (width {w}) (layer "F.Cu") (net 1))"#, a.0, a.1, b.0, b.1)
    }
    /// Source J1 and a 1 A load U1, each with one pad.
    fn ends(src: &str, load: &str) -> String {
        part("J1", r#"(property "PI_SOURCE" "+3V3")"#, src)
            + "\n"
            + &part("U1", r#"(property "PI_LOAD" "+3V3=1A")"#, load)
    }
    fn solve_at(b: &Board, cell: f64, line_width: Option<f64>) -> PiResult {
        let (rails, warn) = rails_from_fields(b);
        assert!(warn.is_empty(), "{warn:?}");
        solve_rail(b, &rails[0], &PiParams { cell: Some(cell), line_width, ..Default::default() }).unwrap()
    }
    /// The voltage of the cell at `p` on copper layer `l`.
    fn v_at(r: &PiResult, l: u16, p: P) -> f64 {
        let lg = r.layers.iter().find(|x| x.layer == l).unwrap();
        let (i, j) = (((p[0] - lg.x0) / r.cell) as usize, ((p[1] - lg.y0) / r.cell) as usize);
        r.v[lg.offset + j * lg.nx + i] as f64
    }
    /// Two vias (0.3 mm drill) 6 mm apart in a 20 mm × 20 mm plane: the plane's resistance between the two
    /// barrels is the one between two discs of the barrel radius (images for the edges). Without the contact
    /// correction a cell contact is a disc of 0.198 cell: +29 % at 0.25 mm cells, +53 % at 0.1 mm. Peaceman's
    /// link factor (0.25, 0.5 mm: r_w < 0.7 cell) and the disc contact (0.1 mm) get it at any cell.
    #[test]
    fn two_vias_in_a_sheet() {
        let items = [
            zone(&[(0.0, 0.0), (20.0, 0.0), (20.0, 20.0), (0.0, 20.0)]),
            via(7.0, 10.0, 0.3),
            via(13.0, 10.0, 0.3),
            ends(&circle_pad(1, 7.0, 10.0, 0.6, "B.Cu"), &circle_pad(1, 13.0, 10.0, 0.6, "B.Cu")),
        ]
        .join("\n");
        let b = board(&items);
        let expect = RS * sheet_pair(20.0, 20.0, 7.0, 13.0, 10.0, 0.15);
        for cell in [0.1, 0.15, 0.25, 0.5] {
            let r = solve_at(&b, cell, None);
            let d = v_at(&r, 0, [7.0, 10.0]) - v_at(&r, 0, [13.0, 10.0]);
            let e = d / expect - 1.0;
            eprintln!("two vias cell {cell}: {:.4} mΩ vs {:.4} mΩ {:+.2} %", d * 1e3, expect * 1e3, e * 100.0);
            assert!(e.abs() < 0.03, "cell {cell}: {:+.2} %", e * 100.0);
        }
    }

    /// A via in the middle of a round plane fed from a ring of pads: Rs/2π · ln(R/r_w) between the ring's
    /// inner edge and the barrel (+33 % without the contact correction at 0.25 mm cells).
    #[test]
    fn radial_via() {
        use std::f64::consts::PI;
        let (ro, d, n) = (5.0, 0.8, 64);
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
        let items =
            [zone(&disc), via(10.0, 10.0, 0.3), ends(&ring, &circle_pad(1, 10.0, 10.0, 0.6, "B.Cu"))].join("\n");
        let b = board(&items);
        // the ring's inner edge: pads of diameter d every 2πR/64 ≈ 0.49 mm, scalloped; its mean radius
        let gap = PI * ro / n as f64;
        let r_in = ro - ((d / 2.0).powi(2) - gap * gap / 3.0).sqrt();
        let expect = RS / (2.0 * PI) * (r_in / 0.15).ln();
        for cell in [0.1, 0.25] {
            let r = solve_at(&b, cell, None);
            let e = (3.3 - v_at(&r, 0, [10.0, 10.0])) / expect - 1.0;
            eprintln!("radial via cell {cell}: {:+.2} %", e * 100.0);
            assert!(e.abs() < 0.03, "cell {cell}: {:+.2} %", e * 100.0);
        }
    }

    /// A 4 mm strip, 20 mm long, with a 1 mm × 2 mm neck in the middle: the strip's squares plus the
    /// constriction resistance of the two steps (conformal mapping, ratio α = w/W):
    /// ΔR = Rs/π · [(1+α²)/(2α) · ln((1+α)/(1−α)) − ln(4α/(1−α²))].
    #[test]
    fn strip_with_neck() {
        use std::f64::consts::PI;
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
        let rect = |x: f64| format!(r#"(pad "1" smd rect (at {x} 2) (size 1 4) (layers "F.Cu") (net 1 "+3V3"))"#);
        let b = board(&[zone(&pts), ends(&rect(0.5), &rect(19.5))].join("\n"));
        let a: f64 = 0.25;
        let step =
            1.0 / PI * ((1.0 + a * a) / (2.0 * a) * ((1.0 + a) / (1.0 - a)).ln() - (4.0 * a / (1.0 - a * a)).ln());
        // 16 mm of the wide strip (between the pads), the neck, half of the load pad (current drawn evenly)
        let expect = RS * (16.0 / 4.0 + 2.0 / 1.0 + 0.5 / 4.0 + 2.0 * step);
        let e = solve_at(&b, 0.25, None).max_drop_mv.unwrap() * 1e-3 / expect - 1.0;
        assert!(e.abs() < 0.03, "{:+.2} %", e * 100.0);
    }

    /// A pour 1.13 mm wide (4.5 cells) off the grid: with copper at the cells' centres its width rounds to 4
    /// or 5 rows of cells (−11 … +11 %), with the copper fraction of the edge cells (cut cells) it is kept.
    #[test]
    fn cut_cells() {
        let (y0, wd) = (0.07, 1.13);
        let pts = [(0.0, y0), (20.0, y0), (20.0, y0 + wd), (0.0, y0 + wd)];
        let rect = |x: f64| {
            format!(r#"(pad "1" smd rect (at {x} {}) (size 1 {wd}) (layers "F.Cu") (net 1 "+3V3"))"#, y0 + wd / 2.0)
        };
        let b = board(&[zone(&pts), ends(&rect(0.5), &rect(19.5))].join("\n"));
        // 18 mm between the pads, half of the load pad (current drawn evenly)
        let expect = RS * 18.5 / wd;
        for cell in [0.2, 0.25] {
            let e = solve_at(&b, cell, None).max_drop_mv.unwrap() * 1e-3 / expect - 1.0;
            eprintln!("pour {wd} mm on {cell} mm cells: {:+.2} %", e * 100.0);
            assert!(e.abs() < 0.02, "cell {cell}: {:+.2} %", e * 100.0);
        }
    }

    /// Tracks at 0°, 30° and 45°, 0.15 … 0.5 mm wide, 10 mm long, on a 0.2 mm raster: as line conductors
    /// R = Rs · L / w at any angle (as a staircase of cells up to +110 %), the current density of the track
    /// I / (w·t) in its cells.
    #[test]
    fn track_direction() {
        let len = 10.0;
        for w in [0.15, 0.3, 0.5] {
            for deg in [0.0f64, 30.0, 45.0] {
                let (a, b) = ((1.0, 1.0), (1.0 + len * deg.to_radians().cos(), 1.0 + len * deg.to_radians().sin()));
                let items = [
                    segment(a, b, w),
                    ends(&circle_pad(1, a.0, a.1, 0.05, "F.Cu"), &circle_pad(1, b.0, b.1, 0.05, "F.Cu")),
                    // fixes the grid origin off the track's diagonal
                    part("TP1", "", &circle_pad(1, -1.013, -1.029, 0.05, "F.Cu")),
                ]
                .join("\n");
                let r = solve_at(&board(&items), 0.2, None);
                let e = r.max_drop_mv.unwrap() * 1e-3 / (RS * len / w) - 1.0;
                let ej = r.max_j / (1.0 / (w * 0.035)) - 1.0;
                eprintln!("track {w} mm {deg}°: {:+.2} %, current density {:+.2} %", e * 100.0, ej * 100.0);
                // the line starts at the centre of the end cells, not at the track's ends: up to −1.5 %
                assert!(
                    e.abs() < 0.03 && ej.abs() < 0.03,
                    "{w} mm {deg}°: {:+.2} %, j {:+.2} %",
                    e * 100.0,
                    ej * 100.0
                );
            }
        }
    }

    /// 45° tracks that the raster used to break apart: a thin track whose centre line runs through cell
    /// centres (its side cells exactly at half the cell diagonal, dropped by rounding) and a track 1.05 … 1.41
    /// cells wide (filled by its width alone: one diagonal row of cells, not 4-connected). Rasterised
    /// (`line_width` 0) they stay connected; as line conductors they have the track's resistance.
    #[test]
    fn diagonal_breaks() {
        for (w, x0) in [(0.15, 1.0), (0.25, 1.0), (0.25, 1.05), (0.27, 1.1)] {
            let (a, b) = ((x0, 1.0), (x0 + 7.071, 8.071));
            let items =
                [segment(a, b, w), ends(&circle_pad(1, a.0, a.1, 0.1, "F.Cu"), &circle_pad(1, b.0, b.1, 0.1, "F.Cu"))];
            let bd = board(&items.join("\n"));
            let raster = solve_at(&bd, 0.2, Some(0.0));
            assert!(raster.loads[0].connected && raster.warnings.is_empty(), "{w} mm from {x0}: {:?}", raster.warnings);
            let e = solve_at(&bd, 0.2, None).max_drop_mv.unwrap() / (RS * 10.0 / w * 1e3) - 1.0;
            assert!(e.abs() < 0.03, "{w} mm from {x0}: {:+.2} %", e * 100.0);
        }
    }

    /// Line tracks joined at their ends: a T (the stem's end on the bar's centre line, off the bar's cells'
    /// centres) and an L whose corner lies on a cell boundary; R = Rs · Σ L / w along the current's path.
    #[test]
    fn line_junctions() {
        let (wa, wb) = (0.2, 0.12);
        let items = [
            segment((1.0, 3.03), (9.0, 3.03), wa),
            segment((5.1, 3.03), (5.1, 8.0), wb),
            segment((5.1, 8.0), (8.0, 8.0), wb),
            ends(&circle_pad(1, 1.0, 3.03, 0.05, "F.Cu"), &circle_pad(1, 8.0, 8.0, 0.05, "F.Cu")),
        ];
        let r = solve_at(&board(&items.join("\n")), 0.2, None);
        let expect = RS * (4.1 / wa + (4.97 + 2.9) / wb);
        let e = r.max_drop_mv.unwrap() * 1e-3 / expect - 1.0;
        assert!(e.abs() < 0.03, "{:+.2} % {:?}", e * 100.0, r.warnings);
    }

    /// Rail setups written as fields read back as the same rails (the pads chosen on a part are
    /// reported as lost).
    #[test]
    fn setups_to_fields_round_trip() {
        let pcb = r#"(kicad_pcb (version 20240108) (generator "pcbnew") (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
  (net 0 "") (net 1 "+3V3") (net 2 "VCORE") (net 3 "/A B/VDD 1")
  (footprint "J" (layer "F.Cu") (at 0 0) (property "Reference" "J1") (property "Value" "x") (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net 1 "+3V3")) (pad "2" smd rect (at 0 1) (size 1 1) (layers "F.Cu") (net 2 "VCORE")))
  (footprint "U" (layer "F.Cu") (at 3 0) (property "Reference" "U1") (property "Value" "x") (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net 1 "+3V3")) (pad "2" smd rect (at 0 1) (size 1 1) (layers "F.Cu") (net 2 "VCORE")) (pad "3" smd rect (at 0 2) (size 1 1) (layers "F.Cu") (net 3 "/A B/VDD 1"))))"#;
        let setups = vec![
            RailSetup {
                net: "+3V3".into(),
                voltage: 3.3,
                sources: vec![SourceSpec { refdes: "J1".into(), pads: vec![] }],
                loads: vec![LoadSpec { refdes: "U1".into(), current: 0.12, pads: vec![], origin: "user".into() }],
                ground: false,
            },
            RailSetup {
                net: "VCORE".into(),
                voltage: 0.9,
                sources: vec![SourceSpec { refdes: "J1".into(), pads: vec!["2".into()] }],
                loads: vec![LoadSpec { refdes: "U1".into(), current: 1.5, pads: vec![], origin: "field".into() }],
                ground: false,
            },
            RailSetup {
                net: "/A B/VDD 1".into(),
                voltage: 1.8,
                sources: vec![SourceSpec { refdes: "J1".into(), pads: vec![] }],
                loads: vec![LoadSpec { refdes: "U1".into(), current: 0.002, pads: vec![], origin: "user".into() }],
                ground: false,
            },
        ];
        let (fields, lost) = setups_to_fields(&setups);
        assert_eq!(lost.len(), 1, "{lost:?}");
        let get = |r: &str, n: &str| fields.iter().find(|x| x.0 == r && x.1 == n).map(|x| x.2.clone());
        assert_eq!(get("J1", "PI_SOURCE").as_deref(), Some("+3V3, VCORE=0.9V, \"/A B/VDD 1\"=1.8V"));
        assert_eq!(get("U1", "PI_LOAD").as_deref(), Some("+3V3=120mA, VCORE=1.5A, \"/A B/VDD 1\"=2mA"));
        let mut b = load_pcb(pcb).unwrap();
        for (r, n, v) in &fields {
            b.footprints.iter_mut().find(|f| &f.reference == r).unwrap().props.push((n.clone(), v.clone()));
        }
        let (rails, w) = rails_from_fields(&b);
        assert!(w.is_empty(), "{w:?}");
        for s in &setups {
            let r = rails.iter().find(|r| r.net == s.net).unwrap();
            assert!(
                (r.voltage - s.voltage).abs() < 1e-9
                    && r.sources[0].refdes == "J1"
                    && (r.loads[0].current - s.loads[0].current).abs() < 1e-12,
                "{r:?}"
            );
        }
    }
}

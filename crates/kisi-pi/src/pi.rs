//! DC power integrity: IR drop of supply nets.
//!
//! Each copper layer of a net is rasterised into square cells; neighbouring
//! cells are connected by the sheet conductance of the layer, vias and
//! through-hole pads connect the layers. Source pads are held at the rail
//! voltage, load pads draw their current. The resulting SPD system is solved
//! with a sparse Cholesky factorisation (kisi-sparse / faer).
//!
//! Configuration comes from footprint fields in the board (PI_SOURCE,
//! PI_LOAD), the board configuration, or the viewer.

use crate::board::{pad_polygon, Board, Pad};
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
}

impl Default for PiParams {
    fn default() -> Self {
        PiParams { cell: None, plating_um: 25.0, rho: 1.72e-8, max_cells: 250_000 }
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
    let mut push = |r: &str, name: &str, v: String, out: &mut Vec<(String, String, String)>| match out
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
        xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let j = j0 + r;
        for pair in xs.chunks_exact(2) {
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
    // thin tracks (up to about one cell wide: a track as wide as the cell along a cell boundary
    // would otherwise lose cells to rounding): half the cell diagonal keeps the chain of cells
    // 4-connected for any direction
    let thin = width < g.cell * 1.05;
    let reach = if thin { g.cell * std::f64::consts::FRAC_1_SQRT_2 } else { r };
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
    let d = geom::sub(b, a);
    let l2 = geom::dot(d, d).max(1e-18);
    for j in j0..=j1 {
        for i in i0..=i1 {
            let c = g.center(i, j);
            let t = (geom::dot(geom::sub(c, a), d) / l2).clamp(0.0, 1.0);
            let q = geom::add(a, geom::scale(d, t));
            if geom::len(geom::sub(c, q)) <= reach {
                let k = j * g.nx + i;
                if w[k] < frac {
                    w[k] = frac;
                }
            }
        }
    }
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

    // weights per layer
    let mut w: Vec<Vec<f32>> = vec![Vec::new(); ncu];
    for &l in &layers {
        let mut wl = vec![0f32; per];
        for z in board.zones.iter().filter(|z| z.net == net && z.layer as usize == l) {
            fill_polygon(&g, &z.pts, &mut wl);
        }
        for t in board.tracks.iter().filter(|t| t.net == net && !t.pad && t.layer as usize == l) {
            fill_track(&g, t.a, t.b, t.width, &mut wl);
        }
        for p in board.pads.iter().filter(|p| p.net == net && p.layers.contains(&(l as u16))) {
            for k in pad_cells(&g, p) {
                wl[k] = 1.0;
            }
        }
        w[l] = wl;
    }

    // node numbering
    let mut node_of: Vec<Vec<u32>> = vec![Vec::new(); ncu];
    let mut node_pos: Vec<(usize, usize)> = Vec::new(); // (layer, cell)
    for &l in &layers {
        let mut m = vec![u32::MAX; per];
        for k in 0..per {
            if w[l][k] > 0.0 {
                m[k] = node_pos.len() as u32;
                node_pos.push((l, k));
            }
        }
        node_of[l] = m;
    }
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
    for &l in &layers {
        let gs = su.copper[l].thickness * 1e-3 / params.rho; // sheet conductance (S per square)
        let (wl, m) = (&w[l], &node_of[l]);
        for j in 0..g.ny {
            for i in 0..g.nx {
                let k = j * g.nx + i;
                if m[k] == u32::MAX {
                    continue;
                }
                for (ni, nj) in [(i + 1, j), (i, j + 1)] {
                    if ni >= g.nx || nj >= g.ny {
                        continue;
                    }
                    let k2 = nj * g.nx + ni;
                    if m[k2] == u32::MAX {
                        continue;
                    }
                    let (a, b) = (wl[k] as f64, wl[k2] as f64);
                    let wf = 2.0 * a * b / (a + b);
                    edges.push((m[k], m[k2], gs * wf / dv(m[k], m[k2])));
                }
            }
        }
    }
    // vertical: vias and through-hole pads of the net
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
    let mut verticals: Vec<(P, f64, Vec<usize>)> = Vec::new();
    for v in board.vias.iter().filter(|v| v.net == net) {
        verticals.push((v.at, v.drill, (v.from as usize..=v.to as usize).collect()));
    }
    for p in board.pads.iter().filter(|p| p.net == net && p.drill > 0.0) {
        verticals.push((p.at, p.drill, p.layers.iter().map(|&l| l as usize).collect()));
    }
    let find_node = |l: usize, at: P, r: f64| -> Option<u32> {
        let m = &node_of[l];
        if m.is_empty() {
            return None;
        }
        let (ci, cj) = g.cell_of(at)?;
        if m[cj * g.nx + ci] != u32::MAX {
            return Some(m[cj * g.nx + ci]);
        }
        let rc = (r / g.cell).ceil() as isize;
        let mut best: Option<(f64, u32)> = None;
        for dj in -rc..=rc {
            for di in -rc..=rc {
                let (ii, jj) = (ci as isize + di, cj as isize + dj);
                if ii < 0 || jj < 0 || ii as usize >= g.nx || jj as usize >= g.ny {
                    continue;
                }
                let k = jj as usize * g.nx + ii as usize;
                if m[k] != u32::MAX {
                    let d = geom::len(geom::sub(g.center(ii as usize, jj as usize), at));
                    if d <= r && best.is_none_or(|b| d < b.0) {
                        best = Some((d, m[k]));
                    }
                }
            }
        }
        best.map(|b| b.1)
    };
    for (at, drill, ls) in &verticals {
        let mut prev: Option<(usize, u32)> = None;
        for &l in ls {
            if l >= ncu || !used[l] {
                continue;
            }
            let Some(nid) = find_node(l, *at, drill / 2.0 + g.cell) else { continue };
            if let Some((pl, pn)) = prev {
                if pn != nid {
                    let len = (layer_z[l] - layer_z[pl]).abs() * 1e-3;
                    let gv = barrel(*drill) / (params.rho * len.max(1e-6)) / dv(pn, nid);
                    via_edges.push((edges.len(), *at, *drill));
                    edges.push((pn, nid, gv));
                }
            }
            prev = Some((l, nid));
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

    // per-layer buffers, current density
    let mut grids = Vec::new();
    let mut vb: Vec<f32> = Vec::with_capacity(per * layers.len());
    let mut jb: Vec<f32> = Vec::with_capacity(per * layers.len());
    let mut max_j = 0.0f64;
    let mut max_j_at = None;
    let cm = g.cell * 1e-3;
    for &l in &layers {
        let off = vb.len();
        let m = &node_of[l];
        let val = |i: isize, j: isize| -> Option<f64> {
            if i < 0 || j < 0 || i as usize >= g.nx || j as usize >= g.ny {
                return None;
            }
            let n = m[j as usize * g.nx + i as usize];
            if n == u32::MAX {
                return None;
            }
            let x = v[n as usize];
            x.is_finite().then_some(x)
        };
        for j in 0..g.ny as isize {
            for i in 0..g.nx as isize {
                match val(i, j) {
                    None => {
                        vb.push(f32::NAN);
                        jb.push(f32::NAN);
                    }
                    Some(c) => {
                        vb.push(c as f32);
                        let d = |a: Option<f64>, b: Option<f64>| -> f64 {
                            match (a, b) {
                                (Some(a), Some(b)) => (b - a) / (2.0 * cm),
                                (Some(a), None) => (c - a) / cm,
                                (None, Some(b)) => (b - c) / cm,
                                _ => 0.0,
                            }
                        };
                        let gx = d(val(i - 1, j), val(i + 1, j));
                        let gy = d(val(i, j - 1), val(i, j + 1));
                        let dn = if div.is_empty() { 1.0 } else { div[m[j as usize * g.nx + i as usize] as usize] };
                        let jm = (gx * gx + gy * gy).sqrt() / (params.rho * dn) * 1e-6; // A/mm²
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
    })
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

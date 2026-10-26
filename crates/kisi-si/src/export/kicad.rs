//! A KiCad project that simulates a lane: the transmitter's PRBS sources (subcircuits), the profile's
//! linear ends as R, L and C symbols (KiCad infers their models from the reference and the value),
//! ground, a `.tran` directive and the lane itself –
//!
//! * detailed (the default): one symbol per element of the lane subcircuit, wired in its order –
//!   line sections, coupled runs spanning a pair's two rows, vias and via pairs, parts (with their
//!   parasitics; the value as a subcircuit parameter), pads and stubs. Each symbol calls its element's
//!   subcircuit through a small wrapper in the bench library (`Sim.Device = SUBCKT`);
//! * compact: the whole lane as one symbol.
//!
//! Written in the KiCad 7 schematic format (20230121) with kisi's own symbols in `lib_symbols`, so
//! KiCad 7 and the later versions open it without any library. Pins are visible, every wire ends on a
//! pin end or another wire's end, points where three or more meet carry a junction, and everything
//! lies on KiCad's 1.27 mm grid (`validation/channel_export/kicad_sheet.py` checks this and runs
//! kicad-cli's netlist, and its ERC where available, through ngspice).

use super::circuit::num;
use super::files::{pwl_points, BenchEnds};
use super::{sig4, LaneCircuit, TopInst};
use std::collections::{BTreeSet, HashMap};

/// Deterministic UUIDs (KiCad only needs them unique within the file).
struct Ids {
    seed: u64,
    n: u64,
}

impl Ids {
    fn new(name: &str) -> Ids {
        let mut h: u64 = 0xcbf29ce484222325;
        for b in name.bytes() {
            h = (h ^ b as u64).wrapping_mul(0x100000001b3);
        }
        Ids { seed: h, n: 0 }
    }
    fn next(&mut self) -> String {
        self.n += 1;
        let mut x = self.seed ^ self.n.wrapping_mul(0x9e3779b97f4a7c15);
        let mut word = || {
            x ^= x >> 33;
            x = x.wrapping_mul(0xff51afd7ed558ccd);
            x ^= x >> 33;
            x
        };
        let (a, b) = (word(), word());
        format!(
            "{:08x}-{:04x}-4{:03x}-8{:03x}-{:012x}",
            (a >> 32) as u32,
            (a >> 16) & 0xffff,
            a & 0xfff,
            (b >> 52) & 0xfff,
            b & 0xffff_ffff_ffff
        )
    }
}

fn q(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n"))
}

const FONT: &str = "(effects (font (size 1.27 1.27)))";
const FONT_HIDE: &str = "(effects (font (size 1.27 1.27)) hide)";

/// Row distance of a pair's two lines on the sheet (P above N); coupled symbols span it.
const D: f64 = 25.4;

fn prop(name: &str, val: &str, x: f64, y: f64, hide: bool) -> String {
    format!(
        "    (property {} {} (at {x:.2} {y:.2} 0)\n      {}\n    )\n",
        q(name),
        q(val),
        if hide { FONT_HIDE } else { FONT }
    )
}

/// A visible property, left-justified at (x, y).
fn prop_left(name: &str, val: &str, x: f64, y: f64) -> String {
    format!(
        "    (property {} {} (at {x:.2} {y:.2} 0)\n      (effects (font (size 1.27 1.27)) (justify left))\n    )\n",
        q(name),
        q(val)
    )
}

// --- kisi's symbols ---------------------------------------------------------------------------

/// A pin: connection point (x, y) in symbol coordinates (y up), direction towards the body.
struct Pin {
    x: f64,
    y: f64,
    ang: u32,
    len: f64,
    name: &'static str,
}

fn pn(x: f64, y: f64, ang: u32, len: f64, name: &'static str) -> Pin {
    Pin { x, y, ang, len, name }
}

struct Sym {
    pins: Vec<Pin>,
    body: String,
    /// pin names shown (the lane symbol)
    names: bool,
    power: Option<&'static str>,
    /// reference and value positions in the library (the sheet places its own)
    reference: &'static str,
}

const STROKE: &str = "(stroke (width 0) (type default))";

fn poly(pts: &[(f64, f64)], w: f64) -> String {
    let p: Vec<String> = pts.iter().map(|(x, y)| format!("(xy {x} {y})")).collect();
    format!("        (polyline (pts {}) (stroke (width {w}) (type default)) (fill (type none)))\n", p.join(" "))
}

fn rect(a: (f64, f64), b: (f64, f64), fill: &str, dash: bool) -> String {
    format!(
        "        (rectangle (start {} {}) (end {} {}) (stroke (width {}) (type {})) (fill (type {fill})))\n",
        a.0,
        a.1,
        b.0,
        b.1,
        if dash { 0.0 } else { 0.254 },
        if dash { "dash" } else { "default" }
    )
}

fn circle(c: (f64, f64), r: f64) -> String {
    format!("        (circle (center {} {}) (radius {r}) {STROKE} (fill (type none)))\n", c.0, c.1)
}

fn arc(s: (f64, f64), m: (f64, f64), e: (f64, f64)) -> String {
    format!(
        "        (arc (start {} {}) (mid {} {}) (end {} {}) {STROKE} (fill (type none)))\n",
        s.0, s.1, m.0, m.1, e.0, e.1
    )
}

fn label(t: &str, x: f64, y: f64) -> String {
    format!("        (text {} (at {x} {y} 0) (effects (font (size 1.016 1.016))))\n", q(t))
}

/// A horizontal transmission line body (a cylinder seen from the side) centred at (0, y).
fn tl_body(y: f64) -> String {
    let mut s = poly(&[(-5.08, y + 1.27), (5.08, y + 1.27)], 0.254);
    s.push_str(&poly(&[(-5.08, y - 1.27), (5.08, y - 1.27)], 0.254));
    s.push_str(&circle((-5.08, y), 1.27));
    s.push_str(&arc((5.08, y + 1.27), (6.35, y), (5.08, y - 1.27)));
    s
}

/// A via body (barrel between two pads) centred at (0, y).
fn via_body(y: f64) -> String {
    let mut s = rect((-1.27, y + 2.54), (1.27, y - 2.54), "background", false);
    s.push_str(&poly(&[(-2.54, y + 2.54), (2.54, y + 2.54)], 0.508));
    s.push_str(&poly(&[(-2.54, y - 2.54), (2.54, y - 2.54)], 0.508));
    s
}

/// A vertical open stub hanging from (0, y0) down to y1 (y1 < y0), open end at y1.
fn stub_body(y0: f64, y1: f64) -> String {
    let mut s = String::new();
    let (top, bot) = if y1 < y0 { (y0, y1) } else { (y1, y0) };
    s.push_str(&poly(&[(-1.27, top), (-1.27, bot)], 0.254));
    s.push_str(&poly(&[(1.27, top), (1.27, bot)], 0.254));
    if y1 < y0 {
        s.push_str(&poly(&[(-1.27, top), (1.27, top)], 0.254));
        s.push_str(&arc((-1.27, bot), (0.0, bot - 1.27), (1.27, bot)));
    } else {
        s.push_str(&poly(&[(-1.27, bot), (1.27, bot)], 0.254));
        s.push_str(&arc((-1.27, top), (0.0, top + 1.27), (1.27, top)));
    }
    s
}

fn sym(name: &str) -> Sym {
    let plain = |pins: Vec<Pin>, body: String, reference: &'static str| Sym {
        pins,
        body,
        names: false,
        power: None,
        reference,
    };
    match name {
        "SRC" => {
            let mut b = circle((0.0, 0.0), 2.54);
            b.push_str(&poly(
                &[(-1.27, -0.635), (-0.635, -0.635), (-0.635, 0.635), (0.635, 0.635), (0.635, -0.635), (1.27, -0.635)],
                0.0,
            ));
            plain(vec![pn(0.0, 5.08, 270, 2.54, "out"), pn(0.0, -5.08, 90, 2.54, "ref")], b, "X")
        }
        "R" => plain(
            vec![pn(-5.08, 0.0, 0, 2.54, "a"), pn(5.08, 0.0, 180, 2.54, "b")],
            rect((-2.54, 1.016), (2.54, -1.016), "none", false),
            "R",
        ),
        "R_V" => plain(
            vec![pn(0.0, 5.08, 270, 2.54, "a"), pn(0.0, -5.08, 90, 2.54, "b")],
            rect((-1.016, 2.54), (1.016, -2.54), "none", false),
            "R",
        ),
        "C" => {
            let mut b = poly(&[(-0.635, 2.032), (-0.635, -2.032)], 0.508);
            b.push_str(&poly(&[(0.635, 2.032), (0.635, -2.032)], 0.508));
            plain(vec![pn(-5.08, 0.0, 0, 4.445, "a"), pn(5.08, 0.0, 180, 4.445, "b")], b, "C")
        }
        "C_V" => {
            let mut b = poly(&[(-2.032, 0.635), (2.032, 0.635)], 0.508);
            b.push_str(&poly(&[(-2.032, -0.635), (2.032, -0.635)], 0.508));
            plain(vec![pn(0.0, 5.08, 270, 4.445, "a"), pn(0.0, -5.08, 90, 4.445, "b")], b, "C")
        }
        "L" => {
            let mut b = String::new();
            for k in 0..4 {
                let x = -2.54 + 1.27 * k as f64;
                b.push_str(&arc((x, 0.0), (x + 0.635, 0.635), (x + 1.27, 0.0)));
            }
            plain(vec![pn(-5.08, 0.0, 0, 2.54, "a"), pn(5.08, 0.0, 180, 2.54, "b")], b, "L")
        }
        "L_V" => {
            let mut b = String::new();
            for k in 0..4 {
                let y = 2.54 - 1.27 * k as f64;
                b.push_str(&arc((0.0, y), (0.635, y - 0.635), (0.0, y - 1.27)));
            }
            plain(vec![pn(0.0, 5.08, 270, 2.54, "a"), pn(0.0, -5.08, 90, 2.54, "b")], b, "L")
        }
        "TL" => plain(vec![pn(-7.62, 0.0, 0, 1.27, "a"), pn(7.62, 0.0, 180, 1.27, "b")], tl_body(0.0), "TL"),
        "VIA" => plain(vec![pn(-5.08, 0.0, 0, 3.81, "a"), pn(5.08, 0.0, 180, 3.81, "b")], via_body(0.0), "VIA"),
        "STUB" => plain(vec![pn(0.0, 0.0, 270, 1.27, "a")], stub_body(-1.27, -8.89), "ST"),
        "CPL" => {
            let mut b = tl_body(0.0);
            b.push_str(&tl_body(-D));
            b.push_str(&rect((-6.35, 3.81), (6.35, -D - 3.81), "none", true));
            b.push_str(&label("coupled", 0.0, -D / 2.0));
            plain(
                vec![
                    pn(-7.62, 0.0, 0, 1.27, "p1"),
                    pn(-7.62, -D, 0, 1.27, "n1"),
                    pn(7.62, 0.0, 180, 1.27, "p2"),
                    pn(7.62, -D, 180, 1.27, "n2"),
                ],
                b,
                "CPL",
            )
        }
        "VIAPAIR" => {
            let mut b = via_body(0.0);
            b.push_str(&via_body(-D));
            b.push_str(&rect((-3.81, 3.81), (3.81, -D - 3.81), "none", true));
            b.push_str(&label("pair", 0.0, -D / 2.0));
            plain(
                vec![
                    pn(-5.08, 0.0, 0, 3.81, "p1"),
                    pn(-5.08, -D, 0, 3.81, "n1"),
                    pn(5.08, 0.0, 180, 3.81, "p2"),
                    pn(5.08, -D, 180, 3.81, "n2"),
                ],
                b,
                "VP",
            )
        }
        "STUBPAIR" => {
            let mut b = stub_body(-1.27, -8.89);
            b.push_str(&stub_body(-D + 1.27, -D + 8.89));
            b.push_str(&rect((-2.54, 0.0), (2.54, -D), "none", true));
            plain(vec![pn(0.0, 0.0, 270, 1.27, "p1"), pn(0.0, -D, 90, 1.27, "n1")], b, "SP")
        }
        "LANE2" | "LANE4" => {
            let pair = name == "LANE4";
            let h = if pair { 7.62 } else { 5.08 };
            let pins = if pair {
                vec![
                    pn(-12.7, 2.54, 0, 2.54, "ap"),
                    pn(-12.7, -2.54, 0, 2.54, "an"),
                    pn(12.7, 2.54, 180, 2.54, "bp"),
                    pn(12.7, -2.54, 180, 2.54, "bn"),
                    pn(0.0, -h - 2.54, 90, 2.54, "ref"),
                ]
            } else {
                vec![pn(-12.7, 0.0, 0, 2.54, "a"), pn(12.7, 0.0, 180, 2.54, "b"), pn(0.0, -h - 2.54, 90, 2.54, "ref")]
            };
            Sym {
                pins,
                body: rect((-10.16, h), (10.16, -h), "background", false),
                names: true,
                power: None,
                reference: "X",
            }
        }
        "GND" => Sym {
            pins: vec![pn(0.0, 0.0, 270, 0.0, "GND")],
            body: poly(&[(0.0, 0.0), (0.0, -1.27), (1.27, -1.27), (0.0, -2.54), (-1.27, -1.27), (0.0, -1.27)], 0.0),
            names: false,
            power: Some("power_in"),
            reference: "#PWR",
        },
        _ => Sym {
            // PWR_FLAG
            pins: vec![pn(0.0, 0.0, 90, 0.0, "pwr")],
            body: poly(&[(0.0, 0.0), (0.0, 1.27), (-1.016, 1.905), (0.0, 2.54), (1.016, 1.905), (0.0, 1.27)], 0.0),
            names: false,
            power: Some("power_out"),
            reference: "#FLG",
        },
    }
}

/// Every symbol of kisi's library.
const ALL_SYMBOLS: [&str; 17] = [
    "C", "C_V", "CPL", "GND", "L", "L_V", "LANE2", "LANE4", "PWR_FLAG", "R", "R_V", "SRC", "STUB", "STUBPAIR", "TL",
    "VIA", "VIAPAIR",
];

/// One symbol definition; `prefix` "kisi:" in the sheet's `lib_symbols`, "" in the library file.
fn symbol_def(name: &str, prefix: &str) -> String {
    let y = sym(name);
    let flags = if y.power.is_some() { " (power)" } else { "" };
    let names = if y.names { "(pin_names (offset 1.016))" } else { "(pin_names (offset 0) hide)" };
    let mut s = format!(
        "    (symbol {}{flags} (pin_numbers hide) {names} (in_bom yes) (on_board yes)\n",
        q(&format!("{prefix}{name}"))
    );
    s.push_str(&prop("Reference", y.reference, 0.0, 3.81, y.power.is_some()));
    s.push_str(&prop("Value", name, 0.0, -3.81, false));
    s.push_str(&prop("Footprint", "", 0.0, 0.0, true));
    s.push_str(&prop("Datasheet", "", 0.0, 0.0, true));
    s.push_str(&format!("      (symbol \"{name}_0_1\"\n{}      )\n      (symbol \"{name}_1_1\"\n", y.body));
    for (i, p) in y.pins.iter().enumerate() {
        let kind = y.power.unwrap_or("passive");
        // power symbols: the pin hidden at the origin, as in KiCad's own power library
        let hide = if y.power.is_some() { " hide" } else { "" };
        s.push_str(&format!(
            "        (pin {kind} line (at {} {} {}) (length {}){hide}\n          (name {} (effects (font (size 1.27 1.27))))\n          (number \"{}\" (effects (font (size 1.27 1.27))))\n        )\n",
            p.x,
            p.y,
            p.ang,
            p.len,
            q(p.name),
            i + 1
        ));
    }
    s.push_str("      )\n    )\n");
    s
}

/// The `lib_symbols` section with the symbols the sheet uses.
fn lib_symbols(used: &BTreeSet<&'static str>) -> String {
    let mut s = String::from("  (lib_symbols\n");
    for &name in used {
        s.push_str(&symbol_def(name, "kisi:"));
    }
    s.push_str("  )\n");
    s
}

/// The project's symbol library `kisi.kicad_sym` (the symbols the sheet embeds, so KiCad finds the
/// library they come from) and its `sym-lib-table`.
pub fn symbol_library() -> (String, String) {
    let mut s = String::from("(kicad_symbol_lib (version 20220914) (generator kisi)\n");
    let set: BTreeSet<&str> = ALL_SYMBOLS.iter().copied().collect();
    for name in set {
        s.push_str(&symbol_def(name, ""));
    }
    s.push_str(")\n");
    let table = "(sym_lib_table\n  (lib (name \"kisi\")(type \"KiCad\")(uri \"${KIPRJMOD}/kisi.kicad_sym\")(options \"\")(descr \"symbols of the kisi export\"))\n)\n".to_string();
    (s, table)
}

// --- the sheet --------------------------------------------------------------------------------

/// Where a symbol's reference and value go.
#[derive(Clone, Copy)]
enum Lab {
    /// centred above the symbol: value `dy` above `at`, reference one line higher
    Above(f64),
    /// left-justified to the right: reference at (`dx`, `dy` − 1.27), value one line lower
    Right(f64, f64),
    /// the reference hidden (power symbols), the value at `dy` (negative: above)
    Power(f64),
}

fn g(v: f64) -> f64 {
    (v / 1.27).round() * 1.27
}

fn pt_eq(a: (f64, f64), b: (f64, f64)) -> bool {
    (a.0 - b.0).abs() < 1e-6 && (a.1 - b.1).abs() < 1e-6
}

struct Sheet {
    ids: Ids,
    root: String,
    project: String,
    items: Vec<String>,
    pwr: usize,
    used: BTreeSet<&'static str>,
    wires: Vec<((f64, f64), (f64, f64))>,
    pins: Vec<(f64, f64)>,
    /// the lowest and rightmost point used (for the paper size)
    extent: (f64, f64),
}

impl Sheet {
    fn new(seed: &str, project: &str) -> Sheet {
        let mut ids = Ids::new(seed);
        let root = ids.next();
        Sheet {
            ids,
            root,
            project: project.into(),
            items: vec![],
            pwr: 0,
            used: BTreeSet::new(),
            wires: vec![],
            pins: vec![],
            extent: (0.0, 0.0),
        }
    }
    fn grow(&mut self, p: (f64, f64)) {
        self.extent.0 = self.extent.0.max(p.0);
        self.extent.1 = self.extent.1.max(p.1);
    }
    fn wire(&mut self, a: (f64, f64), b: (f64, f64)) {
        let (a, b) = ((g(a.0), g(a.1)), (g(b.0), g(b.1)));
        if pt_eq(a, b) {
            return;
        }
        self.wires.push((a, b));
        self.grow(a);
        self.grow(b);
    }
    /// A wire along the points, zero-length and collinear pieces merged.
    fn route(&mut self, pts: &[(f64, f64)]) {
        let mut v: Vec<(f64, f64)> = Vec::new();
        for &p in pts {
            let p = (g(p.0), g(p.1));
            if v.last().is_some_and(|q| pt_eq(*q, p)) {
                continue;
            }
            if v.len() >= 2 {
                let (a, b) = (v[v.len() - 2], v[v.len() - 1]);
                if ((a.0 - b.0).abs() < 1e-6 && (b.0 - p.0).abs() < 1e-6)
                    || ((a.1 - b.1).abs() < 1e-6 && (b.1 - p.1).abs() < 1e-6)
                {
                    v.pop();
                }
            }
            v.push(p);
        }
        for w in v.windows(2) {
            self.wire(w[0], w[1]);
        }
    }
    fn text(&mut self, t: &str, at: (f64, f64)) {
        let u = self.ids.next();
        self.items.push(format!("  (text {} (at {:.2} {:.2} 0)\n    (effects (font (size 1.27 1.27)) (justify left bottom))\n    (uuid {u})\n  )\n", q(t), at.0, at.1));
        let w = t.lines().map(|l| l.chars().count()).max().unwrap_or(0) as f64;
        self.grow((at.0 + w * 1.1, at.1));
    }
    /// A symbol of kisi's library at `at` with extra (hidden) fields.
    fn place(
        &mut self,
        name: &'static str,
        reference: &str,
        value: &str,
        at: (f64, f64),
        lab: Lab,
        fields: &[(&str, String)],
    ) {
        let at = (g(at.0), g(at.1));
        self.used.insert(name);
        let y = sym(name);
        for p in &y.pins {
            self.pins.push((at.0 + p.x, at.1 - p.y));
            self.grow((at.0 + p.x, at.1 - p.y));
        }
        let u = self.ids.next();
        let mut s = format!("  (symbol (lib_id {}) (at {:.2} {:.2} 0) (unit 1)\n    (in_bom yes) (on_board yes) (dnp no)\n    (uuid {u})\n", q(&format!("kisi:{name}")), at.0, at.1);
        match lab {
            Lab::Above(dy) => {
                s.push_str(&prop("Reference", reference, at.0, at.1 - dy - 2.54, false));
                s.push_str(&prop("Value", value, at.0, at.1 - dy, false));
            }
            Lab::Right(dx, dy) => {
                s.push_str(&prop_left("Reference", reference, at.0 + dx, at.1 + dy - 1.27));
                s.push_str(&prop_left("Value", value, at.0 + dx, at.1 + dy + 1.27));
                let w = reference.chars().count().max(value.chars().count()) as f64;
                self.grow((at.0 + dx + w * 1.1, at.1 + dy + 1.27));
            }
            Lab::Power(dy) => {
                s.push_str(&prop("Reference", reference, at.0, at.1, true));
                s.push_str(&prop("Value", value, at.0, at.1 + dy, false));
            }
        }
        s.push_str(&prop("Footprint", "", at.0, at.1, true));
        s.push_str(&prop("Datasheet", "", at.0, at.1, true));
        for (k, v) in fields {
            s.push_str(&prop(k, v, at.0, at.1, true));
        }
        for p in 1..=y.pins.len() {
            let u = self.ids.next();
            s.push_str(&format!("    (pin \"{p}\" (uuid {u}))\n"));
        }
        s.push_str(&format!("    (instances\n      (project {}\n        (path \"/{}\"\n          (reference {}) (unit 1)\n        )\n      )\n    )\n  )\n", q(&self.project), self.root, q(reference)));
        self.items.push(s);
    }
    fn gnd(&mut self, at: (f64, f64)) {
        self.pwr += 1;
        let r = format!("#PWR{:02}", self.pwr);
        self.place("GND", &r, "GND", at, Lab::Power(3.81), &[]);
    }
    /// Wires and junctions as KiCad items: a junction wherever three or more wire ends and pin ends
    /// meet (a wire passing through counts twice).
    fn finish_wires(&mut self) {
        let mut pts: Vec<(f64, f64)> = Vec::new();
        for (a, b) in &self.wires {
            for p in [*a, *b] {
                if !pts.iter().any(|q| pt_eq(*q, p)) {
                    pts.push(p);
                }
            }
        }
        let through = |p: (f64, f64), a: (f64, f64), b: (f64, f64)| -> bool {
            let on_x = (a.0 - b.0).abs() < 1e-6
                && (p.0 - a.0).abs() < 1e-6
                && p.1 > a.1.min(b.1) + 1e-6
                && p.1 < a.1.max(b.1) - 1e-6;
            let on_y = (a.1 - b.1).abs() < 1e-6
                && (p.1 - a.1).abs() < 1e-6
                && p.0 > a.0.min(b.0) + 1e-6
                && p.0 < a.0.max(b.0) - 1e-6;
            on_x || on_y
        };
        let mut junctions = Vec::new();
        for p in pts {
            let ends = self.wires.iter().map(|(a, b)| pt_eq(*a, p) as usize + pt_eq(*b, p) as usize).sum::<usize>();
            let pins = self.pins.iter().filter(|q| pt_eq(**q, p)).count();
            let thr = self.wires.iter().filter(|(a, b)| through(p, *a, *b)).count();
            if ends + pins + 2 * thr >= 3 {
                junctions.push(p);
            }
        }
        let wires = std::mem::take(&mut self.wires);
        for (a, b) in &wires {
            let u = self.ids.next();
            self.items.push(format!("  (wire (pts (xy {:.2} {:.2}) (xy {:.2} {:.2}))\n    (stroke (width 0) (type default))\n    (uuid {u})\n  )\n", a.0, a.1, b.0, b.1));
        }
        for p in junctions {
            let u = self.ids.next();
            self.items.push(format!(
                "  (junction (at {:.2} {:.2}) (diameter 0) (color 0 0 0 0)\n    (uuid {u})\n  )\n",
                p.0, p.1
            ));
        }
    }
    /// The schematic file.
    fn file(mut self, title: &str) -> String {
        self.finish_wires();
        // the smallest landscape ISO sheet that holds the drawing and the title block
        let need = (self.extent.0 + 15.0, self.extent.1 + 45.0);
        let paper = [
            ("A4", 297.0, 210.0),
            ("A3", 420.0, 297.0),
            ("A2", 594.0, 420.0),
            ("A1", 841.0, 594.0),
            ("A0", 1189.0, 841.0),
        ]
        .iter()
        .find(|p| p.1 >= need.0 && p.2 >= need.1)
        .map(|p| format!("\"{}\"", p.0))
        .unwrap_or_else(|| format!("\"User\" {:.0} {:.0}", need.0, need.1));
        let mut s = format!(
            "(kicad_sch (version 20230121) (generator eeschema)\n\n  (uuid {})\n\n  (paper {paper})\n\n  (title_block\n    (title {})\n    (comment 1 {})\n  )\n\n",
            self.root,
            q(title),
            q("generated by kisi – linear test bench")
        );
        s.push_str(&lib_symbols(&self.used));
        s.push('\n');
        for it in &self.items {
            s.push_str(it);
        }
        s.push_str("\n  (sheet_instances\n    (path \"/\" (page \"1\"))\n  )\n)\n");
        s
    }
}

/// A value as KiCad expects it in the Value field of an R, L or C (SI prefix, '.' as decimal).
fn si(v: f64) -> String {
    let (m, p) = if v >= 1e9 {
        (v * 1e-9, "G")
    } else if v >= 1e6 {
        (v * 1e-6, "M")
    } else if v >= 1e3 {
        (v * 1e-3, "k")
    } else if v >= 1.0 {
        (v, "")
    } else if v >= 1e-3 {
        (v * 1e3, "m")
    } else if v >= 1e-6 {
        (v * 1e6, "u")
    } else if v >= 1e-9 {
        (v * 1e9, "n")
    } else if v >= 1e-12 {
        (v * 1e12, "p")
    } else {
        (v * 1e15, "f")
    };
    let t = format!("{m:.4}");
    let t = t.trim_end_matches('0').trim_end_matches('.');
    format!("{t}{p}")
}

// --- the bench library ------------------------------------------------------------------------

/// How a lane element appears on the sheet and in the bench library.
struct ElemSym {
    /// kisi symbol
    sym: &'static str,
    /// wrapper subcircuit `<sub>_k`: its ports (= the symbol's pins in order) and body
    ports: Vec<&'static str>,
    body: String,
    /// subcircuit parameter of a part (name, value as written)
    param: Option<(char, String)>,
    /// a pair element on both rows; a shunt (hangs below its row)
    both: bool,
    shunt: bool,
}

/// The symbol and wrapper of a lane element. The wrappers tie the element's reference to node 0
/// (ground), so lines, vias and coupled runs need no ground pin; pads and shunt parts keep theirs.
fn elem_sym(t: &TopInst) -> ElemSym {
    let tag = &t.tag;
    let sub = &t.sub;
    let n = t.ports.len();
    // a part as a parameterised copy of its subcircuit: the value can be changed in KiCad
    // (Sim.Params), the parasitics stay
    if let (Some((kind, series, r, l, c)), "part" | "pad") = (tag.part, tag.kind) {
        let main = if let Some(c) = c.filter(|_| kind == 'C' || tag.kind == "pad") {
            Some(('c', c))
        } else if c.is_none() && kind == 'L' && l > 0.0 {
            Some(('l', l))
        } else if c.is_none() && kind != 'L' && r > 0.0 {
            Some(('r', r))
        } else {
            None
        };
        if let Some((pc, pv)) = main {
            let b_node = if series { "b" } else { "g" };
            let mut els: Vec<(char, String)> = Vec::new();
            if r > 0.0 {
                els.push(('R', if pc == 'r' { "{r}".into() } else { num(r) }));
            }
            if l > 0.0 {
                els.push(('L', if pc == 'l' { "{l}".into() } else { num(l) }));
            }
            if let Some(c) = c {
                els.push(('C', if pc == 'c' { "{c}".into() } else { num(c) }));
            }
            let mut body = String::new();
            let mut cur = "a".to_string();
            let mut cnt: HashMap<char, usize> = HashMap::new();
            for (i, (k, v)) in els.iter().enumerate() {
                let nxt = if i + 1 == els.len() { b_node.to_string() } else { format!("n{}", i + 1) };
                let c = cnt.entry(*k).or_insert(0);
                *c += 1;
                body.push_str(&format!("{k}{c} {cur} {nxt} {v}\n"));
                cur = nxt;
            }
            if series && c.is_some() {
                let c = cnt.entry('R').or_insert(0);
                *c += 1;
                body.push_str(&format!("R{c} a b 1.000000e9\n"));
            }
            let sym = match (pc, series) {
                ('c', true) => "C",
                ('c', false) => "C_V",
                ('l', true) => "L",
                ('l', false) => "L_V",
                (_, true) => "R",
                _ => "R_V",
            };
            // SPICE notation for the parameter (M is milli there)
            return ElemSym {
                sym,
                ports: vec!["a", b_node],
                body,
                param: Some((pc, si(pv).replace('M', "Meg"))),
                both: false,
                shunt: !series,
            };
        }
    }
    let x = |ports: &[&'static str], extra: &str| format!("X1 {}{extra} {sub}\n", ports.join(" "));
    match n {
        5 => {
            let p = vec!["p1", "n1", "p2", "n2"];
            let body = x(&p, " 0");
            ElemSym {
                sym: if tag.kind == "viapair" { "VIAPAIR" } else { "CPL" },
                ports: p,
                body,
                param: None,
                both: true,
                shunt: false,
            }
        }
        3 if t.ports[0] == "p1" => {
            let p = vec!["p1", "n1"];
            let body = x(&p, " 0");
            ElemSym { sym: "STUBPAIR", ports: p, body, param: None, both: true, shunt: true }
        }
        3 => {
            let p = vec!["a", "b"];
            let body = x(&p, " 0");
            let sym = match (tag.kind, tag.part.map(|p| p.0)) {
                ("via", _) => "VIA",
                ("part", Some('C')) => "C",
                ("part", Some('L')) => "L",
                ("part", _) => "R",
                _ => "TL",
            };
            ElemSym { sym, ports: p, body, param: None, both: false, shunt: false }
        }
        _ => {
            if tag.kind == "pad" || tag.kind == "part" {
                let p = vec!["a", "g"];
                let body = x(&p, "");
                let sym = match tag.part.map(|p| p.0) {
                    Some('L') => "L_V",
                    Some('R') => "R_V",
                    _ => "C_V",
                };
                ElemSym { sym, ports: p, body, param: None, both: false, shunt: true }
            } else {
                let p = vec!["a"];
                let body = x(&p, " 0");
                ElemSym { sym: "STUB", ports: p, body, param: None, both: false, shunt: true }
            }
        }
    }
}

/// The library of the KiCad project: the lane library included, small wrapper subcircuits (KiCad
/// reads the file it names for the models; a short file loads at once) – `<prefix>_kicad` for the
/// compact sheet, `<element>_k` for each element of the detailed one – and one PRBS7 source
/// subcircuit per line (`out ref`).
pub fn bench_lib(
    lc: &LaneCircuit,
    ends: &BenchEnds,
    prefix: &str,
    lib_file: &str,
    detailed: bool,
) -> (String, f64, f64) {
    let ui = 1.0 / ends.bitrate;
    let bits: Vec<bool> = super::files::prbs7().into_iter().chain(super::files::prbs7()).collect();
    let a = super::files::emf_amp(ends, lc.pair);
    let pts = pwl_points(&bits, ui, ends.rise, a);
    let mut s = format!(
        "* kisi {}: KiCad test bench of lane {}\n.include \"{lib_file}\"\n\n",
        env!("CARGO_PKG_VERSION"),
        lc.lane
    );
    if detailed {
        s.push_str(&format!("* the lane's elements (subcircuits in {lib_file}), reference node = ground; parts with their value as parameter\n"));
        for t in &lc.top {
            let e = elem_sym(t);
            let params = e.param.as_ref().map(|(k, v)| format!(" params: {k}={v}")).unwrap_or_default();
            s.push_str(&format!(".subckt {}_k {}{params}\n{}.ends {}_k\n", t.sub, e.ports.join(" "), e.body, t.sub));
        }
        s.push('\n');
    } else {
        let ports = lc.ports.join(" ");
        s.push_str(&format!("* the lane (subcircuit {} in {lib_file})\n.subckt {prefix}_kicad {ports} ref\nX1 {ports} ref {}\n.ends {prefix}_kicad\n\n", lc.subckt, lc.subckt));
    }
    s.push_str(&format!(
        "* sources: PRBS7 at {} Gbit/s, edge {} ps (10-90 %), emf amplitude {} V per line\n",
        sig4(ends.bitrate / 1e9),
        sig4(ends.rise * 1e12),
        sig4(a)
    ));
    let lines: &[(&str, f64)] = if lc.pair { &[("P", 1.0), ("N", -1.0)] } else { &[("P", 1.0)] };
    for (l, sign) in lines {
        s.push_str(&format!(".subckt {prefix}_TX{l} out ref\n"));
        let mut line = String::from("V1 out ref PWL(");
        let mut ll = line.len();
        for (i, (t, v)) in pts.iter().enumerate() {
            let item = format!("{}{} {}", if i == 0 { "" } else { " " }, num(*t), num(v * sign));
            if ll + item.len() > 100 {
                line.push_str("\n+");
                ll = 1;
            }
            line.push_str(&item);
            ll += item.len();
        }
        line.push(')');
        s.push_str(&line);
        s.push_str(&format!("\n.ends {prefix}_TX{l}\n"));
    }
    let tstop = (bits.len() + 2) as f64 * ui + 2.0 * lc.delay_ps * 1e-12;
    let tstep = (ends.rise / 10.0).min(ui / 20.0);
    (s, tstep, tstop)
}

// --- the ends -----------------------------------------------------------------------------------

/// The transmitter of line `l` on row `y`, source at `xs`: PRBS7 source, R_tx, C_tx at the die,
/// L_tx. Returns the x of its output on the row.
fn tx_end(sh: &mut Sheet, ends: &BenchEnds, l: &str, y: f64, xs: f64, prefix: &str, bench_file: &str) -> f64 {
    sh.place(
        "SRC",
        &format!("XTX{l}1"),
        &format!("PRBS7 {l}"),
        (xs, y + 5.08),
        Lab::Right(3.81, 0.0),
        &[
            ("Sim.Device", "SUBCKT".into()),
            ("Sim.Library", bench_file.into()),
            ("Sim.Name", format!("{prefix}_TX{l}")),
            ("Sim.Pins", "1=out 2=ref".into()),
        ],
    );
    sh.gnd((xs, y + 10.16));
    let mut x = xs;
    if ends.tx.r > 0.0 {
        let c = x + 15.24;
        sh.wire((x, y), (c - 5.08, y));
        sh.place("R", &format!("RTX{l}1"), &si(ends.tx.r), (c, y), Lab::Above(2.54), &[]);
        x = c + 5.08;
    }
    if ends.tx.c_pf > 0.0 {
        let c = x + 7.62;
        sh.wire((x, y), (c, y));
        sh.place("C_V", &format!("CTX{l}1"), &si(ends.tx.c_pf * 1e-12), (c, y + 5.08), Lab::Right(3.81, 0.0), &[]);
        sh.gnd((c, y + 10.16));
        x = c;
    }
    if ends.tx.l_nh > 0.0 {
        let c = x + 15.24;
        sh.wire((x, y), (c - 5.08, y));
        sh.place("L", &format!("LTX{l}1"), &si(ends.tx.l_nh * 1e-9), (c, y), Lab::Above(2.54), &[]);
        x = c + 5.08;
    }
    x
}

/// The receiver of line `l` on row `y` from `x`: L_rx, then C_rx and R_rx from the die to ground.
fn rx_end(sh: &mut Sheet, ends: &BenchEnds, l: &str, y: f64, x: f64) {
    let mut x = x;
    if ends.rx.l_nh > 0.0 {
        let c = x + 7.62;
        sh.wire((x, y), (c - 5.08, y));
        sh.place("L", &format!("LRX{l}1"), &si(ends.rx.l_nh * 1e-9), (c, y), Lab::Above(2.54), &[]);
        x = c + 5.08;
    }
    let mut shunts: Vec<(&'static str, String, f64)> = Vec::new();
    if ends.rx.c_pf > 0.0 {
        shunts.push(("C_V", format!("CRX{l}1"), ends.rx.c_pf * 1e-12));
    }
    if ends.rx.r > 0.0 {
        shunts.push(("R_V", format!("RRX{l}1"), ends.rx.r));
    }
    for (s, r, v) in &shunts {
        let c = x + 12.7;
        sh.wire((x, y), (c, y));
        sh.place(s, r, &si(*v), (c, y + 5.08), Lab::Right(3.81, 0.0), &[]);
        sh.gnd((c, y + 10.16));
        x = c;
    }
}

/// The notes on the sheet: what it is, the analysis, what the ends stand for.
fn notes(
    sh: &mut Sheet,
    lc: &LaneCircuit,
    ends: &BenchEnds,
    lib_file: &str,
    bench_file: &str,
    tstep: f64,
    tstop: f64,
    detailed: bool,
    y_end: f64,
) {
    let how = if detailed {
        format!("Each element symbol calls its subcircuit in {lib_file} through a wrapper `<name>_k` in {bench_file} (reference node = ground).\nA part's Value is its subcircuit parameter c, r or l (Sim.Params refers to the Value field; SPICE notation): edit the value, the parasitics stay.")
    } else {
        format!(
            "The lane subcircuit {} is in {lib_file} (included by {bench_file}, with the PRBS7 sources).",
            lc.subckt
        )
    };
    sh.text(
        &format!(
            "kisi {}: lane {} – linear test bench\n{how}\nProbe the receiver die; for a pair the difference of P and N. Inspect → Simulator → Run.\nAdd a termination or swap the ends here; IBIS ends: replace the source/receiver by KiCad IBIS symbols.",
            env!("CARGO_PKG_VERSION"),
            lc.lane,
        ),
        (38.1, 38.1),
    );
    // an open receiver: Gear integration (`files::ngspice_options`)
    let opts = super::files::ngspice_options(ends).map(|o| format!("{o}\n")).unwrap_or_default();
    sh.text(&format!("{opts}.tran {} {}", num(tstep), num(tstop)), (38.1, 45.72));
    let (tx, rx) = super::files::ends_text(lc, ends);
    let mut note = tx.join("\n");
    note.push_str("\n\n");
    note.push_str(&rx.join("\n"));
    for n in &ends.notes {
        note.push_str("\nNote: ");
        note.push_str(n);
    }
    let nlines = note.lines().count() as f64;
    sh.text(&note, (38.1, g(y_end + 2.54 * (nlines * 0.85).ceil())));
}

fn project_file(name: &str, root: &str) -> String {
    format!(
        "{{\n  \"meta\": {{\n    \"filename\": \"{name}.kicad_pro\",\n    \"version\": 1\n  }},\n  \"schematic\": {{\n    \"legacy_lib_dir\": \"\",\n    \"legacy_lib_list\": []\n  }},\n  \"sheets\": [\n    [\n      \"{root}\",\n      \"\"\n    ]\n  ],\n  \"text_variables\": {{}}\n}}\n"
    )
}

/// The power flag on ground (KiCad's ERC: the ground net is driven).
fn power_flag(sh: &mut Sheet, x: f64, y: f64) {
    sh.place("PWR_FLAG", "#FLG01", "PWR_FLAG", (x, y), Lab::Power(-3.81), &[]);
    sh.wire((x, y), (x, y + 5.08));
    sh.gnd((x, y + 5.08));
}

/// The KiCad project: `<name>.kicad_pro` and `<name>.kicad_sch` (detailed when the lane's elements
/// are known, else compact).
pub fn project(
    lc: &LaneCircuit,
    ends: &BenchEnds,
    name: &str,
    lib_file: &str,
    bench_file: &str,
    tstep: f64,
    tstop: f64,
    prefix: &str,
    detailed: bool,
) -> (String, String) {
    if detailed {
        if let Some(r) = detailed_project(lc, ends, name, lib_file, bench_file, tstep, tstop, prefix) {
            return r;
        }
    }
    compact_project(lc, ends, name, lib_file, bench_file, tstep, tstop, prefix)
}

/// Whether the detailed sheet can be drawn for this lane (its elements chain from port to port).
pub fn can_detail(lc: &LaneCircuit) -> bool {
    !lc.top.is_empty() && chain_order(lc).is_some()
}

/// The lane's elements with the line each one is on (None: both lines of a pair), checked to
/// follow each other from the ports.
fn chain_order(lc: &LaneCircuit) -> Option<Vec<Option<usize>>> {
    let mut cur: Vec<String> = if lc.pair { vec!["ap".into(), "an".into()] } else { vec!["a".into()] };
    let mut out = Vec::new();
    for t in &lc.top {
        let n = &t.nodes;
        match t.ports.len() {
            5 if lc.pair => {
                if n[0] != cur[0] || n[1] != cur[1] {
                    return None;
                }
                cur = vec![n[2].clone(), n[3].clone()];
                out.push(None);
            }
            3 if lc.pair && t.ports[0] == "p1" => {
                if n[0] != cur[0] || n[1] != cur[1] {
                    return None;
                }
                out.push(None);
            }
            3 => {
                let l = cur.iter().position(|c| *c == n[0])?;
                cur[l] = n[1].clone();
                out.push(Some(l));
            }
            2 => {
                let l = cur.iter().position(|c| *c == n[0])?;
                out.push(Some(l));
            }
            _ => return None,
        }
    }
    let end: Vec<String> = if lc.pair { vec!["bp".into(), "bn".into()] } else { vec!["b".into()] };
    (cur == end).then_some(out)
}

/// Reference designators of the elements: the board's for parts, else by kind.
fn references(lc: &LaneCircuit) -> Vec<String> {
    let mut count: HashMap<&str, usize> = HashMap::new();
    let mut taken: BTreeSet<String> = BTreeSet::new();
    let mut out = Vec::new();
    for t in &lc.top {
        let stem = match t.tag.kind {
            "line" => "TL",
            "coupled" => "CPL",
            "via" => "VIA",
            "viapair" => "VP",
            "stub" => "ST",
            "stubpair" => "SP",
            "pad" => "CP",
            "part" => "",
            _ => "X",
        };
        let r = if stem.is_empty() && !t.tag.refdes.is_empty() && !taken.contains(&t.tag.refdes) {
            t.tag.refdes.clone()
        } else {
            let stem = if stem.is_empty() { "XP" } else { stem };
            loop {
                let c = count.entry(stem).or_insert(0);
                *c += 1;
                let r = format!("{stem}{c}");
                if !taken.contains(&r) {
                    break r;
                }
            }
        };
        taken.insert(r.clone());
        out.push(r);
    }
    out
}

fn value_of(t: &TopInst) -> String {
    match t.tag.kind {
        "pad" => format!("{} {}", t.tag.refdes, t.tag.value),
        "" => t.sub.clone(),
        _ => t.tag.value.clone(),
    }
}

/// Text width estimate (mm) at the default 1.27 mm font.
fn tw(s: &str) -> f64 {
    s.chars().count() as f64 * 1.1
}

fn up(v: f64) -> f64 {
    (v / 2.54 - 1e-6).ceil() * 2.54
}

fn detailed_project(
    lc: &LaneCircuit,
    ends: &BenchEnds,
    name: &str,
    lib_file: &str,
    bench_file: &str,
    tstep: f64,
    tstop: f64,
    prefix: &str,
) -> Option<(String, String)> {
    let order = chain_order(lc)?;
    if lc.top.is_empty() {
        return None;
    }
    let refs = references(lc);
    let mut sh = Sheet::new(&format!("{name}{}", lc.subckt), name);
    let lines: Vec<&str> = if lc.pair { vec!["P", "N"] } else { vec!["P"] };
    let nl = lines.len();
    // bands: rows P (and N), the wrap corridors below, then the next band
    let pitch = if lc.pair { D + 43.18 } else { 40.64 };
    let y0 = 63.5;
    let x_left = 30.48;
    // width: about 16 elements per band, at least what the longest line needs in four bands
    let x_max = 380.0f64;
    let mut band = 0usize;
    let rows = |band: usize| -> Vec<f64> { (0..nl).map(|l| y0 + band as f64 * pitch + l as f64 * D).collect() };
    let mut y = rows(0);
    let mut pos = vec![0.0f64; nl];
    let mut lab = vec![0.0f64; nl];
    for (l, nm) in lines.iter().enumerate() {
        pos[l] = tx_end(&mut sh, ends, nm, y[l], 50.8, prefix, bench_file);
        lab[l] = pos[l];
    }
    power_flag(&mut sh, 25.4, y0 + 5.08);
    let x_start = pos.iter().cloned().fold(0.0, f64::max) + 2.54;
    for l in 0..nl {
        sh.wire((pos[l], y[l]), (x_start, y[l]));
        pos[l] = x_start;
    }
    // wrap both lines into the next band
    let wrap = |sh: &mut Sheet, band: &mut usize, y: &mut Vec<f64>, pos: &mut Vec<f64>, lab: &mut Vec<f64>| {
        let xr = up(pos.iter().cloned().fold(0.0, f64::max) + 2.54);
        let last = y[nl - 1];
        *band += 1;
        let ny = rows(*band);
        for l in 0..nl {
            // P outermost on the right and innermost on the left, so the two never cross
            let k = (nl - l) as f64;
            let xo = xr + 2.54 * (k - 1.0);
            let yc = last + 17.78 + if nl == 2 && l == 0 { 2.54 } else { 0.0 };
            let xl = x_left - 2.54 * (l as f64 + 1.0);
            sh.route(&[(pos[l], y[l]), (xo, y[l]), (xo, yc), (xl, yc), (xl, ny[l]), (x_left, ny[l])]);
            pos[l] = x_left;
            lab[l] = x_left;
        }
        *y = ny;
    };
    for (i, t) in lc.top.iter().enumerate() {
        let e = elem_sym(t);
        let refd = &refs[i];
        // a part's value is its parameter (SPICE notation); Sim.Params takes it from the field
        let part_param = t.tag.kind == "part" && e.param.is_some();
        let val = if part_param { e.param.as_ref().map(|p| p.1.clone()).unwrap_or_default() } else { value_of(t) };
        let lw = tw(refd).max(tw(&val));
        let half = sym(e.sym).pins.iter().map(|p| p.x.abs()).fold(0.0, f64::max);
        let mut fields: Vec<(&str, String)> = vec![
            ("Sim.Device", "SUBCKT".into()),
            ("Sim.Library", bench_file.into()),
            ("Sim.Name", format!("{}_k", t.sub)),
            (
                "Sim.Pins",
                e.ports.iter().enumerate().map(|(k, p)| format!("{}={p}", k + 1)).collect::<Vec<_>>().join(" "),
            ),
        ];
        if let Some((k, v)) = &e.param {
            fields.push(("Sim.Params", if part_param { format!("{k}=${{VALUE}}") } else { format!("{k}={v}") }));
        }
        let ls: Vec<usize> = match order[i] {
            Some(l) => vec![l],
            None => (0..nl).collect(),
        };
        // the element's x: after the wire ends on its lines and clear of the labels before it
        let place_x = |pos: &[f64], lab: &[f64]| -> f64 {
            let mut x = 0.0f64;
            for &l in &ls {
                if e.shunt {
                    x = x.max(pos[l] + 2.54).max(lab[l] + 2.54);
                } else {
                    x = x.max(pos[l] + 2.54 + half).max(lab[l] + 1.27 + lw / 2.0);
                }
            }
            up(x)
        };
        let mut x = place_x(&pos, &lab);
        let right = if e.shunt { x + 3.81 + lw } else { x + half.max(lw / 2.0) };
        if right > x_max {
            wrap(&mut sh, &mut band, &mut y, &mut pos, &mut lab);
            x = place_x(&pos, &lab);
        }
        let row = y[ls[0]];
        if e.shunt {
            for &l in &ls {
                sh.wire((pos[l], y[l]), (x, y[l]));
                pos[l] = x;
            }
            if e.both {
                sh.place(e.sym, refd, &val, (x, row), Lab::Right(3.81, D / 2.0), &fields);
                for &l in &ls {
                    lab[l] = x + 3.81 + lw;
                }
            } else if e.sym == "STUB" {
                sh.place(e.sym, refd, &val, (x, row), Lab::Right(3.81, 5.08), &fields);
                lab[ls[0]] = x + 3.81 + lw;
            } else {
                sh.place(e.sym, refd, &val, (x, row + 5.08), Lab::Right(3.81, 0.0), &fields);
                sh.gnd((x, row + 10.16));
                lab[ls[0]] = x + 3.81 + lw;
            }
        } else {
            for &l in &ls {
                sh.wire((pos[l], y[l]), (x - half, y[l]));
                pos[l] = x + half;
                lab[l] = x + lw / 2.0;
            }
            let dy = match e.sym {
                "TL" => 3.81,
                "VIAPAIR" | "CPL" => 6.35,
                _ => 5.08,
            };
            sh.place(e.sym, refd, &val, (x, row), Lab::Above(dy), &fields);
        }
    }
    // the receiver ends, aligned
    let rx_w = 12.7 * 2.0 + if ends.rx.l_nh > 0.0 { 12.7 } else { 0.0 } + 12.0;
    if pos.iter().cloned().fold(0.0, f64::max) + rx_w > x_max + 20.0 {
        wrap(&mut sh, &mut band, &mut y, &mut pos, &mut lab);
    }
    let xr = up(pos.iter().cloned().fold(0.0, f64::max).max(lab.iter().cloned().fold(0.0, f64::max)) + 2.54);
    for (l, nm) in lines.iter().enumerate() {
        sh.wire((pos[l], y[l]), (xr, y[l]));
        rx_end(&mut sh, ends, nm, y[l], xr);
    }
    let y_end = y[nl - 1] + 22.86;
    notes(&mut sh, lc, ends, lib_file, bench_file, tstep, tstop, true, y_end);
    let root = sh.root.clone();
    let sch = sh.file(&format!("kisi lane {}", lc.lane));
    Some((project_file(name, &root), sch))
}

/// The compact sheet: the whole lane as one symbol.
fn compact_project(
    lc: &LaneCircuit,
    ends: &BenchEnds,
    name: &str,
    lib_file: &str,
    bench_file: &str,
    tstep: f64,
    tstop: f64,
    prefix: &str,
) -> (String, String) {
    let mut sh = Sheet::new(&format!("{name}{}", lc.subckt), name);
    let lines: Vec<(&str, f64)> = if lc.pair { vec![("P", 0.0), ("N", 30.48)] } else { vec![("P", 0.0)] };
    let y0 = 63.5;
    let x_lane = 152.4;
    let mut lane_in = Vec::new();
    let mut rx_in = Vec::new();
    for (l, dy) in &lines {
        let y = y0 + dy;
        lane_in.push((tx_end(&mut sh, ends, l, y, 50.8, prefix, bench_file), y));
        let xr = x_lane + 20.32;
        rx_end(&mut sh, ends, l, y, xr);
        rx_in.push((xr, y));
    }
    // the lane symbol with its pins: inputs at x − 12.7, outputs at x + 12.7
    let yl = if lc.pair { y0 + 2.54 } else { y0 };
    let pins = if lc.pair { "1=ap 2=an 3=bp 4=bn 5=ref" } else { "1=a 2=b 3=ref" };
    let lab = Lab::Above(if lc.pair { 10.16 } else { 7.62 });
    sh.place(
        if lc.pair { "LANE4" } else { "LANE2" },
        "XLANE1",
        &lc.subckt,
        (x_lane, yl),
        lab,
        &[
            ("Sim.Device", "SUBCKT".into()),
            ("Sim.Library", bench_file.into()),
            ("Sim.Name", format!("{prefix}_kicad")),
            ("Sim.Pins", pins.into()),
        ],
    );
    // lane pins in schematic coordinates (symbol y up → sheet y down)
    let pin_in: Vec<(f64, f64)> =
        if lc.pair { vec![(x_lane - 12.7, yl - 2.54), (x_lane - 12.7, yl + 2.54)] } else { vec![(x_lane - 12.7, yl)] };
    let pin_out: Vec<(f64, f64)> =
        if lc.pair { vec![(x_lane + 12.7, yl - 2.54), (x_lane + 12.7, yl + 2.54)] } else { vec![(x_lane + 12.7, yl)] };
    let ref_pin = (x_lane, yl + if lc.pair { 7.62 } else { 5.08 } + 2.54);
    for (i, &(x, y)) in lane_in.iter().enumerate() {
        // along the line's row to a column before the lane, then to the pin's height
        let col = x_lane - 20.32 + 2.54 * i as f64;
        sh.route(&[(x, y), (col, y), (col, pin_in[i].1), pin_in[i]]);
    }
    for (i, &end) in rx_in.iter().enumerate() {
        let start = pin_out[i];
        let col = x_lane + 17.78 - 2.54 * i as f64;
        sh.route(&[start, (col, start.1), (col, end.1), end]);
    }
    sh.wire(ref_pin, (ref_pin.0, ref_pin.1 + 2.54));
    sh.gnd((ref_pin.0, ref_pin.1 + 2.54));
    power_flag(&mut sh, 25.4, y0 + 5.08);
    let y_end = y0 + lines.last().map(|l| l.1).unwrap_or(0.0) + 22.86;
    notes(&mut sh, lc, ends, lib_file, bench_file, tstep, tstop, false, y_end);
    let root = sh.root.clone();
    let sch = sh.file(&format!("kisi lane {}", lc.lane));
    (project_file(name, &root), sch)
}

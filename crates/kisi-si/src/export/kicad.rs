//! A small KiCad project that simulates a lane: the lane subcircuit as a symbol
//! (`Sim.Device = SUBCKT`), the transmitter's PRBS source (a subcircuit as well), the
//! profile's linear ends as R, L and C symbols (KiCad infers their models from the reference
//! and the value), ground, and a `.tran` directive.
//!
//! Written in the KiCad 7 schematic format (20230121) with kisi's own symbols in `lib_symbols`, so
//! KiCad 7 and every later version (8, 9, 10) open it without any library; checked with
//! `kicad-cli sch export netlist --format spice` (KiCad 7.0) and ngspice.

use super::circuit::num;
use super::files::{pwl_points, BenchEnds};
use super::{sig4, LaneCircuit};

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
        format!("{:08x}-{:04x}-4{:03x}-8{:03x}-{:012x}", (a >> 32) as u32, (a >> 16) & 0xffff, a & 0xfff, (b >> 52) & 0xfff, b & 0xffff_ffff_ffff)
    }
}

fn q(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n"))
}

const FONT: &str = "(effects (font (size 1.27 1.27)))";
const FONT_HIDE: &str = "(effects (font (size 1.27 1.27)) hide)";

fn prop(name: &str, val: &str, x: f64, y: f64, hide: bool) -> String {
    format!("    (property {} {} (at {x:.2} {y:.2} 0)\n      {}\n    )\n", q(name), q(val), if hide { FONT_HIDE } else { FONT })
}

fn pin(kind: &str, x: f64, y: f64, ang: u32, len: f64, name: &str, number: &str, hide_name: bool) -> String {
    format!(
        "        (pin {kind} line (at {x} {y} {ang}) (length {len}){}\n          (name {} (effects (font (size 1.27 1.27))))\n          (number {} (effects (font (size 1.27 1.27))))\n        )\n",
        if hide_name { " hide" } else { "" },
        q(name),
        q(number)
    )
}

/// kisi's symbols: horizontal and vertical two-pin parts, the source, the lane (2 or 4 ports
/// plus the reference) and ground.
fn lib_symbols(pair: bool) -> String {
    let mut s = String::from("  (lib_symbols\n");
    let two = |name: &str, vertical: bool| -> String {
        let short = name.trim_start_matches("kisi:");
        let (p1, p2) = if vertical { ((0.0, 5.08, 270), (0.0, -5.08, 90)) } else { ((-5.08, 0.0, 0), (5.08, 0.0, 180)) };
        let (w, h) = if vertical { (1.016, 2.54) } else { (2.54, 1.016) };
        format!(
            "    (symbol {} (pin_numbers hide) (pin_names (offset 0) hide) (in_bom yes) (on_board yes)\n{}{}{}{}      (symbol \"{short}_0_1\"\n        (rectangle (start {} {}) (end {w} {}) (stroke (width 0.254) (type default)) (fill (type none)))\n      )\n      (symbol \"{short}_1_1\"\n{}{}      )\n    )\n",
            q(name),
            prop("Reference", "Z", 2.54, 2.54, false),
            prop("Value", "Z", 2.54, -2.54, false),
            prop("Footprint", "", 0.0, 0.0, true),
            prop("Datasheet", "", 0.0, 0.0, true),
            -w,
            -h,
            h,
            pin("passive", p1.0, p1.1, p1.2, 2.54, "~", "1", true),
            pin("passive", p2.0, p2.1, p2.2, 2.54, "~", "2", true),
        )
    };
    s.push_str(&two("kisi:Z_H", false));
    s.push_str(&two("kisi:Z_V", true));
    // source: out at the top, reference at the bottom
    s.push_str(&format!(
        "    (symbol \"kisi:SRC\" (pin_names (offset 0) hide) (in_bom yes) (on_board yes)\n{}{}{}{}      (symbol \"SRC_0_1\"\n        (circle (center 0 0) (radius 2.54) (stroke (width 0.254) (type default)) (fill (type none)))\n        (polyline (pts (xy -1.27 -0.635) (xy -0.635 -0.635) (xy -0.635 0.635) (xy 0.635 0.635) (xy 0.635 -0.635) (xy 1.27 -0.635)) (stroke (width 0) (type default)) (fill (type none)))\n      )\n      (symbol \"SRC_1_1\"\n{}{}      )\n    )\n",
        prop("Reference", "X", 3.81, 1.27, false),
        prop("Value", "SRC", 3.81, -1.27, false),
        prop("Footprint", "", 0.0, 0.0, true),
        prop("Datasheet", "", 0.0, 0.0, true),
        pin("passive", 0.0, 5.08, 270, 2.54, "out", "1", true),
        pin("passive", 0.0, -5.08, 90, 2.54, "ref", "2", true),
    ));
    // lane: inputs on the left, outputs on the right, reference at the bottom
    let (name, h) = if pair { ("LANE4", 7.62) } else { ("LANE2", 5.08) };
    let mut pins = String::new();
    if pair {
        pins.push_str(&pin("passive", -12.7, 2.54, 0, 2.54, "ap", "1", false));
        pins.push_str(&pin("passive", -12.7, -2.54, 0, 2.54, "an", "2", false));
        pins.push_str(&pin("passive", 12.7, 2.54, 180, 2.54, "bp", "3", false));
        pins.push_str(&pin("passive", 12.7, -2.54, 180, 2.54, "bn", "4", false));
        pins.push_str(&pin("passive", 0.0, -h - 2.54, 90, 2.54, "ref", "5", false));
    } else {
        pins.push_str(&pin("passive", -12.7, 0.0, 0, 2.54, "a", "1", false));
        pins.push_str(&pin("passive", 12.7, 0.0, 180, 2.54, "b", "2", false));
        pins.push_str(&pin("passive", 0.0, -h - 2.54, 90, 2.54, "ref", "3", false));
    }
    s.push_str(&format!(
        "    (symbol \"kisi:{name}\" (pin_names (offset 1.016)) (in_bom yes) (on_board yes)\n{}{}{}{}      (symbol \"{name}_0_1\"\n        (rectangle (start -10.16 {h}) (end 10.16 {}) (stroke (width 0.254) (type default)) (fill (type background)))\n      )\n      (symbol \"{name}_1_1\"\n{pins}      )\n    )\n",
        prop("Reference", "X", 0.0, h + 1.27, false),
        prop("Value", "LANE", 0.0, h + 3.81, false),
        prop("Footprint", "", 0.0, 0.0, true),
        prop("Datasheet", "", 0.0, 0.0, true),
        -h
    ));
    s.push_str(&format!(
        "    (symbol \"kisi:GND\" (power) (pin_names (offset 0)) (in_bom yes) (on_board yes)\n{}{}{}{}      (symbol \"GND_0_1\"\n        (polyline (pts (xy 0 0) (xy 0 -1.27) (xy 1.27 -1.27) (xy 0 -2.54) (xy -1.27 -1.27) (xy 0 -1.27)) (stroke (width 0) (type default)) (fill (type none)))\n      )\n      (symbol \"GND_1_1\"\n{}      )\n    )\n",
        prop("Reference", "#PWR", 0.0, -6.35, true),
        prop("Value", "GND", 0.0, -3.81, false),
        prop("Footprint", "", 0.0, 0.0, true),
        prop("Datasheet", "", 0.0, 0.0, true),
        pin("power_in", 0.0, 0.0, 270, 0.0, "GND", "1", true),
    ));
    s.push_str("  )\n");
    s
}

struct Sheet {
    ids: Ids,
    root: String,
    project: String,
    items: Vec<String>,
    pwr: usize,
}

impl Sheet {
    fn wire(&mut self, a: (f64, f64), b: (f64, f64)) {
        if (a.0 - b.0).abs() < 1e-9 && (a.1 - b.1).abs() < 1e-9 {
            return;
        }
        let u = self.ids.next();
        self.items.push(format!("  (wire (pts (xy {:.2} {:.2}) (xy {:.2} {:.2}))\n    (stroke (width 0) (type default))\n    (uuid {u})\n  )\n", a.0, a.1, b.0, b.1));
    }
    fn junction(&mut self, p: (f64, f64)) {
        let u = self.ids.next();
        self.items.push(format!("  (junction (at {:.2} {:.2}) (diameter 0) (color 0 0 0 0)\n    (uuid {u})\n  )\n", p.0, p.1));
    }
    fn text(&mut self, t: &str, at: (f64, f64)) {
        let u = self.ids.next();
        self.items.push(format!("  (text {} (at {:.2} {:.2} 0)\n    (effects (font (size 1.27 1.27)) (justify left bottom))\n    (uuid {u})\n  )\n", q(t), at.0, at.1));
    }
    /// A symbol at `at` with extra (hidden) fields; `npins` pins numbered from 1.
    fn symbol(&mut self, lib_id: &str, reference: &str, value: &str, at: (f64, f64), npins: usize, fields: &[(&str, String)]) {
        let u = self.ids.next();
        let mut s = format!("  (symbol (lib_id {}) (at {:.2} {:.2} 0) (unit 1)\n    (in_bom yes) (on_board yes) (dnp no)\n    (uuid {u})\n", q(lib_id), at.0, at.1);
        let hide_ref = reference.starts_with('#');
        s.push_str(&prop("Reference", reference, at.0 + 2.54, at.1 - 2.54, hide_ref));
        s.push_str(&prop("Value", value, at.0 + 2.54, at.1 + 3.81, false));
        s.push_str(&prop("Footprint", "", at.0, at.1, true));
        s.push_str(&prop("Datasheet", "", at.0, at.1, true));
        for (k, v) in fields {
            s.push_str(&prop(k, v, at.0, at.1, true));
        }
        for p in 1..=npins {
            let u = self.ids.next();
            s.push_str(&format!("    (pin \"{p}\" (uuid {u}))\n"));
        }
        s.push_str(&format!("    (instances\n      (project {}\n        (path \"/{}\"\n          (reference {}) (unit 1)\n        )\n      )\n    )\n  )\n", q(&self.project), self.root, q(reference)));
        self.items.push(s);
    }
    fn gnd(&mut self, at: (f64, f64)) {
        self.pwr += 1;
        let r = format!("#PWR{:02}", self.pwr);
        self.symbol("kisi:GND", &r, "GND", at, 1, &[]);
    }
}

/// A value as KiCad expects it in the Value field of an R, L or C (SI prefix, '.' as decimal).
fn si(v: f64) -> String {
    let (m, p) = if v >= 1.0 {
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

/// The library of the KiCad project: the lane library included and a small wrapper subcircuit
/// `<prefix>_kicad` (KiCad parses the file it names for the model; a short file loads at once), and
/// one PRBS7 source subcircuit per line (`out ref`).
pub fn bench_lib(lc: &LaneCircuit, ends: &BenchEnds, prefix: &str, lib_file: &str) -> (String, f64, f64) {
    let ui = 1.0 / ends.bitrate;
    let bits: Vec<bool> = super::files::prbs7().into_iter().chain(super::files::prbs7()).collect();
    let (rt, rr) = (ends.tx.r.max(0.0), ends.rx.r);
    let k = if rr > 0.0 { (rt + rr) / rr } else { 1.0 };
    let a = ends.swing * k / if lc.pair { 2.0 } else { 1.0 };
    let pts = pwl_points(&bits, ui, ends.rise, a);
    let mut s = format!("* kisi {}: KiCad test bench of lane {}\n.include \"{lib_file}\"\n\n", env!("CARGO_PKG_VERSION"), lc.lane);
    let ports = lc.ports.join(" ");
    s.push_str(&format!("* the lane (subcircuit {} in {lib_file})\n.subckt {prefix}_kicad {ports} ref\nX1 {ports} ref {}\n.ends {prefix}_kicad\n\n", lc.subckt, lc.subckt));
    s.push_str(&format!("* sources: PRBS7 at {} Gbit/s, edge {} ps (10-90 %), emf amplitude {} V per line\n", sig4(ends.bitrate / 1e9), sig4(ends.rise * 1e12), sig4(a)));
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

/// The KiCad project: `<name>.kicad_pro`, `<name>.kicad_sch` and the bench library.
pub fn project(lc: &LaneCircuit, ends: &BenchEnds, name: &str, lib_file: &str, bench_file: &str, tstep: f64, tstop: f64, prefix: &str) -> (String, String) {
    let mut ids = Ids::new(&format!("{name}{}", lc.subckt));
    let root = ids.next();
    let mut sh = Sheet { ids, root: root.clone(), project: name.to_string(), items: vec![], pwr: 0 };
    let lines: Vec<(&str, f64)> = if lc.pair { vec![("P", 0.0), ("N", 30.48)] } else { vec![("P", 0.0)] };
    let y0 = 63.5;
    let x_lane = 152.4;
    let mut lane_in = Vec::new();
    let mut lane_out = Vec::new();
    let mut nref = 0;
    for (li, (l, dy)) in lines.iter().enumerate() {
        let y = y0 + dy;
        // source (vertical): out at the top, ground at the bottom
        let xs = 38.1;
        sh.symbol("kisi:SRC", &format!("XTX{l}1"), &format!("PRBS7 {l}"), (xs, y + 5.08), 2, &[
            ("Sim.Device", "SUBCKT".into()),
            ("Sim.Library", bench_file.into()),
            ("Sim.Name", format!("{prefix}_TX{l}")),
            ("Sim.Pins", "1=out 2=ref".into()),
        ]);
        sh.gnd((xs, y + 10.16));
        let mut x = xs;
        // R_tx (horizontal), C_tx at the die to ground (vertical), L_tx towards the pin
        let series = |sh: &mut Sheet, x: &mut f64, refd: &str, val: Option<f64>| {
            if let Some(v) = val {
                let c = *x + 12.7;
                sh.wire((*x, y), (c - 5.08, y));
                sh.symbol("kisi:Z_H", refd, &si(v), (c, y), 2, &[]);
                *x = c + 5.08;
            }
        };
        series(&mut sh, &mut x, &format!("RTX{l}1"), (ends.tx.r > 0.0).then_some(ends.tx.r));
        if ends.tx.c_pf > 0.0 {
            let c = x + 7.62;
            sh.wire((x, y), (c, y));
            sh.junction((c, y));
            sh.symbol("kisi:Z_V", &format!("CTX{l}1"), &si(ends.tx.c_pf * 1e-12), (c, y + 5.08), 2, &[]);
            sh.gnd((c, y + 10.16));
            x = c;
        }
        series(&mut sh, &mut x, &format!("LTX{l}1"), (ends.tx.l_nh > 0.0).then_some(ends.tx.l_nh * 1e-9));
        lane_in.push((x, y));
        // receiver side
        let mut xr = x_lane + 12.7 + 7.62;
        let yr = y;
        let mut pts_out = vec![(x_lane + 12.7, if lc.pair { y0 + 2.54 - 5.08 * li as f64 } else { y0 })];
        if ends.rx.l_nh > 0.0 {
            let c = xr + 5.08;
            sh.symbol("kisi:Z_H", &format!("LRX{l}1"), &si(ends.rx.l_nh * 1e-9), (c, yr), 2, &[]);
            pts_out.push((c - 5.08, yr));
            xr = c + 5.08;
        } else {
            pts_out.push((xr, yr));
        }
        let mut shunts = Vec::new();
        if ends.rx.c_pf > 0.0 {
            shunts.push((format!("CRX{l}1"), ends.rx.c_pf * 1e-12));
        }
        if ends.rx.r > 0.0 {
            shunts.push((format!("RRX{l}1"), ends.rx.r));
        }
        let mut xx = xr;
        for (i, (r, v)) in shunts.iter().enumerate() {
            let c = xx + 7.62;
            sh.wire((xx, yr), (c, yr));
            if i + 1 < shunts.len() {
                sh.junction((c, yr));
            }
            sh.symbol("kisi:Z_V", r, &si(*v), (c, yr + 5.08), 2, &[]);
            sh.gnd((c, yr + 10.16));
            xx = c;
        }
        lane_out.push(pts_out);
        nref += 1;
    }
    let _ = nref;
    // the lane symbol with its pins: inputs at x − 12.7, outputs at x + 12.7
    let yl = if lc.pair { y0 + 2.54 } else { y0 };
    let pins = if lc.pair { "1=ap 2=an 3=bp 4=bn 5=ref" } else { "1=a 2=b 3=ref" };
    sh.symbol(if lc.pair { "kisi:LANE4" } else { "kisi:LANE2" }, "XLANE1", &lc.subckt, (x_lane, yl), if lc.pair { 5 } else { 3 }, &[
        ("Sim.Device", "SUBCKT".into()),
        ("Sim.Library", bench_file.into()),
        ("Sim.Name", format!("{prefix}_kicad")),
        ("Sim.Pins", pins.into()),
    ]);
    // lane pins in schematic coordinates (symbol y up → sheet y down)
    let pin_in: Vec<(f64, f64)> = if lc.pair { vec![(x_lane - 12.7, yl - 2.54), (x_lane - 12.7, yl + 2.54)] } else { vec![(x_lane - 12.7, yl)] };
    let pin_out: Vec<(f64, f64)> = if lc.pair { vec![(x_lane + 12.7, yl - 2.54), (x_lane + 12.7, yl + 2.54)] } else { vec![(x_lane + 12.7, yl)] };
    let ref_pin = (x_lane, yl + if lc.pair { 7.62 } else { 5.08 } + 2.54);
    for (i, &(x, y)) in lane_in.iter().enumerate() {
        // route: along the line's row to a column before the lane, then to the pin's height
        let col = x_lane - 20.32 + 2.54 * i as f64;
        sh.wire((x, y), (col, y));
        sh.wire((col, y), (col, pin_in[i].1));
        sh.wire((col, pin_in[i].1), pin_in[i]);
    }
    for (i, pts) in lane_out.iter().enumerate() {
        let start = pin_out[i];
        let end = pts[1];
        let col = x_lane + 17.78 - 2.54 * i as f64;
        sh.wire(start, (col, start.1));
        sh.wire((col, start.1), (col, end.1));
        sh.wire((col, end.1), end);
    }
    sh.wire(ref_pin, (ref_pin.0, ref_pin.1 + 2.54));
    sh.gnd((ref_pin.0, ref_pin.1 + 2.54));
    sh.text(&format!(".tran {} {}", num(tstep), num(tstop)), (38.1, 30.48));
    sh.text(
        &format!(
            "kisi {}: lane {}\nTransmitter (left) and receiver (right): the linear ends of profile {} per line.\nThe lane subcircuit {} is in {lib_file} (through {bench_file}, with the PRBS7 sources).\nProbe the receiver die (the node at C/R of the receiver) – for a pair the difference of P and N.\nAdd a termination or swap the ends here; IBIS ends: replace the source/receiver by KiCad IBIS symbols.",
            env!("CARGO_PKG_VERSION"),
            lc.lane,
            ends.profile,
            lc.subckt
        ),
        (38.1, 20.32),
    );
    let mut s = format!("(kicad_sch (version 20230121) (generator eeschema)\n\n  (uuid {root})\n\n  (paper \"A4\")\n\n  (title_block\n    (title {})\n    (comment 1 {})\n  )\n\n", q(&format!("kisi lane {}", lc.lane)), q("generated by kisi – linear test bench"));
    s.push_str(&lib_symbols(lc.pair));
    s.push('\n');
    for it in &sh.items {
        s.push_str(it);
    }
    s.push_str("\n  (sheet_instances\n    (path \"/\" (page \"1\"))\n  )\n)\n");
    let pro = format!(
        "{{\n  \"meta\": {{\n    \"filename\": \"{name}.kicad_pro\",\n    \"version\": 1\n  }},\n  \"schematic\": {{\n    \"legacy_lib_dir\": \"\",\n    \"legacy_lib_list\": []\n  }},\n  \"sheets\": [\n    [\n      \"{root}\",\n      \"\"\n    ]\n  ],\n  \"text_variables\": {{}}\n}}\n"
    );
    (pro, s)
}

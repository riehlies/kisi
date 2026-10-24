//! The files of an export: the subcircuit library, test benches for ngspice and Xyce, the
//! S-parameter decks, Touchstone files, a KiCad project and a README.

use super::circuit::num;
use super::{sig4, LaneCircuit};
use crate::eye::EndModel;
use crate::num::Cx;

/// One file of an export (path relative to the export folder).
#[derive(Debug, Clone, serde::Serialize)]
pub struct ExportFile {
    pub path: String,
    pub content: String,
}

/// What to write.
#[derive(Debug, Clone, Copy)]
pub struct Formats {
    pub spice: bool,
    pub xyce: bool,
    pub touchstone: bool,
    pub kicad: bool,
}

impl Formats {
    pub const ALL: Formats = Formats { spice: true, xyce: true, touchstone: true, kicad: true };
    /// From a comma-separated list: spice, xyce, touchstone, kicad, all.
    pub fn parse(s: &str) -> Result<Formats, String> {
        let mut f = Formats { spice: false, xyce: false, touchstone: false, kicad: false };
        for p in s.split(',').map(|p| p.trim().to_ascii_lowercase()).filter(|p| !p.is_empty()) {
            match p.as_str() {
                "spice" | "ngspice" => f.spice = true,
                "xyce" => f.xyce = true,
                "touchstone" | "snp" => f.touchstone = true,
                "kicad" => f.kicad = true,
                "all" => f = Formats::ALL,
                _ => return Err(format!("--format {p}: use spice, xyce, touchstone, kicad or all (comma-separated)")),
            }
        }
        if !(f.spice || f.xyce || f.touchstone || f.kicad) {
            return Err("--format: nothing to write".into());
        }
        Ok(f)
    }
}

/// The linear ends of the test bench (per line), from the interface profile as kisi's eye uses
/// them (a termination next to the receiver as the receiver's termination).
#[derive(Debug, Clone)]
pub struct BenchEnds {
    pub profile: String,
    pub bitrate: f64,
    /// 10–90 % rise time of the transmitter edge (s)
    pub rise: f64,
    /// amplitude at the receiver at DC (V; a pair: differential, ±)
    pub swing: f64,
    pub tx: EndModel,
    pub rx: EndModel,
    pub notes: Vec<String>,
}

/// Simulator dialect of a deck.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Dialect {
    Ngspice,
    Xyce,
}

/// PRBS7 (x⁷ + x⁶ + 1), 127 bits.
pub fn prbs7() -> Vec<bool> {
    let mut s: u8 = 0x7f;
    (0..127)
        .map(|_| {
            let b = ((s >> 6) ^ (s >> 5)) & 1;
            s = ((s << 1) | b) & 0x7f;
            b == 1
        })
        .collect()
}

/// PWL points (t, v) of an NRZ pattern with linear edges (10–90 % = `rise`), values ±a, one bit
/// of the low level first.
pub fn pwl_points(bits: &[bool], ui: f64, rise: f64, a: f64) -> Vec<(f64, f64)> {
    let tr = rise / 0.8;
    let mut pts = vec![(0.0, -a)];
    let mut prev = false;
    for (i, &b) in bits.iter().enumerate() {
        if b != prev {
            let t = (i + 1) as f64 * ui;
            let (v0, v1) = if b { (-a, a) } else { (a, -a) };
            pts.push((t - tr / 2.0, v0));
            pts.push((t + tr / 2.0, v1));
            prev = b;
        }
    }
    pts
}

/// A PWL source line with continuation lines.
fn pwl_source(name: &str, p: &str, n: &str, pts: &[(f64, f64)], scale: f64) -> String {
    let mut s = format!("{name} {p} {n} PWL(");
    let mut line_len = s.len();
    for (i, (t, v)) in pts.iter().enumerate() {
        let item = format!("{}{} {}", if i == 0 { "" } else { " " }, num(*t), num(v * scale));
        if line_len + item.len() > 100 {
            s.push_str("\n+");
            line_len = 1;
        }
        s.push_str(&item);
        line_len += item.len();
    }
    s.push(')');
    s
}

fn header(lc: &LaneCircuit, info: &[String], what: &str) -> String {
    let mut s = format!("* kisi {}: {what}\n* lane {}\n", env!("CARGO_PKG_VERSION"), lc.lane);
    for l in info {
        s.push_str(&format!("* {l}\n"));
    }
    s
}

/// Element lines of one end (per line `k` of the lane): transmitter = emf → R → die (C to
/// ground) → L → pin; receiver = pin → L → die, C and R to ground. Returns lines and the die node.
fn tx_end(k: &str, pin: &str, e: &EndModel, src: &str) -> (Vec<String>, String) {
    let mut v = Vec::new();
    let die = format!("txd_{k}");
    let emf = format!("txe_{k}");
    v.push(src.replace("{emf}", &emf));
    if e.r > 0.0 {
        v.push(format!("RTX_{k} {emf} {die} {}", num(e.r)));
    } else {
        v.push(format!("VTXS_{k} {emf} {die} 0"));
    }
    if e.c_pf > 0.0 {
        v.push(format!("CTX_{k} {die} 0 {}", num(e.c_pf * 1e-12)));
    }
    if e.l_nh > 0.0 {
        v.push(format!("LTX_{k} {die} {pin} {}", num(e.l_nh * 1e-9)));
    } else {
        v.push(format!("VTXL_{k} {die} {pin} 0"));
    }
    (v, die)
}

fn rx_end(k: &str, pin: &str, e: &EndModel) -> (Vec<String>, String) {
    let mut v = Vec::new();
    let die = format!("rxd_{k}");
    if e.l_nh > 0.0 {
        v.push(format!("LRX_{k} {pin} {die} {}", num(e.l_nh * 1e-9)));
    } else {
        v.push(format!("VRXL_{k} {pin} {die} 0"));
    }
    if e.c_pf > 0.0 {
        v.push(format!("CRX_{k} {die} 0 {}", num(e.c_pf * 1e-12)));
    }
    if e.r > 0.0 {
        v.push(format!("RRX_{k} {die} 0 {}", num(e.r)));
    } else {
        // high-impedance receiver: a DC path for the simulator only
        v.push(format!("RRX_{k} {die} 0 1e9"));
    }
    (v, die)
}

/// Emf amplitude per line so that the receiver sees ±swing at DC (a pair: differential).
fn emf_amp(ends: &BenchEnds, pair: bool) -> f64 {
    let (rt, rr) = (ends.tx.r.max(0.0), ends.rx.r);
    let k = if rr > 0.0 { (rt + rr) / rr } else { 1.0 };
    let a = ends.swing * k;
    if pair {
        a / 2.0
    } else {
        a
    }
}

/// Transient test bench: PRBS7 at the profile's bit rate through the lane with the profile's
/// linear ends, two pattern periods.
pub fn bench(lc: &LaneCircuit, lib_file: &str, ends: &BenchEnds, d: Dialect) -> String {
    let ui = 1.0 / ends.bitrate;
    let bits: Vec<bool> = prbs7().into_iter().chain(prbs7()).collect();
    let a = emf_amp(ends, lc.pair);
    let pts = pwl_points(&bits, ui, ends.rise, a);
    let tstop = (bits.len() + 2) as f64 * ui + 2.0 * lc.delay_ps * 1e-12;
    let tstep = (ends.rise / 10.0).min(ui / 20.0);
    let mut info = vec![
        format!("transient test bench: PRBS7 at {} Gbit/s, edge {} ps (10-90 %), profile {}", sig4(ends.bitrate / 1e9), sig4(ends.rise * 1e12), ends.profile),
        format!("transmitter per line: R {} ohm, C {} pF, L {} nH; receiver per line: L {} nH, C {} pF, R {} ohm", sig4(ends.tx.r), sig4(ends.tx.c_pf), sig4(ends.tx.l_nh), sig4(ends.rx.l_nh), sig4(ends.rx.c_pf), if ends.rx.r > 0.0 { sig4(ends.rx.r) } else { "open".into() }),
        format!("emf scaled so that the receiver sees +-{} V at DC{}", sig4(ends.swing), if lc.pair { " (differential; P and N in opposite phase)" } else { "" }),
    ];
    info.extend(ends.notes.iter().cloned());
    let mut s = header(lc, &info, "transient test bench");
    s.push_str(&format!(".include \"{lib_file}\"\n\n"));
    let lines: Vec<(&str, &str, &str, f64)> = if lc.pair { vec![("p", "ap", "bp", 1.0), ("n", "an", "bn", -1.0)] } else { vec![("p", "a", "b", 1.0)] };
    let mut outs = Vec::new();
    for (k, pa, pb, sign) in &lines {
        let src = pwl_source(&format!("VTX_{k}"), "{emf}", "0", &pts, *sign);
        let (t, _) = tx_end(k, pa, &ends.tx, &src);
        let (r, die) = rx_end(k, pb, &ends.rx);
        s.push_str(&format!("* line {k}: transmitter\n{}\n* line {k}: receiver\n{}\n", t.join("\n"), r.join("\n")));
        outs.push(die);
    }
    let ports = if lc.pair { "ap an bp bn 0" } else { "a b 0" };
    s.push_str(&format!("\nXLANE {ports} {}\n\n", lc.subckt));
    let probe = if lc.pair { format!("v({})-v({})", outs[0], outs[1]) } else { format!("v({})", outs[0]) };
    match d {
        Dialect::Ngspice => {
            s.push_str(&format!(".tran {} {}\n", num(tstep), num(tstop)));
            s.push_str(".control\nrun\n");
            s.push_str(&format!("wrdata bench_ngspice.txt {probe}\n"));
            s.push_str(".endc\n.end\n");
        }
        Dialect::Xyce => {
            s.push_str(&format!(".tran {} {}\n", num(tstep), num(tstop)));
            let probe = if lc.pair { format!("{{V({})-V({})}}", outs[0], outs[1]) } else { format!("V({})", outs[0]) };
            s.push_str(&format!(".print tran format=csv file=bench_xyce.csv {probe}\n.end\n"));
        }
    }
    s
}

/// S-parameter deck: every port with the reference impedance, kisi's frequency grid.
pub fn sparam_deck(lc: &LaneCircuit, lib_file: &str, d: Dialect) -> String {
    let n = lc.ports.len();
    let mut s = header(lc, &lc.port_info.iter().enumerate().map(|(i, p)| format!("port {} = {} = {p}", i + 1, lc.ports[i])).collect::<Vec<_>>(), &format!("S-parameters of the lane subcircuit, reference {} ohm per port", sig4(lc.z_ref)));
    s.push_str(&format!(".include \"{lib_file}\"\n\n"));
    let (f0, f1, np) = (lc.freq[0], *lc.freq.last().unwrap(), lc.freq.len());
    for (i, p) in lc.ports.iter().enumerate() {
        match d {
            Dialect::Ngspice => s.push_str(&format!("VP{} {p} 0 dc 0 ac 1 portnum {} z0 {}\n", i + 1, i + 1, num(lc.z_ref))),
            Dialect::Xyce => s.push_str(&format!("P{} {p} 0 port={} z0={}\n", i + 1, i + 1, num(lc.z_ref))),
        }
    }
    s.push_str(&format!("XLANE {} 0 {}\n\n", lc.ports.join(" "), lc.subckt));
    match d {
        Dialect::Ngspice => {
            s.push_str(&format!(".sp lin {np} {} {}\n.control\nrun\n", num(f0), num(f1)));
            let vecs: Vec<String> = (1..=n).flat_map(|i| (1..=n).map(move |j| format!("S_{i}_{j}"))).collect();
            s.push_str(&format!("wrdata sparam_ngspice.txt {}\n.endc\n.end\n", vecs.join(" ")));
        }
        Dialect::Xyce => {
            s.push_str(&format!(".ac lin {np} {} {}\n", num(f0), num(f1)));
            s.push_str(&format!(".lin sparcalc=1 format=touchstone2 dataformat=ri file=sparam_xyce.s{n}p\n.end\n"));
        }
    }
    s
}

/// Touchstone of an S-matrix list (n ports, row-major per frequency).
pub fn touchstone(comments: &[String], freq: &[f64], s: &[Vec<Cx>], n: usize, z0: f64) -> String {
    let mut out = String::new();
    for c in comments {
        out.push_str(&format!("! {c}\n"));
    }
    out.push_str(&format!("# HZ S RI R {}\n", num(z0)));
    let cell = |x: Cx| if x.re == 0.0 && x.im == 0.0 { "0 0".to_string() } else { format!("{:.6e} {:.6e}", x.re, x.im) };
    for (k, &f) in freq.iter().enumerate() {
        let m = &s[k];
        if n == 2 {
            // Touchstone 2-port order: S11 S21 S12 S22
            out.push_str(&format!("{:.6e} {} {} {} {}\n", f, cell(m[0]), cell(m[2]), cell(m[1]), cell(m[3])));
            continue;
        }
        for i in 0..n {
            let mut line = if i == 0 { format!("{f:.6e}") } else { " ".to_string() };
            for j in 0..n {
                if j > 0 && j % 4 == 0 {
                    out.push_str(&line);
                    out.push('\n');
                    line = " ".to_string();
                }
                line.push(' ');
                line.push_str(&cell(m[i * n + j]));
            }
            out.push_str(&line);
            out.push('\n');
        }
    }
    out
}

/// Lines describing the lane (for headers and the README).
pub fn lane_info(lc: &LaneCircuit, board: &str) -> Vec<String> {
    let mut v = vec![format!("board {board}"), format!("subcircuit {} ({})", lc.subckt, if lc.pair { "pair: P and N on their own paths, coupled sections in modal form" } else if lc.odd_mode { "pair in the odd mode: one line with the odd-mode models" } else { "single line" })];
    for (i, p) in lc.port_info.iter().enumerate() {
        v.push(format!("port {} ({}): {p}", i + 1, lc.ports[i]));
    }
    v.push(format!("track length {} mm, delay {} ps, reference {} ohm per port", sig4(lc.length_mm), sig4(lc.delay_ps), sig4(lc.z_ref)));
    v
}

/// The lane's S-parameters from kisi's model as Touchstone (.s2p / .s4p).
pub fn lane_touchstone(lc: &LaneCircuit, board: &str) -> String {
    let mut c = vec![format!("kisi {}: S-parameters of lane {} (kisi's channel model of the board)", env!("CARGO_PKG_VERSION"), lc.lane)];
    c.extend(lane_info(lc, board));
    if lc.pair {
        c.push("mixed mode with this order: differential port 1 = (port 1 - port 2)/sqrt(2), port 2 = (port 3 - port 4)/sqrt(2)".into());
    }
    if lc.odd_mode {
        c.push(format!("odd mode per line: differential reference {} ohm", sig4(2.0 * lc.z_ref)));
    }
    c.push("the receiver's termination resistors are not included (they act as the receiver's termination)".into());
    touchstone(&c, &lc.freq, &lc.s_kisi, lc.ports.len(), lc.z_ref)
}

/// The library file: header and the subcircuits.
pub fn lib_file(lc: &LaneCircuit, board: &str, fmax: f64, per_lambda: f64) -> String {
    let mut s = String::new();
    s.push_str(&format!("* kisi {}: lane {} as a SPICE subcircuit (ngspice, Xyce, LTspice, KiCad)\n", env!("CARGO_PKG_VERSION"), lc.lane));
    for l in lane_info(lc, board) {
        s.push_str(&format!("* {l}\n"));
    }
    s.push_str(&format!(
        "* model: kisi's channel sections; lines as RLGC ladders ({} sections per wavelength at {} GHz) with fitted\n\
         *   Foster (R(f), L_int(f)) and Debye (G(f), C(f)) networks, vias as kisi's via ladder, parts with estimated\n\
         *   parasitics, open stubs; coupled sections of a pair as even/odd chains between ideal mode converters (E/F)\n",
        sig4(per_lambda),
        sig4(fmax / 1e9)
    ));
    s.push_str(&format!(
        "* check against kisi's S-parameters (same model): |dS21| <= {} dB to 1.5 x Nyquist ({} dB to the top), largest |dS| {} to 1.5 x Nyquist ({} to the top)\n* fits of the main line sections (each at least 10 % of the length): R {} %, X {} %, G {} %, B {} %; via ladders {} %{}\n",
        sig4(lc.fidelity.max_ds21_db_nyq),
        sig4(lc.fidelity.max_ds21_db),
        sig4(lc.fidelity.max_ds_nyq),
        sig4(lc.fidelity.max_ds),
        sig4(lc.fidelity.fit_line_pct[0]),
        sig4(lc.fidelity.fit_line_pct[1]),
        sig4(lc.fidelity.fit_line_pct[2]),
        sig4(lc.fidelity.fit_line_pct[3]),
        sig4(lc.fidelity.fit_via_pct),
        if lc.fidelity.via_spans_active > 0 { format!(" ({} via spans of kisi's model not passive, fitted passive)", lc.fidelity.via_spans_active) } else { String::new() }
    ));
    s.push_str(&format!("* elements after flattening: {} R, {} L, {} C, {} sources; {} RLGC sections\n", lc.counts.r, lc.counts.l, lc.counts.c, lc.counts.sources, lc.counts.sections));
    s.push_str(&format!("* usage: X1 {} 0 {}   (the last port is the reference)\n\n", lc.ports.join(" "), lc.subckt));
    s.push_str(&lc.lib);
    s
}

/// File-name stem of a lane: its last name component.
pub fn stem(lane: &str) -> String {
    let t: String = lane.rsplit('/').next().unwrap_or(lane).chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' }).collect();
    if t.is_empty() {
        "lane".into()
    } else {
        t
    }
}

/// All files of one lane's export, under `dir` ("" or "name/").
#[allow(clippy::too_many_arguments)]
pub fn lane_files(lc: &LaneCircuit, ends: &BenchEnds, f: Formats, board: &str, fmax: f64, per_lambda: f64, dir: &str) -> Vec<ExportFile> {
    let st = stem(&lc.lane);
    let lib = format!("{st}.lib");
    let mut out = Vec::new();
    let mut add = |p: String, c: String| out.push(ExportFile { path: format!("{dir}{p}"), content: c });
    if f.spice || f.xyce || f.kicad {
        add(lib.clone(), lib_file(lc, board, fmax, per_lambda));
    }
    if f.spice {
        add("bench_ngspice.cir".into(), bench(lc, &lib, ends, Dialect::Ngspice));
        add("sparam_ngspice.cir".into(), sparam_deck(lc, &lib, Dialect::Ngspice));
    }
    if f.xyce {
        add("bench_xyce.cir".into(), bench(lc, &lib, ends, Dialect::Xyce));
        add("sparam_xyce.cir".into(), sparam_deck(lc, &lib, Dialect::Xyce));
    }
    if f.touchstone {
        add(format!("{st}.s{}p", lc.ports.len()), lane_touchstone(lc, board));
    }
    if f.kicad {
        let prefix = lc.subckt.trim_end_matches("_lane").to_string();
        let bench_name = format!("{st}_kicad.lib");
        let (bl, tstep, tstop) = super::kicad::bench_lib(lc, ends, &prefix, &lib);
        let (pro, sch) = super::kicad::project(lc, ends, &st, &lib, &bench_name, tstep, tstop, &prefix);
        add(bench_name, bl);
        add(format!("{st}.kicad_pro"), pro);
        add(format!("{st}.kicad_sch"), sch);
    }
    add("README.md".into(), readme(lc, ends, f, board, &st));
    out
}

/// What the export folder of a lane contains and how to use it.
pub fn readme(lc: &LaneCircuit, ends: &BenchEnds, f: Formats, board: &str, st: &str) -> String {
    let n = lc.ports.len();
    let mut s = format!("# kisi export: lane {}\n\nkisi {} – board `{board}`. The lane's board channel as kisi's channel check models it, for circuit and RF simulators.\n\n", lc.lane, env!("CARGO_PKG_VERSION"));
    s.push_str("## Ports\n\n| port | node | on the board |\n|---|---|---|\n");
    for (i, p) in lc.port_info.iter().enumerate() {
        s.push_str(&format!("| {} | `{}` | {p} |\n", i + 1, lc.ports[i]));
    }
    s.push_str(&format!("| – | `ref` | reference (ground); connect to node 0 |\n\nReference impedance {} Ω per port{}. Track length {} mm, delay {} ps.\n\n", sig4(lc.z_ref), if lc.pair || lc.odd_mode { format!(" ({} Ω differential)", sig4(2.0 * lc.z_ref)) } else { String::new() }, sig4(lc.length_mm), sig4(lc.delay_ps)));
    s.push_str("## Files\n\n");
    if f.spice || f.xyce || f.kicad {
        s.push_str(&format!("* `{st}.lib` – the lane as SPICE subcircuit `{}` (plain R, L, C, E, F, V elements: ngspice, Xyce, LTspice, KiCad). One subcircuit per line section, via, part and stub; the lane subcircuit last.\n", lc.subckt));
    }
    if f.spice {
        s.push_str("* `bench_ngspice.cir` – transient test bench (PRBS7, the profile's linear ends): `ngspice -b bench_ngspice.cir` writes `bench_ngspice.txt` (time, receiver voltage).\n");
        s.push_str(&format!("* `sparam_ngspice.cir` – S-parameter analysis on kisi's frequency grid: `ngspice -b sparam_ngspice.cir` writes `sparam_ngspice.txt` (S_i_j for i, j = 1…{n}).\n"));
    }
    if f.xyce {
        s.push_str(&format!("* `bench_xyce.cir`, `sparam_xyce.cir` – the same for Xyce (`.print tran` to CSV; `.lin` writes `sparam_xyce.s{n}p`).\n"));
    }
    if f.touchstone {
        s.push_str(&format!("* `{st}.s{n}p` – kisi's S-parameters of the same channel (Touchstone 1.0, RI) for RF tools (scikit-rf, PyBERT, Qucs-S/Qucsator).\n"));
    }
    if f.kicad {
        s.push_str(&format!("* `{st}.kicad_pro`, `{st}.kicad_sch`, `{st}_kicad.lib` – a KiCad simulation project (KiCad 7 format, opens in KiCad 7–10): the lane as a symbol, the PRBS7 sources and the linear ends as R, L, C symbols, a `.tran` directive. Open it, then Inspect → Simulator → Run, and probe the receiver die.\n"));
    }
    s.push_str("\n## Test bench ends (per line)\n\n");
    s.push_str(&format!("Profile `{}`: {} Gbit/s, edge {} ps (10–90 %), the receiver sees ±{} V at DC{}.\n\n", ends.profile, sig4(ends.bitrate / 1e9), sig4(ends.rise * 1e12), sig4(ends.swing), if lc.pair { " (differential)" } else { "" }));
    s.push_str(&format!("* transmitter: emf → R {} Ω → die (C {} pF to ground) → L {} nH → port\n", sig4(ends.tx.r), sig4(ends.tx.c_pf), sig4(ends.tx.l_nh)));
    s.push_str(&format!("* receiver: port → L {} nH → die: C {} pF and R {} to ground\n", sig4(ends.rx.l_nh), sig4(ends.rx.c_pf), if ends.rx.r > 0.0 { format!("{} Ω", sig4(ends.rx.r)) } else { "open (high impedance)".into() }));
    for nte in &ends.notes {
        s.push_str(&format!("* {nte}\n"));
    }
    s.push_str("\n## Model\n\n");
    s.push_str("* Line sections: RLGC ladders of π sections (at least the number per wavelength at the top frequency given in the library header); per section the external inductance, a Foster network (R0 + R‖L terms) for the conductor impedance R(f) + jωL_int(f) and a Debye network (C + R–C branches) for the dielectric admittance G(f) + jωC(f), fitted to kisi's line model (skin effect, Huray roughness, Djordjevic-Sarkar) with positive elements only – passive and causal by construction.\n");
    s.push_str("* Vias: kisi's via model – the physics-based ladder (node capacitances, plane-pair impedances, open barrel stubs, junction inductances) or the coaxial section stack. The via models run from via centre to via centre; the feed trace they remove or add is taken from the neighbouring line sections.\n");
    s.push_str("* Parts (series and shunt) with the parasitics kisi uses (estimates from `data/components.json`); open stubs as their own chain. A series capacitor has 1 GΩ across it (a DC path for the simulator).\n");
    if lc.pair {
        s.push_str("* Pair: both lines on their own paths; coupled runs, via pairs and stub pairs as modal blocks – even- and odd-mode chains between ideal mode converters (E and F sources: V_e = (V_P + V_N)/2, V_o = (V_P − V_N)/2, I_P = I_e + I_o, I_N = I_e − I_o), as kisi computes them. Differential and common mode, mode conversion and skew are included.\n");
    }
    s.push_str(&format!(
        "\nCheck against kisi's own S-parameters of this channel (the circuit evaluated in kisi): up to 1.5 × Nyquist |ΔS21| ≤ {} dB and the largest difference of any S-parameter {}; up to {} GHz {} dB and {}. Fits of the main line sections (each at least 10 % of the length): R(f) within {} %, X(f) {} %, G(f) {} %, B(f) {} %; via ladders {} %. `validation/channel_export/` in the kisi repository runs these files in ngspice and compares.\n",
        sig4(lc.fidelity.max_ds21_db_nyq),
        sig4(lc.fidelity.max_ds_nyq),
        sig4(lc.freq.last().copied().unwrap_or(0.0) / 1e9),
        sig4(lc.fidelity.max_ds21_db),
        sig4(lc.fidelity.max_ds),
        sig4(lc.fidelity.fit_line_pct[0]),
        sig4(lc.fidelity.fit_line_pct[1]),
        sig4(lc.fidelity.fit_line_pct[2]),
        sig4(lc.fidelity.fit_line_pct[3]),
        sig4(lc.fidelity.fit_via_pct)
    ));
    s.push_str("\nWhy it is not exact: kisi's roughness adds resistance without its internal inductance, the via models use a constant loss tangent and a barrel resistance without internal inductance, and some via spans have a negative resistance at high frequency – a passive, causal circuit cannot follow those exactly; the fits keep the loss and, closer, the reactance (the delay).\n");
    s.push_str(&format!("\nElements after flattening: {} R, {} L, {} C, {} controlled sources and probes; {} RLGC sections.\n", lc.counts.r, lc.counts.l, lc.counts.c, lc.counts.sources, lc.counts.sections));
    s.push_str("\n## Not included\n\n* The receiver's termination resistors on the board: they act as the receiver's termination (bench and Touchstone leave them out, as kisi's eye does).\n* The channel behind a connector, crosstalk from other lanes (kisi's coupling scan is separate), non-linear buffers: IBIS (KiCad's KIBIS) ends need `kisi.json` IBIS models – not part of this export yet; replace the source and the receiver in the KiCad schematic by KiCad's IBIS symbols by hand.\n");
    if !lc.notes.is_empty() {
        s.push_str("\n## Notes from kisi's model\n\n");
        for n in &lc.notes {
            s.push_str(&format!("* {n}\n"));
        }
    }
    s
}

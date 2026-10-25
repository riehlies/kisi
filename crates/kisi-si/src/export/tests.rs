use super::files::{prbs7, Formats};
use super::*;
use crate::topology::{buses, ComponentData};

/// U1 → U2: a single line SIG (30 mm on F.Cu, a series resistor R1, 10 mm on B.Cu through a via)
/// and a pair D_P/D_N (40 mm, 0.15 mm gap) over GND planes.
const PCB: &str = r#"(kicad_pcb (version 20241229) (generator "pcbnew")
  (layers (0 "F.Cu" signal) (4 "In1.Cu" signal) (6 "In2.Cu" signal) (2 "B.Cu" signal))
  (setup (stackup
    (layer "F.Cu" (type "copper") (thickness 0.035))
    (layer "dielectric 1" (type "prepreg") (thickness 0.2) (material "FR4") (epsilon_r 4.3) (loss_tangent 0.02))
    (layer "In1.Cu" (type "copper") (thickness 0.035))
    (layer "dielectric 2" (type "core") (thickness 1.0) (material "FR4") (epsilon_r 4.5) (loss_tangent 0.02))
    (layer "In2.Cu" (type "copper") (thickness 0.035))
    (layer "dielectric 3" (type "prepreg") (thickness 0.2) (material "FR4") (epsilon_r 4.3) (loss_tangent 0.02))
    (layer "B.Cu" (type "copper") (thickness 0.035))))
  (net 0 "") (net 1 "SIG") (net 2 "SIG2") (net 3 "D_P") (net 4 "D_N") (net 5 "GND")
  (footprint "U" (layer "F.Cu") (at 0 0) (property "Reference" "U1") (property "Value" "FPGA") (attr smd)
    (pad "1" smd rect (at 0 0) (size 0.5 0.5) (layers "F.Cu") (net 1 "SIG"))
    (pad "2" smd rect (at 0 5) (size 0.3 0.3) (layers "F.Cu") (net 3 "D_P"))
    (pad "3" smd rect (at 0 5.45) (size 0.3 0.3) (layers "F.Cu") (net 4 "D_N")))
  (footprint "R" (layer "F.Cu") (at 30.5 0) (property "Reference" "R1") (property "Value" "22") (attr smd)
    (pad "1" smd rect (at -0.5 0) (size 0.6 0.6) (layers "F.Cu") (net 1 "SIG"))
    (pad "2" smd rect (at 0.5 0) (size 0.6 0.6) (layers "F.Cu") (net 2 "SIG2")))
  (footprint "U" (layer "B.Cu") (at 45 0) (property "Reference" "U2") (property "Value" "ADC") (attr smd)
    (pad "1" smd rect (at 0 0) (size 0.5 0.5) (layers "B.Cu") (net 2 "SIG2"))
    (pad "2" smd rect (at 0 5) (size 0.3 0.3) (layers "B.Cu") (net 3 "D_P"))
    (pad "3" smd rect (at 0 5.45) (size 0.3 0.3) (layers "B.Cu") (net 4 "D_N")))
  (segment (start 0 0) (end 30 0) (width 0.35) (layer "F.Cu") (net 1))
  (segment (start 31 0) (end 35 0) (width 0.35) (layer "F.Cu") (net 2))
  (via (at 35 0) (size 0.6) (drill 0.3) (layers "F.Cu" "B.Cu") (net 2))
  (segment (start 35 0) (end 45 0) (width 0.35) (layer "B.Cu") (net 2))
  (segment (start 0 5) (end 40 5) (width 0.15) (layer "F.Cu") (net 3))
  (segment (start 0 5.45) (end 40 5.45) (width 0.15) (layer "F.Cu") (net 4))
  (via (at 40 5) (size 0.45) (drill 0.2) (layers "F.Cu" "B.Cu") (net 3))
  (via (at 40 5.45) (size 0.45) (drill 0.2) (layers "F.Cu" "B.Cu") (net 4))
  (segment (start 40 5) (end 45 5) (width 0.15) (layer "B.Cu") (net 3))
  (segment (start 40 5.45) (end 45 5.45) (width 0.15) (layer "B.Cu") (net 4))
  (zone (net 5) (net_name "GND") (layer "In1.Cu") (filled_polygon (layer "In1.Cu") (pts (xy -5 -5) (xy 50 -5) (xy 50 10) (xy -5 10))))
  (zone (net 5) (net_name "GND") (layer "In2.Cu") (filled_polygon (layer "In2.Cu") (pts (xy -5 -5) (xy 50 -5) (xy 50 10) (xy -5 10)))))
"#;

fn setup() -> (crate::board::Board, crate::extract::Analysis, Vec<Lane>) {
    let board = crate::board::load_pcb(PCB).unwrap();
    let mut params = crate::extract::Params::default();
    params.targets.insert("Default".into(), crate::extract::Target { z: 50.0, diff: false });
    let an = crate::analyze(&board, &params);
    let lanes = buses(&board, &an, &ComponentData::builtin()).into_iter().flat_map(|b| b.lanes).collect();
    (board, an, lanes)
}

fn rp() -> RfParams {
    RfParams { fmax: 20e9, npts: 200, ..Default::default() }
}

fn compact() -> ExportOpts {
    ExportOpts { model: Model::Compact, ..ExportOpts::default() }
}

fn ladder() -> ExportOpts {
    ExportOpts { model: Model::Ladder, ..ExportOpts::default() }
}

fn lane_of<'a>(board: &crate::board::Board, lanes: &'a [Lane], net: &str) -> &'a Lane {
    lanes.iter().find(|l| l.nets.iter().any(|&n| board.net_name(n) == net)).unwrap()
}

/// The exported single line follows kisi's own S-parameters of the same build: |S21| within
/// 0.05 dB up to 10 GHz, every S-parameter within 0.05 up to 10 GHz; the library has the lane
/// subcircuit with its ports, the series resistor and the via.
#[test]
fn single_line_follows_kisi() {
    let (board, an, lanes) = setup();
    let l = lanes.iter().find(|l| l.nets.iter().any(|&n| board.net_name(n) == "SIG")).expect("lane SIG");
    let field = HashMap::new();
    let lc = lane_circuit(&board, &an, &field, l, Some((50.0, false)), &rp(), l.rx_end == 0, &ComponentData::builtin(), 5e9, &ladder()).unwrap();
    assert!(!lc.pair && lc.ports == ["a", "b"]);
    for k in 0..lc.freq.len() {
        if lc.freq[k] > 10e9 {
            break;
        }
        let d21 = (db(lc.s_kisi[k][2]) - db(lc.s_export[k][2])).abs();
        let ds = lc.s_kisi[k].iter().zip(&lc.s_export[k]).map(|(a, b)| (*a - *b).abs()).fold(0.0, f64::max);
        assert!(d21 < 0.05 && ds < 0.05, "{} GHz: |dS21| {d21:.4} dB, |dS| {ds:.4}", lc.freq[k] / 1e9);
    }
    assert!(lc.lib.contains(&format!(".subckt {} a b ref", lc.subckt)) && lc.lib.contains(&format!(".ends {}", lc.subckt)));
    assert!(lc.lib.contains("R1 22") || lc.lib.contains("_R1"), "series resistor block");
    assert!(lc.lib.contains("_VIA1_"), "via block");
    assert!(lc.counts.sections > 10 && lc.counts.r > 0 && lc.counts.c > 0);
    // every subcircuit that is instantiated is defined
    let defined: std::collections::HashSet<&str> = lc.lib.lines().filter_map(|l| l.strip_prefix(".subckt ")).map(|l| l.split(' ').next().unwrap()).collect();
    for line in lc.lib.lines().filter(|l| l.starts_with('X')) {
        assert!(defined.contains(line.rsplit(' ').next().unwrap()), "{line}");
    }
}

/// The pair as a 4-port with modal blocks: the differential transmission within 0.1 dB of kisi's
/// up to 10 GHz; the mode converters are in the library.
#[test]
fn pair_follows_kisi() {
    let (board, an, lanes) = setup();
    let l = lanes.iter().find(|l| l.diff).expect("pair lane");
    let field = HashMap::new();
    let lc = lane_circuit(&board, &an, &field, l, Some((100.0, true)), &rp(), l.rx_end == 0, &ComponentData::builtin(), 5e9, &ladder()).unwrap();
    assert!(lc.pair && lc.ports.len() == 4, "{:?}", lc.notes);
    assert!(lc.fidelity.max_ds21_db_nyq < 0.1, "{:?}", lc.fidelity);
    assert!(lc.lib.contains(" ref 0.5\n") && lc.lib.contains(" -0.5\n"), "mode converters");
    assert!(lc.counts.sources > 0);
}

/// The files: library, benches, decks, Touchstone, KiCad project, README; the Touchstone file has
/// one data line per frequency (2-port) and the KiCad schematic is a 20230121 file.
#[test]
fn lane_files_are_complete() {
    let (board, an, lanes) = setup();
    let l = lanes.iter().find(|l| l.nets.iter().any(|&n| board.net_name(n) == "SIG")).unwrap();
    let field = HashMap::new();
    let lc = lane_circuit(&board, &an, &field, l, Some((50.0, false)), &rp(), l.rx_end == 0, &ComponentData::builtin(), 5e9, &ladder()).unwrap();
    let ends = files::BenchEnds { profile: "se50".into(), bitrate: 1e9, rise: 100e-12, swing: 0.4, tx: crate::eye::EndModel { r: 50.0, c_pf: 1.0, l_nh: 0.5 }, rx: crate::eye::EndModel { r: 50.0, c_pf: 1.0, l_nh: 0.5 }, notes: vec![] };
    let fs = files::lane_files(&lc, &ends, Formats { model: Model::Ladder, ..Formats::ALL }, "t.kicad_pcb", 20e9, 20.0, "");
    let names: Vec<&str> = fs.iter().map(|f| f.path.as_str()).collect();
    for want in ["SIG.lib", "bench_ngspice.cir", "sparam_ngspice.cir", "bench_xyce.cir", "sparam_xyce.cir", "SIG.s2p", "SIG.kicad_pro", "SIG.kicad_sch", "SIG_kicad.lib", "README.md"] {
        assert!(names.contains(&want), "{want} missing in {names:?}");
    }
    let ts = &fs.iter().find(|f| f.path == "SIG.s2p").unwrap().content;
    assert_eq!(ts.lines().filter(|l| !l.starts_with('!') && !l.starts_with('#')).count(), lc.freq.len());
    let sch = &fs.iter().find(|f| f.path == "SIG.kicad_sch").unwrap().content;
    assert!(sch.starts_with("(kicad_sch (version 20230121)") && sch.contains("\"Sim.Device\" \"SUBCKT\""));
    assert_eq!(sch.matches('(').count(), sch.matches(')').count());
    let bench = &fs.iter().find(|f| f.path == "bench_ngspice.cir").unwrap().content;
    assert!(bench.contains(".include \"SIG.lib\"") && bench.contains("PWL(") && bench.contains(".tran"));
}

/// The detailed KiCad sheet: one symbol per element of the lane subcircuit, each calling its own
/// wrapper; parts with their value as parameter; visible pins; the compact sheet on request.
#[test]
fn kicad_sheet_has_every_element() {
    let (board, an, lanes) = setup();
    let field = HashMap::new();
    for name in ["SIG", "D_P"] {
        let l = lanes.iter().find(|l| l.nets.iter().any(|&n| board.net_name(n) == name)).unwrap();
        let target = if l.diff { Some((100.0, true)) } else { Some((50.0, false)) };
        let lc = lane_circuit(&board, &an, &field, l, target, &rp(), l.rx_end == 0, &ComponentData::builtin(), 5e9, &ladder()).unwrap();
        assert!(!lc.top.is_empty() && super::kicad::can_detail(&lc), "{name}: no element list");
        let ends = files::BenchEnds { profile: "p".into(), bitrate: 1e9, rise: 100e-12, swing: 0.4, tx: crate::eye::EndModel { r: 5000.0, c_pf: 1.0, l_nh: 0.0 }, rx: crate::eye::EndModel { r: 50.0, c_pf: 1.0, l_nh: 0.0 }, notes: vec![] };
        let fs = files::lane_files(&lc, &ends, Formats { model: Model::Ladder, ..Formats::ALL }, "t.kicad_pcb", 20e9, 20.0, "");
        let get = |suffix: &str| fs.iter().find(|f| f.path.ends_with(suffix)).unwrap().content.clone();
        let (sch, blib) = (get(".kicad_sch"), get("_kicad.lib"));
        for t in &lc.top {
            assert!(sch.contains(&format!("\"Sim.Name\" \"{}_k\"", t.sub)), "{name}: {} not on the sheet", t.sub);
            assert!(blib.contains(&format!(".subckt {}_k ", t.sub)), "{name}: no wrapper for {}", t.sub);
        }
        // pins of parts visible: only power symbols hide theirs
        let hidden = sch.lines().filter(|l| l.trim_start().starts_with("(pin ") && l.contains(") hide")).count();
        assert_eq!(hidden, sch.matches("(pin power_").count(), "{name}: hidden pins");
        assert!(sch.contains("\"Value\" \"5k\"") && sch.contains("current-mode driver"), "{name}: transmitter shown as 5k with its note");
        if name == "SIG" {
            assert!(sch.contains("\"Sim.Params\" \"r=${VALUE}\"") && blib.contains("params: r=22"), "the 22 Ω part with its value as parameter");
        } else {
            assert!(sch.contains("kisi:CPL") || sch.contains("kisi:VIAPAIR"), "pair: a coupled element");
        }
        let compact = files::lane_files(&lc, &ends, Formats { kicad_compact: true, model: Model::Ladder, ..Formats::ALL }, "t.kicad_pcb", 20e9, 20.0, "");
        let csch = &compact.iter().find(|f| f.path.ends_with(".kicad_sch")).unwrap().content;
        assert!(csch.contains("kisi:LANE") && !csch.contains("kisi:TL\""));
    }
}

/// The LTspice files: the shared library with Rser=0 on every inductor, no ngspice constructs, .meas lines,
/// and the .net deck only for a single-ended lane.
#[test]
fn ltspice_files() {
    let (board, an, lanes) = setup();
    let field = HashMap::new();
    for name in ["SIG", "D_P"] {
        let l = lanes.iter().find(|l| l.nets.iter().any(|&n| board.net_name(n) == name)).unwrap();
        let target = if l.diff { Some((100.0, true)) } else { Some((50.0, false)) };
        let lc = lane_circuit(&board, &an, &field, l, target, &rp(), l.rx_end == 0, &ComponentData::builtin(), 5e9, &ladder()).unwrap();
        let ends = files::BenchEnds { profile: "p".into(), bitrate: 1e9, rise: 100e-12, swing: 0.4, tx: crate::eye::EndModel { r: 50.0, c_pf: 1.0, l_nh: 0.5 }, rx: crate::eye::EndModel { r: 50.0, c_pf: 1.0, l_nh: 0.5 }, notes: vec![] };
        let fs = files::lane_files(&lc, &ends, Formats { model: Model::Ladder, ..Formats::ALL }, "t.kicad_pcb", 20e9, 20.0, "");
        let get = |p: &str| fs.iter().find(|f| f.path == p).map(|f| f.content.clone());
        let (lib, llib, bench) = (get(&format!("{name}.lib")).unwrap(), get(&format!("{name}_ltspice.lib")).unwrap(), get("bench_ltspice.cir").unwrap());
        assert!(llib.is_ascii() && bench.is_ascii());
        let strip = |t: &str| t.lines().filter(|l| !l.starts_with('*')).map(|l| l.replace(" Rser=0 Rpar=1e30", "")).collect::<Vec<_>>();
        assert_eq!(strip(&lib), strip(&llib), "{name}: the LTspice library is the shared one");
        for l in llib.lines().chain(bench.lines()).filter(|l| l.starts_with('L')) {
            assert!(l.ends_with(" Rser=0 Rpar=1e30"), "{name}: {l}");
        }
        assert!(!bench.contains(".control") && !bench.contains("wrdata") && bench.contains(".meas TRAN t50 WHEN V(vrx)=0 RISE=1") && bench.contains("Bvrx vrx 0 V="));
        assert_eq!(get("sparam_ltspice.cir").is_some(), lc.ports.len() == 2, "{name}: .net only for a 2-port");
    }
}

#[test]
fn prbs_and_formats() {
    let p = prbs7();
    assert_eq!((p.len(), p.iter().filter(|&&b| b).count()), (127, 64));
    assert!(Formats::parse("spice,touchstone").is_ok_and(|f| f.spice && f.touchstone && !f.kicad));
    assert!(Formats::parse("bogus").is_err());
}

/// Renormalising to another reference and back gives the matrix again; a matched line stays
/// reflection-free when the reference equals its impedance.
#[test]
fn renormalize_round_trip() {
    let s = vec![Cx::new(0.1, 0.05), Cx::new(0.8, -0.3), Cx::new(0.8, -0.3), Cx::new(-0.05, 0.1)];
    let r = renormalize(&s, 2, 50.0, 42.5);
    let back = renormalize(&r, 2, 42.5, 50.0);
    for (a, b) in s.iter().zip(&back) {
        assert!((*a - *b).abs() < 1e-12);
    }
    // a 42.5 Ω matched load seen with 50 Ω: Γ = (42.5 − 50)/(42.5 + 50)
    let m = renormalize(&[Cx::new(0.0, 0.0)], 1, 42.5, 50.0);
    assert!((m[0].re - (42.5 - 50.0) / 92.5).abs() < 1e-12);
}

/// kisi's section matrices (`kisi_abcd`) multiply to the channel's cascade.
#[test]
fn kisi_sections_give_the_cascade() {
    let (board, an, lanes) = setup();
    let l = lanes.iter().find(|l| l.nets.iter().any(|&n| board.net_name(n) == "SIG")).unwrap();
    let field = HashMap::new();
    let rp = rp();
    let tb = crate::topology::lane_builds(&board, &an, &field, l, Some((50.0, false)), &rp, l.rx_end == 0, &ComponentData::builtin()).unwrap();
    let crate::topology::LaneBuilds::Single { b, .. } = tb.builds else { panic!("single") };
    let s = b.sections(Some((50.0, false)));
    let ch = b.finish(0, "x".into(), None, Some((50.0, false)));
    let k = 60;
    let m = (0..s.segs.len()).fold(Abcd::identity(), |m, i| m.mul(&kisi_abcd(&s, i, ch.freq[k], &rp, None)));
    assert!((m.a - ch.abcd[k].a).abs() < 1e-9 && (m.b - ch.abcd[k].b).abs() < 1e-6);
}

/// The compact model of the single line: as close to kisi's S-parameters as the ladder (|S21|
/// within 0.05 dB, every S-parameter within 0.05 up to 10 GHz), lossless LTRA lines in the
/// library, fewer elements than the ladder, passive from 1 kHz to 10 × f max.
#[test]
fn compact_single_line_follows_kisi() {
    let (board, an, lanes) = setup();
    let l = lane_of(&board, &lanes, "SIG");
    let field = HashMap::new();
    let run = |o: &ExportOpts| lane_circuit(&board, &an, &field, l, Some((50.0, false)), &rp(), l.rx_end == 0, &ComponentData::builtin(), 5e9, o).unwrap();
    let (lad, cmp) = (run(&ladder()), run(&compact()));
    assert_eq!((lad.model, cmp.model), (Model::Ladder, Model::Compact));
    for k in 0..cmp.freq.len() {
        if cmp.freq[k] > 10e9 {
            break;
        }
        let d21 = (db(cmp.s_kisi[k][2]) - db(cmp.s_export[k][2])).abs();
        let ds = cmp.s_kisi[k].iter().zip(&cmp.s_export[k]).map(|(a, b)| (*a - *b).abs()).fold(0.0, f64::max);
        assert!(d21 < 0.05 && ds < 0.05, "{} GHz: |dS21| {d21:.4} dB, |dS| {ds:.4}", cmp.freq[k] / 1e9);
    }
    assert!(cmp.counts.cells > 0 && lad.counts.cells == 0, "{:?}", cmp.counts);
    assert!(cmp.lib.contains("lossless-line cells (LC ladders of") && cmp.lib.contains("_cell a b ref"));
    assert!(cmp.counts.elements() < lad.counts.elements() / 2, "compact {} vs ladder {}", cmp.counts.elements(), lad.counts.elements());
    for lc in [&lad, &cmp] {
        assert!(lc.fidelity.circuit_max_sv <= 1.0 + 1e-9, "{:?}: largest singular value {} at {} GHz", lc.model, lc.fidelity.circuit_max_sv, lc.fidelity.circuit_max_sv_ghz);
    }
    // only R, L, C, sources and subcircuits
    assert!(cmp.lib.lines().filter(|x| !x.starts_with('*') && !x.starts_with('.') && !x.is_empty()).all(|x| "RLCEFVX".contains(x.chars().next().unwrap())));
}

/// The compact pair: modal blocks with cells per mode, Sdd21 within 0.1 dB of kisi's up to 1.5 ×
/// Nyquist, passive beyond the band, fewer elements than the ladder.
#[test]
fn compact_pair_follows_kisi() {
    let (board, an, lanes) = setup();
    let l = lanes.iter().find(|l| l.diff).unwrap();
    let field = HashMap::new();
    let run = |o: &ExportOpts| lane_circuit(&board, &an, &field, l, Some((100.0, true)), &rp(), l.rx_end == 0, &ComponentData::builtin(), 5e9, o).unwrap();
    let (lad, cmp) = (run(&ladder()), run(&compact()));
    assert!(cmp.pair && cmp.fidelity.max_ds21_db_nyq < 0.1, "{:?}", cmp.fidelity);
    assert!((cmp.fidelity.max_ds_nyq - lad.fidelity.max_ds_nyq).abs() < 0.01, "compact {:?} ladder {:?}", cmp.fidelity, lad.fidelity);
    assert!(cmp.lib.contains("_CPL1_even") && cmp.counts.cells > 0);
    assert!(cmp.counts.elements() < lad.counts.elements());
    assert!(cmp.fidelity.circuit_max_sv <= 1.0 + 1e-9 && lad.fidelity.circuit_max_sv <= 1.0 + 1e-9);
}

/// Consecutive sections of one line with nearly the same model are merged into one section in
/// the compact model (the ladder keeps them): a track whose width changes by 1 µm per segment.
#[test]
fn compact_merges_equal_sections() {
    let pcb = PCB.replace(r#"(segment (start 0 0) (end 30 0) (width 0.35) (layer "F.Cu") (net 1))"#, r#"(segment (start 0 0) (end 10 0) (width 0.35) (layer "F.Cu") (net 1)) (segment (start 10 0) (end 20 0) (width 0.351) (layer "F.Cu") (net 1)) (segment (start 20 0) (end 30 0) (width 0.352) (layer "F.Cu") (net 1))"#);
    assert_ne!(pcb, PCB);
    let board = crate::board::load_pcb(&pcb).unwrap();
    let mut params = crate::extract::Params::default();
    params.targets.insert("Default".into(), crate::extract::Target { z: 50.0, diff: false });
    let an = crate::analyze(&board, &params);
    let lanes: Vec<Lane> = buses(&board, &an, &ComponentData::builtin()).into_iter().flat_map(|b| b.lanes).collect();
    let l = lane_of(&board, &lanes, "SIG");
    let field = HashMap::new();
    let run = |o: &ExportOpts| lane_circuit(&board, &an, &field, l, Some((50.0, false)), &rp(), l.rx_end == 0, &ComponentData::builtin(), 5e9, o).unwrap();
    let (lad, cmp) = (run(&ladder()), run(&compact()));
    assert_eq!(lad.merged, 0);
    assert!(cmp.merged >= 1, "{} merges", cmp.merged);
    assert!(cmp.lib.contains("sections merged"));
    assert!(cmp.fidelity.max_ds_nyq < 0.02, "{:?}", cmp.fidelity);
}

/// Lossless-line cells without losses are the lossless line of n × td, and the star products
/// (the passivity check beyond the band) equal the cascade matrices where both are finite.
#[test]
fn cells_and_star_products() {
    use circuit::{lossless, pair_s, pair_t};
    let w = 2.0 * PI * 7.3e9;
    // 5 cells of 9 ps, 47 ohm, 40 sections per wavelength at 25 GHz: within 0.1 % of the line's phase
    let (zl, td) = (47.0, 9e-12);
    let el = El::Cells { z: Imp(vec![]), y: Adm(vec![]), l: zl * td, c: td / zl, n: 5, m: 9 };
    let (a, b) = (el.abcd(2.0 * PI * 25e9), lossless(zl, 2.0 * PI * 25e9 * 45e-12));
    let (sa, sb) = (s2(&a, zl), s2(&b, zl));
    assert!((sa[2].arg() - sb[2].arg()).abs() < 1e-3 * 2.0 * PI * 25e9 * 45e-12 && sa[0].abs() < 0.01, "{:?} {:?}", sa, sb);
    let fz = Foster { r0: 0.2, l: 0.0, terms: vec![(0.5, 1e-11)], tanks: vec![] };
    let fy = Debye { g0: 1e-6, c: 0.0, terms: vec![(5e3, 2e-14)], res: vec![] };
    let cells = El::Cells { z: Imp::from_foster(&fz), y: Adm::from_debye(&fy), l: 52.0 * 1e-11, c: 1e-11 / 52.0, n: 7, m: 10 };
    let ladder = El::Ladder { z: Imp(vec![Zel::R(0.1), Zel::L(2e-10)]), y: Adm(vec![Imp(vec![Zel::C(8e-14)])]), n: 30 };
    for e in [&cells, &ladder] {
        let (s, m) = (e.s(w, 50.0), s2(&e.abcd(w), 50.0));
        for (x, y) in s.iter().zip(&m) {
            assert!((*x - *y).abs() < 1e-9, "{x:?} {y:?}");
        }
    }
    // far above the ladder's cut-off its cascade matrix overflows; the star product stays passive
    let s = Chain(vec![ladder.clone(); 30]).s(2.0 * PI * 2e12, 50.0);
    assert!(s.iter().all(|x| x.re.is_finite() && x.im.is_finite()) && crate::rf::max_singular(&s, 2) <= 1.0 + 1e-9);
    let pels = vec![
        PairEl::Line(0, cells.clone()),
        PairEl::Modal { name: "M".into(), note: String::new(), even: Chain(vec![ladder.clone()]), odd: Chain(vec![cells.clone()]), tag: Tag::default() },
        PairEl::Line(1, ladder.clone()),
    ];
    let (a, b) = (pair_s(&pels, w, 45.0), s4(&pair_t(&pels, w), 45.0));
    for i in 0..4 {
        for j in 0..4 {
            assert!((a[i][j] - b[i * 4 + j]).abs() < 1e-9, "S{i}{j}: {:?} {:?}", a[i][j], b[i * 4 + j]);
        }
    }
}

/// The files of a compact export: the LTspice library is the shared one with Rser/Rpar on the
/// inductors, every deck is written, the README and the library header name the model.
#[test]
fn compact_files() {
    let (board, an, lanes) = setup();
    let field = HashMap::new();
    for name in ["SIG", "D_P"] {
        let l = lane_of(&board, &lanes, name);
        let target = if l.diff { Some((100.0, true)) } else { Some((50.0, false)) };
        let lc = lane_circuit(&board, &an, &field, l, target, &rp(), l.rx_end == 0, &ComponentData::builtin(), 5e9, &compact()).unwrap();
        let ends = files::BenchEnds { profile: "p".into(), bitrate: 1e9, rise: 100e-12, swing: 0.4, tx: crate::eye::EndModel { r: 50.0, c_pf: 1.0, l_nh: 0.5 }, rx: crate::eye::EndModel { r: 50.0, c_pf: 1.0, l_nh: 0.5 }, notes: vec![] };
        let fs = files::lane_files(&lc, &ends, Formats { model: Model::Compact, ..Formats::ALL }, "t.kicad_pcb", 20e9, 20.0, "");
        let get = |p: &str| fs.iter().find(|f| f.path == p).map(|f| f.content.clone());
        let (lib, llib, readme) = (get(&format!("{name}.lib")).unwrap(), get(&format!("{name}_ltspice.lib")).unwrap(), get("README.md").unwrap());
        let strip = |t: &str| t.lines().filter(|l| !l.starts_with('*')).map(|l| l.replace(" Rser=0 Rpar=1e30", "")).collect::<Vec<_>>();
        assert_eq!(strip(&lib), strip(&llib), "{name}");
        assert!(get("sparam_xyce.cir").is_some() && get("bench_xyce.cir").is_some() && get("sparam_ngspice.cir").is_some());
        assert!(lib.contains("* model (compact)") && readme.contains("compact model") && readme.contains("lossless-line cells"));
    }
    assert!(Model::parse("compact").is_ok_and(|m| m == Model::Compact) && Model::parse("x").is_err());
}

/// An open (high-impedance) receiver gets Gear integration in the ngspice bench and the KiCad
/// project (trapezoidal integration rings on the undamped lane); a terminated one keeps the default.
#[test]
fn bench_gear_for_open_receiver() {
    let (board, an, lanes) = setup();
    let l = lane_of(&board, &lanes, "SIG");
    let field = HashMap::new();
    let lc = lane_circuit(&board, &an, &field, l, Some((50.0, false)), &rp(), l.rx_end == 0, &ComponentData::builtin(), 5e9, &compact()).unwrap();
    for (r, gear) in [(50.0, false), (0.0, true)] {
        let ends = files::BenchEnds { profile: "p".into(), bitrate: 1e9, rise: 100e-12, swing: 0.4, tx: crate::eye::EndModel { r: 50.0, c_pf: 1.0, l_nh: 0.0 }, rx: crate::eye::EndModel { r, c_pf: 1.0, l_nh: 0.0 }, notes: vec![] };
        let fs = files::lane_files(&lc, &ends, Formats { model: Model::Compact, ..Formats::ALL }, "t.kicad_pcb", 20e9, 20.0, "");
        let get = |p: &str| fs.iter().find(|f| f.path.ends_with(p)).unwrap().content.clone();
        assert_eq!(get("bench_ngspice.cir").contains(".options method=gear"), gear);
        assert_eq!(get(".kicad_sch").contains(".options method=gear"), gear);
    }
}

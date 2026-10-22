//! Parser and checks on the fixtures in tests/data (sources and licences: tests/data/README.md).

use kisi_ibis::{derive, read, Corner, Severity};
use std::collections::BTreeSet;

fn load(rel: &str) -> kisi_ibis::IbisFile {
    let b = std::fs::read(format!("{}/tests/data/{rel}", env!("CARGO_MANIFEST_DIR"))).unwrap();
    read(&b, Some(rel.rsplit('/').next().unwrap()))
}

fn codes(f: &kisi_ibis::IbisFile, s: Severity) -> BTreeSet<&'static str> {
    f.issues.iter().filter(|i| i.severity == s).map(|i| i.code).collect()
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9 * b.abs().max(1e-30) + 1e-18
}

#[test]
fn kicad_v1_1() {
    let f = load("kicad/ibis_v1_1.ibs");
    assert!(codes(&f, Severity::Error).is_empty(), "{:#?}", f.issues);
    assert_eq!(f.header.comment_char, '#');
    assert_eq!(f.header.ibis_ver.as_deref(), Some("1.1"));
    assert_eq!(f.header.source.as_deref(), Some("This is the\nsource for the files"));
    let c = &f.components[0];
    assert_eq!((c.name.as_str(), c.manufacturer.as_deref()), ("Virtual", Some("KiCad")));
    let r = c.package.r.unwrap();
    assert!(close(r.typ.unwrap(), 50e-3) && close(r.min.unwrap(), 40e-3) && close(r.max.unwrap(), 60e-3));
    let l = c.package.l.unwrap();
    assert!(close(l.typ.unwrap(), 2e-9) && l.min.is_none() && l.max.is_none());
    assert_eq!(c.pins.len(), 4);
    let p1 = c.pin("1").unwrap();
    assert!(close(p1.r.unwrap(), 5e-3) && close(p1.l.unwrap(), 2e-9) && p1.c.is_none());
    assert_eq!(p1.reserved(), Some("POWER"));
    // NA in the pin: the [Package] value applies
    let p3 = c.pin("3").unwrap();
    let [_, (l3, from_pin), _] = c.pin_rlc(p3, Corner::Typ);
    assert!(close(l3.unwrap(), 2e-9) && !from_pin);
    let m = f.model("Input").unwrap();
    assert_eq!(m.model_type.as_deref(), Some("Input"));
    assert!(close(m.vinl.unwrap(), 0.8) && close(m.vinh.unwrap(), 2.0));
    assert!(close(m.c_comp.unwrap().min.unwrap(), 0.5e-12));
    let o = f.model("Output").unwrap();
    let ramp = o.ramp.as_ref().unwrap();
    let rise = ramp.rising.unwrap();
    assert!(close(rise.typ.unwrap().dv, 3.0) && close(rise.typ.unwrap().dt, 30e-9) && rise.max.is_none());
    assert!(close(ramp.falling.unwrap().get(Corner::Max).unwrap().dv, 3.2));
    assert_eq!(ramp.r_load(), 50.0);
    // a 10 Ω-ish driver: (2.5 V) / I_pulldown(2.5 V) from the 4-point table
    assert!(close(derive::ron_pulldown(o, Corner::Typ).unwrap(), 2.5 / 0.25));
}

#[test]
fn kicad_v5_1() {
    let f = load("kicad/ibis_v5_1.ibs");
    assert!(codes(&f, Severity::Error).is_empty(), "{:#?}", f.issues);
    let c = &f.components[0];
    assert_eq!(c.package_model.as_deref(), Some("BGA_package"));
    assert_eq!(c.pin_mapping.len(), 4);
    assert_eq!(c.pin_mapping[2].gnd_clamp_ref.as_deref(), Some("VSS"));
    assert!(c.pin_rlc_columns);
    let sel = f.selector("DQ").unwrap();
    assert_eq!(sel.entries.iter().map(|e| e.model.as_str()).collect::<Vec<_>>(), ["DQ40", "DQ40_ODT40"]);
    assert_eq!(sel.entries[1].description, "Data I/O with 40ohm ODT");
    let ac = f.model("AC40").unwrap();
    assert!(close(ac.c_comp_at(Corner::Typ).unwrap(), 1.345e-12 + 1.335e-12));
    assert_eq!(ac.temperature_range.unwrap(), kisi_ibis::Tmm::new(50.0, 100.0, 0.0));
    assert_eq!(ac.waveforms.len(), 4);
    let w = &ac.waveforms[1];
    assert!(w.rising && w.v_fixture_at(Corner::Max) == Some(1.42) && w.composite_current.as_ref().unwrap().len() == 5);
    assert_eq!(ac.extra_tables.len(), 2); // ISSO PD / PU
    let dq = f.model("DQ40").unwrap();
    assert_eq!(dq.model_spec.len(), 4);
    assert!(close(dq.spec("Vinh").unwrap().max.unwrap(), 0.872));
    let rs = dq.receiver_thresholds.iter().find(|r| r.name == "Reference_supply").unwrap();
    assert_eq!((rs.value, rs.text.as_str()), (None, "Pullup_ref"));
    assert!(close(dq.receiver_thresholds.iter().find(|r| r.name == "Tslew_ac").unwrap().value.unwrap(), 5e-9));
    assert_eq!(f.model("DQ40_ODT40").unwrap().add_submodel[0].mode, "Non-Driving");
    assert_eq!(f.submodels[0].submodel_type.as_deref(), Some("Dynamic_clamp"));
    let pm = f.package_model("BGA_package").unwrap();
    assert_eq!((pm.number_of_pins, pm.pins.len()), (Some(4), 4));
    assert!(close(pm.l.as_ref().unwrap().diagonal(0).unwrap(), 1.15e-9) && close(pm.r.as_ref().unwrap().diagonal(3).unwrap(), 0.225));
    let w = codes(&f, Severity::Warning);
    for c in ["unsupported-keyword", "na-typ", "waveform-endpoint", "ramp-waveform", "package-model"] {
        assert!(w.contains(c), "{c} missing in {w:?}");
    }
    let unsup: Vec<&str> = f.issues.iter().filter(|i| i.code == "unsupported-keyword").map(|i| i.message.as_str()).collect();
    assert!(unsup.iter().any(|m| m.starts_with("[Algorithmic Model]")) && unsup.iter().any(|m| m.starts_with("[Composite Current] (4×)")), "{unsup:?}");
}

#[test]
fn kicad_series() {
    let f = load("kicad/ibis_v4_1_r_series.ibs");
    assert!(codes(&f, Severity::Error).is_empty(), "{:#?}", f.issues);
    assert!(f.header.notes.as_deref().unwrap().lines().count() == 5);
    let c = &f.components[0];
    assert_eq!(c.series_pins.len(), 3);
    assert_eq!(c.series_pins[1].group.as_deref(), Some("group_a"));
    let r = f.model("series_r").unwrap();
    assert!(r.kind().series);
    assert_eq!(r.series.r.unwrap(), kisi_ibis::Tmm::new(25.0, 22.0, 28.0));
    assert!(r.series.l.is_some() && r.series.c.is_some() && r.series.rl.is_some() && r.series.rc.is_some() && r.series.lc.is_some());
    assert_eq!(r.series.current.as_ref().unwrap().len(), 3);
    let sw = f.model("series_sw").unwrap();
    assert!(sw.kind().series_switch && sw.series.is_empty());
    assert!(close(sw.off.as_ref().unwrap().r.unwrap().typ.unwrap(), 1e6));
    let mo = f.model("series_mosfet").unwrap();
    let (vds, t) = &mo.on.as_ref().unwrap().mosfet[0];
    assert_eq!((*vds, t.len()), (Some(1.0), 4));
    // [Series Pin Mapping] names a model that the file does not define
    let f = load("kicad/ibis_v4_1_series_pin_mapping.ibs");
    assert!(f.issues.iter().any(|i| i.code == "unknown-model" && i.message.contains("series_switch")));
}

#[test]
fn synthetic_full_is_clean() {
    let f = load("synthetic_full.ibs");
    let other: Vec<_> = f.issues.iter().filter(|i| !matches!(i.code, "unsupported-keyword" | "after-end")).collect();
    assert!(other.is_empty(), "{other:#?}");
    let unsup: BTreeSet<&str> = f.issues.iter().filter(|i| i.code == "unsupported-keyword").map(|i| i.message.split(':').next().unwrap()).collect();
    assert_eq!(unsup, BTreeSet::from(["[Add Submodel]", "[Define Package Model]", "[Driver Schedule]", "[Series Switch Groups]", "[Submodel]"]));
    // header: comment character changed and changed back
    let h = &f.header;
    assert_eq!(h.comment_char, '|');
    assert_eq!(h.disclaimer.as_deref(), Some("Synthetic data | not a real component."));
    assert_eq!(h.file_rev.as_deref(), Some("1.0"));
    assert_eq!(h.notes.as_deref().unwrap().lines().count(), 2);
    let c = &f.components[0];
    assert_eq!(c.pins.len(), 16);
    let pc = c.package.c.unwrap();
    assert!(close(pc.typ.unwrap(), 0.5e-12) && pc.min.is_none() && pc.max.is_none());
    assert_eq!(pc.get(Corner::Max), pc.typ);
    let p12 = c.pin("12").unwrap();
    assert!(p12.r.is_none() && close(p12.l.unwrap(), 2.1e-9) && p12.c.is_none());
    let p15 = c.pin("15").unwrap();
    assert!(p15.r.is_none() && p15.l.is_none() && p15.model == "IN_ODT");
    let d = &c.diff_pins[0];
    assert_eq!((d.pin.as_str(), d.inv_pin.as_str()), ("5", "6"));
    assert!(close(d.vdiff.unwrap(), 0.1) && d.tdelay.typ == Some(0.0) && d.tdelay.min.is_none());
    assert_eq!(c.pin_mapping.len(), 6);
    assert_eq!(c.pin_mapping[0].gnd_clamp_ref, None);
    assert_eq!(f.selector("IO_SEL").unwrap().entries[0].model, "IO_50");
    // analytic drivers: Ron per corner (min = slow = weak)
    let io = f.model("IO_50").unwrap();
    for (c, r) in [(Corner::Typ, 50.0), (Corner::Min, 60.0), (Corner::Max, 40.0)] {
        assert!((derive::ron_pulldown(io, c).unwrap() - r).abs() < 1e-6, "{c:?}");
        assert!((derive::ron_pullup(io, c).unwrap() - r).abs() < 1e-6, "{c:?}");
    }
    assert_eq!(derive::roff(io, Corner::Typ), Some(None)); // ideal clamps: no conduction at Vcc/2
    let odt = f.model("IN_ODT").unwrap();
    assert!((derive::roff(odt, Corner::Typ).unwrap().unwrap() - 50.0).abs() < 1e-6);
    assert!(close(odt.c_comp_gnd_clamp.unwrap().typ.unwrap(), 300e-15));
    assert_eq!(odt.params, vec![("Rref_diff".to_string(), 100.0), ("Cref_diff".to_string(), 0.0)]);
    assert_eq!(odt.model_type.as_deref(), Some("Input"));
    assert_eq!(io.temperature_range.unwrap(), kisi_ibis::Tmm::new(25.0, 85.0, -40.0));
    // [Ramp] equals the rising waveform into 50 Ω to ground: 60 % of 1.65 V
    let rise = io.ramp.as_ref().unwrap().rising.unwrap().typ.unwrap();
    assert!((rise.dv - 0.99).abs() < 1e-9);
    // waveform end points equal the DC solution into the fixture
    let w = io.waveforms.iter().find(|w| w.rising && w.v_fixture == Some(0.0)).unwrap();
    let e = derive::edge(w, Corner::Min).unwrap();
    let dc = derive::dc_level(io, Corner::Min, derive::State::High, 50.0, 0.0).unwrap();
    assert!((e.v_end - dc).abs() < 1e-5, "{} vs {dc}", e.v_end);
    assert!(close(f.model("TERM").unwrap().rpower.unwrap().typ.unwrap(), 1e12));
    assert!(close(f.model("SW").unwrap().off.as_ref().unwrap().r.unwrap().max.unwrap(), 1.1e6));
    let drv = f.model("DRV").unwrap();
    assert_eq!(drv.driver_schedule[0].model, "IO_34");
    assert!(close(drv.driver_schedule[0].rise_on.unwrap(), 0.1e-9) && drv.driver_schedule[0].rise_off.is_none());
    let pm = f.package_model("PKG_SYNTH").unwrap();
    let l = pm.l.as_ref().unwrap();
    // full matrix: the row of pin 4 continues on the next line
    assert_eq!(l.entries.len(), 6);
    assert!(l.entries.iter().any(|e| e.0 == 0 && e.1 == 2 && close(e.2, 0.1e-9)));
    let cm = pm.c.as_ref().unwrap();
    assert_eq!((cm.kind.as_str(), cm.bandwidth, cm.entries.len()), ("banded_matrix", Some(1), 5));
    assert!(close(pm.r.as_ref().unwrap().diagonal(1).unwrap(), 0.09));
}

#[test]
fn synthetic_errors_are_found() {
    let f = load("synthetic_errors.ibs");
    let all: BTreeSet<&str> = f.issues.iter().map(|i| i.code).collect();
    for c in [
        "stray-data", "version", "file-name", "missing-keyword", "comment-char", "unknown-keyword", "out-of-range", "na-typ", "duplicate-pin", "unknown-model", "columns", "unknown-pin",
        "unknown-parameter", "model-type", "non-monotonic", "bad-row", "coverage", "ramp-sign", "scale-suffix", "ramp", "waveform-direction", "missing-parameter",
        "unsupported-keyword", "threshold", "keyword-column", "bad-keyword", "duplicate-name", "missing-threshold", "misplaced-keyword", "missing-end",
    ] {
        assert!(all.contains(c), "'{c}' not reported; got {all:?}");
    }
    // every finding from the file itself carries its line
    let line = |code: &str, msg: &str| f.issues.iter().find(|i| i.code == code && i.message.contains(msg)).and_then(|i| i.line);
    assert_eq!(line("duplicate-pin", "pin 1"), Some(15));
    assert_eq!(line("bad-row", "'x'"), Some(41));
    assert_eq!(line("unknown-keyword", "Bogus"), Some(6));
    assert_eq!(line("missing-parameter", "R_fixture"), Some(55));
    // the pin row with too few columns is skipped, the others are kept
    assert_eq!(f.components[0].pins.len(), 6);
    // the bad keyword line did not create a table
    assert!(f.model("IN2").unwrap().pullup.is_none());
    // [Test Data] and the blocks were skipped: the model after them is read
    assert!(f.model("TERM1").is_some());
    assert!(!f.ended);
}

#[test]
fn bytes_line_endings_and_condensing() {
    // Latin-1 ©, CRLF line endings and a byte-order mark
    let mut b: Vec<u8> = b"\xef\xbb\xbf".to_vec();
    b.extend_from_slice(b"[IBIS Ver] 3.2\r\n[File Name] x.ibs\r\n[File Rev] 1\r\n[Copyright] \xa9 2026 Someone\r\n[END]\r\n");
    let utf8_bom = b.clone();
    let f = read(&utf8_bom[3..], None); // not valid UTF-8 because of 0xA9: Latin-1
    assert_eq!(f.header.copyright.as_deref(), Some("© 2026 Someone"));
    assert_eq!(f.header.ibis_ver.as_deref(), Some("3.2"));
    // CRLF gives the same result as LF
    let lf = std::fs::read(format!("{}/tests/data/synthetic_full.ibs", env!("CARGO_MANIFEST_DIR"))).unwrap();
    let crlf: Vec<u8> = String::from_utf8(lf.clone()).unwrap().replace('\n', "\r\n").into_bytes();
    let (a, c) = (read(&lf, None), read(&crlf, None));
    assert_eq!(serde_json_like(&a), serde_json_like(&c));
    // many findings of one kind at one place: five listed, then a note
    let mut t = String::from("[IBIS Ver] 5.1\n[File Name] y.ibs\n[File Rev] 1\n[Component] C\n[Manufacturer] M\n[Package]\nR_pkg 1 NA NA\nL_pkg 1n NA NA\nC_pkg 1p NA NA\n[Pin] signal_name model_name\n");
    for k in 0..12 {
        t.push_str(&format!("{k} S{k} M 1 2\n"));
    }
    t.push_str("[END]\n");
    let f = read(t.as_bytes(), None);
    let cols = f.issues.iter().filter(|i| i.code == "columns").count();
    assert_eq!(cols, kisi_ibis::KEEP_PER_CODE);
    assert!(f.issues.iter().any(|i| i.code == "more" && i.message.starts_with("7 more 'columns'")));
    assert!(f.issues.iter().any(|i| i.code == "unknown-model" && i.message.contains("12 pin(s)")));
}

/// A comparable digest of a parse result (line numbers are the same for LF and CRLF).
fn serde_json_like(f: &kisi_ibis::IbisFile) -> String {
    format!("{:?}|{:?}|{:?}|{:?}", f.header, f.components, f.models.iter().map(|m| (&m.name, &m.pulldown, m.waveforms.len())).collect::<Vec<_>>(), f.issues.iter().map(|i| (i.code, i.line)).collect::<Vec<_>>())
}

#[test]
fn keyword_spelling_and_comment_char() {
    let t = "[ibis ver] 4.2\n[FILE_NAME] z.ibs\n[File Rev] 1\n[Comment Char] #_char\n[Component] Z # comment\n[Manufacturer] m | not a comment\n[Package]\nR_pkg 1m NA NA\nL_pkg 1n NA NA\nC_pkg 1p NA NA\n[Pin] signal_name model_name\n| an old-style comment line\n1 A IN\n2 B IN\n|   another one\n[Diff Pin] inv_pin vdiff tdelay_typ tdelay_min tdelay_max\n1 2 NA NA NA NA\n[Model] IN\nModel_type Input\nVinl = 0.8\nVinh = 2\nC_comp 1p NA NA\n[Voltage_Range] 3.3 NA NA\n[gnd  clamp]\n-3.3 -1 NA NA\n0 0 NA NA\n3.3 0 NA NA\n[End]\n";
    let f = read(t.as_bytes(), Some("z.ibs"));
    assert_eq!(f.components[0].name, "Z");
    assert_eq!(f.components[0].manufacturer.as_deref(), Some("m | not a comment"));
    // lines starting with the old comment character: comments, reported once
    assert_eq!(f.components[0].pins.len(), 2);
    assert_eq!(f.issues.iter().filter(|i| i.code == "comment-char").count(), 1);
    // NA in [Diff Pin] is accepted
    let d = &f.components[0].diff_pins[0];
    assert!(d.vdiff.is_none() && d.tdelay.typ.is_none());
    let m = f.model("IN").unwrap();
    assert!(m.voltage_range.is_some() && m.gnd_clamp.as_ref().unwrap().len() == 3);
    assert!(f.ended);
    assert!(!f.issues.iter().any(|i| i.severity == Severity::Error), "{:#?}", f.issues);
}

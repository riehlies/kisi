use super::*;
use crate::analytic;
use crate::extract::Kind;
use std::f64::consts::PI;

fn close(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol * b.abs()
}

/// 50 Ω microstrip, w 3 mm, h 1.6 mm, t 35 µm, εr 4.5, tanδ 0.02, 1 GHz.
/// HFSS (js_2d_fields test suite): R 3.26 Ω/m, G 13.86 mS/m, L 307 nH/m, C 123 pF/m.
#[test]
fn microstrip_rlgc_vs_hfss() {
    let (z, eeff) = analytic::microstrip(3.0, 1.6, 0.035, 4.5);
    let rp = RfParams { roughness: Roughness::Smooth, ..Default::default() };
    let la = l_air(Kind::Microstrip, 3.0, 0.035, 1.6, 0.0, None);
    let dn = 1e-3;
    let lb = l_air(Kind::Microstrip, 3.0 - 2.0 * dn, 0.035 - 2.0 * dn, 1.6 + 2.0 * dn, 0.0, None);
    let k_ac = (PI * MU0 / rp.sigma).sqrt() / MU0 * (lb - la) / (dn * 1e-3);
    let m = LineModel {
        z0: z,
        eeff0: eeff,
        l_ext: z * eeff.sqrt() / C0,
        c0: eeff.sqrt() / (z * C0),
        r_dc: 1.0 / (rp.sigma * 3e-3 * 35e-6),
        k_ac,
        er: 4.5,
        tand: 0.02,
        q_fill: (eeff - 1.0) / 3.5,
        tand_eff0: None,
        ms_h: Some(1.6e-3),
        source: "formula",
    };
    let p = m.rlgc(1e9, &rp);
    eprintln!("R {:.3} Ω/m  L {:.1} nH/m  G {:.2} mS/m  C {:.1} pF/m", p.r, p.l * 1e9, p.g * 1e3, p.c * 1e12);
    assert!(close(p.r, 3.26, 0.15), "R {}", p.r);
    assert!(close(p.l, 307e-9, 0.03), "L {}", p.l);
    assert!(close(p.c, 123e-12, 0.03), "C {}", p.c);
    assert!(close(p.g, 13.86e-3, 0.10), "G {}", p.g);
}

#[test]
fn ds_model() {
    let ds = DjordjevicSarkar::fit(4.0, 0.02);
    let (e1, t1) = ds.at(1e9);
    assert!(close(e1, 4.0, 1e-9) && close(t1, 0.02, 1e-9));
    let (e10, t10) = ds.at(10e9);
    assert!(e10 < e1 && e10 > 3.8, "{e10}");
    assert!((t10 - 0.02).abs() < 0.003, "{t10}");
}

#[test]
fn matched_and_mismatched_lines() {
    // lossless 50 Ω line in a 50 Ω system: |S21| = 1, S11 = 0
    let w = 2.0 * PI * 5e9;
    let l = 50.0 * 2.0f64.sqrt() / C0;
    let c = 2.0f64.sqrt() / (50.0 * C0);
    let m = Abcd::line(0.0, l, 0.0, c, w, 0.1);
    let (s11, s21, _) = m.s_params(50.0);
    assert!(s11.abs() < 1e-9 && (s21.abs() - 1.0).abs() < 1e-9);
    // quarter-wave 70.7 Ω transformer between 50 Ω and 100 Ω: matched
    let z1 = 50.0 * 2.0f64.sqrt();
    let lq = z1 / C0;
    let cq = 1.0 / (z1 * C0);
    let lam4 = C0 / 5e9 / 4.0;
    let m = Abcd::line(0.0, lq, 0.0, cq, w, lam4).mul(&Abcd::shunt(Cx::new(1.0 / 100.0, 0.0)));
    // input impedance = A/C for an open termination after the shunt load
    let zin = m.a / m.c;
    assert!((zin.re - 50.0).abs() < 1e-6 && zin.im.abs() < 1e-6, "{zin:?}");
}

#[test]
fn tdr_of_step_in_impedance() {
    // 50 Ω reference, line of 75 Ω, 100 mm, open end far away (long)
    let n = 2000;
    let fmax = 50e9;
    let df = fmax / n as f64;
    let freq: Vec<f64> = (1..=n).map(|k| k as f64 * df).collect();
    let z = 75.0;
    let l = z * 2.0f64.sqrt() / C0;
    let c = 2.0f64.sqrt() / (z * C0);
    let s11: Vec<Cx> = freq
        .iter()
        .map(|&f| {
            let w = 2.0 * PI * f;
            // terminated in 75 Ω: no reflection from the end
            let m = Abcd::line(0.0, l, 0.0, c, w, 0.1).mul(&Abcd::shunt(Cx::new(1.0 / z, 0.0)));
            let zin = m.a / m.c;
            (zin - Cx::new(50.0, 0.0)) / (zin + Cx::new(50.0, 0.0))
        })
        .collect();
    let (t, zs, _) = tdr(&freq, &s11, 50.0, 20e-12, 400e-12, 0.0, 0.0);
    let k = t.iter().position(|&x| x > 200.0).unwrap();
    assert!((zs[k] - 75.0).abs() < 1.5, "Z at 200 ps = {}", zs[k]);
}

#[test]
fn coupling_symmetric() {
    // homogeneous stripline: no FEXT
    let (ze, zo) = analytic::cohn_coupled(0.15, 0.15, 0.4, 4.0);
    let (kb, kf) = coupling_from_modes(ze, zo, 4.0, 4.0);
    assert!(kb > 0.0 && kb < 0.2, "{kb}");
    assert!(kf.abs() < 1e-15);
    // microstrip: FEXT negative-coupling term non-zero
    let (ze, zo, ee, eo) = analytic::kirschning_jansen(0.2, 0.2, 0.1, 4.0);
    let (kb, kf) = coupling_from_modes(ze, zo, ee, eo);
    assert!(kb > 0.0 && kf.abs() > 0.0);
}

#[test]
fn net_path_ends_branches_and_stub() {
    // U1 ── 20 mm ── junction ── 10 mm ── J1, branch 3 mm to TP1, over a ground plane
    let pcb = r#"(kicad_pcb (version 20240108) (generator "pcbnew")
  (general (thickness 1.6))
  (layers (0 "F.Cu" signal) (1 "In1.Cu" signal) (31 "B.Cu" signal))
  (setup (stackup
    (layer "F.Cu" (type "copper") (thickness 0.035))
    (layer "dielectric 1" (type "prepreg") (thickness 0.2) (material "FR4") (epsilon_r 4.3) (loss_tangent 0.02))
    (layer "In1.Cu" (type "copper") (thickness 0.035))
    (layer "dielectric 2" (type "core") (thickness 1.2) (material "FR4") (epsilon_r 4.5) (loss_tangent 0.02))
    (layer "B.Cu" (type "copper") (thickness 0.035))))
  (net 0 "") (net 1 "SIG") (net 2 "GND")
  (footprint "U" (layer "F.Cu") (at 0 0) (property "Reference" "U1") (property "Value" "IC")
    (pad "3" smd rect (at 0 0) (size 0.6 0.6) (layers "F.Cu") (net 1 "SIG")))
  (footprint "J" (layer "F.Cu") (at 30 0) (property "Reference" "J1") (property "Value" "conn")
    (pad "1" smd rect (at 0 0) (size 0.6 0.6) (layers "F.Cu") (net 1 "SIG")))
  (footprint "TP" (layer "F.Cu") (at 20 3) (property "Reference" "TP1") (property "Value" "tp")
    (pad "1" smd rect (at 0 0) (size 0.6 0.6) (layers "F.Cu") (net 1 "SIG")))
  (segment (start 0 0) (end 20 0) (width 0.35) (layer "F.Cu") (net 1))
  (segment (start 20 0) (end 30 0) (width 0.35) (layer "F.Cu") (net 1))
  (segment (start 20 0) (end 20 3) (width 0.35) (layer "F.Cu") (net 1))
  (zone (net 2) (net_name "GND") (layer "In1.Cu") (filled_polygon (layer "In1.Cu") (pts (xy -5 -5) (xy 35 -5) (xy 35 8) (xy -5 8)))))
"#;
    let board = kisi_board::load_pcb(pcb).unwrap();
    let an = crate::analyze(&board, &crate::Params::default());
    let net = board.nets.iter().position(|n| n == "SIG").unwrap() as u32;
    let p = net_path_info(&board, &an, net, false, None).unwrap();
    assert_eq!((p.start.label.as_str(), p.end.label.as_str()), ("U1.3", "J1.1"), "{:?}", (&p.start, &p.end));
    assert!((p.length_mm - 30.0).abs() < 0.8, "{}", p.length_mm);
    assert_eq!(p.branches.len(), 1);
    let b = &p.branches[0];
    assert_eq!(b.end.label, "TP1.1");
    assert!((b.at_s - 20.0).abs() < 0.8 && (b.len_mm - 3.0).abs() < 0.8 && b.modelled, "{} {}", b.at_s, b.len_mm);
    // positions run monotonically from the start
    assert!(p.pieces.windows(2).all(|w| (w[0].s1 - w[1].s0).abs() < 1e-9));
    let f = net_path_info(&board, &an, net, true, None).unwrap();
    assert_eq!((f.start.label.as_str(), f.end.label.as_str()), ("J1.1", "U1.3"));
    assert!((f.branches[0].at_s - 10.0).abs() < 0.8);
    // a hint near J1 starts there
    assert_eq!(net_path_info(&board, &an, net, false, Some([29.0, 0.0])).unwrap().start.label, "J1.1");
    let ch = channel_dir(
        &board,
        &an,
        net,
        &std::collections::HashMap::new(),
        Some((50.0, false)),
        &RfParams::default(),
        false,
        None,
    )
    .unwrap();
    assert!(ch.segments.iter().any(|s| s.kind == "branch"), "stub in the cascade");
    assert!(ch.path.is_some());
}

fn geom_len(a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

#[test]
fn crosstalk_into_a_pair() {
    // pair D_P/D_N (0.15 / 0.15 mm, 30 mm) with an aggressor AGG 0.2 mm beside D_P
    let pcb = r#"(kicad_pcb (version 20240108) (generator "pcbnew")
  (general (thickness 1.6))
  (layers (0 "F.Cu" signal) (1 "In1.Cu" signal) (31 "B.Cu" signal))
  (setup (stackup
    (layer "F.Cu" (type "copper") (thickness 0.035))
    (layer "dielectric 1" (type "prepreg") (thickness 0.1) (material "FR4") (epsilon_r 4.0) (loss_tangent 0.02))
    (layer "In1.Cu" (type "copper") (thickness 0.035))
    (layer "dielectric 2" (type "core") (thickness 1.3) (material "FR4") (epsilon_r 4.5) (loss_tangent 0.02))
    (layer "B.Cu" (type "copper") (thickness 0.035))))
  (net 0 "") (net 1 "D_P") (net 2 "D_N") (net 3 "GND") (net 4 "AGG")
  (segment (start 0 -0.15) (end 30 -0.15) (width 0.15) (layer "F.Cu") (net 1))
  (segment (start 0 0.15) (end 30 0.15) (width 0.15) (layer "F.Cu") (net 2))
  (segment (start 0 -0.5) (end 30 -0.5) (width 0.15) (layer "F.Cu") (net 4))
  (zone (net 3) (net_name "GND") (layer "In1.Cu") (filled_polygon (layer "In1.Cu") (pts (xy -5 -5) (xy 35 -5) (xy 35 5) (xy -5 5)))))
"#;
    let board = kisi_board::load_pcb(pcb).unwrap();
    let an = crate::analyze(&board, &crate::Params::default());
    let id = |n: &str| board.nets.iter().position(|x| x == n).unwrap() as u32;
    let rp = RfParams::default();
    let field = std::collections::HashMap::new();
    let xp = crosstalk(&board, &an, id("D_P"), &field, &rp);
    let xn = crosstalk(&board, &an, id("D_N"), &field, &rp);
    let xa = crosstalk(&board, &an, id("AGG"), &field, &rp);
    assert!(xp.differential && xn.differential && !xa.differential);
    let (np, nn) = (xp.aggressors[0].next, xn.aggressors[0].next);
    eprintln!("diff NEXT into the pair {np:.4} / {nn:.4}, geometries {}", xp.geoms.len());
    for g in &xp.geoms {
        eprintln!("--- {}\n{}", g.0, g.1);
    }
    // same differential noise seen from either line; the far line cancels part of the near one
    assert!((np - nn).abs() < 0.05 * np, "{np} {nn}");
    // AGG as victim of the pair driven differentially: (k_P − k_N)/2
    let a = &xa.aggressors[0];
    assert!(a.pair.is_some(), "the pair counts as one differential aggressor");
    eprintln!("NEXT from the pair into AGG {:.4}", a.next);
    // single-line coupling D_P ↔ AGG (as a single-ended victim would see it) is larger
    let (ze, zo, ee, eo) = crate::analytic::kirschning_jansen(0.15, 0.2, 0.1, 4.0);
    let kb1 = coupling_from_modes(ze, zo, ee, eo).0;
    assert!(np < kb1 && np > 0.2 * kb1, "{np} vs single {kb1}");
    assert!(
        xp.geoms.len() == 2
            && xp
                .geoms
                .iter()
                .all(|g| g.1.contains("sig+") && g.1.contains("sig-") && g.1.matches("gnd x=").count() >= 2)
    );
    // common mode: the far line adds instead of cancelling, (k_P + k_N)/2 lies between
    // the differential noise and the single-line coupling
    let c = xp.aggressors[0].next_c;
    assert!(c > 0.5 * np && c < kb1, "common {c} differential {np} single {kb1}");
    // profile along the path: FEXT grows to the total at the far end, NEXT is flat where the
    // coupled length exceeds half the rise length; stretches for the board on both lines
    let xd = crosstalk_dir(&board, &an, id("D_P"), &field, &rp, false, None);
    let pr = xd.profile.as_ref().unwrap();
    let it = &pr.items[0];
    assert!((pr.length_mm - 30.0).abs() < 0.5 && it.fext.len() == pr.s_mm.len());
    let fe = *it.fext.last().unwrap();
    assert!((fe - xd.aggressors[0].fext).abs() < 0.02 * xd.aggressors[0].fext + 1e-6, "{fe} {}", xd.aggressors[0].fext);
    assert!(it.fext.windows(2).all(|w| w[1] >= w[0] - 1e-12));
    let mid = it.next[it.next.len() / 2];
    assert!((mid - np).abs() < 0.05 * np, "NEXT density {mid} vs {np}");
    assert!(
        !xd.segments.is_empty()
            && xd.segments.iter().all(|s| (s.aa[1] + 0.5).abs() < 1e-6 && (s.va[1] + 0.15).abs() < 1e-6)
    );
    assert!(xd.broadside.is_empty());
    // scan: the pair once, critical from 1 %, its coupling cross-sections for stage 2
    let sc = crosstalk_scan(&board, &an, &[id("D_P"), id("D_N"), id("AGG")], &field, &rp, XT_CRITICAL);
    assert_eq!(sc.rows.len(), 2);
    assert!(sc.rows.iter().all(|r| r.critical) && !sc.critical_geoms.is_empty());
    // board map: the critical stretches on both lines of the pair (and on AGG)
    assert!(sc.map.iter().all(|h| h.next.max(h.fext) >= XT_CRITICAL));
    for n in ["D_P", "D_N", "AGG"] {
        let len: f64 = sc.map.iter().filter(|h| h.net == id(n)).map(|h| geom_len(h.a, h.b)).sum();
        assert!(len > 25.0, "{n}: {len} mm marked");
    }
}

#[test]
fn crosstalk_broadside_detected() {
    // two tracks on adjacent signal layers (no plane between), one over the other for 20 mm
    let pcb = r#"(kicad_pcb (version 20240108) (generator "pcbnew")
  (general (thickness 1.6))
  (layers (0 "F.Cu" signal) (1 "In1.Cu" signal) (2 "In2.Cu" signal) (31 "B.Cu" signal))
  (setup (stackup
    (layer "F.Cu" (type "copper") (thickness 0.035))
    (layer "dielectric 1" (type "prepreg") (thickness 0.1) (material "FR4") (epsilon_r 4.0) (loss_tangent 0.02))
    (layer "In1.Cu" (type "copper") (thickness 0.035))
    (layer "dielectric 2" (type "core") (thickness 0.2) (material "FR4") (epsilon_r 4.0) (loss_tangent 0.02))
    (layer "In2.Cu" (type "copper") (thickness 0.035))
    (layer "dielectric 3" (type "prepreg") (thickness 0.1) (material "FR4") (epsilon_r 4.0) (loss_tangent 0.02))
    (layer "B.Cu" (type "copper") (thickness 0.035))))
  (net 0 "") (net 1 "A") (net 2 "B") (net 3 "GND")
  (segment (start 0 0) (end 30 0) (width 0.15) (layer "In1.Cu") (net 1))
  (segment (start 5 0.05) (end 25 0.05) (width 0.15) (layer "In2.Cu") (net 2))
  (zone (net 3) (net_name "GND") (layer "F.Cu") (filled_polygon (layer "F.Cu") (pts (xy -5 -5) (xy 35 -5) (xy 35 5) (xy -5 5))))
  (zone (net 3) (net_name "GND") (layer "B.Cu") (filled_polygon (layer "B.Cu") (pts (xy -5 -5) (xy 35 -5) (xy 35 5) (xy -5 5)))))
"#;
    let board = kisi_board::load_pcb(pcb).unwrap();
    let an = crate::analyze(&board, &crate::Params::default());
    let id = |n: &str| board.nets.iter().position(|x| x == n).unwrap() as u32;
    let x = crosstalk(&board, &an, id("A"), &std::collections::HashMap::new(), &RfParams::default());
    assert_eq!(x.broadside.len(), 1, "{:?}", x.broadside);
    let b = &x.broadside[0];
    assert!(b.net == id("B") && (b.overlap_mm - 20.0).abs() < 0.6 && (b.offset_mm - 0.05).abs() < 0.01, "{b:?}");
    assert!(x.aggressors.is_empty(), "broadside is not computed yet");
}

/// L1 → L4 through a via whose traces run over the antipads right at the via (no reference
/// there): the return path must still be found from the referenced trace before and after it
/// (GND In1 → GND In2 with a stitching via at 1 mm).
#[test]
fn via_return_path_across_antipad() {
    // plane with a round antipad at (10, 0), joined to the bottom edge by a cut (fractured polygon)
    let plane = |layer: &str| -> String {
        let mut pts = vec![(-5.0, 5.0), (10.0, 5.0)];
        for k in 0..=48 {
            let a = std::f64::consts::FRAC_PI_2 - 2.0 * PI * k as f64 / 48.0;
            pts.push((10.0 + 0.4 * a.cos(), 0.4 * a.sin()));
        }
        pts.extend([(10.0, 5.0), (25.0, 5.0), (25.0, -5.0), (-5.0, -5.0)]);
        let p: Vec<String> = pts.iter().map(|(x, y)| format!("(xy {x:.4} {y:.4})")).collect();
        format!(
            "(zone (net 2) (net_name \"GND\") (layer \"{layer}\") (filled_polygon (layer \"{layer}\") (pts {})))",
            p.join(" ")
        )
    };
    let pcb = format!(
        r#"(kicad_pcb (version 20240108) (generator "pcbnew")
  (general (thickness 1.6))
  (layers (0 "F.Cu" signal) (1 "In1.Cu" signal) (2 "In2.Cu" signal) (31 "B.Cu" signal))
  (setup (stackup
    (layer "F.Cu" (type "copper") (thickness 0.035))
    (layer "dielectric 1" (type "prepreg") (thickness 0.2) (material "FR4") (epsilon_r 4.3) (loss_tangent 0.02))
    (layer "In1.Cu" (type "copper") (thickness 0.035))
    (layer "dielectric 2" (type "core") (thickness 1.2) (material "FR4") (epsilon_r 4.5) (loss_tangent 0.02))
    (layer "In2.Cu" (type "copper") (thickness 0.035))
    (layer "dielectric 3" (type "prepreg") (thickness 0.2) (material "FR4") (epsilon_r 4.3) (loss_tangent 0.02))
    (layer "B.Cu" (type "copper") (thickness 0.035))))
  (net 0 "") (net 1 "SIG") (net 2 "GND")
  (footprint "U" (layer "F.Cu") (at 0 0) (property "Reference" "U1") (property "Value" "IC")
    (pad "1" smd rect (at 0 0) (size 0.4 0.4) (layers "F.Cu") (net 1 "SIG")))
  (footprint "J" (layer "B.Cu") (at 20 0) (property "Reference" "J1") (property "Value" "conn")
    (pad "1" smd rect (at 0 0) (size 0.4 0.4) (layers "B.Cu") (net 1 "SIG")))
  (segment (start 0 0) (end 10 0) (width 0.35) (layer "F.Cu") (net 1))
  (segment (start 10 0) (end 20 0) (width 0.35) (layer "B.Cu") (net 1))
  (via (at 10 0) (size 0.6) (drill 0.3) (layers "F.Cu" "B.Cu") (net 1))
  (via (at 10 1) (size 0.6) (drill 0.3) (layers "F.Cu" "B.Cu") (net 2))
  {}
  {})
"#,
        plane("In1.Cu"),
        plane("In2.Cu")
    );
    let board = kisi_board::load_pcb(&pcb).unwrap();
    let an = crate::analyze(&board, &crate::Params::default());
    let net = board.nets.iter().position(|n| n == "SIG").unwrap() as u32;
    let ch = channel(&board, &an, net, &std::collections::HashMap::new(), None, &RfParams::default()).unwrap();
    let v = &ch.via_info[0];
    assert_eq!(v.return_kind, "stitch", "{v:?}");
    assert!((v.return_mm.unwrap() - 1.0).abs() < 0.05, "{v:?}");
}

/// Line models for the passivity tests: thin and thick microstrip (formula), stripline, a
/// field-solved mode (its own loss tangent) and the fallback of sections without a plane.
fn sample_models() -> Vec<LineModel> {
    let ms = |w: f64, h: f64| {
        let (z, eeff) = analytic::microstrip(w, h, 0.035, 4.4);
        LineModel {
            z0: z,
            eeff0: eeff,
            l_ext: z * eeff.sqrt() / C0,
            c0: eeff.sqrt() / (z * C0),
            r_dc: 1.0 / (5.8e7 * w * 1e-3 * 35e-6),
            k_ac: 4e-4 * 0.3 / w,
            er: 4.4,
            tand: 0.02,
            q_fill: (eeff - 1.0) / 3.4,
            tand_eff0: None,
            ms_h: Some(h * 1e-3),
            source: "formula",
        }
    };
    let sl = LineModel {
        z0: 50.0,
        eeff0: 3.66,
        l_ext: 50.0 * 3.66f64.sqrt() / C0,
        c0: 3.66f64.sqrt() / (50.0 * C0),
        r_dc: 8.0,
        k_ac: 6e-4,
        er: 3.66,
        tand: 0.02,
        q_fill: 1.0,
        tand_eff0: None,
        ms_h: None,
        source: "formula",
    };
    let field = LineModel { tand_eff0: Some(0.017), source: "field", ..ms(0.2, 0.11) };
    vec![ms(0.2, 0.11), ms(3.0, 1.6), ms(0.3, 1.0), sl, field, crate::rf::Build::fallback(50.0)]
}

/// Series impedance and shunt admittance per metre are positive-real – Re ≥ 0 everywhere in the
/// right half of the s plane (they are analytic there by construction) –, and the RLGC at real
/// frequencies are their values at s = jω: passive and causal lines.
#[test]
fn line_models_positive_real() {
    for rough in [Roughness::Smooth, Roughness::Hvlp, Roughness::Standard] {
        let rp = RfParams { roughness: rough, ..Default::default() };
        for m in sample_models() {
            for &sr in &[1e-3, 1e6, 1e8, 1e9, 1e10, 1e11, 1e12, 1e13] {
                for e in -30..=26 {
                    let wi = if e < 0 { -(10f64.powf(-e as f64 / 2.0)) } else { 10f64.powf(e as f64 / 2.0) };
                    let s = Cx::new(sr, wi);
                    let (z, y) = (m.z_s(s, &rp), m.y_s(s, &rp));
                    assert!(z.re >= -1e-12 * z.abs(), "{} Z({s:?}) = {z:?}", m.source);
                    assert!(y.re >= -1e-12 * y.abs(), "{} Y({s:?}) = {y:?}", m.source);
                }
            }
            for f in [1e7, 1e8, 1e9, 1e10, 2.5e10] {
                let p = m.rlgc(f, &rp);
                let s = Cx::new(0.0, 2.0 * PI * f);
                let (z, y) = (m.z_s(s, &rp), m.y_s(s, &rp));
                assert!(
                    (z.re - p.r).abs() < 1e-9 * z.abs() && (z.im - s.im * p.l).abs() < 1e-9 * z.abs(),
                    "{} {f}",
                    m.source
                );
                assert!(
                    (y.re - p.g).abs() < 1e-9 * y.abs() && (y.im - s.im * p.c).abs() < 1e-9 * y.abs(),
                    "{} {f}",
                    m.source
                );
            }
        }
    }
}

/// The causal Huray factor's loss part is the Huray power factor; it adds inductance.
#[test]
fn causal_huray_is_huray() {
    for f in [1e8, 1e9, 1e10, 5e10] {
        let delta = 1.0 / (PI * f * MU0 * 5.8e7).sqrt();
        let (r, sr) = (1.0e-6, 1.0);
        let huray = 1.0 + 1.5 * sr / (1.0 + delta / r + delta * delta / (2.0 * r * r));
        assert!((Roughness::Standard.factor(f, 5.8e7) - huray).abs() < 1e-12, "{f}");
        let k = Roughness::Standard.k(f, 5.8e7);
        // the inductance part (Im of (1+j)K) exceeds smooth copper's
        assert!(k.re + k.im > 1.0, "{f} {k:?}");
    }
}

/// A 200 mm lossy microstrip and stripline (standard roughness, tanδ 0.02): the pulse response has
/// nothing before the wave can arrive – at the line's front velocity, from L_ext and the
/// capacitance at infinite frequency.
#[test]
fn lossy_line_causal() {
    let rp = RfParams { fmax: 100e9, ..Default::default() };
    let n = 4000;
    let freq: Vec<f64> = (1..=n).map(|k| k as f64 * rp.fmax / n as f64).collect();
    let len = 0.2;
    for m in &sample_models()[..4] {
        let h: Vec<Cx> = freq
            .iter()
            .map(|&f| {
                let p = m.rlgc(f, &rp);
                let zc = (Cx::new(p.r, 2.0 * PI * f * p.l) / Cx::new(p.g, 2.0 * PI * f * p.c)).sqrt().re;
                Abcd::line(p.r, p.l, p.g, p.c, 2.0 * PI * f, len).s_params(zc).1
            })
            .collect();
        let em = m.eps_model(&rp);
        let t_front = len * (m.l_ext * m.c0 / m.eeff0 * (em.a - em.delta)).sqrt();
        // the pulse response (a step less its copy two rises later: free of the DC point's ramp)
        let rise = Step::window_rise(rp.fmax) * 3f64.sqrt();
        let st = Step::new(&freq, &h, None, rise);
        let p = |t: f64| st.at(t) - st.at(t - 2.0 * rise);
        let (mut pre, mut peak) = (0.0f64, 0.0f64);
        for i in 0..4000 {
            let t = -10e-9 + i as f64 * 4e-12;
            if t < t_front - 2.0 * rise {
                pre = pre.max(p(t).abs())
            } else {
                peak = peak.max(p(t).abs())
            }
        }
        assert!(pre < 1e-5 * peak, "{}: {pre} of {peak}", m.source);
    }
}

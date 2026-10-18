use crate::analytic;
use crate::extract::Kind;
use std::f64::consts::PI;
use super::*;

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


//! Validation of the thermal solver against analytic cases and JEDEC θJA values
//! (docs/research/thermal.md, section 7).

use super::*;
use crate::pi::LayerGrid;

/// A rectangular board `w` × `h` mm with the given copper thicknesses (µm) and dielectrics (mm),
/// full copper planes (net GND) on the layers in `planes`, and `extra` S-expressions.
fn board_text(w: f64, h: f64, cu_um: &[f64], diel_mm: &[f64], planes: &[usize], extra: &str) -> String {
    let n = cu_um.len();
    let name = |i: usize| {
        if i == 0 {
            "F.Cu".to_string()
        } else if i == n - 1 {
            "B.Cu".to_string()
        } else {
            format!("In{i}.Cu")
        }
    };
    let ord = |i: usize| {
        if i == 0 {
            0
        } else if i == n - 1 {
            2
        } else {
            2 + 2 * i
        }
    };
    let layers: String = (0..n).map(|i| format!("({} \"{}\" signal)", ord(i), name(i))).collect::<Vec<_>>().join(" ");
    let mut stack = String::new();
    for i in 0..n {
        stack += &format!("(layer \"{}\" (type \"copper\") (thickness {}))", name(i), cu_um[i] * 1e-3);
        if i + 1 < n {
            stack += &format!("(layer \"dielectric {}\" (type \"core\") (thickness {}) (material \"FR4\") (epsilon_r 4.5) (loss_tangent 0.02))", i + 1, diel_mm[i]);
        }
    }
    let zones: String = planes
        .iter()
        .map(|&l| format!("(zone (net 1) (net_name \"GND\") (layer \"{0}\") (filled_polygon (layer \"{0}\") (pts (xy 0 0) (xy {w} 0) (xy {w} {h}) (xy 0 {h}))))", name(l)))
        .collect();
    format!(
        "(kicad_pcb (version 20241229) (generator \"pcbnew\") (layers {layers}) (setup (stackup {stack}))
  (net 0 \"\") (net 1 \"GND\") (net 2 \"SIG\")
  (gr_rect (start 0 0) (end {w} {h}) (stroke (width 0.05)) (layer \"Edge.Cuts\"))
  {zones}
  {extra})"
    )
}

/// A DC result whose only content is a copper loss per cell (W) on one layer.
fn heat(layer: u16, x0: f64, y0: f64, cell: f64, nx: usize, ny: usize, loss: Vec<f32>) -> PiResult {
    PiResult {
        net: "HEAT".into(),
        voltage: 1.0,
        ground: false,
        cell,
        layers: vec![LayerGrid { layer, x0, y0, nx, ny, offset: 0 }],
        loads: vec![],
        sources: vec![],
        vias: vec![],
        nodes: 0,
        v_min: None,
        max_drop_mv: None,
        max_j: 0.0,
        max_j_at: None,
        power_loss_w: loss.iter().map(|&x| x as f64).sum(),
        total_load: 0.0,
        warnings: vec![],
        v: vec![],
        j: vec![],
        loss,
    }
}

fn mask_r() -> f64 {
    data().solder_mask.thickness_mm * 1e-3 / data().solder_mask.k_w_mk
}

/// Mean temperature of all layers at a point.
fn t_mean(r: &ThermalResult, p: P) -> f64 {
    (0..r.layers.len()).map(|l| r.temp_at(l, p)).sum::<f64>() / r.layers.len() as f64
}

/// 1D slab: uniform heat on the top layer of a board with planes on every layer. Laterally
/// uniform, so the board is a ladder: top node to the air, through every dielectric to the bottom
/// node and the air. Exact up to the solver's precision; checks through-plane conduction, the
/// surface coefficient with the mask in series and the energy balance.
#[test]
fn slab_1d() {
    for (cu, diel) in
        [(vec![35.0, 35.0], vec![1.5]), (vec![35.0, 17.5, 17.5, 17.5, 17.5, 35.0], vec![0.11, 0.1, 0.9, 0.11, 0.1])]
    {
        let n = cu.len();
        let b = crate::load_pcb(&board_text(60.0, 40.0, &cu, &diel, &(0..n).collect::<Vec<_>>(), "")).unwrap();
        let (h, q_area) = (12.0, 150.0); // W/m²K per side, W/m² on the top layer
        let cell = 0.5;
        let (nx, ny) = (120, 80);
        let w_cell = q_area * (cell * 1e-3f64).powi(2);
        let r = heat(0, 0.0, 0.0, cell, nx, ny, vec![w_cell as f32; nx * ny]);
        let prm = ThermalParams { h: Some(h), ambient_c: Some(25.0), cell: Some(1.0), ..Default::default() };
        let t = solve(&b, &[&r], &[], &prm).unwrap();
        // analytic: top node conductance to air h', bottom path Σ h_d/k_z + 1/h'
        let hp = 1.0 / (1.0 / h + mask_r());
        let r_bottom: f64 = diel.iter().map(|d| d * 1e-3 / data().laminate.k_through_w_mk).sum::<f64>() + 1.0 / hp;
        let rise_top = q_area / (hp + 1.0 / r_bottom);
        let got = t.temp_at(0, [30.0, 20.0]) - 25.0;
        eprintln!("slab {n} layers: top rise {got:.5} K, analytic {rise_top:.5} K");
        assert!((got - rise_top).abs() < 1e-6 * rise_top, "{got} vs {rise_top}");
        // every cell the same
        assert!((t.t_max_c - 25.0 - rise_top).abs() < 1e-6 * rise_top);
        assert!(
            t.balance.error_rel < 1e-8 && (t.balance.in_w - q_area * 60e-3 * 40e-3).abs() < 1e-6,
            "{:?}",
            t.balance
        );
        assert_eq!(t.balance.radiation_w, 0.0);
    }
}

/// Infinite fin: a 400 × 10 mm strip with two copper planes, heat across the width at the
/// middle. θ(x) = P/(2·K·w·m)·exp(−m|x|), m = √(2h'/K), K the sheet conductance of the strip.
#[test]
fn fin_strip() {
    let (len, wid, diel) = (400.0, 10.0, 0.2);
    let b = crate::load_pcb(&board_text(len, wid, &[35.0, 35.0], &[diel], &[0, 1], "")).unwrap();
    let h = 10.0;
    let cell = 1.0;
    let (nx, ny) = (400usize, 10usize);
    let mut loss = vec![0f32; nx * ny];
    for j in 0..ny {
        loss[j * nx + 200] = 0.1; // 1 W in the column at x = 200…201 mm
    }
    let r = heat(0, 0.0, 0.0, cell, nx, ny, loss);
    let t = solve(
        &b,
        &[&r],
        &[],
        &ThermalParams { h: Some(h), ambient_c: Some(20.0), cell: Some(cell), ..Default::default() },
    )
    .unwrap();
    let d = data();
    let k_sheet = 2.0 * d.copper.k_w_mk * 35e-6 + d.laminate.k_inplane_w_mk * diel * 1e-3;
    let hp = 1.0 / (1.0 / h + mask_r());
    let m = (2.0 * hp / k_sheet).sqrt();
    let theta0 = 1.0 / (2.0 * k_sheet * wid * 1e-3 * m);
    let mut worst: f64 = 0.0;
    for dx in [10.0, 20.0, 40.0, 80.0] {
        let x = 200.5 + dx;
        let want = theta0 * (-m * dx * 1e-3).exp();
        let got = t_mean(&t, [x, 5.0]) - 20.0;
        let err = (got - want) / want;
        worst = worst.max(err.abs());
        eprintln!("fin: x = {dx:>4} mm: {got:.4} K, analytic {want:.4} K ({:+.2} %)", err * 100.0);
    }
    assert!(worst < 0.01, "fin: worst error {:.2} %", worst * 100.0);
    assert!(t.balance.error_rel < 1e-8);
}

// Bessel functions (Abramowitz & Stegun 9.8.1, 9.8.5, 9.8.6)
fn bessel_i0(x: f64) -> f64 {
    let t = (x / 3.75).powi(2);
    1.0 + t * (3.5156229 + t * (3.0899424 + t * (1.2067492 + t * (0.2659732 + t * (0.0360768 + t * 0.0045813)))))
}
fn bessel_k0(x: f64) -> f64 {
    if x <= 2.0 {
        let t = (x / 2.0).powi(2);
        -(x / 2.0).ln() * bessel_i0(x) - 0.57721566
            + t * (0.42278420
                + t * (0.23069756 + t * (0.03488590 + t * (0.00262698 + t * (0.00010750 + t * 0.0000074)))))
    } else {
        let y = 2.0 / x;
        (-x).exp() / x.sqrt()
            * (1.25331414
                + y * (-0.07832358
                    + y * (0.02189568 + y * (-0.01062446 + y * (0.00587872 + y * (-0.00251540 + y * 0.00053208))))))
    }
}

/// Radial spreading in a plane with convection on both sides: a 4 × 4 mm source in the middle
/// of a 300 × 300 mm board with two planes. θ(r) = P/(2πK)·K0(mr), plus the images of the
/// adiabatic board edges.
#[test]
fn radial_spreading() {
    let (size, diel, cell) = (300.0, 0.2, 1.5);
    let b = crate::load_pcb(&board_text(size, size, &[35.0, 35.0], &[diel], &[0, 1], "")).unwrap();
    let h = 10.0;
    let rc = 0.5;
    let n = (size / rc) as usize;
    let mut loss = vec![0f32; n * n];
    let (c0, c1) = ((148.0 / rc) as usize, (152.0 / rc) as usize);
    let cnt = ((c1 - c0) * (c1 - c0)) as f32;
    for j in c0..c1 {
        for i in c0..c1 {
            loss[j * n + i] = 2.0 / cnt; // 2 W
        }
    }
    let r = heat(0, 0.0, 0.0, rc, n, n, loss);
    let t = solve(
        &b,
        &[&r],
        &[],
        &ThermalParams { h: Some(h), ambient_c: Some(20.0), cell: Some(cell), ..Default::default() },
    )
    .unwrap();
    let d = data();
    let k_sheet = 2.0 * d.copper.k_w_mk * 35e-6 + d.laminate.k_inplane_w_mk * diel * 1e-3;
    let hp = 1.0 / (1.0 / h + mask_r());
    let m = (2.0 * hp / k_sheet).sqrt();
    let theta = |x: f64, y: f64| -> f64 {
        // source at the centre (150, 150) and its images in the edges x, y ∈ {0, 300}
        let mut s = 0.0;
        for i in -3i32..=3 {
            for j in -3i32..=3 {
                let (sx, sy) = (150.0 + i as f64 * size, 150.0 + j as f64 * size);
                let rr = ((x - sx).hypot(y - sy) * 1e-3).max(1e-6);
                s += bessel_k0(m * rr);
            }
        }
        2.0 / (2.0 * std::f64::consts::PI * k_sheet) * s
    };
    let mut worst: f64 = 0.0;
    for dr in [12.0, 24.0, 48.0, 96.0] {
        // the centre of the thermal cell at that distance (on the axis through the source)
        let p = [((150.0 + dr) / cell).floor() * cell + cell / 2.0, 150.75];
        let want = theta(p[0], p[1]);
        let got = t_mean(&t, p) - 20.0;
        let err = (got - want) / want;
        worst = worst.max(err.abs());
        eprintln!("radial: r = {dr:>4} mm: {got:.4} K, analytic {want:.4} K ({:+.2} %)", err * 100.0);
    }
    assert!(worst < 0.01, "radial: worst error {:.2} %", worst * 100.0);
    assert!(t.balance.error_rel < 1e-8);
}

/// Natural convection and radiation: the coefficients at typical conditions, and the energy
/// balance with estimated coefficients (in = convection + radiation).
#[test]
fn convection_coefficients() {
    // 100 × 100 mm, 20 K above 25 °C
    let (up, down, rad) = surface_coefficients(&ThermalParams::default(), 0.01, 0.4, 0.1, 20.0, 298.15);
    eprintln!("h up {up:.2}, down {down:.2}, rad {rad:.2} W/m²K");
    assert!((6.5..8.0).contains(&up) && (3.0..4.5).contains(&down) && (5.5..6.5).contains(&rad));
    let (v, _, _) =
        surface_coefficients(&ThermalParams { vertical: true, ..Default::default() }, 0.01, 0.4, 0.1, 20.0, 298.15);
    assert!((4.0..6.5).contains(&v), "vertical {v}");
    let (f, _, _) =
        surface_coefficients(&ThermalParams { airflow_m_s: 1.0, ..Default::default() }, 0.01, 0.4, 0.1, 20.0, 298.15);
    assert!((11.0..14.0).contains(&f), "1 m/s {f}");
    // estimated coefficients: the balance closes and the mean rise matches the one h was taken at
    let b = crate::load_pcb(&board_text(100.0, 100.0, &[35.0, 35.0], &[1.5], &[0, 1], "")).unwrap();
    let r = heat(0, 0.0, 0.0, 1.0, 100, 100, vec![2e-4; 10000]); // 2 W
    let t = solve(&b, &[&r], &[], &ThermalParams::default()).unwrap();
    eprintln!("balance {:?}, cooling {:?}", t.balance, t.cooling);
    assert!(t.balance.error_rel < 1e-8 && t.balance.radiation_w > 0.3 * t.balance.in_w);
    let mean = (t.layers[0].t_mean_c - t.ambient_c + t.layers[1].t_mean_c - t.ambient_c) / 2.0;
    assert!((mean - t.cooling.dt_mean_k).abs() < 0.05 * mean, "{mean} vs {}", t.cooling.dt_mean_k);
}

#[test]
fn pi_thermal_field() {
    let f = parse_pi_thermal("theta_jb=17.4 theta_jc=53.6K/W tj_max=125°C").unwrap();
    assert_eq!((f.theta_jb, f.theta_jc, f.tj_max, f.psi_jb), (Some(17.4), Some(53.6), Some(125.0), None));
    assert_eq!(parse_pi_thermal("psi_jb=11.9, p=500mW").unwrap().p, Some(0.5));
    assert_eq!(parse_pi_thermal("p=0").unwrap().p, Some(0.0));
    assert_eq!(parse_pi_thermal("theta_ja=45C/W").unwrap().theta_ja, Some(45.0));
    assert!(parse_pi_thermal("theta_jb=-3").is_err());
    assert!(parse_pi_thermal("rja=3").is_err());
    assert!(parse_pi_thermal("theta_jb").is_err());
    assert!(parse_pi_thermal("p=2A").is_err());
}

// --- parts ---------------------------------------------------------------------------------

/// Footprint of a QFN-16 3 × 3 mm with a 1.7 mm exposed pad (4 vias to In1), SOIC-8 or SOT-23-5
/// at (cx, cy), 0.25 mm traces of 25 mm from every lead (JESD51-7 style), and a PI_THERMAL field.
fn part(kind: &str, cx: f64, cy: f64, field: &str) -> String {
    let mut pads = String::new();
    let mut traces = String::new();
    let mut vias = String::new();
    let lead = |pads: &mut String, traces: &mut String, n: usize, x: f64, y: f64, sx: f64, sy: f64, dir: [f64; 2]| {
        pads.push_str(&format!(
            "(pad \"{n}\" smd rect (at {x} {y}) (size {sx} {sy}) (layers \"F.Cu\") (net 2 \"SIG\"))"
        ));
        let (ax, ay) = (cx + x, cy + y);
        traces.push_str(&format!(
            "(segment (start {ax} {ay}) (end {} {}) (width 0.25) (layer \"F.Cu\") (net 2))",
            ax + 25.0 * dir[0],
            ay + 25.0 * dir[1]
        ));
    };
    let lib = match kind {
        "qfn" => {
            let mut n = 1;
            for k in 0..4 {
                let o = -0.75 + 0.5 * k as f64;
                lead(&mut pads, &mut traces, n, -1.45, o, 0.6, 0.25, [-1.0, 0.0]);
                lead(&mut pads, &mut traces, n + 1, 1.45, o, 0.6, 0.25, [1.0, 0.0]);
                lead(&mut pads, &mut traces, n + 2, o, -1.45, 0.25, 0.6, [0.0, -1.0]);
                lead(&mut pads, &mut traces, n + 3, o, 1.45, 0.25, 0.6, [0.0, 1.0]);
                n += 4;
            }
            pads.push_str("(pad \"17\" smd rect (at 0 0) (size 1.7 1.7) (layers \"F.Cu\") (net 1 \"GND\"))");
            for (x, y) in [(-0.45, -0.45), (0.45, -0.45), (-0.45, 0.45), (0.45, 0.45)] {
                vias.push_str(&format!(
                    "(via (at {} {}) (size 0.6) (drill 0.3) (layers \"F.Cu\" \"B.Cu\") (net 1))",
                    cx + x,
                    cy + y
                ));
            }
            "Package_DFN_QFN:QFN-16-1EP_3x3mm_P0.5mm_EP1.7x1.7mm"
        }
        "soic" => {
            for k in 0..4 {
                let y = -1.905 + 1.27 * k as f64;
                lead(&mut pads, &mut traces, k + 1, -2.7, y, 1.55, 0.6, [-1.0, 0.0]);
                lead(&mut pads, &mut traces, 8 - k, 2.7, y, 1.55, 0.6, [1.0, 0.0]);
            }
            "Package_SO:SOIC-8_3.9x4.9mm_P1.27mm"
        }
        _ => {
            for (k, x) in [-0.95, 0.0, 0.95].iter().enumerate() {
                lead(&mut pads, &mut traces, k + 1, *x, 1.1, 0.6, 1.1, [0.0, 1.0]);
            }
            for (k, x) in [-0.95, 0.95].iter().enumerate() {
                lead(&mut pads, &mut traces, k + 4, *x, -1.1, 0.6, 1.1, [0.0, -1.0]);
            }
            "Package_TO_SOT_SMD:SOT-23-5"
        }
    };
    format!("(footprint \"{lib}\" (layer \"F.Cu\") (at {cx} {cy}) (property \"Reference\" \"U1\") (property \"Value\" \"{kind}\") (property \"PI_THERMAL\" \"{field}\") {pads}) {traces} {vias}")
}

/// The JESD51-7 high-conductivity test board (76.2 × 114.3 mm, 1.6 mm, 2 oz outer layers with
/// the traces, two 1 oz planes) in still air: θJA from kisi with the datasheet's θJB and θJC,
/// against the datasheet's θJA (TI, simulated on that board). A plausibility check of the board
/// model, the convection and the two-resistor coupling; the JEDEC details (via count, trace
/// pattern, the enclosure) are approximated.
#[test]
fn jedec_theta_ja() {
    let cases = [
        // (kind, θJB, θJC, θJA datasheet, P) – TPS6215x RGT, LM358 D, TLV704 DBV (new chip)
        ("qfn", 17.4, 53.6, 45.0, 0.5),
        ("soic", 67.9, 66.9, 124.7, 0.2),
        ("sot", 76.7, 68.7, 170.8, 0.15),
    ];
    for (kind, jb, jc, ja, p) in cases {
        let (w, h) = (76.2, 114.3);
        let extra = part(kind, w / 2.0, h / 2.0, &format!("theta_jb={jb} theta_jc={jc} tj_max=125 p={p}"));
        let b =
            crate::load_pcb(&board_text(w, h, &[70.0, 35.0, 35.0, 70.0], &[0.3, 0.86, 0.3], &[1, 2], &extra)).unwrap();
        let t = solve(&b, &[], &[], &ThermalParams { ambient_c: Some(25.0), ..Default::default() }).unwrap();
        let u = &t.parts[0];
        let theta = (u.tj_c - 25.0) / p;
        eprintln!(
            "JEDEC {kind}: θJA kisi {theta:.1} K/W vs datasheet {ja} K/W ({:+.0} %), board under the part {:.1} °C, {:.0} % into the board, h top {:.1} bottom {:.1} W/m²K, cell {} mm",
            (theta - ja) / ja * 100.0,
            u.t_board_c,
            u.to_board.unwrap() * 100.0,
            t.cooling.h_top,
            t.cooling.h_bottom,
            t.cell
        );
        assert_eq!((u.model.as_str(), u.estimate, u.w_source.as_str()), ("theta_jb", false, "PI_THERMAL p"));
        assert!((theta - ja).abs() < 0.35 * ja, "{kind}: θJA {theta:.1} vs {ja}");
        assert!(t.balance.error_rel < 1e-8);
    }
}

/// Package defaults by footprint name and pad geometry.
#[test]
fn package_families() {
    let names = [
        ("qfn", "Package_DFN_QFN:QFN-16-1EP_3x3mm_P0.5mm_EP1.7x1.7mm"),
        ("so", "Package_SO:SOIC-8_3.9x4.9mm_P1.27mm"),
        ("sot", "Package_TO_SOT_SMD:SOT-23-5"),
    ];
    for (fam, lib) in names {
        let kind = match fam {
            "so" => "soic",
            k => k,
        };
        let extra = part(kind, 20.0, 20.0, "p=0.1");
        let b = crate::load_pcb(&board_text(40.0, 40.0, &[35.0, 35.0], &[1.5], &[], &extra)).unwrap();
        let (f, jb, jc, area, _) = package_defaults(&b, 0);
        eprintln!("{lib}: {f} θJB {jb:.1} θJC {jc:?} area {area:.1} mm²");
        assert_eq!(f, fam);
        if fam == "qfn" {
            assert!((14.0..19.0).contains(&jb), "{jb}");
        }
        // a QFN under a custom name: found by its exposed centre pad
        if fam == "qfn" {
            let b2 = crate::load_pcb(&board_text(
                40.0,
                40.0,
                &[35.0, 35.0],
                &[1.5],
                &[],
                &extra.replace(lib, "MyLib:TPS62A02"),
            ))
            .unwrap();
            assert_eq!(package_defaults(&b2, 0).0, "qfn");
        }
    }
}

/// Parts without PI_THERMAL get package defaults (estimate), parts with only ψJB or θJA use them
/// as documented, and one electro-thermal step scales the IR drop by the copper's ρ(T).
#[test]
fn part_models_and_electro_thermal() {
    let extra = part("qfn", 20.0, 20.0, "psi_jb=17.4 tj_max=85");
    let b = crate::load_pcb(&board_text(40.0, 40.0, &[35.0, 35.0], &[1.5], &[1], &extra)).unwrap();
    let comps = vec![CompLoss { refdes: "U1".into(), part: "qfn".into(), w: 0.4, role: "load".into(), at: None }];
    let t = solve(&b, &[], &comps, &ThermalParams::default()).unwrap();
    let u = &t.parts[0];
    assert_eq!((u.model.as_str(), u.w_source.as_str()), ("psi_jb", "power tree"));
    assert!((u.tj_c - (u.t_board_c + 17.4 * 0.4)).abs() < 1e-9 && u.tj_max_c == 85.0);
    let b2 =
        crate::load_pcb(&board_text(40.0, 40.0, &[35.0, 35.0], &[1.5], &[1], &part("qfn", 20.0, 20.0, "theta_ja=45")))
            .unwrap();
    let t2 = solve(&b2, &[], &comps, &ThermalParams::default()).unwrap();
    assert!(
        t2.parts[0].model == "theta_ja"
            && (t2.parts[0].tj_c - (25.0 + 45.0 * 0.4)).abs() < 1e-9
            && t2.parts[0].estimate
    );
    let b3 = crate::load_pcb(&board_text(
        40.0,
        40.0,
        &[35.0, 35.0],
        &[1.5],
        &[1],
        &part("qfn", 20.0, 20.0, "tj_max=125").replace("(property \"PI_THERMAL\" \"tj_max=125\")", ""),
    ))
    .unwrap();
    let t3 = solve(&b3, &[], &comps, &ThermalParams::default()).unwrap();
    assert!(
        t3.parts[0].data_source.starts_with("package default qfn")
            && t3.parts[0].estimate
            && t3.parts[0].tj_max_source.starts_with("default")
    );
    assert!(t3.notes.iter().any(|n| n.contains("package default")));

    // electro-thermal: a copper strip (20 × 1 mm, 1 A) on a board at a uniform 45.445 °C (no heat):
    // ρ(T)/ρ₂₀ = 1 + 0.00393·25.445 = 1.1 → the drop rises by 10 %
    let strip = "(footprint \"R\" (layer \"F.Cu\") (at 5 5) (property \"Reference\" \"J1\") (property \"PI_SOURCE\" \"GND\") (pad \"1\" smd rect (at 0 0) (size 1 1) (layers \"F.Cu\") (net 1 \"GND\")))
      (footprint \"R\" (layer \"F.Cu\") (at 25 5) (property \"Reference\" \"U9\") (property \"PI_LOAD\" \"GND=1A\") (pad \"1\" smd rect (at 0 0) (size 1 1) (layers \"F.Cu\") (net 1 \"GND\")))
      (segment (start 5 5) (end 25 5) (width 1) (layer \"F.Cu\") (net 1))";
    let bs = crate::load_pcb(&board_text(30.0, 10.0, &[35.0, 35.0], &[1.5], &[], strip)).unwrap();
    let setup = RailSetup {
        net: "GND".into(),
        voltage: 3.3,
        sources: vec![crate::pi::SourceSpec { refdes: "J1".into(), pads: vec![] }],
        loads: vec![crate::pi::LoadSpec { refdes: "U9".into(), current: 1.0, pads: vec![], origin: "test".into() }],
        ground: false,
    };
    let pp = PiParams { cell: Some(0.1), ..Default::default() };
    let base = crate::pi::solve_rail(&bs, &setup, &pp).unwrap();
    let ta = 20.0 + 0.1 / data().copper.alpha_per_k;
    let th = solve(&bs, &[], &[], &ThermalParams { ambient_c: Some(ta), ..Default::default() }).unwrap();
    let rt = rail_at_temperature(&bs, &setup, &base, &th, &pp);
    let (d0, d1) = (rt.drop_mv_20c.unwrap(), rt.drop_mv_hot.unwrap());
    eprintln!("electro-thermal: drop {d0:.3} mV at 20 °C, {d1:.3} mV at {ta:.2} °C");
    // (the temperature map is stored as f32)
    assert!((d1 / d0 - 1.1).abs() < 1e-6 && (rt.loss_w_hot.unwrap() / rt.loss_w_20c - 1.1).abs() < 1e-6, "{}", d1 / d0);
    assert!((rt.t_max_c.unwrap() - ta).abs() < 1e-6);
}

/// Board outline from separate Edge.Cuts lines (chained into a loop), with a cut-out.
#[test]
fn outline_chaining() {
    let lines = "(gr_line (start 0 0) (end 50 0) (layer \"Edge.Cuts\")) (gr_line (start 50 30) (end 50 0) (layer \"Edge.Cuts\"))
      (gr_line (start 50 30) (end 0 30) (layer \"Edge.Cuts\")) (gr_line (start 0 0) (end 0 30) (layer \"Edge.Cuts\"))
      (gr_rect (start 20 10) (end 30 20) (layer \"Edge.Cuts\"))";
    let txt = board_text(1.0, 1.0, &[35.0, 35.0], &[1.5], &[], lines)
        .replace("(gr_rect (start 0 0) (end 1 1) (stroke (width 0.05)) (layer \"Edge.Cuts\"))", "");
    let b = crate::load_pcb(&txt).unwrap();
    let loops = outline_loops(&b);
    assert_eq!(loops.len(), 2, "{loops:?}");
    let mut loss = vec![1e-4f32; 1500];
    for j in 10..20 {
        for i in 20..30 {
            loss[j * 50 + i] = 0.0;
        }
    }
    let r = heat(0, 0.0, 0.0, 1.0, 50, 30, loss);
    let t = solve(&b, &[&r], &[], &ThermalParams { cell: Some(1.0), ..Default::default() }).unwrap();
    assert!((t.area_mm2 - 1400.0).abs() < 1.0, "{}", t.area_mm2);
    assert!((t.balance.in_w - 0.14).abs() < 1e-6 && t.warnings.is_empty(), "{} {:?}", t.balance.in_w, t.warnings);
    assert!(!t.temp_at(0, [25.0, 15.0]).is_nan());
    assert_eq!(t.temp_at(0, [25.0, 15.0]), t.ambient_c); // outside the board: ambient
                                                         // only the cut-out closed (the outline drawn elsewhere), copper all over: the copper's box
    let open =
        board_text(1.0, 1.0, &[35.0, 35.0], &[1.5], &[0], "(gr_rect (start 20 10) (end 30 20) (layer \"Edge.Cuts\"))")
            .replace("(gr_rect (start 0 0) (end 1 1) (stroke (width 0.05)) (layer \"Edge.Cuts\"))", "")
            .replace("(xy 0 0) (xy 1 0) (xy 1 1) (xy 0 1)", "(xy 0 0) (xy 50 0) (xy 50 30) (xy 0 30)");
    let b2 = crate::load_pcb(&open).unwrap();
    let t2 = solve(&b2, &[], &[], &ThermalParams { cell: Some(1.0), ..Default::default() }).unwrap();
    assert!(
        (t2.area_mm2 - 1500.0).abs() < 1.0 && t2.warnings.iter().any(|w| w.contains("does not contain all copper")),
        "{} {:?}",
        t2.area_mm2,
        t2.warnings
    );
    assert!(t2.warnings.iter().any(|w| w.starts_with("no heat")));
}

/// Heat through the vias under an exposed pad, the case top, and the hint for a thermal pad
/// without vias.
#[test]
fn vias_and_case() {
    let board = |field: &str, vias: bool| {
        let mut extra = part("qfn", 38.1, 57.15, field);
        if !vias {
            extra = extra.split("(via ").next().unwrap().to_string();
        }
        crate::load_pcb(&board_text(76.2, 114.3, &[70.0, 35.0, 35.0, 70.0], &[0.3, 0.86, 0.3], &[1, 2], &extra))
            .unwrap()
    };
    let prm = ThermalParams { ambient_c: Some(25.0), ..Default::default() };
    let t = solve(&board("theta_jb=17.4 theta_jc=53.6 p=0.5", true), &[], &[], &prm).unwrap();
    let u = &t.parts[0];
    let v = u.vias.as_ref().unwrap();
    eprintln!(
        "QFN with 4 vias: {v:?}, Tj {:.2}, case {:?} ({:?}), {} vias, top {:?}",
        u.tj_c,
        u.t_case_c,
        u.case_model,
        t.vias_total,
        t.vias.first()
    );
    assert_eq!((v.count, v.thermal_pad, t.vias_total), (4, true, 4));
    assert!(
        (v.via_w + v.laminate_w + v.lateral_w - v.to_board_w).abs() < 1e-9
            && v.via_w > 0.2 * v.to_board_w
            && v.dt_next_k > 0.0
            && v.hint.is_none()
    );
    // the four vias carry the heat downwards; the case top is (nearly) at the junction
    assert!(t.vias.iter().all(|x| x.q_w > 0.0 && x.dt_k > 0.0 && x.from == 0 && x.to == 3));
    assert!((t.vias.iter().map(|x| x.q_w).sum::<f64>() - v.via_w).abs() < 1e-9);
    let (tc, tj) = (u.t_case_c.unwrap(), u.tj_c);
    assert!(u.case_model.as_deref() == Some("theta_jc") && tc < tj && tj - tc < 1.0);
    // ψJT from the datasheet: case = Tj − ψJT·P
    let t2 = solve(&board("theta_jb=17.4 theta_jc=53.6 psi_jt=1.1 p=0.5", true), &[], &[], &prm).unwrap();
    assert!(
        (t2.parts[0].t_case_c.unwrap() - (t2.parts[0].tj_c - 0.55)).abs() < 1e-9
            && t2.parts[0].case_model.as_deref() == Some("psi_jt")
    );
    // no vias under the thermal pad: hotter, and the hint
    let t3 = solve(&board("theta_jb=17.4 theta_jc=53.6 p=0.5", false), &[], &[], &prm).unwrap();
    let v3 = t3.parts[0].vias.as_ref().unwrap();
    eprintln!("QFN without vias: {v3:?}, Tj {:.2}", t3.parts[0].tj_c);
    assert!(
        v3.count == 0
            && v3.hint.as_deref().is_some_and(|h| h.starts_with("no via under the thermal pad"))
            && t3.parts[0].tj_c > tj
    );
}

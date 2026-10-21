//! Compare kisi's impedance models against reference results.
//!
//! Usage: kisi-validate <cases.json> [reference.out.json]
//!        kisi-validate models <via_cases dir> <out dir> [--grid uniform|graded] [--only k]
//!        (reduced via / cavity models on the 3D reference cases, see models.rs)
//!
//! Case format (lengths in mm):
//!   {name, type, w, h, t, er, s?, gap?, cover?, er_top?, top_h?, sm_t?, sm_er?,
//!    ref?: {z0 | zdiff}, source?, note?}
//! types: microstrip | embedded_microstrip | gcpw | diff_microstrip | diff_gcpw |
//!        stripline | diff_stripline
//! For striplines `h` is the dielectric below the trace and `top_h` the
//! dielectric above the substrate surface (it includes the trace thickness).
//! `cover` is the dielectric above the trace top for embedded microstrip.
//! References come from `ref` in the case or from the parallel entry of the
//! reference file (`z0` / `zdiff`).
//!
//! Field-solver chain (kisi geometry generator + solver backend):
//!   kisi-validate cases.json --geoms g.json        writes geometry texts
//!   node tools/solve_xs.mjs g.json r.json          solves them
//!   kisi-validate cases.json --field r.json        compares field results

use kisi_core::analytic as a;
use kisi_core::board::{CopperLayer, Dielectric, SolderMask, Stackup};
use kisi_core::xsgeom::{self, PlaneCut, XsDesc};
use serde_json::Value;

mod models;

/// Synthetic stackup + description for a validation case (None if the case
/// type has no board equivalent, e.g. a dielectric cover on a surface trace).
fn case_geometry(c: &Value) -> Option<String> {
    let f = |k: &str| c[k].as_f64();
    let (w, h, t, er) = (f("w")?, f("h")?, f("t")?, f("er")?);
    let ty = c["type"].as_str()?;
    let cu = |n: &str, t: f64| CopperLayer { name: n.into(), thickness: t };
    let di = |k: &str, h: f64, er: f64| Dielectric { name: "d".into(), kind: k.into(), material: "m".into(), thickness: h, epsilon_r: er, loss_tangent: 0.0, assumed: vec![] };
    let mask = f("sm_t").map(|tm| (tm, f("sm_er").unwrap_or(3.3)));
    let partner = f("s").map(|s| (s, w, true));
    let (su, desc) = match ty {
        "microstrip" | "gcpw" | "diff_microstrip" | "diff_gcpw" => {
            let su = Stackup {
                copper: vec![cu("F.Cu", t), cu("B.Cu", 0.035)],
                dielectrics: vec![di("core", h, er)],
                mask_top: mask.map(|m| SolderMask { thickness: m.0, epsilon_r: m.1 }),
                mask_bottom: None,
                from_file: true,
            };
            let gap = f("gap");
            (su, XsDesc { layer: 0, w, ref_up: None, ref_dn: Some(1), s_left: gap.map(|g| g + partner.map(|p| p.0 + p.1).unwrap_or(0.0)), s_right: gap, partner, mask, planes: vec![], ..Default::default() })
        }
        "microstrip_cut" => {
            // plane directly under the trace has an opening of width `cut`;
            // reference is the next plane `cut_sub_h` further down
            let cut = f("cut")?;
            let su = Stackup {
                copper: vec![cu("F.Cu", t), cu("In1.Cu", 0.035), cu("B.Cu", 0.035)],
                dielectrics: vec![di("core", h, er), di("core", f("cut_sub_h")?, er)],
                mask_top: None,
                mask_bottom: None,
                from_file: true,
            };
            let planes = vec![PlaneCut { layer: 1, left: Some(cut / 2.0), right: Some(cut / 2.0) }];
            (su, XsDesc { layer: 0, w, ref_up: None, ref_dn: Some(2), s_left: None, s_right: None, partner: None, mask: None, planes, ..Default::default() })
        }
        "stripline" | "diff_stripline" => {
            let h2 = f("top_h")? - t;
            let su = Stackup {
                copper: vec![cu("In1.Cu", 0.035), cu("In2.Cu", t), cu("In3.Cu", 0.035)],
                dielectrics: vec![di("prepreg", h2, f("er_top").unwrap_or(er)), di("core", h, er)],
                mask_top: None,
                mask_bottom: None,
                from_file: true,
            };
            (su, XsDesc { layer: 1, w, ref_up: Some(0), ref_dn: Some(2), s_left: None, s_right: None, partner, mask: None, planes: vec![], ..Default::default() })
        }
        _ => return None,
    };
    xsgeom::build(&su, &desc)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 2 && args[1] == "via-geom" {
        // diagnostics: a ViaGeom JSON line (KISI_DEBUG_VIA output) → nodes, capacitances, row charges
        use kisi_core::via_pbm::{charge_rows, Grid, Mode, PbmVia, ViaGeom};
        let g: ViaGeom = serde_json::from_str(std::fs::read_to_string(&args[2]).unwrap().trim()).unwrap();
        let grid = if args.iter().any(|a| a == "--uniform") { Grid::Uniform { dr: 0.01, rmax: 3.0 } } else { Grid::default() };
        let m = PbmVia::build(&g, if g.partner.is_some() { Mode::Odd } else { Mode::Single }, grid);
        println!("nodes {:?}\nC fF {:?}", m.nodes, m.cap.iter().map(|c| (c * 1e16).round() / 10.0).collect::<Vec<_>>());
        for f in [1e9, 4e9, 8e9, 12e9, 16e9, 20e9] {
            let (s11, s21, _) = m.abcd(f).s_params(50.0);
            println!("{:5.1} GHz  |S21| {:7.2} dB  |S11| {:7.2} dB", f / 1e9, 20.0 * s21.abs().log10(), 20.0 * s11.abs().log10());
        }
        for (z, q) in charge_rows(&g, grid) {
            if q.abs() > 2e-15 {
                println!("row z {z:.4}  {:.1} fF", q * 1e15);
            }
        }
        return;
    }
    if args.len() > 3 && args[1] == "models" {
        models::run(&args[2..]);
        return;
    }
    if args.len() < 2 {
        eprintln!("usage: kisi-validate <cases.json> [reference.out.json]");
        std::process::exit(2);
    }
    let cases: Vec<Value> = serde_json::from_str(&std::fs::read_to_string(&args[1]).unwrap()).unwrap();
    let opt = |k: &str| args.iter().position(|a| a == k).and_then(|i| args.get(i + 1)).cloned();
    if let Some(out) = opt("--geoms") {
        let list: Vec<Value> = cases
            .iter()
            .filter_map(|c| case_geometry(c).map(|text| serde_json::json!({"key": c["name"], "text": text})))
            .collect();
        std::fs::write(&out, serde_json::to_string_pretty(&list).unwrap()).unwrap();
        println!("{} geometries written to {out}", list.len());
        return;
    }
    let field: Vec<Value> = opt("--field")
        .map(|p| serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap())
        .unwrap_or_default();
    let refs: Vec<Value> = args
        .get(2)
        .filter(|a| !a.starts_with("--"))
        .map(|p| serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap())
        .unwrap_or_default();
    println!("{:<46} {:>9} {:>9} {:>8}  {}", "case", "ref Ω", "kisi Ω", "err", "source");
    let mut worst: f64 = 0.0;
    let mut n = 0usize;
    for (i, c) in cases.iter().enumerate() {
        let f = |k: &str| c[k].as_f64();
        let (w, h, t, er) = (f("w").unwrap(), f("h").unwrap(), f("t").unwrap(), f("er").unwrap());
        let ty = c["type"].as_str().unwrap();
        let mask = f("sm_t").map(|tm| (tm, f("sm_er").unwrap_or(3.3)));
        let gap = f("gap");
        let (ours, key) = match ty {
            "microstrip" => (a::microstrip_model(w, h, t, er, mask), "z0"),
            // formula path of the extraction: reference on the deeper plane
            "microstrip_cut" => (a::microstrip_model(w, h + 0.035 + f("cut_sub_h").unwrap(), t, er, mask), "z0"),
            "gcpw" => (a::microstrip_coplanar(w, h, t, er, mask, gap, gap), "z0"),
            "embedded_microstrip" => (a::embedded_microstrip(w, h, t, f("cover").unwrap(), er).0, "z0"),
            "diff_microstrip" => (a::diff_microstrip_model(w, f("s").unwrap(), h, t, er, mask), "zdiff"),
            "diff_gcpw" => (a::diff_microstrip_coplanar(w, f("s").unwrap(), h, t, er, mask, gap, gap), "zdiff"),
            "stripline" | "diff_stripline" => {
                let h2 = f("top_h").unwrap() - t;
                let e2 = f("er_top").unwrap_or(er);
                if ty == "stripline" {
                    (a::stripline_asym(w, h, h2, t, er, e2), "z0")
                } else {
                    (a::diff_stripline_model(w, f("s").unwrap(), h, h2, t, er, e2), "zdiff")
                }
            }
            other => {
                eprintln!("unknown type {other}");
                continue;
            }
        };
        // with --field, compare the field result instead of the formula
        let ours = if field.is_empty() {
            ours
        } else {
            let Some(r) = field.iter().find(|r| r["key"] == c["name"]) else { continue };
            match (key, r["z0"].as_f64(), r["zodd"].as_f64()) {
                ("z0", Some(z), _) => z,
                ("zdiff", _, Some(zo)) => 2.0 * zo,
                _ => continue,
            }
        };
        let reference = c["ref"][key].as_f64().or_else(|| refs.get(i).and_then(|r| r[key].as_f64()));
        let Some(rv) = reference else { continue };
        let err = (ours - rv) / rv;
        worst = worst.max(err.abs());
        n += 1;
        println!(
            "{:<46} {:>9.2} {:>9.2} {:>+7.1}%  {}",
            c["name"].as_str().unwrap_or(""),
            rv,
            ours,
            err * 100.0,
            c["source"].as_str().unwrap_or("")
        );
    }
    println!("{n} cases, worst |err| = {:.1}%", worst * 100.0);
}

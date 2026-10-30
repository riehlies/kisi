//! Cross-section cases for the external 2D-solver comparison (validation/cst_xs).
//!
//! ```text
//! kisi-validate xs-cases <cases.json> --out <out.json>
//!     every case with a `spec` (stackup + cross-section description): kisi's geometry text
//!     (xsgeom::build, the text the field solver gets) and the extraction's formula values
//!     (extract::formula_xs) → [{key, text|null, formula}]
//! kisi-validate xs-board <board.kicad_pcb> --out <out.json> [--pro F] [--config F | --no-config]
//!     [--variant NAME]
//!     the impedance cross-sections kisi extracts on a board: per geometry its text, routed length,
//!     nets and net classes, and the spec it was built from (stackup + description), so that
//!     `xs-cases` reproduces the same text
//! ```
//!
//! A spec: `{"stackup": {"copper": [{"name", "thickness"}], "dielectrics": [{"kind", "thickness",
//! "epsilon_r", "loss_tangent"}], "mask_top": {"thickness", "epsilon_r"} | null, "mask_bottom": …},
//! "desc": {"layer", "w", "ref_up", "ref_dn", "s_left", "s_right", "partner": [gap, w, left],
//! "mask": [t, er], "planes": [[layer, left, right]], "extra": [[d, w, kind]], "trace_gnd"}}`
//! (lengths in mm, absent or null = None), the fields of `board::Stackup` and `xsgeom::XsDesc`.

use kisi_core::board::{CopperLayer, Dielectric, SolderMask, Stackup};
use kisi_core::extract::{formula_xs, Kind};
use kisi_core::service::Project;
use kisi_core::xsgeom::{self, Extra, PlaneCut, XsDesc};
use serde_json::{json, Value};
use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;

fn die(msg: &str) -> ! {
    eprintln!("kisi-validate: {msg}");
    std::process::exit(2);
}

fn opt(args: &[String], k: &str) -> Option<String> {
    args.iter().position(|a| a == k).and_then(|i| args.get(i + 1)).cloned()
}

fn stackup_from(v: &Value) -> Result<Stackup, String> {
    let num = |x: &Value, k: &str| x[k].as_f64().ok_or_else(|| format!("stackup: missing {k}"));
    let mut copper = Vec::new();
    for (i, c) in v["copper"].as_array().ok_or("stackup: no copper")?.iter().enumerate() {
        copper.push(CopperLayer {
            name: c["name"].as_str().map(str::to_string).unwrap_or_else(|| format!("L{}", i + 1)),
            thickness: num(c, "thickness")?,
        });
    }
    let mut dielectrics = Vec::new();
    for d in v["dielectrics"].as_array().ok_or("stackup: no dielectrics")? {
        dielectrics.push(Dielectric {
            name: d["name"].as_str().unwrap_or("d").into(),
            kind: d["kind"].as_str().unwrap_or("core").into(),
            material: d["material"].as_str().unwrap_or("m").into(),
            thickness: num(d, "thickness")?,
            epsilon_r: num(d, "epsilon_r")?,
            loss_tangent: d["loss_tangent"].as_f64().unwrap_or(0.0),
            assumed: vec![],
        });
    }
    if dielectrics.len() + 1 != copper.len() {
        return Err("stackup: need one dielectric between each pair of copper layers".into());
    }
    let mask = |m: &Value| -> Option<SolderMask> {
        Some(SolderMask { thickness: m["thickness"].as_f64()?, epsilon_r: m["epsilon_r"].as_f64()? })
    };
    Ok(Stackup {
        copper,
        dielectrics,
        mask_top: mask(&v["mask_top"]),
        mask_bottom: mask(&v["mask_bottom"]),
        from_file: true,
    })
}

fn desc_from(v: &Value) -> Result<XsDesc, String> {
    let idx = |k: &str| v[k].as_u64().map(|x| x as usize);
    let f = |k: &str| v[k].as_f64();
    let pair = |k: &str| -> Option<(f64, f64)> {
        let a = v[k].as_array()?;
        Some((a.first()?.as_f64()?, a.get(1)?.as_f64()?))
    };
    let partner = v["partner"].as_array().and_then(|a| {
        Some((a.first()?.as_f64()?, a.get(1)?.as_f64()?, a.get(2).and_then(Value::as_bool).unwrap_or(true)))
    });
    let mut planes = Vec::new();
    for p in v["planes"].as_array().into_iter().flatten() {
        let a = p.as_array().ok_or("desc: planes entry is not [layer, left, right]")?;
        planes.push(PlaneCut {
            layer: a.first().and_then(Value::as_u64).ok_or("desc: plane layer")? as usize,
            left: a.get(1).and_then(Value::as_f64),
            right: a.get(2).and_then(Value::as_f64),
        });
    }
    let mut extra = Vec::new();
    for e in v["extra"].as_array().into_iter().flatten() {
        let a = e.as_array().ok_or("desc: extra entry is not [d, w, kind]")?;
        let kind = match a.get(2).and_then(Value::as_str).unwrap_or("gnd") {
            "sig+" => "sig+",
            "sig-" => "sig-",
            _ => "gnd",
        };
        extra.push(Extra {
            d: a.first().and_then(Value::as_f64).ok_or("desc: extra d")?,
            w: a.get(1).and_then(Value::as_f64).ok_or("desc: extra w")?,
            kind,
        });
    }
    Ok(XsDesc {
        layer: idx("layer").ok_or("desc: no layer")?,
        w: f("w").ok_or("desc: no w")?,
        ref_up: idx("ref_up"),
        ref_dn: idx("ref_dn"),
        s_left: f("s_left"),
        s_right: f("s_right"),
        partner,
        mask: pair("mask"),
        planes,
        extra,
        trace_gnd: v["trace_gnd"].as_bool().unwrap_or(false),
    })
}

fn kind_name(k: &Kind) -> &'static str {
    match k {
        Kind::Microstrip => "microstrip",
        Kind::Stripline => "stripline",
        Kind::EmbeddedMicrostrip => "embedded_microstrip",
        Kind::NoReference => "no_reference",
    }
}

fn formula_json(su: &Stackup, d: &XsDesc) -> Value {
    let fx = formula_xs(su, d);
    json!({
        "kind": kind_name(&fx.kind),
        "z0": fx.z0, "eeff": fx.eeff, "zdiff": fx.zdiff, "zeven": fx.zeven,
        "zodd": fx.zdiff.map(|z| z / 2.0),
        "eeff_odd": fx.eeff_odd, "eeff_even": fx.eeff_even, "flags": fx.flags,
    })
}

fn desc_json(d: &XsDesc) -> Value {
    json!({
        "layer": d.layer, "w": d.w, "ref_up": d.ref_up, "ref_dn": d.ref_dn,
        "s_left": d.s_left, "s_right": d.s_right,
        "partner": d.partner.map(|p| json!([p.0, p.1, p.2])),
        "mask": d.mask.map(|m| json!([m.0, m.1])),
        "planes": d.planes.iter().map(|c| json!([c.layer, c.left, c.right])).collect::<Vec<_>>(),
        "extra": d.extra.iter().map(|e| json!([e.d, e.w, e.kind])).collect::<Vec<_>>(),
        "trace_gnd": d.trace_gnd,
    })
}

fn stackup_json(su: &Stackup) -> Value {
    json!({
        "copper": su.copper.iter().map(|c| json!({"name": c.name, "thickness": c.thickness})).collect::<Vec<_>>(),
        "dielectrics": su.dielectrics.iter().map(|d| json!({"name": d.name, "kind": d.kind,
            "thickness": d.thickness, "epsilon_r": d.epsilon_r, "loss_tangent": d.loss_tangent})).collect::<Vec<_>>(),
        "mask_top": su.mask_top.as_ref().map(|m| json!({"thickness": m.thickness, "epsilon_r": m.epsilon_r})),
        "mask_bottom": su.mask_bottom.as_ref().map(|m| json!({"thickness": m.thickness, "epsilon_r": m.epsilon_r})),
    })
}

/// `kisi-validate xs-cases`
pub fn run_cases(args: &[String]) {
    let path = args.first().unwrap_or_else(|| die("xs-cases: no cases file"));
    let out = opt(args, "--out").unwrap_or_else(|| die("xs-cases: --out missing"));
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| die(&format!("{path}: {e}")));
    let v: Value = serde_json::from_str(&text).unwrap_or_else(|e| die(&format!("{path}: {e}")));
    let cases = v.get("cases").unwrap_or(&v).as_array().unwrap_or_else(|| die("xs-cases: no case list")).clone();
    let mut res = Vec::new();
    let mut bad = 0;
    for c in &cases {
        let Some(spec) = c.get("spec").filter(|s| s.is_object()) else { continue };
        let key = c.get("id").or_else(|| c.get("key")).cloned().unwrap_or(Value::Null);
        match (stackup_from(&spec["stackup"]), desc_from(&spec["desc"])) {
            (Ok(su), Ok(d)) => {
                if d.layer >= su.n() {
                    bad += 1;
                    res.push(json!({"key": key, "error": "layer outside the stackup"}));
                    continue;
                }
                res.push(json!({"key": key, "text": xsgeom::build(&su, &d), "formula": formula_json(&su, &d)}));
            }
            (Err(e), _) | (_, Err(e)) => {
                bad += 1;
                res.push(json!({"key": key, "error": e}));
            }
        }
    }
    std::fs::write(&out, serde_json::to_string_pretty(&res).unwrap()).unwrap_or_else(|e| die(&format!("{out}: {e}")));
    println!(
        "{} cases written to {out}{}",
        res.len(),
        if bad > 0 { format!(", {bad} with errors") } else { String::new() }
    );
}

/// `kisi-validate xs-board`
pub fn run_board(args: &[String]) {
    let pcb = PathBuf::from(args.first().unwrap_or_else(|| die("xs-board: no board")));
    let out = opt(args, "--out").unwrap_or_else(|| die("xs-board: --out missing"));
    let pro = opt(args, "--pro").map(PathBuf::from);
    let cfg = if args.iter().any(|a| a == "--no-config") {
        None
    } else {
        opt(args, "--config")
            .map(PathBuf::from)
            .or_else(|| Some(kisi_core::service::config_path_for(&pcb)).filter(|p| p.is_file()))
    };
    let variant = opt(args, "--variant");
    let p =
        Project::load_with(&pcb, pro.as_deref(), None, cfg.as_deref(), variant.as_deref()).unwrap_or_else(|e| die(&e));
    let b = &p.board;
    let su = b.stackup();
    // nets and classes along each geometry
    let mut nets: HashMap<u32, BTreeSet<String>> = HashMap::new();
    let mut classes: HashMap<u32, BTreeSet<String>> = HashMap::new();
    for c in p.an.pieces.chunks(4) {
        let (ti, xi) = (c[0] as usize, c[3] as usize);
        let (Some(tr), Some(gi)) = (b.tracks.get(ti), p.an.cross_sections.get(xi).and_then(|x| x.geom)) else {
            continue;
        };
        nets.entry(gi).or_default().insert(b.net_name(tr.net).to_string());
        classes.entry(gi).or_default().insert(b.class_of(tr.net).name.clone());
    }
    let mut res = Vec::new();
    let mut mismatch = 0;
    for (gi, g) in p.an.geometries.iter().enumerate() {
        let gi = gi as u32;
        let Some(xs) = p.an.cross_sections.iter().find(|x| x.geom == Some(gi)) else { continue };
        let li = xs.layer as usize;
        let mask = if !xs.mask {
            None
        } else if li == 0 {
            su.mask_top.as_ref()
        } else {
            su.mask_bottom.as_ref()
        };
        let d = XsDesc {
            layer: li,
            w: xs.w,
            ref_up: xs.ref_up.map(usize::from),
            ref_dn: xs.ref_dn.map(usize::from),
            s_left: xs.s_left,
            s_right: xs.s_right,
            partner: xs.diff_gap.zip(xs.partner_w).map(|(g, w)| (g, w, true)),
            mask: mask.map(|m| (m.thickness, m.epsilon_r)),
            planes: xs.planes.iter().map(|&(l, a, b)| PlaneCut { layer: l as usize, left: a, right: b }).collect(),
            ..Default::default()
        };
        let same = xsgeom::build(su, &d).as_deref() == Some(g.text.as_str());
        if !same {
            mismatch += 1;
        }
        res.push(json!({
            "id": g.id, "text": g.text, "length": g.length, "target_length": g.target_length,
            "layer_name": su.copper[li].name,
            "nets": nets.get(&gi).map(|s| s.iter().take(6).cloned().collect::<Vec<_>>()).unwrap_or_default(),
            "n_nets": nets.get(&gi).map(|s| s.len()).unwrap_or(0),
            "classes": classes.get(&gi).map(|s| s.iter().cloned().collect::<Vec<_>>()).unwrap_or_default(),
            "flags": xs.flags,
            "spec": if same { json!({"stackup": stackup_json(su), "desc": desc_json(&d)}) } else { Value::Null },
            "formula": formula_json(su, &d),
        }));
    }
    std::fs::write(&out, serde_json::to_string_pretty(&res).unwrap()).unwrap_or_else(|e| die(&format!("{out}: {e}")));
    println!(
        "{}: {} geometries written to {out}{}",
        pcb.display(),
        res.len(),
        if mismatch > 0 {
            format!(", {mismatch} not reproduced from their description (spec null)")
        } else {
            String::new()
        }
    );
}

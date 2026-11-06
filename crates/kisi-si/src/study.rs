//! Stackup study: the impedance of each net class's main cross-section per layer under other
//! stackups (fab templates, edited copies) and its band for manufacturing tolerances.
//!
//! A row is the cross-section that carries most of a class's length on a layer (pads left out; for
//! a differential target a coupled one where there is one). Its description (`GeomEntry::desc`) is
//! evaluated under any stackup with the same functions as the extraction: the field-solver result of
//! the geometry text where one exists, else the formulas. The cross-section's topology (references,
//! coplanar copper, partner) stays as found on the board.
//!
//! Tolerances: one-at-a-time sensitivities by central differences, ΔZ_i = (Z(x_i + δ_i) −
//! Z(x_i − δ_i)) / 2 for each dielectric's thickness and εr, the trace layer's copper thickness and
//! the trace width (symmetric: every copper edge moves by half the width change, so gaps to the
//! partner and to coplanar copper shrink by as much as the widths grow – no etch factor, no
//! trapezoid), at the same solver level as the nominal value where the field solver has every
//! perturbed geometry, else from the formulas scaled to the nominal value. Combined as worst case
//! Σ|ΔZ_i| and as root sum of squares √(ΣΔZ_i²) – a linear estimate around the nominal value.

use crate::board::{Board, Stackup};
use crate::extract::{formula_xs, Analysis, CrossSection, FieldResult, Kind, Target, F_PAD};
use crate::xsgeom::{self, XsDesc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

/// Share of a class's length (with target) a layer needs to get a row.
pub const MIN_LAYER_SHARE: f64 = 0.05;

/// One row: the main cross-section of a net class on one layer.
#[derive(Debug, Clone, Serialize)]
pub struct Row {
    pub class: String,
    pub target: Target,
    pub layer: usize,
    pub layer_name: String,
    pub kind: Kind,
    /// trace width (mm), the gap to a pair's partner (mm), coplanar copper beside it
    pub w_mm: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gap_mm: Option<f64>,
    pub coplanar: bool,
    /// the class's length with this cross-section, and its share of the class's length on the layer
    /// and of the class's whole length
    pub length_mm: f64,
    pub share_of_layer: f64,
    pub share_of_class: f64,
    /// index into `Analysis::geometries`
    pub geom: u32,
    #[serde(skip)]
    pub desc: XsDesc,
    #[serde(skip)]
    pub xs: Option<CrossSection>,
}

/// The rows of a board's analysis, by class name and layer.
pub fn rows(board: &Board, an: &Analysis) -> Vec<Row> {
    let net_info: HashMap<u32, (&str, Target)> =
        an.nets.iter().filter_map(|n| Some((n.net, (n.class.as_str(), n.target?)))).collect();
    // (class, layer) → geom → (length, coupled, first cross-section index)
    let mut acc: BTreeMap<(String, usize), HashMap<u32, (f64, bool, usize)>> = BTreeMap::new();
    let mut class_len: HashMap<String, f64> = HashMap::new();
    let mut layer_len: HashMap<(String, usize), f64> = HashMap::new();
    for pc in an.pieces.chunks(4) {
        let (ti, t0, t1, xi) = (pc[0] as usize, pc[1], pc[2], pc[3] as usize);
        let (Some(tr), Some(xs)) = (board.tracks.get(ti), an.cross_sections.get(xi)) else { continue };
        let Some(&(class, _)) = net_info.get(&tr.net) else { continue };
        let len = crate::geom::len(crate::geom::sub(tr.b, tr.a)) * (t1 - t0).abs();
        *class_len.entry(class.to_string()).or_default() += len;
        let key = (class.to_string(), xs.layer as usize);
        *layer_len.entry(key.clone()).or_default() += len;
        let Some(g) = xs.geom else { continue };
        if xs.flags & F_PAD != 0 || xs.kind == Kind::NoReference {
            continue;
        }
        let e = acc.entry(key).or_default().entry(g).or_insert((0.0, xs.diff_gap.is_some(), xi));
        e.0 += len;
    }
    let su = board.stackup();
    let targets: HashMap<&str, Target> = net_info.values().map(|&(c, t)| (c, t)).collect();
    let mut out = Vec::new();
    for ((class, layer), geoms) in acc {
        let total = class_len.get(&class).copied().unwrap_or(0.0);
        let on_layer = layer_len.get(&(class.clone(), layer)).copied().unwrap_or(0.0);
        if total <= 0.0 || on_layer < MIN_LAYER_SHARE * total {
            continue;
        }
        let target = targets[class.as_str()];
        let want_coupled = target.diff && geoms.values().any(|g| g.1);
        let Some((&g, &(len, _, xi))) = geoms
            .iter()
            .filter(|(_, v)| !want_coupled || v.1)
            .max_by(|a, b| a.1 .0.total_cmp(&b.1 .0).then(b.0.cmp(a.0)))
        else {
            continue;
        };
        let xs = &an.cross_sections[xi];
        let Some(ge) = an.geometries.get(g as usize) else { continue };
        out.push(Row {
            class: class.clone(),
            target,
            layer,
            layer_name: su.copper.get(layer).map(|c| c.name.clone()).unwrap_or_default(),
            kind: xs.kind,
            w_mm: xs.w,
            gap_mm: xs.diff_gap,
            coplanar: xs.s_left.is_some() || xs.s_right.is_some(),
            length_mm: len,
            share_of_layer: len / on_layer,
            share_of_class: on_layer / total,
            geom: g,
            desc: ge.desc.clone(),
            xs: Some(xs.clone()),
        });
    }
    out
}

/// The cross-section's description with the solder mask of stackup `su` (the board's description
/// carries the board's mask).
fn with_mask(su: &Stackup, d: &XsDesc) -> XsDesc {
    let mut d = d.clone();
    if d.mask.is_some() {
        let m = if d.layer == 0 { su.mask_top.as_ref() } else { su.mask_bottom.as_ref() };
        d.mask = m.map(|m| (m.thickness, m.epsilon_r));
    }
    d
}

/// The impedance of a description under a stackup, compared as the target is (Zdiff for a
/// differential target – 2·Z0 where the pair is not coupled –, else Z0).
#[derive(Debug, Clone)]
pub struct Eval {
    pub z: f64,
    /// from the field solver (else the formulas)
    pub field: bool,
    /// the formula value (also when `z` is the field solver's)
    pub z_formula: Option<f64>,
    /// the geometry for the field solver: (id, text)
    pub geom: Option<(String, String)>,
}

pub fn eval(su: &Stackup, desc: &XsDesc, t: Target, field: &HashMap<String, FieldResult>) -> Option<Eval> {
    let d = with_mask(su, desc);
    let fx = formula_xs(su, &d);
    let pick = |z0: Option<f64>, zd: Option<f64>| if t.diff { zd.or(z0.map(|z| 2.0 * z)) } else { z0 };
    let zf = pick(fx.z0, fx.zdiff);
    let geom = xsgeom::build(su, &d).map(|text| (xsgeom::geom_id(&text), text));
    let fr = geom.as_ref().and_then(|g| field.get(&g.0));
    let zfield = fr.and_then(|fr| match (fr.zodd, fr.zeven) {
        (Some(zo), Some(ze)) => pick(Some((zo * ze).sqrt()), Some(2.0 * zo)),
        _ => fr.z0.and_then(|z| pick(Some(z), None)),
    });
    match (zfield, zf) {
        (Some(z), _) => Some(Eval { z, field: true, z_formula: zf, geom }),
        (None, Some(z)) => Some(Eval { z, field: false, z_formula: zf, geom }),
        _ => None,
    }
}

/// Manufacturing tolerances (± around the nominal value). `width_pct`, when given, wins over
/// `width_mm`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tolerances {
    /// every dielectric's thickness, %
    pub dielectric_thickness_pct: f64,
    /// every dielectric's εr, %
    pub er_pct: f64,
    /// the trace layer's copper thickness, %
    pub copper_pct: f64,
    /// trace width, mm (symmetric: each edge half of it)
    pub width_mm: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width_pct: Option<f64>,
}

/// One parameter's effect: ΔZ at +tolerance (linear, from the central difference), Ω.
#[derive(Debug, Clone, Serialize)]
pub struct Sens {
    /// "thickness", "er", "copper", "width"
    pub kind: &'static str,
    /// what was varied ("prepreg F.Cu/In1.Cu", "copper In1.Cu", "width 0.127 mm")
    pub what: String,
    /// the varied quantity at the nominal value and its tolerance (absolute, in mm or εr)
    pub nominal: f64,
    pub delta: f64,
    pub dz: f64,
}

/// The tolerance band of one row.
#[derive(Debug, Clone, Serialize)]
pub struct Band {
    pub sens: Vec<Sens>,
    /// per kind: Σ|ΔZ| of its parameters (Ω)
    pub by_kind: BTreeMap<&'static str, f64>,
    pub worst_ohm: f64,
    pub rss_ohm: f64,
    /// "field solver" when every perturbed geometry had a field result, else "formula"
    pub level: &'static str,
}

/// A perturbed copy of stackup and description.
fn perturbations(
    su: &Stackup,
    d: &XsDesc,
    tol: &Tolerances,
) -> Vec<(&'static str, String, f64, f64, [(Stackup, XsDesc); 2])> {
    let mut v = Vec::new();
    let name = |k: usize| {
        let a = su.copper.get(k).map(|c| c.name.as_str()).unwrap_or("?");
        let b = su.copper.get(k + 1).map(|c| c.name.as_str()).unwrap_or("?");
        format!("{} {a}/{b}", su.dielectrics[k].kind.trim())
    };
    // the dielectrics of the modelled stack (as `xsgeom::build`): between the references, up to the
    // board surface on a side without one; the others only shift the geometry's coordinates
    let n = su.n();
    let k_top = d.ref_up.unwrap_or(0);
    let k_bot = d.ref_dn.unwrap_or(n.saturating_sub(1));
    for k in k_top..k_bot.min(su.dielectrics.len()) {
        let h = su.dielectrics[k].thickness;
        let dh = h * tol.dielectric_thickness_pct / 100.0;
        if dh > 0.0 {
            let mk = |s: f64| {
                let mut x = su.clone();
                x.dielectrics[k].thickness = h + s * dh;
                (x, d.clone())
            };
            v.push(("thickness", name(k), h, dh, [mk(1.0), mk(-1.0)]));
        }
        let er = su.dielectrics[k].epsilon_r;
        let de = er * tol.er_pct / 100.0;
        if de > 0.0 {
            let mk = |s: f64| {
                let mut x = su.clone();
                x.dielectrics[k].epsilon_r = er + s * de;
                (x, d.clone())
            };
            v.push(("er", name(k), er, de, [mk(1.0), mk(-1.0)]));
        }
    }
    let li = d.layer;
    if let Some(c) = su.copper.get(li) {
        let t = c.thickness;
        let dt = t * tol.copper_pct / 100.0;
        if dt > 0.0 {
            let mk = |s: f64| {
                let mut x = su.clone();
                x.copper[li].thickness = t + s * dt;
                (x, d.clone())
            };
            v.push(("copper", format!("copper {}", c.name), t, dt, [mk(1.0), mk(-1.0)]));
        }
    }
    let dw = tol.width_pct.map(|p| d.w * p / 100.0).unwrap_or(tol.width_mm);
    if dw > 0.0 {
        let mk = |s: f64| {
            let dw = s * dw;
            let gap = |g: f64| (g - dw).max(0.005);
            let mut x = d.clone();
            x.w = (d.w + dw).max(0.01);
            x.s_left = d.s_left.map(gap);
            x.s_right = d.s_right.map(gap);
            x.partner = d.partner.map(|(g, pw, left)| (gap(g), (pw + dw).max(0.01), left));
            (su.clone(), x)
        };
        v.push(("width", format!("width {} mm", fmt(d.w)), d.w, dw, [mk(1.0), mk(-1.0)]));
    }
    v
}

fn fmt(v: f64) -> String {
    let s = format!("{v:.4}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// The tolerance band of a description under a stackup, and the perturbed geometries the field
/// solver would need for it (id, text). `nominal` is the row's nominal evaluation.
pub fn band(
    su: &Stackup,
    desc: &XsDesc,
    t: Target,
    tol: &Tolerances,
    field: &HashMap<String, FieldResult>,
    nominal: &Eval,
) -> (Option<Band>, Vec<(String, String)>) {
    let base_text = nominal.geom.as_ref().map(|g| g.1.clone());
    let mut items = Vec::new();
    let mut need = Vec::new();
    let mut all_field = nominal.field;
    for (kind, what, x0, dx, [(sp, dp), (sm, dm)]) in perturbations(su, desc, tol) {
        let (Some(a), Some(b)) = (eval(&sp, &dp, t, field), eval(&sm, &dm, t, field)) else { continue };
        // a parameter that changes neither the formula nor the geometry has no effect (a dielectric
        // outside the cross-section)
        let same_geom = |e: &Eval| e.geom.as_ref().map(|g| &g.1) == base_text.as_ref();
        let no_formula_change = a.z_formula == nominal.z_formula && b.z_formula == nominal.z_formula;
        if same_geom(&a) && same_geom(&b) && no_formula_change {
            continue;
        }
        for e in [&a, &b] {
            if !e.field {
                all_field = false;
                if let Some(g) = &e.geom {
                    if !need.iter().any(|x: &(String, String)| x.0 == g.0) {
                        need.push(g.clone());
                    }
                }
            }
        }
        items.push((kind, what, x0, dx, a, b));
    }
    if items.is_empty() {
        return (None, need);
    }
    // the same level as the nominal value: field differences where every perturbed geometry has a
    // field result, else formula differences scaled to the nominal value
    let level = if nominal.field && all_field { "field solver" } else { "formula" };
    let scale = match (nominal.field && !all_field, nominal.z_formula) {
        (true, Some(zf)) if zf > 0.0 => nominal.z / zf,
        _ => 1.0,
    };
    let mut sens = Vec::new();
    for (kind, what, x0, dx, a, b) in items {
        let dz = if level == "field solver" {
            (a.z - b.z) / 2.0
        } else {
            match (a.z_formula, b.z_formula) {
                (Some(za), Some(zb)) => scale * (za - zb) / 2.0,
                _ => continue,
            }
        };
        sens.push(Sens { kind, what, nominal: x0, delta: dx, dz });
    }
    let mut by_kind: BTreeMap<&'static str, f64> = BTreeMap::new();
    for s in &sens {
        *by_kind.entry(s.kind).or_default() += s.dz.abs();
    }
    let worst_ohm = sens.iter().map(|s| s.dz.abs()).sum();
    let rss_ohm = sens.iter().map(|s| s.dz * s.dz).sum::<f64>().sqrt();
    (Some(Band { sens, by_kind, worst_ohm, rss_ohm, level }), need)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::{CopperLayer, Dielectric, SolderMask};
    use crate::extract::FormulaXs;

    fn stack() -> Stackup {
        let d = |k: &str, h: f64, er: f64| Dielectric {
            name: k.into(),
            kind: k.into(),
            material: "FR4".into(),
            thickness: h,
            epsilon_r: er,
            loss_tangent: 0.02,
            assumed: vec![],
        };
        Stackup {
            copper: ["F.Cu", "In1.Cu", "In2.Cu", "B.Cu"]
                .iter()
                .map(|n| CopperLayer { name: n.to_string(), thickness: 0.035 })
                .collect(),
            dielectrics: vec![d("prepreg", 0.2, 4.4), d("core", 1.1, 4.6), d("prepreg", 0.2, 4.4)],
            mask_top: Some(SolderMask { thickness: 0.02, epsilon_r: 3.8 }),
            mask_bottom: Some(SolderMask { thickness: 0.02, epsilon_r: 3.8 }),
            from_file: true,
        }
    }

    fn ms(w: f64) -> XsDesc {
        XsDesc { layer: 0, w, ref_dn: Some(1), mask: Some((0.01, 3.3)), ..Default::default() }
    }

    fn z(su: &Stackup, d: &XsDesc) -> f64 {
        let f: FormulaXs = formula_xs(su, &with_mask(su, d));
        f.z0.unwrap()
    }

    /// The central differences against the formula directly, and the combination rules.
    #[test]
    fn sensitivities() {
        let su = stack();
        let d = ms(0.35);
        let t = Target { z: 50.0, diff: false };
        let tol = Tolerances {
            dielectric_thickness_pct: 10.0,
            er_pct: 5.0,
            copper_pct: 10.0,
            width_mm: 0.02,
            width_pct: None,
        };
        let f = HashMap::new();
        let n = eval(&su, &d, t, &f).unwrap();
        assert!(!n.field && (n.z - z(&su, &d)).abs() < 1e-12);
        let (b, need) = band(&su, &d, t, &tol, &f, &n);
        let b = b.unwrap();
        // only the prepreg under the trace, its εr, the copper and the width matter
        let kinds: Vec<&str> = b.sens.iter().map(|s| s.kind).collect();
        assert_eq!(kinds, ["thickness", "er", "copper", "width"], "{:?}", b.sens);
        let mut sp = su.clone();
        sp.dielectrics[0].thickness = 0.22;
        let mut sm = su.clone();
        sm.dielectrics[0].thickness = 0.18;
        let dz_h = (z(&sp, &d) - z(&sm, &d)) / 2.0;
        assert!((b.sens[0].dz - dz_h).abs() < 1e-9 && dz_h > 0.0, "{} {dz_h}", b.sens[0].dz);
        // wider trace: lower impedance
        assert!(b.sens[3].dz < 0.0 && b.sens[3].delta == 0.02);
        // εr up: lower impedance
        assert!(b.sens[1].dz < 0.0);
        let worst: f64 = b.sens.iter().map(|s| s.dz.abs()).sum();
        let rss = b.sens.iter().map(|s| s.dz * s.dz).sum::<f64>().sqrt();
        assert!((b.worst_ohm - worst).abs() < 1e-12 && (b.rss_ohm - rss).abs() < 1e-12 && rss < worst);
        assert_eq!(b.level, "formula");
        // the formula level needs no field geometries beyond those it lists; all differ from the nominal
        assert_eq!(need.len(), 8);
        // a 10 % thicker prepreg moves a 50 Ω microstrip by a few ohms
        assert!(dz_h > 1.0 && dz_h < 5.0, "{dz_h}");
        // relative width tolerance
        let tp = Tolerances { width_pct: Some(10.0), ..tol.clone() };
        let (bp, _) = band(&su, &d, t, &tp, &f, &n);
        assert!((bp.unwrap().sens[3].delta - 0.035).abs() < 1e-12);
    }

    /// With field results for the nominal and every perturbed geometry the band uses them; with the
    /// nominal only, the formula differences scaled to the field value.
    #[test]
    fn field_level() {
        let su = stack();
        let d = ms(0.35);
        let t = Target { z: 50.0, diff: false };
        let tol =
            Tolerances { dielectric_thickness_pct: 10.0, er_pct: 0.0, copper_pct: 0.0, width_mm: 0.0, width_pct: None };
        let mut f = HashMap::new();
        let n0 = eval(&su, &d, t, &f).unwrap();
        let (gid, _) = n0.geom.clone().unwrap();
        f.insert(gid, FieldResult { z0: Some(2.0 * n0.z), ..Default::default() });
        let n = eval(&su, &d, t, &f).unwrap();
        assert!(n.field && (n.z - 2.0 * n0.z).abs() < 1e-9);
        let (b, need) = band(&su, &d, t, &tol, &f, &n);
        let b = b.unwrap();
        assert_eq!((b.level, need.len()), ("formula", 2));
        // scaled: twice the formula difference
        let (bf, _) = band(&su, &d, t, &tol, &HashMap::new(), &n0);
        assert!((b.sens[0].dz - 2.0 * bf.unwrap().sens[0].dz).abs() < 1e-9);
        // the perturbed geometries solved: field differences
        for (id, _) in need {
            f.insert(id, FieldResult { z0: Some(100.0), ..Default::default() });
        }
        let (b2, need2) = band(&su, &d, t, &tol, &f, &n);
        let b2 = b2.unwrap();
        assert!(need2.is_empty() && b2.level == "field solver" && b2.sens[0].dz.abs() < 1e-12);
    }

    /// A pair: the width change narrows the gap by as much (pitch kept), Zdiff for a differential target.
    #[test]
    fn pair_width() {
        let su = stack();
        let d = XsDesc { partner: Some((0.15, 0.2, true)), ..ms(0.2) };
        let t = Target { z: 100.0, diff: true };
        let f = HashMap::new();
        let n = eval(&su, &d, t, &f).unwrap();
        let zd = formula_xs(&su, &with_mask(&su, &d)).zdiff.unwrap();
        assert!((n.z - zd).abs() < 1e-12);
        let tol =
            Tolerances { dielectric_thickness_pct: 0.0, er_pct: 0.0, copper_pct: 0.0, width_mm: 0.02, width_pct: None };
        let p = perturbations(&su, &d, &tol);
        let (_, _, _, _, [(_, wide), (_, narrow)]) = &p[0];
        assert_eq!((wide.w, wide.partner.unwrap().0, wide.partner.unwrap().1), (0.22, 0.13, 0.22));
        assert!((narrow.partner.unwrap().0 - 0.17).abs() < 1e-12);
        let (b, _) = band(&su, &d, t, &tol, &f, &n);
        assert!(b.unwrap().sens[0].dz < 0.0);
    }
}

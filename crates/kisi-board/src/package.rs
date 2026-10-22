//! Case size of a two-terminal chip part (resistor, capacitor, inductor): from the footprint
//! name first, from the pad geometry otherwise.
//!
//! The case size picks the package parasitics in `data/components.json` (ESL, pad
//! capacitance). Names: KiCad's library (`C_0805_2012Metric`, `R_0402_1005Metric_Pad0.72x0.64mm_HandSolder`),
//! IPC-7351 names with the metric code (`CAPC2012X90N`, `RESC1005X40N`) and plain imperial codes
//! (`GEN_C_0805`, `C0603`), the latter only when the pad pitch fits. Geometry: the nearest
//! standard land pattern by pad pitch (centre to centre) and pad width across the part's axis –
//! the pitch alone cannot tell an 0805 (KiCad: 1.9 mm) from an 0603 hand-solder pattern (1.725 mm).

use crate::board::Pad;
use crate::geom::{self, P};

/// Case sizes the parasitics table knows (larger parts: "large").
pub const CASES: &[&str] = &["0201", "0402", "0603", "0805", "1206", "1210", "large"];

/// Nominal land patterns (KiCad library, IPC-7351 nominal density): case, pad pitch (mm),
/// pad width across the axis (mm). 1812, 2010 and 2512 map to "large".
const LAND: &[(&str, f64, f64)] = &[
    ("0201", 0.64, 0.40),
    ("0402", 1.00, 0.63),
    ("0603", 1.60, 0.95),
    ("0805", 1.90, 1.43),
    ("1206", 2.95, 1.78),
    ("1210", 2.95, 2.65),
    ("1812", 4.20, 3.40),
    ("2010", 4.70, 2.60),
    ("2512", 6.00, 3.30),
];

/// Metric (IPC, KiCad "…Metric") code → imperial case code.
const METRIC: &[(&str, &str)] = &[
    ("0402", "01005"),
    ("0603", "0201"),
    ("1005", "0402"),
    ("1608", "0603"),
    ("2012", "0805"),
    ("3216", "1206"),
    ("3225", "1210"),
    ("4532", "1812"),
    ("5025", "2010"),
    ("6332", "2512"),
];

/// Imperial code → one of `CASES` (01005 counts as 0201, the big ones as "large").
fn to_case(imperial: &str) -> Option<&'static str> {
    Some(match imperial {
        "01005" | "0201" => "0201",
        "0402" => "0402",
        "0603" => "0603",
        "0805" => "0805",
        "1206" => "1206",
        "1210" => "1210",
        "1812" | "2010" | "2512" => "large",
        _ => return None,
    })
}

fn nominal_pitch(imperial: &str) -> Option<f64> {
    let case = if imperial == "01005" { "0201" } else { imperial };
    LAND.iter().find(|l| l.0 == case).map(|l| l.1)
}

/// How the case size was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Basis {
    /// KiCad library name (`<imperial>_<metric>Metric`) or IPC-7351 name (metric code)
    Name,
    /// a plain imperial code in the name, confirmed by the pad pitch
    NameAndPads,
    /// nearest standard land pattern (pitch, pad width)
    Pads,
}

/// Case size from a footprint name (`lib:name` or `name`); `pitch` (mm) confirms a plain code.
pub fn case_from_name(name: &str, pitch: Option<f64>) -> Option<(&'static str, Basis)> {
    let n = name.rsplit(':').next().unwrap_or(name);
    let b = n.as_bytes();
    let digits_at = |i: usize, k: usize| i + k <= b.len() && b[i..i + k].iter().all(u8::is_ascii_digit);
    let bounded = |i: usize, k: usize| digits_at(i, k) && (i == 0 || !(b[i - 1].is_ascii_digit() || b[i - 1] == b'.')) && (i + k == b.len() || !(b[i + k].is_ascii_digit() || b[i + k] == b'.'));
    // KiCad: "<imperial>_<metric>Metric"
    if let Some(m) = n.find("Metric") {
        if m >= 4 && digits_at(m - 4, 4) {
            let metric = &n[m - 4..m];
            if let Some((_, imp)) = METRIC.iter().find(|x| x.0 == metric) {
                if let Some(c) = to_case(imp) {
                    return Some((c, Basis::Name));
                }
            }
        }
    }
    // IPC-7351: CAPC2012X90N, RESC1005X40N, INDC1608X95N, LEDC…, DIOC…, FUSC…
    let up = n.to_ascii_uppercase();
    for pre in ["CAP", "RES", "IND", "LED", "DIO", "FUS"] {
        let mut from = 0;
        while let Some(k) = up[from..].find(pre) {
            let mut i = from + k + pre.len();
            // one or two family letters (C, M, AE, …)
            let mut letters = 0;
            while i < b.len() && b[i].is_ascii_alphabetic() && letters < 2 && !digits_at(i, 1) {
                i += 1;
                letters += 1;
            }
            if letters >= 1 && digits_at(i, 4) && up.as_bytes().get(i + 4) == Some(&b'X') {
                if let Some((_, imp)) = METRIC.iter().find(|x| x.0 == &n[i..i + 4]) {
                    if let Some(c) = to_case(imp) {
                        return Some((c, Basis::Name));
                    }
                }
            }
            from += k + pre.len();
        }
    }
    // a plain imperial code ("GEN_C_0805", "C0603", "R_0402"): only if the pads fit (a metric
    // "0603" is an imperial 0201)
    for i in 0..b.len() {
        for k in [5, 4] {
            if !bounded(i, k) {
                continue;
            }
            let code = &n[i..i + k];
            let (Some(c), Some(p0)) = (to_case(code), nominal_pitch(code)) else { continue };
            if pitch.map_or(true, |p| (0.6..=1.6).contains(&(p / p0))) {
                return Some((c, Basis::NameAndPads));
            }
        }
    }
    None
}

/// Case size from the pad geometry: pad pitch and pad width across the axis (mm).
pub fn case_from_pads(pitch: f64, across: f64) -> &'static str {
    if pitch <= 0.0 {
        return "large";
    }
    // reverse-geometry (wide-terminal) parts, e.g. 0508/0612: the current path is the short side
    if across <= 0.0 || across > 1.3 * pitch {
        return LAND.iter().take(6).min_by(|a, b| (pitch / a.1).ln().abs().total_cmp(&(pitch / b.1).ln().abs())).map(|l| to_case(l.0).unwrap()).unwrap();
    }
    let d = |l: &(&str, f64, f64)| (pitch / l.1).ln().powi(2) + (across / l.2).ln().powi(2);
    LAND.iter().min_by(|a, b| d(a).total_cmp(&d(b))).map(|l| to_case(l.0).unwrap_or("large")).unwrap_or("large")
}

/// Extent of a pad along and across the unit direction `axis` (rotated rectangle of its size).
pub fn pad_extent(p: &Pad, axis: P) -> (f64, f64) {
    let th = -p.angle.to_radians();
    let (s, c) = th.sin_cos();
    let (u, v) = ([c, s], [-s, c]);
    let perp = [-axis[1], axis[0]];
    let ext = |d: P| (p.size[0] * geom::dot(u, d)).abs() + (p.size[1] * geom::dot(v, d)).abs();
    (ext(axis), ext(perp))
}

/// Case size of a two-pad part with footprint name `name` (may be empty).
pub fn chip_case(name: &str, a: &Pad, b: &Pad) -> (&'static str, Basis) {
    let d = geom::sub(b.at, a.at);
    let pitch = geom::len(d);
    if let Some(r) = case_from_name(name, Some(pitch)) {
        return r;
    }
    let axis = if pitch > 1e-9 { geom::scale(d, 1.0 / pitch) } else { [1.0, 0.0] };
    let across = (pad_extent(a, axis).1 + pad_extent(b, axis).1) / 2.0;
    (case_from_pads(pitch, across), Basis::Pads)
}

/// Case size of a two-pad part (see `chip_case`).
pub fn chip_package(name: &str, a: &Pad, b: &Pad) -> &'static str {
    chip_case(name, a, b).0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pad(at: P, size: P, angle: f64) -> Pad {
        Pad { at, size, angle, shape: "roundrect".into(), rratio: 0.25, custom: vec![], layers: vec![0], net: 1, drill: 0.0, number: "1".into(), pinfunction: String::new(), fp: 0 }
    }

    /// Two pads of a KiCad standard land pattern (pad centres at ±x, size along × across),
    /// placed with the footprint rotated by `rot` degrees (pad angles absolute, as in the file).
    fn part(x: f64, along: f64, across: f64, rot: f64) -> (Pad, Pad) {
        let r = rot.to_radians();
        let (s, c) = r.sin_cos();
        let at = |lx: f64| [10.0 + lx * c, 20.0 - lx * s];
        (pad(at(-x), [along, across], rot), pad(at(x), [along, across], rot))
    }

    #[test]
    fn kicad_names() {
        for (name, case) in [
            ("Capacitor_SMD:C_0805_2012Metric", "0805"),
            ("Resistor_SMD:R_0402_1005Metric", "0402"),
            ("C_0603_1608Metric_Pad1.08x0.95mm_HandSolder", "0603"),
            ("L_0805_2012Metric_Pad1.15x1.40mm_HandSolder", "0805"),
            ("C_1206_3216Metric", "1206"),
            ("C_1210_3225Metric", "1210"),
            ("R_0201_0603Metric", "0201"),
            ("C_01005_0402Metric", "0201"),
            ("R_2512_6332Metric", "large"),
            ("CAPC2012X90N", "0805"),
            ("RESC1005X40N", "0402"),
            ("Capacitors:CAPC3225X270N", "1210"),
        ] {
            assert_eq!(case_from_name(name, None).map(|x| x.0), Some(case), "{name}");
        }
        // plain codes: only when the pitch fits
        assert_eq!(case_from_name("GEN_C_0805", Some(1.9)), Some(("0805", Basis::NameAndPads)));
        assert_eq!(case_from_name("C0603", Some(1.55)).map(|x| x.0), Some("0603"));
        assert_eq!(case_from_name("C_0603", Some(0.62)), None, "metric 0603 = imperial 0201: the pads decide");
        assert_eq!(case_from_name("SOT-23-5", Some(0.95)), None);
        assert_eq!(case_from_name("QFN-24-1EP_4x4mm_P0.5mm_EP2.6x2.6mm", Some(1.0)), None);
        assert_eq!(case_from_name("GEN_R_0508", Some(1.15)), None, "0508 is not a case of the table");
    }

    /// KiCad's standard land patterns (Capacitor_SMD / Resistor_SMD, nominal and hand-solder)
    /// are recognised from the pads alone, at every rotation.
    #[test]
    fn kicad_land_patterns_by_geometry() {
        let cases = [
            // (case, pad centre x, pad size along, across)
            ("0201", 0.32, 0.46, 0.40),
            ("0402", 0.48, 0.56, 0.62),
            ("0402", 0.51, 0.54, 0.64),
            ("0402", 0.5675, 0.6875, 0.62), // C_0402 hand solder
            ("0603", 0.775, 0.90, 0.95),
            ("0603", 0.825, 0.80, 0.95),
            ("0603", 0.8625, 1.075, 0.95), // hand solder: pitch 1.725
            ("0805", 0.95, 1.00, 1.45),    // C_0805_2012Metric: pitch 1.9 (was taken as 0603)
            ("0805", 0.9125, 1.025, 1.40), // R_0805
            ("0805", 1.0375, 1.175, 1.45), // hand solder
            ("1206", 1.475, 1.05, 1.75),
            ("1206", 1.4625, 1.125, 1.75),
            ("1206", 1.5625, 1.325, 1.80),
            ("1210", 1.475, 1.05, 2.65),
            ("1210", 1.4625, 1.125, 2.65),
        ];
        for (case, x, along, across) in cases {
            for rot in [0.0, 90.0, 180.0, 270.0, -90.0, 45.0] {
                let (a, b) = part(x, along, across, rot);
                assert_eq!(chip_case("", &a, &b), (case, Basis::Pads), "{case} x={x} rot={rot}");
                // and with the pads listed the other way round
                assert_eq!(chip_package("", &b, &a), case);
            }
        }
        // the name wins over the geometry, a plain code needs a fitting pitch
        let (a, b) = part(0.95, 1.0, 1.45, 90.0);
        assert_eq!(chip_case("Capacitor_SMD:C_0805_2012Metric", &a, &b), ("0805", Basis::Name));
        assert_eq!(chip_case("MyLib:C_0603", &a, &b), ("0603", Basis::NameAndPads), "a plausible plain code is trusted");
        // reverse geometry (wide terminals, 0508): by the pitch
        let (a, b) = part(0.575, 0.762, 2.25, 0.0);
        assert_eq!(chip_package("GEN_R_0508", &a, &b), "0402");
        // big parts
        let (a, b) = part(2.1, 1.4, 3.4, 0.0);
        assert_eq!(chip_package("", &a, &b), "large");
    }
}

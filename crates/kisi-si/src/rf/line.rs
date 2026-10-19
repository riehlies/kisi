//! Per-unit-length line models: materials (Djordjevic-Sarkar, Huray roughness),
//! RLGC of a cross-section from the field solver or closed-form formulas, and the
//! ABCD matrix algebra used by the channel.

use crate::analytic;
#[allow(unused_imports)]
use super::*;
use crate::board::Stackup;
use crate::extract::{CrossSection, FieldResult, Kind};
use serde::{Deserialize, Serialize};
use std::f64::consts::PI;

pub const F_REF: f64 = 1.0e9;

pub(crate) fn cosh_sinh(x: Cx) -> (Cx, Cx) {
    let e = x.exp();
    let ei = (Cx::new(-x.re, -x.im)).exp();
    ((e + ei).scale(0.5), (e - ei).scale(0.5))
}

/// ABCD matrix [a b; c d]
#[derive(Debug, Clone, Copy)]
pub struct Abcd {
    pub a: Cx,
    pub b: Cx,
    pub c: Cx,
    pub d: Cx,
}

impl Abcd {
    pub fn identity() -> Self {
        Abcd { a: Cx::new(1.0, 0.0), b: Cx::new(0.0, 0.0), c: Cx::new(0.0, 0.0), d: Cx::new(1.0, 0.0) }
    }
    pub fn mul(&self, o: &Abcd) -> Abcd {
        Abcd {
            a: self.a * o.a + self.b * o.c,
            b: self.a * o.b + self.b * o.d,
            c: self.c * o.a + self.d * o.c,
            d: self.c * o.b + self.d * o.d,
        }
    }
    /// Transmission line of length `len` (m) with RLGC at angular frequency w.
    pub fn line(r: f64, l: f64, g: f64, c: f64, w: f64, len: f64) -> Abcd {
        let z = Cx::new(r, w * l);
        let y = Cx::new(g, w * c);
        let gamma = (z * y).sqrt();
        let zc = (z / y).sqrt();
        let (ch, sh) = cosh_sinh(gamma.scale(len));
        Abcd { a: ch, b: zc * sh, c: sh / zc, d: ch }
    }
    pub fn series(z: Cx) -> Abcd {
        Abcd { a: Cx::new(1.0, 0.0), b: z, c: Cx::new(0.0, 0.0), d: Cx::new(1.0, 0.0) }
    }
    pub fn shunt(y: Cx) -> Abcd {
        Abcd { a: Cx::new(1.0, 0.0), b: Cx::new(0.0, 0.0), c: y, d: Cx::new(1.0, 0.0) }
    }
    /// S11, S21 for a real reference impedance (reciprocal, both ports).
    pub fn s_params(&self, z0: f64) -> (Cx, Cx, Cx) {
        let bz = self.b.scale(1.0 / z0);
        let cz = self.c.scale(z0);
        let den = self.a + bz + cz + self.d;
        let s11 = (self.a + bz - cz - self.d) / den;
        let s21 = Cx::new(2.0, 0.0) / den;
        let s22 = (Cx::new(0.0, 0.0) - self.a + bz - cz + self.d) / den;
        (s11, s21, s22)
    }
}

// --- materials ---------------------------------------------------------------------

/// Djordjevic-Sarkar wideband Debye model fitted to (er, tanδ) at 1 GHz,
/// poles from 10 kHz to 1 THz.
#[derive(Debug, Clone, Copy)]
pub struct DjordjevicSarkar {
    pub e_inf: f64,
    pub de: f64,
}

impl DjordjevicSarkar {
    const M1: f64 = 4.0;
    const M2: f64 = 12.0;
    fn k(f: f64) -> Cx {
        let w = 2.0 * PI * f;
        let w1 = 2.0 * PI * 10f64.powf(Self::M1);
        let w2 = 2.0 * PI * 10f64.powf(Self::M2);
        let num = Cx::new(w2, w);
        let den = Cx::new(w1, w);
        let q = num / den;
        // log10 of a complex number
        Cx::new(q.abs().ln(), q.arg()).scale(1.0 / (std::f64::consts::LN_10 * (Self::M2 - Self::M1)))
    }
    pub fn fit(er: f64, tand: f64) -> Self {
        let k = Self::k(F_REF);
        let de = if tand > 0.0 { -er * tand / k.im } else { 0.0 };
        DjordjevicSarkar { e_inf: er - de * k.re, de }
    }
    /// (ε', tanδ) at frequency f
    pub fn at(&self, f: f64) -> (f64, f64) {
        let k = Self::k(f.max(1.0));
        let re = self.e_inf + self.de * k.re;
        let im = -self.de * k.im;
        (re, if re > 0.0 { im / re } else { 0.0 })
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum Roughness {
    Smooth,
    Hvlp,
    Standard,
}

impl Roughness {
    /// Huray "cannonball" factor: 1 + 1.5·SR / (1 + δ/r + δ²/(2r²)).
    /// Preset (r, SR) are estimates for typical foils.
    pub fn factor(&self, f: f64, sigma: f64) -> f64 {
        let (r, sr) = match self {
            Roughness::Smooth => return 1.0,
            Roughness::Hvlp => (0.5e-6, 0.5),
            Roughness::Standard => (1.0e-6, 1.0),
        };
        let delta = 1.0 / (PI * f * MU0 * sigma).sqrt();
        1.0 + 1.5 * sr / (1.0 + delta / r + delta * delta / (2.0 * r * r))
    }
}

// --- parameters -------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct RfParams {
    /// upper frequency (Hz)
    pub fmax: f64,
    pub npts: usize,
    /// 10-90 % rise time for TDR and crosstalk (s)
    pub rise: f64,
    pub roughness: Roughness,
    /// copper conductivity (S/m)
    pub sigma: f64,
    /// reference impedance; None: target of the net class, else 50 / 100 Ω
    pub z_ref: Option<f64>,
    /// include the via model
    pub vias: bool,
    /// assume every via is back-drilled to this residual stub (mm); None: as in the layout
    pub backdrill: Option<f64>,
}

impl Default for RfParams {
    fn default() -> Self {
        RfParams { fmax: 25e9, npts: 500, rise: 35e-12, roughness: Roughness::Standard, sigma: 5.8e7, z_ref: None, vias: true, backdrill: None }
    }
}

// --- per-unit-length model ------------------------------------------------------------

/// Frequency-dependent line model of one cross-section (single line or the
/// odd mode of a pair, per line).
#[derive(Debug, Clone, Serialize)]
pub struct LineModel {
    pub z0: f64,
    pub eeff0: f64,
    /// external inductance (H/m) and static capacitance (F/m)
    pub l_ext: f64,
    pub c0: f64,
    pub r_dc: f64,
    /// smooth-copper AC resistance coefficient: R_ac = k_ac·√f (Ω/m/√Hz)
    pub k_ac: f64,
    /// material permittivity / loss tangent at 1 GHz and filling factor
    pub er: f64,
    pub tand: f64,
    pub q_fill: f64,
    /// dielectric loss tangent of the mode at 1 GHz when it came from a field solve
    pub tand_eff0: Option<f64>,
    /// substrate height for microstrip dispersion (m), None for stripline
    pub ms_h: Option<f64>,
    pub source: &'static str,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct Rlgc {
    pub r: f64,
    pub l: f64,
    pub g: f64,
    pub c: f64,
}

impl LineModel {
    pub fn rlgc(&self, f: f64, rp: &RfParams) -> Rlgc {
        let w = 2.0 * PI * f;
        let ds = DjordjevicSarkar::fit(self.er, self.tand);
        let (er_f, tand_f) = ds.at(f);
        let (er_0, tand_0) = ds.at(F_REF);
        // effective permittivity with material dispersion
        let ee_mat = |e: f64| if self.ms_h.is_some() { 1.0 + self.q_fill * (e - 1.0) } else { e };
        let mut eeff = self.eeff0 * ee_mat(er_f) / ee_mat(er_0);
        if let Some(h) = self.ms_h {
            // Getsinger dispersion of microstrip
            let fp = self.z0 / (2.0 * MU0 * h);
            let gg = 0.6 + 0.009 * self.z0;
            eeff = er_f - (er_f - eeff) / (1.0 + gg * (f / fp).powi(2));
        }
        let c = self.c0 * eeff / self.eeff0;
        let r_ac_smooth = self.k_ac * f.sqrt();
        let r_ac = r_ac_smooth * rp.roughness.factor(f, rp.sigma);
        let r = (self.r_dc * self.r_dc + r_ac * r_ac).sqrt();
        let l = self.l_ext + if w > 0.0 { r_ac_smooth / w } else { 0.0 };
        let tand_eff = match self.tand_eff0 {
            Some(t0) => t0 * if tand_0 > 0.0 { tand_f * er_f / (tand_0 * er_0) } else { 0.0 },
            None => tand_f * self.q_fill * er_f / eeff,
        };
        Rlgc { r, l, g: w * c * tand_eff, c }
    }
}

/// Inductance per metre of a geometry in air (formula path), for Wheeler's
/// incremental inductance rule. Lengths in mm.
pub(crate) fn l_air(kind: Kind, w: f64, t: f64, h1: f64, h2: f64, odd_gap: Option<f64>) -> f64 {
    let z = match (kind, odd_gap) {
        (Kind::Stripline, None) => analytic::stripline_asym(w, h1, h2, t, 1.0, 1.0),
        (Kind::Stripline, Some(s)) => {
            // odd mode: Cohn ratio at the plane spacing applied to the SE value
            let b = h1 + h2 + t;
            let (_, zo) = analytic::cohn_coupled(w, s, b, 1.0);
            let (_, z0c) = analytic::cohn_coupled(w, 1e3, b, 1.0);
            analytic::stripline_asym(w, h1, h2, t, 1.0, 1.0) * zo / z0c
        }
        (_, None) => analytic::microstrip(w, h1, t, 1.0).0,
        (_, Some(s)) => {
            let (_, zo, _, _) = analytic::kirschning_jansen(w, s, h1, 1.0);
            let z00 = analytic::microstrip(w, h1, 0.0, 1.0).0;
            analytic::microstrip(w, h1, t, 1.0).0 * zo / z00
        }
    };
    z / C0
}

/// Material seen by a cross-section: (er, tanδ) at 1 GHz, weighted by the
/// capacitance of the dielectric above and below.
pub(crate) fn xs_material(su: &Stackup, xs: &CrossSection) -> (f64, f64) {
    let li = xs.layer as usize;
    let tand_between = |j: usize| -> f64 {
        let (a, b) = if li < j { (li, j) } else { (j, li) };
        let (mut h, mut s) = (0.0, 0.0);
        for k in a..b {
            let d = &su.dielectrics[k];
            h += d.thickness;
            s += d.thickness * d.loss_tangent;
        }
        if h > 0.0 { s / h } else { 0.02 }
    };
    let mut parts = Vec::new();
    for (r, h, e) in [(xs.ref_up, xs.h_up, xs.er_up), (xs.ref_dn, xs.h_dn, xs.er_dn)] {
        if let (Some(j), Some(h), Some(e)) = (r, h, e) {
            parts.push((1.0 / h.max(1e-6), e, tand_between(j as usize)));
        }
    }
    let wsum: f64 = parts.iter().map(|p| p.0).sum();
    if wsum <= 0.0 {
        return (4.2, 0.02);
    }
    let er = parts.iter().map(|p| p.0 * p.1).sum::<f64>() / wsum;
    let tand = parts.iter().map(|p| p.0 * p.2).sum::<f64>() / wsum;
    (er, tand)
}

/// Line model of a cross-section. `odd`: odd mode of the diff pair.
/// Propagation mode of a line model: a single line, or one line of a coupled pair driven
/// differentially (odd) or in common (even).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Single,
    Odd,
    Even,
}

pub fn line_model(su: &Stackup, xs: &CrossSection, field: Option<&FieldResult>, odd: bool, rp: &RfParams) -> Option<LineModel> {
    line_model_mode(su, xs, field, if odd { Mode::Odd } else { Mode::Single }, rp)
}

/// Line model per mode. Odd and even values are per line of the pair (Zdiff = 2·Zodd,
/// Zcommon = Zeven/2). Without a field solve the even mode comes from the formulas
/// (Zeven ≈ Z0²/Zodd, Kirschning-Jansen permittivities): an estimate.
pub fn line_model_mode(su: &Stackup, xs: &CrossSection, field: Option<&FieldResult>, mode: Mode, rp: &RfParams) -> Option<LineModel> {
    let (er, tand) = xs_material(su, xs);
    let odd = mode == Mode::Odd;
    let (z, eeff) = match mode {
        Mode::Odd => (xs.zdiff? / 2.0, field.and_then(|f| f.eeff_odd).or(xs.eeff_odd).or(xs.eeff)?),
        Mode::Even => (xs.zeven?, field.and_then(|f| f.eeff_even).or(xs.eeff_even).or(xs.eeff)?),
        Mode::Single => (xs.z0?, xs.eeff?),
    };
    let w_mm = xs.w;
    let t_mm = xs.t;
    let (h1, h2) = match xs.kind {
        Kind::Stripline => (xs.h_dn.unwrap_or(0.1), xs.h_up.unwrap_or(0.1)),
        _ => (xs.h_dn.or(xs.h_up).unwrap_or(0.1), 0.0),
    };
    let ms_h = if xs.kind == Kind::Stripline { None } else { Some(h1 * 1e-3) };
    let q_fill = if xs.kind == Kind::Stripline || er <= 1.0 { 1.0 } else { ((eeff - 1.0) / (er - 1.0)).clamp(0.05, 1.0) };
    let r_dc = 1.0 / (rp.sigma * (w_mm * 1e-3) * (t_mm * 1e-3));

    // field solve at f0 if available (smooth copper)
    let fr = field.and_then(|f| {
        let f0 = f.f0.unwrap_or(F_REF);
        match mode {
            Mode::Odd => Some((f.r_odd?, f.l_odd?, f.g_odd, f.c_odd?, f0)),
            Mode::Even => Some((f.r_even?, f.l_even?, f.g_even, f.c_even?, f0)),
            Mode::Single => Some((f.r?, f.l?, f.g, f.c?, f0)),
        }
    });
    if let Some((r0, l0, g0, c0f, f0)) = fr {
        let w0 = 2.0 * PI * f0;
        let r_ac0 = (r0 * r0 - r_dc * r_dc).max(0.0).sqrt();
        let l_ext = (l0 - r_ac0 / w0).max(l0 * 0.5);
        return Some(LineModel {
            z0: (l_ext / c0f).sqrt(),
            eeff0: l_ext * c0f * C0 * C0,
            l_ext,
            c0: c0f,
            r_dc,
            k_ac: r_ac0 / f0.sqrt(),
            er,
            tand,
            q_fill,
            tand_eff0: g0.map(|g| g / (w0 * c0f)),
            ms_h,
            source: "field",
        });
    }

    // formula path: Wheeler's incremental inductance for the AC resistance (even mode:
    // the single-line value, the currents of an even pair crowd less than odd ones)
    let dn = 1e-3; // mm (1 µm recession)
    let gap = if odd { xs.diff_gap } else { None };
    let la = l_air(xs.kind, w_mm, t_mm, h1, h2, gap);
    let lb = l_air(xs.kind, w_mm - 2.0 * dn, (t_mm - 2.0 * dn).max(1e-4), h1 + 2.0 * dn, h2 + 2.0 * dn, gap.map(|s| s + 2.0 * dn));
    let dl_dn = ((lb - la) / (dn * 1e-3)).max(0.0);
    let k_ac = (PI * MU0 / rp.sigma).sqrt() / MU0 * dl_dn;
    let l_ext = z * eeff.sqrt() / C0;
    let c0 = eeff.sqrt() / (z * C0);
    Some(LineModel { z0: z, eeff0: eeff, l_ext, c0, r_dc, k_ac, er, tand, q_fill, tand_eff0: None, ms_h, source: "formula" })
}


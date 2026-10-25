//! Per-unit-length line models: materials (Djordjevic-Sarkar, Huray roughness),
//! RLGC of a cross-section from the field solver or closed-form formulas, and the
//! ABCD matrix algebra used by the channel.
//!
//! Causal and passive by construction: the series impedance and the shunt admittance per metre
//! are positive-real functions of s = jω, each a product or sum of terms that are –
//! * the conductor: R_dc·√(1 + jf/f_c) (the DC resistance turning into the skin effect, internal
//!   inductance included) times the causal Huray factor (Bracken 2012), [`conductor_z`];
//! * the dielectric: the mode's complex effective permittivity A + B·k(f) with the
//!   Djordjevic-Sarkar kernel k (ε′ and ε″ from one wideband Debye distribution), plus for
//!   microstrip the dispersion as a Lorentz term with its resonance above the band,
//!   [`LineModel::eps_eff`].
//!
//! The quasi-TEM microstrip dispersion (Getsinger: ε_eff rising towards εr) cannot be followed
//! exactly by a passive shunt admittance – Kramers-Kronig gives C(0) ≥ C(∞) for any passive one
//! – only below a resonance; the Lorentz term matches Getsinger at the top of the band and keeps
//! its resonance at fp/√G (Getsinger's corner) or at least at twice the band.

use super::*;
use crate::analytic;
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
    /// the kernel log10((ω2 + s)/(ω1 + s))/(M2 − M1) at complex frequency s: a causal, passive
    /// relaxation (a uniform distribution of Debye poles from 10^M1 to 10^M2 Hz), 1 at 0 Hz and 0
    /// at infinity, analytic in the right half plane
    pub fn k_s(s: Cx) -> Cx {
        let w1 = 2.0 * PI * 10f64.powf(Self::M1);
        let w2 = 2.0 * PI * 10f64.powf(Self::M2);
        let q = (s + Cx::new(w2, 0.0)) / (s + Cx::new(w1, 0.0));
        // log10 of a complex number
        Cx::new(q.abs().ln(), q.arg()).scale(1.0 / (std::f64::consts::LN_10 * (Self::M2 - Self::M1)))
    }
    /// the kernel at f (s = j2πf)
    pub fn k(f: f64) -> Cx {
        Self::k_s(Cx::new(0.0, 2.0 * PI * f))
    }
    pub fn fit(er: f64, tand: f64) -> Self {
        let k = Self::k(F_REF);
        let de = if tand > 0.0 { -er * tand / k.im } else { 0.0 };
        DjordjevicSarkar { e_inf: er - de * k.re, de }
    }
    /// complex permittivity ε′ − jε″ at f
    pub fn eps(&self, f: f64) -> Cx {
        Cx::new(self.e_inf, 0.0) + Self::k(f.max(1.0)).scale(self.de)
    }
    /// (ε', tanδ) at frequency f
    pub fn at(&self, f: f64) -> (f64, f64) {
        let e = self.eps(f);
        (e.re, if e.re > 0.0 { -e.im / e.re } else { 0.0 })
    }
}

/// Complex relative permittivity ε′ − jε″ at f of a material given by (εr, tanδ) at 1 GHz
/// (Djordjevic-Sarkar): what every model of kisi uses for a dielectric – lines, via ladders,
/// coaxial via sections.
pub fn eps_c(er: f64, tand: f64, f: f64) -> Cx {
    DjordjevicSarkar::fit(er, tand).eps(f)
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum Roughness {
    Smooth,
    Hvlp,
    Standard,
}

impl Roughness {
    /// Huray "cannonball" (r, SR): sphere radius (m) and the spheres' surface per flat area
    /// (4πr²N/A). Preset values are estimates for typical foils.
    fn huray(&self) -> Option<(f64, f64)> {
        match self {
            Roughness::Smooth => None,
            Roughness::Hvlp => Some((0.5e-6, 0.5)),
            Roughness::Standard => Some((1.0e-6, 1.0)),
        }
    }

    /// Causal Huray factor of the surface impedance at complex frequency s (E. Bracken, "A causal
    /// Huray model for surface roughness", DesignCon 2012): K = 1 + 1.5·SR / (1 + 1/(r·√(sμσ))),
    /// at s = jω 1 + 1.5·SR / (1 + δ/((1 + j)·r)). Its loss part Re[(1 + j)K] is the Huray power
    /// factor 1 + 1.5·SR / (1 + δ/r + δ²/(2r²)); its imaginary part is the roughness's internal
    /// inductance that Kramers-Kronig requires.
    pub fn k_s(&self, s: Cx, sigma: f64) -> Cx {
        let Some((r, sr)) = self.huray() else { return Cx::new(1.0, 0.0) };
        let p = s.scale(MU0 * sigma).sqrt().scale(r);
        Cx::new(1.0, 0.0) + Cx::new(1.5 * sr, 0.0) / (Cx::new(1.0, 0.0) + p.inv())
    }

    /// the causal Huray factor at f
    pub fn k(&self, f: f64, sigma: f64) -> Cx {
        if f <= 0.0 {
            return Cx::new(1.0, 0.0);
        }
        self.k_s(Cx::new(0.0, 2.0 * PI * f), sigma)
    }

    /// The Huray loss factor (real): the resistance of rough over smooth copper at f.
    pub fn factor(&self, f: f64, sigma: f64) -> f64 {
        let k = self.k(f, sigma);
        k.re - k.im
    }
}

/// Series impedance of the conductors per metre (Ω/m) at complex frequency s:
/// R_dc·√(1 + s/ω_c)·K(s) – the DC resistance turning into the skin effect (1 + j)·k_ac·√f above
/// f_c = ω_c/2π = R_dc²/(2k_ac²), with its internal inductance, times the causal roughness factor
/// K. Each factor maps the right half of the s plane into |arg| < π/4, so their product has a
/// positive real part there: passive and causal. (Before: R = √(R_dc² + (K_Huray·k_ac√f)²),
/// L = L_ext + k_ac√f/ω – a resistance without its inductance.)
pub fn conductor_zs(r_dc: f64, k_ac: f64, s: Cx, rough: Roughness, sigma: f64) -> Cx {
    let smooth = if k_ac > 0.0 && r_dc > 0.0 {
        let wc = PI * r_dc * r_dc / (k_ac * k_ac);
        (Cx::new(1.0, 0.0) + s.scale(1.0 / wc)).sqrt().scale(r_dc)
    } else if k_ac > 0.0 {
        s.scale(1.0 / PI).sqrt().scale(k_ac)
    } else {
        Cx::new(r_dc, 0.0)
    };
    smooth * rough.k_s(s, sigma)
}

/// [`conductor_zs`] at f.
pub fn conductor_z(r_dc: f64, k_ac: f64, f: f64, rough: Roughness, sigma: f64) -> Cx {
    if f <= 0.0 {
        return Cx::new(r_dc, 0.0);
    }
    conductor_zs(r_dc, k_ac, Cx::new(0.0, 2.0 * PI * f), rough, sigma)
}

/// k_ac (Ω/m/√Hz) such that the smooth part of [`conductor_z`] has the resistance `r0` at `f0`:
/// Re √(1 + jx) = √((√(1 + x²) + 1)/2) = r0/R_dc.
fn k_ac_for(r_dc: f64, r0: f64, f0: f64) -> f64 {
    let rho = (r0 / r_dc).max(1.0 + 1e-9);
    let x = ((2.0 * rho * rho - 1.0).powi(2) - 1.0).sqrt();
    let fc = f0 / x;
    r_dc / (2.0 * fc).sqrt()
}

/// The complex effective permittivity of a line mode: ε(s) = A + B·k(s) + Δ·(1/(1 + s²/ω_r²) − 1)
/// (Djordjevic-Sarkar kernel k; the Lorentz term only for microstrip). The shunt admittance
/// s·C_air·ε(s) is positive-real when B ≥ 0, Δ ≥ 0 and ε(∞) = A − Δ ≥ 0 – kept at ≥ 1 here.
#[derive(Debug, Clone, Copy)]
pub struct EpsModel {
    pub a: f64,
    pub b: f64,
    /// Lorentz term: amplitude and resonance (rad/s); Δ = 0: none
    pub delta: f64,
    pub wr: f64,
}

impl EpsModel {
    /// at complex frequency s
    pub fn at_s(&self, s: Cx) -> Cx {
        let mut e = Cx::new(self.a, 0.0) + DjordjevicSarkar::k_s(s).scale(self.b);
        if self.delta > 0.0 {
            let q = Cx::new(1.0, 0.0) + (s * s).scale(1.0 / (self.wr * self.wr));
            e = e + (q.inv() - Cx::new(1.0, 0.0)).scale(self.delta);
        }
        e
    }
    /// at f; the Lorentz term held above 0.9·f_r (outside the band it is fitted to)
    pub fn at(&self, f: f64) -> Cx {
        let w = 2.0 * PI * f.max(1.0);
        let mut e = Cx::new(self.a, 0.0) + DjordjevicSarkar::k_s(Cx::new(0.0, w)).scale(self.b);
        if self.delta > 0.0 {
            let x = (w / self.wr).min(0.9);
            e = e + Cx::new(self.delta * x * x / (1.0 - x * x), 0.0);
        }
        e
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
    /// plane pairs of the via ladder as finite cavities (`via_cavity`, validation/fdtd/
    /// BOARD_RESONANCES.md) instead of infinite plates; off by default
    pub via_cavity: bool,
    /// return groups per plane pair in the via ladder (`via::ViaCtx::returns`, validation/fdtd/
    /// BOARD_RETURNS.md)
    pub via_returns: bool,
}

impl Default for RfParams {
    fn default() -> Self {
        RfParams {
            fmax: 25e9,
            npts: 500,
            rise: 35e-12,
            roughness: Roughness::Standard,
            sigma: 5.8e7,
            z_ref: None,
            vias: true,
            backdrill: None,
            via_cavity: false,
            via_returns: false,
        }
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
        let zc = conductor_z(self.r_dc, self.k_ac, f, rp.roughness, rp.sigma);
        let e = self.eps_eff(f, rp);
        let c_air = self.c0 / self.eeff0;
        Rlgc { r: zc.re, l: self.l_ext + if w > 0.0 { zc.im / w } else { 0.0 }, g: -w * c_air * e.im, c: c_air * e.re }
    }

    /// Complex effective permittivity of the mode at f (ε′ − jε″; the shunt admittance per metre
    /// is jω·c0/εeff0·ε_eff), see [`eps_model`](Self::eps_model).
    pub fn eps_eff(&self, f: f64, rp: &RfParams) -> Cx {
        self.eps_model(rp).at(f)
    }

    /// The mode's permittivity as an [`EpsModel`]: A + B·k with the Djordjevic-Sarkar kernel k – B
    /// from the mode's loss tangent at 1 GHz (the field solve's, else the material's times the
    /// filling factor), A from εeff0 –, so that ε′ follows ε″ as Kramers-Kronig demands. Microstrip
    /// adds its dispersion as a lossless Lorentz term Δ·x²/(1 − x²), x = f/f_r (at real
    /// frequencies): the rise of ε_eff below a resonance f_r = max(fp/√G, 2·fmax) (Getsinger's
    /// fp = Z0/(2μ0h), G = 0.6 + 0.009·Z0), Δ such that it equals Getsinger's rise
    /// (εr − εeff0)·G(f/fp)²/(1 + G(f/fp)²) at fmax, at most A − 1. Below the band it follows
    /// Getsinger within a few percent of the (small) rise where f_r = fp/√G lies far above the band
    /// – thin dielectrics; a thick one (fp inside the band) gets the rise at the top of the band
    /// and less below it.
    pub fn eps_model(&self, rp: &RfParams) -> EpsModel {
        let ds = DjordjevicSarkar::fit(self.er, self.tand);
        let k0 = DjordjevicSarkar::k(F_REF);
        let er0 = ds.eps(F_REF).re;
        let t0 = match self.tand_eff0 {
            Some(t) => t.max(0.0),
            None => self.tand * self.q_fill * er0 / self.eeff0,
        };
        let b = if k0.im < 0.0 { -self.eeff0 * t0 / k0.im } else { 0.0 };
        let a = self.eeff0 - b * k0.re;
        let mut m = EpsModel { a, b, delta: 0.0, wr: 1.0 };
        let rise = er0 - self.eeff0;
        if let (Some(h), true) = (self.ms_h, rise > 0.0 && rp.fmax > 0.0) {
            let fp = self.z0 / (2.0 * MU0 * h);
            let gg = 0.6 + 0.009 * self.z0;
            let u = gg * (rp.fmax / fp).powi(2);
            let fr = (fp / gg.sqrt()).max(2.0 * rp.fmax);
            let xt = rp.fmax / fr;
            m.delta = (rise * u / (1.0 + u) * (1.0 - xt * xt) / (xt * xt)).min(a - 1.0).max(0.0);
            m.wr = 2.0 * PI * fr;
        }
        m
    }

    /// Series impedance per metre at complex frequency s: s·L_ext + the conductors' impedance.
    pub fn z_s(&self, s: Cx, rp: &RfParams) -> Cx {
        s.scale(self.l_ext) + conductor_zs(self.r_dc, self.k_ac, s, rp.roughness, rp.sigma)
    }

    /// Shunt admittance per metre at complex frequency s: s·C_air·ε(s).
    pub fn y_s(&self, s: Cx, rp: &RfParams) -> Cx {
        s.scale(self.c0 / self.eeff0) * self.eps_model(rp).at_s(s)
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
        if h > 0.0 {
            s / h
        } else {
            0.02
        }
    };
    let mut parts = Vec::new();
    for (r, h, e) in [(xs.ref_up, xs.h_up, xs.er_up), (xs.ref_dn, xs.h_dn, xs.er_dn)] {
        if let (Some(j), Some(h), Some(e)) = (r, h, e) {
            parts.push((1.0 / h.max(1e-6), e, tand_between(j as usize)));
        }
    }
    let wsum: f64 = parts.iter().map(|p| p.0).sum();
    if wsum <= 0.0 {
        // no plane: the dielectric next to the trace
        let k = if li == 0 { 0 } else { li - 1 };
        return su.dielectrics.get(k).map(|d| (d.epsilon_r, d.loss_tangent)).unwrap_or((4.2, 0.02));
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

pub fn line_model(
    su: &Stackup,
    xs: &CrossSection,
    field: Option<&FieldResult>,
    odd: bool,
    rp: &RfParams,
) -> Option<LineModel> {
    line_model_mode(su, xs, field, if odd { Mode::Odd } else { Mode::Single }, rp)
}

/// Line model per mode. Odd and even values are per line of the pair (Zdiff = 2·Zodd,
/// Zcommon = Zeven/2). Without a field solve the even mode comes from the formulas
/// (Zeven ≈ Z0²/Zodd, Kirschning-Jansen permittivities): an estimate.
pub fn line_model_mode(
    su: &Stackup,
    xs: &CrossSection,
    field: Option<&FieldResult>,
    mode: Mode,
    rp: &RfParams,
) -> Option<LineModel> {
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
        // no plane: the current crowds towards the nearest coplanar return (for Wheeler's rule a
        // microstrip at that distance; an estimate)
        Kind::NoReference => {
            ([xs.s_left, xs.s_right, xs.diff_gap].into_iter().flatten().fold(f64::INFINITY, f64::min).min(0.5), 0.0)
        }
        _ => (xs.h_dn.or(xs.h_up).unwrap_or(0.1), 0.0),
    };
    let ms_h = if matches!(xs.kind, Kind::Stripline | Kind::NoReference) { None } else { Some(h1 * 1e-3) };
    let q_fill =
        if xs.kind == Kind::Stripline || er <= 1.0 { 1.0 } else { ((eeff - 1.0) / (er - 1.0)).clamp(0.05, 1.0) };
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
        // the conductor model's smooth part through the field solve's resistance (smooth copper),
        // its internal inductance taken out of the solve's inductance
        let k_ac = if r0 > r_dc { k_ac_for(r_dc, r0, f0) } else { 0.0 };
        let l_int0 = conductor_z(r_dc, k_ac, f0, Roughness::Smooth, rp.sigma).im / w0;
        let l_ext = (l0 - l_int0).max(l0 * 0.5);
        return Some(LineModel {
            z0: (l_ext / c0f).sqrt(),
            eeff0: l_ext * c0f * C0 * C0,
            l_ext,
            c0: c0f,
            r_dc,
            k_ac,
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
    let lb = l_air(
        xs.kind,
        w_mm - 2.0 * dn,
        (t_mm - 2.0 * dn).max(1e-4),
        h1 + 2.0 * dn,
        h2 + 2.0 * dn,
        gap.map(|s| s + 2.0 * dn),
    );
    let dl_dn = ((lb - la) / (dn * 1e-3)).max(0.0);
    let k_ac = (PI * MU0 / rp.sigma).sqrt() / MU0 * dl_dn;
    let l_ext = z * eeff.sqrt() / C0;
    let c0 = eeff.sqrt() / (z * C0);
    Some(LineModel {
        z0: z,
        eeff0: eeff,
        l_ext,
        c0,
        r_dc,
        k_ac,
        er,
        tand,
        q_fill,
        tand_eff0: None,
        ms_h,
        source: "formula",
    })
}

//! Closed-form transmission line formulas (milestone M1).
//!
//! These are the classic quasi-static approximations used by most impedance
//! calculators. They are a stop-gap until the 2D field solver (M2) replaces
//! them; typical accuracy is 1–3 % for single-ended microstrip/stripline and
//! roughly 5–10 % for the differential approximations.
//!
//! All lengths in the same unit (mm), impedances in ohms.

use std::f64::consts::{E, PI};

pub const ETA0: f64 = 376.730_313_668;

// ---------------------------------------------------------------------------
// Microstrip – Hammerstad & Jensen (1980) incl. finite strip thickness
// ---------------------------------------------------------------------------

fn hj_z01(u: f64) -> f64 {
    let f = 6.0 + (2.0 * PI - 6.0) * (-(30.666 / u).powf(0.7528)).exp();
    ETA0 / (2.0 * PI) * (f / u + (1.0 + 4.0 / (u * u)).sqrt()).ln()
}

fn hj_eeff(u: f64, er: f64) -> f64 {
    let a = 1.0
        + (1.0 / 49.0) * ((u.powi(4) + (u / 52.0).powi(2)) / (u.powi(4) + 0.432)).ln()
        + (1.0 / 18.7) * (1.0 + (u / 18.1).powi(3)).ln();
    let b = 0.564 * ((er - 0.9) / (er + 3.0)).powf(0.053);
    (er + 1.0) / 2.0 + (er - 1.0) / 2.0 * (1.0 + 10.0 / u).powf(-a * b)
}

/// Returns (Z0, eps_eff) of a surface microstrip.
pub fn microstrip(w: f64, h: f64, t: f64, er: f64) -> (f64, f64) {
    let u = w / h;
    let (u1, ur) = if t > 0.0 {
        let th = t / h;
        let coth = |x: f64| 1.0 / x.tanh();
        let du1 = th / PI * (1.0 + 4.0 * E / (th * coth((6.517 * u).sqrt()).powi(2))).ln();
        let dur = 0.5 * (1.0 + 1.0 / (er - 1.0).max(0.0).sqrt().cosh()) * du1;
        (u + du1, u + dur)
    } else {
        (u, u)
    };
    let ee_r = hj_eeff(ur, er);
    let z = hj_z01(ur) / ee_r.sqrt();
    let eeff = ee_r * (hj_z01(u1) / hj_z01(ur)).powi(2);
    (z, eeff)
}

/// Microstrip covered by solder mask: simple correction of eps_eff.
/// The mask (thickness `tm`, permittivity `em`) replaces part of the air
/// above the line. We interpolate eps_eff between the uncoated value and
/// the value for an infinitely thick coating of permittivity em, with the
/// filling factor of the coating approximated by 1-exp(-k*tm/w).
/// Good to roughly ±1 Ω for typical masks; replaced by the field solver in M2.
pub fn microstrip_masked(w: f64, h: f64, t: f64, er: f64, tm: f64, em: f64) -> (f64, f64) {
    let (z_air_line, _) = microstrip(w, h, t, 1.0);
    let (z, eeff) = microstrip(w, h, t, er);
    if tm <= 0.0 || em <= 1.0 {
        return (z, eeff);
    }
    // filling factor of the substrate: q = (eeff-1)/(er-1)
    let q = if er > 1.0 { (eeff - 1.0) / (er - 1.0) } else { 0.5 };
    // fraction of the remaining (air) field that is inside the coating
    let fill = 1.0 - (-2.0 * tm / (w + t).max(1e-9)).exp();
    let fill = fill.clamp(0.0, 1.0);
    let eeff_c = 1.0 + q * (er - 1.0) + (1.0 - q) * fill * (em - 1.0);
    let _ = z;
    (z_air_line / eeff_c.sqrt(), eeff_c)
}

// ---------------------------------------------------------------------------
// Stripline – Wheeler (1978) / Cohn, finite thickness, centred
// ---------------------------------------------------------------------------

/// Centred stripline between planes `b` apart (b includes the strip thickness).
pub fn stripline_sym(w: f64, b: f64, t: f64, er: f64) -> f64 {
    let x = (t / b).min(0.95);
    let wb = w / b;
    let m = 2.0 / (1.0 + (2.0 / 3.0) * x / (1.0 - x));
    let dw = if x > 0.0 {
        x / (PI * (1.0 - x)) * (1.0 - 0.5 * ((x / (2.0 - x)).powi(2) + (0.0796 * x / (wb + 1.1 * x)).powf(m)).ln())
    } else {
        0.0
    };
    let wp = w / (b - t) + dw; // W'/(b-t)
    let r = 8.0 / (PI * wp);
    ETA0 / (4.0 * PI * er.sqrt()) * (1.0 + 4.0 / (PI * wp) * (r + (r * r + 6.27).sqrt())).ln()
}

/// Embedded (buried) microstrip: strip at height h above the plane, covered
/// by `d` of dielectric (IPC-2141 style permittivity correction).
pub fn embedded_microstrip(w: f64, h: f64, t: f64, d: f64, er: f64) -> (f64, f64) {
    let (z_air, _) = microstrip(w, h, t, 1.0);
    let (_, eeff_surface) = microstrip(w, h, t, er);
    // blend between surface microstrip and fully embedded (eeff -> er)
    let k = 1.0 - (-1.55 * (d + t) / h).exp();
    let eeff = eeff_surface + (er - eeff_surface) * k;
    (z_air / eeff.sqrt(), eeff)
}

// ---------------------------------------------------------------------------
// Grounded coplanar waveguide (conformal mapping, zero thickness)
// ---------------------------------------------------------------------------

/// Complete elliptic integral of the first kind K(k) via AGM.
pub fn ellip_k(k: f64) -> f64 {
    let kp = (1.0 - k * k).max(0.0).sqrt();
    let (mut a, mut b) = (1.0f64, kp);
    for _ in 0..40 {
        let an = 0.5 * (a + b);
        let bn = (a * b).sqrt();
        a = an;
        b = bn;
        if (a - b).abs() < 1e-15 {
            break;
        }
    }
    PI / (2.0 * a)
}

/// K(k)/K(k')
pub fn k_ratio(k: f64) -> f64 {
    let kp = (1.0 - k * k).max(0.0).sqrt();
    ellip_k(k) / ellip_k(kp)
}

/// Grounded CPW, strip w, gap s to infinite coplanar grounds, substrate h.
/// Returns (Z0, eps_eff) for zero strip thickness.
pub fn cpwg(w: f64, s: f64, h: f64, er: f64) -> (f64, f64) {
    let a = w;
    let b = w + 2.0 * s;
    let k = a / b;
    let k3 = (PI * a / (4.0 * h)).tanh() / (PI * b / (4.0 * h)).tanh();
    let r1 = k_ratio(k); // K(k)/K(k')
    let r3 = k_ratio(k3);
    let q = (1.0 / r1) * r3; // (K(k')/K(k)) * (K(k3)/K(k3'))
    let eeff = (1.0 + er * q) / (1.0 + q);
    let z = 60.0 * PI / eeff.sqrt() / (r1 + r3);
    (z, eeff)
}

/// Coplanar waveguide without a ground plane (conformal mapping; Simons, Coplanar Waveguide Circuits,
/// Components, and Systems, 2001, ch. 2): strip `w`, gaps `s` to coplanar grounds, copper thickness `t`,
/// on a dielectric slab of thickness `h` (εr) with air on both sides – `h` = ∞ (`f64::INFINITY`): fully
/// embedded. The thickness adds the two slots' capacitance ([`slot_cap`]; in air on a slab, in εr when
/// embedded). Returns (Z0, εeff).
pub fn cpw(w: f64, s: f64, h: f64, t: f64, er: f64) -> (f64, f64) {
    let k0 = w / (w + 2.0 * s);
    // C/ε0 of the zero-thickness line in air (both half-spaces) and with the slab
    let ca0 = 4.0 * k_ratio(k0);
    let (c0, e_slot) = if h.is_finite() {
        let k1 = (PI * w / (4.0 * h)).sinh() / (PI * (w + 2.0 * s) / (4.0 * h)).sinh();
        (ca0 + 2.0 * (er - 1.0) * k_ratio(k1), 1.0)
    } else {
        (er * ca0, er)
    };
    let cs = 2.0 * slot_cap(t, s);
    z_of_caps(c0 + e_slot * cs, ca0 + cs)
}

/// Coplanar strips (two strips of width `w`, gap `s`, thickness `t`, no ground; Simons ch. 7) on a slab
/// `h` (εr) as in [`cpw`]. Returns (Zdiff, εeff).
pub fn cps(w: f64, s: f64, h: f64, t: f64, er: f64) -> (f64, f64) {
    let k = s / (s + 2.0 * w);
    // C/ε0 between the strips: K(k')/K(k) in air (both half-spaces)
    let ca0 = 1.0 / k_ratio(k);
    let (c0, e_slot) = if h.is_finite() {
        let k1 = (PI * s / (4.0 * h)).sinh() / (PI * (s + 2.0 * w) / (4.0 * h)).sinh();
        (ca0 * (1.0 + 0.5 * (er - 1.0) * k_ratio(k1) / k_ratio(k)), 1.0)
    } else {
        (er * ca0, er)
    };
    let cs = slot_cap(t, s);
    z_of_caps(c0 + e_slot * cs, ca0 + cs)
}

// ---------------------------------------------------------------------------
// Coplanar lines with a ground plane: partial capacitances
// ---------------------------------------------------------------------------
//
// A line with coplanar ground (grounded CPW, its coupled pair, a stripline with coplanar ground) as
// partial capacitances per unit length, normalised by ε0 (C/ε0; Z = η0/√(C·C_air), εeff = C/C_air):
// * each region above and below the conductor plane from the conformal mapping of the zero-thickness
//   line (Ghione & Naldi, "Coplanar waveguides for MMIC applications: effect of upper shielding,
//   conductor backing, finite-extent ground planes, and line-to-line coupling", IEEE Trans. MTT 35,
//   1987; Simons, Coplanar Waveguide Circuits, Components, and Systems, 2001, ch. 2, 3 and 7): open air
//   K(k)/K(k′) with k = w/(w + 2s), a dielectric of height h on a plane with
//   k = tanh(πw/4h)/tanh(π(w + 2s)/4h), a solder-mask cover layer with the sinh modulus (Simons ch. 2);
// * the copper thickness as the slots between the conductors' side walls ([`slot_cap`]);
// * solder mask (kisi's geometry: a layer of thickness tm on the substrate, the trace coated by tm,
//   the coplanar copper bare) in the slots as a series/parallel mix and above the copper as half a
//   cover layer of thickness tm (the trace's coating) plus a full one where tm exceeds t;
// * for wide gaps the partial-capacitance split fails (the field reaches the plane around the slot):
//   each capacitance is the smooth maximum ([`smooth_max`]) of the coplanar value and the value
//   without coplanar copper (kisi's microstrip / stripline formulas), so the formula joins the plain
//   line smoothly. The extraction looks for coplanar copper up to 3 h (at most 2 mm) beside the trace,
//   as for the field solver's geometry; there the formula is 0.4–1.7 % below the plain line (CST: about
//   0.5 % at 4 h, 1 % at 2 h), so leaving copper beyond that out changes Z by about 1 % at most.
// The slot corner term and the smoothing width are fitted to kisi's field solver on 1 100 synthetic
// cross-sections (grounded CPW with and without mask, stripline with coplanar ground, coupled grounded
// CPW, CPW without a plane; h 0.1–1.6 mm, εr 3–10, t 17.5–70 µm, w/h 0.5–4, gap/h 0.1–4):
// validation/cst_xs/fit_coplanar.py, the sweep stored in validation/cst_xs/coplanar_fit.json (the fit
// re-runs from it with --from-json); checked on the CST cases of validation/cst_xs (README.md there).

/// Corner term of a slot between two copper side walls (C/ε0 = t/s + SLOT_CORNER·(t/s)^SLOT_CORNER_EXP):
/// fitted to kisi's field solver (validation/cst_xs/fit_coplanar.py on coplanar_fit.json, the sweep of
/// 2026-11-03; `--from-json` reproduces these values; the same term brings the no-plane CPW of
/// validation/cst_xs from +6…+16 % to within 0.5 % of CST).
const SLOT_CORNER: f64 = 0.067;
const SLOT_CORNER_EXP: f64 = 0.090;
/// Width of the smooth maximum between the coplanar and the plain-line capacitance (relative; the same
/// for C and C_air, so a homogeneous dielectric keeps εeff = εr): fitted to the same field solves
/// (fit_coplanar.py, sweep of 2026-11-03).
const TAU: f64 = 0.045;

/// Capacitance (C/ε0, per unit length, without the dielectric) of one slot of width `s` between copper
/// side walls of height `t`: the parallel-plate part t/s and the corners.
pub fn slot_cap(t: f64, s: f64) -> f64 {
    if t <= 0.0 || s <= 0.0 {
        return 0.0;
    }
    let r = t / s;
    r + SLOT_CORNER * r.powf(SLOT_CORNER_EXP)
}

/// (Z, εeff) from the capacitances C/ε0 and C_air/ε0.
pub fn z_of_caps(c: f64, ca: f64) -> (f64, f64) {
    (ETA0 / (c * ca).sqrt(), c / ca)
}

/// (C/ε0, C_air/ε0) of a line with impedance `z` and effective permittivity `eeff`.
pub fn caps_of_z(z: f64, eeff: f64) -> (f64, f64) {
    let ca = ETA0 / (z * eeff.sqrt());
    (eeff * ca, ca)
}

/// Smooth maximum of `a` and `b` (> 0) with relative width `tau`: b·(1 + τ·ln(1 + e^((a/b − 1)/τ))) –
/// a where a ≫ b, b where a ≪ b, slightly above both where they are equal.
pub fn smooth_max(a: f64, b: f64, tau: f64) -> f64 {
    let x = (a / b - 1.0) / tau;
    let sp = if x > 30.0 { x + (-x).exp().ln_1p() } else { x.exp().ln_1p() };
    b * (1.0 + tau * sp)
}

/// One open half-space above a symmetric coplanar line (strip `w`, both gaps `s`): C/ε0 of the strip.
fn open_cap(w: f64, s: f64) -> f64 {
    2.0 * k_ratio(w / (w + 2.0 * s))
}

/// A dielectric of height `h` between the coplanar plane and a ground plane (εr = 1): C/ε0 of the strip.
fn slab_cap(w: f64, s: f64, h: f64) -> f64 {
    2.0 * k_ratio((PI * w / (4.0 * h)).tanh() / (PI * (w + 2.0 * s) / (4.0 * h)).tanh())
}

/// A cover layer of thickness `d` over the coplanar plane (partial capacitance per unit εr − 1).
fn cover_cap(w: f64, s: f64, d: f64) -> f64 {
    if d <= 0.0 {
        return 0.0;
    }
    let (a, b) = (PI * w / (4.0 * d), PI * (w + 2.0 * s) / (4.0 * d));
    // sinh(a)/sinh(b) without overflow for thin layers
    let k = if b > 300.0 { (a - b).exp() } else { a.sinh() / b.sinh() };
    2.0 * k_ratio(k)
}

/// Solder mask (`tm`, `em`) on a slot of width `s` next to a strip `w` of thickness `t`: (factor on the
/// open half-space's capacitance, permittivity of the slot).
fn mask_on_slot(w: f64, s: f64, t: f64, mask: Option<(f64, f64)>) -> (f64, f64) {
    let Some((tm, em)) = mask.filter(|m| m.0 > 0.0 && m.1 > 1.0) else {
        return (1.0, 1.0);
    };
    // slot: the bottom tm filled (in parallel with the air above it), in series with the trace's
    // coating of width tm along its side wall
    let e_slot = if tm >= s || t <= 0.0 {
        em
    } else {
        let e_mix = 1.0 + (em - 1.0) * (tm / t).min(1.0);
        1.0 / (tm / s / em + (1.0 - tm / s) / e_mix)
    };
    // above the copper: the trace's coating covers half of the slots' mouths; a mask thicker than the
    // copper covers them fully by tm − t
    let cu = open_cap(w, s);
    let cu_m = (cu + (em - 1.0) * (0.5 * cover_cap(w, s, tm) + 0.5 * cover_cap(w, s, tm - t))).min(em * cu);
    (cu_m / cu, e_slot)
}

/// Symmetric grounded CPW (strip `w`, both gaps `s`, substrate `h`/`er`, thickness `t`, optional mask):
/// (C/ε0, C_air/ε0).
fn cpwg_caps(w: f64, s: f64, h: f64, t: f64, er: f64, mask: Option<(f64, f64)>) -> (f64, f64) {
    let cl = slab_cap(w, s, h);
    let cu = open_cap(w, s);
    let cs = 2.0 * slot_cap(t, s);
    let (f_up, e_slot) = mask_on_slot(w, s, t, mask);
    (er * cl + f_up * cu + e_slot * cs, cl + cu + cs)
}

/// Combines the two sides of a line (each the symmetric structure with that side's gap, None = no
/// coplanar copper: the plain line `plain`) as the mean of their capacitances, each the smooth
/// maximum of the coplanar and the plain value.
fn combine_sides(plain: (f64, f64), sides: [Option<(f64, f64)>; 2]) -> (f64, f64) {
    let mut c = 0.0;
    let mut ca = 0.0;
    for s in sides {
        let (sc, sca) = s.map_or(plain, |(sc, sca)| (smooth_max(sc, plain.0, TAU), smooth_max(sca, plain.1, TAU)));
        c += 0.5 * sc;
        ca += 0.5 * sca;
    }
    z_of_caps(c, ca)
}

/// Surface microstrip with coplanar copper (grounded CPW; gaps `s_left`/`s_right`, None = none within
/// reach), thickness and optional solder mask (`mask` = (thickness, εr)). Returns (Z0, εeff); without
/// coplanar copper the plain microstrip ([`microstrip_model`]).
pub fn microstrip_cpwg(
    w: f64,
    h: f64,
    t: f64,
    er: f64,
    mask: Option<(f64, f64)>,
    s_left: Option<f64>,
    s_right: Option<f64>,
) -> (f64, f64) {
    let (z, e) = microstrip_eeff(w, h, t, er, mask);
    if s_left.is_none() && s_right.is_none() {
        return (z, e);
    }
    let side = |s: Option<f64>| s.map(|s| cpwg_caps(w, s, h, t, er, mask));
    combine_sides(caps_of_z(z, e), [side(s_left), side(s_right)])
}

/// Stripline with coplanar copper: strip `w`, thickness `t`, dielectric `h1`/`e1` to one plane and
/// `h2`/`e2` to the other, the copper layer filled with `e_fill`; gaps `s_left`/`s_right` (None = none
/// within reach). Returns (Z0, εeff); without coplanar copper kisi's stripline ([`stripline_asym`]).
#[allow(clippy::too_many_arguments)] // the cross-section's dimensions
pub fn stripline_cpw(
    w: f64,
    h1: f64,
    h2: f64,
    t: f64,
    e1: f64,
    e2: f64,
    e_fill: f64,
    s_left: Option<f64>,
    s_right: Option<f64>,
) -> (f64, f64) {
    let z = stripline_asym(w, h1, h2, t, e1, e2);
    let e = (e1 / h1 + e2 / h2) / (1.0 / h1 + 1.0 / h2);
    if s_left.is_none() && s_right.is_none() {
        return (z, e);
    }
    let side = |s: Option<f64>| {
        s.map(|s| {
            let (a, b, cs) = (slab_cap(w, s, h1), slab_cap(w, s, h2), 2.0 * slot_cap(t, s));
            (e1 * a + e2 * b + e_fill * cs, a + b + cs)
        })
    };
    combine_sides(caps_of_z(z, e), [side(s_left), side(s_right)])
}

/// Modes of an edge-coupled surface microstrip pair from kisi's pair formulas: Zodd from the IPC
/// differential impedance ([`diff_microstrip`]), Zeven = Z0²/Zodd, the modal εeff from the
/// Kirschning-Jansen ratios around the single line's εeff. Returns ((Zodd, εodd), (Zeven, εeven)).
pub fn diff_microstrip_modes(
    w: f64,
    g: f64,
    h: f64,
    t: f64,
    er: f64,
    mask: Option<(f64, f64)>,
) -> ((f64, f64), (f64, f64)) {
    let (zp, ep) = microstrip_eeff(w, h, t, er, mask);
    let zo = diff_microstrip(zp, g, h) / 2.0;
    let (_, _, ee, eo) = kirschning_jansen(w, g, h, er);
    let em = 0.5 * (ee + eo);
    let (eo, ee) = if em > 0.0 { (ep * eo / em, ep * ee / em) } else { (ep, ep) };
    ((zo, eo), (zp * zp / zo, ee))
}

/// Per-line capacitances (C/ε0, C_air/ε0) of the even and the odd mode of a symmetric coupled grounded
/// CPW: strips `w`, centre gap `g`, outer gaps `gout` (None = no coplanar ground), substrate `h`/`er`,
/// thickness `t`, optional mask. Conformal mapping of each half-space with the symmetry plane as a
/// magnetic (even) or electric (odd) wall: with x1 = g/2, x2 = x1 + w, x3 = x2 + gout (in the substrate
/// tanh(πx/2h) in place of x), the per-line capacitance of a half-space is K(k)/K(k′) with
/// k_e² = (x2² − x1²)/(x3² − x1²) and k_o² = (x2² − x1²)·x3²/((x3² − x1²)·x2²) (Ghione & Naldi 1987;
/// Simons ch. 7); for x1 → 0 the even mode is the single CPW of width 2w, for x3 → ∞ the odd mode the
/// coplanar strips. Returns ((C_e, C_air,e), (C_o, C_air,o)).
fn coupled_cpwg_caps(
    w: f64,
    g: f64,
    gout: Option<f64>,
    h: f64,
    t: f64,
    er: f64,
    mask: Option<(f64, f64)>,
) -> ((f64, f64), (f64, f64)) {
    let ke = |a: f64, b: f64, c: f64| if c.is_finite() { ((b * b - a * a) / (c * c - a * a)).sqrt() } else { 0.0 };
    let ko = |a: f64, b: f64, c: f64| {
        let r = if c.is_finite() { c * c / (c * c - a * a) } else { 1.0 };
        ((b * b - a * a) / (b * b) * r).sqrt()
    };
    let (x1, x2) = (g / 2.0, g / 2.0 + w);
    let x3 = gout.map_or(f64::INFINITY, |s| x2 + s);
    let m = |x: f64| if x.is_finite() { (PI * x / (2.0 * h)).tanh() } else { 1.0 };
    let (m1, m2, m3) = (m(x1), m(x2), m(x3));
    let (ue, de) = (k_ratio(ke(x1, x2, x3)), k_ratio(ke(m1, m2, m3)));
    let (uo, d_o) = (k_ratio(ko(x1, x2, x3)), k_ratio(ko(m1, m2, m3)));
    // slots: the outer one in both modes, the centre one (to the electric wall at g/2) in the odd mode
    let (f_out, e_out, s_out) = match gout {
        Some(s) => {
            let (f, e) = mask_on_slot(w, s, t, mask);
            (f, e, slot_cap(t, s))
        }
        None => (1.0, 1.0, 0.0),
    };
    let (f_in, e_in) = mask_on_slot(w, g, t, mask);
    let s_in = 2.0 * slot_cap(t, g);
    let even = (f_out * ue + er * de + e_out * s_out, ue + de + s_out);
    let odd = (f_in * uo + er * d_o + e_out * s_out + e_in * s_in, uo + d_o + s_out + s_in);
    (even, odd)
}

/// Edge-coupled surface microstrip pair with coplanar ground (coupled grounded CPW): strip `w`, partner
/// width `pw`, gap `g`, the coplanar gap beside this strip `s_out` and beside the partner `s_out_partner`
/// (None = no coplanar copper on that side), substrate `h`/`er`, thickness `t`, optional mask. The
/// per-line capacitances are the mean of the symmetric pairs with either outer gap (an approximation
/// for unequal gaps), each mode joined to kisi's pair formulas ([`diff_microstrip_modes`]) by the smooth
/// maximum. The partner's width enters only through the gaps (the formula takes equal strips).
/// Returns ((Zodd, εodd), (Zeven, εeven)).
#[allow(clippy::too_many_arguments)] // the cross-section's dimensions
pub fn diff_microstrip_cpwg(
    w: f64,
    g: f64,
    h: f64,
    t: f64,
    er: f64,
    mask: Option<(f64, f64)>,
    s_out: Option<f64>,
    s_out_partner: Option<f64>,
) -> ((f64, f64), (f64, f64)) {
    let (lo, le) = diff_microstrip_modes(w, g, h, t, er, mask);
    if s_out.is_none() && s_out_partner.is_none() {
        return (lo, le);
    }
    let (e1, o1) = coupled_cpwg_caps(w, g, s_out, h, t, er, mask);
    let (e2, o2) = coupled_cpwg_caps(w, g, s_out_partner, h, t, er, mask);
    let join = |a: (f64, f64), b: (f64, f64), lim: (f64, f64)| {
        let l = caps_of_z(lim.0, lim.1);
        z_of_caps(smooth_max(0.5 * (a.0 + b.0), l.0, TAU), smooth_max(0.5 * (a.1 + b.1), l.1, TAU))
    };
    (join(o1, o2, lo), join(e1, e2, le))
}

/// A trace without a reference plane (no plane copper above or below it on any layer – a card-edge
/// finger, a pad over a cut-out through the whole stack): the return runs in the coplanar copper
/// beside it (gaps `s_left`, `s_right`, None = none within reach) and, for a pair, in the partner
/// (gap `gap`); copper thickness `t`. Each side with a coplanar conductor is half a [`cpw`] (2·Z_cpw of
/// that gap); the
/// pair's odd mode adds the partner as coplanar strips ([`cps`]), the even mode only the coplanar
/// copper. Returns (Z0, εeff, Zdiff, Zeven) – None where no return is within reach (a single trace
/// without coplanar copper: no Z0; a pair without it: no even mode, Z0 = Zodd). An estimate: the
/// sides combine as parallel slots.
pub fn no_plane(
    w: f64,
    h: f64,
    t: f64,
    er: f64,
    s_left: Option<f64>,
    s_right: Option<f64>,
    gap: Option<f64>,
) -> Option<(f64, f64, Option<f64>, Option<f64>)> {
    let side = |s: f64| {
        let (z, e) = cpw(w, s, h, t, er);
        (2.0 * z, e)
    };
    let mut y = 0.0;
    let mut ye = 0.0;
    for s in [s_left, s_right].into_iter().flatten() {
        let (z, e) = side(s);
        y += 1.0 / z;
        ye += e / z;
    }
    let single = (y > 0.0).then(|| (1.0 / y, ye / y));
    match gap {
        None => single.map(|(z, e)| (z, e, None, None)),
        Some(g) => {
            // the partner's side is no coplanar ground: only the outer gap (the side the partner
            // is not on) counts for this trace; with the canonical orientation the partner is on
            // the left
            let outer = s_right.map(side);
            let (zd, ed) = cps(w, g, h, t, er);
            let (yo, yeo) = outer.map_or((0.0, 0.0), |(z, e)| (1.0 / z, e / z));
            let y_odd = 2.0 / zd + yo;
            let z_odd = 1.0 / y_odd;
            let e_odd = (2.0 * ed / zd + yeo) / y_odd;
            let z_even = outer.map(|o| o.0);
            let z0 = z_even.map_or(z_odd, |ze| (ze * z_odd).sqrt());
            Some((z0, e_odd, Some(2.0 * z_odd), z_even))
        }
    }
}

// ---------------------------------------------------------------------------
// Differential approximations (IPC-2141)
// ---------------------------------------------------------------------------

/// Edge-coupled surface microstrip: Zdiff from single-ended Z0, gap s, height h.
pub fn diff_microstrip(z0: f64, s: f64, h: f64) -> f64 {
    2.0 * z0 * (1.0 - 0.48 * (-0.96 * s / h).exp())
}

/// Edge-coupled stripline: Zdiff from Z0, gap s, plane spacing b.
pub fn diff_stripline(z0: f64, s: f64, b: f64) -> f64 {
    2.0 * z0 * (1.0 - 0.347 * (-2.9 * s / b).exp())
}

// ---------------------------------------------------------------------------
// Asymmetric stripline variants
// ---------------------------------------------------------------------------

/// Asymmetric stripline with different dielectrics above (h2, e2) and below
/// (h1, e1): parallel combination of the two symmetric half-problems.
pub fn stripline_parallel(w: f64, h1: f64, h2: f64, t: f64, e1: f64, e2: f64) -> f64 {
    let z1 = stripline_sym(w, 2.0 * h1 + t, t, e1);
    let z2 = stripline_sym(w, 2.0 * h2 + t, t, e2);
    2.0 * z1 * z2 / (z1 + z2)
}

/// Model used by kisi for asymmetric striplines.
pub fn stripline_asym(w: f64, h1: f64, h2: f64, t: f64, e1: f64, e2: f64) -> f64 {
    stripline_parallel(w, h1, h2, t, e1, e2)
}

// ---------------------------------------------------------------------------
// Coupled lines – the models used by the extraction (IPC-2141 based)
// ---------------------------------------------------------------------------
//
// Note: Kirschning-Jansen and Cohn-based alternatives were evaluated against
// the 2D field solver reference (tools/validation) and did not do better for
// thick copper on thin dielectrics (t/h ≈ 0.4), so the simpler IPC forms stay
// until the field solver (M2) replaces all of this.

/// Single surface microstrip incl. thickness and optional solder mask.
pub fn microstrip_model(w: f64, h: f64, t: f64, er: f64, mask: Option<(f64, f64)>) -> f64 {
    match mask {
        Some((tm, em)) => microstrip_masked(w, h, t, er, tm, em).0,
        None => microstrip(w, h, t, er).0,
    }
}

/// Single surface microstrip incl. thickness and optional solder mask: (Z0, εeff).
pub fn microstrip_eeff(w: f64, h: f64, t: f64, er: f64, mask: Option<(f64, f64)>) -> (f64, f64) {
    match mask {
        Some((tm, em)) => microstrip_masked(w, h, t, er, tm, em),
        None => microstrip(w, h, t, er),
    }
}

/// Edge-coupled surface microstrip pair: differential impedance.
pub fn diff_microstrip_model(w: f64, s: f64, h: f64, t: f64, er: f64, mask: Option<(f64, f64)>) -> f64 {
    diff_microstrip(microstrip_model(w, h, t, er, mask), s, h)
}

/// Edge-coupled (possibly asymmetric) stripline pair: differential impedance.
pub fn diff_stripline_model(w: f64, s: f64, h1: f64, h2: f64, t: f64, e1: f64, e2: f64) -> f64 {
    diff_stripline(stripline_asym(w, h1, h2, t, e1, e2), s, h1 + h2 + t)
}

// ---------------------------------------------------------------------------
// Coupled lines (even / odd modes) – used for crosstalk and odd-mode losses
// ---------------------------------------------------------------------------

/// Kirschning & Jansen (1984) coupled microstrip, zero thickness (as in Qucs,
/// GPL-2.0-or-later). Returns (Z_even, Z_odd, eeff_even, eeff_odd).
pub fn kirschning_jansen(w: f64, s: f64, h: f64, er: f64) -> (f64, f64, f64, f64) {
    let u = w / h;
    let g = (s / h).max(1e-4);
    let ab = |u: f64| -> (f64, f64) {
        let a = 1.0
            + ((u.powi(4) + (u / 52.0).powi(2)) / (u.powi(4) + 0.432)).ln() / 49.0
            + (1.0 + (u / 18.1).powi(3)).ln() / 18.7;
        let b = 0.564 * ((er - 0.9) / (er + 3.0)).powf(0.053);
        (a, b)
    };
    let eer = |u: f64| -> f64 {
        let (a, b) = ab(u);
        (er + 1.0) / 2.0 + (er - 1.0) / 2.0 * (1.0 + 10.0 / u).powf(-a * b)
    };
    let zl = |u: f64| -> f64 {
        let fu = 6.0 + (2.0 * PI - 6.0) * (-(30.666 / u).powf(0.7528)).exp();
        ETA0 / 2.0 / PI * (fu / u + (1.0 + (2.0 / u).powi(2)).sqrt()).ln()
    };
    let v = u * (20.0 + g * g) / (10.0 + g * g) + g * (-g).exp();
    let ee = eer(v);
    let e1 = eer(u);
    let d = 0.593 + 0.694 * (-0.562 * u).exp();
    let bo = 0.747 * er / (0.15 + er);
    let co = bo - (bo - 0.207) * (-0.414 * u).exp();
    let ao = 0.7287 * (e1 - (er + 1.0) / 2.0) * (1.0 - (-0.179 * u).exp());
    let eo = ((er + 1.0) / 2.0 + ao - e1) * (-co * g.powf(d)).exp() + e1;
    let zl1 = zl(u) / e1.sqrt();
    let q1 = 0.8695 * u.powf(0.194);
    let q2 = 1.0 + 0.7519 * g + 0.189 * g.powf(2.31);
    let q3 = 0.1975 + (16.6 + (8.4 / g).powi(6)).powf(-0.387) + (g.powi(10) / (1.0 + (g / 3.4).powi(10))).ln() / 241.0;
    let q4 = q1 / q2 * 2.0 / ((-g).exp() * u.powf(q3) + (2.0 - (-g).exp()) * u.powf(-q3));
    let ze = (e1 / ee).sqrt() * zl1 / (1.0 - zl1 * e1.sqrt() * q4 / ETA0);
    let q5 = 1.794 + 1.14 * (1.0 + 0.638 / (g + 0.517 * g.powf(2.43))).ln();
    let q6 = 0.2305 + (g.powi(10) / (1.0 + (g / 5.8).powi(10))).ln() / 281.3 + (1.0 + 0.598 * g.powf(1.154)).ln() / 5.1;
    let q7 = (10.0 + 190.0 * g * g) / (1.0 + 82.3 * g.powi(3));
    let q8 = (-6.5 - 0.95 * g.ln() - (g / 0.15).powi(5)).exp();
    let q9 = q7.ln() * (q8 + 1.0 / 16.5);
    let q10 = (q2 * q4 - q5 * (u.ln() * q6 * u.powf(-q9)).exp()) / q2;
    let zo = (e1 / eo).sqrt() * zl1 / (1.0 - zl1 * e1.sqrt() * q10 / ETA0);
    (ze, zo, ee, eo)
}

/// Edge-coupled centred stripline, Cohn's exact zero-thickness result.
/// Returns (Z_even, Z_odd) for plane spacing b.
pub fn cohn_coupled(w: f64, s: f64, b: f64, er: f64) -> (f64, f64) {
    let a = (PI * w / (2.0 * b)).tanh();
    let c = (PI * (w + s) / (2.0 * b)).tanh();
    let z = |k: f64| 30.0 * PI / er.sqrt() / k_ratio(k);
    (z(a * c), z(a / c))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol * b.abs()
    }

    #[test]
    fn microstrip_reference() {
        // Classic 50 Ω microstrip on 1.6 mm FR4 (er 4.6, t=0): w ≈ 2.9 mm.
        // Hammerstad gives Z0 ≈ 50 Ω, eeff ≈ 3.5 (textbook value)
        let (z, e) = microstrip(2.9, 1.6, 0.0, 4.6);
        assert!(close(z, 50.0, 0.03), "z={z}");
        assert!(close(e, 3.48, 0.03), "e={e}");
        // Wide line limit: Z should fall with width
        assert!(microstrip(1.0, 0.2, 0.035, 4.3).0 < microstrip(0.3, 0.2, 0.035, 4.3).0);
    }

    #[test]
    fn stripline_reference() {
        // Zero thickness: compare with Cohn's exact conformal-mapping result
        // Z0 = 30π/√εr · K(k)/K(k'), k = sech(πw/2b)
        for &(w, er) in &[(0.2, 1.0), (0.4, 4.3), (0.8, 3.5), (1.5, 4.0)] {
            let k = 1.0 / (PI * w / 2.0).cosh();
            let exact = 30.0 * PI / f64::sqrt(er) * k_ratio(k);
            let z = stripline_sym(w, 1.0, 0.0, er);
            assert!(close(z, exact, 0.01), "w/b={w}: wheeler {z} vs cohn {exact}");
        }
        // symmetric case of asymmetric formula equals symmetric formula
        let za = stripline_asym(0.15, 0.2, 0.2, 0.0175, 4.0, 4.0);
        let zs = stripline_sym(0.15, 0.4175, 0.0175, 4.0);
        assert!(close(za, zs, 1e-9));
    }

    #[test]
    fn elliptic() {
        // K(0) = pi/2, K(1/sqrt2) = 1.854075
        assert!(close(ellip_k(0.0), PI / 2.0, 1e-12));
        assert!(close(ellip_k(0.5f64.sqrt()), 1.854_074_677, 1e-9));
    }

    #[test]
    fn cpwg_limits() {
        // narrow gap lowers Z compared to wide gap
        let (z1, _) = cpwg(0.3, 0.1, 0.2, 4.3);
        let (z2, _) = cpwg(0.3, 0.5, 0.2, 4.3);
        assert!(z1 < z2);
    }

    /// Coplanar lines with a ground plane against CST's 2D port modes (validation/cst_xs, open-boundary
    /// estimate): grounded CPW with and without mask, a stripline with coplanar ground (homogeneous:
    /// εeff = εr), a coupled grounded CPW; far coplanar copper leaves the plain line.
    #[test]
    fn coplanar_with_plane() {
        // cpwg_u1_g0.5, cpwg_u0.5_g0.25, cpwg_u0.5_g0.25_mask
        assert!(close(microstrip_cpwg(0.2, 0.2, 0.035, 4.4, None, Some(0.1), Some(0.1)).0, 53.38, 0.02));
        assert!(close(microstrip_cpwg(0.1, 0.2, 0.035, 4.4, None, Some(0.05), Some(0.05)).0, 53.02, 0.02));
        let m = Some((0.02, 3.3));
        assert!(close(microstrip_cpwg(0.1, 0.2, 0.035, 4.4, m, Some(0.05), Some(0.05)).0, 46.99, 0.03));
        // one side only lies between both sides and none
        let one = microstrip_cpwg(0.2, 0.2, 0.035, 4.4, None, Some(0.1), None).0;
        assert!(one > 53.0 && one < microstrip(0.2, 0.2, 0.035, 4.4).0);
        // copper at the end of the extraction's search (3 h): within 2 % of the plain microstrip, below it
        let zm = microstrip(0.2, 0.2, 0.035, 4.4).0;
        let zf = microstrip_cpwg(0.2, 0.2, 0.035, 4.4, None, Some(0.6), Some(0.6)).0;
        assert!(zf <= zm && zf > 0.98 * zm, "{zf} {zm}");
        assert_eq!(
            microstrip_cpwg(0.2, 0.2, 0.035, 4.4, m, None, None),
            microstrip_masked(0.2, 0.2, 0.035, 4.4, 0.02, 3.3)
        );
        // slcpw_g0.5
        let (z, e) = stripline_cpw(0.1, 0.0825, 0.1, 0.0175, 4.2, 4.2, 4.2, Some(0.05), Some(0.05));
        assert!(close(z, 36.35, 0.02) && (e - 4.2).abs() < 1e-9, "{z} {e}");
        // dcpwg_s0.5_g0.5: Zdiff 85.8, Zeven 70.6
        let ((zo, _), (ze, _)) = diff_microstrip_cpwg(0.2, 0.1, 0.2, 0.035, 4.4, None, Some(0.1), Some(0.1));
        assert!(close(2.0 * zo, 85.8, 0.03) && close(ze, 70.6, 0.04), "{zo} {ze}");
        assert_eq!(slot_cap(0.0, 0.1), 0.0);
    }

    /// Coplanar lines without a ground plane: on a thick substrate εeff → (εr + 1)/2; CPW and CPS are
    /// duals (Z_cpw·Z_cps = (η0/2)²/εeff for the same k); a 0.7 mm finger between ground fingers at
    /// 0.3 mm on a 1.35 mm board is about 75 Ω; a pair adds the partner.
    #[test]
    fn coplanar_without_plane() {
        let (z, e) = cpw(0.3, 0.15, 1e3, 0.0, 4.4);
        assert!((e - 2.7).abs() < 1e-3, "{e}");
        // CPS with the CPW's k (s/(s + 2w) = w/(w + 2s) for w = 0.15, s = 0.3 against w = 0.3, s = 0.15)
        let (zs, _) = cps(0.15, 0.3, 1e3, 0.0, 4.4);
        assert!(((z * zs) - (ETA0 / 2.0).powi(2) / e).abs() < 1e-6 * z * zs, "{z} {zs}");
        let (zf, _) = cpw(0.7, 0.3, 1.35, 0.035, 3.5);
        assert!((65.0..85.0).contains(&zf), "{zf}");
        let (z0, _, zd, ze) = no_plane(0.7, 1.35, 0.035, 3.5, Some(0.3), Some(0.3), None).unwrap();
        assert!((z0 - zf).abs() < 1e-9 && zd.is_none() && ze.is_none());
        let (_, _, zd, ze) = no_plane(0.7, 1.35, 0.035, 3.5, None, Some(0.3), Some(0.3)).unwrap();
        assert!(zd.unwrap() < 2.0 * ze.unwrap(), "{zd:?} {ze:?}");
        assert!(no_plane(0.2, 1.35, 0.035, 3.5, None, None, None).is_none());
    }
}

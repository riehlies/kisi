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
        x / (PI * (1.0 - x))
            * (1.0 - 0.5 * ((x / (2.0 - x)).powi(2) + (0.0796 * x / (wb + 1.1 * x)).powf(m)).ln())
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
fn k_ratio(k: f64) -> f64 {
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

/// Surface microstrip with optional coplanar ground on either side
/// (gaps `s_left`/`s_right`, None = no conductor nearby).
/// Each side is a grounded CPW (Wadell) corrected for thickness and mask by
/// the ratio to the zero-thickness microstrip; the two sides are combined as
/// an admittance average. A side can only lower the impedance.
pub fn microstrip_coplanar(
    w: f64,
    h: f64,
    t: f64,
    er: f64,
    mask: Option<(f64, f64)>,
    s_left: Option<f64>,
    s_right: Option<f64>,
) -> f64 {
    let zt = microstrip_model(w, h, t, er, mask);
    let (z_ms0, _) = microstrip(w, h, 0.0, er);
    let corr = zt / z_ms0;
    let side = |s: Option<f64>| -> f64 {
        match s {
            Some(s) => (cpwg(w, s, h, er).0 * corr).min(zt),
            None => zt,
        }
    };
    let zl = side(s_left);
    let zr = side(s_right);
    2.0 / (1.0 / zl + 1.0 / zr)
}

/// Edge-coupled surface microstrip pair: differential impedance.
pub fn diff_microstrip_model(w: f64, s: f64, h: f64, t: f64, er: f64, mask: Option<(f64, f64)>) -> f64 {
    diff_microstrip(microstrip_model(w, h, t, er, mask), s, h)
}

/// Diff microstrip pair with optional coplanar ground: the IPC coupling
/// factor applied to the plain microstrip, scaled by the coplanar effect.
#[allow(clippy::too_many_arguments)]
pub fn diff_microstrip_coplanar(
    w: f64,
    s: f64,
    h: f64,
    t: f64,
    er: f64,
    mask: Option<(f64, f64)>,
    s_left: Option<f64>,
    s_right: Option<f64>,
) -> f64 {
    let zp = microstrip_model(w, h, t, er, mask);
    let z = microstrip_coplanar(w, h, t, er, mask, s_left, s_right);
    diff_microstrip(zp, s, h) * (z / zp)
}

/// Edge-coupled (possibly asymmetric) stripline pair: differential impedance.
pub fn diff_stripline_model(w: f64, s: f64, h1: f64, h2: f64, t: f64, e1: f64, e2: f64) -> f64 {
    diff_stripline(stripline_asym(w, h1, h2, t, e1, e2), s, h1 + h2 + t)
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
}

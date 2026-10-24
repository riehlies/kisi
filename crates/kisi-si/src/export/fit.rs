//! Passive rational fits of frequency-dependent impedances and admittances, for the SPICE export.
//!
//! A series impedance is fitted as a Foster network `Z(s) = R0 + s·L + Σ R_k ∥ L_k` and a shunt
//! admittance as a Debye network `Y(s) = G0 + s·C + Σ (R_k + 1/(s·C_k))⁻¹`, with fixed poles spaced
//! logarithmically over the band and non-negative coefficients (non-negative least squares). Every
//! element is a positive R, L or C, so the result is passive and causal by construction; the fit
//! error (real and imaginary part, each relative to its own size) is reported. This is how the
//! skin effect R(f), L_int(f) and the dielectric loss G(f), C(f) of kisi's line models
//! (Djordjevic-Sarkar, Huray) become SPICE elements.

use crate::num::Cx;
use std::f64::consts::PI;

/// Series impedance `R0 + s·L + Σ R_k ∥ L_k + Σ (R_m ∥ L_m ∥ C_m)` (the parallel pairs `terms`:
/// (R_k, L_k); the parallel tanks `tanks`: (R_m, L_m, C_m) – resonances above the band, for
/// impedances whose inductance rises with frequency, like the plane-pair spans of a via).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Foster {
    pub r0: f64,
    pub l: f64,
    pub terms: Vec<(f64, f64)>,
    pub tanks: Vec<(f64, f64, f64)>,
}

impl Foster {
    pub fn z(&self, w: f64) -> Cx {
        let mut z = Cx::new(self.r0, w * self.l);
        for &(r, l) in &self.terms {
            // R ∥ jωL = jωLR / (R + jωL)
            let num = Cx::new(0.0, w * l * r);
            let den = Cx::new(r, w * l);
            z = z + num / den;
        }
        for &(r, l, c) in &self.tanks {
            if w > 0.0 {
                z = z + Cx::new(1.0 / r, w * c - 1.0 / (w * l)).inv();
            }
        }
        z
    }
    /// The same network scaled by `k` (a section of length k of a per-metre network).
    pub fn scaled(&self, k: f64) -> Foster {
        Foster { r0: self.r0 * k, l: self.l * k, terms: self.terms.iter().map(|&(r, l)| (r * k, l * k)).collect(), tanks: self.tanks.iter().map(|&(r, l, c)| (r * k, l * k, c / k)).collect() }
    }
}

/// Shunt admittance `G0 + s·C + Σ 1/(R_k + 1/(s·C_k)) + Σ 1/(R_m + s·L_m + 1/(s·C_m))` (the series
/// R–C branches `terms`: (R_k, C_k); series R–L–C branches `res`: (R_m, L_m, C_m) – resonances
/// above the band, for a capacitance that rises with frequency, like the dispersion of microstrip).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Debye {
    pub g0: f64,
    pub c: f64,
    pub terms: Vec<(f64, f64)>,
    pub res: Vec<(f64, f64, f64)>,
}

impl Debye {
    pub fn y(&self, w: f64) -> Cx {
        let mut y = Cx::new(self.g0, w * self.c);
        for &(r, c) in &self.terms {
            if w > 0.0 {
                y = y + Cx::new(r, -1.0 / (w * c)).inv();
            }
        }
        for &(r, l, c) in &self.res {
            if w > 0.0 {
                y = y + Cx::new(r, w * l - 1.0 / (w * c)).inv();
            }
        }
        y
    }
    /// The same network scaled by `k` in admittance (a section of length k).
    pub fn scaled(&self, k: f64) -> Debye {
        Debye { g0: self.g0 * k, c: self.c * k, terms: self.terms.iter().map(|&(r, c)| (r / k, c * k)).collect(), res: self.res.iter().map(|&(r, l, c)| (r / k, l / k, c * k)).collect() }
    }
}

/// Result of a fit: the network and its worst relative error over the fit band (real and
/// imaginary part, each relative to its own magnitude with a floor).
#[derive(Debug, Clone)]
pub struct Fit<T> {
    pub net: T,
    pub err: f64,
}

/// Frequencies of the fit (Hz): logarithmic from `f_lo` to `f_hi`.
pub fn fit_freqs(f_lo: f64, f_hi: f64, n: usize) -> Vec<f64> {
    let (a, b) = (f_lo.ln(), f_hi.ln());
    (0..n).map(|i| (a + (b - a) * i as f64 / (n - 1).max(1) as f64).exp()).collect()
}

/// Poles (rad/s) of a fit with `n` terms over [f_lo, f_hi]: logarithmic, half a step beyond both ends.
fn poles(f_lo: f64, f_hi: f64, n: usize) -> Vec<f64> {
    if n == 0 {
        return vec![];
    }
    let (a, b) = (f_lo.ln(), f_hi.ln());
    (0..n).map(|i| 2.0 * PI * (a + (b - a) * (i as f64 + 0.5) / n as f64).exp()).collect()
}

/// Non-negative least squares (Lawson–Hanson): min ‖A x − b‖, x ≥ 0. `a` is row-major m×n.
pub fn nnls(a: &[f64], b: &[f64], m: usize, n: usize) -> Vec<f64> {
    // scale columns to unit norm (conditioning), undo at the end
    let mut sc = vec![1.0; n];
    for j in 0..n {
        let s: f64 = (0..m).map(|i| a[i * n + j] * a[i * n + j]).sum::<f64>().sqrt();
        sc[j] = if s > 0.0 { 1.0 / s } else { 0.0 };
    }
    let col = |i: usize, j: usize| a[i * n + j] * sc[j];
    let mut x = vec![0.0; n];
    let mut passive: Vec<bool> = vec![false; n]; // true: in the positive set P
    let resid = |x: &[f64]| -> Vec<f64> { (0..m).map(|i| b[i] - (0..n).map(|j| col(i, j) * x[j]).sum::<f64>()).collect() };
    let tol = 1e-12 * b.iter().map(|v| v.abs()).fold(0.0, f64::max).max(1e-300);
    for _outer in 0..(3 * n + 10) {
        let r = resid(&x);
        let wv: Vec<f64> = (0..n).map(|j| (0..m).map(|i| col(i, j) * r[i]).sum()).collect();
        let cand = (0..n).filter(|&j| !passive[j] && sc[j] > 0.0).max_by(|&p, &q| wv[p].total_cmp(&wv[q]));
        match cand {
            Some(j) if wv[j] > tol => passive[j] = true,
            _ => break,
        }
        for _inner in 0..(3 * n + 10) {
            let idx: Vec<usize> = (0..n).filter(|&j| passive[j]).collect();
            let z = lsq(&|i, k| col(i, idx[k]), b, m, idx.len());
            if z.iter().all(|&v| v > 0.0) {
                for (k, &j) in idx.iter().enumerate() {
                    x[j] = z[k];
                }
                break;
            }
            // step back towards x until a variable leaves P
            let mut alpha = 1.0f64;
            for (k, &j) in idx.iter().enumerate() {
                if z[k] <= 0.0 {
                    let d = x[j] - z[k];
                    if d > 0.0 {
                        alpha = alpha.min(x[j] / d);
                    }
                }
            }
            for (k, &j) in idx.iter().enumerate() {
                x[j] += alpha * (z[k] - x[j]);
                if x[j] <= 1e-300 || (z[k] <= 0.0 && x[j] <= tol) {
                    x[j] = 0.0;
                    passive[j] = false;
                }
            }
        }
    }
    x.iter().zip(&sc).map(|(v, s)| v * s).collect()
}

/// Unconstrained least squares by Householder QR; `a(i, k)` gives the m×n matrix.
fn lsq(a: &dyn Fn(usize, usize) -> f64, b: &[f64], m: usize, n: usize) -> Vec<f64> {
    if n == 0 {
        return vec![];
    }
    let mut q: Vec<f64> = (0..m * n).map(|t| a(t / n, t % n)).collect();
    let mut y = b.to_vec();
    let mut diag = vec![0.0; n];
    for k in 0..n.min(m) {
        let norm: f64 = (k..m).map(|i| q[i * n + k] * q[i * n + k]).sum::<f64>().sqrt();
        if norm == 0.0 {
            continue;
        }
        let alpha = if q[k * n + k] > 0.0 { -norm } else { norm };
        let mut v: Vec<f64> = (k..m).map(|i| q[i * n + k]).collect();
        v[0] -= alpha;
        let vn: f64 = v.iter().map(|t| t * t).sum();
        if vn == 0.0 {
            diag[k] = alpha;
            continue;
        }
        for j in k..n {
            let s: f64 = (k..m).map(|i| v[i - k] * q[i * n + j]).sum::<f64>() * 2.0 / vn;
            for i in k..m {
                q[i * n + j] -= s * v[i - k];
            }
        }
        let s: f64 = (k..m).map(|i| v[i - k] * y[i]).sum::<f64>() * 2.0 / vn;
        for i in k..m {
            y[i] -= s * v[i - k];
        }
        diag[k] = q[k * n + k];
    }
    let mut x = vec![0.0; n];
    for k in (0..n.min(m)).rev() {
        let mut s = y[k];
        for j in k + 1..n {
            s -= q[k * n + j] * x[j];
        }
        x[k] = if diag[k].abs() > 1e-300 { s / diag[k] } else { 0.0 };
    }
    x
}

/// How the fit error is measured (and the rows weighted).
#[derive(Debug, Clone, Copy)]
pub enum Weight {
    /// real and imaginary part each relative to its own size, the imaginary part counted `im`
    /// times (lines: the loss within the tolerance, the reactance – the delay – `im` times closer)
    Parts { im: f64, re_floor: f64 },
    /// the complex difference relative to the magnitude (small lumped elements)
    Complex,
}

fn row_weights(t: Cx, w: Weight) -> (f64, f64) {
    let mag = t.abs().max(1e-300);
    match w {
        Weight::Parts { im, re_floor } => (1.0 / t.re.abs().max(re_floor.max(1e-6) * mag), im / t.im.abs().max(1e-6 * mag)),
        Weight::Complex => (1.0 / mag, 1.0 / mag),
    }
}

fn err_of(fit: Cx, t: Cx, w: Weight) -> f64 {
    let (wr, wi) = row_weights(t, w);
    match w {
        Weight::Parts { .. } => ((fit.re - t.re).abs() * wr).max((fit.im - t.im).abs() * wi),
        Weight::Complex => (fit - t).abs() * wr,
    }
}

/// Fit a series impedance `R0 + s·L + Σ R_k ∥ L_k` over `freqs` (Hz), all coefficients ≥ 0.
/// Adds terms until the error is below `tol` (at most `max_terms`).
pub fn fit_z(freqs: &[f64], target: &dyn Fn(f64) -> Cx, weight: Weight, tol: f64, max_terms: usize, tanks: bool) -> Fit<Foster> {
    let (f_lo, f_hi) = (freqs[0], *freqs.last().unwrap());
    let tv: Vec<Cx> = freqs.iter().map(|&f| target(f)).collect();
    // tanks a·s·ωm/(s² + s·ωm/Q + ωm²): resonances 1.25 … 4 × the top frequency, Q 0.7 and 2
    let tk: Vec<(f64, f64)> = if tanks { [1.25, 1.6, 2.0, 2.5, 3.2, 4.0].iter().flat_map(|m| [0.7, 2.0].map(|q| (2.0 * PI * f_hi * m, q))).collect() } else { vec![] };
    let mut best: Option<Fit<Foster>> = None;
    for nt in 0..=max_terms {
        let ps = poles(f_lo / 10.0, f_hi * 10.0, nt);
        let ncol = 2 + nt + tk.len();
        let m = 2 * freqs.len();
        let mut a = vec![0.0; m * ncol];
        let mut b = vec![0.0; m];
        for (i, (&f, &z)) in freqs.iter().zip(&tv).enumerate() {
            let w = 2.0 * PI * f;
            let (wr, wi) = row_weights(z, weight);
            let (re, im) = (2 * i, 2 * i + 1);
            a[re * ncol] = wr; // R0
            a[im * ncol + 1] = w * wi; // s·L
            for (k, &p) in ps.iter().enumerate() {
                let d = w * w + p * p;
                a[re * ncol + 2 + k] = w * w / d * wr;
                a[im * ncol + 2 + k] = w * p / d * wi;
            }
            for (k, &(wm, q)) in tk.iter().enumerate() {
                // h = jω ωm / (ωm² − ω² + jω ωm/Q)
                let h = Cx::new(0.0, w * wm) / Cx::new(wm * wm - w * w, w * wm / q);
                a[re * ncol + 2 + ps.len() + k] = h.re * wr;
                a[im * ncol + 2 + ps.len() + k] = h.im * wi;
            }
            b[re] = z.re * wr;
            b[im] = z.im * wi;
        }
        let x = nnls(&a, &b, m, ncol);
        let mut fo = Foster { r0: x[0], l: x[1], terms: vec![], tanks: vec![] };
        for (k, &p) in ps.iter().enumerate() {
            if x[2 + k] > 0.0 {
                fo.terms.push((x[2 + k], x[2 + k] / p));
            }
        }
        for (k, &(wm, q)) in tk.iter().enumerate() {
            let a = x[2 + ps.len() + k];
            if a > 0.0 {
                // parallel R, L, C: C = 1/(a ωm), L = a/ωm, R = Q a
                fo.tanks.push((q * a, a / wm, 1.0 / (a * wm)));
            }
        }
        let err = freqs.iter().zip(&tv).map(|(&f, &t)| err_of(fo.z(2.0 * PI * f), t, weight)).fold(0.0, f64::max);
        if best.as_ref().is_none_or(|b| err < b.err) {
            best = Some(Fit { net: fo, err });
        }
        if err <= tol {
            break;
        }
    }
    best.unwrap()
}

/// Fit a shunt admittance `G0 + s·C + Σ (R_k + 1/(s·C_k))⁻¹` over `freqs` (Hz), all elements
/// ≥ 0; terms are added until the error is below `tol`.
pub fn fit_y(freqs: &[f64], target: &dyn Fn(f64) -> Cx, weight: Weight, tol: f64, max_terms: usize, resonances: bool) -> Fit<Debye> {
    let (f_lo, f_hi) = (freqs[0], *freqs.last().unwrap());
    let tv: Vec<Cx> = freqs.iter().map(|&f| target(f)).collect();
    let tk: Vec<(f64, f64)> = if resonances { [1.25, 1.6, 2.0, 2.5, 3.2, 4.0].iter().flat_map(|m| [0.7, 2.0].map(|q| (2.0 * PI * f_hi * m, q))).collect() } else { vec![] };
    // a lossless admittance (G = 0) is weighted against 0.01 % of its susceptance
    let wt = |y: Cx| if y.re.abs() < 1e-4 * y.im.abs() { Cx::new(1e-4 * y.im.abs(), y.im) } else { y };
    let mut best: Option<Fit<Debye>> = None;
    for nt in 0..=max_terms {
        let qs = poles(f_lo / 10.0, f_hi * 10.0, nt);
        let ncol = 2 + nt + tk.len();
        let m = 2 * freqs.len();
        let mut a = vec![0.0; m * ncol];
        let mut b = vec![0.0; m];
        for (i, (&f, &y)) in freqs.iter().zip(&tv).enumerate() {
            let w = 2.0 * PI * f;
            let (wr, wi) = row_weights(wt(y), weight);
            let (re, im) = (2 * i, 2 * i + 1);
            a[re * ncol] = wr; // G0
            a[im * ncol + 1] = w * wi; // s·C
            for (k, &q) in qs.iter().enumerate() {
                // e·s/(s+q): series R = 1/e, C = e/q
                let d = w * w + q * q;
                a[re * ncol + 2 + k] = w * w / d * wr;
                a[im * ncol + 2 + k] = w * q / d * wi;
            }
            for (k, &(wm, qf)) in tk.iter().enumerate() {
                let h = Cx::new(0.0, w * wm) / Cx::new(wm * wm - w * w, w * wm / qf);
                a[re * ncol + 2 + qs.len() + k] = h.re * wr;
                a[im * ncol + 2 + qs.len() + k] = h.im * wi;
            }
            b[re] = y.re * wr;
            b[im] = y.im * wi;
        }
        let x = nnls(&a, &b, m, ncol);
        let mut de = Debye { g0: x[0], c: x[1], terms: vec![], res: vec![] };
        for (k, &q) in qs.iter().enumerate() {
            let e = x[2 + k];
            if e > 0.0 {
                de.terms.push((1.0 / e, e / q));
            }
        }
        for (k, &(wm, qf)) in tk.iter().enumerate() {
            let a = x[2 + qs.len() + k];
            if a > 0.0 {
                // series R, L, C: L = 1/(a ωm), C = a/ωm, R = 1/(a Q)
                de.res.push((1.0 / (a * qf), 1.0 / (a * wm), a / wm));
            }
        }
        let err = freqs.iter().zip(&tv).map(|(&f, &t)| err_of(de.y(2.0 * PI * f), wt(t), weight).min(err_of(de.y(2.0 * PI * f), t, weight))).fold(0.0, f64::max);
        if best.as_ref().is_none_or(|b| err < b.err) {
            best = Some(Fit { net: de, err });
        }
        if err <= tol {
            break;
        }
    }
    best.unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nnls_small() {
        // x0 + x1 = 1, x0 − x1 = 3 → unconstrained (2, −1), with x ≥ 0: x1 = 0, x0 = 2 (LS of 1 and 3)
        let x = nnls(&[1.0, 1.0, 1.0, -1.0], &[1.0, 3.0], 2, 2);
        assert!((x[0] - 2.0).abs() < 1e-9 && x[1].abs() < 1e-12, "{x:?}");
    }

    /// The skin effect R = Rdc + k√f (with the internal reactance k√f) is followed within 1 %
    /// from 1 MHz to 25 GHz; a Debye admittance of the Djordjevic-Sarkar form within 1 %.
    #[test]
    fn skin_and_dielectric() {
        let fs = fit_freqs(2.5e6, 25e9, 60);
        // a 50 Ω line: L = 330 nH/m, Rdc 5 Ω/m, smooth skin effect with its internal reactance
        let (rdc, k, l) = (5.0, 2e-3, 330e-9);
        let z = |f: f64| {
            let rac = k * f.sqrt();
            Cx::new((rdc * rdc + rac * rac).sqrt(), rac + 2.0 * PI * f * l)
        };
        // kisi's grid starts at 50 MHz; below, √(Rdc² + Rac²) is not the causal pair of its reactance
        let fz = fit_z(&fit_freqs(50e6, 25e9, 60), &z, Weight::Parts { im: 10.0, re_floor: 0.0 }, 0.01, 14, false);
        assert!(fz.err < 0.02, "skin fit {:.4}", fz.err);
        let ds = crate::rf::DjordjevicSarkar::fit(4.2, 0.02);
        let c0 = 100e-12;
        let y = |f: f64| {
            let (e, t) = ds.at(f);
            let c = c0 * e / 4.2;
            Cx::new(2.0 * PI * f * c * t, 2.0 * PI * f * c)
        };
        let fy = fit_y(&fs, &y, Weight::Parts { im: 10.0, re_floor: 0.0 }, 0.01, 12, false);
        assert!(fy.err < 0.01, "dielectric fit {:.4}", fy.err);
        assert!(fy.net.terms.iter().all(|t| t.0 > 0.0 && t.1 > 0.0) && fy.net.c > 0.0);
    }
}

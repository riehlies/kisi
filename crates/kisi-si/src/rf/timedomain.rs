//! Step responses from transfer functions on a frequency grid (milestone M2): the eye and,
//! later, the transient solver of M3 take the channel's impulse response from here.
//!
//! The data: H(f) on the uniform grid f_k = k·df (k = 1 … n) and a DC point H(0). The step is
//! a unit step with a Gaussian edge of the transmitter's 10–90 % rise time centred at t = 0; the
//! Gaussian is the window (for a bare impulse response take `Step::window_rise`, 1 % of the
//! spectrum left at the top of the band). The response is the antiderivative of the
//! band-limited impulse response, evaluated directly (no FFT):
//!
//! ```text
//! s(t) = df·[H(0)·t + 2·Σ Re(H_k·G_k·e^{jω_k t} / (jω_k))] − s(t₀)
//! ```
//!
//! Checks of the result, relative to the final value: how much of the edge spectrum the band
//! cuts off (the step then rings, Gibbs), the response before the signal can arrive (data that
//! is not causal, or time aliasing when the response is longer than 1/df) and how far the step
//! still is from its final value at the end of the window.
//!
//! An extra taper at the top of the band was tried and left out: with the Gaussian already in
//! place it gave more overshoot for edges the band covers (Hann over the top quarter at 25 GHz:
//! 30 ps edge 0.7 % instead of 0.4 %, 40 ps 0.4 % instead of 0.05 %) and helped little where the
//! band cuts the edge (18 ps: 3.3 % instead of 4.8 %).

use super::Cx;
use serde::Serialize;
use std::f64::consts::{LN_2, PI};

/// A precursor or an unsettled step above this share of the final value gives a warning.
pub const TD_WARN: f64 = 0.02;
/// The DC point is used when it agrees with |H| at the lowest frequency within this share;
/// otherwise the channel changes below the grid (AC coupling with a small capacitor, a bias
/// choke, a band-pass filter) and the lowest frequency stands for DC, as the grid cannot resolve
/// what happens below it.
pub const DC_AGREE: f64 = 0.05;
/// Edge spectrum left at the top of the band above which the step rings noticeably (overshoot
/// of the bare edge: 0.4 % at 0.18, about 2 % at 0.35, 4.8 % at 0.54 – 30, 24 and 18 ps at 25 GHz).
pub const BAND_EDGE_WARN: f64 = 0.35;

/// What went into a step response and how well it behaves (shares of the final value).
#[derive(Debug, Clone, Default, Serialize)]
pub struct TdCheck {
    /// transfer at 0 Hz and where it comes from: "network at 0 Hz" or "lowest frequency"
    pub dc: f64,
    pub dc_source: &'static str,
    /// the transmitter's edge spectrum (Gaussian) left at the top of the band
    pub band_edge: f64,
    /// largest response before the signal can arrive
    pub precursor: f64,
    /// distance from the final value at the end of the window
    pub unsettled: f64,
}

impl TdCheck {
    /// Warnings for the checks above the limits.
    pub fn warnings(&self) -> Vec<String> {
        let mut w = Vec::new();
        if self.band_edge > BAND_EDGE_WARN {
            w.push(format!(
                "time domain: the band ends inside the transmitter's edge spectrum ({:.0} % left at the highest frequency) – the edge rings by a few percent; the result is band-limited",
                100.0 * self.band_edge
            ));
        }
        if self.precursor > TD_WARN {
            w.push(format!(
                "time domain: {:.1} % response before the signal can arrive – data that is not causal (e.g. a Touchstone file) or a response longer than the frequency step allows",
                100.0 * self.precursor
            ));
        }
        if self.unsettled > 2.5 * TD_WARN {
            w.push(format!("time domain: the step response is still {:.1} % from its final value at the end of the window (long reflections or slow loss tail)", 100.0 * self.unsettled));
        }
        w
    }
}

/// Step response of one transfer function (see the module notes).
pub struct Step {
    terms: Vec<(f64, Cx)>,
    s0: f64,
    df: f64,
    f_ref: f64,
    pub check: TdCheck,
}

impl Step {
    /// The rise time of a Gaussian window that leaves 1 % of the spectrum at `fmax`.
    pub fn window_rise(fmax: f64) -> f64 {
        0.3394 * (2.0 * (100.0f64).ln() / LN_2).sqrt() / fmax
    }

    /// `freq` must be the uniform grid k·df (k ≥ 1); `dc`: H(0) (None: |H(f₁)| with the sign of
    /// its real part, the old extrapolation); `rise`: 10–90 % of the Gaussian edge.
    pub fn new(freq: &[f64], h: &[Cx], dc: Option<f64>, rise: f64) -> Step {
        let df = freq[0];
        let fmax = *freq.last().unwrap();
        let f3 = 0.3394 / rise;
        let gauss = |f: f64| (-(LN_2 / 2.0) * (f / f3).powi(2)).exp();
        let low = h[0].abs() * if h[0].re < 0.0 { -1.0 } else { 1.0 };
        let (s0, dc_source) = match dc {
            Some(v) if v.is_finite() && (v - low).abs() <= DC_AGREE * v.abs().max(low.abs()) => (v, "network at 0 Hz"),
            Some(v) if v.is_finite() => (low, "lowest frequency (the channel changes below it)"),
            _ => (low, "lowest frequency"),
        };
        let terms: Vec<(f64, Cx)> = freq.iter().zip(h).map(|(&f, &s)| (2.0 * PI * f, s.scale(gauss(f)))).filter(|(_, s)| s.abs() > 1e-9).collect();
        let mut st = Step { terms, s0, df, f_ref: 0.0, check: TdCheck { dc: s0, dc_source, band_edge: gauss(fmax), ..Default::default() } };
        st.f_ref = st.raw(-3.0 * rise);
        st
    }

    fn raw(&self, t: f64) -> f64 {
        let mut acc = self.s0 * t;
        for &(w, sh) in &self.terms {
            let (s, c) = (w * t).sin_cos();
            // Re(sh·e^{jωt}/(jω)) = (sh.im·cos + sh.re·sin)/ω … with e^{jωt} = c + js
            acc += 2.0 * (sh.re * s + sh.im * c) / w;
        }
        acc * self.df
    }

    /// Step response at time t (s), 0 before the edge.
    pub fn at(&self, t: f64) -> f64 {
        self.raw(t) - self.f_ref
    }

    /// Samples at t = i·dt, i < nt.
    pub fn samples(&self, dt: f64, nt: usize) -> Vec<f64> {
        (0..nt).map(|i| self.at(i as f64 * dt)).collect()
    }

    /// Fill in the checks: the response between −0.45/df and `t_arrive` (the earliest time the
    /// signal can arrive, edge included) should be zero; at `t_end` it should have reached the
    /// final value (the DC point, or the last sample when that is far off).
    pub fn verify(&mut self, t_arrive: f64, t_end: f64) {
        let fin = if self.s0.abs() > 1e-6 { self.s0 } else { self.at(t_end) };
        let norm = fin.abs().max(1e-6);
        let t_lo = -0.45 / self.df;
        let n = 400usize;
        let mut pre: f64 = 0.0;
        if t_arrive > t_lo {
            for i in 0..=n {
                let t = t_lo + (t_arrive - t_lo) * i as f64 / n as f64;
                pre = pre.max(self.at(t).abs());
            }
        }
        self.check.precursor = pre / norm;
        self.check.unsettled = (self.at(t_end) - fin).abs() / norm;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(fmax: f64, n: usize) -> Vec<f64> {
        (1..=n).map(|k| k as f64 * fmax / n as f64).collect()
    }

    /// A delay line: the step is a Gaussian edge at the delay, with nothing before it.
    #[test]
    fn delay_line_step() {
        let freq = grid(40e9, 800);
        let tau = 200e-12;
        let h: Vec<Cx> = freq.iter().map(|&f| Cx::new((2.0 * PI * f * tau).cos(), -(2.0 * PI * f * tau).sin())).collect();
        let mut st = Step::new(&freq, &h, Some(1.0), 30e-12);
        assert!(st.at(tau).abs() - 0.5 < 1e-3 && (st.at(tau) - 0.5).abs() < 1e-3, "{}", st.at(tau));
        assert!((st.at(tau + 100e-12) - 1.0).abs() < 1e-3);
        st.verify(tau - 60e-12, 2e-9);
        assert!(st.check.precursor < 1e-3 && st.check.unsettled < 1e-3, "{:?}", st.check);
        assert!(st.check.warnings().is_empty());
        assert_eq!(st.check.dc_source, "network at 0 Hz");
    }

    /// A band that ends inside the edge spectrum is reported; the window rise leaves 1 %.
    #[test]
    fn band_edge_reported() {
        let freq = grid(25e9, 500);
        let h: Vec<Cx> = freq.iter().map(|_| Cx::new(1.0, 0.0)).collect();
        let st = Step::new(&freq, &h, Some(1.0), 18e-12);
        assert!(st.check.band_edge > 0.5 && st.check.warnings().iter().any(|w| w.contains("edge spectrum")));
        let w = Step::new(&freq, &h, Some(1.0), Step::window_rise(25e9));
        assert!((w.check.band_edge - 0.01).abs() < 1e-9);
        let over = (0..2000).map(|i| w.at(i as f64 * 0.2e-12)).fold(f64::NEG_INFINITY, f64::max);
        assert!(over < 1.001, "overshoot {over}");
    }

    /// Non-causal data (a response that leads the input) is found.
    #[test]
    fn precursor_found() {
        let freq = grid(20e9, 400);
        let tau = -150e-12; // the output 150 ps before the input
        let h: Vec<Cx> = freq.iter().map(|&f| Cx::new((2.0 * PI * f * tau).cos(), -(2.0 * PI * f * tau).sin())).collect();
        let mut st = Step::new(&freq, &h, None, 30e-12);
        st.verify(0.0, 2e-9);
        assert!(st.check.precursor > 0.5, "{:?}", st.check);
        assert_eq!(st.check.dc_source, "lowest frequency");
        assert!(!st.check.warnings().is_empty());
    }
}

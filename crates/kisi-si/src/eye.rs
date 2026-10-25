//! Eye diagram of a channel (milestone M5): NRZ PRBS through the channel.
//!
//! The step response of the transfer between the ends (Gaussian transmitter edge of the
//! given 10-90 % rise time, DC point, taper and checks from `rf::timedomain`) gives the
//! single-bit pulse response; a periodic PRBS is superposed on it (linear, time-invariant
//! channel, no equalisation). Amplitudes are fractions of the transmitter swing. The
//! transfer comes from the cascade (`transfer`) or from a network (`rf::network`, e.g. the
//! 4-port of a pair, see `kisi_si::lanenet`) through `eye_of`.

use crate::rf::{Abcd, Channel, Cx, Step, TdCheck};
use serde::{Deserialize, Serialize};
use std::f64::consts::PI;

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct EyeParams {
    /// bit rate (bit/s)
    pub bitrate: f64,
    /// 10-90 % rise time of the transmitter edge (s)
    pub rise: f64,
    /// PRBS order (7, 9 or 11)
    pub prbs: u32,
    pub samples_per_ui: usize,
    pub rows: usize,
    /// transmitter / receiver models; None = ideal terminations with the reference impedance
    pub tx: Option<EndModel>,
    pub rx: Option<EndModel>,
}

/// Linear model of a transmitter or receiver pin, per line (a differential
/// channel is computed in the odd mode, so values are per line, i.e. half the
/// differential resistance).
///
/// * transmitter: source resistance `r` in series, pin capacitance `c_pf` to
///   ground behind it, package inductance `l_nh` towards the board
/// * receiver: package inductance `l_nh`, then `c_pf` and the termination `r`
///   to (AC) ground; `r` ≤ 0 means no termination (high impedance input)
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(default)]
pub struct EndModel {
    pub r: f64,
    pub c_pf: f64,
    pub l_nh: f64,
}

impl Default for EndModel {
    fn default() -> Self {
        EndModel { r: 50.0, c_pf: 0.0, l_nh: 0.0 }
    }
}

/// The end of a lane at a connector (the rest of the channel is off the board): the
/// termination there when the connector is the receiving end, the driver when it sends.
/// `model`: "profile" (the profile's receiver / transmitter, as if the device sat at the
/// connector), "matched" (the target impedance per line, no capacitance), "high_impedance"
/// (no termination; as a driver an ideal source) or "custom" (`r_ohm`, `c_pf`, `l_nh`).
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct ConnEnd {
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r_ohm: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub c_pf: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub l_nh: Option<f64>,
    /// the channel behind the connector as a simple lossy line (target impedance per line):
    /// loss at the profile's Nyquist frequency in dB (positive; half skin effect ∝ √f, half
    /// dielectric ∝ f) – with it the eye covers the whole channel and is judged
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rest_loss_db: Option<f64>,
    /// its delay (ps), default 500
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rest_delay_ps: Option<f64>,
    /// the channel behind the connector from a Touchstone file (in place of the lossy line)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub touchstone: Option<TsRef>,
}

/// A Touchstone file for the channel behind a connector: `.s2p` (single-ended lanes, or the
/// same 2-port on every line of a pair) or `.s4p` (a pair; `port_order` "1-3,2-4" or
/// "1-2,3-4"); port 1 at the connector unless `flip`. Or one file for all lanes of the
/// connector (`.sNp`, also a 4-port of two single-ended lanes): `ports` gives, per connector
/// pin, the file's port at the connector and at the far end (1-based); the other lanes of the
/// connector in the file are aggressors in the eye.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct TsRef {
    /// path relative to the kisi.json (CLI) or the file name (viewer)
    pub file: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port_order: Option<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub flip: bool,
    /// connector pin → [port at the connector, port at the far end] (1-based)
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub ports: std::collections::BTreeMap<String, [usize; 2]>,
}

impl ConnEnd {
    pub const MODELS: &'static [&'static str] = &["profile", "matched", "high_impedance", "custom"];

    /// A channel model behind the connector is set (the eye then covers the whole channel).
    pub fn has_rest(&self) -> bool {
        self.touchstone.as_ref().is_some_and(|t| !t.file.is_empty()) || self.rest_loss_db.is_some_and(|v| v.is_finite())
    }

    /// ABCD of the channel behind the connector at frequency `f` (Hz): a line with
    /// characteristic impedance `z0` (per line), delay `rest_delay_ps` and the loss
    /// `rest_loss_db` at `f_nyq`, split evenly into a √f and an f term.
    pub fn rest_abcd(&self, f: f64, f_nyq: f64, z0: f64) -> Option<crate::rf::Abcd> {
        let loss = self.rest_loss_db.filter(|v| v.is_finite())?.abs();
        let x = (f / f_nyq.max(1.0)).max(0.0);
        let alpha = loss / (20.0 / std::f64::consts::LN_10) * (0.5 * x.sqrt() + 0.5 * x);
        let beta = 2.0 * PI * f * self.rest_delay_ps.unwrap_or(500.0).max(0.0) * 1e-12;
        let gl = Cx::new(alpha, beta);
        let (e, ei) = (gl.exp(), Cx::new(-alpha, -beta).exp());
        let ch = (e + ei).scale(0.5);
        let sh = (e - ei).scale(0.5);
        Some(crate::rf::Abcd { a: ch, b: sh.scale(z0), c: sh.scale(1.0 / z0), d: ch })
    }

    /// The end model at the connector; `profile_end` is the profile's receiver or transmitter
    /// for that end, `target` the lane's target impedance (single-ended, or differential).
    pub fn resolve(&self, profile_end: Option<EndModel>, target: Option<(f64, bool)>) -> Result<Option<EndModel>, String> {
        match self.model.as_str() {
            "" | "profile" => Ok(profile_end),
            "matched" => {
                let (z, diff) = target.ok_or("connector end 'matched' needs a target impedance")?;
                Ok(Some(EndModel { r: if diff { z / 2.0 } else { z }, c_pf: 0.0, l_nh: 0.0 }))
            }
            "high_impedance" => Ok(Some(EndModel { r: 0.0, c_pf: self.c_pf.unwrap_or(0.0), l_nh: self.l_nh.unwrap_or(0.0) })),
            "custom" => {
                let r = self.r_ohm.ok_or("connector end 'custom' needs r_ohm")?;
                Ok(Some(EndModel { r, c_pf: self.c_pf.unwrap_or(0.0), l_nh: self.l_nh.unwrap_or(0.0) }))
            }
            m => Err(format!("unknown connector end model '{m}' ({})", Self::MODELS.join(", "))),
        }
    }
}

/// Voltage transfer V_rx / V_tx of the channel between the given ends, in
/// units of the transmitter swing: the swing is defined into the receiver's
/// nominal termination (a lossless link without parasitics gives |H| = 1 at
/// DC), or as the open-circuit swing for a high-impedance receiver (CMOS).
pub fn transfer(ch: &Channel, tx: &EndModel, rx: &EndModel) -> Vec<Cx> {
    let rs = tx.r.max(0.0);
    let scale = if rx.r > 0.0 { (rs + rx.r) / rx.r } else { 1.0 };
    ch.freq
        .iter()
        .zip(&ch.abcd)
        .map(|(&f, m)| {
            let w = 2.0 * PI * f;
            let mut t = Abcd::shunt(Cx::new(0.0, w * tx.c_pf * 1e-12));
            if tx.l_nh > 0.0 {
                t = t.mul(&Abcd::series(Cx::new(0.0, w * tx.l_nh * 1e-9)));
            }
            t = t.mul(m);
            if rx.l_nh > 0.0 {
                t = t.mul(&Abcd::series(Cx::new(0.0, w * rx.l_nh * 1e-9)));
            }
            let g = if rx.r > 0.0 { 1.0 / rx.r } else { 0.0 };
            t = t.mul(&Abcd::shunt(Cx::new(g, w * rx.c_pf * 1e-12)));
            // V_tx = (A + Rs·C)·V_rx with the load already inside the cascade
            (t.a + t.c.scale(rs)).inv().scale(scale)
        })
        .collect()
}

/// `transfer` at 0 Hz (the channel's DC cascade `abcd_dc`; resistances only): the DC point of
/// the eye's step response.
pub fn transfer_dc(ch: &Channel, tx: &EndModel, rx: &EndModel) -> f64 {
    let rs = tx.r.max(0.0);
    let scale = if rx.r > 0.0 { (rs + rx.r) / rx.r } else { 1.0 };
    let g = if rx.r > 0.0 { 1.0 / rx.r } else { 0.0 };
    let t = ch.abcd_dc.mul(&Abcd::shunt(Cx::new(g, 0.0)));
    (t.a + t.c.scale(rs)).inv().scale(scale).re
}

impl Default for EyeParams {
    fn default() -> Self {
        EyeParams { bitrate: 8e9, rise: 35e-12, prbs: 7, samples_per_ui: 32, rows: 96, tx: None, rx: None }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Eye {
    pub bitrate: f64,
    pub ui_ps: f64,
    /// histogram, `cols` = 2 UI, row 0 = `v_hi`
    pub cols: usize,
    pub rows: usize,
    pub hist: Vec<u32>,
    pub v_lo: f64,
    pub v_hi: f64,
    /// vertical opening at the best sampling phase (fraction of swing)
    pub height: f64,
    /// horizontal opening at the decision threshold (ps)
    pub width_ps: f64,
    /// column of the best sampling phase in the histogram
    pub best_col: usize,
    /// decision threshold (fraction of swing)
    pub threshold: f64,
    /// DC gain of the channel (fraction of swing)
    pub dc: f64,
    /// main cursor and largest pre/post cursors (fraction of swing)
    pub main_cursor: f64,
    pub isi_sum: f64,
    /// pulse response around the main cursor: (t ps relative to the cursor, v)
    pub pulse_t_ps: Vec<f64>,
    pub pulse_v: Vec<f64>,
    /// the step response behind it: DC point, band edge, precursor, settling
    pub td: TdCheck,
    /// crosstalk from other lanes (an N-port file behind a connector): included in `height` and
    /// `width_ps` as the worst case
    #[serde(skip_serializing_if = "Option::is_none")]
    pub xt: Option<EyeXt>,
    pub warnings: Vec<String>,
}

/// Worst-case crosstalk in an eye: every aggressor with its worst bit pattern and phase.
#[derive(Debug, Clone, Serialize)]
pub struct EyeXt {
    /// aggressor lanes and the peak crosstalk of each (fraction of the swing: Σ|cursors| at
    /// the aggressor's worst phase)
    pub aggressors: Vec<(String, f64)>,
    /// their sum: the eye closes by twice this (ones pulled down, zeros pulled up)
    pub peak: f64,
    /// the eye height without crosstalk
    pub height_alone: f64,
}

fn prbs(order: u32) -> Vec<u8> {
    // Fibonacci LFSR, taps per ITU-T O.150: 7 → x^7+x^6+1, 9 → x^9+x^5+1, 11 → x^11+x^9+1
    let (n, t) = match order {
        9 => (9, 5),
        11 => (11, 9),
        _ => (7, 6),
    };
    let len = (1usize << n) - 1;
    let mut s: u32 = (1 << n) - 1;
    (0..len)
        .map(|_| {
            let b = ((s >> (n - 1)) ^ (s >> (t - 1))) & 1;
            s = ((s << 1) | b) & ((1 << n) - 1);
            b as u8
        })
        .collect()
}

/// Eye of a channel between the ends of `ep` (None: the reference impedance), from the
/// cascade.
pub fn eye(ch: &Channel, ep: &EyeParams) -> Result<Eye, String> {
    if ch.freq.len() < 10 || ch.s21.len() != ch.freq.len() {
        return Err("no S-parameters".into());
    }
    let full = ch.abcd.len() == ch.freq.len();
    let (h, dc): (Vec<Cx>, Option<f64>) = match (&ep.tx, &ep.rx) {
        (None, None) => (ch.s21.clone(), full.then(|| ch.abcd_dc.s_params(ch.z_ref).1.re)),
        (tx, rx) if full => {
            let d = EndModel { r: ch.z_ref, c_pf: 0.0, l_nh: 0.0 };
            let (tx, rx) = (tx.as_ref().unwrap_or(&d), rx.as_ref().unwrap_or(&d));
            (transfer(ch, tx, rx), Some(transfer_dc(ch, tx, rx)))
        }
        _ => (ch.s21.clone(), None),
    };
    eye_of(&ch.freq, &h, dc, ch.delay_ps, ep)
}

/// Eye from a transfer function `h` on the channel grid (f_k = k·df) with its DC point `dc`
/// (None: extrapolated from the lowest frequency) and the channel's static delay (ps).
pub fn eye_of(freq: &[f64], h: &[Cx], dc: Option<f64>, delay_ps: f64, ep: &EyeParams) -> Result<Eye, String> {
    eye_with_xt(freq, h, dc, delay_ps, ep, &[])
}

/// As `eye_of`, with crosstalk from aggressors (label, transfer from the aggressor's emf to this
/// receiver, its value at 0 Hz; same units as `h`): each with its worst bit pattern at its worst
/// phase, summed (peak distortion) – the eye's height and width include it.
pub fn eye_with_xt(freq: &[f64], h: &[Cx], dc: Option<f64>, delay_ps: f64, ep: &EyeParams, xt: &[(String, Vec<Cx>, f64)]) -> Result<Eye, String> {
    if freq.len() < 10 || h.len() != freq.len() {
        return Err("no transfer function".into());
    }
    let mut warnings = Vec::new();
    let ui = 1.0 / ep.bitrate;
    let spu = ep.samples_per_ui.clamp(8, 128);
    let dt = ui / spu as f64;
    let df = freq[0];
    let fmax = *freq.last().unwrap();
    if ep.bitrate / 2.0 * 2.5 > fmax {
        warnings.push(format!(
            "frequency range up to {:.0} GHz is tight for {:.1} Gbit/s (Nyquist {:.1} GHz)",
            fmax / 1e9,
            ep.bitrate / 1e9,
            ep.bitrate / 2e9
        ));
    }
    // window: delay + settling, but well inside the 1/df period of the sweep
    let delay = delay_ps * 1e-12;
    let t_win = (3.0 * delay + 6.0 * ep.rise + 48.0 * ui).min(0.7 / df);
    let nt = (t_win / dt).ceil() as usize + spu;
    let mut step = Step::new(freq, h, dc, ep.rise);
    step.verify(0.8 * delay - 2.0 * ep.rise, t_win);
    warnings.extend(step.check.warnings());
    let s = step.samples(dt, nt);
    let dc = step.check.dc.abs();
    // single-bit pulse response
    let p: Vec<f64> = (0..nt).map(|i| s[i] - if i >= spu { s[i - spu] } else { 0.0 }).collect();
    let (imax, &vpk) = p.iter().enumerate().max_by(|a, b| a.1.partial_cmp(b.1).unwrap()).unwrap();
    // main cursor = middle of the pulse top (≥ 95 % of the peak), not the
    // first or last ripple maximum on a flat top
    let top = 0.95 * vpk;
    let (mut a0, mut a1) = (imax, imax);
    while a0 > 0 && p[a0 - 1] >= top {
        a0 -= 1;
    }
    while a1 + 1 < nt && p[a1 + 1] >= top {
        a1 += 1;
    }
    let ipk = (a0 + a1) / 2;
    // cursors at the main-cursor phase
    let mut isi = 0.0;
    let mut k = ipk % spu;
    while k < nt {
        if k != ipk {
            isi += p[k].abs();
        }
        k += spu;
    }
    let pulse_lo = ipk.saturating_sub(4 * spu);
    let pulse_hi = (ipk + 12 * spu).min(nt);
    let pulse_t_ps: Vec<f64> = (pulse_lo..pulse_hi).map(|i| (i as f64 - ipk as f64) * dt * 1e12).collect();
    let pulse_v: Vec<f64> = p[pulse_lo..pulse_hi].to_vec();

    // periodic PRBS response, one period, sample n ↔ time n·dt
    let bits = prbs(ep.prbs);
    let nb = bits.len();
    let np = nb * spu;
    let mut v = vec![0.0; np];
    for (n, out) in v.iter_mut().enumerate() {
        // v[n] = Σ_j b_j · p[n − j·spu], periodic in the PRBS
        let mut acc = 0.0;
        let mut m = n % spu;
        while m < nt {
            let j = (n as isize - m as isize).div_euclid(spu as isize).rem_euclid(nb as isize) as usize;
            if bits[j] == 1 {
                acc += p[m];
            }
            m += spu;
        }
        *out = acc;
    }
    // crosstalk: per aggressor the largest sum of |cursors| over the phases
    let xt_each: Vec<(String, f64)> = xt
        .iter()
        .map(|(label, hx, dcx)| {
            let s = Step::new(freq, hx, Some(*dcx), ep.rise).samples(dt, nt);
            let q: Vec<f64> = (0..nt).map(|i| s[i] - if i >= spu { s[i - spu] } else { 0.0 }).collect();
            let worst = (0..spu).map(|d| (d..nt).step_by(spu).map(|i| q[i].abs()).sum::<f64>()).fold(0.0, f64::max);
            (label.clone(), worst)
        })
        .collect();
    let xpk: f64 = xt_each.iter().map(|x| x.1).sum();
    // the bit decided at sample n with phase offset d from the main cursor:
    // n = k·spu + ipk + d  →  bit k
    let thr = dc / 2.0;
    let mut best_alone = f64::NEG_INFINITY;
    let half = (spu / 2) as isize;
    let mut best = (f64::NEG_INFINITY, 0isize);
    let mut open: Vec<bool> = Vec::with_capacity(spu);
    for d in -half..half {
        let (mut lo1, mut hi0) = (f64::INFINITY, f64::NEG_INFINITY);
        for kb in 0..nb {
            let n = (kb as isize * spu as isize + ipk as isize + d).rem_euclid(np as isize) as usize;
            if bits[kb] == 1 {
                lo1 = lo1.min(v[n]);
            } else {
                hi0 = hi0.max(v[n]);
            }
        }
        best_alone = best_alone.max(lo1 - hi0);
        let (lo1, hi0) = (lo1 - xpk, hi0 + xpk);
        let h = lo1 - hi0;
        open.push(lo1 > thr && hi0 < thr);
        if h > best.0 {
            best = (h, d);
        }
    }
    // width: contiguous open phases around the best one
    let bi = (best.1 + half) as usize;
    let mut w = 0;
    if open[bi] {
        w = 1;
        let mut j = bi;
        while j > 0 && open[j - 1] {
            j -= 1;
            w += 1;
        }
        let mut j = bi;
        while j + 1 < open.len() && open[j + 1] {
            j += 1;
            w += 1;
        }
    }
    // histogram: 2 UI, best phase in the middle of each UI
    let cols = 2 * spu;
    let rows = ep.rows.clamp(32, 256);
    let vmin = v.iter().cloned().fold(f64::INFINITY, f64::min);
    let vmax = v.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let m = 0.08 * (vmax - vmin).max(1e-3);
    let (v_lo, v_hi) = (vmin - m, vmax + m);
    let row = |x: f64| (((v_hi - x) / (v_hi - v_lo)) * (rows - 1) as f64).round().clamp(0.0, (rows - 1) as f64) as usize;
    let mut hist = vec![0u32; rows * cols];
    let centre = ipk as isize + best.1; // sampling instant of bit 0
    for n in 0..np {
        let c = ((n as isize - centre + (spu / 2) as isize).rem_euclid(cols as isize)) as usize;
        let n2 = (n + 1) % np;
        let (r0, r1) = (row(v[n]), row(v[n2]));
        let (a, b) = if r0 <= r1 { (r0, r1) } else { (r1, r0) };
        // fill to the next sample so steep edges stay continuous
        for r in a..=b {
            hist[r * cols + c] += 1;
        }
    }
    if best.0 <= 0.0 {
        warnings.push("eye closed (without equalisation)".into());
    }
    Ok(Eye {
        bitrate: ep.bitrate,
        ui_ps: ui * 1e12,
        cols,
        rows,
        hist,
        v_lo,
        v_hi,
        height: best.0.max(0.0),
        width_ps: w as f64 * dt * 1e12,
        best_col: spu / 2,
        threshold: thr,
        dc,
        main_cursor: p[ipk],
        isi_sum: isi,
        pulse_t_ps,
        pulse_v,
        td: step.check.clone(),
        xt: (!xt.is_empty()).then(|| EyeXt { aggressors: xt_each, peak: xpk, height_alone: best_alone.max(0.0) }),
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connector_end_models_and_rest_line() {
        let prof = Some(EndModel { r: 42.5, c_pf: 0.5, l_nh: 0.0 });
        let ce = |m: &str| ConnEnd { model: m.into(), ..Default::default() };
        assert_eq!(ce("profile").resolve(prof, Some((85.0, true))).unwrap().unwrap().r, 42.5);
        assert_eq!(ce("matched").resolve(prof, Some((85.0, true))).unwrap().unwrap().r, 42.5);
        assert_eq!(ce("matched").resolve(prof, Some((50.0, false))).unwrap().unwrap().r, 50.0);
        assert!(ce("matched").resolve(prof, None).is_err());
        assert_eq!(ce("high_impedance").resolve(prof, None).unwrap().unwrap().r, 0.0);
        assert!(ce("custom").resolve(prof, None).is_err());
        let c = ConnEnd { model: "custom".into(), r_ohm: Some(60.0), c_pf: Some(0.3), ..Default::default() };
        assert_eq!(c.resolve(prof, None).unwrap().unwrap().c_pf, 0.3);
        assert!(ce("bogus").resolve(prof, None).is_err());
        // the rest line: matched, its |S21| at Nyquist is the given loss, √f + f below it
        let r = ConnEnd { rest_loss_db: Some(6.0), ..ce("profile") };
        assert!(r.has_rest() && !ce("profile").has_rest());
        let s21 = |f: f64| 20.0 * r.rest_abcd(f, 4e9, 50.0).unwrap().s_params(50.0).1.abs().log10();
        assert!((s21(4e9) + 6.0).abs() < 1e-9, "{}", s21(4e9));
        assert!((s21(1e9) + 6.0 * (0.5 * 0.5 + 0.5 * 0.25)).abs() < 1e-9);
    }

    #[test]
    fn prbs7_period_and_balance() {
        let b = prbs(7);
        assert_eq!(b.len(), 127);
        assert_eq!(b.iter().filter(|&&x| x == 1).count(), 64);
    }
}

/// Test helpers shared with other modules.
#[cfg(test)]
pub(crate) mod tests_support {
    use super::*;

    /// A channel with only frequencies, S21 and a delay.
    pub(crate) fn channel_from(freq: Vec<f64>, s21: Vec<Cx>, delay_ps: f64) -> Channel {
        Channel {
            net: 0,
            name: String::new(),
            diff: false,
            partner: None,
            z_ref: 50.0,
            length_mm: 0.0,
            delay_ps,
            vias: 0,
            segments: vec![],
            s21_db: vec![],
            s11_db: vec![],
            s22_db: vec![],
            il_marks: vec![],
            tdr_t_ps: vec![],
            tdr_z: vec![],
            tdr_x_mm: vec![],
            tdr_end_mm: 0.0,
            loss_db_per_in_at: vec![],
            via_info: vec![],
            field_share: 0.0,
            warnings: vec![],
            path: None,
            s11: vec![Cx::new(0.0, 0.0); freq.len()],
            s21,
            s22: vec![Cx::new(0.0, 0.0); freq.len()],
            abcd: vec![],
            abcd_dc: Abcd::identity(),
            taps: vec![],
            pieces: vec![],
            freq,
        }
    }
}

#[cfg(test)]
mod tests2 {
    use super::tests_support::channel_from;
    use super::*;

    #[test]
    fn transfer_matched_equals_s21_and_open_end() {
        let n = 100;
        let df = 10e9 / n as f64;
        let freq: Vec<f64> = (1..=n).map(|k| k as f64 * df).collect();
        // 50 Ω lossless line, 10 mm, εeff 4
        let (l, c) = (50.0 * 2.0 / crate::num::C0, 2.0 / (50.0 * crate::num::C0));
        let abcd: Vec<Abcd> = freq.iter().map(|&f| Abcd::line(0.0, l, 0.0, c, 2.0 * PI * f, 0.01)).collect();
        let s21: Vec<Cx> = abcd.iter().map(|m| m.s_params(50.0).1).collect();
        let mut ch = channel_from(freq, s21.clone(), 67.0);
        ch.abcd = abcd;
        let m = EndModel { r: 50.0, c_pf: 0.0, l_nh: 0.0 };
        let h = transfer(&ch, &m, &m);
        for (a, b) in h.iter().zip(&s21) {
            assert!((*a - *b).abs() < 1e-9);
        }
        let open = EndModel { r: 0.0, c_pf: 0.0, l_nh: 0.0 };
        let h2 = transfer(&ch, &m, &open);
        // high-impedance receiver: open-circuit swing, DC level 1
        assert!((h2[0].abs() - 1.0).abs() < 0.01, "{}", h2[0].abs());
    }

    #[test]
    fn ideal_delay_line_gives_open_eye() {
        let n = 500;
        let df = 25e9 / n as f64;
        let freq: Vec<f64> = (1..=n).map(|k| k as f64 * df).collect();
        let tau = 100e-12;
        let s21: Vec<Cx> = freq.iter().map(|&f| Cx::new((2.0 * PI * f * tau).cos(), -(2.0 * PI * f * tau).sin())).collect();
        let ch = channel_from(freq, s21, 100.0);
        let e = eye(&ch, &EyeParams { bitrate: 5e9, rise: 35e-12, ..Default::default() }).unwrap();
        assert!(e.height > 0.95, "height {}", e.height);
        assert!(e.width_ps > 0.7 * e.ui_ps, "width {}", e.width_ps);
    }

    #[test]
    fn lossy_channel_closes_eye_partly() {
        // first-order low pass, f3 = 2 GHz, at 10 Gbit/s → strong ISI
        let n = 500;
        let df = 25e9 / n as f64;
        let freq: Vec<f64> = (1..=n).map(|k| k as f64 * df).collect();
        let s21: Vec<Cx> = freq.iter().map(|&f| Cx::new(1.0, f / 2e9).inv()).collect();
        let ch = channel_from(freq, s21, 0.0);
        let e = eye(&ch, &EyeParams { bitrate: 10e9, rise: 20e-12, ..Default::default() }).unwrap();
        assert!(e.height < 0.6, "height {}", e.height);
        assert!(e.isi_sum > 0.2);
    }
}

// --- channel check over many nets -------------------------------------------------------

/// One row of the board-level channel check.
#[derive(Debug, Clone, Serialize)]
pub struct CheckRow {
    pub net: u32,
    pub name: String,
    pub diff: bool,
    pub gbps: f64,
    pub length_mm: f64,
    pub delay_ps: f64,
    pub vias: usize,
    pub nyquist_ghz: f64,
    /// insertion loss at Nyquist (dB, negative)
    pub il_nyq_db: f64,
    /// worst return loss up to 1.5 × Nyquist (dB, positive)
    pub rl_db: f64,
    pub eye_height: f64,
    pub eye_width_ui: f64,
    pub skew_ps: Option<f64>,
    pub skew_uncertain: bool,
    /// lowest via-stub resonance (GHz)
    pub stub_ghz: Option<f64>,
    pub warnings: usize,
    pub error: Option<String>,
    /// false when no eye was computed
    pub eye: bool,
    /// the lane ends at a connector: the eye covers the board share only, with the connector
    /// end model (termination or driver) – information, not judged
    pub eye_board_share: bool,
    /// side stubs and branches in the channel (open stubs: unused pads, parts not fitted,
    /// the other path of a fork)
    pub stubs: Vec<String>,
    /// how the channel is modelled: "single-ended", "4-port" (a pair: both lines, coupling, mode
    /// conversion) or "odd mode" (a pair as a 2-port)
    pub model: &'static str,
    /// pairs with the 4-port: worst mode conversion Scd21 up to 1.5 × Nyquist (dB); information
    pub mode_conversion_db: Option<f64>,
    /// multi-drop: the eye at every receiver (the row's eye is the worst)
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub receivers: Vec<RxEye>,
    /// crosstalk from other lanes of an N-port file behind the connector: the worst-case peak
    /// (fraction of the swing; the eye height includes twice this)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub xt_peak: Option<f64>,
    /// vias of the channel that cross a GND/supply plane pair no return via bridges
    /// (`via::ViaInfo::open_pairs`), for the row's warning
    #[serde(skip)]
    pub open_vias: Vec<crate::via::ViaInfo>,
}

/// The eye at one receiver of a multi-drop line.
#[derive(Debug, Clone, Serialize)]
pub struct RxEye {
    /// the receiver's pin, e.g. "U3.5"
    pub pin: String,
    pub eye_height: f64,
    pub eye_width_ui: f64,
    /// |H| at Nyquist between the ends (dB, 0 dB = the full swing at the receiver)
    pub gain_nyq_db: f64,
}

/// Largest value of `curve` (on `freq`) up to `f_hi`.
pub(crate) fn worst_up_to(freq: &[f64], curve: &[f64], f_hi: f64) -> f64 {
    freq.iter().zip(curve).filter(|(&f, _)| f <= f_hi * 1.0001).map(|(_, &v)| v).fold(f64::NEG_INFINITY, f64::max)
}

pub(crate) fn empty_row(net: u32, name: String, gbps: f64) -> CheckRow {
    CheckRow {
        net,
        name,
        diff: false,
        gbps,
        length_mm: 0.0,
        delay_ps: 0.0,
        vias: 0,
        nyquist_ghz: gbps / 2.0,
        il_nyq_db: 0.0,
        rl_db: 0.0,
        eye_height: 0.0,
        eye_width_ui: 0.0,
        skew_ps: None,
        skew_uncertain: false,
        stub_ghz: None,
        warnings: 0,
        error: None,
        eye: true,
        eye_board_share: false,
        stubs: vec![],
        model: "single-ended",
        mode_conversion_db: None,
        receivers: vec![],
        xt_peak: None,
        open_vias: vec![],
    }
}

/// Fill loss, return loss, stubs and (if `with_eye`) the eye of a row from a channel.
pub(crate) fn fill_row(row: &mut CheckRow, ch: &crate::rf::Channel, gbps: f64, rise: f64, ends: (Option<EndModel>, Option<EndModel>), with_eye: bool) {
    row.diff = ch.diff;
    row.model = if ch.diff { "odd mode" } else { "single-ended" };
    row.length_mm = ch.length_mm;
    row.delay_ps = ch.delay_ps;
    row.vias = ch.vias;
    row.warnings = ch.warnings.len();
    row.open_vias = ch.via_info.iter().filter(|v| !v.open_pairs.is_empty()).cloned().collect();
    row.stubs = ch.segments.iter().filter(|s| s.kind == "branch").filter_map(|s| s.part.clone()).collect();
    let fn_ = gbps * 0.5e9;
    let df = ch.freq[0];
    let k = ((fn_ / df).round() as usize).clamp(1, ch.freq.len()) - 1;
    row.il_nyq_db = ch.s21_db[k];
    let k2 = ((1.5 * fn_ / df).round() as usize).clamp(1, ch.freq.len());
    row.rl_db = -ch.s11_db[..k2].iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    row.stub_ghz = ch.via_info.iter().filter(|v| v.stub_mm > 0.01).filter_map(|v| v.stub_f_ghz).fold(None, |a: Option<f64>, b| Some(a.map_or(b, |x| x.min(b))));
    row.eye = with_eye;
    if !with_eye {
        return;
    }
    let (tx, rx) = ends;
    match eye(ch, &EyeParams { bitrate: gbps * 1e9, rise, tx, rx, ..Default::default() }) {
        Ok(e) => {
            row.eye_height = e.height;
            row.eye_width_ui = e.width_ps / e.ui_ps;
        }
        Err(e) => row.error = Some(e),
    }
}

pub fn check_net(
    board: &crate::board::Board,
    an: &crate::extract::Analysis,
    field: &std::collections::HashMap<String, crate::extract::FieldResult>,
    net: u32,
    target: Option<(f64, bool)>,
    rp: &crate::rf::RfParams,
    gbps: f64,
    ends: (Option<EndModel>, Option<EndModel>, Option<f64>),
) -> CheckRow {
    let mut row = empty_row(net, board.net_name(net).to_string(), gbps);
    // enough bandwidth for the eye, at least 2.5 × Nyquist
    let rp = crate::rf::RfParams { fmax: rp.fmax.max(gbps * 1e9 * 1.25), ..rp.clone() };
    let mut ch = match crate::rf::channel(board, an, net, field, target, &rp) {
        Ok(c) => c,
        Err(e) => {
            row.error = Some(e);
            return row;
        }
    };
    let (tx, rx, rise) = ends;
    // a pair: the 4-port of both lines (mode conversion), as for lanes
    let pair = if ch.diff {
        match crate::rf::mixed_mode(board, an, net, field, target, &rp, false, None) {
            Ok(m) => Some(m.pair_net()),
            Err(e) => {
                ch.warnings.push(format!("pair computed in the odd mode (2-port): {e}"));
                None
            }
        }
    } else {
        None
    };
    let mm = pair.as_ref().map(|p| p.mixed());
    if let Some(mm) = &mm {
        crate::lanenet::apply_pair(&mut ch, mm, rp.rise);
    }
    fill_row(&mut row, &ch, gbps, rise.unwrap_or(rp.rise), (tx, rx), pair.is_none());
    if let (Some(p), Some(mm)) = (&pair, &mm) {
        row.model = "4-port";
        let scd: Vec<f64> = mm.iter().map(|m| 20.0 * m[3][0].abs().max(1e-12).log10()).collect();
        row.mode_conversion_db = Some(worst_up_to(&ch.freq, &scd, 1.5 * gbps * 0.5e9));
        row.eye = true;
        let d = EndModel { r: ch.z_ref, c_pf: 0.0, l_nh: 0.0 };
        let ep = EyeParams { bitrate: gbps * 1e9, rise: rise.unwrap_or(rp.rise), tx, rx, ..Default::default() };
        match crate::lanenet::ends_transfer(&ch.freq, &[p.block()], 2, &tx.unwrap_or(d), &rx.unwrap_or(d)).and_then(|(h, dc)| eye_of(&ch.freq, &h, Some(dc), ch.delay_ps, &ep)) {
            Ok(e) => {
                row.eye_height = e.height;
                row.eye_width_ui = e.width_ps / e.ui_ps;
            }
            Err(e) => row.error = Some(e),
        }
    }
    if let (true, Some(pn)) = (ch.diff, ch.partner) {
        let rq = crate::rf::RfParams { npts: 10, ..rp.clone() };
        if let Ok(pc) = crate::rf::channel(board, an, pn, field, target, &rq) {
            row.skew_ps = Some(ch.delay_ps - pc.delay_ps);
            row.skew_uncertain = ch.warnings.iter().chain(&pc.warnings).any(|w| w.starts_with("net branches") || w.starts_with("net splits"));
        }
    }
    row
}

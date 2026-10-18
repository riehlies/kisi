//! Eye diagram of a channel (milestone M5): NRZ PRBS through S21.
//!
//! The step response of S21 (Gaussian transmitter edge of the given 10-90 %
//! rise time) gives the single-bit pulse response; a periodic PRBS is
//! superposed on it (linear, time-invariant channel, matched terminations,
//! no equalisation). Amplitudes are fractions of the transmitter swing.

use crate::rf::{Channel, Cx};
use serde::{Deserialize, Serialize};
use std::f64::consts::{LN_2, PI};

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
}

impl Default for EyeParams {
    fn default() -> Self {
        EyeParams { bitrate: 8e9, rise: 35e-12, prbs: 7, samples_per_ui: 32, rows: 96 }
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
    pub warnings: Vec<String>,
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

/// Step response of S21 on t = i·dt, i = 0..nt (unit step at t = 0).
fn step_response(freq: &[f64], s21: &[Cx], rise: f64, dt: f64, nt: usize) -> Vec<f64> {
    let df = freq[0];
    let f3 = 0.3394 / rise;
    let s0 = s21[0].abs(); // DC gain extrapolated from the lowest frequency
    let terms: Vec<(f64, Cx)> = freq
        .iter()
        .zip(s21)
        .map(|(&f, &s)| (2.0 * PI * f, s.scale((-(LN_2 / 2.0) * (f / f3).powi(2)).exp())))
        .filter(|(_, s)| s.abs() > 1e-7)
        .collect();
    let big_f = |t: f64| -> f64 {
        let mut acc = s0 * t;
        for &(w, sh) in &terms {
            let e = Cx::new((w * t).cos(), (w * t).sin());
            acc += 2.0 * (sh * e / Cx::new(0.0, w)).re;
        }
        acc * df
    };
    let f0 = big_f(-3.0 * rise);
    (0..nt).map(|i| big_f(i as f64 * dt) - f0).collect()
}

pub fn eye(ch: &Channel, ep: &EyeParams) -> Result<Eye, String> {
    if ch.freq.len() < 10 || ch.s21.len() != ch.freq.len() {
        return Err("keine S-Parameter".into());
    }
    let mut warnings = Vec::new();
    let ui = 1.0 / ep.bitrate;
    let spu = ep.samples_per_ui.clamp(8, 128);
    let dt = ui / spu as f64;
    let df = ch.freq[0];
    let fmax = *ch.freq.last().unwrap();
    if ep.bitrate / 2.0 * 2.5 > fmax {
        warnings.push(format!(
            "Frequenzbereich bis {:.0} GHz ist knapp für {:.1} Gbit/s (Nyquist {:.1} GHz)",
            fmax / 1e9,
            ep.bitrate / 1e9,
            ep.bitrate / 2e9
        ));
    }
    // window: delay + settling, but well inside the 1/df period of the sweep
    let delay = ch.delay_ps * 1e-12;
    let t_win = (3.0 * delay + 6.0 * ep.rise + 48.0 * ui).min(0.7 / df);
    let nt = (t_win / dt).ceil() as usize + spu;
    let s = step_response(&ch.freq, &ch.s21, ep.rise, dt, nt);
    let dc = ch.s21[0].abs();
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
    // the bit decided at sample n with phase offset d from the main cursor:
    // n = k·spu + ipk + d  →  bit k
    let thr = dc / 2.0;
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
        warnings.push("Auge geschlossen (ohne Entzerrung)".into());
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
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prbs7_period_and_balance() {
        let b = prbs(7);
        assert_eq!(b.len(), 127);
        assert_eq!(b.iter().filter(|&&x| x == 1).count(), 64);
    }

    pub(super) fn channel_from(freq: Vec<f64>, s21: Vec<Cx>, delay_ps: f64) -> Channel {
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
            il_marks: vec![],
            tdr_t_ps: vec![],
            tdr_z: vec![],
            tdr_x_mm: vec![],
            loss_db_per_in_at: vec![],
            via_info: vec![],
            field_share: 0.0,
            warnings: vec![],
            s11: vec![Cx::new(0.0, 0.0); freq.len()],
            s21,
            freq,
        }
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
}

pub fn check_net(
    board: &crate::board::Board,
    an: &crate::extract::Analysis,
    field: &std::collections::HashMap<String, crate::extract::FieldResult>,
    net: u32,
    target: Option<(f64, bool)>,
    rp: &crate::rf::RfParams,
    gbps: f64,
) -> CheckRow {
    let mut row = CheckRow {
        net,
        name: board.net_name(net).to_string(),
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
    };
    // enough bandwidth for the eye, at least 2.5 × Nyquist
    let rp = crate::rf::RfParams { fmax: rp.fmax.max(gbps * 1e9 * 1.25), ..rp.clone() };
    let ch = match crate::rf::channel(board, an, net, field, target, &rp) {
        Ok(c) => c,
        Err(e) => {
            row.error = Some(e);
            return row;
        }
    };
    row.diff = ch.diff;
    row.length_mm = ch.length_mm;
    row.delay_ps = ch.delay_ps;
    row.vias = ch.vias;
    row.warnings = ch.warnings.len();
    let fn_ = gbps * 0.5e9;
    let df = ch.freq[0];
    let k = ((fn_ / df).round() as usize).clamp(1, ch.freq.len()) - 1;
    row.il_nyq_db = ch.s21_db[k];
    let k2 = ((1.5 * fn_ / df).round() as usize).clamp(1, ch.freq.len());
    row.rl_db = -ch.s11_db[..k2].iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    row.stub_ghz = ch.via_info.iter().filter(|v| v.stub_mm > 0.01).filter_map(|v| v.stub_f_ghz).fold(None, |a: Option<f64>, b| Some(a.map_or(b, |x| x.min(b))));
    match eye(&ch, &EyeParams { bitrate: gbps * 1e9, rise: rp.rise, ..Default::default() }) {
        Ok(e) => {
            row.eye_height = e.height;
            row.eye_width_ui = e.width_ps / e.ui_ps;
        }
        Err(e) => row.error = Some(e),
    }
    if let (true, Some(pn)) = (ch.diff, ch.partner) {
        let rq = crate::rf::RfParams { npts: 10, ..rp.clone() };
        if let Ok(pc) = crate::rf::channel(board, an, pn, field, target, &rq) {
            row.skew_ps = Some(ch.delay_ps - pc.delay_ps);
            row.skew_uncertain = ch.warnings.iter().chain(&pc.warnings).any(|w| w.contains("verzweigt") || w.contains("zerfallen"));
        }
    }
    row
}

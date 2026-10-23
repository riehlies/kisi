//! Reading Touchstone files (S-parameters, versions 1.x and 2.x) for the channel behind a
//! connector: a 2-port (single-ended lane, or the same 2-port on every line of a pair), a 4-port
//! of a pair, or an N-port for all lanes of a connector with a port map per pin (`eye::TsRef`);
//! the lane network (`kisi_si::lanenet`) takes 4-ports and N-ports as S-parameter blocks.

use super::{Abcd, Cx};
use std::f64::consts::PI;

/// S-parameters of a file: `s[k]` is the n×n matrix (row-major) at `freq[k]` (Hz).
#[derive(Debug, Clone)]
pub struct Touchstone {
    pub name: String,
    pub nports: usize,
    pub z0: Vec<f64>,
    pub freq: Vec<f64>,
    pub s: Vec<Vec<Cx>>,
    pub warnings: Vec<String>,
}

/// Where the two lines of a pair are in a 4-port: "1-3,2-4" (P 1→3, N 2→4) or
/// "1-2,3-4" (P 1→2, N 3→4).
pub const PORT_ORDERS: &[&str] = &["1-3,2-4", "1-2,3-4"];

fn nports_from_name(name: &str) -> Option<usize> {
    let ext = name.rsplit('.').next()?.to_ascii_lowercase();
    let mid = ext.strip_prefix('s')?.strip_suffix('p')?;
    mid.parse().ok()
}

/// Parse a Touchstone file. `name` (the file name) gives the number of ports for version 1
/// files (`.s2p`, `.s4p`).
pub fn parse_touchstone(text: &str, name: &str) -> Result<Touchstone, String> {
    let (mut unit, mut fmt, mut r0) = (1e9, "MA".to_string(), 50.0);
    let mut param_s = true;
    let mut v2 = false;
    let mut nports = nports_from_name(name);
    let mut order_21_12 = true; // version 1 two-ports: S11 S21 S12 S22
    let mut refs: Vec<f64> = vec![];
    let mut in_ref = false;
    let mut nums: Vec<f64> = vec![];
    let mut option_seen = false;
    let mut warnings = vec![];
    for raw in text.lines() {
        let line = raw.split('!').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') {
            in_ref = false;
            let close = line.find(']').ok_or_else(|| format!("{name}: bad keyword line '{line}'"))?;
            let key = line[1..close].trim().to_ascii_lowercase();
            let rest = line[close + 1..].trim();
            match key.as_str() {
                "version" => v2 = true,
                "number of ports" => nports = Some(rest.parse().map_err(|_| format!("{name}: [Number of Ports] '{rest}'"))?),
                "two-port data order" => order_21_12 = rest.eq_ignore_ascii_case("21_12"),
                "reference" => {
                    in_ref = true;
                    refs.extend(rest.split_whitespace().filter_map(|t| t.parse::<f64>().ok()));
                }
                "matrix format" if !rest.eq_ignore_ascii_case("full") => return Err(format!("{name}: [Matrix Format] {rest} is not supported (only Full)")),
                "mixed-mode order" => return Err(format!("{name}: mixed-mode files are not supported; give the single-ended 4-port")),
                "noise data" | "end" => break,
                _ => {}
            }
            continue;
        }
        if line.starts_with('#') {
            option_seen = true;
            for t in line[1..].split_whitespace() {
                match t.to_ascii_uppercase().as_str() {
                    "HZ" => unit = 1.0,
                    "KHZ" => unit = 1e3,
                    "MHZ" => unit = 1e6,
                    "GHZ" => unit = 1e9,
                    "S" => param_s = true,
                    "Y" | "Z" | "H" | "G" => param_s = false,
                    "DB" | "MA" | "RI" => fmt = t.to_ascii_uppercase(),
                    "R" => {}
                    x => {
                        if let Ok(v) = x.parse::<f64>() {
                            r0 = v;
                        }
                    }
                }
            }
            continue;
        }
        if in_ref && refs.len() < nports.unwrap_or(0) {
            refs.extend(line.split_whitespace().filter_map(|t| t.parse::<f64>().ok()));
            continue;
        }
        for t in line.split_whitespace() {
            nums.push(t.parse::<f64>().map_err(|_| format!("{name}: not a number: '{t}'"))?);
        }
    }
    if !option_seen {
        warnings.push("no option line (#): GHz, S, MA, R 50 assumed".into());
    }
    if !param_s {
        return Err(format!("{name}: only S-parameters are read (the option line names Y, Z, H or G)"));
    }
    let n = nports.ok_or_else(|| format!("{name}: number of ports unknown (name the file .s2p / .s4p or give [Number of Ports])"))?;
    if n == 0 || n > 64 {
        return Err(format!("{name}: {n}-port – kisi reads 1 to 64 ports"));
    }
    let per = 1 + 2 * n * n;
    let (mut freq, mut s) = (vec![], vec![]);
    let mut i = 0;
    while i + per <= nums.len() {
        let f = nums[i] * unit;
        // version 1 two-ports may carry noise data after the network data: frequencies
        // restart lower
        if freq.last().is_some_and(|&p| f <= p) {
            if n == 2 && !v2 {
                break;
            }
            return Err(format!("{name}: frequencies not increasing at {f} Hz"));
        }
        let mut m = vec![Cx::new(0.0, 0.0); n * n];
        for k in 0..n * n {
            let (a, b) = (nums[i + 1 + 2 * k], nums[i + 2 + 2 * k]);
            let v = match fmt.as_str() {
                "RI" => Cx::new(a, b),
                "DB" => {
                    let mag = 10f64.powf(a / 20.0);
                    Cx::new(mag * (b * PI / 180.0).cos(), mag * (b * PI / 180.0).sin())
                }
                _ => Cx::new(a * (b * PI / 180.0).cos(), a * (b * PI / 180.0).sin()),
            };
            // position in the row-major matrix
            let (r, c) = if n == 2 && order_21_12 { [(0, 0), (1, 0), (0, 1), (1, 1)][k] } else { (k / n, k % n) };
            m[r * n + c] = v;
        }
        freq.push(f);
        s.push(m);
        i += per;
    }
    if freq.len() < 2 {
        return Err(format!("{name}: fewer than two frequency points"));
    }
    if i < nums.len() && !(n == 2 && !v2) {
        warnings.push(format!("{} numbers at the end ignored", nums.len() - i));
    }
    let z0 = if refs.len() >= n { refs[..n].to_vec() } else { vec![r0; n] };
    let mut ts = Touchstone { name: name.to_string(), nports: n, z0, freq, s, warnings };
    if let Some(w) = ts.passivity_note() {
        ts.warnings.push(w);
    }
    Ok(ts)
}

/// Interpolation of one S-parameter over frequency: linear in magnitude and in the unwrapped
/// phase; below the first point the magnitude is held and the phase goes linearly to 0 at DC,
/// above the last point the magnitude is held and the phase continues with the last slope.
fn interp(freq: &[f64], vals: &[Cx], f: f64) -> Cx {
    let mag: Vec<f64> = vals.iter().map(|v| v.abs()).collect();
    let mut ph: Vec<f64> = vals.iter().map(|v| v.arg()).collect();
    for k in 1..ph.len() {
        let mut d = ph[k] - ph[k - 1];
        while d > PI {
            d -= 2.0 * PI;
        }
        while d < -PI {
            d += 2.0 * PI;
        }
        ph[k] = ph[k - 1] + d;
    }
    let n = freq.len();
    let (m, p) = if f <= freq[0] {
        (mag[0], if freq[0] > 0.0 { ph[0] * f / freq[0] } else { ph[0] })
    } else if f >= freq[n - 1] {
        let slope = (ph[n - 1] - ph[n - 2]) / (freq[n - 1] - freq[n - 2]);
        (mag[n - 1], ph[n - 1] + slope * (f - freq[n - 1]))
    } else {
        let k = freq.partition_point(|&x| x <= f).max(1) - 1;
        let t = (f - freq[k]) / (freq[k + 1] - freq[k]);
        (mag[k] + t * (mag[k + 1] - mag[k]), ph[k] + t * (ph[k + 1] - ph[k]))
    };
    Cx::new(m * p.cos(), m * p.sin())
}

/// ABCD of a 2-port from S (real reference `z0`, the same at both ports).
pub fn abcd_from_s(s11: Cx, s12: Cx, s21: Cx, s22: Cx, z0: f64) -> Abcd {
    let one = Cx::new(1.0, 0.0);
    let d = (s21.scale(2.0)).inv();
    Abcd {
        a: ((one + s11) * (one - s22) + s12 * s21) * d,
        b: ((one + s11) * (one + s22) - s12 * s21) * d.scale(z0),
        c: ((one - s11) * (one - s22) - s12 * s21) * d.scale(1.0 / z0),
        d: ((one - s11) * (one + s22) + s12 * s21) * d,
    }
}

/// The network seen from the other side (port 2 as input).
pub fn reversed(m: &Abcd) -> Abcd {
    let det = m.a * m.d - m.b * m.c;
    let k = det.inv();
    Abcd { a: m.d * k, b: m.b * k, c: m.c * k, d: m.a * k }
}

impl Touchstone {
    /// The per-line ABCD of the file at the given frequencies, port 1 (near end) at the
    /// connector: a 2-port as it is (for a pair: the same 2-port on each line, without
    /// coupling); a 4-port of a pair reduced to the differential mode and expressed per
    /// line (odd mode, half the differential impedance). `flip` swaps the near and far end.
    /// Returns the matrices and notes (extrapolation, reference impedances).
    pub fn per_line_abcd(&self, freq: &[f64], diff: bool, port_order: Option<&str>, flip: bool) -> Result<(Vec<Abcd>, Vec<String>), String> {
        let mut notes = vec![];
        let (fmin, fmax) = (self.freq[0], *self.freq.last().unwrap());
        if freq.iter().any(|&f| f > fmax * 1.001) {
            notes.push(format!("{} ends at {:.2} GHz: above it extrapolated (magnitude held)", self.name, fmax / 1e9));
        }
        if freq.iter().any(|&f| f < fmin * 0.999) && fmin > 0.0 {
            notes.push(format!("{} starts at {:.3} GHz: towards DC extrapolated", self.name, fmin / 1e9));
        }
        let el = |r: usize, c: usize| -> Vec<Cx> { self.s.iter().map(|m| m[r * self.nports + c]).collect() };
        let mut out = Vec::with_capacity(freq.len());
        match self.nports {
            2 => {
                let z0 = self.z0[0];
                if (self.z0[1] - z0).abs() > 1e-9 {
                    notes.push(format!("{}: different reference impedances per port; {z0} Ω used", self.name));
                }
                let (s11, s12, s21, s22) = (el(0, 0), el(0, 1), el(1, 0), el(1, 1));
                for &f in freq {
                    out.push(abcd_from_s(interp(&self.freq, &s11, f), interp(&self.freq, &s12, f), interp(&self.freq, &s21, f), interp(&self.freq, &s22, f), z0));
                }
                if diff {
                    notes.push(format!("{}: the same 2-port on each line of the pair, no coupling", self.name));
                }
            }
            4 => {
                if !diff {
                    return Err(format!("{}: a 4-port needs a differential lane (give a 2-port for single-ended lanes)", self.name));
                }
                let order = port_order.unwrap_or("1-3,2-4");
                // indices (0-based) of P near, N near, P far, N far
                let [pn, nn, pf, nf] = match order {
                    "1-3,2-4" => [0, 1, 2, 3],
                    "1-2,3-4" => [0, 2, 1, 3],
                    o => return Err(format!("unknown port order '{o}' ({})", PORT_ORDERS.join(", "))),
                };
                let z0 = self.z0.iter().sum::<f64>() / 4.0;
                if self.z0.iter().any(|z| (z - z0).abs() > 1e-9) {
                    notes.push(format!("{}: different reference impedances per port; their mean {z0:.1} Ω used", self.name));
                }
                let ports = [(pn, nn), (pf, nf)];
                // Sdd_ij = (S_PiPj − S_PiNj − S_NiPj + S_NiNj) / 2, reference 2·z0
                let sdd: Vec<Vec<Cx>> = (0..4)
                    .map(|k| {
                        let ((pi, ni), (pj, nj)) = (ports[k / 2], ports[k % 2]);
                        let (a, b, c, d) = (el(pi, pj), el(pi, nj), el(ni, pj), el(ni, nj));
                        (0..self.freq.len()).map(|q| (a[q] - b[q] - c[q] + d[q]).scale(0.5)).collect()
                    })
                    .collect();
                for &f in freq {
                    let s = |k: usize| interp(&self.freq, &sdd[k], f);
                    let dm = abcd_from_s(s(0), s(1), s(2), s(3), 2.0 * z0);
                    // differential (V = Vp − Vn, I) → per line odd mode (V/2, I)
                    out.push(Abcd { a: dm.a, b: dm.b.scale(0.5), c: dm.c.scale(2.0), d: dm.d });
                }
                notes.push(format!("{}: 4-port reduced to the differential mode (ports {order})", self.name));
            }
            n => return Err(format!("{}: {n}-port – give the port of every connector pin (touchstone \"ports\": {{\"<pin>\": [port at the connector, port at the far end]}})", self.name)),
        }
        if flip {
            out = out.iter().map(reversed).collect();
        }
        if out.iter().any(|m| !(m.a.re.is_finite() && m.b.re.is_finite() && m.c.re.is_finite())) {
            return Err(format!("{}: transmission too small to convert (S21 ≈ 0)", self.name));
        }
        Ok((out, notes))
    }

    /// The whole S-matrix (row-major) interpolated at `f` (as `per_line_abcd`: magnitude held
    /// outside the file's range, the phase to 0 at DC).
    pub fn s_at(&self, f: f64) -> Vec<Cx> {
        let n = self.nports;
        (0..n * n).map(|i| interp(&self.freq, &self.s.iter().map(|m| m[i]).collect::<Vec<_>>(), f)).collect()
    }

    /// Notes when `freq` reaches beyond the file's frequency range.
    pub fn extrapolation_notes(&self, freq: &[f64]) -> Vec<String> {
        let mut notes = vec![];
        let (fmin, fmax) = (self.freq[0], *self.freq.last().unwrap());
        if freq.iter().any(|&f| f > fmax * 1.001) {
            notes.push(format!("{} ends at {:.2} GHz: above it extrapolated (magnitude held)", self.name, fmax / 1e9));
        }
        if freq.iter().any(|&f| f < fmin * 0.999) && fmin > 0.0 {
            notes.push(format!("{} starts at {:.3} GHz: towards DC extrapolated", self.name, fmin / 1e9));
        }
        notes
    }

    /// Passivity of the file's data: None when every S-matrix has singular values ≤ 1 (+ 0.1 %
    /// for measurement noise), else a note with the worst value and its frequency.
    pub fn passivity_note(&self) -> Option<String> {
        let (worst, k) = super::passivity(&self.s, self.nports);
        (worst > 1.001).then(|| format!("{}: not passive – largest singular value of S {:.3} at {:.2} GHz (measurement noise or a bad de-embedding); results with it are doubtful", self.name, worst, self.freq[k] / 1e9))
    }

    /// Short description for the viewer and the CLI.
    pub fn summary(&self) -> serde_json::Value {
        serde_json::json!({"name": self.name, "nports": self.nports, "points": self.freq.len(),
            "f_min_ghz": self.freq[0] / 1e9, "f_max_ghz": self.freq.last().unwrap() / 1e9, "z0": self.z0, "warnings": self.warnings})
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line_s(len_ps: f64, loss_db: f64, f: f64) -> Cx {
        let mag = 10f64.powf(-loss_db / 20.0);
        let ph = -2.0 * PI * f * len_ps * 1e-12;
        Cx::new(mag * ph.cos(), mag * ph.sin())
    }

    #[test]
    fn two_port_v1_ma_and_ri() {
        // a matched line: 100 ps, 1 dB at 1 GHz, 2 dB at 2 GHz
        let mut t = String::from("! test\n# GHz S MA R 50\n");
        for (f, l) in [(1.0, 1.0), (2.0, 2.0), (3.0, 3.0)] {
            let s = line_s(100.0, l, f * 1e9);
            t += &format!("{f} 0 0 {} {} {} {} 0 0\n", s.abs(), s.arg() * 180.0 / PI, s.abs(), s.arg() * 180.0 / PI);
        }
        let ts = parse_touchstone(&t, "cable.s2p").unwrap();
        assert_eq!((ts.nports, ts.freq.len(), ts.z0[0]), (2, 3, 50.0));
        let (m, notes) = ts.per_line_abcd(&[1.5e9, 4e9, 0.5e9], false, None, false).unwrap();
        assert!(notes.iter().any(|n| n.contains("extrapolated")));
        // S21 back from the ABCD: magnitude interpolated linearly between 1 and 2 dB (−1.486 dB)
        let (_, s21, _) = m[0].s_params(50.0);
        assert!((20.0 * s21.abs().log10() + 1.486).abs() < 0.005, "{}", 20.0 * s21.abs().log10());
        // phase: 100 ps → −54° at 1.5 GHz
        assert!((s21.arg() * 180.0 / PI + 54.0).abs() < 0.5);
        // RI format, flip of a symmetric line changes nothing
        let ri = t.replace("# GHz S MA R 50", "# GHz S RI R 50");
        assert!(parse_touchstone(&ri, "x.s2p").is_ok());
        let (mf, _) = ts.per_line_abcd(&[1.5e9], false, None, true).unwrap();
        assert!((mf[0].s_params(50.0).1.abs() - s21.abs()).abs() < 1e-9);
    }

    #[test]
    fn four_port_differential_mode() {
        // two uncoupled identical lines in port order 1-3,2-4: Sdd21 = S31
        let mut t = String::from("# GHz S RI R 50\n");
        for f in [1.0, 2.0] {
            let s = line_s(50.0, f, f * 1e9);
            // row-major 4×4: S13 = S31 = S24 = S42 = s, rest 0
            let mut m = [[Cx::new(0.0, 0.0); 4]; 4];
            m[0][2] = s; m[2][0] = s; m[1][3] = s; m[3][1] = s;
            t += &format!("{f}");
            for r in 0..4 {
                for c in 0..4 {
                    t += &format!(" {} {}", m[r][c].re, m[r][c].im);
                }
                t += "\n";
            }
        }
        let ts = parse_touchstone(&t, "pair.s4p").unwrap();
        let (m, _) = ts.per_line_abcd(&[1e9], true, Some("1-3,2-4"), false).unwrap();
        // per line (odd mode) at 50 Ω: the line itself
        let (s11, s21, _) = m[0].s_params(50.0);
        assert!(s11.abs() < 1e-9 && (20.0 * s21.abs().log10() + 1.0).abs() < 1e-9);
        // the wrong port order sees no transmission
        assert!(ts.per_line_abcd(&[1e9], true, Some("1-2,3-4"), false).is_err());
        assert!(ts.per_line_abcd(&[1e9], false, None, false).is_err());
    }

    #[test]
    fn refused_files() {
        assert!(parse_touchstone("# GHz Z MA R 50\n1 1 0 0 0 0 0 1 0\n2 1 0 0 0 0 0 1 0\n", "z.s2p").unwrap_err().contains("only S-parameters"));
        // a 3-port is read (as an N-port it needs a port map), 65 ports are not
        let three = "# GHz S MA R 50\n1 1 0 0 0 0 0 0 0 0 0 1 0 0 0 0 0 0 0\n2 1 0 0 0 0 0 0 0 0 0 1 0 0 0 0 0 0 0\n";
        let t3 = parse_touchstone(three, "x.s3p").unwrap();
        assert_eq!(t3.nports, 3);
        assert!(t3.per_line_abcd(&[1e9], false, None, false).unwrap_err().contains("port of every connector pin"));
        assert!(parse_touchstone("# GHz S MA R 50\n1 1 0\n", "x.s65p").unwrap_err().contains("65-port"));
        assert!(parse_touchstone("# GHz S MA R 50\n1 1 0 0 0 0 0 1 0\n", "x.s2p").unwrap_err().contains("fewer than two"));
        // version 2 with keywords and 12_21 order
        let v2 = "[Version] 2.0\n# GHz S RI R 50\n[Number of Ports] 2\n[Two-Port Data Order] 12_21\n[Number of Frequencies] 2\n[Network Data]\n1 0 0 0.1 0 0.9 0 0 0\n2 0 0 0.1 0 0.8 0 0 0\n[End]\n";
        let ts = parse_touchstone(v2, "v2.ts").unwrap();
        // 12_21: the second pair is S12
        assert!((ts.s[0][1].re - 0.1).abs() < 1e-12 && (ts.s[0][2].re - 0.9).abs() < 1e-12);
        assert!(ts.passivity_note().is_none());
        // gain: not passive, named in the warnings
        let gain = parse_touchstone("# GHz S MA R 50\n1 0 0 1.1 0 1.1 0 0 0\n2 0 0 0.9 0 0.9 0 0 0\n", "amp.s2p").unwrap();
        assert!(gain.warnings.iter().any(|w| w.contains("not passive") && w.contains("1.100 at 1.00 GHz")), "{:?}", gain.warnings);
    }
}

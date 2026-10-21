//! Rectangular plane-pair cavity, modal (double cosine series) model (Okoshi), ported from
//! validation/fdtd/cavity_model.py and checked against the kisi-fdtd references of group G
//! (resonances ±0.8 %, transfer impedance ±0.7 %, see validation/fdtd/TESTS.md).
//!
//! Plane pair a × b, height d, magnetic walls at the open board edges, square ports of side w:
//!
//! Z_ij = jωμd/(ab) · Σ_m Σ_n χm² χn² cos(kxm xi) cos(kyn yi) cos(kxm xj) cos(kyn yj)
//!                      · sinc²(kxm w/2) sinc²(kyn w/2) / (kxm² + kyn² − k²)
//!
//! kxm = mπ/a, kyn = nπ/b, χ0 = 1, χ = √2 otherwise, k² = ω²με(1 − j tanδ). A complement to
//! the grid model in `pdn` for rectangular planes: no meshing, all ports in one pass.
//! Lengths in m.

use kisi_board::num::{Cx, EPS0, MU0};
use std::f64::consts::PI;

#[derive(Debug, Clone)]
pub struct Cavity {
    pub a: f64,
    pub b: f64,
    pub d: f64,
    pub er: f64,
    pub tand: f64,
    /// loss frequency: Some(f_l) scales tanδ by f_l/f (fixed σ, as the FDTD references)
    pub f_loss: Option<f64>,
    /// fringing extension of each edge (m), e.g. d·ln2/π; 0 for none
    pub fringe: f64,
    pub modes: (usize, usize),
}

/// Precomputed modal weights for a set of ports of side `w` (one entry per port pair).
pub struct CavityPorts {
    kx2: Vec<f64>,
    ky2: Vec<f64>,
    /// weight[(i,j)][m*N+n] = χ²·sinc²·cos·cos (real)
    w: Vec<Vec<f64>>,
    pub n: usize,
    pre: f64,
}

fn sinc(x: f64) -> f64 {
    if x.abs() < 1e-12 {
        1.0
    } else {
        x.sin() / x
    }
}

impl Cavity {
    pub fn ports(&self, ports: &[(f64, f64)], w: f64) -> CavityPorts {
        let (mm, nn) = self.modes;
        let (a2, b2) = (self.a + 2.0 * self.fringe, self.b + 2.0 * self.fringe);
        let kx: Vec<f64> = (0..mm).map(|m| m as f64 * PI / a2).collect();
        let ky: Vec<f64> = (0..nn).map(|n| n as f64 * PI / b2).collect();
        let np = ports.len();
        let mut base = vec![0.0; mm * nn];
        for m in 0..mm {
            for n in 0..nn {
                let chi2 = if m == 0 { 1.0 } else { 2.0 } * if n == 0 { 1.0 } else { 2.0 };
                base[m * nn + n] = chi2 * sinc(kx[m] * w / 2.0).powi(2) * sinc(ky[n] * w / 2.0).powi(2);
            }
        }
        let cx: Vec<Vec<f64>> = ports.iter().map(|&(x, _)| kx.iter().map(|k| (k * (x + self.fringe)).cos()).collect()).collect();
        let cy: Vec<Vec<f64>> = ports.iter().map(|&(_, y)| ky.iter().map(|k| (k * (y + self.fringe)).cos()).collect()).collect();
        let mut wts = vec![];
        for i in 0..np {
            for j in 0..np {
                let mut v = vec![0.0; mm * nn];
                for m in 0..mm {
                    for n in 0..nn {
                        v[m * nn + n] = base[m * nn + n] * cx[i][m] * cy[i][n] * cx[j][m] * cy[j][n];
                    }
                }
                wts.push(v);
            }
        }
        CavityPorts { kx2: kx.iter().map(|k| k * k).collect(), ky2: ky.iter().map(|k| k * k).collect(), w: wts, n: np, pre: MU0 * self.d / (a2 * b2) }
    }

    /// Z matrix (row-major n×n, Ω) at frequency f.
    pub fn z(&self, p: &CavityPorts, f: f64) -> Vec<Cx> {
        let w0 = 2.0 * PI * f;
        let td = match self.f_loss {
            Some(fl) => self.tand * fl / f,
            None => self.tand,
        };
        let k2 = Cx::new(w0 * w0 * MU0 * EPS0 * self.er, -w0 * w0 * MU0 * EPS0 * self.er * td);
        let nn = p.ky2.len();
        // 1/(kx²+ky²−k²) once per mode
        let inv: Vec<Cx> = (0..p.kx2.len() * nn).map(|q| (Cx::new(p.kx2[q / nn] + p.ky2[q % nn], 0.0) - k2).inv()).collect();
        let jw = Cx::new(0.0, w0 * p.pre);
        p.w
            .iter()
            .map(|wt| {
                let (mut re, mut im) = (0.0, 0.0);
                for (x, c) in wt.iter().zip(&inv) {
                    re += x * c.re;
                    im += x * c.im;
                }
                jw * Cx::new(re, im)
            })
            .collect()
    }
}

/// Loop inductance (H) at `f` of a square port of side `w_port` at `port` with a second
/// port (side `w_short`) at `short` shorted (e.g. a capacitor's vias):
/// L = Im(Z11 − Z12 Z21 / Z22)/ω, ports given by their own sizes.
pub fn loop_inductance(c: &Cavity, port: (f64, f64), w_port: f64, short: (f64, f64), w_short: f64, f: f64) -> f64 {
    let pa = c.ports(&[port, short], w_port);
    let pb = c.ports(&[port, short], w_short);
    let za = c.z(&pa, f);
    let zb = c.z(&pb, f);
    let zin = za[0] - za[1] * za[2] / zb[3];
    zin.im / (2.0 * PI * f)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_resonance() {
        // 50 × 40 mm, εr 4.3: TM10 at c/(2a√εr) ≈ 1.446 GHz
        let c = Cavity { a: 0.05, b: 0.04, d: 0.2e-3, er: 4.3, tand: 0.0, f_loss: None, fringe: 0.0, modes: (60, 60) };
        let p = c.ports(&[(0.001, 0.02)], 0.3e-3);
        let f10 = 299_792_458.0 / (2.0 * 0.05 * 4.3f64.sqrt());
        let below = c.z(&p, f10 * 0.995)[0].im;
        let above = c.z(&p, f10 * 1.005)[0].im;
        assert!(below > 0.0 && above < 0.0, "{below} {above}");
        // low frequency: plate capacitance C = εA/d
        let cpl = EPS0 * 4.3 * 0.05 * 0.04 / 0.2e-3;
        let z = c.z(&p, 10e6)[0];
        let cz = -1.0 / (2.0 * PI * 10e6 * z.im);
        assert!((cz / cpl - 1.0).abs() < 0.02, "{cz} {cpl}");
    }

    #[test]
    fn loop_inductance_grows_with_distance() {
        let c = Cavity { a: 0.03, b: 0.03, d: 0.233e-3, er: 4.0, tand: 0.0, f_loss: None, fringe: 0.0, modes: (200, 200) };
        let l1 = loop_inductance(&c, (0.01, 0.015), 2.4e-3, (0.015, 0.015), 0.2e-3, 50e6);
        let l2 = loop_inductance(&c, (0.01, 0.015), 2.4e-3, (0.02, 0.015), 0.2e-3, 50e6);
        assert!(l1 > 0.0 && l2 > l1, "{l1} {l2}");
    }
}

//! Lanes as networks for the channel check and the eye (milestone M2).
//!
//! A lane's channel is a row of blocks between boundaries – the board's own copper (the cascade
//! of one line, or the 4-port of a pair from `rf::mixed`), the channel behind a connector (a
//! lossy line or a Touchstone file) – with the transmitter and the receiver models at the ends.
//! `rf::network` solves it at every frequency and at 0 Hz: the transfer from the transmitter's
//! emf to the receiver's die, for a pair the differential voltage with the transmitter driving
//! P and N in opposite phase, mode conversion included. Single lines give the cascade's
//! `eye::transfer` to numerical precision (test).
//!
//! The ends are per line: transmitter = emf behind its resistance, its capacitance at the die,
//! its inductance towards the pin; receiver = inductance, then capacitance and termination to AC
//! ground. A pair's ends are two such lines, so the common mode is terminated by the same
//! per-line resistance (centre-tapped, as CML and PCIe/USB receivers); a termination across the
//! pair (LVDS) would leave the common mode open – that only matters for signals converted to
//! common mode and back.

use crate::eye::{ConnEnd, EndModel};
use crate::rf::{mixed_of, reversed, s_of, sep, tdr_map, Abcd, Channel, Cx, Network, Source, Touchstone, GND};
use std::f64::consts::PI;

pub type M4 = [[Cx; 4]; 4];

fn cz() -> Cx {
    Cx::new(0.0, 0.0)
}

/// One block of a lane's channel, between two boundaries.
#[derive(Debug, Clone)]
pub enum Block {
    /// one line: cascade per frequency and at 0 Hz
    Two { m: Vec<Abcd>, dc: Abcd },
    /// a pair: 4×4 cascade matrices (state V_P V_N I_P I_N) per frequency and at 0 Hz
    Four { t: Vec<M4>, dc: M4 },
    /// S-parameters on the channel's grid (a Touchstone file): port indices `near` (one per line,
    /// P first) at the boundary before the block, `far` after it; other ports are terminated in
    /// their reference impedance. `s[k]` is N×N row-major, `dc` the same at 0 Hz.
    S { s: Vec<Vec<Cx>>, dc: Vec<Cx>, z0: Vec<f64>, near: Vec<usize>, far: Vec<usize> },
}

impl Block {
    fn lines(&self) -> usize {
        match self {
            Block::Two { .. } => 1,
            Block::Four { .. } => 2,
            Block::S { near, .. } => near.len(),
        }
    }

    /// The block seen from its far end.
    pub fn flipped(&self) -> Block {
        match self {
            Block::Two { m, dc } => Block::Two { m: m.iter().map(reversed).collect(), dc: reversed(dc) },
            Block::Four { t, dc } => Block::Four { t: t.iter().map(flip4).collect(), dc: flip4(dc) },
            Block::S { s, dc, z0, near, far } => Block::S { s: s.clone(), dc: dc.clone(), z0: z0.clone(), near: far.clone(), far: near.clone() },
        }
    }
}

/// A 4-port cascade matrix seen from the other end: [V; I]_far = T⁻¹·[V; I]_near with the
/// currents reversed.
fn flip4(t: &M4) -> M4 {
    let mut a = [[cz(); 8]; 4];
    for i in 0..4 {
        for j in 0..4 {
            a[i][j] = t[i][j];
        }
        a[i][4 + i] = Cx::new(1.0, 0.0);
    }
    // Gauss-Jordan with partial pivoting
    for c in 0..4 {
        let p = (c..4).max_by(|&x, &y| a[x][c].abs().total_cmp(&a[y][c].abs())).unwrap();
        a.swap(c, p);
        let inv = a[c][c].inv();
        for j in 0..8 {
            a[c][j] = a[c][j] * inv;
        }
        for r in 0..4 {
            if r != c {
                let f = a[r][c];
                for j in 0..8 {
                    a[r][j] = a[r][j] - f * a[c][j];
                }
            }
        }
    }
    // T⁻¹ with the current rows and columns negated: S = diag(1, 1, −1, −1)
    let mut out = [[cz(); 4]; 4];
    for i in 0..4 {
        for j in 0..4 {
            let sg = if (i < 2) == (j < 2) { 1.0 } else { -1.0 };
            out[i][j] = a[i][4 + j].scale(sg);
        }
    }
    out
}

/// Transfer from the transmitter's emf to the receiver's die through `blocks` (in order from
/// the transmitter), normalised as `eye::transfer`: the swing is defined into the receiver's
/// termination (a lossless link without parasitics gives 1 at DC) or, for a high-impedance
/// receiver, as the open-circuit swing. `lines` = 2: a pair driven differentially, the result
/// is the differential voltage. Returns H on `freq` and H at 0 Hz.
pub fn ends_transfer(freq: &[f64], blocks: &[Block], lines: usize, tx: &EndModel, rx: &EndModel) -> Result<(Vec<Cx>, f64), String> {
    if blocks.iter().any(|b| b.lines() != lines) {
        return Err("blocks with different numbers of lines".into());
    }
    let npts = freq.len() + 1;
    let ws: Vec<f64> = std::iter::once(0.0).chain(freq.iter().map(|&f| 2.0 * PI * f)).collect();
    let mut net = Network::new(npts);
    let bnd: Vec<Vec<usize>> = (0..=blocks.len()).map(|i| (0..lines).map(|l| net.node(format!("boundary {i}, line {l}"))).collect()).collect();
    for (i, b) in blocks.iter().enumerate() {
        match b {
            Block::Two { m, dc } => {
                let all: Vec<Abcd> = std::iter::once(*dc).chain(m.iter().copied()).collect();
                net.two_port(bnd[i][0], bnd[i + 1][0], &all);
            }
            Block::Four { t, dc } => {
                let all: Vec<M4> = std::iter::once(*dc).chain(t.iter().copied()).collect();
                net.four_port([bnd[i][0], bnd[i][1]], [bnd[i + 1][0], bnd[i + 1][1]], &all);
            }
            Block::S { s, dc, z0, near, far } => {
                let np = z0.len();
                let mut nodes = Vec::with_capacity(np);
                for p in 0..np {
                    let node = if let Some(l) = near.iter().position(|&q| q == p) {
                        bnd[i][l]
                    } else if let Some(l) = far.iter().position(|&q| q == p) {
                        bnd[i + 1][l]
                    } else {
                        // a port of the file this lane does not use: terminated
                        let n = net.node(format!("block {i}, port {}", p + 1));
                        net.adm(n, GND, vec![Cx::new(1.0 / z0[p], 0.0); npts]);
                        n
                    };
                    nodes.push(node);
                }
                let all: Vec<Vec<Cx>> = std::iter::once(dc.clone()).chain(s.iter().cloned()).collect();
                net.s_block(nodes, z0.clone(), all);
            }
        }
    }
    // ends, per line
    let mut src = Vec::new();
    let mut probe = Vec::new();
    for l in 0..lines {
        let pin_tx = bnd[0][l];
        let die_tx = if tx.l_nh > 0.0 {
            let d = net.node(format!("transmitter die, line {l}"));
            let z: Vec<Cx> = ws.iter().map(|w| Cx::new(0.0, w * tx.l_nh * 1e-9)).collect();
            net.series(d, pin_tx, &z);
            d
        } else {
            pin_tx
        };
        if tx.c_pf > 0.0 {
            net.adm(die_tx, GND, ws.iter().map(|w| Cx::new(0.0, w * tx.c_pf * 1e-12)).collect());
        }
        let e = if lines == 2 { if l == 0 { 0.5 } else { -0.5 } } else { 1.0 };
        src.push(Source { node: die_tx, z: Cx::new(tx.r.max(0.0), 0.0), e: vec![Cx::new(e, 0.0)] });
        let pin_rx = bnd[blocks.len()][l];
        let die_rx = if rx.l_nh > 0.0 {
            let d = net.node(format!("receiver die, line {l}"));
            let z: Vec<Cx> = ws.iter().map(|w| Cx::new(0.0, w * rx.l_nh * 1e-9)).collect();
            net.series(pin_rx, d, &z);
            d
        } else {
            pin_rx
        };
        let g = if rx.r > 0.0 { 1.0 / rx.r } else { 0.0 };
        net.adm(die_rx, GND, ws.iter().map(|w| Cx::new(g, w * rx.c_pf * 1e-12)).collect());
        probe.push(die_rx);
    }
    let scale = if rx.r > 0.0 { (tx.r.max(0.0) + rx.r) / rx.r } else { 1.0 };
    let mut h = Vec::with_capacity(npts);
    for k in 0..npts {
        let v = net.solve(k, &src)?;
        let out = if lines == 2 { v[0][probe[0]] - v[0][probe[1]] } else { v[0][probe[0]] };
        h.push(out.scale(scale));
    }
    let dc = h[0].re;
    Ok((h[1..].to_vec(), dc))
}

/// The channel behind a connector as a block (`conn_end`): the Touchstone file (2-port on each
/// line, or the 4-port of a pair with its port order, mode conversion included) or the lossy
/// line, on the grid `freq`, with port 1 (the near end) at the connector unless `flip`.
/// `lines`: 2 for a pair's 4-port; 1 with `diff` for a pair in the odd mode (a 4-port file is
/// then reduced to the differential mode). `nyq`: the profile's Nyquist frequency (the lossy
/// line's loss is given there), `z0` per line. Returns the block (near end at the connector)
/// and a note.
pub fn rest_block(ce: &ConnEnd, ts: Option<&Touchstone>, freq: &[f64], lines: usize, diff: bool, nyq: f64, z0: f64) -> Result<(Block, String), String> {
    let line_block = |m: Vec<Abcd>, dc: Abcd| -> Block {
        if lines == 2 {
            Block::Four { t: m.iter().map(|x| sep(x, x)).collect(), dc: sep(&dc, &dc) }
        } else {
            Block::Two { m, dc }
        }
    };
    match (&ce.touchstone, ts) {
        (Some(tr), Some(ts)) => {
            if ts.nports == 4 && lines == 2 {
                let order = tr.port_order.as_deref().unwrap_or("1-3,2-4");
                let [pn, nn, pf, nf] = match order {
                    "1-3,2-4" => [0, 1, 2, 3],
                    "1-2,3-4" => [0, 2, 1, 3],
                    o => return Err(format!("unknown port order '{o}' ({})", crate::rf::PORT_ORDERS.join(", "))),
                };
                let (mut near, mut far) = (vec![pn, nn], vec![pf, nf]);
                if tr.flip {
                    std::mem::swap(&mut near, &mut far);
                }
                let mut notes = ts.extrapolation_notes(freq);
                notes.push(format!("{}: 4-port with mode conversion (ports {order})", ts.name));
                let dc = ts.s_at(0.0);
                let s: Vec<Vec<Cx>> = freq.iter().map(|&f| ts.s_at(f)).collect();
                return Ok((Block::S { s, dc, z0: ts.z0.clone(), near, far }, notes.join("; ")));
            }
            let grid: Vec<f64> = std::iter::once(0.0).chain(freq.iter().copied()).collect();
            let (m, notes) = ts.per_line_abcd(&grid, diff || lines == 2, tr.port_order.as_deref(), tr.flip)?;
            Ok((line_block(m[1..].to_vec(), m[0]), notes.join("; ")))
        }
        (Some(tr), None) => Err(format!("Touchstone file {} not loaded", tr.file)),
        (None, _) => {
            let m: Vec<Abcd> = freq.iter().map(|&f| ce.rest_abcd(f, nyq, z0).unwrap_or_else(Abcd::identity)).collect();
            Ok((line_block(m, Abcd::identity()), format!("a lossy line of {:.1} dB at {:.2} GHz, {:.0} ps", ce.rest_loss_db.unwrap_or(0.0).abs(), nyq / 1e9, ce.rest_delay_ps.unwrap_or(500.0))))
        }
    }
}

/// The 4-port of a pair's lane or net: cascade matrices on the channel grid and at 0 Hz.
#[derive(Debug, Clone)]
pub struct PairNet {
    pub freq: Vec<f64>,
    pub t: Vec<M4>,
    pub t_dc: M4,
    /// single-ended reference per port (Zdiff,ref = 2·z_ref)
    pub z_ref: f64,
    pub coupled_mm: f64,
    /// track length of P and N (mm)
    pub length_mm: (f64, f64),
    pub warnings: Vec<String>,
}

impl PairNet {
    /// Mixed-mode matrices [[Sdd, Sdc], [Scd, Scc]] per frequency.
    pub fn mixed(&self) -> Vec<M4> {
        self.t.iter().map(|t| mixed_of(&s_of(t, self.z_ref))).collect()
    }

    pub fn block(&self) -> Block {
        Block::Four { t: self.t.clone(), dc: self.t_dc }
    }
}

/// Worst mode conversion Scd21 (dB) up to `f_hi` and its frequency (Hz).
pub fn worst_scd21(freq: &[f64], mm: &[M4], f_hi: f64) -> (f64, f64) {
    freq.iter().zip(mm).filter(|(&f, _)| f <= f_hi * 1.0001).map(|(&f, m)| (20.0 * m[3][0].abs().max(1e-12).log10(), f)).fold((f64::NEG_INFINITY, 0.0), |a, b| if b.0 > a.0 { b } else { a })
}

/// Put the differential mode of the pair's 4-port into the odd-mode channel `ch` (same grid):
/// S11/S21/S22 become Sdd11/Sdd21/Sdd22 (reference 2·z_ref differential, i.e. z_ref per line as
/// before), with the insertion-loss marks and the TDR from Sdd11 (per line, as the channel's).
/// `rise`: the TDR edge.
pub fn apply_pair(ch: &mut Channel, mm: &[M4], rise: f64) {
    let db = |x: Cx| 20.0 * x.abs().max(1e-12).log10();
    ch.s11 = mm.iter().map(|m| m[0][0]).collect();
    ch.s21 = mm.iter().map(|m| m[1][0]).collect();
    ch.s22 = mm.iter().map(|m| m[1][1]).collect();
    ch.s11_db = ch.s11.iter().map(|&x| db(x)).collect();
    ch.s21_db = ch.s21.iter().map(|&x| db(x)).collect();
    ch.s22_db = ch.s22.iter().map(|&x| db(x)).collect();
    let df = ch.freq[0];
    let n = ch.freq.len();
    for m in ch.il_marks.iter_mut() {
        let k = ((m.0 * 1e9 / df).round() as usize).clamp(1, n) - 1;
        m.1 = ch.s21_db[k];
    }
    // TDR on the same time axis, positions from the odd-mode channel
    if let (Some(&t_last), true) = (ch.tdr_t_ps.last(), ch.tdr_t_ps.len() > 1) {
        let (ts, xs) = (ch.tdr_t_ps.clone(), ch.tdr_x_mm.clone());
        let t_max = t_last * 1e-12 * ts.len() as f64 / (ts.len() - 1) as f64;
        let xmap = move |t: f64| -> f64 {
            let tp = t * 1e12;
            match ts.iter().position(|&x| x >= tp) {
                Some(0) => xs[0],
                Some(k) => xs[k - 1] + (xs[k] - xs[k - 1]) * (tp - ts[k - 1]) / (ts[k] - ts[k - 1]).max(1e-12),
                None => *xs.last().unwrap(),
            }
        };
        let (t, z, x) = tdr_map(&ch.freq, &ch.s11, ch.z_ref, rise, t_max, &xmap);
        ch.tdr_t_ps = t;
        ch.tdr_z = z;
        ch.tdr_x_mm = x;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rf::{coupled, C0};

    fn line(zc: f64, eeff: f64, r: f64, f: f64, len: f64) -> Abcd {
        Abcd::line(r, zc * eeff.sqrt() / C0, 1e-4, eeff.sqrt() / (zc * C0), 2.0 * PI * f, len)
    }

    fn channel(freq: &[f64], m: &[Abcd]) -> Channel {
        let mut ch = crate::eye::tests_support::channel_from(freq.to_vec(), m.iter().map(|x| x.s_params(50.0).1).collect(), 100.0);
        ch.abcd = m.to_vec();
        ch.abcd_dc = Abcd::series(Cx::new(3.0, 0.0));
        ch
    }

    /// One line through the network equals the cascade's `eye::transfer` (also at 0 Hz), with
    /// every kind of end; a symmetric pair driven differentially equals its odd mode.
    #[test]
    fn network_transfer_equals_cascade() {
        let freq: Vec<f64> = (1..=40).map(|k| k as f64 * 0.5e9).collect();
        let m: Vec<Abcd> = freq.iter().map(|&f| line(47.0, 3.3, 9.0, f, 0.04).mul(&Abcd::series(Cx::new(10.0, 2.0 * PI * f * 0.5e-9))).mul(&line(55.0, 3.0, 9.0, f, 0.01))).collect();
        let ch = channel(&freq, &m);
        let ends = [
            (EndModel { r: 50.0, c_pf: 1.0, l_nh: 0.5 }, EndModel { r: 50.0, c_pf: 0.8, l_nh: 0.3 }),
            (EndModel { r: 0.0, c_pf: 2.0, l_nh: 0.0 }, EndModel { r: 0.0, c_pf: 1.5, l_nh: 0.0 }),
            (EndModel { r: 34.0, c_pf: 0.0, l_nh: 0.0 }, EndModel { r: 48.0, c_pf: 1.2, l_nh: 0.0 }),
        ];
        for (tx, rx) in ends {
            let h0 = crate::eye::transfer(&ch, &tx, &rx);
            let d0 = crate::eye::transfer_dc(&ch, &tx, &rx);
            let (h, d) = ends_transfer(&freq, &[Block::Two { m: m.clone(), dc: ch.abcd_dc }], 1, &tx, &rx).unwrap();
            for k in 0..freq.len() {
                assert!((h[k] - h0[k]).abs() < 1e-10 * (1.0 + h0[k].abs()), "{tx:?} {rx:?} {k}: {:?} vs {:?}", h[k], h0[k]);
            }
            assert!((d - d0).abs() < 1e-12, "{d} {d0}");
            // symmetric pair, no coupling difference: differential = odd mode per line
            let t: Vec<M4> = m.iter().map(|x| coupled(x, x)).collect();
            let (hp, dp) = ends_transfer(&freq, &[Block::Four { t, dc: sep(&ch.abcd_dc, &ch.abcd_dc) }], 2, &tx, &rx).unwrap();
            for k in 0..freq.len() {
                assert!((hp[k] - h0[k]).abs() < 1e-10 * (1.0 + h0[k].abs()));
            }
            assert!((dp - d0).abs() < 1e-12);
        }
        // a block flipped twice is the block; flipping a 2-port twice as well
        let t4 = coupled(&line(60.0, 3.4, 5.0, 3e9, 0.02), &line(45.0, 3.0, 6.0, 3e9, 0.02));
        let back = flip4(&flip4(&t4));
        for i in 0..4 {
            for j in 0..4 {
                assert!((back[i][j] - t4[i][j]).abs() < 1e-9 * (1.0 + t4[i][j].abs()));
            }
        }
    }

    /// A Touchstone 4-port behind the board: the S block equals the cascade of its 4-port.
    #[test]
    fn s_block_equals_its_chain() {
        let freq: Vec<f64> = (1..=10).map(|k| k as f64 * 1e9).collect();
        let t: Vec<M4> = freq.iter().map(|&f| coupled(&line(62.0, 3.4, 4.0, f, 0.03), &line(44.0, 3.0, 5.0, f, 0.03))).collect();
        let board: Vec<M4> = freq.iter().map(|&f| sep(&line(50.0, 3.2, 4.0, f, 0.02), &line(50.0, 3.2, 4.0, f, 0.021))).collect();
        // S of the rest in the port order 1-2,3-4 (P 1→2, N 3→4)
        let s: Vec<Vec<Cx>> = t
            .iter()
            .map(|x| {
                let s = s_of(x, 50.0); // order P1 N1 P2 N2
                let map = [0, 2, 1, 3]; // file port → s_of index
                let mut out = vec![cz(); 16];
                for i in 0..4 {
                    for j in 0..4 {
                        out[i * 4 + j] = s[map[i]][map[j]];
                    }
                }
                out
            })
            .collect();
        let id = sep(&Abcd::identity(), &Abcd::identity());
        let tx = EndModel { r: 50.0, c_pf: 0.5, l_nh: 0.0 };
        let rx = EndModel { r: 50.0, c_pf: 0.5, l_nh: 0.2 };
        let (h1, _) = ends_transfer(&freq, &[Block::Four { t: board.clone(), dc: id }, Block::Four { t: t.clone(), dc: id }], 2, &tx, &rx).unwrap();
        let blk = Block::S { s: s.clone(), dc: s[0].clone(), z0: vec![50.0; 4], near: vec![0, 2], far: vec![1, 3] };
        let (h2, _) = ends_transfer(&freq, &[Block::Four { t: board, dc: id }, blk], 2, &tx, &rx).unwrap();
        for k in 0..freq.len() {
            assert!((h1[k] - h2[k]).abs() < 1e-10, "{k}");
        }
    }
}

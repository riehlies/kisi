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
use crate::rf::{mixed_of, reversed, s_of, sep, tdr_map, Abcd, Channel, Cx, Network, Source, Tap, Touchstone, GND};
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
    /// P first) at the boundary before the block, `far` after it; the ports of aggressors `xt`
    /// carry their transmitter and receiver, other ports are terminated in their reference
    /// impedance. `s[k]` is N×N row-major, `dc` the same at 0 Hz.
    S { s: Vec<Vec<Cx>>, dc: Vec<Cx>, z0: Vec<f64>, near: Vec<usize>, far: Vec<usize>, xt: Vec<XtSrc> },
    /// one line with receivers on side stubs (multi-drop): the main path in pieces and the
    /// stubs to the receivers (`rf::Tap`, hanging behind piece `after`)
    Line { pieces: Vec<(Vec<Abcd>, Abcd)>, taps: Vec<Tap> },
}

/// An aggressor in an S block (another lane of the connector in the same file): its
/// transmitter at the file's ports `drive` (one per line, P first), its other end at `load`
/// (terminated with the receiver model).
#[derive(Debug, Clone)]
pub struct XtSrc {
    pub label: String,
    pub drive: Vec<usize>,
    pub load: Vec<usize>,
}

/// The transfer to one receiver (`label`: its pin; empty for the lane's own receiver), and the
/// crosstalk into it from every aggressor (label, transfer, value at 0 Hz), in the same units.
#[derive(Debug, Clone)]
pub struct RxTransfer {
    pub label: String,
    pub h: Vec<Cx>,
    pub dc: f64,
    pub xt: Vec<(String, Vec<Cx>, f64)>,
}

impl Block {
    fn lines(&self) -> usize {
        match self {
            Block::Two { .. } | Block::Line { .. } => 1,
            Block::Four { .. } => 2,
            Block::S { near, .. } => near.len(),
        }
    }

    /// The block seen from its far end.
    pub fn flipped(&self) -> Block {
        match self {
            Block::Two { m, dc } => Block::Two { m: m.iter().map(reversed).collect(), dc: reversed(dc) },
            Block::Four { t, dc } => Block::Four { t: t.iter().map(flip4).collect(), dc: flip4(dc) },
            Block::S { s, dc, z0, near, far, xt } => Block::S { s: s.clone(), dc: dc.clone(), z0: z0.clone(), near: far.clone(), far: near.clone(), xt: xt.clone() },
            Block::Line { pieces, taps } => {
                // the pieces in reverse order, each seen from its other end; a tap behind piece j
                // now hangs behind piece n − 2 − j
                let n = pieces.len();
                Block::Line {
                    pieces: pieces.iter().rev().map(|(m, dc)| (m.iter().map(reversed).collect(), reversed(dc))).collect(),
                    taps: taps.iter().map(|t| Tap { after: n - 2 - t.after, ..t.clone() }).collect(),
                }
            }
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
    let mut all = receivers_transfer(freq, blocks, lines, tx, rx)?;
    let main = all.remove(0);
    Ok((main.h, main.dc))
}

/// As `ends_transfer`, for every receiver: the lane's own receiver first, then one per tap of a
/// `Block::Line` (each with the same receiver model, all loading the line at once).
pub fn receivers_transfer(freq: &[f64], blocks: &[Block], lines: usize, tx: &EndModel, rx: &EndModel) -> Result<Vec<RxTransfer>, String> {
    if blocks.iter().any(|b| b.lines() != lines) {
        return Err("blocks with different numbers of lines".into());
    }
    let npts = freq.len() + 1;
    let ws: Vec<f64> = std::iter::once(0.0).chain(freq.iter().map(|&f| 2.0 * PI * f)).collect();
    let mut net = Network::new(npts);
    let mut taps: Vec<(String, usize)> = Vec::new();
    // aggressors: (label, drive nodes per line) and the nodes of their other ends
    let mut aggr: Vec<(String, Vec<usize>)> = Vec::new();
    let mut aggr_loads: Vec<usize> = Vec::new();
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
            Block::S { s, dc, z0, near, far, xt } => {
                let np = z0.len();
                let mut nodes = Vec::with_capacity(np);
                let mut xt_nodes: Vec<Vec<usize>> = xt.iter().map(|x| vec![0; x.drive.len()]).collect();
                for p in 0..np {
                    let node = if let Some(l) = near.iter().position(|&q| q == p) {
                        bnd[i][l]
                    } else if let Some(l) = far.iter().position(|&q| q == p) {
                        bnd[i + 1][l]
                    } else if let Some((a, l)) = xt.iter().enumerate().find_map(|(a, x)| x.drive.iter().position(|&q| q == p).map(|l| (a, l))) {
                        let n = net.node(format!("block {i}, port {} (aggressor {})", p + 1, xt[a].label));
                        xt_nodes[a][l] = n;
                        n
                    } else if xt.iter().any(|x| x.load.contains(&p)) {
                        let n = net.node(format!("block {i}, port {} (aggressor end)", p + 1));
                        aggr_loads.push(n);
                        n
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
                for (x, n) in xt.iter().zip(xt_nodes) {
                    aggr.push((x.label.clone(), n));
                }
            }
            Block::Line { pieces, taps: tl } => {
                let n = pieces.len();
                let mut at: Vec<usize> = vec![bnd[i][0]];
                for j in 1..n {
                    at.push(net.node(format!("block {i}, junction {j}")));
                }
                at.push(bnd[i + 1][0]);
                for (j, (m, dc)) in pieces.iter().enumerate() {
                    let all: Vec<Abcd> = std::iter::once(*dc).chain(m.iter().copied()).collect();
                    net.two_port(at[j], at[j + 1], &all);
                }
                for t in tl {
                    let end = net.node(format!("tap {}", t.label));
                    let all: Vec<Abcd> = std::iter::once(t.dc).chain(t.abcd.iter().copied()).collect();
                    net.two_port(at[(t.after + 1).min(n)], end, &all);
                    taps.push((t.label.clone(), end));
                }
            }
        }
    }
    // ends, per line; a receiver: inductance, then capacitance and termination to AC ground
    let add_rx = |net: &mut Network, pin: usize, name: String| -> usize {
        let die = if rx.l_nh > 0.0 {
            let d = net.node(name);
            let z: Vec<Cx> = ws.iter().map(|w| Cx::new(0.0, w * rx.l_nh * 1e-9)).collect();
            net.series(pin, d, &z);
            d
        } else {
            pin
        };
        let g = if rx.r > 0.0 { 1.0 / rx.r } else { 0.0 };
        net.adm(die, GND, ws.iter().map(|w| Cx::new(g, w * rx.c_pf * 1e-12)).collect());
        die
    };
    // excitations: 0 = the lane's own transmitter, 1 + j = aggressor j
    let nexc = 1 + aggr.len();
    let one = |x: usize, v: f64| -> Vec<Cx> { (0..nexc).map(|q| Cx::new(if q == x { v } else { 0.0 }, 0.0)).collect() };
    let mut src = Vec::new();
    for (j, (_, drive)) in aggr.iter().enumerate() {
        for (l, &n) in drive.iter().enumerate() {
            if tx.c_pf > 0.0 {
                net.adm(n, GND, ws.iter().map(|w| Cx::new(0.0, w * tx.c_pf * 1e-12)).collect());
            }
            let e = if drive.len() == 2 { if l == 0 { 0.5 } else { -0.5 } } else { 1.0 };
            src.push(Source { node: n, z: Cx::new(tx.r.max(0.0), 0.0), e: one(1 + j, e) });
        }
    }
    for &n in &aggr_loads {
        add_rx(&mut net, n, String::new());
    }
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
        src.push(Source { node: die_tx, z: Cx::new(tx.r.max(0.0), 0.0), e: one(0, e) });
        let pin_rx = bnd[blocks.len()][l];
        probe.push(add_rx(&mut net, pin_rx, format!("receiver die, line {l}")));
    }
    let tap_dies: Vec<(String, usize)> = taps.iter().map(|(label, pin)| (label.clone(), add_rx(&mut net, *pin, format!("receiver die at {label}")))).collect();
    let scale = if rx.r > 0.0 { (tx.r.max(0.0) + rx.r) / rx.r } else { 1.0 };
    // h[receiver][excitation][point]
    let mut h: Vec<Vec<Vec<Cx>>> = vec![vec![Vec::with_capacity(npts); nexc]; 1 + tap_dies.len()];
    for k in 0..npts {
        let v = net.solve(k, &src)?;
        for x in 0..nexc {
            let out = if lines == 2 { v[x][probe[0]] - v[x][probe[1]] } else { v[x][probe[0]] };
            h[0][x].push(out.scale(scale));
            for (q, (_, die)) in tap_dies.iter().enumerate() {
                h[q + 1][x].push(v[x][*die].scale(scale));
            }
        }
    }
    let labels = std::iter::once(String::new()).chain(tap_dies.iter().map(|t| t.0.clone()));
    Ok(labels
        .zip(h)
        .map(|(label, mut hx)| {
            let own = hx.remove(0);
            let xt = aggr.iter().zip(hx).map(|((al, _), hx)| (al.clone(), hx[1..].to_vec(), hx[0].re)).collect();
            RxTransfer { label, dc: own[0].re, h: own[1..].to_vec(), xt }
        })
        .collect())
}

/// The channel behind a connector as a block (`conn_end`): the Touchstone file (2-port on each
/// line, or the 4-port of a pair with its port order, mode conversion included) or the lossy
/// line, on the grid `freq`, with port 1 (the near end) at the connector unless `flip`.
/// `lines`: 2 for a pair's 4-port; 1 with `diff` for a pair in the odd mode (a 4-port file is
/// then reduced to the differential mode). `nyq`: the profile's Nyquist frequency (the lossy
/// line's loss is given there), `z0` per line. Returns the block (near end at the connector)
/// and a note.
pub fn rest_block(ce: &ConnEnd, ts: Option<&Touchstone>, freq: &[f64], lines: usize, diff: bool, nyq: f64, z0: f64, pins: &[String], aggressors: &[XtLane]) -> Result<(Block, String), String> {
    let line_block = |m: Vec<Abcd>, dc: Abcd| -> Block {
        if lines == 2 {
            Block::Four { t: m.iter().map(|x| sep(x, x)).collect(), dc: sep(&dc, &dc) }
        } else {
            Block::Two { m, dc }
        }
    };
    match (&ce.touchstone, ts) {
        (Some(tr), Some(ts)) if !tr.ports.is_empty() => {
            // one file for the lanes of the connector: the lane's pins, the others as aggressors
            let port = |pin: &str| -> Result<(usize, usize), String> {
                let p = tr.ports.get(pin).ok_or_else(|| format!("{}: pin {pin} is not in the port map (touchstone \"ports\")", ts.name))?;
                if p.iter().any(|&x| x == 0 || x > ts.nports) {
                    return Err(format!("{}: pin {pin} maps to port {} / {}, the file has {} ports", ts.name, p[0], p[1], ts.nports));
                }
                Ok(if tr.flip { (p[1] - 1, p[0] - 1) } else { (p[0] - 1, p[1] - 1) })
            };
            if pins.len() != lines {
                return Err(format!("{}: the lane's connector pins are not known", ts.name));
            }
            let own: Vec<(usize, usize)> = pins.iter().map(|p| port(p)).collect::<Result<_, _>>()?;
            let mut xt = Vec::new();
            let mut skipped = Vec::new();
            for a in aggressors {
                if a.pins.iter().any(|p| pins.contains(p)) {
                    continue;
                }
                match a.pins.iter().map(|p| port(p)).collect::<Result<Vec<_>, _>>() {
                    Ok(ps) => {
                        let (near, far): (Vec<usize>, Vec<usize>) = ps.into_iter().unzip();
                        // its transmitter at the connector's side when the connector receives
                        let (drive, load) = if a.sends { (far, near) } else { (near, far) };
                        xt.push(XtSrc { label: a.label.clone(), drive, load });
                    }
                    Err(_) => skipped.push(a.label.clone()),
                }
            }
            let mut notes = ts.extrapolation_notes(freq);
            notes.extend(ts.passivity_note());
            notes.push(format!("ports {} of the lane{}", own.iter().map(|(a, b)| format!("{}→{}", a + 1, b + 1)).collect::<Vec<_>>().join(", "), if xt.is_empty() { String::new() } else { format!(", {} aggressor lane(s) of the connector in the file (their board traces not included: an estimate, conservative)", xt.len()) }));
            if !skipped.is_empty() {
                notes.push(format!("{} other lane(s) of the connector not in the port map", skipped.len()));
            }
            let dc = ts.s_at(0.0);
            let s: Vec<Vec<Cx>> = freq.iter().map(|&f| ts.s_at(f)).collect();
            let (near, far) = own.into_iter().unzip();
            Ok((Block::S { s, dc, z0: ts.z0.clone(), near, far, xt }, notes.join("; ")))
        }
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
                notes.extend(ts.passivity_note());
                let dc = ts.s_at(0.0);
                let s: Vec<Vec<Cx>> = freq.iter().map(|&f| ts.s_at(f)).collect();
                return Ok((Block::S { s, dc, z0: ts.z0.clone(), near, far, xt: vec![] }, notes.join("; ")));
            }
            let grid: Vec<f64> = std::iter::once(0.0).chain(freq.iter().copied()).collect();
            let (m, mut notes) = ts.per_line_abcd(&grid, diff || lines == 2, tr.port_order.as_deref(), tr.flip)?;
            notes.extend(ts.passivity_note());
            Ok((line_block(m[1..].to_vec(), m[0]), notes.join("; ")))
        }
        (Some(tr), None) => Err(format!("Touchstone file {} not loaded", tr.file)),
        (None, _) => {
            let m: Vec<Abcd> = freq.iter().map(|&f| ce.rest_abcd(f, nyq, z0).unwrap_or_else(Abcd::identity)).collect();
            Ok((line_block(m, Abcd::identity()), format!("a lossy line of {:.1} dB at {:.2} GHz, {:.0} ps", ce.rest_loss_db.unwrap_or(0.0).abs(), nyq / 1e9, ce.rest_delay_ps.unwrap_or(500.0))))
        }
    }
}

/// Another lane at the same connector, a possible aggressor in an N-port file: its connector
/// pins (P first) and whether the connector is its transmitting end.
#[derive(Debug, Clone)]
pub struct XtLane {
    pub label: String,
    pub pins: Vec<String>,
    pub sends: bool,
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

/// A note when the board's own model is not passive somewhere (largest singular value of S above
/// 1 + 1e-6): a modelling error, never expected – the line, via and part models are passive.
pub fn passivity_check(freq: &[f64], s: &[Vec<Cx>], n: usize) -> Option<String> {
    let (worst, k) = crate::rf::passivity(s, n);
    (worst > 1.0 + 1e-6).then(|| format!("model not passive at {:.2} GHz (largest singular value of S {:.6}): a modelling error, please report", freq[k] / 1e9, worst))
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

    /// A receiver on a side stub (multi-drop): the network gives the cascade with the stub loaded
    /// by the receiver, Y_in = (C + D·Y_L)/(A + B·Y_L), and the voltage at the tap V_j/(A + B·Y_L);
    /// flipping the block twice changes nothing.
    #[test]
    fn tap_receiver() {
        let freq: Vec<f64> = (1..=30).map(|k| k as f64 * 0.5e9).collect();
        let p1: Vec<Abcd> = freq.iter().map(|&f| line(50.0, 3.2, 6.0, f, 0.03)).collect();
        let p2: Vec<Abcd> = freq.iter().map(|&f| line(50.0, 3.2, 6.0, f, 0.02)).collect();
        let st: Vec<Abcd> = freq.iter().map(|&f| line(55.0, 3.0, 6.0, f, 0.008)).collect();
        let tx = EndModel { r: 0.0, c_pf: 2.0, l_nh: 0.0 };
        let rx = EndModel { r: 0.0, c_pf: 1.5, l_nh: 0.0 };
        let id = Abcd::identity();
        let tap = Tap { label: "U3.5".into(), after: 0, abcd: st.clone(), dc: id };
        let blk = Block::Line { pieces: vec![(p1.clone(), id), (p2.clone(), id)], taps: vec![tap] };
        let r = receivers_transfer(&freq, &[blk.clone()], 1, &tx, &rx).unwrap();
        assert_eq!((r.len(), r[1].label.as_str()), (2, "U3.5"));
        for (k, &f) in freq.iter().enumerate() {
            let w = 2.0 * PI * f;
            let yl = Cx::new(0.0, w * 1.5e-12);
            let yin = (st[k].c + st[k].d * yl) / (st[k].a + st[k].b * yl);
            // cascade to the main receiver with the loaded stub as a shunt, ideal source
            let m = p1[k].mul(&Abcd::shunt(yin)).mul(&p2[k]).mul(&Abcd::shunt(yl));
            let h_main = m.a.inv();
            assert!((r[0].h[k] - h_main).abs() < 1e-10 * (1.0 + h_main.abs()), "{k}");
            // the junction voltage, then down the stub
            let mj = Abcd::shunt(yin).mul(&p2[k]).mul(&Abcd::shunt(yl));
            let vj = h_main * mj.a;
            let h_tap = vj / (st[k].a + st[k].b * yl);
            assert!((r[1].h[k] - h_tap).abs() < 1e-10 * (1.0 + h_tap.abs()), "{k}");
        }
        let back = blk.flipped().flipped();
        let r2 = receivers_transfer(&freq, &[back], 1, &tx, &rx).unwrap();
        for k in 0..freq.len() {
            assert!((r2[1].h[k] - r[1].h[k]).abs() < 1e-9);
        }
    }

    /// One file for two single-ended lanes behind a connector (ports: pin 1 → 1/3, pin 2 → 2/4):
    /// the lane's own transfer through the S block equals the 2-port of its line when the lines
    /// are uncoupled; coupled, the other lane is an aggressor and closes the eye by twice its
    /// peak crosstalk.
    #[test]
    fn n_port_file_with_aggressor() {
        let freq: Vec<f64> = (1..=200).map(|k| k as f64 * 0.1e9).collect();
        let line50 = |f: f64| line(50.0, 3.2, 3.0, f, 0.05);
        let file = |coupled_: bool| -> String {
            // S of the two lines in the file's port order: 1 = A near, 2 = B near, 3 = A far, 4 = B far
            let mut t = String::from("# Hz S RI R 50\n");
            for &f in &freq {
                let tm = if coupled_ { coupled(&line(56.0, 3.4, 3.0, f, 0.05), &line(44.5, 3.0, 3.0, f, 0.05)) } else { sep(&line50(f), &line50(f)) };
                let sm = s_of(&tm, 50.0);
                t += &format!("{f}");
                for r in 0..4 {
                    for c in 0..4 {
                        t += &format!(" {:.12e} {:.12e}", sm[r][c].re, sm[r][c].im);
                    }
                }
                t += "\n";
            }
            t
        };
        let mut ports = std::collections::BTreeMap::new();
        ports.insert("1".to_string(), [1, 3]);
        ports.insert("2".to_string(), [2, 4]);
        let ce = ConnEnd { model: "matched".into(), touchstone: Some(crate::eye::TsRef { file: "two.s4p".into(), ports, ..Default::default() }), ..Default::default() };
        let aggr = [XtLane { label: "B".into(), pins: vec!["2".into()], sends: false }, XtLane { label: "A".into(), pins: vec!["1".into()], sends: false }];
        let end = EndModel { r: 50.0, c_pf: 0.0, l_nh: 0.0 };
        let board: Vec<Abcd> = freq.iter().map(|&f| line50(f).mul(&line50(f))).collect();
        let id = Abcd::identity();
        let mut heights = vec![];
        for coupled_ in [false, true] {
            let ts = crate::rf::parse_touchstone(&file(coupled_), "two.s4p").unwrap();
            let (blk, note) = rest_block(&ce, Some(&ts), &freq, 1, false, 2.5e9, 50.0, &["1".to_string()], &aggr).unwrap();
            assert!(note.starts_with("ports 1→3") && note.contains("1 aggressor lane"), "{note}");
            let r = receivers_transfer(&freq, &[Block::Two { m: board.clone(), dc: id }, blk], 1, &end, &end).unwrap();
            assert_eq!(r[0].xt.len(), 1);
            let ep = crate::eye::EyeParams { bitrate: 5e9, rise: 50e-12, ..Default::default() };
            let e = crate::eye::eye_with_xt(&freq, &r[0].h, Some(r[0].dc), 600.0, &ep, &r[0].xt).unwrap();
            let x = e.xt.clone().unwrap();
            if !coupled_ {
                // uncoupled: the lane alone, through its line in the file
                let lone: Vec<Abcd> = freq.iter().map(|&f| line50(f).mul(&line50(f)).mul(&line50(f))).collect();
                let (h1, _) = ends_transfer(&freq, &[Block::Two { m: lone, dc: id }], 1, &end, &end).unwrap();
                for k in 0..freq.len() {
                    assert!((r[0].h[k] - h1[k]).abs() < 1e-8, "{k}");
                }
                assert!(x.peak < 1e-6, "{x:?}");
            } else {
                assert!(x.peak > 0.01, "coupled lines: crosstalk {x:?}");
                assert!((x.height_alone - e.height - 2.0 * x.peak).abs() < 0.02, "{x:?} {}", e.height);
            }
            heights.push(e.height);
        }
        assert!(heights[1] < heights[0], "{heights:?}");
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
        let blk = Block::S { s: s.clone(), dc: s[0].clone(), z0: vec![50.0; 4], near: vec![0, 2], far: vec![1, 3], xt: vec![] };
        let (h2, _) = ends_transfer(&freq, &[Block::Four { t: board, dc: id }, blk], 2, &tx, &rx).unwrap();
        for k in 0..freq.len() {
            assert!((h1[k] - h2[k]).abs() < 1e-10, "{k}");
        }
    }
}

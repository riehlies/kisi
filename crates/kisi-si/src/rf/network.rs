//! Linear networks of N-port elements (milestone M2 of the channel-simulation concept,
//! `docs/research/channel-simulation-ibis.md`): a lane or a net as nodes joined by chains of
//! 2-ports (ABCD) or of coupled line pairs (4×4 cascade matrices, `rf::mixed`), admittances
//! and S-parameter blocks (Touchstone files), solved by modified nodal analysis at every
//! frequency. It gives the S-parameters at any set of ports and the voltage at every node for
//! any set of sources – the transfer from a transmitter to every receiver.
//!
//! A chain of m lines between the node lists `from` and `to` (m nodes each) adds 2m unknowns:
//! the currents into the chain at `from` and out of it at `to`, with
//!
//! ```text
//! [V_from; I_from] = T · [V_to; I_to]
//! ```
//!
//! (T = ABCD for m = 1; for m = 2 the cascade matrix of a pair, state order V1 V2 I1 I2). Any T
//! works, also a pure shunt or a zero-impedance series part, which have no admittance matrix.
//! Today's cascade (`rf::channel`) is one chain; a branch is a chain to a node of its own, and
//! an open branch is the shunt admittance C/A the cascade uses – both give the same result to
//! numerical precision (tests).
//!
//! An S-parameter block with N ports (reference z0 per port, to ground) adds N unknowns, the
//! port currents, with (v − z0·i) = S·(v + z0·i) in power waves.
//!
//! Units: SI. Nodes are numbered from 0; `GND` is the reference node.

use super::{Abcd, Cx};

/// The reference node (ground).
pub const GND: usize = usize::MAX;

fn cz() -> Cx {
    Cx::new(0.0, 0.0)
}
fn c1() -> Cx {
    Cx::new(1.0, 0.0)
}

/// One element of a network; values per point of the network's frequency list.
#[derive(Debug, Clone)]
pub enum Elem {
    /// m coupled lines from `from` to `to`; `t[k]` is the (2m)×(2m) cascade matrix, row-major
    Chain { from: Vec<usize>, to: Vec<usize>, t: Vec<Vec<Cx>> },
    /// admittance between `a` and `b` (either may be `GND`)
    Adm { a: usize, b: usize, y: Vec<Cx> },
    /// N-port to ground at `nodes`, reference impedances `z0`; `s[k]` is N×N, row-major
    SBlock { nodes: Vec<usize>, z0: Vec<f64>, s: Vec<Vec<Cx>> },
}

/// A source at a node: emf `e` (one value per excitation) behind the impedance `z`
/// (0: an ideal voltage source).
#[derive(Debug, Clone)]
pub struct Source {
    pub node: usize,
    pub z: Cx,
    pub e: Vec<Cx>,
}

#[derive(Debug, Clone, Default)]
pub struct Network {
    /// number of points every element carries values for
    pub npts: usize,
    /// node names (for messages)
    pub names: Vec<String>,
    pub elems: Vec<Elem>,
}

impl Network {
    pub fn new(npts: usize) -> Self {
        Network { npts, names: vec![], elems: vec![] }
    }

    /// A new node.
    pub fn node(&mut self, name: impl Into<String>) -> usize {
        self.names.push(name.into());
        self.names.len() - 1
    }

    pub fn nodes(&self) -> usize {
        self.names.len()
    }

    /// A chain of `from.len()` lines (cascade matrices per point).
    pub fn chain(&mut self, from: Vec<usize>, to: Vec<usize>, t: Vec<Vec<Cx>>) {
        let m = from.len();
        assert!(
            m > 0 && to.len() == m && t.len() == self.npts && t.iter().all(|x| x.len() == 4 * m * m),
            "chain: sizes"
        );
        self.elems.push(Elem::Chain { from, to, t });
    }

    /// A 2-port from `a` (port 1) to `b` (port 2).
    pub fn two_port(&mut self, a: usize, b: usize, m: &[Abcd]) {
        self.chain(vec![a], vec![b], m.iter().map(|x| vec![x.a, x.b, x.c, x.d]).collect());
    }

    /// A coupled pair from (`a1`, `a2`) to (`b1`, `b2`): 4×4 cascade matrices (state V1 V2 I1 I2).
    pub fn four_port(&mut self, a: [usize; 2], b: [usize; 2], t: &[[[Cx; 4]; 4]]) {
        self.chain(a.to_vec(), b.to_vec(), t.iter().map(|m| m.iter().flatten().copied().collect()).collect());
    }

    pub fn adm(&mut self, a: usize, b: usize, y: Vec<Cx>) {
        assert!(y.len() == self.npts, "admittance: one value per point");
        self.elems.push(Elem::Adm { a, b, y });
    }

    /// Impedance between `a` and `b` as a chain (works for z = 0 as well).
    pub fn series(&mut self, a: usize, b: usize, z: &[Cx]) {
        let m: Vec<Abcd> = z.iter().map(|&z| Abcd::series(z)).collect();
        self.two_port(a, b, &m);
    }

    pub fn s_block(&mut self, nodes: Vec<usize>, z0: Vec<f64>, s: Vec<Vec<Cx>>) {
        let n = nodes.len();
        assert!(z0.len() == n && s.len() == self.npts && s.iter().all(|x| x.len() == n * n), "S block: sizes");
        self.elems.push(Elem::SBlock { nodes, z0, s });
    }

    /// Node voltages at point `k` for the sources: one vector of node voltages per excitation
    /// (every source has the same number of excitations).
    pub fn solve(&self, k: usize, src: &[Source]) -> Result<Vec<Vec<Cx>>, String> {
        let nn = self.nodes();
        let nexc = src.first().map(|s| s.e.len()).unwrap_or(1);
        // unknowns: node voltages, element currents, currents of ideal sources
        let mut aux = Vec::with_capacity(self.elems.len());
        let mut n = nn;
        for e in &self.elems {
            aux.push(n);
            n += match e {
                Elem::Chain { from, .. } => 2 * from.len(),
                Elem::Adm { .. } => 0,
                Elem::SBlock { nodes, .. } => nodes.len(),
            };
        }
        let ideal: Vec<usize> = src.iter().enumerate().filter(|(_, s)| s.z.abs() == 0.0).map(|(i, _)| i).collect();
        let first_src = n;
        n += ideal.len();
        let mut a = vec![cz(); n * n];
        let mut b = vec![cz(); n * nexc];
        let add = |r: usize, c: usize, v: Cx, a: &mut Vec<Cx>| {
            if r != GND && c != GND {
                a[r * n + c] = a[r * n + c] + v;
            }
        };
        for (ei, e) in self.elems.iter().enumerate() {
            let c0 = aux[ei];
            match e {
                Elem::Adm { a: na, b: nb, y } => {
                    let y = y[k];
                    add(*na, *na, y, &mut a);
                    add(*nb, *nb, y, &mut a);
                    add(*na, *nb, cz() - y, &mut a);
                    add(*nb, *na, cz() - y, &mut a);
                }
                Elem::Chain { from, to, t } => {
                    let m = from.len();
                    let t = &t[k];
                    let tt = |r: usize, c: usize| t[r * 2 * m + c];
                    for i in 0..m {
                        // current into the chain leaves node from[i]; the one out of it enters to[i]
                        add(from[i], c0 + i, c1(), &mut a);
                        add(to[i], c0 + m + i, Cx::new(-1.0, 0.0), &mut a);
                    }
                    for r in 0..m {
                        // V_from[r] − Σ T[r][j]·V_to[j] − Σ T[r][m+j]·I_to[j] = 0
                        let row = c0 + r;
                        add(row, from[r], c1(), &mut a);
                        // I_from[r] − Σ T[m+r][j]·V_to[j] − Σ T[m+r][m+j]·I_to[j] = 0
                        let row2 = c0 + m + r;
                        a[row2 * n + c0 + r] = a[row2 * n + c0 + r] + c1();
                        for j in 0..m {
                            add(row, to[j], cz() - tt(r, j), &mut a);
                            a[row * n + c0 + m + j] = a[row * n + c0 + m + j] - tt(r, m + j);
                            add(row2, to[j], cz() - tt(m + r, j), &mut a);
                            a[row2 * n + c0 + m + j] = a[row2 * n + c0 + m + j] - tt(m + r, m + j);
                        }
                    }
                }
                Elem::SBlock { nodes, z0, s } => {
                    let np = nodes.len();
                    let s = &s[k];
                    for p in 0..np {
                        add(nodes[p], c0 + p, c1(), &mut a);
                    }
                    for r in 0..np {
                        // v_r/√z_r − √z_r·i_r − Σ_c S_rc·(v_c + z_c·i_c)/√z_c = 0
                        let row = c0 + r;
                        let sr = z0[r].sqrt();
                        add(row, nodes[r], Cx::new(1.0 / sr, 0.0), &mut a);
                        a[row * n + c0 + r] = a[row * n + c0 + r] - Cx::new(sr, 0.0);
                        for c in 0..np {
                            let sc = z0[c].sqrt();
                            let src_ = s[r * np + c];
                            add(row, nodes[c], cz() - src_.scale(1.0 / sc), &mut a);
                            a[row * n + c0 + c] = a[row * n + c0 + c] - src_.scale(sc);
                        }
                    }
                }
            }
        }
        for (si, s) in src.iter().enumerate() {
            if s.e.len() != nexc {
                return Err("sources with different numbers of excitations".into());
            }
            if s.node == GND {
                continue;
            }
            match ideal.iter().position(|&i| i == si) {
                Some(q) => {
                    // ideal source: its current enters the node, V_node = e
                    let col = first_src + q;
                    a[s.node * n + col] = a[s.node * n + col] - c1();
                    a[col * n + s.node] = c1();
                    for x in 0..nexc {
                        b[col * nexc + x] = s.e[x];
                    }
                }
                None => {
                    // Norton: e/z into the node, 1/z to ground
                    let y = s.z.inv();
                    a[s.node * n + s.node] = a[s.node * n + s.node] + y;
                    for x in 0..nexc {
                        b[s.node * nexc + x] = b[s.node * nexc + x] + s.e[x] * y;
                    }
                }
            }
        }
        solve_dense(&mut a, n, &mut b, nexc).map_err(|e| format!("{e} (network with {nn} nodes)"))?;
        Ok((0..nexc).map(|x| (0..nn).map(|i| b[i * nexc + x]).collect()).collect())
    }

    /// S-matrix (N×N, row-major) at point `k` for the ports (node, reference impedance).
    pub fn s_params(&self, k: usize, ports: &[(usize, f64)]) -> Result<Vec<Cx>, String> {
        let np = ports.len();
        let src: Vec<Source> = ports
            .iter()
            .enumerate()
            .map(|(j, &(node, z0))| Source {
                node,
                z: Cx::new(z0, 0.0),
                e: (0..np).map(|x| if x == j { c1() } else { cz() }).collect(),
            })
            .collect();
        let v = self.solve(k, &src)?;
        let mut s = vec![cz(); np * np];
        for j in 0..np {
            for i in 0..np {
                // b_i / a_j = (2·V_i − δij)·√(z0_j / z0_i)
                let vi = v[j][ports[i].0];
                let d = if i == j { c1() } else { cz() };
                s[i * np + j] = (vi.scale(2.0) - d).scale((ports[j].1 / ports[i].1).sqrt());
            }
        }
        Ok(s)
    }
}

/// Solve A·X = B in place (A n×n, B n×m, both row-major) by LU with partial pivoting.
pub(crate) fn solve_dense(a: &mut [Cx], n: usize, b: &mut [Cx], m: usize) -> Result<(), String> {
    let scale = a.iter().map(|x| x.abs()).fold(0.0, f64::max).max(1e-300);
    for k in 0..n {
        let (mut p, mut best) = (k, a[k * n + k].abs());
        for i in k + 1..n {
            let v = a[i * n + k].abs();
            if v > best {
                best = v;
                p = i;
            }
        }
        if !(best > scale * 1e-15) {
            return Err("singular network: a node without a connection".into());
        }
        if p != k {
            for j in 0..n {
                a.swap(k * n + j, p * n + j);
            }
            for j in 0..m {
                b.swap(k * m + j, p * m + j);
            }
        }
        let inv = a[k * n + k].inv();
        for i in k + 1..n {
            let f = a[i * n + k] * inv;
            if f.re == 0.0 && f.im == 0.0 {
                continue;
            }
            a[i * n + k] = f;
            for j in k + 1..n {
                let u = a[k * n + j];
                if u.re != 0.0 || u.im != 0.0 {
                    a[i * n + j] = a[i * n + j] - f * u;
                }
            }
            for j in 0..m {
                b[i * m + j] = b[i * m + j] - f * b[k * m + j];
            }
        }
    }
    for i in (0..n).rev() {
        let inv = a[i * n + i].inv();
        for j in 0..m {
            let mut s = b[i * m + j];
            for c in i + 1..n {
                s = s - a[i * n + c] * b[c * m + j];
            }
            b[i * m + j] = s * inv;
        }
    }
    Ok(())
}

/// Largest singular value of an n×n matrix (row-major): > 1 means the S-matrix is not passive.
pub fn max_singular(s: &[Cx], n: usize) -> f64 {
    if n == 0 {
        return 0.0;
    }
    // power iteration on G = Sᴴ·S (Hermitian, positive semi-definite)
    let mut g = vec![cz(); n * n];
    for i in 0..n {
        for j in 0..n {
            let mut acc = cz();
            for r in 0..n {
                let a = s[r * n + i];
                acc = acc + Cx::new(a.re, -a.im) * s[r * n + j];
            }
            g[i * n + j] = acc;
        }
    }
    let mut v: Vec<Cx> = (0..n).map(|i| Cx::new(1.0, 0.1 * i as f64)).collect();
    let mut lam = 0.0;
    for _ in 0..200 {
        let w: Vec<Cx> = (0..n).map(|i| (0..n).fold(cz(), |acc, j| acc + g[i * n + j] * v[j])).collect();
        let norm = w.iter().map(|x| x.abs() * x.abs()).sum::<f64>().sqrt();
        if norm == 0.0 {
            return 0.0;
        }
        let vn = v.iter().map(|x| x.abs() * x.abs()).sum::<f64>().sqrt();
        let l = norm / vn.max(1e-300);
        v = w.iter().map(|x| x.scale(1.0 / norm)).collect();
        if (l - lam).abs() <= 1e-13 * l {
            lam = l;
            break;
        }
        lam = l;
    }
    lam.sqrt()
}

/// Passivity of S-matrices over frequency: the worst largest singular value and its point.
pub fn passivity(s: &[Vec<Cx>], n: usize) -> (f64, usize) {
    s.iter().enumerate().map(|(k, m)| (max_singular(m, n), k)).fold((0.0, 0), |a, b| if b.0 > a.0 { b } else { a })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rf::{coupled, mixed_of, s_of, sep, C0};
    use std::f64::consts::PI;

    fn line(zc: f64, eeff: f64, loss: f64, f: f64, len: f64) -> Abcd {
        let l = zc * eeff.sqrt() / C0;
        let c = eeff.sqrt() / (zc * C0);
        Abcd::line(loss * zc, l, loss / zc * 0.3, c, 2.0 * PI * f, len)
    }

    fn close(a: Cx, b: Cx, tol: f64) -> bool {
        (a - b).abs() <= tol * (1.0 + b.abs())
    }

    /// A cascade of lines, a series part and a shunt part: the network of one chain per element
    /// (with nodes between them) gives the cascade's S-parameters.
    #[test]
    fn cascade_as_network() {
        let freq = [0.1e9, 1e9, 5e9, 13e9, 25e9];
        let mut net = Network::new(freq.len());
        let n: Vec<usize> = (0..5).map(|i| net.node(format!("n{i}"))).collect();
        let els = |f: f64| {
            let w = 2.0 * PI * f;
            vec![
                line(48.0, 3.1, 0.4, f, 0.021),
                Abcd::series(Cx::new(22.0, w * 0.4e-9)),
                Abcd::shunt(Cx::new(1e-3, w * 0.3e-12)),
                line(55.0, 3.6, 0.6, f, 0.013),
            ]
        };
        for i in 0..4 {
            let m: Vec<Abcd> = freq.iter().map(|&f| els(f)[i]).collect();
            net.two_port(n[i], n[i + 1], &m);
        }
        for (k, &f) in freq.iter().enumerate() {
            let m = els(f).iter().fold(Abcd::identity(), |a, b| a.mul(b));
            let (s11, s21, s22) = m.s_params(50.0);
            let s = net.s_params(k, &[(n[0], 50.0), (n[4], 50.0)]).unwrap();
            assert!(
                close(s[0], s11, 1e-11) && close(s[2], s21, 1e-11) && close(s[1], s21, 1e-9) && close(s[3], s22, 1e-11),
                "{f}: {s:?} vs {s11:?} {s21:?} {s22:?}"
            );
        }
    }

    /// An open branch as a chain to its own node is the shunt C/A of the cascade; a branch with a
    /// load at its end is the shunt (C + D·Y)/(A + B·Y), and the voltage there is V/(A + B·Y).
    #[test]
    fn branch_is_the_shunt_of_the_cascade() {
        let freq = [0.5e9, 3e9, 9e9, 20e9];
        for load in [None, Some(Cx::new(1.0 / 50.0, 0.0))] {
            let mut net = Network::new(freq.len());
            let (a, j, b, e) = (net.node("A"), net.node("junction"), net.node("B"), net.node("branch end"));
            let main1: Vec<Abcd> = freq.iter().map(|&f| line(50.0, 3.3, 0.5, f, 0.018)).collect();
            let main2: Vec<Abcd> = freq.iter().map(|&f| line(50.0, 3.3, 0.5, f, 0.007)).collect();
            let br: Vec<Abcd> = freq.iter().map(|&f| line(60.0, 3.0, 0.5, f, 0.0042)).collect();
            net.two_port(a, j, &main1);
            net.two_port(j, b, &main2);
            net.two_port(j, e, &br);
            if let Some(y) = load {
                net.adm(e, GND, vec![y; freq.len()]);
            }
            for k in 0..freq.len() {
                let yb = match load {
                    None => br[k].c / br[k].a,
                    Some(y) => (br[k].c + br[k].d * y) / (br[k].a + br[k].b * y),
                };
                let m = main1[k].mul(&Abcd::shunt(yb)).mul(&main2[k]);
                let (s11, s21, _) = m.s_params(50.0);
                let s = net.s_params(k, &[(a, 50.0), (b, 50.0)]).unwrap();
                assert!(close(s[0], s11, 1e-11) && close(s[2], s21, 1e-11), "{load:?} {k}");
                if let Some(y) = load {
                    // voltage at the branch end for a 1 V source behind 50 Ω at A, 50 Ω at B
                    let v = net
                        .solve(
                            k,
                            &[
                                Source { node: a, z: Cx::new(50.0, 0.0), e: vec![c1()] },
                                Source { node: b, z: Cx::new(50.0, 0.0), e: vec![cz()] },
                            ],
                        )
                        .unwrap();
                    let vj = v[0][j];
                    assert!(close(v[0][e], vj / (br[k].a + br[k].b * y), 1e-11));
                }
            }
        }
    }

    /// A zero-impedance series part and a pure shunt are chains too (no admittance matrix); an
    /// ideal source drives its node to the emf.
    #[test]
    fn degenerate_chains_and_ideal_source() {
        let mut net = Network::new(1);
        let (a, b, c) = (net.node("a"), net.node("b"), net.node("c"));
        net.series(a, b, &[cz()]);
        net.two_port(b, c, &[Abcd::shunt(Cx::new(0.01, 0.0))]);
        net.adm(c, GND, vec![Cx::new(0.02, 0.0)]);
        let v = net.solve(0, &[Source { node: a, z: cz(), e: vec![Cx::new(2.0, 0.0)] }]).unwrap();
        assert!(
            close(v[0][a], Cx::new(2.0, 0.0), 1e-14)
                && close(v[0][b], Cx::new(2.0, 0.0), 1e-14)
                && close(v[0][c], Cx::new(2.0, 0.0), 1e-14)
        );
        // a floating node is reported
        let mut f = Network::new(1);
        let (x, _y) = (f.node("x"), f.node("y"));
        f.adm(x, GND, vec![c1()]);
        assert!(f.solve(0, &[Source { node: x, z: c1(), e: vec![c1()] }]).is_err());
    }

    /// A coupled pair as one 4-port chain gives `rf::mixed::s_of`; two uncoupled lines (`sep`) as
    /// a 4-port equal two 2-port chains.
    #[test]
    fn pair_chain_matches_the_mixed_mode_model() {
        let f = 6e9;
        let w = 2.0 * PI * f;
        let ev = Abcd::line(2.0, 60.0 * 3.4f64.sqrt() / C0, 1e-4, 3.4f64.sqrt() / (60.0 * C0), w, 0.025);
        let od = Abcd::line(3.0, 45.0 * 3.0f64.sqrt() / C0, 2e-4, 3.0f64.sqrt() / (45.0 * C0), w, 0.025);
        let t = coupled(&ev, &od);
        let mut net = Network::new(1);
        let p: Vec<usize> = (0..4).map(|i| net.node(format!("p{i}"))).collect();
        net.four_port([p[0], p[1]], [p[2], p[3]], &[t]);
        let s = net.s_params(0, &[(p[0], 45.0), (p[1], 45.0), (p[2], 45.0), (p[3], 45.0)]).unwrap();
        let s_ref = s_of(&t, 45.0);
        for i in 0..4 {
            for j in 0..4 {
                assert!(close(s[i * 4 + j], s_ref[i][j], 1e-11), "S{}{}", i + 1, j + 1);
            }
        }
        let mm = mixed_of(&s_ref);
        // symmetric pair: no mode conversion
        assert!(mm[3][0].abs() < 1e-12 && mm[1][2].abs() < 1e-12);
        let (l1, l2) = (line(50.0, 3.2, 0.3, f, 0.02), line(50.0, 3.2, 0.3, f, 0.023));
        let mut a = Network::new(1);
        let q: Vec<usize> = (0..4).map(|i| a.node(format!("q{i}"))).collect();
        a.four_port([q[0], q[1]], [q[2], q[3]], &[sep(&l1, &l2)]);
        let mut b = Network::new(1);
        let r: Vec<usize> = (0..4).map(|i| b.node(format!("r{i}"))).collect();
        b.two_port(r[0], r[2], &[l1]);
        b.two_port(r[1], r[3], &[l2]);
        let sa = a.s_params(0, &[(q[0], 50.0), (q[1], 50.0), (q[2], 50.0), (q[3], 50.0)]).unwrap();
        let sb = b.s_params(0, &[(r[0], 50.0), (r[1], 50.0), (r[2], 50.0), (r[3], 50.0)]).unwrap();
        for i in 0..16 {
            assert!(close(sa[i], sb[i], 1e-12));
        }
    }

    /// An S block of a 2-port gives back its S-parameters, also with another reference impedance
    /// at the ports; passivity of a lossless and of an amplifying matrix.
    #[test]
    fn s_block_and_passivity() {
        let f = 4e9;
        let m = line(62.0, 3.0, 0.8, f, 0.03);
        let (s11, s21, s22) = m.s_params(50.0);
        let mut net = Network::new(1);
        let (a, b) = (net.node("a"), net.node("b"));
        net.s_block(vec![a, b], vec![50.0, 50.0], vec![vec![s11, s21, s21, s22]]);
        let s = net.s_params(0, &[(a, 50.0), (b, 50.0)]).unwrap();
        assert!(close(s[0], s11, 1e-12) && close(s[2], s21, 1e-12) && close(s[3], s22, 1e-12));
        let (t11, t21, _) = m.s_params(75.0);
        let s75 = net.s_params(0, &[(a, 75.0), (b, 75.0)]).unwrap();
        assert!(close(s75[0], t11, 1e-12) && close(s75[2], t21, 1e-12));
        // passivity: a lossy line < 1, a lossless one = 1, gain > 1
        assert!(max_singular(&[s11, s21, s21, s22], 2) < 1.0);
        let ll = line(62.0, 3.0, 0.0, f, 0.03).s_params(50.0);
        assert!((max_singular(&[ll.0, ll.1, ll.1, ll.2], 2) - 1.0).abs() < 1e-9);
        assert!((max_singular(&[cz(), Cx::new(1.2, 0.0), Cx::new(0.3, 0.0), cz()], 2) - 1.2).abs() < 1e-9);
    }
}

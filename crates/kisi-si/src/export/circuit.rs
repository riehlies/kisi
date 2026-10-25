//! The exported circuit: a small netlist representation that can be evaluated (cascade
//! matrices, to check it against kisi's own model) and written as SPICE subcircuits.
//!
//! A single line is a `Chain` of two-port elements between its two ports (series impedances,
//! shunt admittances to the reference, RLGC ladders, open stubs, named sub-chains). A pair is a
//! row of `PairEl`: an element on one line, or a coupled block in modal form – the even- and
//! odd-mode chains between ideal mode converters (V_P = V_e + V_o, V_N = V_e − V_o,
//! I_P = I_e + I_o, I_N = I_e − I_o), exactly how kisi computes coupled sections
//! (`rf::mixed::coupled`). The converters are voltage-controlled voltage sources and
//! current-controlled current sources (E and F elements), which ngspice, Xyce, LTspice and Qucs
//! all read.
//!
//! The compact model (`export::Model::Compact`) writes line sections as lossless-line cells
//! (`El::Cells`): lossless LC ladders (40 π sections per wavelength at the top frequency, 2 elements
//! each) carry the external inductance and the high-frequency capacitance – the delay –, and the
//! fitted loss networks are lumped once per cell instead of in every section. The simulators' own
//! line elements were tried for the lossless part and rejected (`docs/research/vector-fitting.md`):
//! ngspice's `T` stalls in a cascade of short lines, its LTRA bounds the time step to the shortest
//! line delay and went unstable in a lane with long cells, and Xyce runs neither in `.AC`.

use super::fit::{Debye, Foster};
use crate::lanenet::M4;
use crate::num::Cx;
use crate::rf::{coupled, sep, Abcd};
use std::collections::HashMap;
use std::f64::consts::PI;

/// One element of a series chain.
#[derive(Debug, Clone, PartialEq)]
pub enum Zel {
    R(f64),
    L(f64),
    C(f64),
    /// R ∥ L (a Foster term)
    RpL(f64, f64),
    /// R ∥ L ∥ C (a resonance above the band)
    Tank(f64, f64, f64),
}

/// Series impedance: elements in series.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Imp(pub Vec<Zel>);

impl Imp {
    pub fn z(&self, w: f64) -> Cx {
        let mut z = Cx::new(0.0, 0.0);
        for e in &self.0 {
            z = z + match *e {
                Zel::R(r) => Cx::new(r, 0.0),
                Zel::L(l) => Cx::new(0.0, w * l),
                Zel::C(c) => Cx::new(0.0, -1.0 / (w * c)),
                Zel::RpL(r, l) => Cx::new(0.0, w * l * r) / Cx::new(r, w * l),
                Zel::Tank(r, l, c) => Cx::new(1.0 / r, w * c - 1.0 / (w * l)).inv(),
            };
        }
        z
    }
    pub fn from_foster(f: &Foster) -> Imp {
        let mut v = Vec::new();
        if f.r0 > 0.0 {
            v.push(Zel::R(f.r0));
        }
        if f.l > 0.0 {
            v.push(Zel::L(f.l));
        }
        v.extend(f.terms.iter().map(|&(r, l)| Zel::RpL(r, l)));
        v.extend(f.tanks.iter().map(|&(r, l, c)| Zel::Tank(r, l, c)));
        Imp(v)
    }
    /// Impedance multiplied by k.
    pub fn scaled(&self, k: f64) -> Imp {
        Imp(self
            .0
            .iter()
            .map(|e| match *e {
                Zel::R(r) => Zel::R(r * k),
                Zel::L(l) => Zel::L(l * k),
                Zel::C(c) => Zel::C(c / k),
                Zel::RpL(r, l) => Zel::RpL(r * k, l * k),
                Zel::Tank(r, l, c) => Zel::Tank(r * k, l * k, c / k),
            })
            .collect())
    }
    fn has_c(&self) -> bool {
        self.0.iter().any(|e| matches!(e, Zel::C(_)))
    }
}

/// Shunt admittance: branches in parallel, each an `Imp` to the reference.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Adm(pub Vec<Imp>);

impl Adm {
    pub fn y(&self, w: f64) -> Cx {
        self.0.iter().fold(Cx::new(0.0, 0.0), |y, b| {
            let z = b.z(w);
            if z.abs() > 0.0 {
                y + z.inv()
            } else {
                y + Cx::new(1e12, 0.0)
            }
        })
    }
    pub fn from_debye(d: &Debye) -> Adm {
        let mut v = Vec::new();
        if d.g0 > 0.0 {
            v.push(Imp(vec![Zel::R(1.0 / d.g0)]));
        }
        if d.c > 0.0 {
            v.push(Imp(vec![Zel::C(d.c)]));
        }
        v.extend(d.terms.iter().map(|&(r, c)| Imp(vec![Zel::R(r), Zel::C(c)])));
        v.extend(d.res.iter().map(|&(r, l, c)| Imp(vec![Zel::R(r), Zel::L(l), Zel::C(c)])));
        Adm(v)
    }
    /// Admittance multiplied by k.
    pub fn scaled(&self, k: f64) -> Adm {
        Adm(self.0.iter().map(|b| b.scaled(1.0 / k)).collect())
    }
}

/// A two-port element of one line.
#[derive(Debug, Clone)]
pub enum El {
    Series(Imp),
    Shunt(Adm),
    /// RLGC ladder of `n` sections `z`, `y`: y/2 – (z – y) × (n−1) – z – y/2 (π sections)
    Ladder { z: Imp, y: Adm, n: usize },
    /// open stub to the reference
    Stub(Chain),
    /// lossless-line cells: `n` cells of a lossless line (inductance `l`, capacitance `c` per cell,
    /// written as an LC ladder of `m` π sections) with the series loss `z` and the shunt loss `y` of
    /// a cell in its middle: line/2 – (z – y – line) × (n−1) – z – y – line/2 (the half lines with
    /// ⌈m/2⌉ sections)
    Cells { z: Imp, y: Adm, l: f64, c: f64, n: usize, m: usize },
    /// a named sub-chain: its own `.subckt` (a line segment, a via, a part)
    Block { name: String, note: String, chain: Chain, tag: Tag },
}

/// What a lane element is, for the KiCad sheet (empty for inner blocks).
#[derive(Debug, Clone, Default)]
pub struct Tag {
    /// "line", "coupled", "via", "viapair", "part", "pad", "stub", "stubpair" ("" inner block)
    pub kind: &'static str,
    /// the part's reference designator on the board (parts and pads)
    pub refdes: String,
    /// readable value: length, layer and Z0 of a line, the part's value, the via's layers
    pub value: String,
    /// a part as built: kind ('R', 'C', 'L'), series, R, L, C (parasitics included)
    pub part: Option<(char, bool, f64, f64, Option<f64>)>,
    /// lines and coupled runs: length (mm), range of Z0 (a coupled run: differential), layer
    pub len_mm: f64,
    pub z: (f64, f64),
    pub layer: String,
}

impl Tag {
    /// A line section or a coupled run.
    pub fn line(kind: &'static str, len_mm: f64, z: f64, layer: &str) -> Tag {
        let mut t = Tag { kind, len_mm, z: (z, z), layer: layer.to_string(), ..Tag::default() };
        t.value = t.line_value();
        t
    }
    fn line_value(&self) -> String {
        let z = if self.z.1 - self.z.0 < 0.5 { format!("{:.1}", self.z.0) } else { format!("{:.1}-{:.1}", self.z.0, self.z.1) };
        format!("{:.2} mm {} {z} Ω{}", self.len_mm, self.layer, if self.kind == "coupled" { " diff." } else { "" })
    }
}

/// Join two consecutive blocks of the same kind ("line" or "coupled") on the same layer into one
/// (the same circuit, one element on the KiCad sheet).
pub fn join_blocks(a: &mut El, b: &El) -> bool {
    if let (El::Block { note, chain, tag, .. }, El::Block { note: nb, chain: cb, tag: tb, .. }) = (a, b) {
        if tag.kind == "line" && tb.kind == "line" && tag.layer == tb.layer {
            chain.0.extend(cb.0.iter().cloned());
            note.push('\n');
            note.push_str(nb);
            tag.len_mm += tb.len_mm;
            tag.z = (tag.z.0.min(tb.z.0), tag.z.1.max(tb.z.1));
            tag.value = tag.line_value();
            return true;
        }
    }
    false
}

/// Join a coupled run into the previous modal block when both are coupled runs on one layer.
pub fn join_modal(a: &mut PairEl, b: &PairEl) -> bool {
    if let (PairEl::Modal { note, even, odd, tag, .. }, PairEl::Modal { note: nb, even: eb, odd: ob, tag: tb, .. }) = (a, b) {
        if tag.kind == "coupled" && tb.kind == "coupled" && tag.layer == tb.layer {
            even.0.extend(eb.0.iter().cloned());
            odd.0.extend(ob.0.iter().cloned());
            note.push('\n');
            note.push_str(nb);
            tag.len_mm += tb.len_mm;
            tag.z = (tag.z.0.min(tb.z.0), tag.z.1.max(tb.z.1));
            tag.value = tag.line_value();
            return true;
        }
    }
    false
}

/// Elements in a row from port a to port b.
#[derive(Debug, Clone, Default)]
pub struct Chain(pub Vec<El>);

impl Chain {
    pub fn abcd(&self, w: f64) -> Abcd {
        let mut m = Abcd::identity();
        for e in &self.0 {
            m = m.mul(&e.abcd(w));
        }
        m
    }
    /// Only shunt elements (no series path: both ports are one node).
    pub fn shunt_only(&self) -> bool {
        self.0.iter().all(|e| match e {
            El::Shunt(_) | El::Stub(_) => true,
            El::Block { chain, .. } => chain.shunt_only(),
            _ => false,
        })
    }
    /// The chain with every impedance multiplied by k.
    pub fn scaled(&self, k: f64) -> Chain {
        Chain(self
            .0
            .iter()
            .map(|e| match e {
                El::Series(z) => El::Series(z.scaled(k)),
                El::Shunt(y) => El::Shunt(y.scaled(1.0 / k)),
                El::Ladder { z, y, n } => El::Ladder { z: z.scaled(k), y: y.scaled(1.0 / k), n: *n },
                El::Cells { z, y, l, c, n, m } => El::Cells { z: z.scaled(k), y: y.scaled(1.0 / k), l: l * k, c: c / k, n: *n, m: *m },
                El::Stub(c) => El::Stub(c.scaled(k)),
                El::Block { name, note, chain, tag } => El::Block { name: name.clone(), note: note.clone(), chain: chain.scaled(k), tag: tag.clone() },
            })
            .collect())
    }
}

impl El {
    pub fn abcd(&self, w: f64) -> Abcd {
        match self {
            El::Series(z) => Abcd::series(z.z(w)),
            El::Shunt(y) => Abcd::shunt(y.y(w)),
            El::Ladder { z, y, n } => {
                let (zz, yy) = (z.z(w), y.y(w));
                let half = Abcd::shunt(yy.scale(0.5));
                let sec = Abcd::series(zz).mul(&Abcd::shunt(yy));
                let mut m = half;
                for _ in 1..*n {
                    m = m.mul(&sec);
                }
                m.mul(&Abcd::series(zz)).mul(&half)
            }
            El::Cells { z, y, l, c, n, m } => {
                let (full, half) = cell_lines(*l, *c, *m);
                let lump = Abcd::series(z.z(w)).mul(&Abcd::shunt(y.y(w)));
                let (lf, lh) = (full.abcd(w), half.abcd(w));
                let mut t = lh;
                for k in 0..*n {
                    t = t.mul(&lump).mul(if k + 1 == *n { &lh } else { &lf });
                }
                t
            }
            El::Stub(c) => {
                let m = c.abcd(w);
                Abcd::shunt(m.c / m.a)
            }
            El::Block { chain, .. } => chain.abcd(w),
        }
    }
}

// --- scattering matrices (the passivity check beyond the band) --------------------------------
//
// Cascade matrices of long ladders overflow far above their cut-off frequency (the growth of an
// evanescent wave over hundreds of sections); scattering matrices combined by star products stay
// bounded at every frequency.

/// A 2-port's S-matrix (S11, S12, S21, S22), reference z0 at both ports.
pub type S2 = [Cx; 4];
/// A pair's 4-port S-matrix, ports P near, N near, P far, N far.
pub type S4 = [[Cx; 4]; 4];

fn s2_of(m: &Abcd, z0: f64) -> S2 {
    let (s11, s21, s22) = m.s_params(z0);
    [s11, s21, s21, s22]
}

/// Star product: the 2-port `a` followed by `b`.
pub fn star2(a: &S2, b: &S2) -> S2 {
    let den = (Cx::new(1.0, 0.0) - a[3] * b[0]).inv();
    [a[0] + a[1] * b[0] * a[2] * den, a[1] * b[1] * den, a[2] * b[2] * den, b[3] + b[2] * a[3] * b[1] * den]
}

const S2_THRU: S2 = [Cx { re: 0.0, im: 0.0 }, Cx { re: 1.0, im: 0.0 }, Cx { re: 1.0, im: 0.0 }, Cx { re: 0.0, im: 0.0 }];

/// `s` repeated `n` times (star products by binary exponentiation).
fn star2_pow(s: &S2, mut n: usize) -> S2 {
    let (mut acc, mut base) = (S2_THRU, *s);
    while n > 0 {
        if n & 1 == 1 {
            acc = star2(&acc, &base);
        }
        base = star2(&base, &base);
        n >>= 1;
    }
    acc
}

impl Chain {
    /// The chain's S-matrix at ω (star products: bounded at every frequency).
    pub fn s(&self, w: f64, z0: f64) -> S2 {
        self.0.iter().fold(S2_THRU, |acc, e| star2(&acc, &e.s(w, z0)))
    }
}

impl El {
    /// The element's S-matrix at ω, reference z0.
    pub fn s(&self, w: f64, z0: f64) -> S2 {
        match self {
            El::Series(z) => s2_of(&Abcd::series(z.z(w)), z0),
            El::Shunt(y) => s2_of(&Abcd::shunt(y.y(w)), z0),
            El::Ladder { z, y, n } => {
                let (zz, yy) = (z.z(w), y.y(w));
                let half = s2_of(&Abcd::shunt(yy.scale(0.5)), z0);
                let sec = s2_of(&Abcd::series(zz).mul(&Abcd::shunt(yy)), z0);
                let m = star2(&half, &star2_pow(&sec, n - 1));
                star2(&star2(&m, &s2_of(&Abcd::series(zz), z0)), &half)
            }
            El::Cells { z, y, l, c, n, m } => {
                let (full, half) = cell_lines(*l, *c, *m);
                let (lf, lh) = (full.s(w, z0), half.s(w, z0));
                let lump = s2_of(&Abcd::series(z.z(w)).mul(&Abcd::shunt(y.y(w))), z0);
                let cell = star2(&lump, &lf);
                star2(&star2(&star2(&lh, &star2_pow(&cell, n - 1)), &lump), &lh)
            }
            El::Stub(c) => {
                // the open chain's input reflection Γ, then a shunt of y·z0 = (1 − Γ)/(1 + Γ)
                let m = c.s(w, z0);
                let one = Cx::new(1.0, 0.0);
                let g = m[0] + m[1] * m[2] * (one - m[3]).inv();
                let d = (Cx::new(3.0, 0.0) + g).inv();
                let s21 = (one + g).scale(2.0) * d;
                let s11 = Cx::new(0.0, 0.0) - (one - g) * d;
                [s11, s21, s21, s11]
            }
            El::Block { chain, .. } => chain.s(w, z0),
        }
    }
}

type M2 = [[Cx; 2]; 2];

fn m2(a: Cx, b: Cx, c: Cx, d: Cx) -> M2 {
    [[a, b], [c, d]]
}
fn m2_mul(a: &M2, b: &M2) -> M2 {
    let mut c = [[Cx::new(0.0, 0.0); 2]; 2];
    for i in 0..2 {
        for j in 0..2 {
            c[i][j] = a[i][0] * b[0][j] + a[i][1] * b[1][j];
        }
    }
    c
}
fn m2_add(a: &M2, b: &M2) -> M2 {
    [[a[0][0] + b[0][0], a[0][1] + b[0][1]], [a[1][0] + b[1][0], a[1][1] + b[1][1]]]
}
/// (1 − a)⁻¹
fn m2_one_minus_inv(a: &M2) -> M2 {
    let one = Cx::new(1.0, 0.0);
    let (p, q, r, t) = (one - a[0][0], Cx::new(0.0, 0.0) - a[0][1], Cx::new(0.0, 0.0) - a[1][0], one - a[1][1]);
    let det = (p * t - q * r).inv();
    m2(t * det, Cx::new(0.0, 0.0) - q * det, Cx::new(0.0, 0.0) - r * det, p * det)
}

/// Star product of two 4-ports (near ports 0, 1 and far ports 2, 3).
fn star4(a: &S4, b: &S4) -> S4 {
    let blk = |s: &S4, r: usize, c: usize| -> M2 { [[s[r][c], s[r][c + 1]], [s[r + 1][c], s[r + 1][c + 1]]] };
    let (a11, a12, a21, a22) = (blk(a, 0, 0), blk(a, 0, 2), blk(a, 2, 0), blk(a, 2, 2));
    let (b11, b12, b21, b22) = (blk(b, 0, 0), blk(b, 0, 2), blk(b, 2, 0), blk(b, 2, 2));
    let k1 = m2_one_minus_inv(&m2_mul(&b11, &a22)); // (1 − B11 A22)⁻¹
    let k2 = m2_one_minus_inv(&m2_mul(&a22, &b11)); // (1 − A22 B11)⁻¹
    let s11 = m2_add(&a11, &m2_mul(&m2_mul(&m2_mul(&a12, &k1), &b11), &a21));
    let s12 = m2_mul(&m2_mul(&a12, &k1), &b12);
    let s21 = m2_mul(&m2_mul(&b21, &k2), &a21);
    let s22 = m2_add(&b22, &m2_mul(&m2_mul(&m2_mul(&b21, &k2), &a22), &b12));
    let mut s = [[Cx::new(0.0, 0.0); 4]; 4];
    for i in 0..2 {
        for j in 0..2 {
            s[i][j] = s11[i][j];
            s[i][j + 2] = s12[i][j];
            s[i + 2][j] = s21[i][j];
            s[i + 2][j + 2] = s22[i][j];
        }
    }
    s
}

/// S-matrix (ports P near, N near, P far, N far; z0 per line) of a pair's elements at ω, by star
/// products – bounded at every frequency, unlike `pair_t`'s cascade matrices.
pub fn pair_s(els: &[PairEl], w: f64, z0: f64) -> S4 {
    let zero = Cx::new(0.0, 0.0);
    let mut acc = [[zero; 4]; 4];
    for i in 0..2 {
        acc[i][i + 2] = Cx::new(1.0, 0.0);
        acc[i + 2][i] = Cx::new(1.0, 0.0);
    }
    let thru = acc;
    for e in els {
        let m = match e {
            PairEl::Line(l, el) => {
                let (l, o) = ((*l).min(1), 1 - (*l).min(1));
                let s = el.s(w, z0);
                let mut m = thru;
                m[o][o + 2] = Cx::new(1.0, 0.0);
                m[l][l] = s[0];
                m[l][l + 2] = s[1];
                m[l + 2][l] = s[2];
                m[l + 2][l + 2] = s[3];
                m
            }
            PairEl::Modal { even, odd, .. } => {
                // V_P = V_e + V_o, V_N = V_e − V_o with the same reference per line in both modes:
                // S[x_i, y_j] = (Se_ij + σx σy So_ij)/2, σ = +1 for P, −1 for N
                let (se, so) = (even.s(w, z0), odd.s(w, z0));
                let mut m = [[zero; 4]; 4];
                for (r, &(ri, rs)) in [(0usize, 1.0), (0, -1.0), (1, 1.0), (1, -1.0)].iter().enumerate() {
                    for (c, &(ci, cs)) in [(0usize, 1.0), (0, -1.0), (1, 1.0), (1, -1.0)].iter().enumerate() {
                        let k = ri * 2 + ci;
                        m[r][c] = (se[k] + so[k].scale(rs * cs)).scale(0.5);
                    }
                }
                m
            }
        };
        acc = star4(&acc, &m);
    }
    acc
}

/// The lossless lines of a cell (inductance `l`, capacitance `c`, `m` π sections) and of half a
/// cell (⌈m/2⌉ sections) as LC ladders.
pub fn cell_lines(l: f64, c: f64, m: usize) -> (El, El) {
    let lad = |l: f64, c: f64, k: usize| El::Ladder { z: Imp(vec![Zel::L(l / k as f64)]), y: Adm(vec![Imp(vec![Zel::C(c / k as f64)])]), n: k };
    let mh = m.div_ceil(2).max(1);
    (lad(l, c, m.max(1)), lad(l / 2.0, c / 2.0, mh))
}

/// Cascade matrix of a lossless line of impedance `z` and electrical length `theta` (rad).
pub fn lossless(z: f64, theta: f64) -> Abcd {
    let (s, c) = theta.sin_cos();
    Abcd { a: Cx::new(c, 0.0), b: Cx::new(0.0, z * s), c: Cx::new(0.0, s / z), d: Cx::new(c, 0.0) }
}

/// An element of a pair: on line 0 (P) or 1 (N), or a coupled block in modal form.
#[derive(Debug, Clone)]
pub enum PairEl {
    Line(usize, El),
    Modal { name: String, note: String, even: Chain, odd: Chain, tag: Tag },
}

fn m4_mul(a: &M4, b: &M4) -> M4 {
    let mut c = [[Cx::new(0.0, 0.0); 4]; 4];
    for i in 0..4 {
        for j in 0..4 {
            let mut s = Cx::new(0.0, 0.0);
            for k in 0..4 {
                s = s + a[i][k] * b[k][j];
            }
            c[i][j] = s;
        }
    }
    c
}

/// Cascade matrix (state V_P V_N I_P I_N) of a pair's elements at ω.
pub fn pair_t(els: &[PairEl], w: f64) -> M4 {
    let id = Abcd::identity();
    let mut t = sep(&id, &id);
    for e in els {
        let m = match e {
            PairEl::Line(0, el) => sep(&el.abcd(w), &id),
            PairEl::Line(_, el) => sep(&id, &el.abcd(w)),
            PairEl::Modal { even, odd, .. } => coupled(&even.abcd(w), &odd.abcd(w)),
        };
        t = m4_mul(&t, &m);
    }
    t
}

// --- SPICE text ------------------------------------------------------------------------------

/// Number of each kind of element written.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct Counts {
    pub r: usize,
    pub l: usize,
    pub c: usize,
    /// controlled sources and 0 V current probes of the mode converters, shorts
    pub sources: usize,
    /// subcircuit instances
    pub x: usize,
    pub subckts: usize,
    /// RLGC ladder sections
    pub sections: usize,
    /// lossless-line cells of the compact model
    pub cells: usize,
}

impl Counts {
    pub fn elements(&self) -> usize {
        self.r + self.l + self.c + self.sources
    }
    /// Elements after flattening every instance (what the simulator solves).
    pub fn add(&mut self, o: &Counts, times: usize) {
        self.r += o.r * times;
        self.l += o.l * times;
        self.c += o.c * times;
        self.sources += o.sources * times;
        self.sections += o.sections * times;
        self.cells += o.cells * times;
    }
}

/// Writes subcircuits; names are unique within one library.
pub struct Lib {
    pub prefix: String,
    out: Vec<String>,
    names: HashMap<String, usize>,
    /// flattened element counts of every written subcircuit
    flat: HashMap<String, Counts>,
    pub counts: Counts,
    /// per written subcircuit: its ports, its subcircuit instances (name, nodes) and whether it
    /// holds other elements too
    pub defs: HashMap<String, Def>,
    /// tags of the subcircuits written for tagged blocks
    pub tags: HashMap<String, Tag>,
}

/// A written subcircuit's interface and instances.
#[derive(Debug, Clone, Default)]
pub struct Def {
    pub ports: Vec<String>,
    pub insts: Vec<(String, Vec<String>)>,
    /// elements other than subcircuit instances (R, L, C, sources)
    pub other: usize,
}

/// One subcircuit body being written.
struct Body {
    insts: Vec<(String, Vec<String>)>,
    lines: Vec<String>,
    n: HashMap<char, usize>,
    node: usize,
    flat: Counts,
}

impl Body {
    fn new() -> Body {
        Body { insts: vec![], lines: vec![], n: HashMap::new(), node: 0, flat: Counts::default() }
    }
    fn name(&mut self, k: char) -> String {
        let c = self.n.entry(k).or_insert(0);
        *c += 1;
        format!("{k}{c}")
    }
    fn node(&mut self) -> String {
        self.node += 1;
        format!("n{}", self.node)
    }
    fn el(&mut self, k: char, a: &str, b: &str, v: f64) {
        let nm = self.name(k);
        self.lines.push(format!("{nm} {a} {b} {}", num(v)));
        match k {
            'R' => self.flat.r += 1,
            'L' => self.flat.l += 1,
            _ => self.flat.c += 1,
        }
    }
    /// A series chain from a to b (intermediate nodes as needed); an empty chain is a short.
    fn imp(&mut self, z: &Imp, a: &str, b: &str) {
        self.imp_dc(z, a, b, true)
    }
    /// `dc_path`: a series capacitor gets 1 GΩ across the chain (not needed for shunt branches).
    fn imp_dc(&mut self, z: &Imp, a: &str, b: &str, dc_path: bool) {
        let parts: Vec<&Zel> = z.0.iter().filter(|e| !matches!(e, Zel::R(r) if *r == 0.0)).collect();
        if parts.is_empty() {
            self.short(a, b);
            return;
        }
        let mut cur = a.to_string();
        for (i, e) in parts.iter().enumerate() {
            let nxt = if i + 1 == parts.len() { b.to_string() } else { self.node() };
            match **e {
                Zel::R(r) => self.el('R', &cur, &nxt, r),
                Zel::L(l) => self.el('L', &cur, &nxt, l),
                Zel::C(c) => self.el('C', &cur, &nxt, c),
                Zel::RpL(r, l) => {
                    self.el('R', &cur, &nxt, r);
                    self.el('L', &cur, &nxt, l);
                }
                Zel::Tank(r, l, c) => {
                    self.el('R', &cur, &nxt, r);
                    self.el('L', &cur, &nxt, l);
                    self.el('C', &cur, &nxt, c);
                }
            }
            cur = nxt;
        }
        // a series capacitor leaves the far side without a DC path when nothing else gives one:
        // 1 GΩ across the chain (negligible in the signal band)
        if dc_path && z.has_c() {
            self.el('R', a, b, 1e9);
        }
    }
    fn adm(&mut self, y: &Adm, a: &str, r: &str) {
        for b in &y.0 {
            if b.0.is_empty() {
                continue;
            }
            self.imp_dc(b, a, r, false);
        }
    }
    fn short(&mut self, a: &str, b: &str) {
        let nm = self.name('V');
        self.lines.push(format!("{nm} {a} {b} 0"));
        self.flat.sources += 1;
    }
    /// A lossless line from a to b as an LC ladder of `k` π sections (inductance `l`, capacitance
    /// `c` in all): c/2k – l/k – c/k – … – l/k – c/2k.
    fn lc_line(&mut self, a: &str, b: &str, l: f64, c: f64, k: usize) {
        let k = k.max(1);
        let (ls, cs) = (l / k as f64, c / k as f64);
        self.el('C', a, "ref", cs / 2.0);
        let mut cur = a.to_string();
        for i in 0..k {
            let nxt = if i + 1 == k { b.to_string() } else { self.node() };
            self.el('L', &cur, &nxt, ls);
            self.el('C', &nxt, "ref", if i + 1 == k { cs / 2.0 } else { cs });
            cur = nxt;
        }
    }
}

/// A number in SPICE notation (7 significant digits).
pub fn num(v: f64) -> String {
    format!("{v:.6e}")
}

fn sanitize(s: &str) -> String {
    let t: String = s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' }).collect();
    let t = t.trim_matches('_').to_string();
    if t.is_empty() {
        "x".into()
    } else if t.chars().next().unwrap().is_ascii_digit() {
        format!("n{t}")
    } else {
        t
    }
}

impl Lib {
    pub fn new(prefix: &str) -> Lib {
        Lib { prefix: sanitize(prefix), out: vec![], names: HashMap::new(), flat: HashMap::new(), counts: Counts::default(), defs: HashMap::new(), tags: HashMap::new() }
    }

    /// A unique subcircuit name from `hint` (with the library's prefix once).
    pub fn unique(&mut self, hint: &str) -> String {
        let pre = format!("{}_", self.prefix);
        let hint = hint.strip_prefix(&pre).unwrap_or(hint);
        let base = format!("{}_{}", self.prefix, sanitize(hint));
        let n = self.names.entry(base.to_lowercase()).or_insert(0);
        *n += 1;
        if *n == 1 {
            base
        } else {
            format!("{base}_{n}")
        }
    }

    fn finish(&mut self, name: &str, ports: &str, note: &str, b: Body) {
        let mut s = String::new();
        if !note.is_empty() {
            for l in note.lines() {
                s.push_str(&format!("* {l}\n"));
            }
        }
        s.push_str(&format!(".subckt {name} {ports}\n"));
        for l in &b.lines {
            s.push_str(l);
            s.push('\n');
        }
        s.push_str(&format!(".ends {name}\n"));
        self.out.push(s);
        self.counts.subckts += 1;
        self.flat.insert(name.to_string(), b.flat);
        let other = b.lines.len() - b.insts.len();
        self.defs.insert(name.to_string(), Def { ports: ports.split_whitespace().map(String::from).collect(), insts: b.insts, other });
    }

    fn inst(&mut self, b: &mut Body, sub: &str, nodes: &[&str]) {
        let nm = b.name('X');
        b.lines.push(format!("{nm} {} {sub}", nodes.join(" ")));
        b.insts.push((sub.to_string(), nodes.iter().map(|n| n.to_string()).collect()));
        self.counts.x += 1;
        let f = self.flat.get(sub).cloned().unwrap_or_default();
        b.flat.add(&f, 1);
    }

    /// Write `chain` as a subcircuit: `name a b ref`, or `name a ref` when it has shunt elements
    /// only (both ends are one node).
    pub fn chain_subckt(&mut self, name: &str, note: &str, chain: &Chain) {
        let mut b = Body::new();
        if chain.shunt_only() {
            self.emit(&mut b, chain, "a", "a", name);
            self.finish(name, "a ref", note, b);
        } else {
            self.emit(&mut b, chain, "a", "b", name);
            self.finish(name, "a b ref", note, b);
        }
    }

    /// Instance of a chain subcircuit between `a` and `z` (one-port chains at `a`).
    fn inst_chain(&mut self, b: &mut Body, sub: &str, chain: &Chain, a: &str, z: &str) {
        if chain.shunt_only() {
            self.inst(b, sub, &[a, "ref"]);
        } else {
            self.inst(b, sub, &[a, z, "ref"]);
        }
    }

    /// Elements of `chain` from node `a` to node `z` into `body` (a shunt-only chain needs
    /// `z == a`, else a short joins them).
    fn emit(&mut self, b: &mut Body, chain: &Chain, a: &str, z: &str, hint: &str) {
        // the last element that moves along the line ends at `z`
        let last = chain.0.iter().rposition(|e| match e {
            El::Series(_) | El::Ladder { .. } | El::Cells { .. } => true,
            El::Block { chain, .. } => !chain.shunt_only(),
            _ => false,
        });
        let mut cur = a.to_string();
        if last.is_none() && a != z {
            b.short(a, z);
        }
        for (i, e) in chain.0.iter().enumerate() {
            match e {
                El::Shunt(y) => b.adm(y, &cur, "ref"),
                El::Stub(c) => {
                    let nm = self.unique(&format!("{hint}_stub"));
                    self.chain_subckt(&nm, "open stub", c);
                    let end = b.node();
                    self.inst_chain(b, &nm, c, &cur, &end);
                }
                El::Series(zz) => {
                    let nxt = if Some(i) == last { z.to_string() } else { b.node() };
                    b.imp(zz, &cur, &nxt);
                    cur = nxt;
                }
                El::Ladder { z: zs, y, n } => {
                    let nxt = if Some(i) == last { z.to_string() } else { b.node() };
                    let sec = self.unique(&format!("{hint}_sec"));
                    let end = self.unique(&format!("{hint}_end"));
                    for (nm, k) in [(&sec, 1.0), (&end, 0.5)] {
                        if k == 1.0 && *n < 2 {
                            continue;
                        }
                        let mut sb = Body::new();
                        sb.imp(zs, "a", "b");
                        sb.adm(&y.scaled(k), "b", "ref");
                        self.finish(nm, "a b ref", if k == 1.0 { "one RLGC section: z, then y" } else { "last RLGC section: z, then y/2" }, sb);
                    }
                    b.adm(&y.scaled(0.5), &cur, "ref");
                    for s in 0..*n {
                        let to = if s + 1 == *n { nxt.clone() } else { b.node() };
                        let sub = if s + 1 == *n { end.clone() } else { sec.clone() };
                        self.inst(b, &sub, &[&cur, &to, "ref"]);
                        cur = to;
                    }
                    self.counts.sections += n;
                    b.flat.sections += n;
                }
                El::Cells { z: zs, y, l, c, n, m } => {
                    let nxt = if Some(i) == last { z.to_string() } else { b.node() };
                    // three subcircuits: half a lossless line, a cell (z, y, line) and the last cell
                    // (z, y, half a line); the lines as LC ladders
                    let head = self.unique(&format!("{hint}_head"));
                    let cell = self.unique(&format!("{hint}_cell"));
                    let end = self.unique(&format!("{hint}_end"));
                    let mh = m.div_ceil(2).max(1);
                    let mut hb = Body::new();
                    hb.lc_line("a", "b", l / 2.0, c / 2.0, mh);
                    self.finish(&head, "a b ref", "half a lossless line (LC ladder)", hb);
                    for (nm, full) in [(&cell, true), (&end, false)] {
                        if full && *n < 2 {
                            continue;
                        }
                        let mut cb = Body::new();
                        cb.imp(zs, "a", "x");
                        cb.adm(y, "x", "ref");
                        if full {
                            cb.lc_line("x", "b", *l, *c, *m);
                        } else {
                            cb.lc_line("x", "b", l / 2.0, c / 2.0, mh);
                        }
                        cb.flat.cells += 1;
                        self.finish(nm, "a b ref", if full { "one lossless-line cell: series loss z, shunt loss y, then the lossless line (LC ladder)" } else { "last cell: z, y, then half the lossless line" }, cb);
                    }
                    let first = b.node();
                    self.inst(b, &head, &[&cur, &first, "ref"]);
                    cur = first;
                    for k in 0..*n {
                        let to = if k + 1 == *n { nxt.clone() } else { b.node() };
                        let sub = if k + 1 == *n { end.clone() } else { cell.clone() };
                        self.inst(b, &sub, &[&cur, &to, "ref"]);
                        cur = to;
                    }
                    self.counts.cells += n;
                }
                El::Block { name, note, chain, tag } => {
                    let nm = self.unique(name);
                    if !tag.kind.is_empty() {
                        self.tags.insert(nm.clone(), tag.clone());
                    }
                    self.chain_subckt(&nm, note, chain);
                    if chain.shunt_only() {
                        self.inst(b, &nm, &[&cur, "ref"]);
                    } else {
                        let nxt = if Some(i) == last { z.to_string() } else { b.node() };
                        self.inst(b, &nm, &[&cur, &nxt, "ref"]);
                        cur = nxt;
                    }
                }
            }
        }
    }

    /// Write a pair as subcircuit `name ap an bp bn ref`.
    pub fn pair_subckt(&mut self, name: &str, note: &str, els: &[PairEl]) {
        let mut b = Body::new();
        let moves = |e: &PairEl, line: usize| match e {
            PairEl::Line(l, el) => *l == line && !Chain(vec![el.clone()]).shunt_only(),
            PairEl::Modal { even, odd, .. } => !(even.shunt_only() && odd.shunt_only()),
        };
        let last = [els.iter().rposition(|e| moves(e, 0)), els.iter().rposition(|e| moves(e, 1))];
        let ports = [["ap", "bp"], ["an", "bn"]];
        let mut cur = [ports[0][0].to_string(), ports[1][0].to_string()];
        for l in 0..2 {
            if last[l].is_none() {
                b.short(ports[l][0], ports[l][1]);
            }
        }
        for (i, e) in els.iter().enumerate() {
            match e {
                PairEl::Line(l, el) => {
                    let l = (*l).min(1);
                    let shunt = Chain(vec![el.clone()]).shunt_only();
                    let nxt = if shunt { cur[l].clone() } else if Some(i) == last[l] { ports[l][1].to_string() } else { b.node() };
                    let hint = format!("{}_{}", if l == 0 { "p" } else { "n" }, i);
                    let from = cur[l].clone();
                    self.emit(&mut b, &Chain(vec![el.clone()]), &from, &nxt, &hint);
                    cur[l] = nxt;
                }
                PairEl::Modal { name, note, even, odd, tag } => {
                    let shunt = even.shunt_only() && odd.shunt_only();
                    let nm = self.unique(name);
                    if !tag.kind.is_empty() {
                        self.tags.insert(nm.clone(), tag.clone());
                    }
                    let ne = self.unique(&format!("{name}_even"));
                    let no = self.unique(&format!("{name}_odd"));
                    self.chain_subckt(&ne, "even mode, per line", even);
                    self.chain_subckt(&no, "odd mode, per line", odd);
                    let mut mb = Body::new();
                    // near end: modal voltages from the line voltages, line currents from the modal currents
                    converter(&mut mb, "p1", "n1", "e1", "o1", true);
                    if shunt {
                        self.inst(&mut mb, &ne, &["e1", "ref"]);
                        self.inst(&mut mb, &no, &["o1", "ref"]);
                        self.finish(&nm, "p1 n1 ref", &format!("{note}\ncoupled shunt block in modal form: V_e = (V_P + V_N)/2, V_o = (V_P − V_N)/2, I_P = I_e + I_o, I_N = I_e − I_o"), mb);
                        let (cp, cn) = (cur[0].clone(), cur[1].clone());
                        self.inst(&mut b, &nm, &[&cp, &cn, "ref"]);
                    } else {
                        self.inst(&mut mb, &ne, &["e1", "e2", "ref"]);
                        self.inst(&mut mb, &no, &["o1", "o2", "ref"]);
                        converter(&mut mb, "p2", "n2", "e2", "o2", false);
                        self.finish(&nm, "p1 n1 p2 n2 ref", &format!("{note}\ncoupled block in modal form: V_e = (V_P + V_N)/2, V_o = (V_P − V_N)/2, I_P = I_e + I_o, I_N = I_e − I_o"), mb);
                        let np = if Some(i) == last[0] { ports[0][1].to_string() } else { b.node() };
                        let nn = if Some(i) == last[1] { ports[1][1].to_string() } else { b.node() };
                        self.inst(&mut b, &nm, &[&cur[0], &cur[1], &np, &nn, "ref"]);
                        cur = [np, nn];
                    }
                }
            }
        }
        self.finish(name, "ap an bp bn ref", note, b);
    }

    pub fn text(&self) -> String {
        self.out.join("\n")
    }

    /// Flattened element counts of subcircuit `name`.
    pub fn flat_of(&self, name: &str) -> Counts {
        let mut c = self.flat.get(name).cloned().unwrap_or_default();
        c.subckts = self.counts.subckts;
        c.x = self.counts.x;
        c
    }
}

/// Mode converter at one end of a modal block (V_e = (V_P + V_N)/2, V_o = (V_P − V_N)/2,
/// I_P = I_e + I_o, I_N = I_e − I_o). `near`: the modal chains start here – the modal voltages are set
/// from the line voltages (E sources, 0 V probes into the chains) and the line nodes draw the modal
/// currents (F sources). Far end (`near` false), the dual: the line voltages are set from the modal
/// voltages (V_P = V_e + V_o, V_N = V_e − V_o) and the chains' ends deliver the line currents
/// (I_e = (I_P + I_N)/2, I_o = (I_P − I_N)/2). So every node between two blocks is driven by a
/// voltage-defined element and has a DC path (LTspice reports nodes fed by current sources alone as
/// floating); the relation is the same at both ends.
fn converter(b: &mut Body, p: &str, n: &str, e: &str, o: &str, near: bool) {
    let push = |b: &mut Body, s: String| {
        b.lines.push(s);
        b.flat.sources += 1;
    };
    let (e1, e2, e3, e4) = (b.name('E'), b.name('E'), b.name('E'), b.name('E'));
    let (v1, v2) = (b.name('V'), b.name('V'));
    let (f1, f2, f3, f4) = (b.name('F'), b.name('F'), b.name('F'), b.name('F'));
    if near {
        let (es, os) = (format!("{e}s"), format!("{o}s"));
        let (ex, ox) = (format!("{e}x"), format!("{o}x"));
        push(b, format!("{e1} {ex} ref {p} ref 0.5"));
        push(b, format!("{e2} {es} {ex} {n} ref 0.5"));
        push(b, format!("{e3} {ox} ref {p} ref 0.5"));
        push(b, format!("{e4} {os} {ox} {n} ref -0.5"));
        // probes in the direction into the modal chains; the line nodes draw I_e ± I_o
        push(b, format!("{v1} {es} {e} 0"));
        push(b, format!("{v2} {os} {o} 0"));
        push(b, format!("{f1} {p} ref {v1} 1"));
        push(b, format!("{f2} {p} ref {v2} 1"));
        push(b, format!("{f3} {n} ref {v1} 1"));
        push(b, format!("{f4} {n} ref {v2} -1"));
    } else {
        let (ps, ns) = (format!("{p}s"), format!("{n}s"));
        let (px, nx) = (format!("{p}x"), format!("{n}x"));
        push(b, format!("{e1} {px} ref {e} ref 1"));
        push(b, format!("{e2} {ps} {px} {o} ref 1"));
        push(b, format!("{e3} {nx} ref {e} ref 1"));
        push(b, format!("{e4} {ns} {nx} {o} ref -1"));
        // probes in the direction out to the line nodes; the chains' ends deliver (I_P ± I_N)/2
        push(b, format!("{v1} {ps} {p} 0"));
        push(b, format!("{v2} {ns} {n} 0"));
        push(b, format!("{f1} {e} ref {v1} 0.5"));
        push(b, format!("{f2} {e} ref {v2} 0.5"));
        push(b, format!("{f3} {o} ref {v1} 0.5"));
        push(b, format!("{f4} {o} ref {v2} -0.5"));
    }
}

/// ω of a frequency.
pub fn omega(f: f64) -> f64 {
    2.0 * PI * f
}

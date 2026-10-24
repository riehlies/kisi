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
            El::Stub(c) => {
                let m = c.abcd(w);
                Abcd::shunt(m.c / m.a)
            }
            El::Block { chain, .. } => chain.abcd(w),
        }
    }
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
            El::Series(_) | El::Ladder { .. } => true,
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

/// Mode converter at one end of a modal block. `near`: the modal chains start here (their
/// currents flow into them); else they end here.
fn converter(b: &mut Body, p: &str, n: &str, e: &str, o: &str, near: bool) {
    let (es, os) = (format!("{e}s"), format!("{o}s"));
    let (ex, ox) = (format!("{e}x"), format!("{o}x"));
    let push = |b: &mut Body, s: String| {
        b.lines.push(s);
        b.flat.sources += 1;
    };
    let (e1, e2, e3, e4) = (b.name('E'), b.name('E'), b.name('E'), b.name('E'));
    push(b, format!("{e1} {ex} ref {p} ref 0.5"));
    push(b, format!("{e2} {es} {ex} {n} ref 0.5"));
    push(b, format!("{e3} {ox} ref {p} ref 0.5"));
    push(b, format!("{e4} {os} {ox} {n} ref -0.5"));
    let (ve, vo) = (b.name('V'), b.name('V'));
    let (f1, f2, f3, f4) = (b.name('F'), b.name('F'), b.name('F'), b.name('F'));
    if near {
        // probes in the direction into the modal chains; the line nodes draw I_e ± I_o
        push(b, format!("{ve} {es} {e} 0"));
        push(b, format!("{vo} {os} {o} 0"));
        push(b, format!("{f1} {p} ref {ve} 1"));
        push(b, format!("{f2} {p} ref {vo} 1"));
        push(b, format!("{f3} {n} ref {ve} 1"));
        push(b, format!("{f4} {n} ref {vo} -1"));
    } else {
        // probes in the direction out of the modal chains; the line nodes deliver I_e ± I_o
        push(b, format!("{ve} {e} {es} 0"));
        push(b, format!("{vo} {o} {os} 0"));
        push(b, format!("{f1} ref {p} {ve} 1"));
        push(b, format!("{f2} ref {p} {vo} 1"));
        push(b, format!("{f3} ref {n} {ve} 1"));
        push(b, format!("{f4} ref {n} {vo} -1"));
    }
}

/// ω of a frequency.
pub fn omega(f: f64) -> f64 {
    2.0 * PI * f
}

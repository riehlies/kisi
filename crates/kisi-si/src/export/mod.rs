//! Export of device-to-device connections to circuit and RF simulators (ROADMAP block 3).
//!
//! A lane's board channel – the same sections the channel check computes (`rf::Build`: lines,
//! vias, parts, stubs; a pair aligned into coupled sections by `rf::mixed::align_pair`) – becomes
//! a SPICE subcircuit with ports at both ends:
//! * a **line section** is an RLGC ladder of π sections, at least `per_lambda` per wavelength at
//!   the top frequency; per section the external inductance, a Foster network for the conductor
//!   impedance R(f) + jωL_int(f) and a Debye network for the dielectric admittance G(f) + jωC(f),
//!   both fitted to kisi's line model (`rf::LineModel::rlgc`: skin effect, Huray roughness,
//!   Djordjevic-Sarkar) over the band with positive elements only (`fit`);
//! * a **via** is kisi's via model: the physics-based ladder (`via_pbm`: node capacitances,
//!   plane-pair span impedances fitted like the lines, open barrel stubs, junction inductances,
//!   the trace over the antipad) or the coaxial section stack;
//! * **parts** are their R, L, C with the estimated parasitics as kisi models them, open **stubs**
//!   their own chain of sections;
//! * a **pair** keeps both lines on their own paths; coupled sections, via pairs and stub pairs are
//!   modal blocks (even and odd chains between mode converters, `circuit::PairEl::Modal`).
//!
//! The export evaluates its own circuit (`circuit::Chain::abcd`) and reports the difference to
//! kisi's S-parameters of the same build; `validation/channel_export/` runs the netlists in ngspice
//! and compares again.

pub mod circuit;
pub mod fit;
pub mod files;
pub mod kicad;

use crate::board::Board;
use crate::extract::{Analysis, FieldResult};
use crate::lanenet::M4;
use crate::num::Cx;
use crate::rf::{Abcd, LineModel, LumpedPart, RfParams, Sections};
use crate::topology::{ComponentData, Lane};
use crate::via::{Sec, ViaModel};
use circuit::{Adm, Chain, Counts, El, Imp, Lib, PairEl, Zel};
use fit::{fit_freqs, fit_y, fit_z, Debye, Foster, Weight};
use std::collections::HashMap;
use std::f64::consts::PI;

/// Settings of the circuit export.
#[derive(Debug, Clone)]
pub struct ExportOpts {
    /// RLGC sections per wavelength at the top frequency
    pub per_lambda: f64,
    /// relative error allowed in the fits of R(f), L_int(f), G(f), C(f)
    pub fit_tol: f64,
}

impl Default for ExportOpts {
    fn default() -> Self {
        ExportOpts { per_lambda: 20.0, fit_tol: 0.01 }
    }
}

/// How well the exported circuit follows kisi's model (both on kisi's frequency grid).
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct Fidelity {
    /// worst |ΔS21| (dB) of the transmission (pairs: Sdd21) up to the top frequency and up to
    /// the profile's Nyquist frequency × 1.5
    pub max_ds21_db: f64,
    pub max_ds21_db_nyq: f64,
    /// worst complex |ΔS11| (pairs: Sdd11)
    pub max_ds11: f64,
    /// worst complex difference of any S-parameter (all ports)
    pub max_ds: f64,
    /// worst relative error of the fitted R(f), L_int(f), G(f), C(f) networks
    pub worst_fit: f64,
}

/// A lane as a circuit, with kisi's S-parameters of the same build for comparison.
#[derive(Debug, Clone)]
pub struct LaneCircuit {
    /// name of the lane subcircuit
    pub subckt: String,
    pub lane: String,
    /// 4-port pair (both lines) or a single line (2-port; a pair without its partner's path in
    /// the odd mode)
    pub pair: bool,
    /// the pair as its odd mode (2-port per line)
    pub odd_mode: bool,
    /// subcircuit definitions (text), the lane last
    pub lib: String,
    /// port names in subcircuit order (without the reference)
    pub ports: Vec<String>,
    /// what each port is on the board, e.g. "/FPGA/SYNC_OUT_P at U18.A3"
    pub port_info: Vec<String>,
    pub z_ref: f64,
    pub freq: Vec<f64>,
    /// S-matrices (n×n row-major, port order as `ports`): kisi's model and the exported circuit
    pub s_kisi: Vec<Vec<Cx>>,
    pub s_export: Vec<Vec<Cx>>,
    pub counts: Counts,
    pub fidelity: Fidelity,
    pub notes: Vec<String>,
    /// one-way static delay (ps, pairs: differential) and track length (mm)
    pub delay_ps: f64,
    pub length_mm: f64,
}

/// Fit weighting of line sections: R(f) and G(f) within the tolerance, the reactance and the
/// susceptance (the delay and the impedance) ten times closer.
const LINE_W: Weight = Weight::Parts { im: 10.0, re_floor: 0.0 };
/// Via models (short): the reactance and the susceptance first – kisi's via models use a constant
/// loss tangent and a barrel resistance without internal inductance, which no causal circuit
/// follows exactly; their loss is small, their reactance sets the reflections.
const VIA_W: Weight = Weight::Parts { im: 100.0, re_floor: 0.0 };
/// The spans' resistance (barrel, plates, radiation into the plane pair) against at least 2 % of
/// the span's impedance.
const SPAN_W: Weight = Weight::Parts { im: 10.0, re_floor: 0.02 };

/// Turns kisi's sections into circuit elements.
struct Composer<'a> {
    rp: &'a RfParams,
    fmax: f64,
    opts: ExportOpts,
    freqs: Vec<f64>,
    layers: Vec<String>,
    cache: HashMap<Vec<u64>, (Foster, Debye, f64)>,
    worst_fit: f64,
    /// feed trace length (m) of via models that could not be applied to a neighbouring line
    unabsorbed: f64,
    n_via: usize,
    n_line: usize,
    n_part: usize,
    n_stub: usize,
}

/// One element before the feed corrections of the vias are applied to the neighbouring lines.
enum Item {
    /// a line on line 0 / 1 of length m
    Line { line: usize, lm: LineModel, len: f64, layer: u16 },
    /// a coupled run (both lines) of length m
    Coupled { even: LineModel, odd: LineModel, len: f64, layer: u16 },
    /// any other element on one line, with the feed changes it asks of its neighbours (m, entry/exit)
    El { line: usize, el: El, cuts: [f64; 2] },
    /// a coupled block (both lines) in modal form
    Modal { name: String, note: String, even: Chain, odd: Chain, cuts: [f64; 2] },
}

impl Item {
    fn on_line(&self, l: usize) -> bool {
        match self {
            Item::Line { line, .. } | Item::El { line, .. } => *line == l,
            _ => true,
        }
    }
}

fn key_of(lm: &LineModel) -> Vec<u64> {
    [lm.z0, lm.eeff0, lm.l_ext, lm.c0, lm.r_dc, lm.k_ac, lm.er, lm.tand, lm.q_fill, lm.tand_eff0.unwrap_or(-1.0), lm.ms_h.unwrap_or(-1.0)].iter().map(|v| v.to_bits()).collect()
}

impl<'a> Composer<'a> {
    fn new(board: &Board, rp: &'a RfParams, fmax: f64, opts: &ExportOpts) -> Self {
        let su = board.stackup();
        Composer {
            rp,
            fmax,
            opts: opts.clone(),
            freqs: fit_freqs(fmax * 1e-4, fmax, 60),
            layers: su.copper.iter().map(|c| c.name.clone()).collect(),
            cache: HashMap::new(),
            worst_fit: 0.0,
            unabsorbed: 0.0,
            n_via: 0,
            n_line: 0,
            n_part: 0,
            n_stub: 0,
        }
    }

    fn layer(&self, l: u16) -> String {
        self.layers.get(l as usize).cloned().unwrap_or_else(|| format!("L{l}"))
    }

    /// Fitted per-metre networks of a line model (cached).
    fn fits(&mut self, lm: &LineModel) -> (Foster, Debye) {
        let k = key_of(lm);
        if let Some((z, y, _)) = self.cache.get(&k) {
            return (z.clone(), y.clone());
        }
        let rp = self.rp;
        let zt = |f: f64| {
            let p = lm.rlgc(f, rp);
            Cx::new(p.r, 2.0 * PI * f * p.l)
        };
        let yt = |f: f64| {
            let p = lm.rlgc(f, rp);
            Cx::new(p.g, 2.0 * PI * f * p.c)
        };
        let fz = fit_z(&self.freqs, &zt, LINE_W, self.opts.fit_tol, 14, false);
        let fy = fit_y(&self.freqs, &yt, LINE_W, self.opts.fit_tol, 14, lm.ms_h.is_some());
        let err = fz.err.max(fy.err);
        self.worst_fit = self.worst_fit.max(err);
        self.cache.insert(k, (fz.net.clone(), fy.net.clone(), err));
        (fz.net, fy.net)
    }

    /// Number of π sections for a line of `len` m with per-metre L and C.
    fn sections(&self, len: f64, l: f64, c: f64) -> usize {
        let tau = len * (l * c).sqrt();
        ((tau * self.fmax * self.opts.per_lambda).ceil() as usize).max(1)
    }

    /// RLGC ladder of a line section.
    fn ladder(&mut self, lm: &LineModel, len: f64) -> (El, usize) {
        let (fz, fy) = self.fits(lm);
        let c_lo = fy.y(2.0 * PI * self.freqs[0]).im / (2.0 * PI * self.freqs[0]);
        let n = self.sections(len, fz.z(2.0 * PI * self.freqs[0]).im / (2.0 * PI * self.freqs[0]), c_lo);
        let d = len / n as f64;
        (El::Ladder { z: Imp::from_foster(&fz.scaled(d)), y: Adm::from_debye(&fy.scaled(d)), n }, n)
    }

    fn line_block(&mut self, lm: &LineModel, len: f64, layer: u16, what: &str) -> El {
        self.n_line += 1;
        let (ld, n) = self.ladder(lm, len);
        let note = format!(
            "{what}: {:.3} mm on {}, Z0 {:.2} ohm, eps_eff {:.3} at 1 GHz ({}), {n} RLGC sections",
            len * 1e3,
            self.layer(layer),
            lm.z0,
            lm.eeff0,
            if lm.source == "field" { "field solver" } else if lm.source == "formula" { "formula" } else { "no reference plane: estimate" }
        );
        El::Block { name: format!("TL{}_{}", self.n_line, self.layer(layer)), note, chain: Chain(vec![ld]) }
    }

    /// A lossless line (Z, εeff, length m) as an LC ladder.
    fn lossless(&self, z: f64, ee: f64, len: f64) -> El {
        let v = crate::num::C0 / ee.sqrt();
        let (l, c) = (z / v, 1.0 / (z * v));
        let n = self.sections(len, l, c);
        let d = len / n as f64;
        El::Ladder { z: Imp(vec![Zel::L(l * d)]), y: Adm(vec![Imp(vec![Zel::C(c * d)])]), n }
    }

    /// One coaxial section of the via's fallback model as a ladder.
    fn coax(&mut self, s: &Sec) -> El {
        let v = crate::num::C0 / s.er.sqrt();
        let (l, c) = (s.z / v, 1.0 / (s.z * v));
        let sigma = self.rp.sigma;
        let a = s.a;
        let zt = |f: f64| Cx::new((PI * f * crate::num::MU0 / sigma).sqrt() / (2.0 * PI * a), 2.0 * PI * f * l);
        let yt = |f: f64| Cx::new(2.0 * PI * f * c * s.tand, 2.0 * PI * f * c);
        let fz = fit_z(&self.freqs, &zt, VIA_W, self.opts.fit_tol, 10, false);
        let fy = fit_y(&self.freqs, &yt, VIA_W, self.opts.fit_tol, 10, false);
        self.worst_fit = self.worst_fit.max(fz.err.max(fy.err));
        let n = self.sections(s.len, l, c);
        let d = s.len / n as f64;
        let el = El::Ladder { z: Imp::from_foster(&fz.net.scaled(d)), y: Adm::from_debye(&fy.net.scaled(d)), n };
        el
    }

    /// A via as a chain, with the feed changes (m) at its entry and exit.
    fn via(&mut self, vm: &ViaModel, layers: (u16, u16)) -> (El, [f64; 2]) {
        self.n_via += 1;
        let tol = self.opts.fit_tol;
        let mut chain = Vec::new();
        let mut cuts = [0.0; 2];
        let note;
        if let Some(p) = &vm.pbm {
            let (n, i_in, i_out) = p.ladder_ports();
            let freqs = self.freqs.clone();
            let mut node = |k: usize| -> El {
                let fy = fit_y(&freqs, &|f| p.node_y(k, f), LINE_W, tol, 10, false);
                self.worst_fit = self.worst_fit.max(fy.err);
                El::Shunt(Adm::from_debye(&fy.net))
            };
            let mut span_els: Vec<Option<El>> = vec![None; n.saturating_sub(1)];
            let mut node_els: Vec<Option<El>> = (0..n).map(|_| None).collect();
            for (k, ne) in node_els.iter_mut().enumerate() {
                *ne = Some(node(k));
            }
            for (k, se) in span_els.iter_mut().enumerate() {
                let fz = fit_z(&freqs, &|f| p.span_z(k, f), SPAN_W, tol, 10, true);
                self.worst_fit = self.worst_fit.max(fz.err);
                *se = Some(El::Series(Imp::from_foster(&fz.net)));
            }
            let (lo, hi) = (i_in.min(i_out), i_in.max(i_out));
            // open stub from node `from` outwards to `end` (not including node `from`)
            let stub = |from: usize, end: usize| -> Option<El> {
                if from == end {
                    return None;
                }
                let mut c = Vec::new();
                let mut k = from;
                while k != end {
                    let kn = if end > from { k + 1 } else { k - 1 };
                    c.push(span_els[k.min(kn)].clone().unwrap());
                    c.push(node_els[kn].clone().unwrap());
                    k = kn;
                }
                Some(El::Stub(Chain(c)))
            };
            let (stub_a, stub_b) = if i_in <= i_out { (stub(lo, 0), stub(hi, n - 1)) } else { (stub(hi, n - 1), stub(lo, 0)) };
            let mut core = Vec::new();
            core.extend(stub_a);
            core.push(node_els[i_in].clone().unwrap());
            let mut k = i_in;
            while k != i_out {
                let kn = if i_out > i_in { k + 1 } else { k - 1 };
                core.push(span_els[k.min(kn)].clone().unwrap());
                core.push(node_els[kn].clone().unwrap());
                k = kn;
            }
            core.extend(stub_b);
            let (lp, ap, fc) = p.junctions();
            let side = |s: usize, c: &Composer| -> Vec<El> {
                let mut v = Vec::new();
                if let Some((z, er, ln)) = ap[s] {
                    v.push(c.lossless(z, er, ln));
                }
                if lp[s] != 0.0 {
                    v.push(El::Series(Imp(vec![Zel::L(lp[s])])));
                }
                v
            };
            let mut s0 = side(0, self);
            chain.append(&mut s0);
            chain.extend(core);
            let mut s1 = side(1, self);
            s1.reverse();
            chain.extend(s1);
            // feed trace the via model adds (a stripline keeps its line up to the via centre) is a
            // lossless line inside the via; feed trace it removes is taken from the neighbouring
            // line sections (`apply_cuts`)
            for k in 0..2 {
                let (z, ee, ln) = fc[k];
                if ln > 0.0 {
                    let el = self.lossless(z, ee, ln);
                    if k == 0 {
                        chain.insert(0, el);
                    } else {
                        chain.push(el);
                    }
                } else {
                    cuts[k] = ln;
                }
            }
            note = format!("via {} -> {}: kisi's physics-based via ladder ({n} nodes, barrel stubs as open stubs)", self.layer(layers.0), self.layer(layers.1));
        } else {
            let stub = |secs: &[Sec], c: &mut Composer| -> Option<El> {
                if secs.is_empty() {
                    return None;
                }
                Some(El::Stub(Chain(secs.iter().map(|s| c.coax(s)).collect())))
            };
            chain.extend(stub(&vm.stub_in, self));
            let half = vm.barrel.len() / 2;
            for (i, s) in vm.barrel.iter().enumerate() {
                if i == half && vm.l_ret > 0.0 {
                    chain.push(El::Series(Imp(vec![Zel::L(vm.l_ret)])));
                }
                chain.push(self.coax(s));
            }
            if vm.barrel.len() < 2 && vm.l_ret > 0.0 {
                chain.push(El::Series(Imp(vec![Zel::L(vm.l_ret)])));
            }
            chain.extend(stub(&vm.stub_out, self));
            note = format!("via {} -> {}: kisi's coaxial section model ({} sections)", self.layer(layers.0), self.layer(layers.1), vm.barrel.len());
        }
        (El::Block { name: format!("VIA{}_{}_{}", self.n_via, self.layer(layers.0), self.layer(layers.1)), note, chain: Chain(chain) }, cuts)
    }

    fn part(&mut self, p: &LumpedPart) -> El {
        self.n_part += 1;
        let mut z = Vec::new();
        if p.r > 0.0 {
            z.push(Zel::R(p.r));
        }
        if p.l > 0.0 {
            z.push(Zel::L(p.l));
        }
        if let Some(c) = p.c {
            z.push(Zel::C(c));
        }
        let el = if p.series {
            El::Series(Imp(z))
        } else if z.is_empty() {
            // a shunt of 0 Ω would short the line: kisi builds none (placeholders of stubs)
            El::Shunt(Adm(vec![]))
        } else {
            El::Shunt(Adm(vec![Imp(z)]))
        };
        let label = p.label.split_whitespace().next().unwrap_or("PART").to_string();
        let note = format!(
            "{} {}: R {} ohm, L {} nH{} (parasitics as kisi models them, estimates from data/components.json)",
            p.label,
            if p.series { "in series" } else { "to the reference" },
            sig4(p.r),
            sig4(p.l * 1e9),
            p.c.map(|c| format!(", C {} pF", sig4(c * 1e12))).unwrap_or_default()
        );
        El::Block { name: label, note, chain: Chain(vec![el]) }
    }

    /// Items of one line's sections (`len` overrides a line's length, m).
    fn item(&mut self, s: &Sections, i: usize, line: usize, len_mm: Option<f64>, odd_stub: bool) -> Option<Item> {
        let sg = &s.segs[i];
        match sg.kind {
            "line" => {
                let len = len_mm.unwrap_or(sg.len_mm) * 1e-3;
                let lm = s.models[i].clone().unwrap_or_else(|| s.fallback.clone());
                Some(Item::Line { line, lm, len, layer: sg.layer })
            }
            "via" => {
                let vm = s.via_models[i].as_ref()?;
                let (el, cuts) = self.via(vm, (via_from(s, i), sg.layer));
                Some(Item::El { line, el, cuts })
            }
            "series" | "shunt" => s.lumped[i].as_ref().map(|p| Item::El { line, el: self.part(p), cuts: [0.0; 2] }),
            "branch" => {
                let sub = if odd_stub { s.branch_odd[i].as_ref() } else { s.branch[i].as_ref() }?;
                Some(Item::El { line, el: self.stub(sub, &sg.part.clone().unwrap_or_default()), cuts: [0.0; 2] })
            }
            _ => None,
        }
    }


    fn stub(&mut self, sub: &Sections, label: &str) -> El {
        self.n_stub += 1;
        let c = self.chain(sub);
        El::Block { name: format!("STUB{}", self.n_stub), note: format!("open stub: {label}"), chain: Chain(vec![El::Stub(c)]) }
    }

    /// A single line's sections as a chain.
    fn chain(&mut self, s: &Sections) -> Chain {
        let items: Vec<Item> = (0..s.segs.len()).filter_map(|i| self.item(s, i, 0, None, false)).collect();
        let items = self.apply_cuts(items);
        Chain(items.into_iter().filter_map(|it| self.single_el(it)).collect())
    }

    fn single_el(&mut self, it: Item) -> Option<El> {
        match it {
            Item::Line { lm, len, layer, .. } => (len > 0.0).then(|| {
                let b = self.line_block(&lm, len, layer, "line");
                b
            }),
            Item::El { el, .. } => Some(el),
            _ => None,
        }
    }

    /// Apply the feed changes of the vias to the neighbouring line sections: the via models run
    /// from via centre to via centre and remove (or add) feed trace that the lines also count.
    fn apply_cuts(&mut self, mut items: Vec<Item>) -> Vec<Item> {
        for i in 0..items.len() {
            let (cuts, lines): ([f64; 2], Vec<usize>) = match &items[i] {
                Item::El { cuts, line, .. } => (*cuts, vec![*line]),
                Item::Modal { cuts, .. } => (*cuts, vec![0, 1]),
                _ => continue,
            };
            for side in 0..2 {
                let d = cuts[side];
                if d == 0.0 {
                    continue;
                }
                // the first item with length on each of the via's lines, walking away from the via
                let nearest = |items: &Vec<Item>, l: usize| -> Option<usize> {
                    let range: Box<dyn Iterator<Item = usize>> = if side == 0 { Box::new((0..i).rev()) } else { Box::new(i + 1..items.len()) };
                    for j in range {
                        if !items[j].on_line(l) {
                            continue;
                        }
                        match &items[j] {
                            Item::Line { len, .. } | Item::Coupled { len, .. } if *len <= 0.0 => continue,
                            Item::Line { .. } | Item::Coupled { .. } => return Some(j),
                            _ => return None,
                        }
                    }
                    None
                };
                let hits: Vec<Option<usize>> = lines.iter().map(|&l| nearest(&items, l)).collect();
                // a coupled run counts only when it is the nearest on all the via's lines
                let shared = hits.iter().all(|h| h.is_some() && *h == hits[0]) && hits[0].is_some_and(|j| matches!(items[j], Item::Coupled { .. }));
                let adjust = |items: &mut Vec<Item>, j: usize| -> f64 {
                    match &mut items[j] {
                        Item::Line { len, .. } | Item::Coupled { len, .. } => {
                            let nl = (*len + d).max(0.0);
                            let left = d - (nl - *len);
                            *len = nl;
                            left
                        }
                        _ => d,
                    }
                };
                if shared {
                    let left = adjust(&mut items, hits[0].unwrap());
                    self.unabsorbed += left.abs();
                } else {
                    for h in &hits {
                        match h {
                            Some(j) if matches!(items[*j], Item::Line { .. }) => {
                                let left = adjust(&mut items, *j);
                                self.unabsorbed += left.abs();
                            }
                            _ => self.unabsorbed += d.abs(),
                        }
                    }
                }
            }
        }
        items
    }

    /// A pair's aligned elements as modal blocks and single-line elements.
    fn pair(&mut self, bp: &Sections, bn: &Sections, els: &[crate::rf::El]) -> Vec<PairEl> {
        use crate::rf::El as E;
        let mut items = Vec::new();
        for e in els {
            match e {
                E::Single { line, seg, len } => {
                    let s = if *line == 0 { bp } else { bn };
                    items.extend(self.item(s, *seg, *line, *len, false));
                }
                E::Coupled { len, even, odd, layer } => {
                    items.push(Item::Coupled { even: even.clone(), odd: odd.clone(), len: len * 1e-3, layer: *layer });
                }
                E::ViaPair { p } => {
                    let layers = (via_from(bp, *p), bp.segs[*p].layer);
                    let ev = bp.via_models[*p].clone();
                    let od = bp.via_pair[*p].clone().or_else(|| ev.clone());
                    if let (Some(ev), Some(od)) = (ev, od) {
                        let (ee, cuts) = self.via(&ev, layers);
                        self.n_via -= 1;
                        let (oe, _) = self.via(&od, layers);
                        let name = format!("VIAPAIR{}_{}_{}", self.n_via, self.layer(layers.0), self.layer(layers.1));
                        items.push(Item::Modal { name, note: format!("via pair {} -> {}: even mode = the single via, odd mode = the via with its partner (kisi's via models)", self.layer(layers.0), self.layer(layers.1)), even: Chain(vec![ee]), odd: Chain(vec![oe]), cuts });
                    }
                }
                E::StubPair { p, n } => {
                    // kisi: even mode = mean of the two single stubs' admittances, odd = mean of the
                    // stubs built as a pair: each stub with doubled impedances, in parallel
                    let mode = |odd: bool, c: &mut Composer| -> Chain {
                        let mut v = Vec::new();
                        for (s, i) in [(bp, *p), (bn, *n)] {
                            let sub = if odd { s.branch_odd[i].as_ref() } else { s.branch[i].as_ref() };
                            if let Some(sub) = sub {
                                v.push(El::Stub(c.chain(sub).scaled(2.0)));
                            }
                        }
                        Chain(v)
                    };
                    let even = mode(false, self);
                    let odd = mode(true, self);
                    self.n_stub += 1;
                    items.push(Item::Modal { name: format!("STUBPAIR{}", self.n_stub), note: format!("stub pair: {}", bp.segs[*p].part.clone().unwrap_or_default()), even, odd, cuts: [0.0; 2] });
                }
            }
        }
        let items = self.apply_cuts(items);
        // consecutive coupled items form one modal block
        let mut out: Vec<PairEl> = Vec::new();
        let mut run: Vec<(Chain, Chain, String)> = Vec::new();
        let flush = |out: &mut Vec<PairEl>, run: &mut Vec<(Chain, Chain, String)>, k: &mut usize| {
            if run.is_empty() {
                return;
            }
            *k += 1;
            let (mut ev, mut od, mut notes) = (Vec::new(), Vec::new(), Vec::new());
            for (e, o, n) in run.drain(..) {
                ev.extend(e.0);
                od.extend(o.0);
                notes.push(n);
            }
            out.push(PairEl::Modal { name: format!("CPL{k}"), note: notes.join("\n"), even: Chain(ev), odd: Chain(od) });
        };
        let mut k = 0usize;
        for it in items {
            match it {
                Item::Coupled { even, odd, len, layer } => {
                    if len <= 0.0 {
                        continue;
                    }
                    let e = self.line_block(&even, len, layer, "coupled line, even mode");
                    let o = self.line_block(&odd, len, layer, "coupled line, odd mode");
                    run.push((Chain(vec![e]), Chain(vec![o]), format!("coupled run {:.3} mm on {} (Zodd {:.2}, Zeven {:.2} ohm per line)", len * 1e3, self.layer(layer), odd.z0, even.z0)));
                }
                Item::Modal { name, note, even, odd, .. } => run.push((Chain(vec![El::Block { name: format!("{name}_e"), note: note.clone(), chain: even }]), Chain(vec![El::Block { name: format!("{name}_o"), note: note.clone(), chain: odd }]), note)),
                other => {
                    let line = match &other {
                        Item::Line { line, .. } | Item::El { line, .. } => *line,
                        _ => 0,
                    };
                    if let Some(el) = self.single_el(other) {
                        flush(&mut out, &mut run, &mut k);
                        out.push(PairEl::Line(line, el));
                    }
                }
            }
        }
        flush(&mut out, &mut run, &mut k);
        out
    }
}

/// kisi's cascade matrix of section `i` (as `rf::Build::seg_abcd`), for checks.
#[cfg(test)]
pub(crate) fn kisi_abcd(s: &Sections, i: usize, f: f64, rp: &RfParams, len_mm: Option<f64>) -> Abcd {
    let w = 2.0 * PI * f;
    let sg = &s.segs[i];
    match sg.kind {
        "branch" => match s.branch[i].as_ref() {
            Some(sub) => {
                let m = (0..sub.segs.len()).fold(Abcd::identity(), |m, j| m.mul(&kisi_abcd(sub, j, f, rp, None)));
                Abcd::shunt(m.c / m.a)
            }
            None => Abcd::identity(),
        },
        "via" => s.via_models[i].as_ref().map(|v| v.abcd(f, rp.sigma)).unwrap_or_else(Abcd::identity),
        "series" | "shunt" => s.lumped[i].as_ref().map(|p| p.abcd(w)).unwrap_or_else(Abcd::identity),
        _ => {
            let lm = s.models[i].as_ref().unwrap_or(&s.fallback);
            let p = lm.rlgc(f, rp);
            Abcd::line(p.r, p.l, p.g, p.c, w, len_mm.unwrap_or(sg.len_mm) * 1e-3)
        }
    }
}

/// A value with 4 significant digits.
pub fn sig4(v: f64) -> String {
    if v == 0.0 || !v.is_finite() {
        return format!("{v}");
    }
    let d = (3 - v.abs().log10().floor() as i32).max(0) as usize;
    format!("{v:.d$}")
}

/// The layer a via section starts on: the layer of the line before it.
fn via_from(s: &Sections, i: usize) -> u16 {
    s.segs[..i].iter().rev().find(|x| x.kind == "line").map(|x| x.layer).unwrap_or(s.segs[i].layer)
}

/// S-matrix (row-major) of a 2-port cascade.
fn s2(m: &Abcd, z0: f64) -> Vec<Cx> {
    let (s11, s21, s22) = m.s_params(z0);
    vec![s11, s21, s21, s22]
}

fn s4(t: &M4, z0: f64) -> Vec<Cx> {
    crate::rf::s_of(t, z0).iter().flatten().copied().collect()
}

/// Differential S (Sdd11, Sdd21) of a 4-port in the order P1 N1 P2 N2.
fn sdd(s: &[Cx]) -> (Cx, Cx) {
    let mut m = [[Cx::new(0.0, 0.0); 4]; 4];
    for i in 0..4 {
        for j in 0..4 {
            m[i][j] = s[i * 4 + j];
        }
    }
    let mm = crate::rf::mixed_of(&m);
    (mm[0][0], mm[1][0])
}

fn db(x: Cx) -> f64 {
    20.0 * x.abs().max(1e-15).log10()
}

/// Compare the exported circuit with kisi's S-parameters.
fn fidelity(freq: &[f64], a: &[Vec<Cx>], b: &[Vec<Cx>], pair: bool, f_nyq: f64) -> Fidelity {
    let mut fd = Fidelity::default();
    for k in 0..freq.len() {
        let (s11a, s21a, s11b, s21b) = if pair {
            let (x, y) = sdd(&a[k]);
            let (u, v) = sdd(&b[k]);
            (x, y, u, v)
        } else {
            (a[k][0], a[k][2], b[k][0], b[k][2])
        };
        let d21 = (db(s21a) - db(s21b)).abs();
        fd.max_ds21_db = fd.max_ds21_db.max(d21);
        if freq[k] <= 1.5 * f_nyq {
            fd.max_ds21_db_nyq = fd.max_ds21_db_nyq.max(d21);
        }
        fd.max_ds11 = fd.max_ds11.max((s11a - s11b).abs());
        fd.max_ds = fd.max_ds.max(a[k].iter().zip(&b[k]).map(|(x, y)| (*x - *y).abs()).fold(0.0, f64::max));
    }
    fd
}

/// The lane's board channel as a circuit: the sections kisi's channel check builds for the
/// network of the eye (the terminations next to the receiver are left out – they act as the
/// receiver's termination, `topology::rx_terminations`), transmitter end = port 1 (P, N), receiver
/// end = port 2. `rp` as for the check (`fmax`, `npts`); `f_nyq` for the in-band fidelity.
#[allow(clippy::too_many_arguments)]
pub fn lane_circuit(board: &Board, an: &Analysis, field: &HashMap<String, FieldResult>, lane: &Lane, target: Option<(f64, bool)>, rp: &RfParams, reversed: bool, cd: &ComponentData, f_nyq: f64, opts: &ExportOpts) -> Result<LaneCircuit, String> {
    let n = rp.npts.max(10);
    let df = rp.fmax / n as f64;
    let freq: Vec<f64> = (1..=n).map(|k| k as f64 * df).collect();
    let tb = crate::topology::lane_builds(board, an, field, lane, target, rp, reversed, cd)?;
    let mut comp = Composer::new(board, rp, rp.fmax, opts);
    let prefix = lane.name.rsplit('/').next().unwrap_or(&lane.name).to_string();
    let mut lib = Lib::new(&prefix);
    let subckt = lib.unique("lane");
    let ends = if reversed { (1, 0) } else { (0, 1) };
    let pin = |e: Option<&crate::topology::EndPin>| e.map(|e| format!("{}.{}", e.refdes, e.pin)).unwrap_or_else(|| "?".into());
    let net = |id: u32| board.net_name(id).to_string();
    let mut notes = tb.notes.clone();
    let (pair, odd_mode, s_kisi, s_export, ports, port_info, z_ref, delay_ps, length_mm);
    match tb.builds {
        crate::topology::LaneBuilds::Pair { bp, bn, z_ref: zr } => {
            let (sp, sn) = (bp.sections(target), bn.sections(target));
            let els = crate::rf::align_pair(&bp, &bn, an, board.stackup(), field, rp).els;
            let pels = comp.pair(&sp, &sn, &els);
            let pn = crate::rf::pair_net(board, an, field, target, rp, bp, bn);
            notes.extend(pn.warnings.iter().cloned());
            s_kisi = pn.t.iter().map(|t| s4(t, zr)).collect::<Vec<_>>();
            s_export = freq.iter().map(|&f| s4(&circuit::pair_t(&pels, 2.0 * PI * f), zr)).collect::<Vec<_>>();
            lib.pair_subckt(&subckt, &format!("lane {}: pair, transmitter end = ap/an, receiver end = bp/bn", lane.name), &pels);
            pair = true;
            odd_mode = false;
            z_ref = zr;
            let (pnet, nnet) = (net(lane.nets.first().copied().unwrap_or(0)), net(lane.partner_nets.first().copied().unwrap_or(0)));
            let (pnet_b, nnet_b) = (net(lane.nets.last().copied().unwrap_or(0)), net(lane.partner_nets.last().copied().unwrap_or(0)));
            let (pa, pb) = if reversed { (pnet_b.clone(), pnet.clone()) } else { (pnet.clone(), pnet_b.clone()) };
            let (na, nb) = if reversed { (nnet_b.clone(), nnet.clone()) } else { (nnet.clone(), nnet_b.clone()) };
            let pe = |i: usize| lane.partner_ends.iter().find(|e| lane.ends.get(i).is_some_and(|x| x.refdes == e.refdes));
            ports = vec!["ap".into(), "an".into(), "bp".into(), "bn".into()];
            port_info = vec![
                format!("P {pa} at {} (transmitter end)", pin(lane.ends.get(ends.0))),
                format!("N {na} at {} (transmitter end)", pin(pe(ends.0))),
                format!("P {pb} at {} (receiver end)", pin(lane.ends.get(ends.1))),
                format!("N {nb} at {} (receiver end)", pin(pe(ends.1))),
            ];
            let len = |s: &Sections| s.segs.iter().filter(|x| x.kind == "line").map(|x| x.len_mm).sum::<f64>();
            length_mm = len(&sp).max(len(&sn));
            delay_ps = tb.delay_ps;
        }
        crate::topology::LaneBuilds::Single { b, odd } => {
            let s = b.sections(target);
            let chain = comp.chain(&s);
            let ch = b.finish(lane.nets[0], lane.name.clone(), None, target);
            notes.extend(ch.warnings.iter().cloned());
            z_ref = ch.z_ref;
            s_kisi = ch.abcd.iter().map(|m| s2(m, z_ref)).collect::<Vec<_>>();
            s_export = freq.iter().map(|&f| s2(&chain.abcd(2.0 * PI * f), z_ref)).collect::<Vec<_>>();
            lib.chain_subckt(&subckt, &format!("lane {}: {}, transmitter end = a, receiver end = b", lane.name, if odd { "pair in the odd mode (one line, odd-mode models; differential = 2 x per line)" } else { "single line" }), &chain);
            pair = false;
            odd_mode = odd;
            let (na, nb) = if reversed { (net(*lane.nets.last().unwrap()), net(lane.nets[0])) } else { (net(lane.nets[0]), net(*lane.nets.last().unwrap())) };
            ports = vec!["a".into(), "b".into()];
            port_info = vec![format!("{na} at {} (transmitter end)", pin(lane.ends.get(ends.0))), format!("{nb} at {} (receiver end)", pin(lane.ends.get(ends.1)))];
            length_mm = ch.length_mm;
            delay_ps = ch.delay_ps;
        }
    }
    let mut fd = fidelity(&freq, &s_kisi, &s_export, pair, f_nyq);
    fd.worst_fit = comp.worst_fit;
    if comp.unabsorbed > 1e-6 {
        notes.push(format!("{:.3} mm of via feed corrections could not be applied to a neighbouring line (no line next to the via)", comp.unabsorbed * 1e3));
    }
    let counts = lib.flat_of(&subckt);
    let mut dedup = Vec::new();
    for w in notes {
        if !dedup.contains(&w) {
            dedup.push(w);
        }
    }
    Ok(LaneCircuit {
        subckt,
        lane: lane.name.clone(),
        pair,
        odd_mode,
        lib: lib.text(),
        ports,
        port_info,
        z_ref,
        freq,
        s_kisi,
        s_export,
        counts,
        fidelity: fd,
        notes: dedup,
        delay_ps,
        length_mm,
    })
}

/// Inverse of a complex n×n matrix (row-major, Gauss-Jordan with pivoting).
fn cinv(a: &[Cx], n: usize) -> Vec<Cx> {
    let mut m = a.to_vec();
    let mut inv: Vec<Cx> = (0..n * n).map(|k| if k / n == k % n { Cx::new(1.0, 0.0) } else { Cx::new(0.0, 0.0) }).collect();
    for c in 0..n {
        let p = (c..n).max_by(|&x, &y| m[x * n + c].abs().total_cmp(&m[y * n + c].abs())).unwrap();
        if p != c {
            for j in 0..n {
                m.swap(c * n + j, p * n + j);
                inv.swap(c * n + j, p * n + j);
            }
        }
        let d = m[c * n + c].inv();
        for j in 0..n {
            m[c * n + j] = m[c * n + j] * d;
            inv[c * n + j] = inv[c * n + j] * d;
        }
        for r in 0..n {
            if r != c {
                let f = m[r * n + c];
                if f.abs() == 0.0 {
                    continue;
                }
                for j in 0..n {
                    m[r * n + j] = m[r * n + j] - f * m[c * n + j];
                    inv[r * n + j] = inv[r * n + j] - f * inv[c * n + j];
                }
            }
        }
    }
    inv
}

/// S-matrix (n×n, row-major) renormalised from `z_old` to `z_new` at every port:
/// S' = (S − Γ)(1 − Γ S)⁻¹ with Γ = (z_new − z_old)/(z_new + z_old).
pub fn renormalize(s: &[Cx], n: usize, z_old: f64, z_new: f64) -> Vec<Cx> {
    if (z_old - z_new).abs() < 1e-12 {
        return s.to_vec();
    }
    let g = (z_new - z_old) / (z_new + z_old);
    let a: Vec<Cx> = (0..n * n).map(|k| s[k] - if k / n == k % n { Cx::new(g, 0.0) } else { Cx::new(0.0, 0.0) }).collect();
    let b: Vec<Cx> = (0..n * n).map(|k| (if k / n == k % n { Cx::new(1.0, 0.0) } else { Cx::new(0.0, 0.0) }) - s[k].scale(g)).collect();
    let bi = cinv(&b, n);
    (0..n * n)
        .map(|k| {
            let (i, j) = (k / n, k % n);
            (0..n).fold(Cx::new(0.0, 0.0), |acc, q| acc + a[i * n + q] * bi[q * n + j])
        })
        .collect()
}

/// The ports of a set of lanes at the two devices of a bus: lane `i`'s ports in the bus port
/// order (near = device A side first). Returns per lane whether it runs from B to A (flipped).
pub fn bus_flips(lanes: &[&Lane], reversed: &[bool], device_a: &str) -> Vec<bool> {
    lanes
        .iter()
        .zip(reversed)
        .map(|(l, &rev)| {
            let tx = &l.ends[if rev { 1 } else { 0 }];
            tx.refdes != device_a
        })
        .collect()
}

#[cfg(test)]
mod tests;

//! High-frequency models (milestone M3): frequency-dependent RLGC per
//! cross-section, the channel of a net (cascade → S-parameters, TDR) and
//! crosstalk to neighbouring traces.
//!
//! Units inside this module are SI (m, Ω, H, S, F, Hz, s) unless a name says
//! otherwise; board geometry (mm) is converted on entry.

use crate::analytic;
use crate::board::{Board, Stackup};
use crate::extract::{Analysis, CrossSection, FieldResult, Kind};
use crate::geom::{self, P};
use crate::xsgeom::{self, PlaneCut, XsDesc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::f64::consts::PI;

pub const C0: f64 = 299_792_458.0;
pub const MU0: f64 = 4.0e-7 * PI;
pub const F_REF: f64 = 1.0e9;

// --- complex numbers ------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Cx {
    pub re: f64,
    pub im: f64,
}

impl Cx {
    pub const fn new(re: f64, im: f64) -> Self {
        Cx { re, im }
    }
    pub fn abs(self) -> f64 {
        self.re.hypot(self.im)
    }
    pub fn arg(self) -> f64 {
        self.im.atan2(self.re)
    }
    pub fn sqrt(self) -> Cx {
        let r = self.abs().sqrt();
        let a = self.arg() / 2.0;
        Cx::new(r * a.cos(), r * a.sin())
    }
    pub fn exp(self) -> Cx {
        let e = self.re.exp();
        Cx::new(e * self.im.cos(), e * self.im.sin())
    }
    pub fn inv(self) -> Cx {
        let d = self.re * self.re + self.im * self.im;
        Cx::new(self.re / d, -self.im / d)
    }
    pub fn scale(self, k: f64) -> Cx {
        Cx::new(self.re * k, self.im * k)
    }
}
impl std::ops::Add for Cx {
    type Output = Cx;
    fn add(self, o: Cx) -> Cx {
        Cx::new(self.re + o.re, self.im + o.im)
    }
}
impl std::ops::Sub for Cx {
    type Output = Cx;
    fn sub(self, o: Cx) -> Cx {
        Cx::new(self.re - o.re, self.im - o.im)
    }
}
impl std::ops::Mul for Cx {
    type Output = Cx;
    fn mul(self, o: Cx) -> Cx {
        Cx::new(self.re * o.re - self.im * o.im, self.re * o.im + self.im * o.re)
    }
}
impl std::ops::Div for Cx {
    type Output = Cx;
    fn div(self, o: Cx) -> Cx {
        self * o.inv()
    }
}

fn cosh_sinh(x: Cx) -> (Cx, Cx) {
    let e = x.exp();
    let ei = (Cx::new(-x.re, -x.im)).exp();
    ((e + ei).scale(0.5), (e - ei).scale(0.5))
}

/// ABCD matrix [a b; c d]
#[derive(Debug, Clone, Copy)]
pub struct Abcd {
    pub a: Cx,
    pub b: Cx,
    pub c: Cx,
    pub d: Cx,
}

impl Abcd {
    pub fn identity() -> Self {
        Abcd { a: Cx::new(1.0, 0.0), b: Cx::new(0.0, 0.0), c: Cx::new(0.0, 0.0), d: Cx::new(1.0, 0.0) }
    }
    pub fn mul(&self, o: &Abcd) -> Abcd {
        Abcd {
            a: self.a * o.a + self.b * o.c,
            b: self.a * o.b + self.b * o.d,
            c: self.c * o.a + self.d * o.c,
            d: self.c * o.b + self.d * o.d,
        }
    }
    /// Transmission line of length `len` (m) with RLGC at angular frequency w.
    pub fn line(r: f64, l: f64, g: f64, c: f64, w: f64, len: f64) -> Abcd {
        let z = Cx::new(r, w * l);
        let y = Cx::new(g, w * c);
        let gamma = (z * y).sqrt();
        let zc = (z / y).sqrt();
        let (ch, sh) = cosh_sinh(gamma.scale(len));
        Abcd { a: ch, b: zc * sh, c: sh / zc, d: ch }
    }
    pub fn series(z: Cx) -> Abcd {
        Abcd { a: Cx::new(1.0, 0.0), b: z, c: Cx::new(0.0, 0.0), d: Cx::new(1.0, 0.0) }
    }
    pub fn shunt(y: Cx) -> Abcd {
        Abcd { a: Cx::new(1.0, 0.0), b: Cx::new(0.0, 0.0), c: y, d: Cx::new(1.0, 0.0) }
    }
    /// S11, S21 for a real reference impedance (reciprocal, both ports).
    pub fn s_params(&self, z0: f64) -> (Cx, Cx, Cx) {
        let bz = self.b.scale(1.0 / z0);
        let cz = self.c.scale(z0);
        let den = self.a + bz + cz + self.d;
        let s11 = (self.a + bz - cz - self.d) / den;
        let s21 = Cx::new(2.0, 0.0) / den;
        let s22 = (Cx::new(0.0, 0.0) - self.a + bz - cz + self.d) / den;
        (s11, s21, s22)
    }
}

// --- materials ---------------------------------------------------------------------

/// Djordjevic-Sarkar wideband Debye model fitted to (er, tanδ) at 1 GHz,
/// poles from 10 kHz to 1 THz.
#[derive(Debug, Clone, Copy)]
pub struct DjordjevicSarkar {
    pub e_inf: f64,
    pub de: f64,
}

impl DjordjevicSarkar {
    const M1: f64 = 4.0;
    const M2: f64 = 12.0;
    fn k(f: f64) -> Cx {
        let w = 2.0 * PI * f;
        let w1 = 2.0 * PI * 10f64.powf(Self::M1);
        let w2 = 2.0 * PI * 10f64.powf(Self::M2);
        let num = Cx::new(w2, w);
        let den = Cx::new(w1, w);
        let q = num / den;
        // log10 of a complex number
        Cx::new(q.abs().ln(), q.arg()).scale(1.0 / (std::f64::consts::LN_10 * (Self::M2 - Self::M1)))
    }
    pub fn fit(er: f64, tand: f64) -> Self {
        let k = Self::k(F_REF);
        let de = if tand > 0.0 { -er * tand / k.im } else { 0.0 };
        DjordjevicSarkar { e_inf: er - de * k.re, de }
    }
    /// (ε', tanδ) at frequency f
    pub fn at(&self, f: f64) -> (f64, f64) {
        let k = Self::k(f.max(1.0));
        let re = self.e_inf + self.de * k.re;
        let im = -self.de * k.im;
        (re, if re > 0.0 { im / re } else { 0.0 })
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum Roughness {
    Smooth,
    Hvlp,
    Standard,
}

impl Roughness {
    /// Huray "cannonball" factor: 1 + 1.5·SR / (1 + δ/r + δ²/(2r²)).
    /// Preset (r, SR) are estimates for typical foils.
    pub fn factor(&self, f: f64, sigma: f64) -> f64 {
        let (r, sr) = match self {
            Roughness::Smooth => return 1.0,
            Roughness::Hvlp => (0.5e-6, 0.5),
            Roughness::Standard => (1.0e-6, 1.0),
        };
        let delta = 1.0 / (PI * f * MU0 * sigma).sqrt();
        1.0 + 1.5 * sr / (1.0 + delta / r + delta * delta / (2.0 * r * r))
    }
}

// --- parameters -------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct RfParams {
    /// upper frequency (Hz)
    pub fmax: f64,
    pub npts: usize,
    /// 10-90 % rise time for TDR and crosstalk (s)
    pub rise: f64,
    pub roughness: Roughness,
    /// copper conductivity (S/m)
    pub sigma: f64,
    /// reference impedance; None: target of the net class, else 50 / 100 Ω
    pub z_ref: Option<f64>,
    /// include the via model
    pub vias: bool,
    /// assume every via is back-drilled to this residual stub (mm); None: as in the layout
    pub backdrill: Option<f64>,
}

impl Default for RfParams {
    fn default() -> Self {
        RfParams { fmax: 25e9, npts: 500, rise: 35e-12, roughness: Roughness::Standard, sigma: 5.8e7, z_ref: None, vias: true, backdrill: None }
    }
}

// --- per-unit-length model ------------------------------------------------------------

/// Frequency-dependent line model of one cross-section (single line or the
/// odd mode of a pair, per line).
#[derive(Debug, Clone, Serialize)]
pub struct LineModel {
    pub z0: f64,
    pub eeff0: f64,
    /// external inductance (H/m) and static capacitance (F/m)
    pub l_ext: f64,
    pub c0: f64,
    pub r_dc: f64,
    /// smooth-copper AC resistance coefficient: R_ac = k_ac·√f (Ω/m/√Hz)
    pub k_ac: f64,
    /// material permittivity / loss tangent at 1 GHz and filling factor
    pub er: f64,
    pub tand: f64,
    pub q_fill: f64,
    /// dielectric loss tangent of the mode at 1 GHz when it came from a field solve
    pub tand_eff0: Option<f64>,
    /// substrate height for microstrip dispersion (m), None for stripline
    pub ms_h: Option<f64>,
    pub source: &'static str,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct Rlgc {
    pub r: f64,
    pub l: f64,
    pub g: f64,
    pub c: f64,
}

impl LineModel {
    pub fn rlgc(&self, f: f64, rp: &RfParams) -> Rlgc {
        let w = 2.0 * PI * f;
        let ds = DjordjevicSarkar::fit(self.er, self.tand);
        let (er_f, tand_f) = ds.at(f);
        let (er_0, tand_0) = ds.at(F_REF);
        // effective permittivity with material dispersion
        let ee_mat = |e: f64| if self.ms_h.is_some() { 1.0 + self.q_fill * (e - 1.0) } else { e };
        let mut eeff = self.eeff0 * ee_mat(er_f) / ee_mat(er_0);
        if let Some(h) = self.ms_h {
            // Getsinger dispersion of microstrip
            let fp = self.z0 / (2.0 * MU0 * h);
            let gg = 0.6 + 0.009 * self.z0;
            eeff = er_f - (er_f - eeff) / (1.0 + gg * (f / fp).powi(2));
        }
        let c = self.c0 * eeff / self.eeff0;
        let r_ac_smooth = self.k_ac * f.sqrt();
        let r_ac = r_ac_smooth * rp.roughness.factor(f, rp.sigma);
        let r = (self.r_dc * self.r_dc + r_ac * r_ac).sqrt();
        let l = self.l_ext + if w > 0.0 { r_ac_smooth / w } else { 0.0 };
        let tand_eff = match self.tand_eff0 {
            Some(t0) => t0 * if tand_0 > 0.0 { tand_f * er_f / (tand_0 * er_0) } else { 0.0 },
            None => tand_f * self.q_fill * er_f / eeff,
        };
        Rlgc { r, l, g: w * c * tand_eff, c }
    }
}

/// Inductance per metre of a geometry in air (formula path), for Wheeler's
/// incremental inductance rule. Lengths in mm.
fn l_air(kind: Kind, w: f64, t: f64, h1: f64, h2: f64, odd_gap: Option<f64>) -> f64 {
    let z = match (kind, odd_gap) {
        (Kind::Stripline, None) => analytic::stripline_asym(w, h1, h2, t, 1.0, 1.0),
        (Kind::Stripline, Some(s)) => {
            // odd mode: Cohn ratio at the plane spacing applied to the SE value
            let b = h1 + h2 + t;
            let (_, zo) = analytic::cohn_coupled(w, s, b, 1.0);
            let (_, z0c) = analytic::cohn_coupled(w, 1e3, b, 1.0);
            analytic::stripline_asym(w, h1, h2, t, 1.0, 1.0) * zo / z0c
        }
        (_, None) => analytic::microstrip(w, h1, t, 1.0).0,
        (_, Some(s)) => {
            let (_, zo, _, _) = analytic::kirschning_jansen(w, s, h1, 1.0);
            let z00 = analytic::microstrip(w, h1, 0.0, 1.0).0;
            analytic::microstrip(w, h1, t, 1.0).0 * zo / z00
        }
    };
    z / C0
}

/// Material seen by a cross-section: (er, tanδ) at 1 GHz, weighted by the
/// capacitance of the dielectric above and below.
fn xs_material(su: &Stackup, xs: &CrossSection) -> (f64, f64) {
    let li = xs.layer as usize;
    let tand_between = |j: usize| -> f64 {
        let (a, b) = if li < j { (li, j) } else { (j, li) };
        let (mut h, mut s) = (0.0, 0.0);
        for k in a..b {
            let d = &su.dielectrics[k];
            h += d.thickness;
            s += d.thickness * d.loss_tangent;
        }
        if h > 0.0 { s / h } else { 0.02 }
    };
    let mut parts = Vec::new();
    for (r, h, e) in [(xs.ref_up, xs.h_up, xs.er_up), (xs.ref_dn, xs.h_dn, xs.er_dn)] {
        if let (Some(j), Some(h), Some(e)) = (r, h, e) {
            parts.push((1.0 / h.max(1e-6), e, tand_between(j as usize)));
        }
    }
    let wsum: f64 = parts.iter().map(|p| p.0).sum();
    if wsum <= 0.0 {
        return (4.2, 0.02);
    }
    let er = parts.iter().map(|p| p.0 * p.1).sum::<f64>() / wsum;
    let tand = parts.iter().map(|p| p.0 * p.2).sum::<f64>() / wsum;
    (er, tand)
}

/// Line model of a cross-section. `odd`: odd mode of the diff pair.
pub fn line_model(su: &Stackup, xs: &CrossSection, field: Option<&FieldResult>, odd: bool, rp: &RfParams) -> Option<LineModel> {
    let (er, tand) = xs_material(su, xs);
    let (z, eeff) = if odd {
        let zd = xs.zdiff?;
        let ee = field.and_then(|f| match (f.eeff_odd, f.eeff_even) {
            (Some(o), _) => Some(o),
            _ => None,
        });
        (zd / 2.0, ee.or(xs.eeff)?)
    } else {
        (xs.z0?, xs.eeff?)
    };
    let w_mm = xs.w;
    let t_mm = xs.t;
    let (h1, h2) = match xs.kind {
        Kind::Stripline => (xs.h_dn.unwrap_or(0.1), xs.h_up.unwrap_or(0.1)),
        _ => (xs.h_dn.or(xs.h_up).unwrap_or(0.1), 0.0),
    };
    let ms_h = if xs.kind == Kind::Stripline { None } else { Some(h1 * 1e-3) };
    let q_fill = if xs.kind == Kind::Stripline || er <= 1.0 { 1.0 } else { ((eeff - 1.0) / (er - 1.0)).clamp(0.05, 1.0) };
    let r_dc = 1.0 / (rp.sigma * (w_mm * 1e-3) * (t_mm * 1e-3));

    // field solve at f0 if available (smooth copper)
    let fr = field.and_then(|f| {
        let f0 = f.f0.unwrap_or(F_REF);
        if odd {
            Some((f.r_odd?, f.l_odd?, f.g_odd, f.c_odd?, f0))
        } else {
            Some((f.r?, f.l?, f.g, f.c?, f0))
        }
    });
    if let Some((r0, l0, g0, c0f, f0)) = fr {
        let w0 = 2.0 * PI * f0;
        let r_ac0 = (r0 * r0 - r_dc * r_dc).max(0.0).sqrt();
        let l_ext = (l0 - r_ac0 / w0).max(l0 * 0.5);
        return Some(LineModel {
            z0: (l_ext / c0f).sqrt(),
            eeff0: l_ext * c0f * C0 * C0,
            l_ext,
            c0: c0f,
            r_dc,
            k_ac: r_ac0 / f0.sqrt(),
            er,
            tand,
            q_fill,
            tand_eff0: g0.map(|g| g / (w0 * c0f)),
            ms_h,
            source: "field",
        });
    }

    // formula path: Wheeler's incremental inductance for the AC resistance
    let dn = 1e-3; // mm (1 µm recession)
    let gap = if odd { xs.diff_gap } else { None };
    let la = l_air(xs.kind, w_mm, t_mm, h1, h2, gap);
    let lb = l_air(xs.kind, w_mm - 2.0 * dn, (t_mm - 2.0 * dn).max(1e-4), h1 + 2.0 * dn, h2 + 2.0 * dn, gap.map(|s| s + 2.0 * dn));
    let dl_dn = ((lb - la) / (dn * 1e-3)).max(0.0);
    let k_ac = (PI * MU0 / rp.sigma).sqrt() / MU0 * dl_dn;
    let l_ext = z * eeff.sqrt() / C0;
    let c0 = eeff.sqrt() / (z * C0);
    Some(LineModel { z0: z, eeff0: eeff, l_ext, c0, r_dc, k_ac, er, tand, q_fill, tand_eff0: None, ms_h, source: "formula" })
}

// --- channel ---------------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct Segment {
    /// "line" or "via"
    pub kind: &'static str,
    pub len_mm: f64,
    pub layer: u16,
    pub xs: Option<u32>,
    pub z: Option<f64>,
    /// via: (L nH, C pF)
    pub via_lc: Option<(f64, f64)>,
    pub source: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct Channel {
    pub net: u32,
    pub name: String,
    pub diff: bool,
    pub partner: Option<u32>,
    pub z_ref: f64,
    pub length_mm: f64,
    pub delay_ps: f64,
    pub vias: usize,
    pub segments: Vec<Segment>,
    pub freq: Vec<f64>,
    pub s21_db: Vec<f64>,
    pub s11_db: Vec<f64>,
    /// insertion loss at Nyquist-like frequencies: (GHz, dB)
    pub il_marks: Vec<(f64, f64)>,
    pub tdr_t_ps: Vec<f64>,
    pub tdr_z: Vec<f64>,
    pub tdr_x_mm: Vec<f64>,
    pub loss_db_per_in_at: Vec<(f64, f64, f64)>,
    pub via_info: Vec<crate::via::ViaInfo>,
    pub field_share: f64,
    pub warnings: Vec<String>,
    #[serde(skip)]
    pub s11: Vec<Cx>,
    #[serde(skip)]
    pub s21: Vec<Cx>,
}

/// Ordered path of a net: (piece indices in order, reversed flag per piece,
/// via positions between pieces). Longest path through the track graph.
fn net_path(board: &Board, an: &Analysis, net: u32, warnings: &mut Vec<String>) -> Vec<(usize, bool)> {
    // tracks of the net (real tracks and synthetic pad tracks)
    let mut tracks: Vec<usize> = Vec::new();
    let mut pieces_of: HashMap<usize, Vec<usize>> = HashMap::new();
    for (pi, pc) in an.pieces.chunks_exact(4).enumerate() {
        let ti = pc[0] as usize;
        if board.tracks[ti].net != net {
            continue;
        }
        if !pieces_of.contains_key(&ti) {
            tracks.push(ti);
        }
        pieces_of.entry(ti).or_default().push(pi);
    }
    if tracks.is_empty() {
        return vec![];
    }
    // nodes = endpoints per layer (5 µm grid); layers connect only through
    // vias and plated holes (joined below); pad tracks connect at their centre too
    let key = |p: P, l: u16| ((p[0] * 200.0).round() as i64, (p[1] * 200.0).round() as i64, l);
    let mut node_id: HashMap<(i64, i64, u16), usize> = HashMap::new();
    let nid = |p: P, l: u16, node_id: &mut HashMap<(i64, i64, u16), usize>| -> usize {
        let k = key(p, l);
        let n = node_id.len();
        *node_id.entry(k).or_insert(n)
    };
    let mut edges: Vec<(usize, usize, usize, f64)> = Vec::new(); // (track, a, b, len)
    let mut node_layers: Vec<u64> = Vec::new();
    for &ti in &tracks {
        let t = &board.tracks[ti];
        let a = nid(t.a, t.layer, &mut node_id);
        let b = nid(t.b, t.layer, &mut node_id);
        node_layers.resize(node_id.len(), 0);
        node_layers[a] |= 1u64 << t.layer.min(63);
        node_layers[b] |= 1u64 << t.layer.min(63);
        edges.push((ti, a, b, geom::len(geom::sub(t.b, t.a))));
    }
    // join nodes that lie inside a pad track or very close (track ending in a
    // pad off-centre, small gaps from rounding)
    let n = node_id.len();
    let mut pos = vec![[0.0; 2]; n];
    for (&k, &i) in &node_id {
        pos[i] = [k.0 as f64 / 200.0, k.1 as f64 / 200.0];
    }
    let mut parent: Vec<usize> = (0..n).collect();
    fn find(p: &mut Vec<usize>, x: usize) -> usize {
        let mut r = x;
        while p[r] != r {
            r = p[r];
        }
        let mut c = x;
        while p[c] != r {
            let nx = p[c];
            p[c] = r;
            c = nx;
        }
        r
    }
    for &ti in &tracks {
        let t = &board.tracks[ti];
        let tol = if t.pad { t.width / 2.0 } else { 0.02 };
        for i in 0..n {
            // only copper on the same layer connects (vias are joined below)
            if node_layers[i] & (1u64 << t.layer.min(63)) == 0 {
                continue;
            }
            // distance of node to the track centre line
            let d = geom::sub(t.b, t.a);
            let l2 = geom::dot(d, d).max(1e-12);
            let u = (geom::dot(geom::sub(pos[i], t.a), d) / l2).clamp(0.0, 1.0);
            let q = geom::add(t.a, geom::scale(d, u));
            if geom::len(geom::sub(pos[i], q)) <= tol {
                // a pad is one node; a trace end joins nodes that touch that end
                let end = if t.pad || u <= 1e-6 {
                    Some(t.a)
                } else if u >= 1.0 - 1e-6 {
                    Some(t.b)
                } else {
                    None
                };
                if let Some(e) = end {
                    let a = node_id[&key(e, t.layer)];
                    let (ra, rb) = (find(&mut parent, a), find(&mut parent, i));
                    if ra != rb {
                        parent[rb] = ra;
                    }
                }
            }
        }
    }
    // vias of the net connect all track ends inside their pad
    for v in board.vias.iter().filter(|v| v.net == net) {
        let r = v.size / 2.0 + 0.03;
        let span: u64 = (v.from..=v.to).fold(0, |m, l| m | 1u64 << l.min(63));
        let inside: Vec<usize> = (0..n).filter(|&i| node_layers[i] & span != 0 && geom::len(geom::sub(pos[i], v.at)) <= r).collect();
        for w in inside.windows(2) {
            let (ra, rb) = (find(&mut parent, w[0]), find(&mut parent, w[1]));
            if ra != rb {
                parent[rb] = ra;
            }
        }
    }
    // plated through-hole pads likewise
    for p in board.pads.iter().filter(|p| p.net == net && p.drill > 0.0 && p.layers.len() > 1) {
        let r = p.size[0].min(p.size[1]) / 2.0 + 0.03;
        let span: u64 = p.layers.iter().fold(0, |m, &l| m | 1u64 << l.min(63));
        let inside: Vec<usize> = (0..n).filter(|&i| node_layers[i] & span != 0 && geom::len(geom::sub(pos[i], p.at)) <= r).collect();
        for w in inside.windows(2) {
            let (ra, rb) = (find(&mut parent, w[0]), find(&mut parent, w[1]));
            if ra != rb {
                parent[rb] = ra;
            }
        }
    }
    let edges: Vec<(usize, usize, usize, f64)> = edges.into_iter().map(|(t, a, b, l)| (t, find(&mut parent, a), find(&mut parent, b), l)).collect();
    let mut adj: HashMap<usize, Vec<(usize, usize)>> = HashMap::new(); // node -> (edge idx, other node)
    for (ei, &(_, a, b, _)) in edges.iter().enumerate() {
        if a == b {
            continue;
        }
        adj.entry(a).or_default().push((ei, b));
        adj.entry(b).or_default().push((ei, a));
    }
    if adj.values().any(|v| v.len() > 2) {
        warnings.push("Netz verzweigt (Stichleitungen, Testpunkte): gerechnet wird der längste Pfad, Abzweige fehlen".into());
    }
    // farthest node by path length (Dijkstra on a tree = DFS)
    let farthest = |start: usize| -> (usize, HashMap<usize, (f64, Option<(usize, usize)>)>) {
        let mut dist: HashMap<usize, (f64, Option<(usize, usize)>)> = HashMap::new();
        dist.insert(start, (0.0, None));
        let mut stack = vec![start];
        while let Some(u) = stack.pop() {
            let du = dist[&u].0;
            for &(ei, v) in adj.get(&u).map(|x| x.as_slice()).unwrap_or(&[]) {
                if !dist.contains_key(&v) {
                    dist.insert(v, (du + edges[ei].3, Some((ei, u))));
                    stack.push(v);
                }
            }
        }
        let far = dist.iter().max_by(|a, b| a.1 .0.partial_cmp(&b.1 .0).unwrap()).map(|x| *x.0).unwrap_or(start);
        (far, dist)
    };
    let start = *adj.keys().next().unwrap_or(&0);
    let (a, _) = farthest(start);
    let (b, dist) = farthest(a);
    let comps = {
        // count connected components among edges to warn about gaps
        let mut seen = std::collections::HashSet::new();
        let mut c = 0;
        for &k in adj.keys() {
            if seen.contains(&k) {
                continue;
            }
            c += 1;
            let mut st = vec![k];
            while let Some(u) = st.pop() {
                if !seen.insert(u) {
                    continue;
                }
                for &(_, v) in adj.get(&u).map(|x| x.as_slice()).unwrap_or(&[]) {
                    st.push(v);
                }
            }
        }
        c
    };
    if comps > 1 {
        warnings.push(format!("Netz in {comps} Teile zerfallen (z. B. über Bauteile verbunden): gerechnet wird das längste Teilstück"));
    }
    // walk back from b to a
    let mut order: Vec<(usize, usize)> = Vec::new(); // (edge, from-node)
    let mut cur = b;
    while let Some((_, Some((ei, prev)))) = dist.get(&cur).copied() {
        order.push((ei, prev));
        cur = prev;
    }
    order.reverse();
    let mut out = Vec::new();
    for (ei, from) in order {
        let (ti, ea, _, _) = edges[ei];
        let fwd = ea == from;
        let mut ps = pieces_of[&ti].clone();
        ps.sort_by(|x, y| an.pieces[x * 4 + 1].partial_cmp(&an.pieces[y * 4 + 1]).unwrap());
        if !fwd {
            ps.reverse();
        }
        for p in ps {
            out.push((p, !fwd));
        }
    }
    out
}

pub fn channel(board: &Board, an: &Analysis, net: u32, field: &HashMap<String, FieldResult>, target: Option<(f64, bool)>, rp: &RfParams) -> Result<Channel, String> {
    let su = board.stackup();
    let mut warnings = Vec::new();
    let partner = crate::extract::diff_partners(&board.nets).get(net as usize).copied().flatten();
    let diff = partner.is_some() && target.map(|t| t.1).unwrap_or(true);
    let path = net_path(board, an, net, &mut warnings);
    if path.is_empty() {
        return Err("Netz hat keine Leiterbahnen".into());
    }
    // segments, merging equal cross-sections, vias at layer changes
    let mut segs: Vec<Segment> = Vec::new();
    let mut models: Vec<Option<LineModel>> = Vec::new();
    let mut model_cache: HashMap<u32, Option<LineModel>> = HashMap::new();
    let mut prev_layer: Option<(u16, P)> = None;
    let mut prev_xs: Option<u32> = None;
    let mut via_models: Vec<Option<crate::via::ViaModel>> = Vec::new();
    let mut via_info: Vec<crate::via::ViaInfo> = Vec::new();
    let refs = |xi: Option<u32>| -> Vec<u16> {
        xi.map(|i| {
            let x = &an.cross_sections[i as usize];
            [x.ref_up, x.ref_dn].into_iter().flatten().collect()
        })
        .unwrap_or_default()
    };
    let mut field_len = 0.0;
    let mut total_len = 0.0;
    for &(pi, rev) in &path {
        let pc = &an.pieces[pi * 4..pi * 4 + 4];
        let tr = &board.tracks[pc[0] as usize];
        let xi = pc[3] as u32;
        let len = geom::len(geom::sub(tr.b, tr.a)) * (pc[2] - pc[1]);
        let start = geom::lerp(tr.a, tr.b, if rev { pc[2] } else { pc[1] });
        if let Some((pl, ppos)) = prev_layer {
            if pl != tr.layer && rp.vias {
                // nearest via of the net
                let v = board.vias.iter().filter(|v| v.net == net).min_by(|a, b| {
                    geom::len(geom::sub(a.at, ppos)).partial_cmp(&geom::len(geom::sub(b.at, ppos))).unwrap()
                });
                let fallback_via;
                let v = match v {
                    Some(v) if geom::len(geom::sub(v.at, ppos)) < 1.0 => v,
                    _ => {
                        // a plated through-hole pad (connector pin) acts as the via
                        let th = board.pads.iter().filter(|p| p.net == net && p.drill > 0.0 && p.layers.len() > 1).min_by(|a, b| {
                            geom::len(geom::sub(a.at, ppos)).partial_cmp(&geom::len(geom::sub(b.at, ppos))).unwrap()
                        });
                        let th = th.filter(|p| geom::len(geom::sub(p.at, ppos)) < p.size[0].max(p.size[1]) + 0.5);
                        fallback_via = match th {
                            Some(p) => crate::board::Via {
                                at: p.at,
                                size: p.size[0].min(p.size[1]),
                                drill: p.drill,
                                from: *p.layers.iter().min().unwrap(),
                                to: *p.layers.iter().max().unwrap(),
                                net,
                                backdrill: None,
                            },
                            None => crate::board::Via { at: ppos, size: 0.45, drill: 0.2, from: 0, to: (su.n() - 1) as u16, net, backdrill: None },
                        };
                        if th.is_none() {
                            warnings.push(format!("Lagenwechsel bei ({:.2}, {:.2}) mm ohne Via in der Nähe: Standard-Via 0,2/0,45 mm angenommen", ppos[0], ppos[1]));
                        }
                        &fallback_via
                    }
                };
                let ctx = crate::via::ViaCtx {
                    board,
                    net,
                    partner: if diff { partner } else { None },
                    refs_in: refs(prev_xs),
                    refs_out: refs(Some(xi)),
                    backdrill: rp.backdrill,
                };
                let (vm, info) = crate::via::build(&ctx, v, pl, tr.layer);
                let (mut lt, mut ct) = (0.0, 0.0);
                for s in &vm.barrel {
                    let vv = C0 / s.er.sqrt();
                    lt += s.z / vv * s.len;
                    ct += s.len / (s.z * vv);
                }
                let zv = if diff { info.z_via / 2.0 } else { info.z_via };
                segs.push(Segment { kind: "via", len_mm: info.barrel_mm, layer: tr.layer, xs: None, z: Some(zv), via_lc: Some((lt * 1e9, ct * 1e12)), source: "via-model" });
                models.push(None);
                via_models.resize(segs.len() - 1, None);
                via_models.push(Some(vm));
                via_info.push(info);
            }
        }
        prev_layer = Some((tr.layer, geom::lerp(tr.a, tr.b, if rev { pc[1] } else { pc[2] })));
        prev_xs = Some(xi);
        let _ = start;
        let xs = &an.cross_sections[xi as usize];
        let model = model_cache
            .entry(xi)
            .or_insert_with(|| {
                let fr = xs.geom.and_then(|g| field.get(&an.geometries[g as usize].id));
                let odd = diff && xs.zdiff.is_some();
                line_model(su, xs, fr, odd, rp).or_else(|| line_model(su, xs, fr, false, rp))
            })
            .clone();
        total_len += len;
        if model.as_ref().is_some_and(|m| m.source == "field") {
            field_len += len;
        }
        let z = model.as_ref().map(|m| m.z0);
        if let (Some(last), true) = (segs.last_mut(), true) {
            if last.kind == "line" && last.xs == Some(xi) {
                last.len_mm += len;
                continue;
            }
        }
        segs.push(Segment { kind: "line", len_mm: len, layer: tr.layer, xs: Some(xi), z, via_lc: None, source: model.as_ref().map(|m| m.source).unwrap_or("none") });
        models.push(model);
    }
    if models.iter().zip(&segs).any(|(m, s)| s.kind == "line" && m.is_none()) {
        warnings.push("Abschnitte ohne Referenzebene: als 1,5-fache Nennimpedanz angenähert".into());
    }
    // reference impedance
    let z_ref = rp.z_ref.unwrap_or_else(|| match target {
        Some((z, true)) if diff => z / 2.0,
        Some((z, false)) => z,
        _ => {
            if diff {
                50.0
            } else {
                50.0
            }
        }
    });
    // fallback model for sections without reference: crude high-Z line
    let fallback = LineModel {
        z0: z_ref * 1.5,
        eeff0: 2.5,
        l_ext: z_ref * 1.5 * 2.5f64.sqrt() / C0,
        c0: 2.5f64.sqrt() / (z_ref * 1.5 * C0),
        r_dc: 0.0,
        k_ac: 0.0,
        er: 4.0,
        tand: 0.02,
        q_fill: 0.5,
        tand_eff0: None,
        ms_h: None,
        source: "none",
    };

    // frequency sweep
    let n = rp.npts.max(10);
    let df = rp.fmax / n as f64;
    let freq: Vec<f64> = (1..=n).map(|k| k as f64 * df).collect();
    let mut s11 = Vec::with_capacity(n);
    let mut s21 = Vec::with_capacity(n);
    for &f in &freq {
        let w = 2.0 * PI * f;
        let mut m = Abcd::identity();
        for (i, (s, md)) in segs.iter().zip(&models).enumerate() {
            let e = if s.kind == "via" {
                match via_models.get(i).and_then(|v| v.as_ref()) {
                    Some(vm) => vm.abcd(f, rp.sigma),
                    None => Abcd::identity(),
                }
            } else {
                let lm = md.as_ref().unwrap_or(&fallback);
                let p = lm.rlgc(f, rp);
                Abcd::line(p.r, p.l, p.g, p.c, w, s.len_mm * 1e-3)
            };
            m = m.mul(&e);
        }
        let (a, b, _) = m.s_params(z_ref);
        s11.push(a);
        s21.push(b);
    }
    let db = |x: Cx| 20.0 * x.abs().max(1e-12).log10();
    let s21_db: Vec<f64> = s21.iter().map(|&x| db(x)).collect();
    let s11_db: Vec<f64> = s11.iter().map(|&x| db(x)).collect();
    let il_at = |fghz: f64| -> Option<(f64, f64)> {
        if fghz * 1e9 > rp.fmax {
            return None;
        }
        let k = ((fghz * 1e9 / df).round() as usize).clamp(1, n) - 1;
        Some((fghz, s21_db[k]))
    };
    let il_marks: Vec<(f64, f64)> = [1.0, 2.5, 4.0, 5.0, 8.0, 12.5, 16.0, 25.0].iter().filter_map(|&f| il_at(f)).collect();

    // delay (lossless, static)
    let mut delay = 0.0;
    for (i, (s, md)) in segs.iter().zip(&models).enumerate() {
        if s.kind == "line" {
            let lm = md.as_ref().unwrap_or(&fallback);
            delay += s.len_mm * 1e-3 * (lm.l_ext * lm.c0).sqrt();
        } else if let Some(Some(vm)) = via_models.get(i) {
            delay += vm.delay();
        }
    }
    let total_len = total_len + via_info.iter().map(|v| v.barrel_mm).sum::<f64>();
    for v in &via_info {
        if let Some(fr) = v.stub_f_ghz {
            if fr * 1e9 < rp.fmax {
                warnings.push(format!(
                    "Via bei ({:.2}, {:.2}) mm: Stub {:.2} mm, Viertelwellen-Resonanz {:.1} GHz liegt im betrachteten Band",
                    v.at[0], v.at[1], v.stub_mm, fr
                ));
            }
        }
        match v.return_kind {
            "missing" => warnings.push(format!("Via bei ({:.2}, {:.2}) mm: Bezugsebene wechselt, kein Stitching-Via innerhalb 5 mm", v.at[0], v.at[1])),
            "decap" => warnings.push(format!("Via bei ({:.2}, {:.2}) mm: Bezug wechselt auf ein anderes Netz, Rückstrom nur über Kondensatoren", v.at[0], v.at[1])),
            _ => {}
        }
    }
    // loss per inch of the longest segment model at a few frequencies
    let mut loss_db_per_in_at = Vec::new();
    if let Some((s, md)) = segs.iter().zip(&models).filter(|(s, m)| s.kind == "line" && m.is_some()).max_by(|a, b| a.0.len_mm.partial_cmp(&b.0.len_mm).unwrap()) {
        let lm = md.as_ref().unwrap();
        let _ = s;
        for fg in [1.0, 4.0, 8.0, 16.0] {
            if fg * 1e9 > rp.fmax {
                continue;
            }
            let p = lm.rlgc(fg * 1e9, rp);
            let w = 2.0 * PI * fg * 1e9;
            let gamma = (Cx::new(p.r, w * p.l) * Cx::new(p.g, w * p.c)).sqrt();
            let zc = (p.l / p.c).sqrt();
            let a_c = p.r / (2.0 * zc);
            let a_d = p.g * zc / 2.0;
            let k = 8.686 * 0.0254;
            let _ = gamma;
            loss_db_per_in_at.push((fg, a_c * k, a_d * k));
        }
    }

    // TDR: step response from S11 with a Gaussian edge of the given rise time
    let (tdr_t_ps, tdr_z, tdr_x_mm) = tdr(&freq, &s11, z_ref, rp.rise, (2.0 * delay * 1.3 + 4.0 * rp.rise).max(200e-12), delay, total_len);

    Ok(Channel {
        net,
        name: board.net_name(net).to_string(),
        diff,
        partner,
        z_ref,
        length_mm: total_len,
        delay_ps: delay * 1e12,
        vias: segs.iter().filter(|s| s.kind == "via").count(),
        segments: segs,
        freq,
        s21_db,
        s11_db,
        il_marks,
        tdr_t_ps,
        tdr_z,
        tdr_x_mm,
        loss_db_per_in_at,
        via_info,
        field_share: if total_len > 0.0 { field_len / total_len } else { 0.0 },
        warnings,
        s11,
        s21,
    })
}

/// Step response of S11 on a uniform grid f_k = k·df (k ≥ 1), with a Gaussian
/// filter whose 10-90 % rise time is `rise`. Returns (t ps, Z Ω, x mm).
pub fn tdr(freq: &[f64], s11: &[Cx], z_ref: f64, rise: f64, t_max: f64, delay: f64, length_mm: f64) -> (Vec<f64>, Vec<f64>, Vec<f64>) {
    if freq.is_empty() {
        return (vec![], vec![], vec![]);
    }
    let df = freq[0];
    // Gaussian: 10-90 % rise = 0.3394/f_3dB ; H = exp(-ln2/2 (f/f3)^2) gives -3 dB at f3
    let f3 = 0.3394 / rise;
    // S11 at DC: extrapolate from the first point (real part)
    let s0 = s11[0].re;
    let nt = 600usize;
    let dt = t_max / nt as f64;
    // antiderivative of the filtered impulse response
    //   F(t) = df·[S0·t + 2·Σ Re(S_k H_k e^{jω t}/(jω))]
    // step(t) = F(t) − F(t0) with t0 before the Gaussian edge starts
    let terms: Vec<(f64, Cx)> = freq
        .iter()
        .zip(s11)
        .map(|(&f, &s)| (2.0 * PI * f, s.scale((-(std::f64::consts::LN_2 / 2.0) * (f / f3).powi(2)).exp())))
        .filter(|(w, _)| (-(std::f64::consts::LN_2 / 2.0) * (w / (2.0 * PI) / f3).powi(2)).exp() > 1e-6)
        .collect();
    let big_f = |t: f64| -> f64 {
        let mut acc = s0 * t;
        for &(w, sh) in &terms {
            let e = Cx::new((w * t).cos(), (w * t).sin());
            acc += 2.0 * (sh * e / Cx::new(0.0, w)).re;
        }
        acc * df
    };
    let t0 = -3.0 * rise;
    let f0 = big_f(t0);
    let mut ts = Vec::with_capacity(nt);
    let mut zs = Vec::with_capacity(nt);
    let mut xs = Vec::with_capacity(nt);
    for i in 0..nt {
        let t = i as f64 * dt;
        let rho = (big_f(t) - f0).clamp(-0.99, 0.99);
        ts.push(t * 1e12);
        zs.push(z_ref * (1.0 + rho) / (1.0 - rho));
        xs.push(if delay > 0.0 { t / (2.0 * delay) * length_mm } else { 0.0 });
    }
    (ts, zs, xs)
}

/// Touchstone (.s2p, RI) of a channel.
pub fn touchstone(ch: &Channel) -> String {
    let mut s = String::new();
    s.push_str(&format!("! kisi channel {}{}\n", ch.name, if ch.diff { " (differential mode, per-line odd-mode model)" } else { "" }));
    s.push_str(&format!("! length {:.2} mm, {} vias, reference {:.1} ohm\n", ch.length_mm, ch.vias, ch.z_ref));
    s.push_str(&format!("# HZ S RI R {:.3}\n", ch.z_ref));
    for (k, &f) in ch.freq.iter().enumerate() {
        let (a, b) = (ch.s11[k], ch.s21[k]);
        s.push_str(&format!("{:.6e} {:.6e} {:.6e} {:.6e} {:.6e} {:.6e} {:.6e} {:.6e} {:.6e}\n", f, a.re, a.im, b.re, b.im, b.re, b.im, a.re, a.im));
    }
    s
}

// --- crosstalk ----------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct Aggressor {
    pub net: u32,
    pub name: String,
    pub layer: u16,
    pub coupled_mm: f64,
    pub gap_min_mm: f64,
    pub gap_mean_mm: f64,
    /// backward coupling coefficient (saturated NEXT, fraction)
    pub kb: f64,
    /// NEXT and FEXT at the given rise time (fraction of aggressor swing)
    pub next: f64,
    pub fext: f64,
    pub source: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct Crosstalk {
    pub net: u32,
    pub rise_ps: f64,
    pub aggressors: Vec<Aggressor>,
    /// geometries (id, text) a field solver can refine
    pub geoms: Vec<(String, String)>,
    pub next_total: f64,
    pub fext_total: f64,
}

/// Coupling of two lines from their modal parameters:
/// (Kb, Kf in s/m) with FEXT = Kf·len/tr.
pub fn coupling_from_modes(ze: f64, zo: f64, ee: f64, eo: f64) -> (f64, f64) {
    let ve = C0 / ee.sqrt();
    let vo = C0 / eo.sqrt();
    let (lp, lm_) = (ze / ve, zo / vo); // L+Lm, L-Lm
    let (cm_, cp) = (1.0 / (ze * ve), 1.0 / (zo * vo)); // C-Cm, C+Cm
    let l = (lp + lm_) / 2.0;
    let lm = (lp - lm_) / 2.0;
    let c = (cp + cm_) / 2.0;
    let cm = (cp - cm_) / 2.0;
    coupling_from_lc(l, lm, c, cm)
}

pub fn coupling_from_lc(l: f64, lm: f64, c: f64, cm: f64) -> (f64, f64) {
    let kb = 0.25 * (lm / l + cm / c);
    let kf = 0.5 * (cm / c - lm / l) * (l * c).sqrt();
    (kb, kf)
}

pub fn crosstalk(board: &Board, an: &Analysis, net: u32, field: &HashMap<String, FieldResult>, rp: &RfParams) -> Crosstalk {
    let su = board.stackup();
    let partner = crate::extract::diff_partners(&board.nets).get(net as usize).copied().flatten();
    let step = 0.25;
    // index of other-net tracks (grid 1 mm)
    let mut grid: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
    for (ti, t) in board.tracks.iter().enumerate() {
        // supply / ground nets are quiet: not counted as aggressors
        if t.net == net || Some(t.net) == partner || t.net == 0 || board.plane_net.get(t.net as usize).copied().unwrap_or(false) {
            continue;
        }
        let (x0, x1) = (t.a[0].min(t.b[0]).floor() as i64, t.a[0].max(t.b[0]).floor() as i64);
        let (y0, y1) = (t.a[1].min(t.b[1]).floor() as i64, t.a[1].max(t.b[1]).floor() as i64);
        for x in x0..=x1 {
            for y in y0..=y1 {
                grid.entry((x, y)).or_default().push(ti);
            }
        }
    }
    // (aggressor net, xs, aggressor width q, gap q, side) -> length
    let mut acc: HashMap<(u32, u32, i64, i64, bool), f64> = HashMap::new();
    let mut per_net: HashMap<u32, (f64, f64, f64, u16)> = HashMap::new(); // len, gap min, gap*len, layer
    for pc in an.pieces.chunks_exact(4) {
        let tr = &board.tracks[pc[0] as usize];
        if tr.net != net || tr.pad {
            continue;
        }
        let xs = &an.cross_sections[pc[3] as usize];
        let hmin = [xs.h_up, xs.h_dn].into_iter().flatten().fold(f64::INFINITY, f64::min);
        if !hmin.is_finite() {
            continue;
        }
        // beyond ~5 h the backward coupling is below ~1 %
        let gmax = (5.0 * hmin).max(3.0 * tr.width).min(2.0);
        let d = geom::sub(tr.b, tr.a);
        let len = geom::len(d);
        if len < 1e-6 {
            continue;
        }
        let dir = geom::scale(d, 1.0 / len);
        let nrm = [-dir[1], dir[0]];
        let l0 = len * pc[1];
        let l1 = len * pc[2];
        let ns = (((l1 - l0) / step).ceil() as usize).max(1);
        let seg = (l1 - l0) / ns as f64;
        for k in 0..ns {
            let p = geom::add(tr.a, geom::scale(dir, l0 + (k as f64 + 0.5) * seg));
            let mut best: HashMap<u32, (f64, f64, bool)> = HashMap::new(); // net -> (gap, width, side)
            let r = (gmax + 1.0).ceil() as i64;
            let (cx, cy) = (p[0].floor() as i64, p[1].floor() as i64);
            for x in cx - r..=cx + r {
                for y in cy - r..=cy + r {
                    for &oi in grid.get(&(x, y)).map(|v| v.as_slice()).unwrap_or(&[]) {
                        let o = &board.tracks[oi];
                        if o.layer != tr.layer {
                            continue;
                        }
                        let od = geom::sub(o.b, o.a);
                        let ol = geom::len(od);
                        if ol < 1e-6 {
                            continue;
                        }
                        let odir = geom::scale(od, 1.0 / ol);
                        if geom::cross(dir, odir).abs() > 0.26 {
                            continue;
                        }
                        let ap = geom::sub(p, o.a);
                        let along = geom::dot(ap, odir);
                        if along < 0.0 || along > ol {
                            continue;
                        }
                        let dist = geom::cross(odir, ap).abs();
                        let gap = dist - (tr.width + o.width) / 2.0;
                        if gap <= 0.0 || gap > gmax {
                            continue;
                        }
                        let side = geom::dot(geom::sub(geom::add(o.a, geom::scale(odir, along)), p), nrm) > 0.0;
                        let e = best.entry(o.net).or_insert((f64::INFINITY, o.width, side));
                        if gap < e.0 {
                            *e = (gap, o.width, side);
                        }
                    }
                }
            }
            for (onet, (gap, ow, side)) in best {
                let gq = (gap / 0.01).round() as i64;
                let wq = (ow / 0.001).round() as i64;
                *acc.entry((onet, pc[3] as u32, wq, gq, side)).or_default() += seg;
                let e = per_net.entry(onet).or_insert((0.0, f64::INFINITY, 0.0, tr.layer));
                e.0 += seg;
                e.1 = e.1.min(gap);
                e.2 += gap * seg;
            }
        }
    }
    // coupling per key
    let mut geoms: Vec<(String, String)> = Vec::new();
    let mut agg: HashMap<u32, (f64, f64, &'static str)> = HashMap::new(); // next, fext, source
    for (&(onet, xi, wq, gq, side), &len) in &acc {
        let xs = &an.cross_sections[xi as usize];
        let gap = gq as f64 * 0.01;
        let ow = wq as f64 * 0.001;
        // field geometry: victim with the aggressor as partner
        let desc = XsDesc {
            layer: xs.layer as usize,
            w: xs.w,
            ref_up: xs.ref_up.map(|x| x as usize),
            ref_dn: xs.ref_dn.map(|x| x as usize),
            s_left: if side { None } else { xs.s_left },
            s_right: if side { xs.s_right } else { None },
            partner: Some((gap, ow, true)),
            mask: if xs.mask { su.mask_top.as_ref().or(su.mask_bottom.as_ref()).map(|m| (m.thickness, m.epsilon_r)) } else { None },
            planes: xs.planes.iter().map(|&(k, l, r)| PlaneCut { layer: k as usize, left: l, right: r }).collect(),
        };
        let gtext = xsgeom::build(su, &desc);
        let gid = gtext.as_ref().map(|t| xsgeom::geom_id(t));
        let fr = gid.as_ref().and_then(|id| field.get(id));
        let (kb, kf, src) = match fr {
            Some(f) if f.mat_l.is_some() && f.mat_c.is_some() => {
                let (ml, mc) = (f.mat_l.unwrap(), f.mat_c.unwrap());
                let (kb, kf) = coupling_from_lc(ml[0], ml[1], mc[0], -mc[1]);
                (kb, kf, "field")
            }
            Some(FieldResult { zodd: Some(zo), zeven: Some(ze), eeff_odd: Some(eo), eeff_even: Some(ee), .. }) => {
                let (kb, kf) = coupling_from_modes(*ze, *zo, *ee, *eo);
                (kb, kf, "field")
            }
            _ => {
                let (er, _) = xs_material(su, xs);
                let (ze, zo, ee, eo) = match xs.kind {
                    Kind::Stripline => {
                        let b = xs.h_up.unwrap_or(0.1) + xs.h_dn.unwrap_or(0.1) + xs.t;
                        let (ze, zo) = analytic::cohn_coupled(xs.w, gap, b, er);
                        (ze, zo, er, er)
                    }
                    _ => {
                        let h = xs.h_dn.or(xs.h_up).unwrap_or(0.1);
                        analytic::kirschning_jansen(xs.w, gap, h, er)
                    }
                };
                let (kb, kf) = coupling_from_modes(ze, zo, ee, eo);
                (kb, kf, "formula")
            }
        };
        if let (Some(id), Some(t)) = (gid, gtext) {
            if fr.is_none() && !geoms.iter().any(|g| g.0 == id) {
                geoms.push((id, t));
            }
        }
        let eeff = xs.eeff.unwrap_or(3.0);
        let v = C0 / eeff.sqrt();
        let lm = len * 1e-3;
        let next = kb * (2.0 * lm / (v * rp.rise)).min(1.0);
        let fext = kf.abs() * lm / rp.rise;
        let e = agg.entry(onet).or_insert((0.0, 0.0, "formula"));
        e.0 = e.0.max(next);
        e.1 += fext;
        if src == "field" {
            e.2 = "field";
        }
    }
    let mut aggressors: Vec<Aggressor> = agg
        .into_iter()
        .map(|(onet, (next, fext, src))| {
            let pn = per_net[&onet];
            let kb_sat = next; // already limited by length
            Aggressor {
                net: onet,
                name: board.net_name(onet).to_string(),
                layer: pn.3,
                coupled_mm: pn.0,
                gap_min_mm: pn.1,
                gap_mean_mm: pn.2 / pn.0.max(1e-9),
                kb: kb_sat,
                next,
                fext,
                source: src,
            }
        })
        .filter(|a| a.coupled_mm >= 0.5)
        .collect();
    aggressors.sort_by(|a, b| (b.next + b.fext).partial_cmp(&(a.next + a.fext)).unwrap());
    let next_total = aggressors.iter().map(|a| a.next).fold(0.0, |x, y| x + y);
    let fext_total = aggressors.iter().map(|a| a.fext).fold(0.0, |x, y| x + y);
    Crosstalk { net, rise_ps: rp.rise * 1e12, aggressors, geoms, next_total, fext_total }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol * b.abs()
    }

    /// 50 Ω microstrip, w 3 mm, h 1.6 mm, t 35 µm, εr 4.5, tanδ 0.02, 1 GHz.
    /// HFSS (js_2d_fields test suite): R 3.26 Ω/m, G 13.86 mS/m, L 307 nH/m, C 123 pF/m.
    #[test]
    fn microstrip_rlgc_vs_hfss() {
        let (z, eeff) = analytic::microstrip(3.0, 1.6, 0.035, 4.5);
        let rp = RfParams { roughness: Roughness::Smooth, ..Default::default() };
        let la = l_air(Kind::Microstrip, 3.0, 0.035, 1.6, 0.0, None);
        let dn = 1e-3;
        let lb = l_air(Kind::Microstrip, 3.0 - 2.0 * dn, 0.035 - 2.0 * dn, 1.6 + 2.0 * dn, 0.0, None);
        let k_ac = (PI * MU0 / rp.sigma).sqrt() / MU0 * (lb - la) / (dn * 1e-3);
        let m = LineModel {
            z0: z,
            eeff0: eeff,
            l_ext: z * eeff.sqrt() / C0,
            c0: eeff.sqrt() / (z * C0),
            r_dc: 1.0 / (rp.sigma * 3e-3 * 35e-6),
            k_ac,
            er: 4.5,
            tand: 0.02,
            q_fill: (eeff - 1.0) / 3.5,
            tand_eff0: None,
            ms_h: Some(1.6e-3),
            source: "formula",
        };
        let p = m.rlgc(1e9, &rp);
        eprintln!("R {:.3} Ω/m  L {:.1} nH/m  G {:.2} mS/m  C {:.1} pF/m", p.r, p.l * 1e9, p.g * 1e3, p.c * 1e12);
        assert!(close(p.r, 3.26, 0.15), "R {}", p.r);
        assert!(close(p.l, 307e-9, 0.03), "L {}", p.l);
        assert!(close(p.c, 123e-12, 0.03), "C {}", p.c);
        assert!(close(p.g, 13.86e-3, 0.10), "G {}", p.g);
    }

    #[test]
    fn ds_model() {
        let ds = DjordjevicSarkar::fit(4.0, 0.02);
        let (e1, t1) = ds.at(1e9);
        assert!(close(e1, 4.0, 1e-9) && close(t1, 0.02, 1e-9));
        let (e10, t10) = ds.at(10e9);
        assert!(e10 < e1 && e10 > 3.8, "{e10}");
        assert!((t10 - 0.02).abs() < 0.003, "{t10}");
    }

    #[test]
    fn matched_and_mismatched_lines() {
        // lossless 50 Ω line in a 50 Ω system: |S21| = 1, S11 = 0
        let w = 2.0 * PI * 5e9;
        let l = 50.0 * 2.0f64.sqrt() / C0;
        let c = 2.0f64.sqrt() / (50.0 * C0);
        let m = Abcd::line(0.0, l, 0.0, c, w, 0.1);
        let (s11, s21, _) = m.s_params(50.0);
        assert!(s11.abs() < 1e-9 && (s21.abs() - 1.0).abs() < 1e-9);
        // quarter-wave 70.7 Ω transformer between 50 Ω and 100 Ω: matched
        let z1 = 50.0 * 2.0f64.sqrt();
        let lq = z1 / C0;
        let cq = 1.0 / (z1 * C0);
        let lam4 = C0 / 5e9 / 4.0;
        let m = Abcd::line(0.0, lq, 0.0, cq, w, lam4).mul(&Abcd::shunt(Cx::new(1.0 / 100.0, 0.0)));
        // input impedance = A/C for an open termination after the shunt load
        let zin = m.a / m.c;
        assert!((zin.re - 50.0).abs() < 1e-6 && zin.im.abs() < 1e-6, "{zin:?}");
    }

    #[test]
    fn tdr_of_step_in_impedance() {
        // 50 Ω reference, line of 75 Ω, 100 mm, open end far away (long)
        let n = 2000;
        let fmax = 50e9;
        let df = fmax / n as f64;
        let freq: Vec<f64> = (1..=n).map(|k| k as f64 * df).collect();
        let z = 75.0;
        let l = z * 2.0f64.sqrt() / C0;
        let c = 2.0f64.sqrt() / (z * C0);
        let s11: Vec<Cx> = freq
            .iter()
            .map(|&f| {
                let w = 2.0 * PI * f;
                // terminated in 75 Ω: no reflection from the end
                let m = Abcd::line(0.0, l, 0.0, c, w, 0.1).mul(&Abcd::shunt(Cx::new(1.0 / z, 0.0)));
                let zin = m.a / m.c;
                (zin - Cx::new(50.0, 0.0)) / (zin + Cx::new(50.0, 0.0))
            })
            .collect();
        let (t, zs, _) = tdr(&freq, &s11, 50.0, 20e-12, 400e-12, 0.0, 0.0);
        let k = t.iter().position(|&x| x > 200.0).unwrap();
        assert!((zs[k] - 75.0).abs() < 1.5, "Z at 200 ps = {}", zs[k]);
    }

    #[test]
    fn coupling_symmetric() {
        // homogeneous stripline: no FEXT
        let (ze, zo) = analytic::cohn_coupled(0.15, 0.15, 0.4, 4.0);
        let (kb, kf) = coupling_from_modes(ze, zo, 4.0, 4.0);
        assert!(kb > 0.0 && kb < 0.2, "{kb}");
        assert!(kf.abs() < 1e-15);
        // microstrip: FEXT negative-coupling term non-zero
        let (ze, zo, ee, eo) = analytic::kirschning_jansen(0.2, 0.2, 0.1, 4.0);
        let (kb, kf) = coupling_from_modes(ze, zo, ee, eo);
        assert!(kb > 0.0 && kf.abs() > 0.0);
    }
}

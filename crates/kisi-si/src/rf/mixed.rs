//! Differential pair as a 4-port: both lines of the pair along their own paths, coupled
//! sections from the modal (odd / even) line models, mixed-mode S-parameters
//! (Sdd, Scc, Scd, Sdc), differential and common-mode TDR, skew and Touchstone (.s4p).
//!
//! Port order of the 4-port: 1 = P near, 2 = N near, 3 = P far, 4 = N far ("near" is the
//! start of the P path; the N path starts at the end next to it). Every port uses the
//! single-ended reference z_ref, so Zdiff,ref = 2·z_ref and Zcomm,ref = z_ref/2.
//!
//! Coupled sections: T = P·blockdiag(T_even, T_odd)·P⁻¹ with the modal transform
//! V1 = Ve + Vo, V2 = Ve − Vo, I1 = Ie + Io, I2 = Ie − Io. Uncoupled sections, lumped parts
//! and branch stubs act on one line only. A via pair is coupled (odd mode with the partner
//! via, even mode as the single via) when both vias have the same barrel length, otherwise
//! each via on its own line.

#[allow(unused_imports)]
use super::*;
use crate::board::Board;
use crate::extract::{Analysis, FieldResult};
use crate::geom::P;
use serde::Serialize;
use std::collections::HashMap;
use std::f64::consts::PI;

type M4 = [[Cx; 4]; 4];
type M2 = [[Cx; 2]; 2];

fn z() -> Cx {
    Cx::new(0.0, 0.0)
}
fn one() -> Cx {
    Cx::new(1.0, 0.0)
}
fn m4_id() -> M4 {
    let mut m = [[z(); 4]; 4];
    for (i, r) in m.iter_mut().enumerate() {
        r[i] = one();
    }
    m
}
fn m4_mul(a: &M4, b: &M4) -> M4 {
    let mut m = [[z(); 4]; 4];
    for i in 0..4 {
        for j in 0..4 {
            let mut s = z();
            for k in 0..4 {
                s = s + a[i][k] * b[k][j];
            }
            m[i][j] = s;
        }
    }
    m
}
fn m2_mul(a: &M2, b: &M2) -> M2 {
    [[a[0][0] * b[0][0] + a[0][1] * b[1][0], a[0][0] * b[0][1] + a[0][1] * b[1][1]], [a[1][0] * b[0][0] + a[1][1] * b[1][0], a[1][0] * b[0][1] + a[1][1] * b[1][1]]]
}
fn m2_inv(a: &M2) -> M2 {
    let det = a[0][0] * a[1][1] - a[0][1] * a[1][0];
    let k = det.inv();
    [[a[1][1] * k, (z() - a[0][1]) * k], [(z() - a[1][0]) * k, a[0][0] * k]]
}
fn m2_add(a: &M2, b: &M2, sb: f64) -> M2 {
    [[a[0][0] + b[0][0].scale(sb), a[0][1] + b[0][1].scale(sb)], [a[1][0] + b[1][0].scale(sb), a[1][1] + b[1][1].scale(sb)]]
}
fn m2_scale(a: &M2, k: f64) -> M2 {
    [[a[0][0].scale(k), a[0][1].scale(k)], [a[1][0].scale(k), a[1][1].scale(k)]]
}

/// Two independent 2-ports on line 1 and line 2 (state order V1, V2, I1, I2).
pub fn sep(a1: &Abcd, a2: &Abcd) -> M4 {
    let mut m = [[z(); 4]; 4];
    m[0][0] = a1.a;
    m[0][2] = a1.b;
    m[2][0] = a1.c;
    m[2][2] = a1.d;
    m[1][1] = a2.a;
    m[1][3] = a2.b;
    m[3][1] = a2.c;
    m[3][3] = a2.d;
    m
}

/// Symmetric coupled 2-line element from its even- and odd-mode per-line cascades.
pub fn coupled(even: &Abcd, odd: &Abcd) -> M4 {
    // P = blockdiag(K, K), P⁻¹ = blockdiag(K/2, K/2), K = [[1, 1], [1, −1]]
    let mut p = [[z(); 4]; 4];
    let mut pi = [[z(); 4]; 4];
    for b in [0usize, 2] {
        p[b][b] = one();
        p[b][b + 1] = one();
        p[b + 1][b] = one();
        p[b + 1][b + 1] = Cx::new(-1.0, 0.0);
        pi[b][b] = Cx::new(0.5, 0.0);
        pi[b][b + 1] = Cx::new(0.5, 0.0);
        pi[b + 1][b] = Cx::new(0.5, 0.0);
        pi[b + 1][b + 1] = Cx::new(-0.5, 0.0);
    }
    m4_mul(&m4_mul(&p, &sep(even, odd)), &pi)
}

/// S-matrix (port order 1 = line 1 near, 2 = line 2 near, 3 = line 1 far, 4 = line 2 far)
/// of a 4-port cascade matrix, reference z0 at every port.
pub fn s_of(t: &M4, z0: f64) -> M4 {
    let blk = |r: usize, c: usize| -> M2 { [[t[r][c], t[r][c + 1]], [t[r + 1][c], t[r + 1][c + 1]]] };
    let (a, b, c, d) = (blk(0, 0), blk(0, 2), blk(2, 0), blk(2, 2));
    let bz = m2_scale(&b, 1.0 / z0);
    let cz = m2_scale(&c, z0);
    // M1 = A + z0C + B/z0 + D, M2 = A + z0C − B/z0 − D, M3 = A − z0C − B/z0 + D, M4 = A − z0C + B/z0 − D
    let m1 = m2_add(&m2_add(&m2_add(&a, &cz, 1.0), &bz, 1.0), &d, 1.0);
    let m2 = m2_add(&m2_add(&m2_add(&a, &cz, 1.0), &bz, -1.0), &d, -1.0);
    let m3 = m2_add(&m2_add(&m2_add(&a, &cz, -1.0), &bz, -1.0), &d, 1.0);
    let m4 = m2_add(&m2_add(&m2_add(&a, &cz, -1.0), &bz, 1.0), &d, -1.0);
    let m1i = m2_inv(&m1);
    let s21 = m2_scale(&m1i, 2.0);
    let s22 = m2_scale(&m2_mul(&m1i, &m2), -1.0);
    let s11 = m2_mul(&m4, &m1i);
    let s12 = m2_scale(&m2_add(&m3, &m2_mul(&m2_mul(&m4, &m1i), &m2), -1.0), 0.5);
    let mut s = [[z(); 4]; 4];
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

/// Mixed-mode matrix [[Sdd, Sdc], [Scd, Scc]] (rows/columns d1, d2, c1, c2) of a 4-port S
/// in the port order above (P1, N1, P2, N2).
pub fn mixed_of(s: &M4) -> M4 {
    let k = std::f64::consts::FRAC_1_SQRT_2;
    let m: [[f64; 4]; 4] = [[k, -k, 0.0, 0.0], [0.0, 0.0, k, -k], [k, k, 0.0, 0.0], [0.0, 0.0, k, k]];
    // M·S·Mᵀ (M orthogonal)
    let mut out = [[z(); 4]; 4];
    for i in 0..4 {
        for j in 0..4 {
            let mut acc = z();
            for a in 0..4 {
                if m[i][a] == 0.0 {
                    continue;
                }
                for b in 0..4 {
                    if m[j][b] != 0.0 {
                        acc = acc + s[a][b].scale(m[i][a] * m[j][b]);
                    }
                }
            }
            out[i][j] = acc;
        }
    }
    out
}

/// One element of the aligned pair cascade.
enum El {
    /// section `seg` of line 0 (P) or 1 (N), optionally with a shorter length (mm)
    Single { line: usize, seg: usize, len: Option<f64> },
    /// coupled run of `len` mm with the modal models of P's cross-section
    Coupled { len: f64, even: LineModel, odd: LineModel },
    /// via pair: odd model with the partner via, even model = single via (of P)
    ViaPair { p: usize },
}

#[derive(Clone, Copy, PartialEq)]
enum Tok {
    /// coupled line section on a layer
    C(u16),
    V,
    U,
}

/// A stretch of the pair where P and N differ (between two coupled runs).
#[derive(Debug, Clone, Serialize)]
pub struct Imbalance {
    /// along P's own path (mm from A)
    pub s_from_mm: f64,
    pub s_to_mm: f64,
    /// P − N: extra length (mm) and delay (ps)
    pub dl_mm: f64,
    pub dt_ps: f64,
    /// the vias of P and N differ here (each modelled on its own line)
    pub vias_differ: bool,
    /// on the board: middle of the stretch on the longer line
    pub at: P,
}

#[derive(Default)]
struct Region {
    sp0: f64,
    sn0: f64,
    lp: f64,
    ln: f64,
    dp: f64,
    dn: f64,
    vias: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct MixedMode {
    pub p_id: u32,
    pub n_id: u32,
    /// ends of each line (pad labels): A = start (next to the start of the asked net), B = end
    pub p_a: String,
    pub p_b: String,
    pub n_a: String,
    pub n_b: String,
    pub p_net: String,
    pub n_net: String,
    /// single-ended reference of each port (Zdiff,ref = 2·z_ref, Zcomm,ref = z_ref/2)
    pub z_ref: f64,
    pub freq: Vec<f64>,
    pub sdd21_db: Vec<f64>,
    pub sdd11_db: Vec<f64>,
    pub sdd22_db: Vec<f64>,
    pub scc21_db: Vec<f64>,
    pub scc11_db: Vec<f64>,
    pub scc22_db: Vec<f64>,
    /// differential in → common out (mode conversion), far end and near end
    pub scd21_db: Vec<f64>,
    pub scd11_db: Vec<f64>,
    /// worst mode conversion over the band (dB) and its frequency (Hz)
    pub scd21_worst: (f64, f64),
    pub tdr_t_ps: Vec<f64>,
    pub tdr_zdiff: Vec<f64>,
    pub tdr_zcomm: Vec<f64>,
    /// TDR position (track mm along the pair) for the differential and the common trace
    pub tdr_x_diff_mm: Vec<f64>,
    pub tdr_x_comm_mm: Vec<f64>,
    pub tdr_end_mm: f64,
    /// each line alone with the other terminated (S11 / S22 of the 4-port, reference z_ref),
    /// positions along its own path
    pub tdr_zp: Vec<f64>,
    pub tdr_xp_mm: Vec<f64>,
    pub tdr_zn: Vec<f64>,
    pub tdr_xn_mm: Vec<f64>,
    pub imbalance: Vec<Imbalance>,
    /// track length and static delay of each line (P, N)
    pub length_mm: (f64, f64),
    pub delay_ps: (f64, f64),
    /// P − N
    pub skew_ps: f64,
    pub skew_mm: f64,
    /// length the two lines run as a coupled pair, and the coupled share of the P length
    pub coupled_mm: f64,
    pub coupled_share: f64,
    /// modal delays of the pair (ps): differential and common mode
    pub delay_diff_ps: f64,
    pub delay_comm_ps: f64,
    pub vias: (usize, usize),
    pub source: String,
    pub warnings: Vec<String>,
    #[serde(skip)]
    pub s: Vec<M4>,
}

/// 4-port model of the pair `net` + its partner. `flip` / `hint` choose the start as for
/// the channel of `net`; the partner starts at the end next to it.
#[allow(clippy::too_many_arguments)]
pub fn mixed_mode(board: &Board, an: &Analysis, net: u32, field: &HashMap<String, FieldResult>, target: Option<(f64, bool)>, rp: &RfParams, flip: bool, hint: Option<P>) -> Result<MixedMode, String> {
    let partner = crate::extract::diff_partners(&board.nets).get(net as usize).copied().flatten().ok_or("net is not part of a differential pair")?;
    let ip = net_path_info(board, an, net, flip, hint).ok_or("net has no tracks")?;
    let inn = net_path_info(board, an, partner, false, Some(ip.start.at)).ok_or("partner net has no tracks")?;
    let mut bp = net_build(board, an, net, field, target, rp, &ip, false, true);
    let mut bn = net_build(board, an, partner, field, target, rp, &inn, false, true);
    // line 1 is the positive net of the pair (name ending in +, P or p), whichever was asked for
    let (mut pnet, mut nnet) = (net, partner);
    let (mut path_p, mut path_n) = (ip, inn);
    if !board.net_name(net).ends_with(['+', 'P', 'p']) {
        std::mem::swap(&mut bp, &mut bn);
        std::mem::swap(&mut pnet, &mut nnet);
        std::mem::swap(&mut path_p, &mut path_n);
    }
    let mut warnings: Vec<String> = Vec::new();
    for w in bp.warnings.iter().chain(&bn.warnings) {
        if !warnings.contains(w) {
            warnings.push(w.clone());
        }
    }
    let z_ref = rp.z_ref.unwrap_or(match target {
        Some((z, true)) => z / 2.0,
        Some((z, false)) => z,
        _ => 50.0,
    });
    let fallback = Build::fallback(z_ref);
    let su = board.stackup();
    let xs_of = |b: &Build, i: usize| b.segs[i].xs.map(|x| &an.cross_sections[x as usize]);
    let tok = |b: &Build, i: usize| -> Tok {
        match b.segs[i].kind {
            "line" => match xs_of(b, i) {
                Some(x) if x.diff_gap.is_some() && x.zdiff.is_some() => Tok::C(b.segs[i].layer),
                _ => Tok::U,
            },
            "via" => Tok::V,
            _ => Tok::U,
        }
    };
    let modal = |b: &Build, i: usize| -> Option<(LineModel, LineModel)> {
        let x = xs_of(b, i)?;
        let fr = x.geom.and_then(|g| field.get(&an.geometries[g as usize].id));
        Some((line_model_mode(su, x, fr, Mode::Even, rp)?, line_model_mode(su, x, fr, Mode::Odd, rp)?))
    };

    // align the two section lists (greedy): coupled runs on the same layer pair up over
    // the shorter length, the rest of the longer one stays a single line; vias pair up
    let mut els: Vec<El> = Vec::new();
    let (mut i, mut j) = (0usize, 0usize);
    let (mut rem_p, mut rem_n) = (f64::NAN, f64::NAN); // remaining length of the head line
    let len_left = |b: &Build, k: usize, rem: f64| if rem.is_nan() { b.segs[k].len_mm } else { rem };
    let mut coupled_mm = 0.0;
    let mut asym_vias = 0usize;
    let mut no_modal = false;
    while i < bp.segs.len() || j < bn.segs.len() {
        let tp = (i < bp.segs.len()).then(|| tok(&bp, i));
        let tn = (j < bn.segs.len()).then(|| tok(&bn, j));
        let single_p = |els: &mut Vec<El>, rem: f64| els.push(El::Single { line: 0, seg: i, len: (!rem.is_nan()).then_some(rem) });
        let single_n = |els: &mut Vec<El>, rem: f64| els.push(El::Single { line: 1, seg: j, len: (!rem.is_nan()).then_some(rem) });
        match (tp, tn) {
            (Some(Tok::U), _) | (Some(_), None) => {
                single_p(&mut els, rem_p);
                i += 1;
                rem_p = f64::NAN;
            }
            (_, Some(Tok::U)) | (None, Some(_)) => {
                single_n(&mut els, rem_n);
                j += 1;
                rem_n = f64::NAN;
            }
            (Some(Tok::C(lp)), Some(Tok::C(ln))) if lp == ln => {
                let (a, b) = (len_left(&bp, i, rem_p), len_left(&bn, j, rem_n));
                let l = a.min(b);
                match modal(&bp, i).or_else(|| modal(&bn, j)) {
                    Some((even, odd)) => {
                        els.push(El::Coupled { len: l, even, odd });
                        coupled_mm += l;
                    }
                    None => {
                        no_modal = true;
                        els.push(El::Single { line: 0, seg: i, len: Some(l) });
                        els.push(El::Single { line: 1, seg: j, len: Some(l) });
                    }
                }
                rem_p = a - l;
                rem_n = b - l;
                if rem_p <= 1e-6 {
                    i += 1;
                    rem_p = f64::NAN;
                }
                if rem_n <= 1e-6 {
                    j += 1;
                    rem_n = f64::NAN;
                }
            }
            (Some(Tok::V), Some(Tok::V)) => {
                let same = match (&bp.via_models[i], &bn.via_models[j]) {
                    (Some(a), Some(b)) => (a.delay() - b.delay()).abs() <= 0.1 * a.delay().max(b.delay()) + 1e-15,
                    _ => false,
                };
                if same && bp.via_pair[i].is_some() {
                    els.push(El::ViaPair { p: i });
                } else {
                    asym_vias += 1;
                    els.push(El::Single { line: 0, seg: i, len: None });
                    els.push(El::Single { line: 1, seg: j, len: None });
                }
                i += 1;
                j += 1;
            }
            // coupled on one side only (other layer, or the partner already changes layer)
            (Some(Tok::C(_)), _) => {
                single_p(&mut els, rem_p);
                i += 1;
                rem_p = f64::NAN;
            }
            (_, Some(Tok::C(_))) => {
                single_n(&mut els, rem_n);
                j += 1;
                rem_n = f64::NAN;
            }
            (None, None) => break,
        }
    }
    if asym_vias > 0 {
        warnings.push(format!("{asym_vias} via pair(s) with different barrels: each via modelled on its own line"));
    }
    if no_modal {
        warnings.push("coupled sections without modal data: modelled as uncoupled lines".into());
    }
    let est = els.iter().any(|e| matches!(e, El::Coupled { odd, .. } if odd.source != "field"));
    let source = if est { "odd mode from the field solver or formulas, even mode estimated where no field solve (Zeven ≈ Z0²/Zodd)" } else { "field solver (odd and even mode)" };

    // sweep
    let n = rp.npts.max(10);
    let df = rp.fmax / n as f64;
    let freq: Vec<f64> = (1..=n).map(|k| k as f64 * df).collect();
    let id = Abcd::identity();
    let mut s_all: Vec<M4> = Vec::with_capacity(n);
    let mut mm_all: Vec<M4> = Vec::with_capacity(n);
    for (k, &f) in freq.iter().enumerate() {
        let w = 2.0 * PI * f;
        let mut t = m4_id();
        for e in &els {
            let m = match e {
                El::Single { line: 0, seg, len } => sep(&bp.seg_abcd(*seg, k, f, &fallback, *len), &id),
                El::Single { seg, len, .. } => sep(&id, &bn.seg_abcd(*seg, k, f, &fallback, *len)),
                El::Coupled { len, even, odd } => {
                    let (pe, po) = (even.rlgc(f, rp), odd.rlgc(f, rp));
                    coupled(&Abcd::line(pe.r, pe.l, pe.g, pe.c, w, len * 1e-3), &Abcd::line(po.r, po.l, po.g, po.c, w, len * 1e-3))
                }
                El::ViaPair { p, .. } => {
                    let ev = bp.via_models[*p].as_ref().map(|v| v.abcd(f, rp.sigma)).unwrap_or(id);
                    let od = bp.via_pair[*p].as_ref().map(|v| v.abcd(f, rp.sigma)).unwrap_or(ev);
                    coupled(&ev, &od)
                }
            };
            t = m4_mul(&t, &m);
        }
        let s = s_of(&t, z_ref);
        mm_all.push(mixed_of(&s));
        s_all.push(s);
    }
    let db = |x: Cx| 20.0 * x.abs().max(1e-12).log10();
    let col = |r: usize, c: usize| -> Vec<f64> { mm_all.iter().map(|m| db(m[r][c])).collect() };
    let scd21_db = col(3, 0);
    let scd21_worst = scd21_db.iter().zip(&freq).fold((f64::NEG_INFINITY, 0.0), |a, (&d, &f)| if d > a.0 { (d, f) } else { a });

    // static delays: per line, and per mode along the aligned cascade (position = mean track)
    let line_delay = |b: &Build| (0..b.segs.len()).map(|k| b.seg_delay(k, &fallback)).sum::<f64>();
    let line_len = |b: &Build| b.segs.iter().filter(|s| s.kind == "line").map(|s| s.len_mm).sum::<f64>();
    let (dp, dn) = (line_delay(&bp), line_delay(&bn));
    let (lp, ln) = (line_len(&bp), line_len(&bn));
    let mut pos_d: Vec<(f64, f64)> = vec![(0.0, 0.0)];
    let mut pos_c: Vec<(f64, f64)> = vec![(0.0, 0.0)];
    let (mut td, mut tc, mut s) = (0.0, 0.0, 0.0);
    // where P and N differ: the stretches between coupled runs, with their length and delay balance
    let mut imbalance: Vec<Imbalance> = Vec::new();
    let (mut sp, mut sn) = (0.0, 0.0); // own track position of P and N
    let mut reg = Region::default();
    let close = |reg: &mut Region, sp: f64, sn: f64, out: &mut Vec<Imbalance>| {
        let (dl, dt) = (reg.lp - reg.ln, reg.dp - reg.dn);
        if dl.abs() >= 0.05 || dt.abs() >= 0.2e-12 || reg.vias {
            let at = if dl >= 0.0 { path_point(board, an, &path_p, 0.5 * (reg.sp0 + sp)) } else { path_point(board, an, &path_n, 0.5 * (reg.sn0 + sn)) };
            out.push(Imbalance { s_from_mm: reg.sp0, s_to_mm: sp, dl_mm: dl, dt_ps: dt * 1e12, vias_differ: reg.vias, at });
        }
        *reg = Region { sp0: sp, sn0: sn, ..Default::default() };
    };
    for e in &els {
        match e {
            El::Single { line, seg, len } => {
                let b = if *line == 0 { &bp } else { &bn };
                let frac = match (len, b.segs[*seg].kind) {
                    (Some(l), "line") if b.segs[*seg].len_mm > 0.0 => l / b.segs[*seg].len_mm,
                    _ => 1.0,
                };
                let dl = if b.segs[*seg].kind == "line" { len.unwrap_or(b.segs[*seg].len_mm) } else { 0.0 };
                let dd = b.seg_delay(*seg, &fallback) * frac;
                if *line == 0 {
                    reg.lp += dl;
                    reg.dp += dd;
                    sp += dl;
                } else {
                    reg.ln += dl;
                    reg.dn += dd;
                    sn += dl;
                }
                reg.vias |= b.segs[*seg].kind == "via";
            }
            El::Coupled { len, .. } => {
                close(&mut reg, sp, sn, &mut imbalance);
                sp += len;
                sn += len;
                reg.sp0 = sp;
                reg.sn0 = sn;
            }
            El::ViaPair { .. } => {}
        }
    }
    close(&mut reg, sp, sn, &mut imbalance);
    // stretches less than 1 mm apart count as one (a fan-out that is longer on P here and on N there)
    let mut merged: Vec<Imbalance> = Vec::new();
    for x in imbalance {
        match merged.last_mut() {
            Some(m) if x.s_from_mm - m.s_to_mm < 1.0 => {
                if x.dl_mm.abs() > m.dl_mm.abs() {
                    m.at = x.at;
                }
                m.s_to_mm = x.s_to_mm;
                m.dl_mm += x.dl_mm;
                m.dt_ps += x.dt_ps;
                m.vias_differ |= x.vias_differ;
            }
            _ => merged.push(x),
        }
    }
    let imbalance: Vec<Imbalance> = merged.into_iter().filter(|x| x.dl_mm.abs() >= 0.05 || x.dt_ps.abs() >= 0.2 || x.vias_differ).collect();
    for e in &els {
        let (d_d, d_c, ds) = match e {
            El::Single { line, seg, len } => {
                let b = if *line == 0 { &bp } else { &bn };
                let full = b.seg_delay(*seg, &fallback);
                let l0 = b.segs[*seg].len_mm;
                let frac = match (len, b.segs[*seg].kind) {
                    (Some(l), "line") if l0 > 0.0 => l / l0,
                    _ => 1.0,
                };
                let l = if b.segs[*seg].kind == "line" { len.unwrap_or(l0) } else { 0.0 };
                // a section on one line only moves the mode by half of it
                (full * frac / 2.0, full * frac / 2.0, l / 2.0)
            }
            El::Coupled { len, even, odd } => (len * 1e-3 * (odd.l_ext * odd.c0).sqrt(), len * 1e-3 * (even.l_ext * even.c0).sqrt(), *len),
            El::ViaPair { p, .. } => {
                let ev = bp.via_models[*p].as_ref().map(|v| v.delay()).unwrap_or(0.0);
                let od = bp.via_pair[*p].as_ref().map(|v| v.delay()).unwrap_or(ev);
                (od, ev, 0.0)
            }
        };
        td += d_d;
        tc += d_c;
        s += ds;
        pos_d.push((td, s));
        pos_c.push((tc, s));
    }
    let xmap_of = |tab: &Vec<(f64, f64)>, total: f64| {
        let tab = tab.clone();
        move |t: f64| -> f64 {
            let h = t / 2.0;
            let v = if total > 0.0 { s / total } else { 0.0 };
            match tab.iter().position(|&(d, _)| d >= h) {
                Some(0) => 0.0,
                Some(k) => {
                    let ((d0, s0), (d1, s1)) = (tab[k - 1], tab[k]);
                    if d1 > d0 { s0 + (s1 - s0) * (h - d0) / (d1 - d0) } else { s1 }
                }
                None => s + (h - total) * v,
            }
        }
    };
    let t_max = (2.0 * td.max(tc) * 1.3 + 4.0 * rp.rise).max(200e-12);
    let sdd11: Vec<Cx> = mm_all.iter().map(|m| m[0][0]).collect();
    let scc11: Vec<Cx> = mm_all.iter().map(|m| m[2][2]).collect();
    let (tdr_t_ps, tdr_zdiff, tdr_x_diff_mm) = tdr_map(&freq, &sdd11, 2.0 * z_ref, rp.rise, t_max, &xmap_of(&pos_d, td));
    let (_, tdr_zcomm, tdr_x_comm_mm) = tdr_map(&freq, &scc11, z_ref / 2.0, rp.rise, t_max, &xmap_of(&pos_c, tc));
    // each line alone (the other one terminated): S11 and S22 of the 4-port, positions along its own path
    let own_map = |b: &Build| {
        let mut tab = vec![(0.0, 0.0)];
        let (mut d, mut x) = (0.0, 0.0);
        for k in 0..b.segs.len() {
            d += b.seg_delay(k, &fallback);
            if b.segs[k].kind == "line" {
                x += b.segs[k].len_mm;
            }
            tab.push((d, x));
        }
        (tab, d)
    };
    let (tab_p, d_p) = own_map(&bp);
    let (tab_n, d_n) = own_map(&bn);
    let s_line = |i: usize| -> Vec<Cx> { s_all.iter().map(|m| m[i][i]).collect() };
    let line_x = |tab: Vec<(f64, f64)>, total: f64, len: f64| {
        move |t: f64| -> f64 {
            let h = t / 2.0;
            match tab.iter().position(|&(d, _)| d >= h) {
                Some(0) => 0.0,
                Some(k) => {
                    let ((d0, s0), (d1, s1)) = (tab[k - 1], tab[k]);
                    if d1 > d0 { s0 + (s1 - s0) * (h - d0) / (d1 - d0) } else { s1 }
                }
                None => len + (h - total) * if total > 0.0 { len / total } else { 0.0 },
            }
        }
    };
    let (_, tdr_zp, tdr_xp_mm) = tdr_map(&freq, &s_line(0), z_ref, rp.rise, t_max, &line_x(tab_p, d_p, lp));
    let (_, tdr_zn, tdr_xn_mm) = tdr_map(&freq, &s_line(1), z_ref, rp.rise, t_max, &line_x(tab_n, d_n, ln));
    let nvia = |b: &Build| b.segs.iter().filter(|s| s.kind == "via").count();
    Ok(MixedMode {
        p_id: pnet,
        n_id: nnet,
        p_a: path_p.start.label.clone(),
        p_b: path_p.end.label.clone(),
        n_a: path_n.start.label.clone(),
        n_b: path_n.end.label.clone(),
        p_net: board.net_name(pnet).to_string(),
        n_net: board.net_name(nnet).to_string(),
        z_ref,
        sdd21_db: col(1, 0),
        sdd11_db: col(0, 0),
        sdd22_db: col(1, 1),
        scc21_db: col(3, 2),
        scc11_db: col(2, 2),
        scc22_db: col(3, 3),
        scd21_db,
        scd11_db: col(2, 0),
        scd21_worst,
        freq,
        tdr_t_ps,
        tdr_zdiff,
        tdr_zcomm,
        tdr_x_diff_mm,
        tdr_x_comm_mm,
        tdr_end_mm: s,
        tdr_zp,
        tdr_xp_mm,
        tdr_zn,
        tdr_xn_mm,
        imbalance,
        length_mm: (lp, ln),
        delay_ps: (dp * 1e12, dn * 1e12),
        skew_ps: (dp - dn) * 1e12,
        skew_mm: lp - ln,
        coupled_mm,
        coupled_share: if lp > 0.0 { (coupled_mm / lp).min(1.0) } else { 0.0 },
        delay_diff_ps: td * 1e12,
        delay_comm_ps: tc * 1e12,
        vias: (nvia(&bp), nvia(&bn)),
        source: source.into(),
        warnings,
        s: s_all,
    })
}

/// Touchstone (.s2p, RI) of the differential mode: Sdd of the 4-port, reference 2·z_ref.
pub fn touchstone_dd(m: &MixedMode) -> String {
    let mut s = String::new();
    s.push_str(&format!("! kisi {}: differential S-parameters (Sdd) of the pair P = {}, N = {}\n", env!("CARGO_PKG_VERSION"), m.p_net, m.n_net));
    s.push_str(&format!("! 2-port, differential mode (mixed-mode Sdd of the 4-port model), reference {:.1} ohm differential at both ports\n", 2.0 * m.z_ref));
    s.push_str(&format!("! Port 1 = A: the pair driven differentially at {} (P) and {} (N)\n", m.p_a, m.n_a));
    s.push_str(&format!("! Port 2 = B: the pair at {} (P) and {} (N)\n", m.p_b, m.n_b));
    s.push_str("! Data per frequency: Sdd11 Sdd21 Sdd12 Sdd22, each as real and imaginary part\n");
    s.push_str("! Contains the differential mode only\n");
    s.push_str(&format!("# HZ S RI R {:.3}\n", 2.0 * m.z_ref));
    for (k, &f) in m.freq.iter().enumerate() {
        let mm = mixed_of(&m.s[k]);
        let (a, b, c, d) = (mm[0][0], mm[1][0], mm[0][1], mm[1][1]);
        // Touchstone 2-port order: S11 S21 S12 S22
        s.push_str(&format!("{:.6e} {:.6e} {:.6e} {:.6e} {:.6e} {:.6e} {:.6e} {:.6e} {:.6e}\n", f, a.re, a.im, b.re, b.im, c.re, c.im, d.re, d.im));
    }
    s
}

/// Touchstone (.s4p, RI) of a pair: single-ended 4-port, port order in the header.
pub fn touchstone4(m: &MixedMode) -> String {
    let mut s = String::new();
    s.push_str(&format!("! kisi {}: single-ended 4-port of the pair P = {}, N = {}\n", env!("CARGO_PKG_VERSION"), m.p_net, m.n_net));
    s.push_str(&format!("! reference {:.1} ohm at every port\n", m.z_ref));
    s.push_str("! Contains both lines with their coupling: differential and common mode, mode conversion (Scd, Sdc), skew between P and N\n");
    s.push_str(&format!("! Port 1 = P at A: {} at {}\n", m.p_net, m.p_a));
    s.push_str(&format!("! Port 2 = N at A: {} at {}\n", m.n_net, m.n_a));
    s.push_str(&format!("! Port 3 = P at B: {} at {}\n", m.p_net, m.p_b));
    s.push_str(&format!("! Port 4 = N at B: {} at {}\n", m.n_net, m.n_b));
    s.push_str("! Mixed mode with this order: differential port 1 = (port 1 - port 2)/sqrt(2), common port 1 = (port 1 + port 2)/sqrt(2),\n");
    s.push_str("!   ports 3 and 4 likewise for port 2; e.g. Sdd21 = (S31 - S32 - S41 + S42)/2, Scd21 = (S31 - S32 + S41 - S42)/2\n");
    s.push_str("! Data per frequency: 4 lines, line i = Si1 Si2 Si3 Si4, each as real and imaginary part\n");
    s.push_str(&format!("! P {:.2} mm, N {:.2} mm, coupled {:.2} mm, skew {:.2} ps; {}\n", m.length_mm.0, m.length_mm.1, m.coupled_mm, m.skew_ps, m.source.replace('≈', "~").replace('²', "^2")));
    s.push_str(&format!("# HZ S RI R {:.3}\n", m.z_ref));
    for (k, &f) in m.freq.iter().enumerate() {
        let sm = &m.s[k];
        // 4-port: four rows per frequency, row i holds S(i,1..4)
        for (i, row) in sm.iter().enumerate() {
            if i == 0 {
                s.push_str(&format!("{:.6e}", f));
            } else {
                s.push_str("              ");
            }
            for x in row {
                s.push_str(&format!(" {:.6e} {:.6e}", x.re, x.im));
            }
            s.push('\n');
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lossless(zc: f64, eeff: f64, f: f64, len: f64) -> Abcd {
        let l = zc * eeff.sqrt() / C0;
        let c = eeff.sqrt() / (zc * C0);
        Abcd::line(0.0, l, 0.0, c, 2.0 * PI * f, len)
    }

    #[test]
    fn equal_modes_are_two_lines() {
        let a = lossless(50.0, 3.0, 3e9, 0.02);
        let c = coupled(&a, &a);
        let s = sep(&a, &a);
        for i in 0..4 {
            for j in 0..4 {
                assert!((c[i][j] - s[i][j]).abs() < 1e-12);
            }
        }
    }

    #[test]
    fn symmetric_coupled_pair() {
        // Zodd 45, Zeven 60 per line, inhomogeneous; reference 45 Ω: matched differentially
        let (f, len) = (5e9, 0.03);
        let t = coupled(&lossless(60.0, 3.4, f, len), &lossless(45.0, 3.0, f, len));
        let mm = mixed_of(&s_of(&t, 45.0));
        assert!(mm[0][0].abs() < 1e-9, "Sdd11 {:?}", mm[0][0]);
        assert!((mm[1][0].abs() - 1.0).abs() < 1e-9);
        // phase of Sdd21 = −β_odd·len
        let beta = 2.0 * PI * f * 3.0f64.sqrt() / C0;
        let ph = (mm[1][0].arg() + beta * len).rem_euclid(2.0 * PI);
        assert!(ph < 1e-6 || (2.0 * PI - ph) < 1e-6, "{ph}");
        // no mode conversion; common mode sees Zeven/2 = 30 Ω against 22.5 Ω
        for (r, c) in [(2, 0), (3, 0), (0, 2), (1, 2)] {
            assert!(mm[r][c].abs() < 1e-12, "S[{r}][{c}]");
        }
        assert!(mm[2][2].abs() > 0.05);
        // passive, lossless: unitary columns
        let s = s_of(&t, 45.0);
        for j in 0..4 {
            let p: f64 = (0..4).map(|i| s[i][j].abs().powi(2)).sum();
            assert!((p - 1.0).abs() < 1e-9, "col {j}: {p}");
        }
    }

    #[test]
    fn length_mismatch_converts_modes() {
        // two uncoupled matched lines, P longer by dl: |Sdd21| = |cos(β dl/2)|, |Scd21| = |sin(β dl/2)|
        let (f, len, dl, ee) = (8e9, 0.02, 0.004, 3.2);
        let t = sep(&lossless(50.0, ee, f, len + dl), &lossless(50.0, ee, f, len));
        let mm = mixed_of(&s_of(&t, 50.0));
        let x = 2.0 * PI * f * ee.sqrt() / C0 * dl / 2.0;
        assert!((mm[1][0].abs() - x.cos().abs()).abs() < 1e-9);
        assert!((mm[3][0].abs() - x.sin().abs()).abs() < 1e-9);
        assert!(mm[0][0].abs() < 1e-9);
    }

    fn pair_board(jog: f64) -> Board {
        // edge-coupled microstrip pair DP/DN, 0.15 mm / 0.15 mm gap, 30 mm, over GND;
        // DP optionally with a jog (2 × `jog` mm extra) in the middle
        let mut segs = String::new();
        let y_p = -0.15;
        let y_n = 0.15;
        if jog > 0.0 {
            segs += &format!("(segment (start 0 {y_p}) (end 14 {y_p}) (width 0.15) (layer \"F.Cu\") (net 1))\n");
            segs += &format!("(segment (start 14 {y_p}) (end 14 {}) (width 0.15) (layer \"F.Cu\") (net 1))\n", y_p - jog);
            segs += &format!("(segment (start 14 {}) (end 15 {}) (width 0.15) (layer \"F.Cu\") (net 1))\n", y_p - jog, y_p - jog);
            segs += &format!("(segment (start 15 {}) (end 15 {y_p}) (width 0.15) (layer \"F.Cu\") (net 1))\n", y_p - jog);
            segs += &format!("(segment (start 15 {y_p}) (end 30 {y_p}) (width 0.15) (layer \"F.Cu\") (net 1))\n");
        } else {
            segs += &format!("(segment (start 0 {y_p}) (end 30 {y_p}) (width 0.15) (layer \"F.Cu\") (net 1))\n");
        }
        segs += &format!("(segment (start 0 {y_n}) (end 30 {y_n}) (width 0.15) (layer \"F.Cu\") (net 2))\n");
        let pcb = format!(
            r#"(kicad_pcb (version 20240108) (generator "pcbnew")
  (general (thickness 1.6))
  (layers (0 "F.Cu" signal) (1 "In1.Cu" signal) (31 "B.Cu" signal))
  (setup (stackup
    (layer "F.Cu" (type "copper") (thickness 0.035))
    (layer "dielectric 1" (type "prepreg") (thickness 0.1) (material "FR4") (epsilon_r 4.0) (loss_tangent 0.02))
    (layer "In1.Cu" (type "copper") (thickness 0.035))
    (layer "dielectric 2" (type "core") (thickness 1.3) (material "FR4") (epsilon_r 4.5) (loss_tangent 0.02))
    (layer "B.Cu" (type "copper") (thickness 0.035))))
  (net 0 "") (net 1 "D_P") (net 2 "D_N") (net 3 "GND")
  (footprint "U" (layer "F.Cu") (at 0 0) (property "Reference" "U1") (property "Value" "IC")
    (pad "1" smd rect (at 0 {y_p}) (size 0.15 0.15) (layers "F.Cu") (net 1 "D_P"))
    (pad "2" smd rect (at 0 {y_n}) (size 0.15 0.15) (layers "F.Cu") (net 2 "D_N")))
  (footprint "U" (layer "F.Cu") (at 30 0) (property "Reference" "U2") (property "Value" "IC")
    (pad "1" smd rect (at 0 {y_p}) (size 0.15 0.15) (layers "F.Cu") (net 1 "D_P"))
    (pad "2" smd rect (at 0 {y_n}) (size 0.15 0.15) (layers "F.Cu") (net 2 "D_N")))
  {segs}
  (zone (net 3) (net_name "GND") (layer "In1.Cu") (filled_polygon (layer "In1.Cu") (pts (xy -5 -5) (xy 35 -5) (xy 35 5) (xy -5 5)))))
"#
        );
        kisi_board::load_pcb(&pcb).unwrap()
    }

    #[test]
    fn pair_on_a_board() {
        let rp = RfParams { fmax: 20e9, npts: 200, ..Default::default() };
        for jog in [0.0, 1.0] {
            let board = pair_board(jog);
            let an = crate::analyze(&board, &crate::Params::default());
            let net = board.nets.iter().position(|n| n == "D_P").unwrap() as u32;
            let m = mixed_mode(&board, &an, net, &HashMap::new(), Some((100.0, true)), &rp, false, None).unwrap();
            eprintln!(
                "jog {jog}: P {:.2} N {:.2} mm, coupled {:.2} mm, skew {:.2} ps, Scd21 worst {:.1} dB @ {:.1} GHz, Sdd21(20G) {:.2} dB, Zdiff TDR mid {:.1}",
                m.length_mm.0,
                m.length_mm.1,
                m.coupled_mm,
                m.skew_ps,
                m.scd21_worst.0,
                m.scd21_worst.1 / 1e9,
                m.sdd21_db.last().unwrap(),
                m.tdr_zdiff[m.tdr_x_diff_mm.iter().position(|&x| x > 15.0).unwrap()]
            );
            assert_eq!(m.n_net, "D_N");
            assert!(m.coupled_mm > 25.0, "coupled {}", m.coupled_mm);
            let zdiff = m.tdr_zdiff[m.tdr_x_diff_mm.iter().position(|&x| x > 15.0).unwrap()];
            assert!(zdiff > 60.0 && zdiff < 130.0, "{zdiff}");
            if jog == 0.0 {
                assert!(m.skew_ps.abs() < 0.5, "{}", m.skew_ps);
                assert!(m.imbalance.is_empty(), "{:?}", m.imbalance);
                assert!(m.scd21_worst.0 < -60.0, "{:?}", m.scd21_worst);
                // symmetric pair: Sdd21 = S21 of the odd-mode channel
                let ch = channel_dir(&board, &an, net, &HashMap::new(), Some((100.0, true)), &rp, false, None).unwrap();
                let (a, b) = (m.sdd21_db.last().unwrap(), ch.s21_db.last().unwrap());
                assert!((a - b).abs() < 0.05, "Sdd21 {a} vs odd-mode S21 {b}");
            } else {
                assert!((m.skew_mm - 2.0).abs() < 0.3, "{}", m.skew_mm);
                assert!(m.skew_ps > 8.0 && m.skew_ps < 20.0, "{}", m.skew_ps);
                assert!(m.scd21_worst.0 > -25.0, "{:?}", m.scd21_worst);
                // the jog is the one place where P and N differ
                assert_eq!(m.imbalance.len(), 1, "{:?}", m.imbalance);
                let im = &m.imbalance[0];
                assert!((im.dl_mm - 2.0).abs() < 0.3 && im.dt_ps > 8.0, "{im:?}");
                assert!(im.s_from_mm > 13.0 && im.s_to_mm < 18.0 && (im.at[0] - 14.5).abs() < 1.0 && im.at[1] < -0.5, "{im:?}");
                // single lines: P alone sees the jog, N does not
                assert_eq!(m.tdr_zp.len(), m.tdr_zn.len());
                assert!(touchstone_dd(&m).lines().filter(|l| !l.starts_with('!') && !l.starts_with('#')).count() == m.freq.len());
                // asked from the N side: same pair, P first
                let nn = board.nets.iter().position(|n| n == "D_N").unwrap() as u32;
                let m2 = mixed_mode(&board, &an, nn, &HashMap::new(), Some((100.0, true)), &rp, false, None).unwrap();
                assert_eq!(m2.p_net, "D_P");
                assert!((m2.skew_ps - m.skew_ps).abs() < 0.01);
            }
            let ts = touchstone4(&m);
            assert_eq!(ts.lines().filter(|l| !l.starts_with('!') && !l.starts_with('#')).count(), 4 * m.freq.len());
        }
    }
}

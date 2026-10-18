//! Channel of a net (or of a device-to-device lane): path through the track
//! graph, cascade of line sections, vias and lumped parts, S-parameters, TDR
//! and Touchstone export.

#[allow(unused_imports)]
use super::*;
use crate::board::Board;
use crate::extract::{Analysis, FieldResult};
use crate::geom::{self, P};
use serde::Serialize;
use std::collections::HashMap;
use std::f64::consts::PI;

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
    /// lumped part ("R38 0 Ω"), for kind "series" / "shunt"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub part: Option<String>,
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
    /// cascade matrix of the whole channel per frequency (for other terminations)
    #[serde(skip)]
    pub abcd: Vec<Abcd>,
}

/// Track graph of one net: nodes are joined track ends (per layer; layers join
/// only through vias and plated holes), edges are tracks.
pub(crate) struct NetGraph {
    /// (track, node a, node b, length mm)
    edges: Vec<(usize, usize, usize, f64)>,
    adj: HashMap<usize, Vec<(usize, usize)>>,
    /// position of every node (after joining: representative positions)
    pos: Vec<P>,
    root: Vec<usize>,
    pieces_of: HashMap<usize, Vec<usize>>,
}

impl NetGraph {
    pub(crate) fn build(board: &Board, an: &Analysis, net: u32) -> Option<NetGraph> {
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
            return None;
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
        let mut edges: Vec<(usize, usize, usize, f64)> = Vec::new();
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
        let root: Vec<usize> = (0..n).map(|i| find(&mut parent, i)).collect();
        let edges: Vec<(usize, usize, usize, f64)> = edges.into_iter().map(|(t, a, b, l)| (t, root[a], root[b], l)).collect();
        let mut adj: HashMap<usize, Vec<(usize, usize)>> = HashMap::new(); // node -> (edge idx, other node)
        for (ei, &(_, a, b, _)) in edges.iter().enumerate() {
            if a == b {
                continue;
            }
            adj.entry(a).or_default().push((ei, b));
            adj.entry(b).or_default().push((ei, a));
        }
        Some(NetGraph { edges, adj, pos, root, pieces_of })
    }

    pub(crate) fn has_branches(&self) -> bool {
        self.branches()
    }

    fn branches(&self) -> bool {
        self.adj.values().any(|v| v.len() > 2)
    }

    fn components(&self) -> usize {
        let mut seen = std::collections::HashSet::new();
        let mut c = 0;
        for &k in self.adj.keys() {
            if seen.contains(&k) {
                continue;
            }
            c += 1;
            let mut st = vec![k];
            while let Some(u) = st.pop() {
                if !seen.insert(u) {
                    continue;
                }
                for &(_, v) in self.adj.get(&u).map(|x| x.as_slice()).unwrap_or(&[]) {
                    st.push(v);
                }
            }
        }
        c
    }

    /// Distances and predecessors from `start` (the graph is a tree in practice).
    fn reach(&self, start: usize) -> HashMap<usize, (f64, Option<(usize, usize)>)> {
        let mut dist: HashMap<usize, (f64, Option<(usize, usize)>)> = HashMap::new();
        dist.insert(start, (0.0, None));
        let mut stack = vec![start];
        while let Some(u) = stack.pop() {
            let du = dist[&u].0;
            for &(ei, v) in self.adj.get(&u).map(|x| x.as_slice()).unwrap_or(&[]) {
                if !dist.contains_key(&v) {
                    dist.insert(v, (du + self.edges[ei].3, Some((ei, u))));
                    stack.push(v);
                }
            }
        }
        dist
    }

    /// Node of the graph closest to a point (any layer).
    pub(crate) fn nearest(&self, p: P) -> Option<usize> {
        self.adj
            .keys()
            .copied()
            .min_by(|&a, &b| geom::len(geom::sub(self.pos[a], p)).partial_cmp(&geom::len(geom::sub(self.pos[b], p))).unwrap().then(a.cmp(&b)))
    }

    /// Pieces from node `a` to node `b`: (piece index, reversed).
    pub(crate) fn path(&self, an: &Analysis, a: usize, b: usize) -> Vec<(usize, bool)> {
        let dist = self.reach(a);
        let mut order: Vec<(usize, usize)> = Vec::new(); // (edge, from-node)
        let mut cur = b;
        while let Some((_, Some((ei, prev)))) = dist.get(&cur).copied() {
            order.push((ei, prev));
            cur = prev;
        }
        order.reverse();
        let mut out = Vec::new();
        for (ei, from) in order {
            let (ti, ea, _, _) = self.edges[ei];
            let fwd = ea == from;
            let mut ps = self.pieces_of[&ti].clone();
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

    /// Longest path (both ends farthest apart).
    fn longest(&self, an: &Analysis) -> Vec<(usize, bool)> {
        let far = |start: usize| -> usize {
            let dist = self.reach(start);
            // deterministic: longest distance, ties to the lower node id
            dist.iter().max_by(|a, b| a.1 .0.partial_cmp(&b.1 .0).unwrap().then(b.0.cmp(a.0))).map(|x| *x.0).unwrap_or(start)
        };
        let start = self.adj.keys().copied().min().unwrap_or(0);
        let a = far(start);
        let b = far(a);
        self.path(an, a, b)
    }
}

/// Ordered path of a net: (piece indices in order, reversed flag per piece).
/// Longest path through the track graph.
pub(crate) fn net_path(board: &Board, an: &Analysis, net: u32, warnings: &mut Vec<String>) -> Vec<(usize, bool)> {
    let Some(g) = NetGraph::build(board, an, net) else { return vec![] };
    if g.branches() {
        warnings.push("net branches (stubs, test points): the longest path is computed, branches are missing".into());
    }
    let comps = g.components();
    if comps > 1 {
        warnings.push(format!("net splits into {comps} parts (e.g. connected through components): the longest part is computed"));
    }
    let _ = &g.root;
    g.longest(an)
}

/// A lumped part in the cascade (series element or shunt to AC ground).
#[derive(Debug, Clone)]
pub struct LumpedPart {
    pub label: String,
    pub series: bool,
    /// impedance model: R + jωL, or 1/(jωC) + R + jωL when `c` is set
    pub r: f64,
    pub l: f64,
    pub c: Option<f64>,
}

impl LumpedPart {
    pub fn z(&self, w: f64) -> Cx {
        let mut z = Cx::new(self.r, w * self.l);
        if let Some(c) = self.c {
            z = z + Cx::new(0.0, -1.0 / (w * c));
        }
        z
    }
    pub fn abcd(&self, w: f64) -> Abcd {
        let z = self.z(w);
        if self.series {
            Abcd::series(z)
        } else {
            Abcd::shunt(Cx::new(1.0, 0.0) / z)
        }
    }
}

/// Builds the section list of a channel (lines, vias, lumped parts).
pub(crate) struct Build<'a> {
    board: &'a Board,
    an: &'a Analysis,
    field: &'a HashMap<String, FieldResult>,
    rp: &'a RfParams,
    diff: bool,
    partner_of: Vec<Option<u32>>,
    pub segs: Vec<Segment>,
    models: Vec<Option<LineModel>>,
    via_models: Vec<Option<crate::via::ViaModel>>,
    lumped: Vec<Option<LumpedPart>>,
    /// open stubs (other branches of a multi-drop line): cascade per frequency
    branch: Vec<Option<Vec<Abcd>>>,
    pub via_info: Vec<crate::via::ViaInfo>,
    field_len: f64,
    total_len: f64,
    model_cache: HashMap<(u32, bool), Option<LineModel>>,
    pub warnings: Vec<String>,
}

impl<'a> Build<'a> {
    pub(crate) fn new(board: &'a Board, an: &'a Analysis, field: &'a HashMap<String, FieldResult>, rp: &'a RfParams, diff: bool) -> Self {
        Build {
            board,
            an,
            field,
            rp,
            diff,
            partner_of: crate::extract::diff_partners(&board.nets),
            segs: vec![],
            models: vec![],
            via_models: vec![],
            lumped: vec![],
            branch: vec![],
            via_info: vec![],
            field_len: 0.0,
            total_len: 0.0,
            model_cache: HashMap::new(),
            warnings: vec![],
        }
    }

    fn push(&mut self, seg: Segment, model: Option<LineModel>, via: Option<crate::via::ViaModel>, part: Option<LumpedPart>) {
        self.segs.push(seg);
        self.models.push(model);
        self.via_models.push(via);
        self.lumped.push(part);
        self.branch.push(None);
    }

    /// An open branch (stub) hanging off the line here; `ch` is the branch's own
    /// channel built on the same frequency grid, starting at the junction.
    pub(crate) fn add_branch(&mut self, label: String, ch: &Channel) {
        self.add_part(LumpedPart { label: label.clone(), series: false, r: 0.0, l: 0.0, c: None });
        let i = self.segs.len() - 1;
        self.segs[i].kind = "branch";
        self.segs[i].len_mm = ch.length_mm;
        self.lumped[i] = None;
        self.branch[i] = Some(ch.abcd.clone());
    }

    pub(crate) fn add_part(&mut self, part: LumpedPart) {
        let seg = Segment { kind: if part.series { "series" } else { "shunt" }, len_mm: 0.0, layer: 0, xs: None, z: None, via_lc: None, source: "estimate", part: Some(part.label.clone()) };
        self.push(seg, None, None, Some(part));
    }

    /// Line sections and vias along `path` (pieces of one net). Returns the end
    /// position of every piece (for placing shunts).
    pub(crate) fn add_path(&mut self, net: u32, path: &[(usize, bool)], mut on_piece: impl FnMut(&mut Self, P)) {
        let (board, an, rp) = (self.board, self.an, self.rp);
        let su = board.stackup();
        let partner = if self.diff { self.partner_of.get(net as usize).copied().flatten() } else { None };
        let refs = |xi: Option<u32>| -> Vec<u16> {
            xi.map(|i| {
                let x = &an.cross_sections[i as usize];
                [x.ref_up, x.ref_dn].into_iter().flatten().collect()
            })
            .unwrap_or_default()
        };
        let mut prev_layer: Option<(u16, P)> = None;
        let mut prev_xs: Option<u32> = None;
        for &(pi, rev) in path {
            let pc = &an.pieces[pi * 4..pi * 4 + 4];
            let tr = &board.tracks[pc[0] as usize];
            let xi = pc[3] as u32;
            let len = geom::len(geom::sub(tr.b, tr.a)) * (pc[2] - pc[1]);
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
                                self.warnings.push(format!("layer change at ({:.2}, {:.2}) mm without a nearby via: standard via 0.2/0.45 mm assumed", ppos[0], ppos[1]));
                            }
                            &fallback_via
                        }
                    };
                    let ctx = crate::via::ViaCtx { board, net, partner, refs_in: refs(prev_xs), refs_out: refs(Some(xi)), backdrill: rp.backdrill };
                    let (vm, info) = crate::via::build(&ctx, v, pl, tr.layer);
                    let (mut lt, mut ct) = (0.0, 0.0);
                    for s in &vm.barrel {
                        let vv = C0 / s.er.sqrt();
                        lt += s.z / vv * s.len;
                        ct += s.len / (s.z * vv);
                    }
                    let zv = if self.diff { info.z_via / 2.0 } else { info.z_via };
                    self.push(
                        Segment { kind: "via", len_mm: info.barrel_mm, layer: tr.layer, xs: None, z: Some(zv), via_lc: Some((lt * 1e9, ct * 1e12)), source: "via-model", part: None },
                        None,
                        Some(vm),
                        None,
                    );
                    self.via_info.push(info);
                }
            }
            let end = geom::lerp(tr.a, tr.b, if rev { pc[1] } else { pc[2] });
            prev_layer = Some((tr.layer, end));
            prev_xs = Some(xi);
            let xs = &an.cross_sections[xi as usize];
            let diff = self.diff;
            let field = self.field;
            let model = self
                .model_cache
                .entry((xi, diff))
                .or_insert_with(|| {
                    let fr = xs.geom.and_then(|g| field.get(&an.geometries[g as usize].id));
                    let odd = diff && xs.zdiff.is_some();
                    line_model(su, xs, fr, odd, rp).or_else(|| line_model(su, xs, fr, false, rp))
                })
                .clone();
            self.total_len += len;
            if model.as_ref().is_some_and(|m| m.source == "field") {
                self.field_len += len;
            }
            let z = model.as_ref().map(|m| m.z0);
            let merge = matches!(self.segs.last(), Some(last) if last.kind == "line" && last.xs == Some(xi));
            if merge {
                self.segs.last_mut().unwrap().len_mm += len;
            } else {
                self.push(Segment { kind: "line", len_mm: len, layer: tr.layer, xs: Some(xi), z, via_lc: None, source: model.as_ref().map(|m| m.source).unwrap_or("none"), part: None }, model, None, None);
            }
            on_piece(self, end);
        }
    }

    pub(crate) fn finish(mut self, net: u32, name: String, partner: Option<u32>, target: Option<(f64, bool)>) -> Channel {
        let rp = self.rp;
        let diff = self.diff;
        if self.models.iter().zip(&self.segs).any(|(m, s)| s.kind == "line" && m.is_none()) {
            self.warnings.push("sections without reference plane: approximated as 1.5 × nominal impedance".into());
        }
        // reference impedance
        let z_ref = rp.z_ref.unwrap_or(match target {
            Some((z, true)) if diff => z / 2.0,
            Some((z, false)) => z,
            _ => 50.0,
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
        let mut abcd = Vec::with_capacity(n);
        for (k, &f) in freq.iter().enumerate() {
            let w = 2.0 * PI * f;
            let mut m = Abcd::identity();
            for i in 0..self.segs.len() {
                let s = &self.segs[i];
                let e = match s.kind {
                    // open stub: input admittance C/A of its cascade
                    "branch" => match self.branch[i].as_ref().and_then(|b| b.get(k)) {
                        Some(b) => Abcd::shunt(b.c / b.a),
                        None => Abcd::identity(),
                    },
                    "via" => self.via_models[i].as_ref().map(|vm| vm.abcd(f, rp.sigma)).unwrap_or_else(Abcd::identity),
                    "series" | "shunt" => self.lumped[i].as_ref().map(|p| p.abcd(w)).unwrap_or_else(Abcd::identity),
                    _ => {
                        let lm = self.models[i].as_ref().unwrap_or(&fallback);
                        let p = lm.rlgc(f, rp);
                        Abcd::line(p.r, p.l, p.g, p.c, w, s.len_mm * 1e-3)
                    }
                };
                m = m.mul(&e);
            }
            let (a, b, _) = m.s_params(z_ref);
            s11.push(a);
            s21.push(b);
            abcd.push(m);
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
        for i in 0..self.segs.len() {
            match self.segs[i].kind {
                "line" => {
                    let lm = self.models[i].as_ref().unwrap_or(&fallback);
                    delay += self.segs[i].len_mm * 1e-3 * (lm.l_ext * lm.c0).sqrt();
                }
                "via" => {
                    if let Some(vm) = &self.via_models[i] {
                        delay += vm.delay();
                    }
                }
                _ => {}
            }
        }
        let total_len = self.total_len + self.via_info.iter().map(|v| v.barrel_mm).sum::<f64>();
        for v in &self.via_info {
            if let Some(fr) = v.stub_f_ghz {
                if fr * 1e9 < rp.fmax {
                    self.warnings.push(format!(
                        "via at ({:.2}, {:.2}) mm: stub {:.2} mm, quarter-wave resonance {:.1} GHz lies inside the analysed band",
                        v.at[0], v.at[1], v.stub_mm, fr
                    ));
                }
            }
            match v.return_kind {
                "missing" => self.warnings.push(format!("via at ({:.2}, {:.2}) mm: reference plane changes, no stitching via within 5 mm", v.at[0], v.at[1])),
                "decap" => self.warnings.push(format!("via at ({:.2}, {:.2}) mm: reference changes to another net, return current only through capacitors", v.at[0], v.at[1])),
                _ => {}
            }
        }
        // loss per inch of the longest line section at a few frequencies
        let mut loss_db_per_in_at = Vec::new();
        if let Some(i) = (0..self.segs.len()).filter(|&i| self.segs[i].kind == "line" && self.models[i].is_some()).max_by(|&a, &b| self.segs[a].len_mm.partial_cmp(&self.segs[b].len_mm).unwrap()) {
            let lm = self.models[i].as_ref().unwrap();
            for fg in [1.0, 4.0, 8.0, 16.0] {
                if fg * 1e9 > rp.fmax {
                    continue;
                }
                let p = lm.rlgc(fg * 1e9, rp);
                let zc = (p.l / p.c).sqrt();
                let a_c = p.r / (2.0 * zc);
                let a_d = p.g * zc / 2.0;
                let k = 8.686 * 0.0254;
                loss_db_per_in_at.push((fg, a_c * k, a_d * k));
            }
        }
        // TDR: step response from S11 with a Gaussian edge of the given rise time
        let (tdr_t_ps, tdr_z, tdr_x_mm) = tdr(&freq, &s11, z_ref, rp.rise, (2.0 * delay * 1.3 + 4.0 * rp.rise).max(200e-12), delay, total_len);
        let vias = self.segs.iter().filter(|s| s.kind == "via").count();
        Channel {
            net,
            name,
            diff,
            partner,
            z_ref,
            length_mm: total_len,
            delay_ps: delay * 1e12,
            vias,
            segments: self.segs,
            freq,
            s21_db,
            s11_db,
            il_marks,
            tdr_t_ps,
            tdr_z,
            tdr_x_mm,
            loss_db_per_in_at,
            via_info: self.via_info,
            field_share: if total_len > 0.0 { self.field_len / total_len } else { 0.0 },
            warnings: self.warnings,
            s11,
            s21,
            abcd,
        }
    }
}

pub fn channel(board: &Board, an: &Analysis, net: u32, field: &HashMap<String, FieldResult>, target: Option<(f64, bool)>, rp: &RfParams) -> Result<Channel, String> {
    let mut warnings = Vec::new();
    let partner = crate::extract::diff_partners(&board.nets).get(net as usize).copied().flatten();
    let diff = partner.is_some() && target.map(|t| t.1).unwrap_or(true);
    let path = net_path(board, an, net, &mut warnings);
    if path.is_empty() {
        return Err("net has no tracks".into());
    }
    let mut b = Build::new(board, an, field, rp, diff);
    b.warnings = warnings;
    b.add_path(net, &path, |_, _| {});
    Ok(b.finish(net, board.net_name(net).to_string(), partner, target))
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


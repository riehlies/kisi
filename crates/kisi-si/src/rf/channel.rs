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
    /// position along the path where the segment starts (mm of track, vias add none)
    pub s_mm: f64,
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
    /// return loss seen from the far end (port 2); S12 = S21 (passive, reciprocal)
    pub s22_db: Vec<f64>,
    /// insertion loss at Nyquist-like frequencies: (GHz, dB)
    pub il_marks: Vec<(f64, f64)>,
    pub tdr_t_ps: Vec<f64>,
    pub tdr_z: Vec<f64>,
    /// TDR position along the path (track mm, from the delay of every section; beyond the end
    /// extrapolated with the mean velocity)
    pub tdr_x_mm: Vec<f64>,
    /// end of the line on the TDR position axis
    pub tdr_end_mm: f64,
    pub loss_db_per_in_at: Vec<(f64, f64, f64)>,
    pub via_info: Vec<crate::via::ViaInfo>,
    pub field_share: f64,
    pub warnings: Vec<String>,
    /// the path with its ends and branches (channel of a net; None for lanes)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<NetPath>,
    #[serde(skip)]
    pub s11: Vec<Cx>,
    #[serde(skip)]
    pub s21: Vec<Cx>,
    #[serde(skip)]
    pub s22: Vec<Cx>,
    /// cascade matrix of the whole channel per frequency (for other terminations)
    #[serde(skip)]
    pub abcd: Vec<Abcd>,
    /// the same at 0 Hz: the DC point of the time-domain responses (`Build::seg_abcd_dc`)
    #[serde(skip)]
    pub abcd_dc: Abcd,
    /// receivers on side stubs (multi-drop): each stub's cascade; `abcd` has them as open stubs
    #[serde(skip)]
    pub taps: Vec<Tap>,
    /// with taps: the main path split at them (cascade per frequency and at 0 Hz)
    #[serde(skip)]
    pub pieces: Vec<(Vec<Abcd>, Abcd)>,
}

/// A receiver on a side stub of a multi-drop line: the stub from the junction on the main path
/// (behind piece `after` of `Channel::pieces`) to the receiver's pad.
#[derive(Debug, Clone)]
pub struct Tap {
    /// the receiver's pin, e.g. "U3.5"
    pub label: String,
    pub after: usize,
    pub abcd: Vec<Abcd>,
    pub dc: Abcd,
}

/// Track graph of one net: nodes are joined track ends (per layer; layers join
/// only through vias and plated holes), edges are tracks.
pub(crate) struct NetGraph {
    /// (track, node a, node b, length mm)
    edges: Vec<(usize, usize, usize, f64)>,
    adj: HashMap<usize, Vec<(usize, usize)>>,
    /// position of every node (after joining: representative positions)
    pos: Vec<P>,
    #[allow(dead_code)]
    root: Vec<usize>,
    pieces_of: HashMap<usize, Vec<usize>>,
    /// copper layers of every node (bit per layer; a via node has all layers it joins)
    mask: Vec<u64>,
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
            let tol = 0.02;
            // a synthetic pad track stands for its pad: what lies on the pad's copper joins its
            // centre (not a capsule around the track, which would reach beyond the pad's ends)
            let pad = if t.pad { board.pads.get(t.item as usize) } else { None };
            for i in 0..n {
                // only copper on the same layer connects (vias are joined below)
                if node_layers[i] & (1u64 << t.layer.min(63)) == 0 {
                    continue;
                }
                if let Some(pd) = pad {
                    if crate::board::pad_contains(pd, pos[i], 0.02) {
                        let a = node_id[&key(t.a, t.layer)];
                        let (ra, rb) = (find(&mut parent, a), find(&mut parent, i));
                        if ra != rb {
                            parent[rb] = ra;
                        }
                    }
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
        let mut mask = vec![0u64; n];
        for i in 0..n {
            mask[root[i]] |= node_layers[i];
        }
        let edges: Vec<(usize, usize, usize, f64)> = edges.into_iter().map(|(t, a, b, l)| (t, root[a], root[b], l)).collect();
        let mut adj: HashMap<usize, Vec<(usize, usize)>> = HashMap::new(); // node -> (edge idx, other node)
        for (ei, &(_, a, b, _)) in edges.iter().enumerate() {
            if a == b {
                continue;
            }
            adj.entry(a).or_default().push((ei, b));
            adj.entry(b).or_default().push((ei, a));
        }
        Some(NetGraph { edges, adj, pos, root, pieces_of, mask })
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

    /// Node closest to a point among the nodes on the copper layers in `layers` (bit per
    /// layer, 0 = any layer; falls back to any layer). Parts placed back to back on F.Cu and
    /// B.Cu have their pads at the same position: the layer tells them apart.
    pub(crate) fn nearest_on(&self, p: P, layers: u64) -> Option<usize> {
        let best = |want: u64| {
            self.adj
                .keys()
                .copied()
                .filter(|&n| want == 0 || self.mask.get(n).is_some_and(|m| m & want != 0))
                .min_by(|&a, &b| geom::len(geom::sub(self.pos[a], p)).partial_cmp(&geom::len(geom::sub(self.pos[b], p))).unwrap().then(a.cmp(&b)))
        };
        best(layers).or_else(|| best(0))
    }

    /// The two nodes farthest apart (deterministic).
    fn far_pair(&self) -> (usize, usize) {
        let far = |start: usize| -> usize {
            let dist = self.reach(start);
            dist.iter().max_by(|a, b| a.1 .0.partial_cmp(&b.1 .0).unwrap().then(b.0.cmp(a.0))).map(|x| *x.0).unwrap_or(start)
        };
        let start = self.adj.keys().copied().min().unwrap_or(0);
        let a = far(start);
        (a, far(a))
    }

    /// Distances and predecessors from `start`, not entering `blocked` nodes.
    fn reach_from(&self, start: usize, blocked: &std::collections::HashSet<usize>) -> HashMap<usize, (f64, Option<(usize, usize)>)> {
        let mut dist: HashMap<usize, (f64, Option<(usize, usize)>)> = HashMap::new();
        dist.insert(start, (0.0, None));
        let mut stack = vec![start];
        while let Some(u) = stack.pop() {
            let du = dist[&u].0;
            for &(ei, v) in self.adj.get(&u).map(|x| x.as_slice()).unwrap_or(&[]) {
                if !dist.contains_key(&v) && !blocked.contains(&v) {
                    dist.insert(v, (du + self.edges[ei].3, Some((ei, u))));
                    stack.push(v);
                }
            }
        }
        dist
    }

    /// Edges from the start of `dist` to `to`: (edge, from node, to node).
    fn chain(dist: &HashMap<usize, (f64, Option<(usize, usize)>)>, to: usize) -> Vec<(usize, usize, usize)> {
        let mut out = Vec::new();
        let mut cur = to;
        while let Some((_, Some((ei, prev)))) = dist.get(&cur).copied() {
            out.push((ei, prev, cur));
            cur = prev;
        }
        out.reverse();
        out
    }

    /// Pieces along a chain of edges with their positions from `s` on.
    fn pieces_along(&self, board: &Board, an: &Analysis, chain: &[(usize, usize, usize)], mut s: f64, mut after: impl FnMut(usize, f64, usize)) -> (Vec<PathPiece>, f64) {
        let mut out = Vec::new();
        for &(ei, from, to) in chain {
            let (ti, ea, _, _) = self.edges[ei];
            let fwd = ea == from;
            let mut ps = self.pieces_of[&ti].clone();
            ps.sort_by(|x, y| an.pieces[x * 4 + 1].partial_cmp(&an.pieces[y * 4 + 1]).unwrap());
            if !fwd {
                ps.reverse();
            }
            let tl = geom::len(geom::sub(board.tracks[ti].b, board.tracks[ti].a));
            for p in ps {
                let l = tl * (an.pieces[p * 4 + 2] - an.pieces[p * 4 + 1]) as f64;
                out.push(PathPiece { piece: p, rev: !fwd, s0: s, s1: s + l });
                s += l;
            }
            after(to, s, out.len());
        }
        (out, s)
    }

    fn layer_at(&self, board: &Board, node: usize) -> u16 {
        self.adj.get(&node).and_then(|v| v.first()).map(|&(ei, _)| board.tracks[self.edges[ei].0].layer).unwrap_or(0)
    }

}

// --- path with ends and branches (impedance profile, TDR, board markers) --------------------

/// One end of a path or branch: the pad of a part, a via or an open track end.
#[derive(Debug, Clone, Serialize)]
pub struct PathEnd {
    pub at: P,
    pub layer: u16,
    /// "pad", "via" or "open"
    pub kind: &'static str,
    pub refdes: Option<String>,
    pub pad: Option<String>,
    /// "U18.A3", "via", "open end"
    pub label: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PathPiece {
    pub piece: usize,
    pub rev: bool,
    /// position along the path (mm)
    pub s0: f64,
    pub s1: f64,
}

/// A branch off the main path (stub, test point, other device of a multi-drop line).
#[derive(Debug, Clone, Serialize)]
pub struct PathBranch {
    /// position of the junction on the main path (mm) and on the board
    pub at_s: f64,
    pub at: P,
    pub len_mm: f64,
    pub end: PathEnd,
    /// pieces from the junction (positions from 0 at the junction)
    pub pieces: Vec<PathPiece>,
    /// included in the channel as an open stub (shorter ones are only marked)
    pub modelled: bool,
    /// the branch branches again (only its longest way is followed)
    pub sub_branches: bool,
    #[serde(skip)]
    pub(crate) after_piece: usize,
}

/// Main path of a net from `start` to `end`, with its branches.
#[derive(Debug, Clone, Serialize)]
pub struct NetPath {
    pub net: u32,
    pub length_mm: f64,
    pub start: PathEnd,
    pub end: PathEnd,
    pub pieces: Vec<PathPiece>,
    pub branches: Vec<PathBranch>,
    /// how the start was chosen
    pub direction: String,
    /// reversed against the automatic choice (by the user)
    pub flipped: bool,
    /// copper parts of the net that are not connected to this path (through parts)
    pub parts: usize,
}

/// Point on the board at position `s` (mm) along the main path.
pub fn path_point(board: &Board, an: &Analysis, path: &NetPath, s: f64) -> P {
    let pc = path.pieces.iter().find(|p| s <= p.s1).or(path.pieces.last());
    match pc {
        Some(p) => {
            let q = &an.pieces[p.piece * 4..p.piece * 4 + 4];
            let tr = &board.tracks[q[0] as usize];
            let f = if p.s1 > p.s0 { ((s - p.s0) / (p.s1 - p.s0)).clamp(0.0, 1.0) } else { 0.0 };
            let (t0, t1) = if p.rev { (q[2], q[1]) } else { (q[1], q[2]) };
            geom::lerp(tr.a, tr.b, t0 + (t1 - t0) * f)
        }
        None => path.start.at,
    }
}

/// Branches shorter than this are marked but not modelled.
pub const BRANCH_MIN_MM: f64 = 0.5;

fn path_end(board: &Board, net: u32, at: P, layer: u16) -> PathEnd {
    // pads on the node's layer first (parts back to back on F.Cu and B.Cu share positions)
    let near = |p: &&crate::board::Pad| p.net == net && geom::len(geom::sub(p.at, at)) <= p.size[0].max(p.size[1]) / 2.0 + 0.05;
    let closest = |a: &&crate::board::Pad, b: &&crate::board::Pad| geom::len(geom::sub(a.at, at)).partial_cmp(&geom::len(geom::sub(b.at, at))).unwrap();
    let pad = board.pads.iter().filter(near).filter(|p| p.layers.contains(&layer)).min_by(closest).or_else(|| board.pads.iter().filter(near).min_by(closest));
    if let Some(p) = pad {
        let r = board.footprints[p.fp as usize].reference.clone();
        return PathEnd { at: p.at, layer, kind: "pad", label: format!("{}.{}", r, p.number), refdes: Some(r), pad: Some(p.number.clone()) };
    }
    if board.vias.iter().any(|v| v.net == net && geom::len(geom::sub(v.at, at)) <= v.size / 2.0 + 0.05) {
        return PathEnd { at, layer, kind: "via", refdes: None, pad: None, label: "via".into() };
    }
    PathEnd { at, layer, kind: "open", refdes: None, pad: None, label: "open end".into() }
}

/// Preference of an end as the start: ICs (drivers) first, connectors and test points last.
fn start_score(e: &PathEnd) -> i32 {
    let Some(r) = &e.refdes else { return 0 };
    let pre: String = r.chars().take_while(|c| c.is_ascii_alphabetic()).collect::<String>().to_ascii_uppercase();
    match pre.as_str() {
        "U" | "IC" => 3,
        "J" | "P" | "CN" | "X" | "CON" | "SW" => 1,
        "TP" => 0,
        _ => 2,
    }
}

/// Main path of a net with ends and branches. The start is the end nearest to `hint`
/// (e.g. the transmitter side of a lane) or else the IC end; `flip` reverses that choice.
pub fn net_path_info(board: &Board, an: &Analysis, net: u32, flip: bool, hint: Option<P>) -> Option<NetPath> {
    let g = NetGraph::build(board, an, net)?;
    let parts = g.components();
    let end_of = |n: usize| path_end(board, net, g.pos[n], g.layer_at(board, n));
    // ends: the two farthest apart among the leaves at parts (not test points, vias or open
    // ends) if there are two; else the two farthest apart overall
    let (fa, _) = g.far_pair();
    let in_part = g.reach(fa);
    let mut good: Vec<usize> = g.adj.iter().filter(|(n, v)| v.len() == 1 && in_part.contains_key(n)).map(|(n, _)| *n).filter(|&n| start_score(&end_of(n)) >= 1).collect();
    good.sort();
    let (a, b) = if good.len() >= 2 {
        let mut best = (good[0], good[1], -1.0);
        for &x in &good {
            let d = g.reach(x);
            for &y in &good {
                if y > x && d.get(&y).is_some_and(|v| v.0 > best.2) {
                    best = (x, y, d[&y].0);
                }
            }
        }
        (best.0, best.1)
    } else {
        g.far_pair()
    };
    let (ea, eb) = (end_of(a), end_of(b));
    let (mut s_node, mut e_node, mut s_end, mut e_end, direction);
    let dist = |e: &PathEnd, h: P| geom::len(geom::sub(e.at, h));
    if let Some(h) = hint.filter(|_| a != b) {
        let swap = dist(&eb, h) < dist(&ea, h);
        (s_node, e_node, s_end, e_end) = if swap { (b, a, eb, ea) } else { (a, b, ea, eb) };
        direction = "lane direction (transmitter first)".to_string();
    } else if start_score(&eb) > start_score(&ea) {
        (s_node, e_node, s_end, e_end) = (b, a, eb, ea);
        direction = "starts at the IC side".to_string();
    } else {
        let d = if start_score(&ea) > start_score(&eb) { "starts at the IC side" } else { "no preferred end (both ends of the same kind)" };
        (s_node, e_node, s_end, e_end) = (a, b, ea, eb);
        direction = d.to_string();
    }
    if flip {
        std::mem::swap(&mut s_node, &mut e_node);
        std::mem::swap(&mut s_end, &mut e_end);
    }
    let none = std::collections::HashSet::new();
    let main = NetGraph::chain(&g.reach_from(s_node, &none), e_node);
    let mut node_at: HashMap<usize, (f64, usize)> = HashMap::new(); // node -> (s, pieces so far)
    node_at.insert(s_node, (0.0, 0));
    let (pieces, length) = g.pieces_along(board, an, &main, 0.0, |n, s, k| {
        node_at.insert(n, (s, k));
    });
    let branches = collect_branches(&g, board, an, net, s_node, &main, &node_at, [&s_end.label, &e_end.label]);
    Some(NetPath { net, length_mm: length, start: s_end, end: e_end, pieces, branches, direction, flipped: flip, parts })
}

/// Branches leaving the main path `main` (edges from `s_node`; `node_at`: position along the
/// path and pieces so far per main node). A branch follows its longest way to an end.
#[allow(clippy::too_many_arguments)]
fn collect_branches(g: &NetGraph, board: &Board, an: &Analysis, net: u32, s_node: usize, main: &[(usize, usize, usize)], node_at: &HashMap<usize, (f64, usize)>, own_ends: [&str; 2]) -> Vec<PathBranch> {
    let end_of = |n: usize| path_end(board, net, g.pos[n], g.layer_at(board, n));
    let main_nodes: std::collections::HashSet<usize> = node_at.keys().copied().collect();
    let main_edges: std::collections::HashSet<usize> = main.iter().map(|x| x.0).collect();
    let mut branches = Vec::new();
    for &(_, _, n) in std::iter::once(&(usize::MAX, usize::MAX, s_node)).chain(main.iter()) {
        let (s_at, after) = node_at[&n];
        for &(ei, v) in g.adj.get(&n).map(|x| x.as_slice()).unwrap_or(&[]) {
            if main_edges.contains(&ei) || main_nodes.contains(&v) {
                continue;
            }
            let dist = g.reach_from(v, &main_nodes);
            let far = dist.iter().max_by(|x, y| x.1 .0.partial_cmp(&y.1 .0).unwrap().then(y.0.cmp(x.0))).map(|x| *x.0).unwrap_or(v);
            let mut ch = vec![(ei, n, v)];
            ch.extend(NetGraph::chain(&dist, far));
            let (bp, len) = g.pieces_along(board, an, &ch, 0.0, |_, _, _| {});
            let sub_branches = dist.keys().any(|k| g.adj.get(k).map(|x| x.len()).unwrap_or(0) > 2);
            let bend = end_of(far);
            // copper inside the pad of the path's own start or end (pad outline, rounding, the
            // pad's own synthetic track)
            let only_pad = bp.iter().all(|p| board.tracks[an.pieces[p.piece * 4] as usize].pad);
            if bend.kind == "pad" && own_ends.contains(&bend.label.as_str()) && (len < 1.0 || only_pad) {
                continue;
            }
            branches.push(PathBranch {
                at_s: s_at,
                at: g.pos[n],
                len_mm: len,
                end: bend,
                pieces: bp,
                modelled: len >= BRANCH_MIN_MM,
                sub_branches,
                after_piece: after,
            });
        }
    }
    branches.sort_by(|x, y| x.at_s.partial_cmp(&y.at_s).unwrap());
    branches
}

/// Path of one net between two points – a hop of a lane, from pad to pad – with the branches
/// that leave it (open stubs: unused pads, test points, the copper to parts that are not
/// fitted). `from_layers`/`to_layers`: copper layers of the pads at the ends (bit per layer,
/// 0 = any). None if the net has no copper.
pub(crate) fn hop_path(board: &Board, an: &Analysis, net: u32, from: (P, u64), to: (P, u64)) -> Option<NetPath> {
    let g = NetGraph::build(board, an, net)?;
    let (s_node, e_node) = (g.nearest_on(from.0, from.1)?, g.nearest_on(to.0, to.1)?);
    let end_of = |n: usize| path_end(board, net, g.pos[n], g.layer_at(board, n));
    let main = if s_node == e_node { vec![] } else { NetGraph::chain(&g.reach_from(s_node, &std::collections::HashSet::new()), e_node) };
    let mut node_at: HashMap<usize, (f64, usize)> = HashMap::new();
    node_at.insert(s_node, (0.0, 0));
    let (pieces, length) = g.pieces_along(board, an, &main, 0.0, |n, s, k| {
        node_at.insert(n, (s, k));
    });
    let (s_end, e_end) = (end_of(s_node), end_of(e_node));
    let branches = collect_branches(&g, board, an, net, s_node, &main, &node_at, [&s_end.label, &e_end.label]);
    Some(NetPath { net, length_mm: length, start: s_end, end: e_end, pieces, branches, direction: "lane hop".into(), flipped: false, parts: g.components() })
}


/// Length of copper (mm) from the point `from` to each of the points `to` along the tracks of
/// `net` (points with their copper layers, bit masks, 0 = any); None where not connected.
pub(crate) fn copper_distances(board: &Board, an: &Analysis, net: u32, from: (P, u64), to: &[(P, u64)]) -> Vec<Option<f64>> {
    let Some(g) = NetGraph::build(board, an, net) else { return vec![None; to.len()] };
    let Some(s) = g.nearest_on(from.0, from.1) else { return vec![None; to.len()] };
    let d = g.reach(s);
    to.iter().map(|&(p, l)| g.nearest_on(p, l).and_then(|n| d.get(&n).map(|x| x.0))).collect()
}

/// A lumped part in the cascade (series element or shunt to AC ground).
#[derive(Debug, Clone)]
pub struct LumpedPart {
    pub label: String,
    pub series: bool,
    /// 'R', 'C' (also pad capacitances) or 'L' – how the part acts at 0 Hz (`abcd_dc`)
    pub kind: char,
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

    /// At 0 Hz for the DC point of a time-domain response: the resistances of the signal band.
    /// A capacitor in series (AC coupling) is a short – the receiver biases itself, the coupling
    /// corner lies far below the PRBS repetition rate –, a shunt capacitor is open, and an
    /// inductor to AC ground (a bias choke) is open as well: both act below the signal band.
    /// (`rf::Step` falls back to the lowest frequency when the channel changes below it.)
    pub fn abcd_dc(&self) -> Abcd {
        if self.series {
            Abcd::series(Cx::new(self.r, 0.0))
        } else if self.kind != 'R' || self.r <= 0.0 {
            Abcd::identity()
        } else {
            Abcd::shunt(Cx::new(1.0 / self.r, 0.0))
        }
    }
}

/// A lumped part inside a path: in piece `step` (index along the path), at fraction `frac` of
/// that piece's length in the path's direction.
#[derive(Debug, Clone)]
pub(crate) struct Cut {
    pub step: usize,
    pub frac: f64,
    pub part: LumpedPart,
}

/// A part from a net to AC ground (a shunt capacitor, a pull resistor, an inductor) or across
/// a pair, at its pad: placed where that pad lies on the net's path (`kisi_si::topology::net_parts`).
#[derive(Debug, Clone)]
pub struct NetPart {
    pub refdes: String,
    /// how it is named in the channel's notes, e.g. "C3 100nF (0805)"
    pub note: String,
    /// pad position and its copper layers (bit per layer, 0 = through-hole)
    pub at: P,
    pub layers: u64,
    pub part: LumpedPart,
}

/// Where a point lies on a path of pieces: (piece index along the path, fraction of that piece
/// in the path's direction, distance mm). Pieces on the copper layers `layers` (bit mask, 0 =
/// any) are preferred: a piece on another layer counts 100 mm farther.
pub(crate) fn locate_on_path(board: &Board, an: &Analysis, path: &[(usize, bool)], at: P, layers: u64) -> Option<(usize, f64, f64)> {
    let mut best: Option<(usize, f64, f64, f64)> = None;
    for (step, &(pi, rev)) in path.iter().enumerate() {
        let pc = &an.pieces[pi * 4..pi * 4 + 4];
        let tr = &board.tracks[pc[0] as usize];
        let (t0, t1) = (pc[1], pc[2]);
        let d = geom::sub(tr.b, tr.a);
        let l2 = geom::dot(d, d).max(1e-12);
        let t = (geom::dot(geom::sub(at, tr.a), d) / l2).clamp(t0, t1);
        let dist = geom::len(geom::sub(at, geom::lerp(tr.a, tr.b, t)));
        let frac = if t1 > t0 { (t - t0) / (t1 - t0) } else { 0.0 };
        let frac = if rev { 1.0 - frac } else { frac };
        // another copper layer counts as far away (a through-hole pad reaches every layer)
        let score = dist + if layers != 0 && layers & (1u64 << tr.layer.min(63)) == 0 { 100.0 } else { 0.0 };
        if best.is_none_or(|b| score < b.3 - 1e-12) {
            best = Some((step, frac, dist, score));
        }
    }
    best.map(|b| (b.0, b.1, b.3))
}

/// Builds the section list of a channel (lines, vias, lumped parts).
pub(crate) struct Build<'a> {
    pub(crate) board: &'a Board,
    pub(crate) an: &'a Analysis,
    pub(crate) field: &'a HashMap<String, FieldResult>,
    pub(crate) rp: &'a RfParams,
    diff: bool,
    /// also build the odd-mode model of every via with the partner net (4-port pair model)
    pub(crate) pair: bool,
    partner_of: Vec<Option<u32>>,
    pub segs: Vec<Segment>,
    pub(crate) models: Vec<Option<LineModel>>,
    pub(crate) via_models: Vec<Option<crate::via::ViaModel>>,
    /// odd-mode via model with the partner via (only with `pair`)
    pub(crate) via_pair: Vec<Option<crate::via::ViaModel>>,
    pub(crate) lumped: Vec<Option<LumpedPart>>,
    /// open stubs (other branches of a multi-drop line): cascade per frequency
    pub(crate) branch: Vec<Option<Vec<Abcd>>>,
    /// their cascade at 0 Hz
    pub(crate) branch_dc: Vec<Option<Abcd>>,
    /// one line of a pair's 4-port (`pair`): the stub also in the odd mode with the partner's
    /// stub (pairs up with the partner's stub in `rf::mixed::align_pair`)
    pub(crate) branch_odd: Vec<Option<Vec<Abcd>>>,
    /// the stub ends at a receiver (multi-drop): its pin label
    pub(crate) branch_tap: Vec<Option<String>>,
    pub via_info: Vec<crate::via::ViaInfo>,
    field_len: f64,
    pub(crate) total_len: f64,
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
            pair: false,
            partner_of: crate::extract::diff_partners(&board.nets),
            segs: vec![],
            models: vec![],
            via_models: vec![],
            via_pair: vec![],
            lumped: vec![],
            branch: vec![],
            branch_dc: vec![],
            branch_odd: vec![],
            branch_tap: vec![],
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
        self.via_pair.push(None);
        self.lumped.push(part);
        self.branch.push(None);
        self.branch_dc.push(None);
        self.branch_odd.push(None);
        self.branch_tap.push(None);
    }

    /// An open branch (stub) hanging off the line here; `ch` is the branch's own
    /// channel built on the same frequency grid, starting at the junction.
    pub(crate) fn add_branch(&mut self, label: String, ch: &Channel) {
        self.add_part(LumpedPart { label: label.clone(), series: false, kind: 'C', r: 0.0, l: 0.0, c: None });
        let i = self.segs.len() - 1;
        self.segs[i].kind = "branch";
        self.segs[i].len_mm = ch.length_mm;
        self.lumped[i] = None;
        self.branch[i] = Some(ch.abcd.clone());
        self.branch_dc[i] = Some(ch.abcd_dc);
    }

    /// As `add_branch`, with the stub's odd-mode cascade (`odd`, the same copper built with the
    /// pair's odd-mode models) for a line of a pair's 4-port.
    pub(crate) fn add_branch_pair(&mut self, label: String, ch: &Channel, odd: Option<&Channel>) {
        self.add_branch(label, ch);
        *self.branch_odd.last_mut().unwrap() = odd.map(|o| o.abcd.clone());
    }

    /// Mark the branch just added as ending at the receiver `pin` (a tap of a multi-drop line).
    pub(crate) fn mark_tap(&mut self, pin: String) {
        *self.branch_tap.last_mut().unwrap() = Some(pin);
    }

    pub(crate) fn add_part(&mut self, part: LumpedPart) {
        let seg = Segment { kind: if part.series { "series" } else { "shunt" }, len_mm: 0.0, layer: 0, xs: None, z: None, via_lc: None, source: "estimate", part: Some(part.label.clone()), s_mm: 0.0 };
        self.push(seg, None, None, Some(part));
    }

    /// A new, empty build with the same board, models and mode (for stubs).
    pub(crate) fn sub(&self) -> Build<'a> {
        Build::new(self.board, self.an, self.field, self.rp, self.diff)
    }

    /// Line sections and vias along `path` (pieces of one net), for a path that may start on
    /// another path at a junction: `start` = (layer,
    /// position, cross-section) of the other path there. A layer change at the start (a stub
    /// that leaves through a via) adds that via. `before(build, k)` runs before the line of
    /// piece `k` (after the via in front of it, if any), `on_piece` after it.
    pub(crate) fn add_path_with(&mut self, net: u32, path: &[(usize, bool)], start: Option<(u16, P, u32)>, before: impl FnMut(&mut Self, usize), on_piece: impl FnMut(&mut Self, P)) {
        self.add_path_cuts(net, path, start, &[], before, on_piece)
    }

    /// As `add_path_with`, with lumped parts inside the pieces (`cuts`: piece index along
    /// `path`, fraction of its length in the path's direction).
    pub(crate) fn add_path_cuts(&mut self, net: u32, path: &[(usize, bool)], start: Option<(u16, P, u32)>, cuts: &[Cut], mut before: impl FnMut(&mut Self, usize), mut on_piece: impl FnMut(&mut Self, P)) {
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
        let mut prev_layer: Option<(u16, P)> = start.map(|s| (s.0, s.1));
        let mut prev_xs: Option<u32> = start.map(|s| s.2);
        // reference planes next to a via: the trace right at the via often runs over the antipad
        // (no reference there), so take the last referenced piece before it and the next after it
        let path_xs: Vec<u32> = path.iter().map(|&(pi, _)| an.pieces[pi * 4 + 3] as u32).collect();
        let mut last_ref_xs: Option<u32> = None;
        let mut prev_width = 0.1;
        // pieces after a via that the ladder model covers (left out of the cascade)
        let mut skip_until = 0usize;
        for (step, &(pi, rev)) in path.iter().enumerate() {
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
                                    remove_unused: None,
                                },
                                None => crate::board::Via { at: ppos, size: 0.45, drill: 0.2, from: 0, to: (su.n() - 1) as u16, net, backdrill: None, remove_unused: None },
                            };
                            if th.is_none() {
                                self.warnings.push(format!("layer change at ({:.2}, {:.2}) mm without a nearby via: standard via 0.2/0.45 mm assumed", ppos[0], ppos[1]));
                            }
                            &fallback_via
                        }
                    };
                    let next_ref = path_xs[step..].iter().copied().find(|&x| !refs(Some(x)).is_empty()).unwrap_or(xi);
                    // feed traces for the ladder model: the referenced trace on each side; the
                    // unreferenced pieces between it and the via (over the antipad) are left out
                    // of the cascade, the ladder models them (up to 1.5 mm per side)
                    let feeds = {
                        let mut drop_in = vec![];
                        let mut len_in = 0.0;
                        let mut feed_in = None;
                        for i in (0..self.segs.len()).rev() {
                            let sg = &self.segs[i];
                            if sg.kind != "line" {
                                break;
                            }
                            if refs(sg.xs).is_empty() {
                                drop_in.push(i);
                                len_in += sg.len_mm;
                                continue;
                            }
                            feed_in = self.models[i].clone().map(|m| (m, sg.xs));
                            break;
                        }
                        let mut len_out = 0.0;
                        let mut n_out = 0;
                        let mut feed_out = None;
                        for (j, &(pj, _)) in path.iter().enumerate().skip(step) {
                            let x = path_xs[j];
                            let pcj = &an.pieces[pj * 4..pj * 4 + 4];
                            let trj = &board.tracks[pcj[0] as usize];
                            if trj.layer != tr.layer {
                                break;
                            }
                            if refs(Some(x)).is_empty() {
                                len_out += geom::len(geom::sub(trj.b, trj.a)) * (pcj[2] - pcj[1]);
                                n_out += 1;
                                continue;
                            }
                            feed_out = self.line_model_for(x).map(|m| (m, trj.width));
                            break;
                        }
                        let w_in = prev_width;
                        match (feed_in, feed_out) {
                            (Some((mi, _)), Some((mo, w_out))) if len_in <= 1.5 && len_out <= 1.5 => {
                                for i in drop_in {
                                    self.segs[i].len_mm = 0.0;
                                }
                                skip_until = step + n_out;
                                Some([
                                    crate::via::FeedSpec { w: w_in, zc: mi.z0, eeff: mi.eeff0, dropped: len_in },
                                    crate::via::FeedSpec { w: w_out, zc: mo.z0, eeff: mo.eeff0, dropped: len_out },
                                ])
                            }
                            _ => None,
                        }
                    };
                    let ctx = crate::via::ViaCtx { board, net, partner, refs_in: refs(last_ref_xs.or(prev_xs)), refs_out: refs(Some(next_ref)), backdrill: rp.backdrill, feeds, sigma: Some(rp.sigma) };
                    let (vm, info) = crate::via::build(&ctx, v, pl, tr.layer);
                    let pair_partner = self.partner_of.get(net as usize).copied().flatten().filter(|_| self.pair);
                    let vm_pair = pair_partner.map(|pn| crate::via::build(&crate::via::ViaCtx { partner: Some(pn), ..ctx }, v, pl, tr.layer).0);
                    let (mut lt, mut ct) = (0.0, 0.0);
                    for s in &vm.barrel {
                        let vv = C0 / s.er.sqrt();
                        lt += s.z / vv * s.len;
                        ct += s.len / (s.z * vv);
                    }
                    let zv = if self.diff { info.z_via / 2.0 } else { info.z_via };
                    self.push(
                        Segment { kind: "via", len_mm: info.barrel_mm, layer: tr.layer, xs: None, z: Some(zv), via_lc: Some((lt * 1e9, ct * 1e12)), source: "via-model", part: None, s_mm: 0.0 },
                        None,
                        Some(vm),
                        None,
                    );
                    *self.via_pair.last_mut().unwrap() = vm_pair;
                    self.via_info.push(info);
                }
            }
            let end = geom::lerp(tr.a, tr.b, if rev { pc[1] } else { pc[2] });
            prev_layer = Some((tr.layer, end));
            if !refs(Some(xi)).is_empty() {
                prev_width = tr.width;
            }
            prev_xs = Some(xi);
            if !refs(Some(xi)).is_empty() {
                last_ref_xs = Some(xi);
            }
            before(self, step);
            let model = self.line_model_for(xi);
            self.total_len += len;
            if model.as_ref().is_some_and(|m| m.source == "field") {
                self.field_len += len;
            }
            let z = model.as_ref().map(|m| m.z0);
            // pieces over the antipad that the via's ladder model covers
            let len = if step < skip_until { 0.0 } else { len };
            let push_line = |b: &mut Self, l: f64| {
                let merge = matches!(b.segs.last(), Some(last) if last.kind == "line" && last.xs == Some(xi));
                if merge {
                    b.segs.last_mut().unwrap().len_mm += l;
                } else {
                    b.push(Segment { kind: "line", len_mm: l, layer: tr.layer, xs: Some(xi), z, via_lc: None, source: model.as_ref().map(|m| m.source).unwrap_or("none"), part: None, s_mm: 0.0 }, model.clone(), None, None);
                }
            };
            // parts inside this piece, in the path's direction
            let mut mine: Vec<&Cut> = cuts.iter().filter(|c| c.step == step).collect();
            mine.sort_by(|a, b| a.frac.total_cmp(&b.frac));
            let mut done = 0.0;
            for c in mine {
                let f = c.frac.clamp(done, 1.0);
                if f > done {
                    push_line(self, len * (f - done));
                }
                self.add_part(c.part.clone());
                done = f;
            }
            push_line(self, len * (1.0 - done));
            on_piece(self, end);
        }
    }

    /// Line model of cross-section `xi` in the channel's mode (cached).
    fn line_model_for(&mut self, xi: u32) -> Option<LineModel> {
        let (an, rp, diff, field) = (self.an, self.rp, self.diff, self.field);
        let su = self.board.stackup();
        self.model_cache
            .entry((xi, diff))
            .or_insert_with(|| {
                let xs = &an.cross_sections[xi as usize];
                let fr = xs.geom.and_then(|g| field.get(&an.geometries[g as usize].id));
                let odd = diff && xs.zdiff.is_some();
                line_model(su, xs, fr, odd, rp).or_else(|| line_model(su, xs, fr, false, rp))
            })
            .clone()
    }

    /// Crude high-Z line for sections without a reference plane.
    pub(crate) fn fallback(z_ref: f64) -> LineModel {
        LineModel {
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
        }
    }

    /// Cascade matrix of section `i` at frequency index `k` (frequency `f`); `len_mm`
    /// overrides the length of a line section.
    pub(crate) fn seg_abcd(&self, i: usize, k: usize, f: f64, fallback: &LineModel, len_mm: Option<f64>) -> Abcd {
        let rp = self.rp;
        let w = 2.0 * PI * f;
        let s = &self.segs[i];
        match s.kind {
            // open stub: input admittance C/A of its cascade
            "branch" => match self.branch[i].as_ref().and_then(|b| b.get(k)) {
                Some(b) => Abcd::shunt(b.c / b.a),
                None => Abcd::identity(),
            },
            "via" => self.via_models[i].as_ref().map(|vm| vm.abcd(f, rp.sigma)).unwrap_or_else(Abcd::identity),
            "series" | "shunt" => self.lumped[i].as_ref().map(|p| p.abcd(w)).unwrap_or_else(Abcd::identity),
            _ => {
                let lm = self.models[i].as_ref().unwrap_or(fallback);
                let p = lm.rlgc(f, rp);
                Abcd::line(p.r, p.l, p.g, p.c, w, len_mm.unwrap_or(s.len_mm) * 1e-3)
            }
        }
    }

    /// Cascade matrix of section `i` at 0 Hz (the DC point of time-domain responses): a line is
    /// its DC resistance, a via a short, lumped parts as in `LumpedPart::abcd_dc`, an open stub
    /// its DC input admittance.
    pub(crate) fn seg_abcd_dc(&self, i: usize, fallback: &LineModel, len_mm: Option<f64>) -> Abcd {
        let s = &self.segs[i];
        match s.kind {
            "branch" => match self.branch_dc[i] {
                Some(b) if b.a.abs() > 0.0 => Abcd::shunt(b.c / b.a),
                _ => Abcd::identity(),
            },
            "via" => Abcd::identity(),
            "series" | "shunt" => self.lumped[i].as_ref().map(|p| p.abcd_dc()).unwrap_or_else(Abcd::identity),
            _ => {
                let lm = self.models[i].as_ref().unwrap_or(fallback);
                Abcd::series(Cx::new(lm.r_dc * len_mm.unwrap_or(s.len_mm) * 1e-3, 0.0))
            }
        }
    }

    /// One-way static delay of section `i` (s).
    pub(crate) fn seg_delay(&self, i: usize, fallback: &LineModel) -> f64 {
        match self.segs[i].kind {
            "line" => {
                let lm = self.models[i].as_ref().unwrap_or(fallback);
                self.segs[i].len_mm * 1e-3 * (lm.l_ext * lm.c0).sqrt()
            }
            "via" => self.via_models[i].as_ref().map(|v| v.delay()).unwrap_or(0.0),
            _ => 0.0,
        }
    }

    pub(crate) fn finish(mut self, net: u32, name: String, partner: Option<u32>, target: Option<(f64, bool)>) -> Channel {
        let rp = self.rp;
        let diff = self.diff;
        if self.models.iter().zip(&self.segs).any(|(m, s)| s.kind == "line" && m.is_none() && s.len_mm > 0.0) {
            self.warnings.push("sections without reference plane: approximated as 1.5 × nominal impedance".into());
        }
        // reference impedance
        let z_ref = rp.z_ref.unwrap_or(match target {
            Some((z, true)) if diff => z / 2.0,
            Some((z, false)) => z,
            _ => 50.0,
        });
        // fallback model for sections without reference: crude high-Z line
        let fallback = Self::fallback(z_ref);
        // frequency sweep
        let n = rp.npts.max(10);
        let df = rp.fmax / n as f64;
        let freq: Vec<f64> = (1..=n).map(|k| k as f64 * df).collect();
        let mut s11 = Vec::with_capacity(n);
        let mut s21 = Vec::with_capacity(n);
        let mut s22 = Vec::with_capacity(n);
        let mut abcd = Vec::with_capacity(n);
        for (k, &f) in freq.iter().enumerate() {
            let mut m = Abcd::identity();
            for i in 0..self.segs.len() {
                m = m.mul(&self.seg_abcd(i, k, f, &fallback, None));
            }
            let (a, b, c) = m.s_params(z_ref);
            s11.push(a);
            s21.push(b);
            s22.push(c);
            abcd.push(m);
        }
        let abcd_dc = (0..self.segs.len()).fold(Abcd::identity(), |m, i| m.mul(&self.seg_abcd_dc(i, &fallback, None)));
        // taps (receivers on side stubs): the main path split at them, the stubs on their own
        let tap_at: Vec<usize> = (0..self.segs.len()).filter(|&i| self.branch_tap[i].is_some()).collect();
        let (mut pieces, mut taps) = (Vec::new(), Vec::new());
        if !tap_at.is_empty() {
            let mut a = 0;
            for &t in tap_at.iter().chain(std::iter::once(&self.segs.len())) {
                let ms: Vec<Abcd> = freq.iter().enumerate().map(|(k, &f)| (a..t).fold(Abcd::identity(), |m, i| m.mul(&self.seg_abcd(i, k, f, &fallback, None)))).collect();
                let dc = (a..t).fold(Abcd::identity(), |m, i| m.mul(&self.seg_abcd_dc(i, &fallback, None)));
                pieces.push((ms, dc));
                a = t + 1;
            }
            for (j, &i) in tap_at.iter().enumerate() {
                taps.push(Tap { label: self.branch_tap[i].clone().unwrap_or_default(), after: j, abcd: self.branch[i].clone().unwrap_or_default(), dc: self.branch_dc[i].unwrap_or_else(Abcd::identity) });
            }
        }
        let db = |x: Cx| 20.0 * x.abs().max(1e-12).log10();
        let s21_db: Vec<f64> = s21.iter().map(|&x| db(x)).collect();
        let s11_db: Vec<f64> = s11.iter().map(|&x| db(x)).collect();
        let s22_db: Vec<f64> = s22.iter().map(|&x| db(x)).collect();
        let il_at = |fghz: f64| -> Option<(f64, f64)> {
            if fghz * 1e9 > rp.fmax {
                return None;
            }
            let k = ((fghz * 1e9 / df).round() as usize).clamp(1, n) - 1;
            Some((fghz, s21_db[k]))
        };
        let il_marks: Vec<(f64, f64)> = [1.0, 2.5, 4.0, 5.0, 8.0, 12.5, 16.0, 25.0].iter().filter_map(|&f| il_at(f)).collect();

        // delay (lossless, static) and position along the path per section: (one-way delay, track mm)
        let mut delay = 0.0;
        let mut s_pos = 0.0;
        let mut pos_table: Vec<(f64, f64)> = vec![(0.0, 0.0)];
        for i in 0..self.segs.len() {
            self.segs[i].s_mm = s_pos;
            delay += self.seg_delay(i, &fallback);
            match self.segs[i].kind {
                "line" => {
                    s_pos += self.segs[i].len_mm;
                    pos_table.push((delay, s_pos));
                }
                "via" if self.via_models[i].is_some() => pos_table.push((delay, s_pos)),
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
        // round trip t → one-way delay t/2 → track position (piecewise from the sections)
        let v_mean = if delay > 0.0 { s_pos / delay } else { 0.0 };
        let xmap = |t: f64| -> f64 {
            let h = t / 2.0;
            match pos_table.iter().position(|&(d, _)| d >= h) {
                Some(0) => 0.0,
                Some(k) => {
                    let ((d0, s0), (d1, s1)) = (pos_table[k - 1], pos_table[k]);
                    if d1 > d0 { s0 + (s1 - s0) * (h - d0) / (d1 - d0) } else { s1 }
                }
                None => s_pos + (h - delay) * v_mean,
            }
        };
        let (tdr_t_ps, tdr_z, tdr_x_mm) = tdr_map(&freq, &s11, z_ref, rp.rise, (2.0 * delay * 1.3 + 4.0 * rp.rise).max(200e-12), &xmap);
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
            s22_db,
            il_marks,
            tdr_t_ps,
            tdr_z,
            tdr_x_mm,
            tdr_end_mm: s_pos,
            loss_db_per_in_at,
            via_info: self.via_info,
            field_share: if total_len > 0.0 { self.field_len / total_len } else { 0.0 },
            warnings: self.warnings,
            path: None,
            s11,
            s21,
            s22,
            abcd,
            abcd_dc,
            taps,
            pieces,
        }
    }
}

pub fn channel(board: &Board, an: &Analysis, net: u32, field: &HashMap<String, FieldResult>, target: Option<(f64, bool)>, rp: &RfParams) -> Result<Channel, String> {
    channel_dir(board, an, net, field, target, rp, false, None)
}

/// Channel of a net along its main path (start chosen as in `net_path_info`, `flip`
/// reverses it); branches of at least `BRANCH_MIN_MM` are included as open stubs.
#[allow(clippy::too_many_arguments)]
pub fn channel_dir(board: &Board, an: &Analysis, net: u32, field: &HashMap<String, FieldResult>, target: Option<(f64, bool)>, rp: &RfParams, flip: bool, hint: Option<P>) -> Result<Channel, String> {
    let partner = crate::extract::diff_partners(&board.nets).get(net as usize).copied().flatten();
    let diff = partner.is_some() && target.map(|t| t.1).unwrap_or(true);
    let info = net_path_info(board, an, net, flip, hint).ok_or("net has no tracks")?;
    let (parts, notes) = crate::topology::net_parts(board, net, if diff { crate::topology::PartMode::Odd } else { crate::topology::PartMode::Single });
    let mut b = net_build(board, an, net, field, target, rp, &info, diff, false, &parts);
    b.warnings.extend(notes);
    let mut ch = b.finish(net, board.net_name(net).to_string(), partner, target);
    ch.path = Some(info);
    Ok(ch)
}

/// Sections of a net along `info` (main path with its modelled branches as open stubs), with
/// `parts` (shunt parts of the net) at their pads: on the main path or on a modelled branch,
/// wherever the pad lies closest.
#[allow(clippy::too_many_arguments)]
pub(crate) fn net_build<'a>(board: &'a Board, an: &'a Analysis, net: u32, field: &'a HashMap<String, FieldResult>, target: Option<(f64, bool)>, rp: &'a RfParams, info: &NetPath, diff: bool, pair: bool, parts: &[NetPart]) -> Build<'a> {
    let mut warnings = Vec::new();
    if info.parts > 1 {
        warnings.push(format!("net splits into {} parts (e.g. connected through components): the part with the longest path is computed", info.parts));
    }
    let path: Vec<(usize, bool)> = info.pieces.iter().map(|p| (p.piece, p.rev)).collect();
    let modelled: Vec<&PathBranch> = info.branches.iter().filter(|b| b.modelled).collect();
    let bpaths: Vec<Vec<(usize, bool)>> = modelled.iter().map(|br| br.pieces.iter().map(|p| (p.piece, p.rev)).collect()).collect();
    // parts: (0 = main path, k + 1 = branch k) → cuts
    let mut cuts: Vec<Vec<Cut>> = vec![vec![]; bpaths.len() + 1];
    let (mut placed, mut end_loads): (Vec<&str>, Vec<&str>) = (Vec::new(), Vec::new());
    for np in parts {
        // a part whose pad is an end of the computed path loads that port (an AC termination,
        // the grounded leg of a balun): like a termination resistor it is not in the channel
        if [&info.start, &info.end].iter().any(|e| e.refdes.as_deref() == Some(np.refdes.as_str())) {
            end_loads.push(&np.note);
            continue;
        }
        let mut best: Option<(usize, usize, f64, f64)> = None;
        for (k, p) in std::iter::once(&path).chain(bpaths.iter()).enumerate() {
            if let Some((step, frac, d)) = locate_on_path(board, an, p, np.at, np.layers) {
                if best.is_none_or(|b| d < b.3) {
                    best = Some((k, step, frac, d));
                }
            }
        }
        match best {
            Some((k, step, frac, d)) => {
                if d > 1.0 {
                    warnings.push(format!("{}: pad {:.1} mm from the computed path (another part of the net or another layer), placed at the nearest point", np.refdes, d.min(99.9)));
                }
                cuts[k].push(Cut { step, frac, part: np.part.clone() });
                placed.push(&np.note);
            }
            None => warnings.push(format!("{}: no copper of the path to attach it to, not modelled", np.refdes)),
        }
    }
    if !placed.is_empty() {
        warnings.push(format!("shunt part(s) modelled at their pads: {} – package parasitics are estimates (data/components.json)", placed.join(", ")));
    }
    if !end_loads.is_empty() {
        warnings.push(format!("{} at an end of the computed path: a load of that port, not in the channel (loss, return loss and TDR describe the interconnect)", end_loads.join(", ")));
    }
    // branch channels (from the junction), inserted after the piece before the junction
    let mut stubs: Vec<(usize, String, Channel)> = Vec::new();
    for (k, br) in modelled.iter().enumerate() {
        let mut bb = Build::new(board, an, field, rp, diff);
        bb.add_path_cuts(net, &bpaths[k], None, &cuts[k + 1], |_, _| {}, |_, _| {});
        let ch = bb.finish(net, "branch".into(), None, target);
        stubs.push((br.after_piece, format!("branch {:.1} mm to {}", br.len_mm, br.end.label), ch));
    }
    let n_short = info.branches.iter().filter(|b| !b.modelled).count();
    if !stubs.is_empty() {
        warnings.push(format!("{} branch(es) included as open stubs", stubs.len()));
    }
    if n_short > 0 {
        warnings.push(format!("{n_short} branch(es) shorter than {BRANCH_MIN_MM} mm only marked, not modelled"));
    }
    if info.branches.iter().any(|b| b.sub_branches) {
        warnings.push("a branch branches again: only its longest way is included".into());
    }
    let mut b = Build::new(board, an, field, rp, diff);
    b.pair = pair;
    b.warnings = warnings;
    let mut k = 0usize;
    b.add_path_cuts(net, &path, None, &cuts[0], |_, _| {}, |bld, _| {
        k += 1;
        for (_, label, ch) in stubs.iter().filter(|s| s.0 == k) {
            bld.add_branch(label.clone(), ch);
        }
    });
    b
}

/// Step response of S11 on a uniform grid f_k = k·df (k ≥ 1), with a Gaussian
/// filter whose 10-90 % rise time is `rise`. Returns (t ps, Z Ω, x mm).
pub fn tdr(freq: &[f64], s11: &[Cx], z_ref: f64, rise: f64, t_max: f64, delay: f64, length_mm: f64) -> (Vec<f64>, Vec<f64>, Vec<f64>) {
    tdr_map(freq, s11, z_ref, rise, t_max, &|t: f64| if delay > 0.0 { t / (2.0 * delay) * length_mm } else { 0.0 })
}

/// As `tdr`, with the position of round-trip time t given by `xmap` (mm).
pub fn tdr_map(freq: &[f64], s11: &[Cx], z_ref: f64, rise: f64, t_max: f64, xmap: &dyn Fn(f64) -> f64) -> (Vec<f64>, Vec<f64>, Vec<f64>) {
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
        xs.push(xmap(t));
    }
    (ts, zs, xs)
}

/// Touchstone (.s2p, RI) of a channel.
pub fn touchstone(ch: &Channel) -> String {
    let mut s = String::new();
    let (a, b) = match &ch.path {
        Some(p) => (format!("{} at {} (start of the path)", ch.name, p.start.label), format!("{} at {} (end of the path)", ch.name, p.end.label)),
        None => (format!("{} at the start of the channel", ch.name), format!("{} at the end of the channel", ch.name)),
    };
    s.push_str(&format!("! kisi {}: S-parameters of {}\n", env!("CARGO_PKG_VERSION"), ch.name));
    if ch.diff {
        s.push_str(&format!("! 2-port, differential mode (odd-mode model per line), reference {:.1} ohm per line = {:.1} ohm differential\n", ch.z_ref, 2.0 * ch.z_ref));
    } else {
        s.push_str(&format!("! 2-port, single-ended, reference {:.1} ohm at both ports\n", ch.z_ref));
    }
    s.push_str(&format!("! Port 1 = A: {a}\n! Port 2 = B: {b}\n"));
    s.push_str("! Data per frequency: S11 S21 S12 S22, each as real and imaginary part; S12 = S21 (passive, reciprocal)\n");
    s.push_str(&format!("! Path {:.2} mm incl. {} vias\n", ch.length_mm, ch.vias));
    s.push_str(&format!("# HZ S RI R {:.3}\n", ch.z_ref));
    for (k, &f) in ch.freq.iter().enumerate() {
        // S11 S21 S12 S22; S12 = S21 (passive, reciprocal)
        let (a, b, c) = (ch.s11[k], ch.s21[k], ch.s22.get(k).copied().unwrap_or(ch.s11[k]));
        s.push_str(&format!("{:.6e} {:.6e} {:.6e} {:.6e} {:.6e} {:.6e} {:.6e} {:.6e} {:.6e}\n", f, a.re, a.im, b.re, b.im, b.re, b.im, c.re, c.im));
    }
    s
}


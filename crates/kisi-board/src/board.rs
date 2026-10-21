//! Board model built from a `.kicad_pcb` file (KiCad 8/9/10 S-expression format)
//! and, optionally, the matching `.kicad_pro` (net classes).

use crate::geom::{self, BBox, P};
use crate::sexpr::{Node, ParseError, Reader};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize)]
pub struct CopperLayer {
    pub name: String,
    /// copper thickness in mm
    pub thickness: f64,
}

/// Dielectric between copper layer `i` and `i+1` (possibly several sublayers).
#[derive(Debug, Clone, Serialize)]
pub struct Dielectric {
    pub name: String,
    pub kind: String,
    pub material: String,
    pub thickness: f64,
    /// Series-equivalent permittivity of all sublayers (vertical field).
    pub epsilon_r: f64,
    pub loss_tangent: f64,
    /// properties not given in the file (default values used): "material", "epsilon_r", "loss_tangent"
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub assumed: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SolderMask {
    pub thickness: f64,
    pub epsilon_r: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Stackup {
    pub copper: Vec<CopperLayer>,
    /// `dielectrics[i]` lies between `copper[i]` and `copper[i+1]`.
    pub dielectrics: Vec<Dielectric>,
    pub mask_top: Option<SolderMask>,
    pub mask_bottom: Option<SolderMask>,
    /// true if the stackup came from the file, false if defaults were used
    pub from_file: bool,
}

impl Stackup {
    pub fn layer_index(&self, name: &str) -> Option<usize> {
        self.copper.iter().position(|l| l.name == name)
    }
    pub fn n(&self) -> usize {
        self.copper.len()
    }
    /// Total thickness of material strictly between copper layers i and j (i<j):
    /// all dielectrics plus intermediate copper layers.
    pub fn gap_between(&self, i: usize, j: usize) -> f64 {
        let (i, j) = if i < j { (i, j) } else { (j, i) };
        let mut h = 0.0;
        for k in i..j {
            h += self.dielectrics[k].thickness;
            if k > i {
                h += self.copper[k].thickness;
            }
        }
        h
    }
    /// Series-equivalent permittivity of material between copper layers i and j.
    pub fn er_between(&self, i: usize, j: usize) -> f64 {
        let (i, j) = if i < j { (i, j) } else { (j, i) };
        let mut h = 0.0;
        let mut s = 0.0;
        for k in i..j {
            let d = &self.dielectrics[k];
            h += d.thickness;
            s += d.thickness / d.epsilon_r;
            if k > i {
                // intermediate copper layer: assume it is embedded in dielectric
                // of the neighbours (resin fill); use adjacent permittivity.
                let t = self.copper[k].thickness;
                h += t;
                s += t / d.epsilon_r;
            }
        }
        if s > 0.0 { h / s } else { 1.0 }
    }
}

/// User changes to the stackup read from the file (viewer: "Lagenaufbau
/// bearbeiten"). Values that are `None` keep the file value.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct StackupOverride {
    pub dielectrics: Vec<DielectricOverride>,
    pub mask_epsilon_r: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DielectricOverride {
    /// index into `Stackup::dielectrics`
    pub index: usize,
    pub epsilon_r: Option<f64>,
    pub loss_tangent: Option<f64>,
    /// mm
    pub thickness: Option<f64>,
}

impl Stackup {
    /// Copy of the stackup with the overrides applied (invalid values ignored).
    pub fn with_overrides(&self, ov: &StackupOverride) -> Stackup {
        let mut s = self.clone();
        for o in &ov.dielectrics {
            let Some(d) = s.dielectrics.get_mut(o.index) else { continue };
            if let Some(v) = o.epsilon_r.filter(|v| *v >= 1.0 && *v < 20.0) {
                d.epsilon_r = v;
            }
            if let Some(v) = o.loss_tangent.filter(|v| *v >= 0.0 && *v < 0.2) {
                d.loss_tangent = v;
            }
            if let Some(v) = o.thickness.filter(|v| *v > 0.0 && *v < 10.0) {
                d.thickness = v;
            }
        }
        if let Some(v) = ov.mask_epsilon_r.filter(|v| *v >= 1.0 && *v < 20.0) {
            for m in [&mut s.mask_top, &mut s.mask_bottom].into_iter().flatten() {
                m.epsilon_r = v;
            }
        }
        s
    }
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct Track {
    pub a: P,
    pub b: P,
    pub width: f64,
    pub layer: u16,
    pub net: u32,
    /// index of the originating KiCad item (segment or arc)
    pub item: u32,
    pub is_arc: bool,
    /// synthetic track through an SMD pad of a signal net (see `add_pad_tracks`)
    pub pad: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Via {
    pub at: P,
    pub size: f64,
    pub drill: f64,
    pub from: u16,
    pub to: u16,
    pub net: u32,
    /// back-drilled copper layer range (KiCad 10 `backdrill`), inclusive
    pub backdrill: Option<(u16, u16)>,
    /// `remove_unused_layers`: None = annular ring on every layer (KiCad default),
    /// Some(keep_end_layers) = only on connected layers (and the two end layers if true)
    pub remove_unused: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Pad {
    pub at: P,
    pub size: P,
    /// absolute orientation in degrees (KiCad convention)
    pub angle: f64,
    pub shape: String,
    /// copper layer indices this pad exists on
    pub layers: Vec<u16>,
    pub net: u32,
    pub drill: f64,
    /// pad number / name, e.g. "1", "A3"
    pub number: String,
    /// index into `Board::footprints`
    pub fp: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct Footprint {
    pub reference: String,
    pub value: String,
    pub at: P,
    /// user fields (symbol fields copied to the footprint), e.g. PI_LOAD
    pub props: Vec<(String, String)>,
    /// marked "Do not populate" (KiCad 8+: `(attr … dnp)`)
    pub dnp: bool,
}

impl Footprint {
    pub fn prop(&self, name: &str) -> Option<&str> {
        self.props.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

#[derive(Debug, Clone)]
pub struct ZoneFill {
    pub layer: u16,
    pub net: u32,
    pub pts: Vec<P>,
    pub bbox: BBox,
}

#[derive(Debug, Clone, Serialize)]
pub struct NetClass {
    pub name: String,
    pub track_width: Option<f64>,
    pub diff_pair_width: Option<f64>,
    pub diff_pair_gap: Option<f64>,
}

#[derive(Debug, Clone, Default)]
pub struct Board {
    pub version: String,
    pub generator: String,
    pub stackup: Option<Stackup>,
    pub nets: Vec<String>,
    pub tracks: Vec<Track>,
    pub vias: Vec<Via>,
    pub pads: Vec<Pad>,
    pub footprints: Vec<Footprint>,
    pub zones: Vec<ZoneFill>,
    pub outline: Vec<Vec<P>>,
    pub bbox: Option<BBox>,
    pub netclasses: Vec<NetClass>,
    /// net id -> netclass index (0 = Default)
    pub net_class: Vec<u32>,
    /// nets that own copper zones (ground / power): their copper acts as
    /// reference or coplanar ground
    pub plane_net: Vec<bool>,
    pub warnings: Vec<String>,
}

impl Board {
    pub fn stackup(&self) -> &Stackup {
        self.stackup.as_ref().expect("stackup always set after load")
    }
    pub fn net_name(&self, id: u32) -> &str {
        self.nets.get(id as usize).map(|s| s.as_str()).unwrap_or("")
    }
    pub fn class_of(&self, net: u32) -> &NetClass {
        let idx = self.net_class.get(net as usize).copied().unwrap_or(0) as usize;
        &self.netclasses[idx.min(self.netclasses.len() - 1)]
    }
}

const WANTED: &[&str] = &[
    "version", "generator", "layers", "setup", "net", "segment", "arc", "via", "zone",
    "footprint", "gr_line", "gr_arc", "gr_rect", "gr_circle", "gr_poly",
];

struct NetTable {
    names: Vec<String>,
    by_name: HashMap<String, u32>,
}

impl NetTable {
    fn new() -> Self {
        NetTable { names: vec![String::new()], by_name: HashMap::from([(String::new(), 0)]) }
    }
    fn declare(&mut self, id: u32, name: &str) {
        let id = id as usize;
        if self.names.len() <= id {
            self.names.resize(id + 1, String::new());
        }
        self.names[id] = name.to_string();
        self.by_name.insert(name.to_string(), id as u32);
    }
    fn by_name(&mut self, name: &str) -> u32 {
        if let Some(&id) = self.by_name.get(name) {
            return id;
        }
        let id = self.names.len() as u32;
        self.names.push(name.to_string());
        self.by_name.insert(name.to_string(), id);
        id
    }
    /// `(net 3)`, `(net 3 "GND")` or `(net "GND")` (newer formats)
    fn resolve(&mut self, n: Option<&Node>) -> u32 {
        let Some(n) = n else { return 0 };
        match n.items().get(1) {
            Some(Node::Sym(s)) => {
                let id: u32 = s.parse().unwrap_or(0);
                if let Some(name) = n.atom(2) {
                    if (id as usize) >= self.names.len() || self.names[id as usize].is_empty() {
                        self.declare(id, name);
                    }
                }
                id
            }
            Some(Node::Str(s)) => {
                let s = s.clone();
                self.by_name(&s)
            }
            _ => 0,
        }
    }
}

pub fn load_pcb(text: &str) -> Result<Board, ParseError> {
    let mut r = Reader::new(text);
    r.expect(b'(')?;
    let head = r.peek_head();
    if head != "kicad_pcb" {
        return Err(ParseError(format!("not a KiCad board file (head '{}')", head)));
    }
    r.skip_ws();
    r.read_symbol();

    let mut items: Vec<Node> = Vec::new();
    loop {
        match r.peek() {
            Some(b'(') => {
                r.i += 1;
                let h = r.peek_head();
                if WANTED.contains(&h) {
                    items.push(r.parse_rest()?);
                } else {
                    r.skip_rest()?;
                }
            }
            Some(b')') | None => break,
            Some(_) => {
                return Err(ParseError(format!("unexpected token at byte {}", r.i)));
            }
        }
    }

    let mut b = Board::default();
    let mut nets = NetTable::new();

    // --- header / stackup ---------------------------------------------------
    let mut layer_names_from_header: Vec<String> = Vec::new();
    for it in &items {
        match it.head() {
            Some("version") => b.version = it.atom(1).unwrap_or("").to_string(),
            Some("generator") => b.generator = it.atom(1).unwrap_or("").to_string(),
            Some("layers") => {
                for l in it.items().iter().skip(1) {
                    if let Some(name) = l.atom(1) {
                        if name.ends_with(".Cu") {
                            layer_names_from_header.push(name.to_string());
                        }
                    }
                }
            }
            Some("setup") => {
                if let Some(st) = it.child("stackup") {
                    b.stackup = parse_stackup(st);
                }
            }
            Some("net") => {
                if let (Some(id), Some(name)) = (it.f64(1), it.atom(2)) {
                    nets.declare(id as u32, name);
                }
            }
            _ => {}
        }
    }

    if b.stackup.is_none() {
        b.stackup = Some(default_stackup(&layer_names_from_header));
        b.warnings.push(
            "no stackup in the file – default values used (1.6 mm FR4, εr 4.5)".into(),
        );
    }
    let su = b.stackup.clone().unwrap();
    let lidx = |name: &str| su.layer_index(name).map(|i| i as u16);

    // --- copper items --------------------------------------------------------
    let mut bbox = BBox::empty();
    for (item_no, it) in items.iter().enumerate() {
        match it.head() {
            Some("segment") => {
                let (Some(a), Some(e), Some(w)) =
                    (it.child_xy("start"), it.child_xy("end"), it.child_f64("width"))
                else {
                    continue;
                };
                let Some(layer) = it.child_str("layer").and_then(lidx) else { continue };
                let net = nets.resolve(it.child("net"));
                bbox.add(a);
                bbox.add(e);
                b.tracks.push(Track { a, b: e, width: w, layer, net, item: item_no as u32, is_arc: false, pad: false });
            }
            Some("arc") => {
                let (Some(a), Some(m), Some(e), Some(w)) = (
                    it.child_xy("start"),
                    it.child_xy("mid"),
                    it.child_xy("end"),
                    it.child_f64("width"),
                ) else {
                    continue;
                };
                let Some(layer) = it.child_str("layer").and_then(lidx) else { continue };
                let net = nets.resolve(it.child("net"));
                let pts = geom::tessellate_arc(a, m, e, 5f64.to_radians());
                for w2 in pts.windows(2) {
                    bbox.add(w2[0]);
                    bbox.add(w2[1]);
                    b.tracks.push(Track {
                        a: w2[0],
                        b: w2[1],
                        width: w,
                        layer,
                        net,
                        item: item_no as u32,
                        is_arc: true,
                        pad: false,
                    });
                }
            }
            Some("via") => {
                let (Some(at), Some(size)) = (it.child_xy("at"), it.child_f64("size")) else {
                    continue;
                };
                let drill = it.child_f64("drill").unwrap_or(size * 0.5);
                let (mut from, mut to) = (0u16, (su.n() - 1) as u16);
                if let Some(ls) = it.child("layers") {
                    let a = ls.atom(1).and_then(lidx);
                    let c = ls.atom(2).and_then(lidx);
                    if let (Some(a), Some(c)) = (a, c) {
                        from = a.min(c);
                        to = a.max(c);
                    }
                }
                let net = nets.resolve(it.child("net"));
                // KiCad 10: (backdrill (size ..) (layers "B.Cu" "In3.Cu"))
                let backdrill = it.child("backdrill").and_then(|bd| {
                    let ls = bd.child("layers")?;
                    let a = ls.atom(1).and_then(lidx)?;
                    let c = ls.atom(2).and_then(lidx).unwrap_or(a);
                    Some((a.min(c), a.max(c)))
                });
                // `(remove_unused_layers)` (KiCad 7 flag) or `(remove_unused_layers yes|no)`
                let flag = |name: &str| it.child(name).map(|n| n.atom(1) != Some("no"));
                let remove_unused = if flag("remove_unused_layers") == Some(true) { Some(flag("keep_end_layers").unwrap_or(false)) } else { None };
                b.vias.push(Via { at, size, drill, from, to, net, backdrill, remove_unused });
            }
            Some("zone") => {
                let net = nets.resolve(it.child("net"));
                for fp in it.children("filled_polygon") {
                    let Some(layer) = fp.child_str("layer").and_then(lidx) else { continue };
                    // `(pts ...)` is parsed into a Node::Pts item of the filled_polygon
                    let Some(pts) = fp.pts().map(|p| p.to_vec()) else { continue };
                    if pts.len() < 3 {
                        continue;
                    }
                    let bb = BBox::of(&pts);
                    b.zones.push(ZoneFill { layer, net, pts, bbox: bb });
                }
            }
            Some("footprint") => parse_footprint(it, &su, &mut nets, &mut b.pads, &mut b.footprints),
            Some(h @ ("gr_line" | "gr_arc" | "gr_rect" | "gr_circle" | "gr_poly")) => {
                if it.child_str("layer") != Some("Edge.Cuts") {
                    continue;
                }
                let poly: Vec<P> = match h {
                    "gr_line" => match (it.child_xy("start"), it.child_xy("end")) {
                        (Some(a), Some(e)) => vec![a, e],
                        _ => continue,
                    },
                    "gr_arc" => match (it.child_xy("start"), it.child_xy("mid"), it.child_xy("end")) {
                        (Some(a), Some(m), Some(e)) => geom::tessellate_arc(a, m, e, 5f64.to_radians()),
                        _ => continue,
                    },
                    "gr_rect" => match (it.child_xy("start"), it.child_xy("end")) {
                        (Some(a), Some(e)) => vec![a, [e[0], a[1]], e, [a[0], e[1]], a],
                        _ => continue,
                    },
                    "gr_circle" => match (it.child_xy("center"), it.child_xy("end")) {
                        (Some(c), Some(e)) => {
                            let r = geom::len(geom::sub(e, c));
                            (0..=72)
                                .map(|i| {
                                    let t = i as f64 / 72.0 * std::f64::consts::TAU;
                                    [c[0] + r * t.cos(), c[1] + r * t.sin()]
                                })
                                .collect()
                        }
                        _ => continue,
                    },
                    _ => match it.pts() {
                        Some(p) => {
                            let mut v = p.to_vec();
                            if let Some(&f) = v.first() {
                                v.push(f);
                            }
                            v
                        }
                        None => match it.child("pts").and_then(|n| n.pts()) {
                            Some(p) => p.to_vec(),
                            None => continue,
                        },
                    },
                };
                for &p in &poly {
                    bbox.add(p);
                }
                b.outline.push(poly);
            }
            _ => {}
        }
    }
    for z in &b.zones {
        bbox.add(z.bbox.min);
        bbox.add(z.bbox.max);
    }
    b.bbox = if bbox.is_valid() { Some(bbox) } else { None };
    b.nets = nets.names;
    b.netclasses = vec![NetClass {
        name: "Default".into(),
        track_width: None,
        diff_pair_width: None,
        diff_pair_gap: None,
    }];
    b.net_class = vec![0; b.nets.len()];
    b.plane_net = vec![false; b.nets.len()];
    for z in &b.zones {
        if (z.net as usize) < b.plane_net.len() && z.net != 0 {
            b.plane_net[z.net as usize] = true;
        }
    }
    add_pad_tracks(&mut b);
    Ok(b)
}

/// Outline of a pad as a polygon (rounded shapes approximated).
pub fn pad_polygon(p: &Pad) -> Vec<P> {
    let (hx, hy) = (p.size[0] / 2.0, p.size[1] / 2.0);
    let mut local: Vec<P> = match p.shape.as_str() {
        "circle" => (0..16)
            .map(|i| {
                let t = i as f64 / 16.0 * std::f64::consts::TAU;
                [hx * t.cos(), hx * t.sin()]
            })
            .collect(),
        "oval" => {
            // stadium: two half circles on the long axis
            let (r, long_x) = if hx >= hy { (hy, true) } else { (hx, false) };
            let c = if long_x { hx - r } else { hy - r };
            let mut v = Vec::new();
            for i in 0..=8 {
                let t = -std::f64::consts::FRAC_PI_2 + i as f64 / 8.0 * std::f64::consts::PI;
                v.push(if long_x { [c + r * t.cos(), r * t.sin()] } else { [r * t.sin(), c + r * t.cos()] });
            }
            for i in 0..=8 {
                let t = std::f64::consts::FRAC_PI_2 + i as f64 / 8.0 * std::f64::consts::PI;
                v.push(if long_x { [-c + r * t.cos(), r * t.sin()] } else { [r * t.sin(), -c + r * t.cos()] });
            }
            v
        }
        _ => vec![[-hx, -hy], [hx, -hy], [hx, hy], [-hx, hy]],
    };
    // same orientation convention as the viewer: rotate by -angle (y down)
    let th = -p.angle.to_radians();
    let (s, c) = th.sin_cos();
    for q in local.iter_mut() {
        *q = [p.at[0] + q[0] * c - q[1] * s, p.at[1] + q[0] * s + q[1] * c];
    }
    local
}

/// Pad axes in board coordinates: (u along size.x, v along size.y).
fn pad_axes(p: &Pad) -> (P, P) {
    let th = -p.angle.to_radians();
    let (s, c) = th.sin_cos();
    ([c, s], [-s, c])
}

/// Adds a synthetic track through every SMD pad of a signal net that a track
/// connects to, oriented along the connecting track. The pad is then sampled
/// like a trace: its impedance (wide copper, antipads below it) shows up in
/// the map and in the net profile.
fn add_pad_tracks(b: &mut Board) {
    // track endpoints by (net, layer, cell)
    let cell = 1.0;
    let mut ends: HashMap<(u32, u16, i64, i64), Vec<(P, P)>> = HashMap::new();
    for t in &b.tracks {
        let d = geom::sub(t.b, t.a);
        let l = geom::len(d);
        if l < 1e-6 {
            continue;
        }
        let dir = geom::scale(d, 1.0 / l);
        for e in [t.a, t.b] {
            let k = (t.net, t.layer, (e[0] / cell).floor() as i64, (e[1] / cell).floor() as i64);
            ends.entry(k).or_default().push((e, dir));
        }
    }
    let n_layers = b.stackup().n() as u16;
    let mut new_tracks = Vec::new();
    for (pi, p) in b.pads.iter().enumerate() {
        if p.drill > 0.0 || p.net == 0 || p.layers.len() != 1 {
            continue;
        }
        if b.plane_net.get(p.net as usize).copied().unwrap_or(false) {
            continue;
        }
        let layer = p.layers[0];
        if layer >= n_layers {
            continue;
        }
        let r = p.size[0].max(p.size[1]) / 2.0;
        // a connecting track ends inside the pad
        let mut dir: Option<P> = None;
        let (cx, cy) = ((p.at[0] / cell).floor() as i64, (p.at[1] / cell).floor() as i64);
        'search: for dx in -1..=1 {
            for dy in -1..=1 {
                if let Some(v) = ends.get(&(p.net, layer, cx + dx, cy + dy)) {
                    for &(e, d) in v {
                        if geom::len(geom::sub(e, p.at)) <= r {
                            dir = Some(d);
                            break 'search;
                        }
                    }
                }
            }
        }
        let Some(dir) = dir else { continue };
        let (u, v) = pad_axes(p);
        let (axis, along, across) = if geom::dot(dir, u).abs() >= geom::dot(dir, v).abs() {
            (u, p.size[0], p.size[1])
        } else {
            (v, p.size[1], p.size[0])
        };
        // drawn with round caps of radius width/2: shorten so the drawing
        // matches the pad outline
        let half = ((along - across).max(0.3 * along)) / 2.0;
        new_tracks.push(Track {
            a: geom::sub(p.at, geom::scale(axis, half)),
            b: geom::add(p.at, geom::scale(axis, half)),
            width: across,
            layer,
            net: p.net,
            item: pi as u32,
            is_arc: false,
            pad: true,
        });
    }
    b.tracks.extend(new_tracks);
}

fn parse_stackup(st: &Node) -> Option<Stackup> {
    let mut copper = Vec::new();
    let mut diel: Vec<Dielectric> = Vec::new();
    let mut pending: Vec<Dielectric> = Vec::new();
    let mut mask_top = None;
    let mut mask_bottom = None;
    for l in st.children("layer") {
        let name = l.atom(1).unwrap_or("").to_string();
        let ty = l.child_str("type").unwrap_or("").to_string();
        if name.ends_with(".Cu") {
            if !copper.is_empty() {
                diel.push(merge_dielectrics(&pending));
            }
            pending.clear();
            copper.push(CopperLayer {
                thickness: l.child_f64("thickness").unwrap_or(0.035),
                name,
            });
        } else if name.starts_with("dielectric") || ty == "core" || ty == "prepreg" {
            // sublayers: a sequence of (thickness)(material)(epsilon_r)(loss_tangent)
            // groups separated by (addsublayer)
            let all = || vec!["material".to_string(), "epsilon_r".to_string(), "loss_tangent".to_string()];
            let mut cur = Dielectric {
                name: name.clone(),
                kind: ty.clone(),
                material: String::new(),
                thickness: 0.0,
                epsilon_r: 4.5,
                loss_tangent: 0.02,
                assumed: all(),
            };
            let mut have = false;
            for c in l.items().iter().skip(2) {
                match c.head() {
                    Some("thickness") => {
                        if have {
                            pending.push(cur.clone());
                            cur.assumed = all();
                        }
                        cur.thickness = c.f64(1).unwrap_or(0.0);
                        have = true;
                    }
                    Some("material") => {
                        cur.material = c.atom(1).unwrap_or("").to_string();
                        cur.assumed.retain(|a| a != "material" || cur.material.is_empty());
                    }
                    Some("epsilon_r") => {
                        cur.epsilon_r = c.f64(1).unwrap_or(4.5);
                        cur.assumed.retain(|a| a != "epsilon_r");
                    }
                    Some("loss_tangent") => {
                        cur.loss_tangent = c.f64(1).unwrap_or(0.02);
                        cur.assumed.retain(|a| a != "loss_tangent");
                    }
                    _ => {}
                }
            }
            if have {
                pending.push(cur);
            }
        } else if name == "F.Mask" || name == "B.Mask" {
            let m = SolderMask {
                thickness: l.child_f64("thickness").unwrap_or(0.01),
                epsilon_r: l.child_f64("epsilon_r").unwrap_or(3.3),
            };
            if name == "F.Mask" {
                mask_top = Some(m);
            } else {
                mask_bottom = Some(m);
            }
        }
    }
    if copper.len() < 2 || diel.len() != copper.len() - 1 {
        return None;
    }
    Some(Stackup { copper, dielectrics: diel, mask_top, mask_bottom, from_file: true })
}

fn merge_dielectrics(parts: &[Dielectric]) -> Dielectric {
    if parts.is_empty() {
        return Dielectric {
            name: "?".into(),
            kind: String::new(),
            material: "?".into(),
            thickness: 0.2,
            epsilon_r: 4.5,
            loss_tangent: 0.02,
            assumed: vec!["material".into(), "epsilon_r".into(), "loss_tangent".into(), "thickness".into()],
        };
    }
    let h: f64 = parts.iter().map(|p| p.thickness).sum();
    let s: f64 = parts.iter().map(|p| p.thickness / p.epsilon_r).sum();
    let tand: f64 = parts.iter().map(|p| p.thickness * p.loss_tangent).sum::<f64>() / h.max(1e-12);
    let mut mats: Vec<String> = parts.iter().map(|p| p.material.clone()).collect();
    mats.dedup();
    let mut assumed: Vec<String> = parts.iter().flat_map(|p| p.assumed.clone()).collect();
    assumed.sort();
    assumed.dedup();
    Dielectric {
        name: parts[0].name.clone(),
        kind: parts[0].kind.clone(),
        material: mats.join(" + "),
        thickness: h,
        epsilon_r: if s > 0.0 { h / s } else { 4.5 },
        loss_tangent: tand,
        assumed,
    }
}

fn default_stackup(names: &[String]) -> Stackup {
    let mut cu: Vec<String> = names.to_vec();
    if cu.len() < 2 {
        cu = vec!["F.Cu".into(), "B.Cu".into()];
    }
    // physical order: F.Cu, In1..InN, B.Cu
    let key = |n: &String| -> i32 {
        if n == "F.Cu" {
            0
        } else if n == "B.Cu" {
            1000
        } else {
            n.trim_start_matches("In").trim_end_matches(".Cu").parse::<i32>().unwrap_or(500)
        }
    };
    cu.sort_by_key(key);
    let n = cu.len();
    let h = (1.6 - 0.035 * n as f64) / (n - 1) as f64;
    Stackup {
        copper: cu.into_iter().map(|name| CopperLayer { name, thickness: 0.035 }).collect(),
        dielectrics: (0..n - 1)
            .map(|i| Dielectric {
                name: format!("dielectric {}", i + 1),
                kind: "default".into(),
                material: "FR4 (assumed)".into(),
                thickness: h,
                epsilon_r: 4.5,
                loss_tangent: 0.02,
                assumed: vec!["material".into(), "epsilon_r".into(), "loss_tangent".into(), "thickness".into()],
            })
            .collect(),
        mask_top: Some(SolderMask { thickness: 0.01, epsilon_r: 3.3 }),
        mask_bottom: Some(SolderMask { thickness: 0.01, epsilon_r: 3.3 }),
        from_file: false,
    }
}

fn parse_footprint(fp: &Node, su: &Stackup, nets: &mut NetTable, out: &mut Vec<Pad>, fps: &mut Vec<Footprint>) {
    let Some(at) = fp.child("at") else { return };
    let o = [at.f64(1).unwrap_or(0.0), at.f64(2).unwrap_or(0.0)];
    // KiCad 8+: (property "Reference" "U3" ...); older: (fp_text reference "U3" ...)
    let mut reference = String::new();
    let mut value = String::new();
    let mut props = Vec::new();
    for pr in fp.children("property") {
        let (Some(k), Some(v)) = (pr.atom(1), pr.atom(2)) else { continue };
        match k {
            "Reference" => reference = v.to_string(),
            "Value" => value = v.to_string(),
            _ => props.push((k.to_string(), v.to_string())),
        }
    }
    for t in fp.children("fp_text") {
        match t.atom(1) {
            Some("reference") if reference.is_empty() => reference = t.atom(2).unwrap_or("").to_string(),
            Some("value") if value.is_empty() => value = t.atom(2).unwrap_or("").to_string(),
            _ => {}
        }
    }
    let dnp = fp.child("attr").is_some_and(|a| (1..8).any(|i| a.atom(i) == Some("dnp")));
    let fp_idx = fps.len() as u32;
    fps.push(Footprint { reference, value, at: o, props, dnp });
    let fa = at.f64(3).unwrap_or(0.0).to_radians();
    let (s, c) = fa.sin_cos();
    for pad in fp.children("pad") {
        let Some(pat) = pad.child("at") else { continue };
        let lx = pat.f64(1).unwrap_or(0.0);
        let ly = pat.f64(2).unwrap_or(0.0);
        let angle = pat.f64(3).unwrap_or(0.0);
        // KiCad: positive angles rotate counter-clockwise on screen (y down)
        let p = [o[0] + lx * c + ly * s, o[1] - lx * s + ly * c];
        let size = pad.child_xy("size").unwrap_or([0.0, 0.0]);
        let shape = pad.atom(3).unwrap_or("rect").to_string();
        let mut layers = Vec::new();
        if let Some(ls) = pad.child("layers") {
            for l in ls.items().iter().skip(1) {
                let name = match l {
                    Node::Sym(s) | Node::Str(s) => s.as_str(),
                    _ => continue,
                };
                if name == "*.Cu" {
                    layers = (0..su.n() as u16).collect();
                    break;
                } else if name == "F&B.Cu" {
                    layers.push(0);
                    layers.push((su.n() - 1) as u16);
                } else if let Some(i) = su.layer_index(name) {
                    layers.push(i as u16);
                }
            }
        }
        if layers.is_empty() {
            continue;
        }
        let drill = pad.child("drill").and_then(|d| {
            d.items().iter().skip(1).find_map(|x| match x {
                Node::Sym(s) => s.parse::<f64>().ok(),
                _ => None,
            })
        });
        let net = nets.resolve(pad.child("net"));
        let number = pad.atom(1).unwrap_or("").to_string();
        out.push(Pad { at: p, size, angle, shape, layers, net, drill: drill.unwrap_or(0.0), number, fp: fp_idx });
    }
}

// --- .kicad_pro (net classes) --------------------------------------------------

/// Simple KiCad wildcard match: `*` any sequence, `?` any single char.
pub fn wildcard_match(pat: &str, s: &str) -> bool {
    let p: Vec<char> = pat.chars().collect();
    let t: Vec<char> = s.chars().collect();
    let (mut i, mut j, mut star, mut mark) = (0usize, 0usize, None::<usize>, 0usize);
    while j < t.len() {
        if i < p.len() && (p[i] == '?' || p[i] == t[j]) {
            i += 1;
            j += 1;
        } else if i < p.len() && p[i] == '*' {
            star = Some(i);
            mark = j;
            i += 1;
        } else if let Some(si) = star {
            i = si + 1;
            mark += 1;
            j = mark;
        } else {
            return false;
        }
    }
    while i < p.len() && p[i] == '*' {
        i += 1;
    }
    i == p.len()
}

pub fn apply_project(b: &mut Board, pro_json: &str) -> Result<(), String> {
    let v: serde_json::Value = serde_json::from_str(pro_json).map_err(|e| e.to_string())?;
    let ns = &v["net_settings"];
    let mut classes = vec![NetClass {
        name: "Default".into(),
        track_width: None,
        diff_pair_width: None,
        diff_pair_gap: None,
    }];
    if let Some(arr) = ns["classes"].as_array() {
        for c in arr {
            let name = c["name"].as_str().unwrap_or("").to_string();
            let nc = NetClass {
                name: name.clone(),
                track_width: c["track_width"].as_f64(),
                diff_pair_width: c["diff_pair_width"].as_f64(),
                diff_pair_gap: c["diff_pair_gap"].as_f64(),
            };
            if name == "Default" {
                classes[0] = nc;
            } else {
                classes.push(nc);
            }
        }
    }
    let idx_of = |name: &str, classes: &Vec<NetClass>| classes.iter().position(|c| c.name == name);
    let mut net_class = vec![0u32; b.nets.len()];
    let patterns: Vec<(String, String)> = ns["netclass_patterns"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|p| Some((p["pattern"].as_str()?.to_string(), p["netclass"].as_str()?.to_string())))
                .collect()
        })
        .unwrap_or_default();
    for (id, name) in b.nets.iter().enumerate() {
        if name.is_empty() {
            continue;
        }
        // explicit assignment first
        let mut cls: Option<String> = None;
        match &ns["netclass_assignments"][name.as_str()] {
            serde_json::Value::String(s) => cls = Some(s.clone()),
            serde_json::Value::Array(a) => cls = a.first().and_then(|x| x.as_str()).map(|s| s.to_string()),
            _ => {}
        }
        if cls.is_none() {
            for (pat, c) in &patterns {
                if wildcard_match(pat, name) {
                    cls = Some(c.clone());
                    break;
                }
            }
        }
        if let Some(c) = cls {
            if let Some(i) = idx_of(&c, &classes) {
                net_class[id] = i as u32;
            }
        }
    }
    b.netclasses = classes;
    b.net_class = net_class;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wildcard() {
        assert!(wildcard_match("*/TERM_1M*", "/Front End/TERM_1M_A"));
        assert!(wildcard_match("PORT?LO*", "PORT1LO2"));
        assert!(!wildcard_match("PORT?LO*", "PORT12LO"));
        assert!(wildcard_match("*", ""));
    }
}

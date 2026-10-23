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
    /// absolute orientation in degrees (KiCad convention: in a board file the pad angle includes
    /// the footprint's rotation; only the position is relative to the footprint)
    pub angle: f64,
    pub shape: String,
    /// corner radius of a "roundrect" pad as a fraction of the smaller side (KiCad
    /// `roundrect_rratio`, default 0.25); 0 for other shapes
    pub rratio: f64,
    /// outline of a "custom" pad: the anchor shape and its primitives (polygons, rectangles,
    /// circles) in the pad's own frame (x along `size[0]` before rotation); empty otherwise
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub custom: Vec<Vec<P>>,
    /// copper layer indices this pad exists on
    pub layers: Vec<u16>,
    pub net: u32,
    pub drill: f64,
    /// pad number / name, e.g. "1", "A3"
    pub number: String,
    /// index into `Board::footprints`
    pub fp: u32,
    /// pin name and electrical type of the symbol pin (`(pinfunction "VDD")`, `(pintype "power_in")`),
    /// empty when the board file has none
    #[serde(skip)]
    pub pinfunction: String,
    #[serde(skip)]
    pub pintype: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Footprint {
    /// footprint name as in the file, e.g. "Capacitor_SMD:C_0805_2012Metric"
    pub lib: String,
    pub reference: String,
    pub value: String,
    pub at: P,
    /// user fields (symbol fields copied to the footprint), e.g. PI_LOAD
    pub props: Vec<(String, String)>,
    /// not fitted: "Do not populate" (KiCad 8+: `(attr … dnp)`), or not fitted in the
    /// chosen assembly variant (see `kisi_core::variant`)
    pub dnp: bool,
    /// left out of the bill of materials (`(attr … exclude_from_bom)`): gold fingers,
    /// test points, fiducials – parts a BOM never lists
    pub exclude_from_bom: bool,
    /// hierarchical sheet of the symbol (`(sheetname "/Power/")`); empty if unknown
    pub sheet: String,
    /// KiCad 10 assembly variants: the overrides of this footprint per variant
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub variants: Vec<FootprintVariant>,
}

impl Footprint {
    pub fn prop(&self, name: &str) -> Option<&str> {
        self.props.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
    /// Manufacturer part number from a footprint field – `MPN`, `Manufacturer Part Number`,
    /// `MFR PN`, … (case, spaces and punctuation ignored) –, the first non-empty one.
    pub fn mpn(&self) -> Option<&str> {
        const KEYS: &[&str] = &["mpn", "manufacturerpartnumber", "manufacturerpn", "mfrpartnumber", "mfrpn", "mfrno", "mfgpartnumber", "mfgpn"];
        let norm = |k: &str| k.chars().filter(|c| c.is_alphanumeric()).collect::<String>().to_lowercase();
        KEYS.iter().find_map(|key| self.props.iter().find(|(k, v)| norm(k) == *key && !v.trim().is_empty() && v.trim() != "~").map(|(_, v)| v.trim()))
    }
    /// The MPN when it says more than the Value field: present, not an extension of the value and
    /// not contained in it (Value `TPS62130`, MPN `TPS62130RGTR` → None; Value `IC FPGA XC7A50T`,
    /// MPN `XC7A50T` → None; Value `100n` or a manufacturer name → the MPN). For lists where a
    /// part number is expected.
    pub fn mpn_if_not_value(&self) -> Option<&str> {
        let m = self.mpn()?;
        let norm = |s: &str| s.chars().filter(|c| c.is_alphanumeric()).collect::<String>().to_lowercase();
        let (nm, nv) = (norm(m), norm(&self.value));
        (nv.is_empty() || !(nm.starts_with(&nv) || nv.contains(&nm))).then_some(m)
    }
    /// The part as named in part lists: the value, with the MPN in brackets when the value does
    /// not show it (`英联 (UM3204H)`, `120R (BLM03AG121SN1D)`); the MPN alone for an empty value.
    pub fn part_label(&self) -> String {
        match self.mpn_if_not_value() {
            Some(m) if self.value.trim().is_empty() || self.value.trim() == "~" => m.to_string(),
            Some(m) => format!("{} ({m})", self.value),
            None => self.value.clone(),
        }
    }
    /// Set a user field (or the value with name "Value").
    pub fn set_prop(&mut self, name: &str, value: &str) {
        if name.eq_ignore_ascii_case("Value") {
            self.value = value.to_string();
            return;
        }
        match self.props.iter_mut().find(|(k, _)| k.eq_ignore_ascii_case(name)) {
            Some(p) => p.1 = value.to_string(),
            None => self.props.push((name.to_string(), value.to_string())),
        }
    }
}

/// Overrides of one footprint in a KiCad 10 assembly variant (only what differs from the
/// footprint's own attributes; `None` = as in the footprint).
#[derive(Debug, Clone, Default, Serialize)]
pub struct FootprintVariant {
    pub name: String,
    pub dnp: Option<bool>,
    pub exclude_from_bom: Option<bool>,
    /// field overrides (name, value), e.g. ("Value", "10k")
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<(String, String)>,
}

/// An assembly variant declared in the board file (KiCad 10, `(variants (variant (name …)
/// (description …)))`).
#[derive(Debug, Clone, Default, Serialize)]
pub struct BoardVariant {
    pub name: String,
    pub description: String,
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
    /// assembly variants declared in the board file (KiCad 10)
    pub variants: Vec<BoardVariant>,
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
    "footprint", "gr_line", "gr_arc", "gr_rect", "gr_circle", "gr_poly", "variants",
];

/// `(dnp)`, `(dnp yes)`, `(dnp no)` (KiCad's "maybe absent bool": a bare keyword is true).
fn flag_value(n: &Node) -> bool {
    !matches!(n.atom(1), Some("no") | Some("false"))
}

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
            Some("variants") => {
                // KiCad 10: (variants (variant (name "A") (description "…")) …)
                for v in it.children("variant") {
                    let Some(name) = v.child_str("name").filter(|n| !n.is_empty()) else { continue };
                    if b.variants.iter().all(|x| x.name != name) {
                        b.variants.push(BoardVariant { name: name.to_string(), description: v.child_str("description").unwrap_or("").to_string() });
                    }
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

/// Outline of a pad in its own frame (centre at the origin, x along `size[0]`), rounded shapes
/// approximated; one polygon per piece of copper (custom pads: anchor and primitives).
pub fn pad_local_polygons(p: &Pad) -> Vec<Vec<P>> {
    if !p.custom.is_empty() {
        return p.custom.clone();
    }
    let (hx, hy) = (p.size[0] / 2.0, p.size[1] / 2.0);
    let r = pad_corner_radius(p).min(hx).min(hy);
    if r <= 1e-9 {
        return vec![vec![[-hx, -hy], [hx, -hy], [hx, hy], [-hx, hy]]];
    }
    // rectangle with rounded corners (oval: r = half the smaller side, circle: r = radius)
    let steps = if p.shape == "circle" { 4 } else { 3 };
    let mut v = Vec::with_capacity(4 * (steps + 1));
    for (cx, cy, a0) in [(hx - r, hy - r, 0.0), (-(hx - r), hy - r, 90.0), (-(hx - r), -(hy - r), 180.0), (hx - r, -(hy - r), 270.0)] {
        for i in 0..=steps {
            let t = (a0 + 90.0 * i as f64 / steps as f64).to_radians();
            v.push([cx + r * t.cos(), cy + r * t.sin()]);
        }
    }
    vec![v]
}

/// Corner radius of a pad: roundrect from its ratio, oval and circle half the smaller side.
pub fn pad_corner_radius(p: &Pad) -> f64 {
    let m = p.size[0].min(p.size[1]);
    match p.shape.as_str() {
        "circle" | "oval" => m / 2.0,
        "roundrect" => p.rratio * m,
        _ => 0.0,
    }
}

/// Outline polygons of a pad on the board (see `pad_local_polygons`).
pub fn pad_polygons(p: &Pad) -> Vec<Vec<P>> {
    // same orientation convention as the viewer: rotate by -angle (y down)
    let th = -p.angle.to_radians();
    let (s, c) = th.sin_cos();
    pad_local_polygons(p).into_iter().map(|poly| poly.into_iter().map(|q| [p.at[0] + q[0] * c - q[1] * s, p.at[1] + q[0] * s + q[1] * c]).collect()).collect()
}

/// Outline of a pad as one polygon (custom pads: the convex hull of their pieces).
pub fn pad_polygon(p: &Pad) -> Vec<P> {
    let mut polys = pad_polygons(p);
    if polys.len() == 1 {
        return polys.pop().unwrap();
    }
    convex_hull(polys.into_iter().flatten().collect())
}

fn convex_hull(mut pts: Vec<P>) -> Vec<P> {
    pts.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));
    pts.dedup();
    if pts.len() < 3 {
        return pts;
    }
    let mut lower: Vec<P> = Vec::new();
    for &p in &pts {
        while lower.len() >= 2 && geom::cross(geom::sub(lower[lower.len() - 1], lower[lower.len() - 2]), geom::sub(p, lower[lower.len() - 2])) <= 0.0 {
            lower.pop();
        }
        lower.push(p);
    }
    let mut upper: Vec<P> = Vec::new();
    for &p in pts.iter().rev() {
        while upper.len() >= 2 && geom::cross(geom::sub(upper[upper.len() - 1], upper[upper.len() - 2]), geom::sub(p, upper[upper.len() - 2])) <= 0.0 {
            upper.pop();
        }
        upper.push(p);
    }
    lower.pop();
    upper.pop();
    lower.extend(upper);
    lower
}

/// Copper of a custom pad in its own frame: the anchor (`(options (anchor rect|circle))`, the
/// pad size) and the primitives (`gr_poly`, `gr_rect`, `gr_circle`; thick lines and arcs as
/// rectangles of their width).
fn custom_outline(pad: &Node, size: P) -> Vec<Vec<P>> {
    let (hx, hy) = (size[0] / 2.0, size[1] / 2.0);
    let circle = |c: P, r: f64| -> Vec<P> { (0..16).map(|i| (i as f64 / 16.0 * std::f64::consts::TAU).sin_cos()).map(|(s, co)| [c[0] + r * co, c[1] + r * s]).collect() };
    let mut out = Vec::new();
    let anchor = pad.child("options").and_then(|o| o.child_str("anchor")).unwrap_or("rect");
    if hx > 0.0 && hy > 0.0 {
        out.push(if anchor == "circle" { circle([0.0, 0.0], hx) } else { vec![[-hx, -hy], [hx, -hy], [hx, hy], [-hx, hy]] });
    }
    let thick = |a: P, b: P, w: f64| -> Option<Vec<P>> {
        let d = geom::sub(b, a);
        let l = geom::len(d);
        if l < 1e-9 || w <= 0.0 {
            return None;
        }
        let n = geom::scale([-d[1] / l, d[0] / l], w / 2.0);
        Some(vec![geom::add(a, n), geom::add(b, n), geom::sub(b, n), geom::sub(a, n)])
    };
    if let Some(prims) = pad.child("primitives") {
        for g in prims.items().iter().skip(1) {
            let w = g.child_f64("width").unwrap_or(0.0);
            match g.head() {
                Some("gr_poly") => {
                    if let Some(p) = g.pts().or_else(|| g.child("pts").and_then(|n| n.pts())) {
                        if p.len() >= 3 {
                            out.push(p.to_vec());
                        }
                    }
                }
                Some("gr_rect") => {
                    if let (Some(a), Some(b)) = (g.child_xy("start"), g.child_xy("end")) {
                        out.push(vec![a, [b[0], a[1]], b, [a[0], b[1]]]);
                    }
                }
                Some("gr_circle") => {
                    if let (Some(c), Some(e)) = (g.child_xy("center"), g.child_xy("end")) {
                        out.push(circle(c, geom::len(geom::sub(e, c)) + w / 2.0));
                    }
                }
                Some("gr_line") => {
                    if let Some(p) = g.child_xy("start").zip(g.child_xy("end")).and_then(|(a, b)| thick(a, b, w)) {
                        out.push(p);
                    }
                }
                Some("gr_arc") => {
                    if let (Some(a), Some(m), Some(b)) = (g.child_xy("start"), g.child_xy("mid"), g.child_xy("end")) {
                        let pts = geom::tessellate_arc(a, m, b, 15f64.to_radians());
                        for s in pts.windows(2) {
                            if let Some(p) = thick(s[0], s[1], w) {
                                out.push(p);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
    out
}

/// Pad axes in board coordinates: (u along size.x, v along size.y).
fn pad_axes(p: &Pad) -> (P, P) {
    let th = -p.angle.to_radians();
    let (s, c) = th.sin_cos();
    ([c, s], [-s, c])
}

/// Half extents of a pad's copper along its own axes (custom pads: of all their pieces).
fn pad_half_extents(p: &Pad) -> (f64, f64) {
    if p.custom.is_empty() {
        return (p.size[0] / 2.0, p.size[1] / 2.0);
    }
    p.custom.iter().flatten().fold((0.0f64, 0.0f64), |m, q| (m.0.max(q[0].abs()), m.1.max(q[1].abs())))
}

/// Whether point `q` lies on the copper of pad `p` (within `tol` mm of its outline): rounded
/// corners of roundrect, oval and circle pads count, custom pads by their pieces.
pub fn pad_contains(p: &Pad, q: P, tol: f64) -> bool {
    let (u, v) = pad_axes(p);
    let o = geom::sub(q, p.at);
    let (x, y) = (geom::dot(o, u), geom::dot(o, v));
    if !p.custom.is_empty() {
        let inside = |poly: &Vec<P>| {
            let mut c = false;
            for k in 0..poly.len() {
                let (a, b) = (poly[k], poly[(k + 1) % poly.len()]);
                if (a[1] > y) != (b[1] > y) && x < a[0] + (y - a[1]) / (b[1] - a[1]) * (b[0] - a[0]) {
                    c = !c;
                }
            }
            c
        };
        let near = |poly: &Vec<P>| {
            (0..poly.len()).any(|k| {
                let (a, b) = (poly[k], poly[(k + 1) % poly.len()]);
                let d = geom::sub(b, a);
                let t = (geom::dot(geom::sub([x, y], a), d) / geom::dot(d, d).max(1e-12)).clamp(0.0, 1.0);
                geom::len(geom::sub([x, y], geom::add(a, geom::scale(d, t)))) <= tol
            })
        };
        return p.custom.iter().any(|poly| poly.len() >= 3 && (inside(poly) || near(poly)));
    }
    let (hx, hy) = (p.size[0] / 2.0, p.size[1] / 2.0);
    let r = pad_corner_radius(p).min(hx).min(hy);
    let (dx, dy) = ((x.abs() - (hx - r)).max(0.0), (y.abs() - (hy - r)).max(0.0));
    (dx * dx + dy * dy).sqrt() <= r + tol
}

/// The axis along which a track passes through a pad (`true`: the pad's u axis, along
/// `size[0]`). `off` is where the track ends, relative to the pad centre, `dir` the track's
/// direction there (sign irrelevant), `(hu, hv)` the half extents. A track that ends off-centre
/// enters along the axis of its offset (e.g. at one end of a long pad); a track ending near the
/// centre along its own direction; a diagonal entry along the pad's long axis.
pub fn pad_entry_axis_u(off: (f64, f64), dir: (f64, f64), half: (f64, f64)) -> bool {
    let (nu, nv) = (off.0.abs() / half.0.max(1e-9), off.1.abs() / half.1.max(1e-9));
    if nu.max(nv) >= 0.3 && (nu - nv).abs() >= 0.2 {
        return nu > nv;
    }
    let (cu, cv) = (dir.0.abs(), dir.1.abs());
    if (cu - cv).abs() >= 0.2 {
        return cu > cv;
    }
    half.0 >= half.1
}

/// Adds synthetic tracks through every SMD pad of a signal net that a track connects to, along
/// the axis on which the track enters it (`pad_entry_axis_u`): two halves from the pad centre to
/// either end, so that the pad is a T-junction where the track ends (the track graph joins
/// everything inside a pad to its centre). Their width is the pad's extent across that axis,
/// their total length the extent along it, shortened for rounded corners so that the copper
/// area matches the pad (circle: π/4 of the diameter). The pad is then sampled like a trace: its
/// impedance (wide copper, antipads below it) shows up in the map and in the net profile. The
/// viewer draws these tracks as the pad's own outline.
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
        let (u, v) = pad_axes(p);
        let (hu, hv) = pad_half_extents(p);
        let reach = hu.max(hv);
        // a connecting track ends inside the pad (nearest to the centre if several)
        let mut entry: Option<((f64, f64), (f64, f64), f64)> = None;
        let (cx, cy) = ((p.at[0] / cell).floor() as i64, (p.at[1] / cell).floor() as i64);
        let span = (reach / cell).ceil() as i64;
        for dx in -span..=span {
            for dy in -span..=span {
                for &(e, d) in ends.get(&(p.net, layer, cx + dx, cy + dy)).map(|x| x.as_slice()).unwrap_or(&[]) {
                    let o = geom::sub(e, p.at);
                    let (ou, ov) = (geom::dot(o, u), geom::dot(o, v));
                    if ou.abs() > hu + 0.01 || ov.abs() > hv + 0.01 {
                        continue;
                    }
                    let dist = geom::len(o);
                    if entry.is_none_or(|x| dist < x.2) {
                        entry = Some(((ou, ov), (geom::dot(d, u), geom::dot(d, v)), dist));
                    }
                }
            }
        }
        let Some((off, dir, _)) = entry else { continue };
        let along_u = pad_entry_axis_u(off, dir, (hu, hv));
        let (axis, along, across) = if along_u { (u, 2.0 * hu, 2.0 * hv) } else { (v, 2.0 * hv, 2.0 * hu) };
        if along <= 0.0 || across <= 0.0 {
            continue;
        }
        // same copper area as the pad: corners of radius r take (4 − π)·r² off the rectangle
        let r = if p.custom.is_empty() { pad_corner_radius(p).min(along / 2.0).min(across / 2.0) } else { 0.0 };
        let length = (along - (4.0 - std::f64::consts::PI) * r * r / across).max(0.05 * along);
        let half = length / 2.0;
        for sign in [-1.0, 1.0] {
            new_tracks.push(Track { a: p.at, b: geom::add(p.at, geom::scale(axis, sign * half)), width: across, layer, net: p.net, item: pi as u32, is_arc: false, pad: true });
        }
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
    let attr = |name: &str| fp.child("attr").is_some_and(|a| (1..10).any(|i| a.atom(i) == Some(name)));
    let (dnp, exclude_from_bom) = (attr("dnp"), attr("exclude_from_bom"));
    let sheet = fp.child_str("sheetname").unwrap_or("").to_string();
    // KiCad 10: (variant (name "B") (dnp yes) (exclude_from_bom yes) (field (name "Value") (value "10k")))
    let variants = fp
        .children("variant")
        .filter_map(|v| {
            let name = v.child_str("name").filter(|n| !n.is_empty())?.to_string();
            let fields = v.children("field").filter_map(|f| Some((f.child_str("name")?.to_string(), f.child_str("value").unwrap_or("").to_string()))).collect();
            Some(FootprintVariant { name, dnp: v.child("dnp").map(flag_value), exclude_from_bom: v.child("exclude_from_bom").map(flag_value), fields })
        })
        .collect();
    let fp_idx = fps.len() as u32;
    let lib = fp.atom(1).unwrap_or("").to_string();
    fps.push(Footprint { lib, reference, value, at: o, props, dnp, exclude_from_bom, sheet, variants });
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
        let rratio = if shape == "roundrect" { pad.child_f64("roundrect_rratio").unwrap_or(0.25).clamp(0.0, 0.5) } else { 0.0 };
        let custom = if shape == "custom" { custom_outline(pad, size) } else { vec![] };
        let pinfunction = pad.child_str("pinfunction").unwrap_or("").to_string();
        let pintype = pad.child_str("pintype").unwrap_or("").to_string();
        out.push(Pad { at: p, size, angle, shape, rratio, custom, layers, net, drill: drill.unwrap_or(0.0), number, fp: fp_idx, pinfunction, pintype });
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

    /// Pin names and types of the pads; the MPN field under its usual names, shown where the
    /// value is not a part number.
    #[test]
    fn pin_functions_and_mpn() {
        let pcb = r#"(kicad_pcb (version 20241229) (generator "pcbnew")
  (layers (0 "F.Cu" signal) (2 "B.Cu" signal))
  (net 0 "") (net 1 "VSYS")
  (footprint "U" (layer "F.Cu") (at 0 0) (property "Reference" "U4") (property "Value" "英联")
    (property "Manufacturer Part Number" "") (property "MPN" "UM3204H")
    (pad "C1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net 1 "VSYS") (pinfunction "VCCB") (pintype "power_in")))
  (footprint "U" (layer "F.Cu") (at 5 0) (property "Reference" "U1") (property "Value" "TPS62130")
    (property "MANUFACTURER PART NUMBER" "TPS62130RGTR")
    (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net 1 "VSYS")))
  (footprint "C" (layer "F.Cu") (at 9 0) (property "Reference" "C1") (property "Value" "100n") (property "Mfr. PN" "GRM155R61A104KA01D")
    (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net 1 "VSYS")))
  (footprint "U" (layer "F.Cu") (at 15 0) (property "Reference" "U18") (property "Value" "IC FPGA XC7A50T-2CSG325C") (property "MPN" "XC7A50T-2CSG325C")
    (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net 1 "VSYS"))))"#;
        let b = load_pcb(pcb).unwrap();
        assert_eq!((b.pads[0].pinfunction.as_str(), b.pads[0].pintype.as_str()), ("VCCB", "power_in"));
        assert_eq!(b.pads[1].pinfunction, "");
        let f = &b.footprints;
        assert_eq!((f[0].mpn(), f[0].mpn_if_not_value()), (Some("UM3204H"), Some("UM3204H")));
        assert_eq!((f[1].mpn(), f[1].mpn_if_not_value()), (Some("TPS62130RGTR"), None));
        assert_eq!(f[2].mpn_if_not_value(), Some("GRM155R61A104KA01D"));
        assert_eq!((f[0].part_label().as_str(), f[1].part_label().as_str()), ("英联 (UM3204H)", "TPS62130"));
        assert_eq!(f[3].mpn_if_not_value(), None);
    }
    /// KiCad 10 board (format 20260206): board-level variant list and per-footprint
    /// overrides, as written by `PCB_IO_KICAD_SEXPR::formatVariants` and the footprint writer.
    #[test]
    fn kicad10_variants_and_sheets() {
        let pcb = r#"(kicad_pcb (version 20260206) (generator "pcbnew") (generator_version "10.0")
  (layers (0 "F.Cu" signal) (2 "B.Cu" signal))
  (variants (variant (name "Lite") (description "without the radio")) (variant (name "Full")))
  (net 0 "") (net 1 "A") (net 2 "B")
  (footprint "R_0402" (layer "F.Cu") (uuid "1") (at 10 10 90)
    (property "Reference" "R1" (at 0 0 0) (layer "F.SilkS")) (property "Value" "0" (at 0 0 0) (layer "F.Fab"))
    (path "/aaa/bbb") (sheetname "/Radio/") (sheetfile "radio.kicad_sch")
    (attr smd)
    (pad "1" smd roundrect (at -0.5 0) (size 0.6 0.6) (layers "F.Cu") (net 1 "A"))
    (pad "2" smd roundrect (at 0.5 0) (size 0.6 0.6) (layers "F.Cu") (net 2 "B"))
    (variant (name "Lite") (dnp yes) (exclude_from_bom yes))
    (variant (name "Full") (field (name "Value") (value "22")) (field (name "MPN") (value "RC0402-22R"))))
  (footprint "TP" (layer "B.Cu") (at 20 10)
    (property "Reference" "TP1") (property "Value" "PCB") (sheetname "/") (attr smd exclude_from_bom dnp)
    (pad "1" smd circle (at 0 0) (size 1 1) (layers "B.Cu") (net 1 "A"))
    (variant (name "Full") (dnp no))))
"#;
        let b = load_pcb(pcb).unwrap();
        assert_eq!(b.variants.len(), 2);
        assert_eq!((b.variants[0].name.as_str(), b.variants[0].description.as_str()), ("Lite", "without the radio"));
        let r1 = &b.footprints[0];
        assert_eq!((r1.reference.as_str(), r1.sheet.as_str(), r1.dnp, r1.exclude_from_bom), ("R1", "/Radio/", false, false));
        assert_eq!(r1.variants.len(), 2);
        assert_eq!((r1.variants[0].dnp, r1.variants[0].exclude_from_bom), (Some(true), Some(true)));
        assert_eq!(r1.variants[1].dnp, None);
        assert_eq!(r1.variants[1].fields, vec![("Value".to_string(), "22".to_string()), ("MPN".to_string(), "RC0402-22R".to_string())]);
        let tp = &b.footprints[1];
        assert!(tp.dnp && tp.exclude_from_bom && tp.sheet == "/");
        assert_eq!(tp.variants[0].dnp, Some(false));
        // KiCad 8/9 boards: no variants, sheets still read
        let old = pcb.replace("(variants (variant (name \"Lite\") (description \"without the radio\")) (variant (name \"Full\")))", "");
        assert!(load_pcb(&old).unwrap().variants.is_empty());
    }

    /// Board with one footprint (rotated by `fp_rot`) whose pad "2" (absolute angle `pad_rot`,
    /// size `size`, shape `shape`) is reached by a track from `from` to `to`.
    fn pad_board(fp_rot: f64, pad_rot: f64, shape: &str, size: (f64, f64), from: P, to: P) -> Board {
        let pcb = format!(
            r#"(kicad_pcb (version 20241229) (generator "pcbnew")
  (layers (0 "F.Cu" signal) (2 "B.Cu" signal))
  (net 0 "") (net 1 "SIG") (net 2 "OTHER")
  (footprint "Thunderscope_Rev5:KEMET_UD2-4.5NUN-L" (layer "F.Cu") (at 106.6211 70.0161 {fp_rot})
    (property "Reference" "K1") (property "Value" "relay")
    (pad "1" smd {shape} (at -3.8 2.97 {pad_rot}) (size {sx} {sy}) (roundrect_rratio 0.25) (layers "F.Cu" "F.Mask") (net 2 "OTHER"))
    (pad "2" smd {shape} (at -0.6 2.97 {pad_rot}) (size {sx} {sy}) (roundrect_rratio 0.25) (layers "F.Cu" "F.Mask") (net 1 "SIG")))
  (segment (start {fx} {fy}) (end {tx} {ty}) (width 0.213) (layer "F.Cu") (net 1)))"#,
            sx = size.0,
            sy = size.1,
            fx = from[0],
            fy = from[1],
            tx = to[0],
            ty = to[1]
        );
        load_pcb(&pcb).unwrap()
    }

    /// The two synthetic half tracks of the pad, from its centre, as one track end to end.
    fn pad_track(b: &Board) -> Track {
        let t: Vec<&Track> = b.tracks.iter().filter(|t| t.pad).collect();
        assert_eq!(t.len(), 2, "two synthetic half tracks");
        assert!(t[0].a == t[1].a && t[0].width == t[1].width && t[0].item == t[1].item, "halves from the pad centre");
        let (l0, l1) = (geom::len(geom::sub(t[0].b, t[0].a)), geom::len(geom::sub(t[1].b, t[1].a)));
        assert!((l0 - l1).abs() < 1e-9 && geom::dot(geom::sub(t[0].b, t[0].a), geom::sub(t[1].b, t[1].a)) < 0.0, "opposite halves of equal length");
        Track { a: t[0].b, b: t[1].b, ..*t[0] }
    }

    /// ThunderScope K1000 (`KEMET_UD2-4.5NUN-L` at −90°, pad "2" at 270° absolute, 0.8 × 1.86 mm
    /// roundrect): the track enters diagonally and ends 0.525 mm off-centre along the pad's long
    /// side. The pad is modelled along its long axis, 0.8 mm wide – it was a 1.86 mm wide stub of
    /// 0.24 mm, drawn as a 1.86 mm blob.
    #[test]
    fn pad_track_kemet_relay() {
        let b = pad_board(-90.0, 270.0, "roundrect", (0.8, 1.86), [105.0511, 68.5411], [104.1761, 69.4161]);
        let p = &b.pads[1];
        // position: local (-0.6, 2.97) rotated by -90° (clockwise on screen) around the footprint
        assert!((p.at[0] - 103.6511).abs() < 1e-9 && (p.at[1] - 69.4161).abs() < 1e-9, "{:?}", p.at);
        // the pad's long side runs along x on the board: 1.86 mm wide, 0.8 mm high
        let poly = pad_polygon(p);
        let (w, h) = (poly.iter().map(|q| q[0]).fold(f64::MIN, f64::max) - poly.iter().map(|q| q[0]).fold(f64::MAX, f64::min), poly.iter().map(|q| q[1]).fold(f64::MIN, f64::max) - poly.iter().map(|q| q[1]).fold(f64::MAX, f64::min));
        assert!((w - 1.86).abs() < 1e-6 && (h - 0.8).abs() < 1e-6, "{w} × {h}");
        let t = pad_track(&b);
        assert!((t.width - 0.8).abs() < 1e-9, "width {}", t.width);
        assert!((t.a[1] - t.b[1]).abs() < 1e-9, "along x");
        // length: 1.86 minus the rounded corners (r = 0.2 mm): same copper area as the pad
        let len = geom::len(geom::sub(t.b, t.a));
        let area = 0.8 * 1.86 - (4.0 - std::f64::consts::PI) * 0.2 * 0.2;
        assert!((len * t.width - area).abs() < 1e-9, "len {len}");
    }

    /// Pads of every orientation (footprint rotation and pad angle): the pad track runs along the
    /// axis the track enters on, as wide as the pad across it.
    #[test]
    fn pad_track_orientations_and_shapes() {
        let pi = std::f64::consts::PI;
        for fp_rot in [0.0, 90.0, 180.0, 270.0, -90.0] {
            for local in [0.0, 90.0, 180.0, 270.0] {
                let pad_rot = fp_rot + local;
                for (shape, area) in [("rect", 0.8 * 1.86), ("roundrect", 0.8 * 1.86 - (4.0 - pi) * 0.04), ("oval", 0.8 * 1.86 - (4.0 - pi) * 0.16)] {
                    // pad "2" on the board, its long axis from its polygon
                    let probe = pad_board(fp_rot, pad_rot, shape, (0.8, 1.86), [0.0, 0.0], [0.0, 0.0]);
                    let p = &probe.pads[1];
                    let th = -pad_rot.to_radians();
                    let long = [-th.sin(), th.cos()]; // size.y (1.86) axis
                    // a track arriving along the long axis, ending at the centre
                    let from = geom::add(p.at, geom::scale(long, 3.0));
                    let b = pad_board(fp_rot, pad_rot, shape, (0.8, 1.86), from, p.at);
                    let t = pad_track(&b);
                    let d = geom::sub(t.b, t.a);
                    assert!((t.width - 0.8).abs() < 1e-9, "{shape} fp {fp_rot} pad {pad_rot}: width {}", t.width);
                    assert!(geom::cross(d, long).abs() < 1e-9, "{shape} fp {fp_rot} pad {pad_rot}: along the long axis");
                    assert!((geom::len(d) * t.width - area).abs() < 1e-9, "{shape}: area");
                    // the same pad entered from its long side (track perpendicular, ending at the
                    // centre): a short, 1.86 mm wide section
                    let side = [long[1], -long[0]];
                    let b = pad_board(fp_rot, pad_rot, shape, (0.8, 1.86), geom::add(p.at, geom::scale(side, 3.0)), p.at);
                    let t = pad_track(&b);
                    assert!((t.width - 1.86).abs() < 1e-9 && geom::dot(geom::sub(t.b, t.a), long).abs() < 1e-9, "{shape} fp {fp_rot} pad {pad_rot}: side entry");
                }
            }
        }
        // a round pad: π/4 of the diameter, so that the area matches
        let probe = pad_board(0.0, 0.0, "circle", (0.6, 0.6), [0.0, 0.0], [0.0, 0.0]);
        let c = probe.pads[1].at;
        let b = pad_board(0.0, 0.0, "circle", (0.6, 0.6), [c[0] + 2.0, c[1]], c);
        let t = pad_track(&b);
        assert!((t.width - 0.6).abs() < 1e-9 && (geom::len(geom::sub(t.b, t.a)) - pi / 4.0 * 0.6).abs() < 1e-9);
        // a track ending outside the pad's outline (but inside its circumscribed circle) adds none
        let b = pad_board(0.0, 0.0, "rect", (0.8, 1.86), [c[0] + 3.0, c[1] - 0.6], [c[0] + 0.5, c[1] - 0.6]);
        assert!(b.tracks.iter().all(|t| !t.pad));
    }

    /// `pad_contains` follows the rotated outline with its rounded corners.
    #[test]
    fn pad_contains_outline() {
        // K1000 pad "2": 0.8 × 1.86 roundrect (r 0.2) at 270°: 1.86 along x, 0.8 along y on the board
        let b = pad_board(-90.0, 270.0, "roundrect", (0.8, 1.86), [0.0, 0.0], [0.0, 0.0]);
        let p = &b.pads[1];
        let at = |dx: f64, dy: f64| [p.at[0] + dx, p.at[1] + dy];
        assert!(pad_contains(p, at(0.9, 0.0), 0.0) && pad_contains(p, at(0.0, 0.39), 0.0));
        assert!(!pad_contains(p, at(0.0, 0.5), 0.0) && !pad_contains(p, at(0.95, 0.0), 0.0), "not across the long side");
        // the corner is rounded: (0.92, 0.39) lies in the bounding box but off the copper
        assert!(!pad_contains(p, at(0.92, 0.39), 0.02) && pad_contains(p, at(0.85, 0.33), 0.0));
        assert!(pad_contains(p, at(0.94, 0.0), 0.02), "tolerance");
        // a circle
        let c = pad_board(0.0, 0.0, "circle", (0.6, 0.6), [0.0, 0.0], [0.0, 0.0]);
        let q = &c.pads[1];
        assert!(pad_contains(q, [q.at[0] + 0.2, q.at[1] + 0.2], 0.0) && !pad_contains(q, [q.at[0] + 0.25, q.at[1] + 0.25], 0.0));
    }

    #[test]
    fn custom_pad_outline() {
        let pcb = r#"(kicad_pcb (version 20241229) (generator "pcbnew")
  (layers (0 "F.Cu" signal) (2 "B.Cu" signal))
  (net 0 "") (net 1 "GND")
  (footprint "QFN" (layer "F.Cu") (at 10 10 -90) (property "Reference" "U1")
    (pad "1" smd custom (at 1 0 270) (size 0.2 0.2) (layers "F.Cu") (net 1 "GND")
      (options (clearance outline) (anchor rect))
      (primitives (gr_poly (pts (xy 0 -0.1) (xy 1.0 -0.1) (xy 1.0 0.1) (xy 0 0.1)) (width 0) (fill yes)))))
  (zone (net 1) (net_name "GND") (layer "F.Cu") (filled_polygon (layer "F.Cu") (pts (xy 0 0) (xy 1 0) (xy 1 1))))
  )"#;
        let b = load_pcb(pcb).unwrap();
        let p = &b.pads[0];
        assert_eq!((p.shape.as_str(), p.custom.len()), ("custom", 2));
        // the primitive reaches 1 mm along the pad's x axis; the pad is at 270° absolute, so on
        // the board that is +y (rotation by −270° = +90° with y down): from y 11 to 12
        let poly = pad_polygon(p);
        let ymax = poly.iter().map(|q| q[1]).fold(f64::MIN, f64::max);
        assert!((p.at[0] - 10.0).abs() < 1e-9 && (p.at[1] - 11.0).abs() < 1e-9, "{:?}", p.at);
        assert!((ymax - 12.0).abs() < 1e-9, "{ymax}");
        assert!(pad_contains(p, [10.0, 11.8], 0.0) && !pad_contains(p, [10.3, 11.5], 0.0), "on its pieces only");
        assert_eq!(b.footprints[0].lib, "QFN");
    }

    #[test]
    fn wildcard() {
        assert!(wildcard_match("*/TERM_1M*", "/Front End/TERM_1M_A"));
        assert!(wildcard_match("PORT?LO*", "PORT1LO2"));
        assert!(!wildcard_match("PORT?LO*", "PORT12LO"));
        assert!(wildcard_match("*", ""));
    }
}

//! Compact scene description for the viewer (JSON + flat zone buffer).

use crate::board::{Board, NetClass, Stackup};
use serde::Serialize;

#[derive(Serialize)]
pub struct Scene<'a> {
    pub version: &'a str,
    pub generator: &'a str,
    pub stackup: &'a Stackup,
    pub nets: &'a [String],
    pub net_class: &'a [u32],
    pub netclasses: &'a [NetClass],
    /// [minx, miny, maxx, maxy]
    pub bbox: [f64; 4],
    /// 7 per track: ax, ay, bx, by, width, layer, net
    pub tracks: Vec<f32>,
    /// 7 per via: x, y, size, drill, from, to, net
    pub vias: Vec<f32>,
    /// 8 per pad: x, y, sx, sy, angle, shape(0 rect,1 round,2 oval,3 roundrect,4 custom), layer mask, net
    pub pads: Vec<f32>,
    /// corner radius per pad (mm): roundrect from its ratio, oval and circle half the smaller side
    pub pad_radius: Vec<f32>,
    /// custom pads: (pad index, outline polygons in the pad's own frame, like `pads` x/y/angle)
    pub pad_custom: Vec<(u32, Vec<Vec<[f32; 2]>>)>,
    pub outline: Vec<Vec<[f32; 2]>>,
    /// indices of synthetic tracks through SMD pads
    pub pad_tracks: Vec<u32>,
    /// pad index of every synthetic pad track (same order as `pad_tracks`): drawn as the pad
    pub pad_track_pad: Vec<u32>,
    /// footprint index per pad, and footprint reference designators
    pub pad_fp: Vec<u32>,
    pub fp_refs: Vec<String>,
    pub warnings: &'a [String],
}

pub fn scene(b: &Board) -> Scene<'_> {
    let bb = b.bbox.map(|x| [x.min[0], x.min[1], x.max[0], x.max[1]]).unwrap_or([0.0, 0.0, 100.0, 100.0]);
    let mut tracks = Vec::with_capacity(b.tracks.len() * 7);
    for t in &b.tracks {
        tracks.extend_from_slice(&[
            t.a[0] as f32, t.a[1] as f32, t.b[0] as f32, t.b[1] as f32, t.width as f32, t.layer as f32, t.net as f32,
        ]);
    }
    let mut vias = Vec::with_capacity(b.vias.len() * 7);
    for v in &b.vias {
        vias.extend_from_slice(&[
            v.at[0] as f32, v.at[1] as f32, v.size as f32, v.drill as f32, v.from as f32, v.to as f32, v.net as f32,
        ]);
    }
    let mut pads = Vec::with_capacity(b.pads.len() * 8);
    for p in &b.pads {
        let shape = match p.shape.as_str() {
            "circle" => 1.0,
            "oval" => 2.0,
            "roundrect" => 3.0,
            "custom" if !p.custom.is_empty() => 4.0,
            _ => 0.0,
        };
        let mask: u32 = p.layers.iter().fold(0, |m, &l| if l < 24 { m | (1 << l) } else { m });
        pads.extend_from_slice(&[
            p.at[0] as f32, p.at[1] as f32, p.size[0] as f32, p.size[1] as f32, p.angle as f32, shape, mask as f32, p.net as f32,
        ]);
    }
    Scene {
        version: &b.version,
        generator: &b.generator,
        stackup: b.stackup(),
        nets: &b.nets,
        net_class: &b.net_class,
        netclasses: &b.netclasses,
        bbox: bb,
        tracks,
        vias,
        pads,
        pad_radius: b.pads.iter().map(|p| crate::board::pad_corner_radius(p) as f32).collect(),
        pad_custom: b.pads.iter().enumerate().filter(|(_, p)| !p.custom.is_empty()).map(|(i, p)| (i as u32, p.custom.iter().map(|c| c.iter().map(|q| [q[0] as f32, q[1] as f32]).collect()).collect())).collect(),
        outline: b.outline.iter().map(|p| p.iter().map(|q| [q[0] as f32, q[1] as f32]).collect()).collect(),
        pad_tracks: b.tracks.iter().enumerate().filter(|(_, t)| t.pad).map(|(i, _)| i as u32).collect(),
        pad_track_pad: b.tracks.iter().filter(|t| t.pad).map(|t| t.item).collect(),
        pad_fp: b.pads.iter().map(|p| p.fp).collect(),
        fp_refs: b.footprints.iter().map(|f| f.reference.clone()).collect(),
        warnings: &b.warnings,
    }
}

/// Zone fills as flat buffers: `pts` (x,y pairs) and `index` (layer, net, start, count)
/// where start/count are in points.
pub fn zone_buffers(b: &Board) -> (Vec<f32>, Vec<u32>) {
    let total: usize = b.zones.iter().map(|z| z.pts.len()).sum();
    let mut pts = Vec::with_capacity(total * 2);
    let mut index = Vec::with_capacity(b.zones.len() * 4);
    for z in &b.zones {
        index.extend_from_slice(&[z.layer as u32, z.net, (pts.len() / 2) as u32, z.pts.len() as u32]);
        for p in &z.pts {
            pts.push(p[0] as f32);
            pts.push(p[1] as f32);
        }
    }
    (pts, index)
}

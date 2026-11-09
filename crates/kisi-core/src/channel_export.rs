//! `kisi export`: a lane or a whole bus for circuit and RF simulators (`kisi_si::export`) – SPICE
//! subcircuits with test benches for ngspice and Xyce, Touchstone files (a bus as one N-port with
//! its port map) and a KiCad simulation project. The lanes, profiles and ends are the channel
//! check's.

use crate::board::Board;
use crate::export::files::{lane_files, stem, touchstone, BenchEnds, ExportFile, Formats};
use crate::export::{lane_circuit, renormalize, sig4, ExportOpts, LaneCircuit};
use crate::extract::{Analysis, FieldResult, Params};
use crate::eye::{ConnEnd, EndModel};
use crate::num::Cx;
use crate::profile::Profiles;
use crate::rf::RfParams;
use crate::service::{lane_net, Project};
use crate::topology::{self, Bus, Lane};
use serde_json::{json, Value};
use std::collections::HashMap;

/// What to export: one lane (by its name, a net of it, or the last part of either) or a bus.
pub enum Pick<'a> {
    Lane(&'a str),
    Bus(&'a str),
}

fn short(s: &str) -> &str {
    s.rsplit('/').next().unwrap_or(s)
}

/// The lanes `pick` names, with their bus.
pub fn find<'p>(p: &'p Project, pick: &Pick) -> Result<(&'p Bus, Vec<&'p Lane>), String> {
    let buses = p.buses();
    match pick {
        Pick::Bus(name) => {
            let b = buses.iter().find(|b| b.name == *name || b.name.eq_ignore_ascii_case(name)).ok_or_else(|| {
                format!(
                    "bus '{name}' not found (buses: {})",
                    buses.iter().map(|b| b.name.as_str()).collect::<Vec<_>>().join(", ")
                )
            })?;
            Ok((b, b.lanes.iter().collect()))
        }
        Pick::Lane(name) => {
            let hit = |l: &Lane| -> bool {
                let nets = l.nets.iter().chain(&l.partner_nets).map(|&n| p.board.net_name(n));
                l.name == *name || short(&l.name) == *name || nets.clone().any(|n| n == *name || short(n) == *name)
            };
            for b in buses {
                if let Some(l) = b.lanes.iter().find(|l| hit(l)) {
                    return Ok((b, vec![l]));
                }
            }
            Err(format!("lane '{name}' not found (see `kisi interfaces`)"))
        }
    }
}

/// One lane exported: its circuit, the bench ends, its files (with its corners' files) and its
/// tolerance corners (tag, circuit, mean impedance of the lane's nets).
pub struct Done {
    lc: LaneCircuit,
    ends: BenchEnds,
    files: Vec<ExportFile>,
    z_mean: Option<f64>,
    corners: Vec<(&'static str, String, LaneCircuit, Option<f64>)>,
}

/// A tolerance corner of an export (`--corners`, the viewer's choice with the option "fab stackups &
/// tolerances" on): the board with every tolerance at its limit toward a lower ("zlo") or a higher
/// ("zhi") impedance (`stackups::corner_board`), analysed again with the same parameters and field
/// results (cross-sections without one at the formula level).
pub struct Corner {
    pub tag: &'static str,
    /// what it changes ("dielectric thickness -10 %, er +5 %, …")
    pub what: String,
    pub board: Board,
    pub an: Analysis,
}

/// The two corners of `board` under the tolerances `t`.
pub fn corners(board: &Board, params: &Params, t: &crate::study::Tolerances) -> Vec<Corner> {
    let pr = Params { tolerance: None, ..params.clone() };
    [("zlo", true), ("zhi", false)]
        .into_iter()
        .map(|(tag, low)| {
            let b = crate::stackups::corner_board(board, t, low);
            let an = crate::extract::analyze(&b, &pr);
            Corner { tag, what: crate::stackups::corner_text(t, low), board: b, an }
        })
        .collect()
}

/// The cross-sections along the lanes' nets on a board (id → text, routed length).
pub fn lane_geoms(board: &Board, an: &Analysis, lanes: &[&Lane]) -> HashMap<String, (String, f64)> {
    let nets: std::collections::HashSet<u32> =
        lanes.iter().flat_map(|l| l.nets.iter().chain(&l.partner_nets).copied()).collect();
    let mut w: HashMap<String, (String, f64)> = HashMap::new();
    for ch in an.pieces.chunks(4) {
        let (ti, t0, t1, xi) = (ch[0] as usize, ch[1], ch[2], ch[3] as usize);
        let Some(tr) = board.tracks.get(ti).filter(|t| nets.contains(&t.net)) else { continue };
        let Some(g) = an.cross_sections.get(xi).and_then(|x| x.geom).and_then(|g| an.geometries.get(g as usize)) else {
            continue;
        };
        let e = w.entry(g.id.clone()).or_insert_with(|| (g.text.clone(), 0.0));
        e.1 += crate::geom::len(crate::geom::sub(tr.b, tr.a)) * (t1 - t0).abs();
    }
    w
}

/// The cross-sections of the lanes' nets at the corners that have no field result yet (id, text,
/// routed length): the field solver's work before a corner export at the nominal value's level.
pub fn corner_geoms(
    cs: &[Corner],
    lanes: &[&Lane],
    field: &HashMap<String, FieldResult>,
) -> Vec<(String, String, f64)> {
    let mut w: HashMap<String, (String, f64)> = HashMap::new();
    for c in cs {
        for (id, (text, l)) in lane_geoms(&c.board, &c.an, lanes) {
            if !field.contains_key(&id) {
                w.entry(id).or_insert((text, 0.0)).1 += l;
            }
        }
    }
    w.into_iter().map(|(id, (text, l))| (id, text, l)).collect()
}

/// Length-weighted mean impedance of a lane's nets.
fn lane_z(an: &Analysis, l: &Lane) -> Option<f64> {
    let (mut len, mut zs) = (0.0, 0.0);
    for st in an.nets.iter().filter(|s| l.nets.contains(&s.net)) {
        if let Some(z) = st.z_mean {
            len += st.length;
            zs += z * st.length;
        }
    }
    (len > 0.0).then(|| zs / len)
}

/// Insertion loss at `f` (dB, positive): |S21|, a pair's |Sdd21|, from kisi's S-parameters.
fn il_at(lc: &LaneCircuit, f: f64) -> Option<f64> {
    let k = lc.freq.iter().enumerate().min_by(|a, b| (a.1 - f).abs().total_cmp(&(b.1 - f).abs()))?.0;
    let s = lc.s_kisi.get(k)?;
    let n = lc.ports.len();
    let at = |i: usize, j: usize| s[i * n + j];
    let t = if n == 4 { (at(2, 0) - at(3, 0) - at(2, 1) + at(3, 1)).scale(0.5) } else { at(1, 0) };
    Some(-20.0 * t.abs().max(1e-30).log10())
}

/// One lane to export, with its profile and ends as the channel check (CLI) or the viewer (with
/// the user's settings) has them.
pub struct LaneReq<'a> {
    pub lane: &'a Lane,
    pub target: Option<(f64, bool)>,
    /// as for the check (`fmax` at least 1.25 × the bit rate)
    pub rp: RfParams,
    pub profile: String,
    pub gbps: f64,
    /// 10–90 % edge of the transmitter (s) and its amplitude at the receiver (V)
    pub rise: f64,
    pub swing: f64,
    pub tx: EndModel,
    pub rx: EndModel,
    pub reversed: bool,
    pub conn_end: Option<&'a ConnEnd>,
    /// the export's frequency range (`rp` as the check uses it, unless given)
    pub freq: FreqRange,
}

/// The top frequency a lane's profile needs: 5 × Nyquist (2.5 × the bit rate, NRZ) or 0.5 / t_r of the
/// 10–90 % edge (the edge's knee 0.35 / t_r with margin), whichever is higher.
pub fn profile_fmax(gbps: f64, rise: f64) -> f64 {
    let edge = if rise > 0.0 { 0.5 / rise } else { 0.0 };
    (2.5 * gbps * 1e9).max(edge)
}

/// The export's frequency range: the ladder's top frequency and the Touchstone grid. `fmax` (Hz)
/// replaces the check's, `fmax_profile` follows the lane's profile: 5 × Nyquist, or the edge's
/// bandwidth with margin (0.5 / t_r of the 10–90 % edge) where that is higher – a slow bit rate with a
/// fast edge (a reference clock, the generic profiles) keeps its edge; at least 1.25 × the bit rate
/// either way. The grid runs from `fmin` in steps of `fmin` (Hz; at most 10000 points),
/// by default 500 points (from fmax/500).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct FreqRange {
    pub fmin: Option<f64>,
    pub fmax: Option<f64>,
    pub fmax_profile: bool,
}

impl FreqRange {
    /// From the CLI's `--fmin GHZ` and `--fmax GHZ|profile`.
    pub fn parse(fmin: Option<&str>, fmax: Option<&str>) -> Result<FreqRange, String> {
        let ghz = |s: &str, name: &str| -> Result<f64, String> {
            match s.parse::<f64>() {
                Ok(v) if v > 0.0 && v.is_finite() => Ok(v * 1e9),
                _ => {
                    Err(format!("{name} {s}: a frequency in GHz{}", if name == "--fmax" { ", or profile" } else { "" }))
                }
            }
        };
        let mut r = FreqRange { fmin: fmin.map(|s| ghz(s, "--fmin")).transpose()?, ..Default::default() };
        match fmax {
            Some("profile") => r.fmax_profile = true,
            Some(s) => r.fmax = Some(ghz(s, "--fmax")?),
            None => {}
        }
        if let (Some(a), Some(b)) = (r.fmin, r.fmax) {
            if a >= b {
                return Err("--fmin must be below --fmax".into());
            }
        }
        Ok(r)
    }
    /// One range for all lanes of a bus (the N-port needs one grid): the highest f max of the lanes
    /// (`rates`: their bit rates in Gbit/s and edges in s).
    pub fn for_bus(&self, rp: &RfParams, rates: &[(f64, f64)]) -> FreqRange {
        let fmax = rates.iter().map(|&(g, tr)| self.apply(rp, g, tr).fmax).fold(0.0, f64::max);
        FreqRange { fmin: self.fmin, fmax: (fmax > 0.0).then_some(fmax), fmax_profile: false }
    }
    /// The check's RF parameters with this range for a lane at `gbps` with a 10–90 % edge of `rise` s.
    pub fn apply(&self, rp: &RfParams, gbps: f64, rise: f64) -> RfParams {
        let mut r = rp.clone();
        if self.fmax_profile {
            r.fmax = profile_fmax(gbps, rise);
        } else if let Some(f) = self.fmax {
            r.fmax = f;
        }
        r.fmax = r.fmax.max(1.25 * gbps * 1e9);
        if let Some(f) = self.fmin {
            r.npts = ((r.fmax / f).round() as usize).clamp(10, 10000);
        }
        r
    }
}

/// Export one lane (`dir`: folder prefix of its files, "" or "name/").
pub fn export_req(
    board: &Board,
    an: &Analysis,
    field: &HashMap<String, FieldResult>,
    req: &LaneReq,
    f: Formats,
    dir: &str,
    board_name: &str,
    corners: &[Corner],
) -> Result<Done, String> {
    let rp_range = req.freq.apply(&req.rp, req.gbps, req.rise);
    let (l, rp, cd, opts) = (
        req.lane,
        &rp_range,
        topology::ComponentData::builtin(),
        ExportOpts { model: f.model, ..ExportOpts::default() },
    );
    if l.ends.len() < 2 {
        return Err(format!("lane {}: open (one end), nothing to export", l.name));
    }
    // the ends as the channel check's eye uses them (connector end models, a termination next to
    // the receiver as the receiver's termination)
    let ev = topology::lane_eval(
        board,
        an,
        field,
        l,
        req.target,
        rp,
        Some((req.gbps * 1e9, req.rise, 7, Some(req.tx), Some(req.rx))),
        req.reversed,
        req.conn_end,
        None,
        &[],
        &cd,
    )?;
    let mut notes: Vec<String> = Vec::new();
    notes.extend(ev.rx_note.clone());
    if let Some(n) = &ev.end_note {
        notes.push(n.clone());
    }
    if l.kind == "connector-connector" {
        notes.push("lane from connector to connector: the profile's transmitter and receiver stand in for the devices behind the connectors".into());
    }
    let ends = BenchEnds {
        profile: req.profile.clone(),
        bitrate: req.gbps * 1e9,
        rise: req.rise,
        swing: req.swing,
        tx: ev.tx_used.unwrap_or(req.tx),
        rx: ev.rx_used.unwrap_or(req.rx),
        notes,
    };
    let lc = lane_circuit(board, an, field, l, req.target, rp, req.reversed, &cd, req.gbps * 0.5e9, &opts)?;
    let mut files = lane_files(&lc, &ends, f, board_name, rp.fmax, opts.per_lambda, dir);
    // the tolerance corners: the same lane on the corner boards, named <lane>_zlo / _zhi (files,
    // subcircuits) in folders of their own, SPICE and Touchstone only
    let mut cs = Vec::new();
    for c in corners {
        let mut lane = l.clone();
        lane.name = format!("{}_{}", l.name, c.tag);
        let clc =
            lane_circuit(&c.board, &c.an, field, &lane, req.target, rp, req.reversed, &cd, req.gbps * 0.5e9, &opts)?;
        let mut cf = lane_files(
            &clc,
            &ends,
            Formats { kicad: false, ..f },
            board_name,
            rp.fmax,
            opts.per_lambda,
            &format!("{dir}{}/", c.tag),
        );
        cf.retain(|x| !x.path.ends_with("README.md"));
        let line = vec![format!("tolerance corner {}: {} (kisi's fab tolerances)", c.tag, c.what)];
        stamp(&mut cf, &line, &line);
        files.extend(cf);
        cs.push((c.tag, c.what.clone(), clc, lane_z(&c.an, l)));
    }
    let mut d = Done { lc, ends, files, z_mean: lane_z(an, l), corners: cs };
    if !d.corners.is_empty() {
        corner_readme(&mut d, req.gbps * 0.5e9, dir);
    }
    Ok(d)
}

/// The README's section on the tolerance corners: the three variants, their files, what each
/// changes, the mean impedance and the insertion loss at Nyquist.
fn corner_readme(d: &mut Done, f_nyq: f64, dir: &str) {
    let st = stem(&d.lc.lane);
    let ext = format!("s{}p", d.lc.ports.len());
    let num = |v: Option<f64>, u: &str, d: usize| v.map(|x| format!("{x:.d$} {u}")).unwrap_or_else(|| "–".into());
    let mut t = format!(
        "\n## Tolerance corners\n\nThe lane in three variants – each tolerance of kisi's set (the stackup and tolerances above; kisi's defaults are estimates) at its limit at once, toward a lower or a higher impedance: a worst case, wider than the RSS band. Every dielectric, copper layer and track of the board alike (pads and vias as they are; no etch factor). Each corner folder holds the same SPICE decks and Touchstone file for its subcircuit; load the three side by side to see the spread.\n\n| variant | subcircuit, files | what changes | mean Z of the lane's nets | IL at Nyquist ({:.3} GHz) |\n|---|---|---|---|---|\n| nominal | `{}`: `{st}.lib`, `{st}.{ext}` | the stackup above | {} | {} |\n",
        f_nyq / 1e9,
        d.lc.subckt,
        num(d.z_mean, "Ω", 1),
        num(il_at(&d.lc, f_nyq), "dB", 2),
    );
    for (tag, what, lc, z) in &d.corners {
        let cs = stem(&lc.lane);
        t.push_str(&format!(
            "| {} (`{tag}/`) | `{}`: `{tag}/{cs}.lib`, `{tag}/{cs}.{ext}` | {what} | {} | {} |\n",
            if *tag == "zlo" { "low Z" } else { "high Z" },
            lc.subckt,
            num(*z, "Ω", 1),
            num(il_at(lc, f_nyq), "dB", 2),
        ));
    }
    if let Some(r) = d.files.iter_mut().find(|x| x.path == format!("{dir}README.md")) {
        r.content.push_str(&t);
    }
}

/// The channel check's request for a lane of the project (profile from the net class).
fn req_of<'a>(
    p: &'a Project,
    profiles: &Profiles,
    assign: &HashMap<String, String>,
    l: &'a Lane,
    rp: &RfParams,
    freq: FreqRange,
) -> Result<LaneReq<'a>, String> {
    let tnet = lane_net(p, l);
    let class = p.class_name(tnet);
    let assign = p.assign(assign);
    let pid = assign
        .get(&class)
        .cloned()
        .unwrap_or_else(|| profiles.auto(p.board.net_name(tnet), &class, l.diff).to_string());
    let prof = profiles.get(&pid).ok_or_else(|| format!("unknown profile '{pid}'"))?;
    let gbps = prof.rate_gbps;
    let cend = l
        .ends
        .iter()
        .find(|e| e.role == "connector")
        .and_then(|e| p.config.as_ref()?.connector_ends.get(&e.refdes))
        .map(|e| &e.end);
    Ok(LaneReq {
        lane: l,
        target: p.target(tnet),
        rp: RfParams { fmax: rp.fmax.max(gbps * 1e9 * 1.25), ..rp.clone() },
        profile: pid.clone(),
        gbps,
        rise: prof.tx.rise_ps * 1e-12,
        swing: prof.tx.swing_mv * 1e-3,
        tx: prof.tx_model(),
        rx: prof.rx,
        reversed: l.rx_end == 0,
        conn_end: cend,
        freq,
    })
}

fn lane_json(d: &Done) -> Value {
    let mut v = lane_meta(&d.lc, &d.ends);
    if !d.corners.is_empty() {
        let f_nyq = d.ends.bitrate * 0.5;
        v["z_mean_ohm"] = json!(d.z_mean);
        v["il_nyq_db"] = json!(il_at(&d.lc, f_nyq));
        v["corners"] = d
            .corners
            .iter()
            .map(|(tag, what, lc, z)| {
                let mut c = lane_meta(lc, &d.ends);
                c["corner"] = json!(tag);
                c["changes"] = json!(what);
                c["folder"] = json!(tag);
                c["z_mean_ohm"] = json!(z);
                c["il_nyq_db"] = json!(il_at(lc, f_nyq));
                c
            })
            .collect();
    }
    v
}

fn lane_meta(lc: &LaneCircuit, ends: &BenchEnds) -> Value {
    json!({
        "lane": lc.lane, "subckt": lc.subckt, "model": if lc.pair { "4-port (pair)" } else if lc.odd_mode { "2-port (pair in the odd mode)" } else { "2-port" },
        "ports": lc.ports.iter().zip(&lc.port_info).enumerate().map(|(i, (n, info))| json!({"port": i + 1, "node": n, "at": info})).collect::<Vec<_>>(),
        "z_ref_ohm": lc.z_ref, "length_mm": lc.length_mm, "delay_ps": lc.delay_ps,
        "profile": ends.profile, "rate_gbps": ends.bitrate / 1e9,
        "bench_ends": {"tx": ends.tx, "rx": ends.rx, "rise_ps": ends.rise * 1e12, "swing_v": ends.swing, "notes": ends.notes},
        "frequency": {"fmin_hz": lc.freq.first(), "fmax_hz": lc.freq.last(), "points": lc.freq.len()},
        "line_model": lc.model, "merged_sections": lc.merged,
        "elements": {"r": lc.counts.r, "l": lc.counts.l, "c": lc.counts.c, "sources": lc.counts.sources, "rlgc_sections": lc.counts.sections, "cells": lc.counts.cells, "total": lc.counts.elements()},
        "check_against_kisi": lc.fidelity,
        "notes": lc.notes,
    })
}

/// Run the export. Returns the summary, the files (paths relative to the export folder) and the
/// folder's name.
pub fn export(
    p: &Project,
    profiles: &Profiles,
    assign: &HashMap<String, String>,
    pick: Pick,
    f: Formats,
    rp: &RfParams,
    freq: FreqRange,
    corners: Option<&crate::study::Tolerances>,
) -> Result<(Value, Vec<ExportFile>, String), String> {
    let (bus, lanes) = find(p, &pick)?;
    let board = p.pcb_path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let field = &p.params.field;
    let cs = corners.map(|t| self::corners(&p.board, &p.params, t)).unwrap_or_default();
    match pick {
        Pick::Lane(_) => {
            let d = export_req(
                &p.board,
                &p.an,
                field,
                &req_of(p, profiles, assign, lanes[0], rp, freq)?,
                f,
                "",
                &board,
                &cs,
            )?;
            Ok(lane_result(d, &bus.name))
        }
        Pick::Bus(_) => {
            let mut done = Vec::new();
            let mut skipped = Vec::new();
            let rates: Vec<(f64, f64)> = lanes
                .iter()
                .filter_map(|l| req_of(p, profiles, assign, l, rp, freq).ok().map(|r| (r.gbps, r.rise)))
                .collect();
            let freq = freq.for_bus(rp, &rates);
            for l in &lanes {
                let dir = format!("{}/", stem(&l.name));
                match req_of(p, profiles, assign, l, rp, freq)
                    .and_then(|r| export_req(&p.board, &p.an, field, &r, f, &dir, &board, &cs))
                {
                    Ok(d) => done.push(d),
                    Err(e) => skipped.push(format!("{}: {e}", l.name)),
                }
            }
            bus_result(&bus.name, &bus.devices, done, skipped, f, &board)
        }
    }
}

/// Put a comment block (the stackup used, the tolerances; ASCII lines) at the head of every file of
/// an export, in each format's comment syntax: SPICE files (`.lib`, `.cir`) and Touchstone files the
/// short form after their leading comment lines (`*`, `!` – the title line of a deck stays first), the
/// README the full one as a section at its end, the KiCad sheet the first line as a comment of its
/// title block. Other files stay as they are.
pub fn stamp(files: &mut [ExportFile], short: &[String], lines: &[String]) {
    if lines.is_empty() {
        return;
    }
    let after_comments = |text: &str, mark: char, prefix: &str| -> String {
        let mut out = String::with_capacity(text.len() + 80 * short.len());
        let mut done = false;
        for l in text.split_inclusive('\n') {
            if !done && !l.starts_with(mark) {
                for x in short {
                    out.push_str(prefix);
                    out.push_str(x);
                    out.push('\n');
                }
                done = true;
            }
            out.push_str(l);
        }
        if !done {
            for x in short {
                out.push_str(prefix);
                out.push_str(x);
                out.push('\n');
            }
        }
        out
    };
    for f in files.iter_mut() {
        let ext = f.path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
        if ext == "lib" || ext == "cir" {
            f.content = after_comments(&f.content, '*', "* ");
        } else if ext.starts_with('s')
            && ext.ends_with('p')
            && ext[1..ext.len() - 1].chars().all(|c| c.is_ascii_digit())
            && ext.len() > 2
        {
            f.content = after_comments(&f.content, '!', "! ");
        } else if f.path.ends_with("README.md") {
            f.content.push_str(&format!("\n## Stackup\n\n```text\n{}\n```\n", lines.join("\n")));
        } else if ext == "kicad_sch" {
            if let Some(i) = f.content.find("    (comment 1 ") {
                if let Some(j) = f.content[i..].find('\n') {
                    let q = |t: &str| format!("\"{}\"", t.replace('\\', "\\\\").replace('"', "\\\""));
                    let first = lines.first().map(|l| l.trim_end_matches(':').to_string()).unwrap_or_default();
                    f.content.insert_str(
                        i + j + 1,
                        &format!("    (comment 2 {})\n", q(&format!("{first} - layers in the README"))),
                    );
                }
            }
        }
    }
}

/// Summary and files of one lane's export (and the folder's name).
pub fn lane_result(d: Done, bus: &str) -> (Value, Vec<ExportFile>, String) {
    let name = stem(&d.lc.lane);
    let v = json!({"export": "lane", "name": d.lc.lane, "bus": bus, "lanes": [lane_json(&d)], "files": d.files.iter().map(|x| x.path.clone()).collect::<Vec<_>>(), "note": NOTE});
    (v, d.files, name)
}

/// Summary and files of a bus's export: the lanes in their folders, the bus subcircuit and the
/// N-port Touchstone file with its port map.
pub fn bus_result(
    bus: &str,
    devices: &[String],
    done: Vec<Done>,
    skipped: Vec<String>,
    f: Formats,
    board: &str,
) -> Result<(Value, Vec<ExportFile>, String), String> {
    if done.is_empty() {
        return Err(format!("bus {bus}: no lane could be exported ({})", skipped.join("; ")));
    }
    let name = stem(&bus.replace(' ', "_").replace('↔', "-"));
    let mut files: Vec<ExportFile> = done.iter().flat_map(|d| d.files.iter().cloned()).collect();
    let dev_a = devices.first().cloned().unwrap_or_default();
    let (bus_files, port_map, nports, z_bus) = bus_level(&done, &dev_a, bus, board, f, &name);
    files.extend(bus_files);
    let v = json!({"export": "bus", "name": bus, "devices": devices,
        "lanes": done.iter().map(lane_json).collect::<Vec<_>>(), "skipped": skipped,
        "n_port": {"ports": nports, "z_ref_ohm": z_bus, "port_map": port_map,
            "note": if done.iter().all(|d| d.lc.freq == done[0].lc.freq) { Value::Null } else { json!("no N-port Touchstone file: the lanes have different frequency grids (f max from profiles with different bit rates)") }},
        "files": files.iter().map(|x| x.path.clone()).collect::<Vec<_>>(), "note": NOTE});
    Ok((v, files, name))
}

const NOTE: &str = "SPICE: the lane's board channel as kisi's check models it (RLGC ladders with fitted R(f)/G(f) networks – or, with --model compact, lossless-line cells with the same networks lumped –, kisi's via model, parts with estimated parasitics, stubs; pairs with modal coupled blocks); check_against_kisi compares the exported circuit with kisi's own S-parameters. Benches use the profile's linear ends; the receiver's termination resistors on the board act as the receiver's termination and are not in the subcircuit or the Touchstone file. Not included: the channel behind a connector, crosstalk between lanes, IBIS ends.";

/// Bus-level files: the bus subcircuit (all lanes, ports ordered by device side) and the N-port
/// Touchstone file with its port map.
fn bus_level(
    done: &[Done],
    dev_a: &str,
    bus: &str,
    board: &str,
    f: Formats,
    name: &str,
) -> (Vec<ExportFile>, Vec<Value>, usize, f64) {
    // per lane: does it run from device B to device A (then its ports are swapped)?
    let flips: Vec<bool> =
        done.iter().map(|d| d.lc.port_info.first().is_some_and(|s| !s.contains(&format!(" at {dev_a}.")))).collect();
    // reference: the most common lane reference
    let mut zs: Vec<f64> = done.iter().map(|d| d.lc.z_ref).collect();
    zs.sort_by(|a, b| a.total_cmp(b));
    let z_bus = zs.iter().copied().max_by_key(|z| zs.iter().filter(|x| (*x - z).abs() < 1e-9).count()).unwrap_or(50.0);
    // ports: side A of every lane, then side B
    let mut side_a: Vec<(usize, usize, String)> = Vec::new(); // (lane, local port, info)
    let mut side_b: Vec<(usize, usize, String)> = Vec::new();
    for (i, d) in done.iter().enumerate() {
        let half = d.lc.ports.len() / 2;
        for k in 0..half {
            let (near, far) = (k, k + half);
            let (a, b) = if flips[i] { (far, near) } else { (near, far) };
            side_a.push((i, a, d.lc.port_info[a].clone()));
            side_b.push((i, b, d.lc.port_info[b].clone()));
        }
    }
    let all: Vec<(usize, usize, String)> = side_a.into_iter().chain(side_b).collect();
    let n = all.len();
    let port_map: Vec<Value> = all.iter().enumerate().map(|(q, (i, k, info))| json!({"port": q + 1, "lane": done[*i].lc.lane, "lane_port": k + 1, "node": done[*i].lc.ports[*k], "at": info})).collect();
    let mut out = Vec::new();
    let one_grid = done.iter().all(|d| d.lc.freq == done[0].lc.freq);
    if f.touchstone && one_grid {
        let freq = &done[0].lc.freq;
        let mut s_all: Vec<Vec<Cx>> = Vec::with_capacity(freq.len());
        let ren: Vec<Vec<Vec<Cx>>> = done
            .iter()
            .map(|d| d.lc.s_kisi.iter().map(|s| renormalize(s, d.lc.ports.len(), d.lc.z_ref, z_bus)).collect())
            .collect();
        for k in 0..freq.len() {
            let mut m = vec![Cx::new(0.0, 0.0); n * n];
            for (r, (ir, kr, _)) in all.iter().enumerate() {
                for (c, (ic, kc, _)) in all.iter().enumerate() {
                    if ir == ic {
                        let np = done[*ir].lc.ports.len();
                        m[r * n + c] = ren[*ir][k][kr * np + kc];
                    }
                }
            }
            s_all.push(m);
        }
        let mut c = vec![format!(
            "kisi {}: bus {bus} as one {n}-port (kisi's channel model of the board), board {board}",
            env!("CARGO_PKG_VERSION")
        )];
        c.push(format!("ports 1..{} at device {dev_a}, ports {}..{n} at the other device; port k and port k+{} are the two ends of the same line", n / 2, n / 2 + 1, n / 2));
        c.push(format!("reference {} ohm at every port (lanes with another reference renormalised)", sig4(z_bus)));
        c.push("the lanes are independent in this model: crosstalk between lanes is not included (zero blocks)".into());
        for (q, pm) in port_map.iter().enumerate() {
            c.push(format!(
                "port {}: lane {} port {} ({})",
                q + 1,
                pm["lane"].as_str().unwrap_or(""),
                pm["lane_port"],
                pm["at"].as_str().unwrap_or("")
            ));
        }
        out.push(ExportFile { path: format!("{name}.s{n}p"), content: touchstone(&c, freq, &s_all, n, z_bus) });
        let mut csv = String::from("port,lane,lane_port,node,at\n");
        for pm in &port_map {
            csv.push_str(&format!(
                "{},{},{},{},\"{}\"\n",
                pm["port"],
                pm["lane"].as_str().unwrap_or(""),
                pm["lane_port"],
                pm["node"].as_str().unwrap_or(""),
                pm["at"].as_str().unwrap_or("").replace('"', "'")
            ));
        }
        out.push(ExportFile { path: format!("{name}_ports.csv"), content: csv });
    }
    if f.lib || f.spice || f.xyce || f.kicad {
        let mut s = format!("* kisi {}: bus {bus} as one subcircuit, board {board}\n* ports: side {dev_a} (lane by lane, P then N), then the other device in the same order, then the reference\n", env!("CARGO_PKG_VERSION"));
        for (q, (i, k, info)) in all.iter().enumerate() {
            s.push_str(&format!("*   {} = lane {} {} ({info})\n", q + 1, done[*i].lc.lane, done[*i].lc.ports[*k]));
        }
        for d in done {
            let st = stem(&d.lc.lane);
            s.push_str(&format!(".include \"{st}/{st}.lib\"\n"));
        }
        let pnames: Vec<String> = (1..=n).map(|q| format!("p{q}")).collect();
        let bname = format!("{name}_bus");
        s.push_str(&format!(".subckt {bname} {} ref\n", pnames.join(" ")));
        for (i, d) in done.iter().enumerate() {
            let nodes: Vec<String> = (0..d.lc.ports.len())
                .map(|k| {
                    all.iter()
                        .position(|(ii, kk, _)| *ii == i && *kk == k)
                        .map(|q| pnames[q].clone())
                        .unwrap_or_default()
                })
                .collect();
            s.push_str(&format!("X{} {} ref {}\n", i + 1, nodes.join(" "), d.lc.subckt));
        }
        s.push_str(&format!(".ends {bname}\n"));
        out.push(ExportFile { path: format!("{name}.lib"), content: s });
    }
    let mut r = format!("# kisi export: bus {bus}\n\nkisi {} – board `{board}`. {} lanes, each in its own folder (subcircuit, test benches, Touchstone, KiCad project – see the README there).\n\n", env!("CARGO_PKG_VERSION"), done.len());
    if f.touchstone {
        r.push_str(&format!("* `{name}.s{n}p` – the bus as one {n}-port (Touchstone 1.0, RI, {} Ω at every port), port map in its header and in `{name}_ports.csv`: ports 1…{} at device {dev_a}, ports {}…{n} at the other device, port k and port k + {} are the two ends of one line. The lanes are independent in kisi's model (no crosstalk between lanes).\n", sig4(z_bus), n / 2, n / 2 + 1, n / 2));
    }
    if f.spice || f.xyce || f.kicad {
        r.push_str(&format!("* `{name}.lib` – subcircuit `{name}_bus` with all lanes, ports in the same order as the N-port, then the reference.\n"));
    }
    r.push_str("\n| port | lane | at |\n|---|---|---|\n");
    for pm in &port_map {
        r.push_str(&format!(
            "| {} | {} | {} |\n",
            pm["port"],
            pm["lane"].as_str().unwrap_or(""),
            pm["at"].as_str().unwrap_or("")
        ));
    }
    out.push(ExportFile { path: "README.md".into(), content: r });
    (out, port_map, n, z_bus)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// --fmin/--fmax: GHz or "profile", the floor of 1.25 × the bit rate, the grid from fmin, one range
    /// for a bus.
    #[test]
    fn frequency_range() {
        let rp = RfParams::default();
        let d = FreqRange::parse(None, None).unwrap();
        assert_eq!(d, FreqRange::default());
        assert_eq!((d.apply(&rp, 1.0, 300e-12).fmax, d.apply(&rp, 1.0, 300e-12).npts), (25e9, 500));
        let p = FreqRange::parse(None, Some("profile")).unwrap();
        // LVDS (1 Gbit/s, 300 ps): 5 x Nyquist; a generic 1 Gbit/s profile with 100 ps edges: the edge;
        // the 100 MHz reference clock (0.2 Gbit/s, 300 ps): the edge
        assert_eq!(p.apply(&rp, 1.0, 300e-12).fmax, 2.5e9);
        assert!((p.apply(&rp, 1.0, 100e-12).fmax - 5e9).abs() < 1.0);
        assert!((p.apply(&rp, 0.2, 300e-12).fmax - 0.5 / 300e-12).abs() < 1.0);
        let f = FreqRange::parse(Some("0.01"), Some("10")).unwrap();
        let r = f.apply(&rp, 16.0, 18e-12);
        assert_eq!((r.fmax, r.npts), (20e9, 2000), "at least 1.25 x the bit rate, grid from 10 MHz");
        assert!(FreqRange::parse(Some("5"), Some("2")).is_err() && FreqRange::parse(None, Some("x")).is_err());
        assert_eq!(p.for_bus(&rp, &[(1.0, 300e-12), (16.0, 25e-12)]).fmax, Some(40e9));
    }
}

//! `kisi export`: a lane or a whole bus for circuit and RF simulators (`kisi_si::export`) – SPICE
//! subcircuits with test benches for ngspice and Xyce, Touchstone files (a bus as one N-port with
//! its port map) and a KiCad simulation project. The lanes, profiles and ends are the channel
//! check's.

use crate::export::files::{lane_files, stem, touchstone, BenchEnds, ExportFile, Formats};
use crate::export::{lane_circuit, renormalize, sig4, ExportOpts, LaneCircuit};
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
fn find<'p>(p: &'p Project, pick: &Pick) -> Result<(&'p Bus, Vec<&'p Lane>), String> {
    let buses = p.buses();
    match pick {
        Pick::Bus(name) => {
            let b = buses.iter().find(|b| b.name == *name || b.name.eq_ignore_ascii_case(name)).ok_or_else(|| format!("bus '{name}' not found (buses: {})", buses.iter().map(|b| b.name.as_str()).collect::<Vec<_>>().join(", ")))?;
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

/// One lane exported: its circuit, the bench ends and its files.
struct Done {
    lc: LaneCircuit,
    ends: BenchEnds,
    files: Vec<ExportFile>,
}

#[allow(clippy::too_many_arguments)]
fn export_lane(p: &Project, profiles: &Profiles, assign: &HashMap<String, String>, l: &Lane, f: Formats, rp: &RfParams, opts: &ExportOpts, dir: &str, cd: &topology::ComponentData) -> Result<Done, String> {
    if l.ends.len() < 2 {
        return Err(format!("lane {}: open (one end), nothing to export", l.name));
    }
    let tnet = lane_net(p, l);
    let class = p.class_name(tnet);
    let assign = p.assign(assign);
    let pid = assign.get(&class).cloned().unwrap_or_else(|| profiles.auto(p.board.net_name(tnet), &class, l.diff).to_string());
    let prof = profiles.get(&pid).ok_or_else(|| format!("unknown profile '{pid}'"))?;
    let target = p.target(tnet);
    let gbps = prof.rate_gbps;
    let rp = RfParams { fmax: rp.fmax.max(gbps * 1e9 * 1.25), ..rp.clone() };
    let reversed = l.rx_end == 0;
    let field = &p.params.field;
    // the ends as the channel check's eye uses them (connector end models, a termination next to
    // the receiver as the receiver's termination)
    let cend = l.ends.iter().find(|e| e.role == "connector").and_then(|e| p.config.as_ref()?.connector_ends.get(&e.refdes)).map(|e| &e.end);
    let rise = prof.tx.rise_ps * 1e-12;
    let ev = topology::lane_eval(&p.board, &p.an, field, l, target, &rp, Some((gbps * 1e9, rise, 7, Some(prof.tx_model()), Some(prof.rx))), reversed, cend, None, &[], cd)?;
    let mut notes: Vec<String> = Vec::new();
    notes.extend(ev.rx_note.clone());
    if let Some(n) = &ev.end_note {
        notes.push(n.clone());
    }
    if l.kind == "connector-connector" {
        notes.push("lane from connector to connector: the profile's transmitter and receiver stand in for the devices behind the connectors".into());
    }
    let ends = BenchEnds { profile: pid.clone(), bitrate: gbps * 1e9, rise, swing: prof.tx.swing_mv * 1e-3, tx: ev.tx_used.unwrap_or(prof.tx_model()), rx: ev.rx_used.unwrap_or(prof.rx), notes };
    let lc = lane_circuit(&p.board, &p.an, field, l, target, &rp, reversed, cd, gbps * 0.5e9, opts)?;
    let board = p.pcb_path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let files = lane_files(&lc, &ends, f, &board, rp.fmax, opts.per_lambda, dir);
    Ok(Done { lc, ends, files })
}

fn lane_json(d: &Done) -> Value {
    let lc = &d.lc;
    json!({
        "lane": lc.lane, "subckt": lc.subckt, "model": if lc.pair { "4-port (pair)" } else if lc.odd_mode { "2-port (pair in the odd mode)" } else { "2-port" },
        "ports": lc.ports.iter().zip(&lc.port_info).enumerate().map(|(i, (n, info))| json!({"port": i + 1, "node": n, "at": info})).collect::<Vec<_>>(),
        "z_ref_ohm": lc.z_ref, "length_mm": lc.length_mm, "delay_ps": lc.delay_ps,
        "profile": d.ends.profile, "rate_gbps": d.ends.bitrate / 1e9,
        "bench_ends": {"tx": d.ends.tx, "rx": d.ends.rx, "rise_ps": d.ends.rise * 1e12, "swing_v": d.ends.swing, "notes": d.ends.notes},
        "elements": {"r": lc.counts.r, "l": lc.counts.l, "c": lc.counts.c, "sources": lc.counts.sources, "rlgc_sections": lc.counts.sections},
        "check_against_kisi": lc.fidelity,
        "notes": lc.notes,
    })
}

/// Run the export. Returns the summary and the files (paths relative to the export folder).
pub fn export(p: &Project, profiles: &Profiles, assign: &HashMap<String, String>, pick: Pick, f: Formats, rp: &RfParams) -> Result<(Value, Vec<ExportFile>, String), String> {
    let (bus, lanes) = find(p, &pick)?;
    let cd = topology::ComponentData::builtin();
    let opts = ExportOpts::default();
    let board = p.pcb_path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    match pick {
        Pick::Lane(_) => {
            let d = export_lane(p, profiles, assign, lanes[0], f, rp, &opts, "", &cd)?;
            let name = stem(&d.lc.lane);
            let v = json!({"export": "lane", "name": d.lc.lane, "bus": bus.name, "lanes": [lane_json(&d)], "files": d.files.iter().map(|x| x.path.clone()).collect::<Vec<_>>(),
                "note": NOTE});
            Ok((v, d.files, name))
        }
        Pick::Bus(_) => {
            let mut done: Vec<Done> = Vec::new();
            let mut skipped: Vec<String> = Vec::new();
            for l in &lanes {
                let dir = format!("{}/", stem(&l.name));
                match export_lane(p, profiles, assign, l, f, rp, &opts, &dir, &cd) {
                    Ok(d) => done.push(d),
                    Err(e) => skipped.push(format!("{}: {e}", l.name)),
                }
            }
            if done.is_empty() {
                return Err(format!("bus {}: no lane could be exported ({})", bus.name, skipped.join("; ")));
            }
            let name = stem(&bus.name.replace(' ', "_").replace('↔', "-"));
            let mut files: Vec<ExportFile> = done.iter().flat_map(|d| d.files.iter().cloned()).collect();
            let dev_a = bus.devices.first().cloned().unwrap_or_default();
            let (bus_files, port_map, nports, z_bus) = bus_level(&done, &dev_a, &bus.name, &board, f, &name);
            files.extend(bus_files);
            let v = json!({"export": "bus", "name": bus.name, "devices": bus.devices,
                "lanes": done.iter().map(lane_json).collect::<Vec<_>>(), "skipped": skipped,
                "n_port": {"ports": nports, "z_ref_ohm": z_bus, "port_map": port_map},
                "files": files.iter().map(|x| x.path.clone()).collect::<Vec<_>>(), "note": NOTE});
            Ok((v, files, name))
        }
    }
}

const NOTE: &str = "SPICE: the lane's board channel as kisi's check models it (RLGC ladders with fitted R(f)/G(f) networks, kisi's via model, parts with estimated parasitics, stubs; pairs with modal coupled blocks); check_against_kisi compares the exported circuit with kisi's own S-parameters. Benches use the profile's linear ends; the receiver's termination resistors on the board act as the receiver's termination and are not in the subcircuit or the Touchstone file. Not included: the channel behind a connector, crosstalk between lanes, IBIS ends.";

/// Bus-level files: the bus subcircuit (all lanes, ports ordered by device side) and the N-port
/// Touchstone file with its port map.
fn bus_level(done: &[Done], dev_a: &str, bus: &str, board: &str, f: Formats, name: &str) -> (Vec<ExportFile>, Vec<Value>, usize, f64) {
    // per lane: does it run from device B to device A (then its ports are swapped)?
    let flips: Vec<bool> = done.iter().map(|d| d.lc.port_info.first().is_some_and(|s| !s.contains(&format!(" at {dev_a}.")))).collect();
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
    if f.touchstone {
        let freq = &done[0].lc.freq;
        let mut s_all: Vec<Vec<Cx>> = Vec::with_capacity(freq.len());
        let ren: Vec<Vec<Vec<Cx>>> = done.iter().map(|d| d.lc.s_kisi.iter().map(|s| renormalize(s, d.lc.ports.len(), d.lc.z_ref, z_bus)).collect()).collect();
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
        let mut c = vec![format!("kisi {}: bus {bus} as one {n}-port (kisi's channel model of the board), board {board}", env!("CARGO_PKG_VERSION"))];
        c.push(format!("ports 1..{} at device {dev_a}, ports {}..{n} at the other device; port k and port k+{} are the two ends of the same line", n / 2, n / 2 + 1, n / 2));
        c.push(format!("reference {} ohm at every port (lanes with another reference renormalised)", sig4(z_bus)));
        c.push("the lanes are independent in this model: crosstalk between lanes is not included (zero blocks)".into());
        for (q, pm) in port_map.iter().enumerate() {
            c.push(format!("port {}: lane {} port {} ({})", q + 1, pm["lane"].as_str().unwrap_or(""), pm["lane_port"], pm["at"].as_str().unwrap_or("")));
        }
        out.push(ExportFile { path: format!("{name}.s{n}p"), content: touchstone(&c, freq, &s_all, n, z_bus) });
        let mut csv = String::from("port,lane,lane_port,node,at\n");
        for pm in &port_map {
            csv.push_str(&format!("{},{},{},{},\"{}\"\n", pm["port"], pm["lane"].as_str().unwrap_or(""), pm["lane_port"], pm["node"].as_str().unwrap_or(""), pm["at"].as_str().unwrap_or("").replace('"', "'")));
        }
        out.push(ExportFile { path: format!("{name}_ports.csv"), content: csv });
    }
    if f.spice || f.xyce || f.kicad {
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
            let nodes: Vec<String> = (0..d.lc.ports.len()).map(|k| all.iter().position(|(ii, kk, _)| *ii == i && *kk == k).map(|q| pnames[q].clone()).unwrap_or_default()).collect();
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
        r.push_str(&format!("| {} | {} | {} |\n", pm["port"], pm["lane"].as_str().unwrap_or(""), pm["at"].as_str().unwrap_or("")));
    }
    out.push(ExportFile { path: "README.md".into(), content: r });
    (out, port_map, n, z_bus)
}

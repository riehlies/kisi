//! `kisi-validate pi-refine <board.kicad_pcb> [--config F | --no-config] [--rails A,B] [--variants a,b]
//!  [--auto N] [--setups FILE] [--ref-level N] [--max-ref N] [--json FILE] [--dump DIR --dump-rail NAME [--dump-layer L] [--window x0,y0,x1,y1]]`
//!
//! Experiment (docs/research/pi-quadtree.md): the DC IR drop of a board's rails on today's uniform raster,
//! on refined rasters and with corrected via contacts (`kisi_core::pi_refine`), each against a converged
//! reference – the base grid refined uniformly `--ref-level` times (default 3: cell/8) with the barrels as
//! equipotential discs and tracks narrower than 4 base cells as line conductors (as fits in `--max-ref` unknowns,
//! default 3 M). Per rail and variant: unknowns, time, peak memory, the error of the voltage field
//! (max, mean, away from vias and pads), at the loads and at the barrels. `--dump` writes the error maps
//! of one rail (raw f32 rasters and a JSON index) for `docs/research/pi-quadtree/plot.py`. `--auto N`: synthetic
//! setups for the N largest supply pours of a board without a power configuration; `--setups FILE` writes the setups.

use kisi_core::geom::{self, P};
use kisi_core::pi::{PiParams, RailSetup};
use kisi_core::pi_refine::{self, Contact, Detail, Mesh, Refine};
use kisi_core::service::{self, Project};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::time::Instant;

fn die(msg: &str) -> ! {
    eprintln!("pi-refine: {msg}");
    std::process::exit(2);
}

/// Peak resident memory (Linux; 0 elsewhere), in MB: reset to the current one, then read; the
/// difference to the memory before is what a solve adds (approximately: freed memory the allocator keeps
/// counts as resident).
fn peak_reset() {
    let _ = std::fs::write("/proc/self/clear_refs", "5");
}
fn status_mb(key: &str) -> f64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with(key))
                .and_then(|l| l.split_whitespace().nth(1).and_then(|x| x.parse::<f64>().ok()))
        })
        .map(|kb| kb / 1024.0)
        .unwrap_or(0.0)
}

/// Tracks narrower than this many base cells as line conductors in the "lines" variants (wider ones are
/// rasterised with at least this many cells across).
pub const LINES: f64 = pi_refine::LINE_CELLS;
/// … and in the reference (on its finer raster, wider tracks have ≥ 32 leaves across).
pub const REF_LINES: f64 = 4.0;

pub fn variant(name: &str) -> Option<Mesh> {
    let d = Mesh::default();
    let pm = Mesh::from(Refine::Peaceman);
    let qt = |l: u8| Mesh::from(Refine::Quadtree { levels: l, radius: 3.0 });
    let lines = |m: Mesh| Mesh { track_lines: LINES, ..m };
    Some(match name {
        "uniform" => d,
        "uniform_x2" => Mesh { uniform: 1, ..d },
        "disc" => Mesh { contact: Contact::Disc, disc_grow: pi_refine::DISC_GROW, ..d },
        "peaceman" => pm,
        "cut4" => Mesh { cut_cell: 4, ..d },
        "lines" => lines(d),
        "lines_pm" => lines(pm),
        "lines_pm_cut4" => Mesh { cut_cell: 4, ..lines(pm) },
        "lines_pm_x2" => Mesh { uniform: 1, ..lines(pm) },
        "qt1" => qt(1),
        "qt2" => qt(2),
        "qt2_pm" => Mesh { peaceman: true, ..qt(2) },
        "qt2_lines" => lines(qt(2)),
        "qt2_lines_pm" => lines(Mesh { peaceman: true, ..qt(2) }),
        "qt1_lines_pm" => lines(Mesh { peaceman: true, ..qt(1) }),
        "qt2s_lines_pm" => lines(Mesh { peaceman: true, via_radius: 1.5, ..qt(2) }),
        "pads2_lines_pm" => lines(Mesh { pad_levels: 2, ..pm }),
        "qt2_lines_pm_cut4" => Mesh { cut_cell: 4, ..lines(Mesh { peaceman: true, ..qt(2) }) },
        "qt2_neck" => Mesh { neck_levels: 1, ..qt(2) },
        "qt2_edge" => Mesh { edge_levels: 1, ..qt(2) },
        "qt3" => qt(3),
        _ => return None,
    })
}

struct Run {
    name: String,
    d: Detail,
    secs: f64,
    peak_mb: f64,
    cell: f64,
}

fn run(
    board: &kisi_core::board::Board,
    s: &RailSetup,
    name: &str,
    mesh: &Mesh,
    cell: Option<f64>,
) -> Result<Run, String> {
    // a cell given: no limit from the automatic cell's budget
    let params = PiParams { cell, max_cells: if cell.is_some() { 1 << 40 } else { 250_000 }, ..Default::default() };
    peak_reset();
    let rss0 = status_mb("VmRSS:");
    let t0 = Instant::now();
    let d = pi_refine::solve(board, s, &params, mesh, None)?;
    let secs = t0.elapsed().as_secs_f64();
    let cell = d.result.cell;
    Ok(Run { name: name.to_string(), d, secs, peak_mb: status_mb("VmHWM:") - rss0, cell })
}

/// Errors of a run against the reference: field (max, mean, away from vias/pads), loads, barrels.
fn compare(r: &Run, rf: &Run, near: &[(P, f64)]) -> Value {
    let drop_ref = rf.d.result.max_drop_mv.unwrap_or(0.0);
    let v0 = rf.d.result.voltage.abs() * if rf.d.result.ground { 0.0 } else { rf.d.result.voltage.signum() };
    let (mut emax, mut emax_at, mut emax_slot) = (0.0f64, [0.0, 0.0], 0usize);
    let (mut sum, mut area, mut far_max, mut far_sum, mut far_area, mut missing) = (0.0, 0.0, 0.0f64, 0.0, 0.0, 0.0);
    let (mut dmax_ref, mut dmax_var) = (0.0f64, 0.0f64);
    rf.d.field.for_each_leaf(|slot, p, h, v| {
        let Some(vr) = v else { return };
        let a = h * h;
        dmax_ref = dmax_ref.max((vr - v0).abs());
        let Some(vv) = r.d.field.at(slot, p) else {
            missing += a;
            return;
        };
        dmax_var = dmax_var.max((vv - v0).abs());
        let e = (vv - vr).abs();
        sum += e * a;
        area += a;
        if e > emax {
            (emax, emax_at, emax_slot) = (e, p, slot);
        }
        if !near.iter().any(|&(q, rr)| geom::len(geom::sub(p, q)) < rr) {
            far_max = far_max.max(e);
            far_sum += e * a;
            far_area += a;
        }
    });
    // loads: v_min (the reported voltage) and v_mean
    let mut load_err: Vec<Value> = Vec::new();
    let (mut lmax, mut lmax_rel) = (0.0f64, 0.0f64);
    for (lv, lr) in r.d.result.loads.iter().zip(&rf.d.result.loads) {
        let (Some(a), Some(b)) = (lv.v_min, lr.v_min) else { continue };
        let drop = (v0 - b).abs();
        let e = (a - b).abs();
        lmax = lmax.max(e);
        if drop > 1e-6 {
            lmax_rel = lmax_rel.max(e / drop);
        }
        load_err.push(json!({"load": lv.refdes, "drop_ref_mv": drop * 1e3, "err_uv": (a - b) * 1e6}));
    }
    // barrels: per contact (vertical, layer)
    let (mut bmax, mut bsum, mut bn) = (0.0f64, 0.0, 0usize);
    let mut bmax_at = None;
    for (cv, cr) in r.d.contacts.iter().zip(&rf.d.contacts) {
        if cv.0 != cr.0 || cv.1 != cr.1 {
            continue;
        }
        let (Some(a), Some(b)) = (r.d.field.node_v(cv.2), rf.d.field.node_v(cr.2)) else { continue };
        let e = (a - b).abs();
        bsum += e;
        bn += 1;
        if e > bmax {
            bmax = e;
            bmax_at = Some(rf.d.verticals[cv.0].0);
        }
    }
    let pct = |x: f64| if drop_ref > 0.0 { x * 1e3 / drop_ref * 100.0 } else { 0.0 };
    let dv = r.d.result.max_drop_mv.unwrap_or(0.0);
    json!({
        "variant": r.name, "cell_mm": r.cell, "unknowns": r.d.unknowns, "nnz": r.d.nnz, "copper_leaves": r.d.copper_leaves,
        "time_s": r.secs, "peak_mb": r.peak_mb,
        "max_drop_mv": dv, "drop_err_pct": if drop_ref > 0.0 { (dv - drop_ref) / drop_ref * 100.0 } else { 0.0 },
        "field_max_uv": emax * 1e6, "field_max_pct": pct(emax), "field_max_at": emax_at, "field_max_layer": rf.d.field.layers[emax_slot],
        "field_mean_pct": pct(if area > 0.0 { sum / area } else { 0.0 }),
        "far_max_pct": pct(far_max), "far_mean_pct": pct(if far_area > 0.0 { far_sum / far_area } else { 0.0 }),
        "missing_area_mm2": missing,
        "load_max_uv": lmax * 1e6, "load_max_pct": pct(lmax), "load_max_rel_pct": lmax_rel * 100.0,
        "barrel_max_uv": bmax * 1e6, "barrel_max_pct": pct(bmax), "barrel_mean_pct": pct(if bn > 0 { bsum / bn as f64 } else { 0.0 }),
        "barrel_max_at": bmax_at, "loads": load_err,
        "unconnected_loads": r.d.result.loads.iter().filter(|l| !l.connected && l.current != 0.0).count(),
        "warnings": r.d.result.warnings,
    })
}

/// Error maps of one rail on one layer slot: the base grid in blocks of `ds` × `ds` cells (largest |error|
/// of the reference leaves in a block, µV; the reference drop in mV) and, in `window`, every reference leaf
/// (error, µV); f32 little-endian, row-major, with a JSON index.
fn dump(dir: &PathBuf, runs: &[Run], rf: &Run, slot: usize, window: Option<[f64; 4]>) -> Value {
    std::fs::create_dir_all(dir).unwrap_or_else(|e| die(&e.to_string()));
    let (o, cell, nx, ny) = rf.d.field.base();
    let ds = nx.max(ny).div_ceil(1200).max(1);
    let (mx, my) = (nx.div_ceil(ds), ny.div_ceil(ds));
    let bc = cell * ds as f64;
    let v0 = if rf.d.result.ground { 0.0 } else { rf.d.result.voltage };
    let write = |name: &str, data: &[f32]| {
        let bytes: Vec<u8> = data.iter().flat_map(|x| x.to_le_bytes()).collect();
        std::fs::write(dir.join(name), bytes).unwrap_or_else(|e| die(&e.to_string()));
    };
    let mut drop = vec![f32::NAN; mx * my];
    rf.d.field.for_each_leaf(|s, p, _, v| {
        let Some(v) = v else { return };
        if s != slot {
            return;
        }
        let k = ((p[1] - o[1]) / bc) as usize * mx + ((p[0] - o[0]) / bc) as usize;
        let d = ((v0 - v).abs() * 1e3) as f32;
        if drop[k].is_nan() || drop[k] < d {
            drop[k] = d;
        }
    });
    write("reference_drop_mv.f32", &drop);
    let hr = rf.cell / (1usize << rf.d.field.lvl.iter().copied().max().unwrap_or(0)) as f64;
    let mut maps = vec![];
    for r in runs {
        let mut m = vec![f32::NAN; mx * my];
        let wn = window.map(|w| (((w[2] - w[0]) / hr).ceil() as usize, ((w[3] - w[1]) / hr).ceil() as usize));
        let mut wv = vec![f32::NAN; wn.map(|x| x.0 * x.1).unwrap_or(0)];
        rf.d.field.for_each_leaf(|s, p, _, v| {
            if s != slot {
                return;
            }
            let (Some(vr), Some(vv)) = (v, r.d.field.at(s, p)) else { return };
            let e = ((vv - vr) * 1e6) as f32;
            let k = ((p[1] - o[1]) / bc) as usize * mx + ((p[0] - o[0]) / bc) as usize;
            if m[k].is_nan() || m[k].abs() < e.abs() {
                m[k] = e;
            }
            if let (Some(w), Some((wx, wy))) = (window, wn) {
                if p[0] >= w[0] && p[0] < w[2] && p[1] >= w[1] && p[1] < w[3] {
                    let (a, b) = (((p[0] - w[0]) / hr) as usize, ((p[1] - w[1]) / hr) as usize);
                    if a < wx && b < wy {
                        wv[b * wx + a] = e;
                    }
                }
            }
        });
        write(&format!("{}_err_uv.f32", r.name), &m);
        let mut e = json!({"variant": r.name, "file": format!("{}_err_uv.f32", r.name),
            "drop_err_pct": (r.d.result.max_drop_mv.unwrap_or(0.0) / rf.d.result.max_drop_mv.unwrap_or(1.0) - 1.0) * 100.0,
            "unknowns": r.d.unknowns});
        if let (Some(w), Some((wx, wy))) = (window, wn) {
            write(&format!("{}_win_err_uv.f32", r.name), &wv);
            e["window"] = json!({"file": format!("{}_win_err_uv.f32", r.name), "x0": w[0], "y0": w[1], "h": hr, "nx": wx, "ny": wy});
        }
        maps.push(e);
    }
    let index = json!({"rail": rf.d.result.net, "x0": o[0], "y0": o[1], "cell": bc, "nx": mx, "ny": my,
        "layer": rf.d.field.layers[slot], "drop_ref_mv": rf.d.result.max_drop_mv, "maps": maps,
        "verticals": rf.d.verticals.iter().map(|v| v.0).collect::<Vec<_>>()});
    std::fs::write(dir.join("index.json"), serde_json::to_string_pretty(&index).unwrap()).unwrap();
    index
}

/// Synthetic rail setups for a board without a power configuration (`--auto N`): the N supply nets with
/// the largest zone area; the source is the inductor (else connector, else other part) with the most pads on
/// the net, the loads are the ICs (U…, IC…) on it, 1 A in all, split by their pads on the net.
fn auto_setups(board: &kisi_core::board::Board, n: usize) -> Vec<RailSetup> {
    use kisi_core::pi::{LoadSpec, SourceSpec};
    let mut out = vec![];
    for pn in kisi_core::pi::power_nets(board).into_iter().filter(|p| !p.ground && p.zone_area > 50.0) {
        let mut pads: Vec<(String, usize)> = vec![];
        for pad in board.pads.iter().filter(|q| q.net == pn.net) {
            let r = board.footprints[pad.fp as usize].reference.clone();
            match pads.iter_mut().find(|x| x.0 == r) {
                Some(x) => x.1 += 1,
                None => pads.push((r, 1)),
            }
        }
        let prefix = |r: &str, ps: &[&str]| {
            ps.iter().any(|p| r.starts_with(p) && r[p.len()..].chars().next().is_some_and(|c| c.is_ascii_digit()))
        };
        let rank = |r: &str| {
            if prefix(r, &["L"]) {
                0
            } else if prefix(r, &["J", "P", "CN", "X", "F"]) {
                1
            } else if prefix(r, &["U", "IC", "Q"]) {
                2
            } else {
                9
            }
        };
        let Some(src) = pads.iter().filter(|x| rank(&x.0) < 9).min_by_key(|x| (rank(&x.0), usize::MAX - x.1)).cloned()
        else {
            continue;
        };
        let loads: Vec<&(String, usize)> = pads.iter().filter(|x| x.0 != src.0 && prefix(&x.0, &["U", "IC"])).collect();
        let total: usize = loads.iter().map(|x| x.1).sum();
        if loads.is_empty() {
            continue;
        }
        out.push(RailSetup {
            net: pn.name.clone(),
            voltage: pn.voltage.filter(|v| *v != 0.0).unwrap_or(1.0),
            sources: vec![SourceSpec { refdes: src.0.clone(), pads: vec![] }],
            loads: loads
                .iter()
                .map(|x| LoadSpec {
                    refdes: x.0.clone(),
                    current: x.1 as f64 / total as f64,
                    pads: vec![],
                    origin: "auto".into(),
                })
                .collect(),
            ground: false,
        });
        if out.len() >= n {
            break;
        }
    }
    out
}

pub fn run_cmd(args: &[String]) {
    let pcb = PathBuf::from(&args[0]);
    let opt = |k: &str| args.iter().position(|a| a == k).and_then(|i| args.get(i + 1)).cloned();
    let cfg = if args.iter().any(|a| a == "--no-config") {
        None
    } else {
        opt("--config").map(PathBuf::from).or_else(|| Some(service::config_path_for(&pcb)).filter(|p| p.is_file()))
    };
    let p = Project::load_with(&pcb, None, None, cfg.as_deref(), None).unwrap_or_else(|e| die(&e));
    let setups: Vec<RailSetup> = match opt("--auto").and_then(|x| x.parse::<usize>().ok()) {
        Some(n) => auto_setups(&p.board, n),
        None => service::power_run(&p, None).unwrap_or_else(|e| die(&e)).solved.into_iter().map(|x| x.0).collect(),
    };
    if let Some(f) = opt("--setups") {
        // the rail setups, e.g. for a timing of the WASM engine (`kisi_pi_solve`)
        std::fs::write(&f, serde_json::to_string_pretty(&setups).unwrap()).unwrap_or_else(|e| die(&e.to_string()));
    }
    let want: Option<Vec<String>> = opt("--rails").map(|s| s.split(',').map(str::to_string).collect());
    let variants: Vec<String> = opt("--variants")
        .unwrap_or(
            "uniform,uniform_x2,peaceman,cut4,lines,lines_pm,lines_pm_cut4,lines_pm_x2,qt2,qt2_lines_pm,uniform_eq,lines_pm_eq"
                .into(),
        )
        .split(',')
        .map(str::to_string)
        .collect();
    let ref_level: u8 = opt("--ref-level").and_then(|s| s.parse().ok()).unwrap_or(3);
    let max_ref: usize = opt("--max-ref").and_then(|s| s.parse().ok()).unwrap_or(3_000_000);
    let dump_rail = opt("--dump-rail");
    let window: Option<[f64; 4]> = opt("--window").map(|s| {
        let v: Vec<f64> = s.split(',').filter_map(|x| x.parse().ok()).collect();
        [v[0], v[1], v[2], v[3]]
    });
    let mut out = vec![];
    for s in &setups {
        if let Some(w) = &want {
            if !w.iter().any(|x| x == &s.net) {
                continue;
            }
        }
        let board = &p.board;
        let u = match run(board, s, "uniform", &Mesh::default(), None) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("{}: {e}", s.net);
                continue;
            }
        };
        // `pi_refine` without refinement is `solve_rail`
        let same = kisi_core::pi::solve_rail(board, s, &PiParams::default()).ok().map(|b| b.max_drop_mv)
            == Some(u.d.result.max_drop_mv);
        // reference: as fine as fits
        let mut lv = ref_level;
        while lv > 1 && u.d.copper_leaves << (2 * lv) > max_ref {
            lv -= 1;
        }
        let rm = Mesh {
            uniform: lv,
            contact: Contact::Disc,
            disc_grow: pi_refine::DISC_GROW,
            track_lines: REF_LINES,
            ..Mesh::default()
        };
        let rf = match run(board, s, &format!("ref_l{lv}"), &rm, None) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("{}: reference: {e}", s.net);
                continue;
            }
        };
        let rf_lo = run(board, s, &format!("ref_l{}", lv - 1), &Mesh { uniform: lv - 1, ..rm.clone() }, None).ok();
        let mut near: Vec<(P, f64)> = rf.d.verticals.iter().map(|&(at, d)| (at, (2.0 * d).max(0.5))).collect();
        for pad in board.pads.iter().filter(|q| {
            board.net_name(q.net) == s.net
                && s.loads
                    .iter()
                    .map(|l| &l.refdes)
                    .chain(s.sources.iter().map(|x| &x.refdes))
                    .any(|r| *r == board.footprints[q.fp as usize].reference)
        }) {
            near.push((pad.at, 0.5 * pad.size[0].max(pad.size[1]) + 0.25));
        }
        let mut runs: Vec<Run> = vec![];
        let mut qt_unknowns = None;
        for name in &variants {
            let r = if name.ends_with("_eq") {
                // a uniform raster with as many unknowns as the quadtree `qt2`
                let Some(nq) = qt_unknowns else { continue };
                let c = u.cell * (u.d.unknowns as f64 / nq as f64).sqrt();
                let m =
                    if name == "uniform_eq" { Mesh::default() } else { variant(name.trim_end_matches("_eq")).unwrap() };
                // the line threshold in cells of the finer raster: the same widths as on the base raster
                let m = Mesh { track_lines: m.track_lines * u.cell / c, ..m };
                run(board, s, name, &m, Some(c))
            } else if name == "uniform" {
                continue;
            } else {
                let Some(m) = variant(name) else { die(&format!("unknown variant {name}")) };
                run(board, s, name, &m, None)
            };
            match r {
                Ok(r) => {
                    if r.name == "qt2" || (r.name == "qt2_lines_pm" && qt_unknowns.is_none()) {
                        qt_unknowns = Some(r.d.unknowns);
                    }
                    runs.push(r);
                }
                Err(e) => eprintln!("{} {name}: {e}", s.net),
            }
        }
        runs.insert(0, u);
        let mut rows: Vec<Value> = runs.iter().map(|r| compare(r, &rf, &near)).collect();
        if let Some(lo) = &rf_lo {
            rows.push(compare(lo, &rf, &near));
        }
        let mut rail = json!({"rail": s.net, "ground": s.ground, "same_as_solve_rail": same, "reference": rf.name,
            "ref_unknowns": rf.d.unknowns, "ref_time_s": rf.secs, "ref_peak_mb": rf.peak_mb,
            "ref_drop_mv": rf.d.result.max_drop_mv, "rows": rows});
        if dump_rail.as_deref() == Some(s.net.as_str()) {
            if let Some(dir) = opt("--dump") {
                // the layer of the largest error of today's raster unless `--dump-layer` (copper index)
                let worst = rows[0]["field_max_layer"].as_u64().unwrap_or(0) as usize;
                let l = opt("--dump-layer").and_then(|x| x.parse().ok()).unwrap_or(worst);
                let slot = rf.d.field.layers.iter().position(|&x| x == l).unwrap_or(0);
                let d = dump(&PathBuf::from(&dir), &runs, &rf, slot, window);
                rail["dump"] = json!({"dir": dir, "layer": d["layer"]});
            }
        }
        // one line per variant
        eprintln!(
            "{}  ref {} ({} unknowns, {:.1} s): drop {:.4} mV",
            s.net,
            rf.name,
            rf.d.unknowns,
            rf.secs,
            rf.d.result.max_drop_mv.unwrap_or(0.0)
        );
        for r in rail["rows"].as_array().unwrap() {
            eprintln!(
                "  {:14} cell {:.3} N {:8} t {:6.2}s mem {:6.0}MB  drop {:+6.2}%  field max {:6.2}% mean {:5.2}% far {:5.2}%  load {:6.2}% (rel {:5.1}%)  barrel {:6.2}%{}",
                r["variant"].as_str().unwrap(),
                r["cell_mm"].as_f64().unwrap(),
                r["unknowns"].as_u64().unwrap(),
                r["time_s"].as_f64().unwrap(),
                r["peak_mb"].as_f64().unwrap(),
                r["drop_err_pct"].as_f64().unwrap(),
                r["field_max_pct"].as_f64().unwrap(),
                r["field_mean_pct"].as_f64().unwrap(),
                r["far_max_pct"].as_f64().unwrap(),
                r["load_max_pct"].as_f64().unwrap(),
                r["load_max_rel_pct"].as_f64().unwrap(),
                r["barrel_max_pct"].as_f64().unwrap(),
                match r["unconnected_loads"].as_u64() {
                    Some(0) | None => String::new(),
                    Some(n) => format!("  [{n} loads not connected]"),
                }
            );
        }
        out.push(rail);
    }
    let res = json!({"board": pcb.file_name().map(|f| f.to_string_lossy().to_string()), "rails": out});
    if let Some(f) = opt("--json") {
        std::fs::write(&f, serde_json::to_string_pretty(&res).unwrap()).unwrap_or_else(|e| die(&e.to_string()));
    }
}

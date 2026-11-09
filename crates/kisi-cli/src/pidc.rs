//! `kisi-validate pi-dc <board.kicad_pcb> [--config F | --no-config] [--auto N] [--rails A,B] [--ref FILE]
//!  [--cell MM] [--fine N] [--max-fine N] [--repeat N] [--json FILE]`
//!
//! Accuracy and run time of the DC IR drop (`pi::solve_rail`, the default raster) on a board's rails
//! (docs/research/pi-quadtree.md; `--cell`: a cell size instead of the automatic one). Per rail: the largest drop, the unknowns, the run time (best of `--repeat`,
//! default 3), loads left unconnected, and the error of the largest drop against a reference: `--ref FILE` – the
//! stored references of the quadtree experiment (`results-*.json` of `kisi-validate pi-refine` on branch
//! `exp/pi-quadtree`: the base grid refined uniformly, barrels as equipotential discs, line tracks);
//! `--fine N` – a convergence check: the same solver on
//! the cell / 2^N raster (as fits in `--max-fine` unknowns, default 2 M), tracks narrower than 4 cells of
//! the default raster as line conductors. `--auto N`: synthetic setups for the
//! N largest supply pours of a board without a power configuration (the source the inductor – else connector,
//! else IC – with the most pads on the net, the ICs on it as loads, 1 A split by pads).

use kisi_core::pi::{self, LoadSpec, PiParams, RailSetup, SourceSpec};
use kisi_core::service::{self, Project};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::time::Instant;

fn die(msg: &str) -> ! {
    eprintln!("pi-dc: {msg}");
    std::process::exit(2);
}

/// Synthetic rail setups (`--auto N`), as in the experiment.
fn auto_setups(board: &kisi_core::board::Board, n: usize) -> Vec<RailSetup> {
    let mut out = vec![];
    for pn in pi::power_nets(board).into_iter().filter(|p| !p.ground && p.zone_area > 50.0) {
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
        let rank = |r: &str| match () {
            _ if prefix(r, &["L"]) => 0,
            _ if prefix(r, &["J", "P", "CN", "X", "F"]) => 1,
            _ if prefix(r, &["U", "IC", "Q"]) => 2,
            _ => 9,
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

/// The reference drop (mV) of a rail in a `pi-refine` results file.
fn stored_ref(file: &Value, rail: &str) -> Option<f64> {
    file["rails"].as_array()?.iter().find(|r| r["rail"] == rail)?["ref_drop_mv"].as_f64()
}

pub fn run(args: &[String]) {
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
    let want: Option<Vec<String>> = opt("--rails").map(|s| s.split(',').map(str::to_string).collect());
    let stored: Option<Value> = opt("--ref").map(|f| {
        serde_json::from_str(&std::fs::read_to_string(&f).unwrap_or_else(|e| die(&format!("{f}: {e}"))))
            .unwrap_or_else(|e| die(&format!("{f}: {e}")))
    });
    let fine: u32 = opt("--fine").and_then(|s| s.parse().ok()).unwrap_or(0);
    let max_fine: usize = opt("--max-fine").and_then(|s| s.parse().ok()).unwrap_or(2_000_000);
    let repeat: usize = opt("--repeat").and_then(|s| s.parse().ok()).unwrap_or(3).max(1);
    let mut out = vec![];
    for s in setups.iter().filter(|s| want.as_ref().is_none_or(|w| w.contains(&s.net))) {
        let board = &p.board;
        let params = PiParams { cell: opt("--cell").and_then(|c| c.parse().ok()), ..Default::default() };
        let mut best = f64::INFINITY;
        let mut res = None;
        for _ in 0..repeat {
            let t0 = Instant::now();
            match pi::solve_rail(board, s, &params) {
                Ok(r) => {
                    best = best.min(t0.elapsed().as_secs_f64());
                    res = Some(r);
                }
                Err(e) => {
                    eprintln!("{}: {e}", s.net);
                    break;
                }
            }
        }
        let Some(r) = res else { continue };
        let open = r.loads.iter().filter(|l| !l.connected && l.current != 0.0).count();
        let mut row = json!({"rail": s.net, "cell_mm": r.cell, "nodes": r.nodes, "time_s": best,
            "max_drop_mv": r.max_drop_mv, "unconnected_loads": open, "warnings": r.warnings});
        let mut line = format!(
            "{:12} cell {:.3} nodes {:7} t {:6.3}s drop {:9.4} mV",
            s.net,
            r.cell,
            r.nodes,
            best,
            r.max_drop_mv.unwrap_or(f64::NAN)
        );
        if open > 0 {
            line += &format!(" [{open} loads open]");
        }
        if let Some(d) = stored.as_ref().and_then(|f| stored_ref(f, &s.net)) {
            let e = (r.max_drop_mv.unwrap_or(0.0) - d) / d * 100.0;
            row["ref_drop_mv"] = json!(d);
            row["drop_err_pct"] = json!(e);
            line += &format!("  ref {d:9.4} mV  error {e:+6.2} %");
        }
        if fine > 0 {
            // the same solver on a finer raster: the cell halved as often as fits
            let mut k = fine;
            while k > 1 && r.nodes << (2 * k) > max_fine {
                k -= 1;
            }
            // tracks narrower than 4 cells of the default raster as line conductors (wider ones have ≥ 8 cells
            // across on the finer raster)
            let pf = PiParams {
                cell: Some(r.cell / (1u32 << k) as f64),
                max_cells: 1 << 40,
                line_width: Some(4.0 * r.cell),
                ..Default::default()
            };
            let t0 = Instant::now();
            match pi::solve_rail(board, s, &pf) {
                Ok(f) => {
                    let df = f.max_drop_mv.unwrap_or(f64::NAN);
                    row["fine"] =
                        json!({"level": k, "nodes": f.nodes, "time_s": t0.elapsed().as_secs_f64(), "max_drop_mv": df});
                    line += &format!("  cell/{}: {df:9.4} mV", 1u32 << k);
                }
                Err(e) => eprintln!("{} fine: {e}", s.net),
            }
        }
        eprintln!("{line}");
        out.push(row);
    }
    let res = json!({"board": pcb.file_name().map(|f| f.to_string_lossy().to_string()), "rails": out});
    if let Some(f) = opt("--json") {
        std::fs::write(&f, serde_json::to_string_pretty(&res).unwrap()).unwrap_or_else(|e| die(&e.to_string()));
    }
}

//! Passivity and causality of kisi's channel models on the example boards' lanes.
//!
//! Every lane of ThunderScope and LibreVNA (with `web/samples/{ts,vna}.kisi.json`), as the channel
//! check computes it (formula impedances, default `RfParams`: 25 GHz, 500 points): the board's
//! S-parameters – a pair's 4-port, a single line's 2-port – must be passive (largest singular value
//! of S ≤ 1 at every frequency) and the step responses of the transfer (S21, a pair's Sdd21) and of
//! the reflection (S11, Sdd11) causal: nothing before the signal can arrive (`rf::timedomain`'s
//! pulse with a Gaussian window that leaves 1e-6 at the top of the band). The example boards are not in the repository
//! (`tools/fetch_examples.sh`); without them the test reports that and passes.
//! `KISI_CAUSALITY_REPORT=1` prints the numbers per lane.

use kisi_core::rf::{mixed_of, passivity, s_of, Cx, RfParams, Step};
use kisi_core::service::Project;
use kisi_core::topology;
use std::collections::HashSet;
use std::path::PathBuf;

/// largest singular value allowed (numerical noise of the cascade)
const PASSIVE: f64 = 1.0 + 1e-9;
/// largest pulse response before the signal can arrive: the transfer as a share of its peak, the
/// reflection as a share of the incident pulse (before the causal dielectric, roughness and
/// dispersion models of 2026-10: up to 7e-4 and 9e-3 on these lanes)
const PRE_T: f64 = 3e-4;
const PRE_R: f64 = 1e-3;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

struct Worst {
    sigma: f64,
    pre_t: f64,
    pre_r: f64,
}

fn check_board(stem: &str, key: &str, report: bool) -> Option<(Worst, Vec<String>)> {
    let dir = std::env::var("KISI_EXAMPLES").map(PathBuf::from).unwrap_or_else(|_| root().join("examples"));
    let pcb = dir.join(format!("{stem}.kicad_pcb"));
    if !pcb.is_file() {
        eprintln!("{}: not found, skipped (tools/fetch_examples.sh)", pcb.display());
        return None;
    }
    let cfg = root().join(format!("web/samples/{key}.kisi.json"));
    let p = Project::load_with(&pcb, None, None, Some(&cfg), None).expect("board loads");
    let rp = RfParams::default();
    let cd = topology::ComponentData::builtin();
    // a Gaussian window that leaves 1e-6 of the spectrum at the top of the band: its own ringing
    // stays far below the limit
    let rise = Step::window_rise(rp.fmax) * (6.0f64 / 2.0).sqrt();
    let mut worst = Worst { sigma: 0.0, pre_t: 0.0, pre_r: 0.0 };
    let mut fails = Vec::new();
    for bus in p.buses() {
        for lane in &bus.lanes {
            let target = p.target(lane.name_net);
            let rev = lane.rx_end == 0;
            let Ok(ch) = topology::lane_channel(&p.board, &p.an, lane, &p.params.field, target, &rp, rev, false, &cd)
            else {
                continue;
            };
            let freq = ch.freq.clone();
            // S per frequency (row-major), the transfer and the reflection
            let (s, n, h21, h11, kind): (Vec<Vec<Cx>>, usize, Vec<Cx>, Vec<Cx>, &str) =
                if ch.diff && !lane.partner_hops.is_empty() {
                    match topology::lane_pair(
                        &p.board,
                        &p.an,
                        lane,
                        &p.params.field,
                        target,
                        &rp,
                        rev,
                        false,
                        &HashSet::new(),
                        &cd,
                    ) {
                        Ok(pn) => {
                            let s4: Vec<_> = pn.t.iter().map(|t| s_of(t, pn.z_ref)).collect();
                            let mm: Vec<_> = s4.iter().map(mixed_of).collect();
                            (
                                s4.iter().map(|m| m.iter().flatten().copied().collect()).collect(),
                                4,
                                mm.iter().map(|m| m[1][0]).collect(),
                                mm.iter().map(|m| m[0][0]).collect(),
                                "4-port",
                            )
                        }
                        Err(_) => continue,
                    }
                } else {
                    (
                        (0..freq.len()).map(|k| vec![ch.s11[k], ch.s21[k], ch.s21[k], ch.s22[k]]).collect(),
                        2,
                        ch.s21.clone(),
                        ch.s11.clone(),
                        "2-port",
                    )
                };
            let (sig, k) = passivity(&s, n);
            // the transfer: nothing before 80 % of the static delay (less the edge); the reflection:
            // nothing before the edge reaches port 1
            let (pre_t, tail_t) = precursor(&freq, &h21, rise, 0.8 * ch.delay_ps * 1e-12 - 2.0 * rise, true);
            let (pre_r, tail_r) = precursor(&freq, &h11, rise, -2.0 * rise, false);
            // a response still ringing after 0.6/df (filters with resonances below the frequency
            // step) wraps around the sweep's period: no judgement of its causality
            let settled = tail_t < PRE_T / 3.0 && tail_r < PRE_R / 3.0;
            if report {
                eprintln!("{key} {:<28} {kind} {:>6.1} mm  σmax {:.7} at {:5.2} GHz  precursor S21 {:.2e} S11 {:.2e} tail {:.1e} {:.1e}{}", lane.name, ch.length_mm, sig, freq[k] / 1e9, pre_t, pre_r, tail_t, tail_r, if settled { "" } else { " (not settled within the sweep: not judged)" });
            }
            if sig > PASSIVE {
                fails.push(format!("{key} {}: not passive, σmax {sig:.7} at {:.2} GHz", lane.name, freq[k] / 1e9));
            }
            if settled && (pre_t > PRE_T || pre_r > PRE_R) {
                fails.push(format!(
                    "{key} {}: response before the signal arrives {:.2e} (S21) / {:.2e} (S11)",
                    lane.name, pre_t, pre_r
                ));
            }
            worst.sigma = worst.sigma.max(sig);
            if settled {
                worst.pre_t = worst.pre_t.max(pre_t);
                worst.pre_r = worst.pre_r.max(pre_r);
            }
        }
    }
    Some((worst, fails))
}

/// The pulse response (a step and its copy two window rises later, `rf::timedomain::Step` with a
/// Gaussian window) before `t_arrive`, as a share of
/// its peak after it (`relative`; else of the incident pulse – a reflection): a pulse rather than
/// the step itself, so that AC-coupled lanes – a step that
/// decays over microseconds, aliased by the frequency step – are judged like the others.
fn precursor(freq: &[f64], h: &[Cx], rise: f64, t_arrive: f64, relative: bool) -> (f64, f64) {
    let st = Step::new(freq, h, Some(0.0), rise);
    let p = |t: f64| st.at(t) - st.at(t - 2.0 * rise);
    let period = 1.0 / freq[0];
    let n = 2000;
    let (mut pre, mut peak, mut tail) = (0.0f64, 0.0f64, 0.0f64);
    // one period of the sweep, from −0.3/df: before the arrival, after it, and the last 0.1/df of
    // it (where a response longer than the sweep allows wraps around)
    for i in 0..=n {
        let t = -0.3 * period + period * i as f64 / n as f64;
        let v = p(t).abs();
        if t < t_arrive {
            pre = pre.max(v);
        } else {
            peak = peak.max(v);
        }
        if t > 0.6 * period {
            tail = tail.max(v);
        }
    }
    let norm = if relative { peak.max(1e-9) } else { 1.0 };
    (pre / norm, tail / norm)
}

#[test]
fn example_lanes_passive_and_causal() {
    let report = std::env::var("KISI_CAUSALITY_REPORT").is_ok();
    let mut fails = Vec::new();
    for (stem, key) in [("Thunderscope_Rev5.3", "ts"), ("VNA", "vna")] {
        if let Some((w, f)) = check_board(stem, key, report) {
            eprintln!(
                "{key}: largest singular value {:.7}, largest precursor {:.2e} (transfer) / {:.2e} (reflection)",
                w.sigma, w.pre_t, w.pre_r
            );
            fails.extend(f);
        }
    }
    assert!(fails.is_empty(), "{}", fails.join("\n"));
}

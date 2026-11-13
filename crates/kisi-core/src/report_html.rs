//! The printable report: one to two pages of results from the report JSON (`service::report`) –
//! `kisi report --html FILE` / `--csv DIR` and the viewer's Export → Report (WASM `kisi_report_html`).
//!
//! Rendered from that JSON alone, so the CLIs and the viewer print the same page. Verdicts are the
//! report's own (`status` of the channel rows and the impedance classes from `profile::judge`, the
//! temperature's `status`); this module only counts, sorts and formats them. Maps, curves, per-net
//! details and via lists stay in the viewer and the JSON. The page is self-contained (no scripts, fonts
//! or images), light, for A4 and Letter; tables break across pages with their header repeated.

use serde_json::Value;

/// A table of the report: also written as CSV (`--csv`). `rows`: cells and the row's verdict class
/// (`ok`, `warn`, `fail` or empty).
pub struct Table {
    pub id: &'static str,
    pub title: String,
    pub head: Vec<String>,
    pub rows: Vec<(Vec<String>, &'static str)>,
    pub note: String,
}

/// The verdict class of a status word of the report.
#[inline(never)]
fn class(s: &str) -> &'static str {
    match s {
        "green" | "ok" => "ok",
        "yellow" | "near" => "warn",
        "red" | "over" => "fail",
        _ => "",
    }
}

#[inline(never)]
fn word(c: &str) -> &'static str {
    match c {
        "ok" => "ok",
        "warn" => "warning",
        "fail" => "fail",
        _ => "",
    }
}

/// A number with `d` decimals (dot, no grouping); empty for no value.
#[inline(never)]
fn num(v: &Value, d: usize) -> String {
    v.as_f64().map(|x| fx(x, d)).unwrap_or_default()
}

#[inline(never)]
fn fx(x: f64, d: usize) -> String {
    let s = format!("{x:.d$}");
    if s.starts_with('-') && s[1..].chars().all(|c| c == '0' || c == '.') {
        s[1..].to_string()
    } else {
        s
    }
}

#[inline(never)]
/// A value as text; KiCad's escaped slash in net names (`{slash}`) as `/`, as the viewer shows it.
fn s(v: &Value) -> String {
    match v {
        Value::String(t) => t.replace("{slash}", "/"),
        Value::Null => String::new(),
        x => x.to_string(),
    }
}

/// A channel criterion: label (with its unit), the factor to that unit, decimals.
#[inline(never)]
fn crit(key: &str) -> (&'static str, f64, usize) {
    match key {
        "z" => ("Z (Ω)", 1.0, 1),
        "eye_h" => ("eye height (%)", 100.0, 0),
        "eye_w" => ("eye width (UI)", 1.0, 2),
        "il" => ("loss at Nyquist (dB)", 1.0, 1),
        "rl" => ("return loss (dB)", 1.0, 1),
        "skew" => ("skew (ps)", 1.0, 1),
        "stub" => ("stub resonance (GHz)", 1.0, 1),
        _ => ("error", 1.0, 1),
    }
}

/// A row's worst criterion – the worst status, then the least margin relative to its limit –
/// with that margin: (criterion, relative margin).
#[inline(never)]
fn worst_criterion(row: &Value) -> Option<(&Value, f64)> {
    let rel = |c: &Value| {
        let v = g(c, "value").as_f64()?;
        let lo = g(c, "lo").as_f64().map(|l| (v - l) / l.abs().max(1e-9));
        let hi = g(c, "hi").as_f64().map(|h| (h - v) / h.abs().max(1e-9));
        match (lo, hi) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    };
    let mut best: Option<(&Value, Key)> = None;
    for c in g(row, "criteria").as_array()? {
        let k = (rank(g(c, "status")), if g(c, "key") == "error" { -1.0 } else { rel(c).unwrap_or(f64::INFINITY) });
        if best.is_none_or(|b| less(k, b.1)) {
            best = Some((c, k));
        }
    }
    best.map(|(c, k)| (c, k.1))
}

/// Criterion cells: label, value, limit, margin (in the criterion's unit).
#[inline(never)]
fn crit_cells(c: &Value) -> [String; 4] {
    let (label, f, d) = crit(g(c, "key").as_str().unwrap_or(""));
    if g(c, "key") == "error" {
        return [label.into(), String::new(), String::new(), String::new()];
    }
    let g = |k: &str| c[k].as_f64().map(|x| x * f);
    let v = g("value");
    let (lo, hi) = (g("lo"), g("hi"));
    let limit = match (lo, hi) {
        (Some(a), Some(b)) => format!("{} … {}", fx(a, d), fx(b, d)),
        (Some(a), None) => format!("≥ {}", fx(a, d)),
        (None, Some(b)) => format!("≤ {}", fx(b, d)),
        _ => String::new(),
    };
    let m = v.and_then(|v| match (lo, hi) {
        (Some(a), Some(b)) => Some((v - a).min(b - v)),
        (Some(a), None) => Some(v - a),
        (None, Some(b)) => Some(b - v),
        _ => None,
    });
    [label.into(), v.map(|x| fx(x, d)).unwrap_or_default(), limit, m.map(|x| fx(x, d)).unwrap_or_default()]
}

/// A channel row's name: the lane, or the net without devices.
#[inline(never)]
fn lane(r: &Value) -> String {
    s(if g(r, "lane").is_null() { g(r, "net") } else { g(r, "lane") })
}

/// An ordering key: a rank (fail, warning, ok), then a margin; smaller first.
type Key = (u8, f64);
type Line = (Key, Vec<String>, &'static str);

#[inline(never)]
fn less(a: Key, b: Key) -> bool {
    a.0 < b.0 || (a.0 == b.0 && a.1 < b.1)
}

/// Sort by key, stable (insertion: the lists are short, and one function keeps the WebAssembly small).
#[inline(never)]
fn sort_lines(v: &mut [Line]) {
    for i in 1..v.len() {
        let mut j = i;
        while j > 0 && less(v[j].0, v[j - 1].0) {
            v.swap(j, j - 1);
            j -= 1;
        }
    }
}

#[inline(never)]
fn rank(st: &Value) -> u8 {
    match class(st.as_str().unwrap_or("")) {
        "fail" => 0,
        "warn" => 1,
        _ => 2,
    }
}

#[inline(never)]
fn heads(h: &[&str]) -> Vec<String> {
    h.iter().map(|x| x.to_string()).collect()
}

/// `v[k]`, as one function (indexing inlined at every place makes the WebAssembly much larger).
#[inline(never)]
fn g<'a>(v: &'a Value, k: &str) -> &'a Value {
    v.get(k).unwrap_or(&Value::Null)
}

#[inline(never)]
fn arr(v: &Value) -> &[Value] {
    v.as_array().map(|a| a.as_slice()).unwrap_or(&[])
}

/// The report's tables in their order.
#[inline(never)]
pub fn tables(rep: &Value) -> Vec<Table> {
    let mut out = Vec::new();
    // impedance per net class
    let cov = g(g(g(rep, "overview"), "field_solver"), "target_length_coverage_pct").as_f64().unwrap_or(0.0);
    let tol = g(g(rep, "options"), "tolerances") == true;
    let mut head = heads(&[
        "Class",
        "Target (Ω)",
        "Type",
        "Worst net",
        "Its mean Z (Ω)",
        "Deviation (%)",
        "Window (± %)",
        "Within ±5 % (% of length)",
    ]);
    if tol {
        head.extend(["Band RSS (± Ω)".into(), "Band".into()]);
    }
    head.push("Verdict".into());
    let rows = arr(g(rep, "impedance"))
        .iter()
        .map(|c| {
            let (t, z) = (g(c, "target_ohm").as_f64().unwrap_or(0.0), g(c, "z_ohm").as_f64());
            let cl = class(g(c, "status").as_str().unwrap_or(""));
            let mut r = vec![
                s(g(c, "class")),
                num(g(c, "target_ohm"), 0),
                if g(c, "diff") == true { "diff" } else { "SE" }.into(),
                s(g(c, "worst_net")),
                num(g(c, "z_ohm"), 1),
                z.filter(|_| t > 0.0).map(|z| fx(100.0 * (z / t - 1.0), 1)).unwrap_or_default(),
                num(g(c, "window_pct"), 0),
                g(c, "within_5pct").as_f64().map(|x| fx(100.0 * x, 0)).unwrap_or_default(),
            ];
            if tol {
                r.extend([num(g(c, "z_tol_rss_ohm"), 1), s(g(c, "z_tol_status")).replace('_', " ")]);
            }
            r.push(word(cl).into());
            (r, cl)
        })
        .collect();
    out.push(Table {
        id: "impedance",
        title: "Impedance per net class".into(),
        head,
        rows,
        note: format!(
            "Verdict: the worst net's mean impedance within the class window (its interface profile's tolerance, else the default), as the channel check judges it. Field solver for {} % of the routed length with a target, formula values (estimates) elsewhere.",
            fx(cov, 0)
        ),
    });
    // channel: a line per bus with its worst lane, the failing lanes one by one
    let ch = g(rep, "channels");
    let mut buses: Vec<(String, Vec<&Value>)> = Vec::new();
    for r in arr(g(ch, "rows")) {
        let k =
            g(r, "bus").as_str().map(str::to_string).unwrap_or_else(|| format!("{} (no devices)", s(g(r, "class"))));
        match buses.iter_mut().find(|b| b.0 == k) {
            Some(b) => b.1.push(r),
            None => buses.push((k, vec![r])),
        }
    }
    let lane_key = |r: &Value| (rank(g(r, "status")), worst_criterion(r).map(|x| x.1).unwrap_or(f64::INFINITY));
    let mut lines: Vec<Line> = buses
        .iter()
        .map(|(b, rs)| {
            let n =
                |c: &str| rs.iter().filter(|r| class(g(r, "status").as_str().unwrap_or("")) == c).count().to_string();
            let mut w: &Value = &Value::Null;
            for r in rs {
                if w.is_null() || less(lane_key(r), lane_key(w)) {
                    w = r;
                }
            }
            let cl = class(g(w, "status").as_str().unwrap_or(""));
            let cc = worst_criterion(w).map(|(c, _)| crit_cells(c)).unwrap_or_default();
            let mut r =
                vec![b.clone(), s(g(w, "profile")), rs.len().to_string(), n("ok"), n("warn"), n("fail"), lane(w)];
            r.extend(cc);
            r.push(word(cl).into());
            (lane_key(w), r, cl)
        })
        .collect();
    sort_lines(&mut lines);
    out.push(Table {
        id: "channel",
        title: "Channel: worst lane per interface".into(),
        head: heads(&[
            "Bus",
            "Profile",
            "Lanes",
            "ok",
            "warning",
            "fail",
            "Worst lane",
            "Its worst criterion",
            "Value",
            "Limit",
            "Margin",
            "Verdict",
        ]),
        rows: lines.into_iter().map(|(_, r, c)| (r, c)).collect(),
        note: s(g(ch, "not_checked_unused")),
    });
    let failing: Vec<(Vec<String>, &'static str)> = arr(g(ch, "rows"))
        .iter()
        .filter(|r| class(g(r, "status").as_str().unwrap_or("")) == "fail")
        .map(|r| {
            let mut v = vec![lane(r), s(g(r, "bus")), s(g(r, "profile"))];
            v.extend(worst_criterion(r).map(|(c, _)| crit_cells(c)).unwrap_or_default());
            (v, "fail")
        })
        .collect();
    out.push(Table {
        id: "failing_lanes",
        title: "Failing lanes".into(),
        head: heads(&["Lane", "Bus", "Profile", "Worst criterion", "Value", "Limit", "Margin"]),
        note: if failing.is_empty() { "No lane fails.".into() } else { String::new() },
        rows: failing,
    });
    // power: the largest drop per rail
    let pw = g(rep, "power");
    let (rows, note) = if g(pw, "error").is_null() {
        let mut rails: Vec<Line> = arr(g(pw, "rails"))
            .iter()
            .map(|r| {
                let gnd = g(r, "ground_return") == true;
                let d = if gnd { g(r, "max_offset_mv") } else { g(r, "max_drop_mv") };
                let v = g(r, "voltage").as_f64().filter(|v| v.abs() > 0.0);
                let pct = match (d.as_f64(), v) {
                    (Some(d), Some(v)) => d / 10.0 / v.abs(),
                    _ => 0.0,
                };
                let w = arr(g(r, "warnings")).iter().map(s).collect::<Vec<_>>().join("; ");
                let cl = if w.is_empty() { "" } else { "warn" };
                let rail = if gnd { format!("{} (ground return)", s(g(r, "rail"))) } else { s(g(r, "rail")) };
                let load = if gnd { g(r, "return_a") } else { g(r, "load_a") };
                (
                    (0, -pct),
                    vec![
                        rail,
                        num(g(r, "voltage"), 2),
                        num(load, 3),
                        num(d, 2),
                        if v.is_some() { fx(pct, 2) } else { String::new() },
                        num(g(r, "copper_loss_mw"), 1),
                        w,
                    ],
                    cl,
                )
            })
            .collect();
        sort_lines(&mut rails);
        let n = rails.len();
        let keep = 8.min(n);
        let rest = rails.get(keep).map(|r| -r.0 .1);
        let w: Vec<String> = arr(g(pw, "warnings")).iter().map(s).collect();
        let mut note = match rest {
            Some(p) => format!("{} more rails with a drop of {} % or less (in the JSON).", n - keep, fx(p, 2)),
            None => String::new(),
        };
        note.push_str(
            " kisi has no limit for the DC drop: compare it with the supply tolerance of the parts on each rail.",
        );
        if !w.is_empty() {
            note = format!("Warnings: {}. {}", w.join("; "), note.trim_start());
        }
        (rails.into_iter().take(keep).map(|(_, r, c)| (r, c)).collect(), note.trim().to_string())
    } else {
        (vec![], format!("not computed: {}", s(g(pw, "error"))))
    };
    out.push(Table {
        id: "power",
        title: "Power: largest DC drop per rail".into(),
        head: heads(&[
            "Rail",
            "Voltage (V)",
            "Load (A)",
            "Largest drop (mV)",
            "Drop (% of V)",
            "Copper loss (mW)",
            "Warnings",
        ]),
        rows,
        note,
    });
    // temperature: hot spots, junctions over or near their limit
    let th = g(rep, "thermal");
    let done = g(th, "error").is_null() && th.get("board").is_some();
    let spots = if done {
        arr(g(th, "hot_spots"))
            .iter()
            .take(5)
            .map(|h| (vec![s(g(h, "near")), s(g(h, "layer")), num(g(h, "t_c"), 1)], ""))
            .collect()
    } else {
        vec![]
    };
    out.push(Table {
        id: "hot_spots",
        title: "Temperature (estimate): hot spots of the board".into(),
        head: heads(&["Near", "Layer", "Temperature (°C)"]),
        rows: spots,
        note: if done {
            format!("Ambient {} °C, {}.", num(g(th, "ambient_c"), 0), cooling(g(th, "cooling")))
        } else {
            format!("not computed{}", g(th, "error").as_str().map(|e| format!(": {e}")).unwrap_or_default())
        },
    });
    let hot: Vec<(Vec<String>, &'static str)> = arr(g(th, "parts"))
        .iter()
        .filter(|p| matches!(class(g(p, "status").as_str().unwrap_or("")), "fail" | "warn"))
        .map(|p| {
            let cl = class(g(p, "status").as_str().unwrap_or(""));
            let src = format!(
                "{}{}",
                s(g(p, "data_source")),
                if g(p, "estimate") == true && !s(g(p, "data_source")).contains("estimate") {
                    " (estimate)"
                } else {
                    ""
                }
            );
            (
                vec![
                    s(g(p, "ref")),
                    s(g(p, "part")),
                    num(g(p, "tj_c"), 1),
                    num(g(p, "tj_max_c"), 0),
                    num(g(p, "margin_k"), 1),
                    src,
                    word(cl).into(),
                ],
                cl,
            )
        })
        .collect();
    let hp = g(g(th, "summary"), "hottest_part");
    out.push(Table {
        id: "junctions",
        title: "Temperature (estimate): junctions over or within 10 K of Tj max".into(),
        head: heads(&["Part", "Value", "Tj (°C)", "Tj max (°C)", "Margin (K)", "Thermal data", "Verdict"]),
        note: match (done, hot.is_empty()) {
            (false, _) => "not computed".into(),
            (true, true) => format!(
                "No part over or within 10 K of its Tj max; the hottest junction: {} {} at {} °C{}.",
                s(g(hp, "ref")),
                s(g(hp, "part")),
                num(g(hp, "tj_c"), 1),
                if g(hp, "estimate") == true { " (estimated package data)" } else { "" }
            ),
            _ => String::new(),
        },
        rows: hot,
    });
    // the open gaps
    out.push(Table {
        id: "gaps",
        title: "Open gaps".into(),
        head: heads(&["Gap", "Priority", "Affects", "Issue"]),
        rows: arr(g(g(rep, "gaps"), "gaps"))
            .iter()
            .map(|x| {
                (
                    vec![
                        s(g(x, "id")),
                        s(g(x, "priority")),
                        arr(g(x, "views")).iter().map(s).collect::<Vec<_>>().join(", "),
                        s(g(x, "issue")),
                    ],
                    "",
                )
            })
            .collect(),
        note: if arr(g(g(rep, "gaps"), "gaps")).is_empty() {
            "None.".into()
        } else {
            "Gap ids as in docs/preparing-a-board.md; `kisi gaps` says where to find each value.".into()
        },
    });
    out
}

#[inline(never)]
fn cooling(c: &Value) -> String {
    match g(c, "airflow_m_s").as_f64() {
        Some(a) if a > 0.0 => format!("air flow {} m/s", fx(a, 1)),
        _ => format!("still air, {}", s(g(c, "orientation"))),
    }
}

/// Counts of ok / warning / fail per area, and the five worst items: (counts, worst).
#[inline(never)]
fn glance(rep: &Value, ts: &[Table]) -> (Table, Table) {
    let count = |t: &Table| ["ok", "warn", "fail"].map(|c| t.rows.iter().filter(|r| r.1 == c).count());
    let mut rows = Vec::new();
    // per area: the counts, or None (not computed)
    let mut add = |area: &str, n: Option<[usize; 3]>, what: &str| {
        let (cells, cl) = match n {
            Some(n) => (
                [n[0], n[1], n[2]].map(|x| x.to_string()),
                ["fail", "warn", "ok"][if n[2] > 0 {
                    0
                } else if n[1] > 0 {
                    1
                } else {
                    2
                }],
            ),
            None => (Default::default(), ""),
        };
        let what = if n.is_none() { "not computed" } else { what };
        rows.push((std::iter::once(area.to_string()).chain(cells).chain([what.to_string()]).collect(), cl));
    };
    add("Impedance (net classes)", Some(count(&ts[0])), "");
    let ch = |c: &str| {
        arr(g(g(rep, "channels"), "rows")).iter().filter(|r| class(g(r, "status").as_str().unwrap_or("")) == c).count()
    };
    add("Channel (lanes and nets)", Some([ch("ok"), ch("warn"), ch("fail")]), "");
    let r = arr(g(g(rep, "power"), "rails"));
    let w = r.iter().filter(|x| !arr(g(x, "warnings")).is_empty()).count();
    add(
        "Power (rails)",
        g(g(rep, "power"), "error").is_null().then_some([r.len() - w, w, 0]),
        "warning: a rail with a warning; no drop limit",
    );
    let th = g(rep, "thermal");
    let (f, w) = (
        arr(g(g(th, "summary"), "parts_over_tj_max")).len(),
        arr(g(g(th, "summary"), "parts_within_10k_of_tj_max")).len(),
    );
    let all = arr(g(th, "parts")).len() + g(th, "parts_omitted").as_u64().unwrap_or(0) as usize;
    let done = g(th, "error").is_null() && th.get("board").is_some();
    add(
        "Temperature (parts with heat)",
        done.then_some([all.saturating_sub(f + w), w, f]),
        "estimate; warning: within 10 K of Tj max",
    );
    let counts = Table {
        id: "at_a_glance",
        title: "At a glance".into(),
        head: heads(&["Area", "ok", "warning", "fail", "Note"]),
        rows,
        note: String::new(),
    };
    // the five worst: fail before warning, then the least relative margin
    let mut w: Vec<Line> = Vec::new();
    for r in arr(g(g(rep, "channels"), "rows")) {
        let cl = class(g(r, "status").as_str().unwrap_or(""));
        if let (Some((c, m)), "fail" | "warn") = (worst_criterion(r), cl) {
            let [l, v, lim, _] = crit_cells(c);
            w.push((
                (u8::from(cl == "warn"), m),
                vec!["Channel".into(), lane(r), format!("{l}: {v} ({lim})"), word(cl).into()],
                cl,
            ));
        }
    }
    for c in arr(g(rep, "impedance")) {
        let cl = class(g(c, "status").as_str().unwrap_or(""));
        if let (Some(z), Some(t), Some(win), "fail" | "warn") =
            (g(c, "z_ohm").as_f64(), g(c, "target_ohm").as_f64(), g(c, "window_pct").as_f64(), cl)
        {
            let dev = 100.0 * (z / t - 1.0);
            let what = format!(
                "{}: mean {} Ω, {} % from {} Ω (± {} %)",
                s(g(c, "worst_net")),
                fx(z, 1),
                fx(dev, 1),
                fx(t, 0),
                fx(win, 0)
            );
            w.push((
                (u8::from(cl == "warn"), (win - dev.abs()) / win),
                vec!["Impedance".into(), s(g(c, "class")), what, word(cl).into()],
                cl,
            ));
        }
    }
    for p in arr(g(g(rep, "thermal"), "parts")) {
        let cl = class(g(p, "status").as_str().unwrap_or(""));
        if let (Some(m), Some(mx), "fail" | "warn") = (g(p, "margin_k").as_f64(), g(p, "tj_max_c").as_f64(), cl) {
            let what = format!("Tj {} °C, max {} °C (estimate)", num(g(p, "tj_c"), 1), fx(mx, 0));
            w.push((
                (u8::from(cl == "warn"), m / mx),
                vec!["Temperature".into(), format!("{} {}", s(g(p, "ref")), s(g(p, "part"))), what, word(cl).into()],
                cl,
            ));
        }
    }
    sort_lines(&mut w);
    let worst = Table {
        id: "worst",
        title: "The five worst items".into(),
        head: heads(&["Area", "Item", "Value (limit)", "Verdict"]),
        note: if w.is_empty() { "Nothing over a limit.".into() } else { String::new() },
        rows: w.into_iter().take(5).map(|(_, r, c)| (r, c)).collect(),
    };
    (counts, worst)
}

/// The report's tables for `--csv`: (file name, CSV text), in the format of the viewer's Export →
/// Table (UTF-8 with BOM, CRLF, the title line first; a text that Excel would take for a formula
/// starts with an apostrophe).
pub fn csv_files(rep: &Value) -> Vec<(String, String)> {
    let ts = tables(rep);
    let (a, b) = glance(rep, &ts);
    [a, b].into_iter().chain(ts).map(|t| (format!("{}.csv", t.id), csv(&t))).collect()
}

#[inline(never)]
fn csv(t: &Table) -> String {
    let q = |x: &str| {
        let x = if x.starts_with(['=', '+', '-', '@']) && x.parse::<f64>().is_err() {
            format!("'{x}")
        } else {
            x.to_string()
        };
        if x.contains([',', '"', '\r', '\n']) || x.starts_with(' ') || x.ends_with(' ') {
            format!("\"{}\"", x.replace('"', "\"\""))
        } else {
            x
        }
    };
    let line = |r: &[String]| r.iter().map(|c| q(c)).collect::<Vec<_>>().join(",");
    let mut out = format!("\u{feff}{}\r\n{}\r\n", q(&t.title), line(&t.head));
    for (r, _) in &t.rows {
        out.push_str(&line(r));
        out.push_str("\r\n");
    }
    out
}

#[inline(never)]
fn esc(x: &str) -> String {
    x.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

#[inline(never)]
fn table_html(t: &Table, h: &str) -> String {
    // numbers right-aligned, short cells kept on one line
    let col = |i: usize| t.rows.iter().map(move |r| r.0[i].as_str()).filter(|x| !x.is_empty());
    let numeric: Vec<bool> = (0..t.head.len()).map(|i| col(i).all(|x| x.parse::<f64>().is_ok())).collect();
    let short: Vec<bool> = (0..t.head.len()).map(|i| col(i).all(|x| x.chars().count() <= 16)).collect();
    let mut o = format!("<{h}>{}</{h}>", esc(&t.title));
    if !t.rows.is_empty() {
        o.push_str("<table><thead><tr>");
        for x in &t.head {
            o.push_str(&format!("<th>{}</th>", esc(x)));
        }
        o.push_str("</tr></thead><tbody>");
        for (r, c) in &t.rows {
            o.push_str(&format!("<tr class=\"{c}\">"));
            for (i, x) in r.iter().enumerate() {
                let v = if t.head[i] == "Verdict" { format!("<b class=\"v {c}\">{}</b>", esc(x)) } else { esc(x) };
                o.push_str(&format!(
                    "<td{}>{v}</td>",
                    if numeric[i] {
                        " class=\"n\""
                    } else if short[i] {
                        " class=\"nw\""
                    } else {
                        ""
                    }
                ));
            }
            o.push_str("</tr>");
        }
        o.push_str("</tbody></table>");
    }
    if !t.note.is_empty() {
        o.push_str(&format!("<p class=\"note\">{}</p>", esc(&t.note)));
    }
    o
}

const CSS: &str = "@page{margin:10mm}*{box-sizing:border-box}html{background:#fff;color:#111}\
body{font:8pt/1.3 system-ui,-apple-system,'Segoe UI',Roboto,Helvetica,Arial,sans-serif;max-width:190mm;margin:0 auto;padding:6mm 0}\
h1{font-size:15pt;margin:0}h2{font-size:10.5pt;margin:8pt 0 2pt;padding-bottom:1pt;border-bottom:1.5px solid #333;break-after:avoid}\
h3{font-size:9pt;margin:7pt 0 2pt;break-after:avoid}table{border-collapse:collapse;width:100%;margin:2pt 0}\
thead{display:table-header-group}tr{break-inside:avoid}th,td{text-align:left;vertical-align:top;padding:1pt 3pt;border-bottom:.5pt solid #ccc}\
th{background:#eee;font-weight:600}td.n{text-align:right;font-variant-numeric:tabular-nums;white-space:nowrap}td.nw{white-space:nowrap}\
tr.warn td{background:#fff3d6}tr.fail td{background:#fde0e0}.v.ok{color:#126b35}.v.warn{color:#7a5200}.v.fail{color:#a4001d}\
.meta{display:grid;grid-template-columns:max-content 1fr;gap:1pt 10pt;margin:5pt 0}.meta b{font-weight:600}\
.note{margin:2pt 0 4pt;color:#444}ul{margin:2pt 0;padding-left:14pt}footer{margin-top:10pt;padding-top:3pt;border-top:.5pt solid #999;color:#555;font-size:7.5pt}\
@media screen{body{padding:16px}}@media print{.hint{display:none}}.hint{background:#eef3f8;padding:4pt 6pt;border-radius:3pt}";

/// The stackup's sources in one line: board file, kisi.json, a fab template, edited.
#[inline(never)]
fn stackup_source(p: &Value) -> String {
    let all: Vec<String> =
        arr(g(p, "dielectrics")).iter().chain(arr(g(p, "copper"))).chain([g(p, "mask")]).map(s).collect();
    let mut v: Vec<String> = Vec::new();
    for x in &all {
        let k = if let Some(t) = x.strip_prefix("preset ") {
            format!("fab template {t}")
        } else if x.starts_with("kisi.json") {
            "kisi.json".into()
        } else if x.contains("--stackup") || x.contains("viewer") {
            "edited".into()
        } else {
            x.clone()
        };
        if !k.is_empty() && !v.contains(&k) {
            v.push(k);
        }
    }
    // in the order they are laid over each other: the board file, kisi.json, a template, edits
    let at =
        |k: &String| ["kisi.json", "fab template", "edited"].iter().position(|p| k.starts_with(p)).map_or(0, |i| i + 1);
    let mut o: Vec<(usize, String)> = v.into_iter().map(|k| (at(&k), k)).collect();
    for i in 1..o.len() {
        let mut j = i;
        while j > 0 && o[j].0 < o[j - 1].0 {
            o.swap(j, j - 1);
            j -= 1;
        }
    }
    o.into_iter().map(|x| x.1).collect::<Vec<_>>().join(" < ")
}

/// The report as one self-contained HTML page; `date` (YYYY-MM-DD) from the caller (the WASM build has
/// no clock).
pub fn html(rep: &Value, date: &str) -> String {
    let ts = tables(rep);
    let (counts, worst) = glance(rep, &ts);
    let ov = g(rep, "overview");
    let var = g(rep, "variant");
    let o = g(rep, "options");
    let fsol = g(ov, "field_solver");
    let field = if g(fsol, "cross_sections_field").as_u64().unwrap_or(0) > 0 {
        format!(
            "field solver for {} % of the routed length with a target, formulas elsewhere",
            num(g(fsol, "target_length_coverage_pct"), 0)
        )
    } else {
        "formulas (no field solver)".into()
    };
    let via: Vec<&str> = [("via_cavity", "via cavities"), ("via_returns", "via return groups")]
        .iter()
        .filter(|(k, _)| o[*k] == true)
        .map(|x| x.1)
        .collect();
    let cfg = g(rep, "config");
    let cfg_txt = match g(cfg, "file").as_str() {
        Some(f) => {
            let f = f.rsplit(['/', '\\']).next().unwrap_or(f);
            if g(cfg, "ok") == false {
                format!("{f} ({} error(s))", arr(g(cfg, "errors")).len())
            } else {
                f.to_string()
            }
        }
        None => "none".into(),
    };
    let sha = g(g(rep, "board_file"), "sha256").as_str().unwrap_or("");
    let meta: Vec<(&str, String)> = vec![
        (
            "Board",
            format!(
                "{} · {}{}",
                s(g(ov, "board")),
                s(g(g(rep, "board_file"), "name")),
                if sha.is_empty() { String::new() } else { format!(" · sha256 {}", &sha[..12.min(sha.len())]) }
            ),
        ),
        (
            "Assembly variant",
            g(var, "name")
                .as_str()
                .map(|n| format!("{n} ({})", s(g(var, "chosen_by"))))
                .unwrap_or_else(|| "none: the board file as it is".into()),
        ),
        ("Configuration", cfg_txt),
        (
            "Stackup",
            format!(
                "{} copper layers · {}",
                s(g(ov, "layers")),
                stackup_source(g(g(rep, "stackup_used"), "provenance"))
            ),
        ),
        (
            "Options",
            format!(
                "impedance by {field}; fab tolerances {}; experimental via options: {}",
                if g(o, "tolerances") == true { "on" } else { "off" },
                if via.is_empty() { "off".into() } else { via.join(", ") }
            ),
        ),
        ("kisi", format!("{} · {date}", s(g(rep, "version")))),
    ];
    let mut b = String::with_capacity(32 * 1024);
    b.push_str(&format!("<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><title>kisi report – {}</title><style>{CSS}</style></head><body>", esc(&s(g(ov, "board")))));
    b.push_str("<p class=\"hint\">Print this page or save it as PDF with the browser's print dialog (Ctrl+P, on a Mac ⌘P), on A4 or Letter.</p>");
    b.push_str(&format!("<h1>kisi report · {}</h1><div class=\"meta\">", esc(&s(g(ov, "board")))));
    for (k, v) in meta {
        b.push_str(&format!("<b>{k}</b><span>{}</span>", esc(&v)));
    }
    b.push_str("</div>");
    b.push_str(&table_html(&counts, "h2"));
    b.push_str(&table_html(&worst, "h3"));
    b.push_str("<h2>Impedance</h2>");
    b.push_str(&table_html(&ts[0], "h3"));
    b.push_str("<h2>Channel</h2>");
    b.push_str(&table_html(&ts[1], "h3"));
    b.push_str(&table_html(&ts[2], "h3"));
    b.push_str("<h2>Power</h2>");
    for t in &ts[3..6] {
        b.push_str(&table_html(t, "h3"));
    }
    // assumptions: what is estimated, and the open gaps
    let est = |x: &Value| x.as_bool() == Some(true);
    let eff =
        arr(g(g(g(rep, "power"), "balance"), "elements")).iter().filter(|e| est(g(e, "efficiency_estimate"))).count();
    let pkg = arr(g(g(rep, "thermal"), "parts")).iter().filter(|p| est(g(p, "estimate"))).count();
    let mut items = vec![
        format!("Impedance: {field}; formula values are estimates (typical deviations in docs/models.md)."),
        "Channel: parasitics of series parts and packages are estimates (data/components.json); the eye uses the interface profiles' linear transmitter and receiver.".to_string(),
    ];
    if eff > 0 {
        items.push(format!(
            "Power: {eff} regulator(s) without an efficiency in PI_REG – kisi's default of 85 % (estimate)."
        ));
    }
    if let Some(e) = g(g(rep, "thermal"), "estimate").as_str() {
        items.push(format!(
            "{e}{}",
            if pkg > 0 { format!(" {pkg} of the parts listed with estimated package data.") } else { String::new() }
        ));
    }
    if g(o, "tolerances") == true {
        items.push(
            "Fab tolerances: kisi's default tolerances are estimates where neither kisi.json nor --tol gives them."
                .into(),
        );
    }
    b.push_str("<h2>Assumptions and gaps</h2><ul>");
    for i in items {
        b.push_str(&format!("<li>{}</li>", esc(&i)));
    }
    b.push_str("</ul>");
    b.push_str(&table_html(&ts[6], "h3"));
    b.push_str("<footer>Left out on purpose: maps, curves, per-net details and via lists – they are in the viewer and in the report's JSON (kisi report, Export → Data). A quick design check: results are estimates, without any warranty of correctness. kisi, GPL-3.0, https://github.com/riehlies/kisi</footer></body></html>");
    b
}

/// SHA-256 of `data` as hex (FIPS 180-4): the board file's hash in the report's header.
pub fn sha256_hex(data: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98,
        0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786,
        0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8,
        0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
        0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819,
        0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a,
        0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] =
        [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];
    let mut m = data.to_vec();
    m.push(0x80);
    while m.len() % 64 != 56 {
        m.push(0);
    }
    m.extend_from_slice(&((data.len() as u64) * 8).to_be_bytes());
    for c in m.chunks(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([c[4 * i], c[4 * i + 1], c[4 * i + 2], c[4 * i + 3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let mut v = h;
        for i in 0..64 {
            let s1 = v[4].rotate_right(6) ^ v[4].rotate_right(11) ^ v[4].rotate_right(25);
            let ch = (v[4] & v[5]) ^ (!v[4] & v[6]);
            let t1 = v[7].wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = v[0].rotate_right(2) ^ v[0].rotate_right(13) ^ v[0].rotate_right(22);
            let maj = (v[0] & v[1]) ^ (v[0] & v[2]) ^ (v[1] & v[2]);
            let t2 = s0.wrapping_add(maj);
            v = [t1.wrapping_add(t2), v[0], v[1], v[2], v[3].wrapping_add(t1), v[4], v[5], v[6]];
        }
        for i in 0..8 {
            h[i] = h[i].wrapping_add(v[i]);
        }
    }
    h.iter().map(|x| format!("{x:08x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn sha256_known() {
        assert_eq!(sha256_hex(b""), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_eq!(sha256_hex(&[b'a'; 64]), "ffe054fe7ae0cb6dc65c3af9b61d5209f439851db43d0ba5997337df154668eb");
    }

    /// A small report: the verdicts and counts come from its statuses, the failing lane is listed with
    /// its worst criterion, power not computed reads so, CSV and HTML carry the same rows.
    #[test]
    fn renders_a_small_report() {
        let crit = |k: &str, st: &str, v: f64, lo: Option<f64>, hi: Option<f64>| json!({"key": k, "status": st, "value": v, "lo": lo, "hi": hi});
        let rep = json!({
            "version": "0.0.0", "board_file": {"name": "b.kicad_pcb", "sha256": "0123456789abcdef"},
            "options": {"tolerances": false, "via_cavity": true, "via_returns": false},
            "variant": {"name": null}, "config": {"file": null},
            "overview": {"board": "b", "layers": 4, "field_solver": {"cross_sections_field": 0, "target_length_coverage_pct": 0.0}},
            "impedance": [{"class": "50Z", "target_ohm": 50.0, "diff": false, "worst_net": "/A{slash}B", "z_ohm": 56.0, "window_pct": 10.0, "status": "yellow", "within_5pct": 0.25},
                          {"class": "D100", "target_ohm": 100.0, "diff": true, "worst_net": "/B_P", "z_ohm": 101.0, "window_pct": 10.0, "status": "green", "within_5pct": 1.0}],
            "channels": {"rows": [
                {"lane": "/A", "bus": "U1 ↔ U2", "profile": "se50", "status": "red", "criteria": [crit("z", "yellow", 56.0, Some(45.0), Some(55.0)), crit("eye_h", "red", 0.3, Some(0.65), None)]},
                {"lane": "/B_P", "bus": "U1 ↔ U2", "profile": "lvds", "status": "green", "criteria": [crit("eye_h", "green", 0.9, Some(0.65), None)]}]},
            "power": {"error": "no loads"},
            "thermal": {"error": "no power tree"},
            "stackup_used": {"provenance": {"dielectrics": ["board file", "kisi.json (datasheet)"], "copper": ["board file"], "mask": "preset jlc4 (JLCPCB x)"}},
            "gaps": {"gaps": [{"id": "laminate", "priority": "high", "views": ["impedance"], "issue": "εr & tanδ <default>"}]}
        });
        let ts = tables(&rep);
        assert_eq!(ts[0].rows.iter().map(|r| r.1).collect::<Vec<_>>(), ["warn", "ok"]);
        assert_eq!((ts[0].rows[0].0[2].as_str(), ts[0].rows[0].0[5].as_str()), ("SE", "12.0")); // deviation from the target, %
                                                                                                // the bus line: its worst lane and criterion (the eye, red), counts per verdict
        let b = &ts[1].rows[0].0;
        assert_eq!(
            (b[2].as_str(), b[3].as_str(), b[5].as_str(), b[6].as_str(), b[7].as_str()),
            ("2", "1", "1", "/A", "eye height (%)")
        );
        assert_eq!((b[8].as_str(), b[9].as_str(), b[10].as_str(), b[11].as_str()), ("30", "≥ 65", "-35", "fail"));
        assert_eq!(ts[2].rows.len(), 1); // the failing lane
        assert!(ts[3].note.starts_with("not computed: no loads") && ts[3].rows.is_empty());
        let (counts, worst) = glance(&rep, &ts);
        assert_eq!(counts.rows[0].0[1..4], ["1", "1", "0"]);
        assert_eq!(counts.rows[1].0[1..4], ["1", "0", "1"]);
        assert_eq!(worst.rows[0].0[1], "/A");
        let h = html(&rep, "2026-01-02");
        assert!(h.starts_with("<!doctype html>") && h.contains("sha256 0123456789ab") && h.contains("2026-01-02"));
        assert!(
            h.contains("board file &lt; kisi.json &lt; fab template jlc4")
                && h.contains("experimental via options: via cavities")
        );
        assert!(h.contains("εr &amp; tanδ &lt;default&gt;") && !h.contains("<script") && !h.contains("http://"));
        assert!(h.contains("<thead><tr><th>Class</th>") && h.contains("Left out on purpose"));
        let csv = csv_files(&rep);
        let imp = &csv.iter().find(|f| f.0 == "impedance.csv").unwrap().1;
        assert!(imp.starts_with("\u{feff}Impedance per net class\r\nClass,Target (Ω)"));
        assert!(imp.contains("\r\n50Z,50,SE,/A/B,56.0,12.0,10,25,warning\r\n"), "{imp}");
    }
}

//! The findings of `kisi report`: every result over or near a limit, and every open gap, with where it
//! applies on the board (`locate::Spot`) and the geometry of a small board sketch – one list for the
//! report's JSON (`findings`, `sketch_board`), its HTML and CSV (`report_html`) and the viewer (Export →
//! Report and its jump to a finding).
//!
//! What a finding is comes from the report's own verdicts: channel rows that are not green (their worst
//! criterion, `profile::judge`), net classes whose worst net's mean impedance is not green, rails with
//! vias over their current capacity estimate, rails with other warnings, parts over or within 10 K of Tj
//! max (estimates) and the open gaps. This module only collects them, places them and orders them.
//!
//! Ids are stable for a board and its results: `<area>:<item>` – `channel:<lane or net>`,
//! `impedance:<net class>`, `vias:<rail>`, `power:<rail>`, `temperature:<part>`, `gap:<gap id>` (KiCad's
//! escaped slash in net names as `/`); a second finding with the same id gets `#2`.

use crate::locate::{r2, Spot};
use crate::report_html::{crit_cells, fx, g, n2, s, worst_criterion};
use crate::service::Project;
use serde_json::{json, Value};

/// Most points of a sketch's overview lines; above it the lines are simplified more.
const OVERVIEW_POINTS: usize = 300;
/// Most copper items of a sketch's detail view (the nearest to its centre).
const DETAIL_ITEMS: usize = 50;
/// Most markers of one finding (gap items, vias over their estimate).
const MARKERS: usize = 20;

/// The findings, their groups (one per net, each with its sketch) and the board's outline for the
/// sketches: (findings, finding_groups, sketch_board).
pub fn findings(p: &Project, rep: &Value) -> (Value, Value, Value) {
    let mut out: Vec<(u8, u8, f64, Value)> = Vec::new(); // (severity, area, margin, finding)
    let sev_rank = |s: &str| match s {
        "fail" => 0,
        "warn" => 1,
        _ => 2,
    };
    let cls = |st: &str| match st {
        "red" | "over" => "fail",
        "yellow" | "near" => "warn",
        _ => "",
    };
    let names = |v: &Value| -> Vec<String> { crate::report_html::arr(v).iter().map(s).collect() };
    // channel rows
    for r in crate::report_html::arr(g(g(rep, "channels"), "rows")) {
        let sev = cls(g(r, "status").as_str().unwrap_or(""));
        if sev.is_empty() {
            continue;
        }
        let item = s(if g(r, "lane").is_null() { g(r, "net") } else { g(r, "lane") });
        let (c, m) = match worst_criterion(r) {
            Some(x) => x,
            None => continue,
        };
        let [label, value, limit, _] = crit_cells(c);
        let mut nets = names(g(r, "nets"));
        if nets.is_empty() {
            nets.push(s(g(r, "net")));
        }
        let detail = match (g(r, "bus").as_str(), g(r, "from").as_str(), g(r, "to").as_str()) {
            (Some(b), Some(f), Some(t)) => format!("bus {b}, profile {}, {f} → {t}", s(g(r, "profile"))),
            (Some(b), _, _) => format!("bus {b}, profile {}", s(g(r, "profile"))),
            _ => format!("net without devices, class {}, profile {}", s(g(r, "class")), s(g(r, "profile"))),
        };
        let what = if c["key"] == "error" {
            format!("{item}: not computed ({})", s(g(g(r, "metrics"), "error")))
        } else {
            format!("{item}: {label} {value} (limit {limit})")
        };
        out.push((
            sev_rank(sev),
            0,
            m,
            finding(p, "channel", &item, sev, what, detail, false, g(r, "location"), vec![], &nets),
        ));
    }
    // impedance per net class
    for c in crate::report_html::arr(g(rep, "impedance")) {
        let sev = cls(g(c, "status").as_str().unwrap_or(""));
        let (Some(z), Some(t), Some(win)) =
            (g(c, "z_ohm").as_f64(), g(c, "target_ohm").as_f64(), g(c, "window_pct").as_f64())
        else {
            continue;
        };
        if sev.is_empty() {
            continue;
        }
        let dev = 100.0 * (z / t - 1.0);
        let net = s(g(c, "worst_net"));
        let what = format!(
            "net class {}: worst net {net}, mean {} Ω, {} % from {} Ω (window ± {} %)",
            s(g(c, "class")),
            fx(z, 1),
            fx(dev, 1),
            fx(t, 0),
            fx(win, 0)
        );
        let detail = "the location: the worst net's track that moves its mean impedance most".to_string();
        out.push((sev_rank(sev), 1, (win - dev.abs()) / win, {
            let mut f = finding(
                p,
                "impedance",
                &s(g(c, "class")),
                sev,
                what,
                detail,
                false,
                g(c, "location"),
                vec![],
                std::slice::from_ref(&net),
            );
            f["_names"] = json!([net]);
            f
        }));
    }
    // power: vias over their current capacity estimate, other warnings of a rail
    let pw = g(rep, "power");
    let dt = crate::report_html::num(g(g(pw, "via_capacity"), "delta_t_k"), 0);
    for r in crate::report_html::arr(g(pw, "rails")) {
        let rail = s(g(r, "rail"));
        let vias = g(r, "vias");
        let n = g(vias, "over_capacity").as_u64().unwrap_or(0);
        if n > 0 {
            let spot = |v: &Value| -> Option<Spot> {
                let at = g(v, "at").as_array()?;
                Some(crate::locate::via_spot(
                    &p.board,
                    [at.first()?.as_f64()?, at.get(1)?.as_f64()?],
                    g(v, "from").as_str().unwrap_or(""),
                    g(v, "to").as_str().unwrap_or(""),
                    g(v, "drill_mm").as_f64().unwrap_or(0.0),
                    g(v, "plated_hole") == true,
                ))
            };
            let most = g(vias, "most_loaded");
            let more: Vec<Spot> = crate::report_html::arr(g(vias, "highest_current"))
                .iter()
                .filter(|v| g(v, "over_capacity") == true && g(v, "at") != g(most, "at"))
                .filter_map(spot)
                .collect();
            let l = g(most, "current_per_capacity").as_f64().unwrap_or(0.0);
            let what = format!(
                "{rail}: {n} via{} over the current capacity estimate (IPC-2152 at ΔT {dt} K); the most loaded {} A, {} % of {} A (estimate)",
                if n == 1 { "" } else { "s" },
                crate::report_html::num(g(most, "current_a"), 2),
                fx(100.0 * l, 0),
                crate::report_html::num(g(most, "capacity_a"), 2)
            );
            let detail = format!(
                "markers: the most loaded via and the other vias over the estimate among the {} with the highest current",
                crate::report_html::arr(g(vias, "highest_current")).len()
            );
            out.push((1, 2, -l, finding(p, "vias", &rail, "warn", what, detail, true, &json!(spot(most)), more, &[])));
        }
        let ws: Vec<String> = crate::report_html::arr(g(r, "warnings"))
            .iter()
            .map(s)
            .filter(|w| !w.contains("current capacity estimate"))
            .collect();
        if !ws.is_empty() {
            let detail = "the location: the load with the largest drop on this rail".to_string();
            out.push((
                1,
                3,
                0.0,
                finding(
                    p,
                    "power",
                    &rail,
                    "warn",
                    format!("{rail}: {}", ws.join("; ")),
                    detail,
                    false,
                    g(r, "max_at"),
                    vec![],
                    &[],
                ),
            ));
        }
    }
    // temperature: junctions over or near Tj max
    for t in crate::report_html::arr(g(g(rep, "thermal"), "parts")) {
        let sev = cls(g(t, "status").as_str().unwrap_or(""));
        if sev.is_empty() {
            continue;
        }
        let r = s(g(t, "ref"));
        let what = format!(
            "{r} {}: Tj {} °C, Tj max {} °C, margin {} K (estimate)",
            s(g(t, "part")),
            crate::report_html::num(g(t, "tj_c"), 1),
            crate::report_html::num(g(t, "tj_max_c"), 0),
            crate::report_html::num(g(t, "margin_k"), 1)
        );
        let detail = format!("thermal data: {}", s(g(t, "data_source")));
        let spot = crate::locate::part_spot(&p.board, &r, None);
        let m = g(t, "margin_k").as_f64().unwrap_or(0.0);
        out.push((
            sev_rank(sev),
            4,
            m,
            finding(p, "temperature", &r, sev, what, detail, true, &json!(spot), vec![], &[]),
        ));
    }
    // the open gaps: where their items are parts, those parts
    for x in crate::report_html::arr(g(g(rep, "gaps"), "gaps")) {
        let items = crate::report_html::arr(g(x, "items"));
        let spots: Vec<Spot> = items
            .iter()
            .filter_map(|i| g(i, "ref").as_str())
            .take(MARKERS)
            .filter_map(|r| crate::locate::part_spot(&p.board, r, None))
            .collect();
        let what = format!("{}: {}", s(g(x, "id")), s(g(x, "issue")));
        let detail = format!(
            "priority {}; affects {}{}",
            s(g(x, "priority")),
            crate::report_html::arr(g(x, "views")).iter().map(s).collect::<Vec<_>>().join(", "),
            if items.len() > spots.len() && !spots.is_empty() {
                format!("; markers: the first {} of {} items", spots.len(), items.len())
            } else {
                String::new()
            }
        );
        let mut it = spots.into_iter();
        let first = it.next();
        out.push((
            2,
            5,
            0.0,
            finding(p, "gap", &s(g(x, "id")), "gap", what, detail, false, &json!(first), it.collect(), &[]),
        ));
    }
    // fail, warning, gap; then by area; then the least margin (stable)
    let less = |a: &(u8, u8, f64, Value), b: &(u8, u8, f64, Value)| {
        (a.0, a.1) < (b.0, b.1) || ((a.0, a.1) == (b.0, b.1) && a.2 < b.2)
    };
    for i in 1..out.len() {
        let mut j = i;
        while j > 0 && less(&out[j], &out[j - 1]) {
            out.swap(j, j - 1);
            j -= 1;
        }
    }
    // unique ids
    let mut ids: Vec<String> = Vec::new();
    let list: Vec<Value> = out
        .into_iter()
        .map(|(_, _, _, mut f)| {
            let base = s(g(&f, "id"));
            let mut id = base.clone();
            let mut k = 2;
            while ids.contains(&id) {
                id = format!("{base}#{k}");
                k += 1;
            }
            ids.push(id.clone());
            f["id"] = json!(id);
            f
        })
        .collect();
    let mut list = list;
    let gs = groups(p, &mut list);
    (json!(list), gs, sketch_board(p))
}

/// One finding: what, where and its sketch's geometry. `loc`: the location (a `Spot` as JSON, or
/// null); `more`: further markers; `nets`: the nets to highlight (their partners added).
#[allow(clippy::too_many_arguments)]
fn finding(
    p: &Project,
    area: &str,
    item: &str,
    severity: &str,
    what: String,
    detail: String,
    estimate: bool,
    loc: &Value,
    more: Vec<Spot>,
    nets: &[String],
) -> Value {
    let spot: Option<Spot> = serde_json::from_value(loc.clone()).ok();
    let mut ids: Vec<u32> = nets.iter().filter_map(|n| p.net_id(n)).collect();
    if let Some(n) = spot.as_ref().filter(|s| !s.net.is_empty()).and_then(|s| p.net_id(&s.net)) {
        if !ids.contains(&n) && area != "vias" && area != "power" {
            ids.push(n);
        }
    }
    let partners: Vec<u32> = p.an.nets.iter().filter(|st| ids.contains(&st.net)).filter_map(|st| st.partner).collect();
    for n in partners {
        if !ids.contains(&n) {
            ids.push(n);
        }
    }
    let item = item.replace("{slash}", "/");
    let mut f = json!({
        "id": format!("{area}:{item}"), "severity": severity, "area": area, "item": item,
        "what": what.replace("{slash}", "/"), "detail": detail.replace("{slash}", "/"), "estimate": estimate,
        "location": spot, "markers": more,
    });
    // the nets the finding is on, for its group (a rail: the rail; parts and gaps: none)
    let rail = spot.as_ref().filter(|_| area == "vias" || area == "power").and_then(|s| p.net_id(&s.net));
    f["_nets"] = json!(match rail {
        Some(r) => vec![r],
        None if area == "temperature" || area == "gap" => vec![],
        None => ids,
    });
    f["_rail"] = json!(rail.is_some());
    f
}

/// The findings in groups: one per net – the findings on the same nets (a lane's nets with both lines
/// of a pair, a net class's worst net, a rail) together –, a part's or a gap's on their own; in the
/// order of their worst finding. Each group gets one sketch: its nets, the place of its worst finding,
/// markers at the others (numbered as listed). Removes the findings' working keys.
fn groups(p: &Project, list: &mut [Value]) -> Value {
    // (net ids, rail, members)
    let mut gs: Vec<(Vec<u32>, bool, Vec<usize>)> = Vec::new();
    for (i, f) in list.iter().enumerate() {
        let nets: Vec<u32> =
            crate::report_html::arr(g(f, "_nets")).iter().filter_map(|x| x.as_u64().map(|x| x as u32)).collect();
        let rail = g(f, "_rail") == true;
        match gs.iter_mut().find(|x| !nets.is_empty() && x.1 == rail && x.0.iter().any(|n| nets.contains(n))) {
            Some(x) => {
                x.2.push(i);
                for n in nets {
                    if !x.0.contains(&n) {
                        x.0.push(n);
                    }
                }
            }
            None => gs.push((nets, rail, vec![i])),
        }
    }
    let mut out = Vec::new();
    for (nets, rail, members) in gs {
        let first = &list[members[0]];
        let area = s(g(first, "area"));
        let title = match area.as_str() {
            "impedance" => {
                crate::report_html::arr(g(first, "_names")).first().map(s).unwrap_or_else(|| s(g(first, "item")))
            }
            "gap" => format!("gap {}", s(g(first, "item"))),
            _ => s(g(first, "item")),
        };
        let kind = match area.as_str() {
            "channel" | "impedance" => "net",
            "vias" | "power" => "rail",
            "temperature" => "part",
            _ => "gap",
        };
        let id = s(g(first, "id"));
        // the sketch: at the first finding with a place, markers at every place, numbered by member
        let spots: Vec<(usize, Spot)> = members
            .iter()
            .enumerate()
            .filter_map(|(k, &i)| Some((k + 1, serde_json::from_value::<Spot>(g(&list[i], "location").clone()).ok()?)))
            .collect();
        let sketch = match spots.first() {
            Some((_, sp)) => {
                // one marker per place (two findings at the same place: the first's number)
                let mut marks: Vec<([f64; 2], usize)> = Vec::new();
                let mut add = |at: [f64; 2], k: usize| {
                    if !marks.iter().any(|m| (m.0[0] - at[0]).abs() < 0.05 && (m.0[1] - at[1]).abs() < 0.05) {
                        marks.push((at, k));
                    }
                };
                for (k, s) in &spots {
                    add(s.at, *k);
                }
                for &i in &members {
                    for m in crate::report_html::arr(g(&list[i], "markers")) {
                        if let Ok(s) = serde_json::from_value::<Spot>(m.clone()) {
                            add(s.at, 0);
                        }
                    }
                }
                sketch(p, &nets, sp, &marks, rail)
            }
            None => Value::Null,
        };
        let names: Vec<String> = nets.iter().map(|&n| p.board.net_name(n).replace("{slash}", "/")).collect();
        let severity = g(first, "severity").clone();
        for &i in &members {
            list[i]["group"] = json!(id);
        }
        out.push(json!({"id": id, "title": title.replace("{slash}", "/"), "kind": kind, "severity": severity,
            "findings": members.iter().map(|&i| g(&list[i], "id").clone()).collect::<Vec<_>>(), "nets": names, "sketch": sketch}));
    }
    for f in list.iter_mut() {
        if let Some(o) = f.as_object_mut() {
            o.remove("_nets");
            o.remove("_rail");
            o.remove("_names");
        }
    }
    json!(out)
}

// ---- sketch geometry

#[inline]
fn r1(x: f64) -> f64 {
    (x * 10.0).round() / 10.0 + 0.0
}

/// Ramer–Douglas–Peucker: the points of `pts` kept within `eps`.
fn simplify(pts: &[[f64; 2]], eps: f64) -> Vec<[f64; 2]> {
    if pts.len() < 3 {
        return pts.to_vec();
    }
    let mut keep = vec![false; pts.len()];
    keep[0] = true;
    keep[pts.len() - 1] = true;
    let mut stack = vec![(0usize, pts.len() - 1)];
    while let Some((a, b)) = stack.pop() {
        let (pa, pb) = (pts[a], pts[b]);
        let d = [pb[0] - pa[0], pb[1] - pa[1]];
        let l = (d[0] * d[0] + d[1] * d[1]).sqrt();
        let mut best = (0.0, 0);
        for (i, q) in pts.iter().enumerate().take(b).skip(a + 1) {
            let e = if l < 1e-12 {
                ((q[0] - pa[0]).powi(2) + (q[1] - pa[1]).powi(2)).sqrt()
            } else {
                ((q[0] - pa[0]) * d[1] - (q[1] - pa[1]) * d[0]).abs() / l
            };
            if e > best.0 {
                best = (e, i);
            }
        }
        if best.0 > eps {
            keep[best.1] = true;
            stack.push((a, best.1));
            stack.push((best.1, b));
        }
    }
    pts.iter().zip(keep).filter(|x| x.1).map(|x| *x.0).collect()
}

/// Segments chained into polylines where they share an end (greedy, in their order).
fn chain(segs: &[([f64; 2], [f64; 2])]) -> Vec<Vec<[f64; 2]>> {
    let near = |a: [f64; 2], b: [f64; 2]| (a[0] - b[0]).abs() < 1e-3 && (a[1] - b[1]).abs() < 1e-3;
    let mut used = vec![false; segs.len()];
    let mut out = Vec::new();
    for i in 0..segs.len() {
        if used[i] {
            continue;
        }
        used[i] = true;
        let mut line = vec![segs[i].0, segs[i].1];
        loop {
            let end = *line.last().unwrap();
            let next = (0..segs.len()).find(|&k| !used[k] && (near(segs[k].0, end) || near(segs[k].1, end)));
            let Some(k) = next else { break };
            used[k] = true;
            line.push(if near(segs[k].0, end) { segs[k].1 } else { segs[k].0 });
        }
        out.push(line);
    }
    out
}

/// The board for the sketches: outline (simplified to 0.05 mm, at most 1500 points), its box, the
/// copper layers' names.
fn sketch_board(p: &Project) -> Value {
    let b = &p.board;
    let mut eps = 0.05;
    let lines = loop {
        let ls: Vec<Vec<[f64; 2]>> = b.outline.iter().map(|o| simplify(o, eps)).collect();
        if ls.iter().map(|l| l.len()).sum::<usize>() <= 1500 || eps > 2.0 {
            break ls;
        }
        eps *= 2.0;
    };
    let bb = b.bbox.map(|x| [r2(x.min[0]), r2(x.min[1]), r2(x.max[0]), r2(x.max[1])]);
    json!({
        "outline": lines.iter().map(|l| l.iter().flat_map(|q| [r2(q[0]), r2(q[1])]).collect::<Vec<_>>()).collect::<Vec<_>>(),
        "bbox": bb,
        "layers": b.stackup().copper.iter().map(|c| c.name.clone()).collect::<Vec<_>>(),
    })
}

/// One finding's sketch: the highlighted nets over the whole board (`overview`: polylines per layer,
/// simplified, their vias) and the copper around the location (`detail`: tracks, vias and pads of every
/// net in a window, the highlighted ones marked), the focus layer and the markers.
fn sketch(p: &Project, nets: &[u32], sp: &Spot, marks: &[([f64; 2], usize)], rail: bool) -> Value {
    let b = &p.board;
    let su = b.stackup();
    let li = |n: &str| su.layer_index(n).map(|i| i as i64).unwrap_or(-1);
    let focus = if !sp.layer.is_empty() { li(&sp.layer) } else { sp.layers.first().map(|l| li(l)).unwrap_or(-1) };
    // overview: the nets' tracks as simplified polylines per layer (not for rails: their copper is planes)
    let diag = b.bbox.map(|x| ((x.max[0] - x.min[0]).hypot(x.max[1] - x.min[1])).max(1.0)).unwrap_or(100.0);
    let own: Vec<u32> = if rail { vec![] } else { nets.to_vec() };
    let mut eps = diag / 800.0;
    let lines = loop {
        let mut ls: Vec<Value> = Vec::new();
        let mut npts = 0;
        for l in 0..su.copper.len() as u16 {
            let segs: Vec<([f64; 2], [f64; 2])> = b
                .tracks
                .iter()
                .filter(|t| t.layer == l && !t.pad && own.contains(&t.net))
                .map(|t| (t.a, t.b))
                .collect();
            for c in chain(&segs) {
                let q = simplify(&c, eps);
                npts += q.len();
                let mut v = l.to_string();
                for x in &q {
                    v.push_str(&format!(" {} {}", n2(r1(x[0])), n2(r1(x[1]))));
                }
                ls.push(json!(v));
            }
        }
        if npts <= OVERVIEW_POINTS || eps > diag / 20.0 {
            break ls;
        }
        eps *= 2.0;
    };
    let ov_vias: Vec<String> = b
        .vias
        .iter()
        .filter(|v| own.contains(&v.net))
        .map(|v| format!("{} {}", n2(r1(v.at[0])), n2(r1(v.at[1]))))
        .collect();
    // detail: a window around the location – the item and its neighbourhood, half 2.5 … 5 mm across
    let w = (sp.bbox[2] - sp.bbox[0]).max(0.0);
    let h = (sp.bbox[3] - sp.bbox[1]).max(0.0);
    let half = if sp.item == "track" || sp.item == "part" { (0.5 * w.max(h)).clamp(2.5, 5.0) } else { 3.0 };
    let (cx, cy) = (sp.at[0], sp.at[1]);
    let win = [cx - half, cy - half, cx + half, cy + half];
    let inside =
        |a: [f64; 2], m: f64| a[0] >= win[0] - m && a[0] <= win[2] + m && a[1] >= win[1] - m && a[1] <= win[3] + m;
    let seg_in = |a: [f64; 2], c: [f64; 2]| {
        // the segment's box overlaps the window
        a[0].min(c[0]) <= win[2] && a[0].max(c[0]) >= win[0] && a[1].min(c[1]) <= win[3] && a[1].max(c[1]) >= win[1]
    };
    // highlighted in the detail: the nets, and the location's own net (a rail's)
    let mut hl: Vec<u32> = nets.to_vec();
    if let Some(n) = p.net_id(&sp.net).filter(|n| !hl.contains(n)) {
        hl.push(n);
    }
    let mine = |n: u32| hl.contains(&n);
    let dist = |a: [f64; 2]| (a[0] - cx).hypot(a[1] - cy);
    // one text per item, numbers separated by spaces (compact in the report's pretty JSON):
    // `t layer width x0 y0 x1 y1 own`, `v x y size drill own`, `p side own x y w h angle r drill`
    let txt = |kind: &str, q: &[f64]| {
        let mut o = kind.to_string();
        for x in q {
            o.push(' ');
            o.push_str(&n2(r2(*x)));
        }
        o
    };
    let f = |b: bool| if b { 1.0 } else { 0.0 };
    // (distance, highlighted, text)
    let mut items: Vec<(f64, bool, String)> = Vec::new();
    for t in b.tracks.iter().filter(|t| !t.pad && seg_in(t.a, t.b)) {
        let o = mine(t.net);
        items.push((
            dist(geom_mid(t.a, t.b)),
            o,
            txt("t", &[t.layer as f64, t.width, t.a[0], t.a[1], t.b[0], t.b[1], f(o)]),
        ));
    }
    for v in b.vias.iter().filter(|v| inside(v.at, v.size)) {
        let o = mine(v.net);
        items.push((dist(v.at), o, txt("v", &[v.at[0], v.at[1], v.size, v.drill, f(o)])));
    }
    for pd in b.pads.iter().filter(|pd| inside(pd.at, pd.size[0].max(pd.size[1]))) {
        // the pad's side: 0 top, 1 bottom, 2 through, 3 inner only
        let n = su.copper.len() as u16;
        let side = if pd.layers.len() > 1 {
            2
        } else if pd.layers.first() == Some(&0) {
            0
        } else if pd.layers.first() == Some(&(n.saturating_sub(1))) {
            1
        } else {
            3
        };
        // a rectangle with its corner radius, turned as KiCad turns it (custom pads: their anchor)
        let o = mine(pd.net);
        let r = crate::board::pad_corner_radius(pd);
        items.push((
            dist(pd.at),
            o,
            txt("p", &[side as f64, f(o), pd.at[0], pd.at[1], pd.size[0], pd.size[1], pd.angle, r, pd.drill]),
        ));
    }
    // the nearest first: at most DETAIL_ITEMS of the other nets, 1.5 times as many of the highlighted ones
    items.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut kept: Vec<String> = Vec::new();
    let (mut n_own, mut n_other) = (0, 0);
    for (_, is_own, v) in items {
        let n = if is_own { &mut n_own } else { &mut n_other };
        if *n < if is_own { 3 * DETAIL_ITEMS / 2 } else { DETAIL_ITEMS } {
            *n += 1;
            kept.push(v);
        }
    }
    json!({
        "focus": focus,
        "overview": {"lines": lines, "vias": ov_vias},
        "detail": {"window": [r2(win[0]), r2(win[1]), r2(win[2]), r2(win[3])], "items": kept},
        // [x, y, n]: the n-th finding of the group (0: a further place of one)
        "markers": marks.iter().map(|m| json!([m.0[0], m.0[1], m.1])).collect::<Vec<_>>(),
    })
}

fn geom_mid(a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    [0.5 * (a[0] + b[0]), 0.5 * (a[1] + b[1])]
}

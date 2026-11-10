#!/usr/bin/env python3
"""The viewer's results with an active stackup against the native CLI with `--stackup-preset`.

Usage: compare_viewer_cli.py <kisi native binary> <dist/kisi-examples.html> <examples dir> [--preset ID]
                             [--boards ts,vna]

For each example board (with web/samples/<key>.kisi.json) the viewer (headless Chromium) estimates the
impedances, runs the channel check, the DC voltage drop and the temperature with the board's stackup,
then turns the option "Fab stackups & tolerances" on, makes the fab template ID (default jlc6_7628) the
stackup of all views ("Use in all views") – the views compute again – and the same once more. The native
CLI runs `overview`, `check --all`, `power` and `thermal` without and with `--stackup-preset ID`
(formula level: the viewer's field solver does not run). Compared: per net class the length and the
share within ±5 % and the worst nets' impedances (overview), per channel the insertion loss at
Nyquist, return loss and eye height (check), per rail the largest voltage drop and the copper loss
(power; a rail without load current, which the viewer leaves out, is skipped), the board's hottest temperature (thermal; the CLI rounds it to 0.01 °C). Numbers agree to
1e-6 relative (or the CLI's rounding). Also prints how long the switch took. Exit code 1 on a
difference or a console error.
"""
import json, os, subprocess, sys
from playwright.sync_api import sync_playwright

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BOARDS = {"ts": "Thunderscope_Rev5.3", "vna": "VNA"}
BUSY = "document.getElementById('busy').hidden === true"


def cli(native, cmd, pcb, cfg, preset, extra=()):
    a = [native, cmd, pcb, "--config", cfg, *extra] + (["--stackup-preset", preset] if preset else [])
    r = subprocess.run(a, capture_output=True, text=True, timeout=3600)
    if r.returncode != 0:
        raise SystemExit(f"{' '.join(a)}: {r.stderr[-400:]}")
    return json.loads(r.stdout)


def close(a, b, rel=1e-6, abs_=1e-9):
    if a is None or b is None:
        return a is None and b is None
    return abs(a - b) <= max(abs_, rel * max(abs(a), abs(b)))


def viewer_state(pg):
    return pg.evaluate("""() => ({
      nets: S.an.nets.filter((n) => n.target).map((n) => ({ name: n.name, cls: n.class, length: n.length, within5: n.within5, z_mean: n.z_mean, z_min: n.z_min, z_max: n.z_max })),
      rows: (S.check ? S.check.rows : []).map((r) => ({ name: r.name, il: r.il_nyq_db, rl: r.rl_db, eye: r.eye ? r.eye_height : null })),
      rails: [...S.piRes].filter(([, r]) => r.res && !r.res.ground).map(([n, r]) => ({ rail: n, drop: r.res.max_drop_mv, loss_w: r.res.power_loss_w })),
      t_max: S.therm ? S.therm.t_max_c : null })""")


def compare(tag, v, ov, ck, pw, th, problems):
    n = 0
    bad = lambda what: problems.append(f"{tag}: {what}")
    # overview: per class from the viewer's nets, as kisi_core::service::overview
    for c in ov["classes"]:
        nets = [x for x in v["nets"] if x["cls"] == c["class"]]
        ln = sum(x["length"] for x in nets)
        ok = sum(x["length"] * (x["within5"] or 0) for x in nets)
        n += 2
        if not close(ln, c["length_mm"]) or not close(ok / ln if ln else 0.0, c["within_5pct"]):
            bad(f"class {c['class']}: length {ln:.6g} / {c['length_mm']:.6g}, within ±5 % {ok / ln if ln else 0:.6g} / {c['within_5pct']:.6g}")
        byname = {x["name"]: x for x in nets}
        for w in c["worst"]:
            x = byname.get(w["net"])
            n += 3
            if not x or not all(close(x[k], w[k]) for k in ("z_mean", "z_min", "z_max")):
                bad(f"net {w['net']}: viewer {x and (x['z_mean'], x['z_min'], x['z_max'])}, CLI {(w['z_mean'], w['z_min'], w['z_max'])}")
    # check: the rows the CLI lists
    rows = {r["name"]: r for r in v["rows"]}
    for r in ck["rows"]:
        name = r.get("lane") or r.get("net")
        x = rows.get(name)
        m = r["metrics"]
        n += 3
        if not x:
            bad(f"channel {name}: not in the viewer's check")
            continue
        for k, kv in (("il_nyq_db", "il"), ("rl_db", "rl"), ("eye_h", "eye")):
            if m.get(k) is not None and not close(m[k], x[kv]):
                bad(f"channel {name} {k}: viewer {x[kv]}, CLI {m[k]}")
    # power: every rail with a result
    rails = {r["rail"]: r for r in v["rails"]}
    for r in pw["rails"]:
        if r.get("max_drop_mv") is None:
            continue
        x = rails.get(r["rail"])
        if not x and abs(r["max_drop_mv"]) < 1e-6:
            continue  # a rail without load current: the viewer solves only rails with a load
        n += 2
        if not x or not close(x["drop"], r["max_drop_mv"]) or not close(x["loss_w"] * 1e3, r["copper_loss_mw"]):
            bad(f"rail {r['rail']}: viewer {x and (x['drop'], x['loss_w'] * 1e3)}, CLI {(r['max_drop_mv'], r['copper_loss_mw'])}")
    # thermal: the hottest board temperature (the CLI rounds to 0.01 °C)
    n += 1
    if v["t_max"] is None or abs(v["t_max"] - th["summary"]["t_board_max_c"]) > 0.0051:
        bad(f"thermal: hottest board temperature viewer {v['t_max']}, CLI {th['summary']['t_board_max_c']}")
    return n


def main():
    a = sys.argv[1:]
    if len(a) < 3:
        raise SystemExit(__doc__)
    native, html, ex = a[:3]
    preset = a[a.index("--preset") + 1] if "--preset" in a else "jlc6_7628"
    boards = a[a.index("--boards") + 1].split(",") if "--boards" in a else list(BOARDS)
    problems, errs = [], []
    with sync_playwright() as p:
        br = p.chromium.launch()
        for key in boards:
            stem = BOARDS[key]
            pcb, cfg = os.path.join(ex, stem + ".kicad_pcb"), os.path.join(ROOT, "web", "samples", key + ".kisi.json")
            pg = br.new_page(viewport={"width": 1500, "height": 950})
            pg.on("pageerror", lambda e: errs.append(f"pageerror: {e}"))
            pg.on("console", lambda m: errs.append(f"console {m.type}: {m.text}") if m.type == "error" else None)
            pg.goto("file://" + os.path.abspath(html))
            pg.wait_for_function(f"!!S.scene && {BUSY}", timeout=120000)
            pg.evaluate("() => { for (const k of Object.keys(localStorage)) if (k.startsWith('kisi.')) localStorage.removeItem(k); }")
            pg.click(f"#sample-{key}")
            pg.wait_for_function(f"S.name === '{stem}' && {BUSY}", timeout=180000)
            pg.click("#an-start"); pg.wait_for_function(f"S.full && {BUSY}", timeout=180000)
            pg.click("#view-ch"); pg.wait_for_timeout(300)
            pg.click("#ck-start"); pg.wait_for_function("!S.checkRunning && S.laneRes", timeout=1800000)
            pg.click("#view-pi"); pg.wait_for_timeout(300)
            pg.click("#pi-run"); pg.wait_for_function("S.piSolved && !S.piRunning", timeout=1800000)
            pg.click("#therm-run"); pg.wait_for_function("!!S.therm && !S.thermRunning", timeout=1800000)
            runs = [("board", None, viewer_state(pg), None)]
            # the template in all views: everything computed follows
            pg.click("#fab-on"); pg.wait_for_function(f"fabOn() && !S.switching && {BUSY}", timeout=600000)
            pg.click("#view-stk"); pg.wait_for_timeout(300)
            pg.select_option("#stk-add", f"preset:{preset}"); pg.wait_for_timeout(300)
            col = pg.evaluate(f"stkState().cols.find((c) => c.preset === '{preset}').id")
            pg.click(f"[data-stk-act='{col}']"); pg.wait_for_timeout(200)
            pg.wait_for_function("!S.switching", timeout=3600000)
            runs.append((preset, preset, viewer_state(pg), pg.evaluate("S.switchMs")))
            # back to the board's stackup (cached impedance) and to the template again
            pg.click("#stk-act-back"); pg.wait_for_timeout(200); pg.wait_for_function("!S.switching", timeout=3600000)
            back = pg.evaluate("S.switchMs")
            pg.close()
            for name, pr, v, ms in runs:
                tag = f"{stem} {name}"
                n = compare(tag, v, cli(native, "overview", pcb, cfg, pr), cli(native, "check", pcb, cfg, pr, ["--all"]),
                            cli(native, "power", pcb, cfg, pr), cli(native, "thermal", pcb, cfg, pr), problems)
                print(f"{tag}: {n} values compared, {len([x for x in problems if x.startswith(tag + ':')])} differ")
                if ms:
                    print(f"{tag}: switch {ms}")
            print(f"{stem} back to the board's stackup: {back}")
        br.close()
    for x in problems[:40]:
        print("DIFFERENT", x)
    for e in errs:
        print("console:", e)
    sys.exit(1 if problems or errs else 0)


if __name__ == "__main__":
    main()

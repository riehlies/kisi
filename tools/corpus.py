#!/usr/bin/env python3
"""The corpus run: kisi on open-source KiCad boards from data/corpus.json, fetched at pinned commits.

  corpus.py list [--set weekly|robustness]
      the boards of a set
  corpus.py fetch [--set S] [--only NAME,…] [--cache DIR] [--jobs 4]
      blobless, sparse git fetch of only the board file and its project file at the pinned commit into the cache
      (default $KISI_CORPUS_CACHE or ~/.cache/kisi-corpus; never inside the repository unless git-ignored). A board
      already there at the right commit is not fetched again. A repository that is gone, private or unreachable is
      reported and skipped – it never fails the command.
  corpus.py run <kisi> [--set S] [--only …] [--cache DIR] [--mjs dist/kisi.mjs] [--out FILE] [--timeout 600]
      runs the native kisi (with --mjs also node kisi.mjs --formula) per board: overview, interfaces, check, gaps,
      power, thermal (weekly: also crosstalk), all with --no-config (or the entry's kisi_json). A board that stops
      for an assembly variant (a documented stop) is run again with --variant default. Writes the results
      (default <cache>/results-<set>.json).
  corpus.py compare <kisi> [same options] [--results FILE] [--update] [--summary FILE.md]
      runs (or reads --results) and compares with validation/corpus/<set>.json: a new crash, panic, output that is
      not JSON, a changed exit status or a changed summary number fails; run times may vary (fails only above
      4 × the stored time + 10 s); boards that could not be fetched are listed, not failed. --update stores the
      results instead. --summary writes a Markdown table (the CI job summary).

What is stored per board and command: exit status, crash/panic, the error message, the run time, and summary numbers
(layers, stackup from the file, classes with a target, buses, lanes, check rows by colour, gap ids with item
counts, views, rails, crosstalk rows, thermal warnings) – kisi's own numbers, no board content (no net or part
names). Boards are fetched at test time and never committed.
"""
import argparse, concurrent.futures as cf, json, os, re, shutil, subprocess, sys, time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
LIST = os.path.join(ROOT, "data", "corpus.json")
STORED = os.path.join(ROOT, "validation", "corpus")
COMMANDS = ["overview", "interfaces", "check", "crosstalk", "gaps", "power", "thermal"]
SET_COMMANDS = {"weekly": COMMANDS, "robustness": [c for c in COMMANDS if c != "crosstalk"]}
# a run time counts as a regression only above this (seconds): runners differ a lot
TIME_FACTOR, TIME_SLACK = 4.0, 10.0


def load_list():
    with open(LIST) as f:
        return json.load(f)


def boards(args):
    d = load_list()
    sel = d["sets"][args.set]
    if args.only:
        want = set(args.only.split(","))
        sel = [b for b in sel if b["name"] in want]
        missing = want - {b["name"] for b in sel}
        if missing:
            sys.exit(f"not in the set {args.set}: {', '.join(sorted(missing))}")
    return sel


def cache_dir(args):
    c = args.cache or os.environ.get("KISI_CORPUS_CACHE") or os.path.join(os.path.expanduser("~"), ".cache", "kisi-corpus")
    os.makedirs(c, exist_ok=True)
    return os.path.abspath(c)


# ---- fetch -----------------------------------------------------------------------------------------------------
def sparse_pattern(path):
    """A path as a literal sparse-checkout pattern (gitignore syntax, anchored)."""
    p = re.sub(r"([\\*?\[\]!#])", r"\\\1", path)
    return "/" + p


def git(cwd, *a, timeout=600):
    env = dict(os.environ, GIT_TERMINAL_PROMPT="0", GIT_ASKPASS="true")
    return subprocess.run(["git", *a], cwd=cwd, capture_output=True, text=True, timeout=timeout, env=env)


def board_files(b):
    pcb = b["pcb"]
    return [pcb, pcb[: -len(".kicad_pcb")] + ".kicad_pro"]


def fetched(b, cache):
    d = os.path.join(cache, b["name"])
    pcb = os.path.join(d, b["pcb"])
    if not os.path.isfile(pcb):
        return False
    r = git(d, "rev-parse", "HEAD", timeout=30)
    return r.returncode == 0 and r.stdout.strip() == b["commit"]


def fetch_one(b, cache):
    d = os.path.join(cache, b["name"])
    t0 = time.time()
    if fetched(b, cache):
        return {"ok": True, "cached": True, "s": 0.0}
    shutil.rmtree(d, ignore_errors=True)
    os.makedirs(d)
    url = b.get("url") or f"https://github.com/{b['repo']}.git"
    steps = [("init", "-q", "."), ("remote", "add", "origin", url),
             ("sparse-checkout", "set", "--no-cone", *[sparse_pattern(p) for p in board_files(b)]),
             ("fetch", "-q", "--depth", "1", "--filter=blob:none", "origin", b["commit"]),
             ("-c", "advice.detachedHead=false", "checkout", "-q", "FETCH_HEAD")]
    for st in steps:
        try:
            r = git(d, *st)
        except subprocess.TimeoutExpired:
            return {"ok": False, "error": f"git {st[0]}: timeout", "s": round(time.time() - t0, 1)}
        if r.returncode != 0:
            msg = (r.stderr.strip().splitlines() or ["?"])[-1]
            return {"ok": False, "error": f"git {st[0] if st[0] != '-c' else 'checkout'}: {msg}", "s": round(time.time() - t0, 1)}
    if not os.path.isfile(os.path.join(d, b["pcb"])):
        return {"ok": False, "error": "board file not in the repository at this commit", "s": round(time.time() - t0, 1)}
    return {"ok": True, "cached": False, "s": round(time.time() - t0, 1)}


def cmd_fetch(args):
    cache = cache_dir(args)
    sel = boards(args)
    res = {}
    with cf.ThreadPoolExecutor(max_workers=args.jobs) as ex:
        futs = {ex.submit(fetch_one, b, cache): b for b in sel}
        for f in cf.as_completed(futs):
            b = futs[f]
            try:
                r = f.result()
            except Exception as e:  # never fail the whole fetch for one board
                r = {"ok": False, "error": str(e)}
            res[b["name"]] = r
            state = "cached" if r.get("cached") else ("ok" if r["ok"] else "FAILED: " + r["error"])
            print(f"{b['name']}: {state}" + (f" ({r['s']} s)" if r.get("s") else ""), flush=True)
    bad = [n for n, r in res.items() if not r["ok"]]
    with open(os.path.join(cache, f"fetch-{args.set}.json"), "w") as f:
        json.dump(res, f, indent=1)
    print(f"{len(sel) - len(bad)} of {len(sel)} boards available" + (f"; not fetched: {', '.join(sorted(bad))}" if bad else ""))
    return 0


# ---- run -------------------------------------------------------------------------------------------------------
def summarize(cmd, v):
    """kisi's own summary numbers of one command's JSON (no board content)."""
    k = {}
    if cmd == "overview":
        k["layers"] = v.get("layers")
        k["stackup_from_file"] = v.get("stackup_from_file")
        cls = v.get("classes") or []
        k["classes_with_target"] = len(cls)
        k["targets"] = sorted([c.get("target_ohm"), bool(c.get("diff")), c.get("target_source", "")[:20]] for c in cls)
        k["warnings"] = len(v.get("warnings") or [])
    elif cmd == "interfaces":
        buses = v.get("buses") or []
        k["buses"] = len(buses)
        k["lanes"] = sum(len(b.get("lanes") or []) for b in buses)
    elif cmd == "check":
        k["rows"] = v.get("summary")
    elif cmd == "crosstalk":
        k["victims"] = v.get("victims")
        k["critical"] = v.get("critical")
    elif cmd == "gaps":
        k["views"] = {n: x.get("state") for n, x in (v.get("views") or {}).items()}
        # items per gap; per net where the items group nets (nets-without-target: per class)
        k["gaps"] = {g["id"]: sum(len(i["nets"]) if isinstance(i, dict) and isinstance(i.get("nets"), list) else 1
                                  for i in g.get("items") or []) for g in v.get("gaps") or []}
    elif cmd == "power":
        k["rails"] = len(v.get("rails") or [])
        k["warnings"] = len(v.get("warnings") or [])
    elif cmd == "thermal":
        g = v.get("grid") or {}
        k["area_mm2"] = g.get("board_area_mm2")
        k["nodes"] = g.get("nodes")
        k["t_board_max_c"] = (v.get("summary") or {}).get("t_board_max_c")
        k["warnings"] = len(v.get("warnings") or [])
    return k


def run_cmd(argv, timeout):
    t0 = time.time()
    try:
        r = subprocess.run(argv, capture_output=True, text=True, timeout=timeout)
    except subprocess.TimeoutExpired:
        return {"rc": None, "s": round(time.time() - t0, 2), "timeout": True}
    out = {"rc": r.returncode, "s": round(time.time() - t0, 2)}
    if "panicked at" in r.stderr or r.returncode < 0 or r.returncode == 101:
        out["panic"] = True
        out["error"] = next((l for l in r.stderr.splitlines() if "panicked" in l or l.strip()), "")[:200]
    try:
        out["json"] = json.loads(r.stdout)
    except ValueError:
        out["json_error"] = True
        if not out.get("error"):
            out["error"] = (r.stderr.strip().splitlines() or [""])[-1][:200]
    if isinstance(out.get("json"), dict) and out["json"].get("error"):
        out["error"] = str(out["json"]["error"])[:300]
    # kisi's documented stop for assembly variants or forks (the message names --variant)
    out["variant_stop"] = r.returncode == 1 and "--variant" in r.stderr
    return out


def variant_stop(res):
    return res.get("variant_stop", False)


def run_board(b, cache, kisi, mjs, cmds, timeout):
    pcb = os.path.join(cache, b["name"], b["pcb"])
    cfg = ["--config", os.path.join(ROOT, b["kisi_json"])] if b.get("kisi_json") else ["--no-config"]
    extra = {"check": ["--all"]}
    variant = ["--variant", b["variant"]] if b.get("variant") else []
    out = {"commands": {}}
    tools = [("kisi", [kisi], [])] + ([("kisi.mjs", ["node", mjs], ["--formula"])] if mjs else [])
    for tool, base, flags in tools:
        for c in cmds:
            argv = base + [c, pcb] + cfg + variant + extra.get(c, []) + flags
            r = run_cmd(argv, timeout)
            if variant_stop(r) and not variant:
                # a documented stop: the board has variants or forks; compute it as it is
                out["variant_stop"] = True
                variant = ["--variant", "default"]
                r2 = run_cmd(base + [c, pcb] + cfg + variant + extra.get(c, []) + flags, timeout)
                r2["s"] = round(r2["s"] + r["s"], 2)
                r = r2
            rec = {k: r[k] for k in ("rc", "s", "panic", "timeout", "json_error", "error") if k in r}
            if isinstance(r.get("json"), dict) and r.get("rc") == 0:
                rec["key"] = summarize(c, r["json"])
            key = c if tool == "kisi" else f"mjs.{c}"
            out["commands"][key] = rec
    out["total_s"] = round(sum(x["s"] for x in out["commands"].values()), 1)
    return out


def kisi_version(kisi):
    """kisi's version (workspace Cargo.toml) and the commit of this checkout."""
    v = "?"
    try:
        with open(os.path.join(ROOT, "Cargo.toml")) as f:
            m = re.search(r'^version\s*=\s*"([^"]+)"', f.read(), re.M)
            v = m.group(1) if m else v
    except OSError:
        pass
    sha = subprocess.run(["git", "rev-parse", "--short", "HEAD"], cwd=ROOT, capture_output=True, text=True).stdout.strip()
    return v, sha


def do_run(args):
    cache = cache_dir(args)
    sel = boards(args)
    cmds = SET_COMMANDS[args.set]
    kisi = os.path.abspath(args.kisi)
    ver, sha = kisi_version(kisi)
    res = {"set": args.set, "kisi": ver, "commit": sha, "boards": {}, "unavailable": {}}
    fetch_info = {}
    p = os.path.join(cache, f"fetch-{args.set}.json")
    if os.path.exists(p):
        with open(p) as f:
            fetch_info = json.load(f)
    t0 = time.time()
    for i, b in enumerate(sel, 1):
        if not os.path.isfile(os.path.join(cache, b["name"], b["pcb"])):
            res["unavailable"][b["name"]] = fetch_info.get(b["name"], {}).get("error", "not fetched")
            print(f"[{i}/{len(sel)}] {b['name']}: not available ({res['unavailable'][b['name']]})", flush=True)
            continue
        r = run_board(b, cache, kisi, args.mjs and os.path.abspath(args.mjs), cmds, args.timeout)
        res["boards"][b["name"]] = r
        bad = [c for c, x in r["commands"].items() if x.get("rc") != 0]
        print(f"[{i}/{len(sel)}] {b['name']}: {r['total_s']} s" + (f"; exit ≠ 0: {', '.join(bad)}" if bad else "")
              + (" (variant default)" if r.get("variant_stop") else ""), flush=True)
    res["total_s"] = round(time.time() - t0, 1)
    out = args.out or os.path.join(cache, f"results-{args.set}.json")
    with open(out, "w") as f:
        json.dump(res, f, indent=1, ensure_ascii=False)
    print(f"results: {out} ({res['total_s']} s)")
    return res


def cmd_run(args):
    do_run(args)
    return 0


# ---- compare ---------------------------------------------------------------------------------------------------
def problems_of(rec):
    """Crashes and broken output of one command (whatever the stored result says)."""
    p = []
    if rec.get("panic"):
        p.append(f"panic: {rec.get('error', '')}")
    if rec.get("timeout"):
        p.append("timeout")
    if rec.get("json_error"):
        p.append(f"output is not JSON ({rec.get('error', '')})")
    return p


def diff_key(a, b, path, out):
    if isinstance(a, dict) and isinstance(b, dict):
        for k in sorted(set(a) | set(b)):
            if k not in b:
                out.append(f"{path}.{k}: removed")
            elif k not in a:
                out.append(f"{path}.{k}: new ({json.dumps(b[k], ensure_ascii=False)[:60]})")
            else:
                diff_key(a[k], b[k], f"{path}.{k}", out)
    elif isinstance(a, float) or isinstance(b, float):
        if not (isinstance(a, (int, float)) and isinstance(b, (int, float))) or abs(a - b) > max(1e-6, 1e-3 * max(abs(a), abs(b))):
            out.append(f"{path}: {a} -> {b}")
    elif a != b:
        out.append(f"{path}: {json.dumps(a, ensure_ascii=False)[:70]} -> {json.dumps(b, ensure_ascii=False)[:70]}")


def compare(stored, new):
    """(failures per board, notes per board)"""
    fails, notes = {}, {}
    for name, nb in new["boards"].items():
        f, n = [], []
        sb = (stored or {}).get("boards", {}).get(name)
        for c, rec in nb["commands"].items():
            f += [f"{c}: {x}" for x in problems_of(rec)]
            if sb is None or c not in sb["commands"]:
                continue
            old = sb["commands"][c]
            if old.get("rc") != rec.get("rc"):
                f.append(f"{c}: exit {old.get('rc')} -> {rec.get('rc')}" + (f" ({rec['error']})" if rec.get("error") else ""))
            d = []
            diff_key(old.get("key"), rec.get("key"), c, d)
            f += d
            if rec["s"] > TIME_FACTOR * old["s"] + TIME_SLACK:
                f.append(f"{c}: {old['s']} s -> {rec['s']} s")
            elif rec["s"] > 2 * old["s"] + 2:
                n.append(f"{c}: {old['s']} s -> {rec['s']} s")
        # the Node CLI (--formula) gives the same summary numbers as the native one
        for c, rec in nb["commands"].items():
            if c.startswith("mjs.") and c[4:] in nb["commands"]:
                nat = nb["commands"][c[4:]]
                if nat.get("rc") != rec.get("rc"):
                    f.append(f"{c}: exit {rec.get('rc')}, native {nat.get('rc')}")
                d = []
                diff_key(nat.get("key"), rec.get("key"), c, d)
                f += [f"{x} (native -> kisi.mjs)" for x in d]
        if sb is None:
            n.append("new board (no stored result)")
        elif bool(sb.get("variant_stop")) != bool(nb.get("variant_stop")):
            f.append(f"variant stop: {bool(sb.get('variant_stop'))} -> {bool(nb.get('variant_stop'))}")
        if f:
            fails[name] = f
        if n:
            notes[name] = n
    return fails, notes


def summary_md(new, fails, notes, stored):
    cmds = SET_COMMANDS[new["set"]]
    L = [f"### kisi corpus run: {new['set']} (kisi {new['kisi']}, {new.get('commit', '?')})", ""]
    nb = len(new["boards"])
    L.append(f"{nb} boards run in {new['total_s']} s; {len(fails)} with regressions; {len(new['unavailable'])} not available.")
    L.append("")
    L.append("| Board | " + " | ".join(cmds) + " | total s | result |")
    L.append("|---|" + "---|" * (len(cmds) + 2))
    for name, b in sorted(new["boards"].items()):
        cells = []
        for c in cmds:
            r = b["commands"].get(c, {})
            mark = "" if r.get("rc") == 0 else ("PANIC" if r.get("panic") else f"exit {r.get('rc')}")
            cells.append(f"{r.get('s', '–')}{(' ' + mark) if mark else ''}")
        res = "**regression**" if name in fails else ("variant default" if b.get("variant_stop") else "ok")
        L.append(f"| {name} | " + " | ".join(cells) + f" | {b['total_s']} | {res} |")
    if fails:
        L += ["", "#### Regressions", ""]
        for n, f in sorted(fails.items()):
            L.append(f"* **{n}**: " + "; ".join(f[:8]) + (f" … ({len(f)} in all)" if len(f) > 8 else ""))
    if notes:
        L += ["", "#### Notes (slower runs, new boards)", ""]
        for n, f in sorted(notes.items()):
            L.append(f"* {n}: " + "; ".join(f[:5]))
    if new["unavailable"]:
        L += ["", "#### Not available (network, repository gone or private)", ""]
        for n, e in sorted(new["unavailable"].items()):
            L.append(f"* {n}: {e}")
    gone = sorted(set((stored or {}).get("boards", {})) - set(new["boards"]) - set(new["unavailable"]))
    if gone and not new.get("partial"):
        L += ["", f"Stored but not in this run: {', '.join(gone)}"]
    return "\n".join(L) + "\n"


def strip_for_store(res):
    """What goes into validation/corpus: no paths, rounded times."""
    out = {"set": res["set"], "kisi": res["kisi"], "boards": {}}
    for n, b in sorted(res["boards"].items()):
        out["boards"][n] = b
    return out


def cmd_compare(args):
    stored_path = os.path.join(STORED, f"{args.set}.json")
    stored = None
    if os.path.exists(stored_path):
        with open(stored_path) as f:
            stored = json.load(f)
    if args.results:
        with open(args.results) as f:
            new = json.load(f)
    else:
        new = do_run(args)
    new["partial"] = bool(args.only)
    if args.update:
        keep = dict(stored["boards"]) if stored and args.only else {}
        keep.update(new["boards"])
        new_store = strip_for_store(dict(new, boards=keep))
        os.makedirs(STORED, exist_ok=True)
        with open(stored_path, "w") as f:
            json.dump(new_store, f, indent=1, ensure_ascii=False, sort_keys=True)
            f.write("\n")
        print(f"stored: {os.path.relpath(stored_path, ROOT)} ({len(keep)} boards)")
        fails, notes = {}, {}
    else:
        fails, notes = compare(stored, new)
    md = summary_md(new, fails, notes, stored)
    if args.summary:
        with open(args.summary, "a") as f:
            f.write(md)
    for n, f in sorted(fails.items()):
        print(f"REGRESSION {n}: " + "; ".join(f))
    for n, f in sorted(notes.items()):
        print(f"note {n}: " + "; ".join(f))
    for n, e in sorted(new["unavailable"].items()):
        print(f"not available {n}: {e}")
    print(f"{len(new['boards'])} boards, {len(fails)} with regressions, {len(new['unavailable'])} not available")
    return 1 if fails else 0


def cmd_list(args):
    for b in boards(args):
        print(f"{b['name']:40} {b['tier']} K{b.get('kicad', '?'):6} {b.get('layers', '?'):>3} L  {b['repo']}@{b['commit'][:8]}  {b['pcb']}")
    return 0


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    for name in ("list", "fetch", "run", "compare"):
        p = sub.add_parser(name)
        if name in ("run", "compare"):
            p.add_argument("kisi", help="the native kisi binary (target/release/kisi)")
            p.add_argument("--mjs", help="also run node kisi.mjs --formula")
            p.add_argument("--out", help="results file of the run")
            p.add_argument("--timeout", type=float, default=600, help="seconds per command")
        if name == "compare":
            p.add_argument("--results", help="compare this results file instead of running")
            p.add_argument("--update", action="store_true", help="store the results in validation/corpus/<set>.json")
            p.add_argument("--summary", help="append a Markdown summary to this file")
        p.add_argument("--set", default="weekly", choices=["weekly", "robustness"])
        p.add_argument("--only", help="comma-separated board names")
        p.add_argument("--cache", help="cache directory of the fetched boards")
        if name == "fetch":
            p.add_argument("--jobs", type=int, default=4)
    args = ap.parse_args()
    return {"list": cmd_list, "fetch": cmd_fetch, "run": cmd_run, "compare": cmd_compare}[args.cmd](args)


if __name__ == "__main__":
    sys.exit(main())

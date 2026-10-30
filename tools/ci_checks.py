#!/usr/bin/env python3
"""Small checks of the CI (.github/workflows/ci.yml); each one also runs locally.

  ci_checks.py changes <base> [<head>]   which check groups the changed files need (key=true|false lines; a file
                                         that matches no group, or any change to the CI itself, runs everything;
                                         `changes --full`: everything)
  ci_checks.py data                      every number in data/*.json has a source (in its object or a parent),
                                         every source has a known kind and a text, links are https
  ci_checks.py links                     relative links in the repository's Markdown files point to files that exist
  ci_checks.py cli-docs <kisi>           every `kisi <command>` and its --options in README, docs/, the skills and
                                         the contributor notes exist in `kisi help`
  ci_checks.py upstream [--latest]       third_party/js_2d_fields is upstream's tree at third_party/UPSTREAM_REV,
                                         unchanged (needs network); --latest: also report a newer upstream revision
  ci_checks.py sizes <dist dir>          the bundles against tools/size_budget.json (fails above the budget)
  ci_checks.py same <a> <b>              two builds of a bundle are byte-identical (reproducible bundles)
"""
import fnmatch, glob, hashlib, json, os, re, subprocess, sys, tempfile, urllib.parse

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

# ---- changes: file patterns per check group --------------------------------------------------------------------
CODE = ["crates/*", "Cargo.toml", "Cargo.lock", "data/*", "rust-toolchain*", "rustfmt.toml"]
BOARDS = ["web/samples/*", "tools/fetch_examples.sh", "tools/strip_pcb.py"]
SOLVER = ["web/solver/*", "third_party/*", "crates/kisi-si/src/xsgeom*", "crates/kisi-sparse/*", "tools/bundle.py",
          "tools/solver_bundle.py", "tools/esm_classic.py", "tools/forsten_sparse_shim.js", "tools/ref_forsten.mjs",
          "tools/compare_baseline.mjs", "tools/solve_xs.mjs", "tools/upstream_hooks.mjs", "tools/upstream_update.sh",
          "tools/validate_field.sh", "validation/*.json"]
GROUPS = {
    # Rust: formatting, clippy, tests, the CLI options named in the docs
    "rust": CODE,
    # the viewer in headless Chromium (needs the bundles)
    "viewer": CODE + BOARDS + ["web/*", "tools/bundle.py", "tools/solver_bundle.py", "tools/esm_classic.py",
                               "tools/check_viewer.py", "tools/test_units.mjs", "third_party/*",
                               # the plugin's page in the browser (tools/check_kicad_plugin.py)
                               "kicad-plugin/kisi_kicad.py", "kicad-plugin/kisi_action.py", "tools/check_kicad_plugin.py"],
    # Node CLI against the native CLI
    "cli": CODE + BOARDS + ["web/cli/*", "web/solver/*", "third_party/*", "tools/bundle_cli.py",
                            "tools/solver_bundle.py", "tools/compare_cli.py"],
    # 2D field solver against HFSS / baseline
    "solver": SOLVER,
    # stored results of the example boards and the 3D via references, example configurations
    "models": CODE + BOARDS + ["validation/golden/*", "validation/via_ref/*", "validation/fdtd/compare_kisi.py",
                               "validation/fdtd/kisi_board.py", "validation/fdtd/sparam.py",
                               "validation/fdtd/results/kisi_models.json", "tools/golden.py", "docs/examples/*"],
    # the channel export run in ngspice against kisi's S-parameters
    "export": CODE + BOARDS + ["validation/channel_export/*"],
    # the 3D reference page
    "refpage": ["validation/via_ref/*", "validation/fdtd/make_viewer.py", "validation/fdtd/viewer_template.html"],
    # Forstén's solver unchanged against upstream
    "upstream": ["third_party/*"],
    # the KiCad plugin: its unit tests, the package against KiCad's schemas (cheap, no bundles needed)
    "kicad": ["kicad-plugin/*", "tools/build_kicad_pcm.py", "tools/agent_guide.py", "claude-plugin/skills/kisi-prepare/*"],
    # Markdown links (cheap; also on every code change) and the documentation PDF
    "docs": ["*.md", "docs/*", "claude-plugin/*", ".claude-plugin/*", "LICENSE*", "tools/docs_pdf.py"],
}
# changes that need no check of their own (the cheap checks still run)
QUIET = ["docs/img/*", "tools/screenshots.py", ".gitignore", ".gitattributes", ".git-blame-ignore-revs", "web/pages/*",
         "validation/fdtd/*", "validation/*.md", "tools/tree_to_fields.py"]
# the corpus run (data/corpus.json is no data kisi reads): runs weekly and by hand, no check of its own on a push
CORPUS = ["data/corpus.json", "tools/corpus.py", "validation/corpus/*"]
# changes to the CI itself run everything
FULL = [".github/*", "tools/ci_checks.py", "tools/size_budget.json", "deny.toml"]


def match(path, pats):
    return any(fnmatch.fnmatch(path, p) for p in pats)


def changes(base, head="HEAD"):
    if base == "--full":
        return emit({g: True for g in GROUPS}, True)
    files = subprocess.run(["git", "diff", "--name-only", f"{base}...{head}"], capture_output=True, text=True,
                           cwd=ROOT, check=True).stdout.split()
    flags = {g: False for g in GROUPS}
    full, unknown = False, []
    for f in files:
        if match(f, FULL):
            full = True
        hit = [] if match(f, CORPUS) else [g for g, pats in GROUPS.items() if match(f, pats)]
        for g in hit:
            flags[g] = True
        if not hit and not match(f, QUIET + FULL + CORPUS):
            unknown.append(f)
    if unknown:
        full = True
    if full:
        flags = {g: True for g in flags}
    print(f"{len(files)} changed file(s) since {base[:12]}" + (f"; not in any group: {' '.join(unknown[:8])}" if unknown else ""),
          file=sys.stderr)
    return emit(flags, full)


def emit(flags, full):
    lines = [f"{g}={'true' if v else 'false'}" for g, v in flags.items()] + [f"full={'true' if full else 'false'}"]
    out = os.environ.get("GITHUB_OUTPUT")
    if out:
        with open(out, "a") as fh:
            fh.write("\n".join(lines) + "\n")
    print("\n".join(lines))
    return 0


# ---- data: values with sources ----------------------------------------------------------------------------------
KINDS = {"standard", "spec", "guideline", "datasheet", "measurement", "kisi", "assumption"}


def data():
    bad = []
    for f in sorted(glob.glob(os.path.join(ROOT, "data", "*.json"))):
        name = os.path.relpath(f, ROOT)
        with open(f) as fh:
            d = json.load(fh)

        def walk(x, path, covered):
            if isinstance(x, dict):
                src = x.get("sources")
                if src is not None:
                    if not isinstance(src, list) or not src:
                        bad.append(f"{name} {path}.sources: empty or not a list")
                    for i, s in enumerate(src or []):
                        if s.get("kind") not in KINDS:
                            bad.append(f"{name} {path}.sources[{i}]: kind {s.get('kind')!r} (one of {', '.join(sorted(KINDS))})")
                        if not str(s.get("text", "")).strip():
                            bad.append(f"{name} {path}.sources[{i}]: no text")
                        if "url" in s and not str(s["url"]).startswith("https://"):
                            bad.append(f"{name} {path}.sources[{i}]: url is not https")
                covered = covered or bool(src)
                nums = [k for k, v in x.items() if isinstance(v, (int, float)) and not isinstance(v, bool) and k != "version"]
                if nums and not covered:
                    bad.append(f"{name} {path}: numbers without a source ({', '.join(nums[:5])})")
                for k, v in x.items():
                    if k != "sources":
                        walk(v, f"{path}.{k}", covered)
            elif isinstance(x, list):
                for i, v in enumerate(x):
                    walk(v, f"{path}[{i}]", covered)

        walk(d, "$", False)
    for b in bad:
        print("PROBLEM:", b)
    print(f"data: {'ok' if not bad else f'{len(bad)} problem(s)'}")
    return 1 if bad else 0


# ---- links: relative links in Markdown --------------------------------------------------------------------------
def tracked_md():
    out = subprocess.run(["git", "ls-files", "*.md"], capture_output=True, text=True, cwd=ROOT, check=True).stdout.split()
    return [f for f in out if not f.startswith("third_party/")]


LINK = re.compile(r"!?\[[^\]]*\]\(([^)\s]+)(?:\s+\"[^\"]*\")?\)|<(?:img|a)\s[^>]*(?:src|href)=\"([^\"]+)\"")


def links():
    bad = []
    for f in tracked_md():
        text = open(os.path.join(ROOT, f), encoding="utf-8").read()
        text = re.sub(r"```.*?```", "", text, flags=re.S)  # code blocks
        for m in LINK.finditer(text):
            target = m.group(1) or m.group(2)
            if re.match(r"^[a-z]+:", target) or target.startswith("#"):
                continue
            path = urllib.parse.unquote(target.split("#")[0].split("?")[0])  # GitHub needs %20 for spaces
            if not path:
                continue
            full = os.path.normpath(os.path.join(ROOT, os.path.dirname(f), path))
            if not os.path.exists(full):
                bad.append(f"{f}: link to {target} – no such file")
    for b in bad:
        print("PROBLEM:", b)
    print(f"links: {'ok' if not bad else f'{len(bad)} broken'}")
    return 1 if bad else 0


# ---- cli-docs: commands and options named in the docs exist -----------------------------------------------------
CALL = re.compile(r"(?:^|[\s`(])(?:node\s+\S*kisi\.mjs|kisi)\s+([a-z][a-z-]*)([^`\n]*)")


def cli_docs(kisi):
    help_text = subprocess.run([kisi, "help"], capture_output=True, text=True, check=True).stdout
    commands = set(re.findall(r"^  ([a-z][a-z-]*)", help_text, re.M)) | {"help"}
    options = set(re.findall(r"--[a-z][a-z-]*", help_text)) | {"--version", "--help"}
    # research notes describe planned commands and options
    files = [f for f in tracked_md() if not f.startswith(("docs/research/", "validation/"))]
    bad = []
    for f in files:
        for n, line in enumerate(open(os.path.join(ROOT, f), encoding="utf-8"), 1):
            for span in re.findall(r"`([^`]+)`", line) or ([line] if line.startswith(("    ", "kisi ", "node ")) else []):
                for m in CALL.finditer(span):
                    cmd, rest = m.group(1), m.group(2)
                    if cmd not in commands:
                        # prose such as "kisi analyses": only flag words that look like a command call
                        if re.match(r"^\s*(<|\S+\.kicad_pcb|path/|board|\[)", rest) or "--" in rest:
                            bad.append(f"{f}:{n}: unknown command `kisi {cmd}`")
                        continue
                    for opt in re.findall(r"(?<![\w-])--[a-z][a-z-]*", rest):
                        if opt not in options:
                            bad.append(f"{f}:{n}: `kisi {cmd}` – option {opt} not in `kisi help`")
    for b in sorted(set(bad)):
        print("PROBLEM:", b)
    print(f"cli-docs: {len(files)} files, {'ok' if not bad else f'{len(set(bad))} problem(s)'}")
    return 1 if bad else 0


# ---- upstream: Forstén's solver unchanged -----------------------------------------------------------------------
def tree_hashes(root):
    out = {}
    for dp, _, fs in os.walk(root):
        for fn in fs:
            p = os.path.join(dp, fn)
            out[os.path.relpath(p, root)] = hashlib.sha256(open(p, "rb").read()).hexdigest()
    return out


def upstream(latest):
    rev = open(os.path.join(ROOT, "third_party", "UPSTREAM_REV")).read().strip()
    url = "https://github.com/Ttl/js_2d_fields.git"
    tmp = tempfile.mkdtemp()
    subprocess.run(["git", "clone", "-q", url, os.path.join(tmp, "repo")], check=True)
    subprocess.run(["git", "-C", os.path.join(tmp, "repo"), "checkout", "-q", rev], check=True)
    os.makedirs(os.path.join(tmp, "tree"))
    arch = subprocess.run(["git", "-C", os.path.join(tmp, "repo"), "archive", "HEAD"], capture_output=True, check=True).stdout
    subprocess.run(["tar", "-x", "-C", os.path.join(tmp, "tree")], input=arch, check=True)
    want, have = tree_hashes(os.path.join(tmp, "tree")), tree_hashes(os.path.join(ROOT, "third_party", "js_2d_fields"))
    bad = sorted(set(want) ^ set(have)) + sorted(k for k in set(want) & set(have) if want[k] != have[k])
    for b in bad:
        print("PROBLEM: third_party/js_2d_fields/" + b, "differs from upstream" if b in want and b in have else
              ("missing" if b in want else "not in upstream"))
    print(f"upstream: {len(have)} files at {rev[:12]}, {'unchanged' if not bad else f'{len(bad)} difference(s)'}")
    if latest:
        head = subprocess.run(["git", "-C", os.path.join(tmp, "repo"), "rev-parse", "origin/HEAD"], capture_output=True,
                              text=True, check=True).stdout.strip()
        if head != rev:
            n = subprocess.run(["git", "-C", os.path.join(tmp, "repo"), "rev-list", "--count", f"{rev}..{head}"],
                               capture_output=True, text=True).stdout.strip()
            msg = f"js_2d_fields has {n} newer commit(s) upstream ({head[:12]}); update with tools/upstream_update.sh"
            print(msg)
            if os.environ.get("GITHUB_ACTIONS"):
                print(f"::notice title=upstream::{msg}")
        else:
            print("upstream: the vendored revision is upstream's latest")
    return 1 if bad else 0


# ---- sizes and reproducibility ----------------------------------------------------------------------------------
def sizes(dist):
    budget = json.load(open(os.path.join(ROOT, "tools", "size_budget.json")))["max_bytes"]
    bad, rows = 0, []
    for name, limit in budget.items():
        p = os.path.join(dist, name)
        if not os.path.exists(p):
            continue
        n = os.path.getsize(p)
        ok = n <= limit
        bad += not ok
        rows.append(f"| {name} | {n / 1e6:.2f} MB | {limit / 1e6:.2f} MB | {'ok' if ok else 'ABOVE'} |")
        print(f"{name}: {n} bytes (budget {limit}){'' if ok else ' – above the budget'}")
    s = os.environ.get("GITHUB_STEP_SUMMARY")
    if s:
        with open(s, "a") as fh:
            fh.write("\n".join(["### Bundle sizes", "", "| file | size | budget | |", "|---|---|---|---|"] + rows) + "\n")
    if bad:
        print("A bundle grew above tools/size_budget.json: if intended, raise the budget in the same commit.")
    return 1 if bad else 0


def same(a, b):
    ha, hb = (hashlib.sha256(open(p, "rb").read()).hexdigest() for p in (a, b))
    print(f"{os.path.basename(a)}: {'identical' if ha == hb else 'DIFFERENT'} ({ha[:16]} / {hb[:16]})")
    return 0 if ha == hb else 1


def main():
    a = sys.argv[1:]
    if not a:
        raise SystemExit(__doc__)
    cmd = a[0]
    if cmd == "changes" and len(a) in (2, 3):
        return changes(*a[1:])
    if cmd == "data":
        return data()
    if cmd == "links":
        return links()
    if cmd == "cli-docs" and len(a) == 2:
        return cli_docs(a[1])
    if cmd == "upstream":
        return upstream("--latest" in a)
    if cmd == "sizes" and len(a) == 2:
        return sizes(a[1])
    if cmd == "same" and len(a) == 3:
        return same(a[1], a[2])
    raise SystemExit(__doc__)


if __name__ == "__main__":
    sys.exit(main())

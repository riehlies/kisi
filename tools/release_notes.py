#!/usr/bin/env python3
"""The field pages' build info and the source offer in the release notes (.github/workflows/release.yml).

Usage: release_notes.py build-info <BUILD_INFO.json> <kisi-advanced.html> <kisi-advanced-lite.html>   > build-info.json
       release_notes.py notes <template.md> <build-info.json> <tag>                                    > notes.md

build-info: the fork's pin and kisi's patches (with their SHA-256), what the full-wave modules were built from
(tools/build_forsten_fullwave.sh's BUILD_INFO.json: upstream revision, submodules, OCCT, Emscripten SDK, the modules'
SHA-256) and each field page's size, build and SHA-256 (the release workflow builds each twice and compares them).
notes: .github/release-notes.md.in with those values – the corresponding source of the GPL-3.0 field pages.
"""
import hashlib, json, os, re, sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PATCHES = os.path.join(ROOT, "web", "advanced", "fork-patches")


def sha(path):
    return hashlib.sha256(open(path, "rb").read()).hexdigest()


def build_info(fw_info, *pages):
    pin = json.load(open(os.path.join(ROOT, "third_party", "js_2d_fields_fork.json"), encoding="utf-8"))
    fw = json.load(open(fw_info, encoding="utf-8"))
    out = {"about": "What kisi-advanced.html and kisi-advanced-lite.html were built from (tools/release_notes.py)",
           "kisi_commit": os.environ.get("GITHUB_SHA", ""),
           "fork": {"repo": pin["repo"], "commit": pin["commit"], "upstream_base": pin["upstream_base"],
                    "patches": {p: sha(os.path.join(PATCHES, p)) for p in sorted(os.listdir(PATCHES)) if p.endswith(".patch")}},
           "fullwave": {k: fw.get(k) for k in ("upstream", "upstream_rev", "submodules", "emsdk", "emcc", "occt", "built", "files")},
           "pages": {}}
    for p in pages:
        head = open(p, encoding="utf-8").read(2000)
        m = re.search(r"kisi-advanced\.html \((full|lite) build\)", head)
        out["pages"][os.path.basename(p)] = {"bytes": os.path.getsize(p), "build": m.group(1) if m else None, "sha256": sha(p)}
    return out


def notes(template, info_path, tag):
    info = json.load(open(info_path, encoding="utf-8"))
    f, fw = info["fork"], info["fullwave"]
    repo = os.environ.get("GITHUB_REPOSITORY", "riehlies/kisi")
    return open(template, encoding="utf-8").read().format(
        fork_repo=f["repo"], fork_repo_name=f["repo"].split("github.com/")[-1], fork_commit=f["commit"], fork_commit_short=f["commit"][:7],
        patches=", ".join(f"`{p}`" for p in f["patches"]), kisi_tree=f"https://github.com/{repo}/tree/{tag}",
        upstream_rev=fw.get("upstream_rev") or "", upstream_rev_short=(fw.get("upstream_rev") or "")[:7],
        submodules=", ".join(f"{k} `{str(v)[:12]}`" for k, v in sorted((fw.get("submodules") or {}).items())),
        occt=fw.get("occt") or "", emsdk=fw.get("emsdk") or "",
        page_hashes="; ".join(f"`{n}` {p['sha256']}" for n, p in info["pages"].items()))


if __name__ == "__main__":
    a = sys.argv[1:]
    if len(a) >= 2 and a[0] == "build-info":
        print(json.dumps(build_info(*a[1:]), indent=1))
    elif len(a) == 4 and a[0] == "notes":
        sys.stdout.write(notes(*a[1:]))
    else:
        raise SystemExit(__doc__)

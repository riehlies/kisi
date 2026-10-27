#!/usr/bin/env python3
"""Build kisi's KiCad plugin package (KiCad 9/10 IPC plugin) from kicad-plugin/ and the viewer bundle.

Usage: build_kicad_pcm.py <kisi.html> <kisi.mjs> [--out DIR] [--version X.Y.Z]

Writes into DIR (default dist/):
  kisi-kicad-<version>.zip                 the package: "Install from File…" in KiCad's Plugin and Content Manager,
                                           and the GitHub release asset that the PCM repository entry downloads
  kisi-kicad-<version>.metadata.json       the entry for KiCad's PCM repository (gitlab.com/kicad/addons/metadata,
                                           packages/com.github.riehlies.kisi/metadata.json): the package's metadata
                                           plus download_url, download_sha256, download_size and install_size
  pcm/com.github.riehlies.kisi/            the same entry and the 64×64 icon, laid out as in the merge request

The zip (layout as KiCad's PCM extracts it: <zip>/plugins/… → 3rdparty/plugins/com_github_riehlies_kisi/…, where
KiCad's plugin manager finds plugin.json; docs/research/kicad-plugin.md):
  metadata.json                package metadata, one version, no download fields (kicad-plugin/metadata.json)
  plugins/plugin.json          the IPC plugin and its toolbar action
  plugins/requirements.txt     kicad-python, installed by KiCad into the plugin's Python environment
  plugins/kisi_action.py       entry point (KiCad API) · plugins/kisi_kicad.py (the page, standard library only)
  plugins/kisi_build.json      version and the viewer's SHA-256
  plugins/icons/*.png          toolbar icons (24 px, 48 px; light and dark)
  plugins/viewer/kisi.html     the viewer of this version (the hand-over marker of web/index.html inside)
  plugins/agent/KISI-AGENT-GUIDE.md  the agent guide for any assistant (tools/agent_guide.py, from the kisi-prepare
                               skill; the viewer's agent card copies it)
  plugins/cli/kisi.mjs         the command line of this version (Node.js ≥ 18), for the agent path – the viewer's
                               agent card names it with its installed path
  plugins/LICENSE              GPL-3.0
  resources/icon.png           64×64 package icon

Checks (fail the build): metadata against KiCad 10's PCM schema v2 and KiCad 9's schema v1 (Install from File
validates against them), plugin.json against KiCad's plugin schema and identifier rule, the files plugin.json names
exist, the viewer has one hand-over marker and this version, the CLI this version. Reproducible: fixed order, timestamps and modes.
"""
import argparse, hashlib, io, json, os, re, sys, zipfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import agent_guide  # noqa: E402

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SRC = os.path.join(ROOT, "kicad-plugin")
SCHEMAS = os.path.join(SRC, "tests", "schemas")
REPO_URL = "https://github.com/riehlies/kisi"
MARKER = "null; /*@kicad-board*/"
# KiCad's API_PLUGIN::IsValidIdentifier (common/api/api_plugin.cpp): reverse DNS, at least three parts
KICAD_ID = re.compile(r"^[a-zA-Z]{2,}(\.([a-zA-Z0-9][a-zA-Z0-9-]*[a-zA-Z0-9]|[a-zA-Z0-9])){2,}$")
DATE = (1980, 1, 1, 0, 0, 0)  # zip timestamps: fixed, so the package is reproducible
PLUGIN_FILES = ["plugin.json", "requirements.txt", "kisi_action.py", "kisi_kicad.py", "agent/KISI-AGENT-GUIDE.md",
                "icons/kisi-24.png", "icons/kisi-48.png", "icons/kisi-24-dark.png", "icons/kisi-48-dark.png"]


def rd(path, mode="r"):
    with open(path, mode, **({} if "b" in mode else {"encoding": "utf-8"})) as f:
        return f.read()


def wr(path, data):
    with open(path, "wb" if isinstance(data, bytes) else "w", **({} if isinstance(data, bytes) else {"encoding": "utf-8"})) as f:
        f.write(data)


def cargo_version():
    cargo = rd(os.path.join(ROOT, "Cargo.toml"))
    return re.search(r'^version\s*=\s*"([^"]+)"', cargo, re.M).group(1)


def schema(name):
    return json.loads(rd(os.path.join(SCHEMAS, name)))


def validate(instance, sch, what):
    """Problems of instance against a draft-07 schema (jsonschema), as strings."""
    import jsonschema  # pip install jsonschema
    v = jsonschema.Draft7Validator(sch)
    return [f"{what}: {'/'.join(map(str, e.absolute_path)) or '/'}: {e.message}" for e in sorted(v.iter_errors(instance), key=str)]


def check_plugin_json(pj):
    bad = validate(pj, schema("api.v1.schema.json"), "plugin.json")
    if not KICAD_ID.match(pj.get("identifier", "")):
        bad.append(f"plugin.json: identifier {pj.get('identifier')!r} fails KiCad's rule (reverse DNS, three parts)")
    if pj.get("runtime", {}).get("type") != "python":
        bad.append("plugin.json: runtime.type must be python")
    for a in pj.get("actions", []):
        for f in [a.get("entrypoint", "")] + a.get("icons-light", []) + a.get("icons-dark", []):
            if f not in PLUGIN_FILES:
                bad.append(f"plugin.json: {f} is not among the packaged files")
            elif not os.path.isfile(os.path.join(SRC, f)):
                bad.append(f"plugin.json: {f} does not exist in kicad-plugin/")
        if "pcb" not in a.get("scopes", []):
            bad.append(f"plugin.json: action {a.get('identifier')} has no pcb scope (KiCad 9/10 run IPC plugins in the PCB editor only)")
    return bad


def check_metadata(md, submission=False):
    """The package metadata against KiCad 10's v2 schema and KiCad 9's v1 schema; the PCM's own rules."""
    bad = validate(md, schema("pcm.v2.schema.json"), "metadata v2 (KiCad 10)")
    bad += validate(md, schema("pcm.v1.kicad9.schema.json"), "metadata v1 (KiCad 9)")
    if len(md.get("versions", [])) != 1:
        bad.append("metadata: exactly one version (KiCad's Install from File)")
    v = (md.get("versions") or [{}])[0]
    dl = [k for k in ("download_url", "download_sha256", "download_size", "install_size") if k in v]
    if submission and len(dl) != 4:
        bad.append("submission metadata: download_url, download_sha256, download_size and install_size are needed")
    if not submission and dl:
        bad.append(f"package metadata: {', '.join(dl)} belong only in the repository entry, not in the zip")
    if v.get("runtime") != "ipc":
        bad.append("metadata: runtime must be ipc")
    if len(md.get("description", "")) > 150:
        bad.append("metadata: description longer than 150 characters (KiCad's add-on rules)")
    return bad


def check_viewer(html, version, test_viewer=False):
    bad = []
    n = html.count(MARKER)
    if n != 1:
        bad.append(f"viewer: {n} hand-over markers ({MARKER!r}) instead of one – built from web/index.html of this branch?")
    if not test_viewer and f'<span class="ver">v{version}</span>' not in html:
        bad.append(f"viewer: not version {version} (bundle it from this tree)")
    return bad


def check_cli(js, version, test_viewer=False):
    if not test_viewer and f'VERSION = "{version}"' not in js:
        return [f"cli: kisi.mjs is not version {version} (bundle it from this tree: tools/bundle_cli.py)"]
    return []


def zip_bytes(entries):
    """entries: [(name, bytes)] → a reproducible zip."""
    buf = io.BytesIO()
    with zipfile.ZipFile(buf, "w") as z:
        for name, data in entries:
            zi = zipfile.ZipInfo(name, DATE)
            zi.compress_type = zipfile.ZIP_DEFLATED
            zi.create_system = 3  # Unix, so the mode below is read the same everywhere
            zi.external_attr = 0o644 << 16
            z.writestr(zi, data, compresslevel=9)
    return buf.getvalue()


def build(viewer, cli, out, version=None, test_viewer=False):
    """Build the package; returns {zip, metadata, sha256, size, install_size}. Raises SystemExit on problems."""
    version = version or cargo_version()
    pj = json.loads(rd(os.path.join(SRC, "plugin.json")))
    md = json.loads(rd(os.path.join(SRC, "metadata.json")))
    md["versions"][0]["version"] = version
    html, js = rd(viewer), rd(cli)
    bad = check_plugin_json(pj) + check_metadata(md) + check_viewer(html, version, test_viewer) + check_cli(js, version, test_viewer)
    guide = agent_guide.HEAD + agent_guide.body()
    if rd(os.path.join(SRC, "agent", "KISI-AGENT-GUIDE.md")) != guide:
        bad.append("kicad-plugin/agent/KISI-AGENT-GUIDE.md is not up to date – run tools/agent_guide.py")
    bad += agent_guide.problems(agent_guide.body())
    if md["identifier"] != pj["identifier"]:
        bad.append("metadata.json and plugin.json: different identifiers")
    if bad:
        raise SystemExit("build_kicad_pcm: " + "\n  ".join(["problems:"] + bad))
    hb = html.encode("utf-8")
    jb = js.encode("utf-8")
    info = {"version": version, "viewer_sha256": hashlib.sha256(hb).hexdigest(), "cli_sha256": hashlib.sha256(jb).hexdigest()}
    entries = [("metadata.json", (json.dumps(md, indent=2, ensure_ascii=False) + "\n").encode("utf-8"))]
    plugin = [(f, rd(os.path.join(SRC, f), "rb")) for f in PLUGIN_FILES]
    plugin += [("kisi_build.json", (json.dumps(info, indent=2) + "\n").encode()), ("viewer/kisi.html", hb), ("cli/kisi.mjs", jb),
               ("LICENSE", rd(os.path.join(ROOT, "LICENSE"), "rb"))]
    entries += sorted(("plugins/" + f, d) for f, d in plugin)
    entries.append(("resources/icon.png", rd(os.path.join(SRC, "resources", "icon.png"), "rb")))
    data = zip_bytes(entries)
    name = f"kisi-kicad-{version}.zip"
    os.makedirs(out, exist_ok=True)
    zpath = os.path.join(out, name)
    wr(zpath, data)
    sha = hashlib.sha256(data).hexdigest()
    install = sum(len(d) for n, d in entries if n.startswith(("plugins/", "resources/")))  # what the PCM extracts
    sub = json.loads(json.dumps(md))
    sub["versions"][0].update({"download_url": f"{REPO_URL}/releases/download/v{version}/{name}", "download_sha256": sha,
                               "download_size": len(data), "install_size": install})
    bad = check_metadata(sub, submission=True)
    if bad:
        raise SystemExit("build_kicad_pcm: " + "\n  ".join(["problems in the submission metadata:"] + bad))
    stext = json.dumps(sub, indent=2, ensure_ascii=False) + "\n"
    mpath = os.path.join(out, f"kisi-kicad-{version}.metadata.json")
    wr(mpath, stext)
    pdir = os.path.join(out, "pcm", md["identifier"])
    os.makedirs(pdir, exist_ok=True)
    wr(os.path.join(pdir, "metadata.json"), stext)
    wr(os.path.join(pdir, "icon.png"), rd(os.path.join(SRC, "resources", "icon.png"), "rb"))
    return {"zip": zpath, "metadata": mpath, "sha256": sha, "size": len(data), "install_size": install, "entries": [n for n, _ in entries]}


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("viewer", help="the viewer bundle dist/kisi.html of this tree")
    ap.add_argument("cli", help="the command line bundle dist/kisi.mjs of this tree")
    ap.add_argument("--out", default=os.path.join(ROOT, "dist"))
    ap.add_argument("--version", help="default: the workspace version in Cargo.toml")
    a = ap.parse_args()
    r = build(a.viewer, a.cli, a.out, a.version)
    print(f"{r['zip']}: {r['size'] / 1e6:.2f} MB (installed {r['install_size'] / 1e6:.2f} MB), sha256 {r['sha256']}")
    print(f"{r['metadata']}: the PCM repository entry (also in {os.path.join(a.out, 'pcm')})")
    return 0


if __name__ == "__main__":
    sys.exit(main())

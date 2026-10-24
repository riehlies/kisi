#!/usr/bin/env python3
"""Render the user documentation in docs/ into one PDF with a title page and a table of contents.

Usage: python3 tools/docs_pdf.py [out.pdf]            (default dist/kisi-docs.pdf)
       python3 tools/docs_pdf.py --html out.html      (the intermediate HTML, for a look in a browser)

Markdown → HTML with python-markdown (tables, fenced code), → PDF with the headless Chromium of
Playwright (the same browser tools/check_viewer.py uses):
    python3 -m pip install markdown playwright pypdf && python3 -m playwright install chromium

The pages and their order are ORDER below; every Markdown file directly in docs/ must be listed there
or in SKIP, so that a new page is not forgotten. Links between the pages become links inside the PDF,
links to other files of the repository point to GitHub; a link to a missing file or heading is an
error (the tool doubles as a link check of docs/). Images are embedded. With pypdf installed, the
table of contents gets page numbers (a second pass finds the headings in the first PDF's text).
The date on the title page is the date of the last commit, so a rebuild of the same commit gives the
same text.
"""
import base64, html, os, re, subprocess, sys, tempfile, urllib.parse

ROOT = os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
DOCS = os.path.join(ROOT, "docs")
REPO_URL = "https://github.com/riehlies/kisi"
ORDER = ["getting-started.md", "preparing-a-board.md", "reference.md", "models.md", "cli.md", "agents.md"]
SKIP = {"README.md"}  # the index of docs/ on GitHub; the PDF has its own table of contents
TOC_END = "Links in this PDF lead to the pages and headings; links to other files open them on GitHub."


def fail(msg):
    sys.exit(f"docs_pdf: {msg}")


def github_slug(text):
    """The anchor GitHub gives a heading: lower case, punctuation removed, spaces → '-'."""
    t = re.sub(r"<[^>]+>", "", text)
    t = html.unescape(t).strip().lower()
    t = re.sub(r"[^\w\- ]", "", t, flags=re.UNICODE)
    return t.replace(" ", "-")


def doc_id(name):
    return "doc-" + name[:-3]


def version():
    m = re.search(r'^version\s*=\s*"([^"]+)"', open(os.path.join(ROOT, "Cargo.toml"), encoding="utf-8").read(), re.M)
    return m.group(1) if m else "?"


def commit():
    try:
        out = subprocess.run(["git", "log", "-1", "--format=%h %cs"], cwd=ROOT, capture_output=True, text=True, check=True).stdout.split()
        return out[0], out[1]
    except (OSError, subprocess.CalledProcessError, IndexError):
        return "", ""


def blank_before_lists(text):
    """GitHub starts a list right after a paragraph line; python-markdown needs a blank line before it."""
    out, fence, prev = [], False, ""
    for line in text.split("\n"):
        if line.lstrip().startswith("```"):
            fence = not fence
        item = re.match(r"^\s*([-*+]|\d+\.)\s", line)
        prev_item = re.match(r"^\s*([-*+]|\d+\.)\s", prev) or prev.startswith((" ", "\t"))
        if not fence and item and prev.strip() and not prev_item and not line.startswith(" "):
            out.append("")
        out.append(line)
        prev = line if not fence else ""
    return "\n".join(out)


def convert(name, text):
    """One page: Markdown → HTML body, heading ids prefixed with the page, links and images rewritten.
    Returns (html, headings [(level, id, text)], links to check [(href, where)])."""
    import markdown
    for n, line in enumerate(text.split("\n"), 1):
        if re.match(r"^\s+```", line):
            fail(f"{name}:{n}: an indented code fence (inside a list item) does not render in the PDF – put the block after the list")
    text = blank_before_lists(text)
    body = markdown.markdown(text, extensions=["tables", "fenced_code", "sane_lists"], output_format="html")
    pid = doc_id(name)
    heads, seen = [], {}

    def head(m):
        lvl, inner = int(m.group(1)), m.group(2)
        slug = github_slug(inner)
        n = seen.get(slug, 0)
        seen[slug] = n + 1
        hid = f"{pid}--{slug}" + (f"-{n}" if n else "")
        heads.append((lvl, hid, html.unescape(re.sub(r"<[^>]+>", "", inner))))
        return f'<h{lvl} id="{hid}">{inner}</h{lvl}>'

    body = re.sub(r"<h([1-4])>(.*?)</h\1>", head, body, flags=re.S)
    links = []

    def link(m):
        href = html.unescape(m.group(1))
        if re.match(r"^[a-z]+:", href):
            return m.group(0)
        path, _, frag = href.partition("#")
        path = urllib.parse.unquote(path)
        if not path:  # an anchor on this page
            target = f"#{pid}--{frag}"
            links.append((target, f"{name}: #{frag}"))
            return f'href="{target}"'
        rel = os.path.normpath(os.path.join(os.path.dirname(os.path.join("docs", name)), path)).replace(os.sep, "/")
        full = os.path.join(ROOT, rel)
        if not os.path.exists(full):
            fail(f"{name}: link to {href}: {rel} does not exist")
        if rel.startswith("docs/") and rel[5:] in ORDER:
            target = f"#{doc_id(rel[5:])}--{frag}" if frag else f"#{doc_id(rel[5:])}"
            links.append((target, f"{name}: {href}"))
            return f'href="{target}"'
        kind = "tree" if os.path.isdir(full) else "blob"
        url = f"{REPO_URL}/{kind}/main/{urllib.parse.quote(rel)}" + (f"#{frag}" if frag else "")
        return f'href="{html.escape(url)}"'

    body = re.sub(r'href="([^"]*)"', link, body)

    def image(m):
        src = urllib.parse.unquote(html.unescape(m.group(1)))
        if re.match(r"^[a-z]+:", src):
            fail(f"{name}: image {src} is not a file of the repository")
        full = os.path.normpath(os.path.join(DOCS, os.path.dirname(name), src))
        if not os.path.isfile(full):
            fail(f"{name}: image {src} does not exist")
        mime = {"png": "image/png", "jpg": "image/jpeg", "jpeg": "image/jpeg", "svg": "image/svg+xml"}[full.rsplit(".", 1)[1].lower()]
        return f'src="data:{mime};base64,{base64.b64encode(open(full, "rb").read()).decode()}"'

    body = re.sub(r'src="([^"]*)"', image, body)
    return f'<section class="doc" id="{pid}">\n{body}\n</section>', heads, links


CSS = """
@font-face { font-family: Barlow; font-weight: 400; src: url(FONTS/barlow-latin-400-normal.woff2) }
@font-face { font-family: Barlow; font-weight: 600; src: url(FONTS/barlow-latin-600-normal.woff2) }
@font-face { font-family: Barlow; font-weight: 700; src: url(FONTS/barlow-latin-600-normal.woff2) }
@font-face { font-family: "Barlow Semi Condensed"; font-weight: 600; src: url(FONTS/barlow-semi-condensed-latin-600-normal.woff2) }
@font-face { font-family: "Barlow Semi Condensed"; font-weight: 700; src: url(FONTS/barlow-semi-condensed-latin-700-normal.woff2) }
@font-face { font-family: "JetBrains Mono"; font-weight: 400; src: url(FONTS/jetbrains-mono-latin-400-normal.woff2) }
:root { --fg: #17201b; --muted: #59665f; --line: #d2d9d4; --panel: #eef2ef; --accent: #a9561f }
@page { size: A4; margin: 18mm 17mm 18mm 17mm }
body { font: 10pt/1.45 Barlow, "DejaVu Sans", Arial, sans-serif; color: var(--fg); margin: 0 }
h1, h2, h3, h4 { font-family: "Barlow Semi Condensed", "DejaVu Sans Condensed", Arial, sans-serif; break-after: avoid; line-height: 1.2 }
h1 { font-size: 22pt; font-weight: 700; margin: 0 0 10pt; padding-bottom: 6pt; border-bottom: 2px solid var(--accent) }
h2 { font-size: 14pt; font-weight: 700; margin: 16pt 0 6pt; color: var(--accent) }
h3 { font-size: 11.5pt; font-weight: 600; margin: 12pt 0 4pt }
h4 { font-size: 10.5pt; font-weight: 600; margin: 10pt 0 4pt }
p, ul, ol { margin: 0 0 6pt } li { margin: 1pt 0 } ul, ol { padding-left: 16pt }
a { color: var(--accent); text-decoration: none }
code { font: 8.6pt "JetBrains Mono", "DejaVu Sans Mono", monospace; font-variant-ligatures: none; background: var(--panel); padding: 0 2px; border-radius: 2px }
pre { background: var(--panel); border: 1px solid var(--line); border-radius: 3px; padding: 6pt 8pt; white-space: pre-wrap;
      overflow-wrap: anywhere; margin: 0 0 8pt }
pre code { background: none; padding: 0; font-size: 7.2pt; line-height: 1.4 }
h1 code, h2 code, h3 code, h4 code { font-size: .9em; background: none; padding: 0 }
table { border-collapse: collapse; width: 100%; margin: 4pt 0 8pt; font-size: 8.8pt }
th, td { border: 1px solid var(--line); padding: 3pt 5pt; text-align: left; vertical-align: top; overflow-wrap: break-word }
th { background: var(--panel); font-weight: 600 }
tr { break-inside: avoid }
img { max-width: 100%; border: 1px solid var(--line); border-radius: 3px; margin: 2pt 0 8pt }
blockquote { margin: 0 0 8pt; padding: 4pt 10pt; border-left: 3px solid var(--accent); color: var(--muted) }
.doc { break-before: page }
.title { height: 245mm; display: flex; flex-direction: column; justify-content: center }
.title .name { font: 700 54pt/1 "Barlow Semi Condensed", Arial, sans-serif; color: var(--accent) }
.title .sub { font: 600 18pt/1.3 "Barlow Semi Condensed", Arial, sans-serif; margin-top: 8pt }
.title .meta { margin-top: 28pt; color: var(--muted); font-size: 10.5pt; line-height: 1.6 }
.title .note { margin-top: auto; color: var(--muted); font-size: 8.8pt; max-width: 150mm }
.toc { break-before: page }
.toc h1 { border: 0 }
.toc ol { list-style: none; padding: 0; margin: 0 }
.toc li { display: flex; gap: 6pt; margin: 2pt 0 }
.toc li a { color: var(--fg) }
.toc li .dots { flex: 1; border-bottom: 1px dotted var(--line); margin-bottom: 4pt }
.toc li.l1 { font: 600 12pt "Barlow Semi Condensed", Arial, sans-serif; margin-top: 10pt }
.toc li.l2 { padding-left: 14pt; font-size: 10pt }
.toc .tocend { margin-top: 14pt; color: var(--muted); font-size: 8.8pt }
"""


def build_html(pages=None):
    names = sorted(f for f in os.listdir(DOCS) if f.endswith(".md"))
    for f in names:
        if f not in ORDER and f not in SKIP:
            fail(f"docs/{f} is neither in ORDER nor in SKIP of tools/docs_pdf.py")
    for f in ORDER:
        if f not in names:
            fail(f"docs/{f} (ORDER) does not exist")
    parts, toc, ids, links = [], [], set(), []
    for name in ORDER:
        body, heads, lk = convert(name, open(os.path.join(DOCS, name), encoding="utf-8").read())
        parts.append(body)
        ids.add(doc_id(name))
        ids.update(h[1] for h in heads)
        links += lk
        for lvl, hid, text in heads:
            if lvl == 1:
                hid = doc_id(name)
            if lvl <= 2:
                toc.append((lvl, hid, text))
    for target, where in links:
        if target[1:] not in ids:
            fail(f"{where}: no such heading in the documentation")
    pages = pages or {}
    toc_html = "\n".join(
        f'<li class="l{lvl}"><a href="#{hid}">{html.escape(text)}</a><span class="dots"></span><span>{pages.get(hid, "")}</span></li>' for lvl, hid, text in toc
    )
    h, date = commit()
    meta = f"Version {version()}" + (f" · {date} · commit {h}" if h else "")
    fonts = "file://" + urllib.parse.quote(os.path.join(ROOT, "web", "fonts"))
    return f"""<!doctype html>
<html lang="en"><head><meta charset="utf-8"><title>kisi documentation</title>
<style>{CSS.replace("FONTS", fonts)}</style></head>
<body>
<section class="title">
  <div class="name">kisi</div>
  <div class="sub">Quick signal and power integrity checks for KiCad<br>Documentation</div>
  <div class="meta">{html.escape(meta)}<br><a href="{REPO_URL}">{REPO_URL}</a> · live version:
    <a href="https://riehlies.github.io/kisi/">riehlies.github.io/kisi</a></div>
  <div class="note">kisi is a quick design check: its results are estimates, without any warranty of correctness – not a
    replacement for sign-off simulation or measurement. The documentation is part of kisi, GPL-3.0-only.
    Rendered from <code>docs/</code> by <code>tools/docs_pdf.py</code>; the pages on GitHub are the same text.</div>
</section>
<nav class="toc"><h1>Contents</h1><ol>
{toc_html}
</ol><p class="tocend">{TOC_END}</p></nav>
{chr(10).join(parts)}
</body></html>
""", toc


def find_pages(pdf_path, toc):
    """Page numbers of the table-of-contents entries, from the PDF's outline (Chromium makes one
    bookmark per heading): the entries in order, matched by their text."""
    from pypdf import PdfReader
    r = PdfReader(pdf_path)
    norm = lambda s: re.sub(r"[^0-9a-z]+", "", s.lower())
    flat = []

    def walk(items):
        for it in items:
            if isinstance(it, list):
                walk(it)
            else:
                flat.append((norm(it.title), r.get_destination_page_number(it) + 1))

    walk(r.outline)
    pages, at = {}, 0
    for lvl, hid, text in toc:
        key = norm(text)
        for i in range(at, len(flat)):
            if flat[i][0] in (key, key + key):  # Chromium sometimes doubles a bookmark's title
                pages[hid], at = str(flat[i][1]), i + 1
                break
    return pages


def render(html_text, out):
    from playwright.sync_api import sync_playwright
    with tempfile.TemporaryDirectory() as d:
        src = os.path.join(d, "docs.html")
        open(src, "w", encoding="utf-8").write(html_text)
        with sync_playwright() as p:
            b = p.chromium.launch()
            page = b.new_page()
            page.goto("file://" + src, wait_until="load")
            page.evaluate("document.fonts.ready")
            foot = ('<div style="width:100%;font:8px Arial,sans-serif;color:#59665f;padding:0 17mm;display:flex">'
                    '<span>kisi documentation</span><span style="margin-left:auto" class="pageNumber"></span></div>')
            page.pdf(path=out, format="A4", print_background=True, display_header_footer=True, header_template="<span></span>",
                     footer_template=foot, margin={"top": "18mm", "bottom": "18mm", "left": "17mm", "right": "17mm"},
                     outline=True, tagged=True)
            b.close()


def main(argv):
    if argv[:1] == ["--html"]:
        out = argv[1] if len(argv) > 1 else "dist/kisi-docs.html"
        open(out, "w", encoding="utf-8").write(build_html()[0])
        print(f"{out}: written")
        return
    out = argv[0] if argv else os.path.join(ROOT, "dist", "kisi-docs.pdf")
    os.makedirs(os.path.dirname(os.path.abspath(out)), exist_ok=True)
    text, toc = build_html()
    render(text, out)
    try:
        import pypdf  # noqa: F401
    except ImportError:
        print("docs_pdf: pypdf not installed – table of contents without page numbers", file=sys.stderr)
    else:
        pages = find_pages(out, toc)
        missing = [t for _, hid, t in toc if hid not in pages]
        if missing:
            print(f"docs_pdf: page not found for {len(missing)} entries: {', '.join(missing[:5])}", file=sys.stderr)
        render(build_html(pages)[0], out)
    print(f"{out}: {os.path.getsize(out) / 1e6:.2f} MB, {len(ORDER)} pages of docs/, {len(toc)} contents entries")


if __name__ == "__main__":
    main(sys.argv[1:])

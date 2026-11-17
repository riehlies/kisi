# Advanced solvers: field views of a segment (prototype)

Design note for the ROADMAP item "Field views of a selected segment or via", 2D part (2026-11-17, branch
`feat/advanced-solvers`). Question: how does kisi show the field views of the fork
[riehlies/js_2d_fields](https://github.com/riehlies/js_2d_fields) (`FORK.md` there) for a clicked line segment –
potential, |E|, |H| with field lines, current density J, power flow S with the 50 / 90 / 99 % lines and the ∫S dA
check, losses split into signal conductor, ground and dielectric, arrows, equipotentials, odd / even, one excitation –
and what does each way of delivering them cost? The prototype builds and measures three variants.

## What the prototype does

* **Option.** A viewer built with `tools/bundle.py --advanced` has the option *Advanced solvers (field views)* in
  the field solver's *Solver options* (off by default; kept per browser). On, a net's detail gets a row *Fields*
  with the net's cross-sections (the clicked segment's first) and a button *Fields…*; the board tooltip says so.
  Settings: where to open (kisi-advanced.html next to the page, the fork's hosted page, or the copy built into the
  file), the solver there (full-wave: all views; quasi-static: potential and |E|), the frequency (100 MHz – 10 GHz)
  and the first view. Without `--advanced` the bundles are byte-identical to main (`cmp` in the prototype; the
  golden files and the size budget do not move).
* **Hand-over.** kisi writes the cross-section as js_2d_fields geometry text already (`xsgeom.rs`). *Fields…*
  opens the field page with the original's own link form – `#params=` + the settings (`tl_type: custom`, the text,
  the solver, one frequency) as deflate-raw + base64url, which the original applies on load and on every
  `hashchange` – plus keys read by `web/advanced/handover.js`: `solve=1`, `view=`, `mode=`, `exc=`, `title=`,
  `editor=hide`. The page solves and shows the chosen view without further clicks. The window is reused: the next
  segment changes only its fragment (no reload). The fragments of the three test segments are 0.49–0.58 kB.
* **kisi-advanced.html** (`tools/advanced_bundle.py`): the fork's page and its 68 modules as one file that also
  runs from disk. The fork is fetched at a pinned commit (`third_party/js_2d_fields_fork.json`,
  `tools/fetch_fork.sh`), not vendored. Its modules become one classic script (`tools/esm_classic.py`, as kisi's
  solver bundle); `web/advanced/runtime.js` serves the embedded files to `fetch`, starts the module worker from a
  blob and hands the worker its files on request (lazy: the 12 MB mesher is decoded only when the full-wave backend
  runs). Changed at build time, as in kisi.html: import/export statements, `import.meta.url` (a fixed URL per module),
  dynamic `import()` (resolved in the bundle), the page's stylesheet and Plotly loader and module tags (each
  replacement must match exactly once, so a changed fork fails the build, not the page). Nothing in
  `third_party/js_2d_fields` changes; the fork's original files and its `src/fork/` layer are used as they are.
* **Stand-ins without the full-wave build.** The original needs Emscripten builds that are in no git: `solver.wasm`
  (Eigen; kisi already replaces it with kisi-sparse), `eigen_solver.wasm` (Eigen + Spectra) and `gmsh.wasm`
  (gmsh + OCCT). Since upstream b5ea9eb even the quasi-static plot fields solve a complex system through
  `eigen_solver` (`_plot_fields`, complex permittivity). `web/advanced/eigen_sparse_module.js` provides the two
  linear solves of that module from kisi-sparse (complex symmetric as the real 2N block system, LU), so a build
  without `--fw` shows potential and |E|; the full-wave choice is disabled there. With `--fw DIR`
  (`tools/build_forsten_fullwave.sh`) the real modules go in and every view works.

Measured with `tools/measure_advanced.py` (headless Chromium: a real click on the board, *Fields…*, the window, the
solve, the view; sizes; memory as the resident size of all browser processes). Local: 2 cores, quasi-static only.
GitHub Actions (workflow `fullwave-xs`, input `only_advanced`, job `advanced`): with the full-wave modules.

## Numbers

Builds of one commit on GitHub Actions (run 44138096429; 4 vCPU AMD EPYC 9V45) unless marked *local* (2 cores, the
quasi-static build only). MB = 10⁶ bytes.

**Sizes**

| File | raw MB | gzip MB | note |
|---|---:|---:|---|
| `kisi.html` (default, as main) | 3.77 | 2.52 | unchanged by the prototype |
| `kisi.html --advanced` | 3.78 | 2.52 | +9 kB (the option's script) |
| `kisi-examples.html` / `--advanced` | 7.45 / 7.46 | 5.21 / 5.21 | |
| `kisi-advanced.html`, quasi-static | 3.14 | 2.32 | potential and \|E\| only |
| `kisi-advanced.html`, full-wave | 9.16 | 6.82 | every view |
| `kisi-examples.html --advanced-inline` (full-wave) | 16.56 | 12.10 | variant 2; `kisi.html` alone: ≈ 12.9 MB |
| `kisi.mjs` | 2.82 | 2.13 | + the two full-wave modules (gzip + base64): ≈ 8.8 MB, computed, not built |

Inside `kisi-advanced.html` (raw / gzip MB): `gmsh.wasm` 12.04 / 4.32, `eigen_solver.wasm` 0.48 / 0.17, Plotly 4.84 /
1.46, the fork's app (68 modules) 2.20 / 0.64, kisi-sparse 0.36 / 0.12, licence texts 0.04 / 0.01. The WASM modules
come from neither repository (both keep them out of git; kisi's `fullwave-xs` workflow kept them 14 days as
artifacts): `tools/build_forsten_fullwave.sh` built them in the measuring run (1608 s uncached on 4 vCPU, OCCT most
of it; cached afterwards). They could not be built or downloaded on the prototype's machine (the egress policy blocks
the emsdk download host and artifact downloads), hence the split between local and CI numbers.

**Times** (s; solve = from the page's *Solve* to its end, shown = the view drawn, wall = from the click on
*Fields…*, the first segment including the opening of the window, the later ones a fragment change of it)

| | microstrip 213 µm F.Cu | pair 191/127 µm F.Cu (PCIe) | stripline 127 µm In3 |
|---|---|---|---|
| quasi-static, CI: solve / shown / wall | 0.81 / 0.94 / 2.30 | 1.52 / 1.68 / 1.85 | 2.01 / 2.15 / 2.33 |
| quasi-static, local | 2.46 / 2.70 / 3.69 | 3.91 / 4.20 / 4.55 | 4.11 / 4.30 / 4.63 |
| full-wave, CI | 3.26 / 3.33 / 4.77 | 4.91 / 4.98 / 5.32 | 4.09 / 4.16 / 4.49 |
| full-wave, inline (variant 2), CI | 3.86 / 3.93 / 4.84 | – | – |
| quasi-static, other origin over http (variant 3 stand-in), CI | 1.42 / 1.56 / 2.28 | 1.45 / 1.58 / 1.86 | – |

The pair's other views (potential, \|H\|, J, losses) by hand-over: 4.4–4.6 s each in full-wave (every hand-over
solves again; switching views inside the page reuses the solve). Loading: `kisi-advanced.html` alone 1.2 s to
*Ready* (both builds; the embedded files decoded in 0.4 s, the 12 MB mesher only when the full-wave solve starts);
the viewer 0.76 s (default), 0.96–1.08 s (`--advanced`), 0.94 s (inline) – the built-in page costs no load time
until it is used.

**Memory** (PSS of all browser processes, MB; a fresh browser per measurement): about:blank 227 (local); the viewer
with the ThunderScope after the estimate 536; `kisi-advanced.html` alone, loaded 339; peak with the viewer and the field
window through all hand-overs: quasi-static 1045, full-wave 1930; inline, one full-wave segment 1352.

**Results the views show** (1 GHz, smooth copper): Z0 / Zodd, Zeven in Ω, full-wave: 50.67 / 43.21, 61.84 / 57.13;
the page's quasi-static: 50.59 / 43.14, 61.78 / 56.98; kisi's own field solver (`backend_forsten.js`, upstream
3d5009c): 50.47 / 43.01, 61.63 / 56.19. Power flow check of the full-wave views, ∫S dA against P: 9.82 / 9.78 mW
(+0.4 %), 22.9 / 22.9 mW (pair, odd), 8.45 / 8.65 mW (−2.3 %, stripline). No page logged an error or a field-view
warning; the |H|, J and loss views of the pair drew their fields (the loss view with the split into signal conductor,
ground and dielectric).

## The three variants

### 1. Second file `kisi-advanced.html` (built, measured end to end)

kisi.html stays as it is (+5 kB for the option in an `--advanced` build); `kisi-advanced.html` lies next to it and is
opened in a new window. Works from disk (`file://` → `file://`, tested), on GitHub Pages (same origin) and from any
other origin (the fragment is a navigation, not a script access). **KiCad plugin:** the plugin's page
(`analysis/<board>.kisi.html`, or the page served by `kisi_server` on `127.0.0.1`) has no `kisi-advanced.html` next to
it; the plugin would have to ship it (+ the file's size in the package) and either write it next to the page or
serve it at one more fixed path from `kisi_server` (a page on `http://127.0.0.1` may not open a `file://` URL).
Updates: a new fork commit in the pin file; the build fails on any change it cannot place. Licences: the file is
GPL-3.0 (the fork) with Plotly (MIT), kisi-sparse (faer, MIT/Apache-2.0) and, with the full-wave modules, gmsh
(GPL-2.0-or-later), OCCT (LGPL-2.1 with exception), Eigen and Spectra (MPL-2.0) and the Emscripten runtime
(MIT/UIUC); the fork's `LICENSE` and `THIRD_PARTY_NOTICES.txt` are embedded and linked from the page (the About tab's
links and a line in the tab bar); the corresponding source is the fork at the pinned commit plus kisi's build script
(versions in the page's `__kisiAdvInfo` and `BUILD_INFO.json`). A release would attach `kisi-advanced.html` and the
WASM build's `BUILD_INFO.json`.

### 2. In the bundle (measured)

`bundle.py --advanced-inline kisi-advanced.html` puts the whole page into kisi.html (gzip + base64, decoded only when
*Fields…* is first used, opened from a `blob:` URL; the runtime catches the original's `replaceState` that a `blob:`
URL cannot resolve). Opening and the solve work as in variant 1; the viewer's own load time hardly changes (the
embedded string is not parsed until used), but every copy of kisi.html – the release, kisi-examples.html, the KiCad
plugin's package and every page the plugin writes next to a board – carries the field page: see the sizes. For the
Node CLI (`kisi.mjs`) only the solver part would matter (no views): the two full-wave modules, gzip + base64. Lazy
options inside one file: what the prototype does (one gzip + base64 string per file, decoded on first use) is the
cheapest; a split per view is not possible because Plotly (4.8 MB raw) and the app (2.1 MB) are needed for any view.

### 3. Link-out to the hosted fork (measured with a stand-in)

kisi.html opens `https://advpcb.davidriehl.de/tl/field_solver.html#params=…`. Nothing is bundled; works from disk,
from the KiCad plugin's page and from GitHub Pages, but needs the internet (the hosted page could not be reached from
the measuring machine – egress policy –, so `kisi-advanced.html` served over `http://127.0.0.1` stood in for it: the
same hand-over from a `file://` viewer into another origin, the second segment by fragment only). How a geometry can
cross origins:

| Way | file:// → https | across origins | limit | needs in the fork |
|---|---|---|---|---|
| URL fragment (`#params=`) | yes | yes (navigation) | none by the server (never sent); browsers: Chromium 2 MB URL, the original caps the decompressed link at 1 MB | nothing for the geometry (the original reads it); `solve`/`view` need the patch |
| `postMessage` | yes (opener handle) | yes | structured clone, no practical limit | a listener; origin `null` from disk, so it must accept any sender |
| `localStorage` / `BroadcastChannel` | no | no (per origin) | – | – |

Today the hosted page already takes the geometry from the fragment; the user then presses *Solve* and picks the view.
The proposed fork patch (`fork-link-handover.patch` in the review folder, not pushed) adds `src/fork/handover.js` –
the same file as kisi's `web/advanced/handover.js` – with one script tag, a line in `deploy/make_site.mjs`, the hook
check and FORK.md; with it the hosted page solves and shows the view like kisi-advanced.html. Version skew: the
hosted page follows the fork's head, kisi's geometry text follows kisi's vendored upstream (3d5009c; the fork is on
b5ea9eb) – the text format has not changed between them, but nothing pins it. Licences: nothing is distributed by
kisi. Privacy: the geometry stays in the browser (fragment), but the page itself comes from a third-party host.

## Recommendation

**Variant 1, the second file, with the full-wave modules** – and variant 3 as the second choice in the same option.

* It keeps kisi.html as it is (+9 kB) and works offline, from disk and on GitHub Pages; the field page loads in
  about a second and the first full-wave view of a segment is there in 3–5 s, later segments without a reload.
* Variant 2 gives nothing the second file does not (load time and solve time are the same) and puts 9 MB into every
  copy of kisi.html – the release, kisi-examples.html, the plugin package and each page the plugin writes next to a
  board – and into the size budget that has been guarded to the kB so far.
* Variant 3 costs nothing in size and works today for the geometry (the original reads `#params=`); with the
  proposed fork patch it is as convenient as variant 1. But it needs the internet, a third-party host and a hosted
  version that matches kisi's geometry text; keep it as the fallback where `kisi-advanced.html` is not at hand.
* Ship the full-wave build (9.2 MB): the quasi-static one (3.1 MB) shows only potential and |E|, which kisi's users
  can mostly read off the numbers already; H, J, S and the loss split are the point of the feature.

**Effort to production quality: about 7 days** (3.5 for a first release without the plugin and the upstream move):

| Part | Days |
|---|---:|
| Viewer: the option into `net-detail.js` / `interaction.js` instead of wrapping their functions, its place in the UI, docs (getting started, reference), `check_viewer.py` coverage | 1.5 |
| Release and CI: the full-wave WASM build in `release.yml` (cached; 27 min cold), `kisi-advanced.html` on the release and Pages, a size budget entry, the reproducibility check, a quasi-static smoke test in `ci.yml` | 1.5 |
| Licences: the WASM build's versions in the page's notices, the source offer (fork commit, build script, pinned versions) in the release notes | 0.5 |
| Fork: the hand-over patch, plus a small exported API (view, fit) so kisi needs neither the Plotly menu event nor the plot's layout | 0.5–1 |
| One upstream revision: kisi's vendored upstream (3d5009c) to the fork's base (b5ea9eb), with `tools/upstream_update.sh`, the field validation and the golden files – the geometry text and the numbers then come from one solver | 1 |
| Hand-over content: kisi's conductivity and roughness, the frequency from the lane's profile, pair mode and excitation (voltage between conductors for a pair), the what-if stackup; estimates marked | 1 |
| KiCad plugin: ship `kisi-advanced.html` and serve it from `kisi_server` at one fixed path (an `http://127.0.0.1` page cannot open `file://`), the offline page next to it; a test in KiCad | 1 |

## Open questions

1. Delivery: variant 1 with variant 3 as the fallback (recommended) – or variant 3 only (no size, but online and
   version skew)?
2. The release: the full-wave `kisi-advanced.html` (9.2 MB), the quasi-static one (3.1 MB), or both?
3. KiCad plugin: carry `kisi-advanced.html` in the package (+≈ 6.8 MB in the zip, which is ≈ 6.4 MB now)?
4. The fork: take `src/fork/handover.js` (patch in the review folder)? And offer `setView` / a fit as exports, so
   that kisi's adapter needs no Plotly internals (now: the `plotly_buttonclicked` event of the menu named `view`, and
   deleting the plot's axis ranges before the next draw)?
5. Move kisi's vendored upstream to b5ea9eb (the fork's base), so that the impedance kisi shows and the field page's
   come from one revision? The stripline above differs by 1.4 % between kisi's FDM and the page's quasi-static solve
   at the same tolerance; that wants a look either way.
6. The field page is dark only (the original's style); a light theme would be a change in the fork.
7. Defaults handed over: 1 GHz and 1 V peak per conductor now; the lane's Nyquist frequency, a pair as voltage
   between conductors, power instead of voltage?
8. Should the hosted URL be configurable (a self-hosted copy of the fork)?
9. The 3D part of the ROADMAP item (a via in kisi-fdtd with field output) is not touched here.

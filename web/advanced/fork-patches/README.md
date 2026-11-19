# kisi's patches to the js_2d_fields fork

The field page `kisi-advanced.html` (its 2D cross-section views) is built from the fork
[riehlies/js_2d_fields](https://github.com/riehlies/js_2d_fields) at the commit pinned in
[`third_party/js_2d_fields_fork.json`](../../../third_party/js_2d_fields_fork.json). The changes kisi needs in the fork
are kept here as patches – **proposed for the fork, not pushed to it**. Nobody changes the fork automatically; its
maintainer takes a patch over by hand (`git am web/advanced/fork-patches/000N-*.patch` in a checkout of the fork, or
the same change written anew).

[`tools/fetch_fork.sh`](../../../tools/fetch_fork.sh) applies them in name order to a clean checkout of the pinned
commit on every build (`git apply`). A patch that does not apply stops the build with its name and git's message:
either the new pin contains it (drop it here) or the fork changed under it (refresh it: apply the patches to a
checkout of the old pin, rebase them onto the new one, `git format-patch --no-signature --zero-commit -N`).

| Patch | What it does | Files in the fork | Drop it with the first pin that contains |
|---|---|---|---|
| `0001-plot-api.patch` | `window.forkPlot`: `view()`, `views()`, `setView(view, { fit })`, `fit()` – a script that drives the plot (the hand-over) needs neither the Plotly menu's `plotly_buttonclicked` event nor deleting the plot's axis ranges | `src/fork/plot_fields.js`, `FORK.md` | `forkPlot.setView` in `src/fork/plot_fields.js` |
| `0002-handover.patch` | link hand-over `src/fork/handover.js`: the original's `#params=` link plus `solve`, `view` (|E| where the solver does not offer the view), `mode`, `exc`, `title`, `editor`; each step reported to the opener by `postMessage`; uses 0001 | `src/fork/handover.js` (new), `src/field_solver.html` (one module tag), `deploy/make_site.mjs`, `tests/fork/check_hooks.mjs`, `FORK.md` | `src/fork/handover.js` |
| `0003-theme.patch` | light and dark theme: the page follows the browser's preference, the link key `theme=light\|dark` or `forkTheme.set()`; the plots' layout colours follow, the field colour scales stay | `src/fork/theme.css`, `src/fork/theme.js` (new), `src/field_solver.html` (two tags in `<head>`), `deploy/make_site.mjs`, `tests/fork/check_hooks.mjs`, `src/fork/handover.js` (the `theme` key in its list), `FORK.md` | `src/fork/theme.js` |
| `0004-adaptive-settings.patch` | the quasi-static adaptive solve's parameter tolerance (fixed at 5 % in `solve_worker.js`) and initial grid (fixed at 30 cells per axis in `getParams`) as the settings *Parameter Tolerance (%)* and *Initial Grid (cells per axis)* in the Solver Settings and the link's settings, so that kisi's hand-over asks for kisi's own adaptive mesh and the page gives kisi's numbers for the same cross-section | `src/app_solver.js` (defaults, settings table, bounds, `getParams`), `src/solve_worker.js` (two lines), `src/field_solver.html` (two inputs), `tests/fork/check_hooks.mjs`, `FORK.md` | `initial_grid` in `src/app_solver.js` |

Each patch keeps the fork's rules (FORK.md there): new code in `src/fork/`, every line in the original's files marked
`FORK` and counted by `tests/fork/check_hooks.mjs` (`node tests/fork/check_hooks.mjs` passes with all four applied).
The bundler [`tools/advanced_bundle.py`](../../../tools/advanced_bundle.py) bundles every module tag of the page and
inlines `fork/theme.css` and `fork/theme.js`; without the patches the build stops there.

# Corpus run: kisi on open-source boards

`data/corpus.json` lists open-source KiCad boards at pinned commits; `tools/corpus.py` fetches them (blobless and
sparse: only the board and its project file, into `~/.cache/kisi-corpus` or `$KISI_CORPUS_CACHE` – never into the
repository) and runs the native `kisi` on each: `overview`, `interfaces`, `check --all`, `gaps`, `power`,
`thermal` and, for the weekly set, `crosstalk`, all without a `kisi.json` (`--no-config`) unless the entry names
one. A board that stops for an assembly variant (a documented stop) runs again with `--variant default`.

The files here are kisi's own numbers per board and command – exit status, crash or panic, the error message, the
run time, and summary numbers (layers, stackup from the file, classes with a target and their source, buses,
lanes, check rows by colour, crosstalk victims, gap ids with item counts, the three views, rails, the thermal
grid) – no board content:

* `weekly.json` – 32 boards (16 SI-rich, 8 PI-rich, 8 parser cases), run by the weekly CI and on a manual run;
* `robustness.json` – about 200 boards across KiCad 5 to 10, run by hand (Actions → ci → Run workflow, corpus:
  robustness, or `gh workflow run ci --ref <branch> -f corpus=robustness`) and before a release.

```
python3 tools/corpus.py fetch --set weekly                         # once; later runs reuse the cache
python3 tools/corpus.py compare target/release/kisi --set weekly   # fails on a regression
python3 tools/corpus.py compare target/release/kisi --set weekly --update   # an intended change: store
python3 tools/corpus.py compare target/release/kisi --only openrx,marble    # some boards (with --update: only these)
```

`compare` fails on a new crash, panic, timeout or output that is not JSON, on a changed exit status or variant stop,
and on a changed summary number (numbers within 0.1 %); a run time fails only above 4 × the stored time + 10 s
(runners differ) and is a note above 2 × + 2 s. A repository that cannot be fetched (network, deleted, made
private) is listed and skipped, never failed. A change that moves these numbers on purpose stores them with
`--update` in the same commit, like `validation/golden/`.

The candidate list comes from the [kicad-happy test harness](https://github.com/aklofas/kicad-happy-testharness)
(its `repos.md`: repository URLs with pinned commits); only repository names and commits were taken from it. The
licence in each entry is the licence file found at the commit; boards without one are used for local tests only,
not for release artefacts or numbers quoted in the documentation.

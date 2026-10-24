# Stored results of the example boards

kisi's JSON output on ThunderScope and LibreVNA (`web/samples/{ts,vna}.kisi.json`) for `overview`, `interfaces`,
`check --all`, `crosstalk`, `gaps`, `power --pdn <rail>` and `thermal`, from the native CLI (formula impedances).
Numbers are rounded to 6 significant digits; the config echo, field-solver status and timings are left out.

CI compares every run with these files (`tools/golden.py boards`): verdicts, warnings and names must be identical,
numbers within 1e-4 relative. A model change that moves results updates them in the same commit:

```
python3 tools/golden.py boards target/release/kisi examples --update
python3 tools/golden.py vias target/release/kisi --update      # validation/fdtd/results/kisi_models.json
```

The diff of these files in the review is the list of what the change moved. Whether the new values are better is
decided against the references (`validation/`, `validation/fdtd/TESTS.md`), not here.

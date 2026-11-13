# Stored results of the example boards

kisi's JSON output on ThunderScope and LibreVNA (`web/samples/{ts,vna}.kisi.json`) for `overview`, `interfaces`,
`check --all`, `crosstalk`, `gaps`, `power --pdn <rail>` and `thermal`, from the native CLI (formula impedances).
Numbers are rounded to 6 significant digits; the config echo, field-solver status and timings are left out.
`<board>.report.html` is the printable report (`report --html`, dated 1970-01-01 by `SOURCE_DATE_EPOCH=0`, the
version left out): its text must stay the same, each number within 1e-4 relative or one unit of its last decimal.

CI compares every run with these files (`tools/golden.py boards`): verdicts, warnings and names must be identical,
numbers within 1e-4 relative. A model change that moves results updates them in the same commit:

```
python3 tools/golden.py boards target/release/kisi examples --update
python3 tools/golden.py vias target/release/kisi --update      # validation/fdtd/results/kisi_models.json
```

The diff of these files in the review is the list of what the change moved. Whether the new values are better is
decided against the references (`validation/`, `validation/fdtd/TESTS.md`), not here.

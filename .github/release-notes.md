## Corresponding source of the field pages

`kisi-advanced.html` (full-wave, every view) and `kisi-advanced-lite.html` (quasi-static) are GPL-3.0 and built by the
release workflow from:

* the fork [{fork_repo_name}]({fork_repo}) of js_2d_fields (original by Henrik Forstén) at commit
  [`{fork_commit_short}`]({fork_repo}/tree/{fork_commit}), with kisi's patches {patches} from
  [`web/advanced/fork-patches/`]({kisi_tree}/web/advanced/fork-patches) of this tag;
* kisi's scripts at this tag: [`tools/fetch_fork.sh`]({kisi_tree}/tools/fetch_fork.sh),
  [`tools/advanced_bundle.py`]({kisi_tree}/tools/advanced_bundle.py), [`web/advanced/`]({kisi_tree}/web/advanced) and
  kisi-sparse ([`crates/kisi-sparse`]({kisi_tree}/crates/kisi-sparse)) in place of the original's Eigen solver;
* `kisi-advanced.html` only: the full-wave WebAssembly modules, built by
  [`tools/build_forsten_fullwave.sh`]({kisi_tree}/tools/build_forsten_fullwave.sh) from js_2d_fields
  [`{upstream_rev_short}`](https://github.com/Ttl/js_2d_fields/tree/{upstream_rev}) with its submodules {submodules},
  OpenCASCADE {occt} and the Emscripten SDK {emsdk} – licences in the page's third-party notices (About tab).

SHA-256: {page_hashes}. Versions, commits and the modules' SHA-256: `kisi-advanced.build-info.json`.

"""Rust port of the reduced models against the Python prototypes and the 3D references.

Runs `kisi-validate models` (crates/kisi-cli/src/models.rs) on all via, cavity and PDN-loop
cases, then compares
* parity: Rust with the prototype's model (uniform r–z grid, no junction model) against Python
  (via_pbm.py, cavity_model.py, pdn_loop.py);
* kisi's ladder (graded grid, equivalent antipad radius, junction model) against Python and 3D,
  written to validation/via_ref/<case>/ladder.s2p for the reference page;
* accuracy against the de-embedded 3D result with the same metrics for Python and Rust;
* run time: Python prototype (in-process, without start-up) against Rust.

usage: python3 rust_models.py [--exe kisi-validate] [--out DIR] [--no-python-timing]
       → results/rust_models.json (S-parameters of the Rust runs go to DIR, not to the repository)
"""
import argparse, json, os, subprocess, sys, tempfile, time
import numpy as np

HERE = os.path.dirname(os.path.abspath(__file__))
REF = os.path.join(os.path.dirname(HERE), "via_ref")
sys.path.insert(0, HERE)
import sparam  # noqa: E402
import via_pbm  # noqa: E402
from cavity_model import zmatrix  # noqa: E402


def tdr_err(f, s3, sm):
    """relative errors of the TDR minimum and maximum (25 ps, 80 ps window), % (as TESTS.md:
    both values of every case are pooled for the median and the 90th percentile)"""
    _, z3 = sparam.tdr(f, s3, rise=25e-12, tmax=80e-12)
    _, zm = sparam.tdr(f, sm, rise=25e-12, tmax=80e-12)
    return [round(abs(zm.min() / z3.min() - 1) * 100, 2), round(abs(zm.max() / z3.max() - 1) * 100, 2)]


def notch(f, s21):
    n = sparam.db(s21)
    return float(f[np.argmin(n)] / 1e9) if n.min() < -10 else None


def pct(v, q):
    v = [x for x in v if x is not None]
    return round(float(np.percentile(v, q)), 3) if v else None


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--exe", default=os.environ.get("KISI_VALIDATE", "kisi-validate"))
    ap.add_argument("--out", default=os.path.join(tempfile.gettempdir(), "kisi_rust_models"))
    ap.add_argument("--no-python-timing", action="store_true")
    a = ap.parse_args()
    t0 = time.time()
    rust = json.loads(subprocess.run([a.exe, "models", os.path.join(HERE, "via_cases"), a.out], capture_output=True, text=True, check=True).stdout)
    t_rust_total = time.time() - t0
    rows = {}
    for cid, r in sorted(rust.items()):
        c = json.load(open(os.path.join(HERE, "via_cases", cid + ".json")))
        g = c["meta"]["group"]
        if "rust_pbm" in r:
            pp = os.path.join(REF, cid, "pbm.s2p")
            if not os.path.exists(pp):
                continue
            f, Sp = sparam.read_touchstone(pp)
            _, Su = sparam.read_touchstone(os.path.join(a.out, cid, "rust_pbm.s2p"))
            _, Sg = sparam.read_touchstone(os.path.join(a.out, cid, "rust_pbm_graded.s2p"))
            # kisi's ladder (graded grid, junction model) next to the case, for the reference page
            sparam.write_touchstone(os.path.join(REF, cid, "ladder.s2p"), f, Sg, [f"{cid}: via ladder as kisi uses it (kisi_si::via_pbm, graded grid, junction model)" +
                                                                                  (", differential mode (per line 50 ohm)" if g == "E" else ", ports at the via")])
            # 3D, de-embedded as in via_pbm.py
            if g == "E":
                f3, S3 = sparam.read_touchstone(os.path.join(REF, cid, "kisi.s4p"))
                th = [os.path.join(REF, f"thru_{c['meta']['stackup']}_{q['layer']}", "kisi.s2p") for q in c["meta"]["ports"]]
                D = sparam.deembed_shift(S3, [sparam.fixture_shift(f3, sparam.read_touchstone(t)[1]) for t in th])
                D = sparam.mixed_mode(D)["dd"]
            else:
                _, D = sparam.read_touchstone(os.path.join(REF, cid, "kisi_deembedded.s2p"))
            m = f <= 20e9
            row = {"group": g}
            for tag, S in (("python", Sp), ("rust", Su), ("rust_graded", Sg)):
                d21 = np.abs(sparam.db(D[m, 1, 0]) - sparam.db(S[m, 1, 0]))
                ph = np.abs(np.degrees(np.angle(D[m, 1, 0] * np.conj(S[m, 1, 0]))))
                row[tag] = {"dS21_max_db": round(float(d21.max()), 3), "dS21_mean_db": round(float(d21.mean()), 3),
                            "dphase_max_deg": round(float(ph.max()), 2), "notch_ghz": notch(f, S[:, 1, 0])}
                if g != "E":
                    row[tag]["tdr_err_pct"] = tdr_err(f, D[:, 0, 0], S[:, 0, 0])
            row["notch_3d_ghz"] = notch(f, D[:, 1, 0])
            for tag, S in (("rust", Su), ("rust_graded", Sg)):
                row[f"parity_{tag}"] = {"dS21_max_db": round(float(np.max(np.abs(sparam.db(S[:, 1, 0]) - sparam.db(Sp[:, 1, 0])))), 4),
                                        "dS21_lin_max": round(float(np.max(np.abs(S[:, 1, 0] - Sp[:, 1, 0]))), 4),
                                        "dS11_max": round(float(np.max(np.abs(S[:, 0, 0] - Sp[:, 0, 0]))), 4),
                                        "notch_shift_pct": round((notch(f, S[:, 1, 0]) / notch(f, Sp[:, 1, 0]) - 1) * 100, 2) if notch(f, S[:, 1, 0]) and notch(f, Sp[:, 1, 0]) else None,
                                        "dphase21_max_deg": round(float(np.max(np.abs(np.degrees(np.angle(S[:, 1, 0] * np.conj(Sp[:, 1, 0])))))), 3)}
            row["time_ms"] = {"rust_build": round(r["rust_pbm"]["build_ms"], 2), "rust_graded_build": round(r["rust_pbm_graded"]["build_ms"], 2),
                              "rust_eval": round(r["rust_pbm_graded"]["eval_ms"], 2), "nf": r["rust_pbm"]["nf"],
                              "laplace_unknowns": [r["rust_pbm"]["laplace_unknowns"], r["rust_pbm_graded"]["laplace_unknowns"]]}
            if not a.no_python_timing:
                t1 = time.perf_counter()
                via_pbm.ladder(c, f, mode="odd" if g == "E" else None)
                row["time_ms"]["python"] = round((time.perf_counter() - t1) * 1e3, 1)
            rows[cid] = row
        elif "cavity" in r:
            f, Sm = sparam.read_touchstone(os.path.join(REF, cid, "model.s2p"))
            _, Sr = sparam.read_touchstone(os.path.join(a.out, cid, "rust_model.s2p"))
            Zp, Zr = sparam.s2z(Sm, 1.0), sparam.s2z(Sr, 1.0)
            row = {"group": g, "parity": {"dZ_rel_max": float(np.max(np.abs(Zr - Zp) / np.abs(Zp)))},
                   "time_ms": {"rust_eval": round(r["cavity"]["eval_ms"], 2), "nf": r["cavity"]["nf"], "modes": r["cavity"]["modes"]}}
            if not a.no_python_timing:
                bx = c["board"]
                lay = c["layers"][0]
                ports = [((q["x"] - bx[0]) * 1e-3, ((q["y0"] + q["y1"]) / 2 - bx[1]) * 1e-3) for q in c["ports"]]
                t1 = time.perf_counter()
                zmatrix(f, (bx[2] - bx[0]) * 1e-3, (bx[3] - bx[1]) * 1e-3, (lay["z1"] - lay["z0"]) * 1e-3, lay["er"], lay.get("tand", 0),
                        c.get("f_loss") or c["fmax"] / 2, ports, c["mesh"]["dfine"] * 1e-3)
                row["time_ms"]["python"] = round((time.perf_counter() - t1) * 1e3, 1)
            rows[cid] = row
        elif "pdn_loop" in r:
            pl = {x["case"]: x for x in json.load(open(os.path.join(HERE, "results", "pdn_loop.json")))["rows"]}
            rows[cid] = {"group": g, "L_rust_nH": round(r["pdn_loop"]["L_plane_model_nH"], 4), "L_python_nH": pl[cid]["L_plane_model_nH"],
                         "time_ms": {"rust": round(r["pdn_loop"]["ms"], 3)}}
    # summary
    single = [v for v in rows.values() if v["group"] in "ABCDX" and "python" in v]
    pairs = [v for v in rows.values() if v["group"] == "E"]
    vias = single + pairs
    summ = {"rust_total_s": round(t_rust_total, 2), "cases": len(rows)}
    for tag in ("python", "rust", "rust_graded"):
        summ[tag] = {
            "single_tdr_err_pct_median": pct([e for v in single for e in v[tag]["tdr_err_pct"]], 50),
            "single_tdr_err_pct_p90": pct([e for v in single for e in v[tag]["tdr_err_pct"]], 90),
            "single_dS21_mean_db_median": pct([v[tag]["dS21_mean_db"] for v in single], 50),
            "single_dS21_max_db_median": pct([v[tag]["dS21_max_db"] for v in single], 50),
            "pairs_dS21_max_db_median": pct([v[tag]["dS21_max_db"] for v in pairs], 50),
        }
    for tag in ("rust", "rust_graded"):
        summ[f"parity_{tag}"] = {"dS21_max_db_max": max(v[f"parity_{tag}"]["dS21_max_db"] for v in vias),
                                 "dS21_max_db_median": pct([v[f"parity_{tag}"]["dS21_max_db"] for v in vias], 50),
                                 "dS21_lin_max_max": max(v[f"parity_{tag}"]["dS21_lin_max"] for v in vias),
                                 "dS21_lin_max_median": pct([v[f"parity_{tag}"]["dS21_lin_max"] for v in vias], 50),
                                 "dS11_max_max": max(v[f"parity_{tag}"]["dS11_max"] for v in vias),
                                 "notch_shift_pct": [v[f"parity_{tag}"]["notch_shift_pct"] for v in vias if v[f"parity_{tag}"]["notch_shift_pct"] is not None],
                                 "dphase21_max_deg_max": max(v[f"parity_{tag}"]["dphase21_max_deg"] for v in vias)}
    tm = [v["time_ms"] for v in vias]
    summ["via_time_ms_median"] = {k: pct([t[k] for t in tm if k in t], 50) for k in ("python", "rust_build", "rust_graded_build", "rust_eval")}
    summ["via_eval_us_per_freq_median"] = pct([t["rust_eval"] / t["nf"] * 1e3 for t in tm], 50)
    cav = [v for v in rows.values() if v["group"] == "G"]
    if cav:
        summ["cavity"] = {"dZ_rel_max": max(v["parity"]["dZ_rel_max"] for v in cav),
                          "rust_eval_ms": [v["time_ms"]["rust_eval"] for v in cav], "python_ms": [v["time_ms"].get("python") for v in cav],
                          "nf": cav[0]["time_ms"]["nf"], "modes": cav[0]["time_ms"]["modes"]}
    out = {"summary": summ, "cases": rows}
    json.dump(out, open(os.path.join(HERE, "results", "rust_models.json"), "w"), indent=1)
    print(json.dumps(summ, indent=1))


if __name__ == "__main__":
    main()

"""Solve the cross-sections of cases.json with CST Studio Suite's 2D port-mode solver and store one
JSON per case in results/ (README.md: how to run, what is computed, how results come back).

    python cst_xs.py [--tier 1] [--group G,..] [--ids a,b] [--limit N] [--skip-done]
                     [--walls both|electric|magnetic] [--freq 1] [--tol 0.1] [--max-levels 5]
                     [--lossy --freqs 0.1,1,5,10,20,40] [--margin-scale S] [--tag T]
                     [--detach] [--status] [--dry-run] [--probe] [--keep-projects]

Per case (all in one CST project, built from VBA history blocks through `cst.interface`):
* the geometry text (case["cst_text"] or case["text"], kisi's js_2d_fields syntax) cut into
  non-overlapping rectangles (xsgeo.tiles) and extruded along z by 1/40 of the box size; dielectrics as
  bricks (air = εr 1), conductors as PEC bricks (lossy runs: copper σ = 5.8e7 S/m), trapezoids
  (etch group) as extruded polygons cut into the bricks;
* the box = case["domain"]["box"] (port size rule, xsgeo.domain); side boundaries electric, or
  magnetic with ground straps (xsgeo.straps) for the second solve; a side closed by a full ground
  plane is that plane; z boundaries electric;
* one waveguide port on the whole zmin face, multipin with potentials: a single line
  sig+ positive / grounds negative (one quasi-TEM mode, Z = line impedance Z0); a pair mode 1
  sig+ positive / sig- negative (differential: Zdiff = 2 Zodd) and mode 2 both signals positive /
  grounds negative (common: Zcomm = Zeven / 2);
* port modes only (time-domain solver, CalculateModesOnly), frequency f0 (default 1 GHz), lossless;
* mesh convergence: the hexahedral mesh refined level by level (lines per box ×2, edge refinement
  and ratio limit up) until Z and εeff of every mode change by less than --tol % (default 0.1) from
  the previous level; CST's adaptive port meshing on in addition (if this CST version takes it).
  The magnetic-wall solve then runs on the converged mesh level.
* --lossy: on the converged level, lossy materials (tanδ of the geometry, copper), port modes at
  each of --freqs; result in results/<id>.lossy.json.

Results: results/<id>[.tag].json with every level (mesh settings, cells, time, per mode Z, β, εeff),
the converged values per wall kind, Z0 / Zdiff / Zcomm / Zodd / Zeven / εeff, and the open-boundary
estimate (mean of the electric- and magnetic-wall values, README.md). Run with the Python that has
CST's `cst` package (CST's python_cst_libraries on PYTHONPATH, as validation/fdtd/run_cst.py).
Standard library only.
"""
import argparse, datetime, glob, json, math, os, re, shutil, subprocess, sys, time, traceback

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import xsgeo  # noqa: E402

C0 = 299792458.0

for _s in (sys.stdout, sys.stderr):
    try:
        _s.reconfigure(encoding="utf-8", errors="replace")
    except Exception:
        pass

LOG = None


def log(*a):
    line = time.strftime("%Y-%m-%d %H:%M:%S ") + " ".join(str(x) for x in a)
    print(line, flush=True)
    if LOG:
        with open(LOG, "a", encoding="utf-8") as fh:
            fh.write(line + "\n")


def q(x):
    if isinstance(x, str):
        return f'"{x}"'
    return f'"{float(x):.9g}"'


# --- mesh levels -------------------------------------------------------------------------------------

def mesh_level(k):
    """hexahedral mesh of refinement level k: lines per box 20·2^k, edge refinement 4+2k, ratio limit
    20·2^k (capped)"""
    return {"level": k, "lpb": 20 * 2 ** k, "edge_ratio": 4 + 2 * k, "ratio": min(20 * 2 ** k, 1000), "lpw": 20}


# --- the model -----------------------------------------------------------------------------------------

class Model:
    def __init__(self):
        self.blocks = []
        self.n = 0

    def add(self, caption, code, kind="geo", optional=False):
        self.blocks.append({"caption": caption, "code": code.strip() + "\n", "kind": kind, "optional": optional})


def material_block(name, er, tand, lossy, f_ghz):
    if lossy and tand > 0:
        loss = f' .TanD {q(tand)}\n .TanDFreq {q(f_ghz)}\n .TanDGiven "True"\n .TanDModel "ConstTanD"\n'
    else:
        loss = ' .TanD "0.0"\n .TanDGiven "False"\n'
    return f"""
With Material
 .Reset
 .Name "{name}"
 .Folder ""
 .FrqType "all"
 .Type "Normal"
 .MaterialUnit "Frequency", "GHz"
 .MaterialUnit "Geometry", "mm"
 .Epsilon {q(er)}
 .Mu "1.0"
 .Sigma "0"
{loss} .SigmaM "0"
 .Colour "0.2", "0.6", "0.2"
 .Transparency "70"
 .Create
End With"""


def copper_block():
    return """
With Material
 .Reset
 .Name "copper_kisi"
 .Folder ""
 .FrqType "all"
 .Type "Lossy metal"
 .MaterialUnit "Frequency", "GHz"
 .MaterialUnit "Geometry", "mm"
 .Mu "1.0"
 .Sigma "5.8e7"
 .Colour "1", "0.6", "0.2"
 .Create
End With"""


def boundary_block(walls, box_closed):
    w = "electric" if walls == "electric" else "magnetic"
    return f"""
With Boundary
 .Xmin "{w}"
 .Xmax "{w}"
 .Ymin "{'electric' if box_closed[0] else w}"
 .Ymax "{'electric' if box_closed[1] else w}"
 .Zmin "electric"
 .Zmax "electric"
 .Xsymmetry "none"
 .Ysymmetry "none"
 .Zsymmetry "none"
 .ApplyInAllDirections "False"
End With"""


def mesh_block(m):
    return f"""
With Mesh
 .MeshType "PBA"
 .SetCreator "High Frequency"
End With
With MeshSettings
 .SetMeshType "Hex"
 .Set "Version", 1%
 .Set "StepsPerWaveNear", {q(m['lpw'])}
 .Set "StepsPerWaveFar", {q(m['lpw'])}
 .Set "WavelengthRefinementSameAsNear", "1"
 .Set "StepsPerBoxNear", {q(m['lpb'])}
 .Set "StepsPerBoxFar", {q(m['lpb'])}
 .Set "MaxStepNear", "0"
 .Set "MaxStepFar", "0"
 .Set "ModelBoxDescrNear", "maxedge"
 .Set "ModelBoxDescrFar", "maxedge"
 .Set "UseMaxStepAbsolute", "0"
 .Set "GeometryRefinementSameAsNear", "0"
 .Set "UseRatioLimitGeometry", "1"
 .Set "RatioLimitGeometry", {q(m['ratio'])}
 .Set "MinStepGeometryX", "0"
 .Set "MinStepGeometryY", "0"
 .Set "MinStepGeometryZ", "0"
 .Set "UseSameMinStepGeometryXYZ", "1"
End With
With MeshSettings
 .SetMeshType "Hex"
 .Set "FaceRefinementOn", "0"
 .Set "EdgeRefinementOn", "1"
 .Set "EdgeRefinementRatio", {q(m['edge_ratio'])}
 .Set "EdgeRefinementBufferLines", "3"
End With"""


def mesh_minimal(m):
    return f"""
With Mesh
 .MeshType "PBA"
End With
With MeshSettings
 .SetMeshType "Hex"
 .Set "StepsPerBoxNear", {q(m['lpb'])}
 .Set "RatioLimitGeometry", {q(m['ratio'])}
 .Set "EdgeRefinementRatio", {q(m['edge_ratio'])}
End With"""


def solver_block(f_ghz):
    return f"""
Solver.FrequencyRange {q(f_ghz * 0.98)}, {q(f_ghz * 1.02)}
With Solver
 .Method "Hexahedral"
 .CalculationType "TD-S"
 .StimulationPort "All"
 .StimulationMode "All"
 .SteadyStateLimit "-40"
 .MeshAdaption "False"
 .AutoNormImpedance "False"
 .NormingImpedance "50"
 .CalculateModesOnly "True"
 .SParaSymmetry "False"
 .StoreTDResultsInCache "False"
 .FullDeembedding "False"
 .SuperimposePLWExcitation "False"
 .UseSensitivityAnalysis "False"
End With"""


def port_adapt_block(passes, on=True):
    """adaptive port meshing; CST 2025.5's default is on (accuracy 1 %, 4 passes; the accuracy is an integer
    percentage), so "off" has to be set explicitly"""
    if not on:
        return """
With Solver
 .AdaptivePortMeshing "False"
End With"""
    return f"""
With Solver
 .AdaptivePortMeshing "True"
 .AccuracyAdaptivePortMeshing "1"
 .PassesAdaptivePortMeshing {q(passes)}
End With"""


def potential_sets(g, box, extras):
    """[(set number, [("Positive"|"Negative", x, y)], meaning)] for the port: single line or pair"""
    sig = [r for r in g["rects"] if r["kind"] in ("sig+", "sig-")]
    gnd = [r for r in g["rects"] if r["kind"] == "gnd"]
    if not gnd and not extras:
        raise ValueError("no ground conductor")
    # one point in the main ground (the conductor with the largest extent in the box), plus every
    # isolated ground
    main = max(gnd, key=lambda r: (min(r["x1"], box[1]) - max(r["x0"], box[0])) * (min(r["y1"], box[3]) - max(r["y0"], box[2])))
    gpts = [xsgeo.interior_point(main, box)]
    for k in xsgeo.isolated_grounds(g, box):
        gpts.append(xsgeo.interior_point(g["rects"][k], box))
    pt = lambda r: xsgeo.interior_point(r, box)
    if len(sig) == 1:
        return [(1, [("Positive",) + pt(sig[0])] + [("Negative",) + p for p in gpts], "line")]
    if len(sig) == 2:
        sp = [r for r in sig if r["kind"] == "sig+"][0]
        sn = [r for r in sig if r["kind"] == "sig-"][0]
        return [(1, [("Positive",) + pt(sp), ("Negative",) + pt(sn)], "differential"),
                (2, [("Positive",) + pt(sp), ("Positive",) + pt(sn)] + [("Negative",) + p for p in gpts], "common")]
    raise ValueError(f"{len(sig)} signal conductors")


def port_block(sets, potentials=True):
    lines = ["With Port", " .Reset", ' .PortNumber "1"', ' .Label ""', ' .Folder ""', f' .NumberOfModes "{len(sets)}"',
             ' .AdjustPolarization "False"', ' .PolarizationAngle "0.0"', ' .ReferencePlaneDistance "0"', ' .TextSize "50"',
             ' .TextMaxLimit "0"', ' .Coordinates "Full"', ' .Orientation "zmin"', ' .PortOnBound "True"',
             ' .ClipPickedPortToBound "False"', ' .Xrange "0", "0"', ' .Yrange "0", "0"', ' .Zrange "0", "0"',
             ' .XrangeAdd "0.0", "0.0"', ' .YrangeAdd "0.0", "0.0"', ' .ZrangeAdd "0.0", "0.0"',
             ' .SingleEnded "False"', ' .WaveguideMonitor "False"']
    if potentials:
        for num, pts, _ in sets:
            for sign, x, y in pts:
                lines.append(f' .AddPotentialNumerically "{num}", "{sign}", {q(x)}, {q(y)}')
    lines += [" .Create", "End With"]
    return "\n".join(lines)


def build(case, walls="electric", lossy=False, f_ghz=1.0, box=None):
    """VBA history blocks of the model (geometry, boundaries, port, solver; no mesh block)"""
    text = case.get("cst_text") or case["text"]
    g = xsgeo.parse(text)
    box = box or case["domain"]["box"]
    X0, X1, Y0, Y1 = box
    extras = xsgeo.wall_extras(g, box, walls)
    tl = xsgeo.tiles(g, box, extras)
    span = max(X1 - X0, Y1 - Y0)
    L = span / 40.0  # extrusion length (z): the port modes do not depend on it; few cells along z (lpb / 40)
    m = Model()
    m.add("units", 'With Units\n .Geometry "mm"\n .Frequency "GHz"\n .Time "ns"\n .TemperatureUnit "Kelvin"\nEnd With', "setup")
    m.add("background", """
With Background
 .ResetBackground
 .Type "Normal"
 .Epsilon "1.0"
 .Mu "1.0"
 .XminSpace "0.0"
 .XmaxSpace "0.0"
 .YminSpace "0.0"
 .YmaxSpace "0.0"
 .ZminSpace "0.0"
 .ZmaxSpace "0.0"
 .ApplyInAllDirections "False"
End With""", "setup")
    # a side closed by a full plane is electric whatever the wall kind (the plane is there)
    dom = case["domain"]
    m.add("boundaries", boundary_block(walls, (dom.get("closed_bottom", False), dom.get("closed_top", False))), "setup")
    mats = {}
    cond = "copper_kisi" if lossy else "PEC"
    if lossy:
        m.add("material copper", copper_block())
    for t in tl:
        if t["mat"][0] == "diel":
            key = (t["mat"][1], t["mat"][2] if lossy else 0.0)
            if key not in mats:
                mats[key] = f"er{key[0]:g}_tand{key[1]:g}".replace(".", "p").replace("-", "m")
                m.add(f"material {mats[key]}", material_block(mats[key], key[0], key[1], lossy, f_ghz))
    names = []
    for n, t in enumerate(tl):
        if t["mat"][0] == "diel":
            comp, mat, nm = "diel", mats[(t["mat"][1], t["mat"][2] if lossy else 0.0)], f"d{n}"
        else:
            k = t["cond"]
            kind = t["mat"][0]
            comp, mat = "cond", cond
            nm = f"{'sigp' if kind == 'sig+' else 'sign' if kind == 'sig-' else 'gnd'}{k}_{n}"
        names.append((comp, nm, t))
        m.add(f"brick {nm}", f"""
With Brick
 .Reset
 .Name "{nm}"
 .Component "{comp}"
 .Material "{mat}"
 .Xrange {q(t['x0'])}, {q(t['x1'])}
 .Yrange {q(t['y0'])}, {q(t['y1'])}
 .Zrange "0", {q(L)}
 .Create
End With""")
    # trapezoids: extruded polygons cut into the bricks they overlap
    for k, r in enumerate(g["rects"]):
        if r["shape"] != "trap":
            continue
        pts = xsgeo.trap_polygon(r)
        nm = f"{'sigp' if r['kind'] == 'sig+' else 'sign' if r['kind'] == 'sig-' else 'gnd'}{k}_trap"
        code = f'WCS.ActivateWCS "global"\nWith Polygon\n .Reset\n .Name "p{k}"\n .Curve "c{k}"\n .Point {q(pts[0][0])}, {q(pts[0][1])}\n'
        for p in pts[1:] + [pts[0]]:
            code += f" .LineTo {q(p[0])}, {q(p[1])}\n"
        code += f""" .Create
End With
With ExtrudeCurve
 .Reset
 .Name "{nm}"
 .Component "cond"
 .Material "{cond}"
 .Thickness {q(L)}
 .Twistangle "0.0"
 .Taperangle "0.0"
 .DeleteProfile "True"
 .Curve "c{k}:p{k}"
 .Create
End With
"""
        xa, xb = min(p[0] for p in pts), max(p[0] for p in pts)
        ya, yb = min(p[1] for p in pts), max(p[1] for p in pts)
        for comp, bn, t in names:
            if comp == "diel" and t["x0"] < xb and t["x1"] > xa and t["y0"] < yb and t["y1"] > ya:
                code += f'Solid.Insert "diel:{bn}", "cond:{nm}"\n'
        m.add(f"trapezoid {nm}", code)
    sets = potential_sets(g, box, extras)
    m.add("port", port_block(sets), "port")
    m.add("solver", solver_block(f_ghz), "setup")
    info = {"tiles": len(tl), "extrusion_mm": L, "box": box, "walls": walls, "extras": extras,
            "potential_sets": [{"set": s, "meaning": mm, "pins": [list(p) for p in pts]} for s, pts, mm in sets]}
    return m.blocks, sets, info


def to_bas(blocks, mesh=None):
    s = "' generated by kisi validation/cst_xs/cst_xs.py\nSub Main\n"
    for b in blocks:
        s += f"' --- {b['caption']}\n" + b["code"]
    if mesh:
        s += "' --- mesh\n" + mesh_block(mesh).strip() + "\n"
    return s + "End Sub\n"


# --- CST ---------------------------------------------------------------------------------------------

def cst_import(extra):
    if extra:
        sys.path.insert(0, extra)
    import cst  # noqa: F401
    import cst.interface
    import cst.results
    return cst


def start_de(cst):
    di = cst.interface.DesignEnvironment
    for how in ("connect_to_any", "new", "ctor"):
        try:
            de = di.connect_to_any() if how == "connect_to_any" else di.new() if how == "new" else di()
            log("design environment via", how)
            return de
        except Exception as e:
            log("design environment", how, "failed:", repr(e))
    raise RuntimeError("cannot start the CST design environment")


def solver_logs(prj_dir):
    files = sorted(glob.glob(os.path.join(prj_dir, "**", "*.log"), recursive=True), key=os.path.getmtime, reverse=True)
    res = {}
    for f in files[:12]:
        try:
            res[os.path.relpath(f, prj_dir)] = open(f, encoding="utf-8", errors="replace").read()
        except OSError:
            pass
    return res


def find_int(texts, patterns):
    for t in texts:
        for p in patterns:
            mm = re.search(p, t, re.I)
            if mm:
                try:
                    return int(re.sub(r"[,\. ]", "", mm.group(1)))
                except ValueError:
                    pass
    return None


def find_last_int(texts, pattern):
    """the last match of pattern in the first log that has one"""
    for t in texts:
        mm = re.findall(pattern, t)
        if mm:
            try:
                return int(re.sub(r"[,\. ]", "", mm[-1]))
            except ValueError:
                pass
    return None


def mode_of(path):
    for p in (r"[Mm]ode\s*(\d+)", r"\[(\d+)\]", r"\((\d+)\)\s*$", r"Port\s*\d+\D+(\d+)\s*$"):
        mm = re.search(p, path.split("\\")[-1])
        if mm:
            return int(mm.group(1))
    return 1


QUANT = (("line impedance", "z"), ("wave impedance", "zwave"), ("beta", "beta"), ("gamma", "gamma"), ("alpha", "alpha"),
         ("accuracy", "accuracy"), ("eps", "eeff_cst"), ("cutoff", "fc"))


def read_port_results(cst, prj_path):
    """port mode quantities from the result tree: {"items": {path: [[x], [y]]}, "modes": {m: {q: value}}}
    (y complex as [re, im]); every item under a Port folder is kept raw, the known ones parsed"""
    pf = cst.results.ProjectFile(prj_path, allow_interactive=True)
    r3 = pf.get_3d()
    tree = r3.get_tree_items()
    items = {}
    modes = {}
    for t in tree:
        tl = t.lower()
        if "port" not in tl or not any(k in tl for k in ("information", "port mode", "impedance", "beta", "gamma", "accuracy")):
            continue
        try:
            try:
                ids = r3.get_run_ids(t)
                it = r3.get_result_item(t, max(ids)) if ids else r3.get_result_item(t)
            except Exception:
                it = r3.get_result_item(t)
            xs = [float(x) for x in it.get_xdata()]
            ys = [complex(y) for y in it.get_ydata()]
        except Exception:
            continue
        if not ys:
            continue
        items[t] = [xs, [[y.real, y.imag] for y in ys]]
        for key, qn in QUANT:
            if key in tl.split("\\")[-2].lower() if "\\" in t else key in tl:
                modes.setdefault(mode_of(t), {})[qn] = [xs, [[y.real, y.imag] for y in ys]]
                break
    return {"tree_size": len(tree), "items": items, "modes": modes}


def at_freq(series, f_ghz):
    """the value of a [xs, [[re, im]]] series at f (GHz; xs in GHz as the project unit), complex"""
    xs, ys = series
    vals = [complex(a, b) for a, b in ys]
    if len(xs) <= 1:
        return vals[0]
    k = min(range(len(xs)), key=lambda i: abs(xs[i] - f_ghz))
    return vals[k]


def log_values(texts):
    """line impedances / betas printed in the solver log (fallback source)"""
    out = {}
    for t in texts:
        for mm in re.finditer(r"[Mm]ode\s*(\d+)[^\n]{0,200}?[Ll]ine\s+[Ii]mpedance[^\d\-]{0,20}([\d.]+(?:[eE][+-]?\d+)?)", t):
            out.setdefault(int(mm.group(1)), {})["z_log"] = float(mm.group(2))
    return out


def eeff_from_beta(beta, f_ghz, er_max):
    """εeff = (β c / ω)², β in 1/m (CST's default) or 1/mm; the unit that gives 1 ≤ εeff ≤ εr,max"""
    k0 = 2 * math.pi * f_ghz * 1e9 / C0
    for scale, unit in ((1.0, "1/m"), (1e3, "1/mm")):
        e = (beta * scale / k0) ** 2
        if 0.95 <= e <= er_max * 1.05:
            return e, unit
    return (beta / k0) ** 2, "1/m?"


def level_modes(res, sets, f_ghz, er_max, logs):
    """per potential set: Z (real part of the line impedance), β, εeff"""
    out = []
    lv = log_values(logs)
    for num, _, meaning in sets:
        md = res["modes"].get(num, {})
        e = {"mode": num, "meaning": meaning}
        if "z" in md:
            z = at_freq(md["z"], f_ghz)
            e["z"], e["z_im"] = z.real, z.imag
        if "beta" in md:
            b = at_freq(md["beta"], f_ghz).real
        elif "gamma" in md:
            b = at_freq(md["gamma"], f_ghz).imag
        else:
            b = None
        if b:
            e["beta"] = b
            e["eeff"], e["beta_unit"] = eeff_from_beta(b, f_ghz, er_max)
        if "accuracy" in md:
            e["accuracy"] = at_freq(md["accuracy"], f_ghz).real
        if num in lv:
            e.update(lv[num])
            if "z" not in e:
                e["z"] = lv[num]["z_log"]
        out.append(e)
    # pairs: the larger impedance is the differential mode (Zdiff = 2 Zodd > Zcomm = Zeven / 2)
    if len(out) == 2 and all("z" in e for e in out) and out[0]["z"] < out[1]["z"]:
        for e, mng in zip(out, ("common", "differential")):
            e["meaning_by_value"] = mng
    return out


def derive(modes):
    """Z0 / Zdiff, Zcomm, Zodd, Zeven, εeff from the modes of one level"""
    if len(modes) == 1:
        e = modes[0]
        return {"z0": e.get("z"), "eeff": e.get("eeff")}
    d = {e.get("meaning_by_value", e["meaning"]): e for e in modes}
    dif, com = d.get("differential", {}), d.get("common", {})
    r = {"zdiff": dif.get("z"), "zcomm": com.get("z"), "eeff_odd": dif.get("eeff"), "eeff_even": com.get("eeff")}
    r["zodd"] = r["zdiff"] / 2 if r["zdiff"] else None
    r["zeven"] = 2 * r["zcomm"] if r["zcomm"] else None
    return r


def rel_change(a, b):
    ch = 0.0
    for k in set(a) & set(b):
        if a.get(k) and b.get(k):
            ch = max(ch, abs(a[k] / b[k] - 1) * 100)
    return ch


class Runner:
    def __init__(self, cst, de, a):
        self.cst, self.de, self.a = cst, de, a

    def add_blocks(self, prj, blocks, meta):
        for b in blocks:
            try:
                prj.model3d.add_to_history(b["caption"], b["code"])
            except Exception as e:
                if not any(f["caption"] == b["caption"] for f in meta["blocks_failed"]):
                    meta["blocks_failed"].append({"caption": b["caption"], "error": str(e)})
                if b["kind"] == "port":
                    # without potentials: the port's eigenmodes (CST's own impedance definition)
                    log(f"   port with potentials failed ({e}); eigenmodes without potentials")
                    prj.model3d.add_to_history("port (no potentials)", port_block(meta["_sets"], potentials=False))
                    meta["potentials"] = False
                elif not b.get("optional"):
                    meta["blocks_failed"][-1]["code"] = b["code"]
                    raise

    def solve(self, prj, cp, wd, meta, sets, f_ghz, er_max, mesh):
        try:
            prj.model3d.add_to_history(f"mesh level {mesh['level']}", mesh_block(mesh))
        except Exception as e:
            meta["blocks_failed"].append({"caption": f"mesh level {mesh['level']}", "error": str(e)})
            prj.model3d.add_to_history(f"mesh level {mesh['level']} (minimal)", mesh_minimal(mesh))
            meta["mesh_block"] = "minimal"
        t0 = time.time()
        prj.model3d.run_solver()
        secs = time.time() - t0
        prj.save()
        logs = solver_logs(os.path.splitext(cp)[0])
        texts = list(logs.values())
        # the hex mesh of the port-mode calculation (MCalc.log: one line per adaptive port pass, the last one
        # is the final mesh); not "The maximum supported number of mesh cells ... is: 2,147,483,647"
        cells = find_last_int(texts, r"Hex mesh: global:[^\n]*?np \(no TSTs\):\s*([\d,]+)")
        if cells is None:
            cells = find_int(texts, [r"Number of mesh cells(?! without)[^\d]*([\d,\. ]+\d)",
                                     r"Total number of mesh cells[^\d]*([\d,\. ]+\d)"])
        port_nodes = find_int(texts, [r"\b[Pp]ort\b(?![^\n]*without MPI)[^\n]{0,80}?(?:mesh|nodes|cells)[^\d\n]{0,20}([\d,]+)"])
        if not meta.get("cst_version"):
            for t in texts:
                mm = re.search(r"Version:\s*(20\d\d\.\d+[^\n\r(]*)", t)
                if mm:
                    meta["cst_version"] = "CST Studio Suite " + mm.group(1).strip()
                    break
        res = read_port_results(self.cst, cp)
        modes = level_modes(res, sets, f_ghz, er_max, texts)
        meta["_last_logs"] = logs
        meta["_last_raw"] = res
        return {"mesh": mesh, "cells": cells, "port_mesh": port_nodes, "seconds": round(secs, 1), "modes": modes, "values": derive(modes)}

    def solve_fresh(self, make, tag, meta, sets, f_ghz, er_max, mesh, extra=None):
        """build a new project (make), solve it once, close it. CST 2025.5 keeps the port-mode results of
        the first solve of a project: a second solve with another mesh writes no new port results
        (Port1_Info1[1].mmd and the result tree stay as they were; seen in the pilot), so each mesh level
        and each frequency gets a project of its own"""
        prj, cp = make(tag)
        try:
            if extra:
                for cap, code in extra:
                    try:
                        prj.model3d.add_to_history(cap, code)
                    except Exception as e:
                        if not any(f["caption"] == cap for f in meta["blocks_failed"]):
                            meta["blocks_failed"].append({"caption": cap, "error": str(e)})
            return self.solve(prj, cp, os.path.dirname(cp), meta, sets, f_ghz, er_max, mesh)
        finally:
            try:
                prj.close()
            except Exception:
                pass

    def converge(self, make, meta, sets, f_ghz, er_max, start=0, fixed=None):
        a = self.a
        levels = []
        if fixed is not None:
            lv = self.solve_fresh(make, f"L{fixed}", meta, sets, f_ghz, er_max, mesh_level(fixed))
            log(f"   level {fixed}: {fmt_values(lv['values'])} ({lv['cells']} cells, {lv['seconds']} s)")
            return {"levels": [lv], "converged": None, "level": fixed, "values": lv["values"]}
        for k in range(start, start + a.max_levels):
            lv = self.solve_fresh(make, f"L{k}", meta, sets, f_ghz, er_max, mesh_level(k))
            levels.append(lv)
            ch = rel_change(lv["values"], levels[-2]["values"]) if len(levels) > 1 else None
            lv["change_pct"] = ch
            log(f"   level {k}: {fmt_values(lv['values'])} ({lv['cells']} cells, {lv['seconds']} s)"
                + (f", change {ch:.3f} %" if ch is not None else ""))
            if not any(v for v in lv["values"].values()):
                break
            if ch is not None and ch < a.tol:
                return {"levels": levels, "converged": True, "level": k, "change_pct": ch, "values": lv["values"],
                        "values_extrap": extrapolate(levels), "values_richardson": richardson(levels)}
            if lv["cells"] and lv["cells"] > a.max_cells:
                log(f"   cell limit {a.max_cells} reached")
                break
        last = levels[-1]
        return {"levels": levels, "converged": False, "level": last["mesh"]["level"], "change_pct": last.get("change_pct"),
                "values": last["values"], "values_extrap": extrapolate(levels), "values_richardson": richardson(levels)}

    def run_case(self, case):
        a = self.a
        cid = case["id"]
        stem = cid + (f".{a.tag}" if a.tag else "") + (".lossy" if a.lossy else "")
        out = os.path.join(a.out, stem + ".json")
        wd = os.path.abspath(os.path.join(a.work, stem))
        os.makedirs(wd, exist_ok=True)
        box = case["domain"]["box"]
        if a.margin_scale:
            box = xsgeo.domain(xsgeo.parse(case.get("cst_text") or case["text"]), a.margin_scale)["box"]
        g = xsgeo.parse(case.get("cst_text") or case["text"])
        er_max = max([r["er"] for r in g["rects"] if r["kind"] == "diel"] + [1.0])
        walls = ["electric", "magnetic"] if a.walls == "both" else [a.walls]
        if a.lossy:
            walls = ["electric"]  # dispersion: electric walls only (README.md)
        if "magnetic" in walls and xsgeo.isolated_grounds(g, box):
            walls = [w for w in walls if w != "magnetic"] or ["electric"]
            log("   isolated ground: electric walls only (ground tied to the wall)")
        meta = {"id": cid, "group": case["group"], "tier": case["tier"], "tag": a.tag, "date": datetime.date.today().isoformat(),
                "tool": "CST Studio Suite", "solver": "time domain solver, port modes only (CalculateModesOnly), hexahedral PBA mesh",
                "cst_version": None, "f_ghz": a.freq, "tol_pct": a.tol, "box": box, "margin_scale": a.margin_scale or case.get("margin_scale", 1.0),
                "text_hash": text_hash(case.get("cst_text") or case["text"]), "potentials": True, "blocks_failed": [],
                "port_adapt": a.port_adapt, "start_level": a.start_level, "status": "started", "walls": {}, "lossy": a.lossy}
        t_case = time.time()
        log(f"== {cid} ({case['group']}, tier {case['tier']}): {case['title']}")
        try:
            prev_level = None
            if a.lossy:
                prev = load_json(os.path.join(a.out, cid + (f".{a.tag}" if a.tag else "") + ".json"))
                prev_level = (prev or {}).get("walls", {}).get("electric", {}).get("level")
            for wk in walls:
                blocks, sets, info = build(case, wk, lossy=a.lossy, f_ghz=a.freq if not a.lossy else a.freqs[0], box=box)
                meta["_sets"] = sets

                def make(tag, wk=wk, blocks=blocks):
                    prj = self.de.new_mws()
                    cp = os.path.join(wd, f"{cid}_{wk}_{tag}.cst")
                    try:
                        self.add_blocks(prj, blocks, meta)
                        self.add_blocks(prj, [{"caption": "adaptive port meshing", "code": port_adapt_block(a.port_passes, a.port_adapt), "kind": "setup", "optional": True}], meta)
                        try:
                            prj.save(cp, allow_overwrite=True)
                        except TypeError:
                            prj.save(cp)
                    except Exception:
                        try:
                            prj.close()
                        except Exception:
                            pass
                        raise
                    return prj, cp

                try:
                    if a.no_solve:
                        make("built")[0].close()
                        meta["status"] = "built"
                        continue
                    if not a.lossy:
                        if wk == "electric" or "electric" not in meta["walls"]:
                            res = self.converge(make, meta, sets, a.freq, er_max, start=a.start_level)
                        else:  # magnetic: on the converged level of the electric solve
                            res = self.converge(make, meta, sets, a.freq, er_max, fixed=meta["walls"]["electric"]["level"])
                    else:
                        lvl = prev_level if prev_level is not None else 2
                        res = {"level": lvl, "freqs": []}
                        for fg in a.freqs:
                            lv = self.solve_fresh(make, f"f{fg:g}", meta, sets, fg, er_max, mesh_level(lvl),
                                                  extra=[(f"frequency {fg}", f"Solver.FrequencyRange {q(fg * 0.98)}, {q(fg * 1.02)}")])
                            lv["f_ghz"] = fg
                            res["freqs"].append(lv)
                            log(f"   {fg:g} GHz: {fmt_values(lv['values'])}")
                    res["build"] = {k: v for k, v in info.items() if k != "box"}
                    meta["walls"][wk] = res
                    meta["cst_raw_last"] = {"tree_size": meta["_last_raw"]["tree_size"], "items": list(meta["_last_raw"]["items"])[:80]}
                    with open(os.path.join(a.logs, f"{stem}_{wk}_logs.txt"), "w", encoding="utf-8") as fh:
                        for k, t in meta.get("_last_logs", {}).items():
                            fh.write(f"==== {k}\n{t}\n")
                finally:
                    pass
            if a.no_solve:
                return meta
            meta["result"] = combine(meta["walls"]) if not a.lossy else None
            ok = all(w.get("converged") is not False for w in meta["walls"].values()) if not a.lossy else True
            have = any(v for w in meta["walls"].values() for v in (w.get("values") or {}).values()) if not a.lossy else True
            meta["status"] = "ok" if (ok and have) else "not_converged" if have else "no_values"
            if meta["status"] == "ok" and not a.keep_projects:
                shutil.rmtree(wd, ignore_errors=True)
        except Exception as e:
            meta["status"] = "failed"
            meta["error"] = str(e)
            log("   FAILED:", repr(e))
            traceback.print_exc()
        finally:
            meta["seconds_total"] = round(time.time() - t_case, 1)
            for k in ("_sets", "_last_logs", "_last_raw"):
                meta.pop(k, None)
            with open(out, "w", encoding="utf-8") as fh:
                json.dump(meta, fh, indent=1, ensure_ascii=False)
                fh.write("\n")
            log(f"   {meta['status']} in {meta['seconds_total']} s → {os.path.relpath(out, HERE)}")
        return meta


def extrapolate(levels):
    """mesh-converged estimate from the last three levels: where the changes shrink geometrically
    (ratio r = d2 / d1 in 0..1), Z∞ = Z3 + d2 · r / (1 − r); None otherwise. Recorded, not used as
    the result."""
    if len(levels) < 3:
        return None
    v1, v2, v3 = (lv["values"] for lv in levels[-3:])
    out = {}
    for k in v3:
        if not (v1.get(k) and v2.get(k) and v3.get(k)):
            continue
        d1, d2 = v2[k] - v1[k], v3[k] - v2[k]
        if d1 == 0 or d2 == 0:
            out[k] = v3[k]
            continue
        r = d2 / d1
        if 0 < r < 0.9:
            out[k] = v3[k] + d2 * r / (1 - r)
    return out or None


def richardson(levels, p=2):
    """Richardson estimate from the last two levels: the lines per box double from level to level
    (h ratio 2), so V∞ ≈ V2 + (V2 − V1) / (2^p − 1); p = 2 assumed (second-order FIT/PBA). Recorded with
    the levels it uses; the anchors, solved up to level 3, measure how good it is."""
    if len(levels) < 2:
        return None
    a, b = levels[-2], levels[-1]
    va, vb = a["values"], b["values"]
    vals = {k: vb[k] + (vb[k] - va[k]) / (2 ** p - 1) for k in vb if va.get(k) and vb.get(k)}
    return {"p": p, "from_levels": [a["mesh"]["level"], b["mesh"]["level"]], "values": vals} if vals else None


def combine(walls):
    """converged values per wall kind and the open-boundary estimate (mean of electric and magnetic)"""
    r = {k: w.get("values") for k, w in walls.items()}
    e, m = r.get("electric") or {}, r.get("magnetic") or {}
    if e and m:
        r["open_est"] = {k: (e[k] + m[k]) / 2 for k in e if e.get(k) and m.get(k)}
        r["open_spread_pct"] = rel_change(e, m)
    return r


def fmt_values(v):
    return ", ".join(f"{k} {x:.4g}" for k, x in v.items() if x) or "no values"


def text_hash(t):
    h = 0xcbf29ce484222325
    for b in t.encode():
        h ^= b
        h = (h * 0x100000001b3) & 0xFFFFFFFFFFFFFFFF
    return f"{h:016x}"


def load_json(p):
    try:
        return json.load(open(p, encoding="utf-8"))
    except (OSError, ValueError):
        return None


# --- selection, status, detach ---------------------------------------------------------------------

def select(cases, a):
    sel = cases
    if a.ids:
        want = set(a.ids.split(","))
        sel = [c for c in sel if c["id"] in want]
    if a.tier:
        tiers = {int(t) for t in str(a.tier).split(",")}
        sel = [c for c in sel if c["tier"] in tiers]
    if a.group:
        gs = set(a.group.split(","))
        sel = [c for c in sel if c["group"] in gs]
    if a.lossy and not a.ids and not a.all_cases:
        sel = [c for c in sel if c.get("dispersion")]
    if a.limit:
        sel = sel[:a.limit]
    return sel


def done(a, c):
    stem = c["id"] + (f".{a.tag}" if a.tag else "") + (".lossy" if a.lossy else "")
    m = load_json(os.path.join(a.out, stem + ".json"))
    # ok and not_converged are final (a rerun gives the same); failed / no_values / started run again
    return bool(m and m.get("status") in ("ok", "not_converged") and m.get("text_hash") == text_hash(c.get("cst_text") or c["text"]))


def status(a, cases):
    sel = select(cases, a)
    st = {}
    secs = []
    for c in sel:
        stem = c["id"] + (f".{a.tag}" if a.tag else "") + (".lossy" if a.lossy else "")
        m = load_json(os.path.join(a.out, stem + ".json"))
        s = m.get("status") if m else "todo"
        if m and s == "ok" and m.get("text_hash") != text_hash(c.get("cst_text") or c["text"]):
            s = "stale"
        st[s] = st.get(s, 0) + 1
        if m and m.get("seconds_total"):
            secs.append(m["seconds_total"])
    todo = st.get("todo", 0) + st.get("stale", 0)
    mean = sum(secs) / len(secs) if secs else 60.0
    print(f"selection {len(sel)}: " + ", ".join(f"{k} {v}" for k, v in sorted(st.items())))
    print(f"mean {mean:.0f} s per case → about {todo * mean / 3600:.1f} h for the {todo} left")
    pidf = os.path.join(a.logs, "run.pid")
    if os.path.isfile(pidf):
        print("pid file:", open(pidf).read().strip())
    if LOG and os.path.isfile(LOG):
        lines = open(LOG, encoding="utf-8", errors="replace").read().splitlines()
        print("--- last log lines ---")
        print("\n".join(lines[-12:]))


def detach(argv, a):
    """start this run again in the background (no console), output to the log; returns at once"""
    args = [sys.executable, os.path.abspath(__file__)] + [x for x in argv if x != "--detach"]
    out = open(os.path.join(a.logs, "run.out"), "a", encoding="utf-8")
    kw = {}
    if os.name == "nt":
        # a console of its own (minimised), not DETACHED_PROCESS: without a console, CST's version query
        # (DesignEnvironment.version) starts the design environment in a new window that waits for a click
        kw["creationflags"] = 0x00000010 | 0x00000200  # CREATE_NEW_CONSOLE | CREATE_NEW_PROCESS_GROUP
        si = subprocess.STARTUPINFO()
        si.dwFlags |= subprocess.STARTF_USESHOWWINDOW
        si.wShowWindow = 7  # SW_SHOWMINNOACTIVE
        kw["startupinfo"] = si
    else:
        kw["start_new_session"] = True
    p = subprocess.Popen(args, stdout=out, stderr=subprocess.STDOUT, stdin=subprocess.DEVNULL, cwd=os.getcwd(), **kw)
    with open(os.path.join(a.logs, "run.pid"), "w") as fh:
        fh.write(f"{p.pid} {datetime.datetime.now().isoformat(timespec='seconds')} {' '.join(argv)}\n")
    print(f"started in the background, pid {p.pid}; progress: python cst_xs.py --status [same selection]; log {LOG}")


def main(argv=None):
    global LOG
    argv = sys.argv[1:] if argv is None else argv
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--cases", default=os.path.join(HERE, "cases.json"))
    ap.add_argument("--out", default=os.path.join(HERE, "results"))
    ap.add_argument("--work", default=os.path.join(HERE, "work"))
    ap.add_argument("--logs", default=os.path.join(HERE, "logs"))
    ap.add_argument("--tier", default="")
    ap.add_argument("--group", default="")
    ap.add_argument("--ids", default="")
    ap.add_argument("--limit", type=int, default=0)
    ap.add_argument("--skip-done", action="store_true", help="skip cases with a final result (ok, not_converged) for the same geometry")
    ap.add_argument("--walls", choices=["both", "electric", "magnetic"], default="both")
    ap.add_argument("--freq", type=float, default=1.0, help="port-mode frequency of the lossless solve, GHz")
    ap.add_argument("--tol", type=float, default=0.1, help="mesh convergence: largest change of Z / εeff between levels, %%")
    ap.add_argument("--max-levels", type=int, default=5)
    ap.add_argument("--start-level", type=int, default=0, help="first mesh level of the convergence study (the level before the one most cases converge on)")
    ap.add_argument("--max-cells", type=float, default=3e6)
    ap.add_argument("--no-port-adapt", dest="port_adapt", action="store_false")
    ap.add_argument("--port-passes", type=int, default=6)
    ap.add_argument("--lossy", action="store_true", help="dispersion run: lossy materials, port modes at --freqs")
    ap.add_argument("--freqs", default="0.1,1,5,10,20,40")
    ap.add_argument("--all-cases", action="store_true", help="--lossy on every selected case, not only those marked dispersion")
    ap.add_argument("--margin-scale", type=float, default=0.0, help="recompute the box with this margin scale")
    ap.add_argument("--tag", default="")
    ap.add_argument("--cst-lib", default="", help="directory with CST's python_cst_libraries (added to sys.path)")
    ap.add_argument("--keep-projects", action="store_true")
    ap.add_argument("--detach", action="store_true", help="run in the background and return at once")
    ap.add_argument("--status", action="store_true", help="progress of the selection, last log lines")
    ap.add_argument("--dry-run", action="store_true", help="no CST: write the VBA macro (.bas) of each case to --work")
    ap.add_argument("--no-solve", action="store_true", help="build and save the CST projects only")
    ap.add_argument("--probe", action="store_true", help="import CST, print its version and the result-tree API, nothing else")
    a = ap.parse_args(argv)
    a.freqs = [float(x) for x in a.freqs.split(",")]
    for d in (a.out, a.work, a.logs):
        os.makedirs(d, exist_ok=True)
    LOG = os.path.join(a.logs, "run.log")
    cases = json.load(open(a.cases, encoding="utf-8"))["cases"]
    if a.status:
        status(a, cases)
        return
    if a.detach:
        detach(argv, a)
        return
    sel = select(cases, a)
    if a.dry_run:
        for c in sel:
            for wk in ("electric", "magnetic"):
                blocks, sets, info = build(c, wk, lossy=a.lossy, f_ghz=a.freq)
                with open(os.path.join(a.work, f"{c['id']}_{wk}.bas"), "w", encoding="utf-8") as fh:
                    fh.write(to_bas(blocks, mesh_level(1)))
        print(f"{len(sel)} cases: macros (electric and magnetic walls) written to {a.work}")
        return
    log("python", sys.version.split()[0], "| cases", len(sel), "| args", " ".join(argv))
    cst = cst_import(a.cst_lib)
    de = start_de(cst)
    try:
        de.set_quiet_mode(True)
    except Exception as e:
        log("quiet mode n/a", repr(e))
    try:
        v = de.version()
        log("CST version", v)
    except Exception as e:
        log("version n/a", repr(e))
    if a.probe:
        import pydoc
        p = os.path.join(a.logs, "cst_api.txt")
        with open(p, "w", encoding="utf-8") as fh:
            for mod in ("cst.interface", "cst.results"):
                fh.write(pydoc.render_doc(mod, renderer=pydoc.plaintext))
        log("API written to", p)
        return
    r = Runner(cst, de, a)
    n_ok = 0
    for i, c in enumerate(sel):
        if a.skip_done and done(a, c):
            continue
        log(f"[{i + 1}/{len(sel)}]")
        m = r.run_case(c)
        n_ok += m.get("status") == "ok"
    log(f"all done: {n_ok} ok of {len(sel)} selected")


if __name__ == "__main__":
    main()

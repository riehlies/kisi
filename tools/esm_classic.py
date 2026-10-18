"""Turn the field solver's ES modules into one classic script.

Module workers created from blob: URLs cannot load (or import) modules when the
viewer is opened from disk (file:// has an opaque origin, module loading needs
CORS), so the solver would fall back to a single thread on the main page. A
classic script runs everywhere: in a worker built from a blob and on the page.

Supported syntax (what js_2d_fields and the kisi adapter use; anything else
raises so an upstream change is noticed at bundle time, not at run time):
  import X from "M";  import { a, b as c } from "M";  import X, { a } from "M";
  import * as N from "M";
  export function|async function|class|const|let|var NAME ...
  export default [async] function NAME ...
  export { a, b as c };
Dynamic import() is left as is (only used by upstream's unbundled triangular backend).

Input: modules in dependency order, import specifiers already replaced by
"__KISI_MOD_<index>__". Output: a JS expression that evaluates to the exports of
the last module (the backend).
"""
import re

IMPORT_RE = re.compile(
    r"""^[ \t]*import\s+(?P<clause>[^'";]*?)\s+from\s*(['"])__KISI_MOD_(?P<idx>\d+)__\2[ \t]*;?""",
    re.M | re.S)
EXPORT_LIST_RE = re.compile(r"^[ \t]*export\s*\{(?P<names>[^}]*)\}[ \t]*;?", re.M)
EXPORT_DEFAULT_RE = re.compile(r"^([ \t]*)export\s+default\s+((?:async\s+)?function\s*\*?\s*)(?P<name>[A-Za-z_$][\w$]*)", re.M)
EXPORT_DECL_RE = re.compile(
    r"^([ \t]*)export\s+((?:async\s+)?function\s*\*?\s*|class\s+|const\s+|let\s+|var\s+)(?P<name>[A-Za-z_$][\w$]*)", re.M)
LEFTOVER_RE = re.compile(r"^[ \t]*(?:import\s+(?!\()|export\s)", re.M)


def _names(spec):
    out = []
    for part in spec.split(","):
        part = part.strip()
        if not part:
            continue
        m = re.fullmatch(r"([A-Za-z_$][\w$]*)(?:\s+as\s+([A-Za-z_$][\w$]*))?", part)
        if not m:
            raise ValueError(f"unsupported name list entry: {part!r}")
        out.append((m.group(1), m.group(2) or m.group(1)))
    return out


def _import(m):
    clause, idx = m.group("clause").strip(), m.group("idx")
    req = f"__kisiReq({idx})"
    lines = []
    ns = re.fullmatch(r"\*\s+as\s+([A-Za-z_$][\w$]*)", clause)
    if ns:
        return f"const {ns.group(1)} = {req};"
    named = re.search(r"\{([^}]*)\}", clause, re.S)
    default = re.sub(r"\{[^}]*\}", "", clause, flags=re.S).strip().rstrip(",").strip()
    if default:
        if not re.fullmatch(r"[A-Za-z_$][\w$]*", default):
            raise ValueError(f"unsupported import clause: {clause!r}")
        lines.append(f"const {default} = {req}.default;")
    if named:
        pairs = _names(named.group(1))
        lines.append("const { " + ", ".join(a if a == b else f"{a}: {b}" for a, b in pairs) + f" }} = {req};")
    if not lines:
        raise ValueError(f"unsupported import clause: {clause!r}")
    return " ".join(lines)


def convert_module(src):
    exports = []  # (exported name, local name)
    src = IMPORT_RE.sub(_import, src)

    def exp_list(m):
        exports.extend((b, a) for a, b in _names(m.group("names")))
        return ""

    src = EXPORT_LIST_RE.sub(exp_list, src)

    def exp_default(m):
        exports.append(("default", m.group("name")))
        return m.group(1) + m.group(2) + m.group("name")

    src = EXPORT_DEFAULT_RE.sub(exp_default, src)

    def exp_decl(m):
        exports.append((m.group("name"), m.group("name")))
        return m.group(1) + m.group(2) + m.group("name")

    src = EXPORT_DECL_RE.sub(exp_decl, src)
    left = LEFTOVER_RE.search(src)
    if left:
        line = src[left.start():src.find("\n", left.start())]
        raise ValueError(f"unsupported module syntax: {line.strip()[:120]!r}")
    # getters keep `export let` bindings live
    body = ", ".join(f"{e!r}: {{ get: () => {l}, enumerable: true }}" for e, l in exports)
    return src + f"\nObject.defineProperties(__exports, {{ {body} }});\n"


def classic_bundle(modules):
    """modules: list of {"name", "src"} in dependency order; returns a JS expression."""
    parts = ["(() => {", "\"use strict\";",
             "const __kisiMods = [];",
             "const __kisiReq = (i) => { const m = __kisiMods[i]; if (!m.e) { m.e = {}; m.f(m.e); } return m.e; };"]
    for i, m in enumerate(modules):
        parts.append(f"// ---- {m['name']}")
        parts.append(f"__kisiMods[{i}] = {{ f: function (__exports) {{\n{convert_module(m['src'])}}} }};")
    parts.append(f"return __kisiReq({len(modules) - 1});")
    parts.append("})()")
    return "\n".join(parts)

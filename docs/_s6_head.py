#!/usr/bin/env python3
"""Section 6 data-table extractor (preprocessor-aware, dual-view)."""
import os, re, functools

REF = "/home/m-kikuchi/projects/oprs/reference/opus-1.6.1"
OUT = "/home/m-kikuchi/projects/oprs/docs/_s6.md"
INLINE_LIMIT = 600
ELIDE_FILES = {"celt/static_modes_fixed.h", "celt/static_modes_float.h",
               "celt/static_modes_fixed_arm_ne10.h",
               "celt/static_modes_float_arm_ne10.h"}
ALWAYS_ELIDE = {"src/mapping_matrix.c", "src/mlp_data.c", "src/mlp.c"}
ALWAYS_INLINE = {"celt/cwrs.c"}
SKIP_DIR = re.compile(r"/(x86|arm|mips|tests?)(/|$)")
SKIP_HDR = {"MacroCount.h", "MacroDebug.h", "fixed_debug.h"}
GUARD_RE = re.compile(r"^!?_{0,2}[A-Za-z0-9_]+_H_{0,2}$")
REAL_MACROS = [
    "FIXED_POINT", "ENABLE_QEXT", "ENABLE_RES24", "ENABLE_DRED",
    "ENABLE_DEEP_PLC", "ENABLE_OSCE", "ENABLE_OSCE_BWE",
    "ENABLE_OSCE_TRAINING_DATA", "DISABLE_FLOAT_API", "FLOAT_APPROX",
    "CUSTOM_MODES", "ENABLE_OPUS_CUSTOM_API", "SMALL_FOOTPRINT",
    "CWRS_EXTRA_ROWS", "OPUS_ARM_ASM", "OPUS_ARM_INLINE_ASM",
    "OPUS_ARM_INLINE_EDSP", "OPUS_ARM_INLINE_MEDIA", "OPUS_ARM_INLINE_NEON",
    "OPUS_ARM_MAY_HAVE_EDSP", "OPUS_ARM_MAY_HAVE_MEDIA",
    "OPUS_ARM_MAY_HAVE_NEON", "OPUS_ARM_MAY_HAVE_NEON_INTR",
    "OPUS_ARM_MAY_HAVE_DOTPROD", "OPUS_ARM_PRESUME_NEON_INTR",
    "OPUS_ARM_PRESUME_AARCH64_NEON_INTR", "OPUS_ARM_PRESUME_DOTPROD",
    "OPUS_X86_MAY_HAVE_SSE", "OPUS_X86_MAY_HAVE_SSE2",
    "OPUS_X86_MAY_HAVE_SSE4_1", "OPUS_X86_MAY_HAVE_AVX2",
    "OPUS_X86_PRESUME_SSE", "OPUS_X86_PRESUME_SSE2",
    "OPUS_X86_PRESUME_SSE4_1", "OPUS_X86_PRESUME_AVX2",
    "OPUS_HAVE_RTCD", "OPUS_CHECK_ASM", "OPUS_FAST_INT64",
    "HAVE_ARM_NE10", "USE_SMALL_DIV_TABLE", "DISABLE_DOT_PROD",
    "ENABLE_ASSERTIONS", "ENABLE_HARDENING", "FUZZING", "ENABLE_LOSSGEN",
    "MIPS_ASM", "FIXED_DEBUG", "FIXED_POINT_DEBUG", "USE_ALLOCA",
    "VAR_ARRAYS", "NONTHREADSAFE_PSEUDOSTACK", "DISABLE_DEBUG_FLOAT",
    "OPUS_ARM_MAY_HAVE_NEON", "ENABLE_OSCE",
]
REAL_MACRO_RE = re.compile(r"\b(" + "|".join(REAL_MACROS) + r")\b")
PP_RE = re.compile(r"^[ \t]*#[ \t]*(ifdef|ifndef|if|elif|else|endif)\b[ \t]*(.*)$")


def read(p):
    return open(os.path.join(REF, p), errors="replace").read()


def src_files():
    out = []
    for root in ("celt", "silk", "src"):
        for dp, dn, fn in os.walk(os.path.join(REF, root)):
            rel = os.path.relpath(dp, REF)
            if SKIP_DIR.search(rel):
                continue
            for f in sorted(fn):
                if f.endswith((".c", ".h")) and f not in SKIP_HDR:
                    out.append(os.path.join(rel, f))
    return sorted(out)


def blank_comments(s):
    out = list(s); i, n = 0, len(s)
    while i < n:
        if s.startswith("/*", i):
            j = s.find("*/", i + 2); j = n if j < 0 else j + 2
            for k in range(i, j):
                if out[k] != "\n":
                    out[k] = " "
            i = j
        elif s.startswith("//", i):
            j = s.find("\n", i); j = n if j < 0 else j
            for k in range(i, j):
                out[k] = " "
            i = j
        else:
            i += 1
    return "".join(out)


def select_view(raw, mode):
    """'first' keeps every #if branch; 'last' keeps every #else branch."""
    out, stack = [], []
    for ln in raw.split("\n"):
        m = PP_RE.match(ln)
        if m:
            kind = m.group(1)
            if kind in ("ifdef", "ifndef", "if"):
                parent = all(f[0] for f in stack)
                stack.append([parent and (mode == "first"), True])
            elif kind in ("elif", "else"):
                if stack:
                    parent = all(f[0] for f in stack[:-1])
                    stack[-1] = [parent and (mode == "last"), False]
            elif kind == "endif":
                if stack:
                    stack.pop()
            out.append(" " * len(ln))
            continue
        out.append(ln if all(f[0] for f in stack) else " " * len(ln))
    return "\n".join(out)


def pp_guards(raw):
    res, stack = [], []
    for ln in raw.split("\n"):
        m = PP_RE.match(ln)
        if m:
            kind, rest = m.group(1), m.group(2).strip()
            if kind == "ifdef":
                stack.append(rest)
            elif kind == "ifndef":
                stack.append("!" + rest)
            elif kind == "if":
                stack.append(rest)
            elif kind == "elif":
                if stack:
                    stack[-1] = rest
            elif kind == "else":
                if stack:
                    stack[-1] = "!(" + stack[-1] + ")"
            elif kind == "endif":
                if stack:
                    stack.pop()
        res.append([c for c in stack if not GUARD_RE.match(c)
                    and (("defined(" in c) or REAL_MACRO_RE.search(c))])
    return res


DECL_RE = re.compile(
    r"(?m)^(?P<ind>[ \t]*)(?P<decl>(?:(?:static|extern|const|OPUS_[A-Z_]+|"
    r"unsigned|signed|struct[ \t]+[A-Za-z_][A-Za-z0-9_]*|"
    r"[A-Za-z_][A-Za-z0-9_]*)[ \t\*]+)+)"
    r"(?P<name>[A-Za-z_][A-Za-z0-9_]*)"
    r"(?P<dims>(?:[ \t]*\[[^\]]*\])+)[ \t]*=")


def strip_outer(b):
    b = b.strip()
    if b.startswith("{"):
        b = b[1:]
    if b.endswith("}"):
        b = b[:-1]
    return b


def count_scalars(body):
    b = list(strip_outer(body)); pd = 0
    for i, c in enumerate(b):
        if c == "(":
            pd += 1
        elif c == ")":
            pd = max(0, pd - 1)
        elif c == "," and pd > 0:
            b[i] = ";"
    txt = "".join(b).replace("{", ",").replace("}", ",")
    return sum(1 for tok in txt.split(",") if tok.strip())


def count_groups(body):
    b = strip_outer(body)
    n = depth = pd = 0
    intok = False
    for c in b:
        if c == "(":
            pd += 1
            if depth == 0 and not intok:
                n += 1; intok = True
        elif c == ")":
            pd = max(0, pd - 1)
        elif c == "{" and pd == 0:
            if depth == 0 and not intok:
                n += 1
            depth += 1; intok = False
        elif c == "}" and pd == 0:
            depth = max(0, depth - 1); intok = False
        elif c == "," and depth == 0 and pd == 0:
            intok = False
        elif c.isspace():
            pass
        else:
            if depth == 0 and not intok:
                n += 1; intok = True
    return n


def extract(path, raw, view, guards, mode):
    bl = blank_comments(view)
    out = []; last_end = -1
    for m in DECL_RE.finditer(bl):
        if m.start() < last_end:
            continue
        name = m.group("name")
        dims = re.sub(r"\s+", " ", m.group("dims")).strip()
        k = m.end()
        while k < len(bl) and bl[k] in " \t\r\n":
            k += 1
        if k >= len(bl) or bl[k] != "{":
            continue
        depth, j = 0, k
        while j < len(bl):
            if bl[j] == "{":
                depth += 1
            elif bl[j] == "}":
                depth -= 1
                if depth == 0:
                    break
            j += 1
        if j >= len(bl):
            continue
        body, body_b = raw[k:j + 1], bl[k:j + 1]
        last_end = j + 1
        t = raw[:m.start()].count("\n") + 1
        e = raw[:j].count("\n") + 1
        dc = [int(x.strip()) if x.strip().isdigit() else None
              for x in re.findall(r"\[([^\]]*)\]", dims)]
        out.append(dict(
            path=path, line=t, endline=e, name=name, decl=m.group("decl").strip(),
            dims=dims, body=body, count=count_scalars(body_b),
            groups=count_groups(body_b),
            scope="file" if m.group("ind") == "" else "function-local",
            ifdef=" && ".join(guards[t - 1]) if guards[t - 1] else "",
            ndims=dims.count("["), nested=("{" in body_b[1:-1]),
            viewmode=mode,
            decl_count=(None if (not dc or any(x is None for x in dc))
                        else functools.reduce(lambda a, b: a * b, dc, 1))))
    return out


ALL = {}
for path in src_files():
    raw = read(path); guards = pp_guards(raw); byline = {}
    for mode in ("first", "last"):
        for t in extract(path, raw, select_view(raw, mode), guards, mode):
            byline.setdefault((t["line"], t["name"]), []).append(t)
    for k, lst in byline.items():
        t = lst[0]; other = lst[1] if len(lst) > 1 else None
        if other and other["body"] == t["body"] and other["dims"] == t["dims"]:
            t["bidir"] = True
            ALL[(path, t["line"], t["name"])] = t
        elif other:
            t["variant"] = "`#if` branch"; other["variant"] = "`#else` branch"
            ALL[(path, t["line"], t["name"])] = t
            ALL[(path, other["line"], other["name"] + "#else")] = other
        else:
            if t["viewmode"] == "last":
                t["variant"] = "`#else` branch only"
            ALL[(path, t["line"], t["name"])] = t
T = sorted(ALL.values(), key=lambda t: (t["path"], t["line"]))

EX = []
for path in src_files():
    raw = read(path); guards = pp_guards(raw); bl = blank_comments(raw)
    for m in re.finditer(
            r"(?m)^([ \t]*)(extern[ \t]+(?:const[ \t]+)?[A-Za-z_][A-Za-z0-9_ \t\*]*?)"
            r"([A-Za-z_][A-Za-z0-9_]*)((?:[ \t]*\[[^\]]*\])+)[ \t]*;", bl):
        line = raw[:m.start()].count("\n") + 1
        EX.append((path, line, re.sub(r"\s+", " ", m.group(0).strip()),
                   " && ".join(guards[line - 1])))


def inlined(t):
    if t["path"] in ELIDE_FILES or t["path"] in ALWAYS_ELIDE:
        return False
    if t["path"] in ALWAYS_INLINE:
        return True
    return max(t["count"], t["groups"]) <= INLINE_LIMIT


def note_for(t):
    bits = []
    if t["ndims"] >= 2:
        bits.append("%d-D array" % t["ndims"])
    elif t["nested"]:
        bits.append("array of aggregates (nested initialiser)")
    else:
        bits.append("flat array")
    bits.append("**declared %s, %d top-level group(s), %d scalar leaf/leaves "
                "in the initialiser**"
                % (t["decl_count"] if t["decl_count"] is not None else "?",
                   t["groups"], t["count"]))
    bits.append("`%s`" % t["dims"])
    if t["ifdef"]:
        bits.append("**`#if %s`**" % t["ifdef"])
    if t.get("variant"):
        bits.append(t["variant"])
    if t.get("bidir"):
        bits.append("identical in both preprocessor branches")
    if t["scope"] == "function-local":
        bits.append("function-local `static`")
    if t["path"] in ELIDE_FILES:
        bits.append("generated-mode table (values elided, see §6.6)")
    if t["path"] in ALWAYS_ELIDE:
        bits.append("non-core (wrapper / float-API); values elided")
    return "; ".join(bits)


def code(text):
    return "```c\n" + text.rstrip("\n") + "\n```"


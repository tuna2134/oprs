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



o = []
o.append("## 6. Data tables\n")
o.append("Exhaustive manifest of every constant table in the portable core "
         "(`celt/`, `silk/`, `src/`), with the **exact C declaration text**. "
         "SIMD-only directories, `celt/tests`, and the debug/macro-count headers "
         "are excluded.\n")
o.append("**Method.** The extractor is preprocessor-aware: it builds two "
         "*selected views* of each source file — first-branch-of-every-`#if` and "
         "last-branch-of-every-`#if` — and extracts tables from both, because "
         "some tables put the **declaration** inside `#if/#else` (e.g. "
         "`CELT_PVQ_U_DATA[1488]` vs `[1272]`, `celt/cwrs.c:215-217`) and others "
         "put the **initialiser** inside `#if/#else` (e.g. "
         "`fft_twiddles48000_960`, `celt/static_modes_fixed.h:159-483`). Where "
         "the two views differ, both variants are listed and tagged.\n")
o.append("**Counts.** *Declared* is the product of the numeric array dimensions. "
         "*Groups* is the number of comma-separated items at brace depth 1 (an "
         "aggregate such as `{r,i}` counts as one group). *Leaves* is the total "
         "number of leaf values. So a flat scalar array has "
         "`groups == leaves == declared`; an array of 2-field structs has "
         "`groups == declared` and `leaves == 2*declared`; a 2-D scalar array "
         "`[R][C]` has `groups == R` and `leaves == declared == R*C`. Commas "
         "inside parentheses (macro invocations such as `QCONST16(x,15)`) are not "
         "separators.\n")
o.append("Tables with more than %d elements, and every table in the generated "
         "mode headers, have their initialiser elided and are referenced by "
         "`file:line` for mechanical extraction (`sed -n 'A,Bp'`).\n"
         % INLINE_LIMIT)

o.append("### 6.1 Summary index\n")
o.append("| Symbol | File:line | C type (verbatim head) | Dims | Declared | Groups | Leaves | Scope | `#if` guard | Note |")
o.append("|---|---|---|---|---:|---:|---:|---|---|---|")
for t in T:
    note = "elided" if not inlined(t) else "inlined"
    if t["path"] in ELIDE_FILES:
        note = "generated/elided"
    if t.get("variant"):
        note = "variant"
    o.append("| `%s` | `%s:%d` | `%s` | `%s` | %s | %d | %d | %s | %s | %s |" % (
        t["name"], t["path"], t["line"], t["decl"].replace("|", "\\|"),
        t["dims"],
        ("%d" % t["decl_count"]) if t["decl_count"] is not None else "—",
        t["groups"], t["count"], t["scope"],
        ("`%s`" % t["ifdef"].replace("|", "\\|")) if t["ifdef"] else "—",
        note))
o.append("")
o.append("**Total: %d constant tables** (%d in `celt/`, %d in `silk/`, %d in "
         "`src/`), plus %d `extern` declarations.\n" % (
             len(T), sum(1 for t in T if t["path"].startswith("celt/")),
             sum(1 for t in T if t["path"].startswith("silk/")),
             sum(1 for t in T if t["path"].startswith("src/")), len(EX)))

for title, pred in [("6.2 CELT core tables", lambda t: t["path"].startswith("celt/")),
                    ("6.3 SILK tables", lambda t: t["path"].startswith("silk/")),
                    ("6.4 Opus-layer tables", lambda t: t["path"].startswith("src/"))]:
    sel = [t for t in T if pred(t)]
    o.append("### " + title + "\n")
    if not sel:
        o.append("_(none)_\n")
        continue
    cur = None
    for t in sel:
        if t["path"] != cur:
            cur = t["path"]
            o.append("#### `%s`\n" % cur)
        label = t["name"]
        if t.get("variant", "").startswith("`#else`"):
            label += " (the `#else` variant)"
        o.append("**`%s`** — `%s:%d-%d` — %s\n" % (
            label, t["path"], t["line"], t["endline"], note_for(t)))
        if inlined(t):
            o.append(code(t["body"]))
        else:
            o.append(code(t["decl"] + t["dims"] + " = { ... };"))
            o.append("_Full initialiser: `%s:%d-%d` — extract with "
                     "`sed -n '%d,%dp' %s`._\n" % (t["path"], t["line"],
                                                   t["endline"], t["line"],
                                                   t["endline"], t["path"]))
        o.append("")

o.append("### 6.5 `extern` table declarations\n")
o.append("These are the `tables.h`-style declarations; the definitions are the "
         "tables listed in §6.1-6.4. A Rust port needs the *definition*, so these "
         "rows exist to prove completeness of the manifest and to show which "
         "symbols are shared across translation units.\n")
o.append("| Declaration | File:line | `#if` guard |")
o.append("|---|---|---|")
for path, line, txt, cond in sorted(EX, key=lambda x: (x[0], x[1])):
    o.append("| `%s` | `%s:%d` | %s |" % (
        txt.replace("|", "\\|"), path, line,
        ("`%s`" % cond.replace("|", "\\|")) if cond else "—"))
o.append("")

o.append("""### 6.6 The generated mode tables

`celt/static_modes_fixed.h` (4446 lines) and `celt/static_modes_float.h` (2516
lines) are **generated**, not hand-written. `celt/modes.c` includes one of them
based on `FIXED_POINT`:

```c
#ifdef FIXED_POINT
 #include "static_modes_fixed.h"
#else
 #include "static_modes_float.h"
#endif
```

A Rust port must **not** hand-transcribe them. Generate Rust tables by running
the C `opus_custom_mode_create(sampling_rate, frame_size, NULL)` for every mode
(`8000/12000/16000/24000/48000`, plus `96000` under `ENABLE_QEXT`) and
serialising the resulting `CELTMode` fields. Full symbol list in §6.1; the
significant ones:

| Symbol | File:line | Type | Declared | Meaning |
|---|---|---|---|---|
| `window120`, `window240` | `celt/static_modes_fixed.h:14`, `:1571` | `celt_coef[]` | 120 / 240 | MDCT overlap windows (Q15, or Q31 under `ENABLE_QEXT`); two variants each |
| `logN400` | `:71`, `:1676` | `opus_int16[21]` | 21 | Q3 log2 of each band's MDCT size |
| `cache_index50` | `:77`, `:1682` | `opus_int16[105]` | 105 | `PulseCache.index` |
| `cache_bits50` | `:86`, `:1691` | `unsigned char[392]` | 392 | `PulseCache.bits` |
| `cache_caps50` | `:114`, `:1719` | `unsigned char[168]` | 168 | `PulseCache.caps` |
| `qext_cache_index50`, `qext_cache_bits50`, `qext_cache_caps50` | `:132`, `:138`, `:145` | `opus_int16[70]`, `unsigned char[86]`, `unsigned char[112]` | 70 / 86 / 112 | QEXT pulse cache (`ENABLE_QEXT`) |
| `fft_twiddles48000_960` | `:159` | `kiss_twiddle_cpx[480]` | 480 | FFT twiddles, two variants under `ENABLE_QEXT` |
| `fft_twiddles96000_1920` | `:1764` | `kiss_twiddle_cpx[960]` | 960 | QEXT 96 kHz FFT twiddles |
| `fft_bitrev480/240/120/60/960` | `:646`, `:684`, `:706`, `:720`, `:2731` | `opus_int16[]` | 480/240/120/60/960 | Bit-reversal permutations |
| `mdct_twiddles960`, `mdct_twiddles1920` | `:820`, `:2965` | `celt_coef[1800]`, `[3600]` | 1800 / 3600 | MDCT trig tables (`1920` is `ENABLE_QEXT`) |
| `static_mode_list` | `:4437`, `:4443` | `const CELTMode * const [TOTAL_MODES]` | 7 | The mode objects; two variants under `ENABLE_QEXT` |
| `band_allocation`, `eband5ms`, `bark_freq` | `celt/modes.c:50`, `:42`, `:82` | `unsigned char[]`, `opus_int16[]`, `opus_int16[26]` | 231 / 22 / 26 | Hand-written base data used to **build** the modes; worth transcribing directly |
| `qext_eBands_180`, `qext_logN_180`, `qext_eBands_240`, `qext_logN_240` | `celt/modes.c:486`, `:491`, `:494`, `:499` | `opus_int16[]` | 15 / 15 / 15 / 14 | QEXT band layout (EBands indices and log2 sizes) |

`celt/static_modes_*_arm_ne10.h` (388/404 lines) are the NE10 variants; SIMD-only,
must **not** be ported.

### 6.7 `#ifdef`-conditioned tables, duplicate symbols and zero padding
""")

mism = [t for t in T if t["decl_count"] is not None
        and t["decl_count"] not in (t["count"], t["groups"])]
o.append("**Declared size vs. initialiser count.** %d tables declare a size that "
         "matches neither the group count nor the leaf count. In each case the C "
         "compiler zero-fills the remainder, and the Rust port must reproduce "
         "that:\n" % len(mism))
o.append("| Symbol | File:line | Declared | Groups | Leaves | Note |")
o.append("|---|---|---:|---:|---:|---|")
for t in sorted(mism, key=lambda t: (t["path"], t["line"])):
    if t["name"] == "SMALL_DIV_TABLE":
        note = ("**index 0 is deliberately uninitialised and never read** "
                "(`celt_udiv` uses indices 1..128 only, `celt/entcode.h:126-140`). "
                "Reproduce as 129 entries with `[0] = 0`.")
    elif t["ndims"] >= 2 and t["decl_count"] == t["count"]:
        note = "2-D scalar array; `groups` is the outer dimension, `leaves` matches."
    elif t["decl_count"] > max(t["count"], t["groups"]):
        note = "**padded with implicit zeros**"
    else:
        note = "check manually (partial initialiser)"
    o.append("| `%s` | `%s:%d` | %d | %d | %d | %s |" % (
        t["name"], t["path"], t["line"], t["decl_count"], t["groups"],
        t["count"], note))
o.append("")

guards = {}
for t in T:
    if t["ifdef"]:
        guards.setdefault(t["ifdef"], set()).add(t["name"])
o.append("**`#if` guards that change tables** (%d of %d tables sit inside at "
         "least one conditional):\n" % (sum(1 for t in T if t["ifdef"]), len(T)))
o.append("| Guard | Tables it gates | Consequence for a Rust port |")
o.append("|---|---|---|")
GUARD_NOTES = [
 ("FIXED_POINT", "selects `static_modes_fixed.h` vs `static_modes_float.h` — **different element types and values for the same symbols** — and the fixed branch of `celt/mathops.h`."),
 ("ENABLE_QEXT", "adds the `qext_*` pulse caches, `mdct_twiddles1920`, `fft_twiddles96000_1920`, `fft_bitrev960`, `qext_eBands_180/240`, `qext_logN_180/240`, a second `static_mode_list`, and a second variant of `fft_twiddles48000_960`/`window120`/`window240`/`mdct_twiddles960`. It also changes `celt_coef` (opus_val16 → opus_val32), changing the element type of every `celt_coef` table."),
 ("CUSTOM_MODES", "adds `toOpusTable[20]` / `fromOpusTable[16]` (`celt/celt.h:199-215`), the non-Opus ↔ Opus TOC-byte mapping."),
 ("ENABLE_OPUS_CUSTOM_API", "same table set as `CUSTOM_MODES` (`celt/celt.h:199-215`)."),
 ("CWRS_EXTRA_ROWS", "changes `CELT_PVQ_U_DATA` from `[1272]` to `[1488]` (`celt/cwrs.c:215-217`). **Not set by any build system** — an embedder macro; the default is the `#else` branch, `[1272]`."),
 ("SMALL_FOOTPRINT", "removes the tabulated `V(N,K)` fast path in `celt/cwrs.c` and shrinks `silk/PLC.c` windows. **Not set by configure/CMake/meson**; default off."),
 ("USE_SMALL_DIV_TABLE", "`OPUS_ARM_ASM` defines it (`celt/entcode.h:39-41`), switching `celt_udiv` to `SMALL_DIV_TABLE[129]`. The table is always compiled; only its *use* is conditional."),
 ("ENABLE_RES24", "changes no table; it changes the Q domain of `opus_res` variables."),
 ("DISABLE_FLOAT_API", "removes `src/analysis.c`, `src/mlp.c`, `src/mlp_data.c` from the build, so every `src/mlp_data.c` table disappears. No core table changes."),
 ("FLOAT_APPROX", "selects inline approximations in `celt/mathops.h`; no table changes."),
]
seen = set()
for g, note in GUARD_NOTES:
    names = sorted({n for gg, ns in guards.items() for n in ns if g in gg})
    if not names:
        continue
    seen.add(g)
    o.append("| `%s` | %d: %s | %s |" % (
        g.replace("|", "\\|"), len(names),
        ", ".join("`%s`" % n for n in names[:10]) + (" …" if len(names) > 10 else ""),
        note))
for g, ns in sorted(guards.items()):
    if any(k in g for k in seen):
        continue
    o.append("| `%s` | %d: %s | — |" % (
        g.replace("|", "\\|"), len(ns), ", ".join("`%s`" % n for n in sorted(ns)[:10])))
o.append("")
o.append("**Duplicate symbol names across files.** `logN400`, `cache_index50`, "
         "`cache_bits50`, `cache_caps50`, `qext_cache_*`, `fft_bitrev*`, "
         "`window120/240`, `mdct_twiddles*`, `fft_twiddles*` each appear in "
         "**both** generated mode headers and sometimes twice within one header "
         "(different sampling rate). They are `static` (file-local) and the header "
         "is included once, so exactly one instance exists per build. In Rust this "
         "becomes one module per mode, or a mode-parameterised table set.\n")
o.append("**Ordering matters.** `fft_bitrev*` and `fft_twiddles*` are indexed by "
         "the FFT stage order produced by `kf_factor()`; do not re-sort or "
         "re-derive them from a generic FFT library.\n")

open(OUT, "w").write("\n".join(o))
print("wrote", OUT, sum(1 for _ in open(OUT)), "lines;", len(T), "tables;",
      len(EX), "externs;", len(mism), "size mismatches")
for t in mism:
    print("   %-44s %-32s decl=%s groups=%d leaves=%d"
          % (t["path"], t["name"], t["decl_count"], t["groups"], t["count"]))

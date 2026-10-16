

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

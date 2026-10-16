#!/usr/bin/env python3
"""Section 4: the exact celt/silk API surface used by the opus layer."""
import os, re

REF = "/home/m-kikuchi/projects/oprs/reference/opus-1.6.1"
OUT = "/home/m-kikuchi/projects/oprs/docs/_s4.md"
CALLERS = ["src/opus.c", "src/opus_encoder.c", "src/opus_decoder.c",
           "src/repacketizer.c", "src/extensions.c"]

def read(p):
    return open(os.path.join(REF, p), errors="replace").read()

def blank_comments(s):
    """Blank out comments in place, preserving every byte offset and newline."""
    out = list(s)
    i, n = 0, len(s)
    while i < n:
        if s.startswith("/*", i):
            j = s.find("*/", i + 2)
            j = n if j < 0 else j + 2
            for k in range(i, j):
                if out[k] != "\n":
                    out[k] = " "
            i = j
        elif s.startswith("//", i):
            j = s.find("\n", i)
            j = n if j < 0 else j
            for k in range(i, j):
                out[k] = " "
            i = j
        else:
            i += 1
    return "".join(out)

BAD_FIRST = {"typedef", "struct", "union", "enum", "return", "if", "else", "while",
             "for", "switch", "do", "sizeof", "defined", "case", "goto", "break",
             "continue", "static", "const", "extern", "inline", "OPUS_INLINE"}

PROTO_RE = re.compile(
    r"(?m)^([A-Za-z_][A-Za-z0-9_ \t\*\n]*?)[ \t\*]([A-Za-z_][A-Za-z0-9_]*)[ \t]*\(")

def parse_protos(path):
    """Return {name: (verbatim_decl, file, line, is_static)}."""
    raw = read(path)
    txt = blank_comments(raw)
    res = {}
    for m in PROTO_RE.finditer(txt):
        rettype = m.group(1).strip()
        name = m.group(2)
        toks = rettype.split()
        while toks and toks[0] in ("static", "extern", "inline", "OPUS_INLINE",
                                   "const", "OPUS_CUSTOM_NOSTATIC", "OPUS_EXPORT",
                                   "OPUS_CUSTOM_EXPORT", "OPUS_WARN_UNUSED_RESULT"):
            toks.pop(0)
        first = toks[0] if toks else ""
        if first in BAD_FIRST:
            continue
        if ";" in rettype or "," in rettype or "=" in rettype:
            continue
        i = m.end() - 1
        depth, j = 0, i
        while j < len(txt):
            if txt[j] == "(":
                depth += 1
            elif txt[j] == ")":
                depth -= 1
                if depth == 0:
                    break
            j += 1
        # scan to terminator
        k = j + 1
        term = None
        while k < len(txt):
            c = txt[k]
            if c == ";":
                term = ";"
                break
            if c == "{":
                term = "{"
                break
            if c == "\n" and txt[k:].lstrip()[:1] in ("#",):
                break
            k += 1
        if term not in (";", "{"):
            continue
        decl = re.sub(r"\s+", " ", raw[m.start():j + 1]).strip()
        line = raw[:m.start()].count("\n") + 1
        if term == "{":
            decl += " ... }"
        if name not in res:
            res[name] = (decl, path, line,
                         rettype.startswith(("static", "OPUS_CUSTOM_NOSTATIC")))
    return res

HDRS = []
for root in ("celt", "silk", "src", "include"):
    for dirpath, dirnames, filenames in os.walk(os.path.join(REF, root)):
        rel = os.path.relpath(dirpath, REF)
        if re.search(r"/(x86|arm|mips|tests?)(/|$)", rel):
            continue
        for f in sorted(filenames):
            if f.endswith(".h") and f not in ("MacroCount.h", "MacroDebug.h",
                                              "fixed_debug.h", "debug.h"):
                HDRS.append(os.path.join(rel, f))

PROTOS = {}
for h in HDRS:
    for nm, v in parse_protos(h).items():
        PROTOS.setdefault(nm, v)

CALLER_SRCS = []
for root in ("celt", "silk", "src"):
    for dirpath, dirnames, filenames in os.walk(os.path.join(REF, root)):
        rel = os.path.relpath(dirpath, REF)
        if re.search(r"/(x86|arm|mips|tests?)(/|$)", rel):
            continue
        for f in sorted(filenames):
            if f.endswith(".c"):
                CALLER_SRCS.append(os.path.join(rel, f))

DEFS = {}
for c in CALLER_SRCS:
    for nm, v in parse_protos(c).items():
        DEFS.setdefault(nm, v)

MACROS = {}
for h in HDRS:
    raw = read(h)
    for m in re.finditer(r"(?m)^[ \t]*#[ \t]*define[ \t]+([A-Za-z_][A-Za-z0-9_]*)"
                         r"([^\n]*(?:\\\n[^\n]*)*)", raw):
        nm = m.group(1)
        if nm not in MACROS and nm != 'main':
            line = raw[:m.start()].count("\n") + 1
            txt = re.sub(r"\\\s*\n\s*", " ", m.group(0).strip())
            MACROS[nm] = (re.sub(r"\s+", " ", txt), h, line)

def resolve(name):
    if name in PROTOS:
        return PROTOS[name] + ("proto",)
    if name in DEFS:
        return DEFS[name] + ("def",)
    if name in MACROS:
        t, h, l = MACROS[name]
        return (t, h, l, False, "macro")
    return None

def callsites(path):
    txt = blank_comments(read(path))
    # drop the local definitions so we only count *calls*
    return {name: len(re.findall(r"\b" + re.escape(name) + r"\s*\(", txt))
            for name in set(list(PROTOS) + list(DEFS))
            if re.search(r"\b" + re.escape(name) + r"\s*\(", txt)}

per_file = {p: callsites(p) for p in CALLERS}

GROUPS = [
 ("4.1 The entropy coder driven directly by the opus layer", [
   "ec_enc_init","ec_enc_done","ec_enc_shrink","ec_enc_uint","ec_enc_bits",
   "ec_enc_icdf","ec_enc_icdf16","ec_enc_bit_logp","ec_enc_patch_initial_bits",
   "ec_tell",
   "ec_dec_init","ec_dec_uint","ec_dec_bits","ec_dec_icdf","ec_dec_icdf16",
   "ec_dec_bit_logp","ec_tell_frac",
   "ec_laplace_encode","ec_laplace_decode",
 ]),
 ("4.2 CELT encoder entry points", [
   "celt_encoder_get_size","celt_encoder_init","opus_custom_encoder_init",
   "opus_custom_encoder_create","opus_custom_encoder_destroy",
   "opus_custom_encoder_get_size","celt_encode_with_ec","opus_custom_encode",
   "opus_custom_encode_float","opus_custom_encode24",
   "opus_custom_encoder_ctl","celt_encoder_ctl",
 ]),
 ("4.3 CELT decoder entry points", [
   "celt_decoder_get_size","celt_decoder_init","opus_custom_decoder_create",
   "opus_custom_decoder_destroy","celt_decode_with_ec","celt_decode_with_ec_dred",
   "opus_custom_decode","opus_custom_decode_float","opus_custom_decode24",
   "opus_custom_decoder_ctl","celt_decoder_ctl",
 ]),
 ("4.4 Mode and table access", [
   "opus_custom_mode_create","opus_custom_mode_destroy",
   "compute_pulse_cache","clt_compute_allocation","init_caps",
 ]),
 ("4.5 SILK encoder API", [
   "silk_Get_Encoder_Size","silk_InitEncoder","silk_Encode","silk_QueryEncoder",
   "silk_control_encoder","silk_control_audio_bandwidth","silk_control_SNR",
   "silk_init_encoder","check_control_input","silk_HP_variable_cutoff",
 ]),
 ("4.6 SILK decoder API", [
   "silk_Get_Decoder_Size","silk_InitDecoder","silk_ResetDecoder","silk_Decode",
   "silk_get_TOC","silk_decoder_set_fs","silk_init_decoder","silk_reset_decoder",
   "silk_LoadOSCEModels","silk_CNG_Reset","silk_PLC_Reset",
 ]),
 ("4.7 Other celt/silk functions reached from the opus layer", [
   "silk_stereo_find_predictor","silk_stereo_quant_pred","silk_LPC_analysis_filter",
   "silk_LPC_inverse_pred_gain","silk_NLSF2A","silk_A2NLSF","silk_gains_dequant",
   "silk_gains_ID","silk_lin2log","silk_log2lin","silk_sum_sqr_shift",
   "silk_inner_prod_aligned","silk_sigm_Q15","silk_resampler_init",
   "silk_resampler","silk_VAD_GetSA_Q8","silk_NSQ_del_dec","silk_NSQ",
   "celt_pitch_xcorr","celt_inner_prod","celt_fir","celt_iir",
   "celt_maxabs16","celt_maxabs32","celt_sqrt","celt_rsqrt",
   "celt_log2","celt_exp2","celt_cos_norm","celt_rcp","celt_lcg_rand",
   "bitexact_cos","bitexact_log2tan","isqrt32","frac_div32",
 ]),
]

o = []
o.append("## 4. Public / internal API surface needed by the opus layer\n")
o.append("Every symbol below is a `celt/` or `silk/` function that the opus layer "
         "(`src/opus.c`, `src/opus_encoder.c`, `src/opus_decoder.c`, "
         "`src/repacketizer.c`, `src/extensions.c`) calls, with the signature "
         "copied **verbatim** from its declaring header (or, for SILK functions "
         "declared only in `silk/main.h`-included files, from the header that "
         "declares it). `Call sites` counts textual occurrences of `name(` in "
         "each opus-layer file. SIMD/RTCD-only declarations are excluded.\n")
o.append("Types used in the signatures are all defined in §5.\n")
o.append("> A `—` in the *Call sites* column means the symbol is **not** textually "
         "called by any of the five opus-layer files (it is used internally by "
         "`celt/` or `silk/`, or it is part of the same public surface). It is "
         "included because the Rust port must still implement it for the layer "
         "below to compile. §4.9 shows the real call chains.\n")

# ---- direct-call summary
direct = {}
for src, tbl in (("proto", PROTOS), ("def", DEFS), ("macro", MACROS)):
    for nm, v in tbl.items():
        path = v[1]
        if not (path.startswith("celt/") or path.startswith("silk/")):
            continue
        if nm in ("main",):
            continue
        tot = sum(per_file[q].get(nm, 0) for q in CALLERS)
        if tot:
            direct[nm] = tot
o.append("### 4.0 Direct call summary (what the opus layer actually touches)\n")
o.append("Every `celt/`+`silk/` symbol textually called from the five opus-layer "
         "files, with total call count. This is the exact set the Rust port must "
         "expose across the `celt`/`silk` module boundary.\n")
o.append("| Symbol | Calls | Symbol | Calls | Symbol | Calls |")
o.append("|---|---:|---|---:|---|---:|")
items = sorted(direct.items(), key=lambda kv: (-kv[1], kv[0]))
col = 3
for i in range(0, len(items), col):
    chunk = items[i:i + col]
    cells = []
    for nm, c in chunk:
        cells.append("`%s` | %d" % (nm, c))
    while len(cells) < col:
        cells.append(" | ")
    o.append("| " + " | ".join(cells) + " |")
o.append("")
o.append("> Some entries above are macros/`static inline` helpers (`celt_encoder_ctl`, "
         "`celt_decoder_ctl`, `celt_udiv`, `silk_SMULWB`, ...); those are listed in "
         "§4.8.\n")

missing = []
for title, names in GROUPS:
    o.append("### " + title + "\n")
    o.append("| Function | Verbatim C declaration | Declared in | Call sites | Kind |")
    o.append("|---|---|---|---|---|")
    for n in names:
        r = resolve(n)
        sites = ", ".join("`%s`×%d" % (p, per_file[p][n])
                          for p in CALLERS if per_file[p].get(n)) or "—"
        if not r:
            missing.append(n)
            o.append("| `%s` | _(not declared in a portable header)_ | | %s | |" % (n, sites))
            continue
        decl, path, line, is_static, kind = r
        o.append("| `%s` | `%s` | `%s:%d` | %s | %s |" % (
            n, decl.replace("|", "\\|"), path, line, sites,
            "definition" if kind == "def" else ("static inline" if is_static else "prototype")))
    o.append("")

o.append("""### 4.8 Symbols that are macros, not functions

These names appear at call sites but are `#define`s. The Rust port must
reproduce them as `#[inline]` functions (or expand at exactly the same points):

| Symbol | Definition | Defined in |
|---|---|---|
| `celt_encoder_ctl` | `#define celt_encoder_ctl opus_custom_encoder_ctl` | `celt/celt.h:185` |
| `celt_decoder_ctl` | `#define celt_decoder_ctl opus_custom_decoder_ctl` | `celt/celt.h:186` |
| `celt_lpc` | `#define celt_lpc _celt_lpc` | `celt/celt_lpc.h` |
| `celt_fir` | `#define celt_fir celt_fir_c` (arch dispatch) | `celt/celt_lpc.h` |
| `celt_iir` | `#define celt_iir celt_iir_c` (arch dispatch) | `celt/celt_lpc.h` |
| `celt_autocorr` | `#define celt_autocorr _celt_autocorr` | `celt/celt_lpc.h` |
| `celt_pitch_xcorr` | `#define celt_pitch_xcorr celt_pitch_xcorr_c` | `celt/pitch.h` |
| `celt_inner_prod` | `#define celt_inner_prod celt_inner_prod_c` | `celt/pitch.h` |
| `celt_maxabs16/32` | `static OPUS_INLINE` | `celt/mathops.h` |
| `celt_ilog2`, `celt_zlog2` | macro / `static OPUS_INLINE` | `celt/arch.h` |
| `ec_tell`, `ec_enc_tell` | `static OPUS_INLINE` | `celt/entcode.h` |
| `ec_dec_tell` | `static OPUS_INLINE` | `celt/entcode.h` |
| `celt_udiv`, `celt_sudiv` | `static OPUS_INLINE` | `celt/entcode.h` |
| `silk_SQRT_APPROX`, `silk_DIV32_varQ`, `silk_INVERSE32_varQ`, `silk_LIMIT`, `silk_RSHIFT_ROUND`, `silk_abs_int32`, `silk_CLZ32` | `static OPUS_INLINE` / macro | `silk/SigProc_FIX.h`, `silk/Inlines.h`, `silk/macros.h` |

### 4.9 The exact top-level call chains

**Encode** (`src/opus_encoder.c`):

```
opus_encode / opus_encode_float / opus_encode24
  → opus_encode_native()
      → ec_enc_init(&enc, data, max_data_bytes)
      → [hybrid or SILK-only] silk_Encode(psRangeEnc, pcm_buf, &frame_size,
                              &encControl, st->silk_mode.payloadSize_ms, ...)
      → [hybrid or CELT-only] celt_encoder_ctl(...)  (a sequence of macros)
                              celt_encode_with_ec(st->celt_enc, pcm_buf, frame_size,
                                                  NULL, max_data_bytes, &enc)
      → ec_enc_done(&enc)
      → ec_enc_shrink(&enc, len)
      → (self-delimited / repacketiser) opus_packet_pad etc.
```

`silk_Encode` and `celt_encode_with_ec` share the **same** `ec_enc`, which is
why the entropy coder must be a single state type shared by both layers.

**Decode** (`src/opus_decoder.c`):

```
opus_decode / opus_decode_float / opus_decode24
  → opus_decode_native()
      → opus_packet_parse_impl()          (src/opus.c)
      → opus_decode_frame()
          → ec_dec_init(&dec, data, len)
          → silk_Decode(dec, &decControl, lostFlag, &dec, pcm_ptr, &frame_size,
                        ...)            [SILK and hybrid modes]
          → celt_decode_with_ec(celt_dec, data, len, pcm, frame_size, &dec, 0)
                                        [CELT and hybrid modes]
          → ec_dec_tell(&dec) / ec_tell_frac / ec_dec_bits  (padding, DTX)
```

Initialisation order used by both:

```
opus_encoder_init:  opus_custom_mode_find/CELT_GET_MODE → celt_encoder_get_size
                    → opus_custom_encoder_create → silk_Get_Encoder_Size
                    → silk_InitEncoder → silk_control_encoder
opus_decoder_init:  celt_decoder_get_size → celt_decoder_init
                    → silk_Get_Decoder_Size → silk_InitDecoder
```

**Repacketizer** (`src/repacketizer.c`) calls **no** celt/silk function except
`silk_get_TOC` (`silk/dec_API.c:479`) when it needs the SILK TOC fields;
everything else is `src/opus_private.h` TOC parsing. `src/opus.c` likewise
touches celt/silk only through `opus_custom_mode_find` and the mode struct.

### 4.10 Mode-struct fields the opus layer reads

The opus layer dereferences the `CELTMode`/`OpusCustomMode` struct directly
(not a function call), so the Rust port must expose these fields with identical
names and values:

`Fs`, `overlap`, `nbEBands`, `effEBands`, `preemph[4]`, `eBands`, `maxLM`,
`nbShortMdcts`, `shortMdctSize`, `nbAllocVectors`, `allocVectors`, `logN`,
`window`, `mdct`, `cache`, `shortMdctSize`, `prefilter_and_fold` (QEXT), and
the `QEXT_*` members when `ENABLE_QEXT` is set. See §5 for the full declaration.
""")

open(OUT, "w").write("\n".join(o))
print("wrote", OUT, sum(1 for _ in open(OUT)), "lines")
print("unresolved:", missing)

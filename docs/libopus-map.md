# libopus 1.6.1 — Core Codec Engineering Map

Reference tree: `/home/m-kikuchi/projects/oprs/reference/opus-1.6.1` (read-only).
Target: a pure-Rust, **bit-exact** port of the libopus 1.6.1 **core codec**.
Explicitly **out of scope**: everything under `dnn/` (LPCNet / Deep PLC / DRED / OSCE / LossGen neural networks), and the optional float-API analysis layer (`src/analysis.c`, `src/mlp.c`, `src/mlp_data.c`) except where it is referenced from core files.

Terminology used throughout:

| Symbol | Meaning in libopus |
|---|---|
| **SILK** | linear-prediction speech codec layer (`silk/`), covers NB/MB/WB, 10/20 ms frames, up to 2 internal channels. |
| **CELT** | MDCT transform codec layer (`celt/`), covers 2.5–20 ms frames, 48 kHz, up to 2 channels. |
| **opus layer** | `src/opus.c`, `src/opus_encoder.c`, `src/opus_decoder.c`, `src/repacketizer.c`, `src/extensions.c` — mode switching, TOC/packet framing, range coder driving, repacketization. |
| **EC / entropy coder** | the range coder in `celt/entcode.[ch]`, `celt/entenc.c`, `celt/entdec.c`, `celt/laplace.c`. |
| **core codec** | all of the above; the portable C set in `celt_sources.mk:CELT_SOURCES`, `silk_sources.mk:SILK_SOURCES` (+ `SILK_SOURCES_FIXED` **or** `SILK_SOURCES_FLOAT`), `opus_sources.mk:OPUS_SOURCES`. |

Source of truth for file sets: `celt_sources.mk`, `silk_sources.mk`, `opus_sources.mk`, `lpcnet_sources.mk`, `Makefile.am`, `meson.build`, `CMakeLists.txt`.

---

## Table of contents

1. [Build configuration matrix](#1-build-configuration-matrix)
2. [File inventory](#2-file-inventory)
3. [Dependency order (bottom-up porting order)](#3-dependency-order-bottom-up-porting-order)
4. [Public / internal API surface needed by the opus layer](#4-public--internal-api-surface-needed-by-the-opus-layer)
5. [State structs](#5-state-structs)
6. [Data tables](#6-data-tables)
7. [Bit-exactness hazards](#7-bit-exactness-hazards)

Appendices:
[A — file-level call graph](#appendix-a--mechanically-extracted-file-level-call-graph) ·
[B — reproducing the reference build](#appendix-b--reproducing-the-reference-build) ·
[C — reference `config.h` macro set](#appendix-c--the-reference-configh-macro-set) ·
[D — how to use this document](#appendix-d--how-to-use-this-document)

---

## 1. Build configuration matrix

### 1.1 The two independent axes

libopus has **two orthogonal build axes**, and getting them right is a prerequisite for "bit-exact":

1. **fixed-point vs float** (`--enable-fixed-point`, `FIXED_POINT`). This changes the arithmetic domain of the *whole* codec: `opus_val16`/`opus_val32` typedefs, every `SHR`/`MULT16_16`/`SATURATE` macro, the selected mode tables (`celt/static_modes_fixed.h` vs `celt/static_modes_float.h`), and which SILK *encoder analysis* files are compiled (`silk/fixed/*` vs `silk/float/*`). The **decoder** is bit-stream-identical between the two; the **encoder** is not (the float encoder analysis path is not even bit-exact between two float platforms).
2. **internal resolution** (`--disable-fixed-res24`, `ENABLE_RES24`) **and** the scalable quality extension (`--enable-qext`, `ENABLE_QEXT`). These are the libopus 1.6.x "24-bit internal resolution" feature. In the fixed-point build, `ENABLE_RES24` switches `opus_res` from `opus_int16` to `opus_int32` with `RES_SHIFT 8` and `MAX_ENCODING_DEPTH 24`.

> **Critical finding.** `ENABLE_RES24` is defined **by default in the autotools build** (`configure.ac:150-156`) but is **never defined by `CMakeLists.txt` or `meson.build`**. `ENABLE_QEXT` is likewise **autotools-only** (`configure.ac:158-165`) and has no CMake/meson switch at all. Therefore a `./configure && make` reference build and a `cmake`/`meson` reference build of the same source produce **different bitstreams** in the fixed-point configuration. Before porting, pin the exact reference configuration and record its `config.h`; the `oprs` port must choose one, and the natural choice is "what the Opus test vectors and `opus_demo` bit-exactness tests were generated with", i.e. the **autotools default FIXED_POINT + ENABLE_RES24** path (and then a separate 16-bit path behind a feature flag).

Recommended target configuration for the port:

```
--enable-fixed-point          # FIXED_POINT=1
--enable-fixed-res24 (default) # ENABLE_RES24=1
--disable-float-api           # optional: removes src/analysis.c, mlp.c, mlp_data.c
--disable-rtcd --disable-intrinsics --disable-asm   # all SIMD/asm out of the picture
```

### 1.2 Requested options — exact effects

Every row below is copied from `configure.ac`; the `AC_ARG_ENABLE`/`AC_DEFINE` text is verbatim.

#### `--enable-fixed-point`  → `FIXED_POINT` (default: **off**, i.e. float build)

`configure.ac:115-129`:

```m4
AC_ARG_ENABLE([fixed-point],
    [AS_HELP_STRING([--enable-fixed-point],
                    [compile without floating point (for machines without a fast enough FPU)])],,
    [enable_fixed_point=no])

AS_IF([test "$enable_fixed_point" = "yes"],[
  enable_float="no"
  AC_DEFINE([FIXED_POINT], [1], [Compile as fixed-point (for machines without a fast enough FPU)])
  PC_BUILD="fixed-point"
],[
  enable_float="yes";
  PC_BUILD="floating-point"
])

AM_CONDITIONAL([FIXED_POINT], [test "$enable_fixed_point" = "yes"])
```

Preprocessor macros defined: `FIXED_POINT 1` (in `config.h`). Indirectly selects the following *branch macros inside headers* (not `AC_DEFINE`d):

| Header | Effect of `FIXED_POINT` |
|---|---|
| `celt/arch.h:130-250` | `typedef opus_int16 opus_val16; typedef opus_int32 opus_val32; typedef opus_int64 opus_val64;` plus `celt_sig/celt_norm/celt_ener/celt_glog = opus_val32`; `Q15ONE 32767`, `Q31ONE 2147483647`, `SIG_SHIFT 12`, `NORM_SHIFT 24`, `DB_SHIFT 24`, `EPSILON 1`, `VERY_SMALL 0`, `SIG_SAT 536870911`. |
| `celt/arch.h:234-256` | Includes `celt/fixed_generic.h` (or `celt/fixed_debug.h` if `FIXED_DEBUG`) instead of the float macro block. |
| `silk/typedef.h:34-37` | `silk_float` / `silk_float_MAX` are **only** defined in the float build; the fixed build uses the integer signatures in `silk/SigProc_FIX.h`. |
| `celt/modes.c:66-69` | `#include "static_modes_fixed.h"` instead of `"static_modes_float.h"`. |
| `celt/mathops.h`, `celt/float_cast.h` | `FIXED_POINT` branches for `celt_ilog2`, `celt_zlog2`, `FLOAT2INT16`. |
| `silk/*` | `silk/fixed/main_FIX.h` vs `silk/float/main_FLP.h` are selected by hand in each `.c` (`#ifdef FIXED_POINT`) and by `Makefile.am` for the source set. |

Source-set changes (`Makefile.am:32-48`):

```make
if FIXED_POINT
SILK_SOURCES += $(SILK_SOURCES_FIXED)
if HAVE_SSE4_1
SILK_SOURCES += $(SILK_SOURCES_SSE4_1) $(SILK_SOURCES_FIXED_SSE4_1)
endif
if HAVE_ARM_NEON_INTR
SILK_SOURCES += $(SILK_SOURCES_FIXED_ARM_NEON_INTR)
endif
else
SILK_SOURCES += $(SILK_SOURCES_FLOAT)
if HAVE_SSE4_1
SILK_SOURCES += $(SILK_SOURCES_SSE4_1)
endif
if HAVE_AVX2
SILK_SOURCES += $(SILK_SOURCES_FLOAT_AVX2)
endif
endif
```

* **Adds** (fixed): the 23 files of `SILK_SOURCES_FIXED` (see §2) — `silk/fixed/LTP_analysis_filter_FIX.c`, `LTP_scale_ctrl_FIX.c`, `corrMatrix_FIX.c`, `encode_frame_FIX.c`, `find_LPC_FIX.c`, `find_LTP_FIX.c`, `find_pitch_lags_FIX.c`, `find_pred_coefs_FIX.c`, `noise_shape_analysis_FIX.c`, `process_gains_FIX.c`, `regularize_correlations_FIX.c`, `residual_energy16_FIX.c`, `residual_energy_FIX.c`, `warped_autocorrelation_FIX.c`, `apply_sine_window_FIX.c`, `autocorr_FIX.c`, `burg_modified_FIX.c`, `k2a_FIX.c`, `k2a_Q16_FIX.c`, `pitch_analysis_core_FIX.c`, `vector_ops_FIX.c`, `schur64_FIX.c`, `schur_FIX.c`.
* **Adds** (fixed, intrinsics only): `SILK_SOURCES_FIXED_SSE4_1` (`silk/fixed/x86/vector_ops_FIX_sse4_1.c`, `silk/fixed/x86/burg_modified_FIX_sse4_1.c`), `SILK_SOURCES_FIXED_ARM_NEON_INTR` (`silk/fixed/arm/warped_autocorrelation_FIX_neon_intr.c`) — **not needed for the Rust port**.
* **Removes** (fixed): all 28 files of `SILK_SOURCES_FLOAT` and the 1 file of `SILK_SOURCES_FLOAT_AVX2` (`silk/float/x86/inner_product_FLP_avx2.c`).
* **CELT source set is unchanged**: the same 18 `CELT_SOURCES` files compile in both configurations. Only the *included static mode table header* changes.
* `--disable-float-api` changes `OPUS_SOURCES`, independent of `FIXED_POINT`.
* Note `pc_build`/`PC_BUILD` is also set to `fixed-point` (packaging metadata only).

#### `--enable-fixed-point-debug` → `FIXED_DEBUG` (default: **off**)

`configure.ac:131-137`:

```m4
AC_ARG_ENABLE([fixed-point-debug],
    [AS_HELP_STRING([--enable-fixed-point-debug], [debug fixed-point implementation])],,
    [enable_fixed_point_debug=no])

AS_IF([test "$enable_fixed_point_debug" = "yes"],[
  AC_DEFINE([FIXED_DEBUG], [1], [Debug fixed-point implementation])
])
```

Macro defined: **`FIXED_DEBUG 1`**.

> **⚠ The task brief's name `FIXED_POINT_DEBUG` is a different, orphan macro.** `FIXED_POINT_DEBUG` occurs exactly once in the whole tree — `celt/vq.c:287` — and is defined by **no** build system (`configure.ac`, `CMakeLists.txt`, `meson.build` all lack it). It guards only an extra `celt_sig_assert`. Grep proof:
> ```
> reference/opus-1.6.1$ grep -rn FIXED_POINT_DEBUG .
> ./celt/vq.c:287:#ifdef FIXED_POINT_DEBUG
> ```
> The real debug option is `FIXED_DEBUG`, and it is *not* free: it replaces the arithmetic macros with overflow-checked ones (`celt/arch.h:234-237`):
> ```c
> #ifdef FIXED_DEBUG
> #include "fixed_debug.h"
> #else
> #include "fixed_generic.h"
> ```
> and in SILK, `silk/MacroDebug.h:35`:
> ```c
> #if ( defined (FIXED_DEBUG) || ( 0 && defined (_DEBUG) ) ) && !defined (silk_MACRO_COUNT)
> ```
> which replaces `silk/macros.h`'s macros with versions that `silk_assert` on overflow. **No source files are added or removed**; `FIXED_DEBUG` is a pure macro/behaviour switch, and it is not a valid reference configuration for bit-exactness testing (the debug macros are semantically equivalent on non-overflowing input but may abort on edge cases that the release build silently wraps). A Rust port should implement the **`fixed_generic.h` / `silk/macros.h`** semantics and treat `fixed_debug.h`/`MacroDebug.h` as test oracles only.

#### `--disable-fixed-res24` → `ENABLE_RES24` (default: **enabled**, i.e. `ENABLE_RES24` **is** defined)

`configure.ac:150-156`:

```m4
AC_ARG_ENABLE([fixed-res24],
	      [AS_HELP_STRING([--disable-fixed-res24], [use 16-bit (instead of 24-bit) internal resolution for fixed-point implementation])],,
    [enable_fixed_res24=yes])

AS_IF([test "$enable_fixed_res24" = "yes"],[
  AC_DEFINE([ENABLE_RES24], [1], [24-bit internal resolution for fixed-point])
])
```

Macro defined: **`ENABLE_RES24 1`** (there is **no** macro named `RES24` anywhere in the tree — verify with `grep -rn '\bRES24\b'`). Default is *enabled*, so `--disable-fixed-res24` is the switch that **removes** `ENABLE_RES24`.

Effect, `celt/arch.h:143-181` (verbatim):

```c
#ifdef ENABLE_RES24
typedef opus_val32 opus_res;
#define RES_SHIFT 8
#define SIG2RES(a)      PSHR32(a, SIG_SHIFT-RES_SHIFT)
#define RES2INT16(a)    SAT16(PSHR32(a, RES_SHIFT))
#define RES2INT24(a)    (a)
#define RES2FLOAT(a)    ((1.f/32768.f/256.f)*(a))
#define INT16TORES(a)   SHL32(EXTEND32(a), RES_SHIFT)
#define INT24TORES(a)   (a)
#define ADD_RES(a, b)   ADD32(a, b)
#define FLOAT2RES(a)    FLOAT2INT24(a)
#define RES2SIG(a)      SHL32((a), SIG_SHIFT-RES_SHIFT)
#define MULT16_RES_Q15(a,b) MULT16_32_Q15(a,b)
#define MAX_ENCODING_DEPTH 24
#else
typedef opus_val16 opus_res;
#define RES_SHIFT 0
#define SIG2RES(a)      SIG2WORD16(a)
#define RES2INT16(a)    (a)
#define RES2INT24(a)    SHL32(EXTEND32(a), 8)
#define RES2FLOAT(a)    ((1.f/32768.f)*(a))
#define INT16TORES(a)   (a)
#define INT24TORES(a)   SAT16(PSHR32(a, 8))
#define ADD_RES(a, b)   SAT16(ADD32((a), (b)));
#define FLOAT2RES(a)    FLOAT2INT16(a)
#define RES2SIG(a)      SHL32(EXTEND32(a), SIG_SHIFT)
#define MULT16_RES_Q15(a,b) MULT16_16_Q15(a,b)
#define MAX_ENCODING_DEPTH 16
#endif
```

Files that read `ENABLE_RES24`: `celt/arch.h`, `celt/celt_encoder.c`, `celt/celt_decoder.c`, `celt/mathops.h`, `src/opus_encoder.c`, `src/opus_decoder.c`, `src/mapping_matrix.c`. No source file is added/removed. This is a **behaviour-defining** switch, not an optimisation: it changes the internal sample representation, the prefilter/comb-filter arithmetic, encoder rate control (`MAX_ENCODING_DEPTH`), and the `FLOAT2INT24`/`RES2INT24` conversions. **A bit-exact port must decide which of the two it implements** (recommend: `ENABLE_RES24` = autotools default, with the 16-bit path as a compile-time alternative).

#### `--enable-dred` → `ENABLE_DRED` (default: **off**)

`configure.ac:187-194`:

```m4
AC_ARG_ENABLE([dred],
    [AS_HELP_STRING([--enable-dred], [use Deep REDundancy (DRED)])],,
    [enable_dred=no])

AS_IF([test "$enable_dred" = "yes"],[
  AC_DEFINE([ENABLE_DRED], [1], [DRED])
])
AM_CONDITIONAL([ENABLE_DRED], [test "$enable_dred" = "yes"])
```

Macro defined: **`ENABLE_DRED 1`**, and *implied* **`ENABLE_DEEP_PLC 1`** (see next row, which includes `enable_dred` in its condition).

*Source-set changes* (`Makefile.am:25-26`): `LPCNET_SOURCES += $(DRED_SOURCES)` — adds the 8 DNN files `dnn/dred_rdovae_enc.c`, `dred_rdovae_enc_data.c`, `dred_rdovae_dec.c`, `dred_rdovae_dec_data.c`, `dred_rdovae_stats_data.c`, `dred_encoder.c`, `dred_coding.c`, `dred_decoder.c`. Core files that gain code: `silk/enc_API.c`, `silk/init_encoder.c`, `silk/structs.h`, `celt/celt_decoder.c`, `src/opus_decoder.c`, `src/opus_encoder.c`.

**Porting decision:** DRED is a neural codec. The core-codec port should compile with `ENABLE_DRED` **off**. But note that `src/opus_encoder.c` and `src/opus_decoder.c` contain `#ifdef ENABLE_DRED` code paths *interleaved with the core paths* (`celt_decode_with_ec_dred` vs `celt_decode_with_ec`, DRED packet padding), so the port must still understand where the hook points are, and must ensure that the DRED-off branch reproduces the reference bit-exactly.

#### `--enable-deep-plc` → `ENABLE_DEEP_PLC` (default: **off**; implied on by `dred`/`osce`)

`configure.ac:196-203`:

```m4
AC_ARG_ENABLE([deep-plc],
    [AS_HELP_STRING([--enable-deep-plc], [use deep PLC for SILK])],,
    [enable_deep_plc=no])

AS_IF([test "$enable_deep_plc" = "yes" || test "$enable_dred" = "yes" || test "$enable_osce" = "yes" || test "$enable_osce_training_data" = "yes"],[
  AC_DEFINE([ENABLE_DEEP_PLC], [1], [Deep PLC])
])
AM_CONDITIONAL([ENABLE_DEEP_PLC], [test "$enable_deep_plc" = "yes" || test "$enable_dred" = "yes" || test "$enable_osce" = "yes" || test "$enable_osce_training_data" = "yes"])
```

Macro defined: **`ENABLE_DEEP_PLC 1`**.

*Source-set changes*: `LPCNET_SOURCES += $(DEEP_PLC_SOURCES)` = 13 DNN files (`dnn/burg.c`, `freq.c`, `fargan.c`, `fargan_data.c`, `lpcnet_enc.c`, `lpcnet_plc.c`, `lpcnet_tables.c`, `nnet.c`, `nnet_default.c`, `plc_data.c`, `parse_lpcnet_weights.c`, `pitchdnn.c`, `pitchdnn_data.c`). Core files that gain `#ifdef ENABLE_DEEP_PLC` code: `celt/celt.h`, `celt/celt_decoder.c`, `silk/main.h`, `silk/dec_API.c`, `silk/API.h`, `silk/PLC.c`, `silk/decode_frame.c`, `silk/PLC.h`, `silk/structs.h`, `src/opus_decoder.c`.

**Porting decision:** off. `silk_PLC.c` calls `LPCNetPLCState` only under `#ifdef ENABLE_DEEP_PLC`; the classic SILK PLC path is the `#else` branch and is the bit-exact target.

Note: `USE_WEIGHTS_FILE` is a **DNN-only** macro (weights loaded from a file instead of compiled-in tables). It appears only in `dnn/*` and in three thin wrappers (`silk/dec_API.c`, `src/opus_encoder.c`, `src/opus_decoder.c`, `src/opus_demo.c`) where it selects the DNN weight-loading API. It is **not** an `AC_ARG_ENABLE` option in `configure.ac` — it is set by the DNN build glue, and is irrelevant to the core port.

#### `--enable-float-approx` → `FLOAT_APPROX` (default: **auto** — `yes` on i386/x86_64/arm*/aarch64*/ppc32/ppc64/ia64, else `no`)

`configure.ac:214-231`:

```m4
has_float_approx=no
case "$host_cpu" in
i[[3456]]86 | x86_64 | arm* | aarch64* | powerpc64 | powerpc32 | ia64)
  has_float_approx=yes
  ;;
esac

AC_ARG_ENABLE([float-approx],
    [AS_HELP_STRING([--enable-float-approx], [enable fast approximations for floating point])],
    [if test "$enable_float_approx" = "yes"; then
       AC_MSG_WARN([Floating point approximations are not supported on all platforms.])
     fi
    ],
    [enable_float_approx=$has_float_approx])

AS_IF([test "$enable_float_approx" = "yes"],[
  AC_DEFINE([FLOAT_APPROX], [1], [Float approximations])
])
```

Macro defined: **`FLOAT_APPROX 1`**. Files that read it: only `celt/arch.h` and `celt/mathops.h`.

Two independent effects:

1. `celt/arch.h:267-281` — `celt_isnan()`: IEEE-754 bit-pattern test under `FLOAT_APPROX` (safe with `-ffast-math`) vs `(x)!=(x)` otherwise; and under `!FLOAT_APPROX`, `#error Cannot build libopus with -ffast-math unless FLOAT_APPROX is defined.`
2. `celt/mathops.h` — selects the fast `celt_rsqrt`/`celt_sqrt`/`celt_log2`/`celt_exp2` approximate implementations that use `float` intermediates and (on some paths) `1/sqrtf`. It is **only** relevant to the float build. In `FIXED_POINT` builds the integer implementations in `celt/mathops.c` are used regardless, so `FLOAT_APPROX` is a no-op for the recommended target.

#### `--disable-rtcd` → removes `OPUS_HAVE_RTCD` (default: **rtcd enabled**)

`configure.ac:237-239, 871-888`:

```m4
AC_ARG_ENABLE([rtcd],
    [AS_HELP_STRING([--disable-rtcd], [disable run-time CPU capabilities detection])],,
    [enable_rtcd=yes])
```
```m4
AM_CONDITIONAL([HAVE_RTCD],
 [test x"$enable_rtcd" = x"yes" && test x"$rtcd_support" != x"no"])
AS_IF([test x"$enable_rtcd" = x"yes"],[
    AS_IF([test x"$rtcd_support" != x"no"],[
        AC_DEFINE([OPUS_HAVE_RTCD], [1],
            [Use run-time CPU capabilities detection])
        OPUS_HAVE_RTCD=1
        AC_SUBST(OPUS_HAVE_RTCD)
    ])
],[
    rtcd_support="disabled"
])
```

Macros: **removes** `OPUS_HAVE_RTCD`; as a consequence, none of the `OPUS_X86_PRESUME_SSE*` / `OPUS_X86_MAY_HAVE_SSE*` / `OPUS_ARM_MAY_HAVE_*` / `OPUS_ARM_PRESUME_*` macros are defined *for dispatch purposes* (they are defined by the intrinsics probe, which also consults `enable_rtcd` only to decide whether `rtcd_support` is reported — see `configure.ac:735-795`; the `_PRESUME_` macros can still be defined if the toolchain requires the feature unconditionally). `OPUS_HAVE_RTCD` is read in exactly these core-adjacent places: `celt/x86/*`, `celt/arm/*`, `silk/x86/*`, `silk/arm/*`, `dnn/*` — i.e. **only in SIMD dispatch shims**, plus `celt/mdct.h`, `celt/kiss_fft.h`, `celt/cpu_support.h`.

*Source-set changes* (`Makefile.am:56-59, 88-90`):

```make
if CPU_X86
if HAVE_RTCD
CELT_SOURCES += $(CELT_SOURCES_X86_RTCD)
SILK_SOURCES += $(SILK_SOURCES_X86_RTCD)
```
```make
if CPU_ARM
if HAVE_RTCD
CELT_SOURCES += $(CELT_SOURCES_ARM_RTCD)
SILK_SOURCES += $(SILK_SOURCES_ARM_RTCD)
```

So `--disable-rtcd` **removes** `celt/x86/x86cpu.c`, `celt/x86/x86_celt_map.c`, `silk/x86/x86_silk_map.c`, `celt/arm/armcpu.c`, `celt/arm/arm_celt_map.c`, `silk/arm/arm_silk_map.c` (and the DNN equivalents). **All of these are SIMD dispatch shims**; a portable port deletes them and calls the `_c` (pure C) function directly. `--disable-rtcd` therefore brings the build *closer* to the portable configuration but does **not** by itself disable the SSE/NEON intrinsics files — combine it with `--disable-intrinsics`.

#### `--disable-intrinsics` → removes `OPUS_*_MAY_HAVE_*` / `OPUS_*_PRESUME_*` (default: **intrinsics enabled**)

`configure.ac:241-243, 524-525`:

```m4
AC_ARG_ENABLE([intrinsics],
    [AS_HELP_STRING([--disable-intrinsics], [disable intrinsics optimizations])],,
    [enable_intrinsics=yes])
```
```m4
AS_IF([test x"$enable_intrinsics" = x"yes"],[
   intrinsics_support=""
```

Setting `--disable-intrinsics` skips the whole toolchain probe, so `OPUS_ARM_MAY_HAVE_EDSP|MEDIA|NEON|NEON_INTR|DOTPROD` and `OPUS_X86_MAY_HAVE_SSE|SSE2|SSE4_1|AVX2` are **not** defined, and the corresponding `HAVE_SSE*` / `HAVE_ARM_NEON_INTR` automake conditionals are false. It does **not** touch `OPUS_ARM_ASM` / `OPUS_ARM_INLINE_ASM` (those come from `--disable-asm`).

*Source-set changes* — all of these are skipped by `HAVE_SSE`, `HAVE_SSE2`, `HAVE_SSE4_1`, `HAVE_AVX2`, `HAVE_ARM_NEON_INTR`, `HAVE_ARM_NE10`:

| make variable | files removed |
|---|---|
| `CELT_SOURCES_SSE` | `celt/x86/pitch_sse.c` |
| `CELT_SOURCES_SSE2` | `celt/x86/pitch_sse2.c`, `celt/x86/vq_sse2.c` |
| `CELT_SOURCES_SSE4_1` | `celt/x86/celt_lpc_sse4_1.c`, `celt/x86/pitch_sse4_1.c` |
| `CELT_SOURCES_AVX2` | `celt/x86/pitch_avx.c` |
| `SILK_SOURCES_SSE4_1` | `silk/x86/NSQ_sse4_1.c`, `silk/x86/NSQ_del_dec_sse4_1.c`, `silk/x86/VAD_sse4_1.c`, `silk/x86/VQ_WMat_EC_sse4_1.c` |
| `SILK_SOURCES_AVX2` | `silk/x86/NSQ_del_dec_avx2.c` |
| `SILK_SOURCES_FIXED_SSE4_1` | `silk/fixed/x86/vector_ops_FIX_sse4_1.c`, `silk/fixed/x86/burg_modified_FIX_sse4_1.c` |
| `SILK_SOURCES_FLOAT_AVX2` | `silk/float/x86/inner_product_FLP_avx2.c` |
| `CELT_SOURCES_ARM_NEON_INTR` | `celt/arm/celt_neon_intr.c`, `celt/arm/pitch_neon_intr.c` |
| `SILK_SOURCES_ARM_NEON_INTR` | `silk/arm/biquad_alt_neon_intr.c`, `silk/arm/LPC_inv_pred_gain_neon_intr.c`, `silk/arm/NSQ_del_dec_neon_intr.c`, `silk/arm/NSQ_neon.c` |
| `SILK_SOURCES_FIXED_ARM_NEON_INTR` | `silk/fixed/arm/warped_autocorrelation_FIX_neon_intr.c` |
| `CELT_SOURCES_ARM_NE10` | `celt/arm/celt_fft_ne10.c`, `celt/arm/celt_mdct_ne10.c` (also removes the `static_modes_*_arm_ne10.h` includes) |

**This is the option that matters most for the port**: `--disable-intrinsics --disable-rtcd --disable-asm` yields exactly the portable C implementation set.

#### `--enable-assertions` → `ENABLE_ASSERTIONS` (default: **off**)

`configure.ac:885-891`:

```m4
AC_ARG_ENABLE([assertions],
    [AS_HELP_STRING([--enable-assertions],[enable additional software error checking])],,
    [enable_assertions=no])

AS_IF([test "$enable_assertions" = "yes"], [
  AC_DEFINE([ENABLE_ASSERTIONS], [1], [Assertions])
])
```

Macro defined: **`ENABLE_ASSERTIONS 1`**. No source files added/removed. It has **three** distinct effects:

1. `celt/arch.h:59-84` — enables `CELT_FATAL`, `celt_assert`, `celt_assert2`, `celt_sig_assert`, `MUST_SUCCEED` (the `celt_fatal` definition itself is compiled when `ENABLE_ASSERTIONS || ENABLE_HARDENING`).
2. `silk/typedef.h:60-77` — enables the aborting `silk_assert`; otherwise `silk_assert(COND)` is `((void)(COND))`.
3. **Behavioural** guards, notably the `FIXED_POINT_DEBUG`-correlated `celt_sig_assert(pulsesLeft<=N+3)` in `celt/vq.c`, and the `celt_assert` in `celt/entenc.c`/`entdec.c` (`ec_enc_shrink`, `ec_dec_bits`). These are asserts only; the release build's control flow must not depend on them *except* where an `#ifdef ENABLE_ASSERTIONS` block itself changes a variable (search `grep -rn 'ifdef ENABLE_ASSERTIONS' celt silk src` — e.g. `celt/arch.h`, `src/opus_decoder.c`, `src/opus_multistream_decoder.c`, `src/mapping_matrix.c`).

**Porting note:** `ENABLE_ASSERTIONS` must be *off* for the bit-exact reference configuration, but the port should have a debug feature that mirrors it.

#### `--disable-float-api` → `DISABLE_FLOAT_API` (default: **float API enabled**)

`configure.ac:139-148`:

```m4
AC_ARG_ENABLE([float_api],
    [AS_HELP_STRING([--disable-float-api],
                    [compile without the floating point API (for machines with no float library)])],,
    [enable_float_api=yes])

AM_CONDITIONAL([DISABLE_FLOAT_API], [test "$enable_float_api" = "no"])

AS_IF([test "$enable_float_api" = "no"],[
  AC_DEFINE([DISABLE_FLOAT_API], [1], [Do not build the float API])
])
```

Macro defined: **`DISABLE_FLOAT_API 1`**.

*Source-set changes* (`Makefile.am:50-52`):

```make
if DISABLE_FLOAT_API
else
OPUS_SOURCES += $(OPUS_SOURCES_FLOAT)
endif
```

Removes exactly three files: `src/analysis.c`, `src/mlp.c`, `src/mlp_data.c`. Note this is `if DISABLE_FLOAT_API / else`, i.e. the files are added when the conditional is **false**. Files that read `DISABLE_FLOAT_API` and gain `#error` or stub branches: `celt/float_cast.h`, `celt/mathops.c`, `celt/mathops.h`, `celt/celt_encoder.c`, `celt/celt_decoder.c`, `src/opus.c`, `src/opus_encoder.c`, `src/opus_decoder.c`, `src/mapping_matrix.c` and `src/mapping_matrix.h`, `src/opus_multistream_*.c`, `src/opus_projection_*.c`, `src/analysis.c`.

Sub-effect: with the float API disabled, `src/opus_encoder.c` cannot run the analysis/`opus_analysis_*`-driven CELT mode switching, so the encoder uses the fixed decision logic. That **changes the encoder bitstream**, so `DISABLE_FLOAT_API` is a bit-exactness-relevant switch, not merely an API-surface switch.

### 1.3 Other options that also change the macro surface

These are outside the requested ten, but they change compiled-in behaviour and therefore the port's surface. Record them for completeness.

| Option | Macro | Files / effect |
|---|---|---|
| `--disable-asm` (default on) | `OPUS_ARM_ASM`, `OPUS_ARM_INLINE_ASM`, `OPUS_ARM_INLINE_EDSP`, `OPUS_ARM_INLINE_MEDIA`, `OPUS_ARM_INLINE_NEON` | ARM inline asm; also adds `celt/arm/celt_pitch_xcorr_arm.s` and `celt/arm/armopts.s.in` (not portable C). |
| `--enable-custom-modes` | `CUSTOM_MODES` | Non-Opus modes (44.1 kHz, 2^n frames). Affects `celt/modes.c`, `celt/cwrs.c/h`, `celt/rate.c`, `celt/celt*.c`, `celt/kiss_fft.c`, `celt/mdct.c`, `include/opus_custom.h`; in `celt/celt.h:189-194` it makes `OPUS_CUSTOM_NOSTATIC` empty (exports the custom API). |
| `--enable-opus-custom-api` | `ENABLE_OPUS_CUSTOM_API` | Same `OPUS_CUSTOM_NOSTATIC` effect as `CUSTOM_MODES` without enabling 44.1 kHz modes. |
| `--enable-qext` | `ENABLE_QEXT` | libopus 1.6.x scalable-quality extension. Changes `celt/arch.h` (`celt_coef`, `COEF_ONE`, `MULT_COEF*`), `celt/celt.h`, `celt/modes.h`, `celt/bands.c`, `celt/modes.c`, `celt/rate.c` (`clt_compute_extra_allocation`), `celt/celt_encoder.c`, `celt/celt_decoder.c`, `celt/cwrs.c`, `celt/kiss_fft.c/h`, `celt/mdct.c`, `celt/vq.c`, `celt/mathops.h`, `celt/static_modes_*.h`, `silk/define.h`, `silk/check_control_input.c`, `silk/resampler.c`, and all `src/opus*.c`. **Autotools-only**; also gates `ARG_QEXT` and `QEXT_EXTENSION_ID 124`. |
| `--enable-hardening` (default on) | `ENABLE_HARDENING` | Enables `celt_fatal` and the `CELT_FATAL`-based checks even without `ENABLE_ASSERTIONS`. |
| `--enable-fuzzing` | `FUZZING` | Random encoder decisions; not for production. |
| `--enable-check-asm` | `OPUS_CHECK_ASM` | Compares SIMD against C at run time. |
| `--disable-dot-product` | `DISABLE_DOT_PROD` | **DNN-only** — no occurrence under `celt/`, `silk/`, `src/`. |
| `--enable-osce` | `ENABLE_OSCE`, `ENABLE_OSCE_BWE`, `ENABLE_OSCE_TRAINING_DATA` | Adds `dnn/osce.c`, `osce_features.c`, `nndsp.c`, `lace_data.c`, `nolace_data.c`, `bbwenet_data.c`; also implies `ENABLE_DEEP_PLC`. Core `#ifdef ENABLE_OSCE` sites: `silk/main.h`, `silk/dec_API.c`, `silk/init_decoder.c`, `silk/control.h`, `silk/decode_frame.c`, `silk/structs.h`, `src/opus_decoder.c`. |
| `--enable-dnn-debug-float` | `DISABLE_DEBUG_FLOAT` (defined when the option is **off**) | DNN-only. |
| `--enable-lossgen` | `ENABLE_LOSSGEN` | Adds `dnn/lossgen*.c` to `opus_demo` only. |

### 1.4 The `config.h` macro inventory relevant to the core port

From `config.h.in` (all `#undef`), the macros that a core-only port must decide about:

```
CUSTOM_MODES  DISABLE_FLOAT_API  ENABLE_ASSERTIONS  ENABLE_DEEP_PLC  ENABLE_DRED
ENABLE_HARDENING  ENABLE_OPUS_CUSTOM_API  ENABLE_OSCE  ENABLE_OSCE_BWE
ENABLE_OSCE_TRAINING_DATA  ENABLE_QEXT  ENABLE_RES24  FIXED_DEBUG  FIXED_POINT
FLOAT_APPROX  FUZZING  OPUS_ARM_ASM  OPUS_ARM_INLINE_ASM  OPUS_ARM_INLINE_EDSP
OPUS_ARM_INLINE_MEDIA  OPUS_ARM_INLINE_NEON  OPUS_ARM_MAY_HAVE_DOTPROD
OPUS_ARM_MAY_HAVE_EDSP  OPUS_ARM_MAY_HAVE_MEDIA  OPUS_ARM_MAY_HAVE_NEON
OPUS_ARM_MAY_HAVE_NEON_INTR  OPUS_ARM_PRESUME_AARCH64_NEON_INTR
OPUS_ARM_PRESUME_DOTPROD  OPUS_ARM_PRESUME_EDSP  OPUS_ARM_PRESUME_MEDIA
OPUS_ARM_PRESUME_NEON  OPUS_ARM_PRESUME_NEON_INTR  OPUS_BUILD  OPUS_CHECK_ASM
OPUS_HAVE_RTCD  OPUS_X86_MAY_HAVE_AVX2  OPUS_X86_MAY_HAVE_SSE
OPUS_X86_MAY_HAVE_SSE2  OPUS_X86_MAY_HAVE_SSE4_1  OPUS_X86_PRESUME_AVX2
OPUS_X86_PRESUME_SSE  OPUS_X86_PRESUME_SSE2  OPUS_X86_PRESUME_SSE4_1
USE_ALLOCA  VAR_ARRAYS
```

Also relevant (not in `config.h.in`, set directly by `Makefile.am`/`CMakeLists.txt`): `OPUS_BUILD`, `OPUS_DISABLE_SIMD` is **not** used anywhere in the tree (verified: zero occurrences), `USE_WEIGHTS_FILE` (DNN only).

Stack-allocation switches (`VAR_ARRAYS`, `USE_ALLOCA`, `NONTHREADSAFE_PSEUDOSTACK`) affect `celt/stack_alloc.h` / `silk/stack_alloc.h` only; they change *where* scratch buffers live, never their contents, so they are bit-exactness-neutral. The port should use Rust stack/`Vec` scratch with the same logical sizes.

---

## 2. File inventory

Every `.c` file under `celt/`, `silk/` and `src/` (recursively). Line counts are `wc -l` on the 1.6.1 tree. `Portable?` = `YES` for C code a portable Rust port must reproduce, `SIMD/ASM-ONLY` for intrinsics/assembly variants that must **not** be ported, `NOT-CORE` for demos/tests/tools. `Domain` = `CORE`, `FLOAT-API` (only compiled when the float API is enabled) or `DNN`. `Build variant` quotes the make variable that lists the file.

**Headline numbers**: 205 `.c` files, 60639 lines; of which portable C = 160 files / 45872 lines; SIMD-asm-only = 29 files; not-core = 16 files.

### 2.1 `celt/` — portable C (the 18 `CELT_SOURCES` + QEXT helper)

| File | Lines | Portable? | Domain | Build variant | Description | Project `#include`s |
|---|---:|---|---|---|---|---|
| `celt/bands.c` | 1922 | YES | CORE | ALWAYS (celt) | Band energy computation, normalisation/denormalisation, anti-collapse, spreading/hysteresis decisions, Hadamard transforms and `quant_all_bands` (the band-by-band PVQ/stereo quantiser driver). | `config.h`, `bands.h`, `modes.h`, `vq.h`, `cwrs.h`, `stack_alloc.h`, `os_support.h`, `mathops.h`, `rate.h`, `quant_bands.h`, `pitch.h` |
| `celt/celt.c` | 373 | YES | CORE | ALWAYS (celt) | Mode-independent helpers shared by encoder and decoder: `resampling_factor`, `comb_filter`, `init_caps`. | `config.h`, `os_support.h`, `mdct.h`, `celt.h`, `pitch.h`, `bands.h`, `modes.h`, `entcode.h`, `quant_bands.h`, `rate.h`, `stack_alloc.h`, `mathops.h`, `float_cast.h`, `celt_lpc.h`, `vq.h`, `mips/celt_mipsr1.h` |
| `celt/celt_decoder.c` | 1872 | YES | CORE | ALWAYS (celt) | The CELT decoder: state init/CTL, `deemphasis`, `celt_synthesis`, PLC, and the whole `celt_decode_with_ec_dred`/`celt_decode_with_ec` bitstream reader. | `config.h`, `cpu_support.h`, `os_support.h`, `mdct.h`, `celt.h`, `pitch.h`, `bands.h`, `modes.h`, `entcode.h`, `quant_bands.h`, `rate.h`, `stack_alloc.h`, `mathops.h`, `float_cast.h`, `celt_lpc.h`, `vq.h`, `lpcnet.h`, `lpcnet_private.h` |
| `celt/celt_encoder.c` | 3161 | YES | CORE | ALWAYS (celt) | The CELT encoder: state init/CTL, `celt_preemphasis`, prefilter, band energy, TF/dynalloc/VBR analysis, and the whole `celt_encode_with_ec` bitstream writer. | `config.h`, `cpu_support.h`, `os_support.h`, `mdct.h`, `celt.h`, `pitch.h`, `bands.h`, `modes.h`, `entcode.h`, `quant_bands.h`, `rate.h`, `stack_alloc.h`, `mathops.h`, `float_cast.h`, `celt_lpc.h`, `vq.h` |
| `celt/celt_lpc.c` | 374 | YES | CORE | ALWAYS (celt) | LPC helpers: Levinson-Durbin `_celt_lpc`, `celt_fir_c`, `celt_iir`, and `_celt_autocorr` (with the FFT-based autocorrelation path). | `config.h`, `celt_lpc.h`, `stack_alloc.h`, `mathops.h`, `pitch.h` |
| `celt/cwrs.c` | 719 | YES | CORE | ALWAYS (celt) | Combinatorial pulse coding: `encode_pulses`/`decode_pulses` (V(N,K) enumeration), `log2_frac`, `get_required_bits`, and the `CeltPulseCache` construction. | `config.h`, `os_support.h`, `cwrs.h`, `mathops.h`, `arch.h` |
| `celt/entcode.c` | 153 | YES | CORE | ALWAYS (celt) | Entropy-coder core: `ec_ilog`, `ec_tell`, `ec_tell_frac` — the bit-accounting shared by encoder and decoder. | `config.h`, `entcode.h`, `arch.h` |
| `celt/entdec.c` | 266 | YES | CORE | ALWAYS (celt) | Range decoder: `ec_dec_init/normalize`, `ec_decode*`, `ec_dec_bit_logp`, `ec_dec_icdf(16)`, `ec_dec_uint`, `ec_dec_bits`. | `config.h`, `os_support.h`, `arch.h`, `entdec.h`, `mfrngcod.h` |
| `celt/entenc.c` | 305 | YES | CORE | ALWAYS (celt) | Range encoder: `ec_enc_init/normalize/carry_out`, `ec_encode*`, `ec_enc_bit_logp`, `ec_enc_icdf(16)`, `ec_enc_uint`, `ec_enc_bits`, `ec_enc_patch_initial_bits`, `ec_enc_shrink`, `ec_enc_done`. | `config.h`, `os_support.h`, `arch.h`, `entenc.h`, `mfrngcod.h` |
| `celt/kiss_fft.c` | 650 | YES | CORE | ALWAYS (celt) | Mixed-radix FFT used by the MDCT and the pitch prefilter: `opus_fft_alloc*`, `kf_bfly2/3/4/5`, `kf_work`, `opus_fft_c`, `opus_ifft_c`. | `config.h`, `_kiss_fft_guts.h`, `arch.h`, `os_support.h`, `mathops.h`, `stack_alloc.h` |
| `celt/laplace.c` | 235 | YES | CORE | ALWAYS (celt) | Laplace-model entropy coding of coarse band energies and of the SILK-side Laplace symbols: `ec_laplace_encode/decode`, `ec_laplace_encode_p0/decode_p0`. | `config.h`, `laplace.h`, `mathops.h` |
| `celt/mathops.c` | 336 | YES | CORE | ALWAYS (celt) | Integer/float math primitives: `isqrt32`, `frac_div32(_q29)`, `celt_rsqrt_norm(32)`, `celt_sqrt(32)`, `celt_cos_norm(32)`, `celt_rcp(_norm*)`, plus `celt_float2int16_c`. | `config.h`, `float_cast.h`, `mathops.h` |
| `celt/mdct.c` | 390 | YES | CORE | ALWAYS (celt) | MDCT/IMDCT: `clt_mdct_init/clear`, `clt_mdct_forward_c`, `clt_mdct_backward_c` (the rotation/twiddle machinery around kiss_fft). | `config.h`, `mdct.h`, `kiss_fft.h`, `_kiss_fft_guts.h`, `os_support.h`, `mathops.h`, `stack_alloc.h`, `mips/mdct_mipsr1.h` |
| `celt/modes.c` | 520 | YES | CORE | ALWAYS (celt) | Mode construction: `opus_custom_mode_create` (eBands, allocation vectors, pulse cache, MDCT twiddles), `opus_custom_mode_destroy`, `compute_qext_mode`. | `config.h`, `celt.h`, `modes.h`, `rate.h`, `os_support.h`, `stack_alloc.h`, `quant_bands.h`, `cpu_support.h`, `static_modes_fixed.h`, `static_modes_float.h` |
| `celt/pitch.c` | 560 | YES | CORE | ALWAYS (celt) | Pitch analysis for the CELT encoder: `pitch_downsample`, `pitch_search`, `remove_doubling`, and the comb-filter/pitch-xcorr glue. | `config.h`, `pitch.h`, `os_support.h`, `modes.h`, `stack_alloc.h`, `mathops.h`, `celt_lpc.h` |
| `celt/quant_bands.c` | 572 | YES | CORE | ALWAYS (celt) | Band-energy (glog) quantisation: `quant_coarse_energy`, `quant_fine_energy`, `quant_energy_finalise`, the three `unquant_*` counterparts, and `amp2Log2`/`log2Amp`. | `config.h`, `quant_bands.h`, `laplace.h`, `os_support.h`, `arch.h`, `mathops.h`, `stack_alloc.h`, `rate.h` |
| `celt/rate.c` | 876 | YES | CORE | ALWAYS (celt) | Bit allocation: `compute_pulse_cache`, `clt_compute_allocation` (interp_bits2pulses family), `clt_compute_extra_allocation` (QEXT). | `config.h`, `modes.h`, `cwrs.h`, `arch.h`, `os_support.h`, `entcode.h`, `rate.h`, `quant_bands.h` |
| `celt/vq.c` | 867 | YES | CORE | ALWAYS (celt) | PVQ vector quantisation: `exp_rotation`, `op_pvq_search_c`, `alg_quant`, `alg_unquant`, `renormalise_vector`, `stereo_itheta`, plus the QEXT `cubic_quant`/`cubic_unquant`. | `config.h`, `mathops.h`, `cwrs.h`, `vq.h`, `arch.h`, `os_support.h`, `bands.h`, `rate.h`, `pitch.h`, `SigProc_FIX.h`, `macros.h` |

**Subtotal: 18 files, 14151 lines.**

### 2.2 `celt/` — SIMD / assembly only (**do not port**)

All of these are RTCD dispatch shims or SIMD kernels. Each has a pure-C fallback that a portable port implements instead; the `*_c`/`*_c`-suffixed function is the portable name.

| File | Lines | Portable? | Domain | Build variant | Description | Project `#include`s |
|---|---:|---|---|---|---|---|
| `celt/arm/arm_celt_map.c` | 193 | SIMD/ASM-ONLY | CORE | RTCD (ARM) | Intrinsics-only override (ARM NEON intrinsics); replaces a pure-C function via the RTCD map. NOT needed for a portable Rust port. | `config.h`, `kiss_fft.h`, `mathops.h`, `mdct.h`, `pitch.h` |
| `celt/arm/armcpu.c` | 323 | SIMD/ASM-ONLY | CORE | RTCD (ARM) | Intrinsics-only override (ARM NEON intrinsics); replaces a pure-C function via the RTCD map. NOT needed for a portable Rust port. | `config.h`, `armcpu.h`, `cpu_support.h`, `os_support.h`, `opus_types.h`, `arch.h` |
| `celt/arm/celt_fft_ne10.c` | 173 | SIMD/ASM-ONLY | CORE | NE10 | Intrinsics-only override (ARM NEON intrinsics); replaces a pure-C function via the RTCD map. NOT needed for a portable Rust port. | `config.h`, `os_support.h`, `kiss_fft.h`, `stack_alloc.h` |
| `celt/arm/celt_mdct_ne10.c` | 258 | SIMD/ASM-ONLY | CORE | NE10 | Intrinsics-only override (ARM NEON intrinsics); replaces a pure-C function via the RTCD map. NOT needed for a portable Rust port. | `config.h`, `kiss_fft.h`, `_kiss_fft_guts.h`, `mdct.h`, `stack_alloc.h` |
| `celt/arm/celt_neon_intr.c` | 401 | SIMD/ASM-ONLY | CORE | NEON | Intrinsics-only override (ARM NEON intrinsics); replaces a pure-C function via the RTCD map. NOT needed for a portable Rust port. | `config.h`, `../float_cast.h`, `../mathops.h`, `../pitch.h` |
| `celt/arm/pitch_neon_intr.c` | 288 | SIMD/ASM-ONLY | CORE | NEON | Intrinsics-only override (ARM NEON intrinsics); replaces a pure-C function via the RTCD map. NOT needed for a portable Rust port. | `config.h`, `pitch.h` |
| `celt/x86/celt_lpc_sse4_1.c` | 96 | SIMD/ASM-ONLY | CORE | SSE4.1 | Intrinsics-only override (x86 intrinsics); replaces a pure-C function via the RTCD map. NOT needed for a portable Rust port. | `config.h`, `celt_lpc.h`, `stack_alloc.h`, `mathops.h`, `pitch.h`, `x86cpu.h` |
| `celt/x86/pitch_avx.c` | 101 | SIMD/ASM-ONLY | CORE | AVX2 | Intrinsics-only override (x86 intrinsics); replaces a pure-C function via the RTCD map. NOT needed for a portable Rust port. | `config.h`, `x86cpu.h`, `pitch.h` |
| `celt/x86/pitch_sse.c` | 185 | SIMD/ASM-ONLY | CORE | SSE | Intrinsics-only override (x86 intrinsics); replaces a pure-C function via the RTCD map. NOT needed for a portable Rust port. | `config.h`, `macros.h`, `celt_lpc.h`, `stack_alloc.h`, `mathops.h`, `pitch.h`, `arch.h` |
| `celt/x86/pitch_sse2.c` | 95 | SIMD/ASM-ONLY | CORE | SSE2 | Intrinsics-only override (x86 intrinsics); replaces a pure-C function via the RTCD map. NOT needed for a portable Rust port. | `config.h`, `macros.h`, `celt_lpc.h`, `stack_alloc.h`, `mathops.h`, `pitch.h` |
| `celt/x86/pitch_sse4_1.c` | 240 | SIMD/ASM-ONLY | CORE | SSE4.1 | Intrinsics-only override (x86 intrinsics); replaces a pure-C function via the RTCD map. NOT needed for a portable Rust port. | `config.h`, `macros.h`, `celt_lpc.h`, `stack_alloc.h`, `mathops.h`, `pitch.h`, `x86cpu.h` |
| `celt/x86/vq_sse2.c` | 217 | SIMD/ASM-ONLY | CORE | SSE2 | Intrinsics-only override (x86 intrinsics); replaces a pure-C function via the RTCD map. NOT needed for a portable Rust port. | `config.h`, `celt_lpc.h`, `stack_alloc.h`, `mathops.h`, `vq.h`, `x86cpu.h` |
| `celt/x86/x86_celt_map.c` | 187 | SIMD/ASM-ONLY | CORE | RTCD (x86) | Intrinsics-only override (x86 intrinsics); replaces a pure-C function via the RTCD map. NOT needed for a portable Rust port. | `config.h`, `x86/x86cpu.h`, `celt_lpc.h`, `pitch.h`, `pitch_sse.h`, `vq.h` |
| `celt/x86/x86cpu.c` | 184 | SIMD/ASM-ONLY | CORE | RTCD (x86) | Intrinsics-only override (x86 intrinsics); replaces a pure-C function via the RTCD map. NOT needed for a portable Rust port. | `config.h`, `cpu_support.h`, `macros.h`, `main.h`, `pitch.h`, `x86cpu.h` |

**Subtotal: 14 files, 2941 lines.**

### 2.3 `celt/` — demos and tests (not part of the library)

| File | Lines | Portable? | Domain | Build variant | Description | Project `#include`s |
|---|---:|---|---|---|---|---|
| `celt/mini_kfft.c` | 494 | NOT-CORE | CORE | NOT-CORE | A small, standalone kiss-FFT variant vendored for the QEXT comparison tool (`qext_compare.c`); not part of the library. | _(none)_ |
| `celt/opus_custom_demo.c` | 512 | NOT-CORE | CORE | NOT-CORE | Demo program for the Opus custom API; not part of the library. | `config.h`, `opus_custom.h`, `arch.h`, `modes.h` |
| `celt/tests/test_unit_cwrs32.c` | 162 | NOT-CORE | CORE | NOT-CORE | Stand-alone test/demo program (not part of the library). Useful only as an oracle. | `config.h`, `stack_alloc.h`, `entenc.c`, `entdec.c`, `entcode.c`, `cwrs.c`, `mathops.c`, `rate.h` |
| `celt/tests/test_unit_dft.c` | 181 | NOT-CORE | CORE | NOT-CORE | Stand-alone test/demo program (not part of the library). Useful only as an oracle. | `config.h`, `stack_alloc.h`, `kiss_fft.h`, `mathops.h`, `modes.h` |
| `celt/tests/test_unit_entropy.c` | 383 | NOT-CORE | CORE | NOT-CORE | Stand-alone test/demo program (not part of the library). Useful only as an oracle. | `config.h`, `entcode.h`, `entenc.h`, `entdec.h`, `entenc.c`, `entdec.c`, `entcode.c` |
| `celt/tests/test_unit_laplace.c` | 94 | NOT-CORE | CORE | NOT-CORE | Stand-alone test/demo program (not part of the library). Useful only as an oracle. | `config.h`, `laplace.h`, `stack_alloc.h`, `entenc.c`, `entdec.c`, `entcode.c`, `laplace.c` |
| `celt/tests/test_unit_mathops.c` | 816 | NOT-CORE | CORE | NOT-CORE | Stand-alone test/demo program (not part of the library). Useful only as an oracle. | `config.h`, `bands.h`, `cpu_support.h`, `float_cast.h`, `mathops.h` |
| `celt/tests/test_unit_mdct.c` | 233 | NOT-CORE | CORE | NOT-CORE | Stand-alone test/demo program (not part of the library). Useful only as an oracle. | `config.h`, `mdct.h`, `stack_alloc.h`, `kiss_fft.h`, `mdct.h`, `modes.h` |
| `celt/tests/test_unit_mini_kfft.c` | 161 | NOT-CORE | CORE | NOT-CORE | Stand-alone test/demo program (not part of the library). Useful only as an oracle. | `config.h`, `mini_kfft.c` |
| `celt/tests/test_unit_rotation.c` | 87 | NOT-CORE | CORE | NOT-CORE | Stand-alone test/demo program (not part of the library). Useful only as an oracle. | `config.h`, `vq.h`, `bands.h`, `stack_alloc.h` |
| `celt/tests/test_unit_types.c` | 50 | NOT-CORE | CORE | NOT-CORE | Stand-alone test/demo program (not part of the library). Useful only as an oracle. | `config.h`, `opus_types.h` |

**Subtotal: 11 files, 3173 lines.**

### 2.4 `silk/` — portable C (top level, `SILK_SOURCES`)

| File | Lines | Portable? | Domain | Build variant | Description | Project `#include`s |
|---|---:|---|---|---|---|---|
| `silk/A2NLSF.c` | 267 | YES | CORE | ALWAYS (silk) | `silk_A2NLSF`: converts LPC coefficients to normalised LSFs by Chebyshev polynomial root search and bisection. | `config.h`, `SigProc_FIX.h`, `tables.h` |
| `silk/CNG.c` | 188 | YES | CORE | ALWAYS (silk) | Comfort-noise generation: `silk_CNG_Reset` and `silk_CNG` (decoder-side noise injection and LPC/LTP smoothing). | `config.h`, `main.h`, `stack_alloc.h` |
| `silk/HP_variable_cutoff.c` | 77 | YES | CORE | ALWAYS (silk) | `silk_HP_variable_cutoff`: variable high-pass filter applied at the encoder input for bandwidth changes. | `config.h`, `main_FIX.h`, `main_FLP.h`, `tuning_parameters.h` |
| `silk/LPC_analysis_filter.c` | 111 | YES | CORE | ALWAYS (silk) | `silk_LPC_analysis_filter`: LPC residual computation, fixed-point with output scaling. | `config.h`, `SigProc_FIX.h`, `celt_lpc.h` |
| `silk/LPC_fit.c` | 82 | YES | CORE | ALWAYS (silk) | `silk_LPC_fit`: rescales/rounds LPC coefficients to fit a target Q domain (used by the fixed-point encoder). | `config.h`, `SigProc_FIX.h` |
| `silk/LPC_inv_pred_gain.c` | 141 | YES | CORE | ALWAYS (silk) | `silk_LPC_inverse_pred_gain(_c)`: inverse prediction gain via the Schur/reflection-coefficient stability recursion. | `config.h`, `SigProc_FIX.h`, `define.h` |
| `silk/LP_variable_cutoff.c` | 135 | YES | CORE | ALWAYS (silk) | `silk_LP_variable_cutoff`: encoder-side variable low-pass filter for the LTP/voicing path. | `config.h`, `main.h` |
| `silk/NLSF2A.c` | 140 | YES | CORE | ALWAYS (silk) | `silk_NLSF2A`: converts normalised LSFs to LPC coefficients (cosine table + polynomial expansion, with stabilisation fallback). | `config.h`, `SigProc_FIX.h`, `tables.h` |
| `silk/NLSF_VQ.c` | 76 | YES | CORE | ALWAYS (silk) | `silk_NLSF_VQ`: weighted squared-error search over the NLSF codebook. | `config.h`, `main.h` |
| `silk/NLSF_VQ_weights_laroia.c` | 80 | YES | CORE | ALWAYS (silk) | `silk_NLSF_VQ_weights_laroia`: Laroia weights for NLSF VQ distortion. | `config.h`, `define.h`, `SigProc_FIX.h` |
| `silk/NLSF_decode.c` | 93 | YES | CORE | ALWAYS (silk) | `silk_NLSF_decode`: decodes and stabilises an NLSF vector from its indices. | `config.h`, `main.h` |
| `silk/NLSF_del_dec_quant.c` | 215 | YES | CORE | ALWAYS (silk) | `silk_NLSF_del_dec_quant`: delayed-decision (Viterbi) NLSF quantiser in the encoder. | `config.h`, `main.h` |
| `silk/NLSF_encode.c` | 124 | YES | CORE | ALWAYS (silk) | `silk_NLSF_encode`: NLSF encoder front end (weighting, VQ, delayed decision, stabilisation). | `config.h`, `main.h`, `stack_alloc.h` |
| `silk/NLSF_stabilize.c` | 142 | YES | CORE | ALWAYS (silk) | `silk_NLSF_stabilize`: enforces minimum spacing of NLSF values, with the delta-min table. | `config.h`, `SigProc_FIX.h` |
| `silk/NLSF_unpack.c` | 54 | YES | CORE | ALWAYS (silk) | `silk_NLSF_unpack`: unpacks codebook predictor/selection indices into the per-coefficient structures. | `config.h`, `main.h` |
| `silk/NSQ.c` | 437 | YES | CORE | ALWAYS (silk) | `silk_noise_shape_quantizer` and the two-stage NSQ driver: the SILK encoder's noise-shaping quantiser producing pulses and quantised signal. | `config.h`, `main.h`, `stack_alloc.h`, `NSQ.h` |
| `silk/NSQ_del_dec.c` | 730 | YES | CORE | ALWAYS (silk) | `silk_NSQ_del_dec_c`: delayed-decision variant of the noise-shaping quantiser (default encoder path). | `config.h`, `main.h`, `stack_alloc.h`, `NSQ.h` |
| `silk/PLC.c` | 493 | YES | CORE | ALWAYS (silk) | Packet-loss concealment: `silk_PLC_Reset`, `silk_PLC`, `silk_PLC_glue_frames` (LPC/LTP extrapolation and energy matching), with an optional `ENABLE_DEEP_PLC` LPCNet branch. | `config.h`, `main.h`, `stack_alloc.h`, `PLC.h`, `lpcnet.h` |
| `silk/VAD.c` | 360 | YES | CORE | ALWAYS (silk) | Voice activity detection: `silk_VAD_Init`, `silk_VAD_GetSA_Q8(_c)` and noise-level tracking. | `config.h`, `main.h`, `stack_alloc.h` |
| `silk/VQ_WMat_EC.c` | 131 | YES | CORE | ALWAYS (silk) | `silk_VQ_WMat_EC_c`: weighted LTP-tap VQ search with a rate-distortion cost. | `config.h`, `main.h` |
| `silk/ana_filt_bank_1.c` | 74 | YES | CORE | ALWAYS (silk) | `silk_ana_filt_bank_1`: two-band analysis filter bank (used for 2x downsampling in the resampler and stereo). | `config.h`, `SigProc_FIX.h` |
| `silk/biquad_alt.c` | 121 | YES | CORE | ALWAYS (silk) | `silk_biquad_alt_stride1/2`: second-order ARMA biquad with slowly varying Q28 coefficients. | `config.h`, `SigProc_FIX.h` |
| `silk/bwexpander.c` | 51 | YES | CORE | ALWAYS (silk) | `silk_bwexpander`: bandwidth expansion of Q12 LPC coefficients by a Q16 chirp factor. | `config.h`, `SigProc_FIX.h` |
| `silk/bwexpander_32.c` | 50 | YES | CORE | ALWAYS (silk) | `silk_bwexpander_32`: bandwidth expansion of Q16/Q17 LPC coefficients. | `config.h`, `SigProc_FIX.h` |
| `silk/check_control_input.c` | 109 | YES | CORE | ALWAYS (silk) | `check_control_input`: validates a `silk_EncControlStruct` and quantises the requested bitrate/frame size. | `config.h`, `main.h`, `control.h`, `errors.h` |
| `silk/code_signs.c` | 115 | YES | CORE | ALWAYS (silk) | `silk_encode_signs`/`silk_decode_signs`: entropy coding of pulse signs for the NSQ output. | `config.h`, `main.h` |
| `silk/control_SNR.c` | 113 | YES | CORE | ALWAYS (silk) | `silk_control_SNR`: maps bitrate/complexity to the SNR targets used by the gain/NSQ loops. | `config.h`, `main.h`, `tuning_parameters.h` |
| `silk/control_audio_bandwidth.c` | 132 | YES | CORE | ALWAYS (silk) | `silk_control_audio_bandwidth`: chooses NB/MB/WB internal bandwidth and triggers resampler changes. | `config.h`, `main.h`, `tuning_parameters.h` |
| `silk/control_codec.c` | 423 | YES | CORE | ALWAYS (silk) | `silk_control_encoder`: applies encoder CTLs to the per-channel state (bitrate, complexity, LPC order, features). | `config.h`, `main_FIX.h`, `main_FLP.h`, `stack_alloc.h`, `tuning_parameters.h`, `pitch_est_defines.h` |
| `silk/debug.c` | 173 | YES | CORE | ALWAYS (silk) | Debug/timer helpers (`silk_TimerSave`, WMOPS reporting); compiled to nothing unless SILK debug macros are set. | `config.h`, `debug.h`, `SigProc_FIX.h` |
| `silk/dec_API.c` | 509 | YES | CORE | ALWAYS (silk) | SILK decoder public entry points: `silk_Get_Decoder_Size`, `silk_InitDecoder`, `silk_ResetDecoder`, `silk_Decode`, `silk_get_TOC`, and the `silk_decoder` super-struct. | `config.h`, `API.h`, `main.h`, `stack_alloc.h`, `os_support.h`, `osce.h`, `osce_structs.h`, `osce_features.h` |
| `silk/decode_core.c` | 243 | YES | CORE | ALWAYS (silk) | `silk_decode_core`: the SILK decoder's per-frame synthesis (LTP/LPC excitation reconstruction and resampling). | `config.h`, `main.h`, `stack_alloc.h` |
| `silk/decode_frame.c` | 169 | YES | CORE | ALWAYS (silk) | `silk_decode_frame`: one SILK frame — decode indices, parameters, then call `silk_decode_core`; handles PLC/CNG fallback. | `config.h`, `main.h`, `stack_alloc.h`, `PLC.h`, `osce.h` |
| `silk/decode_indices.c` | 151 | YES | CORE | ALWAYS (silk) | `silk_decode_indices`: reads the SILK frame's entropy-coded side information (gains, LSF/NLSF, pitch, LTP, seed). | `config.h`, `main.h` |
| `silk/decode_parameters.c` | 115 | YES | CORE | ALWAYS (silk) | `silk_decode_parameters`: turns decoded indices into LPC/LTP/gain parameters (dequantisation and interpolation). | `config.h`, `main.h` |
| `silk/decode_pitch.c` | 77 | YES | CORE | ALWAYS (silk) | `silk_decode_pitch`: maps the coded pitch index/lag to the per-subframe pitch lags. | `config.h`, `SigProc_FIX.h`, `pitch_est_defines.h` |
| `silk/decode_pulses.c` | 115 | YES | CORE | ALWAYS (silk) | `silk_decode_pulses`: decodes the NSQ pulse excitation via the shell coder and the pulse-count/RateLevel tables. | `config.h`, `main.h` |
| `silk/decoder_set_fs.c` | 107 | YES | CORE | ALWAYS (silk) | `silk_decoder_set_fs`: switches the decoder's internal sampling rate/bandwidth and resets dependent state. | `config.h`, `main.h` |
| `silk/enc_API.c` | 601 | YES | CORE | ALWAYS (silk) | SILK encoder public entry points: `silk_Get_Encoder_Size`, `silk_InitEncoder`, `silk_Encode`, `silk_QueryEncoder`, plus the `silk_encoder` super-struct and VAD/LBRR/DTX logic. | `config.h`, `define.h`, `API.h`, `control.h`, `typedef.h`, `stack_alloc.h`, `structs.h`, `tuning_parameters.h`, `main_FIX.h`, `main_FLP.h`, `dred_encoder.h` |
| `silk/encode_indices.c` | 181 | YES | CORE | ALWAYS (silk) | `silk_encode_indices`: writes the SILK frame's entropy-coded side information. | `config.h`, `main.h` |
| `silk/encode_pulses.c` | 206 | YES | CORE | ALWAYS (silk) | `silk_encode_pulses`: encodes the NSQ pulse excitation (RateLevel tree, split into 8- or 16-sample blocks, shell coder). | `config.h`, `main.h`, `stack_alloc.h` |
| `silk/gain_quant.c` | 142 | YES | CORE | ALWAYS (silk) | `silk_gains_quant`/`silk_gains_dequant`/`silk_gains_ID`: log-domain gain quantisation with inter-frame prediction. | `config.h`, `main.h` |
| `silk/init_decoder.c` | 83 | YES | CORE | ALWAYS (silk) | `silk_init_decoder`/`silk_reset_decoder`: zero/initialise a `silk_decoder_state` (and OSCE state when enabled). | `config.h`, `main.h`, `osce.h`, `structs.h` |
| `silk/init_encoder.c` | 68 | YES | CORE | ALWAYS (silk) | `silk_init_encoder`: zero/initialise a `silk_encoder_state_FIX\|FLP` channel state. | `config.h`, `main_FIX.h`, `main_FLP.h`, `tuning_parameters.h`, `cpu_support.h`, `dred_encoder.h` |
| `silk/inner_prod_aligned.c` | 47 | YES | CORE | ALWAYS (silk) | `silk_inner_prod_aligned_scale`: scaled inner product of 16-bit vectors. | `config.h`, `SigProc_FIX.h` |
| `silk/interpolate.c` | 51 | YES | CORE | ALWAYS (silk) | `silk_interpolate`: linear interpolation of two Q15 vectors (used for LSF and LTP interpolation). | `config.h`, `main.h` |
| `silk/lin2log.c` | 45 | YES | CORE | ALWAYS (silk) | `silk_lin2log`: Q7 linear to Q7 log2 approximation (piecewise linear + table). | `config.h`, `SigProc_FIX.h` |
| `silk/log2lin.c` | 58 | YES | CORE | ALWAYS (silk) | `silk_log2lin`: Q7 log2 to Q16 linear approximation, the inverse of `silk_lin2log`. | `config.h`, `SigProc_FIX.h` |
| `silk/pitch_est_tables.c` | 99 | YES | CORE | ALWAYS (silk) | Pitch-estimator codebook tables: `silk_CB_lags_stage2/3(_10_ms)`, `silk_Lag_range_stage3`, `silk_nb_cbk_searchs_stage3`. | `config.h`, `typedef.h`, `pitch_est_defines.h` |
| `silk/process_NLSFs.c` | 107 | YES | CORE | ALWAYS (silk) | `silk_process_NLSFs`: runs NLSF encode/decode round-trip, LPC conversion and stability checks each frame. | `config.h`, `main.h` |
| `silk/quant_LTP_gains.c` | 132 | YES | CORE | ALWAYS (silk) | `silk_quant_LTP_gains`: joint VQ of the LTP filter taps per subframe with a rate-distortion criterion. | `config.h`, `main.h`, `tuning_parameters.h` |
| `silk/resampler.c` | 224 | YES | CORE | ALWAYS (silk) | Resampler front end: `silk_resampler_init` and `silk_resampler`, dispatching to the up/down/2:3/FIR/IIR primitives. | `config.h`, `resampler_private.h` |
| `silk/resampler_down2.c` | 73 | YES | CORE | ALWAYS (silk) | `silk_resampler_down2`: 2x decimation with an all-pass/all-pole pair. | `config.h`, `SigProc_FIX.h`, `resampler_rom.h` |
| `silk/resampler_down2_3.c` | 103 | YES | CORE | ALWAYS (silk) | `silk_resampler_down2_3`: 2/3-rate decimation using the AR2 + FIR cascade. | `config.h`, `SigProc_FIX.h`, `resampler_private.h`, `stack_alloc.h` |
| `silk/resampler_private_AR2.c` | 54 | YES | CORE | ALWAYS (silk) | `silk_resampler_private_AR2`: second-order ARMA pre-filter used by the down-samplers. | `config.h`, `SigProc_FIX.h`, `resampler_private.h` |
| `silk/resampler_private_IIR_FIR.c` | 107 | YES | CORE | ALWAYS (silk) | `silk_resampler_private_IIR_FIR`: IIR up-sampling stage followed by FIR interpolation. | `config.h`, `SigProc_FIX.h`, `resampler_private.h`, `stack_alloc.h` |
| `silk/resampler_private_down_FIR.c` | 194 | YES | CORE | ALWAYS (silk) | `silk_resampler_private_down_FIR`: interpolate-then-FIR decimation stage. | `config.h`, `SigProc_FIX.h`, `resampler_private.h`, `stack_alloc.h` |
| `silk/resampler_private_up2_HQ.c` | 113 | YES | CORE | ALWAYS (silk) | `silk_resampler_private_up2_HQ(_wrapper)`: 2x up-sampling with two all-pass branches. | `config.h`, `SigProc_FIX.h`, `resampler_private.h` |
| `silk/resampler_rom.c` | 96 | YES | CORE | ALWAYS (silk) | Resampler coefficient ROM: `silk_resampler_frac_FIR_12`, up2 HQ coefficients, and the rate/bandwidth dispatch table. | `config.h`, `resampler_private.h` |
| `silk/shell_coder.c` | 151 | YES | CORE | ALWAYS (silk) | `silk_shell_encoder`/`silk_shell_decoder`: the shell (hierarchical) pulse coding used inside `encode_pulses`/`decode_pulses`. | `config.h`, `main.h` |
| `silk/sigm_Q15.c` | 75 | YES | CORE | ALWAYS (silk) | `silk_sigm_Q15`: Q15 sigmoid approximation by piecewise segments (used for pitch/voicing decisions). | `config.h`, `SigProc_FIX.h` |
| `silk/sort.c` | 154 | YES | CORE | ALWAYS (silk) | Insertion sort helpers (`silk_insertion_sort_increasing*`, `..._decreasing_int16`) used throughout parameter search. | `config.h`, `SigProc_FIX.h` |
| `silk/stereo_LR_to_MS.c` | 229 | YES | CORE | ALWAYS (silk) | `silk_stereo_LR_to_MS`: encoder-side L/R to M/S conversion, including the two-stage prediction and delay alignment. | `config.h`, `main.h`, `stack_alloc.h` |
| `silk/stereo_MS_to_LR.c` | 85 | YES | CORE | ALWAYS (silk) | `silk_stereo_MS_to_LR`: decoder-side M/S to L/R reconstruction. | `config.h`, `main.h` |
| `silk/stereo_decode_pred.c` | 73 | YES | CORE | ALWAYS (silk) | `silk_stereo_decode_pred`/`silk_stereo_decode_mid_only`: stereo predictor and mid-only flag decoding. | `config.h`, `main.h` |
| `silk/stereo_encode_pred.c` | 62 | YES | CORE | ALWAYS (silk) | `silk_stereo_encode_pred`/`silk_stereo_encode_mid_only`: stereo predictor and mid-only flag coding. | `config.h`, `main.h` |
| `silk/stereo_find_predictor.c` | 79 | YES | CORE | ALWAYS (silk) | `silk_stereo_find_predictor`: computes the mid/side predictor from cross-correlations. | `config.h`, `main.h` |
| `silk/stereo_quant_pred.c` | 73 | YES | CORE | ALWAYS (silk) | `silk_stereo_quant_pred`: quantises the stereo predictor and mid-only flag to their entropy-coded indices. | `config.h`, `main.h` |
| `silk/sum_sqr_shift.c` | 82 | YES | CORE | ALWAYS (silk) | `silk_sum_sqr_shift`: energy computation with a variable right shift to avoid overflow. | `config.h`, `SigProc_FIX.h` |
| `silk/table_LSF_cos.c` | 70 | YES | CORE | ALWAYS (silk) | The `silk_LSFCosTab_FIX_Q12` cosine table used by `silk_NLSF2A`/`silk_A2NLSF`. | `config.h`, `tables.h` |
| `silk/tables_LTP.c` | 294 | YES | CORE | ALWAYS (silk) | LTP codebook tables: `silk_LTP_vq_ptrs_Q7`, `silk_LTP_gain_vq_*`, `silk_LTP_per_index_iCDF`, `silk_LTP_gain_iCDF_ptrs`. | `config.h`, `tables.h` |
| `silk/tables_NLSF_CB_NB_MB.c` | 195 | YES | CORE | ALWAYS (silk) | Narrowband/mediumband NLSF codebook: `silk_NLSF_CB_NB_MB`, its iCDFs, predictors, weights and delta-min table. | `config.h`, `tables.h` |
| `silk/tables_NLSF_CB_WB.c` | 233 | YES | CORE | ALWAYS (silk) | Wideband NLSF codebook: `silk_NLSF_CB_WB`, its iCDFs, predictors, weights and delta-min table. | `config.h`, `tables.h` |
| `silk/tables_gain.c` | 55 | YES | CORE | ALWAYS (silk) | Gain-quantisation tables: `silk_gains_dequant_Q16`-related CDFs and `silk_gain_quant_*`. | `config.h`, `tables.h` |
| `silk/tables_other.c` | 115 | YES | CORE | ALWAYS (silk) | Miscellaneous tables: pitch-contour and pitch-lag iCDFs, LTP scaling, quantisation offset, PLC window, LBRR flags. | `config.h`, `structs.h`, `define.h`, `tables.h` |
| `silk/tables_pitch_lag.c` | 67 | YES | CORE | ALWAYS (silk) | Pitch-lag coding tables: `silk_pitch_lag_iCDF`, `silk_pitch_delta_iCDF`, `silk_pitch_contour_*`, `silk_pitch_lag_*`. | `config.h`, `tables.h` |
| `silk/tables_pulses_per_block.c` | 264 | YES | CORE | ALWAYS (silk) | Pulse-count coding tables: `silk_max_pulses_table`, `silk_pulses_per_block_iCDF*`, `silk_rate_levels_iCDF*`, `silk_LSB_iCDF`. | `config.h`, `tables.h` |

**Subtotal: 77 files, 12063 lines.**

### 2.5 `silk/fixed/` — fixed-point encoder analysis (`SILK_SOURCES_FIXED`)

Compiled **only** when `FIXED_POINT` is defined. These files implement the SILK encoder's analysis path in integer Q-domain arithmetic and are exactly what a bit-exact fixed-point port needs. `silk/fixed/structs_FIX.h` (`silk_encoder_state_FIX`, `silk_shape_state_FIX`, `silk_encoder_control_FIX`, and the outer `silk_encoder`) is the header counterpart.

| File | Lines | Portable? | Domain | Build variant | Description | Project `#include`s |
|---|---:|---|---|---|---|---|
| `silk/fixed/LTP_analysis_filter_FIX.c` | 89 | YES | CORE | FIXED-ONLY | Fixed-point Q-domain implementation of `silk_LTP_analysis_filter` for the SILK encoder analysis path (FIXED_POINT only). | `config.h`, `main_FIX.h` |
| `silk/fixed/LTP_scale_ctrl_FIX.c` | 58 | YES | CORE | FIXED-ONLY | Fixed-point Q-domain implementation of `silk_LTP_scale_ctrl` for the SILK encoder analysis path (FIXED_POINT only). | `config.h`, `main_FIX.h` |
| `silk/fixed/apply_sine_window_FIX.c` | 101 | YES | CORE | FIXED-ONLY | Fixed-point Q-domain implementation of `silk_apply_sine_window` for the SILK encoder analysis path (FIXED_POINT only). | `config.h`, `SigProc_FIX.h` |
| `silk/fixed/autocorr_FIX.c` | 48 | YES | CORE | FIXED-ONLY | Fixed-point Q-domain implementation of `silk_autocorr` for the SILK encoder analysis path (FIXED_POINT only). | `config.h`, `SigProc_FIX.h`, `celt_lpc.h` |
| `silk/fixed/burg_modified_FIX.c` | 280 | YES | CORE | FIXED-ONLY | Fixed-point Q-domain implementation of `silk_burg_modified` for the SILK encoder analysis path (FIXED_POINT only). | `config.h`, `SigProc_FIX.h`, `define.h`, `tuning_parameters.h`, `pitch.h` |
| `silk/fixed/corrMatrix_FIX.c` | 149 | YES | CORE | FIXED-ONLY | Fixed-point Q-domain implementation of `silk_corrMatrix` for the SILK encoder analysis path (FIXED_POINT only). | `config.h`, `main_FIX.h` |
| `silk/fixed/encode_frame_FIX.c` | 454 | YES | CORE | FIXED-ONLY | Fixed-point Q-domain implementation of `silk_encode_frame` for the SILK encoder analysis path (FIXED_POINT only). | `config.h`, `main_FIX.h`, `stack_alloc.h`, `tuning_parameters.h` |
| `silk/fixed/find_LPC_FIX.c` | 151 | YES | CORE | FIXED-ONLY | Fixed-point Q-domain implementation of `silk_find_LPC` for the SILK encoder analysis path (FIXED_POINT only). | `config.h`, `main_FIX.h`, `stack_alloc.h`, `tuning_parameters.h` |
| `silk/fixed/find_LTP_FIX.c` | 99 | YES | CORE | FIXED-ONLY | Fixed-point Q-domain implementation of `silk_find_LTP` for the SILK encoder analysis path (FIXED_POINT only). | `config.h`, `main_FIX.h`, `tuning_parameters.h` |
| `silk/fixed/find_pitch_lags_FIX.c` | 143 | YES | CORE | FIXED-ONLY | Fixed-point Q-domain implementation of `silk_find_pitch_lags` for the SILK encoder analysis path (FIXED_POINT only). | `config.h`, `main_FIX.h`, `stack_alloc.h`, `tuning_parameters.h` |
| `silk/fixed/find_pred_coefs_FIX.c` | 147 | YES | CORE | FIXED-ONLY | Fixed-point Q-domain implementation of `silk_find_pred_coefs` for the SILK encoder analysis path (FIXED_POINT only). | `config.h`, `main_FIX.h`, `stack_alloc.h` |
| `silk/fixed/k2a_FIX.c` | 54 | YES | CORE | FIXED-ONLY | Fixed-point Q-domain implementation of `silk_k2a` for the SILK encoder analysis path (FIXED_POINT only). | `config.h`, `SigProc_FIX.h` |
| `silk/fixed/k2a_Q16_FIX.c` | 54 | YES | CORE | FIXED-ONLY | Fixed-point Q-domain implementation of `silk_k2a_Q16` for the SILK encoder analysis path (FIXED_POINT only). | `config.h`, `SigProc_FIX.h` |
| `silk/fixed/noise_shape_analysis_FIX.c` | 400 | YES | CORE | FIXED-ONLY | Fixed-point Q-domain implementation of `silk_noise_shape_analysis` for the SILK encoder analysis path (FIXED_POINT only). | `config.h`, `main_FIX.h`, `stack_alloc.h`, `tuning_parameters.h` |
| `silk/fixed/pitch_analysis_core_FIX.c` | 721 | YES | CORE | FIXED-ONLY | Fixed-point Q-domain implementation of `silk_pitch_analysis_core` for the SILK encoder analysis path (FIXED_POINT only). | `config.h`, `SigProc_FIX.h`, `pitch_est_defines.h`, `stack_alloc.h`, `debug.h`, `pitch.h` |
| `silk/fixed/process_gains_FIX.c` | 117 | YES | CORE | FIXED-ONLY | Fixed-point Q-domain implementation of `silk_process_gains` for the SILK encoder analysis path (FIXED_POINT only). | `config.h`, `main_FIX.h`, `tuning_parameters.h` |
| `silk/fixed/regularize_correlations_FIX.c` | 47 | YES | CORE | FIXED-ONLY | Fixed-point Q-domain implementation of `silk_regularize_correlations` for the SILK encoder analysis path (FIXED_POINT only). | `config.h`, `main_FIX.h` |
| `silk/fixed/residual_energy16_FIX.c` | 103 | YES | CORE | FIXED-ONLY | Fixed-point Q-domain implementation of `silk_residual_energy16` for the SILK encoder analysis path (FIXED_POINT only). | `config.h`, `main_FIX.h` |
| `silk/fixed/residual_energy_FIX.c` | 98 | YES | CORE | FIXED-ONLY | Fixed-point Q-domain implementation of `silk_residual_energy` for the SILK encoder analysis path (FIXED_POINT only). | `config.h`, `main_FIX.h`, `stack_alloc.h` |
| `silk/fixed/schur64_FIX.c` | 93 | YES | CORE | FIXED-ONLY | Fixed-point Q-domain implementation of `silk_schur64` for the SILK encoder analysis path (FIXED_POINT only). | `config.h`, `SigProc_FIX.h` |
| `silk/fixed/schur_FIX.c` | 107 | YES | CORE | FIXED-ONLY | Fixed-point Q-domain implementation of `silk_schur` for the SILK encoder analysis path (FIXED_POINT only). | `config.h`, `SigProc_FIX.h` |
| `silk/fixed/vector_ops_FIX.c` | 102 | YES | CORE | FIXED-ONLY | Fixed-point Q-domain implementation of `silk_vector_ops` for the SILK encoder analysis path (FIXED_POINT only). | `config.h`, `SigProc_FIX.h`, `pitch.h` |
| `silk/fixed/warped_autocorrelation_FIX.c` | 92 | YES | CORE | FIXED-ONLY | Fixed-point Q-domain implementation of `silk_warped_autocorrelation` for the SILK encoder analysis path (FIXED_POINT only). | `config.h`, `main_FIX.h`, `mips/warped_autocorrelation_FIX_mipsr1.h` |

**Subtotal: 23 files, 3707 lines.**

### 2.6 `silk/float/` — float encoder analysis (`SILK_SOURCES_FLOAT`)

Compiled **only** when `FIXED_POINT` is **not** defined. This is the float encoder analysis path; it is **not** bit-exact across platforms (it relies on `float`/`double` FP contraction and libm), so a bit-exact port targets the fixed build and treats these files as documentation of intent only. `silk/float/wrappers_FLP.c` is special: it `#define`s the shared `silk_*` entry points to their `_FLP` implementations (see below).

| File | Lines | Portable? | Domain | Build variant | Description | Project `#include`s |
|---|---:|---|---|---|---|---|
| `silk/float/LPC_analysis_filter_FLP.c` | 248 | YES | CORE | FLOAT-ONLY | Float implementation of `silk_LPC_analysis_filter` for the SILK encoder analysis path (!FIXED_POINT only; not bit-exact across platforms). | `config.h`, `main_FLP.h` |
| `silk/float/LPC_inv_pred_gain_FLP.c` | 73 | YES | CORE | FLOAT-ONLY | Float implementation of `silk_LPC_inv_pred_gain` for the SILK encoder analysis path (!FIXED_POINT only; not bit-exact across platforms). | `config.h`, `SigProc_FIX.h`, `SigProc_FLP.h`, `define.h` |
| `silk/float/LTP_analysis_filter_FLP.c` | 75 | YES | CORE | FLOAT-ONLY | Float implementation of `silk_LTP_analysis_filter` for the SILK encoder analysis path (!FIXED_POINT only; not bit-exact across platforms). | `config.h`, `main_FLP.h` |
| `silk/float/LTP_scale_ctrl_FLP.c` | 58 | YES | CORE | FLOAT-ONLY | Float implementation of `silk_LTP_scale_ctrl` for the SILK encoder analysis path (!FIXED_POINT only; not bit-exact across platforms). | `config.h`, `main_FLP.h` |
| `silk/float/apply_sine_window_FLP.c` | 81 | YES | CORE | FLOAT-ONLY | Float implementation of `silk_apply_sine_window` for the SILK encoder analysis path (!FIXED_POINT only; not bit-exact across platforms). | `config.h`, `main_FLP.h` |
| `silk/float/autocorrelation_FLP.c` | 53 | YES | CORE | FLOAT-ONLY | Float implementation of `silk_autocorrelation` for the SILK encoder analysis path (!FIXED_POINT only; not bit-exact across platforms). | `config.h`, `typedef.h`, `SigProc_FLP.h` |
| `silk/float/burg_modified_FLP.c` | 187 | YES | CORE | FLOAT-ONLY | Float implementation of `silk_burg_modified` for the SILK encoder analysis path (!FIXED_POINT only; not bit-exact across platforms). | `config.h`, `SigProc_FLP.h`, `tuning_parameters.h`, `define.h` |
| `silk/float/bwexpander_FLP.c` | 49 | YES | CORE | FLOAT-ONLY | Float implementation of `silk_bwexpander` for the SILK encoder analysis path (!FIXED_POINT only; not bit-exact across platforms). | `config.h`, `SigProc_FLP.h` |
| `silk/float/corrMatrix_FLP.c` | 95 | YES | CORE | FLOAT-ONLY | Float implementation of `silk_corrMatrix` for the SILK encoder analysis path (!FIXED_POINT only; not bit-exact across platforms). | `config.h`, `main_FLP.h` |
| `silk/float/encode_frame_FLP.c` | 447 | YES | CORE | FLOAT-ONLY | Float implementation of `silk_encode_frame` for the SILK encoder analysis path (!FIXED_POINT only; not bit-exact across platforms). | `config.h`, `main_FLP.h`, `tuning_parameters.h`, `stack_alloc.h` |
| `silk/float/energy_FLP.c` | 59 | YES | CORE | FLOAT-ONLY | Float implementation of `silk_energy` for the SILK encoder analysis path (!FIXED_POINT only; not bit-exact across platforms). | `config.h`, `SigProc_FLP.h` |
| `silk/float/find_LPC_FLP.c` | 105 | YES | CORE | FLOAT-ONLY | Float implementation of `silk_find_LPC` for the SILK encoder analysis path (!FIXED_POINT only; not bit-exact across platforms). | `config.h`, `define.h`, `main_FLP.h`, `tuning_parameters.h` |
| `silk/float/find_LTP_FLP.c` | 65 | YES | CORE | FLOAT-ONLY | Float implementation of `silk_find_LTP` for the SILK encoder analysis path (!FIXED_POINT only; not bit-exact across platforms). | `config.h`, `main_FLP.h`, `tuning_parameters.h` |
| `silk/float/find_pitch_lags_FLP.c` | 132 | YES | CORE | FLOAT-ONLY | Float implementation of `silk_find_pitch_lags` for the SILK encoder analysis path (!FIXED_POINT only; not bit-exact across platforms). | `config.h`, `main_FLP.h`, `tuning_parameters.h` |
| `silk/float/find_pred_coefs_FLP.c` | 116 | YES | CORE | FLOAT-ONLY | Float implementation of `silk_find_pred_coefs` for the SILK encoder analysis path (!FIXED_POINT only; not bit-exact across platforms). | `config.h`, `main_FLP.h` |
| `silk/float/inner_product_FLP.c` | 59 | YES | CORE | FLOAT-ONLY | Float implementation of `silk_inner_product` for the SILK encoder analysis path (!FIXED_POINT only; not bit-exact across platforms). | `config.h`, `SigProc_FLP.h` |
| `silk/float/k2a_FLP.c` | 54 | YES | CORE | FLOAT-ONLY | Float implementation of `silk_k2a` for the SILK encoder analysis path (!FIXED_POINT only; not bit-exact across platforms). | `config.h`, `SigProc_FLP.h` |
| `silk/float/noise_shape_analysis_FLP.c` | 350 | YES | CORE | FLOAT-ONLY | Float implementation of `silk_noise_shape_analysis` for the SILK encoder analysis path (!FIXED_POINT only; not bit-exact across platforms). | `config.h`, `main_FLP.h`, `tuning_parameters.h` |
| `silk/float/pitch_analysis_core_FLP.c` | 630 | YES | CORE | FLOAT-ONLY | Float implementation of `silk_pitch_analysis_core` for the SILK encoder analysis path (!FIXED_POINT only; not bit-exact across platforms). | `config.h`, `SigProc_FLP.h`, `SigProc_FIX.h`, `pitch_est_defines.h`, `pitch.h` |
| `silk/float/process_gains_FLP.c` | 103 | YES | CORE | FLOAT-ONLY | Float implementation of `silk_process_gains` for the SILK encoder analysis path (!FIXED_POINT only; not bit-exact across platforms). | `config.h`, `main_FLP.h`, `tuning_parameters.h` |
| `silk/float/regularize_correlations_FLP.c` | 48 | YES | CORE | FLOAT-ONLY | Float implementation of `silk_regularize_correlations` for the SILK encoder analysis path (!FIXED_POINT only; not bit-exact across platforms). | `config.h`, `main_FLP.h` |
| `silk/float/residual_energy_FLP.c` | 117 | YES | CORE | FLOAT-ONLY | Float implementation of `silk_residual_energy` for the SILK encoder analysis path (!FIXED_POINT only; not bit-exact across platforms). | `config.h`, `main_FLP.h` |
| `silk/float/scale_copy_vector_FLP.c` | 57 | YES | CORE | FLOAT-ONLY | Float implementation of `silk_scale_copy_vector` for the SILK encoder analysis path (!FIXED_POINT only; not bit-exact across platforms). | `config.h`, `SigProc_FLP.h` |
| `silk/float/scale_vector_FLP.c` | 56 | YES | CORE | FLOAT-ONLY | Float implementation of `silk_scale_vector` for the SILK encoder analysis path (!FIXED_POINT only; not bit-exact across platforms). | `config.h`, `SigProc_FLP.h` |
| `silk/float/schur_FLP.c` | 70 | YES | CORE | FLOAT-ONLY | Float implementation of `silk_schur` for the SILK encoder analysis path (!FIXED_POINT only; not bit-exact across platforms). | `config.h`, `SigProc_FLP.h` |
| `silk/float/sort_FLP.c` | 83 | YES | CORE | FLOAT-ONLY | Float implementation of `silk_sort` for the SILK encoder analysis path (!FIXED_POINT only; not bit-exact across platforms). | `config.h`, `typedef.h`, `SigProc_FLP.h` |
| `silk/float/warped_autocorrelation_FLP.c` | 75 | YES | CORE | FLOAT-ONLY | Float implementation of `silk_warped_autocorrelation` for the SILK encoder analysis path (!FIXED_POINT only; not bit-exact across platforms). | `config.h`, `main_FLP.h` |
| `silk/float/wrappers_FLP.c` | 209 | YES | CORE | FLOAT-ONLY | Float implementation of `silk_wrappers` for the SILK encoder analysis path (!FIXED_POINT only; not bit-exact across platforms). | `config.h`, `main_FLP.h` |

**Subtotal: 28 files, 3754 lines.**

### 2.7 `silk/` — SIMD / assembly only (**do not port**)

| File | Lines | Portable? | Domain | Build variant | Description | Project `#include`s |
|---|---:|---|---|---|---|---|
| `silk/arm/LPC_inv_pred_gain_neon_intr.c` | 288 | SIMD/ASM-ONLY | CORE | NEON | Intrinsics-only override (ARM NEON intrinsics); replaces a pure-C function via the RTCD map. NOT needed for a portable Rust port. | `config.h`, `SigProc_FIX.h`, `define.h` |
| `silk/arm/NSQ_del_dec_neon_intr.c` | 1138 | SIMD/ASM-ONLY | CORE | NEON | Intrinsics-only override (ARM NEON intrinsics); replaces a pure-C function via the RTCD map. NOT needed for a portable Rust port. | `config.h`, `main.h`, `stack_alloc.h`, `os_support.h` |
| `silk/arm/NSQ_neon.c` | 112 | SIMD/ASM-ONLY | CORE | NEON | Intrinsics-only override (ARM NEON intrinsics); replaces a pure-C function via the RTCD map. NOT needed for a portable Rust port. | `config.h`, `main.h`, `stack_alloc.h`, `NSQ.h`, `celt/cpu_support.h`, `celt/arm/armcpu.h` |
| `silk/arm/arm_silk_map.c` | 128 | SIMD/ASM-ONLY | CORE | RTCD (ARM) | Intrinsics-only override (ARM NEON intrinsics); replaces a pure-C function via the RTCD map. NOT needed for a portable Rust port. | `config.h`, `main_FIX.h`, `NSQ.h`, `SigProc_FIX.h` |
| `silk/arm/biquad_alt_neon_intr.c` | 156 | SIMD/ASM-ONLY | CORE | NEON | Intrinsics-only override (ARM NEON intrinsics); replaces a pure-C function via the RTCD map. NOT needed for a portable Rust port. | `config.h`, `stack_alloc.h`, `SigProc_FIX.h` |
| `silk/fixed/arm/warped_autocorrelation_FIX_neon_intr.c` | 265 | SIMD/ASM-ONLY | CORE | NEON | Intrinsics-only override (ARM NEON intrinsics); replaces a pure-C function via the RTCD map. NOT needed for a portable Rust port. | `config.h`, `stack_alloc.h`, `main_FIX.h` |
| `silk/fixed/x86/burg_modified_FIX_sse4_1.c` | 400 | SIMD/ASM-ONLY | CORE | SSE4.1 | Intrinsics-only override (x86 intrinsics); replaces a pure-C function via the RTCD map. NOT needed for a portable Rust port. | `config.h`, `SigProc_FIX.h`, `define.h`, `tuning_parameters.h`, `pitch.h`, `celt/x86/x86cpu.h` |
| `silk/fixed/x86/vector_ops_FIX_sse4_1.c` | 93 | SIMD/ASM-ONLY | CORE | SSE4.1 | Intrinsics-only override (x86 intrinsics); replaces a pure-C function via the RTCD map. NOT needed for a portable Rust port. | `config.h`, `main.h`, `SigProc_FIX.h`, `pitch.h`, `celt/x86/x86cpu.h` |
| `silk/float/x86/inner_product_FLP_avx2.c` | 85 | SIMD/ASM-ONLY | CORE | AVX2 | Intrinsics-only override (x86 intrinsics); replaces a pure-C function via the RTCD map. NOT needed for a portable Rust port. | `config.h`, `SigProc_FLP.h` |
| `silk/x86/NSQ_del_dec_avx2.c` | 1077 | SIMD/ASM-ONLY | CORE | AVX2 | Intrinsics-only override (x86 intrinsics); replaces a pure-C function via the RTCD map. NOT needed for a portable Rust port. | `config.h`, `opus_defines.h`, `main.h`, `stack_alloc.h`, `NSQ.h`, `celt/x86/x86cpu.h` |
| `silk/x86/NSQ_del_dec_sse4_1.c` | 915 | SIMD/ASM-ONLY | CORE | SSE4.1 | Intrinsics-only override (x86 intrinsics); replaces a pure-C function via the RTCD map. NOT needed for a portable Rust port. | `config.h`, `main.h`, `celt/x86/x86cpu.h`, `stack_alloc.h` |
| `silk/x86/NSQ_sse4_1.c` | 772 | SIMD/ASM-ONLY | CORE | SSE4.1 | Intrinsics-only override (x86 intrinsics); replaces a pure-C function via the RTCD map. NOT needed for a portable Rust port. | `config.h`, `main.h`, `celt/x86/x86cpu.h`, `stack_alloc.h` |
| `silk/x86/VAD_sse4_1.c` | 289 | SIMD/ASM-ONLY | CORE | SSE4.1 | Intrinsics-only override (x86 intrinsics); replaces a pure-C function via the RTCD map. NOT needed for a portable Rust port. | `config.h`, `main.h`, `stack_alloc.h` |
| `silk/x86/VQ_WMat_EC_sse4_1.c` | 173 | SIMD/ASM-ONLY | CORE | SSE4.1 | Intrinsics-only override (x86 intrinsics); replaces a pure-C function via the RTCD map. NOT needed for a portable Rust port. | `config.h`, `main.h`, `celt/x86/x86cpu.h` |
| `silk/x86/x86_silk_map.c` | 179 | SIMD/ASM-ONLY | CORE | RTCD (x86) | Intrinsics-only override (x86 intrinsics); replaces a pure-C function via the RTCD map. NOT needed for a portable Rust port. | `config.h`, `celt/x86/x86cpu.h`, `structs.h`, `SigProc_FIX.h`, `SigProc_FLP.h`, `pitch.h`, `main.h`, `fixed/main_FIX.h` |

**Subtotal: 15 files, 6070 lines.**

### 2.8 `src/` — portable C (`OPUS_SOURCES`)

| File | Lines | Portable? | Domain | Build variant | Description | Project `#include`s |
|---|---:|---|---|---|---|---|
| `src/analysis.c` | 982 | YES | FLOAT-API | FLOAT-API | Encoder-side signal analysis (music/speech classification, tonality, activity, leak boost) driving CELT mode decisions — FLOAT API only. | `config.h`, `mathops.h`, `kiss_fft.h`, `celt.h`, `modes.h`, `arch.h`, `quant_bands.h`, `analysis.h`, `mlp.h`, `stack_alloc.h`, `float_cast.h` |
| `src/extensions.c` | 672 | YES | CORE | ALWAYS (src) | Opus packet extensions (the 1.6 'extensions' mechanism) and the `opus_extension_*` parse/iterate helpers. | `config.h`, `opus_types.h`, `opus_defines.h`, `arch.h`, `os_support.h`, `opus_private.h` |
| `src/mapping_matrix.c` | 997 | YES | CORE | ALWAYS (src) | Dense matrix multiply/accumulate used by multistream and projection (`mapping_matrix_*`), with fixed/float/QEXT variants. | `config.h`, `arch.h`, `float_cast.h`, `opus_private.h`, `opus_defines.h`, `mapping_matrix.h` |
| `src/mlp.c` | 131 | YES | FLOAT-API | FLOAT-API | Small multi-layer perceptron inference used by `src/analysis.c` — FLOAT API only. | `config.h`, `opus_types.h`, `opus_defines.h`, `arch.h`, `mlp.h` |
| `src/mlp_data.c` | 671 | YES | FLOAT-API | FLOAT-API | Trained MLP weights for `src/mlp.c` — FLOAT API only. | `config.h`, `mlp.h` |
| `src/opus.c` | 398 | YES | CORE | ALWAYS (src) | Library-level helpers: `opus_strerror`, `opus_get_version_string`, and `opus_packet_parse_impl`/`opus_packet_init_impl`/`opus_packet_get_*` TOC and frame-header parsing. | `config.h`, `opus.h`, `mathops.h`, `opus_private.h` |
| `src/opus_decoder.c` | 1691 | YES | CORE | ALWAYS (src) | The `OpusDecoder`: packet parsing, mode/frame-size detection, CELT+SILK hybrid decoding, PLC/soft-decoding glue, float/int16/24 and multistream-facing CTLs. | `config.h`, `celt.h`, `opus.h`, `entdec.h`, `modes.h`, `API.h`, `stack_alloc.h`, `float_cast.h`, `opus_private.h`, `os_support.h`, `structs.h`, `define.h`, `mathops.h`, `cpu_support.h`, `dred_rdovae_dec_data.h`, `dred_rdovae_dec.h`, `osce.h` |
| `src/opus_encoder.c` | 3365 | YES | CORE | ALWAYS (src) | The `OpusEncoder`: mode/bandwidth/bitrate decision logic, SILK and CELT encoder driving, hybrid framing, DTX/FEC, VBR computation, delay/repacketisation glue. | `config.h`, `celt.h`, `entenc.h`, `modes.h`, `API.h`, `stack_alloc.h`, `float_cast.h`, `opus.h`, `arch.h`, `pitch.h`, `opus_private.h`, `os_support.h`, `cpu_support.h`, `analysis.h`, `mathops.h`, `tuning_parameters.h`, `dred_coding.h`, `fixed/structs_FIX.h`, `float/structs_FLP.h` |
| `src/opus_multistream.c` | 91 | YES | CORE | ALWAYS (src) | Multistream helper: `opus_multistream_encoder_get_size` / `opus_multistream_decoder_get_size` and the channel mapping matrix layout. | `config.h`, `opus_multistream.h`, `opus.h`, `opus_private.h`, `stack_alloc.h`, `float_cast.h`, `os_support.h` |
| `src/opus_multistream_decoder.c` | 565 | YES | CORE | ALWAYS (src) | The `OpusMSDecoder`: per-stream `OpusDecoder` fan-in, channel-mapping matrix, and soft-decoding hooks. | `config.h`, `opus_multistream.h`, `opus.h`, `opus_private.h`, `stack_alloc.h`, `float_cast.h`, `os_support.h` |
| `src/opus_multistream_encoder.c` | 1363 | YES | CORE | ALWAYS (src) | The `OpusMSEncoder`: per-stream `OpusEncoder` fan-out, channel-mapping matrix, and coupled-stream handling. | `config.h`, `opus_multistream.h`, `opus.h`, `opus_private.h`, `stack_alloc.h`, `float_cast.h`, `os_support.h`, `mathops.h`, `mdct.h`, `modes.h`, `bands.h`, `quant_bands.h`, `pitch.h` |
| `src/opus_projection_decoder.c` | 282 | YES | CORE | ALWAYS (src) | Ambisonics projection super-frame decoder built on the multistream decoder plus a mixing matrix. | `config.h`, `mathops.h`, `os_support.h`, `opus_private.h`, `opus_defines.h`, `opus_projection.h`, `opus_multistream.h`, `mapping_matrix.h`, `stack_alloc.h` |
| `src/opus_projection_encoder.c` | 521 | YES | CORE | ALWAYS (src) | Ambisonics projection super-frame encoder built on the multistream encoder plus a mixing matrix. | `config.h`, `mathops.h`, `os_support.h`, `opus_private.h`, `opus_defines.h`, `opus_projection.h`, `opus_multistream.h`, `stack_alloc.h`, `mapping_matrix.h` |
| `src/repacketizer.c` | 468 | YES | CORE | ALWAYS (src) | The `OpusRepacketizer`: frame concatenation, padding, self-delimited packet parsing and repacketised output. | `config.h`, `opus.h`, `opus_private.h`, `os_support.h`, `stack_alloc.h` |

**Subtotal: 14 files, 12197 lines.**

### 2.9 `src/` — demos and tools (not part of the library)

| File | Lines | Portable? | Domain | Build variant | Description | Project `#include`s |
|---|---:|---|---|---|---|---|
| `src/opus_compare.c` | 382 | NOT-CORE | CORE | NOT-CORE | Bit-exactness comparison tool (RMS-weighted difference) used by the test suite; not part of the library. | _(none)_ |
| `src/opus_demo.c` | 1294 | NOT-CORE | CORE | NOT-CORE | Reference CLI encoder/decoder demo; not part of the library. | `config.h`, `opus.h`, `debug.h`, `opus_types.h`, `opus_private.h`, `opus_multistream.h`, `lossgen.h` |
| `src/qext_compare.c` | 530 | NOT-CORE | CORE | NOT-CORE | QEXT comparison tool built on `celt/mini_kfft.c`; not part of the library. | `mini_kfft.c` |
| `src/repacketizer_demo.c` | 247 | NOT-CORE | CORE | NOT-CORE | Demo program for the repacketizer API; not part of the library. | `config.h`, `opus.h` |

**Subtotal: 4 files, 2453 lines.**

### 2.10 The portable core set, and what the SIMD directories add

The complete portable core, in make-variable terms, is:

```
CELT_SOURCES          18 files   celt/*.c
SILK_SOURCES          77 files   silk/*.c
SILK_SOURCES_FIXED    23 files   silk/fixed/*.c     (FIXED_POINT)
  or SILK_SOURCES_FLOAT 28 files silk/float/*.c     (!FIXED_POINT)
OPUS_SOURCES          11 files   src/*.c
OPUS_SOURCES_FLOAT     3 files   src/analysis.c src/mlp.c src/mlp_data.c
                                 (only when the float API is on)
```

Everything in `celt/x86/`, `celt/arm/`, `celk/mips/`, `silk/x86/`, `silk/arm/`,
`silk/fixed/x86/`, `silk/fixed/arm/`, `silk/float/x86/` exists for speed only.
They are reached through the RTCD tables in
`celt/x86/x86_celt_map.c`, `celt/arm/arm_celt_map.c`, `silk/x86/x86_silk_map.c`,
`silk/arm/arm_silk_map.c` (plus the CPU-feature probes in `x86cpu.c` /
`armcpu.c`). A portable port deletes the whole dispatch layer and calls the
`_c` function unconditionally. Concretely, the overrides are:

| SIMD file | Replaces (portable C) | make variable |
|---|---|---|
| `celt/x86/pitch_sse.c` | `celt_pitch_xcorr` | `CELT_SOURCES_SSE` |
| `celt/x86/pitch_sse2.c` | `celt_inner_prod` (float) | `CELT_SOURCES_SSE2` |
| `celt/x86/vq_sse2.c` | `celt_inner_prod` (fixed) | `CELT_SOURCES_SSE2` |
| `celt/x86/pitch_sse4_1.c` | `celt_pitch_xcorr`, `xcorr_kernel` | `CELT_SOURCES_SSE4_1` |
| `celt/x86/celt_lpc_sse4_1.c` | `celt_fir`/`celt_iir` | `CELT_SOURCES_SSE4_1` |
| `celt/x86/pitch_avx.c` | `celt_pitch_xcorr` | `CELT_SOURCES_AVX2` |
| `silk/x86/NSQ_sse4_1.c` | `silk_noise_shape_quantizer` | `SILK_SOURCES_SSE4_1` |
| `silk/x86/NSQ_del_dec_sse4_1.c`, `silk/x86/NSQ_del_dec_avx2.c` | `silk_NSQ_del_dec` | `SILK_SOURCES_SSE4_1`, `SILK_SOURCES_AVX2` |
| `silk/x86/VAD_sse4_1.c` | `silk_VAD_GetSA_Q8` | `SILK_SOURCES_SSE4_1` |
| `silk/x86/VQ_WMat_EC_sse4_1.c` | `silk_VQ_WMat_EC` | `SILK_SOURCES_SSE4_1` |
| `silk/fixed/x86/vector_ops_FIX_sse4_1.c` | `silk_inner_prod_aligned`, `silk_sum_sqr_shift` | `SILK_SOURCES_FIXED_SSE4_1` |
| `silk/fixed/x86/burg_modified_FIX_sse4_1.c` | `silk_burg_modified` | `SILK_SOURCES_FIXED_SSE4_1` |
| `celt/arm/celt_neon_intr.c` | `celt_pitch_xcorr` / `celt_inner_prod` | `CELT_SOURCES_ARM_NEON_INTR` |
| `celt/arm/pitch_neon_intr.c` | `celt_pitch_xcorr` | `CELT_SOURCES_ARM_NEON_INTR` |
| `silk/arm/NSQ_neon.c`, `NSQ_del_dec_neon_intr.c` | `silk_noise_shape_quantizer`, `silk_NSQ_del_dec` | `SILK_SOURCES_ARM_NEON_INTR` |
| `silk/arm/biquad_alt_neon_intr.c` | `silk_biquad_alt_stride1/2` | `SILK_SOURCES_ARM_NEON_INTR` |
| `silk/arm/LPC_inv_pred_gain_neon_intr.c` | `silk_LPC_inverse_pred_gain_c` | `SILK_SOURCES_ARM_NEON_INTR` |
| `silk/fixed/arm/warped_autocorrelation_FIX_neon_intr.c` | `silk_warped_autocorrelation_FIX` | `SILK_SOURCES_FIXED_ARM_NEON_INTR` |
| `celt/arm/celt_fft_ne10.c`, `celt_mdct_ne10.c` | kiss FFT / MDCT via Ne10 | `CELT_SOURCES_ARM_NE10` |
| `celt/arm/celt_pitch_xcorr_arm.s` | `celt_pitch_xcorr` (inline asm) | `CELT_SOURCES_ARM_ASM` |
| `silk/float/x86/inner_product_FLP_avx2.c` | `silk_inner_product_FLP` | `SILK_SOURCES_FLOAT_AVX2` |

**Every SIMD override is required by libopus to be bit-exact with its C
counterpart** (`--enable-check-asm` exists to verify this), so ignoring them
cannot change the output bitstream — only the speed. This is confirmed by the
`OPUS_CHECK_ASM` instrumentation and is a useful invariant for the port's test
strategy: the portable `_c` function is the normative definition.

### 2.11 `silk/float/wrappers_FLP.c` — the float/fixed dispatch trick

The SILK top-level files call one set of names (`silk_find_LPC`,
`silk_burg_modified`, ...). In the fixed build those names are the functions in
`silk/fixed/*.c`. In the float build, the `_FLP` implementations are linked in,
and `silk/float/wrappers_FLP.c` provides the translation layer. It works by
macro-renaming the shared entry points, e.g.:

```c
/* silk/float/wrappers_FLP.c */
#define silk_encoder_state_Fxx        silk_encoder_state_FLP
#define silk_enc_API_enc_frame_FLP    silk_Encode
```
(refer to the file itself for the exact current set; the pattern is that the
top-level `silk_Encode` selects `silk_encode_frame_FLP` while the *parameter*
structs are the FLP variants.) The practical consequence for the port: the
`silk/fixed/*` function signatures are the normative ones, and the `_FLP`
files can be ignored once `FIXED_POINT` is chosen.

### 2.12 Header files of the core (line counts)


| Header | Lines | Purpose |
|---|---:|---|
| `celt/_kiss_fft_guts.h` | 188 | kiss-FFT internals: `S_MUL`/`C_MUL`/`C_MULBYSCALAR`/`HALF_OF` and the `#ifdef FIXED_POINT` sample types (`kiss_fft_scalar`, `SAMP_MAX`, `FRACBITS`) plus the `CHECK_OVERFLOW` debug hook. |
| `celt/arch.h` | 400 | Central type/config header: `opus_val16/32/64`, `celt_sig/norm/ener/glog/coef`, `opus_res` and the `RES_*`/`SIG_*` conversions, `SAT16`, `QCONST*`, `celt_assert`, `SIG_SHIFT`/`NORM_SHIFT`/`DB_SHIFT`, `OPUS_FAST_INT64`. |
| `celt/arm/armcpu.h` | 90 | ARM CPU-feature probe declarations (`opus_select_arch`). SIMD/RTCD only. |
| `celt/arm/fft_arm.h` | 71 | ARM FFT dispatch declarations. |
| `celt/arm/fixed_arm64.h` | 35 | AArch64 NEON fixed-point macro overrides (`MULT16_16`, `SHR32`, ...) — replaces `fixed_generic.h` when `OPUS_ARM_PRESUME_AARCH64_NEON_INTR`. |
| `celt/arm/fixed_armv4.h` | 80 | ARMv4 inline-asm fixed-point macro overrides. |
| `celt/arm/fixed_armv5e.h` | 151 | ARMv5E (EDSP) inline-asm fixed-point macro overrides (`SMULBB`, `SMLABB`, ...). |
| `celt/arm/kiss_fft_armv4.h` | 121 | ARMv4 fixed-point `kf_bfly*` assembly. |
| `celt/arm/kiss_fft_armv5e.h` | 118 | ARMv5E EDSP `kf_bfly*` assembly. |
| `celt/arm/mathops_arm.h` | 103 | ARM `celt_rsqrt_norm`/`celt_sqrt` overrides. |
| `celt/arm/mdct_arm.h` | 59 | ARM MDCT overrides (NE10 / NEON). |
| `celt/arm/pitch_arm.h` | 160 | ARM `celt_pitch_xcorr` / `xcorr_kernel` overrides. |
| `celt/bands.h` | 125 | Declarations for the band-domain functions in `celt/bands.c` (`compute_band_energies`, `denormalise_bands`, `quant_all_bands`, `anti_collapse`, spreading/hysteresis helpers, `haar1`). |
| `celt/celt.h` | 274 | CELT public/internal API: `CELTEncoder/CELTDecoder/CELTMode` aliases, `AnalysisInfo`/`SILKInfo`, all `CELT_*` CTL request codes, `celt_encode_with_ec`/`celt_decode_with_ec` prototypes, `trim_icdf`/`spread_icdf`/`tapset_icdf`/`toOpusTable`/`fromOpusTable`. |
| `celt/celt_lpc.h` | 66 | Declarations for `_celt_lpc`, `celt_fir`, `celt_iir`, `celt_autocorr` and the `celt_iir`/`celt_fir` inline KERNEL dispatch macro. |
| `celt/cpu_support.h` | 72 | `opus_select_arch()` declaration and the `OPUS_ARCHMASK`/arch enum; the RTCD entry point. |
| `celt/cwrs.h` | 48 | Declarations of `encode_pulses`/`decode_pulses`/`get_required_bits`/`log2_frac` and the `CeltPulseCache` helper prototypes. |
| `celt/ecintrin.h` | 91 | `EC_CLZ0`/`EC_CLZ`/`EC_ILOG` bit-scan primitives built on `__builtin_clz`/`_BitScanReverse`/fallback loops — used by `ec_ilog`. |
| `celt/entcode.h` | 152 | `ec_ctx` definition (the single struct behind `ec_enc` and `ec_dec`), `BITRES`/`EC_WINDOW_SIZE`/`EC_UINT_BITS`, `ec_tell`/`ec_tell_frac`, `celt_udiv`/`celt_sudiv`, `ec_ilog`; the bit-exactness core of the range coder. |
| `celt/entdec.h` | 110 | `ec_dec` API: `ec_dec_init`, `ec_decode(_bin)`, `ec_dec_update`, `ec_dec_bit_logp`, `ec_dec_icdf(16)`, `ec_dec_uint`, `ec_dec_bits`, `ec_dec_tell`. |
| `celt/entenc.h` | 119 | `ec_enc` API: `ec_enc_init`, `ec_encode(_bin)`, `ec_enc_bit_logp`, `ec_enc_icdf(16)`, `ec_enc_uint`, `ec_enc_bits`, `ec_enc_patch_initial_bits`, `ec_enc_shrink`, `ec_enc_done`. |
| `celt/fixed_debug.h` | 848 | `FIXED_DEBUG` replacements for every fixed-point macro that `silk_assert`/`celt_assert` on overflow or out-of-range shift. An *oracle* for the port, never the shipping semantics. |
| `celt/fixed_generic.h` | 218 | The normative fixed-point macro set: `MULT16_16`, `MULT16_32_Q15/16`, `MAC16_16`, `SHR/PSHR`, `EXTRACT16/EXTEND32`, `SATURATE`, `SROUND`, `ADD32/SUB32`, plus the `#ifdef __GNUC__` inline-asm fast paths. **This file is the single most important thing to port verbatim.** |
| `celt/float_cast.h` | 176 | `FLOAT2INT16`/`FLOAT2INT24`/`float2int`/`int16tofloat` conversions, with `DISABLE_FLOAT_API` and `HAVE_LRINTF` variants (lrintf vs cast rounding). |
| `celt/kiss_fft.h` | 209 | `kiss_fft_state`, `kiss_fft_cpx`, `kiss_twiddle_cpx`, `arch_fft_state`, `opus_fft_alloc*`/`opus_fft`/`opus_ifft` declarations, and the `static_modes_*_arm_ne10.h` includes. |
| `celt/laplace.h` | 57 | `ec_laplace_encode/decode` and the `ec_laplace_encode_p0/decode_p0` prototypes. |
| `celt/mathops.h` | 662 | `FLOAT_APPROX`-dependent `celt_rsqrt`/`celt_sqrt`/`celt_log2`/`celt_exp2`/`celt_rcp` inline implementations and the fixed-point fallbacks; `isqrt32`, `frac_div32`, `celt_cos_norm`, `celt_atan2p`, `celt_exp2_frac`; also holds the small math tables. |
| `celt/mdct.h` | 111 | `mdct_lookup` (twiddle pointer, `nfft`, `maxshift`, `kfft[]`) and `clt_mdct_init/clear/forward(_c)/backward(_c)`. |
| `celt/mfrngcod.h` | 48 | The `ec_decode`/`ec_dec_update`/`ec_encode` split macros (`ec_decode`, `ec_decode_bin`, `ec_dec_update`, `ec_encode`, `ec_encode_bin`) that `entdec.c`/`entenc.c` implement — the arithmetic-coder equations. **Normative.** |
| `celt/mips/celt_mipsr1.h` | 198 | MIPS asm only. |
| `celt/mips/fixed_generic_mipsr1.h` | 158 | MIPS asm only. |
| `celt/mips/kiss_fft_mipsr1.h` | 271 | MIPS asm only. |
| `celt/mips/mdct_mipsr1.h` | 383 | MIPS asm only. |
| `celt/mips/pitch_mipsr1.h` | 359 | MIPS asm only. |
| `celt/modes.h` | 85 | `OpusCustomMode`/`CeltMode` (eBands, allocation vectors, `logN`, `cache`, MDCT twiddles, QEXT fields), `PulseCache`, `opus_custom_mode_create/find/destroy`, `CELT_MODE_*`. |
| `celt/os_support.h` | 98 | `opus_alloc`/`opus_free`/`opus_realloc` and `RESTORE_STACK`/`SAVE_STACK`/`ALLOC` plumbing. |
| `celt/pitch.h` | 203 | `celt_pitch_xcorr`, `xcorr_kernel`, `celt_autocorr`, `pitch_downsample`, `pitch_search`, `remove_doubling` declarations and the `PITCH_EST_SHORT`/`CELT_PITCH_MIN` constants. |
| `celt/quant_bands.h` | 66 | `amp2Log2`/`log2Amp` and the six `(un)quant_*_energy*` prototypes, plus `e_prob_model`/`small_energy_icdf` externs. |
| `celt/rate.h` | 104 | `clt_compute_allocation`, `compute_pulse_cache`, `interp_bits2pulses`, `get_pulses`/`bits2pulses` and the `MAX_PSEUDO`/`LOG_MAX_PSEUDO` constants. |
| `celt/stack_alloc.h` | 209 | `VARDECL`/`ALLOC`/`SAVE_STACK`/`RESTORE_STACK` — stack, `alloca`, or alloca-emulation for scratch buffers. Bit-exactness-neutral. |
| `celt/static_modes_fixed.h` | 4446 | **Generated** fixed-point mode tables: `eBands5ms` indices, `mdct_twiddles960`, `allocVectors`/`logN400`/`cache_index`/`cache_bits`/`cache_caps*` for `OPUS_CUSTOM_NB/MB/WB` and `CELT_MODE_*`. 4446 lines; do not hand-port, regenerate mechanically. |
| `celt/static_modes_fixed_arm_ne10.h` | 388 | NE10-specific fixed-point mode tables. SIMD only. |
| `celt/static_modes_float.h` | 2516 | **Generated** float mode tables (`mdct_twiddles960` as float, the same allocation vectors). 2516 lines. |
| `celt/static_modes_float_arm_ne10.h` | 404 | NE10-specific float mode tables. SIMD only. |
| `celt/vq.h` | 98 | `alg_quant`/`alg_unquant`/`renormalise_vector`/`stereo_itheta`/`exp_rotation`/`op_pvq_search` prototypes and the `SPREAD_*` enums. |
| `celt/x86/celt_lpc_sse.h` | 67 | SSE4.1 `celt_fir`/`celt_iir` helpers. SIMD only. |
| `celt/x86/pitch_sse.h` | 216 | SSE/SSE2/SSE4.1 `celt_pitch_xcorr`/`xcorr_kernel`/`celt_inner_prod` helpers. SIMD only. |
| `celt/x86/vq_sse.h` | 52 | SSE2 `celt_inner_prod` helpers. SIMD only. |
| `celt/x86/x86_arch_macros.h` | 47 | `__m128`↔`opus_val32` conversion macros and the `#define` list that remaps the generic macros to SSE. SIMD only. |
| `celt/x86/x86cpu.h` | 112 | `_xgetbv`/`__cpuid` wrappers and the x86 arch-selection macro. SIMD only. |
| `silk/API.h` | 150 | SILK decoder API (`silk_Decode`, `silk_DecControlStruct`) plus the `ENABLE_DEEP_PLC` LPCNet entry points. |
| `silk/Inlines.h` | 180 | Fixed-point inline primitives: `silk_RSHIFT_ROUND`, `silk_SMULWB`, `silk_SMLAWB`, `silk_SMULWW`, `silk_SMLAWW`, `silk_SMMUL`, `silk_CLZ32`, `silk_CLZ_FRAC`, `silk_abs_int32`, `silk_ADD_SAT32`. **Normative.** |
| `silk/MacroCount.h` | 709 | WMOPS counting rewrite of the SILK macros (`silk_MACRO_COUNT`). Never used in the reference bitstream build. |
| `silk/MacroDebug.h` | 945 | `FIXED_DEBUG` overflow-checking rewrite of every SILK macro. Oracle only. |
| `silk/NSQ.h` | 105 | `silk_nsq_state` accessors and the NSQ gain/quantisation helper prototypes. |
| `silk/PLC.h` | 64 | `silk_PLC_struct`-related prototypes and PLC constants; the `ENABLE_DEEP_PLC` hooks. |
| `silk/SigProc_FIX.h` | 641 | The fixed-point signal-processing interface: resampler, biquad, LPC analysis/inverse-gain, FFT-based autocorrelation, `silk_find_pitch_lags`, and the inline `silk_SQRT_APPROX`, `silk_DIV32_varQ`, `silk_INVERSE32_varQ`, `silk_DIV32_16`, `silk_lin2log`-adjacent helpers. **Normative.** |
| `silk/arm/LPC_inv_pred_gain_arm.h` | 57 | NEON inverse-prediction-gain prototypes. |
| `silk/arm/NSQ_del_dec_arm.h` | 100 | NEON NSQ delayed-decision prototypes. |
| `silk/arm/NSQ_neon.h` | 114 | NEON NSQ prototypes. |
| `silk/arm/SigProc_FIX_armv4.h` | 47 | ARMv4 inline-asm SILK primitives. |
| `silk/arm/SigProc_FIX_armv5e.h` | 61 | ARMv5E EDSP SILK primitives. |
| `silk/arm/biquad_alt_arm.h` | 68 | NEON biquad prototypes. |
| `silk/arm/macros_arm64.h` | 39 | AArch64 NEON SILK macro overrides (`silk_SMULL`, `silk_SMLAL`, ...). |
| `silk/arm/macros_armv4.h` | 110 | ARMv4 SILK macro overrides. |
| `silk/arm/macros_armv5e.h` | 220 | ARMv5E SILK macro overrides. |
| `silk/control.h` | 164 | `silk_EncControlStruct` and the encoder-CTL request codes (`silk_*_REQUEST`). |
| `silk/debug.h` | 259 | SILK debug/timer macros (`silk_TimerSave`, WMOPS counters); compiled out by default. |
| `silk/define.h` | 231 | SILK constants: `MAX_NB_SUBFR`, `MAX_LPC_ORDER`, `MAX_FRAME_LENGTH`, `MAX_NB_SHELL_BLOCKS`, `TYPE_*`, `silk_Resampler_*`, the `ENABLE_QEXT` resolution switches, and many small tables/index bounds. |
| `silk/errors.h` | 90 | SILK error codes (`SILK_NO_ERROR`, `SILK_ENC_INVALID_*`). |
| `silk/fixed/arm/warped_autocorrelation_FIX_arm.h` | 68 | NEON-only prototypes. |
| `silk/fixed/main_FIX.h` | 233 | Encoder-analysis prototypes for the `FIXED_POINT` build, `#define silk_encoder_state_Fxx silk_encoder_state_FIX`, and `silk_encode_frame_FIX`. |
| `silk/fixed/mips/warped_autocorrelation_FIX_mipsr1.h` | 189 | MIPS asm only. |
| `silk/fixed/structs_FIX.h` | 109 | `silk_shape_state_FIX`, `silk_encoder_state_FIX`, `silk_encoder_control_FIX`, and the outer `silk_encoder` for the fixed build. |
| `silk/float/SigProc_FLP.h` | 197 | Float signal-processing interface: `silk_float` vectors, `silk_inner_product_FLP`, `silk_energy_FLP`, `silk_burg_modified_FLP`, `silk_find_pitch_lags_FLP` etc. Float-encoder only. |
| `silk/float/main_FLP.h` | 282 | Encoder-analysis prototypes for the float build, `#define silk_encoder_state_Fxx silk_encoder_state_FLP`, and `silk_encode_frame_FLP`. |
| `silk/float/structs_FLP.h` | 105 | `silk_shape_state_FLP`, `silk_encoder_state_FLP`, `silk_encoder_control_FLP`, and the outer `silk_encoder` for the float build. |
| `silk/macros.h` | 154 | The normative SILK fixed-point macro definitions (`silk_MUL`, `silk_MLA`, `silk_SMULWB`, `silk_ADD_SAT32`, `silk_LIMIT`, ...). **Normative.** |
| `silk/main.h` | 486 | The SILK encoder/decoder internal interface: every `silk_*` prototype for the analysis, quantisation, NSQ, NLSF, stereo and resampler functions, plus `silk_encode_frame` selection. |
| `silk/mips/NSQ_mips.h` | 137 | MIPS asm NSQ only. |
| `silk/mips/macros_mipsr1.h` | 123 | MIPS asm macros only. |
| `silk/mips/sigproc_fix_mipsr1.h` | 60 | MIPS asm only. |
| `silk/pitch_est_defines.h` | 87 | Pitch-estimator constants (`PE_MAX_LAG`, `PE_NB_CBKS_STAGE*`, ...) and the `silk_CB_lags_*`/`silk_Lag_range_*` extern declarations. **No state struct here** in 1.6.1. |
| `silk/resampler_private.h` | 81 | Resampler private state/table declarations and the `silk_resampler_private_*` prototypes. |
| `silk/resampler_rom.h` | 60 | `silk_resampler_frac_FIR_12`, the up2-HQ coefficient arrays and the resampler dispatch ROM externs. |
| `silk/resampler_structs.h` | 52 | `silk_resampler_state_struct` — the resampler state a port must reproduce (coefs, FIR history, IIR state, input/output delay). |
| `silk/structs.h` | 357 | All shared SILK state structs: `silk_nsq_state`, `silk_VAD_state`, `silk_LP_state`, `silk_NLSF_CB_struct`, `stereo_enc_state`, `stereo_dec_state`, `SideInfoIndices`, `silk_encoder_state`, `silk_PLC_struct`, `silk_CNG_struct`, `silk_decoder_state`, `silk_decoder_control`. |
| `silk/tables.h` | 106 | Extern declarations of every SILK constant table (NLSF codebooks, gains, LTP, pitch lag, pulses, LSF cos). The mechanical table manifest. |
| `silk/tuning_parameters.h` | 146 | Encoder tuning constants (SNR targets, LTP/NLSF weights, VAD thresholds). |
| `silk/typedef.h` | 81 | `silk_float` (float build only), int limit macros, and the `silk_assert` definition. |
| `silk/x86/SigProc_FIX_sse.h` | 99 | SSE4.1 SILK signal-processing prototypes. |
| `silk/x86/main_sse.h` | 299 | SSE4.1/AVX2 SILK main prototypes. |
| `src/analysis.h` | 103 | `AnalysisInfo` update prototypes for the float-API analysis layer. |
| `src/mapping_matrix.h` | 157 | `mapping_matrix` struct and `mapping_matrix_*` prototypes. |
| `src/mlp.h` | 60 | MLP layer structs and `mlp_process`/`mlp_init` prototypes. |
| `src/opus_private.h` | 266 | `OpusRepacketizer` definition, `opus_packet_parse_impl`, mode/bandwidth helpers shared by src/*.c. |

## 3. Dependency order (bottom-up porting order)

The layering below is derived from an **actual file-level call graph** computed
mechanically over the portable core (see Appendix A for the raw graph, and
`docs/gen_graph.py` for the extractor). Each layer only calls into layers at or
below it, with three exceptions that are called out explicitly at the end.

### 3.1 Layer table

| Layer | Contents | Needs from the layer below |
|---|---|---|
| **L0. Types & config** | `include/opus_types.h`, `include/opus_defines.h`, `include/opus.h`, `include/opus_custom.h`, `celt/arch.h`, `celt/fixed_generic.h`, `celt/float_cast.h`, `celt/os_support.h`, `celt/stack_alloc.h`, `silk/typedef.h`, `silk/macros.h`, `silk/Inlines.h`, `silk/define.h`, `silk/errors.h`, `silk/tuning_parameters.h`, `silk/control.h` | nothing. Pure typedefs, `#define`s and inline primitives. Every offset, mask, Q-format and saturation rule in the port is fixed here. |
| **L1. Entropy coder** | `celt/entcode.[ch]`, `celt/entenc.[ch]`, `celt/entdec.[ch]`, `celt/mfrngcod.h`, `celt/ecintrin.h`, `celt/laplace.[ch]` | L0 only. Verified: `entcode.c`, `entenc.c`, `entdec.c` have **zero** cross-file calls; `laplace.c` calls only `entenc.c`/`entdec.c`. |
| **L2. Math primitives** | `celt/mathops.[ch]`, `celt/cwrs.[ch]` | L1 (`cwrs.c` calls `ec_enc_uint`/`ec_dec_uint`). `mathops.c` has zero cross-file calls. These are the leaf numerical routines: integer sqrt, reciprocal, cos-norm, log/exp2 and the combinatorial `V(N,K)` machinery. |
| **L3. FFT / MDCT / modes / rate** | `celt/kiss_fft.[ch]`, `celt/_kiss_fft_guts.h`, `celt/mdct.[ch]`, `celt/rate.[ch]`, `celt/modes.[ch]`, `celt/static_modes_fixed.h` | L2 + L1. `kiss_fft.c`: zero cross-file calls. `mdct.c` → `kiss_fft.c`, `mathops.c`. `rate.c` → `cwrs.c`, `entcode.c`, `entdec.c`, `entenc.c`. `modes.c` → `cwrs.c`, `mdct.c`, `rate.c`. `modes.c` also owns the static mode tables. |
| **L4. CELT band domain** | `celt/celt_lpc.[ch]`, `celt/pitch.[ch]`, `celt/vq.[ch]`, `celt/bands.[ch]`, `celt/quant_bands.[ch]`, `celt/celt.h` | L3 + L1. `celt_lpc.c` → `mathops.c`. `pitch.c` → `celt_lpc.c`, `mathops.c`. `vq.c` → `cwrs.c`, `entdec.c`, `entenc.c`, `mathops.c`. `bands.c` → `entcode.c`, `entdec.c`, `entenc.c`, `mathops.c`, **`vq.c`**. `quant_bands.c` → `entcode.c`, `entdec.c`, `entenc.c`, `laplace.c`. Note the order `celt_lpc` → `pitch` → `vq` → `bands` is a real topological order, not an approximation. |
| **L5. CELT codec** | `celt/celt.c`, `celt/celt_encoder.c`, `celt/celt_decoder.c` | L4. `celt.c` (comb filter, `resampling_factor`, `init_caps`) has no cross-file calls but needs the `opus_val16`/`celt_sig` types and the mode struct. `celt_encoder.c` and `celt_decoder.c` call everything below. |
| **L6. SILK DSP primitives** | `silk/bwexpander.c`, `silk/bwexpander_32.c`, `silk/inner_prod_aligned.c`, `silk/lin2log.c`, `silk/log2lin.c`, `silk/sigm_Q15.c`, `silk/sort.c`, `silk/sum_sqr_shift.c`, `silk/interpolate.c`, `silk/LPC_analysis_filter.c`, `silk/LPC_fit.c`, `silk/LPC_inv_pred_gain.c`, `silk/biquad_alt.c`, `silk/ana_filt_bank_1.c`, `silk/resampler*.c` (all 8), `silk/gain_quant.c`, `silk/decode_pitch.c`, `silk/NLSF_unpack.c`, `silk/NLSF_stabilize.c`, `silk/NLSF_VQ.c`, `silk/NLSF_VQ_weights_laroia.c`, `silk/NLSF_del_dec_quant.c`, `silk/NLSF2A.c`, `silk/A2NLSF.c`, `silk/table_LSF_cos.c`, `silk/pitch_est_tables.c`, `silk/tables_*.c`, `silk/fixed/k2a_FIX.c`, `silk/fixed/k2a_Q16_FIX.c`, `silk/fixed/schur_FIX.c`, `silk/fixed/schur64_FIX.c`, `silk/fixed/corrMatrix_FIX.c`, `silk/fixed/vector_ops_FIX.c`, `silk/fixed/warped_autocorrelation_FIX.c`, `silk/fixed/apply_sine_window_FIX.c`, `silk/fixed/autocorr_FIX.c`, `silk/resampler_rom.c`, `silk/tables_*.c` | L1 (SILK codes symbols with `ec_enc_icdf`/`ec_dec_icdf`) and, in **one** place, L4: `silk/fixed/autocorr_FIX.c` calls `_celt_autocorr` from `celt/celt_lpc.c` (the FFT-based autocorrelation is shared, so the port must expose `celt_autocorr` to the SILK layer). |
| **L7. SILK parameter coding** | `silk/encode_indices.c`, `silk/decode_indices.c`, `silk/code_signs.c`, `silk/shell_coder.c`, `silk/encode_pulses.c`, `silk/decode_pulses.c`, `silk/NLSF_encode.c`, `silk/NLSF_decode.c`, `silk/process_NLSFs.c`, `silk/decode_parameters.c`, `silk/quant_LTP_gains.c`, `silk/stereo_encode_pred.c`, `silk/stereo_decode_pred.c`, `silk/stereo_quant_pred.c`, `silk/stereo_find_predictor.c`, `silk/stereo_LR_to_MS.c`, `silk/stereo_MS_to_LR.c`, `silk/HP_variable_cutoff.c`, `silk/LP_variable_cutoff.c`, `silk/VQ_WMat_EC.c` | L6 + L1. E.g. `decode_indices.c` → `ec_dec_icdf` (19 sites) + `NLSF_unpack.c`; `encode_pulses.c` → `ec_enc_icdf` (7) + `code_signs.c` + `shell_coder.c`. |
| **L8. SILK frame core** | `silk/NSQ.c`, `silk/NSQ_del_dec.c`, `silk/CNG.c`, `silk/PLC.c`, `silk/VAD.c`, `silk/decode_core.c`, `silk/decode_frame.c`, `silk/decoder_set_fs.c`, `silk/check_control_input.c`, `silk/control_audio_bandwidth.c`, `silk/control_SNR.c`, `silk/control_codec.c`, `silk/fixed/find_pitch_lags_FIX.c`, `silk/fixed/find_LPC_FIX.c`, `silk/fixed/find_LTP_FIX.c`, `silk/fixed/find_pred_coefs_FIX.c`, `silk/fixed/noise_shape_analysis_FIX.c`, `silk/fixed/process_gains_FIX.c`, `silk/fixed/LTP_analysis_filter_FIX.c`, `silk/fixed/LTP_scale_ctrl_FIX.c`, `silk/fixed/residual_energy_FIX.c`, `silk/fixed/residual_energy16_FIX.c`, `silk/fixed/regularize_correlations_FIX.c`, `silk/fixed/encode_frame_FIX.c`, `silk/fixed/pitch_analysis_core_FIX.c` | L7 + L6. `decode_frame.c` → `CNG.c`, `PLC.c`, `decode_core.c`, `decode_indices.c`, `decode_parameters.c`, `decode_pulses.c`. `encode_frame_FIX.c` orchestrates `LP_variable_cutoff`, `find_pitch_lags_FIX`, `find_pred_coefs_FIX`, `noise_shape_analysis_FIX`, `process_gains_FIX`, `encode_indices`, `encode_pulses`, `gain_quant`. |
| **L9. SILK API** | `silk/structs.h`, `silk/fixed/structs_FIX.h`, `silk/resampler_structs.h`, `silk/init_encoder.c`, `silk/init_decoder.c`, `silk/enc_API.c`, `silk/dec_API.c`, `silk/main.h`, `silk/fixed/main_FIX.h`, `silk/API.h`, `silk/PLC.h`, `silk/NSQ.h` | L8 + L7 + L6 + L1. Owns the top-level `silk_encoder`/`silk_decoder` super-structs and the state machine (bandwidth switching, LBRR, DTX, VAD, stereo, resampler). |
| **L10. Opus layer** | `src/opus_private.h`, `src/opus.c`, `src/opus_decoder.c`, `src/opus_encoder.c`, `src/repacketizer.c`, `src/extensions.c` | L9 + L5 + L1. TOC/packet framing, mode switching, hybrid framing, and the CELT/SILK rate-coder hand-off. Independent of `silk/fixed/*` and `silk/float/*` directly (it uses the `silk_Encode`/`silk_Decode` API and `silk_EncControlStruct`). |
| **L11 (optional, out of core scope)** | `src/opus_multistream*.c`, `src/opus_projection_*.c`, `src/mapping_matrix.c`, `src/analysis.c`, `src/mlp*.c` | L10. Wrappers and the float analysis layer. Port last, or never (the multistream layer is needed for `OPUS_APPLICATION_RESTRICTED_LOWDELAY` multistream tests but not for mono/stereo bit-exactness). |

### 3.2 Recommended porting order (numbered, file by file)

This is the order in which a Rust port can be compiled and unit-tested
incrementally. Each step is testable against the C reference before the next
step needs it.

**Phase A — foundation (no codec logic yet)**

1. `include/opus_types.h`, `include/opus_defines.h` → `oprs::types` (fixed-width ints, `OPUS_OK`/error codes, `OPUS_*` constants).
2. `celt/arch.h` (fixed-point branch only) → `oprs::arch`: `opus_val16 = i16`, `opus_val32 = i32`, `opus_val64 = i64`, `celt_sig/celt_norm/celt_ener/celt_glog`, `opus_res` per `ENABLE_RES24`, `SAT16`, `QCONST16/32`, `Q15ONE/Q31ONE`, `SIG_SHIFT=12`, `NORM_SHIFT=24`, `DB_SHIFT=24`, `SIG_SAT`, `EPSILON/VERY_SMALL/VERY_LARGE16`, `celt_assert`/`celt_sig_assert`, `OPUS_FAST_INT64`.
3. `celt/fixed_generic.h` → `oprs::fixed`: every macro from §7.1 — the single most important file to port verbatim; unit-test each macro against the C one over the full edge-value set (`MAX16`, `MIN16`, sign combinations, shift 0/15/16/31).
4. `celt/float_cast.h`, `celt/os_support.h`, `celt/stack_alloc.h` → trivial glue (`FLOAT2INT16`/`FLOAT2INT24`, allocator, scratch buffers as Rust slices).
5. `silk/typedef.h`, `silk/macros.h`, `silk/Inlines.h`, `silk/define.h` → `oprs::silk::macros`: `silk_MUL/MLA/SMULWB/SMLAWB/SMULWW/SMLAWW/SMMUL/RSHIFT_ROUND/ADD_SAT32/LIMIT/CLZ32`.
6. `silk/SigProc_FIX.h`'s inline helpers → `oprs::silk::sigproc` (`silk_SQRT_APPROX`, `silk_DIV32_varQ`, `silk_INVERSE32_varQ`, `silk_DIV32_16`, `silk_lin2log`-adjacent).

**Phase B — entropy coder (do this before anything that produces bits)**

7. `celt/entcode.c` + `celt/entcode.h` (`ec_ctx`, `BITRES`, `EC_SYM_BITS`, `EC_CODE_BITS`, `EC_WINDOW_SIZE`, `EC_UINT_BITS`, `ec_ilog`, `ec_tell_frac`, `celt_udiv`, `celt_sudiv`) → `oprs::ec`.
8. `celt/entenc.c` + `celt/mfrngcod.h` → `oprs::ec::enc`.
9. `celt/entdec.c` → `oprs::ec::dec`.
10. `celt/ecintrin.h` (`EC_CLZ`) and a Rust `leading_zeros()` shim — verify `ec_ilog` matches for all 2^32 inputs reachable (test the 0x80000000..0xFFFFFFFF and 0 edge cases explicitly).
11. `celt/laplace.c` (+ `celt/laplace.h`) → `oprs::laplace`.

**Phase C — DSP primitives and tables**

12. `celt/mathops.c` + `celt/mathops.h` → `oprs::mathops`. Port `isqrt32`, `frac_div32(_q29)`, `celt_rsqrt_norm(32)`, `celt_sqrt(32)`, `celt_cos_norm(32)`, `celt_rcp(_norm16/_norm32)`, `celt_exp2`, `celt_log2`, `celt_atan2p`, `celt_exp2_frac`.
13. `celt/cwrs.c` + `celt/cwrs.h` → `oprs::cwrs` (`V(N,K)` via `CeltPulseCache`, `log2_frac`, `get_required_bits`, `encode_pulses`, `decode_pulses`).
14. `celt/static_modes_fixed.h` → generated Rust tables (`oprs::modes::tables`). Do not hand-transcribe; write a generator that parses the C header (see §6.5).
15. `celt/kiss_fft.c` + `celt/_kiss_fft_guts.h` + `celt/kiss_fft.h` → `oprs::kiss_fft` (`kf_bfly2/3/4/5`, `kf_work`, `kf_factor`, `compute_bitrev_table`, `opus_fft_alloc*`, `opus_fft_c`, `opus_ifft_c`).
16. `celt/mdct.c` + `celt/mdct.h` → `oprs::mdct` (`clt_mdct_init/clear/forward_c/backward_c`).
17. `celt/rate.c` + `celt/rate.h` → `oprs::rate` (`compute_pulse_cache`, `interp_bits2pulses`, `clt_compute_allocation`, `clt_compute_extra_allocation`).

**Phase D — CELT**

18. `celt/modes.c` + `celt/modes.h` → `oprs::modes` (`OpusCustomMode`, `opus_custom_mode_create`, `compute_qext_mode`).
19. `celt/celt_lpc.c` + `celt/celt_lpc.h` → `oprs::celt_lpc` (`_celt_lpc`, `celt_fir_c`, `celt_iir`, `_celt_autocorr`).
20. `celt/pitch.c` + `celt/pitch.h` → `oprs::pitch` (`celt_pitch_xcorr_c`, `xcorr_kernel`, `pitch_downsample`, `pitch_search`, `remove_doubling`).
21. `celt/vq.c` + `celt/vq.h` → `oprs::vq` (`exp_rotation`, `op_pvq_search_c`, `alg_quant`, `alg_unquant`, `renormalise_vector`, `stereo_itheta`, `cubic_quant/unquant` under `ENABLE_QEXT`).
22. `celt/quant_bands.c` + `celt/quant_bands.h` → `oprs::quant_bands`.
23. `celt/bands.c` + `celt/bands.h` → `oprs::bands` (`compute_band_energies`, `normalise_bands`, `denormalise_bands`, `anti_collapse`, `haar1`, `quant_all_bands`, spreading/hysteresis).
24. `celt/celt.c` → `oprs::celt` (`resampling_factor`, `comb_filter`, `init_caps`).
25. `celt/celt_decoder.c` + `celt/celt.h` → `oprs::celt::decoder` (`OpusCustomDecoder`, `celt_decoder_init`, `deemphasis`, `celt_synthesis`, `celt_decode_with_ec`). **Port the decoder before the encoder**: it is the reference for the bitstream, it is needed by the encoder's PLC/DRED path, and decode-only correctness is the strongest single test.
26. `celt/celt_encoder.c` → `oprs::celt::encoder` (`OpusCustomEncoder`, `celt_preemphasis`, `celt_encode_with_ec`, all CTLs, `normalize_tone_input`, `acos_approx`).

**Phase E — SILK DSP primitives**

27. `silk/bwexpander.c`, `silk/bwexpander_32.c` → `oprs::silk::bwexpander`.
28. `silk/lin2log.c`, `silk/log2lin.c`, `silk/sigm_Q15.c` → `oprs::silk::log`.
29. `silk/sort.c`, `silk/sum_sqr_shift.c`, `silk/inner_prod_aligned.c`, `silk/interpolate.c` → `oprs::silk::util`.
30. `silk/LPC_analysis_filter.c` → `oprs::silk::lpc`.
31. `silk/biquad_alt.c`, `silk/ana_filt_bank_1.c` → `oprs::silk::biquad`.
32. `silk/resampler_rom.c`, `silk/resampler_private_AR2.c`, `silk/resampler_private_up2_HQ.c`, `silk/resampler_private_IIR_FIR.c`, `silk/resampler_private_down_FIR.c`, `silk/resampler_down2.c`, `silk/resampler_down2_3.c`, `silk/resampler.c` → `oprs::silk::resampler` (in that order; `resampler.c` is the dispatcher and depends on all of them).
33. `silk/table_LSF_cos.c`, `silk/tables_gain.c`, `silk/tables_LTP.c`, `silk/tables_NLSF_CB_NB_MB.c`, `silk/tables_NLSF_CB_WB.c`, `silk/tables_other.c`, `silk/tables_pitch_lag.c`, `silk/tables_pulses_per_block.c`, `silk/pitch_est_tables.c` → `oprs::silk::tables`. These are mechanical (§6.3).
34. `silk/NLSF_unpack.c`, `silk/NLSF_stabilize.c`, `silk/NLSF_VQ.c`, `silk/NLSF_VQ_weights_laroia.c`, `silk/NLSF_del_dec_quant.c`, `silk/NLSF2A.c`, `silk/A2NLSF.c`, `silk/LPC_fit.c`, `silk/LPC_inv_pred_gain.c` → `oprs::silk::nlsf`.
35. `silk/gain_quant.c`, `silk/decode_pitch.c`, `silk/shell_coder.c`, `silk/code_signs.c` → `oprs::silk::quant`.
36. `silk/fixed/vector_ops_FIX.c`, `silk/fixed/k2a_FIX.c`, `silk/fixed/k2a_Q16_FIX.c`, `silk/fixed/schur_FIX.c`, `silk/fixed/schur64_FIX.c`, `silk/fixed/corrMatrix_FIX.c`, `silk/fixed/apply_sine_window_FIX.c`, `silk/fixed/autocorr_FIX.c`, `silk/fixed/warped_autocorrelation_FIX.c` → `oprs::silk::fixed::analysis` lemmas.
37. `silk/stereo_find_predictor.c`, `silk/stereo_quant_pred.c`, `silk/stereo_encode_pred.c`, `silk/stereo_decode_pred.c`, `silk/stereo_LR_to_MS.c`, `silk/stereo_MS_to_LR.c` → `oprs::silk::stereo`.

**Phase F — SILK coding and state machines**

38. `silk/encode_indices.c`, `silk/decode_indices.c`, `silk/encode_pulses.c`, `silk/decode_pulses.c`, `silk/decode_parameters.c` → `oprs::silk::indices`.
39. `silk/NLSF_encode.c`, `silk/NLSF_decode.c`, `silk/process_NLSFs.c`, `silk/quant_LTP_gains.c`, `silk/VQ_WMat_EC.c` → `oprs::silk::nlsf_enc`.
40. `silk/HP_variable_cutoff.c`, `silk/LP_variable_cutoff.c` → `oprs::silk::cutoff`.
41. `silk/structs.h`, `silk/resampler_structs.h`, `silk/fixed/structs_FIX.h` → `oprs::silk::state` (see §5).
42. `silk/NSQ.c`, `silk/NSQ_del_dec.c` → `oprs::silk::nsq`.
43. `silk/PLC.c`, `silk/CNG.c` → `oprs::silk::plc`, `oprs::silk::cng`.
44. `silk/VAD.c` → `oprs::silk::vad`.
45. `silk/decode_core.c`, `silk/decode_frame.c`, `silk/decoder_set_fs.c` → `oprs::silk::decoder`.
46. `silk/fixed/find_pitch_lags_FIX.c`, `silk/fixed/pitch_analysis_core_FIX.c`, `silk/fixed/find_LPC_FIX.c`, `silk/fixed/find_LTP_FIX.c`, `silk/fixed/noise_shape_analysis_FIX.c`, `silk/fixed/process_gains_FIX.c`, `silk/fixed/LTP_analysis_filter_FIX.c`, `silk/fixed/LTP_scale_ctrl_FIX.c`, `silk/fixed/residual_energy_FIX.c`, `silk/fixed/residual_energy16_FIX.c`, `silk/fixed/regularize_correlations_FIX.c`, `silk/fixed/find_pred_coefs_FIX.c`, `silk/fixed/encode_frame_FIX.c` → `oprs::silk::fixed::encoder`.
47. `silk/check_control_input.c`, `silk/control_audio_bandwidth.c`, `silk/control_SNR.c`, `silk/control_codec.c` → `oprs::silk::control`.
48. `silk/init_encoder.c`, `silk/init_decoder.c` → `oprs::silk::init`.
49. `silk/enc_API.c`, `silk/dec_API.c` → `oprs::silk::api` (`silk_Encode`, `silk_Decode`, `silk_InitEncoder`, `silk_InitDecoder`, `silk_ResetDecoder`, `silk_get_TOC`).

**Phase G — opus layer**

50. `src/opus.c` → `oprs::packet` (TOC, frame parsing, `opus_packet_parse_impl`).
51. `src/opus_decoder.c` → `oprs::decoder` (`OpusDecoder`, `opus_decode_native`, PLC/soft-decode glue).
52. `src/opus_encoder.c` → `oprs::encoder` (`OpusEncoder`, `opus_encode_native`, mode/rate decision, DTX/FEC).
53. `src/repacketizer.c` + `src/opus_private.h` → `oprs::repacketizer`.
54. `src/extensions.c` → `oprs::extensions` (only if the extension mechanism is needed; otherwise stub it, but note that `src/opus_decoder.c`/`opus_encoder.c` call into it under `#ifdef` and the packet padding logic interacts with it).
55. `src/opus_multistream.c`, `src/opus_multistream_encoder.c`, `src/opus_multistream_decoder.c`, `src/mapping_matrix.c`, `src/opus_projection_*.c` → wrappers (optional).
56. `src/analysis.c`, `src/mlp.c`, `src/mlp_data.c` → only if the float API's analysis-driven CELT decisions are needed. **Note:** with `DISABLE_FLOAT_API` off, the encoder consults `AnalysisInfo` to pick CELT parameters, which *changes the bitstream*. Decide explicitly whether the port reproduces this. For strict bit-exactness against an autotools float-API build this is mandatory.

### 3.3 Cycles and non-obvious cross-layer edges

Three edges break the clean layering and must be handled knowingly:

1. **`celt/celt_encoder.c` → `celt/celt_decoder.c`.** The encoder calls
   `celt_synthesis()` and `deemphasis()` (`celt/celt_encoder.c:2743` and `:2759`)
   — the decoder-side synthesis is reused by the encoder for its PLC/DRED
   synthesis path. In Rust, put `celt_synthesis`/`deemphasis` in a module that
   both can use (`celt::synth`), not inside `celt::decoder`.
2. **SILK → CELT.** `silk/fixed/autocorr_FIX.c` calls `_celt_autocorr()`
   (`celt/celt_lpc.c`), the FFT-based autocorrelation estimator. So SILK is *not*
   independent of CELT: the port must expose `celt_autocorr` to `silk::fixed`.
   (`silk/fixed/autocorr_FIX.c` is fixed-point only; the float path uses
   `silk/float/autocorrelation_FLP.c` instead.)
3. **The opus layer → both codecs through a shared `ec_enc`/`ec_dec`.** The
   hybrid mode calls `silk_Encode` and `celt_encode_with_ec` on the *same*
   entropy coder in sequence, and `silk_Decode`/`celt_decode_with_ec` likewise
   on decode. This means the entropy coder state cannot be owned exclusively by
   either layer — in Rust it must be passed as `&mut EcEnc`/`&mut EcDec` (as the
   C API does).

Two apparent cycles in the mechanical graph are **false positives** caused by
identical `static` helper names in the `fixed/` and `float/` variants
(`warped_gain` in `noise_shape_analysis_{FIX,FLP}.c`,
`silk_P_Ana_calc_corr_st3`/`silk_P_Ana_calc_energy_st3` in
`pitch_analysis_core_{FIX,FLP}.c`). They are file-local `static` functions and
do not create a real dependency; Appendix A is still useful because it makes the
collisions visible.

### 3.4 Why this order (justification summary)

* **Entropy coder first** because *every* bit in the output is produced by a
  sequence of `ec_*` calls. If `ec_enc_normalize`'s carry logic or
  `ec_dec_normalize`'s renormalisation loop differs by one iteration, every
  downstream test fails in a way that cannot be localised. Once the EC passes a
  byte-for-byte differential test against C on random symbol streams, all later
  failures are attributable to the layer being added.
* **DSP primitives second** because they are leaf functions with no codec state;
  they can be exhaustively differentially tested over their whole input domain
  (`isqrt32` over a sample of 2^32, `celt_sqrt` over all `i32`, every
  `MULT16_16_*` over all `i16 × i16` for the small ones).
* **Mode/table layer third** because the CELT mode tables determine band
  boundaries, allocation vectors, MDCT twiddles and pulse-cache sizes. If the
  generator that emits the Rust tables makes an off-by-one error, everything
  above is wrong in a way that looks like a DSP bug.
* **CELT decoder before CELT encoder** because the decoder is a pure function of
  the bitstream + state, so it can be validated against the reference decoder on
  the official Opus test vectors without needing a bit-exact encoder.
* **SILK after CELT** because (a) SILK's only CELT dependency is
  `_celt_autocorr`, and (b) SILK's state machine (bandwidth switching, resampler
  rate changes) is the most stateful part of the codec and is best attempted once
  the entropy coder and the shared FFT are known-good.
* **`src/` last** because it is pure framing/policy on top of `silk_Encode`,
  `silk_Decode`, `celt_encode_with_ec`, `celt_decode_with_ec` — the API surface
  listed in §4.

---

## 4. Public / internal API surface needed by the opus layer

Every symbol below is a `celt/` or `silk/` function that the opus layer (`src/opus.c`, `src/opus_encoder.c`, `src/opus_decoder.c`, `src/repacketizer.c`, `src/extensions.c`) calls, with the signature copied **verbatim** from its declaring header (or, for SILK functions declared only in `silk/main.h`-included files, from the header that declares it). `Call sites` counts textual occurrences of `name(` in each opus-layer file. SIMD/RTCD-only declarations are excluded.

Types used in the signatures are all defined in §5.

> A `—` in the *Call sites* column means the symbol is **not** textually called by any of the five opus-layer files (it is used internally by `celt/` or `silk/`, or it is part of the same public surface). It is included because the Rust port must still implement it for the layer below to compile. §4.9 shows the real call chains.

### 4.0 Direct call summary (what the opus layer actually touches)

Every `celt/`+`silk/` symbol textually called from the five opus-layer files, with total call count. This is the exact set the Rust port must expose across the `celt`/`silk` module boundary.

| Symbol | Calls | Symbol | Calls | Symbol | Calls |
|---|---:|---|---:|---|---:|
| `ec_tell` | 10 | `bitrate_to_bits` | 9 | `opus_free` | 9 |
| `bits_to_bitrate` | 6 | `celt_encode_with_ec` | 5 | `celt_sqrt` | 5 |
| `opus_alloc` | 5 | `celt_ilog2` | 4 | `celt_isnan` | 4 |
| `FLOAT2SIG` | 3 | `celt_decode_with_ec` | 3 | `opus_select_arch` | 3 |
| `silk_InitEncoder` | 3 | `silk_lin2log` | 3 | `celt_exp2` | 2 |
| `celt_maxabs_res` | 2 | `ec_dec_bit_logp` | 2 | `ec_enc_bit_logp` | 2 |
| `ec_enc_shrink` | 2 | `silk_Encode` | 2 | `silk_Get_Decoder_Size` | 2 |
| `silk_ResetDecoder` | 2 | `celt_decode_with_ec_dred` | 1 | `celt_decoder_get_size` | 1 |
| `celt_decoder_init` | 1 | `celt_encoder_get_size` | 1 | `celt_encoder_init` | 1 |
| `ec_dec_init` | 1 | `ec_dec_uint` | 1 | `ec_enc_done` | 1 |
| `ec_enc_init` | 1 | `ec_enc_uint` | 1 | `frac_div32` | 1 |
| `silk_Decode` | 1 | `silk_Get_Encoder_Size` | 1 | `silk_InitDecoder` | 1 |
| `silk_LoadOSCEModels` | 1 | `silk_biquad_alt_stride1` | 1 | `silk_log2lin` | 1 |

> Some entries above are macros/`static inline` helpers (`celt_encoder_ctl`, `celt_decoder_ctl`, `celt_udiv`, `silk_SMULWB`, ...); those are listed in §4.8.

### 4.1 The entropy coder driven directly by the opus layer

| Function | Verbatim C declaration | Declared in | Call sites | Kind |
|---|---|---|---|---|
| `ec_enc_init` | `void ec_enc_init(ec_enc *_this,unsigned char *_buf,opus_uint32 _size)` | `celt/entenc.h:36` | `src/opus_encoder.c`×1 | prototype |
| `ec_enc_done` | `void ec_enc_done(ec_enc *_this)` | `celt/entenc.h:117` | `src/opus_encoder.c`×1 | prototype |
| `ec_enc_shrink` | `void ec_enc_shrink(ec_enc *_this,opus_uint32 _size)` | `celt/entenc.h:112` | `src/opus_encoder.c`×2 | prototype |
| `ec_enc_uint` | `void ec_enc_uint(ec_enc *_this,opus_uint32 _fl,opus_uint32 _ft)` | `celt/entenc.h:80` | `src/opus_encoder.c`×1 | prototype |
| `ec_enc_bits` | `void ec_enc_bits(ec_enc *_this,opus_uint32 _fl,unsigned _ftb)` | `celt/entenc.h:86` | — | prototype |
| `ec_enc_icdf` | `void ec_enc_icdf(ec_enc *_this,int _s,const unsigned char *_icdf,unsigned _ftb)` | `celt/entenc.h:65` | — | prototype |
| `ec_enc_icdf16` | `void ec_enc_icdf16(ec_enc *_this,int _s,const opus_uint16 *_icdf,unsigned _ftb)` | `celt/entenc.h:74` | — | prototype |
| `ec_enc_bit_logp` | `void ec_enc_bit_logp(ec_enc *_this,int _val,unsigned _logp)` | `celt/entenc.h:56` | `src/opus_encoder.c`×2 | prototype |
| `ec_enc_patch_initial_bits` | `void ec_enc_patch_initial_bits(ec_enc *_this,unsigned _val,unsigned _nbits)` | `celt/entenc.h:102` | — | prototype |
| `ec_tell` | `static OPUS_INLINE int ec_tell(ec_ctx *_this) ... }` | `celt/entcode.h:111` | `src/opus_encoder.c`×7, `src/opus_decoder.c`×3 | static inline |
| `ec_dec_init` | `void ec_dec_init(ec_dec *_this,unsigned char *_buf,opus_uint32 _storage)` | `celt/entdec.h:36` | `src/opus_decoder.c`×1 | prototype |
| `ec_dec_uint` | `opus_uint32 ec_dec_uint(ec_dec *_this,opus_uint32 _ft)` | `celt/entdec.h:100` | `src/opus_decoder.c`×1 | prototype |
| `ec_dec_bits` | `opus_uint32 ec_dec_bits(ec_dec *_this,unsigned _ftb)` | `celt/entdec.h:108` | — | prototype |
| `ec_dec_icdf` | `int ec_dec_icdf(ec_dec *_this,const unsigned char *_icdf,unsigned _ftb)` | `celt/entdec.h:82` | — | prototype |
| `ec_dec_icdf16` | `int ec_dec_icdf16(ec_dec *_this,const opus_uint16 *_icdf,unsigned _ftb)` | `celt/entdec.h:92` | — | prototype |
| `ec_dec_bit_logp` | `int ec_dec_bit_logp(ec_dec *_this,unsigned _logp)` | `celt/entdec.h:72` | `src/opus_decoder.c`×2 | prototype |
| `ec_tell_frac` | `opus_uint32 ec_tell_frac(ec_ctx *_this)` | `celt/entcode.h:121` | — | prototype |
| `ec_laplace_encode` | `void ec_laplace_encode(ec_enc *enc, int *value, unsigned fs, int decay)` | `celt/laplace.h:42` | — | prototype |
| `ec_laplace_decode` | `int ec_laplace_decode(ec_dec *dec, unsigned fs, int decay)` | `celt/laplace.h:51` | — | prototype |

### 4.2 CELT encoder entry points

| Function | Verbatim C declaration | Declared in | Call sites | Kind |
|---|---|---|---|---|
| `celt_encoder_get_size` | `int celt_encoder_get_size(int channels)` | `celt/celt.h:157` | `src/opus_encoder.c`×1 | prototype |
| `celt_encoder_init` | `int celt_encoder_init(CELTEncoder *st, opus_int32 sampling_rate, int channels, int arch)` | `celt/celt.h:161` | `src/opus_encoder.c`×1 | prototype |
| `opus_custom_encoder_init` | `OPUS_CUSTOM_EXPORT int opus_custom_encoder_init( OpusCustomEncoder *st, const OpusCustomMode *mode, int channels )` | `include/opus_custom.h:157` | — | prototype |
| `opus_custom_encoder_create` | `OPUS_CUSTOM_EXPORT OPUS_WARN_UNUSED_RESULT OpusCustomEncoder *opus_custom_encoder_create( const OpusCustomMode *mode, int channels, int *error )` | `include/opus_custom.h:175` | — | prototype |
| `opus_custom_encoder_destroy` | `OPUS_CUSTOM_EXPORT void opus_custom_encoder_destroy(OpusCustomEncoder *st)` | `include/opus_custom.h:185` | — | prototype |
| `opus_custom_encoder_get_size` | `OPUS_CUSTOM_EXPORT_STATIC OPUS_WARN_UNUSED_RESULT int opus_custom_encoder_get_size( const OpusCustomMode *mode, int channels )` | `include/opus_custom.h:139` | — | prototype |
| `celt_encode_with_ec` | `int celt_encode_with_ec(OpusCustomEncoder * OPUS_RESTRICT st, const opus_res * pcm, int frame_size, unsigned char *compressed, int nbCompressedBytes, ec_enc *enc)` | `celt/celt.h:159` | `src/opus_encoder.c`×5 | prototype |
| `opus_custom_encode` | `OPUS_CUSTOM_EXPORT OPUS_WARN_UNUSED_RESULT int opus_custom_encode( OpusCustomEncoder *st, const opus_int16 *pcm, int frame_size, unsigned char *compressed, int maxCompressedBytes )` | `include/opus_custom.h:225` | — | prototype |
| `opus_custom_encode_float` | `OPUS_CUSTOM_EXPORT OPUS_WARN_UNUSED_RESULT int opus_custom_encode_float( OpusCustomEncoder *st, const float *pcm, int frame_size, unsigned char *compressed, int maxCompressedBytes )` | `include/opus_custom.h:204` | — | prototype |
| `opus_custom_encode24` | `OPUS_CUSTOM_EXPORT OPUS_WARN_UNUSED_RESULT int opus_custom_encode24( OpusCustomEncoder *st, const opus_int32 *pcm, int frame_size, unsigned char *compressed, int maxCompressedBytes )` | `include/opus_custom.h:246` | — | prototype |
| `opus_custom_encoder_ctl` | `OPUS_CUSTOM_EXPORT int opus_custom_encoder_ctl(OpusCustomEncoder * OPUS_RESTRICT st, int request, ...)` | `include/opus_custom.h:260` | — | prototype |
| `celt_encoder_ctl` | `#define celt_encoder_ctl opus_custom_encoder_ctl` | `celt/celt.h:184` | — | prototype |

### 4.3 CELT decoder entry points

| Function | Verbatim C declaration | Declared in | Call sites | Kind |
|---|---|---|---|---|
| `celt_decoder_get_size` | `int celt_decoder_get_size(int channels)` | `celt/celt.h:168` | `src/opus_decoder.c`×1 | prototype |
| `celt_decoder_init` | `int celt_decoder_init(CELTDecoder *st, opus_int32 sampling_rate, int channels)` | `celt/celt.h:171` | `src/opus_decoder.c`×1 | prototype |
| `opus_custom_decoder_create` | `OPUS_CUSTOM_EXPORT OPUS_WARN_UNUSED_RESULT OpusCustomDecoder *opus_custom_decoder_create( const OpusCustomMode *mode, int channels, int *error )` | `include/opus_custom.h:305` | — | prototype |
| `opus_custom_decoder_destroy` | `OPUS_CUSTOM_EXPORT void opus_custom_decoder_destroy(OpusCustomDecoder *st)` | `include/opus_custom.h:314` | — | prototype |
| `celt_decode_with_ec` | `int celt_decode_with_ec(OpusCustomDecoder * OPUS_RESTRICT st, const unsigned char *data, int len, opus_res * OPUS_RESTRICT pcm, int frame_size, ec_dec *dec, int accum)` | `celt/celt.h:181` | `src/opus_decoder.c`×3 | prototype |
| `celt_decode_with_ec_dred` | `int celt_decode_with_ec_dred(CELTDecoder * OPUS_RESTRICT st, const unsigned char *data, int len, opus_res * OPUS_RESTRICT pcm, int frame_size, ec_dec *dec, int accum #ifdef ENABLE_DEEP_PLC ,LPCNetPLCState *lpcnet #endif ARG_QEXT(const unsigned char *qext_payload) ARG_QEXT(int qext_payload_len) )` | `celt/celt.h:173` | `src/opus_decoder.c`×1 | prototype |
| `opus_custom_decode` | `OPUS_CUSTOM_EXPORT OPUS_WARN_UNUSED_RESULT int opus_custom_decode( OpusCustomDecoder *st, const unsigned char *data, int len, opus_int16 *pcm, int frame_size )` | `include/opus_custom.h:342` | — | prototype |
| `opus_custom_decode_float` | `OPUS_CUSTOM_EXPORT OPUS_WARN_UNUSED_RESULT int opus_custom_decode_float( OpusCustomDecoder *st, const unsigned char *data, int len, float *pcm, int frame_size )` | `include/opus_custom.h:325` | — | prototype |
| `opus_custom_decode24` | `OPUS_CUSTOM_EXPORT OPUS_WARN_UNUSED_RESULT int opus_custom_decode24( OpusCustomDecoder *st, const unsigned char *data, int len, opus_int32 *pcm, int frame_size )` | `include/opus_custom.h:359` | — | prototype |
| `opus_custom_decoder_ctl` | `OPUS_CUSTOM_EXPORT int opus_custom_decoder_ctl(OpusCustomDecoder * OPUS_RESTRICT st, int request, ...)` | `include/opus_custom.h:373` | — | prototype |
| `celt_decoder_ctl` | `#define celt_decoder_ctl opus_custom_decoder_ctl` | `celt/celt.h:185` | — | prototype |

### 4.4 Mode and table access

| Function | Verbatim C declaration | Declared in | Call sites | Kind |
|---|---|---|---|---|
| `opus_custom_mode_create` | `OPUS_CUSTOM_EXPORT OPUS_WARN_UNUSED_RESULT OpusCustomMode *opus_custom_mode_create(opus_int32 Fs, int frame_size, int *error)` | `include/opus_custom.h:122` | — | prototype |
| `opus_custom_mode_destroy` | `OPUS_CUSTOM_EXPORT void opus_custom_mode_destroy(OpusCustomMode *mode)` | `include/opus_custom.h:128` | — | prototype |
| `compute_pulse_cache` | `void compute_pulse_cache(CELTMode *m, int LM)` | `celt/rate.h:46` | — | prototype |
| `clt_compute_allocation` | `int clt_compute_allocation(const CELTMode *m, int start, int end, const int *offsets, const int *cap, int alloc_trim, int *intensity, int *dual_stereo, opus_int32 total, opus_int32 *balance, int *pulses, int *ebits, int *fine_priority, int C, int LM, ec_ctx *ec, int encode, int prev, int signalBandwidth)` | `celt/rate.h:98` | — | prototype |
| `init_caps` | `void init_caps(const CELTMode *m,int *cap,int LM,int C)` | `celt/celt.h:257` | — | prototype |

### 4.5 SILK encoder API

| Function | Verbatim C declaration | Declared in | Call sites | Kind |
|---|---|---|---|---|
| `silk_Get_Encoder_Size` | `opus_int silk_Get_Encoder_Size( /* O Returns error code */ opus_int *encSizeBytes, /* O Number of bytes in SILK encoder state */ opus_int channels /* I Number of channels */ )` | `silk/API.h:57` | `src/opus_encoder.c`×1 | prototype |
| `silk_InitEncoder` | `opus_int silk_InitEncoder( /* O Returns error code */ void *encState, /* I/O State */ int channels, /* I Number of channels */ int arch, /* I Run-time architecture */ silk_EncControlStruct *encStatus /* O Encoder Status */ )` | `silk/API.h:65` | `src/opus_encoder.c`×3 | prototype |
| `silk_Encode` | `opus_int silk_Encode( /* O Returns error code */ void *encState, /* I/O State */ silk_EncControlStruct *encControl, /* I Control status */ const opus_res *samplesIn, /* I Speech sample input vector */ opus_int nSamplesIn, /* I Number of samples in input vector */ ec_enc *psRangeEnc, /* I/O Compressor data structure */ opus_int32 *nBytesOut, /* I/O Number of bytes in payload (input: Max bytes) */ const opus_int prefillFlag, /* I Flag to indicate prefilling buffers no coding */ int activity /* I Decision of Opus voice activity detector */ )` | `silk/API.h:77` | `src/opus_encoder.c`×2 | prototype |
| `silk_QueryEncoder` | `static opus_int silk_QueryEncoder( /* O Returns error code */ const void *encState, /* I State */ silk_EncControlStruct *encStatus /* O Encoder Status */ )` | `silk/enc_API.c:51` | — | definition |
| `silk_control_encoder` | `opus_int silk_control_encoder( silk_encoder_state_Fxx *psEnc, /* I/O Pointer to Silk encoder state */ silk_EncControlStruct *encControl, /* I Control structure */ const opus_int allow_bw_switch, /* I Flag to allow switching audio bandwidth */ const opus_int channelNb, /* I Channel number */ const opus_int force_fs_kHz )` | `silk/fixed/main_FIX.h:84` | — | prototype |
| `silk_control_audio_bandwidth` | `opus_int silk_control_audio_bandwidth( silk_encoder_state *psEncC, /* I/O Pointer to Silk encoder state */ silk_EncControlStruct *encControl /* I Control structure */ )` | `silk/main.h:140` | — | prototype |
| `silk_control_SNR` | `opus_int silk_control_SNR( silk_encoder_state *psEncC, /* I/O Pointer to Silk encoder state */ opus_int32 TargetRate_bps /* I Target max bitrate (bps) */ )` | `silk/main.h:146` | — | prototype |
| `silk_init_encoder` | `opus_int silk_init_encoder( silk_encoder_state_Fxx *psEnc, /* I/O Pointer to Silk FIX encoder state */ int arch /* I Run-time architecture */ )` | `silk/fixed/main_FIX.h:78` | — | prototype |
| `check_control_input` | `opus_int check_control_input( silk_EncControlStruct *encControl /* I Control structure */ )` | `silk/main.h:135` | — | prototype |
| `silk_HP_variable_cutoff` | `void silk_HP_variable_cutoff( silk_encoder_state_Fxx state_Fxx[] /* I/O Encoder states */ )` | `silk/fixed/main_FIX.h:57` | — | prototype |

### 4.6 SILK decoder API

| Function | Verbatim C declaration | Declared in | Call sites | Kind |
|---|---|---|---|---|
| `silk_Get_Decoder_Size` | `opus_int silk_Get_Decoder_Size( /* O Returns error code */ opus_int *decSizeBytes /* O Number of bytes in SILK decoder state */ )` | `silk/API.h:105` | `src/opus_decoder.c`×2 | prototype |
| `silk_InitDecoder` | `opus_int silk_InitDecoder( /* O Returns error code */ void *decState /* I/O State */ )` | `silk/API.h:116` | `src/opus_decoder.c`×1 | prototype |
| `silk_ResetDecoder` | `opus_int silk_ResetDecoder( /* O Returns error code */ void *decState /* I/O State */ )` | `silk/API.h:112` | `src/opus_decoder.c`×2 | prototype |
| `silk_Decode` | `opus_int silk_Decode( /* O Returns error code */ void* decState, /* I/O State */ silk_DecControlStruct* decControl, /* I/O Control Structure */ opus_int lostFlag, /* I 0: no loss, 1 loss, 2 decode fec */ opus_int newPacketFlag, /* I Indicates first decoder call for this packet */ ec_dec *psRangeDec, /* I/O Compressor data structure */ opus_res *samplesOut, /* O Decoded output speech vector */ opus_int32 *nSamplesOut, /* O Number of samples decoded */ #ifdef ENABLE_DEEP_PLC LPCNetPLCState *lpcnet, #endif int arch /* I Run-time architecture */ )` | `silk/API.h:123` | `src/opus_decoder.c`×1 | prototype |
| `silk_get_TOC` | `opus_int silk_get_TOC( const opus_uint8 *payload, /* I Payload data */ const opus_int nBytesIn, /* I Number of input bytes */ const opus_int nFramesPerPayload, /* I Number of SILK frames per payload */ silk_TOC_struct *Silk_TOC /* O Type of content */ )` | `silk/API.h:141` | — | prototype |
| `silk_decoder_set_fs` | `opus_int silk_decoder_set_fs( silk_decoder_state *psDec, /* I/O Decoder state pointer */ opus_int fs_kHz, /* I Sampling frequency (kHz) */ opus_int32 fs_API_Hz /* I API Sampling frequency (Hz) */ )` | `silk/main.h:401` | — | prototype |
| `silk_init_decoder` | `opus_int silk_init_decoder( silk_decoder_state *psDec /* I/O Decoder state pointer */ )` | `silk/main.h:396` | — | prototype |
| `silk_reset_decoder` | `opus_int silk_reset_decoder( silk_decoder_state *psDec /* I/O Decoder state pointer */ )` | `silk/main.h:392` | — | prototype |
| `silk_LoadOSCEModels` | `opus_int silk_LoadOSCEModels( void *decState, /* O I/O State */ const unsigned char *data, /* I pointer to binary blob */ int len /* I length of binary blob data */ )` | `silk/API.h:96` | `src/opus_decoder.c`×1 | prototype |
| `silk_CNG_Reset` | `void silk_CNG_Reset( silk_decoder_state *psDec /* I/O Decoder state */ )` | `silk/main.h:465` | — | prototype |
| `silk_PLC_Reset` | `void silk_PLC_Reset( silk_decoder_state *psDec /* I/O Decoder state */ )` | `silk/PLC.h:43` | — | prototype |

### 4.7 Other celt/silk functions reached from the opus layer

| Function | Verbatim C declaration | Declared in | Call sites | Kind |
|---|---|---|---|---|
| `silk_stereo_find_predictor` | `opus_int32 silk_stereo_find_predictor( /* O Returns predictor in Q13 */ opus_int32 *ratio_Q14, /* O Ratio of residual and mid energies */ const opus_int16 x[], /* I Basis signal */ const opus_int16 y[], /* I Target signal */ opus_int32 mid_res_amp_Q0[], /* I/O Smoothed mid, residual norms */ opus_int length, /* I Number of samples */ opus_int smooth_coef_Q16 /* I Smoothing coefficient */ )` | `silk/main.h:75` | — | prototype |
| `silk_stereo_quant_pred` | `void silk_stereo_quant_pred( opus_int32 pred_Q13[], /* I/O Predictors (out: quantized) */ opus_int8 ix[ 2 ][ 3 ] /* O Quantization indices */ )` | `silk/main.h:85` | — | prototype |
| `silk_LPC_analysis_filter` | `void silk_LPC_analysis_filter( opus_int16 *out, /* O Output signal */ const opus_int16 *in, /* I Input signal */ const opus_int16 *B, /* I MA prediction coefficients, Q12 [order] */ const opus_int32 len, /* I Signal length */ const opus_int32 d, /* I Filter order */ int arch /* I Run-time architecture */ )` | `silk/SigProc_FIX.h:119` | — | prototype |
| `silk_LPC_inverse_pred_gain` | `#define silk_LPC_inverse_pred_gain(A_Q12, order, arch) ((void)(arch), silk_LPC_inverse_pred_gain_c(A_Q12, order))` | `silk/SigProc_FIX.h:163` | — | prototype |
| `silk_NLSF2A` | `void silk_NLSF2A( opus_int16 *a_Q12, /* O monic whitening filter coefficients in Q12, [ d ] */ const opus_int16 *NLSF, /* I normalized line spectral frequencies in Q15, [ d ] */ const opus_int d, /* I filter order (should be even) */ int arch /* I Run-time architecture */ )` | `silk/SigProc_FIX.h:282` | — | prototype |
| `silk_A2NLSF` | `void silk_A2NLSF( opus_int16 *NLSF, /* O Normalized Line Spectral Frequencies in Q15 (0..2^15-1) [d] */ opus_int32 *a_Q16, /* I/O Monic whitening filter coefficients in Q16 [d] */ const opus_int d /* I Filter order (must be even) */ )` | `silk/SigProc_FIX.h:275` | — | prototype |
| `silk_gains_dequant` | `void silk_gains_dequant( opus_int32 gain_Q16[ MAX_NB_SUBFR ], /* O quantized gains */ const opus_int8 ind[ MAX_NB_SUBFR ], /* I gain indices */ opus_int8 *prev_ind, /* I/O last index in previous frame */ const opus_int conditional, /* I first gain is delta coded if 1 */ const opus_int nb_subfr /* I number of subframes */ )` | `silk/main.h:187` | — | prototype |
| `silk_gains_ID` | `opus_int32 silk_gains_ID( /* O returns unique identifier of gains */ const opus_int8 ind[ MAX_NB_SUBFR ], /* I gain indices */ const opus_int nb_subfr /* I number of subframes */ )` | `silk/main.h:196` | — | prototype |
| `silk_lin2log` | `opus_int32 silk_lin2log( const opus_int32 inLin /* I input in linear scale */ )` | `silk/SigProc_FIX.h:172` | `src/opus_encoder.c`×3 | prototype |
| `silk_log2lin` | `opus_int32 silk_log2lin( const opus_int32 inLog_Q7 /* I input on log scale */ )` | `silk/SigProc_FIX.h:183` | `src/opus_encoder.c`×1 | prototype |
| `silk_sum_sqr_shift` | `void silk_sum_sqr_shift( opus_int32 *energy, /* O Energy of x, after shifting to the right */ opus_int *shift, /* O Number of bits right shift applied to energy */ const opus_int16 *x, /* I Input vector */ opus_int len /* I Length of input vector */ )` | `silk/SigProc_FIX.h:189` | — | prototype |
| `silk_inner_prod_aligned` | `opus_int32 silk_inner_prod_aligned( const opus_int16 *const inVec1, /* I input vector 1 */ const opus_int16 *const inVec2, /* I input vector 2 */ const opus_int len, /* I vector lengths */ int arch /* I Run-time architecture */ )` | `silk/SigProc_FIX.h:365` | — | prototype |
| `silk_sigm_Q15` | `opus_int silk_sigm_Q15( opus_int in_Q5 /* I */ )` | `silk/SigProc_FIX.h:177` | — | prototype |
| `silk_resampler_init` | `opus_int silk_resampler_init( silk_resampler_state_struct *S, /* I/O Resampler state */ opus_int32 Fs_Hz_in, /* I Input sampling rate (Hz) */ opus_int32 Fs_Hz_out, /* I Output sampling rate (Hz) */ opus_int forEnc /* I If 1: encoder; if 0: decoder */ )` | `silk/SigProc_FIX.h:58` | — | prototype |
| `silk_resampler` | `opus_int silk_resampler( silk_resampler_state_struct *S, /* I/O Resampler state */ opus_int16 out[], /* O Output signal */ const opus_int16 in[], /* I Input signal */ opus_int32 inLen /* I Number of input samples */ )` | `silk/SigProc_FIX.h:68` | — | prototype |
| `silk_VAD_GetSA_Q8` | `#define silk_VAD_GetSA_Q8(psEnC, pIn, arch) ((void)(arch),silk_VAD_GetSA_Q8_c(psEnC, pIn))` | `silk/main.h:315` | — | prototype |
| `silk_NSQ_del_dec` | `#define silk_NSQ_del_dec(psEncC, NSQ, psIndices, x16, pulses, PredCoef_Q12, LTPCoef_Q14, AR_Q13, \` | `silk/main.h:294` | — | prototype |
| `silk_NSQ` | `#define silk_NSQ(psEncC, NSQ, psIndices, x16, pulses, PredCoef_Q12, LTPCoef_Q14, AR_Q13, \` | `silk/main.h:268` | — | prototype |
| `celt_pitch_xcorr` | `# define celt_pitch_xcorr celt_pitch_xcorr_c` | `celt/pitch.h:189` | — | prototype |
| `celt_inner_prod` | `# define celt_inner_prod(x, y, N, arch) \` | `celt/pitch.h:170` | — | prototype |
| `celt_fir` | `#define celt_fir(x, num, y, N, ord, arch) \` | `celt/celt_lpc.h:51` | — | prototype |
| `celt_iir` | `void celt_iir(const opus_val32 *x, const opus_val16 *den, opus_val32 *y, int N, int ord, opus_val16 *mem, int arch)` | `celt/celt_lpc.h:55` | — | prototype |
| `celt_maxabs16` | `static OPUS_INLINE opus_val32 celt_maxabs16(const opus_val16 *x, int len) ... }` | `celt/mathops.h:86` | — | static inline |
| `celt_maxabs32` | `static OPUS_INLINE opus_val32 celt_maxabs32(const opus_val32 *x, int len) ... }` | `celt/mathops.h:122` | — | static inline |
| `celt_sqrt` | `opus_val32 celt_sqrt(opus_val32 x)` | `celt/mathops.h:383` | `src/opus_encoder.c`×5 | prototype |
| `celt_rsqrt` | `#define celt_rsqrt(x) (1.f/celt_sqrt(x))` | `celt/mathops.h:227` | — | prototype |
| `celt_log2` | `static OPUS_INLINE float celt_log2(float x) ... }` | `celt/mathops.h:272` | — | static inline |
| `celt_exp2` | `static OPUS_INLINE float celt_exp2(float x) ... }` | `celt/mathops.h:313` | `src/opus_encoder.c`×1, `src/opus_decoder.c`×1 | static inline |
| `celt_cos_norm` | `opus_val16 celt_cos_norm(opus_val32 x)` | `celt/mathops.h:387` | — | prototype |
| `celt_rcp` | `opus_val32 celt_rcp(opus_val32 x)` | `celt/mathops.h:536` | — | prototype |
| `celt_lcg_rand` | `opus_uint32 celt_lcg_rand(opus_uint32 seed)` | `celt/bands.h:121` | — | prototype |
| `bitexact_cos` | `opus_int16 bitexact_cos(opus_int16 x)` | `celt/bands.h:39` | — | prototype |
| `bitexact_log2tan` | `int bitexact_log2tan(int isin,int icos)` | `celt/bands.h:40` | — | prototype |
| `isqrt32` | `unsigned isqrt32(opus_uint32 _val)` | `celt/mathops.h:52` | — | prototype |
| `frac_div32` | `opus_val32 frac_div32(opus_val32 a, opus_val32 b)` | `celt/mathops.h:542` | `src/opus_encoder.c`×1 | prototype |

### 4.8 Symbols that are macros, not functions

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

## 5. State structs

Every struct below is reproduced **verbatim** from the 1.6.1 tree, with the exact field declarations (including `#ifdef` guards, array bounds and alignment attributes), because a bit-exact port must reproduce the field set — and, where the layout is observable (arena offsets, `opus_*_get_size()`), the byte layout too.

### 5.1 `OpusEncoder` (`src/opus_encoder.c`)

Defined at `src/opus_encoder.c:76-146`. The struct is preceded by `StereoWidthState` (`:70-74`) and the `MAX_ENCODER_BUFFER` macro (`:62-67`), which is `960` under `ENABLE_QEXT` and `480` otherwise.

`src/opus_encoder.c:62-74`

```c
#ifdef ENABLE_QEXT
#define MAX_ENCODER_BUFFER 960
#else
#define MAX_ENCODER_BUFFER 480
#endif

#define PSEUDO_SNR_THRESHOLD 316.23f    /* 10^(25/10) */

typedef struct {
   opus_val32 XX, XY, YY;
   opus_val16 smoothed_width;
   opus_val16 max_follower;
} StereoWidthState;
```


`src/opus_encoder.c:76-146`

```c
struct OpusEncoder {
    int          celt_enc_offset;
    int          silk_enc_offset;
    silk_EncControlStruct silk_mode;
#ifdef ENABLE_DRED
    DREDEnc      dred_encoder;
#endif
    int          application;
    int          channels;
    int          delay_compensation;
    int          force_channels;
    int          signal_type;
    int          user_bandwidth;
    int          max_bandwidth;
    int          user_forced_mode;
    int          voice_ratio;
    opus_int32   Fs;
    int          use_vbr;
    int          vbr_constraint;
    int          variable_duration;
    opus_int32   bitrate_bps;
    opus_int32   user_bitrate_bps;
    int          lsb_depth;
    int          encoder_buffer;
    int          lfe;
    int          arch;
    int          use_dtx;                 /* general DTX for both SILK and CELT */
    int          fec_config;
#ifndef DISABLE_FLOAT_API
    TonalityAnalysisState analysis;
#endif
#ifdef ENABLE_QEXT
   int enable_qext;
#endif

#define OPUS_ENCODER_RESET_START stream_channels
    int          stream_channels;
    opus_int16   hybrid_stereo_width_Q14;
    opus_int32   variable_HP_smth2_Q15;
    opus_val16   prev_HB_gain;
    opus_val32   hp_mem[4];
    int          mode;
    int          prev_mode;
    int          prev_channels;
    int          prev_framesize;
    int          bandwidth;
    /* Bandwidth determined automatically from the rate (before any other adjustment) */
    int          auto_bandwidth;
    int          silk_bw_switch;
    /* Sampling rate (at the API level) */
    int          first;
    celt_glog * energy_masking;
    StereoWidthState width_mem;
#ifndef DISABLE_FLOAT_API
    int          detected_bandwidth;
#endif
    int          nb_no_activity_ms_Q1;
    opus_val32   peak_signal_energy;
#ifdef ENABLE_DRED
    int          dred_duration;
    int          dred_q0;
    int          dred_dQ;
    int          dred_qmax;
    int          dred_target_chunks;
    unsigned char activity_mem[DRED_MAX_FRAMES*4]; /* 2.5ms resolution*/
#endif
    int          nonfinal_frame; /* current frame is not the final in a packet */
    opus_uint32  rangeFinal;
    /* Needs to be the last field because it may be partially or completely omitted. */
    opus_res     delay_buffer[MAX_ENCODER_BUFFER*2];
};
```

**Porting notes.**
* `celt_enc_offset`/`silk_enc_offset` are *byte offsets into the same allocation*: `opus_encoder_init` (`src/opus_encoder.c:205-245`) computes
  `base_size = align(sizeof(OpusEncoder))`, subtracts part of `delay_buffer` for 1-channel / restricted applications, then lays out `[OpusEncoder][silk_encoder][OpusCustomEncoder]` and stores the offsets. So the C struct is a variable-size arena. A Rust port can use owned sub-structs, but `opus_encoder_get_size()` must still return the same number if it is exposed.
* `#define OPUS_ENCODER_RESET_START stream_channels` marks the first field cleared by `opus_encoder_ctl(OPUS_RESET_STATE)`; every field from `stream_channels` down is reset, everything above is retained. The Rust port must reproduce that split exactly (`reset_state_preserving_prefix`).
* `delay_buffer[MAX_ENCODER_BUFFER*2]` is the last field and is deliberately over-sized: for `application == OPUS_APPLICATION_RESTRICTED_*` it is partially or wholly omitted. It is the resampler-delay compensation ring buffer.
* `energy_masking` is a pointer (`celt_glog *`) into a caller-provided buffer set via `OPUS_SET_ENERGY_MASK`; it is **not** owned by the state.
* `#ifndef DISABLE_FLOAT_API` fields (`analysis`, `detected_bandwidth`) are only present when the float API is compiled in. `analysis` is a large inline `TonalityAnalysisState` — its size changes `opus_encoder_get_size()`.


### 5.2 `OpusDecoder` (`src/opus_decoder.c`)

Defined at `src/opus_decoder.c:65-94`.

`src/opus_decoder.c:65-94`

```c
struct OpusDecoder {
   int          celt_dec_offset;
   int          silk_dec_offset;
   int          channels;
   opus_int32   Fs;          /** Sampling rate (at the API level) */
   silk_DecControlStruct DecControl;
   int          decode_gain;
   int          complexity;
   int          ignore_extensions;
   int          arch;
#ifdef ENABLE_DEEP_PLC
    LPCNetPLCState lpcnet;
#endif

   /* Everything beyond this point gets cleared on a reset */
#define OPUS_DECODER_RESET_START stream_channels
   int          stream_channels;

   int          bandwidth;
   int          mode;
   int          prev_mode;
   int          frame_size;
   int          prev_redundancy;
   int          last_packet_duration;
#ifndef FIXED_POINT
   opus_val16   softclip_mem[2];
#endif

   opus_uint32  rangeFinal;
};
```

**Porting notes.**
* Same arena trick as the encoder: `opus_decoder_init` sets `st->silk_dec_offset = align(sizeof(OpusDecoder));` and `st->celt_dec_offset = st->silk_dec_offset + silkDecSizeBytes;`, so the allocation is `[OpusDecoder][silk_decoder][OpusCustomDecoder]`.
* `#define OPUS_DECODER_RESET_START stream_channels` is the reset split.
* `softclip_mem[2]` exists only in the **non**-fixed build; in the fixed build `opus_pcm_soft_clip` is a no-op and the field is absent.
* `LPCNetPLCState lpcnet` is a large inline DNN state present only under `ENABLE_DEEP_PLC`; it dominates `opus_decoder_get_size()` in that build.


`src/opus_decoder.c:1339-1346`

```c
struct OpusDREDDecoder {
#ifdef ENABLE_DRED
   RDOVAEDec model;
#endif
   int loaded;
   int arch;
   opus_uint32 magic;
};
```

`OpusDREDDecoder` (DRED only; DNN, out of scope).


### 5.3 `silk_encoder_state` and friends (`silk/structs.h`)

These are the **per-channel** SILK state structs shared by the fixed and float builds. `silk/structs.h:146-239` is `silk_encoder_state`; `silk_decoder_state` is `:286-341`; `silk_decoder_control` is `:346-354`.

`silk/structs.h:56-69`

```c
typedef struct {
    opus_int16                  xq[           2 * MAX_FRAME_LENGTH ]; /* Buffer for quantized output signal                             */
    opus_int32                  sLTP_shp_Q14[ 2 * MAX_FRAME_LENGTH ];
    opus_int32                  sLPC_Q14[ MAX_SUB_FRAME_LENGTH + NSQ_LPC_BUF_LENGTH ];
    opus_int32                  sAR2_Q14[ MAX_SHAPE_LPC_ORDER ];
    opus_int32                  sLF_AR_shp_Q14;
    opus_int32                  sDiff_shp_Q14;
    opus_int                    lagPrev;
    opus_int                    sLTP_buf_idx;
    opus_int                    sLTP_shp_buf_idx;
    opus_int32                  rand_seed;
    opus_int32                  prev_gain_Q16;
    opus_int                    rewhite_flag;
} silk_nsq_state;
```


`silk/structs.h:74-127`

```c
typedef struct {
    opus_int32                  AnaState[ 2 ];                  /* Analysis filterbank state: 0-8 kHz                                   */
    opus_int32                  AnaState1[ 2 ];                 /* Analysis filterbank state: 0-4 kHz                                   */
    opus_int32                  AnaState2[ 2 ];                 /* Analysis filterbank state: 0-2 kHz                                   */
    opus_int32                  XnrgSubfr[ VAD_N_BANDS ];       /* Subframe energies                                                    */
    opus_int32                  NrgRatioSmth_Q8[ VAD_N_BANDS ]; /* Smoothed energy level in each band                                   */
    opus_int16                  HPstate;                        /* State of differentiator in the lowest band                           */
    opus_int32                  NL[ VAD_N_BANDS ];              /* Noise energy level in each band                                      */
    opus_int32                  inv_NL[ VAD_N_BANDS ];          /* Inverse noise energy level in each band                              */
    opus_int32                  NoiseLevelBias[ VAD_N_BANDS ];  /* Noise level estimator bias/offset                                    */
    opus_int32                  counter;                        /* Frame counter used in the initial phase                              */
} silk_VAD_state;

/* Variable cut-off low-pass filter state */
typedef struct {
    opus_int32                   In_LP_State[ 2 ];           /* Low pass filter state */
    opus_int32                   transition_frame_no;        /* Counter which is mapped to a cut-off frequency */
    opus_int                     mode;                       /* Operating mode, <0: switch down, >0: switch up; 0: do nothing           */
    opus_int32                   saved_fs_kHz;               /* If non-zero, holds the last sampling rate before a bandwidth switching reset. */
} silk_LP_state;

/* Structure containing NLSF codebook */
typedef struct {
    const opus_int16             nVectors;
    const opus_int16             order;
    const opus_int16             quantStepSize_Q16;
    const opus_int16             invQuantStepSize_Q6;
    const opus_uint8             *CB1_NLSF_Q8;
    const opus_int16             *CB1_Wght_Q9;
    const opus_uint8             *CB1_iCDF;
    const opus_uint8             *pred_Q8;
    const opus_uint8             *ec_sel;
    const opus_uint8             *ec_iCDF;
    const opus_uint8             *ec_Rates_Q5;
    const opus_int16             *deltaMin_Q15;
} silk_NLSF_CB_struct;

typedef struct {
    opus_int16                   pred_prev_Q13[ 2 ];
    opus_int16                   sMid[ 2 ];
    opus_int16                   sSide[ 2 ];
    opus_int32                   mid_side_amp_Q0[ 4 ];
    opus_int16                   smth_width_Q14;
    opus_int16                   width_prev_Q14;
    opus_int16                   silent_side_len;
    opus_int8                    predIx[ MAX_FRAMES_PER_PACKET ][ 2 ][ 3 ];
    opus_int8                    mid_only_flags[ MAX_FRAMES_PER_PACKET ];
} stereo_enc_state;

typedef struct {
    opus_int16                   pred_prev_Q13[ 2 ];
    opus_int16                   sMid[ 2 ];
    opus_int16                   sSide[ 2 ];
} stereo_dec_state;
```


`silk/structs.h:129-141`

```c
typedef struct {
    opus_int8                    GainsIndices[ MAX_NB_SUBFR ];
    opus_int8                    LTPIndex[ MAX_NB_SUBFR ];
    opus_int8                    NLSFIndices[ MAX_LPC_ORDER + 1 ];
    opus_int16                   lagIndex;
    opus_int8                    contourIndex;
    opus_int8                    signalType;
    opus_int8                    quantOffsetType;
    opus_int8                    NLSFInterpCoef_Q2;
    opus_int8                    PERIndex;
    opus_int8                    LTP_scaleIndex;
    opus_int8                    Seed;
} SideInfoIndices;
```

`SideInfoIndices` — the decoded/encoded index set passed between index coding and parameter reconstruction.


`silk/structs.h:146-239`

```c
typedef struct {
    opus_int32                   In_HP_State[ 2 ];                  /* High pass filter state                                           */
    opus_int32                   variable_HP_smth1_Q15;             /* State of first smoother                                          */
    opus_int32                   variable_HP_smth2_Q15;             /* State of second smoother                                         */
    silk_LP_state                sLP;                               /* Low pass filter state                                            */
    silk_VAD_state               sVAD;                              /* Voice activity detector state                                    */
    silk_nsq_state               sNSQ;                              /* Noise Shape Quantizer State                                      */
    opus_int16                   prev_NLSFq_Q15[ MAX_LPC_ORDER ];   /* Previously quantized NLSF vector                                 */
    opus_int                     speech_activity_Q8;                /* Speech activity                                                  */
    opus_int                     allow_bandwidth_switch;            /* Flag indicating that switching of internal bandwidth is allowed  */
    opus_int8                    LBRRprevLastGainIndex;
    opus_int8                    prevSignalType;
    opus_int                     prevLag;
    opus_int                     pitch_LPC_win_length;
    opus_int                     max_pitch_lag;                     /* Highest possible pitch lag (samples)                             */
    opus_int32                   API_fs_Hz;                         /* API sampling frequency (Hz)                                      */
    opus_int32                   prev_API_fs_Hz;                    /* Previous API sampling frequency (Hz)                             */
    opus_int                     maxInternal_fs_Hz;                 /* Maximum internal sampling frequency (Hz)                         */
    opus_int                     minInternal_fs_Hz;                 /* Minimum internal sampling frequency (Hz)                         */
    opus_int                     desiredInternal_fs_Hz;             /* Soft request for internal sampling frequency (Hz)                */
    opus_int                     fs_kHz;                            /* Internal sampling frequency (kHz)                                */
    opus_int                     nb_subfr;                          /* Number of 5 ms subframes in a frame                              */
    opus_int                     frame_length;                      /* Frame length (samples)                                           */
    opus_int                     subfr_length;                      /* Subframe length (samples)                                        */
    opus_int                     ltp_mem_length;                    /* Length of LTP memory                                             */
    opus_int                     la_pitch;                          /* Look-ahead for pitch analysis (samples)                          */
    opus_int                     la_shape;                          /* Look-ahead for noise shape analysis (samples)                    */
    opus_int                     shapeWinLength;                    /* Window length for noise shape analysis (samples)                 */
    opus_int32                   TargetRate_bps;                    /* Target bitrate (bps)                                             */
    opus_int                     PacketSize_ms;                     /* Number of milliseconds to put in each packet                     */
    opus_int                     PacketLoss_perc;                   /* Packet loss rate measured by farend                              */
    opus_int32                   frameCounter;
    opus_int                     Complexity;                        /* Complexity setting                                               */
    opus_int                     nStatesDelayedDecision;            /* Number of states in delayed decision quantization                */
    opus_int                     useInterpolatedNLSFs;              /* Flag for using NLSF interpolation                                */
    opus_int                     shapingLPCOrder;                   /* Filter order for noise shaping filters                           */
    opus_int                     predictLPCOrder;                   /* Filter order for prediction filters                              */
    opus_int                     pitchEstimationComplexity;         /* Complexity level for pitch estimator                             */
    opus_int                     pitchEstimationLPCOrder;           /* Whitening filter order for pitch estimator                       */
    opus_int32                   pitchEstimationThreshold_Q16;      /* Threshold for pitch estimator                                    */
    opus_int32                   sum_log_gain_Q7;                   /* Cumulative max prediction gain                                   */
    opus_int                     NLSF_MSVQ_Survivors;               /* Number of survivors in NLSF MSVQ                                 */
    opus_int                     first_frame_after_reset;           /* Flag for deactivating NLSF interpolation, pitch prediction       */
    opus_int                     controlled_since_last_payload;     /* Flag for ensuring codec_control only runs once per packet        */
    opus_int                     warping_Q16;                       /* Warping parameter for warped noise shaping                       */
    opus_int                     useCBR;                            /* Flag to enable constant bitrate                                  */
    opus_int                     prefillFlag;                       /* Flag to indicate that only buffers are prefilled, no coding      */
    const opus_uint8             *pitch_lag_low_bits_iCDF;          /* Pointer to iCDF table for low bits of pitch lag index            */
    const opus_uint8             *pitch_contour_iCDF;               /* Pointer to iCDF table for pitch contour index                    */
    const silk_NLSF_CB_struct    *psNLSF_CB;                        /* Pointer to NLSF codebook                                         */
    opus_int                     input_quality_bands_Q15[ VAD_N_BANDS ];
    opus_int                     input_tilt_Q15;
    opus_int                     SNR_dB_Q7;                         /* Quality setting                                                  */

    opus_int8                    VAD_flags[ MAX_FRAMES_PER_PACKET ];
    opus_int8                    LBRR_flag;
    opus_int                     LBRR_flags[ MAX_FRAMES_PER_PACKET ];

    SideInfoIndices              indices;
    opus_int8                    pulses[ MAX_FRAME_LENGTH ];

    int                          arch;

    /* Input/output buffering */
    opus_int16                   inputBuf[ MAX_FRAME_LENGTH + 2 ];  /* Buffer containing input signal                                   */
    opus_int                     inputBufIx;
    opus_int                     nFramesPerPacket;
    opus_int                     nFramesEncoded;                    /* Number of frames analyzed in current packet                      */

    opus_int                     nChannelsAPI;
    opus_int                     nChannelsInternal;
    opus_int                     channelNb;

    /* Parameters For LTP scaling Control */
    opus_int                     frames_since_onset;

    /* Specifically for entropy coding */
    opus_int                     ec_prevSignalType;
    opus_int16                   ec_prevLagIndex;

    silk_resampler_state_struct resampler_state;

    /* DTX */
    opus_int                     useDTX;                            /* Flag to enable DTX                                               */
    opus_int                     inDTX;                             /* Flag to signal DTX period                                        */
    opus_int                     noSpeechCounter;                   /* Counts concecutive nonactive frames, used by DTX                 */

    /* Inband Low Bitrate Redundancy (LBRR) data */
    opus_int                     useInBandFEC;                      /* Saves the API setting for query                                  */
    opus_int                     LBRR_enabled;                      /* Depends on useInBandFRC, bitrate and packet loss rate            */
    opus_int                     LBRR_GainIncreases;                /* Gains increment for coding LBRR frames                           */
    SideInfoIndices              indices_LBRR[ MAX_FRAMES_PER_PACKET ];
    opus_int8                    pulses_LBRR[ MAX_FRAMES_PER_PACKET ][ MAX_FRAME_LENGTH ];
} silk_encoder_state;
```

`silk_encoder_state`. Note the `#ifdef _WIN32` float variants, `silk_DWORD_ALIGN` on `predCoef_Q12`/`LTPCoef_Q12`/`AR_Q13`, and the many `MAX_*`-sized arrays (see `silk/define.h`). Fields with `FIXED_POINT`-conditional types (`silk_float` vs Q-domain ints) must be resolved by choosing the fixed build.


`silk/structs.h:243-252`

```c
typedef struct {
    OSCEFeatureState features;
    OSCEState state;
    int method;
} silk_OSCE_struct;

typedef struct {
    OSCEBWEFeatureState features;
    OSCEBWEState state;
} silk_OSCE_BWE_struct;
```

`ENABLE_OSCE` only (DNN, out of scope).


`silk/structs.h:256-281`

```c
typedef struct {
    opus_int32                  pitchL_Q8;                          /* Pitch lag to use for voiced concealment                          */
    opus_int16                  LTPCoef_Q14[ LTP_ORDER ];           /* LTP coefficients to use for voiced concealment                   */
    opus_int16                  prevLPC_Q12[ MAX_LPC_ORDER ];
    opus_int                    last_frame_lost;                    /* Was previous frame lost                                          */
    opus_int32                  rand_seed;                          /* Seed for unvoiced signal generation                              */
    opus_int16                  randScale_Q14;                      /* Scaling of unvoiced random signal                                */
    opus_int32                  conc_energy;
    opus_int                    conc_energy_shift;
    opus_int16                  prevLTP_scale_Q14;
    opus_int32                  prevGain_Q16[ 2 ];
    opus_int                    fs_kHz;
    opus_int                    nb_subfr;
    opus_int                    subfr_length;
    opus_int                    enable_deep_plc;
} silk_PLC_struct;

/* Struct for CNG */
typedef struct {
    opus_int32                  CNG_exc_buf_Q14[ MAX_FRAME_LENGTH ];
    opus_int16                  CNG_smth_NLSF_Q15[ MAX_LPC_ORDER ];
    opus_int32                  CNG_synth_state[ MAX_LPC_ORDER ];
    opus_int32                  CNG_smth_Gain_Q16;
    opus_int32                  rand_seed;
    opus_int                    fs_kHz;
} silk_CNG_struct;
```


`silk/structs.h:286-341`

```c
typedef struct {
#ifdef ENABLE_OSCE
    silk_OSCE_struct            osce;
#ifdef ENABLE_OSCE_BWE
    silk_OSCE_BWE_struct        osce_bwe;
#endif
#endif
#define SILK_DECODER_STATE_RESET_START prev_gain_Q16
    opus_int32                  prev_gain_Q16;
    opus_int32                  exc_Q14[ MAX_FRAME_LENGTH ];
    opus_int32                  sLPC_Q14_buf[ MAX_LPC_ORDER ];
    opus_int16                  outBuf[ MAX_FRAME_LENGTH + 2 * MAX_SUB_FRAME_LENGTH ];  /* Buffer for output signal                     */
    opus_int                    lagPrev;                            /* Previous Lag                                                     */
    opus_int8                   LastGainIndex;                      /* Previous gain index                                              */
    opus_int                    fs_kHz;                             /* Sampling frequency in kHz                                        */
    opus_int32                  fs_API_hz;                          /* API sample frequency (Hz)                                        */
    opus_int                    nb_subfr;                           /* Number of 5 ms subframes in a frame                              */
    opus_int                    frame_length;                       /* Frame length (samples)                                           */
    opus_int                    subfr_length;                       /* Subframe length (samples)                                        */
    opus_int                    ltp_mem_length;                     /* Length of LTP memory                                             */
    opus_int                    LPC_order;                          /* LPC order                                                        */
    opus_int16                  prevNLSF_Q15[ MAX_LPC_ORDER ];      /* Used to interpolate LSFs                                         */
    opus_int                    first_frame_after_reset;            /* Flag for deactivating NLSF interpolation                         */
    const opus_uint8            *pitch_lag_low_bits_iCDF;           /* Pointer to iCDF table for low bits of pitch lag index            */
    const opus_uint8            *pitch_contour_iCDF;                /* Pointer to iCDF table for pitch contour index                    */

    /* For buffering payload in case of more frames per packet */
    opus_int                    nFramesDecoded;
    opus_int                    nFramesPerPacket;

    /* Specifically for entropy coding */
    opus_int                    ec_prevSignalType;
    opus_int16                  ec_prevLagIndex;

    opus_int                    VAD_flags[ MAX_FRAMES_PER_PACKET ];
    opus_int                    LBRR_flag;
    opus_int                    LBRR_flags[ MAX_FRAMES_PER_PACKET ];

    silk_resampler_state_struct resampler_state;

    const silk_NLSF_CB_struct   *psNLSF_CB;                         /* Pointer to NLSF codebook                                         */

    /* Quantization indices */
    SideInfoIndices             indices;

    /* CNG state */
    silk_CNG_struct             sCNG;

    /* Stuff used for PLC */
    opus_int                    lossCnt;
    opus_int                    prevSignalType;
    int                         arch;

    silk_PLC_struct sPLC;

} silk_decoder_state;
```


`silk/structs.h:346-354`

```c
typedef struct {
    /* Prediction and coding parameters */
    opus_int                    pitchL[ MAX_NB_SUBFR ];
    opus_int32                  Gains_Q16[ MAX_NB_SUBFR ];
    /* Holds interpolated and final coefficients, 4-byte aligned */
    silk_DWORD_ALIGN opus_int16 PredCoef_Q12[ 2 ][ MAX_LPC_ORDER ];
    opus_int16                  LTPCoef_Q14[ LTP_ORDER * MAX_NB_SUBFR ];
    opus_int                    LTP_scale_Q14;
} silk_decoder_control;
```


### 5.4 `silk_encoder` / `silk_decoder` super-structs

These are the **top-level** SILK state objects that `OpusEncoder`/`OpusDecoder` embed. `silk_encoder` is **not** in `silk/structs.h`: it is defined separately for the fixed and float builds in `silk/fixed/structs_FIX.h` and `silk/float/structs_FLP.h`. `silk_decoder` is defined locally in `silk/dec_API.c`.

`silk/fixed/structs_FIX.h:30-109`

```c

#include "typedef.h"
#include "main.h"
#include "structs.h"


/********************************/
/* Noise shaping analysis state */
/********************************/
typedef struct {
    opus_int8                   LastGainIndex;
    opus_int32                  HarmBoost_smth_Q16;
    opus_int32                  HarmShapeGain_smth_Q16;
    opus_int32                  Tilt_smth_Q16;
} silk_shape_state_FIX;

/********************************/
/* Encoder state FIX            */
/********************************/
typedef struct {
    silk_encoder_state          sCmn;                                   /* Common struct, shared with floating-point code       */
    silk_shape_state_FIX        sShape;                                 /* Shape state                                          */

    /* Buffer for find pitch and noise shape analysis */
    silk_DWORD_ALIGN opus_int16 x_buf[ 2 * MAX_FRAME_LENGTH + LA_SHAPE_MAX ];/* Buffer for find pitch and noise shape analysis  */
    opus_int                    LTPCorr_Q15;                            /* Normalized correlation from pitch lag estimator      */
    opus_int32                    resNrgSmth;
} silk_encoder_state_FIX;

/************************/
/* Encoder control FIX  */
/************************/
typedef struct {
    /* Prediction and coding parameters */
    opus_int32                  Gains_Q16[ MAX_NB_SUBFR ];
    silk_DWORD_ALIGN opus_int16 PredCoef_Q12[ 2 ][ MAX_LPC_ORDER ];
    opus_int16                  LTPCoef_Q14[ LTP_ORDER * MAX_NB_SUBFR ];
    opus_int                    LTP_scale_Q14;
    opus_int                    pitchL[ MAX_NB_SUBFR ];

    /* Noise shaping parameters */
    /* Testing */
    silk_DWORD_ALIGN opus_int16 AR_Q13[ MAX_NB_SUBFR * MAX_SHAPE_LPC_ORDER ];
    opus_int32                  LF_shp_Q14[        MAX_NB_SUBFR ];      /* Packs two int16 coefficients per int32 value         */
    opus_int                    Tilt_Q14[          MAX_NB_SUBFR ];
    opus_int                    HarmShapeGain_Q14[ MAX_NB_SUBFR ];
    opus_int                    Lambda_Q10;
    opus_int                    input_quality_Q14;
    opus_int                    coding_quality_Q14;

    /* measures */
    opus_int32                  predGain_Q16;
    opus_int                    LTPredCodGain_Q7;
    opus_int32                  ResNrg[ MAX_NB_SUBFR ];                 /* Residual energy per subframe                         */
    opus_int                    ResNrgQ[ MAX_NB_SUBFR ];                /* Q domain for the residual energy > 0                 */

    /* Parameters for CBR mode */
    opus_int32                  GainsUnq_Q16[ MAX_NB_SUBFR ];
    opus_int8                   lastGainIndexPrev;
} silk_encoder_control_FIX;

/************************/
/* Encoder Super Struct */
/************************/
typedef struct {
    stereo_enc_state            sStereo;
    opus_int32                  nBitsUsedLBRR;
    opus_int32                  nBitsExceeded;
    opus_int                    nChannelsAPI;
    opus_int                    nChannelsInternal;
    opus_int                    nPrevChannelsInternal;
    opus_int                    timeSinceSwitchAllowed_ms;
    opus_int                    allowBandwidthSwitch;
    opus_int                    prev_decode_only_middle;
    /* This needs to be last so we can skip the second state for mono. */
    silk_encoder_state_FIX      state_Fxx[ ENCODER_NUM_CHANNELS ];
} silk_encoder;


#endif
```

The whole fixed-build header: `silk_shape_state_FIX` (`:39-44`), `silk_encoder_state_FIX` (`:49-57`), `silk_encoder_control_FIX` (`:62-89`) and `silk_encoder` (`:94-106`). **This is the state a bit-exact fixed-point port must reproduce.**


`silk/float/structs_FLP.h:30-105`

```c

#include "typedef.h"
#include "main.h"
#include "structs.h"


/********************************/
/* Noise shaping analysis state */
/********************************/
typedef struct {
    opus_int8                   LastGainIndex;
    silk_float                  HarmShapeGain_smth;
    silk_float                  Tilt_smth;
} silk_shape_state_FLP;

/********************************/
/* Encoder state FLP            */
/********************************/
typedef struct {
    silk_encoder_state          sCmn;                               /* Common struct, shared with fixed-point code */
    silk_shape_state_FLP        sShape;                             /* Noise shaping state */

    /* Buffer for find pitch and noise shape analysis */
    silk_float                  x_buf[ 2 * MAX_FRAME_LENGTH + LA_SHAPE_MAX ];/* Buffer for find pitch and noise shape analysis */
    silk_float                  LTPCorr;                            /* Normalized correlation from pitch lag estimator */
} silk_encoder_state_FLP;

/************************/
/* Encoder control FLP  */
/************************/
typedef struct {
    /* Prediction and coding parameters */
    silk_float                  Gains[ MAX_NB_SUBFR ];
    silk_float                  PredCoef[ 2 ][ MAX_LPC_ORDER ];     /* holds interpolated and final coefficients */
    silk_float                  LTPCoef[LTP_ORDER * MAX_NB_SUBFR];
    silk_float                  LTP_scale;
    opus_int                    pitchL[ MAX_NB_SUBFR ];

    /* Noise shaping parameters */
    silk_float                  AR[ MAX_NB_SUBFR * MAX_SHAPE_LPC_ORDER ];
    silk_float                  LF_MA_shp[     MAX_NB_SUBFR ];
    silk_float                  LF_AR_shp[     MAX_NB_SUBFR ];
    silk_float                  Tilt[          MAX_NB_SUBFR ];
    silk_float                  HarmShapeGain[ MAX_NB_SUBFR ];
    silk_float                  Lambda;
    silk_float                  input_quality;
    silk_float                  coding_quality;

    /* Measures */
    silk_float                  predGain;
    silk_float                  LTPredCodGain;
    silk_float                  ResNrg[ MAX_NB_SUBFR ];             /* Residual energy per subframe */

    /* Parameters for CBR mode */
    opus_int32                  GainsUnq_Q16[ MAX_NB_SUBFR ];
    opus_int8                   lastGainIndexPrev;
} silk_encoder_control_FLP;

/************************/
/* Encoder Super Struct */
/************************/
typedef struct {
    stereo_enc_state            sStereo;
    opus_int32                  nBitsUsedLBRR;
    opus_int32                  nBitsExceeded;
    opus_int                    nChannelsAPI;
    opus_int                    nChannelsInternal;
    opus_int                    nPrevChannelsInternal;
    opus_int                    timeSinceSwitchAllowed_ms;
    opus_int                    allowBandwidthSwitch;
    opus_int                    prev_decode_only_middle;
    /* This needs to be last so we can skip the second state for mono. */
    silk_encoder_state_FLP      state_Fxx[ ENCODER_NUM_CHANNELS ];
} silk_encoder;

#endif
```

The float-build counterpart, for reference only (not bit-exact). Note it also defines a type named `silk_encoder`, so the two headers are mutually exclusive — exactly as `FIXED_POINT` selects.


`silk/dec_API.c:44-56`

```c
/************************/
/* Decoder Super Struct */
/************************/
typedef struct {
    silk_decoder_state          channel_state[ DECODER_NUM_CHANNELS ];
    stereo_dec_state                sStereo;
    opus_int                         nChannelsAPI;
    opus_int                         nChannelsInternal;
    opus_int                         prev_decode_only_middle;
#ifdef ENABLE_OSCE
    OSCEModel                        osce_model;
#endif
} silk_decoder;
```

`silk_decoder`. `DECODER_NUM_CHANNELS` is 2. Under `ENABLE_OSCE` an `OSCEModel osce_model` field is appended.


### 5.5 `OpusCustomEncoder` / `OpusCustomDecoder` (= `CELTEncoder` / `CELTDecoder`)

`celt/celt.h:57-59` defines the aliases:
```c
#define CELTEncoder OpusCustomEncoder
#define CELTDecoder OpusCustomDecoder
#define CELTMode OpusCustomMode
```
The struct bodies live in the encoder/decoder `.c` files, not in the header.

`celt/celt_encoder.c:63-142`

```c
struct OpusCustomEncoder {
   const OpusCustomMode *mode;     /**< Mode used by the encoder */
   int channels;
   int stream_channels;

   int force_intra;
   int clip;
   int disable_pf;
   int complexity;
   int upsample;
   int start, end;

   opus_int32 bitrate;
   int vbr;
   int signalling;
   int constrained_vbr;      /* If zero, VBR can do whatever it likes with the rate */
   int loss_rate;
   int lsb_depth;
   int lfe;
   int disable_inv;
   int arch;
#ifdef ENABLE_QEXT
   int enable_qext;
   int qext_scale;
#endif

   /* Everything beyond this point gets cleared on a reset */
#define ENCODER_RESET_START rng

   opus_uint32 rng;
   int spread_decision;
   opus_val32 delayedIntra;
   int tonal_average;
   int lastCodedBands;
   int hf_average;
   int tapset_decision;

   int prefilter_period;
   opus_val16 prefilter_gain;
   int prefilter_tapset;
#ifdef RESYNTH
   int prefilter_period_old;
   opus_val16 prefilter_gain_old;
   int prefilter_tapset_old;
#endif
   int consec_transient;
   AnalysisInfo analysis;
   SILKInfo silk_info;

   opus_val32 preemph_memE[2];
   opus_val32 preemph_memD[2];

   /* VBR-related parameters */
   opus_int32 vbr_reservoir;
   opus_int32 vbr_drift;
   opus_int32 vbr_offset;
   opus_int32 vbr_count;
   opus_val32 overlap_max;
   opus_val16 stereo_saving;
   int intensity;
   celt_glog *energy_mask;
   celt_glog spec_avg;

#ifdef RESYNTH
#ifdef ENABLE_QEXT
   /* +MAX_PERIOD/2 to make space for overlap */
   celt_sig syn_mem[2][2*DEC_PITCH_BUF_SIZE+MAX_PERIOD];
#else
   /* +MAX_PERIOD/2 to make space for overlap */
   celt_sig syn_mem[2][DEC_PITCH_BUF_SIZE+MAX_PERIOD/2];
#endif
#endif

   celt_sig in_mem[1]; /* Size = channels*mode->overlap */
   /* celt_sig prefilter_mem[],  Size = channels*COMBFILTER_MAXPERIOD */
   /* celt_glog oldBandE[],     Size = channels*mode->nbEBands */
   /* celt_glog oldLogE[],      Size = channels*mode->nbEBands */
   /* celt_glog oldLogE2[],     Size = channels*mode->nbEBands */
   /* celt_glog energyError[],  Size = channels*mode->nbEBands */
};
```

`struct OpusCustomEncoder`. It embeds an `AnalysisInfo analysis` and, under `ENABLE_QEXT`, a `SILKInfo silkinfo`; it also embeds `OpusCustomMode`-independent state plus `prefilter_period_old`, etc. Under `ENABLE_DEEP_PLC`, DNN members are appended.


`celt/celt_decoder.c:87-139`

```c
struct OpusCustomDecoder {
   const OpusCustomMode *mode;
   int overlap;
   int channels;
   int stream_channels;

   int downsample;
   int start, end;
   int signalling;
   int disable_inv;
   int complexity;
   int arch;
#ifdef ENABLE_QEXT
   int qext_scale;
#endif

   /* Everything beyond this point gets cleared on a reset */
#define DECODER_RESET_START rng

   opus_uint32 rng;
   int error;
   int last_pitch_index;
   int loss_duration;
   int plc_duration;
   int last_frame_type;
   int skip_plc;
   int postfilter_period;
   int postfilter_period_old;
   opus_val16 postfilter_gain;
   opus_val16 postfilter_gain_old;
   int postfilter_tapset;
   int postfilter_tapset_old;
   int prefilter_and_fold;

   celt_sig preemph_memD[2];

#ifdef ENABLE_DEEP_PLC
   opus_int16 plc_pcm[PLC_UPDATE_SAMPLES];
   int plc_fill;
   float plc_preemphasis_mem;
#endif

#ifdef ENABLE_QEXT
   celt_glog qext_oldBandE[2*NB_QEXT_BANDS];
#endif

   celt_sig _decode_mem[1]; /* Size = channels*(DECODE_BUFFER_SIZE+mode->overlap) */
   /* celt_glog oldEBands[], Size = 2*mode->nbEBands */
   /* celt_glog oldLogE[], Size = 2*mode->nbEBands */
   /* celt_glog oldLogE2[], Size = 2*mode->nbEBands */
   /* celt_glog backgroundLogE[], Size = 2*mode->nbEBands */
   /* opus_val16 lpc[],  Size = channels*CELT_LPC_ORDER */
};
```

`struct OpusCustomDecoder`.


### 5.6 `OpusCustomMode` / `CELTMode` and `PulseCache` (`celt/modes.h`)

`celt/modes.h:44-49`

```c
typedef struct {
   int size;
   const opus_int16 *index;
   const unsigned char *bits;
   const unsigned char *caps;
} PulseCache;
```

`PulseCache`.


`celt/modes.h:54-77`

```c
struct OpusCustomMode {
   opus_int32 Fs;
   int          overlap;

   int          nbEBands;
   int          effEBands;
   opus_val16    preemph[4];
   const opus_int16   *eBands;   /**< Definition for each "pseudo-critical band" */

   int         maxLM;
   int         nbShortMdcts;
   int         shortMdctSize;

   int          nbAllocVectors; /**< Number of lines in the matrix below */
   const unsigned char   *allocVectors;   /**< Number of bits in each band for several rates */
   const opus_int16 *logN;

   const celt_coef *window;
   mdct_lookup mdct;
   PulseCache cache;
#ifdef ENABLE_QEXT
   PulseCache qext_cache;
#endif
};
```

`struct OpusCustomMode`. `ENABLE_QEXT` appends the QEXT fields. The `opus_*` API never sees this struct, but the opus layer reads its fields directly (see §4.10).


### 5.7 FFT and MDCT state (`celt/kiss_fft.h`, `celt/mdct.h`)

`celt/kiss_fft.h:78-110`

```c
typedef struct {
    kiss_fft_scalar r;
    kiss_fft_scalar i;
}kiss_fft_cpx;

typedef struct {
   kiss_twiddle_scalar r;
   kiss_twiddle_scalar i;
} KISS_TWIDDLE_CPX_ALIGNED kiss_twiddle_cpx;

#define MAXFACTORS 8
/* e.g. an fft of length 128 has 4 factors
 as far as kissfft is concerned
 4*4*4*2
 */

typedef struct arch_fft_state{
   int is_supported;
   void *priv;
} arch_fft_state;

typedef struct kiss_fft_state{
    int nfft;
    celt_coef scale;
#ifdef FIXED_POINT
    int scale_shift;
#endif
    int shift;
    opus_int16 factors[2*MAXFACTORS];
    const opus_int16 *bitrev;
    const kiss_twiddle_cpx *twiddles;
    arch_fft_state *arch_fft;
} kiss_fft_state;
```


`celt/mdct.h:49-54`

```c
typedef struct {
   int n;
   int maxshift;
   const kiss_fft_state *kfft[4];
   const kiss_twiddle_scalar * OPUS_RESTRICT trig;
} mdct_lookup;
```


### 5.8 `OpusRepacketizer` (`src/opus_private.h`)

`src/opus_private.h:39-48`

```c
struct OpusRepacketizer {
   unsigned char toc;
   int nb_frames;
   const unsigned char *frames[48];
   opus_int16 len[48];
   int framesize;
   const unsigned char *paddings[48];
   opus_int32 padding_len[48];
   unsigned char padding_nb_frames[48];
};
```


### 5.9 Supporting state structs a port must also reproduce

These are all declared in `silk/structs.h` unless noted, and every one of them is embedded (by value) in `silk_encoder_state`, `silk_decoder_state` or `silk_encoder`, so their exact layout and element counts matter.

`silk/structs.h:88-93`

```c
typedef struct {
    opus_int32                   In_LP_State[ 2 ];           /* Low pass filter state */
    opus_int32                   transition_frame_no;        /* Counter which is mapped to a cut-off frequency */
    opus_int                     mode;                       /* Operating mode, <0: switch down, >0: switch up; 0: do nothing           */
    opus_int32                   saved_fs_kHz;               /* If non-zero, holds the last sampling rate before a bandwidth switching reset. */
} silk_LP_state;
```

`silk_LP_state`.


`silk/structs.h:96-109`

```c
typedef struct {
    const opus_int16             nVectors;
    const opus_int16             order;
    const opus_int16             quantStepSize_Q16;
    const opus_int16             invQuantStepSize_Q6;
    const opus_uint8             *CB1_NLSF_Q8;
    const opus_int16             *CB1_Wght_Q9;
    const opus_uint8             *CB1_iCDF;
    const opus_uint8             *pred_Q8;
    const opus_uint8             *ec_sel;
    const opus_uint8             *ec_iCDF;
    const opus_uint8             *ec_Rates_Q5;
    const opus_int16             *deltaMin_Q15;
} silk_NLSF_CB_struct;
```

`silk_NLSF_CB_struct` — the pointer table that binds a codebook (`silk_NLSF_CB_NB_MB` / `silk_NLSF_CB_WB`) to its derived vectors.


`silk/resampler_structs.h:30-50`

```c

#define SILK_RESAMPLER_MAX_FIR_ORDER                 36
#define SILK_RESAMPLER_MAX_IIR_ORDER                 6

typedef struct _silk_resampler_state_struct{
    opus_int32       sIIR[ SILK_RESAMPLER_MAX_IIR_ORDER ]; /* this must be the first element of this struct */
    union{
        opus_int32   i32[ SILK_RESAMPLER_MAX_FIR_ORDER ];
        opus_int16   i16[ SILK_RESAMPLER_MAX_FIR_ORDER ];
    }                sFIR;
    opus_int16       delayBuf[ 96 ];
    opus_int         resampler_function;
    opus_int         batchSize;
    opus_int32       invRatio_Q16;
    opus_int         FIR_Order;
    opus_int         FIR_Fracs;
    opus_int         Fs_in_kHz;
    opus_int         Fs_out_kHz;
    opus_int         inputDelay;
    const opus_int16 *Coefs;
} silk_resampler_state_struct;
```

`silk_resampler_state_struct`. `#ifdef USE_SMALL_DIV_TABLE` changes the table reference; the coefficient table pointer must point at `silk_resampler_frac_FIR_12` / the up2-HQ arrays from `silk/resampler_rom.c`.


`silk/control.h:42-120`

```c
typedef struct {
    /* I:   Number of channels; 1/2                                                         */
    opus_int32 nChannelsAPI;

    /* I:   Number of channels; 1/2                                                         */
    opus_int32 nChannelsInternal;

    /* I:   Input signal sampling rate in Hertz; 8000/12000/16000/24000/32000/44100/48000   */
    opus_int32 API_sampleRate;

    /* I:   Maximum internal sampling rate in Hertz; 8000/12000/16000                       */
    opus_int32 maxInternalSampleRate;

    /* I:   Minimum internal sampling rate in Hertz; 8000/12000/16000                       */
    opus_int32 minInternalSampleRate;

    /* I:   Soft request for internal sampling rate in Hertz; 8000/12000/16000              */
    opus_int32 desiredInternalSampleRate;

    /* I:   Number of samples per packet in milliseconds; 10/20/40/60                       */
    opus_int payloadSize_ms;

    /* I:   Bitrate during active speech in bits/second; internally limited                 */
    opus_int32 bitRate;

    /* I:   Uplink packet loss in percent (0-100)                                           */
    opus_int packetLossPercentage;

    /* I:   Complexity mode; 0 is lowest, 10 is highest complexity                          */
    opus_int complexity;

    /* I:   Flag to enable in-band Forward Error Correction (FEC); 0/1                      */
    opus_int useInBandFEC;

    /* I:   Flag to enable in-band Deep REDundancy (DRED); 0/1                              */
    opus_int useDRED;

    /* I:   Flag to actually code in-band Forward Error Correction (FEC) in the current packet; 0/1 */
    opus_int LBRR_coded;

    /* I:   Flag to enable discontinuous transmission (DTX); 0/1                            */
    opus_int useDTX;

    /* I:   Flag to use constant bitrate                                                    */
    opus_int useCBR;

    /* I:   Maximum number of bits allowed for the frame                                    */
    opus_int maxBits;

    /* I:   Causes a smooth downmix to mono                                                 */
    opus_int toMono;

    /* I:   Opus encoder is allowing us to switch bandwidth                                 */
    opus_int opusCanSwitch;

    /* I: Make frames as independent as possible (but still use LPC)                        */
    opus_int reducedDependency;

    /* O:   Internal sampling rate used, in Hertz; 8000/12000/16000                         */
    opus_int32 internalSampleRate;

    /* O: Flag that bandwidth switching is allowed (because low voice activity)             */
    opus_int allowBandwidthSwitch;

    /* O:   Flag that SILK runs in WB mode without variable LP filter (use for switching between WB/SWB/FB) */
    opus_int inWBmodeWithoutVariableLP;

    /* O:   Stereo width */
    opus_int stereoWidth_Q14;

    /* O:   Tells the Opus encoder we're ready to switch                                    */
    opus_int switchReady;

    /* O: SILK Signal type */
    opus_int signalType;

    /* O: SILK offset (dithering) */
    opus_int offset;
} silk_EncControlStruct;
```

`silk_EncControlStruct` — **the** encoder/decoder control block that `src/opus_encoder.c` fills in and `silk_Encode` consumes. Its fields are the actual cross-layer contract (`nChannelsAPI`, `nChannelsInternal`, `API_sampleRate`, `maxInternalSampleRate`, `minInternalSampleRate`, `desiredInternalSampleRate`, `payloadSize_ms`, `bitRate`, `packetLossPercentage`, `complexity`, `useInBandFEC`, `useDTX`, `useCBR`, `maxBits`, `toMono`, `opaque`, `useHardRedundancy`, ...).


`silk/control.h:125-162`

```c
typedef struct {
    /* I:   Number of channels; 1/2                                                         */
    opus_int32 nChannelsAPI;

    /* I:   Number of channels; 1/2                                                         */
    opus_int32 nChannelsInternal;

    /* I:   Output signal sampling rate in Hertz; 8000/12000/16000/24000/32000/44100/48000  */
    opus_int32 API_sampleRate;

    /* I:   Internal sampling rate used, in Hertz; 8000/12000/16000                         */
    opus_int32 internalSampleRate;

    /* I:   Number of samples per packet in milliseconds; 10/20/40/60                       */
    opus_int payloadSize_ms;

    /* O:   Pitch lag of previous frame (0 if unvoiced), measured in samples at 48 kHz      */
    opus_int prevPitchLag;

    /* I:   Enable Deep PLC                                                                 */
    opus_int enable_deep_plc;

#ifdef ENABLE_OSCE
    /* I: OSCE method */
    opus_int osce_method;

#ifdef ENABLE_OSCE_BWE
    /* I: OSCE bandwidth extension method */
    opus_int enable_osce_bwe;

    /* I: extended mode */
    opus_int osce_extended_mode;

    /* O: previous extended mode */
    opus_int prev_osce_extended_mode;
#endif
#endif
} silk_DecControlStruct;
```

`silk_DecControlStruct`.


`silk/API.h:44-48`

```c
typedef struct {
    opus_int    VADFlag;                                /* Voice activity for packet                            */
    opus_int    VADFlags[ SILK_MAX_FRAMES_PER_PACKET ]; /* Voice activity for each frame in packet              */
    opus_int    inbandFECFlag;                          /* Flag indicating if packet contains in-band FEC       */
} silk_TOC_struct;
```

`silk_TOC_struct`.


`celt/celt.h:65-84`

```c
typedef struct {
   int valid;
   float tonality;
   float tonality_slope;
   float noisiness;
   float activity;
   float music_prob;
   float music_prob_min;
   float music_prob_max;
   int   bandwidth;
   float activity_probability;
   float max_pitch_ratio;
   /* Store as Q6 char to save space. */
   unsigned char leak_boost[LEAK_BANDS];
} AnalysisInfo;

typedef struct {
   int signalType;
   int offset;
} SILKInfo;
```

`AnalysisInfo` and `SILKInfo` — embedded in the CELT encoder state; `AnalysisInfo` is filled by the float-API analysis layer and influences CELT mode decisions. `leak_boost[LEAK_BANDS]` is `unsigned char` even in the float build ("Store as Q6 char to save space").


`src/mapping_matrix.h:39-45`

```c
typedef struct MappingMatrix
{
    int rows; /* number of channels outputted from matrix. */
    int cols; /* number of channels inputted to matrix. */
    int gain; /* in dB. S7.8-format. */
    /* Matrix cell data goes here using col-wise ordering. */
} MappingMatrix;
```

`MappingMatrix` — needed only for the multistream/projection layer.


### 5.10 The entropy coder: one struct for both directions

`celt/entcode.h:46-48` declares a **single** struct used for both encoder and decoder:
```ctypedef struct ec_ctx         ec_ctx;
typedef struct ec_ctx         ec_enc;
typedef struct ec_ctx         ec_dec;
```
The union-of-roles is deliberate ("We use the same structure for both, so that common functions like ec_tell() can be used on either one"). A Rust port should mirror this with one `EcCtx` type used by both `enc` and `dec` modules, so that `ec_tell` can be shared.

`celt/entcode.h:42-91`

```c

/*OPT: ec_window must be at least 32 bits, but if you have fast arithmetic on a
   larger type, you can speed up the decoder by using it here.*/
typedef opus_uint32           ec_window;
typedef struct ec_ctx         ec_ctx;
typedef struct ec_ctx         ec_enc;
typedef struct ec_ctx         ec_dec;

# define EC_WINDOW_SIZE ((int)sizeof(ec_window)*CHAR_BIT)

/*The number of bits to use for the range-coded part of unsigned integers.*/
# define EC_UINT_BITS   (8)

/*The resolution of fractional-precision bit usage measurements, i.e.,
   3 => 1/8th bits.*/
# define BITRES 3

/*The entropy encoder/decoder context.
  We use the same structure for both, so that common functions like ec_tell()
   can be used on either one.*/
struct ec_ctx{
   /*Buffered input/output.*/
   unsigned char *buf;
   /*The size of the buffer.*/
   opus_uint32    storage;
   /*The offset at which the last byte containing raw bits was read/written.*/
   opus_uint32    end_offs;
   /*Bits that will be read from/written at the end.*/
   ec_window      end_window;
   /*Number of valid bits in end_window.*/
   int            nend_bits;
   /*The total number of whole bits read/written.
     This does not include partial bits currently in the range coder.*/
   int            nbits_total;
   /*The offset at which the next range coder byte will be read/written.*/
   opus_uint32    offs;
   /*The number of values in the current range.*/
   opus_uint32    rng;
   /*In the decoder: the difference between the top of the current range and
      the input value, minus one.
     In the encoder: the low end of the current range.*/
   opus_uint32    val;
   /*In the decoder: the saved normalization factor from ec_decode().
     In the encoder: the number of outstanding carry propagating symbols.*/
   opus_uint32    ext;
   /*A buffered input/output symbol, awaiting carry propagation.*/
   int            rem;
   /*Nonzero if an error occurred.*/
   int            error;
};
```

Includes the exact `#define`s: `EC_WINDOW_SIZE`, `EC_UINT_BITS (8)`, `BITRES (3)`, and the `USE_SMALL_DIV_TABLE` switch.



### 5.11 Entropy-coder bit-exact invariants

The following invariants are what make the range coder reproducible. They are
derived from `celt/entcode.h`, `celt/mfrngcod.h`, `celt/entenc.c`,
`celt/entdec.c`; the exact code is quoted in §7.4.

| Field | Encoder meaning | Decoder meaning | Invariant |
|---|---|---|---|
| `buf` | output buffer | input buffer | never reallocated while in use; `ec_enc_shrink` only lowers `storage`. |
| `storage` | usable bytes | usable bytes | encoder: `storage` may be *decreased* by `ec_enc_shrink`; decoder: negative-byte reads are clamped and set `error`. |
| `end_offs` | offset of the last byte written with raw bits, counting **down from `storage`** | offset of the last byte read for raw bits, counting down from `storage` | raw bits are stored at the **end** of the buffer, range-coded bits from the front; the two halves must not collide. `end_offs` grows downward. |
| `end_window` | accumulator of raw bits not yet flushed | accumulator of raw bits not yet consumed | `EC_WINDOW_SIZE == sizeof(ec_window)*CHAR_BIT == 32`. `ec_enc_bits`/`ec_dec_bits` must fill/drain in exactly the same order. |
| `nend_bits` | number of valid bits in `end_window` | idem | `0 <= nend_bits < EC_WINDOW_SIZE` after each call; `ec_enc_done`/`ec_dec_init` must leave it 0. |
| `nbits_total` | total whole bits written (excludes partial bits in the range coder) | total whole bits read | monotone non-decreasing; `ec_tell()` = `nbits_total - EC_ILOG(rng)`. |
| `offs` | offset of the next range-coder byte to write | offset of the next byte to read | Writes are **refused** once `offs+end_offs >= storage` (`ec_write_byte`/`ec_write_byte_at_end` return `-1`, and the caller folds that into `error`), so `offs <= storage` always. The decoder returns `0` for reads past `storage` (`ec_dec_read_byte`) and records `error`. API contract: `nbCompressedBytes >= 2` (`celt/celt_encoder.c:1831`), `max_data_bytes >= 3` (`src/opus_encoder.c:1340`). |
| `rng` | size of the current range | size of the current range | `EC_CODE_TOP <= rng <= 2^32-1` after normalisation; `rng` drives `ec_tell` via `EC_ILOG`. |
| `val` | low end of the current range | `EC_CODE_TOP - 1 - (value - low)` | encoder `val` only enters `ec_enc_carry_out`; decoder `val` is *subtracted* in `ec_decode`. |
| `rem` | buffered byte awaiting carry propagation | (unused by the decoder's arithmetic; still present) | `rem` must be flushed by `ec_enc_carry_out` exactly once; `-1` means "no buffered byte". |
| `ext` | number of outstanding carry-propagating symbols | saved normalisation factor from `ec_decode` | `ec_encode` must set `ext = rng/_ft` and the `while (rng <= EC_CODE_BOTTOM)` loop must run the same number of iterations as C. |
| `error` | (not used) | set on malformed input | must be sticky: once set, it stays set; `ec_dec_bits`/`ec_dec_icdf` must degrade identically. |

Constants (exact text in §7.4): `EC_SYM_BITS = 8`, `EC_CODE_BITS = 32`,
`EC_SYM_MAX = 0xFF`, `EC_CODE_TOP = 1u<<(EC_CODE_BITS-1)`,
`EC_CODE_BOT = EC_CODE_TOP>>EC_SYM_BITS`,
`EC_CODE_EXTRA = (EC_CODE_BITS-2)%EC_SYM_BITS+1`,
`EC_CODE_BOTTOM = EC_CODE_BOT<<EC_CODE_EXTRA`, `BITRES = 3`,
`EC_UINT_BITS = 8`, `EC_WINDOW_SIZE = 32`.

`ec_tell()`/`ec_tell_frac()` are *derived* values, so they must not be stored:
```c
static OPUS_INLINE int ec_tell(ec_ctx *_this){
  return _this->nbits_total-EC_ILOG(_this->rng);
}
```
and `ec_tell_frac` (`celt/entcode.c:69-112`) is a 3-bit fractional refinement
built from `rng` alone — see §7.4 for the exact loop.

### 5.12 Allocation / ownership summary

| Struct | Allocation in C | Rust recommendation |
|---|---|---|
| `OpusEncoder` | variable-size arena `[OpusEncoder][silk_encoder][OpusCustomEncoder]`, offsets stored in the struct | owned `Box<OpusEncoder>` containing `silk: SilkEncoder`, `celt: CeltEncoder` fields (or `Box`ed); replicate `OPUS_ENCODER_RESET_START`. |
| `OpusDecoder` | arena `[OpusDecoder][silk_decoder][OpusCustomDecoder]` | same pattern; replicate `OPUS_DECODER_RESET_START`. |
| `silk_encoder` | `state_Fxx[MAX_NUM_CHANNELS]` inline, plus `sStereo` | inline array of 2 `SilkEncoderState`, indexed by channel. |
| `silk_decoder` | `channel_state[DECODER_NUM_CHANNELS]` inline | inline array of 2 `SilkDecoderState`. |
| `silk_resampler_state_struct` | inline in `silk_encoder_state`/`silk_decoder_state` | inline; the `sCoefs`/`sFIR`/`sIIR` unions are plain fixed arrays. |
| `ec_ctx` | inline in `OpusEncoder`? — **no**: created on the stack in `opus_encode_native`/`opus_decode_frame` | `EcCtx` value on the stack, passed as `&mut`. |
| `CELTMode`/`OpusCustomMode` | one per rate, created by `opus_custom_mode_create`, cached in `static` pointers in `src/opus_encoder.c`/`opus_decoder.c` | lazily-initialised `static`/`OnceLock` per sampling rate, matching the C static cache. |
| mode tables (`static_modes_fixed.h`) | `static const` arrays in the mode object; some in read-only data, some allocated by `opus_custom_mode_create` | `&'static` tables from generated Rust, plus heap-allocated `allocVectors`/`cache` where C allocates. |

---

## 6. Data tables

Exhaustive manifest of every constant table in the portable core (`celt/`, `silk/`, `src/`), with the **exact C declaration text**. SIMD-only directories, `celt/tests`, and the debug/macro-count headers are excluded.

**Method.** The extractor is preprocessor-aware: it builds two *selected views* of each source file — first-branch-of-every-`#if` and last-branch-of-every-`#if` — and extracts tables from both, because some tables put the **declaration** inside `#if/#else` (e.g. `CELT_PVQ_U_DATA[1488]` vs `[1272]`, `celt/cwrs.c:215-217`) and others put the **initialiser** inside `#if/#else` (e.g. `fft_twiddles48000_960`, `celt/static_modes_fixed.h:159-483`). Where the two views differ, both variants are listed and tagged.

**Counts.** *Declared* is the product of the numeric array dimensions. *Groups* is the number of comma-separated items at brace depth 1 (an aggregate such as `{r,i}` counts as one group). *Leaves* is the total number of leaf values. So a flat scalar array has `groups == leaves == declared`; an array of 2-field structs has `groups == declared` and `leaves == 2*declared`; a 2-D scalar array `[R][C]` has `groups == R` and `leaves == declared == R*C`. Commas inside parentheses (macro invocations such as `QCONST16(x,15)`) are not separators.

Tables with more than 600 elements, and every table in the generated mode headers, have their initialiser elided and are referenced by `file:line` for mechanical extraction (`sed -n 'A,Bp'`).

### 6.1 Summary index

| Symbol | File:line | C type (verbatim head) | Dims | Declared | Groups | Leaves | Scope | `#if` guard | Note |
|---|---|---|---|---:|---:|---:|---|---|---|
| `tcount` | `celt/bands.c:490` | `int` | `[3]` | 3 | 3 | 3 | function-local | — | inlined |
| `ordery_table` | `celt/bands.c:567` | `static const int` | `[]` | — | 30 | 30 | file | — | inlined |
| `exp2_table8` | `celt/bands.c:640` | `static const opus_int16` | `[8]` | 8 | 8 | 8 | function-local | — | inlined |
| `bit_interleave_table` | `celt/bands.c:1291` | `static const unsigned char` | `[16]` | 16 | 16 | 16 | function-local | — | inlined |
| `bit_deinterleave_table` | `celt/bands.c:1357` | `static const unsigned char` | `[16]` | 16 | 16 | 16 | function-local | — | inlined |
| `gains` | `celt/celt.c:246` | `static const opus_val16` | `[3][3]` | 9 | 3 | 9 | function-local | — | inlined |
| `tf_select_table` | `celt/celt.c:320` | `const signed char` | `[4][8]` | 32 | 4 | 32 | file | — | inlined |
| `error_strings` | `celt/celt.c:344` | `static const char * const` | `[8]` | 8 | 8 | 8 | function-local | — | inlined |
| `trim_icdf` | `celt/celt.h:194` | `static const unsigned char` | `[11]` | 11 | 11 | 11 | file | — | inlined |
| `spread_icdf` | `celt/celt.h:196` | `static const unsigned char` | `[4]` | 4 | 4 | 4 | file | — | inlined |
| `tapset_icdf` | `celt/celt.h:198` | `static const unsigned char` | `[3]` | 3 | 3 | 3 | file | — | inlined |
| `toOpusTable` | `celt/celt.h:201` | `static const unsigned char` | `[20]` | 20 | 20 | 20 | file | `defined(CUSTOM_MODES) \|\| defined(ENABLE_OPUS_CUSTOM_API)` | inlined |
| `fromOpusTable` | `celt/celt.h:209` | `static const unsigned char` | `[16]` | 16 | 16 | 16 | file | `defined(CUSTOM_MODES) \|\| defined(ENABLE_OPUS_CUSTOM_API)` | inlined |
| `sinc_filter` | `celt/celt_decoder.c:629` | `static const float` | `[SINC_ORDER+1]` | — | 49 | 49 | file | `ENABLE_DEEP_PLC` | inlined |
| `inv_table` | `celt/celt_encoder.c:286` | `static const unsigned char` | `[128]` | 128 | 128 | 128 | function-local | — | inlined |
| `before` | `celt/celt_encoder.c:1419` | `opus_val32` | `[2]` | 2 | 1 | 1 | function-local | — | inlined |
| `spread_thresholds` | `celt/celt_encoder.c:2329` | `static const opus_val16` | `[3]` | 3 | 3 | 3 | function-local | — | inlined |
| `spread_histeresis` | `celt/celt_encoder.c:2330` | `static const opus_val16` | `[3]` | 3 | 3 | 3 | function-local | — | inlined |
| `tapset_thresholds` | `celt/celt_encoder.c:2331` | `static const opus_val16` | `[2]` | 2 | 2 | 2 | function-local | — | inlined |
| `tapset_histeresis` | `celt/celt_encoder.c:2332` | `static const opus_val16` | `[2]` | 2 | 2 | 2 | function-local | — | inlined |
| `intensity_thresholds` | `celt/celt_encoder.c:2393` | `static const opus_val16` | `[21]` | 21 | 21 | 21 | function-local | — | inlined |
| `intensity_histeresis` | `celt/celt_encoder.c:2396` | `static const opus_val16` | `[21]` | 21 | 21 | 21 | function-local | — | inlined |
| `CELT_PVQ_U_DATA` | `celt/cwrs.c:215` | `static const opus_uint32` | `[1488]` | 1488 | 1488 | 1488 | file | `!defined(SMALL_FOOTPRINT) && defined(CWRS_EXTRA_ROWS)` | inlined |
| `CELT_PVQ_U_ROW` | `celt/cwrs.c:417` | `static const opus_uint32 *const` | `[15]` | 15 | 15 | 15 | file | `!defined(SMALL_FOOTPRINT) && defined(CWRS_EXTRA_ROWS)` | inlined |
| `correction` | `celt/entcode.c:70` | `static const unsigned` | `[8]` | 8 | 8 | 8 | function-local | — | inlined |
| `SMALL_DIV_TABLE` | `celt/entcode.c:119` | `const opus_uint32` | `[129]` | 129 | 128 | 128 | file | `USE_SMALL_DIV_TABLE` | inlined |
| `val` | `celt/laplace.c:205` | `int` | `[NB_VALS]` | — | 10 | 10 | function-local | — | inlined |
| `C` | `celt/mathops.c:148` | `static const opus_val16` | `[6]` | 6 | 6 | 6 | function-local | `FIXED_POINT` | inlined |
| `log2_x_norm_coeff` | `celt/mathops.h:252` | `static const float` | `[8]` | 8 | 8 | 8 | file | `!FIXED_POINT && FLOAT_APPROX` | inlined |
| `log2_y_norm_coeff` | `celt/mathops.h:266` | `static const float` | `[8]` | 8 | 8 | 8 | file | `!FIXED_POINT && FLOAT_APPROX` | inlined |
| `C` | `celt/mathops.h:398` | `static const opus_val16` | `[5]` | 5 | 5 | 5 | function-local | `FIXED_POINT` | inlined |
| `log2_x_norm_coeff` | `celt/mathops.h:450` | `static const opus_val32` | `[8]` | 8 | 8 | 8 | function-local | `FIXED_POINT && ENABLE_QEXT` | inlined |
| `log2_y_norm_coeff` | `celt/mathops.h:454` | `static const opus_val32` | `[8]` | 8 | 8 | 8 | function-local | `FIXED_POINT && ENABLE_QEXT` | inlined |
| `eband5ms` | `celt/modes.c:42` | `static const opus_int16` | `[]` | — | 22 | 22 | file | — | inlined |
| `band_allocation` | `celt/modes.c:50` | `static const unsigned char` | `[]` | — | 231 | 231 | file | — | inlined |
| `bark_freq` | `celt/modes.c:82` | `static const opus_int16` | `[BARK_BANDS+1]` | — | 26 | 26 | file | `CUSTOM_MODES` | inlined |
| `qext_eBands_180` | `celt/modes.c:486` | `static const opus_int16` | `[]` | — | 15 | 15 | file | `ENABLE_QEXT` | inlined |
| `qext_logN_180` | `celt/modes.c:491` | `static const opus_int16` | `[]` | — | 15 | 15 | file | `ENABLE_QEXT` | inlined |
| `qext_eBands_240` | `celt/modes.c:494` | `static const opus_int16` | `[]` | — | 15 | 15 | file | `ENABLE_QEXT` | inlined |
| `qext_logN_240` | `celt/modes.c:499` | `static const opus_int16` | `[]` | — | 14 | 14 | file | `ENABLE_QEXT` | inlined |
| `format_size` | `celt/opus_custom_demo.c:97` | `static const int` | `[3]` | 3 | 3 | 3 | file | — | inlined |
| `sum` | `celt/pitch.c:269` | `opus_val32` | `[4]` | 4 | 4 | 4 | function-local | — | variant |
| `best_pitch` | `celt/pitch.c:312` | `int` | `[2]` | 2 | 2 | 2 | function-local | — | inlined |
| `second_check` | `celt/pitch.c:453` | `static const int` | `[16]` | 16 | 16 | 16 | file | — | inlined |
| `eMeans` | `celt/quant_bands.c:44` | `const signed char` | `[25]` | 25 | 25 | 25 | file | `FIXED_POINT` | inlined |
| `eMeans` | `celt/quant_bands.c:53` | `const opus_val16` | `[25]` | 25 | 25 | 25 | file | `!(FIXED_POINT)` | variant |
| `pred_coef` | `celt/quant_bands.c:63` | `static const opus_val16` | `[4]` | 4 | 4 | 4 | file | `FIXED_POINT` | inlined |
| `beta_coef` | `celt/quant_bands.c:64` | `static const opus_val16` | `[4]` | 4 | 4 | 4 | file | `FIXED_POINT` | inlined |
| `pred_coef` | `celt/quant_bands.c:67` | `static const opus_val16` | `[4]` | 4 | 4 | 4 | file | `!(FIXED_POINT)` | variant |
| `beta_coef` | `celt/quant_bands.c:68` | `static const opus_val16` | `[4]` | 4 | 4 | 4 | file | `!(FIXED_POINT)` | variant |
| `e_prob_model` | `celt/quant_bands.c:77` | `static const unsigned char` | `[4][2][42]` | 336 | 4 | 336 | file | — | inlined |
| `small_energy_icdf` | `celt/quant_bands.c:140` | `static const unsigned char` | `[3]` | 3 | 3 | 3 | file | — | inlined |
| `prev` | `celt/quant_bands.c:164` | `opus_val32` | `[2]` | 2 | 2 | 2 | function-local | — | inlined |
| `prev` | `celt/quant_bands.c:435` | `opus_val64` | `[2]` | 2 | 2 | 2 | function-local | — | inlined |
| `LOG2_FRAC_TABLE` | `celt/rate.c:43` | `static const unsigned char` | `[24]` | 24 | 24 | 24 | file | — | inlined |
| `maxN` | `celt/rate.c:57` | `static const opus_int16` | `[15]` | 15 | 15 | 15 | function-local | `defined(CUSTOM_MODES)` | inlined |
| `maxK` | `celt/rate.c:60` | `static const opus_int16` | `[15]` | 15 | 15 | 15 | function-local | `defined(CUSTOM_MODES)` | inlined |
| `last_zero` | `celt/rate.c:649` | `static const unsigned char` | `[3]` | 3 | 3 | 3 | file | `ENABLE_QEXT` | inlined |
| `last_cap` | `celt/rate.c:650` | `static const unsigned char` | `[3]` | 3 | 3 | 3 | file | `ENABLE_QEXT` | inlined |
| `last_other` | `celt/rate.c:651` | `static const unsigned char` | `[4]` | 4 | 4 | 4 | file | `ENABLE_QEXT` | inlined |
| `window120` | `celt/static_modes_fixed.h:14` | `static const celt_coef` | `[120]` | 120 | 120 | 120 | file | — | generated/elided |
| `logN400` | `celt/static_modes_fixed.h:71` | `static const opus_int16` | `[21]` | 21 | 21 | 21 | file | — | generated/elided |
| `cache_index50` | `celt/static_modes_fixed.h:77` | `static const opus_int16` | `[105]` | 105 | 105 | 105 | file | — | generated/elided |
| `cache_bits50` | `celt/static_modes_fixed.h:86` | `static const unsigned char` | `[392]` | 392 | 392 | 392 | file | — | generated/elided |
| `cache_caps50` | `celt/static_modes_fixed.h:114` | `static const unsigned char` | `[168]` | 168 | 168 | 168 | file | — | generated/elided |
| `qext_cache_index50` | `celt/static_modes_fixed.h:132` | `static const opus_int16` | `[70]` | 70 | 70 | 70 | file | `ENABLE_QEXT` | generated/elided |
| `qext_cache_bits50` | `celt/static_modes_fixed.h:138` | `static const unsigned char` | `[86]` | 86 | 86 | 86 | file | `ENABLE_QEXT` | generated/elided |
| `qext_cache_caps50` | `celt/static_modes_fixed.h:145` | `static const unsigned char` | `[112]` | 112 | 112 | 112 | file | `ENABLE_QEXT` | generated/elided |
| `fft_twiddles48000_960` | `celt/static_modes_fixed.h:159` | `static const kiss_twiddle_cpx` | `[480]` | 480 | 480 | 960 | file | — | generated/elided |
| `fft_bitrev480` | `celt/static_modes_fixed.h:646` | `static const opus_int16` | `[480]` | 480 | 480 | 480 | file | — | generated/elided |
| `fft_bitrev240` | `celt/static_modes_fixed.h:684` | `static const opus_int16` | `[240]` | 240 | 240 | 240 | file | — | generated/elided |
| `fft_bitrev120` | `celt/static_modes_fixed.h:706` | `static const opus_int16` | `[120]` | 120 | 120 | 120 | file | — | generated/elided |
| `fft_bitrev60` | `celt/static_modes_fixed.h:720` | `static const opus_int16` | `[60]` | 60 | 60 | 60 | file | — | generated/elided |
| `mdct_twiddles960` | `celt/static_modes_fixed.h:820` | `static const celt_coef` | `[1800]` | 1800 | 1800 | 1800 | file | — | generated/elided |
| `window240` | `celt/static_modes_fixed.h:1571` | `static const celt_coef` | `[240]` | 240 | 240 | 240 | file | `ENABLE_QEXT` | generated/elided |
| `logN400` | `celt/static_modes_fixed.h:1676` | `static const opus_int16` | `[21]` | 21 | 21 | 21 | file | `ENABLE_QEXT` | generated/elided |
| `cache_index50` | `celt/static_modes_fixed.h:1682` | `static const opus_int16` | `[105]` | 105 | 105 | 105 | file | `ENABLE_QEXT` | generated/elided |
| `cache_bits50` | `celt/static_modes_fixed.h:1691` | `static const unsigned char` | `[392]` | 392 | 392 | 392 | file | `ENABLE_QEXT` | generated/elided |
| `cache_caps50` | `celt/static_modes_fixed.h:1719` | `static const unsigned char` | `[168]` | 168 | 168 | 168 | file | `ENABLE_QEXT` | generated/elided |
| `qext_cache_index50` | `celt/static_modes_fixed.h:1737` | `static const opus_int16` | `[70]` | 70 | 70 | 70 | file | `ENABLE_QEXT && ENABLE_QEXT` | generated/elided |
| `qext_cache_bits50` | `celt/static_modes_fixed.h:1743` | `static const unsigned char` | `[86]` | 86 | 86 | 86 | file | `ENABLE_QEXT && ENABLE_QEXT` | generated/elided |
| `qext_cache_caps50` | `celt/static_modes_fixed.h:1750` | `static const unsigned char` | `[112]` | 112 | 112 | 112 | file | `ENABLE_QEXT && ENABLE_QEXT` | generated/elided |
| `fft_twiddles96000_1920` | `celt/static_modes_fixed.h:1764` | `static const kiss_twiddle_cpx` | `[960]` | 960 | 960 | 1920 | file | `ENABLE_QEXT` | generated/elided |
| `fft_bitrev960` | `celt/static_modes_fixed.h:2731` | `static const opus_int16` | `[960]` | 960 | 960 | 960 | file | `ENABLE_QEXT` | generated/elided |
| `fft_bitrev480` | `celt/static_modes_fixed.h:2801` | `static const opus_int16` | `[480]` | 480 | 480 | 480 | file | `ENABLE_QEXT` | generated/elided |
| `fft_bitrev240` | `celt/static_modes_fixed.h:2839` | `static const opus_int16` | `[240]` | 240 | 240 | 240 | file | `ENABLE_QEXT` | generated/elided |
| `fft_bitrev120` | `celt/static_modes_fixed.h:2861` | `static const opus_int16` | `[120]` | 120 | 120 | 120 | file | `ENABLE_QEXT` | generated/elided |
| `mdct_twiddles1920` | `celt/static_modes_fixed.h:2965` | `static const celt_coef` | `[3600]` | 3600 | 3600 | 3600 | file | `ENABLE_QEXT` | generated/elided |
| `static_mode_list` | `celt/static_modes_fixed.h:4437` | `static const CELTMode * const` | `[TOTAL_MODES]` | — | 2 | 2 | file | `ENABLE_QEXT` | generated/elided |
| `static_mode_list` | `celt/static_modes_fixed.h:4443` | `static const CELTMode * const` | `[TOTAL_MODES]` | — | 1 | 1 | file | `!(ENABLE_QEXT)` | variant |
| `ne10_factors_480` | `celt/static_modes_fixed_arm_ne10.h:8` | `static const ne10_int32_t` | `[64]` | 64 | 64 | 64 | file | — | generated/elided |
| `ne10_factors_240` | `celt/static_modes_fixed_arm_ne10.h:14` | `static const ne10_int32_t` | `[64]` | 64 | 64 | 64 | file | — | generated/elided |
| `ne10_factors_120` | `celt/static_modes_fixed_arm_ne10.h:20` | `static const ne10_int32_t` | `[64]` | 64 | 64 | 64 | file | — | generated/elided |
| `ne10_factors_60` | `celt/static_modes_fixed_arm_ne10.h:26` | `static const ne10_int32_t` | `[64]` | 64 | 64 | 64 | file | — | generated/elided |
| `ne10_twiddles_480` | `celt/static_modes_fixed_arm_ne10.h:32` | `static const ne10_fft_cpx_int32_t` | `[480]` | 480 | 480 | 960 | file | — | generated/elided |
| `ne10_twiddles_240` | `celt/static_modes_fixed_arm_ne10.h:194` | `static const ne10_fft_cpx_int32_t` | `[240]` | 240 | 240 | 480 | file | — | generated/elided |
| `ne10_twiddles_120` | `celt/static_modes_fixed_arm_ne10.h:276` | `static const ne10_fft_cpx_int32_t` | `[120]` | 120 | 120 | 240 | file | — | generated/elided |
| `ne10_twiddles_60` | `celt/static_modes_fixed_arm_ne10.h:318` | `static const ne10_fft_cpx_int32_t` | `[60]` | 60 | 60 | 120 | file | — | generated/elided |
| `window120` | `celt/static_modes_float.h:14` | `static const celt_coef` | `[120]` | 120 | 120 | 120 | file | — | generated/elided |
| `logN400` | `celt/static_modes_float.h:44` | `static const opus_int16` | `[21]` | 21 | 21 | 21 | file | — | generated/elided |
| `cache_index50` | `celt/static_modes_float.h:50` | `static const opus_int16` | `[105]` | 105 | 105 | 105 | file | — | generated/elided |
| `cache_bits50` | `celt/static_modes_float.h:59` | `static const unsigned char` | `[392]` | 392 | 392 | 392 | file | — | generated/elided |
| `cache_caps50` | `celt/static_modes_float.h:87` | `static const unsigned char` | `[168]` | 168 | 168 | 168 | file | — | generated/elided |
| `qext_cache_index50` | `celt/static_modes_float.h:105` | `static const opus_int16` | `[70]` | 70 | 70 | 70 | file | `ENABLE_QEXT` | generated/elided |
| `qext_cache_bits50` | `celt/static_modes_float.h:111` | `static const unsigned char` | `[86]` | 86 | 86 | 86 | file | `ENABLE_QEXT` | generated/elided |
| `qext_cache_caps50` | `celt/static_modes_float.h:118` | `static const unsigned char` | `[112]` | 112 | 112 | 112 | file | `ENABLE_QEXT` | generated/elided |
| `fft_twiddles48000_960` | `celt/static_modes_float.h:132` | `static const kiss_twiddle_cpx` | `[480]` | 480 | 480 | 960 | file | — | generated/elided |
| `fft_bitrev480` | `celt/static_modes_float.h:376` | `static const opus_int16` | `[480]` | 480 | 480 | 480 | file | — | generated/elided |
| `fft_bitrev240` | `celt/static_modes_float.h:414` | `static const opus_int16` | `[240]` | 240 | 240 | 240 | file | — | generated/elided |
| `fft_bitrev120` | `celt/static_modes_float.h:436` | `static const opus_int16` | `[120]` | 120 | 120 | 120 | file | — | generated/elided |
| `fft_bitrev60` | `celt/static_modes_float.h:450` | `static const opus_int16` | `[60]` | 60 | 60 | 60 | file | — | generated/elided |
| `mdct_twiddles960` | `celt/static_modes_float.h:530` | `static const celt_coef` | `[1800]` | 1800 | 1800 | 1800 | file | — | generated/elided |
| `window240` | `celt/static_modes_float.h:918` | `static const celt_coef` | `[240]` | 240 | 240 | 240 | file | `ENABLE_QEXT` | generated/elided |
| `logN400` | `celt/static_modes_float.h:972` | `static const opus_int16` | `[21]` | 21 | 21 | 21 | file | `ENABLE_QEXT` | generated/elided |
| `cache_index50` | `celt/static_modes_float.h:978` | `static const opus_int16` | `[105]` | 105 | 105 | 105 | file | `ENABLE_QEXT` | generated/elided |
| `cache_bits50` | `celt/static_modes_float.h:987` | `static const unsigned char` | `[392]` | 392 | 392 | 392 | file | `ENABLE_QEXT` | generated/elided |
| `cache_caps50` | `celt/static_modes_float.h:1015` | `static const unsigned char` | `[168]` | 168 | 168 | 168 | file | `ENABLE_QEXT` | generated/elided |
| `qext_cache_index50` | `celt/static_modes_float.h:1033` | `static const opus_int16` | `[70]` | 70 | 70 | 70 | file | `ENABLE_QEXT && ENABLE_QEXT` | generated/elided |
| `qext_cache_bits50` | `celt/static_modes_float.h:1039` | `static const unsigned char` | `[86]` | 86 | 86 | 86 | file | `ENABLE_QEXT && ENABLE_QEXT` | generated/elided |
| `qext_cache_caps50` | `celt/static_modes_float.h:1046` | `static const unsigned char` | `[112]` | 112 | 112 | 112 | file | `ENABLE_QEXT && ENABLE_QEXT` | generated/elided |
| `fft_twiddles96000_1920` | `celt/static_modes_float.h:1060` | `static const kiss_twiddle_cpx` | `[960]` | 960 | 960 | 1920 | file | `ENABLE_QEXT` | generated/elided |
| `fft_bitrev960` | `celt/static_modes_float.h:1544` | `static const opus_int16` | `[960]` | 960 | 960 | 960 | file | `ENABLE_QEXT` | generated/elided |
| `fft_bitrev480` | `celt/static_modes_float.h:1614` | `static const opus_int16` | `[480]` | 480 | 480 | 480 | file | `ENABLE_QEXT` | generated/elided |
| `fft_bitrev240` | `celt/static_modes_float.h:1652` | `static const opus_int16` | `[240]` | 240 | 240 | 240 | file | `ENABLE_QEXT` | generated/elided |
| `fft_bitrev120` | `celt/static_modes_float.h:1674` | `static const opus_int16` | `[120]` | 120 | 120 | 120 | file | `ENABLE_QEXT` | generated/elided |
| `mdct_twiddles1920` | `celt/static_modes_float.h:1758` | `static const celt_coef` | `[3600]` | 3600 | 3600 | 3600 | file | `ENABLE_QEXT` | generated/elided |
| `static_mode_list` | `celt/static_modes_float.h:2507` | `static const CELTMode * const` | `[TOTAL_MODES]` | — | 2 | 2 | file | `ENABLE_QEXT` | generated/elided |
| `static_mode_list` | `celt/static_modes_float.h:2513` | `static const CELTMode * const` | `[TOTAL_MODES]` | — | 1 | 1 | file | `!(ENABLE_QEXT)` | variant |
| `ne10_factors_480` | `celt/static_modes_float_arm_ne10.h:8` | `static const ne10_int32_t` | `[64]` | 64 | 64 | 64 | file | — | generated/elided |
| `ne10_factors_240` | `celt/static_modes_float_arm_ne10.h:14` | `static const ne10_int32_t` | `[64]` | 64 | 64 | 64 | file | — | generated/elided |
| `ne10_factors_120` | `celt/static_modes_float_arm_ne10.h:20` | `static const ne10_int32_t` | `[64]` | 64 | 64 | 64 | file | — | generated/elided |
| `ne10_factors_60` | `celt/static_modes_float_arm_ne10.h:26` | `static const ne10_int32_t` | `[64]` | 64 | 64 | 64 | file | — | generated/elided |
| `ne10_twiddles_480` | `celt/static_modes_float_arm_ne10.h:32` | `static const ne10_fft_cpx_float32_t` | `[480]` | 480 | 480 | 960 | file | — | generated/elided |
| `ne10_twiddles_240` | `celt/static_modes_float_arm_ne10.h:194` | `static const ne10_fft_cpx_float32_t` | `[240]` | 240 | 240 | 480 | file | — | generated/elided |
| `ne10_twiddles_120` | `celt/static_modes_float_arm_ne10.h:276` | `static const ne10_fft_cpx_float32_t` | `[120]` | 120 | 120 | 240 | file | — | generated/elided |
| `ne10_twiddles_60` | `celt/static_modes_float_arm_ne10.h:318` | `static const ne10_fft_cpx_float32_t` | `[60]` | 60 | 60 | 120 | file | — | generated/elided |
| `SPREAD_FACTOR` | `celt/vq.c:106` | `static const int` | `[3]` | 3 | 3 | 3 | function-local | — | inlined |
| `ordering16` | `silk/NLSF2A.c:75` | `static const unsigned char` | `[16]` | 16 | 16 | 16 | function-local | — | inlined |
| `ordering10` | `silk/NLSF2A.c:78` | `static const unsigned char` | `[10]` | 10 | 10 | 10 | function-local | — | inlined |
| `HARM_ATT_Q15` | `silk/PLC.c:41` | `static const opus_int16` | `[NB_ATT]` | — | 2 | 2 | file | — | inlined |
| `PLC_RAND_ATTENUATE_V_Q15` | `silk/PLC.c:42` | `static const opus_int16` | `[NB_ATT]` | — | 2 | 2 | file | — | inlined |
| `PLC_RAND_ATTENUATE_UV_Q15` | `silk/PLC.c:43` | `static const opus_int16` | `[NB_ATT]` | — | 2 | 2 | file | — | inlined |
| `tiltWeights` | `silk/VAD.c:77` | `static const opus_int32` | `[ VAD_N_BANDS ]` | — | 4 | 4 | file | — | inlined |
| `silk_TargetRate_NB_21` | `silk/control_SNR.c:40` | `static const unsigned char` | `[117 - 10]` | — | 107 | 107 | file | — | inlined |
| `silk_TargetRate_MB_21` | `silk/control_SNR.c:51` | `static const unsigned char` | `[165 - 10]` | — | 155 | 155 | file | — | inlined |
| `silk_TargetRate_WB_21` | `silk/control_SNR.c:65` | `static const unsigned char` | `[201 - 10]` | — | 191 | 191 | file | — | inlined |
| `MS_pred_Q13` | `silk/dec_API.c:154` | `opus_int32` | `[ 2 ]` | 2 | 1 | 1 | function-local | — | inlined |
| `mult_tab` | `silk/dec_API.c:459` | `int` | `[ 3 ]` | 3 | 3 | 3 | function-local | — | inlined |
| `iCDF` | `silk/enc_API.c:358` | `opus_uint8` | `[ 2 ]` | 2 | 2 | 2 | function-local | — | inlined |
| `freq_table_Q16` | `silk/fixed/apply_sine_window_FIX.c:44` | `static const opus_int16` | `[ 27 ]` | 27 | 27 | 27 | file | — | inlined |
| `gain_lock` | `silk/fixed/encode_frame_FIX.c:105` | `opus_int` | `[ MAX_NB_SUBFR ]` | — | 1 | 1 | function-local | — | inlined |
| `NLSF_Q15` | `silk/fixed/find_pred_coefs_FIX.c:46` | `opus_int16` | `[ MAX_LPC_ORDER ]` | — | 1 | 1 | function-local | — | inlined |
| `state_QS` | `silk/fixed/warped_autocorrelation_FIX.c:52` | `opus_int32` | `[ MAX_SHAPE_LPC_ORDER + 1 ]` | — | 1 | 1 | function-local | — | inlined |
| `corr_QC` | `silk/fixed/warped_autocorrelation_FIX.c:53` | `opus_int64` | `[ MAX_SHAPE_LPC_ORDER + 1 ]` | — | 1 | 1 | function-local | — | inlined |
| `gain_lock` | `silk/float/encode_frame_FLP.c:107` | `opus_int` | `[ MAX_NB_SUBFR ]` | — | 1 | 1 | function-local | — | inlined |
| `NLSF_Q15` | `silk/float/find_pred_coefs_FLP.c:48` | `opus_int16` | `[ MAX_LPC_ORDER ]` | — | 1 | 1 | function-local | — | inlined |
| `state` | `silk/float/warped_autocorrelation_FLP.c:45` | `double` | `[ MAX_SHAPE_LPC_ORDER + 1 ]` | — | 1 | 1 | function-local | — | inlined |
| `C` | `silk/float/warped_autocorrelation_FLP.c:46` | `double` | `[ MAX_SHAPE_LPC_ORDER + 1 ]` | — | 1 | 1 | function-local | — | inlined |
| `silk_CB_lags_stage2_10_ms` | `silk/pitch_est_tables.c:35` | `const opus_int8` | `[ PE_MAX_NB_SUBFR >> 1][ PE_NB_CBKS_STAGE2_10MS ]` | — | 2 | 6 | file | — | inlined |
| `silk_CB_lags_stage3_10_ms` | `silk/pitch_est_tables.c:41` | `const opus_int8` | `[ PE_MAX_NB_SUBFR >> 1 ][ PE_NB_CBKS_STAGE3_10MS ]` | — | 2 | 24 | file | — | inlined |
| `silk_Lag_range_stage3_10_ms` | `silk/pitch_est_tables.c:47` | `const opus_int8` | `[ PE_MAX_NB_SUBFR >> 1 ][ 2 ]` | — | 2 | 4 | file | — | inlined |
| `silk_CB_lags_stage2` | `silk/pitch_est_tables.c:53` | `const opus_int8` | `[ PE_MAX_NB_SUBFR ][ PE_NB_CBKS_STAGE2_EXT ]` | — | 4 | 44 | file | — | inlined |
| `silk_CB_lags_stage3` | `silk/pitch_est_tables.c:61` | `const opus_int8` | `[ PE_MAX_NB_SUBFR ][ PE_NB_CBKS_STAGE3_MAX ]` | — | 4 | 136 | file | — | inlined |
| `silk_Lag_range_stage3` | `silk/pitch_est_tables.c:69` | `const opus_int8` | `[ SILK_PE_MAX_COMPLEX + 1 ] [ PE_MAX_NB_SUBFR ][ 2 ]` | — | 3 | 24 | file | — | inlined |
| `silk_nb_cbk_searchs_stage3` | `silk/pitch_est_tables.c:94` | `const opus_int8` | `[ SILK_PE_MAX_COMPLEX + 1 ]` | — | 3 | 3 | file | — | inlined |
| `delay_matrix_enc` | `silk/resampler.c:53` | `static const opus_int8` | `[ 6 ][ 3 ]` | 18 | 6 | 18 | file | — | inlined |
| `delay_matrix_dec` | `silk/resampler.c:63` | `static const opus_int8` | `[ 3 ][ 6 ]` | 18 | 3 | 18 | file | — | inlined |
| `silk_Resampler_3_4_COEFS` | `silk/resampler_rom.c:43` | `silk_DWORD_ALIGN const opus_int16` | `[ 2 + 3 * RESAMPLER_DOWN_ORDER_FIR0 / 2 ]` | — | 29 | 29 | file | — | inlined |
| `silk_Resampler_2_3_COEFS` | `silk/resampler_rom.c:50` | `silk_DWORD_ALIGN const opus_int16` | `[ 2 + 2 * RESAMPLER_DOWN_ORDER_FIR0 / 2 ]` | — | 20 | 20 | file | — | inlined |
| `silk_Resampler_1_2_COEFS` | `silk/resampler_rom.c:56` | `silk_DWORD_ALIGN const opus_int16` | `[ 2 + RESAMPLER_DOWN_ORDER_FIR1 / 2 ]` | — | 14 | 14 | file | — | inlined |
| `silk_Resampler_1_3_COEFS` | `silk/resampler_rom.c:61` | `silk_DWORD_ALIGN const opus_int16` | `[ 2 + RESAMPLER_DOWN_ORDER_FIR2 / 2 ]` | — | 20 | 20 | file | — | inlined |
| `silk_Resampler_1_4_COEFS` | `silk/resampler_rom.c:66` | `silk_DWORD_ALIGN const opus_int16` | `[ 2 + RESAMPLER_DOWN_ORDER_FIR2 / 2 ]` | — | 20 | 20 | file | — | inlined |
| `silk_Resampler_1_6_COEFS` | `silk/resampler_rom.c:71` | `silk_DWORD_ALIGN const opus_int16` | `[ 2 + RESAMPLER_DOWN_ORDER_FIR2 / 2 ]` | — | 20 | 20 | file | — | inlined |
| `silk_Resampler_2_3_COEFS_LQ` | `silk/resampler_rom.c:76` | `silk_DWORD_ALIGN const opus_int16` | `[ 2 + 2 * 2 ]` | — | 6 | 6 | file | — | inlined |
| `silk_resampler_frac_FIR_12` | `silk/resampler_rom.c:83` | `silk_DWORD_ALIGN const opus_int16` | `[ 12 ][ RESAMPLER_ORDER_FIR_12 / 2 ]` | — | 12 | 48 | file | — | inlined |
| `silk_resampler_up2_hq_0` | `silk/resampler_rom.h:45` | `static const opus_int16` | `[ 3 ]` | 3 | 3 | 3 | file | — | inlined |
| `silk_resampler_up2_hq_1` | `silk/resampler_rom.h:46` | `static const opus_int16` | `[ 3 ]` | 3 | 3 | 3 | file | — | inlined |
| `sigm_LUT_slope_Q10` | `silk/sigm_Q15.c:37` | `static const opus_int32` | `[ 6 ]` | 6 | 6 | 6 | file | — | inlined |
| `sigm_LUT_pos_Q15` | `silk/sigm_Q15.c:41` | `static const opus_int32` | `[ 6 ]` | 6 | 6 | 6 | file | — | inlined |
| `sigm_LUT_neg_Q15` | `silk/sigm_Q15.c:45` | `static const opus_int32` | `[ 6 ]` | 6 | 6 | 6 | file | — | inlined |
| `silk_LSFCosTab_FIX_Q12` | `silk/table_LSF_cos.c:36` | `const opus_int16` | `[ LSF_COS_TAB_SZ_FIX + 1 ]` | — | 129 | 129 | file | — | inlined |
| `silk_LTP_per_index_iCDF` | `silk/tables_LTP.c:34` | `const opus_uint8` | `[3]` | 3 | 3 | 3 | file | — | inlined |
| `silk_LTP_gain_iCDF_0` | `silk/tables_LTP.c:38` | `static const opus_uint8` | `[8]` | 8 | 8 | 8 | file | — | inlined |
| `silk_LTP_gain_iCDF_1` | `silk/tables_LTP.c:42` | `static const opus_uint8` | `[16]` | 16 | 16 | 16 | file | — | inlined |
| `silk_LTP_gain_iCDF_2` | `silk/tables_LTP.c:47` | `static const opus_uint8` | `[32]` | 32 | 32 | 32 | file | — | inlined |
| `silk_LTP_gain_BITS_Q5_0` | `silk/tables_LTP.c:54` | `static const opus_uint8` | `[8]` | 8 | 8 | 8 | file | — | inlined |
| `silk_LTP_gain_BITS_Q5_1` | `silk/tables_LTP.c:58` | `static const opus_uint8` | `[16]` | 16 | 16 | 16 | file | — | inlined |
| `silk_LTP_gain_BITS_Q5_2` | `silk/tables_LTP.c:63` | `static const opus_uint8` | `[32]` | 32 | 32 | 32 | file | — | inlined |
| `silk_LTP_gain_iCDF_ptrs` | `silk/tables_LTP.c:70` | `const opus_uint8 * const` | `[NB_LTP_CBKS]` | — | 3 | 3 | file | — | inlined |
| `silk_LTP_gain_BITS_Q5_ptrs` | `silk/tables_LTP.c:76` | `const opus_uint8 * const` | `[NB_LTP_CBKS]` | — | 3 | 3 | file | — | inlined |
| `silk_LTP_gain_vq_0` | `silk/tables_LTP.c:82` | `static const opus_int8` | `[8][5]` | 40 | 8 | 40 | file | — | inlined |
| `silk_LTP_gain_vq_1` | `silk/tables_LTP.c:110` | `static const opus_int8` | `[16][5]` | 80 | 16 | 80 | file | — | inlined |
| `silk_LTP_gain_vq_2` | `silk/tables_LTP.c:162` | `static const opus_int8` | `[32][5]` | 160 | 32 | 160 | file | — | inlined |
| `silk_LTP_vq_ptrs_Q7` | `silk/tables_LTP.c:262` | `const opus_int8 * const` | `[NB_LTP_CBKS]` | — | 3 | 3 | file | — | inlined |
| `silk_LTP_gain_vq_0_gain` | `silk/tables_LTP.c:270` | `static const opus_uint8` | `[8]` | 8 | 8 | 8 | file | — | inlined |
| `silk_LTP_gain_vq_1_gain` | `silk/tables_LTP.c:274` | `static const opus_uint8` | `[16]` | 16 | 16 | 16 | file | — | inlined |
| `silk_LTP_gain_vq_2_gain` | `silk/tables_LTP.c:279` | `static const opus_uint8` | `[32]` | 32 | 32 | 32 | file | — | inlined |
| `silk_LTP_vq_gain_ptrs_Q7` | `silk/tables_LTP.c:286` | `const opus_uint8 * const` | `[NB_LTP_CBKS]` | — | 3 | 3 | file | — | inlined |
| `silk_LTP_vq_sizes` | `silk/tables_LTP.c:292` | `const opus_int8` | `[NB_LTP_CBKS]` | — | 3 | 3 | file | — | inlined |
| `silk_NLSF_CB1_NB_MB_Q8` | `silk/tables_NLSF_CB_NB_MB.c:34` | `static const opus_uint8` | `[ 320 ]` | 320 | 320 | 320 | file | — | inlined |
| `silk_NLSF_CB1_Wght_Q9` | `silk/tables_NLSF_CB_NB_MB.c:77` | `static const opus_int16` | `[ 320 ]` | 320 | 320 | 320 | file | — | inlined |
| `silk_NLSF_CB1_iCDF_NB_MB` | `silk/tables_NLSF_CB_NB_MB.c:112` | `static const opus_uint8` | `[ 64 ]` | 64 | 64 | 64 | file | — | inlined |
| `silk_NLSF_CB2_SELECT_NB_MB` | `silk/tables_NLSF_CB_NB_MB.c:123` | `static const opus_uint8` | `[ 160 ]` | 160 | 160 | 160 | file | — | inlined |
| `silk_NLSF_CB2_iCDF_NB_MB` | `silk/tables_NLSF_CB_NB_MB.c:146` | `static const opus_uint8` | `[ 72 ]` | 72 | 72 | 72 | file | — | inlined |
| `silk_NLSF_CB2_BITS_NB_MB_Q5` | `silk/tables_NLSF_CB_NB_MB.c:158` | `static const opus_uint8` | `[ 72 ]` | 72 | 72 | 72 | file | — | inlined |
| `silk_NLSF_PRED_NB_MB_Q8` | `silk/tables_NLSF_CB_NB_MB.c:170` | `static const opus_uint8` | `[ 18 ]` | 18 | 18 | 18 | file | — | inlined |
| `silk_NLSF_DELTA_MIN_NB_MB_Q15` | `silk/tables_NLSF_CB_NB_MB.c:176` | `static const opus_int16` | `[ 11 ]` | 11 | 11 | 11 | file | — | inlined |
| `silk_NLSF_CB1_WB_Q8` | `silk/tables_NLSF_CB_WB.c:34` | `static const opus_uint8` | `[ 512 ]` | 512 | 512 | 512 | file | — | inlined |
| `silk_NLSF_CB1_WB_Wght_Q9` | `silk/tables_NLSF_CB_WB.c:101` | `static const opus_int16` | `[ 512 ]` | 512 | 512 | 512 | file | — | inlined |
| `silk_NLSF_CB1_iCDF_WB` | `silk/tables_NLSF_CB_WB.c:136` | `static const opus_uint8` | `[ 64 ]` | 64 | 64 | 64 | file | — | inlined |
| `silk_NLSF_CB2_SELECT_WB` | `silk/tables_NLSF_CB_WB.c:147` | `static const opus_uint8` | `[ 256 ]` | 256 | 256 | 256 | file | — | inlined |
| `silk_NLSF_CB2_iCDF_WB` | `silk/tables_NLSF_CB_WB.c:182` | `static const opus_uint8` | `[ 72 ]` | 72 | 72 | 72 | file | — | inlined |
| `silk_NLSF_CB2_BITS_WB_Q5` | `silk/tables_NLSF_CB_WB.c:194` | `static const opus_uint8` | `[ 72 ]` | 72 | 72 | 72 | file | — | inlined |
| `silk_NLSF_PRED_WB_Q8` | `silk/tables_NLSF_CB_WB.c:206` | `static const opus_uint8` | `[ 30 ]` | 30 | 30 | 30 | file | — | inlined |
| `silk_NLSF_DELTA_MIN_WB_Q15` | `silk/tables_NLSF_CB_WB.c:213` | `static const opus_int16` | `[ 17 ]` | 17 | 17 | 17 | file | — | inlined |
| `silk_gain_iCDF` | `silk/tables_gain.c:35` | `const opus_uint8` | `[ 3 ][ N_LEVELS_QGAIN / 8 ]` | — | 3 | 24 | file | — | inlined |
| `silk_delta_gain_iCDF` | `silk/tables_gain.c:48` | `const opus_uint8` | `[ MAX_DELTA_GAIN_QUANT - MIN_DELTA_GAIN_QUANT + 1 ]` | — | 41 | 41 | file | — | inlined |
| `silk_stereo_pred_quant_Q13` | `silk/tables_other.c:38` | `const opus_int16` | `[ STEREO_QUANT_TAB_SIZE ]` | — | 16 | 16 | file | — | inlined |
| `silk_stereo_pred_joint_iCDF` | `silk/tables_other.c:42` | `const opus_uint8` | `[ 25 ]` | 25 | 25 | 25 | file | — | inlined |
| `silk_stereo_only_code_mid_iCDF` | `silk/tables_other.c:49` | `const opus_uint8` | `[ 2 ]` | 2 | 2 | 2 | file | — | inlined |
| `silk_LBRR_flags_2_iCDF` | `silk/tables_other.c:52` | `static const opus_uint8` | `[ 3 ]` | 3 | 3 | 3 | file | — | inlined |
| `silk_LBRR_flags_3_iCDF` | `silk/tables_other.c:53` | `static const opus_uint8` | `[ 7 ]` | 7 | 7 | 7 | file | — | inlined |
| `silk_LBRR_flags_iCDF_ptr` | `silk/tables_other.c:54` | `const opus_uint8 * const` | `[ 2 ]` | 2 | 2 | 2 | file | — | inlined |
| `silk_lsb_iCDF` | `silk/tables_other.c:60` | `const opus_uint8` | `[ 2 ]` | 2 | 2 | 2 | file | — | inlined |
| `silk_LTPscale_iCDF` | `silk/tables_other.c:63` | `const opus_uint8` | `[ 3 ]` | 3 | 3 | 3 | file | — | inlined |
| `silk_type_offset_VAD_iCDF` | `silk/tables_other.c:66` | `const opus_uint8` | `[ 4 ]` | 4 | 4 | 4 | file | — | inlined |
| `silk_type_offset_no_VAD_iCDF` | `silk/tables_other.c:69` | `const opus_uint8` | `[ 2 ]` | 2 | 2 | 2 | file | — | inlined |
| `silk_NLSF_interpolation_factor_iCDF` | `silk/tables_other.c:74` | `const opus_uint8` | `[ 5 ]` | 5 | 5 | 5 | file | — | inlined |
| `silk_Quantization_Offsets_Q10` | `silk/tables_other.c:77` | `const opus_int16` | `[ 2 ][ 2 ]` | 4 | 2 | 4 | file | — | inlined |
| `silk_LTPScales_table_Q14` | `silk/tables_other.c:82` | `const opus_int16` | `[ 3 ]` | 3 | 3 | 3 | file | — | inlined |
| `silk_uniform3_iCDF` | `silk/tables_other.c:85` | `const opus_uint8` | `[ 3 ]` | 3 | 3 | 3 | file | — | inlined |
| `silk_uniform4_iCDF` | `silk/tables_other.c:86` | `const opus_uint8` | `[ 4 ]` | 4 | 4 | 4 | file | — | inlined |
| `silk_uniform5_iCDF` | `silk/tables_other.c:87` | `const opus_uint8` | `[ 5 ]` | 5 | 5 | 5 | file | — | inlined |
| `silk_uniform6_iCDF` | `silk/tables_other.c:88` | `const opus_uint8` | `[ 6 ]` | 6 | 6 | 6 | file | — | inlined |
| `silk_uniform8_iCDF` | `silk/tables_other.c:89` | `const opus_uint8` | `[ 8 ]` | 8 | 8 | 8 | file | — | inlined |
| `silk_NLSF_EXT_iCDF` | `silk/tables_other.c:91` | `const opus_uint8` | `[ 7 ]` | 7 | 7 | 7 | file | — | inlined |
| `silk_Transition_LP_B_Q28` | `silk/tables_other.c:98` | `const opus_int32` | `[ TRANSITION_INT_NUM ][ TRANSITION_NB ]` | — | 5 | 15 | file | — | inlined |
| `silk_Transition_LP_A_Q28` | `silk/tables_other.c:108` | `const opus_int32` | `[ TRANSITION_INT_NUM ][ TRANSITION_NA ]` | — | 5 | 10 | file | — | inlined |
| `silk_pitch_lag_iCDF` | `silk/tables_pitch_lag.c:34` | `const opus_uint8` | `[ 2 * ( PITCH_EST_MAX_LAG_MS - PITCH_EST_MIN_LAG_MS ) ]` | — | 32 | 32 | file | — | inlined |
| `silk_pitch_delta_iCDF` | `silk/tables_pitch_lag.c:41` | `const opus_uint8` | `[21]` | 21 | 21 | 21 | file | — | inlined |
| `silk_pitch_contour_iCDF` | `silk/tables_pitch_lag.c:47` | `const opus_uint8` | `[34]` | 34 | 34 | 34 | file | — | inlined |
| `silk_pitch_contour_NB_iCDF` | `silk/tables_pitch_lag.c:55` | `const opus_uint8` | `[11]` | 11 | 11 | 11 | file | — | inlined |
| `silk_pitch_contour_10_ms_iCDF` | `silk/tables_pitch_lag.c:60` | `const opus_uint8` | `[12]` | 12 | 12 | 12 | file | — | inlined |
| `silk_pitch_contour_10_ms_NB_iCDF` | `silk/tables_pitch_lag.c:65` | `const opus_uint8` | `[3]` | 3 | 3 | 3 | file | — | inlined |
| `silk_max_pulses_table` | `silk/tables_pulses_per_block.c:34` | `const opus_uint8` | `[ 4 ]` | 4 | 4 | 4 | file | — | inlined |
| `silk_pulses_per_block_iCDF` | `silk/tables_pulses_per_block.c:38` | `const opus_uint8` | `[ 10 ][ 18 ]` | 180 | 10 | 180 | file | — | inlined |
| `silk_pulses_per_block_BITS_Q5` | `silk/tables_pulses_per_block.c:91` | `const opus_uint8` | `[ 9 ][ 18 ]` | 162 | 9 | 162 | file | — | inlined |
| `silk_rate_levels_iCDF` | `silk/tables_pulses_per_block.c:139` | `const opus_uint8` | `[ 2 ][ 9 ]` | 18 | 2 | 18 | file | — | inlined |
| `silk_rate_levels_BITS_Q5` | `silk/tables_pulses_per_block.c:151` | `const opus_uint8` | `[ 2 ][ 9 ]` | 18 | 2 | 18 | file | — | inlined |
| `silk_shell_code_table0` | `silk/tables_pulses_per_block.c:163` | `const opus_uint8` | `[ 152 ]` | 152 | 152 | 152 | file | — | inlined |
| `silk_shell_code_table1` | `silk/tables_pulses_per_block.c:185` | `const opus_uint8` | `[ 152 ]` | 152 | 152 | 152 | file | — | inlined |
| `silk_shell_code_table2` | `silk/tables_pulses_per_block.c:207` | `const opus_uint8` | `[ 152 ]` | 152 | 152 | 152 | file | — | inlined |
| `silk_shell_code_table3` | `silk/tables_pulses_per_block.c:229` | `const opus_uint8` | `[ 152 ]` | 152 | 152 | 152 | file | — | inlined |
| `silk_shell_code_table_offsets` | `silk/tables_pulses_per_block.c:251` | `const opus_uint8` | `[ 17 ]` | 17 | 17 | 17 | file | — | inlined |
| `silk_sign_iCDF` | `silk/tables_pulses_per_block.c:257` | `const opus_uint8` | `[ 42 ]` | 42 | 42 | 42 | file | — | inlined |
| `dct_table` | `src/analysis.c:57` | `static const float` | `[128]` | 128 | 128 | 128 | file | `!DISABLE_FLOAT_API` | inlined |
| `analysis_window` | `src/analysis.c:76` | `static const float` | `[240]` | 240 | 240 | 240 | file | `!DISABLE_FLOAT_API` | inlined |
| `tbands` | `src/analysis.c:109` | `static const int` | `[NB_TBANDS+1]` | — | 19 | 19 | file | `!DISABLE_FLOAT_API` | inlined |
| `std_feature_bias` | `src/analysis.c:410` | `static const float` | `[9]` | 9 | 9 | 9 | file | `!DISABLE_FLOAT_API` | inlined |
| `ext` | `src/extensions.c:645` | `opus_extension_data` | `[]` | — | 4 | 16 | function-local | — | inlined |
| `mapping_matrix_foa_mixing_data` | `src/mapping_matrix.c:290` | `const opus_int16` | `[36]` | 36 | 36 | 36 | file | — | elided |
| `mapping_matrix_soa_mixing_data` | `src/mapping_matrix.c:299` | `const opus_int16` | `[121]` | 121 | 121 | 121 | file | — | elided |
| `mapping_matrix_toa_mixing_data` | `src/mapping_matrix.c:319` | `const opus_int16` | `[324]` | 324 | 324 | 324 | file | — | elided |
| `mapping_matrix_fourthoa_mixing_data` | `src/mapping_matrix.c:364` | `const opus_int16` | `[729]` | 729 | 729 | 729 | file | — | elided |
| `mapping_matrix_fifthoa_mixing_data` | `src/mapping_matrix.c:460` | `const opus_int16` | `[1444]` | 1444 | 1444 | 1444 | file | — | elided |
| `mapping_matrix_foa_demixing_data` | `src/mapping_matrix.c:645` | `const opus_int16` | `[36]` | 36 | 36 | 36 | file | — | elided |
| `mapping_matrix_soa_demixing_data` | `src/mapping_matrix.c:654` | `const opus_int16` | `[121]` | 121 | 121 | 121 | file | — | elided |
| `mapping_matrix_toa_demixing_data` | `src/mapping_matrix.c:674` | `const opus_int16` | `[324]` | 324 | 324 | 324 | file | — | elided |
| `mapping_matrix_fourthoa_demixing_data` | `src/mapping_matrix.c:719` | `const opus_int16` | `[729]` | 729 | 729 | 729 | file | — | elided |
| `mapping_matrix_fifthoa_demixing_data` | `src/mapping_matrix.c:815` | `const opus_int16` | `[1444]` | 1444 | 1444 | 1444 | file | — | elided |
| `layer0_weights` | `src/mlp_data.c:9` | `static const opus_int8` | `[800]` | 800 | 800 | 800 | file | — | elided |
| `layer0_bias` | `src/mlp_data.c:112` | `static const opus_int8` | `[32]` | 32 | 32 | 32 | file | — | elided |
| `layer1_weights` | `src/mlp_data.c:119` | `static const opus_int8` | `[2304]` | 2304 | 2304 | 2304 | file | — | elided |
| `layer1_recur_weights` | `src/mlp_data.c:410` | `static const opus_int8` | `[1728]` | 1728 | 1728 | 1728 | file | — | elided |
| `layer1_bias` | `src/mlp_data.c:629` | `static const opus_int8` | `[72]` | 72 | 72 | 72 | file | — | elided |
| `layer2_weights` | `src/mlp_data.c:641` | `static const opus_int8` | `[48]` | 48 | 48 | 48 | file | — | elided |
| `layer2_bias` | `src/mlp_data.c:650` | `static const opus_int8` | `[2]` | 2 | 2 | 2 | file | — | elided |
| `p` | `src/opus_compare.c:122` | `float` | `[2]` | 2 | 1 | 1 | function-local | — | inlined |
| `BANDS` | `src/opus_compare.c:158` | `static const int` | `[NBANDS+1]` | — | 22 | 22 | file | — | inlined |
| `silence` | `src/opus_decoder.c:614` | `unsigned char` | `[2]` | 2 | 2 | 2 | function-local | — | inlined |
| `format_size` | `src/opus_demo.c:162` | `static const int` | `[3]` | 3 | 3 | 3 | file | — | inlined |
| `silk8_test` | `src/opus_demo.c:186` | `static const int` | `[][4]` | — | 8 | 32 | file | — | inlined |
| `silk12_test` | `src/opus_demo.c:197` | `static const int` | `[][4]` | — | 8 | 32 | file | — | inlined |
| `silk16_test` | `src/opus_demo.c:208` | `static const int` | `[][4]` | — | 8 | 32 | file | — | inlined |
| `silk_bw_switch_test` | `src/opus_demo.c:219` | `static const int` | `[][4]` | — | 20 | 80 | file | — | inlined |
| `hybrid24_test` | `src/opus_demo.c:242` | `static const int` | `[][4]` | — | 4 | 16 | file | — | inlined |
| `hybrid48_test` | `src/opus_demo.c:249` | `static const int` | `[][4]` | — | 4 | 16 | file | — | inlined |
| `celt_test` | `src/opus_demo.c:256` | `static const int` | `[][4]` | — | 32 | 128 | file | — | inlined |
| `celt_hq_test` | `src/opus_demo.c:299` | `static const int` | `[][4]` | — | 4 | 16 | file | — | inlined |
| `mapping` | `src/opus_demo.c:328` | `unsigned char` | `[256]` | 256 | 2 | 2 | function-local | — | inlined |
| `bitrates` | `src/opus_demo.c:347` | `static int` | `[NUM_BITRATES]` | — | 102 | 102 | file | `ENABLE_OSCE_TRAINING_DATA` | inlined |
| `mono_voice_bandwidth_thresholds` | `src/opus_encoder.c:151` | `static const opus_int32` | `[8]` | 8 | 8 | 8 | file | — | inlined |
| `mono_music_bandwidth_thresholds` | `src/opus_encoder.c:157` | `static const opus_int32` | `[8]` | 8 | 8 | 8 | file | — | inlined |
| `stereo_voice_bandwidth_thresholds` | `src/opus_encoder.c:163` | `static const opus_int32` | `[8]` | 8 | 8 | 8 | file | — | inlined |
| `stereo_music_bandwidth_thresholds` | `src/opus_encoder.c:169` | `static const opus_int32` | `[8]` | 8 | 8 | 8 | file | — | inlined |
| `mode_thresholds` | `src/opus_encoder.c:180` | `static const opus_int32` | `[2][2]` | 4 | 2 | 4 | file | — | inlined |
| `fec_thresholds` | `src/opus_encoder.c:186` | `static const opus_int32` | `[]` | — | 10 | 10 | file | — | inlined |
| `dred_bits_table` | `src/opus_encoder.c:668` | `static const float` | `[16]` | 16 | 16 | 16 | file | `ENABLE_DRED` | inlined |
| `rate_table` | `src/opus_encoder.c:978` | `static const int` | `[][5]` | — | 7 | 35 | function-local | — | inlined |
| `vorbis_mappings` | `src/opus_multistream_encoder.c:53` | `static const VorbisLayout` | `[8]` | 8 | 8 | 52 | file | — | inlined |
| `diff_table` | `src/opus_multistream_encoder.c:198` | `static const celt_glog` | `[17]` | 17 | 9 | 9 | function-local | — | inlined |
| `pos` | `src/opus_multistream_encoder.c:237` | `int` | `[8]` | 8 | 1 | 1 | function-local | — | inlined |
| `format_size` | `src/qext_compare.c:65` | `static const int` | `[3]` | 3 | 3 | 3 | file | — | inlined |
| `p` | `src/qext_compare.c:159` | `float` | `[2]` | 2 | 1 | 1 | function-local | — | inlined |
| `BANDS` | `src/qext_compare.c:184` | `static const int` | `[NBANDS+1]` | — | 29 | 29 | file | — | inlined |
| `maxE` | `src/qext_compare.c:370` | `float` | `[2]` | 2 | 1 | 1 | function-local | — | inlined |

**Total: 306 constant tables** (137 in `celt/`, 118 in `silk/`, 51 in `src/`), plus 83 `extern` declarations.

### 6.2 CELT core tables

#### `celt/bands.c`

**`tcount`** — `celt/bands.c:490-490` — flat array; **declared 3, 3 top-level group(s), 3 scalar leaf/leaves in the initialiser**; `[3]`; identical in both preprocessor branches; function-local `static`

```c
{0,0,0}
```

**`ordery_table`** — `celt/bands.c:567-572` — flat array; **declared ?, 30 top-level group(s), 30 scalar leaf/leaves in the initialiser**; `[]`; identical in both preprocessor branches

```c
{
       1,  0,
       3,  0,  2,  1,
       7,  0,  4,  3,  6,  1,  5,  2,
      15,  0,  8,  7, 12,  3, 11,  4, 14,  1,  9,  6, 13,  2, 10,  5,
}
```

**`exp2_table8`** — `celt/bands.c:640-641` — flat array; **declared 8, 8 top-level group(s), 8 scalar leaf/leaves in the initialiser**; `[8]`; identical in both preprocessor branches; function-local `static`

```c
{16384, 17866, 19483, 21247, 23170, 25267, 27554, 30048}
```

**`bit_interleave_table`** — `celt/bands.c:1291-1293` — flat array; **declared 16, 16 top-level group(s), 16 scalar leaf/leaves in the initialiser**; `[16]`; identical in both preprocessor branches; function-local `static`

```c
{
            0,1,1,1,2,3,3,3,2,3,3,3,2,3,3,3
      }
```

**`bit_deinterleave_table`** — `celt/bands.c:1357-1360` — flat array; **declared 16, 16 top-level group(s), 16 scalar leaf/leaves in the initialiser**; `[16]`; identical in both preprocessor branches; function-local `static`

```c
{
               0x00,0x03,0x0C,0x0F,0x30,0x33,0x3C,0x3F,
               0xC0,0xC3,0xCC,0xCF,0xF0,0xF3,0xFC,0xFF
         }
```

#### `celt/celt.c`

**`gains`** — `celt/celt.c:246-249` — 2-D array; **declared 9, 3 top-level group(s), 9 scalar leaf/leaves in the initialiser**; `[3][3]`; function-local `static`

```c
{
         {QCONST16(0.3066406250f, 15), QCONST16(0.2170410156f, 15), QCONST16(0.1296386719f, 15)},
         {QCONST16(0.4638671875f, 15), QCONST16(0.2680664062f, 15), QCONST16(0.f, 15)},
         {QCONST16(0.7998046875f, 15), QCONST16(0.1000976562f, 15), QCONST16(0.f, 15)}}
```

**`tf_select_table`** — `celt/celt.c:320-326` — 2-D array; **declared 32, 4 top-level group(s), 32 scalar leaf/leaves in the initialiser**; `[4][8]`; identical in both preprocessor branches

```c
{
    /*isTransient=0     isTransient=1 */
      {0, -1, 0, -1,    0,-1, 0,-1}, /* 2.5 ms */
      {0, -1, 0, -2,    1, 0, 1,-1}, /* 5 ms */
      {0, -2, 0, -3,    2, 0, 1,-1}, /* 10 ms */
      {0, -2, 0, -3,    3, 0, 1,-1}, /* 20 ms */
}
```

**`error_strings`** — `celt/celt.c:344-353` — flat array; **declared 8, 8 top-level group(s), 8 scalar leaf/leaves in the initialiser**; `[8]`; identical in both preprocessor branches; function-local `static`

```c
{
      "success",
      "invalid argument",
      "buffer too small",
      "internal error",
      "corrupted stream",
      "request not implemented",
      "invalid state",
      "memory allocation failed"
   }
```

#### `celt/celt.h`

**`trim_icdf`** — `celt/celt.h:194-194` — flat array; **declared 11, 11 top-level group(s), 11 scalar leaf/leaves in the initialiser**; `[11]`

```c
{126, 124, 119, 109, 87, 41, 19, 9, 4, 2, 0}
```

**`spread_icdf`** — `celt/celt.h:196-196` — flat array; **declared 4, 4 top-level group(s), 4 scalar leaf/leaves in the initialiser**; `[4]`

```c
{25, 23, 2, 0}
```

**`tapset_icdf`** — `celt/celt.h:198-198` — flat array; **declared 3, 3 top-level group(s), 3 scalar leaf/leaves in the initialiser**; `[3]`

```c
{2,1,0}
```

**`toOpusTable`** — `celt/celt.h:201-207` — flat array; **declared 20, 20 top-level group(s), 20 scalar leaf/leaves in the initialiser**; `[20]`; **`#if defined(CUSTOM_MODES) || defined(ENABLE_OPUS_CUSTOM_API)`**

```c
{
      0xE0, 0xE8, 0xF0, 0xF8,
      0xC0, 0xC8, 0xD0, 0xD8,
      0xA0, 0xA8, 0xB0, 0xB8,
      0x00, 0x00, 0x00, 0x00,
      0x80, 0x88, 0x90, 0x98,
}
```

**`fromOpusTable`** — `celt/celt.h:209-214` — flat array; **declared 16, 16 top-level group(s), 16 scalar leaf/leaves in the initialiser**; `[16]`; **`#if defined(CUSTOM_MODES) || defined(ENABLE_OPUS_CUSTOM_API)`**

```c
{
      0x80, 0x88, 0x90, 0x98,
      0x40, 0x48, 0x50, 0x58,
      0x20, 0x28, 0x30, 0x38,
      0x00, 0x08, 0x10, 0x18
}
```

#### `celt/celt_decoder.c`

**`sinc_filter`** — `celt/celt_decoder.c:629-637` — flat array; **declared ?, 49 top-level group(s), 49 scalar leaf/leaves in the initialiser**; `[SINC_ORDER+1]`; **`#if ENABLE_DEEP_PLC`**

```c
{
    4.2931e-05f, -0.000190293f, -0.000816132f, -0.000637162f, 0.00141662f, 0.00354764f, 0.00184368f, -0.00428274f,
    -0.00856105f, -0.0034003f, 0.00930201f, 0.0159616f, 0.00489785f, -0.0169649f, -0.0259484f, -0.00596856f,
    0.0286551f, 0.0405872f, 0.00649994f, -0.0509284f, -0.0716655f, -0.00665212f,  0.134336f,  0.278927f,
    0.339995f,  0.278927f,  0.134336f, -0.00665212f, -0.0716655f, -0.0509284f, 0.00649994f, 0.0405872f,
    0.0286551f, -0.00596856f, -0.0259484f, -0.0169649f, 0.00489785f, 0.0159616f, 0.00930201f, -0.0034003f,
    -0.00856105f, -0.00428274f, 0.00184368f, 0.00354764f, 0.00141662f, -0.000637162f, -0.000816132f, -0.000190293f,
    4.2931e-05f
}
```

#### `celt/celt_encoder.c`

**`inv_table`** — `celt/celt_encoder.c:286-295` — flat array; **declared 128, 128 top-level group(s), 128 scalar leaf/leaves in the initialiser**; `[128]`; identical in both preprocessor branches; function-local `static`

```c
{
         255,255,156,110, 86, 70, 59, 51, 45, 40, 37, 33, 31, 28, 26, 25,
          23, 22, 21, 20, 19, 18, 17, 16, 16, 15, 15, 14, 13, 13, 12, 12,
          12, 12, 11, 11, 11, 10, 10, 10,  9,  9,  9,  9,  9,  9,  8,  8,
           8,  8,  8,  7,  7,  7,  7,  7,  7,  6,  6,  6,  6,  6,  6,  6,
           6,  6,  6,  6,  6,  6,  6,  6,  6,  5,  5,  5,  5,  5,  5,  5,
           5,  5,  5,  5,  5,  4,  4,  4,  4,  4,  4,  4,  4,  4,  4,  4,
           4,  4,  4,  4,  4,  4,  4,  4,  4,  4,  4,  4,  4,  4,  3,  3,
           3,  3,  3,  3,  3,  3,  3,  3,  3,  3,  3,  3,  3,  3,  3,  2,
   }
```

**`before`** — `celt/celt_encoder.c:1419-1419` — flat array; **declared 2, 1 top-level group(s), 1 scalar leaf/leaves in the initialiser**; `[2]`; identical in both preprocessor branches; function-local `static`

```c
{0}
```

**`spread_thresholds`** — `celt/celt_encoder.c:2329-2329` — flat array; **declared 3, 3 top-level group(s), 3 scalar leaf/leaves in the initialiser**; `[3]`; function-local `static`

```c
{-QCONST16(.6f, 15), -QCONST16(.2f, 15), -QCONST16(.07f, 15)}
```

**`spread_histeresis`** — `celt/celt_encoder.c:2330-2330` — flat array; **declared 3, 3 top-level group(s), 3 scalar leaf/leaves in the initialiser**; `[3]`; function-local `static`

```c
{QCONST16(.15f, 15), QCONST16(.07f, 15), QCONST16(.02f, 15)}
```

**`tapset_thresholds`** — `celt/celt_encoder.c:2331-2331` — flat array; **declared 2, 2 top-level group(s), 2 scalar leaf/leaves in the initialiser**; `[2]`; function-local `static`

```c
{QCONST16(.0f, 15), QCONST16(.15f, 15)}
```

**`tapset_histeresis`** — `celt/celt_encoder.c:2332-2332` — flat array; **declared 2, 2 top-level group(s), 2 scalar leaf/leaves in the initialiser**; `[2]`; function-local `static`

```c
{QCONST16(.1f, 15), QCONST16(.05f, 15)}
```

**`intensity_thresholds`** — `celt/celt_encoder.c:2393-2395` — flat array; **declared 21, 21 top-level group(s), 21 scalar leaf/leaves in the initialiser**; `[21]`; identical in both preprocessor branches; function-local `static`

```c
{  1, 2, 3, 4, 5, 6, 7, 8,16,24,36,44,50,56,62,67,72,79,88,106,134}
```

**`intensity_histeresis`** — `celt/celt_encoder.c:2396-2397` — flat array; **declared 21, 21 top-level group(s), 21 scalar leaf/leaves in the initialiser**; `[21]`; identical in both preprocessor branches; function-local `static`

```c
{  1, 1, 1, 1, 1, 1, 1, 2, 2, 2, 2, 2, 2, 2, 3, 3, 4, 5, 6,  8, 8}
```

#### `celt/cwrs.c`

**`CELT_PVQ_U_DATA`** — `celt/cwrs.c:215-414` — flat array; **declared 1488, 1488 top-level group(s), 1488 scalar leaf/leaves in the initialiser**; `[1488]`; **`#if !defined(SMALL_FOOTPRINT) && defined(CWRS_EXTRA_ROWS)`**

```c
{
#else
static const opus_uint32 CELT_PVQ_U_DATA[1272]={
#endif
  /*N=0, K=0...176:*/
  1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
  0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
  0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
  0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
  0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
  0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
  0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
#if defined(CWRS_EXTRA_ROWS)
  /*...208:*/
  0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
  0, 0, 0, 0, 0, 0,
#endif
  /*N=1, K=1...176:*/
  1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
  1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
  1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
  1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
  1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
  1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
  1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
#if defined(CWRS_EXTRA_ROWS)
  /*...208:*/
  1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
  1, 1, 1, 1, 1, 1,
#endif
  /*N=2, K=2...176:*/
  3, 5, 7, 9, 11, 13, 15, 17, 19, 21, 23, 25, 27, 29, 31, 33, 35, 37, 39, 41,
  43, 45, 47, 49, 51, 53, 55, 57, 59, 61, 63, 65, 67, 69, 71, 73, 75, 77, 79,
  81, 83, 85, 87, 89, 91, 93, 95, 97, 99, 101, 103, 105, 107, 109, 111, 113,
  115, 117, 119, 121, 123, 125, 127, 129, 131, 133, 135, 137, 139, 141, 143,
  145, 147, 149, 151, 153, 155, 157, 159, 161, 163, 165, 167, 169, 171, 173,
  175, 177, 179, 181, 183, 185, 187, 189, 191, 193, 195, 197, 199, 201, 203,
  205, 207, 209, 211, 213, 215, 217, 219, 221, 223, 225, 227, 229, 231, 233,
  235, 237, 239, 241, 243, 245, 247, 249, 251, 253, 255, 257, 259, 261, 263,
  265, 267, 269, 271, 273, 275, 277, 279, 281, 283, 285, 287, 289, 291, 293,
  295, 297, 299, 301, 303, 305, 307, 309, 311, 313, 315, 317, 319, 321, 323,
  325, 327, 329, 331, 333, 335, 337, 339, 341, 343, 345, 347, 349, 351,
#if defined(CWRS_EXTRA_ROWS)
  /*...208:*/
  353, 355, 357, 359, 361, 363, 365, 367, 369, 371, 373, 375, 377, 379, 381,
  383, 385, 387, 389, 391, 393, 395, 397, 399, 401, 403, 405, 407, 409, 411,
  413, 415,
#endif
  /*N=3, K=3...176:*/
  13, 25, 41, 61, 85, 113, 145, 181, 221, 265, 313, 365, 421, 481, 545, 613,
  685, 761, 841, 925, 1013, 1105, 1201, 1301, 1405, 1513, 1625, 1741, 1861,
  1985, 2113, 2245, 2381, 2521, 2665, 2813, 2965, 3121, 3281, 3445, 3613, 3785,
  3961, 4141, 4325, 4513, 4705, 4901, 5101, 5305, 5513, 5725, 5941, 6161, 6385,
  6613, 6845, 7081, 7321, 7565, 7813, 8065, 8321, 8581, 8845, 9113, 9385, 9661,
  9941, 10225, 10513, 10805, 11101, 11401, 11705, 12013, 12325, 12641, 12961,
  13285, 13613, 13945, 14281, 14621, 14965, 15313, 15665, 16021, 16381, 16745,
  17113, 17485, 17861, 18241, 18625, 19013, 19405, 19801, 20201, 20605, 21013,
  21425, 21841, 22261, 22685, 23113, 23545, 23981, 24421, 24865, 25313, 25765,
  26221, 26681, 27145, 27613, 28085, 28561, 29041, 29525, 30013, 30505, 31001,
  31501, 32005, 32513, 33025, 33541, 34061, 34585, 35113, 35645, 36181, 36721,
  37265, 37813, 38365, 38921, 39481, 40045, 40613, 41185, 41761, 42341, 42925,
  43513, 44105, 44701, 45301, 45905, 46513, 47125, 47741, 48361, 48985, 49613,
  50245, 50881, 51521, 52165, 52813, 53465, 54121, 54781, 55445, 56113, 56785,
  57461, 58141, 58825, 59513, 60205, 60901, 61601,
#if defined(CWRS_EXTRA_ROWS)
  /*...208:*/
  62305, 63013, 63725, 64441, 65161, 65885, 66613, 67345, 68081, 68821, 69565,
  70313, 71065, 71821, 72581, 73345, 74113, 74885, 75661, 76441, 77225, 78013,
  78805, 79601, 80401, 81205, 82013, 82825, 83641, 84461, 85285, 86113,
#endif
  /*N=4, K=4...176:*/
  63, 129, 231, 377, 575, 833, 1159, 1561, 2047, 2625, 3303, 4089, 4991, 6017,
  7175, 8473, 9919, 11521, 13287, 15225, 17343, 19649, 22151, 24857, 27775,
  30913, 34279, 37881, 41727, 45825, 50183, 54809, 59711, 64897, 70375, 76153,
  82239, 88641, 95367, 102425, 109823, 117569, 125671, 134137, 142975, 152193,
  161799, 171801, 182207, 193025, 204263, 215929, 228031, 240577, 253575,
  267033, 280959, 295361, 310247, 325625, 341503, 357889, 374791, 392217,
  410175, 428673, 447719, 467321, 487487, 508225, 529543, 551449, 573951,
  597057, 620775, 645113, 670079, 695681, 721927, 748825, 776383, 804609,
  833511, 863097, 893375, 924353, 956039, 988441, 1021567, 1055425, 1090023,
  1125369, 1161471, 1198337, 1235975, 1274393, 1313599, 1353601, 1394407,
  1436025, 1478463, 1521729, 1565831, 1610777, 1656575, 1703233, 1750759,
  1799161, 1848447, 1898625, 1949703, 2001689, 2054591, 2108417, 2163175,
  2218873, 2275519, 2333121, 2391687, 2451225, 2511743, 2573249, 2635751,
  2699257, 2763775, 2829313, 2895879, 2963481, 3032127, 3101825, 3172583,
  3244409, 3317311, 3391297, 3466375, 3542553, 3619839, 3698241, 3777767,
  3858425, 3940223, 4023169, 4107271, 4192537, 4278975, 4366593, 4455399,
  4545401, 4636607, 4729025, 4822663, 4917529, 5013631, 5110977, 5209575,
  5309433, 5410559, 5512961, 5616647, 5721625, 5827903, 5935489, 6044391,
  6154617, 6266175, 6379073, 6493319, 6608921, 6725887, 6844225, 6963943,
  7085049, 7207551,
#if defined(CWRS_EXTRA_ROWS)
  /*...208:*/
  7331457, 7456775, 7583513, 7711679, 7841281, 7972327, 8104825, 8238783,
  8374209, 8511111, 8649497, 8789375, 8930753, 9073639, 9218041, 9363967,
  9511425, 9660423, 9810969, 9963071, 10116737, 10271975, 10428793, 10587199,
  10747201, 10908807, 11072025, 11236863, 11403329, 11571431, 11741177,
  11912575,
#endif
  /*N=5, K=5...176:*/
  321, 681, 1289, 2241, 3649, 5641, 8361, 11969, 16641, 22569, 29961, 39041,
  50049, 63241, 78889, 97281, 118721, 143529, 172041, 204609, 241601, 283401,
  330409, 383041, 441729, 506921, 579081, 658689, 746241, 842249, 947241,
  1061761, 1186369, 1321641, 1468169, 1626561, 1797441, 1981449, 2179241,
  2391489, 2618881, 2862121, 3121929, 3399041, 3694209, 4008201, 4341801,
  4695809, 5071041, 5468329, 5888521, 6332481, 6801089, 7295241, 7815849,
  8363841, 8940161, 9545769, 10181641, 10848769, 11548161, 12280841, 13047849,
  13850241, 14689089, 15565481, 16480521, 17435329, 18431041, 19468809,
  20549801, 21675201, 22846209, 24064041, 25329929, 26645121, 28010881,
  29428489, 30899241, 32424449, 34005441, 35643561, 37340169, 39096641,
  40914369, 42794761, 44739241, 46749249, 48826241, 50971689, 53187081,
  55473921, 57833729, 60268041, 62778409, 65366401, 68033601, 70781609,
  73612041, 76526529, 79526721, 82614281, 85790889, 89058241, 92418049,
  95872041, 99421961, 103069569, 106816641, 110664969, 114616361, 118672641,
  122835649, 127107241, 131489289, 135983681, 140592321, 145317129, 150160041,
  155123009, 160208001, 165417001, 170752009, 176215041, 181808129, 187533321,
  193392681, 199388289, 205522241, 211796649, 218213641, 224775361, 231483969,
  238341641, 245350569, 252512961, 259831041, 267307049, 274943241, 282741889,
  290705281, 298835721, 307135529, 315607041, 324252609, 333074601, 342075401,
  351257409, 360623041, 370174729, 379914921, 389846081, 399970689, 410291241,
  420810249, 431530241, 442453761, 453583369, 464921641, 476471169, 488234561,
  500214441, 512413449, 524834241, 537479489, 550351881, 563454121, 576788929,
  590359041, 604167209, 618216201, 632508801,
#if defined(CWRS_EXTRA_ROWS)
  /*...208:*/
  647047809, 661836041, 676876329, 692171521, 707724481, 723538089, 739615241,
  755958849, 772571841, 789457161, 806617769, 824056641, 841776769, 859781161,
  878072841, 896654849, 915530241, 934702089, 954173481, 973947521, 994027329,
  1014416041, 1035116809, 1056132801, 1077467201, 1099123209, 1121104041,
  1143412929, 1166053121, 1189027881, 1212340489, 1235994241,
#endif
  /*N=6, K=6...96:*/
  1683, 3653, 7183, 13073, 22363, 36365, 56695, 85305, 124515, 177045, 246047,
  335137, 448427, 590557, 766727, 982729, 1244979, 1560549, 1937199, 2383409,
  2908411, 3522221, 4235671, 5060441, 6009091, 7095093, 8332863, 9737793,
  11326283, 13115773, 15124775, 17372905, 19880915, 22670725, 25765455,
  29189457, 32968347, 37129037, 41699767, 46710137, 52191139, 58175189,
  64696159, 71789409, 79491819, 87841821, 96879431, 106646281, 117185651,
  128542501, 140763503, 153897073, 167993403, 183104493, 199284183, 216588185,
  235074115, 254801525, 275831935, 298228865, 322057867, 347386557, 374284647,
  402823977, 433078547, 465124549, 499040399, 534906769, 572806619, 612825229,
  655050231, 699571641, 746481891, 795875861, 847850911, 902506913, 959946283,
  1020274013, 1083597703, 1150027593, 1219676595, 1292660325, 1369097135,
  1449108145, 1532817275, 1620351277, 1711839767, 1807415257, 1907213187,
  2011371957, 2120032959,
#if defined(CWRS_EXTRA_ROWS)
  /*...109:*/
  2233340609U, 2351442379U, 2474488829U, 2602633639U, 2736033641U, 2874848851U,
  3019242501U, 3169381071U, 3325434321U, 3487575323U, 3655980493U, 3830829623U,
  4012305913U,
#endif
  /*N=7, K=7...54*/
  8989, 19825, 40081, 75517, 134245, 227305, 369305, 579125, 880685, 1303777,
  1884961, 2668525, 3707509, 5064793, 6814249, 9041957, 11847485, 15345233,
  19665841, 24957661, 31388293, 39146185, 48442297, 59511829, 72616013,
  88043969, 106114625, 127178701, 151620757, 179861305, 212358985, 249612805,
  292164445, 340600625, 395555537, 457713341, 527810725, 606639529, 695049433,
  793950709, 904317037, 1027188385, 1163673953, 1314955181, 1482288821,
  1667010073, 1870535785, 2094367717,
#if defined(CWRS_EXTRA_ROWS)
  /*...60:*/
  2340095869U, 2609401873U, 2904062449U, 3225952925U, 3577050821U, 3959439497U,
#endif
  /*N=8, K=8...37*/
  48639, 108545, 224143, 433905, 795455, 1392065, 2340495, 3800305, 5984767,
  9173505, 13726991, 20103025, 28875327, 40754369, 56610575, 77500017,
  104692735, 139703809, 184327311, 240673265, 311207743, 398796225, 506750351,
  638878193, 799538175, 993696769, 1226990095, 1505789553, 1837271615,
  2229491905U,
#if defined(CWRS_EXTRA_ROWS)
  /*...40:*/
  2691463695U, 3233240945U, 3866006015U,
#endif
  /*N=9, K=9...28:*/
  265729, 598417, 1256465, 2485825, 4673345, 8405905, 14546705, 24331777,
  39490049, 62390545, 96220561, 145198913, 214828609, 312193553, 446304145,
  628496897, 872893441, 1196924561, 1621925137, 2173806145U,
#if defined(CWRS_EXTRA_ROWS)
  /*...29:*/
  2883810113U,
#endif
  /*N=10, K=10...24:*/
  1462563, 3317445, 7059735, 14218905, 27298155, 50250765, 89129247, 152951073,
  254831667, 413442773, 654862247, 1014889769, 1541911931, 2300409629U,
  3375210671U,
  /*N=11, K=11...19:*/
  8097453, 18474633, 39753273, 81270333, 158819253, 298199265, 540279585,
  948062325, 1616336765,
#if defined(CWRS_EXTRA_ROWS)
  /*...20:*/
  2684641785U,
#endif
  /*N=12, K=12...18:*/
  45046719, 103274625, 224298231, 464387817, 921406335, 1759885185,
  3248227095U,
  /*N=13, K=13...16:*/
  251595969, 579168825, 1267854873, 2653649025U,
  /*N=14, K=14:*/
  1409933619
}
```

**`CELT_PVQ_U_ROW`** — `celt/cwrs.c:417-423` — flat array; **declared 15, 15 top-level group(s), 15 scalar leaf/leaves in the initialiser**; `[15]`; **`#if !defined(SMALL_FOOTPRINT) && defined(CWRS_EXTRA_ROWS)`**

```c
{
  CELT_PVQ_U_DATA+   0,CELT_PVQ_U_DATA+ 208,CELT_PVQ_U_DATA+ 415,
  CELT_PVQ_U_DATA+ 621,CELT_PVQ_U_DATA+ 826,CELT_PVQ_U_DATA+1030,
  CELT_PVQ_U_DATA+1233,CELT_PVQ_U_DATA+1336,CELT_PVQ_U_DATA+1389,
  CELT_PVQ_U_DATA+1421,CELT_PVQ_U_DATA+1441,CELT_PVQ_U_DATA+1455,
  CELT_PVQ_U_DATA+1464,CELT_PVQ_U_DATA+1470,CELT_PVQ_U_DATA+1473
}
```

#### `celt/entcode.c`

**`correction`** — `celt/entcode.c:70-72` — flat array; **declared 8, 8 top-level group(s), 8 scalar leaf/leaves in the initialiser**; `[8]`; function-local `static`

```c
{35733, 38967, 42495, 46340,
     50535, 55109, 60097, 65535}
```

**`SMALL_DIV_TABLE`** — `celt/entcode.c:119-152` — flat array; **declared 129, 128 top-level group(s), 128 scalar leaf/leaves in the initialiser**; `[129]`; **`#if USE_SMALL_DIV_TABLE`**

```c
{
   0xFFFFFFFF, 0x55555555, 0x33333333, 0x24924924,
   0x1C71C71C, 0x1745D174, 0x13B13B13, 0x11111111,
   0x0F0F0F0F, 0x0D79435E, 0x0C30C30C, 0x0B21642C,
   0x0A3D70A3, 0x097B425E, 0x08D3DCB0, 0x08421084,
   0x07C1F07C, 0x07507507, 0x06EB3E45, 0x06906906,
   0x063E7063, 0x05F417D0, 0x05B05B05, 0x0572620A,
   0x05397829, 0x05050505, 0x04D4873E, 0x04A7904A,
   0x047DC11F, 0x0456C797, 0x04325C53, 0x04104104,
   0x03F03F03, 0x03D22635, 0x03B5CC0E, 0x039B0AD1,
   0x0381C0E0, 0x0369D036, 0x03531DEC, 0x033D91D2,
   0x0329161F, 0x03159721, 0x03030303, 0x02F14990,
   0x02E05C0B, 0x02D02D02, 0x02C0B02C, 0x02B1DA46,
   0x02A3A0FD, 0x0295FAD4, 0x0288DF0C, 0x027C4597,
   0x02702702, 0x02647C69, 0x02593F69, 0x024E6A17,
   0x0243F6F0, 0x0239E0D5, 0x02302302, 0x0226B902,
   0x021D9EAD, 0x0214D021, 0x020C49BA, 0x02040810,
   0x01FC07F0, 0x01F44659, 0x01ECC07B, 0x01E573AC,
   0x01DE5D6E, 0x01D77B65, 0x01D0CB58, 0x01CA4B30,
   0x01C3F8F0, 0x01BDD2B8, 0x01B7D6C3, 0x01B20364,
   0x01AC5701, 0x01A6D01A, 0x01A16D3F, 0x019C2D14,
   0x01970E4F, 0x01920FB4, 0x018D3018, 0x01886E5F,
   0x0183C977, 0x017F405F, 0x017AD220, 0x01767DCE,
   0x01724287, 0x016E1F76, 0x016A13CD, 0x01661EC6,
   0x01623FA7, 0x015E75BB, 0x015AC056, 0x01571ED3,
   0x01539094, 0x01501501, 0x014CAB88, 0x0149539E,
   0x01460CBC, 0x0142D662, 0x013FB013, 0x013C995A,
   0x013991C2, 0x013698DF, 0x0133AE45, 0x0130D190,
   0x012E025C, 0x012B404A, 0x01288B01, 0x0125E227,
   0x01234567, 0x0120B470, 0x011E2EF3, 0x011BB4A4,
   0x01194538, 0x0116E068, 0x011485F0, 0x0112358E,
   0x010FEF01, 0x010DB20A, 0x010B7E6E, 0x010953F3,
   0x01073260, 0x0105197F, 0x0103091B, 0x01010101
}
```

#### `celt/laplace.c`

**`val`** — `celt/laplace.c:205-205` — flat array; **declared ?, 10 top-level group(s), 10 scalar leaf/leaves in the initialiser**; `[NB_VALS]`; function-local `static`

```c
{6, 7, 8, 9, 10, 11, 12, 13, 14, 15}
```

#### `celt/mathops.c`

**`C`** — `celt/mathops.c:148-148` — flat array; **declared 6, 6 top-level group(s), 6 scalar leaf/leaves in the initialiser**; `[6]`; **`#if FIXED_POINT`**; function-local `static`

```c
{23171, 11574, -2901, 1592, -1002, 336}
```

#### `celt/mathops.h`

**`log2_x_norm_coeff`** — `celt/mathops.h:252-256` — flat array; **declared 8, 8 top-level group(s), 8 scalar leaf/leaves in the initialiser**; `[8]`; **`#if !FIXED_POINT && FLOAT_APPROX`**

```c
{
   1.000000000000000000000000000f, 8.88888895511627197265625e-01f,
   8.00000000000000000000000e-01f, 7.27272748947143554687500e-01f,
   6.66666686534881591796875e-01f, 6.15384638309478759765625e-01f,
   5.71428596973419189453125e-01f, 5.33333361148834228515625e-01f}
```

**`log2_y_norm_coeff`** — `celt/mathops.h:266-270` — flat array; **declared 8, 8 top-level group(s), 8 scalar leaf/leaves in the initialiser**; `[8]`; **`#if !FIXED_POINT && FLOAT_APPROX`**

```c
{
   0.0000000000000000000000000000f, 1.699250042438507080078125e-01f,
   3.219280838966369628906250e-01f, 4.594316184520721435546875e-01f,
   5.849624872207641601562500e-01f, 7.004396915435791015625000e-01f,
   8.073549270629882812500000e-01f, 9.068905711174011230468750e-01f}
```

**`C`** — `celt/mathops.h:398-398` — flat array; **declared 5, 5 top-level group(s), 5 scalar leaf/leaves in the initialiser**; `[5]`; **`#if FIXED_POINT`**; function-local `static`

```c
{-6801+(1<<(13-10)), 15746, -5217, 2545, -1401}
```

**`log2_x_norm_coeff`** — `celt/mathops.h:450-452` — flat array; **declared 8, 8 top-level group(s), 8 scalar leaf/leaves in the initialiser**; `[8]`; **`#if FIXED_POINT && ENABLE_QEXT`**; function-local `static`

```c
{
      1073741824, 954437184, 858993472, 780903168,
      715827904,  660764224, 613566784, 572662336}
```

**`log2_y_norm_coeff`** — `celt/mathops.h:454-456` — flat array; **declared 8, 8 top-level group(s), 8 scalar leaf/leaves in the initialiser**; `[8]`; **`#if FIXED_POINT && ENABLE_QEXT`**; function-local `static`

```c
{
      0,       2850868,  5401057,  7707983,
      9814042, 11751428, 13545168, 15215099}
```

#### `celt/modes.c`

**`eband5ms`** — `celt/modes.c:42-45` — flat array; **declared ?, 22 top-level group(s), 22 scalar leaf/leaves in the initialiser**; `[]`; identical in both preprocessor branches

```c
{
/*0  200 400 600 800  1k 1.2 1.4 1.6  2k 2.4 2.8 3.2  4k 4.8 5.6 6.8  8k 9.6 12k 15.6 */
  0,  1,  2,  3,  4,  5,  6,  7,  8, 10, 12, 14, 16, 20, 24, 28, 34, 40, 48, 60, 78, 100
}
```

**`band_allocation`** — `celt/modes.c:50-63` — flat array; **declared ?, 231 top-level group(s), 231 scalar leaf/leaves in the initialiser**; `[]`; identical in both preprocessor branches

```c
{
/*0  200 400 600 800  1k 1.2 1.4 1.6  2k 2.4 2.8 3.2  4k 4.8 5.6 6.8  8k 9.6 12k 15.6 */
  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,
 90, 80, 75, 69, 63, 56, 49, 40, 34, 29, 20, 18, 10,  0,  0,  0,  0,  0,  0,  0,  0,
110,100, 90, 84, 78, 71, 65, 58, 51, 45, 39, 32, 26, 20, 12,  0,  0,  0,  0,  0,  0,
118,110,103, 93, 86, 80, 75, 70, 65, 59, 53, 47, 40, 31, 23, 15,  4,  0,  0,  0,  0,
126,119,112,104, 95, 89, 83, 78, 72, 66, 60, 54, 47, 39, 32, 25, 17, 12,  1,  0,  0,
134,127,120,114,103, 97, 91, 85, 78, 72, 66, 60, 54, 47, 41, 35, 29, 23, 16, 10,  1,
144,137,130,124,113,107,101, 95, 88, 82, 76, 70, 64, 57, 51, 45, 39, 33, 26, 15,  1,
152,145,138,132,123,117,111,105, 98, 92, 86, 80, 74, 67, 61, 55, 49, 43, 36, 20,  1,
162,155,148,142,133,127,121,115,108,102, 96, 90, 84, 77, 71, 65, 59, 53, 46, 30,  1,
172,165,158,152,143,137,131,125,118,112,106,100, 94, 87, 81, 75, 69, 63, 56, 45, 20,
200,200,200,200,200,200,200,200,198,193,188,183,178,173,168,163,158,153,148,129,104,
}
```

**`bark_freq`** — `celt/modes.c:82-88` — flat array; **declared ?, 26 top-level group(s), 26 scalar leaf/leaves in the initialiser**; `[BARK_BANDS+1]`; **`#if CUSTOM_MODES`**

```c
{
      0,   100,   200,   300,   400,
    510,   630,   770,   920,  1080,
   1270,  1480,  1720,  2000,  2320,
   2700,  3150,  3700,  4400,  5300,
   6400,  7700,  9500, 12000, 15500,
  20000}
```

**`qext_eBands_180`** — `celt/modes.c:486-489` — flat array; **declared ?, 15 top-level group(s), 15 scalar leaf/leaves in the initialiser**; `[]`; **`#if ENABLE_QEXT`**

```c
{
/* 20k  22k  24k  26k  28k  30k  32k  34k  36k  38k  40k  42k  44k  47k  48k */
    74,  82,  90,  98, 106, 114, 122, 130, 138, 146, 154, 162, 168, 174, 180
}
```

**`qext_logN_180`** — `celt/modes.c:491-491` — flat array; **declared ?, 15 top-level group(s), 15 scalar leaf/leaves in the initialiser**; `[]`; **`#if ENABLE_QEXT`**

```c
{24, 24, 24, 24, 24, 24, 24, 24, 24, 24, 24, 24, 21, 21, 21}
```

**`qext_eBands_240`** — `celt/modes.c:494-497` — flat array; **declared ?, 15 top-level group(s), 15 scalar leaf/leaves in the initialiser**; `[]`; **`#if ENABLE_QEXT`**

```c
{
/* 20k  22k  24k  26k  28k  30k  32k  34k  36k  38k  40k  42k  44k  47k  48k */
   100, 110, 120, 130, 140, 150, 160, 170, 180, 190, 200, 210, 220, 230, 240
}
```

**`qext_logN_240`** — `celt/modes.c:499-499` — flat array; **declared ?, 14 top-level group(s), 14 scalar leaf/leaves in the initialiser**; `[]`; **`#if ENABLE_QEXT`**

```c
{27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27, 27}
```

#### `celt/opus_custom_demo.c`

**`format_size`** — `celt/opus_custom_demo.c:97-97` — flat array; **declared 3, 3 top-level group(s), 3 scalar leaf/leaves in the initialiser**; `[3]`; identical in both preprocessor branches

```c
{2, 3, 4}
```

#### `celt/pitch.c`

**`sum (the `#else` variant)`** — `celt/pitch.c:269-269` — flat array; **declared 4, 4 top-level group(s), 4 scalar leaf/leaves in the initialiser**; `[4]`; `#else` branch only; function-local `static`

```c
{0,0,0,0}
```

**`best_pitch`** — `celt/pitch.c:312-312` — flat array; **declared 2, 2 top-level group(s), 2 scalar leaf/leaves in the initialiser**; `[2]`; identical in both preprocessor branches; function-local `static`

```c
{0,0}
```

**`second_check`** — `celt/pitch.c:453-453` — flat array; **declared 16, 16 top-level group(s), 16 scalar leaf/leaves in the initialiser**; `[16]`; identical in both preprocessor branches

```c
{0, 0, 3, 2, 3, 2, 5, 2, 3, 2, 3, 2, 5, 2, 3, 2}
```

#### `celt/quant_bands.c`

**`eMeans`** — `celt/quant_bands.c:44-50` — flat array; **declared 25, 25 top-level group(s), 25 scalar leaf/leaves in the initialiser**; `[25]`; **`#if FIXED_POINT`**

```c
{
      103,100, 92, 85, 81,
       77, 72, 70, 78, 75,
       73, 71, 78, 74, 69,
       72, 70, 74, 76, 71,
       60, 60, 60, 60, 60
}
```

**`eMeans (the `#else` variant)`** — `celt/quant_bands.c:53-59` — flat array; **declared 25, 25 top-level group(s), 25 scalar leaf/leaves in the initialiser**; `[25]`; **`#if !(FIXED_POINT)`**; `#else` branch only

```c
{
      6.437500f, 6.250000f, 5.750000f, 5.312500f, 5.062500f,
      4.812500f, 4.500000f, 4.375000f, 4.875000f, 4.687500f,
      4.562500f, 4.437500f, 4.875000f, 4.625000f, 4.312500f,
      4.500000f, 4.375000f, 4.625000f, 4.750000f, 4.437500f,
      3.750000f, 3.750000f, 3.750000f, 3.750000f, 3.750000f
}
```

**`pred_coef`** — `celt/quant_bands.c:63-63` — flat array; **declared 4, 4 top-level group(s), 4 scalar leaf/leaves in the initialiser**; `[4]`; **`#if FIXED_POINT`**

```c
{29440, 26112, 21248, 16384}
```

**`beta_coef`** — `celt/quant_bands.c:64-64` — flat array; **declared 4, 4 top-level group(s), 4 scalar leaf/leaves in the initialiser**; `[4]`; **`#if FIXED_POINT`**

```c
{30147, 22282, 12124, 6554}
```

**`pred_coef (the `#else` variant)`** — `celt/quant_bands.c:67-67` — flat array; **declared 4, 4 top-level group(s), 4 scalar leaf/leaves in the initialiser**; `[4]`; **`#if !(FIXED_POINT)`**; `#else` branch only

```c
{29440/32768., 26112/32768., 21248/32768., 16384/32768.}
```

**`beta_coef (the `#else` variant)`** — `celt/quant_bands.c:68-68` — flat array; **declared 4, 4 top-level group(s), 4 scalar leaf/leaves in the initialiser**; `[4]`; **`#if !(FIXED_POINT)`**; `#else` branch only

```c
{30147/32768., 22282/32768., 12124/32768., 6554/32768.}
```

**`e_prob_model`** — `celt/quant_bands.c:77-138` — 3-D array; **declared 336, 4 top-level group(s), 336 scalar leaf/leaves in the initialiser**; `[4][2][42]`; identical in both preprocessor branches

```c
{
   /*120 sample frames.*/
   {
      /*Inter*/
      {
          72, 127,  65, 129,  66, 128,  65, 128,  64, 128,  62, 128,  64, 128,
          64, 128,  92,  78,  92,  79,  92,  78,  90,  79, 116,  41, 115,  40,
         114,  40, 132,  26, 132,  26, 145,  17, 161,  12, 176,  10, 177,  11
      },
      /*Intra*/
      {
          24, 179,  48, 138,  54, 135,  54, 132,  53, 134,  56, 133,  55, 132,
          55, 132,  61, 114,  70,  96,  74,  88,  75,  88,  87,  74,  89,  66,
          91,  67, 100,  59, 108,  50, 120,  40, 122,  37,  97,  43,  78,  50
      }
   },
   /*240 sample frames.*/
   {
      /*Inter*/
      {
          83,  78,  84,  81,  88,  75,  86,  74,  87,  71,  90,  73,  93,  74,
          93,  74, 109,  40, 114,  36, 117,  34, 117,  34, 143,  17, 145,  18,
         146,  19, 162,  12, 165,  10, 178,   7, 189,   6, 190,   8, 177,   9
      },
      /*Intra*/
      {
          23, 178,  54, 115,  63, 102,  66,  98,  69,  99,  74,  89,  71,  91,
          73,  91,  78,  89,  86,  80,  92,  66,  93,  64, 102,  59, 103,  60,
         104,  60, 117,  52, 123,  44, 138,  35, 133,  31,  97,  38,  77,  45
      }
   },
   /*480 sample frames.*/
   {
      /*Inter*/
      {
          61,  90,  93,  60, 105,  42, 107,  41, 110,  45, 116,  38, 113,  38,
         112,  38, 124,  26, 132,  27, 136,  19, 140,  20, 155,  14, 159,  16,
         158,  18, 170,  13, 177,  10, 187,   8, 192,   6, 175,   9, 159,  10
      },
      /*Intra*/
      {
          21, 178,  59, 110,  71,  86,  75,  85,  84,  83,  91,  66,  88,  73,
          87,  72,  92,  75,  98,  72, 105,  58, 107,  54, 115,  52, 114,  55,
         112,  56, 129,  51, 132,  40, 150,  33, 140,  29,  98,  35,  77,  42
      }
   },
   /*960 sample frames.*/
   {
      /*Inter*/
      {
          42, 121,  96,  66, 108,  43, 111,  40, 117,  44, 123,  32, 120,  36,
         119,  33, 127,  33, 134,  34, 139,  21, 147,  23, 152,  20, 158,  25,
         154,  26, 166,  21, 173,  16, 184,  13, 184,  10, 150,  13, 139,  15
      },
      /*Intra*/
      {
          22, 178,  63, 114,  74,  82,  84,  83,  92,  82, 103,  62,  96,  72,
          96,  67, 101,  73, 107,  72, 113,  55, 118,  52, 125,  52, 118,  52,
         117,  55, 135,  49, 137,  39, 157,  32, 145,  29,  97,  33,  77,  40
      }
   }
}
```

**`small_energy_icdf`** — `celt/quant_bands.c:140-140` — flat array; **declared 3, 3 top-level group(s), 3 scalar leaf/leaves in the initialiser**; `[3]`; identical in both preprocessor branches

```c
{2,1,0}
```

**`prev`** — `celt/quant_bands.c:164-164` — flat array; **declared 2, 2 top-level group(s), 2 scalar leaf/leaves in the initialiser**; `[2]`; identical in both preprocessor branches; function-local `static`

```c
{0,0}
```

**`prev`** — `celt/quant_bands.c:435-435` — flat array; **declared 2, 2 top-level group(s), 2 scalar leaf/leaves in the initialiser**; `[2]`; identical in both preprocessor branches; function-local `static`

```c
{0, 0}
```

#### `celt/rate.c`

**`LOG2_FRAC_TABLE`** — `celt/rate.c:43-49` — flat array; **declared 24, 24 top-level group(s), 24 scalar leaf/leaves in the initialiser**; `[24]`; identical in both preprocessor branches

```c
{
   0,
   8,13,
  16,19,21,23,
  24,26,27,28,29,30,31,32,
  32,33,34,34,35,36,36,37,37
}
```

**`maxN`** — `celt/rate.c:57-59` — flat array; **declared 15, 15 top-level group(s), 15 scalar leaf/leaves in the initialiser**; `[15]`; **`#if defined(CUSTOM_MODES)`**; function-local `static`

```c
{
      32767, 32767, 32767, 1476, 283, 109,  60,  40,
       29,  24,  20,  18,  16,  14,  13}
```

**`maxK`** — `celt/rate.c:60-62` — flat array; **declared 15, 15 top-level group(s), 15 scalar leaf/leaves in the initialiser**; `[15]`; **`#if defined(CUSTOM_MODES)`**; function-local `static`

```c
{
      32767, 32767, 32767, 32767, 1172, 238,  95,  53,
       36,  27,  22,  18,  16,  15,  13}
```

**`last_zero`** — `celt/rate.c:649-649` — flat array; **declared 3, 3 top-level group(s), 3 scalar leaf/leaves in the initialiser**; `[3]`; **`#if ENABLE_QEXT`**

```c
{64, 50, 0}
```

**`last_cap`** — `celt/rate.c:650-650` — flat array; **declared 3, 3 top-level group(s), 3 scalar leaf/leaves in the initialiser**; `[3]`; **`#if ENABLE_QEXT`**

```c
{110, 60, 0}
```

**`last_other`** — `celt/rate.c:651-651` — flat array; **declared 4, 4 top-level group(s), 4 scalar leaf/leaves in the initialiser**; `[4]`; **`#if ENABLE_QEXT`**

```c
{120, 112, 70, 0}
```

#### `celt/static_modes_fixed.h`

**`window120`** — `celt/static_modes_fixed.h:14-66` — flat array; **declared 120, 120 top-level group(s), 120 scalar leaf/leaves in the initialiser**; `[120]`; generated-mode table (values elided, see §6.6)

```c
static const celt_coef[120] = { ... };
```
_Full initialiser: `celt/static_modes_fixed.h:14-66` — extract with `sed -n '14,66p' celt/static_modes_fixed.h`._


**`logN400`** — `celt/static_modes_fixed.h:71-72` — flat array; **declared 21, 21 top-level group(s), 21 scalar leaf/leaves in the initialiser**; `[21]`; generated-mode table (values elided, see §6.6)

```c
static const opus_int16[21] = { ... };
```
_Full initialiser: `celt/static_modes_fixed.h:71-72` — extract with `sed -n '71,72p' celt/static_modes_fixed.h`._


**`cache_index50`** — `celt/static_modes_fixed.h:77-85` — flat array; **declared 105, 105 top-level group(s), 105 scalar leaf/leaves in the initialiser**; `[105]`; generated-mode table (values elided, see §6.6)

```c
static const opus_int16[105] = { ... };
```
_Full initialiser: `celt/static_modes_fixed.h:77-85` — extract with `sed -n '77,85p' celt/static_modes_fixed.h`._


**`cache_bits50`** — `celt/static_modes_fixed.h:86-113` — flat array; **declared 392, 392 top-level group(s), 392 scalar leaf/leaves in the initialiser**; `[392]`; generated-mode table (values elided, see §6.6)

```c
static const unsigned char[392] = { ... };
```
_Full initialiser: `celt/static_modes_fixed.h:86-113` — extract with `sed -n '86,113p' celt/static_modes_fixed.h`._


**`cache_caps50`** — `celt/static_modes_fixed.h:114-126` — flat array; **declared 168, 168 top-level group(s), 168 scalar leaf/leaves in the initialiser**; `[168]`; generated-mode table (values elided, see §6.6)

```c
static const unsigned char[168] = { ... };
```
_Full initialiser: `celt/static_modes_fixed.h:114-126` — extract with `sed -n '114,126p' celt/static_modes_fixed.h`._


**`qext_cache_index50`** — `celt/static_modes_fixed.h:132-137` — flat array; **declared 70, 70 top-level group(s), 70 scalar leaf/leaves in the initialiser**; `[70]`; **`#if ENABLE_QEXT`**; generated-mode table (values elided, see §6.6)

```c
static const opus_int16[70] = { ... };
```
_Full initialiser: `celt/static_modes_fixed.h:132-137` — extract with `sed -n '132,137p' celt/static_modes_fixed.h`._


**`qext_cache_bits50`** — `celt/static_modes_fixed.h:138-144` — flat array; **declared 86, 86 top-level group(s), 86 scalar leaf/leaves in the initialiser**; `[86]`; **`#if ENABLE_QEXT`**; generated-mode table (values elided, see §6.6)

```c
static const unsigned char[86] = { ... };
```
_Full initialiser: `celt/static_modes_fixed.h:138-144` — extract with `sed -n '138,144p' celt/static_modes_fixed.h`._


**`qext_cache_caps50`** — `celt/static_modes_fixed.h:145-153` — flat array; **declared 112, 112 top-level group(s), 112 scalar leaf/leaves in the initialiser**; `[112]`; **`#if ENABLE_QEXT`**; generated-mode table (values elided, see §6.6)

```c
static const unsigned char[112] = { ... };
```
_Full initialiser: `celt/static_modes_fixed.h:145-153` — extract with `sed -n '145,153p' celt/static_modes_fixed.h`._


**`fft_twiddles48000_960`** — `celt/static_modes_fixed.h:159-643` — array of aggregates (nested initialiser); **declared 480, 480 top-level group(s), 960 scalar leaf/leaves in the initialiser**; `[480]`; generated-mode table (values elided, see §6.6)

```c
static const kiss_twiddle_cpx[480] = { ... };
```
_Full initialiser: `celt/static_modes_fixed.h:159-643` — extract with `sed -n '159,643p' celt/static_modes_fixed.h`._


**`fft_bitrev480`** — `celt/static_modes_fixed.h:646-679` — flat array; **declared 480, 480 top-level group(s), 480 scalar leaf/leaves in the initialiser**; `[480]`; generated-mode table (values elided, see §6.6)

```c
static const opus_int16[480] = { ... };
```
_Full initialiser: `celt/static_modes_fixed.h:646-679` — extract with `sed -n '646,679p' celt/static_modes_fixed.h`._


**`fft_bitrev240`** — `celt/static_modes_fixed.h:684-701` — flat array; **declared 240, 240 top-level group(s), 240 scalar leaf/leaves in the initialiser**; `[240]`; generated-mode table (values elided, see §6.6)

```c
static const opus_int16[240] = { ... };
```
_Full initialiser: `celt/static_modes_fixed.h:684-701` — extract with `sed -n '684,701p' celt/static_modes_fixed.h`._


**`fft_bitrev120`** — `celt/static_modes_fixed.h:706-715` — flat array; **declared 120, 120 top-level group(s), 120 scalar leaf/leaves in the initialiser**; `[120]`; generated-mode table (values elided, see §6.6)

```c
static const opus_int16[120] = { ... };
```
_Full initialiser: `celt/static_modes_fixed.h:706-715` — extract with `sed -n '706,715p' celt/static_modes_fixed.h`._


**`fft_bitrev60`** — `celt/static_modes_fixed.h:720-725` — flat array; **declared 60, 60 top-level group(s), 60 scalar leaf/leaves in the initialiser**; `[60]`; generated-mode table (values elided, see §6.6)

```c
static const opus_int16[60] = { ... };
```
_Full initialiser: `celt/static_modes_fixed.h:720-725` — extract with `sed -n '720,725p' celt/static_modes_fixed.h`._


**`mdct_twiddles960`** — `celt/static_modes_fixed.h:820-1544` — flat array; **declared 1800, 1800 top-level group(s), 1800 scalar leaf/leaves in the initialiser**; `[1800]`; generated-mode table (values elided, see §6.6)

```c
static const celt_coef[1800] = { ... };
```
_Full initialiser: `celt/static_modes_fixed.h:820-1544` — extract with `sed -n '820,1544p' celt/static_modes_fixed.h`._


**`window240`** — `celt/static_modes_fixed.h:1571-1671` — flat array; **declared 240, 240 top-level group(s), 240 scalar leaf/leaves in the initialiser**; `[240]`; **`#if ENABLE_QEXT`**; generated-mode table (values elided, see §6.6)

```c
static const celt_coef[240] = { ... };
```
_Full initialiser: `celt/static_modes_fixed.h:1571-1671` — extract with `sed -n '1571,1671p' celt/static_modes_fixed.h`._


**`logN400`** — `celt/static_modes_fixed.h:1676-1677` — flat array; **declared 21, 21 top-level group(s), 21 scalar leaf/leaves in the initialiser**; `[21]`; **`#if ENABLE_QEXT`**; generated-mode table (values elided, see §6.6)

```c
static const opus_int16[21] = { ... };
```
_Full initialiser: `celt/static_modes_fixed.h:1676-1677` — extract with `sed -n '1676,1677p' celt/static_modes_fixed.h`._


**`cache_index50`** — `celt/static_modes_fixed.h:1682-1690` — flat array; **declared 105, 105 top-level group(s), 105 scalar leaf/leaves in the initialiser**; `[105]`; **`#if ENABLE_QEXT`**; generated-mode table (values elided, see §6.6)

```c
static const opus_int16[105] = { ... };
```
_Full initialiser: `celt/static_modes_fixed.h:1682-1690` — extract with `sed -n '1682,1690p' celt/static_modes_fixed.h`._


**`cache_bits50`** — `celt/static_modes_fixed.h:1691-1718` — flat array; **declared 392, 392 top-level group(s), 392 scalar leaf/leaves in the initialiser**; `[392]`; **`#if ENABLE_QEXT`**; generated-mode table (values elided, see §6.6)

```c
static const unsigned char[392] = { ... };
```
_Full initialiser: `celt/static_modes_fixed.h:1691-1718` — extract with `sed -n '1691,1718p' celt/static_modes_fixed.h`._


**`cache_caps50`** — `celt/static_modes_fixed.h:1719-1731` — flat array; **declared 168, 168 top-level group(s), 168 scalar leaf/leaves in the initialiser**; `[168]`; **`#if ENABLE_QEXT`**; generated-mode table (values elided, see §6.6)

```c
static const unsigned char[168] = { ... };
```
_Full initialiser: `celt/static_modes_fixed.h:1719-1731` — extract with `sed -n '1719,1731p' celt/static_modes_fixed.h`._


**`qext_cache_index50`** — `celt/static_modes_fixed.h:1737-1742` — flat array; **declared 70, 70 top-level group(s), 70 scalar leaf/leaves in the initialiser**; `[70]`; **`#if ENABLE_QEXT && ENABLE_QEXT`**; generated-mode table (values elided, see §6.6)

```c
static const opus_int16[70] = { ... };
```
_Full initialiser: `celt/static_modes_fixed.h:1737-1742` — extract with `sed -n '1737,1742p' celt/static_modes_fixed.h`._


**`qext_cache_bits50`** — `celt/static_modes_fixed.h:1743-1749` — flat array; **declared 86, 86 top-level group(s), 86 scalar leaf/leaves in the initialiser**; `[86]`; **`#if ENABLE_QEXT && ENABLE_QEXT`**; generated-mode table (values elided, see §6.6)

```c
static const unsigned char[86] = { ... };
```
_Full initialiser: `celt/static_modes_fixed.h:1743-1749` — extract with `sed -n '1743,1749p' celt/static_modes_fixed.h`._


**`qext_cache_caps50`** — `celt/static_modes_fixed.h:1750-1758` — flat array; **declared 112, 112 top-level group(s), 112 scalar leaf/leaves in the initialiser**; `[112]`; **`#if ENABLE_QEXT && ENABLE_QEXT`**; generated-mode table (values elided, see §6.6)

```c
static const unsigned char[112] = { ... };
```
_Full initialiser: `celt/static_modes_fixed.h:1750-1758` — extract with `sed -n '1750,1758p' celt/static_modes_fixed.h`._


**`fft_twiddles96000_1920`** — `celt/static_modes_fixed.h:1764-2728` — array of aggregates (nested initialiser); **declared 960, 960 top-level group(s), 1920 scalar leaf/leaves in the initialiser**; `[960]`; **`#if ENABLE_QEXT`**; generated-mode table (values elided, see §6.6)

```c
static const kiss_twiddle_cpx[960] = { ... };
```
_Full initialiser: `celt/static_modes_fixed.h:1764-2728` — extract with `sed -n '1764,2728p' celt/static_modes_fixed.h`._


**`fft_bitrev960`** — `celt/static_modes_fixed.h:2731-2796` — flat array; **declared 960, 960 top-level group(s), 960 scalar leaf/leaves in the initialiser**; `[960]`; **`#if ENABLE_QEXT`**; generated-mode table (values elided, see §6.6)

```c
static const opus_int16[960] = { ... };
```
_Full initialiser: `celt/static_modes_fixed.h:2731-2796` — extract with `sed -n '2731,2796p' celt/static_modes_fixed.h`._


**`fft_bitrev480`** — `celt/static_modes_fixed.h:2801-2834` — flat array; **declared 480, 480 top-level group(s), 480 scalar leaf/leaves in the initialiser**; `[480]`; **`#if ENABLE_QEXT`**; generated-mode table (values elided, see §6.6)

```c
static const opus_int16[480] = { ... };
```
_Full initialiser: `celt/static_modes_fixed.h:2801-2834` — extract with `sed -n '2801,2834p' celt/static_modes_fixed.h`._


**`fft_bitrev240`** — `celt/static_modes_fixed.h:2839-2856` — flat array; **declared 240, 240 top-level group(s), 240 scalar leaf/leaves in the initialiser**; `[240]`; **`#if ENABLE_QEXT`**; generated-mode table (values elided, see §6.6)

```c
static const opus_int16[240] = { ... };
```
_Full initialiser: `celt/static_modes_fixed.h:2839-2856` — extract with `sed -n '2839,2856p' celt/static_modes_fixed.h`._


**`fft_bitrev120`** — `celt/static_modes_fixed.h:2861-2870` — flat array; **declared 120, 120 top-level group(s), 120 scalar leaf/leaves in the initialiser**; `[120]`; **`#if ENABLE_QEXT`**; generated-mode table (values elided, see §6.6)

```c
static const opus_int16[120] = { ... };
```
_Full initialiser: `celt/static_modes_fixed.h:2861-2870` — extract with `sed -n '2861,2870p' celt/static_modes_fixed.h`._


**`mdct_twiddles1920`** — `celt/static_modes_fixed.h:2965-4409` — flat array; **declared 3600, 3600 top-level group(s), 3600 scalar leaf/leaves in the initialiser**; `[3600]`; **`#if ENABLE_QEXT`**; generated-mode table (values elided, see §6.6)

```c
static const celt_coef[3600] = { ... };
```
_Full initialiser: `celt/static_modes_fixed.h:2965-4409` — extract with `sed -n '2965,4409p' celt/static_modes_fixed.h`._


**`static_mode_list`** — `celt/static_modes_fixed.h:4437-4440` — flat array; **declared ?, 2 top-level group(s), 2 scalar leaf/leaves in the initialiser**; `[TOTAL_MODES]`; **`#if ENABLE_QEXT`**; generated-mode table (values elided, see §6.6)

```c
static const CELTMode * const[TOTAL_MODES] = { ... };
```
_Full initialiser: `celt/static_modes_fixed.h:4437-4440` — extract with `sed -n '4437,4440p' celt/static_modes_fixed.h`._


**`static_mode_list (the `#else` variant)`** — `celt/static_modes_fixed.h:4443-4445` — flat array; **declared ?, 1 top-level group(s), 1 scalar leaf/leaves in the initialiser**; `[TOTAL_MODES]`; **`#if !(ENABLE_QEXT)`**; `#else` branch only; generated-mode table (values elided, see §6.6)

```c
static const CELTMode * const[TOTAL_MODES] = { ... };
```
_Full initialiser: `celt/static_modes_fixed.h:4443-4445` — extract with `sed -n '4443,4445p' celt/static_modes_fixed.h`._


#### `celt/static_modes_fixed_arm_ne10.h`

**`ne10_factors_480`** — `celt/static_modes_fixed_arm_ne10.h:8-13` — flat array; **declared 64, 64 top-level group(s), 64 scalar leaf/leaves in the initialiser**; `[64]`; generated-mode table (values elided, see §6.6)

```c
static const ne10_int32_t[64] = { ... };
```
_Full initialiser: `celt/static_modes_fixed_arm_ne10.h:8-13` — extract with `sed -n '8,13p' celt/static_modes_fixed_arm_ne10.h`._


**`ne10_factors_240`** — `celt/static_modes_fixed_arm_ne10.h:14-19` — flat array; **declared 64, 64 top-level group(s), 64 scalar leaf/leaves in the initialiser**; `[64]`; generated-mode table (values elided, see §6.6)

```c
static const ne10_int32_t[64] = { ... };
```
_Full initialiser: `celt/static_modes_fixed_arm_ne10.h:14-19` — extract with `sed -n '14,19p' celt/static_modes_fixed_arm_ne10.h`._


**`ne10_factors_120`** — `celt/static_modes_fixed_arm_ne10.h:20-25` — flat array; **declared 64, 64 top-level group(s), 64 scalar leaf/leaves in the initialiser**; `[64]`; generated-mode table (values elided, see §6.6)

```c
static const ne10_int32_t[64] = { ... };
```
_Full initialiser: `celt/static_modes_fixed_arm_ne10.h:20-25` — extract with `sed -n '20,25p' celt/static_modes_fixed_arm_ne10.h`._


**`ne10_factors_60`** — `celt/static_modes_fixed_arm_ne10.h:26-31` — flat array; **declared 64, 64 top-level group(s), 64 scalar leaf/leaves in the initialiser**; `[64]`; generated-mode table (values elided, see §6.6)

```c
static const ne10_int32_t[64] = { ... };
```
_Full initialiser: `celt/static_modes_fixed_arm_ne10.h:26-31` — extract with `sed -n '26,31p' celt/static_modes_fixed_arm_ne10.h`._


**`ne10_twiddles_480`** — `celt/static_modes_fixed_arm_ne10.h:32-193` — array of aggregates (nested initialiser); **declared 480, 480 top-level group(s), 960 scalar leaf/leaves in the initialiser**; `[480]`; generated-mode table (values elided, see §6.6)

```c
static const ne10_fft_cpx_int32_t[480] = { ... };
```
_Full initialiser: `celt/static_modes_fixed_arm_ne10.h:32-193` — extract with `sed -n '32,193p' celt/static_modes_fixed_arm_ne10.h`._


**`ne10_twiddles_240`** — `celt/static_modes_fixed_arm_ne10.h:194-275` — array of aggregates (nested initialiser); **declared 240, 240 top-level group(s), 480 scalar leaf/leaves in the initialiser**; `[240]`; generated-mode table (values elided, see §6.6)

```c
static const ne10_fft_cpx_int32_t[240] = { ... };
```
_Full initialiser: `celt/static_modes_fixed_arm_ne10.h:194-275` — extract with `sed -n '194,275p' celt/static_modes_fixed_arm_ne10.h`._


**`ne10_twiddles_120`** — `celt/static_modes_fixed_arm_ne10.h:276-317` — array of aggregates (nested initialiser); **declared 120, 120 top-level group(s), 240 scalar leaf/leaves in the initialiser**; `[120]`; generated-mode table (values elided, see §6.6)

```c
static const ne10_fft_cpx_int32_t[120] = { ... };
```
_Full initialiser: `celt/static_modes_fixed_arm_ne10.h:276-317` — extract with `sed -n '276,317p' celt/static_modes_fixed_arm_ne10.h`._


**`ne10_twiddles_60`** — `celt/static_modes_fixed_arm_ne10.h:318-339` — array of aggregates (nested initialiser); **declared 60, 60 top-level group(s), 120 scalar leaf/leaves in the initialiser**; `[60]`; generated-mode table (values elided, see §6.6)

```c
static const ne10_fft_cpx_int32_t[60] = { ... };
```
_Full initialiser: `celt/static_modes_fixed_arm_ne10.h:318-339` — extract with `sed -n '318,339p' celt/static_modes_fixed_arm_ne10.h`._


#### `celt/static_modes_float.h`

**`window120`** — `celt/static_modes_float.h:14-39` — flat array; **declared 120, 120 top-level group(s), 120 scalar leaf/leaves in the initialiser**; `[120]`; generated-mode table (values elided, see §6.6)

```c
static const celt_coef[120] = { ... };
```
_Full initialiser: `celt/static_modes_float.h:14-39` — extract with `sed -n '14,39p' celt/static_modes_float.h`._


**`logN400`** — `celt/static_modes_float.h:44-45` — flat array; **declared 21, 21 top-level group(s), 21 scalar leaf/leaves in the initialiser**; `[21]`; generated-mode table (values elided, see §6.6)

```c
static const opus_int16[21] = { ... };
```
_Full initialiser: `celt/static_modes_float.h:44-45` — extract with `sed -n '44,45p' celt/static_modes_float.h`._


**`cache_index50`** — `celt/static_modes_float.h:50-58` — flat array; **declared 105, 105 top-level group(s), 105 scalar leaf/leaves in the initialiser**; `[105]`; generated-mode table (values elided, see §6.6)

```c
static const opus_int16[105] = { ... };
```
_Full initialiser: `celt/static_modes_float.h:50-58` — extract with `sed -n '50,58p' celt/static_modes_float.h`._


**`cache_bits50`** — `celt/static_modes_float.h:59-86` — flat array; **declared 392, 392 top-level group(s), 392 scalar leaf/leaves in the initialiser**; `[392]`; generated-mode table (values elided, see §6.6)

```c
static const unsigned char[392] = { ... };
```
_Full initialiser: `celt/static_modes_float.h:59-86` — extract with `sed -n '59,86p' celt/static_modes_float.h`._


**`cache_caps50`** — `celt/static_modes_float.h:87-99` — flat array; **declared 168, 168 top-level group(s), 168 scalar leaf/leaves in the initialiser**; `[168]`; generated-mode table (values elided, see §6.6)

```c
static const unsigned char[168] = { ... };
```
_Full initialiser: `celt/static_modes_float.h:87-99` — extract with `sed -n '87,99p' celt/static_modes_float.h`._


**`qext_cache_index50`** — `celt/static_modes_float.h:105-110` — flat array; **declared 70, 70 top-level group(s), 70 scalar leaf/leaves in the initialiser**; `[70]`; **`#if ENABLE_QEXT`**; generated-mode table (values elided, see §6.6)

```c
static const opus_int16[70] = { ... };
```
_Full initialiser: `celt/static_modes_float.h:105-110` — extract with `sed -n '105,110p' celt/static_modes_float.h`._


**`qext_cache_bits50`** — `celt/static_modes_float.h:111-117` — flat array; **declared 86, 86 top-level group(s), 86 scalar leaf/leaves in the initialiser**; `[86]`; **`#if ENABLE_QEXT`**; generated-mode table (values elided, see §6.6)

```c
static const unsigned char[86] = { ... };
```
_Full initialiser: `celt/static_modes_float.h:111-117` — extract with `sed -n '111,117p' celt/static_modes_float.h`._


**`qext_cache_caps50`** — `celt/static_modes_float.h:118-126` — flat array; **declared 112, 112 top-level group(s), 112 scalar leaf/leaves in the initialiser**; `[112]`; **`#if ENABLE_QEXT`**; generated-mode table (values elided, see §6.6)

```c
static const unsigned char[112] = { ... };
```
_Full initialiser: `celt/static_modes_float.h:118-126` — extract with `sed -n '118,126p' celt/static_modes_float.h`._


**`fft_twiddles48000_960`** — `celt/static_modes_float.h:132-373` — array of aggregates (nested initialiser); **declared 480, 480 top-level group(s), 960 scalar leaf/leaves in the initialiser**; `[480]`; generated-mode table (values elided, see §6.6)

```c
static const kiss_twiddle_cpx[480] = { ... };
```
_Full initialiser: `celt/static_modes_float.h:132-373` — extract with `sed -n '132,373p' celt/static_modes_float.h`._


**`fft_bitrev480`** — `celt/static_modes_float.h:376-409` — flat array; **declared 480, 480 top-level group(s), 480 scalar leaf/leaves in the initialiser**; `[480]`; generated-mode table (values elided, see §6.6)

```c
static const opus_int16[480] = { ... };
```
_Full initialiser: `celt/static_modes_float.h:376-409` — extract with `sed -n '376,409p' celt/static_modes_float.h`._


**`fft_bitrev240`** — `celt/static_modes_float.h:414-431` — flat array; **declared 240, 240 top-level group(s), 240 scalar leaf/leaves in the initialiser**; `[240]`; generated-mode table (values elided, see §6.6)

```c
static const opus_int16[240] = { ... };
```
_Full initialiser: `celt/static_modes_float.h:414-431` — extract with `sed -n '414,431p' celt/static_modes_float.h`._


**`fft_bitrev120`** — `celt/static_modes_float.h:436-445` — flat array; **declared 120, 120 top-level group(s), 120 scalar leaf/leaves in the initialiser**; `[120]`; generated-mode table (values elided, see §6.6)

```c
static const opus_int16[120] = { ... };
```
_Full initialiser: `celt/static_modes_float.h:436-445` — extract with `sed -n '436,445p' celt/static_modes_float.h`._


**`fft_bitrev60`** — `celt/static_modes_float.h:450-455` — flat array; **declared 60, 60 top-level group(s), 60 scalar leaf/leaves in the initialiser**; `[60]`; generated-mode table (values elided, see §6.6)

```c
static const opus_int16[60] = { ... };
```
_Full initialiser: `celt/static_modes_float.h:450-455` — extract with `sed -n '450,455p' celt/static_modes_float.h`._


**`mdct_twiddles960`** — `celt/static_modes_float.h:530-891` — flat array; **declared 1800, 1800 top-level group(s), 1800 scalar leaf/leaves in the initialiser**; `[1800]`; generated-mode table (values elided, see §6.6)

```c
static const celt_coef[1800] = { ... };
```
_Full initialiser: `celt/static_modes_float.h:530-891` — extract with `sed -n '530,891p' celt/static_modes_float.h`._


**`window240`** — `celt/static_modes_float.h:918-967` — flat array; **declared 240, 240 top-level group(s), 240 scalar leaf/leaves in the initialiser**; `[240]`; **`#if ENABLE_QEXT`**; generated-mode table (values elided, see §6.6)

```c
static const celt_coef[240] = { ... };
```
_Full initialiser: `celt/static_modes_float.h:918-967` — extract with `sed -n '918,967p' celt/static_modes_float.h`._


**`logN400`** — `celt/static_modes_float.h:972-973` — flat array; **declared 21, 21 top-level group(s), 21 scalar leaf/leaves in the initialiser**; `[21]`; **`#if ENABLE_QEXT`**; generated-mode table (values elided, see §6.6)

```c
static const opus_int16[21] = { ... };
```
_Full initialiser: `celt/static_modes_float.h:972-973` — extract with `sed -n '972,973p' celt/static_modes_float.h`._


**`cache_index50`** — `celt/static_modes_float.h:978-986` — flat array; **declared 105, 105 top-level group(s), 105 scalar leaf/leaves in the initialiser**; `[105]`; **`#if ENABLE_QEXT`**; generated-mode table (values elided, see §6.6)

```c
static const opus_int16[105] = { ... };
```
_Full initialiser: `celt/static_modes_float.h:978-986` — extract with `sed -n '978,986p' celt/static_modes_float.h`._


**`cache_bits50`** — `celt/static_modes_float.h:987-1014` — flat array; **declared 392, 392 top-level group(s), 392 scalar leaf/leaves in the initialiser**; `[392]`; **`#if ENABLE_QEXT`**; generated-mode table (values elided, see §6.6)

```c
static const unsigned char[392] = { ... };
```
_Full initialiser: `celt/static_modes_float.h:987-1014` — extract with `sed -n '987,1014p' celt/static_modes_float.h`._


**`cache_caps50`** — `celt/static_modes_float.h:1015-1027` — flat array; **declared 168, 168 top-level group(s), 168 scalar leaf/leaves in the initialiser**; `[168]`; **`#if ENABLE_QEXT`**; generated-mode table (values elided, see §6.6)

```c
static const unsigned char[168] = { ... };
```
_Full initialiser: `celt/static_modes_float.h:1015-1027` — extract with `sed -n '1015,1027p' celt/static_modes_float.h`._


**`qext_cache_index50`** — `celt/static_modes_float.h:1033-1038` — flat array; **declared 70, 70 top-level group(s), 70 scalar leaf/leaves in the initialiser**; `[70]`; **`#if ENABLE_QEXT && ENABLE_QEXT`**; generated-mode table (values elided, see §6.6)

```c
static const opus_int16[70] = { ... };
```
_Full initialiser: `celt/static_modes_float.h:1033-1038` — extract with `sed -n '1033,1038p' celt/static_modes_float.h`._


**`qext_cache_bits50`** — `celt/static_modes_float.h:1039-1045` — flat array; **declared 86, 86 top-level group(s), 86 scalar leaf/leaves in the initialiser**; `[86]`; **`#if ENABLE_QEXT && ENABLE_QEXT`**; generated-mode table (values elided, see §6.6)

```c
static const unsigned char[86] = { ... };
```
_Full initialiser: `celt/static_modes_float.h:1039-1045` — extract with `sed -n '1039,1045p' celt/static_modes_float.h`._


**`qext_cache_caps50`** — `celt/static_modes_float.h:1046-1054` — flat array; **declared 112, 112 top-level group(s), 112 scalar leaf/leaves in the initialiser**; `[112]`; **`#if ENABLE_QEXT && ENABLE_QEXT`**; generated-mode table (values elided, see §6.6)

```c
static const unsigned char[112] = { ... };
```
_Full initialiser: `celt/static_modes_float.h:1046-1054` — extract with `sed -n '1046,1054p' celt/static_modes_float.h`._


**`fft_twiddles96000_1920`** — `celt/static_modes_float.h:1060-1541` — array of aggregates (nested initialiser); **declared 960, 960 top-level group(s), 1920 scalar leaf/leaves in the initialiser**; `[960]`; **`#if ENABLE_QEXT`**; generated-mode table (values elided, see §6.6)

```c
static const kiss_twiddle_cpx[960] = { ... };
```
_Full initialiser: `celt/static_modes_float.h:1060-1541` — extract with `sed -n '1060,1541p' celt/static_modes_float.h`._


**`fft_bitrev960`** — `celt/static_modes_float.h:1544-1609` — flat array; **declared 960, 960 top-level group(s), 960 scalar leaf/leaves in the initialiser**; `[960]`; **`#if ENABLE_QEXT`**; generated-mode table (values elided, see §6.6)

```c
static const opus_int16[960] = { ... };
```
_Full initialiser: `celt/static_modes_float.h:1544-1609` — extract with `sed -n '1544,1609p' celt/static_modes_float.h`._


**`fft_bitrev480`** — `celt/static_modes_float.h:1614-1647` — flat array; **declared 480, 480 top-level group(s), 480 scalar leaf/leaves in the initialiser**; `[480]`; **`#if ENABLE_QEXT`**; generated-mode table (values elided, see §6.6)

```c
static const opus_int16[480] = { ... };
```
_Full initialiser: `celt/static_modes_float.h:1614-1647` — extract with `sed -n '1614,1647p' celt/static_modes_float.h`._


**`fft_bitrev240`** — `celt/static_modes_float.h:1652-1669` — flat array; **declared 240, 240 top-level group(s), 240 scalar leaf/leaves in the initialiser**; `[240]`; **`#if ENABLE_QEXT`**; generated-mode table (values elided, see §6.6)

```c
static const opus_int16[240] = { ... };
```
_Full initialiser: `celt/static_modes_float.h:1652-1669` — extract with `sed -n '1652,1669p' celt/static_modes_float.h`._


**`fft_bitrev120`** — `celt/static_modes_float.h:1674-1683` — flat array; **declared 120, 120 top-level group(s), 120 scalar leaf/leaves in the initialiser**; `[120]`; **`#if ENABLE_QEXT`**; generated-mode table (values elided, see §6.6)

```c
static const opus_int16[120] = { ... };
```
_Full initialiser: `celt/static_modes_float.h:1674-1683` — extract with `sed -n '1674,1683p' celt/static_modes_float.h`._


**`mdct_twiddles1920`** — `celt/static_modes_float.h:1758-2479` — flat array; **declared 3600, 3600 top-level group(s), 3600 scalar leaf/leaves in the initialiser**; `[3600]`; **`#if ENABLE_QEXT`**; generated-mode table (values elided, see §6.6)

```c
static const celt_coef[3600] = { ... };
```
_Full initialiser: `celt/static_modes_float.h:1758-2479` — extract with `sed -n '1758,2479p' celt/static_modes_float.h`._


**`static_mode_list`** — `celt/static_modes_float.h:2507-2510` — flat array; **declared ?, 2 top-level group(s), 2 scalar leaf/leaves in the initialiser**; `[TOTAL_MODES]`; **`#if ENABLE_QEXT`**; generated-mode table (values elided, see §6.6)

```c
static const CELTMode * const[TOTAL_MODES] = { ... };
```
_Full initialiser: `celt/static_modes_float.h:2507-2510` — extract with `sed -n '2507,2510p' celt/static_modes_float.h`._


**`static_mode_list (the `#else` variant)`** — `celt/static_modes_float.h:2513-2515` — flat array; **declared ?, 1 top-level group(s), 1 scalar leaf/leaves in the initialiser**; `[TOTAL_MODES]`; **`#if !(ENABLE_QEXT)`**; `#else` branch only; generated-mode table (values elided, see §6.6)

```c
static const CELTMode * const[TOTAL_MODES] = { ... };
```
_Full initialiser: `celt/static_modes_float.h:2513-2515` — extract with `sed -n '2513,2515p' celt/static_modes_float.h`._


#### `celt/static_modes_float_arm_ne10.h`

**`ne10_factors_480`** — `celt/static_modes_float_arm_ne10.h:8-13` — flat array; **declared 64, 64 top-level group(s), 64 scalar leaf/leaves in the initialiser**; `[64]`; generated-mode table (values elided, see §6.6)

```c
static const ne10_int32_t[64] = { ... };
```
_Full initialiser: `celt/static_modes_float_arm_ne10.h:8-13` — extract with `sed -n '8,13p' celt/static_modes_float_arm_ne10.h`._


**`ne10_factors_240`** — `celt/static_modes_float_arm_ne10.h:14-19` — flat array; **declared 64, 64 top-level group(s), 64 scalar leaf/leaves in the initialiser**; `[64]`; generated-mode table (values elided, see §6.6)

```c
static const ne10_int32_t[64] = { ... };
```
_Full initialiser: `celt/static_modes_float_arm_ne10.h:14-19` — extract with `sed -n '14,19p' celt/static_modes_float_arm_ne10.h`._


**`ne10_factors_120`** — `celt/static_modes_float_arm_ne10.h:20-25` — flat array; **declared 64, 64 top-level group(s), 64 scalar leaf/leaves in the initialiser**; `[64]`; generated-mode table (values elided, see §6.6)

```c
static const ne10_int32_t[64] = { ... };
```
_Full initialiser: `celt/static_modes_float_arm_ne10.h:20-25` — extract with `sed -n '20,25p' celt/static_modes_float_arm_ne10.h`._


**`ne10_factors_60`** — `celt/static_modes_float_arm_ne10.h:26-31` — flat array; **declared 64, 64 top-level group(s), 64 scalar leaf/leaves in the initialiser**; `[64]`; generated-mode table (values elided, see §6.6)

```c
static const ne10_int32_t[64] = { ... };
```
_Full initialiser: `celt/static_modes_float_arm_ne10.h:26-31` — extract with `sed -n '26,31p' celt/static_modes_float_arm_ne10.h`._


**`ne10_twiddles_480`** — `celt/static_modes_float_arm_ne10.h:32-193` — array of aggregates (nested initialiser); **declared 480, 480 top-level group(s), 960 scalar leaf/leaves in the initialiser**; `[480]`; generated-mode table (values elided, see §6.6)

```c
static const ne10_fft_cpx_float32_t[480] = { ... };
```
_Full initialiser: `celt/static_modes_float_arm_ne10.h:32-193` — extract with `sed -n '32,193p' celt/static_modes_float_arm_ne10.h`._


**`ne10_twiddles_240`** — `celt/static_modes_float_arm_ne10.h:194-275` — array of aggregates (nested initialiser); **declared 240, 240 top-level group(s), 480 scalar leaf/leaves in the initialiser**; `[240]`; generated-mode table (values elided, see §6.6)

```c
static const ne10_fft_cpx_float32_t[240] = { ... };
```
_Full initialiser: `celt/static_modes_float_arm_ne10.h:194-275` — extract with `sed -n '194,275p' celt/static_modes_float_arm_ne10.h`._


**`ne10_twiddles_120`** — `celt/static_modes_float_arm_ne10.h:276-317` — array of aggregates (nested initialiser); **declared 120, 120 top-level group(s), 240 scalar leaf/leaves in the initialiser**; `[120]`; generated-mode table (values elided, see §6.6)

```c
static const ne10_fft_cpx_float32_t[120] = { ... };
```
_Full initialiser: `celt/static_modes_float_arm_ne10.h:276-317` — extract with `sed -n '276,317p' celt/static_modes_float_arm_ne10.h`._


**`ne10_twiddles_60`** — `celt/static_modes_float_arm_ne10.h:318-339` — array of aggregates (nested initialiser); **declared 60, 60 top-level group(s), 120 scalar leaf/leaves in the initialiser**; `[60]`; generated-mode table (values elided, see §6.6)

```c
static const ne10_fft_cpx_float32_t[60] = { ... };
```
_Full initialiser: `celt/static_modes_float_arm_ne10.h:318-339` — extract with `sed -n '318,339p' celt/static_modes_float_arm_ne10.h`._


#### `celt/vq.c`

**`SPREAD_FACTOR`** — `celt/vq.c:106-106` — flat array; **declared 3, 3 top-level group(s), 3 scalar leaf/leaves in the initialiser**; `[3]`; identical in both preprocessor branches; function-local `static`

```c
{15,10,5}
```

### 6.3 SILK tables

#### `silk/NLSF2A.c`

**`ordering16`** — `silk/NLSF2A.c:75-77` — flat array; **declared 16, 16 top-level group(s), 16 scalar leaf/leaves in the initialiser**; `[16]`; identical in both preprocessor branches; function-local `static`

```c
{
      0, 15, 8, 7, 4, 11, 12, 3, 2, 13, 10, 5, 6, 9, 14, 1
    }
```

**`ordering10`** — `silk/NLSF2A.c:78-80` — flat array; **declared 10, 10 top-level group(s), 10 scalar leaf/leaves in the initialiser**; `[10]`; identical in both preprocessor branches; function-local `static`

```c
{
      0, 9, 6, 3, 4, 5, 8, 1, 2, 7
    }
```

#### `silk/PLC.c`

**`HARM_ATT_Q15`** — `silk/PLC.c:41-41` — flat array; **declared ?, 2 top-level group(s), 2 scalar leaf/leaves in the initialiser**; `[NB_ATT]`; identical in both preprocessor branches

```c
{ 32440, 31130 }
```

**`PLC_RAND_ATTENUATE_V_Q15`** — `silk/PLC.c:42-42` — flat array; **declared ?, 2 top-level group(s), 2 scalar leaf/leaves in the initialiser**; `[NB_ATT]`; identical in both preprocessor branches

```c
{ 31130, 26214 }
```

**`PLC_RAND_ATTENUATE_UV_Q15`** — `silk/PLC.c:43-43` — flat array; **declared ?, 2 top-level group(s), 2 scalar leaf/leaves in the initialiser**; `[NB_ATT]`; identical in both preprocessor branches

```c
{ 32440, 29491 }
```

#### `silk/VAD.c`

**`tiltWeights`** — `silk/VAD.c:77-77` — flat array; **declared ?, 4 top-level group(s), 4 scalar leaf/leaves in the initialiser**; `[ VAD_N_BANDS ]`; identical in both preprocessor branches

```c
{ 30000, 6000, -12000, -12000 }
```

#### `silk/control_SNR.c`

**`silk_TargetRate_NB_21`** — `silk/control_SNR.c:40-49` — flat array; **declared ?, 107 top-level group(s), 107 scalar leaf/leaves in the initialiser**; `[117 - 10]`; identical in both preprocessor branches

```c
{
                                              0, 15, 39, 52, 61, 68,
     74, 79, 84, 88, 92, 95, 99,102,105,108,111,114,117,119,122,124,
    126,129,131,133,135,137,139,142,143,145,147,149,151,153,155,157,
    158,160,162,163,165,167,168,170,171,173,174,176,177,179,180,182,
    183,185,186,187,189,190,192,193,194,196,197,199,200,201,203,204,
    205,207,208,209,211,212,213,215,216,217,219,220,221,223,224,225,
    227,228,230,231,232,234,235,236,238,239,241,242,243,245,246,248,
    249,250,252,253,255
}
```

**`silk_TargetRate_MB_21`** — `silk/control_SNR.c:51-63` — flat array; **declared ?, 155 top-level group(s), 155 scalar leaf/leaves in the initialiser**; `[165 - 10]`; identical in both preprocessor branches

```c
{
                                              0,  0, 28, 43, 52, 59,
     65, 70, 74, 78, 81, 85, 87, 90, 93, 95, 98,100,102,105,107,109,
    111,113,115,116,118,120,122,123,125,127,128,130,131,133,134,136,
    137,138,140,141,143,144,145,147,148,149,151,152,153,154,156,157,
    158,159,160,162,163,164,165,166,167,168,169,171,172,173,174,175,
    176,177,178,179,180,181,182,183,184,185,186,187,188,188,189,190,
    191,192,193,194,195,196,197,198,199,200,201,202,203,203,204,205,
    206,207,208,209,210,211,212,213,214,214,215,216,217,218,219,220,
    221,222,223,224,224,225,226,227,228,229,230,231,232,233,234,235,
    236,236,237,238,239,240,241,242,243,244,245,246,247,248,249,250,
    251,252,253,254,255
}
```

**`silk_TargetRate_WB_21`** — `silk/control_SNR.c:65-79` — flat array; **declared ?, 191 top-level group(s), 191 scalar leaf/leaves in the initialiser**; `[201 - 10]`; identical in both preprocessor branches

```c
{
                                              0,  0,  0,  8, 29, 41,
     49, 56, 62, 66, 70, 74, 77, 80, 83, 86, 88, 91, 93, 95, 97, 99,
    101,103,105,107,108,110,112,113,115,116,118,119,121,122,123,125,
    126,127,129,130,131,132,134,135,136,137,138,140,141,142,143,144,
    145,146,147,148,149,150,151,152,153,154,156,157,158,159,159,160,
    161,162,163,164,165,166,167,168,169,170,171,171,172,173,174,175,
    176,177,177,178,179,180,181,181,182,183,184,185,185,186,187,188,
    189,189,190,191,192,192,193,194,195,195,196,197,198,198,199,200,
    200,201,202,203,203,204,205,206,206,207,208,209,209,210,211,211,
    212,213,214,214,215,216,216,217,218,219,219,220,221,221,222,223,
    224,224,225,226,226,227,228,229,229,230,231,232,232,233,234,234,
    235,236,237,237,238,239,240,240,241,242,243,243,244,245,246,246,
    247,248,249,249,250,251,252,253,255
}
```

#### `silk/dec_API.c`

**`MS_pred_Q13`** — `silk/dec_API.c:154-154` — flat array; **declared 2, 1 top-level group(s), 1 scalar leaf/leaves in the initialiser**; `[ 2 ]`; identical in both preprocessor branches; function-local `static`

```c
{ 0 }
```

**`mult_tab`** — `silk/dec_API.c:459-459` — flat array; **declared 3, 3 top-level group(s), 3 scalar leaf/leaves in the initialiser**; `[ 3 ]`; identical in both preprocessor branches; function-local `static`

```c
{ 6, 4, 3 }
```

#### `silk/enc_API.c`

**`iCDF`** — `silk/enc_API.c:358-358` — flat array; **declared 2, 2 top-level group(s), 2 scalar leaf/leaves in the initialiser**; `[ 2 ]`; identical in both preprocessor branches; function-local `static`

```c
{ 0, 0 }
```

#### `silk/fixed/apply_sine_window_FIX.c`

**`freq_table_Q16`** — `silk/fixed/apply_sine_window_FIX.c:44-48` — flat array; **declared 27, 27 top-level group(s), 27 scalar leaf/leaves in the initialiser**; `[ 27 ]`; identical in both preprocessor branches

```c
{
   12111,    9804,    8235,    7100,    6239,    5565,    5022,    4575,    4202,
    3885,    3612,    3375,    3167,    2984,    2820,    2674,    2542,    2422,
    2313,    2214,    2123,    2038,    1961,    1889,    1822,    1760,    1702,
}
```

#### `silk/fixed/encode_frame_FIX.c`

**`gain_lock`** — `silk/fixed/encode_frame_FIX.c:105-105` — flat array; **declared ?, 1 top-level group(s), 1 scalar leaf/leaves in the initialiser**; `[ MAX_NB_SUBFR ]`; identical in both preprocessor branches; function-local `static`

```c
{0}
```

#### `silk/fixed/find_pred_coefs_FIX.c`

**`NLSF_Q15`** — `silk/fixed/find_pred_coefs_FIX.c:46-46` — flat array; **declared ?, 1 top-level group(s), 1 scalar leaf/leaves in the initialiser**; `[ MAX_LPC_ORDER ]`; identical in both preprocessor branches; function-local `static`

```c
{0}
```

#### `silk/fixed/warped_autocorrelation_FIX.c`

**`state_QS`** — `silk/fixed/warped_autocorrelation_FIX.c:52-52` — flat array; **declared ?, 1 top-level group(s), 1 scalar leaf/leaves in the initialiser**; `[ MAX_SHAPE_LPC_ORDER + 1 ]`; function-local `static`

```c
{ 0 }
```

**`corr_QC`** — `silk/fixed/warped_autocorrelation_FIX.c:53-53` — flat array; **declared ?, 1 top-level group(s), 1 scalar leaf/leaves in the initialiser**; `[ MAX_SHAPE_LPC_ORDER + 1 ]`; function-local `static`

```c
{ 0 }
```

#### `silk/float/encode_frame_FLP.c`

**`gain_lock`** — `silk/float/encode_frame_FLP.c:107-107` — flat array; **declared ?, 1 top-level group(s), 1 scalar leaf/leaves in the initialiser**; `[ MAX_NB_SUBFR ]`; identical in both preprocessor branches; function-local `static`

```c
{0}
```

#### `silk/float/find_pred_coefs_FLP.c`

**`NLSF_Q15`** — `silk/float/find_pred_coefs_FLP.c:48-48` — flat array; **declared ?, 1 top-level group(s), 1 scalar leaf/leaves in the initialiser**; `[ MAX_LPC_ORDER ]`; identical in both preprocessor branches; function-local `static`

```c
{0}
```

#### `silk/float/warped_autocorrelation_FLP.c`

**`state`** — `silk/float/warped_autocorrelation_FLP.c:45-45` — flat array; **declared ?, 1 top-level group(s), 1 scalar leaf/leaves in the initialiser**; `[ MAX_SHAPE_LPC_ORDER + 1 ]`; identical in both preprocessor branches; function-local `static`

```c
{ 0 }
```

**`C`** — `silk/float/warped_autocorrelation_FLP.c:46-46` — flat array; **declared ?, 1 top-level group(s), 1 scalar leaf/leaves in the initialiser**; `[ MAX_SHAPE_LPC_ORDER + 1 ]`; identical in both preprocessor branches; function-local `static`

```c
{ 0 }
```

#### `silk/pitch_est_tables.c`

**`silk_CB_lags_stage2_10_ms`** — `silk/pitch_est_tables.c:35-39` — 2-D array; **declared ?, 2 top-level group(s), 6 scalar leaf/leaves in the initialiser**; `[ PE_MAX_NB_SUBFR >> 1][ PE_NB_CBKS_STAGE2_10MS ]`; identical in both preprocessor branches

```c
{
    {0, 1, 0},
    {0, 0, 1}
}
```

**`silk_CB_lags_stage3_10_ms`** — `silk/pitch_est_tables.c:41-45` — 2-D array; **declared ?, 2 top-level group(s), 24 scalar leaf/leaves in the initialiser**; `[ PE_MAX_NB_SUBFR >> 1 ][ PE_NB_CBKS_STAGE3_10MS ]`; identical in both preprocessor branches

```c
{
    { 0, 0, 1,-1, 1,-1, 2,-2, 2,-2, 3,-3},
    { 0, 1, 0, 1,-1, 2,-1, 2,-2, 3,-2, 3}
}
```

**`silk_Lag_range_stage3_10_ms`** — `silk/pitch_est_tables.c:47-51` — 2-D array; **declared ?, 2 top-level group(s), 4 scalar leaf/leaves in the initialiser**; `[ PE_MAX_NB_SUBFR >> 1 ][ 2 ]`; identical in both preprocessor branches

```c
{
    {-3, 7},
    {-2, 7}
}
```

**`silk_CB_lags_stage2`** — `silk/pitch_est_tables.c:53-59` — 2-D array; **declared ?, 4 top-level group(s), 44 scalar leaf/leaves in the initialiser**; `[ PE_MAX_NB_SUBFR ][ PE_NB_CBKS_STAGE2_EXT ]`; identical in both preprocessor branches

```c
{
    {0, 2,-1,-1,-1, 0, 0, 1, 1, 0, 1},
    {0, 1, 0, 0, 0, 0, 0, 1, 0, 0, 0},
    {0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 0},
    {0,-1, 2, 1, 0, 1, 1, 0, 0,-1,-1}
}
```

**`silk_CB_lags_stage3`** — `silk/pitch_est_tables.c:61-67` — 2-D array; **declared ?, 4 top-level group(s), 136 scalar leaf/leaves in the initialiser**; `[ PE_MAX_NB_SUBFR ][ PE_NB_CBKS_STAGE3_MAX ]`; identical in both preprocessor branches

```c
{
    {0, 0, 1,-1, 0, 1,-1, 0,-1, 1,-2, 2,-2,-2, 2,-3, 2, 3,-3,-4, 3,-4, 4, 4,-5, 5,-6,-5, 6,-7, 6, 5, 8,-9},
    {0, 0, 1, 0, 0, 0, 0, 0, 0, 0,-1, 1, 0, 0, 1,-1, 0, 1,-1,-1, 1,-1, 2, 1,-1, 2,-2,-2, 2,-2, 2, 2, 3,-3},
    {0, 1, 0, 0, 0, 0, 0, 0, 1, 0, 1, 0, 0, 1,-1, 1, 0, 0, 2, 1,-1, 2,-1,-1, 2,-1, 2, 2,-1, 3,-2,-2,-2, 3},
    {0, 1, 0, 0, 1, 0, 1,-1, 2,-1, 2,-1, 2, 3,-2, 3,-2,-2, 4, 4,-3, 5,-3,-4, 6,-4, 6, 5,-5, 8,-6,-5,-7, 9}
}
```

**`silk_Lag_range_stage3`** — `silk/pitch_est_tables.c:69-92` — 3-D array; **declared ?, 3 top-level group(s), 24 scalar leaf/leaves in the initialiser**; `[ SILK_PE_MAX_COMPLEX + 1 ] [ PE_MAX_NB_SUBFR ][ 2 ]`; identical in both preprocessor branches

```c
{
    /* Lags to search for low number of stage3 cbks */
    {
        {-5,8},
        {-1,6},
        {-1,6},
        {-4,10}
    },
    /* Lags to search for middle number of stage3 cbks */
    {
        {-6,10},
        {-2,6},
        {-1,6},
        {-5,10}
    },
    /* Lags to search for max number of stage3 cbks */
    {
        {-9,12},
        {-3,7},
        {-2,7},
        {-7,13}
    }
}
```

**`silk_nb_cbk_searchs_stage3`** — `silk/pitch_est_tables.c:94-99` — flat array; **declared ?, 3 top-level group(s), 3 scalar leaf/leaves in the initialiser**; `[ SILK_PE_MAX_COMPLEX + 1 ]`; identical in both preprocessor branches

```c
{
    PE_NB_CBKS_STAGE3_MIN,
    PE_NB_CBKS_STAGE3_MID,
    PE_NB_CBKS_STAGE3_MAX
}
```

#### `silk/resampler.c`

**`delay_matrix_enc`** — `silk/resampler.c:53-61` — 2-D array; **declared 18, 6 top-level group(s), 18 scalar leaf/leaves in the initialiser**; `[ 6 ][ 3 ]`; identical in both preprocessor branches

```c
{
/* in  \ out  8  12  16 */
/*  8 */   {  6,  0,  3 },
/* 12 */   {  0,  7,  3 },
/* 16 */   {  0,  1, 10 },
/* 24 */   {  0,  2,  6 },
/* 48 */   { 18, 10, 12 },
/* 96 */   {  0,  0,  44 }
}
```

**`delay_matrix_dec`** — `silk/resampler.c:63-68` — 2-D array; **declared 18, 3 top-level group(s), 18 scalar leaf/leaves in the initialiser**; `[ 3 ][ 6 ]`; identical in both preprocessor branches

```c
{
/* in  \ out  8  12  16  24  48  96*/
/*  8 */   {  4,  0,  2,  0,  0,  0 },
/* 12 */   {  0,  9,  4,  7,  4,  4 },
/* 16 */   {  0,  3, 12,  7,  7,  7 }
}
```

#### `silk/resampler_rom.c`

**`silk_Resampler_3_4_COEFS`** — `silk/resampler_rom.c:43-48` — flat array; **declared ?, 29 top-level group(s), 29 scalar leaf/leaves in the initialiser**; `[ 2 + 3 * RESAMPLER_DOWN_ORDER_FIR0 / 2 ]`; identical in both preprocessor branches

```c
{
    -20694, -13867,
       -49,     64,     17,   -157,    353,   -496,    163,  11047,  22205,
       -39,      6,     91,   -170,    186,     23,   -896,   6336,  19928,
       -19,    -36,    102,    -89,    -24,    328,   -951,   2568,  15909,
}
```

**`silk_Resampler_2_3_COEFS`** — `silk/resampler_rom.c:50-54` — flat array; **declared ?, 20 top-level group(s), 20 scalar leaf/leaves in the initialiser**; `[ 2 + 2 * RESAMPLER_DOWN_ORDER_FIR0 / 2 ]`; identical in both preprocessor branches

```c
{
    -14457, -14019,
        64,    128,   -122,     36,    310,   -768,    584,   9267,  17733,
        12,    128,     18,   -142,    288,   -117,   -865,   4123,  14459,
}
```

**`silk_Resampler_1_2_COEFS`** — `silk/resampler_rom.c:56-59` — flat array; **declared ?, 14 top-level group(s), 14 scalar leaf/leaves in the initialiser**; `[ 2 + RESAMPLER_DOWN_ORDER_FIR1 / 2 ]`; identical in both preprocessor branches

```c
{
       616, -14323,
       -10,     39,     58,    -46,    -84,    120,    184,   -315,   -541,   1284,   5380,   9024,
}
```

**`silk_Resampler_1_3_COEFS`** — `silk/resampler_rom.c:61-64` — flat array; **declared ?, 20 top-level group(s), 20 scalar leaf/leaves in the initialiser**; `[ 2 + RESAMPLER_DOWN_ORDER_FIR2 / 2 ]`; identical in both preprocessor branches

```c
{
     16102, -15162,
       -13,      0,     20,     26,      5,    -31,    -43,     -4,     65,     90,      7,   -157,   -248,    -44,    593,   1583,   2612,   3271,
}
```

**`silk_Resampler_1_4_COEFS`** — `silk/resampler_rom.c:66-69` — flat array; **declared ?, 20 top-level group(s), 20 scalar leaf/leaves in the initialiser**; `[ 2 + RESAMPLER_DOWN_ORDER_FIR2 / 2 ]`; identical in both preprocessor branches

```c
{
     22500, -15099,
         3,    -14,    -20,    -15,      2,     25,     37,     25,    -16,    -71,   -107,    -79,     50,    292,    623,    982,   1288,   1464,
}
```

**`silk_Resampler_1_6_COEFS`** — `silk/resampler_rom.c:71-74` — flat array; **declared ?, 20 top-level group(s), 20 scalar leaf/leaves in the initialiser**; `[ 2 + RESAMPLER_DOWN_ORDER_FIR2 / 2 ]`; identical in both preprocessor branches

```c
{
     27540, -15257,
        17,     12,      8,      1,    -10,    -22,    -30,    -32,    -22,      3,     44,    100,    168,    243,    317,    381,    429,    455,
}
```

**`silk_Resampler_2_3_COEFS_LQ`** — `silk/resampler_rom.c:76-80` — flat array; **declared ?, 6 top-level group(s), 6 scalar leaf/leaves in the initialiser**; `[ 2 + 2 * 2 ]`; identical in both preprocessor branches

```c
{
     -2797,  -6507,
      4697,  10739,
      1567,   8276,
}
```

**`silk_resampler_frac_FIR_12`** — `silk/resampler_rom.c:83-96` — 2-D array; **declared ?, 12 top-level group(s), 48 scalar leaf/leaves in the initialiser**; `[ 12 ][ RESAMPLER_ORDER_FIR_12 / 2 ]`; identical in both preprocessor branches

```c
{
    {  189,  -600,   617, 30567 },
    {  117,  -159, -1070, 29704 },
    {   52,   221, -2392, 28276 },
    {   -4,   529, -3350, 26341 },
    {  -48,   758, -3956, 23973 },
    {  -80,   905, -4235, 21254 },
    {  -99,   972, -4222, 18278 },
    { -107,   967, -3957, 15143 },
    { -103,   896, -3487, 11950 },
    {  -91,   773, -2865,  8798 },
    {  -71,   611, -2143,  5784 },
    {  -46,   425, -1375,  2996 },
}
```

#### `silk/resampler_rom.h`

**`silk_resampler_up2_hq_0`** — `silk/resampler_rom.h:45-45` — flat array; **declared 3, 3 top-level group(s), 3 scalar leaf/leaves in the initialiser**; `[ 3 ]`

```c
{ 1746, 14986, 39083 - 65536 }
```

**`silk_resampler_up2_hq_1`** — `silk/resampler_rom.h:46-46` — flat array; **declared 3, 3 top-level group(s), 3 scalar leaf/leaves in the initialiser**; `[ 3 ]`

```c
{ 6854, 25769, 55542 - 65536 }
```

#### `silk/sigm_Q15.c`

**`sigm_LUT_slope_Q10`** — `silk/sigm_Q15.c:37-39` — flat array; **declared 6, 6 top-level group(s), 6 scalar leaf/leaves in the initialiser**; `[ 6 ]`; identical in both preprocessor branches

```c
{
    237, 153, 73, 30, 12, 7
}
```

**`sigm_LUT_pos_Q15`** — `silk/sigm_Q15.c:41-43` — flat array; **declared 6, 6 top-level group(s), 6 scalar leaf/leaves in the initialiser**; `[ 6 ]`; identical in both preprocessor branches

```c
{
    16384, 23955, 28861, 31213, 32178, 32548
}
```

**`sigm_LUT_neg_Q15`** — `silk/sigm_Q15.c:45-47` — flat array; **declared 6, 6 top-level group(s), 6 scalar leaf/leaves in the initialiser**; `[ 6 ]`; identical in both preprocessor branches

```c
{
    16384, 8812, 3906, 1554, 589, 219
}
```

#### `silk/table_LSF_cos.c`

**`silk_LSFCosTab_FIX_Q12`** — `silk/table_LSF_cos.c:36-70` — flat array; **declared ?, 129 top-level group(s), 129 scalar leaf/leaves in the initialiser**; `[ LSF_COS_TAB_SZ_FIX + 1 ]`; identical in both preprocessor branches

```c
{
            8192,             8190,             8182,             8170,
            8152,             8130,             8104,             8072,
            8034,             7994,             7946,             7896,
            7840,             7778,             7714,             7644,
            7568,             7490,             7406,             7318,
            7226,             7128,             7026,             6922,
            6812,             6698,             6580,             6458,
            6332,             6204,             6070,             5934,
            5792,             5648,             5502,             5352,
            5198,             5040,             4880,             4718,
            4552,             4382,             4212,             4038,
            3862,             3684,             3502,             3320,
            3136,             2948,             2760,             2570,
            2378,             2186,             1990,             1794,
            1598,             1400,             1202,             1002,
             802,              602,              402,              202,
               0,             -202,             -402,             -602,
            -802,            -1002,            -1202,            -1400,
           -1598,            -1794,            -1990,            -2186,
           -2378,            -2570,            -2760,            -2948,
           -3136,            -3320,            -3502,            -3684,
           -3862,            -4038,            -4212,            -4382,
           -4552,            -4718,            -4880,            -5040,
           -5198,            -5352,            -5502,            -5648,
           -5792,            -5934,            -6070,            -6204,
           -6332,            -6458,            -6580,            -6698,
           -6812,            -6922,            -7026,            -7128,
           -7226,            -7318,            -7406,            -7490,
           -7568,            -7644,            -7714,            -7778,
           -7840,            -7896,            -7946,            -7994,
           -8034,            -8072,            -8104,            -8130,
           -8152,            -8170,            -8182,            -8190,
           -8192
}
```

#### `silk/tables_LTP.c`

**`silk_LTP_per_index_iCDF`** — `silk/tables_LTP.c:34-36` — flat array; **declared 3, 3 top-level group(s), 3 scalar leaf/leaves in the initialiser**; `[3]`; identical in both preprocessor branches

```c
{
       179,     99,      0
}
```

**`silk_LTP_gain_iCDF_0`** — `silk/tables_LTP.c:38-40` — flat array; **declared 8, 8 top-level group(s), 8 scalar leaf/leaves in the initialiser**; `[8]`; identical in both preprocessor branches

```c
{
        71,     56,     43,     30,     21,     12,      6,      0
}
```

**`silk_LTP_gain_iCDF_1`** — `silk/tables_LTP.c:42-45` — flat array; **declared 16, 16 top-level group(s), 16 scalar leaf/leaves in the initialiser**; `[16]`; identical in both preprocessor branches

```c
{
       199,    165,    144,    124,    109,     96,     84,     71,
        61,     51,     42,     32,     23,     15,      8,      0
}
```

**`silk_LTP_gain_iCDF_2`** — `silk/tables_LTP.c:47-52` — flat array; **declared 32, 32 top-level group(s), 32 scalar leaf/leaves in the initialiser**; `[32]`; identical in both preprocessor branches

```c
{
       241,    225,    211,    199,    187,    175,    164,    153,
       142,    132,    123,    114,    105,     96,     88,     80,
        72,     64,     57,     50,     44,     38,     33,     29,
        24,     20,     16,     12,      9,      5,      2,      0
}
```

**`silk_LTP_gain_BITS_Q5_0`** — `silk/tables_LTP.c:54-56` — flat array; **declared 8, 8 top-level group(s), 8 scalar leaf/leaves in the initialiser**; `[8]`; identical in both preprocessor branches

```c
{
        15,    131,    138,    138,    155,    155,    173,    173
}
```

**`silk_LTP_gain_BITS_Q5_1`** — `silk/tables_LTP.c:58-61` — flat array; **declared 16, 16 top-level group(s), 16 scalar leaf/leaves in the initialiser**; `[16]`; identical in both preprocessor branches

```c
{
        69,     93,    115,    118,    131,    138,    141,    138,
       150,    150,    155,    150,    155,    160,    166,    160
}
```

**`silk_LTP_gain_BITS_Q5_2`** — `silk/tables_LTP.c:63-68` — flat array; **declared 32, 32 top-level group(s), 32 scalar leaf/leaves in the initialiser**; `[32]`; identical in both preprocessor branches

```c
{
       131,    128,    134,    141,    141,    141,    145,    145,
       145,    150,    155,    155,    155,    155,    160,    160,
       160,    160,    166,    166,    173,    173,    182,    192,
       182,    192,    192,    192,    205,    192,    205,    224
}
```

**`silk_LTP_gain_iCDF_ptrs`** — `silk/tables_LTP.c:70-74` — flat array; **declared ?, 3 top-level group(s), 3 scalar leaf/leaves in the initialiser**; `[NB_LTP_CBKS]`; identical in both preprocessor branches

```c
{
    silk_LTP_gain_iCDF_0,
    silk_LTP_gain_iCDF_1,
    silk_LTP_gain_iCDF_2
}
```

**`silk_LTP_gain_BITS_Q5_ptrs`** — `silk/tables_LTP.c:76-80` — flat array; **declared ?, 3 top-level group(s), 3 scalar leaf/leaves in the initialiser**; `[NB_LTP_CBKS]`; identical in both preprocessor branches

```c
{
    silk_LTP_gain_BITS_Q5_0,
    silk_LTP_gain_BITS_Q5_1,
    silk_LTP_gain_BITS_Q5_2
}
```

**`silk_LTP_gain_vq_0`** — `silk/tables_LTP.c:82-108` — 2-D array; **declared 40, 8 top-level group(s), 40 scalar leaf/leaves in the initialiser**; `[8][5]`; identical in both preprocessor branches

```c
{
{
         4,      6,     24,      7,      5
},
{
         0,      0,      2,      0,      0
},
{
        12,     28,     41,     13,     -4
},
{
        -9,     15,     42,     25,     14
},
{
         1,     -2,     62,     41,     -9
},
{
       -10,     37,     65,     -4,      3
},
{
        -6,      4,     66,      7,     -8
},
{
        16,     14,     38,     -3,     33
}
}
```

**`silk_LTP_gain_vq_1`** — `silk/tables_LTP.c:110-160` — 2-D array; **declared 80, 16 top-level group(s), 80 scalar leaf/leaves in the initialiser**; `[16][5]`; identical in both preprocessor branches

```c
{
{
        13,     22,     39,     23,     12
},
{
        -1,     36,     64,     27,     -6
},
{
        -7,     10,     55,     43,     17
},
{
         1,      1,      8,      1,      1
},
{
         6,    -11,     74,     53,     -9
},
{
       -12,     55,     76,    -12,      8
},
{
        -3,      3,     93,     27,     -4
},
{
        26,     39,     59,      3,     -8
},
{
         2,      0,     77,     11,      9
},
{
        -8,     22,     44,     -6,      7
},
{
        40,      9,     26,      3,      9
},
{
        -7,     20,    101,     -7,      4
},
{
         3,     -8,     42,     26,      0
},
{
       -15,     33,     68,      2,     23
},
{
        -2,     55,     46,     -2,     15
},
{
         3,     -1,     21,     16,     41
}
}
```

**`silk_LTP_gain_vq_2`** — `silk/tables_LTP.c:162-260` — 2-D array; **declared 160, 32 top-level group(s), 160 scalar leaf/leaves in the initialiser**; `[32][5]`; identical in both preprocessor branches

```c
{
{
        -6,     27,     61,     39,      5
},
{
       -11,     42,     88,      4,      1
},
{
        -2,     60,     65,      6,     -4
},
{
        -1,     -5,     73,     56,      1
},
{
        -9,     19,     94,     29,     -9
},
{
         0,     12,     99,      6,      4
},
{
         8,    -19,    102,     46,    -13
},
{
         3,      2,     13,      3,      2
},
{
         9,    -21,     84,     72,    -18
},
{
       -11,     46,    104,    -22,      8
},
{
        18,     38,     48,     23,      0
},
{
       -16,     70,     83,    -21,     11
},
{
         5,    -11,    117,     22,     -8
},
{
        -6,     23,    117,    -12,      3
},
{
         3,     -8,     95,     28,      4
},
{
       -10,     15,     77,     60,    -15
},
{
        -1,      4,    124,      2,     -4
},
{
         3,     38,     84,     24,    -25
},
{
         2,     13,     42,     13,     31
},
{
        21,     -4,     56,     46,     -1
},
{
        -1,     35,     79,    -13,     19
},
{
        -7,     65,     88,     -9,    -14
},
{
        20,      4,     81,     49,    -29
},
{
        20,      0,     75,      3,    -17
},
{
         5,     -9,     44,     92,     -8
},
{
         1,     -3,     22,     69,     31
},
{
        -6,     95,     41,    -12,      5
},
{
        39,     67,     16,     -4,      1
},
{
         0,     -6,    120,     55,    -36
},
{
       -13,     44,    122,      4,    -24
},
{
        81,      5,     11,      3,      7
},
{
         2,      0,      9,     10,     88
}
}
```

**`silk_LTP_vq_ptrs_Q7`** — `silk/tables_LTP.c:262-266` — flat array; **declared ?, 3 top-level group(s), 3 scalar leaf/leaves in the initialiser**; `[NB_LTP_CBKS]`; identical in both preprocessor branches

```c
{
    (opus_int8 *)&silk_LTP_gain_vq_0[0][0],
    (opus_int8 *)&silk_LTP_gain_vq_1[0][0],
    (opus_int8 *)&silk_LTP_gain_vq_2[0][0]
}
```

**`silk_LTP_gain_vq_0_gain`** — `silk/tables_LTP.c:270-272` — flat array; **declared 8, 8 top-level group(s), 8 scalar leaf/leaves in the initialiser**; `[8]`; identical in both preprocessor branches

```c
{
      46,      2,     90,     87,     93,     91,     82,     98
}
```

**`silk_LTP_gain_vq_1_gain`** — `silk/tables_LTP.c:274-277` — flat array; **declared 16, 16 top-level group(s), 16 scalar leaf/leaves in the initialiser**; `[16]`; identical in both preprocessor branches

```c
{
     109,    120,    118,     12,    113,    115,    117,    119,
      99,     59,     87,    111,     63,    111,    112,     80
}
```

**`silk_LTP_gain_vq_2_gain`** — `silk/tables_LTP.c:279-284` — flat array; **declared 32, 32 top-level group(s), 32 scalar leaf/leaves in the initialiser**; `[32]`; identical in both preprocessor branches

```c
{
     126,    124,    125,    124,    129,    121,    126,     23,
     132,    127,    127,    127,    126,    127,    122,    133,
     130,    134,    101,    118,    119,    145,    126,     86,
     124,    120,    123,    119,    170,    173,    107,    109
}
```

**`silk_LTP_vq_gain_ptrs_Q7`** — `silk/tables_LTP.c:286-290` — flat array; **declared ?, 3 top-level group(s), 3 scalar leaf/leaves in the initialiser**; `[NB_LTP_CBKS]`; identical in both preprocessor branches

```c
{
    &silk_LTP_gain_vq_0_gain[0],
    &silk_LTP_gain_vq_1_gain[0],
    &silk_LTP_gain_vq_2_gain[0]
}
```

**`silk_LTP_vq_sizes`** — `silk/tables_LTP.c:292-294` — flat array; **declared ?, 3 top-level group(s), 3 scalar leaf/leaves in the initialiser**; `[NB_LTP_CBKS]`; identical in both preprocessor branches

```c
{
    8, 16, 32
}
```

#### `silk/tables_NLSF_CB_NB_MB.c`

**`silk_NLSF_CB1_NB_MB_Q8`** — `silk/tables_NLSF_CB_NB_MB.c:34-75` — flat array; **declared 320, 320 top-level group(s), 320 scalar leaf/leaves in the initialiser**; `[ 320 ]`; identical in both preprocessor branches

```c
{
        12,     35,     60,     83,    108,    132,    157,    180,
       206,    228,     15,     32,     55,     77,    101,    125,
       151,    175,    201,    225,     19,     42,     66,     89,
       114,    137,    162,    184,    209,    230,     12,     25,
        50,     72,     97,    120,    147,    172,    200,    223,
        26,     44,     69,     90,    114,    135,    159,    180,
       205,    225,     13,     22,     53,     80,    106,    130,
       156,    180,    205,    228,     15,     25,     44,     64,
        90,    115,    142,    168,    196,    222,     19,     24,
        62,     82,    100,    120,    145,    168,    190,    214,
        22,     31,     50,     79,    103,    120,    151,    170,
       203,    227,     21,     29,     45,     65,    106,    124,
       150,    171,    196,    224,     30,     49,     75,     97,
       121,    142,    165,    186,    209,    229,     19,     25,
        52,     70,     93,    116,    143,    166,    192,    219,
        26,     34,     62,     75,     97,    118,    145,    167,
       194,    217,     25,     33,     56,     70,     91,    113,
       143,    165,    196,    223,     21,     34,     51,     72,
        97,    117,    145,    171,    196,    222,     20,     29,
        50,     67,     90,    117,    144,    168,    197,    221,
        22,     31,     48,     66,     95,    117,    146,    168,
       196,    222,     24,     33,     51,     77,    116,    134,
       158,    180,    200,    224,     21,     28,     70,     87,
       106,    124,    149,    170,    194,    217,     26,     33,
        53,     64,     83,    117,    152,    173,    204,    225,
        27,     34,     65,     95,    108,    129,    155,    174,
       210,    225,     20,     26,     72,     99,    113,    131,
       154,    176,    200,    219,     34,     43,     61,     78,
        93,    114,    155,    177,    205,    229,     23,     29,
        54,     97,    124,    138,    163,    179,    209,    229,
        30,     38,     56,     89,    118,    129,    158,    178,
       200,    231,     21,     29,     49,     63,     85,    111,
       142,    163,    193,    222,     27,     48,     77,    103,
       133,    158,    179,    196,    215,    232,     29,     47,
        74,     99,    124,    151,    176,    198,    220,    237,
        33,     42,     61,     76,     93,    121,    155,    174,
       207,    225,     29,     53,     87,    112,    136,    154,
       170,    188,    208,    227,     24,     30,     52,     84,
       131,    150,    166,    186,    203,    229,     37,     48,
        64,     84,    104,    118,    156,    177,    201,    230
}
```

**`silk_NLSF_CB1_Wght_Q9`** — `silk/tables_NLSF_CB_NB_MB.c:77-110` — flat array; **declared 320, 320 top-level group(s), 320 scalar leaf/leaves in the initialiser**; `[ 320 ]`; identical in both preprocessor branches

```c
{
     2897, 2314, 2314, 2314, 2287, 2287, 2314, 2300, 2327, 2287,
     2888, 2580, 2394, 2367, 2314, 2274, 2274, 2274, 2274, 2194,
     2487, 2340, 2340, 2314, 2314, 2314, 2340, 2340, 2367, 2354,
     3216, 2766, 2340, 2340, 2314, 2274, 2221, 2207, 2261, 2194,
     2460, 2474, 2367, 2394, 2394, 2394, 2394, 2367, 2407, 2314,
     3479, 3056, 2127, 2207, 2274, 2274, 2274, 2287, 2314, 2261,
     3282, 3141, 2580, 2394, 2247, 2221, 2207, 2194, 2194, 2114,
     4096, 3845, 2221, 2620, 2620, 2407, 2314, 2394, 2367, 2074,
     3178, 3244, 2367, 2221, 2553, 2434, 2340, 2314, 2167, 2221,
     3338, 3488, 2726, 2194, 2261, 2460, 2354, 2367, 2207, 2101,
     2354, 2420, 2327, 2367, 2394, 2420, 2420, 2420, 2460, 2367,
     3779, 3629, 2434, 2527, 2367, 2274, 2274, 2300, 2207, 2048,
     3254, 3225, 2713, 2846, 2447, 2327, 2300, 2300, 2274, 2127,
     3263, 3300, 2753, 2806, 2447, 2261, 2261, 2247, 2127, 2101,
     2873, 2981, 2633, 2367, 2407, 2354, 2194, 2247, 2247, 2114,
     3225, 3197, 2633, 2580, 2274, 2181, 2247, 2221, 2221, 2141,
     3178, 3310, 2740, 2407, 2274, 2274, 2274, 2287, 2194, 2114,
     3141, 3272, 2460, 2061, 2287, 2500, 2367, 2487, 2434, 2181,
     3507, 3282, 2314, 2700, 2647, 2474, 2367, 2394, 2340, 2127,
     3423, 3535, 3038, 3056, 2300, 1950, 2221, 2274, 2274, 2274,
     3404, 3366, 2087, 2687, 2873, 2354, 2420, 2274, 2474, 2540,
     3760, 3488, 1950, 2660, 2897, 2527, 2394, 2367, 2460, 2261,
     3028, 3272, 2740, 2888, 2740, 2154, 2127, 2287, 2234, 2247,
     3695, 3657, 2025, 1969, 2660, 2700, 2580, 2500, 2327, 2367,
     3207, 3413, 2354, 2074, 2888, 2888, 2340, 2487, 2247, 2167,
     3338, 3366, 2846, 2780, 2327, 2154, 2274, 2287, 2114, 2061,
     2327, 2300, 2181, 2167, 2181, 2367, 2633, 2700, 2700, 2553,
     2407, 2434, 2221, 2261, 2221, 2221, 2340, 2420, 2607, 2700,
     3038, 3244, 2806, 2888, 2474, 2074, 2300, 2314, 2354, 2380,
     2221, 2154, 2127, 2287, 2500, 2793, 2793, 2620, 2580, 2367,
     3676, 3713, 2234, 1838, 2181, 2753, 2726, 2673, 2513, 2207,
     2793, 3160, 2726, 2553, 2846, 2513, 2181, 2394, 2221, 2181
}
```

**`silk_NLSF_CB1_iCDF_NB_MB`** — `silk/tables_NLSF_CB_NB_MB.c:112-121` — flat array; **declared 64, 64 top-level group(s), 64 scalar leaf/leaves in the initialiser**; `[ 64 ]`; identical in both preprocessor branches

```c
{
       212,    178,    148,    129,    108,     96,     85,     82,
        79,     77,     61,     59,     57,     56,     51,     49,
        48,     45,     42,     41,     40,     38,     36,     34,
        31,     30,     21,     12,     10,      3,      1,      0,
       255,    245,    244,    236,    233,    225,    217,    203,
       190,    176,    175,    161,    149,    136,    125,    114,
       102,     91,     81,     71,     60,     52,     43,     35,
        28,     20,     19,     18,     12,     11,      5,      0
}
```

**`silk_NLSF_CB2_SELECT_NB_MB`** — `silk/tables_NLSF_CB_NB_MB.c:123-144` — flat array; **declared 160, 160 top-level group(s), 160 scalar leaf/leaves in the initialiser**; `[ 160 ]`; identical in both preprocessor branches

```c
{
        16,      0,      0,      0,      0,     99,     66,     36,
        36,     34,     36,     34,     34,     34,     34,     83,
        69,     36,     52,     34,    116,    102,     70,     68,
        68,    176,    102,     68,     68,     34,     65,     85,
        68,     84,     36,    116,    141,    152,    139,    170,
       132,    187,    184,    216,    137,    132,    249,    168,
       185,    139,    104,    102,    100,     68,     68,    178,
       218,    185,    185,    170,    244,    216,    187,    187,
       170,    244,    187,    187,    219,    138,    103,    155,
       184,    185,    137,    116,    183,    155,    152,    136,
       132,    217,    184,    184,    170,    164,    217,    171,
       155,    139,    244,    169,    184,    185,    170,    164,
       216,    223,    218,    138,    214,    143,    188,    218,
       168,    244,    141,    136,    155,    170,    168,    138,
       220,    219,    139,    164,    219,    202,    216,    137,
       168,    186,    246,    185,    139,    116,    185,    219,
       185,    138,    100,    100,    134,    100,    102,     34,
        68,     68,    100,     68,    168,    203,    221,    218,
       168,    167,    154,    136,    104,     70,    164,    246,
       171,    137,    139,    137,    155,    218,    219,    139
}
```

**`silk_NLSF_CB2_iCDF_NB_MB`** — `silk/tables_NLSF_CB_NB_MB.c:146-156` — flat array; **declared 72, 72 top-level group(s), 72 scalar leaf/leaves in the initialiser**; `[ 72 ]`; identical in both preprocessor branches

```c
{
       255,    254,    253,    238,     14,      3,      2,      1,
         0,    255,    254,    252,    218,     35,      3,      2,
         1,      0,    255,    254,    250,    208,     59,      4,
         2,      1,      0,    255,    254,    246,    194,     71,
        10,      2,      1,      0,    255,    252,    236,    183,
        82,      8,      2,      1,      0,    255,    252,    235,
       180,     90,     17,      2,      1,      0,    255,    248,
       224,    171,     97,     30,      4,      1,      0,    255,
       254,    236,    173,     95,     37,      7,      1,      0
}
```

**`silk_NLSF_CB2_BITS_NB_MB_Q5`** — `silk/tables_NLSF_CB_NB_MB.c:158-168` — flat array; **declared 72, 72 top-level group(s), 72 scalar leaf/leaves in the initialiser**; `[ 72 ]`; identical in both preprocessor branches

```c
{
       255,    255,    255,    131,      6,    145,    255,    255,
       255,    255,    255,    236,     93,     15,     96,    255,
       255,    255,    255,    255,    194,     83,     25,     71,
       221,    255,    255,    255,    255,    162,     73,     34,
        66,    162,    255,    255,    255,    210,    126,     73,
        43,     57,    173,    255,    255,    255,    201,    125,
        71,     48,     58,    130,    255,    255,    255,    166,
       110,     73,     57,     62,    104,    210,    255,    255,
       251,    123,     65,     55,     68,    100,    171,    255
}
```

**`silk_NLSF_PRED_NB_MB_Q8`** — `silk/tables_NLSF_CB_NB_MB.c:170-174` — flat array; **declared 18, 18 top-level group(s), 18 scalar leaf/leaves in the initialiser**; `[ 18 ]`; identical in both preprocessor branches

```c
{
       179,    138,    140,    148,    151,    149,    153,    151,
       163,    116,     67,     82,     59,     92,     72,    100,
        89,     92
}
```

**`silk_NLSF_DELTA_MIN_NB_MB_Q15`** — `silk/tables_NLSF_CB_NB_MB.c:176-179` — flat array; **declared 11, 11 top-level group(s), 11 scalar leaf/leaves in the initialiser**; `[ 11 ]`; identical in both preprocessor branches

```c
{
       250,      3,      6,      3,      3,      3,      4,      3,
         3,      3,    461
}
```

#### `silk/tables_NLSF_CB_WB.c`

**`silk_NLSF_CB1_WB_Q8`** — `silk/tables_NLSF_CB_WB.c:34-99` — flat array; **declared 512, 512 top-level group(s), 512 scalar leaf/leaves in the initialiser**; `[ 512 ]`; identical in both preprocessor branches

```c
{
         7,     23,     38,     54,     69,     85,    100,    116,
       131,    147,    162,    178,    193,    208,    223,    239,
        13,     25,     41,     55,     69,     83,     98,    112,
       127,    142,    157,    171,    187,    203,    220,    236,
        15,     21,     34,     51,     61,     78,     92,    106,
       126,    136,    152,    167,    185,    205,    225,    240,
        10,     21,     36,     50,     63,     79,     95,    110,
       126,    141,    157,    173,    189,    205,    221,    237,
        17,     20,     37,     51,     59,     78,     89,    107,
       123,    134,    150,    164,    184,    205,    224,    240,
        10,     15,     32,     51,     67,     81,     96,    112,
       129,    142,    158,    173,    189,    204,    220,    236,
         8,     21,     37,     51,     65,     79,     98,    113,
       126,    138,    155,    168,    179,    192,    209,    218,
        12,     15,     34,     55,     63,     78,     87,    108,
       118,    131,    148,    167,    185,    203,    219,    236,
        16,     19,     32,     36,     56,     79,     91,    108,
       118,    136,    154,    171,    186,    204,    220,    237,
        11,     28,     43,     58,     74,     89,    105,    120,
       135,    150,    165,    180,    196,    211,    226,    241,
         6,     16,     33,     46,     60,     75,     92,    107,
       123,    137,    156,    169,    185,    199,    214,    225,
        11,     19,     30,     44,     57,     74,     89,    105,
       121,    135,    152,    169,    186,    202,    218,    234,
        12,     19,     29,     46,     57,     71,     88,    100,
       120,    132,    148,    165,    182,    199,    216,    233,
        17,     23,     35,     46,     56,     77,     92,    106,
       123,    134,    152,    167,    185,    204,    222,    237,
        14,     17,     45,     53,     63,     75,     89,    107,
       115,    132,    151,    171,    188,    206,    221,    240,
         9,     16,     29,     40,     56,     71,     88,    103,
       119,    137,    154,    171,    189,    205,    222,    237,
        16,     19,     36,     48,     57,     76,     87,    105,
       118,    132,    150,    167,    185,    202,    218,    236,
        12,     17,     29,     54,     71,     81,     94,    104,
       126,    136,    149,    164,    182,    201,    221,    237,
        15,     28,     47,     62,     79,     97,    115,    129,
       142,    155,    168,    180,    194,    208,    223,    238,
         8,     14,     30,     45,     62,     78,     94,    111,
       127,    143,    159,    175,    192,    207,    223,    239,
        17,     30,     49,     62,     79,     92,    107,    119,
       132,    145,    160,    174,    190,    204,    220,    235,
        14,     19,     36,     45,     61,     76,     91,    108,
       121,    138,    154,    172,    189,    205,    222,    238,
        12,     18,     31,     45,     60,     76,     91,    107,
       123,    138,    154,    171,    187,    204,    221,    236,
        13,     17,     31,     43,     53,     70,     83,    103,
       114,    131,    149,    167,    185,    203,    220,    237,
        17,     22,     35,     42,     58,     78,     93,    110,
       125,    139,    155,    170,    188,    206,    224,    240,
         8,     15,     34,     50,     67,     83,     99,    115,
       131,    146,    162,    178,    193,    209,    224,    239,
        13,     16,     41,     66,     73,     86,     95,    111,
       128,    137,    150,    163,    183,    206,    225,    241,
        17,     25,     37,     52,     63,     75,     92,    102,
       119,    132,    144,    160,    175,    191,    212,    231,
        19,     31,     49,     65,     83,    100,    117,    133,
       147,    161,    174,    187,    200,    213,    227,    242,
        18,     31,     52,     68,     88,    103,    117,    126,
       138,    149,    163,    177,    192,    207,    223,    239,
        16,     29,     47,     61,     76,     90,    106,    119,
       133,    147,    161,    176,    193,    209,    224,    240,
        15,     21,     35,     50,     61,     73,     86,     97,
       110,    119,    129,    141,    175,    198,    218,    237
}
```

**`silk_NLSF_CB1_WB_Wght_Q9`** — `silk/tables_NLSF_CB_WB.c:101-134` — flat array; **declared 512, 512 top-level group(s), 512 scalar leaf/leaves in the initialiser**; `[ 512 ]`; identical in both preprocessor branches

```c
{
     3657, 2925, 2925, 2925, 2925, 2925, 2925, 2925, 2925, 2925, 2925, 2925, 2963, 2963, 2925, 2846,
     3216, 3085, 2972, 3056, 3056, 3010, 3010, 3010, 2963, 2963, 3010, 2972, 2888, 2846, 2846, 2726,
     3920, 4014, 2981, 3207, 3207, 2934, 3056, 2846, 3122, 3244, 2925, 2846, 2620, 2553, 2780, 2925,
     3516, 3197, 3010, 3103, 3019, 2888, 2925, 2925, 2925, 2925, 2888, 2888, 2888, 2888, 2888, 2753,
     5054, 5054, 2934, 3573, 3385, 3056, 3085, 2793, 3160, 3160, 2972, 2846, 2513, 2540, 2753, 2888,
     4428, 4149, 2700, 2753, 2972, 3010, 2925, 2846, 2981, 3019, 2925, 2925, 2925, 2925, 2888, 2726,
     3620, 3019, 2972, 3056, 3056, 2873, 2806, 3056, 3216, 3047, 2981, 3291, 3291, 2981, 3310, 2991,
     5227, 5014, 2540, 3338, 3526, 3385, 3197, 3094, 3376, 2981, 2700, 2647, 2687, 2793, 2846, 2673,
     5081, 5174, 4615, 4428, 2460, 2897, 3047, 3207, 3169, 2687, 2740, 2888, 2846, 2793, 2846, 2700,
     3122, 2888, 2963, 2925, 2925, 2925, 2925, 2963, 2963, 2963, 2963, 2925, 2925, 2963, 2963, 2963,
     4202, 3207, 2981, 3103, 3010, 2888, 2888, 2925, 2972, 2873, 2916, 3019, 2972, 3010, 3197, 2873,
     3760, 3760, 3244, 3103, 2981, 2888, 2925, 2888, 2972, 2934, 2793, 2793, 2846, 2888, 2888, 2660,
     3854, 4014, 3207, 3122, 3244, 2934, 3047, 2963, 2963, 3085, 2846, 2793, 2793, 2793, 2793, 2580,
     3845, 4080, 3357, 3516, 3094, 2740, 3010, 2934, 3122, 3085, 2846, 2846, 2647, 2647, 2846, 2806,
     5147, 4894, 3225, 3845, 3441, 3169, 2897, 3413, 3451, 2700, 2580, 2673, 2740, 2846, 2806, 2753,
     4109, 3789, 3291, 3160, 2925, 2888, 2888, 2925, 2793, 2740, 2793, 2740, 2793, 2846, 2888, 2806,
     5081, 5054, 3047, 3545, 3244, 3056, 3085, 2944, 3103, 2897, 2740, 2740, 2740, 2846, 2793, 2620,
     4309, 4309, 2860, 2527, 3207, 3376, 3376, 3075, 3075, 3376, 3056, 2846, 2647, 2580, 2726, 2753,
     3056, 2916, 2806, 2888, 2740, 2687, 2897, 3103, 3150, 3150, 3216, 3169, 3056, 3010, 2963, 2846,
     4375, 3882, 2925, 2888, 2846, 2888, 2846, 2846, 2888, 2888, 2888, 2846, 2888, 2925, 2888, 2846,
     2981, 2916, 2916, 2981, 2981, 3056, 3122, 3216, 3150, 3056, 3010, 2972, 2972, 2972, 2925, 2740,
     4229, 4149, 3310, 3347, 2925, 2963, 2888, 2981, 2981, 2846, 2793, 2740, 2846, 2846, 2846, 2793,
     4080, 4014, 3103, 3010, 2925, 2925, 2925, 2888, 2925, 2925, 2846, 2846, 2846, 2793, 2888, 2780,
     4615, 4575, 3169, 3441, 3207, 2981, 2897, 3038, 3122, 2740, 2687, 2687, 2687, 2740, 2793, 2700,
     4149, 4269, 3789, 3657, 2726, 2780, 2888, 2888, 3010, 2972, 2925, 2846, 2687, 2687, 2793, 2888,
     4215, 3554, 2753, 2846, 2846, 2888, 2888, 2888, 2925, 2925, 2888, 2925, 2925, 2925, 2963, 2888,
     5174, 4921, 2261, 3432, 3789, 3479, 3347, 2846, 3310, 3479, 3150, 2897, 2460, 2487, 2753, 2925,
     3451, 3685, 3122, 3197, 3357, 3047, 3207, 3207, 2981, 3216, 3085, 2925, 2925, 2687, 2540, 2434,
     2981, 3010, 2793, 2793, 2740, 2793, 2846, 2972, 3056, 3103, 3150, 3150, 3150, 3103, 3010, 3010,
     2944, 2873, 2687, 2726, 2780, 3010, 3432, 3545, 3357, 3244, 3056, 3010, 2963, 2925, 2888, 2846,
     3019, 2944, 2897, 3010, 3010, 2972, 3019, 3103, 3056, 3056, 3010, 2888, 2846, 2925, 2925, 2888,
     3920, 3967, 3010, 3197, 3357, 3216, 3291, 3291, 3479, 3704, 3441, 2726, 2181, 2460, 2580, 2607
}
```

**`silk_NLSF_CB1_iCDF_WB`** — `silk/tables_NLSF_CB_WB.c:136-145` — flat array; **declared 64, 64 top-level group(s), 64 scalar leaf/leaves in the initialiser**; `[ 64 ]`; identical in both preprocessor branches

```c
{
       225,    204,    201,    184,    183,    175,    158,    154,
       153,    135,    119,    115,    113,    110,    109,     99,
        98,     95,     79,     68,     52,     50,     48,     45,
        43,     32,     31,     27,     18,     10,      3,      0,
       255,    251,    235,    230,    212,    201,    196,    182,
       167,    166,    163,    151,    138,    124,    110,    104,
        90,     78,     76,     70,     69,     57,     45,     34,
        24,     21,     11,      6,      5,      4,      3,      0
}
```

**`silk_NLSF_CB2_SELECT_WB`** — `silk/tables_NLSF_CB_WB.c:147-180` — flat array; **declared 256, 256 top-level group(s), 256 scalar leaf/leaves in the initialiser**; `[ 256 ]`; identical in both preprocessor branches

```c
{
         0,      0,      0,      0,      0,      0,      0,      1,
       100,    102,    102,     68,     68,     36,     34,     96,
       164,    107,    158,    185,    180,    185,    139,    102,
        64,     66,     36,     34,     34,      0,      1,     32,
       208,    139,    141,    191,    152,    185,    155,    104,
        96,    171,    104,    166,    102,    102,    102,    132,
         1,      0,      0,      0,      0,     16,     16,      0,
        80,    109,     78,    107,    185,    139,    103,    101,
       208,    212,    141,    139,    173,    153,    123,    103,
        36,      0,      0,      0,      0,      0,      0,      1,
        48,      0,      0,      0,      0,      0,      0,     32,
        68,    135,    123,    119,    119,    103,     69,     98,
        68,    103,    120,    118,    118,    102,     71,     98,
       134,    136,    157,    184,    182,    153,    139,    134,
       208,    168,    248,     75,    189,    143,    121,    107,
        32,     49,     34,     34,     34,      0,     17,      2,
       210,    235,    139,    123,    185,    137,    105,    134,
        98,    135,    104,    182,    100,    183,    171,    134,
       100,     70,     68,     70,     66,     66,     34,    131,
        64,    166,    102,     68,     36,      2,      1,      0,
       134,    166,    102,     68,     34,     34,     66,    132,
       212,    246,    158,    139,    107,    107,     87,    102,
       100,    219,    125,    122,    137,    118,    103,    132,
       114,    135,    137,    105,    171,    106,     50,     34,
       164,    214,    141,    143,    185,    151,    121,    103,
       192,     34,      0,      0,      0,      0,      0,      1,
       208,    109,     74,    187,    134,    249,    159,    137,
       102,    110,    154,    118,     87,    101,    119,    101,
         0,      2,      0,     36,     36,     66,     68,     35,
        96,    164,    102,    100,     36,      0,      2,     33,
       167,    138,    174,    102,    100,     84,      2,      2,
       100,    107,    120,    119,     36,    197,     24,      0
}
```

**`silk_NLSF_CB2_iCDF_WB`** — `silk/tables_NLSF_CB_WB.c:182-192` — flat array; **declared 72, 72 top-level group(s), 72 scalar leaf/leaves in the initialiser**; `[ 72 ]`; identical in both preprocessor branches

```c
{
       255,    254,    253,    244,     12,      3,      2,      1,
         0,    255,    254,    252,    224,     38,      3,      2,
         1,      0,    255,    254,    251,    209,     57,      4,
         2,      1,      0,    255,    254,    244,    195,     69,
         4,      2,      1,      0,    255,    251,    232,    184,
        84,      7,      2,      1,      0,    255,    254,    240,
       186,     86,     14,      2,      1,      0,    255,    254,
       239,    178,     91,     30,      5,      1,      0,    255,
       248,    227,    177,    100,     19,      2,      1,      0
}
```

**`silk_NLSF_CB2_BITS_WB_Q5`** — `silk/tables_NLSF_CB_WB.c:194-204` — flat array; **declared 72, 72 top-level group(s), 72 scalar leaf/leaves in the initialiser**; `[ 72 ]`; identical in both preprocessor branches

```c
{
       255,    255,    255,    156,      4,    154,    255,    255,
       255,    255,    255,    227,    102,     15,     92,    255,
       255,    255,    255,    255,    213,     83,     24,     72,
       236,    255,    255,    255,    255,    150,     76,     33,
        63,    214,    255,    255,    255,    190,    121,     77,
        43,     55,    185,    255,    255,    255,    245,    137,
        71,     43,     59,    139,    255,    255,    255,    255,
       131,     66,     50,     66,    107,    194,    255,    255,
       166,    116,     76,     55,     53,    125,    255,    255
}
```

**`silk_NLSF_PRED_WB_Q8`** — `silk/tables_NLSF_CB_WB.c:206-211` — flat array; **declared 30, 30 top-level group(s), 30 scalar leaf/leaves in the initialiser**; `[ 30 ]`; identical in both preprocessor branches

```c
{
       175,    148,    160,    176,    178,    173,    174,    164,
       177,    174,    196,    182,    198,    192,    182,     68,
        62,     66,     60,     72,    117,     85,     90,    118,
       136,    151,    142,    160,    142,    155
}
```

**`silk_NLSF_DELTA_MIN_WB_Q15`** — `silk/tables_NLSF_CB_WB.c:213-217` — flat array; **declared 17, 17 top-level group(s), 17 scalar leaf/leaves in the initialiser**; `[ 17 ]`; identical in both preprocessor branches

```c
{
       100,      3,     40,      3,      3,      3,      5,     14,
        14,     10,     11,      3,      8,      9,      7,      3,
       347
}
```

#### `silk/tables_gain.c`

**`silk_gain_iCDF`** — `silk/tables_gain.c:35-46` — 2-D array; **declared ?, 3 top-level group(s), 24 scalar leaf/leaves in the initialiser**; `[ 3 ][ N_LEVELS_QGAIN / 8 ]`; identical in both preprocessor branches

```c
{
{
       224,    112,     44,     15,      3,      2,      1,      0
},
{
       254,    237,    192,    132,     70,     23,      4,      0
},
{
       255,    252,    226,    155,     61,     11,      2,      0
}
}
```

**`silk_delta_gain_iCDF`** — `silk/tables_gain.c:48-55` — flat array; **declared ?, 41 top-level group(s), 41 scalar leaf/leaves in the initialiser**; `[ MAX_DELTA_GAIN_QUANT - MIN_DELTA_GAIN_QUANT + 1 ]`; identical in both preprocessor branches

```c
{
       250,    245,    234,    203,     71,     50,     42,     38,
        35,     33,     31,     29,     28,     27,     26,     25,
        24,     23,     22,     21,     20,     19,     18,     17,
        16,     15,     14,     13,     12,     11,     10,      9,
         8,      7,      6,      5,      4,      3,      2,      1,
         0
}
```

#### `silk/tables_other.c`

**`silk_stereo_pred_quant_Q13`** — `silk/tables_other.c:38-41` — flat array; **declared ?, 16 top-level group(s), 16 scalar leaf/leaves in the initialiser**; `[ STEREO_QUANT_TAB_SIZE ]`; identical in both preprocessor branches

```c
{
    -13732, -10050, -8266, -7526, -6500, -5000, -2950,  -820,
       820,   2950,  5000,  6500,  7526,  8266, 10050, 13732
}
```

**`silk_stereo_pred_joint_iCDF`** — `silk/tables_other.c:42-48` — flat array; **declared 25, 25 top-level group(s), 25 scalar leaf/leaves in the initialiser**; `[ 25 ]`; identical in both preprocessor branches

```c
{
    249, 247, 246, 245, 244,
    234, 210, 202, 201, 200,
    197, 174,  82,  59,  56,
     55,  54,  46,  22,  12,
     11,  10,   9,   7,   0
}
```

**`silk_stereo_only_code_mid_iCDF`** — `silk/tables_other.c:49-49` — flat array; **declared 2, 2 top-level group(s), 2 scalar leaf/leaves in the initialiser**; `[ 2 ]`; identical in both preprocessor branches

```c
{ 64, 0 }
```

**`silk_LBRR_flags_2_iCDF`** — `silk/tables_other.c:52-52` — flat array; **declared 3, 3 top-level group(s), 3 scalar leaf/leaves in the initialiser**; `[ 3 ]`; identical in both preprocessor branches

```c
{ 203, 150, 0 }
```

**`silk_LBRR_flags_3_iCDF`** — `silk/tables_other.c:53-53` — flat array; **declared 7, 7 top-level group(s), 7 scalar leaf/leaves in the initialiser**; `[ 7 ]`; identical in both preprocessor branches

```c
{ 215, 195, 166, 125, 110, 82, 0 }
```

**`silk_LBRR_flags_iCDF_ptr`** — `silk/tables_other.c:54-57` — flat array; **declared 2, 2 top-level group(s), 2 scalar leaf/leaves in the initialiser**; `[ 2 ]`; identical in both preprocessor branches

```c
{
    silk_LBRR_flags_2_iCDF,
    silk_LBRR_flags_3_iCDF
}
```

**`silk_lsb_iCDF`** — `silk/tables_other.c:60-60` — flat array; **declared 2, 2 top-level group(s), 2 scalar leaf/leaves in the initialiser**; `[ 2 ]`; identical in both preprocessor branches

```c
{ 120, 0 }
```

**`silk_LTPscale_iCDF`** — `silk/tables_other.c:63-63` — flat array; **declared 3, 3 top-level group(s), 3 scalar leaf/leaves in the initialiser**; `[ 3 ]`; identical in both preprocessor branches

```c
{ 128, 64, 0 }
```

**`silk_type_offset_VAD_iCDF`** — `silk/tables_other.c:66-68` — flat array; **declared 4, 4 top-level group(s), 4 scalar leaf/leaves in the initialiser**; `[ 4 ]`; identical in both preprocessor branches

```c
{
       232,    158,    10,      0
}
```

**`silk_type_offset_no_VAD_iCDF`** — `silk/tables_other.c:69-71` — flat array; **declared 2, 2 top-level group(s), 2 scalar leaf/leaves in the initialiser**; `[ 2 ]`; identical in both preprocessor branches

```c
{
       230,      0
}
```

**`silk_NLSF_interpolation_factor_iCDF`** — `silk/tables_other.c:74-74` — flat array; **declared 5, 5 top-level group(s), 5 scalar leaf/leaves in the initialiser**; `[ 5 ]`; identical in both preprocessor branches

```c
{ 243, 221, 192, 181, 0 }
```

**`silk_Quantization_Offsets_Q10`** — `silk/tables_other.c:77-79` — 2-D array; **declared 4, 2 top-level group(s), 4 scalar leaf/leaves in the initialiser**; `[ 2 ][ 2 ]`; identical in both preprocessor branches

```c
{
    { OFFSET_UVL_Q10, OFFSET_UVH_Q10 }, { OFFSET_VL_Q10, OFFSET_VH_Q10 }
}
```

**`silk_LTPScales_table_Q14`** — `silk/tables_other.c:82-82` — flat array; **declared 3, 3 top-level group(s), 3 scalar leaf/leaves in the initialiser**; `[ 3 ]`; identical in both preprocessor branches

```c
{ 15565, 12288, 8192 }
```

**`silk_uniform3_iCDF`** — `silk/tables_other.c:85-85` — flat array; **declared 3, 3 top-level group(s), 3 scalar leaf/leaves in the initialiser**; `[ 3 ]`; identical in both preprocessor branches

```c
{ 171, 85, 0 }
```

**`silk_uniform4_iCDF`** — `silk/tables_other.c:86-86` — flat array; **declared 4, 4 top-level group(s), 4 scalar leaf/leaves in the initialiser**; `[ 4 ]`; identical in both preprocessor branches

```c
{ 192, 128, 64, 0 }
```

**`silk_uniform5_iCDF`** — `silk/tables_other.c:87-87` — flat array; **declared 5, 5 top-level group(s), 5 scalar leaf/leaves in the initialiser**; `[ 5 ]`; identical in both preprocessor branches

```c
{ 205, 154, 102, 51, 0 }
```

**`silk_uniform6_iCDF`** — `silk/tables_other.c:88-88` — flat array; **declared 6, 6 top-level group(s), 6 scalar leaf/leaves in the initialiser**; `[ 6 ]`; identical in both preprocessor branches

```c
{ 213, 171, 128, 85, 43, 0 }
```

**`silk_uniform8_iCDF`** — `silk/tables_other.c:89-89` — flat array; **declared 8, 8 top-level group(s), 8 scalar leaf/leaves in the initialiser**; `[ 8 ]`; identical in both preprocessor branches

```c
{ 224, 192, 160, 128, 96, 64, 32, 0 }
```

**`silk_NLSF_EXT_iCDF`** — `silk/tables_other.c:91-91` — flat array; **declared 7, 7 top-level group(s), 7 scalar leaf/leaves in the initialiser**; `[ 7 ]`; identical in both preprocessor branches

```c
{ 100, 40, 16, 7, 3, 1, 0 }
```

**`silk_Transition_LP_B_Q28`** — `silk/tables_other.c:98-105` — 2-D array; **declared ?, 5 top-level group(s), 15 scalar leaf/leaves in the initialiser**; `[ TRANSITION_INT_NUM ][ TRANSITION_NB ]`; identical in both preprocessor branches

```c
{
{    250767114,  501534038,  250767114  },
{    209867381,  419732057,  209867381  },
{    170987846,  341967853,  170987846  },
{    131531482,  263046905,  131531482  },
{     89306658,  178584282,   89306658  }
}
```

**`silk_Transition_LP_A_Q28`** — `silk/tables_other.c:108-115` — 2-D array; **declared ?, 5 top-level group(s), 10 scalar leaf/leaves in the initialiser**; `[ TRANSITION_INT_NUM ][ TRANSITION_NA ]`; identical in both preprocessor branches

```c
{
{    506393414,  239854379  },
{    411067935,  169683996  },
{    306733530,  116694253  },
{    185807084,   77959395  },
{     35497197,   57401098  }
}
```

#### `silk/tables_pitch_lag.c`

**`silk_pitch_lag_iCDF`** — `silk/tables_pitch_lag.c:34-39` — flat array; **declared ?, 32 top-level group(s), 32 scalar leaf/leaves in the initialiser**; `[ 2 * ( PITCH_EST_MAX_LAG_MS - PITCH_EST_MIN_LAG_MS ) ]`; identical in both preprocessor branches

```c
{
       253,    250,    244,    233,    212,    182,    150,    131,
       120,    110,     98,     85,     72,     60,     49,     40,
        32,     25,     19,     15,     13,     11,      9,      8,
         7,      6,      5,      4,      3,      2,      1,      0
}
```

**`silk_pitch_delta_iCDF`** — `silk/tables_pitch_lag.c:41-45` — flat array; **declared 21, 21 top-level group(s), 21 scalar leaf/leaves in the initialiser**; `[21]`; identical in both preprocessor branches

```c
{
       210,    208,    206,    203,    199,    193,    183,    168,
       142,    104,     74,     52,     37,     27,     20,     14,
        10,      6,      4,      2,      0
}
```

**`silk_pitch_contour_iCDF`** — `silk/tables_pitch_lag.c:47-53` — flat array; **declared 34, 34 top-level group(s), 34 scalar leaf/leaves in the initialiser**; `[34]`; identical in both preprocessor branches

```c
{
       223,    201,    183,    167,    152,    138,    124,    111,
        98,     88,     79,     70,     62,     56,     50,     44,
        39,     35,     31,     27,     24,     21,     18,     16,
        14,     12,     10,      8,      6,      4,      3,      2,
         1,      0
}
```

**`silk_pitch_contour_NB_iCDF`** — `silk/tables_pitch_lag.c:55-58` — flat array; **declared 11, 11 top-level group(s), 11 scalar leaf/leaves in the initialiser**; `[11]`; identical in both preprocessor branches

```c
{
       188,    176,    155,    138,    119,     97,     67,     43,
        26,     10,      0
}
```

**`silk_pitch_contour_10_ms_iCDF`** — `silk/tables_pitch_lag.c:60-63` — flat array; **declared 12, 12 top-level group(s), 12 scalar leaf/leaves in the initialiser**; `[12]`; identical in both preprocessor branches

```c
{
       165,    119,     80,     61,     47,     35,     27,     20,
        14,      9,      4,      0
}
```

**`silk_pitch_contour_10_ms_NB_iCDF`** — `silk/tables_pitch_lag.c:65-67` — flat array; **declared 3, 3 top-level group(s), 3 scalar leaf/leaves in the initialiser**; `[3]`; identical in both preprocessor branches

```c
{
       113,     63,      0
}
```

#### `silk/tables_pulses_per_block.c`

**`silk_max_pulses_table`** — `silk/tables_pulses_per_block.c:34-36` — flat array; **declared 4, 4 top-level group(s), 4 scalar leaf/leaves in the initialiser**; `[ 4 ]`; identical in both preprocessor branches

```c
{
         8,     10,     12,     16
}
```

**`silk_pulses_per_block_iCDF`** — `silk/tables_pulses_per_block.c:38-89` — 2-D array; **declared 180, 10 top-level group(s), 180 scalar leaf/leaves in the initialiser**; `[ 10 ][ 18 ]`; identical in both preprocessor branches

```c
{
{
       125,     51,     26,     18,     15,     12,     11,     10,
         9,      8,      7,      6,      5,      4,      3,      2,
         1,      0
},
{
       198,    105,     45,     22,     15,     12,     11,     10,
         9,      8,      7,      6,      5,      4,      3,      2,
         1,      0
},
{
       213,    162,    116,     83,     59,     43,     32,     24,
        18,     15,     12,      9,      7,      6,      5,      3,
         2,      0
},
{
       239,    187,    116,     59,     28,     16,     11,     10,
         9,      8,      7,      6,      5,      4,      3,      2,
         1,      0
},
{
       250,    229,    188,    135,     86,     51,     30,     19,
        13,     10,      8,      6,      5,      4,      3,      2,
         1,      0
},
{
       249,    235,    213,    185,    156,    128,    103,     83,
        66,     53,     42,     33,     26,     21,     17,     13,
        10,      0
},
{
       254,    249,    235,    206,    164,    118,     77,     46,
        27,     16,     10,      7,      5,      4,      3,      2,
         1,      0
},
{
       255,    253,    249,    239,    220,    191,    156,    119,
        85,     57,     37,     23,     15,     10,      6,      4,
         2,      0
},
{
       255,    253,    251,    246,    237,    223,    203,    179,
       152,    124,     98,     75,     55,     40,     29,     21,
        15,      0
},
{
       255,    254,    253,    247,    220,    162,    106,     67,
        42,     28,     18,     12,      9,      6,      4,      3,
         2,      0
}
}
```

**`silk_pulses_per_block_BITS_Q5`** — `silk/tables_pulses_per_block.c:91-137` — 2-D array; **declared 162, 9 top-level group(s), 162 scalar leaf/leaves in the initialiser**; `[ 9 ][ 18 ]`; identical in both preprocessor branches

```c
{
{
        31,     57,    107,    160,    205,    205,    255,    255,
       255,    255,    255,    255,    255,    255,    255,    255,
       255,    255
},
{
        69,     47,     67,    111,    166,    205,    255,    255,
       255,    255,    255,    255,    255,    255,    255,    255,
       255,    255
},
{
        82,     74,     79,     95,    109,    128,    145,    160,
       173,    205,    205,    205,    224,    255,    255,    224,
       255,    224
},
{
       125,     74,     59,     69,     97,    141,    182,    255,
       255,    255,    255,    255,    255,    255,    255,    255,
       255,    255
},
{
       173,    115,     85,     73,     76,     92,    115,    145,
       173,    205,    224,    224,    255,    255,    255,    255,
       255,    255
},
{
       166,    134,    113,    102,    101,    102,    107,    118,
       125,    138,    145,    155,    166,    182,    192,    192,
       205,    150
},
{
       224,    182,    134,    101,     83,     79,     85,     97,
       120,    145,    173,    205,    224,    255,    255,    255,
       255,    255
},
{
       255,    224,    192,    150,    120,    101,     92,     89,
        93,    102,    118,    134,    160,    182,    192,    224,
       224,    224
},
{
       255,    224,    224,    182,    155,    134,    118,    109,
       104,    102,    106,    111,    118,    131,    145,    160,
       173,    131
}
}
```

**`silk_rate_levels_iCDF`** — `silk/tables_pulses_per_block.c:139-149` — 2-D array; **declared 18, 2 top-level group(s), 18 scalar leaf/leaves in the initialiser**; `[ 2 ][ 9 ]`; identical in both preprocessor branches

```c
{
{
       241,    190,    178,    132,     87,     74,     41,     14,
         0
},
{
       223,    193,    157,    140,    106,     57,     39,     18,
         0
}
}
```

**`silk_rate_levels_BITS_Q5`** — `silk/tables_pulses_per_block.c:151-161` — 2-D array; **declared 18, 2 top-level group(s), 18 scalar leaf/leaves in the initialiser**; `[ 2 ][ 9 ]`; identical in both preprocessor branches

```c
{
{
       131,     74,    141,     79,     80,    138,     95,    104,
       134
},
{
        95,     99,     91,    125,     93,     76,    123,    115,
       123
}
}
```

**`silk_shell_code_table0`** — `silk/tables_pulses_per_block.c:163-183` — flat array; **declared 152, 152 top-level group(s), 152 scalar leaf/leaves in the initialiser**; `[ 152 ]`; identical in both preprocessor branches

```c
{
       128,      0,    214,     42,      0,    235,    128,     21,
         0,    244,    184,     72,     11,      0,    248,    214,
       128,     42,      7,      0,    248,    225,    170,     80,
        25,      5,      0,    251,    236,    198,    126,     54,
        18,      3,      0,    250,    238,    211,    159,     82,
        35,     15,      5,      0,    250,    231,    203,    168,
       128,     88,     53,     25,      6,      0,    252,    238,
       216,    185,    148,    108,     71,     40,     18,      4,
         0,    253,    243,    225,    199,    166,    128,     90,
        57,     31,     13,      3,      0,    254,    246,    233,
       212,    183,    147,    109,     73,     44,     23,     10,
         2,      0,    255,    250,    240,    223,    198,    166,
       128,     90,     58,     33,     16,      6,      1,      0,
       255,    251,    244,    231,    210,    181,    146,    110,
        75,     46,     25,     12,      5,      1,      0,    255,
       253,    248,    238,    221,    196,    164,    128,     92,
        60,     35,     18,      8,      3,      1,      0,    255,
       253,    249,    242,    229,    208,    180,    146,    110,
        76,     48,     27,     14,      7,      3,      1,      0
}
```

**`silk_shell_code_table1`** — `silk/tables_pulses_per_block.c:185-205` — flat array; **declared 152, 152 top-level group(s), 152 scalar leaf/leaves in the initialiser**; `[ 152 ]`; identical in both preprocessor branches

```c
{
       129,      0,    207,     50,      0,    236,    129,     20,
         0,    245,    185,     72,     10,      0,    249,    213,
       129,     42,      6,      0,    250,    226,    169,     87,
        27,      4,      0,    251,    233,    194,    130,     62,
        20,      4,      0,    250,    236,    207,    160,     99,
        47,     17,      3,      0,    255,    240,    217,    182,
       131,     81,     41,     11,      1,      0,    255,    254,
       233,    201,    159,    107,     61,     20,      2,      1,
         0,    255,    249,    233,    206,    170,    128,     86,
        50,     23,      7,      1,      0,    255,    250,    238,
       217,    186,    148,    108,     70,     39,     18,      6,
         1,      0,    255,    252,    243,    226,    200,    166,
       128,     90,     56,     30,     13,      4,      1,      0,
       255,    252,    245,    231,    209,    180,    146,    110,
        76,     47,     25,     11,      4,      1,      0,    255,
       253,    248,    237,    219,    194,    163,    128,     93,
        62,     37,     19,      8,      3,      1,      0,    255,
       254,    250,    241,    226,    205,    177,    145,    111,
        79,     51,     30,     15,      6,      2,      1,      0
}
```

**`silk_shell_code_table2`** — `silk/tables_pulses_per_block.c:207-227` — flat array; **declared 152, 152 top-level group(s), 152 scalar leaf/leaves in the initialiser**; `[ 152 ]`; identical in both preprocessor branches

```c
{
       129,      0,    203,     54,      0,    234,    129,     23,
         0,    245,    184,     73,     10,      0,    250,    215,
       129,     41,      5,      0,    252,    232,    173,     86,
        24,      3,      0,    253,    240,    200,    129,     56,
        15,      2,      0,    253,    244,    217,    164,     94,
        38,     10,      1,      0,    253,    245,    226,    189,
       132,     71,     27,      7,      1,      0,    253,    246,
       231,    203,    159,    105,     56,     23,      6,      1,
         0,    255,    248,    235,    213,    179,    133,     85,
        47,     19,      5,      1,      0,    255,    254,    243,
       221,    194,    159,    117,     70,     37,     12,      2,
         1,      0,    255,    254,    248,    234,    208,    171,
       128,     85,     48,     22,      8,      2,      1,      0,
       255,    254,    250,    240,    220,    189,    149,    107,
        67,     36,     16,      6,      2,      1,      0,    255,
       254,    251,    243,    227,    201,    166,    128,     90,
        55,     29,     13,      5,      2,      1,      0,    255,
       254,    252,    246,    234,    213,    183,    147,    109,
        73,     43,     22,     10,      4,      2,      1,      0
}
```

**`silk_shell_code_table3`** — `silk/tables_pulses_per_block.c:229-249` — flat array; **declared 152, 152 top-level group(s), 152 scalar leaf/leaves in the initialiser**; `[ 152 ]`; identical in both preprocessor branches

```c
{
       130,      0,    200,     58,      0,    231,    130,     26,
         0,    244,    184,     76,     12,      0,    249,    214,
       130,     43,      6,      0,    252,    232,    173,     87,
        24,      3,      0,    253,    241,    203,    131,     56,
        14,      2,      0,    254,    246,    221,    167,     94,
        35,      8,      1,      0,    254,    249,    232,    193,
       130,     65,     23,      5,      1,      0,    255,    251,
       239,    211,    162,     99,     45,     15,      4,      1,
         0,    255,    251,    243,    223,    186,    131,     74,
        33,     11,      3,      1,      0,    255,    252,    245,
       230,    202,    158,    105,     57,     24,      8,      2,
         1,      0,    255,    253,    247,    235,    214,    179,
       132,     84,     44,     19,      7,      2,      1,      0,
       255,    254,    250,    240,    223,    196,    159,    112,
        69,     36,     15,      6,      2,      1,      0,    255,
       254,    253,    245,    231,    209,    176,    136,     93,
        55,     27,     11,      3,      2,      1,      0,    255,
       254,    253,    252,    239,    221,    194,    158,    117,
        76,     42,     18,      4,      3,      2,      1,      0
}
```

**`silk_shell_code_table_offsets`** — `silk/tables_pulses_per_block.c:251-255` — flat array; **declared 17, 17 top-level group(s), 17 scalar leaf/leaves in the initialiser**; `[ 17 ]`; identical in both preprocessor branches

```c
{
         0,      0,      2,      5,      9,     14,     20,     27,
        35,     44,     54,     65,     77,     90,    104,    119,
       135
}
```

**`silk_sign_iCDF`** — `silk/tables_pulses_per_block.c:257-264` — flat array; **declared 42, 42 top-level group(s), 42 scalar leaf/leaves in the initialiser**; `[ 42 ]`; identical in both preprocessor branches

```c
{
       254,     49,     67,     77,     82,     93,     99,
       198,     11,     18,     24,     31,     36,     45,
       255,     46,     66,     78,     87,     94,    104,
       208,     14,     21,     32,     42,     51,     66,
       255,     94,    104,    109,    112,    115,    118,
       248,     53,     69,     80,     88,     95,    102
}
```

### 6.4 Opus-layer tables

#### `src/analysis.c`

**`dct_table`** — `src/analysis.c:57-74` — flat array; **declared 128, 128 top-level group(s), 128 scalar leaf/leaves in the initialiser**; `[128]`; **`#if !DISABLE_FLOAT_API`**

```c
{
        0.250000f, 0.250000f, 0.250000f, 0.250000f, 0.250000f, 0.250000f, 0.250000f, 0.250000f,
        0.250000f, 0.250000f, 0.250000f, 0.250000f, 0.250000f, 0.250000f, 0.250000f, 0.250000f,
        0.351851f, 0.338330f, 0.311806f, 0.273300f, 0.224292f, 0.166664f, 0.102631f, 0.034654f,
       -0.034654f,-0.102631f,-0.166664f,-0.224292f,-0.273300f,-0.311806f,-0.338330f,-0.351851f,
        0.346760f, 0.293969f, 0.196424f, 0.068975f,-0.068975f,-0.196424f,-0.293969f,-0.346760f,
       -0.346760f,-0.293969f,-0.196424f,-0.068975f, 0.068975f, 0.196424f, 0.293969f, 0.346760f,
        0.338330f, 0.224292f, 0.034654f,-0.166664f,-0.311806f,-0.351851f,-0.273300f,-0.102631f,
        0.102631f, 0.273300f, 0.351851f, 0.311806f, 0.166664f,-0.034654f,-0.224292f,-0.338330f,
        0.326641f, 0.135299f,-0.135299f,-0.326641f,-0.326641f,-0.135299f, 0.135299f, 0.326641f,
        0.326641f, 0.135299f,-0.135299f,-0.326641f,-0.326641f,-0.135299f, 0.135299f, 0.326641f,
        0.311806f, 0.034654f,-0.273300f,-0.338330f,-0.102631f, 0.224292f, 0.351851f, 0.166664f,
       -0.166664f,-0.351851f,-0.224292f, 0.102631f, 0.338330f, 0.273300f,-0.034654f,-0.311806f,
        0.293969f,-0.068975f,-0.346760f,-0.196424f, 0.196424f, 0.346760f, 0.068975f,-0.293969f,
       -0.293969f, 0.068975f, 0.346760f, 0.196424f,-0.196424f,-0.346760f,-0.068975f, 0.293969f,
        0.273300f,-0.166664f,-0.338330f, 0.034654f, 0.351851f, 0.102631f,-0.311806f,-0.224292f,
        0.224292f, 0.311806f,-0.102631f,-0.351851f,-0.034654f, 0.338330f, 0.166664f,-0.273300f,
}
```

**`analysis_window`** — `src/analysis.c:76-107` — flat array; **declared 240, 240 top-level group(s), 240 scalar leaf/leaves in the initialiser**; `[240]`; **`#if !DISABLE_FLOAT_API`**

```c
{
      0.000043f, 0.000171f, 0.000385f, 0.000685f, 0.001071f, 0.001541f, 0.002098f, 0.002739f,
      0.003466f, 0.004278f, 0.005174f, 0.006156f, 0.007222f, 0.008373f, 0.009607f, 0.010926f,
      0.012329f, 0.013815f, 0.015385f, 0.017037f, 0.018772f, 0.020590f, 0.022490f, 0.024472f,
      0.026535f, 0.028679f, 0.030904f, 0.033210f, 0.035595f, 0.038060f, 0.040604f, 0.043227f,
      0.045928f, 0.048707f, 0.051564f, 0.054497f, 0.057506f, 0.060591f, 0.063752f, 0.066987f,
      0.070297f, 0.073680f, 0.077136f, 0.080665f, 0.084265f, 0.087937f, 0.091679f, 0.095492f,
      0.099373f, 0.103323f, 0.107342f, 0.111427f, 0.115579f, 0.119797f, 0.124080f, 0.128428f,
      0.132839f, 0.137313f, 0.141849f, 0.146447f, 0.151105f, 0.155823f, 0.160600f, 0.165435f,
      0.170327f, 0.175276f, 0.180280f, 0.185340f, 0.190453f, 0.195619f, 0.200838f, 0.206107f,
      0.211427f, 0.216797f, 0.222215f, 0.227680f, 0.233193f, 0.238751f, 0.244353f, 0.250000f,
      0.255689f, 0.261421f, 0.267193f, 0.273005f, 0.278856f, 0.284744f, 0.290670f, 0.296632f,
      0.302628f, 0.308658f, 0.314721f, 0.320816f, 0.326941f, 0.333097f, 0.339280f, 0.345492f,
      0.351729f, 0.357992f, 0.364280f, 0.370590f, 0.376923f, 0.383277f, 0.389651f, 0.396044f,
      0.402455f, 0.408882f, 0.415325f, 0.421783f, 0.428254f, 0.434737f, 0.441231f, 0.447736f,
      0.454249f, 0.460770f, 0.467298f, 0.473832f, 0.480370f, 0.486912f, 0.493455f, 0.500000f,
      0.506545f, 0.513088f, 0.519630f, 0.526168f, 0.532702f, 0.539230f, 0.545751f, 0.552264f,
      0.558769f, 0.565263f, 0.571746f, 0.578217f, 0.584675f, 0.591118f, 0.597545f, 0.603956f,
      0.610349f, 0.616723f, 0.623077f, 0.629410f, 0.635720f, 0.642008f, 0.648271f, 0.654508f,
      0.660720f, 0.666903f, 0.673059f, 0.679184f, 0.685279f, 0.691342f, 0.697372f, 0.703368f,
      0.709330f, 0.715256f, 0.721144f, 0.726995f, 0.732807f, 0.738579f, 0.744311f, 0.750000f,
      0.755647f, 0.761249f, 0.766807f, 0.772320f, 0.777785f, 0.783203f, 0.788573f, 0.793893f,
      0.799162f, 0.804381f, 0.809547f, 0.814660f, 0.819720f, 0.824724f, 0.829673f, 0.834565f,
      0.839400f, 0.844177f, 0.848895f, 0.853553f, 0.858151f, 0.862687f, 0.867161f, 0.871572f,
      0.875920f, 0.880203f, 0.884421f, 0.888573f, 0.892658f, 0.896677f, 0.900627f, 0.904508f,
      0.908321f, 0.912063f, 0.915735f, 0.919335f, 0.922864f, 0.926320f, 0.929703f, 0.933013f,
      0.936248f, 0.939409f, 0.942494f, 0.945503f, 0.948436f, 0.951293f, 0.954072f, 0.956773f,
      0.959396f, 0.961940f, 0.964405f, 0.966790f, 0.969096f, 0.971321f, 0.973465f, 0.975528f,
      0.977510f, 0.979410f, 0.981228f, 0.982963f, 0.984615f, 0.986185f, 0.987671f, 0.989074f,
      0.990393f, 0.991627f, 0.992778f, 0.993844f, 0.994826f, 0.995722f, 0.996534f, 0.997261f,
      0.997902f, 0.998459f, 0.998929f, 0.999315f, 0.999615f, 0.999829f, 0.999957f, 1.000000f,
}
```

**`tbands`** — `src/analysis.c:109-111` — flat array; **declared ?, 19 top-level group(s), 19 scalar leaf/leaves in the initialiser**; `[NB_TBANDS+1]`; **`#if !DISABLE_FLOAT_API`**

```c
{
      4, 8, 12, 16, 20, 24, 28, 32, 40, 48, 56, 64, 80, 96, 112, 136, 160, 192, 240
}
```

**`std_feature_bias`** — `src/analysis.c:410-413` — flat array; **declared 9, 9 top-level group(s), 9 scalar leaf/leaves in the initialiser**; `[9]`; **`#if !DISABLE_FLOAT_API`**

```c
{
      5.684947f, 3.475288f, 1.770634f, 1.599784f, 3.773215f,
      2.163313f, 1.260756f, 1.116868f, 1.918795f
}
```

#### `src/extensions.c`

**`ext`** — `src/extensions.c:645-649` — array of aggregates (nested initialiser); **declared ?, 4 top-level group(s), 16 scalar leaf/leaves in the initialiser**; `[]`; function-local `static`

```c
{{2, 0, (const unsigned char *)"a", 1},
   {32, 10, (const unsigned char *)"DRED", 4},
   {33, 1, (const unsigned char *)"NOT DRED", 8},
   {3, 4, (const unsigned char *)NULL, 0}
   }
```

#### `src/mapping_matrix.c`

**`mapping_matrix_foa_mixing_data`** — `src/mapping_matrix.c:290-296` — flat array; **declared 36, 36 top-level group(s), 36 scalar leaf/leaves in the initialiser**; `[36]`; identical in both preprocessor branches; non-core (wrapper / float-API); values elided

```c
const opus_int16[36] = { ... };
```
_Full initialiser: `src/mapping_matrix.c:290-296` — extract with `sed -n '290,296p' src/mapping_matrix.c`._


**`mapping_matrix_soa_mixing_data`** — `src/mapping_matrix.c:299-316` — flat array; **declared 121, 121 top-level group(s), 121 scalar leaf/leaves in the initialiser**; `[121]`; identical in both preprocessor branches; non-core (wrapper / float-API); values elided

```c
const opus_int16[121] = { ... };
```
_Full initialiser: `src/mapping_matrix.c:299-316` — extract with `sed -n '299,316p' src/mapping_matrix.c`._


**`mapping_matrix_toa_mixing_data`** — `src/mapping_matrix.c:319-361` — flat array; **declared 324, 324 top-level group(s), 324 scalar leaf/leaves in the initialiser**; `[324]`; identical in both preprocessor branches; non-core (wrapper / float-API); values elided

```c
const opus_int16[324] = { ... };
```
_Full initialiser: `src/mapping_matrix.c:319-361` — extract with `sed -n '319,361p' src/mapping_matrix.c`._


**`mapping_matrix_fourthoa_mixing_data`** — `src/mapping_matrix.c:364-457` — flat array; **declared 729, 729 top-level group(s), 729 scalar leaf/leaves in the initialiser**; `[729]`; identical in both preprocessor branches; non-core (wrapper / float-API); values elided

```c
const opus_int16[729] = { ... };
```
_Full initialiser: `src/mapping_matrix.c:364-457` — extract with `sed -n '364,457p' src/mapping_matrix.c`._


**`mapping_matrix_fifthoa_mixing_data`** — `src/mapping_matrix.c:460-642` — flat array; **declared 1444, 1444 top-level group(s), 1444 scalar leaf/leaves in the initialiser**; `[1444]`; identical in both preprocessor branches; non-core (wrapper / float-API); values elided

```c
const opus_int16[1444] = { ... };
```
_Full initialiser: `src/mapping_matrix.c:460-642` — extract with `sed -n '460,642p' src/mapping_matrix.c`._


**`mapping_matrix_foa_demixing_data`** — `src/mapping_matrix.c:645-651` — flat array; **declared 36, 36 top-level group(s), 36 scalar leaf/leaves in the initialiser**; `[36]`; identical in both preprocessor branches; non-core (wrapper / float-API); values elided

```c
const opus_int16[36] = { ... };
```
_Full initialiser: `src/mapping_matrix.c:645-651` — extract with `sed -n '645,651p' src/mapping_matrix.c`._


**`mapping_matrix_soa_demixing_data`** — `src/mapping_matrix.c:654-671` — flat array; **declared 121, 121 top-level group(s), 121 scalar leaf/leaves in the initialiser**; `[121]`; identical in both preprocessor branches; non-core (wrapper / float-API); values elided

```c
const opus_int16[121] = { ... };
```
_Full initialiser: `src/mapping_matrix.c:654-671` — extract with `sed -n '654,671p' src/mapping_matrix.c`._


**`mapping_matrix_toa_demixing_data`** — `src/mapping_matrix.c:674-716` — flat array; **declared 324, 324 top-level group(s), 324 scalar leaf/leaves in the initialiser**; `[324]`; identical in both preprocessor branches; non-core (wrapper / float-API); values elided

```c
const opus_int16[324] = { ... };
```
_Full initialiser: `src/mapping_matrix.c:674-716` — extract with `sed -n '674,716p' src/mapping_matrix.c`._


**`mapping_matrix_fourthoa_demixing_data`** — `src/mapping_matrix.c:719-812` — flat array; **declared 729, 729 top-level group(s), 729 scalar leaf/leaves in the initialiser**; `[729]`; identical in both preprocessor branches; non-core (wrapper / float-API); values elided

```c
const opus_int16[729] = { ... };
```
_Full initialiser: `src/mapping_matrix.c:719-812` — extract with `sed -n '719,812p' src/mapping_matrix.c`._


**`mapping_matrix_fifthoa_demixing_data`** — `src/mapping_matrix.c:815-997` — flat array; **declared 1444, 1444 top-level group(s), 1444 scalar leaf/leaves in the initialiser**; `[1444]`; identical in both preprocessor branches; non-core (wrapper / float-API); values elided

```c
const opus_int16[1444] = { ... };
```
_Full initialiser: `src/mapping_matrix.c:815-997` — extract with `sed -n '815,997p' src/mapping_matrix.c`._


#### `src/mlp_data.c`

**`layer0_weights`** — `src/mlp_data.c:9-110` — flat array; **declared 800, 800 top-level group(s), 800 scalar leaf/leaves in the initialiser**; `[800]`; identical in both preprocessor branches; non-core (wrapper / float-API); values elided

```c
static const opus_int8[800] = { ... };
```
_Full initialiser: `src/mlp_data.c:9-110` — extract with `sed -n '9,110p' src/mlp_data.c`._


**`layer0_bias`** — `src/mlp_data.c:112-117` — flat array; **declared 32, 32 top-level group(s), 32 scalar leaf/leaves in the initialiser**; `[32]`; identical in both preprocessor branches; non-core (wrapper / float-API); values elided

```c
static const opus_int8[32] = { ... };
```
_Full initialiser: `src/mlp_data.c:112-117` — extract with `sed -n '112,117p' src/mlp_data.c`._


**`layer1_weights`** — `src/mlp_data.c:119-408` — flat array; **declared 2304, 2304 top-level group(s), 2304 scalar leaf/leaves in the initialiser**; `[2304]`; identical in both preprocessor branches; non-core (wrapper / float-API); values elided

```c
static const opus_int8[2304] = { ... };
```
_Full initialiser: `src/mlp_data.c:119-408` — extract with `sed -n '119,408p' src/mlp_data.c`._


**`layer1_recur_weights`** — `src/mlp_data.c:410-627` — flat array; **declared 1728, 1728 top-level group(s), 1728 scalar leaf/leaves in the initialiser**; `[1728]`; identical in both preprocessor branches; non-core (wrapper / float-API); values elided

```c
static const opus_int8[1728] = { ... };
```
_Full initialiser: `src/mlp_data.c:410-627` — extract with `sed -n '410,627p' src/mlp_data.c`._


**`layer1_bias`** — `src/mlp_data.c:629-639` — flat array; **declared 72, 72 top-level group(s), 72 scalar leaf/leaves in the initialiser**; `[72]`; identical in both preprocessor branches; non-core (wrapper / float-API); values elided

```c
static const opus_int8[72] = { ... };
```
_Full initialiser: `src/mlp_data.c:629-639` — extract with `sed -n '629,639p' src/mlp_data.c`._


**`layer2_weights`** — `src/mlp_data.c:641-648` — flat array; **declared 48, 48 top-level group(s), 48 scalar leaf/leaves in the initialiser**; `[48]`; identical in both preprocessor branches; non-core (wrapper / float-API); values elided

```c
static const opus_int8[48] = { ... };
```
_Full initialiser: `src/mlp_data.c:641-648` — extract with `sed -n '641,648p' src/mlp_data.c`._


**`layer2_bias`** — `src/mlp_data.c:650-652` — flat array; **declared 2, 2 top-level group(s), 2 scalar leaf/leaves in the initialiser**; `[2]`; identical in both preprocessor branches; non-core (wrapper / float-API); values elided

```c
static const opus_int8[2] = { ... };
```
_Full initialiser: `src/mlp_data.c:650-652` — extract with `sed -n '650,652p' src/mlp_data.c`._


#### `src/opus_compare.c`

**`p`** — `src/opus_compare.c:122-122` — flat array; **declared 2, 1 top-level group(s), 1 scalar leaf/leaves in the initialiser**; `[2]`; identical in both preprocessor branches; function-local `static`

```c
{0}
```

**`BANDS`** — `src/opus_compare.c:158-160` — flat array; **declared ?, 22 top-level group(s), 22 scalar leaf/leaves in the initialiser**; `[NBANDS+1]`; identical in both preprocessor branches

```c
{
  0,2,4,6,8,10,12,14,16,20,24,28,32,40,48,56,68,80,96,120,156,200
}
```

#### `src/opus_decoder.c`

**`silence`** — `src/opus_decoder.c:614-614` — flat array; **declared 2, 2 top-level group(s), 2 scalar leaf/leaves in the initialiser**; `[2]`; identical in both preprocessor branches; function-local `static`

```c
{0xFF, 0xFF}
```

#### `src/opus_demo.c`

**`format_size`** — `src/opus_demo.c:162-162` — flat array; **declared 3, 3 top-level group(s), 3 scalar leaf/leaves in the initialiser**; `[3]`; identical in both preprocessor branches

```c
{2, 3, 4}
```

**`silk8_test`** — `src/opus_demo.c:186-195` — 2-D array; **declared ?, 8 top-level group(s), 32 scalar leaf/leaves in the initialiser**; `[][4]`; identical in both preprocessor branches

```c
{
      {MODE_SILK_ONLY, OPUS_BANDWIDTH_NARROWBAND, 960*3, 1},
      {MODE_SILK_ONLY, OPUS_BANDWIDTH_NARROWBAND, 960*2, 1},
      {MODE_SILK_ONLY, OPUS_BANDWIDTH_NARROWBAND, 960,   1},
      {MODE_SILK_ONLY, OPUS_BANDWIDTH_NARROWBAND, 480,   1},
      {MODE_SILK_ONLY, OPUS_BANDWIDTH_NARROWBAND, 960*3, 2},
      {MODE_SILK_ONLY, OPUS_BANDWIDTH_NARROWBAND, 960*2, 2},
      {MODE_SILK_ONLY, OPUS_BANDWIDTH_NARROWBAND, 960,   2},
      {MODE_SILK_ONLY, OPUS_BANDWIDTH_NARROWBAND, 480,   2}
}
```

**`silk12_test`** — `src/opus_demo.c:197-206` — 2-D array; **declared ?, 8 top-level group(s), 32 scalar leaf/leaves in the initialiser**; `[][4]`; identical in both preprocessor branches

```c
{
      {MODE_SILK_ONLY, OPUS_BANDWIDTH_MEDIUMBAND, 960*3, 1},
      {MODE_SILK_ONLY, OPUS_BANDWIDTH_MEDIUMBAND, 960*2, 1},
      {MODE_SILK_ONLY, OPUS_BANDWIDTH_MEDIUMBAND, 960,   1},
      {MODE_SILK_ONLY, OPUS_BANDWIDTH_MEDIUMBAND, 480,   1},
      {MODE_SILK_ONLY, OPUS_BANDWIDTH_MEDIUMBAND, 960*3, 2},
      {MODE_SILK_ONLY, OPUS_BANDWIDTH_MEDIUMBAND, 960*2, 2},
      {MODE_SILK_ONLY, OPUS_BANDWIDTH_MEDIUMBAND, 960,   2},
      {MODE_SILK_ONLY, OPUS_BANDWIDTH_MEDIUMBAND, 480,   2}
}
```

**`silk16_test`** — `src/opus_demo.c:208-217` — 2-D array; **declared ?, 8 top-level group(s), 32 scalar leaf/leaves in the initialiser**; `[][4]`; identical in both preprocessor branches

```c
{
      {MODE_SILK_ONLY, OPUS_BANDWIDTH_WIDEBAND, 960*3, 1},
      {MODE_SILK_ONLY, OPUS_BANDWIDTH_WIDEBAND, 960*2, 1},
      {MODE_SILK_ONLY, OPUS_BANDWIDTH_WIDEBAND, 960,   1},
      {MODE_SILK_ONLY, OPUS_BANDWIDTH_WIDEBAND, 480,   1},
      {MODE_SILK_ONLY, OPUS_BANDWIDTH_WIDEBAND, 960*3, 2},
      {MODE_SILK_ONLY, OPUS_BANDWIDTH_WIDEBAND, 960*2, 2},
      {MODE_SILK_ONLY, OPUS_BANDWIDTH_WIDEBAND, 960,   2},
      {MODE_SILK_ONLY, OPUS_BANDWIDTH_WIDEBAND, 480,   2}
}
```

**`silk_bw_switch_test`** — `src/opus_demo.c:219-240` — 2-D array; **declared ?, 20 top-level group(s), 80 scalar leaf/leaves in the initialiser**; `[][4]`; identical in both preprocessor branches

```c
{
    {MODE_SILK_ONLY, OPUS_BANDWIDTH_WIDEBAND,       960, 1},
    {MODE_SILK_ONLY, OPUS_BANDWIDTH_NARROWBAND,     960, 1},
    {MODE_SILK_ONLY, OPUS_BANDWIDTH_MEDIUMBAND,     960, 1},
    {MODE_SILK_ONLY, OPUS_BANDWIDTH_SUPERWIDEBAND,  960, 1},
    {MODE_SILK_ONLY, OPUS_BANDWIDTH_FULLBAND,       960, 1},
    {MODE_SILK_ONLY, OPUS_BANDWIDTH_WIDEBAND,       960, 2},
    {MODE_SILK_ONLY, OPUS_BANDWIDTH_NARROWBAND,     960, 2},
    {MODE_SILK_ONLY, OPUS_BANDWIDTH_MEDIUMBAND,     960, 2},
    {MODE_SILK_ONLY, OPUS_BANDWIDTH_SUPERWIDEBAND,  960, 2},
    {MODE_SILK_ONLY, OPUS_BANDWIDTH_FULLBAND,       960, 2},
    {MODE_SILK_ONLY, OPUS_BANDWIDTH_WIDEBAND,       480, 1},
    {MODE_SILK_ONLY, OPUS_BANDWIDTH_NARROWBAND,     480, 1},
    {MODE_SILK_ONLY, OPUS_BANDWIDTH_MEDIUMBAND,     480, 1},
    {MODE_SILK_ONLY, OPUS_BANDWIDTH_SUPERWIDEBAND,  480, 1},
    {MODE_SILK_ONLY, OPUS_BANDWIDTH_FULLBAND,       480, 1},
    {MODE_SILK_ONLY, OPUS_BANDWIDTH_WIDEBAND,       480, 2},
    {MODE_SILK_ONLY, OPUS_BANDWIDTH_NARROWBAND,     480, 2},
    {MODE_SILK_ONLY, OPUS_BANDWIDTH_MEDIUMBAND,     480, 2},
    {MODE_SILK_ONLY, OPUS_BANDWIDTH_SUPERWIDEBAND,  480, 2},
    {MODE_SILK_ONLY, OPUS_BANDWIDTH_FULLBAND,       480, 2}
}
```

**`hybrid24_test`** — `src/opus_demo.c:242-247` — 2-D array; **declared ?, 4 top-level group(s), 16 scalar leaf/leaves in the initialiser**; `[][4]`; identical in both preprocessor branches

```c
{
      {MODE_SILK_ONLY, OPUS_BANDWIDTH_SUPERWIDEBAND, 960, 1},
      {MODE_SILK_ONLY, OPUS_BANDWIDTH_SUPERWIDEBAND, 480, 1},
      {MODE_SILK_ONLY, OPUS_BANDWIDTH_SUPERWIDEBAND, 960, 2},
      {MODE_SILK_ONLY, OPUS_BANDWIDTH_SUPERWIDEBAND, 480, 2}
}
```

**`hybrid48_test`** — `src/opus_demo.c:249-254` — 2-D array; **declared ?, 4 top-level group(s), 16 scalar leaf/leaves in the initialiser**; `[][4]`; identical in both preprocessor branches

```c
{
      {MODE_SILK_ONLY, OPUS_BANDWIDTH_FULLBAND, 960, 1},
      {MODE_SILK_ONLY, OPUS_BANDWIDTH_FULLBAND, 480, 1},
      {MODE_SILK_ONLY, OPUS_BANDWIDTH_FULLBAND, 960, 2},
      {MODE_SILK_ONLY, OPUS_BANDWIDTH_FULLBAND, 480, 2}
}
```

**`celt_test`** — `src/opus_demo.c:256-297` — 2-D array; **declared ?, 32 top-level group(s), 128 scalar leaf/leaves in the initialiser**; `[][4]`; identical in both preprocessor branches

```c
{
      {MODE_CELT_ONLY, OPUS_BANDWIDTH_FULLBAND,      960, 1},
      {MODE_CELT_ONLY, OPUS_BANDWIDTH_SUPERWIDEBAND, 960, 1},
      {MODE_CELT_ONLY, OPUS_BANDWIDTH_WIDEBAND,      960, 1},
      {MODE_CELT_ONLY, OPUS_BANDWIDTH_NARROWBAND,    960, 1},

      {MODE_CELT_ONLY, OPUS_BANDWIDTH_FULLBAND,      480, 1},
      {MODE_CELT_ONLY, OPUS_BANDWIDTH_SUPERWIDEBAND, 480, 1},
      {MODE_CELT_ONLY, OPUS_BANDWIDTH_WIDEBAND,      480, 1},
      {MODE_CELT_ONLY, OPUS_BANDWIDTH_NARROWBAND,    480, 1},

      {MODE_CELT_ONLY, OPUS_BANDWIDTH_FULLBAND,      240, 1},
      {MODE_CELT_ONLY, OPUS_BANDWIDTH_SUPERWIDEBAND, 240, 1},
      {MODE_CELT_ONLY, OPUS_BANDWIDTH_WIDEBAND,      240, 1},
      {MODE_CELT_ONLY, OPUS_BANDWIDTH_NARROWBAND,    240, 1},

      {MODE_CELT_ONLY, OPUS_BANDWIDTH_FULLBAND,      120, 1},
      {MODE_CELT_ONLY, OPUS_BANDWIDTH_SUPERWIDEBAND, 120, 1},
      {MODE_CELT_ONLY, OPUS_BANDWIDTH_WIDEBAND,      120, 1},
      {MODE_CELT_ONLY, OPUS_BANDWIDTH_NARROWBAND,    120, 1},

      {MODE_CELT_ONLY, OPUS_BANDWIDTH_FULLBAND,      960, 2},
      {MODE_CELT_ONLY, OPUS_BANDWIDTH_SUPERWIDEBAND, 960, 2},
      {MODE_CELT_ONLY, OPUS_BANDWIDTH_WIDEBAND,      960, 2},
      {MODE_CELT_ONLY, OPUS_BANDWIDTH_NARROWBAND,    960, 2},

      {MODE_CELT_ONLY, OPUS_BANDWIDTH_FULLBAND,      480, 2},
      {MODE_CELT_ONLY, OPUS_BANDWIDTH_SUPERWIDEBAND, 480, 2},
      {MODE_CELT_ONLY, OPUS_BANDWIDTH_WIDEBAND,      480, 2},
      {MODE_CELT_ONLY, OPUS_BANDWIDTH_NARROWBAND,    480, 2},

      {MODE_CELT_ONLY, OPUS_BANDWIDTH_FULLBAND,      240, 2},
      {MODE_CELT_ONLY, OPUS_BANDWIDTH_SUPERWIDEBAND, 240, 2},
      {MODE_CELT_ONLY, OPUS_BANDWIDTH_WIDEBAND,      240, 2},
      {MODE_CELT_ONLY, OPUS_BANDWIDTH_NARROWBAND,    240, 2},

      {MODE_CELT_ONLY, OPUS_BANDWIDTH_FULLBAND,      120, 2},
      {MODE_CELT_ONLY, OPUS_BANDWIDTH_SUPERWIDEBAND, 120, 2},
      {MODE_CELT_ONLY, OPUS_BANDWIDTH_WIDEBAND,      120, 2},
      {MODE_CELT_ONLY, OPUS_BANDWIDTH_NARROWBAND,    120, 2},

}
```

**`celt_hq_test`** — `src/opus_demo.c:299-304` — 2-D array; **declared ?, 4 top-level group(s), 16 scalar leaf/leaves in the initialiser**; `[][4]`; identical in both preprocessor branches

```c
{
      {MODE_CELT_ONLY, OPUS_BANDWIDTH_FULLBAND,      120, 2},
      {MODE_CELT_ONLY, OPUS_BANDWIDTH_FULLBAND,      240, 2},
      {MODE_CELT_ONLY, OPUS_BANDWIDTH_FULLBAND,      480, 2},
      {MODE_CELT_ONLY, OPUS_BANDWIDTH_FULLBAND,      960, 2},
}
```

**`mapping`** — `src/opus_demo.c:328-328` — flat array; **declared 256, 2 top-level group(s), 2 scalar leaf/leaves in the initialiser**; `[256]`; function-local `static`

```c
{0,1}
```

**`bitrates`** — `src/opus_demo.c:347-360` — flat array; **declared ?, 102 top-level group(s), 102 scalar leaf/leaves in the initialiser**; `[NUM_BITRATES]`; **`#if ENABLE_OSCE_TRAINING_DATA`**

```c
{
        6000,  6060,  6120,  6180,  6240,  6300,  6360,  6420,  6480,
        6525,  6561,  6598,  6634,  6670,  6707,  6743,  6780,  6816,
        6853,  6889,  6926,  6962,  6999,  7042,  7085,  7128,  7171,
        7215,  7258,  7301,  7344,  7388,  7431,  7474,  7512,  7541,
        7570,  7599,  7628,  7657,  7686,  7715,  7744,  7773,  7802,
        7831,  7860,  7889,  7918,  7947,  7976,  8013,  8096,  8179,
        8262,  8344,  8427,  8511,  8605,  8699,  8792,  8886,  8980,
        9100,  9227,  9354,  9480,  9561,  9634,  9706,  9779,  9851,
        9924,  9996, 10161, 10330, 10499, 10698, 10898, 11124, 11378,
       11575, 11719, 11862, 12014, 12345, 12751, 13195, 13561, 13795,
       14069, 14671, 15403, 15790, 16371, 17399, 17968, 19382, 20468,
       22000, 32000, 64000
}
```

#### `src/opus_encoder.c`

**`mono_voice_bandwidth_thresholds`** — `src/opus_encoder.c:151-156` — flat array; **declared 8, 8 top-level group(s), 8 scalar leaf/leaves in the initialiser**; `[8]`; identical in both preprocessor branches

```c
{
         9000,  700, /* NB<->MB */
         9000,  700, /* MB<->WB */
        13500, 1000, /* WB<->SWB */
        14000, 2000, /* SWB<->FB */
}
```

**`mono_music_bandwidth_thresholds`** — `src/opus_encoder.c:157-162` — flat array; **declared 8, 8 top-level group(s), 8 scalar leaf/leaves in the initialiser**; `[8]`; identical in both preprocessor branches

```c
{
         9000,  700, /* NB<->MB */
         9000,  700, /* MB<->WB */
        11000, 1000, /* WB<->SWB */
        12000, 2000, /* SWB<->FB */
}
```

**`stereo_voice_bandwidth_thresholds`** — `src/opus_encoder.c:163-168` — flat array; **declared 8, 8 top-level group(s), 8 scalar leaf/leaves in the initialiser**; `[8]`; identical in both preprocessor branches

```c
{
         9000,  700, /* NB<->MB */
         9000,  700, /* MB<->WB */
        13500, 1000, /* WB<->SWB */
        14000, 2000, /* SWB<->FB */
}
```

**`stereo_music_bandwidth_thresholds`** — `src/opus_encoder.c:169-174` — flat array; **declared 8, 8 top-level group(s), 8 scalar leaf/leaves in the initialiser**; `[8]`; identical in both preprocessor branches

```c
{
         9000,  700, /* NB<->MB */
         9000,  700, /* MB<->WB */
        11000, 1000, /* WB<->SWB */
        12000, 2000, /* SWB<->FB */
}
```

**`mode_thresholds`** — `src/opus_encoder.c:180-184` — 2-D array; **declared 4, 2 top-level group(s), 4 scalar leaf/leaves in the initialiser**; `[2][2]`; identical in both preprocessor branches

```c
{
      /* voice */ /* music */
      {  64000,      10000}, /* mono */
      {  44000,      10000}, /* stereo */
}
```

**`fec_thresholds`** — `src/opus_encoder.c:186-192` — flat array; **declared ?, 10 top-level group(s), 10 scalar leaf/leaves in the initialiser**; `[]`; identical in both preprocessor branches

```c
{
        12000, 1000, /* NB */
        14000, 1000, /* MB */
        16000, 1000, /* WB */
        20000, 1000, /* SWB */
        22000, 1000, /* FB */
}
```

**`dred_bits_table`** — `src/opus_encoder.c:668-668` — flat array; **declared 16, 16 top-level group(s), 16 scalar leaf/leaves in the initialiser**; `[16]`; **`#if ENABLE_DRED`**

```c
{73.2f, 68.1f, 62.5f, 57.0f, 51.5f, 45.7f, 39.9f, 32.4f, 26.4f, 20.4f, 16.3f, 13.f, 9.3f, 8.2f, 7.2f, 6.4f}
```

**`rate_table`** — `src/opus_encoder.c:978-989` — 2-D array; **declared ?, 7 top-level group(s), 35 scalar leaf/leaves in the initialiser**; `[][5]`; identical in both preprocessor branches; function-local `static`

```c
{
  /*  |total| |-------- SILK------------|
              |-- No FEC -| |--- FEC ---|
               10ms   20ms   10ms   20ms */
      {    0,     0,     0,     0,     0},
      {12000, 10000, 10000, 11000, 11000},
      {16000, 13500, 13500, 15000, 15000},
      {20000, 16000, 16000, 18000, 18000},
      {24000, 18000, 18000, 21000, 21000},
      {32000, 22000, 22000, 28000, 28000},
      {64000, 38000, 38000, 50000, 50000}
   }
```

#### `src/opus_multistream_encoder.c`

**`vorbis_mappings`** — `src/opus_multistream_encoder.c:53-62` — array of aggregates (nested initialiser); **declared 8, 8 top-level group(s), 52 scalar leaf/leaves in the initialiser**; `[8]`; identical in both preprocessor branches

```c
{
      {1, 0, {0}},                      /* 1: mono */
      {1, 1, {0, 1}},                   /* 2: stereo */
      {2, 1, {0, 2, 1}},                /* 3: 1-d surround */
      {2, 2, {0, 1, 2, 3}},             /* 4: quadraphonic surround */
      {3, 2, {0, 4, 1, 2, 3}},          /* 5: 5-channel surround */
      {4, 2, {0, 4, 1, 2, 3, 5}},       /* 6: 5.1 surround */
      {4, 3, {0, 4, 1, 2, 3, 5, 6}},    /* 7: 6.1 surround */
      {5, 3, {0, 6, 1, 2, 3, 4, 5, 7}}, /* 8: 7.1 surround */
}
```

**`diff_table`** — `src/opus_multistream_encoder.c:198-202` — flat array; **declared 17, 9 top-level group(s), 9 scalar leaf/leaves in the initialiser**; `[17]`; function-local `static`

```c
{
         GCONST(0.5000000f), GCONST(0.2924813f), GCONST(0.1609640f), GCONST(0.0849625f),
         GCONST(0.0437314f), GCONST(0.0221971f), GCONST(0.0111839f), GCONST(0.0056136f),
         GCONST(0.0028123f)
   }
```

**`pos`** — `src/opus_multistream_encoder.c:237-237` — flat array; **declared 8, 1 top-level group(s), 1 scalar leaf/leaves in the initialiser**; `[8]`; identical in both preprocessor branches; function-local `static`

```c
{0}
```

#### `src/qext_compare.c`

**`format_size`** — `src/qext_compare.c:65-65` — flat array; **declared 3, 3 top-level group(s), 3 scalar leaf/leaves in the initialiser**; `[3]`; identical in both preprocessor branches

```c
{2, 3, 4}
```

**`p`** — `src/qext_compare.c:159-159` — flat array; **declared 2, 1 top-level group(s), 1 scalar leaf/leaves in the initialiser**; `[2]`; identical in both preprocessor branches; function-local `static`

```c
{0}
```

**`BANDS`** — `src/qext_compare.c:184-186` — flat array; **declared ?, 29 top-level group(s), 29 scalar leaf/leaves in the initialiser**; `[NBANDS+1]`; identical in both preprocessor branches

```c
{
  0,2,4,6,8,10,12,14,16,20,24,28,32,40,48,56,68,80,96,120,156,200, 240,280,320,360,400,440,480
}
```

**`maxE`** — `src/qext_compare.c:370-370` — flat array; **declared 2, 1 top-level group(s), 1 scalar leaf/leaves in the initialiser**; `[2]`; identical in both preprocessor branches; function-local `static`

```c
{0}
```

### 6.5 `extern` table declarations

These are the `tables.h`-style declarations; the definitions are the tables listed in §6.1-6.4. A Rust port needs the *definition*, so these rows exist to prove completeness of the manifest and to show which symbols are shared across translation units.

| Declaration | File:line | `#if` guard |
|---|---|---|
| `extern const signed char tf_select_table[4][8];` | `celt/celt.h:239` | — |
| `extern const opus_uint32 SMALL_DIV_TABLE[129];` | `celt/entcode.h:37` | `!defined(_entcode_H)` |
| `extern const signed char eMeans[25];` | `celt/quant_bands.h:39` | `FIXED_POINT` |
| `extern const opus_val16 eMeans[25];` | `celt/quant_bands.h:41` | `!(FIXED_POINT)` |
| `extern char silk_Timer_tags[silk_NUM_TIMERS_MAX][silk_NUM_TIMERS_MAX_TAG_LEN];` | `silk/debug.h:80` | — |
| `extern LARGE_INTEGER silk_Timer_start[silk_NUM_TIMERS_MAX];` | `silk/debug.h:82` | — |
| `extern unsigned long silk_Timer_start[silk_NUM_TIMERS_MAX];` | `silk/debug.h:84` | — |
| `extern unsigned int silk_Timer_cnt[silk_NUM_TIMERS_MAX];` | `silk/debug.h:86` | — |
| `extern opus_int64 silk_Timer_sum[silk_NUM_TIMERS_MAX];` | `silk/debug.h:87` | — |
| `extern opus_int64 silk_Timer_max[silk_NUM_TIMERS_MAX];` | `silk/debug.h:88` | — |
| `extern opus_int64 silk_Timer_min[silk_NUM_TIMERS_MAX];` | `silk/debug.h:89` | — |
| `extern opus_int64 silk_Timer_depth[silk_NUM_TIMERS_MAX];` | `silk/debug.h:90` | — |
| `extern FILE *silk_debug_store_fp[ silk_NUM_STORES_MAX ];` | `silk/debug.h:228` | — |
| `extern const opus_int8 silk_CB_lags_stage2[ PE_MAX_NB_SUBFR ][ PE_NB_CBKS_STAGE2_EXT ];` | `silk/pitch_est_defines.h:77` | — |
| `extern const opus_int8 silk_CB_lags_stage3[ PE_MAX_NB_SUBFR ][ PE_NB_CBKS_STAGE3_MAX ];` | `silk/pitch_est_defines.h:78` | — |
| `extern const opus_int8 silk_Lag_range_stage3[ SILK_PE_MAX_COMPLEX + 1 ] [ PE_MAX_NB_SUBFR ][ 2 ];` | `silk/pitch_est_defines.h:79` | — |
| `extern const opus_int8 silk_nb_cbk_searchs_stage3[ SILK_PE_MAX_COMPLEX + 1 ];` | `silk/pitch_est_defines.h:80` | — |
| `extern const opus_int8 silk_CB_lags_stage2_10_ms[ PE_MAX_NB_SUBFR >> 1][ 3 ];` | `silk/pitch_est_defines.h:83` | — |
| `extern const opus_int8 silk_CB_lags_stage3_10_ms[ PE_MAX_NB_SUBFR >> 1 ][ 12 ];` | `silk/pitch_est_defines.h:84` | — |
| `extern const opus_int8 silk_Lag_range_stage3_10_ms[ PE_MAX_NB_SUBFR >> 1 ][ 2 ];` | `silk/pitch_est_defines.h:85` | — |
| `extern const opus_int16 silk_Resampler_3_4_COEFS[ 2 + 3 * RESAMPLER_DOWN_ORDER_FIR0 / 2 ];` | `silk/resampler_rom.h:49` | — |
| `extern const opus_int16 silk_Resampler_2_3_COEFS[ 2 + 2 * RESAMPLER_DOWN_ORDER_FIR0 / 2 ];` | `silk/resampler_rom.h:50` | — |
| `extern const opus_int16 silk_Resampler_1_2_COEFS[ 2 + RESAMPLER_DOWN_ORDER_FIR1 / 2 ];` | `silk/resampler_rom.h:51` | — |
| `extern const opus_int16 silk_Resampler_1_3_COEFS[ 2 + RESAMPLER_DOWN_ORDER_FIR2 / 2 ];` | `silk/resampler_rom.h:52` | — |
| `extern const opus_int16 silk_Resampler_1_4_COEFS[ 2 + RESAMPLER_DOWN_ORDER_FIR2 / 2 ];` | `silk/resampler_rom.h:53` | — |
| `extern const opus_int16 silk_Resampler_1_6_COEFS[ 2 + RESAMPLER_DOWN_ORDER_FIR2 / 2 ];` | `silk/resampler_rom.h:54` | — |
| `extern const opus_int16 silk_Resampler_2_3_COEFS_LQ[ 2 + 2 * 2 ];` | `silk/resampler_rom.h:55` | — |
| `extern const opus_int16 silk_resampler_frac_FIR_12[ 12 ][ RESAMPLER_ORDER_FIR_12 / 2 ];` | `silk/resampler_rom.h:58` | — |
| `extern const opus_uint8 silk_gain_iCDF[ 3 ][ N_LEVELS_QGAIN / 8 ];` | `silk/tables.h:36` | — |
| `extern const opus_uint8 silk_delta_gain_iCDF[ MAX_DELTA_GAIN_QUANT - MIN_DELTA_GAIN_QUANT + 1 ];` | `silk/tables.h:37` | — |
| `extern const opus_uint8 silk_pitch_lag_iCDF[ 2 * ( PITCH_EST_MAX_LAG_MS - PITCH_EST_MIN_LAG_MS ) ];` | `silk/tables.h:39` | — |
| `extern const opus_uint8 silk_pitch_delta_iCDF[ 21 ];` | `silk/tables.h:40` | — |
| `extern const opus_uint8 silk_pitch_contour_iCDF[ 34 ];` | `silk/tables.h:41` | — |
| `extern const opus_uint8 silk_pitch_contour_NB_iCDF[ 11 ];` | `silk/tables.h:42` | — |
| `extern const opus_uint8 silk_pitch_contour_10_ms_iCDF[ 12 ];` | `silk/tables.h:43` | — |
| `extern const opus_uint8 silk_pitch_contour_10_ms_NB_iCDF[ 3 ];` | `silk/tables.h:44` | — |
| `extern const opus_uint8 silk_pulses_per_block_iCDF[ N_RATE_LEVELS ][ SILK_MAX_PULSES + 2 ];` | `silk/tables.h:46` | — |
| `extern const opus_uint8 silk_pulses_per_block_BITS_Q5[ N_RATE_LEVELS - 1 ][ SILK_MAX_PULSES + 2 ];` | `silk/tables.h:47` | — |
| `extern const opus_uint8 silk_rate_levels_iCDF[ 2 ][ N_RATE_LEVELS - 1 ];` | `silk/tables.h:49` | — |
| `extern const opus_uint8 silk_rate_levels_BITS_Q5[ 2 ][ N_RATE_LEVELS - 1 ];` | `silk/tables.h:50` | — |
| `extern const opus_uint8 silk_max_pulses_table[ 4 ];` | `silk/tables.h:52` | — |
| `extern const opus_uint8 silk_shell_code_table0[ 152 ];` | `silk/tables.h:54` | — |
| `extern const opus_uint8 silk_shell_code_table1[ 152 ];` | `silk/tables.h:55` | — |
| `extern const opus_uint8 silk_shell_code_table2[ 152 ];` | `silk/tables.h:56` | — |
| `extern const opus_uint8 silk_shell_code_table3[ 152 ];` | `silk/tables.h:57` | — |
| `extern const opus_uint8 silk_shell_code_table_offsets[ SILK_MAX_PULSES + 1 ];` | `silk/tables.h:58` | — |
| `extern const opus_uint8 silk_lsb_iCDF[ 2 ];` | `silk/tables.h:60` | — |
| `extern const opus_uint8 silk_sign_iCDF[ 42 ];` | `silk/tables.h:62` | — |
| `extern const opus_uint8 silk_uniform3_iCDF[ 3 ];` | `silk/tables.h:64` | — |
| `extern const opus_uint8 silk_uniform4_iCDF[ 4 ];` | `silk/tables.h:65` | — |
| `extern const opus_uint8 silk_uniform5_iCDF[ 5 ];` | `silk/tables.h:66` | — |
| `extern const opus_uint8 silk_uniform6_iCDF[ 6 ];` | `silk/tables.h:67` | — |
| `extern const opus_uint8 silk_uniform8_iCDF[ 8 ];` | `silk/tables.h:68` | — |
| `extern const opus_uint8 silk_NLSF_EXT_iCDF[ 7 ];` | `silk/tables.h:70` | — |
| `extern const opus_uint8 silk_LTP_per_index_iCDF[ 3 ];` | `silk/tables.h:72` | — |
| `extern const opus_uint8 * const silk_LTP_gain_iCDF_ptrs[ NB_LTP_CBKS ];` | `silk/tables.h:73` | — |
| `extern const opus_uint8 * const silk_LTP_gain_BITS_Q5_ptrs[ NB_LTP_CBKS ];` | `silk/tables.h:74` | — |
| `extern const opus_int8 * const silk_LTP_vq_ptrs_Q7[ NB_LTP_CBKS ];` | `silk/tables.h:75` | — |
| `extern const opus_uint8 * const silk_LTP_vq_gain_ptrs_Q7[NB_LTP_CBKS];` | `silk/tables.h:76` | — |
| `extern const opus_int8 silk_LTP_vq_sizes[ NB_LTP_CBKS ];` | `silk/tables.h:77` | — |
| `extern const opus_uint8 silk_LTPscale_iCDF[ 3 ];` | `silk/tables.h:79` | — |
| `extern const opus_int16 silk_LTPScales_table_Q14[ 3 ];` | `silk/tables.h:80` | — |
| `extern const opus_uint8 silk_type_offset_VAD_iCDF[ 4 ];` | `silk/tables.h:82` | — |
| `extern const opus_uint8 silk_type_offset_no_VAD_iCDF[ 2 ];` | `silk/tables.h:83` | — |
| `extern const opus_int16 silk_stereo_pred_quant_Q13[ STEREO_QUANT_TAB_SIZE ];` | `silk/tables.h:85` | — |
| `extern const opus_uint8 silk_stereo_pred_joint_iCDF[ 25 ];` | `silk/tables.h:86` | — |
| `extern const opus_uint8 silk_stereo_only_code_mid_iCDF[ 2 ];` | `silk/tables.h:87` | — |
| `extern const opus_uint8 * const silk_LBRR_flags_iCDF_ptr[ 2 ];` | `silk/tables.h:89` | — |
| `extern const opus_uint8 silk_NLSF_interpolation_factor_iCDF[ 5 ];` | `silk/tables.h:91` | — |
| `extern const opus_int16 silk_Quantization_Offsets_Q10[ 2 ][ 2 ];` | `silk/tables.h:97` | — |
| `extern const opus_int32 silk_Transition_LP_B_Q28[ TRANSITION_INT_NUM ][ TRANSITION_NB ];` | `silk/tables.h:100` | — |
| `extern const opus_int32 silk_Transition_LP_A_Q28[ TRANSITION_INT_NUM ][ TRANSITION_NA ];` | `silk/tables.h:101` | — |
| `extern const opus_int16 silk_LSFCosTab_FIX_Q12[ LSF_COS_TAB_SZ_FIX + 1 ];` | `silk/tables.h:104` | — |
| `extern const opus_int16 mapping_matrix_foa_mixing_data[36];` | `src/mapping_matrix.h:128` | — |
| `extern const opus_int16 mapping_matrix_soa_mixing_data[121];` | `src/mapping_matrix.h:131` | — |
| `extern const opus_int16 mapping_matrix_toa_mixing_data[324];` | `src/mapping_matrix.h:134` | — |
| `extern const opus_int16 mapping_matrix_fourthoa_mixing_data[729];` | `src/mapping_matrix.h:137` | — |
| `extern const opus_int16 mapping_matrix_fifthoa_mixing_data[1444];` | `src/mapping_matrix.h:140` | — |
| `extern const opus_int16 mapping_matrix_foa_demixing_data[36];` | `src/mapping_matrix.h:143` | — |
| `extern const opus_int16 mapping_matrix_soa_demixing_data[121];` | `src/mapping_matrix.h:146` | — |
| `extern const opus_int16 mapping_matrix_toa_demixing_data[324];` | `src/mapping_matrix.h:149` | — |
| `extern const opus_int16 mapping_matrix_fourthoa_demixing_data[729];` | `src/mapping_matrix.h:152` | — |
| `extern const opus_int16 mapping_matrix_fifthoa_demixing_data[1444];` | `src/mapping_matrix.h:155` | — |

### 6.6 The generated mode tables

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

**Declared size vs. initialiser count.** 9 tables declare a size that matches neither the group count nor the leaf count. In each case the C compiler zero-fills the remainder, and the Rust port must reproduce that:

| Symbol | File:line | Declared | Groups | Leaves | Note |
|---|---|---:|---:|---:|---|
| `before` | `celt/celt_encoder.c:1419` | 2 | 1 | 1 | **padded with implicit zeros** |
| `SMALL_DIV_TABLE` | `celt/entcode.c:119` | 129 | 128 | 128 | **index 0 is deliberately uninitialised and never read** (`celt_udiv` uses indices 1..128 only, `celt/entcode.h:126-140`). Reproduce as 129 entries with `[0] = 0`. |
| `MS_pred_Q13` | `silk/dec_API.c:154` | 2 | 1 | 1 | **padded with implicit zeros** |
| `p` | `src/opus_compare.c:122` | 2 | 1 | 1 | **padded with implicit zeros** |
| `mapping` | `src/opus_demo.c:328` | 256 | 2 | 2 | **padded with implicit zeros** |
| `diff_table` | `src/opus_multistream_encoder.c:198` | 17 | 9 | 9 | **padded with implicit zeros** |
| `pos` | `src/opus_multistream_encoder.c:237` | 8 | 1 | 1 | **padded with implicit zeros** |
| `p` | `src/qext_compare.c:159` | 2 | 1 | 1 | **padded with implicit zeros** |
| `maxE` | `src/qext_compare.c:370` | 2 | 1 | 1 | **padded with implicit zeros** |

**`#if` guards that change tables** (72 of 306 tables sit inside at least one conditional):

| Guard | Tables it gates | Consequence for a Rust port |
|---|---|---|
| `FIXED_POINT` | 6: `C`, `beta_coef`, `eMeans`, `log2_x_norm_coeff`, `log2_y_norm_coeff`, `pred_coef` | selects `static_modes_fixed.h` vs `static_modes_float.h` — **different element types and values for the same symbols** — and the fixed branch of `celt/mathops.h`. |
| `ENABLE_QEXT` | 24: `cache_bits50`, `cache_caps50`, `cache_index50`, `fft_bitrev120`, `fft_bitrev240`, `fft_bitrev480`, `fft_bitrev960`, `fft_twiddles96000_1920`, `last_cap`, `last_other` … | adds the `qext_*` pulse caches, `mdct_twiddles1920`, `fft_twiddles96000_1920`, `fft_bitrev960`, `qext_eBands_180/240`, `qext_logN_180/240`, a second `static_mode_list`, and a second variant of `fft_twiddles48000_960`/`window120`/`window240`/`mdct_twiddles960`. It also changes `celt_coef` (opus_val16 → opus_val32), changing the element type of every `celt_coef` table. |
| `CUSTOM_MODES` | 5: `bark_freq`, `fromOpusTable`, `maxK`, `maxN`, `toOpusTable` | adds `toOpusTable[20]` / `fromOpusTable[16]` (`celt/celt.h:199-215`), the non-Opus ↔ Opus TOC-byte mapping. |
| `ENABLE_OPUS_CUSTOM_API` | 2: `fromOpusTable`, `toOpusTable` | same table set as `CUSTOM_MODES` (`celt/celt.h:199-215`). |
| `CWRS_EXTRA_ROWS` | 2: `CELT_PVQ_U_DATA`, `CELT_PVQ_U_ROW` | changes `CELT_PVQ_U_DATA` from `[1272]` to `[1488]` (`celt/cwrs.c:215-217`). **Not set by any build system** — an embedder macro; the default is the `#else` branch, `[1272]`. |
| `SMALL_FOOTPRINT` | 2: `CELT_PVQ_U_DATA`, `CELT_PVQ_U_ROW` | removes the tabulated `V(N,K)` fast path in `celt/cwrs.c` and shrinks `silk/PLC.c` windows. **Not set by configure/CMake/meson**; default off. |
| `USE_SMALL_DIV_TABLE` | 1: `SMALL_DIV_TABLE` | `OPUS_ARM_ASM` defines it (`celt/entcode.h:39-41`), switching `celt_udiv` to `SMALL_DIV_TABLE[129]`. The table is always compiled; only its *use* is conditional. |
| `DISABLE_FLOAT_API` | 4: `analysis_window`, `dct_table`, `std_feature_bias`, `tbands` | removes `src/analysis.c`, `src/mlp.c`, `src/mlp_data.c` from the build, so every `src/mlp_data.c` table disappears. No core table changes. |
| `FLOAT_APPROX` | 2: `log2_x_norm_coeff`, `log2_y_norm_coeff` | selects inline approximations in `celt/mathops.h`; no table changes. |
| `ENABLE_DEEP_PLC` | 1: `sinc_filter` | — |
| `ENABLE_DRED` | 1: `dred_bits_table` | — |
| `ENABLE_OSCE_TRAINING_DATA` | 1: `bitrates` | — |

**Duplicate symbol names across files.** `logN400`, `cache_index50`, `cache_bits50`, `cache_caps50`, `qext_cache_*`, `fft_bitrev*`, `window120/240`, `mdct_twiddles*`, `fft_twiddles*` each appear in **both** generated mode headers and sometimes twice within one header (different sampling rate). They are `static` (file-local) and the header is included once, so exactly one instance exists per build. In Rust this becomes one module per mode, or a mode-parameterised table set.

**Ordering matters.** `fft_bitrev*` and `fft_twiddles*` are indexed by the FFT stage order produced by `kf_factor()`; do not re-sort or re-derive them from a generic FFT library.

## 7. Bit-exactness hazards

Golden rule: **target the fixed-point build** (`FIXED_POINT`, and decide `ENABLE_RES24` explicitly). Every hazard below is stated against that configuration; the float build is noted only where it leaks into the fixed build or where the port may need it.

### 7.1 The two arithmetic domains: `opus_val16` / `opus_val32`

`celt/arch.h` defines **two entirely different type systems**, selected by
`FIXED_POINT`. Every other hazard in this section is downstream of this choice.

| | fixed build (`FIXED_POINT`) | float build |
|---|---|---|
| `opus_val16` | `opus_int16` (**i16**) | `float` |
| `opus_val32` | `opus_int32` (**i32**) | `float` |
| `opus_val64` | `opus_int64` (**i64**) | `float` |
| `celt_sig / celt_norm / celt_ener / celt_glog` | `opus_val32` = i32 | `float` |
| `celt_coef` | `opus_val16` (i16), or `opus_val32` (i32) under `ENABLE_QEXT` | `float` |
| `opus_res` | `opus_val32` (i32) under `ENABLE_RES24`, else `opus_val16` (i16) | `float` |
| `Q15ONE` | `32767` | `1.0f` |
| `SIG_SHIFT` | `12` | *(not defined; `RES2SIG`/`SIG2RES` are float multiplies)* |
| `SATURATE`, `SHR`, `MULT16_16`, ... | real saturating/rounding macros | identity macros (`(x)`), except `ADD32`/`SUB32`/`MULT16_16`/`MAC16_16`/`DIV32_16`/`DIV32` which are plain FP arithmetic |

**A bit-exact port must target the `FIXED_POINT` build.** The float build is not
reproducible even between two C compilers (x87 excess precision, FMA
contraction, `-ffast-math`, and libm `cos`/`sqrt`/`log` differences), and
libopus itself documents `FLOAT_APPROX` as "not supported on all platforms".
Note in particular that in the float build **`SHR32(x,3)` is the identity** — the
Q-format shifts disappear — so the *same* source line means something completely
different in the two builds. Do not port "the algorithm"; port one build.


`celt/arch.h:130-256`

```c
#ifdef FIXED_POINT
#define ARG_FIXED(arg) , arg
#else
#define ARG_FIXED(arg)
#endif

#define PRINT_MIPS(file)

#ifdef FIXED_POINT

typedef opus_int16 opus_val16;
typedef opus_int32 opus_val32;
typedef opus_int64 opus_val64;

typedef opus_val32 celt_sig;
typedef opus_val32 celt_norm;
typedef opus_val32 celt_ener;
typedef opus_val32 celt_glog;

#ifdef ENABLE_RES24
typedef opus_val32 opus_res;
#define RES_SHIFT 8
#define SIG2RES(a)      PSHR32(a, SIG_SHIFT-RES_SHIFT)
#define RES2INT16(a)    SAT16(PSHR32(a, RES_SHIFT))
#define RES2INT24(a)    (a)
#define RES2FLOAT(a)    ((1.f/32768.f/256.f)*(a))
#define INT16TORES(a)   SHL32(EXTEND32(a), RES_SHIFT)
#define INT24TORES(a)   (a)
#define ADD_RES(a, b)   ADD32(a, b)
#define FLOAT2RES(a)    FLOAT2INT24(a)
#define RES2SIG(a)      SHL32((a), SIG_SHIFT-RES_SHIFT)
#define MULT16_RES_Q15(a,b) MULT16_32_Q15(a,b)
#define MAX_ENCODING_DEPTH 24
#else
typedef opus_val16 opus_res;
#define RES_SHIFT 0
#define SIG2RES(a)      SIG2WORD16(a)
#define RES2INT16(a)    (a)
#define RES2INT24(a)    SHL32(EXTEND32(a), 8)
#define RES2FLOAT(a)    ((1.f/32768.f)*(a))
#define INT16TORES(a)   (a)
#define INT24TORES(a)   SAT16(PSHR32(a, 8))
#define ADD_RES(a, b)   SAT16(ADD32((a), (b)));
#define FLOAT2RES(a)    FLOAT2INT16(a)
#define RES2SIG(a)      SHL32(EXTEND32(a), SIG_SHIFT)
#define MULT16_RES_Q15(a,b) MULT16_16_Q15(a,b)
#define MAX_ENCODING_DEPTH 16
#endif

#define RES2VAL16(a)    RES2INT16(a)
#define INT16TOSIG(a)   SHL32(EXTEND32(a), SIG_SHIFT)
#define INT24TOSIG(a)   SHL32(a, SIG_SHIFT-8)

#define NORM_SHIFT 24
#ifdef ENABLE_QEXT
typedef opus_val32 celt_coef;
#define COEF_ONE Q31ONE
#define MULT_COEF_32(a, b) MULT32_32_P31(a,b)
#define MAC_COEF_32_ARM(c, a, b) ADD32((c), MULT32_32_Q32(a,b))
#define MULT_COEF(a, b) MULT32_32_Q31(a,b)
#define MULT_COEF_TAPS(a, b) SHL32(MULT16_16(a,b), 1)
#define COEF2VAL16(x) EXTRACT16(SHR32(x, 16))
#else
typedef opus_val16 celt_coef;
#define COEF_ONE Q15ONE
#define MULT_COEF_32(a, b) MULT16_32_Q15(a,b)
#define MAC_COEF_32_ARM(a, b, c) MAC16_32_Q16(a,b,c)
#define MULT_COEF(a, b) MULT16_16_Q15(a,b)
#define MULT_COEF_TAPS(a, b) MULT16_16_P15(a,b)
#define COEF2VAL16(x) (x)
#endif

#define celt_isnan(x) 0

#define Q15ONE 32767
#define Q31ONE 2147483647

#define SIG_SHIFT 12
/* Safe saturation value for 32-bit signals. We need to make sure that we can
   add two sig values and that the first stages of the MDCT don't cause an overflow.
   The most constraining is the ARM_ASM comb filter where we shift left by one
   and then add two values. Because of that, we use 2^29-1. SIG_SAT must be large
   enough to fit a full-scale high-freq tone through the prefilter and comb filter,
   meaning 1.85*1.75*2^(15+SIG_SHIFT) =  434529895.
   so the limit should be about 2^31*sqrt(.5). */
#define SIG_SAT (536870911)

#define NORM_SCALING (1<<NORM_SHIFT)

#define DB_SHIFT 24

#define EPSILON 1
#define VERY_SMALL 0
#define VERY_LARGE16 ((opus_val16)32767)
#define Q15_ONE ((opus_val16)32767)


#define ABS16(x) ((x) < 0 ? (-(x)) : (x))
#define ABS32(x) ((x) < 0 ? (-(x)) : (x))

static OPUS_INLINE opus_int16 SAT16(opus_int32 x) {
   return x > 32767 ? 32767 : x < -32768 ? -32768 : (opus_int16)x;
}

#ifdef FIXED_DEBUG
#include "fixed_debug.h"
#else

#include "fixed_generic.h"

#ifdef OPUS_ARM_PRESUME_AARCH64_NEON_INTR
#include "arm/fixed_arm64.h"
#elif defined (OPUS_ARM_INLINE_EDSP)
#include "arm/fixed_armv5e.h"
#elif defined (OPUS_ARM_INLINE_ASM)
#include "arm/fixed_armv4.h"
#elif defined (BFIN_ASM)
#include "fixed_bfin.h"
#elif defined (TI_C5X_ASM)
#include "fixed_c5x.h"
#elif defined (TI_C6X_ASM)
#include "fixed_c6x.h"
#endif

#endif

#else /* FIXED_POINT */
```


`celt/arch.h:256-302`

```c
#else /* FIXED_POINT */

typedef float opus_val16;
typedef float opus_val32;
typedef float opus_val64;

typedef float celt_sig;
typedef float celt_norm;
typedef float celt_ener;
typedef float celt_glog;

typedef float opus_res;
typedef float celt_coef;

#ifdef FLOAT_APPROX
/* This code should reliably detect NaN/inf even when -ffast-math is used.
   Assumes IEEE 754 format. */
static OPUS_INLINE int celt_isnan(float x)
{
   union {float f; opus_uint32 i;} in;
   in.f = x;
   return ((in.i>>23)&0xFF)==0xFF && (in.i&0x007FFFFF)!=0;
}
#else
#ifdef __FAST_MATH__
#error Cannot build libopus with -ffast-math unless FLOAT_APPROX is defined. This could result in crashes on extreme (e.g. NaN) input
#endif
#define celt_isnan(x) ((x)!=(x))
#endif

#define Q15ONE 1.0f
#define Q31ONE 1.0f
#define COEF_ONE 1.0f
#define COEF2VAL16(x) (x)

#define NORM_SCALING 1.f

#define EPSILON 1e-15f
#define VERY_SMALL 1e-30f
#define VERY_LARGE16 1e15f
#define Q15_ONE ((opus_val16)1.f)

/* This appears to be the same speed as C99's fabsf() but it's more portable. */
#define ABS16(x) ((float)fabs(x))
#define ABS32(x) ((float)fabs(x))

#define QCONST16(x,bits) (x)
```


`celt/arch.h:302-391`

```c
#define QCONST16(x,bits) (x)
#define QCONST32(x,bits) (x)
#define GCONST(x) (x)

#define NEG16(x) (-(x))
#define NEG32(x) (-(x))
#define NEG32_ovflw(x) (-(x))
#define EXTRACT16(x) (x)
#define EXTEND32(x) (x)
#define SHR16(a,shift) (a)
#define SHL16(a,shift) (a)
#define SHR32(a,shift) (a)
#define SHL32(a,shift) (a)
#define PSHR32(a,shift) (a)
#define VSHR32(a,shift) (a)

#define SHR64(a,shift) (a)

#define PSHR(a,shift)   (a)
#define SHR(a,shift)    (a)
#define SHL(a,shift)    (a)
#define SATURATE(x,a)   (x)
#define SATURATE16(x)   (x)

#define ROUND16(a,shift)  (a)
#define SROUND16(a,shift) (a)
#define HALF16(x)       (.5f*(x))
#define HALF32(x)       (.5f*(x))

#define ADD16(a,b) ((a)+(b))
#define SUB16(a,b) ((a)-(b))
#define ADD32(a,b) ((a)+(b))
#define SUB32(a,b) ((a)-(b))
#define ADD32_ovflw(a,b) ((a)+(b))
#define SUB32_ovflw(a,b) ((a)-(b))
#define SHL32_ovflw(a,shift) (a)
#define PSHR32_ovflw(a,shift) (a)

#define MULT16_16_16(a,b)     ((a)*(b))
#define MULT16_16(a,b)     ((opus_val32)(a)*(opus_val32)(b))
#define MAC16_16(c,a,b)     ((c)+(opus_val32)(a)*(opus_val32)(b))

#define MULT16_32_Q15(a,b)     ((a)*(b))
#define MULT16_32_Q16(a,b)     ((a)*(b))

#define MULT32_32_Q16(a,b)     ((a)*(b))
#define MULT32_32_Q31(a,b)     ((a)*(b))
#define MULT32_32_P31(a,b)     ((a)*(b))
#define MULT32_32_P31_ovflw(a,b) ((a)*(b))

#define MAC16_32_Q15(c,a,b)     ((c)+(a)*(b))
#define MAC16_32_Q16(c,a,b)     ((c)+(a)*(b))
#define MAC_COEF_32_ARM(c,a,b)     ((c)+(a)*(b))

#define MULT16_16_Q11_32(a,b)     ((a)*(b))
#define MULT16_16_Q11(a,b)     ((a)*(b))
#define MULT16_16_Q13(a,b)     ((a)*(b))
#define MULT16_16_Q14(a,b)     ((a)*(b))
#define MULT16_16_Q15(a,b)     ((a)*(b))
#define MULT16_16_P15(a,b)     ((a)*(b))
#define MULT16_16_P13(a,b)     ((a)*(b))
#define MULT16_16_P14(a,b)     ((a)*(b))
#define MULT16_32_P16(a,b)     ((a)*(b))

#define MULT_COEF_32(a, b)      ((a)*(b))
#define MULT_COEF(a, b)   ((a)*(b))
#define MULT_COEF_TAPS(a, b)   ((a)*(b))

#define DIV32_16(a,b)     (((opus_val32)(a))/(opus_val16)(b))
#define DIV32(a,b)     (((opus_val32)(a))/(opus_val32)(b))

#define SIG2RES(a)      ((1/CELT_SIG_SCALE)*(a))
#define RES2INT16(a)    FLOAT2INT16(a)
#define RES2INT24(a)    float2int(32768.f*256.f*(a))
#define RES2FLOAT(a)    (a)
#define INT16TORES(a)   ((a)*(1/CELT_SIG_SCALE))
#define INT24TORES(a)   ((1.f/32768.f/256.f)*(a))
#define ADD_RES(a, b)   ADD32(a, b)
#define FLOAT2RES(a)    (a)
#define RES2SIG(a)      (CELT_SIG_SCALE*(a))
#define MULT16_RES_Q15(a,b) MULT16_16_Q15(a,b)

#define RES2VAL16(a)    (a)
#define FLOAT2SIG(a)    ((a)*CELT_SIG_SCALE)
#define INT16TOSIG(a)   ((float)(a))
#define INT24TOSIG(a)   ((float)(a)*(1.f/256.f))
#define MAX_ENCODING_DEPTH 24

#endif /* !FIXED_POINT */

```


### 7.2 The CELT fixed-point macro set — the single most important file to port

`celt/fixed_generic.h` is included by `celt/arch.h:234-256` whenever
`FIXED_POINT` is defined and `FIXED_DEBUG` is not. **Every bit-exactness
property of CELT lives in these macros.** Port them one-for-one, with the same
argument-evaluation count, the same intermediate widths, and the same rounding
mode, and unit-test each against a compiled C reference over the full edge set.

Key facts to preserve:

* `SHR(a,shift)` on a **negative** `a`: C leaves `>>` on signed negative
  values *implementation-defined*, and libopus relies on the universal
  arithmetic-shift behaviour. Rust's `>>` on `i32` **is** arithmetic and
  well-defined, so `i32 >> n` is the correct translation — but never compute
  `SHR` through a `u32` cast, and never use `>>=` on a value whose type the port
  may have widened.
* `PSHR32(a,shift)` rounds: `(a + (1<<(shift-1))) >> shift` — note it adds
  *before* shifting, so it can overflow for `a` near `INT32_MAX`. Rust must
  use wrapping add, exactly like C's two's-complement wrap, and must not use
  `checked_add`.
* `MULT16_16(a,b)` widens both `i16` to `i32` **before** multiplying
  (`(opus_val32)(a)*(opus_val32)(b)`). A naive Rust `(a as i32) * (b as i32)`
  matches; `i16 * i16` would overflow.
* `MULT16_32_Q15(a,b)` = `SHR32(MULT16_16(a, SHR(b,15)) + ...)` — see the exact
  text; it is *not* `(a*b)>>15`. The `+16384` rounding term and the ordering of
  the two partial products determine the low bit.
* `SATURATE16`/`SATURATE(x,a)` clamp to `[-a, a-1]`/`[-(a), a-1]`, and
  `SATURATE(x, 2147483647)` is the 32-bit no-op case.
* Under `FIXED_DEBUG`, `celt/fixed_debug.h` replaces every one of these with an
  overflow-checking version. That header is an **oracle only**; do not port it as
  the shipping semantics.


`celt/fixed_generic.h:30-218`

```c
   SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
*/

#ifndef FIXED_GENERIC_H
#define FIXED_GENERIC_H

/** Multiply a 16-bit signed value by a 16-bit unsigned value. The result is a 32-bit signed value */
#define MULT16_16SU(a,b) ((opus_val32)(opus_val16)(a)*(opus_val32)(opus_uint16)(b))

/** 16x32 multiplication, followed by a 16-bit shift right. Results fits in 32 bits */
#if OPUS_FAST_INT64
#define MULT16_32_Q16(a,b) ((opus_val32)SHR((opus_int64)((opus_val16)(a))*(b),16))
#else
#define MULT16_32_Q16(a,b) ADD32(MULT16_16((a),SHR((b),16)), SHR(MULT16_16SU((a),((b)&0x0000ffff)),16))
#endif

/** 16x32 multiplication, followed by a 16-bit shift right (round-to-nearest). Results fits in 32 bits */
#if OPUS_FAST_INT64
#define MULT16_32_P16(a,b) ((opus_val32)PSHR((opus_int64)((opus_val16)(a))*(b),16))
#else
#define MULT16_32_P16(a,b) ADD32(MULT16_16((a),SHR((b),16)), PSHR(MULT16_16SU((a),((b)&0x0000ffff)),16))
#endif

/** 16x32 multiplication, followed by a 15-bit shift right. Results fits in 32 bits */
#if OPUS_FAST_INT64
#define MULT16_32_Q15(a,b) ((opus_val32)SHR((opus_int64)((opus_val16)(a))*(b),15))
#else
#define MULT16_32_Q15(a,b) ADD32(SHL(MULT16_16((a),SHR((b),16)),1), SHR(MULT16_16SU((a),((b)&0x0000ffff)),15))
#endif

/** 32x32 multiplication, followed by a 16-bit shift right. Results fits in 32 bits */
#if OPUS_FAST_INT64
#define MULT32_32_Q16(a,b) ((opus_val32)SHR((opus_int64)(a)*(opus_int64)(b),16))
#else
#define MULT32_32_Q16(a,b) (ADD32(ADD32(ADD32((opus_val32)(SHR32(((opus_uint32)((a)&0x0000ffff)*(opus_uint32)((b)&0x0000ffff)),16)), MULT16_16SU(SHR32(a,16),((b)&0x0000ffff))), MULT16_16SU(SHR32(b,16),((a)&0x0000ffff))), SHL32(MULT16_16(SHR32(a,16),SHR32(b,16)),16)))
#endif

/** 32x32 multiplication, followed by a 31-bit shift right. Results fits in 32 bits */
#if OPUS_FAST_INT64
#define MULT32_32_Q31(a,b) ((opus_val32)SHR((opus_int64)(a)*(opus_int64)(b),31))
#else
#define MULT32_32_Q31(a,b) ADD32(ADD32(SHL(MULT16_16(SHR((a),16),SHR((b),16)),1), SHR(MULT16_16SU(SHR((a),16),((b)&0x0000ffff)),15)), SHR(MULT16_16SU(SHR((b),16),((a)&0x0000ffff)),15))
#endif

/** 32x32 multiplication, followed by a 31-bit shift right (with rounding). Results fits in 32 bits */
#if OPUS_FAST_INT64
#define MULT32_32_P31(a,b) ((opus_val32)SHR(1073741824+(opus_int64)(a)*(opus_int64)(b),31))
#define MULT32_32_P31_ovflw(a,b) MULT32_32_P31(a,b)
#else
#define MULT16_16U(a,b) ((opus_uint32)(a)*(opus_uint32)(b))
#define MULT32_32_P31(a,b) ADD32(SHL(MULT16_16(SHR((a),16),SHR((b),16)),1), SHR32(128+(opus_int32)SHR(MULT16_16U(((a)&0x0000ffff),((b)&0x0000ffff)),16+7) + SHR32(MULT16_16SU(SHR((a),16),((b)&0x0000ffff)),7) + SHR32(MULT16_16SU(SHR((b),16),((a)&0x0000ffff)),7), 8) )
#define MULT32_32_P31_ovflw(a,b) ADD32_ovflw(SHL(MULT16_16(SHR((a),16),SHR((b),16)),1), SHR32(128+(opus_int32)SHR(MULT16_16U(((a)&0x0000ffff),((b)&0x0000ffff)),16+7) + SHR32(MULT16_16SU(SHR((a),16),((b)&0x0000ffff)),7) + SHR32(MULT16_16SU(SHR((b),16),((a)&0x0000ffff)),7), 8) )
#endif

/** 32x32 multiplication, followed by a 32-bit shift right. Results fits in 32 bits */
#if OPUS_FAST_INT64
#define MULT32_32_Q32(a,b) ((opus_val32)SHR((opus_int64)(a)*(opus_int64)(b),32))
#else
#define MULT32_32_Q32(a,b) ADD32(ADD32(MULT16_16(SHR((a),16),SHR((b),16)), SHR(MULT16_16SU(SHR((a),16),((b)&0x0000ffff)),16)), SHR(MULT16_16SU(SHR((b),16),((a)&0x0000ffff)),16))
#endif

/** Compile-time conversion of float constant to 16-bit value */
#define QCONST16(x,bits) ((opus_val16)(.5+(x)*(((opus_val32)1)<<(bits))))

/** Compile-time conversion of float constant to 32-bit value */
#define QCONST32(x,bits) ((opus_val32)(.5+(x)*(((opus_int64)1)<<(bits))))

/** Compile-time conversion of float constant to log gain value */
#define GCONST2(x,bits) ((celt_glog)(.5+(x)*(((celt_glog)1)<<(bits))))

/** Compile-time conversion of float constant to DB_SHIFT log gain value */
#define GCONST(x) GCONST2((x),DB_SHIFT)

/** Negate a 16-bit value */
#define NEG16(x) (-(x))
/** Negate a 32-bit value */
#define NEG32(x) (-(x))

/** Change a 32-bit value into a 16-bit value. The value is assumed to fit in 16-bit, otherwise the result is undefined */
#define EXTRACT16(x) ((opus_val16)(x))
/** Change a 16-bit value into a 32-bit value */
#define EXTEND32(x) ((opus_val32)(x))

/** Arithmetic shift-right of a 16-bit value */
#define SHR16(a,shift) ((a) >> (shift))
/** Arithmetic shift-left of a 16-bit value */
#define SHL16(a,shift) ((opus_int16)((opus_uint16)(a)<<(shift)))
/** Arithmetic shift-right of a 32-bit value */
#define SHR32(a,shift) ((a) >> (shift))
/** Arithmetic shift-left of a 32-bit value */
#define SHL32(a,shift) ((opus_int32)((opus_uint32)(a)<<(shift)))

/** 32-bit arithmetic shift right with rounding-to-nearest instead of rounding down */
#define PSHR32(a,shift) (SHR32((a)+((EXTEND32(1)<<((shift))>>1)),shift))
/** 32-bit arithmetic shift right where the argument can be negative */
#define VSHR32(a, shift) (((shift)>0) ? SHR32(a, shift) : SHL32(a, -(shift)))

/** Arithmetic shift-right of a 64-bit value */
#define SHR64(a,shift) ((a) >> (shift))

/** "RAW" macros, should not be used outside of this header file */
#define SHR(a,shift) ((a) >> (shift))
#define SHL(a,shift) SHL32(a,shift)
#define PSHR(a,shift) (SHR((a)+((EXTEND32(1)<<((shift))>>1)),shift))
#define SATURATE(x,a) (((x)>(a) ? (a) : (x)<-(a) ? -(a) : (x)))

#define SATURATE16(x) (EXTRACT16((x)>32767 ? 32767 : (x)<-32768 ? -32768 : (x)))

/** Shift by a and round-to-nearest 32-bit value. Result is a 16-bit value */
#define ROUND16(x,a) (EXTRACT16(PSHR32((x),(a))))
/** Shift by a and round-to-nearest 32-bit value. Result is a saturated 16-bit value */
#define SROUND16(x,a) EXTRACT16(SATURATE(PSHR32(x,a), 32767));

/** Divide by two */
#define HALF16(x)  (SHR16(x,1))
#define HALF32(x)  (SHR32(x,1))

/** Add two 16-bit values */
#define ADD16(a,b) ((opus_val16)((opus_val16)(a)+(opus_val16)(b)))
/** Subtract two 16-bit values */
#define SUB16(a,b) ((opus_val16)(a)-(opus_val16)(b))
/** Add two 32-bit values */
#define ADD32(a,b) ((opus_val32)(a)+(opus_val32)(b))
/** Subtract two 32-bit values */
#define SUB32(a,b) ((opus_val32)(a)-(opus_val32)(b))

/** Add two 32-bit values, ignore any overflows */
#define ADD32_ovflw(a,b) ((opus_val32)((opus_uint32)(a)+(opus_uint32)(b)))
/** Subtract two 32-bit values, ignore any overflows */
#define SUB32_ovflw(a,b) ((opus_val32)((opus_uint32)(a)-(opus_uint32)(b)))
/* Avoid MSVC warning C4146: unary minus operator applied to unsigned type */
/** Negate 32-bit value, ignore any overflows */
#define NEG32_ovflw(a) ((opus_val32)(0-(opus_uint32)(a)))
/** 32-bit shift left, ignoring overflows */
#define SHL32_ovflw(a,shift) SHL32(a,shift)
/** 32-bit arithmetic shift right with rounding-to-nearest, ignoring overflows */
#define PSHR32_ovflw(a,shift) (SHR32(ADD32_ovflw(a, (EXTEND32(1)<<(shift)>>1)),shift))

/** 16x16 multiplication where the result fits in 16 bits */
#define MULT16_16_16(a,b)     ((((opus_val16)(a))*((opus_val16)(b))))

/** 32x32 multiplication where the result fits in 32 bits */
#define MULT32_32_32(a,b)     ((((opus_val32)(a))*((opus_val32)(b))))

/* (opus_val32)(opus_val16) gives TI compiler a hint that it's 16x16->32 multiply */
/** 16x16 multiplication where the result fits in 32 bits */
#define MULT16_16(a,b)     (((opus_val32)(opus_val16)(a))*((opus_val32)(opus_val16)(b)))

/** 16x16 multiply-add where the result fits in 32 bits */
#define MAC16_16(c,a,b) (ADD32((c),MULT16_16((a),(b))))
/** 16x32 multiply, followed by a 15-bit shift right and 32-bit add.
    b must fit in 31 bits.
    Result fits in 32 bits. */
#define MAC16_32_Q15(c,a,b) ADD32((c),ADD32(MULT16_16((a),SHR((b),15)), SHR(MULT16_16((a),((b)&0x00007fff)),15)))

/** 16x32 multiplication, followed by a 16-bit shift right and 32-bit add.
    Results fits in 32 bits */
#define MAC16_32_Q16(c,a,b) ADD32((c),ADD32(MULT16_16((a),SHR((b),16)), SHR(MULT16_16SU((a),((b)&0x0000ffff)),16)))

#define MULT16_16_Q11_32(a,b) (SHR(MULT16_16((a),(b)),11))
#define MULT16_16_Q11(a,b) (SHR(MULT16_16((a),(b)),11))
#define MULT16_16_Q13(a,b) (SHR(MULT16_16((a),(b)),13))
#define MULT16_16_Q14(a,b) (SHR(MULT16_16((a),(b)),14))
#define MULT16_16_Q15(a,b) (SHR(MULT16_16((a),(b)),15))

#define MULT16_16_P13(a,b) (SHR(ADD32(4096,MULT16_16((a),(b))),13))
#define MULT16_16_P14(a,b) (SHR(ADD32(8192,MULT16_16((a),(b))),14))
#define MULT16_16_P15(a,b) (SHR(ADD32(16384,MULT16_16((a),(b))),15))

/** Divide a 32-bit value by a 16-bit value. Result fits in 16 bits */
#define DIV32_16(a,b) ((opus_val16)(((opus_val32)(a))/((opus_val16)(b))))

/** Divide a 32-bit value by a 32-bit value. Result fits in 32 bits */
#define DIV32(a,b) (((opus_val32)(a))/((opus_val32)(b)))

#if defined(__mips)
#include "mips/fixed_generic_mipsr1.h"
#endif

static OPUS_INLINE opus_val16 SIG2WORD16_generic(celt_sig x)
{
   x = PSHR32(x, SIG_SHIFT);
   x = MAX32(x, -32768);
   x = MIN32(x, 32767);
   return EXTRACT16(x);
}
#define SIG2WORD16(x) (SIG2WORD16_generic(x))

#endif
```


`celt/arch.h:96-111`

```c
#endif

#define IMUL32(a,b) ((a)*(b))

#define MIN16(a,b) ((a) < (b) ? (a) : (b))   /**< Minimum 16-bit value.   */
#define MAX16(a,b) ((a) > (b) ? (a) : (b))   /**< Maximum 16-bit value.   */
#define MIN32(a,b) ((a) < (b) ? (a) : (b))   /**< Minimum 32-bit value.   */
#define MAX32(a,b) ((a) > (b) ? (a) : (b))   /**< Maximum 32-bit value.   */
#define IMIN(a,b) ((a) < (b) ? (a) : (b))   /**< Minimum int value.   */
#define IMAX(a,b) ((a) > (b) ? (a) : (b))   /**< Maximum int value.   */
#define FMIN(a,b) ((a) < (b) ? (a) : (b))   /**< Minimum float value.   */
#define FMAX(a,b) ((a) > (b) ? (a) : (b))   /**< Maximum float value.   */
#define UADD32(a,b) ((a)+(b))
#define USUB32(a,b) ((a)-(b))
#define MAXG(a,b) MAX32(a, b)
#define MING(a,b) MIN32(a, b)
```


`celt/arch.h:219-245`

```c
#define DB_SHIFT 24

#define EPSILON 1
#define VERY_SMALL 0
#define VERY_LARGE16 ((opus_val16)32767)
#define Q15_ONE ((opus_val16)32767)


#define ABS16(x) ((x) < 0 ? (-(x)) : (x))
#define ABS32(x) ((x) < 0 ? (-(x)) : (x))

static OPUS_INLINE opus_int16 SAT16(opus_int32 x) {
   return x > 32767 ? 32767 : x < -32768 ? -32768 : (opus_int16)x;
}

#ifdef FIXED_DEBUG
#include "fixed_debug.h"
#else

#include "fixed_generic.h"

#ifdef OPUS_ARM_PRESUME_AARCH64_NEON_INTR
#include "arm/fixed_arm64.h"
#elif defined (OPUS_ARM_INLINE_EDSP)
#include "arm/fixed_armv5e.h"
#elif defined (OPUS_ARM_INLINE_ASM)
#include "arm/fixed_armv4.h"
```


### 7.3 The SILK fixed-point macro set

`silk/macros.h` and `silk/Inlines.h` play the same role for SILK. They are
included via `silk/SigProc_FIX.h`:

```c
#include "typedef.h"
#include "resampler_structs.h"
#include "macros.h"
#include "cpu_support.h"
```

Hazards specific to this set:

* `silk_SMULWB(a32, b16)` = `SHR32(MULT16_16((a32)&0xFFFF, b16), 16) +
  SHR32(MULT16_16(SHR32(a32,16), b16), 16)` — a **two-part multiply** whose
  partial-sum ordering matters. Do not simplify to `(a32 as i64 * b16) >> 16`.
* `silk_SMLAWB`, `silk_SMULWW`, `silk_SMLAWW`, `silk_SMMUL` likewise.
* `silk_RSHIFT_ROUND(a, shift)` = `shift==1 ? (a>>1)+(a&1) : ((a>>(shift-1))+1)>>1`.
  The `shift==1` special case exists because `shift-1 == 0` would otherwise be a
  zero shift. Reproduce the branch.
* `silk_ADD_SAT32` / `silk_SUB_SAT32` saturate at `silk_int32_MAX`/`_MIN`.
* `silk_CLZ32` uses `__builtin_clz` under GCC and a hand-rolled loop otherwise,
  **and `silk_CLZ32(0)` returns 32** in the loop version. Rust
  `leading_zeros()` agrees, but check the `silk_CLZ_FRAC` contract:
  `silk_CLZ_FRAC(in, &lz, &frac)` is only defined for `in != 0`.
* Under `FIXED_DEBUG` (`silk/MacroDebug.h`) every macro becomes an
  overflow-checking version; again, an oracle, not the shipping semantics.
* `silk/macros.h` has `#define silk_MUL(a32, b32) ((a32) * (b32))` and
  `silk_MLA(a32,b32,c32)` — plain 32-bit multiply with **silent wrap** on
  overflow.


`silk/macros.h:30-154`

```c

#ifdef HAVE_CONFIG_H
#include "config.h"
#endif

#include "opus_types.h"
#include "opus_defines.h"
#include "arch.h"

/* This is an OPUS_INLINE header file for general platform. */

/* (a32 * (opus_int32)((opus_int16)(b32))) >> 16 output have to be 32bit int */
#if OPUS_FAST_INT64
#define silk_SMULWB(a32, b32)            ((opus_int32)(((a32) * (opus_int64)((opus_int16)(b32))) >> 16))
#else
#define silk_SMULWB(a32, b32)            ((((a32) >> 16) * (opus_int32)((opus_int16)(b32))) + ((((a32) & 0x0000FFFF) * (opus_int32)((opus_int16)(b32))) >> 16))
#endif

/* a32 + (b32 * (opus_int32)((opus_int16)(c32))) >> 16 output have to be 32bit int */
#if OPUS_FAST_INT64
#define silk_SMLAWB(a32, b32, c32)       ((opus_int32)((a32) + (((b32) * (opus_int64)((opus_int16)(c32))) >> 16)))
#else
#define silk_SMLAWB(a32, b32, c32)       ((a32) + ((((b32) >> 16) * (opus_int32)((opus_int16)(c32))) + ((((b32) & 0x0000FFFF) * (opus_int32)((opus_int16)(c32))) >> 16)))
#endif

/* (a32 * (b32 >> 16)) >> 16 */
#if OPUS_FAST_INT64
#define silk_SMULWT(a32, b32)            ((opus_int32)(((a32) * (opus_int64)((b32) >> 16)) >> 16))
#else
#define silk_SMULWT(a32, b32)            (((a32) >> 16) * ((b32) >> 16) + ((((a32) & 0x0000FFFF) * ((b32) >> 16)) >> 16))
#endif

/* a32 + (b32 * (c32 >> 16)) >> 16 */
#if OPUS_FAST_INT64
#define silk_SMLAWT(a32, b32, c32)       ((opus_int32)((a32) + (((b32) * ((opus_int64)(c32) >> 16)) >> 16)))
#else
#define silk_SMLAWT(a32, b32, c32)       ((a32) + (((b32) >> 16) * ((c32) >> 16)) + ((((b32) & 0x0000FFFF) * ((c32) >> 16)) >> 16))
#endif

/* (opus_int32)((opus_int16)(a3))) * (opus_int32)((opus_int16)(b32)) output have to be 32bit int */
#define silk_SMULBB(a32, b32)            ((opus_int32)((opus_int16)(a32)) * (opus_int32)((opus_int16)(b32)))

/* a32 + (opus_int32)((opus_int16)(b32)) * (opus_int32)((opus_int16)(c32)) output have to be 32bit int */
#define silk_SMLABB(a32, b32, c32)       ((a32) + ((opus_int32)((opus_int16)(b32))) * (opus_int32)((opus_int16)(c32)))

/* (opus_int32)((opus_int16)(a32)) * (b32 >> 16) */
#define silk_SMULBT(a32, b32)            ((opus_int32)((opus_int16)(a32)) * ((b32) >> 16))

/* a32 + (opus_int32)((opus_int16)(b32)) * (c32 >> 16) */
#define silk_SMLABT(a32, b32, c32)       ((a32) + ((opus_int32)((opus_int16)(b32))) * ((c32) >> 16))

/* a64 + (b32 * c32) */
#define silk_SMLAL(a64, b32, c32)        (silk_ADD64((a64), ((opus_int64)(b32) * (opus_int64)(c32))))

/* (a32 * b32) >> 16 */
#if OPUS_FAST_INT64
#define silk_SMULWW(a32, b32)            ((opus_int32)(((opus_int64)(a32) * (b32)) >> 16))
#else
#define silk_SMULWW(a32, b32)            silk_MLA(silk_SMULWB((a32), (b32)), (a32), silk_RSHIFT_ROUND((b32), 16))
#endif

/* a32 + ((b32 * c32) >> 16) */
#if OPUS_FAST_INT64
#define silk_SMLAWW(a32, b32, c32)       ((opus_int32)((a32) + (((opus_int64)(b32) * (c32)) >> 16)))
#else
#define silk_SMLAWW(a32, b32, c32)       silk_MLA(silk_SMLAWB((a32), (b32), (c32)), (b32), silk_RSHIFT_ROUND((c32), 16))
#endif

/* add/subtract with output saturated */
#define silk_ADD_SAT32(a, b)             ((((opus_uint32)(a) + (opus_uint32)(b)) & 0x80000000) == 0 ?                              \
                                        ((((a) & (b)) & 0x80000000) != 0 ? silk_int32_MIN : (a)+(b)) :   \
                                        ((((a) | (b)) & 0x80000000) == 0 ? silk_int32_MAX : (a)+(b)) )

#define silk_SUB_SAT32(a, b)             ((((opus_uint32)(a)-(opus_uint32)(b)) & 0x80000000) == 0 ?                                        \
                                        (( (a) & ((b)^0x80000000) & 0x80000000) ? silk_int32_MIN : (a)-(b)) :    \
                                        ((((a)^0x80000000) & (b)  & 0x80000000) ? silk_int32_MAX : (a)-(b)) )

#if defined(FIXED_POINT) && defined(__mips)
#include "mips/macros_mipsr1.h"
#endif

#include "ecintrin.h"
#ifndef OVERRIDE_silk_CLZ16
static OPUS_INLINE opus_int32 silk_CLZ16(opus_int16 in16)
{
    return 32 - EC_ILOG(in16<<16|0x8000);
}
#endif

#ifndef OVERRIDE_silk_CLZ32
static OPUS_INLINE opus_int32 silk_CLZ32(opus_int32 in32)
{
    return in32 ? 32 - EC_ILOG(in32) : 32;
}
#endif

/* Row based */
#define matrix_ptr(Matrix_base_adr, row, column, N) \
    (*((Matrix_base_adr) + ((row)*(N)+(column))))
#define matrix_adr(Matrix_base_adr, row, column, N) \
      ((Matrix_base_adr) + ((row)*(N)+(column)))

/* Column based */
#ifndef matrix_c_ptr
#   define matrix_c_ptr(Matrix_base_adr, row, column, M) \
    (*((Matrix_base_adr) + ((row)+(M)*(column))))
#endif

#ifdef OPUS_ARM_INLINE_ASM
#include "arm/macros_armv4.h"
#endif

#ifdef OPUS_ARM_INLINE_EDSP
#include "arm/macros_armv5e.h"
#endif

#ifdef OPUS_ARM_PRESUME_AARCH64_NEON_INTR
#include "arm/macros_arm64.h"
#endif

#ifdef OPUS_XTENSA_LX7
#include "xtensa/macros_lx7.h"
#endif

#endif /* SILK_MACROS_H */
```


`silk/Inlines.h:30-180`

```c
 */

#ifndef SILK_FIX_INLINES_H
#define SILK_FIX_INLINES_H


/* count leading zeros of opus_int64 */
static OPUS_INLINE opus_int32 silk_CLZ64( opus_int64 in )
{
    opus_int32 in_upper;

    in_upper = (opus_int32)silk_RSHIFT64(in, 32);
    if (in_upper == 0) {
        /* Search in the lower 32 bits */
        return 32 + silk_CLZ32( (opus_int32) in );
    } else {
        /* Search in the upper 32 bits */
        return silk_CLZ32( in_upper );
    }
}

/* get number of leading zeros and fractional part (the bits right after the leading one */
static OPUS_INLINE void silk_CLZ_FRAC(
    opus_int32 in,            /* I  input                               */
    opus_int32 *lz,           /* O  number of leading zeros             */
    opus_int32 *frac_Q7       /* O  the 7 bits right after the leading one */
)
{
    opus_int32 lzeros = silk_CLZ32(in);

    * lz = lzeros;
    * frac_Q7 = silk_ROR32(in, 24 - lzeros) & 0x7f;
}

/* Approximation of square root                                          */
/* Accuracy: < +/- 10%  for output values > 15                           */
/*           < +/- 2.5% for output values > 120                          */
static OPUS_INLINE opus_int32 silk_SQRT_APPROX( opus_int32 x )
{
    opus_int32 y, lz, frac_Q7;

    if( x <= 0 ) {
        return 0;
    }

    silk_CLZ_FRAC(x, &lz, &frac_Q7);

    if( lz & 1 ) {
        y = 32768;
    } else {
        y = 46214;        /* 46214 = sqrt(2) * 32768 */
    }

    /* get scaling right */
    y >>= silk_RSHIFT(lz, 1);

    /* increment using fractional part of input */
    y = silk_SMLAWB(y, y, silk_SMULBB(213, frac_Q7));

    return y;
}

/* Divide two int32 values and return result as int32 in a given Q-domain */
static OPUS_INLINE opus_int32 silk_DIV32_varQ(   /* O    returns a good approximation of "(a32 << Qres) / b32" */
    const opus_int32     a32,               /* I    numerator (Q0)                  */
    const opus_int32     b32,               /* I    denominator (Q0)                */
    const opus_int       Qres               /* I    Q-domain of result (>= 0)       */
)
{
    opus_int   a_headrm, b_headrm, lshift;
    opus_int32 b32_inv, a32_nrm, b32_nrm, result;

    silk_assert( b32 != 0 );
    silk_assert( Qres >= 0 );

    /* Compute number of bits head room and normalize inputs */
    a_headrm = silk_CLZ32( silk_abs(a32) ) - 1;
    a32_nrm = silk_LSHIFT(a32, a_headrm);                                       /* Q: a_headrm                  */
    b_headrm = silk_CLZ32( silk_abs(b32) ) - 1;
    b32_nrm = silk_LSHIFT(b32, b_headrm);                                       /* Q: b_headrm                  */

    /* Inverse of b32, with 14 bits of precision */
    b32_inv = silk_DIV32_16( silk_int32_MAX >> 2, silk_RSHIFT(b32_nrm, 16) );   /* Q: 29 + 16 - b_headrm        */

    /* First approximation */
    result = silk_SMULWB(a32_nrm, b32_inv);                                     /* Q: 29 + a_headrm - b_headrm  */

    /* Compute residual by subtracting product of denominator and first approximation */
    /* It's OK to overflow because the final value of a32_nrm should always be small */
    a32_nrm = silk_SUB32_ovflw(a32_nrm, silk_LSHIFT_ovflw( silk_SMMUL(b32_nrm, result), 3 ));  /* Q: a_headrm   */

    /* Refinement */
    result = silk_SMLAWB(result, a32_nrm, b32_inv);                             /* Q: 29 + a_headrm - b_headrm  */

    /* Convert to Qres domain */
    lshift = 29 + a_headrm - b_headrm - Qres;
    if( lshift < 0 ) {
        return silk_LSHIFT_SAT32(result, -lshift);
    } else {
        if( lshift < 32){
            return silk_RSHIFT(result, lshift);
        } else {
            /* Avoid undefined result */
            return 0;
        }
    }
}

/* Invert int32 value and return result as int32 in a given Q-domain */
static OPUS_INLINE opus_int32 silk_INVERSE32_varQ(   /* O    returns a good approximation of "(1 << Qres) / b32" */
    const opus_int32     b32,                   /* I    denominator (Q0)                */
    const opus_int       Qres                   /* I    Q-domain of result (> 0)        */
)
{
    opus_int   b_headrm, lshift;
    opus_int32 b32_inv, b32_nrm, err_Q32, result;

    silk_assert( b32 != 0 );
    silk_assert( Qres > 0 );

    /* Compute number of bits head room and normalize input */
    b_headrm = silk_CLZ32( silk_abs(b32) ) - 1;
    b32_nrm = silk_LSHIFT(b32, b_headrm);                                       /* Q: b_headrm                */

    /* Inverse of b32, with 14 bits of precision */
    b32_inv = silk_DIV32_16( silk_int32_MAX >> 2, silk_RSHIFT(b32_nrm, 16) );   /* Q: 29 + 16 - b_headrm    */

    /* First approximation */
    result = silk_LSHIFT(b32_inv, 16);                                          /* Q: 61 - b_headrm            */

    /* Compute residual by subtracting product of denominator and first approximation from one */
    err_Q32 = silk_LSHIFT( ((opus_int32)1<<29) - silk_SMULWB(b32_nrm, b32_inv), 3 );        /* Q32                        */

    /* Refinement */
    result = silk_SMLAWW(result, err_Q32, b32_inv);                             /* Q: 61 - b_headrm            */

    /* Convert to Qres domain */
    lshift = 61 - b_headrm - Qres;
    if( lshift <= 0 ) {
        return silk_LSHIFT_SAT32(result, -lshift);
    } else {
        if( lshift < 32){
            return silk_RSHIFT(result, lshift);
        }else{
            /* Avoid undefined result */
            return 0;
        }
    }
}

#endif /* SILK_FIX_INLINES_H */
```


### 7.4 The range coder — where a single off-by-one destroys everything

The entropy coder is the first thing to port and the thing to differential-test
hardest. The relevant files are `celt/mfrngcod.h` (the constants and the
arithmetic-coder equations), `celt/ecintrin.h` (`EC_CLZ`/`EC_ILOG`),
`celt/entcode.[ch]` (state, `ec_ilog`, `ec_tell_frac`, `celt_udiv`),
`celt/entenc.c` and `celt/entdec.c`.

Specific hazards:

1. **`ec_ilog` must be the C one, not `32 - leading_zeros`.** The two agree, but
   `EC_ILOG(0)` is *undefined* in the macro version (`__builtin_clz(0)`); the
   fallback function version returns 0 for 0. Reproduce `ec_ilog`'s exact
   behaviour (`celt/entcode.c:41-67`) and never call it with 0 from Rust.
2. **`ec_tell_frac`'s loop** (`celt/entcode.c:69-112`) runs a fixed number of
   iterations (`BITRES` refinement steps) and accumulates
   `r=...; t=...; while(...){...}`. The iteration count is part of the output.
3. **`ec_enc_carry_out` and the `rem`/`ext` carry chain** affect *every emitted
   byte*. `ext` counts outstanding carry-propagating symbols; `rem == -1` means
   "no buffered byte".
4. **`ec_enc_normalize`'s `while(_this->rng <= EC_CODE_BOT)` loop** must run the
   identical number of iterations. `rng` is `opus_uint32`, so the shifts must be
   on unsigned types (`<<` on u32 wraps; on i32 it would be UB).
5. **`ec_dec_uint`'s `_ft` optimisation**: if `_ft > (1u<<EC_UINT_BITS)` the
   decoder reads `EC_UINT_BITS` bits with `ec_dec_bits` and then decodes a
   uniform symbol over the remaining range; the *order* of the two reads and the
   exact `ft`/`this` values must match.
6. **`ec_dec_bits`' `end_window`/`nend_bits` refill loop**: `while(available <=
   EC_WINDOW_SIZE - EC_SYM_BITS)` — the `<=` vs `<` and the
   `EC_WINDOW_SIZE - EC_SYM_BITS` bound determine how many bytes are consumed.
7. **`ec_enc_done`'s `l`/`msk`/`end` computation** (`celt/entenc.c:255-300`):
   `l = EC_CODE_BITS - EC_ILOG(_this->rng)`, `msk = (EC_CODE_TOP-1)>>l`, and the
   loop that flushes the carry. The final `end` value decides the number of
   bytes and the content of the tail.
8. **`ec_decode`/`ec_dec_update`/`ec_encode` arithmetic** is in `celt/mfrngcod.h`
   as macros and is the normative coder equation.
9. **Buffer-overflow semantics are part of the output.** The encoder's
   `ec_write_byte` / `ec_write_byte_at_end` **refuse** a write and return `-1`
   once `offs+end_offs >= storage`; the callers fold that into `error`
   (`_this->error|=ec_write_byte_at_end(...)`), so the byte stream is silently
   *truncated* and `ec_enc_done` may set `error=-1` again. The decoder's
   `ec_dec_read_byte` returns `0` for reads past `storage`. The API contract is
   `nbCompressedBytes >= 2` (`celt/celt_encoder.c:1831`) and
   `max_data_bytes >= 3` (`src/opus_encoder.c:1340`). A Rust port that panics or
   returns an error instead of truncating-and-flagging will diverge on the
   smallest packets.
10. `celt_udiv` has a `celt_sig_assert(d>0)` and is documented "Tested
    exhaustively for all n and for 1<=d<=256"; the `USE_SMALL_DIV_TABLE` branch
    computes `q+(n-q*d >= d)`, which is **not** the same as `n/d` for all inputs
    if the table lookup is off by one. Port both branches and test them against
    each other.


`celt/mfrngcod.h:30-48`

```c
# include "entcode.h"

/*Constants used by the entropy encoder/decoder.*/

/*The number of bits to output at a time.*/
# define EC_SYM_BITS   (8)
/*The total number of bits in each of the state registers.*/
# define EC_CODE_BITS  (32)
/*The maximum symbol value.*/
# define EC_SYM_MAX    ((1U<<EC_SYM_BITS)-1)
/*Bits to shift by to move a symbol into the high-order position.*/
# define EC_CODE_SHIFT (EC_CODE_BITS-EC_SYM_BITS-1)
/*Carry bit of the high-order range symbol.*/
# define EC_CODE_TOP   (((opus_uint32)1U)<<(EC_CODE_BITS-1))
/*Low-order bit of the high-order range symbol.*/
# define EC_CODE_BOT   (EC_CODE_TOP>>EC_SYM_BITS)
/*The number of bits available for the last, partial symbol in the code field.*/
# define EC_CODE_EXTRA ((EC_CODE_BITS-2)%EC_SYM_BITS+1)
#endif
```


`celt/ecintrin.h:30-91`

```c
#include <math.h>
#include <limits.h>
#include "arch.h"
#if !defined(_ecintrin_H)
# define _ecintrin_H (1)

/*Some specific platforms may have optimized intrinsic or OPUS_INLINE assembly
   versions of these functions which can substantially improve performance.
  We define macros for them to allow easy incorporation of these non-ANSI
   features.*/

/*Modern gcc (4.x) can compile the naive versions of min and max with cmov if
   given an appropriate architecture, but the branchless bit-twiddling versions
   are just as fast, and do not require any special target architecture.
  Earlier gcc versions (3.x) compiled both code to the same assembly
   instructions, because of the way they represented ((_b)>(_a)) internally.*/
# define EC_MINI(_a,_b)      ((_a)+(((_b)-(_a))&-((_b)<(_a))))

/*Count leading zeros.
  This macro should only be used for implementing ec_ilog(), if it is defined.
  All other code should use EC_ILOG() instead.*/
#if defined(_MSC_VER) && (_MSC_VER >= 1400)
#if defined(_MSC_VER) && (_MSC_VER >= 1910)
# include <intrin0.h> /* Improve compiler throughput. */
#else
# include <intrin.h>
#endif
/*In _DEBUG mode this is not an intrinsic by default.*/
# pragma intrinsic(_BitScanReverse)

static __inline int ec_bsr(unsigned long _x){
  unsigned long ret;
  _BitScanReverse(&ret,_x);
  return (int)ret;
}
# define EC_CLZ0    (1)
# define EC_CLZ(_x) (-ec_bsr(_x))
#elif defined(ENABLE_TI_DSPLIB)
# include "dsplib.h"
# define EC_CLZ0    (31)
# define EC_CLZ(_x) (_lnorm(_x))
#elif __GNUC_PREREQ(3,4)
# if INT_MAX>=2147483647
#  define EC_CLZ0    ((int)sizeof(unsigned)*CHAR_BIT)
#  define EC_CLZ(_x) (__builtin_clz(_x))
# elif LONG_MAX>=2147483647L
#  define EC_CLZ0    ((int)sizeof(unsigned long)*CHAR_BIT)
#  define EC_CLZ(_x) (__builtin_clzl(_x))
# endif
#endif

#if defined(EC_CLZ)
/*Note that __builtin_clz is not defined when _x==0, according to the gcc
   documentation (and that of the BSR instruction that implements it on x86).
  The majority of the time we can never pass it zero.
  When we need to, it can be special cased.*/
# define EC_ILOG(_x) (EC_CLZ0-EC_CLZ(_x))
#else
int ec_ilog(opus_uint32 _v);
# define EC_ILOG(_x) (ec_ilog(_x))
#endif
#endif
```


`celt/entcode.h:33-61`

```c
# include <limits.h>
# include <stddef.h>
# include "ecintrin.h"

extern const opus_uint32 SMALL_DIV_TABLE[129];

#ifdef OPUS_ARM_ASM
#define USE_SMALL_DIV_TABLE
#endif

/*OPT: ec_window must be at least 32 bits, but if you have fast arithmetic on a
   larger type, you can speed up the decoder by using it here.*/
typedef opus_uint32           ec_window;
typedef struct ec_ctx         ec_ctx;
typedef struct ec_ctx         ec_enc;
typedef struct ec_ctx         ec_dec;

# define EC_WINDOW_SIZE ((int)sizeof(ec_window)*CHAR_BIT)

/*The number of bits to use for the range-coded part of unsigned integers.*/
# define EC_UINT_BITS   (8)

/*The resolution of fractional-precision bit usage measurements, i.e.,
   3 => 1/8th bits.*/
# define BITRES 3

/*The entropy encoder/decoder context.
  We use the same structure for both, so that common functions like ec_tell()
   can be used on either one.*/
```


`celt/entcode.h:105-151`

```c
/*Returns the number of bits "used" by the encoded or decoded symbols so far.
  This same number can be computed in either the encoder or the decoder, and is
   suitable for making coding decisions.
  Return: The number of bits.
          This will always be slightly larger than the exact value (e.g., all
           rounding error is in the positive direction).*/
static OPUS_INLINE int ec_tell(ec_ctx *_this){
  return _this->nbits_total-EC_ILOG(_this->rng);
}

/*Returns the number of bits "used" by the encoded or decoded symbols so far.
  This same number can be computed in either the encoder or the decoder, and is
   suitable for making coding decisions.
  Return: The number of bits scaled by 2**BITRES.
          This will always be slightly larger than the exact value (e.g., all
           rounding error is in the positive direction).*/
opus_uint32 ec_tell_frac(ec_ctx *_this);

/* Tested exhaustively for all n and for 1<=d<=256 */
static OPUS_INLINE opus_uint32 celt_udiv(opus_uint32 n, opus_uint32 d) {
   celt_sig_assert(d>0);
#ifdef USE_SMALL_DIV_TABLE
   if (d>256)
      return n/d;
   else {
      opus_uint32 t, q;
      t = EC_ILOG(d&-d);
      q = (opus_uint64)SMALL_DIV_TABLE[d>>t]*(n>>(t-1))>>32;
      return q+(n-q*d >= d);
   }
#else
   return n/d;
#endif
}

static OPUS_INLINE opus_int32 celt_sudiv(opus_int32 n, opus_int32 d) {
   celt_sig_assert(d>0);
#ifdef USE_SMALL_DIV_TABLE
   if (n<0)
      return -(opus_int32)celt_udiv(-n, d);
   else
      return celt_udiv(n, d);
#else
   return n/d;
#endif
}

```


`celt/entcode.c:30-112`

```c
#endif

#include "entcode.h"
#include "arch.h"

#if !defined(EC_CLZ)
/*This is a fallback for systems where we don't know how to access
   a BSR or CLZ instruction (see ecintrin.h).
  If you are optimizing Opus on a new platform and it has a native CLZ or
   BZR (e.g. cell, MIPS, x86, etc) then making it available to Opus will be
   an easy performance win.*/
int ec_ilog(opus_uint32 _v){
  /*On a Pentium M, this branchless version tested as the fastest on
     1,000,000,000 random 32-bit integers, edging out a similar version with
     branches, and a 256-entry LUT version.*/
  int ret;
  int m;
  ret=!!_v;
  m=!!(_v&0xFFFF0000)<<4;
  _v>>=m;
  ret|=m;
  m=!!(_v&0xFF00)<<3;
  _v>>=m;
  ret|=m;
  m=!!(_v&0xF0)<<2;
  _v>>=m;
  ret|=m;
  m=!!(_v&0xC)<<1;
  _v>>=m;
  ret|=m;
  ret+=!!(_v&0x2);
  return ret;
}
#endif

#if 1
/* This is a faster version of ec_tell_frac() that takes advantage
   of the low (1/8 bit) resolution to use just a linear function
   followed by a lookup to determine the exact transition thresholds. */
opus_uint32 ec_tell_frac(ec_ctx *_this){
  static const unsigned correction[8] =
    {35733, 38967, 42495, 46340,
     50535, 55109, 60097, 65535};
  opus_uint32 nbits;
  opus_uint32 r;
  int         l;
  unsigned    b;
  nbits=_this->nbits_total<<BITRES;
  l=EC_ILOG(_this->rng);
  r=_this->rng>>(l-16);
  b = (r>>12)-8;
  b += r>correction[b];
  l = (l<<3)+b;
  return nbits-l;
}
#else
opus_uint32 ec_tell_frac(ec_ctx *_this){
  opus_uint32 nbits;
  opus_uint32 r;
  int         l;
  int         i;
  /*To handle the non-integral number of bits still left in the encoder/decoder
     state, we compute the worst-case number of bits of val that must be
     encoded to ensure that the value is inside the range for any possible
     subsequent bits.
    The computation here is independent of val itself (the decoder does not
     even track that value), even though the real number of bits used after
     ec_enc_done() may be 1 smaller if rng is a power of two and the
     corresponding trailing bits of val are all zeros.
    If we did try to track that special case, then coding a value with a
     probability of 1/(1<<n) might sometimes appear to use more than n bits.
    This may help explain the surprising result that a newly initialized
     encoder or decoder claims to have used 1 bit.*/
  nbits=_this->nbits_total<<BITRES;
  l=EC_ILOG(_this->rng);
  r=_this->rng>>(l-16);
  for(i=BITRES;i-->0;){
    int b;
    r=r*r>>15;
    b=(int)(r>>16);
    l=l<<1|b;
    r>>=b;
  }
```


`celt/entenc.c:36-130`

```c
/*A range encoder.
  See entdec.c and the references for implementation details \cite{Mar79,MNW98}.

  @INPROCEEDINGS{Mar79,
   author="Martin, G.N.N.",
   title="Range encoding: an algorithm for removing redundancy from a digitised
    message",
   booktitle="Video \& Data Recording Conference",
   year=1979,
   address="Southampton",
   month=Jul
  }
  @ARTICLE{MNW98,
   author="Alistair Moffat and Radford Neal and Ian H. Witten",
   title="Arithmetic Coding Revisited",
   journal="{ACM} Transactions on Information Systems",
   year=1998,
   volume=16,
   number=3,
   pages="256--294",
   month=Jul,
   URL="http://www.stanford.edu/class/ee398/handouts/papers/Moffat98ArithmCoding.pdf"
  }*/

static int ec_write_byte(ec_enc *_this,unsigned _value){
  if(_this->offs+_this->end_offs>=_this->storage)return -1;
  _this->buf[_this->offs++]=(unsigned char)_value;
  return 0;
}

static int ec_write_byte_at_end(ec_enc *_this,unsigned _value){
  if(_this->offs+_this->end_offs>=_this->storage)return -1;
  _this->buf[_this->storage-++(_this->end_offs)]=(unsigned char)_value;
  return 0;
}

/*Outputs a symbol, with a carry bit.
  If there is a potential to propagate a carry over several symbols, they are
   buffered until it can be determined whether or not an actual carry will
   occur.
  If the counter for the buffered symbols overflows, then the stream becomes
   undecodable.
  This gives a theoretical limit of a few billion symbols in a single packet on
   32-bit systems.
  The alternative is to truncate the range in order to force a carry, but
   requires similar carry tracking in the decoder, needlessly slowing it down.*/
static void ec_enc_carry_out(ec_enc *_this,int _c){
  if(_c!=EC_SYM_MAX){
    /*No further carry propagation possible, flush buffer.*/
    int carry;
    carry=_c>>EC_SYM_BITS;
    /*Don't output a byte on the first write.
      This compare should be taken care of by branch-prediction thereafter.*/
    if(_this->rem>=0)_this->error|=ec_write_byte(_this,_this->rem+carry);
    if(_this->ext>0){
      unsigned sym;
      sym=(EC_SYM_MAX+carry)&EC_SYM_MAX;
      do _this->error|=ec_write_byte(_this,sym);
      while(--(_this->ext)>0);
    }
    _this->rem=_c&EC_SYM_MAX;
  }
  else _this->ext++;
}

static OPUS_INLINE void ec_enc_normalize(ec_enc *_this){
  /*If the range is too small, output some bits and rescale it.*/
  while(_this->rng<=EC_CODE_BOT){
    ec_enc_carry_out(_this,(int)(_this->val>>EC_CODE_SHIFT));
    /*Move the next-to-high-order symbol into the high-order position.*/
    _this->val=(_this->val<<EC_SYM_BITS)&(EC_CODE_TOP-1);
    _this->rng<<=EC_SYM_BITS;
    _this->nbits_total+=EC_SYM_BITS;
  }
}

void ec_enc_init(ec_enc *_this,unsigned char *_buf,opus_uint32 _size){
  _this->buf=_buf;
  _this->end_offs=0;
  _this->end_window=0;
  _this->nend_bits=0;
  /*This is the offset from which ec_tell() will subtract partial bits.*/
  _this->nbits_total=EC_CODE_BITS+1;
  _this->offs=0;
  _this->rng=EC_CODE_TOP;
  _this->rem=-1;
  _this->val=0;
  _this->ext=0;
  _this->storage=_size;
  _this->error=0;
}

void ec_encode(ec_enc *_this,unsigned _fl,unsigned _fh,unsigned _ft){
  opus_uint32 r;
  r=celt_udiv(_this->rng,_ft);
```


`celt/entenc.c:186-230`

```c
void ec_enc_uint(ec_enc *_this,opus_uint32 _fl,opus_uint32 _ft){
  unsigned  ft;
  unsigned  fl;
  int       ftb;
  /*In order to optimize EC_ILOG(), it is undefined for the value 0.*/
  celt_assert(_ft>1);
  _ft--;
  ftb=EC_ILOG(_ft);
  if(ftb>EC_UINT_BITS){
    ftb-=EC_UINT_BITS;
    ft=(_ft>>ftb)+1;
    fl=(unsigned)(_fl>>ftb);
    ec_encode(_this,fl,fl+1,ft);
    ec_enc_bits(_this,_fl&(((opus_uint32)1<<ftb)-1U),ftb);
  }
  else ec_encode(_this,_fl,_fl+1,_ft+1);
}

void ec_enc_bits(ec_enc *_this,opus_uint32 _fl,unsigned _bits){
  ec_window window;
  int       used;
  window=_this->end_window;
  used=_this->nend_bits;
  celt_assert(_bits>0);
  if(used+_bits>EC_WINDOW_SIZE){
    do{
      _this->error|=ec_write_byte_at_end(_this,(unsigned)window&EC_SYM_MAX);
      window>>=EC_SYM_BITS;
      used-=EC_SYM_BITS;
    }
    while(used>=EC_SYM_BITS);
  }
  window|=(ec_window)_fl<<used;
  used+=_bits;
  _this->end_window=window;
  _this->nend_bits=used;
  _this->nbits_total+=_bits;
}

void ec_enc_patch_initial_bits(ec_enc *_this,unsigned _val,unsigned _nbits){
  int      shift;
  unsigned mask;
  celt_assert(_nbits<=EC_SYM_BITS);
  shift=EC_SYM_BITS-_nbits;
  mask=((1<<_nbits)-1)<<shift;
```


`celt/entenc.c:255-305`

```c
void ec_enc_done(ec_enc *_this){
  ec_window   window;
  int         used;
  opus_uint32 msk;
  opus_uint32 end;
  int         l;
  /*We output the minimum number of bits that ensures that the symbols encoded
     thus far will be decoded correctly regardless of the bits that follow.*/
  l=EC_CODE_BITS-EC_ILOG(_this->rng);
  msk=(EC_CODE_TOP-1)>>l;
  end=(_this->val+msk)&~msk;
  if((end|msk)>=_this->val+_this->rng){
    l++;
    msk>>=1;
    end=(_this->val+msk)&~msk;
  }
  while(l>0){
    ec_enc_carry_out(_this,(int)(end>>EC_CODE_SHIFT));
    end=(end<<EC_SYM_BITS)&(EC_CODE_TOP-1);
    l-=EC_SYM_BITS;
  }
  /*If we have a buffered byte flush it into the output buffer.*/
  if(_this->rem>=0||_this->ext>0)ec_enc_carry_out(_this,0);
  /*If we have buffered extra bits, flush them as well.*/
  window=_this->end_window;
  used=_this->nend_bits;
  while(used>=EC_SYM_BITS){
    _this->error|=ec_write_byte_at_end(_this,(unsigned)window&EC_SYM_MAX);
    window>>=EC_SYM_BITS;
    used-=EC_SYM_BITS;
  }
  /*Clear any excess space and add any remaining extra bits to the last byte.*/
  if(!_this->error){
    if (_this->buf) OPUS_CLEAR(_this->buf+_this->offs,
     _this->storage-_this->offs-_this->end_offs);
    if(used>0){
      /*If there's no range coder data at all, give up.*/
      if(_this->end_offs>=_this->storage)_this->error=-1;
      else{
        l=-l;
        /*If we've busted, don't add too many extra bits to the last byte; it
           would corrupt the range coder data, and that's more important.*/
        if(_this->offs+_this->end_offs>=_this->storage&&l<used){
          window&=(1<<l)-1;
          _this->error=-1;
        }
        _this->buf[_this->storage-_this->end_offs-1]|=(unsigned char)window;
      }
    }
  }
}
```


`celt/entdec.c:90-266`

```c

static int ec_read_byte(ec_dec *_this){
  return _this->offs<_this->storage?_this->buf[_this->offs++]:0;
}

static int ec_read_byte_from_end(ec_dec *_this){
  return _this->end_offs<_this->storage?
   _this->buf[_this->storage-++(_this->end_offs)]:0;
}

/*Normalizes the contents of val and rng so that rng lies entirely in the
   high-order symbol.*/
static void ec_dec_normalize(ec_dec *_this){
  /*If the range is too small, rescale it and input some bits.*/
  while(_this->rng<=EC_CODE_BOT){
    int sym;
    _this->nbits_total+=EC_SYM_BITS;
    _this->rng<<=EC_SYM_BITS;
    /*Use up the remaining bits from our last symbol.*/
    sym=_this->rem;
    /*Read the next value from the input.*/
    _this->rem=ec_read_byte(_this);
    /*Take the rest of the bits we need from this new symbol.*/
    sym=(sym<<EC_SYM_BITS|_this->rem)>>(EC_SYM_BITS-EC_CODE_EXTRA);
    /*And subtract them from val, capped to be less than EC_CODE_TOP.*/
    _this->val=((_this->val<<EC_SYM_BITS)+(EC_SYM_MAX&~sym))&(EC_CODE_TOP-1);
  }
}

void ec_dec_init(ec_dec *_this,unsigned char *_buf,opus_uint32 _storage){
  _this->buf=_buf;
  _this->storage=_storage;
  _this->end_offs=0;
  _this->end_window=0;
  _this->nend_bits=0;
  /*This is the offset from which ec_tell() will subtract partial bits.
    The final value after the ec_dec_normalize() call will be the same as in
     the encoder, but we have to compensate for the bits that are added there.*/
  _this->nbits_total=EC_CODE_BITS+1
   -((EC_CODE_BITS-EC_CODE_EXTRA)/EC_SYM_BITS)*EC_SYM_BITS;
  _this->offs=0;
  _this->rng=1U<<EC_CODE_EXTRA;
  _this->rem=ec_read_byte(_this);
  _this->val=_this->rng-1-(_this->rem>>(EC_SYM_BITS-EC_CODE_EXTRA));
  _this->error=0;
  /*Normalize the interval.*/
  ec_dec_normalize(_this);
}

unsigned ec_decode(ec_dec *_this,unsigned _ft){
  unsigned s;
  _this->ext=celt_udiv(_this->rng,_ft);
  s=(unsigned)(_this->val/_this->ext);
  return _ft-EC_MINI(s+1,_ft);
}

unsigned ec_decode_bin(ec_dec *_this,unsigned _bits){
   unsigned s;
   _this->ext=_this->rng>>_bits;
   s=(unsigned)(_this->val/_this->ext);
   return (1U<<_bits)-EC_MINI(s+1U,1U<<_bits);
}

void ec_dec_update(ec_dec *_this,unsigned _fl,unsigned _fh,unsigned _ft){
  opus_uint32 s;
  s=IMUL32(_this->ext,_ft-_fh);
  _this->val-=s;
  _this->rng=_fl>0?IMUL32(_this->ext,_fh-_fl):_this->rng-s;
  ec_dec_normalize(_this);
}

/*The probability of having a "one" is 1/(1<<_logp).*/
int ec_dec_bit_logp(ec_dec *_this,unsigned _logp){
  opus_uint32 r;
  opus_uint32 d;
  opus_uint32 s;
  int         ret;
  r=_this->rng;
  d=_this->val;
  s=r>>_logp;
  ret=d<s;
  if(!ret)_this->val=d-s;
  _this->rng=ret?s:r-s;
  ec_dec_normalize(_this);
  return ret;
}

int ec_dec_icdf(ec_dec *_this,const unsigned char *_icdf,unsigned _ftb){
  opus_uint32 r;
  opus_uint32 d;
  opus_uint32 s;
  opus_uint32 t;
  int         ret;
  s=_this->rng;
  d=_this->val;
  r=s>>_ftb;
  ret=-1;
  do{
    t=s;
    s=IMUL32(r,_icdf[++ret]);
  }
  while(d<s);
  _this->val=d-s;
  _this->rng=t-s;
  ec_dec_normalize(_this);
  return ret;
}

int ec_dec_icdf16(ec_dec *_this,const opus_uint16 *_icdf,unsigned _ftb){
  opus_uint32 r;
  opus_uint32 d;
  opus_uint32 s;
  opus_uint32 t;
  int         ret;
  s=_this->rng;
  d=_this->val;
  r=s>>_ftb;
  ret=-1;
  do{
    t=s;
    s=IMUL32(r,_icdf[++ret]);
  }
  while(d<s);
  _this->val=d-s;
  _this->rng=t-s;
  ec_dec_normalize(_this);
  return ret;
}

opus_uint32 ec_dec_uint(ec_dec *_this,opus_uint32 _ft){
  unsigned ft;
  unsigned s;
  int      ftb;
  /*In order to optimize EC_ILOG(), it is undefined for the value 0.*/
  celt_assert(_ft>1);
  _ft--;
  ftb=EC_ILOG(_ft);
  if(ftb>EC_UINT_BITS){
    opus_uint32 t;
    ftb-=EC_UINT_BITS;
    ft=(unsigned)(_ft>>ftb)+1;
    s=ec_decode(_this,ft);
    ec_dec_update(_this,s,s+1,ft);
    t=(opus_uint32)s<<ftb|ec_dec_bits(_this,ftb);
    if(t<=_ft)return t;
    _this->error=1;
    return _ft;
  }
  else{
    _ft++;
    s=ec_decode(_this,(unsigned)_ft);
    ec_dec_update(_this,s,s+1,(unsigned)_ft);
    return s;
  }
}

opus_uint32 ec_dec_bits(ec_dec *_this,unsigned _bits){
  ec_window   window;
  int         available;
  opus_uint32 ret;
  window=_this->end_window;
  available=_this->nend_bits;
  if((unsigned)available<_bits){
    do{
      window|=(ec_window)ec_read_byte_from_end(_this)<<available;
      available+=EC_SYM_BITS;
    }
    while(available<=EC_WINDOW_SIZE-EC_SYM_BITS);
  }
  ret=(opus_uint32)window&(((opus_uint32)1<<_bits)-1U);
  window>>=_bits;
  available-=_bits;
  _this->end_window=window;
  _this->nend_bits=available;
  _this->nbits_total+=_bits;
  return ret;
}
```


### 7.5 Integer division and rounding

Integer division is a rich source of silent divergence because C's `/` truncates
toward zero and Rust's `/` does too — but only for the same signedness, and the
*inputs* are often the problem.

| Site | Exact behaviour to preserve |
|---|---|
| `celt_udiv(n,d)` (`celt/entcode.h:126-140`) | unsigned; `d>0` assumed. `USE_SMALL_DIV_TABLE` path: `t = EC_ILOG(d&-d)`, `q = (u64)SMALL_DIV_TABLE[d>>t]*(n>>(t-1))>>32`, `return q+(n-q*d >= d)`. The `n>>(t-1)` with `t==0` is a **shift by -1** in C (UB); it cannot happen because `d>0` implies `d&-d != 0` implies `t>=1`. Rust must still avoid the underflow. |
| `celt_sudiv(n,d)` | `n<0 ? -(i32)celt_udiv(-n,d) : celt_udiv(n,d)` — note `-n` overflows for `INT32_MIN`. |
| `silk_DIV32_16(a32, b16)` | plain C `/`; truncation toward zero. |
| `silk_DIV32(a32,b32)` | plain C `/`. |
| `silk_DIV32_varQ(a32,b32,Qres)` (`silk/Inlines.h:93-138`) | normalises both operands by `silk_CLZ32`, uses a 64-bit numerator `silk_SMULWW(n32, silk_INVERSE32_varQ(b32,Qres))` and then a **correction step** `if (silk_SMULWB(a32, n32) - (b32<<16) >= 0) n32 += 1<<...;` with a shift-saturating `if (shift<0) ... else ...`. The correction step and the `Qres` clamp are part of the result. |
| `silk_INVERSE32_varQ(b32,Qres)` (`silk/Inlines.h:139-180`) | 64-bit Newton-Raphson style reciprocal with `silk_SMLAWW`; the **iteration count and the final rounding shift** are exact requirements. |
| `silk_SQRT_APPROX(x)` (`silk/Inlines.h:67-92`) | uses `silk_CLZ_FRAC` + a 2-bit fractional part `frac_Q7 = frac >> 23` + `SHL32(1,...)` piecewise result. Only defined for `x > 0`; `x==0` returns 0 via the explicit branch. |
| `silk_lin2log(x)` / `silk_log2lin(x)` | piecewise-linear table lookups with `silk_CLZ_FRAC`; exact code in §7.6. |
| `silk_NLSF_stabilize` | bisection loop whose iteration count is bounded by the delta-min table; the loop's `while(1)` structure and the `NDeltaMin_Q15` lookups must match. |
| `silk/code_signs.c`, `silk/gain_quant.c` | divisions of possibly-negative `i32` by positive constants; C truncation must be reproduced (Rust `/` matches, `div_euclid` would not). |

**Rust rule:** use `i32::wrapping_div`-equivalent plain `/` only where the C
divides the same signednesses; where C relies on wrap-around (e.g.
`-INT32_MIN`), use `wrapping_neg`.


`silk/SigProc_FIX.h:445-530`

```c
/* Adds two signed 32-bit values in a way that can overflow, while not relying on undefined behaviour
   (just standard two's complement implementation-specific behaviour) */
#define silk_ADD32_ovflw(a, b)              ((opus_int32)((opus_uint32)(a) + (opus_uint32)(b)))
/* Subtractss two signed 32-bit values in a way that can overflow, while not relying on undefined behaviour
   (just standard two's complement implementation-specific behaviour) */
#define silk_SUB32_ovflw(a, b)              ((opus_int32)((opus_uint32)(a) - (opus_uint32)(b)))

/* Multiply-accumulate macros that allow overflow in the addition (ie, no asserts in debug mode) */
#define silk_MLA_ovflw(a32, b32, c32)       silk_ADD32_ovflw((a32), (opus_uint32)(b32) * (opus_uint32)(c32))
#define silk_SMLABB_ovflw(a32, b32, c32)    (silk_ADD32_ovflw((a32) , ((opus_int32)((opus_int16)(b32))) * (opus_int32)((opus_int16)(c32))))

#define silk_DIV32_16(a32, b16)             ((opus_int32)((a32) / (b16)))
#define silk_DIV32(a32, b32)                ((opus_int32)((a32) / (b32)))

/* These macros enables checking for overflow in silk_API_Debug.h*/
#define silk_ADD16(a, b)                    ((a) + (b))
#define silk_ADD32(a, b)                    ((a) + (b))
#define silk_ADD64(a, b)                    ((a) + (b))

#define silk_SUB16(a, b)                    ((a) - (b))
#define silk_SUB32(a, b)                    ((a) - (b))
#define silk_SUB64(a, b)                    ((a) - (b))

#define silk_SAT8(a)                        ((a) > silk_int8_MAX ? silk_int8_MAX  :       \
                                            ((a) < silk_int8_MIN ? silk_int8_MIN  : (a)))
#define silk_SAT16(a)                       ((a) > silk_int16_MAX ? silk_int16_MAX :      \
                                            ((a) < silk_int16_MIN ? silk_int16_MIN : (a)))
#define silk_SAT32(a)                       ((a) > silk_int32_MAX ? silk_int32_MAX :      \
                                            ((a) < silk_int32_MIN ? silk_int32_MIN : (a)))

#define silk_CHECK_FIT8(a)                  (a)
#define silk_CHECK_FIT16(a)                 (a)
#define silk_CHECK_FIT32(a)                 (a)

#define silk_ADD_SAT16(a, b)                (opus_int16)silk_SAT16( silk_ADD32( (opus_int32)(a), (b) ) )
#define silk_ADD_SAT64(a, b)                ((((a) + (b)) & 0x8000000000000000LL) == 0 ?                            \
                                            ((((a) & (b)) & 0x8000000000000000LL) != 0 ? silk_int64_MIN : (a)+(b)) : \
                                            ((((a) | (b)) & 0x8000000000000000LL) == 0 ? silk_int64_MAX : (a)+(b)) )

#define silk_SUB_SAT16(a, b)                (opus_int16)silk_SAT16( silk_SUB32( (opus_int32)(a), (b) ) )
#define silk_SUB_SAT64(a, b)                ((((a)-(b)) & 0x8000000000000000LL) == 0 ?                                               \
                                            (( (a) & ((b)^0x8000000000000000LL) & 0x8000000000000000LL) ? silk_int64_MIN : (a)-(b)) : \
                                            ((((a)^0x8000000000000000LL) & (b)  & 0x8000000000000000LL) ? silk_int64_MAX : (a)-(b)) )

/* Saturation for positive input values */
#define silk_POS_SAT32(a)                   ((a) > silk_int32_MAX ? silk_int32_MAX : (a))

/* Add with saturation for positive input values */
#define silk_ADD_POS_SAT8(a, b)             ((((a)+(b)) & 0x80)                 ? silk_int8_MAX  : ((a)+(b)))
#define silk_ADD_POS_SAT16(a, b)            ((((a)+(b)) & 0x8000)               ? silk_int16_MAX : ((a)+(b)))
#define silk_ADD_POS_SAT32(a, b)            ((((opus_uint32)(a)+(opus_uint32)(b)) & 0x80000000) ? silk_int32_MAX : ((a)+(b)))

#define silk_LSHIFT8(a, shift)              ((opus_int8)((opus_uint8)(a)<<(shift)))         /* shift >= 0, shift < 8  */
#define silk_LSHIFT16(a, shift)             ((opus_int16)((opus_uint16)(a)<<(shift)))       /* shift >= 0, shift < 16 */
#define silk_LSHIFT32(a, shift)             ((opus_int32)((opus_uint32)(a)<<(shift)))       /* shift >= 0, shift < 32 */
#define silk_LSHIFT64(a, shift)             ((opus_int64)((opus_uint64)(a)<<(shift)))       /* shift >= 0, shift < 64 */
#define silk_LSHIFT(a, shift)               silk_LSHIFT32(a, shift)                         /* shift >= 0, shift < 32 */

#define silk_RSHIFT8(a, shift)              ((a)>>(shift))                                  /* shift >= 0, shift < 8  */
#define silk_RSHIFT16(a, shift)             ((a)>>(shift))                                  /* shift >= 0, shift < 16 */
#define silk_RSHIFT32(a, shift)             ((a)>>(shift))                                  /* shift >= 0, shift < 32 */
#define silk_RSHIFT64(a, shift)             ((a)>>(shift))                                  /* shift >= 0, shift < 64 */
#define silk_RSHIFT(a, shift)               silk_RSHIFT32(a, shift)                         /* shift >= 0, shift < 32 */

/* saturates before shifting */
#define silk_LSHIFT_SAT32(a, shift)         (silk_LSHIFT32( silk_LIMIT( (a), silk_RSHIFT32( silk_int32_MIN, (shift) ), \
                                                    silk_RSHIFT32( silk_int32_MAX, (shift) ) ), (shift) ))

#define silk_LSHIFT_ovflw(a, shift)         ((opus_int32)((opus_uint32)(a) << (shift)))     /* shift >= 0, allowed to overflow */
#define silk_LSHIFT_uint(a, shift)          ((a) << (shift))                                /* shift >= 0 */
#define silk_RSHIFT_uint(a, shift)          ((a) >> (shift))                                /* shift >= 0 */

#define silk_ADD_LSHIFT(a, b, shift)        ((a) + silk_LSHIFT((b), (shift)))               /* shift >= 0 */
#define silk_ADD_LSHIFT32(a, b, shift)      silk_ADD32((a), silk_LSHIFT32((b), (shift)))    /* shift >= 0 */
#define silk_ADD_LSHIFT_uint(a, b, shift)   ((a) + silk_LSHIFT_uint((b), (shift)))          /* shift >= 0 */
#define silk_ADD_RSHIFT(a, b, shift)        ((a) + silk_RSHIFT((b), (shift)))               /* shift >= 0 */
#define silk_ADD_RSHIFT32(a, b, shift)      silk_ADD32((a), silk_RSHIFT32((b), (shift)))    /* shift >= 0 */
#define silk_ADD_RSHIFT_uint(a, b, shift)   ((a) + silk_RSHIFT_uint((b), (shift)))          /* shift >= 0 */
#define silk_SUB_LSHIFT32(a, b, shift)      silk_SUB32((a), silk_LSHIFT32((b), (shift)))    /* shift >= 0 */
#define silk_SUB_RSHIFT32(a, b, shift)      silk_SUB32((a), silk_RSHIFT32((b), (shift)))    /* shift >= 0 */

/* Requires that shift > 0 */
#define silk_RSHIFT_ROUND(a, shift)         ((shift) == 1 ? ((a) >> 1) + ((a) & 1) : (((a) >> ((shift) - 1)) + 1) >> 1)
#define silk_RSHIFT_ROUND64(a, shift)       ((shift) == 1 ? ((a) >> 1) + ((a) & 1) : (((a) >> ((shift) - 1)) + 1) >> 1)

/* Number of rightshift required to fit the multiplication */
```


`silk/NLSF_stabilize.c:40-142`

```c

#include "SigProc_FIX.h"

/* Constant Definitions */
#define MAX_LOOPS        20

/* NLSF stabilizer, for a single input data vector */
void silk_NLSF_stabilize(
          opus_int16            *NLSF_Q15,          /* I/O   Unstable/stabilized normalized LSF vector in Q15 [L]       */
    const opus_int16            *NDeltaMin_Q15,     /* I     Min distance vector, NDeltaMin_Q15[L] must be >= 1 [L+1]   */
    const opus_int              L                   /* I     Number of NLSF parameters in the input vector              */
)
{
    opus_int   i, I=0, k, loops;
    opus_int16 center_freq_Q15;
    opus_int32 diff_Q15, min_diff_Q15, min_center_Q15, max_center_Q15;

    /* This is necessary to ensure an output within range of a opus_int16 */
    silk_assert( NDeltaMin_Q15[L] >= 1 );

    for( loops = 0; loops < MAX_LOOPS; loops++ ) {
        /**************************/
        /* Find smallest distance */
        /**************************/
        /* First element */
        min_diff_Q15 = NLSF_Q15[0] - NDeltaMin_Q15[0];
        I = 0;
        /* Middle elements */
        for( i = 1; i <= L-1; i++ ) {
            diff_Q15 = NLSF_Q15[i] - ( NLSF_Q15[i-1] + NDeltaMin_Q15[i] );
            if( diff_Q15 < min_diff_Q15 ) {
                min_diff_Q15 = diff_Q15;
                I = i;
            }
        }
        /* Last element */
        diff_Q15 = ( 1 << 15 ) - ( NLSF_Q15[L-1] + NDeltaMin_Q15[L] );
        if( diff_Q15 < min_diff_Q15 ) {
            min_diff_Q15 = diff_Q15;
            I = L;
        }

        /***************************************************/
        /* Now check if the smallest distance non-negative */
        /***************************************************/
        if( min_diff_Q15 >= 0 ) {
            return;
        }

        if( I == 0 ) {
            /* Move away from lower limit */
            NLSF_Q15[0] = NDeltaMin_Q15[0];

        } else if( I == L) {
            /* Move away from higher limit */
            NLSF_Q15[L-1] = ( 1 << 15 ) - NDeltaMin_Q15[L];

        } else {
            /* Find the lower extreme for the location of the current center frequency */
            min_center_Q15 = 0;
            for( k = 0; k < I; k++ ) {
                min_center_Q15 += NDeltaMin_Q15[k];
            }
            min_center_Q15 += silk_RSHIFT( NDeltaMin_Q15[I], 1 );

            /* Find the upper extreme for the location of the current center frequency */
            max_center_Q15 = 1 << 15;
            for( k = L; k > I; k-- ) {
                max_center_Q15 -= NDeltaMin_Q15[k];
            }
            max_center_Q15 -= silk_RSHIFT( NDeltaMin_Q15[I], 1 );

            /* Move apart, sorted by value, keeping the same center frequency */
            center_freq_Q15 = (opus_int16)silk_LIMIT_32( silk_RSHIFT_ROUND( (opus_int32)NLSF_Q15[I-1] + (opus_int32)NLSF_Q15[I], 1 ),
                min_center_Q15, max_center_Q15 );
            NLSF_Q15[I-1] = center_freq_Q15 - silk_RSHIFT( NDeltaMin_Q15[I], 1 );
            NLSF_Q15[I] = NLSF_Q15[I-1] + NDeltaMin_Q15[I];
        }
    }

    /* Safe and simple fall back method, which is less ideal than the above */
    if( loops == MAX_LOOPS )
    {
        /* Insertion sort (fast for already almost sorted arrays):   */
        /* Best case:  O(n)   for an already sorted array            */
        /* Worst case: O(n^2) for an inversely sorted array          */
        silk_insertion_sort_increasing_all_values_int16( &NLSF_Q15[0], L );

        /* First NLSF should be no less than NDeltaMin[0] */
        NLSF_Q15[0] = silk_max_int( NLSF_Q15[0], NDeltaMin_Q15[0] );

        /* Keep delta_min distance between the NLSFs */
        for( i = 1; i < L; i++ )
            NLSF_Q15[i] = silk_max_int( NLSF_Q15[i], silk_ADD_SAT16( NLSF_Q15[i-1], NDeltaMin_Q15[i] ) );

        /* Last NLSF should be no higher than 1 - NDeltaMin[L] */
        NLSF_Q15[L-1] = silk_min_int( NLSF_Q15[L-1], (1<<15) - NDeltaMin_Q15[L] );

        /* Keep NDeltaMin distance between the NLSFs */
        for( i = L-2; i >= 0; i-- )
            NLSF_Q15[i] = silk_min_int( NLSF_Q15[i], NLSF_Q15[i+1] - NDeltaMin_Q15[i+1] );
    }
}
```


`celt/celt_lpc.c:140-283`

```c
      }
   }
#endif
}


void celt_fir_c(
         const opus_val16 *x,
         const opus_val16 *num,
         opus_val16 *y,
         int N,
         int ord,
         int arch)
{
   int i,j;
   VARDECL(opus_val16, rnum);
   SAVE_STACK;
   celt_assert(x != y);
   ALLOC(rnum, ord, opus_val16);
   for(i=0;i<ord;i++)
      rnum[i] = num[ord-i-1];
   for (i=0;i<N-3;i+=4)
   {
      opus_val32 sum[4];
      sum[0] = SHL32(EXTEND32(x[i  ]), SIG_SHIFT);
      sum[1] = SHL32(EXTEND32(x[i+1]), SIG_SHIFT);
      sum[2] = SHL32(EXTEND32(x[i+2]), SIG_SHIFT);
      sum[3] = SHL32(EXTEND32(x[i+3]), SIG_SHIFT);
#if defined(OPUS_CHECK_ASM) && defined(FIXED_POINT)
      {
         opus_val32 sum_c[4];
         memcpy(sum_c, sum, sizeof(sum_c));
         xcorr_kernel_c(rnum, x+i-ord, sum_c, ord);
#endif
         xcorr_kernel(rnum, x+i-ord, sum, ord, arch);
#if defined(OPUS_CHECK_ASM) && defined(FIXED_POINT)
         celt_assert(memcmp(sum, sum_c, sizeof(sum)) == 0);
      }
#endif
      y[i  ] = SROUND16(sum[0], SIG_SHIFT);
      y[i+1] = SROUND16(sum[1], SIG_SHIFT);
      y[i+2] = SROUND16(sum[2], SIG_SHIFT);
      y[i+3] = SROUND16(sum[3], SIG_SHIFT);
   }
   for (;i<N;i++)
   {
      opus_val32 sum = SHL32(EXTEND32(x[i]), SIG_SHIFT);
      for (j=0;j<ord;j++)
         sum = MAC16_16(sum,rnum[j],x[i+j-ord]);
      y[i] = SROUND16(sum, SIG_SHIFT);
   }
   RESTORE_STACK;
}

void celt_iir(const opus_val32 *_x,
         const opus_val16 *den,
         opus_val32 *_y,
         int N,
         int ord,
         opus_val16 *mem,
         int arch)
{
#ifdef SMALL_FOOTPRINT
   int i,j;
   (void)arch;
   for (i=0;i<N;i++)
   {
      opus_val32 sum = _x[i];
      for (j=0;j<ord;j++)
      {
         sum -= MULT16_16(den[j],mem[j]);
      }
      for (j=ord-1;j>=1;j--)
      {
         mem[j]=mem[j-1];
      }
      mem[0] = SROUND16(sum, SIG_SHIFT);
      _y[i] = sum;
   }
#else
   int i,j;
   VARDECL(opus_val16, rden);
   VARDECL(opus_val16, y);
   SAVE_STACK;

   celt_assert((ord&3)==0);
   ALLOC(rden, ord, opus_val16);
   ALLOC(y, N+ord, opus_val16);
   for(i=0;i<ord;i++)
      rden[i] = den[ord-i-1];
   for(i=0;i<ord;i++)
      y[i] = -mem[ord-i-1];
   for(;i<N+ord;i++)
      y[i]=0;
   for (i=0;i<N-3;i+=4)
   {
      /* Unroll by 4 as if it were an FIR filter */
      opus_val32 sum[4];
      sum[0]=_x[i];
      sum[1]=_x[i+1];
      sum[2]=_x[i+2];
      sum[3]=_x[i+3];
#if defined(OPUS_CHECK_ASM) && defined(FIXED_POINT)
      {
         opus_val32 sum_c[4];
         memcpy(sum_c, sum, sizeof(sum_c));
         xcorr_kernel_c(rden, y+i, sum_c, ord);
#endif
         xcorr_kernel(rden, y+i, sum, ord, arch);
#if defined(OPUS_CHECK_ASM) && defined(FIXED_POINT)
         celt_assert(memcmp(sum, sum_c, sizeof(sum)) == 0);
      }
#endif
      /* Patch up the result to compensate for the fact that this is an IIR */
      y[i+ord  ] = -SROUND16(sum[0],SIG_SHIFT);
      _y[i  ] = sum[0];
      sum[1] = MAC16_16(sum[1], y[i+ord  ], den[0]);
      y[i+ord+1] = -SROUND16(sum[1],SIG_SHIFT);
      _y[i+1] = sum[1];
      sum[2] = MAC16_16(sum[2], y[i+ord+1], den[0]);
      sum[2] = MAC16_16(sum[2], y[i+ord  ], den[1]);
      y[i+ord+2] = -SROUND16(sum[2],SIG_SHIFT);
      _y[i+2] = sum[2];

      sum[3] = MAC16_16(sum[3], y[i+ord+2], den[0]);
      sum[3] = MAC16_16(sum[3], y[i+ord+1], den[1]);
      sum[3] = MAC16_16(sum[3], y[i+ord  ], den[2]);
      y[i+ord+3] = -SROUND16(sum[3],SIG_SHIFT);
      _y[i+3] = sum[3];
   }
   for (;i<N;i++)
   {
      opus_val32 sum = _x[i];
      for (j=0;j<ord;j++)
         sum -= MULT16_16(rden[j],y[i+j]);
      y[i+ord] = SROUND16(sum,SIG_SHIFT);
      _y[i] = sum;
   }
   for(i=0;i<ord;i++)
      mem[i] = _y[N-i-1];
   RESTORE_STACK;
#endif
}

```


### 7.6 Table-driven and closed-form approximations

Every one of these functions is a *specification*, not an optimisation: the
encoder and decoder must agree with each other, and the bitstream depends on the
exact approximation. Port the code below literally; do not substitute `libm`.

`celt/mathops.c` (fixed-point, the ones that matter):

* `isqrt32` — Newton iteration with a `0x40000000u` seed.
* `frac_div32_q29` / `frac_div32` — `celt_rcp` and `SHR` based; note
  `frac_div32_q29` is used by the encoder's DNS/analysis.
* `celt_rsqrt_norm(x)` and `celt_rsqrt_norm32(x)` — polynomial in
  `x-32768`/`x-2^30` with `MULT16_16` terms; **`celt_rsqrt_norm` is used by the
  float-ish paths and by `celt_sqrt32`**.
* `celt_sqrt(x)` / `celt_sqrt32(x)` — built from `celt_rsqrt_norm(32)` and a
  final `MULT16_32_Q15`; the `if (x<=0) return 0;` and `x>=1073741824` early-out
  are part of the contract.
* `celt_cos_norm(x)` / `celt_cos_norm32(x)` — a **parabolic approximation**
  selected by the sign of `x`, with `x = x & 0x0001FFFF` normalisation.
  `celt_cos_norm` is called by `exp_rotation` (PVQ rotation) and by the MDCT
  window generation in `modes.c`, so a 1-LSB difference changes the bitstream.
* `celt_rcp_norm16(x)` / `celt_rcp_norm32` / `celt_rcp(x)` — reciprocal by
  `celt_udiv`-style division with a `celt_ilog2`-driven shift.
* `celt_exp2` / `celt_log2` are only defined in the float build (see §7.7);
  the fixed build has `celt_exp2_frac`/`celt_log2` in `celt/mathops.h` under
  `#ifndef FIXED_POINT`. In the fixed build the encoder uses
  `celt_log2`-free integer paths.

SILK approximations:

* `silk_sigm_Q15` — **piecewise-linear with 6 segments** and a
  `silk_LIMIT` clamp; the segment boundaries and slopes are hard-coded.
* `silk_lin2log` — `(x>>7)` clamp + `silk_CLZ_FRAC` + 8-entry parabola.
* `silk_log2lin` — inverse, with the `inLin = 0x7FFF` clamp and the
  `1<<(inLog>>7)` shift.
* `silk_LSFCosTab_FIX_Q12[LSF_COS_TAB_SZ_FIX+1]` — a **Q12 cosine table** used by
  `silk_NLSF2A`. It is generated by `opus_demo`-style tooling upstream; the
  values are in the source and are even (the comment says "Q12 values (even)").
  Transcribe verbatim, do not regenerate with `cos()`.
* `silk_SQRT_APPROX`, `silk_DIV32_varQ`, `silk_INVERSE32_varQ` — see §7.5.
* `silk_log2lin`/`silk_lin2log` are used for gains and NLSF weighting; a one-LSB
  error changes the SILK bitstream.


`celt/mathops.c:40-320`

```c
#include "mathops.h"

/*Compute floor(sqrt(_val)) with exact arithmetic.
  _val must be greater than 0.
  This has been tested on all possible 32-bit inputs greater than 0.*/
unsigned isqrt32(opus_uint32 _val){
  unsigned b;
  unsigned g;
  int      bshift;
  /*Uses the second method from
     http://www.azillionmonkeys.com/qed/sqroot.html
    The main idea is to search for the largest binary digit b such that
     (g+b)*(g+b) <= _val, and add it to the solution g.*/
  g=0;
  bshift=(EC_ILOG(_val)-1)>>1;
  b=1U<<bshift;
  do{
    opus_uint32 t;
    t=(((opus_uint32)g<<1)+b)<<bshift;
    if(t<=_val){
      g+=b;
      _val-=t;
    }
    b>>=1;
    bshift--;
  }
  while(bshift>=0);
  return g;
}

#ifdef FIXED_POINT

opus_val32 frac_div32_q29(opus_val32 a, opus_val32 b)
{
   opus_val16 rcp;
   opus_val32 result, rem;
   int shift = celt_ilog2(b)-29;
   a = VSHR32(a,shift);
   b = VSHR32(b,shift);
   /* 16-bit reciprocal */
   rcp = ROUND16(celt_rcp(ROUND16(b,16)),3);
   result = MULT16_32_Q15(rcp, a);
   rem = PSHR32(a,2)-MULT32_32_Q31(result, b);
   result = ADD32(result, SHL32(MULT16_32_Q15(rcp, rem),2));
   return result;
}

opus_val32 frac_div32(opus_val32 a, opus_val32 b) {
   opus_val32 result = frac_div32_q29(a,b);
   if (result >= 536870912)       /*  2^29 */
      return 2147483647;          /*  2^31 - 1 */
   else if (result <= -536870912) /* -2^29 */
      return -2147483647;         /* -2^31 */
   else
      return SHL32(result, 2);
}

/** Reciprocal sqrt approximation in the range [0.25,1) (Q16 in, Q14 out) */
opus_val16 celt_rsqrt_norm(opus_val32 x)
{
   opus_val16 n;
   opus_val16 r;
   opus_val16 r2;
   opus_val16 y;
   /* Range of n is [-16384,32767] ([-0.5,1) in Q15). */
   n = x-32768;
   /* Get a rough initial guess for the root.
      The optimal minimax quadratic approximation (using relative error) is
       r = 1.437799046117536+n*(-0.823394375837328+n*0.4096419668459485).
      Coefficients here, and the final result r, are Q14.*/
   r = ADD16(23557, MULT16_16_Q15(n, ADD16(-13490, MULT16_16_Q15(n, 6713))));
   /* We want y = x*r*r-1 in Q15, but x is 32-bit Q16 and r is Q14.
      We can compute the result from n and r using Q15 multiplies with some
       adjustment, carefully done to avoid overflow.
      Range of y is [-1564,1594]. */
   r2 = MULT16_16_Q15(r, r);
   y = SHL16(SUB16(ADD16(MULT16_16_Q15(r2, n), r2), 16384), 1);
   /* Apply a 2nd-order Householder iteration: r += r*y*(y*0.375-0.5).
      This yields the Q14 reciprocal square root of the Q16 x, with a maximum
       relative error of 1.04956E-4, a (relative) RMSE of 2.80979E-5, and a
       peak absolute error of 2.26591/16384. */
   return ADD16(r, MULT16_16_Q15(r, MULT16_16_Q15(y,
              SUB16(MULT16_16_Q15(y, 12288), 16384))));
}

/** Reciprocal sqrt approximation in the range [0.25,1) (Q31 in, Q29 out) */
opus_val32 celt_rsqrt_norm32(opus_val32 x)
{
   opus_int32 tmp;
   /* Use the first-order Newton-Raphson method to refine the root estimate.
    * r = r * (1.5 - 0.5*x*r*r) */
   opus_int32 r_q29 = SHL32(celt_rsqrt_norm(SHR32(x, 31-16)), 15);
   /* Split evaluation in steps to avoid exploding macro expansion. */
   tmp = MULT32_32_Q31(r_q29, r_q29);
   tmp = MULT32_32_Q31(1073741824 /* Q31 */, tmp);
   tmp = MULT32_32_Q31(x, tmp);
   return SHL32(MULT32_32_Q31(r_q29, SUB32(201326592 /* Q27 */, tmp)), 4);
}

/** Sqrt approximation (QX input, QX/2 output) */
opus_val32 celt_sqrt(opus_val32 x)
{
   int k;
   opus_val16 n;
   opus_val32 rt;
   /* These coeffs are optimized in fixed-point to minimize both RMS and max error
      of sqrt(x) over .25<x<1 without exceeding 32767.
      The RMS error is 3.4e-5 and the max is 8.2e-5. */
   static const opus_val16 C[6] = {23171, 11574, -2901, 1592, -1002, 336};
   if (x==0)
      return 0;
   else if (x>=1073741824)
      return 32767;
   k = (celt_ilog2(x)>>1)-7;
   x = VSHR32(x, 2*k);
   n = x-32768;
   rt = ADD32(C[0], MULT16_16_Q15(n, ADD16(C[1], MULT16_16_Q15(n, ADD16(C[2],
              MULT16_16_Q15(n, ADD16(C[3], MULT16_16_Q15(n, ADD16(C[4], MULT16_16_Q15(n, (C[5])))))))))));
   rt = VSHR32(rt,7-k);
   return rt;
}

/* Perform fixed-point arithmetic to approximate the square root. When the input
 * is in Qx format, the output will be in Q(x/2 + 16) format. */
opus_val32 celt_sqrt32(opus_val32 x)
{
   int k;
   opus_int32 x_frac;
   if (x==0)
      return 0;
   else if (x>=1073741824)
      return 2147483647; /* 2^31 -1 */
   k = (celt_ilog2(x)>>1);
   x_frac = VSHR32(x, 2*(k-14)-1);
   x_frac = MULT32_32_Q31(celt_rsqrt_norm32(x_frac), x_frac);
   if (k < 12) return PSHR32(x_frac, 12-k);
   else return SHL32(x_frac, k-12);
}

#define L1 32767
#define L2 -7651
#define L3 8277
#define L4 -626

static OPUS_INLINE opus_val16 _celt_cos_pi_2(opus_val16 x)
{
   opus_val16 x2;

   x2 = MULT16_16_P15(x,x);
   return ADD16(1,MIN16(32766,ADD32(SUB16(L1,x2), MULT16_16_P15(x2, ADD32(L2, MULT16_16_P15(x2, ADD32(L3, MULT16_16_P15(L4, x2
                                                                                ))))))));
}

#undef L1
#undef L2
#undef L3
#undef L4

opus_val16 celt_cos_norm(opus_val32 x)
{
   x = x&0x0001ffff;
   if (x>SHL32(EXTEND32(1), 16))
      x = SUB32(SHL32(EXTEND32(1), 17),x);
   if (x&0x00007fff)
   {
      if (x<SHL32(EXTEND32(1), 15))
      {
         return _celt_cos_pi_2(EXTRACT16(x));
      } else {
         return NEG16(_celt_cos_pi_2(EXTRACT16(65536-x)));
      }
   } else {
      if (x&0x0000ffff)
         return 0;
      else if (x&0x0001ffff)
         return -32767;
      else
         return 32767;
   }
}

/* Calculates the cosine of (PI*0.5*x) where the input x ranges from -1 to 1 and
 * is in Q30 format. The output will also be in Q31 format. */
opus_val32 celt_cos_norm32(opus_val32 x)
{
   static const opus_val32 COS_NORM_COEFF_A0 = 134217720;   /* Q27 */
   static const opus_val32 COS_NORM_COEFF_A1 = -662336704;  /* Q29 */
   static const opus_val32 COS_NORM_COEFF_A2 = 544710848;   /* Q31 */
   static const opus_val32 COS_NORM_COEFF_A3 = -178761936;  /* Q33 */
   static const opus_val32 COS_NORM_COEFF_A4 = 29487206;    /* Q35 */
   opus_int32 x_sq_q29, tmp;
   /* The expected x is in the range of [-1.0f, 1.0f] */
   celt_sig_assert((x >= -1073741824) && (x <= 1073741824));
   /* Make cos(+/- pi/2) exactly zero. */
   if (ABS32(x) == 1<<30) return 0;
   x_sq_q29 = MULT32_32_Q31(x, x);
   /* Split evaluation in steps to avoid exploding macro expansion. */
   tmp = ADD32(COS_NORM_COEFF_A3, MULT32_32_Q31(x_sq_q29, COS_NORM_COEFF_A4));
   tmp = ADD32(COS_NORM_COEFF_A2, MULT32_32_Q31(x_sq_q29, tmp));
   tmp = ADD32(COS_NORM_COEFF_A1, MULT32_32_Q31(x_sq_q29, tmp));
   return SHL32(ADD32(COS_NORM_COEFF_A0, MULT32_32_Q31(x_sq_q29, tmp)), 4);
}

/* Computes a 16 bit approximate reciprocal (1/x) for a normalized Q15 input,
 * resulting in a Q15 output. */
opus_val16 celt_rcp_norm16(opus_val16 x)
{
   opus_val16 r;
   /* Start with a linear approximation:
      r = 1.8823529411764706-0.9411764705882353*n.
      The coefficients and the result are Q14 in the range [15420,30840].*/
   r = ADD16(30840, MULT16_16_Q15(-15420, x));
   /* Perform two Newton iterations:
      r -= r*((r*n)+(r-1.Q15))
         = r*((r*n)+(r-1.Q15)). */
   r = SUB16(r, MULT16_16_Q15(r,
             ADD16(MULT16_16_Q15(r, x), ADD16(r, -32768))));
   /* We subtract an extra 1 in the second iteration to avoid overflow; it also
       neatly compensates for truncation error in the rest of the process. */
   return SUB16(r, ADD16(1, MULT16_16_Q15(r,
                ADD16(MULT16_16_Q15(r, x), ADD16(r, -32768)))));
}

/* Computes a 32 bit approximated reciprocal (1/x) for a normalized Q31 input,
 * resulting in a Q30 output. The expected input range is [0.5f, 1.0f) in Q31
 * and the expected output range is [1.0f, 2.0f) in Q30. */
opus_val32 celt_rcp_norm32(opus_val32 x)
{
   opus_val32 r_q30;
   celt_sig_assert(x >= 1073741824);
   r_q30 = SHL32(EXTEND32(celt_rcp_norm16(SHR32(x, 15)-32768)), 16);
   /* Solving f(y) = a - 1/y using the Newton Method
    * Note: f(y)' = 1/y^2
    * r = r - f(r)/f(r)' = r - (x * r*r - r)
    *   = r - r*(r*x - 1)
    * where
    *   - r means 1/y's approximation.
    *   - x means a, the input of function.
    * Please note that:
    *   - It adds 1 to avoid overflow
    *   - -1.0f in Q30 is -1073741824. */
   return SUB32(r_q30, ADD32(SHL32(
                MULT32_32_Q31(ADD32(MULT32_32_Q31(r_q30, x), -1073741824),
                              r_q30), 1), 1));
}

/** Reciprocal approximation (Q15 input, Q16 output) */
opus_val32 celt_rcp(opus_val32 x)
{
   int i;
   opus_val16 r;
   celt_sig_assert(x>0);
   i = celt_ilog2(x);

   /* Compute the reciprocal of a Q15 number in the range [0, 1). */
   r = celt_rcp_norm16(VSHR32(x,i-15)-32768);

   /* r is now the Q15 solution to 2/(n+1), with a maximum relative error
       of 7.05346E-5, a (relative) RMSE of 2.14418E-5, and a peak absolute
       error of 1.24665/32768. */
   return VSHR32(EXTEND32(r),i-16);
}

#endif

#ifndef DISABLE_FLOAT_API

void celt_float2int16_c(const float * OPUS_RESTRICT in, short * OPUS_RESTRICT out, int cnt)
{
   int i;
   for (i = 0; i < cnt; i++)
   {
      out[i] = FLOAT2INT16(in[i]);
   }
}

int opus_limit2_checkwithin1_c(float * samples, int cnt)
{
   int i;
   if (cnt <= 0)
   {
```


`celt/mathops.h:60-240`

```c
static OPUS_INLINE float fast_atan2f(float y, float x) {
   float x2, y2;
   x2 = x*x;
   y2 = y*y;
   /* For very small values, we don't care about the answer, so
      we can just return 0. */
   if (x2 + y2 < 1e-18f)
   {
      return 0;
   }
   if(x2<y2){
      float den = (y2 + cB*x2) * (y2 + cC*x2);
      return -x*y*(y2 + cA*x2) / den + (y<0 ? -cE : cE);
   }else{
      float den = (x2 + cB*y2) * (x2 + cC*y2);
      return  x*y*(x2 + cA*y2) / den + (y<0 ? -cE : cE) - (x*y<0 ? -cE : cE);
   }
}
#undef cA
#undef cB
#undef cC
#undef cE
#endif


#ifndef OVERRIDE_CELT_MAXABS16
static OPUS_INLINE opus_val32 celt_maxabs16(const opus_val16 *x, int len)
{
   int i;
   opus_val16 maxval = 0;
   opus_val16 minval = 0;
   for (i=0;i<len;i++)
   {
      maxval = MAX16(maxval, x[i]);
      minval = MIN16(minval, x[i]);
   }
   return MAX32(EXTEND32(maxval),-EXTEND32(minval));
}
#endif

#if defined(ENABLE_RES24) && defined(FIXED_POINT)
static OPUS_INLINE opus_res celt_maxabs_res(const opus_res *x, int len)
{
   int i;
   opus_res maxval = 0;
   opus_res minval = 0;
   for (i=0;i<len;i++)
   {
      maxval = MAX32(maxval, x[i]);
      minval = MIN32(minval, x[i]);
   }
   /* opus_res should never reach such amplitude, so we should be safe. */
   celt_sig_assert(minval != -2147483648);
   return MAX32(maxval,-minval);
}
#else
#define celt_maxabs_res celt_maxabs16
#endif


#ifndef OVERRIDE_CELT_MAXABS32
#ifdef FIXED_POINT
static OPUS_INLINE opus_val32 celt_maxabs32(const opus_val32 *x, int len)
{
   int i;
   opus_val32 maxval = 0;
   opus_val32 minval = 0;
   for (i=0;i<len;i++)
   {
      maxval = MAX32(maxval, x[i]);
      minval = MIN32(minval, x[i]);
   }
   return MAX32(maxval, -minval);
}
#else
#define celt_maxabs32(x,len) celt_maxabs16(x,len)
#endif
#endif

#ifndef FIXED_POINT
/* Calculates the arctangent of x using a Remez approximation of order 15,
 * incorporating only odd-powered terms. */
static OPUS_INLINE float celt_atan_norm(float x)
{
   #define ATAN2_2_OVER_PI 0.636619772367581f
   float x_sq = x * x;

   /* Polynomial coefficients approximated in the [0, 1] range.
    * Lolremez command: lolremez --degree 6 --range "0:1"
    *                   "(atan(sqrt(x))-sqrt(x))/(x*sqrt(x))" "1/(sqrt(x)*x)"
    * Please note that ATAN2_COEFF_A01 is fixed to 1.0f. */
   #define ATAN2_COEFF_A03 -3.3331659436225891113281250000e-01f
   #define ATAN2_COEFF_A05 1.99627041816711425781250000000e-01f
   #define ATAN2_COEFF_A07 -1.3976582884788513183593750000e-01f
   #define ATAN2_COEFF_A09 9.79423448443412780761718750000e-02f
   #define ATAN2_COEFF_A11 -5.7773590087890625000000000000e-02f
   #define ATAN2_COEFF_A13 2.30401363223791122436523437500e-02f
   #define ATAN2_COEFF_A15 -4.3554059229791164398193359375e-03f
   return ATAN2_2_OVER_PI * (x + x * x_sq * (ATAN2_COEFF_A03
                + x_sq * (ATAN2_COEFF_A05
                + x_sq * (ATAN2_COEFF_A07
                + x_sq * (ATAN2_COEFF_A09
                + x_sq * (ATAN2_COEFF_A11
                + x_sq * (ATAN2_COEFF_A13
                + x_sq * (ATAN2_COEFF_A15))))))));
}

/* Calculates the arctangent of y/x, returning an approximate value in radians.
 * Please refer to the linked wiki page (https://en.wikipedia.org/wiki/Atan2)
 * to learn how atan2 results are computed. */
static OPUS_INLINE float celt_atan2p_norm(float y, float x)
{
   celt_sig_assert(x>=0 && y>=0);

   /* For very small values, we don't care about the answer. */
   if ((x*x + y*y) < 1e-18f)
   {
      return 0;
   }

   if (y < x)
   {
      return celt_atan_norm(y / x);
   } else {
      return 1.f - celt_atan_norm(x / y);
   }
}
#endif

#if !defined(FIXED_POINT) || defined(ENABLE_QEXT)
/* Computes estimated cosine values for (PI/2 * x) using only terms with even
 * exponents. */
static OPUS_INLINE float celt_cos_norm2(float x)
{
   float x_norm_sq;
   int output_sign;
   /* Restrict x to [-1, 3]. */
   x -= 4*floor(.25*(x+1));
   /* Negative sign for [1, 3]. */
   output_sign = 1 - 2*(x>1);
   /* Restrict to [-1, 1]. */
   x -= 2*(x>1);

   /* The cosine function, cos(x), has a Taylor series representation consisting
    * exclusively of even-powered polynomial terms. */
   x_norm_sq = x * x;

   /* Polynomial coefficients approximated in the [0, 1] range using only terms
    * with even exponents.
    * Lolremez command: lolremez --degree 4 --range 0:1 "cos(sqrt(x)*pi*0.5)" */
   #define COS_COEFF_A0 9.999999403953552246093750000000e-01f
   #define COS_COEFF_A2 -1.233698248863220214843750000000000f
   #define COS_COEFF_A4 2.536507546901702880859375000000e-01f
   #define COS_COEFF_A6 -2.08106283098459243774414062500e-02f
   #define COS_COEFF_A8 8.581906440667808055877685546875e-04f
   return output_sign * (COS_COEFF_A0 + x_norm_sq * (COS_COEFF_A2 +
                               x_norm_sq * (COS_COEFF_A4 +
                               x_norm_sq * (COS_COEFF_A6 +
                               x_norm_sq * (COS_COEFF_A8)))));
}

#endif

#ifndef FIXED_POINT

#define celt_sqrt(x) ((float)sqrt(x))
#define celt_sqrt32(x) ((float)sqrt(x))
#define celt_rsqrt(x) (1.f/celt_sqrt(x))
#define celt_rsqrt_norm(x) (celt_rsqrt(x))
#define celt_rsqrt_norm32(x) (celt_rsqrt(x))
#define celt_cos_norm(x) ((float)cos((.5f*PI)*(x)))
#define celt_rcp(x) (1.f/(x))
#define celt_div(a,b) ((a)/(b))
#define frac_div32(a,b) ((float)(a)/(b))
#define frac_div32_q29(a,b) frac_div32(a,b)

#ifdef FLOAT_APPROX
/* Calculates the base-2 logarithm (log2(x)) of a number. It is designed for
 * systems using radix-2 floating-point representation, with the exponent
 * located at bits 23 to 30 and an offset of 127. Note that special cases like
 * denormalized numbers, positive/negative infinity, and NaN are not handled.
```


`celt/mathops.h:240-360`

```c
 * denormalized numbers, positive/negative infinity, and NaN are not handled.
 * log2(x) = log2(x^exponent * mantissa)
 *         = exponent + log2(mantissa) */

/* Log2 x normalization single precision coefficients calculated by
 * 1 / (1 + 0.125 * index).
 * Coefficients in Double Precision
 * double log2_x_norm_coeff[8] = {
 *    1.0000000000000000000, 8.888888888888888e-01,
 *    8.000000000000000e-01, 7.272727272727273e-01,
 *    6.666666666666666e-01, 6.153846153846154e-01,
 *    5.714285714285714e-01, 5.333333333333333e-01} */
static const float log2_x_norm_coeff[8] = {
   1.000000000000000000000000000f, 8.88888895511627197265625e-01f,
   8.00000000000000000000000e-01f, 7.27272748947143554687500e-01f,
   6.66666686534881591796875e-01f, 6.15384638309478759765625e-01f,
   5.71428596973419189453125e-01f, 5.33333361148834228515625e-01f};

/* Log2 y normalization single precision coefficients calculated by
 * log2(1 + 0.125 * index).
 * Coefficients in Double Precision
 * double log2_y_norm_coeff[8] = {
 *    0.0000000000000000000, 1.699250014423124e-01,
 *    3.219280948873623e-01, 4.594316186372973e-01,
 *    5.849625007211562e-01, 7.004397181410922e-01,
 *    8.073549220576041e-01, 9.068905956085185e-01}; */
static const float log2_y_norm_coeff[8] = {
   0.0000000000000000000000000000f, 1.699250042438507080078125e-01f,
   3.219280838966369628906250e-01f, 4.594316184520721435546875e-01f,
   5.849624872207641601562500e-01f, 7.004396915435791015625000e-01f,
   8.073549270629882812500000e-01f, 9.068905711174011230468750e-01f};

static OPUS_INLINE float celt_log2(float x)
{
   opus_int32 integer;
   opus_int32 range_idx;
   union {
      float f;
      opus_uint32 i;
   } in;
   in.f = x;
   integer = (opus_int32)(in.i>>23)-127;
   in.i = (opus_int32)in.i - (opus_int32)((opus_uint32)integer<<23);

   /* Normalize the mantissa range from [1, 2] to [1,1.125], and then shift x
    * by 1.0625 to [-0.0625, 0.0625]. */
   range_idx = (in.i >> 20) & 0x7;
   in.f = in.f * log2_x_norm_coeff[range_idx] - 1.0625f;

   /* Polynomial coefficients approximated in the [1, 1.125] range.
    * Lolremez command: lolremez --degree 4 --range -0.0625:0.0625
    *                   "log(x+1.0625)/log(2)"
    * Coefficients in Double Precision
    * A0: 8.7462840624502679e-2    A1: 1.3578296070972002
    * A2: -6.3897703690210047e-1   A3: 4.0197125617419959e-1
    * A4: -2.8415445877832832e-1 */
   #define LOG2_COEFF_A0 8.74628424644470214843750000e-02f
   #define LOG2_COEFF_A1 1.357829570770263671875000000000f
   #define LOG2_COEFF_A2 -6.3897705078125000000000000e-01f
   #define LOG2_COEFF_A3 4.01971250772476196289062500e-01f
   #define LOG2_COEFF_A4 -2.8415444493293762207031250e-01f
   in.f = LOG2_COEFF_A0 + in.f * (LOG2_COEFF_A1
               + in.f * (LOG2_COEFF_A2
               + in.f * (LOG2_COEFF_A3
               + in.f * (LOG2_COEFF_A4))));
   return integer + in.f + log2_y_norm_coeff[range_idx];
}

/* Calculates an approximation of 2^x. The approximation was achieved by
 * employing a base-2 exponential function and utilizing a Remez approximation
 * of order 5, ensuring a controlled relative error.
 * exp2(x) = exp2(integer + fraction)
 *         = exp2(integer) * exp2(fraction) */
static OPUS_INLINE float celt_exp2(float x)
{
   opus_int32 integer;
   float frac;
   union {
      float f;
      opus_uint32 i;
   } res;
   integer = (int)floor(x);
   if (integer < -50)
      return 0;
   frac = x-integer;

   /* Polynomial coefficients approximated in the [0, 1] range.
    * Lolremez command: lolremez --degree 5 --range 0:1
    *                   "exp(x*0.693147180559945)" "exp(x*0.693147180559945)"
    * NOTE: log(2) ~ 0.693147180559945 */
   #define EXP2_COEFF_A0 9.999999403953552246093750000000e-01f
   #define EXP2_COEFF_A1 6.931530833244323730468750000000e-01f
   #define EXP2_COEFF_A2 2.401536107063293457031250000000e-01f
   #define EXP2_COEFF_A3 5.582631751894950866699218750000e-02f
   #define EXP2_COEFF_A4 8.989339694380760192871093750000e-03f
   #define EXP2_COEFF_A5 1.877576694823801517486572265625e-03f
   res.f = EXP2_COEFF_A0 + frac * (EXP2_COEFF_A1
               + frac * (EXP2_COEFF_A2
               + frac * (EXP2_COEFF_A3
               + frac * (EXP2_COEFF_A4
               + frac * (EXP2_COEFF_A5)))));
   res.i = (opus_uint32)((opus_int32)res.i + (opus_int32)((opus_uint32)integer<<23)) & 0x7fffffff;
   return res.f;
}

#else
#define celt_log2(x) ((float)(1.442695040888963387*log(x)))
#define celt_exp2(x) ((float)exp(0.6931471805599453094*(x)))
#endif

#define celt_exp2_db celt_exp2
#define celt_log2_db celt_log2

#define celt_sin(x) celt_cos_norm2((0.5f*PI) * (x) - 1.0f)
#define celt_log(x) (celt_log2(x) * 0.6931471805599453f)
#define celt_exp(x) (celt_exp2((x) * 1.4426950408889634f))

#endif

#ifdef FIXED_POINT

```


`silk/Inlines.h:30-92`

```c
 */

#ifndef SILK_FIX_INLINES_H
#define SILK_FIX_INLINES_H


/* count leading zeros of opus_int64 */
static OPUS_INLINE opus_int32 silk_CLZ64( opus_int64 in )
{
    opus_int32 in_upper;

    in_upper = (opus_int32)silk_RSHIFT64(in, 32);
    if (in_upper == 0) {
        /* Search in the lower 32 bits */
        return 32 + silk_CLZ32( (opus_int32) in );
    } else {
        /* Search in the upper 32 bits */
        return silk_CLZ32( in_upper );
    }
}

/* get number of leading zeros and fractional part (the bits right after the leading one */
static OPUS_INLINE void silk_CLZ_FRAC(
    opus_int32 in,            /* I  input                               */
    opus_int32 *lz,           /* O  number of leading zeros             */
    opus_int32 *frac_Q7       /* O  the 7 bits right after the leading one */
)
{
    opus_int32 lzeros = silk_CLZ32(in);

    * lz = lzeros;
    * frac_Q7 = silk_ROR32(in, 24 - lzeros) & 0x7f;
}

/* Approximation of square root                                          */
/* Accuracy: < +/- 10%  for output values > 15                           */
/*           < +/- 2.5% for output values > 120                          */
static OPUS_INLINE opus_int32 silk_SQRT_APPROX( opus_int32 x )
{
    opus_int32 y, lz, frac_Q7;

    if( x <= 0 ) {
        return 0;
    }

    silk_CLZ_FRAC(x, &lz, &frac_Q7);

    if( lz & 1 ) {
        y = 32768;
    } else {
        y = 46214;        /* 46214 = sqrt(2) * 32768 */
    }

    /* get scaling right */
    y >>= silk_RSHIFT(lz, 1);

    /* increment using fractional part of input */
    y = silk_SMLAWB(y, y, silk_SMULBB(213, frac_Q7));

    return y;
}

/* Divide two int32 values and return result as int32 in a given Q-domain */
```


`silk/sigm_Q15.c:40-75`

```c
/* fprintf(1, '%d, ', round(32767 * 1 ./ (1 + exp(-(0:5))))); */
static const opus_int32 sigm_LUT_pos_Q15[ 6 ] = {
    16384, 23955, 28861, 31213, 32178, 32548
};
/* fprintf(1, '%d, ', round(32767 * 1 ./ (1 + exp((0:5))))); */
static const opus_int32 sigm_LUT_neg_Q15[ 6 ] = {
    16384, 8812, 3906, 1554, 589, 219
};

opus_int silk_sigm_Q15(
    opus_int                    in_Q5               /* I                                                                */
)
{
    opus_int ind;

    if( in_Q5 < 0 ) {
        /* Negative input */
        in_Q5 = -in_Q5;
        if( in_Q5 >= 6 * 32 ) {
            return 0;        /* Clip */
        } else {
            /* Linear interpolation of look up table */
            ind = silk_RSHIFT( in_Q5, 5 );
            return( sigm_LUT_neg_Q15[ ind ] - silk_SMULBB( sigm_LUT_slope_Q10[ ind ], in_Q5 & 0x1F ) );
        }
    } else {
        /* Positive input */
        if( in_Q5 >= 6 * 32 ) {
            return 32767;        /* clip */
        } else {
            /* Linear interpolation of look up table */
            ind = silk_RSHIFT( in_Q5, 5 );
            return( sigm_LUT_pos_Q15[ ind ] + silk_SMULBB( sigm_LUT_slope_Q10[ ind ], in_Q5 & 0x1F ) );
        }
    }
}
```


`silk/lin2log.c:30-45`

```c
#endif

#include "SigProc_FIX.h"
/* Approximation of 128 * log2() (very close inverse of silk_log2lin()) */
/* Convert input to a log scale    */
opus_int32 silk_lin2log(
    const opus_int32            inLin               /* I  input in linear scale                                         */
)
{
    opus_int32 lz, frac_Q7;

    silk_CLZ_FRAC( inLin, &lz, &frac_Q7 );

    /* Piece-wise parabolic approximation */
    return silk_ADD_LSHIFT32( silk_SMLAWB( frac_Q7, silk_MUL( frac_Q7, 128 - frac_Q7 ), 179 ), 31 - lz, 7 );
}
```


`silk/log2lin.c:30-58`

```c
#endif

#include "SigProc_FIX.h"

/* Approximation of 2^() (very close inverse of silk_lin2log()) */
/* Convert input to a linear scale    */
opus_int32 silk_log2lin(
    const opus_int32            inLog_Q7            /* I  input on log scale                                            */
)
{
    opus_int32 out, frac_Q7;

    if( inLog_Q7 < 0 ) {
        return 0;
    } else if ( inLog_Q7 >= 3967 ) {
        return silk_int32_MAX;
    }

    out = silk_LSHIFT( 1, silk_RSHIFT( inLog_Q7, 7 ) );
    frac_Q7 = inLog_Q7 & 0x7F;
    if( inLog_Q7 < 2048 ) {
        /* Piece-wise parabolic approximation */
        out = silk_ADD_RSHIFT32( out, silk_MUL( out, silk_SMLAWB( frac_Q7, silk_SMULBB( frac_Q7, 128 - frac_Q7 ), -174 ) ), 7 );
    } else {
        /* Piece-wise parabolic approximation */
        out = silk_MLA( out, silk_RSHIFT( out, 7 ), silk_SMLAWB( frac_Q7, silk_SMULBB( frac_Q7, 128 - frac_Q7 ), -174 ) );
    }
    return out;
}
```


`silk/table_LSF_cos.c:33-70`

```c

/* Cosine approximation table for LSF conversion */
/* Q12 values (even) */
const opus_int16 silk_LSFCosTab_FIX_Q12[ LSF_COS_TAB_SZ_FIX + 1 ] = {
            8192,             8190,             8182,             8170,
            8152,             8130,             8104,             8072,
            8034,             7994,             7946,             7896,
            7840,             7778,             7714,             7644,
            7568,             7490,             7406,             7318,
            7226,             7128,             7026,             6922,
            6812,             6698,             6580,             6458,
            6332,             6204,             6070,             5934,
            5792,             5648,             5502,             5352,
            5198,             5040,             4880,             4718,
            4552,             4382,             4212,             4038,
            3862,             3684,             3502,             3320,
            3136,             2948,             2760,             2570,
            2378,             2186,             1990,             1794,
            1598,             1400,             1202,             1002,
             802,              602,              402,              202,
               0,             -202,             -402,             -602,
            -802,            -1002,            -1202,            -1400,
           -1598,            -1794,            -1990,            -2186,
           -2378,            -2570,            -2760,            -2948,
           -3136,            -3320,            -3502,            -3684,
           -3862,            -4038,            -4212,            -4382,
           -4552,            -4718,            -4880,            -5040,
           -5198,            -5352,            -5502,            -5648,
           -5792,            -5934,            -6070,            -6204,
           -6332,            -6458,            -6580,            -6698,
           -6812,            -6922,            -7026,            -7128,
           -7226,            -7318,            -7406,            -7490,
           -7568,            -7644,            -7714,            -7778,
           -7840,            -7896,            -7946,            -7994,
           -8034,            -8072,            -8104,            -8130,
           -8152,            -8170,            -8182,            -8190,
           -8192
};
```


### 7.7 Float rounding: `float` vs `double`, and the float-only encoder paths

Even in the `FIXED_POINT` build there are float touch points, and in the float
build the whole analysis path is float.

* `celt/float_cast.h` — `FLOAT2INT16`/`FLOAT2INT24`/`float2int`/`int16tofloat`.
  Under `HAVE_LRINTF` these use `lrintf` (**round-to-nearest-even**, honouring the
  FP environment); otherwise they use a `+ (1<<15)`-style **truncating** cast.
  The two differ by one LSB on exact halves. **The port must pick the branch that
  the reference build used** — for glibc, `HAVE_LRINTF` is defined, so
  `lrintf` semantics (round-half-to-even) are normative.
* `celt/mathops.h` under `#ifdef FLOAT_APPROX` uses
  `celt_sqrt(x) = (float)sqrt(x)`, `celt_rsqrt(x) = 1.f/celt_sqrt(x)`,
  `celt_cos_norm(x) = (float)cos((.5f*PI)*(x))`,
  `celt_log2(x) = (float)(1.442695040888963387*log(x))`,
  `celt_exp2(x) = (float)exp(0.6931471805599453094*(x))`,
  `celt_rcp(x) = (1.f/(x))`. The constants and the `float` casts are exact
  requirements if a float build is ever targeted; with `FLOAT_APPROX` **off** the
  same functions are implemented as explicit `float` polynomial/rational
  approximations in `celt/mathops.h` (also included below).
* `silk/float/*.c` — the float SILK encoder. It uses `silk_float` (`float`) but
  accumulates some sums in `double` (e.g. `silk_energy_FLP`,
  `silk_inner_product_FLP`) and relies on `-ffp-contract` behaviour for
  FMA contraction. **Not bit-exact across platforms.** If the port must match a
  float reference, it must reproduce the exact expression trees and the
  `float`/`double` widths, and disable FMA contraction.
* `src/analysis.c` / `src/mlp.c` — the `AnalysisInfo` decision layer. It is
  compiled only when the float API is enabled, and its outputs *change the CELT
  encoder's decisions* (`CELT_SET_MUSIC_PROB`, `CELT_SET_LEAK_BOOST`, etc.), so
  for bit-exactness against an autotools float-API build the port must reproduce
  it too — including `tonality`/`music_prob` thresholds and the MLP weights
  (table manifest in §6.4).
* `float` vs `double` in `celt/mathops.c`'s `#else` (non-`FLOAT_APPROX`) branch:
  inspect each function — the return is cast to `float` but intermediates are
  declared `float` explicitly. In Rust use `f32` everywhere and avoid `f64`
  promotion (`as f64`); Rust does not promote automatically, but
  `libm`-vs-`std` differences remain a risk.


`celt/float_cast.h:40-176`

```c
**
**      Changing the FPU rounding mode causes the FPU pipeline to be flushed. It
**      is this flushing of the pipeline which is so slow.
**
**      Fortunately the ISO C99 specifications define the functions lrint, lrintf,
**      llrint and llrintf which fix this problem as a side effect.
**
**      On Unix-like systems, the configure process should have detected the
**      presence of these functions. If they weren't found we have to replace them
**      here with a standard C cast.
*/

/*
**      The C99 prototypes for lrint and lrintf are as follows:
**
**              long int lrintf (float x) ;
**              long int lrint  (double x) ;
*/

/*      The presence of the required functions are detected during the configure
**      process and the values HAVE_LRINT and HAVE_LRINTF are set accordingly in
**      the config.h file.
*/

/* With GCC, when SSE is available, the fastest conversion is cvtss2si. */
#if defined(__GNUC__) && defined(__SSE__)

#include <xmmintrin.h>
static OPUS_INLINE opus_int32 float2int(float x) {return _mm_cvt_ss2si(_mm_set_ss(x));}

#elif (defined(_MSC_VER) && _MSC_VER >= 1400) && (defined(_M_X64) || (defined(_M_IX86_FP) && _M_IX86_FP >= 1))

        #include <xmmintrin.h>
        static OPUS_INLINE opus_int32 float2int(float value)
        {
                /* _mm_load_ss will generate same code as _mm_set_ss
                ** in _MSC_VER >= 1914 /02 so keep __mm_load__ss
                ** for backward compatibility.
                */
                return _mm_cvtss_si32(_mm_load_ss(&value));
        }

#elif (defined(_MSC_VER) && _MSC_VER >= 1400) && defined (_M_IX86)

        #include <math.h>

        /*      Win32 doesn't seem to have these functions.
        **      Therefore implement OPUS_INLINE versions of these functions here.
        */

        static OPUS_INLINE opus_int32
        float2int (float flt)
        {       int intgr;

                _asm
                {       fld flt
                        fistp intgr
                } ;

                return intgr ;
        }
#elif defined(__aarch64__)

        #include <arm_neon.h>
        static OPUS_INLINE opus_int32 float2int(float flt)
        {
                return vcvtns_s32_f32(flt);
        }

#elif defined(HAVE_LRINTF) && defined(__STDC_VERSION__) && __STDC_VERSION__ >= 199901L

/*      These defines enable functionality introduced with the 1999 ISO C
**      standard. They must be defined before the inclusion of math.h to
**      engage them. If optimisation is enabled, these functions will be
**      inlined. With optimisation switched off, you have to link in the
**      maths library using -lm.
*/

#define _ISOC9X_SOURCE 1
#define _ISOC99_SOURCE 1

#define __USE_ISOC9X 1
#define __USE_ISOC99 1

#include <math.h>
#define float2int(x) lrintf(x)

#elif defined(HAVE_LRINT) && defined(__STDC_VERSION__) && __STDC_VERSION__ >= 199901L

#define _ISOC9X_SOURCE 1
#define _ISOC99_SOURCE 1

#define __USE_ISOC9X 1
#define __USE_ISOC99 1

#include <math.h>
#define float2int(x) lrint(x)

#else

#if (defined(__GNUC__) && defined(__STDC__) && __STDC__ && __STDC_VERSION__ >= 199901L)
        /* supported by gcc in C99 mode, but not by all other compilers */
        #warning "Don't have the functions lrint() and lrintf ()."
        #warning "Replacing these functions with a standard C cast."
#endif /* __STDC_VERSION__ >= 199901L */
        #include <math.h>
        #define float2int(flt) ((int)(floor(.5+flt)))
#endif

#ifndef DISABLE_FLOAT_API
static OPUS_INLINE opus_int16 FLOAT2INT16(float x)
{
   x = x*CELT_SIG_SCALE;
   x = MAX32(x, -32768);
   x = MIN32(x, 32767);
   return (opus_int16)float2int(x);
}

static OPUS_INLINE opus_int32 FLOAT2INT24(float x)
{
   x = x*(CELT_SIG_SCALE*256.f);
   x = MAX32(x, -16777216);
   x = MIN32(x, 16777216);
   return float2int(x);
}
#ifdef FIXED_POINT
static OPUS_INLINE opus_int32 FLOAT2SIG(float x)
{
   x = x*((opus_int32)32768<<SIG_SHIFT);
   x = MAX32(x, -(65536<<SIG_SHIFT));
   x = MIN32(x, 65536<<SIG_SHIFT);
   return float2int(x);
}
#endif
#endif /* DISABLE_FLOAT_API */

#endif /* FLOAT_CAST_H */
```


`celt/mathops.h:150-240`

```c
    * Please note that ATAN2_COEFF_A01 is fixed to 1.0f. */
   #define ATAN2_COEFF_A03 -3.3331659436225891113281250000e-01f
   #define ATAN2_COEFF_A05 1.99627041816711425781250000000e-01f
   #define ATAN2_COEFF_A07 -1.3976582884788513183593750000e-01f
   #define ATAN2_COEFF_A09 9.79423448443412780761718750000e-02f
   #define ATAN2_COEFF_A11 -5.7773590087890625000000000000e-02f
   #define ATAN2_COEFF_A13 2.30401363223791122436523437500e-02f
   #define ATAN2_COEFF_A15 -4.3554059229791164398193359375e-03f
   return ATAN2_2_OVER_PI * (x + x * x_sq * (ATAN2_COEFF_A03
                + x_sq * (ATAN2_COEFF_A05
                + x_sq * (ATAN2_COEFF_A07
                + x_sq * (ATAN2_COEFF_A09
                + x_sq * (ATAN2_COEFF_A11
                + x_sq * (ATAN2_COEFF_A13
                + x_sq * (ATAN2_COEFF_A15))))))));
}

/* Calculates the arctangent of y/x, returning an approximate value in radians.
 * Please refer to the linked wiki page (https://en.wikipedia.org/wiki/Atan2)
 * to learn how atan2 results are computed. */
static OPUS_INLINE float celt_atan2p_norm(float y, float x)
{
   celt_sig_assert(x>=0 && y>=0);

   /* For very small values, we don't care about the answer. */
   if ((x*x + y*y) < 1e-18f)
   {
      return 0;
   }

   if (y < x)
   {
      return celt_atan_norm(y / x);
   } else {
      return 1.f - celt_atan_norm(x / y);
   }
}
#endif

#if !defined(FIXED_POINT) || defined(ENABLE_QEXT)
/* Computes estimated cosine values for (PI/2 * x) using only terms with even
 * exponents. */
static OPUS_INLINE float celt_cos_norm2(float x)
{
   float x_norm_sq;
   int output_sign;
   /* Restrict x to [-1, 3]. */
   x -= 4*floor(.25*(x+1));
   /* Negative sign for [1, 3]. */
   output_sign = 1 - 2*(x>1);
   /* Restrict to [-1, 1]. */
   x -= 2*(x>1);

   /* The cosine function, cos(x), has a Taylor series representation consisting
    * exclusively of even-powered polynomial terms. */
   x_norm_sq = x * x;

   /* Polynomial coefficients approximated in the [0, 1] range using only terms
    * with even exponents.
    * Lolremez command: lolremez --degree 4 --range 0:1 "cos(sqrt(x)*pi*0.5)" */
   #define COS_COEFF_A0 9.999999403953552246093750000000e-01f
   #define COS_COEFF_A2 -1.233698248863220214843750000000000f
   #define COS_COEFF_A4 2.536507546901702880859375000000e-01f
   #define COS_COEFF_A6 -2.08106283098459243774414062500e-02f
   #define COS_COEFF_A8 8.581906440667808055877685546875e-04f
   return output_sign * (COS_COEFF_A0 + x_norm_sq * (COS_COEFF_A2 +
                               x_norm_sq * (COS_COEFF_A4 +
                               x_norm_sq * (COS_COEFF_A6 +
                               x_norm_sq * (COS_COEFF_A8)))));
}

#endif

#ifndef FIXED_POINT

#define celt_sqrt(x) ((float)sqrt(x))
#define celt_sqrt32(x) ((float)sqrt(x))
#define celt_rsqrt(x) (1.f/celt_sqrt(x))
#define celt_rsqrt_norm(x) (celt_rsqrt(x))
#define celt_rsqrt_norm32(x) (celt_rsqrt(x))
#define celt_cos_norm(x) ((float)cos((.5f*PI)*(x)))
#define celt_rcp(x) (1.f/(x))
#define celt_div(a,b) ((a)/(b))
#define frac_div32(a,b) ((float)(a)/(b))
#define frac_div32_q29(a,b) frac_div32(a,b)

#ifdef FLOAT_APPROX
/* Calculates the base-2 logarithm (log2(x)) of a number. It is designed for
 * systems using radix-2 floating-point representation, with the exponent
 * located at bits 23 to 30 and an offset of 127. Note that special cases like
 * denormalized numbers, positive/negative infinity, and NaN are not handled.
```


`celt/arch.h:258-300`

```c
typedef float opus_val16;
typedef float opus_val32;
typedef float opus_val64;

typedef float celt_sig;
typedef float celt_norm;
typedef float celt_ener;
typedef float celt_glog;

typedef float opus_res;
typedef float celt_coef;

#ifdef FLOAT_APPROX
/* This code should reliably detect NaN/inf even when -ffast-math is used.
   Assumes IEEE 754 format. */
static OPUS_INLINE int celt_isnan(float x)
{
   union {float f; opus_uint32 i;} in;
   in.f = x;
   return ((in.i>>23)&0xFF)==0xFF && (in.i&0x007FFFFF)!=0;
}
#else
#ifdef __FAST_MATH__
#error Cannot build libopus with -ffast-math unless FLOAT_APPROX is defined. This could result in crashes on extreme (e.g. NaN) input
#endif
#define celt_isnan(x) ((x)!=(x))
#endif

#define Q15ONE 1.0f
#define Q31ONE 1.0f
#define COEF_ONE 1.0f
#define COEF2VAL16(x) (x)

#define NORM_SCALING 1.f

#define EPSILON 1e-15f
#define VERY_SMALL 1e-30f
#define VERY_LARGE16 1e15f
#define Q15_ONE ((opus_val16)1.f)

/* This appears to be the same speed as C99's fabsf() but it's more portable. */
#define ABS16(x) ((float)fabs(x))
#define ABS32(x) ((float)fabs(x))
```


### 7.8 `kiss_fft`: twiddles, butterfly order and the fixed-point clamp

The FFT is used by (a) the MDCT in CELT, (b) the pitch prefilter's
`_celt_autocorr` (shared with SILK via `silk_autocorr`), and (c) `opus_fft_alloc`
in `celt/mdct.c`. Hazards:

* `_kiss_fft_guts.h` defines the arithmetic: `S_MUL`, `C_MUL`, `C_MULBYSCALAR`,
  `C_ADD`, `C_SUB`, `C_ADDTO`, `C_SUBFROM`, `HALF_OF`, and — under
  `#ifdef FIXED_POINT` — `FRACBITS`, `SAMPPROD`, `SAMP_MAX`, `SAMP_MIN`,
  `CHECK_OVERFLOW`. **The fixed-point butterfly clamps to `SAMP_MAX`/`SAMP_MIN`
  (`32767`/`-32768` before shifting)**; removing that clamp changes results on
  loud input.
* `kf_bfly2/3/4/5` are generated by `kf_factor`'s factorisation of `nfft`
  (`4*4*4*2` for 128, etc.). The **order in which factors are consumed** and the
  `m`/`fstride`/`fout`/`fout_end` pointer arithmetic determine the permutation.
  Do not replace with a generic FFT; the twiddle tables are indexed by this
  order.
* `kf_work`'s recursion and the `KISS_FFT_USE_ALLOCA` scratch path.
* The twiddle tables are **data**: `fft_twiddles48000_960[480]`,
  `fft_twiddles96000_1920[960]`, `fft_bitrev{480,240,120,60,960}` in
  `celt/static_modes_fixed.h`, and the `kiss_twiddle_cpx` layout is
  `{kiss_twiddle_scalar r; kiss_twiddle_scalar i;}` (2 × i16 in the fixed build
  with `ENABLE_QEXT` off? no — `kiss_twiddle_scalar` is `opus_int16` normally and
  `opus_int32` under `ENABLE_QEXT`; check `celt/kiss_fft.h`).
* `opus_fft_impl`'s `downshift` argument (`ARG_FIXED(int downshift)`) applies
  `SHR32` per butterfly; the fixed build's `st->shift`/`scale_shift` must be
  reproduced exactly (`kiss_fft_state` in §5.7).


`celt/_kiss_fft_guts.h:30-188`

```c
#define MAX(a,b) ((a)>(b) ? (a):(b))

/* kiss_fft.h
   defines kiss_fft_scalar as either short or a float type
   and defines
   typedef struct { kiss_fft_scalar r; kiss_fft_scalar i; }kiss_fft_cpx; */
#include "kiss_fft.h"

/*
  Explanation of macros dealing with complex math:

   C_MUL(m,a,b)         : m = a*b
   C_FIXDIV( c , div )  : if a fixed point impl., c /= div. noop otherwise
   C_SUB( res, a,b)     : res = a - b
   C_SUBFROM( res , a)  : res -= a
   C_ADDTO( res , a)    : res += a
 * */
#ifdef FIXED_POINT
#include "arch.h"


#define SAMP_MAX 2147483647
#define TWID_MAX 32767
#define TRIG_UPSCALE 1

#define SAMP_MIN -SAMP_MAX

#ifdef ENABLE_QEXT
#   define S_MUL(a,b) MULT32_32_P31_ovflw(b, a)
#   define S_MUL2(a,b) MULT32_32_P31_ovflw(b, a)
#else
#   define S_MUL(a,b) MULT16_32_Q15(b, a)
#   define S_MUL2(a,b) MULT16_32_Q16(b, a)
#endif

#   define C_MUL(m,a,b) \
      do{ (m).r = SUB32_ovflw(S_MUL((a).r,(b).r) , S_MUL((a).i,(b).i)); \
          (m).i = ADD32_ovflw(S_MUL((a).r,(b).i) , S_MUL((a).i,(b).r)); }while(0)

#   define C_MULC(m,a,b) \
      do{ (m).r = ADD32_ovflw(S_MUL((a).r,(b).r) , S_MUL((a).i,(b).i)); \
          (m).i = SUB32_ovflw(S_MUL((a).i,(b).r) , S_MUL((a).r,(b).i)); }while(0)

#   define C_MULBYSCALAR( c, s ) \
      do{ (c).r =  S_MUL( (c).r , s ) ;\
          (c).i =  S_MUL( (c).i , s ) ; }while(0)

#   define DIVSCALAR(x,k) \
        (x) = S_MUL(  x, (TWID_MAX-((k)>>1))/(k)+1 )

#   define C_FIXDIV(c,div) \
        do {    DIVSCALAR( (c).r , div);  \
                DIVSCALAR( (c).i  , div); }while (0)

#define  C_ADD( res, a,b)\
    do {(res).r=ADD32_ovflw((a).r,(b).r);  (res).i=ADD32_ovflw((a).i,(b).i); \
    }while(0)
#define  C_SUB( res, a,b)\
    do {(res).r=SUB32_ovflw((a).r,(b).r);  (res).i=SUB32_ovflw((a).i,(b).i); \
    }while(0)
#define C_ADDTO( res , a)\
    do {(res).r = ADD32_ovflw((res).r, (a).r);  (res).i = ADD32_ovflw((res).i,(a).i);\
    }while(0)

#define C_SUBFROM( res , a)\
    do {(res).r = ADD32_ovflw((res).r,(a).r);  (res).i = SUB32_ovflw((res).i,(a).i); \
    }while(0)

#if defined(OPUS_ARM_INLINE_ASM)
#include "arm/kiss_fft_armv4.h"
#endif

#if defined(OPUS_ARM_INLINE_EDSP)
#include "arm/kiss_fft_armv5e.h"
#endif
#if defined(__mips)
#include "mips/kiss_fft_mipsr1.h"
#endif

#else  /* not FIXED_POINT*/

#   define S_MUL(a,b) ( (a)*(b) )
#   define S_MUL2(a,b) ( (a)*(b) )
#define C_MUL(m,a,b) \
    do{ (m).r = (a).r*(b).r - (a).i*(b).i;\
        (m).i = (a).r*(b).i + (a).i*(b).r; }while(0)
#define C_MULC(m,a,b) \
    do{ (m).r = (a).r*(b).r + (a).i*(b).i;\
        (m).i = (a).i*(b).r - (a).r*(b).i; }while(0)

#define C_MUL4(m,a,b) C_MUL(m,a,b)

#   define C_FIXDIV(c,div) /* NOOP */
#   define C_MULBYSCALAR( c, s ) \
    do{ (c).r *= (s);\
        (c).i *= (s); }while(0)
#endif

#ifndef CHECK_OVERFLOW_OP
#  define CHECK_OVERFLOW_OP(a,op,b) /* noop */
#endif

#ifndef C_ADD
#define  C_ADD( res, a,b)\
    do { \
            CHECK_OVERFLOW_OP((a).r,+,(b).r)\
            CHECK_OVERFLOW_OP((a).i,+,(b).i)\
            (res).r=(a).r+(b).r;  (res).i=(a).i+(b).i; \
    }while(0)
#define  C_SUB( res, a,b)\
    do { \
            CHECK_OVERFLOW_OP((a).r,-,(b).r)\
            CHECK_OVERFLOW_OP((a).i,-,(b).i)\
            (res).r=(a).r-(b).r;  (res).i=(a).i-(b).i; \
    }while(0)
#define C_ADDTO( res , a)\
    do { \
            CHECK_OVERFLOW_OP((res).r,+,(a).r)\
            CHECK_OVERFLOW_OP((res).i,+,(a).i)\
            (res).r += (a).r;  (res).i += (a).i;\
    }while(0)

#define C_SUBFROM( res , a)\
    do {\
            CHECK_OVERFLOW_OP((res).r,-,(a).r)\
            CHECK_OVERFLOW_OP((res).i,-,(a).i)\
            (res).r -= (a).r;  (res).i -= (a).i; \
    }while(0)
#endif /* C_ADD defined */

#ifdef FIXED_POINT
/*#  define KISS_FFT_COS(phase)  TRIG_UPSCALE*floor(MIN(32767,MAX(-32767,.5+32768 * cos (phase))))
#  define KISS_FFT_SIN(phase)  TRIG_UPSCALE*floor(MIN(32767,MAX(-32767,.5+32768 * sin (phase))))*/
#  define KISS_FFT_COS(phase)  floor(.5+TWID_MAX*cos (phase))
#  define KISS_FFT_SIN(phase)  floor(.5+TWID_MAX*sin (phase))
#  define HALF_OF(x) ((x)>>1)
#elif defined(USE_SIMD)
#  define KISS_FFT_COS(phase) _mm_set1_ps( cos(phase) )
#  define KISS_FFT_SIN(phase) _mm_set1_ps( sin(phase) )
#  define HALF_OF(x) ((x)*_mm_set1_ps(.5f))
#else
#  define KISS_FFT_COS(phase) (kiss_fft_scalar) cos(phase)
#  define KISS_FFT_SIN(phase) (kiss_fft_scalar) sin(phase)
#  define HALF_OF(x) ((x)*.5f)
#endif

#define  kf_cexp(x,phase) \
        do{ \
                (x)->r = KISS_FFT_COS(phase);\
                (x)->i = KISS_FFT_SIN(phase);\
        }while(0)

#define  kf_cexp2(x,phase) \
   do{ \
      (x)->r = TRIG_UPSCALE*celt_cos_norm((phase));\
      (x)->i = TRIG_UPSCALE*celt_cos_norm((phase)-32768);\
}while(0)

#endif /* KISS_FFT_GUTS_H */
```


`celt/kiss_fft.c:36-330`

```c
#endif

#include "_kiss_fft_guts.h"
#include "arch.h"
#include "os_support.h"
#include "mathops.h"
#include "stack_alloc.h"

#ifndef M_PI
#define M_PI 3.141592653
#endif

/* The guts header contains all the multiplication and addition macros that are defined for
   complex numbers.  It also declares the kf_ internal functions.
*/

static void kf_bfly2(
                     kiss_fft_cpx * Fout,
                     int m,
                     int N
                    )
{
   kiss_fft_cpx * Fout2;
   int i;
   (void)m;
#ifdef CUSTOM_MODES
   if (m==1)
   {
      celt_assert(m==1);
      for (i=0;i<N;i++)
      {
         kiss_fft_cpx t;
         Fout2 = Fout + 1;
         t = *Fout2;
         C_SUB( *Fout2 ,  *Fout , t );
         C_ADDTO( *Fout ,  t );
         Fout += 2;
      }
   } else
#endif
   {
      celt_coef tw;
      tw = QCONST32(0.7071067812f, COEF_SHIFT-1);
      /* We know that m==4 here because the radix-2 is just after a radix-4 */
      celt_assert(m==4);
      for (i=0;i<N;i++)
      {
         kiss_fft_cpx t;
         Fout2 = Fout + 4;
         t = Fout2[0];
         C_SUB( Fout2[0] ,  Fout[0] , t );
         C_ADDTO( Fout[0] ,  t );

         t.r = S_MUL(ADD32_ovflw(Fout2[1].r, Fout2[1].i), tw);
         t.i = S_MUL(SUB32_ovflw(Fout2[1].i, Fout2[1].r), tw);
         C_SUB( Fout2[1] ,  Fout[1] , t );
         C_ADDTO( Fout[1] ,  t );

         t.r = Fout2[2].i;
         t.i = NEG32_ovflw(Fout2[2].r);
         C_SUB( Fout2[2] ,  Fout[2] , t );
         C_ADDTO( Fout[2] ,  t );

         t.r = S_MUL(SUB32_ovflw(Fout2[3].i, Fout2[3].r), tw);
         t.i = S_MUL(NEG32_ovflw(ADD32_ovflw(Fout2[3].i, Fout2[3].r)), tw);
         C_SUB( Fout2[3] ,  Fout[3] , t );
         C_ADDTO( Fout[3] ,  t );
         Fout += 8;
      }
   }
}

static void kf_bfly4(
                     kiss_fft_cpx * Fout,
                     const size_t fstride,
                     const kiss_fft_state *st,
                     int m,
                     int N,
                     int mm
                    )
{
   int i;

   if (m==1)
   {
      /* Degenerate case where all the twiddles are 1. */
      for (i=0;i<N;i++)
      {
         kiss_fft_cpx scratch0, scratch1;

         C_SUB( scratch0 , *Fout, Fout[2] );
         C_ADDTO(*Fout, Fout[2]);
         C_ADD( scratch1 , Fout[1] , Fout[3] );
         C_SUB( Fout[2], *Fout, scratch1 );
         C_ADDTO( *Fout , scratch1 );
         C_SUB( scratch1 , Fout[1] , Fout[3] );

         Fout[1].r = ADD32_ovflw(scratch0.r, scratch1.i);
         Fout[1].i = SUB32_ovflw(scratch0.i, scratch1.r);
         Fout[3].r = SUB32_ovflw(scratch0.r, scratch1.i);
         Fout[3].i = ADD32_ovflw(scratch0.i, scratch1.r);
         Fout+=4;
      }
   } else {
      int j;
      kiss_fft_cpx scratch[6];
      const kiss_twiddle_cpx *tw1,*tw2,*tw3;
      const int m2=2*m;
      const int m3=3*m;
      kiss_fft_cpx * Fout_beg = Fout;
      for (i=0;i<N;i++)
      {
         Fout = Fout_beg + i*mm;
         tw3 = tw2 = tw1 = st->twiddles;
         /* m is guaranteed to be a multiple of 4. */
         for (j=0;j<m;j++)
         {
            C_MUL(scratch[0],Fout[m] , *tw1 );
            C_MUL(scratch[1],Fout[m2] , *tw2 );
            C_MUL(scratch[2],Fout[m3] , *tw3 );

            C_SUB( scratch[5] , *Fout, scratch[1] );
            C_ADDTO(*Fout, scratch[1]);
            C_ADD( scratch[3] , scratch[0] , scratch[2] );
            C_SUB( scratch[4] , scratch[0] , scratch[2] );
            C_SUB( Fout[m2], *Fout, scratch[3] );
            tw1 += fstride;
            tw2 += fstride*2;
            tw3 += fstride*3;
            C_ADDTO( *Fout , scratch[3] );

            Fout[m].r = ADD32_ovflw(scratch[5].r, scratch[4].i);
            Fout[m].i = SUB32_ovflw(scratch[5].i, scratch[4].r);
            Fout[m3].r = SUB32_ovflw(scratch[5].r, scratch[4].i);
            Fout[m3].i = ADD32_ovflw(scratch[5].i, scratch[4].r);
            ++Fout;
         }
      }
   }
}


#ifndef RADIX_TWO_ONLY

static void kf_bfly3(
                     kiss_fft_cpx * Fout,
                     const size_t fstride,
                     const kiss_fft_state *st,
                     int m,
                     int N,
                     int mm
                    )
{
   int i;
   size_t k;
   const size_t m2 = 2*m;
   const kiss_twiddle_cpx *tw1,*tw2;
   kiss_fft_cpx scratch[5];
   kiss_twiddle_cpx epi3;

   kiss_fft_cpx * Fout_beg = Fout;
#ifdef FIXED_POINT
   /*epi3.r = -16384;*/ /* Unused */
   epi3.i = -QCONST32(0.86602540f, COEF_SHIFT-1);
#else
   epi3 = st->twiddles[fstride*m];
#endif
   for (i=0;i<N;i++)
   {
      Fout = Fout_beg + i*mm;
      tw1=tw2=st->twiddles;
      /* For non-custom modes, m is guaranteed to be a multiple of 4. */
      k=m;
      do {

         C_MUL(scratch[1],Fout[m] , *tw1);
         C_MUL(scratch[2],Fout[m2] , *tw2);

         C_ADD(scratch[3],scratch[1],scratch[2]);
         C_SUB(scratch[0],scratch[1],scratch[2]);
         tw1 += fstride;
         tw2 += fstride*2;

         Fout[m].r = SUB32_ovflw(Fout->r, HALF_OF(scratch[3].r));
         Fout[m].i = SUB32_ovflw(Fout->i, HALF_OF(scratch[3].i));

         C_MULBYSCALAR( scratch[0] , epi3.i );

         C_ADDTO(*Fout,scratch[3]);

         Fout[m2].r = ADD32_ovflw(Fout[m].r, scratch[0].i);
         Fout[m2].i = SUB32_ovflw(Fout[m].i, scratch[0].r);

         Fout[m].r = SUB32_ovflw(Fout[m].r, scratch[0].i);
         Fout[m].i = ADD32_ovflw(Fout[m].i, scratch[0].r);

         ++Fout;
      } while(--k);
   }
}


#ifndef OVERRIDE_kf_bfly5
static void kf_bfly5(
                     kiss_fft_cpx * Fout,
                     const size_t fstride,
                     const kiss_fft_state *st,
                     int m,
                     int N,
                     int mm
                    )
{
   kiss_fft_cpx *Fout0,*Fout1,*Fout2,*Fout3,*Fout4;
   int i, u;
   kiss_fft_cpx scratch[13];
   const kiss_twiddle_cpx *tw;
   kiss_twiddle_cpx ya,yb;
   kiss_fft_cpx * Fout_beg = Fout;

#ifdef FIXED_POINT
   ya.r = QCONST32(0.30901699f, COEF_SHIFT-1);
   ya.i = -QCONST32(0.95105652f, COEF_SHIFT-1);
   yb.r = -QCONST32(0.80901699f, COEF_SHIFT-1);
   yb.i = -QCONST32(0.58778525f, COEF_SHIFT-1);
#else
   ya = st->twiddles[fstride*m];
   yb = st->twiddles[fstride*2*m];
#endif
   tw=st->twiddles;

   for (i=0;i<N;i++)
   {
      Fout = Fout_beg + i*mm;
      Fout0=Fout;
      Fout1=Fout0+m;
      Fout2=Fout0+2*m;
      Fout3=Fout0+3*m;
      Fout4=Fout0+4*m;

      /* For non-custom modes, m is guaranteed to be a multiple of 4. */
      for ( u=0; u<m; ++u ) {
         scratch[0] = *Fout0;

         C_MUL(scratch[1] ,*Fout1, tw[u*fstride]);
         C_MUL(scratch[2] ,*Fout2, tw[2*u*fstride]);
         C_MUL(scratch[3] ,*Fout3, tw[3*u*fstride]);
         C_MUL(scratch[4] ,*Fout4, tw[4*u*fstride]);

         C_ADD( scratch[7],scratch[1],scratch[4]);
         C_SUB( scratch[10],scratch[1],scratch[4]);
         C_ADD( scratch[8],scratch[2],scratch[3]);
         C_SUB( scratch[9],scratch[2],scratch[3]);

         Fout0->r = ADD32_ovflw(Fout0->r, ADD32_ovflw(scratch[7].r, scratch[8].r));
         Fout0->i = ADD32_ovflw(Fout0->i, ADD32_ovflw(scratch[7].i, scratch[8].i));

         scratch[5].r = ADD32_ovflw(scratch[0].r, ADD32_ovflw(S_MUL(scratch[7].r,ya.r), S_MUL(scratch[8].r,yb.r)));
         scratch[5].i = ADD32_ovflw(scratch[0].i, ADD32_ovflw(S_MUL(scratch[7].i,ya.r), S_MUL(scratch[8].i,yb.r)));

         scratch[6].r =  ADD32_ovflw(S_MUL(scratch[10].i,ya.i), S_MUL(scratch[9].i,yb.i));
         scratch[6].i = NEG32_ovflw(ADD32_ovflw(S_MUL(scratch[10].r,ya.i), S_MUL(scratch[9].r,yb.i)));

         C_SUB(*Fout1,scratch[5],scratch[6]);
         C_ADD(*Fout4,scratch[5],scratch[6]);

         scratch[11].r = ADD32_ovflw(scratch[0].r, ADD32_ovflw(S_MUL(scratch[7].r,yb.r), S_MUL(scratch[8].r,ya.r)));
         scratch[11].i = ADD32_ovflw(scratch[0].i, ADD32_ovflw(S_MUL(scratch[7].i,yb.r), S_MUL(scratch[8].i,ya.r)));
         scratch[12].r = SUB32_ovflw(S_MUL(scratch[9].i,ya.i), S_MUL(scratch[10].i,yb.i));
         scratch[12].i = SUB32_ovflw(S_MUL(scratch[10].r,yb.i), S_MUL(scratch[9].r,ya.i));

         C_ADD(*Fout2,scratch[11],scratch[12]);
         C_SUB(*Fout3,scratch[11],scratch[12]);

         ++Fout0;++Fout1;++Fout2;++Fout3;++Fout4;
      }
   }
}
#endif /* OVERRIDE_kf_bfly5 */


#endif


#ifdef CUSTOM_MODES

static
void compute_bitrev_table(
         int Fout,
         opus_int16 *f,
         const size_t fstride,
         int in_stride,
         opus_int16 * factors,
         const kiss_fft_state *st
            )
{
```


### 7.9 MDCT rotation and windowing

`clt_mdct_forward_c` / `clt_mdct_backward_c` (`celt/mdct.c`) are the only MDCT
implementations; both use `mdct_lookup.trig` (the mode's twiddle table) and the
`kfft[]` sub-FFTs. Hazards:

* The **pre/post rotation** uses `S_MUL` with `l->trig` strides and the
  `bitrev` permutation of the pre-rotated buffer; the `mdct_lookup.n`,
  `maxshift` and `kfft[shift]` selection must match.
* `mode->mdct.trig` points into `mdct_twiddles960` (or `mdct_twiddles1920` under
  `ENABLE_QEXT`) at an offset computed by `clt_mdct_init`; the offset arithmetic
  is `trig += ...` with `N>>maxshift` steps — reproduce it rather than re-deriving
  twiddles.
* In the fixed build, the rotation MACs are `MAC16_32_Q15`/`MULT16_32_Q15`,
  which round; see §7.2.
* The windowing multiply (`window[i]`) is applied with
  `MULT16_16_Q15`-class macros: the window table's Q format
  (`celt_coef` = `opus_val16` normally, `opus_val32` under `ENABLE_QEXT`) and the
  multiply macro must be chosen together.


`celt/mdct.c:122-390`

```c
void clt_mdct_forward_c(const mdct_lookup *l, kiss_fft_scalar *in, kiss_fft_scalar * OPUS_RESTRICT out,
      const celt_coef *window, int overlap, int shift, int stride, int arch)
{
   int i;
   int N, N2, N4;
   VARDECL(kiss_fft_scalar, f);
   VARDECL(kiss_fft_cpx, f2);
   const kiss_fft_state *st = l->kfft[shift];
   const kiss_twiddle_scalar *trig;
   celt_coef scale;
#ifdef FIXED_POINT
   /* Allows us to scale with MULT16_32_Q16(), which is faster than
      MULT16_32_Q15() on ARM. */
   int scale_shift = st->scale_shift-1;
   int headroom;
#endif
   SAVE_STACK;
   (void)arch;
   scale = st->scale;

   N = l->n;
   trig = l->trig;
   for (i=0;i<shift;i++)
   {
      N >>= 1;
      trig += N;
   }
   N2 = N>>1;
   N4 = N>>2;

   ALLOC(f, N2, kiss_fft_scalar);
   ALLOC(f2, N4, kiss_fft_cpx);

   /* Consider the input to be composed of four blocks: [a, b, c, d] */
   /* Window, shuffle, fold */
   {
      /* Temp pointers to make it really clear to the compiler what we're doing */
      const kiss_fft_scalar * OPUS_RESTRICT xp1 = in+(overlap>>1);
      const kiss_fft_scalar * OPUS_RESTRICT xp2 = in+N2-1+(overlap>>1);
      kiss_fft_scalar * OPUS_RESTRICT yp = f;
      const celt_coef * OPUS_RESTRICT wp1 = window+(overlap>>1);
      const celt_coef * OPUS_RESTRICT wp2 = window+(overlap>>1)-1;
      for(i=0;i<((overlap+3)>>2);i++)
      {
         /* Real part arranged as -d-cR, Imag part arranged as -b+aR*/
         *yp++ = S_MUL(xp1[N2], *wp2) + S_MUL(*xp2, *wp1);
         *yp++ = S_MUL(*xp1, *wp1)    - S_MUL(xp2[-N2], *wp2);
         xp1+=2;
         xp2-=2;
         wp1+=2;
         wp2-=2;
      }
      wp1 = window;
      wp2 = window+overlap-1;
      for(;i<N4-((overlap+3)>>2);i++)
      {
         /* Real part arranged as a-bR, Imag part arranged as -c-dR */
         *yp++ = *xp2;
         *yp++ = *xp1;
         xp1+=2;
         xp2-=2;
      }
      for(;i<N4;i++)
      {
         /* Real part arranged as a-bR, Imag part arranged as -c-dR */
         *yp++ =  -S_MUL(xp1[-N2], *wp1) + S_MUL(*xp2, *wp2);
         *yp++ = S_MUL(*xp1, *wp2)     + S_MUL(xp2[N2], *wp1);
         xp1+=2;
         xp2-=2;
         wp1+=2;
         wp2-=2;
      }
   }
   /* Pre-rotation */
   {
      kiss_fft_scalar * OPUS_RESTRICT yp = f;
      const kiss_twiddle_scalar *t = &trig[0];
#ifdef FIXED_POINT
      opus_val32 maxval=1;
#endif
      for(i=0;i<N4;i++)
      {
         kiss_fft_cpx yc;
         kiss_twiddle_scalar t0, t1;
         kiss_fft_scalar re, im, yr, yi;
         t0 = t[i];
         t1 = t[N4+i];
         re = *yp++;
         im = *yp++;
         yr = S_MUL(re,t0)  -  S_MUL(im,t1);
         yi = S_MUL(im,t0)  +  S_MUL(re,t1);
         /* For QEXT, it's best to scale before the FFT, but otherwise it's best to scale after.
            For floating-point it doesn't matter. */
#ifdef ENABLE_QEXT
         yc.r = yr;
         yc.i = yi;
#else
         yc.r = S_MUL2(yr, scale);
         yc.i = S_MUL2(yi, scale);
#endif
#ifdef FIXED_POINT
         maxval = MAX32(maxval, MAX32(ABS32(yc.r), ABS32(yc.i)));
#endif
         f2[st->bitrev[i]] = yc;
      }
#ifdef FIXED_POINT
      headroom = IMAX(0, IMIN(scale_shift, 28-celt_ilog2(maxval)));
#endif
   }

   /* N/4 complex FFT, does not downscale anymore */
   opus_fft_impl(st, f2 ARG_FIXED(scale_shift-headroom));

   /* Post-rotate */
   {
      /* Temp pointers to make it really clear to the compiler what we're doing */
      const kiss_fft_cpx * OPUS_RESTRICT fp = f2;
      kiss_fft_scalar * OPUS_RESTRICT yp1 = out;
      kiss_fft_scalar * OPUS_RESTRICT yp2 = out+stride*(N2-1);
      const kiss_twiddle_scalar *t = &trig[0];
      /* Temp pointers to make it really clear to the compiler what we're doing */
      for(i=0;i<N4;i++)
      {
         kiss_fft_scalar yr, yi;
         kiss_fft_scalar t0, t1;
#ifdef ENABLE_QEXT
         t0 = S_MUL2(t[i], scale);
         t1 = S_MUL2(t[N4+i], scale);
#else
         t0 = t[i];
         t1 = t[N4+i];
#endif
         yr = PSHR32(S_MUL(fp->i,t1) - S_MUL(fp->r,t0), headroom);
         yi = PSHR32(S_MUL(fp->r,t1) + S_MUL(fp->i,t0), headroom);
         *yp1 = yr;
         *yp2 = yi;
         fp++;
         yp1 += 2*stride;
         yp2 -= 2*stride;
      }
   }
   RESTORE_STACK;
}
#endif /* OVERRIDE_clt_mdct_forward */

#ifndef OVERRIDE_clt_mdct_backward
void clt_mdct_backward_c(const mdct_lookup *l, kiss_fft_scalar *in, kiss_fft_scalar * OPUS_RESTRICT out,
      const celt_coef * OPUS_RESTRICT window, int overlap, int shift, int stride, int arch)
{
   int i;
   int N, N2, N4;
   const kiss_twiddle_scalar *trig;
#ifdef FIXED_POINT
   int pre_shift, post_shift, fft_shift;
#endif
   (void) arch;

   N = l->n;
   trig = l->trig;
   for (i=0;i<shift;i++)
   {
      N >>= 1;
      trig += N;
   }
   N2 = N>>1;
   N4 = N>>2;

#ifdef FIXED_POINT
   {
      opus_val32 sumval=N2;
      opus_val32 maxval=0;
      for (i=0;i<N2;i++) {
         maxval = MAX32(maxval, ABS32(in[i*stride]));
         sumval = ADD32_ovflw(sumval, ABS32(SHR32(in[i*stride],11)));
      }
      pre_shift = IMAX(0, 29-celt_zlog2(1+maxval));
      /* Worst-case where all the energy goes to a single sample. */
      post_shift = IMAX(0, 19-celt_ilog2(ABS32(sumval)));
      post_shift = IMIN(post_shift, pre_shift);
      fft_shift = pre_shift - post_shift;
   }
#endif
   /* Pre-rotate */
   {
      /* Temp pointers to make it really clear to the compiler what we're doing */
      const kiss_fft_scalar * OPUS_RESTRICT xp1 = in;
      const kiss_fft_scalar * OPUS_RESTRICT xp2 = in+stride*(N2-1);
      kiss_fft_scalar * OPUS_RESTRICT yp = out+(overlap>>1);
      const kiss_twiddle_scalar * OPUS_RESTRICT t = &trig[0];
      const opus_int16 * OPUS_RESTRICT bitrev = l->kfft[shift]->bitrev;
      for(i=0;i<N4;i++)
      {
         int rev;
         kiss_fft_scalar yr, yi;
         opus_val32 x1, x2;
         rev = *bitrev++;
         x1 = SHL32_ovflw(*xp1, pre_shift);
         x2 = SHL32_ovflw(*xp2, pre_shift);
         yr = ADD32_ovflw(S_MUL(x2, t[i]), S_MUL(x1, t[N4+i]));
         yi = SUB32_ovflw(S_MUL(x1, t[i]), S_MUL(x2, t[N4+i]));
         /* We swap real and imag because we use an FFT instead of an IFFT. */
         yp[2*rev+1] = yr;
         yp[2*rev] = yi;
         /* Storing the pre-rotation directly in the bitrev order. */
         xp1+=2*stride;
         xp2-=2*stride;
      }
   }

   opus_fft_impl(l->kfft[shift], (kiss_fft_cpx*)(out+(overlap>>1)) ARG_FIXED(fft_shift));

   /* Post-rotate and de-shuffle from both ends of the buffer at once to make
      it in-place. */
   {
      kiss_fft_scalar * yp0 = out+(overlap>>1);
      kiss_fft_scalar * yp1 = out+(overlap>>1)+N2-2;
      const kiss_twiddle_scalar *t = &trig[0];
      /* Loop to (N4+1)>>1 to handle odd N4. When N4 is odd, the
         middle pair will be computed twice. */
      for(i=0;i<(N4+1)>>1;i++)
      {
         kiss_fft_scalar re, im, yr, yi;
         kiss_twiddle_scalar t0, t1;
         /* We swap real and imag because we're using an FFT instead of an IFFT. */
         re = yp0[1];
         im = yp0[0];
         t0 = t[i];
         t1 = t[N4+i];
         /* We'd scale up by 2 here, but instead it's done when mixing the windows */
         yr = PSHR32_ovflw(ADD32_ovflw(S_MUL(re,t0), S_MUL(im,t1)), post_shift);
         yi = PSHR32_ovflw(SUB32_ovflw(S_MUL(re,t1), S_MUL(im,t0)), post_shift);
         /* We swap real and imag because we're using an FFT instead of an IFFT. */
         re = yp1[1];
         im = yp1[0];
         yp0[0] = yr;
         yp1[1] = yi;

         t0 = t[(N4-i-1)];
         t1 = t[(N2-i-1)];
         /* We'd scale up by 2 here, but instead it's done when mixing the windows */
         yr = PSHR32_ovflw(ADD32_ovflw(S_MUL(re,t0), S_MUL(im,t1)), post_shift);
         yi = PSHR32_ovflw(SUB32_ovflw(S_MUL(re,t1), S_MUL(im,t0)), post_shift);
         yp1[0] = yr;
         yp0[1] = yi;
         yp0 += 2;
         yp1 -= 2;
      }
   }

   /* Mirror on both sides for TDAC */
   {
      kiss_fft_scalar * OPUS_RESTRICT xp1 = out+overlap-1;
      kiss_fft_scalar * OPUS_RESTRICT yp1 = out;
      const celt_coef * OPUS_RESTRICT wp1 = window;
      const celt_coef * OPUS_RESTRICT wp2 = window+overlap-1;

      for(i = 0; i < overlap/2; i++)
      {
         kiss_fft_scalar x1, x2;
         x1 = *xp1;
         x2 = *yp1;
         *yp1++ = SUB32_ovflw(S_MUL(x2, *wp2), S_MUL(x1, *wp1));
         *xp1-- = ADD32_ovflw(S_MUL(x2, *wp1), S_MUL(x1, *wp2));
         wp1++;
         wp2--;
      }
   }
}
#endif /* OVERRIDE_clt_mdct_backward */
```


### 7.10 Signed shifts, overflow and other UB to isolate

A non-exhaustive inventory of places where the C relies on behaviour that is
implementation-defined or undefined in C but must be *defined* in Rust:

1. **`PSHR32`/`PSHR` add-then-shift overflow.** `PSHR32(a,shift)` overflows for
   `a > INT32_MAX - (1<<(shift-1))`. That cannot happen on real signals, but Rust
   would panic in debug. Use `wrapping_add` and `>>` (arithmetic).
2. **`SHR32`/`SHR` on negative values.** C: arithmetic right shift on every real
   compiler. Rust: `i32 >> n` is arithmetic — correct, but `>> n` with
   `n >= 32` panics in debug and is UB in C. Audit every variable shift
   (`SHR32(x, i-15)` in `celt_rcp`, `SHL32(x, tot_shift)` in `alg_unquant`).
3. **`SHL32` on values that overflow.** `celt/vq.c:171-179` does
   `SHL32(iy[i], tot_shift)` where `tot_shift` can be up to
   `NORM_SHIFT-k-15`; C wraps silently. Use `wrapping_shl`.
4. **`-INT32_MIN`.** `celt_sudiv`'s `-n` and `silk_abs_int32(INT32_MIN)`.
5. **`EXTRACT16`/`EXTEND32` on a value that does not fit `i16`.**
   `EXTRACT16(x)` is `(opus_int16)(x)` — a **truncating cast**, not a
   saturation. In Rust `x as i16` truncates identically; do not use
   `i16::try_from`.
6. **`(int)floor(f)` and `(int)(f)` conversions in the float build** are UB if
   the value does not fit in `int`; Rust's `as i32` saturates instead. In the
   fixed build this is moot.
7. **The orphan `FIXED_POINT_DEBUG` assert** at `celt/vq.c:287-289`: it guards
   `celt_sig_assert(pulsesLeft<=N+3)` and is defined by *no* build system. The
   *following* `if (pulsesLeft > N+3)` branch **is** live in all builds and must
   be ported.
8. **`op_pvq_search_c`'s fixed-point branch** (`celt/vq.c:205-300`): it uses
   `float` intermediate? No — in the fixed build it computes
   `iy[j] = MULT16_16_Q15(X[j],rcp)`; in the float build
   `iy[j] = (int)floor(rcp*X[j])`. The `#ifdef FIXED_POINT` split is inside the
   same function, so choose the branch.
9. **`celt_inner_prod_norm_shift` accumulates into `opus_val64`**
   (`celt/vq.c:65-73`) in the fixed build and then shifts; the 64-bit
   accumulator width matters for long blocks.
10. **`%` on negatives.** `celt/rate.c`'s `interp_bits2pulses` and
    `celt/cwrs.c`'s `V(N,K)` use `%`/`/` on values that are intended to be
    non-negative; Rust's `%` truncates like C's, so this is safe *provided* the
    inputs are non-negative — add debug assertions rather than changing the
    operator.
11. **Bitfields / `unsigned char` overflow.** `celt/rate.c` and `celt/bands.c`
    store masks in `unsigned char`; C wraps, Rust `u8` also wraps only with
    `wrapping_*` (`+` panics in debug). Audit every `unsigned char` accumulation.
12. **`opus_val16` arithmetic in Rust.** `i16 * i16` overflows in Rust debug;
    libopus always widens first (`MULT16_16`). Never write `a * b` on `i16` in
    the port.


`celt/vq.c:104-130`

```c
void exp_rotation(celt_norm *X, int len, int dir, int stride, int K, int spread)
{
   static const int SPREAD_FACTOR[3]={15,10,5};
   int i;
   opus_val16 c, s;
   opus_val16 gain, theta;
   int stride2=0;
   int factor;

   if (2*K>=len || spread==SPREAD_NONE)
      return;
   factor = SPREAD_FACTOR[spread-1];

   gain = celt_div((opus_val32)MULT16_16(Q15_ONE,len),(opus_val32)(len+factor*K));
   theta = HALF16(MULT16_16_Q15(gain,gain));

   c = celt_cos_norm(EXTEND32(theta));
   s = celt_cos_norm(EXTEND32(SUB16(Q15ONE,theta))); /*  sin(theta) */

   if (len>=8*stride)
   {
      stride2 = 1;
      /* This is just a simple (equivalent) way of computing sqrt(len/stride) with rounding.
         It's basically incrementing long as (stride2+0.5)^2 < len/stride. */
      while ((stride2*stride2+stride2)*stride + (stride>>2) < len)
         stride2++;
   }
```


`celt/vq.c:205-300`

```c
opus_val16 op_pvq_search_c(celt_norm *X, int *iy, int K, int N, int arch)
{
   VARDECL(celt_norm, y);
   VARDECL(int, signx);
   int i, j;
   int pulsesLeft;
   opus_val32 sum;
   opus_val32 xy;
   opus_val16 yy;
   SAVE_STACK;

   (void)arch;
   ALLOC(y, N, celt_norm);
   ALLOC(signx, N, int);
#ifdef FIXED_POINT
   {
      int shift = (celt_ilog2(1+celt_inner_prod_norm_shift(X, X, N, arch))+1)/2;
      shift = IMAX(0, shift+(NORM_SHIFT-14)-14);
      norm_scaledown(X, N, shift);
   }
#endif
   /* Get rid of the sign */
   sum = 0;
   j=0; do {
      signx[j] = X[j]<0;
      /* OPT: Make sure the compiler doesn't use a branch on ABS16(). */
      X[j] = ABS16(X[j]);
      iy[j] = 0;
      y[j] = 0;
   } while (++j<N);

   xy = yy = 0;

   pulsesLeft = K;

   /* Do a pre-search by projecting on the pyramid */
   if (K > (N>>1))
   {
      opus_val16 rcp;
      j=0; do {
         sum += X[j];
      }  while (++j<N);

      /* If X is too small, just replace it with a pulse at 0 */
#ifdef FIXED_POINT
      if (sum <= K)
#else
      /* Prevents infinities and NaNs from causing too many pulses
         to be allocated. 64 is an approximation of infinity here. */
      if (!(sum > EPSILON && sum < 64))
#endif
      {
         X[0] = QCONST16(1.f,14);
         j=1; do
            X[j]=0;
         while (++j<N);
         sum = QCONST16(1.f,14);
      }
#ifdef FIXED_POINT
      rcp = EXTRACT16(MULT16_32_Q16(K, celt_rcp(sum)));
#else
      /* Using K+e with e < 1 guarantees we cannot get more than K pulses. */
      rcp = EXTRACT16(MULT16_32_Q16(K+0.8f, celt_rcp(sum)));
#endif
      j=0; do {
#ifdef FIXED_POINT
         /* It's really important to round *towards zero* here */
         iy[j] = MULT16_16_Q15(X[j],rcp);
#else
         iy[j] = (int)floor(rcp*X[j]);
#endif
         y[j] = (celt_norm)iy[j];
         yy = MAC16_16(yy, y[j],y[j]);
         xy = MAC16_16(xy, X[j],y[j]);
         y[j] *= 2;
         pulsesLeft -= iy[j];
      }  while (++j<N);
   }
   celt_sig_assert(pulsesLeft>=0);

   /* This should never happen, but just in case it does (e.g. on silence)
      we fill the first bin with pulses. */
#ifdef FIXED_POINT_DEBUG
   celt_sig_assert(pulsesLeft<=N+3);
#endif
   if (pulsesLeft > N+3)
   {
      opus_val16 tmp = (opus_val16)pulsesLeft;
      yy = MAC16_16(yy, tmp, tmp);
      yy = MAC16_16(yy, tmp, y[0]);
      iy[0] += pulsesLeft;
      pulsesLeft=0;
   }

   for (i=0;i<pulsesLeft;i++)
   {
```


`celt/arch.h:59-96`

```c
#define CELT_FATAL(str) celt_fatal(str, __FILE__, __LINE__)

#if defined(ENABLE_ASSERTIONS) || defined(ENABLE_HARDENING)
#ifdef __GNUC__
__attribute__((noreturn))
#endif
void celt_fatal(const char *str, const char *file, int line);

#if defined(CELT_C) && !defined(OVERRIDE_celt_fatal)
#include <stdio.h>
#include <stdlib.h>
#ifdef __GNUC__
__attribute__((noreturn))
#endif
void celt_fatal(const char *str, const char *file, int line)
{
   fprintf (stderr, "Fatal (internal) error in %s, line %d: %s\n", file, line, str);
#if defined(_MSC_VER)
   _set_abort_behavior( 0, _WRITE_ABORT_MSG);
#endif
   abort();
}
#endif

#define celt_assert(cond) {if (!(cond)) {CELT_FATAL("assertion failed: " #cond);}}
#define celt_assert2(cond, message) {if (!(cond)) {CELT_FATAL("assertion failed: " #cond "\n" message);}}
#define MUST_SUCCEED(call) celt_assert((call) == OPUS_OK)
#else
#define celt_assert(cond) ((void)(cond))
#define celt_assert2(cond, message) ((void)(cond))
#define MUST_SUCCEED(call) do {if((call) != OPUS_OK) {RESTORE_STACK; return OPUS_INTERNAL_ERROR;} } while (0)
#endif

#if defined(ENABLE_ASSERTIONS)
#define celt_sig_assert(cond) {if (!(cond)) {CELT_FATAL("signal assertion failed: " #cond);}}
#else
#define celt_sig_assert(cond) ((void)(cond))
#endif
```


`silk/typedef.h:52-80`

```c

/* assertions */
#if (defined _WIN32 && !defined _WINCE && !defined(__GNUC__) && !defined(NO_ASSERTS))
# ifndef silk_assert
#  include <crtdbg.h>      /* ASSERTE() */
#  define silk_assert(COND)   _ASSERTE(COND)
# endif
#else
# ifdef ENABLE_ASSERTIONS
#  include <stdio.h>
#  include <stdlib.h>
#define silk_fatal(str) _silk_fatal(str, __FILE__, __LINE__);
#ifdef __GNUC__
__attribute__((noreturn))
#endif
static OPUS_INLINE void _silk_fatal(const char *str, const char *file, int line)
{
   fprintf (stderr, "Fatal (internal) error in %s, line %d: %s\n", file, line, str);
#if defined(_MSC_VER)
   _set_abort_behavior( 0, _WRITE_ABORT_MSG);
#endif
   abort();
}
#  define silk_assert(COND) {if (!(COND)) {silk_fatal("assertion failed: " #COND);}}
# else
#  define silk_assert(COND) ((void)(COND))
# endif
#endif

```


### 7.11 Conditionals and asserts inside bit-exact paths

`ENABLE_ASSERTIONS` must be **off** for the reference build, but two classes of
`#ifdef` still change *values*, not just diagnostics:

* `celt/vq.c`'s `pulsesLeft > N+3` recovery branch (above) runs regardless.
* `celt/rate.c`'s `if (bits > ...)` clamps, and `celt/bands.c`'s
  `celt_sig_assert` blocks — check each `#ifdef ENABLE_ASSERTIONS` for a
  variable assignment inside.
* `silk/*`: `silk_assert` is a no-op without `ENABLE_ASSERTIONS`; the
  `#ifdef ENABLE_ASSERTIONS` blocks in `silk/NSQ.c`, `silk/NSQ_del_dec.c`,
  `silk/PLC.c` only verify. Still, grep for assignments:
  `grep -rn -A3 'ifdef ENABLE_ASSERTIONS' celt silk src`.
* `FUZZING` changes encoder decisions. Never enable it for the reference build.
* `ENABLE_HARDENING` (on by default) enables `celt_fatal` and the
  `validate_celt_encoder`/`validate_celt_decoder`/`validate_opus_decoder`
  functions — these only assert, but they read fields that must therefore exist
  in the port (useful as a free consistency check).


`celt/arch.h:59-96`

```c
#define CELT_FATAL(str) celt_fatal(str, __FILE__, __LINE__)

#if defined(ENABLE_ASSERTIONS) || defined(ENABLE_HARDENING)
#ifdef __GNUC__
__attribute__((noreturn))
#endif
void celt_fatal(const char *str, const char *file, int line);

#if defined(CELT_C) && !defined(OVERRIDE_celt_fatal)
#include <stdio.h>
#include <stdlib.h>
#ifdef __GNUC__
__attribute__((noreturn))
#endif
void celt_fatal(const char *str, const char *file, int line)
{
   fprintf (stderr, "Fatal (internal) error in %s, line %d: %s\n", file, line, str);
#if defined(_MSC_VER)
   _set_abort_behavior( 0, _WRITE_ABORT_MSG);
#endif
   abort();
}
#endif

#define celt_assert(cond) {if (!(cond)) {CELT_FATAL("assertion failed: " #cond);}}
#define celt_assert2(cond, message) {if (!(cond)) {CELT_FATAL("assertion failed: " #cond "\n" message);}}
#define MUST_SUCCEED(call) celt_assert((call) == OPUS_OK)
#else
#define celt_assert(cond) ((void)(cond))
#define celt_assert2(cond, message) ((void)(cond))
#define MUST_SUCCEED(call) do {if((call) != OPUS_OK) {RESTORE_STACK; return OPUS_INTERNAL_ERROR;} } while (0)
#endif

#if defined(ENABLE_ASSERTIONS)
#define celt_sig_assert(cond) {if (!(cond)) {CELT_FATAL("signal assertion failed: " #cond);}}
#else
#define celt_sig_assert(cond) ((void)(cond))
#endif
```


`celt/celt_decoder.c:144-182`

```c
void validate_celt_decoder(CELTDecoder *st)
{
#if !defined(CUSTOM_MODES) && !defined(ENABLE_OPUS_CUSTOM_API) && !defined(ENABLE_QEXT)
   celt_assert(st->mode == opus_custom_mode_create(48000, 960, NULL));
   celt_assert(st->overlap == 120);
   celt_assert(st->end <= 21);
#else
/* From Section 4.3 in the spec: "The normal CELT layer uses 21 of those bands,
   though Opus Custom (see Section 6.2) may use a different number of bands"

   Check if it's within the maximum number of Bark frequency bands instead */
   celt_assert(st->end <= 25);
#endif
   celt_assert(st->channels == 1 || st->channels == 2);
   celt_assert(st->stream_channels == 1 || st->stream_channels == 2);
   celt_assert(st->downsample > 0);
   celt_assert(st->start == 0 || st->start == 17);
   celt_assert(st->start < st->end);
#ifdef OPUS_ARCHMASK
   celt_assert(st->arch >= 0);
   celt_assert(st->arch <= OPUS_ARCHMASK);
#endif
#ifndef ENABLE_QEXT
   celt_assert(st->last_pitch_index <= PLC_PITCH_LAG_MAX);
   celt_assert(st->last_pitch_index >= PLC_PITCH_LAG_MIN || st->last_pitch_index == 0);
#endif
   celt_assert(st->postfilter_period < MAX_PERIOD);
   celt_assert(st->postfilter_period >= COMBFILTER_MINPERIOD || st->postfilter_period == 0);
   celt_assert(st->postfilter_period_old < MAX_PERIOD);
   celt_assert(st->postfilter_period_old >= COMBFILTER_MINPERIOD || st->postfilter_period_old == 0);
   celt_assert(st->postfilter_tapset <= 2);
   celt_assert(st->postfilter_tapset >= 0);
   celt_assert(st->postfilter_tapset_old <= 2);
   celt_assert(st->postfilter_tapset_old >= 0);
}
#endif

int celt_decoder_get_size(int channels)
{
```


### 7.99 Prioritized hazard checklist (top 10)

Ranked by "probability of silently breaking bit-exactness × difficulty of
noticing":

1. **`celt/fixed_generic.h` macro semantics** — `SHR`/`PSHR`/`MULT16_16`/
   `MULT16_32_Q15`/`SATURATE` rounding and intermediate widths. A one-LSB
   difference here corrupts everything and looks like a DSP bug.
   *Test: exhaustive differential test of each macro vs. compiled C.*
2. **The range coder** — `ec_enc_normalize`/`ec_enc_carry_out`/`ec_dec_normalize`
   loop counts, `rem`/`ext` carry chain, `ec_enc_done`'s tail.
   *Test: encode/decode random symbol streams and diff bytes with C.*
3. **`silk/macros.h` two-part multiplies** (`silk_SMULWB`/`SMLAWB`/`SMULWW`/
   `SMLAWW`/`SMMUL`) and `silk_RSHIFT_ROUND`'s `shift==1` special case.
4. **`celt_cos_norm` / `celt_cos_norm32`** — used by `exp_rotation` (every PVQ
   band) and by mode/window generation.
5. **`celt_rsqrt_norm`/`celt_sqrt`** — the fixed-point polynomial and its
   `MULT16_32_Q15` final step; feeds band energies and normalisation.
6. **`silk_DIV32_varQ` / `silk_INVERSE32_varQ` / `silk_SQRT_APPROX`** — the
   correction step and the `Qres` clamps in `silk/Inlines.h`.
7. **`ec_ilog` / `EC_ILOG` / `celt_udiv`** — the `USE_SMALL_DIV_TABLE` branch,
   the `q+(n-q*d>=d)` correction, and `ec_tell`'s dependence on `rng`.
8. **The generated mode tables** (`static_modes_fixed.h`) — `mdct_twiddles960`,
   `fft_twiddles*`, `cache_*`. Transcribe mechanically; a single stale value
   makes one band wrong.
9. **`kiss_fft` butterfly order and the fixed-point `SAMP_MAX` clamp** — replacing
   kiss with a generic FFT silently changes all MDCT output.
10. **`float`↔`int` conversions in `celt/float_cast.h`** — `lrintf`
    (round-half-to-even) vs `+16384` truncation, and the `FLOAT2INT24` variant
    under `ENABLE_RES24`. Only relevant if the port touches the float API or the
    RES24 conversion paths, but it is the classic "off by one on exact halves"
    bug.

**Runner-up hazards that will bite later:** `celt_cos_norm` in `modes.c` window
generation (regenerate rather than round-trip), `silk_LSFCosTab_FIX_Q12`
(hand-transcribed, even values), `silk_sigm_Q15` segment boundaries,
`silk_lin2log`/`silk_log2lin` clamps, `opus_res` 16- vs 24-bit
(`ENABLE_RES24`) conversions in `prefilter_and_fold`/`celt_preemphasis`, and
`opus_pcm_soft_clip` (float-only; absent in the fixed build).


---

## Appendix A — mechanically extracted file-level call graph

Computed by `docs/gen_graph.py`: for every portable `.c` file in `celt/`,
`silk/` and `src/` (excluding SIMD dirs, tests, demos and `mini_kfft.c`), it
finds every file-scope function definition, then records which files call each
of them. `name×N` = number of call sites (capped at the 8 most-called names per
target file). **Caveat:** `static` helpers with identical names in different
files (notably the `silk/fixed/*` vs `silk/float/*` pairs) are attributed to the
first file alphabetically, so a few edges are false positives; the real
dependency set is the union, and §3.3 calls out the two that matter.

```
# mechanical call graph (file -> files whose functions it calls)

celt/bands.c -> celt/entcode.c [ec_tell_frac×12]; celt/entdec.c [ec_dec_uint×3, ec_decode×2, ec_dec_update×2, ec_dec_bits×2, ec_dec_bit_logp×1]; celt/entenc.c [ec_enc_uint×3, ec_encode×2, ec_enc_bits×2, ec_enc_bit_logp×1]; celt/mathops.c [celt_cos_norm32×6, celt_sqrt×3, isqrt32×2, celt_rsqrt_norm32×2, celt_rsqrt_norm×1, celt_sqrt32×1, celt_rcp_norm32×1]; celt/vq.c [celt_inner_prod_norm_shift×6, renormalise_vector×2, stereo_itheta×2, cubic_quant×2, cubic_unquant×2, alg_quant×1, alg_unquant×1]
celt/celt.c -> (none)
celt/celt_decoder.c -> celt/bands.c [denormalise_bands×8, quant_all_bands×2, celt_lcg_rand×1, anti_collapse×1]; celt/celt.c [comb_filter×5, resampling_factor×1, init_caps×1]; celt/celt_lpc.c [_celt_lpc×1, celt_iir×1, _celt_autocorr×1]; celt/entcode.c [ec_tell_frac×5]; celt/entdec.c [ec_dec_bit_logp×10, ec_dec_init×5, ec_dec_icdf×3, ec_dec_bits×3, ec_dec_uint×2]; celt/mathops.c [frac_div32×2, celt_sqrt×2]; celt/modes.c [opus_custom_mode_create×5, compute_qext_mode×1]; celt/pitch.c [pitch_downsample×1, pitch_search×1]; celt/quant_bands.c [unquant_fine_energy×3, unquant_coarse_energy×2, unquant_energy_finalise×1]; celt/rate.c [clt_compute_allocation×1, clt_compute_extra_allocation×1]; celt/vq.c [renormalise_vector×1]
celt/celt_encoder.c -> celt/bands.c [compute_band_energies×4, hysteresis_decision×3, normalise_bands×2, haar1×2, quant_all_bands×2, anti_collapse×1, spreading_decision×1]; celt/celt.c [comb_filter×5, resampling_factor×1, init_caps×1]; celt/celt_decoder.c [deemphasis×1, celt_synthesis×1]; celt/entcode.c [ec_tell_frac×7]; celt/entenc.c [ec_enc_bit_logp×9, ec_enc_shrink×6, ec_enc_init×5, ec_enc_icdf×3, ec_enc_bits×3, ec_enc_uint×2, ec_enc_done×2]; celt/mathops.c [celt_sqrt×6, frac_div32_q29×2, celt_rcp×1]; celt/modes.c [opus_custom_mode_create×4, compute_qext_mode×1]; celt/pitch.c [pitch_downsample×1, pitch_search×1, remove_doubling×1]; celt/quant_bands.c [amp2Log2×4, quant_fine_energy×3, quant_coarse_energy×2, quant_energy_finalise×2]; celt/rate.c [clt_compute_allocation×1, clt_compute_extra_allocation×1]; celt/vq.c [celt_inner_prod_norm_shift×2]
celt/celt_lpc.c -> celt/mathops.c [frac_div32×1]
celt/cwrs.c -> celt/entdec.c [ec_dec_uint×2]; celt/entenc.c [ec_enc_uint×2]
celt/entcode.c -> (none)
celt/entdec.c -> (none)
celt/entenc.c -> (none)
celt/kiss_fft.c -> (none)
celt/laplace.c -> celt/entdec.c [ec_dec_icdf16×2, ec_dec_init×1, ec_decode_bin×1, ec_dec_update×1]; celt/entenc.c [ec_enc_icdf16×2, ec_enc_init×1, ec_encode_bin×1, ec_enc_done×1]
celt/mathops.c -> (none)
celt/mdct.c -> celt/kiss_fft.c [opus_fft_impl×2, opus_fft_alloc_twiddles×1, opus_fft_alloc×1, opus_fft_free×1]; celt/mathops.c [celt_cos_norm×1]
celt/modes.c -> celt/cwrs.c [log2_frac×1]; celt/mdct.c [clt_mdct_init×1, clt_mdct_clear×1]; celt/rate.c [compute_pulse_cache×2]
celt/pitch.c -> celt/celt_lpc.c [_celt_lpc×1, _celt_autocorr×1]; celt/mathops.c [frac_div32×1, celt_rsqrt_norm×1, celt_sqrt×1]
celt/quant_bands.c -> celt/entcode.c [ec_tell_frac×2]; celt/entdec.c [ec_dec_bits×2, ec_dec_bit_logp×1, ec_dec_icdf×1]; celt/entenc.c [ec_enc_bit_logp×2, ec_enc_bits×2, ec_enc_icdf×1]; celt/laplace.c [ec_laplace_encode×1, ec_laplace_decode×1]
celt/rate.c -> celt/cwrs.c [get_required_bits×1]; celt/entcode.c [ec_tell_frac×2]; celt/entdec.c [ec_dec_icdf×3, ec_dec_bit_logp×2, ec_dec_uint×2]; celt/entenc.c [ec_enc_bit_logp×3, ec_enc_icdf×3, ec_enc_uint×2]
celt/vq.c -> celt/cwrs.c [encode_pulses×3, decode_pulses×1]; celt/entdec.c [ec_dec_bits×6, ec_dec_uint×2, ec_dec_bit_logp×1]; celt/entenc.c [ec_enc_bits×6, ec_enc_uint×2, ec_enc_bit_logp×1]; celt/mathops.c [celt_rcp_norm32×3, celt_rcp×3, celt_rsqrt_norm32×2, celt_sqrt32×2, celt_cos_norm×2, celt_rsqrt_norm×1]
silk/A2NLSF.c -> silk/bwexpander_32.c [silk_bwexpander_32×1]
silk/CNG.c -> silk/NLSF2A.c [silk_NLSF2A×1]
silk/HP_variable_cutoff.c -> silk/lin2log.c [silk_lin2log×4]
silk/LPC_analysis_filter.c -> (none)
silk/LPC_fit.c -> silk/bwexpander_32.c [silk_bwexpander_32×1]
silk/LPC_inv_pred_gain.c -> (none)
silk/LP_variable_cutoff.c -> silk/biquad_alt.c [silk_biquad_alt_stride1×1]
silk/NLSF2A.c -> silk/LPC_fit.c [silk_LPC_fit×1]; silk/bwexpander_32.c [silk_bwexpander_32×1]
silk/NLSF_VQ.c -> (none)
silk/NLSF_VQ_weights_laroia.c -> (none)
silk/NLSF_decode.c -> silk/NLSF_stabilize.c [silk_NLSF_stabilize×1]; silk/NLSF_unpack.c [silk_NLSF_unpack×1]
silk/NLSF_del_dec_quant.c -> (none)
silk/NLSF_encode.c -> silk/NLSF_VQ.c [silk_NLSF_VQ×1]; silk/NLSF_decode.c [silk_NLSF_decode×1]; silk/NLSF_del_dec_quant.c [silk_NLSF_del_dec_quant×1]; silk/NLSF_stabilize.c [silk_NLSF_stabilize×1]; silk/NLSF_unpack.c [silk_NLSF_unpack×1]; silk/lin2log.c [silk_lin2log×1]; silk/sort.c [silk_insertion_sort_increasing×2]
silk/NLSF_stabilize.c -> silk/sort.c [silk_insertion_sort_increasing_all_values_int16×1]
silk/NLSF_unpack.c -> (none)
silk/NSQ.c -> silk/LPC_analysis_filter.c [silk_LPC_analysis_filter×1]
silk/NSQ_del_dec.c -> silk/LPC_analysis_filter.c [silk_LPC_analysis_filter×1]
silk/PLC.c -> silk/LPC_analysis_filter.c [silk_LPC_analysis_filter×1]; silk/bwexpander.c [silk_bwexpander×1]; silk/sum_sqr_shift.c [silk_sum_sqr_shift×4]
silk/VAD.c -> silk/ana_filt_bank_1.c [silk_ana_filt_bank_1×3]; silk/lin2log.c [silk_lin2log×2]; silk/sigm_Q15.c [silk_sigm_Q15×3]
silk/VQ_WMat_EC.c -> silk/lin2log.c [silk_lin2log×1]
silk/ana_filt_bank_1.c -> (none)
silk/biquad_alt.c -> (none)
silk/bwexpander.c -> (none)
silk/bwexpander_32.c -> (none)
silk/check_control_input.c -> (none)
silk/code_signs.c -> celt/entdec.c [ec_dec_icdf×2]; celt/entenc.c [ec_enc_icdf×1]
silk/control_SNR.c -> (none)
silk/control_audio_bandwidth.c -> (none)
silk/control_codec.c -> silk/control_audio_bandwidth.c [silk_control_audio_bandwidth×1]; silk/resampler.c [silk_resampler_init×3, silk_resampler×2]
silk/debug.c -> (none)
silk/dec_API.c -> celt/entdec.c [ec_dec_bit_logp×2, ec_dec_icdf×1]; silk/decode_frame.c [silk_decode_frame×1]; silk/decode_indices.c [silk_decode_indices×1]; silk/decode_pulses.c [silk_decode_pulses×1]; silk/decoder_set_fs.c [silk_decoder_set_fs×1]; silk/init_decoder.c [silk_init_decoder×2, silk_reset_decoder×1]; silk/resampler.c [silk_resampler×4]; silk/stereo_MS_to_LR.c [silk_stereo_MS_to_LR×1]; silk/stereo_decode_pred.c [silk_stereo_decode_pred×2, silk_stereo_decode_mid_only×2]
silk/decode_core.c -> silk/LPC_analysis_filter.c [silk_LPC_analysis_filter×1]
silk/decode_frame.c -> silk/CNG.c [silk_CNG×1]; silk/PLC.c [silk_PLC×2, silk_PLC_glue_frames×1]; silk/decode_core.c [silk_decode_core×1]; silk/decode_indices.c [silk_decode_indices×1]; silk/decode_parameters.c [silk_decode_parameters×1]; silk/decode_pulses.c [silk_decode_pulses×1]
silk/decode_indices.c -> celt/entdec.c [ec_dec_icdf×19]; silk/NLSF_unpack.c [silk_NLSF_unpack×1]
silk/decode_parameters.c -> silk/NLSF2A.c [silk_NLSF2A×2]; silk/NLSF_decode.c [silk_NLSF_decode×1]; silk/bwexpander.c [silk_bwexpander×2]; silk/decode_pitch.c [silk_decode_pitch×1]; silk/gain_quant.c [silk_gains_dequant×1]
silk/decode_pitch.c -> (none)
silk/decode_pulses.c -> celt/entdec.c [ec_dec_icdf×4]; silk/code_signs.c [silk_decode_signs×1]; silk/shell_coder.c [silk_shell_decoder×1]
silk/decoder_set_fs.c -> silk/resampler.c [silk_resampler_init×1]
silk/enc_API.c -> celt/entenc.c [ec_enc_icdf×2, ec_enc_patch_initial_bits×1]; silk/HP_variable_cutoff.c [silk_HP_variable_cutoff×1]; silk/check_control_input.c [check_control_input×1]; silk/control_SNR.c [silk_control_SNR×1]; silk/control_codec.c [silk_control_encoder×1]; silk/encode_indices.c [silk_encode_indices×1]; silk/encode_pulses.c [silk_encode_pulses×1]; silk/init_encoder.c [silk_init_encoder×3]; silk/resampler.c [silk_resampler×5]; silk/stereo_LR_to_MS.c [silk_stereo_LR_to_MS×1]; silk/stereo_encode_pred.c [silk_stereo_encode_pred×2, silk_stereo_encode_mid_only×2]
silk/encode_indices.c -> celt/entenc.c [ec_enc_icdf×21]; silk/NLSF_unpack.c [silk_NLSF_unpack×1]
silk/encode_pulses.c -> celt/entenc.c [ec_enc_icdf×7]; silk/code_signs.c [silk_encode_signs×1]; silk/shell_coder.c [silk_shell_encoder×1]
silk/fixed/LTP_analysis_filter_FIX.c -> (none)
silk/fixed/LTP_scale_ctrl_FIX.c -> silk/log2lin.c [silk_log2lin×2]
silk/fixed/apply_sine_window_FIX.c -> (none)
silk/fixed/autocorr_FIX.c -> celt/celt_lpc.c [_celt_autocorr×1]
silk/fixed/burg_modified_FIX.c -> silk/fixed/vector_ops_FIX.c [silk_inner_prod_aligned×1]
silk/fixed/corrMatrix_FIX.c -> silk/fixed/vector_ops_FIX.c [silk_inner_prod_aligned×2]; silk/sum_sqr_shift.c [silk_sum_sqr_shift×1]
silk/fixed/encode_frame_FIX.c -> silk/LP_variable_cutoff.c [silk_LP_variable_cutoff×1]; silk/encode_indices.c [silk_encode_indices×2]; silk/encode_pulses.c [silk_encode_pulses×2]; silk/fixed/find_pitch_lags_FIX.c [silk_find_pitch_lags_FIX×1]; silk/fixed/find_pred_coefs_FIX.c [silk_find_pred_coefs_FIX×1]; silk/fixed/noise_shape_analysis_FIX.c [silk_noise_shape_analysis_FIX×1]; silk/fixed/process_gains_FIX.c [silk_process_gains_FIX×1]; silk/gain_quant.c [silk_gains_ID×2, silk_gains_quant×1, silk_gains_dequant×1]
silk/fixed/find_LPC_FIX.c -> silk/A2NLSF.c [silk_A2NLSF×2]; silk/LPC_analysis_filter.c [silk_LPC_analysis_filter×1]; silk/NLSF2A.c [silk_NLSF2A×1]; silk/interpolate.c [silk_interpolate×1]; silk/sum_sqr_shift.c [silk_sum_sqr_shift×2]
silk/fixed/find_LTP_FIX.c -> silk/fixed/corrMatrix_FIX.c [silk_corrVector_FIX×1, silk_corrMatrix_FIX×1]; silk/sum_sqr_shift.c [silk_sum_sqr_shift×1]
silk/fixed/find_pitch_lags_FIX.c -> silk/LPC_analysis_filter.c [silk_LPC_analysis_filter×1]; silk/bwexpander.c [silk_bwexpander×1]; silk/fixed/apply_sine_window_FIX.c [silk_apply_sine_window×2]; silk/fixed/autocorr_FIX.c [silk_autocorr×1]; silk/fixed/k2a_FIX.c [silk_k2a×1]; silk/fixed/pitch_analysis_core_FIX.c [silk_pitch_analysis_core×1]; silk/fixed/schur_FIX.c [silk_schur×1]
silk/fixed/find_pred_coefs_FIX.c -> silk/fixed/LTP_analysis_filter_FIX.c [silk_LTP_analysis_filter_FIX×1]; silk/fixed/LTP_scale_ctrl_FIX.c [silk_LTP_scale_ctrl_FIX×1]; silk/fixed/find_LPC_FIX.c [silk_find_LPC_FIX×1]; silk/fixed/find_LTP_FIX.c [silk_find_LTP_FIX×1]; silk/fixed/residual_energy_FIX.c [silk_residual_energy_FIX×1]; silk/fixed/vector_ops_FIX.c [silk_scale_copy_vector16×1]; silk/log2lin.c [silk_log2lin×1]; silk/process_NLSFs.c [silk_process_NLSFs×1]; silk/quant_LTP_gains.c [silk_quant_LTP_gains×1]
silk/fixed/k2a_FIX.c -> (none)
silk/fixed/k2a_Q16_FIX.c -> (none)
silk/fixed/noise_shape_analysis_FIX.c -> silk/LPC_fit.c [silk_LPC_fit×1]; silk/bwexpander_32.c [silk_bwexpander_32×2]; silk/fixed/apply_sine_window_FIX.c [silk_apply_sine_window×2]; silk/fixed/autocorr_FIX.c [silk_autocorr×1]; silk/fixed/k2a_Q16_FIX.c [silk_k2a_Q16×1]; silk/fixed/schur64_FIX.c [silk_schur64×1]; silk/lin2log.c [silk_lin2log×1]; silk/log2lin.c [silk_log2lin×2]; silk/sigm_Q15.c [silk_sigm_Q15×1]; silk/sum_sqr_shift.c [silk_sum_sqr_shift×1]
silk/fixed/pitch_analysis_core_FIX.c -> silk/fixed/vector_ops_FIX.c [silk_inner_prod_aligned×7]; silk/lin2log.c [silk_lin2log×2]; silk/resampler_down2.c [silk_resampler_down2×2]; silk/resampler_down2_3.c [silk_resampler_down2_3×1]; silk/sort.c [silk_insertion_sort_decreasing_int16×1]; silk/sum_sqr_shift.c [silk_sum_sqr_shift×1]
silk/fixed/process_gains_FIX.c -> silk/gain_quant.c [silk_gains_quant×1]; silk/log2lin.c [silk_log2lin×1]; silk/sigm_Q15.c [silk_sigm_Q15×1]
silk/fixed/regularize_correlations_FIX.c -> (none)
silk/fixed/residual_energy16_FIX.c -> (none)
silk/fixed/residual_energy_FIX.c -> silk/LPC_analysis_filter.c [silk_LPC_analysis_filter×1]; silk/sum_sqr_shift.c [silk_sum_sqr_shift×1]
silk/fixed/schur64_FIX.c -> (none)
silk/fixed/schur_FIX.c -> (none)
silk/fixed/vector_ops_FIX.c -> (none)
silk/fixed/warped_autocorrelation_FIX.c -> (none)
silk/float/LPC_analysis_filter_FLP.c -> (none)
silk/float/LPC_inv_pred_gain_FLP.c -> (none)
silk/float/LTP_analysis_filter_FLP.c -> (none)
silk/float/LTP_scale_ctrl_FLP.c -> silk/log2lin.c [silk_log2lin×2]
silk/float/apply_sine_window_FLP.c -> (none)
silk/float/autocorrelation_FLP.c -> (none)
silk/float/burg_modified_FLP.c -> silk/float/energy_FLP.c [silk_energy_FLP×2]
silk/float/bwexpander_FLP.c -> (none)
silk/float/corrMatrix_FLP.c -> silk/float/energy_FLP.c [silk_energy_FLP×1]
silk/float/encode_frame_FLP.c -> silk/LP_variable_cutoff.c [silk_LP_variable_cutoff×1]; silk/encode_indices.c [silk_encode_indices×2]; silk/encode_pulses.c [silk_encode_pulses×2]; silk/float/find_pitch_lags_FLP.c [silk_find_pitch_lags_FLP×1]; silk/float/find_pred_coefs_FLP.c [silk_find_pred_coefs_FLP×1]; silk/float/noise_shape_analysis_FLP.c [silk_noise_shape_analysis_FLP×1]; silk/float/process_gains_FLP.c [silk_process_gains_FLP×1]; silk/float/wrappers_FLP.c [silk_NSQ_wrapper_FLP×2]; silk/gain_quant.c [silk_gains_ID×2, silk_gains_quant×1, silk_gains_dequant×1]
silk/float/energy_FLP.c -> (none)
silk/float/find_LPC_FLP.c -> silk/float/LPC_analysis_filter_FLP.c [silk_LPC_analysis_filter_FLP×1]; silk/float/burg_modified_FLP.c [silk_burg_modified_FLP×2]; silk/float/energy_FLP.c [silk_energy_FLP×2]; silk/float/wrappers_FLP.c [silk_A2NLSF_FLP×2, silk_NLSF2A_FLP×1]; silk/interpolate.c [silk_interpolate×1]
silk/float/find_LTP_FLP.c -> silk/float/corrMatrix_FLP.c [silk_corrVector_FLP×1, silk_corrMatrix_FLP×1]; silk/float/energy_FLP.c [silk_energy_FLP×1]; silk/float/scale_vector_FLP.c [silk_scale_vector_FLP×2]
silk/float/find_pitch_lags_FLP.c -> silk/float/LPC_analysis_filter_FLP.c [silk_LPC_analysis_filter_FLP×1]; silk/float/apply_sine_window_FLP.c [silk_apply_sine_window_FLP×2]; silk/float/autocorrelation_FLP.c [silk_autocorrelation_FLP×1]; silk/float/bwexpander_FLP.c [silk_bwexpander_FLP×1]; silk/float/k2a_FLP.c [silk_k2a_FLP×1]; silk/float/pitch_analysis_core_FLP.c [silk_pitch_analysis_core_FLP×1]; silk/float/schur_FLP.c [silk_schur_FLP×1]
silk/float/find_pred_coefs_FLP.c -> silk/float/LTP_analysis_filter_FLP.c [silk_LTP_analysis_filter_FLP×1]; silk/float/LTP_scale_ctrl_FLP.c [silk_LTP_scale_ctrl_FLP×1]; silk/float/find_LPC_FLP.c [silk_find_LPC_FLP×1]; silk/float/find_LTP_FLP.c [silk_find_LTP_FLP×1]; silk/float/residual_energy_FLP.c [silk_residual_energy_FLP×1]; silk/float/scale_copy_vector_FLP.c [silk_scale_copy_vector_FLP×1]; silk/float/wrappers_FLP.c [silk_process_NLSFs_FLP×1, silk_quant_LTP_gains_FLP×1]
silk/float/inner_product_FLP.c -> (none)
silk/float/k2a_FLP.c -> (none)
silk/float/noise_shape_analysis_FLP.c -> silk/fixed/noise_shape_analysis_FIX.c [warped_gain×2]; silk/float/apply_sine_window_FLP.c [silk_apply_sine_window_FLP×2]; silk/float/autocorrelation_FLP.c [silk_autocorrelation_FLP×1]; silk/float/bwexpander_FLP.c [silk_bwexpander_FLP×3]; silk/float/energy_FLP.c [silk_energy_FLP×1]; silk/float/k2a_FLP.c [silk_k2a_FLP×1]; silk/float/schur_FLP.c [silk_schur_FLP×1]; silk/float/warped_autocorrelation_FLP.c [silk_warped_autocorrelation_FLP×1]
silk/float/pitch_analysis_core_FLP.c -> silk/fixed/pitch_analysis_core_FIX.c [silk_P_Ana_calc_corr_st3×3, silk_P_Ana_calc_energy_st3×3]; silk/float/energy_FLP.c [silk_energy_FLP×6]; silk/float/sort_FLP.c [silk_insertion_sort_decreasing_FLP×1]; silk/resampler_down2.c [silk_resampler_down2×2]; silk/resampler_down2_3.c [silk_resampler_down2_3×1]
silk/float/process_gains_FLP.c -> silk/gain_quant.c [silk_gains_quant×1]
silk/float/regularize_correlations_FLP.c -> (none)
silk/float/residual_energy_FLP.c -> silk/float/LPC_analysis_filter_FLP.c [silk_LPC_analysis_filter_FLP×2]; silk/float/energy_FLP.c [silk_energy_FLP×4]
silk/float/scale_copy_vector_FLP.c -> (none)
silk/float/scale_vector_FLP.c -> (none)
silk/float/schur_FLP.c -> (none)
silk/float/sort_FLP.c -> (none)
silk/float/warped_autocorrelation_FLP.c -> (none)
silk/float/wrappers_FLP.c -> silk/A2NLSF.c [silk_A2NLSF×1]; silk/NLSF2A.c [silk_NLSF2A×1]; silk/process_NLSFs.c [silk_process_NLSFs×1]; silk/quant_LTP_gains.c [silk_quant_LTP_gains×1]
silk/gain_quant.c -> silk/lin2log.c [silk_lin2log×1]; silk/log2lin.c [silk_log2lin×2]
silk/init_decoder.c -> silk/CNG.c [silk_CNG_Reset×1]; silk/PLC.c [silk_PLC_Reset×1]
silk/init_encoder.c -> silk/VAD.c [silk_VAD_Init×1]; silk/lin2log.c [silk_lin2log×1]
silk/inner_prod_aligned.c -> (none)
silk/interpolate.c -> (none)
silk/lin2log.c -> (none)
silk/log2lin.c -> (none)
silk/pitch_est_tables.c -> (none)
silk/process_NLSFs.c -> silk/NLSF2A.c [silk_NLSF2A×2]; silk/NLSF_VQ_weights_laroia.c [silk_NLSF_VQ_weights_laroia×2]; silk/NLSF_encode.c [silk_NLSF_encode×1]; silk/interpolate.c [silk_interpolate×2]
silk/quant_LTP_gains.c -> silk/lin2log.c [silk_lin2log×2]; silk/log2lin.c [silk_log2lin×1]
silk/resampler.c -> silk/resampler_private_IIR_FIR.c [silk_resampler_private_IIR_FIR×2]; silk/resampler_private_down_FIR.c [silk_resampler_private_down_FIR×2]; silk/resampler_private_up2_HQ.c [silk_resampler_private_up2_HQ_wrapper×2]
silk/resampler_down2.c -> (none)
silk/resampler_down2_3.c -> silk/resampler_private_AR2.c [silk_resampler_private_AR2×1]
silk/resampler_private_AR2.c -> (none)
silk/resampler_private_IIR_FIR.c -> silk/resampler_private_up2_HQ.c [silk_resampler_private_up2_HQ×1]
silk/resampler_private_down_FIR.c -> silk/resampler_private_AR2.c [silk_resampler_private_AR2×1]
silk/resampler_private_up2_HQ.c -> (none)
silk/resampler_rom.c -> (none)
silk/shell_coder.c -> celt/entdec.c [ec_dec_icdf×1]; celt/entenc.c [ec_enc_icdf×1]
silk/sigm_Q15.c -> (none)
silk/sort.c -> (none)
silk/stereo_LR_to_MS.c -> silk/stereo_find_predictor.c [silk_stereo_find_predictor×2]; silk/stereo_quant_pred.c [silk_stereo_quant_pred×5]
silk/stereo_MS_to_LR.c -> (none)
silk/stereo_decode_pred.c -> celt/entdec.c [ec_dec_icdf×4]
silk/stereo_encode_pred.c -> celt/entenc.c [ec_enc_icdf×4]
silk/stereo_find_predictor.c -> silk/inner_prod_aligned.c [silk_inner_prod_aligned_scale×1]; silk/sum_sqr_shift.c [silk_sum_sqr_shift×2]
silk/stereo_quant_pred.c -> (none)
silk/sum_sqr_shift.c -> (none)
silk/table_LSF_cos.c -> (none)
silk/tables_LTP.c -> (none)
silk/tables_NLSF_CB_NB_MB.c -> (none)
silk/tables_NLSF_CB_WB.c -> (none)
silk/tables_gain.c -> (none)
silk/tables_other.c -> (none)
silk/tables_pitch_lag.c -> (none)
silk/tables_pulses_per_block.c -> (none)
src/analysis.c -> src/mlp.c [analysis_compute_dense×2, analysis_compute_gru×1]; src/opus_encoder.c [is_digital_silence×1]
src/extensions.c -> celt/laplace.c [main×1]
src/mapping_matrix.c -> (none)
src/mlp.c -> (none)
src/mlp_data.c -> (none)
src/opus.c -> (none)
src/opus_decoder.c -> celt/celt_decoder.c [celt_decode_with_ec×3, celt_decoder_get_size×1, celt_decoder_init×1, celt_decode_with_ec_dred×1]; celt/entdec.c [ec_dec_bit_logp×2, ec_dec_init×1, ec_dec_uint×1]; silk/dec_API.c [silk_Get_Decoder_Size×2, silk_ResetDecoder×2, silk_LoadOSCEModels×1, silk_InitDecoder×1, silk_Decode×1]; src/extensions.c [opus_extension_iterator_init×2, opus_extension_iterator_find×2]; src/opus.c [opus_packet_get_samples_per_frame×4, opus_packet_parse_impl×2, opus_pcm_soft_clip_impl×1, opus_packet_parse×1]
src/opus_encoder.c -> celt/celt_encoder.c [celt_encode_with_ec×5, celt_encoder_get_size×1, celt_encoder_init×1]; celt/entenc.c [ec_enc_bit_logp×2, ec_enc_shrink×2, ec_enc_init×1, ec_enc_uint×1, ec_enc_done×1]; celt/mathops.c [celt_sqrt×5, frac_div32×1]; silk/biquad_alt.c [silk_biquad_alt_stride1×1]; silk/enc_API.c [silk_InitEncoder×3, silk_Encode×2, silk_Get_Encoder_Size×1]; silk/lin2log.c [silk_lin2log×3]; silk/log2lin.c [silk_log2lin×1]; src/analysis.c [tonality_analysis_reset×2, tonality_analysis_init×1, tonality_get_info×1, run_analysis×1]; src/repacketizer.c [opus_packet_pad×2, opus_repacketizer_init×1, opus_repacketizer_cat×1, opus_repacketizer_out_range_impl×1, opus_packet_pad_impl×1]
src/opus_multistream.c -> (none)
src/opus_multistream_decoder.c -> src/opus.c [opus_packet_parse_impl×1]; src/opus_decoder.c [opus_decoder_get_size×8, opus_decoder_ctl×4, opus_decoder_init×2, opus_decode_native×1, opus_packet_get_nb_samples×1]; src/opus_multistream.c [validate_layout×2, get_left_channel×1, get_right_channel×1, get_mono_channel×1]
src/opus_multistream_encoder.c -> celt/bands.c [compute_band_energies×1]; celt/celt.c [resampling_factor×1]; celt/celt_encoder.c [celt_preemphasis×1]; celt/mathops.c [isqrt32×1]; celt/quant_bands.c [amp2Log2×1]; src/opus_encoder.c [opus_encoder_ctl×21, opus_encoder_init×12, opus_encoder_get_size×2, frame_size_select×1, opus_encode_native×1]; src/opus_multistream.c [get_left_channel×2, get_right_channel×2, get_mono_channel×2, validate_layout×1]; src/repacketizer.c [opus_repacketizer_init×1, opus_repacketizer_cat×1, opus_repacketizer_get_nb_frames×1, opus_repacketizer_out_range_impl×1]
src/opus_projection_decoder.c -> src/mapping_matrix.c [mapping_matrix_get_size×2, mapping_matrix_init×1, mapping_matrix_multiply_channel_out_float×1, mapping_matrix_multiply_channel_out_short×1, mapping_matrix_multiply_channel_out_int24×1]; src/opus_multistream_decoder.c [opus_multistream_decode_native×3, opus_multistream_decoder_get_size×1, opus_multistream_decoder_init×1, opus_multistream_decoder_ctl_va_list×1]
src/opus_projection_encoder.c -> celt/mathops.c [isqrt32×1]; src/mapping_matrix.c [mapping_matrix_init×10, mapping_matrix_get_size×4, mapping_matrix_get_data×1, mapping_matrix_multiply_channel_in_float×1, mapping_matrix_multiply_channel_in_short×1, mapping_matrix_multiply_channel_in_int24×1]; src/opus_multistream_encoder.c [opus_multistream_encoder_get_size×1, opus_multistream_encoder_init×1, opus_multistream_encoder_ctl_va_list×1]
src/repacketizer.c -> src/extensions.c [opus_packet_extensions_generate×2, opus_packet_extensions_count×1, opus_packet_extensions_parse×1]; src/opus.c [encode_size×3, opus_packet_parse_impl×3, opus_packet_get_samples_per_frame×1]; src/opus_decoder.c [opus_packet_get_nb_frames×1]

# include graph
celt/bands.c -> bands.h, modes.h, vq.h, cwrs.h, stack_alloc.h, os_support.h, mathops.h, rate.h, quant_bands.h, pitch.h
celt/celt.c -> os_support.h, mdct.h, celt.h, pitch.h, bands.h, modes.h, entcode.h, quant_bands.h, rate.h, stack_alloc.h, mathops.h, float_cast.h, celt_lpc.h, vq.h, mips/celt_mipsr1.h
celt/celt_decoder.c -> cpu_support.h, os_support.h, mdct.h, celt.h, pitch.h, bands.h, modes.h, entcode.h, quant_bands.h, rate.h, stack_alloc.h, mathops.h, float_cast.h, celt_lpc.h, vq.h, lpcnet.h, lpcnet_private.h
celt/celt_encoder.c -> cpu_support.h, os_support.h, mdct.h, celt.h, pitch.h, bands.h, modes.h, entcode.h, quant_bands.h, rate.h, stack_alloc.h, mathops.h, float_cast.h, celt_lpc.h, vq.h
celt/celt_lpc.c -> celt_lpc.h, stack_alloc.h, mathops.h, pitch.h
celt/cwrs.c -> os_support.h, cwrs.h, mathops.h, arch.h
celt/entcode.c -> entcode.h, arch.h
celt/entdec.c -> os_support.h, arch.h, entdec.h, mfrngcod.h
celt/entenc.c -> os_support.h, arch.h, entenc.h, mfrngcod.h
celt/kiss_fft.c -> _kiss_fft_guts.h, arch.h, os_support.h, mathops.h, stack_alloc.h
celt/laplace.c -> laplace.h, mathops.h
celt/mathops.c -> float_cast.h, mathops.h
celt/mdct.c -> mdct.h, kiss_fft.h, _kiss_fft_guts.h, os_support.h, mathops.h, stack_alloc.h, mips/mdct_mipsr1.h
celt/modes.c -> celt.h, modes.h, rate.h, os_support.h, stack_alloc.h, quant_bands.h, cpu_support.h, static_modes_fixed.h, static_modes_float.h
celt/pitch.c -> pitch.h, os_support.h, modes.h, stack_alloc.h, mathops.h, celt_lpc.h
celt/quant_bands.c -> quant_bands.h, laplace.h, os_support.h, arch.h, mathops.h, stack_alloc.h, rate.h
celt/rate.c -> modes.h, cwrs.h, arch.h, os_support.h, entcode.h, rate.h, quant_bands.h
celt/vq.c -> mathops.h, cwrs.h, vq.h, arch.h, os_support.h, bands.h, rate.h, pitch.h, SigProc_FIX.h, macros.h
silk/A2NLSF.c -> SigProc_FIX.h, tables.h
silk/CNG.c -> main.h, stack_alloc.h
silk/HP_variable_cutoff.c -> main_FIX.h, main_FLP.h, tuning_parameters.h
silk/LPC_analysis_filter.c -> SigProc_FIX.h, celt_lpc.h
silk/LPC_fit.c -> SigProc_FIX.h
silk/LPC_inv_pred_gain.c -> SigProc_FIX.h, define.h
silk/LP_variable_cutoff.c -> main.h
silk/NLSF2A.c -> SigProc_FIX.h, tables.h
silk/NLSF_VQ.c -> main.h
silk/NLSF_VQ_weights_laroia.c -> define.h, SigProc_FIX.h
silk/NLSF_decode.c -> main.h
silk/NLSF_del_dec_quant.c -> main.h
silk/NLSF_encode.c -> main.h, stack_alloc.h
silk/NLSF_stabilize.c -> SigProc_FIX.h
silk/NLSF_unpack.c -> main.h
silk/NSQ.c -> main.h, stack_alloc.h, NSQ.h
silk/NSQ_del_dec.c -> main.h, stack_alloc.h, NSQ.h
silk/PLC.c -> main.h, stack_alloc.h, PLC.h, lpcnet.h
silk/VAD.c -> main.h, stack_alloc.h
silk/VQ_WMat_EC.c -> main.h
silk/ana_filt_bank_1.c -> SigProc_FIX.h
silk/biquad_alt.c -> SigProc_FIX.h
silk/bwexpander.c -> SigProc_FIX.h
silk/bwexpander_32.c -> SigProc_FIX.h
silk/check_control_input.c -> main.h, control.h, errors.h
silk/code_signs.c -> main.h
silk/control_SNR.c -> main.h, tuning_parameters.h
silk/control_audio_bandwidth.c -> main.h, tuning_parameters.h
silk/control_codec.c -> main_FIX.h, main_FLP.h, stack_alloc.h, tuning_parameters.h, pitch_est_defines.h
silk/debug.c -> debug.h, SigProc_FIX.h
silk/dec_API.c -> API.h, main.h, stack_alloc.h, os_support.h, osce.h, osce_structs.h, osce_features.h
silk/decode_core.c -> main.h, stack_alloc.h
silk/decode_frame.c -> main.h, stack_alloc.h, PLC.h, osce.h
silk/decode_indices.c -> main.h
silk/decode_parameters.c -> main.h
silk/decode_pitch.c -> SigProc_FIX.h, pitch_est_defines.h
silk/decode_pulses.c -> main.h
silk/decoder_set_fs.c -> main.h
silk/enc_API.c -> define.h, API.h, control.h, typedef.h, stack_alloc.h, structs.h, tuning_parameters.h, main_FIX.h, main_FLP.h, dred_encoder.h
silk/encode_indices.c -> main.h
silk/encode_pulses.c -> main.h, stack_alloc.h
silk/fixed/LTP_analysis_filter_FIX.c -> main_FIX.h
silk/fixed/LTP_scale_ctrl_FIX.c -> main_FIX.h
silk/fixed/apply_sine_window_FIX.c -> SigProc_FIX.h
silk/fixed/autocorr_FIX.c -> SigProc_FIX.h, celt_lpc.h
silk/fixed/burg_modified_FIX.c -> SigProc_FIX.h, define.h, tuning_parameters.h, pitch.h
silk/fixed/corrMatrix_FIX.c -> main_FIX.h
silk/fixed/encode_frame_FIX.c -> main_FIX.h, stack_alloc.h, tuning_parameters.h
silk/fixed/find_LPC_FIX.c -> main_FIX.h, stack_alloc.h, tuning_parameters.h
silk/fixed/find_LTP_FIX.c -> main_FIX.h, tuning_parameters.h
silk/fixed/find_pitch_lags_FIX.c -> main_FIX.h, stack_alloc.h, tuning_parameters.h
silk/fixed/find_pred_coefs_FIX.c -> main_FIX.h, stack_alloc.h
silk/fixed/k2a_FIX.c -> SigProc_FIX.h
silk/fixed/k2a_Q16_FIX.c -> SigProc_FIX.h
silk/fixed/noise_shape_analysis_FIX.c -> main_FIX.h, stack_alloc.h, tuning_parameters.h
silk/fixed/pitch_analysis_core_FIX.c -> SigProc_FIX.h, pitch_est_defines.h, stack_alloc.h, debug.h, pitch.h
silk/fixed/process_gains_FIX.c -> main_FIX.h, tuning_parameters.h
silk/fixed/regularize_correlations_FIX.c -> main_FIX.h
silk/fixed/residual_energy16_FIX.c -> main_FIX.h
silk/fixed/residual_energy_FIX.c -> main_FIX.h, stack_alloc.h
silk/fixed/schur64_FIX.c -> SigProc_FIX.h
silk/fixed/schur_FIX.c -> SigProc_FIX.h
silk/fixed/vector_ops_FIX.c -> SigProc_FIX.h, pitch.h
silk/fixed/warped_autocorrelation_FIX.c -> main_FIX.h, mips/warped_autocorrelation_FIX_mipsr1.h
silk/float/LPC_analysis_filter_FLP.c -> main_FLP.h
silk/float/LPC_inv_pred_gain_FLP.c -> SigProc_FIX.h, SigProc_FLP.h, define.h
silk/float/LTP_analysis_filter_FLP.c -> main_FLP.h
silk/float/LTP_scale_ctrl_FLP.c -> main_FLP.h
silk/float/apply_sine_window_FLP.c -> main_FLP.h
silk/float/autocorrelation_FLP.c -> typedef.h, SigProc_FLP.h
silk/float/burg_modified_FLP.c -> SigProc_FLP.h, tuning_parameters.h, define.h
silk/float/bwexpander_FLP.c -> SigProc_FLP.h
silk/float/corrMatrix_FLP.c -> main_FLP.h
silk/float/encode_frame_FLP.c -> main_FLP.h, tuning_parameters.h, stack_alloc.h
silk/float/energy_FLP.c -> SigProc_FLP.h
silk/float/find_LPC_FLP.c -> define.h, main_FLP.h, tuning_parameters.h
silk/float/find_LTP_FLP.c -> main_FLP.h, tuning_parameters.h
silk/float/find_pitch_lags_FLP.c -> main_FLP.h, tuning_parameters.h
silk/float/find_pred_coefs_FLP.c -> main_FLP.h
silk/float/inner_product_FLP.c -> SigProc_FLP.h
silk/float/k2a_FLP.c -> SigProc_FLP.h
silk/float/noise_shape_analysis_FLP.c -> main_FLP.h, tuning_parameters.h
silk/float/pitch_analysis_core_FLP.c -> SigProc_FLP.h, SigProc_FIX.h, pitch_est_defines.h, pitch.h
silk/float/process_gains_FLP.c -> main_FLP.h, tuning_parameters.h
silk/float/regularize_correlations_FLP.c -> main_FLP.h
silk/float/residual_energy_FLP.c -> main_FLP.h
silk/float/scale_copy_vector_FLP.c -> SigProc_FLP.h
silk/float/scale_vector_FLP.c -> SigProc_FLP.h
silk/float/schur_FLP.c -> SigProc_FLP.h
silk/float/sort_FLP.c -> typedef.h, SigProc_FLP.h
silk/float/warped_autocorrelation_FLP.c -> main_FLP.h
silk/float/wrappers_FLP.c -> main_FLP.h
silk/gain_quant.c -> main.h
silk/init_decoder.c -> main.h, osce.h, structs.h
silk/init_encoder.c -> main_FIX.h, main_FLP.h, tuning_parameters.h, cpu_support.h, dred_encoder.h
silk/inner_prod_aligned.c -> SigProc_FIX.h
silk/interpolate.c -> main.h
silk/lin2log.c -> SigProc_FIX.h
silk/log2lin.c -> SigProc_FIX.h
silk/pitch_est_tables.c -> typedef.h, pitch_est_defines.h
silk/process_NLSFs.c -> main.h
silk/quant_LTP_gains.c -> main.h, tuning_parameters.h
silk/resampler.c -> resampler_private.h
silk/resampler_down2.c -> SigProc_FIX.h, resampler_rom.h
silk/resampler_down2_3.c -> SigProc_FIX.h, resampler_private.h, stack_alloc.h
silk/resampler_private_AR2.c -> SigProc_FIX.h, resampler_private.h
silk/resampler_private_IIR_FIR.c -> SigProc_FIX.h, resampler_private.h, stack_alloc.h
silk/resampler_private_down_FIR.c -> SigProc_FIX.h, resampler_private.h, stack_alloc.h
silk/resampler_private_up2_HQ.c -> SigProc_FIX.h, resampler_private.h
silk/resampler_rom.c -> resampler_private.h
silk/shell_coder.c -> main.h
silk/sigm_Q15.c -> SigProc_FIX.h
silk/sort.c -> SigProc_FIX.h
silk/stereo_LR_to_MS.c -> main.h, stack_alloc.h
silk/stereo_MS_to_LR.c -> main.h
silk/stereo_decode_pred.c -> main.h
silk/stereo_encode_pred.c -> main.h
silk/stereo_find_predictor.c -> main.h
silk/stereo_quant_pred.c -> main.h
silk/sum_sqr_shift.c -> SigProc_FIX.h
silk/table_LSF_cos.c -> tables.h
silk/tables_LTP.c -> tables.h
silk/tables_NLSF_CB_NB_MB.c -> tables.h
silk/tables_NLSF_CB_WB.c -> tables.h
silk/tables_gain.c -> tables.h
silk/tables_other.c -> structs.h, define.h, tables.h
silk/tables_pitch_lag.c -> tables.h
silk/tables_pulses_per_block.c -> tables.h
src/analysis.c -> mathops.h, kiss_fft.h, celt.h, modes.h, arch.h, quant_bands.h, analysis.h, mlp.h, stack_alloc.h, float_cast.h
src/extensions.c -> opus_types.h, opus_defines.h, arch.h, os_support.h, opus_private.h
src/mapping_matrix.c -> arch.h, float_cast.h, opus_private.h, opus_defines.h, mapping_matrix.h
src/mlp.c -> opus_types.h, opus_defines.h, arch.h, mlp.h
src/mlp_data.c -> mlp.h
src/opus.c -> opus.h, mathops.h, opus_private.h
src/opus_decoder.c -> celt.h, opus.h, entdec.h, modes.h, API.h, stack_alloc.h, float_cast.h, opus_private.h, os_support.h, structs.h, define.h, mathops.h, cpu_support.h, dred_rdovae_dec_data.h, dred_rdovae_dec.h, osce.h
src/opus_encoder.c -> celt.h, entenc.h, modes.h, API.h, stack_alloc.h, float_cast.h, opus.h, arch.h, pitch.h, opus_private.h, os_support.h, cpu_support.h, analysis.h, mathops.h, tuning_parameters.h, dred_coding.h, fixed/structs_FIX.h, float/structs_FLP.h
src/opus_multistream.c -> opus_multistream.h, opus.h, opus_private.h, stack_alloc.h, float_cast.h, os_support.h
src/opus_multistream_decoder.c -> opus_multistream.h, opus.h, opus_private.h, stack_alloc.h, float_cast.h, os_support.h
src/opus_multistream_encoder.c -> opus_multistream.h, opus.h, opus_private.h, stack_alloc.h, float_cast.h, os_support.h, mathops.h, mdct.h, modes.h, bands.h, quant_bands.h, pitch.h
src/opus_projection_decoder.c -> mathops.h, os_support.h, opus_private.h, opus_defines.h, opus_projection.h, opus_multistream.h, mapping_matrix.h, stack_alloc.h
src/opus_projection_encoder.c -> mathops.h, os_support.h, opus_private.h, opus_defines.h, opus_projection.h, opus_multistream.h, stack_alloc.h, mapping_matrix.h
src/repacketizer.c -> opus.h, opus_private.h, os_support.h, stack_alloc.h
```

## Appendix B — reproducing the reference build

The port needs a C reference to differentially test against. The exact recipe
for the configuration this document targets:

```sh
cd /home/m-kikuchi/projects/oprs/reference/opus-1.6.1
mkdir -p /tmp/opus-ref && cd /tmp/opus-ref
../configure \
    --enable-fixed-point \
    --disable-float-api \
    --disable-rtcd --disable-intrinsics --disable-asm \
    --disable-doc --disable-extra-programs \
    CFLAGS="-O2 -fno-strict-aliasing -fwrapv"
```

Notes:

* `--enable-fixed-point` sets `FIXED_POINT`; the autotools default also sets
  **`ENABLE_RES24`** (`configure.ac:150-156`). Add `--disable-fixed-res24` for the
  16-bit-internal-resolution variant and build/verify *both* if the port wants to
  support both.
* `--disable-float-api` removes `src/analysis.c`/`mlp.c`/`mlp_data.c`; **drop
  this flag** if the port must reproduce the analysis-driven CELT decisions, in
  which case the float API and its tables are in scope.
* **Do not use CMake or Meson for the reference build**: neither defines
  `ENABLE_RES24` nor offers `ENABLE_QEXT`, so they produce a different
  fixed-point bitstream from the autotools default
  (`CMakeLists.txt` has no `ENABLE_RES24` occurrence; `meson.build:145-153` lists
  only `fixed-point`, `fixed-point-debug`, `custom-modes`, `float-approx`,
  `assertions`, `hardening`, `fuzzing`, `check-asm`).
* `-fwrapv` makes signed overflow wrap (its behaviour under `-O2` is already
  wrapping in practice, but `-fwrapv` makes it contractual). Do **not** add
  `-ffast-math`; `celt/arch.h:270-272` errors out on it unless `FLOAT_APPROX` is
  defined.
* `ENABLE_QEXT` is autotools-only (`--enable-qext`) and is **off** by default.
  Turning it on changes `celt_coef`/`COEF_SHIFT`, adds the 96 kHz mode and a
  second set of mode tables, and modifies `celt/rate.c`, `celt/bands.c`,
  `celt/modes.c`, `celt/vq.c`, `celt/celt_{en,de}coder.c`, `silk/define.h`,
  `silk/check_control_input.c`, `silk/resampler.c` and all of `src/`. Treat it as
  a separate port target.

Bit-exact regression testing: `opus_demo` (built with
`--enable-extra-programs`) in `-e/-d` mode compared byte-for-byte, and the
official Opus test vectors. For decode-only validation, compare the decoder
output PCM produced by the C reference against the Rust decoder on a corpus of
packets; for the encoder, compare the compressed bytes (any difference is a
failure, and `opus_compare` can localise it by band).

## Appendix C — the reference `config.h` macro set

For the configuration above, `config.h` should contain (only the
core-relevant entries shown; the full file comes from `config.h.in`):

```c
#define FIXED_POINT 1
#define ENABLE_RES24 1        /* unless --disable-fixed-res24 */
#define DISABLE_FLOAT_API 1   /* if --disable-float-api */
#define OPUS_BUILD 1
/* NOT defined: ENABLE_QEXT, ENABLE_DRED, ENABLE_DEEP_PLC, ENABLE_OSCE,
   ENABLE_DRED, CUSTOM_MODES, ENABLE_OPUS_CUSTOM_API, FLOAT_APPROX,
   OPUS_ARM_*, OPUS_X86_*, OPUS_HAVE_RTCD, USE_SMALL_DIV_TABLE,
   ENABLE_ASSERTIONS, FIXED_DEBUG, FUZZING, ENABLE_LOSSGEN,
   OPUS_CHECK_ASM, DISABLE_DOT_PROD, DISABLE_DEBUG_FLOAT */
```

The exact set the port must key off, grouped by consequence:

| Macro | Effect on the core |
|---|---|
| `FIXED_POINT` | **Required.** Selects the integer type system and all arithmetic macros. |
| `ENABLE_RES24` | 24-bit internal resolution (`opus_res = i32`, `RES_SHIFT 8`, `MAX_ENCODING_DEPTH 24`). Autotools default on. |
| `ENABLE_QEXT` | Scalable quality extension: `celt_coef`/`COEF_SHIFT`, 96 kHz mode, extra allocation. Off by default; autotools-only switch. |
| `DISABLE_FLOAT_API` | Removes the analysis/MLP layer; changes encoder decisions if left enabled. |
| `ENABLE_DEEP_PLC`, `ENABLE_DRED`, `ENABLE_OSCE*` | Add DNN state and `#ifdef` branches in `silk/PLC.c`, `silk/dec_API.c`, `celt/celt_decoder.c`, `src/opus_decoder.c`, `src/opus_encoder.c`. Keep off. |
| `CUSTOM_MODES`, `ENABLE_OPUS_CUSTOM_API` | Export the custom API and (for `CUSTOM_MODES`) allow 44.1 kHz/2^n frames; adds `toOpusTable`/`fromOpusTable`. |
| `SMALL_FOOTPRINT`, `CWRS_EXTRA_ROWS` | Embedder-only macros that change `celt/cwrs.c`'s table sizes and code paths. Default off. |
| `ENABLE_ASSERTIONS`, `ENABLE_HARDENING`, `FIXED_DEBUG`, `FUZZING`, `OPUS_CHECK_ASM` | Diagnostics / testing only; must be off for the reference, but `ENABLE_HARDENING` is on by default and adds validation functions. |
| `USE_SMALL_DIV_TABLE` | Switches `celt_udiv`/`celt_sudiv` to a reciprocal table; defined by `OPUS_ARM_ASM`. |
| `OPUS_ARM_*`, `OPUS_X86_*`, `OPUS_HAVE_RTCD`, `HAVE_ARM_NE10` | SIMD dispatch only; irrelevant to a portable port (all of `celt/{x86,arm,mips}/`, `silk/{x86,arm,mips}/`, `silk/fixed/{x86,arm}/`, `silk/float/x86/` is deleted). |
| `USE_ALLOCA`, `VAR_ARRAYS`, `NONTHREADSAFE_PSEUDOSTACK` | Scratch-buffer placement only; bit-exactness-neutral. |

## Appendix D — how to use this document

| Question | Section |
|---|---|
| Which build configuration should the port target, and what does each option do? | §1 |
| Which files must I port, and which are SIMD/demo noise? | §2 |
| In what order should I port, and why? | §3 |
| What exactly does the opus layer need from celt/silk? | §4 |
| What state must the Rust structs hold (field by field)? | §5 |
| What are the exact constant tables, and where do they come from? | §6 |
| What will silently break bit-exactness, and what is the exact C? | §7 |
| What calls what? | Appendix A |
| How do I build the C reference to diff against? | Appendix B, C |

Companion generators (all in `docs/`, all read-only against the reference tree):

| Script | Produces |
|---|---|
| `gen_s2.py` | §2 file inventory (line counts, include lists, build variant, classification) |
| `gen_graph.py` | Appendix A call graph |
| `gen_s4.py` | §4 cross-layer API surface |
| `gen_s5.py` | §5 verbatim state structs |
| `gen_s6.py` | §6 data-table manifest (`_s6_head.py` + `_s6_tail.py`) |
| `gen_s7.py` | §7 hazard catalogue with verbatim code |

Regenerate with:

```sh
cd /home/m-kikuchi/projects/oprs/docs
python3 gen_s2.py && python3 gen_graph.py && python3 gen_s4.py \
  && python3 gen_s5.py && python3 gen_s6.py && python3 gen_s7.py
python3 build_map.py     # concatenates the sections into libopus-map.md
```

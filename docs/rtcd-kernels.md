# RTCD / SIMD kernel specification — libopus 1.6.1, **float** build, **arch 4 (AVX2)**

Specification of every runtime-dispatched (RTCD) function that is **active** in the reference build
described below, with the exact arithmetic of the implementation that is actually selected at
`arch = 4`, sufficient to re-implement it bit-exactly in Rust.

Reference tree: `/home/m-kikuchi/projects/oprs/reference/opus-1.6.1` (read-only).

Everything in this document was taken from the C sources *and* cross-checked against the artefacts of
the reference build (generated `config.h`, libtool compile flags, symbol tables and relocation records
of the object files). The cross-checks are listed in §1.4 so they can be re-run.

---

## 1. Build configuration

### 1.1 `config.h` (root of the reference tree)

```
#define OPUS_HAVE_RTCD 1                 /* line 176 */
#define OPUS_X86_MAY_HAVE_AVX2 1         /* 179 */
#define OPUS_X86_MAY_HAVE_SSE 1          /* 182 */
#define OPUS_X86_MAY_HAVE_SSE2 1         /* 185 */
#define OPUS_X86_MAY_HAVE_SSE4_1 1       /* 188 */
/* #undef OPUS_X86_PRESUME_AVX2 */      /* 191 */
#define OPUS_X86_PRESUME_SSE 1           /* 194  <-- ! */
#define OPUS_X86_PRESUME_SSE2 1          /* 197  <-- ! */
/* #undef OPUS_X86_PRESUME_SSE4_1 */    /* 200 */
#define CPU_INFO_BY_ASM 1                /* 5    (cpuid() via inline asm) */
/* #undef FIXED_POINT */                /* 62 */
/* #undef FLOAT_APPROX */               /* 65 */
#define CUSTOM_MODES 1
#define ENABLE_DEEP_PLC 1
#define ENABLE_DRED 1
#define ENABLE_HARDENING 1
#define ENABLE_OSCE 1
#define ENABLE_OSCE_BWE 1                /* 47  <-- see note below */
#define ENABLE_RES24 1
/* #undef ENABLE_QEXT */                /* 53  */
/* #undef HAVE_ARM_NE10 */              /* 74  */
/* #undef OPUS_CHECK_ASM */             /* 173 */
/* #undef ENABLE_ASSERTIONS */          /* 26  */
/* #undef ENABLE_OPUS_CUSTOM_API */
```

> **Note / correction to the task statement:** in the checked-out reference tree `ENABLE_OSCE_BWE`
> **is** defined (`config.h:47`, emitted by `./configure --enable-osce ...`, see `config.log:7`).
> It has no influence on the x86 RTCD tables documented here, but it does pull the `dnn/nndsp.c`
> code (and therefore `celt_pitch_xcorr`) onto the decode path — see §5.

`OPUS_CHECK_ASM` is **not** defined, so none of the `silk_assert(!memcmp(...))` cross-checks
against the C versions are compiled in; and none of the "this code is more correct" fallbacks are
switched back to the C idiom.

### 1.2 Per-file compiler flags (from `Makefile` / `config.log`)

```
X86_SSE_CFLAGS    = -msse        but OPUS_X86_SSE_CFLAGS    = ''      (x86-64 baseline already has SSE/SSE2)
X86_SSE2_CFLAGS   = -msse2       but OPUS_X86_SSE2_CFLAGS   = ''
X86_SSE4_1_CFLAGS = -msse4.1     and OPUS_X86_SSE4_1_CFLAGS = -msse4.1
X86_AVX2_CFLAGS   = -mavx -mfma -mavx2   and OPUS_X86_AVX2_CFLAGS = -mavx -mfma -mavx2
base CFLAGS       = -march=x86-64 -mtune=generic -O2 -pipe -fvisibility=hidden ...
```

Consequences that matter for bit-exactness:

* `OPUS_X86_PRESUME_SSE`/`PRESUME_SSE2` are set **because** the baseline already implies SSE/SSE2,
  therefore the SSE/SSE2 kernels are **not** put behind an RTCD table in the CELT maps; the header
  macros call them *directly at every arch value* (§2.3).
* Generic C files (`celt.c`, `pitch.c`, `vq.c`, `celt_lpc.c`, …) are compiled **without** `-mfma`,
  so no compiler-generated FMA contraction happens there. Only `pitch_avx.c`,
  `silk/x86/NSQ_del_dec_avx2.c` and `silk/float/x86/inner_product_FLP_avx2.c` are compiled with
  `-mfma`. FMA fusion in those files is always written explicitly with `*_fmadd*` intrinsics.
* `silk/macros.h` arithmetic is the `OPUS_FAST_INT64 == 1` variant (x86-64, `celt/arch.h:123-127`):
  ```
  silk_SMULWB(a,b)  = (opus_int32)(((a) * (opus_int64)((opus_int16)(b))) >> 16)     /* macros.h:43 */
  silk_SMLAWB(a,b,c)= (opus_int32)((a) + (((b) * (opus_int64)((opus_int16)(c))) >> 16)) /* :50 */
  silk_SMULWT(a,b)  = (opus_int32)(((a) * (opus_int64)((b) >> 16)) >> 16)           /* :57 */
  silk_SMLAWT(a,b,c)= (opus_int32)((a) + (((b) * ((opus_int64)(c) >> 16)) >> 16))   /* :64 */
  silk_SMULWW(a,b)  = (opus_int32)(((opus_int64)(a) * (b)) >> 16)                   /* :86 */
  silk_SMULBB(a,b)  = (opus_int32)((opus_int16)(a)) * (opus_int32)((opus_int16)(b)) /* :70 */
  silk_RSHIFT_ROUND(a,s) = (s==1) ? ((a)>>1)+((a)&1) : (((a)>>((s)-1))+1)>>1       /* SigProc_FIX.h:527 */
  silk_SAT16(a), silk_ADD_SAT32(a,b) ...                                            /* SigProc_FIX.h:470, macros.h:99 */
  ```

### 1.3 `OPUS_ARCHMASK` and the arch domain

`celt/cpu_support.h:47-62` — the x86/RTCD branch is taken (`OPUS_HAVE_RTCD` && SSE4.1 may-have and not
presume), so:

```c
/* We currently support 5 x86 variants:
 * arch[0] -> non-sse
 * arch[1] -> sse
 * arch[2] -> sse2
 * arch[3] -> sse4.1
 * arch[4] -> avx
 */
#define OPUS_ARCHMASK 7
int opus_select_arch(void);
```

Every table is declared `[OPUS_ARCHMASK + 1]` = **8 entries** but only 5 are initialised; entries
5, 6 and 7 are zero. This is visible in the built object files
(`nm -S`: `PITCH_XCORR_IMPL 00000040` = 64 bytes = 8 pointers, relocation records only for offsets
0x00…0x20). Dispatch is by `(arch) & OPUS_ARCHMASK`; since `opus_select_arch()` never returns more
than 4, a NULL entry can never be reached.

### 1.4 CPU detection — the one and only path
There is **one** CPU-detection implementation, `opus_select_arch()` in `celt/x86/x86cpu.c`
(shared by CELT and SILK). `silk/x86/x86_silk_map.c` performs **no** detection of its own: it only
`#include "celt/x86/x86cpu.h"` (line 32) and uses the `MAY_HAVE_*` macros. SILK therefore sees exactly
the same integer as CELT.

`celt/x86/x86cpu.c:102-182`:

```c
typedef struct CPU_Feature{
    /*  SIMD: 128-bit */
    int HW_SSE;  int HW_SSE2;  int HW_SSE41;
    /*  SIMD: 256-bit */
    int HW_AVX2;
} CPU_Feature;

static void opus_cpu_feature_check(CPU_Feature *cpu_feature)
{
    unsigned int info[4];  unsigned int nIds = 0;
    cpuid(info, 0);  nIds = info[0];
    if (nIds >= 1){
        cpuid(info, 1);
        cpu_feature->HW_SSE   = (info[3] & (1 << 25)) != 0;
        cpu_feature->HW_SSE2  = (info[3] & (1 << 26)) != 0;
        cpu_feature->HW_SSE41 = (info[2] & (1 << 19)) != 0;
        cpu_feature->HW_AVX2  = (info[2] & (1 << 28)) != 0 && (info[2] & (1 << 12)) != 0;
        if (cpu_feature->HW_AVX2 && nIds >= 7) {
            cpuid(info, 7);
            cpu_feature->HW_AVX2 = cpu_feature->HW_AVX2 && (info[1] & (1 << 5)) != 0;
        } else {
            cpu_feature->HW_AVX2 = 0;
        }
    } else { ...all zero... }
}

static int opus_select_arch_impl(void)
{
    ...
    arch = 0;
    if (!cpu_feature.HW_SSE)   return arch;   arch++;   /* 1 */
    if (!cpu_feature.HW_SSE2)  return arch;   arch++;   /* 2 */
    if (!cpu_feature.HW_SSE41) return arch;   arch++;   /* 3 */
    if (!cpu_feature.HW_AVX2)  return arch;   arch++;   /* 4 */
    return arch;
}
int opus_select_arch(void) { int arch = opus_select_arch_impl(); ...FUZZING only... return arch; }
```

**Answer for a CPU with SSE,SSE2,SSSE3,SSE4.1,AVX,AVX2,FMA:**

* `celt` path (`opus_select_arch()`): **`arch = 4`**.
* `silk` path (`silk/x86/x86_silk_map.c` + `opus_select_arch()`): **`arch = 4`** (identical code path;
  SILK itself has no detection code).

Note the AVX2 test is `leaf1.ECX bit 28 (AVX) && leaf1.ECX bit 12 (FMA) && leaf7.EBX bit 5 (AVX2)`
and additionally requires `nIds >= 7`. So *FMA is implicitly required* to reach arch 4, although the
value 4 is documented as "avx". `FUZZING` (not defined here) is the only way to get another value.

The `MAY_HAVE_*` macros of `celt/x86/x86cpu.h:31-53` therefore expand as follows in this build:

| macro | condition | expansion here |
|---|---|---|
| `MAY_HAVE_SSE(name)` | `OPUS_X86_MAY_HAVE_SSE` defined | `name ## _sse` |
| `MAY_HAVE_SSE2(name)` | `OPUS_X86_MAY_HAVE_SSE2` defined | `name ## _sse2` |
| `MAY_HAVE_SSE4_1(name)` | `OPUS_X86_MAY_HAVE_SSE4_1` defined | `name ## _sse4_1` |
| `MAY_HAVE_AVX2(name)` | `OPUS_X86_MAY_HAVE_AVX2` defined | `name ## _avx2` |

`celt/x86/x86cpu.h:55-61` declares `opus_select_arch()` only when RTCD is on and at least one ISA is
not presumed — true here.

---

## 2. Complete inventory of active dispatch in this configuration

### 2.1 The **only** CELT RTCD array: `PITCH_XCORR_IMPL`

`celt/x86/x86_celt_map.c:93-110` (float branch, `MAY_HAVE_AVX2` && !`PRESUME_AVX2`):

```c
void (*const PITCH_XCORR_IMPL[OPUS_ARCHMASK + 1])(
         const float *_x,
         const float *_y,
         float *xcorr,
         int len,
         int max_pitch,
         int arch
) = {
  celt_pitch_xcorr_c,                /* non-sse */
  celt_pitch_xcorr_c,
  celt_pitch_xcorr_c,
  celt_pitch_xcorr_c,
  MAY_HAVE_AVX2(celt_pitch_xcorr)    /* avx  == celt_pitch_xcorr_avx2 */
};
```

Selected entries: **arch 3 → `celt_pitch_xcorr_c`**, **arch 4 → `celt_pitch_xcorr_avx2`**.

Verified empirically (`objdump -r celt/x86/x86_celt_map.o`):

```
0000000000000000 R_X86_64_64  celt_pitch_xcorr_c
0000000000000008 R_X86_64_64  celt_pitch_xcorr_c
0000000000000010 R_X86_64_64  celt_pitch_xcorr_c
0000000000000018 R_X86_64_64  celt_pitch_xcorr_c
0000000000000020 R_X86_64_64  celt_pitch_xcorr_avx2
```

Consumer macro (`celt/x86/pitch_sse.h:189-209`):

```c
void celt_pitch_xcorr_avx2(const float *_x, const float *_y, float *xcorr, int len, int max_pitch, int arch);

#if defined(OPUS_X86_PRESUME_AVX2)
# define OVERRIDE_PITCH_XCORR
# define celt_pitch_xcorr celt_pitch_xcorr_avx2
#elif defined(OPUS_HAVE_RTCD) && defined(OPUS_X86_MAY_HAVE_AVX2)
# define OVERRIDE_PITCH_XCORR
extern void (*const PITCH_XCORR_IMPL[OPUS_ARCHMASK + 1])( ... int arch);
# define celt_pitch_xcorr(_x, _y, xcorr, len, max_pitch, arch) \
    ((*PITCH_XCORR_IMPL[(arch) & OPUS_ARCHMASK])(_x, _y, xcorr, len, max_pitch, arch))
#endif
```

### 2.2 All CELT functions that are **not** dispatched in this build

These hooks exist in the sources but are inert here; a Rust port must **not** look for tables for them:

| hook / table | where declared | why inactive |
|---|---|---|
| `CELT_FIR_IMPL` | `x86_celt_map.c:42-57`, `x86/celt_lpc_sse.h:35` | inside `#if defined(FIXED_POINT)` |
| `XCORR_KERNEL_IMPL` (fixed) | `x86_celt_map.c:59-70` | same |
| `CELT_INNER_PROD_IMPL` (fixed) | `x86_celt_map.c:74-89` | same |
| `OP_PVQ_SEARCH_IMPL` (RTCD table) | `x86_celt_map.c:174-184`, `vq_sse.h:40-49` | guarded by `!defined(OPUS_X86_PRESUME_SSE2)`; **PRESUME_SSE2 is defined** |
| `XCORR_KERNEL_IMPL`, `CELT_INNER_PROD_IMPL`, `DUAL_INNER_PROD_IMPL`, `COMB_FILTER_CONST_IMPL` (float) | `x86_celt_map.c:113-172` | guarded by `!defined(OPUS_X86_PRESUME_SSE)`; **PRESUME_SSE is defined** |
| `celt_fir` / `celt_iir` / `_celt_lpc` overrides | `celt/celt_lpc.h`, `x86/celt_lpc_sse.h:35` | `FIXED_POINT` only |
| `CLT_MDCT_FORWARD/BACKWARD_IMPL`, `OPUS_FFT*` | `celt/mdct.h:78`, `celt/kiss_fft.h:169` | `HAVE_ARM_NE10` only (undefined) |
| `silk_inner_prod16`, `silk_burg_modified` | `silk/x86/x86_silk_map.c:43-59, 141-161` | inside `#if defined(FIXED_POINT)` |

Verified: `nm celt/x86/x86_celt_map.o` exports **only** `PITCH_XCORR_IMPL`; `celt/x86/pitch_sse2.o`,
`celt/x86/pitch_sse4_1.o`, `celt/x86/celt_lpc_sse4_1.o` are empty objects;
`nm celt/celt.o` has **no** `comb_filter_const_c` symbol (that function is not even compiled — see §2.3).

### 2.2b Empirical FMA check

`objdump -d <obj> | grep -cE 'vfmadd|vfnmadd'` over the built objects gives

```
celt/x86/pitch_sse.o                      0      (xcorr_kernel_sse, dual/celt_inner_prod, comb_filter_const)
celt/x86/vq_sse2.o                        0      (op_pvq_search_sse2)
celt/x86/pitch_avx.o                     16      (celt_pitch_xcorr_avx2: 8 loop + 8 masked-tail fmadds)
silk/x86/NSQ_sse4_1.o                     0
silk/x86/NSQ_del_dec_avx2.o               0      (integer only)
silk/float/x86/inner_product_FLP_avx2.o   3      (the 3 _mm256_fmadd_pd sites in the source)
```

So FMA appears in exactly two kernels and nowhere else; every other multiply+add is two separate
IEEE roundings.

### 2.3 The "presumed SSE/SSE2" overrides (no table, no arch dependence)

With `OPUS_X86_PRESUME_SSE`/`PRESUME_SSE2` defined, `celt/x86/pitch_sse.h` and `celt/x86/vq_sse.h`
short-circuit the dispatch macros to the SSE/SSE2 implementations **at every arch value, including 0**:

```c
/* pitch_sse.h:61-64 */
#elif defined(OPUS_X86_PRESUME_SSE) && !defined(FIXED_POINT)
#define OVERRIDE_XCORR_KERNEL
#define xcorr_kernel(x, y, sum, len, arch) \
    ((void)arch, xcorr_kernel_sse(x, y, sum, len))

/* pitch_sse.h:112-115 */
#elif defined(OPUS_X86_PRESUME_SSE) && !defined(FIXED_POINT)
#define OVERRIDE_CELT_INNER_PROD
#define celt_inner_prod(x, y, N, arch) \
    ((void)arch, celt_inner_prod_sse(x, y, N))

/* pitch_sse.h:150-157 */
#if defined(OPUS_X86_PRESUME_SSE)
#define OVERRIDE_DUAL_INNER_PROD
#define OVERRIDE_COMB_FILTER_CONST
# define dual_inner_prod(x, y01, y02, N, xy1, xy2, arch) \
    ((void)(arch),dual_inner_prod_sse(x, y01, y02, N, xy1, xy2))
# define comb_filter_const(y, x, T, N, g10, g11, g12, arch) \
    ((void)(arch),comb_filter_const_sse(y, x, T, N, g10, g11, g12))

/* vq_sse.h:34-38 */
#if defined(OPUS_X86_PRESUME_SSE2)
#define OVERRIDE_OP_PVQ_SEARCH
#define op_pvq_search(x, iy, K, N, arch) \
    (op_pvq_search_sse2(x, iy, K, N, arch))
#endif
```

Therefore the following are used **unconditionally**:

| public name | actual implementation | arch argument |
|---|---|---|
| `xcorr_kernel` | `xcorr_kernel_sse` | passed but ignored (`(void)arch`) |
| `celt_inner_prod` | `celt_inner_prod_sse` | passed but ignored |
| `dual_inner_prod` | `dual_inner_prod_sse` | passed but ignored |
| `comb_filter_const` | `comb_filter_const_sse` | passed but ignored |
| `celt_inner_prod_norm`, `celt_inner_prod_norm_shift` | same as `celt_inner_prod` (`vq.h:52-53`) | — |
| `op_pvq_search` | `op_pvq_search_sse2` | receives `arch` (and ignores it) |

Two important corollaries:

1. **`xcorr_kernel_c`, `celt_inner_prod_c`, `dual_inner_prod_c` (the `static OPUS_INLINE` functions in
   `celt/pitch.h:65,137,159`) are dead code in this build**: nothing expands to them. Likewise the
   `arch 0..3` entries `celt_pitch_xcorr_c` internally expand the same macros, so even the
   "non-SSE" entries use the SSE kernels and the SSE accumulation order.
2. `comb_filter_const_c` is **not compiled at all**. `celt/celt.c:96` is
   `#if !defined(OVERRIDE_COMB_FILTER_CONST) || defined(NON_STATIC_COMB_FILTER_CONST_C)` and the
   `NON_STATIC_COMB_FILTER_CONST_C` define lives only in the (not taken) `OPUS_HAVE_RTCD` branch of
   `pitch_sse.h:185`. Confirmed: no such symbol in `celt/celt.o`.

### 2.4 SILK RTCD arrays (`silk/x86/x86_silk_map.c`)

Guard: `#if defined(OPUS_HAVE_RTCD) && !defined(OPUS_X86_PRESUME_AVX2)` → **true**.

```c
/* line 61 */
opus_int (*const SILK_VAD_GETSA_Q8_IMPL[ OPUS_ARCHMASK + 1 ] )(
    silk_encoder_state *psEncC, const opus_int16   pIn[]
) = {
  silk_VAD_GetSA_Q8_c,                  /* non-sse */
  silk_VAD_GetSA_Q8_c,
  silk_VAD_GetSA_Q8_c,
  MAY_HAVE_SSE4_1( silk_VAD_GetSA_Q8 ), /* sse4.1  == silk_VAD_GetSA_Q8_sse4_1 */
  MAY_HAVE_SSE4_1( silk_VAD_GetSA_Q8 )  /* avx     == silk_VAD_GetSA_Q8_sse4_1 */
};

/* line 72 */
void (*const SILK_NSQ_IMPL[ OPUS_ARCHMASK + 1 ] )( ... ) = {
  silk_NSQ_c, silk_NSQ_c, silk_NSQ_c,
  MAY_HAVE_SSE4_1( silk_NSQ ),          /* sse4.1  == silk_NSQ_sse4_1 */
  MAY_HAVE_SSE4_1( silk_NSQ )           /* avx     == silk_NSQ_sse4_1 */
};

/* line 96 */
void (*const SILK_VQ_WMAT_EC_IMPL[ OPUS_ARCHMASK + 1 ] )( ... ) = {
  silk_VQ_WMat_EC_c, silk_VQ_WMat_EC_c, silk_VQ_WMat_EC_c,
  MAY_HAVE_SSE4_1( silk_VQ_WMat_EC ),   /* sse4.1  == silk_VQ_WMat_EC_sse4_1 */
  MAY_HAVE_SSE4_1( silk_VQ_WMat_EC )    /* avx     == silk_VQ_WMat_EC_sse4_1 */
};

/* line 117 */
void (*const SILK_NSQ_DEL_DEC_IMPL[ OPUS_ARCHMASK + 1 ] )( ... ) = {
  silk_NSQ_del_dec_c,                   /* non-sse */
  silk_NSQ_del_dec_c,
  silk_NSQ_del_dec_c,
  MAY_HAVE_SSE4_1( silk_NSQ_del_dec ),  /* sse4.1  == silk_NSQ_del_dec_sse4_1 */
  MAY_HAVE_AVX2( silk_NSQ_del_dec )     /* avx     == silk_NSQ_del_dec_avx2   */
};

/* line 165, #ifndef FIXED_POINT */
double (*const SILK_INNER_PRODUCT_FLP_IMPL[ OPUS_ARCHMASK + 1 ] )(
    const silk_float *data1, const silk_float *data2, opus_int dataSize
) = {
  silk_inner_product_FLP_c,                  /* non-sse */
  silk_inner_product_FLP_c,
  silk_inner_product_FLP_c,
  silk_inner_product_FLP_c, /* sse4.1 */
  MAY_HAVE_AVX2( silk_inner_product_FLP )  /* avx  == silk_inner_product_FLP_avx2 */
};
```

Verified empirically (`objdump -r silk/x86/x86_silk_map.o`, all five arrays, 0x40 bytes each):

```
SILK_INNER_PRODUCT_FLP_IMPL 0x00: c, c, c, c, silk_inner_product_FLP_avx2   (0x28-0x38 zero)
SILK_NSQ_DEL_DEC_IMPL       0x40: c, c, c, silk_NSQ_del_dec_sse4_1, silk_NSQ_del_dec_avx2 (0x68-0x78 zero)
SILK_VQ_WMAT_EC_IMPL        0x80: c, c, c, silk_VQ_WMat_EC_sse4_1, silk_VQ_WMat_EC_sse4_1
SILK_NSQ_IMPL               0xc0: c, c, c, silk_NSQ_sse4_1, silk_NSQ_sse4_1
SILK_VAD_GETSA_Q8_IMPL      0x100: c, c, c, silk_VAD_GetSA_Q8_sse4_1, silk_VAD_GetSA_Q8_sse4_1
```

**arch 3 vs arch 4 entries**

| function | arch 3 | arch 4 |
|---|---|---|
| `silk_VAD_GetSA_Q8` | `silk_VAD_GetSA_Q8_sse4_1` | `silk_VAD_GetSA_Q8_sse4_1` |
| `silk_NSQ` | `silk_NSQ_sse4_1` | `silk_NSQ_sse4_1` |
| `silk_VQ_WMat_EC` | `silk_VQ_WMat_EC_sse4_1` | `silk_VQ_WMat_EC_sse4_1` |
| `silk_NSQ_del_dec` | `silk_NSQ_del_dec_sse4_1` | **`silk_NSQ_del_dec_avx2`** |
| `silk_inner_product_FLP` | `silk_inner_product_FLP_c` | **`silk_inner_product_FLP_avx2`** |

Consumer macros (`silk/x86/main_sse.h`), all reached through the `OPUS_HAVE_RTCD` branch because
`PRESUME_SSE4_1`/`PRESUME_AVX2` are undefined:

```c
/* main_sse.h:211-215 */
#   define OVERRIDE_silk_NSQ_del_dec
#   define silk_NSQ_del_dec(...) \
    ((*SILK_NSQ_DEL_DEC_IMPL[(arch) & OPUS_ARCHMASK])(...))

/* main_sse.h:266-268 */
#   define OVERRIDE_silk_VAD_GetSA_Q8
#   define silk_VAD_GetSA_Q8(psEnC, pIn, arch) \
      ((*SILK_VAD_GETSA_Q8_IMPL[(arch) & OPUS_ARCHMASK])(psEnC, pIn))

/* main_sse.h:77-81 / :131-135 */
#   define silk_VQ_WMat_EC(...)  ((*SILK_VQ_WMAT_EC_IMPL[(arch) & OPUS_ARCHMASK])(...))
#   define silk_NSQ(...)         ((*SILK_NSQ_IMPL[(arch) & OPUS_ARCHMASK])(...))

/* main_sse.h:286-293 */
#define OVERRIDE_inner_product_FLP
#define silk_inner_product_FLP(data1, data2, dataSize, arch) \
    ((void)arch,(*SILK_INNER_PRODUCT_FLP_IMPL[(arch) & OPUS_ARCHMASK])(data1, data2, dataSize))
```

### 2.5 Complete list of functions that must be re-implemented for this target

```
CELT (float):
  xcorr_kernel            -> xcorr_kernel_sse          (celt/x86/pitch_sse.c:43)     [always]
  celt_inner_prod         -> celt_inner_prod_sse       (celt/x86/pitch_sse.c:108)    [always]
  dual_inner_prod         -> dual_inner_prod_sse       (celt/x86/pitch_sse.c:79)     [always]
  comb_filter_const       -> comb_filter_const_sse     (celt/x86/pitch_sse.c:133)    [always]
  op_pvq_search           -> op_pvq_search_sse2        (celt/x86/vq_sse2.c:44)       [always]
  celt_pitch_xcorr        -> celt_pitch_xcorr_avx2     (celt/x86/pitch_avx.c:86)     [arch 4]
SILK (encoder only):
  silk_VAD_GetSA_Q8       -> silk_VAD_GetSA_Q8_sse4_1  (silk/x86/VAD_sse4_1.c:45)          [arch 3,4]
  silk_VQ_WMat_EC         -> silk_VQ_WMat_EC_sse4_1    (silk/x86/VQ_WMat_EC_sse4_1.c:39)   [arch 3,4]
  silk_NSQ                -> silk_NSQ_sse4_1           (silk/x86/NSQ_sse4_1.c:74)          [arch 3,4]
  silk_NSQ_del_dec        -> silk_NSQ_del_dec_sse4_1   (silk/x86/NSQ_del_dec_sse4_1.c:116) [arch 3]
                          -> silk_NSQ_del_dec_avx2     (silk/x86/NSQ_del_dec_avx2.c:364)   [arch 4]
  silk_inner_product_FLP  -> silk_inner_product_FLP_avx2 (silk/float/x86/inner_product_FLP_avx2.c:38) [arch 4]
```

---

## 3. CELT kernels — exact arithmetic

Typedefs in this build (`celt/arch.h:258-268`): `opus_val16 = opus_val32 = opus_val64 = float`,
`celt_sig = celt_norm = celt_ener = celt_glog = opus_res = celt_coef = float`.
Float macros (`celt/arch.h:302-371`): `MAC16_16(c,a,b) = ((c)+(opus_val32)(a)*(opus_val32)(b))`,
`MULT16_32_Q15(a,b) = ((a)*(b))`, `ADD32(a,b) = ((a)+(b))`, `SATURATE(x,a) = (x)`,
`SHR32(a,s) = (a)`, `QCONST16(x,b) = (x)`, `HALF32(x) = .5f*(x)`.
Note `MAC16_16` is *not* an FMA: the multiply and the add are separate IEEE-754 binary32 operations
(and the containing files are compiled without `-mfma`).

### 3.1 `xcorr_kernel` → `celt/x86/pitch_sse.c:43` `xcorr_kernel_sse`

```c
void xcorr_kernel_sse(const opus_val16 *x, const opus_val16 *y, opus_val32 sum[4], int len)
{
   int j;
   __m128 xsum1, xsum2;
   xsum1 = _mm_loadu_ps(sum);
   xsum2 = _mm_setzero_ps();

   for (j = 0; j < len-3; j += 4)
   {
      __m128 x0 = _mm_loadu_ps(x+j);
      __m128 yj = _mm_loadu_ps(y+j);
      __m128 y3 = _mm_loadu_ps(y+j+3);

      xsum1 = _mm_add_ps(xsum1,_mm_mul_ps(_mm_shuffle_ps(x0,x0,0x00),yj));
      xsum2 = _mm_add_ps(xsum2,_mm_mul_ps(_mm_shuffle_ps(x0,x0,0x55),
                                          _mm_shuffle_ps(yj,y3,0x49)));
      xsum1 = _mm_add_ps(xsum1,_mm_mul_ps(_mm_shuffle_ps(x0,x0,0xaa),
                                          _mm_shuffle_ps(yj,y3,0x9e)));
      xsum2 = _mm_add_ps(xsum2,_mm_mul_ps(_mm_shuffle_ps(x0,x0,0xff),y3));
   }
   if (j < len)
   {
      xsum1 = _mm_add_ps(xsum1,_mm_mul_ps(_mm_load1_ps(x+j),_mm_loadu_ps(y+j)));
      if (++j < len)
      {
         xsum2 = _mm_add_ps(xsum2,_mm_mul_ps(_mm_load1_ps(x+j),_mm_loadu_ps(y+j)));
         if (++j < len)
         {
            xsum1 = _mm_add_ps(xsum1,_mm_mul_ps(_mm_load1_ps(x+j),_mm_loadu_ps(y+j)));
         }
      }
   }
   _mm_storeu_ps(sum,_mm_add_ps(xsum1,xsum2));
}
```

* **Signature / semantics.** No `arch` argument. Computes, for `k = 0..3`,
  `sum_out[k] = Σ_{m=0}^{len-1} x[m]·y[m+k]` (i.e. 4 lags at once; `y` must provide `len+3` values).
  In `celt_pitch_xcorr_c` it is called as `xcorr_kernel(_x, _y+i, sum, len, arch)` producing
  `xcorr[i..i+3]`. `celt_assert(len>=3)` is in the *C* version only.
* **Shuffle decodes** (`_mm_shuffle_ps(a,b,imm)`: `dst0=a[imm1:0]`, `dst1=a[imm3:2]`,
  `dst2=b[imm5:4]`, `dst3=b[imm7:6]`):
  * `shuffle(x0,x0,0x00)` = `[x_j,x_j,x_j,x_j]`
  * `shuffle(yj,y3,0x49)` = `[y_{j+1}, y_{j+2}, y_{j+3}, y_{j+4}]`
  * `shuffle(x0,x0,0xaa)` = `[x_{j+2}]×4`
  * `shuffle(yj,y3,0x9e)` = `[y_{j+2}, y_{j+3}, y_{j+4}, y_{j+5}]`
  * `shuffle(x0,x0,0xff)` = `[x_{j+3}]×4`, `y3` = `[y_{j+3}, y_{j+4}, y_{j+5}, y_{j+6}]`
* **Exact order.** Two partial accumulators, initialised `xsum1 = {sum[0],sum[1],sum[2],sum[3]}`
  (the caller's existing values, in practice 0) and `xsum2 = 0`. Per main-loop iteration with
  `b = j` (multiple of 4), in this exact order:
  ```
  xsum1[k] = xsum1[k] + x[b  ]·y[b  +k]      (mul then add, no FMA)
  xsum2[k] = xsum2[k] + x[b+1]·y[b+1+k]
  xsum1[k] = xsum1[k] + x[b+2]·y[b+2+k]
  xsum2[k] = xsum2[k] + x[b+3]·y[b+3+k]
  ```
  so lanes hold: `xsum1[k] = Σ_{m ≡ b (mod 4), m even offset} …`, i.e. terms with
  `m ∈ {b, b+2, b+4, …}` and `xsum2[k]` terms with `m ∈ {b+1, b+3, …}`.
* **Tail** with `r = len − j ∈ {1,2,3}` (main loop exits with `j = 4·⌊len/4⌋`, remainder `len mod 4`):
  ```
  r>=1: xsum1 += x[j  ] · [y[j  ], y[j+1], y[j+2], y[j+3]]
  r>=2: xsum2 += x[j+1] · [y[j+1], y[j+2], y[j+3], y[j+4]]
  r>=3: xsum1 += x[j+2] · [y[j+2], y[j+3], y[j+4], y[j+5]]
  ```
  Each of these is a full 4-lane multiply-add: all four lanes receive a term, and all four are part
  of the mathematical result (lane `k` receives `x[m]·y[m+k]`). The `_mm_loadu_ps(y+j)` may read up
  to 3 floats past the logical end — no arithmetic consequence, but a Rust port must not read past
  its buffer and must still add *all four* lanes of the tail products.
* **Final store:** `sum_out[k] = xsum1[k] + xsum2[k]` (one add per lane).
* **Bit-exactness vs `xcorr_kernel_c` (`celt/pitch.h:65`): NOT identical.** The C version keeps the
  four running sums `sum[0..3]` and, for every `m`, adds `x[m]·y[m+k]` to all four in
  ascending-`m` order; the SSE version splits the `m` sequence between two accumulators (even/odd
  offsets within a 4-element block) and folds them together at the very end. Same mathematical value,
  different IEEE-754 rounding. The tail blocks are identical in structure to C (`x[m]·y[m+k]` for all
  k) but are still added into the split accumulators.

Call sites (all reach the SSE kernel because the macro is overridden at every arch):

| file:line | enclosing function | side |
|---|---|---|
| `celt/celt_lpc.c:174` | `celt_fir_c` | **DECODER** (`celt_decoder.c:901`) |
| `celt/celt_lpc.c:248` | `celt_iir` | **DECODER** (`celt_decoder.c:970`) |
| `celt/pitch.c:275` | `celt_pitch_xcorr_c` | reached only via `PITCH_XCORR_IMPL[0..3]` (not on this host) — would be BOTH |

### 3.2 `celt_inner_prod` → `celt/x86/pitch_sse.c:108` `celt_inner_prod_sse`

```c
opus_val32 celt_inner_prod_sse(const opus_val16 *x, const opus_val16 *y, int N)
{
   int i; float xy; __m128 sum;
   sum = _mm_setzero_ps();
   for (i=0;i<N-3;i+=4)
   {
      __m128 xi = _mm_loadu_ps(x+i);
      __m128 yi = _mm_loadu_ps(y+i);
      sum = _mm_add_ps(sum,_mm_mul_ps(xi, yi));
   }
   /* Horizontal sum */
   sum = _mm_add_ps(sum, _mm_movehl_ps(sum, sum));
   sum = _mm_add_ss(sum, _mm_shuffle_ps(sum, sum, 0x55));
   _mm_store_ss(&xy, sum);
   for (;i<N;i++)
   {
      xy = MAC16_16(xy, x[i], y[i]);
   }
   return xy;
}
```

* Signature `(const float*, const float*, int)`, returns `float`; **no arch argument**.
* One 4-lane accumulator. For `k = 0..3`: `s[k] = Σ_{i ≡ k (mod 4), i < 4⌊N/4⌋} x[i]·y[i]`
  (each term `mul` then `add`).
* `_mm_movehl_ps(sum,sum)` = `[s2, s3, s2, s3]`; after the first add
  `t = [s0+s2, s1+s3, s2+s2, s3+s3]`. `_mm_shuffle_ps(t,t,0x55)` = `[t1,t1,t1,t1]`;
  `_mm_add_ss` adds only lane 0. So the horizontal result is exactly
  ```
  xy = (s0 + s2) + (s1 + s3)
  ```
  (that add order; `(s0+s2)` computed first, then `+ (s1+s3)`).
* Tail: for the remaining `N mod 4` indices, `xy = xy + x[i]·y[i]` sequentially, starting from the
  float value produced above.
* **NOT bit-identical to `celt_inner_prod_c` (`celt/pitch.h:159`)** except when `N ≤ 4`
  (then only the tail runs). The C version is a single sequential `xy += x[i]*y[i]` over all `i`.

Call sites (`celt/`, `silk/` scope only; `arch` is ignored by the macro):

| file:line | enclosing function | side |
|---|---|---|
| `celt/bands.c:160` | `compute_band_energies` (float branch) | **ENCODER** (`celt_encoder.c:2081,2096`) |
| `celt/pitch.c:295` | `celt_pitch_xcorr_c` tail | arch ≤ 3 table entry only |
| `celt/pitch.c:384` | `pitch_search` (fine search) | **BOTH** (enc `celt_encoder.c:1468`, dec `celt_decoder.c:568`) |
| `celt/pitch.c:545` | `remove_doubling` | **ENCODER** (`celt_encoder.c:1473`) |
| `celt/x86/pitch_avx.c:97` | `celt_pitch_xcorr_avx2` tail | **BOTH** |
| `celt/vq.c:603` (indirect via `celt_inner_prod_norm`) | `alg_quant` | **ENCODER** |
| `silk/fixed/vector_ops_FIX.c:79` | `silk_inner_prod16_fallback` | *not compiled* (FIXED_POINT file) |

Out of the `celt/`+`silk/` scope but on real paths of the same library:
`src/opus_encoder.c:1110,2014`, `src/opus_multistream_encoder.c:280` (**arch is hard-coded to 0
there — harmless, the macro ignores arch**), `dnn/lpcnet_enc.c:171,172,189,190,191` (ENCODER).

### 3.3 `dual_inner_prod` → `celt/x86/pitch_sse.c:79` `dual_inner_prod_sse`

```c
void dual_inner_prod_sse(const opus_val16 *x, const opus_val16 *y01, const opus_val16 *y02,
      int N, opus_val32 *xy1, opus_val32 *xy2)
{
   int i; __m128 xsum1, xsum2;
   xsum1 = _mm_setzero_ps();  xsum2 = _mm_setzero_ps();
   for (i=0;i<N-3;i+=4)
   {
      __m128 xi = _mm_loadu_ps(x+i);
      __m128 y1i = _mm_loadu_ps(y01+i);
      __m128 y2i = _mm_loadu_ps(y02+i);
      xsum1 = _mm_add_ps(xsum1,_mm_mul_ps(xi, y1i));
      xsum2 = _mm_add_ps(xsum2,_mm_mul_ps(xi, y2i));
   }
   /* Horizontal sum */
   xsum1 = _mm_add_ps(xsum1, _mm_movehl_ps(xsum1, xsum1));
   xsum1 = _mm_add_ss(xsum1, _mm_shuffle_ps(xsum1, xsum1, 0x55));
   _mm_store_ss(xy1, xsum1);
   xsum2 = _mm_add_ps(xsum2, _mm_movehl_ps(xsum2, xsum2));
   xsum2 = _mm_add_ss(xsum2, _mm_shuffle_ps(xsum2, xsum2, 0x55));
   _mm_store_ss(xy2, xsum2);
   for (;i<N;i++)
   {
      *xy1 = MAC16_16(*xy1, x[i], y01[i]);
      *xy2 = MAC16_16(*xy2, x[i], y02[i]);
   }
}
```

* No `arch` argument. Two independent 4-lane accumulators with the same lane grouping as §3.2;
  identical horizontal reduction `((t0+t2)+(t1+t3))` for each of them; the ≤3-element tail reads the
  stored float back and accumulates sequentially.
* **NOT bit-identical to `dual_inner_prod_c` (`celt/pitch.h:137`)** for `N > 4`: the C version
  accumulates `xy01 = xy01 + x[i]*y01[i]` strictly in ascending `i` from 0.

Call sites (only two, both encoder-only via `remove_doubling`):

| file:line | enclosing function | side |
|---|---|---|
| `celt/pitch.c:480` | `remove_doubling` | **ENCODER** |
| `celt/pitch.c:513` | `remove_doubling` | **ENCODER** |

### 3.4 `comb_filter_const` → `celt/x86/pitch_sse.c:133` `comb_filter_const_sse`

```c
void comb_filter_const_sse(opus_val32 *y, opus_val32 *x, int T, int N,
      opus_val16 g10, opus_val16 g11, opus_val16 g12)
{
   int i; __m128 x0v; __m128 g10v, g11v, g12v;
   g10v = _mm_load1_ps(&g10);  g11v = _mm_load1_ps(&g11);  g12v = _mm_load1_ps(&g12);
   x0v = _mm_loadu_ps(&x[-T-2]);
   for (i=0;i<N-3;i+=4)
   {
      __m128 yi, yi2, x1v, x2v, x3v, x4v;
      const opus_val32 *xp = &x[i-T-2];
      yi = _mm_loadu_ps(x+i);
      x4v = _mm_loadu_ps(xp+4);
      x2v = _mm_shuffle_ps(x0v, x4v, 0x4e);
      x1v = _mm_shuffle_ps(x0v, x2v, 0x99);
      x3v = _mm_shuffle_ps(x2v, x4v, 0x99);

      yi = _mm_add_ps(yi, _mm_mul_ps(g10v,x2v));
#if 0 /* Set to 1 to make it bit-exact with the non-SSE version */
      yi = _mm_add_ps(yi, _mm_mul_ps(g11v,_mm_add_ps(x3v,x1v)));
      yi = _mm_add_ps(yi, _mm_mul_ps(g12v,_mm_add_ps(x4v,x0v)));
#else
      /* Use partial sums */
      yi2 = _mm_add_ps(_mm_mul_ps(g11v,_mm_add_ps(x3v,x1v)),
                       _mm_mul_ps(g12v,_mm_add_ps(x4v,x0v)));
      yi = _mm_add_ps(yi, yi2);
#endif
      x0v=x4v;
      _mm_storeu_ps(y+i, yi);
   }
#ifdef CUSTOM_MODES
   for (;i<N;i++)
   {
      y[i] = x[i]
               + MULT16_32_Q15(g10,x[i-T])
               + MULT16_32_Q15(g11,ADD32(x[i-T+1],x[i-T-1]))
               + MULT16_32_Q15(g12,ADD32(x[i-T+2],x[i-T-2]));
   }
#endif
}
```

* No `arch` argument; `opus_val32 = float`, so `y`/`x` are `float*` (celt_sig buffers).
* Shuffle decodes (with `j = i-T-2` so `x0v = [x_{j}, x_{j+1}, x_{j+2}, x_{j+3}]`,
  `x4v = [x_{j+4}, x_{j+5}, x_{j+6}, x_{j+7}]`):
  * `shuffle(x0v,x4v,0x4e)` = `[x0v2, x0v3, x4v0, x4v1]` = `[x_{i-T}, x_{i-T+1}, x_{i-T+2}, x_{i-T+3}]`
  * `shuffle(x0v,x2v,0x99)` = `[x0v1, x0v2, x2v1, x2v2]` = `[x_{i-T-1}, x_{i-T}, x_{i-T+1}, x_{i-T+2}]`
  * `shuffle(x2v,x4v,0x99)` = `[x2v1, x2v2, x4v1, x4v2]` = `[x_{i-T+1}, x_{i-T+2}, x_{i-T+3}, x_{i-T+4}]`
  * `x0v` for the next block becomes the old `x4v`.
* **Exact per-output order** (per lane, `k = 0..3`, `n = i+k`), with all operations separate
  `mul`/`add` (no FMA):
  ```
  a  = g10 * x[n-T]
  p  = x[n]
  p  = p + a                                  /* (x[n] + g10*x[n-T])            */
  s1 = x[n-T+1] + x[n-T-1]
  m1 = g11 * s1
  s2 = x[n-T+2] + x[n-T-2]
  m2 = g12 * s2
  q  = m1 + m2                                /* partial sum                    */
  y[n] = p + q
  ```
  i.e. `y[n] = (x[n] + g10·x[n-T]) + (g11·(x[n-T+1]+x[n-T-1]) + g12·(x[n-T+2]+x[n-T-2]))`.
* **Tail (active! `CUSTOM_MODES` is defined):** the loop runs while `i < N-3`, so it exits with
  `i_end = 4·⌈(N-3)/4⌉` and the tail is exactly `N mod 4` samples (`N=8→0, 9→1, 10→2, 11→3, 12→0`).
  Those last `0..3` samples use the plain left-to-right C expression, i.e.
  `y[n] = ((x[n] + g10·x[n-T]) + g11·(x[n-T+1]+x[n-T-1])) + g12·(x[n-T+2]+x[n-T-2])`.
  So one call can mix the two groupings, and a Rust port must implement both.
* **Bit-exactness vs `comb_filter_const_c`: NOT identical** — this is the case flagged by the source's
  own `#if 0 /* Set to 1 to make it bit-exact with the non-SSE version */`. The difference is exactly
  the association of the last two terms (`(a+b) + (c+d)` vs `((a+b)+c)+d`). The C function is also
  **not compiled at all** in this build (§2.3).
* Aliasing note: the decoder calls `comb_filter(out_syn[c], out_syn[c], …)` in-place. Reads are at
  `n-T±2` with `T ≥ COMBFILTER_MINPERIOD` (≥15), always ≥ 13 samples behind the writes, and both C and
  SSE process strictly left to right in blocks of 4, so in-place behaviour is identical.

Call sites:

| file:line | enclosing function | side |
|---|---|---|
| `celt/celt.c:312` | `comb_filter` (constant-filter section) | **BOTH** |

`comb_filter` itself is called from (ENCODER) `celt/celt_encoder.c:1549, 1553, 1576, 2749, 2753` and
(DECODER) `celt/celt_decoder.c:606, 792, 796, 1540, 1544`.

### 3.5 `celt_pitch_xcorr` → `celt/x86/pitch_avx.c:86` `celt_pitch_xcorr_avx2` (arch 4)

```c
void celt_pitch_xcorr_avx2(const float *_x, const float *_y, float *xcorr, int len, int max_pitch, int arch)
{
   int i;
   celt_assert(max_pitch>0);
   (void)arch;
   for (i=0;i<max_pitch-7;i+=8)
   {
      xcorr_kernel_avx(_x, _y+i, &xcorr[i], len);
   }
   for (;i<max_pitch;i++)
   {
      xcorr[i] = celt_inner_prod(_x, _y+i, len, arch);
   }
}
```

Signature identical to `celt_pitch_xcorr_c`; `arch` is ignored. `celt_inner_prod` in the tail is the
macro → **`celt_inner_prod_sse`** (§3.2).

```c
static void xcorr_kernel_avx(const float *x, const float *y, float sum[8], int len)
{
    __m256 xsum0, xsum1, xsum2, xsum3, xsum4, xsum5, xsum6, xsum7;
    xsum7 = xsum6 = xsum5 = xsum4 = xsum3 = xsum2 = xsum1 = xsum0 = _mm256_setzero_ps();
    int i; __m256 x0;
    /* Compute 8 inner products using partial sums. */
    for (i=0;i<len-7;i+=8)
    {
        x0 = _mm256_loadu_ps(x+i);
        xsum0 = _mm256_fmadd_ps(x0, _mm256_loadu_ps(y+i  ), xsum0);
        xsum1 = _mm256_fmadd_ps(x0, _mm256_loadu_ps(y+i+1), xsum1);
        xsum2 = _mm256_fmadd_ps(x0, _mm256_loadu_ps(y+i+2), xsum2);
        xsum3 = _mm256_fmadd_ps(x0, _mm256_loadu_ps(y+i+3), xsum3);
        xsum4 = _mm256_fmadd_ps(x0, _mm256_loadu_ps(y+i+4), xsum4);
        xsum5 = _mm256_fmadd_ps(x0, _mm256_loadu_ps(y+i+5), xsum5);
        xsum6 = _mm256_fmadd_ps(x0, _mm256_loadu_ps(y+i+6), xsum6);
        xsum7 = _mm256_fmadd_ps(x0, _mm256_loadu_ps(y+i+7), xsum7);
    }
    if (i != len) {
        static const int mask[15] = {-1,-1,-1,-1,-1,-1,-1,0,0,0,0,0,0,0,0};
        __m256i m;
        m = _mm256_loadu_si256((__m256i*)(void*)(mask + 7+i-len));
        x0 = _mm256_maskload_ps(x+i, m);
        xsum0 = _mm256_fmadd_ps(x0, _mm256_maskload_ps(y+i  , m), xsum0);
        ... xsum1..xsum7 with y+i+1 … y+i+7 ...
    }
    /* 8 horizontal adds. */
    /* Compute [0 4] [1 5] [2 6] [3 7] */
    xsum0 = _mm256_add_ps(_mm256_permute2f128_ps(xsum0, xsum4, 2<<4), _mm256_permute2f128_ps(xsum0, xsum4, 1 | (3<<4)));
    xsum1 = _mm256_add_ps(_mm256_permute2f128_ps(xsum1, xsum5, 2<<4), _mm256_permute2f128_ps(xsum1, xsum5, 1 | (3<<4)));
    xsum2 = _mm256_add_ps(_mm256_permute2f128_ps(xsum2, xsum6, 2<<4), _mm256_permute2f128_ps(xsum2, xsum6, 1 | (3<<4)));
    xsum3 = _mm256_add_ps(_mm256_permute2f128_ps(xsum3, xsum7, 2<<4), _mm256_permute2f128_ps(xsum3, xsum7, 1 | (3<<4)));
    /* Compute [0 1 4 5] [2 3 6 7] */
    xsum0 = _mm256_hadd_ps(xsum0, xsum1);
    xsum1 = _mm256_hadd_ps(xsum2, xsum3);
    /* Compute [0 1 2 3 4 5 6 7] */
    xsum0 = _mm256_hadd_ps(xsum0, xsum1);
    _mm256_storeu_ps(sum, xsum0);
}
```

* **FMA is used** in the accumulation loop and in the masked tail: each term is one fused
  `fma(x, y, acc)` (single rounding). The tail's `if (i != len)` executes for every `len` not a
  multiple of 8 (`i = 8⌊len/8⌋`, remainder `len mod 8 ∈ 1..7`); the mask
  `mask + 7 + i - len` selects exactly the remaining `len - i` lanes (verified: for `r = len-i`,
  offsets `7-r+t` are `-1` for `t ≤ r-1`), and masked lanes contribute `fma(0,0,acc) = acc` exactly.
  `len < 8` therefore takes only the masked path (with `mask + 7 - len`).
* **Lane meaning.** All eight accumulators are `__m256` (8 × f32).
  `xsum_j[k] = Σ_{m ≡ k (mod 8), m < 8⌊len/8⌋} x[m]·y[m+j]`, and output `sum[j] = xsum_j` reduced.
* **Reduction order (the critical part).** With `s_k = xsum_p[k]` (lane k of the accumulator for
  output `p`):
  ```
  A_p    = combine(xsum_p, xsum_{p+4})              for p = 0..3, where
           combine(a,b) = [a0+a4, a1+a5, a2+a6, a3+a7, b0+b4, b1+b5, b2+b6, b3+b7]
           (permute2f128(a,b,0x20) + permute2f128(a,b,0x31))
  B0     = hadd(A0, A1)       /* 128-bit lane-wise [a0+a1, a2+a3, b0+b1, b2+b3] */
  B1     = hadd(A2, A3)
  C      = hadd(B0, B1)
  sum[p] = C[p]
  ```
  which unrolls to, **for every output p**:
  ```
  sum[p] = ( (s0 + s4) + (s1 + s5) ) + ( (s2 + s6) + (s3 + s7) )
  ```
  where `s_k` is lane `k` of `xsum_p` (the accumulator whose index equals the output lag).
  `_mm256_hadd_ps(a,b)` per 128-bit lane yields `[a0+a1, a2+a3, b0+b1, b2+b3]`.
* **Lags ≥ `8⌊max_pitch/8⌋`** are computed by `celt_inner_prod_sse(_x, _y+i, len)` one lag at a time
  (§3.2 accumulation order).
* **Bit-exactness vs `celt_pitch_xcorr_c`: NOT identical.** Differences: (1) FMA in the AVX2 path vs
  separate `mul`+`add` in the C/SSE paths (`celt_pitch_xcorr_c` compiles the `xcorr_kernel` macro =
  `xcorr_kernel_sse`); (2) the 8-way partial-sum/reduction tree above vs the 2-way split of
  `xcorr_kernel_sse`; (3) for the trailing `< 8` lags the AVX2 version uses the `celt_inner_prod_sse`
  order, exactly as the C version would.

Call sites:

| file:line | enclosing function | side |
|---|---|---|
| `celt/celt_lpc.c:344` | `_celt_autocorr` | **BOTH** — ENCODER via `pitch_downsample` (`celt/pitch.c:189`), DECODER via `celt_decoder.c:859` |
| `celt/pitch.c:361` | `pitch_search` | **BOTH** |
| `silk/float/pitch_analysis_core_FLP.c:181` | `silk_pitch_analysis_core_FLP` | **ENCODER** |
| `silk/float/pitch_analysis_core_FLP.c:533` | same | **ENCODER** |
| `silk/fixed/pitch_analysis_core_FIX.c:200,616` | — | not compiled (FIXED_POINT) |
| `silk/fixed/burg_modified_FIX.c:98`, `silk/fixed/x86/burg_modified_FIX_sse4_1.c:106` | — | not compiled |
| `dnn/nndsp.c:215,216,308,310` | ADA-Conv/ADA-Comb | **DECODER** (used by `dnn/osce.c` → OSCE) |
| `dnn/lpcnet_enc.c:170` | LPCNet encoder | ENCODER (dnn) |

### 3.6 `op_pvq_search` → `celt/x86/vq_sse2.c:44` `op_pvq_search_sse2`

Signature `opus_val16 op_pvq_search_sse2(celt_norm *_X, int *iy, int K, int N, int arch)` — i.e.
`float (float*, int*, int, int, int)`; `arch` is received and `(void)arch`-ed (line 57). `N > 1`,
`K > 0`. `_X` is modified in place (the sign-stripped magnitudes are written back), `iy[]` gets the
signed pulse allocation, and the returned `yy` is the squared norm of the pre-search `y` vector
(double-counted: `y[]` holds `2·iy[]`).

Structure (float build, all `FIXED_POINT` branches absent):

```c
   signmask = _mm_set_ps1(-0.f);  fours = _mm_set_epi32(4,4,4,4);
   ALLOC(y, N+3, celt_norm); ALLOC(X, N+3, celt_norm); ALLOC(signy, N+3, float);
   OPUS_COPY(X, _X, N);  X[N]=X[N+1]=X[N+2]=0;
   sums = _mm_setzero_ps();
   for (j=0;j<N;j+=4) {
      x4 = _mm_loadu_ps(&X[j]);
      s4 = _mm_cmplt_ps(x4, _mm_setzero_ps());
      x4 = _mm_andnot_ps(signmask, x4);          /* |X[j]| */
      sums = _mm_add_ps(sums, x4);
      _mm_storeu_ps(&y[j], _mm_setzero_ps());
      _mm_storeu_si128(&iy[j], _mm_setzero_si128());
      _mm_storeu_ps(&X[j], x4);
      _mm_storeu_ps(&signy[j], s4);
   }
   sums = _mm_add_ps(sums, _mm_shuffle_ps(sums, sums, _MM_SHUFFLE(1,0,3,2)));   /* 0x4E */
   sums = _mm_add_ps(sums, _mm_shuffle_ps(sums, sums, _MM_SHUFFLE(2,3,0,1)));   /* 0xB1 */
   xy = yy = 0;  pulsesLeft = K;
   if (K > (N>>1)) {
      opus_val32 sum = _mm_cvtss_f32(sums);
      if (!(sum > EPSILON && sum < 64)) { X[0]=1.f; X[1..N-1]=0; sums = _mm_set_ps1(1.f); }
      rcp4 = _mm_mul_ps(_mm_set_ps1((float)(K+.8)), _mm_rcp_ps(sums));
      xy4 = yy4 = 0;  pulses_sum = 0;
      for (j=0;j<N;j+=4) {
         x4 = _mm_loadu_ps(&X[j]);
         rx4 = _mm_mul_ps(x4, rcp4);
         iy4 = _mm_cvttps_epi32(rx4);
         pulses_sum = _mm_add_epi32(pulses_sum, iy4);
         _mm_storeu_si128(&iy[j], iy4);
         y4 = _mm_cvtepi32_ps(iy4);
         xy4 = _mm_add_ps(xy4, _mm_mul_ps(x4, y4));
         yy4 = _mm_add_ps(yy4, _mm_mul_ps(y4, y4));
         _mm_storeu_ps(&y[j], _mm_add_ps(y4, y4));
      }
      /* horizontal reductions: shuffles 0x4E then 0xB1, lane 0 taken */
      pulsesLeft -= (int)lane0(pulses_sum);   xy = lane0(xy4);   yy = lane0(yy4);
   }
   X[N]=X[N+1]=X[N+2]=-100;  y[N]=y[N+1]=y[N+2]=100;
   if (pulsesLeft > N+3) { yy = yy + pulsesLeft*pulsesLeft; yy = yy + pulsesLeft*y[0];
                           iy[0] += pulsesLeft; pulsesLeft = 0; }
   for (i=0;i<pulsesLeft;i++) {
      yy = yy + 1;
      xy4 = _mm_load1_ps(&xy);  yy4 = _mm_load1_ps(&yy);
      max = 0;  pos = 0;  count = (3,2,1,0);
      for (j=0;j<N;j+=4) {
         x4 = _mm_loadu_ps(&X[j]);  y4 = _mm_loadu_ps(&y[j]);
         x4 = _mm_add_ps(x4, xy4);
         y4 = _mm_add_ps(y4, yy4);
         y4 = _mm_rsqrt_ps(y4);
         r4 = _mm_mul_ps(x4, y4);
         pos = _mm_max_epi16(pos, _mm_and_si128(count, _mm_castps_si128(_mm_cmpgt_ps(r4, max))));
         max = _mm_max_ps(max, r4);
         count = _mm_add_epi32(count, fours);
      }
      max2 = _mm_max_ps(max, _mm_shuffle_ps(max, max, 0x4E));
      max2 = _mm_max_ps(max2, _mm_shuffle_ps(max2, max2, 0xB1));
      pos = _mm_and_si128(pos, _mm_castps_si128(_mm_cmpeq_ps(max, max2)));
      pos = _mm_max_epi16(pos, _mm_unpackhi_epi64(pos, pos));
      pos = _mm_max_epi16(pos, _mm_shufflelo_epi16(pos, _MM_SHUFFLE(1,0,3,2)));
      best_id = _mm_cvtsi128_si32(pos);
      xy = xy + X[best_id];   yy = yy + y[best_id];
      y[best_id] += 2;  iy[best_id]++;
   }
   for (j=0;j<N;j+=4) {           /* restore sign, branchless */
      y4 = _mm_loadu_si128(&iy[j]);
      s4 = _mm_castps_si128(_mm_loadu_ps(&signy[j]));
      y4 = _mm_xor_si128(_mm_add_epi32(y4, s4), s4);
      _mm_storeu_si128(&iy[j], y4);
   }
   return yy;
```

Exact arithmetic points that must be reproduced:

* `sums` starts as the 4-lane |X| accumulator; the reductions are
  `t = [s0+s2, s1+s3, …]`, then `t2 = [t0+t1, t1+t0, …]`; **lane 0 of the result is
  `((s0+s2)+(s1+s3))`**, and it is *broadcast to all four lanes only after the reduction*
  (`_mm_rcp_ps` is applied to the vector whose lanes all hold that one value). See `t`/`t2`: after
  the second add all lanes equal the sum, which is why `_mm_cvtss_f32` and `_mm_rcp_ps(sums)` agree.
  The sum includes the `j = N..⌈N/4⌉·4-1` padding lanes, which are zero (`X[N..N+2]=0` at that point).
* **`_mm_rcp_ps` is the 12-bit approximate reciprocal** (Relative Error ≤ 1.5·2⁻¹²), *not* IEEE
  division. The C reference uses `celt_rcp(sum)` = `1.f/sum` (`celt/mathops.h:231`).
* `iy[j] = (int)trunc(rcp·X[j])` (`_mm_cvttps_epi32`), matching C's `floor` because `X[j] ≥ 0`,
  `rcp > 0`.
* The pulse-search score is `r4 = (X[j]+xy) · rsqrt(y[j]+yy)` with **`_mm_rsqrt_ps` (approximate
  reciprocal square root, ~12-bit)**, maximised directly. The C reference computes
  `Rxy = (xy + X[j])`, `Rxy = Rxy*Rxy`, `Ryy = yy + y[j]` and compares with the exact products
  `MULT16_16(best_den,Rxy) > MULT16_16(Ryy,best_num)` (`celt/vq.c:326-350`).
* `best_id` selection: the SSE tracks, per lane, the largest `r4` and the index of the lane where
  `r4 > max` occurred, then keeps positions whose lane-max equals the global max and takes the
  **largest** such index (`_mm_max_epi16` reductions over `pos`). The C version takes the **first**
  index with a strictly larger score (`best_id` updated only on `>`, scanning `j` upward).
* The `x4 = X[j]+xy; y4 = y[j]+yy` additions, the `y[best_id] += 2`, `iy[best_id]++`,
  `xy += X[best_id]`, `yy += y[best_id]` updates and the final branchless sign restoration
  (`(iy^sign)+sign`, `sign = 0` or `-1`) are identical to C.
* The `pulsesLeft > N+3` fallback (`yy = yy + tmp*tmp; yy = yy + tmp*y[0]; iy[0] += pulsesLeft`)
  is identical to C.

* **Bit-exactness vs `op_pvq_search_c` (`celt/vq.c:205`): NOT identical.** Four independent reasons:
  1. `sum` is reduced as `((s0+s2)+(s1+s3))` instead of a left-to-right running sum;
  2. `_mm_rcp_ps` (approximate) instead of `1.f/sum`;
  3. `_mm_rsqrt_ps` + direct score maximisation instead of the exact cross-multiplied ratio test;
  4. different index tie-break (largest vs first).

Call sites (encoder only — `alg_unquant` uses `decode_pulses`/`normalise_residual`, not this search):

| file:line | enclosing function | side |
|---|---|---|
| `celt/vq.c:603` | `alg_quant` (float, non-QEXT branch) | **ENCODER** |

---

## 4. SILK kernels — exact arithmetic

`sil_float = float`, `opus_int16/32/64` are the fixed-width integer types, `silk_VAD_GetSA_Q8`,
`silk_NSQ*`, `silk_VQ_WMat_EC` and `silk_inner_product_FLP_c` use integer arithmetic except the last
one. All SILK kernels here are compiled with the flags of §1.2 (SSE4.1 files with `-msse4.1`,
`NSQ_del_dec_avx2.c`/`inner_product_FLP_avx2.c` with `-mavx -mfma -mavx2`); note that
`silk/x86/*.c` are compiled even though `FIXED_POINT` is undefined — none of those files has a
`FIXED_POINT` guard.

Whether the SILK SIMD code is reached at all in a normal encode:

* default `st->silk_mode.complexity = 9` (`src/opus_encoder.c:269`) → `silk_setup_complexity`
  (`silk/control_codec.c:376-385`) sets `shapingLPCOrder = 24`,
  `nStatesDelayedDecision = MAX_DEL_DEC_STATES = 4`.
* `silk/float/wrappers_FLP.c:163-169` chooses
  `silk_NSQ_del_dec` iff `nStatesDelayedDecision > 1 || warping_Q16 > 0` → so the default path is
  `SILK_NSQ_DEL_DEC_IMPL[4] = silk_NSQ_del_dec_avx2`.
* `silk_NSQ` (the non-delayed-decision one) is only used for complexity < 1 (`nStates = 1`,
  `warping = 0`).

### 4.1 `silk_VAD_GetSA_Q8` → `silk/x86/VAD_sse4_1.c:45` `silk_VAD_GetSA_Q8_sse4_1`

`opus_int silk_VAD_GetSA_Q8_sse4_1(silk_encoder_state *psEncC, const opus_int16 pIn[])` — **no arch
argument**. Only the per-subframe energy accumulation is vectorised (lines 139-167); everything else
is a verbatim copy of `silk_VAD_GetSA_Q8_c` (`silk/VAD.c:91`…), including `silk_ana_filt_bank_1` ×3,
the differentiator, the noise estimation, `silk_sigm_Q15`, etc. Verified by diffing the two files
besides the energy loop and the `#ifdef OPUS_CHECK_ASM` block.

```c
        for( s = 0; s < VAD_INTERNAL_SUBFRAMES; s++ ) {
            __m128i xmm_X, xmm_acc;
            sumSquared = 0;
            xmm_acc = _mm_setzero_si128();
            for( i = 0; i < dec_subframe_length - 7; i += 8 )
            {
                xmm_X   = _mm_loadu_si128( &X[ X_offset[b] + i + dec_subframe_offset ] );
                xmm_X   = _mm_srai_epi16( xmm_X, 3 );        /* silk_RSHIFT(x,3), int16 arithmetic */
                xmm_X   = _mm_madd_epi16( xmm_X, xmm_X );    /* pairwise x0^2+x1^2, x2^2+x3^2, ...  */
                xmm_acc = _mm_add_epi32( xmm_acc, xmm_X );
            }
            xmm_acc = _mm_add_epi32( xmm_acc, _mm_unpackhi_epi64( xmm_acc, xmm_acc ) );
            xmm_acc = _mm_add_epi32( xmm_acc, _mm_shufflelo_epi16( xmm_acc, 0x0E ) );
            sumSquared += _mm_cvtsi128_si32( xmm_acc );
            for( ; i < dec_subframe_length; i++ ) {
                x_tmp = silk_RSHIFT( X[ X_offset[b] + i + dec_subframe_offset ], 3 );
                sumSquared = silk_SMLABB( sumSquared, x_tmp, x_tmp );
            }
            if( s < VAD_INTERNAL_SUBFRAMES - 1 ) Xnrg[b] = silk_ADD_POS_SAT32( Xnrg[b], sumSquared );
            else Xnrg[b] = silk_ADD_POS_SAT32( Xnrg[b], silk_RSHIFT( sumSquared, 1 ) );
            dec_subframe_offset += dec_subframe_length;
        }
        psSilk_VAD->XnrgSubfr[ b ] = sumSquared;
```

* `_mm_madd_epi16(x,x)` gives 4 int32 lanes `[x0²+x1², x2²+x3², …]` (products of *shifted* int16).
  The horizontal reduction is the same idiom as §3.6 for 32-bit lanes:
  `u = acc + unpackhi_epi64(acc,acc) = [a0+a2, a1+a3, …, a2+…, a3+…]`, then
  `u += shufflelo_epi16(u,0x0E)` (32-bit lane 0 ← lane 1) and lane 0 is taken, so
  `sumSquared += (a0+a2) + (a1+a3)`, i.e. `(x0²+x1² + x4²+x5²) + (x2²+x3² + x6²+x7²)` per 8 samples.
* Main loop runs while `i < dec_subframe_length - 7`, so `i` steps by 8 while at least 8 elements
  remain; the ≤7 remainder uses scalar `silk_SMLABB(sumSquared, x_tmp, x_tmp)`
  (`= sumSquared + (int16)x_tmp·(int16)x_tmp`) in ascending `i`.
* **Bit-identical to `silk_VAD_GetSA_Q8_c`**: every product/sum is integer; per-term products are
  identical to `silk_SMLABB`, the only difference is the *grouping* of the int32 additions, and
  int32 addition is associative modulo 2³². The source's own comment documents the bound
  (`dec_subframe_length ≤ 128`, energy < `128·(2¹²)²`) that makes overflow impossible, so no
  saturation difference can arise.

Call sites:

| file:line | enclosing function | side |
|---|---|---|
| `silk/float/encode_frame_FLP.c:55` | `silk_encode_frame_FLP` | **ENCODER** |
| `silk/fixed/encode_frame_FIX.c:55` | — | not compiled in this build |

### 4.2 `silk_NSQ` → `silk/x86/NSQ_sse4_1.c:74` `silk_NSQ_sse4_1`

`void silk_NSQ_sse4_1(const silk_encoder_state*, silk_nsq_state*, SideInfoIndices*, const opus_int16 x16[], opus_int8 pulses[], const opus_int16 *PredCoef_Q12, const opus_int16 LTPCoef_Q14[25], const opus_int16 AR_Q13[MAX_NB_SUBFR*24], const opus_int HarmShapeGain_Q14[4], const opus_int Tilt_Q14[4], const opus_int32 LF_shp_Q14[4], const opus_int32 Gains_Q16[4], const opus_int pitchL[4], const opus_int Lambda_Q10, const opus_int LTP_scale_Q14)` — **no arch argument** (the macro's `arch` is dropped).

* The function is a line-for-line copy of `silk_NSQ_c` (`silk/NSQ.c:76`) for the setup, the 64×4
  RD table (`table[64][4]`, `silk_NSQ_sse4_1.c:147-201`, identical arithmetic to `NSQ.c:286-323`), the
  subframe loop, the `LSF_interpolation_flag` handling, re-whitening (`silk_LPC_analysis_filter`, the
  **non-SIMD** one) and the final `memmove`s.
* It calls `silk_nsq_scale_states_sse4_1` instead of `silk_nsq_scale_states`; the vectorised parts are
  `OP_CVTEPI16_EPI32_M64` + `_mm_mul_epi32` + `(srli/slli 16)` + `_mm_blend_epi16` which implement
  `silk_SMULWW(a,b) = (int32)(((int64)a·b) >> 16)` exactly, for:
  * `x_sc_Q10[i] = silk_SMULWW(x16[i], inv_gain_Q26)`, `i` stepping 4 while `i < subfr_length-3`,
    then scalar for the remainder;
  * `NSQ->sLTP_shp_Q14[i] = silk_SMULWW(gain_adj_Q16, sLTP_shp_Q14[i])`, stepping 4 while
    `i < sLTP_shp_buf_idx-3`, then scalar.
  All other work in that helper is scalar and identical to `silk_nsq_scale_states` (`silk/NSQ.c:368`).
  (`inv_gain_Q26 = silk_RSHIFT_ROUND(inv_gain_Q31,5)` identical.)
* **The only genuinely new kernel** is
  `silk_noise_shape_quantizer_10_16_sse4_1` (`NSQ_sse4_1.c:284`), used only when
  `(10 == psEncC->shapingLPCOrder) && (16 == psEncC->predictLPCOrder)` (line 247). For any other
  order pair the wrapper calls the plain `silk_noise_shape_quantizer` from `silk/NSQ.c:183`
  (which with `OPUS_X86_MAY_HAVE_SSE4_1` defined is a non-static external function).
  **In this build the condition can never be true**: `silk_setup_complexity` only assigns
  `shapingLPCOrder ∈ {12,14,12,14,16,20,24}` (`control_codec.c:320-380`), never 10. The 10/16 kernel
  is therefore dead code, but it is compiled and must be reproduced if the Rust port wants byte-exact
  parity for hypothetical custom configurations.
* If it *were* reached, the differences from C (all analysed, see below) are: the short-term
  prediction uses a hi/lo `_mm_madd_epi16`/`_mm_mulhi_epi16` decomposition of
  `silk_SMLAWB` (exact, because `OPUS_FAST_INT64` makes `silk_SMULWB(a,b) = (a·(int16)b)>>16` and the
  hi/lo split reproduces that exactly; the accumulation order is irrelelvant modulo 2³²); the LTP
  prediction uses `_mm_mul_epi32` + `_mm_srli_si128(.,2)` for taps 0..3 plus one scalar
  `silk_SMLAWB` for tap 4 (exact); the RD decision is table-driven:
  `if (r_Q10 * table[q1_Q0][2] - table[q1_Q0][3] < 0) q1_Q10 = q2_Q10;` which is *algebraically
  identical* to `NSQ.c:324-331`'s `rd2 < rd1` because
  `table[.][3] = (rd1lin-rd2lin) + (q1²-q2²)` and `table[.][2] = 2(q1-q2)` (derivation:
  `rd2-rd1 = 2r(q1-q2) - [ (rd1lin-rd2lin) + (q1²-q2²) ]`); and the `xq` scaling uses
  `(int64(psLPC_Q14[i])·Gain_Q10) >> 16` then `(x+128)>>8` + `packs_epi32` saturation, which equals
  `silk_SAT16(silk_RSHIFT_ROUND(silk_SMULWW(x,Gain_Q10),8))` because `OPUS_FAST_INT64` makes
  `silk_SMULWW = (int64 product)>>16`. Bounds that make the C int16 truncations irrelevant:
  `Lambda_Q10 < 3072` (`silk/fixed/process_gains_FIX.c:108-115` plus the ×1.5 LBRR bump at
  `encode_frame_FIX.c:283`), `|q1_Q10|,|q2_Q10| ≤ 32064`, `|rr_Q10| ≤ ~1104`.
* **Bit-exactness:** for every configuration that can actually occur in this build, `silk_NSQ_sse4_1`
  produces bit-identical integer results to `silk_NSQ_c` (it *is* the C code plus a bit-exact
  `SMULWW` vectorisation). The unused 10/16 kernel is also bit-exact by the argument above.

Call sites:

| file:line | enclosing function | side |
|---|---|---|
| `silk/float/wrappers_FLP.c:167` | `silk_NSQ_wrapper_FLP` (else-branch: `nStatesDelayedDecision==1 && warping_Q16==0`) | **ENCODER** |
| `silk/fixed/encode_frame_FIX.c:213, 444` | — | not compiled |

### 4.3 `silk_VQ_WMat_EC` → `silk/x86/VQ_WMat_EC_sse4_1.c:39` `silk_VQ_WMat_EC_sse4_1`

`void silk_VQ_WMat_EC_sse4_1(opus_int8 *ind, opus_int32 *res_nrg_Q15, opus_int32 *rate_dist_Q8, opus_int *gain_Q7, const opus_int32 *XX_Q17, const opus_int32 *xX_Q17, const opus_int8 *cb_Q7, const opus_uint8 *cb_gain_Q7, const opus_uint8 *cl_Q5, const opus_int subfr_len, const opus_int32 max_gain_Q7, const opus_int L)` — **no arch argument**.

Only the **first row** of the `XX_Q17` product is vectorised (lines 88-98); rows 2..5, the loop over
the codebook and the "find best" block are literally `silk_VQ_WMat_EC_c` (`silk/VQ_WMat_EC.c:35`):

```c
        v_cb_row_31_Q7 = OP_CVTEPI8_EPI32_M32( &cb_row_Q7[ 1 ] );        /* sign-extend cb[1..4] */
        v_cb_row_42_Q7 = _mm_shuffle_epi32( v_cb_row_31_Q7, _MM_SHUFFLE(0,3,2,1) );  /* 0x1B: cb4,cb3,cb2,cb1 */
        v_cb_row_31_Q7 = _mm_mul_epi32( v_XX_31_Q17, v_cb_row_31_Q7 );    /* even lanes: XX1·cb1, XX3·cb3 */
        v_cb_row_42_Q7 = _mm_mul_epi32( v_XX_42_Q17, v_cb_row_42_Q7 );    /* even lanes: XX4·cb4, XX2·cb2 */
        v_acc1_Q24 = _mm_add_epi64( v_cb_row_31_Q7, v_cb_row_42_Q7);      /* [XX1cb1+XX4cb4, XX3cb3+XX2cb2] */
        v_acc2_Q24 = _mm_shuffle_epi32( v_acc1_Q24, _MM_SHUFFLE(1,0,3,2) );/* 0x4E: 64-bit [P1, P0] */
        v_acc1_Q24 = _mm_add_epi64( v_acc1_Q24, v_acc2_Q24);              /* P0 + P1 (int64) */
        sum2_Q24 = _mm_cvtsi128_si32( v_acc1_Q24 );                       /* low 32 bits */
        sum2_Q24 = silk_ADD32( neg_xX_Q24[ 0 ], sum2_Q24 );
        sum2_Q24 = silk_LSHIFT32( sum2_Q24, 1 );
        sum2_Q24 = silk_MLA( sum2_Q24,        XX_Q17[  0 ], cb_row_Q7[ 0 ] );
        sum1_Q15 = silk_SMLAWB( sum1_Q15,        sum2_Q24,  cb_row_Q7[ 0 ] );
```

* Exact per-iteration arithmetic: `sum2_Q24 = ( (XX1·cb1 + XX4·cb4) + (XX3·cb3 + XX2·cb2) )` computed
  in int64 then truncated to int32, `+ neg_xX_Q24[0]`, `<< 1`, `+ XX0·cb0`; then
  `sum1_Q15 = sum1_Q15 + silk_SMULWB(sum2_Q24, cb0)`. The C order is
  `neg + XX1cb1 + XX2cb2 + XX3cb3 + XX4cb4` (int32 wrapping) `<< 1` `+ XX0cb0`. Because every product
  fits in int32 and the final truncation is modulo 2³², the int64 intermediate is irrelevant:
  **bit-identical**.
* Rows 2-5 and the decision use the same scalar helpers as C (lines 101-125 vs
  `silk/VQ_WMat_EC.c:88-112`), including `bits_res_Q8 = silk_SMULBB(subfr_len, silk_lin2log(...) - (15<<7))`
  and `bits_tot_Q8 = silk_ADD_LSHIFT32(bits_res_Q8, cl_Q5[k], 3-1)`.
* **Bit-identical to `silk_VQ_WMat_EC_c`** for all inputs (the file's own `OPUS_CHECK_ASM` block
  asserts equality of all four outputs).

Call sites:

| file:line | enclosing function | side |
|---|---|---|
| `silk/quant_LTP_gains.c:84` | `silk_quant_LTP_gains` (per subframe/codebook) | **ENCODER** |

### 4.4 `silk_NSQ_del_dec` at arch 3 → `silk/x86/NSQ_del_dec_sse4_1.c:116` `silk_NSQ_del_dec_sse4_1`

Signature identical to `silk_NSQ_del_dec_c`, **no arch argument**. The file is a copy of
`silk/NSQ_del_dec.c` with:
* local typedefs `NSQ_del_dec_struct`/`NSQ_sample_struct`/`NSQ_sample_pair` (identical layout/members,
  `NSQ_del_dec_sse4_1.c:40-65` vs `NSQ_del_dec.c:40-58`);
* the **same** setup, `k==2` delayed-decision reset (including `Gains_Q16[1]`),
  winner selection, `silk_memcpy`-based state replacement and output writing;
* all of the per-sample math scalar and character-for-character identical to `NSQ_del_dec.c:402-559`,
  **except** the short-term prediction (lines 463-557), which is vectorised:

```c
                LPC_pred_Q14 = silk_RSHIFT( predictLPCOrder, 1 );    /* 5 or 8, added as bias */
                tmpb = _mm_setzero_si128();
                /* 4 consecutive groups of 4 taps, for predictLPCOrder==16 */
                psLPC_Q14_tmp = _mm_loadu_si128( &psLPC_Q14[-3] );
                psLPC_Q14_tmp = _mm_shuffle_epi32( psLPC_Q14_tmp, 0x1B );      /* [0,-1,-2,-3] */
                tmpa = _mm_mul_epi32( psLPC_Q14_tmp, a_Q12_0123 );             /* even lanes 0·a0, -2·a2 */
                tmpa = _mm_srli_epi64( tmpa, 16 );                             /* (prod)>>16 */
                tmpb = _mm_add_epi32( tmpb, tmpa );
                psLPC_Q14_tmp = _mm_shuffle_epi32( psLPC_Q14_tmp, 0x39 );      /* rotate: [-1,-2,-3,0] */
                a_Q12_tmp     = _mm_shuffle_epi32( a_Q12_0123,     0x39 );
                psLPC_Q14_tmp = _mm_mul_epi32( psLPC_Q14_tmp, a_Q12_tmp );     /* -1·a1, -3·a3 */
                psLPC_Q14_tmp = _mm_srli_epi64( psLPC_Q14_tmp, 16 );
                tmpb = _mm_add_epi32( tmpb, psLPC_Q14_tmp );
                /* ... same pattern for taps 4..7, 8..11 (16-tap only), 12..15 (16-tap only) ... */
                tmpa = _mm_shuffle_epi32( tmpb, _MM_SHUFFLE(0,0,3,2) );        /* 0x0E: [t2,t3,t0,t1] */
                tmpb = _mm_add_epi32( tmpb, tmpa );
                LPC_pred_Q14 += _mm_cvtsi128_si32( tmpb );                     /* lane0 = t0+t2 -> sum */
                /* for predictLPCOrder==10 only: */
                LPC_pred_Q14 = silk_SMLAWB( LPC_pred_Q14, psLPC_Q14[-8], a_Q12[8] );
                LPC_pred_Q14 = silk_SMLAWB( LPC_pred_Q14, psLPC_Q14[-9], a_Q12[9] );
                LPC_pred_Q14 = silk_LSHIFT( LPC_pred_Q14, 4 );                 /* Q10 -> Q14 */
```

* `a_Q12_0123/a_Q12_4567/a_Q12_89AB/a_Q12_CDEF` are the 4-tap groups loaded as int32 and pre-shuffled
  (lines 340-360 of the file) so that the even `_mm_mul_epi32` lanes pick `(x[-i]·a_i)` pairs.
  `_mm_srli_epi64(prod,16)` puts bits 16..47 of each 32×16 product into the low 32-bit lane, which is
  exactly `silk_SMULWB(x, a)` modulo 2³² (the high lane of the 64-bit product is *not* used:
  the final reduction only reads lanes 0 after the `0x0E` shuffle).
* Per-tap grouping: after each step, lane 0 accumulates taps `{0,1,4,5,8,9,12,13}` and lane 2
  accumulates taps `{2,3,6,7,10,11,14,15}`; the final `shuffle_epi32(0x0E)` + add + `cvtsi128_si32`
  yields `lane0 + lane2` = all taps. Every term is `silk_SMULWB(psLPC_Q14[-i], a_Q12[i])` and the sum
  is a plain 32-bit sum of those terms plus `RSHIFT(order,1)`, so **bit-identical to the C
  `silk_noise_shape_quantizer_short_prediction`** (associativity modulo 2³²).
* **Bit-identical to `silk_NSQ_del_dec_c` overall** (only this prediction changed, and it is exact).

Call sites: the dispatched `silk_NSQ_del_dec` macro — see §4.5.

### 4.5 `silk_NSQ_del_dec` at arch 4 → `silk/x86/NSQ_del_dec_avx2.c:364` `silk_NSQ_del_dec_avx2`

`void silk_NSQ_del_dec_avx2(const silk_encoder_state*, silk_nsq_state*, SideInfoIndices*, const opus_int16 x16[], opus_int8 pulses[], const opus_int16 *PredCoef_Q12, const opus_int16 LTPCoef_Q14[25], const opus_int16 AR_Q13[MAX_NB_SUBFR*24], const opus_int HarmShapeGain_Q14[4], const opus_int Tilt_Q14[4], const opus_int32 LF_shp_Q14[4], const opus_int32 Gains_Q16[4], const opus_int32 pitchL[4], const opus_int Lambda_Q10, const opus_int LTP_scale_Q14)` — **no arch argument**.

This is the only kernel in this configuration that is a *rewrite* rather than a transcription, so it
needs the most care.

#### Runtime fallback (mandatory to reproduce)

```c
    if (!verify_assumptions(psEncC))
    {
        silk_NSQ_del_dec_c(psEncC, NSQ, psIndices, x16, pulses, PredCoef_Q12, LTPCoef_Q14, AR_Q13,
                           HarmShapeGain_Q14, Tilt_Q14, LF_shp_Q14, Gains_Q16, pitchL,
                           Lambda_Q10, LTP_scale_Q14);
        return;
    }
```

`verify_assumptions` (`NSQ_del_dec_avx2.c:45-71`) asserts compile-time invariants
(`MAX_DEL_DEC_STATES ≤ 4`, `MAX_FRAME_LENGTH % 4 == 0`, `MAX_SUB_FRAME_LENGTH % 4 == 0`,
`LTP_MEM_LENGTH_MS % 4 == 0`, `fs_kHz ∈ {8,12,16}`, `0 < nb_subfr ≤ MAX_NB_SUBFR`,
`0 < nStatesDelayedDecision ≤ MAX_DEL_DEC_STATES`, `ltp_mem_length == fs_kHz·LTP_MEM_LENGTH_MS`) and
then

```c
    return psEncC->nStatesDelayedDecision == 3 ||
           psEncC->nStatesDelayedDecision == 4;
```

So the AVX2 kernel is used **only for `nStatesDelayedDecision ∈ {3,4}`** (silk complexity 7 and 8+,
including the default 9/10); for 1 or 2 it delegates to `silk_NSQ_del_dec_c`. `OPUS_CHECK_ASM` is not
defined, so the `silk_NSQ_del_dec_c` cross-check block is absent.

#### Data layout (4 states in the 4 lanes of every `__m128i`)

```c
typedef struct { __m128i RandState, Q_Q10, Xq_Q14, Pred_Q15, Shape_Q14; } NSQ_del_dec_sample_struct;
typedef struct {
    __m128i sLPC_Q14[MAX_SUB_FRAME_LENGTH + NSQ_LPC_BUF_LENGTH];
    __m128i LF_AR_Q14, Seed, SeedInit, RD_Q10;
    __m128i Diff_Q14;
    __m128i sAR2_Q14[MAX_SHAPE_LPC_ORDER];
    NSQ_del_dec_sample_struct Samples[DECISION_DELAY];
} NSQ_del_dec_struct;
```

Lane `k` = delayed-decision state `k` (states ≥ `nStatesDelayedDecision` are kept but masked out of
the min/max decisions by `MaskDelDec`). `Samples[]` holds one `__m128i` per field for all 4 states
(C's `psDelDec[k].Q_Q10[idx]` etc. coalesced vertically).

Initialisation (maps 1:1 onto `NSQ_del_dec.c:152-165`):

```
MaskDelDec       = cvtepi8_epi32(cvtsi32_si128(0xFFFFFF00ul << ((nStates-1)<<3)))   /* lane k = (k>=nStates) ? -1 : 0 */
lag              = NSQ->lagPrev
Seed[lane k]     = (psIndices->Seed + k) & 3        /* C: (k + Seed) & 3 */
SeedInit         = Seed ;  RD_Q10 = 0
LF_AR_Q14        = broadcast(NSQ->sLF_AR_shp_Q14)
Diff_Q14         = broadcast(NSQ->sDiff_shp_Q14)
Samples[0].Shape_Q14 = broadcast(NSQ->sLTP_shp_Q14[ltp_mem_length-1])
sLPC_Q14[i]      = broadcast(NSQ->sLPC_Q14[i])      for i < NSQ_LPC_BUF_LENGTH
sAR2_Q14[i]      = broadcast(NSQ->sAR2_Q14[i])      for i < MAX_SHAPE_LPC_ORDER
offset_Q10 = silk_Quantization_Offsets_Q10[signalType>>1][quantOffsetType] ;  smpl_buf_idx = 0
decisionDelay = min(DECISION_DELAY, subfr_length); if VOICED: min over k of (pitchL[k]-LTP_ORDER/2-1)
                else if lag > 0: min(decisionDelay, lag - LTP_ORDER/2 - 1)
A_Q12 = &PredCoef_Q12[((k>>1) | (1 ^ LSF_interpolation_flag)) * MAX_LPC_ORDER]
```

The `k == 2` re-whitening reset (`NSQ_del_dec_avx2.c:495-537`) implements `NSQ_del_dec.c:214-245`:
winner = first lane with the minimum `RD_Q10` among unmasked lanes
(`silk_mm_mask_hmin_epi32` = `blendv(num, INT32_MAX, mask)` then min-reduce),
then `RD_Q10[i] += INT32_MAX>>4` for every lane except the winner, then
```
pulses[i-decisionDelay] = sar_round(winner Q_Q10, 10)
pxq[i-decisionDelay]    = sat16( sar_round_smulww(winner Xq_Q14, Gains_Q16[1], 14) )
sLTP_shp_Q14[buf_idx - decisionDelay + i] = winner Shape_Q14
```
and the re-whitening itself uses the file-local `silk_LPC_analysis_filter_avx2` (§4.5.4).

#### 4.5.1 Per-sample common computations

Exact scalar translation of `silk_noise_shape_quantizer_del_dec_avx2`'s prologue and the
state-independent part (`NSQ_del_dec_avx2.c:694-726`); the C numbers in comments are the
`silk_NSQ_del_dec_c` equivalents:

```
if (signalType == TYPE_VOICED):
    LTP_pred_Q14  = 2
    LTP_pred_Q14 += silk_SMULWB(pred_lag_ptr[ 0], b_Q14[0])
    LTP_pred_Q14 += silk_SMULWB(pred_lag_ptr[-1], b_Q14[1])
    LTP_pred_Q14 += silk_SMULWB(pred_lag_ptr[-2], b_Q14[2])
    LTP_pred_Q14 += silk_SMULWB(pred_lag_ptr[-3], b_Q14[3])
    LTP_pred_Q14 += silk_SMULWB(pred_lag_ptr[-4], b_Q14[4])
    LTP_pred_Q14  = LTP_pred_Q14 << 1            /* Q13 -> Q14 */
    pred_lag_ptr++
else: LTP_pred_Q14 = 0

if (lag > 0):
    n_LTP_Q14 = silk_add_sat32(shp_lag_ptr[0], shp_lag_ptr[-2])
    n_LTP_Q14 = silk_SMULWB(n_LTP_Q14, HarmShapeFIRPacked_Q14)
    n_LTP_Q14 = n_LTP_Q14 + silk_SMULWT(shp_lag_ptr[-1], HarmShapeFIRPacked_Q14)
    n_LTP_Q14 = LTP_pred_Q14 - (n_LTP_Q14 << 2)  /* signed 32-bit subtract, no saturation */
    shp_lag_ptr++
else: n_LTP_Q14 = 0
```

Note this is a Q14 restatement of C's `LTP_pred_Q13`/`n_LTP_Q14` Q13 arithmetic; because
`silk_SMULWB` is exact and all subsequent ops are exact modulo 2³² (the `<<1`/`<<2` included), the
values agree with `NSQ_del_dec.c:394-400` for every input representable without overflow.

#### 4.5.2 Per-state computation (vectorised across the 4 lanes)

Short-term prediction helper `silk_noise_shape_quantizer_short_prediction_x4`
(`NSQ_del_dec_avx2.c:600-628`) — the crucial bias trick:

```c
    out = _mm256_set1_epi32(order >> 1);                     /* ALL 8 32-bit lanes, i.e. both 32-bit
                                                                halves of each 64-bit lane */
    out = _mm256_add_epi32(out, _mm256_mul_epi32(_mm256_cvtepi32_epi64(buf32[-0]),
                             _mm256_set1_epi32(silk_LSHIFT(coef16[0], 16))));
    ... taps 1 .. order-1 identically ...
    return silk_cvtepi64_epi32_high(out);
```

* `_mm256_mul_epi32(cvtepi32_epi64(buf), set1(coef<<16))` builds, in each 64-bit lane,
  `P = (int64)buf · (coef·2¹⁶)`; the **high 32 bits** of `P` are `(buf·coef) >> 16 = silk_SMULWB(buf, coef)`.
* `_mm256_add_epi32` adds the 32-bit halves independently, so the high halves accumulate
  `Σ_i silk_SMULWB(buf_i, coef_i)` while the low halves accumulate garbage that is discarded.
  Because `_mm256_set1_epi32(order>>1)` also initialised the *high* halves, the extracted high
  halves give exactly `(order>>1) + Σ_i silk_SMULWB(...)` — the same value as C's
  `silk_noise_shape_quantizer_short_prediction_c` (`silk/NSQ.h:35-63`).
* `silk_cvtepi64_epi32_high` = `permutevar8x32(num, (0,0,0,0,7,5,3,1))` then take the low 128 bits =
  the high 32 bits of the four 64-bit lanes.
* The kernel result is then `LPC_pred_Q14 = _mm_slli_epi32(result, 4)` (Q10→Q14), matching C.

Then, per lane `p = 0..3` (lane = delayed-decision state) — note `tmp0`, `tmp1`, `n_AR_Q14`,
`n_LF_Q14` are `__m128i` vectors, so the AR loop below advances **all four lanes simultaneously** at
each tap `j`, and `X[p]` denotes lane `p` of a vector `X`:

```
/* Noise-shape feedback, exact scalar equivalent of lines 741-761 */
tmp0[p] = Diff_Q14[p] + SMULWB(sAR2_Q14[0][p], warping_Q16)
n_AR_Q14[p] = (shapingLPCOrder >> 1)
for j = 0 .. shapingLPCOrder-2:
    tmp1       = sAR2_Q14[j]                              /* whole vector, saved */
    sAR2_Q14[j] = tmp0
    n_AR_Q14[p] += SMULWB(tmp0[p], AR_shp_Q13[j])
    tmp0[p]     = tmp1[p] + SMULWB(sAR2_Q14[j+1][p] - tmp0[p], warping_Q16)
sAR2_Q14[shapingLPCOrder-1] = tmp0
n_AR_Q14[p] += SMULWB(tmp0[p], AR_shp_Q13[shapingLPCOrder-1])
n_AR_Q14[p] = (n_AR_Q14[p] << 1) + SMULWB(LF_AR_Q14[p], Tilt_Q14)     /* Q11->Q12 then tilt */
n_AR_Q14[p] = n_AR_Q14[p] << 2                                        /* Q12 -> Q14 */
n_LF_Q14[p] = ( SMULWB(Samples[smpl_buf_idx].Shape_Q14[p], LF_shp_Q14)
              + SMULWB(LF_AR_Q14[p], LF_shp_Q14 >> 16) ) << 2         /* Q12 -> Q14 */

/* residual, lines 763-774 */
tmp0 = add_sat32(n_AR_Q14, n_LF_Q14)                    /* silk_mm_add_sat_epi32 */
tmp1 = n_LTP_Q14 + LPC_pred_Q14                         /* wrapping add_epi32  */
tmp0 = sub_sat32(tmp1, tmp0)                            /* silk_mm_sub_sat_epi32 */
tmp0 = (tmp0 + 8) >> 4                                  /* silk_mm_srai_round_epi32(.,4); C: RSHIFT_ROUND(x,4) */
r_Q10 = x_Q10[i] - tmp0                                 /* _mm_sub_epi32(x, tmp0) */
r_Q10 = (Seed[p] < 0) ? -r_Q10 : r_Q10                  /* silk_mm_sign_epi32 */
r_Q10 = max(-(31<<10), min(30<<10, r_Q10))              /* silk_mm_limit_epi32 */
```

* `silk_add_sat32`/`silk_mm_add_sat_epi32`/`silk_mm_sub_sat_epi32`/`silk_mm256_sub_sat_epi32`
  reproduce `silk_ADD_SAT32`/`silk_SUB_SAT32` exactly (overflow detected as
  `(a^r)&(b^r)` / `(a^r)&~(b^r)`, saturated to `(a>>31) + 0x7FFFFFFF` = INT32_MIN/MAX).
* **`silk_SMLAWT` replaced by two `SMULWB`s (n_LF):** C computes
  `n_LF_Q14 = silk_SMLAWT(SMULWB(Shape,LF_shp_Q14), LF_AR_Q14, LF_shp_Q14)`, whose increment is
  `(LF_AR_Q14 · (int64)(LF_shp_Q14>>16)) >> 16` (`silk/macros.h:64`), while the AVX2 adds
  `silk_mm_smulwb_epi32(LF_AR_Q14, LF_shp_Q14 >> 16) = (LF_AR_Q14 · (int16)(LF_shp_Q14>>16)) >> 16`.
  These agree because `|LF_shp_Q14| < 2³¹` (it is a Q14 shaping coefficient ≤ ~2¹⁶), so the `(int16)`
  cast is the identity. The same remark applies to the `silk_SMLAWB → silk_mm_smulwb_epi32`
  substitutions elsewhere in the kernel, where the coefficient is always an `opus_int16`.
* `silk_mm_smulwb_epi32(a,b) = cvtepi64_epi32_high(mul_epi32(cvtepi32_epi64(a), set1(b<<16)))`
  implements `(a · (int16)b) >> 16` = `silk_SMULWB` (low-32 truncation identical).
* `silk_mm_smulww_epi32(a,b) = cvtepi64_epi32_high(slli_epi64(mul_epi32(cvtepi32_epi64(a), set1(b)), 16))`
  implements `(int32)(((int64)a·b) >> 16)` = `silk_SMULWW` exactly.
* `silk_mm256_smulbb_epi32(a,b)` gathers the low 16 bits of each 32-bit lane via a byte shuffle over
  `mullo_epi16`/`mulhi_epi16` and re-interleaves, giving `(int16)a_i · (int16)b_i` per lane
  (`silk_SMULBB`). It is exact only if `|a_i| < 2¹⁵` etc., which holds for `|q|,|rr| < 2¹⁵`.
* `silk_mm_srai_round_epi32(a,bits) = srai_epi32(add_epi32(a, 1<<(bits-1)), bits)`. This equals the
  C `silk_RSHIFT_ROUND` except when `a + 2^(bits-1)` overflows int32 — impossible for the values used
  here (`Q_Q10` magnitudes ≤ ~2¹⁵, `r_Q10` ≤ 2¹⁵).

Quantisation decision (lines 776-817), exactly as C (`NSQ_del_dec.c:464-518`):

```
q1_Q10 = r_Q10 - offset_Q10 ;  q1_Q0 = q1_Q10 >> 10
if (Lambda_Q10 > 2048):
    tmp0 = |q1_Q10| - (Lambda_Q10/2 - 512)
    q1_Q0 = (q1_Q10 >> 31)                                   /* -1 if negative else 0 */
    if (tmp0 > 0): q1_Q0 = (sign(tmp0, q1_Q10)) >> 10        /* == C's rdo_offset branches */
tmp0 = sign(QUANT_LEVEL_ADJUST_Q10, q1_Q0)                   /* q1_Q0<0 ? -80 : (q1_Q0==0 ? 0 : 80) */
q1_Q10 = (q1_Q0 << 10) - tmp0 + offset_Q10
t = (q1_Q0 >>> 31) + q1_Q0
q2_Q10 = q1_Q10 + ((t == 0) ? (1024 - QUANT_LEVEL_ADJUST_Q10) : 1024)
q_Q10  = [q1_Q10 (lanes 0..3), q2_Q10 (lanes 4..7)]           /* _mm256_set_m128i(q2,q1) */
rr_Q10 = broadcast(r_Q10) - q_Q10
rd_Q10 = smulbb(|q_Q10|, Lambda_Q10) + smulbb(rr_Q10, rr_Q10) ;  rd_Q10 >>= 10 (arithmetic)
mask   = broadcast(rd_Q10.lo128 < rd_Q10.hi128)
SS_RD_Q10 = broadcast(psDelDec->RD_Q10) + blendv(swap128(rd_Q10), rd_Q10, mask)
SS_Q_Q10  = blendv(swap128(q_Q10), q_Q10, mask)
```

where `swap128(x) = permute2x128(x,x,0x1)`. This reproduces C's
`if (rd1_Q10 < rd2_Q10) {RD0=RD+rd1,Q0=q1; RD1=RD+rd2,Q1=q2} else {…}` including the linear-part sign
conventions, because `|q|` matches C's branch-wise `SMULBB(±q, Λ)` (all four branches keep `q` and
`q+1024` on the same side of zero).

State update for both candidate sets (lines 819-832), identical in form to `NSQ_del_dec.c:520-558`:

```
exc_Q14    = (Seed < 0 ? -1 : 1) * (SS_Q_Q10 << 4)
SS_LPC_exc = (exc_Q14 + LTP_pred_Q14) << 1
SS_xq_Q14  = exc_Q14 + LTP_pred_Q14 + LPC_pred_Q14
SS_Diff_Q14      = SS_xq_Q14 - (x_Q10[i] << 4)
SS_LF_AR_Q14     = SS_Diff_Q14 - n_AR_Q14
SS_sLTP_shp_Q14  = sub_sat32(SS_LF_AR_Q14, n_LF_Q14)
```

Winner / state-replacement machinery (lines 836-925), exactly mirroring
`NSQ_del_dec.c:561-638`:

```
smpl_buf_idx   = (smpl_buf_idx + DECISION_DELAY - 1) % DECISION_DELAY
last_smple_idx = (smpl_buf_idx + decisionDelay) % DECISION_DELAY
psLastSample   = &Samples[last_smple_idx]
RDmin  = min over unmasked lanes of SS_RD_Q10.lo128
Winner = index of the first lane equal to RDmin
Winner_rand_state = psLastSample->RandState[Winner]
for each lane p: if (RandState[p] != Winner_rand_state) SS_RD_Q10[p] += (INT32_MAX>>4)
RDmax     = max over unmasked lanes of SS_RD_Q10[0..3]        /* silk_mm_mask_hmax_epi32 */
RDmin2    = min over unmasked lanes of SS_RD_Q10[4..7]        /* silk_mm_mask_hmin_epi32 */
if (RDmin2 < RDmax):
    RDmax_ind = first lane equal to RDmax in [0..3]
    RDmin_ind = first lane equal to RDmin2 in [4..7]
    /* build the byte-shuffle selector that copies lane RDmin_ind into lane RDmax_ind */
    apply it to sLPC_Q14[t] for t in [i, MAX_SUB_FRAME_LENGTH+NSQ_LPC_BUF_LENGTH),
                Seed, SeedInit, sAR2_Q14[0..MAX_SHAPE_LPC_ORDER),
                and Samples[t].{RandState,Q_Q10,Xq_Q14,Pred_Q15,Shape_Q14} for t < DECISION_DELAY
    permute SS_{Q_Q10,RD_Q10,xq_Q14,LF_AR_Q14,Diff_Q14,sLTP_shp_Q14,LPC_exc_Q14} so that
    lane RDmax_ind takes the value of lane (RDmin_ind + 4)
   /* note: sLPC_Q14 is shuffled from index i only — same as C's `+ i` memcpy offset */
if (subfr > 0 || i >= decisionDelay):
    pulses[i-decisionDelay] = sar_round(select(psLastSample->Q_Q10, Winner), 10)
    xq[i-decisionDelay]     = sat16(sar_round_smulww(select(psLastSample->Xq_Q14, Winner),
                                                    delayedGain_Q10[last_smple_idx], 8))
    sLTP_shp_Q14[buf_idx-decisionDelay] = select(psLastSample->Shape_Q14, Winner)
    sLTP_Q15[buf_idx-decisionDelay]     = select(psLastSample->Pred_Q15, Winner)
sLTP_shp_buf_idx++ ; sLTP_buf_idx++
psSample = &Samples[smpl_buf_idx]
Seed        = Seed + srai_round(SS_Q_Q10.lo128, 10)     /* C: ADD32_ovflw(Seed, RSHIFT_ROUND(Q_Q10,10)) */
LF_AR_Q14   = SS_LF_AR_Q14.lo128
Diff_Q14    = SS_Diff_Q14.lo128
sLPC_Q14[i+NSQ_LPC_BUF_LENGTH] = SS_xq_Q14.lo128
RD_Q10      = SS_RD_Q10.lo128
psSample->{Xq_Q14,Q_Q10,Pred_Q15,Shape_Q14,RandState} = {SS_xq_Q14,SS_Q_Q10,SS_LPC_exc,SS_sLTP_shp_Q14,Seed}
delayedGain_Q10[smpl_buf_idx] = Gain_Q10 = Gain_Q16 >> 6
```

Finalisation (`NSQ_del_dec_avx2.c:553-589`) mirrors `NSQ_del_dec.c:273-307`: global winner from
`RD_Q10`, `psIndices->Seed = SeedInit[winner]`, `Gain_Q10 = Gains_Q16[nb_subfr-1] >> 6`, the same
`decisionDelay` output loop with `sar_round_smulww(..., Gain_Q10, 8)`, copying
`sLPC_Q14[i]` (already shifted by the end-of-subframe loop, line 927-930, so it corresponds to C's
`&psDD->sLPC_Q14[subfr_length]`) and `sAR2_Q14`, then the same `silk_memmove`s.

#### 4.5.3 Helpers and their exact semantics

| helper (`NSQ_del_dec_avx2.c`) | semantics |
|---|---|
| `silk_cvtepi64_epi32_high(v)` :88 | take the high 32 bits of each of the four 64-bit lanes |
| `silk_sat16(x)` :93 | `min(max(x,-32768),32767)` |
| `silk_sar_round_32(a,bits)` :100 | `(a + (1<<(bits-1))) >> bits` (arithmetic shift) |
| `silk_sar_round_smulww(a,b,bits)` :107 | **see the warning below** |
| `silk_add_sat32` :125 | `silk_ADD_SAT32` via `__builtin_sadd_overflow` |
| `silk_mm_srai_round_epi32` :135 | `(a + (1<<(bits-1))) >> bits` per lane |
| `silk_mm_add_sat_epi32` / `silk_mm_sub_sat_epi32` / `silk_mm256_sub_sat_epi32` :142,149,156 | 32-bit saturating add/sub per lane |
| `silk_mm_limit_epi32` :164 | `min(max(num,lo),hi)` with `lo=min(l1,l2)`, `hi=max(l1,l2)` |
| `silk_mm_sign_epi32` / `silk_mm256_sign_epi32` :175,179 | `cond<0 ? -num : num` (`_mm_sign_epi32(num, cond|1)`) |
| `silk_mm_smulww_epi32` :185 | `silk_SMULWW` (exact, incl. int32 truncation) |
| `silk_mm_smulwb_epi32` :191 | `silk_SMULWB` (exact) |
| `silk_mm256_smulbb_epi32` :197 | `silk_SMULBB` per lane (int16×int16) |
| `silk_mm256_reverse_epi32` :210 | reverse the 8 32-bit lanes |
| `silk_mm256_hsum_epi32` :217 | `((v0+v4)+(v2+v6)) + ((v1+v5)+(v3+v7))` (mod 2³², order irrelevant) |
| `silk_mm_hmin_epi32`/`hmax`, `mask_hmin`/`mask_hmax` :225-249 | horizontal min/max with `blendv` masking to ±INT32_MAX |
| `silk_mm256_rand_epi32` :251 | `seed·RAND_MULTIPLIER + RAND_INCREMENT` per lane (`mullo/add_epi32`, mod 2³²) = `silk_RAND` |
| `silk_index_of_first_equal_epi32` :258 | `ctz(movemask(cmpeq(a,b)) & 0x1111) >> 2` = index of first equal lane |
| `silk_index_to_selector` :265 | `_mm_set_epi8(idx+3,idx+2,idx+1,idx+0, ×4)` = the byte selector copying that lane |
| `silk_select_winner` :276 | lane extraction with a selector |

> **`silk_sar_round_smulww` is the one place where the AVX2 kernel is documented to differ from C.**
> ```c
> static OPUS_INLINE opus_int64 silk_sar_round_smulww(opus_int32 a, opus_int32 b, int bits)
> {
> #ifdef OPUS_CHECK_ASM
>     return silk_RSHIFT_ROUND(silk_SMULWW(a, b), bits);
> #else
>     /* This code is more correct, but it won't overflow like the C code in some rare cases. */
>     t = ((opus_int64)a) * ((opus_int64)b);
>     bits += 16;
>     t += 1ull << (bits-1);
>     return t >> bits;
> #endif
> }
> ```
> With `OPUS_CHECK_ASM` undefined (this build) the int64 form is used: `((int64)a·b + 2^(bits+15)) >> (bits+16)`.
> The C reference computes `silk_RSHIFT_ROUND((opus_int32)(((int64)a·b) >> 16), bits)`, which first
> **truncates the intermediate to int32**. The two agree whenever `((int64)a·b) >> 16` fits in int32
> (which is the normal case) and differ when it does not. This function is used for the reconstructed
> `xq`/`pxq` samples (lines 520, 569, 903), which feed back into the synthesis state, so on such
> inputs the AVX2 result can diverge from `silk_NSQ_del_dec_c`.

#### 4.5.4 `silk_LPC_analysis_filter_avx2` (file-local, used for re-whitening)

```c
    for (i = order; i < len; i++ ) {
        in_v = _mm256_cvtepi16_epi32( _mm_loadu_si128( &in_ptr[-8] ) );      /* in[i-8 .. i-1] */
        B_v  = _mm256_cvtepi16_epi32( _mm_loadu_si128( &B[0] ) );            /* B[0..7]      */
        sum  = _mm256_mullo_epi32( in_v, silk_mm256_reverse_epi32(B_v) );
        if (order > 10) {
            in_v = _mm256_cvtepi16_epi32( _mm_loadu_si128( &in_ptr[-16] ) ); /* in[i-16..i-9] */
            B_v  = _mm256_reverse_epi32( _mm256_cvtepi16_epi32( _mm_loadu_si128( &B[8] ) ) );
        } else {
            in_v = _mm256_cvtepi16_epi32( _mm_loadu_si32( &in_ptr[-10] ) );  /* in[i-10], in[i-9] */
            B_v  = _mm256_shuffle_epi32( _mm256_cvtepi16_epi32( _mm_loadu_si32( &B[8] ) ), 0x01 );
        }
        sum = _mm256_add_epi32( sum, _mm256_mullo_epi32( in_v, B_v ) );
        out32_Q12 = silk_mm256_hsum_epi32( sum );
        out32_Q12 = silk_SUB32_ovflw( silk_LSHIFT( (opus_int32)*in_ptr, 12 ), out32_Q12 );
        out32     = silk_sar_round_32( out32_Q12, 12 );
        out[ i ]  = silk_sat16( out32 );
    }
    silk_memset( out, 0, order * sizeof( opus_int16 ) );
```

* The lane products pair `in[i-8+j]·B[7-j]` and `in[i-16+j]·B[15-j]` (or `in[i-10]·B[9]`,
  `in[i-9]·B[8]` for order 10), so the sum is exactly `Σ_{m=1..order} in[i-m]·B[m-1]` =
  `silk_SMULBB`-chain of `silk_LPC_analysis_filter` (`silk/LPC_analysis_filter.c:71-90`).
  `mullo_epi32` products of int16 values are exact, and the grouping is irrelevant modulo 2³².
* `out32_Q12 = (in[i]<<12) - sum` (wrapping) and `(out32_Q12 + 2048) >> 12` equal C's
  `silk_SUB32_ovflw`/`silk_RSHIFT_ROUND(...,12)` (again up to int32 overflow of the `+2048`).
* **Bit-identical to the C `silk_LPC_analysis_filter` for all non-pathological inputs**; the C code
  explicitly allows wrap-around for "invalid streams" anyway.

#### 4.5.5 `silk_nsq_del_dec_scale_states_avx2`

Line-for-line equivalent of `silk_NSQ_del_dec.c:648-729` with two vectorised loops:

* `x_sc_Q10[i] = silk_SMULWW(x16[i], inv_gain_Q26)` for `i += 4` over the whole `subfr_length`
  (guaranteed multiple of 4 by `MAX_SUB_FRAME_LENGTH % 4 == 0`), via
  `cvtepi16_epi64`+`mul_epi32`+`slli_epi64 16`+high-32 extraction (§2.2 style) = exact `silk_SMULWW`.
  `inv_gain_Q26 = silk_sar_round_32(inv_gain_Q31,5)`; `inv_gain_Q31 > 0`, so identical to
  `silk_RSHIFT_ROUND`.
* the `sLTP_shp_Q14` rescaling loop strides 4 from `sLTP_shp_buf_idx - ltp_mem_length` while
  `i < sLTP_shp_buf_idx` — it can write up to 3 entries *past* the loop bound, which is harmless
  (those slots are overwritten later in the subframe).
* `sLTP_Q15[i] = ((int64)sLTP_Q15[i] · (int64)gain_adj_Q16) >> 16` (line 999) = C's `silk_SMULWW`
  (the assignment truncates to opus_int32, matching C's cast).
* `psSample->Pred_Q15` / `Shape_Q14` and all scalar states use `silk_mm_smulww_epi32` (§4.5.3) = exact.

#### 4.5.6 Bit-exactness verdict for `silk_NSQ_del_dec_avx2`

Same value as `silk_NSQ_del_dec_c` for the overwhelming majority of inputs, but **NOT provably
bit-identical**:

1. `silk_sar_round_smulww` (lines 107-123, used at 520, 569, 903) deliberately does *not* reproduce
   the int32 truncation of the C `silk_SMULWW + silk_RSHIFT_ROUND` combination; the source comment
   states this outright. Divergence requires the intermediate `(int64)Xq_Q14 · gain` to overflow
   int32 after `>>16`, which the author calls "rare".
2. `silk_mm_srai_round_epi32` / `silk_sar_round_32` use `(a + 2^(bits-1)) >> bits`; the C
   `silk_RSHIFT_ROUND` is `((a >> (bits-1)) + 1) >> 1`, which differs if the addition overflows
   int32 (not reachable for the values used).
3. The whole per-state computation is re-derived in Q14 with saturating add/sub; the author's
   range analysis (and the unreachable `silk_assert`s) make it equal, but unlike the SSE4.1 version
   it is a rewrite rather than a transcription, so it should be validated per-vector on real data.

For `nStatesDelayedDecision ∈ {1,2}` the function *is* `silk_NSQ_del_dec_c` (fallback), so a Rust port
that follows the C algorithm in that case is exact.

Call sites of the `silk_NSQ_del_dec` dispatch (arch 3/4):

| file:line | enclosing function | side | entry used |
|---|---|---|---|
| `silk/float/wrappers_FLP.c:164` | `silk_NSQ_wrapper_FLP` (`nStatesDelayedDecision > 1 \|\| warping_Q16 > 0`) | **ENCODER** | arch 4 → `silk_NSQ_del_dec_avx2` |
| `silk/fixed/encode_frame_FIX.c:208, 439` | — | not compiled | — |

### 4.6 `silk_inner_product_FLP` → `silk/float/x86/inner_product_FLP_avx2.c:38` `silk_inner_product_FLP_avx2`

`double silk_inner_product_FLP_avx2(const silk_float *data1, const silk_float *data2, opus_int dataSize)`
— **no arch argument**; `silk_float = float`, result `double`. This file **is** compiled with
`-mavx -mfma -mavx2`.

```c
    opus_int i;
    __m256d accum1, accum2;
    double   result;
    result = 0.0;
    accum1 = accum2 = _mm256_setzero_pd();
    for( i = 0; i < dataSize - 7; i += 8 ) {
        x1f = _mm_loadu_ps( &data1[ i ] );      x2f = _mm_loadu_ps( &data2[ i ] );
        x1d = _mm256_cvtps_pd( x1f );           x2d = _mm256_cvtps_pd( x2f );
        accum1 = _mm256_fmadd_pd( x1d, x2d, accum1 );          /* elements i .. i+3   */
        x1f = _mm_loadu_ps( &data1[ i + 4 ] );  x2f = _mm_loadu_ps( &data2[ i + 4 ] );
        x1d = _mm256_cvtps_pd( x1f );           x2d = _mm256_cvtps_pd( x2f );
        accum2 = _mm256_fmadd_pd( x1d, x2d, accum2 );          /* elements i+4 .. i+7 */
    }
    for( ; i < dataSize - 3; i += 4 ) {
        ... accum1 = _mm256_fmadd_pd( x1d, x2d, accum1 );      /* elements i .. i+3   */
    }
    accum1 = _mm256_add_pd(accum1, accum2);
    accum1 = _mm256_add_pd(accum1, _mm256_permute2f128_pd(accum1, accum1, 1));
    accum1 = _mm256_hadd_pd(accum1,accum1);
    result = _mm256_cvtsd_f64(accum1);
    for( ; i < dataSize; i++ ) {
        result += data1[ i ] * (double)data2[ i ];
    }
    return result;
```

* `_mm256_cvtps_pd` keeps element order: lane `j` of `accum1` accumulates `data1[i+j]·data2[i+j]`
  for `j = 0..3` of every 8-element block, lane `j` of `accum2` the `j = 4..7` halves.
  The first loop handles whole groups of 8 (`i < dataSize-7`); the second handles groups of 4 that
  are not part of a group of 8 (`i < dataSize-3`, always into `accum1`); the remainder (< 4) is scalar.
* Every accumulation is a **fused** `fma` (`_mm256_fmadd_pd`), i.e. the float→double product is exact
  anyway, but the addition is a single correctly-rounded double add of the exact product — identical
  to the C idiom `result += data1[i]*(double)data2[i]` per term.
* **Reduction order** (with `A = accum1 + accum2`, `A[j]` = lanes):
  `permute2f128_pd(A,A,1)` swaps the two 128-bit halves → `P = [A2, A3, A0, A1]`;
  `add_pd(A,P)` → `B = [A0+A2, A1+A3, A2+A0, A3+A1]`;
  `hadd_pd(B,B)` per 128-bit lane → `[B0+B1, B1+B0, B2+B3, B3+B2]`;
  `cvtsd_f64` takes lane 0. Final:
  ```
  result = ( (A0+A2) + (A1+A3) )
  ```
  followed by the scalar remainder added one term at a time.
* **Bit-exactness vs `silk_inner_product_FLP_c` (`silk/float/inner_product_FLP.c:35`): NOT identical**
  for `dataSize ≥ 8` — the C version accumulates `result` strictly left to right over all elements
  (with `result` a `double`). Only the accumulation *order* differs (the products themselves are
  exact in double in both versions), but that is enough to change the rounded result.

Call sites (all encoder-side FLP paths):

| file:line | enclosing function | side |
|---|---|---|
| `silk/float/pitch_analysis_core_FLP.c:294` | `silk_pitch_analysis_core_FLP` | **ENCODER** |
| `silk/float/autocorrelation_FLP.c:51` | `silk_autocorrelation_FLP` | **ENCODER** |
| `silk/float/burg_modified_FLP.c:64` | `silk_burg_modified_FLP` | **ENCODER** |
| `silk/float/corrMatrix_FLP.c:54` | `silk_corrMatrix_FLP` | **ENCODER** |
| `silk/float/corrMatrix_FLP.c:84` | `silk_corrMatrix_FLP` | **ENCODER** |

---

## 5. Complete call-site inventory for `celt/` and `silk/` (float + arch 4)

`arch` reaching these calls is always `st->arch` / `psEncC->arch` = `opus_select_arch()` = **4** for
CELT/SILK (set in `celt/celt_encoder.c:212` via `opus_custom_encoder_init_arch`,
`celt/celt_decoder.c:265`, `silk/init_decoder.c:53`, `src/opus_encoder.c:250`, `src/opus_decoder.c:182`,
`src/analysis.c:219`, `celt/modes.c:236,443`), except the one hard-coded `0` in
`src/opus_multistream_encoder.c:280` (harmless — the CELT kernels ignore `arch` in this build).

### 5.1 `celt_pitch_xcorr` (`PITCH_XCORR_IMPL`, arch 4 → `celt_pitch_xcorr_avx2`) — **BOTH**

| file:line | enclosing function | side |
|---|---|---|
| `celt/celt_lpc.c:344` | `_celt_autocorr` | BOTH (enc `celt/pitch.c:189`; dec `celt/celt_decoder.c:859`) |
| `celt/pitch.c:361` | `pitch_search` | BOTH (enc `celt_encoder.c:1468`; dec PLC `celt_decoder.c:568`) |
| `silk/float/pitch_analysis_core_FLP.c:181` | `silk_pitch_analysis_core_FLP` | ENCODER |
| `silk/float/pitch_analysis_core_FLP.c:533` | `silk_pitch_analysis_core_FLP` | ENCODER |

### 5.2 `xcorr_kernel` (`xcorr_kernel_sse`) — **DECODER** (in this build's live paths)

| file:line | enclosing function | side |
|---|---|---|
| `celt/celt_lpc.c:174` | `celt_fir_c` | DECODER (`celt_decoder.c:901`) |
| `celt/celt_lpc.c:248` | `celt_iir` | DECODER (`celt_decoder.c:970`) |
| `celt/pitch.c:275` | `celt_pitch_xcorr_c` | only via `PITCH_XCORR_IMPL[0..3]`, unreachable at arch 4 |

### 5.3 `celt_inner_prod` (`celt_inner_prod_sse`) — **BOTH**

| file:line | enclosing function | side |
|---|---|---|
| `celt/bands.c:160` | `compute_band_energies` | ENCODER |
| `celt/pitch.c:295` | `celt_pitch_xcorr_c` | unreachable at arch 4 |
| `celt/pitch.c:384` | `pitch_search` | BOTH |
| `celt/pitch.c:545` | `remove_doubling` | ENCODER |
| `celt/x86/pitch_avx.c:97` | `celt_pitch_xcorr_avx2` tail (< 8 lags) | BOTH |

### 5.4 `dual_inner_prod` (`dual_inner_prod_sse`) — **ENCODER**

| file:line | enclosing function | side |
|---|---|---|
| `celt/pitch.c:480` | `remove_doubling` | ENCODER |
| `celt/pitch.c:513` | `remove_doubling` | ENCODER |

### 5.5 `comb_filter_const` (`comb_filter_const_sse`) — **BOTH**

| file:line | enclosing function | side |
|---|---|---|
| `celt/celt.c:312` | `comb_filter` | BOTH |

`comb_filter` call sites: encoder `celt/celt_encoder.c:1549, 1553, 1576, 2749, 2753`;
decoder `celt/celt_decoder.c:606, 792, 796, 1540, 1544`.

### 5.6 `op_pvq_search` (`op_pvq_search_sse2`) — **ENCODER**

| file:line | enclosing function | side |
|---|---|---|
| `celt/vq.c:603` | `alg_quant` | ENCODER |

(The decoder's `alg_unquant`, `celt/vq.c:619`, uses `decode_pulses` + `normalise_residual` and never
calls the search.)

### 5.7 SILK dispatches — **ENCODER only**

| public function | call site | enclosing function | side |
|---|---|---|---|
| `silk_VAD_GetSA_Q8` | `silk/float/encode_frame_FLP.c:55` | `silk_encode_frame_FLP` | ENCODER |
| `silk_NSQ` | `silk/float/wrappers_FLP.c:167` | `silk_NSQ_wrapper_FLP` | ENCODER |
| `silk_NSQ_del_dec` | `silk/float/wrappers_FLP.c:164` | `silk_NSQ_wrapper_FLP` | ENCODER |
| `silk_VQ_WMat_EC` | `silk/quant_LTP_gains.c:84` | `silk_quant_LTP_gains` | ENCODER |
| `silk_inner_product_FLP` | `silk/float/pitch_analysis_core_FLP.c:294`, `silk/float/autocorrelation_FLP.c:51`, `silk/float/burg_modified_FLP.c:64`, `silk/float/corrMatrix_FLP.c:54, 84` | (FLP analysis) | ENCODER |

No SILK RTCD array is referenced by any decoder object: `nm`/`objdump -r` over the built objects
shows the five `SILK_*_IMPL` symbols are referenced only from
`silk/float/{encode_frame_FLP,wrappers_FLP,autocorrelation_FLP,burg_modified_FLP,corrMatrix_FLP,pitch_analysis_core_FLP}.o`
and `silk/quant_LTP_gains.o`. (`silk/init_decoder.c` calls `opus_select_arch()` but no dispatched
function.)

Extra-library call sites that matter for a full-library bit-exact port:
`dnn/nndsp.o` references `PITCH_XCORR_IMPL` (OSCE, decoder side; `ENABLE_OSCE_BWE` is on),
`dnn/lpcnet_enc.o` references `PITCH_XCORR_IMPL` and `celt_inner_prod_sse`, `src/opus_encoder.o` and
`src/opus_multistream_encoder.o` reference `celt_inner_prod_sse`.

---

## 6. Summary: compatibility with the plain C code

| dispatched function | selected impl at arch 4 | input side(s) | bit-identical to the plain C version? |
|---|---|---|---|
| **CELT** | | | |
| `celt_inner_prod` | `celt_inner_prod_sse` (always) | ENC+DEC | **NO** — 4-lane grouping, horizontal `((s0+s2)+(s1+s3))` vs sequential |
| `dual_inner_prod` | `dual_inner_prod_sse` (always) | ENC | **NO** — same reason |
| `xcorr_kernel` | `xcorr_kernel_sse` (always) | DEC (live) | **NO** — two partial accumulators folded at the end |
| `comb_filter_const` | `comb_filter_const_sse` (always) | ENC+DEC | **NO** — documented `#if 0`; `(a+b)+(c+d)` vs `((a+b)+c)+d`; C version not even compiled |
| `celt_pitch_xcorr` | `celt_pitch_xcorr_avx2` | ENC+DEC | **NO** — FMA + 8-way partial sums + tree reduction |
| `op_pvq_search` | `op_pvq_search_sse2` (always) | ENC | **NO** — approx `rcp`/`rsqrt`, different score, different tie-break, different `sum` reduction |
| **SILK** | | | |
| `silk_VAD_GetSA_Q8` | `silk_VAD_GetSA_Q8_sse4_1` | ENC | **YES** (integer; grouping only) |
| `silk_VQ_WMat_EC` | `silk_VQ_WMat_EC_sse4_1` | ENC | **YES** |
| `silk_NSQ` | `silk_NSQ_sse4_1` | ENC | **YES** (the SIMD 10/16 kernel is unreachable in this build) |
| `silk_NSQ_del_dec` (arch 3) | `silk_NSQ_del_dec_sse4_1` | ENC | **YES** (only the short-term prediction is vectorised, exactly) |
| `silk_NSQ_del_dec` (arch 4) | `silk_NSQ_del_dec_avx2` | ENC | **NO, not provably** — `silk_sar_round_smulww` differs from the C `SMULWW+RSHIFT_ROUND` on int32 overflow ("rare cases", per the source comment); plus the whole kernel is a Q14 rewrite (falls back to `silk_NSQ_del_dec_c` when `nStatesDelayedDecision < 3`) |
| `silk_inner_product_FLP` | `silk_inner_product_FLP_avx2` | ENC | **NO** — FMA + different summation order/reduction tree |

> **Note (what "the reference" means for this table).** The "bit-identical to the plain C version?" column compares a dispatched kernel with the **fallback** body. It is not the contract this crate is held to: the contract is bit-exactness with libopus **as built and configured**, i.e. at `opus_select_arch() == 4` the reference *is* the dispatched implementation. The clearest case is `silk_NSQ_del_dec`: on this host libopus runs `silk_NSQ_del_dec_avx2`, so matching `silk_NSQ_del_dec_c` instead changes 13 of 16 oracle configurations (with identical packet sizes) while every `deldec_cmp.*_c` probe field stays byte-identical — i.e. the C body is a correct *fallback*, not a more exact reference. Measurements and the full argument: `docs/reports/silk-nsq-fast.md`.

---

## 7. Direct answers

### (a) Dispatched functions required for **DECODER** bit-exactness

The decoder never enters a SILK RTCD kernel. The CELT kernels it reaches are:

1. `xcorr_kernel` → `xcorr_kernel_sse` — via `celt_fir_c` (`celt/celt_lpc.c:174`, called from
   `celt_decoder.c:901`) and `celt_iir` (`celt/celt_lpc.c:248`, called from `celt_decoder.c:970`).
2. `celt_inner_prod` → `celt_inner_prod_sse` — via `pitch_search` (`celt/pitch.c:384`, PLC
   `celt_decoder.c:568`).
3. `celt_pitch_xcorr` → `celt_pitch_xcorr_avx2` — via `pitch_search` (`celt/pitch.c:361`) and
   `_celt_autocorr` (`celt/celt_lpc.c:344`, called from `celt_decoder.c:859`); also from
   `dnn/nndsp.c:215,216,308,310` (OSCE, `ENABLE_OSCE_BWE` is defined).
4. `comb_filter_const` → `comb_filter_const_sse` — via `comb_filter` (`celt/celt.c:312`), called from
   `celt_decoder.c:606, 792, 796, 1540, 1544`.
5. `celt_inner_prod` (again) — the `celt_pitch_xcorr_avx2` tail calls `celt_inner_prod_sse`
   (`celt/x86/pitch_avx.c:97`).

**Not needed for the decoder:** `dual_inner_prod`, `op_pvq_search`, and all six SILK kernels.

### (b) Dispatched functions required for **ENCODER** bit-exactness

Everything in §2.5:

* CELT: `celt_inner_prod` (`celt_inner_prod_sse`), `dual_inner_prod` (`dual_inner_prod_sse`),
  `xcorr_kernel` (`xcorr_kernel_sse`, via `celt_fir`/`celt_iir`/`celt_pitch_xcorr_c`),
  `comb_filter_const` (`comb_filter_const_sse`), `celt_pitch_xcorr` (`celt_pitch_xcorr_avx2`),
  `op_pvq_search` (`op_pvq_search_sse2`).
* SILK: `silk_VAD_GetSA_Q8` (`silk_VAD_GetSA_Q8_sse4_1`), `silk_VQ_WMat_EC`
  (`silk_VQ_WMat_EC_sse4_1`), `silk_NSQ` (`silk_NSQ_sse4_1`), `silk_NSQ_del_dec`
  (`silk_NSQ_del_dec_avx2` at arch 4, with the `nStatesDelayedDecision < 3 → silk_NSQ_del_dec_c`
  fallback, and the internal fallback to the scalar `silk_noise_shape_quantizer` for
  `silk_NSQ_sse4_1`), `silk_inner_product_FLP` (`silk_inner_product_FLP_avx2`).

### (c) Which are **NOT** bit-identical to the plain C version

1. **`celt_inner_prod_sse`** — different summation order (4 partial lanes; `(s0+s2)+(s1+s3)`).
2. **`dual_inner_prod_sse`** — same as above, for both outputs.
3. **`xcorr_kernel_sse`** — two partial accumulators combined only at the end.
4. **`comb_filter_const_sse`** — the source's own `#if 0 /* Set to 1 to make it bit-exact with the
   non-SSE version */`; the last two terms are associated as `(a+b)+(c+d)` instead of `((a+b)+c)+d`;
   additionally the last ≤3 samples of every call use the *C* grouping (the `CUSTOM_MODES` tail).
5. **`celt_pitch_xcorr_avx2`** — FMA (`_mm256_fmadd_ps`) and an 8-way partial-sum tree.
6. **`op_pvq_search_sse2`** — approximate `_mm_rcp_ps` (vs `1.f/sum`), approximate `_mm_rsqrt_ps` with
   a different decision rule, different argmax tie-break, and a different reduction order for `sum`.
7. **`silk_NSQ_del_dec_avx2`** — not provably identical: `silk_sar_round_smulww` intentionally omits
   the C int32 truncation (documented in the source as differing "in some rare cases"), and the
   kernel is a Q14 rewrite.
8. **`silk_inner_product_FLP_avx2`** — FMA and a different summation/reduction order vs the
   left-to-right C accumulation.

Identical to the plain C version: `silk_VAD_GetSA_Q8_sse4_1`, `silk_VQ_WMat_EC_sse4_1`,
`silk_NSQ_sse4_1`, `silk_NSQ_del_dec_sse4_1` (plus `silk_NSQ_del_dec_avx2`'s C fallback path).

---

## 8. Implementation notes for the Rust port

1. In this configuration the "arch" argument is a *no-op* for `celt_inner_prod`, `dual_inner_prod`,
   `xcorr_kernel`, `comb_filter_const` and `op_pvq_search`: the SSE/SSE2 implementations are used at
   every arch. Only `celt_pitch_xcorr`, `silk_NSQ_del_dec` and `silk_inner_product_FLP` really
   dispatch on `arch`, and on this host the value is always 4.
2. Use "true" FMA semantics in exactly three places, all marked `_mm*_fmadd_*` in the source:
   `celt/x86/pitch_avx.c:49-70` (`xcorr_kernel_avx`, f32) and
   `silk/float/x86/inner_product_FLP_avx2.c:58,63,72` (f64). Everywhere else the source uses plain
   `_mm_mul_ps` + `_mm_add_ps`, which are *separate* roundings (e.g. `MAC16_16`, `comb_filter_const_sse`'s
   `yi2`, `celt_inner_prod_sse`). In Rust, `f32::mul_add`/`f64::mul_add` give the fused result
   independently of target flags; do **not** let the compiler contract plain `a*b+c` into FMA for the
   non-FMA kernels (`-C fp-contract=off` / avoid `mul_add` there).
3. Keep the two families of accumulation orders distinct:
   * `celt_inner_prod_sse` / `dual_inner_prod_sse`: 4 lanes, then `((s0+s2)+(s1+s3))`, then the
     ≤3-element sequential tail.
   * `xcorr_kernel_sse`: two lane-vectors (`xsum1` even offsets, `xsum2` odd offsets within each
     4-block), then `xsum1+xsum2`; the last `len mod 4` elements follow the tail pattern above.
   * `celt_pitch_xcorr_avx2`: 8 accumulators with FMA, then
     `((s0+s4)+(s1+s5)) + ((s2+s6)+(s3+s7))` per output, and `celt_inner_prod_sse` for the last
     `max_pitch mod 8` lags.
4. SILK integer kernels can be written as exact integer arithmetic; the *only* two things that must be
   replicated literally are the saturating operations (wrap vs saturate differs!) and
   `silk_sar_round_smulww`'s int64 form. All other vector groupings may be collapsed, because int32
   addition is associative modulo 2³².
5. Reproduce the internal fallbacks: `silk_NSQ_del_dec_avx2` must delegate to the C algorithm when
   `nStatesDelayedDecision ∈ {1,2}`; `silk_NSQ_sse4_1` must use the scalar `silk_noise_shape_quantizer`
   unless `shapingLPCOrder == 10 && predictLPCOrder == 16` (unreachable through the public encoder
   controls, since `shapingLPCOrder ∈ {12,14,16,20,24}`).

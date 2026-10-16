#!/usr/bin/env python3
"""Section 7: bit-exactness hazards, with verbatim C."""
import os

REF = "/home/m-kikuchi/projects/oprs/reference/opus-1.6.1"
OUT = "/home/m-kikuchi/projects/oprs/docs/_s7.md"


def grab(p, a, b):
    ls = open(os.path.join(REF, p), errors="replace").read().split("\n")
    return "\n".join(ls[a - 1:b])


def code(p, a, b):
    return "`%s:%d-%d`\n\n```c\n%s\n```\n" % (p, a, b, grab(p, a, b))


SECTIONS = []

# ---------------------------------------------------------------- 7.1
SECTIONS.append((3, "7.1 The two arithmetic domains: `opus_val16` / `opus_val32`", """\
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
""", [
    ("celt/arch.h", 130, 256),
    ("celt/arch.h", 256, 302),
    ("celt/arch.h", 302, 391),
]))

# ---------------------------------------------------------------- 7.2
SECTIONS.append((3, "7.2 The CELT fixed-point macro set — the single most important file to port", """\
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
""", [
    ("celt/fixed_generic.h", 30, 218),
    ("celt/arch.h", 96, 111),
    ("celt/arch.h", 219, 245),
]))

# ---------------------------------------------------------------- 7.3
SECTIONS.append((3, "7.3 The SILK fixed-point macro set", """\
`silk/macros.h` and `silk/Inlines.h` play the same role for SILK. They are
included via `silk/SigProc_FIX.h`:

```c
#include \"typedef.h\"
#include \"resampler_structs.h\"
#include \"macros.h\"
#include \"cpu_support.h\"
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
""", [
    ("silk/macros.h", 30, 154),
    ("silk/Inlines.h", 30, 180),
]))

# ---------------------------------------------------------------- 7.4
SECTIONS.append((3, "7.4 The range coder — where a single off-by-one destroys everything", """\
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
""", [
    ("celt/mfrngcod.h", 30, 48),
    ("celt/ecintrin.h", 30, 91),
    ("celt/entcode.h", 33, 61),
    ("celt/entcode.h", 105, 151),
    ("celt/entcode.c", 30, 112),
    ("celt/entenc.c", 36, 130),
    ("celt/entenc.c", 186, 230),
    ("celt/entenc.c", 255, 305),
    ("celt/entdec.c", 90, 266),
]))

# ---------------------------------------------------------------- 7.5
SECTIONS.append((3, "7.5 Integer division and rounding", """\
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
""", [
    ("silk/SigProc_FIX.h", 445, 530),
    ("silk/NLSF_stabilize.c", 40, 142),
    ("celt/celt_lpc.c", 140, 283),
]))

# ---------------------------------------------------------------- 7.6
SECTIONS.append((3, "7.6 Table-driven and closed-form approximations", """\
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
""", [
    ("celt/mathops.c", 40, 320),
    ("celt/mathops.h", 60, 240),
    ("celt/mathops.h", 240, 360),
    ("silk/Inlines.h", 30, 92),
    ("silk/sigm_Q15.c", 40, 75),
    ("silk/lin2log.c", 30, 45),
    ("silk/log2lin.c", 30, 58),
    ("silk/table_LSF_cos.c", 33, 70),
]))

# ---------------------------------------------------------------- 7.7
SECTIONS.append((3, "7.7 Float rounding: `float` vs `double`, and the float-only encoder paths", """\
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
""", [
    ("celt/float_cast.h", 40, 176),
    ("celt/mathops.h", 150, 240),
    ("celt/arch.h", 258, 300),
]))

# ---------------------------------------------------------------- 7.8
SECTIONS.append((3, "7.8 `kiss_fft`: twiddles, butterfly order and the fixed-point clamp", """\
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
""", [
    ("celt/_kiss_fft_guts.h", 30, 188),
    ("celt/kiss_fft.c", 36, 330),
]))

# ---------------------------------------------------------------- 7.9
SECTIONS.append((3, "7.9 MDCT rotation and windowing", """\
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
""", [
    ("celt/mdct.c", 122, 390),
]))

# ---------------------------------------------------------------- 7.10
SECTIONS.append((3, "7.10 Signed shifts, overflow and other UB to isolate", """\
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
""", [
    ("celt/vq.c", 104, 130),
    ("celt/vq.c", 205, 300),
    ("celt/arch.h", 59, 96),
    ("silk/typedef.h", 52, 80),
]))

# ---------------------------------------------------------------- 7.11
SECTIONS.append((3, "7.11 Conditionals and asserts inside bit-exact paths", """\
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
""", [
    ("celt/arch.h", 59, 96),
    ("celt/celt_decoder.c", 144, 182),
]))

# ---------------------------------------------------------------- 7.99 checklist
SECTIONS.append((3, "7.99 Prioritized hazard checklist (top 10)", """\
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
""", []))


def main():
    o = []
    o.append("## 7. Bit-exactness hazards\n")
    o.append("Golden rule: **target the fixed-point build** (`FIXED_POINT`, and "
             "decide `ENABLE_RES24` explicitly). Every hazard below is stated "
             "against that configuration; the float build is noted only where it "
             "leaks into the fixed build or where the port may need it.\n")
    for lvl, head, prose, grabs in SECTIONS:
        o.append("#" * lvl + " " + head + "\n")
        if prose:
            o.append(prose + "\n")
        for (p, a, b) in grabs:
            o.append(code(p, a, b) + "\n")
    open(OUT, "w").write("\n".join(o))
    print("wrote", OUT, sum(1 for _ in open(OUT)), "lines")


main()

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

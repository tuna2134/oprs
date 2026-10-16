//! Mirror of `silk/SigProc_FIX.h` from libopus 1.6.1, transliterated to safe Rust.
//!
//! The SILK fixed-point primitive macros.  Only the function *declarations* of the header are skipped (Rust does not need them); the function-dispatch macros (`silk_biquad_alt_stride2`, `silk_LPC_inverse_pred_gain`, `silk_burg_modified`, `silk_inner_prod16`) are documented at the end of the file instead, and their `_c` implementations are called directly by the caller.
//!
//! Original C copyright notice (this file is a derived work):
// /***********************************************************************
// Copyright (c) 2006-2011, Skype Limited. All rights reserved.
// Redistribution and use in source and binary forms, with or without
// modification, are permitted provided that the following conditions
// are met:
// - Redistributions of source code must retain the above copyright notice,
// this list of conditions and the following disclaimer.
// - Redistributions in binary form must reproduce the above copyright
// notice, this list of conditions and the following disclaimer in the
// documentation and/or other materials provided with the distribution.
// - Neither the name of Internet Society, IETF or IETF Trust, nor the
// names of specific contributors, may be used to endorse or promote
// products derived from this software without specific prior written
// permission.
// THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS"
// AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
// IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE
// ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT OWNER OR CONTRIBUTORS BE
// LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR
// CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF
// SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS
// INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN
// CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE)
// ARISING IN ANY WAY OUT OF THE USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE
// POSSIBILITY OF SUCH DAMAGE.
// ***********************************************************************/

use crate::silk::macros::*;
use crate::silk::typedef::*;
use crate::types::*;

#[allow(unused_imports)]
pub(crate) use crate::silk::typedef::silk_assert;

/// `SILK_MAX_ORDER_LPC` — max order of the LPC analysis in schur() and k2a().
pub const SILK_MAX_ORDER_LPC: opus_int = 24;

/* `#if (defined(OPUS_X86_MAY_HAVE_SSE4_1))` pulls in `x86/SigProc_FIX_sse.h`,
   which only overrides `silk_burg_modified` and `silk_inner_prod16` (with
   runtime CPU dispatch); those are document here, not transliterated. */

/// `silk_ROR32(a32, rot)` — rotate right, negative `rot` rotates left.
#[inline(always)]
pub fn silk_ROR32(a32: opus_int32, rot: opus_int) -> opus_int32 {
    let x = a32 as opus_uint32;
    let r = rot as opus_uint32;
    let m = rot.wrapping_neg() as opus_uint32;
    if rot == 0 {
        a32
    } else if rot < 0 {
        (x.wrapping_shl(m) | x.wrapping_shr(32u32.wrapping_sub(m))) as opus_int32
    } else {
        (x.wrapping_shl(32u32.wrapping_sub(r)) | x.wrapping_shr(r)) as opus_int32
    }
}

/// `silk_MUL(a32, b32)` — `(a32 * b32)` as a 32-bit int.
#[inline(always)]
pub fn silk_MUL(a32: opus_int32, b32: opus_int32) -> opus_int32 {
    a32.wrapping_mul(b32)
}

/// `silk_MUL_uint(a32, b32)` — `(a32 * b32)` as a 32-bit uint.
#[inline(always)]
pub fn silk_MUL_uint(a32: opus_uint32, b32: opus_uint32) -> opus_uint32 {
    a32.wrapping_mul(b32)
}

/// `silk_MLA(a32, b32, c32)` — `a32 + (b32 * c32)`.
#[inline(always)]
pub fn silk_MLA(a32: opus_int32, b32: opus_int32, c32: opus_int32) -> opus_int32 {
    silk_ADD32(a32, b32.wrapping_mul(c32))
}

/// `silk_MLA_uint(a32, b32, c32)` — `a32 + (b32 * c32)` as a 32-bit uint.
#[inline(always)]
pub fn silk_MLA_uint(a32: opus_uint32, b32: opus_uint32, c32: opus_uint32) -> opus_uint32 {
    a32.wrapping_add(b32.wrapping_mul(c32))
}

/// `silk_SMULTT(a32, b32)` — `((a32 >> 16) * (b32 >> 16))`.
#[inline(always)]
pub fn silk_SMULTT(a32: opus_int32, b32: opus_int32) -> opus_int32 {
    (a32 >> 16).wrapping_mul(b32 >> 16)
}

/// `silk_SMLATT(a32, b32, c32)` — `a32 + ((b32 >> 16) * (c32 >> 16))`.
#[inline(always)]
pub fn silk_SMLATT(a32: opus_int32, b32: opus_int32, c32: opus_int32) -> opus_int32 {
    silk_ADD32(a32, (b32 >> 16).wrapping_mul(c32 >> 16))
}

/// `silk_SMLALBB(a64, b16, c16)` — `a64 + (opus_int64)(b16 * c16)`.
#[inline(always)]
pub fn silk_SMLALBB(a64: opus_int64, b16: opus_int16, c16: opus_int16) -> opus_int64 {
    a64.wrapping_add(((b16 as opus_int32).wrapping_mul(c16 as opus_int32)) as opus_int64)
}

/// `silk_SMULL(a32, b32)` — `(opus_int64)(a32) * (b32)`.
#[inline(always)]
pub fn silk_SMULL(a32: opus_int32, b32: opus_int32) -> opus_int64 {
    (a32 as opus_int64).wrapping_mul(b32 as opus_int64)
}

/// `silk_ADD32_ovflw(a, b)` — wrapping 32-bit addition.
#[inline(always)]
pub fn silk_ADD32_ovflw(a: opus_int32, b: opus_int32) -> opus_int32 {
    a.wrapping_add(b)
}

/// `silk_SUB32_ovflw(a, b)` — wrapping 32-bit subtraction.
#[inline(always)]
pub fn silk_SUB32_ovflw(a: opus_int32, b: opus_int32) -> opus_int32 {
    a.wrapping_sub(b)
}

/// `silk_MLA_ovflw(a32, b32, c32)` — `a32 + (b32 * c32)` with wrapping add.
#[inline(always)]
pub fn silk_MLA_ovflw(a32: opus_int32, b32: opus_int32, c32: opus_int32) -> opus_int32 {
    silk_ADD32_ovflw(a32, b32.wrapping_mul(c32))
}

/// `silk_SMLABB_ovflw(a32, b32, c32)`
#[inline(always)]
pub fn silk_SMLABB_ovflw(a32: opus_int32, b32: opus_int32, c32: opus_int32) -> opus_int32 {
    silk_ADD32_ovflw(a32, ((b32 as opus_int16) as opus_int32).wrapping_mul((c32 as opus_int16) as opus_int32))
}

/// `silk_DIV32_16(a32, b16)` — truncating division (C99 semantics, which Rust
/// matches for signed integers).
#[inline(always)]
pub fn silk_DIV32_16(a32: opus_int32, b16: opus_int32) -> opus_int32 {
    debug_assert!(b16 != 0);
    a32 / b16
}

/// `silk_DIV32(a32, b32)`
#[inline(always)]
pub fn silk_DIV32(a32: opus_int32, b32: opus_int32) -> opus_int32 {
    debug_assert!(b32 != 0);
    a32 / b32
}

/// `silk_ADD16(a, b)` — the operands are promoted to `int`, so the C result is
/// an `int` too; callers that want 16 bits rely on the implicit narrowing on
/// assignment and must write `as opus_int16`.
#[inline(always)]
pub fn silk_ADD16(a: opus_int16, b: opus_int16) -> opus_int32 {
    (a as opus_int32).wrapping_add(b as opus_int32)
}

/// `silk_ADD32(a, b)`
#[inline(always)]
pub fn silk_ADD32(a: opus_int32, b: opus_int32) -> opus_int32 {
    a.wrapping_add(b)
}

/// `silk_ADD64(a, b)`
#[inline(always)]
pub fn silk_ADD64(a: opus_int64, b: opus_int64) -> opus_int64 {
    a.wrapping_add(b)
}

/// `silk_SUB16(a, b)` — promoted to `int` like `silk_ADD16`.
#[inline(always)]
pub fn silk_SUB16(a: opus_int16, b: opus_int16) -> opus_int32 {
    (a as opus_int32).wrapping_sub(b as opus_int32)
}

/// `silk_SUB32(a, b)`
#[inline(always)]
pub fn silk_SUB32(a: opus_int32, b: opus_int32) -> opus_int32 {
    a.wrapping_sub(b)
}

/// `silk_SUB64(a, b)`
#[inline(always)]
pub fn silk_SUB64(a: opus_int64, b: opus_int64) -> opus_int64 {
    a.wrapping_sub(b)
}

/// `silk_SAT8(a)` — the C macro returns `int`, callers cast.
#[inline(always)]
pub fn silk_SAT8(a: opus_int32) -> opus_int32 {
    if a > silk_int8_MAX as opus_int32 {
        silk_int8_MAX as opus_int32
    } else if a < silk_int8_MIN as opus_int32 {
        silk_int8_MIN as opus_int32
    } else {
        a
    }
}

/// `silk_SAT16(a)` — the C macro returns `int`, callers cast.
#[inline(always)]
pub fn silk_SAT16(a: opus_int32) -> opus_int32 {
    if a > silk_int16_MAX as opus_int32 {
        silk_int16_MAX as opus_int32
    } else if a < silk_int16_MIN as opus_int32 {
        silk_int16_MIN as opus_int32
    } else {
        a
    }
}

/// `silk_SAT32(a)`
#[inline(always)]
pub fn silk_SAT32(a: opus_int32) -> opus_int32 {
    if a > silk_int32_MAX {
        silk_int32_MAX
    } else if a < silk_int32_MIN {
        silk_int32_MIN
    } else {
        a
    }
}

/// `silk_CHECK_FIT8(a)` — identity unless `silk_API_Debug.h` is used.
#[inline(always)]
pub fn silk_CHECK_FIT8(a: opus_int32) -> opus_int32 {
    a
}

/// `silk_CHECK_FIT16(a)`
#[inline(always)]
pub fn silk_CHECK_FIT16(a: opus_int32) -> opus_int32 {
    a
}

/// `silk_CHECK_FIT32(a)`
#[inline(always)]
pub fn silk_CHECK_FIT32(a: opus_int32) -> opus_int32 {
    a
}

/// `silk_ADD_SAT16(a, b)`
#[inline(always)]
pub fn silk_ADD_SAT16(a: opus_int16, b: opus_int16) -> opus_int16 {
    silk_SAT16(silk_ADD32(a as opus_int32, b as opus_int32)) as opus_int16
}

/// `silk_ADD_SAT64(a, b)` — the C sign test is an overflow test, i.e. a
/// saturating addition.
#[inline(always)]
pub fn silk_ADD_SAT64(a: opus_int64, b: opus_int64) -> opus_int64 {
    a.saturating_add(b)
}

/// `silk_SUB_SAT16(a, b)`
#[inline(always)]
pub fn silk_SUB_SAT16(a: opus_int16, b: opus_int16) -> opus_int16 {
    silk_SAT16(silk_SUB32(a as opus_int32, b as opus_int32)) as opus_int16
}

/// `silk_SUB_SAT64(a, b)`
#[inline(always)]
pub fn silk_SUB_SAT64(a: opus_int64, b: opus_int64) -> opus_int64 {
    a.saturating_sub(b)
}

/// `silk_POS_SAT32(a)` — saturation for positive input values.
#[inline(always)]
pub fn silk_POS_SAT32(a: opus_int32) -> opus_int32 {
    if a > silk_int32_MAX {
        silk_int32_MAX
    } else {
        a
    }
}

/// `silk_ADD_POS_SAT8(a, b)`
#[inline(always)]
pub fn silk_ADD_POS_SAT8(a: opus_int32, b: opus_int32) -> opus_int32 {
    let sum = a.wrapping_add(b);
    if sum & 0x80 != 0 {
        silk_int8_MAX as opus_int32
    } else {
        sum
    }
}

/// `silk_ADD_POS_SAT16(a, b)`
#[inline(always)]
pub fn silk_ADD_POS_SAT16(a: opus_int32, b: opus_int32) -> opus_int32 {
    let sum = a.wrapping_add(b);
    if sum & 0x8000 != 0 {
        silk_int16_MAX as opus_int32
    } else {
        sum
    }
}

/// `silk_ADD_POS_SAT32(a, b)`
#[inline(always)]
pub fn silk_ADD_POS_SAT32(a: opus_int32, b: opus_int32) -> opus_int32 {
    if (a as opus_uint32).wrapping_add(b as opus_uint32) & 0x8000_0000 != 0 {
        silk_int32_MAX
    } else {
        a.wrapping_add(b)
    }
}

/// `silk_LSHIFT8(a, shift)` — shift in `0..8`.
#[inline(always)]
pub fn silk_LSHIFT8(a: opus_int8, shift: opus_int32) -> opus_int8 {
    ((a as opus_uint8).wrapping_shl(shift as u32)) as opus_int8
}

/// `silk_LSHIFT16(a, shift)` — shift in `0..16`.
#[inline(always)]
pub fn silk_LSHIFT16(a: opus_int16, shift: opus_int32) -> opus_int16 {
    ((a as opus_uint16).wrapping_shl(shift as u32)) as opus_int16
}

/// `silk_LSHIFT32(a, shift)` — shift in `0..32`.
#[inline(always)]
pub fn silk_LSHIFT32(a: opus_int32, shift: opus_int32) -> opus_int32 {
    ((a as opus_uint32).wrapping_shl(shift as u32)) as opus_int32
}

/// `silk_LSHIFT64(a, shift)` — shift in `0..64`.
#[inline(always)]
pub fn silk_LSHIFT64(a: opus_int64, shift: opus_int32) -> opus_int64 {
    ((a as opus_uint64).wrapping_shl(shift as u32)) as opus_int64
}

/// `silk_LSHIFT(a, shift)` — shift in `0..32`.
#[inline(always)]
pub fn silk_LSHIFT(a: opus_int32, shift: opus_int32) -> opus_int32 {
    silk_LSHIFT32(a, shift)
}

/// `silk_RSHIFT8(a, shift)` — C integer promotion makes the result an `int`.
#[inline(always)]
pub fn silk_RSHIFT8(a: opus_int8, shift: opus_int32) -> opus_int32 {
    (a as opus_int32) >> shift
}

/// `silk_RSHIFT16(a, shift)` — C integer promotion makes the result an `int`.
#[inline(always)]
pub fn silk_RSHIFT16(a: opus_int16, shift: opus_int32) -> opus_int32 {
    (a as opus_int32) >> shift
}

/// `silk_RSHIFT32(a, shift)` — arithmetic shift.
#[inline(always)]
pub fn silk_RSHIFT32(a: opus_int32, shift: opus_int32) -> opus_int32 {
    a >> shift
}

/// `silk_RSHIFT64(a, shift)` — arithmetic shift.
#[inline(always)]
pub fn silk_RSHIFT64(a: opus_int64, shift: opus_int32) -> opus_int64 {
    a >> shift
}

/// `silk_RSHIFT(a, shift)` — arithmetic shift.
#[inline(always)]
pub fn silk_RSHIFT(a: opus_int32, shift: opus_int32) -> opus_int32 {
    silk_RSHIFT32(a, shift)
}

/// `silk_LSHIFT_SAT32(a, shift)` — saturates before shifting.
#[inline(always)]
pub fn silk_LSHIFT_SAT32(a: opus_int32, shift: opus_int32) -> opus_int32 {
    silk_LSHIFT32(
        silk_LIMIT(a, silk_RSHIFT32(silk_int32_MIN, shift), silk_RSHIFT32(silk_int32_MAX, shift)),
        shift,
    )
}

/// `silk_LSHIFT_ovflw(a, shift)` — shift `>= 0`, allowed to overflow.
#[inline(always)]
pub fn silk_LSHIFT_ovflw(a: opus_int32, shift: opus_int32) -> opus_int32 {
    ((a as opus_uint32).wrapping_shl(shift as u32)) as opus_int32
}

/// `silk_LSHIFT_uint(a, shift)` — shift `>= 0`.
#[inline(always)]
pub fn silk_LSHIFT_uint(a: opus_uint32, shift: opus_int32) -> opus_uint32 {
    a.wrapping_shl(shift as u32)
}

/// `silk_RSHIFT_uint(a, shift)` — logical shift, `shift >= 0`.
#[inline(always)]
pub fn silk_RSHIFT_uint(a: opus_uint32, shift: opus_int32) -> opus_uint32 {
    a.wrapping_shr(shift as u32)
}

/// `silk_ADD_LSHIFT(a, b, shift)`
#[inline(always)]
pub fn silk_ADD_LSHIFT(a: opus_int32, b: opus_int32, shift: opus_int32) -> opus_int32 {
    a.wrapping_add(silk_LSHIFT(b, shift))
}

/// `silk_ADD_LSHIFT32(a, b, shift)`
#[inline(always)]
pub fn silk_ADD_LSHIFT32(a: opus_int32, b: opus_int32, shift: opus_int32) -> opus_int32 {
    silk_ADD32(a, silk_LSHIFT32(b, shift))
}

/// `silk_ADD_LSHIFT_uint(a, b, shift)`
#[inline(always)]
pub fn silk_ADD_LSHIFT_uint(a: opus_uint32, b: opus_uint32, shift: opus_int32) -> opus_uint32 {
    a.wrapping_add(silk_LSHIFT_uint(b, shift))
}

/// `silk_ADD_RSHIFT(a, b, shift)`
#[inline(always)]
pub fn silk_ADD_RSHIFT(a: opus_int32, b: opus_int32, shift: opus_int32) -> opus_int32 {
    a.wrapping_add(silk_RSHIFT(b, shift))
}

/// `silk_ADD_RSHIFT32(a, b, shift)`
#[inline(always)]
pub fn silk_ADD_RSHIFT32(a: opus_int32, b: opus_int32, shift: opus_int32) -> opus_int32 {
    silk_ADD32(a, silk_RSHIFT32(b, shift))
}

/// `silk_ADD_RSHIFT_uint(a, b, shift)`
#[inline(always)]
pub fn silk_ADD_RSHIFT_uint(a: opus_uint32, b: opus_uint32, shift: opus_int32) -> opus_uint32 {
    a.wrapping_add(silk_RSHIFT_uint(b, shift))
}

/// `silk_SUB_LSHIFT32(a, b, shift)`
#[inline(always)]
pub fn silk_SUB_LSHIFT32(a: opus_int32, b: opus_int32, shift: opus_int32) -> opus_int32 {
    silk_SUB32(a, silk_LSHIFT32(b, shift))
}

/// `silk_SUB_RSHIFT32(a, b, shift)`
#[inline(always)]
pub fn silk_SUB_RSHIFT32(a: opus_int32, b: opus_int32, shift: opus_int32) -> opus_int32 {
    silk_SUB32(a, silk_RSHIFT32(b, shift))
}

/// `silk_RSHIFT_ROUND(a, shift)` — requires `shift > 0`.  The C macro is
/// type-generic; 64-bit call sites use [`silk_RSHIFT_ROUND64`].
#[inline(always)]
pub fn silk_RSHIFT_ROUND(a: opus_int32, shift: opus_int32) -> opus_int32 {
    if shift == 1 {
        (a >> 1) + (a & 1)
    } else {
        ((a >> (shift - 1)).wrapping_add(1)) >> 1
    }
}

/// `silk_RSHIFT_ROUND64(a, shift)` — requires `shift > 0`.
#[inline(always)]
pub fn silk_RSHIFT_ROUND64(a: opus_int64, shift: opus_int32) -> opus_int64 {
    if shift == 1 {
        (a >> 1) + (a & 1)
    } else {
        ((a >> (shift - 1)).wrapping_add(1)) >> 1
    }
}

/// `silk_NSHIFT_MUL_32_32(a, b)` — number of right shifts needed to fit the
/// multiplication.  `silk_abs` is kept bit-exactly, including its documented
/// misbehaviour for `silk_int32_MIN`.
#[inline(always)]
pub fn silk_NSHIFT_MUL_32_32(a: opus_int32, b: opus_int32) -> opus_int32 {
    -(31 - ((32 - silk_CLZ32(silk_abs(a))) + (32 - silk_CLZ32(silk_abs(b)))))
}

/// `silk_NSHIFT_MUL_16_16(a, b)`
#[inline(always)]
pub fn silk_NSHIFT_MUL_16_16(a: opus_int16, b: opus_int16) -> opus_int32 {
    -(15 - ((16 - silk_CLZ16(silk_abs(a as opus_int32) as opus_int16))
        + (16 - silk_CLZ16(silk_abs(b as opus_int32) as opus_int16))))
}

/// `silk_min(a, b)`
#[inline(always)]
pub fn silk_min<T: PartialOrd>(a: T, b: T) -> T {
    if a < b {
        a
    } else {
        b
    }
}

/// `silk_max(a, b)`
#[inline(always)]
pub fn silk_max<T: PartialOrd>(a: T, b: T) -> T {
    if a > b {
        a
    } else {
        b
    }
}

/// `SILK_FIX_CONST(C, Q)` — `(opus_int32)((C) * ((opus_int64)1 << (Q)) + 0.5)`.
///
/// The C macro always evaluates the product in `double` (the `1 << Q` is
/// promoted); Rust has no implicit float promotion, so the argument is `f64`
/// and `f32` call sites write `x as f64` exactly like the C conversion.
#[inline(always)]
pub const fn SILK_FIX_CONST(C: f64, Q: opus_int32) -> opus_int32 {
    ((C * ((1i64 << Q) as f64)) + 0.5) as opus_int32
}

/// `silk_min_int(a, b)`
#[inline(always)]
pub fn silk_min_int(a: opus_int, b: opus_int) -> opus_int {
    if a < b { a } else { b }
}

/// `silk_min_16(a, b)`
#[inline(always)]
pub fn silk_min_16(a: opus_int16, b: opus_int16) -> opus_int16 {
    if a < b { a } else { b }
}

/// `silk_min_32(a, b)`
#[inline(always)]
pub fn silk_min_32(a: opus_int32, b: opus_int32) -> opus_int32 {
    if a < b { a } else { b }
}

/// `silk_min_64(a, b)`
#[inline(always)]
pub fn silk_min_64(a: opus_int64, b: opus_int64) -> opus_int64 {
    if a < b { a } else { b }
}

/// `silk_max_int(a, b)`
#[inline(always)]
pub fn silk_max_int(a: opus_int, b: opus_int) -> opus_int {
    if a > b { a } else { b }
}

/// `silk_max_16(a, b)`
#[inline(always)]
pub fn silk_max_16(a: opus_int16, b: opus_int16) -> opus_int16 {
    if a > b { a } else { b }
}

/// `silk_max_32(a, b)`
#[inline(always)]
pub fn silk_max_32(a: opus_int32, b: opus_int32) -> opus_int32 {
    if a > b { a } else { b }
}

/// `silk_max_64(a, b)`
#[inline(always)]
pub fn silk_max_64(a: opus_int64, b: opus_int64) -> opus_int64 {
    if a > b { a } else { b }
}

/// `silk_LIMIT(a, limit1, limit2)`
#[inline(always)]
pub fn silk_LIMIT<T: PartialOrd>(a: T, limit1: T, limit2: T) -> T {
    if limit1 > limit2 {
        if a > limit1 {
            limit1
        } else if a < limit2 {
            limit2
        } else {
            a
        }
    } else if a > limit2 {
        limit2
    } else if a < limit1 {
        limit1
    } else {
        a
    }
}

// `silk_LIMIT_int`, `silk_LIMIT_16` and `silk_LIMIT_32` are aliases of
// `silk_LIMIT`; the generic Rust function covers all three.

/// `silk_abs(a)` — careful, wrong when the input equals `silk_int32_MIN`
/// (the C `-(a)` is UB there; x86 wraps, which `wrapping_neg` reproduces).
#[inline(always)]
pub fn silk_abs(a: opus_int32) -> opus_int32 {
    if a > 0 {
        a
    } else {
        a.wrapping_neg()
    }
}

/// `silk_abs_int(a)` — `((a) ^ ((a) >> 31)) - ((a) >> 31)`.
#[inline(always)]
pub fn silk_abs_int(a: opus_int32) -> opus_int32 {
    (a ^ (a >> 31)).wrapping_sub(a >> 31)
}

/// `silk_abs_int32(a)`
#[inline(always)]
pub fn silk_abs_int32(a: opus_int32) -> opus_int32 {
    (a ^ (a >> 31)).wrapping_sub(a >> 31)
}

/// `silk_abs_int64(a)`
#[inline(always)]
pub fn silk_abs_int64(a: opus_int64) -> opus_int64 {
    if a > 0 {
        a
    } else {
        a.wrapping_neg()
    }
}

/// `silk_sign(a)`
#[inline(always)]
pub fn silk_sign(a: opus_int32) -> opus_int32 {
    if a > 0 {
        1
    } else if a < 0 {
        -1
    } else {
        0
    }
}

/// `RAND_MULTIPLIER`
pub const RAND_MULTIPLIER: opus_int32 = 196314165;
/// `RAND_INCREMENT`
pub const RAND_INCREMENT: opus_int32 = 907633515;

/// `silk_RAND(seed)` — `silk_MLA_ovflw(RAND_INCREMENT, seed, RAND_MULTIPLIER)`.
#[inline(always)]
pub fn silk_RAND(seed: opus_int32) -> opus_int32 {
    silk_MLA_ovflw(RAND_INCREMENT, seed, RAND_MULTIPLIER)
}

/// `silk_SMMUL(a32, b32)` — signed top word multiply:
/// `(opus_int32)silk_RSHIFT64(silk_SMULL(a32, b32), 32)`.
#[inline(always)]
pub fn silk_SMMUL(a32: opus_int32, b32: opus_int32) -> opus_int32 {
    (silk_RSHIFT64(silk_SMULL(a32, b32), 32)) as opus_int32
}

/* Function-dispatch macros that the x86 build resolves at run time.  In the
   Rust port the caller invokes the plain-C implementation directly:
     `silk_biquad_alt_stride2(in, B_Q28, A_Q28, S, out, len, arch)`
        -> `silk_biquad_alt_stride2_c(...)`
     `silk_LPC_inverse_pred_gain(A_Q12, order, arch)`
        -> `silk_LPC_inverse_pred_gain_c(...)`
     `silk_burg_modified(..., arch)` -> `silk_burg_modified_c(...)`
     `silk_inner_prod16(v1, v2, len, arch)` -> `silk_inner_prod16_c(...)`
   `silk_inner_prod_aligned`, `silk_inner_prod_aligned_scale`,
   `silk_scale_copy_vector16`, `silk_scale_vector32_Q26_lshift_18` and
   `silk_RSHIFT_ROUND` on 64-bit operands are declared in this header but
   defined in the corresponding `.c` files. */

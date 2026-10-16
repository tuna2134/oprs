//! Mirror of `silk/Inlines.h` from libopus 1.6.1, transliterated to safe Rust.
//!
//! `OPUS_INLINE` signal-processing helpers.  The two out-parameter helpers (`silk_CLZ_FRAC`) return a tuple instead of writing through pointers.
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
use crate::silk::sigproc_fix::*;
use crate::silk::typedef::*;
use crate::types::*;

/// `silk_CLZ64(in)` — count leading zeros of an `opus_int64`.
#[inline(always)]
pub fn silk_CLZ64(input: opus_int64) -> opus_int32 {
    let in_upper = silk_RSHIFT64(input, 32) as opus_int32;
    if in_upper == 0 {
        /* Search in the lower 32 bits */
        32 + silk_CLZ32(input as opus_int32)
    } else {
        /* Search in the upper 32 bits */
        silk_CLZ32(in_upper)
    }
}

/// `silk_CLZ_FRAC(in, &lz, &frac_Q7)` — returns `(lz, frac_Q7)`; the C writes
/// the two results through pointers.
#[inline(always)]
pub fn silk_CLZ_FRAC(input: opus_int32) -> (opus_int32, opus_int32) {
    let lzeros = silk_CLZ32(input);
    let frac_Q7 = silk_ROR32(input, 24 - lzeros) & 0x7f;
    (lzeros, frac_Q7)
}

/// `silk_SQRT_APPROX(x)` — approximation of the square root.
#[inline(always)]
pub fn silk_SQRT_APPROX(x: opus_int32) -> opus_int32 {
    if x <= 0 {
        return 0;
    }

    let (lz, frac_Q7) = silk_CLZ_FRAC(x);

    let mut y = if lz & 1 != 0 {
        32768
    } else {
        46214 // 46214 = sqrt(2) * 32768
    };

    /* get scaling right */
    y >>= silk_RSHIFT(lz, 1);

    /* increment using fractional part of input */
    y = silk_SMLAWB(y, y, silk_SMULBB(213, frac_Q7));

    y
}

/// `silk_DIV32_varQ(a32, b32, Qres)` — a good approximation of
/// `(a32 << Qres) / b32`.
#[inline(always)]
pub fn silk_DIV32_varQ(a32: opus_int32, b32: opus_int32, Qres: opus_int) -> opus_int32 {
    silk_assert!(b32 != 0);
    silk_assert!(Qres >= 0);

    /* Compute number of bits head room and normalize inputs */
    let a_headrm = silk_CLZ32(silk_abs(a32)) - 1;
    let mut a32_nrm = silk_LSHIFT(a32, a_headrm); /* Q: a_headrm                  */
    let b_headrm = silk_CLZ32(silk_abs(b32)) - 1;
    let b32_nrm = silk_LSHIFT(b32, b_headrm); /* Q: b_headrm                  */

    /* Inverse of b32, with 14 bits of precision */
    let b32_inv = silk_DIV32_16(silk_int32_MAX >> 2, silk_RSHIFT(b32_nrm, 16)); /* Q: 29 + 16 - b_headrm        */

    /* First approximation */
    let mut result = silk_SMULWB(a32_nrm, b32_inv); /* Q: 29 + a_headrm - b_headrm  */

    /* Compute residual by subtracting product of denominator and first approximation */
    /* It's OK to overflow because the final value of a32_nrm should always be small */
    a32_nrm = silk_SUB32_ovflw(a32_nrm, silk_LSHIFT_ovflw(silk_SMMUL(b32_nrm, result), 3)); /* Q: a_headrm  */

    /* Refinement */
    result = silk_SMLAWB(result, a32_nrm, b32_inv); /* Q: 29 + a_headrm - b_headrm  */

    /* Convert to Qres domain */
    let lshift = 29 + a_headrm - b_headrm - Qres;
    if lshift < 0 {
        silk_LSHIFT_SAT32(result, -lshift)
    } else if lshift < 32 {
        silk_RSHIFT(result, lshift)
    } else {
        /* Avoid undefined result */
        0
    }
}

/// `silk_INVERSE32_varQ(b32, Qres)` — a good approximation of
/// `(1 << Qres) / b32`.
#[inline(always)]
pub fn silk_INVERSE32_varQ(b32: opus_int32, Qres: opus_int) -> opus_int32 {
    silk_assert!(b32 != 0);
    silk_assert!(Qres > 0);

    /* Compute number of bits head room and normalize input */
    let b_headrm = silk_CLZ32(silk_abs(b32)) - 1;
    let b32_nrm = silk_LSHIFT(b32, b_headrm); /* Q: b_headrm                */

    /* Inverse of b32, with 14 bits of precision */
    let b32_inv = silk_DIV32_16(silk_int32_MAX >> 2, silk_RSHIFT(b32_nrm, 16)); /* Q: 29 + 16 - b_headrm    */

    /* First approximation */
    let mut result = silk_LSHIFT(b32_inv, 16); /* Q: 61 - b_headrm            */

    /* Compute residual by subtracting product of denominator and first approximation from one */
    let err_Q32 = silk_LSHIFT(((1 as opus_int32) << 29) - silk_SMULWB(b32_nrm, b32_inv), 3); /* Q32                        */

    /* Refinement */
    result = silk_SMLAWW(result, err_Q32, b32_inv); /* Q: 61 - b_headrm            */

    /* Convert to Qres domain */
    let lshift = 61 - b_headrm - Qres;
    if lshift <= 0 {
        silk_LSHIFT_SAT32(result, -lshift)
    } else if lshift < 32 {
        silk_RSHIFT(result, lshift)
    } else {
        /* Avoid undefined result */
        0
    }
}

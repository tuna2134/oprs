//! Mirror of `silk/LPC_inv_pred_gain.c` from libopus 1.6.1, transliterated to safe Rust.
//!
//! Compute the inverse of the LPC prediction gain, and test whether the LPC
//! coefficients are stable (all poles within the unit circle).  The file defines
//! the `_c` implementation that `silk_LPC_inverse_pred_gain` dispatches to in
//! the reference (non-SIMD-overridden) configuration.
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

use crate::silk::define::*;
use crate::silk::inlines::*;
use crate::silk::macros::*;
use crate::silk::sigproc_fix::*;
use crate::silk::typedef::*;
use crate::types::*;

const QA: opus_int = 24;
const A_LIMIT: opus_int32 = SILK_FIX_CONST(0.99975, QA);

/// `SILK_FIX_CONST( 1.0f / MAX_PREDICTION_POWER_GAIN, 30 )` — the C evaluates
/// `float * (opus_int64)` in `float`, so the product is computed in `f32` here.
const MIN_INV_PRED_GAIN_Q30: opus_int32 =
    (((1.0f32 / MAX_PREDICTION_POWER_GAIN) * 1073741824.0f32) as f64 + 0.5) as opus_int32;

/// `MUL32_FRAC_Q(a32, b32, Q)`
#[inline(always)]
fn MUL32_FRAC_Q(a32: opus_int32, b32: opus_int32, Q: opus_int) -> opus_int32 {
    silk_RSHIFT_ROUND64(silk_SMULL(a32, b32), Q) as opus_int32
}

/* Compute inverse of LPC prediction gain, and                          */
/* test if LPC coefficients are stable (all poles within unit circle)   */
fn LPC_inverse_pred_gain_QA_c(
    /* O   Returns inverse prediction gain in energy domain, Q30 */
    A_QA: &mut [opus_int32],  /* I   Prediction coefficients */
    order: opus_int,          /* I   Prediction order */
) -> opus_int32 {
    let mut k: opus_int;
    let mut mult2Q: opus_int;
    let mut invGain_Q30: opus_int32;
    let mut rc_Q31: opus_int32;
    let mut rc_mult1_Q30: opus_int32;
    let mut rc_mult2: opus_int32;
    let mut tmp1: opus_int32;
    let mut tmp2: opus_int32;
    let mut tmp64: opus_int64;

    invGain_Q30 = SILK_FIX_CONST(1.0, 30);
    k = order - 1;
    while k > 0 {
        /* Check for stability */
        if (A_QA[k as usize] > A_LIMIT) || (A_QA[k as usize] < -A_LIMIT) {
            return 0;
        }

        /* Set RC equal to negated AR coef */
        rc_Q31 = -silk_LSHIFT(A_QA[k as usize], 31 - QA);

        /* rc_mult1_Q30 range: [ 1 : 2^30 ] */
        rc_mult1_Q30 = silk_SUB32(SILK_FIX_CONST(1.0, 30), silk_SMMUL(rc_Q31, rc_Q31));
        silk_assert!(rc_mult1_Q30 > (1 << 15)); /* reduce A_LIMIT if fails */
        silk_assert!(rc_mult1_Q30 <= (1 << 30));

        /* Update inverse gain */
        /* invGain_Q30 range: [ 0 : 2^30 ] */
        invGain_Q30 = silk_LSHIFT(silk_SMMUL(invGain_Q30, rc_mult1_Q30), 2);
        silk_assert!(invGain_Q30 >= 0);
        silk_assert!(invGain_Q30 <= (1 << 30));
        if invGain_Q30 < MIN_INV_PRED_GAIN_Q30 {
            return 0;
        }

        /* rc_mult2 range: [ 2^30 : silk_int32_MAX ] */
        mult2Q = 32 - silk_CLZ32(silk_abs(rc_mult1_Q30));
        rc_mult2 = silk_INVERSE32_varQ(rc_mult1_Q30, mult2Q + 30);

        /* Update AR coefficient */
        for n in 0..((k + 1) >> 1) {
            tmp1 = A_QA[n as usize];
            tmp2 = A_QA[(k - n - 1) as usize];
            tmp64 = silk_RSHIFT_ROUND64(
                silk_SMULL(silk_SUB_SAT32(tmp1, MUL32_FRAC_Q(tmp2, rc_Q31, 31)), rc_mult2),
                mult2Q,
            );
            if tmp64 > silk_int32_MAX as opus_int64 || tmp64 < silk_int32_MIN as opus_int64 {
                return 0;
            }
            A_QA[n as usize] = tmp64 as opus_int32;
            tmp64 = silk_RSHIFT_ROUND64(
                silk_SMULL(silk_SUB_SAT32(tmp2, MUL32_FRAC_Q(tmp1, rc_Q31, 31)), rc_mult2),
                mult2Q,
            );
            if tmp64 > silk_int32_MAX as opus_int64 || tmp64 < silk_int32_MIN as opus_int64 {
                return 0;
            }
            A_QA[(k - n - 1) as usize] = tmp64 as opus_int32;
        }
        k -= 1;
    }

    /* Check for stability */
    if (A_QA[k as usize] > A_LIMIT) || (A_QA[k as usize] < -A_LIMIT) {
        return 0;
    }

    /* Set RC equal to negated AR coef */
    rc_Q31 = -silk_LSHIFT(A_QA[0], 31 - QA);

    /* Range: [ 1 : 2^30 ] */
    rc_mult1_Q30 = silk_SUB32(SILK_FIX_CONST(1.0, 30), silk_SMMUL(rc_Q31, rc_Q31));

    /* Update inverse gain */
    /* Range: [ 0 : 2^30 ] */
    invGain_Q30 = silk_LSHIFT(silk_SMMUL(invGain_Q30, rc_mult1_Q30), 2);
    silk_assert!(invGain_Q30 >= 0);
    silk_assert!(invGain_Q30 <= (1 << 30));
    if invGain_Q30 < MIN_INV_PRED_GAIN_Q30 {
        return 0;
    }

    invGain_Q30
}

/// `silk_LPC_inverse_pred_gain_c(A_Q12, order)` — for input in the Q12 domain.
pub fn silk_LPC_inverse_pred_gain_c(
    /* O   Returns inverse prediction gain in energy domain, Q30 */
    A_Q12: &[opus_int16],  /* I   Prediction coefficients, Q12 [order] */
    order: opus_int,       /* I   Prediction order */
) -> opus_int32 {
    let mut Atmp_QA = [0 as opus_int32; SILK_MAX_ORDER_LPC as usize];
    let mut DC_resp: opus_int32 = 0;

    /* Increase Q domain of the AR coefficients */
    for k in 0..order {
        DC_resp = DC_resp.wrapping_add(A_Q12[k as usize] as opus_int32);
        Atmp_QA[k as usize] = silk_LSHIFT32(A_Q12[k as usize] as opus_int32, QA - 12);
    }
    /* If the DC is unstable, we don't even need to do the full calculations */
    if DC_resp >= 4096 {
        return 0;
    }
    LPC_inverse_pred_gain_QA_c(&mut Atmp_QA, order)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_filter_has_unit_gain() {
        let a = [0i16; 16];
        assert_eq!(silk_LPC_inverse_pred_gain_c(&a, 16), 1 << 30);
    }

    #[test]
    fn large_dc_is_unstable() {
        let a = [32767i16; 10];
        assert_eq!(silk_LPC_inverse_pred_gain_c(&a, 10), 0);
    }

    #[test]
    fn prediction_gain_is_bounded() {
        let a: [i16; 16] = [100, -50, 20, -10, 5, -2, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        let g = silk_LPC_inverse_pred_gain_c(&a, 16);
        assert!(g >= 0 && g <= (1 << 30));
    }
}

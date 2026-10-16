//! Mirror of `silk/NLSF2A.c` from libopus 1.6.1, transliterated to safe Rust.
//!
//! Conversion between prediction filter coefficients and LSFs.  Order should be
//! even.  A piecewise linear approximation maps LSF <-> cos(LSF), therefore the
//! result is not accurate LSFs, but the two functions are accurate inverses of
//! each other.
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

use crate::celt::arch::celt_assert;
use crate::silk::bwexpander_32::*;
use crate::silk::define::*;
use crate::silk::lpc_fit::*;
use crate::silk::lpc_inv_pred_gain::*;
use crate::silk::sigproc_fix::*;
use crate::silk::table_lsf_cos::*;
use crate::silk::typedef::*;
use crate::types::*;

const QA: opus_int = 16;

/* helper function for NLSF2A(..) */
fn silk_NLSF2A_find_poly(
    out: &mut [opus_int32],      /* O    intermediate polynomial, QA [dd+1] */
    cLSF: &[opus_int32],         /* I    vector of interleaved 2*cos(LSFs), QA [d] */
    dd: opus_int,                /* I    polynomial order (= 1/2 * filter order) */
) {
    let mut n: opus_int;
    let mut ftmp: opus_int32;

    out[0] = silk_LSHIFT(1, QA);
    out[1] = -cLSF[0];
    for k in 1..dd {
        ftmp = cLSF[(2 * k) as usize]; /* QA*/
        out[(k + 1) as usize] = silk_LSHIFT(out[(k - 1) as usize], 1)
            .wrapping_sub(silk_RSHIFT_ROUND64(silk_SMULL(ftmp, out[k as usize]), QA) as opus_int32);
        n = k;
        while n > 1 {
            out[n as usize] = out[n as usize].wrapping_add(
                out[(n - 2) as usize]
                    .wrapping_sub(
                        silk_RSHIFT_ROUND64(silk_SMULL(ftmp, out[(n - 1) as usize]), QA)
                            as opus_int32,
                    ),
            );
            n -= 1;
        }
        out[1] = out[1].wrapping_sub(ftmp);
    }
}

/* compute whitening filter coefficients from normalized line spectral frequencies */
/// `silk_NLSF2A(a_Q12, NLSF, d, arch)`
pub fn silk_NLSF2A(
    a_Q12: &mut [opus_int16],   /* O    monic whitening filter coefficients in Q12, [ d ] */
    NLSF: &[opus_int16],        /* I    normalized line spectral frequencies in Q15, [ d ] */
    d: opus_int,                /* I    filter order (should be even) */
    arch: opus_int,             /* I    Run-time architecture */
) {
    /* This ordering was found to maximize quality. It improves numerical accuracy of
       silk_NLSF2A_find_poly() compared to "standard" ordering. */
    static ORDERING16: [u8; 16] = [0, 15, 8, 7, 4, 11, 12, 3, 2, 13, 10, 5, 6, 9, 14, 1];
    static ORDERING10: [u8; 10] = [0, 9, 6, 3, 4, 5, 8, 1, 2, 7];
    let ordering: &[u8];
    let mut i: opus_int;
    let dd: opus_int;
    let mut cos_LSF_QA = [0 as opus_int32; SILK_MAX_ORDER_LPC as usize];
    let mut P = [0 as opus_int32; (SILK_MAX_ORDER_LPC / 2 + 1) as usize];
    let mut Q = [0 as opus_int32; (SILK_MAX_ORDER_LPC / 2 + 1) as usize];
    let mut Ptmp: opus_int32;
    let mut Qtmp: opus_int32;
    let mut f_int: opus_int32;
    let mut f_frac: opus_int32;
    let mut cos_val: opus_int32;
    let mut delta: opus_int32;
    let mut a32_QA1 = [0 as opus_int32; SILK_MAX_ORDER_LPC as usize];

    let _ = arch; /* the C macro expands to `((void)(arch), ...)` */

    silk_assert!(LSF_COS_TAB_SZ_FIX == 128);
    celt_assert!(d == 10 || d == 16);

    /* convert LSFs to 2*cos(LSF), using piecewise linear curve from table */
    ordering = if d == 16 { &ORDERING16 } else { &ORDERING10 };
    for k in 0..d {
        silk_assert!(NLSF[k as usize] >= 0);

        /* f_int on a scale 0-127 (rounded down) */
        f_int = silk_RSHIFT(NLSF[k as usize] as opus_int32, 15 - 7);

        /* f_frac, range: 0..255 */
        f_frac = NLSF[k as usize] as opus_int32 - silk_LSHIFT(f_int, 15 - 7);

        silk_assert!(f_int >= 0);
        silk_assert!(f_int < LSF_COS_TAB_SZ_FIX);

        /* Read start and end value from table */
        cos_val = silk_LSFCosTab_FIX_Q12[f_int as usize] as opus_int32; /* Q12 */
        delta = silk_LSFCosTab_FIX_Q12[(f_int + 1) as usize] as opus_int32 - cos_val; /* Q12, with a range of 0..200 */

        /* Linear interpolation */
        cos_LSF_QA[ordering[k as usize] as usize] = silk_RSHIFT_ROUND(
            silk_LSHIFT(cos_val, 8) + silk_MUL(delta, f_frac),
            20 - QA,
        ); /* QA */
    }

    dd = silk_RSHIFT(d, 1);

    /* generate even and odd polynomials using convolution */
    silk_NLSF2A_find_poly(&mut P, &cos_LSF_QA[0..], dd);
    silk_NLSF2A_find_poly(&mut Q, &cos_LSF_QA[1..], dd);

    /* convert even and odd polynomials to opus_int32 Q12 filter coefs */
    for k in 0..dd {
        Ptmp = P[(k + 1) as usize].wrapping_add(P[k as usize]);
        Qtmp = Q[(k + 1) as usize].wrapping_sub(Q[k as usize]);

        /* the Ptmp and Qtmp values at this stage need to fit in int32 */
        a32_QA1[k as usize] = Qtmp.wrapping_neg().wrapping_sub(Ptmp); /* QA+1 */
        a32_QA1[(d - k - 1) as usize] = Qtmp.wrapping_sub(Ptmp); /* QA+1 */
    }

    /* Convert int32 coefficients to Q12 int16 coefs */
    silk_LPC_fit(a_Q12, &mut a32_QA1, 12, QA + 1, d);

    i = 0;
    while silk_LPC_inverse_pred_gain_c(a_Q12, d) == 0 && i < MAX_LPC_STABILIZE_ITERATIONS {
        /* Prediction coefficients are (too close to) unstable; apply bandwidth expansion   */
        /* on the unscaled coefficients, convert to Q12 and measure again                   */
        silk_bwexpander_32(&mut a32_QA1, d, 65536 - silk_LSHIFT(2, i));
        for k in 0..d {
            a_Q12[k as usize] = silk_RSHIFT_ROUND(a32_QA1[k as usize], QA + 1 - 12) as opus_int16; /* QA+1 -> Q12 */
        }
        i += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::silk::lpc_inv_pred_gain::silk_LPC_inverse_pred_gain_c;
    use crate::silk::nlsf_stabilize::silk_NLSF_stabilize;
    use crate::silk::tables_nlsf_cb_wb::silk_NLSF_CB_WB;

    #[test]
    fn stabilized_nlsf_gives_a_stable_filter() {
        let cb = &silk_NLSF_CB_WB;
        let mut nlsf = [0i16; 16];
        for (i, v) in nlsf.iter_mut().enumerate() {
            *v = (1000 + 1800 * i as i32) as i16;
        }
        silk_NLSF_stabilize(&mut nlsf, cb.deltaMin_Q15, 16);
        let mut a = [0i16; 16];
        silk_NLSF2A(&mut a, &nlsf, 16, 0);
        assert_ne!(a, [0i16; 16]);
        assert!(silk_LPC_inverse_pred_gain_c(&a, 16) > 0);
    }
}

//! Mirror of `silk/A2NLSF.c` from libopus 1.6.1, transliterated to safe Rust.
//!
//! The C threads a `opus_int32 *p` pointer that alternates between the `P` and
//! `Q` polynomials; the port keeps the same two arrays and an `p_is_q` flag.
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

use crate::silk::bwexpander_32::*;
use crate::silk::macros::*;
use crate::silk::sigproc_fix::*;
use crate::silk::table_lsf_cos::*;
use crate::silk::typedef::silk_assert;
use crate::silk::typedef::*;
use crate::types::*;
use crate::silk::define::LSF_COS_TAB_SZ_FIX;

/* Number of binary divisions, when not in low complexity mode */
const BIN_DIV_STEPS_A2NLSF_FIX: opus_int = 3; /* must be no higher than 16 - log2( LSF_COS_TAB_SZ_FIX ) */
const MAX_ITERATIONS_A2NLSF_FIX: opus_int = 16;

/* Helper function for A2NLSF(..)                    */
/* Transforms polynomials from cos(n*f) to cos(f)^n  */
fn silk_A2NLSF_trans_poly(p: &mut [opus_int32], dd: opus_int) {
    for k in 2..=dd {
        for n in (k + 1..=dd).rev() {
            p[(n - 2) as usize] -= p[n as usize];
        }
        p[(k - 2) as usize] -= silk_LSHIFT(p[k as usize], 1);
    }
}

/* Helper function for A2NLSF(..) */
/* Polynomial evaluation          */
fn silk_A2NLSF_eval_poly(p: &[opus_int32], x: opus_int32, dd: opus_int) -> opus_int32 {
    let mut y32 = p[dd as usize]; /* Q16 */
    let x_Q16 = silk_LSHIFT(x, 4);

    if 8 == dd {
        y32 = silk_SMLAWW(p[7], y32, x_Q16);
        y32 = silk_SMLAWW(p[6], y32, x_Q16);
        y32 = silk_SMLAWW(p[5], y32, x_Q16);
        y32 = silk_SMLAWW(p[4], y32, x_Q16);
        y32 = silk_SMLAWW(p[3], y32, x_Q16);
        y32 = silk_SMLAWW(p[2], y32, x_Q16);
        y32 = silk_SMLAWW(p[1], y32, x_Q16);
        y32 = silk_SMLAWW(p[0], y32, x_Q16);
    } else {
        let mut n = dd - 1;
        while n >= 0 {
            y32 = silk_SMLAWW(p[n as usize], y32, x_Q16); /* Q16 */
            n -= 1;
        }
    }
    y32
}

fn silk_A2NLSF_init(a_Q16: &[opus_int32], P: &mut [opus_int32], Q: &mut [opus_int32], dd: opus_int) {
    /* Convert filter coefs to even and odd polynomials */
    P[dd as usize] = silk_LSHIFT(1, 16);
    Q[dd as usize] = silk_LSHIFT(1, 16);
    for k in 0..dd {
        P[k as usize] = -a_Q16[(dd - k - 1) as usize] - a_Q16[(dd + k) as usize]; /* Q16 */
        Q[k as usize] = -a_Q16[(dd - k - 1) as usize] + a_Q16[(dd + k) as usize]; /* Q16 */
    }

    /* Divide out zeros as we have that for even filter orders, */
    /* z =  1 is always a root in Q, and                        */
    /* z = -1 is always a root in P                             */
    for k in (1..=dd).rev() {
        P[(k - 1) as usize] -= P[k as usize];
        Q[(k - 1) as usize] += Q[k as usize];
    }

    /* Transform polynomials from cos(n*f) to cos(f)^n */
    silk_A2NLSF_trans_poly(P, dd);
    silk_A2NLSF_trans_poly(Q, dd);
}

/* Compute Normalized Line Spectral Frequencies (NLSFs) from whitening filter coefficients      */
/* If not all roots are found, the a_Q16 coefficients are bandwidth expanded until convergence. */
pub fn silk_A2NLSF(
    NLSF: &mut [opus_int16], /* O    Normalized Line Spectral Frequencies in Q15 (0..2^15-1) [d] */
    a_Q16: &mut [opus_int32], /* I/O  Monic whitening filter coefficients in Q16 [d]              */
    d: opus_int,             /* I    Filter order (must be even)                                 */
) {
    let mut P = [0i32; (SILK_MAX_ORDER_LPC / 2 + 1) as usize];
    let mut Q = [0i32; (SILK_MAX_ORDER_LPC / 2 + 1) as usize];

    let dd = silk_RSHIFT(d, 1);

    silk_A2NLSF_init(a_Q16, &mut P, &mut Q, dd);

    /* Find roots, alternating between P and Q */
    let mut p_is_q = false; /* p = P */

    let mut xlo = silk_LSFCosTab_FIX_Q12[0] as opus_int32; /* Q12*/
    let mut ylo = silk_A2NLSF_eval_poly(if p_is_q { &Q } else { &P }, xlo, dd);

    let mut root_ix: opus_int;
    if ylo < 0 {
        /* Set the first NLSF to zero and move on to the next */
        NLSF[0] = 0;
        p_is_q = true; /* p = Q */
        ylo = silk_A2NLSF_eval_poly(&Q, xlo, dd);
        root_ix = 1; /* Index of current root */
    } else {
        root_ix = 0; /* Index of current root */
    }
    let mut k: opus_int = 1; /* Loop counter */
    let mut i: opus_int = 0; /* Counter for bandwidth expansions applied */
    let mut thr: opus_int32 = 0;
    let mut yhi: opus_int32;
    loop {
        /* Evaluate polynomial */
        let mut xhi = silk_LSFCosTab_FIX_Q12[k as usize] as opus_int32; /* Q12 */
        yhi = silk_A2NLSF_eval_poly(if p_is_q { &Q } else { &P }, xhi, dd);

        /* Detect zero crossing */
        if (ylo <= 0 && yhi >= thr) || (ylo >= 0 && yhi <= -thr) {
            if yhi == 0 {
                /* If the root lies exactly at the end of the current       */
                /* interval, look for the next root in the next interval    */
                thr = 1;
            } else {
                thr = 0;
            }
            /* Binary division */
            let mut ffrac: opus_int32 = -256;
            for m in 0..BIN_DIV_STEPS_A2NLSF_FIX {
                /* Evaluate polynomial */
                let xmid = silk_RSHIFT_ROUND(xlo + xhi, 1);
                let ymid = silk_A2NLSF_eval_poly(if p_is_q { &Q } else { &P }, xmid, dd);

                /* Detect zero crossing */
                if (ylo <= 0 && ymid >= 0) || (ylo >= 0 && ymid <= 0) {
                    /* Reduce frequency */
                    xhi = xmid;
                    yhi = ymid;
                } else {
                    /* Increase frequency */
                    xlo = xmid;
                    ylo = ymid;
                    ffrac = silk_ADD_RSHIFT(ffrac, 128, m);
                }
            }

            /* Interpolate */
            if silk_abs(ylo) < 65536 {
                /* Avoid dividing by zero */
                let den = ylo - yhi;
                let nom = silk_LSHIFT(ylo, 8 - BIN_DIV_STEPS_A2NLSF_FIX) + silk_RSHIFT(den, 1);
                if den != 0 {
                    ffrac += silk_DIV32(nom, den);
                }
            } else {
                /* No risk of dividing by zero because abs(ylo - yhi) >= abs(ylo) >= 65536 */
                ffrac += silk_DIV32(ylo, silk_RSHIFT(ylo - yhi, 8 - BIN_DIV_STEPS_A2NLSF_FIX));
            }
            NLSF[root_ix as usize] =
                silk_min_32(silk_LSHIFT(k, 8) + ffrac, silk_int16_MAX as opus_int32) as opus_int16;

            silk_assert!(NLSF[root_ix as usize] >= 0);

            root_ix += 1; /* Next root */
            if root_ix >= d {
                /* Found all roots */
                break;
            }
            /* Alternate pointer to polynomial */
            p_is_q = (root_ix & 1) == 1;

            /* Evaluate polynomial */
            xlo = silk_LSFCosTab_FIX_Q12[(k - 1) as usize] as opus_int32; /* Q12*/
            ylo = silk_LSHIFT(1 - (root_ix & 2), 12);
        } else {
            /* Increment loop counter */
            k += 1;
            xlo = xhi;
            ylo = yhi;
            thr = 0;

            if k > LSF_COS_TAB_SZ_FIX {
                i += 1;
                if i > MAX_ITERATIONS_A2NLSF_FIX {
                    /* Set NLSFs to white spectrum and exit */
                    NLSF[0] = silk_DIV32_16(1 << 15, d + 1) as opus_int16;
                    for k in 1..d {
                        NLSF[k as usize] =
                            silk_ADD16(NLSF[(k - 1) as usize], NLSF[0]) as opus_int16;
                    }
                    return;
                }

                /* Error: Apply progressively more bandwidth expansion and run again */
                silk_bwexpander_32(a_Q16, d, 65536 - silk_LSHIFT(1, i));

                silk_A2NLSF_init(a_Q16, &mut P, &mut Q, dd);
                p_is_q = false; /* p = P */
                xlo = silk_LSFCosTab_FIX_Q12[0] as opus_int32; /* Q12*/
                ylo = silk_A2NLSF_eval_poly(&P, xlo, dd);
                if ylo < 0 {
                    /* Set the first NLSF to zero and move on to the next */
                    NLSF[0] = 0;
                    p_is_q = true; /* p = Q */
                    ylo = silk_A2NLSF_eval_poly(&Q, xlo, dd);
                    root_ix = 1; /* Index of current root */
                } else {
                    root_ix = 0; /* Index of current root */
                }
                k = 1; /* Reset loop counter */
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `silk_A2NLSF_trans_poly` for dd = 2 only runs the `k = 2` row, i.e.
    /// `p[0] -= 2*p[2]` (the inner `n` loop is empty because `n > k`).
    #[test]
    fn trans_poly_small_case() {
        let mut p = [3i32, 5, 7, 0];
        silk_A2NLSF_trans_poly(&mut p, 2);
        assert_eq!(p, [3 - 14, 5, 7, 0]);
    }

    /// A2NLSF of a flat (all-zero) filter produces d strictly increasing
    /// normalised frequencies inside [0, 2^15).
    #[test]
    fn zero_filter_gives_increasing_spectrum() {
        for &d in &[10i32, 16] {
            let mut a = vec![0i32; d as usize];
            let mut nlsf = vec![0i16; d as usize];
            silk_A2NLSF(&mut nlsf, &mut a, d);
            for i in 0..d as usize {
                assert!(nlsf[i] >= 0);
                if i > 0 {
                    assert!(nlsf[i] > nlsf[i - 1], "NLSF must increase: {:?}", nlsf);
                }
            }
        }
    }
}

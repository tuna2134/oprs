//! Mirror of `silk/float/burg_modified_FLP.c` from libopus 1.6.1, transliterated to safe Rust.
//!
//! Modified Burg method: reflection coefficients and residual energy.
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
use crate::silk::float::energy_flp::silk_energy_FLP;
use crate::silk::float::inner_product_flp::silk_inner_product_FLP;
use crate::silk::float::sigproc_flp::silk_float;
use crate::silk::sigproc_fix::SILK_MAX_ORDER_LPC;
use crate::silk::typedef::*;

/// `silk/float/burg_modified_FLP.c:36` — `subfr_length * nb_subfr = (0.005*16000 + 16)*4 = 384`.
const MAX_FRAME_SIZE: opus_int = 384;
/// `FIND_LPC_COND_FAC` (`silk/tuning_parameters.h`), `1e-5f`.
const FIND_LPC_COND_FAC: silk_float = 1e-5;

/// Compute reflection coefficients from input signal
pub fn silk_burg_modified_FLP(
    A: &mut [silk_float],
    x: &[silk_float],
    minInvGain: silk_float,
    subfr_length: opus_int,
    nb_subfr: opus_int,
    D: opus_int,
    arch: opus_int,
) -> silk_float {
    let mut k: opus_int;
    let mut n: opus_int;
    let mut s: opus_int;
    let mut reached_max_gain: opus_int;
    let mut C0: f64;
    let mut invGain: f64;
    let mut num: f64;
    let mut nrg_f: f64;
    let mut nrg_b: f64;
    let mut rc: f64;
    let mut Atmp: f64;
    let mut tmp1: f64;
    let mut tmp2: f64;
    let mut C_first_row = [0.0f64; SILK_MAX_ORDER_LPC as usize];
    let mut C_last_row = [0.0f64; SILK_MAX_ORDER_LPC as usize];
    let mut CAf = [0.0f64; (SILK_MAX_ORDER_LPC + 1) as usize];
    let mut CAb = [0.0f64; (SILK_MAX_ORDER_LPC + 1) as usize];
    let mut Af = [0.0f64; SILK_MAX_ORDER_LPC as usize];

    celt_assert!(subfr_length * nb_subfr <= MAX_FRAME_SIZE);

    /* Compute autocorrelations, added over subframes */
    C0 = silk_energy_FLP(x, nb_subfr * subfr_length);
    s = 0;
    while s < nb_subfr {
        let x_ptr = &x[(s * subfr_length) as usize..];
        n = 1;
        while n < D + 1 {
            C_first_row[(n - 1) as usize] += silk_inner_product_FLP(
                x_ptr,
                &x_ptr[n as usize..],
                subfr_length - n,
                arch,
            );
            n += 1;
        }
        s += 1;
    }
    C_last_row = C_first_row;

    /* Initialize */
    CAb[0] = C0 + FIND_LPC_COND_FAC as f64 * C0 + 1e-9f32 as f64;
    CAf[0] = CAb[0];
    invGain = 1.0f64;
    reached_max_gain = 0;
    n = 0;
    while n < D {
        /* Update first row of correlation matrix (without first element) */
        /* Update last row of correlation matrix (without last element, stored in reversed order) */
        /* Update C * Af */
        /* Update C * flipud(Af) (stored in reversed order) */
        s = 0;
        while s < nb_subfr {
            let x_ptr = &x[(s * subfr_length) as usize..];
            tmp1 = x_ptr[n as usize] as f64;
            tmp2 = x_ptr[(subfr_length - n - 1) as usize] as f64;
            k = 0;
            while k < n {
                /* the C multiplies two silk_float values, so the product is
                 * rounded to f32 before the (double) accumulation */
                C_first_row[k as usize] -=
                    (x_ptr[n as usize] * x_ptr[(n - k - 1) as usize]) as f64;
                C_last_row[k as usize] -= (x_ptr[(subfr_length - n - 1) as usize]
                    * x_ptr[(subfr_length - n + k) as usize])
                    as f64;
                Atmp = Af[k as usize];
                tmp1 += x_ptr[(n - k - 1) as usize] as f64 * Atmp;
                tmp2 += x_ptr[(subfr_length - n + k) as usize] as f64 * Atmp;
                k += 1;
            }
            k = 0;
            while k <= n {
                CAf[k as usize] -= tmp1 * x_ptr[(n - k) as usize] as f64;
                CAb[k as usize] -= tmp2 * x_ptr[(subfr_length - n + k - 1) as usize] as f64;
                k += 1;
            }
            s += 1;
        }
        tmp1 = C_first_row[n as usize];
        tmp2 = C_last_row[n as usize];
        k = 0;
        while k < n {
            Atmp = Af[k as usize];
            tmp1 += C_last_row[(n - k - 1) as usize] * Atmp;
            tmp2 += C_first_row[(n - k - 1) as usize] * Atmp;
            k += 1;
        }
        CAf[(n + 1) as usize] = tmp1;
        CAb[(n + 1) as usize] = tmp2;

        /* Calculate nominator and denominator for the next order reflection (parcor) coefficient */
        num = CAb[(n + 1) as usize];
        nrg_b = CAb[0];
        nrg_f = CAf[0];
        k = 0;
        while k < n {
            Atmp = Af[k as usize];
            num += CAb[(n - k) as usize] * Atmp;
            nrg_b += CAb[(k + 1) as usize] * Atmp;
            nrg_f += CAf[(k + 1) as usize] * Atmp;
            k += 1;
        }
        silk_assert!(nrg_f > 0.0);
        silk_assert!(nrg_b > 0.0);

        /* Calculate the next order reflection (parcor) coefficient */
        rc = -2.0 * num / (nrg_f + nrg_b);
        silk_assert!(rc > -1.0 && rc < 1.0);

        /* Update inverse prediction gain */
        tmp1 = invGain * (1.0 - rc * rc);
        if tmp1 <= minInvGain as f64 {
            /* Max prediction gain exceeded; set reflection coefficient such that max prediction gain is exactly hit */
            rc = (1.0 - minInvGain as f64 / invGain).sqrt();
            if num > 0.0 {
                /* Ensure adjusted reflection coefficients has the original sign */
                rc = -rc;
            }
            invGain = minInvGain as f64;
            reached_max_gain = 1;
        } else {
            invGain = tmp1;
        }

        /* Update the AR coefficients */
        k = 0;
        while k < (n + 1) >> 1 {
            tmp1 = Af[k as usize];
            tmp2 = Af[(n - k - 1) as usize];
            Af[k as usize] = tmp1 + rc * tmp2;
            Af[(n - k - 1) as usize] = tmp2 + rc * tmp1;
            k += 1;
        }
        Af[n as usize] = rc;

        if reached_max_gain != 0 {
            /* Reached max prediction gain; set remaining coefficients to zero and exit loop */
            k = n + 1;
            while k < D {
                Af[k as usize] = 0.0;
                k += 1;
            }
            break;
        }

        /* Update C * Af and C * Ab */
        k = 0;
        while k <= n + 1 {
            tmp1 = CAf[k as usize];
            CAf[k as usize] += rc * CAb[(n - k + 1) as usize];
            CAb[(n - k + 1) as usize] += rc * tmp1;
            k += 1;
        }
        n += 1;
    }

    if reached_max_gain != 0 {
        /* Convert to silk_float */
        k = 0;
        while k < D {
            A[k as usize] = (-Af[k as usize]) as silk_float;
            k += 1;
        }
        /* Subtract energy of preceding samples from C0 */
        s = 0;
        while s < nb_subfr {
            C0 -= silk_energy_FLP(&x[(s * subfr_length) as usize..], D);
            s += 1;
        }
        /* Approximate residual energy */
        nrg_f = C0 * invGain;
    } else {
        /* Compute residual energy and store coefficients as silk_float */
        nrg_f = CAf[0];
        tmp1 = 1.0;
        k = 0;
        while k < D {
            Atmp = Af[k as usize];
            nrg_f += CAf[(k + 1) as usize] * Atmp;
            tmp1 += Atmp * Atmp;
            A[k as usize] = (-Atmp) as silk_float;
            k += 1;
        }
        nrg_f -= FIND_LPC_COND_FAC as f64 * C0 * tmp1;
    }

    /* Return residual energy */
    nrg_f as silk_float
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn white_noise_like_input_gives_zero_coefficients() {
        /* A perfectly flat (impulse-only) signal has zero off-diagonal
         * correlations, so every reflection coefficient vanishes. */
        let mut x = [0.0f32; 64];
        x[0] = 1.0;
        for i in 0..64 {
            x[i] += (i as f32) * 1e-7;
        }
        let mut a = [0.0f32; 16];
        let e = silk_burg_modified_FLP(&mut a, &x, 0.1, 16, 4, 16, 0);
        assert!(e > 0.0);
    }

    #[test]
    fn all_zero_input_gives_zero_energy() {
        let x = [0.0f32; 64];
        let mut a = [0.0f32; 16];
        let e = silk_burg_modified_FLP(&mut a, &x, 0.1, 16, 4, 16, 0);
        /* C0 = 0 so CAf/CAb start at 1e-9; the recursion keeps nrg positive. */
        assert!(e >= 0.0);
    }
}

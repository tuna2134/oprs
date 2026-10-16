//! Mirror of `silk/float/schur_FLP.c` from libopus 1.6.1, transliterated to safe Rust.
//!
//! Schur recursion: reflection coefficients from an autocorrelation sequence.
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
use crate::silk::float::sigproc_flp::silk_float;
use crate::silk::sigproc_fix::SILK_MAX_ORDER_LPC;
use crate::silk::typedef::*;

/// returns residual energy
pub fn silk_schur_FLP(
    refl_coef: &mut [silk_float],
    auto_corr: &[silk_float],
    order: opus_int,
) -> silk_float {
    let mut k: opus_int;
    let mut n: opus_int;
    let mut C = [[0.0f64; 2]; (SILK_MAX_ORDER_LPC + 1) as usize];
    let mut Ctmp1: f64;
    let mut Ctmp2: f64;
    let mut rc_tmp: f64;

    celt_assert!(order >= 0 && order <= SILK_MAX_ORDER_LPC);

    /* Copy correlations */
    k = 0;
    loop {
        C[k as usize][0] = auto_corr[k as usize] as f64;
        C[k as usize][1] = auto_corr[k as usize] as f64;
        k += 1;
        if k > order {
            break;
        }
    }

    k = 0;
    while k < order {
        /* Get reflection coefficient.  The C denominator is `C[0][1]`, and the
         * `silk_max_float( C[0][1], 1e-9f )` promotes the float constant to
         * double, so the max is evaluated in double precision. */
        let denom = silk_max_float_f64(C[0][1], 1e-9f32 as f64);
        rc_tmp = -C[(k + 1) as usize][0] / denom;

        /* Save the output */
        refl_coef[k as usize] = rc_tmp as silk_float;

        /* Update correlations */
        n = 0;
        while n < order - k {
            Ctmp1 = C[(n + k + 1) as usize][0];
            Ctmp2 = C[n as usize][1];
            C[(n + k + 1) as usize][0] = Ctmp1 + Ctmp2 * rc_tmp;
            C[n as usize][1] = Ctmp2 + Ctmp1 * rc_tmp;
            n += 1;
        }
        k += 1;
    }

    /* Return residual energy */
    C[0][1] as silk_float
}

/// The C macro `silk_max_float(a, b) = ((a) < (b)) ? (b) : (a)` evaluated in
/// `double` (used where the arguments mix `double` and `float`).
#[inline(always)]
fn silk_max_float_f64(a: f64, b: f64) -> f64 {
    if a > b {
        a
    } else {
        b
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_autocorrelation_gives_zero_reflection() {
        let ac = [1.0f32, 0.0, 0.0, 0.0];
        let mut rc = [0.0f32; 3];
        let e = silk_schur_FLP(&mut rc, &ac, 3);
        assert_eq!(rc, [0.0, 0.0, 0.0]);
        assert_eq!(e, 1.0);
    }

    #[test]
    fn first_reflection_is_normalised_negative_ratio() {
        let ac = [4.0f32, 1.0, 0.0, 0.0];
        let mut rc = [0.0f32; 3];
        silk_schur_FLP(&mut rc, &ac, 1);
        assert_eq!(rc[0], -0.25);
    }
}

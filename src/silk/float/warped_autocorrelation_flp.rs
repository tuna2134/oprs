//! Mirror of `silk/float/warped_autocorrelation_FLP.c` from libopus 1.6.1, transliterated to safe Rust.
//!
//! Autocorrelations on a warped frequency axis.
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
use crate::silk::define::MAX_SHAPE_LPC_ORDER;
use crate::silk::float::sigproc_flp::silk_float;
use crate::silk::typedef::*;

/// Autocorrelations for a warped frequency axis
pub fn silk_warped_autocorrelation_FLP(
    corr: &mut [silk_float],
    input: &[silk_float],
    warping: silk_float,
    length: opus_int,
    order: opus_int,
) {
    let mut n: opus_int;
    let mut i: opus_int;
    let mut tmp1: f64;
    let mut tmp2: f64;
    let mut state = [0.0f64; (MAX_SHAPE_LPC_ORDER + 1) as usize];
    let mut C = [0.0f64; (MAX_SHAPE_LPC_ORDER + 1) as usize];

    /* Order must be even */
    celt_assert!((order & 1) == 0);

    let w = warping as f64;

    /* Loop over samples */
    n = 0;
    while n < length {
        tmp1 = input[n as usize] as f64;
        /* Loop over allpass sections */
        i = 0;
        while i < order {
            /* Output of allpass section */
            /* We voluntarily use two multiples instead of factoring the expression to
               reduce the length of the dependency chain (tmp1->tmp2->tmp1... ). */
            tmp2 = state[i as usize] + w * state[(i + 1) as usize] - w * tmp1;
            state[i as usize] = tmp1;
            C[i as usize] += state[0] * tmp1;
            /* Output of allpass section */
            tmp1 = state[(i + 1) as usize] + w * state[(i + 2) as usize] - w * tmp2;
            state[(i + 1) as usize] = tmp2;
            C[(i + 1) as usize] += state[0] * tmp2;
            i += 2;
        }
        state[order as usize] = tmp1;
        C[order as usize] += state[0] * tmp1;
        n += 1;
    }

    /* Copy correlations in silk_float output format */
    i = 0;
    while i < order + 1 {
        corr[i as usize] = C[i as usize] as silk_float;
        i += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_warping_matches_plain_autocorrelation() {
        let x: [f32; 8] = [1.0, 2.0, 3.0, 4.0, -1.0, -2.0, 0.5, 0.25];
        let mut corr = [0.0f32; 5];
        silk_warped_autocorrelation_FLP(&mut corr, &x, 0.0, 8, 4);
        /* With warping 0 every allpass reduces to a delay line, so corr[k] is
         * the plain (partial) autocorrelation over the first 8-k samples. */
        let mut expect = [0.0f64; 5];
        for k in 0..5 {
            for n in 0..8 - k {
                expect[k] += x[n] as f64 * x[n + k] as f64;
            }
        }
        for k in 0..5 {
            assert_eq!(corr[k], expect[k] as f32, "k={k}");
        }
    }
}

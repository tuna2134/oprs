//! Mirror of `silk/ana_filt_bank_1.c` from libopus 1.6.1, transliterated to safe Rust.
//!
//! Split a signal into two decimated bands using first-order allpass filters.
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
use crate::types::*;

/* Coefficients for 2-band filter bank based on first-order allpass filters */
static A_fb1_20: opus_int16 = 5394 << 1;
static A_fb1_21: opus_int16 = -24290; /* (opus_int16)(20623 << 1) */

/// `silk_ana_filt_bank_1(in, S, outL, outH, N)`
pub fn silk_ana_filt_bank_1(
    in_: &[opus_int16],       /* I    Input signal [N] */
    S: &mut [opus_int32],     /* I/O  State vector [2] */
    outL: &mut [opus_int16],  /* O    Low band [N/2] */
    outH: &mut [opus_int16],  /* O    High band [N/2] */
    N: opus_int32,            /* I    Number of input samples */
) {
    let N2: opus_int32 = silk_RSHIFT(N, 1);
    let mut in32: opus_int32;
    let mut X: opus_int32;
    let mut Y: opus_int32;
    let mut out_1: opus_int32;
    let mut out_2: opus_int32;

    /* Internal variables and state are in Q10 format */
    for k in 0..N2 {
        /* Convert to Q10 */
        in32 = silk_LSHIFT(in_[(2 * k) as usize] as opus_int32, 10);

        /* All-pass section for even input sample */
        Y = silk_SUB32(in32, S[0]);
        X = silk_SMLAWB(Y, Y, A_fb1_21 as opus_int32);
        out_1 = silk_ADD32(S[0], X);
        S[0] = silk_ADD32(in32, X);

        /* Convert to Q10 */
        in32 = silk_LSHIFT(in_[(2 * k + 1) as usize] as opus_int32, 10);

        /* All-pass section for odd input sample, and add to output of previous section */
        Y = silk_SUB32(in32, S[1]);
        X = silk_SMULWB(Y, A_fb1_20 as opus_int32);
        out_2 = silk_ADD32(S[1], X);
        S[1] = silk_ADD32(in32, X);

        /* Add/subtract, convert back to int16 and store to output */
        outL[k as usize] =
            silk_SAT16(silk_RSHIFT_ROUND(silk_ADD32(out_2, out_1), 11)) as opus_int16;
        outH[k as usize] =
            silk_SAT16(silk_RSHIFT_ROUND(silk_SUB32(out_2, out_1), 11)) as opus_int16;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_input_gives_zero_output() {
        let mut s = [0i32; 2];
        let mut out_l = [0i16; 4];
        let mut out_h = [0i16; 4];
        silk_ana_filt_bank_1(&[0i16; 8], &mut s, &mut out_l, &mut out_h, 8);
        assert_eq!(out_l, [0i16; 4]);
        assert_eq!(out_h, [0i16; 4]);
        assert_eq!(s, [0, 0]);
    }
}

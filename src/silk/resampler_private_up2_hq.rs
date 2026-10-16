//! Mirror of `silk/resampler_private_up2_HQ.c` from libopus 1.6.1, transliterated to safe Rust.
//!
//! Upsample by a factor 2, high quality.  Uses 2nd order allpass filters for
//! the 2x upsampling, followed by a notch filter just above Nyquist.
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
use crate::silk::resampler_rom::*;
use crate::silk::sigproc_fix::*;
use crate::silk::structs::*;
use crate::silk::typedef::*;
use crate::types::*;

/// `silk_resampler_private_up2_HQ(S, out, in, len)`
pub fn silk_resampler_private_up2_HQ(
    S: &mut [opus_int32],     /* I/O  Resampler state [ 6 ] */
    out: &mut [opus_int16],   /* O    Output signal [ 2 * len ] */
    in_: &[opus_int16],       /* I    Input signal [ len ] */
    len: opus_int32,          /* I    Number of input samples */
) {
    let mut in32: opus_int32;
    let mut out32_1: opus_int32;
    let mut out32_2: opus_int32;
    let mut Y: opus_int32;
    let mut X: opus_int32;

    silk_assert!(silk_resampler_up2_hq_0[0] > 0);
    silk_assert!(silk_resampler_up2_hq_0[1] > 0);
    silk_assert!(silk_resampler_up2_hq_0[2] < 0);
    silk_assert!(silk_resampler_up2_hq_1[0] > 0);
    silk_assert!(silk_resampler_up2_hq_1[1] > 0);
    silk_assert!(silk_resampler_up2_hq_1[2] < 0);

    /* Internal variables and state are in Q10 format */
    for k in 0..len {
        /* Convert to Q10 */
        in32 = silk_LSHIFT(in_[k as usize] as opus_int32, 10);

        /* First all-pass section for even output sample */
        Y = silk_SUB32(in32, S[0]);
        X = silk_SMULWB(Y, silk_resampler_up2_hq_0[0] as opus_int32);
        out32_1 = silk_ADD32(S[0], X);
        S[0] = silk_ADD32(in32, X);

        /* Second all-pass section for even output sample */
        Y = silk_SUB32(out32_1, S[1]);
        X = silk_SMULWB(Y, silk_resampler_up2_hq_0[1] as opus_int32);
        out32_2 = silk_ADD32(S[1], X);
        S[1] = silk_ADD32(out32_1, X);

        /* Third all-pass section for even output sample */
        Y = silk_SUB32(out32_2, S[2]);
        X = silk_SMLAWB(Y, Y, silk_resampler_up2_hq_0[2] as opus_int32);
        out32_1 = silk_ADD32(S[2], X);
        S[2] = silk_ADD32(out32_2, X);

        /* Apply gain in Q15, convert back to int16 and store to output */
        out[(2 * k) as usize] = silk_SAT16(silk_RSHIFT_ROUND(out32_1, 10)) as opus_int16;

        /* First all-pass section for odd output sample */
        Y = silk_SUB32(in32, S[3]);
        X = silk_SMULWB(Y, silk_resampler_up2_hq_1[0] as opus_int32);
        out32_1 = silk_ADD32(S[3], X);
        S[3] = silk_ADD32(in32, X);

        /* Second all-pass section for odd output sample */
        Y = silk_SUB32(out32_1, S[4]);
        X = silk_SMULWB(Y, silk_resampler_up2_hq_1[1] as opus_int32);
        out32_2 = silk_ADD32(S[4], X);
        S[4] = silk_ADD32(out32_1, X);

        /* Third all-pass section for odd output sample */
        Y = silk_SUB32(out32_2, S[5]);
        X = silk_SMLAWB(Y, Y, silk_resampler_up2_hq_1[2] as opus_int32);
        out32_1 = silk_ADD32(S[5], X);
        S[5] = silk_ADD32(out32_2, X);

        /* Apply gain in Q15, convert back to int16 and store to output */
        out[(2 * k + 1) as usize] = silk_SAT16(silk_RSHIFT_ROUND(out32_1, 10)) as opus_int16;
    }
}

/// `silk_resampler_private_up2_HQ_wrapper(SS, out, in, len)`
pub fn silk_resampler_private_up2_HQ_wrapper(
    S: &mut silk_resampler_state_struct, /* I/O  Resampler state (unused) */
    out: &mut [opus_int16],              /* O    Output signal [ 2 * len ] */
    in_: &[opus_int16],                  /* I    Input signal [ len ] */
    len: opus_int32,                     /* I    Number of input samples */
) {
    silk_resampler_private_up2_HQ(&mut S.sIIR, out, in_, len);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_input_gives_zero_output() {
        let mut s = [0i32; 6];
        let mut out = [0i16; 8];
        silk_resampler_private_up2_HQ(&mut s, &mut out, &[0i16; 4], 4);
        assert_eq!(out, [0i16; 8]);
        assert_eq!(s, [0i32; 6]);
    }
}

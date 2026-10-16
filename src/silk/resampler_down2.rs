//! Mirror of `silk/resampler_down2.c` from libopus 1.6.1, transliterated to safe Rust.
//!
//! Downsample by a factor 2.
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
use crate::silk::macros::*;
use crate::silk::resampler_rom::*;
use crate::silk::sigproc_fix::*;
use crate::types::*;

/// `silk_resampler_down2(S, out, in, inLen)`
pub fn silk_resampler_down2(
    S: &mut [opus_int32],     /* I/O  State vector [ 2 ] */
    out: &mut [opus_int16],   /* O    Output signal [ floor(len/2) ] */
    in_: &[opus_int16],       /* I    Input signal [ len ] */
    inLen: opus_int32,        /* I    Number of input samples */
) {
    let len2: opus_int32 = silk_RSHIFT32(inLen, 1);
    let mut in32: opus_int32;
    let mut out32: opus_int32;
    let mut Y: opus_int32;
    let mut X: opus_int32;

    celt_assert!(silk_resampler_down2_0 > 0);
    celt_assert!(silk_resampler_down2_1 < 0);

    /* Internal variables and state are in Q10 format */
    for k in 0..len2 {
        /* Convert to Q10 */
        in32 = silk_LSHIFT(in_[(2 * k) as usize] as opus_int32, 10);

        /* All-pass section for even input sample */
        Y = silk_SUB32(in32, S[0]);
        X = silk_SMLAWB(Y, Y, silk_resampler_down2_1 as opus_int32);
        out32 = silk_ADD32(S[0], X);
        S[0] = silk_ADD32(in32, X);

        /* Convert to Q10 */
        in32 = silk_LSHIFT(in_[(2 * k + 1) as usize] as opus_int32, 10);

        /* All-pass section for odd input sample, and add to output of previous section */
        Y = silk_SUB32(in32, S[1]);
        X = silk_SMULWB(Y, silk_resampler_down2_0 as opus_int32);
        out32 = silk_ADD32(out32, S[1]);
        out32 = silk_ADD32(out32, X);
        S[1] = silk_ADD32(in32, X);

        /* Add, convert back to int16 and store to output */
        out[k as usize] = silk_SAT16(silk_RSHIFT_ROUND(out32, 11)) as opus_int16;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_input_gives_zero_output() {
        let mut s = [0i32; 2];
        let mut out = [0i16; 8];
        silk_resampler_down2(&mut s, &mut out, &[0i16; 16], 16);
        assert_eq!(out, [0i16; 8]);
        assert_eq!(s, [0, 0]);
    }
}

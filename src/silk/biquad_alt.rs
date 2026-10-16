//! Mirror of `silk/biquad_alt.c` from libopus 1.6.1, transliterated to safe Rust.
//!
//! Second order ARMA filter that can handle slowly varying filter coefficients.
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

/// `silk_biquad_alt_stride1(in, B_Q28, A_Q28, S, out, len)`
pub fn silk_biquad_alt_stride1(
    in_: &[opus_int16],       /* I     input signal */
    B_Q28: &[opus_int32],     /* I     MA coefficients [3] */
    A_Q28: &[opus_int32],     /* I     AR coefficients [2] */
    S: &mut [opus_int32],     /* I/O   State vector [2] */
    out: &mut [opus_int16],   /* O     output signal */
    len: opus_int32,          /* I     signal length (must be even) */
) {
    /* DIRECT FORM II TRANSPOSED (uses 2 element state vector) */
    let A0_L_Q28: opus_int32;
    let A0_U_Q28: opus_int32;
    let A1_L_Q28: opus_int32;
    let A1_U_Q28: opus_int32;

    /* Negate A_Q28 values and split in two parts */
    A0_L_Q28 = (-A_Q28[0]) & 0x00003FFF; /* lower part */
    A0_U_Q28 = silk_RSHIFT(-A_Q28[0], 14); /* upper part */
    A1_L_Q28 = (-A_Q28[1]) & 0x00003FFF; /* lower part */
    A1_U_Q28 = silk_RSHIFT(-A_Q28[1], 14); /* upper part */

    for k in 0..len {
        /* S[ 0 ], S[ 1 ]: Q12 */
        let inval = in_[k as usize] as opus_int32;
        let out32_Q14 = silk_LSHIFT(silk_SMLAWB(S[0], B_Q28[0], inval), 2);

        S[0] = S[1].wrapping_add(silk_RSHIFT_ROUND(silk_SMULWB(out32_Q14, A0_L_Q28), 14));
        S[0] = silk_SMLAWB(S[0], out32_Q14, A0_U_Q28);
        S[0] = silk_SMLAWB(S[0], B_Q28[1], inval);

        S[1] = silk_RSHIFT_ROUND(silk_SMULWB(out32_Q14, A1_L_Q28), 14);
        S[1] = silk_SMLAWB(S[1], out32_Q14, A1_U_Q28);
        S[1] = silk_SMLAWB(S[1], B_Q28[2], inval);

        /* Scale back to Q0 and saturate */
        out[k as usize] =
            silk_SAT16(silk_RSHIFT(out32_Q14.wrapping_add((1 << 14) - 1), 14)) as opus_int16;
    }
}

/// `silk_biquad_alt_stride2_c(in, B_Q28, A_Q28, S, out, len)`
pub fn silk_biquad_alt_stride2_c(
    in_: &[opus_int16],       /* I     input signal */
    B_Q28: &[opus_int32],     /* I     MA coefficients [3] */
    A_Q28: &[opus_int32],     /* I     AR coefficients [2] */
    S: &mut [opus_int32],     /* I/O   State vector [4] */
    out: &mut [opus_int16],   /* O     output signal */
    len: opus_int32,          /* I     signal length (must be even) */
) {
    /* DIRECT FORM II TRANSPOSED (uses 2 element state vector) */
    let A0_U_Q28: opus_int32;
    let A0_L_Q28: opus_int32;
    let A1_U_Q28: opus_int32;
    let A1_L_Q28: opus_int32;
    let mut out32_Q14 = [0 as opus_int32; 2];

    /* Negate A_Q28 values and split in two parts */
    A0_L_Q28 = (-A_Q28[0]) & 0x00003FFF; /* lower part */
    A0_U_Q28 = silk_RSHIFT(-A_Q28[0], 14); /* upper part */
    A1_L_Q28 = (-A_Q28[1]) & 0x00003FFF; /* lower part */
    A1_U_Q28 = silk_RSHIFT(-A_Q28[1], 14); /* upper part */

    for k in 0..len {
        /* S[ 0 ], S[ 1 ], S[ 2 ], S[ 3 ]: Q12 */
        out32_Q14[0] = silk_LSHIFT(silk_SMLAWB(S[0], B_Q28[0], in_[(2 * k) as usize] as opus_int32), 2);
        out32_Q14[1] =
            silk_LSHIFT(silk_SMLAWB(S[2], B_Q28[0], in_[(2 * k + 1) as usize] as opus_int32), 2);

        S[0] = S[1].wrapping_add(silk_RSHIFT_ROUND(silk_SMULWB(out32_Q14[0], A0_L_Q28), 14));
        S[2] = S[3].wrapping_add(silk_RSHIFT_ROUND(silk_SMULWB(out32_Q14[1], A0_L_Q28), 14));
        S[0] = silk_SMLAWB(S[0], out32_Q14[0], A0_U_Q28);
        S[2] = silk_SMLAWB(S[2], out32_Q14[1], A0_U_Q28);
        S[0] = silk_SMLAWB(S[0], B_Q28[1], in_[(2 * k) as usize] as opus_int32);
        S[2] = silk_SMLAWB(S[2], B_Q28[1], in_[(2 * k + 1) as usize] as opus_int32);

        S[1] = silk_RSHIFT_ROUND(silk_SMULWB(out32_Q14[0], A1_L_Q28), 14);
        S[3] = silk_RSHIFT_ROUND(silk_SMULWB(out32_Q14[1], A1_L_Q28), 14);
        S[1] = silk_SMLAWB(S[1], out32_Q14[0], A1_U_Q28);
        S[3] = silk_SMLAWB(S[3], out32_Q14[1], A1_U_Q28);
        S[1] = silk_SMLAWB(S[1], B_Q28[2], in_[(2 * k) as usize] as opus_int32);
        S[3] = silk_SMLAWB(S[3], B_Q28[2], in_[(2 * k + 1) as usize] as opus_int32);

        /* Scale back to Q0 and saturate */
        out[(2 * k) as usize] =
            silk_SAT16(silk_RSHIFT(out32_Q14[0].wrapping_add((1 << 14) - 1), 14)) as opus_int16;
        out[(2 * k + 1) as usize] =
            silk_SAT16(silk_RSHIFT(out32_Q14[1].wrapping_add((1 << 14) - 1), 14)) as opus_int16;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_coefficients_give_zero_output() {
        let b = [0i32; 3];
        let a = [0i32; 2];
        let mut s = [0i32; 2];
        let mut out = [0i16; 8];
        silk_biquad_alt_stride1(&[0i16; 8], &b, &a, &mut s, &mut out, 8);
        assert_eq!(out, [0i16; 8]);
        assert_eq!(s, [0, 0]);
    }

    #[test]
    fn stride2_matches_two_stride1_calls() {
        let b = [1i32 << 10, -(1 << 9), 1 << 8];
        let a = [1i32 << 12, -(1 << 11)];
        let mut in_ = [0i16; 16];
        for (i, v) in in_.iter_mut().enumerate() {
            *v = (i as i16) * 100 - 500;
        }

        let mut s2 = [0i32; 4];
        let mut out2 = [0i16; 16];
        silk_biquad_alt_stride2_c(&in_, &b, &a, &mut s2, &mut out2, 8);

        let mut s1 = [0i32; 4];
        let mut out1 = [0i16; 16];
        let mut even = [0i16; 8];
        let mut odd = [0i16; 8];
        for i in 0..8 {
            even[i] = in_[2 * i];
            odd[i] = in_[2 * i + 1];
        }
        silk_biquad_alt_stride1(&even, &b, &a, &mut s1[0..2], &mut out1[0..8], 8);
        silk_biquad_alt_stride1(&odd, &b, &a, &mut s1[2..4], &mut out1[8..16], 8);

        for i in 0..8 {
            assert_eq!(out2[2 * i], out1[i]);
            assert_eq!(out2[2 * i + 1], out1[8 + i]);
        }
        assert_eq!(s2[0], s1[0]);
        assert_eq!(s2[1], s1[1]);
        assert_eq!(s2[2], s1[2]);
        assert_eq!(s2[3], s1[3]);
    }
}

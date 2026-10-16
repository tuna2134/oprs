//! Mirror of `silk/LPC_analysis_filter.c` from libopus 1.6.1, transliterated to safe Rust.
//!
//! The C function takes an output pointer that may be an offset into a larger
//! buffer (`&sLTP[start_idx]`); the Rust port takes the whole slice and indexes
//! it exactly as the C does.
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
use crate::silk::sigproc_fix::*;
use crate::silk::typedef::*;
use crate::types::*;

/*******************************************/
/* LPC analysis filter                     */
/* NB! State is kept internally and the    */
/* filter always starts with zero state    */
/* first d output samples are set to zero  */
/*******************************************/

/* `USE_CELT_FIR` is 0 in the reference build, so only the plain loop below is
   compiled.  The `FIXED_POINT && USE_CELT_FIR` branch is dead and dropped. */

pub fn silk_LPC_analysis_filter(
    out: &mut [opus_int16],       /* O    Output signal                                               */
    in_: &[opus_int16],           /* I    Input signal                                                */
    B: &[opus_int16],             /* I    MA prediction coefficients, Q12 [order]                     */
    len: opus_int32,              /* I    Signal length                                               */
    d: opus_int32,                /* I    Filter order                                                */
    arch: opus_int,               /* I    Run-time architecture                                       */
) {
    celt_assert!(d >= 6);
    celt_assert!((d & 1) == 0);
    celt_assert!(d <= len);

    let _ = arch;

    for ix in d..len {
        let in_ptr = (ix - 1) as usize;

        let mut out32_Q12 = silk_SMULBB(in_[in_ptr] as opus_int32, B[0] as opus_int32);
        /* Allowing wrap around so that two wraps can cancel each other. The rare
           cases where the result wraps around can only be triggered by invalid streams*/
        out32_Q12 = silk_SMLABB_ovflw(out32_Q12, in_[in_ptr - 1] as opus_int32, B[1] as opus_int32);
        out32_Q12 = silk_SMLABB_ovflw(out32_Q12, in_[in_ptr - 2] as opus_int32, B[2] as opus_int32);
        out32_Q12 = silk_SMLABB_ovflw(out32_Q12, in_[in_ptr - 3] as opus_int32, B[3] as opus_int32);
        out32_Q12 = silk_SMLABB_ovflw(out32_Q12, in_[in_ptr - 4] as opus_int32, B[4] as opus_int32);
        out32_Q12 = silk_SMLABB_ovflw(out32_Q12, in_[in_ptr - 5] as opus_int32, B[5] as opus_int32);
        let mut j: opus_int32 = 6;
        while j < d {
            out32_Q12 = silk_SMLABB_ovflw(
                out32_Q12,
                in_[in_ptr - j as usize] as opus_int32,
                B[j as usize] as opus_int32,
            );
            out32_Q12 = silk_SMLABB_ovflw(
                out32_Q12,
                in_[in_ptr - j as usize - 1] as opus_int32,
                B[j as usize + 1] as opus_int32,
            );
            j += 2;
        }

        /* Subtract prediction */
        out32_Q12 = silk_SUB32_ovflw(silk_LSHIFT(in_[in_ptr + 1] as opus_int32, 12), out32_Q12);

        /* Scale to Q0 */
        let out32 = silk_RSHIFT_ROUND(out32_Q12, 12);

        /* Saturate output */
        out[ix as usize] = silk_SAT16(out32) as opus_int16;
    }

    /* Set first d output samples to zero */
    for i in 0..d as usize {
        out[i] = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// With d = 6 and B = [1,0,0,0,0,0] the filter computes
    /// `out[i] = in[i] - in[i-1]` (Q12/Q0 exact), a hand-computable identity.
    #[test]
    fn unit_first_difference() {
        let mut b = [0i16; 6];
        b[0] = 1 << 12; /* Q12 for 1.0 */
        let in_ = [0i16, 100, 300, 700, 1500, 3100, 6300];
        let mut out = [0i16; 7];
        silk_LPC_analysis_filter(&mut out, &in_, &b, 7, 6, 4);
        assert_eq!(&out[..6], &[0, 0, 0, 0, 0, 0]);
        /* in[6] - in[5] = 6300 - 3100 = 3200 */
        assert_eq!(out[6], 3200);
    }

    #[test]
    fn zero_coefficients_pass_through() {
        let b = [0i16; 10];
        let in_: Vec<i16> = (0..40).map(|i| (i * 37 - 500) as i16).collect();
        let mut out = vec![0i16; 40];
        silk_LPC_analysis_filter(&mut out, &in_, &b, 40, 10, 4);
        for i in 0..10 {
            assert_eq!(out[i], 0);
        }
        for i in 10..40 {
            assert_eq!(out[i], in_[i]);
        }
    }
}

//! Mirror of `silk/NLSF_VQ.c` from libopus 1.6.1, transliterated to safe Rust.
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
use crate::silk::typedef::silk_assert;
use crate::silk::typedef::*;
use crate::types::*;

/* Compute quantization errors for an LPC_order element input vector for a VQ codebook */
pub fn silk_NLSF_VQ(
    err_Q24: &mut [opus_int32],  /* O    Quantization errors [K]                     */
    in_Q15: &[opus_int16],       /* I    Input vectors to be quantized [LPC_order]   */
    pCB_Q8: &[opus_uint8],       /* I    Codebook vectors [K*LPC_order]              */
    pWght_Q9: &[opus_int16],     /* I    Codebook weights [K*LPC_order]              */
    K: opus_int,                 /* I    Number of codebook vectors                  */
    LPC_order: opus_int,         /* I    Number of LPCs                              */
) {
    celt_assert!((LPC_order & 1) == 0);

    /* Loop over codebook */
    for i in 0..K {
        let mut sum_error_Q24: opus_int32 = 0;
        let mut pred_Q24: opus_int32 = 0;
        let cb_Q8_ptr = &pCB_Q8[(i * LPC_order) as usize..];
        let w_Q9_ptr = &pWght_Q9[(i * LPC_order) as usize..];
        let mut m = LPC_order - 2;
        while m >= 0 {
            /* Compute weighted absolute predictive quantization error for index m + 1 */
            let diff_Q15 = silk_SUB_LSHIFT32(
                in_Q15[(m + 1) as usize] as opus_int32,
                cb_Q8_ptr[(m + 1) as usize] as opus_int32,
                7,
            ); /* range: [ -32767 : 32767 ]*/
            let diffw_Q24 = silk_SMULBB(diff_Q15, w_Q9_ptr[(m + 1) as usize] as opus_int32);
            sum_error_Q24 = silk_ADD32(
                sum_error_Q24,
                silk_abs(silk_SUB_RSHIFT32(diffw_Q24, pred_Q24, 1)),
            );
            pred_Q24 = diffw_Q24;

            /* Compute weighted absolute predictive quantization error for index m */
            let diff_Q15 = silk_SUB_LSHIFT32(
                in_Q15[m as usize] as opus_int32,
                cb_Q8_ptr[m as usize] as opus_int32,
                7,
            ); /* range: [ -32767 : 32767 ]*/
            let diffw_Q24 = silk_SMULBB(diff_Q15, w_Q9_ptr[m as usize] as opus_int32);
            sum_error_Q24 =
                silk_ADD32(sum_error_Q24, silk_abs(silk_SUB_RSHIFT32(diffw_Q24, pred_Q24, 1)));
            pred_Q24 = diffw_Q24;

            silk_assert!(sum_error_Q24 >= 0);

            m -= 2;
        }
        err_Q24[i as usize] = sum_error_Q24;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// With a single codebook vector equal to the input the error is zero.
    #[test]
    fn exact_match_has_zero_error() {
        let in_q15 = [128i16, 256, 384, 512, 640, 768];
        let cb2: Vec<u8> = vec![1, 2, 3, 4, 5, 6]; /* Q8 representation of in_q15 */
        let wght = [100i16; 6];
        let mut err = [0i32; 1];
        silk_NLSF_VQ(&mut err, &in_q15, &cb2, &wght, 1, 6);
        assert_eq!(err[0], 0);
    }

    /// A one-count difference in the last coefficient contributes exactly
    /// `w^2`-free `abs(diff * w)` for the first (predictor-free) term.
    #[test]
    fn single_coefficient_error() {
        let in_q15 = [0i16; 4];
        let cb = [0u8; 4];
        let mut wght = [0i16; 4];
        wght[1] = 10;
        let mut err = [0i32; 1];
        /* Only coefficient 1 differs, by (128 << 7) - expect 0 since cb is 0
           and in is 0; then bump in[1] by one Q15 unit. */
        silk_NLSF_VQ(&mut err, &in_q15, &cb, &wght, 1, 4);
        assert_eq!(err[0], 0);
        let in_q15 = [0i16, 128, 0, 0];
        silk_NLSF_VQ(&mut err, &in_q15, &cb, &wght, 1, 4);
        /* diff_Q15 = 128 - (0 << 7) = 128; diffw = 128 * 10 = 1280;
           the predictor chain then adds abs(0 - (1280 >> 1)) = 640. */
        assert_eq!(err[0], 1920);
    }
}

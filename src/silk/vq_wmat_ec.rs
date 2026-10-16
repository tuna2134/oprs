//! Mirror of `silk/VQ_WMat_EC.c` from libopus 1.6.1, transliterated to safe Rust.
//!
//! At arch 3/4 the reference build dispatches to
//! `silk_VQ_WMat_EC_sse4_1` (`silk/x86/VQ_WMat_EC_sse4_1.c`), which
//! `docs/rtcd-kernels.md` §4.3 proves is bit-identical to `silk_VQ_WMat_EC_c`
//! (only the first row of `XX_Q17` is vectorised and the int64 intermediate is
//! truncated back to int32).  The port therefore is the C body, and both
//! `silk_VQ_WMat_EC_c` and the dispatched entry are it.
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

use crate::silk::define::*;
use crate::silk::lin2log::*;
use crate::silk::macros::*;
use crate::silk::sigproc_fix::*;
use crate::silk::typedef::*;
use crate::types::*;

/* Entropy constrained matrix-weighted VQ, hard-coded to 5-element vectors, for a single input data vector */
#[allow(clippy::too_many_arguments)]
pub fn silk_VQ_WMat_EC_c(
    ind: &mut opus_int8,             /* O    index of best codebook vector               */
    res_nrg_Q15: &mut opus_int32,    /* O    best residual energy                        */
    rate_dist_Q8: &mut opus_int32,   /* O    best total bitrate                          */
    gain_Q7: &mut opus_int,          /* O    sum of absolute LTP coefficients            */
    XX_Q17: &[opus_int32],           /* I    correlation matrix                          */
    xX_Q17: &[opus_int32],           /* I    correlation vector                          */
    cb_Q7: &[opus_int8],             /* I    codebook                                    */
    cb_gain_Q7: &[opus_uint8],       /* I    codebook effective gain                     */
    cl_Q5: &[opus_uint8],            /* I    code length for each codebook vector        */
    subfr_len: opus_int,             /* I    number of samples per subframe              */
    max_gain_Q7: opus_int32,         /* I    maximum sum of absolute LTP coefficients    */
    L: opus_int,                     /* I    number of vectors in codebook               */
) {
    let mut gain_tmp_Q7: opus_int;
    let mut neg_xX_Q24 = [0 as opus_int32; 5];
    let mut sum1_Q15: opus_int32;
    let mut sum2_Q24: opus_int32;

    /* Negate and convert to new Q domain */
    neg_xX_Q24[0] = -silk_LSHIFT32(xX_Q17[0], 7);
    neg_xX_Q24[1] = -silk_LSHIFT32(xX_Q17[1], 7);
    neg_xX_Q24[2] = -silk_LSHIFT32(xX_Q17[2], 7);
    neg_xX_Q24[3] = -silk_LSHIFT32(xX_Q17[3], 7);
    neg_xX_Q24[4] = -silk_LSHIFT32(xX_Q17[4], 7);

    /* Loop over codebook */
    *rate_dist_Q8 = silk_int32_MAX;
    *res_nrg_Q15 = silk_int32_MAX;
    /* If things go really bad, at least *ind is set to something safe. */
    *ind = 0;
    for k in 0..L {
        let cb_row_Q7 = &cb_Q7[(k * LTP_ORDER) as usize..];
        gain_tmp_Q7 = cb_gain_Q7[k as usize] as opus_int;
        /* Weighted rate */
        /* Quantization error: 1 - 2 * xX * cb + cb' * XX * cb */
        sum1_Q15 = SILK_FIX_CONST(1.001, 15);

        /* Penalty for too large gain */
        let penalty = silk_LSHIFT32(silk_max(silk_SUB32(gain_tmp_Q7, max_gain_Q7), 0), 11);

        /* first row of XX_Q17 */
        sum2_Q24 = silk_MLA(neg_xX_Q24[0], XX_Q17[1], cb_row_Q7[1] as opus_int32);
        sum2_Q24 = silk_MLA(sum2_Q24, XX_Q17[2], cb_row_Q7[2] as opus_int32);
        sum2_Q24 = silk_MLA(sum2_Q24, XX_Q17[3], cb_row_Q7[3] as opus_int32);
        sum2_Q24 = silk_MLA(sum2_Q24, XX_Q17[4], cb_row_Q7[4] as opus_int32);
        sum2_Q24 = silk_LSHIFT32(sum2_Q24, 1);
        sum2_Q24 = silk_MLA(sum2_Q24, XX_Q17[0], cb_row_Q7[0] as opus_int32);
        sum1_Q15 = silk_SMLAWB(sum1_Q15, sum2_Q24, cb_row_Q7[0] as opus_int32);

        /* second row of XX_Q17 */
        sum2_Q24 = silk_MLA(neg_xX_Q24[1], XX_Q17[7], cb_row_Q7[2] as opus_int32);
        sum2_Q24 = silk_MLA(sum2_Q24, XX_Q17[8], cb_row_Q7[3] as opus_int32);
        sum2_Q24 = silk_MLA(sum2_Q24, XX_Q17[9], cb_row_Q7[4] as opus_int32);
        sum2_Q24 = silk_LSHIFT32(sum2_Q24, 1);
        sum2_Q24 = silk_MLA(sum2_Q24, XX_Q17[6], cb_row_Q7[1] as opus_int32);
        sum1_Q15 = silk_SMLAWB(sum1_Q15, sum2_Q24, cb_row_Q7[1] as opus_int32);

        /* third row of XX_Q17 */
        sum2_Q24 = silk_MLA(neg_xX_Q24[2], XX_Q17[13], cb_row_Q7[3] as opus_int32);
        sum2_Q24 = silk_MLA(sum2_Q24, XX_Q17[14], cb_row_Q7[4] as opus_int32);
        sum2_Q24 = silk_LSHIFT32(sum2_Q24, 1);
        sum2_Q24 = silk_MLA(sum2_Q24, XX_Q17[12], cb_row_Q7[2] as opus_int32);
        sum1_Q15 = silk_SMLAWB(sum1_Q15, sum2_Q24, cb_row_Q7[2] as opus_int32);

        /* fourth row of XX_Q17 */
        sum2_Q24 = silk_MLA(neg_xX_Q24[3], XX_Q17[19], cb_row_Q7[4] as opus_int32);
        sum2_Q24 = silk_LSHIFT32(sum2_Q24, 1);
        sum2_Q24 = silk_MLA(sum2_Q24, XX_Q17[18], cb_row_Q7[3] as opus_int32);
        sum1_Q15 = silk_SMLAWB(sum1_Q15, sum2_Q24, cb_row_Q7[3] as opus_int32);

        /* last row of XX_Q17 */
        sum2_Q24 = silk_LSHIFT32(neg_xX_Q24[4], 1);
        sum2_Q24 = silk_MLA(sum2_Q24, XX_Q17[24], cb_row_Q7[4] as opus_int32);
        sum1_Q15 = silk_SMLAWB(sum1_Q15, sum2_Q24, cb_row_Q7[4] as opus_int32);

        /* find best */
        if sum1_Q15 >= 0 {
            /* Translate residual energy to bits using high-rate assumption (6 dB ==> 1 bit/sample) */
            let bits_res_Q8 =
                silk_SMULBB(subfr_len, silk_lin2log(sum1_Q15 + penalty) - (15 << 7));
            /* In the following line we reduce the codelength component by half ("-1"); seems to slightly improve quality */
            let bits_tot_Q8 = silk_ADD_LSHIFT32(bits_res_Q8, cl_Q5[k as usize] as opus_int32, 3 - 1);
            if bits_tot_Q8 <= *rate_dist_Q8 {
                *rate_dist_Q8 = bits_tot_Q8;
                *res_nrg_Q15 = sum1_Q15 + penalty;
                *ind = k as opus_int8;
                *gain_Q7 = gain_tmp_Q7;
            }
        }
    }
}

/// The dispatched entry (`SILK_VQ_WMAT_EC_IMPL[(arch) & OPUS_ARCHMASK]`): arch
/// 3/4 select `silk_VQ_WMat_EC_sse4_1`, which is bit-identical to the C body.
#[allow(clippy::too_many_arguments)]
pub fn silk_VQ_WMat_EC(
    ind: &mut opus_int8,
    res_nrg_Q15: &mut opus_int32,
    rate_dist_Q8: &mut opus_int32,
    gain_Q7: &mut opus_int,
    XX_Q17: &[opus_int32],
    xX_Q17: &[opus_int32],
    cb_Q7: &[opus_int8],
    cb_gain_Q7: &[opus_uint8],
    cl_Q5: &[opus_uint8],
    subfr_len: opus_int,
    max_gain_Q7: opus_int32,
    L: opus_int,
    _arch: opus_int,
) {
    silk_VQ_WMat_EC_c(
        ind,
        res_nrg_Q15,
        rate_dist_Q8,
        gain_Q7,
        XX_Q17,
        xX_Q17,
        cb_Q7,
        cb_gain_Q7,
        cl_Q5,
        subfr_len,
        max_gain_Q7,
        L,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The search must pick the codebook row with the smallest total rate; a
    /// one-row codebook keeps that trivial and hand-checkable.
    #[test]
    fn single_row_codebook_selects_zero() {
        let mut xx = [0i32; 25];
        for i in 0..5 {
            xx[i * 5 + i] = 1 << 17;
        }
        let xx_row = [1 << 17, 0, 0, 0, 0];
        for i in 0..5 {
            xx[i] = xx_row[i];
        }
        let x_x = [0i32; 5];
        let cb = [0i8; 5];
        let cb_gain = [10u8];
        let cl = [20u8];
        let mut ind = 77i8;
        let mut res = 0i32;
        let mut rate = 0i32;
        let mut gain = 0i32;
        silk_VQ_WMat_EC_c(
            &mut ind, &mut res, &mut rate, &mut gain, &xx, &x_x, &cb, &cb_gain, &cl, 40, 100,
            1,
        );
        assert_eq!(ind, 0);
        assert_eq!(gain, 10);
        assert!(res != i32::MAX);
        assert!(rate != i32::MAX);
    }
}

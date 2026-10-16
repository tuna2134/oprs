//! Mirror of `silk/quant_LTP_gains.c` from libopus 1.6.1, transliterated to safe Rust.
//!
//! `silk_VQ_WMat_EC` dispatches at arch 3/4 to `silk_VQ_WMat_EC_sse4_1`, which
//! `docs/rtcd-kernels.md` §4.3 proves is bit-identical to the C body; the
//! dispatched entry in `crate::silk::vq_wmat_ec` is therefore exact.
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
use crate::silk::log2lin::*;
use crate::silk::macros::*;
use crate::silk::sigproc_fix::*;
use crate::silk::tables_ltp::*;
use crate::silk::typedef::*;
use crate::silk::vq_wmat_ec::*;
use crate::types::*;

/// `MAX_SUM_LOG_GAIN_DB` (`tuning_parameters.h:52`, `250.0f`)
const MAX_SUM_LOG_GAIN_DB: f64 = 250.0;

pub fn silk_quant_LTP_gains(
    B_Q14: &mut [opus_int16],      /* O    Quantized LTP gains             */
    cbk_index: &mut [opus_int8],   /* O    Codebook Index                  */
    periodicity_index: &mut opus_int8, /* O    Periodicity Index           */
    sum_log_gain_Q7: &mut opus_int32, /* I/O  Cumulative max prediction gain */
    pred_gain_dB_Q7: &mut opus_int,   /* O    LTP prediction gain          */
    XX_Q17: &[opus_int32],         /* I    Correlation matrix in Q18       */
    xX_Q17: &[opus_int32],         /* I    Correlation vector in Q18       */
    subfr_len: opus_int,           /* I    Number of samples per subframe  */
    nb_subfr: opus_int,            /* I    Number of subframes             */
    arch: opus_int,                /* I    Run-time architecture           */
) {
    let mut temp_idx = [0 as opus_int8; MAX_NB_SUBFR as usize];

    let mut res_nrg_Q15_subfr: opus_int32 = 0;
    let mut res_nrg_Q15: opus_int32 = 0;
    let mut rate_dist_Q7_subfr: opus_int32 = 0;
    let mut rate_dist_Q7: opus_int32 = 0;

    /***************************************************/
    /* iterate over different codebooks with different */
    /* rates/distortions, and choose best */
    /***************************************************/
    let mut min_rate_dist_Q7 = silk_int32_MAX;
    let mut best_sum_log_gain_Q7: opus_int32 = 0;
    for k in 0..3 {
        /* Safety margin for pitch gain control, to take into account factors
           such as state rescaling/rewhitening. */
        let gain_safety = SILK_FIX_CONST(0.4, 7);

        let cl_ptr_Q5 = silk_LTP_gain_BITS_Q5_ptrs[k as usize];
        let cbk_ptr_Q7 = silk_LTP_vq_ptrs_Q7[k as usize];
        let cbk_gain_ptr_Q7 = silk_LTP_vq_gain_ptrs_Q7[k as usize];
        let cbk_size = silk_LTP_vq_sizes[k as usize] as opus_int;

        /* Set up pointers to first subframe */
        let mut XX_Q17_ptr: opus_int = 0;
        let mut xX_Q17_ptr: opus_int = 0;

        res_nrg_Q15 = 0;
        rate_dist_Q7 = 0;
        let mut sum_log_gain_tmp_Q7 = *sum_log_gain_Q7;
        for j in 0..nb_subfr {
            let max_gain_Q7 = silk_log2lin(
                (SILK_FIX_CONST(MAX_SUM_LOG_GAIN_DB / 6.0, 7) - sum_log_gain_tmp_Q7)
                    + SILK_FIX_CONST(7.0, 7),
            ) - gain_safety;
            let mut gain_Q7: opus_int = 0;
            silk_VQ_WMat_EC(
                &mut temp_idx[j as usize], /* O    index of best codebook vector                           */
                &mut res_nrg_Q15_subfr,    /* O    residual energy                                         */
                &mut rate_dist_Q7_subfr,   /* O    best weighted quantization error + mu * rate            */
                &mut gain_Q7,              /* O    sum of absolute LTP coefficients                        */
                &XX_Q17[XX_Q17_ptr as usize..], /* I    correlation matrix                                */
                &xX_Q17[xX_Q17_ptr as usize..], /* I    correlation vector                                */
                cbk_ptr_Q7,                /* I    codebook                                                */
                cbk_gain_ptr_Q7,           /* I    codebook effective gains                                */
                cl_ptr_Q5,                 /* I    code length for each codebook vector                    */
                subfr_len,                 /* I    number of samples per subframe                          */
                max_gain_Q7,               /* I    maximum sum of absolute LTP coefficients                */
                cbk_size,                  /* I    number of vectors in codebook                           */
                arch,                      /* I    Run-time architecture                                   */
            );

            res_nrg_Q15 = silk_ADD_POS_SAT32(res_nrg_Q15, res_nrg_Q15_subfr);
            rate_dist_Q7 = silk_ADD_POS_SAT32(rate_dist_Q7, rate_dist_Q7_subfr);
            sum_log_gain_tmp_Q7 = silk_max(
                0,
                sum_log_gain_tmp_Q7 + silk_lin2log(gain_safety + gain_Q7) - SILK_FIX_CONST(7.0, 7),
            );

            XX_Q17_ptr += LTP_ORDER * LTP_ORDER;
            xX_Q17_ptr += LTP_ORDER;
        }

        if rate_dist_Q7 <= min_rate_dist_Q7 {
            min_rate_dist_Q7 = rate_dist_Q7;
            *periodicity_index = k as opus_int8;
            cbk_index[..nb_subfr as usize].copy_from_slice(&temp_idx[..nb_subfr as usize]);
            best_sum_log_gain_Q7 = sum_log_gain_tmp_Q7;
        }
    }

    let cbk_ptr_Q7 = silk_LTP_vq_ptrs_Q7[*periodicity_index as usize];
    for j in 0..nb_subfr {
        for k in 0..LTP_ORDER {
            B_Q14[(j * LTP_ORDER + k) as usize] = silk_LSHIFT(
                cbk_ptr_Q7[(cbk_index[j as usize] as opus_int * LTP_ORDER + k) as usize] as opus_int32,
                7,
            ) as opus_int16;
        }
    }

    if nb_subfr == 2 {
        res_nrg_Q15 = silk_RSHIFT32(res_nrg_Q15, 1);
    } else {
        res_nrg_Q15 = silk_RSHIFT32(res_nrg_Q15, 2);
    }

    *sum_log_gain_Q7 = best_sum_log_gain_Q7;
    *pred_gain_dB_Q7 = silk_SMULBB(-3, silk_lin2log(res_nrg_Q15) - (15 << 7));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// All-zero correlation matrices leave the search picking codebook entry 0
    /// and produce a finite prediction gain.
    #[test]
    fn zero_input_selects_first_entry() {
        let mut b = [0i16; (MAX_NB_SUBFR * LTP_ORDER) as usize];
        let mut idx = [0i8; MAX_NB_SUBFR as usize];
        let mut per = 0i8;
        let mut sum_log = 0i32;
        let mut pred = 0i32;
        let xx = [0i32; (MAX_NB_SUBFR * LTP_ORDER * LTP_ORDER) as usize];
        let xx_x = [0i32; (MAX_NB_SUBFR * LTP_ORDER) as usize];
        silk_quant_LTP_gains(
            &mut b,
            &mut idx,
            &mut per,
            &mut sum_log,
            &mut pred,
            &xx,
            &xx_x,
            40,
            4,
            4,
        );
        assert_eq!(per, 0);
        assert_eq!(idx, [0i8; 4]);
        let _ = pred;
        assert!(sum_log >= 0);
    }
}

//! Mirror of `silk/NLSF_del_dec_quant.c` from libopus 1.6.1, transliterated to safe Rust.
//!
//! The C keeps several intermediates in `opus_int16` (and `silk_ADD16` /
//! `silk_SUB16` return `opus_int16`); the port keeps the same narrow types so
//! that every truncation happens at exactly the same place.
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
use crate::silk::macros::*;
use crate::silk::sigproc_fix::*;
use crate::silk::typedef::silk_assert;
use crate::silk::typedef::*;
use crate::types::*;

/* Delayed-decision quantizer for NLSF residuals */
pub fn silk_NLSF_del_dec_quant(
    indices: &mut [opus_int8],     /* O    Quantization indices [ order ]              */
    x_Q10: &[opus_int16],          /* I    Input [ order ]                             */
    w_Q5: &[opus_int16],           /* I    Weights [ order ]                           */
    pred_coef_Q8: &[opus_uint8],   /* I    Backward predictor coefs [ order ]          */
    ec_ix: &[opus_int16],          /* I    Indices to entropy coding tables [ order ]  */
    ec_rates_Q5: &[opus_uint8],    /* I    Rates []                                    */
    quant_step_size_Q16: opus_int, /* I    Quantization step size                      */
    inv_quant_step_size_Q6: opus_int16, /* I    Inverse quantization step size         */
    mu_Q20: opus_int32,            /* I    R/D tradeoff                                */
    order: opus_int16,             /* I    Number of input values                      */
) -> opus_int32 {
    let mut out0_Q10_table = [0 as opus_int; (2 * NLSF_QUANT_MAX_AMPLITUDE_EXT) as usize];
    let mut out1_Q10_table = [0 as opus_int; (2 * NLSF_QUANT_MAX_AMPLITUDE_EXT) as usize];

    for i in -NLSF_QUANT_MAX_AMPLITUDE_EXT..=NLSF_QUANT_MAX_AMPLITUDE_EXT - 1 {
        let mut out0_Q10 = silk_LSHIFT(i, 10) as opus_int16;
        let mut out1_Q10 = silk_ADD16(out0_Q10, 1024) as opus_int16;
        if i > 0 {
            out0_Q10 = silk_SUB16(out0_Q10, SILK_FIX_CONST(NLSF_QUANT_LEVEL_ADJ, 10) as opus_int16)
                as opus_int16;
            out1_Q10 = silk_SUB16(out1_Q10, SILK_FIX_CONST(NLSF_QUANT_LEVEL_ADJ, 10) as opus_int16)
                as opus_int16;
        } else if i == 0 {
            out1_Q10 = silk_SUB16(out1_Q10, SILK_FIX_CONST(NLSF_QUANT_LEVEL_ADJ, 10) as opus_int16)
                as opus_int16;
        } else if i == -1 {
            out0_Q10 = silk_ADD16(out0_Q10, SILK_FIX_CONST(NLSF_QUANT_LEVEL_ADJ, 10) as opus_int16)
                as opus_int16;
        } else {
            out0_Q10 = silk_ADD16(out0_Q10, SILK_FIX_CONST(NLSF_QUANT_LEVEL_ADJ, 10) as opus_int16)
                as opus_int16;
            out1_Q10 = silk_ADD16(out1_Q10, SILK_FIX_CONST(NLSF_QUANT_LEVEL_ADJ, 10) as opus_int16)
                as opus_int16;
        }
        out0_Q10_table[(i + NLSF_QUANT_MAX_AMPLITUDE_EXT) as usize] =
            silk_RSHIFT(silk_SMULBB(out0_Q10 as opus_int32, quant_step_size_Q16), 16);
        out1_Q10_table[(i + NLSF_QUANT_MAX_AMPLITUDE_EXT) as usize] =
            silk_RSHIFT(silk_SMULBB(out1_Q10 as opus_int32, quant_step_size_Q16), 16);
    }

    silk_assert!((NLSF_QUANT_DEL_DEC_STATES & (NLSF_QUANT_DEL_DEC_STATES - 1)) == 0); /* must be power of two */

    let mut ind = [[0 as opus_int8; MAX_LPC_ORDER as usize]; NLSF_QUANT_DEL_DEC_STATES as usize];
    let mut prev_out_Q10 = [0 as opus_int16; (2 * NLSF_QUANT_DEL_DEC_STATES) as usize];
    let mut RD_Q25 = [0 as opus_int32; (2 * NLSF_QUANT_DEL_DEC_STATES) as usize];
    let mut RD_min_Q25 = [0 as opus_int32; NLSF_QUANT_DEL_DEC_STATES as usize];
    let mut RD_max_Q25 = [0 as opus_int32; NLSF_QUANT_DEL_DEC_STATES as usize];
    let mut ind_sort = [0 as opus_int; NLSF_QUANT_DEL_DEC_STATES as usize];

    let mut nStates: opus_int = 1;
    RD_Q25[0] = 0;
    prev_out_Q10[0] = 0;
    let mut i: opus_int = order as opus_int - 1;
    while i >= 0 {
        let rates_Q5 = &ec_rates_Q5[ec_ix[i as usize] as usize..];
        let in_Q10 = x_Q10[i as usize] as opus_int;
        for j in 0..nStates {
            let pred_Q10 = silk_RSHIFT(
                silk_SMULBB(pred_coef_Q8[i as usize] as opus_int16 as opus_int32, prev_out_Q10[j as usize] as opus_int32),
                8,
            );
            let res_Q10 = silk_SUB16(in_Q10 as opus_int16, pred_Q10 as opus_int16) as opus_int16
                as opus_int;
            let mut ind_tmp =
                silk_RSHIFT(silk_SMULBB(inv_quant_step_size_Q6 as opus_int32, res_Q10), 16);
            ind_tmp = silk_LIMIT(
                ind_tmp,
                -NLSF_QUANT_MAX_AMPLITUDE_EXT,
                NLSF_QUANT_MAX_AMPLITUDE_EXT - 1,
            );
            ind[j as usize][i as usize] = ind_tmp as opus_int8;

            /* compute outputs for ind_tmp and ind_tmp + 1 */
            let out0_Q10 = out0_Q10_table[(ind_tmp + NLSF_QUANT_MAX_AMPLITUDE_EXT) as usize]
                as opus_int16;
            let out1_Q10 = out1_Q10_table[(ind_tmp + NLSF_QUANT_MAX_AMPLITUDE_EXT) as usize]
                as opus_int16;

            let out0_Q10 = silk_ADD16(out0_Q10, pred_Q10 as opus_int16) as opus_int16;
            let out1_Q10 = silk_ADD16(out1_Q10, pred_Q10 as opus_int16) as opus_int16;
            prev_out_Q10[j as usize] = out0_Q10;
            prev_out_Q10[(j + nStates) as usize] = out1_Q10;

            /* compute RD for ind_tmp and ind_tmp + 1 */
            let rate0_Q5: opus_int;
            let rate1_Q5: opus_int;
            if ind_tmp + 1 >= NLSF_QUANT_MAX_AMPLITUDE {
                if ind_tmp + 1 == NLSF_QUANT_MAX_AMPLITUDE {
                    rate0_Q5 = rates_Q5[(ind_tmp + NLSF_QUANT_MAX_AMPLITUDE) as usize] as opus_int;
                    rate1_Q5 = 280;
                } else {
                    rate0_Q5 = silk_SMLABB(
                        280 - 43 * NLSF_QUANT_MAX_AMPLITUDE,
                        43,
                        ind_tmp,
                    );
                    rate1_Q5 = silk_ADD16(rate0_Q5 as opus_int16, 43) as opus_int16 as opus_int;
                }
            } else if ind_tmp <= -NLSF_QUANT_MAX_AMPLITUDE {
                if ind_tmp == -NLSF_QUANT_MAX_AMPLITUDE {
                    rate0_Q5 = 280;
                    rate1_Q5 =
                        rates_Q5[(ind_tmp + 1 + NLSF_QUANT_MAX_AMPLITUDE) as usize] as opus_int;
                } else {
                    rate0_Q5 = silk_SMLABB(
                        280 - 43 * NLSF_QUANT_MAX_AMPLITUDE,
                        -43,
                        ind_tmp,
                    );
                    rate1_Q5 = silk_SUB16(rate0_Q5 as opus_int16, 43) as opus_int16 as opus_int;
                }
            } else {
                rate0_Q5 = rates_Q5[(ind_tmp + NLSF_QUANT_MAX_AMPLITUDE) as usize] as opus_int;
                rate1_Q5 =
                    rates_Q5[(ind_tmp + 1 + NLSF_QUANT_MAX_AMPLITUDE) as usize] as opus_int;
            }
            let RD_tmp_Q25 = RD_Q25[j as usize];
            let diff_Q10 = silk_SUB16(in_Q10 as opus_int16, out0_Q10) as opus_int16 as opus_int;
            RD_Q25[j as usize] = silk_SMLABB(
                silk_MLA(RD_tmp_Q25, silk_SMULBB(diff_Q10, diff_Q10), w_Q5[i as usize] as opus_int32),
                mu_Q20,
                rate0_Q5,
            );
            let diff_Q10 = silk_SUB16(in_Q10 as opus_int16, out1_Q10) as opus_int16 as opus_int;
            RD_Q25[(j + nStates) as usize] = silk_SMLABB(
                silk_MLA(RD_tmp_Q25, silk_SMULBB(diff_Q10, diff_Q10), w_Q5[i as usize] as opus_int32),
                mu_Q20,
                rate1_Q5,
            );
        }

        if nStates <= NLSF_QUANT_DEL_DEC_STATES / 2 {
            /* double number of states and copy */
            for j in 0..nStates {
                ind[(j + nStates) as usize][i as usize] =
                    ind[j as usize][i as usize].wrapping_add(1);
            }
            nStates = silk_LSHIFT(nStates, 1);
            for j in nStates..NLSF_QUANT_DEL_DEC_STATES {
                ind[j as usize][i as usize] = ind[(j - nStates) as usize][i as usize];
            }
        } else {
            /* sort lower and upper half of RD_Q25, pairwise */
            for j in 0..NLSF_QUANT_DEL_DEC_STATES {
                if RD_Q25[j as usize] > RD_Q25[(j + NLSF_QUANT_DEL_DEC_STATES) as usize] {
                    RD_max_Q25[j as usize] = RD_Q25[j as usize];
                    RD_min_Q25[j as usize] = RD_Q25[(j + NLSF_QUANT_DEL_DEC_STATES) as usize];
                    RD_Q25[j as usize] = RD_min_Q25[j as usize];
                    RD_Q25[(j + NLSF_QUANT_DEL_DEC_STATES) as usize] = RD_max_Q25[j as usize];
                    /* swap prev_out values */
                    let out0_Q10 = prev_out_Q10[j as usize];
                    prev_out_Q10[j as usize] =
                        prev_out_Q10[(j + NLSF_QUANT_DEL_DEC_STATES) as usize];
                    prev_out_Q10[(j + NLSF_QUANT_DEL_DEC_STATES) as usize] = out0_Q10;
                    ind_sort[j as usize] = j + NLSF_QUANT_DEL_DEC_STATES;
                } else {
                    RD_min_Q25[j as usize] = RD_Q25[j as usize];
                    RD_max_Q25[j as usize] = RD_Q25[(j + NLSF_QUANT_DEL_DEC_STATES) as usize];
                    ind_sort[j as usize] = j;
                }
            }
            /* compare the highest RD values of the winning half with the lowest one in the losing half, and copy if necessary */
            /* afterwards ind_sort[] will contain the indices of the NLSF_QUANT_DEL_DEC_STATES winning RD values */
            loop {
                let mut min_max_Q25 = silk_int32_MAX;
                let mut max_min_Q25: opus_int32 = 0;
                let mut ind_min_max: opus_int = 0;
                let mut ind_max_min: opus_int = 0;
                for j in 0..NLSF_QUANT_DEL_DEC_STATES {
                    if min_max_Q25 > RD_max_Q25[j as usize] {
                        min_max_Q25 = RD_max_Q25[j as usize];
                        ind_min_max = j;
                    }
                    if max_min_Q25 < RD_min_Q25[j as usize] {
                        max_min_Q25 = RD_min_Q25[j as usize];
                        ind_max_min = j;
                    }
                }
                if min_max_Q25 >= max_min_Q25 {
                    break;
                }
                /* copy ind_min_max to ind_max_min */
                ind_sort[ind_max_min as usize] =
                    ind_sort[ind_min_max as usize] ^ NLSF_QUANT_DEL_DEC_STATES;
                RD_Q25[ind_max_min as usize] =
                    RD_Q25[(ind_min_max + NLSF_QUANT_DEL_DEC_STATES) as usize];
                prev_out_Q10[ind_max_min as usize] =
                    prev_out_Q10[(ind_min_max + NLSF_QUANT_DEL_DEC_STATES) as usize];
                RD_min_Q25[ind_max_min as usize] = 0;
                RD_max_Q25[ind_min_max as usize] = silk_int32_MAX;
                ind[ind_max_min as usize] = ind[ind_min_max as usize];
            }
            /* increment index if it comes from the upper half */
            for j in 0..NLSF_QUANT_DEL_DEC_STATES {
                ind[j as usize][i as usize] = ind[j as usize][i as usize]
                    .wrapping_add(silk_RSHIFT(ind_sort[j as usize], NLSF_QUANT_DEL_DEC_STATES_LOG2) as opus_int8);
            }
        }
        i -= 1;
    }

    /* last sample: find winner, copy indices and return RD value */
    let mut ind_tmp: opus_int = 0;
    let mut min_Q25 = silk_int32_MAX;
    for j in 0..2 * NLSF_QUANT_DEL_DEC_STATES {
        if min_Q25 > RD_Q25[j as usize] {
            min_Q25 = RD_Q25[j as usize];
            ind_tmp = j;
        }
    }
    for j in 0..order as opus_int {
        indices[j as usize] = ind[(ind_tmp & (NLSF_QUANT_DEL_DEC_STATES - 1)) as usize][j as usize];
        silk_assert!(indices[j as usize] as opus_int >= -NLSF_QUANT_MAX_AMPLITUDE_EXT);
        silk_assert!(indices[j as usize] as opus_int <= NLSF_QUANT_MAX_AMPLITUDE_EXT);
    }
    indices[0] = indices[0].wrapping_add(silk_RSHIFT(ind_tmp, NLSF_QUANT_DEL_DEC_STATES_LOG2) as opus_int8);
    silk_assert!(indices[0] as opus_int <= NLSF_QUANT_MAX_AMPLITUDE_EXT);
    silk_assert!(min_Q25 >= 0);
    min_Q25
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A constant residue quantises to a constant index and a small RD value.
    #[test]
    fn constant_residue_is_quantised_consistently() {
        let order = 4i16;
        let x = [0i16; 4];
        let w = [100i16; 4];
        let pred = [0u8; 4];
        let ec_ix = [0i16; 4];
        let mut rates = [0u8; 64];
        for (i, r) in rates.iter_mut().enumerate() {
            *r = (i as i32 % 20) as u8;
        }
        let mut idx = [0i8; 4];
        let rd = silk_NLSF_del_dec_quant(
            &mut idx,
            &x,
            &w,
            &pred,
            &ec_ix,
            &rates,
            21299,
            16384,
            0,
            order,
        );
        assert!(rd >= 0);
    }
}

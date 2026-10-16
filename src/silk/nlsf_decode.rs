//! Mirror of `silk/NLSF_decode.c` from libopus 1.6.1, transliterated to safe Rust.
//!
//! Predictive dequantizer for NLSF residuals and the NLSF vector decoder.
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
use crate::silk::nlsf_stabilize::*;
use crate::silk::nlsf_unpack::*;
use crate::silk::sigproc_fix::*;
use crate::silk::structs::*;
use crate::silk::typedef::*;
use crate::types::*;

/* Predictive dequantizer for NLSF residuals */
fn silk_NLSF_residual_dequant(
    /* O    Returns RD value in Q30 */
    x_Q10: &mut [opus_int16],                 /* O    Output [ order ]           */
    indices: &[opus_int8],                    /* I    Quantization indices [ order ] */
    pred_coef_Q8: &[opus_uint8],              /* I    Backward predictor coefs [ order ] */
    quant_step_size_Q16: opus_int,            /* I    Quantization step size     */
    order: opus_int16,                        /* I    Number of input values     */
) {
    let mut i: opus_int;
    let mut out_Q10: opus_int;
    let mut pred_Q10: opus_int;

    out_Q10 = 0;
    i = order as opus_int - 1;
    while i >= 0 {
        pred_Q10 = silk_RSHIFT(
            silk_SMULBB(out_Q10, pred_coef_Q8[i as usize] as opus_int16 as opus_int32),
            8,
        );
        out_Q10 = silk_LSHIFT(indices[i as usize] as opus_int, 10);
        if out_Q10 > 0 {
            /* silk_SUB16(out_Q10, SILK_FIX_CONST(NLSF_QUANT_LEVEL_ADJ, 10)) */
            out_Q10 -= SILK_FIX_CONST(NLSF_QUANT_LEVEL_ADJ, 10);
        } else if out_Q10 < 0 {
            /* silk_ADD16(out_Q10, SILK_FIX_CONST(NLSF_QUANT_LEVEL_ADJ, 10)) */
            out_Q10 += SILK_FIX_CONST(NLSF_QUANT_LEVEL_ADJ, 10);
        }
        out_Q10 = silk_SMLAWB(pred_Q10, out_Q10 as opus_int32, quant_step_size_Q16);
        x_Q10[i as usize] = out_Q10 as opus_int16;
        i -= 1;
    }
}

/***********************/
/* NLSF vector decoder */
/***********************/
/// `silk_NLSF_decode(pNLSF_Q15, NLSFIndices, psNLSF_CB)`
pub fn silk_NLSF_decode(
    pNLSF_Q15: &mut [opus_int16],              /* O    Quantized NLSF vector [ LPC_ORDER ] */
    NLSFIndices: &[opus_int8],                 /* I    Codebook path vector [ LPC_ORDER + 1 ] */
    psNLSF_CB: &silk_NLSF_CB_struct,           /* I    Codebook object */
) {
    let order = psNLSF_CB.order as opus_int;
    let mut pred_Q8 = [0 as opus_uint8; MAX_LPC_ORDER as usize];
    let mut ec_ix = [0 as opus_int16; MAX_LPC_ORDER as usize];
    let mut res_Q10 = [0 as opus_int16; MAX_LPC_ORDER as usize];
    let mut NLSF_Q15_tmp: opus_int32;

    /* Unpack entropy table indices and predictor for current CB1 index */
    silk_NLSF_unpack(&mut ec_ix, &mut pred_Q8, psNLSF_CB, NLSFIndices[0] as opus_int);

    /* Predictive residual dequantizer */
    silk_NLSF_residual_dequant(
        &mut res_Q10,
        &NLSFIndices[1..],
        &pred_Q8,
        psNLSF_CB.quantStepSize_Q16 as opus_int,
        psNLSF_CB.order,
    );

    /* Apply inverse square-rooted weights to first stage and add to output */
    let cb1_off = (NLSFIndices[0] as opus_int * order) as usize;
    for i in 0..order {
        NLSF_Q15_tmp = silk_ADD_LSHIFT32(
            silk_DIV32_16(
                silk_LSHIFT(res_Q10[i as usize] as opus_int32, 14),
                psNLSF_CB.CB1_Wght_Q9[cb1_off + i as usize] as opus_int32,
            ),
            psNLSF_CB.CB1_NLSF_Q8[cb1_off + i as usize] as opus_int16 as opus_int32,
            7,
        );
        pNLSF_Q15[i as usize] = silk_LIMIT(NLSF_Q15_tmp, 0, 32767) as opus_int16;
    }

    /* NLSF stabilization */
    silk_NLSF_stabilize(pNLSF_Q15, psNLSF_CB.deltaMin_Q15, order);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::silk::tables_nlsf_cb_wb::silk_NLSF_CB_WB;

    #[test]
    fn output_is_sorted_and_in_range() {
        let cb = &silk_NLSF_CB_WB;
        let order = cb.order as usize;
        let mut idx = [0i8; (MAX_LPC_ORDER + 1) as usize];
        idx[0] = 3;
        for i in 1..=order {
            idx[i] = 1;
        }
        let mut out = [0i16; MAX_LPC_ORDER as usize];
        silk_NLSF_decode(&mut out, &idx, cb);
        for i in 1..order {
            assert!(out[i] >= out[i - 1]);
        }
        assert!(out[order - 1] >= 0);
    }
}

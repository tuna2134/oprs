//! Mirror of `silk/NLSF_encode.c` from libopus 1.6.1, transliterated to safe Rust.
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
use crate::silk::define::*;
use crate::silk::inlines::*;
use crate::silk::lin2log::*;
use crate::silk::macros::*;
use crate::silk::nlsf_decode::*;
use crate::silk::nlsf_del_dec_quant::*;
use crate::silk::nlsf_stabilize::*;
use crate::silk::nlsf_unpack::*;
use crate::silk::nlsf_vq::*;
use crate::silk::sigproc_fix::*;
use crate::silk::sort::*;
use crate::silk::structs::*;
use crate::silk::typedef::silk_assert;
use crate::silk::typedef::*;
use crate::types::*;

/***********************/
/* NLSF vector encoder */
/***********************/
pub fn silk_NLSF_encode(
    NLSFIndices: &mut [opus_int8],       /* I    Codebook path vector [ LPC_ORDER + 1 ]      */
    pNLSF_Q15: &mut [opus_int16],        /* I/O  (Un)quantized NLSF vector [ LPC_ORDER ]     */
    psNLSF_CB: &silk_NLSF_CB_struct,     /* I    Codebook object                             */
    pW_QW: &[opus_int16],                /* I    NLSF weight vector [ LPC_ORDER ]            */
    NLSF_mu_Q20: opus_int,               /* I    Rate weight for the RD optimization         */
    nSurvivors: opus_int,                /* I    Max survivors after first stage             */
    signalType: opus_int,                /* I    Signal type: 0/1/2                          */
) -> opus_int32 {
    celt_assert!(signalType >= 0 && signalType <= 2);
    silk_assert!(NLSF_mu_Q20 <= 32767 && NLSF_mu_Q20 >= 0);

    let order = psNLSF_CB.order as opus_int;

    /* NLSF stabilization */
    silk_NLSF_stabilize(pNLSF_Q15, psNLSF_CB.deltaMin_Q15, order);

    /* First stage: VQ */
    let mut err_Q24 = vec![0 as opus_int32; psNLSF_CB.nVectors as usize];
    silk_NLSF_VQ(
        &mut err_Q24,
        pNLSF_Q15,
        psNLSF_CB.CB1_NLSF_Q8,
        psNLSF_CB.CB1_Wght_Q9,
        psNLSF_CB.nVectors as opus_int,
        order,
    );

    /* Sort the quantization errors */
    let mut tempIndices1 = vec![0 as opus_int; nSurvivors as usize];
    silk_insertion_sort_increasing(
        &mut err_Q24,
        &mut tempIndices1,
        psNLSF_CB.nVectors as opus_int,
        nSurvivors,
    );

    let mut RD_Q25 = vec![0 as opus_int32; nSurvivors as usize];
    let mut tempIndices2 = vec![0 as opus_int8; nSurvivors as usize * MAX_LPC_ORDER as usize];

    let mut res_Q10 = [0 as opus_int16; MAX_LPC_ORDER as usize];
    let mut NLSF_tmp_Q15 = [0 as opus_int16; MAX_LPC_ORDER as usize];
    let mut W_adj_Q5 = [0 as opus_int16; MAX_LPC_ORDER as usize];
    let mut pred_Q8 = [0 as opus_uint8; MAX_LPC_ORDER as usize];
    let mut ec_ix = [0 as opus_int16; MAX_LPC_ORDER as usize];

    /* Loop over survivors */
    for s in 0..nSurvivors {
        let ind1 = tempIndices1[s as usize];

        /* Residual after first stage */
        let pCB_element = &psNLSF_CB.CB1_NLSF_Q8[(ind1 * order) as usize..];
        let pCB_Wght_Q9 = &psNLSF_CB.CB1_Wght_Q9[(ind1 * order) as usize..];
        for i in 0..order {
            NLSF_tmp_Q15[i as usize] = silk_LSHIFT16(pCB_element[i as usize] as opus_int16, 7);
            let W_tmp_Q9 = pCB_Wght_Q9[i as usize] as opus_int32;
            res_Q10[i as usize] = silk_RSHIFT(
                silk_SMULBB(
                    pNLSF_Q15[i as usize] as opus_int32 - NLSF_tmp_Q15[i as usize] as opus_int32,
                    W_tmp_Q9,
                ),
                14,
            ) as opus_int16;
            W_adj_Q5[i as usize] = silk_DIV32_varQ(
                pW_QW[i as usize] as opus_int32,
                silk_SMULBB(W_tmp_Q9, W_tmp_Q9),
                21,
            ) as opus_int16;
        }

        /* Unpack entropy table indices and predictor for current CB1 index */
        silk_NLSF_unpack(&mut ec_ix, &mut pred_Q8, psNLSF_CB, ind1);

        /* Trellis quantizer */
        RD_Q25[s as usize] = silk_NLSF_del_dec_quant(
            &mut tempIndices2[(s * MAX_LPC_ORDER) as usize..],
            &res_Q10,
            &W_adj_Q5,
            &pred_Q8,
            &ec_ix,
            psNLSF_CB.ec_Rates_Q5,
            psNLSF_CB.quantStepSize_Q16 as opus_int,
            psNLSF_CB.invQuantStepSize_Q6,
            NLSF_mu_Q20 as opus_int32,
            psNLSF_CB.order,
        );

        /* Add rate for first stage */
        let iCDF_ptr =
            &psNLSF_CB.CB1_iCDF[((signalType >> 1) * psNLSF_CB.nVectors as opus_int) as usize..];
        let prob_Q8: opus_int;
        if ind1 == 0 {
            prob_Q8 = 256 - iCDF_ptr[ind1 as usize] as opus_int;
        } else {
            prob_Q8 =
                iCDF_ptr[(ind1 - 1) as usize] as opus_int - iCDF_ptr[ind1 as usize] as opus_int;
        }
        let bits_q7 = (8 << 7) - silk_lin2log(prob_Q8);
        RD_Q25[s as usize] = silk_SMLABB(RD_Q25[s as usize], bits_q7, silk_RSHIFT(NLSF_mu_Q20, 2));
    }

    /* Find the lowest rate-distortion error */
    let mut bestIndex = [0 as opus_int; 1];
    silk_insertion_sort_increasing(&mut RD_Q25, &mut bestIndex, nSurvivors, 1);

    NLSFIndices[0] = tempIndices1[bestIndex[0] as usize] as opus_int8;
    NLSFIndices[1..1 + order as usize].copy_from_slice(
        &tempIndices2[(bestIndex[0] * MAX_LPC_ORDER) as usize
            ..(bestIndex[0] * MAX_LPC_ORDER + order) as usize],
    );

    /* Decode */
    silk_NLSF_decode(pNLSF_Q15, NLSFIndices, psNLSF_CB);

    RD_Q25[0]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::silk::tables_nlsf_cb_nb_mb::silk_NLSF_CB_NB_MB;

    /// Encoding a stabilised NLSF vector yields indices that decode back to a
    /// stabilised (strictly increasing) vector, and a non-negative RD value.
    #[test]
    fn round_trip_is_stable() {
        let cb = &silk_NLSF_CB_NB_MB;
        let order = cb.order as usize;
        let mut nlsf = vec![0i16; order];
        for i in 0..order {
            nlsf[i] = ((i as i32 + 1) * (32768 / (order as i32 + 1))) as i16;
        }
        let mut w = vec![0i16; order];
        silk_NLSF_VQ_weights_laroia_import(&mut w, &nlsf);
        let mut indices = [0i8; (MAX_LPC_ORDER + 1) as usize];
        let rd = silk_NLSF_encode(&mut indices, &mut nlsf, cb, &w, 1000, 8, 0);
        assert!(rd >= 0);
        for i in 1..order {
            assert!(nlsf[i] >= nlsf[i - 1]);
        }
    }

    fn silk_NLSF_VQ_weights_laroia_import(w: &mut [i16], nlsf: &[i16]) {
        crate::silk::nlsf_vq_weights_laroia::silk_NLSF_VQ_weights_laroia(w, nlsf, nlsf.len() as i32);
    }
}

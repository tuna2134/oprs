//! Transliteration of `silk/decode_parameters.c` from libopus 1.6.1 to safe Rust.
//!
//! Original C file copyright notice (this is a derived work):
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

use crate::silk::bwexpander::silk_bwexpander;
use crate::silk::decode_pitch::silk_decode_pitch;
use crate::silk::define::*;
use crate::silk::gain_quant::silk_gains_dequant;
use crate::silk::nlsf2a::silk_NLSF2A;
use crate::silk::nlsf_decode::silk_NLSF_decode;
use crate::silk::sigproc_fix::*;
use crate::silk::structs::*;
use crate::silk::tables_ltp::silk_LTP_vq_ptrs_Q7;
use crate::silk::tables_other::silk_LTPScales_table_Q14;
use crate::silk::typedef::*;
use crate::types::*;

/* Decode parameters from payload */
/// `silk_decode_parameters(psDec, psDecCtrl, condCoding)`
pub fn silk_decode_parameters(
    psDec: &mut silk_decoder_state,      /* I/O  State           */
    psDecCtrl: &mut silk_decoder_control, /* I/O  Decoder control */
    condCoding: opus_int,                /* I    The type of conditional coding to use */
) {
    let mut i: opus_int;
    let mut k: opus_int;
    let mut Ix: opus_int;
    let mut pNLSF_Q15 = [0 as opus_int16; MAX_LPC_ORDER as usize];
    let mut pNLSF0_Q15 = [0 as opus_int16; MAX_LPC_ORDER as usize];

    /* Dequant Gains */
    let conditional = (condCoding == CODE_CONDITIONALLY) as opus_int;
    let nb_subfr = psDec.nb_subfr;
    silk_gains_dequant(
        &mut psDecCtrl.Gains_Q16,
        &psDec.indices.GainsIndices,
        &mut psDec.LastGainIndex,
        conditional,
        nb_subfr,
    );

    /****************/
    /* Decode NLSFs */
    /****************/
    let psNLSF_CB = psDec.psNLSF_CB;
    silk_NLSF_decode(
        &mut pNLSF_Q15,
        &psDec.indices.NLSFIndices,
        psNLSF_CB,
    );

    /* Convert NLSF parameters to AR prediction filter coefficients */
    silk_NLSF2A(
        &mut psDecCtrl.PredCoef_Q12[1],
        &pNLSF_Q15,
        psDec.LPC_order,
        psDec.arch,
    );

    /* If just reset, e.g., because internal Fs changed, do not allow interpolation */
    /* improves the case of packet loss in the first frame after a switch           */
    if psDec.first_frame_after_reset == 1 {
        psDec.indices.NLSFInterpCoef_Q2 = 4;
    }

    if (psDec.indices.NLSFInterpCoef_Q2 as opus_int) < 4 {
        /* Calculation of the interpolated NLSF0 vector from the interpolation factor, */
        /* the previous NLSF1, and the current NLSF1                                   */
        let interp = psDec.indices.NLSFInterpCoef_Q2 as opus_int32;
        for i in 0..psDec.LPC_order {
            pNLSF0_Q15[i as usize] = (psDec.prevNLSF_Q15[i as usize] as opus_int32
                + silk_RSHIFT(
                    silk_MUL(
                        interp,
                        pNLSF_Q15[i as usize] as opus_int32
                            - psDec.prevNLSF_Q15[i as usize] as opus_int32,
                    ),
                    2,
                )) as opus_int16;
        }

        /* Convert NLSF parameters to AR prediction filter coefficients */
        silk_NLSF2A(
            &mut psDecCtrl.PredCoef_Q12[0],
            &pNLSF0_Q15,
            psDec.LPC_order,
            psDec.arch,
        );
    } else {
        /* Copy LPC coefficients for first half from second half */
        let order = psDec.LPC_order as usize;
        let (half0, half1) = psDecCtrl.PredCoef_Q12.split_at_mut(1);
        half0[0][..order].copy_from_slice(&half1[0][..order]);
    }

    let order = psDec.LPC_order as usize;
    psDec.prevNLSF_Q15[..order].copy_from_slice(&pNLSF_Q15[..order]);

    /* After a packet loss do BWE of LPC coefs */
    if psDec.lossCnt != 0 {
        silk_bwexpander(&mut psDecCtrl.PredCoef_Q12[0], psDec.LPC_order, BWE_AFTER_LOSS_Q16);
        silk_bwexpander(&mut psDecCtrl.PredCoef_Q12[1], psDec.LPC_order, BWE_AFTER_LOSS_Q16);
    }

    if psDec.indices.signalType as opus_int == TYPE_VOICED {
        /*********************/
        /* Decode pitch lags */
        /*********************/

        /* Decode pitch values */
        silk_decode_pitch(
            psDec.indices.lagIndex,
            psDec.indices.contourIndex,
            &mut psDecCtrl.pitchL,
            psDec.fs_kHz,
            psDec.nb_subfr,
        );

        /* Decode Codebook Index */
        let cbk_ptr_Q7 = silk_LTP_vq_ptrs_Q7[psDec.indices.PERIndex as usize]; /* set pointer to start of codebook */

        for k in 0..psDec.nb_subfr {
            Ix = psDec.indices.LTPIndex[k as usize] as opus_int;
            for i in 0..LTP_ORDER {
                psDecCtrl.LTPCoef_Q14[(k * LTP_ORDER + i) as usize] =
                    silk_LSHIFT(cbk_ptr_Q7[(Ix * LTP_ORDER + i) as usize] as opus_int32, 7)
                        as opus_int16;
            }
        }

        /**********************/
        /* Decode LTP scaling */
        /**********************/
        Ix = psDec.indices.LTP_scaleIndex as opus_int;
        psDecCtrl.LTP_scale_Q14 = silk_LTPScales_table_Q14[Ix as usize] as opus_int;
    } else {
        psDecCtrl.pitchL[..psDec.nb_subfr as usize].fill(0);
        psDecCtrl.LTPCoef_Q14[..(LTP_ORDER * psDec.nb_subfr) as usize].fill(0);
        psDec.indices.PERIndex = 0;
        psDecCtrl.LTP_scale_Q14 = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::silk::init_decoder::silk_init_decoder;
    use crate::silk::tables_nlsf_cb_wb::silk_NLSF_CB_WB;

    #[test]
    fn unvoiced_frame_zeroes_ltp_and_pitch() {
        let mut st = silk_decoder_state::default();
        silk_init_decoder(&mut st);
        st.nb_subfr = 4;
        st.LPC_order = 16;
        st.subfr_length = 80;
        st.fs_kHz = 16;
        st.psNLSF_CB = &silk_NLSF_CB_WB;
        st.indices.signalType = TYPE_NO_VOICE_ACTIVITY as opus_int8;
        st.indices.NLSFInterpCoef_Q2 = 4;
        let mut ctrl = silk_decoder_control::default();
        silk_decode_parameters(&mut st, &mut ctrl, CODE_INDEPENDENTLY);
        assert_eq!(ctrl.pitchL, [0; 4]);
        assert_eq!(
            ctrl.LTPCoef_Q14,
            [0; (LTP_ORDER * MAX_NB_SUBFR) as usize]
        );
        assert_eq!(st.indices.PERIndex, 0);
        assert_eq!(ctrl.LTP_scale_Q14, 0);
    }
}

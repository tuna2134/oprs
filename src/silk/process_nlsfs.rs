//! Mirror of `silk/process_NLSFs.c` from libopus 1.6.1, transliterated to safe Rust.
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
use crate::silk::interpolate::*;
use crate::silk::macros::*;
use crate::silk::nlsf2a::*;
use crate::silk::nlsf_encode::*;
use crate::silk::nlsf_vq_weights_laroia::*;
use crate::silk::sigproc_fix::*;
use crate::silk::structs::*;
use crate::silk::typedef::silk_assert;
use crate::silk::typedef::*;
use crate::types::*;

/* Limit, stabilize, convert and quantize NLSFs */
pub fn silk_process_NLSFs(
    psEncC: &mut silk_encoder_state,                              /* I/O  Encoder state            */
    PredCoef_Q12: &mut [[opus_int16; MAX_LPC_ORDER as usize]; 2], /* O    Prediction coefficients  */
    pNLSF_Q15: &mut [opus_int16], /* I/O  Normalized LSFs (quant out) (0 - (2^15-1))  */
    prev_NLSFq_Q15: &[opus_int16], /* I    Previous Normalized LSFs (0 - (2^15-1))    */
) {
    silk_assert!(psEncC.speech_activity_Q8 >= 0);
    silk_assert!(psEncC.speech_activity_Q8 <= SILK_FIX_CONST(1.0, 8));
    celt_assert!(
        psEncC.useInterpolatedNLSFs == 1 || psEncC.indices.NLSFInterpCoef_Q2 == (1 << 2)
    );

    /***********************/
    /* Calculate mu values */
    /***********************/
    /* NLSF_mu  = 0.003 - 0.0015 * psEnc->speech_activity; */
    let mut NLSF_mu_Q20 = silk_SMLAWB(
        SILK_FIX_CONST(0.003, 20),
        SILK_FIX_CONST(-0.001, 28),
        psEncC.speech_activity_Q8,
    );
    if psEncC.nb_subfr == 2 {
        /* Multiply by 1.5 for 10 ms packets */
        NLSF_mu_Q20 = silk_ADD_RSHIFT(NLSF_mu_Q20, NLSF_mu_Q20, 1);
    }

    celt_assert!(NLSF_mu_Q20 > 0);
    silk_assert!(NLSF_mu_Q20 <= SILK_FIX_CONST(0.005, 20));

    let order = psEncC.predictLPCOrder;

    /* Calculate NLSF weights */
    let mut pNLSFW_QW = [0 as opus_int16; MAX_LPC_ORDER as usize];
    silk_NLSF_VQ_weights_laroia(&mut pNLSFW_QW, pNLSF_Q15, order);

    /* Update NLSF weights for interpolated NLSFs */
    let mut pNLSF0_temp_Q15 = [0 as opus_int16; MAX_LPC_ORDER as usize];
    let doInterpolate =
        (psEncC.useInterpolatedNLSFs == 1) && (psEncC.indices.NLSFInterpCoef_Q2 < 4);
    if doInterpolate {
        /* Calculate the interpolated NLSF vector for the first half */
        silk_interpolate(
            &mut pNLSF0_temp_Q15,
            prev_NLSFq_Q15,
            pNLSF_Q15,
            psEncC.indices.NLSFInterpCoef_Q2 as opus_int,
            order,
        );

        /* Calculate first half NLSF weights for the interpolated NLSFs */
        let mut pNLSFW0_temp_QW = [0 as opus_int16; MAX_LPC_ORDER as usize];
        silk_NLSF_VQ_weights_laroia(&mut pNLSFW0_temp_QW, &pNLSF0_temp_Q15, order);

        /* Update NLSF weights with contribution from first half */
        let i_sqr_Q15 = silk_LSHIFT(
            silk_SMULBB(
                psEncC.indices.NLSFInterpCoef_Q2 as opus_int32,
                psEncC.indices.NLSFInterpCoef_Q2 as opus_int32,
            ),
            11,
        ) as opus_int16;
        for i in 0..order {
            pNLSFW_QW[i as usize] = silk_ADD16(
                silk_RSHIFT(pNLSFW_QW[i as usize] as opus_int32, 1) as opus_int16,
                silk_RSHIFT(
                    silk_SMULBB(pNLSFW0_temp_QW[i as usize] as opus_int32, i_sqr_Q15 as opus_int32),
                    16,
                ) as opus_int16,
            ) as opus_int16;
            silk_assert!(pNLSFW_QW[i as usize] >= 1);
        }
    }

    let nSurvivors = psEncC.NLSF_MSVQ_Survivors;
    let signalType = psEncC.indices.signalType as opus_int;
    let psNLSF_CB = psEncC.psNLSF_CB;
    silk_NLSF_encode(
        &mut psEncC.indices.NLSFIndices,
        pNLSF_Q15,
        psNLSF_CB,
        &pNLSFW_QW,
        NLSF_mu_Q20,
        nSurvivors,
        signalType,
    );

    /* Convert quantized NLSFs back to LPC coefficients */
    silk_NLSF2A(&mut PredCoef_Q12[1], pNLSF_Q15, order, psEncC.arch);

    if doInterpolate {
        /* Calculate the interpolated, quantized LSF vector for the first half */
        silk_interpolate(
            &mut pNLSF0_temp_Q15,
            prev_NLSFq_Q15,
            pNLSF_Q15,
            psEncC.indices.NLSFInterpCoef_Q2 as opus_int,
            order,
        );

        /* Convert back to LPC coefficients */
        silk_NLSF2A(&mut PredCoef_Q12[0], &pNLSF0_temp_Q15, order, psEncC.arch);
    } else {
        /* Copy LPC coefficients for first half from second half */
        celt_assert!(psEncC.predictLPCOrder <= MAX_LPC_ORDER);
        for i in 0..order as usize {
            PredCoef_Q12[0][i] = PredCoef_Q12[1][i];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The two prediction-coefficient halves must agree when no interpolation
    /// is used, and mu must stay in the documented range.
    #[test]
    fn no_interpolation_copies_half() {
        let mut enc = silk_encoder_state::default();
        enc.predictLPCOrder = 10;
        enc.nb_subfr = 4;
        enc.NLSF_MSVQ_Survivors = 4;
        enc.useInterpolatedNLSFs = 0;
        enc.indices.NLSFInterpCoef_Q2 = 4;
        enc.speech_activity_Q8 = 100;
        enc.psNLSF_CB = &crate::silk::tables_nlsf_cb_nb_mb::silk_NLSF_CB_NB_MB;
        let mut pred = [[0i16; MAX_LPC_ORDER as usize]; 2];
        let mut nlsf = [0i16; MAX_LPC_ORDER as usize];
        for i in 0..10 {
            nlsf[i] = ((i as i32 + 1) * 3000) as i16;
        }
        let prev = [0i16; MAX_LPC_ORDER as usize];
        silk_process_NLSFs(&mut enc, &mut pred, &mut nlsf, &prev);
        assert_eq!(pred[0], pred[1]);
    }
}

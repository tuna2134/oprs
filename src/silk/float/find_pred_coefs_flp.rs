//! Mirror of `silk/float/find_pred_coefs_FLP.c` from libopus 1.6.1, transliterated to safe Rust.
//!
//! Find LPC and LTP coefficients, and the residual energies.
//!
//! `res_pitch` and `x_buf` are the underlying state buffers: the frame starts at
//! `ltp_mem_length` in both, exactly like the C `res_pitch_frame =
//! res_pitch + ltp_mem_length` and `x_frame = psEnc->x_buf + ltp_mem_length`.
//! Both routines read before the frame start, so the whole buffers are passed.
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
use crate::silk::float::find_lpc_flp::silk_find_LPC_FLP;
use crate::silk::float::find_ltp_flp::silk_find_LTP_FLP;
use crate::silk::float::ltp_analysis_filter_flp::silk_LTP_analysis_filter_FLP;
use crate::silk::float::ltp_scale_ctrl_flp::silk_LTP_scale_ctrl_FLP;
use crate::silk::float::main_flp::{silk_encoder_control_FLP, silk_encoder_state_FLP};
use crate::silk::float::residual_energy_flp::silk_residual_energy_FLP;
use crate::silk::float::scale_copy_vector_flp::silk_scale_copy_vector_FLP;
use crate::silk::float::sigproc_flp::silk_float;
use crate::silk::float::wrappers_flp::{silk_process_NLSFs_FLP, silk_quant_LTP_gains_FLP};
use crate::silk::typedef::*;
use crate::types::*;

/// Find LPC and LTP coefficients
pub fn silk_find_pred_coefs_FLP(
    psEnc: &mut silk_encoder_state_FLP,
    psEncCtrl: &mut silk_encoder_control_FLP,
    res_pitch: &[silk_float], /* C: the `res_pitch` buffer base */
    x_buf: &[silk_float],     /* C: the `psEnc->x_buf` buffer base */

    condCoding: opus_int,
) {
    let mut i: opus_int;
    let mut XXLTP = [0.0f32; (MAX_NB_SUBFR * LTP_ORDER * LTP_ORDER) as usize];
    let mut xXLTP = [0.0f32; (MAX_NB_SUBFR * LTP_ORDER) as usize];
    let mut invGains = [0.0f32; MAX_NB_SUBFR as usize];
    /* Set to NLSF_Q15 to zero so we don't copy junk to the state. */
    let mut NLSF_Q15 = [0 as opus_int16; MAX_LPC_ORDER as usize];
    let mut LPC_in_pre =
        [0.0f32; (MAX_NB_SUBFR * MAX_LPC_ORDER + MAX_FRAME_LENGTH) as usize];
    let mut minInvGain: silk_float;

    /* Weighting for weighted least squares */
    i = 0;
    while i < psEnc.sCmn.nb_subfr {
        silk_assert!(psEncCtrl.Gains[i as usize] > 0.0f32);
        invGains[i as usize] = 1.0f32 / psEncCtrl.Gains[i as usize];
        i += 1;
    }

    if psEnc.sCmn.indices.signalType as opus_int == TYPE_VOICED {
        /**********/
        /* VOICED */
        /**********/
        celt_assert!(
            psEnc.sCmn.ltp_mem_length - psEnc.sCmn.predictLPCOrder
                >= psEncCtrl.pitchL[0] + LTP_ORDER / 2
        );

        /* LTP analysis */
        silk_find_LTP_FLP(
            &mut XXLTP,
            &mut xXLTP,
            res_pitch,
            psEnc.sCmn.ltp_mem_length,
            &psEncCtrl.pitchL,
            psEnc.sCmn.subfr_length,
            psEnc.sCmn.nb_subfr,
            psEnc.sCmn.arch,
        );

        /* Quantize LTP gain parameters */
        silk_quant_LTP_gains_FLP(
            &mut psEncCtrl.LTPCoef,
            &mut psEnc.sCmn.indices.LTPIndex,
            &mut psEnc.sCmn.indices.PERIndex,
            &mut psEnc.sCmn.sum_log_gain_Q7,
            &mut psEncCtrl.LTPredCodGain,
            &XXLTP,
            &xXLTP,
            psEnc.sCmn.subfr_length,
            psEnc.sCmn.nb_subfr,
            psEnc.sCmn.arch,
        );

        /* Control LTP scaling */
        silk_LTP_scale_ctrl_FLP(psEnc, psEncCtrl, condCoding);

        /* Create LTP residual */
        silk_LTP_analysis_filter_FLP(
            &mut LPC_in_pre,
            x_buf,
            psEnc.sCmn.ltp_mem_length - psEnc.sCmn.predictLPCOrder,
            &psEncCtrl.LTPCoef,
            &psEncCtrl.pitchL,
            &invGains,
            psEnc.sCmn.subfr_length,
            psEnc.sCmn.nb_subfr,
            psEnc.sCmn.predictLPCOrder,
        );
    } else {
        /************/
        /* UNVOICED */
        /************/
        /* Create signal with prepended subframes, scaled by inverse gains */
        let mut x_ptr: usize =
            (psEnc.sCmn.ltp_mem_length - psEnc.sCmn.predictLPCOrder) as usize;
        let mut x_pre_ptr: usize = 0;
        i = 0;
        while i < psEnc.sCmn.nb_subfr {
            silk_scale_copy_vector_FLP(
                &mut LPC_in_pre[x_pre_ptr..],
                &x_buf[x_ptr..],
                invGains[i as usize],
                psEnc.sCmn.subfr_length + psEnc.sCmn.predictLPCOrder,
            );
            x_pre_ptr += (psEnc.sCmn.subfr_length + psEnc.sCmn.predictLPCOrder) as usize;
            x_ptr += psEnc.sCmn.subfr_length as usize;
            i += 1;
        }
        for v in psEncCtrl
            .LTPCoef
            .iter_mut()
            .take((psEnc.sCmn.nb_subfr * LTP_ORDER) as usize)
        {
            *v = 0.0;
        }
        psEncCtrl.LTPredCodGain = 0.0f32;
        psEnc.sCmn.sum_log_gain_Q7 = 0;
    }

    /* Limit on total predictive coding gain */
    if psEnc.sCmn.first_frame_after_reset != 0 {
        minInvGain = 1.0f32 / MAX_PREDICTION_POWER_GAIN_AFTER_RESET;
    } else {
        minInvGain = (2.0f64).powf((psEncCtrl.LTPredCodGain / 3.0f32) as f64) as silk_float
            / MAX_PREDICTION_POWER_GAIN;
        minInvGain /= 0.25f32 + 0.75f32 * psEncCtrl.coding_quality;
    }

    /* LPC_in_pre contains the LTP-filtered input for voiced, and the unfiltered input for unvoiced */
    let arch = psEnc.sCmn.arch;
    silk_find_LPC_FLP(
        &mut psEnc.sCmn,
        &mut NLSF_Q15,
        &LPC_in_pre,
        minInvGain,
        arch,
    );

    /* Quantize LSFs */
    let prev_NLSFq_Q15 = psEnc.sCmn.prev_NLSFq_Q15;
    silk_process_NLSFs_FLP(
        &mut psEnc.sCmn,
        &mut psEncCtrl.PredCoef,
        &mut NLSF_Q15,
        &prev_NLSFq_Q15,
    );

    /* Calculate residual energy using quantized LPC coefficients */
    silk_residual_energy_FLP(
        &mut psEncCtrl.ResNrg,
        &LPC_in_pre,
        &psEncCtrl.PredCoef,
        &psEncCtrl.Gains,
        psEnc.sCmn.subfr_length,
        psEnc.sCmn.nb_subfr,
        psEnc.sCmn.predictLPCOrder,
    );

    /* Copy to prediction struct for use in next frame for interpolation */
    psEnc.sCmn.prev_NLSFq_Q15.copy_from_slice(&NLSF_Q15);
}

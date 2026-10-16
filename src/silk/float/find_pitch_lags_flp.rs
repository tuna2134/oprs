//! Mirror of `silk/float/find_pitch_lags_FLP.c` from libopus 1.6.1, transliterated to safe Rust.
//!
//! Pitch-lag estimation front-end: windowing, LPC whitening, pitch search.
//!
//! `x_buf` corresponds to the C `x_buf = x - ltp_mem_length`, i.e. slice index 0
//! is `ltp_mem_length` samples before the frame the C call passes in `x`; the
//! window reads start at `buf_len - pitch_LPC_win_length` inside it.
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
use crate::silk::define::{
    FIND_PITCH_LPC_WIN_MAX, MAX_FIND_PITCH_LPC_ORDER, TYPE_NO_VOICE_ACTIVITY, TYPE_UNVOICED,
    TYPE_VOICED,
};
use crate::silk::float::apply_sine_window_flp::silk_apply_sine_window_FLP;
use crate::silk::float::autocorrelation_flp::silk_autocorrelation_FLP;
use crate::silk::float::bwexpander_flp::silk_bwexpander_FLP;
use crate::silk::float::k2a_flp::silk_k2a_FLP;
use crate::silk::float::lpc_analysis_filter_flp::silk_LPC_analysis_filter_FLP;
use crate::silk::float::main_flp::{silk_encoder_control_FLP, silk_encoder_state_FLP};
use crate::silk::float::pitch_analysis_core_flp::silk_pitch_analysis_core_FLP;
use crate::silk::float::schur_flp::silk_schur_FLP;
use crate::silk::float::sigproc_flp::{silk_float, silk_max_float};
use crate::silk::typedef::*;
use crate::types::*;

/// `FIND_PITCH_WHITE_NOISE_FRACTION` (`1e-3f`)
const FIND_PITCH_WHITE_NOISE_FRACTION: silk_float = 1e-3;
/// `FIND_PITCH_BANDWIDTH_EXPANSION` (`0.99f`)
const FIND_PITCH_BANDWIDTH_EXPANSION: silk_float = 0.99;

pub fn silk_find_pitch_lags_FLP(
    psEnc: &mut silk_encoder_state_FLP,
    psEncCtrl: &mut silk_encoder_control_FLP,
    res: &mut [silk_float],
    x_buf: &[silk_float],
    arch: opus_int,
) {
    let buf_len: opus_int;
    let mut thrhld: silk_float;
    let mut res_nrg: silk_float;
    let mut auto_corr = [0.0f32; (MAX_FIND_PITCH_LPC_ORDER + 1) as usize];
    let mut A = [0.0f32; MAX_FIND_PITCH_LPC_ORDER as usize];
    let mut refl_coef = [0.0f32; MAX_FIND_PITCH_LPC_ORDER as usize];
    let mut Wsig = [0.0f32; FIND_PITCH_LPC_WIN_MAX as usize];

    /******************************************/
    /* Set up buffer lengths etc based on Fs  */
    /******************************************/
    buf_len = psEnc.sCmn.la_pitch + psEnc.sCmn.frame_length + psEnc.sCmn.ltp_mem_length;

    /* Safety check */
    celt_assert!(buf_len >= psEnc.sCmn.pitch_LPC_win_length);

    /******************************************/
    /* Estimate LPC AR coefficients           */
    /******************************************/

    /* Calculate windowed signal */

    /* First LA_LTP samples */
    let mut xb_off = (buf_len - psEnc.sCmn.pitch_LPC_win_length) as usize;
    let mut wsig_off = 0usize;
    silk_apply_sine_window_FLP(
        &mut Wsig,
        &x_buf[xb_off..],
        1,
        psEnc.sCmn.la_pitch,
    );

    /* Middle non-windowed samples */
    wsig_off += psEnc.sCmn.la_pitch as usize;
    xb_off += psEnc.sCmn.la_pitch as usize;
    let mid = (psEnc.sCmn.pitch_LPC_win_length - (psEnc.sCmn.la_pitch << 1)) as usize;
    Wsig[wsig_off..wsig_off + mid].copy_from_slice(&x_buf[xb_off..xb_off + mid]);

    /* Last LA_LTP samples */
    wsig_off += mid;
    xb_off += mid;
    silk_apply_sine_window_FLP(
        &mut Wsig[wsig_off..],
        &x_buf[xb_off..],
        2,
        psEnc.sCmn.la_pitch,
    );

    /* Calculate autocorrelation sequence */
    silk_autocorrelation_FLP(
        &mut auto_corr,
        &Wsig,
        psEnc.sCmn.pitch_LPC_win_length,
        psEnc.sCmn.pitchEstimationLPCOrder + 1,
        arch,
    );

    /* Add white noise, as a fraction of the energy */
    auto_corr[0] += auto_corr[0] * FIND_PITCH_WHITE_NOISE_FRACTION + 1.0f32;

    /* Calculate the reflection coefficients using Schur */
    res_nrg = silk_schur_FLP(&mut refl_coef, &auto_corr, psEnc.sCmn.pitchEstimationLPCOrder);

    /* Prediction gain */
    psEncCtrl.predGain = auto_corr[0] / silk_max_float(res_nrg, 1.0f32);

    /* Convert reflection coefficients to prediction coefficients */
    silk_k2a_FLP(&mut A, &refl_coef, psEnc.sCmn.pitchEstimationLPCOrder);

    /* Bandwidth expansion */
    silk_bwexpander_FLP(
        &mut A,
        psEnc.sCmn.pitchEstimationLPCOrder,
        FIND_PITCH_BANDWIDTH_EXPANSION,
    );

    /*****************************************/
    /* LPC analysis filtering                */
    /*****************************************/
    silk_LPC_analysis_filter_FLP(res, &A, x_buf, buf_len, psEnc.sCmn.pitchEstimationLPCOrder);

    if psEnc.sCmn.indices.signalType as opus_int != TYPE_NO_VOICE_ACTIVITY
        && psEnc.sCmn.first_frame_after_reset == 0
    {
        /* Threshold for pitch estimator */
        thrhld = 0.6f32;
        thrhld -= 0.004f32 * psEnc.sCmn.pitchEstimationLPCOrder as silk_float;
        thrhld -= 0.1f32 * psEnc.sCmn.speech_activity_Q8 as silk_float * (1.0f32 / 256.0f32);
        thrhld -= 0.15f32 * ((psEnc.sCmn.prevSignalType as opus_int >> 1) as silk_float);
        thrhld -= 0.1f32 * psEnc.sCmn.input_tilt_Q15 as silk_float * (1.0f32 / 32768.0f32);

        /*****************************************/
        /* Call Pitch estimator                  */
        /*****************************************/
        let threshold = psEnc.sCmn.pitchEstimationThreshold_Q16 as silk_float / 65536.0f32;
        let r = silk_pitch_analysis_core_FLP(
            res,
            &mut psEncCtrl.pitchL,
            &mut psEnc.sCmn.indices.lagIndex,
            &mut psEnc.sCmn.indices.contourIndex,
            &mut psEnc.LTPCorr,
            psEnc.sCmn.prevLag,
            threshold,
            thrhld,
            psEnc.sCmn.fs_kHz,
            psEnc.sCmn.pitchEstimationComplexity,
            psEnc.sCmn.nb_subfr,
            arch,
        );
        if r == 0 {
            psEnc.sCmn.indices.signalType = TYPE_VOICED as opus_int8;
        } else {
            psEnc.sCmn.indices.signalType = TYPE_UNVOICED as opus_int8;
        }
    } else {
        for v in psEncCtrl.pitchL.iter_mut() {
            *v = 0;
        }
        psEnc.sCmn.indices.lagIndex = 0;
        psEnc.sCmn.indices.contourIndex = 0;
        psEnc.LTPCorr = 0.0;
    }
}

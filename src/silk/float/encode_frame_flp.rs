//! Mirror of `silk/float/encode_frame_FLP.c` from libopus 1.6.1, transliterated to safe Rust.
//!
//! The floating-point SILK frame encoder: VAD handling, the LBRR path and the
//! rate-control loop around the noise-shaping quantizer.
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
use crate::celt::entcode::ec_ctx;
use crate::silk::define::*;
use crate::silk::encode_indices::silk_encode_indices;
use crate::silk::encode_pulses::silk_encode_pulses;
use crate::silk::float::find_pitch_lags_flp::silk_find_pitch_lags_FLP;
use crate::silk::float::find_pred_coefs_flp::silk_find_pred_coefs_FLP;
use crate::silk::float::main_flp::{silk_encoder_control_FLP, silk_encoder_state_FLP};
use crate::silk::float::noise_shape_analysis_flp::silk_noise_shape_analysis_FLP;
use crate::silk::float::process_gains_flp::silk_process_gains_FLP;
use crate::silk::float::sigproc_flp::{silk_float, silk_max_float, silk_short2float_array};
use crate::silk::float::wrappers_flp::silk_NSQ_wrapper_FLP;
use crate::silk::gain_quant::{silk_gains_ID, silk_gains_dequant, silk_gains_quant};
use crate::silk::lp_variable_cutoff::silk_LP_variable_cutoff;
use crate::silk::macros::silk_SMULWB;
use crate::silk::sigproc_fix::*;
use crate::silk::structs::silk_nsq_state;
use crate::silk::typedef::*;
use crate::silk::vad::silk_VAD_GetSA_Q8;
use crate::types::*;

/// `SPEECH_ACTIVITY_DTX_THRES` (`silk/tuning_parameters.h`, `0.05f`)
const SPEECH_ACTIVITY_DTX_THRES: silk_float = 0.05;
/// `LBRR_SPEECH_ACTIVITY_THRES` (`silk/tuning_parameters.h`, `0.3f`)
const LBRR_SPEECH_ACTIVITY_THRES: silk_float = 0.3;

/// `silk_encode_do_VAD_FLP`
pub fn silk_encode_do_VAD_FLP(psEnc: &mut silk_encoder_state_FLP, activity: opus_int) {
    let activity_threshold = SILK_FIX_CONST(SPEECH_ACTIVITY_DTX_THRES as f64, 8);

    /****************************/
    /* Voice Activity Detection */
    /****************************/
    /* Rust cannot pass `&mut psEnc->sCmn` and a slice of `psEnc->sCmn.inputBuf`
     * at once, so the (read-only) input window is snapshotted. */
    let fl = psEnc.sCmn.frame_length as usize;
    let arch = psEnc.sCmn.arch;
    let mut inbuf = [0 as opus_int16; MAX_FRAME_LENGTH as usize];
    inbuf[..fl].copy_from_slice(&psEnc.sCmn.inputBuf[1..1 + fl]);
    silk_VAD_GetSA_Q8(&mut psEnc.sCmn, &inbuf, arch);
    /* If Opus VAD is inactive and Silk VAD is active: lower Silk VAD to just under the threshold */
    if activity == VAD_NO_ACTIVITY && psEnc.sCmn.speech_activity_Q8 >= activity_threshold {
        psEnc.sCmn.speech_activity_Q8 = activity_threshold - 1;
    }

    /**************************************************/
    /* Convert speech activity into VAD and DTX flags */
    /**************************************************/
    let nfe = psEnc.sCmn.nFramesEncoded as usize;
    if psEnc.sCmn.speech_activity_Q8 < activity_threshold {
        psEnc.sCmn.indices.signalType = TYPE_NO_VOICE_ACTIVITY as opus_int8;
        psEnc.sCmn.noSpeechCounter += 1;
        if psEnc.sCmn.noSpeechCounter <= NB_SPEECH_FRAMES_BEFORE_DTX {
            psEnc.sCmn.inDTX = 0;
        } else if psEnc.sCmn.noSpeechCounter > MAX_CONSECUTIVE_DTX + NB_SPEECH_FRAMES_BEFORE_DTX {
            psEnc.sCmn.noSpeechCounter = NB_SPEECH_FRAMES_BEFORE_DTX;
            psEnc.sCmn.inDTX = 0;
        }
        psEnc.sCmn.VAD_flags[nfe] = 0;
    } else {
        psEnc.sCmn.noSpeechCounter = 0;
        psEnc.sCmn.inDTX = 0;
        psEnc.sCmn.indices.signalType = TYPE_UNVOICED as opus_int8;
        psEnc.sCmn.VAD_flags[nfe] = 1;
    }
}

/// Low-Bitrate Redundancy (LBRR) encoding. Reuse all parameters but encode excitation at lower bitrate
fn silk_LBRR_encode_FLP(
    psEnc: &mut silk_encoder_state_FLP,
    psEncCtrl: &mut silk_encoder_control_FLP,
    xfw: &[silk_float],
    condCoding: opus_int,
) {
    let mut k: opus_int;
    let mut Gains_Q16 = [0 as opus_int32; MAX_NB_SUBFR as usize];
    let mut TempGains = [0.0f32; MAX_NB_SUBFR as usize];
    let n = psEnc.sCmn.nFramesEncoded as usize;
    let nb = psEnc.sCmn.nb_subfr as usize;
    let mut psIndices_LBRR = psEnc.sCmn.indices_LBRR[n];

    /*******************************************/
    /* Control use of inband LBRR              */
    /*******************************************/
    if psEnc.sCmn.LBRR_enabled != 0
        && psEnc.sCmn.speech_activity_Q8 > SILK_FIX_CONST(LBRR_SPEECH_ACTIVITY_THRES as f64, 8)
    {
        psEnc.sCmn.LBRR_flags[n] = 1;

        /* Copy noise shaping quantizer state and quantization indices from regular encoding */
        let mut nsq_lbrr = psEnc.sCmn.sNSQ;
        psIndices_LBRR = psEnc.sCmn.indices;

        /* Save original gains */
        TempGains[..nb].copy_from_slice(&psEncCtrl.Gains[..nb]);

        if psEnc.sCmn.nFramesEncoded == 0 || psEnc.sCmn.LBRR_flags[n - 1] == 0 {
            /* First frame in packet or previous frame not LBRR coded */
            psEnc.sCmn.LBRRprevLastGainIndex = psEnc.sShape.LastGainIndex;

            /* Increase Gains to get target LBRR rate */
            let v = psIndices_LBRR.GainsIndices[0] as opus_int + psEnc.sCmn.LBRR_GainIncreases;
            psIndices_LBRR.GainsIndices[0] = silk_min_int(v, N_LEVELS_QGAIN - 1) as opus_int8;
        }

        /* Decode to get gains in sync with decoder */
        silk_gains_dequant(
            &mut Gains_Q16,
            &psIndices_LBRR.GainsIndices,
            &mut psEnc.sCmn.LBRRprevLastGainIndex,
            (condCoding == CODE_CONDITIONALLY) as opus_int,
            psEnc.sCmn.nb_subfr,
        );

        /* Overwrite unquantized gains with quantized gains and convert back to Q0 from Q16 */
        k = 0;
        while k < psEnc.sCmn.nb_subfr {
            psEncCtrl.Gains[k as usize] =
                Gains_Q16[k as usize] as silk_float * (1.0f32 / 65536.0f32);
            k += 1;
        }

        /*****************************************/
        /* Noise shaping quantization            */
        /*****************************************/
        let mut pulses = psEnc.sCmn.pulses_LBRR[n];
        silk_NSQ_wrapper_FLP(
            &psEnc.sCmn,
            psEncCtrl,
            &mut psIndices_LBRR,
            &mut nsq_lbrr,
            &mut pulses,
            xfw,
        );
        psEnc.sCmn.pulses_LBRR[n] = pulses;
        psEnc.sCmn.indices_LBRR[n] = psIndices_LBRR;

        /* Restore original gains */
        psEncCtrl.Gains[..nb].copy_from_slice(&TempGains[..nb]);
    }
}

/****************/
/* Encode frame */
/****************/
pub fn silk_encode_frame_FLP(
    psEnc: &mut silk_encoder_state_FLP,
    nBytesOut: &mut opus_int32,
    psRangeEnc: &mut ec_ctx,
    condCoding: opus_int,
    maxBits: opus_int,
    useCBR: opus_int,
) -> opus_int {
    let mut sEncCtrl = silk_encoder_control_FLP::default();
    let mut i: opus_int;
    let mut iter: opus_int;
    let maxIter: opus_int;
    let mut found_upper: opus_int;
    let mut found_lower: opus_int;
    let mut ret: opus_int = 0;
    let mut res_pitch = [0.0f32; (2 * MAX_FRAME_LENGTH + LA_PITCH_MAX) as usize];
    let mut sNSQ_copy = [silk_nsq_state::default(); 2];
    let mut seed_copy: opus_int32;
    let mut nBits: opus_int32 = 0;
    let mut nBits_lower: opus_int32 = 0;
    let mut nBits_upper: opus_int32 = 0;
    let mut gainMult_lower: opus_int32 = 0;
    let mut gainMult_upper: opus_int32 = 0;
    let mut gainsID: opus_int32;
    let mut gainsID_lower: opus_int32;
    let mut gainsID_upper: opus_int32;
    let mut gainMult_Q8: opus_int16;
    let mut ec_prevLagIndex_copy: opus_int16;
    let mut ec_prevSignalType_copy: opus_int;
    let mut LastGainIndex_copy2: opus_int8 = 0;
    let mut pGains_Q16 = [0 as opus_int32; MAX_NB_SUBFR as usize];
    let mut gain_lock = [0 as opus_int; MAX_NB_SUBFR as usize];
    let mut best_gain_mult = [0 as opus_int16; MAX_NB_SUBFR as usize];
    let mut best_sum = [0 as opus_int; MAX_NB_SUBFR as usize];
    let bits_margin: opus_int;

    /* For CBR, 5 bits below budget is close enough. For VBR, allow up to 25% below the cap if we initially busted the budget. */
    bits_margin = if useCBR != 0 { 5 } else { maxBits / 4 };
    /* This is totally unnecessary but many compilers (including gcc) are too dumb to realise it */
    LastGainIndex_copy2 = 0;
    nBits_lower = 0;
    nBits_upper = 0;
    gainMult_lower = 0;
    gainMult_upper = 0;

    psEnc.sCmn.indices.Seed = (psEnc.sCmn.frameCounter & 3) as opus_int8;
    psEnc.sCmn.frameCounter += 1;

    /**************************************************************/
    /* Set up Input Pointers, and insert frame in input buffer    */
    /**************************************************************/
    /* pointers aligned with start of frame to encode */
    let x_frame = psEnc.sCmn.ltp_mem_length as usize;
    let res_pitch_frame = x_frame;
    /* The frame insertion (and the input-buffer update) uses the fixed
     * `LA_SHAPE_MS * fs_kHz` look-ahead, but
     * `silk_noise_shape_analysis_FLP` offsets its `x` argument by
     * `psEnc->sCmn.la_shape`, which is only `3 * fs_kHz` for the lowest
     * pitch-estimation complexities (see `silk_setup_complexity`). */
    let la_shape_win = (LA_SHAPE_MS * psEnc.sCmn.fs_kHz) as usize;
    let la_shape = psEnc.sCmn.la_shape as usize;
    let fl = psEnc.sCmn.frame_length as usize;
    /* Rust: a local copy of `psEnc->x_buf`; the only writes to it are the frame
     * insertion and the pointer `x_frame` lives in the same buffer. */
    let mut x_buf = psEnc.x_buf;

    /***************************************/
    /* Ensure smooth bandwidth transitions */
    /***************************************/
    silk_LP_variable_cutoff(
        &mut psEnc.sCmn.sLP,
        &mut psEnc.sCmn.inputBuf[1..],
        psEnc.sCmn.frame_length,
    );

    /*******************************************/
    /* Copy new frame to front of input buffer */
    /*******************************************/
    silk_short2float_array(
        &mut x_buf[x_frame + la_shape_win..],
        &psEnc.sCmn.inputBuf[1..],
        psEnc.sCmn.frame_length,
    );

    /* Add tiny signal to avoid high CPU load from denormalized floating point numbers */
    i = 0;
    while i < 8 {
        x_buf[x_frame + la_shape_win + (i * (psEnc.sCmn.frame_length >> 3)) as usize] +=
            ((1 - (i & 2)) as silk_float) * 1e-6f32;
        i += 1;
    }

    if psEnc.sCmn.prefillFlag == 0 {
        let mut ec_buf_copy = [0u8; 1275];

        /*****************************************/
        /* Find pitch lags, initial LPC analysis */
        /*****************************************/
        silk_find_pitch_lags_FLP(
            psEnc,
            &mut sEncCtrl,
            &mut res_pitch,
            &x_buf,
            psEnc.sCmn.arch,
        );

        /************************/
        /* Noise shape analysis */
        /************************/
        silk_noise_shape_analysis_FLP(
            psEnc,
            &mut sEncCtrl,
            &res_pitch[res_pitch_frame..],
            &x_buf[x_frame - la_shape..],
        );

        /***************************************************/
        /* Find linear prediction coefficients (LPC + LTP) */
        /***************************************************/
        silk_find_pred_coefs_FLP(
            psEnc,
            &mut sEncCtrl,
            &res_pitch,
            &x_buf,
            condCoding,
        );

        /****************************************/
        /* Process gains                        */
        /****************************************/
        silk_process_gains_FLP(psEnc, &mut sEncCtrl, condCoding);

        /****************************************/
        /* Low Bitrate Redundant Encoding       */
        /****************************************/
        silk_LBRR_encode_FLP(psEnc, &mut sEncCtrl, &x_buf[x_frame..], condCoding);

        /* Loop over quantizer and entroy coding to control bitrate */
        maxIter = 6;
        gainMult_Q8 = SILK_FIX_CONST(1.0, 8) as opus_int16;
        found_lower = 0;
        found_upper = 0;
        gainsID = silk_gains_ID(&psEnc.sCmn.indices.GainsIndices, psEnc.sCmn.nb_subfr);
        gainsID_lower = -1;
        gainsID_upper = -1;
        /* Copy part of the input state */
        let mut sRangeEnc_copy = psRangeEnc.clone();
        sNSQ_copy[0] = psEnc.sCmn.sNSQ;
        seed_copy = psEnc.sCmn.indices.Seed as opus_int32;
        ec_prevLagIndex_copy = psEnc.sCmn.ec_prevLagIndex;
        ec_prevSignalType_copy = psEnc.sCmn.ec_prevSignalType;
        let mut sRangeEnc_copy2 = psRangeEnc.clone();
        iter = 0;
        loop {
            if gainsID == gainsID_lower {
                nBits = nBits_lower;
            } else if gainsID == gainsID_upper {
                nBits = nBits_upper;
            } else {
                /* Restore part of the input state */
                if iter > 0 {
                    *psRangeEnc = sRangeEnc_copy.clone();
                    psEnc.sCmn.sNSQ = sNSQ_copy[0];
                    psEnc.sCmn.indices.Seed = seed_copy as opus_int8;
                    psEnc.sCmn.ec_prevLagIndex = ec_prevLagIndex_copy;
                    psEnc.sCmn.ec_prevSignalType = ec_prevSignalType_copy;
                }

                /*****************************************/
                /* Noise shaping quantization            */
                /*****************************************/
                {
                    let mut indices = psEnc.sCmn.indices;
                    let mut nsq = psEnc.sCmn.sNSQ;
                    let mut pulses = psEnc.sCmn.pulses;
                    silk_NSQ_wrapper_FLP(
                        &psEnc.sCmn,
                        &sEncCtrl,
                        &mut indices,
                        &mut nsq,
                        &mut pulses,
                        &x_buf[x_frame..],
                    );
                    psEnc.sCmn.indices = indices;
                    psEnc.sCmn.sNSQ = nsq;
                    psEnc.sCmn.pulses = pulses;
                }

                if iter == maxIter && found_lower == 0 {
                    sRangeEnc_copy2 = psRangeEnc.clone();
                }

                /****************************************/
                /* Encode Parameters                    */
                /****************************************/
                let nfe = psEnc.sCmn.nFramesEncoded;
                silk_encode_indices(&mut psEnc.sCmn, psRangeEnc, nfe, 0, condCoding);

                /****************************************/
                /* Encode Excitation Signal             */
                /****************************************/
                silk_encode_pulses(
                    psRangeEnc,
                    psEnc.sCmn.indices.signalType as opus_int,
                    psEnc.sCmn.indices.quantOffsetType as opus_int,
                    &mut psEnc.sCmn.pulses,
                    psEnc.sCmn.frame_length,
                );

                nBits = psRangeEnc.ec_tell();

                /* If we still bust after the last iteration, do some damage control. */
                if iter == maxIter && found_lower == 0 && nBits > maxBits {
                    *psRangeEnc = sRangeEnc_copy2.clone();

                    /* Keep gains the same as the last frame. */
                    psEnc.sShape.LastGainIndex = sEncCtrl.lastGainIndexPrev;
                    i = 0;
                    while i < psEnc.sCmn.nb_subfr {
                        psEnc.sCmn.indices.GainsIndices[i as usize] = 4;
                        i += 1;
                    }
                    if condCoding != CODE_CONDITIONALLY {
                        psEnc.sCmn.indices.GainsIndices[0] = sEncCtrl.lastGainIndexPrev;
                    }
                    psEnc.sCmn.ec_prevLagIndex = ec_prevLagIndex_copy;
                    psEnc.sCmn.ec_prevSignalType = ec_prevSignalType_copy;
                    /* Clear all pulses. */
                    i = 0;
                    while i < psEnc.sCmn.frame_length {
                        psEnc.sCmn.pulses[i as usize] = 0;
                        i += 1;
                    }

                    silk_encode_indices(&mut psEnc.sCmn, psRangeEnc, nfe, 0, condCoding);

                    silk_encode_pulses(
                        psRangeEnc,
                        psEnc.sCmn.indices.signalType as opus_int,
                        psEnc.sCmn.indices.quantOffsetType as opus_int,
                        &mut psEnc.sCmn.pulses,
                        psEnc.sCmn.frame_length,
                    );

                    nBits = psRangeEnc.ec_tell();
                }

                if useCBR == 0 && iter == 0 && nBits <= maxBits {
                    break;
                }
            }

            if iter == maxIter {
                if found_lower != 0 && (gainsID == gainsID_lower || nBits > maxBits) {
                    /* Restore output state from earlier iteration that did meet the bitrate budget */
                    *psRangeEnc = sRangeEnc_copy2.clone();
                    celt_assert!(sRangeEnc_copy2.offs <= 1275);
                    psRangeEnc.buf[..sRangeEnc_copy2.offs as usize]
                        .copy_from_slice(&ec_buf_copy[..sRangeEnc_copy2.offs as usize]);
                    psEnc.sCmn.sNSQ = sNSQ_copy[1];
                    psEnc.sShape.LastGainIndex = LastGainIndex_copy2;
                }
                break;
            }

            if nBits > maxBits {
                if found_lower == 0 && iter >= 2 {
                    /* Adjust the quantizer's rate/distortion tradeoff and discard previous "upper" results */
                    sEncCtrl.Lambda = silk_max_float(sEncCtrl.Lambda * 1.5f32, 1.5f32);
                    /* Reducing dithering can help us hit the target. */
                    psEnc.sCmn.indices.quantOffsetType = 0;
                    found_upper = 0;
                    gainsID_upper = -1;
                } else {
                    found_upper = 1;
                    nBits_upper = nBits;
                    gainMult_upper = gainMult_Q8 as opus_int32;
                    gainsID_upper = gainsID;
                }
            } else if nBits < maxBits - bits_margin {
                found_lower = 1;
                nBits_lower = nBits;
                gainMult_lower = gainMult_Q8 as opus_int32;
                if gainsID != gainsID_lower {
                    gainsID_lower = gainsID;
                    /* Copy part of the output state */
                    sRangeEnc_copy2 = psRangeEnc.clone();
                    celt_assert!(psRangeEnc.offs <= 1275);
                    ec_buf_copy[..psRangeEnc.offs as usize]
                        .copy_from_slice(&psRangeEnc.buf[..psRangeEnc.offs as usize]);
                    sNSQ_copy[1] = psEnc.sCmn.sNSQ;
                    LastGainIndex_copy2 = psEnc.sShape.LastGainIndex;
                }
            } else {
                /* Close enough */
                break;
            }

            if found_lower == 0 && nBits > maxBits {
                i = 0;
                while i < psEnc.sCmn.nb_subfr {
                    let mut sum: opus_int = 0;
                    let mut j: opus_int = i * psEnc.sCmn.subfr_length;
                    while j < (i + 1) * psEnc.sCmn.subfr_length {
                        sum += silk_abs(psEnc.sCmn.pulses[j as usize] as opus_int32);
                        j += 1;
                    }
                    if iter == 0 || (sum < best_sum[i as usize] && gain_lock[i as usize] == 0) {
                        best_sum[i as usize] = sum;
                        best_gain_mult[i as usize] = gainMult_Q8;
                    } else {
                        gain_lock[i as usize] = 1;
                    }
                    i += 1;
                }
            }
            if (found_lower & found_upper) == 0 {
                /* Adjust gain according to high-rate rate/distortion curve */
                if nBits > maxBits {
                    gainMult_Q8 =
                        silk_min_32(1024, (gainMult_Q8 as opus_int32 * 3) / 2) as opus_int16;
                } else {
                    gainMult_Q8 =
                        silk_max_32(64, (gainMult_Q8 as opus_int32 * 4) / 5) as opus_int16;
                }
            } else {
                /* Adjust gain by interpolating */
                gainMult_Q8 = (gainMult_lower
                    + ((gainMult_upper - gainMult_lower) * (maxBits - nBits_lower))
                        / (nBits_upper - nBits_lower)) as opus_int16;
                /* New gain multiplier must be between 25% and 75% of old range (note that gainMult_upper < gainMult_lower) */
                if (gainMult_Q8 as opus_int32)
                    > silk_ADD_RSHIFT32(gainMult_lower, gainMult_upper - gainMult_lower, 2)
                {
                    gainMult_Q8 =
                        silk_ADD_RSHIFT32(gainMult_lower, gainMult_upper - gainMult_lower, 2)
                            as opus_int16;
                } else if (gainMult_Q8 as opus_int32)
                    < silk_SUB_RSHIFT32(gainMult_upper, gainMult_upper - gainMult_lower, 2)
                {
                    gainMult_Q8 =
                        silk_SUB_RSHIFT32(gainMult_upper, gainMult_upper - gainMult_lower, 2)
                            as opus_int16;
                }
            }

            i = 0;
            while i < psEnc.sCmn.nb_subfr {
                let tmp: opus_int16 = if gain_lock[i as usize] != 0 {
                    best_gain_mult[i as usize]
                } else {
                    gainMult_Q8
                };
                pGains_Q16[i as usize] = silk_LSHIFT_SAT32(
                    silk_SMULWB(sEncCtrl.GainsUnq_Q16[i as usize], tmp as opus_int32),
                    8,
                );
                i += 1;
            }

            /* Quantize gains */
            psEnc.sShape.LastGainIndex = sEncCtrl.lastGainIndexPrev;
            silk_gains_quant(
                &mut psEnc.sCmn.indices.GainsIndices,
                &mut pGains_Q16,
                &mut psEnc.sShape.LastGainIndex,
                (condCoding == CODE_CONDITIONALLY) as opus_int,
                psEnc.sCmn.nb_subfr,
            );

            /* Unique identifier of gains vector */
            gainsID = silk_gains_ID(&psEnc.sCmn.indices.GainsIndices, psEnc.sCmn.nb_subfr);

            /* Overwrite unquantized gains with quantized gains and convert back to Q0 from Q16 */
            i = 0;
            while i < psEnc.sCmn.nb_subfr {
                sEncCtrl.Gains[i as usize] =
                    pGains_Q16[i as usize] as silk_float / 65536.0f32;
                i += 1;
            }

            iter += 1;
        }
    }

    /* Update input buffer */
    let ltp_mem = psEnc.sCmn.ltp_mem_length as usize;
    x_buf.copy_within(fl..fl + ltp_mem + la_shape_win, 0);
    psEnc.x_buf = x_buf;

    /* Exit without entropy coding */
    if psEnc.sCmn.prefillFlag != 0 {
        /* No payload */
        *nBytesOut = 0;
        return ret;
    }

    /* Parameters needed for next frame */
    psEnc.sCmn.prevLag = sEncCtrl.pitchL[(psEnc.sCmn.nb_subfr - 1) as usize];
    psEnc.sCmn.prevSignalType = psEnc.sCmn.indices.signalType;

    /****************************************/
    /* Finalize payload                     */
    /****************************************/
    psEnc.sCmn.first_frame_after_reset = 0;
    /* Payload size */
    *nBytesOut = silk_RSHIFT(psRangeEnc.ec_tell() + 7, 3);

    ret
}

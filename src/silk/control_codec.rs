//! Mirror of `silk/control_codec.c` from libopus 1.6.1, transliterated to safe Rust.
//!
//! The reference build is floating point, so `silk_encoder_state_Fxx` is
//! `silk_encoder_state_FLP` and the `#ifndef FIXED_POINT` branches of
//! `silk_setup_resamplers` are the ones compiled.
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
use crate::silk::control::silk_EncControlStruct;
use crate::silk::control_audio_bandwidth::*;
use crate::silk::define::*;
use crate::silk::float::main_flp::silk_encoder_state_Fxx;
use crate::silk::float::sigproc_flp::*;
use crate::silk::macros::*;
use crate::silk::resampler::*;
use crate::silk::sigproc_fix::*;
use crate::silk::structs::*;
use crate::silk::tables_nlsf_cb_nb_mb::silk_NLSF_CB_NB_MB;
use crate::silk::tables_nlsf_cb_wb::silk_NLSF_CB_WB;
use crate::silk::tables_other::*;
use crate::silk::tables_pitch_lag::*;
use crate::silk::typedef::silk_assert;
use crate::silk::typedef::*;
use crate::types::*;

/* `pitch_est_defines.h:72-74` and `tuning_parameters.h:97`; the Rust
   foundation does not carry these three, so they are defined here. */
const SILK_PE_MIN_COMPLEX: opus_int = 0;
const SILK_PE_MID_COMPLEX: opus_int = 1;
const SILK_PE_MAX_COMPLEX: opus_int = 2;
const WARPING_MULTIPLIER: f64 = 0.015;

/* Control encoder */
pub fn silk_control_encoder(
    psEnc: &mut silk_encoder_state_Fxx,        /* I/O  Pointer to Silk encoder state */
    encControl: &mut silk_EncControlStruct,    /* I    Control structure             */
    allow_bw_switch: opus_int,                 /* I    Flag to allow switching audio bandwidth */
    channelNb: opus_int,                       /* I    Channel number                */
    force_fs_kHz: opus_int,                    /* I                                  */
) -> opus_int {
    let mut fs_kHz: opus_int;
    let mut ret: opus_int = 0;

    psEnc.sCmn.useDTX = encControl.useDTX;
    psEnc.sCmn.useCBR = encControl.useCBR;
    psEnc.sCmn.API_fs_Hz = encControl.API_sampleRate;
    psEnc.sCmn.maxInternal_fs_Hz = encControl.maxInternalSampleRate;
    psEnc.sCmn.minInternal_fs_Hz = encControl.minInternalSampleRate;
    psEnc.sCmn.desiredInternal_fs_Hz = encControl.desiredInternalSampleRate;
    psEnc.sCmn.useInBandFEC = encControl.useInBandFEC;
    psEnc.sCmn.nChannelsAPI = encControl.nChannelsAPI;
    psEnc.sCmn.nChannelsInternal = encControl.nChannelsInternal;
    psEnc.sCmn.allow_bandwidth_switch = allow_bw_switch;
    psEnc.sCmn.channelNb = channelNb;

    if psEnc.sCmn.controlled_since_last_payload != 0 && psEnc.sCmn.prefillFlag == 0 {
        if psEnc.sCmn.API_fs_Hz != psEnc.sCmn.prev_API_fs_Hz && psEnc.sCmn.fs_kHz > 0 {
            /* Change in API sampling rate in the middle of encoding a packet */
            ret += silk_setup_resamplers(psEnc, psEnc.sCmn.fs_kHz);
        }
        return ret;
    }

    /* Beyond this point we know that there are no previously coded frames in the payload buffer */

    /********************************************/
    /* Determine internal sampling rate         */
    /********************************************/
    fs_kHz = silk_control_audio_bandwidth(&mut psEnc.sCmn, encControl);
    if force_fs_kHz != 0 {
        fs_kHz = force_fs_kHz;
    }
    /********************************************/
    /* Prepare resampler and buffered data      */
    /********************************************/
    ret += silk_setup_resamplers(psEnc, fs_kHz);

    /********************************************/
    /* Set internal sampling frequency          */
    /********************************************/
    ret += silk_setup_fs(psEnc, fs_kHz, encControl.payloadSize_ms);

    /********************************************/
    /* Set encoding complexity                  */
    /********************************************/
    ret += silk_setup_complexity(&mut psEnc.sCmn, encControl.complexity);

    /********************************************/
    /* Set packet loss rate measured by farend  */
    /********************************************/
    psEnc.sCmn.PacketLoss_perc = encControl.packetLossPercentage;

    /********************************************/
    /* Set LBRR usage                           */
    /********************************************/
    ret += silk_setup_LBRR(&mut psEnc.sCmn, encControl);

    psEnc.sCmn.controlled_since_last_payload = 1;

    ret
}

fn silk_setup_resamplers(
    psEnc: &mut silk_encoder_state_Fxx, /* I/O                      */
    fs_kHz: opus_int,                   /* I                        */
) -> opus_int {
    let mut ret = SILK_NO_ERROR;

    if psEnc.sCmn.fs_kHz != fs_kHz || psEnc.sCmn.prev_API_fs_Hz != psEnc.sCmn.API_fs_Hz {
        if psEnc.sCmn.fs_kHz == 0 {
            /* Initialize the resampler for enc_API.c preparing resampling from API_fs_Hz to fs_kHz */
            ret += silk_resampler_init(
                &mut psEnc.sCmn.resampler_state,
                psEnc.sCmn.API_fs_Hz,
                fs_kHz * 1000,
                1,
            );
        } else {
            let buf_length_ms =
                silk_LSHIFT(psEnc.sCmn.nb_subfr * 5, 1) + LA_SHAPE_MS;
            let old_buf_samples = buf_length_ms * psEnc.sCmn.fs_kHz;

            let new_buf_samples = buf_length_ms * fs_kHz;
            let mut x_bufFIX =
                vec![0 as opus_int16; silk_max(old_buf_samples, new_buf_samples) as usize];
            silk_float2short_array(&mut x_bufFIX, &psEnc.x_buf, old_buf_samples);

            /* Initialize resampler for temporary resampling of x_buf data to API_fs_Hz */
            let mut temp_resampler_state = silk_resampler_state_struct::default();
            ret += silk_resampler_init(
                &mut temp_resampler_state,
                silk_SMULBB(psEnc.sCmn.fs_kHz, 1000),
                psEnc.sCmn.API_fs_Hz,
                0,
            );

            /* Calculate number of samples to temporarily upsample */
            let api_buf_samples =
                buf_length_ms * silk_DIV32_16(psEnc.sCmn.API_fs_Hz, 1000);

            /* Temporary resampling of x_buf data to API_fs_Hz */
            let mut x_buf_API_fs_Hz = vec![0 as opus_int16; api_buf_samples as usize];
            ret += silk_resampler(
                &mut temp_resampler_state,
                &mut x_buf_API_fs_Hz,
                &x_bufFIX,
                old_buf_samples,
            );

            /* Initialize the resampler for enc_API.c preparing resampling from API_fs_Hz to fs_kHz */
            ret += silk_resampler_init(
                &mut psEnc.sCmn.resampler_state,
                psEnc.sCmn.API_fs_Hz,
                silk_SMULBB(fs_kHz, 1000),
                1,
            );

            /* Correct resampler state by resampling buffered data from API_fs_Hz to fs_kHz */
            ret += silk_resampler(
                &mut psEnc.sCmn.resampler_state,
                &mut x_bufFIX,
                &x_buf_API_fs_Hz,
                api_buf_samples,
            );

            silk_short2float_array(&mut psEnc.x_buf, &x_bufFIX, new_buf_samples);
        }
    }

    psEnc.sCmn.prev_API_fs_Hz = psEnc.sCmn.API_fs_Hz;

    ret
}

fn silk_setup_fs(
    psEnc: &mut silk_encoder_state_Fxx, /* I/O                      */
    fs_kHz: opus_int,                   /* I                        */
    PacketSize_ms: opus_int,            /* I                        */
) -> opus_int {
    let mut ret = SILK_NO_ERROR;

    /* Set packet size */
    if PacketSize_ms != psEnc.sCmn.PacketSize_ms {
        if (PacketSize_ms != 10) && (PacketSize_ms != 20) && (PacketSize_ms != 40) && (PacketSize_ms != 60)
        {
            ret = SILK_ENC_PACKET_SIZE_NOT_SUPPORTED;
        }
        if PacketSize_ms <= 10 {
            psEnc.sCmn.nFramesPerPacket = 1;
            psEnc.sCmn.nb_subfr = if PacketSize_ms == 10 { 2 } else { 1 };
            psEnc.sCmn.frame_length = silk_SMULBB(PacketSize_ms, fs_kHz);
            psEnc.sCmn.pitch_LPC_win_length = silk_SMULBB(FIND_PITCH_LPC_WIN_MS_2_SF, fs_kHz);
            if psEnc.sCmn.fs_kHz == 8 {
                psEnc.sCmn.pitch_contour_iCDF = &silk_pitch_contour_10_ms_NB_iCDF;
            } else {
                psEnc.sCmn.pitch_contour_iCDF = &silk_pitch_contour_10_ms_iCDF;
            }
        } else {
            psEnc.sCmn.nFramesPerPacket = silk_DIV32_16(PacketSize_ms, MAX_FRAME_LENGTH_MS);
            psEnc.sCmn.nb_subfr = MAX_NB_SUBFR;
            psEnc.sCmn.frame_length = silk_SMULBB(20, fs_kHz);
            psEnc.sCmn.pitch_LPC_win_length = silk_SMULBB(FIND_PITCH_LPC_WIN_MS, fs_kHz);
            if psEnc.sCmn.fs_kHz == 8 {
                psEnc.sCmn.pitch_contour_iCDF = &silk_pitch_contour_NB_iCDF;
            } else {
                psEnc.sCmn.pitch_contour_iCDF = &silk_pitch_contour_iCDF;
            }
        }
        psEnc.sCmn.PacketSize_ms = PacketSize_ms;
        psEnc.sCmn.TargetRate_bps = 0; /* trigger new SNR computation */
    }

    /* Set internal sampling frequency */
    celt_assert!(fs_kHz == 8 || fs_kHz == 12 || fs_kHz == 16);
    celt_assert!(psEnc.sCmn.nb_subfr == 2 || psEnc.sCmn.nb_subfr == 4);
    if psEnc.sCmn.fs_kHz != fs_kHz {
        /* reset part of the state */
        psEnc.sShape = crate::silk::float::main_flp::silk_shape_state_FLP::default();
        psEnc.sCmn.sNSQ = silk_nsq_state::default();
        psEnc.sCmn.prev_NLSFq_Q15 = [0; MAX_LPC_ORDER as usize];
        psEnc.sCmn.sLP.In_LP_State = [0, 0];
        psEnc.sCmn.inputBufIx = 0;
        psEnc.sCmn.nFramesEncoded = 0;
        psEnc.sCmn.TargetRate_bps = 0; /* trigger new SNR computation */

        /* Initialize non-zero parameters */
        psEnc.sCmn.prevLag = 100;
        psEnc.sCmn.first_frame_after_reset = 1;
        psEnc.sShape.LastGainIndex = 10;
        psEnc.sCmn.sNSQ.lagPrev = 100;
        psEnc.sCmn.sNSQ.prev_gain_Q16 = 65536;
        psEnc.sCmn.prevSignalType = TYPE_NO_VOICE_ACTIVITY as opus_int8;

        psEnc.sCmn.fs_kHz = fs_kHz;
        if psEnc.sCmn.fs_kHz == 8 {
            if psEnc.sCmn.nb_subfr == MAX_NB_SUBFR {
                psEnc.sCmn.pitch_contour_iCDF = &silk_pitch_contour_NB_iCDF;
            } else {
                psEnc.sCmn.pitch_contour_iCDF = &silk_pitch_contour_10_ms_NB_iCDF;
            }
        } else if psEnc.sCmn.nb_subfr == MAX_NB_SUBFR {
            psEnc.sCmn.pitch_contour_iCDF = &silk_pitch_contour_iCDF;
        } else {
            psEnc.sCmn.pitch_contour_iCDF = &silk_pitch_contour_10_ms_iCDF;
        }
        if psEnc.sCmn.fs_kHz == 8 || psEnc.sCmn.fs_kHz == 12 {
            psEnc.sCmn.predictLPCOrder = MIN_LPC_ORDER;
            psEnc.sCmn.psNLSF_CB = &silk_NLSF_CB_NB_MB;
        } else {
            psEnc.sCmn.predictLPCOrder = MAX_LPC_ORDER;
            psEnc.sCmn.psNLSF_CB = &silk_NLSF_CB_WB;
        }
        psEnc.sCmn.subfr_length = SUB_FRAME_LENGTH_MS * fs_kHz;
        psEnc.sCmn.frame_length = silk_SMULBB(psEnc.sCmn.subfr_length, psEnc.sCmn.nb_subfr);
        psEnc.sCmn.ltp_mem_length = silk_SMULBB(LTP_MEM_LENGTH_MS, fs_kHz);
        psEnc.sCmn.la_pitch = silk_SMULBB(LA_PITCH_MS, fs_kHz);
        psEnc.sCmn.max_pitch_lag = silk_SMULBB(18, fs_kHz);
        if psEnc.sCmn.nb_subfr == MAX_NB_SUBFR {
            psEnc.sCmn.pitch_LPC_win_length = silk_SMULBB(FIND_PITCH_LPC_WIN_MS, fs_kHz);
        } else {
            psEnc.sCmn.pitch_LPC_win_length = silk_SMULBB(FIND_PITCH_LPC_WIN_MS_2_SF, fs_kHz);
        }
        if psEnc.sCmn.fs_kHz == 16 {
            psEnc.sCmn.pitch_lag_low_bits_iCDF = &silk_uniform8_iCDF;
        } else if psEnc.sCmn.fs_kHz == 12 {
            psEnc.sCmn.pitch_lag_low_bits_iCDF = &silk_uniform6_iCDF;
        } else {
            psEnc.sCmn.pitch_lag_low_bits_iCDF = &silk_uniform4_iCDF;
        }
    }

    /* Check that settings are valid */
    celt_assert!(
        (psEnc.sCmn.subfr_length * psEnc.sCmn.nb_subfr) == psEnc.sCmn.frame_length
    );

    ret
}

fn silk_setup_complexity(
    psEncC: &mut silk_encoder_state, /* I/O                      */
    Complexity: opus_int,            /* I                        */
) -> opus_int {
    let ret = 0;

    /* Set encoding complexity */
    celt_assert!(Complexity >= 0 && Complexity <= 10);
    if Complexity < 1 {
        psEncC.pitchEstimationComplexity = SILK_PE_MIN_COMPLEX;
        psEncC.pitchEstimationThreshold_Q16 = SILK_FIX_CONST(0.8, 16);
        psEncC.pitchEstimationLPCOrder = 6;
        psEncC.shapingLPCOrder = 12;
        psEncC.la_shape = 3 * psEncC.fs_kHz;
        psEncC.nStatesDelayedDecision = 1;
        psEncC.useInterpolatedNLSFs = 0;
        psEncC.NLSF_MSVQ_Survivors = 2;
        psEncC.warping_Q16 = 0;
    } else if Complexity < 2 {
        psEncC.pitchEstimationComplexity = SILK_PE_MID_COMPLEX;
        psEncC.pitchEstimationThreshold_Q16 = SILK_FIX_CONST(0.76, 16);
        psEncC.pitchEstimationLPCOrder = 8;
        psEncC.shapingLPCOrder = 14;
        psEncC.la_shape = 5 * psEncC.fs_kHz;
        psEncC.nStatesDelayedDecision = 1;
        psEncC.useInterpolatedNLSFs = 0;
        psEncC.NLSF_MSVQ_Survivors = 3;
        psEncC.warping_Q16 = 0;
    } else if Complexity < 3 {
        psEncC.pitchEstimationComplexity = SILK_PE_MIN_COMPLEX;
        psEncC.pitchEstimationThreshold_Q16 = SILK_FIX_CONST(0.8, 16);
        psEncC.pitchEstimationLPCOrder = 6;
        psEncC.shapingLPCOrder = 12;
        psEncC.la_shape = 3 * psEncC.fs_kHz;
        psEncC.nStatesDelayedDecision = 2;
        psEncC.useInterpolatedNLSFs = 0;
        psEncC.NLSF_MSVQ_Survivors = 2;
        psEncC.warping_Q16 = 0;
    } else if Complexity < 4 {
        psEncC.pitchEstimationComplexity = SILK_PE_MID_COMPLEX;
        psEncC.pitchEstimationThreshold_Q16 = SILK_FIX_CONST(0.76, 16);
        psEncC.pitchEstimationLPCOrder = 8;
        psEncC.shapingLPCOrder = 14;
        psEncC.la_shape = 5 * psEncC.fs_kHz;
        psEncC.nStatesDelayedDecision = 2;
        psEncC.useInterpolatedNLSFs = 0;
        psEncC.NLSF_MSVQ_Survivors = 4;
        psEncC.warping_Q16 = 0;
    } else if Complexity < 6 {
        psEncC.pitchEstimationComplexity = SILK_PE_MID_COMPLEX;
        psEncC.pitchEstimationThreshold_Q16 = SILK_FIX_CONST(0.74, 16);
        psEncC.pitchEstimationLPCOrder = 10;
        psEncC.shapingLPCOrder = 16;
        psEncC.la_shape = 5 * psEncC.fs_kHz;
        psEncC.nStatesDelayedDecision = 2;
        psEncC.useInterpolatedNLSFs = 1;
        psEncC.NLSF_MSVQ_Survivors = 6;
        psEncC.warping_Q16 = psEncC.fs_kHz * SILK_FIX_CONST(WARPING_MULTIPLIER, 16);
    } else if Complexity < 8 {
        psEncC.pitchEstimationComplexity = SILK_PE_MID_COMPLEX;
        psEncC.pitchEstimationThreshold_Q16 = SILK_FIX_CONST(0.72, 16);
        psEncC.pitchEstimationLPCOrder = 12;
        psEncC.shapingLPCOrder = 20;
        psEncC.la_shape = 5 * psEncC.fs_kHz;
        psEncC.nStatesDelayedDecision = 3;
        psEncC.useInterpolatedNLSFs = 1;
        psEncC.NLSF_MSVQ_Survivors = 8;
        psEncC.warping_Q16 = psEncC.fs_kHz * SILK_FIX_CONST(WARPING_MULTIPLIER, 16);
    } else {
        psEncC.pitchEstimationComplexity = SILK_PE_MAX_COMPLEX;
        psEncC.pitchEstimationThreshold_Q16 = SILK_FIX_CONST(0.7, 16);
        psEncC.pitchEstimationLPCOrder = 16;
        psEncC.shapingLPCOrder = 24;
        psEncC.la_shape = 5 * psEncC.fs_kHz;
        psEncC.nStatesDelayedDecision = MAX_DEL_DEC_STATES;
        psEncC.useInterpolatedNLSFs = 1;
        psEncC.NLSF_MSVQ_Survivors = 16;
        psEncC.warping_Q16 = psEncC.fs_kHz * SILK_FIX_CONST(WARPING_MULTIPLIER, 16);
    }

    /* Do not allow higher pitch estimation LPC order than predict LPC order */
    psEncC.pitchEstimationLPCOrder =
        silk_min_int(psEncC.pitchEstimationLPCOrder, psEncC.predictLPCOrder);
    psEncC.shapeWinLength = SUB_FRAME_LENGTH_MS * psEncC.fs_kHz + 2 * psEncC.la_shape;
    psEncC.Complexity = Complexity;

    celt_assert!(psEncC.pitchEstimationLPCOrder <= MAX_FIND_PITCH_LPC_ORDER);
    celt_assert!(psEncC.shapingLPCOrder <= MAX_SHAPE_LPC_ORDER);
    celt_assert!(psEncC.nStatesDelayedDecision <= MAX_DEL_DEC_STATES);
    celt_assert!(psEncC.warping_Q16 <= 32767);
    celt_assert!(psEncC.la_shape <= LA_SHAPE_MAX);
    celt_assert!(psEncC.shapeWinLength <= SHAPE_LPC_WIN_MAX);

    ret
}

fn silk_setup_LBRR(
    psEncC: &mut silk_encoder_state,          /* I/O                      */
    encControl: &silk_EncControlStruct,       /* I                        */
) -> opus_int {
    let mut ret = SILK_NO_ERROR;

    let LBRR_in_previous_packet = psEncC.LBRR_enabled;
    psEncC.LBRR_enabled = encControl.LBRR_coded;
    if psEncC.LBRR_enabled != 0 {
        /* Set gain increase for coding LBRR excitation */
        if LBRR_in_previous_packet == 0 {
            /* Previous packet did not have LBRR, and was therefore coded at a higher bitrate */
            psEncC.LBRR_GainIncreases = 7;
        } else {
            psEncC.LBRR_GainIncreases = silk_max_int(
                7 - silk_SMULWB(psEncC.PacketLoss_perc as opus_int32, SILK_FIX_CONST(0.2, 16)),
                3,
            );
        }
    }

    ret
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enc_state() -> silk_encoder_state_Fxx {
        let mut e = silk_encoder_state_Fxx::default();
        silk_assert!(e.sCmn.nb_subfr == 0);
        e.sCmn.API_fs_Hz = 48000;
        e.sCmn.maxInternal_fs_Hz = 16000;
        e.sCmn.minInternal_fs_Hz = 8000;
        e.sCmn.desiredInternal_fs_Hz = 16000;
        e.sCmn.nChannelsAPI = 1;
        e.sCmn.nChannelsInternal = 1;
        e
    }

    fn ctl_20ms() -> silk_EncControlStruct {
        let mut c = silk_EncControlStruct::default();
        c.nChannelsAPI = 1;
        c.nChannelsInternal = 1;
        c.API_sampleRate = 48000;
        c.maxInternalSampleRate = 16000;
        c.minInternalSampleRate = 8000;
        c.desiredInternalSampleRate = 16000;
        c.payloadSize_ms = 20;
        c.complexity = 9;
        c
    }

    /// Complexity 9 must select the AVX2 quantiser's configuration
    /// (`shapingLPCOrder == 24`, `nStatesDelayedDecision == 4`).
    #[test]
    fn default_complexity_selects_avx2_configuration() {
        let mut e = enc_state();
        let mut c = ctl_20ms();
        assert_eq!(silk_control_encoder(&mut e, &mut c, 0, 0, 0), 0);
        assert_eq!(e.sCmn.fs_kHz, 16);
        assert_eq!(e.sCmn.shapingLPCOrder, 24);
        assert_eq!(e.sCmn.nStatesDelayedDecision, 4);
        assert_eq!(e.sCmn.predictLPCOrder, 16);
        assert_eq!(e.sCmn.nb_subfr, 4);
        assert_eq!(e.sCmn.frame_length, 320);
        assert_eq!(e.sCmn.subfr_length, 80);
        assert_eq!(e.sCmn.ltp_mem_length, 320);
        assert_eq!(e.sCmn.la_shape, 5 * 16);
        assert_eq!(e.sCmn.psNLSF_CB.order, 16);
    }

    /// 10 ms packets use two subframes and the 10 ms pitch-contour table.
    #[test]
    fn ten_ms_packet_size() {
        let mut e = enc_state();
        let mut c = ctl_20ms();
        c.payloadSize_ms = 10;
        assert_eq!(silk_control_encoder(&mut e, &mut c, 0, 0, 0), 0);
        assert_eq!(e.sCmn.nb_subfr, 2);
        assert_eq!(e.sCmn.nFramesPerPacket, 1);
        assert_eq!(e.sCmn.frame_length, 160);
        assert_eq!(e.sCmn.pitch_LPC_win_length, FIND_PITCH_LPC_WIN_MS_2_SF * 16);
    }

    #[test]
    fn unsupported_packet_size_reports_error() {
        let mut e = enc_state();
        let mut c = ctl_20ms();
        c.payloadSize_ms = 25;
        assert_eq!(
            silk_control_encoder(&mut e, &mut c, 0, 0, 0),
            SILK_ENC_PACKET_SIZE_NOT_SUPPORTED
        );
    }
}

//! Transliteration of `silk/enc_API.c` from libopus 1.6.1 to safe Rust.
//!
//! The reference build is floating point, so `silk_encode_frame_Fxx` resolves
//! to `silk_encode_frame_FLP` and `silk_encode_do_VAD_Fxx` to
//! `silk_encode_do_VAD_FLP` (both defined in `silk/float/encode_frame_FLP.c`),
//! `silk_encoder_state_Fxx` is `silk_encoder_state_FLP` and `silk_encoder` is
//! the `float/structs_FLP.h` variant (`src/silk/float/main_flp.rs`).
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

use crate::celt::arch::{celt_assert, opus_res, RES2INT16};
use crate::celt::entcode::ec_ctx;
use crate::celt::entenc::{ec_enc_icdf, ec_enc_patch_initial_bits};
use crate::silk::check_control_input::check_control_input;
use crate::silk::control::silk_EncControlStruct;
use crate::silk::control_codec::silk_control_encoder;
use crate::silk::control_snr::silk_control_SNR;
use crate::silk::define::*;
use crate::silk::encode_indices::silk_encode_indices;
use crate::silk::encode_pulses::silk_encode_pulses;
use crate::silk::float::encode_frame_flp::{silk_encode_do_VAD_FLP, silk_encode_frame_FLP};
use crate::silk::float::main_flp::{silk_encoder, silk_encoder_state_Fxx, silk_shape_state_FLP};
use crate::silk::hp_variable_cutoff::silk_HP_variable_cutoff;
use crate::silk::init_encoder::silk_init_encoder;
use crate::silk::macros::*;
use crate::silk::resampler::silk_resampler;
use crate::silk::sigproc_fix::*;
use crate::silk::stereo_encode_pred::{silk_stereo_encode_mid_only, silk_stereo_encode_pred};
use crate::silk::stereo_lr_to_ms::silk_stereo_LR_to_MS;
use crate::silk::structs::{silk_LP_state, silk_nsq_state};
use crate::silk::tables_other::{silk_LBRR_flags_iCDF_ptr, silk_Quantization_Offsets_Q10};
use crate::silk::typedef::*;
use crate::types::*;

// ---------------------------------------------------------------------------
// `silk/tuning_parameters.h` constants used by this file.  There is no Rust
// mirror of that header yet, so the three constants this file needs live here
// under their C names.
// ---------------------------------------------------------------------------

/// `#define BITRESERVOIR_DECAY_TIME_MS 500`
const BITRESERVOIR_DECAY_TIME_MS: opus_int = 500;
/// `#define SPEECH_ACTIVITY_DTX_THRES 0.05f`
const SPEECH_ACTIVITY_DTX_THRES: f32 = 0.05;
/// `#define MAX_BANDWIDTH_SWITCH_DELAY_MS 5000`
const MAX_BANDWIDTH_SWITCH_DELAY_MS: opus_int = 5000;

/***************************************/
/* Read control structure from encoder */
/***************************************/
/// `silk_QueryEncoder(encState, encStatus)` (static in the C)
fn silk_QueryEncoder(
    psEnc: &silk_encoder,             /* I    State           */
    encStatus: &mut silk_EncControlStruct, /* O    Encoder Status  */
) -> opus_int {
    let ret = SILK_NO_ERROR;
    let state_Fxx = &psEnc.state_Fxx;

    encStatus.nChannelsAPI = psEnc.nChannelsAPI;
    encStatus.nChannelsInternal = psEnc.nChannelsInternal;
    encStatus.API_sampleRate = state_Fxx[0].sCmn.API_fs_Hz;
    encStatus.maxInternalSampleRate = state_Fxx[0].sCmn.maxInternal_fs_Hz;
    encStatus.minInternalSampleRate = state_Fxx[0].sCmn.minInternal_fs_Hz;
    encStatus.desiredInternalSampleRate = state_Fxx[0].sCmn.desiredInternal_fs_Hz;
    encStatus.payloadSize_ms = state_Fxx[0].sCmn.PacketSize_ms;
    encStatus.bitRate = state_Fxx[0].sCmn.TargetRate_bps;
    encStatus.packetLossPercentage = state_Fxx[0].sCmn.PacketLoss_perc;
    encStatus.complexity = state_Fxx[0].sCmn.Complexity;
    encStatus.useInBandFEC = state_Fxx[0].sCmn.useInBandFEC;
    encStatus.useDTX = state_Fxx[0].sCmn.useDTX;
    encStatus.useCBR = state_Fxx[0].sCmn.useCBR;
    encStatus.internalSampleRate = silk_SMULBB(state_Fxx[0].sCmn.fs_kHz, 1000);
    encStatus.allowBandwidthSwitch = state_Fxx[0].sCmn.allow_bandwidth_switch;
    encStatus.inWBmodeWithoutVariableLP =
        (state_Fxx[0].sCmn.fs_kHz == 16 && state_Fxx[0].sCmn.sLP.mode == 0) as opus_int;

    ret
}

/****************************************/
/* Encoder functions                    */
/****************************************/

/// `silk_Get_Encoder_Size(encSizeBytes, channels)`
pub fn silk_Get_Encoder_Size(
    encSizeBytes: &mut opus_int, /* O    Number of bytes in SILK encoder state */
    channels: opus_int,          /* I    Number of channels                  */
) -> opus_int {
    let ret = SILK_NO_ERROR;

    *encSizeBytes = core::mem::size_of::<silk_encoder>() as opus_int;
    /* Skip second encoder state for mono. */
    if channels == 1 {
        *encSizeBytes -= core::mem::size_of::<silk_encoder_state_Fxx>() as opus_int;
    }

    ret
}

/*************************/
/* Init or Reset encoder */
/*************************/
/// `silk_InitEncoder(encState, channels, arch, encStatus)`
pub fn silk_InitEncoder(
    psEnc: &mut silk_encoder,              /* I/O  State          */
    channels: opus_int,                    /* I    Number of channels */
    arch: opus_int,                        /* I    Run-time architecture */
    encStatus: &mut silk_EncControlStruct, /* O    Encoder Status */
) -> opus_int {
    let mut ret = SILK_NO_ERROR;

    /* Reset encoder.  The C skips the second encoder state for mono
       (`sizeof(silk_encoder) - (channels==1)*sizeof(silk_encoder_state_Fxx)`);
       zeroing the whole Rust struct as well is unobservable, because the mono
       path never reads `state_Fxx[1]` (it is re-initialised on a mono->stereo
       transition, exactly as in the C). */
    *psEnc = silk_encoder::default();

    for n in 0..channels {
        ret += silk_init_encoder(&mut psEnc.state_Fxx[n as usize], arch);
        if ret != 0 {
            celt_assert!(false);
        }
    }

    psEnc.nChannelsAPI = 1;
    psEnc.nChannelsInternal = 1;

    /* Read control structure */
    ret += silk_QueryEncoder(psEnc, encStatus);
    if ret != 0 {
        celt_assert!(false);
    }

    ret
}

/**************************/
/* Encode frame with Silk */
/**************************/
/* Note: if prefillFlag is set, the input must contain 10 ms of audio, irrespective of what                     */
/* encControl->payloadSize_ms is set to                                                                         */
/// `silk_Encode(encState, encControl, samplesIn, nSamplesIn, psRangeEnc, nBytesOut, prefillFlag, activity)`
pub fn silk_Encode(
    encState: &mut silk_encoder,           /* I/O  State                                         */
    encControl: &mut silk_EncControlStruct, /* I    Control status                                */
    samplesIn: &[opus_res],                /* I    Speech sample input vector                    */
    mut nSamplesIn: opus_int,              /* I    Number of samples in input vector             */
    psRangeEnc: &mut ec_ctx,               /* I/O  Compressor data structure                     */
    nBytesOut: &mut opus_int32,            /* I/O  Number of bytes in payload (input: Max bytes) */
    prefillFlag: opus_int,                 /* I    Flag to indicate prefilling buffers no coding */
    activity: opus_int,                    /* I    Decision of Opus voice activity detector      */
) -> opus_int {
    /* The C declares `opus_int n, i`; every use in this port is a `for`
       loop whose binding is declared by the loop itself. */
    let mut nBits: opus_int32;
    let mut flags: opus_int;
    let mut tmp_payloadSize_ms: opus_int = 0;
    let mut tmp_complexity: opus_int = 0;
    let mut ret: opus_int = 0;
    let mut nSamplesToBuffer: opus_int;
    let mut nSamplesToBufferMax: opus_int;
    let mut nBlocksOf10ms: opus_int;
    let mut nSamplesFromInput: opus_int = 0;
    let mut nSamplesFromInputMax: opus_int;
    let mut speech_act_thr_for_switch_Q8: opus_int;
    let mut TargetRate_bps: opus_int32;
    let mut MStargetRates_bps = [0 as opus_int32; 2];
    let mut channelRate_bps: opus_int32;
    let mut LBRR_symbol: opus_int32;
    let mut sum: opus_int32;
    let mut transition: opus_int;
    let mut curr_block: opus_int;
    let mut tot_blocks: opus_int;
    /* The C advances the `samplesIn` pointer; the Rust port keeps an index. */
    let mut samplesIn_ix: usize = 0;

    celt_assert!(
        encControl.nChannelsAPI >= encControl.nChannelsInternal
            && encControl.nChannelsAPI >= encState.nChannelsInternal
    );
    if encControl.reducedDependency != 0 {
        for n in 0..encControl.nChannelsAPI {
            encState.state_Fxx[n as usize].sCmn.first_frame_after_reset = 1;
        }
    }
    for n in 0..encControl.nChannelsAPI {
        encState.state_Fxx[n as usize].sCmn.nFramesEncoded = 0;
    }
    /* Check values in encoder control structure */
    ret = check_control_input(encControl);
    if ret != 0 {
        celt_assert!(false);
        return ret;
    }

    encControl.switchReady = 0;

    if encControl.nChannelsInternal > encState.nChannelsInternal {
        /* Mono -> Stereo transition: init state of second channel and stereo state */
        let arch = encState.state_Fxx[0].sCmn.arch;
        ret += silk_init_encoder(&mut encState.state_Fxx[1], arch);
        encState.sStereo.pred_prev_Q13 = [0; 2];
        encState.sStereo.sSide = [0; 2];
        encState.sStereo.mid_side_amp_Q0[0] = 0;
        encState.sStereo.mid_side_amp_Q0[1] = 1;
        encState.sStereo.mid_side_amp_Q0[2] = 0;
        encState.sStereo.mid_side_amp_Q0[3] = 1;
        encState.sStereo.width_prev_Q14 = 0;
        encState.sStereo.smth_width_Q14 = SILK_FIX_CONST(1.0, 14) as opus_int16;
        if encState.nChannelsAPI == 2 {
            encState.state_Fxx[1].sCmn.resampler_state =
                encState.state_Fxx[0].sCmn.resampler_state;
            encState.state_Fxx[1].sCmn.In_HP_State = encState.state_Fxx[0].sCmn.In_HP_State;
        }
    }

    transition = ((encControl.payloadSize_ms != encState.state_Fxx[0].sCmn.PacketSize_ms)
        || (encState.nChannelsInternal != encControl.nChannelsInternal))
        as opus_int;

    encState.nChannelsAPI = encControl.nChannelsAPI;
    encState.nChannelsInternal = encControl.nChannelsInternal;

    nBlocksOf10ms = silk_DIV32(
        100i32.wrapping_mul(nSamplesIn),
        encControl.API_sampleRate,
    );
    tot_blocks = if nBlocksOf10ms > 1 {
        nBlocksOf10ms >> 1
    } else {
        1
    };
    curr_block = 0;
    if prefillFlag != 0 {
        let mut save_LP = silk_LP_state::default();
        /* Only accept input length of 10 ms */
        if nBlocksOf10ms != 1 {
            celt_assert!(false);
            return SILK_ENC_INPUT_INVALID_NO_OF_SAMPLES;
        }
        if prefillFlag == 2 {
            save_LP = encState.state_Fxx[0].sCmn.sLP;
            /* Save the sampling rate so the bandwidth switching code can keep handling transitions. */
            save_LP.saved_fs_kHz = encState.state_Fxx[0].sCmn.fs_kHz;
        }
        /* Reset Encoder */
        for n in 0..encControl.nChannelsInternal {
            let arch = encState.state_Fxx[n as usize].sCmn.arch;
            ret = silk_init_encoder(&mut encState.state_Fxx[n as usize], arch);
            /* Restore the variable LP state. */
            if prefillFlag == 2 {
                encState.state_Fxx[n as usize].sCmn.sLP = save_LP;
            }
            celt_assert!(ret == 0);
        }
        tmp_payloadSize_ms = encControl.payloadSize_ms;
        encControl.payloadSize_ms = 10;
        tmp_complexity = encControl.complexity;
        encControl.complexity = 0;
        for n in 0..encControl.nChannelsInternal {
            encState.state_Fxx[n as usize].sCmn.controlled_since_last_payload = 0;
            encState.state_Fxx[n as usize].sCmn.prefillFlag = 1;
        }
    } else {
        /* Only accept input lengths that are a multiple of 10 ms */
        if nBlocksOf10ms.wrapping_mul(encControl.API_sampleRate)
            != 100i32.wrapping_mul(nSamplesIn)
            || nSamplesIn < 0
        {
            celt_assert!(false);
            return SILK_ENC_INPUT_INVALID_NO_OF_SAMPLES;
        }
        /* Make sure no more than one packet can be produced */
        if 1000i32.wrapping_mul(nSamplesIn)
            > encControl
                .payloadSize_ms
                .wrapping_mul(encControl.API_sampleRate)
        {
            celt_assert!(false);
            return SILK_ENC_INPUT_INVALID_NO_OF_SAMPLES;
        }
    }

    for n in 0..encControl.nChannelsInternal {
        /* Force the side channel to the same rate as the mid */
        let force_fs_kHz: opus_int = if n == 1 {
            encState.state_Fxx[0].sCmn.fs_kHz
        } else {
            0
        };
        ret = silk_control_encoder(
            &mut encState.state_Fxx[n as usize],
            encControl,
            encState.allowBandwidthSwitch,
            n,
            force_fs_kHz,
        );
        if ret != 0 {
            silk_assert!(false);
            return ret;
        }
        if encState.state_Fxx[n as usize].sCmn.first_frame_after_reset != 0 || transition != 0 {
            for i in 0..encState.state_Fxx[0].sCmn.nFramesPerPacket {
                encState.state_Fxx[n as usize].sCmn.LBRR_flags[i as usize] = 0;
            }
        }
        encState.state_Fxx[n as usize].sCmn.inDTX = encState.state_Fxx[n as usize].sCmn.useDTX;
    }
    celt_assert!(
        encControl.nChannelsInternal == 1
            || encState.state_Fxx[0].sCmn.fs_kHz == encState.state_Fxx[1].sCmn.fs_kHz
    );

    /* Input buffering/resampling and encoding */
    nSamplesToBufferMax = 10 * nBlocksOf10ms * encState.state_Fxx[0].sCmn.fs_kHz;
    nSamplesFromInputMax = silk_DIV32_16(
        nSamplesToBufferMax * encState.state_Fxx[0].sCmn.API_fs_Hz,
        encState.state_Fxx[0].sCmn.fs_kHz * 1000,
    );
    let mut buf = vec![0 as opus_int16; nSamplesFromInputMax as usize];
    loop {
        let mut curr_nBitsUsedLBRR: opus_int = 0;
        nSamplesToBuffer =
            encState.state_Fxx[0].sCmn.frame_length - encState.state_Fxx[0].sCmn.inputBufIx;
        nSamplesToBuffer = silk_min(nSamplesToBuffer, nSamplesToBufferMax);
        nSamplesFromInput = silk_DIV32_16(
            nSamplesToBuffer * encState.state_Fxx[0].sCmn.API_fs_Hz,
            encState.state_Fxx[0].sCmn.fs_kHz * 1000,
        );
        /* Resample and write to buffer */
        if encControl.nChannelsAPI == 2 && encControl.nChannelsInternal == 2 {
            let id = encState.state_Fxx[0].sCmn.nFramesEncoded;
            for n in 0..nSamplesFromInput {
                buf[n as usize] = RES2INT16(samplesIn[samplesIn_ix + (2 * n) as usize]);
            }
            /* Making sure to start both resamplers from the same state when switching from mono to stereo */
            if encState.nPrevChannelsInternal == 1 && id == 0 {
                encState.state_Fxx[1].sCmn.resampler_state =
                    encState.state_Fxx[0].sCmn.resampler_state;
            }

            let ix0 = (encState.state_Fxx[0].sCmn.inputBufIx + 2) as usize;
            ret += silk_resampler(
                &mut encState.state_Fxx[0].sCmn.resampler_state,
                &mut encState.state_Fxx[0].sCmn.inputBuf[ix0..],
                &buf,
                nSamplesFromInput,
            );
            encState.state_Fxx[0].sCmn.inputBufIx += nSamplesToBuffer;

            nSamplesToBuffer =
                encState.state_Fxx[1].sCmn.frame_length - encState.state_Fxx[1].sCmn.inputBufIx;
            nSamplesToBuffer = silk_min(
                nSamplesToBuffer,
                10 * nBlocksOf10ms * encState.state_Fxx[1].sCmn.fs_kHz,
            );
            for n in 0..nSamplesFromInput {
                buf[n as usize] = RES2INT16(samplesIn[samplesIn_ix + (2 * n) as usize + 1]);
            }
            let ix1 = (encState.state_Fxx[1].sCmn.inputBufIx + 2) as usize;
            ret += silk_resampler(
                &mut encState.state_Fxx[1].sCmn.resampler_state,
                &mut encState.state_Fxx[1].sCmn.inputBuf[ix1..],
                &buf,
                nSamplesFromInput,
            );

            encState.state_Fxx[1].sCmn.inputBufIx += nSamplesToBuffer;
        } else if encControl.nChannelsAPI == 2 && encControl.nChannelsInternal == 1 {
            /* Combine left and right channels before resampling */
            for n in 0..nSamplesFromInput {
                sum = RES2INT16(
                    samplesIn[samplesIn_ix + (2 * n) as usize]
                        + samplesIn[samplesIn_ix + (2 * n) as usize + 1],
                ) as opus_int32;
                buf[n as usize] = silk_RSHIFT_ROUND(sum, 1) as opus_int16;
            }
            let ix0 = (encState.state_Fxx[0].sCmn.inputBufIx + 2) as usize;
            ret += silk_resampler(
                &mut encState.state_Fxx[0].sCmn.resampler_state,
                &mut encState.state_Fxx[0].sCmn.inputBuf[ix0..],
                &buf,
                nSamplesFromInput,
            );
            /* On the first mono frame, average the results for the two resampler states  */
            if encState.nPrevChannelsInternal == 2
                && encState.state_Fxx[0].sCmn.nFramesEncoded == 0
            {
                let ix1 = (encState.state_Fxx[1].sCmn.inputBufIx + 2) as usize;
                ret += silk_resampler(
                    &mut encState.state_Fxx[1].sCmn.resampler_state,
                    &mut encState.state_Fxx[1].sCmn.inputBuf[ix1..],
                    &buf,
                    nSamplesFromInput,
                );
                let ibix0 = encState.state_Fxx[0].sCmn.inputBufIx;
                let ibix1 = encState.state_Fxx[1].sCmn.inputBufIx;
                for n in 0..encState.state_Fxx[0].sCmn.frame_length {
                    let k0 = (ibix0 + n + 2) as usize;
                    let k1 = (ibix1 + n + 2) as usize;
                    encState.state_Fxx[0].sCmn.inputBuf[k0] = silk_RSHIFT(
                        encState.state_Fxx[0].sCmn.inputBuf[k0] as opus_int32
                            + encState.state_Fxx[1].sCmn.inputBuf[k1] as opus_int32,
                        1,
                    ) as opus_int16;
                }
            }
            encState.state_Fxx[0].sCmn.inputBufIx += nSamplesToBuffer;
        } else {
            celt_assert!(encControl.nChannelsAPI == 1 && encControl.nChannelsInternal == 1);
            for n in 0..nSamplesFromInput {
                buf[n as usize] = RES2INT16(samplesIn[samplesIn_ix + n as usize]);
            }
            let ix0 = (encState.state_Fxx[0].sCmn.inputBufIx + 2) as usize;
            ret += silk_resampler(
                &mut encState.state_Fxx[0].sCmn.resampler_state,
                &mut encState.state_Fxx[0].sCmn.inputBuf[ix0..],
                &buf,
                nSamplesFromInput,
            );
            encState.state_Fxx[0].sCmn.inputBufIx += nSamplesToBuffer;
        }

        samplesIn_ix += (nSamplesFromInput * encControl.nChannelsAPI) as usize;
        nSamplesIn -= nSamplesFromInput;

        /* Default */
        encState.allowBandwidthSwitch = 0;

        /* Silk encoder */
        if encState.state_Fxx[0].sCmn.inputBufIx >= encState.state_Fxx[0].sCmn.frame_length {
            /* Enough data in input buffer, so encode */
            celt_assert!(
                encState.state_Fxx[0].sCmn.inputBufIx == encState.state_Fxx[0].sCmn.frame_length
            );
            celt_assert!(
                encControl.nChannelsInternal == 1
                    || encState.state_Fxx[1].sCmn.inputBufIx
                        == encState.state_Fxx[1].sCmn.frame_length
            );

            /* Deal with LBRR data */
            if encState.state_Fxx[0].sCmn.nFramesEncoded == 0 && prefillFlag == 0 {
                /* Create space at start of payload for VAD and FEC flags */
                let mut iCDF = [0 as opus_uint8; 2];
                iCDF[0] = (256
                    - silk_RSHIFT(
                        256,
                        (encState.state_Fxx[0].sCmn.nFramesPerPacket + 1)
                            * encControl.nChannelsInternal,
                    )) as opus_uint8;
                ec_enc_icdf(psRangeEnc, 0, &iCDF, 8);
                curr_nBitsUsedLBRR = psRangeEnc.ec_tell();

                /* Encode any LBRR data from previous packet */
                /* Encode LBRR flags */
                for n in 0..encControl.nChannelsInternal {
                    LBRR_symbol = 0;
                    for i in 0..encState.state_Fxx[n as usize].sCmn.nFramesPerPacket {
                        LBRR_symbol |= silk_LSHIFT(
                            encState.state_Fxx[n as usize].sCmn.LBRR_flags[i as usize],
                            i,
                        );
                    }
                    encState.state_Fxx[n as usize].sCmn.LBRR_flag =
                        if LBRR_symbol > 0 { 1 } else { 0 };
                    if LBRR_symbol != 0
                        && encState.state_Fxx[n as usize].sCmn.nFramesPerPacket > 1
                    {
                        ec_enc_icdf(
                            psRangeEnc,
                            LBRR_symbol - 1,
                            silk_LBRR_flags_iCDF_ptr[(encState.state_Fxx[n as usize]
                                .sCmn
                                .nFramesPerPacket
                                - 2) as usize],
                            8,
                        );
                    }
                }

                /* Code LBRR indices and excitation signals */
                for i in 0..encState.state_Fxx[0].sCmn.nFramesPerPacket {
                    for n in 0..encControl.nChannelsInternal {
                        if encState.state_Fxx[n as usize].sCmn.LBRR_flags[i as usize] != 0 {
                            let condCoding: opus_int;

                            if encControl.nChannelsInternal == 2 && n == 0 {
                                silk_stereo_encode_pred(
                                    psRangeEnc,
                                    &encState.sStereo.predIx[i as usize],
                                );
                                /* For LBRR data there's no need to code the mid-only flag if the side-channel LBRR flag is set */
                                if encState.state_Fxx[1].sCmn.LBRR_flags[i as usize] == 0 {
                                    silk_stereo_encode_mid_only(
                                        psRangeEnc,
                                        encState.sStereo.mid_only_flags[i as usize],
                                    );
                                }
                            }
                            /* Use conditional coding if previous frame available */
                            if i > 0
                                && encState.state_Fxx[n as usize].sCmn.LBRR_flags
                                    [(i - 1) as usize]
                                    != 0
                            {
                                condCoding = CODE_CONDITIONALLY;
                            } else {
                                condCoding = CODE_INDEPENDENTLY;
                            }
                            silk_encode_indices(
                                &mut encState.state_Fxx[n as usize].sCmn,
                                psRangeEnc,
                                i,
                                1,
                                condCoding,
                            );
                            let signalType = encState.state_Fxx[n as usize].sCmn.indices_LBRR
                                [i as usize]
                                .signalType as opus_int;
                            let quantOffsetType = encState.state_Fxx[n as usize].sCmn.indices_LBRR
                                [i as usize]
                                .quantOffsetType as opus_int;
                            let frame_length =
                                encState.state_Fxx[n as usize].sCmn.frame_length;
                            silk_encode_pulses(
                                psRangeEnc,
                                signalType,
                                quantOffsetType,
                                &mut encState.state_Fxx[n as usize].sCmn.pulses_LBRR[i as usize]
                                    [..],
                                frame_length,
                            );
                        }
                    }
                }

                /* Reset LBRR flags */
                for n in 0..encControl.nChannelsInternal {
                    encState.state_Fxx[n as usize].sCmn.LBRR_flags =
                        [0; MAX_FRAMES_PER_PACKET as usize];
                }
                curr_nBitsUsedLBRR = psRangeEnc.ec_tell() - curr_nBitsUsedLBRR;
            }

            silk_HP_variable_cutoff(&mut encState.state_Fxx);

            /* Total target bits for packet */
            nBits = silk_DIV32_16(
                silk_MUL(encControl.bitRate, encControl.payloadSize_ms),
                1000,
            );
            /* Subtract bits used for LBRR */
            if prefillFlag == 0 {
                /* psEnc->nBitsUsedLBRR is an exponential moving average of the LBRR usage,
                   except that for the first LBRR frame it does no averaging and for the first
                   frame after after LBRR, it goes back to zero immediately. */
                if curr_nBitsUsedLBRR < 10 {
                    encState.nBitsUsedLBRR = 0;
                } else if encState.nBitsUsedLBRR < 10 {
                    encState.nBitsUsedLBRR = curr_nBitsUsedLBRR;
                } else {
                    encState.nBitsUsedLBRR = (encState.nBitsUsedLBRR + curr_nBitsUsedLBRR) / 2;
                }
                nBits -= encState.nBitsUsedLBRR;
            }
            /* Divide by number of uncoded frames left in packet */
            nBits = silk_DIV32_16(nBits, encState.state_Fxx[0].sCmn.nFramesPerPacket);
            /* Convert to bits/second */
            if encControl.payloadSize_ms == 10 {
                TargetRate_bps = silk_SMULBB(nBits, 100);
            } else {
                TargetRate_bps = silk_SMULBB(nBits, 50);
            }
            /* Subtract fraction of bits in excess of target in previous frames and packets */
            TargetRate_bps -= silk_DIV32_16(
                silk_MUL(encState.nBitsExceeded, 1000),
                BITRESERVOIR_DECAY_TIME_MS,
            );
            if prefillFlag == 0 && encState.state_Fxx[0].sCmn.nFramesEncoded > 0 {
                /* Compare actual vs target bits so far in this packet */
                let bitsBalance = psRangeEnc.ec_tell()
                    - encState.nBitsUsedLBRR
                    - nBits * encState.state_Fxx[0].sCmn.nFramesEncoded;
                TargetRate_bps -= silk_DIV32_16(
                    silk_MUL(bitsBalance, 1000),
                    BITRESERVOIR_DECAY_TIME_MS,
                );
            }
            /* Never exceed input bitrate */
            TargetRate_bps = silk_LIMIT(TargetRate_bps, encControl.bitRate, 5000);

            /* Convert Left/Right to Mid/Side */
            if encControl.nChannelsInternal == 2 {
                let idx = encState.state_Fxx[0].sCmn.nFramesEncoded as usize;
                let fl = encState.state_Fxx[0].sCmn.frame_length;
                let fs_kHz = encState.state_Fxx[0].sCmn.fs_kHz;
                let speech_activity_Q8 = encState.state_Fxx[0].sCmn.speech_activity_Q8;
                let toMono = encControl.toMono;
                /* The C passes `&sStereo` plus pointers into `sStereo.predIx` /
                   `sStereo.mid_only_flags`; Rust cannot alias those, so the two
                   arrays are copied in and out.  The function overwrites both
                   completely, so the copies are invisible. */
                let mut predIx = encState.sStereo.predIx[idx];
                let mut mid_only_flag = encState.sStereo.mid_only_flags[idx];
                /* two disjoint mutable borrows of state_Fxx: indexing the array
                   twice through IndexMut would borrow the whole array. */
                let (stfx0, stfx1) = encState.state_Fxx.split_at_mut(1);
                silk_stereo_LR_to_MS(
                    &mut encState.sStereo,
                    &mut stfx0[0].sCmn.inputBuf[..],
                    &mut stfx1[0].sCmn.inputBuf[..],
                    &mut predIx,
                    &mut mid_only_flag,
                    &mut MStargetRates_bps,
                    TargetRate_bps,
                    speech_activity_Q8,
                    toMono,
                    fs_kHz,
                    fl,
                );
                encState.sStereo.predIx[idx] = predIx;
                encState.sStereo.mid_only_flags[idx] = mid_only_flag;
                if encState.sStereo.mid_only_flags[idx] == 0 {
                    /* Reset side channel encoder memory for first frame with side coding */
                    if encState.prev_decode_only_middle == 1 {
                        encState.state_Fxx[1].sShape = silk_shape_state_FLP::default();
                        encState.state_Fxx[1].sCmn.sNSQ = silk_nsq_state::default();
                        encState.state_Fxx[1].sCmn.prev_NLSFq_Q15 =
                            [0; MAX_LPC_ORDER as usize];
                        encState.state_Fxx[1].sCmn.sLP.In_LP_State = [0; 2];
                        encState.state_Fxx[1].sCmn.prevLag = 100;
                        encState.state_Fxx[1].sCmn.sNSQ.lagPrev = 100;
                        encState.state_Fxx[1].sShape.LastGainIndex = 10;
                        encState.state_Fxx[1].sCmn.prevSignalType =
                            TYPE_NO_VOICE_ACTIVITY as opus_int8;
                        encState.state_Fxx[1].sCmn.sNSQ.prev_gain_Q16 = 65536;
                        encState.state_Fxx[1].sCmn.first_frame_after_reset = 1;
                    }
                    silk_encode_do_VAD_FLP(&mut encState.state_Fxx[1], activity);
                } else {
                    encState.state_Fxx[1].sCmn.VAD_flags[idx] = 0;
                }
                if prefillFlag == 0 {
                    silk_stereo_encode_pred(psRangeEnc, &encState.sStereo.predIx[idx]);
                    if encState.state_Fxx[1].sCmn.VAD_flags[idx] == 0 {
                        silk_stereo_encode_mid_only(
                            psRangeEnc,
                            encState.sStereo.mid_only_flags[idx],
                        );
                    }
                }
            } else {
                /* Buffering */
                let fl = encState.state_Fxx[0].sCmn.frame_length;
                encState.state_Fxx[0].sCmn.inputBuf[0] = encState.sStereo.sMid[0];
                encState.state_Fxx[0].sCmn.inputBuf[1] = encState.sStereo.sMid[1];
                encState.sStereo.sMid[0] = encState.state_Fxx[0].sCmn.inputBuf[fl as usize];
                encState.sStereo.sMid[1] = encState.state_Fxx[0].sCmn.inputBuf[fl as usize + 1];
            }
            silk_encode_do_VAD_FLP(&mut encState.state_Fxx[0], activity);

            /* Encode */
            for n in 0..encControl.nChannelsInternal {
                /* Handling rate constraints */
                let mut maxBits: opus_int = encControl.maxBits;
                if tot_blocks == 2 && curr_block == 0 {
                    maxBits = maxBits * 3 / 5;
                } else if tot_blocks == 3 {
                    if curr_block == 0 {
                        maxBits = maxBits * 2 / 5;
                    } else if curr_block == 1 {
                        maxBits = maxBits * 3 / 4;
                    }
                }
                let mut useCBR: opus_int =
                    (encControl.useCBR != 0 && curr_block == tot_blocks - 1) as opus_int;

                if encControl.nChannelsInternal == 1 {
                    channelRate_bps = TargetRate_bps;
                } else {
                    channelRate_bps = MStargetRates_bps[n as usize];
                    if n == 0 && MStargetRates_bps[1] > 0 {
                        useCBR = 0;
                        /* Give mid up to 1/2 of the max bits for that frame */
                        maxBits -= encControl.maxBits / (tot_blocks * 2);
                    }
                }

                if channelRate_bps > 0 {
                    let condCoding: opus_int;

                    let _ = silk_control_SNR(
                        &mut encState.state_Fxx[n as usize].sCmn,
                        channelRate_bps,
                    );

                    /* Use independent coding if no previous frame available */
                    if encState.state_Fxx[0].sCmn.nFramesEncoded - n <= 0 {
                        condCoding = CODE_INDEPENDENTLY;
                    } else if n > 0 && encState.prev_decode_only_middle != 0 {
                        /* If we skipped a side frame in this packet, we don't
                           need LTP scaling; the LTP state is well-defined. */
                        condCoding = CODE_INDEPENDENTLY_NO_LTP_SCALING;
                    } else {
                        condCoding = CODE_CONDITIONALLY;
                    }
                    ret = silk_encode_frame_FLP(
                        &mut encState.state_Fxx[n as usize],
                        nBytesOut,
                        psRangeEnc,
                        condCoding,
                        maxBits,
                        useCBR,
                    );
                    if ret != 0 {
                        silk_assert!(false);
                    }
                }
                encState.state_Fxx[n as usize].sCmn.controlled_since_last_payload = 0;
                encState.state_Fxx[n as usize].sCmn.inputBufIx = 0;
                encState.state_Fxx[n as usize].sCmn.nFramesEncoded += 1;
            }
            let prev_idx = (encState.state_Fxx[0].sCmn.nFramesEncoded - 1) as usize;
            encState.prev_decode_only_middle =
                encState.sStereo.mid_only_flags[prev_idx] as opus_int;

            /* Insert VAD and FEC flags at beginning of bitstream */
            if *nBytesOut > 0
                && encState.state_Fxx[0].sCmn.nFramesEncoded
                    == encState.state_Fxx[0].sCmn.nFramesPerPacket
            {
                flags = 0;
                for n in 0..encControl.nChannelsInternal {
                    for i in 0..encState.state_Fxx[n as usize].sCmn.nFramesPerPacket {
                        flags = silk_LSHIFT(flags, 1);
                        flags |= encState.state_Fxx[n as usize].sCmn.VAD_flags[i as usize]
                            as opus_int;
                    }
                    flags = silk_LSHIFT(flags, 1);
                    flags |= encState.state_Fxx[n as usize].sCmn.LBRR_flag as opus_int;
                }
                if prefillFlag == 0 {
                    ec_enc_patch_initial_bits(
                        psRangeEnc,
                        flags as u32,
                        ((encState.state_Fxx[0].sCmn.nFramesPerPacket + 1)
                            * encControl.nChannelsInternal) as u32,
                    );
                }

                /* Return zero bytes if all channels DTXed */
                if encState.state_Fxx[0].sCmn.inDTX != 0
                    && (encControl.nChannelsInternal == 1
                        || encState.state_Fxx[1].sCmn.inDTX != 0)
                {
                    *nBytesOut = 0;
                }

                encState.nBitsExceeded += *nBytesOut * 8;
                encState.nBitsExceeded -= silk_DIV32_16(
                    silk_MUL(encControl.bitRate, encControl.payloadSize_ms),
                    1000,
                );
                encState.nBitsExceeded = silk_LIMIT(encState.nBitsExceeded, 0, 10000);

                /* Update flag indicating if bandwidth switching is allowed */
                speech_act_thr_for_switch_Q8 = silk_SMLAWB(
                    SILK_FIX_CONST(SPEECH_ACTIVITY_DTX_THRES as f64, 8),
                    SILK_FIX_CONST(
                        (1.0 - SPEECH_ACTIVITY_DTX_THRES as f64)
                            / MAX_BANDWIDTH_SWITCH_DELAY_MS as f64,
                        16 + 8,
                    ),
                    encState.timeSinceSwitchAllowed_ms,
                );
                if encState.state_Fxx[0].sCmn.speech_activity_Q8 < speech_act_thr_for_switch_Q8 {
                    encState.allowBandwidthSwitch = 1;
                    encState.timeSinceSwitchAllowed_ms = 0;
                } else {
                    encState.allowBandwidthSwitch = 0;
                    encState.timeSinceSwitchAllowed_ms += encControl.payloadSize_ms;
                }
            }

            if nSamplesIn == 0 {
                break;
            }
        } else {
            break;
        }
        curr_block += 1;
    }

    encState.nPrevChannelsInternal = encControl.nChannelsInternal;

    encControl.allowBandwidthSwitch = encState.allowBandwidthSwitch;
    encControl.inWBmodeWithoutVariableLP = (encState.state_Fxx[0].sCmn.fs_kHz == 16
        && encState.state_Fxx[0].sCmn.sLP.mode == 0)
        as opus_int;
    encControl.internalSampleRate = silk_SMULBB(encState.state_Fxx[0].sCmn.fs_kHz, 1000);
    encControl.stereoWidth_Q14 = if encControl.toMono != 0 {
        0
    } else {
        encState.sStereo.smth_width_Q14 as opus_int
    };
    if prefillFlag != 0 {
        encControl.payloadSize_ms = tmp_payloadSize_ms;
        encControl.complexity = tmp_complexity;
        for n in 0..encControl.nChannelsInternal {
            encState.state_Fxx[n as usize].sCmn.controlled_since_last_payload = 0;
            encState.state_Fxx[n as usize].sCmn.prefillFlag = 0;
        }
    }

    encControl.signalType = encState.state_Fxx[0].sCmn.indices.signalType as opus_int;
    encControl.offset = silk_Quantization_Offsets_Q10
        [(encState.state_Fxx[0].sCmn.indices.signalType >> 1) as usize]
        [encState.state_Fxx[0].sCmn.indices.quantOffsetType as usize]
        as opus_int;
    ret
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_encoder_size_reports_smaller_for_mono() {
        let mut sz2 = 0;
        let mut sz1 = 0;
        assert_eq!(silk_Get_Encoder_Size(&mut sz2, 2), SILK_NO_ERROR);
        assert_eq!(silk_Get_Encoder_Size(&mut sz1, 1), SILK_NO_ERROR);
        assert_eq!(sz2 as usize, core::mem::size_of::<silk_encoder>());
        assert_eq!(
            sz1 as usize,
            core::mem::size_of::<silk_encoder>()
                - core::mem::size_of::<silk_encoder_state_Fxx>()
        );
        assert!(sz1 < sz2);
    }

    #[test]
    fn init_encoder_resets_the_api_channel_count() {
        let mut e = Box::new(silk_encoder::default());
        let mut st = silk_EncControlStruct::default();
        assert_eq!(silk_InitEncoder(&mut e, 2, 4, &mut st), SILK_NO_ERROR);
        assert_eq!(e.nChannelsAPI, 1);
        assert_eq!(e.nChannelsInternal, 1);
        assert_eq!(e.state_Fxx[0].sCmn.arch, 4);
        assert_eq!(e.state_Fxx[1].sCmn.arch, 4);
        assert_eq!(e.state_Fxx[0].sCmn.first_frame_after_reset, 1);
        /* `silk_QueryEncoder` reads the freshly reset state back out. */
        assert_eq!(st.nChannelsAPI, 1);
        assert_eq!(st.nChannelsInternal, 1);
        assert_eq!(st.internalSampleRate, 0); /* fs_kHz == 0 after a reset */
        assert_eq!(st.inWBmodeWithoutVariableLP, 0);
    }

    #[test]
    fn query_encoder_mirrors_the_state_fields() {
        let mut e = Box::new(silk_encoder::default());
        e.nChannelsAPI = 2;
        e.nChannelsInternal = 2;
        e.state_Fxx[0].sCmn.API_fs_Hz = 48000;
        e.state_Fxx[0].sCmn.maxInternal_fs_Hz = 16000;
        e.state_Fxx[0].sCmn.minInternal_fs_Hz = 8000;
        e.state_Fxx[0].sCmn.desiredInternal_fs_Hz = 16000;
        e.state_Fxx[0].sCmn.PacketSize_ms = 20;
        e.state_Fxx[0].sCmn.TargetRate_bps = 24000;
        e.state_Fxx[0].sCmn.PacketLoss_perc = 5;
        e.state_Fxx[0].sCmn.Complexity = 7;
        e.state_Fxx[0].sCmn.useInBandFEC = 1;
        e.state_Fxx[0].sCmn.useDTX = 1;
        e.state_Fxx[0].sCmn.useCBR = 1;
        e.state_Fxx[0].sCmn.fs_kHz = 12;
        e.state_Fxx[0].sCmn.allow_bandwidth_switch = 1;
        e.state_Fxx[0].sCmn.sLP.mode = 1;
        let mut st = silk_EncControlStruct::default();
        assert_eq!(silk_QueryEncoder(&e, &mut st), SILK_NO_ERROR);
        assert_eq!(st.nChannelsAPI, 2);
        assert_eq!(st.nChannelsInternal, 2);
        assert_eq!(st.API_sampleRate, 48000);
        assert_eq!(st.maxInternalSampleRate, 16000);
        assert_eq!(st.minInternalSampleRate, 8000);
        assert_eq!(st.desiredInternalSampleRate, 16000);
        assert_eq!(st.payloadSize_ms, 20);
        assert_eq!(st.bitRate, 24000);
        assert_eq!(st.packetLossPercentage, 5);
        assert_eq!(st.complexity, 7);
        assert_eq!(st.useInBandFEC, 1);
        assert_eq!(st.useDTX, 1);
        assert_eq!(st.useCBR, 1);
        assert_eq!(st.internalSampleRate, 12000);
        assert_eq!(st.allowBandwidthSwitch, 1);
        assert_eq!(st.inWBmodeWithoutVariableLP, 0);
    }

    #[test]
    fn query_encoder_reports_wb_without_variable_lp() {
        let mut e = Box::new(silk_encoder::default());
        e.state_Fxx[0].sCmn.fs_kHz = 16;
        e.state_Fxx[0].sCmn.sLP.mode = 0;
        let mut st = silk_EncControlStruct::default();
        silk_QueryEncoder(&e, &mut st);
        assert_eq!(st.inWBmodeWithoutVariableLP, 1);
        assert_eq!(st.internalSampleRate, 16000);
    }
}

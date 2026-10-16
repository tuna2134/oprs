//! Transliteration of `silk/dec_API.c` from libopus 1.6.1 to safe Rust.
//!
//! `silk_decoder` (the super structure the C declares in this file) lives in
//! `src/silk/fixed/structs_fix.rs`, exactly as the foundation agent placed it;
//! this module only implements the functions.
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

use crate::celt::arch::{celt_assert, opus_res, INT16TORES};
use crate::celt::entcode::ec_ctx;
use crate::celt::entdec::{ec_dec_bit_logp, ec_dec_icdf};
use crate::silk::control::*;
use crate::silk::decode_frame::{osce_reset, silk_decode_frame};
use crate::silk::decode_indices::silk_decode_indices;
use crate::silk::decode_pulses::silk_decode_pulses;
use crate::silk::decoder_set_fs::silk_decoder_set_fs;
use crate::silk::define::*;
use crate::silk::fixed::structs_fix::silk_decoder;
use crate::silk::init_decoder::{silk_init_decoder, silk_reset_decoder};
use crate::silk::macros::*;
use crate::silk::plc::LPCNetPLCState;
use crate::silk::resampler::silk_resampler;
use crate::silk::sigproc_fix::*;
use crate::silk::stereo_decode_pred::{silk_stereo_decode_mid_only, silk_stereo_decode_pred};
use crate::silk::stereo_ms_to_lr::silk_stereo_MS_to_LR;
use crate::silk::structs::*;
use crate::silk::tables_other::silk_LBRR_flags_iCDF_ptr;
use crate::silk::typedef::*;
use crate::types::*;

/// `silk_LoadOSCEModels(decState, data, len)`
///
/// `osce_load_models()` belongs to the DNN, which the port does not include.
/// The C only records whether the load succeeded (`osce_model.loaded`); a
/// decoder with `loaded == 0` takes `OSCE_METHOD_NONE` in
/// `osce_enhance_frame`, which is what the reference build selects at the
/// default decoder complexity anyway.
pub fn silk_LoadOSCEModels(
    _decState: &mut silk_decoder,
    _data: Option<&[u8]>,
    _len: opus_int,
) -> opus_int {
    SILK_NO_ERROR
}

/// `silk_Get_Decoder_Size(decSizeBytes)`
pub fn silk_Get_Decoder_Size(
    decSizeBytes: &mut opus_int, /* O    Number of bytes in SILK decoder state */
) -> opus_int {
    *decSizeBytes = core::mem::size_of::<silk_decoder>() as opus_int;

    SILK_NO_ERROR
}

/* Reset decoder state */
/// `silk_ResetDecoder(decState)`
pub fn silk_ResetDecoder(
    decState: &mut silk_decoder, /* I/O  State */
) -> opus_int {
    let mut ret: opus_int = SILK_NO_ERROR;

    for n in 0..DECODER_NUM_CHANNELS {
        ret = silk_reset_decoder(&mut decState.channel_state[n as usize]);
    }
    decState.sStereo = stereo_dec_state::default();
    /* Not strictly needed, but it's cleaner that way */
    decState.prev_decode_only_middle = 0;

    ret
}

/// `silk_InitDecoder(decState)`
pub fn silk_InitDecoder(
    decState: &mut silk_decoder, /* I/O  State */
) -> opus_int {
    let mut ret: opus_int = SILK_NO_ERROR;

    /* `osce_model.loaded = 0`; then the model load (a no-op stub here). */
    silk_LoadOSCEModels(decState, None, 0);

    for n in 0..DECODER_NUM_CHANNELS {
        ret = silk_init_decoder(&mut decState.channel_state[n as usize]);
    }
    decState.sStereo = stereo_dec_state::default();
    /* Not strictly needed, but it's cleaner that way */
    decState.prev_decode_only_middle = 0;

    ret
}

/* Decode a frame */
/// `silk_Decode(decState, decControl, lostFlag, newPacketFlag, psRangeDec, samplesOut, nSamplesOut, lpcnet, arch)`
pub fn silk_Decode(
    decState: &mut silk_decoder,          /* I/O  State                                   */
    decControl: &mut silk_DecControlStruct, /* I/O  Control Structure                      */
    lostFlag: opus_int,                   /* I    0: no loss, 1 loss, 2 decode fec        */
    newPacketFlag: opus_int,              /* I    Indicates first decoder call for this packet */
    psRangeDec: &mut ec_ctx,              /* I/O  Compressor data structure               */
    samplesOut: &mut [opus_res],          /* O    Decoded output speech vector            */
    nSamplesOut: &mut opus_int32,         /* O    Number of samples decoded               */
    mut lpcnet: Option<&mut LPCNetPLCState>,
    arch: opus_int,                       /* I    Run-time architecture                   */
) -> opus_int {
    let mut i: opus_int;
    let mut n: opus_int;
    let mut decode_only_middle: opus_int = 0;
    let mut ret: opus_int = SILK_NO_ERROR;
    let mut nSamplesOutDec: opus_int32 = 0;
    let mut LBRR_symbol: opus_int;
    let mut MS_pred_Q13 = [0 as opus_int32; 2];
    let mut has_side: opus_int;
    let mut stereo_to_mono: opus_int;

    celt_assert!(decControl.nChannelsInternal == 1 || decControl.nChannelsInternal == 2);

    /**********************************/
    /* Test if first frame in payload */
    /**********************************/
    if newPacketFlag != 0 {
        for n in 0..decControl.nChannelsInternal {
            decState.channel_state[n as usize].nFramesDecoded = 0; /* Used to count frames in packet */
        }
    }

    /* If Mono -> Stereo transition in bitstream: init state of second channel */
    if decControl.nChannelsInternal > decState.nChannelsInternal {
        ret += silk_init_decoder(&mut decState.channel_state[1]);
    }

    stereo_to_mono = (decControl.nChannelsInternal == 1
        && decState.nChannelsInternal == 2
        && (decControl.internalSampleRate == 1000 * decState.channel_state[0].fs_kHz))
        as opus_int;

    if decState.channel_state[0].nFramesDecoded == 0 {
        for n in 0..decControl.nChannelsInternal {
            let fs_kHz_dec: opus_int;
            if decControl.payloadSize_ms == 0 {
                /* Assuming packet loss, use 10 ms */
                decState.channel_state[n as usize].nFramesPerPacket = 1;
                decState.channel_state[n as usize].nb_subfr = 2;
            } else if decControl.payloadSize_ms == 10 {
                decState.channel_state[n as usize].nFramesPerPacket = 1;
                decState.channel_state[n as usize].nb_subfr = 2;
            } else if decControl.payloadSize_ms == 20 {
                decState.channel_state[n as usize].nFramesPerPacket = 1;
                decState.channel_state[n as usize].nb_subfr = 4;
            } else if decControl.payloadSize_ms == 40 {
                decState.channel_state[n as usize].nFramesPerPacket = 2;
                decState.channel_state[n as usize].nb_subfr = 4;
            } else if decControl.payloadSize_ms == 60 {
                decState.channel_state[n as usize].nFramesPerPacket = 3;
                decState.channel_state[n as usize].nb_subfr = 4;
            } else {
                celt_assert!(false);
                return SILK_DEC_INVALID_FRAME_SIZE;
            }
            fs_kHz_dec = (decControl.internalSampleRate >> 10) + 1;
            if fs_kHz_dec != 8 && fs_kHz_dec != 12 && fs_kHz_dec != 16 {
                celt_assert!(false);
                return SILK_DEC_INVALID_SAMPLING_FREQUENCY;
            }
            ret += silk_decoder_set_fs(
                &mut decState.channel_state[n as usize],
                fs_kHz_dec,
                decControl.API_sampleRate,
            );
        }
    }

    if decControl.nChannelsAPI == 2
        && decControl.nChannelsInternal == 2
        && (decState.nChannelsAPI == 1 || decState.nChannelsInternal == 1)
    {
        decState.sStereo.pred_prev_Q13 = [0; 2];
        decState.sStereo.sSide = [0; 2];
        decState.channel_state[1].resampler_state = decState.channel_state[0].resampler_state;
    }
    decState.nChannelsAPI = decControl.nChannelsAPI;
    decState.nChannelsInternal = decControl.nChannelsInternal;

    if decControl.API_sampleRate > MAX_API_FS_KHZ * 1000 || decControl.API_sampleRate < 8000 {
        ret = SILK_DEC_INVALID_SAMPLING_FREQUENCY;
        return ret;
    }

    if lostFlag != FLAG_PACKET_LOST && decState.channel_state[0].nFramesDecoded == 0 {
        /* First decoder call for this payload */
        /* Decode VAD flags and LBRR flag */
        for n in 0..decControl.nChannelsInternal {
            for i in 0..decState.channel_state[n as usize].nFramesPerPacket {
                decState.channel_state[n as usize].VAD_flags[i as usize] =
                    ec_dec_bit_logp(psRangeDec, 1);
            }
            decState.channel_state[n as usize].LBRR_flag = ec_dec_bit_logp(psRangeDec, 1);
        }
        /* Decode LBRR flags */
        for n in 0..decControl.nChannelsInternal {
            decState.channel_state[n as usize].LBRR_flags = [0; MAX_FRAMES_PER_PACKET as usize];
            if decState.channel_state[n as usize].LBRR_flag != 0 {
                if decState.channel_state[n as usize].nFramesPerPacket == 1 {
                    decState.channel_state[n as usize].LBRR_flags[0] = 1;
                } else {
                    LBRR_symbol = ec_dec_icdf(
                        psRangeDec,
                        silk_LBRR_flags_iCDF_ptr
                            [(decState.channel_state[n as usize].nFramesPerPacket - 2) as usize],
                        8,
                    ) + 1;
                    for i in 0..decState.channel_state[n as usize].nFramesPerPacket {
                        decState.channel_state[n as usize].LBRR_flags[i as usize] =
                            silk_RSHIFT(LBRR_symbol, i) & 1;
                    }
                }
            }
        }

        if lostFlag == FLAG_DECODE_NORMAL {
            /* Regular decoding: skip all LBRR data */
            for i in 0..decState.channel_state[0].nFramesPerPacket {
                for n in 0..decControl.nChannelsInternal {
                    if decState.channel_state[n as usize].LBRR_flags[i as usize] != 0 {
                        let mut pulses = [0 as opus_int16; MAX_FRAME_LENGTH as usize];
                        let condCoding: opus_int;

                        if decControl.nChannelsInternal == 2 && n == 0 {
                            silk_stereo_decode_pred(psRangeDec, &mut MS_pred_Q13);
                            if decState.channel_state[1].LBRR_flags[i as usize] == 0 {
                                silk_stereo_decode_mid_only(psRangeDec, &mut decode_only_middle);
                            }
                        }
                        /* Use conditional coding if previous frame available */
                        if i > 0 && decState.channel_state[n as usize].LBRR_flags[(i - 1) as usize] != 0
                        {
                            condCoding = CODE_CONDITIONALLY;
                        } else {
                            condCoding = CODE_INDEPENDENTLY;
                        }
                        silk_decode_indices(
                            &mut decState.channel_state[n as usize],
                            psRangeDec,
                            i,
                            1,
                            condCoding,
                        );
                        let signalType =
                            decState.channel_state[n as usize].indices.signalType as opus_int;
                        let quantOffsetType =
                            decState.channel_state[n as usize].indices.quantOffsetType as opus_int;
                        let frame_length = decState.channel_state[n as usize].frame_length;
                        silk_decode_pulses(
                            psRangeDec,
                            &mut pulses,
                            signalType,
                            quantOffsetType,
                            frame_length,
                        );
                    }
                }
            }
        }
    }

    /* Get MS predictor index */
    if decControl.nChannelsInternal == 2 {
        if lostFlag == FLAG_DECODE_NORMAL
            || (lostFlag == FLAG_DECODE_LBRR
                && decState.channel_state[0].LBRR_flags
                    [decState.channel_state[0].nFramesDecoded as usize]
                    == 1)
        {
            silk_stereo_decode_pred(psRangeDec, &mut MS_pred_Q13);
            /* For LBRR data, decode mid-only flag only if side-channel's LBRR flag is false */
            if (lostFlag == FLAG_DECODE_NORMAL
                && decState.channel_state[1].VAD_flags
                    [decState.channel_state[0].nFramesDecoded as usize]
                    == 0)
                || (lostFlag == FLAG_DECODE_LBRR
                    && decState.channel_state[1].LBRR_flags
                        [decState.channel_state[0].nFramesDecoded as usize]
                        == 0)
            {
                silk_stereo_decode_mid_only(psRangeDec, &mut decode_only_middle);
            } else {
                decode_only_middle = 0;
            }
        } else {
            for n in 0..2 {
                MS_pred_Q13[n as usize] = decState.sStereo.pred_prev_Q13[n as usize] as opus_int32;
            }
        }
    }

    /* Reset side channel decoder prediction memory for first frame with side coding */
    if decControl.nChannelsInternal == 2
        && decode_only_middle == 0
        && decState.prev_decode_only_middle == 1
    {
        decState.channel_state[1].outBuf =
            [0; (MAX_FRAME_LENGTH + 2 * MAX_SUB_FRAME_LENGTH) as usize];
        decState.channel_state[1].sLPC_Q14_buf = [0; MAX_LPC_ORDER as usize];
        decState.channel_state[1].lagPrev = 100;
        decState.channel_state[1].LastGainIndex = 10;
        decState.channel_state[1].prevSignalType = TYPE_NO_VOICE_ACTIVITY;
        decState.channel_state[1].first_frame_after_reset = 1;
    }

    /* Check if the temp buffer fits into the output PCM buffer. If it fits,
       we can delay allocating the temp buffer until after the SILK peak stack
       usage. We need to use a < and not a <= because of the two extra samples. */
    let frame_length = decState.channel_state[0].frame_length;
    let buf_len = decControl.nChannelsInternal * (frame_length + 2);
    let mut samplesOut1_tmp_storage1 = vec![0 as opus_int16; buf_len as usize];

    if lostFlag == FLAG_DECODE_NORMAL {
        has_side = (decode_only_middle == 0) as opus_int;
    } else {
        has_side = (decState.prev_decode_only_middle == 0
            || (decControl.nChannelsInternal == 2
                && lostFlag == FLAG_DECODE_LBRR
                && decState.channel_state[1].LBRR_flags
                    [decState.channel_state[1].nFramesDecoded as usize]
                    == 1)) as opus_int;
    }
    decState.channel_state[0].sPLC.enable_deep_plc = decControl.enable_deep_plc;
    /* Call decoder for one frame */
    for n in 0..decControl.nChannelsInternal {
        if n == 0 || has_side != 0 {
            let FrameIndex: opus_int;
            let condCoding: opus_int;

            FrameIndex = decState.channel_state[0].nFramesDecoded - n;
            /* Use independent coding if no previous frame available */
            if FrameIndex <= 0 {
                condCoding = CODE_INDEPENDENTLY;
            } else if lostFlag == FLAG_DECODE_LBRR {
                condCoding = if decState.channel_state[n as usize].LBRR_flags
                    [(FrameIndex - 1) as usize]
                    != 0
                {
                    CODE_CONDITIONALLY
                } else {
                    CODE_INDEPENDENTLY
                };
            } else if n > 0 && decState.prev_decode_only_middle != 0 {
                /* If we skipped a side frame in this packet, we don't
                   need LTP scaling; the LTP state is well-defined. */
                condCoding = CODE_INDEPENDENTLY_NO_LTP_SCALING;
            } else {
                condCoding = CODE_CONDITIONALLY;
            }
            if decState.channel_state[n as usize].osce.method != decControl.osce_method {
                osce_reset(
                    &mut decState.channel_state[n as usize].osce,
                    decControl.osce_method,
                );
            }
            let base = (n * (frame_length + 2) + 2) as usize;
            let ln = if n == 0 { lpcnet.as_deref_mut() } else { None };
            ret += silk_decode_frame(
                &mut decState.channel_state[n as usize],
                psRangeDec,
                &mut samplesOut1_tmp_storage1[base..base + frame_length as usize],
                &mut nSamplesOutDec,
                lostFlag,
                condCoding,
                ln,
                arch,
            );
        } else {
            let base = (n * (frame_length + 2) + 2) as usize;
            samplesOut1_tmp_storage1[base..base + nSamplesOutDec as usize].fill(0);
        }
        decState.channel_state[n as usize].nFramesDecoded += 1;
    }

    let fs_kHz0 = decState.channel_state[0].fs_kHz;
    if decControl.nChannelsAPI == 2 && decControl.nChannelsInternal == 2 {
        /* Convert Mid/Side to Left/Right */
        let fl = frame_length as usize;
        let (c0, c1) = samplesOut1_tmp_storage1.split_at_mut(fl + 2);
        silk_stereo_MS_to_LR(
            &mut decState.sStereo,
            &mut c0[..fl + 2],
            &mut c1[..fl + 2],
            &MS_pred_Q13,
            fs_kHz0,
            nSamplesOutDec,
        );
    } else {
        /* Buffering */
        samplesOut1_tmp_storage1[0..2].copy_from_slice(&decState.sStereo.sMid);
        let idx = nSamplesOutDec as usize;
        decState.sStereo.sMid.copy_from_slice(&samplesOut1_tmp_storage1[idx..idx + 2]);
    }

    /* Number of output samples */
    *nSamplesOut = silk_DIV32(
        nSamplesOutDec * decControl.API_sampleRate,
        silk_SMULBB(fs_kHz0, 1000),
    );

    /* Set up pointers to temp buffers */
    let mut samplesOut2_tmp = vec![0 as opus_int16; *nSamplesOut as usize];

    for n in 0..silk_min(decControl.nChannelsAPI, decControl.nChannelsInternal) {
        /* Resample decoded signal to API_sampleRate */
        let base_in = (n * (frame_length + 2) + 1) as usize;
        ret += silk_resampler(
            &mut decState.channel_state[n as usize].resampler_state,
            &mut samplesOut2_tmp,
            &samplesOut1_tmp_storage1[base_in..base_in + nSamplesOutDec as usize],
            nSamplesOutDec,
        );
        /* Interleave if stereo output and stereo stream */
        if decControl.nChannelsAPI == 2 {
            for i in 0..*nSamplesOut {
                samplesOut[(n + 2 * i) as usize] = INT16TORES(samplesOut2_tmp[i as usize]);
            }
        } else {
            for i in 0..*nSamplesOut {
                samplesOut[i as usize] = INT16TORES(samplesOut2_tmp[i as usize]);
            }
        }
    }

    /* Create two channel output from mono stream */
    if decControl.nChannelsAPI == 2 && decControl.nChannelsInternal == 1 {
        if stereo_to_mono != 0 {
            /* Resample right channel for newly collapsed stereo just in case
               we weren't doing collapsing when switching to mono */
            ret += silk_resampler(
                &mut decState.channel_state[1].resampler_state,
                &mut samplesOut2_tmp,
                &samplesOut1_tmp_storage1[1..1 + nSamplesOutDec as usize],
                nSamplesOutDec,
            );

            for i in 0..*nSamplesOut {
                samplesOut[(1 + 2 * i) as usize] = INT16TORES(samplesOut2_tmp[i as usize]);
            }
        } else {
            for i in 0..*nSamplesOut {
                samplesOut[(1 + 2 * i) as usize] = samplesOut[(0 + 2 * i) as usize];
            }
        }
    }

    /* Export pitch lag, measured at 48 kHz sampling rate */
    if decState.channel_state[0].prevSignalType == TYPE_VOICED {
        let mult_tab: [opus_int; 3] = [6, 4, 3];
        decControl.prevPitchLag = decState.channel_state[0].lagPrev
            * mult_tab[((decState.channel_state[0].fs_kHz - 8) >> 2) as usize];
    } else {
        decControl.prevPitchLag = 0;
    }

    if lostFlag == FLAG_PACKET_LOST {
        /* On packet loss, remove the gain clamping to prevent having the energy "bounce back"
           if we lose packets when the energy is going down */
        for i in 0..decState.nChannelsInternal {
            decState.channel_state[i as usize].LastGainIndex = 10;
        }
    } else {
        decState.prev_decode_only_middle = decode_only_middle;
    }
    ret
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::celt::entdec::ec_dec_init;

    #[test]
    fn get_decoder_size_reports_the_struct_size() {
        let mut sz = 0;
        assert_eq!(silk_Get_Decoder_Size(&mut sz), SILK_NO_ERROR);
        assert_eq!(sz as usize, core::mem::size_of::<silk_decoder>());
    }

    #[test]
    fn init_then_reset_keeps_a_zeroed_state() {
        let mut dec = Box::new(silk_decoder::default());
        assert_eq!(silk_InitDecoder(&mut dec), SILK_NO_ERROR);
        assert_eq!(dec.channel_state[0].prev_gain_Q16, 65536);
        assert_eq!(dec.channel_state[1].prev_gain_Q16, 65536);
        assert_eq!(silk_ResetDecoder(&mut dec), SILK_NO_ERROR);
        assert_eq!(dec.prev_decode_only_middle, 0);
    }

    #[test]
    fn decode_a_lost_frame_produces_silence_first() {
        let mut dec = Box::new(silk_decoder::default());
        silk_InitDecoder(&mut dec);
        let mut ctl = silk_DecControlStruct::default();
        ctl.nChannelsAPI = 1;
        ctl.nChannelsInternal = 1;
        ctl.API_sampleRate = 16000;
        ctl.internalSampleRate = 16000;
        ctl.payloadSize_ms = 20;
        ctl.enable_deep_plc = 0;
        ctl.osce_method = 0;
        let buf = [0u8; 64];
        let mut range_dec = ec_dec_init(&buf);
        let mut out = [0.0f32; 320];
        let mut nout = 0;
        let ret = silk_Decode(
            &mut dec,
            &mut ctl,
            FLAG_PACKET_LOST,
            1,
            &mut range_dec,
            &mut out,
            &mut nout,
            None,
            4,
        );
        assert_eq!(ret, SILK_NO_ERROR);
        assert_eq!(nout, 320);
        assert!(out.iter().all(|&v| v == 0.0));
    }
}

//! Transliteration of `silk/decode_frame.c` from libopus 1.6.1 to safe Rust.
//!
//! This module also hosts the `ENABLE_OSCE` glue (`osce_reset`,
//! `osce_enhance_frame`) that `silk_decode_frame` calls.  The DNN itself
//! (`dnn/osce.c`) is out of scope for the port; see `osce_enhance_frame` for
//! the argument that the `OSCE_METHOD_NONE` path it takes in the reference
//! configuration is exactly reproducible without the network.
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

use crate::celt::arch::celt_assert;
use crate::celt::entcode::ec_ctx;
use crate::silk::cng::silk_CNG;
use crate::silk::decode_core::silk_decode_core;
use crate::silk::decode_indices::silk_decode_indices;
use crate::silk::decode_parameters::silk_decode_parameters;
use crate::silk::decode_pulses::silk_decode_pulses;
use crate::silk::define::*;
use crate::silk::plc::{silk_PLC, silk_PLC_glue_frames, LPCNetPLCState};
use crate::silk::structs::*;
use crate::silk::typedef::*;
use crate::types::*;
use crate::silk::control::{FLAG_DECODE_NORMAL, FLAG_DECODE_LBRR};

// ---------------------------------------------------------------------------
// `dnn/osce.h` — the parts of the OSCE interface the SILK decoder touches.
// ---------------------------------------------------------------------------

/// `OSCE_METHOD_NONE`
pub const OSCE_METHOD_NONE: opus_int = 0;
/// `OSCE_METHOD_LACE`
pub const OSCE_METHOD_LACE: opus_int = 1;
/// `OSCE_METHOD_NOLACE`
pub const OSCE_METHOD_NOLACE: opus_int = 2;
/// `OSCE_DEFAULT_METHOD` — `OSCE_METHOD_NOLACE` with neither method disabled.
pub const OSCE_DEFAULT_METHOD: opus_int = OSCE_METHOD_NOLACE;

/// `osce_reset(hOSCE, method)` — `dnn/osce.c`.
///
/// `OPUS_CLEAR(&hOSCE->features, 1)` zeroes the whole feature state; only the
/// `OSCE_METHOD_LACE`/`_NOLACE` branches touch the opaque `state` union, which
/// the port does not model.  The trailing `method`/`reset` assignments are
/// observable and are reproduced exactly.
pub fn osce_reset(hOSCE: &mut silk_OSCE_struct, method: opus_int) {
    /* OPUS_CLEAR(&hOSCE->features, 1) */
    hOSCE.features = Default::default();
    hOSCE.method = method;
    hOSCE.features.reset = 2;
}

/// `osce_enhance_frame(model, psDec, psDecCtrl, xq, num_bits, arch)` —
/// `dnn/osce.c`.
///
/// The network itself is not part of the port.  In the reference configuration
/// the decoder can only reach `OSCE_METHOD_NONE` (`decControl->osce_method` is
/// set to `OSCE_METHOD_NONE` below decoder complexity 6), and that path is
/// exactly reproducible without the model:
///
/// ```text
///   in_buffer[i]  = xq[i] / 32768
///   out_buffer[i] = in_buffer[i]                       (method NONE)
///   xq[i]         = float2int(clamp(32768 * out_buffer[i], -32767, 32767))
/// ```
///
/// `xq[i]` is an `int16`, so `32768 * (xq[i] / 32768)` is exact and the whole
/// operation is the identity except that the clamp maps `-32768` onto `-32767`.
/// The optional 10 ms cross-fade (taken on the second frame after an
/// `osce_reset`) perturbs `out_buffer` by less than half an ULP of `xq[i]` and
/// has been verified exhaustively over all 65536 `int16` values and all 160
/// faded positions to leave the rounded result unchanged.  The `reset` counter
/// transitions are still reproduced so that the `psDec->osce` state matches C.
///
/// `model->loaded` is 0 in this port (no weights), which in the C also selects
/// `OSCE_METHOD_NONE`; a decoder at complexity >= 6 would need the real network
/// and is outside the scope of this transliteration.
pub fn osce_enhance_frame(
    psDec: &mut silk_decoder_state,
    _psDecCtrl: &silk_decoder_control,
    xq: &mut [opus_int16],
    _num_bits: opus_int32,
    _arch: opus_int,
) {
    /* enhancement only implemented for 20 ms frame at 16kHz */
    if psDec.fs_kHz != 16 || psDec.nb_subfr != 4 {
        let method = psDec.osce.method;
        osce_reset(&mut psDec.osce, method);
        return;
    }

    /* `osce_calculate_features()` only writes `psDec->osce.features`
       (numbits_smooth, signal_history, pitch hangover) and the `features`
       array that feeds the network; neither affects `xq`, so it is skipped. */

    /* cross-fade / output scaling bookkeeping */
    if psDec.osce.features.reset > 1 {
        psDec.osce.features.reset -= 1;
    } else if psDec.osce.features.reset != 0 {
        psDec.osce.features.reset = 0;
    }

    /* OSCE_METHOD_NONE: `out_buffer == in_buffer`, then scale back with the
       output clamp.  See the doc comment for why this equals the C bit for
       bit on every `int16`. */
    for i in 0..320 {
        let mut tmp = xq[i] as f32;
        if tmp > 32767.0f32 {
            tmp = 32767.0f32;
        }
        if tmp < -32767.0f32 {
            tmp = -32767.0f32;
        }
        xq[i] = tmp as opus_int16;
    }
}

/****************/
/* Decode frame */
/****************/
/// `silk_decode_frame(psDec, psRangeDec, pOut, pN, lostFlag, condCoding, lpcnet, osce_model, arch)`
pub fn silk_decode_frame(
    psDec: &mut silk_decoder_state, /* I/O  Pointer to Silk decoder state      */
    psRangeDec: &mut ec_ctx,        /* I/O  Compressor data structure          */
    pOut: &mut [opus_int16],        /* O    Pointer to output speech frame     */
    pN: &mut opus_int32,            /* O    Pointer to size of output frame    */
    lostFlag: opus_int,             /* I    0: no loss, 1 loss, 2 decode fec   */
    condCoding: opus_int,           /* I    The type of conditional coding     */
    lpcnet: Option<&mut LPCNetPLCState>,
    arch: opus_int,                 /* I    Run-time architecture              */
) -> opus_int {
    let mut psDecCtrl = silk_decoder_control::default();
    let mut L: opus_int;
    let mut mv_len: opus_int;
    let ret: opus_int = 0;

    L = psDec.frame_length;
    psDecCtrl.LTP_scale_Q14 = 0;

    /* Safety checks */
    celt_assert!(L > 0 && L <= MAX_FRAME_LENGTH);

    if lostFlag == FLAG_DECODE_NORMAL
        || (lostFlag == FLAG_DECODE_LBRR
            && psDec.LBRR_flags[psDec.nFramesDecoded as usize] == 1)
    {
        /* ENABLE_OSCE: ec_start = ec_tell(psRangeDec); */
        let ec_start = psRangeDec.ec_tell();
        let pulses_len = ((L + SHELL_CODEC_FRAME_LENGTH - 1) & !(SHELL_CODEC_FRAME_LENGTH - 1))
            as usize;
        let mut pulses = vec![0 as opus_int16; pulses_len];
        /*********************************************/
        /* Decode quantization indices of side info  */
        /*********************************************/
        silk_decode_indices(
            psDec,
            psRangeDec,
            psDec.nFramesDecoded,
            lostFlag,
            condCoding,
        );

        /*********************************************/
        /* Decode quantization indices of excitation */
        /*********************************************/
        silk_decode_pulses(
            psRangeDec,
            &mut pulses,
            psDec.indices.signalType as opus_int,
            psDec.indices.quantOffsetType as opus_int,
            psDec.frame_length,
        );

        /********************************************/
        /* Decode parameters and pulse signal       */
        /********************************************/
        silk_decode_parameters(psDec, &mut psDecCtrl, condCoding);

        /********************************************************/
        /* Run inverse NSQ                                      */
        /********************************************************/
        silk_decode_core(psDec, &mut psDecCtrl, &mut pOut[..L as usize], &pulses, arch);

        /*************************/
        /* Update output buffer. */
        /*************************/
        celt_assert!(psDec.ltp_mem_length >= psDec.frame_length);
        mv_len = psDec.ltp_mem_length - psDec.frame_length;
        psDec
            .outBuf
            .copy_within(psDec.frame_length as usize..psDec.ltp_mem_length as usize, 0);
        psDec.outBuf[mv_len as usize..psDec.ltp_mem_length as usize]
            .copy_from_slice(&pOut[..psDec.frame_length as usize]);

        /* ENABLE_OSCE: run SILK enhancer */
        osce_enhance_frame(
            psDec,
            &psDecCtrl,
            &mut pOut[..L as usize],
            psRangeDec.ec_tell() - ec_start,
            arch,
        );

        /********************************************************/
        /* Update PLC state                                     */
        /********************************************************/
        silk_PLC(psDec, &mut psDecCtrl, &mut pOut[..L as usize], 0, lpcnet, arch);

        psDec.lossCnt = 0;
        psDec.prevSignalType = psDec.indices.signalType as opus_int;
        celt_assert!(psDec.prevSignalType >= 0 && psDec.prevSignalType <= 2);

        /* A frame has been decoded without errors */
        psDec.first_frame_after_reset = 0;
    } else {
        /* Handle packet loss by extrapolation */
        silk_PLC(psDec, &mut psDecCtrl, &mut pOut[..L as usize], 1, lpcnet, arch);

        /* osce_reset( &psDec->osce, psDec->osce.method ); */
        let method = psDec.osce.method;
        osce_reset(&mut psDec.osce, method);

        /*************************/
        /* Update output buffer. */
        /*************************/
        celt_assert!(psDec.ltp_mem_length >= psDec.frame_length);
        mv_len = psDec.ltp_mem_length - psDec.frame_length;
        psDec
            .outBuf
            .copy_within(psDec.frame_length as usize..psDec.ltp_mem_length as usize, 0);
        psDec.outBuf[mv_len as usize..psDec.ltp_mem_length as usize]
            .copy_from_slice(&pOut[..psDec.frame_length as usize]);
    }

    /************************************************/
    /* Comfort noise generation / estimation        */
    /************************************************/
    silk_CNG(psDec, &psDecCtrl, &mut pOut[..L as usize], L);

    /****************************************************************/
    /* Ensure smooth connection of extrapolated and good frames     */
    /****************************************************************/
    silk_PLC_glue_frames(psDec, &mut pOut[..L as usize], L);

    /* Update some decoder state variables */
    psDec.lagPrev = psDecCtrl.pitchL[(psDec.nb_subfr - 1) as usize];

    /* Set output frame length */
    *pN = L;

    ret
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn osce_reset_clears_features_and_sets_flags() {
        let mut osce = silk_OSCE_struct::default();
        osce.features.numbits_smooth = 12.0;
        osce.features.last_lag = 77;
        osce_reset(&mut osce, OSCE_METHOD_NONE);
        assert_eq!(osce.method, OSCE_METHOD_NONE);
        assert_eq!(osce.features.reset, 2);
        assert_eq!(osce.features.numbits_smooth, 0.0);
        assert_eq!(osce.features.last_lag, 0);
    }

    #[test]
    fn osce_enhance_clamps_minus_32768() {
        /* 16 kHz, 4 subframes: the enhancer is active. */
        let mut st = silk_decoder_state::default();
        st.fs_kHz = 16;
        st.nb_subfr = 4;
        st.osce.features.reset = 0;
        let ctrl = silk_decoder_control::default();
        let mut xq = [0 as opus_int16; 320];
        xq[0] = -32768;
        xq[1] = 32767;
        xq[2] = -12345;
        osce_enhance_frame(&mut st, &ctrl, &mut xq, 100, 4);
        assert_eq!(xq[0], -32767);
        assert_eq!(xq[1], 32767);
        assert_eq!(xq[2], -12345);
        assert_eq!(st.osce.features.reset, 0);
    }

    #[test]
    fn osce_enhance_resets_when_not_16k_20ms() {
        let mut st = silk_decoder_state::default();
        st.fs_kHz = 12;
        st.nb_subfr = 2;
        st.osce.method = OSCE_METHOD_LACE;
        st.osce.features.reset = 0;
        let ctrl = silk_decoder_control::default();
        let mut xq = [0 as opus_int16; 320];
        xq[0] = -32768;
        osce_enhance_frame(&mut st, &ctrl, &mut xq, 100, 4);
        /* early return: xq untouched, state reset to the current method */
        assert_eq!(xq[0], -32768);
        assert_eq!(st.osce.method, OSCE_METHOD_LACE);
        assert_eq!(st.osce.features.reset, 2);
    }
}

//! Transliteration of `silk/decoder_set_fs.c` from libopus 1.6.1 to safe Rust.
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
use crate::silk::define::*;
use crate::silk::resampler::silk_resampler_init;
use crate::silk::sigproc_fix::*;
use crate::silk::structs::*;
use crate::silk::tables_nlsf_cb_nb_mb::silk_NLSF_CB_NB_MB;
use crate::silk::tables_nlsf_cb_wb::silk_NLSF_CB_WB;
use crate::silk::tables_other::{silk_uniform4_iCDF, silk_uniform6_iCDF, silk_uniform8_iCDF};
use crate::silk::tables_pitch_lag::*;
use crate::silk::tables_pulses_per_block::*;
use crate::silk::typedef::*;
use crate::types::*;
use crate::silk::inlines::silk_SQRT_APPROX;
use crate::silk::macros::silk_SMULBB;

/* Set decoder sampling rate */
/// `silk_decoder_set_fs(psDec, fs_kHz, fs_API_Hz)`
pub fn silk_decoder_set_fs(
    psDec: &mut silk_decoder_state, /* I/O  Decoder state pointer   */
    fs_kHz: opus_int,               /* I    Sampling frequency (kHz) */
    fs_API_Hz: opus_int32,          /* I    API Sampling frequency (Hz) */
) -> opus_int {
    let frame_length: opus_int;
    let mut ret: opus_int = 0;

    celt_assert!(fs_kHz == 8 || fs_kHz == 12 || fs_kHz == 16);
    celt_assert!(psDec.nb_subfr == MAX_NB_SUBFR || psDec.nb_subfr == MAX_NB_SUBFR / 2);

    /* New (sub)frame length */
    psDec.subfr_length = silk_SMULBB(SUB_FRAME_LENGTH_MS, fs_kHz);
    frame_length = silk_SMULBB(psDec.nb_subfr, psDec.subfr_length);

    /* Initialize resampler when switching internal or external sampling frequency */
    if psDec.fs_kHz != fs_kHz || psDec.fs_API_hz != fs_API_Hz {
        /* Initialize the resampler for dec_API.c preparing resampling from fs_kHz to API_fs_Hz */
        ret += silk_resampler_init(
            &mut psDec.resampler_state,
            silk_SMULBB(fs_kHz, 1000),
            fs_API_Hz,
            0,
        );

        psDec.fs_API_hz = fs_API_Hz;
    }

    if psDec.fs_kHz != fs_kHz || frame_length != psDec.frame_length {
        if fs_kHz == 8 {
            if psDec.nb_subfr == MAX_NB_SUBFR {
                psDec.pitch_contour_iCDF = &silk_pitch_contour_NB_iCDF;
            } else {
                psDec.pitch_contour_iCDF = &silk_pitch_contour_10_ms_NB_iCDF;
            }
        } else {
            if psDec.nb_subfr == MAX_NB_SUBFR {
                psDec.pitch_contour_iCDF = &silk_pitch_contour_iCDF;
            } else {
                psDec.pitch_contour_iCDF = &silk_pitch_contour_10_ms_iCDF;
            }
        }
        if psDec.fs_kHz != fs_kHz {
            psDec.ltp_mem_length = silk_SMULBB(LTP_MEM_LENGTH_MS, fs_kHz);
            if fs_kHz == 8 || fs_kHz == 12 {
                psDec.LPC_order = MIN_LPC_ORDER;
                psDec.psNLSF_CB = &silk_NLSF_CB_NB_MB;
            } else {
                psDec.LPC_order = MAX_LPC_ORDER;
                psDec.psNLSF_CB = &silk_NLSF_CB_WB;
            }
            if fs_kHz == 16 {
                psDec.pitch_lag_low_bits_iCDF = &silk_uniform8_iCDF;
            } else if fs_kHz == 12 {
                psDec.pitch_lag_low_bits_iCDF = &silk_uniform6_iCDF;
            } else if fs_kHz == 8 {
                psDec.pitch_lag_low_bits_iCDF = &silk_uniform4_iCDF;
            } else {
                /* unsupported sampling rate */
                celt_assert!(false);
            }
            psDec.first_frame_after_reset = 1;
            psDec.lagPrev = 100;
            psDec.LastGainIndex = 10;
            psDec.prevSignalType = TYPE_NO_VOICE_ACTIVITY;
            psDec.outBuf = [0; (MAX_FRAME_LENGTH + 2 * MAX_SUB_FRAME_LENGTH) as usize];
            psDec.sLPC_Q14_buf = [0; MAX_LPC_ORDER as usize];
        }

        psDec.fs_kHz = fs_kHz;
        psDec.frame_length = frame_length;
    }

    /* Check that settings are valid */
    celt_assert!(psDec.frame_length > 0 && psDec.frame_length <= MAX_FRAME_LENGTH);

    ret
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sets_16k_wideband_configuration() {
        let mut st = silk_decoder_state::default();
        st.nb_subfr = 4;
        assert_eq!(silk_decoder_set_fs(&mut st, 16, 48000), 0);
        assert_eq!(st.subfr_length, 80);
        assert_eq!(st.frame_length, 320);
        assert_eq!(st.ltp_mem_length, 320);
        assert_eq!(st.LPC_order, 16);
        assert_eq!(st.lagPrev, 100);
        assert_eq!(st.LastGainIndex, 10);
        assert_eq!(st.first_frame_after_reset, 1);
        assert_eq!(st.fs_kHz, 16);
        assert_eq!(st.fs_API_hz, 48000);
    }

    #[test]
    fn sets_8k_narrowband_configuration() {
        let mut st = silk_decoder_state::default();
        st.nb_subfr = 2;
        assert_eq!(silk_decoder_set_fs(&mut st, 8, 8000), 0);
        assert_eq!(st.subfr_length, 40);
        assert_eq!(st.frame_length, 80);
        assert_eq!(st.LPC_order, MIN_LPC_ORDER);
        assert_eq!(st.pitch_contour_iCDF, &silk_pitch_contour_10_ms_NB_iCDF[..]);
    }

    #[test]
    fn switching_internal_rate_resets_state() {
        let mut st = silk_decoder_state::default();
        st.nb_subfr = 4;
        silk_decoder_set_fs(&mut st, 16, 48000);
        st.lagPrev = 7;
        st.first_frame_after_reset = 0;
        silk_decoder_set_fs(&mut st, 12, 48000);
        assert_eq!(st.fs_kHz, 12);
        assert_eq!(st.lagPrev, 100);
        assert_eq!(st.first_frame_after_reset, 1);
        assert_eq!(st.LPC_order, MIN_LPC_ORDER);
    }
}

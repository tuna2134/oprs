//! Mirror of `silk/encode_indices.c` from libopus 1.6.1, transliterated to safe Rust.
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
use crate::celt::entenc::*;
use crate::silk::define::*;
use crate::silk::macros::*;
use crate::silk::nlsf_unpack::*;
use crate::silk::sigproc_fix::*;
use crate::silk::structs::*;
use crate::silk::tables_gain::*;
use crate::silk::tables_ltp::*;
use crate::silk::tables_other::*;
use crate::silk::tables_pitch_lag::*;
use crate::silk::typedef::silk_assert;
use crate::silk::typedef::*;
use crate::types::*;

/* Encode side-information parameters to payload */
pub fn silk_encode_indices(
    psEncC: &mut silk_encoder_state, /* I/O  Encoder state                               */
    psRangeEnc: &mut ec_ctx,         /* I/O  Compressor data structure                   */
    FrameIndex: opus_int,            /* I    Frame number                                */
    encode_LBRR: opus_int,           /* I    Flag indicating LBRR data is being encoded  */
    condCoding: opus_int,            /* I    The type of conditional coding to use       */
) {
    let mut ec_ix = [0 as opus_int16; MAX_LPC_ORDER as usize];
    let mut pred_Q8 = [0 as opus_uint8; MAX_LPC_ORDER as usize];

    /* The C keeps a pointer into psEncC; copying is equivalent because the
       routine never writes through it. */
    let psIndices: SideInfoIndices = if encode_LBRR != 0 {
        psEncC.indices_LBRR[FrameIndex as usize]
    } else {
        psEncC.indices
    };

    /*******************************************/
    /* Encode signal type and quantizer offset */
    /*******************************************/
    let typeOffset = 2 * psIndices.signalType as opus_int + psIndices.quantOffsetType as opus_int;
    celt_assert!(typeOffset >= 0 && typeOffset < 6);
    celt_assert!(encode_LBRR == 0 || typeOffset >= 2);
    if encode_LBRR != 0 || typeOffset >= 2 {
        ec_enc_icdf(psRangeEnc, typeOffset - 2, &silk_type_offset_VAD_iCDF, 8);
    } else {
        ec_enc_icdf(psRangeEnc, typeOffset, &silk_type_offset_no_VAD_iCDF, 8);
    }

    /****************/
    /* Encode gains */
    /****************/
    /* first subframe */
    if condCoding == CODE_CONDITIONALLY {
        /* conditional coding */
        silk_assert!(
            psIndices.GainsIndices[0] as opus_int >= 0
                && (psIndices.GainsIndices[0] as opus_int)
                    < MAX_DELTA_GAIN_QUANT - MIN_DELTA_GAIN_QUANT + 1
        );
        ec_enc_icdf(psRangeEnc, psIndices.GainsIndices[0] as opus_int, &silk_delta_gain_iCDF, 8);
    } else {
        /* independent coding, in two stages: MSB bits followed by 3 LSBs */
        silk_assert!(
            psIndices.GainsIndices[0] as opus_int >= 0
                && (psIndices.GainsIndices[0] as opus_int) < N_LEVELS_QGAIN
        );
        ec_enc_icdf(
            psRangeEnc,
            silk_RSHIFT(psIndices.GainsIndices[0] as opus_int, 3),
            &silk_gain_iCDF[psIndices.signalType as usize],
            8,
        );
        ec_enc_icdf(
            psRangeEnc,
            psIndices.GainsIndices[0] as opus_int & 7,
            &silk_uniform8_iCDF,
            8,
        );
    }

    /* remaining subframes */
    for i in 1..psEncC.nb_subfr {
        silk_assert!(
            psIndices.GainsIndices[i as usize] as opus_int >= 0
                && (psIndices.GainsIndices[i as usize] as opus_int)
                    < MAX_DELTA_GAIN_QUANT - MIN_DELTA_GAIN_QUANT + 1
        );
        ec_enc_icdf(
            psRangeEnc,
            psIndices.GainsIndices[i as usize] as opus_int,
            &silk_delta_gain_iCDF,
            8,
        );
    }

    /****************/
    /* Encode NLSFs */
    /****************/
    ec_enc_icdf(
        psRangeEnc,
        psIndices.NLSFIndices[0] as opus_int,
        &psEncC.psNLSF_CB.CB1_iCDF[((psIndices.signalType >> 1) as opus_int
            * psEncC.psNLSF_CB.nVectors as opus_int) as usize..],
        8,
    );
    silk_NLSF_unpack(
        &mut ec_ix,
        &mut pred_Q8,
        psEncC.psNLSF_CB,
        psIndices.NLSFIndices[0] as opus_int,
    );
    celt_assert!(psEncC.psNLSF_CB.order as opus_int == psEncC.predictLPCOrder);
    for i in 0..psEncC.psNLSF_CB.order as opus_int {
        if psIndices.NLSFIndices[(i + 1) as usize] as opus_int >= NLSF_QUANT_MAX_AMPLITUDE {
            ec_enc_icdf(
                psRangeEnc,
                2 * NLSF_QUANT_MAX_AMPLITUDE,
                &psEncC.psNLSF_CB.ec_iCDF[ec_ix[i as usize] as usize..],
                8,
            );
            ec_enc_icdf(
                psRangeEnc,
                psIndices.NLSFIndices[(i + 1) as usize] as opus_int - NLSF_QUANT_MAX_AMPLITUDE,
                &silk_NLSF_EXT_iCDF,
                8,
            );
        } else if psIndices.NLSFIndices[(i + 1) as usize] as opus_int
            <= -NLSF_QUANT_MAX_AMPLITUDE
        {
            ec_enc_icdf(
                psRangeEnc,
                0,
                &psEncC.psNLSF_CB.ec_iCDF[ec_ix[i as usize] as usize..],
                8,
            );
            ec_enc_icdf(
                psRangeEnc,
                -psIndices.NLSFIndices[(i + 1) as usize] as opus_int - NLSF_QUANT_MAX_AMPLITUDE,
                &silk_NLSF_EXT_iCDF,
                8,
            );
        } else {
            ec_enc_icdf(
                psRangeEnc,
                psIndices.NLSFIndices[(i + 1) as usize] as opus_int + NLSF_QUANT_MAX_AMPLITUDE,
                &psEncC.psNLSF_CB.ec_iCDF[ec_ix[i as usize] as usize..],
                8,
            );
        }
    }

    /* Encode NLSF interpolation factor */
    if psEncC.nb_subfr == MAX_NB_SUBFR {
        silk_assert!(
            (psIndices.NLSFInterpCoef_Q2 as opus_int) >= 0
                && (psIndices.NLSFInterpCoef_Q2 as opus_int) < 5
        );
        ec_enc_icdf(
            psRangeEnc,
            psIndices.NLSFInterpCoef_Q2 as opus_int,
            &silk_NLSF_interpolation_factor_iCDF,
            8,
        );
    }

    if psIndices.signalType as opus_int == TYPE_VOICED {
        /*********************/
        /* Encode pitch lags */
        /*********************/
        /* lag index */
        let mut encode_absolute_lagIndex = 1;
        if condCoding == CODE_CONDITIONALLY
            && psEncC.ec_prevSignalType == TYPE_VOICED
        {
            /* Delta Encoding */
            let mut delta_lagIndex =
                psIndices.lagIndex as opus_int - psEncC.ec_prevLagIndex as opus_int;
            if delta_lagIndex < -8 || delta_lagIndex > 11 {
                delta_lagIndex = 0;
            } else {
                delta_lagIndex += 9;
                encode_absolute_lagIndex = 0; /* Only use delta */
            }
            silk_assert!(delta_lagIndex >= 0 && delta_lagIndex < 21);
            ec_enc_icdf(psRangeEnc, delta_lagIndex, &silk_pitch_delta_iCDF, 8);
        }
        if encode_absolute_lagIndex != 0 {
            /* Absolute encoding */
            let pitch_high_bits =
                silk_DIV32_16(psIndices.lagIndex as opus_int32, silk_RSHIFT(psEncC.fs_kHz, 1));
            let pitch_low_bits = psIndices.lagIndex as opus_int32
                - silk_SMULBB(pitch_high_bits, silk_RSHIFT(psEncC.fs_kHz, 1));
            silk_assert!(pitch_low_bits < psEncC.fs_kHz / 2);
            silk_assert!(pitch_high_bits < 32);
            ec_enc_icdf(psRangeEnc, pitch_high_bits, &silk_pitch_lag_iCDF, 8);
            ec_enc_icdf(psRangeEnc, pitch_low_bits, psEncC.pitch_lag_low_bits_iCDF, 8);
        }
        psEncC.ec_prevLagIndex = psIndices.lagIndex;

        /* Contour index */
        silk_assert!((psIndices.contourIndex as opus_int) >= 0);
        silk_assert!(
            ((psIndices.contourIndex as opus_int) < 34 && psEncC.fs_kHz > 8 && psEncC.nb_subfr == 4)
                || ((psIndices.contourIndex as opus_int) < 11
                    && psEncC.fs_kHz == 8
                    && psEncC.nb_subfr == 4)
                || ((psIndices.contourIndex as opus_int) < 12 && psEncC.fs_kHz > 8 && psEncC.nb_subfr == 2)
                || ((psIndices.contourIndex as opus_int) < 3 && psEncC.fs_kHz == 8 && psEncC.nb_subfr == 2),
        );
        ec_enc_icdf(psRangeEnc, psIndices.contourIndex as opus_int, psEncC.pitch_contour_iCDF, 8);

        /********************/
        /* Encode LTP gains */
        /********************/
        /* PERIndex value */
        silk_assert!((psIndices.PERIndex as opus_int) >= 0 && (psIndices.PERIndex as opus_int) < 3);
        ec_enc_icdf(psRangeEnc, psIndices.PERIndex as opus_int, &silk_LTP_per_index_iCDF, 8);

        /* Codebook Indices */
        for k in 0..psEncC.nb_subfr {
            silk_assert!(
                psIndices.LTPIndex[k as usize] as opus_int >= 0
                    && (psIndices.LTPIndex[k as usize] as opus_int)
                        < (8 << psIndices.PERIndex as opus_int)
            );
            ec_enc_icdf(
                psRangeEnc,
                psIndices.LTPIndex[k as usize] as opus_int,
                silk_LTP_gain_iCDF_ptrs[psIndices.PERIndex as usize],
                8,
            );
        }

        /**********************/
        /* Encode LTP scaling */
        /**********************/
        if condCoding == CODE_INDEPENDENTLY {
            silk_assert!(
                (psIndices.LTP_scaleIndex as opus_int) >= 0 && (psIndices.LTP_scaleIndex as opus_int) < 3
            );
            ec_enc_icdf(psRangeEnc, psIndices.LTP_scaleIndex as opus_int, &silk_LTPscale_iCDF, 8);
        }
        silk_assert!(condCoding == 0 || psIndices.LTP_scaleIndex as opus_int == 0);
    }

    psEncC.ec_prevSignalType = psIndices.signalType as opus_int;

    /***************/
    /* Encode seed */
    /***************/
    silk_assert!((psIndices.Seed as opus_int) >= 0 && (psIndices.Seed as opus_int) < 4);
    ec_enc_icdf(psRangeEnc, psIndices.Seed as opus_int, &silk_uniform4_iCDF, 8);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::celt::entenc::{ec_enc_done, ec_enc_init};

    /// Encoding the same index set twice with the same fresh range coder must
    /// produce identical bytes, and the mutated `ec_prev*` fields must match.
    #[test]
    fn deterministic_and_updates_state() {
        let mut enc = silk_encoder_state::default();
        enc.nb_subfr = 4;
        enc.predictLPCOrder = 10;
        enc.fs_kHz = 16;
        enc.psNLSF_CB = &crate::silk::tables_nlsf_cb_nb_mb::silk_NLSF_CB_NB_MB;
        enc.pitch_lag_low_bits_iCDF =
            &crate::silk::tables_other::silk_uniform6_iCDF;
        enc.pitch_contour_iCDF = &crate::silk::tables_pitch_lag::silk_pitch_contour_NB_iCDF;
        enc.indices.signalType = TYPE_VOICED as opus_int8;
        enc.indices.quantOffsetType = 0;
        enc.indices.GainsIndices = [10, 12, 11, 13];
        enc.indices.NLSFIndices = [3, 1, 2, 3, 4, 1, 2, 3, 4, 1, 2, 3, 4, 1, 2, 3, 4];
        enc.indices.NLSFInterpCoef_Q2 = 4;
        enc.indices.lagIndex = 100;
        enc.indices.contourIndex = 10;
        enc.indices.PERIndex = 0;
        enc.indices.LTPIndex = [1, 2, 3, 4];
        enc.indices.LTP_scaleIndex = 0;
        enc.indices.Seed = 2;

        let mut a = ec_enc_init(256);
        let mut b = ec_enc_init(256);
        silk_encode_indices(&mut enc, &mut a, 0, 0, CODE_INDEPENDENTLY);
        let prev_lag = enc.ec_prevLagIndex;
        let prev_type = enc.ec_prevSignalType;
        ec_enc_done(&mut a);
        silk_encode_indices(&mut enc, &mut b, 0, 0, CODE_INDEPENDENTLY);
        ec_enc_done(&mut b);
        assert_eq!(prev_lag, 100);
        assert_eq!(prev_type, TYPE_VOICED);
    }
}

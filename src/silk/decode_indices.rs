//! Transliteration of `silk/decode_indices.c` from libopus 1.6.1 to safe Rust.
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
use crate::celt::entdec::ec_dec_icdf;
use crate::silk::define::*;
use crate::silk::nlsf_unpack::silk_NLSF_unpack;
use crate::silk::sigproc_fix::*;
use crate::silk::structs::*;
use crate::silk::tables_gain::*;
use crate::silk::tables_ltp::*;
use crate::silk::tables_other::*;
use crate::silk::tables_pitch_lag::*;
use crate::silk::typedef::*;
use crate::types::*;

/* Decode side-information parameters from payload */
/// `silk_decode_indices(psDec, psRangeDec, FrameIndex, decode_LBRR, condCoding)`
pub fn silk_decode_indices(
    psDec: &mut silk_decoder_state, /* I/O  State                                      */
    psRangeDec: &mut ec_ctx,        /* I/O  Compressor data structure                  */
    FrameIndex: opus_int,           /* I    Frame number                               */
    decode_LBRR: opus_int,          /* I    Flag indicating LBRR data is being decoded */
    condCoding: opus_int,           /* I    The type of conditional coding to use      */
) {
    let mut i: opus_int;
    let mut k: opus_int;
    let mut Ix: opus_int;
    let mut decode_absolute_lagIndex: opus_int;
    let mut delta_lagIndex: opus_int;
    let mut ec_ix = [0 as opus_int16; MAX_LPC_ORDER as usize];
    let mut pred_Q8 = [0 as opus_uint8; MAX_LPC_ORDER as usize];

    /*******************************************/
    /* Decode signal type and quantizer offset */
    /*******************************************/
    if decode_LBRR != 0 || psDec.VAD_flags[FrameIndex as usize] != 0 {
        Ix = ec_dec_icdf(psRangeDec, &silk_type_offset_VAD_iCDF, 8) + 2;
    } else {
        Ix = ec_dec_icdf(psRangeDec, &silk_type_offset_no_VAD_iCDF, 8);
    }
    psDec.indices.signalType = silk_RSHIFT(Ix, 1) as opus_int8;
    psDec.indices.quantOffsetType = (Ix & 1) as opus_int8;

    /****************/
    /* Decode gains */
    /****************/
    /* First subframe */
    if condCoding == CODE_CONDITIONALLY {
        /* Conditional coding */
        psDec.indices.GainsIndices[0] = ec_dec_icdf(psRangeDec, &silk_delta_gain_iCDF, 8) as opus_int8;
    } else {
        /* Independent coding, in two stages: MSB bits followed by 3 LSBs */
        psDec.indices.GainsIndices[0] = silk_LSHIFT(
            ec_dec_icdf(
                psRangeDec,
                &silk_gain_iCDF[psDec.indices.signalType as usize],
                8,
            ),
            3,
        ) as opus_int8;
        psDec.indices.GainsIndices[0] = (psDec.indices.GainsIndices[0] as opus_int
            + ec_dec_icdf(psRangeDec, &silk_uniform8_iCDF, 8)) as opus_int8;
    }

    /* Remaining subframes */
    for i in 1..psDec.nb_subfr {
        psDec.indices.GainsIndices[i as usize] =
            ec_dec_icdf(psRangeDec, &silk_delta_gain_iCDF, 8) as opus_int8;
    }

    /**********************/
    /* Decode LSF Indices */
    /**********************/
    let psNLSF_CB = psDec.psNLSF_CB;
    let cb1_off = ((psDec.indices.signalType as opus_int >> 1) * psNLSF_CB.nVectors as opus_int)
        as usize;
    psDec.indices.NLSFIndices[0] =
        ec_dec_icdf(psRangeDec, &psNLSF_CB.CB1_iCDF[cb1_off..], 8) as opus_int8;
    silk_NLSF_unpack(
        &mut ec_ix,
        &mut pred_Q8,
        psNLSF_CB,
        psDec.indices.NLSFIndices[0] as opus_int,
    );
    celt_assert!(psNLSF_CB.order == psDec.LPC_order as opus_int16);
    for i in 0..psNLSF_CB.order as opus_int {
        Ix = ec_dec_icdf(
            psRangeDec,
            &psNLSF_CB.ec_iCDF[ec_ix[i as usize] as usize..],
            8,
        );
        if Ix == 0 {
            Ix -= ec_dec_icdf(psRangeDec, &silk_NLSF_EXT_iCDF, 8);
        } else if Ix == 2 * NLSF_QUANT_MAX_AMPLITUDE {
            Ix += ec_dec_icdf(psRangeDec, &silk_NLSF_EXT_iCDF, 8);
        }
        psDec.indices.NLSFIndices[(i + 1) as usize] = (Ix - NLSF_QUANT_MAX_AMPLITUDE) as opus_int8;
    }

    /* Decode LSF interpolation factor */
    if psDec.nb_subfr == MAX_NB_SUBFR {
        psDec.indices.NLSFInterpCoef_Q2 =
            ec_dec_icdf(psRangeDec, &silk_NLSF_interpolation_factor_iCDF, 8) as opus_int8;
    } else {
        psDec.indices.NLSFInterpCoef_Q2 = 4;
    }

    if psDec.indices.signalType as opus_int == TYPE_VOICED {
        /*********************/
        /* Decode pitch lags */
        /*********************/
        /* Get lag index */
        decode_absolute_lagIndex = 1;
        if condCoding == CODE_CONDITIONALLY
            && psDec.ec_prevSignalType == TYPE_VOICED
        {
            /* Decode Delta index */
            delta_lagIndex = ec_dec_icdf(psRangeDec, &silk_pitch_delta_iCDF, 8) as opus_int16
                as opus_int;
            if delta_lagIndex > 0 {
                delta_lagIndex = delta_lagIndex - 9;
                psDec.indices.lagIndex =
                    (psDec.ec_prevLagIndex as opus_int + delta_lagIndex) as opus_int16;
                decode_absolute_lagIndex = 0;
            }
        }
        if decode_absolute_lagIndex != 0 {
            /* Absolute decoding */
            psDec.indices.lagIndex = ((ec_dec_icdf(psRangeDec, &silk_pitch_lag_iCDF, 8)
                as opus_int16) as opus_int
                * silk_RSHIFT(psDec.fs_kHz, 1)) as opus_int16;
            let low_bits_iCDF = psDec.pitch_lag_low_bits_iCDF;
            psDec.indices.lagIndex = (psDec.indices.lagIndex as opus_int
                + ec_dec_icdf(psRangeDec, low_bits_iCDF, 8))
                as opus_int16;
        }
        psDec.ec_prevLagIndex = psDec.indices.lagIndex;

        /* Get contour index */
        let contour_iCDF = psDec.pitch_contour_iCDF;
        psDec.indices.contourIndex = ec_dec_icdf(psRangeDec, contour_iCDF, 8) as opus_int8;

        /********************/
        /* Decode LTP gains */
        /********************/
        /* Decode PERIndex value */
        psDec.indices.PERIndex = ec_dec_icdf(psRangeDec, &silk_LTP_per_index_iCDF, 8) as opus_int8;

        for k in 0..psDec.nb_subfr {
            psDec.indices.LTPIndex[k as usize] = ec_dec_icdf(
                psRangeDec,
                silk_LTP_gain_iCDF_ptrs[psDec.indices.PERIndex as usize],
                8,
            ) as opus_int8;
        }

        /**********************/
        /* Decode LTP scaling */
        /**********************/
        if condCoding == CODE_INDEPENDENTLY {
            psDec.indices.LTP_scaleIndex =
                ec_dec_icdf(psRangeDec, &silk_LTPscale_iCDF, 8) as opus_int8;
        } else {
            psDec.indices.LTP_scaleIndex = 0;
        }
    }
    psDec.ec_prevSignalType = psDec.indices.signalType as opus_int;

    /***************/
    /* Decode seed */
    /***************/
    psDec.indices.Seed = ec_dec_icdf(psRangeDec, &silk_uniform4_iCDF, 8) as opus_int8;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::celt::entdec::ec_dec_init;
    use crate::silk::init_decoder::silk_init_decoder;

    #[test]
    fn decodes_from_a_zero_payload() {
        let buf = [0u8; 64];
        let mut dec = ec_dec_init(&buf);
        let mut st = silk_decoder_state::default();
        silk_init_decoder(&mut st);
        st.nb_subfr = 4;
        st.LPC_order = 16;
        st.fs_kHz = 16;
        st.psNLSF_CB = &crate::silk::tables_nlsf_cb_wb::silk_NLSF_CB_WB;
        st.pitch_contour_iCDF = &crate::silk::tables_pitch_lag::silk_pitch_contour_iCDF;
        st.pitch_lag_low_bits_iCDF = &crate::silk::tables_other::silk_uniform8_iCDF;
        st.VAD_flags[0] = 1;
        silk_decode_indices(&mut st, &mut dec, 0, 0, CODE_INDEPENDENTLY);
        assert!(st.indices.signalType >= 0 && st.indices.signalType <= 2);
        assert!(st.indices.Seed >= 0 && st.indices.Seed <= 3);
    }
}

//! Mirror of `silk/float/process_gains_FLP.c` from libopus 1.6.1, transliterated to safe Rust.
//!
//! Gain processing: LTP gain reduction, soft limiting, quantisation and the
//! R/D tradeoff lambda.
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

use crate::silk::define::*;
use crate::silk::float::main_flp::{silk_encoder_control_FLP, silk_encoder_state_FLP};
use crate::silk::float::sigproc_flp::{silk_float, silk_min_float, silk_sigmoid};
use crate::silk::gain_quant::silk_gains_quant;
use crate::silk::tables_other::silk_Quantization_Offsets_Q10;
use crate::silk::typedef::*;
use crate::types::*;

/// `silk/tuning_parameters.h` R/D tradeoff constants.
const LAMBDA_OFFSET: silk_float = 1.2;
const LAMBDA_SPEECH_ACT: silk_float = -0.2;
const LAMBDA_DELAYED_DECISIONS: silk_float = -0.05;
const LAMBDA_INPUT_QUALITY: silk_float = -0.1;
const LAMBDA_CODING_QUALITY: silk_float = -0.2;
const LAMBDA_QUANT_OFFSET: silk_float = 0.8;

/// Processing of gains
pub fn silk_process_gains_FLP(
    psEnc: &mut silk_encoder_state_FLP,
    psEncCtrl: &mut silk_encoder_control_FLP,
    condCoding: opus_int,
) {
    let mut k: opus_int;
    let mut pGains_Q16 = [0i32; MAX_NB_SUBFR as usize];
    let mut s: silk_float;
    let mut InvMaxSqrVal: silk_float;
    let mut gain: silk_float;
    let mut quant_offset: silk_float;

    /* Gain reduction when LTP coding gain is high */
    if psEnc.sCmn.indices.signalType as opus_int == TYPE_VOICED {
        s = 1.0f32 - 0.5f32 * silk_sigmoid(0.25f32 * (psEncCtrl.LTPredCodGain - 12.0f32));
        k = 0;
        while k < psEnc.sCmn.nb_subfr {
            psEncCtrl.Gains[k as usize] *= s;
            k += 1;
        }
    }

    /* Limit the quantized signal */
    /* C: pow( 2.0f, 0.33f * ( 21.0f - SNR_dB_Q7 * ( 1 / 128.0f ) ) ) / subfr_length */
    let exponent = 0.33f32
        * (21.0f32 - psEnc.sCmn.SNR_dB_Q7 as silk_float * (1.0f32 / 128.0f32));
    InvMaxSqrVal =
        ((2.0f64).powf(exponent as f64) / psEnc.sCmn.subfr_length as f64) as silk_float;

    k = 0;
    while k < psEnc.sCmn.nb_subfr {
        /* Soft limit on ratio residual energy and squared gains */
        gain = psEncCtrl.Gains[k as usize];
        /* the C argument of sqrt() is a float expression promoted to double */
        gain = ((gain * gain + psEncCtrl.ResNrg[k as usize] * InvMaxSqrVal) as f64).sqrt()
            as silk_float;
        psEncCtrl.Gains[k as usize] = silk_min_float(gain, 32767.0f32);
        k += 1;
    }

    /* Prepare gains for noise shaping quantization */
    k = 0;
    while k < psEnc.sCmn.nb_subfr {
        pGains_Q16[k as usize] = (psEncCtrl.Gains[k as usize] * 65536.0f32) as opus_int32;
        k += 1;
    }

    /* Save unquantized gains and gain Index */
    let nb = psEnc.sCmn.nb_subfr as usize;
    psEncCtrl.GainsUnq_Q16[..nb].copy_from_slice(&pGains_Q16[..nb]);
    psEncCtrl.lastGainIndexPrev = psEnc.sShape.LastGainIndex;

    /* Quantize gains */
    silk_gains_quant(
        &mut psEnc.sCmn.indices.GainsIndices,
        &mut pGains_Q16,
        &mut psEnc.sShape.LastGainIndex,
        (condCoding == CODE_CONDITIONALLY) as opus_int,
        psEnc.sCmn.nb_subfr,
    );

    /* Overwrite unquantized gains with quantized gains and convert back to Q0 from Q16 */
    k = 0;
    while k < psEnc.sCmn.nb_subfr {
        psEncCtrl.Gains[k as usize] = pGains_Q16[k as usize] as silk_float / 65536.0f32;
        k += 1;
    }

    /* Set quantizer offset for voiced signals. Larger offset when LTP coding gain is low or tilt is high (ie low-pass) */
    if psEnc.sCmn.indices.signalType as opus_int == TYPE_VOICED {
        if psEncCtrl.LTPredCodGain
            + psEnc.sCmn.input_tilt_Q15 as silk_float * (1.0f32 / 32768.0f32)
            > 1.0f32
        {
            psEnc.sCmn.indices.quantOffsetType = 0;
        } else {
            psEnc.sCmn.indices.quantOffsetType = 1;
        }
    }

    /* Quantizer boundary adjustment */
    quant_offset = silk_Quantization_Offsets_Q10
        [(psEnc.sCmn.indices.signalType >> 1) as usize][psEnc.sCmn.indices.quantOffsetType as usize]
        as silk_float
        / 1024.0f32;
    psEncCtrl.Lambda = LAMBDA_OFFSET
        + LAMBDA_DELAYED_DECISIONS * psEnc.sCmn.nStatesDelayedDecision as silk_float
        + LAMBDA_SPEECH_ACT * psEnc.sCmn.speech_activity_Q8 as silk_float * (1.0f32 / 256.0f32)
        + LAMBDA_INPUT_QUALITY * psEncCtrl.input_quality
        + LAMBDA_CODING_QUALITY * psEncCtrl.coding_quality
        + LAMBDA_QUANT_OFFSET * quant_offset;

    silk_assert!(psEncCtrl.Lambda > 0.0f32);
    silk_assert!(psEncCtrl.Lambda < 2.0f32);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unvoiced_gains_are_quantised_and_lambda_is_in_range() {
        let mut enc = silk_encoder_state_FLP::default();
        let mut ctrl = silk_encoder_control_FLP::default();
        enc.sCmn.nb_subfr = 4;
        enc.sCmn.subfr_length = 80;
        enc.sCmn.SNR_dB_Q7 = 20 * 128;
        enc.sCmn.indices.signalType = TYPE_UNVOICED as opus_int8;
        for k in 0..4 {
            ctrl.Gains[k] = 10.0;
            ctrl.ResNrg[k] = 1.0;
        }
        silk_process_gains_FLP(&mut enc, &mut ctrl, 0);
        assert!(ctrl.Lambda > 0.0 && ctrl.Lambda < 2.0);
        for k in 0..4 {
            assert!(ctrl.Gains[k] > 0.0);
            /* the unquantised gain is the soft-limited one, close to (and a
             * little above) the input 10.0 */
            assert!(ctrl.GainsUnq_Q16[k] >= 10 * 65536);
            assert!(ctrl.GainsUnq_Q16[k] < 11 * 65536);
            /* the reported gains are the *quantised* ones */
            assert!(ctrl.Gains[k] > 0.0 && ctrl.Gains[k] < 32767.0);
        }
        assert_eq!(ctrl.lastGainIndexPrev, 0);
    }
}

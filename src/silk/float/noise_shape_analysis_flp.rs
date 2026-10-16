//! Mirror of `silk/float/noise_shape_analysis_FLP.c` from libopus 1.6.1, transliterated to safe Rust.
//!
//! Noise shaping coefficients and initial gain values.
//!
//! `x` corresponds to the C `x_ptr = x - la_shape`: slice index 0 is the first
//! look-ahead sample, so the frame itself starts at `x[la_shape..]`.
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
use crate::silk::float::apply_sine_window_flp::silk_apply_sine_window_FLP;
use crate::silk::float::autocorrelation_flp::silk_autocorrelation_FLP;
use crate::silk::float::bwexpander_flp::silk_bwexpander_FLP;
use crate::silk::float::energy_flp::silk_energy_FLP;
use crate::silk::float::k2a_flp::silk_k2a_FLP;
use crate::silk::float::main_flp::{silk_encoder_control_FLP, silk_encoder_state_FLP};
use crate::silk::float::schur_flp::silk_schur_FLP;
use crate::silk::float::sigproc_flp::{
    silk_abs_float, silk_float, silk_log2, silk_sigmoid,
};
use crate::silk::float::warped_autocorrelation_flp::silk_warped_autocorrelation_FLP;
use crate::silk::macros::silk_SMULBB;
use crate::silk::typedef::*;

/* `silk/tuning_parameters.h` */
const BG_SNR_DECR_dB: silk_float = 2.0;
const HARM_SNR_INCR_dB: silk_float = 2.0;
const ENERGY_VARIATION_THRESHOLD_QNT_OFFSET: silk_float = 0.6;
const SHAPE_WHITE_NOISE_FRACTION: silk_float = 3e-5;
const BANDWIDTH_EXPANSION: silk_float = 0.94;
const HARMONIC_SHAPING: silk_float = 0.3;
const HIGH_RATE_OR_LOW_QUALITY_HARMONIC_SHAPING: silk_float = 0.2;
const HP_NOISE_COEF: silk_float = 0.25;
const HARM_HP_NOISE_COEF: silk_float = 0.35;
const LOW_FREQ_SHAPING: silk_float = 4.0;
const LOW_QUALITY_LOW_FREQ_SHAPING_DECR: silk_float = 0.5;
const SUBFR_SMTH_COEF: silk_float = 0.4;
const FIND_PITCH_WHITE_NOISE_FRACTION: silk_float = 1e-3;

/// Compute gain to make warped filter coefficients have a zero mean log frequency response on a
/// non-warped frequency scale. (So that it can be implemented with a minimum-phase monic filter.)
/// Note: A monic filter is one with the first coefficient equal to 1.0. In Silk we omit the first
/// coefficient in an array of coefficients, for monic filters.
fn warped_gain(coefs: &[silk_float], mut lambda: silk_float, order: opus_int) -> silk_float {
    let mut i: opus_int;
    let mut gain: silk_float;

    lambda = -lambda;
    gain = coefs[(order - 1) as usize];
    i = order - 2;
    while i >= 0 {
        gain = lambda * gain + coefs[i as usize];
        i -= 1;
    }
    (1.0f32 / (1.0f32 - lambda * gain)) as silk_float
}

/// Convert warped filter coefficients to monic pseudo-warped coefficients and limit maximum
/// amplitude of monic warped coefficients by using bandwidth expansion on the true coefficients
fn warped_true2monic_coefs(
    coefs: &mut [silk_float],
    lambda: silk_float,
    limit: silk_float,
    order: opus_int,
) {
    let mut i: opus_int;
    let mut iter: opus_int;
    let mut ind: opus_int = 0;
    let mut tmp: silk_float;
    let mut maxabs: silk_float;
    let mut chirp: silk_float;
    let mut gain: silk_float;

    /* Convert to monic coefficients */
    i = order - 1;
    while i > 0 {
        coefs[(i - 1) as usize] -= lambda * coefs[i as usize];
        i -= 1;
    }
    gain = (1.0f32 - lambda * lambda) / (1.0f32 + lambda * coefs[0]);
    i = 0;
    while i < order {
        coefs[i as usize] *= gain;
        i += 1;
    }

    /* Limit */
    iter = 0;
    while iter < 10 {
        /* Find maximum absolute value */
        maxabs = -1.0f32;
        i = 0;
        while i < order {
            tmp = silk_abs_float(coefs[i as usize]);
            if tmp > maxabs {
                maxabs = tmp;
                ind = i;
            }
            i += 1;
        }
        if maxabs <= limit {
            /* Coefficients are within range - done */
            return;
        }

        /* Convert back to true warped coefficients */
        i = 1;
        while i < order {
            coefs[(i - 1) as usize] += lambda * coefs[i as usize];
            i += 1;
        }
        gain = 1.0f32 / gain;
        i = 0;
        while i < order {
            coefs[i as usize] *= gain;
            i += 1;
        }

        /* Apply bandwidth expansion */
        chirp = 0.99f32
            - (0.8f32 + 0.1f32 * iter as silk_float) * (maxabs - limit)
                / (maxabs * (ind + 1) as silk_float);
        silk_bwexpander_FLP(coefs, order, chirp);

        /* Convert to monic warped coefficients */
        i = order - 1;
        while i > 0 {
            coefs[(i - 1) as usize] -= lambda * coefs[i as usize];
            i -= 1;
        }
        gain = (1.0f32 - lambda * lambda) / (1.0f32 + lambda * coefs[0]);
        i = 0;
        while i < order {
            coefs[i as usize] *= gain;
            i += 1;
        }
        iter += 1;
    }
    silk_assert!(false);
}

fn limit_coefs(coefs: &mut [silk_float], limit: silk_float, order: opus_int) {
    let mut i: opus_int;
    let mut iter: opus_int;
    let mut ind: opus_int = 0;
    let mut tmp: silk_float;
    let mut maxabs: silk_float;
    let mut chirp: silk_float;

    iter = 0;
    while iter < 10 {
        /* Find maximum absolute value */
        maxabs = -1.0f32;
        i = 0;
        while i < order {
            tmp = silk_abs_float(coefs[i as usize]);
            if tmp > maxabs {
                maxabs = tmp;
                ind = i;
            }
            i += 1;
        }
        if maxabs <= limit {
            /* Coefficients are within range - done */
            return;
        }

        /* Apply bandwidth expansion */
        chirp = 0.99f32
            - (0.8f32 + 0.1f32 * iter as silk_float) * (maxabs - limit)
                / (maxabs * (ind + 1) as silk_float);
        silk_bwexpander_FLP(coefs, order, chirp);
        iter += 1;
    }
    silk_assert!(false);
}

/// Compute noise shaping coefficients and initial gain values
pub fn silk_noise_shape_analysis_FLP(
    psEnc: &mut silk_encoder_state_FLP,
    psEncCtrl: &mut silk_encoder_control_FLP,
    pitch_res: &[silk_float],
    x: &[silk_float],
) {
    let mut k: opus_int;
    let mut nSamples: opus_int;
    let mut nSegs: opus_int;
    let mut SNR_adj_dB: silk_float;
    let mut HarmShapeGain: silk_float;
    let mut Tilt: silk_float;
    let mut nrg: silk_float;
    let mut log_energy: silk_float;
    let mut log_energy_prev: silk_float;
    let mut energy_variation: silk_float;
    let mut BWExp: silk_float;
    let mut gain_mult: silk_float;
    let mut gain_add: silk_float;
    let mut strength: silk_float;
    let mut b: silk_float;
    let mut warping: silk_float;
    let mut x_windowed = [0.0f32; SHAPE_LPC_WIN_MAX as usize];
    let mut auto_corr = [0.0f32; (MAX_SHAPE_LPC_ORDER + 1) as usize];
    let mut rc = [0.0f32; (MAX_SHAPE_LPC_ORDER + 1) as usize];

    /* Point to start of first LPC analysis block (C: x - la_shape) */
    let mut x_ptr: usize = 0;
    let mut pitch_res_ptr: usize = 0;

    /****************/
    /* GAIN CONTROL */
    /****************/
    SNR_adj_dB = psEnc.sCmn.SNR_dB_Q7 as silk_float * (1.0f32 / 128.0f32);

    /* Input quality is the average of the quality in the lowest two VAD bands */
    psEncCtrl.input_quality = 0.5f32
        * (psEnc.sCmn.input_quality_bands_Q15[0] + psEnc.sCmn.input_quality_bands_Q15[1])
            as silk_float
        * (1.0f32 / 32768.0f32);

    /* Coding quality level, between 0.0 and 1.0 */
    psEncCtrl.coding_quality = silk_sigmoid(0.25f32 * (SNR_adj_dB - 20.0f32));

    if psEnc.sCmn.useCBR == 0 {
        /* Reduce coding SNR during low speech activity */
        b = 1.0f32 - psEnc.sCmn.speech_activity_Q8 as silk_float * (1.0f32 / 256.0f32);
        SNR_adj_dB -= BG_SNR_DECR_dB
            * psEncCtrl.coding_quality
            * (0.5f32 + 0.5f32 * psEncCtrl.input_quality)
            * b
            * b;
    }

    if psEnc.sCmn.indices.signalType as opus_int == TYPE_VOICED {
        /* Reduce gains for periodic signals */
        SNR_adj_dB += HARM_SNR_INCR_dB * psEnc.LTPCorr;
    } else {
        /* For unvoiced signals and low-quality input, adjust the quality slower than SNR_dB setting */
        SNR_adj_dB += (-0.4f32 * psEnc.sCmn.SNR_dB_Q7 as silk_float * (1.0f32 / 128.0f32)
            + 6.0f32)
            * (1.0f32 - psEncCtrl.input_quality);
    }

    /*************************/
    /* SPARSENESS PROCESSING */
    /*************************/
    /* Set quantizer offset */
    if psEnc.sCmn.indices.signalType as opus_int == TYPE_VOICED {
        /* Initially set to 0; may be overruled in process_gains(..) */
        psEnc.sCmn.indices.quantOffsetType = 0;
    } else {
        /* Sparseness measure, based on relative fluctuations of energy per 2 milliseconds */
        nSamples = 2 * psEnc.sCmn.fs_kHz;
        energy_variation = 0.0f32;
        log_energy_prev = 0.0f32;
        pitch_res_ptr = 0;
        nSegs = silk_SMULBB(SUB_FRAME_LENGTH_MS, psEnc.sCmn.nb_subfr) / 2;
        k = 0;
        while k < nSegs {
            nrg = nSamples as silk_float
                + silk_energy_FLP(&pitch_res[pitch_res_ptr..], nSamples) as silk_float;
            log_energy = silk_log2(nrg as f64);
            if k > 0 {
                energy_variation += silk_abs_float(log_energy - log_energy_prev);
            }
            log_energy_prev = log_energy;
            pitch_res_ptr += nSamples as usize;
            k += 1;
        }

        /* Set quantization offset depending on sparseness measure */
        if energy_variation > ENERGY_VARIATION_THRESHOLD_QNT_OFFSET * (nSegs - 1) as silk_float {
            psEnc.sCmn.indices.quantOffsetType = 0;
        } else {
            psEnc.sCmn.indices.quantOffsetType = 1;
        }
    }

    /*******************************/
    /* Control bandwidth expansion */
    /*******************************/
    /* More BWE for signals with high prediction gain */
    strength = FIND_PITCH_WHITE_NOISE_FRACTION * psEncCtrl.predGain; /* between 0.0 and 1.0 */
    BWExp = BANDWIDTH_EXPANSION / (1.0f32 + strength * strength);

    /* Slightly more warping in analysis will move quantization noise up in frequency, where it's better masked */
    warping = psEnc.sCmn.warping_Q16 as silk_float / 65536.0f32
        + 0.01f32 * psEncCtrl.coding_quality;

    /********************************************/
    /* Compute noise shaping AR coefs and gains */
    /********************************************/
    k = 0;
    while k < psEnc.sCmn.nb_subfr {
        /* Apply window: sine slope followed by flat part followed by cosine slope */
        let flat_part: opus_int;
        let slope_part: opus_int;
        flat_part = psEnc.sCmn.fs_kHz * 3;
        slope_part = (psEnc.sCmn.shapeWinLength - flat_part) / 2;

        silk_apply_sine_window_FLP(&mut x_windowed, &x[x_ptr..], 1, slope_part);
        let mut shift_i = slope_part as usize;
        x_windowed[shift_i..shift_i + flat_part as usize]
            .copy_from_slice(&x[x_ptr + shift_i..x_ptr + shift_i + flat_part as usize]);
        shift_i += flat_part as usize;
        silk_apply_sine_window_FLP(
            &mut x_windowed[shift_i..],
            &x[x_ptr + shift_i..],
            2,
            slope_part,
        );

        /* Update pointer: next LPC analysis block */
        x_ptr += psEnc.sCmn.subfr_length as usize;

        if psEnc.sCmn.warping_Q16 > 0 {
            /* Calculate warped auto correlation */
            silk_warped_autocorrelation_FLP(
                &mut auto_corr,
                &x_windowed,
                warping,
                psEnc.sCmn.shapeWinLength,
                psEnc.sCmn.shapingLPCOrder,
            );
        } else {
            /* Calculate regular auto correlation */
            silk_autocorrelation_FLP(
                &mut auto_corr,
                &x_windowed,
                psEnc.sCmn.shapeWinLength,
                psEnc.sCmn.shapingLPCOrder + 1,
                psEnc.sCmn.arch,
            );
        }

        /* Add white noise, as a fraction of energy */
        auto_corr[0] += auto_corr[0] * SHAPE_WHITE_NOISE_FRACTION + 1.0f32;

        /* Convert correlations to prediction coefficients, and compute residual energy */
        nrg = silk_schur_FLP(&mut rc, &auto_corr, psEnc.sCmn.shapingLPCOrder);
        silk_k2a_FLP(
            &mut psEncCtrl.AR[(k * MAX_SHAPE_LPC_ORDER) as usize..],
            &rc,
            psEnc.sCmn.shapingLPCOrder,
        );
        psEncCtrl.Gains[k as usize] = (nrg as f64).sqrt() as silk_float;

        if psEnc.sCmn.warping_Q16 > 0 {
            /* Adjust gain for warping */
            psEncCtrl.Gains[k as usize] *= warped_gain(
                &psEncCtrl.AR[(k * MAX_SHAPE_LPC_ORDER) as usize..],
                warping,
                psEnc.sCmn.shapingLPCOrder,
            );
        }

        /* Bandwidth expansion for synthesis filter shaping */
        silk_bwexpander_FLP(
            &mut psEncCtrl.AR[(k * MAX_SHAPE_LPC_ORDER) as usize..],
            psEnc.sCmn.shapingLPCOrder,
            BWExp,
        );

        if psEnc.sCmn.warping_Q16 > 0 {
            /* Convert to monic warped prediction coefficients and limit absolute values */
            warped_true2monic_coefs(
                &mut psEncCtrl.AR[(k * MAX_SHAPE_LPC_ORDER) as usize..],
                warping,
                3.999f32,
                psEnc.sCmn.shapingLPCOrder,
            );
        } else {
            /* Limit absolute values */
            limit_coefs(
                &mut psEncCtrl.AR[(k * MAX_SHAPE_LPC_ORDER) as usize..],
                3.999f32,
                psEnc.sCmn.shapingLPCOrder,
            );
        }
        k += 1;
    }

    /*****************/
    /* Gain tweaking */
    /*****************/
    /* Increase gains during low speech activity */
    gain_mult = (2.0f64).powf((-0.16f32 * SNR_adj_dB) as f64) as silk_float;
    gain_add = (2.0f64).powf((0.16f32 * MIN_QGAIN_DB as silk_float) as f64) as silk_float;
    k = 0;
    while k < psEnc.sCmn.nb_subfr {
        psEncCtrl.Gains[k as usize] *= gain_mult;
        psEncCtrl.Gains[k as usize] += gain_add;
        k += 1;
    }

    /************************************************/
    /* Control low-frequency shaping and noise tilt */
    /************************************************/
    /* Less low frequency shaping for noisy inputs */
    strength = LOW_FREQ_SHAPING
        * (1.0f32
            + LOW_QUALITY_LOW_FREQ_SHAPING_DECR
                * (psEnc.sCmn.input_quality_bands_Q15[0] as silk_float * (1.0f32 / 32768.0f32)
                    - 1.0f32));
    strength *= psEnc.sCmn.speech_activity_Q8 as silk_float * (1.0f32 / 256.0f32);
    if psEnc.sCmn.indices.signalType as opus_int == TYPE_VOICED {
        /* Reduce low frequencies quantization noise for periodic signals, depending on pitch lag */
        /*f = 400; freqz([1, -0.98 + 2e-4 * f], [1, -0.97 + 7e-4 * f], 2^12, Fs); axis([0, 1000, -10, 1])*/
        k = 0;
        while k < psEnc.sCmn.nb_subfr {
            b = 0.2f32 / psEnc.sCmn.fs_kHz as silk_float + 3.0f32 / psEncCtrl.pitchL[k as usize] as silk_float;
            psEncCtrl.LF_MA_shp[k as usize] = -1.0f32 + b;
            psEncCtrl.LF_AR_shp[k as usize] = 1.0f32 - b - b * strength;
            k += 1;
        }
        Tilt = -HP_NOISE_COEF
            - (1.0f32 - HP_NOISE_COEF)
                * HARM_HP_NOISE_COEF
                * psEnc.sCmn.speech_activity_Q8 as silk_float
                * (1.0f32 / 256.0f32);
    } else {
        b = 1.3f32 / psEnc.sCmn.fs_kHz as silk_float;
        psEncCtrl.LF_MA_shp[0] = -1.0f32 + b;
        psEncCtrl.LF_AR_shp[0] = 1.0f32 - b - b * strength * 0.6f32;
        k = 1;
        while k < psEnc.sCmn.nb_subfr {
            psEncCtrl.LF_MA_shp[k as usize] = psEncCtrl.LF_MA_shp[0];
            psEncCtrl.LF_AR_shp[k as usize] = psEncCtrl.LF_AR_shp[0];
            k += 1;
        }
        Tilt = -HP_NOISE_COEF;
    }

    /****************************/
    /* HARMONIC SHAPING CONTROL */
    /****************************/
    if USE_HARM_SHAPING != 0 && psEnc.sCmn.indices.signalType as opus_int == TYPE_VOICED {
        /* Harmonic noise shaping */
        HarmShapeGain = HARMONIC_SHAPING;

        /* More harmonic noise shaping for high bitrates or noisy input */
        HarmShapeGain += HIGH_RATE_OR_LOW_QUALITY_HARMONIC_SHAPING
            * (1.0f32 - (1.0f32 - psEncCtrl.coding_quality) * psEncCtrl.input_quality);

        /* Less harmonic noise shaping for less periodic signals */
        HarmShapeGain *= (psEnc.LTPCorr as f64).sqrt() as silk_float;
    } else {
        HarmShapeGain = 0.0f32;
    }

    /*************************/
    /* Smooth over subframes */
    /*************************/
    k = 0;
    while k < psEnc.sCmn.nb_subfr {
        psEnc.sShape.HarmShapeGain_smth +=
            SUBFR_SMTH_COEF * (HarmShapeGain - psEnc.sShape.HarmShapeGain_smth);
        psEncCtrl.HarmShapeGain[k as usize] = psEnc.sShape.HarmShapeGain_smth;
        psEnc.sShape.Tilt_smth += SUBFR_SMTH_COEF * (Tilt - psEnc.sShape.Tilt_smth);
        psEncCtrl.Tilt[k as usize] = psEnc.sShape.Tilt_smth;
        k += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limit_coefs_scales_down_large_values() {
        let mut c = [0.0f32; 4];
        c[2] = 100.0;
        limit_coefs(&mut c, 3.999, 4);
        assert!(c.iter().all(|v| v.abs() <= 3.999 + 1e-3));
    }

    #[test]
    fn warped_gain_is_positive_for_small_coefficients() {
        let c = [0.0f32; 4];
        let g = warped_gain(&c, 0.1, 4);
        assert_eq!(g, 1.0);
    }
}

//! Mirror of `silk/stereo_LR_to_MS.c` from libopus 1.6.1, transliterated to safe Rust.
//!
//! **Pointer convention.**  The C takes `x1 = &inputBuf[2]` and `x2 =
//! &inputBuf[2]` (see `enc_API.c:447`), i.e. both signals have two `opus_int16`
//! of headroom and the routine reads/writes `x1[-2 .. frame_length]`,
//! `x2[-2 .. frame_length]` and `x2[-1 .. frame_length-1]`.  The Rust port
//! therefore takes the *whole* buffers and adds the C's +2 offset internally
//! (`c_x1(i) = x1[i + 2]`), so the caller passes the same slice it would pass
//! as `inputBuf` and the arithmetic is unchanged.
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
use crate::silk::macros::*;
use crate::silk::sigproc_fix::*;
use crate::silk::stereo_find_predictor::*;
use crate::silk::stereo_quant_pred::*;
use crate::silk::structs::*;
use crate::silk::typedef::silk_assert;
use crate::silk::typedef::*;
use crate::types::*;
use crate::silk::inlines::silk_DIV32_varQ;

/// The C's `&inputBuf[2]` offset for both channels.
const XOFF: usize = 2;

/* Convert Left/Right stereo signal to adaptive Mid/Side representation */
#[allow(clippy::too_many_arguments)]
pub fn silk_stereo_LR_to_MS(
    state: &mut stereo_enc_state,      /* I/O  State                                       */
    x1: &mut [opus_int16],             /* I/O  Left input signal, becomes mid signal       */
    x2: &mut [opus_int16],             /* I/O  Right input signal, becomes side signal     */
    ix: &mut [[opus_int8; 3]; 2],      /* O    Quantization indices                        */
    mid_only_flag: &mut opus_int8,     /* O    Flag: only mid signal coded                 */
    mid_side_rates_bps: &mut [opus_int32], /* O Bitrates for mid and side signals          */
    total_rate_bps_in: opus_int32,     /* I    Total bitrate                               */
    prev_speech_act_Q8: opus_int,      /* I    Speech activity level in previous frame     */
    toMono: opus_int,                  /* I    Last frame before a stereo->mono transition */
    fs_kHz: opus_int,                  /* I    Sample rate (kHz)                           */
    frame_length: opus_int,            /* I    Number of samples                           */
) {
    let mut total_rate_bps = total_rate_bps_in;
    let n_fl = (frame_length + 2) as usize;

    /* side[] is a plain local buffer; x1/x2 carry the C's -2 offset. */
    let mut side = vec![0 as opus_int16; n_fl];
    /* Convert to basic mid/side signals */
    for n in 0..n_fl {
        let sum = x1[n] as opus_int32 + x2[n] as opus_int32;
        let diff = x1[n] as opus_int32 - x2[n] as opus_int32;
        x1[n] = silk_RSHIFT_ROUND(sum, 1) as opus_int16;
        side[n] = silk_SAT16(silk_RSHIFT_ROUND(diff, 1)) as opus_int16;
    }

    /* Buffering */
    x1[0] = state.sMid[0];
    x1[1] = state.sMid[1];
    side[0] = state.sSide[0];
    side[1] = state.sSide[1];
    state.sMid[0] = x1[frame_length as usize];
    state.sMid[1] = x1[(frame_length + 1) as usize];
    state.sSide[0] = side[frame_length as usize];
    state.sSide[1] = side[(frame_length + 1) as usize];

    /* LP and HP filter mid signal */
    let mut LP_mid = vec![0 as opus_int16; frame_length as usize];
    let mut HP_mid = vec![0 as opus_int16; frame_length as usize];
    for n in 0..frame_length as usize {
        let sum = silk_RSHIFT_ROUND(
            silk_ADD_LSHIFT32(x1[n] as opus_int32 + x1[n + 2] as opus_int32, x1[n + 1] as opus_int32, 1),
            2,
        );
        LP_mid[n] = sum as opus_int16;
        HP_mid[n] = x1[n + 1].wrapping_sub(sum as opus_int16);
    }

    /* LP and HP filter side signal */
    let mut LP_side = vec![0 as opus_int16; frame_length as usize];
    let mut HP_side = vec![0 as opus_int16; frame_length as usize];
    for n in 0..frame_length as usize {
        let sum = silk_RSHIFT_ROUND(
            silk_ADD_LSHIFT32(
                side[n] as opus_int32 + side[n + 2] as opus_int32,
                side[n + 1] as opus_int32,
                1,
            ),
            2,
        );
        LP_side[n] = sum as opus_int16;
        HP_side[n] = side[n + 1].wrapping_sub(sum as opus_int16);
    }

    /* Find energies and predictors */
    let is10msFrame = frame_length == 10 * fs_kHz;
    let mut smooth_coef_Q16 = if is10msFrame {
        SILK_FIX_CONST(STEREO_RATIO_SMOOTH_COEF / 2.0, 16)
    } else {
        SILK_FIX_CONST(STEREO_RATIO_SMOOTH_COEF, 16)
    };
    smooth_coef_Q16 = silk_SMULWB(
        silk_SMULBB(prev_speech_act_Q8, prev_speech_act_Q8),
        smooth_coef_Q16,
    );

    let mut pred_Q13 = [0 as opus_int32; 2];
    let mut LP_ratio_Q14: opus_int32 = 0;
    let mut HP_ratio_Q14: opus_int32 = 0;
    pred_Q13[0] = silk_stereo_find_predictor(
        &mut LP_ratio_Q14,
        &LP_mid,
        &LP_side,
        &mut state.mid_side_amp_Q0[0..2],
        frame_length,
        smooth_coef_Q16,
    );
    pred_Q13[1] = silk_stereo_find_predictor(
        &mut HP_ratio_Q14,
        &HP_mid,
        &HP_side,
        &mut state.mid_side_amp_Q0[2..4],
        frame_length,
        smooth_coef_Q16,
    );
    /* Ratio of the norms of residual and mid signals */
    let mut frac_Q16 = silk_SMLABB(HP_ratio_Q14, LP_ratio_Q14, 3);
    frac_Q16 = silk_min(frac_Q16, SILK_FIX_CONST(1.0, 16));

    /* Determine bitrate distribution between mid and side, and possibly reduce stereo width */
    total_rate_bps -= if is10msFrame { 1200 } else { 600 }; /* Subtract approximate bitrate for coding stereo parameters */
    if total_rate_bps < 1 {
        total_rate_bps = 1;
    }
    let min_mid_rate_bps = silk_SMLABB(2000, fs_kHz, 600);
    silk_assert!(min_mid_rate_bps < 32767);
    /* Default bitrate distribution: 8 parts for Mid and (5+3*frac) parts for Side. so: mid_rate = ( 8 / ( 13 + 3 * frac ) ) * total_ rate */
    let frac_3_Q16 = silk_MUL(3, frac_Q16);
    mid_side_rates_bps[0] = silk_DIV32_varQ(
        total_rate_bps,
        SILK_FIX_CONST(8.0 + 5.0, 16) + frac_3_Q16,
        16 + 3,
    );
    /* If Mid bitrate below minimum, reduce stereo width */
    let mut width_Q14: opus_int32;
    if mid_side_rates_bps[0] < min_mid_rate_bps {
        mid_side_rates_bps[0] = min_mid_rate_bps;
        mid_side_rates_bps[1] = total_rate_bps - mid_side_rates_bps[0];
        /* width = 4 * ( 2 * side_rate - min_rate ) / ( ( 1 + 3 * frac ) * min_rate ) */
        width_Q14 = silk_DIV32_varQ(
            silk_LSHIFT(mid_side_rates_bps[1], 1) - min_mid_rate_bps,
            silk_SMULWB(SILK_FIX_CONST(1.0, 16) + frac_3_Q16, min_mid_rate_bps),
            14 + 2,
        );
        width_Q14 = silk_LIMIT(width_Q14, 0, SILK_FIX_CONST(1.0, 14));
    } else {
        mid_side_rates_bps[1] = total_rate_bps - mid_side_rates_bps[0];
        width_Q14 = SILK_FIX_CONST(1.0, 14);
    }

    /* Smoother */
    state.smth_width_Q14 = silk_SMLAWB(
        state.smth_width_Q14 as opus_int32,
        width_Q14 - state.smth_width_Q14 as opus_int32,
        smooth_coef_Q16,
    ) as opus_int16;

    /* At very low bitrates or for inputs that are nearly amplitude panned, switch to panned-mono coding */
    *mid_only_flag = 0;
    if toMono != 0 {
        /* Last frame before stereo->mono transition; collapse stereo width */
        width_Q14 = 0;
        pred_Q13[0] = 0;
        pred_Q13[1] = 0;
        silk_stereo_quant_pred(&mut pred_Q13, ix);
    } else if state.width_prev_Q14 == 0
        && (8 * total_rate_bps < 13 * min_mid_rate_bps
            || silk_SMULWB(frac_Q16, state.smth_width_Q14 as opus_int32)
                < SILK_FIX_CONST(0.05, 14))
    {
        /* Code as panned-mono; previous frame already had zero width */
        /* Scale down and quantize predictors */
        pred_Q13[0] = silk_RSHIFT(
            silk_SMULBB(state.smth_width_Q14 as opus_int32, pred_Q13[0]),
            14,
        );
        pred_Q13[1] = silk_RSHIFT(
            silk_SMULBB(state.smth_width_Q14 as opus_int32, pred_Q13[1]),
            14,
        );
        silk_stereo_quant_pred(&mut pred_Q13, ix);
        /* Collapse stereo width */
        width_Q14 = 0;
        pred_Q13[0] = 0;
        pred_Q13[1] = 0;
        mid_side_rates_bps[0] = total_rate_bps;
        mid_side_rates_bps[1] = 0;
        *mid_only_flag = 1;
    } else if state.width_prev_Q14 != 0
        && (8 * total_rate_bps < 11 * min_mid_rate_bps
            || silk_SMULWB(frac_Q16, state.smth_width_Q14 as opus_int32)
                < SILK_FIX_CONST(0.02, 14))
    {
        /* Transition to zero-width stereo */
        /* Scale down and quantize predictors */
        pred_Q13[0] = silk_RSHIFT(
            silk_SMULBB(state.smth_width_Q14 as opus_int32, pred_Q13[0]),
            14,
        );
        pred_Q13[1] = silk_RSHIFT(
            silk_SMULBB(state.smth_width_Q14 as opus_int32, pred_Q13[1]),
            14,
        );
        silk_stereo_quant_pred(&mut pred_Q13, ix);
        /* Collapse stereo width */
        width_Q14 = 0;
        pred_Q13[0] = 0;
        pred_Q13[1] = 0;
    } else if state.smth_width_Q14 as opus_int32 > SILK_FIX_CONST(0.95, 14) {
        /* Full-width stereo coding */
        silk_stereo_quant_pred(&mut pred_Q13, ix);
        width_Q14 = SILK_FIX_CONST(1.0, 14);
    } else {
        /* Reduced-width stereo coding; scale down and quantize predictors */
        pred_Q13[0] = silk_RSHIFT(
            silk_SMULBB(state.smth_width_Q14 as opus_int32, pred_Q13[0]),
            14,
        );
        pred_Q13[1] = silk_RSHIFT(
            silk_SMULBB(state.smth_width_Q14 as opus_int32, pred_Q13[1]),
            14,
        );
        silk_stereo_quant_pred(&mut pred_Q13, ix);
        width_Q14 = state.smth_width_Q14 as opus_int32;
    }

    /* Make sure to keep on encoding until the tapered output has been transmitted */
    if *mid_only_flag == 1 {
        state.silent_side_len = state
            .silent_side_len
            .wrapping_add((frame_length - STEREO_INTERP_LEN_MS * fs_kHz) as opus_int16);
        if (state.silent_side_len as opus_int) < LA_SHAPE_MS * fs_kHz {
            *mid_only_flag = 0;
        } else {
            /* Limit to avoid wrapping around */
            state.silent_side_len = 10000;
        }
    } else {
        state.silent_side_len = 0;
    }

    if *mid_only_flag == 0 && mid_side_rates_bps[1] < 1 {
        mid_side_rates_bps[1] = 1;
        mid_side_rates_bps[0] = silk_max_int(1, total_rate_bps - mid_side_rates_bps[1]);
    }

    /* Interpolate predictors and subtract prediction from side channel */
    let mut pred0_Q13 = -(state.pred_prev_Q13[0] as opus_int32);
    let mut pred1_Q13 = -(state.pred_prev_Q13[1] as opus_int32);
    let mut w_Q24 = silk_LSHIFT(state.width_prev_Q14 as opus_int32, 10);
    let denom_Q16 = silk_DIV32_16(1i32 << 16, STEREO_INTERP_LEN_MS * fs_kHz);
    let delta0_Q13 = -silk_RSHIFT_ROUND(
        silk_SMULBB(
            pred_Q13[0] - state.pred_prev_Q13[0] as opus_int32,
            denom_Q16,
        ),
        16,
    );
    let delta1_Q13 = -silk_RSHIFT_ROUND(
        silk_SMULBB(
            pred_Q13[1] - state.pred_prev_Q13[1] as opus_int32,
            denom_Q16,
        ),
        16,
    );
    let deltaw_Q24 = silk_LSHIFT(
        silk_SMULWB(width_Q14 - state.width_prev_Q14 as opus_int32, denom_Q16),
        10,
    );
    for n in 0..(STEREO_INTERP_LEN_MS * fs_kHz) as usize {
        pred0_Q13 += delta0_Q13;
        pred1_Q13 += delta1_Q13;
        w_Q24 += deltaw_Q24;
        let mut sum = silk_LSHIFT(
            silk_ADD_LSHIFT32(
                x1[n] as opus_int32 + x1[n + 2] as opus_int32,
                x1[n + 1] as opus_int32,
                1,
            ),
            9,
        ); /* Q11 */
        sum = silk_SMLAWB(silk_SMULWB(w_Q24, side[n + 1] as opus_int32), sum, pred0_Q13); /* Q8  */
        sum = silk_SMLAWB(sum, silk_LSHIFT(x1[n + 1] as opus_int32, 11), pred1_Q13); /* Q8  */
        x2[n + 1] = silk_SAT16(silk_RSHIFT_ROUND(sum, 8)) as opus_int16;
    }

    pred0_Q13 = -pred_Q13[0];
    pred1_Q13 = -pred_Q13[1];
    w_Q24 = silk_LSHIFT(width_Q14, 10);
    for n in (STEREO_INTERP_LEN_MS * fs_kHz) as usize..frame_length as usize {
        let mut sum = silk_LSHIFT(
            silk_ADD_LSHIFT32(
                x1[n] as opus_int32 + x1[n + 2] as opus_int32,
                x1[n + 1] as opus_int32,
                1,
            ),
            9,
        ); /* Q11 */
        sum = silk_SMLAWB(silk_SMULWB(w_Q24, side[n + 1] as opus_int32), sum, pred0_Q13); /* Q8  */
        sum = silk_SMLAWB(sum, silk_LSHIFT(x1[n + 1] as opus_int32, 11), pred1_Q13); /* Q8  */
        x2[n + 1] = silk_SAT16(silk_RSHIFT_ROUND(sum, 8)) as opus_int16;
    }
    state.pred_prev_Q13[0] = pred_Q13[0] as opus_int16;
    state.pred_prev_Q13[1] = pred_Q13[1] as opus_int16;
    state.width_prev_Q14 = width_Q14 as opus_int16;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Running the converter on identical left/right signals must produce a
    /// silent side channel and a mid channel close to the input.
    #[test]
    fn identical_channels_have_no_side() {
        let mut st = stereo_enc_state::default();
        let n = 160usize;
        let mut x1 = vec![0i16; n + 2];
        let mut x2 = vec![0i16; n + 2];
        for i in 0..n {
            let v = ((i * 97 % 4000) as i32 - 2000) as i16;
            x1[i + 2] = v;
            x2[i + 2] = v;
        }
        let mut ix = [[0i8; 3]; 2];
        let mut mid_only = 0i8;
        let mut rates = [0i32; 2];
        silk_stereo_LR_to_MS(
            &mut st,
            &mut x1,
            &mut x2,
            &mut ix,
            &mut mid_only,
            &mut rates,
            64000,
            100,
            0,
            16,
            160,
        );
        /* The side residual is written to `x2[1 ..= frame_length]` (the C's
           `x2[-1 .. frame_length-1]`); the last sample is only read. */
        for i in 1..=n {
            assert_eq!(x2[i], 0, "side must vanish at {i}");
        }
    }
}

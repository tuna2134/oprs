//! Mirror of `silk/stereo_MS_to_LR.c` from libopus 1.6.1, transliterated to safe Rust.
//!
//! Convert an adaptive Mid/Side representation to a Left/Right stereo signal.
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
use crate::silk::structs::*;
use crate::silk::typedef::*;
use crate::types::*;

/// `silk_stereo_MS_to_LR(state, x1, x2, pred_Q13, fs_kHz, frame_length)`
pub fn silk_stereo_MS_to_LR(
    state: &mut stereo_dec_state,  /* I/O  State */
    x1: &mut [opus_int16],         /* I/O  Left input signal, becomes mid signal */
    x2: &mut [opus_int16],         /* I/O  Right input signal, becomes side signal */
    pred_Q13: &[opus_int32],       /* I    Predictors */
    fs_kHz: opus_int,              /* I    Samples rate (kHz) */
    frame_length: opus_int,        /* I    Number of samples */
) {
    let denom_Q16: opus_int32;
    let delta0_Q13: opus_int32;
    let delta1_Q13: opus_int32;
    let mut sum: opus_int32;
    let mut diff: opus_int32;
    let mut pred0_Q13: opus_int32;
    let mut pred1_Q13: opus_int32;

    /* Buffering */
    x1[..2].copy_from_slice(&state.sMid);
    x2[..2].copy_from_slice(&state.sSide);
    state
        .sMid
        .copy_from_slice(&x1[frame_length as usize..frame_length as usize + 2]);
    state
        .sSide
        .copy_from_slice(&x2[frame_length as usize..frame_length as usize + 2]);

    /* Interpolate predictors and add prediction to side channel */
    pred0_Q13 = state.pred_prev_Q13[0] as opus_int32;
    pred1_Q13 = state.pred_prev_Q13[1] as opus_int32;
    denom_Q16 = silk_DIV32_16(1 << 16, STEREO_INTERP_LEN_MS * fs_kHz);
    delta0_Q13 = silk_RSHIFT_ROUND(
        silk_SMULBB(
            pred_Q13[0] - state.pred_prev_Q13[0] as opus_int32,
            denom_Q16,
        ),
        16,
    );
    delta1_Q13 = silk_RSHIFT_ROUND(
        silk_SMULBB(
            pred_Q13[1] - state.pred_prev_Q13[1] as opus_int32,
            denom_Q16,
        ),
        16,
    );
    for n in 0..STEREO_INTERP_LEN_MS * fs_kHz {
        pred0_Q13 += delta0_Q13;
        pred1_Q13 += delta1_Q13;
        sum = silk_LSHIFT(
            silk_ADD_LSHIFT32(
                x1[n as usize] as opus_int32 + x1[(n + 2) as usize] as opus_int32,
                x1[(n + 1) as usize] as opus_int32,
                1,
            ),
            9,
        ); /* Q11 */
        sum = silk_SMLAWB(
            silk_LSHIFT(x2[(n + 1) as usize] as opus_int32, 8),
            sum,
            pred0_Q13,
        ); /* Q8  */
        sum = silk_SMLAWB(sum, silk_LSHIFT(x1[(n + 1) as usize] as opus_int32, 11), pred1_Q13); /* Q8  */
        x2[(n + 1) as usize] = silk_SAT16(silk_RSHIFT_ROUND(sum, 8)) as opus_int16;
    }
    pred0_Q13 = pred_Q13[0];
    pred1_Q13 = pred_Q13[1];
    for n in STEREO_INTERP_LEN_MS * fs_kHz..frame_length {
        sum = silk_LSHIFT(
            silk_ADD_LSHIFT32(
                x1[n as usize] as opus_int32 + x1[(n + 2) as usize] as opus_int32,
                x1[(n + 1) as usize] as opus_int32,
                1,
            ),
            9,
        ); /* Q11 */
        sum = silk_SMLAWB(
            silk_LSHIFT(x2[(n + 1) as usize] as opus_int32, 8),
            sum,
            pred0_Q13,
        ); /* Q8  */
        sum = silk_SMLAWB(sum, silk_LSHIFT(x1[(n + 1) as usize] as opus_int32, 11), pred1_Q13); /* Q8  */
        x2[(n + 1) as usize] = silk_SAT16(silk_RSHIFT_ROUND(sum, 8)) as opus_int16;
    }
    state.pred_prev_Q13[0] = pred_Q13[0] as opus_int16;
    state.pred_prev_Q13[1] = pred_Q13[1] as opus_int16;

    /* Convert to left/right signals */
    for n in 0..frame_length {
        sum = x1[(n + 1) as usize] as opus_int32 + x2[(n + 1) as usize] as opus_int32;
        diff = x1[(n + 1) as usize] as opus_int32 - x2[(n + 1) as usize] as opus_int32;
        x1[(n + 1) as usize] = silk_SAT16(sum) as opus_int16;
        x2[(n + 1) as usize] = silk_SAT16(diff) as opus_int16;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_zero_input_stays_zero() {
        let mut st = stereo_dec_state::default();
        let mut x1 = [0i16; 162];
        let mut x2 = [0i16; 162];
        let pred = [0i32, 0];
        silk_stereo_MS_to_LR(&mut st, &mut x1, &mut x2, &pred, 8, 160);
        assert!(x1.iter().all(|&v| v == 0));
        assert!(x2.iter().all(|&v| v == 0));
    }
}

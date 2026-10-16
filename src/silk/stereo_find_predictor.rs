//! Mirror of `silk/stereo_find_predictor.c` from libopus 1.6.1, transliterated to safe Rust.
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

use crate::silk::inner_prod_aligned::*;
use crate::silk::inlines::*;
use crate::silk::macros::*;
use crate::silk::sigproc_fix::*;
use crate::silk::sum_sqr_shift::*;
use crate::silk::typedef::silk_assert;
use crate::silk::typedef::*;
use crate::types::*;

/* Find least-squares prediction gain for one signal based on another and quantize it */
pub fn silk_stereo_find_predictor( /* O    Returns predictor in Q13                    */
    ratio_Q14: &mut opus_int32, /* O    Ratio of residual and mid energies          */
    x: &[opus_int16],           /* I    Basis signal                                */
    y: &[opus_int16],           /* I    Target signal                               */
    mid_res_amp_Q0: &mut [opus_int32], /* I/O  Smoothed mid, residual norms          */
    length: opus_int,           /* I    Number of samples                           */
    smooth_coef_Q16: opus_int,  /* I    Smoothing coefficient                       */
) -> opus_int32 {
    /* Find  predictor */
    let (nrgx0, scale1) = silk_sum_sqr_shift(x, length);
    let (nrgy0, scale2) = silk_sum_sqr_shift(y, length);
    let mut scale = silk_max_int(scale1, scale2);
    scale += scale & 1; /* make even */
    let mut nrgy = silk_RSHIFT32(nrgy0, scale - scale2);
    let mut nrgx = silk_RSHIFT32(nrgx0, scale - scale1);
    nrgx = silk_max_int(nrgx, 1);
    let corr = silk_inner_prod_aligned_scale(x, y, scale, length);
    let mut pred_Q13 = silk_DIV32_varQ(corr, nrgx, 13);
    pred_Q13 = silk_LIMIT(pred_Q13, -(1 << 14), 1 << 14);
    let pred2_Q10 = silk_SMULWB(pred_Q13, pred_Q13);

    /* Faster update for signals with large prediction parameters */
    let smooth_coef_Q16 = silk_max_int(smooth_coef_Q16, silk_abs(pred2_Q10));

    /* Smoothed mid and residual norms */
    silk_assert!(smooth_coef_Q16 < 32768);
    scale = silk_RSHIFT(scale, 1);
    mid_res_amp_Q0[0] = silk_SMLAWB(
        mid_res_amp_Q0[0],
        silk_LSHIFT(silk_SQRT_APPROX(nrgx), scale) - mid_res_amp_Q0[0],
        smooth_coef_Q16,
    );
    /* Residual energy = nrgy - 2 * pred * corr + pred^2 * nrgx */
    nrgy = silk_SUB_LSHIFT32(nrgy, silk_SMULWB(corr, pred_Q13), 3 + 1);
    nrgy = silk_ADD_LSHIFT32(nrgy, silk_SMULWB(nrgx, pred2_Q10), 6);
    mid_res_amp_Q0[1] = silk_SMLAWB(
        mid_res_amp_Q0[1],
        silk_LSHIFT(silk_SQRT_APPROX(nrgy), scale) - mid_res_amp_Q0[1],
        smooth_coef_Q16,
    );

    /* Ratio of smoothed residual and mid norms */
    *ratio_Q14 = silk_DIV32_varQ(mid_res_amp_Q0[1], silk_max(mid_res_amp_Q0[0], 1), 14);
    *ratio_Q14 = silk_LIMIT(*ratio_Q14, 0, 32767);

    pred_Q13
}

#[cfg(test)]
mod tests {
    use super::*;

    /// For perfectly correlated signals the predictor saturates at 1.0 (Q13)
    /// and the residual/mid ratio goes to zero.
    #[test]
    fn identical_signals_predict_one() {
        let x: Vec<i16> = (0..80).map(|i| ((i * 331) % 2000 - 1000) as i16).collect();
        let y = x.clone();
        let mut ratio = -1i32;
        let mut amps = [0i32; 2];
        let pred = silk_stereo_find_predictor(&mut ratio, &x, &y, &mut amps, 80, 1000);
        assert_eq!(pred, 1 << 13);
        assert_eq!(ratio, 0);
    }

    /// Identical signals with a negative sign give the negative predictor.
    #[test]
    fn anti_correlated_signals_predict_minus_one() {
        let x: Vec<i16> = (0..80).map(|i| ((i * 331) % 2000 - 1000) as i16).collect();
        let y: Vec<i16> = x.iter().map(|&v| -v).collect();
        let mut ratio = -1i32;
        let mut amps = [0i32; 2];
        let pred = silk_stereo_find_predictor(&mut ratio, &x, &y, &mut amps, 80, 1000);
        assert_eq!(pred, -(1 << 13));
    }
}

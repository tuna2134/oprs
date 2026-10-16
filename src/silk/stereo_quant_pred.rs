//! Mirror of `silk/stereo_quant_pred.c` from libopus 1.6.1, transliterated to safe Rust.
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
use crate::silk::tables_other::*;
use crate::silk::typedef::*;
use crate::types::*;

/* Quantize mid/side predictors */
pub fn silk_stereo_quant_pred(
    pred_Q13: &mut [opus_int32], /* I/O  Predictors (out: quantized)                 */
    ix: &mut [[opus_int8; 3]; 2], /* O    Quantization indices                        */
) {
    let mut quant_pred_Q13: opus_int32 = 0;

    /* Quantize */
    for n in 0..2usize {
        /* Brute-force search over quantization levels */
        let mut err_min_Q13 = silk_int32_MAX;
        'levels: for i in 0..STEREO_QUANT_TAB_SIZE - 1 {
            let low_Q13 = silk_stereo_pred_quant_Q13[i as usize] as opus_int32;
            let step_Q13 = silk_SMULWB(
                silk_stereo_pred_quant_Q13[(i + 1) as usize] as opus_int32 - low_Q13,
                SILK_FIX_CONST(0.5 / STEREO_QUANT_SUB_STEPS as f64, 16),
            );
            for j in 0..STEREO_QUANT_SUB_STEPS {
                let lvl_Q13 = silk_SMLABB(low_Q13, step_Q13, 2 * j + 1);
                let err_Q13 = silk_abs(pred_Q13[n] - lvl_Q13);
                if err_Q13 < err_min_Q13 {
                    err_min_Q13 = err_Q13;
                    quant_pred_Q13 = lvl_Q13;
                    ix[n][0] = i as opus_int8;
                    ix[n][1] = j as opus_int8;
                } else {
                    /* Error increasing, so we're past the optimum */
                    break 'levels;
                }
            }
        }
        ix[n][2] = silk_DIV32_16(ix[n][0] as opus_int32, 3) as opus_int8;
        ix[n][0] -= ix[n][2] * 3;
        pred_Q13[n] = quant_pred_Q13;
    }

    /* Subtract second from first predictor (helps when actually applying these) */
    pred_Q13[0] -= pred_Q13[1];
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A predictor that sits exactly on a quantisation level must be
    /// reproduced by the quantiser, and the difference of the two predictors
    /// is returned in `pred_Q13[0]`.
    #[test]
    fn exactly_representable_predictor() {
        for n in 0..(STEREO_QUANT_TAB_SIZE - 1) {
            let low = silk_stereo_pred_quant_Q13[n as usize] as i32;
            let step = silk_SMULWB(
                silk_stereo_pred_quant_Q13[(n + 1) as usize] as i32 - low,
                SILK_FIX_CONST(0.5 / STEREO_QUANT_SUB_STEPS as f64, 16),
            );
            let lvl = silk_SMLABB(low, step, 1);
            let mut pred = [lvl, 0];
            let mut ix = [[0i8; 3]; 2];
            silk_stereo_quant_pred(&mut pred, &mut ix);
            assert_eq!(pred[0], lvl);
            assert_eq!(ix[0][0] as i32, n % 3);
        }
    }
}

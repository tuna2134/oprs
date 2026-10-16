//! Mirror of `silk/stereo_decode_pred.c` from libopus 1.6.1, transliterated to safe Rust.
//!
//! Decode mid/side predictors and the mid-only flag.
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

use crate::celt::entcode::ec_ctx;
use crate::celt::entdec::ec_dec_icdf;
use crate::silk::define::*;
use crate::silk::macros::*;
use crate::silk::sigproc_fix::*;
use crate::silk::tables_other::*;
use crate::silk::typedef::*;
use crate::types::*;

/// `silk_stereo_decode_pred(psRangeDec, pred_Q13)`
pub fn silk_stereo_decode_pred(
    psRangeDec: &mut ec_ctx,   /* I/O  Compressor data structure */
    pred_Q13: &mut [opus_int32], /* O    Predictors */
) {
    let mut n: opus_int;
    let mut ix = [[0 as opus_int; 3]; 2];
    let mut low_Q13: opus_int32;
    let mut step_Q13: opus_int32;

    /* Entropy decoding */
    n = ec_dec_icdf(psRangeDec, &silk_stereo_pred_joint_iCDF, 8);
    ix[0][2] = silk_DIV32_16(n, 5);
    ix[1][2] = n - 5 * ix[0][2];
    for n in 0..2 {
        ix[n][0] = ec_dec_icdf(psRangeDec, &silk_uniform3_iCDF, 8);
        ix[n][1] = ec_dec_icdf(psRangeDec, &silk_uniform5_iCDF, 8);
    }

    /* Dequantize */
    for n in 0..2 {
        ix[n][0] += 3 * ix[n][2];
        low_Q13 = silk_stereo_pred_quant_Q13[ix[n][0] as usize] as opus_int32;
        step_Q13 = silk_SMULWB(
            silk_stereo_pred_quant_Q13[(ix[n][0] + 1) as usize] as opus_int32 - low_Q13,
            SILK_FIX_CONST(0.5 / STEREO_QUANT_SUB_STEPS as f64, 16),
        );
        pred_Q13[n as usize] = silk_SMLABB(low_Q13, step_Q13, 2 * ix[n][1] + 1);
    }

    /* Subtract second from first predictor (helps when actually applying these) */
    pred_Q13[0] -= pred_Q13[1];
}

/// `silk_stereo_decode_mid_only(psRangeDec, decode_only_mid)`
pub fn silk_stereo_decode_mid_only(
    psRangeDec: &mut ec_ctx,      /* I/O  Compressor data structure */
    decode_only_mid: &mut opus_int, /* O    Flag that only mid channel has been coded */
) {
    /* Decode flag that only mid channel is coded */
    *decode_only_mid = ec_dec_icdf(psRangeDec, &silk_stereo_only_code_mid_iCDF, 8);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::celt::entdec::ec_dec_init;

    #[test]
    fn decodes_from_a_zero_buffer() {
        let buf = [0u8; 16];
        let mut dec = ec_dec_init(&buf);
        let mut pred = [0i32; 2];
        silk_stereo_decode_pred(&mut dec, &mut pred);
        let mut mid = 0i32;
        silk_stereo_decode_mid_only(&mut dec, &mut mid);
        assert!(mid == 0 || mid == 1);
    }
}

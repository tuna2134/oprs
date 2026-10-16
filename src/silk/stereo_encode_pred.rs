//! Mirror of `silk/stereo_encode_pred.c` from libopus 1.6.1, transliterated to safe Rust.
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
use crate::silk::tables_other::*;
use crate::silk::typedef::*;
use crate::types::*;

/* Entropy code the mid/side quantization indices */
pub fn silk_stereo_encode_pred(
    psRangeEnc: &mut ec_ctx,  /* I/O  Compressor data structure                   */
    ix: &[[opus_int8; 3]; 2], /* I    Quantization indices                        */
) {
    /* Entropy coding */
    let n = 5 * ix[0][2] as opus_int + ix[1][2] as opus_int;
    celt_assert!(n < 25);
    ec_enc_icdf(psRangeEnc, n, &silk_stereo_pred_joint_iCDF, 8);
    for n in 0..2usize {
        celt_assert!((ix[n][0] as opus_int) < 3);
        celt_assert!((ix[n][1] as opus_int) < STEREO_QUANT_SUB_STEPS);
        ec_enc_icdf(psRangeEnc, ix[n][0] as opus_int, &silk_uniform3_iCDF, 8);
        ec_enc_icdf(psRangeEnc, ix[n][1] as opus_int, &silk_uniform5_iCDF, 8);
    }
}

/* Entropy code the mid-only flag */
pub fn silk_stereo_encode_mid_only(
    psRangeEnc: &mut ec_ctx,  /* I/O  Compressor data structure                   */
    mid_only_flag: opus_int8, /* I    Flag that only mid channel has been coded   */
) {
    /* Encode flag that only mid channel is coded */
    ec_enc_icdf(
        psRangeEnc,
        mid_only_flag as opus_int,
        &silk_stereo_only_code_mid_iCDF,
        8,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::celt::entenc::{ec_enc_done, ec_enc_init};

    /// The joint index packs `5 * ix[0][2] + ix[1][2]`; two inputs with the
    /// same sum but a different packing must produce different bitstreams.
    #[test]
    fn joint_index_packing() {
        let mut a = ec_enc_init(64);
        let ix_a = [[0i8, 0, 1], [0, 0, 2]];
        silk_stereo_encode_pred(&mut a, &ix_a);
        ec_enc_done(&mut a);

        let mut b = ec_enc_init(64);
        let ix_b = [[0i8, 0, 2], [0, 0, 1]];
        silk_stereo_encode_pred(&mut b, &ix_b);
        ec_enc_done(&mut b);

        assert_ne!(a.buf, b.buf);
    }
}

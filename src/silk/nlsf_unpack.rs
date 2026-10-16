//! Mirror of `silk/NLSF_unpack.c` from libopus 1.6.1, transliterated to safe Rust.
//!
//! Unpack predictor values and indices for the entropy coding tables.
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

/// `silk_NLSF_unpack(ec_ix, pred_Q8, psNLSF_CB, CB1_index)`
pub fn silk_NLSF_unpack(
    ec_ix: &mut [opus_int16],
    pred_Q8: &mut [opus_uint8],
    psNLSF_CB: &silk_NLSF_CB_struct,
    CB1_index: opus_int,
) {
    let order = psNLSF_CB.order as opus_int;
    let ec_sel_base = (CB1_index * order / 2) as usize;

    let mut i: opus_int = 0;
    while i < order {
        let entry: opus_uint8 = psNLSF_CB.ec_sel[ec_sel_base + (i / 2) as usize];
        ec_ix[i as usize] = silk_SMULBB(
            silk_RSHIFT(entry as opus_int32, 1) & 7,
            2 * NLSF_QUANT_MAX_AMPLITUDE + 1,
        ) as opus_int16;
        pred_Q8[i as usize] =
            psNLSF_CB.pred_Q8[(i + (entry as opus_int & 1) * (order - 1)) as usize];
        ec_ix[(i + 1) as usize] = silk_SMULBB(
            silk_RSHIFT(entry as opus_int32, 5) & 7,
            2 * NLSF_QUANT_MAX_AMPLITUDE + 1,
        ) as opus_int16;
        pred_Q8[(i + 1) as usize] = psNLSF_CB.pred_Q8
            [(i + (silk_RSHIFT(entry as opus_int32, 4) & 1) * (order - 1) + 1) as usize];
        i += 2;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::silk::tables_nlsf_cb_nb_mb::silk_NLSF_CB_NB_MB;

    #[test]
    fn entropy_indices_are_multiples_of_nine() {
        let cb = &silk_NLSF_CB_NB_MB;
        let order = cb.order as usize;
        let mut ec_ix = [0i16; 16];
        let mut pred_Q8 = [0u8; 16];
        silk_NLSF_unpack(&mut ec_ix, &mut pred_Q8, cb, 0);
        for i in 0..order {
            assert_eq!(ec_ix[i] % 9, 0);
            assert!(ec_ix[i] >= 0 && ec_ix[i] <= 63);
            assert!(pred_Q8[i] < 255);
        }
    }
}

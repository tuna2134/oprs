//! Transliteration of `silk/init_decoder.c` from libopus 1.6.1 to safe Rust.
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

use crate::silk::cng::silk_CNG_Reset;
use crate::silk::decode_frame::{osce_reset, OSCE_DEFAULT_METHOD};
use crate::silk::plc::silk_PLC_Reset;
use crate::silk::structs::*;
use crate::silk::typedef::*;
use crate::types::*;

/// `opus_select_arch()` — `celt/x86/x86cpu.c`, pinned to the host the reference
/// configuration targets (AVX2+FMA capable, so arch == 4).
///
/// The SILK decoder dispatches nothing that changes integer results, so this
/// value is only observable through `silk_decoder_state::arch`.
pub fn opus_select_arch() -> opus_int {
    4
}

/************************/
/* Reset Decoder State  */
/************************/
/// `silk_reset_decoder(psDec)`
pub fn silk_reset_decoder(
    psDec: &mut silk_decoder_state, /* I/O  Decoder state pointer */
) -> opus_int {
    /* Clear the entire state, except anything copied.
       C: `silk_memset(&psDec->SILK_DECODER_STATE_RESET_START, 0, ...)` where
       `SILK_DECODER_STATE_RESET_START` is `prev_gain_Q16`; only the `ENABLE_OSCE`
       member `osce` precedes that field, so it is the sole survivor. */
    let osce = psDec.osce;
    *psDec = silk_decoder_state::default();
    psDec.osce = osce;

    /* Used to deactivate LSF interpolation */
    psDec.first_frame_after_reset = 1;
    psDec.prev_gain_Q16 = 65536;
    psDec.arch = opus_select_arch();

    /* Reset CNG state */
    silk_CNG_Reset(psDec);

    /* Reset PLC state */
    silk_PLC_Reset(psDec);

    /* Reset OSCE state and method */
    osce_reset(&mut psDec.osce, OSCE_DEFAULT_METHOD);

    0
}

/************************/
/* Init Decoder State   */
/************************/
/// `silk_init_decoder(psDec)`
pub fn silk_init_decoder(
    psDec: &mut silk_decoder_state, /* I/O  Decoder state pointer */
) -> opus_int {
    /* Clear the entire encoder state, except anything copied */
    *psDec = silk_decoder_state::default();

    silk_reset_decoder(psDec);

    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reset_preserves_osce_and_sets_defaults() {
        let mut st = silk_decoder_state::default();
        st.osce.method = 7;
        st.osce.features.numbits_smooth = 3.5;
        st.lagPrev = 123;
        st.fs_kHz = 16;
        let ret = silk_reset_decoder(&mut st);
        assert_eq!(ret, 0);
        assert_eq!(st.lagPrev, 0);
        assert_eq!(st.fs_kHz, 0);
        assert_eq!(st.first_frame_after_reset, 1);
        assert_eq!(st.prev_gain_Q16, 65536);
        assert_eq!(st.arch, 4);
        /* `osce` survives the partial memset, then `osce_reset` overwrites the
           method (with `OSCE_DEFAULT_METHOD`) and zeroes the feature state. */
        assert_eq!(st.osce.method, OSCE_DEFAULT_METHOD);
        assert_eq!(st.osce.features.numbits_smooth, 0.0);
        assert_eq!(st.osce.features.reset, 2);
        /* CNG/PLC resets ran. */
        assert_eq!(st.sCNG.rand_seed, 3176576);
        assert_eq!(st.sPLC.subfr_length, 20);
        assert_eq!(st.sPLC.nb_subfr, 2);
    }

    #[test]
    fn init_zeroes_everything_first() {
        let mut st = silk_decoder_state::default();
        st.osce.method = 5;
        st.lagPrev = 999;
        assert_eq!(silk_init_decoder(&mut st), 0);
        assert_eq!(st.lagPrev, 0);
        assert_eq!(st.osce.method, OSCE_DEFAULT_METHOD);
    }
}

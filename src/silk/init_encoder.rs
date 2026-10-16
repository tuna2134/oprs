//! Mirror of `silk/init_encoder.c` from libopus 1.6.1, transliterated to safe Rust.
//!
//! The reference build is floating point, so `silk_encoder_state_Fxx` is
//! `silk_encoder_state_FLP` (aliased in `crate::silk::float::main_flp`).
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

use crate::silk::float::main_flp::silk_encoder_state_Fxx;
use crate::silk::lin2log::*;
use crate::silk::sigproc_fix::*;
use crate::silk::typedef::*;
use crate::silk::vad::*;

/// `VARIABLE_HP_MIN_CUTOFF_HZ` (`tuning_parameters.h:67`)
const VARIABLE_HP_MIN_CUTOFF_HZ: opus_int = 60;

/*********************************/
/* Initialize Silk Encoder state */
/*********************************/
pub fn silk_init_encoder(psEnc: &mut silk_encoder_state_Fxx, arch: opus_int) -> opus_int {
    let mut ret = 0;

    /* Clear the entire encoder state */
    *psEnc = silk_encoder_state_Fxx::default();

    psEnc.sCmn.arch = arch;

    psEnc.sCmn.variable_HP_smth1_Q15 = silk_LSHIFT(
        silk_lin2log(SILK_FIX_CONST(VARIABLE_HP_MIN_CUTOFF_HZ as f64, 16)) - (16 << 7),
        8,
    );
    psEnc.sCmn.variable_HP_smth2_Q15 = psEnc.sCmn.variable_HP_smth1_Q15;

    /* Used to deactivate LSF interpolation, pitch prediction */
    psEnc.sCmn.first_frame_after_reset = 1;

    /* Initialize Silk VAD */
    ret += silk_VAD_Init(&mut psEnc.sCmn.sVAD);

    ret
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_sets_expected_defaults() {
        let mut e = silk_encoder_state_Fxx::default();
        assert_eq!(silk_init_encoder(&mut e, 4), 0);
        assert_eq!(e.sCmn.arch, 4);
        assert_eq!(e.sCmn.first_frame_after_reset, 1);
        let expect = silk_LSHIFT(
            silk_lin2log(SILK_FIX_CONST(VARIABLE_HP_MIN_CUTOFF_HZ as f64, 16)) - (16 << 7),
            8,
        );
        assert_eq!(e.sCmn.variable_HP_smth1_Q15, expect);
        assert_eq!(e.sCmn.variable_HP_smth2_Q15, expect);
        assert_eq!(e.sCmn.sVAD.counter, 15);
    }
}

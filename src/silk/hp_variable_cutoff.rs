//! Mirror of `silk/HP_variable_cutoff.c` from libopus 1.6.1, transliterated to safe Rust.
//!
//! High-pass filter with cutoff frequency adaptation based on pitch lag
//! statistics.  The `VARIABLE_HP_*` constants come from
//! `silk/tuning_parameters.h`, which has no Rust module of its own.
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
use crate::silk::float::main_flp::*;
use crate::silk::lin2log::*;
use crate::silk::macros::*;
use crate::silk::sigproc_fix::*;
use crate::silk::typedef::*;
use crate::types::*;

/// `VARIABLE_HP_SMTH_COEF1` (`0.1f`)
const VARIABLE_HP_SMTH_COEF1: f32 = 0.1;
/// `VARIABLE_HP_MAX_DELTA_FREQ` (`0.4f`)
const VARIABLE_HP_MAX_DELTA_FREQ: f32 = 0.4;
/// `VARIABLE_HP_MIN_CUTOFF_HZ`
const VARIABLE_HP_MIN_CUTOFF_HZ: opus_int = 60;
/// `VARIABLE_HP_MAX_CUTOFF_HZ`
const VARIABLE_HP_MAX_CUTOFF_HZ: opus_int = 100;

/// `silk_HP_variable_cutoff(state_Fxx)`
pub fn silk_HP_variable_cutoff(
    state_Fxx: &mut [silk_encoder_state_Fxx], /* I/O  Encoder states */
) {
    let quality_Q15: opus_int;
    let mut pitch_freq_Hz_Q16: opus_int32;
    let mut pitch_freq_log_Q7: opus_int32;
    let mut delta_freq_Q7: opus_int32;
    let psEncC1 = &mut state_Fxx[0].sCmn;

    /* Adaptive cutoff frequency: estimate low end of pitch frequency range */
    if psEncC1.prevSignalType as opus_int == TYPE_VOICED {
        /* difference, in log domain */
        pitch_freq_Hz_Q16 = silk_DIV32_16(
            silk_LSHIFT(silk_MUL(psEncC1.fs_kHz, 1000), 16),
            psEncC1.prevLag,
        );
        pitch_freq_log_Q7 = silk_lin2log(pitch_freq_Hz_Q16) - (16 << 7);

        /* adjustment based on quality */
        quality_Q15 = psEncC1.input_quality_bands_Q15[0];
        pitch_freq_log_Q7 = silk_SMLAWB(
            pitch_freq_log_Q7,
            silk_SMULWB(silk_LSHIFT(quality_Q15.wrapping_neg(), 2), quality_Q15),
            pitch_freq_log_Q7
                - (silk_lin2log(SILK_FIX_CONST(VARIABLE_HP_MIN_CUTOFF_HZ as f64, 16)) - (16 << 7)),
        );

        /* delta_freq = pitch_freq_log - psEnc->variable_HP_smth1; */
        delta_freq_Q7 = pitch_freq_log_Q7 - silk_RSHIFT(psEncC1.variable_HP_smth1_Q15, 8);
        if delta_freq_Q7 < 0 {
            /* less smoothing for decreasing pitch frequency, to track something close to the minimum */
            delta_freq_Q7 = silk_MUL(delta_freq_Q7, 3);
        }

        /* limit delta, to reduce impact of outliers in pitch estimation */
        delta_freq_Q7 = silk_LIMIT(
            delta_freq_Q7,
            -SILK_FIX_CONST(VARIABLE_HP_MAX_DELTA_FREQ as f64, 7),
            SILK_FIX_CONST(VARIABLE_HP_MAX_DELTA_FREQ as f64, 7),
        );

        /* update smoother */
        psEncC1.variable_HP_smth1_Q15 = silk_SMLAWB(
            psEncC1.variable_HP_smth1_Q15,
            silk_SMULBB(psEncC1.speech_activity_Q8, delta_freq_Q7),
            SILK_FIX_CONST(VARIABLE_HP_SMTH_COEF1 as f64, 16),
        );

        /* limit frequency range */
        psEncC1.variable_HP_smth1_Q15 = silk_LIMIT(
            psEncC1.variable_HP_smth1_Q15,
            silk_LSHIFT(silk_lin2log(VARIABLE_HP_MIN_CUTOFF_HZ), 8),
            silk_LSHIFT(silk_lin2log(VARIABLE_HP_MAX_CUTOFF_HZ), 8),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unvoiced_leaves_the_state_untouched() {
        let mut st = [silk_encoder_state_Fxx::default(); 2];
        st[0].sCmn.prevSignalType = 0;
        st[0].sCmn.variable_HP_smth1_Q15 = 12345;
        silk_HP_variable_cutoff(&mut st);
        assert_eq!(st[0].sCmn.variable_HP_smth1_Q15, 12345);
    }

    #[test]
    fn voiced_updates_and_limits_the_smoother() {
        let mut st = [silk_encoder_state_Fxx::default(); 2];
        st[0].sCmn.prevSignalType = TYPE_VOICED as i8;
        st[0].sCmn.fs_kHz = 16;
        st[0].sCmn.prevLag = 100;
        st[0].sCmn.speech_activity_Q8 = 100;
        silk_HP_variable_cutoff(&mut st);
        let lo = silk_LSHIFT(silk_lin2log(VARIABLE_HP_MIN_CUTOFF_HZ), 8);
        let hi = silk_LSHIFT(silk_lin2log(VARIABLE_HP_MAX_CUTOFF_HZ), 8);
        assert!(st[0].sCmn.variable_HP_smth1_Q15 >= lo);
        assert!(st[0].sCmn.variable_HP_smth1_Q15 <= hi);
    }
}

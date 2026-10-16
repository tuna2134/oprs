//! Mirror of `silk/decode_pitch.c` from libopus 1.6.1, transliterated to safe Rust.
//!
//! Pitch-lag contour decoding.  The `PE_*` constants come from
//! `silk/pitch_est_defines.h`, which has no Rust module of its own.
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
use crate::silk::macros::*;
use crate::silk::pitch_est_tables::*;
use crate::silk::sigproc_fix::*;
use crate::silk::typedef::*;
use crate::types::*;

// `silk/pitch_est_defines.h`:
/// `PE_MAX_NB_SUBFR`
pub const PE_MAX_NB_SUBFR: opus_int = 4;
/// `PE_MIN_LAG_MS`
pub const PE_MIN_LAG_MS: opus_int = 2;
/// `PE_MAX_LAG_MS`
pub const PE_MAX_LAG_MS: opus_int = 18;
/// `PE_NB_CBKS_STAGE2_EXT`
pub const PE_NB_CBKS_STAGE2_EXT: opus_int = 11;
/// `PE_NB_CBKS_STAGE2_10MS`
pub const PE_NB_CBKS_STAGE2_10MS: opus_int = 3;
/// `PE_NB_CBKS_STAGE3_MAX`
pub const PE_NB_CBKS_STAGE3_MAX: opus_int = 34;
/// `PE_NB_CBKS_STAGE3_10MS`
pub const PE_NB_CBKS_STAGE3_10MS: opus_int = 12;

/// `silk_decode_pitch(lagIndex, contourIndex, pitch_lags, Fs_kHz, nb_subfr)`
pub fn silk_decode_pitch(
    lagIndex: opus_int16,
    contourIndex: opus_int8,
    pitch_lags: &mut [opus_int],
    Fs_kHz: opus_int,
    nb_subfr: opus_int,
) {
    let min_lag: opus_int;
    let max_lag: opus_int;
    let lag: opus_int;

    /* The C picks a flattened `Lag_CB_ptr` plus its row width `cbk_size` (one
       of the `PE_NB_CBKS_*` values); the port indexes the same row-major
       tables directly, so only the selection itself remains. */
    if Fs_kHz == 8 {
        if nb_subfr != PE_MAX_NB_SUBFR {
            celt_assert!(nb_subfr == PE_MAX_NB_SUBFR >> 1);
        }
    } else if nb_subfr != PE_MAX_NB_SUBFR {
        celt_assert!(nb_subfr == PE_MAX_NB_SUBFR >> 1);
    }

    min_lag = silk_SMULBB(PE_MIN_LAG_MS, Fs_kHz);
    max_lag = silk_SMULBB(PE_MAX_LAG_MS, Fs_kHz);
    lag = min_lag + lagIndex as opus_int;

    for k in 0..nb_subfr {
        let cb: opus_int = if Fs_kHz == 8 {
            if nb_subfr == PE_MAX_NB_SUBFR {
                silk_CB_lags_stage2[k as usize][contourIndex as usize] as opus_int
            } else {
                silk_CB_lags_stage2_10_ms[k as usize][contourIndex as usize] as opus_int
            }
        } else if nb_subfr == PE_MAX_NB_SUBFR {
            silk_CB_lags_stage3[k as usize][contourIndex as usize] as opus_int
        } else {
            silk_CB_lags_stage3_10_ms[k as usize][contourIndex as usize] as opus_int
        };
        pitch_lags[k as usize] = lag + cb;
        pitch_lags[k as usize] = silk_LIMIT(pitch_lags[k as usize], min_lag, max_lag);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_contour_gives_min_lag() {
        let mut lags = [0i32; 4];
        silk_decode_pitch(0, 0, &mut lags, 16, 4);
        assert_eq!(lags, [32, 32, 32, 32]);
    }

    #[test]
    fn lag_is_clamped_to_max() {
        let mut lags = [0i32; 4];
        silk_decode_pitch((18 * 16 - 2 * 16 + 5) as i16, 0, &mut lags, 16, 4);
        assert_eq!(lags, [18 * 16; 4]);
    }

    #[test]
    fn ten_ms_subframes() {
        let mut lags = [0i32; 2];
        silk_decode_pitch(0, 0, &mut lags, 8, 2);
        assert_eq!(lags, [2 * 8, 2 * 8]);
    }
}

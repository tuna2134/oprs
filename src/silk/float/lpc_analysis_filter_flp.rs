//! Mirror of `silk/float/LPC_analysis_filter_FLP.c` from libopus 1.6.1, transliterated to safe Rust.
//!
//! Fixed-order LPC analysis filters (6/8/10/12/16) sharing one entry point.
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
use crate::silk::float::sigproc_flp::silk_float;
use crate::silk::typedef::*;

/************************************************/
/* LPC analysis filter                          */
/* NB! State is kept internally and the         */
/* filter always starts with zero state         */
/* first Order output samples are set to zero   */
/************************************************/

/// 16th order LPC analysis filter, does not write first 16 samples
fn silk_LPC_analysis_filter16_FLP(
    r_LPC: &mut [silk_float],
    PredCoef: &[silk_float],
    s: &[silk_float],
    length: opus_int,
) {
    let mut ix: opus_int;

    ix = 16;
    while ix < length {
        let b = ix as usize;
        /* s_ptr = &s[ix - 1] */
        let LPC_pred = s[b - 1] * PredCoef[0]
            + s[b - 2] * PredCoef[1]
            + s[b - 3] * PredCoef[2]
            + s[b - 4] * PredCoef[3]
            + s[b - 5] * PredCoef[4]
            + s[b - 6] * PredCoef[5]
            + s[b - 7] * PredCoef[6]
            + s[b - 8] * PredCoef[7]
            + s[b - 9] * PredCoef[8]
            + s[b - 10] * PredCoef[9]
            + s[b - 11] * PredCoef[10]
            + s[b - 12] * PredCoef[11]
            + s[b - 13] * PredCoef[12]
            + s[b - 14] * PredCoef[13]
            + s[b - 15] * PredCoef[14]
            + s[b - 16] * PredCoef[15];

        /* prediction error (s_ptr[1] == s[ix]) */
        r_LPC[b] = s[b] - LPC_pred;
        ix += 1;
    }
}

/// 12th order LPC analysis filter, does not write first 12 samples
fn silk_LPC_analysis_filter12_FLP(
    r_LPC: &mut [silk_float],
    PredCoef: &[silk_float],
    s: &[silk_float],
    length: opus_int,
) {
    let mut ix: opus_int;

    ix = 12;
    while ix < length {
        let b = ix as usize;
        let LPC_pred = s[b - 1] * PredCoef[0]
            + s[b - 2] * PredCoef[1]
            + s[b - 3] * PredCoef[2]
            + s[b - 4] * PredCoef[3]
            + s[b - 5] * PredCoef[4]
            + s[b - 6] * PredCoef[5]
            + s[b - 7] * PredCoef[6]
            + s[b - 8] * PredCoef[7]
            + s[b - 9] * PredCoef[8]
            + s[b - 10] * PredCoef[9]
            + s[b - 11] * PredCoef[10]
            + s[b - 12] * PredCoef[11];

        r_LPC[b] = s[b] - LPC_pred;
        ix += 1;
    }
}

/// 10th order LPC analysis filter, does not write first 10 samples
fn silk_LPC_analysis_filter10_FLP(
    r_LPC: &mut [silk_float],
    PredCoef: &[silk_float],
    s: &[silk_float],
    length: opus_int,
) {
    let mut ix: opus_int;

    ix = 10;
    while ix < length {
        let b = ix as usize;
        let LPC_pred = s[b - 1] * PredCoef[0]
            + s[b - 2] * PredCoef[1]
            + s[b - 3] * PredCoef[2]
            + s[b - 4] * PredCoef[3]
            + s[b - 5] * PredCoef[4]
            + s[b - 6] * PredCoef[5]
            + s[b - 7] * PredCoef[6]
            + s[b - 8] * PredCoef[7]
            + s[b - 9] * PredCoef[8]
            + s[b - 10] * PredCoef[9];

        r_LPC[b] = s[b] - LPC_pred;
        ix += 1;
    }
}

/// 8th order LPC analysis filter, does not write first 8 samples
fn silk_LPC_analysis_filter8_FLP(
    r_LPC: &mut [silk_float],
    PredCoef: &[silk_float],
    s: &[silk_float],
    length: opus_int,
) {
    let mut ix: opus_int;

    ix = 8;
    while ix < length {
        let b = ix as usize;
        let LPC_pred = s[b - 1] * PredCoef[0]
            + s[b - 2] * PredCoef[1]
            + s[b - 3] * PredCoef[2]
            + s[b - 4] * PredCoef[3]
            + s[b - 5] * PredCoef[4]
            + s[b - 6] * PredCoef[5]
            + s[b - 7] * PredCoef[6]
            + s[b - 8] * PredCoef[7];

        r_LPC[b] = s[b] - LPC_pred;
        ix += 1;
    }
}

/// 6th order LPC analysis filter, does not write first 6 samples
fn silk_LPC_analysis_filter6_FLP(
    r_LPC: &mut [silk_float],
    PredCoef: &[silk_float],
    s: &[silk_float],
    length: opus_int,
) {
    let mut ix: opus_int;

    ix = 6;
    while ix < length {
        let b = ix as usize;
        let LPC_pred = s[b - 1] * PredCoef[0]
            + s[b - 2] * PredCoef[1]
            + s[b - 3] * PredCoef[2]
            + s[b - 4] * PredCoef[3]
            + s[b - 5] * PredCoef[4]
            + s[b - 6] * PredCoef[5];

        r_LPC[b] = s[b] - LPC_pred;
        ix += 1;
    }
}

/************************************************/
/* LPC analysis filter                          */
/* NB! State is kept internally and the         */
/* filter always starts with zero state         */
/* first Order output samples are set to zero   */
/************************************************/
pub fn silk_LPC_analysis_filter_FLP(
    r_LPC: &mut [silk_float],
    PredCoef: &[silk_float],
    s: &[silk_float],
    length: opus_int,
    Order: opus_int,
) {
    celt_assert!(Order <= length);

    match Order {
        6 => silk_LPC_analysis_filter6_FLP(r_LPC, PredCoef, s, length),
        8 => silk_LPC_analysis_filter8_FLP(r_LPC, PredCoef, s, length),
        10 => silk_LPC_analysis_filter10_FLP(r_LPC, PredCoef, s, length),
        12 => silk_LPC_analysis_filter12_FLP(r_LPC, PredCoef, s, length),
        16 => silk_LPC_analysis_filter16_FLP(r_LPC, PredCoef, s, length),
        _ => {
            celt_assert!(false);
        }
    }

    /* Set first Order output samples to zero (silk_memset) */
    for i in 0..Order {
        r_LPC[i as usize] = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leading_samples_are_zeroed() {
        let s = [1.0f32; 32];
        let coef = [0.0f32; 16];
        let mut r = [9.0f32; 32];
        silk_LPC_analysis_filter_FLP(&mut r, &coef, &s, 32, 16);
        for i in 0..16 {
            assert_eq!(r[i], 0.0);
        }
        for i in 16..32 {
            assert_eq!(r[i], 1.0);
        }
    }

    #[test]
    fn first_order_prediction_is_previous_sample() {
        let mut s = [0.0f32; 20];
        for i in 0..20 {
            s[i] = i as f32;
        }
        /* A = [1, 0, ...] predicts s[ix] from s[ix-1] */
        let mut coef = [0.0f32; 16];
        coef[0] = 1.0;
        let mut r = [0.0f32; 20];
        silk_LPC_analysis_filter_FLP(&mut r, &coef, &s, 20, 16);
        for i in 16..20 {
            assert_eq!(r[i], 1.0, "i={i}");
        }
    }
}

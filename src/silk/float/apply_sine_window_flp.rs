//! Mirror of `silk/float/apply_sine_window_FLP.c` from libopus 1.6.1, transliterated to safe Rust.
//!
//! Recursive sine/cosine window generation, 4 samples at a time.
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
use crate::silk::float::sigproc_flp::{silk_float, PI};
use crate::silk::typedef::*;

/// Apply sine window to signal vector.
///
/// Window types:
///  1 -> sine window from 0 to pi/2
///  2 -> sine window from pi/2 to pi
pub fn silk_apply_sine_window_FLP(
    px_win: &mut [silk_float],
    px: &[silk_float],
    win_type: opus_int,
    length: opus_int,
) {
    let mut k: opus_int;
    let freq: silk_float;
    let c: silk_float;
    let mut S0: silk_float;
    let mut S1: silk_float;

    celt_assert!(win_type == 1 || win_type == 2);

    /* Length must be multiple of 4 */
    celt_assert!((length & 3) == 0);

    freq = PI / (length + 1) as silk_float;

    /* Approximation of 2 * cos(f) */
    c = 2.0f32 - freq * freq;

    /* Initialize state */
    if win_type < 2 {
        /* Start from 0 */
        S0 = 0.0f32;
        /* Approximation of sin(f) */
        S1 = freq;
    } else {
        /* Start from 1 */
        S0 = 1.0f32;
        /* Approximation of cos(f) */
        S1 = 0.5f32 * c;
    }

    /* Uses the recursive equation:   sin(n*f) = 2 * cos(f) * sin((n-1)*f) - sin((n-2)*f)   */
    /* 4 samples at a time */
    k = 0;
    while k < length {
        let b = k as usize;
        px_win[b] = px[b] * 0.5f32 * (S0 + S1);
        px_win[b + 1] = px[b + 1] * S1;
        S0 = c * S1 - S0;
        px_win[b + 2] = px[b + 2] * 0.5f32 * (S1 + S0);
        px_win[b + 3] = px[b + 3] * S0;
        S1 = c * S0 - S1;
        k += 4;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_type_1_starts_at_zero_and_is_monotone() {
        let px = [1.0f32; 32];
        let mut w = [0.0f32; 32];
        silk_apply_sine_window_FLP(&mut w, &px, 1, 32);
        /* The recursive sine approximation can overshoot 1.0 by a few ULP. */
        for k in 0..32 {
            assert!((-1e-6..=1.001).contains(&w[k]), "k={k} v={}", w[k]);
        }
        for k in 1..32 {
            assert!(w[k] >= w[k - 1], "k={k}");
        }
    }

    #[test]
    fn window_type_2_ends_near_zero() {
        let px = [1.0f32; 32];
        let mut w = [0.0f32; 32];
        silk_apply_sine_window_FLP(&mut w, &px, 2, 32);
        assert!(w[0] > 0.99);
        assert!(w[31].abs() < 0.2);
    }
}

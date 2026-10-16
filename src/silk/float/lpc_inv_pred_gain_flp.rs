//! Mirror of `silk/float/lpc_inv_pred_gain_FLP.c` from libopus 1.6.1, transliterated to safe Rust.
//!
//! Inverse LPC prediction gain / stability test.
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

use crate::silk::define::MAX_PREDICTION_POWER_GAIN;
use crate::silk::float::sigproc_flp::silk_float;
use crate::silk::sigproc_fix::SILK_MAX_ORDER_LPC;
use crate::silk::typedef::*;
use crate::types::*;

/// compute inverse of LPC prediction gain, and
/// test if LPC coefficients are stable (all poles within unit circle)
/// this code is based on `silk_a2k_FLP()`
pub fn silk_LPC_inverse_pred_gain_FLP(A: &[silk_float], order: opus_int32) -> silk_float {
    let mut k: opus_int;
    let mut n: opus_int;
    let mut invGain: f64;
    let mut rc: f64;
    let mut rc_mult1: f64;
    let mut rc_mult2: f64;
    let mut tmp1: f64;
    let mut tmp2: f64;
    let mut Atmp = [0.0f32; SILK_MAX_ORDER_LPC as usize];

    Atmp[..order as usize].copy_from_slice(&A[..order as usize]);

    invGain = 1.0;
    k = order - 1;
    while k > 0 {
        rc = -(Atmp[k as usize] as f64);
        rc_mult1 = 1.0f64 - rc * rc;
        invGain *= rc_mult1;
        if (invGain * MAX_PREDICTION_POWER_GAIN as f64) < 1.0f64 {
            return 0.0f32;
        }
        rc_mult2 = 1.0f64 / rc_mult1;
        n = 0;
        while n < (k + 1) >> 1 {
            tmp1 = Atmp[n as usize] as f64;
            tmp2 = Atmp[(k - n - 1) as usize] as f64;
            Atmp[n as usize] = ((tmp1 - tmp2 * rc) * rc_mult2) as silk_float;
            Atmp[(k - n - 1) as usize] = ((tmp2 - tmp1 * rc) * rc_mult2) as silk_float;
            n += 1;
        }
        k -= 1;
    }
    rc = -(Atmp[0] as f64);
    rc_mult1 = 1.0f64 - rc * rc;
    invGain *= rc_mult1;
    if (invGain * MAX_PREDICTION_POWER_GAIN as f64) < 1.0f64 {
        return 0.0f32;
    }
    invGain as silk_float
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_zero_filter_has_unity_gain() {
        let a = [0.0f32; 16];
        assert_eq!(silk_LPC_inverse_pred_gain_FLP(&a, 16), 1.0);
    }

    #[test]
    fn unstable_filter_returns_zero() {
        let mut a = [0.0f32; 16];
        a[15] = 2.0;
        assert_eq!(silk_LPC_inverse_pred_gain_FLP(&a, 16), 0.0);
    }
}

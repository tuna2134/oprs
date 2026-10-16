//! Mirror of `silk/float/LTP_analysis_filter_FLP.c` from libopus 1.6.1, transliterated to safe Rust.
//!
//! LTP analysis FIR filter (one subframe at a time).
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

use crate::silk::define::LTP_ORDER;
use crate::silk::float::sigproc_flp::silk_float;
use crate::silk::typedef::*;

/// LTP analysis FIR filter.
///
/// `x` is the underlying sample buffer and `x_off` is the index the C `x`
/// pointer refers to (the only caller passes `x - predictLPCOrder`, so the
/// filter reads `pitchL[k] + LTP_ORDER / 2` samples *before* that index).
pub fn silk_LTP_analysis_filter_FLP(
    LTP_res: &mut [silk_float],
    x: &[silk_float],
    x_off: opus_int,
    B: &[silk_float],
    pitchL: &[opus_int],
    invGains: &[silk_float],
    subfr_length: opus_int,
    nb_subfr: opus_int,
    pre_length: opus_int,
) {
    let mut Btmp = [0.0f32; LTP_ORDER as usize];
    let mut inv_gain: silk_float;
    let mut k: opus_int;
    let mut i: opus_int;
    let mut j: opus_int;

    let mut x_ptr: isize = x_off as isize;
    let mut LTP_res_ptr: isize = 0;
    k = 0;
    while k < nb_subfr {
        let mut x_lag_ptr: isize = x_ptr - pitchL[k as usize] as isize;
        inv_gain = invGains[k as usize];
        i = 0;
        while i < LTP_ORDER {
            Btmp[i as usize] = B[(k * LTP_ORDER + i) as usize];
            i += 1;
        }

        /* LTP analysis FIR filter */
        i = 0;
        while i < subfr_length + pre_length {
            LTP_res[(LTP_res_ptr + i as isize) as usize] = x[(x_ptr + i as isize) as usize];
            /* Subtract long-term prediction */
            j = 0;
            while j < LTP_ORDER {
                LTP_res[(LTP_res_ptr + i as isize) as usize] -= Btmp[j as usize]
                    * x[(x_lag_ptr + (LTP_ORDER / 2 - j) as isize) as usize];
                j += 1;
            }
            LTP_res[(LTP_res_ptr + i as isize) as usize] *= inv_gain;
            x_lag_ptr += 1;
            i += 1;
        }

        /* Update pointers */
        LTP_res_ptr += (subfr_length + pre_length) as isize;
        x_ptr += subfr_length as isize;
        k += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_coefficients_copy_scaled_input() {
        let n = 40;
        let mut x = vec![0.0f32; 128];
        for i in 0..128 {
            x[i] = (i as f32) - 64.0;
        }
        let b = [0.0f32; (LTP_ORDER * crate::silk::define::MAX_NB_SUBFR) as usize];
        let pitch = [10i32, 10, 10, 10];
        let gains = [0.5f32; crate::silk::define::MAX_NB_SUBFR as usize];
        let mut res = [0.0f32; 40];
        silk_LTP_analysis_filter_FLP(&mut res, &x, 64, &b, &pitch, &gains, 10, 4, 0);
        for i in 0..n {
            assert_eq!(res[i], x[64 + i] * 0.5);
        }
    }
}

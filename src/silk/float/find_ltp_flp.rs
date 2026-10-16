//! Mirror of `silk/float/find_LTP_FLP.c` from libopus 1.6.1, transliterated to safe Rust.
//!
//! LTP analysis: weighted correlation matrix and vector per subframe.
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
use crate::silk::float::corrmatrix_flp::{silk_corrMatrix_FLP, silk_corrVector_FLP};
use crate::silk::float::energy_flp::silk_energy_FLP;
use crate::silk::float::scale_vector_flp::silk_scale_vector_FLP;
use crate::silk::float::sigproc_flp::silk_float;
use crate::silk::sigproc_fix::silk_max;
use crate::silk::typedef::*;

/// `LTP_CORR_INV_MAX` (`silk/tuning_parameters.h`), `0.03f`.
const LTP_CORR_INV_MAX: silk_float = 0.03;

/// `r` is the underlying residual buffer; `r_off` is the index the C `r_ptr`
/// parameter refers to (`lag_ptr = r_ptr - (lag[k] + LTP_ORDER/2)` reads before
/// it).
pub fn silk_find_LTP_FLP(
    XX: &mut [silk_float],
    xX: &mut [silk_float],
    r: &[silk_float],
    r_off: opus_int,
    lag: &[opus_int],
    subfr_length: opus_int,
    nb_subfr: opus_int,
    arch: opus_int,
) {
    let mut k: opus_int;
    let mut xx: silk_float;
    let mut temp: silk_float;

    k = 0;
    while k < nb_subfr {
        let r_ptr = (r_off + k * subfr_length) as usize;
        let lag_ptr = (r_off + k * subfr_length - (lag[k as usize] + LTP_ORDER / 2)) as usize;
        let xx_ptr = (k * LTP_ORDER * LTP_ORDER) as usize;
        let xx_size = (LTP_ORDER * LTP_ORDER) as usize;
        let x_x_ptr = (k * LTP_ORDER) as usize;

        silk_corrMatrix_FLP(
            &r[lag_ptr..],
            subfr_length,
            LTP_ORDER,
            &mut XX[xx_ptr..xx_ptr + xx_size],
            arch,
        );
        silk_corrVector_FLP(
            &r[lag_ptr..],
            &r[r_ptr..],
            subfr_length,
            LTP_ORDER,
            &mut xX[x_x_ptr..x_x_ptr + LTP_ORDER as usize],
            arch,
        );
        xx = silk_energy_FLP(&r[r_ptr..], subfr_length + LTP_ORDER) as silk_float;
        temp = 1.0f32
            / silk_max(
                xx,
                LTP_CORR_INV_MAX * 0.5f32 * (XX[xx_ptr] + XX[xx_ptr + xx_size - 1]) + 1.0f32,
            );
        silk_scale_vector_FLP(
            &mut XX[xx_ptr..xx_ptr + xx_size],
            temp,
            LTP_ORDER * LTP_ORDER,
        );
        silk_scale_vector_FLP(
            &mut xX[x_x_ptr..x_x_ptr + LTP_ORDER as usize],
            temp,
            LTP_ORDER,
        );

        k += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scales_xx_and_xx_consistently() {
        let n = 64;
        let r: Vec<f32> = (0..n + 32).map(|i| ((i % 7) as f32) - 3.0).collect();
        let lag = [8i32, 8, 8, 8];
        let mut xx = [0.0f32; (crate::silk::define::MAX_NB_SUBFR * LTP_ORDER * LTP_ORDER) as usize];
        let mut x_x = [0.0f32; (crate::silk::define::MAX_NB_SUBFR * LTP_ORDER) as usize];
        silk_find_LTP_FLP(&mut xx, &mut x_x, &r, 16, &lag, 16, 2, 0);
        /* second subframe block must be scaled too */
        let xx2: f32 = xx[(25)..50].iter().map(|v| v * v).sum();
        assert!(xx2 > 0.0);
    }
}

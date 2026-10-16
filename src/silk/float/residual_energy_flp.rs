//! Mirror of `silk/float/residual_energy_FLP.c` from libopus 1.6.1, transliterated to safe Rust.
//!
//! Residual energies of input subframes, plus the covariance-based variant.
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
use crate::silk::define::{MAX_FRAME_LENGTH, MAX_LPC_ORDER, MAX_NB_SUBFR};
use crate::silk::float::energy_flp::silk_energy_FLP;
use crate::silk::float::lpc_analysis_filter_flp::silk_LPC_analysis_filter_FLP;
use crate::silk::float::sigproc_flp::silk_float;
use crate::silk::macros::*;
use crate::silk::typedef::*;

const MAX_ITERATIONS_RESIDUAL_NRG: opus_int = 10;
const REGULARIZATION_FACTOR: silk_float = 1e-8;

/// Residual energy: nrg = wxx - 2 * wXx * c + c' * wXX * c
pub fn silk_residual_energy_covar_FLP(
    c: &[silk_float],
    wXX: &mut [silk_float],
    wXx: &[silk_float],
    wxx: silk_float,
    D: opus_int,
) -> silk_float {
    let mut i: opus_int;
    let mut j: opus_int;
    let mut k: opus_int;
    let mut tmp: silk_float;
    let mut nrg: silk_float = 0.0;
    let mut regularization: silk_float;

    /* Safety checks */
    celt_assert!(D >= 0);

    regularization = REGULARIZATION_FACTOR * (wXX[0] + wXX[(D * D - 1) as usize]);
    k = 0;
    while k < MAX_ITERATIONS_RESIDUAL_NRG {
        nrg = wxx;

        tmp = 0.0;
        i = 0;
        while i < D {
            tmp += wXx[i as usize] * c[i as usize];
            i += 1;
        }
        nrg -= 2.0f32 * tmp;

        /* compute c' * wXX * c, assuming wXX is symmetric */
        i = 0;
        while i < D {
            tmp = 0.0;
            j = i + 1;
            while j < D {
                tmp += matrix_c_ptr(&wXX[0..], i, j, D) * c[j as usize];
                j += 1;
            }
            nrg += c[i as usize]
                * (2.0f32 * tmp + matrix_c_ptr(&wXX[0..], i, i, D) * c[i as usize]);
            i += 1;
        }
        if nrg > 0.0 {
            break;
        } else {
            /* Add white noise */
            i = 0;
            while i < D {
                *matrix_c_ptr_mut(&mut wXX[0..], i, i, D) += regularization;
                i += 1;
            }
            /* Increase noise for next run */
            regularization *= 2.0f32;
        }
        k += 1;
    }
    if k == MAX_ITERATIONS_RESIDUAL_NRG {
        crate::silk::macros::silk_assert!(nrg == 0.0);
        nrg = 1.0;
    }

    nrg
}

/// Calculates residual energies of input subframes where all subframes have LPC_order
/// of preceding samples
pub fn silk_residual_energy_FLP(
    nrgs: &mut [silk_float],
    x: &[silk_float],
    a: &[[silk_float; MAX_LPC_ORDER as usize]; 2],
    gains: &[silk_float],
    subfr_length: opus_int,
    nb_subfr: opus_int,
    LPC_order: opus_int,
) {
    let shift: opus_int;
    let mut LPC_res =
        [0.0f32; ((MAX_FRAME_LENGTH + MAX_NB_SUBFR * MAX_LPC_ORDER) / 2) as usize];

    let LPC_res_ptr = LPC_order as usize; /* LPC_res + LPC_order */
    shift = LPC_order + subfr_length;

    /* Filter input to create the LPC residual for each frame half, and measure subframe energies */
    silk_LPC_analysis_filter_FLP(&mut LPC_res, &a[0], x, 2 * shift, LPC_order);
    /* The C computes `gains[0] * gains[0] * (double)energy`: the two gains are
     * multiplied in f32 first, then the product is promoted to double. */
    nrgs[0] = ((gains[0] * gains[0]) as f64
        * silk_energy_FLP(&LPC_res[LPC_res_ptr..], subfr_length)) as silk_float;
    nrgs[1] = ((gains[1] * gains[1]) as f64
        * silk_energy_FLP(&LPC_res[LPC_res_ptr + shift as usize..], subfr_length))
        as silk_float;

    if nb_subfr == MAX_NB_SUBFR {
        silk_LPC_analysis_filter_FLP(
            &mut LPC_res,
            &a[1],
            &x[(2 * shift) as usize..],
            2 * shift,
            LPC_order,
        );
        nrgs[2] = ((gains[2] * gains[2]) as f64
            * silk_energy_FLP(&LPC_res[LPC_res_ptr..], subfr_length)) as silk_float;
        nrgs[3] = ((gains[3] * gains[3]) as f64
            * silk_energy_FLP(&LPC_res[LPC_res_ptr + shift as usize..], subfr_length))
            as silk_float;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_ar_filter_energy_is_scaled_input_energy() {
        let n = 64;
        let x: Vec<f32> = (0..n).map(|i| (i as f32) * 0.25 - 4.0).collect();
        let a = [[0.0f32; MAX_LPC_ORDER as usize]; 2];
        let gains = [2.0f32, 0.5, 0.0, 0.0];
        let mut nrgs = [0.0f32; 4];
        silk_residual_energy_FLP(&mut nrgs, &x, &a, &gains, 16, 2, 10);
        /* With an all-zero filter the residual equals the input; the first
         * subframe starts at sample LPC_order. */
        let e0: f64 = (10..26).map(|i| x[i] as f64 * x[i] as f64).sum();
        assert_eq!(nrgs[0], (4.0f64 * e0) as f32);
    }
}

//! Mirror of `silk/float/corrMatrix_FLP.c` from libopus 1.6.1, transliterated to safe Rust.
//!
//! Correlation matrix computations for the LS estimate.
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

use crate::silk::float::energy_flp::silk_energy_FLP;
use crate::silk::float::inner_product_flp::silk_inner_product_FLP;
use crate::silk::float::sigproc_flp::silk_float;
use crate::silk::macros::*;
use crate::silk::typedef::*;

/// Calculates correlation vector X'*t
pub fn silk_corrVector_FLP(
    x: &[silk_float],
    t: &[silk_float],
    L: opus_int,
    Order: opus_int,
    Xt: &mut [silk_float],
    arch: opus_int,
) {
    let mut lag: opus_int;

    /* ptr1 = &x[ Order - 1 ]: first sample of column 0 of X: X[:,0] */
    let ptr1 = (Order - 1) as usize;
    lag = 0;
    while lag < Order {
        /* Calculate X[:,lag]'*t */
        Xt[lag as usize] = silk_inner_product_FLP(&x[ptr1 - lag as usize..], t, L, arch)
            as silk_float;
        /* Next column of X */
        lag += 1;
    }
}

/// Calculates correlation matrix X'*X
pub fn silk_corrMatrix_FLP(
    x: &[silk_float],
    L: opus_int,
    Order: opus_int,
    XX: &mut [silk_float],
    arch: opus_int,
) {
    let mut j: opus_int;
    let mut lag: opus_int;
    let mut energy: f64;

    /* ptr1 = &x[ Order - 1 ]: first sample of column 0 of X */
    let ptr1 = (Order - 1) as usize;
    energy = silk_energy_FLP(&x[ptr1..], L); /* X[:,0]'*X[:,0] */
    *matrix_ptr_mut(&mut XX[0..], 0, 0, Order) = energy as silk_float;
    j = 1;
    while j < Order {
        /* Calculate X[:,j]'*X[:,j].  The C products are `float * float`, so
         * each term is rounded to f32 before being added to the `double`
         * accumulator. */
        energy += (x[ptr1 - j as usize] * x[ptr1 - j as usize]
            - x[ptr1 + L as usize - j as usize] * x[ptr1 + L as usize - j as usize])
            as f64;
        *matrix_ptr_mut(&mut XX[0..], j, j, Order) = energy as silk_float;
        j += 1;
    }

    /* ptr2 = &x[ Order - 2 ]: first sample of column 1 of X; the C decrements
     * it once per iteration (`ptr2--` at the end of the loop body), so at
     * iteration `lag` it points at `&x[ Order - 1 - lag ]`. */
    lag = 1;
    while lag < Order {
        let ptr2 = (Order - 1 - lag) as usize;
        /* Calculate X[:,0]'*X[:,lag] */
        energy = silk_inner_product_FLP(&x[ptr1..], &x[ptr2..], L, arch);
        *matrix_ptr_mut(&mut XX[0..], lag, 0, Order) = energy as silk_float;
        *matrix_ptr_mut(&mut XX[0..], 0, lag, Order) = energy as silk_float;
        /* Calculate X[:,j]'*X[:,j + lag] */
        j = 1;
        while j < (Order - lag) {
            energy += (x[ptr1 - j as usize] * x[ptr2 - j as usize]
                - x[ptr1 + L as usize - j as usize] * x[ptr2 + L as usize - j as usize])
                as f64;
            *matrix_ptr_mut(&mut XX[0..], lag + j, j, Order) = energy as silk_float;
            *matrix_ptr_mut(&mut XX[0..], j, lag + j, Order) = energy as silk_float;
            j += 1;
        }
        /* Next column of X (`ptr2--` in the C) */
        lag += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn correlation_matrix_is_symmetric_with_expected_diagonal() {
        let x: [f32; 12] = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0];
        /* L = 8, Order = 3 : x has L + Order - 1 = 10 entries used. */
        let mut xx = [0.0f32; 9];
        silk_corrMatrix_FLP(&x, 8, 3, &mut xx, 0);
        for i in 0..3 {
            for j in 0..3 {
                assert_eq!(xx[i * 3 + j], xx[j * 3 + i], "({i},{j})");
            }
        }
        /* column 0 is x[2..10] -> sum of squares */
        let e: f32 = (3..=10).map(|v| (v * v) as f32).sum();
        assert_eq!(xx[0], e);
    }

    #[test]
    fn correlation_vector_matches_direct_dot_products() {
        let x: [f32; 10] = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0];
        let t: [f32; 8] = [1.0; 8];
        let mut xt = [0.0f32; 3];
        silk_corrVector_FLP(&x, &t, 8, 3, &mut xt, 0);
        assert_eq!(xt[0], (3..=10).map(|v| v as f32).sum::<f32>());
        assert_eq!(xt[1], (2..=9).map(|v| v as f32).sum::<f32>());
        assert_eq!(xt[2], (1..=8).map(|v| v as f32).sum::<f32>());
    }
}

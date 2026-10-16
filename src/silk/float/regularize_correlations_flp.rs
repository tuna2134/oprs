//! Mirror of `silk/float/regularize_correlations_FLP.c` from libopus 1.6.1, transliterated to safe Rust.
//!
//! Add noise to the correlation matrix diagonal.
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

use crate::silk::float::sigproc_flp::silk_float;
use crate::silk::macros::*;
use crate::silk::typedef::*;

/// Add noise to matrix diagonal
pub fn silk_regularize_correlations_FLP(
    XX: &mut [silk_float],
    xx: &mut [silk_float],
    noise: silk_float,
    D: opus_int,
) {
    let mut i: opus_int;

    i = 0;
    while i < D {
        *matrix_ptr_mut(&mut XX[0..], i, i, D) += noise;
        i += 1;
    }
    xx[0] += noise;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_noise_to_diagonal_only() {
        let mut xx = [1.0f32; 4];
        let mut x = [0.0f32; 1];
        silk_regularize_correlations_FLP(&mut xx, &mut x, 0.5, 2);
        assert_eq!(xx, [1.5, 1.0, 1.0, 1.5]);
        assert_eq!(x[0], 0.5);
    }
}

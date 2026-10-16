//! Mirror of `silk/interpolate.c` from libopus 1.6.1, transliterated to safe Rust.
//!
//! Interpolation of two int16 vectors (LPC coefficients, gains, ...).
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
use crate::silk::sigproc_fix::*;
use crate::silk::typedef::*;
use crate::types::*;

/// `silk_interpolate(xi, x0, x1, ifact_Q2, d)`
pub fn silk_interpolate(
    xi: &mut [opus_int16],
    x0: &[opus_int16],
    x1: &[opus_int16],
    ifact_Q2: opus_int,
    d: opus_int,
) {
    celt_assert!(ifact_Q2 >= 0);
    celt_assert!(ifact_Q2 <= 4);

    for i in 0..d {
        xi[i as usize] = silk_ADD_RSHIFT(
            x0[i as usize] as opus_int32,
            silk_SMULBB(x1[i as usize] as opus_int32 - x0[i as usize] as opus_int32, ifact_Q2),
            2,
        ) as opus_int16;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_weight_returns_first_vector() {
        let x0 = [10i16, -20, 300];
        let x1 = [30i16, 40, -700];
        let mut xi = [0i16; 3];
        silk_interpolate(&mut xi, &x0, &x1, 0, 3);
        assert_eq!(xi, x0);
    }

    #[test]
    fn full_weight_returns_second_vector() {
        let x0 = [10i16, -20, 300];
        let x1 = [30i16, 40, -700];
        let mut xi = [0i16; 3];
        silk_interpolate(&mut xi, &x0, &x1, 4, 3);
        assert_eq!(xi, x1);
    }
}

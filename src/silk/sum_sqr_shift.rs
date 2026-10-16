//! Mirror of `silk/sum_sqr_shift.c` from libopus 1.6.1, transliterated to safe Rust.
//!
//! Compute the number of bits to right shift the sum of squares of a vector of
//! int16s to make it fit in an int32.
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

use crate::silk::macros::*;
use crate::silk::sigproc_fix::*;
use crate::silk::typedef::*;
use crate::types::*;

/// `silk_sum_sqr_shift(energy, shift, x, len)` — the two C out-parameters are
/// returned as `(energy, shift)`.
pub fn silk_sum_sqr_shift(x: &[opus_int16], len: opus_int) -> (opus_int32, opus_int) {
    let mut i: opus_int;
    let mut shft: opus_int;
    let mut nrg_tmp: opus_uint32;
    let mut nrg: opus_int32;

    /* Do a first run with the maximum shift we could have. */
    shft = 31 - silk_CLZ32(len);
    /* Let's be conservative with rounding and start with nrg=len. */
    nrg = len;
    i = 0;
    while i < len - 1 {
        nrg_tmp =
            silk_SMULBB(x[i as usize] as opus_int32, x[i as usize] as opus_int32) as opus_uint32;
        nrg_tmp = silk_SMLABB_ovflw(
            nrg_tmp as opus_int32,
            x[(i + 1) as usize] as opus_int32,
            x[(i + 1) as usize] as opus_int32,
        ) as opus_uint32;
        nrg = silk_ADD_RSHIFT_uint(nrg as opus_uint32, nrg_tmp, shft) as opus_int32;
        i += 2;
    }
    if i < len {
        /* One sample left to process */
        nrg_tmp =
            silk_SMULBB(x[i as usize] as opus_int32, x[i as usize] as opus_int32) as opus_uint32;
        nrg = silk_ADD_RSHIFT_uint(nrg as opus_uint32, nrg_tmp, shft) as opus_int32;
    }
    silk_assert!(nrg >= 0);
    /* Make sure the result will fit in a 32-bit signed integer with two bits
       of headroom. */
    shft = silk_max_32(0, shft + 3 - silk_CLZ32(nrg));
    nrg = 0;
    i = 0;
    while i < len - 1 {
        nrg_tmp =
            silk_SMULBB(x[i as usize] as opus_int32, x[i as usize] as opus_int32) as opus_uint32;
        nrg_tmp = silk_SMLABB_ovflw(
            nrg_tmp as opus_int32,
            x[(i + 1) as usize] as opus_int32,
            x[(i + 1) as usize] as opus_int32,
        ) as opus_uint32;
        nrg = silk_ADD_RSHIFT_uint(nrg as opus_uint32, nrg_tmp, shft) as opus_int32;
        i += 2;
    }
    if i < len {
        /* One sample left to process */
        nrg_tmp =
            silk_SMULBB(x[i as usize] as opus_int32, x[i as usize] as opus_int32) as opus_uint32;
        nrg = silk_ADD_RSHIFT_uint(nrg as opus_uint32, nrg_tmp, shft) as opus_int32;
    }

    silk_assert!(nrg >= 0);

    /* Output arguments */
    (nrg, shft)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_vector() {
        let x = [1i16, 2, 3, 4];
        assert_eq!(silk_sum_sqr_shift(&x, 4), (30, 0));
    }

    #[test]
    fn all_zeros() {
        let x = [0i16; 16];
        assert_eq!(silk_sum_sqr_shift(&x, 16), (0, 0));
    }

    #[test]
    fn large_vector_needs_a_shift() {
        let x = [-32768i16; 320];
        let (energy, shift) = silk_sum_sqr_shift(&x, 320);
        assert!(shift > 0);
        assert!(energy >= 0);
    }
}

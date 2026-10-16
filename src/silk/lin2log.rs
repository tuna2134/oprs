//! Mirror of `silk/lin2log.c` from libopus 1.6.1, transliterated to safe Rust.
//!
//! Approximation of `128 * log2()` (very close inverse of `silk_log2lin()`).
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

use crate::silk::inlines::*;
use crate::silk::macros::*;
use crate::silk::sigproc_fix::*;
use crate::types::*;

/// `silk_lin2log(inLin)` — convert input to a log scale.
pub fn silk_lin2log(inLin: opus_int32) -> opus_int32 {
    let (lz, frac_Q7) = silk_CLZ_FRAC(inLin);

    /* Piece-wise parabolic approximation */
    silk_ADD_LSHIFT32(
        silk_SMLAWB(frac_Q7, silk_MUL(frac_Q7, 128 - frac_Q7), 179),
        31 - lz,
        7,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::silk::log2lin::silk_log2lin;

    #[test]
    fn negative_infinity_maps_to_minus_128() {
        assert_eq!(silk_lin2log(0), -128);
    }

    #[test]
    fn powers_of_two_map_to_128k() {
        for k in 0..31 {
            assert_eq!(silk_lin2log(1 << k), 128 * k);
        }
    }

    #[test]
    fn monotone_on_positive_inputs() {
        let mut prev = i32::MIN;
        for k in 1..=20000i32 {
            let v = silk_lin2log(k);
            assert!(v >= prev);
            prev = v;
        }
    }

    #[test]
    fn roundtrip_with_log2lin() {
        for k in 0..24 {
            let x = 1i32 << k;
            let y = silk_log2lin(silk_lin2log(x));
            assert!((y - x).abs() <= x / 4 + 2);
        }
    }
}

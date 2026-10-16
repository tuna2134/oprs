//! Mirror of `silk/bwexpander.c` from libopus 1.6.1, transliterated to safe Rust.
//!
//! Chirp (bandwidth expand) LP AR filter.
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

use crate::silk::sigproc_fix::*;
use crate::silk::typedef::*;
use crate::types::*;

/// `silk_bwexpander(ar, d, chirp_Q16)`
pub fn silk_bwexpander(ar: &mut [opus_int16], d: opus_int, chirp_Q16: opus_int32) {
    let mut chirp_Q16 = chirp_Q16;
    let chirp_minus_one_Q16 = chirp_Q16.wrapping_sub(65536);

    /* NB: Dont use silk_SMULWB, instead of silk_RSHIFT_ROUND( silk_MUL(), 16 ), below.  */
    /* Bias in silk_SMULWB can lead to unstable filters                                */
    for i in 0..d - 1 {
        ar[i as usize] = silk_RSHIFT_ROUND(silk_MUL(chirp_Q16, ar[i as usize] as opus_int32), 16)
            as opus_int16;
        chirp_Q16 = chirp_Q16
            .wrapping_add(silk_RSHIFT_ROUND(silk_MUL(chirp_Q16, chirp_minus_one_Q16), 16));
    }
    ar[(d - 1) as usize] =
        silk_RSHIFT_ROUND(silk_MUL(chirp_Q16, ar[(d - 1) as usize] as opus_int32), 16) as opus_int16;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chirp_one_is_identity() {
        let mut ar = [100i16, -2000, 3000, -4];
        let orig = ar;
        silk_bwexpander(&mut ar, 4, 65536);
        assert_eq!(ar, orig);
    }

    #[test]
    fn chirp_zero_zeroes_the_filter() {
        let mut ar = [100i16, -2000, 3000, -4];
        silk_bwexpander(&mut ar, 4, 0);
        assert_eq!(ar, [0, 0, 0, 0]);
    }
}

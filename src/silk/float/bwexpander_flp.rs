//! Mirror of `silk/float/bwexpander_FLP.c` from libopus 1.6.1, transliterated to safe Rust.
//!
//! Chirp (bandwidth expansion) of an LP AR filter.
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
use crate::silk::typedef::*;

/// Chirp (bw expand) LP AR filter
pub fn silk_bwexpander_FLP(ar: &mut [silk_float], d: opus_int, chirp: silk_float) {
    let mut i: opus_int;
    let mut cfac: silk_float = chirp;

    i = 0;
    while i < d - 1 {
        ar[i as usize] *= cfac;
        cfac *= chirp;
        i += 1;
    }
    ar[(d - 1) as usize] *= cfac;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chirp_powers() {
        let mut ar = [1.0f32, 1.0, 1.0, 1.0];
        silk_bwexpander_FLP(&mut ar, 4, 0.5);
        assert_eq!(ar, [0.5, 0.25, 0.125, 0.0625]);
    }
}

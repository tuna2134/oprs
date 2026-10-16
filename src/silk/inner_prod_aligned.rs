//! Mirror of `silk/inner_prod_aligned.c` from libopus 1.6.1, transliterated to safe Rust.
//!
//! Scaled inner product of two aligned int16 vectors.  The reference build is
//! floating point, but `silk_inner_prod_aligned_scale` only exists in the
//! integer source; the (bit-exact) C loop is ported.
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

/// `silk_inner_prod_aligned_scale(inVec1, inVec2, scale, len)`
pub fn silk_inner_prod_aligned_scale(
    inVec1: &[opus_int16],
    inVec2: &[opus_int16],
    scale: opus_int,
    len: opus_int,
) -> opus_int32 {
    let mut sum: opus_int32 = 0;
    for i in 0..len {
        sum = silk_ADD_RSHIFT32(
            sum,
            silk_SMULBB(
                inVec1[i as usize] as opus_int32,
                inVec2[i as usize] as opus_int32,
            ),
            scale,
        );
    }
    sum
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scaled_inner_product() {
        let a = [1i16, 2, 3];
        let b = [4i16, 5, 6];
        assert_eq!(silk_inner_prod_aligned_scale(&a, &b, 0, 3), 32);
        assert_eq!(silk_inner_prod_aligned_scale(&a, &b, 1, 3), 16);
    }
}

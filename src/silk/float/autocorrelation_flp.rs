//! Mirror of `silk/float/autocorrelation_FLP.c` from libopus 1.6.1, transliterated to safe Rust.
//!
//! Autocorrelation sequence via the dispatched `silk_inner_product_FLP`.
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

use crate::silk::float::inner_product_flp::silk_inner_product_FLP;
use crate::silk::float::sigproc_flp::silk_float;
use crate::silk::typedef::*;

/// compute autocorrelation
pub fn silk_autocorrelation_FLP(
    results: &mut [silk_float],
    inputData: &[silk_float],
    inputDataSize: opus_int,
    correlationCount: opus_int,
    arch: opus_int,
) {
    let mut i: opus_int;
    let mut correlationCount = correlationCount;

    if correlationCount > inputDataSize {
        correlationCount = inputDataSize;
    }

    i = 0;
    while i < correlationCount {
        results[i as usize] = silk_inner_product_FLP(
            inputData,
            &inputData[i as usize..],
            inputDataSize - i,
            arch,
        ) as silk_float;
        i += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamps_correlation_count_to_input_size() {
        let x = [1.0f32, 2.0, 3.0];
        let mut r = [0.0f32; 5];
        silk_autocorrelation_FLP(&mut r, &x, 3, 5, 0);
        assert_eq!(r[0], 14.0);
        assert_eq!(r[1], 8.0);
        assert_eq!(r[2], 3.0);
        assert_eq!(r[3], 0.0);
    }
}

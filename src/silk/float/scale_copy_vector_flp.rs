//! Mirror of `silk/float/scale_copy_vector_FLP.c` from libopus 1.6.1, transliterated to safe Rust.
//!
//! Copy and multiply a vector by a constant.
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

/// copy and multiply a vector by a constant
pub fn silk_scale_copy_vector_FLP(
    data_out: &mut [silk_float],
    data_in: &[silk_float],
    gain: silk_float,
    dataSize: opus_int,
) {
    let mut i: opus_int;
    let dataSize4: opus_int;

    /* 4x unrolled loop */
    dataSize4 = dataSize & 0xFFFC;
    i = 0;
    while i < dataSize4 {
        let b = i as usize;
        data_out[b + 0] = gain * data_in[b + 0];
        data_out[b + 1] = gain * data_in[b + 1];
        data_out[b + 2] = gain * data_in[b + 2];
        data_out[b + 3] = gain * data_in[b + 3];
        i += 4;
    }

    /* any remaining elements */
    while i < dataSize {
        data_out[i as usize] = gain * data_in[i as usize];
        i += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copies_scaled() {
        let src = [1.0f32, -2.0, 3.0, -4.0, 5.0];
        let mut dst = [0.0f32; 5];
        silk_scale_copy_vector_FLP(&mut dst, &src, 0.5, 5);
        assert_eq!(dst, [0.5, -1.0, 1.5, -2.0, 2.5]);
    }
}

//! Mirror of `silk/resampler_private_AR2.c` from libopus 1.6.1, transliterated to safe Rust.
//!
//! Second order AR filter with single delay elements.
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
use crate::types::*;

/// `silk_resampler_private_AR2(S, out_Q8, in, A_Q14, len)`
pub fn silk_resampler_private_AR2(
    S: &mut [opus_int32],      /* I/O  State vector [ 2 ] */
    out_Q8: &mut [opus_int32], /* O    Output signal */
    in_: &[opus_int16],        /* I    Input signal */
    A_Q14: &[opus_int16],      /* I    AR coefficients, Q14 */
    len: opus_int32,           /* I    Signal length */
) {
    let mut out32: opus_int32;

    for k in 0..len {
        out32 = silk_ADD_LSHIFT32(S[0], in_[k as usize] as opus_int32, 8);
        out_Q8[k as usize] = out32;
        out32 = silk_LSHIFT(out32, 2);
        S[0] = silk_SMLAWB(S[1], out32, A_Q14[0] as opus_int32);
        S[1] = silk_SMULWB(out32, A_Q14[1] as opus_int32);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_state_passes_shifted_input() {
        let mut s = [0i32; 2];
        let mut out = [0i32; 4];
        let in_ = [1i16, 2, 3, 4];
        silk_resampler_private_AR2(&mut s, &mut out, &in_, &[0i16, 0], 4);
        assert_eq!(out, [1 << 8, 2 << 8, 3 << 8, 4 << 8]);
        assert_eq!(s, [0, 0]);
    }
}

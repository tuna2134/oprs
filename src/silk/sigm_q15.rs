//! Mirror of `silk/sigm_Q15.c` from libopus 1.6.1, transliterated to safe Rust.
//!
//! Approximate sigmoid function.
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

/* fprintf(1, '%d, ', round(1024 * ([1 ./ (1 + exp(-(1:5))), 1] - 1 ./ (1 + exp(-(0:5)))))); */
static sigm_LUT_slope_Q10: [opus_int32; 6] = [237, 153, 73, 30, 12, 7];
/* fprintf(1, '%d, ', round(32767 * 1 ./ (1 + exp(-(0:5))))); */
static sigm_LUT_pos_Q15: [opus_int32; 6] = [16384, 23955, 28861, 31213, 32178, 32548];
/* fprintf(1, '%d, ', round(32767 * 1 ./ (1 + exp((0:5))))); */
static sigm_LUT_neg_Q15: [opus_int32; 6] = [16384, 8812, 3906, 1554, 589, 219];

/// `silk_sigm_Q15(in_Q5)`
pub fn silk_sigm_Q15(mut in_Q5: opus_int) -> opus_int {
    let ind: opus_int;

    if in_Q5 < 0 {
        /* Negative input */
        in_Q5 = -in_Q5;
        if in_Q5 >= 6 * 32 {
            return 0; /* Clip */
        } else {
            /* Linear interpolation of look up table */
            ind = silk_RSHIFT(in_Q5, 5);
            return sigm_LUT_neg_Q15[ind as usize]
                - silk_SMULBB(sigm_LUT_slope_Q10[ind as usize], in_Q5 & 0x1F);
        }
    } else {
        /* Positive input */
        if in_Q5 >= 6 * 32 {
            return 32767; /* clip */
        } else {
            /* Linear interpolation of look up table */
            ind = silk_RSHIFT(in_Q5, 5);
            return sigm_LUT_pos_Q15[ind as usize]
                + silk_SMULBB(sigm_LUT_slope_Q10[ind as usize], in_Q5 & 0x1F);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn midpoint_and_clipping() {
        assert_eq!(silk_sigm_Q15(0), 16384);
        assert_eq!(silk_sigm_Q15(160), 32548);
        assert_eq!(silk_sigm_Q15(192), 32767);
        assert_eq!(silk_sigm_Q15(-192), 0);
        assert_eq!(silk_sigm_Q15(1000000), 32767);
        assert_eq!(silk_sigm_Q15(-1000000), 0);
    }

    #[test]
    fn always_in_range() {
        for i in -600..=600 {
            let v = silk_sigm_Q15(i);
            assert!((0..=32767).contains(&v));
        }
    }
}

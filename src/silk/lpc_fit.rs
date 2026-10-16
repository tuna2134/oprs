//! Mirror of `silk/LPC_fit.c` from libopus 1.6.1, transliterated to safe Rust.
//!
//! Convert int32 coefficients to int16 coefs and make sure there's no
//! wrap-around.  This logic is reused in `_celt_lpc()`. Any bug fixes should
//! also be applied there.
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

use crate::silk::bwexpander_32::*;
use crate::silk::sigproc_fix::*;
use crate::silk::typedef::*;
use crate::types::*;

/// `silk_LPC_fit(a_QOUT, a_QIN, QOUT, QIN, d)`
pub fn silk_LPC_fit(
    a_QOUT: &mut [opus_int16],   /* O    Output signal */
    a_QIN: &mut [opus_int32],    /* I/O  Input signal */
    QOUT: opus_int,              /* I    Output Q domain */
    QIN: opus_int,               /* I    Input Q domain */
    d: opus_int,                 /* I    Filter order */
) {
    let mut i: opus_int;
    let mut idx: opus_int = 0;
    let mut maxabs: opus_int32;
    let mut absval: opus_int32;
    let mut chirp_Q16: opus_int32;

    /* Limit the maximum absolute value of the prediction coefficients, so that they'll fit in int16 */
    i = 0;
    while i < 10 {
        /* Find maximum absolute value and its index */
        maxabs = 0;
        for k in 0..d {
            absval = silk_abs(a_QIN[k as usize]);
            if absval > maxabs {
                maxabs = absval;
                idx = k;
            }
        }
        maxabs = silk_RSHIFT_ROUND(maxabs, QIN - QOUT);

        if maxabs > silk_int16_MAX as opus_int32 {
            /* Reduce magnitude of prediction coefficients */
            maxabs = silk_min(maxabs, 163838); /* ( silk_int32_MAX >> 14 ) + silk_int16_MAX = 163838 */
            chirp_Q16 = SILK_FIX_CONST(0.999, 16)
                - silk_DIV32(
                    silk_LSHIFT(maxabs - silk_int16_MAX as opus_int32, 14),
                    silk_RSHIFT32(silk_MUL(maxabs, idx + 1), 2),
                );
            silk_bwexpander_32(a_QIN, d, chirp_Q16);
        } else {
            break;
        }
        i += 1;
    }

    if i == 10 {
        /* Reached the last iteration, clip the coefficients */
        for k in 0..d {
            a_QOUT[k as usize] = silk_SAT16(silk_RSHIFT_ROUND(a_QIN[k as usize], QIN - QOUT)) as opus_int16;
            a_QIN[k as usize] = silk_LSHIFT(a_QOUT[k as usize] as opus_int32, QIN - QOUT);
        }
    } else {
        for k in 0..d {
            a_QOUT[k as usize] = silk_RSHIFT_ROUND(a_QIN[k as usize], QIN - QOUT) as opus_int16;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_shift() {
        let mut qin = [16i32, -32, 48];
        let mut qout = [0i16; 3];
        silk_LPC_fit(&mut qout, &mut qin, 12, 16, 3);
        assert_eq!(qout, [1, -2, 3]);
    }

    #[test]
    fn values_that_already_fit_are_unchanged() {
        let mut qin = [0i32; 16];
        for (i, v) in qin.iter_mut().enumerate() {
            *v = (i as i32 - 8) << 4;
        }
        let orig = qin;
        let mut qout = [0i16; 16];
        silk_LPC_fit(&mut qout, &mut qin, 12, 16, 16);
        assert_eq!(qin, orig);
        for i in 0..16 {
            assert_eq!(qout[i] as i32, i as i32 - 8);
        }
    }
}

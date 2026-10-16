//! Mirror of `silk/resampler_private_down_FIR.c` from libopus 1.6.1, transliterated to safe Rust.
//!
//! Resample with a 2nd order AR filter followed by FIR interpolation.
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

use crate::celt::arch::celt_assert;
use crate::silk::macros::*;
use crate::silk::resampler_private_ar2::*;
use crate::silk::resampler_rom::*;
use crate::silk::sigproc_fix::*;
use crate::silk::structs::*;
use crate::silk::typedef::*;
use crate::types::*;

/// `silk_resampler_private_down_FIR_INTERPOL(out, buf, FIR_Coefs, FIR_Order, FIR_Fracs, max_index_Q16, index_increment_Q16)`
/// returns the number of samples written to `out`.
fn silk_resampler_private_down_FIR_INTERPOL(
    out: &mut [opus_int16],
    buf: &[opus_int32],
    FIR_Coefs: &[opus_int16],
    FIR_Order: opus_int,
    FIR_Fracs: opus_int,
    max_index_Q16: opus_int32,
    index_increment_Q16: opus_int32,
) -> usize {
    let mut res_Q6: opus_int32;
    let mut n: usize = 0;
    let mut index_Q16: opus_int32;

    match FIR_Order {
        RESAMPLER_DOWN_ORDER_FIR0 => {
            index_Q16 = 0;
            while index_Q16 < max_index_Q16 {
                /* Integer part gives pointer to buffered input */
                let buf_ptr = silk_RSHIFT(index_Q16, 16) as usize;

                /* Fractional part gives interpolation coefficients */
                let interpol_ind = silk_SMULWB(index_Q16 & 0xFFFF, FIR_Fracs);

                /* Inner product */
                let mut interpol_ptr = (RESAMPLER_DOWN_ORDER_FIR0 / 2 * interpol_ind) as usize;
                res_Q6 = silk_SMULWB(buf[buf_ptr], FIR_Coefs[interpol_ptr] as opus_int32);
                res_Q6 = silk_SMLAWB(res_Q6, buf[buf_ptr + 1], FIR_Coefs[interpol_ptr + 1] as opus_int32);
                res_Q6 = silk_SMLAWB(res_Q6, buf[buf_ptr + 2], FIR_Coefs[interpol_ptr + 2] as opus_int32);
                res_Q6 = silk_SMLAWB(res_Q6, buf[buf_ptr + 3], FIR_Coefs[interpol_ptr + 3] as opus_int32);
                res_Q6 = silk_SMLAWB(res_Q6, buf[buf_ptr + 4], FIR_Coefs[interpol_ptr + 4] as opus_int32);
                res_Q6 = silk_SMLAWB(res_Q6, buf[buf_ptr + 5], FIR_Coefs[interpol_ptr + 5] as opus_int32);
                res_Q6 = silk_SMLAWB(res_Q6, buf[buf_ptr + 6], FIR_Coefs[interpol_ptr + 6] as opus_int32);
                res_Q6 = silk_SMLAWB(res_Q6, buf[buf_ptr + 7], FIR_Coefs[interpol_ptr + 7] as opus_int32);
                res_Q6 = silk_SMLAWB(res_Q6, buf[buf_ptr + 8], FIR_Coefs[interpol_ptr + 8] as opus_int32);
                interpol_ptr = (RESAMPLER_DOWN_ORDER_FIR0 / 2 * (FIR_Fracs - 1 - interpol_ind)) as usize;
                res_Q6 = silk_SMLAWB(res_Q6, buf[buf_ptr + 17], FIR_Coefs[interpol_ptr] as opus_int32);
                res_Q6 = silk_SMLAWB(res_Q6, buf[buf_ptr + 16], FIR_Coefs[interpol_ptr + 1] as opus_int32);
                res_Q6 = silk_SMLAWB(res_Q6, buf[buf_ptr + 15], FIR_Coefs[interpol_ptr + 2] as opus_int32);
                res_Q6 = silk_SMLAWB(res_Q6, buf[buf_ptr + 14], FIR_Coefs[interpol_ptr + 3] as opus_int32);
                res_Q6 = silk_SMLAWB(res_Q6, buf[buf_ptr + 13], FIR_Coefs[interpol_ptr + 4] as opus_int32);
                res_Q6 = silk_SMLAWB(res_Q6, buf[buf_ptr + 12], FIR_Coefs[interpol_ptr + 5] as opus_int32);
                res_Q6 = silk_SMLAWB(res_Q6, buf[buf_ptr + 11], FIR_Coefs[interpol_ptr + 6] as opus_int32);
                res_Q6 = silk_SMLAWB(res_Q6, buf[buf_ptr + 10], FIR_Coefs[interpol_ptr + 7] as opus_int32);
                res_Q6 = silk_SMLAWB(res_Q6, buf[buf_ptr + 9], FIR_Coefs[interpol_ptr + 8] as opus_int32);

                /* Scale down, saturate and store in output array */
                out[n] = silk_SAT16(silk_RSHIFT_ROUND(res_Q6, 6)) as opus_int16;
                n += 1;
                index_Q16 += index_increment_Q16;
            }
        }
        RESAMPLER_DOWN_ORDER_FIR1 => {
            index_Q16 = 0;
            while index_Q16 < max_index_Q16 {
                /* Integer part gives pointer to buffered input */
                let buf_ptr = silk_RSHIFT(index_Q16, 16) as usize;

                /* Inner product */
                res_Q6 = silk_SMULWB(silk_ADD32(buf[buf_ptr], buf[buf_ptr + 23]), FIR_Coefs[0] as opus_int32);
                res_Q6 = silk_SMLAWB(res_Q6, silk_ADD32(buf[buf_ptr + 1], buf[buf_ptr + 22]), FIR_Coefs[1] as opus_int32);
                res_Q6 = silk_SMLAWB(res_Q6, silk_ADD32(buf[buf_ptr + 2], buf[buf_ptr + 21]), FIR_Coefs[2] as opus_int32);
                res_Q6 = silk_SMLAWB(res_Q6, silk_ADD32(buf[buf_ptr + 3], buf[buf_ptr + 20]), FIR_Coefs[3] as opus_int32);
                res_Q6 = silk_SMLAWB(res_Q6, silk_ADD32(buf[buf_ptr + 4], buf[buf_ptr + 19]), FIR_Coefs[4] as opus_int32);
                res_Q6 = silk_SMLAWB(res_Q6, silk_ADD32(buf[buf_ptr + 5], buf[buf_ptr + 18]), FIR_Coefs[5] as opus_int32);
                res_Q6 = silk_SMLAWB(res_Q6, silk_ADD32(buf[buf_ptr + 6], buf[buf_ptr + 17]), FIR_Coefs[6] as opus_int32);
                res_Q6 = silk_SMLAWB(res_Q6, silk_ADD32(buf[buf_ptr + 7], buf[buf_ptr + 16]), FIR_Coefs[7] as opus_int32);
                res_Q6 = silk_SMLAWB(res_Q6, silk_ADD32(buf[buf_ptr + 8], buf[buf_ptr + 15]), FIR_Coefs[8] as opus_int32);
                res_Q6 = silk_SMLAWB(res_Q6, silk_ADD32(buf[buf_ptr + 9], buf[buf_ptr + 14]), FIR_Coefs[9] as opus_int32);
                res_Q6 = silk_SMLAWB(res_Q6, silk_ADD32(buf[buf_ptr + 10], buf[buf_ptr + 13]), FIR_Coefs[10] as opus_int32);
                res_Q6 = silk_SMLAWB(res_Q6, silk_ADD32(buf[buf_ptr + 11], buf[buf_ptr + 12]), FIR_Coefs[11] as opus_int32);

                /* Scale down, saturate and store in output array */
                out[n] = silk_SAT16(silk_RSHIFT_ROUND(res_Q6, 6)) as opus_int16;
                n += 1;
                index_Q16 += index_increment_Q16;
            }
        }
        RESAMPLER_DOWN_ORDER_FIR2 => {
            index_Q16 = 0;
            while index_Q16 < max_index_Q16 {
                /* Integer part gives pointer to buffered input */
                let buf_ptr = silk_RSHIFT(index_Q16, 16) as usize;

                /* Inner product */
                res_Q6 = silk_SMULWB(silk_ADD32(buf[buf_ptr], buf[buf_ptr + 35]), FIR_Coefs[0] as opus_int32);
                res_Q6 = silk_SMLAWB(res_Q6, silk_ADD32(buf[buf_ptr + 1], buf[buf_ptr + 34]), FIR_Coefs[1] as opus_int32);
                res_Q6 = silk_SMLAWB(res_Q6, silk_ADD32(buf[buf_ptr + 2], buf[buf_ptr + 33]), FIR_Coefs[2] as opus_int32);
                res_Q6 = silk_SMLAWB(res_Q6, silk_ADD32(buf[buf_ptr + 3], buf[buf_ptr + 32]), FIR_Coefs[3] as opus_int32);
                res_Q6 = silk_SMLAWB(res_Q6, silk_ADD32(buf[buf_ptr + 4], buf[buf_ptr + 31]), FIR_Coefs[4] as opus_int32);
                res_Q6 = silk_SMLAWB(res_Q6, silk_ADD32(buf[buf_ptr + 5], buf[buf_ptr + 30]), FIR_Coefs[5] as opus_int32);
                res_Q6 = silk_SMLAWB(res_Q6, silk_ADD32(buf[buf_ptr + 6], buf[buf_ptr + 29]), FIR_Coefs[6] as opus_int32);
                res_Q6 = silk_SMLAWB(res_Q6, silk_ADD32(buf[buf_ptr + 7], buf[buf_ptr + 28]), FIR_Coefs[7] as opus_int32);
                res_Q6 = silk_SMLAWB(res_Q6, silk_ADD32(buf[buf_ptr + 8], buf[buf_ptr + 27]), FIR_Coefs[8] as opus_int32);
                res_Q6 = silk_SMLAWB(res_Q6, silk_ADD32(buf[buf_ptr + 9], buf[buf_ptr + 26]), FIR_Coefs[9] as opus_int32);
                res_Q6 = silk_SMLAWB(res_Q6, silk_ADD32(buf[buf_ptr + 10], buf[buf_ptr + 25]), FIR_Coefs[10] as opus_int32);
                res_Q6 = silk_SMLAWB(res_Q6, silk_ADD32(buf[buf_ptr + 11], buf[buf_ptr + 24]), FIR_Coefs[11] as opus_int32);
                res_Q6 = silk_SMLAWB(res_Q6, silk_ADD32(buf[buf_ptr + 12], buf[buf_ptr + 23]), FIR_Coefs[12] as opus_int32);
                res_Q6 = silk_SMLAWB(res_Q6, silk_ADD32(buf[buf_ptr + 13], buf[buf_ptr + 22]), FIR_Coefs[13] as opus_int32);
                res_Q6 = silk_SMLAWB(res_Q6, silk_ADD32(buf[buf_ptr + 14], buf[buf_ptr + 21]), FIR_Coefs[14] as opus_int32);
                res_Q6 = silk_SMLAWB(res_Q6, silk_ADD32(buf[buf_ptr + 15], buf[buf_ptr + 20]), FIR_Coefs[15] as opus_int32);
                res_Q6 = silk_SMLAWB(res_Q6, silk_ADD32(buf[buf_ptr + 16], buf[buf_ptr + 19]), FIR_Coefs[16] as opus_int32);
                res_Q6 = silk_SMLAWB(res_Q6, silk_ADD32(buf[buf_ptr + 17], buf[buf_ptr + 18]), FIR_Coefs[17] as opus_int32);

                /* Scale down, saturate and store in output array */
                out[n] = silk_SAT16(silk_RSHIFT_ROUND(res_Q6, 6)) as opus_int16;
                n += 1;
                index_Q16 += index_increment_Q16;
            }
        }
        _ => {
            celt_assert!(false);
        }
    }
    n
}

/// `silk_resampler_private_down_FIR(SS, out, in, inLen)`
pub fn silk_resampler_private_down_FIR(
    S: &mut silk_resampler_state_struct, /* I/O  Resampler state */
    out: &mut [opus_int16],              /* O    Output signal */
    in_: &[opus_int16],                  /* I    Input signal */
    inLen: opus_int32,                   /* I    Number of input samples */
) {
    let mut nSamplesIn: opus_int32;
    let mut max_index_Q16: opus_int32;
    let index_increment_Q16: opus_int32;
    let fir_order = S.FIR_Order;
    let fir_fracs = S.FIR_Fracs;
    let batch_size = S.batchSize;
    let coefs = S.Coefs;
    let mut buf = vec![0 as opus_int32; (batch_size + fir_order) as usize];
    let mut out_pos: usize = 0;
    let mut in_pos: usize = 0;
    let mut in_len = inLen;

    /* Copy buffered samples to start of buffer */
    buf[..fir_order as usize]
        .copy_from_slice(unsafe { &S.sFIR.i32[..fir_order as usize] });

    let FIR_Coefs = &coefs[2..];

    /* Iterate over blocks of frameSizeIn input samples */
    index_increment_Q16 = S.invRatio_Q16;
    loop {
        nSamplesIn = silk_min(in_len, batch_size);

        /* Second-order AR filter (output in Q8) */
        silk_resampler_private_AR2(
            &mut S.sIIR,
            &mut buf[fir_order as usize..],
            &in_[in_pos..],
            coefs,
            nSamplesIn,
        );

        max_index_Q16 = silk_LSHIFT32(nSamplesIn, 16);

        /* Interpolate filtered signal */
        let written = silk_resampler_private_down_FIR_INTERPOL(
            &mut out[out_pos..],
            &buf,
            FIR_Coefs,
            fir_order,
            fir_fracs,
            max_index_Q16,
            index_increment_Q16,
        );
        out_pos += written;

        in_pos += nSamplesIn as usize;
        in_len -= nSamplesIn;

        if in_len > 1 {
            /* More iterations to do; copy last part of filtered signal to beginning of buffer */
            buf.copy_within(
                nSamplesIn as usize..nSamplesIn as usize + fir_order as usize,
                0,
            );
        } else {
            break;
        }
    }

    /* Copy last part of filtered signal to the state for the next call */
    unsafe {
        S.sFIR.i32[..fir_order as usize].copy_from_slice(
            &buf[nSamplesIn as usize..nSamplesIn as usize + fir_order as usize],
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::silk::resampler::silk_resampler_init;

    #[test]
    fn zero_input_gives_zero_output() {
        let mut s = silk_resampler_state_struct::default();
        silk_resampler_init(&mut s, 48000, 8000, 1);
        let in_ = [0i16; 480];
        let mut out = [0i16; 2048];
        silk_resampler_private_down_FIR(&mut s, &mut out, &in_, 480);
        assert!(out.iter().all(|&v| v == 0));
    }
}

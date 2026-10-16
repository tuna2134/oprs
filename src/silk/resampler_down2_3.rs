//! Mirror of `silk/resampler_down2_3.c` from libopus 1.6.1, transliterated to safe Rust.
//!
//! Downsample by a factor 2/3, low quality.
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
use crate::silk::resampler_private_ar2::*;
use crate::silk::resampler_rom::*;
use crate::silk::sigproc_fix::*;
use crate::types::*;

const ORDER_FIR: opus_int32 = 4;

/// `silk_resampler_down2_3(S, out, in, inLen)`
pub fn silk_resampler_down2_3(
    S: &mut [opus_int32],     /* I/O  State vector [ 6 ] */
    out: &mut [opus_int16],   /* O    Output signal [ floor(2*inLen/3) ] */
    in_: &[opus_int16],       /* I    Input signal [ inLen ] */
    inLen: opus_int32,        /* I    Number of input samples */
) {
    let mut nSamplesIn: opus_int32;
    let mut counter: opus_int32;
    let mut res_Q6: opus_int32;
    let mut buf = vec![0 as opus_int32; (RESAMPLER_MAX_BATCH_SIZE_IN + ORDER_FIR) as usize];
    let mut out_pos: usize = 0;
    let mut in_pos: usize = 0;
    let mut in_len = inLen;

    /* Copy buffered samples to start of buffer */
    buf[..ORDER_FIR as usize].copy_from_slice(&S[..ORDER_FIR as usize]);

    /* Iterate over blocks of frameSizeIn input samples */
    loop {
        nSamplesIn = silk_min(in_len, RESAMPLER_MAX_BATCH_SIZE_IN);

        /* Second-order AR filter (output in Q8) */
        silk_resampler_private_AR2(
            &mut S[ORDER_FIR as usize..],
            &mut buf[ORDER_FIR as usize..],
            &in_[in_pos..],
            &silk_Resampler_2_3_COEFS_LQ,
            nSamplesIn,
        );

        /* Interpolate filtered signal */
        let mut buf_ptr: usize = 0;
        counter = nSamplesIn;
        while counter > 2 {
            /* Inner product */
            res_Q6 = silk_SMULWB(buf[buf_ptr], silk_Resampler_2_3_COEFS_LQ[2] as opus_int32);
            res_Q6 = silk_SMLAWB(res_Q6, buf[buf_ptr + 1], silk_Resampler_2_3_COEFS_LQ[3] as opus_int32);
            res_Q6 = silk_SMLAWB(res_Q6, buf[buf_ptr + 2], silk_Resampler_2_3_COEFS_LQ[5] as opus_int32);
            res_Q6 = silk_SMLAWB(res_Q6, buf[buf_ptr + 3], silk_Resampler_2_3_COEFS_LQ[4] as opus_int32);

            /* Scale down, saturate and store in output array */
            out[out_pos] = silk_SAT16(silk_RSHIFT_ROUND(res_Q6, 6)) as opus_int16;
            out_pos += 1;

            res_Q6 = silk_SMULWB(buf[buf_ptr + 1], silk_Resampler_2_3_COEFS_LQ[4] as opus_int32);
            res_Q6 = silk_SMLAWB(res_Q6, buf[buf_ptr + 2], silk_Resampler_2_3_COEFS_LQ[5] as opus_int32);
            res_Q6 = silk_SMLAWB(res_Q6, buf[buf_ptr + 3], silk_Resampler_2_3_COEFS_LQ[3] as opus_int32);
            res_Q6 = silk_SMLAWB(res_Q6, buf[buf_ptr + 4], silk_Resampler_2_3_COEFS_LQ[2] as opus_int32);

            /* Scale down, saturate and store in output array */
            out[out_pos] = silk_SAT16(silk_RSHIFT_ROUND(res_Q6, 6)) as opus_int16;
            out_pos += 1;

            buf_ptr += 3;
            counter -= 3;
        }

        in_pos += nSamplesIn as usize;
        in_len -= nSamplesIn;

        if in_len > 0 {
            /* More iterations to do; copy last part of filtered signal to beginning of buffer */
            buf.copy_within(
                nSamplesIn as usize..nSamplesIn as usize + ORDER_FIR as usize,
                0,
            );
        } else {
            break;
        }
    }

    /* Copy last part of filtered signal to the state for the next call */
    S[..ORDER_FIR as usize].copy_from_slice(
        &buf[nSamplesIn as usize..nSamplesIn as usize + ORDER_FIR as usize],
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_input_gives_zero_output() {
        let mut s = [0i32; 6];
        let mut out = [0i16; 16];
        silk_resampler_down2_3(&mut s, &mut out, &[0i16; 9], 9);
        assert_eq!(out, [0i16; 16]);
        assert_eq!(s, [0i32; 6]);
    }
}

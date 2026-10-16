//! Mirror of `silk/resampler_private_IIR_FIR.c` from libopus 1.6.1, transliterated to safe Rust.
//!
//! Upsample using a combination of allpass-based 2x upsampling and FIR
//! interpolation.
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
use crate::silk::resampler_private_up2_hq::*;
use crate::silk::resampler_rom::*;
use crate::silk::sigproc_fix::*;
use crate::silk::structs::*;
use crate::types::*;

/// `silk_resampler_private_IIR_FIR_INTERPOL(out, buf, max_index_Q16, index_increment_Q16)`
/// returns the number of samples written to `out`.
fn silk_resampler_private_IIR_FIR_INTERPOL(
    out: &mut [opus_int16],
    buf: &[opus_int16],
    max_index_Q16: opus_int32,
    index_increment_Q16: opus_int32,
) -> usize {
    let mut res_Q15: opus_int32;
    let mut n: usize = 0;

    /* Interpolate upsampled signal and store in output array */
    let mut index_Q16: opus_int32 = 0;
    while index_Q16 < max_index_Q16 {
        let table_index = silk_SMULWB(index_Q16 & 0xFFFF, 12);
        let buf_ptr = silk_RSHIFT(index_Q16, 16) as usize;

        res_Q15 = silk_SMULBB(buf[buf_ptr] as opus_int32, silk_resampler_frac_FIR_12[table_index as usize][0] as opus_int32);
        res_Q15 = silk_SMLABB(res_Q15, buf[buf_ptr + 1] as opus_int32, silk_resampler_frac_FIR_12[table_index as usize][1] as opus_int32);
        res_Q15 = silk_SMLABB(res_Q15, buf[buf_ptr + 2] as opus_int32, silk_resampler_frac_FIR_12[table_index as usize][2] as opus_int32);
        res_Q15 = silk_SMLABB(res_Q15, buf[buf_ptr + 3] as opus_int32, silk_resampler_frac_FIR_12[table_index as usize][3] as opus_int32);
        res_Q15 = silk_SMLABB(res_Q15, buf[buf_ptr + 4] as opus_int32, silk_resampler_frac_FIR_12[(11 - table_index) as usize][3] as opus_int32);
        res_Q15 = silk_SMLABB(res_Q15, buf[buf_ptr + 5] as opus_int32, silk_resampler_frac_FIR_12[(11 - table_index) as usize][2] as opus_int32);
        res_Q15 = silk_SMLABB(res_Q15, buf[buf_ptr + 6] as opus_int32, silk_resampler_frac_FIR_12[(11 - table_index) as usize][1] as opus_int32);
        res_Q15 = silk_SMLABB(res_Q15, buf[buf_ptr + 7] as opus_int32, silk_resampler_frac_FIR_12[(11 - table_index) as usize][0] as opus_int32);
        out[n] = silk_SAT16(silk_RSHIFT_ROUND(res_Q15, 15)) as opus_int16;
        n += 1;
        index_Q16 += index_increment_Q16;
    }
    n
}

/// `silk_resampler_private_IIR_FIR(SS, out, in, inLen)`
pub fn silk_resampler_private_IIR_FIR(
    S: &mut silk_resampler_state_struct, /* I/O  Resampler state */
    out: &mut [opus_int16],              /* O    Output signal */
    in_: &[opus_int16],                  /* I    Input signal */
    inLen: opus_int32,                   /* I    Number of input samples */
) {
    let mut nSamplesIn: opus_int32;
    let mut max_index_Q16: opus_int32;
    let index_increment_Q16: opus_int32;
    let batch_size = S.batchSize;
    let mut buf = vec![0 as opus_int16; (2 * batch_size + RESAMPLER_ORDER_FIR_12) as usize];
    let mut out_pos: usize = 0;
    let mut in_pos: usize = 0;
    let mut in_len = inLen;

    /* Copy buffered samples to start of buffer */
    buf[..RESAMPLER_ORDER_FIR_12 as usize]
        .copy_from_slice(unsafe { &S.sFIR.i16[..RESAMPLER_ORDER_FIR_12 as usize] });

    /* Iterate over blocks of frameSizeIn input samples */
    index_increment_Q16 = S.invRatio_Q16;
    loop {
        nSamplesIn = silk_min(in_len, batch_size);

        /* Upsample 2x */
        silk_resampler_private_up2_HQ(
            &mut S.sIIR,
            &mut buf[RESAMPLER_ORDER_FIR_12 as usize..],
            &in_[in_pos..],
            nSamplesIn,
        );

        max_index_Q16 = silk_LSHIFT32(nSamplesIn, 16 + 1); /* + 1 because 2x upsampling */
        let written = silk_resampler_private_IIR_FIR_INTERPOL(
            &mut out[out_pos..],
            &buf,
            max_index_Q16,
            index_increment_Q16,
        );
        out_pos += written;

        in_pos += nSamplesIn as usize;
        in_len -= nSamplesIn;

        if in_len > 0 {
            /* More iterations to do; copy last part of filtered signal to beginning of buffer */
            buf.copy_within(
                (nSamplesIn << 1) as usize..(nSamplesIn << 1) as usize + RESAMPLER_ORDER_FIR_12 as usize,
                0,
            );
        } else {
            break;
        }
    }

    /* Copy last part of filtered signal to the state for the next call */
    unsafe {
        S.sFIR.i16[..RESAMPLER_ORDER_FIR_12 as usize].copy_from_slice(
            &buf[(nSamplesIn << 1) as usize
                ..(nSamplesIn << 1) as usize + RESAMPLER_ORDER_FIR_12 as usize],
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
        silk_resampler_init(&mut s, 8000, 48000, 0);
        let in_ = [0i16; 160];
        let mut out = [0i16; 2048];
        silk_resampler_private_IIR_FIR(&mut s, &mut out, &in_, 160);
        assert!(out.iter().all(|&v| v == 0));
    }
}

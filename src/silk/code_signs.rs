//! Mirror of `silk/code_signs.c` from libopus 1.6.1, transliterated to safe Rust.
//!
//! Entropy coding of the excitation signs.
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

use crate::celt::entcode::ec_ctx;
use crate::celt::entdec::ec_dec_icdf;
use crate::celt::entenc::ec_enc_icdf;
use crate::silk::define::*;
use crate::silk::macros::*;
use crate::silk::sigproc_fix::*;
use crate::silk::tables_pulses_per_block::*;
use crate::silk::typedef::*;
use crate::types::*;

/*#define silk_enc_map(a)                ((a) > 0 ? 1 : 0)*/
/*#define silk_dec_map(a)                ((a) > 0 ? 1 : -1)*/
/* shifting avoids if-statement */
/// `silk_enc_map(a)` — `( silk_RSHIFT( (a), 15 ) + 1 )`
#[inline(always)]
fn silk_enc_map(a: opus_int) -> opus_int {
    silk_RSHIFT(a, 15) + 1
}
/// `silk_dec_map(a)` — `( silk_LSHIFT( (a), 1 ) - 1 )`
#[inline(always)]
fn silk_dec_map(a: opus_int) -> opus_int {
    silk_LSHIFT(a, 1) - 1
}

/* Encodes signs of excitation */
/// `silk_encode_signs(psRangeEnc, pulses, length, signalType, quantOffsetType, sum_pulses)`
pub fn silk_encode_signs(
    psRangeEnc: &mut ec_ctx,                    /* I/O  compressor data structure */
    pulses: &[opus_int8],                       /* I    pulse signal */
    mut length: opus_int,                       /* I    length of input */
    signalType: opus_int,                       /* I    signal type */
    quantOffsetType: opus_int,                  /* I    quantization offset type */
    sum_pulses: &[opus_int],                    /* I    sum of absolute pulses per block */
) {
    let mut i: opus_int;
    let mut p: opus_int;
    let mut icdf = [0 as opus_uint8; 2];
    let mut q_ptr: usize = 0;

    icdf[1] = 0;
    i = silk_SMULBB(7, silk_ADD_LSHIFT(quantOffsetType, signalType, 1));
    let icdf_ptr = i as usize;
    length = silk_RSHIFT(length + SHELL_CODEC_FRAME_LENGTH / 2, LOG2_SHELL_CODEC_FRAME_LENGTH);
    for i in 0..length {
        p = sum_pulses[i as usize];
        if p > 0 {
            icdf[0] = silk_sign_iCDF[icdf_ptr + silk_min_int(p & 0x1F, 6) as usize];
            for j in 0..SHELL_CODEC_FRAME_LENGTH {
                if pulses[q_ptr + j as usize] != 0 {
                    ec_enc_icdf(
                        psRangeEnc,
                        silk_enc_map(pulses[q_ptr + j as usize] as opus_int),
                        &icdf,
                        8,
                    );
                }
            }
        }
        q_ptr += SHELL_CODEC_FRAME_LENGTH as usize;
    }
}

/* Decodes signs of excitation */
/// `silk_decode_signs(psRangeDec, pulses, length, signalType, quantOffsetType, sum_pulses)`
pub fn silk_decode_signs(
    psRangeDec: &mut ec_ctx,                    /* I/O  compressor data structure */
    pulses: &mut [opus_int16],                  /* I/O  pulse signal */
    mut length: opus_int,                       /* I    length of input */
    signalType: opus_int,                       /* I    signal type */
    quantOffsetType: opus_int,                  /* I    quantization offset type */
    sum_pulses: &[opus_int],                    /* I    sum of absolute pulses per block */
) {
    let mut i: opus_int;
    let mut p: opus_int;
    let mut icdf = [0 as opus_uint8; 2];
    let mut q_ptr: usize = 0;

    icdf[1] = 0;
    i = silk_SMULBB(7, silk_ADD_LSHIFT(quantOffsetType, signalType, 1));
    let icdf_ptr = i as usize;
    length = silk_RSHIFT(length + SHELL_CODEC_FRAME_LENGTH / 2, LOG2_SHELL_CODEC_FRAME_LENGTH);
    for i in 0..length {
        p = sum_pulses[i as usize];
        if p > 0 {
            icdf[0] = silk_sign_iCDF[icdf_ptr + silk_min_int(p & 0x1F, 6) as usize];
            for j in 0..SHELL_CODEC_FRAME_LENGTH {
                if pulses[q_ptr + j as usize] > 0 {
                    /* attach sign */
                    /* implementation with shift, subtraction, multiplication */
                    pulses[q_ptr + j as usize] = (pulses[q_ptr + j as usize] as opus_int
                        * silk_dec_map(ec_dec_icdf(psRangeDec, &icdf, 8)))
                        as opus_int16;
                }
            }
        }
        q_ptr += SHELL_CODEC_FRAME_LENGTH as usize;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::celt::entdec::ec_dec_init;
    use crate::celt::entenc::{ec_enc_done, ec_enc_init};

    #[test]
    fn sign_round_trip() {
        let pulses: [i8; 16] = [3, -2, 0, 1, 0, 0, -4, 0, 2, 0, 0, -1, 0, 5, 0, -6];
        let mut sums = [0i32; 4];
        for (i, &p) in pulses.iter().enumerate() {
            sums[i / 16] += (p as i32).abs();
        }

        let mut enc = ec_enc_init(256);
        silk_encode_signs(&mut enc, &pulses, 16, 0, 0, &sums);
        ec_enc_done(&mut enc);

        let mut dec = ec_dec_init(&enc.buf);
        let mut mag = [0i16; 16];
        for i in 0..16 {
            mag[i] = (pulses[i] as i32).abs() as i16;
        }
        silk_decode_signs(&mut dec, &mut mag, 16, 0, 0, &sums);

        let want: [i16; 16] = [3, -2, 0, 1, 0, 0, -4, 0, 2, 0, 0, -1, 0, 5, 0, -6];
        assert_eq!(mag, want);
    }
}

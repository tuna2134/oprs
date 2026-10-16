//! Mirror of `silk/shell_coder.c` from libopus 1.6.1, transliterated to safe Rust.
//!
//! Shell coder; pulse-subframe length is hardcoded.
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
use crate::silk::tables_pulses_per_block::*;
use crate::silk::typedef::*;
use crate::types::*;

fn combine_pulses(
    out: &mut [opus_int],  /* O    combined pulses vector [len] */
    in_: &[opus_int],      /* I    input vector       [2 * len] */
    len: opus_int,         /* I    number of OUTPUT samples     */
) {
    for k in 0..len {
        out[k as usize] = in_[(2 * k) as usize] + in_[(2 * k + 1) as usize];
    }
}

fn encode_split(
    psRangeEnc: &mut ec_ctx,     /* I/O  compressor data structure */
    p_child1: opus_int,          /* I    pulse amplitude of first child subframe */
    p: opus_int,                 /* I    pulse amplitude of current subframe */
    shell_table: &[opus_uint8],  /* I    table of shell cdfs */
) {
    if p > 0 {
        ec_enc_icdf(
            psRangeEnc,
            p_child1,
            &shell_table[silk_shell_code_table_offsets[p as usize] as usize..],
            8,
        );
    }
}

/// The two C out-parameters `p_child1` / `p_child2` are returned as a tuple,
/// so that two elements of the same output array can be written.
fn decode_split(
    psRangeDec: &mut ec_ctx,     /* I/O  compressor data structure */
    p: opus_int,                 /* I    pulse amplitude of current subframe */
    shell_table: &[opus_uint8],  /* I    table of shell cdfs */
) -> (opus_int16, opus_int16) {
    if p > 0 {
        let p_child1 = ec_dec_icdf(
            psRangeDec,
            &shell_table[silk_shell_code_table_offsets[p as usize] as usize..],
            8,
        ) as opus_int16;
        (p_child1, (p - p_child1 as opus_int) as opus_int16)
    } else {
        (0, 0)
    }
}

/* Shell encoder, operates on one shell code frame of 16 pulses */
/// `silk_shell_encoder(psRangeEnc, pulses0)`
pub fn silk_shell_encoder(
    psRangeEnc: &mut ec_ctx,      /* I/O  compressor data structure */
    pulses0: &[opus_int],         /* I    data: nonnegative pulse amplitudes */
) {
    let mut pulses1 = [0 as opus_int; 8];
    let mut pulses2 = [0 as opus_int; 4];
    let mut pulses3 = [0 as opus_int; 2];
    let mut pulses4 = [0 as opus_int; 1];

    /* this function operates on one shell code frame of 16 pulses */
    silk_assert!(SHELL_CODEC_FRAME_LENGTH == 16);

    /* tree representation per pulse-subframe */
    combine_pulses(&mut pulses1, pulses0, 8);
    combine_pulses(&mut pulses2, &pulses1, 4);
    combine_pulses(&mut pulses3, &pulses2, 2);
    combine_pulses(&mut pulses4, &pulses3, 1);

    encode_split(psRangeEnc, pulses3[0], pulses4[0], &silk_shell_code_table3);

    encode_split(psRangeEnc, pulses2[0], pulses3[0], &silk_shell_code_table2);

    encode_split(psRangeEnc, pulses1[0], pulses2[0], &silk_shell_code_table1);
    encode_split(psRangeEnc, pulses0[0], pulses1[0], &silk_shell_code_table0);
    encode_split(psRangeEnc, pulses0[2], pulses1[1], &silk_shell_code_table0);

    encode_split(psRangeEnc, pulses1[2], pulses2[1], &silk_shell_code_table1);
    encode_split(psRangeEnc, pulses0[4], pulses1[2], &silk_shell_code_table0);
    encode_split(psRangeEnc, pulses0[6], pulses1[3], &silk_shell_code_table0);

    encode_split(psRangeEnc, pulses2[2], pulses3[1], &silk_shell_code_table2);

    encode_split(psRangeEnc, pulses1[4], pulses2[2], &silk_shell_code_table1);
    encode_split(psRangeEnc, pulses0[8], pulses1[4], &silk_shell_code_table0);
    encode_split(psRangeEnc, pulses0[10], pulses1[5], &silk_shell_code_table0);

    encode_split(psRangeEnc, pulses1[6], pulses2[3], &silk_shell_code_table1);
    encode_split(psRangeEnc, pulses0[12], pulses1[6], &silk_shell_code_table0);
    encode_split(psRangeEnc, pulses0[14], pulses1[7], &silk_shell_code_table0);
}

/* Shell decoder, operates on one shell code frame of 16 pulses */
/// `silk_shell_decoder(pulses0, psRangeDec, pulses4)`
pub fn silk_shell_decoder(
    pulses0: &mut [opus_int16],   /* O    data: nonnegative pulse amplitudes */
    psRangeDec: &mut ec_ctx,      /* I/O  compressor data structure */
    pulses4: opus_int,            /* I    number of pulses per pulse-subframe */
) {
    let mut pulses3 = [0 as opus_int16; 2];
    let mut pulses2 = [0 as opus_int16; 4];
    let mut pulses1 = [0 as opus_int16; 8];

    /* this function operates on one shell code frame of 16 pulses */
    silk_assert!(SHELL_CODEC_FRAME_LENGTH == 16);

    let (a, b) = decode_split(psRangeDec, pulses4, &silk_shell_code_table3);
    pulses3[0] = a;
    pulses3[1] = b;

    let (a, b) = decode_split(psRangeDec, pulses3[0] as opus_int, &silk_shell_code_table2);
    pulses2[0] = a;
    pulses2[1] = b;

    let (a, b) = decode_split(psRangeDec, pulses2[0] as opus_int, &silk_shell_code_table1);
    pulses1[0] = a;
    pulses1[1] = b;
    let (a, b) = decode_split(psRangeDec, pulses1[0] as opus_int, &silk_shell_code_table0);
    pulses0[0] = a;
    pulses0[1] = b;
    let (a, b) = decode_split(psRangeDec, pulses1[1] as opus_int, &silk_shell_code_table0);
    pulses0[2] = a;
    pulses0[3] = b;

    let (a, b) = decode_split(psRangeDec, pulses2[1] as opus_int, &silk_shell_code_table1);
    pulses1[2] = a;
    pulses1[3] = b;
    let (a, b) = decode_split(psRangeDec, pulses1[2] as opus_int, &silk_shell_code_table0);
    pulses0[4] = a;
    pulses0[5] = b;
    let (a, b) = decode_split(psRangeDec, pulses1[3] as opus_int, &silk_shell_code_table0);
    pulses0[6] = a;
    pulses0[7] = b;

    let (a, b) = decode_split(psRangeDec, pulses3[1] as opus_int, &silk_shell_code_table2);
    pulses2[2] = a;
    pulses2[3] = b;

    let (a, b) = decode_split(psRangeDec, pulses2[2] as opus_int, &silk_shell_code_table1);
    pulses1[4] = a;
    pulses1[5] = b;
    let (a, b) = decode_split(psRangeDec, pulses1[4] as opus_int, &silk_shell_code_table0);
    pulses0[8] = a;
    pulses0[9] = b;
    let (a, b) = decode_split(psRangeDec, pulses1[5] as opus_int, &silk_shell_code_table0);
    pulses0[10] = a;
    pulses0[11] = b;

    let (a, b) = decode_split(psRangeDec, pulses2[3] as opus_int, &silk_shell_code_table1);
    pulses1[6] = a;
    pulses1[7] = b;
    let (a, b) = decode_split(psRangeDec, pulses1[6] as opus_int, &silk_shell_code_table0);
    pulses0[12] = a;
    pulses0[13] = b;
    let (a, b) = decode_split(psRangeDec, pulses1[7] as opus_int, &silk_shell_code_table0);
    pulses0[14] = a;
    pulses0[15] = b;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::celt::entdec::ec_dec_init;
    use crate::celt::entenc::{ec_enc_done, ec_enc_init};

    #[test]
    fn round_trip() {
        let pulses = [2i32, 0, 1, 0, 0, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        let mut enc = ec_enc_init(256);
        silk_shell_encoder(&mut enc, &pulses);
        ec_enc_done(&mut enc);

        let mut dec = ec_dec_init(&enc.buf);
        let mut out = [0i16; 16];
        silk_shell_decoder(&mut out, &mut dec, 6);
        assert_eq!(out, [2, 0, 1, 0, 0, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn all_pulses_in_one_slot() {
        let mut pulses = [0i32; 16];
        pulses[7] = 16;
        let mut enc = ec_enc_init(256);
        silk_shell_encoder(&mut enc, &pulses);
        ec_enc_done(&mut enc);

        let mut dec = ec_dec_init(&enc.buf);
        let mut out = [0i16; 16];
        silk_shell_decoder(&mut out, &mut dec, 16);
        assert_eq!(out[7], 16);
        assert_eq!(out.iter().map(|&v| v as i32).sum::<i32>(), 16);
    }
}

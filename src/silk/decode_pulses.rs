//! Transliteration of `silk/decode_pulses.c` from libopus 1.6.1 to safe Rust.
//!
//! Original C file copyright notice (this is a derived work):
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
use crate::celt::entcode::ec_ctx;
use crate::celt::entdec::ec_dec_icdf;
use crate::silk::code_signs::silk_decode_signs;
use crate::silk::define::*;
use crate::silk::shell_coder::silk_shell_decoder;
use crate::silk::sigproc_fix::*;
use crate::silk::tables_other::silk_lsb_iCDF;
use crate::silk::tables_pulses_per_block::*;
use crate::silk::typedef::*;
use crate::types::*;

/*********************************************/
/* Decode quantization indices of excitation */
/*********************************************/
/// `silk_decode_pulses(psRangeDec, pulses, signalType, quantOffsetType, frame_length)`
pub fn silk_decode_pulses(
    psRangeDec: &mut ec_ctx,   /* I/O  Compressor data structure */
    pulses: &mut [opus_int16], /* O    Excitation signal         */
    signalType: opus_int,      /* I    Sigtype                   */
    quantOffsetType: opus_int, /* I    quantOffsetType           */
    frame_length: opus_int,    /* I    Frame length              */
) {
    let mut i: opus_int;
    let mut j: opus_int;
    let mut k: opus_int;
    let mut iter: opus_int;
    let mut abs_q: opus_int;
    let mut nLS: opus_int;
    let mut RateLevelIndex: opus_int;
    let mut sum_pulses = [0 as opus_int; MAX_NB_SHELL_BLOCKS as usize];
    let mut nLshifts = [0 as opus_int; MAX_NB_SHELL_BLOCKS as usize];

    /*********************/
    /* Decode rate level */
    /*********************/
    RateLevelIndex = ec_dec_icdf(psRangeDec, &silk_rate_levels_iCDF[(signalType >> 1) as usize], 8);

    /* Calculate number of shell blocks */
    silk_assert!(1 << LOG2_SHELL_CODEC_FRAME_LENGTH == SHELL_CODEC_FRAME_LENGTH);
    iter = silk_RSHIFT(frame_length, LOG2_SHELL_CODEC_FRAME_LENGTH);
    if iter * SHELL_CODEC_FRAME_LENGTH < frame_length {
        celt_assert!(frame_length == 12 * 10); /* Make sure only happens for 10 ms @ 12 kHz */
        iter += 1;
    }

    /***************************************************/
    /* Sum-Weighted-Pulses Decoding                    */
    /***************************************************/
    for i in 0..iter {
        nLshifts[i as usize] = 0;
        sum_pulses[i as usize] = ec_dec_icdf(
            psRangeDec,
            &silk_pulses_per_block_iCDF[RateLevelIndex as usize],
            8,
        );

        /* LSB indication */
        while sum_pulses[i as usize] == SILK_MAX_PULSES + 1 {
            nLshifts[i as usize] += 1;
            /* When we've already got 10 LSBs, we shift the table to not allow (SILK_MAX_PULSES + 1) */
            sum_pulses[i as usize] = ec_dec_icdf(
                psRangeDec,
                &silk_pulses_per_block_iCDF[(N_RATE_LEVELS - 1) as usize]
                    [(nLshifts[i as usize] == 10) as usize..],
                8,
            );
        }
    }

    /***************************************************/
    /* Shell decoding                                  */
    /***************************************************/
    for i in 0..iter {
        if sum_pulses[i as usize] > 0 {
            silk_shell_decoder(
                &mut pulses[(i * SHELL_CODEC_FRAME_LENGTH) as usize..],
                psRangeDec,
                sum_pulses[i as usize],
            );
        } else {
            pulses[(i * SHELL_CODEC_FRAME_LENGTH) as usize
                ..((i + 1) * SHELL_CODEC_FRAME_LENGTH) as usize]
                .fill(0);
        }
    }

    /***************************************************/
    /* LSB Decoding                                    */
    /***************************************************/
    for i in 0..iter {
        if nLshifts[i as usize] > 0 {
            nLS = nLshifts[i as usize];
            let base = (i * SHELL_CODEC_FRAME_LENGTH) as usize;
            for k in 0..SHELL_CODEC_FRAME_LENGTH {
                abs_q = pulses[base + k as usize] as opus_int;
                for _j in 0..nLS {
                    abs_q = silk_LSHIFT(abs_q, 1);
                    abs_q += ec_dec_icdf(psRangeDec, &silk_lsb_iCDF, 8);
                }
                pulses[base + k as usize] = abs_q as opus_int16;
            }
            /* Mark the number of pulses non-zero for sign decoding. */
            sum_pulses[i as usize] |= nLS << 5;
        }
    }

    /****************************************/
    /* Decode and add signs to pulse signal */
    /****************************************/
    silk_decode_signs(
        psRangeDec,
        pulses,
        frame_length,
        signalType,
        quantOffsetType,
        &sum_pulses,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::celt::entdec::ec_dec_init;

    #[test]
    fn decodes_a_silence_payload() {
        let buf = [0u8; 64];
        let mut dec = ec_dec_init(&buf);
        let mut pulses = [0 as opus_int16; 320];
        silk_decode_pulses(&mut dec, &mut pulses, 0, 0, 320);
        /* A zero payload decodes to the first rate level; pulses stay bounded. */
        assert!(pulses.iter().all(|&p| p >= 0 && p <= 1024));
    }

    #[test]
    fn decodes_the_12khz_10ms_odd_frame_length() {
        let buf = [0u8; 64];
        let mut dec = ec_dec_init(&buf);
        let mut pulses = [0 as opus_int16; 128];
        silk_decode_pulses(&mut dec, &mut pulses, 1, 1, 120);
        assert!(pulses.iter().all(|&p| p >= 0 && p <= 1024));
    }
}

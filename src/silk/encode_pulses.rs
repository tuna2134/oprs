//! Mirror of `silk/encode_pulses.c` from libopus 1.6.1, transliterated to safe Rust.
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
use crate::celt::entenc::*;
use crate::celt::entcode::ec_ctx;
use crate::silk::code_signs::*;
use crate::silk::define::*;
use crate::silk::shell_coder::*;
use crate::silk::sigproc_fix::*;
use crate::silk::tables_other::*;
use crate::silk::tables_pulses_per_block::*;
use crate::silk::typedef::silk_assert;
use crate::silk::typedef::*;
use crate::types::*;

/*********************************************/
/* Encode quantization indices of excitation */
/*********************************************/

fn combine_and_check( /* return ok                           */
    pulses_comb: &mut [opus_int], /* O                                   */
    pulses_in: &[opus_int],       /* I                                   */
    max_pulses: opus_int,         /* I    max value for sum of pulses    */
    len: opus_int,                /* I    number of output values        */
) -> opus_int {
    for k in 0..len {
        let sum = pulses_in[(2 * k) as usize] + pulses_in[(2 * k + 1) as usize];
        if sum > max_pulses {
            return 1;
        }
        pulses_comb[k as usize] = sum;
    }

    0
}

/* Encode quantization indices of excitation */
pub fn silk_encode_pulses(
    psRangeEnc: &mut ec_ctx,   /* I/O  compressor data structure                   */
    signalType: opus_int,      /* I    Signal type                                 */
    quantOffsetType: opus_int, /* I    quantOffsetType                             */
    pulses: &mut [opus_int8],  /* I    quantization indices                        */
    frame_length: opus_int,    /* I    Frame length                                */
) {
    let mut pulses_comb = [0 as opus_int; 8];

    /****************************/
    /* Prepare for shell coding */
    /****************************/
    /* Calculate number of shell blocks */
    silk_assert!(1 << LOG2_SHELL_CODEC_FRAME_LENGTH == SHELL_CODEC_FRAME_LENGTH);
    let mut iter = silk_RSHIFT(frame_length, LOG2_SHELL_CODEC_FRAME_LENGTH);
    if iter * SHELL_CODEC_FRAME_LENGTH < frame_length {
        celt_assert!(frame_length == 12 * 10); /* Make sure only happens for 10 ms @ 12 kHz */
        iter += 1;
        for i in 0..SHELL_CODEC_FRAME_LENGTH as usize {
            pulses[frame_length as usize + i] = 0;
        }
    }

    /* Take the absolute value of the pulses */
    let mut abs_pulses = vec![0 as opus_int; (iter * SHELL_CODEC_FRAME_LENGTH) as usize];
    silk_assert!((SHELL_CODEC_FRAME_LENGTH & 3) == 0);
    let mut i: opus_int = 0;
    while i < iter * SHELL_CODEC_FRAME_LENGTH {
        abs_pulses[(i + 0) as usize] = silk_abs(pulses[(i + 0) as usize] as opus_int32) as opus_int;
        abs_pulses[(i + 1) as usize] = silk_abs(pulses[(i + 1) as usize] as opus_int32) as opus_int;
        abs_pulses[(i + 2) as usize] = silk_abs(pulses[(i + 2) as usize] as opus_int32) as opus_int;
        abs_pulses[(i + 3) as usize] = silk_abs(pulses[(i + 3) as usize] as opus_int32) as opus_int;
        i += 4;
    }

    /* Calc sum pulses per shell code frame */
    let mut sum_pulses = vec![0 as opus_int; iter as usize];
    let mut nRshifts = vec![0 as opus_int; iter as usize];
    let mut abs_pulses_ptr: opus_int = 0;
    for i in 0..iter {
        nRshifts[i as usize] = 0;

        loop {
            /* 1+1 -> 2 */
            let mut scale_down = combine_and_check(
                &mut pulses_comb,
                &abs_pulses[abs_pulses_ptr as usize..],
                silk_max_pulses_table[0] as opus_int,
                8,
            );
            /* 2+2 -> 4 */
                        let __comb_src = pulses_comb.clone();
            scale_down += combine_and_check(
                &mut pulses_comb,
                &__comb_src,
                silk_max_pulses_table[1] as opus_int,
                4,
            );
            /* 4+4 -> 8 */
                        let __comb_src = pulses_comb.clone();
            scale_down += combine_and_check(
                &mut pulses_comb,
                &__comb_src,
                silk_max_pulses_table[2] as opus_int,
                2,
            );
            /* 8+8 -> 16 */
            scale_down += combine_and_check_unit(
                &mut sum_pulses[i as usize],
                &mut pulses_comb.clone(),
                silk_max_pulses_table[3] as opus_int,
            );

            if scale_down != 0 {
                /* We need to downscale the quantization signal */
                nRshifts[i as usize] += 1;
                for k in 0..SHELL_CODEC_FRAME_LENGTH as usize {
                    abs_pulses[abs_pulses_ptr as usize + k] =
                        silk_RSHIFT(abs_pulses[abs_pulses_ptr as usize + k], 1);
                }
            } else {
                /* Jump out of while(1) loop and go to next shell coding frame */
                break;
            }
        }
        abs_pulses_ptr += SHELL_CODEC_FRAME_LENGTH;
    }

    /**************/
    /* Rate level */
    /**************/
    /* find rate level that leads to fewest bits for coding of pulses per block info */
    let mut minSumBits_Q5 = silk_int32_MAX;
    let mut RateLevelIndex: opus_int = 0;
    for k in 0..N_RATE_LEVELS - 1 {
        let nBits_ptr = &silk_pulses_per_block_BITS_Q5[k as usize];
        let mut sumBits_Q5 =
            silk_rate_levels_BITS_Q5[(signalType >> 1) as usize][k as usize] as opus_int32;
        for i in 0..iter {
            if nRshifts[i as usize] > 0 {
                sumBits_Q5 += nBits_ptr[(SILK_MAX_PULSES + 1) as usize] as opus_int32;
            } else {
                sumBits_Q5 += nBits_ptr[sum_pulses[i as usize] as usize] as opus_int32;
            }
        }
        if sumBits_Q5 < minSumBits_Q5 {
            minSumBits_Q5 = sumBits_Q5;
            RateLevelIndex = k;
        }
    }
    ec_enc_icdf(
        psRangeEnc,
        RateLevelIndex,
        &silk_rate_levels_iCDF[(signalType >> 1) as usize],
        8,
    );

    /***************************************************/
    /* Sum-Weighted-Pulses Encoding                    */
    /***************************************************/
    let cdf_ptr = &silk_pulses_per_block_iCDF[RateLevelIndex as usize];
    for i in 0..iter {
        if nRshifts[i as usize] == 0 {
            ec_enc_icdf(psRangeEnc, sum_pulses[i as usize], cdf_ptr, 8);
        } else {
            ec_enc_icdf(psRangeEnc, SILK_MAX_PULSES + 1, cdf_ptr, 8);
            for _k in 0..nRshifts[i as usize] - 1 {
                ec_enc_icdf(
                    psRangeEnc,
                    SILK_MAX_PULSES + 1,
                    &silk_pulses_per_block_iCDF[(N_RATE_LEVELS - 1) as usize],
                    8,
                );
            }
            ec_enc_icdf(
                psRangeEnc,
                sum_pulses[i as usize],
                &silk_pulses_per_block_iCDF[(N_RATE_LEVELS - 1) as usize],
                8,
            );
        }
    }

    /******************/
    /* Shell Encoding */
    /******************/
    for i in 0..iter {
        if sum_pulses[i as usize] > 0 {
            silk_shell_encoder(
                psRangeEnc,
                &abs_pulses[(i * SHELL_CODEC_FRAME_LENGTH) as usize..],
            );
        }
    }

    /****************/
    /* LSB Encoding */
    /****************/
    for i in 0..iter {
        if nRshifts[i as usize] > 0 {
            let pulses_ptr = &pulses[(i * SHELL_CODEC_FRAME_LENGTH) as usize..];
            let nLS = nRshifts[i as usize] - 1;
            for k in 0..SHELL_CODEC_FRAME_LENGTH {
                let abs_q = silk_abs(pulses_ptr[k as usize] as opus_int32) as opus_int8 as opus_int32;
                let mut j = nLS;
                while j > 0 {
                    let bit = silk_RSHIFT(abs_q, j) & 1;
                    ec_enc_icdf(psRangeEnc, bit, &silk_lsb_iCDF, 8);
                    j -= 1;
                }
                let bit = abs_q & 1;
                ec_enc_icdf(psRangeEnc, bit, &silk_lsb_iCDF, 8);
            }
        }
    }

    /****************/
    /* Encode signs */
    /****************/
    silk_encode_signs(
        psRangeEnc,
        pulses,
        frame_length,
        signalType,
        quantOffsetType,
        &sum_pulses,
    );
}

/// `combine_and_check` with a single output value (`&sum_pulses[i]`, `len == 1`).
fn combine_and_check_unit(
    out: &mut opus_int,
    pulses_in: &mut [opus_int],
    max_pulses: opus_int,
) -> opus_int {
    let sum = pulses_in[0] + pulses_in[1];
    if sum > max_pulses {
        return 1;
    }
    *out = sum;
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combine_flags_overflow() {
        let mut comb = [0i32; 8];
        let inp = [1i32, 2, 3, 4];
        assert_eq!(combine_and_check(&mut comb, &inp, 3, 2), 1);
        assert_eq!(comb[0], 3);
        let inp = [1i32, 2, 3, 4, 5, 6, 7, 8];
        assert_eq!(combine_and_check(&mut comb, &inp, 100, 4), 0);
        assert_eq!(&comb[..4], &[3, 7, 11, 15]);
    }

    #[test]
    fn combine_unit_limits() {
        let mut out = 0;
        assert_eq!(combine_and_check_unit(&mut out, &mut [5, 5], 10), 0);
        assert_eq!(out, 10);
        assert_eq!(combine_and_check_unit(&mut out, &mut [5, 6], 10), 1);
    }
}

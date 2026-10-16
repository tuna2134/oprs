//! Mirror of `silk/gain_quant.c` from libopus 1.6.1, transliterated to safe Rust.
//!
//! Gain scalar quantization (with hysteresis) and dequantization, uniform on a
//! log scale, plus the unique identifier of a gain-index vector.
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

use crate::silk::define::*;
use crate::silk::lin2log::*;
use crate::silk::log2lin::*;
use crate::silk::macros::*;
use crate::silk::sigproc_fix::*;
use crate::silk::typedef::*;
use crate::types::*;

const OFFSET: opus_int32 = (MIN_QGAIN_DB * 128) / 6 + 16 * 128;
const SCALE_Q16: opus_int32 =
    (65536 * (N_LEVELS_QGAIN - 1)) / (((MAX_QGAIN_DB - MIN_QGAIN_DB) * 128) / 6);
const INV_SCALE_Q16: opus_int32 =
    (65536 * (((MAX_QGAIN_DB - MIN_QGAIN_DB) * 128) / 6)) / (N_LEVELS_QGAIN - 1);

/// `silk_gains_quant(ind, gain_Q16, prev_ind, conditional, nb_subfr)`
pub fn silk_gains_quant(
    ind: &mut [opus_int8],
    gain_Q16: &mut [opus_int32],
    prev_ind: &mut opus_int8,
    conditional: opus_int,
    nb_subfr: opus_int,
) {
    let mut double_step_size_threshold: opus_int;

    for k in 0..nb_subfr {
        /* Convert to log scale, scale, floor() */
        ind[k as usize] = silk_SMULWB(SCALE_Q16, silk_lin2log(gain_Q16[k as usize]) - OFFSET)
            as opus_int8;

        /* Round towards previous quantized gain (hysteresis) */
        if (ind[k as usize] as opus_int) < (*prev_ind as opus_int) {
            ind[k as usize] += 1;
        }
        ind[k as usize] = silk_LIMIT(ind[k as usize] as opus_int, 0, N_LEVELS_QGAIN - 1) as opus_int8;

        /* Compute delta indices and limit */
        if k == 0 && conditional == 0 {
            /* Full index */
            ind[k as usize] = silk_LIMIT(
                ind[k as usize] as opus_int,
                *prev_ind as opus_int + MIN_DELTA_GAIN_QUANT,
                N_LEVELS_QGAIN - 1,
            ) as opus_int8;
            *prev_ind = ind[k as usize];
        } else {
            /* Delta index */
            ind[k as usize] = (ind[k as usize] as opus_int - *prev_ind as opus_int) as opus_int8;

            /* Double the quantization step size for large gain increases, so that the max gain level can be reached */
            double_step_size_threshold =
                2 * MAX_DELTA_GAIN_QUANT - N_LEVELS_QGAIN + *prev_ind as opus_int;
            if (ind[k as usize] as opus_int) > double_step_size_threshold {
                ind[k as usize] = (double_step_size_threshold
                    + silk_RSHIFT(
                        ind[k as usize] as opus_int - double_step_size_threshold + 1,
                        1,
                    )) as opus_int8;
            }

            ind[k as usize] = silk_LIMIT(
                ind[k as usize] as opus_int,
                MIN_DELTA_GAIN_QUANT,
                MAX_DELTA_GAIN_QUANT,
            ) as opus_int8;

            /* Accumulate deltas */
            if (ind[k as usize] as opus_int) > double_step_size_threshold {
                *prev_ind = (*prev_ind as opus_int
                    + silk_LSHIFT(ind[k as usize] as opus_int, 1)
                    - double_step_size_threshold) as opus_int8;
                *prev_ind = silk_min_int(*prev_ind as opus_int, N_LEVELS_QGAIN - 1) as opus_int8;
            } else {
                *prev_ind = (*prev_ind as opus_int + ind[k as usize] as opus_int) as opus_int8;
            }

            /* Shift to make non-negative */
            ind[k as usize] -= MIN_DELTA_GAIN_QUANT as opus_int8;
        }

        /* Scale and convert to linear scale */
        gain_Q16[k as usize] = silk_log2lin(silk_min_32(
            silk_SMULWB(INV_SCALE_Q16, *prev_ind as opus_int32) + OFFSET,
            3967,
        )); /* 3967 = 31 in Q7 */
    }
}

/// `silk_gains_dequant(gain_Q16, ind, prev_ind, conditional, nb_subfr)`
pub fn silk_gains_dequant(
    gain_Q16: &mut [opus_int32],
    ind: &[opus_int8],
    prev_ind: &mut opus_int8,
    conditional: opus_int,
    nb_subfr: opus_int,
) {
    let mut ind_tmp: opus_int;
    let mut double_step_size_threshold: opus_int;

    for k in 0..nb_subfr {
        if k == 0 && conditional == 0 {
            /* Gain index is not allowed to go down more than 16 steps (~21.8 dB) */
            *prev_ind = silk_max_int(ind[0] as opus_int, *prev_ind as opus_int - 16) as opus_int8;
        } else {
            /* Delta index */
            ind_tmp = ind[k as usize] as opus_int + MIN_DELTA_GAIN_QUANT;

            /* Accumulate deltas */
            double_step_size_threshold =
                2 * MAX_DELTA_GAIN_QUANT - N_LEVELS_QGAIN + *prev_ind as opus_int;
            if ind_tmp > double_step_size_threshold {
                *prev_ind = (*prev_ind as opus_int + silk_LSHIFT(ind_tmp, 1)
                    - double_step_size_threshold) as opus_int8;
            } else {
                *prev_ind = (*prev_ind as opus_int + ind_tmp) as opus_int8;
            }
        }
        *prev_ind = silk_LIMIT(*prev_ind as opus_int, 0, N_LEVELS_QGAIN - 1) as opus_int8;

        /* Scale and convert to linear scale */
        gain_Q16[k as usize] = silk_log2lin(silk_min_32(
            silk_SMULWB(INV_SCALE_Q16, *prev_ind as opus_int32) + OFFSET,
            3967,
        )); /* 3967 = 31 in Q7 */
    }
}

/// `silk_gains_ID(ind, nb_subfr)` — unique identifier of a gain indices vector.
pub fn silk_gains_ID(ind: &[opus_int8], nb_subfr: opus_int) -> opus_int32 {
    let mut gainsID: opus_int32;

    gainsID = 0;
    for k in 0..nb_subfr {
        gainsID = silk_ADD_LSHIFT32(ind[k as usize] as opus_int32, gainsID, 8);
    }

    gainsID
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quant_then_dequant_is_consistent() {
        let mut ind = [0i8; 4];
        let mut gain = [1 << 20, 1 << 22, 1 << 24, 1 << 26];
        let mut prev: i8 = 0;
        silk_gains_quant(&mut ind, &mut gain, &mut prev, 0, 4);

        let mut gain2 = [0i32; 4];
        let mut prev2: i8 = 0;
        silk_gains_dequant(&mut gain2, &ind, &mut prev2, 0, 4);

        assert_eq!(prev, prev2);
        assert_eq!(gain, gain2);
    }

    #[test]
    fn id_is_base_256() {
        let ind = [1i8, 2, 3];
        /* gainsID = ind[k] + (gainsID << 8), i.e. the last index is the most significant. */
        assert_eq!(silk_gains_ID(&ind, 3), 3 + (2 << 8) + (1 << 16));
    }
}

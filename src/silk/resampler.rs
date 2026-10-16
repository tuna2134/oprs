//! Mirror of `silk/resampler.c` from libopus 1.6.1, transliterated to safe Rust.
//!
//! Resampler: convert from one sampling rate to another.  Input and output
//! sampling rate are at most 48000 Hz.
//!
//! Matrix of resampling methods used:
//!                                 Fs_out (kHz)
//!                        8      12     16     24     48
//!
//!               8        C      UF     U      UF     UF
//!              12        AF     C      UF     U      UF
//! Fs_in (kHz)  16        D      AF     C      UF     UF
//!              24        AF     D      AF     C      U
//!              48        AF     AF     AF     D      C
//!
//! C   -> Copy (no resampling)
//! D   -> Allpass-based 2x downsampling
//! U   -> Allpass-based 2x upsampling
//! UF  -> Allpass-based 2x upsampling followed by FIR interpolation
//! AF  -> AR2 filter followed by FIR interpolation
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
use crate::silk::resampler_private_down_fir::*;
use crate::silk::resampler_private_iir_fir::*;
use crate::silk::resampler_private_up2_hq::*;
use crate::silk::resampler_rom::*;
use crate::silk::sigproc_fix::*;
use crate::silk::structs::*;
use crate::silk::typedef::*;
use crate::types::*;

/* Tables with delay compensation values to equalize total delay for different modes */
static delay_matrix_enc: [[opus_int8; 3]; 6] = [
    /* in  \ out  8  12  16 */
    /*  8 */ [6, 0, 3],
    /* 12 */ [0, 7, 3],
    /* 16 */ [0, 1, 10],
    /* 24 */ [0, 2, 6],
    /* 48 */ [18, 10, 12],
    /* 96 */ [0, 0, 44],
];

static delay_matrix_dec: [[opus_int8; 6]; 3] = [
    /* in  \ out  8  12  16  24  48  96*/
    /*  8 */ [4, 0, 2, 0, 0, 0],
    /* 12 */ [0, 9, 4, 7, 4, 4],
    /* 16 */ [0, 3, 12, 7, 7, 7],
];

/* Simple way to make [8000, 12000, 16000, 24000, 48000] to [0, 1, 2, 3, 4] */
/// `rateID(R)`
#[inline(always)]
fn rateID(R: opus_int32) -> opus_int {
    silk_min_int(
        5,
        ((((R >> 12) - ((R > 16000) as opus_int32)) >> ((R > 24000) as opus_int32)) - 1),
    )
}

const USE_silk_resampler_copy: opus_int = 0;
const USE_silk_resampler_private_up2_HQ_wrapper: opus_int = 1;
const USE_silk_resampler_private_IIR_FIR: opus_int = 2;
const USE_silk_resampler_private_down_FIR: opus_int = 3;

/* Initialize/reset the resampler state for a given pair of input/output sampling rates */
/// `silk_resampler_init(S, Fs_Hz_in, Fs_Hz_out, forEnc)`
pub fn silk_resampler_init(
    S: &mut silk_resampler_state_struct, /* I/O  Resampler state */
    Fs_Hz_in: opus_int32,                /* I    Input sampling rate (Hz) */
    Fs_Hz_out: opus_int32,               /* I    Output sampling rate (Hz) */
    forEnc: opus_int,                    /* I    If 1: encoder; if 0: decoder */
) -> opus_int {
    let mut up2x: opus_int;

    /* Clear state */
    *S = silk_resampler_state_struct::default();

    /* Input checking */
    if forEnc != 0 {
        if (Fs_Hz_in != 8000 && Fs_Hz_in != 12000 && Fs_Hz_in != 16000 && Fs_Hz_in != 24000 && Fs_Hz_in != 48000)
            || (Fs_Hz_out != 8000 && Fs_Hz_out != 12000 && Fs_Hz_out != 16000)
        {
            celt_assert!(false);
            return -1;
        }
        S.inputDelay = delay_matrix_enc[rateID(Fs_Hz_in) as usize][rateID(Fs_Hz_out) as usize] as opus_int;
    } else {
        if (Fs_Hz_in != 8000 && Fs_Hz_in != 12000 && Fs_Hz_in != 16000)
            || (Fs_Hz_out != 8000 && Fs_Hz_out != 12000 && Fs_Hz_out != 16000 && Fs_Hz_out != 24000 && Fs_Hz_out != 48000)
        {
            celt_assert!(false);
            return -1;
        }
        S.inputDelay = delay_matrix_dec[rateID(Fs_Hz_in) as usize][rateID(Fs_Hz_out) as usize] as opus_int;
    }

    S.Fs_in_kHz = silk_DIV32_16(Fs_Hz_in, 1000);
    S.Fs_out_kHz = silk_DIV32_16(Fs_Hz_out, 1000);

    /* Number of samples processed per batch */
    S.batchSize = S.Fs_in_kHz * RESAMPLER_MAX_BATCH_SIZE_MS;

    /* Find resampler with the right sampling ratio */
    up2x = 0;
    if Fs_Hz_out > Fs_Hz_in {
        /* Upsample */
        if Fs_Hz_out == silk_MUL(Fs_Hz_in, 2) {
            /* Fs_out : Fs_in = 2 : 1 */
            /* Special case: directly use 2x upsampler */
            S.resampler_function = USE_silk_resampler_private_up2_HQ_wrapper;
        } else {
            /* Default resampler */
            S.resampler_function = USE_silk_resampler_private_IIR_FIR;
            up2x = 1;
        }
    } else if Fs_Hz_out < Fs_Hz_in {
        /* Downsample */
        S.resampler_function = USE_silk_resampler_private_down_FIR;
        if silk_MUL(Fs_Hz_out, 4) == silk_MUL(Fs_Hz_in, 3) {
            /* Fs_out : Fs_in = 3 : 4 */
            S.FIR_Fracs = 3;
            S.FIR_Order = RESAMPLER_DOWN_ORDER_FIR0;
            S.Coefs = &silk_Resampler_3_4_COEFS;
        } else if silk_MUL(Fs_Hz_out, 3) == silk_MUL(Fs_Hz_in, 2) {
            /* Fs_out : Fs_in = 2 : 3 */
            S.FIR_Fracs = 2;
            S.FIR_Order = RESAMPLER_DOWN_ORDER_FIR0;
            S.Coefs = &silk_Resampler_2_3_COEFS;
        } else if silk_MUL(Fs_Hz_out, 2) == Fs_Hz_in {
            /* Fs_out : Fs_in = 1 : 2 */
            S.FIR_Fracs = 1;
            S.FIR_Order = RESAMPLER_DOWN_ORDER_FIR1;
            S.Coefs = &silk_Resampler_1_2_COEFS;
        } else if silk_MUL(Fs_Hz_out, 3) == Fs_Hz_in {
            /* Fs_out : Fs_in = 1 : 3 */
            S.FIR_Fracs = 1;
            S.FIR_Order = RESAMPLER_DOWN_ORDER_FIR2;
            S.Coefs = &silk_Resampler_1_3_COEFS;
        } else if silk_MUL(Fs_Hz_out, 4) == Fs_Hz_in {
            /* Fs_out : Fs_in = 1 : 4 */
            S.FIR_Fracs = 1;
            S.FIR_Order = RESAMPLER_DOWN_ORDER_FIR2;
            S.Coefs = &silk_Resampler_1_4_COEFS;
        } else if silk_MUL(Fs_Hz_out, 6) == Fs_Hz_in {
            /* Fs_out : Fs_in = 1 : 6 */
            S.FIR_Fracs = 1;
            S.FIR_Order = RESAMPLER_DOWN_ORDER_FIR2;
            S.Coefs = &silk_Resampler_1_6_COEFS;
        } else {
            /* None available */
            celt_assert!(false);
            return -1;
        }
    } else {
        /* Input and output sampling rates are equal: copy */
        S.resampler_function = USE_silk_resampler_copy;
    }

    /* Ratio of input/output samples */
    S.invRatio_Q16 = silk_LSHIFT32(
        silk_DIV32(silk_LSHIFT32(Fs_Hz_in, 14 + up2x), Fs_Hz_out),
        2,
    );
    /* Make sure the ratio is rounded up */
    while silk_SMULWW(S.invRatio_Q16, Fs_Hz_out) < silk_LSHIFT32(Fs_Hz_in, up2x) {
        S.invRatio_Q16 += 1;
    }

    0
}

/* Resampler: convert from one sampling rate to another */
/* Input and output sampling rate are at most 48000 Hz  */
/// `silk_resampler(S, out, in, inLen)`
pub fn silk_resampler(
    S: &mut silk_resampler_state_struct, /* I/O  Resampler state */
    out: &mut [opus_int16],              /* O    Output signal */
    in_: &[opus_int16],                  /* I    Input signal */
    inLen: opus_int32,                   /* I    Number of input samples */
) -> opus_int {
    let nSamples: opus_int;

    /* Need at least 1 ms of input data */
    celt_assert!(inLen >= S.Fs_in_kHz);
    /* Delay can't exceed the 1 ms of buffering */
    celt_assert!(S.inputDelay <= S.Fs_in_kHz);

    nSamples = S.Fs_in_kHz - S.inputDelay;

    /* Copy to delay buffer */
    S.delayBuf[S.inputDelay as usize..(S.inputDelay + nSamples) as usize]
        .copy_from_slice(&in_[..nSamples as usize]);

    let fs_in_kHz = S.Fs_in_kHz;
    let fs_out_kHz = S.Fs_out_kHz;
    let inputDelay = S.inputDelay;
    let delayBuf = S.delayBuf;

    match S.resampler_function {
        USE_silk_resampler_private_up2_HQ_wrapper => {
            silk_resampler_private_up2_HQ_wrapper(S, out, &delayBuf, fs_in_kHz);
            silk_resampler_private_up2_HQ_wrapper(
                S,
                &mut out[fs_out_kHz as usize..],
                &in_[nSamples as usize..],
                inLen - fs_in_kHz,
            );
        }
        USE_silk_resampler_private_IIR_FIR => {
            silk_resampler_private_IIR_FIR(S, out, &delayBuf, fs_in_kHz);
            silk_resampler_private_IIR_FIR(
                S,
                &mut out[fs_out_kHz as usize..],
                &in_[nSamples as usize..],
                inLen - fs_in_kHz,
            );
        }
        USE_silk_resampler_private_down_FIR => {
            silk_resampler_private_down_FIR(S, out, &delayBuf, fs_in_kHz);
            silk_resampler_private_down_FIR(
                S,
                &mut out[fs_out_kHz as usize..],
                &in_[nSamples as usize..],
                inLen - fs_in_kHz,
            );
        }
        _ => {
            out[..fs_in_kHz as usize].copy_from_slice(&delayBuf[..fs_in_kHz as usize]);
            let tail = (inLen - fs_in_kHz) as usize;
            out[fs_out_kHz as usize..fs_out_kHz as usize + tail]
                .copy_from_slice(&in_[nSamples as usize..nSamples as usize + tail]);
        }
    }

    /* Copy to delay buffer */
    let tail = (inLen - inputDelay) as usize;
    S.delayBuf[..inputDelay as usize]
        .copy_from_slice(&in_[tail..tail + inputDelay as usize]);

    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_the_resampler_function() {
        let mut s = silk_resampler_state_struct::default();
        assert_eq!(silk_resampler_init(&mut s, 8000, 8000, 0), 0);
        assert_eq!(s.resampler_function, 0); /* copy */
        assert_eq!(silk_resampler_init(&mut s, 8000, 16000, 0), 0);
        assert_eq!(s.resampler_function, 1); /* up2 */
        assert_eq!(silk_resampler_init(&mut s, 8000, 48000, 0), 0);
        assert_eq!(s.resampler_function, 2); /* IIR_FIR */
        assert_eq!(silk_resampler_init(&mut s, 48000, 8000, 1), 0);
        assert_eq!(s.resampler_function, 3); /* down_FIR */
        assert_eq!(s.FIR_Order, RESAMPLER_DOWN_ORDER_FIR2);
        assert_eq!(s.FIR_Fracs, 1);
    }

    #[test]
    fn copy_path_delays_the_input() {
        let mut s = silk_resampler_state_struct::default();
        silk_resampler_init(&mut s, 8000, 8000, 0);
        let mut out = [0i16; 8];
        silk_resampler(&mut s, &mut out, &[1, 2, 3, 4, 5, 6, 7, 8], 8);
        assert_eq!(out, [0, 0, 0, 0, 1, 2, 3, 4]);
    }
}

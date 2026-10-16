//! Mirror of `silk/NSQ.c` (plus the `silk/NSQ.h` inline kernels) from libopus 1.6.1,
//! transliterated to safe Rust.
//!
//! Dispatch note (see `docs/rtcd-kernels.md` §4.2): at arch 3/4 the reference
//! build calls `silk_NSQ_sse4_1` (`silk/x86/NSQ_sse4_1.c`), which is a
//! transcription of `silk_NSQ_c` whose only vectorised helpers
//! (`silk_nsq_scale_states_sse4_1`) implement `silk_SMULWW` exactly; the
//! genuinely vectorised `silk_noise_shape_quantizer_10_16_sse4_1` is
//! unreachable because `silk_setup_complexity` never selects
//! `shapingLPCOrder == 10` together with `predictLPCOrder == 16`.  The port
//! below therefore IS the reference arithmetic: `silk_NSQ_c`, `silk_NSQ_sse4_1`
//! and the dispatched `silk_NSQ` are all this body.
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
use crate::silk::typedef::silk_assert;
use crate::silk::define::*;
use crate::silk::inlines::*;
use crate::silk::lpc_analysis_filter::*;
use crate::silk::macros::*;
use crate::silk::sigproc_fix::*;
use crate::silk::structs::*;
use crate::silk::tables_other::*;
use crate::silk::typedef::*;
use crate::types::*;

// ---------------------------------------------------------------------------
// `silk/NSQ.h` inline kernels
// ---------------------------------------------------------------------------

/// `silk_noise_shape_quantizer_short_prediction_c`
///
/// `buf32` is the whole `sLPC_Q14` buffer and `pos` the C's `buf32` pointer
/// position; the C reads `buf32[0]` down to `buf32[-15]`.
pub fn silk_noise_shape_quantizer_short_prediction(
    buf32: &[opus_int32],
    pos: opus_int,
    coef16: &[opus_int16],
    order: opus_int,
) -> opus_int32 {
    silk_assert!(order == 10 || order == 16);

    /* Avoids introducing a bias because silk_SMLAWB() always rounds to -inf */
    let mut out = silk_RSHIFT(order, 1);
    out = silk_SMLAWB(out, buf32[(pos - 0) as usize], coef16[0] as opus_int32);
    out = silk_SMLAWB(out, buf32[(pos - 1) as usize], coef16[1] as opus_int32);
    out = silk_SMLAWB(out, buf32[(pos - 2) as usize], coef16[2] as opus_int32);
    out = silk_SMLAWB(out, buf32[(pos - 3) as usize], coef16[3] as opus_int32);
    out = silk_SMLAWB(out, buf32[(pos - 4) as usize], coef16[4] as opus_int32);
    out = silk_SMLAWB(out, buf32[(pos - 5) as usize], coef16[5] as opus_int32);
    out = silk_SMLAWB(out, buf32[(pos - 6) as usize], coef16[6] as opus_int32);
    out = silk_SMLAWB(out, buf32[(pos - 7) as usize], coef16[7] as opus_int32);
    out = silk_SMLAWB(out, buf32[(pos - 8) as usize], coef16[8] as opus_int32);
    out = silk_SMLAWB(out, buf32[(pos - 9) as usize], coef16[9] as opus_int32);

    if order == 16 {
        out = silk_SMLAWB(out, buf32[(pos - 10) as usize], coef16[10] as opus_int32);
        out = silk_SMLAWB(out, buf32[(pos - 11) as usize], coef16[11] as opus_int32);
        out = silk_SMLAWB(out, buf32[(pos - 12) as usize], coef16[12] as opus_int32);
        out = silk_SMLAWB(out, buf32[(pos - 13) as usize], coef16[13] as opus_int32);
        out = silk_SMLAWB(out, buf32[(pos - 14) as usize], coef16[14] as opus_int32);
        out = silk_SMLAWB(out, buf32[(pos - 15) as usize], coef16[15] as opus_int32);
    }
    out
}

/// `silk_NSQ_noise_shape_feedback_loop_c`
pub fn silk_NSQ_noise_shape_feedback_loop(
    data0: opus_int32,
    data1: &mut [opus_int32],
    coef: &[opus_int16],
    order: opus_int,
) -> opus_int32 {
    let mut tmp2 = data0;
    let mut tmp1 = data1[0];
    data1[0] = tmp2;

    let mut out = silk_RSHIFT(order, 1);
    out = silk_SMLAWB(out, tmp2, coef[0] as opus_int32);

    let mut j: opus_int = 2;
    while j < order {
        tmp2 = data1[(j - 1) as usize];
        data1[(j - 1) as usize] = tmp1;
        out = silk_SMLAWB(out, tmp1, coef[(j - 1) as usize] as opus_int32);
        tmp1 = data1[(j + 0) as usize];
        data1[(j + 0) as usize] = tmp2;
        out = silk_SMLAWB(out, tmp2, coef[(j + 0) as usize] as opus_int32);
        j += 2;
    }
    data1[(order - 1) as usize] = tmp1;
    out = silk_SMLAWB(out, tmp1, coef[(order - 1) as usize] as opus_int32);
    /* Q11 -> Q12 */
    out = silk_LSHIFT32(out, 1);
    out
}

// ---------------------------------------------------------------------------
// silk_noise_shape_quantizer
// ---------------------------------------------------------------------------

/// `silk_noise_shape_quantizer` (`NSQ.c:183`).
///
/// `xq_off` is the C's `xq` pointer offset into `NSQ.xq` (the C passes
/// `&NSQ->xq[psEncC->ltp_mem_length + k * subfr_length]`); indexing the state
/// buffer directly keeps the aliasing within one `&mut silk_nsq_state`.
pub fn silk_noise_shape_quantizer(
    NSQ: &mut silk_nsq_state,               /* I/O  NSQ state                       */
    signalType: opus_int,                   /* I    Signal type                     */
    x_sc_Q10: &[opus_int32],                /* I                                    */
    pulses: &mut [opus_int8],               /* O    [subfr_length]                  */
    xq_off: opus_int,                       /* I    offset of xq in NSQ.xq          */
    sLTP_Q15: &mut [opus_int32],            /* I/O  LTP state                       */
    a_Q12: &[opus_int16],                   /* I    Short term prediction coefs     */
    b_Q14: &[opus_int16],                   /* I    Long term prediction coefs      */
    AR_shp_Q13: &[opus_int16],              /* I    Noise shaping AR coefs          */
    lag: opus_int,                          /* I    Pitch lag                       */
    HarmShapeFIRPacked_Q14: opus_int32,     /* I                                    */
    Tilt_Q14: opus_int,                     /* I    Spectral tilt                   */
    LF_shp_Q14: opus_int32,                 /* I                                    */
    Gain_Q16: opus_int32,                   /* I                                    */
    Lambda_Q10: opus_int,                   /* I    Rate/distortion tradeoff        */
    offset_Q10: opus_int,                   /* I                                    */
    length: opus_int,                       /* I    Input length                    */
    shapingLPCOrder: opus_int,              /* I    Noise shaping AR filter order   */
    predictLPCOrder: opus_int,              /* I    Prediction filter order         */
    arch: opus_int,                         /* I    Architecture                    */
) {
    let _ = arch;

    let mut shp_lag_ptr: opus_int =
        NSQ.sLTP_shp_buf_idx - lag + HARM_SHAPE_FIR_TAPS / 2;
    let mut pred_lag_ptr: opus_int = NSQ.sLTP_buf_idx - lag + LTP_ORDER / 2;
    let Gain_Q10: opus_int32 = silk_RSHIFT(Gain_Q16, 6);

    /* Set up short term AR state */
    let mut psLPC_Q14: opus_int = NSQ_LPC_BUF_LENGTH - 1;

    for i in 0..length {
        /* Generate dither */
        NSQ.rand_seed = silk_RAND(NSQ.rand_seed);

        /* Short-term prediction */
        let LPC_pred_Q10 = silk_noise_shape_quantizer_short_prediction(
            &NSQ.sLPC_Q14,
            psLPC_Q14,
            a_Q12,
            predictLPCOrder,
        );

        /* Long-term prediction */
        let mut LTP_pred_Q13: opus_int32;
        if signalType == TYPE_VOICED {
            /* Unrolled loop */
            /* Avoids introducing a bias because silk_SMLAWB() always rounds to -inf */
            LTP_pred_Q13 = 2;
            LTP_pred_Q13 = silk_SMLAWB(
                LTP_pred_Q13,
                sLTP_Q15[(pred_lag_ptr + 0) as usize],
                b_Q14[0] as opus_int32,
            );
            LTP_pred_Q13 = silk_SMLAWB(
                LTP_pred_Q13,
                sLTP_Q15[(pred_lag_ptr - 1) as usize],
                b_Q14[1] as opus_int32,
            );
            LTP_pred_Q13 = silk_SMLAWB(
                LTP_pred_Q13,
                sLTP_Q15[(pred_lag_ptr - 2) as usize],
                b_Q14[2] as opus_int32,
            );
            LTP_pred_Q13 = silk_SMLAWB(
                LTP_pred_Q13,
                sLTP_Q15[(pred_lag_ptr - 3) as usize],
                b_Q14[3] as opus_int32,
            );
            LTP_pred_Q13 = silk_SMLAWB(
                LTP_pred_Q13,
                sLTP_Q15[(pred_lag_ptr - 4) as usize],
                b_Q14[4] as opus_int32,
            );
            pred_lag_ptr += 1;
        } else {
            LTP_pred_Q13 = 0;
        }

        /* Noise shape feedback */
        celt_assert!((shapingLPCOrder & 1) == 0); /* check that order is even */
        let mut n_AR_Q12 = silk_NSQ_noise_shape_feedback_loop(
            NSQ.sDiff_shp_Q14,
            &mut NSQ.sAR2_Q14,
            AR_shp_Q13,
            shapingLPCOrder,
        );

        n_AR_Q12 = silk_SMLAWB(n_AR_Q12, NSQ.sLF_AR_shp_Q14, Tilt_Q14);

        let mut n_LF_Q12 = silk_SMULWB(NSQ.sLTP_shp_Q14[(NSQ.sLTP_shp_buf_idx - 1) as usize], LF_shp_Q14);
        n_LF_Q12 = silk_SMLAWT(n_LF_Q12, NSQ.sLF_AR_shp_Q14, LF_shp_Q14);

        celt_assert!(lag > 0 || signalType != TYPE_VOICED);

        /* Combine prediction and noise shaping signals */
        let mut tmp1 =
            silk_SUB32_ovflw(silk_LSHIFT32(LPC_pred_Q10, 2), n_AR_Q12); /* Q12 */
        tmp1 = silk_SUB32_ovflw(tmp1, n_LF_Q12); /* Q12 */
        if lag > 0 {
            /* Symmetric, packed FIR coefficients */
            let mut n_LTP_Q13 = silk_SMULWB(
                silk_ADD_SAT32(
                    NSQ.sLTP_shp_Q14[(shp_lag_ptr + 0) as usize],
                    NSQ.sLTP_shp_Q14[(shp_lag_ptr - 2) as usize],
                ),
                HarmShapeFIRPacked_Q14,
            );
            n_LTP_Q13 = silk_SMLAWT(
                n_LTP_Q13,
                NSQ.sLTP_shp_Q14[(shp_lag_ptr - 1) as usize],
                HarmShapeFIRPacked_Q14,
            );
            n_LTP_Q13 = silk_LSHIFT(n_LTP_Q13, 1);
            shp_lag_ptr += 1;

            let tmp2 = silk_SUB32(LTP_pred_Q13, n_LTP_Q13); /* Q13 */
            tmp1 = silk_ADD32_ovflw(tmp2, silk_LSHIFT32(tmp1, 1)); /* Q13 */
            tmp1 = silk_RSHIFT_ROUND(tmp1, 3); /* Q10 */
        } else {
            tmp1 = silk_RSHIFT_ROUND(tmp1, 2); /* Q10 */
        }

        let mut r_Q10 = silk_SUB32(x_sc_Q10[i as usize], tmp1); /* residual error Q10 */

        /* Flip sign depending on dither */
        if NSQ.rand_seed < 0 {
            r_Q10 = -r_Q10;
        }
        r_Q10 = silk_LIMIT(r_Q10, -(31 << 10), 30 << 10);

        /* Find two quantization level candidates and measure their rate-distortion */
        let mut q1_Q10 = silk_SUB32(r_Q10, offset_Q10);
        let mut q1_Q0 = silk_RSHIFT(q1_Q10, 10);
        if Lambda_Q10 > 2048 {
            /* For aggressive RDO, the bias becomes more than one pulse. */
            let rdo_offset = Lambda_Q10 / 2 - 512;
            if q1_Q10 > rdo_offset {
                q1_Q0 = silk_RSHIFT(q1_Q10 - rdo_offset, 10);
            } else if q1_Q10 < -rdo_offset {
                q1_Q0 = silk_RSHIFT(q1_Q10 + rdo_offset, 10);
            } else if q1_Q10 < 0 {
                q1_Q0 = -1;
            } else {
                q1_Q0 = 0;
            }
        }
        let mut rd1_Q20: opus_int32;
        let mut rd2_Q20: opus_int32;
        let q2_Q10: opus_int32;
        if q1_Q0 > 0 {
            q1_Q10 = silk_SUB32(silk_LSHIFT(q1_Q0, 10), QUANT_LEVEL_ADJUST_Q10);
            q1_Q10 = silk_ADD32(q1_Q10, offset_Q10);
            q2_Q10 = silk_ADD32(q1_Q10, 1024);
            rd1_Q20 = silk_SMULBB(q1_Q10, Lambda_Q10);
            rd2_Q20 = silk_SMULBB(q2_Q10, Lambda_Q10);
        } else if q1_Q0 == 0 {
            q1_Q10 = offset_Q10;
            q2_Q10 = silk_ADD32(q1_Q10, 1024 - QUANT_LEVEL_ADJUST_Q10);
            rd1_Q20 = silk_SMULBB(q1_Q10, Lambda_Q10);
            rd2_Q20 = silk_SMULBB(q2_Q10, Lambda_Q10);
        } else if q1_Q0 == -1 {
            q2_Q10 = offset_Q10;
            q1_Q10 = silk_SUB32(q2_Q10, 1024 - QUANT_LEVEL_ADJUST_Q10);
            rd1_Q20 = silk_SMULBB(-q1_Q10, Lambda_Q10);
            rd2_Q20 = silk_SMULBB(q2_Q10, Lambda_Q10);
        } else {
            /* Q1_Q0 < -1 */
            q1_Q10 = silk_ADD32(silk_LSHIFT(q1_Q0, 10), QUANT_LEVEL_ADJUST_Q10);
            q1_Q10 = silk_ADD32(q1_Q10, offset_Q10);
            q2_Q10 = silk_ADD32(q1_Q10, 1024);
            rd1_Q20 = silk_SMULBB(-q1_Q10, Lambda_Q10);
            rd2_Q20 = silk_SMULBB(-q2_Q10, Lambda_Q10);
        }
        let mut rr_Q10 = silk_SUB32(r_Q10, q1_Q10);
        rd1_Q20 = silk_SMLABB(rd1_Q20, rr_Q10, rr_Q10);
        rr_Q10 = silk_SUB32(r_Q10, q2_Q10);
        rd2_Q20 = silk_SMLABB(rd2_Q20, rr_Q10, rr_Q10);

        if rd2_Q20 < rd1_Q20 {
            q1_Q10 = q2_Q10;
        }

        pulses[i as usize] = silk_RSHIFT_ROUND(q1_Q10, 10) as opus_int8;

        /* Excitation */
        let mut exc_Q14 = silk_LSHIFT(q1_Q10, 4);
        if NSQ.rand_seed < 0 {
            exc_Q14 = -exc_Q14;
        }

        /* Add predictions */
        let LPC_exc_Q14 = silk_ADD_LSHIFT32(exc_Q14, LTP_pred_Q13, 1);
        let xq_Q14 = silk_ADD32_ovflw(LPC_exc_Q14, silk_LSHIFT32(LPC_pred_Q10, 4));

        /* Scale XQ back to normal level before saving */
        NSQ.xq[(xq_off + i) as usize] =
            silk_SAT16(silk_RSHIFT_ROUND(silk_SMULWW(xq_Q14, Gain_Q10), 8)) as opus_int16;

        /* Update states */
        psLPC_Q14 += 1;
        NSQ.sLPC_Q14[psLPC_Q14 as usize] = xq_Q14;
        NSQ.sDiff_shp_Q14 = silk_SUB32_ovflw(xq_Q14, silk_LSHIFT32(x_sc_Q10[i as usize], 4));
        let sLF_AR_shp_Q14 = silk_SUB32_ovflw(NSQ.sDiff_shp_Q14, silk_LSHIFT32(n_AR_Q12, 2));
        NSQ.sLF_AR_shp_Q14 = sLF_AR_shp_Q14;

        NSQ.sLTP_shp_Q14[NSQ.sLTP_shp_buf_idx as usize] =
            silk_SUB32_ovflw(sLF_AR_shp_Q14, silk_LSHIFT32(n_LF_Q12, 2));
        sLTP_Q15[NSQ.sLTP_buf_idx as usize] = silk_LSHIFT(LPC_exc_Q14, 1);
        NSQ.sLTP_shp_buf_idx += 1;
        NSQ.sLTP_buf_idx += 1;

        /* Make dither dependent on quantized signal */
        NSQ.rand_seed = silk_ADD32_ovflw(NSQ.rand_seed, pulses[i as usize] as opus_int32);
    }

    /* Update LPC synth buffer */
    for i in 0..NSQ_LPC_BUF_LENGTH as usize {
        NSQ.sLPC_Q14[i] = NSQ.sLPC_Q14[i + length as usize];
    }
}

// ---------------------------------------------------------------------------
// silk_nsq_scale_states
// ---------------------------------------------------------------------------

/// `silk_nsq_scale_states` (`NSQ.c:368`).  `x16` is the subframe's slice
/// (the C advances the pointer by `subfr_length` per subframe).
fn silk_nsq_scale_states(
    psEncC: &silk_encoder_state,            /* I    Encoder State                   */
    NSQ: &mut silk_nsq_state,               /* I/O  NSQ state                       */
    x16: &[opus_int16],                     /* I    input (subframe)                */
    x_sc_Q10: &mut [opus_int32],            /* O    input scaled with 1/Gain        */
    sLTP: &[opus_int16],                    /* I    re-whitened LTP state in Q0     */
    sLTP_Q15: &mut [opus_int32],            /* O    LTP state matching scaled input  */
    subfr: opus_int,                        /* I    subframe number                 */
    LTP_scale_Q14: opus_int,                /* I                                    */
    Gains_Q16: &[opus_int32],               /* I                                    */
    pitchL: &[opus_int],                    /* I    Pitch lag                       */
    signal_type: opus_int,                  /* I    Signal type                     */
) {
    let lag = pitchL[subfr as usize];
    let mut inv_gain_Q31 = silk_INVERSE32_varQ(silk_max(Gains_Q16[subfr as usize], 1), 47);
    silk_assert!(inv_gain_Q31 != 0);

    /* Scale input */
    let inv_gain_Q26 = silk_RSHIFT_ROUND(inv_gain_Q31, 5);
    for i in 0..psEncC.subfr_length as usize {
        x_sc_Q10[i] = silk_SMULWW(x16[i] as opus_int32, inv_gain_Q26);
    }

    /* After rewhitening the LTP state is un-scaled, so scale with inv_gain_Q16 */
    if NSQ.rewhite_flag != 0 {
        if subfr == 0 {
            /* Do LTP downscaling */
            inv_gain_Q31 = silk_LSHIFT(silk_SMULWB(inv_gain_Q31, LTP_scale_Q14), 2);
        }
        let mut i = NSQ.sLTP_buf_idx - lag - LTP_ORDER / 2;
        while i < NSQ.sLTP_buf_idx {
            silk_assert!(i < MAX_FRAME_LENGTH);
            sLTP_Q15[i as usize] = silk_SMULWB(inv_gain_Q31, sLTP[i as usize] as opus_int32);
            i += 1;
        }
    }

    /* Adjust for changing gain */
    if Gains_Q16[subfr as usize] != NSQ.prev_gain_Q16 {
        let gain_adj_Q16 = silk_DIV32_varQ(NSQ.prev_gain_Q16, Gains_Q16[subfr as usize], 16);

        /* Scale long-term shaping state */
        let mut i = NSQ.sLTP_shp_buf_idx - psEncC.ltp_mem_length;
        while i < NSQ.sLTP_shp_buf_idx {
            NSQ.sLTP_shp_Q14[i as usize] =
                silk_SMULWW(gain_adj_Q16, NSQ.sLTP_shp_Q14[i as usize]);
            i += 1;
        }

        /* Scale long-term prediction state */
        if signal_type == TYPE_VOICED && NSQ.rewhite_flag == 0 {
            let mut i = NSQ.sLTP_buf_idx - lag - LTP_ORDER / 2;
            while i < NSQ.sLTP_buf_idx {
                sLTP_Q15[i as usize] = silk_SMULWW(gain_adj_Q16, sLTP_Q15[i as usize]);
                i += 1;
            }
        }

        NSQ.sLF_AR_shp_Q14 = silk_SMULWW(gain_adj_Q16, NSQ.sLF_AR_shp_Q14);
        NSQ.sDiff_shp_Q14 = silk_SMULWW(gain_adj_Q16, NSQ.sDiff_shp_Q14);

        /* Scale short-term prediction and shaping states */
        for i in 0..NSQ_LPC_BUF_LENGTH as usize {
            NSQ.sLPC_Q14[i] = silk_SMULWW(gain_adj_Q16, NSQ.sLPC_Q14[i]);
        }
        for i in 0..MAX_SHAPE_LPC_ORDER as usize {
            NSQ.sAR2_Q14[i] = silk_SMULWW(gain_adj_Q16, NSQ.sAR2_Q14[i]);
        }

        /* Save inverse gain */
        NSQ.prev_gain_Q16 = Gains_Q16[subfr as usize];
    }
}

// ---------------------------------------------------------------------------
// silk_NSQ_c
// ---------------------------------------------------------------------------

pub fn silk_NSQ_c(
    psEncC: &silk_encoder_state,            /* I    Encoder State                   */
    NSQ: &mut silk_nsq_state,               /* I/O  NSQ state                       */
    psIndices: &mut SideInfoIndices,        /* I/O  Quantization Indices            */
    x16: &[opus_int16],                     /* I    Input                           */
    pulses: &mut [opus_int8],               /* O    Quantized pulse signal          */
    PredCoef_Q12: &[[opus_int16; MAX_LPC_ORDER as usize]; 2], /* I  Short term prediction coefs */
    LTPCoef_Q14: &[opus_int16],             /* I    Long term prediction coefs      */
    AR_Q13: &[opus_int16],                  /* I    Noise shaping coefs             */
    HarmShapeGain_Q14: &[opus_int],         /* I    Long term shaping coefs         */
    Tilt_Q14: &[opus_int],                  /* I    Spectral tilt                   */
    LF_shp_Q14: &[opus_int32],              /* I    Low frequency shaping coefs     */
    Gains_Q16: &[opus_int32],               /* I    Quantization step sizes         */
    pitchL: &[opus_int],                    /* I    Pitch lags                      */
    Lambda_Q10: opus_int,                   /* I    Rate/distortion tradeoff        */
    LTP_scale_Q14: opus_int,                /* I    LTP state scaling               */
) {
    let mut lag: opus_int;
    let mut start_idx: opus_int;
    let LSF_interpolation_flag: opus_int;
    let mut HarmShapeFIRPacked_Q14: opus_int32;

    NSQ.rand_seed = psIndices.Seed as opus_int32;

    /* Set unvoiced lag to the previous one, overwrite later for voiced */
    lag = NSQ.lagPrev;

    silk_assert!(NSQ.prev_gain_Q16 != 0);

    let offset_Q10 =
        silk_Quantization_Offsets_Q10[(psIndices.signalType >> 1) as usize][psIndices.quantOffsetType as usize]
            as opus_int;

    if psIndices.NLSFInterpCoef_Q2 == 4 {
        LSF_interpolation_flag = 0;
    } else {
        LSF_interpolation_flag = 1;
    }

    let alloc_len = (psEncC.ltp_mem_length + psEncC.frame_length) as usize;
    let mut sLTP_Q15 = vec![0 as opus_int32; alloc_len];
    let mut sLTP = vec![0 as opus_int16; alloc_len];
    let mut x_sc_Q10 = vec![0 as opus_int32; psEncC.subfr_length as usize];
    /* Set up pointers to start of sub frame */
    NSQ.sLTP_shp_buf_idx = psEncC.ltp_mem_length;
    NSQ.sLTP_buf_idx = psEncC.ltp_mem_length;
    let pxq_off = psEncC.ltp_mem_length;
    for k in 0..psEncC.nb_subfr {
        let A_Q12 = &PredCoef_Q12[((k >> 1) | (1 - LSF_interpolation_flag)) as usize];
        let B_Q14 = &LTPCoef_Q14[(k * LTP_ORDER) as usize..];
        let AR_shp_Q13 = &AR_Q13[(k * MAX_SHAPE_LPC_ORDER) as usize..];

        /* Noise shape parameters */
        silk_assert!(HarmShapeGain_Q14[k as usize] >= 0);
        HarmShapeFIRPacked_Q14 = silk_RSHIFT(HarmShapeGain_Q14[k as usize], 2);
        HarmShapeFIRPacked_Q14 |=
            silk_LSHIFT(silk_RSHIFT(HarmShapeGain_Q14[k as usize], 1), 16);

        NSQ.rewhite_flag = 0;
        if psIndices.signalType as opus_int == TYPE_VOICED {
            /* Voiced */
            lag = pitchL[k as usize];

            /* Re-whitening */
            if (k & (3 - silk_LSHIFT(LSF_interpolation_flag, 1))) == 0 {
                /* Rewhiten with new A coefs */
                start_idx =
                    psEncC.ltp_mem_length - lag - psEncC.predictLPCOrder - LTP_ORDER / 2;
                celt_assert!(start_idx > 0);

                let in_off = start_idx + k * psEncC.subfr_length;
                let out_start = start_idx as usize;
                silk_LPC_analysis_filter(
                    &mut sLTP[out_start..],
                    &NSQ.xq[in_off as usize..],
                    A_Q12,
                    psEncC.ltp_mem_length - start_idx,
                    psEncC.predictLPCOrder,
                    psEncC.arch,
                );

                NSQ.rewhite_flag = 1;
                NSQ.sLTP_buf_idx = psEncC.ltp_mem_length;
            }
        }

        silk_nsq_scale_states(
            psEncC,
            NSQ,
            &x16[(k * psEncC.subfr_length) as usize..],
            &mut x_sc_Q10,
            &sLTP,
            &mut sLTP_Q15,
            k,
            LTP_scale_Q14,
            Gains_Q16,
            pitchL,
            psIndices.signalType as opus_int,
        );

        silk_noise_shape_quantizer(
            NSQ,
            psIndices.signalType as opus_int,
            &x_sc_Q10,
            &mut pulses[(k * psEncC.subfr_length) as usize..],
            pxq_off + k * psEncC.subfr_length,
            &mut sLTP_Q15,
            A_Q12,
            B_Q14,
            AR_shp_Q13,
            lag,
            HarmShapeFIRPacked_Q14,
            Tilt_Q14[k as usize],
            LF_shp_Q14[k as usize],
            Gains_Q16[k as usize],
            Lambda_Q10,
            offset_Q10,
            psEncC.subfr_length,
            psEncC.shapingLPCOrder,
            psEncC.predictLPCOrder,
            psEncC.arch,
        );
    }

    /* Update lagPrev for next frame */
    NSQ.lagPrev = pitchL[(psEncC.nb_subfr - 1) as usize];

    /* Save quantized speech and noise shaping signals */
    /* silk_memmove(NSQ->xq, &NSQ->xq[frame_length], ltp_mem_length) */
    let fl = psEncC.frame_length as usize;
    let lml = psEncC.ltp_mem_length as usize;
    NSQ.xq.copy_within(fl..fl + lml, 0);
    NSQ.sLTP_shp_Q14.copy_within(fl..fl + lml, 0);
}

/// `silk_NSQ_sse4_1` — bit-identical to `silk_NSQ_c` (see the module header).
pub fn silk_NSQ_sse4_1(
    psEncC: &silk_encoder_state,
    NSQ: &mut silk_nsq_state,
    psIndices: &mut SideInfoIndices,
    x16: &[opus_int16],
    pulses: &mut [opus_int8],
    PredCoef_Q12: &[[opus_int16; MAX_LPC_ORDER as usize]; 2],
    LTPCoef_Q14: &[opus_int16],
    AR_Q13: &[opus_int16],
    HarmShapeGain_Q14: &[opus_int],
    Tilt_Q14: &[opus_int],
    LF_shp_Q14: &[opus_int32],
    Gains_Q16: &[opus_int32],
    pitchL: &[opus_int],
    Lambda_Q10: opus_int,
    LTP_scale_Q14: opus_int,
) {
    silk_NSQ_c(
        psEncC,
        NSQ,
        psIndices,
        x16,
        pulses,
        PredCoef_Q12,
        LTPCoef_Q14,
        AR_Q13,
        HarmShapeGain_Q14,
        Tilt_Q14,
        LF_shp_Q14,
        Gains_Q16,
        pitchL,
        Lambda_Q10,
        LTP_scale_Q14,
    );
}

/// The dispatched entry (`SILK_NSQ_IMPL[(arch) & OPUS_ARCHMASK]`).  `arch` 3/4
/// select `silk_NSQ_sse4_1`, which is the same arithmetic.
#[allow(clippy::too_many_arguments)]
pub fn silk_NSQ(
    psEncC: &silk_encoder_state,
    NSQ: &mut silk_nsq_state,
    psIndices: &mut SideInfoIndices,
    x16: &[opus_int16],
    pulses: &mut [opus_int8],
    PredCoef_Q12: &[[opus_int16; MAX_LPC_ORDER as usize]; 2],
    LTPCoef_Q14: &[opus_int16],
    AR_Q13: &[opus_int16],
    HarmShapeGain_Q14: &[opus_int],
    Tilt_Q14: &[opus_int],
    LF_shp_Q14: &[opus_int32],
    Gains_Q16: &[opus_int32],
    pitchL: &[opus_int],
    Lambda_Q10: opus_int,
    LTP_scale_Q14: opus_int,
    arch: opus_int,
) {
    let _ = arch;
    silk_NSQ_sse4_1(
        psEncC,
        NSQ,
        psIndices,
        x16,
        pulses,
        PredCoef_Q12,
        LTPCoef_Q14,
        AR_Q13,
        HarmShapeGain_Q14,
        Tilt_Q14,
        LF_shp_Q14,
        Gains_Q16,
        pitchL,
        Lambda_Q10,
        LTP_scale_Q14,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `silk_RAND`: `RAND_INCREMENT + seed*RAND_MULTIPLIER` mod 2^32, i.e.
    /// `907633515 + seed*196314165` (`silk_MLA_ovflw` argument order).
    #[test]
    fn rand_is_wrapping_mla() {
        let mut seed = 12345i32;
        for _ in 0..8 {
            let expect = seed
                .wrapping_mul(RAND_MULTIPLIER)
                .wrapping_add(RAND_INCREMENT);
            seed = silk_RAND(seed);
            assert_eq!(seed, expect);
        }
    }

    /// The short-term predictor is `order/2 + sum silK_SMLAWB(lpc[-i], a[i])`.
    #[test]
    fn short_prediction_matches_manual_sum() {
        let mut buf = [0i32; 32];
        for (i, b) in buf.iter_mut().enumerate() {
            *b = (i as i32 - 16) * 1000;
        }
        let coef: [i16; 16] = [
            1234, -2345, 3456, -4567, 5678, -6789, 7890, -8901, 9012, -1012, 1113, -1415, 1617,
            -1819, 2021, -2223,
        ];
        let pos = 20i32;
        let got = silk_noise_shape_quantizer_short_prediction(&buf, pos, &coef, 16);
        let mut want = 8i32;
        for i in 0..16 {
            want = silk_SMLAWB(want, buf[(pos - i) as usize], coef[i as usize] as i32);
        }
        assert_eq!(got, want);

        let got10 = silk_noise_shape_quantizer_short_prediction(&buf, pos, &coef, 10);
        let mut want10 = 5i32;
        for i in 0..10 {
            want10 = silk_SMLAWB(want10, buf[(pos - i) as usize], coef[i as usize] as i32);
        }
        assert_eq!(got10, want10);
    }
}

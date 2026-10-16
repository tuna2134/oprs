//! Mirror of `silk/NSQ_del_dec.c` **and** `silk/x86/NSQ_del_dec_avx2.c` from
//! libopus 1.6.1, transliterated to safe Rust.
//!
//! Dispatch (see `docs/rtcd-kernels.md` §4.4/§4.5): the reference build routes
//! `silk_NSQ_del_dec` through `SILK_NSQ_DEL_DEC_IMPL[(arch) & OPUS_ARCHMASK]`.
//! At arch 4 (this host) that is **`silk_NSQ_del_dec_avx2`**, a Q14 rewrite of
//! the scalar kernel which is *not* provably identical to the C version: its
//! `silk_sar_round_smulww` deliberately omits the intermediate int32
//! truncation of `silk_SMULWW + silk_RSHIFT_ROUND`.  The AVX2 kernel itself
//! falls back to `silk_NSQ_del_dec_c` when `psEncC->nStatesDelayedDecision` is
//! 1 or 2 (`verify_assumptions`), and arch 3 selects
//! `silk_NSQ_del_dec_sse4_1`, which `docs/rtcd-kernels.md` §4.4 proves is a
//! transcription of the C body (only the short-term prediction is vectorised,
//! exactly).
//!
//! The `__m128i` vectors of the AVX2 file are modelled as `[i32; 4]` and the
//! `__m256i` ones as `[i32; 8]`, with each intrinsic transliterated literally
//! (see the helper functions below), so the arithmetic — including the
//! saturating add/sub, the rounding, the byte-shuffle winner replacement and
//! the int64 `silk_sar_round_smulww` — is reproduced bit for bit.
//!
//! Original C copyright notices (this file is a derived work):
// /***********************************************************************
// Copyright (c) 2021 Google Inc.
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
use crate::silk::define::*;
use crate::silk::inlines::*;
use crate::silk::macros::*;
use crate::silk::nsq::silk_noise_shape_quantizer_short_prediction;
use crate::silk::sigproc_fix::*;
use crate::silk::structs::*;
use crate::silk::tables_other::*;
use crate::silk::typedef::*;
use crate::types::*;

// ===========================================================================
// `silk/NSQ_del_dec.c`
// ===========================================================================

/// `NSQ_del_dec_struct`
#[repr(C)]
#[derive(Clone, Copy)]
pub struct NSQ_del_dec_struct {
    pub sLPC_Q14: [opus_int32; (MAX_SUB_FRAME_LENGTH + NSQ_LPC_BUF_LENGTH) as usize],
    pub RandState: [opus_int32; DECISION_DELAY as usize],
    pub Q_Q10: [opus_int32; DECISION_DELAY as usize],
    pub Xq_Q14: [opus_int32; DECISION_DELAY as usize],
    pub Pred_Q15: [opus_int32; DECISION_DELAY as usize],
    pub Shape_Q14: [opus_int32; DECISION_DELAY as usize],
    pub sAR2_Q14: [opus_int32; MAX_SHAPE_LPC_ORDER as usize],
    pub LF_AR_Q14: opus_int32,
    pub Diff_Q14: opus_int32,
    pub Seed: opus_int32,
    pub SeedInit: opus_int32,
    pub RD_Q10: opus_int32,
}

impl Default for NSQ_del_dec_struct {
    fn default() -> Self {
        NSQ_del_dec_struct {
            sLPC_Q14: [0; (MAX_SUB_FRAME_LENGTH + NSQ_LPC_BUF_LENGTH) as usize],
            RandState: [0; DECISION_DELAY as usize],
            Q_Q10: [0; DECISION_DELAY as usize],
            Xq_Q14: [0; DECISION_DELAY as usize],
            Pred_Q15: [0; DECISION_DELAY as usize],
            Shape_Q14: [0; DECISION_DELAY as usize],
            sAR2_Q14: [0; MAX_SHAPE_LPC_ORDER as usize],
            LF_AR_Q14: 0,
            Diff_Q14: 0,
            Seed: 0,
            SeedInit: 0,
            RD_Q10: 0,
        }
    }
}

/// `NSQ_sample_struct`
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct NSQ_sample_struct {
    pub Q_Q10: opus_int32,
    pub RD_Q10: opus_int32,
    pub xq_Q14: opus_int32,
    pub LF_AR_Q14: opus_int32,
    pub Diff_Q14: opus_int32,
    pub sLTP_shp_Q14: opus_int32,
    pub LPC_exc_Q14: opus_int32,
}

fn silk_NSQ_del_dec_scale_states(
    psEncC: &silk_encoder_state,
    NSQ: &mut silk_nsq_state,
    psDelDec: &mut [NSQ_del_dec_struct],
    x16: &[opus_int16],
    x_sc_Q10: &mut [opus_int32],
    sLTP: &[opus_int16],
    sLTP_Q15: &mut [opus_int32],
    subfr: opus_int,
    nStatesDelayedDecision: opus_int,
    LTP_scale_Q14: opus_int,
    Gains_Q16: &[opus_int32],
    pitchL: &[opus_int],
    signal_type: opus_int,
    decisionDelay: opus_int,
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
            NSQ.sLTP_shp_Q14[i as usize] = silk_SMULWW(gain_adj_Q16, NSQ.sLTP_shp_Q14[i as usize]);
            i += 1;
        }

        /* Scale long-term prediction state */
        if signal_type == TYPE_VOICED && NSQ.rewhite_flag == 0 {
            let mut i = NSQ.sLTP_buf_idx - lag - LTP_ORDER / 2;
            while i < NSQ.sLTP_buf_idx - decisionDelay {
                sLTP_Q15[i as usize] = silk_SMULWW(gain_adj_Q16, sLTP_Q15[i as usize]);
                i += 1;
            }
        }

        for k in 0..nStatesDelayedDecision as usize {
            let psDD = &mut psDelDec[k];

            /* Scale scalar states */
            psDD.LF_AR_Q14 = silk_SMULWW(gain_adj_Q16, psDD.LF_AR_Q14);
            psDD.Diff_Q14 = silk_SMULWW(gain_adj_Q16, psDD.Diff_Q14);

            /* Scale short-term prediction and shaping states */
            for i in 0..NSQ_LPC_BUF_LENGTH as usize {
                psDD.sLPC_Q14[i] = silk_SMULWW(gain_adj_Q16, psDD.sLPC_Q14[i]);
            }
            for i in 0..MAX_SHAPE_LPC_ORDER as usize {
                psDD.sAR2_Q14[i] = silk_SMULWW(gain_adj_Q16, psDD.sAR2_Q14[i]);
            }
            for i in 0..DECISION_DELAY as usize {
                psDD.Pred_Q15[i] = silk_SMULWW(gain_adj_Q16, psDD.Pred_Q15[i]);
                psDD.Shape_Q14[i] = silk_SMULWW(gain_adj_Q16, psDD.Shape_Q14[i]);
            }
        }

        /* Save inverse gain */
        NSQ.prev_gain_Q16 = Gains_Q16[subfr as usize];
    }
}

/// `silk_noise_shape_quantizer_del_dec` (`NSQ_del_dec.c:315`).
///
/// `pulse_base` is the C's per-subframe `pulses` pointer offset and `xq_off`
/// the `xq` pointer offset inside `NSQ.xq`; both are needed because the kernel
/// writes `decisionDelay` samples *before* the current subframe.
#[allow(clippy::too_many_arguments)]
fn silk_noise_shape_quantizer_del_dec(
    NSQ: &mut silk_nsq_state,
    psDelDec: &mut [NSQ_del_dec_struct],
    signalType: opus_int,
    x_Q10: &[opus_int32],
    pulses: &mut [opus_int8],
    pulse_base: opus_int,
    xq_off: opus_int,
    sLTP_Q15: &mut [opus_int32],
    delayedGain_Q10: &mut [opus_int32],
    a_Q12: &[opus_int16],
    b_Q14: &[opus_int16],
    AR_shp_Q13: &[opus_int16],
    lag: opus_int,
    HarmShapeFIRPacked_Q14: opus_int32,
    Tilt_Q14: opus_int,
    LF_shp_Q14: opus_int32,
    Gain_Q16: opus_int32,
    Lambda_Q10: opus_int,
    offset_Q10: opus_int,
    length: opus_int,
    subfr: opus_int,
    shapingLPCOrder: opus_int,
    predictLPCOrder: opus_int,
    warping_Q16: opus_int,
    nStatesDelayedDecision: opus_int,
    smpl_buf_idx: &mut opus_int,
    decisionDelay: opus_int,
    arch: opus_int,
) {
    let _ = arch;

    celt_assert!(nStatesDelayedDecision > 0);
    let mut psSampleState = vec![[NSQ_sample_struct::default(); 2]; nStatesDelayedDecision as usize];

    let mut shp_lag_ptr: opus_int = NSQ.sLTP_shp_buf_idx - lag + HARM_SHAPE_FIR_TAPS / 2;
    let mut pred_lag_ptr: opus_int = NSQ.sLTP_buf_idx - lag + LTP_ORDER / 2;
    let Gain_Q10: opus_int32 = silk_RSHIFT(Gain_Q16, 6);

    for i in 0..length {
        /* Perform common calculations used in all states */

        /* Long-term prediction */
        let mut LTP_pred_Q14: opus_int32;
        if signalType == TYPE_VOICED {
            /* Unrolled loop */
            /* Avoids introducing a bias because silk_SMLAWB() always rounds to -inf */
            LTP_pred_Q14 = 2;
            LTP_pred_Q14 = silk_SMLAWB(LTP_pred_Q14, sLTP_Q15[(pred_lag_ptr + 0) as usize], b_Q14[0] as opus_int32);
            LTP_pred_Q14 = silk_SMLAWB(LTP_pred_Q14, sLTP_Q15[(pred_lag_ptr - 1) as usize], b_Q14[1] as opus_int32);
            LTP_pred_Q14 = silk_SMLAWB(LTP_pred_Q14, sLTP_Q15[(pred_lag_ptr - 2) as usize], b_Q14[2] as opus_int32);
            LTP_pred_Q14 = silk_SMLAWB(LTP_pred_Q14, sLTP_Q15[(pred_lag_ptr - 3) as usize], b_Q14[3] as opus_int32);
            LTP_pred_Q14 = silk_SMLAWB(LTP_pred_Q14, sLTP_Q15[(pred_lag_ptr - 4) as usize], b_Q14[4] as opus_int32);
            LTP_pred_Q14 = silk_LSHIFT(LTP_pred_Q14, 1); /* Q13 -> Q14 */
            pred_lag_ptr += 1;
        } else {
            LTP_pred_Q14 = 0;
        }

        /* Long-term shaping */
        let mut n_LTP_Q14: opus_int32;
        if lag > 0 {
            /* Symmetric, packed FIR coefficients */
            n_LTP_Q14 = silk_SMULWB(
                silk_ADD_SAT32(
                    NSQ.sLTP_shp_Q14[(shp_lag_ptr + 0) as usize],
                    NSQ.sLTP_shp_Q14[(shp_lag_ptr - 2) as usize],
                ),
                HarmShapeFIRPacked_Q14,
            );
            n_LTP_Q14 = silk_SMLAWT(
                n_LTP_Q14,
                NSQ.sLTP_shp_Q14[(shp_lag_ptr - 1) as usize],
                HarmShapeFIRPacked_Q14,
            );
            n_LTP_Q14 = silk_SUB_LSHIFT32(LTP_pred_Q14, n_LTP_Q14, 2); /* Q12 -> Q14 */
            shp_lag_ptr += 1;
        } else {
            n_LTP_Q14 = 0;
        }

        for k in 0..nStatesDelayedDecision as usize {
            /* Delayed decision state */
            let psDD = &mut psDelDec[k];

            /* Generate dither */
            psDD.Seed = silk_RAND(psDD.Seed);

            /* Pointer used in short term prediction and shaping */
            let psLPC_pos = NSQ_LPC_BUF_LENGTH - 1 + i;
            /* Short-term prediction */
            let mut LPC_pred_Q14 = silk_noise_shape_quantizer_short_prediction(
                &psDD.sLPC_Q14,
                psLPC_pos,
                a_Q12,
                predictLPCOrder,
            );
            LPC_pred_Q14 = silk_LSHIFT(LPC_pred_Q14, 4); /* Q10 -> Q14 */

            /* Noise shape feedback */
            celt_assert!((shapingLPCOrder & 1) == 0); /* check that order is even */
            /* Output of lowpass section */
            let mut tmp2 = silk_SMLAWB(psDD.Diff_Q14, psDD.sAR2_Q14[0], warping_Q16);
            /* Output of allpass section */
            let mut tmp1 = silk_SMLAWB(
                psDD.sAR2_Q14[0],
                silk_SUB32_ovflw(psDD.sAR2_Q14[1], tmp2),
                warping_Q16,
            );
            psDD.sAR2_Q14[0] = tmp2;
            let mut n_AR_Q14 = silk_RSHIFT(shapingLPCOrder, 1);
            n_AR_Q14 = silk_SMLAWB(n_AR_Q14, tmp2, AR_shp_Q13[0] as opus_int32);
            /* Loop over allpass sections */
            let mut j: opus_int = 2;
            while j < shapingLPCOrder {
                /* Output of allpass section */
                tmp2 = silk_SMLAWB(
                    psDD.sAR2_Q14[(j - 1) as usize],
                    silk_SUB32_ovflw(psDD.sAR2_Q14[(j + 0) as usize], tmp1),
                    warping_Q16,
                );
                psDD.sAR2_Q14[(j - 1) as usize] = tmp1;
                n_AR_Q14 = silk_SMLAWB(n_AR_Q14, tmp1, AR_shp_Q13[(j - 1) as usize] as opus_int32);
                /* Output of allpass section */
                tmp1 = silk_SMLAWB(
                    psDD.sAR2_Q14[(j + 0) as usize],
                    silk_SUB32_ovflw(psDD.sAR2_Q14[(j + 1) as usize], tmp2),
                    warping_Q16,
                );
                psDD.sAR2_Q14[(j + 0) as usize] = tmp2;
                n_AR_Q14 = silk_SMLAWB(n_AR_Q14, tmp2, AR_shp_Q13[(j + 0) as usize] as opus_int32);
                j += 2;
            }
            psDD.sAR2_Q14[(shapingLPCOrder - 1) as usize] = tmp1;
            n_AR_Q14 = silk_SMLAWB(
                n_AR_Q14,
                tmp1,
                AR_shp_Q13[(shapingLPCOrder - 1) as usize] as opus_int32,
            );

            let mut n_AR_Q14 = silk_LSHIFT(n_AR_Q14, 1); /* Q11 -> Q12 */
            n_AR_Q14 = silk_SMLAWB(n_AR_Q14, psDD.LF_AR_Q14, Tilt_Q14); /* Q12 */
            let n_AR_Q14 = silk_LSHIFT(n_AR_Q14, 2); /* Q12 -> Q14 */

            let mut n_LF_Q14 = silk_SMULWB(psDD.Shape_Q14[*smpl_buf_idx as usize], LF_shp_Q14); /* Q12 */
            n_LF_Q14 = silk_SMLAWT(n_LF_Q14, psDD.LF_AR_Q14, LF_shp_Q14); /* Q12 */
            let n_LF_Q14 = silk_LSHIFT(n_LF_Q14, 2); /* Q12 -> Q14 */

            /* Input minus prediction plus noise feedback                       */
            /* r = x[ i ] - LTP_pred - LPC_pred + n_AR + n_Tilt + n_LF + n_LTP  */
            let mut tmp1 = silk_ADD_SAT32(n_AR_Q14, n_LF_Q14); /* Q14 */
            let tmp2 = silk_ADD32_ovflw(n_LTP_Q14, LPC_pred_Q14); /* Q13 */
            tmp1 = silk_SUB_SAT32(tmp2, tmp1); /* Q13 */
            let tmp1 = silk_RSHIFT_ROUND(tmp1, 4); /* Q10 */

            let mut r_Q10 = silk_SUB32(x_Q10[i as usize], tmp1); /* residual error Q10 */

            /* Flip sign depending on dither */
            if psDD.Seed < 0 {
                r_Q10 = r_Q10.wrapping_neg();
            }
            let mut r_Q10 = silk_LIMIT(r_Q10, -(31 << 10), 30 << 10);

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
            let q2_Q10: opus_int32;
            let mut rd1_Q10: opus_int32;
            let mut rd2_Q10: opus_int32;
            if q1_Q0 > 0 {
                q1_Q10 = silk_SUB32(silk_LSHIFT(q1_Q0, 10), QUANT_LEVEL_ADJUST_Q10);
                q1_Q10 = silk_ADD32(q1_Q10, offset_Q10);
                q2_Q10 = silk_ADD32(q1_Q10, 1024);
                rd1_Q10 = silk_SMULBB(q1_Q10, Lambda_Q10);
                rd2_Q10 = silk_SMULBB(q2_Q10, Lambda_Q10);
            } else if q1_Q0 == 0 {
                q1_Q10 = offset_Q10;
                q2_Q10 = silk_ADD32(q1_Q10, 1024 - QUANT_LEVEL_ADJUST_Q10);
                rd1_Q10 = silk_SMULBB(q1_Q10, Lambda_Q10);
                rd2_Q10 = silk_SMULBB(q2_Q10, Lambda_Q10);
            } else if q1_Q0 == -1 {
                q2_Q10 = offset_Q10;
                q1_Q10 = silk_SUB32(q2_Q10, 1024 - QUANT_LEVEL_ADJUST_Q10);
                rd1_Q10 = silk_SMULBB(-q1_Q10, Lambda_Q10);
                rd2_Q10 = silk_SMULBB(q2_Q10, Lambda_Q10);
            } else {
                /* q1_Q0 < -1 */
                q1_Q10 = silk_ADD32(silk_LSHIFT(q1_Q0, 10), QUANT_LEVEL_ADJUST_Q10);
                q1_Q10 = silk_ADD32(q1_Q10, offset_Q10);
                q2_Q10 = silk_ADD32(q1_Q10, 1024);
                rd1_Q10 = silk_SMULBB(-q1_Q10, Lambda_Q10);
                rd2_Q10 = silk_SMULBB(-q2_Q10, Lambda_Q10);
            }
            let mut rr_Q10 = silk_SUB32(r_Q10, q1_Q10);
            rd1_Q10 = silk_RSHIFT(silk_SMLABB(rd1_Q10, rr_Q10, rr_Q10), 10);
            rr_Q10 = silk_SUB32(r_Q10, q2_Q10);
            rd2_Q10 = silk_RSHIFT(silk_SMLABB(rd2_Q10, rr_Q10, rr_Q10), 10);

            if rd1_Q10 < rd2_Q10 {
                psSampleState[k][0].RD_Q10 = silk_ADD32(psDD.RD_Q10, rd1_Q10);
                psSampleState[k][1].RD_Q10 = silk_ADD32(psDD.RD_Q10, rd2_Q10);
                psSampleState[k][0].Q_Q10 = q1_Q10;
                psSampleState[k][1].Q_Q10 = q2_Q10;
            } else {
                psSampleState[k][0].RD_Q10 = silk_ADD32(psDD.RD_Q10, rd2_Q10);
                psSampleState[k][1].RD_Q10 = silk_ADD32(psDD.RD_Q10, rd1_Q10);
                psSampleState[k][0].Q_Q10 = q2_Q10;
                psSampleState[k][1].Q_Q10 = q1_Q10;
            }

            /* Update states for best quantization */

            /* Quantized excitation */
            let mut exc_Q14 = silk_LSHIFT32(psSampleState[k][0].Q_Q10, 4);
            if psDD.Seed < 0 {
                exc_Q14 = exc_Q14.wrapping_neg();
            }

            /* Add predictions */
            let LPC_exc_Q14 = silk_ADD32(exc_Q14, LTP_pred_Q14);
            let xq_Q14 = silk_ADD32_ovflw(LPC_exc_Q14, LPC_pred_Q14);

            /* Update states */
            psSampleState[k][0].Diff_Q14 = silk_SUB32_ovflw(xq_Q14, silk_LSHIFT32(x_Q10[i as usize], 4));
            let sLF_AR_shp_Q14 = silk_SUB32_ovflw(psSampleState[k][0].Diff_Q14, n_AR_Q14);
            psSampleState[k][0].sLTP_shp_Q14 = silk_SUB_SAT32(sLF_AR_shp_Q14, n_LF_Q14);
            psSampleState[k][0].LF_AR_Q14 = sLF_AR_shp_Q14;
            psSampleState[k][0].LPC_exc_Q14 = LPC_exc_Q14;
            psSampleState[k][0].xq_Q14 = xq_Q14;

            /* Update states for second best quantization */

            /* Quantized excitation */
            let mut exc_Q14 = silk_LSHIFT32(psSampleState[k][1].Q_Q10, 4);
            if psDD.Seed < 0 {
                exc_Q14 = exc_Q14.wrapping_neg();
            }

            /* Add predictions */
            let LPC_exc_Q14 = silk_ADD32(exc_Q14, LTP_pred_Q14);
            let xq_Q14 = silk_ADD32_ovflw(LPC_exc_Q14, LPC_pred_Q14);

            /* Update states */
            psSampleState[k][1].Diff_Q14 = silk_SUB32_ovflw(xq_Q14, silk_LSHIFT32(x_Q10[i as usize], 4));
            let sLF_AR_shp_Q14 = silk_SUB32_ovflw(psSampleState[k][1].Diff_Q14, n_AR_Q14);
            psSampleState[k][1].sLTP_shp_Q14 = silk_SUB_SAT32(sLF_AR_shp_Q14, n_LF_Q14);
            psSampleState[k][1].LF_AR_Q14 = sLF_AR_shp_Q14;
            psSampleState[k][1].LPC_exc_Q14 = LPC_exc_Q14;
            psSampleState[k][1].xq_Q14 = xq_Q14;
        }

        *smpl_buf_idx = (*smpl_buf_idx - 1) % DECISION_DELAY;
        if *smpl_buf_idx < 0 {
            *smpl_buf_idx += DECISION_DELAY;
        }
        let mut last_smple_idx = (*smpl_buf_idx + decisionDelay) % DECISION_DELAY;

        /* Find winner */
        let mut RDmin_Q10 = psSampleState[0][0].RD_Q10;
        let mut Winner_ind: opus_int = 0;
        for k in 1..nStatesDelayedDecision as usize {
            if psSampleState[k][0].RD_Q10 < RDmin_Q10 {
                RDmin_Q10 = psSampleState[k][0].RD_Q10;
                Winner_ind = k as opus_int;
            }
        }

        /* Increase RD values of expired states */
        let Winner_rand_state = psDelDec[Winner_ind as usize].RandState[last_smple_idx as usize];
        for k in 0..nStatesDelayedDecision as usize {
            if psDelDec[k].RandState[last_smple_idx as usize] != Winner_rand_state {
                psSampleState[k][0].RD_Q10 =
                    silk_ADD32(psSampleState[k][0].RD_Q10, silk_int32_MAX >> 4);
                psSampleState[k][1].RD_Q10 =
                    silk_ADD32(psSampleState[k][1].RD_Q10, silk_int32_MAX >> 4);
                silk_assert!(psSampleState[k][0].RD_Q10 >= 0);
            }
        }

        /* Find worst in first set and best in second set */
        let mut RDmax_Q10 = psSampleState[0][0].RD_Q10;
        let mut RDmin_Q10 = psSampleState[0][1].RD_Q10;
        let mut RDmax_ind: opus_int = 0;
        let mut RDmin_ind: opus_int = 0;
        for k in 1..nStatesDelayedDecision as usize {
            /* find worst in first set */
            if psSampleState[k][0].RD_Q10 > RDmax_Q10 {
                RDmax_Q10 = psSampleState[k][0].RD_Q10;
                RDmax_ind = k as opus_int;
            }
            /* find best in second set */
            if psSampleState[k][1].RD_Q10 < RDmin_Q10 {
                RDmin_Q10 = psSampleState[k][1].RD_Q10;
                RDmin_ind = k as opus_int;
            }
        }

        /* Replace a state if best from second set outperforms worst in first set */
        if RDmin_Q10 < RDmax_Q10 {
            /* silk_memcpy(((opus_int32 *)&psDelDec[RDmax_ind]) + i,
                           ((opus_int32 *)&psDelDec[RDmin_ind]) + i,
                           sizeof(NSQ_del_dec_struct) - i * sizeof(opus_int32));
               The struct is all-int32 and i <= length <= MAX_SUB_FRAME_LENGTH <
               MAX_SUB_FRAME_LENGTH + NSQ_LPC_BUF_LENGTH, so only sLPC_Q14 is
               partially copied; everything after it is copied whole.  When the
               two indices are equal the C memcpy is a self-copy (a no-op). */
            fn struct_copy(
                dst: &mut NSQ_del_dec_struct,
                src: &NSQ_del_dec_struct,
                from: usize,
            ) {
                for p in from..(MAX_SUB_FRAME_LENGTH + NSQ_LPC_BUF_LENGTH) as usize {
                    dst.sLPC_Q14[p] = src.sLPC_Q14[p];
                }
                dst.RandState = src.RandState;
                dst.Q_Q10 = src.Q_Q10;
                dst.Xq_Q14 = src.Xq_Q14;
                dst.Pred_Q15 = src.Pred_Q15;
                dst.Shape_Q14 = src.Shape_Q14;
                dst.sAR2_Q14 = src.sAR2_Q14;
                dst.LF_AR_Q14 = src.LF_AR_Q14;
                dst.Diff_Q14 = src.Diff_Q14;
                dst.Seed = src.Seed;
                dst.SeedInit = src.SeedInit;
                dst.RD_Q10 = src.RD_Q10;
            }
            let a = RDmax_ind as usize;
            let b = RDmin_ind as usize;
            let src_copy = psDelDec[b];
            struct_copy(&mut psDelDec[a], &src_copy, i as usize);

            psSampleState[RDmax_ind as usize][0] = psSampleState[RDmin_ind as usize][1];
        }

        /* Write samples from winner to output and long-term filter states */
        let psDD = &psDelDec[Winner_ind as usize];
        if subfr > 0 || i >= decisionDelay {
            pulses[(pulse_base + i - decisionDelay) as usize] =
                silk_RSHIFT_ROUND(psDD.Q_Q10[last_smple_idx as usize], 10) as opus_int8;
            NSQ.xq[(xq_off + i - decisionDelay) as usize] = silk_SAT16(silk_RSHIFT_ROUND(
                silk_SMULWW(
                    psDD.Xq_Q14[last_smple_idx as usize],
                    delayedGain_Q10[last_smple_idx as usize],
                ),
                8,
            )) as opus_int16;
            NSQ.sLTP_shp_Q14[(NSQ.sLTP_shp_buf_idx - decisionDelay) as usize] =
                psDD.Shape_Q14[last_smple_idx as usize];
            sLTP_Q15[(NSQ.sLTP_buf_idx - decisionDelay) as usize] =
                psDD.Pred_Q15[last_smple_idx as usize];
        }
        NSQ.sLTP_shp_buf_idx += 1;
        NSQ.sLTP_buf_idx += 1;

        /* Update states */
        for k in 0..nStatesDelayedDecision as usize {
            let psSS = &psSampleState[k][0];
            let psDD = &mut psDelDec[k];
            psDD.LF_AR_Q14 = psSS.LF_AR_Q14;
            psDD.Diff_Q14 = psSS.Diff_Q14;
            psDD.sLPC_Q14[(NSQ_LPC_BUF_LENGTH + i) as usize] = psSS.xq_Q14;
            psDD.Xq_Q14[*smpl_buf_idx as usize] = psSS.xq_Q14;
            psDD.Q_Q10[*smpl_buf_idx as usize] = psSS.Q_Q10;
            psDD.Pred_Q15[*smpl_buf_idx as usize] = silk_LSHIFT32(psSS.LPC_exc_Q14, 1);
            psDD.Shape_Q14[*smpl_buf_idx as usize] = psSS.sLTP_shp_Q14;
            psDD.Seed = silk_ADD32_ovflw(psDD.Seed, silk_RSHIFT_ROUND(psSS.Q_Q10, 10));
            psDD.RandState[*smpl_buf_idx as usize] = psDD.Seed;
            psDD.RD_Q10 = psSS.RD_Q10;
        }
        delayedGain_Q10[*smpl_buf_idx as usize] = Gain_Q10;
    }
    /* Update LPC states */
    for k in 0..nStatesDelayedDecision as usize {
        let psDD = &mut psDelDec[k];
        for i in 0..NSQ_LPC_BUF_LENGTH as usize {
            psDD.sLPC_Q14[i] = psDD.sLPC_Q14[i + length as usize];
        }
    }
}

/// `silk_NSQ_del_dec_c`
#[allow(clippy::too_many_arguments)]
pub fn silk_NSQ_del_dec_c(
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
    let mut lag: opus_int;
    let mut start_idx: opus_int;
    let LSF_interpolation_flag: opus_int;
    let mut HarmShapeFIRPacked_Q14: opus_int32;
    let mut subfr: opus_int;

    /* Set unvoiced lag to the previous one, overwrite later for voiced */
    lag = NSQ.lagPrev;

    silk_assert!(NSQ.prev_gain_Q16 != 0);

    /* Initialize delayed decision states */
    let mut psDelDec = vec![NSQ_del_dec_struct::default(); psEncC.nStatesDelayedDecision as usize];
    for k in 0..psEncC.nStatesDelayedDecision as usize {
        let psDD = &mut psDelDec[k];
        psDD.Seed = ((k as opus_int + psIndices.Seed as opus_int) & 3) as opus_int32;
        psDD.SeedInit = psDD.Seed;
        psDD.RD_Q10 = 0;
        psDD.LF_AR_Q14 = NSQ.sLF_AR_shp_Q14;
        psDD.Diff_Q14 = NSQ.sDiff_shp_Q14;
        psDD.Shape_Q14[0] = NSQ.sLTP_shp_Q14[(psEncC.ltp_mem_length - 1) as usize];
        for i in 0..NSQ_LPC_BUF_LENGTH as usize {
            psDD.sLPC_Q14[i] = NSQ.sLPC_Q14[i];
        }
        for i in 0..MAX_SHAPE_LPC_ORDER as usize {
            psDD.sAR2_Q14[i] = NSQ.sAR2_Q14[i];
        }
    }

    let offset_Q10 =
        silk_Quantization_Offsets_Q10[(psIndices.signalType >> 1) as usize][psIndices.quantOffsetType as usize]
            as opus_int;
    let mut smpl_buf_idx: opus_int = 0; /* index of oldest samples */

    let mut decisionDelay = silk_min_int(DECISION_DELAY, psEncC.subfr_length);

    /* For voiced frames limit the decision delay to lower than the pitch lag */
    if psIndices.signalType as opus_int == TYPE_VOICED {
        for k in 0..psEncC.nb_subfr {
            decisionDelay = silk_min_int(decisionDelay, pitchL[k as usize] - LTP_ORDER / 2 - 1);
        }
    } else if lag > 0 {
        decisionDelay = silk_min_int(decisionDelay, lag - LTP_ORDER / 2 - 1);
    }

    if psIndices.NLSFInterpCoef_Q2 == 4 {
        LSF_interpolation_flag = 0;
    } else {
        LSF_interpolation_flag = 1;
    }

    let alloc_len = (psEncC.ltp_mem_length + psEncC.frame_length) as usize;
    let mut sLTP_Q15 = vec![0 as opus_int32; alloc_len];
    let mut sLTP = vec![0 as opus_int16; alloc_len];
    let mut x_sc_Q10 = vec![0 as opus_int32; psEncC.subfr_length as usize];
    let mut delayedGain_Q10 = vec![0 as opus_int32; DECISION_DELAY as usize];
    /* Set up pointers to start of sub frame */
    let pxq_off = psEncC.ltp_mem_length;
    NSQ.sLTP_shp_buf_idx = psEncC.ltp_mem_length;
    NSQ.sLTP_buf_idx = psEncC.ltp_mem_length;
    subfr = 0;
    for k in 0..psEncC.nb_subfr {
        let A_Q12 = &PredCoef_Q12[((k >> 1) | (1 - LSF_interpolation_flag)) as usize];
        let B_Q14 = &LTPCoef_Q14[(k * LTP_ORDER) as usize..];
        let AR_shp_Q13 = &AR_Q13[(k * MAX_SHAPE_LPC_ORDER) as usize..];

        /* Noise shape parameters */
        silk_assert!(HarmShapeGain_Q14[k as usize] >= 0);
        HarmShapeFIRPacked_Q14 = silk_RSHIFT(HarmShapeGain_Q14[k as usize], 2);
        HarmShapeFIRPacked_Q14 |= silk_LSHIFT(silk_RSHIFT(HarmShapeGain_Q14[k as usize], 1), 16);

        NSQ.rewhite_flag = 0;
        if psIndices.signalType as opus_int == TYPE_VOICED {
            /* Voiced */
            lag = pitchL[k as usize];

            /* Re-whitening */
            if (k & (3 - silk_LSHIFT(LSF_interpolation_flag, 1))) == 0 {
                if k == 2 {
                    /* RESET DELAYED DECISIONS */
                    /* Find winner */
                    let mut RDmin_Q10 = psDelDec[0].RD_Q10;
                    let mut Winner_ind: opus_int = 0;
                    for i in 1..psEncC.nStatesDelayedDecision as usize {
                        if psDelDec[i].RD_Q10 < RDmin_Q10 {
                            RDmin_Q10 = psDelDec[i].RD_Q10;
                            Winner_ind = i as opus_int;
                        }
                    }
                    for i in 0..psEncC.nStatesDelayedDecision as usize {
                        if i as opus_int != Winner_ind {
                            psDelDec[i].RD_Q10 += silk_int32_MAX >> 4;
                            silk_assert!(psDelDec[i].RD_Q10 >= 0);
                        }
                    }

                    /* Copy final part of signals from winner state to output and long-term filter states */
                    let mut last_smple_idx = smpl_buf_idx + decisionDelay;
                    for i in 0..decisionDelay {
                        last_smple_idx = (last_smple_idx - 1) % DECISION_DELAY;
                        if last_smple_idx < 0 {
                            last_smple_idx += DECISION_DELAY;
                        }
                        pulses[(k * psEncC.subfr_length + i - decisionDelay) as usize] =
                            silk_RSHIFT_ROUND(
                                psDelDec[Winner_ind as usize].Q_Q10[last_smple_idx as usize],
                                10,
                            ) as opus_int8;
                        NSQ.xq[(pxq_off + k * psEncC.subfr_length + i - decisionDelay) as usize] =
                            silk_SAT16(silk_RSHIFT_ROUND(
                                silk_SMULWW(
                                    psDelDec[Winner_ind as usize].Xq_Q14[last_smple_idx as usize],
                                    Gains_Q16[1],
                                ),
                                14,
                            )) as opus_int16;
                        NSQ.sLTP_shp_Q14
                            [(NSQ.sLTP_shp_buf_idx - decisionDelay + i) as usize] =
                            psDelDec[Winner_ind as usize].Shape_Q14[last_smple_idx as usize];
                    }

                    subfr = 0;
                }

                /* Rewhiten with new A coefs */
                start_idx = psEncC.ltp_mem_length - lag - psEncC.predictLPCOrder - LTP_ORDER / 2;
                celt_assert!(start_idx > 0);

                let in_off = start_idx + k * psEncC.subfr_length;
                silk_LPC_analysis_filter_import(
                    &mut sLTP[start_idx as usize..],
                    &NSQ.xq[in_off as usize..],
                    A_Q12,
                    psEncC.ltp_mem_length - start_idx,
                    psEncC.predictLPCOrder,
                    psEncC.arch,
                );

                NSQ.sLTP_buf_idx = psEncC.ltp_mem_length;
                NSQ.rewhite_flag = 1;
            }
        }

        silk_NSQ_del_dec_scale_states(
            psEncC,
            NSQ,
            &mut psDelDec,
            &x16[(k * psEncC.subfr_length) as usize..],
            &mut x_sc_Q10,
            &sLTP,
            &mut sLTP_Q15,
            k,
            psEncC.nStatesDelayedDecision,
            LTP_scale_Q14,
            Gains_Q16,
            pitchL,
            psIndices.signalType as opus_int,
            decisionDelay,
        );

        silk_noise_shape_quantizer_del_dec(
            NSQ,
            &mut psDelDec,
            psIndices.signalType as opus_int,
            &x_sc_Q10,
            pulses,
            k * psEncC.subfr_length,
            pxq_off + k * psEncC.subfr_length,
            &mut sLTP_Q15,
            &mut delayedGain_Q10,
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
            subfr,
            psEncC.shapingLPCOrder,
            psEncC.predictLPCOrder,
            psEncC.warping_Q16,
            psEncC.nStatesDelayedDecision,
            &mut smpl_buf_idx,
            decisionDelay,
            psEncC.arch,
        );
        subfr += 1;
    }

    /* Find winner */
    let mut RDmin_Q10 = psDelDec[0].RD_Q10;
    let mut Winner_ind: opus_int = 0;
    for k in 1..psEncC.nStatesDelayedDecision as usize {
        if psDelDec[k].RD_Q10 < RDmin_Q10 {
            RDmin_Q10 = psDelDec[k].RD_Q10;
            Winner_ind = k as opus_int;
        }
    }

    /* Copy final part of signals from winner state to output and long-term filter states */
    psIndices.Seed = psDelDec[Winner_ind as usize].SeedInit as opus_int8;
    let mut last_smple_idx = smpl_buf_idx + decisionDelay;
    let Gain_Q10 = silk_RSHIFT32(Gains_Q16[(psEncC.nb_subfr - 1) as usize], 6);
    for i in 0..decisionDelay {
        last_smple_idx = (last_smple_idx - 1) % DECISION_DELAY;
        if last_smple_idx < 0 {
            last_smple_idx += DECISION_DELAY;
        }

        pulses[(psEncC.nb_subfr * psEncC.subfr_length + i - decisionDelay) as usize] =
            silk_RSHIFT_ROUND(
                psDelDec[Winner_ind as usize].Q_Q10[last_smple_idx as usize],
                10,
            ) as opus_int8;
        NSQ.xq[(pxq_off + psEncC.nb_subfr * psEncC.subfr_length + i - decisionDelay) as usize] = silk_SAT16(silk_RSHIFT_ROUND(
            silk_SMULWW(
                psDelDec[Winner_ind as usize].Xq_Q14[last_smple_idx as usize],
                Gain_Q10,
            ),
            8,
        )) as opus_int16;
        NSQ.sLTP_shp_Q14[(NSQ.sLTP_shp_buf_idx - decisionDelay + i) as usize] =
            psDelDec[Winner_ind as usize].Shape_Q14[last_smple_idx as usize];
    }
    for i in 0..NSQ_LPC_BUF_LENGTH as usize {
        NSQ.sLPC_Q14[i] = psDelDec[Winner_ind as usize].sLPC_Q14[i + psEncC.subfr_length as usize];
    }
    NSQ.sAR2_Q14 = psDelDec[Winner_ind as usize].sAR2_Q14;

    /* Update states */
    NSQ.sLF_AR_shp_Q14 = psDelDec[Winner_ind as usize].LF_AR_Q14;
    NSQ.sDiff_shp_Q14 = psDelDec[Winner_ind as usize].Diff_Q14;
    NSQ.lagPrev = pitchL[(psEncC.nb_subfr - 1) as usize];

    /* Save quantized speech signal */
    let fl = psEncC.frame_length as usize;
    let lml = psEncC.ltp_mem_length as usize;
    NSQ.xq.copy_within(fl..fl + lml, 0);
    NSQ.sLTP_shp_Q14.copy_within(fl..fl + lml, 0);
}

/* Small indirection so the C-body port and the AVX2 port share the same
   LPC-analysis-filter call. */
#[inline(always)]
#[allow(clippy::too_many_arguments)]
fn silk_LPC_analysis_filter_import(
    out: &mut [opus_int16],
    in_: &[opus_int16],
    B: &[opus_int16],
    len: opus_int32,
    d: opus_int32,
    arch: opus_int,
) {
    crate::silk::lpc_analysis_filter::silk_LPC_analysis_filter(out, in_, B, len, d, arch);
}

// ===========================================================================
// `silk/x86/NSQ_del_dec_avx2.c`
// ===========================================================================

/// `__m128i` — four 32-bit lanes (one per delayed-decision state).
pub type M128 = [opus_int32; 4];
/// `__m256i` — eight 32-bit lanes (two candidate sets of four states).
pub type M256 = [opus_int32; 8];

#[inline(always)]
fn mm_set1(v: opus_int32) -> M128 {
    [v; 4]
}
#[inline(always)]
fn mm256_set1(v: opus_int32) -> M256 {
    [v; 8]
}
#[inline(always)]
fn mm_add(a: M128, b: M128) -> M128 {
    [
        a[0].wrapping_add(b[0]),
        a[1].wrapping_add(b[1]),
        a[2].wrapping_add(b[2]),
        a[3].wrapping_add(b[3]),
    ]
}
#[inline(always)]
fn mm_sub(a: M128, b: M128) -> M128 {
    [
        a[0].wrapping_sub(b[0]),
        a[1].wrapping_sub(b[1]),
        a[2].wrapping_sub(b[2]),
        a[3].wrapping_sub(b[3]),
    ]
}
#[inline(always)]
fn mm256_add(a: M256, b: M256) -> M256 {
    let mut r = [0i32; 8];
    for i in 0..8 {
        r[i] = a[i].wrapping_add(b[i]);
    }
    r
}
#[inline(always)]
fn mm256_sub(a: M256, b: M256) -> M256 {
    let mut r = [0i32; 8];
    for i in 0..8 {
        r[i] = a[i].wrapping_sub(b[i]);
    }
    r
}
#[inline(always)]
fn mm_slli(a: M128, n: u32) -> M128 {
    [
        ((a[0] as u32) << n) as i32,
        ((a[1] as u32) << n) as i32,
        ((a[2] as u32) << n) as i32,
        ((a[3] as u32) << n) as i32,
    ]
}
#[inline(always)]
fn mm256_slli(a: M256, n: u32) -> M256 {
    let mut r = [0i32; 8];
    for i in 0..8 {
        r[i] = ((a[i] as u32) << n) as i32;
    }
    r
}
#[inline(always)]
fn mm_srai(a: M128, n: u32) -> M128 {
    [a[0] >> n, a[1] >> n, a[2] >> n, a[3] >> n]
}
#[inline(always)]
fn mm256_srai(a: M256, n: u32) -> M256 {
    let mut r = [0i32; 8];
    for i in 0..8 {
        r[i] = a[i] >> n;
    }
    r
}
#[inline(always)]
fn mm_srli(a: M128, n: u32) -> M128 {
    [
        ((a[0] as u32) >> n) as i32,
        ((a[1] as u32) >> n) as i32,
        ((a[2] as u32) >> n) as i32,
        ((a[3] as u32) >> n) as i32,
    ]
}
#[inline(always)]
fn mm_abs(a: M128) -> M128 {
    [a[0].wrapping_abs(), a[1].wrapping_abs(), a[2].wrapping_abs(), a[3].wrapping_abs()]
}
#[inline(always)]
fn mm256_abs(a: M256) -> M256 {
    let mut r = [0i32; 8];
    for i in 0..8 {
        r[i] = a[i].wrapping_abs();
    }
    r
}
#[inline(always)]
fn mm_cmplt(a: M128, b: M128) -> M128 {
    let mut r = [0i32; 4];
    for i in 0..4 {
        r[i] = if a[i] < b[i] { -1 } else { 0 };
    }
    r
}
#[inline(always)]
fn mm_cmpgt(a: M128, b: M128) -> M128 {
    let mut r = [0i32; 4];
    for i in 0..4 {
        r[i] = if a[i] > b[i] { -1 } else { 0 };
    }
    r
}
#[inline(always)]
fn mm_cmpeq(a: M128, b: M128) -> M128 {
    let mut r = [0i32; 4];
    for i in 0..4 {
        r[i] = if a[i] == b[i] { -1 } else { 0 };
    }
    r
}
/// `_mm_blendv_epi8(a, b, mask)`: byte-wise select of `b` where the mask byte
/// has its high bit set.  Since every operand here is a full 32-bit lane of
/// all-ones/all-zeros, this is a lane-wise select.
#[inline(always)]
fn mm_blendv(a: M128, b: M128, mask: M128) -> M128 {
    let mut r = [0i32; 4];
    for i in 0..4 {
        r[i] = if mask[i] < 0 { b[i] } else { a[i] };
    }
    r
}
#[inline(always)]
fn mm256_blendv(a: M256, b: M256, mask: M256) -> M256 {
    let mut r = [0i32; 8];
    for i in 0..8 {
        r[i] = if mask[i] < 0 { b[i] } else { a[i] };
    }
    r
}
/// `silk_cvtepi64_epi32_high`
#[inline(always)]
fn silk_cvtepi64_epi32_high(num: [i64; 4]) -> M128 {
    [
        (num[0] >> 32) as i32,
        (num[1] >> 32) as i32,
        (num[2] >> 32) as i32,
        (num[3] >> 32) as i32,
    ]
}
#[inline(always)]
fn silk_sat16(num: opus_int32) -> opus_int16 {
    let num = if num > silk_int16_MAX as opus_int32 {
        silk_int16_MAX as opus_int32
    } else {
        num
    };
    let num = if num < silk_int16_MIN as opus_int32 {
        silk_int16_MIN as opus_int32
    } else {
        num
    };
    num as opus_int16
}
/// `silk_sar_round_32`
#[inline(always)]
fn silk_sar_round_32(a: opus_int32, bits: i32) -> opus_int32 {
    silk_assert!(bits > 0 && bits < 31);
    let a = a.wrapping_add(1i32 << (bits - 1));
    a >> bits
}
/// `silk_sar_round_smulww` — the `OPUS_CHECK_ASM` branch is *not* compiled in
/// the reference build, so this is the int64 form that omits the C's
/// intermediate int32 truncation (the source comment calls this "more
/// correct" but different on rare overflow).
#[inline(always)]
fn silk_sar_round_smulww(a: opus_int32, b: opus_int32, bits: i32) -> i64 {
    silk_assert!(bits > 0 && bits < 63);
    let mut t = (a as i64) * (b as i64);
    let bits = bits + 16;
    t = t.wrapping_add(1i64 << (bits - 1));
    t >> bits
}
/// `silk_add_sat32` (`__builtin_sadd_overflow` semantics)
#[inline(always)]
fn silk_add_sat32(a: opus_int32, b: opus_int32) -> opus_int32 {
    match a.checked_add(b) {
        Some(sum) => sum,
        None => {
            if a >= 0 {
                silk_int32_MAX
            } else {
                silk_int32_MIN
            }
        }
    }
}
/// `silk_mm_srai_round_epi32`
#[inline(always)]
fn mm_srai_round(a: M128, bits: i32) -> M128 {
    silk_assert!(bits > 0 && bits < 31);
    mm_srai(mm_add(a, mm_set1(1i32 << (bits - 1))), bits as u32)
}
#[inline(always)]
fn mm_add_sat(a: M128, b: M128) -> M128 {
    let mut r = [0i32; 4];
    for i in 0..4 {
        r[i] = silk_add_sat32(a[i], b[i]);
    }
    r
}
#[inline(always)]
fn mm_sub_sat(a: M128, b: M128) -> M128 {
    let mut r = [0i32; 4];
    for i in 0..4 {
        r[i] = match a[i].checked_sub(b[i]) {
            Some(v) => v,
            None => {
                if a[i] >= 0 {
                    silk_int32_MAX
                } else {
                    silk_int32_MIN
                }
            }
        };
    }
    r
}
#[inline(always)]
fn mm256_sub_sat(a: M256, b: M256) -> M256 {
    let mut r = [0i32; 8];
    for i in 0..8 {
        r[i] = match a[i].checked_sub(b[i]) {
            Some(v) => v,
            None => {
                if a[i] >= 0 {
                    silk_int32_MAX
                } else {
                    silk_int32_MIN
                }
            }
        };
    }
    r
}
#[inline(always)]
fn mm_limit(num: M128, limit1: opus_int32, limit2: opus_int32) -> M128 {
    let lo = if limit1 < limit2 { limit1 } else { limit2 };
    let hi = if limit1 > limit2 { limit1 } else { limit2 };
    let mut num = num;
    for i in 0..4 {
        num[i] = num[i].min(hi);
    }
    for i in 0..4 {
        num[i] = num[i].max(lo);
    }
    num
}
/// `_mm_sign_epi32` — `b < 0 ? -a : (b == 0 ? 0 : a)`
#[inline(always)]
fn mm_sign_epi32(a: M128, b: M128) -> M128 {
    let mut r = [0i32; 4];
    for i in 0..4 {
        r[i] = if b[i] < 0 {
            a[i].wrapping_neg()
        } else if b[i] == 0 {
            0
        } else {
            a[i]
        };
    }
    r
}
/// `silk_mm_sign_epi32` — `cond < 0 ? -num : num`
#[inline(always)]
fn mm_sign(num: M128, cond: M128) -> M128 {
    let mut r = [0i32; 4];
    for i in 0..4 {
        r[i] = if cond[i] < 0 { num[i].wrapping_neg() } else { num[i] };
    }
    r
}
#[inline(always)]
fn mm256_sign(num: M256, cond: M256) -> M256 {
    let mut r = [0i32; 8];
    for i in 0..8 {
        r[i] = if cond[i] < 0 { num[i].wrapping_neg() } else { num[i] };
    }
    r
}
/// `silk_mm_smulww_epi32` — `(a32 * b32) >> 16`
#[inline(always)]
fn mm_smulww(a: M128, b: opus_int32) -> M128 {
    let mut r = [0i32; 4];
    for i in 0..4 {
        r[i] = (((a[i] as i64) * (b as i64)) >> 16) as i32;
    }
    r
}
/// `silk_mm_smulwb_epi32` — `(a32 * (opus_int32)((opus_int16)(b32))) >> 16`
#[inline(always)]
fn mm_smulwb(a: M128, b: opus_int32) -> M128 {
    let mut r = [0i32; 4];
    for i in 0..4 {
        r[i] = (((a[i] as i64) * (b as i16 as i64)) >> 16) as i32;
    }
    r
}
/// `silk_mm256_smulbb_epi32` — `(opus_int32)((opus_int16)a) * (opus_int32)((opus_int16)b)`
#[inline(always)]
fn mm256_smulbb(a: M256, b: M256) -> M256 {
    let mut r = [0i32; 8];
    for i in 0..8 {
        r[i] = (a[i] as i16 as i32).wrapping_mul(b[i] as i16 as i32);
    }
    r
}
/// `silk_mm256_reverse_epi32`
#[inline(always)]
fn mm256_reverse(v: M256) -> M256 {
    [v[7], v[6], v[5], v[4], v[3], v[2], v[1], v[0]]
}
/// `silk_mm256_hsum_epi32`
#[inline(always)]
fn mm256_hsum(v: M256) -> opus_int32 {
    let s0 = v[0].wrapping_add(v[4]);
    let s1 = v[1].wrapping_add(v[5]);
    let s2 = v[2].wrapping_add(v[6]);
    let s3 = v[3].wrapping_add(v[7]);
    let t0 = s0.wrapping_add(s2);
    let t1 = s1.wrapping_add(s3);
    t0.wrapping_add(t1)
}
/// `silk_mm_hmin_epi32` / `silk_mm_hmax_epi32` (result is broadcast).
#[inline(always)]
fn mm_hmin(num: M128) -> M128 {
    let mut num = num;
    let sh = [num[2], num[3], num[0], num[1]];
    for i in 0..4 {
        num[i] = num[i].min(sh[i]);
    }
    let sh = [num[1], num[0], num[3], num[2]];
    for i in 0..4 {
        num[i] = num[i].min(sh[i]);
    }
    num
}
#[inline(always)]
fn mm_hmax(num: M128) -> M128 {
    let mut num = num;
    let sh = [num[2], num[3], num[0], num[1]];
    for i in 0..4 {
        num[i] = num[i].max(sh[i]);
    }
    let sh = [num[1], num[0], num[3], num[2]];
    for i in 0..4 {
        num[i] = num[i].max(sh[i]);
    }
    num
}
/// `silk_mm_mask_hmin_epi32`
#[inline(always)]
fn mm_mask_hmin(num: M128, mask: M128) -> M128 {
    mm_hmin(mm_blendv(num, mm_set1(silk_int32_MAX), mask))
}
/// `silk_mm_mask_hmax_epi32`
#[inline(always)]
fn mm_mask_hmax(num: M128, mask: M128) -> M128 {
    mm_hmax(mm_blendv(num, mm_set1(silk_int32_MIN), mask))
}
/// `silk_mm256_rand_epi32` (`silk_RAND` per lane)
#[inline(always)]
fn mm256_rand(seed: M128) -> M128 {
    let mut r = [0i32; 4];
    for i in 0..4 {
        r[i] = seed[i].wrapping_mul(RAND_MULTIPLIER).wrapping_add(RAND_INCREMENT);
    }
    r
}
/// `silk_index_of_first_equal_epi32`
#[inline(always)]
fn index_of_first_equal(a: M128, b: M128) -> opus_int32 {
    let mut mask: u32 = 0;
    for i in 0..4 {
        if a[i] == b[i] {
            mask |= 1 << (4 * i);
        }
    }
    silk_assert!(mask != 0);
    (mask.trailing_zeros() >> 2) as opus_int32
}
/// `silk_mm256_permute2x128_si256(x, x, 0x1)`
#[inline(always)]
fn mm256_swap128(x: M256) -> M256 {
    [x[4], x[5], x[6], x[7], x[0], x[1], x[2], x[3]]
}
/// `_mm256_broadcastsi128_si256`
#[inline(always)]
fn mm256_broadcast(x: M128) -> M256 {
    [x[0], x[1], x[2], x[3], x[0], x[1], x[2], x[3]]
}
/// `_mm_cvtepi8_epi32(_mm_cvtsi32_si128(0xFFU << (idx << 3)))`: lane `idx` is
/// -1, all other lanes 0.
#[inline(always)]
fn mask_lane(idx: opus_int32) -> M128 {
    let v: u32 = 0xFFu32 << (idx << 3);
    let bytes = v.to_le_bytes();
    [
        bytes[0] as i8 as i32,
        bytes[1] as i8 as i32,
        bytes[2] as i8 as i32,
        bytes[3] as i8 as i32,
    ]
}
/// `MaskDelDec = _mm_cvtepi8_epi32(_mm_cvtsi32_si128(0xFFFFFF00ul << ((nStates-1)<<3)))`.
/// The C shifts a *64-bit* `unsigned long` and `_mm_cvtsi32_si128` then keeps
/// only the low 32 bits, so the mask is `[0; nStates] ++ [-1; 4 - nStates]`:
/// `nStates == 4` masks nothing, 3 masks lane 3, 2 masks lanes 2..3 and 1 masks
/// lanes 1..3 (verified against the C macro on the host toolchain).
#[inline(always)]
fn mask_del_dec(nStates: opus_int) -> M128 {
    let v: u32 = ((0xFFFF_FF00u64 << ((nStates - 1) << 3)) & 0xFFFF_FFFF) as u32;
    let bytes = v.to_le_bytes();
    [
        bytes[0] as i8 as i32,
        bytes[1] as i8 as i32,
        bytes[2] as i8 as i32,
        bytes[3] as i8 as i32,
    ]
}

/// `silk_noise_shape_quantizer_short_prediction_x4`
#[inline(always)]
fn short_prediction_x4(buf32: &[M128], pos: opus_int, coef16: &[opus_int16], order: opus_int) -> M128 {
    silk_assert!(order == 10 || order == 16);
    let mut out = mm_set1(order >> 1);
    let mut j: opus_int = 0;
    while j < order {
        out = mm_add(out, mm_smulwb(buf32[(pos - j) as usize], coef16[j as usize] as opus_int32));
        j += 1;
    }
    out
}

/// `silk_LPC_analysis_filter_avx2` (file-local in the AVX2 translation unit).
#[allow(clippy::too_many_arguments)]
fn silk_LPC_analysis_filter_avx2(
    out: &mut [opus_int16],
    in_: &[opus_int16],
    B: &[opus_int16],
    len: opus_int32,
    order: opus_int32,
) {
    silk_assert!(order == 10 || order == 16);

    for i in order..len {
        let in_ptr = i as usize;
        let mut sum: M256 = [0i32; 8];
        /* order > 10: two groups of 8 taps; order == 10: 2 loaded taps + shuffle */
        {
            let mut in_v = [0i32; 8];
            let mut b_v = [0i32; 8];
            for j in 0..8 {
                in_v[j] = in_[in_ptr - 8 + j] as opus_int32;
                b_v[j] = B[j] as opus_int32;
            }
            let r = mm256_reverse(b_v);
            for j in 0..8 {
                sum[j] = in_v[j].wrapping_mul(r[j]);
            }
        }
        if order > 10 {
            let mut in_v = [0i32; 8];
            let mut b_v = [0i32; 8];
            for j in 0..8 {
                in_v[j] = in_[in_ptr - 16 + j] as opus_int32;
                b_v[j] = B[8 + j] as opus_int32;
            }
            let r = mm256_reverse(b_v);
            for j in 0..8 {
                sum[j] = sum[j].wrapping_add(in_v[j].wrapping_mul(r[j]));
            }
        } else {
            /* _mm_loadu_si32 zero-extends the other lanes */
            let in_v = [
                in_[in_ptr - 10] as opus_int32,
                in_[in_ptr - 9] as opus_int32,
                0,
                0,
                0,
                0,
                0,
                0,
            ];
            /* shuffle_epi32(B_v, 0x01) on [B8,B9,0,0,0,0,0,0] gives [B9,B8,B8,B8,0,0,0,0] */
            let b_v = [B[9] as opus_int32, B[8] as opus_int32, B[8] as opus_int32, B[8] as opus_int32, 0, 0, 0, 0];
            for j in 0..8 {
                sum[j] = sum[j].wrapping_add(in_v[j].wrapping_mul(b_v[j]));
            }
        }

        let mut out32_Q12 = mm256_hsum(sum);

        /* Subtract prediction */
        out32_Q12 = silk_SUB32_ovflw(silk_LSHIFT(in_[in_ptr] as opus_int32, 12), out32_Q12);

        /* Scale to Q0 */
        let out32 = silk_sar_round_32(out32_Q12, 12);

        /* Saturate output */
        out[in_ptr] = silk_sat16(out32);
    }

    /* Set first d output samples to zero */
    for i in 0..order as usize {
        out[i] = 0;
    }
}

/// `NSQ_del_dec_sample_struct` (AVX2, vertically coalesced over the 4 states)
#[derive(Clone, Copy)]
pub struct NSQ_del_dec_sample_struct {
    pub RandState: M128,
    pub Q_Q10: M128,
    pub Xq_Q14: M128,
    pub Pred_Q15: M128,
    pub Shape_Q14: M128,
}
impl Default for NSQ_del_dec_sample_struct {
    fn default() -> Self {
        NSQ_del_dec_sample_struct {
            RandState: [0; 4],
            Q_Q10: [0; 4],
            Xq_Q14: [0; 4],
            Pred_Q15: [0; 4],
            Shape_Q14: [0; 4],
        }
    }
}

/// `NSQ_del_dec_struct` (AVX2)
#[derive(Clone, Copy)]
pub struct NSQ_del_dec_struct_avx2 {
    pub sLPC_Q14: [M128; (MAX_SUB_FRAME_LENGTH + NSQ_LPC_BUF_LENGTH) as usize],
    pub LF_AR_Q14: M128,
    pub Seed: M128,
    pub SeedInit: M128,
    pub RD_Q10: M128,
    pub Diff_Q14: M128,
    pub sAR2_Q14: [M128; MAX_SHAPE_LPC_ORDER as usize],
    pub Samples: [NSQ_del_dec_sample_struct; DECISION_DELAY as usize],
}
impl Default for NSQ_del_dec_struct_avx2 {
    fn default() -> Self {
        NSQ_del_dec_struct_avx2 {
            sLPC_Q14: [[0; 4]; (MAX_SUB_FRAME_LENGTH + NSQ_LPC_BUF_LENGTH) as usize],
            LF_AR_Q14: [0; 4],
            Seed: [0; 4],
            SeedInit: [0; 4],
            RD_Q10: [0; 4],
            Diff_Q14: [0; 4],
            sAR2_Q14: [[0; 4]; MAX_SHAPE_LPC_ORDER as usize],
            Samples: [NSQ_del_dec_sample_struct::default(); DECISION_DELAY as usize],
        }
    }
}

/// `silk_nsq_del_dec_scale_states_avx2`
#[allow(clippy::too_many_arguments)]
fn silk_nsq_del_dec_scale_states_avx2(
    psEncC: &silk_encoder_state,
    NSQ: &mut silk_nsq_state,
    psDelDec: &mut NSQ_del_dec_struct_avx2,
    x16: &[opus_int16],
    x_sc_Q10: &mut [opus_int32],
    sLTP: &[opus_int16],
    sLTP_Q15: &mut [opus_int32],
    subfr: opus_int,
    LTP_scale_Q14: opus_int,
    Gains_Q16: &[opus_int32],
    pitchL: &[opus_int],
    signal_type: opus_int,
    decisionDelay: opus_int,
) {
    let lag = pitchL[subfr as usize];
    let mut inv_gain_Q31 = silk_INVERSE32_varQ(silk_max(Gains_Q16[subfr as usize], 1), 47);
    silk_assert!(inv_gain_Q31 != 0);

    /* Scale input (vectorised as exact silk_SMULWW, 4 lanes at a time) */
    let inv_gain_Q26 = silk_sar_round_32(inv_gain_Q31, 5);
    let mut i: opus_int = 0;
    while i < psEncC.subfr_length {
        for j in 0..4 {
            x_sc_Q10[(i + j) as usize] =
                silk_SMULWW(x16[(i + j) as usize] as opus_int32, inv_gain_Q26);
        }
        i += 4;
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

        /* Scale long-term shaping state (4 lanes at a time; exact SMULWW) */
        let mut i = NSQ.sLTP_shp_buf_idx - psEncC.ltp_mem_length;
        while i < NSQ.sLTP_shp_buf_idx {
            NSQ.sLTP_shp_Q14[i as usize] = silk_SMULWW(gain_adj_Q16, NSQ.sLTP_shp_Q14[i as usize]);
            i += 1;
        }

        /* Scale long-term prediction state */
        if signal_type == TYPE_VOICED && NSQ.rewhite_flag == 0 {
            let mut i = NSQ.sLTP_buf_idx - lag - LTP_ORDER / 2;
            while i < NSQ.sLTP_buf_idx - decisionDelay {
                sLTP_Q15[i as usize] =
                    (((sLTP_Q15[i as usize] as i64) * (gain_adj_Q16 as i64)) >> 16) as opus_int32;
                i += 1;
            }
        }

        /* Scale scalar states */
        psDelDec.LF_AR_Q14 = mm_smulww(psDelDec.LF_AR_Q14, gain_adj_Q16);
        psDelDec.Diff_Q14 = mm_smulww(psDelDec.Diff_Q14, gain_adj_Q16);

        /* Scale short-term prediction and shaping states */
        for i in 0..NSQ_LPC_BUF_LENGTH as usize {
            psDelDec.sLPC_Q14[i] = mm_smulww(psDelDec.sLPC_Q14[i], gain_adj_Q16);
        }
        for i in 0..DECISION_DELAY as usize {
            psDelDec.Samples[i].Pred_Q15 = mm_smulww(psDelDec.Samples[i].Pred_Q15, gain_adj_Q16);
            psDelDec.Samples[i].Shape_Q14 = mm_smulww(psDelDec.Samples[i].Shape_Q14, gain_adj_Q16);
        }
        for i in 0..MAX_SHAPE_LPC_ORDER as usize {
            psDelDec.sAR2_Q14[i] = mm_smulww(psDelDec.sAR2_Q14[i], gain_adj_Q16);
        }

        /* Save inverse gain */
        NSQ.prev_gain_Q16 = Gains_Q16[subfr as usize];
    }
}

/// `silk_noise_shape_quantizer_del_dec_avx2`
#[allow(clippy::too_many_arguments)]
fn silk_noise_shape_quantizer_del_dec_avx2(
    NSQ: &mut silk_nsq_state,
    psDelDec: &mut NSQ_del_dec_struct_avx2,
    signalType: opus_int,
    x_Q10: &[opus_int32],
    pulses: &mut [opus_int8],
    pulse_base: opus_int,
    xq_off: opus_int,
    sLTP_Q15: &mut [opus_int32],
    delayedGain_Q10: &mut [opus_int32],
    a_Q12: &[opus_int16],
    b_Q14: &[opus_int16],
    AR_shp_Q13: &[opus_int16],
    lag: opus_int,
    HarmShapeFIRPacked_Q14: opus_int32,
    Tilt_Q14: opus_int,
    LF_shp_Q14: opus_int32,
    Gain_Q16: opus_int32,
    Lambda_Q10: opus_int,
    offset_Q10: opus_int,
    length: opus_int,
    subfr: opus_int,
    shapingLPCOrder: opus_int,
    predictLPCOrder: opus_int,
    warping_Q16: opus_int,
    MaskDelDec: M128,
    smpl_buf_idx: &mut opus_int,
    decisionDelay: opus_int,
) {
    let mut shp_lag_ptr: opus_int = NSQ.sLTP_shp_buf_idx - lag + HARM_SHAPE_FIR_TAPS / 2;
    let mut pred_lag_ptr: opus_int = NSQ.sLTP_buf_idx - lag + LTP_ORDER / 2;
    let Gain_Q10: opus_int32 = Gain_Q16 >> 6;

    for i in 0..length {
        /* Long-term prediction */
        let mut LTP_pred_Q14: opus_int32;
        if signalType == TYPE_VOICED {
            /* Unrolled loop */
            /* Avoids introducing a bias because silk_SMLAWB() always rounds to -inf */
            LTP_pred_Q14 = 2;
            LTP_pred_Q14 += silk_SMULWB(sLTP_Q15[(pred_lag_ptr - 0) as usize], b_Q14[0] as opus_int32);
            LTP_pred_Q14 += silk_SMULWB(sLTP_Q15[(pred_lag_ptr - 1) as usize], b_Q14[1] as opus_int32);
            LTP_pred_Q14 += silk_SMULWB(sLTP_Q15[(pred_lag_ptr - 2) as usize], b_Q14[2] as opus_int32);
            LTP_pred_Q14 += silk_SMULWB(sLTP_Q15[(pred_lag_ptr - 3) as usize], b_Q14[3] as opus_int32);
            LTP_pred_Q14 += silk_SMULWB(sLTP_Q15[(pred_lag_ptr - 4) as usize], b_Q14[4] as opus_int32);
            LTP_pred_Q14 = silk_LSHIFT(LTP_pred_Q14, 1); /* Q13 -> Q14 */
            pred_lag_ptr += 1;
        } else {
            LTP_pred_Q14 = 0;
        }

        /* Long-term shaping */
        let mut n_LTP_Q14: opus_int32;
        if lag > 0 {
            /* Symmetric, packed FIR coefficients */
            n_LTP_Q14 = silk_add_sat32(
                NSQ.sLTP_shp_Q14[(shp_lag_ptr + 0) as usize],
                NSQ.sLTP_shp_Q14[(shp_lag_ptr - 2) as usize],
            );
            n_LTP_Q14 = silk_SMULWB(n_LTP_Q14, HarmShapeFIRPacked_Q14);
            n_LTP_Q14 = n_LTP_Q14
                + silk_SMULWT(
                    NSQ.sLTP_shp_Q14[(shp_lag_ptr - 1) as usize],
                    HarmShapeFIRPacked_Q14,
                );
            n_LTP_Q14 = LTP_pred_Q14 - (silk_LSHIFT(n_LTP_Q14, 2)); /* Q12 -> Q14 */
            shp_lag_ptr += 1;
        } else {
            n_LTP_Q14 = 0;
        }

        /* BEGIN Updating Delayed Decision States */

        /* Generate dither */
        psDelDec.Seed = mm256_rand(psDelDec.Seed);

        /* Short-term prediction */
        let mut LPC_pred_Q14 = short_prediction_x4(
            &psDelDec.sLPC_Q14,
            NSQ_LPC_BUF_LENGTH - 1 + i,
            a_Q12,
            predictLPCOrder,
        );
        LPC_pred_Q14 = mm_slli(LPC_pred_Q14, 4); /* Q10 -> Q14 */

        /* Noise shape feedback */
        silk_assert!(shapingLPCOrder > 0);
        silk_assert!((shapingLPCOrder & 1) == 0); /* check that order is even */
        /* Output of lowpass section */
        let mut tmp0 = mm_add(
            psDelDec.Diff_Q14,
            mm_smulwb(psDelDec.sAR2_Q14[0], warping_Q16),
        );
        let mut n_AR_Q14 = mm_set1(shapingLPCOrder >> 1);
        for j in 0..(shapingLPCOrder - 1) as usize {
            /* Output of allpass section */
            let tmp1 = psDelDec.sAR2_Q14[j];
            psDelDec.sAR2_Q14[j] = tmp0;
            n_AR_Q14 = mm_add(
                n_AR_Q14,
                mm_smulwb(tmp0, AR_shp_Q13[j] as opus_int32),
            );
            tmp0 = mm_add(
                tmp1,
                mm_smulwb(mm_sub(psDelDec.sAR2_Q14[j + 1], tmp0), warping_Q16),
            );
        }
        psDelDec.sAR2_Q14[(shapingLPCOrder - 1) as usize] = tmp0;
        n_AR_Q14 = mm_add(
            n_AR_Q14,
            mm_smulwb(tmp0, AR_shp_Q13[(shapingLPCOrder - 1) as usize] as opus_int32),
        );

        let mut n_AR_Q14 = mm_slli(n_AR_Q14, 1); /* Q11 -> Q12 */
        n_AR_Q14 = mm_add(n_AR_Q14, mm_smulwb(psDelDec.LF_AR_Q14, Tilt_Q14)); /* Q12 */
        let n_AR_Q14 = mm_slli(n_AR_Q14, 2); /* Q12 -> Q14 */

        let tmp0 = mm_smulwb(psDelDec.Samples[*smpl_buf_idx as usize].Shape_Q14, LF_shp_Q14); /* Q12 */
        let tmp1 = mm_smulwb(psDelDec.LF_AR_Q14, LF_shp_Q14 >> 16); /* Q12 */
        let mut n_LF_Q14 = mm_add(tmp0, tmp1); /* Q12 */
        n_LF_Q14 = mm_slli(n_LF_Q14, 2); /* Q12 -> Q14 */

        /* Input minus prediction plus noise feedback                       */
        /* r = x[ i ] - LTP_pred - LPC_pred + n_AR + n_Tilt + n_LF + n_LTP  */
        let tmp0 = mm_add_sat(n_AR_Q14, n_LF_Q14); /* Q14 */
        let tmp1 = mm_add(mm_set1(n_LTP_Q14), LPC_pred_Q14); /* Q13 */
        let tmp0 = mm_sub_sat(tmp1, tmp0); /* Q13 */
        let tmp0 = mm_srai_round(tmp0, 4); /* Q10 */

        let mut r_Q10 = mm_sub(mm_set1(x_Q10[i as usize]), tmp0); /* residual error Q10 */

        /* Flip sign depending on dither */
        r_Q10 = mm_sign(r_Q10, psDelDec.Seed);
        let mut r_Q10 = mm_limit(r_Q10, -(31 << 10), 30 << 10);

        /* Find two quantization level candidates and measure their rate-distortion */
        let mut q1_Q10 = mm_sub(r_Q10, mm_set1(offset_Q10));
        let mut q1_Q0 = mm_srai(q1_Q10, 10);
        if Lambda_Q10 > 2048 {
            /* For aggressive RDO, the bias becomes more than one pulse. */
            let tmp0 = mm_sub(mm_abs(q1_Q10), mm_set1(Lambda_Q10 / 2 - 512)); /* rdo_offset */
            q1_Q0 = mm_srai(q1_Q10, 31);
            let tmp1 = mm_cmpgt(tmp0, [0; 4]);
            let tmp0 = mm_srai(mm_sign(tmp0, q1_Q10), 10);
            q1_Q0 = mm_blendv(q1_Q0, tmp0, tmp1);
        }

        let tmp0 = mm_sign_epi32(mm_set1(QUANT_LEVEL_ADJUST_Q10), q1_Q0);
        q1_Q10 = mm_sub(mm_slli(q1_Q0, 10), tmp0);
        q1_Q10 = mm_add(q1_Q10, mm_set1(offset_Q10));

        /* check if q1_Q0 is 0 or -1 */
        let tmp0 = mm_add(mm_srli(q1_Q0, 31), q1_Q0);
        let tmp1 = mm_cmpeq(tmp0, [0; 4]);
        let tmp0 = mm_blendv(mm_set1(1024), mm_set1(1024 - QUANT_LEVEL_ADJUST_Q10), tmp1);
        let q2_Q10 = mm_add(q1_Q10, tmp0);
        let q_Q10 = mm256_broadcast(q1_Q10);
        let q_Q10 = [
            q_Q10[0], q_Q10[1], q_Q10[2], q_Q10[3], q2_Q10[0], q2_Q10[1], q2_Q10[2], q2_Q10[3],
        ];

        let mut rr_Q10 = mm256_sub(mm256_broadcast(r_Q10), q_Q10);
        let mut rd_Q10 = mm256_abs(q_Q10);
        rr_Q10 = mm256_smulbb(rr_Q10, rr_Q10);
        rd_Q10 = mm256_smulbb(rd_Q10, mm256_set1(Lambda_Q10));
        rd_Q10 = mm256_add(rd_Q10, rr_Q10);
        rd_Q10 = mm256_srai(rd_Q10, 10);

        let rd_lo = [rd_Q10[0], rd_Q10[1], rd_Q10[2], rd_Q10[3]];
        let rd_hi = [rd_Q10[4], rd_Q10[5], rd_Q10[6], rd_Q10[7]];
        let mask = mm256_broadcast(mm_cmplt(rd_lo, rd_hi));
        let mut SS_RD_Q10 = mm256_add(
            mm256_broadcast(psDelDec.RD_Q10),
            mm256_blendv(mm256_swap128(rd_Q10), rd_Q10, mask),
        );
        let mut SS_Q_Q10 = mm256_blendv(mm256_swap128(q_Q10), q_Q10, mask);

        /* Update states for best and second best quantization */

        /* Quantized excitation */
        let mut exc_Q14 = mm256_sign(
            mm256_slli(SS_Q_Q10, 4),
            mm256_broadcast(psDelDec.Seed),
        );

        /* Add predictions */
        exc_Q14 = mm256_add(exc_Q14, mm256_set1(LTP_pred_Q14));
        let mut SS_LPC_exc_Q14 = mm256_slli(exc_Q14, 1);
        let mut SS_xq_Q14 = mm256_add(exc_Q14, mm256_broadcast(LPC_pred_Q14));

        /* Update states */
        let mut SS_Diff_Q14 = mm256_sub(
            SS_xq_Q14,
            mm256_set1(silk_LSHIFT(x_Q10[i as usize], 4)),
        );
        let mut SS_LF_AR_Q14 = mm256_sub(SS_Diff_Q14, mm256_broadcast(n_AR_Q14));
        let mut SS_sLTP_shp_Q14 = mm256_sub_sat(SS_LF_AR_Q14, mm256_broadcast(n_LF_Q14));

        /* END Updating Delayed Decision States */

        *smpl_buf_idx = (*smpl_buf_idx + DECISION_DELAY - 1) % DECISION_DELAY;
        let mut last_smple_idx = (*smpl_buf_idx + decisionDelay) % DECISION_DELAY;
        let psLastSample = last_smple_idx as usize;

        /* Find winner */
        let ss_lo = [SS_RD_Q10[0], SS_RD_Q10[1], SS_RD_Q10[2], SS_RD_Q10[3]];
        let RDmin_Q10 = mm_mask_hmin(ss_lo, MaskDelDec);
        let Winner_ind = index_of_first_equal(RDmin_Q10, ss_lo);

        /* Increase RD values of expired states */
        let Winner_rand_state =
            psDelDec.Samples[psLastSample].RandState[Winner_ind as usize];

        SS_RD_Q10 = mm256_blendv(
            mm256_add(SS_RD_Q10, mm256_set1(silk_int32_MAX >> 4)),
            SS_RD_Q10,
            mm256_broadcast(mm_cmpeq(
                psDelDec.Samples[psLastSample].RandState,
                mm_set1(Winner_rand_state),
            )),
        );

        /* find worst in first set */
        let ss_lo = [SS_RD_Q10[0], SS_RD_Q10[1], SS_RD_Q10[2], SS_RD_Q10[3]];
        let ss_hi = [SS_RD_Q10[4], SS_RD_Q10[5], SS_RD_Q10[6], SS_RD_Q10[7]];
        let RDmax_Q10 = mm_mask_hmax(ss_lo, MaskDelDec);
        let RDmin_Q10 = mm_mask_hmin(ss_hi, MaskDelDec);

        /* Replace a state if best from second set outperforms worst in first set */
        let tmp0 = mm_cmplt(RDmin_Q10, RDmax_Q10);
        if tmp0[0] != 0 || tmp0[1] != 0 || tmp0[2] != 0 || tmp0[3] != 0 {
            let ss_lo = [SS_RD_Q10[0], SS_RD_Q10[1], SS_RD_Q10[2], SS_RD_Q10[3]];
            let ss_hi = [SS_RD_Q10[4], SS_RD_Q10[5], SS_RD_Q10[6], SS_RD_Q10[7]];
            let RDmax_ind = index_of_first_equal(RDmax_Q10, ss_lo);
            let RDmin_ind = index_of_first_equal(RDmin_Q10, ss_hi);
            /* shuffle all lanes: lane RDmax_ind <- lane RDmin_ind, others kept */
            let shfl = |v: M128| -> M128 {
                let mut out = v;
                out[RDmax_ind as usize] = v[RDmin_ind as usize];
                out
            };
            for t in i as usize..(MAX_SUB_FRAME_LENGTH + NSQ_LPC_BUF_LENGTH) as usize {
                psDelDec.sLPC_Q14[t] = shfl(psDelDec.sLPC_Q14[t]);
            }
            psDelDec.Seed = shfl(psDelDec.Seed);
            psDelDec.SeedInit = shfl(psDelDec.SeedInit);
            for t in 0..MAX_SHAPE_LPC_ORDER as usize {
                psDelDec.sAR2_Q14[t] = shfl(psDelDec.sAR2_Q14[t]);
            }
            for t in 0..DECISION_DELAY as usize {
                let s = psDelDec.Samples[t];
                psDelDec.Samples[t].RandState = shfl(s.RandState);
                psDelDec.Samples[t].Q_Q10 = shfl(s.Q_Q10);
                psDelDec.Samples[t].Xq_Q14 = shfl(s.Xq_Q14);
                psDelDec.Samples[t].Pred_Q15 = shfl(s.Pred_Q15);
                psDelDec.Samples[t].Shape_Q14 = shfl(s.Shape_Q14);
            }
            /* permutevar8x32: lane RDmax_ind takes lane RDmin_ind + 4 */
            let perm = |v: M256| -> M256 {
                let mut out = v;
                out[RDmax_ind as usize] = v[(RDmin_ind + 4) as usize];
                out
            };
            SS_Q_Q10 = perm(SS_Q_Q10);
            SS_RD_Q10 = perm(SS_RD_Q10);
            SS_xq_Q14 = perm(SS_xq_Q14);
            SS_LF_AR_Q14 = perm(SS_LF_AR_Q14);
            SS_Diff_Q14 = perm(SS_Diff_Q14);
            SS_sLTP_shp_Q14 = perm(SS_sLTP_shp_Q14);
            SS_LPC_exc_Q14 = perm(SS_LPC_exc_Q14);
        }

        /* Write samples from winner to output and long-term filter states */
        if subfr > 0 || i >= decisionDelay {
            pulses[(pulse_base + i - decisionDelay) as usize] =
                silk_sar_round_32(psDelDec.Samples[psLastSample].Q_Q10[Winner_ind as usize], 10)
                    as opus_int8;
            NSQ.xq[(xq_off + i - decisionDelay) as usize] = silk_sat16(
                silk_sar_round_smulww(
                    psDelDec.Samples[psLastSample].Xq_Q14[Winner_ind as usize],
                    delayedGain_Q10[last_smple_idx as usize],
                    8,
                ) as opus_int32,
            );
            NSQ.sLTP_shp_Q14[(NSQ.sLTP_shp_buf_idx - decisionDelay) as usize] =
                psDelDec.Samples[psLastSample].Shape_Q14[Winner_ind as usize];
            sLTP_Q15[(NSQ.sLTP_buf_idx - decisionDelay) as usize] =
                psDelDec.Samples[psLastSample].Pred_Q15[Winner_ind as usize];
        }
        NSQ.sLTP_shp_buf_idx += 1;
        NSQ.sLTP_buf_idx += 1;

        /* Update states */
        let psSample = *smpl_buf_idx as usize;
        psDelDec.Seed = mm_add(psDelDec.Seed, mm_srai_round(low128(&SS_Q_Q10), 10));
        psDelDec.LF_AR_Q14 = low128(&SS_LF_AR_Q14);
        psDelDec.Diff_Q14 = low128(&SS_Diff_Q14);
        psDelDec.sLPC_Q14[(i + NSQ_LPC_BUF_LENGTH) as usize] = low128(&SS_xq_Q14);
        psDelDec.RD_Q10 = low128(&SS_RD_Q10);
        psDelDec.Samples[psSample].Xq_Q14 = low128(&SS_xq_Q14);
        psDelDec.Samples[psSample].Q_Q10 = low128(&SS_Q_Q10);
        psDelDec.Samples[psSample].Pred_Q15 = low128(&SS_LPC_exc_Q14);
        psDelDec.Samples[psSample].Shape_Q14 = low128(&SS_sLTP_shp_Q14);
        psDelDec.Samples[psSample].RandState = psDelDec.Seed;
        delayedGain_Q10[psSample] = Gain_Q10;
    }
    /* Update LPC states */
    for i in 0..NSQ_LPC_BUF_LENGTH as usize {
        psDelDec.sLPC_Q14[i] = psDelDec.sLPC_Q14[i + length as usize];
    }
}

/// `_mm256_castsi256_si128`
#[inline(always)]
fn low128(v: &M256) -> M128 {
    [v[0], v[1], v[2], v[3]]
}

/// `verify_assumptions`
fn verify_assumptions(psEncC: &silk_encoder_state) -> bool {
    /* The compile-time asserts hold for this configuration. */
    silk_assert!(psEncC.fs_kHz == 8 || psEncC.fs_kHz == 12 || psEncC.fs_kHz == 16);
    silk_assert!(psEncC.nb_subfr <= MAX_NB_SUBFR && psEncC.nb_subfr > 0);
    silk_assert!(
        psEncC.nStatesDelayedDecision <= MAX_DEL_DEC_STATES && psEncC.nStatesDelayedDecision > 0
    );
    silk_assert!(psEncC.ltp_mem_length == psEncC.fs_kHz * LTP_MEM_LENGTH_MS);

    /* Regressions were observed on certain AMD Zen CPUs when
       nStatesDelayedDecision is 1 or 2; the kernel is therefore used only for
       3 or 4. */
    psEncC.nStatesDelayedDecision == 3 || psEncC.nStatesDelayedDecision == 4
}

/// `silk_NSQ_del_dec_avx2` (arch 4)
#[allow(clippy::too_many_arguments)]
pub fn silk_NSQ_del_dec_avx2(
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
    if !verify_assumptions(psEncC) {
        silk_NSQ_del_dec_c(
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
        return;
    }

    let mut lag: opus_int;
    let mut start_idx: opus_int;
    let LSF_interpolation_flag: opus_int;
    let mut HarmShapeFIRPacked_Q14: opus_int32;
    let mut subfr: opus_int;

    let MaskDelDec = mask_del_dec(psEncC.nStatesDelayedDecision);

    /* Set unvoiced lag to the previous one, overwrite later for voiced */
    lag = NSQ.lagPrev;

    silk_assert!(NSQ.prev_gain_Q16 != 0);
    let mut psDelDec = NSQ_del_dec_struct_avx2::default();
    psDelDec.Seed = {
        let add = [0 + psIndices.Seed as opus_int32,
                   1 + psIndices.Seed as opus_int32,
                   2 + psIndices.Seed as opus_int32,
                   3 + psIndices.Seed as opus_int32];
        let mut s = [0i32; 4];
        for i in 0..4 {
            s[i] = add[i] & 3;
        }
        s
    };
    psDelDec.SeedInit = psDelDec.Seed;
    psDelDec.RD_Q10 = [0; 4];
    psDelDec.LF_AR_Q14 = mm_set1(NSQ.sLF_AR_shp_Q14);
    psDelDec.Diff_Q14 = mm_set1(NSQ.sDiff_shp_Q14);
    psDelDec.Samples[0].Shape_Q14 = mm_set1(NSQ.sLTP_shp_Q14[(psEncC.ltp_mem_length - 1) as usize]);
    for i in 0..NSQ_LPC_BUF_LENGTH as usize {
        psDelDec.sLPC_Q14[i] = mm_set1(NSQ.sLPC_Q14[i]);
    }
    for i in 0..MAX_SHAPE_LPC_ORDER as usize {
        psDelDec.sAR2_Q14[i] = mm_set1(NSQ.sAR2_Q14[i]);
    }

    let offset_Q10 =
        silk_Quantization_Offsets_Q10[(psIndices.signalType >> 1) as usize][psIndices.quantOffsetType as usize]
            as opus_int;
    let mut smpl_buf_idx: opus_int = 0; /* index of oldest samples */

    let mut decisionDelay = silk_min_int(DECISION_DELAY, psEncC.subfr_length);

    /* For voiced frames limit the decision delay to lower than the pitch lag */
    if psIndices.signalType as opus_int == TYPE_VOICED {
        for k in 0..psEncC.nb_subfr {
            decisionDelay = silk_min_int(decisionDelay, pitchL[k as usize] - LTP_ORDER / 2 - 1);
        }
    } else if lag > 0 {
        decisionDelay = silk_min_int(decisionDelay, lag - LTP_ORDER / 2 - 1);
    }

    if psIndices.NLSFInterpCoef_Q2 == 4 {
        LSF_interpolation_flag = 0;
    } else {
        LSF_interpolation_flag = 1;
    }

    let alloc_len = (psEncC.ltp_mem_length + psEncC.frame_length) as usize;
    let mut sLTP_Q15 = vec![0 as opus_int32; alloc_len];
    let mut sLTP = vec![0 as opus_int16; alloc_len];
    let mut x_sc_Q10 = vec![0 as opus_int32; MAX_SUB_FRAME_LENGTH as usize];
    let mut delayedGain_Q10 = vec![0 as opus_int32; DECISION_DELAY as usize];
    /* Set up pointers to start of sub frame */
    let pxq_off = psEncC.ltp_mem_length;
    NSQ.sLTP_shp_buf_idx = psEncC.ltp_mem_length;
    NSQ.sLTP_buf_idx = psEncC.ltp_mem_length;
    subfr = 0;
    for k in 0..psEncC.nb_subfr {
        let A_Q12 = &PredCoef_Q12[((k >> 1) | (1 ^ LSF_interpolation_flag)) as usize];
        let B_Q14 = &LTPCoef_Q14[(k * LTP_ORDER) as usize..];
        let AR_shp_Q13 = &AR_Q13[(k * MAX_SHAPE_LPC_ORDER) as usize..];

        /* Noise shape parameters */
        silk_assert!(HarmShapeGain_Q14[k as usize] >= 0);
        HarmShapeFIRPacked_Q14 = silk_RSHIFT(HarmShapeGain_Q14[k as usize], 2);
        HarmShapeFIRPacked_Q14 |= silk_LSHIFT(silk_RSHIFT(HarmShapeGain_Q14[k as usize], 1), 16);

        NSQ.rewhite_flag = 0;
        if psIndices.signalType as opus_int == TYPE_VOICED {
            /* Voiced */
            lag = pitchL[k as usize];

            /* Re-whitening */
            if (k & (3 ^ (LSF_interpolation_flag << 1))) == 0 {
                if k == 2 {
                    /* RESET DELAYED DECISIONS */
                    /* Find winner */
                    let RDmin_Q10 = mm_mask_hmin(psDelDec.RD_Q10, MaskDelDec);
                    let Winner_ind = index_of_first_equal(RDmin_Q10, psDelDec.RD_Q10);
                    psDelDec.RD_Q10 = mm_add(
                        psDelDec.RD_Q10,
                        mm_blendv(
                            mm_set1(silk_int32_MAX >> 4),
                            [0; 4],
                            mask_lane(Winner_ind),
                        ),
                    );

                    /* Copy final part of signals from winner state to output and long-term filter states */
                    let mut last_smple_idx = smpl_buf_idx + decisionDelay;
                    for i in 0..decisionDelay {
                        last_smple_idx = (last_smple_idx + DECISION_DELAY - 1) % DECISION_DELAY;
                        let psSample = &psDelDec.Samples[last_smple_idx as usize];
                        pulses[(k * psEncC.subfr_length + i - decisionDelay) as usize] =
                            silk_sar_round_32(psSample.Q_Q10[Winner_ind as usize], 10) as opus_int8;
                        NSQ.xq[(pxq_off + k * psEncC.subfr_length + i - decisionDelay) as usize] =
                            silk_sat16(silk_sar_round_smulww(
                                psSample.Xq_Q14[Winner_ind as usize],
                                Gains_Q16[1],
                                14,
                            ) as opus_int32);
                        NSQ.sLTP_shp_Q14
                            [(NSQ.sLTP_shp_buf_idx - decisionDelay + i) as usize] =
                            psSample.Shape_Q14[Winner_ind as usize];
                    }

                    subfr = 0;
                }

                /* Rewhiten with new A coefs */
                start_idx = psEncC.ltp_mem_length - lag - psEncC.predictLPCOrder - LTP_ORDER / 2;
                silk_assert!(start_idx > 0);

                let in_off = start_idx + k * psEncC.subfr_length;
                silk_LPC_analysis_filter_avx2(
                    &mut sLTP[start_idx as usize..],
                    &NSQ.xq[in_off as usize..],
                    A_Q12,
                    psEncC.ltp_mem_length - start_idx,
                    psEncC.predictLPCOrder,
                );

                NSQ.sLTP_buf_idx = psEncC.ltp_mem_length;
                NSQ.rewhite_flag = 1;
            }
        }

        silk_nsq_del_dec_scale_states_avx2(
            psEncC,
            NSQ,
            &mut psDelDec,
            &x16[(k * psEncC.subfr_length) as usize..],
            &mut x_sc_Q10,
            &sLTP,
            &mut sLTP_Q15,
            k,
            LTP_scale_Q14,
            Gains_Q16,
            pitchL,
            psIndices.signalType as opus_int,
            decisionDelay,
        );

        silk_noise_shape_quantizer_del_dec_avx2(
            NSQ,
            &mut psDelDec,
            psIndices.signalType as opus_int,
            &x_sc_Q10,
            pulses,
            k * psEncC.subfr_length,
            pxq_off + k * psEncC.subfr_length,
            &mut sLTP_Q15,
            &mut delayedGain_Q10,
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
            subfr,
            psEncC.shapingLPCOrder,
            psEncC.predictLPCOrder,
            psEncC.warping_Q16,
            MaskDelDec,
            &mut smpl_buf_idx,
            decisionDelay,
        );
        subfr += 1;
    }

    /* Find winner */
    let RDmin_Q10 = mm_mask_hmin(psDelDec.RD_Q10, MaskDelDec);
    let Winner_ind = index_of_first_equal(RDmin_Q10, psDelDec.RD_Q10);

    /* Copy final part of signals from winner state to output and long-term filter states */
    psIndices.Seed = psDelDec.SeedInit[Winner_ind as usize] as opus_int8;
    let mut last_smple_idx = smpl_buf_idx + decisionDelay;
    let Gain_Q10 = Gains_Q16[(psEncC.nb_subfr - 1) as usize] >> 6;
    for i in 0..decisionDelay {
        last_smple_idx = (last_smple_idx + DECISION_DELAY - 1) % DECISION_DELAY;
        let psSample = &psDelDec.Samples[last_smple_idx as usize];

        pulses[(psEncC.nb_subfr * psEncC.subfr_length + i - decisionDelay) as usize] =
            silk_sar_round_32(psSample.Q_Q10[Winner_ind as usize], 10) as opus_int8;
        NSQ.xq[(pxq_off + psEncC.nb_subfr * psEncC.subfr_length + i - decisionDelay) as usize] = silk_sat16(silk_sar_round_smulww(
            psSample.Xq_Q14[Winner_ind as usize],
            Gain_Q10,
            8,
        ) as opus_int32);
        NSQ.sLTP_shp_Q14[(NSQ.sLTP_shp_buf_idx - decisionDelay + i) as usize] =
            psSample.Shape_Q14[Winner_ind as usize];
    }
    for i in 0..NSQ_LPC_BUF_LENGTH as usize {
        NSQ.sLPC_Q14[i] = psDelDec.sLPC_Q14[i][Winner_ind as usize];
    }
    for i in 0..MAX_SHAPE_LPC_ORDER as usize {
        NSQ.sAR2_Q14[i] = psDelDec.sAR2_Q14[i][Winner_ind as usize];
    }

    /* Update states */
    NSQ.sLF_AR_shp_Q14 = psDelDec.LF_AR_Q14[Winner_ind as usize];
    NSQ.sDiff_shp_Q14 = psDelDec.Diff_Q14[Winner_ind as usize];
    NSQ.lagPrev = pitchL[(psEncC.nb_subfr - 1) as usize];

    /* Save quantized speech signal */
    let fl = psEncC.frame_length as usize;
    let lml = psEncC.ltp_mem_length as usize;
    NSQ.xq.copy_within(fl..fl + lml, 0);
    NSQ.sLTP_shp_Q14.copy_within(fl..fl + lml, 0);
}

/// The dispatched `silk_NSQ_del_dec` (`SILK_NSQ_DEL_DEC_IMPL[(arch) & OPUS_ARCHMASK]`).
///
/// arch 4 -> `silk_NSQ_del_dec_avx2` (with its internal `nStatesDelayedDecision
/// ∈ {1,2}` fallback to the C body); arch 3 -> `silk_NSQ_del_dec_sse4_1`, which
/// is the C body; arch 0..2 -> `silk_NSQ_del_dec_c`.
#[allow(clippy::too_many_arguments)]
pub fn silk_NSQ_del_dec(
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
    if (arch & 7) == 4 {
        silk_NSQ_del_dec_avx2(
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
    } else {
        silk_NSQ_del_dec_c(
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
}

/// `silk_NSQ_del_dec_sse4_1` — the arch-3 entry; `docs/rtcd-kernels.md` §4.4
/// proves it is a transcription of `silk_NSQ_del_dec_c`.
#[allow(clippy::too_many_arguments)]
pub fn silk_NSQ_del_dec_sse4_1(
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
    silk_NSQ_del_dec_c(
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

    #[test]
    fn mask_del_dec_matches_c_shift() {
        /* `set` lanes are ignored: nStates active lanes are kept, the rest
           are masked (verified against the C macro on the host toolchain). */
        assert_eq!(mask_del_dec(1), [0, -1, -1, -1]);
        assert_eq!(mask_del_dec(2), [0, 0, -1, -1]);
        assert_eq!(mask_del_dec(3), [0, 0, 0, -1]);
        assert_eq!(mask_del_dec(4), [0, 0, 0, 0]);
    }

    #[test]
    fn index_of_first_equal_scans_low_lane_first() {
        assert_eq!(index_of_first_equal([7, 7, 1, 2], [7, 3, 9, 8]), 0);
        assert_eq!(index_of_first_equal([7, 3, 9, 8], [0, 3, 9, 8]), 1);
        assert_eq!(index_of_first_equal([1, 2, 9, 8], [1, 2, 9, 8]), 0);
    }

    #[test]
    fn sar_round_smulww_is_int64_form() {
        /* The AVX2 helper skips the C int32 truncation of SMULWW+RSHIFT_ROUND. */
        let a: opus_int32 = 0x3000_0000;
        let b: opus_int32 = 0x2000;
        let got = silk_sar_round_smulww(a, b, 8);
        let t = ((a as i64) * (b as i64)).wrapping_add(1i64 << (8 + 16 - 1));
        assert_eq!(got, t >> (8 + 16));
    }

    #[test]
    fn hmin_hmax_broadcast() {
        assert_eq!(mm_hmin([5, -1, 9, 2]), [-1; 4]);
        assert_eq!(mm_hmax([5, -1, 9, 2]), [9; 4]);
        /* Lanes with a `set` mask bit are replaced by the identity element. */
        assert_eq!(mm_mask_hmin([5, -1, 9, 2], [0, 0, -1, -1]), [-1; 4]);
        assert_eq!(mm_mask_hmax([5, -1, 9, 2], [0, 0, -1, -1]), [5; 4]);
    }

    #[test]
    fn mulww_smulwb_match_macros() {
        let a = [1234567i32, -987654, i32::MAX, i32::MIN];
        for &x in a.iter() {
            assert_eq!(mm_smulww([x; 4], 45678)[0], silk_SMULWW(x, 45678));
            assert_eq!(mm_smulwb([x; 4], 45678)[0], silk_SMULWB(x, 45678));
            assert_eq!(mm_smulwb([x; 4], -45678)[0], silk_SMULWB(x, -45678));
        }
    }
}

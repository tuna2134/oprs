//! Transliteration of `silk/decode_core.c` from libopus 1.6.1 to safe Rust.
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
use crate::silk::define::*;
use crate::silk::inlines::*;
use crate::silk::lpc_analysis_filter::silk_LPC_analysis_filter;
use crate::silk::macros::*;
use crate::silk::sigproc_fix::*;
use crate::silk::structs::*;
use crate::silk::tables_other::silk_Quantization_Offsets_Q10;
use crate::silk::typedef::*;
use crate::types::*;


/**********************************************************/
/* Core decoder. Performs inverse NSQ operation LTP + LPC */
/**********************************************************/
/// `silk_decode_core(psDec, psDecCtrl, xq, pulses, arch)`
pub fn silk_decode_core(
    psDec: &mut silk_decoder_state,      /* I/O  Decoder state        */
    psDecCtrl: &mut silk_decoder_control, /* I    Decoder control      */
    xq: &mut [opus_int16],               /* O    Decoded speech       */
    pulses: &[opus_int16],               /* I    Pulse signal         */
    arch: opus_int,                      /* I    Run-time architecture */
) {
    let mut i: opus_int;
    let mut k: opus_int;
    let mut lag: opus_int = 0;
    let mut start_idx: opus_int;
    let mut sLTP_buf_idx: opus_int;
    let mut NLSF_interpolation_flag: opus_int;
    let mut signalType: opus_int;
    let mut A_Q12_tmp = [0 as opus_int16; MAX_LPC_ORDER as usize];
    let mut sLTP: Vec<opus_int16>;
    let mut sLTP_Q15: Vec<opus_int32>;
    let mut res_Q14: Vec<opus_int32>;
    let mut sLPC_Q14: Vec<opus_int32>;
    let mut LTP_pred_Q13: opus_int32;
    let mut LPC_pred_Q10: opus_int32;
    let mut Gain_Q10: opus_int32;
    let mut inv_gain_Q31: opus_int32;
    let mut gain_adj_Q16: opus_int32;
    let mut rand_seed: opus_int32;
    let mut offset_Q10: opus_int32;
    let mut pexc_off: opus_int;
    let mut pxq_off: opus_int;

    let ltp_mem_length = psDec.ltp_mem_length;
    let frame_length = psDec.frame_length;
    let subfr_length = psDec.subfr_length;
    let nb_subfr = psDec.nb_subfr;
    let LPC_order = psDec.LPC_order;

    silk_assert!(psDec.prev_gain_Q16 != 0);

    sLTP = vec![0 as opus_int16; ltp_mem_length as usize];
    sLTP_Q15 = vec![0 as opus_int32; (ltp_mem_length + frame_length) as usize];
    res_Q14 = vec![0 as opus_int32; subfr_length as usize];
    sLPC_Q14 = vec![0 as opus_int32; (MAX_SUB_FRAME_LENGTH + MAX_LPC_ORDER) as usize];

    offset_Q10 = silk_Quantization_Offsets_Q10[(psDec.indices.signalType >> 1) as usize]
        [psDec.indices.quantOffsetType as usize] as opus_int32;

    if (psDec.indices.NLSFInterpCoef_Q2 as opus_int) < 1 << 2 {
        NLSF_interpolation_flag = 1;
    } else {
        NLSF_interpolation_flag = 0;
    }

    /* Decode excitation */
    rand_seed = psDec.indices.Seed as opus_int32;
    for i in 0..frame_length {
        rand_seed = silk_RAND(rand_seed);
        psDec.exc_Q14[i as usize] = silk_LSHIFT(pulses[i as usize] as opus_int32, 14);
        if psDec.exc_Q14[i as usize] > 0 {
            psDec.exc_Q14[i as usize] -= (QUANT_LEVEL_ADJUST_Q10 << 4) as opus_int32;
        } else if psDec.exc_Q14[i as usize] < 0 {
            psDec.exc_Q14[i as usize] += (QUANT_LEVEL_ADJUST_Q10 << 4) as opus_int32;
        }
        psDec.exc_Q14[i as usize] += offset_Q10 << 4;
        if rand_seed < 0 {
            psDec.exc_Q14[i as usize] = -psDec.exc_Q14[i as usize];
        }

        rand_seed = silk_ADD32_ovflw(rand_seed, pulses[i as usize] as opus_int32);
    }

    /* Copy LPC state */
    sLPC_Q14[..MAX_LPC_ORDER as usize].copy_from_slice(&psDec.sLPC_Q14_buf);

    pexc_off = 0;
    pxq_off = 0;
    sLTP_buf_idx = ltp_mem_length;
    /* Loop over subframes */
    for k in 0..nb_subfr {
        let pres_is_pexc: bool;

        /* Preload LPC coefficients to array on stack. Gives small performance gain */
        A_Q12_tmp[..LPC_order as usize]
            .copy_from_slice(&psDecCtrl.PredCoef_Q12[(k >> 1) as usize][..LPC_order as usize]);
        signalType = psDec.indices.signalType as opus_int;

        Gain_Q10 = silk_RSHIFT(psDecCtrl.Gains_Q16[k as usize], 6);
        inv_gain_Q31 = silk_INVERSE32_varQ(psDecCtrl.Gains_Q16[k as usize], 47);

        /* Calculate gain adjustment factor */
        if psDecCtrl.Gains_Q16[k as usize] != psDec.prev_gain_Q16 {
            gain_adj_Q16 =
                silk_DIV32_varQ(psDec.prev_gain_Q16, psDecCtrl.Gains_Q16[k as usize], 16);

            /* Scale short term state */
            for i in 0..MAX_LPC_ORDER {
                sLPC_Q14[i as usize] = silk_SMULWW(gain_adj_Q16, sLPC_Q14[i as usize]);
            }
        } else {
            gain_adj_Q16 = 1 << 16;
        }

        /* Save inv_gain */
        silk_assert!(inv_gain_Q31 != 0);
        psDec.prev_gain_Q16 = psDecCtrl.Gains_Q16[k as usize];

        /* Avoid abrupt transition from voiced PLC to unvoiced normal decoding */
        if psDec.lossCnt != 0
            && psDec.prevSignalType == TYPE_VOICED
            && psDec.indices.signalType as opus_int != TYPE_VOICED
            && k < MAX_NB_SUBFR / 2
        {
            let off = (k * LTP_ORDER) as usize;
            psDecCtrl.LTPCoef_Q14[off..off + LTP_ORDER as usize].fill(0);
            psDecCtrl.LTPCoef_Q14[off + (LTP_ORDER / 2) as usize] =
                SILK_FIX_CONST(0.25, 14) as opus_int16;

            signalType = TYPE_VOICED;
            psDecCtrl.pitchL[k as usize] = psDec.lagPrev;
        }

        if signalType == TYPE_VOICED {
            /* Voiced */
            lag = psDecCtrl.pitchL[k as usize];

            /* Re-whitening */
            if k == 0 || (k == 2 && NLSF_interpolation_flag != 0) {
                /* Rewhiten with new A coefs */
                start_idx = ltp_mem_length - lag - LPC_order - LTP_ORDER / 2;
                celt_assert!(start_idx > 0);

                if k == 2 {
                    psDec.outBuf[ltp_mem_length as usize
                        ..(ltp_mem_length + 2 * subfr_length) as usize]
                        .copy_from_slice(&xq[..(2 * subfr_length) as usize]);
                }

                silk_LPC_analysis_filter(
                    &mut sLTP[start_idx as usize..],
                    &psDec.outBuf[(start_idx + k * subfr_length) as usize..],
                    &psDecCtrl.PredCoef_Q12[(k >> 1) as usize],
                    ltp_mem_length - start_idx,
                    LPC_order,
                    arch,
                );

                /* After rewhitening the LTP state is unscaled */
                if k == 0 {
                    /* Do LTP downscaling to reduce inter-packet dependency */
                    inv_gain_Q31 =
                        silk_LSHIFT(silk_SMULWB(inv_gain_Q31, psDecCtrl.LTP_scale_Q14), 2);
                }
                for i in 0..lag + LTP_ORDER / 2 {
                    sLTP_Q15[(sLTP_buf_idx - i - 1) as usize] = silk_SMULWB(
                        inv_gain_Q31,
                        sLTP[(ltp_mem_length - i - 1) as usize] as opus_int32,
                    );
                }
            } else {
                /* Update LTP state when Gain changes */
                if gain_adj_Q16 != 1 << 16 {
                    for i in 0..lag + LTP_ORDER / 2 {
                        sLTP_Q15[(sLTP_buf_idx - i - 1) as usize] = silk_SMULWW(
                            gain_adj_Q16,
                            sLTP_Q15[(sLTP_buf_idx - i - 1) as usize],
                        );
                    }
                }
            }
        }

        /* Long-term prediction */
        if signalType == TYPE_VOICED {
            /* Set up pointer */
            let mut pred_lag_idx: opus_int = sLTP_buf_idx - lag + LTP_ORDER / 2;
            for i in 0..subfr_length {
                /* Unrolled loop */
                /* Avoids introducing a bias because silk_SMLAWB() always rounds to -inf */
                LTP_pred_Q13 = 2;
                LTP_pred_Q13 = silk_SMLAWB(
                    LTP_pred_Q13,
                    sLTP_Q15[pred_lag_idx as usize],
                    psDecCtrl.LTPCoef_Q14[(k * LTP_ORDER) as usize] as opus_int32,
                );
                LTP_pred_Q13 = silk_SMLAWB(
                    LTP_pred_Q13,
                    sLTP_Q15[(pred_lag_idx - 1) as usize],
                    psDecCtrl.LTPCoef_Q14[(k * LTP_ORDER + 1) as usize] as opus_int32,
                );
                LTP_pred_Q13 = silk_SMLAWB(
                    LTP_pred_Q13,
                    sLTP_Q15[(pred_lag_idx - 2) as usize],
                    psDecCtrl.LTPCoef_Q14[(k * LTP_ORDER + 2) as usize] as opus_int32,
                );
                LTP_pred_Q13 = silk_SMLAWB(
                    LTP_pred_Q13,
                    sLTP_Q15[(pred_lag_idx - 3) as usize],
                    psDecCtrl.LTPCoef_Q14[(k * LTP_ORDER + 3) as usize] as opus_int32,
                );
                LTP_pred_Q13 = silk_SMLAWB(
                    LTP_pred_Q13,
                    sLTP_Q15[(pred_lag_idx - 4) as usize],
                    psDecCtrl.LTPCoef_Q14[(k * LTP_ORDER + 4) as usize] as opus_int32,
                );
                pred_lag_idx += 1;

                /* Generate LPC excitation */
                res_Q14[i as usize] = silk_ADD_LSHIFT32(
                    psDec.exc_Q14[(pexc_off + i) as usize],
                    LTP_pred_Q13,
                    1,
                );

                /* Update states */
                sLTP_Q15[sLTP_buf_idx as usize] = silk_LSHIFT(res_Q14[i as usize], 1);
                sLTP_buf_idx += 1;
            }
            pres_is_pexc = false;
        } else {
            pres_is_pexc = true;
        }

        for i in 0..subfr_length {
            /* Short-term prediction */
            celt_assert!(LPC_order == 10 || LPC_order == 16);
            /* Avoids introducing a bias because silk_SMLAWB() always rounds to -inf */
            LPC_pred_Q10 = silk_RSHIFT(LPC_order, 1);
            LPC_pred_Q10 = silk_SMLAWB(
                LPC_pred_Q10,
                sLPC_Q14[(MAX_LPC_ORDER + i - 1) as usize],
                A_Q12_tmp[0] as opus_int32,
            );
            LPC_pred_Q10 = silk_SMLAWB(
                LPC_pred_Q10,
                sLPC_Q14[(MAX_LPC_ORDER + i - 2) as usize],
                A_Q12_tmp[1] as opus_int32,
            );
            LPC_pred_Q10 = silk_SMLAWB(
                LPC_pred_Q10,
                sLPC_Q14[(MAX_LPC_ORDER + i - 3) as usize],
                A_Q12_tmp[2] as opus_int32,
            );
            LPC_pred_Q10 = silk_SMLAWB(
                LPC_pred_Q10,
                sLPC_Q14[(MAX_LPC_ORDER + i - 4) as usize],
                A_Q12_tmp[3] as opus_int32,
            );
            LPC_pred_Q10 = silk_SMLAWB(
                LPC_pred_Q10,
                sLPC_Q14[(MAX_LPC_ORDER + i - 5) as usize],
                A_Q12_tmp[4] as opus_int32,
            );
            LPC_pred_Q10 = silk_SMLAWB(
                LPC_pred_Q10,
                sLPC_Q14[(MAX_LPC_ORDER + i - 6) as usize],
                A_Q12_tmp[5] as opus_int32,
            );
            LPC_pred_Q10 = silk_SMLAWB(
                LPC_pred_Q10,
                sLPC_Q14[(MAX_LPC_ORDER + i - 7) as usize],
                A_Q12_tmp[6] as opus_int32,
            );
            LPC_pred_Q10 = silk_SMLAWB(
                LPC_pred_Q10,
                sLPC_Q14[(MAX_LPC_ORDER + i - 8) as usize],
                A_Q12_tmp[7] as opus_int32,
            );
            LPC_pred_Q10 = silk_SMLAWB(
                LPC_pred_Q10,
                sLPC_Q14[(MAX_LPC_ORDER + i - 9) as usize],
                A_Q12_tmp[8] as opus_int32,
            );
            LPC_pred_Q10 = silk_SMLAWB(
                LPC_pred_Q10,
                sLPC_Q14[(MAX_LPC_ORDER + i - 10) as usize],
                A_Q12_tmp[9] as opus_int32,
            );
            if LPC_order == 16 {
                LPC_pred_Q10 = silk_SMLAWB(
                    LPC_pred_Q10,
                    sLPC_Q14[(MAX_LPC_ORDER + i - 11) as usize],
                    A_Q12_tmp[10] as opus_int32,
                );
                LPC_pred_Q10 = silk_SMLAWB(
                    LPC_pred_Q10,
                    sLPC_Q14[(MAX_LPC_ORDER + i - 12) as usize],
                    A_Q12_tmp[11] as opus_int32,
                );
                LPC_pred_Q10 = silk_SMLAWB(
                    LPC_pred_Q10,
                    sLPC_Q14[(MAX_LPC_ORDER + i - 13) as usize],
                    A_Q12_tmp[12] as opus_int32,
                );
                LPC_pred_Q10 = silk_SMLAWB(
                    LPC_pred_Q10,
                    sLPC_Q14[(MAX_LPC_ORDER + i - 14) as usize],
                    A_Q12_tmp[13] as opus_int32,
                );
                LPC_pred_Q10 = silk_SMLAWB(
                    LPC_pred_Q10,
                    sLPC_Q14[(MAX_LPC_ORDER + i - 15) as usize],
                    A_Q12_tmp[14] as opus_int32,
                );
                LPC_pred_Q10 = silk_SMLAWB(
                    LPC_pred_Q10,
                    sLPC_Q14[(MAX_LPC_ORDER + i - 16) as usize],
                    A_Q12_tmp[15] as opus_int32,
                );
            }

            let pres = if pres_is_pexc {
                psDec.exc_Q14[(pexc_off + i) as usize]
            } else {
                res_Q14[i as usize]
            };

            /* Add prediction to LPC excitation */
            sLPC_Q14[(MAX_LPC_ORDER + i) as usize] =
                silk_ADD_SAT32(pres, silk_LSHIFT_SAT32(LPC_pred_Q10, 4));

            /* Scale with gain */
            xq[(pxq_off + i) as usize] = silk_SAT16(silk_RSHIFT_ROUND(
                silk_SMULWW(sLPC_Q14[(MAX_LPC_ORDER + i) as usize], Gain_Q10),
                8,
            )) as opus_int16;
        }

        /* Update LPC filter state */
        sLPC_Q14.copy_within(
            subfr_length as usize..(subfr_length + MAX_LPC_ORDER) as usize,
            0,
        );
        pexc_off += subfr_length;
        pxq_off += subfr_length;
    }

    /* Save LPC state */
    psDec.sLPC_Q14_buf.copy_from_slice(&sLPC_Q14[..MAX_LPC_ORDER as usize]);
}

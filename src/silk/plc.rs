//! Transliteration of `silk/PLC.c` from libopus 1.6.1 to safe Rust.
//!
//! The `ENABLE_DEEP_PLC` hooks (`lpcnet_plc_update`, `lpcnet_plc_conceal`) are
//! declared here as no-ops: the network is out of scope, and at the default
//! decoder complexity `enable_deep_plc == 0` and `fec_fill_pos == 0`, so the C
//! only ever reaches the `lpcnet_plc_update` branch (whose DNN work does not
//! touch any SILK state).  The macro-level control flow, including the
//! `fs_kHz == 16` guard around the `lpcnet_plc_update` loop, is kept identical.
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
use crate::silk::bwexpander::silk_bwexpander;
use crate::silk::lpc_analysis_filter::silk_LPC_analysis_filter;
use crate::silk::define::*;
use crate::silk::inlines::{silk_INVERSE32_varQ, silk_SQRT_APPROX};
use crate::silk::lpc_inv_pred_gain::silk_LPC_inverse_pred_gain_c;
use crate::silk::macros::*;
use crate::silk::plc_h::*;
use crate::silk::sigproc_fix::*;
use crate::silk::structs::*;
use crate::silk::sum_sqr_shift::silk_sum_sqr_shift;
use crate::silk::typedef::*;
use crate::types::*;

// ---------------------------------------------------------------------------
// `dnn/lpcnet_private.h` — `LPCNetPLCState`, reduced to the members the SILK
// decoder reads.  The network is not ported.
// ---------------------------------------------------------------------------

/// `LPCNetPLCState` (`dnn/lpcnet_private.h`), reduced to the two integer flags
/// that `silk/PLC.c` tests.
#[derive(Clone, Copy, Debug, Default)]
pub struct LPCNetPLCState {
    /// `int loaded`
    pub loaded: opus_int,
    /// `int fec_fill_pos`
    pub fec_fill_pos: opus_int,
}

/// `lpcnet_plc_update()` — `dnn/lpcnet_plc.c`.  **No-op stub**: it only updates
/// the LPCNet PCM/feature history, never any SILK state, so the decoded signal
/// is unaffected at the default decoder complexity.
pub fn lpcnet_plc_update(_st: &mut LPCNetPLCState, _pcm: &mut [opus_int16]) {}

/// `lpcnet_plc_conceal()` — `dnn/lpcnet_plc.c`.  **No-op stub.**  Reaching this
/// call requires `enable_deep_plc != 0` or `fec_fill_pos != 0`, which the
/// public decoder API cannot select at the default complexity; when it *is*
/// selected the DNN overwrites `frame` and the ported code would not match.
pub fn lpcnet_plc_conceal(_st: &mut LPCNetPLCState, _pcm: &mut [opus_int16]) {}

// ---------------------------------------------------------------------------
// `silk/PLC.c`
// ---------------------------------------------------------------------------

const NB_ATT: opus_int = 2;
static HARM_ATT_Q15: [opus_int16; NB_ATT as usize] = [32440, 31130]; /* 0.99, 0.95 */
static PLC_RAND_ATTENUATE_V_Q15: [opus_int16; NB_ATT as usize] = [31130, 26214]; /* 0.95, 0.8 */
static PLC_RAND_ATTENUATE_UV_Q15: [opus_int16; NB_ATT as usize] = [32440, 29491]; /* 0.99, 0.9 */

/// `silk_PLC_Reset(psDec)`
pub fn silk_PLC_Reset(
    psDec: &mut silk_decoder_state, /* I/O  Decoder state        */
) {
    psDec.sPLC.pitchL_Q8 = silk_LSHIFT(psDec.frame_length, 7);
    psDec.sPLC.prevGain_Q16[0] = SILK_FIX_CONST(1.0, 16);
    psDec.sPLC.prevGain_Q16[1] = SILK_FIX_CONST(1.0, 16);
    psDec.sPLC.subfr_length = 20;
    psDec.sPLC.nb_subfr = 2;
}

/// `silk_PLC(psDec, psDecCtrl, frame, lost, lpcnet, arch)`
pub fn silk_PLC(
    psDec: &mut silk_decoder_state,      /* I/O  Decoder state        */
    psDecCtrl: &mut silk_decoder_control, /* I/O  Decoder control      */
    frame: &mut [opus_int16],            /* I/O  signal               */
    lost: opus_int,                      /* I    Loss flag            */
    lpcnet: Option<&mut LPCNetPLCState>,
    arch: opus_int,                      /* I    Run-time architecture */
) {
    /* PLC control function */
    if psDec.fs_kHz != psDec.sPLC.fs_kHz {
        silk_PLC_Reset(psDec);
        psDec.sPLC.fs_kHz = psDec.fs_kHz;
    }

    if lost != 0 {
        /****************************/
        /* Generate Signal          */
        /****************************/
        silk_PLC_conceal(psDec, psDecCtrl, frame, lpcnet, arch);

        psDec.lossCnt += 1;
    } else {
        /****************************/
        /* Update state             */
        /****************************/
        silk_PLC_update(psDec, psDecCtrl);
        #[cfg(not(test))]
        let _ = arch;
        if let Some(ln) = lpcnet {
            if psDec.sPLC.fs_kHz == 16 {
                let subfr_length = psDec.subfr_length;
                let mut k: opus_int = 0;
                while k < psDec.nb_subfr {
                    lpcnet_plc_update(ln, &mut frame[(k * subfr_length) as usize..]);
                    k += 2;
                }
            }
        }
    }
}

/**************************************************/
/* Update state of PLC                            */
/**************************************************/
fn silk_PLC_update(
    psDec: &mut silk_decoder_state,   /* I/O  Decoder state        */
    psDecCtrl: &silk_decoder_control, /* I/O  Decoder control      */
) {
    let mut LTP_Gain_Q14: opus_int32;
    let mut temp_LTP_Gain_Q14: opus_int32;
    let mut j: opus_int;

    /* Update parameters used in case of packet loss */
    psDec.prevSignalType = psDec.indices.signalType as opus_int;
    LTP_Gain_Q14 = 0;
    if psDec.indices.signalType as opus_int == TYPE_VOICED {
        let nb_subfr = psDec.nb_subfr;
        let subfr_length = psDec.subfr_length;
        /* Find the parameters for the last subframe which contains a pitch pulse */
        j = 0;
        while j * subfr_length < psDecCtrl.pitchL[(psDec.nb_subfr - 1) as usize] {
            if j == psDec.nb_subfr {
                break;
            }
            temp_LTP_Gain_Q14 = 0;
            for i in 0..LTP_ORDER {
                temp_LTP_Gain_Q14 +=
                    psDecCtrl.LTPCoef_Q14[((nb_subfr - 1 - j) * LTP_ORDER + i) as usize] as opus_int32;
            }
            if temp_LTP_Gain_Q14 > LTP_Gain_Q14 {
                LTP_Gain_Q14 = temp_LTP_Gain_Q14;
                let off = ((nb_subfr - 1 - j) * LTP_ORDER) as usize;
                psDec.sPLC.LTPCoef_Q14[..LTP_ORDER as usize]
                    .copy_from_slice(&psDecCtrl.LTPCoef_Q14[off..off + LTP_ORDER as usize]);
                psDec.sPLC.pitchL_Q8 = silk_LSHIFT(psDecCtrl.pitchL[(nb_subfr - 1 - j) as usize], 8);
            }
            j += 1;
        }

        psDec.sPLC.LTPCoef_Q14 = [0; LTP_ORDER as usize];
        psDec.sPLC.LTPCoef_Q14[(LTP_ORDER / 2) as usize] = LTP_Gain_Q14 as opus_int16;

        /* Limit LT coefs */
        if LTP_Gain_Q14 < V_PITCH_GAIN_START_MIN_Q14 {
            let tmp: opus_int32 = silk_LSHIFT(V_PITCH_GAIN_START_MIN_Q14, 10);
            let scale_Q10: opus_int = silk_DIV32(tmp, silk_max(LTP_Gain_Q14, 1));
            for i in 0..LTP_ORDER {
                psDec.sPLC.LTPCoef_Q14[i as usize] = silk_RSHIFT(
                    silk_SMULBB(psDec.sPLC.LTPCoef_Q14[i as usize] as opus_int32, scale_Q10),
                    10,
                ) as opus_int16;
            }
        } else if LTP_Gain_Q14 > V_PITCH_GAIN_START_MAX_Q14 {
            let tmp: opus_int32 = silk_LSHIFT(V_PITCH_GAIN_START_MAX_Q14, 14);
            let scale_Q14: opus_int = silk_DIV32(tmp, silk_max(LTP_Gain_Q14, 1));
            for i in 0..LTP_ORDER {
                psDec.sPLC.LTPCoef_Q14[i as usize] = silk_RSHIFT(
                    silk_SMULBB(psDec.sPLC.LTPCoef_Q14[i as usize] as opus_int32, scale_Q14),
                    14,
                ) as opus_int16;
            }
        }
    } else {
        psDec.sPLC.pitchL_Q8 = silk_LSHIFT(silk_SMULBB(psDec.fs_kHz, 18), 8);
        psDec.sPLC.LTPCoef_Q14 = [0; LTP_ORDER as usize];
    }

    /* Save LPC coefficients */
    let LPC_order = psDec.LPC_order;
    psDec.sPLC.prevLPC_Q12[..LPC_order as usize]
        .copy_from_slice(&psDecCtrl.PredCoef_Q12[1][..LPC_order as usize]);
    psDec.sPLC.prevLTP_scale_Q14 = psDecCtrl.LTP_scale_Q14 as opus_int16;

    /* Save last two gains */
    let nb_subfr = psDec.nb_subfr;
    psDec.sPLC.prevGain_Q16[0] = psDecCtrl.Gains_Q16[(nb_subfr - 2) as usize];
    psDec.sPLC.prevGain_Q16[1] = psDecCtrl.Gains_Q16[(nb_subfr - 1) as usize];

    psDec.sPLC.subfr_length = psDec.subfr_length;
    psDec.sPLC.nb_subfr = psDec.nb_subfr;
}

/// `silk_PLC_energy(energy1, shift1, energy2, shift2, exc_Q14, prevGain_Q10, subfr_length, nb_subfr)`
fn silk_PLC_energy(
    energy1: &mut opus_int32,
    shift1: &mut opus_int,
    energy2: &mut opus_int32,
    shift2: &mut opus_int,
    exc_Q14: &[opus_int32],
    prevGain_Q10: &[opus_int32; 2],
    subfr_length: opus_int,
    nb_subfr: opus_int,
) {
    let mut exc_buf = [0 as opus_int16; (2 * MAX_SUB_FRAME_LENGTH) as usize];

    /* Find random noise component */
    /* Scale previous excitation signal */
    for k in 0..2 {
        for i in 0..subfr_length {
            exc_buf[(k * subfr_length + i) as usize] = silk_SAT16(silk_RSHIFT(
                silk_SMULWW(
                    exc_Q14[(i + (k + nb_subfr - 2) * subfr_length) as usize],
                    prevGain_Q10[k as usize],
                ),
                8,
            )) as opus_int16;
        }
    }
    /* Find the subframe with lowest energy of the last two and use that as random noise generator */
    let (e1, s1) = silk_sum_sqr_shift(&exc_buf[..subfr_length as usize], subfr_length);
    let (e2, s2) = silk_sum_sqr_shift(
        &exc_buf[subfr_length as usize..(2 * subfr_length) as usize],
        subfr_length,
    );
    *energy1 = e1;
    *shift1 = s1;
    *energy2 = e2;
    *shift2 = s2;
}

fn silk_PLC_conceal(
    psDec: &mut silk_decoder_state,       /* I/O  Decoder state        */
    psDecCtrl: &mut silk_decoder_control, /* I/O  Decoder control      */
    frame: &mut [opus_int16],             /* O    LPC residual signal  */
    lpcnet: Option<&mut LPCNetPLCState>,
    arch: opus_int,                       /* I    Run-time architecture */
) {
    let mut i: opus_int;
    let mut j: opus_int;
    let mut k: opus_int;
    let mut lag: opus_int;
    let mut idx: opus_int;
    let mut sLTP_buf_idx: opus_int;
    let mut shift1: opus_int = 0;
    let mut shift2: opus_int = 0;
    let mut rand_seed: opus_int32;
    let mut harm_Gain_Q15: opus_int32;
    let mut rand_Gain_Q15: opus_int32;
    let mut inv_gain_Q30: opus_int32;
    let mut energy1: opus_int32 = 0;
    let mut energy2: opus_int32 = 0;
    let mut rand_ptr_off: opus_int;
    let mut LPC_pred_Q10: opus_int32;
    let mut LTP_pred_Q12: opus_int32;
    let mut rand_scale_Q14: opus_int16;
    let mut sLTP_Q14: Vec<opus_int32>;
    let mut sLTP: Vec<opus_int16>;
    let mut A_Q12 = [0 as opus_int16; MAX_LPC_ORDER as usize];
    let mut prevGain_Q10 = [0 as opus_int32; 2];

    let ltp_mem_length = psDec.ltp_mem_length;
    let frame_length = psDec.frame_length;
    let subfr_length = psDec.subfr_length;
    let nb_subfr = psDec.nb_subfr;
    let LPC_order = psDec.LPC_order;
    let fs_kHz = psDec.fs_kHz;

    sLTP_Q14 = vec![0 as opus_int32; (ltp_mem_length + frame_length) as usize];
    sLTP = vec![0 as opus_int16; ltp_mem_length as usize];

    prevGain_Q10[0] = silk_RSHIFT(psDec.sPLC.prevGain_Q16[0], 6);
    prevGain_Q10[1] = silk_RSHIFT(psDec.sPLC.prevGain_Q16[1], 6);

    if psDec.first_frame_after_reset != 0 {
        psDec.sPLC.prevLPC_Q12 = [0; MAX_LPC_ORDER as usize];
    }

    silk_PLC_energy(
        &mut energy1,
        &mut shift1,
        &mut energy2,
        &mut shift2,
        &psDec.exc_Q14,
        &prevGain_Q10,
        subfr_length,
        nb_subfr,
    );

    if silk_RSHIFT(energy1, shift2) < silk_RSHIFT(energy2, shift1) {
        /* First sub-frame has lowest energy */
        rand_ptr_off = silk_max_int(
            0,
            (psDec.sPLC.nb_subfr - 1) * psDec.sPLC.subfr_length - RAND_BUF_SIZE,
        );
    } else {
        /* Second sub-frame has lowest energy */
        rand_ptr_off = silk_max_int(
            0,
            psDec.sPLC.nb_subfr * psDec.sPLC.subfr_length - RAND_BUF_SIZE,
        );
    }

    /* Set up Gain to random noise component */
    rand_scale_Q14 = psDec.sPLC.randScale_Q14;

    /* Set up attenuation gains */
    harm_Gain_Q15 = HARM_ATT_Q15[silk_min_int(NB_ATT - 1, psDec.lossCnt) as usize] as opus_int32;
    if psDec.prevSignalType == TYPE_VOICED {
        rand_Gain_Q15 =
            PLC_RAND_ATTENUATE_V_Q15[silk_min_int(NB_ATT - 1, psDec.lossCnt) as usize] as opus_int32;
    } else {
        rand_Gain_Q15 =
            PLC_RAND_ATTENUATE_UV_Q15[silk_min_int(NB_ATT - 1, psDec.lossCnt) as usize] as opus_int32;
    }

    /* LPC concealment. Apply BWE to previous LPC */
    silk_bwexpander(
        &mut psDec.sPLC.prevLPC_Q12,
        LPC_order,
        SILK_FIX_CONST(BWE_COEF, 16),
    );

    /* Preload LPC coefficients to array on stack. Gives small performance gain */
    A_Q12[..LPC_order as usize].copy_from_slice(&psDec.sPLC.prevLPC_Q12[..LPC_order as usize]);

    /* First Lost frame */
    if psDec.lossCnt == 0 {
        rand_scale_Q14 = 1 << 14;

        /* Reduce random noise Gain for voiced frames */
        if psDec.prevSignalType == TYPE_VOICED {
            for i in 0..LTP_ORDER {
                rand_scale_Q14 -= psDec.sPLC.LTPCoef_Q14[i as usize];
            }
            rand_scale_Q14 = silk_max_16(3277, rand_scale_Q14); /* 0.2 */
            rand_scale_Q14 = silk_RSHIFT(
                silk_SMULBB(
                    rand_scale_Q14 as opus_int32,
                    psDec.sPLC.prevLTP_scale_Q14 as opus_int32,
                ),
                14,
            ) as opus_int16;
        } else {
            /* Reduce random noise for unvoiced frames with high LPC gain */
            let invGain_Q30: opus_int32 =
                silk_LPC_inverse_pred_gain_c(&psDec.sPLC.prevLPC_Q12, LPC_order);

            let mut down_scale_Q30: opus_int32 =
                silk_min_32(silk_RSHIFT(1 << 30, LOG2_INV_LPC_GAIN_HIGH_THRES), invGain_Q30);
            down_scale_Q30 =
                silk_max_32(silk_RSHIFT(1 << 30, LOG2_INV_LPC_GAIN_LOW_THRES), down_scale_Q30);
            down_scale_Q30 = silk_LSHIFT(down_scale_Q30, LOG2_INV_LPC_GAIN_HIGH_THRES);

            rand_Gain_Q15 = silk_RSHIFT(silk_SMULWB(down_scale_Q30, rand_Gain_Q15), 14);
        }
    }

    rand_seed = psDec.sPLC.rand_seed;
    lag = silk_RSHIFT_ROUND(psDec.sPLC.pitchL_Q8, 8);
    sLTP_buf_idx = ltp_mem_length;

    /* Rewhiten LTP state */
    idx = ltp_mem_length - lag - LPC_order - LTP_ORDER / 2;
    celt_assert!(idx > 0);
    silk_LPC_analysis_filter(
        &mut sLTP[idx as usize..],
        &psDec.outBuf[idx as usize..],
        &A_Q12,
        ltp_mem_length - idx,
        LPC_order,
        arch,
    );
    /* Scale LTP state */
    inv_gain_Q30 = silk_INVERSE32_varQ(psDec.sPLC.prevGain_Q16[1], 46);
    inv_gain_Q30 = silk_min(inv_gain_Q30, silk_int32_MAX >> 1);
    for i in idx + LPC_order..ltp_mem_length {
        sLTP_Q14[i as usize] = silk_SMULWB(inv_gain_Q30, sLTP[i as usize] as opus_int32);
    }

    /***************************/
    /* LTP synthesis filtering */
    /***************************/
    for k in 0..nb_subfr {
        /* Set up pointer */
        let mut pred_lag_idx: opus_int = sLTP_buf_idx - lag + LTP_ORDER / 2;
        for _i in 0..subfr_length {
            /* Unrolled loop */
            /* Avoids introducing a bias because silk_SMLAWB() always rounds to -inf */
            LTP_pred_Q12 = 2;
            LTP_pred_Q12 = silk_SMLAWB(
                LTP_pred_Q12,
                sLTP_Q14[pred_lag_idx as usize],
                psDec.sPLC.LTPCoef_Q14[0] as opus_int32,
            );
            LTP_pred_Q12 = silk_SMLAWB(
                LTP_pred_Q12,
                sLTP_Q14[(pred_lag_idx - 1) as usize],
                psDec.sPLC.LTPCoef_Q14[1] as opus_int32,
            );
            LTP_pred_Q12 = silk_SMLAWB(
                LTP_pred_Q12,
                sLTP_Q14[(pred_lag_idx - 2) as usize],
                psDec.sPLC.LTPCoef_Q14[2] as opus_int32,
            );
            LTP_pred_Q12 = silk_SMLAWB(
                LTP_pred_Q12,
                sLTP_Q14[(pred_lag_idx - 3) as usize],
                psDec.sPLC.LTPCoef_Q14[3] as opus_int32,
            );
            LTP_pred_Q12 = silk_SMLAWB(
                LTP_pred_Q12,
                sLTP_Q14[(pred_lag_idx - 4) as usize],
                psDec.sPLC.LTPCoef_Q14[4] as opus_int32,
            );
            pred_lag_idx += 1;

            /* Generate LPC excitation */
            rand_seed = silk_RAND(rand_seed);
            idx = silk_RSHIFT(rand_seed, 25) & RAND_BUF_MASK;
            sLTP_Q14[sLTP_buf_idx as usize] = silk_LSHIFT32(
                silk_SMLAWB(
                    LTP_pred_Q12,
                    psDec.exc_Q14[(rand_ptr_off + idx) as usize],
                    rand_scale_Q14 as opus_int32,
                ),
                2,
            );
            sLTP_buf_idx += 1;
        }

        /* Gradually reduce LTP gain */
        for j in 0..LTP_ORDER {
            psDec.sPLC.LTPCoef_Q14[j as usize] = silk_RSHIFT(
                silk_SMULBB(harm_Gain_Q15, psDec.sPLC.LTPCoef_Q14[j as usize] as opus_int32),
                15,
            ) as opus_int16;
        }
        /* Gradually reduce excitation gain */
        rand_scale_Q14 = silk_RSHIFT(
            silk_SMULBB(rand_scale_Q14 as opus_int32, rand_Gain_Q15),
            15,
        ) as opus_int16;

        /* Slowly increase pitch lag */
        psDec.sPLC.pitchL_Q8 =
            silk_SMLAWB(psDec.sPLC.pitchL_Q8, psDec.sPLC.pitchL_Q8, PITCH_DRIFT_FAC_Q16);
        psDec.sPLC.pitchL_Q8 = silk_min_32(
            psDec.sPLC.pitchL_Q8,
            silk_LSHIFT(silk_SMULBB(MAX_PITCH_LAG_MS, fs_kHz), 8),
        );
        lag = silk_RSHIFT_ROUND(psDec.sPLC.pitchL_Q8, 8);
    }

    /***************************/
    /* LPC synthesis filtering */
    /***************************/
    let sLPC_base: opus_int = ltp_mem_length - MAX_LPC_ORDER;

    /* Copy LPC state */
    sLTP_Q14[sLPC_base as usize..(sLPC_base + MAX_LPC_ORDER) as usize]
        .copy_from_slice(&psDec.sLPC_Q14_buf);

    celt_assert!(LPC_order >= 10); /* check that unrolling works */
    for i in 0..frame_length {
        let b = (sLPC_base + MAX_LPC_ORDER) as usize;
        /* partly unrolled */
        /* Avoids introducing a bias because silk_SMLAWB() always rounds to -inf */
        LPC_pred_Q10 = silk_RSHIFT(LPC_order, 1);
        LPC_pred_Q10 = silk_SMLAWB(LPC_pred_Q10, sLTP_Q14[b + i as usize - 1], A_Q12[0] as opus_int32);
        LPC_pred_Q10 = silk_SMLAWB(LPC_pred_Q10, sLTP_Q14[b + i as usize - 2], A_Q12[1] as opus_int32);
        LPC_pred_Q10 = silk_SMLAWB(LPC_pred_Q10, sLTP_Q14[b + i as usize - 3], A_Q12[2] as opus_int32);
        LPC_pred_Q10 = silk_SMLAWB(LPC_pred_Q10, sLTP_Q14[b + i as usize - 4], A_Q12[3] as opus_int32);
        LPC_pred_Q10 = silk_SMLAWB(LPC_pred_Q10, sLTP_Q14[b + i as usize - 5], A_Q12[4] as opus_int32);
        LPC_pred_Q10 = silk_SMLAWB(LPC_pred_Q10, sLTP_Q14[b + i as usize - 6], A_Q12[5] as opus_int32);
        LPC_pred_Q10 = silk_SMLAWB(LPC_pred_Q10, sLTP_Q14[b + i as usize - 7], A_Q12[6] as opus_int32);
        LPC_pred_Q10 = silk_SMLAWB(LPC_pred_Q10, sLTP_Q14[b + i as usize - 8], A_Q12[7] as opus_int32);
        LPC_pred_Q10 = silk_SMLAWB(LPC_pred_Q10, sLTP_Q14[b + i as usize - 9], A_Q12[8] as opus_int32);
        LPC_pred_Q10 = silk_SMLAWB(LPC_pred_Q10, sLTP_Q14[b + i as usize - 10], A_Q12[9] as opus_int32);
        for j in 10..LPC_order {
            LPC_pred_Q10 = silk_SMLAWB(
                LPC_pred_Q10,
                sLTP_Q14[b + i as usize - j as usize - 1],
                A_Q12[j as usize] as opus_int32,
            );
        }

        /* Add prediction to LPC excitation */
        sLTP_Q14[b + i as usize] = silk_ADD_SAT32(
            sLTP_Q14[b + i as usize],
            silk_LSHIFT_SAT32(LPC_pred_Q10, 4),
        );

        /* Scale with Gain */
        frame[i as usize] = silk_SAT16(silk_SAT16(silk_RSHIFT_ROUND(
            silk_SMULWW(sLTP_Q14[b + i as usize], prevGain_Q10[1]),
            8,
        ))) as opus_int16;
    }

    if let Some(ln) = lpcnet {
        if ln.loaded != 0 && psDec.sPLC.fs_kHz == 16 {
            let run_deep_plc = psDec.sPLC.enable_deep_plc != 0 || ln.fec_fill_pos != 0;
            if run_deep_plc {
                /* Not ported (see `lpcnet_plc_conceal`); unreachable at the
                   default decoder complexity. */
                let mut kk: opus_int = 0;
                while kk < nb_subfr {
                    lpcnet_plc_conceal(ln, &mut frame[(kk * subfr_length) as usize..]);
                    kk += 2;
                }
                /* We *should* be able to copy only from frame_length-MAX_LPC_ORDER,
                   i.e. the last MAX_LPC_ORDER samples. */
                for i in 0..frame_length {
                    let base = (sLPC_base + MAX_LPC_ORDER) as usize;
                    sLTP_Q14[base + i as usize] = (0.5f64
                        + (frame[i as usize] as f32 * ((1 << 24) as f32)
                            / prevGain_Q10[1] as f32) as f64)
                        .floor() as opus_int32;
                }
            } else {
                let mut kk: opus_int = 0;
                while kk < nb_subfr {
                    lpcnet_plc_update(ln, &mut frame[(kk * subfr_length) as usize..]);
                    kk += 2;
                }
            }
        }
    }

    /* Save LPC state */
    let base = (sLPC_base + frame_length) as usize;
    psDec
        .sLPC_Q14_buf
        .copy_from_slice(&sLTP_Q14[base..base + MAX_LPC_ORDER as usize]);

    /**************************************/
    /* Update states                      */
    /**************************************/
    psDec.sPLC.rand_seed = rand_seed;
    psDec.sPLC.randScale_Q14 = rand_scale_Q14;
    for i in 0..MAX_NB_SUBFR {
        psDecCtrl.pitchL[i as usize] = lag;
    }
}

/* Glues concealed frames with new good received frames */
/// `silk_PLC_glue_frames(psDec, frame, length)`
pub fn silk_PLC_glue_frames(
    psDec: &mut silk_decoder_state, /* I/O  decoder state */
    frame: &mut [opus_int16],       /* I/O  signal       */
    length: opus_int,               /* I    length of signal */
) {
    let mut i: opus_int;
    let mut energy: opus_int32 = 0;
    let mut energy_shift: opus_int = 0;

    if psDec.lossCnt != 0 {
        /* Calculate energy in concealed residual */
        let (e, s) = silk_sum_sqr_shift(&frame[..length as usize], length);
        psDec.sPLC.conc_energy = e;
        psDec.sPLC.conc_energy_shift = s;

        psDec.sPLC.last_frame_lost = 1;
    } else {
        if psDec.sPLC.last_frame_lost != 0 {
            /* Calculate residual in decoded signal if last frame was lost */
            let (e, s) = silk_sum_sqr_shift(&frame[..length as usize], length);
            energy = e;
            energy_shift = s;

            /* Normalize energies */
            if energy_shift > psDec.sPLC.conc_energy_shift {
                psDec.sPLC.conc_energy =
                    silk_RSHIFT(psDec.sPLC.conc_energy, energy_shift - psDec.sPLC.conc_energy_shift);
            } else if energy_shift < psDec.sPLC.conc_energy_shift {
                energy = silk_RSHIFT(energy, psDec.sPLC.conc_energy_shift - energy_shift);
            }

            /* Fade in the energy difference */
            if energy > psDec.sPLC.conc_energy {
                let mut LZ: opus_int32 = silk_CLZ32(psDec.sPLC.conc_energy);
                LZ = LZ - 1;
                psDec.sPLC.conc_energy = silk_LSHIFT(psDec.sPLC.conc_energy, LZ);
                energy = silk_RSHIFT(energy, silk_max_32(24 - LZ, 0));

                let frac_Q24: opus_int32 =
                    silk_DIV32(psDec.sPLC.conc_energy, silk_max(energy, 1));

                let gain_Q16: opus_int32 = silk_LSHIFT(silk_SQRT_APPROX(frac_Q24), 4);
                let mut slope_Q16: opus_int32 =
                    silk_DIV32_16((1 << 16) - gain_Q16, length);
                /* Make slope 4x steeper to avoid missing onsets after DTX */
                slope_Q16 = silk_LSHIFT(slope_Q16, 2);

                /* ENABLE_DEEP_PLC: the C skips this loop when fs_kHz == 16. */
                if psDec.sPLC.fs_kHz != 16 {
                    let mut g = gain_Q16;
                    for i2 in 0..length {
                        i = i2;
                        frame[i as usize] =
                            silk_SMULWB(g, frame[i as usize] as opus_int32) as opus_int16;
                        g += slope_Q16;
                        if g > (1 << 16) {
                            break;
                        }
                    }
                }
            }
        }
        psDec.sPLC.last_frame_lost = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plc_reset_defaults() {
        let mut st = silk_decoder_state::default();
        st.frame_length = 320;
        silk_PLC_Reset(&mut st);
        assert_eq!(st.sPLC.pitchL_Q8, 320 << 7);
        assert_eq!(st.sPLC.prevGain_Q16, [65536, 65536]);
        assert_eq!(st.sPLC.subfr_length, 20);
        assert_eq!(st.sPLC.nb_subfr, 2);
    }

    #[test]
    fn plc_energy_scales_the_last_two_subframes() {
        let mut exc = [0 as opus_int32; MAX_FRAME_LENGTH as usize];
        /* nb_subfr = 4: the function looks at subframes nb_subfr-2 and nb_subfr-1,
           i.e. exc[160..320].  Make the first of the two much louder. */
        for i in 0..80 {
            exc[160 + i] = 1 << 20;
            exc[240 + i] = 1 << 14;
        }
        let prev = [1 << 16, 1 << 16];
        let mut e1 = 0;
        let mut s1 = 0;
        let mut e2 = 0;
        let mut s2 = 0;
        silk_PLC_energy(&mut e1, &mut s1, &mut e2, &mut s2, &exc, &prev, 80, 4);
        assert!(e1 > 0 && e2 > 0);
        assert!(e1 > e2);
        /* the shifts absorb the dynamic range so both energies stay comparable */
        assert!(silk_RSHIFT(e1, s2) > silk_RSHIFT(e2, s1));
    }

    #[test]
    fn glue_frames_records_last_frame_lost() {
        let mut st = silk_decoder_state::default();
        let mut frame = [0 as opus_int16; 320];
        st.lossCnt = 1;
        silk_PLC_glue_frames(&mut st, &mut frame, 320);
        assert_eq!(st.sPLC.last_frame_lost, 1);
        assert!(st.sPLC.conc_energy >= 0);
        st.lossCnt = 0;
        silk_PLC_glue_frames(&mut st, &mut frame, 320);
        assert_eq!(st.sPLC.last_frame_lost, 0);
    }
}

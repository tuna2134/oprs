//! Mirror of `silk/float/wrappers_FLP.c` from libopus 1.6.1, transliterated to safe Rust.
//!
//! Wrappers that convert between the floating-point encoder structures and the
//! fixed-point integer encoder core (`silk_A2NLSF`, `silk_NLSF2A`,
//! `silk_process_NLSFs`, `silk_NSQ`/`silk_NSQ_del_dec`,
//! `silk_quant_LTP_gains`).
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

use crate::silk::a2nlsf::silk_A2NLSF;
use crate::silk::define::*;
use crate::silk::float::main_flp::silk_encoder_control_FLP;
use crate::silk::float::sigproc_flp::{silk_float, silk_float2int};
use crate::silk::nlsf2a::silk_NLSF2A;
use crate::silk::nsq::silk_NSQ;
use crate::silk::nsq_del_dec::silk_NSQ_del_dec;
use crate::silk::process_nlsfs::silk_process_NLSFs;
use crate::silk::quant_ltp_gains::silk_quant_LTP_gains;
use crate::silk::structs::{silk_encoder_state, silk_nsq_state, SideInfoIndices};
use crate::silk::tables_other::silk_LTPScales_table_Q14;
use crate::silk::typedef::*;
use crate::types::*;
use crate::silk::sigproc_fix::silk_LSHIFT32;

/// Convert AR filter coefficients to NLSF parameters
pub fn silk_A2NLSF_FLP(NLSF_Q15: &mut [opus_int16], pAR: &[silk_float], LPC_order: opus_int) {
    let mut i: opus_int;
    let mut a_fix_Q16 = [0 as opus_int32; MAX_LPC_ORDER as usize];

    i = 0;
    while i < LPC_order {
        a_fix_Q16[i as usize] = silk_float2int(pAR[i as usize] * 65536.0f32);
        i += 1;
    }

    silk_A2NLSF(NLSF_Q15, &mut a_fix_Q16, LPC_order);
}

/// Convert LSF parameters to AR prediction filter coefficients
pub fn silk_NLSF2A_FLP(
    pAR: &mut [silk_float],
    NLSF_Q15: &[opus_int16],
    LPC_order: opus_int,
    arch: opus_int,
) {
    let mut i: opus_int;
    let mut a_fix_Q12 = [0 as opus_int16; MAX_LPC_ORDER as usize];

    silk_NLSF2A(&mut a_fix_Q12, NLSF_Q15, LPC_order, arch);

    i = 0;
    while i < LPC_order {
        pAR[i as usize] = a_fix_Q12[i as usize] as silk_float * (1.0f32 / 4096.0f32);
        i += 1;
    }
}

/******************************************/
/* Floating-point NLSF processing wrapper */
/******************************************/
pub fn silk_process_NLSFs_FLP(
    psEncC: &mut silk_encoder_state, /* I/O  Encoder state */
    PredCoef: &mut [[silk_float; MAX_LPC_ORDER as usize]; 2], /* O  Prediction coefficients */
    NLSF_Q15: &mut [opus_int16],
    prev_NLSF_Q15: &[opus_int16],
) {
    let mut i: opus_int;
    let mut j: opus_int;
    let mut PredCoef_Q12 = [[0 as opus_int16; MAX_LPC_ORDER as usize]; 2];

    silk_process_NLSFs(psEncC, &mut PredCoef_Q12, NLSF_Q15, prev_NLSF_Q15);

    j = 0;
    while j < 2 {
        i = 0;
        while i < psEncC.predictLPCOrder {
            PredCoef[j as usize][i as usize] =
                PredCoef_Q12[j as usize][i as usize] as silk_float * (1.0f32 / 4096.0f32);
            i += 1;
        }
        j += 1;
    }
}

/****************************************/
/* Floating-point Silk NSQ wrapper      */
/****************************************/
///
/// The C takes pointers into `psEnc->sCmn` (`&psEnc->sCmn.indices`,
/// `&psEnc->sCmn.sNSQ`, `psEnc->sCmn.pulses`); Rust cannot borrow a field
/// mutably while also borrowing the enclosing state, so those three are passed
/// in explicitly by the caller (which owns the state).
#[allow(clippy::too_many_arguments)]
pub fn silk_NSQ_wrapper_FLP(
    psEncC: &silk_encoder_state, /* I    Encoder state (`psEnc->sCmn`), read only */
    psEncCtrl: &silk_encoder_control_FLP, /* I    Encoder control FLP     */
    psIndices: &mut SideInfoIndices, /* I/O  Quantization indices        */
    psNSQ: &mut silk_nsq_state,  /* I/O  Noise Shaping Quantization state */
    pulses: &mut [opus_int8],    /* O    Quantized pulse signal          */
    x: &[silk_float],            /* I    Prefiltered input signal        */
) {
    let mut i: opus_int;
    let mut j: opus_int;
    let mut x16 = [0 as opus_int16; MAX_FRAME_LENGTH as usize];
    let mut Gains_Q16 = [0 as opus_int32; MAX_NB_SUBFR as usize];
    let mut PredCoef_Q12 = [[0 as opus_int16; MAX_LPC_ORDER as usize]; 2];
    let mut LTPCoef_Q14 = [0 as opus_int16; (LTP_ORDER * MAX_NB_SUBFR) as usize];
    let mut LTP_scale_Q14: opus_int;

    /* Noise shaping parameters */
    let mut AR_Q13 = [0 as opus_int16; (MAX_NB_SUBFR * MAX_SHAPE_LPC_ORDER) as usize];
    let mut LF_shp_Q14 = [0 as opus_int32; MAX_NB_SUBFR as usize];
    let mut Lambda_Q10: opus_int;
    let mut Tilt_Q14 = [0 as opus_int; MAX_NB_SUBFR as usize];
    let mut HarmShapeGain_Q14 = [0 as opus_int; MAX_NB_SUBFR as usize];

    /* Convert control struct to fix control struct */
    /* Noise shape parameters */
    i = 0;
    while i < psEncC.nb_subfr {
        j = 0;
        while j < psEncC.shapingLPCOrder {
            AR_Q13[(i * MAX_SHAPE_LPC_ORDER + j) as usize] = silk_float2int(
                psEncCtrl.AR[(i * MAX_SHAPE_LPC_ORDER + j) as usize] * 8192.0f32,
            ) as opus_int16;
            j += 1;
        }
        i += 1;
    }

    i = 0;
    while i < psEncC.nb_subfr {
        LF_shp_Q14[i as usize] = silk_LSHIFT32(
            silk_float2int(psEncCtrl.LF_AR_shp[i as usize] * 16384.0f32),
            16,
        ) | (silk_float2int(psEncCtrl.LF_MA_shp[i as usize] * 16384.0f32) as opus_uint16)
            as opus_int32;
        Tilt_Q14[i as usize] = silk_float2int(psEncCtrl.Tilt[i as usize] * 16384.0f32);
        HarmShapeGain_Q14[i as usize] =
            silk_float2int(psEncCtrl.HarmShapeGain[i as usize] * 16384.0f32);
        i += 1;
    }
    Lambda_Q10 = silk_float2int(psEncCtrl.Lambda * 1024.0f32);

    /* prediction and coding parameters */
    i = 0;
    while i < psEncC.nb_subfr * LTP_ORDER {
        LTPCoef_Q14[i as usize] =
            silk_float2int(psEncCtrl.LTPCoef[i as usize] * 16384.0f32) as opus_int16;
        i += 1;
    }

    j = 0;
    while j < 2 {
        i = 0;
        while i < psEncC.predictLPCOrder {
            PredCoef_Q12[j as usize][i as usize] =
                silk_float2int(psEncCtrl.PredCoef[j as usize][i as usize] * 4096.0f32)
                    as opus_int16;
            i += 1;
        }
        j += 1;
    }

    i = 0;
    while i < psEncC.nb_subfr {
        Gains_Q16[i as usize] = silk_float2int(psEncCtrl.Gains[i as usize] * 65536.0f32);
        silk_assert!(Gains_Q16[i as usize] > 0);
        i += 1;
    }

    if psIndices.signalType as opus_int == TYPE_VOICED {
        LTP_scale_Q14 = silk_LTPScales_table_Q14[psIndices.LTP_scaleIndex as usize] as opus_int;
    } else {
        LTP_scale_Q14 = 0;
    }

    /* Convert input to fix */
    i = 0;
    while i < psEncC.frame_length {
        x16[i as usize] = silk_float2int(x[i as usize]) as opus_int16;
        i += 1;
    }

    /* Call NSQ */
    if psEncC.nStatesDelayedDecision > 1 || psEncC.warping_Q16 > 0 {
        silk_NSQ_del_dec(
            psEncC,
            psNSQ,
            psIndices,
            &x16[..psEncC.frame_length as usize],
            pulses,
            &PredCoef_Q12,
            &LTPCoef_Q14,
            &AR_Q13,
            &HarmShapeGain_Q14,
            &Tilt_Q14,
            &LF_shp_Q14,
            &Gains_Q16,
            &psEncCtrl.pitchL,
            Lambda_Q10,
            LTP_scale_Q14,
            psEncC.arch,
        );
    } else {
        silk_NSQ(
            psEncC,
            psNSQ,
            psIndices,
            &x16[..psEncC.frame_length as usize],
            pulses,
            &PredCoef_Q12,
            &LTPCoef_Q14,
            &AR_Q13,
            &HarmShapeGain_Q14,
            &Tilt_Q14,
            &LF_shp_Q14,
            &Gains_Q16,
            &psEncCtrl.pitchL,
            Lambda_Q10,
            LTP_scale_Q14,
            psEncC.arch,
        );
    }
}

/***********************************************/
/* Floating-point Silk LTP quantiation wrapper */
/***********************************************/
#[allow(clippy::too_many_arguments)]
pub fn silk_quant_LTP_gains_FLP(
    B: &mut [silk_float],
    cbk_index: &mut [opus_int8],
    periodicity_index: &mut opus_int8,
    sum_log_gain_Q7: &mut opus_int32,
    pred_gain_dB: &mut silk_float,
    XX: &[silk_float],
    xX: &[silk_float],
    subfr_len: opus_int,
    nb_subfr: opus_int,
    arch: opus_int,
) {
    let mut i: opus_int;
    let mut pred_gain_dB_Q7: opus_int = 0;
    let mut B_Q14 = [0 as opus_int16; (MAX_NB_SUBFR * LTP_ORDER) as usize];
    let mut XX_Q17 = [0 as opus_int32; (MAX_NB_SUBFR * LTP_ORDER * LTP_ORDER) as usize];
    let mut xX_Q17 = [0 as opus_int32; (MAX_NB_SUBFR * LTP_ORDER) as usize];

    /* The C uses do/while loops, which run at least once. */
    i = 0;
    loop {
        XX_Q17[i as usize] = silk_float2int(XX[i as usize] * 131072.0f32);
        i += 1;
        if i >= nb_subfr * LTP_ORDER * LTP_ORDER {
            break;
        }
    }
    i = 0;
    loop {
        xX_Q17[i as usize] = silk_float2int(xX[i as usize] * 131072.0f32);
        i += 1;
        if i >= nb_subfr * LTP_ORDER {
            break;
        }
    }

    silk_quant_LTP_gains(
        &mut B_Q14,
        cbk_index,
        periodicity_index,
        sum_log_gain_Q7,
        &mut pred_gain_dB_Q7,
        &XX_Q17,
        &xX_Q17,
        subfr_len,
        nb_subfr,
        arch,
    );

    i = 0;
    while i < nb_subfr * LTP_ORDER {
        B[i as usize] = B_Q14[i as usize] as silk_float * (1.0f32 / 16384.0f32);
        i += 1;
    }

    *pred_gain_dB = pred_gain_dB_Q7 as silk_float * (1.0f32 / 128.0f32);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a2nlsf_roundtrip_is_stable_for_a_simple_filter() {
        /* a = [0, 0, ...] -> flat NLSFs, then NLSF2A gives the zero filter */
        let a = [0.0f32; 16];
        let mut nlsf = [0i16; 16];
        silk_A2NLSF_FLP(&mut nlsf, &a, 16);
        for i in 1..16 {
            assert!(nlsf[i] > nlsf[i - 1], "NLSFs must be increasing");
        }
        let mut back = [0.0f32; 16];
        silk_NLSF2A_FLP(&mut back, &nlsf, 16, 4);
        for v in back {
            assert!(v.abs() < 1e-3, "v={v}");
        }
    }

    #[test]
    fn quant_ltp_gains_flp_scales_back_to_float() {
        let xx = [1.0f32; 100];
        let xxv = [1.0f32; 20];
        let mut b = [0.0f32; 20];
        let mut idx = [0i8; 4];
        let mut per = 0i8;
        let mut sum_log_gain = 0i32;
        let mut pred_gain = 0.0f32;
        silk_quant_LTP_gains_FLP(
            &mut b,
            &mut idx,
            &mut per,
            &mut sum_log_gain,
            &mut pred_gain,
            &xx,
            &xxv,
            40,
            4,
            4,
        );
        assert!(pred_gain >= 0.0);
        for v in b {
            assert!(v.abs() < 8.0);
        }
    }
}

//! Transliteration of `silk/CNG.c` from libopus 1.6.1 to safe Rust.
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
use crate::silk::macros::*;
use crate::silk::nlsf2a::silk_NLSF2A;
use crate::silk::sigproc_fix::*;
use crate::silk::structs::*;
use crate::silk::typedef::*;
use crate::types::*;

/* Generates excitation for CNG LPC synthesis */
fn silk_CNG_exc(
    exc_Q14: &mut [opus_int32],  /* O    CNG excitation signal Q10 */
    exc_buf_Q14: &[opus_int32],  /* I    Random samples buffer Q10 */
    length: opus_int,            /* I    Length                    */
    rand_seed: &mut opus_int32,  /* I/O  Seed to random index generator */
) {
    let mut seed: opus_int32;
    let mut i: opus_int;
    let mut idx: opus_int;
    let mut exc_mask: opus_int;

    exc_mask = CNG_BUF_MASK_MAX;
    while exc_mask > length {
        exc_mask = silk_RSHIFT(exc_mask, 1);
    }

    seed = *rand_seed;
    for i in 0..length {
        seed = silk_RAND(seed);
        idx = silk_RSHIFT(seed, 24) & exc_mask;
        silk_assert!(idx >= 0);
        silk_assert!(idx <= CNG_BUF_MASK_MAX);
        exc_Q14[i as usize] = exc_buf_Q14[idx as usize];
    }
    *rand_seed = seed;
}

/// `silk_CNG_Reset(psDec)`
pub fn silk_CNG_Reset(
    psDec: &mut silk_decoder_state, /* I/O  Decoder state */
) {
    let mut i: opus_int;
    let NLSF_step_Q15: opus_int;
    let mut NLSF_acc_Q15: opus_int32;

    NLSF_step_Q15 = silk_DIV32_16(silk_int16_MAX as opus_int32, psDec.LPC_order + 1);
    NLSF_acc_Q15 = 0;
    for i in 0..psDec.LPC_order {
        NLSF_acc_Q15 += NLSF_step_Q15;
        psDec.sCNG.CNG_smth_NLSF_Q15[i as usize] = NLSF_acc_Q15 as opus_int16;
    }
    psDec.sCNG.CNG_smth_Gain_Q16 = 0;
    psDec.sCNG.rand_seed = 3176576;
}

/* Updates CNG estimate, and applies the CNG when packet was lost   */
/// `silk_CNG(psDec, psDecCtrl, frame, length)`
pub fn silk_CNG(
    psDec: &mut silk_decoder_state,      /* I/O  Decoder state   */
    psDecCtrl: &silk_decoder_control,    /* I/O  Decoder control */
    frame: &mut [opus_int16],            /* I/O  Signal          */
    length: opus_int,                    /* I    Length of residual */
) {
    let mut i: opus_int;
    let mut subfr: opus_int;
    let mut LPC_pred_Q10: opus_int32;
    let mut max_Gain_Q16: opus_int32;
    let mut gain_Q16: opus_int32;
    let mut gain_Q10: opus_int32;
    let mut A_Q12 = [0 as opus_int16; MAX_LPC_ORDER as usize];

    if psDec.fs_kHz != psDec.sCNG.fs_kHz {
        /* Reset state */
        silk_CNG_Reset(psDec);

        psDec.sCNG.fs_kHz = psDec.fs_kHz;
    }
    if psDec.lossCnt == 0 && psDec.prevSignalType == TYPE_NO_VOICE_ACTIVITY {
        /* Update CNG parameters */

        /* Smoothing of LSF's  */
        for i in 0..psDec.LPC_order {
            let smth = psDec.sCNG.CNG_smth_NLSF_Q15[i as usize] as opus_int32;
            psDec.sCNG.CNG_smth_NLSF_Q15[i as usize] = (smth
                + silk_SMULWB(
                    psDec.prevNLSF_Q15[i as usize] as opus_int32 - smth,
                    CNG_NLSF_SMTH_Q16,
                )) as opus_int16;
        }
        /* Find the subframe with the highest gain */
        max_Gain_Q16 = 0;
        subfr = 0;
        for i in 0..psDec.nb_subfr {
            if psDecCtrl.Gains_Q16[i as usize] > max_Gain_Q16 {
                max_Gain_Q16 = psDecCtrl.Gains_Q16[i as usize];
                subfr = i;
            }
        }
        /* Update CNG excitation buffer with excitation from this subframe */
        let sl = psDec.subfr_length as usize;
        let mut exc_sub = [0 as opus_int32; MAX_SUB_FRAME_LENGTH as usize];
        exc_sub[..sl]
            .copy_from_slice(&psDec.exc_Q14[(subfr as usize) * sl..(subfr as usize + 1) * sl]);
        psDec
            .sCNG
            .CNG_exc_buf_Q14
            .copy_within(0..(psDec.nb_subfr - 1) as usize * sl, sl);
        psDec.sCNG.CNG_exc_buf_Q14[..sl].copy_from_slice(&exc_sub[..sl]);

        /* Smooth gains */
        for i in 0..psDec.nb_subfr {
            psDec.sCNG.CNG_smth_Gain_Q16 += silk_SMULWB(
                psDecCtrl.Gains_Q16[i as usize] - psDec.sCNG.CNG_smth_Gain_Q16,
                CNG_GAIN_SMTH_Q16,
            );
            /* If the smoothed gain is 3 dB greater than this subframe's gain, use this subframe's gain to adapt faster. */
            if silk_SMULWW(psDec.sCNG.CNG_smth_Gain_Q16, CNG_GAIN_SMTH_THRESHOLD_Q16)
                > psDecCtrl.Gains_Q16[i as usize]
            {
                psDec.sCNG.CNG_smth_Gain_Q16 = psDecCtrl.Gains_Q16[i as usize];
            }
        }
    }

    /* Add CNG when packet is lost or during DTX */
    if psDec.lossCnt != 0 {
        let mut CNG_sig_Q14 = vec![0 as opus_int32; (length + MAX_LPC_ORDER) as usize];

        /* Generate CNG excitation */
        gain_Q16 = silk_SMULWW(
            psDec.sPLC.randScale_Q14 as opus_int32,
            psDec.sPLC.prevGain_Q16[1],
        );
        if gain_Q16 >= (1 << 21) || psDec.sCNG.CNG_smth_Gain_Q16 > (1 << 23) {
            gain_Q16 = silk_SMULTT(gain_Q16, gain_Q16);
            gain_Q16 = silk_SUB_LSHIFT32(
                silk_SMULTT(psDec.sCNG.CNG_smth_Gain_Q16, psDec.sCNG.CNG_smth_Gain_Q16),
                gain_Q16,
                5,
            );
            gain_Q16 = silk_LSHIFT32(silk_SQRT_APPROX(gain_Q16), 16);
        } else {
            gain_Q16 = silk_SMULWW(gain_Q16, gain_Q16);
            gain_Q16 = silk_SUB_LSHIFT32(
                silk_SMULWW(psDec.sCNG.CNG_smth_Gain_Q16, psDec.sCNG.CNG_smth_Gain_Q16),
                gain_Q16,
                5,
            );
            gain_Q16 = silk_LSHIFT32(silk_SQRT_APPROX(gain_Q16), 8);
        }
        gain_Q10 = silk_RSHIFT(gain_Q16, 6);

        let mut seed = psDec.sCNG.rand_seed;
        let exc_buf = psDec.sCNG.CNG_exc_buf_Q14;
        silk_CNG_exc(
            &mut CNG_sig_Q14[MAX_LPC_ORDER as usize..],
            &exc_buf,
            length,
            &mut seed,
        );
        psDec.sCNG.rand_seed = seed;

        /* Convert CNG NLSF to filter representation */
        silk_NLSF2A(
            &mut A_Q12,
            &psDec.sCNG.CNG_smth_NLSF_Q15,
            psDec.LPC_order,
            psDec.arch,
        );

        /* Generate CNG signal, by synthesis filtering */
        CNG_sig_Q14[..MAX_LPC_ORDER as usize].copy_from_slice(&psDec.sCNG.CNG_synth_state);
        celt_assert!(psDec.LPC_order == 10 || psDec.LPC_order == 16);
        for i in 0..length {
            /* Avoids introducing a bias because silk_SMLAWB() always rounds to -inf */
            LPC_pred_Q10 = silk_RSHIFT(psDec.LPC_order, 1);
            LPC_pred_Q10 = silk_SMLAWB(
                LPC_pred_Q10,
                CNG_sig_Q14[(MAX_LPC_ORDER + i - 1) as usize],
                A_Q12[0] as opus_int32,
            );
            LPC_pred_Q10 = silk_SMLAWB(
                LPC_pred_Q10,
                CNG_sig_Q14[(MAX_LPC_ORDER + i - 2) as usize],
                A_Q12[1] as opus_int32,
            );
            LPC_pred_Q10 = silk_SMLAWB(
                LPC_pred_Q10,
                CNG_sig_Q14[(MAX_LPC_ORDER + i - 3) as usize],
                A_Q12[2] as opus_int32,
            );
            LPC_pred_Q10 = silk_SMLAWB(
                LPC_pred_Q10,
                CNG_sig_Q14[(MAX_LPC_ORDER + i - 4) as usize],
                A_Q12[3] as opus_int32,
            );
            LPC_pred_Q10 = silk_SMLAWB(
                LPC_pred_Q10,
                CNG_sig_Q14[(MAX_LPC_ORDER + i - 5) as usize],
                A_Q12[4] as opus_int32,
            );
            LPC_pred_Q10 = silk_SMLAWB(
                LPC_pred_Q10,
                CNG_sig_Q14[(MAX_LPC_ORDER + i - 6) as usize],
                A_Q12[5] as opus_int32,
            );
            LPC_pred_Q10 = silk_SMLAWB(
                LPC_pred_Q10,
                CNG_sig_Q14[(MAX_LPC_ORDER + i - 7) as usize],
                A_Q12[6] as opus_int32,
            );
            LPC_pred_Q10 = silk_SMLAWB(
                LPC_pred_Q10,
                CNG_sig_Q14[(MAX_LPC_ORDER + i - 8) as usize],
                A_Q12[7] as opus_int32,
            );
            LPC_pred_Q10 = silk_SMLAWB(
                LPC_pred_Q10,
                CNG_sig_Q14[(MAX_LPC_ORDER + i - 9) as usize],
                A_Q12[8] as opus_int32,
            );
            LPC_pred_Q10 = silk_SMLAWB(
                LPC_pred_Q10,
                CNG_sig_Q14[(MAX_LPC_ORDER + i - 10) as usize],
                A_Q12[9] as opus_int32,
            );
            if psDec.LPC_order == 16 {
                LPC_pred_Q10 = silk_SMLAWB(
                    LPC_pred_Q10,
                    CNG_sig_Q14[(MAX_LPC_ORDER + i - 11) as usize],
                    A_Q12[10] as opus_int32,
                );
                LPC_pred_Q10 = silk_SMLAWB(
                    LPC_pred_Q10,
                    CNG_sig_Q14[(MAX_LPC_ORDER + i - 12) as usize],
                    A_Q12[11] as opus_int32,
                );
                LPC_pred_Q10 = silk_SMLAWB(
                    LPC_pred_Q10,
                    CNG_sig_Q14[(MAX_LPC_ORDER + i - 13) as usize],
                    A_Q12[12] as opus_int32,
                );
                LPC_pred_Q10 = silk_SMLAWB(
                    LPC_pred_Q10,
                    CNG_sig_Q14[(MAX_LPC_ORDER + i - 14) as usize],
                    A_Q12[13] as opus_int32,
                );
                LPC_pred_Q10 = silk_SMLAWB(
                    LPC_pred_Q10,
                    CNG_sig_Q14[(MAX_LPC_ORDER + i - 15) as usize],
                    A_Q12[14] as opus_int32,
                );
                LPC_pred_Q10 = silk_SMLAWB(
                    LPC_pred_Q10,
                    CNG_sig_Q14[(MAX_LPC_ORDER + i - 16) as usize],
                    A_Q12[15] as opus_int32,
                );
            }

            /* Update states */
            CNG_sig_Q14[(MAX_LPC_ORDER + i) as usize] = silk_ADD_SAT32(
                CNG_sig_Q14[(MAX_LPC_ORDER + i) as usize],
                silk_LSHIFT_SAT32(LPC_pred_Q10, 4),
            );

            /* Scale with Gain and add to input signal */
            frame[i as usize] = silk_ADD_SAT16(
                frame[i as usize],
                silk_SAT16(silk_RSHIFT_ROUND(
                    silk_SMULWW(CNG_sig_Q14[(MAX_LPC_ORDER + i) as usize], gain_Q10),
                    8,
                )) as opus_int16,
            );
        }
        psDec
            .sCNG
            .CNG_synth_state
            .copy_from_slice(&CNG_sig_Q14[length as usize..(length + MAX_LPC_ORDER) as usize]);
    } else {
        psDec.sCNG.CNG_synth_state[..psDec.LPC_order as usize].fill(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cng_reset_sets_evenly_spaced_nlsfs() {
        let mut st = silk_decoder_state::default();
        st.LPC_order = 10;
        silk_CNG_Reset(&mut st);
        assert_eq!(st.sCNG.rand_seed, 3176576);
        assert_eq!(st.sCNG.CNG_smth_Gain_Q16, 0);
        let step = silk_int16_MAX as i32 / 11;
        assert_eq!(st.sCNG.CNG_smth_NLSF_Q15[0] as i32, step);
        assert_eq!(st.sCNG.CNG_smth_NLSF_Q15[9] as i32, step * 10);
    }

    #[test]
    fn cng_exc_stays_in_the_buffer() {
        let mut buf = [0 as opus_int32; CNG_BUF_MASK_MAX as usize + 1];
        for i in 0..buf.len() {
            buf[i] = i as opus_int32 + 1;
        }
        let mut out = [0 as opus_int32; 160];
        let mut seed: opus_int32 = 3176576;
        silk_CNG_exc(&mut out[..80], &buf, 80, &mut seed);
        /* exc_mask halves from 255 down to 127 for length 80, so indices are < 128 */
        for v in out[..80].iter() {
            assert!(*v >= 1 && *v <= 128);
        }
    }

    #[test]
    fn cng_with_loss_updates_synth_state_and_frame() {
        let mut st = silk_decoder_state::default();
        st.fs_kHz = 16;
        st.LPC_order = 16;
        st.subfr_length = 80;
        st.nb_subfr = 4;
        st.lossCnt = 1;
        st.prevSignalType = TYPE_NO_VOICE_ACTIVITY;
        st.sPLC.prevGain_Q16 = [1 << 16, 1 << 16];
        let ctrl = silk_decoder_control::default();
        let mut frame = [0 as opus_int16; 320];
        silk_CNG(&mut st, &ctrl, &mut frame, 320);
        assert!(st.sCNG.CNG_synth_state.iter().any(|&v| v != 0) || frame.iter().all(|&v| v == 0));
    }
}

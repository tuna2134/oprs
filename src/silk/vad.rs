//! Mirror of `silk/VAD.c` from libopus 1.6.1, transliterated to safe Rust.
//!
//! At arch 3/4 the reference build dispatches `silk_VAD_GetSA_Q8` to
//! `silk_VAD_GetSA_Q8_sse4_1` (`silk/x86/VAD_sse4_1.c`).  Only the per-subframe
//! energy accumulation is vectorised there, and `docs/rtcd-kernels.md` §4.1
//! proves it is bit-identical to the C version: every term is an exact int16
//! product accumulated in int32, and int32 addition is associative modulo
//! 2^32, while the source documents the bound (`dec_subframe_length <= 128`,
//! energy < 128 * (2^12)^2) that rules out any saturation difference.  The port
//! is therefore the C body for every arch.
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
use crate::silk::ana_filt_bank_1::*;
use crate::silk::define::*;
use crate::silk::inlines::*;
use crate::silk::lin2log::*;
use crate::silk::macros::*;
use crate::silk::sigm_q15::*;
use crate::silk::sigproc_fix::*;
use crate::silk::structs::*;
use crate::silk::typedef::silk_assert;
use crate::silk::typedef::*;
use crate::types::*;

/**********************************/
/* Initialization of the Silk VAD */
/**********************************/
pub fn silk_VAD_Init(psSilk_VAD: &mut silk_VAD_state) -> opus_int {
    /* reset state memory */
    *psSilk_VAD = silk_VAD_state::default();

    /* init noise levels */
    /* Initialize array with approx pink noise levels (psd proportional to inverse of frequency) */
    for b in 0..VAD_N_BANDS {
        psSilk_VAD.NoiseLevelBias[b as usize] =
            silk_max_32(silk_DIV32_16(VAD_NOISE_LEVELS_BIAS, b + 1), 1);
    }

    /* Initialize state */
    for b in 0..VAD_N_BANDS {
        psSilk_VAD.NL[b as usize] = silk_MUL(100, psSilk_VAD.NoiseLevelBias[b as usize]);
        psSilk_VAD.inv_NL[b as usize] = silk_DIV32(silk_int32_MAX, psSilk_VAD.NL[b as usize]);
    }
    psSilk_VAD.counter = 15;

    /* init smoothed energy-to-noise ratio*/
    for b in 0..VAD_N_BANDS {
        psSilk_VAD.NrgRatioSmth_Q8[b as usize] = 100 * 256; /* 100 * 256 --> 20 dB SNR */
    }

    0
}

/* Weighting factors for tilt measure */
static tiltWeights: [opus_int32; VAD_N_BANDS as usize] = [30000, 6000, -12000, -12000];

/***************************************/
/* Get the speech activity level in Q8 */
/***************************************/
pub fn silk_VAD_GetSA_Q8_c(psEncC: &mut silk_encoder_state, pIn: &[opus_int16]) -> opus_int {
    let mut SA_Q15: opus_int;
    let mut pSNR_dB_Q7: opus_int;
    let mut input_tilt: opus_int;
    let mut decimated_framelength1: opus_int;
    let mut decimated_framelength2: opus_int;
    let mut decimated_framelength: opus_int;
    let mut dec_subframe_length: opus_int;
    let mut dec_subframe_offset: opus_int;
    let mut SNR_Q7: opus_int;
    let mut sumSquared: opus_int32 = 0;
    let mut smooth_coef_Q16: opus_int32;
    let mut Xnrg = [0 as opus_int32; VAD_N_BANDS as usize];
    let mut NrgToNoiseRatio_Q8 = [0 as opus_int32; VAD_N_BANDS as usize];
    let mut speech_nrg: opus_int32;
    let mut x_tmp: opus_int32;
    let mut X_offset = [0 as opus_int; VAD_N_BANDS as usize];

    /* Safety checks */
    silk_assert!(VAD_N_BANDS == 4);
    celt_assert!(MAX_FRAME_LENGTH >= psEncC.frame_length);
    celt_assert!(psEncC.frame_length <= 512);
    celt_assert!(psEncC.frame_length == 8 * silk_RSHIFT(psEncC.frame_length, 3));

    /***********************/
    /* Filter and Decimate */
    /***********************/
    decimated_framelength1 = silk_RSHIFT(psEncC.frame_length, 1);
    decimated_framelength2 = silk_RSHIFT(psEncC.frame_length, 2);
    decimated_framelength = silk_RSHIFT(psEncC.frame_length, 3);
    /* Decimate into 4 bands:
       0       L      3L       L              3L                             5L
               -      --       -              --                             --
               8       8       2               4                              4

       [0-1 kHz| temp. |1-2 kHz|    2-4 kHz    |            4-8 kHz           |

       They're arranged to allow the minimal ( frame_length / 4 ) extra
       scratch space during the downsampling process */
    X_offset[0] = 0;
    X_offset[1] = decimated_framelength + decimated_framelength2;
    X_offset[2] = X_offset[1] + decimated_framelength;
    X_offset[3] = X_offset[2] + decimated_framelength2;
    let mut X = vec![0 as opus_int16; (X_offset[3] + decimated_framelength1) as usize];

    /* 0-8 kHz to 0-4 kHz and 4-8 kHz */
    {
        let (lo, hi) = X.split_at_mut(X_offset[3] as usize);
        silk_ana_filt_bank_1(
            pIn,
            &mut psEncC.sVAD.AnaState,
            &mut lo[0..(psEncC.frame_length >> 1) as usize],
            &mut hi[0..(psEncC.frame_length >> 1) as usize],
            psEncC.frame_length,
        );
    }

    /* 0-4 kHz to 0-2 kHz and 2-4 kHz */
    {
        let tmp: Vec<opus_int16> = X[..decimated_framelength1 as usize].to_vec();
        let (lo, hi) = X.split_at_mut(X_offset[2] as usize);
        silk_ana_filt_bank_1(
            &tmp,
            &mut psEncC.sVAD.AnaState1,
            &mut lo[0..(decimated_framelength1 >> 1) as usize],
            &mut hi[0..(decimated_framelength1 >> 1) as usize],
            decimated_framelength1,
        );
    }

    /* 0-2 kHz to 0-1 kHz and 1-2 kHz */
    {
        let tmp: Vec<opus_int16> = X[..decimated_framelength2 as usize].to_vec();
        let (lo, hi) = X.split_at_mut(X_offset[1] as usize);
        silk_ana_filt_bank_1(
            &tmp,
            &mut psEncC.sVAD.AnaState2,
            &mut lo[0..(decimated_framelength2 >> 1) as usize],
            &mut hi[0..(decimated_framelength2 >> 1) as usize],
            decimated_framelength2,
        );
    }

    /*********************************************/
    /* HP filter on lowest band (differentiator) */
    /*********************************************/
    X[(decimated_framelength - 1) as usize] =
        silk_RSHIFT(X[(decimated_framelength - 1) as usize] as opus_int32, 1) as opus_int16;
    let HPstateTmp = X[(decimated_framelength - 1) as usize];
    let mut i = decimated_framelength - 1;
    while i > 0 {
        X[(i - 1) as usize] = silk_RSHIFT(X[(i - 1) as usize] as opus_int32, 1) as opus_int16;
        X[i as usize] = X[i as usize].wrapping_sub(X[(i - 1) as usize]);
        i -= 1;
    }
    X[0] = X[0].wrapping_sub(psEncC.sVAD.HPstate);
    psEncC.sVAD.HPstate = HPstateTmp;

    /*************************************/
    /* Calculate the energy in each band */
    /*************************************/
    for b in 0..VAD_N_BANDS {
        /* Find the decimated framelength in the non-uniformly divided bands */
        decimated_framelength = silk_RSHIFT(
            psEncC.frame_length,
            silk_min_int(VAD_N_BANDS - b, VAD_N_BANDS - 1),
        );

        /* Split length into subframe lengths */
        dec_subframe_length = silk_RSHIFT(decimated_framelength, VAD_INTERNAL_SUBFRAMES_LOG2);
        dec_subframe_offset = 0;

        /* Compute energy per sub-frame */
        /* initialize with summed energy of last subframe */
        Xnrg[b as usize] = psEncC.sVAD.XnrgSubfr[b as usize];
        for s in 0..VAD_INTERNAL_SUBFRAMES {
            sumSquared = 0;
            for i in 0..dec_subframe_length {
                /* The energy will be less than dec_subframe_length * ( silk_int16_MIN / 8 ) ^ 2.            */
                /* Therefore we can accumulate with no risk of overflow (unless dec_subframe_length > 128)  */
                x_tmp = silk_RSHIFT(
                    X[(X_offset[b as usize] + i + dec_subframe_offset) as usize] as opus_int32,
                    3,
                );
                sumSquared = silk_SMLABB(sumSquared, x_tmp, x_tmp);

                /* Safety check */
                silk_assert!(sumSquared >= 0);
            }

            /* Add/saturate summed energy of current subframe */
            if s < VAD_INTERNAL_SUBFRAMES - 1 {
                Xnrg[b as usize] = silk_ADD_POS_SAT32(Xnrg[b as usize], sumSquared);
            } else {
                /* Look-ahead subframe */
                Xnrg[b as usize] =
                    silk_ADD_POS_SAT32(Xnrg[b as usize], silk_RSHIFT(sumSquared, 1));
            }

            dec_subframe_offset += dec_subframe_length;
        }
        psEncC.sVAD.XnrgSubfr[b as usize] = sumSquared;
    }

    /********************/
    /* Noise estimation */
    /********************/
    silk_VAD_GetNoiseLevels(&Xnrg, &mut psEncC.sVAD);

    /***********************************************/
    /* Signal-plus-noise to noise ratio estimation */
    /***********************************************/
    sumSquared = 0;
    input_tilt = 0;
    for b in 0..VAD_N_BANDS {
        speech_nrg = Xnrg[b as usize] - psEncC.sVAD.NL[b as usize];
        if speech_nrg > 0 {
            /* Divide, with sufficient resolution */
            if (Xnrg[b as usize] & 0xFF800000u32 as opus_int32) == 0 {
                NrgToNoiseRatio_Q8[b as usize] =
                    silk_DIV32(silk_LSHIFT(Xnrg[b as usize], 8), psEncC.sVAD.NL[b as usize] + 1);
            } else {
                NrgToNoiseRatio_Q8[b as usize] = silk_DIV32(
                    Xnrg[b as usize],
                    silk_RSHIFT(psEncC.sVAD.NL[b as usize], 8) + 1,
                );
            }

            /* Convert to log domain */
            SNR_Q7 = silk_lin2log(NrgToNoiseRatio_Q8[b as usize]) - 8 * 128;

            /* Sum-of-squares */
            sumSquared = silk_SMLABB(sumSquared, SNR_Q7, SNR_Q7); /* Q14 */

            /* Tilt measure */
            if speech_nrg < (1i32 << 20) {
                /* Scale down SNR value for small subband speech energies */
                SNR_Q7 = silk_SMULWB(silk_LSHIFT(silk_SQRT_APPROX(speech_nrg), 6), SNR_Q7);
            }
            input_tilt = silk_SMLAWB(input_tilt, tiltWeights[b as usize], SNR_Q7);
        } else {
            NrgToNoiseRatio_Q8[b as usize] = 256;
        }
    }

    /* Mean-of-squares */
    sumSquared = silk_DIV32_16(sumSquared, VAD_N_BANDS); /* Q14 */

    /* Root-mean-square approximation, scale to dBs, and write to output pointer */
    pSNR_dB_Q7 = 3 * silk_SQRT_APPROX(sumSquared); /* Q7 */

    /*********************************/
    /* Speech Probability Estimation */
    /*********************************/
    SA_Q15 = silk_sigm_Q15(silk_SMULWB(VAD_SNR_FACTOR_Q16, pSNR_dB_Q7) - VAD_NEGATIVE_OFFSET_Q5);

    /**************************/
    /* Frequency Tilt Measure */
    /**************************/
    psEncC.input_tilt_Q15 = silk_LSHIFT(silk_sigm_Q15(input_tilt) - 16384, 1);

    /**************************************************/
    /* Scale the sigmoid output based on power levels */
    /**************************************************/
    speech_nrg = 0;
    for b in 0..VAD_N_BANDS {
        /* Accumulate signal-without-noise energies, higher frequency bands have more weight */
        speech_nrg +=
            (b + 1) * silk_RSHIFT(Xnrg[b as usize] - psEncC.sVAD.NL[b as usize], 4);
    }

    if psEncC.frame_length == 20 * psEncC.fs_kHz {
        speech_nrg = silk_RSHIFT32(speech_nrg, 1);
    }
    /* Power scaling */
    if speech_nrg <= 0 {
        SA_Q15 = silk_RSHIFT(SA_Q15, 1);
    } else if speech_nrg < 16384 {
        speech_nrg = silk_LSHIFT32(speech_nrg, 16);

        /* square-root */
        speech_nrg = silk_SQRT_APPROX(speech_nrg);
        SA_Q15 = silk_SMULWB(32768 + speech_nrg, SA_Q15);
    }

    /* Copy the resulting speech activity in Q8 */
    psEncC.speech_activity_Q8 = silk_min_int(silk_RSHIFT(SA_Q15, 7), silk_uint8_MAX as opus_int);

    /***********************************/
    /* Energy Level and SNR estimation */
    /***********************************/
    /* Smoothing coefficient */
    smooth_coef_Q16 = silk_SMULWB(
        VAD_SNR_SMOOTH_COEF_Q18,
        silk_SMULWB(SA_Q15 as opus_int32, SA_Q15 as opus_int32),
    );

    if psEncC.frame_length == 10 * psEncC.fs_kHz {
        smooth_coef_Q16 >>= 1;
    }

    for b in 0..VAD_N_BANDS {
        /* compute smoothed energy-to-noise ratio per band */
        psEncC.sVAD.NrgRatioSmth_Q8[b as usize] = silk_SMLAWB(
            psEncC.sVAD.NrgRatioSmth_Q8[b as usize],
            NrgToNoiseRatio_Q8[b as usize] - psEncC.sVAD.NrgRatioSmth_Q8[b as usize],
            smooth_coef_Q16,
        );

        /* signal to noise ratio in dB per band */
        SNR_Q7 = 3 * (silk_lin2log(psEncC.sVAD.NrgRatioSmth_Q8[b as usize]) - 8 * 128);
        /* quality = sigmoid( 0.25 * ( SNR_dB - 16 ) ); */
        psEncC.input_quality_bands_Q15[b as usize] =
            silk_sigm_Q15(silk_RSHIFT(SNR_Q7 - 16 * 128, 4));
    }

    0
}

/**************************/
/* Noise level estimation */
/**************************/
pub fn silk_VAD_GetNoiseLevels(pX: &[opus_int32], psSilk_VAD: &mut silk_VAD_state) {
    let mut nl: opus_int32;
    let mut nrg: opus_int32;
    let mut inv_nrg: opus_int32;
    let mut coef: opus_int;
    let min_coef: opus_int;

    /* Initially faster smoothing */
    if psSilk_VAD.counter < 1000 {
        /* 1000 = 20 sec */
        min_coef = silk_DIV32_16(
            silk_int16_MAX as opus_int32,
            silk_RSHIFT(psSilk_VAD.counter, 4) + 1,
        );
        /* Increment frame counter */
        psSilk_VAD.counter += 1;
    } else {
        min_coef = 0;
    }

    for k in 0..VAD_N_BANDS as usize {
        /* Get old noise level estimate for current band */
        nl = psSilk_VAD.NL[k];
        silk_assert!(nl >= 0);

        /* Add bias */
        nrg = silk_ADD_POS_SAT32(pX[k], psSilk_VAD.NoiseLevelBias[k]);
        silk_assert!(nrg > 0);

        /* Invert energies */
        inv_nrg = silk_DIV32(silk_int32_MAX, nrg);
        silk_assert!(inv_nrg >= 0);

        /* Less update when subband energy is high */
        if nrg > silk_LSHIFT(nl, 3) {
            coef = VAD_NOISE_LEVEL_SMOOTH_COEF_Q16 >> 3;
        } else if nrg < nl {
            coef = VAD_NOISE_LEVEL_SMOOTH_COEF_Q16;
        } else {
            coef = silk_SMULWB(
                silk_SMULWW(inv_nrg, nl),
                VAD_NOISE_LEVEL_SMOOTH_COEF_Q16 << 1,
            );
        }

        /* Initially faster smoothing */
        coef = silk_max_int(coef, min_coef);

        /* Smooth inverse energies */
        psSilk_VAD.inv_NL[k] =
            silk_SMLAWB(psSilk_VAD.inv_NL[k], inv_nrg - psSilk_VAD.inv_NL[k], coef);
        silk_assert!(psSilk_VAD.inv_NL[k] >= 0);

        /* Compute noise level by inverting again */
        nl = silk_DIV32(silk_int32_MAX, psSilk_VAD.inv_NL[k]);
        silk_assert!(nl >= 0);

        /* Limit noise levels (guarantee 7 bits of head room) */
        nl = silk_min(nl, 0x00FFFFFF);

        /* Store as part of state */
        psSilk_VAD.NL[k] = nl;
    }
}

/// The dispatched entry (`SILK_VAD_GETSA_Q8_IMPL[(arch) & OPUS_ARCHMASK]`);
/// `arch` is ignored because the arch-3/4 implementation is bit-identical.
pub fn silk_VAD_GetSA_Q8(
    psEncC: &mut silk_encoder_state,
    pIn: &[opus_int16],
    _arch: opus_int,
) -> opus_int {
    silk_VAD_GetSA_Q8_c(psEncC, pIn)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_sets_pink_noise_levels() {
        let mut st = silk_VAD_state::default();
        assert_eq!(silk_VAD_Init(&mut st), 0);
        /* DIV32_16(50, b+1) clamped to >= 1 */
        assert_eq!(st.NoiseLevelBias, [50, 25, 16, 12]);
        for b in 0..4 {
            assert_eq!(st.NL[b], 100 * st.NoiseLevelBias[b]);
            assert_eq!(st.inv_NL[b], silk_DIV32(silk_int32_MAX, st.NL[b]));
            assert_eq!(st.NrgRatioSmth_Q8[b], 25600);
        }
        assert_eq!(st.counter, 15);
    }

    #[test]
    fn noise_level_smoothing_raises_nl_on_high_energy() {
        let mut st = silk_VAD_state::default();
        silk_VAD_Init(&mut st);
        let before = st.NL;
        let pX = [1i32 << 24, 1 << 24, 1 << 24, 1 << 24];
        silk_VAD_GetNoiseLevels(&pX, &mut st);
        for b in 0..4 {
            assert!(st.NL[b] >= before[b]);
        }
        assert_eq!(st.counter, 16);
    }
}

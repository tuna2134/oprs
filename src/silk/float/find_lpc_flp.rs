//! Mirror of `silk/float/find_LPC_FLP.c` from libopus 1.6.1, transliterated to safe Rust.
//!
//! LPC analysis (Burg) plus the NLSF-interpolation search.
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
use crate::silk::define::{MAX_FRAME_LENGTH, MAX_LPC_ORDER, MAX_NB_SUBFR};
use crate::silk::float::burg_modified_flp::silk_burg_modified_FLP;
use crate::silk::float::energy_flp::silk_energy_FLP;
use crate::silk::float::lpc_analysis_filter_flp::silk_LPC_analysis_filter_FLP;
use crate::silk::float::sigproc_flp::silk_float;
use crate::silk::float::wrappers_flp::{silk_A2NLSF_FLP, silk_NLSF2A_FLP};
use crate::silk::interpolate::silk_interpolate;
use crate::silk::structs::silk_encoder_state;
use crate::silk::typedef::*;
use crate::types::*;

/// LPC analysis
pub fn silk_find_LPC_FLP(
    psEncC: &mut silk_encoder_state, /* I/O  Encoder state      */
    NLSF_Q15: &mut [opus_int16],     /* O    NLSFs              */
    x: &[silk_float],                /* I    Input signal       */
    minInvGain: silk_float,          /* I    Inverse of max prediction gain */
    arch: opus_int,
) {
    let mut k: opus_int;
    let subfr_length: opus_int;
    let mut a = [0.0f32; MAX_LPC_ORDER as usize];

    /* Used only for NLSF interpolation */
    let mut res_nrg: silk_float;
    let mut res_nrg_2nd: silk_float;
    let mut res_nrg_interp: silk_float;
    let mut NLSF0_Q15 = [0 as opus_int16; MAX_LPC_ORDER as usize];
    let mut a_tmp = [0.0f32; MAX_LPC_ORDER as usize];
    let mut LPC_res = [0.0f32; (MAX_FRAME_LENGTH + MAX_NB_SUBFR * MAX_LPC_ORDER) as usize];

    subfr_length = psEncC.subfr_length + psEncC.predictLPCOrder;

    /* Default: No interpolation */
    psEncC.indices.NLSFInterpCoef_Q2 = 4;

    /* Burg AR analysis for the full frame */
    res_nrg = silk_burg_modified_FLP(
        &mut a,
        x,
        minInvGain,
        subfr_length,
        psEncC.nb_subfr,
        psEncC.predictLPCOrder,
        arch,
    );

    if psEncC.useInterpolatedNLSFs != 0
        && psEncC.first_frame_after_reset == 0
        && psEncC.nb_subfr == MAX_NB_SUBFR
    {
        /* Optimal solution for last 10 ms; subtract residual energy here, as that's easier than        */
        /* adding it to the residual energy of the first 10 ms in each iteration of the search below    */
        res_nrg -= silk_burg_modified_FLP(
            &mut a_tmp,
            &x[((MAX_NB_SUBFR / 2) * subfr_length) as usize..],
            minInvGain,
            subfr_length,
            MAX_NB_SUBFR / 2,
            psEncC.predictLPCOrder,
            arch,
        );

        /* Convert to NLSFs */
        silk_A2NLSF_FLP(NLSF_Q15, &a_tmp, psEncC.predictLPCOrder);

        /* Search over interpolation indices to find the one with lowest residual energy */
        res_nrg_2nd = silk_float_MAX;
        k = 3;
        while k >= 0 {
            /* Interpolate NLSFs for first half */
            silk_interpolate(
                &mut NLSF0_Q15,
                &psEncC.prev_NLSFq_Q15,
                NLSF_Q15,
                k,
                psEncC.predictLPCOrder,
            );

            /* Convert to LPC for residual energy evaluation */
            silk_NLSF2A_FLP(&mut a_tmp, &NLSF0_Q15, psEncC.predictLPCOrder, psEncC.arch);

            /* Calculate residual energy with LSF interpolation */
            silk_LPC_analysis_filter_FLP(
                &mut LPC_res,
                &a_tmp,
                x,
                2 * subfr_length,
                psEncC.predictLPCOrder,
            );
            res_nrg_interp = (silk_energy_FLP(
                &LPC_res[psEncC.predictLPCOrder as usize..],
                subfr_length - psEncC.predictLPCOrder,
            ) + silk_energy_FLP(
                &LPC_res[(psEncC.predictLPCOrder + subfr_length) as usize..],
                subfr_length - psEncC.predictLPCOrder,
            )) as silk_float;

            /* Determine whether current interpolated NLSFs are best so far */
            if res_nrg_interp < res_nrg {
                /* Interpolation has lower residual energy */
                res_nrg = res_nrg_interp;
                psEncC.indices.NLSFInterpCoef_Q2 = k as opus_int8;
            } else if res_nrg_interp > res_nrg_2nd {
                /* No reason to continue iterating - residual energies will continue to climb */
                break;
            }
            res_nrg_2nd = res_nrg_interp;
            k -= 1;
        }
    }

    if psEncC.indices.NLSFInterpCoef_Q2 == 4 {
        /* NLSF interpolation is currently inactive, calculate NLSFs from full frame AR coefficients */
        silk_A2NLSF_FLP(NLSF_Q15, &a, psEncC.predictLPCOrder);
    }

    celt_assert!(
        psEncC.indices.NLSFInterpCoef_Q2 == 4
            || (psEncC.useInterpolatedNLSFs != 0
                && psEncC.first_frame_after_reset == 0
                && psEncC.nb_subfr == MAX_NB_SUBFR)
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_input_gives_flat_nlsfs() {
        let mut e = silk_encoder_state::default();
        e.subfr_length = 80;
        e.predictLPCOrder = 16;
        e.nb_subfr = 4;
        e.arch = 4;
        e.useInterpolatedNLSFs = 0;
        e.first_frame_after_reset = 1;
        let x = vec![0.0f32; 384];
        let mut nlsf = [0i16; 16];
        silk_find_LPC_FLP(&mut e, &mut nlsf, &x, 0.1, 4);
        assert_eq!(e.indices.NLSFInterpCoef_Q2, 4);
        for i in 1..16 {
            assert!(nlsf[i] > nlsf[i - 1]);
        }
    }
}
